using System.Diagnostics;
using System.Diagnostics.CodeAnalysis;
using Kb.Ssd1306;

namespace KbEmulator;

/// <summary>What happened to a Raw HID packet from the computer (mirrors <c>RawHidResult</c> in Rust).</summary>
public enum RawHidResult : byte
{
    Pending = 0,
    Delivered = 1,
    NotLinked = 2,
    Cleared = 3,
    Asleep = 4,
}

/// <summary>Power state of the emulated keyboard (mirrors <c>PowerState</c> in Rust).</summary>
public enum PowerState : byte
{
    Active = 0,
    /// <summary>OLEDs switched off after the idle timeout.</summary>
    Idle = 1,
    /// <summary>Deep sleep: radio off, OLED power cut. The next key press reboots and is lost.</summary>
    Sleep = 2,
}

/// <summary>How a simulated pairing with a code ended (mirrors <c>PairResult</c> in Rust).</summary>
public enum PairResult : byte
{
    /// <summary>The code typed matched: the computer is paired on the profile.</summary>
    Paired = 0,
    WrongCode = 1,
    /// <summary>Escape on the keyboard.</summary>
    Rejected = 2,
    /// <summary>Nothing typed within 60 s.</summary>
    TimedOut = 3,
}

/// <summary>Where key reports go (mirrors <c>Transport</c> in Rust).</summary>
public enum Transport : byte
{
    Usb = 0,
    Bluetooth = 1,
}

/// <summary>
/// Where the mouse keys have taken the pointer: summed motion (px), wheel and
/// pan (detents, or 1/16 detents with smooth scrolling: <see cref="Hires"/> bit 0 wheel, bit 1 pan),
/// buttons held, the pointer active (MOUSE bar), still moving.
/// </summary>
public readonly record struct MouseState(int X, int Y, int Wheel, int Pan, byte Buttons, bool PointerActive, bool Moving, byte Hires);

/// <summary>HID report as the host would receive it: modifier bits plus NKRO key bitmap.</summary>
public sealed record HidReport(byte Mods, byte[] Keys)
{
    public static readonly HidReport Empty = new(0, new byte[32]);

    public bool Contains(int usage) => (Keys[usage / 8] & (1 << (usage % 8))) != 0;

    /// <summary>Whether a usage is down: a key in the bitmap, or a modifier (0xE0..0xE7) in <see cref="Mods"/>.</summary>
    public bool IsDown(int usage) =>
        usage >= FirstModifier ? (Mods & (1 << (usage - FirstModifier))) != 0 : Contains(usage);

    /// <summary>Names of the modifiers in <paramref name="mods"/> (bit 0 LCtrl … bit 7 RGui).</summary>
    public static List<string> ModifierNames(byte mods)
    {
        var names = new List<string>();
        for (var bit = 0; bit < 8; bit++)
        {
            if ((mods & (1 << bit)) != 0) names.Add(EmulatedKeyboard.UsageName(FirstModifier + bit));
        }
        return names;
    }

    /// <summary>Names of the keys held, in usage order (modifiers are in <see cref="Mods"/>).</summary>
    public IEnumerable<string> KeyNames() =>
        Enumerable.Range(0, FirstModifier).Where(Contains).Select(EmulatedKeyboard.UsageName);

    /// <summary>The first modifier usage (LCtrl); usages from here on are the modifier bits.</summary>
    public const int FirstModifier = 0xE0;
}

/// <summary>
/// The Corne's firmware (Rust, via <see cref="Native"/>) plus the two SSD1306
/// displays it talks to. Owns the clock; call <see cref="Tick"/> often. Not thread-safe: use it
/// from one thread (the UI thread in the app).
/// </summary>
[SuppressMessage("Security", "S6640:Using unsafe code blocks is security-sensitive",
    Justification = "Passes pinned byte buffers to the C ABI of libkb_emu.so (Native, StudioNative).")]
public sealed unsafe class EmulatedKeyboard : IDisposable
{
    private static readonly string[] SystemActions = ["Profile previous", "Profile next", "Profile clear", "USB/BT toggle", "Studio unlock", "Show link", "Radio power"];

    private readonly EmulatorHandle _handle = Native.kb_emu_new();
    private readonly Func<uint> _clock;
    private byte[] _displayBuffer = new byte[4096];

    public Ssd1306[] Displays { get; } = [new Ssd1306(), new Ssd1306()];

    /// <param name="clock">Milliseconds since start; defaults to a stopwatch. Tests pass their own.</param>
    public EmulatedKeyboard(Func<uint>? clock = null)
    {
        if (clock == null)
        {
            var stopwatch = Stopwatch.StartNew();
            clock = () => (uint)stopwatch.ElapsedMilliseconds;
        }
        _clock = clock;
    }

    public int KeyCount { get; } = (int)Native.kb_emu_key_count();

    public PowerState Power => (PowerState)Native.kb_emu_power_state(_handle);

    public event Action<PowerState>? PowerChanged;

    /// <summary>Idle and deep-sleep timeouts; null sleep disables deep sleep.</summary>
    public void SetPowerTimeouts(TimeSpan idle, TimeSpan? sleep)
    {
        Native.kb_emu_set_power_timeouts(_handle, (uint)idle.TotalMilliseconds, (uint)(sleep?.TotalMilliseconds ?? 0), Now);
        Drain();
    }
    public HidReport LastReport { get; private set; } = HidReport.Empty;
    public int Layer { get; private set; }

    public event Action<HidReport, HidReport>? ReportSent;
    public event Action<int>? LayerChanged;
    public event Action<bool>? CapsWordChanged;
    public event Action<string>? SystemAction;

    /// <summary>A simulated pairing with a code ended.</summary>
    public event Action<PairResult>? PairingEnded;

    /// <summary>
    /// A computer starts pairing with a code on the selected profile: it shows
    /// <paramref name="code"/> (6 digits), the left screen asks for it. Typing the
    /// digits and Enter on the keyboard pairs, Escape or another code doesn't.
    /// </summary>
    /// <returns>The code the computer shows.</returns>
    public int SimulatePairing(int code)
    {
        var shown = (int)Native.kb_emu_simulate_pairing(_handle, (uint)code, Now);
        Drain();
        return shown;
    }

    /// <summary>A pairing prompt is on the left screen.</summary>
    public bool Pairing => Native.kb_emu_pairing(_handle);

    /// <summary>The radio's transmit power in dBm (RadioPower key on the nav layer).</summary>
    public int RadioDbm => Native.kb_emu_radio_dbm(_handle);

    /// <summary>Radio levels the RadioPower key steps through, in dBm.</summary>
    public static IReadOnlyList<int> RadioLevels { get; } = [8, 4, 0];

    private const int NavKey = 41, RadioKey = 29;

    /// <summary>
    /// Set the radio level the way the keyboard does: tap the RadioPower key
    /// on the nav layer until it shows <paramref name="dbm"/> (the left OLED
    /// shows it too). False if the keymap has no Radio key there.
    /// </summary>
    public bool SetRadioDbm(int dbm)
    {
        for (var i = 0; i < RadioLevels.Count && RadioDbm != dbm; i++)
        {
            Press(NavKey);
            var isRadio = Label(RadioKey).StartsWith("Radio", StringComparison.Ordinal);
            if (isRadio)
            {
                Press(RadioKey);
                Release(RadioKey);
            }
            Release(NavKey);
            if (!isRadio) return false;
        }
        return RadioDbm == dbm;
    }

    /// <summary>A &amp;bt / &amp;out key changed the profile, its pairing or the transport.</summary>
    public event Action? BluetoothChanged;

    // ---- mouse keys --------------------------------------------------------------------

    /// <summary>A mouse report for the computer, and the emulator time it was sent.</summary>
    public event Action<Kb.HostProtocol.MouseReport, uint>? MouseReported;

    /// <summary>Where the mouse keys have taken the pointer (summed since start or reset).</summary>
    public MouseState Mouse => Native.kb_emu_mouse_state(_handle, out var m)
        ? new MouseState(m.X, m.Y, m.Wheel, m.Pan, m.Buttons, m.Pointer != 0, m.Moving != 0, m.Hires)
        : default;

    /// <summary>Put the pointer back at the middle (x, y, wheel, pan = 0).</summary>
    public void ResetMousePosition() => Native.kb_emu_mouse_reset_position(_handle);

    /// <summary>The emulated computer supports smooth scrolling (wheel and pan in 1/16 detents).</summary>
    public void SetSmoothScrolling(bool wheel, bool pan) => Native.kb_emu_mouse_set_resolution(_handle, wheel, pan);

    // ---- Studio (docs/studio-protocol.md) --------------------------------------------

    /// <summary>A Studio protocol line (<c>#studio ...</c>), as the keyboard's log port takes it.</summary>
    public bool StudioCommand(string line)
    {
        var ok = StudioNative.With(line, (p, len) => StudioNative.kb_emu_studio_command(_handle, (byte*)p, len, Now));
        Drain();
        return ok;
    }

    private byte[] _studioLine = new byte[Math.Max(64, (int)StudioNative.kb_emu_studio_line_max())];

    /// <summary>The next answer line (without \r\n), or null when there is none.</summary>
    public string? StudioNextLine()
    {
        while (true)
        {
            nuint n;
            fixed (byte* p = _studioLine) n = StudioNative.kb_emu_studio_next_line(_handle, Now, p, (nuint)_studioLine.Length);
            if (n == 0) return null;
            if ((int)n > _studioLine.Length)
            {
                _studioLine = new byte[(int)n];
                continue;
            }
            return System.Text.Encoding.UTF8.GetString(_studioLine, 0, (int)n).TrimEnd('\r', '\n');
        }
    }

    /// <summary>The compiled (default) binding of a key, encoded.</summary>
    public string StudioDefault(int layer, int pos) =>
        Native.ReadString((buf, cap) => StudioNative.kb_emu_studio_default_binding(_handle, (byte)layer, (byte)pos, (byte*)buf, cap));

    /// <summary>The live binding of a key, encoded.</summary>
    public string StudioBinding(int layer, int pos) =>
        Native.ReadString((buf, cap) => StudioNative.kb_emu_studio_binding(_handle, (byte)layer, (byte)pos, (byte*)buf, cap));

    /// <summary>Dry run of a set (no lock check): 0 ok, else the result code (5 lockout, 6 complex, …).</summary>
    public int StudioCheck(int layer, int pos, string encoding) =>
        StudioNative.With(encoding, (p, len) => StudioNative.kb_emu_studio_check(_handle, (byte)layer, (byte)pos, (byte*)p, len));

    /// <summary>The live combos as Studio's combo text (<c>copy 26,27 0 50 150 kp:LC(C)</c>).</summary>
    public IReadOnlyList<string> ReadStudioCombos() =>
        Enumerable.Range(0, StudioNative.kb_emu_studio_combo_count(_handle))
            .Select(i => Native.ReadString((buf, cap) => StudioNative.kb_emu_studio_combo(_handle, (byte)i, (byte*)buf, cap))).ToList();

    /// <summary>The compiled combos as Studio's combo text.</summary>
    public IReadOnlyList<string> ReadStudioDefaultCombos() =>
        Enumerable.Range(0, StudioNative.kb_emu_studio_default_combo_count(_handle))
            .Select(i => Native.ReadString((buf, cap) => StudioNative.kb_emu_studio_default_combo(_handle, (byte)i, (byte*)buf, cap))).ToList();

    /// <summary>Dry run of a combo (index, or null for a new one); 0 ok, else the result code.</summary>
    public int StudioComboCheck(int? index, string text) =>
        StudioNative.With(text, (p, len) => StudioNative.kb_emu_studio_combo_check(_handle, (byte)(index ?? 255), (byte*)p, len));

    /// <summary>Dry run of removing a combo; 0 ok, 5 lockout.</summary>
    public int StudioComboCheckRemove(int index) => StudioNative.kb_emu_studio_combo_check_remove(_handle, (byte)index);

    /// <summary>Keep Studio's edits in a text file (absolute path). 0 applied, 1 ignored (bad file), 2 dropped (lock-out).</summary>
    public int SetStudioStorage(string path)
    {
        var result = StudioNative.With(path, (p, len) => StudioNative.kb_emu_studio_set_storage_path(_handle, (byte*)p, len));
        Drain();
        return result;
    }

    public TimeSpan StudioUnlockRemaining => TimeSpan.FromMilliseconds(StudioNative.kb_emu_studio_unlock_remaining_ms(_handle, Now));

    public bool StudioDirty => StudioNative.kb_emu_studio_dirty(_handle);

    /// <summary>Changes whenever a live binding changes.</summary>
    public uint StudioGeneration => StudioNative.kb_emu_studio_generation(_handle);

    /// <summary>Press StudioUnlock as on the keyboard: hold nav (41), press 35, let go.</summary>
    public void PressStudioUnlock()
    {
        Press(NavKey);
        Press(35);
        Release(35);
        Release(NavKey);
    }

    private uint Now => _clock();

    /// <summary>The emulator's clock, ms.</summary>
    public uint NowMs => Now;

    public void Press(int pos)
    {
        Native.kb_emu_press(_handle, (byte)pos, Now);
        Drain();
    }

    public void Release(int pos)
    {
        Native.kb_emu_release(_handle, (byte)pos, Now);
        Drain();
    }

    private uint _lastTick;

    public void Tick()
    {
        var linked = PeripheralConnected;
        var now = Now;
        // Mouse keys move in 1 ms steps: run every millisecond the emulator
        // asks for (kb_emu_next_deadline) up to now, so motion stays smooth
        // even when the app's timer comes less often.
        for (var guard = 0; guard < 2000; guard++)
        {
            var deadline = Native.kb_emu_next_deadline(_handle);
            if (deadline < 0 || deadline >= now || deadline <= _lastTick) break;
            Native.kb_emu_tick(_handle, (uint)deadline);
            Drain((uint)deadline);
            _lastTick = (uint)deadline;
        }
        Native.kb_emu_tick(_handle, now);
        Drain(now);
        _lastTick = now;
        // The halves link again by themselves after a half (re)started.
        if (PeripheralConnected != linked) BluetoothChanged?.Invoke();
    }

    public string Label(int pos) =>
        Native.ReadString((buf, cap) => Native.kb_emu_label(_handle, (byte)pos, (byte*)buf, cap));

    public string LayerName(int layer) =>
        Native.ReadString((buf, cap) => Native.kb_emu_layer_name(_handle, (byte)layer, (byte*)buf, cap));

    public static string UsageName(int usage) =>
        Native.ReadString((buf, cap) => Native.kb_emu_usage_name((byte)usage, (byte*)buf, cap));

    public void SetBattery(int half, int percent) =>
        Native.kb_emu_set_battery(_handle, (byte)half, (byte)Math.Clamp(percent, 0, 100));

    /// <summary>A half's power switch: switching on starts it (animation, then the link).</summary>
    public void SetHalfPower(int half, bool on)
    {
        Native.kb_emu_set_half_power(_handle, (byte)half, on, Now);
        Drain();
        BluetoothChanged?.Invoke();
    }

    /// <summary>A half's reset button.</summary>
    public void ResetHalf(int half)
    {
        Native.kb_emu_reset_half(_handle, (byte)half, Now);
        Drain();
        BluetoothChanged?.Invoke();
    }

    /// <summary>Play the start-up animation again, as after switching the keyboard on.</summary>
    public void ReplayBoot() => Native.kb_emu_replay_boot(_handle);

    /// <summary>Whether a half runs on USB power (the screen shows a charging bolt).</summary>
    public void SetCharging(int half, bool charging) =>
        Native.kb_emu_set_charging(_handle, (byte)half, charging);

    /// <summary>A 32-byte Raw HID packet from the computer arrives at the left half.</summary>
    public RawHidResult RawHid(byte[] packet)
    {
        RawHidResult result;
        fixed (byte* p = packet)
        {
            result = (RawHidResult)Native.kb_emu_raw_hid(_handle, p, (nuint)packet.Length, Now);
        }
        Drain(); // a notification can wake the screens
        return result;
    }

    /// <summary>true = upright portrait screens (this firmware), false = the classic sideways status screen.</summary>
    public void SetPortraitScreens(bool portrait) =>
        Native.kb_emu_set_screen_style(_handle, portrait ? (byte)1 : (byte)0);

    /// <summary>Number of Bluetooth profiles (paired computers) the firmware keeps.</summary>
    public static int ProfileCount { get; } = Native.kb_emu_profile_count();

    /// <summary>Split link stages as the firmware names them; index 0 (empty) hides the stage.</summary>
    public static IReadOnlyList<string> LinkStages { get; } =
        Enumerable.Range(0, Native.kb_emu_link_stage_count())
            .Select(i => Native.ReadString((buf, cap) => Native.kb_emu_link_stage_name((byte)i, (byte*)buf, cap)))
            .ToArray();

    /// <summary>Selected Bluetooth profile, 0-based. Each profile keeps its own open/taken state.</summary>
    public int Profile
    {
        get => Native.kb_emu_profile(_handle);
        set => Native.kb_emu_set_profile(_handle, (byte)Math.Clamp(value, 0, ProfileCount - 1));
    }

    /// <summary>Selected profile has no computer paired (shown as +).</summary>
    public bool ProfileOpen => Native.kb_emu_profile_open(_handle);

    /// <summary>Selected profile's computer is connected (shown as ✓; taken but not connected is ✗).</summary>
    public bool ProfileConnected => Native.kb_emu_profile_connected(_handle);

    /// <summary>Right half linked to the left.</summary>
    public bool PeripheralConnected => Native.kb_emu_peripheral_connected(_handle);

    public Transport Transport
    {
        get => (Transport)Native.kb_emu_transport(_handle);
        set => Native.kb_emu_set_transport(_handle, (byte)value);
    }

    /// <summary>Index into <see cref="LinkStages"/> shown at the bottom of the OLEDs; 0 = hidden.</summary>
    public int LinkStage
    {
        get => Native.kb_emu_link_stage(_handle);
        set => Native.kb_emu_set_link_stage(_handle, (byte)Math.Clamp(value, 0, LinkStages.Count - 1));
    }

    /// <summary>Connect or disconnect the selected profile's computer; connecting an open profile pairs it.</summary>
    public void SetProfileConnected(bool connected) =>
        Native.kb_emu_set_profile_connected(_handle, connected);

    /// <summary>Mark the selected profile open (nothing paired) or taken; opening it also disconnects.</summary>
    public void SetProfileOpen(bool open) =>
        Native.kb_emu_set_profile_open(_handle, open);

    public void SetPeripheralConnected(bool connected) =>
        Native.kb_emu_set_peripheral_connected(_handle, connected);

    /// <summary>
    /// Let the firmware refresh both screens. Its I²C writes (framed as
    /// [u16 length][bytes]) go straight into the SSD1306 models.
    /// </summary>
    public void UpdateDisplays() =>
        DisplayFeed.Update(Displays, ref _displayBuffer, (half, buffer) =>
        {
            fixed (byte* p = buffer)
            {
                return (int)Native.kb_emu_display(_handle, (byte)half, Now, p, (nuint)buffer.Length);
            }
        });

    private void Drain(uint? at = null)
    {
        var now = at ?? Now;
        while (Native.kb_emu_poll(_handle, now, out var ev)) Dispatch(ev, now);
    }

    /// <summary>One event from the firmware, raised as the matching .NET event.</summary>
    private void Dispatch(Native.KbEvent ev, uint now)
    {
        switch (ev.Kind)
        {
            case Native.KindMouse:
                if (Kb.HostProtocol.MouseReport.FromBytes(Bytes(ev, Kb.HostProtocol.MouseReport.Size)) is { } mouse)
                    MouseReported?.Invoke(mouse, now);
                break;
            case Native.KindReport:
                var report = new HidReport(ev.Mods, Bytes(ev, 32));
                var previous = LastReport;
                LastReport = report;
                ReportSent?.Invoke(previous, report);
                break;
            case Native.KindLayer:
                Layer = ev.Value;
                LayerChanged?.Invoke(ev.Value);
                break;
            case Native.KindCapsWord:
                CapsWordChanged?.Invoke(ev.Value != 0);
                break;
            case Native.KindPower:
                OnPower((PowerState)ev.Value);
                break;
            case Native.KindPairing:
                PairingEnded?.Invoke((PairResult)ev.Value);
                BluetoothChanged?.Invoke();
                break;
            case Native.KindSystem:
                SystemAction?.Invoke(ev.Value < SystemActions.Length ? SystemActions[ev.Value] : $"#{ev.Value}");
                BluetoothChanged?.Invoke();
                break;
        }
    }

    /// <summary>The first <paramref name="count"/> bytes of an event's key array.</summary>
    private static byte[] Bytes(Native.KbEvent ev, int count)
    {
        var bytes = new byte[count];
        for (var i = 0; i < count; i++) bytes[i] = ev.Keys[i];
        return bytes;
    }

    private void OnPower(PowerState state)
    {
        // VCC to the OLEDs is cut in deep sleep and restored on wake.
        foreach (var display in Displays) display.Powered = state != PowerState.Sleep;
        PowerChanged?.Invoke(state);
    }

    public void Dispose() => _handle.Dispose();
}

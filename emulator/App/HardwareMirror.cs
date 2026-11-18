using System.Diagnostics;
using System.Diagnostics.CodeAnalysis;
using System.Text;
using Kb.HostProtocol;
using Kb.Ssd1306;

namespace KbEmulator;

/// <summary>
/// The real keyboard as its debug stream describes it (DEBUG tab): held
/// keys, the report for the computer, the status, and both OLEDs. The left
/// OLED shows the frame the hardware drew; the right one is rendered from the
/// status by the firmware's own screen code (the left half doesn't have the
/// right half's frame).
/// </summary>
[SuppressMessage("Security", "S6640:Using unsafe code blocks is security-sensitive",
    Justification = "Passes pinned byte buffers and the status struct to the C ABI of libkb_emu.so (MirrorNative).")]
public sealed unsafe class HardwareMirror : IDisposable
{
    private readonly MirrorHandle _handle = MirrorNative.kb_mirror_new();
    private readonly Func<uint> _clock;
    private readonly Func<int, string> _layerName;
    private byte[] _displayBuffer = new byte[4096];

    /// <param name="layerName">Name of a layer (the keymap's).</param>
    /// <param name="clock">Milliseconds, for the right screen's animations; defaults to a stopwatch.</param>
    public HardwareMirror(Func<int, string> layerName, Func<uint>? clock = null)
    {
        _layerName = layerName;
        if (clock == null)
        {
            var stopwatch = Stopwatch.StartNew();
            clock = () => (uint)stopwatch.ElapsedMilliseconds;
        }
        _clock = clock;
        Host = new HostText(EmulatedKeyboard.UsageName);
    }

    public Ssd1306[] Displays { get; } = [new Ssd1306(), new Ssd1306()];

    public DebugHello? Hello { get; private set; }
    public DebugStatus? Status { get; private set; }
    public ulong Held { get; private set; }
    public HidReport Report { get; private set; } = HidReport.Empty;

    /// <summary>What the computer got typed (not while muted).</summary>
    public HostText Host { get; }

    /// <summary>Something happened, for the events pane: a key, a layer, a report.</summary>
    public event Action<string>? Event;

    /// <summary>The layer changed: key labels change.</summary>
    public event Action? LayerChanged;

    public void Apply(DebugEvent e)
    {
        switch (e)
        {
            case DebugHello hello:
                Hello = hello;
                Event?.Invoke($"debug stream from {hello.Name} ({hello.Half} half, {hello.Build}, protocol {hello.Version})");
                break;
            case DebugKeys keys:
                ApplyKeys(keys);
                break;
            case DebugReport report:
                ApplyReport(report);
                break;
            case DebugStatus status:
                ApplyStatus(status);
                break;
            case DebugFrame frame:
                SetFrame(frame.Frame);
                break;
        }
    }

    /// <summary>Bits of <see cref="Held"/>: key positions the stream can report.</summary>
    private const int HeldBits = 64;

    private void ApplyKeys(DebugKeys keys)
    {
        for (var pos = 0; pos < HeldBits; pos++)
        {
            var was = (Held & (1UL << pos)) != 0;
            var now = keys.IsHeld(pos);
            if (was != now) Event?.Invoke($"key {pos} {(now ? "down" : "up")}: {Label(pos).Replace('\n', '/')}");
        }
        Held = keys.Held;
    }

    private void ApplyReport(DebugReport report)
    {
        var next = ToHidReport(report);
        var change = Describe(Report, next);
        var muted = Status?.Mute == true;
        if (!muted) Host.Apply(Report, next);
        if (change.Length > 0) Event?.Invoke((muted ? "report (muted) " : "report ") + change);
        Report = next;
    }

    private void ApplyStatus(DebugStatus status)
    {
        var layerChanged = Status?.Layer != status.Layer;
        Status = status;
        SetNativeStatus(status);
        if (!layerChanged) return;
        Event?.Invoke($"layer → {_layerName(status.Layer)}");
        LayerChanged?.Invoke();
    }

    /// <summary>The left OLED's frame as the hardware drew it; null while the OLED is off.</summary>
    private void SetFrame(byte[]? frame)
    {
        if (frame == null)
        {
            MirrorNative.kb_mirror_set_frame(_handle, 0, null);
            return;
        }
        fixed (byte* p = frame) MirrorNative.kb_mirror_set_frame(_handle, 0, p);
    }

    /// <summary>The connection is gone: nothing is known to be held any more.</summary>
    public void Disconnected()
    {
        Hello = null;
        Held = 0;
        Report = HidReport.Empty;
    }

    /// <summary>Label of a key on the real keyboard's current layer.</summary>
    public string Label(int pos)
    {
        var layer = (byte)(Status?.Layer ?? 0);
        return Native.ReadString((buf, cap) => MirrorNative.kb_mirror_label(layer, (byte)pos, (byte*)buf, cap));
    }

    /// <summary>Bring both OLED models up to date (call often: the right one animates).</summary>
    public void UpdateDisplays()
    {
        var now = _clock();
        DisplayFeed.Update(Displays, ref _displayBuffer, (half, buffer) =>
        {
            fixed (byte* p = buffer)
            {
                return (int)MirrorNative.kb_mirror_display(_handle, (byte)half, now, p, (nuint)buffer.Length);
            }
        });
    }

    /// <summary>A boot report (6 key codes) as the NKRO bitmap the host pane works with.</summary>
    public static HidReport ToHidReport(DebugReport report)
    {
        var keys = new byte[32];
        foreach (var code in report.Keys.Where(code => code != 0))
            keys[code / 8] |= (byte)(1 << (code % 8));
        return new HidReport(report.Mods, keys);
    }

    /// <summary>What changed between two reports: <c>+A -LShift</c>.</summary>
    public static string Describe(HidReport previous, HidReport next)
    {
        var parts = new List<string>();
        for (var usage = 0; usage < 256; usage++)
        {
            var (was, now) = (previous.IsDown(usage), next.IsDown(usage));
            if (was != now) parts.Add((now ? "+" : "-") + EmulatedKeyboard.UsageName(usage));
        }
        return string.Join(' ', parts);
    }

    private void SetNativeStatus(DebugStatus s)
    {
        var native = new MirrorNative.Status
        {
            Layer = (byte)s.Layer,
            CapsWord = Bit(s.CapsWord),
            Mods = s.Mods,
            Transport = (byte)(s.Output == "usb" ? 0 : 1),
            Profile = (byte)Math.Max(0, s.Profile - 1),
            ProfileConnected = Bit(s.HostConnected),
            ProfileOpen = Bit(s.ProfileOpen),
            Linked = Bit(s.Linked),
            BatteryLeft = (byte)Math.Clamp(s.BatteryLeft, 0, 100),
            BatteryRight = (byte)Math.Clamp(s.BatteryRight, 0, 100),
            ChargingLeft = Bit(s.ChargingLeft),
            ChargingRight = Bit(s.ChargingRight),
            Power = s.Power switch { "idle" => 1, "sleep" => 2, _ => 0 },
            Wpm = (ushort)Math.Clamp(s.Wpm, 0, ushort.MaxValue),
            Presses = (ushort)(s.Presses & 0xFFFF),
        };
        var stage = Encoding.UTF8.GetBytes(s.Stage);
        fixed (byte* p = stage)
        {
            MirrorNative.kb_mirror_set_status(_handle, &native, p, (nuint)stage.Length);
        }

        static byte Bit(bool b) => b ? (byte)1 : (byte)0;
    }

    public void Dispose() => _handle.Dispose();
}

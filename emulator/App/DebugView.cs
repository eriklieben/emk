using System.Collections.Concurrent;
using System.Text.RegularExpressions;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Threading;
using Kb.HostProtocol;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>
/// Debug mode: the real keyboard, live. Its left half on USB (a
/// <c>usb-log</c> build) streams held keys, the report for the computer, the
/// status and its OLED frame; clicks and (optionally) this PC's keyboard
/// press keys on it.
/// </summary>
public sealed partial class DebugView : Grid, IDisposable
{
    private const int MaxLogLines = 3000;
    private const int MaxEventLines = 500;
    private const string BootText = "Bootloader…";
    private const string ConnectText = "Connect", DisconnectText = "Disconnect";
    private static readonly TimeSpan BootArmTime = TimeSpan.FromSeconds(3);
    private static readonly TimeSpan HelloTimeout = TimeSpan.FromSeconds(2);

    private readonly HardwarePort _port;
    private readonly HardwareMirror _mirror;
    private readonly Func<int, string> _layerName;
    private readonly CorneView _view;
    private readonly ConcurrentQueue<Action> _fromPort = new();
    private readonly DebugLineSplitter _splitter = new();
    private readonly DispatcherTimer _timer;
    private readonly Dictionary<PhysicalKey, int> _physicalDown = new();

    private readonly Lamp _connection = new("", LampState.Crit, 14) { Wraps = true };
    private readonly LogWell _hostWell = new(textMode: true);
    private readonly LogWell _eventWell = new(MaxEventLines);
    private readonly LogWell _logWell = new(MaxLogLines, msTime: true);
    private readonly LogWell _mouseWell = new(200) { MinHeight = 40 };
    private readonly MouseLog _mouseLog = new();
    private readonly System.Diagnostics.Stopwatch _clock = System.Diagnostics.Stopwatch.StartNew();
    private MouseReport? _lastMouse;
    private readonly TextBlock _logTitle = W.Lbl("Firmware log · following");
    private readonly TextBlock _statusAge = W.Lbl("Status");
    private readonly Dictionary<string, TextBlock> _readout = new();
    private readonly BenchButton _connect = new(DisconnectText);
    private readonly BenchButton _trace = new("Trace", letter: "t", latching: true);
    private readonly BenchButton _mute = new("Mute", letter: "m", latching: true);
    private readonly BenchButton _boot = new(BootText, ButtonKind.Danger);
    private readonly Toggle _capture = new("Drive with this PC's keyboard");
    private readonly Toggle _follow = new("Follow log", true);

    private string? _device;
    private DateTime _connectedAt;
    private DateTime _askedAt;
    private DateTime _statusAt;
    private long _bytesSinceConnect;
    private bool _warnedUnconfirmed;
    private DateTime _muteSentAt;
    private DateTime _bootArmedAt;
    private bool _syncing;
    private bool _released;
    private string? _bootloader;
    private (string Name, string Half)? _bootIdentity;
    private readonly int[] _displayVersions = [-1, -1];

    /// <param name="layerName">Name of a keymap layer.</param>
    /// <param name="keyCount">Key positions (42).</param>
    /// <param name="port">The log port; default finds the keyboard by its USB ID.</param>
    public DebugView(Func<int, string> layerName, int keyCount, HardwarePort? port = null)
    {
        _layerName = layerName;
        _port = port ?? new HardwarePort();
        _mirror = new HardwareMirror(layerName);
        _view = new CorneView(keyCount) { Displays = _mirror.Displays, ShowBoardControls = false, Dimmed = true };
        _view.SetHardwareHeld(0);
        _view.KeyPressed += pos => Send(DebugProtocol.Press(pos));
        _view.KeyReleased += pos => Send(DebugProtocol.Release(pos));
        _mirror.Event += AddEvent;
        _mirror.LayerChanged += RefreshLabels;

        _port.OnConnectSend = DebugProtocol.DebugOn;
        _port.Connected += device => _fromPort.Enqueue(() => OnConnected(device));
        _port.Disconnected += why => _fromPort.Enqueue(() => OnDisconnected(why));
        _port.Received += bytes => _fromPort.Enqueue(() => OnBytes(bytes));

        BuildLayout();
        RefreshLabels();
        RefreshStatus();
        ShowConnection(LampState.Crit, "Not connected");

        _timer = new DispatcherTimer(TimeSpan.FromMilliseconds(30), DispatcherPriority.Input, (_, _) => OnTick());
    }

    internal HardwareMirror Mirror => _mirror;

    /// <summary>The buttons and toggles, for tests that click them.</summary>
    internal (BenchButton Status, BenchButton Dump, BenchButton Resync, BenchButton Release, BenchButton Clear) Controls;

    internal BenchButton TraceButton => _trace;
    internal BenchButton MuteButton => _mute;
    internal BenchButton BootButton => _boot;
    internal BenchButton ConnectButton => _connect;
    internal Toggle FollowToggle => _follow;
    internal string HostText => _hostWell.Text;
    internal bool LogFollows => _logWell.Follow;

    /// <summary>Esc: disarm the Bootloader button. True if it was armed.</summary>
    public bool Escape()
    {
        if (_bootArmedAt == default) return false;
        _bootArmedAt = default;
        _boot.Text = BootText;
        return true;
    }

    /// <summary>A Studio line (#V #L #A #U #D) from the keyboard, for Studio's keyboard target.</summary>
    public event Action<string>? StudioLineReceived;

    /// <summary>Send a raw line to the keyboard (Studio); false when the port isn't open.</summary>
    public bool SendRaw(string text) => _port.Send(text);

    /// <summary>Open the log port for Studio without showing Debug (it keeps running, like Debug's).</summary>
    public void EnsurePort()
    {
        _timer.Start();
        if (!_port.IsRunning && _connect.Text == DisconnectText && !_released) _port.Start();
    }

    /// <summary>Whether the port is open to a keyboard right now.</summary>
    public bool PortConnected => _device != null;

    /// <summary>The latest boot line about the bootloader seen here (Flash and Security show it).</summary>
    public BootFacts? LastBoot { get; private set; }

    /// <summary>Connection, stream, mute or trace changed (the top bar's lamps).</summary>
    public event Action? StateChanged;
    internal CorneView View => _view;

    /// <summary>The top bar's Stream lamp: green with the debug stream, amber with the port only, off without.</summary>
    public LampState StreamState
    {
        get
        {
            if (_mirror.Hello != null) return LampState.Good;
            return _device != null ? LampState.Warn : LampState.Off;
        }
    }

    /// <summary>The mode is shown: connect (the port reconnects by itself from then on).</summary>
    public void Activate() => EnsurePort();

    /// <summary>Let go of the log port so ./hil can use it (flashing). True if it was open.</summary>
    public bool ReleasePort()
    {
        if (!_port.IsRunning) return false;
        ReleaseAll();
        _port.Stop();
        _released = true;
        OnDisconnected("log port handed to ./hil flash");
        return true;
    }

    /// <summary>Take the log port again after <see cref="ReleasePort"/>.</summary>
    public void ResumePort()
    {
        if (!_released) return;
        _released = false;
        if (_connect.Text == DisconnectText) _port.Start();
    }

    internal bool PortRunning => _port.IsRunning;

    /// <summary>The mode is hidden: release every key pressed from here.</summary>
    public void Deactivate()
    {
        ReleaseAll();
    }

    /// <summary>A key on this PC's keyboard (routed here while Debug is shown). True if used.</summary>
    public bool OnPhysicalKey(PhysicalKey key, bool down)
    {
        if (!down)
        {
            if (!_physicalDown.Remove(key, out var held)) return false;
            _view.SetPressed(held, false);
            Send(DebugProtocol.Release(held));
            return true;
        }
        if (!_capture.IsChecked) return false;
        if (PhysicalLayout.Position(key) is not { } pos) return false;
        if (!_physicalDown.TryAdd(key, pos)) return true; // autorepeat
        _view.SetPressed(pos, true);
        Send(DebugProtocol.Press(pos));
        return true;
    }

    /// <summary>Drive the real keyboard with this PC's keyboard (mutes the Corne).</summary>
    internal bool Driving
    {
        get => _capture.IsChecked;
        set => _capture.IsChecked = value;
    }

    /// <summary>Release keys pressed with this PC's keyboard (the window lost focus).</summary>
    public void ReleasePhysical()
    {
        foreach (var (_, pos) in _physicalDown)
        {
            _view.SetPressed(pos, false);
            Send(DebugProtocol.Release(pos));
        }
        _physicalDown.Clear();
    }

    private void ReleaseAll()
    {
        ReleasePhysical();
        _view.ReleaseLatched();
    }

    private void Send(string command)
    {
        var key = command.Length > 0 && command[0] is 'p' or 'r' or 'k';
        if (!_port.Send(command))
        {
            if (key) AddEvent("not connected: key not sent");
            return;
        }
        if (key && _mirror.Hello == null && !_warnedUnconfirmed)
        {
            _warnedUnconfirmed = true;
            AddEvent("key sent, but the keyboard has no debug stream: nothing confirms it arrived");
        }
    }

    /// <summary>
    /// What the connection line says when the port is open but no debug
    /// stream came: nothing at all (the old log port that went silent), or
    /// log lines only (firmware from before the debug stream).
    /// </summary>
    internal static string NoStreamMessage(string device, long bytesReceived) =>
        bytesReceived == 0
            ? $"Port {device} is open but the keyboard sends nothing (firmware with the old log port?). " +
              "Flash the left half with the current build: BLE=1 ./fw-nrf/build-uf2 <name>, then ./hil flash left"
            : $"Port {device} is open, log only: this firmware has no debug stream (keys, screen, status). " +
              "Flash the left half with the current build: BLE=1 ./fw-nrf/build-uf2 <name>, then ./hil flash left";

    /// <summary>
    /// The firmware's boot line about its bootloader:
    /// <c>boot: bootloader 0.11.0, entry serial DFU (signed), debug port open</c>.
    /// </summary>
    internal static (string Version, string Entry, string DebugPort)? ParseBootloaderLine(string line)
    {
        var m = BootloaderLine().Match(line);
        return m.Success ? (m.Groups[1].Value, m.Groups[2].Value, m.Groups[3].Value) : null;
    }

    [GeneratedRegex(@"boot: bootloader (\S+), entry (.+?), debug port (\w+)")]
    private static partial Regex BootloaderLine();

    [GeneratedRegex(@"boot: (\S+) (left|right) (ble|usb-only)")]
    private static partial Regex IdentityLine();

    [GeneratedRegex(@"^\s*(\d+)\s+(.*)$")]
    private static partial Regex TimedLine();

    // ---- from the port (drained on the UI thread) -----------------------------

    /// <summary>Bytes as if read from the port (tests).</summary>
    internal void Feed(string text) => OnBytes(System.Text.Encoding.ASCII.GetBytes(text));

    /// <summary>One UI tick (tests call it instead of waiting for the timer).</summary>
    internal void Pump() => OnTick();

    /// <summary>The connection line shown at the top.</summary>
    internal string ConnectionText => _connection.Text;

    internal string LogText => _logWell.Text;

    internal string EventText => _eventWell.Text;

    /// <summary>A readout cell's value (tests).</summary>
    internal string Readout(string name) => _readout[name].Text ?? "";

    private uint Ms => (uint)_clock.ElapsedMilliseconds;

    /// <summary>The mouse pane as text (tests).</summary>
    internal string MouseText => _mouseWell.Text;

    private void OnTick()
    {
        while (_fromPort.TryDequeue(out var action)) action();
        foreach (var text in _mouseLog.Flush(Ms)) _mouseWell.Add(text, $"{DateTime.Now:HH:mm:ss}");

        if (_device != null && _mirror.Hello == null && DateTime.UtcNow - _connectedAt > HelloTimeout)
        {
            ShowConnection(_bytesSinceConnect == 0 ? LampState.Crit : LampState.Warn, NoStreamMessage(_device, _bytesSinceConnect));
            // Ask again now and then: the first request may have been lost.
            if (DateTime.UtcNow - _askedAt > HelloTimeout)
            {
                _askedAt = DateTime.UtcNow;
                _port.Send(DebugProtocol.DebugOn);
            }
        }
        if (_bootArmedAt != default && DateTime.UtcNow - _bootArmedAt > BootArmTime)
        {
            _bootArmedAt = default;
            _boot.Text = BootText;
        }

        _mirror.UpdateDisplays();
        var changed = false;
        for (var i = 0; i < _displayVersions.Length; i++)
        {
            var v = _mirror.Displays[i].Version;
            changed |= v != _displayVersions[i];
            _displayVersions[i] = v;
        }
        if (changed) _view.InvalidateVisual();
        _statusAge.Text = _mirror.Status == null
            ? "STATUS · NO #S YET"
            : $"STATUS · #S {(DateTime.UtcNow - _statusAt).TotalSeconds:0.0} S AGO";
    }

    private void OnConnected(string device)
    {
        _device = device;
        _connectedAt = DateTime.UtcNow;
        _askedAt = _connectedAt;
        _bytesSinceConnect = 0;
        _warnedUnconfirmed = false;
        _splitter.Reset();
        AddLog($"──── connected to {device} ────");
        ShowConnection(LampState.Warn, $"Port open: {device}, waiting for the debug stream…");
    }

    private void OnDisconnected(string why)
    {
        if (_device != null) AddLog($"──── {why} ────");
        _device = null;
        _splitter.Reset();
        _mirror.Disconnected();
        _view.SetHardwareHeld(0);
        _view.Dimmed = true;
        ShowConnection(LampState.Crit, $"Not connected: {why}. " + (_port.IsRunning ? "Retrying…" : ""));
        RefreshStatus();
    }

    private void OnBytes(byte[] bytes)
    {
        _bytesSinceConnect += bytes.Length;
        foreach (var line in _splitter.Feed(bytes))
        {
            if (StudioLine.IsStudio(line))
            {
                StudioLineReceived?.Invoke(line);
                continue;
            }
            var e = DebugProtocol.Parse(line);
            if (e is DebugLog log)
            {
                if (log.Text.Length > 0) AddLog(log.Text);
                continue;
            }
            _mirror.Apply(e);
            OnEvent(e);
        }
    }

    /// <summary>The views that follow an event the mirror took.</summary>
    private void OnEvent(DebugEvent e)
    {
        switch (e)
        {
            case DebugHello hello:
                _view.Dimmed = false;
                ShowConnection(LampState.Good, $"Connected: {hello.Name} ({hello.Half} half, {hello.Build}) on {_device}");
                break;
            case DebugKeys:
                _view.SetHardwareHeld(_mirror.Held);
                break;
            case DebugReport:
                _hostWell.SetText(_mirror.Host.Text);
                RefreshStatus();
                break;
            case DebugMouse mouse:
                _lastMouse = mouse.Report;
                foreach (var text in _mouseLog.Add(mouse.Report, Ms)) _mouseWell.Add(text, $"{DateTime.Now:HH:mm:ss}");
                RefreshStatus();
                break;
            case DebugStatus:
                _statusAt = DateTime.UtcNow;
                SyncToggles();
                if (_capture.IsChecked) EnsureMuted();
                RefreshStatus();
                break;
        }
    }

    /// <summary>Driving from this PC's keyboard: the Corne must not type, or its keys would come back here.</summary>
    private void EnsureMuted()
    {
        if (_mirror.Status is not { Mute: false }) return;
        if (DateTime.UtcNow - _muteSentAt < TimeSpan.FromSeconds(1)) return; // already asked
        _muteSentAt = DateTime.UtcNow;
        Send(DebugProtocol.Mute);
    }

    private void SyncToggles()
    {
        if (_mirror.Status is not { } s) return;
        _syncing = true;
        _trace.IsLatched = s.Trace;
        _mute.IsLatched = s.Mute;
        _syncing = false;
        StateChanged?.Invoke();
    }

    private void AddLog(string line)
    {
        if (IdentityLine().Match(line) is { Success: true } id) _bootIdentity = (id.Groups[1].Value, id.Groups[2].Value);
        if (ParseBootloaderLine(line) is { } boot)
        {
            var (name, half) = _bootIdentity ?? (_mirror.Hello is { } h ? (h.Name, h.Half) : ("", "left"));
            LastBoot = new BootFacts(name, half, boot.Version, boot.Entry, boot.DebugPort, DateTime.Now);
            _bootloader = boot.Version + (boot.Entry.Contains("signed", StringComparison.OrdinalIgnoreCase) ? " signed" : " UF2");
            RefreshStatus();
        }
        var m = TimedLine().Match(line);
        if (m.Success) _logWell.Add(m.Groups[2].Value, m.Groups[1].Value);
        else _logWell.Add(line);
    }

    private void AddEvent(string line) => _eventWell.Add(line, $"{DateTime.Now:HH:mm:ss.fff}");

    private void ShowConnection(LampState state, string text)
    {
        _connection.State = state;
        _connection.Text = text;
        StateChanged?.Invoke();
    }

    private void RefreshLabels()
    {
        var labels = new string[Keymap.KeyCount];
        for (var i = 0; i < labels.Length; i++) labels[i] = _mirror.Label(i);
        _view.SetLabels(labels);
    }

    private void Set(string cell, string value, string tone = "ink") => _readout[cell].Tok(TextBlock.ForegroundProperty, tone).Text = value;

    private const string NothingTyped = "Nothing typed yet.";

    private void RefreshStatus()
    {
        _readout["Bootloader"].Text = _bootloader ?? "—";
        _readout["Mouse"].Text = _lastMouse?.Describe() ?? "—";
        if (_mirror.Status is not { } s)
        {
            foreach (var name in _readout.Keys.Where(name => name is not ("Bootloader" or "Mouse")))
                Set(name, "—");
            _hostWell.Placeholder = NothingTyped;
            return;
        }
        ShowKeys(s);
        ShowLink(s);
        ShowFlags(s);
        _hostWell.Placeholder = s.Mute ? "Muted: nothing reaches the PC." : NothingTyped;
    }

    /// <summary>Layer, modifiers and the report.</summary>
    private void ShowKeys(DebugStatus s)
    {
        var mods = HidReport.ModifierNames(s.Mods);
        var held = mods.Concat(_mirror.Report.KeyNames()).ToList();
        Set("Layer", $"{s.Layer} {_layerName(s.Layer)}");
        Set("Mods", mods.Count == 0 ? "—" : string.Join("+", mods));
        Set("Report", held.Count == 0 ? "—" : string.Join(" + ", held));
    }

    /// <summary>Output, the right half, batteries, radio and power.</summary>
    private void ShowLink(DebugStatus s)
    {
        if (s.Output == "usb") Set("Output", "USB", "good");
        else Set("Output", $"BT {s.Profile} {ProfileCell.MarkOf(s.HostConnected, s.ProfileOpen)}", s.HostConnected ? "good" : "warn");
        Set("Right half", RightHalf(s), s.Linked ? "good" : "warn");
        Set("Battery L", $"{s.BatteryLeft}%{(s.ChargingLeft ? " ⚡" : "")}");
        Set("Battery R", $"{s.BatteryRight}%{(s.ChargingRight ? " ⚡" : "")}");
        Set("Radio", s.RadioDbm is { } dbm ? $"{dbm:+0;-0;0} dBm" : "—");
        Set("Power", s.Power, s.Power == "active" ? "ink" : "warn");
    }

    /// <summary>Counters and on/off states.</summary>
    private void ShowFlags(DebugStatus s)
    {
        Set("WPM", $"{s.Wpm}");
        Set("Presses", $"{s.Presses}");
        Set("Caps word", OnOff(s.CapsWord));
        Set("OLED", OnOff(s.Lit));
        Set("Mute", OnOff(s.Mute), s.Mute ? "warn" : "ink");
        Set("Trace", OnOff(s.Trace));
        Set("Pointer", OnOff(s.PointerActive), s.PointerActive == true ? "good" : "ink");
    }

    private static string RightHalf(DebugStatus s)
    {
        if (s.Linked) return "linked";
        return s.Stage.Length > 0 ? s.Stage : "not linked";
    }

    /// <summary>"on", "off", or "—" when the firmware doesn't say.</summary>
    private static string OnOff(bool? value) => value switch
    {
        true => "on",
        false => "off",
        null => "—",
    };

    public void Dispose()
    {
        _timer.Stop();
        ReleaseAll();
        _port.Dispose();
        _mirror.Dispose();
    }
}

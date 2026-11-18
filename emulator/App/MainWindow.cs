using System.Diagnostics.CodeAnalysis;
using System.Net.Sockets;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Threading;
using Kb.HostProtocol;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>
/// The app's window: the top bar with the mode switch over the page of the current mode
/// (Emulator in MainWindow.Emulator.cs; Debug, Studio, Flash and Security are views of their own).
/// </summary>
[SuppressMessage("Design", "CA1001:Types that own disposable fields should be disposable",
    Justification = "A window: the socket server, the debug view and the emulator are disposed when it closes (Closed).")]
public sealed partial class MainWindow : Window
{
    private const int MaxLogLines = 300;
    private static readonly TimeSpan WpmWindow = TimeSpan.FromSeconds(10);

    private readonly EmulatedKeyboard _keyboard;

    internal EmulatedKeyboard Keyboard => _keyboard;
    internal CorneView View => _view;
    internal DebugView Debug => _debug;
    internal TopBar Bar => _bar;

    /// <summary>The Emulator's controls, for tests that drive them as a user would.</summary>
    internal EmulatorControls Controls { get; } = new();
    internal StudioView Studio => _studio!;
    internal FlashView Flash => _flash!;
    internal SecurityView Security => _security!;
    private StudioView? _studio;
    private FlashView? _flash;
    private SecurityView? _security;

    private readonly CorneView _view;
    private readonly DebugView _debug;
    private readonly TopBar _bar;
    private readonly FeatureFlags _features;
    private readonly Dictionary<AppMode, Control> _pages = new();
    private readonly HostText _host = new(EmulatedKeyboard.UsageName);
    private readonly LogWell _hostWell = new(textMode: true) { Placeholder = "" };
    private readonly LogWell _logWell = new(MaxLogLines);
    private readonly Dictionary<PhysicalKey, int> _physicalDown = new();
    private readonly int[] _displayVersions = [-1, -1];
    private readonly DispatcherTimer _timer;
    private readonly EmulatorSocketServer _socketServer;
    private readonly TextBox _notificationBox = new() { PlaceholderText = "Text as if sent by the OS" };
    private readonly Queue<DateTime> _presses = new();
    private readonly System.Collections.Concurrent.ConcurrentQueue<byte[]> _notifications = new();
    private readonly Chip _layerChip = new("Layer", "", accent: true);
    private readonly Chip _capsChip = new("Caps word", "off");
    private readonly Chip _heldChip = new("Held", "—");
    private readonly Chip _wpmChip = new("WPM", "0");
    private readonly Chip _mouseChip = new("Mouse", "off");
    private readonly PointerPad _pointer = new();
    private readonly MouseLog _mouseLog = new();
    private readonly Lamp[] _emuLamps = [new("BT"), new("Link"), new("USB"), new("Radio"), new("Power")];
    private readonly Lamp[] _dbgLamps = [new("Stream"), new("Mute"), new("Trace")];
    private readonly Lamp[] _studioLamps = [new("Studio locked"), new("Saved")];
    private uint _studioGeneration;
    private Action? _syncRadio;
    private Action? _syncPower;
    private Action? _syncBattery;
    private AppMode _mode = AppMode.Emulator;
    private int _ticks;
    private bool _capsWord;
    private bool _quiet;

    public MainWindow() : this(null)
    {
    }

    /// <param name="socketPath">Where kbnotify connects; default <see cref="EmulatorLink.DefaultSocketPath"/>.</param>
    /// <param name="hardwarePort">The real keyboard's log port for Debug; default finds it by USB ID.</param>
    /// <param name="clock">Milliseconds for the emulator; tests pass their own to move time.</param>
    /// <param name="features">Optional parts; default the ones read at startup (<see cref="App.Features"/>).</param>
    public MainWindow(string? socketPath, HardwarePort? hardwarePort = null, Func<uint>? clock = null, FeatureFlags? features = null)
    {
        _features = features ?? App.Features;
        _bar = new TopBar(Enum.GetValues<AppMode>().Where(m => m != AppMode.Security || _features.IsOn(FeatureFlags.Security)).ToList());
        _keyboard = new EmulatedKeyboard(clock);
        Title = "EMK";
        Width = 1440;
        Height = 940;
        MinWidth = 720;
        MinHeight = 700;
        FontFamily = BenchTheme.Ui;
        FontSize = 14;
        this.Tok(BackgroundProperty, "face");

        _view = new CorneView(_keyboard.KeyCount) { Displays = _keyboard.Displays };
        WireBoard();
        WireKeyboard();

        _debug = new DebugView(_keyboard.LayerName, _keyboard.KeyCount, hardwarePort);
        _debug.StateChanged += () =>
        {
            if (_mode == AppMode.Debug) RefreshLamps();
        };
        Content = BuildShell();
        RefreshLabels();
        RefreshStatus();
        UseStudioStorage();
        _keyboard.ReplayBoot(); // like switching the keyboard on
        _keyboard.UpdateDisplays(); // first frame right away instead of after the first timer refresh
        WireKeys();

        _timer = new DispatcherTimer(TimeSpan.FromMilliseconds(5), DispatcherPriority.Input, (_, _) => OnTick());
        _timer.Start();

        _socketServer = new EmulatorSocketServer(packet =>
            _notifications.Enqueue(packet), socketPath); // handled on the next tick, on the UI thread
        StartSocketServer();
        if (App.Theme is { Source.Length: > 0 } theme)
        {
            Log(theme.Source);
            Console.Error.WriteLine($"emk: theme {theme.Preference}; {theme.Source}");
        }

        Closed += (_, _) =>
        {
            _timer.Stop();
            _socketServer.Dispose();
            _debug.Dispose();
            _keyboard.Dispose();
        };
    }

    /// <summary>The emulated board's own controls: keys, power switches, USB cables, reset buttons.</summary>
    private void WireBoard()
    {
        _view.KeyPressed += pos => _keyboard.Press(pos);
        _view.KeyReleased += pos => _keyboard.Release(pos);
        _view.PowerToggled += (half, on) =>
        {
            _keyboard.SetHalfPower(half, on);
            Log($"{HalfName(half)} half switched {(on ? "on" : "off")}");
        };
        _view.UsbToggled += (half, on) =>
        {
            _keyboard.SetCharging(half, on);
            Log($"USB {(on ? "plugged into" : "pulled from")} the {HalfName(half)} half");
            _syncBattery?.Invoke();
        };
        _view.ResetPressed += half =>
        {
            _keyboard.ResetHalf(half);
            Log($"{HalfName(half)} half reset");
        };
    }

    /// <summary>What the firmware does, in the log and the controls.</summary>
    private void WireKeyboard()
    {
        _keyboard.ReportSent += OnReport;
        _keyboard.MouseReported += OnMouseReport;
        _keyboard.LayerChanged += layer =>
        {
            if (!_quiet) Log($"layer → {_keyboard.LayerName(layer)}");
            RefreshLabels();
        };
        _keyboard.CapsWordChanged += on =>
        {
            _capsWord = on;
            Log($"caps word {(on ? "on" : "off")}");
            RefreshStatus();
        };
        _keyboard.SystemAction += OnSystemAction;
        _keyboard.PairingEnded += result => Log(result switch
        {
            PairResult.Paired => "host: code typed, computer paired",
            PairResult.WrongCode => "pairing: wrong code typed, pairing failed",
            PairResult.Rejected => "pairing: Escape on the keyboard, rejected",
            _ => "pairing: no code typed within 60 s, rejected",
        });
        _keyboard.PowerChanged += OnPowerChanged;
    }

    private void OnMouseReport(MouseReport report, uint at)
    {
        var pressed = (byte)(report.Buttons & ~_pointer.State.Buttons);
        if (pressed != 0) _pointer.Flash(pressed);
        var hires = _keyboard.Mouse.Hires;
        foreach (var line in _mouseLog.Add(report, at, (hires & 1) != 0, (hires & 2) != 0)) Log(line);
        RefreshPointer();
    }

    private void OnSystemAction(string action)
    {
        if (action == "Radio power")
        {
            Log($"radio: {_keyboard.RadioDbm:+0;-0;0} dBm");
            _syncRadio?.Invoke();
        }
        else
        {
            Log($"system: {action}");
        }
    }

    private void OnPowerChanged(PowerState state)
    {
        Log(state switch
        {
            PowerState.Idle => "power: idle → OLEDs off",
            PowerState.Sleep => "power: deep sleep → Bluetooth off, OLED power cut; next key wakes (and is lost)",
            _ => "power: active",
        });
        RefreshLabels();
        _syncPower?.Invoke();
    }

    /// <summary>This PC's keyboard plays the emulated one (or drives the real one in Debug); Esc closes what asks.</summary>
    private void WireKeys()
    {
        AddHandler(KeyDownEvent, OnPhysicalKeyDown, RoutingStrategies.Tunnel);
        AddHandler(KeyUpEvent, OnPhysicalKeyUp, RoutingStrategies.Tunnel);
        AddHandler(KeyDownEvent, (_, e) =>
        {
            if (!e.Handled && e.Key == Key.Escape) e.Handled = HandleEscape();
        }, RoutingStrategies.Bubble);
        Deactivated += (_, _) =>
        {
            ReleaseAllPhysical();
            _debug.ReleasePhysical();
        };
    }

    private void StartSocketServer()
    {
        try
        {
            Log(_socketServer.Start()
                ? $"listening for kbnotify on {_socketServer.Path}"
                : $"another emulator owns {_socketServer.Path}; kbnotify goes there");
        }
        catch (Exception e) when (e is SocketException or IOException or UnauthorizedAccessException or ArgumentException)
        {
            Log($"kbnotify socket unavailable: {e.Message}");
        }
    }

    /// <summary>
    /// The emulator keeps Studio's saved edits in ~/.config/emk/emulator-keymap.txt
    /// (EMK_EMULATOR_KEYMAP: another file; tests use their own).
    /// </summary>
    private void UseStudioStorage()
    {
        var path = Environment.GetEnvironmentVariable("EMK_EMULATOR_KEYMAP") is { Length: > 0 } p
            ? p
            : Path.Combine(Path.GetDirectoryName(AppSettings.DefaultPath)!, "emulator-keymap.txt");
        var result = _keyboard.SetStudioStorage(Path.GetFullPath(path));
        Log(result switch
        {
            0 => $"studio: keymap edits kept in {path}",
            1 => $"studio: {path} is not a keymap file; ignored",
            2 => $"studio: edits in {path} would lock Studio out; dropped (file cleared)",
            _ => $"studio: storage {path} not usable",
        });
    }

    private static string HalfName(int half) => half == 0 ? "left" : "right";

    // ---- shell --------------------------------------------------------------------

    /// <summary>Top bar with the mode switch over the page of the current mode.</summary>
    private DockPanel BuildShell()
    {
        _pages[AppMode.Emulator] = BuildEmulator();
        _pages[AppMode.Debug] = _debug;
        var sources = new DeviceSources { Boot = () => _debug.LastBoot };
        _studio = new StudioView(_keyboard, _debug);
        _flash = new FlashView(sources);
        // Security is behind a feature flag: without it there is no page to reach.
        _security = _features.IsOn(FeatureFlags.Security) ? new SecurityView(sources) : null;
        WirePages(_studio, _flash, _security);
        _pages[AppMode.Studio] = _studio;
        _pages[AppMode.Flash] = _flash;
        if (_security != null) _pages[AppMode.Security] = _security;

        var host = new Panel();
        foreach (var page in _pages.Values) host.Children.Add(page);

        WireBar();

        var root = new DockPanel();
        DockPanel.SetDock(_bar, Dock.Top);
        root.Children.Add(_bar);
        root.Children.Add(host);
        ApplyMode();
        return root;
    }

    /// <summary>The pages tell the window when the top bar (lamps, mode) has to follow them.</summary>
    private void WirePages(StudioView studio, FlashView flash, SecurityView? security)
    {
        studio.StateChanged += () =>
        {
            if (_mode == AppMode.Studio) RefreshLamps();
        };
        flash.BeforeFlash = side => side == "left" && _debug.ReleasePort();
        flash.AfterFlash = _debug.ResumePort;
        flash.JobChanged += () => ApplyModeIf(AppMode.Flash);
        flash.Refreshed += () => ApplyModeIf(AppMode.Flash);
        if (security == null) return;
        security.OpenFlash += () => ShowMode(AppMode.Flash);
        security.Refreshed += () => ApplyModeIf(AppMode.Security);
    }

    private void ApplyModeIf(AppMode mode)
    {
        if (_mode == mode) ApplyMode();
    }

    /// <summary>The mode switch and the theme switch.</summary>
    private void WireBar()
    {
        _bar.Modes.SelectionChanged += index => ShowMode(_bar.ShownModes[index]);
        _bar.ThemeSwitch.SelectedIndex = (int)(App.Theme?.Preference ?? ThemePreference.System);
        _bar.ThemeSwitch.SelectionChanged += index =>
        {
            App.Theme?.Set((ThemePreference)index);
            if (App.Theme is { Preference: ThemePreference.System, Source.Length: > 0 } t) Log(t.Source);
        };
    }

    internal AppMode Mode => _mode;

    internal IReadOnlyList<string> ModeNames() => _bar.ShownModes.Select(m => m.ToString()).ToList();

    /// <summary>Show Debug (or the emulator).</summary>
    internal void ShowDebug(bool debug) => ShowMode(debug ? AppMode.Debug : AppMode.Emulator);

    internal void ShowMode(AppMode mode)
    {
        if (mode == _mode || !_pages.ContainsKey(mode)) return; // a hidden mode can't be shown
        var was = _mode;
        _mode = mode;
        _bar.Modes.SelectedIndex = _bar.ShownModes.ToList().IndexOf(mode);
        if (was == AppMode.Emulator)
        {
            ReleaseAllPhysical();
            _view.ReleaseLatched();
        }
        if (was == AppMode.Debug) _debug.Deactivate();
        if (was == AppMode.Studio) _studio?.Deactivate();
        if (mode == AppMode.Studio) _studio?.Activate();
        if (mode == AppMode.Debug) _debug.Activate();
        ApplyMode();
        if (mode == AppMode.Flash) _flash?.Refresh();
        if (mode == AppMode.Security) _security?.Refresh();
    }

    private void ApplyMode()
    {
        foreach (var (mode, page) in _pages) page.IsVisible = mode == _mode;
        _bar.SetLamps(_mode switch
        {
            AppMode.Emulator => _emuLamps,
            AppMode.Debug => _dbgLamps,
            AppMode.Flash => _flash?.TopLamps ?? [],
            AppMode.Security => _security?.TopLamps ?? [],
            _ => _studioLamps,
        });
        RefreshLamps();
    }

    private const string BoardName = "Corne v3";

    private void RefreshLamps()
    {
        switch (_mode)
        {
            case AppMode.Emulator:
                RefreshEmulatorLamps();
                break;
            case AppMode.Debug:
                _bar.Model = _debug.Mirror.Hello is { } hello ? $"{BoardName} · {hello.Name} · real keyboard" : $"{BoardName} · real keyboard";
                _dbgLamps[0].State = _debug.StreamState;
                _dbgLamps[1].State = _debug.Mirror.Status?.Mute == true ? LampState.Warn : LampState.Off;
                _dbgLamps[2].State = _debug.Mirror.Status?.Trace == true ? LampState.Accent : LampState.Off;
                break;
            case AppMode.Studio:
                RefreshStudioLamps(_studio!);
                break;
            default:
                var page = _mode == AppMode.Flash ? (DevicePage?)_flash : _security;
                _bar.Model = page?.Selected is { } name ? $"{BoardName} · {name}" : BoardName;
                break;
        }
    }

    private void RefreshEmulatorLamps()
    {
        _bar.Model = $"{BoardName} · emulated";
        var (bt, link, usb, radio, power) = (_emuLamps[0], _emuLamps[1], _emuLamps[2], _emuLamps[3], _emuLamps[4]);
        var onBt = _keyboard.Transport == Transport.Bluetooth;
        bt.Text = $"BT {_keyboard.Profile + 1} {ProfileMark()}";
        bt.State = onBt ? ProfileLamp() : LampState.Off;
        link.State = _keyboard.PeripheralConnected ? LampState.Good : LampState.Crit;
        usb.State = onBt ? LampState.Off : LampState.Good;
        radio.Text = $"Radio {_keyboard.RadioDbm:+0;-0;0} dBm";
        radio.State = LampState.Accent;
        (power.Text, power.State) = _keyboard.Power switch
        {
            PowerState.Idle => ("Power idle", LampState.Warn),
            PowerState.Sleep => ("Power sleep", LampState.Off),
            _ => ("Power active", LampState.Good),
        };
    }

    private void RefreshStudioLamps(StudioView studio)
    {
        var client = studio.Client;
        var target = studio.TargetIsKeyboard ? _debug.Mirror.Hello?.Name ?? "real keyboard" : "emulated";
        _bar.Model = $"{BoardName} · {target} · Studio";
        _studioLamps[0].Text = client.Unlocked ? "Studio unlocked" : "Studio locked";
        _studioLamps[0].State = StudioLamp(client, good: client.Unlocked);
        _studioLamps[1].Text = client.Dirty ? "Unsaved" : "Saved";
        _studioLamps[1].State = StudioLamp(client, good: !client.Dirty);
    }

    /// <summary>The selected profile's mark: ✓ connected, + open, ✗ paired but not connected.</summary>
    private string ProfileMark() => ProfileCell.MarkOf(_keyboard.ProfileConnected, _keyboard.ProfileOpen);

    private LampState ProfileLamp()
    {
        if (_keyboard.ProfileConnected) return LampState.Good;
        return _keyboard.ProfileOpen ? LampState.Accent : LampState.Warn;
    }

    /// <summary>A Studio lamp: off without Studio support, else good or warn.</summary>
    private static LampState StudioLamp(StudioClient client, bool good)
    {
        if (client.Support != StudioSupport.Yes) return LampState.Off;
        return good ? LampState.Good : LampState.Warn;
    }

    // ---- behaviour ----------------------------------------------------------------

    private void SendNotificationFromBox()
    {
        var text = _notificationBox.Text?.Trim();
        if (string.IsNullOrEmpty(text)) return;
        DeliverPackets("OS", NotificationPackets.Encode(text));
        _notificationBox.Text = "";
    }

    /// <summary>Raw HID packets from the computer, as the left half receives them.</summary>
    private void DeliverPackets(string source, IReadOnlyList<byte[]> packets)
    {
        foreach (var packet in packets)
        {
            var result = _keyboard.RawHid(packet);
            switch (result)
            {
                case RawHidResult.Delivered: Log($"{source} → notification shown on the right half"); break;
                case RawHidResult.NotLinked: Log($"{source} → notification dropped: right half not linked"); break;
                case RawHidResult.Cleared: Log($"{source} → notification cleared"); break;
                case RawHidResult.Asleep: Log($"{source} → lost: keyboard in deep sleep (Bluetooth off)"); break;
            }
        }
    }

    /// <summary>One timer tick (tests call it with their own clock).</summary>
    internal void Tick() => OnTick();

    /// <summary>The status chips: layer, caps word, held keys, WPM.</summary>
    internal (string Layer, string CapsWord, string Held, string Wpm, string Mouse) Chips =>
        (_layerChip.Value, _capsChip.Value, _heldChip.Value, _wpmChip.Value, _mouseChip.Value);

    private void OnTick()
    {
        while (_notifications.TryDequeue(out var packet)) DeliverPackets("kbnotify", [packet]);
        _keyboard.Tick();
        if (++_ticks % 10 != 0) return; // displays at ~20 Hz

        _keyboard.UpdateDisplays();
        var changed = false;
        for (var i = 0; i < _displayVersions.Length; i++)
        {
            var v = _keyboard.Displays[i].Version;
            changed |= v != _displayVersions[i];
            _displayVersions[i] = v;
        }
        if (changed) _view.InvalidateVisual();
        foreach (var line in _mouseLog.Flush(_keyboard.NowMs)) Log(line);
        RefreshPointer();
        _studio?.Pump();
        if (_keyboard.StudioGeneration != _studioGeneration)
        {
            _studioGeneration = _keyboard.StudioGeneration;
            RefreshLabels(); // Studio changed a binding: the legends follow
        }
        if (_ticks % 100 == 0)
        {
            RefreshWpm();
            RefreshLamps();
        }
    }

    private void OnReport(HidReport previous, HidReport next)
    {
        _host.Apply(previous, next);
        _hostWell.SetText(_host.Text);
        for (var usage = 4; usage < 0xE0; usage++)
        {
            if (next.Contains(usage) && !previous.Contains(usage)) _presses.Enqueue(DateTime.UtcNow);
        }
        if (!_quiet) Log(HardwareMirror.Describe(previous, next));
        RefreshStatus();
    }

    /// <summary>Words per minute over the last 10 s (5 key presses = a word).</summary>
    private void RefreshWpm()
    {
        var since = DateTime.UtcNow - WpmWindow;
        while (_presses.TryPeek(out var t) && t < since) _presses.Dequeue();
        _wpmChip.Value = $"{(int)Math.Round(_presses.Count / 5.0 * (60 / WpmWindow.TotalSeconds))}";
    }

    private void Log(string line)
    {
        if (line.Length == 0) return;
        _logWell.Add(line, $"{DateTime.Now:HH:mm:ss.fff}");
    }

    /// <summary>Esc: close what is asking (a confirm panel, an armed button). True if something closed.</summary>
    internal bool HandleEscape() => _mode switch
    {
        AppMode.Flash => _flash?.Escape() == true,
        AppMode.Studio => _studio?.Escape() == true,
        AppMode.Debug => _debug.Escape(),
        _ => false,
    };

    /// <summary>The host pane as text (tests).</summary>
    internal string HostText => _hostWell.Text;

    /// <summary>The events pane as text (tests).</summary>
    internal string EventText => _logWell.Text;

    private void RefreshLabels()
    {
        var labels = new string[_keyboard.KeyCount];
        for (var i = 0; i < labels.Length; i++) labels[i] = _keyboard.Label(i);
        _view.SetLabels(labels);
        RefreshStatus();
    }

    private void RefreshStatus()
    {
        var r = _keyboard.LastReport;
        var held = HidReport.ModifierNames(r.Mods).Concat(r.KeyNames()).ToList();
        _layerChip.Value = _keyboard.LayerName(_keyboard.Layer);
        _capsChip.Value = _capsWord ? "on" : "off";
        _heldChip.Value = held.Count == 0 ? "—" : string.Join(" + ", held);
    }

    // ---- this PC's keyboard ---------------------------------------------------------

    private void OnPhysicalKeyDown(object? sender, KeyEventArgs e)
    {
        if (e.Source is TextBox { IsReadOnly: false }) return; // typing a notification
        if (RoutePhysicalKey(e.PhysicalKey, true)) e.Handled = true;
        // In the Emulator, typing belongs to the emulated keyboard: a button
        // that has focus from a click must not fire on Enter or Space.
        else if (_mode == AppMode.Emulator && e.Key is Key.Enter or Key.Space) e.Handled = true;
    }

    private void OnPhysicalKeyUp(object? sender, KeyEventArgs e)
    {
        if (RoutePhysicalKey(e.PhysicalKey, false)) e.Handled = true;
    }

    /// <summary>
    /// A key on this PC's keyboard: in Emulator mode it plays its Corne
    /// position; in Debug it goes to the real keyboard (when driving);
    /// elsewhere it is left alone. A key let go after leaving Emulator still
    /// releases what it pressed. True if used.
    /// </summary>
    internal bool RoutePhysicalKey(PhysicalKey key, bool down)
    {
        if (!down)
        {
            if (_mode == AppMode.Debug && _debug.OnPhysicalKey(key, false)) return true;
            if (!_physicalDown.Remove(key, out var held)) return false;
            _view.SetPressed(held, false);
            _keyboard.Release(held);
            return true;
        }
        if (_mode == AppMode.Debug) return _debug.OnPhysicalKey(key, true);
        if (_mode != AppMode.Emulator) return false;
        if (PhysicalLayout.Position(key) is not { } pos) return false;
        if (!_physicalDown.TryAdd(key, pos)) return true; // OS autorepeat
        _view.SetPressed(pos, true);
        _keyboard.Press(pos);
        return true;
    }

    private void ReleaseAllPhysical()
    {
        foreach (var (_, pos) in _physicalDown)
        {
            _view.SetPressed(pos, false);
            _keyboard.Release(pos);
        }
        _physicalDown.Clear();
    }
}

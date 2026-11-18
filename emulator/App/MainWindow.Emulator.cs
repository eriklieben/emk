using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Primitives;
using Avalonia.Input;
using Avalonia.Layout;
using Avalonia.Media;
using Kb.HostProtocol;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>The Emulator page: the emulated keyboard, its rack of modules and the output panes.</summary>
public sealed partial class MainWindow
{
    // ---- emulator mode --------------------------------------------------------------

    private ContentControl BuildEmulator()
    {
        // Tray: the keyboard, its status chips, and what changes how it's drawn.
        var header = new DockPanel();
        var chips = W.Wrap(6, 6, _layerChip, _capsChip, _heldChip, _wpmChip, _mouseChip);
        chips.HorizontalAlignment = HorizontalAlignment.Right;
        DockPanel.SetDock(chips, Dock.Right);
        header.Children.Add(chips);
        header.Children.Add(W.Lbl("Keyboard · click to press, right-click to latch"));

        var upright = Controls.Upright = new Toggle("Upright screens", true);
        upright.CheckedChanged += on => _keyboard.SetPortraitScreens(on);
        var release = Controls.ReleaseLatched = new BenchButton("Release latched keys");
        release.Click += () => _view.ReleaseLatched();
        var help = W.Hint("Type on your keyboard: Tab/Caps/Shift = outer column, Super Alt Space | AltGr Menu RCtrl = thumbs. " +
                          "Untick Upright screens for the classic sideways status screen.");
        help.VerticalAlignment = VerticalAlignment.Center;
        var footer = new DockPanel();
        var footerLeft = W.Row(14, upright, release);
        DockPanel.SetDock(footerLeft, Dock.Left);
        help.Margin = new Thickness(16, 0, 0, 0);
        footer.Children.Add(footerLeft);
        footer.Children.Add(help);

        _emuTray = KeyboardTray(header, _view, footer);
        _emuSide = Strips.Side(new ScrollViewer { Content = new ChipPanel(SizeReport.TryLoad()), HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled });
        _emuModules = [BatteryModule(), PowerModule(), new BluetoothModule(_keyboard, Controls, Log, RefreshLamps).Build(), RadioModule(), MessageModule()];

        var clear = Controls.ClearOutput = new BenchButton("Clear output");
        clear.Click += () =>
        {
            _host.Clear();
            _hostWell.Clear();
            _logWell.Clear();
        };
        _emuPanes = [Pane(W.Lbl("Host receives · US layout"), HostWithPointer()), Pane(W.Split(W.Lbl("Events"), clear), _logWell)];
        ArrangeEmulator();
        return _emuPage;
    }

    /// <summary>What the computer gets typed, and beside it the computer's screen with the mouse pointer.</summary>
    private Grid HostWithPointer()
    {
        var reset = Controls.ResetPointer = new BenchButton("Reset");
        reset.Click += () =>
        {
            _keyboard.ResetMousePosition();
            RefreshPointer();
        };
        var smooth = Controls.SmoothScrolling = new Toggle("Smooth scrolling");
        smooth.CheckedChanged += on =>
        {
            // The computer sets the Resolution Multiplier: wheel and pan in 1/16 detents.
            _keyboard.SetSmoothScrolling(on, on);
            Log($"computer: smooth scrolling {(on ? "on (1/16 detents)" : "off (whole detents)")}");
        };
        var head = W.Split(W.Lbl("Pointer"), reset);
        head.Margin = new Thickness(0, 0, 0, 6);
        var pointer = new DockPanel { Width = 300, Margin = new Thickness(12, 0, 0, 0) };
        DockPanel.SetDock(head, Dock.Top);
        DockPanel.SetDock(smooth, Dock.Bottom);
        smooth.Margin = new Thickness(0, 6, 0, 0);
        pointer.Children.Add(head);
        pointer.Children.Add(smooth);
        pointer.Children.Add(_pointer);
        var grid = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto") };
        Grid.SetColumn(pointer, 1);
        grid.Children.Add(_hostWell);
        grid.Children.Add(pointer);
        return grid;
    }

    /// <summary>The pointer preview and the Mouse chip follow the emulator.</summary>
    private void RefreshPointer()
    {
        var m = _keyboard.Mouse;
        _pointer.State = m;
        _mouseChip.Value = MouseChip(m);
    }

    private static string MouseChip(MouseState m)
    {
        if (m.Moving) return "moving";
        if (m.Buttons != 0) return MouseReport.ButtonNames(m.Buttons) + " held";
        return m.PointerActive ? "on" : "off";
    }

    /// <summary>The pointer preview (tests).</summary>
    internal PointerPad Pointer => _pointer;

    private Control _emuTray = null!;
    private Control _emuSide = null!;
    private Control[] _emuModules = [];
    private Control[] _emuPanes = [];
    private readonly ContentControl _emuPage = new();
    private int _narrowColumns;
    private double _width = 1440;

    /// <summary>The keyboard tray in a narrow window: it keeps priority.</summary>
    private double TrayHeight => Math.Max(300, _width * 0.45);

    /// <summary>Rack columns of the narrow layout; 0 = wide (tests read it).</summary>
    internal int NarrowColumns => _narrowColumns;

    /// <summary>Place the Emulator's parts for the window's width.</summary>
    private void ArrangeEmulator()
    {
        foreach (var part in _emuModules.Concat(_emuPanes).Append(_emuTray).Append(_emuSide)) Responsive.Detach(part);
        _emuPage.Content = null;
        if (_narrowColumns > 0)
        {
            // Narrow: the keyboard first and tall, then the rack, the chip, the panes.
            _emuTray.Height = TrayHeight;
            _emuSide.Height = double.NaN;
            var rack = Responsive.Wrap(_emuModules, _narrowColumns, new Thickness(16, 12));
            var rackBox = new Border { BorderThickness = new Thickness(0, 1, 0, 0), Child = rack }.Tok(Border.BorderBrushProperty, "ln");
            var side = new Border { BorderThickness = new Thickness(0, 0, 0, 1), Child = _emuSide }.Tok(Border.BorderBrushProperty, "ln");
            var panes = PaneGrid(_emuPanes, "*", stacked: true);
            _emuPage.Content = Responsive.Stack(_emuTray, rackBox, side, panes);
            return;
        }
        _emuTray.Height = double.NaN;
        var main = new Grid { ColumnDefinitions = new ColumnDefinitions("*,300") };
        Grid.SetColumn(_emuSide, 1);
        main.Children.Add(_emuTray);
        main.Children.Add(_emuSide);

        var wide = new Grid { ColumnDefinitions = new ColumnDefinitions("180,210,*,190,280") };
        for (var i = 0; i < _emuModules.Length; i++)
        {
            var module = W.Module(_emuModules[i], last: i == _emuModules.Length - 1);
            Grid.SetColumn(module, i);
            wide.Children.Add(module);
        }
        var rackWide = new Border { BorderThickness = new Thickness(0, 1, 0, 1), Child = wide }.Tok(Border.BorderBrushProperty, "ln");

        var page = new Grid { RowDefinitions = new RowDefinitions("3*,Auto,1.5*") };
        var paneGrid = PaneGrid(_emuPanes, "1*,1.45*", stacked: false);
        Grid.SetRow(rackWide, 1);
        Grid.SetRow(paneGrid, 2);
        page.Children.Add(main);
        page.Children.Add(rackWide);
        page.Children.Add(paneGrid);
        _emuPage.Content = page;
    }

    /// <summary>The window's width changed: pick the layout for it, in every mode.</summary>
    internal void ApplyWidth(double width)
    {
        var columns = Responsive.NarrowColumns(width);
        _width = width;
        if (columns > 0) _emuTray.Height = TrayHeight;
        if (columns == _narrowColumns) return;
        _narrowColumns = columns;
        ArrangeEmulator();
        _bar.Narrow = columns > 0;
        _debug.Arrange(columns);
        _studio?.Arrange(columns);
        if (_flash != null) _flash.NarrowColumns = columns;
        if (_security != null) _security.NarrowColumns = columns;
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == ClientSizeProperty && _emuModules.Length > 0) ApplyWidth(ClientSize.Width);
    }

    /// <summary>The recessed tray with a header, the keyboard scaled to fit, and a footer.</summary>
    internal static Control KeyboardTray(Control header, CorneView view, Control? footer)
    {
        var grid = new Grid { RowDefinitions = new RowDefinitions("Auto,*,Auto"), RowSpacing = 6 };
        // Fills the tray, aspect kept: as wide or as tall as the tray allows.
        var keyboard = new Viewbox { Child = view, Stretch = Stretch.Uniform, StretchDirection = StretchDirection.Both, MinHeight = 160 };
        Grid.SetRow(keyboard, 1);
        grid.Children.Add(header);
        grid.Children.Add(keyboard);
        if (footer != null)
        {
            Grid.SetRow(footer, 2);
            grid.Children.Add(footer);
        }
        return new Panel { Children = { Strips.Tray(grid), new Recess() } };
    }

    /// <summary>An output pane: its header over a log well.</summary>
    internal static DockPanel Pane(Control header, Control well)
    {
        var pane = new DockPanel();
        header.Margin = new Thickness(0, 0, 0, 6);
        header.MinHeight = 22;
        DockPanel.SetDock(header, Dock.Top);
        pane.Children.Add(header);
        pane.Children.Add(well);
        return pane;
    }

    /// <summary>Output panes side by side (CSS <c>.panes</c>), or under each other in a narrow window.</summary>
    internal static Control PaneGrid(IReadOnlyList<Control> panes, string columns, bool stacked)
    {
        var grid = new Grid { Margin = new Thickness(stacked ? 16 : 22, 12, stacked ? 16 : 22, 16), MinHeight = 140 };
        if (stacked)
        {
            grid.RowSpacing = 14;
            for (var i = 0; i < panes.Count; i++)
            {
                grid.RowDefinitions.Add(new RowDefinition(new GridLength(240)));
                Grid.SetRow(Responsive.Detach(panes[i]), i);
                Grid.SetColumn(panes[i], 0);
                grid.Children.Add(panes[i]);
            }
            return grid;
        }
        grid.ColumnDefinitions = new ColumnDefinitions(columns);
        grid.ColumnSpacing = 14;
        for (var i = 0; i < panes.Count; i++)
        {
            Grid.SetRow(Responsive.Detach(panes[i]), 0);
            Grid.SetColumn(panes[i], i);
            grid.Children.Add(panes[i]);
        }
        return grid;
    }

    private StackPanel BatteryModule()
    {
        var cells = W.Row(16);
        var updates = new List<Action>();
        for (var half = 0; half < 2; half++)
        {
            var h = half;
            var gauge = new Gauge { HorizontalAlignment = HorizontalAlignment.Center };
            Controls.Gauges[h] = gauge;
            var value = W.Head("100%", 19);
            value.HorizontalAlignment = HorizontalAlignment.Center;
            var name = W.Lbl(half == 0 ? "Left" : "Right", 11);
            name.HorizontalAlignment = HorizontalAlignment.Center;
            var slider = new Slider { Minimum = 0, Maximum = 100, Value = 100, Width = 64, Focusable = false, Margin = new Thickness(0, -6, 0, -8) };
            Controls.Sliders[h] = slider;
            Controls.BatteryText[h] = value;
            var syncing = false;
            void Set(int pct)
            {
                if (syncing) return;
                syncing = true;
                gauge.Value = pct;
                slider.Value = pct;
                value.Text = $"{pct}%";
                _keyboard.SetBattery(h, pct);
                syncing = false;
            }
            gauge.ValueChanged += Set;
            slider.ValueChanged += (_, e) => Set((int)e.NewValue);
            updates.Add(() => name.Text = (h == 0 ? "LEFT" : "RIGHT") + (_view.IsUsbPlugged(h) ? " ⚡" : ""));
            var cell = W.Column(3, gauge, value, name, slider);
            cells.Children.Add(cell);
        }
        _syncBattery = () => updates.ForEach(u => u());
        return W.Column(9, W.Lbl("Battery"), cells);
    }

    private StackPanel PowerModule()
    {
        var pill = Controls.PowerPill = new Pill("● Active", "good");
        var timeouts = Controls.PowerPreset = new Segmented(["Normal", "Quick", "Never"]);
        var hint = Controls.PowerHint = W.Hint("");
        void Describe()
        {
            hint.Text = timeouts.SelectedIndex switch
            {
                1 => "Idle 5 s → OLEDs off\nSleep 20 s → Bluetooth off",
                2 => "Never idles or sleeps",
                _ => "Idle 30 s → OLEDs off\nSleep 1 h → Bluetooth off",
            };
        }
        timeouts.SelectionChanged += index =>
        {
            var (idle, sleep) = index switch
            {
                1 => (TimeSpan.FromSeconds(5), (TimeSpan?)TimeSpan.FromSeconds(20)),
                2 => (TimeSpan.FromDays(1), null),
                _ => (TimeSpan.FromSeconds(30), TimeSpan.FromHours(1)),
            };
            _keyboard.SetPowerTimeouts(idle, sleep);
            Describe();
        };
        Describe();
        _syncPower = () =>
        {
            var (text, tone) = _keyboard.Power switch
            {
                PowerState.Idle => ("● Idle · OLEDs off", "warn"),
                PowerState.Sleep => ("● Deep sleep", "crit"),
                _ => ("● Active", "good"),
            };
            pill.Set(text, tone);
        };
        var boot = Controls.ReplayStartUp = new BenchButton("Replay start-up");
        boot.Click += () =>
        {
            _keyboard.ReplayBoot();
            Log("start-up replayed: the animation plays on both screens");
        };
        return W.Column(9, W.Lbl("Power"), pill, timeouts, hint, boot);
    }

    private StackPanel RadioModule()
    {
        var levels = EmulatedKeyboard.RadioLevels;
        var readout = W.Head("", 30);
        var unit = W.Hint("dBm", 14);
        unit.VerticalAlignment = VerticalAlignment.Bottom;
        unit.Margin = new Thickness(0, 0, 0, 5);
        var segment = Controls.Radio = new Segmented(levels.Select((l, i) => $"{l:+0;-0;0}" + (i == levels.Count - 1 ? " dBm" : "")).ToArray());
        Controls.RadioReadout = readout;
        void Sync()
        {
            var dbm = _keyboard.RadioDbm;
            readout.Text = $"{dbm:+0;-0;0}";
            segment.SelectedIndex = levels.ToList().IndexOf(dbm);
            RefreshLamps();
        }
        segment.SelectionChanged += index =>
        {
            _quiet = true;
            try
            {
                if (!_keyboard.SetRadioDbm(levels[index])) Log("radio: no Radio key on the nav layer");
            }
            finally
            {
                _quiet = false;
            }
            Sync();
        };
        _syncRadio = Sync;
        Sync();
        return W.Column(9, W.Lbl("Radio · left half"), W.Row(6, readout, unit), segment,
            W.Hint("Right half fixed at 0 dBm. The left OLED shows it for 2.5 s."));
    }

    private StackPanel MessageModule()
    {
        _notificationBox.FontSize = 13;
        _notificationBox.MinHeight = 30;
        _notificationBox.Padding = new Thickness(8, 5);
        _notificationBox.KeyDown += (_, e) =>
        {
            if (e.Key != Key.Enter) return;
            SendNotificationFromBox();
            e.Handled = true;
        };
        var send = Controls.Send = new BenchButton("Send to keyboard", ButtonKind.Primary);
        Controls.Message = _notificationBox;
        send.Click += SendNotificationFromBox;
        var clear = Controls.ClearMessage = new BenchButton("Clear");
        clear.Click += () => DeliverPackets("OS", [NotificationPackets.Clear()]);
        return W.Column(9, W.Lbl("Message to the right screen"), _notificationBox, W.Row(8, send, clear),
            W.Hint("Shown for 15 s. From a terminal:"), W.Mono("dotnet run --project emulator/KbNotify -- \"text\"", 11, "mut"));
    }
}

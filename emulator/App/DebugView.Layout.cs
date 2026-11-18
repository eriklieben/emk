using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Kb.HostProtocol;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>Debug mode's layout: the connection strip, the live keyboard, the readout and the panes.</summary>
public sealed partial class DebugView
{
    private void BuildLayout()
    {
        WireStripControls();
        BuildStrip();
        // Tray with the live keyboard.
        var hint = W.Hint("The left screen is the keyboard's own frame. The right one is drawn here from the forwarded status. " +
                          "Click a key to press it on the keyboard, right-click to latch it.");
        _tray = MainWindow.KeyboardTray(W.Lbl("Live keys · lit = held on the board"), _view, hint);
        _side = Strips.Side(W.Column(12, W.Split(_statusAge, W.Lbl("s to refresh")), BuildReadout(), BuildActions()));
        BuildPanes();
        Arrange(0);
    }

    /// <summary>What the connection strip's buttons and toggles do.</summary>
    private void WireStripControls()
    {
        _trace.LatchedChanged += _ =>
        {
            if (_syncing || _mirror.Status == null) return;
            Send(DebugProtocol.Trace);
        };
        _mute.LatchedChanged += on =>
        {
            if (_syncing || _mirror.Status == null) return;
            // Typing again while driving from this PC's keyboard would loop.
            if (!on) _capture.IsChecked = false;
            Send(DebugProtocol.Mute);
            _muteSentAt = DateTime.UtcNow;
        };
        _capture.CheckedChanged += on =>
        {
            if (on) EnsureMuted();
            else ReleasePhysical();
        };
        _follow.CheckedChanged += on =>
        {
            _logWell.Follow = on;
            _logTitle.Text = on ? "FIRMWARE LOG · FOLLOWING" : "FIRMWARE LOG · PAUSED";
        };
        _boot.Click += OnBootClicked;
        _connect.Click += OnConnectClicked;
    }

    /// <summary>Two clicks: it restarts the keyboard (to flash it).</summary>
    private void OnBootClicked()
    {
        if (DateTime.UtcNow - _bootArmedAt < BootArmTime)
        {
            Send(DebugProtocol.Bootloader);
            _bootArmedAt = default;
            _boot.Text = BootText;
            return;
        }
        _bootArmedAt = DateTime.UtcNow;
        _boot.Text = "Click again: restart into the bootloader";
    }

    private void OnConnectClicked()
    {
        if (_port.IsRunning)
        {
            ReleaseAll();
            _port.Stop();
            _connect.Text = ConnectText;
            OnDisconnected("disconnected (button)");
        }
        else
        {
            _connect.Text = DisconnectText;
            _port.Start();
        }
    }

    private void BuildStrip()
    {
        var conn = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), RowDefinitions = new RowDefinitions("Auto,Auto"), ColumnSpacing = 16, RowSpacing = 8 };
        _connRight = W.Row(16, _follow, _capture, _connect);
        Grid.SetColumn(_connRight, 1);
        conn.Children.Add(_connection);
        conn.Children.Add(_connRight);
        _strip = Strips.Conn(conn);
    }

    /// <summary>The status readout: a cell per value.</summary>
    private Border BuildReadout()
    {
        var readout = _readoutGrid;
        string[] cells = ["Layer", "Mods", "Output", "Right half", "Battery L", "Battery R", "Radio", "Power",
                          "WPM", "Presses", "Caps word", "Report", "OLED", "Mute", "Trace", "Bootloader", "Pointer", "Mouse"];
        for (var i = 0; i < cells.Length; i++)
        {
            var value = W.Head("—", 17);
            value.TextWrapping = Avalonia.Media.TextWrapping.NoWrap;
            value.TextTrimming = Avalonia.Media.TextTrimming.CharacterEllipsis;
            _readout[cells[i]] = value;
            var cell = new Border
            {
                Padding = new Thickness(10, 6),
                Child = W.Column(1, W.Lbl(cells[i]), value),
            }.Tok(Border.BorderBrushProperty, "ln");
            _readoutCells.Add(cell);
            readout.Children.Add(cell);
        }
        LayoutReadout(4);
        return new Border { BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(3), Child = readout }
            .Tok(Border.BorderBrushProperty, "ln").Tok(Border.BackgroundProperty, "face-hi");
    }

    private WrapPanel BuildActions()
    {
        var status = Controls.Status = new BenchButton("Status", letter: "s");
        status.Click += () => Send(DebugProtocol.Status);
        var dump = Controls.Dump = new BenchButton("Replay log", letter: "d");
        dump.Click += () => Send(DebugProtocol.Dump);
        var resync = Controls.Resync = new BenchButton("Resync");
        resync.Click += () => Send(DebugProtocol.DebugOn);
        var release = Controls.Release = new BenchButton("Release latched");
        release.Click += () => _view.ReleaseLatched();
        var clear = Controls.Clear = new BenchButton("Clear panes");
        clear.Click += () =>
        {
            _mirror.Host.Clear();
            _hostWell.Clear();
            _eventWell.Clear();
            _logWell.Clear();
            _mouseWell.Clear();
        };
        return W.Wrap(7, 7, status, _trace, _mute, dump, resync, release, clear, _boot);
    }

    /// <summary>The output panes: what the computer receives (keys and mouse), events, the firmware log.</summary>
    private void BuildPanes()
    {
        _hostWell.Placeholder = NothingTyped;
        var computer = new Grid { RowDefinitions = new RowDefinitions("*,Auto,90"), RowSpacing = 6 };
        var mouseLabel = W.Lbl("Mouse");
        Grid.SetRow(mouseLabel, 1);
        Grid.SetRow(_mouseWell, 2);
        computer.Children.Add(_hostWell);
        computer.Children.Add(mouseLabel);
        computer.Children.Add(_mouseWell);
        _mouseWell.Placeholder = "No mouse reports yet.";
        _panes = [MainWindow.Pane(W.Lbl("Computer receives"), computer), MainWindow.Pane(W.Lbl("Events"), _eventWell), MainWindow.Pane(_logTitle, _logWell)];
    }

    private Control _strip = null!, _connRight = null!, _tray = null!, _side = null!;
    private Control[] _panes = [];
    private readonly Grid _readoutGrid = new();
    private readonly List<Border> _readoutCells = new();
    private int _columns = -1;

    /// <summary>Readout columns (4 wide, 2 narrow; tests).</summary>
    internal int ReadoutColumns => _readoutGrid.ColumnDefinitions.Count;

    private void LayoutReadout(int columns)
    {
        _readoutGrid.ColumnDefinitions.Clear();
        _readoutGrid.RowDefinitions.Clear();
        for (var c = 0; c < columns; c++) _readoutGrid.ColumnDefinitions.Add(new ColumnDefinition(GridLength.Star));
        var rows = (_readoutCells.Count + columns - 1) / columns;
        for (var r = 0; r < rows; r++) _readoutGrid.RowDefinitions.Add(new RowDefinition(GridLength.Auto));
        for (var i = 0; i < _readoutCells.Count; i++)
        {
            var (row, col) = (i / columns, i % columns);
            _readoutCells[i].BorderThickness = new Thickness(0, 0, col == columns - 1 ? 0 : 1, row == rows - 1 ? 0 : 1);
            SetRow(_readoutCells[i], row);
            SetColumn(_readoutCells[i], col);
        }
    }

    /// <summary>
    /// Place the parts for the window's width: <paramref name="narrowColumns"/>
    /// 0 is the wide layout; otherwise the readout has 2 columns, the side
    /// panel goes under the keyboard and the panes stack.
    /// </summary>
    public void Arrange(int narrowColumns)
    {
        var narrow = narrowColumns > 0;
        LayoutReadout(narrowColumns is 0 or >= 3 ? 4 : 2);
        if ((_columns > 0) == narrow && _columns >= 0) return;
        _columns = narrowColumns;
        foreach (var part in _panes.Append(_strip).Append(_tray).Append(_side)) Responsive.Detach(part);
        Children.Clear();
        RowDefinitions.Clear();
        // The connection controls go under the message when narrow.
        SetRow(_connRight, narrow ? 1 : 0);
        SetColumn(_connRight, narrow ? 0 : 1);
        _connRight.HorizontalAlignment = narrow ? HorizontalAlignment.Left : HorizontalAlignment.Stretch;
        if (narrow)
        {
            _tray.Height = 380;
            var side = new Border { BorderThickness = new Thickness(0, 1, 0, 1), Child = _side }.Tok(Border.BorderBrushProperty, "ln");
            RowDefinitions = new RowDefinitions("Auto,*");
            var body = Responsive.Stack(_tray, side, MainWindow.PaneGrid(_panes, "*", stacked: true));
            SetRow(body, 1);
            Children.Add(_strip);
            Children.Add(body);
            return;
        }
        _tray.Height = double.NaN;
        var main = new Grid { ColumnDefinitions = new ColumnDefinitions("*,500") };
        SetColumn(_side, 1);
        main.Children.Add(_tray);
        main.Children.Add(_side);
        var mainBox = new Border { BorderThickness = new Thickness(0, 0, 0, 1), Child = main }.Tok(Border.BorderBrushProperty, "ln");
        var panes = MainWindow.PaneGrid(_panes, ".75*,1*,1.35*", stacked: false);
        RowDefinitions = new RowDefinitions("Auto,3*,1.6*");
        SetRow(_strip, 0);
        SetRow(mainBox, 1);
        SetRow(panes, 2);
        Children.Add(_strip);
        Children.Add(mainBox);
        Children.Add(panes);
    }
}

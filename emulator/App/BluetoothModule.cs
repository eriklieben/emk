using Avalonia.Controls;
using Avalonia.Controls.Primitives;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>
/// The Emulator's Bluetooth module: every Bluetooth state the firmware can show, profile and its
/// mark (+ open, ✗ paired but not connected, ✓ connected), output, the right half's link and the
/// link stage at the bottom of the screens. &amp;bt / &amp;out keys change these too; the controls follow.
/// </summary>
internal sealed class BluetoothModule
{
    private readonly EmulatedKeyboard _keyboard;
    private readonly EmulatorControls _controls;
    private readonly Action<string> _log;
    private readonly Action _changed;
    private readonly int _count = EmulatedKeyboard.ProfileCount;
    private readonly string?[] _known; // marks of profiles seen while selected
    private readonly ProfileCell[] _cells;
    private readonly TextBlock _title = W.Lbl("");
    private readonly Toggle _host = new("Computer connected");
    private readonly Toggle _open = new("Open (nothing paired)");
    private readonly Toggle _linked = new("Right half linked");
    private readonly Segmented _output = new(["USB", "Bluetooth"]);
    private readonly BenchButton _stage = new("");
    private bool _updating;

    /// <param name="log">A line for the events pane.</param>
    /// <param name="changed">The state shown changed (the top bar's lamps follow).</param>
    public BluetoothModule(EmulatedKeyboard keyboard, EmulatorControls controls, Action<string> log, Action changed)
    {
        _keyboard = keyboard;
        _controls = controls;
        _log = log;
        _changed = changed;
        _known = new string?[_count];
        _cells = new ProfileCell[_count + 1];
        (controls.Connected, controls.Open, controls.Linked, controls.Output, controls.LinkStage) = (_host, _open, _linked, _output, _stage);
    }

    public StackPanel Build()
    {
        var stageMenu = StageMenu();
        _stage.Click += () => stageMenu.ShowAt(_stage);
        var pair = _controls.SimulatePairing = new BenchButton("Simulate pairing");
        pair.Click += () =>
        {
            var code = _keyboard.SimulatePairing(Random.Shared.Next(1_000_000));
            _log($"pairing: the computer shows {code:000000}; type it on the keyboard (symbols layer digits), Enter; Esc cancels");
        };
        var grid = new UniformGrid { Columns = _count + 1, ColumnSpacing = 4 };
        for (var i = 0; i <= _count; i++) grid.Children.Add(Cell(i));
        _host.CheckedChanged += on => OnChange(() => _keyboard.SetProfileConnected(on));
        _open.CheckedChanged += on => OnChange(() => _keyboard.SetProfileOpen(on));
        _linked.CheckedChanged += on => OnChange(() => _keyboard.SetPeripheralConnected(on));
        _output.SelectionChanged += index => OnChange(() => _keyboard.Transport = (Transport)index);
        _keyboard.BluetoothChanged += Sync;
        Sync();

        var toggles = W.Wrap(16, 8, _host, _open, _linked);
        var controls = W.Wrap(10, 8, W.Lbl("Output"), _output, W.Lbl("Link stage"), _stage, pair);
        return W.Column(9, _title, grid, toggles, controls);
    }

    /// <summary>The link stages to pick from (index 0, empty, hides the stage).</summary>
    private MenuFlyout StageMenu()
    {
        var menu = new MenuFlyout();
        for (var i = 0; i < EmulatedKeyboard.LinkStages.Count; i++)
        {
            var index = i;
            var item = new MenuItem { Header = StageName(index) };
            item.Click += (_, _) => OnChange(() => _keyboard.LinkStage = index);
            _controls.LinkStages.Add(() => OnChange(() => _keyboard.LinkStage = index));
            menu.Items.Add(item);
        }
        return menu;
    }

    private static string StageName(int index) =>
        EmulatedKeyboard.LinkStages[index] is { Length: > 0 } name ? name : "hidden";

    /// <summary>Profile <paramref name="index"/>'s cell, or the USB cell after the last profile.</summary>
    private ProfileCell Cell(int index)
    {
        var cell = _cells[index] = new ProfileCell(index < _count ? $"{index + 1}" : "USB");
        cell.Click += () =>
        {
            if (index < _count)
            {
                _keyboard.Profile = index;
                _keyboard.Transport = Transport.Bluetooth;
            }
            else
            {
                _keyboard.Transport = Transport.Usb;
            }
            Sync();
        };
        _controls.Profiles.Add(cell);
        return cell;
    }

    private void Sync()
    {
        _updating = true;
        var selected = _keyboard.Profile;
        var onBt = _keyboard.Transport == Transport.Bluetooth;
        var mark = ProfileCell.MarkOf(_keyboard.ProfileConnected, _keyboard.ProfileOpen);
        _known[selected] = mark;
        for (var i = 0; i < _count; i++)
        {
            // Only the selected profile can be connected; others show what they were.
            var m = i == selected ? mark : Remembered(_known[i]);
            _cells[i].Set(m, selected: i == selected && onBt, current: i == selected);
        }
        _cells[_count].Set("·", selected: !onBt, current: false);
        _title.Text = $"BLUETOOTH · PROFILE {selected + 1} OF {_count}, USB LAST";
        _host.IsChecked = _keyboard.ProfileConnected;
        _open.IsChecked = _keyboard.ProfileOpen;
        _linked.IsChecked = _keyboard.PeripheralConnected;
        _output.SelectedIndex = (int)_keyboard.Transport;
        _stage.Text = StageName(_keyboard.LinkStage) + " ▾";
        _updating = false;
        _changed();
    }

    /// <summary>A control changed the keyboard: apply it, then show what the firmware made of it.</summary>
    private void OnChange(Action change)
    {
        if (_updating) return;
        change();
        Sync(); // e.g. connecting an open profile pairs it
    }

    /// <summary>A profile not selected: connected when last seen is paired now; never seen is ·.</summary>
    private static string Remembered(string? mark) => mark == "✓" ? "✗" : mark ?? "·";
}

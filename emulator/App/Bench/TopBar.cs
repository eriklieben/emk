using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.Media;

namespace KbEmulator.Bench;

/// <summary>The modes of the app, in the top bar's order.</summary>
public enum AppMode
{
    Emulator,
    Debug,
    Studio,
    Flash,
    Security,
}

/// <summary>
/// The top bar (CSS <c>.b-top</c>): EMK logo and model line, the mode
/// switch, status lamps for the mode, and the System / Light / Dark switch.
/// </summary>
public sealed class TopBar : Border
{
    private readonly TextBlock _model;
    private readonly StackPanel _lamps = new() { Orientation = Orientation.Horizontal, Spacing = 18, VerticalAlignment = VerticalAlignment.Center };

    public TopBar(IReadOnlyList<AppMode>? modes = null)
    {
        ShownModes = modes ?? Enum.GetValues<AppMode>();
        Height = 56;
        Padding = new Thickness(22, 0);
        BorderThickness = new Thickness(0, 0, 0, 1);
        this.Tok(BackgroundProperty, "face-hi").Tok(BorderBrushProperty, "ln");

        var logo = W.Head("EMK", 23, FontWeight.Bold);
        logo.LetterSpacing = 0.46;
        _model = W.Hint("", 13);
        _model.VerticalAlignment = VerticalAlignment.Center;
        _model.TextWrapping = TextWrapping.NoWrap;
        _model.Margin = new Thickness(0, 4, 0, 0);
        var brand = W.Row(12, logo, _model);

        Modes = new Segmented(ShownModes.Select(m => m.ToString()).ToArray(), 0, SegmentStyle.Modes);
        ThemeSwitch = new Segmented(["System", "Light", "Dark"], 0, SegmentStyle.Theme);

        _brand = brand;
        _right = W.Row(18, _lamps, ThemeSwitch);
        _grid = new Grid { ColumnDefinitions = new ColumnDefinitions("Auto,Auto,*,Auto"), RowDefinitions = new RowDefinitions("Auto,Auto") };
        Modes.Margin = new Thickness(26, 0, 0, 0);
        Grid.SetColumn(Modes, 1);
        Grid.SetColumn(_right, 3);
        _grid.Children.Add(brand);
        _grid.Children.Add(Modes);
        _grid.Children.Add(_right);
        Child = _grid;
    }

    private readonly Grid _grid;
    private readonly StackPanel _right;
    private readonly StackPanel _brand;
    private bool _narrow;

    /// <summary>
    /// Narrow window: logo, modes and the theme switch on the first line; the
    /// model and the lamps on a second one.
    /// </summary>
    public bool Narrow
    {
        get => _narrow;
        set
        {
            if (_narrow == value) return;
            _narrow = value;
            Height = value ? double.NaN : 56;
            Padding = value ? new Thickness(16, 8) : new Thickness(22, 0);
            Modes.Margin = new Thickness(value ? 14 : 26, 0, 0, 0);
            _brand.Children.Remove(_model);
            _right.Children.Remove(_lamps);
            _right.Children.Remove(ThemeSwitch);
            if (_line2 != null)
            {
                _line2.Children.Clear();
                _grid.Children.Remove(_line2);
            }
            _line2 = null;
            if (value)
            {
                _right.Children.Add(ThemeSwitch);
                _model.Margin = new Thickness(0);
                _line2 = W.Row(18, _model, _lamps);
                _line2.Margin = new Thickness(0, 8, 0, 0);
                Grid.SetRow(_line2, 1);
                Grid.SetColumnSpan(_line2, 4);
                _grid.Children.Add(_line2);
            }
            else
            {
                _model.Margin = new Thickness(0, 4, 0, 0);
                _brand.Children.Add(_model);
                _right.Children.Add(_lamps);
                _right.Children.Add(ThemeSwitch);
            }
        }
    }

    private StackPanel? _line2;

    /// <summary>The modes in the switch, in order (Security only when its feature is on).</summary>
    public IReadOnlyList<AppMode> ShownModes { get; }

    /// <summary>Emulator, Debug, Studio, Flash (and Security).</summary>
    public Segmented Modes { get; }

    /// <summary>System, Light, Dark (the order of <see cref="ThemePreference"/>).</summary>
    public Segmented ThemeSwitch { get; }

    public string Model
    {
        get => _model.Text ?? "";
        set => _model.Text = value;
    }

    /// <summary>Show the lamps of the current mode.</summary>
    public void SetLamps(IEnumerable<Lamp> lamps)
    {
        _lamps.Children.Clear();
        foreach (var lamp in lamps) _lamps.Children.Add(lamp);
    }

    public IEnumerable<Lamp> Lamps => _lamps.Children.OfType<Lamp>();
}

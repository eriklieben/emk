using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Layout;
using Avalonia.Media;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>A profile in the Bluetooth grid: its number and mark (✓ ✗ + ·).</summary>
internal sealed class ProfileCell : Border
{
    private readonly TextBlock _name;
    private readonly TextBlock _mark;
    private string _markText = "·";
    private bool _selected, _current;
    private bool _ring;

    public ProfileCell(string name)
    {
        BorderThickness = new Thickness(1);
        CornerRadius = new CornerRadius(3);
        Padding = new Thickness(0, 3, 0, 2);
        Cursor = new Cursor(StandardCursorType.Hand);
        _name = W.Head(name, name.Length > 2 ? 13 : 16);
        _name.HorizontalAlignment = HorizontalAlignment.Center;
        _name.TextWrapping = TextWrapping.NoWrap;
        _name.Height = 20;
        _name.LineHeight = 20;
        _mark = new TextBlock { FontSize = 12, Height = 15, FontFamily = BenchTheme.Ui, HorizontalAlignment = HorizontalAlignment.Center };
        Child = W.Column(0, _name, _mark);
        ActualThemeVariantChanged += (_, _) => Paint();
        AttachedToVisualTree += (_, _) => Paint();
        PointerPressed += (_, e) =>
        {
            if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) return;
            e.Handled = true;
            Click?.Invoke();
        };
        FocusRing.Attach(this, on =>
        {
            _ring = on;
            Paint();
        });
        KeyDown += (_, e) =>
        {
            if (!FocusRing.IsActivate(e)) return;
            e.Handled = true;
            Click?.Invoke();
        };
    }

    public event Action? Click;

    /// <summary>Act as if clicked (tests).</summary>
    internal void PerformClick() => Click?.Invoke();

    internal string Mark => _markText;

    internal bool Selected => _selected;

    /// <summary>A profile's mark: ✓ connected, + open (nothing paired), ✗ paired but not connected.</summary>
    public static string MarkOf(bool connected, bool open)
    {
        if (connected) return "✓";
        return open ? "+" : "✗";
    }

    public void Set(string mark, bool selected, bool current)
    {
        (_markText, _selected, _current) = (mark, selected, current);
        Paint();
    }

    private void Paint()
    {
        _mark.Text = _markText;
        Background = this.Brush(_selected ? "mode-bg" : "btn");
        BorderBrush = this.Brush(Edge);
        BoxShadow = _ring ? FocusRing.Ring(this) : default;
        _name.Tok(TextBlock.ForegroundProperty, _selected ? "mode-ink" : "ink");
        _mark.Tok(TextBlock.ForegroundProperty, _selected ? "mode-ink" : MarkTone(_markText));
    }

    private string Edge
    {
        get
        {
            if (_selected) return "mode-bg";
            return _current ? "acc" : "btn-edge";
        }
    }

    private static string MarkTone(string mark) => mark switch
    {
        "✓" => "good",
        "✗" => "crit",
        "+" => "acc",
        _ => "mut",
    };
}

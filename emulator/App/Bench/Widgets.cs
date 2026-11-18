using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Layout;
using Avalonia.Media;

namespace KbEmulator.Bench;

/// <summary>Small Bench building blocks made in code (labels, chips, lamps, buttons, segments, toggles).</summary>
public static class W
{
    /// <summary>Uppercase tracked label (CSS <c>.lbl</c>).</summary>
    public static TextBlock Lbl(string text, double size = 11.5) => new TextBlock
    {
        Text = text.ToUpperInvariant(),
        FontFamily = BenchTheme.Label,
        FontWeight = FontWeight.SemiBold,
        FontSize = size,
        LetterSpacing = size * 0.13,
        VerticalAlignment = VerticalAlignment.Center,
    }.Tok(TextBlock.ForegroundProperty, "mut");

    /// <summary>Muted hint text.</summary>
    public static TextBlock Hint(string text, double size = 12) => new TextBlock
    {
        Text = text,
        FontFamily = BenchTheme.Ui,
        FontSize = size,
        TextWrapping = TextWrapping.Wrap,
    }.Tok(TextBlock.ForegroundProperty, "mut");

    /// <summary>Body text in ink.</summary>
    public static TextBlock Text(string text, double size = 13, FontWeight? weight = null, bool wrap = true) => new TextBlock
    {
        Text = text,
        FontFamily = BenchTheme.Ui,
        FontSize = size,
        FontWeight = weight ?? FontWeight.Normal,
        TextWrapping = wrap ? TextWrapping.Wrap : TextWrapping.NoWrap,
        VerticalAlignment = VerticalAlignment.Center,
    }.Tok(TextBlock.ForegroundProperty, "ink");

    /// <summary>Monospace number / code text.</summary>
    public static TextBlock Mono(string text, double size = 12.5, string token = "ink") => new TextBlock
    {
        Text = text,
        FontFamily = BenchTheme.Mono,
        FontSize = size,
        VerticalAlignment = VerticalAlignment.Center,
        TextWrapping = TextWrapping.Wrap,
    }.Tok(TextBlock.ForegroundProperty, token);

    /// <summary>Condensed heading text (module titles, big values).</summary>
    public static TextBlock Head(string text, double size = 20, FontWeight? weight = null) => new TextBlock
    {
        Text = text,
        FontFamily = BenchTheme.Label,
        FontWeight = weight ?? FontWeight.SemiBold,
        FontSize = size,
        VerticalAlignment = VerticalAlignment.Center,
        TextWrapping = TextWrapping.Wrap,
    }.Tok(TextBlock.ForegroundProperty, "ink");

    /// <summary>A row: label left, value right (CSS <c>.split</c>).</summary>
    public static Grid Split(Control left, Control right)
    {
        var grid = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto") };
        Grid.SetColumn(right, 1);
        grid.Children.Add(left);
        grid.Children.Add(right);
        return grid;
    }

    public static StackPanel Row(double spacing, params Control[] children)
    {
        var panel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = spacing, VerticalAlignment = VerticalAlignment.Center };
        foreach (var c in children) panel.Children.Add(c);
        return panel;
    }

    public static WrapPanel Wrap(double h, double v, params Control[] children)
    {
        var panel = new WrapPanel { Orientation = Orientation.Horizontal, ItemSpacing = h, LineSpacing = v };
        foreach (var c in children) panel.Children.Add(c);
        return panel;
    }

    public static StackPanel Column(double spacing, params Control[] children)
    {
        var panel = new StackPanel { Spacing = spacing };
        foreach (var c in children) panel.Children.Add(c);
        return panel;
    }

    /// <summary>A rack module: padded, a hairline at its right edge.</summary>
    public static Border Module(Control content, Thickness? padding = null, bool last = false) => new Border
    {
        Padding = padding ?? new Thickness(16, 12),
        BorderThickness = last ? new Thickness(0) : new Thickness(0, 0, 1, 0),
        Child = content,
        ClipToBounds = true,
    }.Tok(Border.BorderBrushProperty, "ln");

    /// <summary>Status stripe (4 px bar) in a semantic color.</summary>
    public static Border Stripe(string tone) => new Border { Height = 4, CornerRadius = new CornerRadius(2) }.Tok(Border.BackgroundProperty, tone);

    /// <summary>A key/value table (CSS <c>.kv</c>).</summary>
    public static Border Kv(IEnumerable<(string Key, Control Value)> rows, double keyWidth = 120)
    {
        var grid = new Grid { ColumnDefinitions = new ColumnDefinitions($"{keyWidth},*") };
        var list = rows.ToList();
        for (var i = 0; i < list.Count; i++)
        {
            grid.RowDefinitions.Add(new RowDefinition(GridLength.Auto));
            var bottom = i < list.Count - 1 ? 1 : 0;
            var key = new Border { Padding = new Thickness(10, 5), BorderThickness = new Thickness(0, 0, 0, bottom), Child = Lbl(list[i].Key) }
                .Tok(Border.BorderBrushProperty, "ln");
            var value = new Border { Padding = new Thickness(10, 5), BorderThickness = new Thickness(0, 0, 0, bottom), Child = list[i].Value }
                .Tok(Border.BorderBrushProperty, "ln");
            Grid.SetRow(key, i);
            Grid.SetRow(value, i);
            Grid.SetColumn(value, 1);
            grid.Children.Add(key);
            grid.Children.Add(value);
        }
        return new Border { BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(3), Child = grid }
            .Tok(Border.BorderBrushProperty, "ln").Tok(Border.BackgroundProperty, "face-hi");
    }

    /// <summary>A dark text well (rules, notes): Barlow on the well color.</summary>
    public static Border Well(Control content) => new Border
    {
        CornerRadius = new CornerRadius(3),
        BorderThickness = new Thickness(1),
        Padding = new Thickness(12, 10),
        Child = content,
    }.Tok(Border.BackgroundProperty, "well").Tok(Border.BorderBrushProperty, "well-edge");
}

/// <summary>
/// Keyboard focus for the Bench controls: a ring in the accent colour, shown
/// only when focus came from the keyboard (Tab, arrows), not from a click.
/// </summary>
public static class FocusRing
{
    public static BoxShadows Ring(StyledElement e) =>
        new(new BoxShadow { Spread = 2, Color = e.ColorOf("acc") });

    /// <summary>Make <paramref name="c"/> a tab stop; <paramref name="changed"/> gets whether the ring shows.</summary>
    public static void Attach(InputElement c, Action<bool> changed)
    {
        c.Focusable = true;
        c.IsTabStop = true;
        c.GotFocus += (_, e) => changed(e.NavigationMethod is NavigationMethod.Tab or NavigationMethod.Directional);
        c.LostFocus += (_, _) => changed(false);
    }

    /// <summary>Enter or Space: activate.</summary>
    public static bool IsActivate(KeyEventArgs e) => e.Key is Key.Enter or Key.Space;
}

/// <summary>A status chip: small label plus value (CSS <c>.chip</c>).</summary>
public sealed class Chip : Border
{
    private readonly TextBlock _value;

    public Chip(string label, string value = "", bool accent = false)
    {
        Padding = new Thickness(9, 3);
        BorderThickness = new Thickness(1);
        CornerRadius = new CornerRadius(3);
        this.Tok(BackgroundProperty, "face-hi").Tok(BorderBrushProperty, accent ? "acc" : "ln");
        _value = W.Text(value, 13, wrap: false);
        var name = W.Lbl(label, 11);
        name.LetterSpacing = 11 * 0.08;
        Child = W.Row(6, name, _value);
    }

    public string Value
    {
        get => _value.Text ?? "";
        set => _value.Text = value;
    }
}

public enum LampState
{
    Off,
    Good,
    Warn,
    Crit,
    Accent,
}

/// <summary>A status lamp with a glow and its label (CSS <c>.lamp</c>).</summary>
public sealed class Lamp : StackPanel
{
    private readonly Border _dot = new() { Width = 9, Height = 9, CornerRadius = new CornerRadius(4.5), VerticalAlignment = VerticalAlignment.Center };
    private readonly TextBlock _text;
    private LampState _state;

    public Lamp(string text = "", LampState state = LampState.Off, double size = 13)
    {
        Orientation = Orientation.Horizontal;
        Spacing = 7;
        _text = new TextBlock
        {
            FontFamily = BenchTheme.Label,
            FontWeight = FontWeight.SemiBold,
            FontSize = size,
            LetterSpacing = size * 0.05,
            VerticalAlignment = VerticalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis,
        }.Tok(TextBlock.ForegroundProperty, "ink");
        Children.Add(_dot);
        Children.Add(_text);
        Text = text;
        ActualThemeVariantChanged += (_, _) => Paint();
        AttachedToVisualTree += (_, _) => Paint();
        State = state;
    }

    public string Text
    {
        get => _text.Text ?? "";
        set => _text.Text = value;
    }

    /// <summary>Let the label wrap (long connection messages).</summary>
    public bool Wraps
    {
        get => _text.TextWrapping == TextWrapping.Wrap;
        set
        {
            _text.TextWrapping = value ? TextWrapping.Wrap : TextWrapping.NoWrap;
            _text.TextTrimming = value ? TextTrimming.None : TextTrimming.CharacterEllipsis;
        }
    }

    public LampState State
    {
        get => _state;
        set
        {
            _state = value;
            Paint();
        }
    }

    public string Token => _state switch
    {
        LampState.Good => "lamp-good",
        LampState.Warn => "lamp-warn",
        LampState.Crit => "lamp-crit",
        LampState.Accent => "acc",
        _ => "lamp-off",
    };

    private void Paint()
    {
        var color = this.ColorOf(Token);
        _dot.Background = new SolidColorBrush(color);
        var glow = BenchTheme.Glow(ActualThemeVariant);
        var blur = _state == LampState.Crit ? 9 : 7;
        _dot.BoxShadow = _state == LampState.Off
            ? new BoxShadows(new BoxShadow { OffsetY = 1, Blur = 1, Color = Color.FromArgb(90, 0, 0, 0), IsInset = true })
            : new BoxShadows(new BoxShadow { Blur = blur * glow, Color = color });
    }
}

public enum ButtonKind
{
    Normal,
    Primary,
    Danger,
}

/// <summary>
/// A Bench button (CSS <c>.btn</c>), optionally with its log-port letter and
/// latching (<c>.btn.on</c>). Never takes focus: keys go to the keyboard.
/// </summary>
public sealed class BenchButton : Border
{
    private readonly TextBlock _text;
    private readonly Border? _kbd;
    private readonly ButtonKind _kind;
    private bool _latched;
    private bool _hover;
    private bool _down;
    private bool _enabled = true;
    private bool _ring;

    public BenchButton(string text, ButtonKind kind = ButtonKind.Normal, string? letter = null, bool latching = false)
    {
        _kind = kind;
        Latching = latching;
        Padding = new Thickness(11, 5);
        BorderThickness = new Thickness(1);
        CornerRadius = new CornerRadius(3);
        Cursor = new Cursor(StandardCursorType.Hand);
        HorizontalAlignment = HorizontalAlignment.Left;
        VerticalAlignment = VerticalAlignment.Center;
        _text = new TextBlock
        {
            Text = text,
            FontFamily = BenchTheme.Label,
            FontWeight = FontWeight.SemiBold,
            FontSize = 13,
            LetterSpacing = 13 * 0.03,
            VerticalAlignment = VerticalAlignment.Center,
        };
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        if (letter != null)
        {
            _kbd = new Border
            {
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(2),
                Padding = new Thickness(4, 0),
                VerticalAlignment = VerticalAlignment.Center,
                Child = new TextBlock { Text = letter, FontFamily = BenchTheme.Mono, FontSize = 11 },
            };
            row.Children.Add(_kbd);
        }
        row.Children.Add(_text);
        Child = row;
        ActualThemeVariantChanged += (_, _) => Paint();
        AttachedToVisualTree += (_, _) => Paint();
        FocusRing.Attach(this, on =>
        {
            _ring = on;
            Paint();
        });
        KeyDown += (_, e) =>
        {
            if (!FocusRing.IsActivate(e)) return;
            e.Handled = true;
            PerformClick();
        };
        Paint();
    }

    public event Action? Click;

    /// <summary>Latched state changed (by a click or in code).</summary>
    public event Action<bool>? LatchedChanged;

    public bool Latching { get; }

    public string Text
    {
        get => _text.Text ?? "";
        set => _text.Text = value;
    }

    public bool IsLatched
    {
        get => _latched;
        set
        {
            if (_latched == value) return;
            _latched = value;
            Paint();
            LatchedChanged?.Invoke(value);
        }
    }

    public bool Enabled
    {
        get => _enabled;
        set
        {
            _enabled = value;
            Opacity = value ? 1 : 0.5;
            Cursor = value ? new Cursor(StandardCursorType.Hand) : Cursor.Default;
            IsTabStop = value;
        }
    }

    /// <summary>Act as if clicked (tests).</summary>
    internal void PerformClick()
    {
        if (!_enabled) return;
        if (Latching) IsLatched = !IsLatched;
        Click?.Invoke();
    }

    protected override void OnPointerEntered(PointerEventArgs e)
    {
        base.OnPointerEntered(e);
        _hover = true;
        Paint();
    }

    protected override void OnPointerExited(PointerEventArgs e)
    {
        base.OnPointerExited(e);
        _hover = false;
        Paint();
    }

    protected override void OnPointerPressed(PointerPressedEventArgs e)
    {
        base.OnPointerPressed(e);
        if (!_enabled || !e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) return;
        _down = true;
        e.Pointer.Capture(this);
        e.Handled = true;
        Paint();
    }

    protected override void OnPointerReleased(PointerReleasedEventArgs e)
    {
        base.OnPointerReleased(e);
        if (!_down) return;
        _down = false;
        e.Pointer.Capture(null);
        Paint();
        if (new Rect(Bounds.Size).Contains(e.GetPosition(this))) PerformClick();
        e.Handled = true;
    }

    private void Paint()
    {
        var (bg, edge, ink) = Tokens();
        Background = this.Brush(bg);
        BorderBrush = this.Brush(edge);
        _text.Foreground = this.Brush(ink);
        BoxShadow = Shadow();
        if (_kbd != null) PaintLetter(_kbd);
    }

    /// <summary>Background, edge and ink tokens for the state: latched, kind, pressed, hovered.</summary>
    private (string Bg, string Edge, string Ink) Tokens()
    {
        if (_latched) return ("mode-bg", "mode-bg", "mode-ink");
        var (bg, edge, ink) = _kind switch
        {
            ButtonKind.Primary => ("acc", "acc", "acc-ink"),
            ButtonKind.Danger => ("btn", "crit", "crit"),
            _ => ("btn", "btn-edge", "ink"),
        };
        if (_down) bg = "face-lo";
        if (_hover && _kind == ButtonKind.Normal) edge = "acc";
        return (bg, edge, ink);
    }

    private BoxShadows Shadow()
    {
        if (_ring) return FocusRing.Ring(this);
        if (_kind == ButtonKind.Normal && !_latched) return new BoxShadows(new BoxShadow { OffsetY = 1, Color = this.ColorOf("btn-hl"), IsInset = true });
        return default;
    }

    /// <summary>The log-port letter's little key.</summary>
    private void PaintLetter(Border kbd)
    {
        kbd.Background = _latched ? Brushes.Transparent : this.Brush("face-lo");
        kbd.BorderBrush = _latched ? this.Brush("mode-ink") : this.Brush("btn-edge");
        ((TextBlock)kbd.Child!).Foreground = _latched ? this.Brush("mode-ink") : this.Brush("mut");
        kbd.Opacity = _latched ? 0.75 : 1;
    }
}

public enum SegmentStyle
{
    /// <summary>Rack control (CSS <c>.seg</c>).</summary>
    Rack,
    /// <summary>The top bar's mode switch (CSS <c>.b-modes</c>).</summary>
    Modes,
    /// <summary>The theme switch (CSS <c>.theme-sw</c>).</summary>
    Theme,
}

/// <summary>A segmented control: one of several, the chosen one dark (light in Dark).</summary>
public sealed class Segmented : Border
{
    private readonly List<(Border Cell, TextBlock Text)> _cells = new();
    private readonly SegmentStyle _style;
    private int _selected;
    private int _hover = -1;
    private bool _ring;

    public Segmented(IReadOnlyList<string> items, int selected = 0, SegmentStyle style = SegmentStyle.Rack)
    {
        _style = style;
        _selected = selected;
        BorderThickness = new Thickness(1);
        CornerRadius = new CornerRadius(style == SegmentStyle.Rack ? 3 : 4);
        ClipToBounds = true;
        HorizontalAlignment = HorizontalAlignment.Left;
        VerticalAlignment = VerticalAlignment.Center;
        var row = new StackPanel { Orientation = Orientation.Horizontal };
        for (var i = 0; i < items.Count; i++) row.Children.Add(Cell(items[i], i, last: i == items.Count - 1));
        Child = row;
        ActualThemeVariantChanged += (_, _) => Paint();
        AttachedToVisualTree += (_, _) => Paint();
        FocusRing.Attach(this, on =>
        {
            _ring = on;
            Paint();
        });
        KeyDown += (_, e) => OnArrowKey(e);
        Paint();
    }

    /// <summary>Text size, padding, letter spacing and case of a style's segments.</summary>
    private static (double Size, Thickness Pad, double Tracking, bool Upper) Look(SegmentStyle style) => style switch
    {
        SegmentStyle.Modes => (13.0, new Thickness(15, 6), 0.1, true),
        SegmentStyle.Theme => (12.0, new Thickness(10, 5), 0.08, true),
        _ => (13.0, new Thickness(10, 5), 0.04, false),
    };

    private Border Cell(string item, int index, bool last)
    {
        var (size, pad, tracking, upper) = Look(_style);
        var text = new TextBlock
        {
            Text = upper ? item.ToUpperInvariant() : item,
            FontFamily = BenchTheme.Label,
            FontWeight = FontWeight.SemiBold,
            FontSize = size,
            LetterSpacing = size * tracking,
            HorizontalAlignment = HorizontalAlignment.Center,
        };
        var cell = new Border
        {
            Padding = pad,
            BorderThickness = last ? new Thickness(0) : new Thickness(0, 0, 1, 0),
            Child = text,
            Cursor = new Cursor(StandardCursorType.Hand),
        };
        cell.PointerEntered += (_, _) =>
        {
            _hover = index;
            Paint();
        };
        cell.PointerExited += (_, _) =>
        {
            if (_hover == index) _hover = -1;
            Paint();
        };
        cell.PointerPressed += (_, e) =>
        {
            if (!e.GetCurrentPoint(cell).Properties.IsLeftButtonPressed) return;
            e.Handled = true;
            Choose(index);
        };
        _cells.Add((cell, text));
        return cell;
    }

    /// <summary>Arrow keys move the choice (keyboard focus).</summary>
    private void OnArrowKey(KeyEventArgs e)
    {
        var step = e.Key switch { Key.Left or Key.Up => -1, Key.Right or Key.Down => 1, _ => 0 };
        if (step == 0 || _cells.Count == 0) return;
        e.Handled = true;
        Choose(Math.Clamp(_selected + step, 0, _cells.Count - 1));
    }

    /// <summary>The user picked a segment.</summary>
    public event Action<int>? SelectionChanged;

    /// <summary>The chosen segment; setting it doesn't raise <see cref="SelectionChanged"/>. -1: none.</summary>
    public int SelectedIndex
    {
        get => _selected;
        set
        {
            if (_selected == value) return;
            _selected = value;
            Paint();
        }
    }

    /// <summary>Pick a segment as the user would.</summary>
    internal void Choose(int index)
    {
        if (index == _selected) return;
        _selected = index;
        Paint();
        SelectionChanged?.Invoke(index);
    }

    private IBrush CellIdle(int i) => i == _hover ? this.Brush("face-lo") : Brushes.Transparent;

    private string IdleInk => _style == SegmentStyle.Rack ? "ink" : "mut";

    private void Paint()
    {
        var edge = _style == SegmentStyle.Rack ? "btn-edge" : "ln";
        BorderBrush = this.Brush(edge);
        BoxShadow = _ring ? FocusRing.Ring(this) : default;
        Background = this.Brush(_style switch
        {
            SegmentStyle.Rack => "btn",
            SegmentStyle.Modes => "face-lo",
            _ => "face-hi",
        });
        for (var i = 0; i < _cells.Count; i++)
        {
            var (cell, text) = _cells[i];
            cell.BorderBrush = this.Brush(edge);
            var on = i == _selected;
            cell.Background = on ? this.Brush("mode-bg") : CellIdle(i);
            text.Foreground = this.Brush(on ? "mode-ink" : IdleInk);
        }
    }
}

/// <summary>A checkbox in Bench style (CSS <c>.tg</c> with <c>.box</c>).</summary>
public sealed class Toggle : StackPanel
{
    private readonly Border _box = new()
    {
        Width = 15,
        Height = 15,
        BorderThickness = new Thickness(1.5),
        CornerRadius = new CornerRadius(2),
        VerticalAlignment = VerticalAlignment.Center,
    };
    private readonly Avalonia.Controls.Shapes.Path _check = new()
    {
        Data = Geometry.Parse("M 1.5,5.5 L 4.5,8.5 L 10,2"),
        StrokeThickness = 1.8,
        Width = 12,
        Height = 11,
        HorizontalAlignment = HorizontalAlignment.Center,
        VerticalAlignment = VerticalAlignment.Center,
    };
    private bool _checked;
    private bool _ring;

    public Toggle(string text, bool isChecked = false)
    {
        Orientation = Orientation.Horizontal;
        Spacing = 7;
        Background = Brushes.Transparent; // the whole row is clickable
        Cursor = new Cursor(StandardCursorType.Hand);
        _box.Child = _check;
        Children.Add(_box);
        Children.Add(W.Text(text, 13, wrap: false));
        _checked = isChecked;
        ActualThemeVariantChanged += (_, _) => Paint();
        AttachedToVisualTree += (_, _) => Paint();
        PointerPressed += (_, e) =>
        {
            if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) return;
            e.Handled = true;
            IsChecked = !IsChecked;
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
            IsChecked = !IsChecked;
        };
        Paint();
    }

    /// <summary>Raised whenever <see cref="IsChecked"/> changes (click or code).</summary>
    public event Action<bool>? CheckedChanged;

    public bool IsChecked
    {
        get => _checked;
        set
        {
            if (_checked == value) return;
            _checked = value;
            Paint();
            CheckedChanged?.Invoke(value);
        }
    }

    private void Paint()
    {
        _box.Background = _checked ? this.Brush("acc") : Brushes.Transparent;
        _box.BorderBrush = this.Brush(_checked ? "acc" : "mut");
        _check.Stroke = this.Brush("acc-ink");
        _box.BoxShadow = _ring ? FocusRing.Ring(this) : default;
        _check.IsVisible = _checked;
    }
}

/// <summary>A rounded status pill (CSS <c>.pill</c>).</summary>
public sealed class Pill : Border
{
    private readonly TextBlock _text = new() { FontFamily = BenchTheme.Ui, FontWeight = FontWeight.SemiBold, FontSize = 12, LetterSpacing = 0.36 };

    public Pill(string text, string tone)
    {
        Padding = new Thickness(9, 2);
        CornerRadius = new CornerRadius(999);
        HorizontalAlignment = HorizontalAlignment.Left;
        VerticalAlignment = VerticalAlignment.Center;
        Child = _text;
        Set(text, tone);
    }

    /// <summary>Text and tone: good, warn or crit.</summary>
    public void Set(string text, string tone)
    {
        _text.Text = text;
        this.Tok(BackgroundProperty, tone + "-bg");
        _text.Tok(TextBlock.ForegroundProperty, tone);
    }
}

/// <summary>A battery gauge with ten cells; drag on it to set the level (CSS <c>.gauge</c>).</summary>
public sealed class Gauge : Control
{
    private int _value = 100;
    private bool _dragging;
    private bool _ring;

    public Gauge()
    {
        Width = 30;
        Height = 85;
        Cursor = new Cursor(StandardCursorType.SizeNorthSouth);
        ActualThemeVariantChanged += (_, _) => InvalidateVisual();
        FocusRing.Attach(this, on =>
        {
            _ring = on;
            InvalidateVisual();
        });
        KeyDown += (_, e) =>
        {
            var step = e.Key switch { Key.Up or Key.Right => 10, Key.Down or Key.Left => -10, _ => 0 };
            if (step == 0) return;
            e.Handled = true;
            Value += step;
        };
    }

    public event Action<int>? ValueChanged;

    public int Value
    {
        get => _value;
        set
        {
            value = Math.Clamp(value, 0, 100);
            if (_value == value) return;
            _value = value;
            InvalidateVisual();
            ValueChanged?.Invoke(value);
        }
    }

    public override void Render(DrawingContext context)
    {
        var body = new Rect(0.75, 5.75, Bounds.Width - 1.5, Bounds.Height - 6.5);
        var ink = this.Brush("mut");
        context.DrawRectangle(ink, null, new Rect(Bounds.Width / 2 - 4.5, 0.5, 9, 4.5), 1, 1);
        context.DrawRectangle(null, new Pen(ink, 1.5), body, 3, 3);
        if (_ring) context.DrawRectangle(null, new Pen(this.Brush("acc"), 2), body.Inflate(2), 4, 4);
        var inner = body.Deflate(3);
        var cell = (inner.Height - 9 * 2) / 10;
        var full = (int)Math.Round(_value / 10.0);
        var on = this.Brush(_value <= 20 ? "crit" : "good");
        var off = this.Brush("gauge-off");
        for (var i = 0; i < 10; i++)
        {
            var y = inner.Bottom - (i + 1) * cell - i * 2;
            context.FillRectangle(i < full ? on : off, new Rect(inner.X, y, inner.Width, cell));
        }
    }

    protected override void OnPointerPressed(PointerPressedEventArgs e)
    {
        base.OnPointerPressed(e);
        _dragging = true;
        e.Pointer.Capture(this);
        SetFrom(e.GetPosition(this));
        e.Handled = true;
    }

    protected override void OnPointerMoved(PointerEventArgs e)
    {
        base.OnPointerMoved(e);
        if (_dragging) SetFrom(e.GetPosition(this));
    }

    protected override void OnPointerReleased(PointerReleasedEventArgs e)
    {
        base.OnPointerReleased(e);
        _dragging = false;
        e.Pointer.Capture(null);
    }

    private void SetFrom(Point p)
    {
        var inner = new Rect(0, 8, Bounds.Width, Bounds.Height - 11);
        var fraction = (inner.Bottom - p.Y) / inner.Height;
        Value = (int)Math.Round(Math.Clamp(fraction, 0, 1) * 10) * 10;
    }
}

/// <summary>A strip at the top of a mode (CSS <c>.conn</c>).</summary>
public static class Strips
{
    public static Border Conn(Control content) => new Border
    {
        Padding = new Thickness(22, 9),
        BorderThickness = new Thickness(0, 0, 0, 1),
        Child = content,
    }.Tok(Border.BackgroundProperty, "face-hi").Tok(Border.BorderBrushProperty, "ln");

    /// <summary>The recessed tray the keyboard sits in (CSS <c>.tray</c>).</summary>
    public static Border Tray(Control content) => new Border
    {
        Padding = new Thickness(16, 10),
        Child = content,
        ClipToBounds = true,
    }.Tok(Border.BackgroundProperty, "face-lo");

    /// <summary>A side panel with a hairline at its left.</summary>
    public static Border Side(Control content, bool left = true) => new Border
    {
        Padding = new Thickness(18, 14),
        BorderThickness = left ? new Thickness(1, 0, 0, 0) : new Thickness(0, 0, 1, 0),
        Child = content,
    }.Tok(Border.BorderBrushProperty, "ln");
}

/// <summary>Inset shadow on top of a tray (the recess), drawn over its content edge.</summary>
public sealed class Recess : Control
{
    public Recess()
    {
        IsHitTestVisible = false;
        ActualThemeVariantChanged += (_, _) => InvalidateVisual();
    }

    public override void Render(DrawingContext context)
    {
        var c = this.ColorOf("recess");
        var brush = new LinearGradientBrush
        {
            StartPoint = new RelativePoint(0, 0, RelativeUnit.Relative),
            EndPoint = new RelativePoint(0, 1, RelativeUnit.Relative),
            GradientStops = { new GradientStop(c, 0), new GradientStop(Color.FromArgb(0, c.R, c.G, c.B), 1) },
        };
        context.FillRectangle(brush, new Rect(0, 0, Bounds.Width, Math.Min(9, Bounds.Height)));
    }
}

/// <summary>Helpers for the clipboard.</summary>
public static class ClipboardText
{
    /// <summary>Put <paramref name="text"/> on the clipboard; never throws (fire and forget is fine).</summary>
    public static async Task CopyAsync(Visual from, string text)
    {
        try
        {
            if (TopLevel.GetTopLevel(from)?.Clipboard is { } clipboard)
            {
                await Avalonia.Input.Platform.ClipboardExtensions.SetTextAsync(clipboard, text);
            }
        }
        catch (Exception)
        {
            // Best effort: no clipboard (headless, no compositor, the platform refused) is nothing to report.
        }
    }
}

using System.Globalization;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;
using KbEmulator.Bench;
using Kb.Ssd1306;

namespace KbEmulator;

/// <summary>
/// Draws both Corne halves with their OLEDs, in the Bench tokens. Click a
/// key to press it while the button is down; right-click to latch it (for
/// combos and holds). Each half's case is the Corne v3 outline: the top
/// steps with the column stagger, the controller and OLED sit at the inner
/// top, and the bottom drops around the fanned thumb cluster.
/// </summary>
public sealed class CorneView : Control
{
    private const double Unit = 60;       // key pitch
    private const double KeySize = 54;
    private const double Pad = 6;         // around the drawing
    private const double Top = Pad + 34;  // room above the halves for the switch and USB cable
    private const double OledScale = 1.375;
    private const double OledX = 6 * Unit + 4; // bezel, in half coordinates
    private const double OledY = 4;
    private const double OledW = 54, OledH = 198;
    private const double HalfWidth = OledX + OledW;
    private const double Gap = 70;        // between the halves
    private const double CaseMargin = 10;

    // Column stagger of a Corne, in key units (outer pinky column first).
    private static readonly double[] Stagger = [0.375, 0.375, 0.125, 0, 0.125, 0.25];

    // Thumb keys of the left half: x, y (units) and angle, inner key lowest and turned most.
    private static readonly (double X, double Y, double Rot)[] Thumbs = [(3.3, 3.42, 0), (4.4, 3.5, 7), (5.5, 3.68, 18)];

    private static readonly Typeface LegendFace = new(BenchTheme.Label, FontStyle.Normal, FontWeight.SemiBold);
    private static readonly Typeface SubFace = new(BenchTheme.Label, FontStyle.Normal, FontWeight.Medium);

    private readonly (Rect Rect, double Rot)[] _keys;
    private readonly Geometry[] _case = new Geometry[2];
    private readonly HashSet<int> _pressed = new();
    private readonly HashSet<int> _latched = new();
    private string[] _labels;
    private bool[] _transparent;
    private int? _mouseKey;
    private int? _hover;
    private readonly bool[] _powerOn = [true, true];
    private readonly bool[] _usb = [false, false];
    private int? _resetDown;
    private ulong _hardwareHeld;
    private bool _hardwareMode;
    private bool _showBoardControls = true;
    private bool _dimmed;
    private int? _selected;
    private static Geometry? _pixelGrid;

    public CorneView(int keyCount)
    {
        _keys = BuildLayout(keyCount);
        _labels = new string[keyCount];
        _transparent = new bool[keyCount];
        for (var half = 0; half < 2; half++) _case[half] = BuildCase(half);
        Focusable = false;
        ActualThemeVariantChanged += (_, _) => InvalidateVisual();
    }

    public Ssd1306[] Displays { get; set; } = [];

    /// <summary>
    /// Colours from here instead of the theme (the poster draws keyboards
    /// off-screen in a theme of its own). Null: the control's theme tokens.
    /// </summary>
    public Func<string, IBrush>? Palette { get; set; }

    private IBrush B(string token) => Palette?.Invoke(token) ?? this.Brush(token);

    private Color C(string token) => B(token) is ISolidColorBrush s ? s.Color : Colors.Magenta;

    /// <summary>Legends larger than in the app (the poster: readable in print); they still shrink to fit a key.</summary>
    public double LegendScale { get; set; } = 1;

    /// <summary>Where the two cases are drawn (the keyboard without the room around it).</summary>
    public Rect CaseBounds => _case[0].Bounds.Union(_case[1].Bounds);

    /// <summary>The drawing's size (what the control measures to).</summary>
    public Size DrawingSize => MeasureOverride(Size.Infinity);

    public event Action<int>? KeyPressed;
    public event Action<int>? KeyReleased;

    /// <summary>A half's power switch was flipped (half, on).</summary>
    public event Action<int, bool>? PowerToggled;

    /// <summary>A USB cable was plugged into or pulled from a half (half, connected).</summary>
    public event Action<int, bool>? UsbToggled;

    /// <summary>A half's reset button was pressed.</summary>
    public event Action<int>? ResetPressed;

    /// <summary>A key was picked (when <see cref="SelectMode"/> is on).</summary>
    public event Action<int>? KeySelected;

    /// <summary>Clicks select keys instead of pressing them (Studio, read-only).</summary>
    public bool SelectMode { get; set; }

    /// <summary>The selected key (Studio), drawn with a ring.</summary>
    public int? SelectedKey
    {
        get => _selected;
        set
        {
            _selected = value;
            InvalidateVisual();
        }
    }

    /// <summary>Combos drawn as pills between their keys (Studio).</summary>
    public IReadOnlyList<(int A, int B, string Name)> Combos { get; set; } = [];

    public void SetLabels(string[] labels)
    {
        _labels = labels;
        InvalidateVisual();
    }

    /// <summary>Keys that fall through to the layer below: drawn with a muted legend.</summary>
    public void SetTransparent(bool[] transparent)
    {
        _transparent = transparent;
        InvalidateVisual();
    }

    private bool[] _changed = [];

    private HashSet<int> _highlighted = new();

    /// <summary>Keys drawn selected together (a combo being edited).</summary>
    public void SetHighlighted(IEnumerable<int> keys)
    {
        _highlighted = keys.ToHashSet();
        InvalidateVisual();
    }

    internal IReadOnlyCollection<int> Highlighted => _highlighted;

    /// <summary>The keys marked changed (tests).</summary>
    internal IReadOnlyList<bool> Changed => _changed;

    /// <summary>Keys whose binding differs from the default (Studio): a dot in the corner.</summary>
    public void SetChanged(bool[] changed)
    {
        _changed = changed;
        InvalidateVisual();
    }

    /// <summary>Show a key as pressed (from the physical keyboard).</summary>
    public void SetPressed(int pos, bool pressed)
    {
        if (pressed ? _pressed.Add(pos) : _pressed.Remove(pos)) InvalidateVisual();
    }

    /// <summary>
    /// Keys the real keyboard reports held (Debug), bit n = position n: lit.
    /// A key pressed here but not (yet) held there gets an outline; one held
    /// both ways is lit with a ring.
    /// </summary>
    public void SetHardwareHeld(ulong held)
    {
        _hardwareMode = true;
        if (held == _hardwareHeld) return;
        _hardwareHeld = held;
        InvalidateVisual();
    }

    /// <summary>The emulated power switches, USB ports and reset buttons (not in Debug).</summary>
    public bool ShowBoardControls
    {
        get => _showBoardControls;
        set
        {
            _showBoardControls = value;
            InvalidateVisual();
        }
    }

    /// <summary>Both halves drawn dimmed (the real keyboard is not connected).</summary>
    public bool Dimmed
    {
        get => _dimmed;
        set
        {
            if (_dimmed == value) return;
            _dimmed = value;
            InvalidateVisual();
        }
    }

    /// <summary>Keys pressed here (mouse, latch, PC keyboard).</summary>
    public IReadOnlyCollection<int> PressedHere =>
        _pressed.Union(_latched).Concat(_mouseKey is { } m ? [m] : []).ToHashSet();

    /// <summary>Latched keys (right-click).</summary>
    public IReadOnlyCollection<int> Latched => _latched;

    /// <summary>A click on key <paramref name="pos"/>: pressed, then released (tests).</summary>
    internal void Click(int pos)
    {
        KeyPressed?.Invoke(pos);
        KeyReleased?.Invoke(pos);
    }

    /// <summary>Latch a key as a right-click would (tests).</summary>
    internal void LatchForTest(int pos) => ToggleLatch(pos);

    public void ReleaseLatched()
    {
        foreach (var pos in _latched.ToArray()) ToggleLatch(pos);
    }

    /// <summary>Whether a half is switched on (drawn dimmed when off).</summary>
    public bool IsOn(int half) => _powerOn[half];

    /// <summary>Whether a USB cable is plugged into a half.</summary>
    public bool IsUsbPlugged(int half) => _usb[half];

    private static double HalfOrigin(int half) => Pad + (half == 0 ? 0 : HalfWidth + Gap);

    /// <summary>
    /// Pixel area of a half's OLED in control coordinates. The 128×32 module
    /// stands vertically beside the inner column, so the area is 32 wide and 128 tall.
    /// </summary>
    internal static Rect OledPixelArea(int half)
    {
        var bezelX = half == 0 ? OledX : 0;
        return new Rect(HalfOrigin(half) + bezelX + 5, Top + OledY + 11, 32 * OledScale, 128 * OledScale);
    }

    private static Rect Bezel(int half) =>
        new(HalfOrigin(half) + (half == 0 ? OledX : 0), Top + OledY, OledW, OledH);

    private static (Rect, double)[] BuildLayout(int keyCount)
    {
        var keys = new (Rect, double)[keyCount];
        static (Rect, double) Left(double x, double y, double rot) => (new Rect(HalfOrigin(0) + x, Top + y, KeySize, KeySize), rot);
        static (Rect, double) Right(double x, double y, double rot) =>
            (new Rect(HalfOrigin(1) + HalfWidth - x - KeySize, Top + y, KeySize, KeySize), -rot);

        for (var row = 0; row < 3; row++)
        {
            for (var col = 0; col < 6; col++)
            {
                var (x, y) = (col * Unit, (row + Stagger[col]) * Unit);
                keys[row * 12 + col] = Left(x, y, 0);
                keys[row * 12 + 11 - col] = Right(x, y, 0);
            }
        }
        for (var i = 0; i < Thumbs.Length; i++)
        {
            var (x, y, rot) = (Thumbs[i].X * Unit, Thumbs[i].Y * Unit, Thumbs[i].Rot);
            if (36 + i < keyCount) keys[36 + i] = Left(x, y, rot);
            if (41 - i < keyCount) keys[41 - i] = Right(x, y, rot);
        }
        return keys;
    }

    private static bool IsLeft(int pos) => pos is >= 36 and <= 38 || (pos < 36 && pos % 12 < 6);

    private static Matrix Rotation(Rect rect, double degrees) =>
        Matrix.CreateTranslation(-rect.Center.X, -rect.Center.Y) *
        Matrix.CreateRotation(degrees * Math.PI / 180) *
        Matrix.CreateTranslation(rect.Center.X, rect.Center.Y);

    /// <summary>
    /// The case of a half as one outline: every key grown by the margin,
    /// the controller/OLED block at the inner top, and the PCB between the
    /// bottom row and the thumb cluster.
    /// </summary>
    private Geometry BuildCase(int half)
    {
        Geometry? outline = null;
        void Add(Rect rect, double rot = 0, double radius = 8)
        {
            var g = new RectangleGeometry(rect, radius, radius);
            if (rot != 0) g.Transform = new MatrixTransform(Rotation(rect, rot));
            outline = outline == null ? g : new CombinedGeometry(GeometryCombineMode.Union, outline, g);
        }
        for (var pos = 0; pos < _keys.Length; pos++)
        {
            if (IsLeft(pos) != (half == 0)) continue;
            var (rect, rot) = _keys[pos];
            Add(rect.Inflate(CaseMargin), rot);
        }
        var bezel = Bezel(half);
        Add(bezel.Inflate(new Thickness(half == 0 ? 6 : CaseMargin, CaseMargin, half == 0 ? CaseMargin : 6, 36)));
        // The board between the bottom row and the thumbs (no notch there on a Corne).
        var o = HalfOrigin(half);
        var fillX = Thumbs[0].X * Unit + 6;
        var fill = new Rect(fillX, 2 * Unit + 40, HalfWidth - fillX, Thumbs[0].Y * Unit - 2 * Unit - 20);
        Add(half == 0 ? fill.Translate(new Vector(o, Top)) : new Rect(o + HalfWidth - fill.Right, Top + fill.Y, fill.Width, fill.Height), 0, 0);
        return outline!;
    }

    // The nice!nano's USB-C port at the top edge of the controller; the power
    // switch next to it, toward the keys; the reset button under the screen.
    private static Rect UsbPort(int half)
    {
        var bezel = Bezel(half);
        return new Rect(bezel.Center.X - 14, bezel.Y - CaseMargin - 6, 28, 10);
    }

    private static Rect UsbCable(int half)
    {
        var port = UsbPort(half);
        return new Rect(port.X + 8, 0, 12, port.Y);
    }

    private static Rect PowerSwitch(int half)
    {
        var bezel = Bezel(half);
        var x = half == 0 ? bezel.X - 58 : bezel.Right + 24;
        return new Rect(x, Top - 18, 34, 16);
    }

    private static Point ResetCenter(int half)
    {
        var bezel = Bezel(half);
        return new Point(bezel.Center.X, bezel.Bottom + 16);
    }

    private const double ResetRadius = 6;

    protected override Size MeasureOverride(Size availableSize)
    {
        var bounds = _case[0].Bounds.Union(_case[1].Bounds);
        return new Size(bounds.Right + Pad, bounds.Bottom + Pad);
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == IsEffectivelyEnabledProperty) InvalidateVisual();
    }

    public override void Render(DrawingContext context)
    {
        var plate = B("plate");
        var plateEdge = new Pen(B("plate-edge"), 1);
        for (var half = 0; half < 2; half++)
        {
            context.DrawGeometry(plate, plateEdge, _case[half]);
            var bezel = Bezel(half);
            context.DrawRectangle(B("bezel"), null, bezel, 4, 4);
            if (_showBoardControls) DrawControls(context, half);
        }

        for (var half = 0; half < 2; half++) DrawOled(context, half < Displays.Length ? Displays[half] : null, half);

        for (var pos = 0; pos < _keys.Length; pos++) DrawKey(context, pos);
        DrawCombos(context);

        // A switched-off half (or no real keyboard): dimmed.
        var shade = C("face-lo");
        var off = new SolidColorBrush(Color.FromArgb(160, shade.R, shade.G, shade.B));
        for (var half = 0; half < 2; half++)
        {
            if (_dimmed || (_showBoardControls && !_powerOn[half])) context.DrawGeometry(off, null, _case[half]);
        }
    }

    private void DrawKey(DrawingContext ctx, int pos)
    {
        var (rect, rot) = _keys[pos];
        using var _ = rot != 0 ? ctx.PushTransform(Rotation(rect, rot)) : default(DrawingContext.PushedState?);
        var here = _pressed.Contains(pos) || _mouseKey == pos;
        var latched = _latched.Contains(pos);
        var hardware = (_hardwareHeld & (1UL << pos)) != 0;
        var label = _labels[pos] ?? "";
        var lit = _hardwareMode ? hardware : here;
        var (fill, stroke, ink, width, dash) = Look(pos, lit, latched, here, none: SelectMode && label.Length == 0);

        if (_selected == pos || _highlighted.Contains(pos) || (_hardwareMode && hardware && here))
        {
            var ring = new Pen(B("k-sel"), 1.2);
            using (ctx.PushOpacity(0.5)) ctx.DrawRectangle(null, ring, rect.Inflate(5), 8, 8);
        }
        ctx.DrawRectangle(B(fill), new Pen(B(stroke), width) { DashStyle = dash }, rect.Deflate(width / 2), 4, 4);
        DrawLegend(ctx, label, rect, B(ink), B(lit ? "k-on-ink" : "k-sub"), LegendScale);
        if (pos < _changed.Length && _changed[pos]) ctx.DrawEllipse(B("warn"), null, new Point(rect.Right - 7, rect.Y + 7), 3.5, 3.5);
    }

    /// <summary>
    /// A key's fill, edge, ink, edge width and dashes: lit (held), latched, pressed here while
    /// mirroring, unbound; then selected or hovered; muted ink when it falls through.
    /// </summary>
    private (string Fill, string Stroke, string Ink, double Width, DashStyle? Dash) Look(int pos, bool lit, bool latched, bool here, bool none)
    {
        string fill = "k-fill", stroke = "k-stroke", ink = "k-ink";
        double width = 1;
        DashStyle? dash = null;
        if (lit) (fill, stroke, ink) = ("k-on", "k-on", "k-on-ink");
        else if (latched) (fill, stroke, width, dash) = ("k-lat-fill", "k-on", 2, new DashStyle([2.5, 1.5], 0));
        else if (_hardwareMode && here) (fill, stroke, width) = ("k-lat-fill", "k-on", 2);
        else if (none) (fill, dash) = ("k-none", new DashStyle([3, 3], 0));

        if (_selected == pos || _highlighted.Contains(pos)) (stroke, width) = ("k-sel", 2.5);
        else if (_hover == pos && !lit) (stroke, width) = ("k-hover", 2);
        if (!lit && !latched && SelectMode && pos < _transparent.Length && _transparent[pos]) ink = "k-sub";
        return (fill, stroke, ink, width, dash);
    }

    private static void DrawLegend(DrawingContext ctx, string label, Rect rect, IBrush ink, IBrush sub, double scale = 1)
    {
        if (label.Length == 0) return;
        var lines = label.Split('\n');
        var main = lines[0];
        var tap = Fit(main, LegendSize(main.Length) * scale, 7, rect.Width - 6, ink, LegendFace);
        if (lines.Length == 1)
        {
            ctx.DrawText(tap, rect.Center - new Point(tap.Width / 2, tap.Height / 2));
            return;
        }
        var hold = Fit(lines[1], 9.5 * scale, 6, rect.Width - 6, sub, SubFace);
        ctx.DrawText(tap, new Point(rect.Center.X - tap.Width / 2, rect.Center.Y - 5 - tap.Height / 2));
        ctx.DrawText(hold, new Point(rect.Center.X - hold.Width / 2, rect.Center.Y + 13 - hold.Height / 2));
    }

    /// <summary>Legend size for a tap legend of this many characters.</summary>
    private static double LegendSize(int length) => length switch
    {
        <= 2 => 19,
        <= 4 => 15,
        <= 6 => 12.5,
        _ => 10.5,
    };

    /// <summary>The text at <paramref name="size"/>, made smaller in half points until it fits (or reaches the minimum).</summary>
    private static FormattedText Fit(string text, double size, double minSize, double maxWidth, IBrush brush, Typeface face)
    {
        var formatted = Text(text, size, brush, face);
        while (formatted.Width > maxWidth && size > minSize)
        {
            size -= 0.5;
            formatted = Text(text, size, brush, face);
        }
        return formatted;
    }

    private void DrawCombos(DrawingContext ctx)
    {
        foreach (var (a, b, name) in Combos)
        {
            if (a >= _keys.Length || b >= _keys.Length) continue;
            var (ra, rb) = (_keys[a].Rect, _keys[b].Rect);
            var text = Text(name, 9, B("face"), LegendFace);
            var mid = new Point((ra.Center.X + rb.Center.X) / 2, Math.Max(ra.Bottom, rb.Bottom) - 2);
            var pill = new Rect(mid.X - text.Width / 2 - 6, mid.Y - 7, text.Width + 12, 14);
            ctx.DrawRectangle(B("ink"), null, pill, 7, 7);
            ctx.DrawText(text, new Point(pill.X + 6, pill.Center.Y - text.Height / 2));
        }
    }

    /// <summary>Power switch and USB port at the top edge of a half, reset button under its screen.</summary>
    private void DrawControls(DrawingContext ctx, int half)
    {
        var hwInk = B("hw-ink");
        // USB-C port, and a cable when plugged in.
        var port = UsbPort(half);
        if (_usb[half])
        {
            var cable = UsbCable(half);
            ctx.DrawRectangle(hwInk, null, cable, 3, 3);
        }
        ctx.DrawRectangle(B("bezel"), new Pen(hwInk, 1.5), port, 5, 5);
        var usbLabel = Text(_usb[half] ? "USB ⚡" : "USB", 10.5, _usb[half] ? B("good") : hwInk, LegendFace);
        var usbAt = half == 0
            ? new Point(port.Right + 6, port.Center.Y - usbLabel.Height / 2 - 6)
            : new Point(port.X - usbLabel.Width - 6, port.Center.Y - usbLabel.Height / 2 - 6);
        ctx.DrawText(usbLabel, usbAt);

        // Slide switch: knob right and green when on.
        var sw = PowerSwitch(half);
        ctx.DrawRectangle(B(_powerOn[half] ? "lamp-good" : "lamp-off"), null, sw, sw.Height / 2, sw.Height / 2);
        var knobX = _powerOn[half] ? sw.Right - sw.Height / 2 : sw.X + sw.Height / 2;
        ctx.DrawEllipse(Brushes.White, null, new Point(knobX, sw.Center.Y), sw.Height / 2 - 2, sw.Height / 2 - 2);
        var label = Text(_powerOn[half] ? "ON" : "OFF", 10.5, hwInk, LegendFace);
        var labelAt = half == 0
            ? new Point(sw.X - label.Width - 6, sw.Center.Y - label.Height / 2)
            : new Point(sw.Right + 6, sw.Center.Y - label.Height / 2);
        ctx.DrawText(label, labelAt);

        // Reset button under the screen, its label toward the outer edge of the board.
        var reset = ResetCenter(half);
        ctx.DrawEllipse(_resetDown == half ? B("k-on") : B("lamp-crit"),
            new Pen(new SolidColorBrush(Color.FromArgb(100, 0, 0, 0)), 1), reset, ResetRadius, ResetRadius);
        var resetLabel = Text("RST", 10, B("plate-ink"), LegendFace);
        var resetX = half == 0 ? reset.X + ResetRadius + 5 : reset.X - ResetRadius - 5 - resetLabel.Width;
        ctx.DrawText(resetLabel, new Point(resetX, reset.Y - resetLabel.Height / 2));
    }

    private static FormattedText Text(string s, double size, IBrush brush, Typeface face) =>
        new(s, CultureInfo.InvariantCulture, FlowDirection.LeftToRight, face, size, brush);

    /// <summary>Every pixel of the module as a faint dot (the dot matrix seen on unlit glass).</summary>
    private static Geometry PixelGrid()
    {
        if (_pixelGrid != null) return _pixelGrid;
        var geometry = new StreamGeometry();
        using (var g = geometry.Open())
        {
            var dot = OledScale - 0.35;
            for (var y = 0; y < 128; y++)
            {
                for (var x = 0; x < 32; x++)
                {
                    var (px, py) = (x * OledScale, y * OledScale);
                    g.BeginFigure(new Point(px, py), true);
                    g.LineTo(new Point(px + dot, py));
                    g.LineTo(new Point(px + dot, py + dot));
                    g.LineTo(new Point(px, py + dot));
                    g.EndFigure(true);
                }
            }
        }
        return _pixelGrid = geometry;
    }

    /// <summary>
    /// Draw the module as it is mounted: its landscape content reads bottom to
    /// top, on both halves. The OLED keeps its cyan look in both themes.
    /// </summary>
    private static void DrawOled(DrawingContext ctx, Ssd1306? display, int half)
    {
        var area = OledPixelArea(half);
        ctx.FillRectangle(new SolidColorBrush(Color.Parse("#030506")), area);
        var on = BenchTheme.OledOn;
        using (ctx.PushTransform(Matrix.CreateTranslation(area.X, area.Y)))
        {
            ctx.DrawGeometry(new SolidColorBrush(on, 0.07), null, PixelGrid());
        }
        if (display == null) return;

        var brightness = 0.45 + 0.55 * display.Contrast / 255.0;
        var lit = new SolidColorBrush(on, brightness);
        var dot = OledScale - 0.35;
        for (var my = 0; my < display.PanelHeight; my++)
        {
            for (var mx = 0; mx < Ssd1306.Columns; mx++)
            {
                if (!display.GetPixel(mx, my)) continue;
                // Both modules are mounted the same way (seen on the gray Corne):
                // landscape content reads bottom to top on both halves.
                var (sx, sy) = (my, Ssd1306.Columns - 1 - mx);
                ctx.FillRectangle(lit, new Rect(area.X + sx * OledScale, area.Y + sy * OledScale, dot, dot));
            }
        }
    }

    private int? HitTest(Point p)
    {
        for (var i = 0; i < _keys.Length; i++)
        {
            var (rect, rot) = _keys[i];
            var local = rot == 0 ? p : p.Transform(Rotation(rect, -rot));
            if (rect.Contains(local)) return i;
        }
        return null;
    }

    private void ToggleLatch(int pos)
    {
        if (_latched.Remove(pos)) KeyReleased?.Invoke(pos);
        else if (_latched.Add(pos)) KeyPressed?.Invoke(pos);
        InvalidateVisual();
    }

    protected override void OnPointerMoved(PointerEventArgs e)
    {
        base.OnPointerMoved(e);
        var hover = HitTest(e.GetPosition(this));
        if (hover == _hover) return;
        _hover = hover;
        InvalidateVisual();
    }

    protected override void OnPointerExited(PointerEventArgs e)
    {
        base.OnPointerExited(e);
        if (_hover == null) return;
        _hover = null;
        InvalidateVisual();
    }

    protected override void OnPointerPressed(PointerPressedEventArgs e)
    {
        base.OnPointerPressed(e);
        var at = e.GetPosition(this);
        if (_showBoardControls && !SelectMode && HitControl(at)) { e.Handled = true; return; }
        if (HitTest(at) is not { } pos) return;
        var point = e.GetCurrentPoint(this);
        if (SelectMode)
        {
            SelectedKey = pos;
            KeySelected?.Invoke(pos);
        }
        else if (point.Properties.IsRightButtonPressed)
        {
            ToggleLatch(pos);
        }
        else if (point.Properties.IsLeftButtonPressed && !_latched.Contains(pos) && _mouseKey == null)
        {
            _mouseKey = pos;
            e.Pointer.Capture(this);
            KeyPressed?.Invoke(pos);
            InvalidateVisual();
        }
        e.Handled = true;
    }

    /// <summary>The middle of a key, in control coordinates (pointer tests).</summary>
    internal Point KeyCenter(int pos) => _keys[pos].Rect.Center;

    /// <summary>Flip a half's power switch (as a click on it does).</summary>
    internal void FlipPower(int half)
    {
        _powerOn[half] = !_powerOn[half];
        PowerToggled?.Invoke(half, _powerOn[half]);
        InvalidateVisual();
    }

    /// <summary>Plug in or pull a half's USB cable (as a click on the port does).</summary>
    internal void PlugUsb(int half)
    {
        _usb[half] = !_usb[half];
        UsbToggled?.Invoke(half, _usb[half]);
        InvalidateVisual();
    }

    /// <summary>Press a half's reset button (as a click on it does).</summary>
    internal void PressReset(int half) => ResetPressed?.Invoke(half);

    /// <summary>Where a half's power switch, USB port and reset button are (pointer tests).</summary>
    internal static (Point Power, Point Usb, Point Reset) ControlCenters(int half) =>
        (PowerSwitch(half).Center, UsbPort(half).Center, ResetCenter(half));

    /// <summary>Power switch, USB port or reset button under the pointer: act on it.</summary>
    private bool HitControl(Point at)
    {
        for (var half = 0; half < 2; half++)
        {
            if (PowerSwitch(half).Inflate(4).Contains(at))
            {
                FlipPower(half);
                return true;
            }
            if (UsbPort(half).Inflate(6).Contains(at) || UsbCable(half).Contains(at))
            {
                PlugUsb(half);
                return true;
            }
            var reset = ResetCenter(half);
            if (Math.Abs(at.X - reset.X) <= ResetRadius + 3 && Math.Abs(at.Y - reset.Y) <= ResetRadius + 3)
            {
                _resetDown = half;
                ResetPressed?.Invoke(half);
                InvalidateVisual();
                return true;
            }
        }
        return false;
    }

    protected override void OnPointerReleased(PointerReleasedEventArgs e)
    {
        base.OnPointerReleased(e);
        if (_resetDown != null)
        {
            _resetDown = null;
            InvalidateVisual();
        }
        if (_mouseKey is { } pos)
        {
            _mouseKey = null;
            e.Pointer.Capture(null);
            KeyReleased?.Invoke(pos);
            InvalidateVisual();
        }
    }
}

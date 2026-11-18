using System.Globalization;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;

namespace KbEmulator.Bench;

/// <summary>
/// The emulated computer's screen with the mouse pointer the keyboard's
/// mouse keys move: the summed motion from the middle of a 1920×1200 screen,
/// the buttons (lit while held, and briefly after a click), the wheel and pan,
/// and whether the pointer is active (the MOUSE bar on the OLED).
/// </summary>
public sealed class PointerPad : Control
{
    public const double ScreenWidth = 1920, ScreenHeight = 1200;

    private static readonly Typeface Mono = new(BenchTheme.Mono);
    private static readonly Typeface Label = new(BenchTheme.Label, FontStyle.Normal, FontWeight.SemiBold);
    private readonly DateTime[] _flash = new DateTime[3];
    private MouseState _state;

    public PointerPad()
    {
        MinHeight = 120;
        ClipToBounds = true;
        ActualThemeVariantChanged += (_, _) => InvalidateVisual();
    }

    public MouseState State
    {
        get => _state;
        set
        {
            if (_state == value && !Flashing) return;
            _state = value;
            InvalidateVisual();
        }
    }

    /// <summary>Buttons just pressed: they light up for a moment even after a quick click.</summary>
    public void Flash(byte buttons)
    {
        for (var b = 0; b < 3; b++)
        {
            if ((buttons & (1 << b)) != 0) _flash[b] = DateTime.UtcNow;
        }
        InvalidateVisual();
    }

    private bool Flashing => _flash.Any(t => DateTime.UtcNow - t < TimeSpan.FromMilliseconds(350));

    /// <summary>Where the pointer is on the screen (px from its top left), kept on the screen.</summary>
    public static Point ScreenPosition(MouseState s) =>
        new(Math.Clamp(ScreenWidth / 2 + s.X, 0, ScreenWidth - 1), Math.Clamp(ScreenHeight / 2 + s.Y, 0, ScreenHeight - 1));

    /// <summary>Wheel and pan in detents (smooth scrolling counts 1/16 detents).</summary>
    public static (double Wheel, double Pan) Detents(MouseState s) =>
        (s.Wheel / ((s.Hires & 1) != 0 ? 16.0 : 1.0), s.Pan / ((s.Hires & 2) != 0 ? 16.0 : 1.0));

    private static FormattedText Text(string s, double size, IBrush brush, Typeface? face = null) =>
        new(s, CultureInfo.InvariantCulture, FlowDirection.LeftToRight, face ?? Mono, size, brush);

    public override void Render(DrawingContext context)
    {
        var bounds = new Rect(Bounds.Size);
        context.DrawRectangle(this.Brush("well"), new Pen(this.Brush("well-edge"), 1), bounds.Deflate(0.5), 3, 3);
        var log = this.Brush("log");
        var dim = this.Brush("log-dim");
        var acc = this.Brush("log-b");

        // The screen: 16:10 inside the well, leaving room for the readouts below and the wheel at the right.
        var area = bounds.Deflate(new Thickness(10, 10, 26, 30));
        var w = Math.Min(area.Width, area.Height * ScreenWidth / ScreenHeight);
        var screen = new Rect(area.X + (area.Width - w) / 2, area.Y, w, w * ScreenHeight / ScreenWidth);
        context.DrawRectangle(new SolidColorBrush(Color.FromArgb(18, 255, 255, 255)), new Pen(dim, 1), screen, 2, 2);
        var scale = screen.Width / ScreenWidth;
        var at = ScreenPosition(_state);
        var p = new Point(screen.X + at.X * scale, screen.Y + at.Y * scale);

        // Arrow pointer, white with a dark edge, brighter while the pointer is active.
        var arrow = new StreamGeometry();
        using (var g = arrow.Open())
        {
            g.BeginFigure(p, true);
            g.LineTo(p + new Point(0, 15));
            g.LineTo(p + new Point(4, 11));
            g.LineTo(p + new Point(7, 17));
            g.LineTo(p + new Point(9.5, 16));
            g.LineTo(p + new Point(6.5, 10));
            g.LineTo(p + new Point(11, 10));
            g.EndFigure(true);
        }
        using (context.PushOpacity(_state.PointerActive || _state.Moving ? 1 : 0.6))
        {
            context.DrawGeometry(Brushes.White, new Pen(Brushes.Black, 1.2), arrow);
        }

        // Wheel (right, vertical) and pan (under the screen, horizontal): a marker per detent.
        var (wheel, pan) = Detents(_state);
        var wheelTrack = new Rect(bounds.Right - 18, screen.Y, 6, screen.Height);
        context.DrawRectangle(dim, null, wheelTrack, 3, 3);
        var wy = wheelTrack.Center.Y - (wheel % 6) / 6 * (wheelTrack.Height / 2 - 4);
        context.DrawRectangle(acc, null, new Rect(wheelTrack.X - 2, wy - 3, 10, 6), 2, 2);

        // Buttons: L M R, lit while held or just clicked.
        var x = bounds.X + 10;
        var y = bounds.Bottom - 22;
        string[] names = ["L", "M", "R"];
        int[] bits = [0, 2, 1];
        for (var i = 0; i < 3; i++)
        {
            var b = bits[i];
            var on = (_state.Buttons & (1 << b)) != 0 || DateTime.UtcNow - _flash[b] < TimeSpan.FromMilliseconds(350);
            var box = new Rect(x, y, 18, 14);
            context.DrawRectangle(on ? acc : null, new Pen(on ? acc : dim, 1), box, 2, 2);
            var t = Text(names[i], 10, on ? this.Brush("well") : dim, Label);
            context.DrawText(t, new Point(box.Center.X - t.Width / 2, box.Center.Y - t.Height / 2));
            x += 22;
        }
        var readout = $"x {MouseReport(_state.X)} y {MouseReport(_state.Y)} · w {Num(wheel)} p {Num(pan)}";
        var text = Text(readout, 11, log);
        text.MaxTextWidth = Math.Max(10, bounds.Right - x - 8);
        text.MaxLineCount = 1;
        text.Trimming = TextTrimming.CharacterEllipsis;
        context.DrawText(text, new Point(x + 4, y));
        if (_state.PointerActive)
        {
            var badge = Text("MOUSE", 9.5, this.Brush("well"), Label);
            var r = new Rect(screen.X + 4, screen.Y + 4, badge.Width + 8, badge.Height + 2);
            context.DrawRectangle(acc, null, r, 2, 2);
            context.DrawText(badge, new Point(r.X + 4, r.Y + 1));
        }
        if (Flashing) Avalonia.Threading.Dispatcher.UIThread.Post(InvalidateVisual, Avalonia.Threading.DispatcherPriority.Background);
    }

    private static string MouseReport(int n) => Kb.HostProtocol.MouseReport.WithSign(n);

    private static string Num(double d)
    {
        if (Math.Abs(d - Math.Round(d)) < 0.01) return Kb.HostProtocol.MouseReport.WithSign((int)Math.Round(d));
        var sign = d > 0 ? "+" : "−";
        return sign + Math.Abs(d).ToString("0.0", CultureInfo.InvariantCulture);
    }
}

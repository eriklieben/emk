using System.Globalization;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.Media;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>
/// The nice!nano's nRF52840 with its flash map and RAM, filled with what
/// the firmware measured by <c>./size-report</c> takes (the side panel).
/// </summary>
public sealed class ChipPanel : StackPanel
{
    public ChipPanel(SizeReport? report)
    {
        Spacing = 11;
        Children.Add(W.Lbl("Chip"));
        var head = new StackPanel { Spacing = 1, VerticalAlignment = VerticalAlignment.Center };
        head.Children.Add(W.Head(report?.Chip ?? "nRF52840", 17));
        head.Children.Add(W.Hint($"{report?.Board ?? "nice!nano v2"} · 1 MB flash · 256 KB RAM", 12.5));
        Children.Add(W.Row(12, new ChipIcon(), head));

        if (report is not { } r)
        {
            Children.Add(W.Hint("No size measurement yet: run ./size-report (./emulate does it for you)."));
            return;
        }

        Children.Add(W.Split(W.Lbl("Flash"),
            W.Mono($"{SizeReport.Kb(r.FlashUsed)} / {SizeReport.Kb(r.FirmwareRegion)} · {r.FlashFraction:P1}")));
        var segments = new List<(double, string)>();
        foreach (var region in r.FlashRegions)
        {
            if (region.Name == "Firmware")
            {
                segments.Add((r.FlashUsed, "acc"));
                segments.Add((Math.Max(0, region.Size - r.FlashUsed), ""));
            }
            else
            {
                segments.Add((region.Size, "dim"));
            }
        }
        Children.Add(new MapBar(segments, r.FlashTotal));
        var reserved = r.FlashTotal - r.FirmwareRegion;
        var legend = new Grid { ColumnDefinitions = new ColumnDefinitions("10,*,Auto"), RowDefinitions = new RowDefinitions("Auto,Auto,Auto") };
        void Legend(int row, string token, string name, long bytes)
        {
            var swatch = new Border { Width = 10, Height = 10, CornerRadius = new CornerRadius(1), VerticalAlignment = VerticalAlignment.Center };
            if (token.Length > 0) swatch.Tok(Border.BackgroundProperty, token);
            else swatch.Tok(Border.BorderBrushProperty, "ln").BorderThickness = new Thickness(1);
            var label = W.Hint(name, 12.5);
            label.Margin = new Thickness(8, 2);
            var value = W.Mono(SizeReport.Kb(bytes), 12, "mut");
            foreach (var (c, col) in new (Control, int)[] { (swatch, 0), (label, 1), (value, 2) })
            {
                Grid.SetRow(c, row);
                Grid.SetColumn(c, col);
                legend.Children.Add(c);
            }
        }
        Legend(0, "acc", "Firmware", r.FlashUsed);
        Legend(1, "", "Free", r.FirmwareRegion - r.FlashUsed);
        Legend(2, "dim", "SoftDevice, bonds, settings, bootloader", reserved);
        Children.Add(legend);

        var ram = W.Split(W.Lbl("RAM"), W.Mono($"{SizeReport.Kb(r.RamUsed)} / {SizeReport.Kb(r.RamTotal)} · {r.RamFraction:P1}"));
        ram.Margin = new Thickness(0, 2, 0, 0);
        Children.Add(ram);
        Children.Add(new MapBar([(r.RamUsed, "acc"), (r.RamTotal - r.RamUsed, "")], r.RamTotal));
        var generated = DateTime.TryParse(r.Generated, CultureInfo.InvariantCulture, DateTimeStyles.None, out var at) ? $" at {at:HH:mm}" : "";
        Children.Add(W.Hint($"From ./size-report{generated}. Measured: {r.Includes}. Not in it yet: {r.Excludes}."));
    }

    /// <summary>A flash/RAM map bar (CSS <c>.mapbar</c>); empty token = free.</summary>
    private sealed class MapBar : Control
    {
        private readonly IReadOnlyList<(double Size, string Token)> _segments;
        private readonly double _total;

        public MapBar(IReadOnlyList<(double, string)> segments, double total)
        {
            _segments = segments;
            _total = Math.Max(1, total);
            Height = 14;
            ActualThemeVariantChanged += (_, _) => InvalidateVisual();
        }

        public override void Render(DrawingContext context)
        {
            var box = new Rect(Bounds.Size).Deflate(0.5);
            context.DrawRectangle(this.Brush("gauge-off"), null, box, 2, 2);
            using (context.PushClip(new RoundedRect(box, 2)))
            {
                var x = box.X;
                var divider = new Pen(new SolidColorBrush(Color.FromArgb(64, 0, 0, 0)), 1);
                for (var i = 0; i < _segments.Count; i++)
                {
                    var (size, token) = _segments[i];
                    var w = box.Width * size / _total;
                    if (token.Length > 0) context.FillRectangle(this.Brush(token), new Rect(x, box.Y, Math.Max(token == "acc" ? 2 : 0, w), box.Height));
                    else context.FillRectangle(this.Brush("face-lo"), new Rect(x, box.Y, w, box.Height));
                    x += w;
                    if (i < _segments.Count - 1) context.DrawLine(divider, new Point(x, box.Y), new Point(x, box.Bottom));
                }
            }
            context.DrawRectangle(null, new Pen(this.Brush("ln"), 1), box, 2, 2);
        }
    }

    /// <summary>The QFN package with pins on all sides.</summary>
    private sealed class ChipIcon : Control
    {
        public ChipIcon()
        {
            Width = 44;
            Height = 44;
            ActualThemeVariantChanged += (_, _) => InvalidateVisual();
        }

        public override void Render(DrawingContext context)
        {
            var s = Bounds.Width / 46;
            var pin = new Pen(new SolidColorBrush(this.ColorOf("ink"), 0.55), 2 * s);
            for (var i = 0; i < 5; i++)
            {
                var t = (12 + i * 6) * s;
                context.DrawLine(pin, new Point(t, 3 * s), new Point(t, 9 * s));
                context.DrawLine(pin, new Point(t, 37 * s), new Point(t, 43 * s));
                context.DrawLine(pin, new Point(3 * s, t), new Point(9 * s, t));
                context.DrawLine(pin, new Point(37 * s, t), new Point(43 * s, t));
            }
            var body = new Rect(8 * s, 8 * s, 30 * s, 30 * s);
            context.DrawRectangle(new SolidColorBrush(Color.Parse("#15171a")), new Pen(new SolidColorBrush(this.ColorOf("ink"), 0.3), 1), body, 2, 2);
            var face = new Typeface(BenchTheme.Label);
            var ink = new SolidColorBrush(Color.Parse("#e2e3dd"));
            foreach (var (text, y) in new[] { ("nRF", 17.0), ("52840", 26.0) })
            {
                var t = new FormattedText(text, System.Globalization.CultureInfo.InvariantCulture, FlowDirection.LeftToRight, face, 7 * s, ink);
                context.DrawText(t, new Point(body.Center.X - t.Width / 2, y * s - t.Height / 2 + 4 * s));
            }
        }
    }
}

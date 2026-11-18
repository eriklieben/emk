using System.Diagnostics.CodeAnalysis;
using System.Globalization;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>
/// The keymap poster: every layer on the real Corne shape (CorneView's own
/// drawing), how to reach it, the combos on small Cornes, a header with the
/// keyboard's name and a legend for two-line keys. Bench palette, OLED cyan
/// accent. Laid out at 1600 units wide and scaled to the format's pixels.
/// </summary>
public sealed class PosterView : Control
{
    public const double LayoutWidth = 1600;

    private static readonly string[] LayerTokens = ["acc", "vio", "good", "warn"];

    private readonly PosterModel _model;
    private readonly PosterOptions _options;
    private readonly Func<string, IBrush> _palette;
    private readonly Dictionary<string, IBrush> _brushes = new();

    private readonly double _scale;
    private readonly double _logicalHeight;

    /// <param name="scale">Drawing scale on top of the 1600-unit layout (the PDF page is in points).</param>
    public PosterView(PosterModel model, PosterOptions options, double scale = 1)
    {
        _scale = scale;
        _model = model;
        _options = options;
        var tokens = options.Dark ? BenchTheme.Dark : BenchTheme.Light;
        _palette = token =>
        {
            if (_brushes.TryGetValue(token, out var cached)) return cached;
            var brush = new SolidColorBrush(tokens.TryGetValue(token, out var css) ? BenchTheme.ParseColor(css) : Colors.Magenta);
            _brushes[token] = brush;
            return brush;
        };
        var px = PosterFormats.Pixels(options.Format);
        _logicalHeight = LayoutWidth * px.Height / px.Width;
        Width = LayoutWidth * scale;
        Height = _logicalHeight * scale;
    }

    public PosterModel Model => _model;
    public PosterOptions Options => _options;

    /// <summary>The colour of a token in the poster's theme (tests).</summary>
    internal IBrush Brush(string token) => _palette(token);

    private bool Wide => _options.Format == PosterFormat.Wallpaper;

    private static readonly Typeface Head = new(BenchTheme.Label, FontStyle.Normal, FontWeight.Bold);
    private static readonly Typeface HeadSemi = new(BenchTheme.Label, FontStyle.Normal, FontWeight.SemiBold);
    private static readonly Typeface Body = new(BenchTheme.Ui);
    private static readonly Typeface Mono = new(BenchTheme.Mono);

    private FormattedText Text(string s, Typeface face, double size, string token) =>
        new(s, CultureInfo.InvariantCulture, FlowDirection.LeftToRight, face, size, _palette(token));

    /// <summary>Uppercase label with letter spacing (drawn per character).</summary>
    private void Tracked(DrawingContext ctx, string s, Point at, Typeface face, double size, string token, double spacing)
    {
        var x = at.X;
        foreach (var ch in s.ToUpperInvariant())
        {
            var t = Text(ch.ToString(), face, size, token);
            ctx.DrawText(t, new Point(x, at.Y));
            x += t.WidthIncludingTrailingWhitespace + spacing;
        }
    }

    public override void Render(DrawingContext context)
    {
        using var _ = context.PushTransform(Matrix.CreateScale(_scale, _scale));
        var (w, h) = (LayoutWidth, _logicalHeight);
        var page = new Rect(0, 0, w, h);
        context.FillRectangle(_palette(_options.Dark ? "face-lo" : "face"), page);
        DrawRain(context, page);

        var margin = Wide ? 36.0 : 64.0;
        DrawHeader(context, new Rect(margin, margin - 8, w - 2 * margin, Wide ? 96 : 120));
        var (layersArea, combosArea) = Areas();
        DrawLayers(context, layersArea);
        if (combosArea is { } c) DrawCombos(context, c, vertical: Wide);
        var footer = 28.0;
        DrawFooter(context, new Rect(margin, h - margin - footer + 10, w - 2 * margin, footer));
    }

    /// <summary>Where the layers and the combos go (no combos area when they're off or there are none).</summary>
    internal (Rect Layers, Rect? Combos) Areas()
    {
        var (w, h) = (LayoutWidth, _logicalHeight);
        var margin = Wide ? 36.0 : 64.0;
        var headerBottom = margin - 8 + (Wide ? 108 : 132);
        var footer = 28.0;
        var body = new Rect(margin, headerBottom + 24, w - 2 * margin, h - headerBottom - 24 - margin - footer);
        if (!_options.Combos || _model.Combos.Count == 0) return (body, null);
        if (Wide)
        {
            var layers = body.WithWidth(body.Width - 288);
            return (layers, new Rect(layers.Right + 16, body.Y, 272, body.Height));
        }
        const double comboHeight = 198;
        var top = body.WithHeight(body.Height - comboHeight - 24);
        return (top, new Rect(body.X, top.Bottom + 24, body.Width, comboHeight));
    }

    /// <summary>A few faint columns of OLED rain in the top right corner: the start-up animation.</summary>
    [SuppressMessage("Security", "S2245:Using pseudorandom number generators (PRNGs) is security-sensitive",
        Justification = "Decoration from a fixed seed: the same picture every time.")]
    private void DrawRain(DrawingContext ctx, Rect page)
    {
        var on = BenchTheme.OledOn;
        var rnd = new Random(7);
        var cell = 5.0;
        for (var col = 0; col < 8; col++)
        {
            var x = page.Right - 24 - col * 14;
            var length = 4 + rnd.Next(10);
            var start = rnd.Next(-4, 8);
            for (var i = 0; i < length; i++)
            {
                var alpha = (1 - i / (double)length) * (1 - col / 8.0) * (_options.Dark ? 0.3 : 0.22);
                var y = 20 + (start + i) * 12;
                if (y < 0) continue;
                ctx.FillRectangle(new SolidColorBrush(on, alpha), new Rect(x, y, cell, cell));
                if (rnd.NextDouble() < 0.35) ctx.FillRectangle(new SolidColorBrush(on, alpha * 0.7), new Rect(x + cell + 1, y + 2, cell - 2, cell - 2));
            }
        }
    }

    /// <summary>The E·L mark as the OLED draws it at start-up: chunky pixels and sparks.</summary>
    [SuppressMessage("Security", "S2245:Using pseudorandom number generators (PRNGs) is security-sensitive",
        Justification = "Decoration from a fixed seed: the same picture every time.")]
    private static void DrawMark(DrawingContext ctx, Point at, double cell, Rect within)
    {
        const string full = "######", stem = "##....", arm = "#####.";
        string[] e = [full, full, stem, arm, arm, stem, full, full];
        string[] l = [stem, stem, stem, stem, stem, stem, full, full];
        var on = new SolidColorBrush(BenchTheme.OledOn);
        void Letter(string[] rows, double x0)
        {
            for (var r = 0; r < rows.Length; r++)
            {
                for (var c = 0; c < rows[r].Length; c++)
                {
                    if (rows[r][c] == '#') ctx.FillRectangle(on, new Rect(x0 + c * cell, at.Y + r * cell, cell - 1, cell - 1));
                }
            }
        }
        Letter(e, at.X);
        ctx.FillRectangle(on, new Rect(at.X + 7 * cell, at.Y + 3.5 * cell, cell - 1, cell - 1));
        Letter(l, at.X + 9 * cell);
        // Sparks thrown off the E.
        var rnd = new Random(3);
        for (var k = 0; k < 14; k++)
        {
            var a = k / 14.0 * Math.PI * 2;
            var d = 6.2 + rnd.NextDouble() * 1.8;
            var x = at.X + 3 * cell + Math.Cos(a) * d * cell;
            var y = at.Y + 4 * cell + Math.Sin(a) * d * cell * 0.8;
            var spark = new Rect(x, y, cell * 0.6, cell * 0.6);
            var shade = 0.35 + rnd.NextDouble() * 0.5;
            if (within.Contains(spark)) ctx.FillRectangle(new SolidColorBrush(BenchTheme.OledOn, shade), spark);
        }
    }

    private void DrawHeader(DrawingContext ctx, Rect area)
    {
        var cell = Wide ? 7.0 : 9.0;
        var markTop = area.Y + 18;
        // The mark sits on OLED glass, as on the keyboard's screen.
        var glass = new Rect(area.X - 6, area.Y + 2, 19.5 * cell, 11.5 * cell);
        ctx.DrawRectangle(_palette("bezel"), new Pen(_palette("plate-edge"), 1), glass, 8, 8);
        // (No PushClip here: under the bitmap's DPI scale it left the transform doubled.)
        DrawMark(ctx, new Point(area.X + 2 * cell, markTop), cell, glass.Deflate(4));
        var x = area.X + 22 * cell;
        ctx.FillRectangle(_palette("ln"), new Rect(x - 2.5 * cell, area.Y + 14, 1.5, Wide ? 70 : 84));
        var title = Text(_options.Title is { Length: > 0 } t ? t : "EMK", Head, Wide ? 76 : 96, "ink");
        ctx.DrawText(title, new Point(x, area.Y - (Wide ? 6 : 10)));
        var subY = area.Y + title.Height - (Wide ? 18 : 24);
        var name = Text(_model.Keyboard, Mono, Wide ? 20 : 24, "acc");
        ctx.DrawText(name, new Point(x + 4, subY));
        var facts = Text($"Corne v3 · 42 keys · {_model.Layers.Count} layers · {_model.Combos.Count} combos · Erik's Mechanical Keyboard",
            Body, Wide ? 17 : 20, "mut");
        ctx.DrawText(facts, new Point(x + 4 + name.Width + 18, subY + (name.Height - facts.Height) / 2));

        // Legend: how to read a key.
        var legendW = Wide ? 330.0 : 350.0;
        var legend = new Rect(area.Right - legendW - (Wide ? 60 : 90), area.Y + 6, legendW, Wide ? 92 : 104);
        var cap = new Rect(legend.X, legend.Y + 4, 66, 66);
        DrawCap(ctx, cap, "Tab", "Alt", false);
        Tracked(ctx, "tap", new Point(cap.Right + 14, cap.Y + 8), HeadSemi, 15, "ink", 2);
        ctx.DrawText(Text("top line: tap the key", Body, 15, "mut"), new Point(cap.Right + 66, cap.Y + 7));
        Tracked(ctx, "hold", new Point(cap.Right + 14, cap.Y + 38), HeadSemi, 15, "ink", 2);
        ctx.DrawText(Text("second line: hold, or mouse mode", Body, 15, "mut"), new Point(cap.Right + 66, cap.Y + 37));
        var muted = Text("Muted legends fall through to the base layer.", Body, 14, "dim");
        ctx.DrawText(muted, new Point(cap.X, cap.Bottom + 10));
    }

    private void DrawCap(DrawingContext ctx, Rect r, string tap, string hold, bool lit)
    {
        ctx.DrawRectangle(_palette(lit ? "k-on" : "k-fill"), new Pen(_palette(lit ? "k-on" : "k-stroke"), 1.2), r, 6, 6);
        var t = Text(tap, HeadSemi, r.Height * 0.3, lit ? "k-on-ink" : "k-ink");
        ctx.DrawText(t, new Point(r.Center.X - t.Width / 2, r.Center.Y - 7 - t.Height / 2));
        var s = Text(hold, Body, r.Height * 0.18, lit ? "k-on-ink" : "k-sub");
        ctx.DrawText(s, new Point(r.Center.X - s.Width / 2, r.Center.Y + 15 - s.Height / 2));
    }

    private void DrawLayers(DrawingContext ctx, Rect area)
    {
        var gap = Wide ? 14.0 : 24.0;
        var cols = 2;
        var rows = (int)Math.Ceiling(_model.Layers.Count / (double)cols);
        var cw = (area.Width - gap * (cols - 1)) / cols;
        var ch = (area.Height - gap * (rows - 1)) / rows;
        for (var i = 0; i < _model.Layers.Count; i++)
        {
            var card = new Rect(area.X + i % cols * (cw + gap), area.Y + i / cols * (ch + gap), cw, ch);
            DrawLayer(ctx, card, _model.Layers[i]);
        }
    }

    private void DrawLayer(DrawingContext ctx, Rect card, PosterLayer layer)
    {
        var tone = LayerTokens[layer.Index % LayerTokens.Length];
        ctx.DrawRectangle(_palette("face-hi"), new Pen(_palette("ln"), 1), card, 10, 10);
        ctx.FillRectangle(_palette(tone), new Rect(card.X, card.Y + 18, 4, 30));

        var number = Text($"{layer.Index}", Mono, 18, tone);
        ctx.DrawText(number, new Point(card.X + 22, card.Y + 22));
        Tracked(ctx, layer.Name, new Point(card.X + 22 + number.Width + 12, card.Y + 17), Head, 26, "ink", 3);
        var reach = Text(layer.Reach, Body, 17, "mut");
        reach.MaxTextWidth = card.Width * 0.55;
        reach.MaxLineCount = 1;
        reach.Trimming = TextTrimming.CharacterEllipsis;
        ctx.DrawText(reach, new Point(card.Right - 22 - reach.Width, card.Y + 25));

        var view = new CorneView(Keymap.KeyCount) { ShowBoardControls = false, SelectMode = true, Palette = _palette, LegendScale = 1.3 };
        view.SetLabels(layer.Labels);
        view.SetTransparent(layer.FallsThrough);
        var drawn = DrawKeyboard(ctx, view, card.Deflate(new Thickness(Wide ? 8 : 14, 60, Wide ? 8 : 14, 12)));
        DrawOledText(ctx, drawn, layer.Name.Length <= 3 ? layer.Name.ToUpperInvariant() : layer.Name[..3].ToUpperInvariant(), 0);
    }

    // 3×5 pixel letters, as on the OLED.
    private static readonly Dictionary<char, string> Font = new()
    {
        ['A'] = "010101111101101", ['B'] = "110101110101110", ['C'] = "011100100100011", ['D'] = "110101101101110",
        ['E'] = "111100110100111", ['F'] = "111100110100100", ['G'] = "011100101101011", ['H'] = "101101111101101",
        ['I'] = "111010010010111", ['J'] = "001001001101010", ['K'] = "101101110101101", ['L'] = "100100100100111",
        ['M'] = "101111111101101", ['N'] = "110101101101101", ['O'] = "010101101101010", ['P'] = "110101110100100",
        ['Q'] = "010101101110011", ['R'] = "110101110101101", ['S'] = "011100010001110", ['T'] = "111010010010010",
        ['U'] = "101101101101111", ['V'] = "101101101101010", ['W'] = "101101111111101", ['X'] = "101101010101101",
        ['Y'] = "101101010010010", ['Z'] = "111001010100111",
    };

    /// <summary>The layer's name on the left OLED, one letter under the other, in OLED pixels.</summary>
    private static void DrawOledText(DrawingContext ctx, (double Scale, Point Origin) drawn, string text, int half)
    {
        var area = CorneView.OledPixelArea(half);
        var cell = area.Width / 32 * 2.6 * drawn.Scale;
        var on = new SolidColorBrush(BenchTheme.OledOn);
        var x0 = drawn.Origin.X + area.X * drawn.Scale + (area.Width * drawn.Scale - 3 * cell) / 2;
        var y = drawn.Origin.Y + area.Y * drawn.Scale + area.Height * drawn.Scale * 0.18;
        foreach (var ch in text)
        {
            if (!Font.TryGetValue(ch, out var g)) continue;
            for (var i = 0; i < 15; i++)
            {
                if (g[i] == '1') ctx.FillRectangle(on, new Rect(x0 + i % 3 * cell, y + i / 3 * cell, cell * 0.82, cell * 0.82));
            }
            y += 6.5 * cell;
        }
    }

    /// <summary>A CorneView's own drawing, scaled into <paramref name="box"/> (centred).</summary>
    private static (double Scale, Point Origin) DrawKeyboard(DrawingContext ctx, CorneView view, Rect box)
    {
        // Fit the cases themselves: the view keeps room around them for the switch and cable.
        var cases = view.CaseBounds;
        var scale = Math.Min(box.Width / cases.Width, box.Height / cases.Height);
        var x = box.X + (box.Width - cases.Width * scale) / 2 - cases.X * scale;
        var y = box.Y + (box.Height - cases.Height * scale) / 2 - cases.Y * scale;
        using (ctx.PushTransform(Matrix.CreateScale(scale, scale) * Matrix.CreateTranslation(x, y)))
        {
            view.Render(ctx);
        }
        return (scale, new Point(x, y));
    }

    private void DrawCombos(DrawingContext ctx, Rect area, bool vertical)
    {
        Tracked(ctx, "Combos", new Point(area.X, area.Y - 2), Head, 22, "ink", 3);
        var note = Text("press the keys together", Body, 16, "mut");
        ctx.DrawText(note, new Point(area.X + 126, area.Y + 3));
        var top = area.Y + 38;
        var combos = _model.Combos;
        var gap = 16.0;
        if (vertical)
        {
            var ch = Math.Min(150, (area.Bottom - top - gap * (combos.Count - 1)) / combos.Count);
            for (var i = 0; i < combos.Count; i++) DrawCombo(ctx, new Rect(area.X, top + i * (ch + gap), area.Width, ch), combos[i], vertical: true);
        }
        else
        {
            var cw = (area.Width - gap * (combos.Count - 1)) / combos.Count;
            for (var i = 0; i < combos.Count; i++) DrawCombo(ctx, new Rect(area.X + i * (cw + gap), top, cw, area.Bottom - top), combos[i], vertical: false);
        }
    }

    private void DrawCombo(DrawingContext ctx, Rect card, PosterCombo combo, bool vertical)
    {
        ctx.DrawRectangle(_palette("face-hi"), new Pen(_palette("ln"), 1), card, 10, 10);
        var view = new CorneView(Keymap.KeyCount) { ShowBoardControls = false, SelectMode = true, Palette = _palette };
        view.SetLabels(new string[Keymap.KeyCount]);
        foreach (var k in combo.Keys) view.SetPressed(k, true);
        var pad = 14.0;
        Rect keys, text;
        if (vertical)
        {
            keys = new Rect(card.X + 10, card.Y + pad, card.Width * 0.33, card.Height - 2 * pad);
            text = new Rect(keys.Right + 12, card.Y + 10, card.Right - keys.Right - 12 - 10, card.Height - 20);
        }
        else
        {
            keys = new Rect(card.X + pad, card.Y + pad - 2, card.Width - 2 * pad, (card.Height - 2 * pad) * 0.42);
            text = new Rect(card.X + pad, keys.Bottom + 10, card.Width - 2 * pad, card.Bottom - keys.Bottom - 10 - pad);
        }
        DrawKeyboard(ctx, view, keys);
        var y = text.Y;
        void Line(string s, Typeface face, double size, string token)
        {
            var t = Text(s, face, size, token);
            t.MaxTextWidth = text.Width;
            t.MaxLineCount = 1;
            t.Trimming = TextTrimming.CharacterEllipsis;
            if (y + t.Height > card.Bottom - 6) return; // nothing crosses the card's edge
            ctx.DrawText(t, new Point(text.X, y));
            y += t.Height + 2;
        }
        if (vertical)
        {
            // Narrow cards: the layers right-aligned on the name line, so the timing stays on one line.
            var layers = Text(combo.Layers, Mono, 11, "mut");
            layers.MaxTextWidth = text.Width * 0.45;
            layers.MaxLineCount = 1;
            layers.Trimming = TextTrimming.CharacterEllipsis;
            ctx.DrawText(layers, new Point(text.Right - layers.Width, y + 5));
            var name = Text(combo.Name, Head, 18, "ink");
            name.MaxTextWidth = text.Width - layers.Width - 8;
            name.MaxLineCount = 1;
            name.Trimming = TextTrimming.CharacterEllipsis;
            ctx.DrawText(name, new Point(text.X, y));
            y += name.Height + 2;
            Line($"{combo.KeyNames} → {combo.Action}", Body, 14.5, "acc");
            Line($"{combo.Timeout} ms{(combo.Idle > 0 ? $" · idle {combo.Idle} ms" : "")}", Mono, 11.5, "mut");
        }
        else
        {
            Line(combo.Name, Head, 20, "ink");
            Line($"{combo.KeyNames}  →  {combo.Action}", Body, 16, "acc");
            Line($"{combo.Timeout} ms{(combo.Idle > 0 ? $" · idle {combo.Idle}" : "")} · {combo.Layers}", Mono, 12.5, "mut");
        }
    }

    private void DrawFooter(DrawingContext ctx, Rect area)
    {
        var left = Text($"EMK · {_model.Keyboard} · keymap of {DateTime.Now:d MMMM yyyy}", Mono, 13, "dim");
        ctx.DrawText(left, area.TopLeft);
        var right = Text("Corne v3 · nice!nano · Rust firmware", Mono, 13, "dim");
        ctx.DrawText(right, new Point(area.Right - right.Width, area.Y));
    }
}


using System.Globalization;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;

namespace KbEmulator.Bench;

/// <summary>Colour of a log line (CSS <c>.lg .la .lb .lv .ld</c>).</summary>
public enum LogTone
{
    Default,
    /// <summary>Dim: idle, background chatter.</summary>
    Dim,
    /// <summary>Green: the host side (computer connected, pairing done).</summary>
    Host,
    /// <summary>Amber: radio, warnings.</summary>
    Radio,
    /// <summary>Cyan: layer changes.</summary>
    Layer,
    /// <summary>Violet: the link between the halves only.</summary>
    Split,
}

/// <summary>
/// A dark screen well with log lines in IBM Plex Mono, coloured by
/// category, with a time column; or (text mode) wrapped text with a block
/// cursor, for what the computer receives. Drawn by hand: thousands of lines
/// stay cheap. Wheel scrolls; at the bottom it follows new lines.
/// </summary>
public sealed class LogWell : Control
{
    private const string DimToken = "log-dim";
    private const double FontSize = 12.5;
    private const double LineHeight = 20;
    private const double PadX = 12, PadY = 10;

    private readonly List<(string Time, string Text, LogTone Tone)> _lines = new();
    private readonly int _maxLines;
    private readonly bool _textMode;
    private readonly bool _msTime;
    private string _text = "";
    private double _scroll; // lines (or px in text mode) up from the bottom
    private static readonly Typeface Face = new(BenchTheme.Mono);

    /// <param name="textMode">Wrapped text with a cursor instead of lines.</param>
    /// <param name="msTime">Times are firmware milliseconds (right-aligned, narrow column).</param>
    public LogWell(int maxLines = 500, bool textMode = false, bool msTime = false)
    {
        _maxLines = maxLines;
        _textMode = textMode;
        _msTime = msTime;
        ClipToBounds = true;
        MinHeight = 60;
        ActualThemeVariantChanged += (_, _) => InvalidateVisual();
        var copy = new MenuItem { Header = "Copy all" };
        copy.Click += (_, _) => _ = ClipboardText.CopyAsync(this, Text);
        ContextMenu = new ContextMenu { Items = { copy } };
    }

    /// <summary>Stay at the newest line.</summary>
    public bool Follow { get; set; } = true;

    /// <summary>Shown dimmed when there is nothing (e.g. "Muted: nothing reaches the PC.").</summary>
    public string Placeholder { get; set; } = "";

    /// <summary>Everything in the well as plain text.</summary>
    public string Text => _textMode ? _text : string.Join('\n', _lines.Select(l => l.Text));

    public int Count => _lines.Count;

    public void SetText(string text)
    {
        _text = text;
        if (Follow) _scroll = 0;
        InvalidateVisual();
    }

    public void Add(string text, string? time = null, LogTone? tone = null)
    {
        _lines.Add((time ?? "", text, tone ?? Classify(text)));
        if (_lines.Count > _maxLines) _lines.RemoveRange(0, _lines.Count - _maxLines);
        if (Follow) _scroll = 0;
        else _scroll += 1; // keep the lines in view where they are
        InvalidateVisual();
    }

    public void Clear()
    {
        _lines.Clear();
        _text = "";
        _scroll = 0;
        InvalidateVisual();
    }

    /// <summary>Which colour a line gets: split violet, host green, layer cyan, radio amber.</summary>
    public static LogTone Classify(string text)
    {
        var t = text.TrimStart();
        if (t.StartsWith("split", StringComparison.OrdinalIgnoreCase) || t.StartsWith("right half linked", StringComparison.Ordinal)) return LogTone.Split;
        if (t.StartsWith("host", StringComparison.OrdinalIgnoreCase) || t.StartsWith("pairing: code typed", StringComparison.Ordinal)) return LogTone.Host;
        if (t.StartsWith("layer", StringComparison.OrdinalIgnoreCase)) return LogTone.Layer;
        if (t.StartsWith("radio", StringComparison.OrdinalIgnoreCase) || t.StartsWith("system: Radio", StringComparison.Ordinal)) return LogTone.Radio;
        if (t.StartsWith("power: idle", StringComparison.Ordinal) || t.StartsWith("listening", StringComparison.Ordinal) || t.StartsWith("────", StringComparison.Ordinal)) return LogTone.Dim;
        return LogTone.Default;
    }

    private static string ToneToken(LogTone tone) => tone switch
    {
        LogTone.Dim => DimToken,
        LogTone.Host => "log-g",
        LogTone.Radio => "log-a",
        LogTone.Layer => "log-b",
        LogTone.Split => "log-v",
        _ => "log",
    };

    private static FormattedText Format(string s, IBrush brush, double maxWidth = double.PositiveInfinity)
    {
        var text = new FormattedText(s, CultureInfo.InvariantCulture, FlowDirection.LeftToRight, Face, FontSize, brush);
        if (!double.IsInfinity(maxWidth))
        {
            text.MaxTextWidth = Math.Max(10, maxWidth);
            text.LineHeight = LineHeight;
        }
        return text;
    }

    protected override Size MeasureOverride(Size availableSize) => new(0, MinHeight);

    public override void Render(DrawingContext context)
    {
        var bounds = new Rect(Bounds.Size);
        context.DrawRectangle(this.Brush("well"), new Pen(this.Brush("well-edge"), 1), bounds.Deflate(0.5), 3, 3);
        // Inner shadow at the top: the well is recessed.
        var shade = new LinearGradientBrush
        {
            StartPoint = new RelativePoint(0, 0, RelativeUnit.Relative),
            EndPoint = new RelativePoint(0, 1, RelativeUnit.Relative),
            GradientStops = { new GradientStop(Color.FromArgb(150, 0, 0, 0), 0), new GradientStop(Color.FromArgb(0, 0, 0, 0), 1) },
        };
        context.FillRectangle(shade, new Rect(1, 1, Math.Max(0, bounds.Width - 2), Math.Min(6, bounds.Height)));

        using var clip = context.PushClip(bounds.Deflate(new Thickness(PadX, PadY - 2, PadX, PadY - 2)));
        var inner = bounds.Deflate(new Thickness(PadX, PadY));
        if (_textMode) RenderText(context, inner);
        else RenderLines(context, inner);
    }

    private void RenderText(DrawingContext ctx, Rect inner)
    {
        if (_text.Length == 0)
        {
            DrawCursor(ctx, inner.TopLeft);
            if (Placeholder.Length > 0) ctx.DrawText(Format(Placeholder, this.Brush(DimToken), inner.Width - 14), inner.TopLeft + new Point(14, 0));
            return;
        }
        using var layout = new Avalonia.Media.TextFormatting.TextLayout(_text, Face, FontSize, this.Brush("log"),
            textWrapping: TextWrapping.Wrap, maxWidth: Math.Max(10, inner.Width - 10), lineHeight: LineHeight);
        var overflow = Math.Max(0, layout.Height - inner.Height);
        _scroll = Math.Clamp(_scroll, 0, overflow);
        var y = inner.Y - overflow + _scroll;
        layout.Draw(ctx, new Point(inner.X, y));
        // Block cursor after the last character.
        var caret = layout.HitTestTextPosition(_text.Length);
        var x = _text.EndsWith('\n') ? 0 : caret.X;
        var cy = _text.EndsWith('\n') ? layout.Height - LineHeight : caret.Y;
        DrawCursor(ctx, new Point(inner.X + x, y + cy));
    }

    private void DrawCursor(DrawingContext ctx, Point at) =>
        ctx.FillRectangle(this.Brush("log-b"), new Rect(at.X, at.Y + 3, 7.5, LineHeight - 5));

    /// <summary>Width of the time column: "+1234 ms" stamps, or clock times; the gap after it.</summary>
    private const double MsTimeWidth = 52, ClockTimeWidth = 96, TimeGap = 10;

    private void RenderLines(DrawingContext ctx, Rect inner)
    {
        if (_lines.Count == 0)
        {
            if (Placeholder.Length > 0) ctx.DrawText(Format(Placeholder, this.Brush(DimToken)), inner.TopLeft);
            return;
        }
        var visible = Math.Max(1, (int)Math.Floor((inner.Height + 4) / LineHeight));
        var maxScroll = Math.Max(0, _lines.Count - visible);
        _scroll = Math.Clamp(_scroll, 0, maxScroll);
        var last = _lines.Count - 1 - (int)_scroll;
        var first = Math.Max(0, last - visible + 1);
        var hasTime = _lines.Any(l => l.Time.Length > 0);
        var timeWidth = 0.0;
        if (hasTime) timeWidth = _msTime ? MsTimeWidth : ClockTimeWidth;
        var textX = hasTime ? timeWidth + TimeGap : 0;
        var y = inner.Y;
        for (var i = first; i <= last; i++)
        {
            var (time, text, tone) = _lines[i];
            if (hasTime && time.Length > 0) DrawTime(ctx, time, inner.X, timeWidth, y);
            var line = Format(text, this.Brush(ToneToken(tone)));
            line.MaxLineCount = 1;
            line.Trimming = TextTrimming.CharacterEllipsis;
            line.MaxTextWidth = Math.Max(10, inner.Width - textX);
            ctx.DrawText(line, new Point(inner.X + textX, y));
            y += LineHeight;
        }
    }

    /// <summary>A line's time: ms stamps right-aligned in the column, clock times left.</summary>
    private void DrawTime(DrawingContext ctx, string time, double left, double timeWidth, double y)
    {
        var t = Format(time, this.Brush(DimToken));
        var x = _msTime ? left + timeWidth - t.Width : left;
        ctx.DrawText(t, new Point(x, y));
    }

    protected override void OnPointerWheelChanged(PointerWheelEventArgs e)
    {
        base.OnPointerWheelChanged(e);
        _scroll += _textMode ? e.Delta.Y * LineHeight * 3 : e.Delta.Y * 3;
        if (_scroll < 0) _scroll = 0;
        InvalidateVisual();
        e.Handled = true;
    }
}

using Avalonia;
using Avalonia.Media.Imaging;

namespace KbEmulator;

/// <summary>Poster sizes: print (landscape, 300 dpi) and a 4K wallpaper.</summary>
public enum PosterFormat
{
    A3Landscape,
    A4Landscape,
    Wallpaper,
}

public static class PosterFormats
{
    /// <summary>Pixel size: A3 and A4 landscape at 300 dpi, 16:9 at 3840×2160.</summary>
    public static PixelSize Pixels(PosterFormat f) => f switch
    {
        PosterFormat.A3Landscape => new PixelSize(4961, 3508),
        PosterFormat.A4Landscape => new PixelSize(3508, 2480),
        _ => new PixelSize(3840, 2160),
    };

    /// <summary>Page size in PDF points (1/72 in).</summary>
    public static Size Points(PosterFormat f) => f switch
    {
        PosterFormat.A3Landscape => new Size(1190.55, 841.89),
        PosterFormat.A4Landscape => new Size(841.89, 595.28),
        _ => new Size(1280, 720),
    };

    public static string Name(PosterFormat f) => f switch
    {
        PosterFormat.A3Landscape => "A3 landscape",
        PosterFormat.A4Landscape => "A4 landscape",
        _ => "Wallpaper 16:9",
    };
}

/// <summary>One layer on the poster: its legends as the app draws them.</summary>
public sealed record PosterLayer(int Index, string Name, string Reach, string[] Labels, bool[] FallsThrough);

/// <summary>One combo on the poster.</summary>
public sealed record PosterCombo(string Name, IReadOnlyList<int> Keys, string KeyNames, string Action, string Layers, int Timeout, int Idle);

/// <summary>What the poster shows: the keymap someone is looking at (Studio's live one), with a title.</summary>
public sealed record PosterModel(string Keyboard, IReadOnlyList<PosterLayer> Layers, IReadOnlyList<PosterCombo> Combos)
{
    /// <summary>
    /// Build from encoded bindings (<c>binding(layer, pos)</c>) and combo texts,
    /// with the firmware's own legends (<see cref="StudioCodec.Label"/>), as Studio draws them.
    /// </summary>
    public static PosterModel From(string keyboard, IReadOnlyList<string> layerNames, Func<int, int, string> binding, IEnumerable<string> combos)
    {
        var layers = Enumerable.Range(0, layerNames.Count).Select(l => Layer(l, layerNames, binding)).ToList();
        var list = combos.Select(StudioCombo.Parse).OfType<StudioCombo>().Select(c => Combo(c, layerNames, binding)).ToList();
        return new PosterModel(keyboard, layers, list);
    }

    private static PosterLayer Layer(int l, IReadOnlyList<string> layerNames, Func<int, int, string> binding)
    {
        var labels = new string[Keymap.KeyCount];
        var through = new bool[Keymap.KeyCount];
        for (var p = 0; p < Keymap.KeyCount; p++)
        {
            var b = binding(l, p);
            through[p] = l > 0 && b.Equals("trans", StringComparison.OrdinalIgnoreCase);
            labels[p] = StudioCodec.Label(through[p] ? binding(0, p) : b);
        }
        return new PosterLayer(l, layerNames[l], Reach(l, layerNames, binding), labels, through);
    }

    private static PosterCombo Combo(StudioCombo c, IReadOnlyList<string> layerNames, Func<int, int, string> binding)
    {
        var first = c.Layers.Count > 0 ? c.Layers.Min : 0;
        var keys = string.Join(" + ", c.Positions.Select(p => StudioCodec.Label(binding(first, p)).Split('\n')[0]));
        var on = c.Layers.Count == 0 ? "all layers" : string.Join(", ", c.Layers.Select(x => layerNames.ElementAtOrDefault(x) ?? $"{x}"));
        return new PosterCombo(c.Name.Length == 0 ? "combo" : c.Name.Replace('_', ' '), c.Positions.ToList(), keys,
            ActionText(c.Action), on, c.Timeout, c.Idle);
    }

    /// <summary>
    /// A combo's action spelled out for the poster: "Ctrl+X" rather than the
    /// key legend "^X", "Profile clear" rather than "BT Clr".
    /// </summary>
    public static string ActionText(string encoding)
    {
        var a = StudioAction.Parse(encoding);
        if (a.Kind == StudioKind.Key && a.Mods.Count > 0)
        {
            var mods = StudioAction.Modifiers.Where(a.Mods.Contains).Select(m => m[1] switch
            {
                'C' => "Ctrl",
                'S' => "Shift",
                'A' => "Alt",
                _ => "Super",
            }).Distinct();
            return string.Join("+", mods.Append(StudioCodec.Label("kp:" + a.Key).Split('\n')[0]));
        }
        if (a.Kind == StudioKind.System)
        {
            return a.System switch
            {
                "BtClear" => "Profile clear",
                "BtNext" => "Profile next",
                "BtPrev" => "Profile previous",
                "OutToggle" => "USB / BT",
                "StudioUnlock" => "Studio unlock",
                "ShowLink" => "show link",
                "RadioPower" => "radio power",
                var other => other,
            };
        }
        return StudioCodec.Label(encoding).Replace('\n', ' ');
    }

    /// <summary>The built-in keymap of the emulator (when nothing is being edited).</summary>
    public static PosterModel Default(EmulatedKeyboard keyboard, string name) =>
        From(name, Keymap.LayerNames(keyboard.LayerName), keyboard.StudioDefault, keyboard.ReadStudioDefaultCombos());

    /// <summary>How to reach a layer: "hold the left outer thumb", "hold symbols + nav".</summary>
    internal static string Reach(int layer, IReadOnlyList<string> names, Func<int, int, string> binding)
    {
        if (layer == 0) return "the base layer";
        var holds = new List<string>();
        for (var l = 0; l < names.Count; l++)
        {
            for (var p = 0; p < Keymap.KeyCount; p++)
            {
                if (binding(l, p).Equals($"mo:{layer}", StringComparison.OrdinalIgnoreCase)) holds.Add(Thumb(p) + (l > 0 ? $" on {names[l]}" : ""));
            }
        }
        if (holds.Count > 0) return "hold " + string.Join(" or ", holds.Distinct());
        var described = Keymap.Describe(names[layer]);
        if (described.Contains('+')) return $"hold {described} together";
        return described.Length > 0 ? described : "conditional layer";
    }

    private static string Thumb(int pos)
    {
        if (pos < 36) return $"key {pos} ({Keymap.Where(pos)})";
        var (left, name) = Keymap.Thumb(pos);
        return $"the {(left ? "left" : "right")} {name} thumb";
    }
}

/// <summary>Poster options from the dialog.</summary>
public sealed record PosterOptions(PosterFormat Format = PosterFormat.A3Landscape, bool Dark = true, bool Combos = true, string? Title = null);

/// <summary>Writes posters as PNG (pixels of the format) or PDF (vector, the format's paper size).</summary>
public static class PosterExport
{
    /// <summary>The poster at <paramref name="scale"/> × its pixel size (1 = full resolution; less for previews).</summary>
    public static RenderTargetBitmap Render(PosterModel model, PosterOptions options, double scale = 1)
    {
        var view = new PosterView(model, options);
        var px = PosterFormats.Pixels(options.Format);
        var size = new PixelSize(Math.Max(1, (int)Math.Round(px.Width * scale)), Math.Max(1, (int)Math.Round(px.Height * scale)));
        var factor = size.Width / PosterView.LayoutWidth;
        view.Measure(new Size(view.Width, view.Height));
        view.Arrange(new Rect(0, 0, view.Width, view.Height));
        var bitmap = new RenderTargetBitmap(size, new Vector(96 * factor, 96 * factor));
        bitmap.Render(view);
        return bitmap;
    }

    /// <summary>Save to <paramref name="path"/>: .pdf writes a vector PDF, anything else a PNG.</summary>
    public static void Save(PosterModel model, PosterOptions options, string path)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(path))!);
        if (path.EndsWith(".pdf", StringComparison.OrdinalIgnoreCase))
        {
            SavePdf(model, options, path);
            return;
        }
        using var bitmap = Render(model, options);
        using var file = File.Create(path);
        bitmap.Save(file, PngBitmapEncoderOptions.Default);
    }

    private static void SavePdf(PosterModel model, PosterOptions options, string path)
    {
        var page = PosterFormats.Points(options.Format);
        // The helper draws one unit per point (it ignores the canvas's scale): lay the poster out at the page size.
        var view = new PosterView(model, options, page.Width / PosterView.LayoutWidth);
        view.Measure(new Size(view.Width, view.Height));
        view.Arrange(new Rect(0, 0, view.Width, view.Height));
        using var stream = File.Create(path);
        using var document = SkiaSharp.SKDocument.CreatePdf(stream);
        using (var canvas = document.BeginPage((float)page.Width, (float)page.Height))
        {
            Avalonia.Skia.Helpers.DrawingContextHelper.RenderAsync(canvas, view).GetAwaiter().GetResult();
        }
        document.EndPage();
        document.Close();
    }

    /// <summary>~/Pictures/emk-keymap-&lt;name&gt;-&lt;theme&gt;.png</summary>
    public static string DefaultPath(string keyboard, bool dark) =>
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyPictures) is { Length: > 0 } pictures
                ? pictures
                : Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), "Pictures"),
            $"emk-keymap-{keyboard}-{(dark ? "dark" : "light")}.png");
}

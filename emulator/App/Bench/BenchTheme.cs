using System.Globalization;
using System.Runtime.CompilerServices;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Styling;

namespace KbEmulator.Bench;

/// <summary>
/// The Bench design tokens (design round 2): an aluminium faceplate in
/// Light, C's slate with a cyan accent in Dark. Every token is a brush
/// resource <c>Bench.&lt;name&gt;</c> in a Light and a Dark theme
/// dictionary, so controls follow the theme variant by themselves.
/// </summary>
public static class BenchTheme
{
    public const string Prefix = "Bench.";

    /// <summary>The accent of each theme: blue on aluminium, cyan on slate.</summary>
    private const string LightAccent = "#2341c4", DarkAccent = "#a3e6f2";

    /// <summary>Light: the aluminium faceplate.</summary>
    public static readonly IReadOnlyDictionary<string, string> Light = new Dictionary<string, string>
    {
        ["face"] = "#d6d7d1", ["face-hi"] = "#e4e5df", ["face-lo"] = "#c9cac3", ["recess"] = "rgba(0,0,0,.16)",
        ["ln"] = "#a3a49c", ["ln-hi"] = "#eff0eb", ["ink"] = "#1a1b19", ["mut"] = "#55564f", ["dim"] = "#7a7b74",
        ["acc"] = LightAccent, ["acc-ink"] = "#ffffff", ["acc-soft"] = "#dfe4fb",
        ["good"] = "#1b7a43", ["warn"] = "#935a02", ["crit"] = "#b3302a", ["vio"] = "#6a4fc4",
        ["good-bg"] = "#cfe8d7", ["warn-bg"] = "#f3e0b9", ["crit-bg"] = "#f4d3cf",
        ["lamp-good"] = "#2fbf63", ["lamp-warn"] = "#f0a21a", ["lamp-crit"] = "#e5483b", ["lamp-off"] = "#a2a39c",
        ["mode-bg"] = "#1a1b19", ["mode-ink"] = "#f3f3ef",
        ["field"] = "#f6f6f2", ["btn"] = "#eaeae5", ["btn-edge"] = "#85867f", ["btn-hl"] = "rgba(255,255,255,.8)",
        ["well"] = "#0e1011", ["well-edge"] = "#8f908a", ["log"] = "#d6dad7", ["log-dim"] = "#7d8483",
        ["log-g"] = "#7fd9a0", ["log-a"] = "#f1c36b", ["log-b"] = "#8fe6ff", ["log-v"] = "#c3b4f5",
        ["gauge-off"] = "#bfc0b9",
        ["k-fill"] = "#ededE7", ["k-stroke"] = "#8f908a", ["k-ink"] = "#1a1b19", ["k-sub"] = "#6a6b65",
        ["k-hover"] = LightAccent, ["k-on"] = LightAccent, ["k-on-ink"] = "#ffffff", ["k-lat-fill"] = "#dfe4fb",
        ["k-sel"] = LightAccent, ["k-none"] = "#dcddd6",
        ["plate"] = "#2a2c2e", ["plate-edge"] = "#121314", ["bezel"] = "#141516", ["hw-ink"] = "#45463f",
        ["plate-ink"] = "#b9bab3",
    };

    /// <summary>Dark: C's slate panels, cyan accent.</summary>
    public static readonly IReadOnlyDictionary<string, string> Dark = new Dictionary<string, string>
    {
        ["face"] = "#1c212a", ["face-hi"] = "#232934", ["face-lo"] = "#161a21", ["recess"] = "rgba(0,0,0,.55)",
        ["ln"] = "#2e3542", ["ln-hi"] = "rgba(255,255,255,.035)", ["ink"] = "#d8dde6", ["mut"] = "#8891a1", ["dim"] = "#626b7b",
        ["acc"] = DarkAccent, ["acc-ink"] = "#0f1318", ["acc-soft"] = "#20343a",
        ["good"] = "#64d28f", ["warn"] = "#e8b34e", ["crit"] = "#ef7370", ["vio"] = "#b9a6f2",
        ["good-bg"] = "rgba(100,210,143,.13)", ["warn-bg"] = "rgba(232,179,78,.14)", ["crit-bg"] = "rgba(239,115,112,.14)",
        ["lamp-good"] = "#64d28f", ["lamp-warn"] = "#e8b34e", ["lamp-crit"] = "#ef7370", ["lamp-off"] = "#3a4250",
        ["mode-bg"] = DarkAccent, ["mode-ink"] = "#0f1318",
        ["field"] = "#11151b", ["btn"] = "#232934", ["btn-edge"] = "#363e4c", ["btn-hl"] = "rgba(255,255,255,.05)",
        ["well"] = "#0c0f14", ["well-edge"] = "#2e3542", ["log"] = "#d8dde6", ["log-dim"] = "#5f6878",
        ["log-g"] = "#64d28f", ["log-a"] = "#e8b34e", ["log-b"] = DarkAccent, ["log-v"] = "#b9a6f2",
        ["gauge-off"] = "#2a313d",
        ["k-fill"] = "#232934", ["k-stroke"] = "#363e4c", ["k-ink"] = "#d3d9e3", ["k-sub"] = "#6b7487",
        ["k-hover"] = DarkAccent, ["k-on"] = DarkAccent, ["k-on-ink"] = "#0f1318", ["k-lat-fill"] = "#20343a",
        ["k-sel"] = DarkAccent, ["k-none"] = "#1a1f27",
        ["plate"] = "#12161c", ["plate-edge"] = "#2e3542", ["bezel"] = "#07090c", ["hw-ink"] = "#8891a1",
        ["plate-ink"] = "#8891a1",
    };

    /// <summary>Lamp glow strength (CSS <c>--glow</c>).</summary>
    public static double Glow(ThemeVariant variant) => variant == ThemeVariant.Dark ? 1 : 0.7;

    private const string Fonts = "avares://KbEmulator/Assets/Fonts";

    /// <summary>UI text.</summary>
    // Barlow has no ✓ ✗ ⚡ ◀ ▶ or arrows: Noto Sans Symbols 2 and IBM Plex Mono fill in, before the OS fallback.
    private const string Symbols = $"{Fonts}#Noto Sans Symbols 2, {Fonts}#IBM Plex Mono";

    public static readonly FontFamily Ui = new($"{Fonts}#Barlow, {Symbols}, Noto Sans, sans-serif");

    /// <summary>Labels (uppercase, tracked), legends, big numbers.</summary>
    public static readonly FontFamily Label = new($"{Fonts}#Barlow Semi Condensed, {Fonts}#Barlow, {Symbols}, Noto Sans, sans-serif");

    /// <summary>Logs and numbers.</summary>
    public static readonly FontFamily Mono = new($"{Fonts}#IBM Plex Mono, {Fonts}#Noto Sans Symbols 2, DejaVu Sans Mono, monospace");

    /// <summary>The OLED's lit pixels (same in both themes).</summary>
    public static readonly Color OledOn = Color.Parse("#8fe6ff");

    public static Color ParseColor(string css)
    {
        if (!css.StartsWith("rgba(", StringComparison.Ordinal)) return Color.Parse(css);
        var parts = css[5..^1].Split(',').Select(p => double.Parse(p, CultureInfo.InvariantCulture)).ToArray();
        return Color.FromArgb((byte)Math.Round(parts[3] * 255), (byte)parts[0], (byte)parts[1], (byte)parts[2]);
    }

    /// <summary>The theme dictionaries with every token as a brush, plus the Fluent keys the stock controls read.</summary>
    public static ResourceDictionary CreateResources()
    {
        var resources = new ResourceDictionary();
        resources.ThemeDictionaries[ThemeVariant.Light] = Dictionary(Light);
        resources.ThemeDictionaries[ThemeVariant.Dark] = Dictionary(Dark);
        return resources;
    }

    private static ResourceDictionary Dictionary(IReadOnlyDictionary<string, string> tokens)
    {
        var d = new ResourceDictionary();
        var brushes = tokens.ToDictionary(t => t.Key, t => (IBrush)new SolidColorBrush(ParseColor(t.Value)));
        foreach (var (name, brush) in brushes) d[Prefix + name] = brush;

        // Fluent's own keys, so the stock controls left (text field, slider, scroll bars, menus) match.
        void Fluent(string token, params string[] keys)
        {
            foreach (var key in keys) d[key] = brushes[token];
        }
        Fluent("field", "TextControlBackground", "TextControlBackgroundPointerOver", "TextControlBackgroundFocused");
        Fluent("ink", "TextControlForeground", "TextControlForegroundPointerOver", "TextControlForegroundFocused");
        Fluent("btn-edge", "TextControlBorderBrush", "TextControlBorderBrushPointerOver");
        Fluent("acc", "TextControlBorderBrushFocused", "SliderTrackValueFill", "SliderTrackValueFillPointerOver",
            "SliderTrackValueFillPressed", "SliderThumbBackground", "SliderThumbBackgroundPointerOver", "SliderThumbBackgroundPressed",
            "TextControlSelectionHighlightColor");
        Fluent("mut", "TextControlPlaceholderForeground", "TextControlPlaceholderForegroundPointerOver", "TextControlPlaceholderForegroundFocused");
        Fluent("gauge-off", "SliderTrackFill", "SliderTrackFillPointerOver", "SliderTrackFillPressed");
        Fluent("face-hi", "MenuFlyoutPresenterBackground");
        Fluent("ln", "MenuFlyoutPresenterBorderBrush");
        Fluent("ink", "MenuFlyoutItemForeground", "MenuFlyoutItemForegroundPointerOver");
        Fluent("face-lo", "MenuFlyoutItemBackgroundPointerOver");
        d["TextControlBorderThemeThickness"] = new Thickness(1);
        d["ControlCornerRadius"] = new CornerRadius(3);
        return d;
    }
}

/// <summary>
/// Tokens applied in code: a property bound to <c>Bench.&lt;token&gt;</c>
/// that follows the theme variant (and the tree the control is in).
/// </summary>
public static class Tokens
{
    private sealed class Bindings
    {
        public readonly Dictionary<AvaloniaProperty, string> Map = new();
    }

    private static readonly ConditionalWeakTable<StyledElement, Bindings> All = new();

    /// <summary>Bind <paramref name="property"/> to the brush token (e.g. "face-hi").</summary>
    public static T Tok<T>(this T element, AvaloniaProperty property, string token) where T : StyledElement
    {
        if (!All.TryGetValue(element, out var bindings))
        {
            bindings = new Bindings();
            All.Add(element, bindings);
            element.ActualThemeVariantChanged += (_, _) => Apply(element, bindings);
            if (element is Visual visual) visual.AttachedToVisualTree += (_, _) => Apply(element, bindings);
        }
        bindings.Map[property] = token;
        if (Find(element, token) is { } brush) element.SetValue(property, brush);
        return element;
    }

    /// <summary>The brush for a token as this element sees it (its theme variant).</summary>
    public static IBrush Brush(this StyledElement element, string token) =>
        Find(element, token) ?? Fallback(element, token);

    public static Color ColorOf(this StyledElement element, string token) =>
        element.Brush(token) is ISolidColorBrush s ? s.Color : Colors.Magenta;

    private static IBrush? Find(StyledElement element, string token)
    {
        var key = BenchTheme.Prefix + token;
        if (element.TryFindResource(key, element.ActualThemeVariant, out var value) && value is IBrush b) return b;
        if (Application.Current is { } app && app.TryFindResource(key, element.ActualThemeVariant, out value) && value is IBrush ab) return ab;
        return null;
    }

    private static IBrush Fallback(StyledElement element, string token)
    {
        var set = element.ActualThemeVariant == ThemeVariant.Light ? BenchTheme.Light : BenchTheme.Dark;
        return set.TryGetValue(token, out var css) ? new SolidColorBrush(BenchTheme.ParseColor(css)) : Brushes.Magenta;
    }

    private static void Apply(StyledElement element, Bindings bindings)
    {
        foreach (var (property, token) in bindings.Map)
        {
            if (Find(element, token) is { } brush) element.SetValue(property, brush);
        }
    }
}

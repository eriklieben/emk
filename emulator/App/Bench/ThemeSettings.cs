using System.ComponentModel;
using System.Diagnostics;
using System.Diagnostics.CodeAnalysis;
using System.Text.Json;
using System.Text.Json.Nodes;
using Avalonia;
using Avalonia.Platform;
using Avalonia.Styling;

namespace KbEmulator.Bench;

/// <summary>Which theme the user picked in the top bar.</summary>
public enum ThemePreference
{
    /// <summary>Follow the OS (the default).</summary>
    System = 0,
    Light = 1,
    Dark = 2,
}

/// <summary>
/// The app's small settings file, <c>~/.config/emk/app.json</c>
/// (<c>$XDG_CONFIG_HOME/emk/app.json</c>):
/// <code>
/// {
///   "theme": "system",          // system | light | dark (the top bar's switch writes it)
///   "features": {
///     "security": false         // true shows the Security mode (hidden by default)
///   }
/// }
/// </code>
/// Features can also be switched on for one run with the environment:
/// <c>EMK_FEATURES=security</c> (a comma list; <c>-security</c> switches one
/// off). Fields the app doesn't know are kept when it saves.
/// </summary>
public sealed class AppSettings
{
    private static readonly JsonSerializerOptions Json = new() { WriteIndented = true };

    public ThemePreference Theme { get; set; } = ThemePreference.System;

    /// <summary>Feature flags from the file's <c>features</c> object (name → on).</summary>
    public Dictionary<string, bool> Features { get; } = new(StringComparer.OrdinalIgnoreCase);

    public static string DefaultPath =>
        Path.Combine(Environment.GetEnvironmentVariable("XDG_CONFIG_HOME") is { Length: > 0 } xdg
            ? xdg
            : Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".config"), "emk", "app.json");

    /// <summary>The settings in <paramref name="path"/>; defaults when it is missing or unreadable.</summary>
    public static AppSettings Load(string path)
    {
        var settings = new AppSettings();
        if (ReadObject(path) is not { } root) return settings;
        if (root["theme"] is JsonValue theme && theme.TryGetValue<string>(out var name) &&
            Enum.TryParse<ThemePreference>(name, ignoreCase: true, out var pref) && Enum.IsDefined(pref))
        {
            settings.Theme = pref;
        }
        if (root["features"] is JsonObject features)
        {
            foreach (var (key, value) in features)
            {
                if (value is JsonValue v && v.TryGetValue<bool>(out var on)) settings.Features[key] = on;
            }
        }
        return settings;
    }

    private static JsonObject? ReadObject(string path)
    {
        try
        {
            return File.Exists(path) ? JsonNode.Parse(File.ReadAllText(path)) as JsonObject : null;
        }
        catch (Exception e) when (e is IOException or JsonException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    /// <summary>
    /// Write the theme (and any features set here) into the file, keeping
    /// everything else in it; false (and nothing thrown) when it can't be written.
    /// </summary>
    public bool Save(string path)
    {
        try
        {
            var root = ReadObject(path) ?? new JsonObject();
            root["theme"] = Theme.ToString().ToLowerInvariant();
            if (Features.Count > 0)
            {
                var features = root["features"] as JsonObject ?? new JsonObject();
                foreach (var (key, on) in Features) features[key] = on;
                root["features"] = features;
            }
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, root.ToJsonString(Json));
            return true;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            return false;
        }
    }
}

/// <summary>
/// Optional parts of the app, off unless the settings or <c>EMK_FEATURES</c>
/// switch them on (see <see cref="AppSettings"/> for how).
/// </summary>
public sealed class FeatureFlags
{
    /// <summary>The Security mode (bootloaders, signing key, debug port, tamper rules).</summary>
    public const string Security = "security";

    private readonly HashSet<string> _on;

    public FeatureFlags(IEnumerable<string>? on = null) => _on = new HashSet<string>(on ?? [], StringComparer.OrdinalIgnoreCase);

    public bool IsOn(string feature) => _on.Contains(feature);

    /// <summary>The file's features, then <c>EMK_FEATURES</c> (<c>security</c> on, <c>-security</c> off).</summary>
    public static FeatureFlags Load(string settingsPath, Func<string, string?>? env = null)
    {
        env ??= Environment.GetEnvironmentVariable;
        var on = AppSettings.Load(settingsPath).Features.Where(f => f.Value).Select(f => f.Key).ToHashSet(StringComparer.OrdinalIgnoreCase);
        foreach (var item in (env("EMK_FEATURES") ?? "").Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            if (item.StartsWith('-')) on.Remove(item[1..]);
            else on.Add(item);
        }
        return new FeatureFlags(on);
    }
}

/// <summary>
/// What the OS prefers, for when Avalonia can't tell. Avalonia reads the
/// freedesktop portal (<c>org.freedesktop.appearance color-scheme</c>); when
/// the portal isn't there it reports Light. Then GTK_THEME (<c>Adwaita:dark</c>)
/// and GNOME's <c>color-scheme</c> / <c>gtk-theme</c> settings decide.
/// </summary>
public static class SystemTheme
{
    /// <summary>Dark, Light, or null when nothing says.</summary>
    public static ThemeVariant? FromDesktop(Func<string, string?>? env = null, Func<string, string?>? gsettings = null)
    {
        env ??= Environment.GetEnvironmentVariable;
        gsettings ??= Gsettings;
        if (env("GTK_THEME") is { Length: > 0 } gtk) return LooksDark(gtk) ? ThemeVariant.Dark : ThemeVariant.Light;
        var scheme = gsettings("color-scheme")?.Trim('\'', ' ', '\n');
        if (scheme == "prefer-dark") return ThemeVariant.Dark;
        if (scheme == "prefer-light") return ThemeVariant.Light;
        if (gsettings("gtk-theme") is { Length: > 0 } theme && LooksDark(theme)) return ThemeVariant.Dark;
        return null;
    }

    private static bool LooksDark(string theme) => theme.Contains("dark", StringComparison.OrdinalIgnoreCase);

    [SuppressMessage("Security", "S4036:Searching OS commands in PATH is security-sensitive",
        Justification = "The desktop's own gsettings, wherever the distribution installs it; read-only query.")]
    private static string? Gsettings(string key)
    {
        try
        {
            var start = new ProcessStartInfo("gsettings", ["get", "org.gnome.desktop.interface", key])
            {
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
            };
            using var process = Process.Start(start);
            if (process == null) return null;
            var output = process.StandardOutput.ReadToEnd();
            if (!process.WaitForExit(1500)) return null;
            return process.ExitCode == 0 ? output.Trim() : null;
        }
        catch (Exception e) when (e is Win32Exception or InvalidOperationException or IOException)
        {
            return null; // no gsettings (not GNOME): no hint
        }
    }
}

/// <summary>
/// Applies a <see cref="ThemePreference"/> to the application and keeps
/// following the OS while it is System.
/// </summary>
public sealed class ThemeController
{
    private readonly Application _app;
    private readonly string _path;
    private readonly Func<ThemeVariant?> _desktop;
    private ThemeVariant? _desktopCache;
    private bool _desktopRead;

    /// <param name="settingsPath">Where the choice is stored; default <see cref="AppSettings.DefaultPath"/>.</param>
    /// <param name="desktop">The desktop's preference when Avalonia can't tell (tests pass their own).</param>
    public ThemeController(Application app, string? settingsPath = null, Func<ThemeVariant?>? desktop = null)
    {
        _app = app;
        _path = settingsPath ?? AppSettings.DefaultPath;
        _desktop = desktop ?? (() => SystemTheme.FromDesktop());
        Preference = AppSettings.Load(_path).Theme;
        if (_app.PlatformSettings is { } platform)
        {
            platform.ColorValuesChanged += (_, _) =>
            {
                _desktopRead = false;
                Apply();
            };
        }
        Apply();
    }

    public ThemePreference Preference { get; private set; }

    /// <summary>What decided the System theme, for the events pane.</summary>
    public string Source { get; private set; } = "";

    public event Action? Changed;

    public void Set(ThemePreference preference)
    {
        Preference = preference;
        new AppSettings { Theme = preference }.Save(_path);
        Apply();
    }

    private void Apply()
    {
        _app.RequestedThemeVariant = Preference switch
        {
            ThemePreference.Light => ThemeVariant.Light,
            ThemePreference.Dark => ThemeVariant.Dark,
            _ => SystemVariant(),
        };
        Changed?.Invoke();
    }

    /// <summary>
    /// The OS theme: Avalonia's when it says Dark (the portal answered);
    /// otherwise the desktop settings, else Default (Avalonia's own guess).
    /// </summary>
    private ThemeVariant SystemVariant()
    {
        var platform = _app.PlatformSettings?.GetColorValues();
        if (platform?.ThemeVariant == PlatformThemeVariant.Dark)
        {
            Source = "OS theme: dark (Avalonia, freedesktop portal)";
            return ThemeVariant.Default;
        }
        if (!_desktopRead)
        {
            _desktopCache = _desktop();
            _desktopRead = true;
        }
        if (_desktopCache is { } desktop)
        {
            Source = $"OS theme: {(desktop == ThemeVariant.Dark ? "dark" : "light")} (GTK_THEME / gsettings; Avalonia reported {platform?.ThemeVariant.ToString() ?? "nothing"})";
            return desktop;
        }
        Source = $"OS theme: {platform?.ThemeVariant.ToString().ToLowerInvariant() ?? "unknown"} (Avalonia)";
        return ThemeVariant.Default;
    }
}

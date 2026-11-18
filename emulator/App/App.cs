using Avalonia;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Themes.Fluent;
using KbEmulator.Bench;

namespace KbEmulator;

public sealed class App : Application
{
    /// <summary>The theme choice (System / Light / Dark), remembered in ~/.config/emk/app.json.</summary>
    public static ThemeController? Theme { get; private set; }

    /// <summary>Optional parts of the app (Security is off unless switched on; see <see cref="AppSettings"/>).</summary>
    public static FeatureFlags Features { get; private set; } = new();

    public override void Initialize()
    {
        Styles.Add(new FluentTheme());
        Resources.MergedDictionaries.Add(BenchTheme.CreateResources());
        // EMK_SETTINGS: another settings file (tests use their own).
        var settings = Environment.GetEnvironmentVariable("EMK_SETTINGS") is { Length: > 0 } path ? path : null;
        Theme = new ThemeController(this, settings);
        Features = FeatureFlags.Load(settings ?? AppSettings.DefaultPath);
    }

    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
        {
            desktop.MainWindow = new MainWindow();
        }
        base.OnFrameworkInitializationCompleted();
    }
}

public static class Program
{
    [STAThread]
    public static void Main(string[] args) =>
        AppBuilder.Configure<App>()
            .UsePlatformDetect()
            .LogToTrace()
            .StartWithClassicDesktopLifetime(args);
}

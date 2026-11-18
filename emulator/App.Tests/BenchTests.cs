using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Input;
using Avalonia.Media;
using Avalonia.Styling;
using KbEmulator.Bench;

namespace KbEmulator.Tests;

/// <summary>The theme choice and where it is kept.</summary>
[NotInParallel("ui-session")]
public class ThemeSettingsTests
{
    private static string TempSettings() => Path.Combine(Path.GetTempPath(), $"emk-settings-{Guid.NewGuid():N}", "app.json");

    [Test]
    public async Task Theme_follows_the_system_when_nothing_is_saved()
    {
        // Arrange
        var path = TempSettings();

        // Act
        var settings = AppSettings.Load(path);

        // Assert
        await Assert.That(settings.Theme).IsEqualTo(ThemePreference.System);
    }

    [Test]
    public async Task Saved_theme_is_loaded_again()
    {
        // Arrange
        var path = TempSettings();
        new AppSettings { Theme = ThemePreference.Light }.Save(path);

        // Act
        var settings = AppSettings.Load(path);

        // Assert
        await Assert.That(settings.Theme).IsEqualTo(ThemePreference.Light);
    }

    [Test]
    public async Task Unreadable_settings_fall_back_to_the_system_theme()
    {
        // Arrange
        var path = TempSettings();
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        await File.WriteAllTextAsync(path, "{ not json");

        // Act
        var settings = AppSettings.Load(path);

        // Assert
        await Assert.That(settings.Theme).IsEqualTo(ThemePreference.System);
    }

    [Test]
    public async Task Choosing_a_theme_in_the_app_is_remembered()
    {
        // Arrange
        var path = TempSettings();
        var preference = await TestAppBuilder.Session.Dispatch(() =>
        {
            var app = Application.Current!;
            var before = app.RequestedThemeVariant;
            new ThemeController(app, path, () => null).Set(ThemePreference.Dark);

            // Act: the next start
            var next = new ThemeController(app, path, () => null).Preference;
            app.RequestedThemeVariant = before;
            return next;
        }, CancellationToken.None);

        // Assert
        await Assert.That(preference).IsEqualTo(ThemePreference.Dark);
    }

    [Test]
    public async Task System_theme_uses_the_desktop_when_avalonia_reports_light()
    {
        // Arrange: headless Avalonia reports no dark theme; the desktop says dark
        var path = TempSettings();

        // Act
        var variant = await TestAppBuilder.Session.Dispatch(() =>
        {
            var app = Application.Current!;
            var before = app.RequestedThemeVariant;
            _ = new ThemeController(app, path, () => ThemeVariant.Dark);
            var applied = app.RequestedThemeVariant;
            app.RequestedThemeVariant = before;
            return applied;
        }, CancellationToken.None);

        // Assert
        await Assert.That(variant).IsEqualTo(ThemeVariant.Dark);
    }

    [Test]
    public async Task System_theme_without_any_hint_is_left_to_avalonia()
    {
        // Arrange
        var path = TempSettings();

        // Act
        var variant = await TestAppBuilder.Session.Dispatch(() =>
        {
            var app = Application.Current!;
            var before = app.RequestedThemeVariant;
            _ = new ThemeController(app, path, () => null);
            var applied = app.RequestedThemeVariant;
            app.RequestedThemeVariant = before;
            return applied;
        }, CancellationToken.None);

        // Assert
        await Assert.That(variant).IsEqualTo(ThemeVariant.Default);
    }

    [Test]
    [Arguments("Adwaita:dark", null, "Dark")]
    [Arguments(null, "'prefer-dark'", "Dark")]
    [Arguments(null, "'prefer-light'", "Light")]
    [Arguments(null, "'default'", "none")]
    public async Task Desktop_theme_is_read_from_gtk_theme_and_gsettings(string? gtkTheme, string? colorScheme, string expected)
    {
        // Arrange
        string? Env(string name) => name == "GTK_THEME" ? gtkTheme : null;
        string? Gsettings(string key) => key == "color-scheme" ? colorScheme : null;

        // Act
        var variant = SystemTheme.FromDesktop(Env, Gsettings);

        // Assert
        await Assert.That(variant?.ToString() ?? "none").IsEqualTo(expected);
    }
}

/// <summary>Bench tokens follow the theme variant.</summary>
[NotInParallel("ui-session")]
public class TokenTests
{
    [Test]
    public async Task A_token_background_differs_between_light_and_dark()
    {
        // Arrange
        var (light, dark) = await TestAppBuilder.Session.Dispatch(() =>
        {
            var border = new Border { Width = 10, Height = 10 }.Tok(Border.BackgroundProperty, "face");
            var window = new Window { Content = border, RequestedThemeVariant = ThemeVariant.Light };
            window.Show();
            var l = ((ISolidColorBrush)border.Background!).Color;

            // Act
            window.RequestedThemeVariant = ThemeVariant.Dark;
            var d = ((ISolidColorBrush)border.Background!).Color;
            window.Close();
            return (l, d);
        }, CancellationToken.None);

        // Assert
        await Assert.That((light.ToString(), dark.ToString()))
            .IsEqualTo((Color.Parse(BenchTheme.Light["face"]).ToString(), Color.Parse(BenchTheme.Dark["face"]).ToString()));
    }

    [Test]
    public async Task Every_token_exists_in_both_themes()
    {
        // Arrange
        var light = BenchTheme.Light.Keys.Order().ToArray();

        // Act
        var dark = BenchTheme.Dark.Keys.Order().ToArray();

        // Assert
        await Assert.That(dark).IsEquivalentTo(light);
    }

    [Test]
    public async Task Css_rgba_becomes_a_color_with_alpha()
    {
        // Arrange
        const string css = "rgba(100,210,143,.13)";

        // Act
        var color = BenchTheme.ParseColor(css);

        // Assert
        await Assert.That((color.A, color.R, color.G, color.B)).IsEqualTo(((byte)33, (byte)100, (byte)210, (byte)143));
    }
}

/// <summary>Log lines are coloured by what they are about.</summary>
public class LogToneTests
{
    [Test]
    [Arguments("split: linked", LogTone.Split)]
    [Arguments("host: profile 1 computer reconnected", LogTone.Host)]
    [Arguments("layer → symbols", LogTone.Layer)]
    [Arguments("radio: +4 dBm", LogTone.Radio)]
    [Arguments("kbnotify → notification shown on the right half", LogTone.Default)]
    public async Task Lines_get_the_colour_of_their_category(string line, LogTone expected)
    {
        // Arrange
        var text = line;

        // Act
        var tone = LogWell.Classify(text);

        // Assert
        await Assert.That(tone).IsEqualTo(expected);
    }
}

/// <summary>Which mode gets this PC's keys.</summary>
[NotInParallel("ui-session")]
public class ModeRoutingTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;
    private const int PosA = 13;
    private const int UsageA = 0x04;

    private static MainWindow NewWindow() =>
        new(Path.Combine(Path.GetTempPath(), $"kb-mode-test-{Guid.NewGuid():N}.sock"), new HardwarePort(() => null));

    [Test]
    public async Task Emulator_mode_plays_a_key_on_the_corne()
    {
        // Arrange
        var (used, pressed, typed) = await Session.Dispatch(() =>
        {
            var window = NewWindow();

            // Act
            var u = window.RoutePhysicalKey(PhysicalKey.A, true);
            var result = (u, window.View.PressedHere.Contains(PosA), window.Keyboard.LastReport.Contains(UsageA));
            window.Close();
            return result;
        }, CancellationToken.None);

        // Assert
        await Assert.That((used, pressed, typed)).IsEqualTo((true, true, true));
    }

    [Test]
    public async Task Other_modes_leave_the_key_alone()
    {
        // Arrange
        var (used, pressed, typed) = await Session.Dispatch(() =>
        {
            var window = NewWindow();
            window.ShowMode(AppMode.Studio);

            // Act
            var u = window.RoutePhysicalKey(PhysicalKey.A, true);
            var result = (u, window.View.PressedHere.Contains(PosA), window.Keyboard.LastReport.Contains(UsageA));
            window.Close();
            return result;
        }, CancellationToken.None);

        // Assert
        await Assert.That((used, pressed, typed)).IsEqualTo((false, false, false));
    }

    [Test]
    public async Task Leaving_the_emulator_releases_held_and_latched_keys()
    {
        // Arrange
        var (held, latched, typed) = await Session.Dispatch(() =>
        {
            var window = NewWindow();
            window.RoutePhysicalKey(PhysicalKey.A, true);
            window.View.LatchForTest(24);

            // Act
            window.ShowMode(AppMode.Flash);
            var result = (window.View.PressedHere.Count, window.View.Latched.Count, window.Keyboard.LastReport.Contains(UsageA));
            window.Close();
            return result;
        }, CancellationToken.None);

        // Assert
        await Assert.That((held, latched, typed)).IsEqualTo((0, 0, false));
    }

    [Test]
    public async Task Debug_mode_takes_keys_only_while_driving()
    {
        // Arrange
        var (notDriving, driving, emulatorTyped) = await Session.Dispatch(() =>
        {
            var window = NewWindow();
            window.ShowMode(AppMode.Debug);
            var off = window.RoutePhysicalKey(PhysicalKey.A, true);
            window.RoutePhysicalKey(PhysicalKey.A, false);
            window.Debug.Driving = true;

            // Act
            var on = window.RoutePhysicalKey(PhysicalKey.A, true);
            var result = (off, on, window.Keyboard.LastReport.Contains(UsageA));
            window.RoutePhysicalKey(PhysicalKey.A, false);
            window.Close();
            return result;
        }, CancellationToken.None);

        // Assert: the key goes to the real keyboard, never the emulator
        await Assert.That((notDriving, driving, emulatorTyped)).IsEqualTo((false, true, false));
    }

    [Test]
    public async Task Mode_switch_in_the_top_bar_changes_the_mode()
    {
        // Arrange
        var mode = await Session.Dispatch(() =>
        {
            var window = NewWindow();

            // Act
            window.Bar.Modes.Choose((int)AppMode.Debug);
            var m = window.Mode;
            window.Close();
            return m;
        }, CancellationToken.None);

        // Assert
        await Assert.That(mode).IsEqualTo(AppMode.Debug);
    }
}

/// <summary>Debug mode's readout.</summary>
[NotInParallel("ui-session")]
public class DebugReadoutTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    [Test]
    public async Task Fields_the_firmware_does_not_send_show_a_dash()
    {
        // Arrange: a status without radio=
        const string status = "#S layer=0 caps=0 mods=00 out=ble profile=1 host=1 open=0 linked=1 stage=- " +
                              "bat=87,90 chg=1,0 wpm=0 power=active presses=0 mute=0 trace=0 lit=1\r\n";
        var (radio, bootloader) = await Session.Dispatch(() =>
        {
            var view = new DebugView(_ => "base", 42, new HardwarePort(() => null));

            // Act
            view.Feed(status);
            var result = (view.Readout("Radio"), view.Readout("Bootloader"));
            view.Dispose();
            return result;
        }, CancellationToken.None);

        // Assert
        await Assert.That((radio, bootloader)).IsEqualTo(("—", "—"));
    }

    [Test]
    public async Task Boot_log_line_fills_in_the_bootloader()
    {
        // Arrange
        const string line = "  1204 boot: bootloader 0.11.0, entry serial DFU (signed), debug port open\r\n";
        var bootloader = await Session.Dispatch(() =>
        {
            var view = new DebugView(_ => "base", 42, new HardwarePort(() => null));

            // Act
            view.Feed(line);
            var result = view.Readout("Bootloader");
            view.Dispose();
            return result;
        }, CancellationToken.None);

        // Assert
        await Assert.That(bootloader).IsEqualTo("0.11.0 signed");
    }

    [Test]
    public async Task Bootloader_line_is_parsed()
    {
        // Arrange
        const string line = "boot: bootloader 0.6.0, entry UF2, debug port locked";

        // Act
        var parsed = DebugView.ParseBootloaderLine(line);

        // Assert
        await Assert.That(parsed).IsEqualTo(("0.6.0", "UF2", "locked"));
    }
}

/// <summary>Setting the emulated radio level from the rack.</summary>
public class RadioTests
{
    [Test]
    [Arguments(8)]
    [Arguments(4)]
    [Arguments(0)]
    public async Task Radio_level_can_be_set_to_each_step(int dbm)
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard(() => 0);

        // Act
        var done = keyboard.SetRadioDbm(dbm);

        // Assert
        await Assert.That((done, keyboard.RadioDbm)).IsEqualTo((true, dbm));
    }
}

/// <summary>Feature flags: Security is hidden unless switched on.</summary>
public class FeatureFlagTests
{
    private static string TempSettings(string? json = null)
    {
        var path = Path.Combine(Path.GetTempPath(), $"emk-features-{Guid.NewGuid():N}", "app.json");
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        if (json != null) File.WriteAllText(path, json);
        return path;
    }

    [Test]
    public async Task Security_is_off_by_default()
    {
        // Arrange
        var path = TempSettings();

        // Act
        var flags = FeatureFlags.Load(path, _ => null);

        // Assert
        await Assert.That(flags.IsOn(FeatureFlags.Security)).IsFalse();
    }

    [Test]
    public async Task The_settings_file_switches_security_on()
    {
        // Arrange
        var path = TempSettings("""{ "theme": "dark", "features": { "security": true } }""");

        // Act
        var flags = FeatureFlags.Load(path, _ => null);

        // Assert
        await Assert.That(flags.IsOn(FeatureFlags.Security)).IsTrue();
    }

    [Test]
    [Arguments("security", false, true)]
    [Arguments("-security", true, false)]
    [Arguments("other,security", false, true)]
    public async Task Emk_features_overrides_the_file(string env, bool inFile, bool expected)
    {
        // Arrange
        var path = TempSettings($$"""{ "features": { "security": {{(inFile ? "true" : "false")}} } }""");

        // Act
        var flags = FeatureFlags.Load(path, name => name == "EMK_FEATURES" ? env : null);

        // Assert
        await Assert.That(flags.IsOn(FeatureFlags.Security)).IsEqualTo(expected);
    }

    [Test]
    public async Task Saving_the_theme_keeps_features_and_unknown_fields()
    {
        // Arrange
        var path = TempSettings("""{ "theme": "dark", "features": { "security": true }, "later": { "x": 1 } }""");
        var settings = AppSettings.Load(path);
        settings.Theme = ThemePreference.Light;

        // Act
        settings.Save(path);

        // Assert
        var root = System.Text.Json.Nodes.JsonNode.Parse(await File.ReadAllTextAsync(path))!;
        await Assert.That((root["theme"]!.GetValue<string>(), root["features"]!["security"]!.GetValue<bool>(), root["later"]!["x"]!.GetValue<int>()))
            .IsEqualTo(("light", true, 1));
    }

    [Test]
    public async Task The_theme_switch_keeps_the_features_in_the_file()
    {
        // Arrange
        var path = TempSettings("""{ "features": { "security": true } }""");

        // Act
        new AppSettings { Theme = ThemePreference.Dark }.Save(path);

        // Assert
        await Assert.That(AppSettings.Load(path).Features[FeatureFlags.Security]).IsTrue();
    }
}

/// <summary>The window without the Security feature.</summary>
[NotInParallel("ui-session")]
public class HiddenSecurityTests
{
    private static Task<T> WithWindow<T>(FeatureFlags flags, Func<MainWindow, T> act) => TestAppBuilder.Session.Dispatch(() =>
    {
        var window = new MainWindow(Path.Combine(Path.GetTempPath(), $"kb-flags-{Guid.NewGuid():N}.sock"), new HardwarePort(() => null), features: flags);
        window.Show();
        try
        {
            return act(window);
        }
        finally
        {
            window.Close();
        }
    }, CancellationToken.None);

    [Test]
    public async Task The_mode_switch_has_no_security_by_default()
    {
        // Arrange
        var flags = new FeatureFlags();

        // Act
        var modes = await WithWindow(flags, w => w.ModeNames());

        // Assert
        await Assert.That(modes).IsEquivalentTo(new[] { "Emulator", "Debug", "Studio", "Flash" });
    }

    [Test]
    public async Task With_the_feature_security_is_in_the_switch()
    {
        // Arrange
        var flags = new FeatureFlags([FeatureFlags.Security]);

        // Act
        var modes = await WithWindow(flags, w => w.ModeNames());

        // Assert
        await Assert.That(modes).Contains("Security");
    }

    [Test]
    public async Task Arrow_right_from_flash_stays_on_flash()
    {
        // Arrange
        var mode = await WithWindow(new FeatureFlags(), w =>
        {
            w.ShowMode(AppMode.Flash);

            // Act
            w.Bar.Modes.RaiseEvent(new Avalonia.Input.KeyEventArgs
            {
                RoutedEvent = Avalonia.Input.InputElement.KeyDownEvent, Key = Avalonia.Input.Key.Right, Source = w.Bar.Modes,
            });
            return w.Mode;
        });

        // Assert
        await Assert.That(mode).IsEqualTo(AppMode.Flash);
    }

    [Test]
    public async Task Security_cannot_be_shown_when_hidden()
    {
        // Arrange
        var mode = await WithWindow(new FeatureFlags(), w =>
        {
            // Act
            w.ShowMode(AppMode.Security);
            return w.Mode;
        });

        // Assert
        await Assert.That(mode).IsEqualTo(AppMode.Emulator);
    }

    [Test]
    public async Task Choosing_the_last_segment_shows_flash_not_security()
    {
        // Arrange
        var mode = await WithWindow(new FeatureFlags(), w =>
        {
            // Act
            w.Bar.Modes.Choose(w.Bar.ShownModes.Count - 1);
            return w.Mode;
        });

        // Assert
        await Assert.That(mode).IsEqualTo(AppMode.Flash);
    }
}

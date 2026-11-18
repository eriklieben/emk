using System.Runtime.InteropServices;
using Avalonia;
using Avalonia.Headless;
using Avalonia.Media.Imaging;
using Avalonia.Styling;
using Avalonia.Threading;
using KbEmulator.Bench;

namespace KbEmulator.Tests;

public static class TestAppBuilder
{
    public static AppBuilder BuildAvaloniaApp()
    {
        // The app's theme choice must not come from (or go to) the user's settings.
        var dir = Path.Combine(Path.GetTempPath(), $"emk-test-{Guid.NewGuid():N}");
        Environment.SetEnvironmentVariable("EMK_SETTINGS", Path.Combine(dir, "app.json"));
        Environment.SetEnvironmentVariable("EMK_EMULATOR_KEYMAP", Path.Combine(dir, "emulator-keymap.txt"));
        // Security is hidden by default; the tests that render or drive it need it.
        Environment.SetEnvironmentVariable("EMK_FEATURES", FeatureFlags.Security);
        return AppBuilder.Configure<App>()
            .UseSkia()
            .UseHeadless(new AvaloniaHeadlessPlatformOptions { UseHeadlessDrawing = false });
    }

    /// <summary>One headless session for every test class (Avalonia can only start once).</summary>
    public static readonly HeadlessUnitTestSession Session = HeadlessUnitTestSession.StartNew(typeof(TestAppBuilder), AvaloniaTestIsolationLevel.PerAssembly);
}

/// <summary>
/// Renders the real window off-screen with Skia, in both Bench themes.
/// Pictures are saved next to the test binary (main-window-*.png) for a
/// visual check.
/// </summary>
[NotInParallel("ui-session")] // timing tests must not wait behind renders on the one UI thread
public class MainWindowRenderTests
{
    private const string DebugStream =
        "#H 1 kb_b left ble\r\n#K 0000000000001002\r\n" +
        "#S layer=1 caps=0 mods=02 out=ble profile=1 host=1 open=0 linked=1 stage=linked bat=87,90 chg=1,0 wpm=40 power=active presses=12 mute=1 trace=0 lit=1 radio=4\r\n" +
        "#R 0200040000000000\r\n  1204 boot: bootloader 0.11.0, entry serial DFU (signed), debug port open\r\n" +
        "  1388 radio: +4 dBm\r\n  4410 host: profile 1 computer reconnected\r\n  6102 split: scanning\r\n  6870 split: linked\r\n  29960 power: idle\r\n";

    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    [Test]
    public async Task Upright_screens_light_the_left_oled_beside_the_inner_column()
    {
        // Arrange
        var path = Out("main-window-portrait.png");

        // Act
        var litPixels = await Session.Dispatch(
            () => RenderWindow(path, new Scene(ThemeVariant.Dark) { Portrait = true, Notification = "Build passed on main" }), CancellationToken.None);

        // Assert
        await Assert.That(litPixels).IsGreaterThan(50);
    }

    [Test]
    public async Task Zmk_screens_light_the_left_oled_beside_the_inner_column()
    {
        // Arrange
        var path = Out("main-window-zmk.png");

        // Act
        var litPixels = await Session.Dispatch(() => RenderWindow(path, new Scene(ThemeVariant.Dark) { Portrait = false }), CancellationToken.None);

        // Assert
        await Assert.That(litPixels).IsGreaterThan(50);
    }

    [Test]
    public async Task Holding_symbols_and_nav_shows_the_wm_layer()
    {
        // Arrange
        var path = Out("main-window-wm.png");

        // Act
        var litPixels = await Session.Dispatch(
            () => RenderWindow(path, new Scene(ThemeVariant.Dark) { Portrait = true, HoldKeys = [36, 41] }), CancellationToken.None);

        // Assert: the left OLED still shows the status (now with layer WM)
        await Assert.That(litPixels).IsGreaterThan(50);
    }

    [Test]
    [Arguments("light")]
    [Arguments("dark")]
    public async Task Emulator_renders_in_both_themes_with_the_oled_lit(string theme)
    {
        // Arrange
        var path = Out($"main-window-{theme}.png");

        // Act
        var litPixels = await Session.Dispatch(
            () => RenderWindow(path, new Scene(Variant(theme)) { Portrait = true, Notification = "Build passed", Latch = 24, HoldKeys = [35] }),
            CancellationToken.None);

        // Assert: the OLED keeps its lit pixels on the light faceplate too
        await Assert.That(litPixels).IsGreaterThan(50);
    }

    [Test]
    [Arguments("light")]
    [Arguments("dark")]
    public async Task Debug_renders_in_both_themes_with_the_keyboards_frame(string theme)
    {
        // Arrange
        var path = Out($"main-window-debug-{theme}.png");

        // Act
        var litPixels = await Session.Dispatch(
            () => RenderWindow(path, new Scene(Variant(theme)) { Mode = AppMode.Debug, Stream = DebugStream }), CancellationToken.None);

        // Assert: the right OLED is drawn from the forwarded status
        await Assert.That(litPixels).IsGreaterThan(50);
    }

    [Test]
    [Arguments(AppMode.Studio, "light")]
    [Arguments(AppMode.Studio, "dark")]
    [Arguments(AppMode.Flash, "light")]
    [Arguments(AppMode.Flash, "dark")]
    [Arguments(AppMode.Security, "light")]
    [Arguments(AppMode.Security, "dark")]
    public async Task Studio_flash_and_security_render_in_both_themes(AppMode mode, string theme)
    {
        // Arrange
        var path = Out($"main-window-{mode.ToString().ToLowerInvariant()}-{theme}.png");

        // Act
        await Session.Dispatch(() => RenderWindow(path, new Scene(Variant(theme)) { Mode = mode }), CancellationToken.None);

        // Assert
        await Assert.That(new FileInfo(path).Length).IsGreaterThan(10_000);
    }

    [Test]
    [MethodDataSource(nameof(NarrowCases))]
    public async Task Every_mode_renders_at_1100_and_760_px(AppMode mode, string theme, int width)
    {
        // Arrange
        var path = Out($"main-window-{mode.ToString().ToLowerInvariant()}-{theme}-{width}.png");

        // Act
        var size = await Session.Dispatch(
            () => RenderWindow(path, new Scene(Variant(theme)) { Mode = mode, Width = width, Stream = mode == AppMode.Debug ? DebugStream : null, SizeOnly = true }),
            CancellationToken.None);

        // Assert: nothing makes the window wider than asked
        await Assert.That(size).IsEqualTo(width);
    }

    public static IEnumerable<Func<(AppMode, string, int)>> NarrowCases()
    {
        foreach (var mode in Enum.GetValues<AppMode>())
        {
            foreach (var theme in new[] { "light", "dark" })
            {
                foreach (var width in new[] { 1100, 760 }) yield return () => (mode, theme, width);
            }
        }
    }

    [Test]
    [Arguments("light")]
    [Arguments("dark")]
    public async Task Studio_editor_renders_a_key_being_edited(string theme)
    {
        // Arrange
        var path = Out($"main-window-studio-edit-{theme}.png");

        // Act
        var message = await Session.Dispatch(() =>
        {
            var app = Avalonia.Application.Current!;
            var previous = app.RequestedThemeVariant;
            app.RequestedThemeVariant = Variant(theme);
            var window = new MainWindow(Path.Combine(Path.GetTempPath(), $"kb-studio-{Guid.NewGuid():N}.sock"), new HardwarePort(() => null));
            window.Show();
            window.ShowMode(AppMode.Studio);
            window.Studio.Pump();
            window.Keyboard.PressStudioUnlock();
            window.Studio.Client.Status();
            window.Studio.Select(0, 13);
            window.Studio.EditForTest(e => e.Mods.Add("LG"));
            window.Studio.Apply(); // A becomes Gui+A: a changed key
            window.Studio.Pump();
            window.Studio.Select(0, 27);
            window.Studio.EditForTest(e =>
            {
                e.Kind = StudioKind.ModTap;
                e.Hold = "LCTRL";
            });
            Dispatcher.UIThread.RunJobs();
            var size = new PixelSize((int)window.Bounds.Width, (int)window.Bounds.Height);
            using var bitmap = new RenderTargetBitmap(size);
            bitmap.Render(window);
            using (var file = File.Create(path)) bitmap.Save(file, PngBitmapEncoderOptions.Default);
            var m = window.Studio.Client.Binding(0, 13);
            window.Close();
            app.RequestedThemeVariant = previous;
            return m;
        }, CancellationToken.None);

        // Assert: the first edit was accepted
        await Assert.That(message).IsEqualTo("kp:LG(A)");
    }

    [Test]
    [Arguments("light")]
    [Arguments("dark")]
    public async Task Studio_combo_editor_renders(string theme)
    {
        // Arrange
        var path = Out($"main-window-studio-combo-{theme}.png");

        // Act
        var keys = await Session.Dispatch(() =>
        {
            var app = Avalonia.Application.Current!;
            var previous = app.RequestedThemeVariant;
            app.RequestedThemeVariant = Variant(theme);
            var window = new MainWindow(Path.Combine(Path.GetTempPath(), $"kb-combo-{Guid.NewGuid():N}.sock"), new HardwarePort(() => null));
            window.Show();
            window.ShowMode(AppMode.Studio);
            window.Studio.Pump();
            window.Studio.Pump();
            window.Keyboard.PressStudioUnlock();
            window.Studio.Pump();
            window.Studio.EditCombo(1);
            window.Studio.ToggleComboKey(28);
            Dispatcher.UIThread.RunJobs();
            var size = new PixelSize((int)window.Bounds.Width, (int)window.Bounds.Height);
            using var bitmap = new RenderTargetBitmap(size);
            bitmap.Render(window);
            using (var file = File.Create(path)) bitmap.Save(file, PngBitmapEncoderOptions.Default);
            var k = string.Join(",", window.Studio.Combo!.Positions);
            window.Close();
            app.RequestedThemeVariant = previous;
            return k;
        }, CancellationToken.None);

        // Assert: copy (X + C) with V clicked in
        await Assert.That(keys).IsEqualTo("26,27,28");
    }

    private static string Out(string name) => Path.Combine(AppContext.BaseDirectory, name);

    private static ThemeVariant Variant(string theme) => theme == "light" ? ThemeVariant.Light : ThemeVariant.Dark;

    /// <summary>What a render shows: theme, screens, a notification, keys held or latched, the mode, its width.</summary>
    private sealed record Scene(ThemeVariant Theme)
    {
        public bool Portrait { get; init; } = true;
        public string? Notification { get; init; }
        public int[]? HoldKeys { get; init; }
        public int? Latch { get; init; }
        public AppMode Mode { get; init; } = AppMode.Emulator;
        public string? Stream { get; init; }
        public int Width { get; init; } = 1440;
        /// <summary>Only the window's width is wanted (returned instead of lit pixels).</summary>
        public bool SizeOnly { get; init; }
    }

    /// <summary>Show the window, render it to <paramref name="path"/>, return lit pixels in the OLED checked.</summary>
    private static int RenderWindow(string path, Scene scene)
    {
        var (theme, portrait, notification, holdKeys, latch) = (scene.Theme, scene.Portrait, scene.Notification, scene.HoldKeys, scene.Latch);
        var (mode, debugStream, width, sizeOnly) = (scene.Mode, scene.Stream, scene.Width, scene.SizeOnly);
        var app = Avalonia.Application.Current!;
        var previous = app.RequestedThemeVariant;
        app.RequestedThemeVariant = theme;
        var socket = Path.Combine(Path.GetTempPath(), $"kb-render-test-{Guid.NewGuid():N}.sock");
        var window = new MainWindow(socket, new HardwarePort(() => null)) { Width = width };
        try
        {
            window.Keyboard.SetPortraitScreens(portrait);
            if (notification != null)
            {
                foreach (var packet in Kb.HostProtocol.NotificationPackets.Encode(notification)) window.Keyboard.RawHid(packet);
            }
            foreach (var key in holdKeys ?? [])
            {
                window.View.SetPressed(key, true);
                window.Keyboard.Press(key);
            }
            if (latch is { } l) window.View.LatchForTest(l);
            window.ShowMode(mode);
            if (debugStream != null)
            {
                // The port's "not found" first (from its thread), then the stream.
                var until = DateTime.UtcNow + TimeSpan.FromSeconds(3);
                do { window.Debug.Pump(); Thread.Sleep(5); } while (!window.Debug.ConnectionText.StartsWith("Not connected:", StringComparison.Ordinal) && DateTime.UtcNow < until);
                window.Debug.Feed(debugStream);
                window.Debug.Pump();
            }
            window.Keyboard.UpdateDisplays();
            window.Show();
            window.ApplyWidth(width);
            Dispatcher.UIThread.RunJobs();

            var size = new PixelSize((int)window.Bounds.Width, (int)window.Bounds.Height);
            using var bitmap = new RenderTargetBitmap(size);
            bitmap.Render(window);
            using (var file = File.Create(path)) bitmap.Save(file, PngBitmapEncoderOptions.Default);
            if (sizeOnly) return size.Width;
            // The view is scaled to fit the window: map its OLED area into window pixels.
            var view = mode == AppMode.Debug ? window.Debug.View : window.View;
            var half = mode == AppMode.Debug ? 1 : 0;
            if (mode is not (AppMode.Emulator or AppMode.Debug)) return 0;
            var transform = view.TransformToVisual(window) ?? throw new InvalidOperationException("view not in window");
            var area = CorneView.OledPixelArea(half).TransformToAABB(transform);
            return CountBrightPixels(bitmap, size, new PixelRect((int)area.X, (int)area.Y, (int)area.Width, (int)area.Height));
        }
        finally
        {
            window.Close();
            app.RequestedThemeVariant = previous;
        }
    }

    private static int CountBrightPixels(Bitmap bitmap, PixelSize size, PixelRect area)
    {
        var stride = size.Width * 4;
        var pixels = new byte[stride * size.Height];
        var handle = GCHandle.Alloc(pixels, GCHandleType.Pinned);
        try
        {
            bitmap.CopyPixels(new PixelRect(size), handle.AddrOfPinnedObject(), pixels.Length, stride);
        }
        finally
        {
            handle.Free();
        }
        var count = 0;
        for (var y = area.Y; y < area.Bottom; y++)
        {
            for (var x = area.X; x < area.Right; x++)
            {
                var i = y * stride + x * 4;
                if (Math.Max(pixels[i], Math.Max(pixels[i + 1], pixels[i + 2])) > 60) count++; // glass is ~10
            }
        }
        return count;
    }
}

using Avalonia;
using Avalonia.Styling;
using KbEmulator.Bench;

namespace KbEmulator.DocsMedia;

/// <summary>The app's modes, rendered like the render tests (MainWindowRenderTests) at 1440 × 940.</summary>
internal static class Screenshots
{
    private const string DebugStream =
        "#H 1 erik_kbd left ble\r\n#K 0000000000001002\r\n" +
        "#S layer=1 caps=0 mods=02 out=ble profile=1 host=1 open=0 linked=1 stage=linked bat=87,90 chg=1,0 wpm=40 power=active presses=12 mute=1 trace=0 lit=1 radio=4\r\n" +
        "#R 0200040000000000\r\n  1204 boot: bootloader 0.11.0, entry serial DFU (signed), debug port open\r\n" +
        "  1388 radio: +4 dBm\r\n  4410 host: profile 1 computer reconnected\r\n  6102 split: scanning\r\n  6870 split: linked\r\n  29960 power: idle\r\n";

    public static void All(string media)
    {
        Shot(Path.Combine(media, "app-emulator.png"), ThemeVariant.Dark, AppMode.Emulator);
        Shot(Path.Combine(media, "app-emulator-light.png"), ThemeVariant.Light, AppMode.Emulator);
        Shot(Path.Combine(media, "app-debug.png"), ThemeVariant.Dark, AppMode.Debug);
        Shot(Path.Combine(media, "app-studio.png"), ThemeVariant.Dark, AppMode.Studio);
        Shot(Path.Combine(media, "app-flash.png"), ThemeVariant.Dark, AppMode.Flash);
    }

    private static void Shot(string path, ThemeVariant theme, AppMode mode)
    {
        var app = Application.Current!;
        var previous = app.RequestedThemeVariant;
        app.RequestedThemeVariant = theme;
        uint now = 0;
        var socket = Path.Combine(Path.GetTempPath(), $"emk-docs-{Guid.NewGuid():N}.sock");
        var window = new MainWindow(socket, new HardwarePort(() => null), () => now) { Width = 1440, Height = 940 };
        try
        {
            window.Show();
            window.ApplyWidth(1440);
            void Run(int ms)
            {
                for (var i = 0; i < ms; i += 5)
                {
                    now += 5;
                    window.Tick();
                }
            }
            // Past the start-up animation, a few words typed, a message on the right half.
            Run(4000);
            foreach (var pos in new[] { 18, 3, 21, 21, 9, 38, 3, 31, 20 })
            {
                window.View.SetPressed(pos, true);
                window.Keyboard.Press(pos);
                Run(60);
                window.View.SetPressed(pos, false);
                window.Keyboard.Release(pos);
                Run(120);
            }
            foreach (var packet in Kb.HostProtocol.NotificationPackets.Encode("Build passed on main")) window.Keyboard.RawHid(packet);
            Run(2500);

            window.ShowMode(mode);
            switch (mode)
            {
                case AppMode.Debug:
                    var until = DateTime.UtcNow + TimeSpan.FromSeconds(3);
                    do { window.Debug.Pump(); Thread.Sleep(5); }
                    while (!window.Debug.ConnectionText.StartsWith("Not connected:", StringComparison.Ordinal) && DateTime.UtcNow < until);
                    window.Debug.Feed(DebugStream);
                    window.Debug.Pump();
                    break;
                case AppMode.Studio:
                    window.Studio.Pump();
                    window.Keyboard.PressStudioUnlock();
                    window.Studio.Client.Status();
                    window.Studio.Pump();
                    window.Studio.Select(2, 37);
                    break;
            }
            Run(100);
            Program.Render(window, path, 1);
            Console.WriteLine($"{new FileInfo(path).Length,9}  {Path.GetRelativePath(Environment.CurrentDirectory, path)}");
        }
        finally
        {
            window.Close();
            app.RequestedThemeVariant = previous;
        }
    }
}

using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Media.Imaging;
using Avalonia.Threading;

namespace KbEmulator.DocsMedia;

/// <summary>
/// The README's pictures that need the app: keyboard scenes (the app's
/// CorneView driven by the emulated firmware on a simulated clock, written
/// as PNG frames for <c>docs-media assemble</c>) and screenshots of the app's
/// modes. Headless (Skia, no window on screen), the same way the render tests work.
/// <code>EmkDocsMedia &lt;media dir&gt; &lt;frames dir&gt; [--smoke]</code>
/// </summary>
public static class Program
{
    public static AppBuilder BuildAvaloniaApp() =>
        AppBuilder.Configure<App>()
            .UseSkia()
            .UseHeadless(new AvaloniaHeadlessPlatformOptions { UseHeadlessDrawing = false });

    public static int Main(string[] args)
    {
        if (args.Length < 2)
        {
            Console.Error.WriteLine("usage: EmkDocsMedia <media dir> <frames dir> [--smoke]");
            return 2;
        }
        var (media, frames, smoke) = (Path.GetFullPath(args[0]), Path.GetFullPath(args[1]), args.Contains("--smoke"));
        Directory.CreateDirectory(media);
        Directory.CreateDirectory(frames);

        // Nothing from (or to) the user's settings, keymap edits, keys or keyboards.
        // A fixed folder, so the Flash screenshot shows the same paths every time.
        var temp = Path.Combine(Path.GetTempPath(), "emk-docs-media");
        if (Directory.Exists(temp)) Directory.Delete(temp, true);
        Directory.CreateDirectory(temp);
        Environment.SetEnvironmentVariable("EMK_SETTINGS", Path.Combine(temp, "app.json"));
        Environment.SetEnvironmentVariable("EMK_EMULATOR_KEYMAP", Path.Combine(temp, "emulator-keymap.txt"));
        Environment.SetEnvironmentVariable("EMK_KEY", DemoRepo.Key(Path.Combine(temp, "keys")));
        Environment.SetEnvironmentVariable("EMK_REPO", DemoRepo.Create(Path.Combine(temp, "repo")));

        using var session = HeadlessUnitTestSession.StartNew(typeof(Program), AvaloniaTestIsolationLevel.PerAssembly);
        try
        {
            session.Dispatch(() =>
            {
                if (smoke)
                {
                    Scenes.Smoke(frames);
                    return;
                }
                foreach (var scene in Scenes.All) scene(frames);
                Scenes.Layers(media);
                Scenes.Combos(media);
                Screenshots.All(media);
                PosterMedia.Write(media);
            }, CancellationToken.None).GetAwaiter().GetResult();
        }
        finally
        {
            try
            {
                Directory.Delete(temp, true);
            }
            catch (IOException)
            {
                // Left in the temp folder: the next run starts by deleting it.
            }
        }
        return 0;
    }

    internal static void Render(Control control, string path, double scale)
    {
        Dispatcher.UIThread.RunJobs();
        var size = new PixelSize((int)Math.Ceiling(control.Bounds.Width * scale), (int)Math.Ceiling(control.Bounds.Height * scale));
        using var bitmap = new RenderTargetBitmap(size, new Vector(96 * scale, 96 * scale));
        bitmap.Render(control);
        using var file = File.Create(path);
        bitmap.Save(file, PngBitmapEncoderOptions.Default);
    }
}

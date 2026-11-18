using Avalonia.Media.Imaging;

namespace KbEmulator.DocsMedia;

/// <summary>
/// The keymap poster for the README (<c>keymap-poster.png</c>): the built-in
/// keymap, dark, as the 16:9 wallpaper downscaled to 1920×1080 so it stays light.
/// The app's "Export poster…" makes the full-size ones.
/// </summary>
internal static class PosterMedia
{
    public static void Write(string media, string keyboard = "erik_kbd")
    {
        using var emulator = new EmulatedKeyboard(() => 0);
        var model = PosterModel.Default(emulator, keyboard);
        using var bitmap = PosterExport.Render(model, new PosterOptions(PosterFormat.Wallpaper, Dark: true), scale: 0.5);
        using var file = File.Create(Path.Combine(media, "keymap-poster.png"));
        bitmap.Save(file, PngBitmapEncoderOptions.Default);
    }
}

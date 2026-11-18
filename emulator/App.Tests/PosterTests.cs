using Avalonia.Headless;
using Avalonia.Media;
using KbEmulator.Bench;

namespace KbEmulator.Tests;

/// <summary>The keymap poster.</summary>
[NotInParallel("ui-session")]
public class PosterTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    private static PosterModel DefaultModel()
    {
        using var keyboard = new EmulatedKeyboard(() => 0);
        return PosterModel.Default(keyboard, "kb_a");
    }

    [Test]
    [Arguments(PosterFormat.A3Landscape, 4961, 3508)]
    [Arguments(PosterFormat.A4Landscape, 3508, 2480)]
    [Arguments(PosterFormat.Wallpaper, 3840, 2160)]
    public async Task Each_preset_renders_at_its_pixel_size(PosterFormat format, int width, int height)
    {
        // Arrange
        var model = await Session.Dispatch(DefaultModel, CancellationToken.None);

        // Act
        var size = await Session.Dispatch(() =>
        {
            using var bitmap = PosterExport.Render(model, new PosterOptions(format));
            return (bitmap.PixelSize.Width, bitmap.PixelSize.Height);
        }, CancellationToken.None);

        // Assert
        await Assert.That(size).IsEqualTo((width, height));
    }

    [Test]
    [Arguments(PosterFormat.A3Landscape, true)]
    [Arguments(PosterFormat.A3Landscape, false)]
    [Arguments(PosterFormat.A4Landscape, true)]
    [Arguments(PosterFormat.Wallpaper, true)]
    [Arguments(PosterFormat.Wallpaper, false)]
    public async Task Saving_writes_the_poster_file(PosterFormat format, bool dark)
    {
        // Arrange
        var model = await Session.Dispatch(DefaultModel, CancellationToken.None);
        var path = Path.Combine(AppContext.BaseDirectory, $"poster-{format.ToString().ToLowerInvariant()}-{(dark ? "dark" : "light")}.png");

        // Act
        await Session.Dispatch(() => PosterExport.Save(model, new PosterOptions(format, dark), path), CancellationToken.None);

        // Assert
        await Assert.That(new FileInfo(path).Length).IsGreaterThan(100_000);
    }

    [Test]
    public async Task A_pdf_is_a_vector_pdf_file()
    {
        // Arrange
        var model = await Session.Dispatch(DefaultModel, CancellationToken.None);
        var path = Path.Combine(AppContext.BaseDirectory, "poster-a3landscape-dark.pdf");

        // Act
        await Session.Dispatch(() => PosterExport.Save(model, new PosterOptions(PosterFormat.A3Landscape), path), CancellationToken.None);

        // Assert
        var head = System.Text.Encoding.ASCII.GetString((await File.ReadAllBytesAsync(path))[..5]);
        await Assert.That((head, new FileInfo(path).Length > 20_000)).IsEqualTo(("%PDF-", true));
    }

    [Test]
    public async Task An_edited_key_appears_on_the_poster()
    {
        // Arrange
        var label = await Session.Dispatch(() =>
        {
            using var keyboard = new EmulatedKeyboard(() => 0);
            var studio = new StudioView(keyboard);
            studio.Activate();
            studio.Pump();
            studio.UnlockButton.PerformClick();
            studio.Pump();
            studio.Select(0, 13);
            studio.EditForTest(e => e.Key = "B");
            studio.Apply(); // applied, not saved
            studio.Pump();

            // Act
            var model = studio.PosterModel("kb_a");
            return model.Layers[0].Labels[13];
        }, CancellationToken.None);

        // Assert
        await Assert.That(label).IsEqualTo("B");
    }

    [Test]
    public async Task An_added_combo_appears_on_the_poster()
    {
        // Arrange
        var names = await Session.Dispatch(() =>
        {
            using var keyboard = new EmulatedKeyboard(() => 0);
            var studio = new StudioView(keyboard);
            studio.Activate();
            studio.Pump();
            studio.UnlockButton.PerformClick();
            studio.Pump();
            studio.Client.ComboAdd("hello 1,2 * 40 0 caps");
            studio.Pump();

            // Act
            return studio.PosterModel("x").Combos.Select(c => c.Name).ToList();
        }, CancellationToken.None);

        // Assert
        await Assert.That(names).Contains("hello");
    }

    [Test]
    public async Task Nav_shows_the_mouse_mode_legend_as_the_second_line()
    {
        // Arrange
        var model = await Session.Dispatch(DefaultModel, CancellationToken.None);

        // Act
        var thumb = model.Layers[2].Labels[37];

        // Assert
        await Assert.That(thumb).IsEqualTo("Bksp\nL click");
    }

    [Test]
    [Arguments(1, "hold the left outer thumb")]
    [Arguments(2, "hold the right outer thumb")]
    [Arguments(3, "hold symbols + nav together")]
    public async Task Each_layer_says_how_to_reach_it(int layer, string reach)
    {
        // Arrange
        var model = await Session.Dispatch(DefaultModel, CancellationToken.None);

        // Act
        var text = model.Layers[layer].Reach;

        // Assert
        await Assert.That(text).IsEqualTo(reach);
    }

    [Test]
    [Arguments(PosterFormat.A3Landscape)]
    [Arguments(PosterFormat.Wallpaper)]
    public async Task The_combos_panel_follows_the_toggle(PosterFormat format)
    {
        // Arrange
        var model = await Session.Dispatch(DefaultModel, CancellationToken.None);

        // Act
        var (with, without) = await Session.Dispatch(() =>
            (new PosterView(model, new PosterOptions(format, Combos: true)).Areas().Combos,
             new PosterView(model, new PosterOptions(format, Combos: false)).Areas().Combos), CancellationToken.None);

        // Assert
        await Assert.That((with.HasValue, without.HasValue)).IsEqualTo((true, false));
    }

    [Test]
    public async Task Light_and_dark_use_the_bench_tokens()
    {
        // Arrange
        var model = await Session.Dispatch(DefaultModel, CancellationToken.None);

        // Act
        var (dark, light) = await Session.Dispatch(() =>
            (((ISolidColorBrush)new PosterView(model, new PosterOptions(Dark: true)).Brush("face-lo")).Color,
             ((ISolidColorBrush)new PosterView(model, new PosterOptions(Dark: false)).Brush("face-lo")).Color), CancellationToken.None);

        // Assert
        await Assert.That((dark.ToString(), light.ToString()))
            .IsEqualTo((Color.Parse(BenchTheme.Dark["face-lo"]).ToString(), Color.Parse(BenchTheme.Light["face-lo"]).ToString()));
    }

    [Test]
    public async Task The_dialog_suggests_a_file_in_pictures()
    {
        // Arrange
        var path = await Session.Dispatch(() =>
        {
            var dialog = new PosterDialog(name => DefaultModel() with { Keyboard = name }, "kb_a", dark: true);

            // Act
            var p = dialog.SuggestedPath;
            dialog.Close();
            return p;
        }, CancellationToken.None);

        // Assert
        await Assert.That(path).EndsWith(Path.Combine("Pictures", "emk-keymap-kb_a-dark.png"));
    }

    [Test]
    public async Task The_dialog_preview_follows_the_format()
    {
        // Arrange
        var (a3, wallpaper) = await Session.Dispatch(() =>
        {
            var dialog = new PosterDialog(name => DefaultModel() with { Keyboard = name }, "kb_a", dark: true);
            var first = dialog.Preview!.PixelSize;

            // Act
            dialog.Update(dialog.Options with { Format = PosterFormat.Wallpaper });
            var r = (first, dialog.Preview!.PixelSize);
            dialog.Close();
            return r;
        }, CancellationToken.None);

        // Assert: a quarter of 4961×3508, then of 3840×2160
        await Assert.That((a3.Width, wallpaper.Width, wallpaper.Height)).IsEqualTo((1240, 960, 540));
    }

    [Test]
    public async Task The_dialog_saves_the_file()
    {
        // Arrange
        var path = Path.Combine(Path.GetTempPath(), $"emk-poster-{Guid.NewGuid():N}", "poster.png");

        // Act
        await Session.Dispatch(() =>
        {
            var dialog = new PosterDialog(name => DefaultModel() with { Keyboard = name }, "kb_a", dark: false);
            dialog.SaveTo(path);
            dialog.Close();
        }, CancellationToken.None);

        // Assert
        await Assert.That(File.Exists(path)).IsTrue();
    }

    [Test]
    [Arguments("kp:LC(X)", "Ctrl+X")]
    [Arguments("kp:LG(LS(N1))", "Super+Shift+1")]
    [Arguments("sys:BtClear", "Profile clear")]
    [Arguments("caps", "Caps Word")]
    [Arguments("kp:ESC", "Esc")]
    public async Task Combo_actions_are_spelled_out(string encoding, string text)
    {
        // Arrange
        var e = encoding;

        // Act
        var shown = PosterModel.ActionText(e);

        // Assert
        await Assert.That(shown).IsEqualTo(text);
    }

    [Test]
    public async Task The_cut_combo_reads_ctrl_x()
    {
        // Arrange
        var model = await Session.Dispatch(DefaultModel, CancellationToken.None);

        // Act
        var cut = model.Combos.First(c => c.Name == "cut").Action;

        // Assert
        await Assert.That(cut).IsEqualTo("Ctrl+X");
    }
}

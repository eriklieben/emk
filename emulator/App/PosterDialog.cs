using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Platform.Storage;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>
/// "Export poster…": format, theme, keyboard name, combos on or off, a live
/// preview, and Save (PNG, or PDF by the file name).
/// </summary>
public sealed class PosterDialog : Window
{
    private static readonly PosterFormat[] Formats = [PosterFormat.A3Landscape, PosterFormat.A4Landscape, PosterFormat.Wallpaper];

    private readonly Func<string, PosterModel> _model;
    private readonly Image _preview = new() { Stretch = Stretch.Uniform };
    private readonly TextBlock _size = W.Hint("", 12.5);
    private readonly TextBlock _status = W.Hint("", 13);
    private readonly TextBox _name = new() { FontSize = 13 };
    private PosterOptions _options;

    /// <param name="model">The poster's keymap for a keyboard name (Studio's live keymap).</param>
    public PosterDialog(Func<string, PosterModel> model, string keyboard, bool dark)
    {
        _model = model;
        _options = new PosterOptions(PosterFormat.A3Landscape, dark, true);
        Title = "Export poster";
        Width = 980;
        Height = 640;
        MinWidth = 720;
        MinHeight = 520;
        FontFamily = BenchTheme.Ui;
        this.Tok(BackgroundProperty, "face");

        _name.Text = keyboard;
        _name.TextChanged += (_, _) => Update();
        var format = new Segmented(Formats.Select(PosterFormats.Name).ToArray());
        format.SelectionChanged += i => Update(_options with { Format = Formats[i] });
        var theme = new Segmented(["Dark", "Light"], dark ? 0 : 1);
        theme.SelectionChanged += i => Update(_options with { Dark = i == 0 });
        var combos = new Toggle("Include combos", true);
        combos.CheckedChanged += on => Update(_options with { Combos = on });
        var save = new BenchButton("Save…", ButtonKind.Primary);
        save.Click += async () => await SaveWithPicker();
        var close = new BenchButton("Close");
        close.Click += Close;

        var side = W.Column(12,
            W.Lbl("Format"), format, _size,
            Spaced(W.Lbl("Theme")), theme,
            Spaced(W.Lbl("Keyboard name")), _name,
            Spaced(combos),
            W.Hint("The keymap is Studio's current one, with edits you applied but haven't saved, and its combos."),
            Spaced(W.Row(8, save, close)),
            _status);
        var sidePanel = new Border { Width = 300, Padding = new Thickness(20), Child = side, BorderThickness = new Thickness(0, 0, 1, 0) }
            .Tok(Border.BorderBrushProperty, "ln");
        var previewWell = new Border { Padding = new Thickness(20), Child = _preview }.Tok(Border.BackgroundProperty, "face-lo");
        var grid = new Grid { ColumnDefinitions = new ColumnDefinitions("Auto,*") };
        Grid.SetColumn(previewWell, 1);
        grid.Children.Add(sidePanel);
        grid.Children.Add(previewWell);
        Content = grid;
        Update();
    }

    private static Control Spaced(Control c)
    {
        c.Margin = new Thickness(0, 6, 0, 0);
        return c;
    }

    internal PosterOptions Options => _options;

    internal PosterModel CurrentModel => _model(KeyboardName);

    internal string KeyboardName => string.IsNullOrWhiteSpace(_name.Text) ? "emk" : _name.Text.Trim();

    /// <summary>~/Pictures/emk-keymap-&lt;name&gt;-&lt;theme&gt;.png</summary>
    internal string SuggestedPath => PosterExport.DefaultPath(KeyboardName, _options.Dark);

    internal Bitmap? Preview => _preview.Source as Bitmap;

    /// <summary>Change the options and redraw the preview.</summary>
    internal void Update(PosterOptions? options = null)
    {
        if (options != null) _options = options;
        var px = PosterFormats.Pixels(_options.Format);
        _size.Text = $"{px.Width} × {px.Height} px{(_options.Format == PosterFormat.Wallpaper ? "" : " · 300 dpi")} · PDF: the same at paper size";
        (_preview.Source as IDisposable)?.Dispose();
        _preview.Source = PosterExport.Render(CurrentModel, _options with { Title = null }, scale: 0.25);
    }

    /// <summary>Write the poster to <paramref name="path"/> (.pdf: vector PDF, else PNG).</summary>
    internal void SaveTo(string path)
    {
        try
        {
            PosterExport.Save(CurrentModel, _options, path);
            _status.Text = $"Saved {path}";
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            _status.Text = $"Not saved: {e.Message}";
        }
    }

    private async Task SaveWithPicker()
    {
        var suggested = SuggestedPath;
        var folder = Path.GetDirectoryName(suggested)!;
        Directory.CreateDirectory(folder);
        var file = await StorageProvider.SaveFilePickerAsync(new FilePickerSaveOptions
        {
            Title = "Save the keymap poster",
            SuggestedFileName = Path.GetFileName(suggested),
            SuggestedStartLocation = await StorageProvider.TryGetFolderFromPathAsync(folder),
            DefaultExtension = "png",
            FileTypeChoices =
            [
                new FilePickerFileType("PNG image") { Patterns = ["*.png"] },
                new FilePickerFileType("PDF (vector)") { Patterns = ["*.pdf"] },
            ],
        });
        if (file?.TryGetLocalPath() is { } path) SaveTo(path);
    }
}

using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Documents;
using Avalonia.Layout;
using Avalonia.Media;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>Where Flash and Security read from; tests point it at their own folders.</summary>
public sealed class DeviceSources
{
    public string? Repo { get; init; } = RepoLocator.Find();
    public string KeyPath { get; init; } = SigningKeyStatus.DefaultPath;
    public string UsbRoot { get; init; } = "/sys/bus/usb/devices";
    public Func<string?> LogPort { get; init; } = HardwarePort.Find;

    /// <summary>The latest boot line a Debug session saw.</summary>
    public Func<BootFacts?> Boot { get; init; } = () => null;

    /// <summary>The keyboard <c>./hil</c> uses without KB_NAME.</summary>
    public static string DefaultKeyboard => Environment.GetEnvironmentVariable("KB_NAME") is { Length: > 0 } n ? n : "erik_kbd";

    public (IReadOnlyList<string> Names, Func<string, KeyboardFacts> Facts) Read()
    {
        var halves = HalfRecord.Load(Repo);
        var packages = FirmwarePackage.Scan(Repo == null ? null : Path.Combine(Repo, "target"));
        var usb = UsbBootloader.Scan(UsbRoot);
        var boot = Boot();
        var names = KeyboardFacts.Names(halves, packages);
        return (names, name => new KeyboardFacts
        {
            Name = name,
            Halves = halves.Where(h => h.Name == name).ToList(),
            Packages = packages.Where(p => p.Name == name).ToList(),
            OnUsb = usb,
            Boot = boot,
            AllHalves = halves,
        });
    }
}

/// <summary>A page with a keyboard selector strip that rebuilds its body from <see cref="DeviceSources"/>.</summary>
public abstract class DevicePage : DockPanel
{
    private readonly ContentControl _body = new();
    private readonly WrapPanel _stripLeft = new() { Orientation = Orientation.Horizontal, ItemSpacing = 22, LineSpacing = 6, VerticalAlignment = VerticalAlignment.Center };
    private readonly Grid _strip = new() { ColumnDefinitions = new ColumnDefinitions("*,Auto"), RowDefinitions = new RowDefinitions("Auto,Auto"), ColumnSpacing = 16, RowSpacing = 8 };
    private readonly StackPanel _stripRight;
    private readonly ContentControl _selectorHost = new() { VerticalAlignment = VerticalAlignment.Center };
    private string? _selected;
    private int _narrowColumns;

    protected DevicePage(DeviceSources sources)
    {
        Sources = sources;
        var refresh = new BenchButton("Refresh");
        refresh.Click += Refresh;
        _stripRight = W.Row(12, W.Lbl("Keyboard"), _selectorHost, refresh);
        Grid.SetColumn(_stripRight, 1);
        _strip.Children.Add(_stripLeft);
        _strip.Children.Add(_stripRight);
        var conn = Strips.Conn(_strip);
        SetDock(conn, Dock.Top);
        Children.Add(conn);
        Children.Add(_body);
    }

    /// <summary>The two halves, in the order the pages show them.</summary>
    protected static readonly string[] Sides = ["left", "right"];

    protected DeviceSources Sources { get; }

    /// <summary>The top bar's lamps for this page, after the last refresh.</summary>
    public IReadOnlyList<Lamp> TopLamps { get; private set; } = [];

    /// <summary>The page was read again (the top bar's lamps change).</summary>
    public event Action? Refreshed;

    /// <summary>The keyboard shown.</summary>
    public string? Selected => _selected;

    /// <summary>Read the files again and redraw.</summary>
    public void Refresh()
    {
        var (names, facts) = Sources.Read();
        if (_selected == null || !names.Contains(_selected)) _selected = Pick(names);
        var selector = new Segmented(names.Count == 0 ? ["none"] : names, Math.Max(0, IndexOf(names, _selected)));
        selector.SelectionChanged += index =>
        {
            _selected = names[index];
            Refresh();
        };
        _selectorHost.Content = selector;
        _stripLeft.Children.Clear();
        var key = SigningKeyStatus.Inspect(Sources.KeyPath);
        _stripLeft.Children.Add(new Lamp(key.Present ? $"Signing key: {key.Display}" : $"No signing key at {key.Display}",
            key.Present ? LampState.Good : LampState.Warn));
        if (Sources.Repo == null)
        {
            _stripLeft.Children.Add(new Lamp("Repository not found: set EMK_REPO", LampState.Crit));
            _body.Content = Message("The keyboard repository wasn't found next to the app. Start the app from the repo, or set EMK_REPO to its path.");
            TopLamps = [];
            Refreshed?.Invoke();
            return;
        }
        if (_selected == null)
        {
            _body.Content = Message($"No keyboards yet: {HalfRecord.FileName} and target/fw-nrf-*-ble-*.uf2|.zip are missing. ./hil flash records the halves; BLE=1 ./fw-nrf/build-uf2 <name> builds the packages.");
            return;
        }
        var f = facts(_selected);
        AddStripLamps(_stripLeft, f, key);
        _body.Content = Build(f, key);
        TopLamps = Lamps(f, key).Select(l => new Lamp(l.Text, l.State)).ToList();
        Refreshed?.Invoke();
    }

    /// <summary>The keyboard <c>./hil</c> uses by default when it is there, else the first one.</summary>
    private static string? Pick(IReadOnlyList<string> names)
    {
        if (names.Contains(DeviceSources.DefaultKeyboard)) return DeviceSources.DefaultKeyboard;
        return names.Count > 0 ? names[0] : null;
    }

    private static int IndexOf(IReadOnlyList<string> names, string? name)
    {
        for (var i = 0; i < names.Count; i++)
        {
            if (names[i] == name) return i;
        }
        return -1;
    }

    protected abstract IEnumerable<(string Text, LampState State)> Lamps(KeyboardFacts facts, SigningKeyStatus key);

    /// <summary>For tests: the keyboard picked in the selector.</summary>
    internal void Select(string name)
    {
        _selected = name;
        Refresh();
    }

    protected virtual void AddStripLamps(Panel strip, KeyboardFacts facts, SigningKeyStatus key)
    {
        // Pages without lamps of their own in the strip keep just the signing key's.
    }

    protected abstract Control Build(KeyboardFacts facts, SigningKeyStatus key);

    protected static Border Message(string text) => new Border
    {
        Padding = new Thickness(22, 18),
        Child = W.Text(text, 14),
    };

    /// <summary>Columns in a narrow window (0: the wide layout).</summary>
    public int NarrowColumns
    {
        get => _narrowColumns;
        set
        {
            if (_narrowColumns == value) return;
            _narrowColumns = value;
            // Narrow: the keyboard selector goes under the lamps.
            Grid.SetRow(_stripRight, value > 0 ? 1 : 0);
            Grid.SetColumn(_stripRight, value > 0 ? 0 : 1);
            _stripRight.HorizontalAlignment = value > 0 ? HorizontalAlignment.Left : HorizontalAlignment.Stretch;
            if (Selected != null || Sources.Repo == null) Refresh();
        }
    }

    /// <summary>Modules side by side; in a narrow window they wrap into <see cref="NarrowColumns"/> columns.</summary>
    protected Control Rack(string columns, params Control[] modules) => Rack(columns, _narrowColumns, modules);

    /// <summary>As <see cref="Rack(string, Control[])"/> with its own column count when narrow.</summary>
    protected Control Rack(string columns, int narrow, params Control[] modules)
    {
        if (_narrowColumns > 0)
        {
            var wrap = new Grid();
            for (var c = 0; c < narrow; c++) wrap.ColumnDefinitions.Add(new ColumnDefinition(GridLength.Star));
            for (var i = 0; i < modules.Length; i++)
            {
                if (i % narrow == 0) wrap.RowDefinitions.Add(new RowDefinition(GridLength.Auto));
                var lastInRow = i % narrow == narrow - 1 || i == modules.Length - 1;
                var module = new Border
                {
                    Padding = new Thickness(18, 14),
                    BorderThickness = new Thickness(0, 0, lastInRow ? 0 : 1, 1),
                    Child = modules[i],
                }.Tok(Border.BorderBrushProperty, "ln");
                Grid.SetRow(module, i / narrow);
                Grid.SetColumn(module, i % narrow);
                wrap.Children.Add(module);
            }
            return wrap;
        }
        var grid = new Grid { ColumnDefinitions = new ColumnDefinitions(columns) };
        for (var i = 0; i < modules.Length; i++)
        {
            var module = W.Module(modules[i], new Thickness(20, 16), last: i == modules.Length - 1);
            Grid.SetColumn(module, i);
            grid.Children.Add(module);
        }
        return grid;
    }

    /// <summary>The body, scrolling in a narrow window.</summary>
    protected Control Scrolling(Control body) =>
        _narrowColumns > 0 ? new ScrollViewer { Content = body, HorizontalScrollBarVisibility = Avalonia.Controls.Primitives.ScrollBarVisibility.Disabled } : body;

    protected static string Side(string side) => side == "left" ? "Left" : "Right";

    /// <summary>A text block with coloured runs.</summary>
    protected static TextBlock Runs(double size, params (string Text, string Token, bool Bold)[] runs)
    {
        var block = new TextBlock { FontFamily = BenchTheme.Ui, FontSize = size, TextWrapping = TextWrapping.Wrap }.Tok(TextBlock.ForegroundProperty, "ink");
        foreach (var (text, token, bold) in runs)
        {
            var run = new Run(text) { FontWeight = bold ? FontWeight.SemiBold : FontWeight.Normal };
            run.Tok(TextElement.ForegroundProperty, token);
            block.Inlines!.Add(run);
        }
        return block;
    }

    /// <summary>A copyable command (CSS well with a Copy button).</summary>
    internal static Border CommandRow(string command)
    {
        var copy = new BenchButton("Copy");
        copy.Click += () =>
        {
            _ = ClipboardText.CopyAsync(copy, command);
            copy.Text = "Copied";
        };
        var text = W.Mono(command, 12.5, "log");
        text.TextWrapping = TextWrapping.Wrap;
        var grid = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), ColumnSpacing = 10 };
        Grid.SetColumn(copy, 1);
        grid.Children.Add(text);
        grid.Children.Add(copy);
        return new Border { CornerRadius = new CornerRadius(3), BorderThickness = new Thickness(1), Padding = new Thickness(10, 6), Child = grid }
            .Tok(Border.BackgroundProperty, "well").Tok(Border.BorderBrushProperty, "well-edge");
    }

    protected static Border Alarm(string text) => new Border
    {
        BorderThickness = new Thickness(1.5),
        CornerRadius = new CornerRadius(3),
        Padding = new Thickness(12, 10),
        Child = W.Column(5,
            W.Head(text, 16).Tok(TextBlock.ForegroundProperty, "crit"),
            W.Text("Someone may have replaced its bootloader, or this is another nice!nano with the same serial. Don't flash it and don't pair it again. Unplug it and follow section 6.", 13),
            W.Mono(TamperRules.Doc + " · tamper alarm", 11.5, "mut")),
    }.Tok(Border.BorderBrushProperty, "crit").Tok(Border.BackgroundProperty, "crit-bg");
}

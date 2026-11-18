using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using KbEmulator.Bench;

namespace KbEmulator.DocsMedia;

/// <summary>
/// The keyboard as the app draws it, with the emulated firmware behind it on
/// a clock that only moves when the script says so. Frames are written every
/// 50 ms (the firmware draws its screens at 20 Hz).
/// </summary>
internal sealed class SceneRecorder : IDisposable
{
    public const int FrameMs = 50;
    public const double Scale = 1.25;

    private uint _now;
    private readonly EmulatedKeyboard _keyboard;
    private readonly CorneView _view;
    private readonly PointerPad? _pointer;
    private readonly Window _window;
    private readonly Control _root;
    private readonly string? _dir;
    private int _frame;

    /// <param name="framesRoot">Where the scene's frames go (a folder per scene); null: no frames (stills only).</param>
    /// <param name="pointer">The computer's screen with the mouse pointer beside the keyboard.</param>
    /// <param name="boot">Start as after switching the keyboard on (the start-up animation).</param>
    /// <param name="controls">The power switches, USB ports and reset buttons, as in the app.</param>
    /// <param name="title">A heading above the keyboard (and <paramref name="subtitle"/> under it).</param>
    public SceneRecorder(string? framesRoot, string name, bool pointer = false, bool boot = false, bool controls = false,
        string? title = null, string? subtitle = null)
    {
        if (framesRoot != null)
        {
            _dir = Path.Combine(framesRoot, name);
            if (Directory.Exists(_dir)) Directory.Delete(_dir, true);
            Directory.CreateDirectory(_dir);
            File.WriteAllText(Path.Combine(_dir, "frame-ms"), $"{FrameMs}\n");
        }

        _keyboard = new EmulatedKeyboard(() => _now);
        // Without the board controls the room above the halves (for the USB cable) is empty: cut most of it.
        _view = new CorneView(_keyboard.KeyCount)
        {
            Displays = _keyboard.Displays,
            ShowBoardControls = controls,
            Margin = new Thickness(0, controls ? 0 : -20, 0, 0),
        };
        _view.PowerToggled += (half, on) => _keyboard.SetHalfPower(half, on);
        _keyboard.LayerChanged += _ => RefreshLabels();
        RefreshLabels();

        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 16 };
        row.Children.Add(_view);
        if (pointer)
        {
            _pointer = new PointerPad { Width = 300, VerticalAlignment = VerticalAlignment.Stretch, Margin = new Thickness(0, 40, 0, 40) };
            row.Children.Add(_pointer);
        }
        Control content = row;
        if (title != null)
        {
            var heading = new StackPanel { Margin = new Thickness(14, 14, 14, 0), Spacing = 2 };
            heading.Children.Add(new TextBlock { Text = title, FontFamily = BenchTheme.Ui, FontSize = 22, FontWeight = Avalonia.Media.FontWeight.SemiBold }
                .Tok(TextBlock.ForegroundProperty, "ink"));
            if (subtitle != null)
            {
                heading.Children.Add(new TextBlock { Text = subtitle, FontFamily = BenchTheme.Ui, FontSize = 14 }
                    .Tok(TextBlock.ForegroundProperty, "mut"));
            }
            var stack = new StackPanel();
            stack.Children.Add(heading);
            stack.Children.Add(row);
            content = stack;
        }
        _root = new Border { Padding = new Thickness(10, 0, 10, 4), Child = content }.Tok(Border.BackgroundProperty, "face");
        _window = new Window { Content = _root, SizeToContent = SizeToContent.WidthAndHeight };
        _window.Show();
        if (boot) _keyboard.ReplayBoot();
        _keyboard.UpdateDisplays();
    }

    public EmulatedKeyboard Keyboard => _keyboard;

    private void RefreshLabels()
    {
        var labels = new string[_keyboard.KeyCount];
        for (var i = 0; i < labels.Length; i++) labels[i] = _keyboard.Label(i);
        _view.SetLabels(labels);
    }

    public void Press(int pos)
    {
        _view.SetPressed(pos, true);
        _keyboard.Press(pos);
    }

    public void Release(int pos)
    {
        _view.SetPressed(pos, false);
        _keyboard.Release(pos);
    }

    /// <summary>Press and let go 90 ms later.</summary>
    public void Tap(int pos, int holdMs = 90)
    {
        Press(pos);
        Run(holdMs);
        Release(pos);
    }

    /// <summary>Flip a half's power switch, as a click on it does.</summary>
    public void FlipPower(int half) => _view.FlipPower(half);

    /// <summary>Type letters and spaces, a key every <paramref name="everyMs"/>.</summary>
    public void Type(string text, int everyMs)
    {
        foreach (var c in text)
        {
            Tap(KeyOf(c));
            Run(everyMs - 90);
        }
    }

    private int KeyOf(char c)
    {
        if (c == ' ') return 38;
        for (var pos = 0; pos < _keyboard.KeyCount; pos++)
        {
            if (_keyboard.Label(pos) == char.ToUpperInvariant(c).ToString()) return pos;
        }
        throw new ArgumentException($"no key for {c}");
    }

    public void Run(int ms)
    {
        for (var i = 0; i < ms; i++)
        {
            _now++;
            _keyboard.Tick();
            if (_now % FrameMs == 0) Frame();
        }
    }

    private void Frame()
    {
        _keyboard.UpdateDisplays();
        _view.InvalidateVisual();
        if (_pointer != null) _pointer.State = _keyboard.Mouse;
        if (_dir != null) Program.Render(_root, Path.Combine(_dir, $"{_frame++:D4}.png"), Scale);
    }

    /// <summary>The keyboard as it is now, as one picture.</summary>
    public void Still(string path, double scale = Scale)
    {
        _keyboard.UpdateDisplays();
        _view.InvalidateVisual();
        Program.Render(_root, path, scale);
        Console.WriteLine($"{new FileInfo(path).Length,9}  {Path.GetRelativePath(Environment.CurrentDirectory, path)}");
    }

    /// <summary>Show the last frame this much longer before the GIF loops.</summary>
    public void Hold(int ms)
    {
        if (_dir != null) File.WriteAllText(Path.Combine(_dir, "hold-ms"), $"{ms}\n");
    }

    public void Dispose()
    {
        _window.Close();
        _keyboard.Dispose();
    }
}

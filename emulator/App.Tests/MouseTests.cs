using Avalonia.Headless;
using Kb.HostProtocol;

namespace KbEmulator.Tests;

/// <summary>Mouse actions in Studio's encoding and legends.</summary>
public class MouseEncodingTests
{
    [Test]
    [Arguments("mm:left")]
    [Arguments("mm:right")]
    [Arguments("mb:middle")]
    [Arguments("ms:down")]
    [Arguments("ms:left")]
    [Arguments("mp")]
    [Arguments("mk:left/BSPC")]
    [Arguments("mk:scroll_left/F2")]
    [Arguments("mk:right/LC(SPACE)")]
    public async Task Mouse_actions_round_trip_through_the_editor(string canonical)
    {
        // Arrange
        var model = StudioAction.Parse(canonical);

        // Act
        var encoded = model.Encode();

        // Assert
        await Assert.That(StudioCodec.Canonical(encoded)).IsEqualTo(canonical);
    }

    [Test]
    [Arguments("mm:left", "◀ mouse")]
    [Arguments("mb:left", "L click")]
    [Arguments("ms:down", "scroll ↓")]
    [Arguments("mp", "Precise")]
    [Arguments("mk:left/BSPC", "Bksp\nL click")]
    [Arguments("mk:scroll_left/F2", "F2\nscroll ←")]
    public async Task Mouse_actions_have_the_firmwares_legends(string encoding, string legend)
    {
        // Arrange
        var e = encoding;

        // Act
        var label = StudioCodec.Label(e);

        // Assert
        await Assert.That(label).IsEqualTo(legend);
    }

    [Test]
    public async Task Mouse_or_key_keeps_the_mouse_action_and_the_key()
    {
        // Arrange
        const string encoding = "mk:scroll_up/F4";

        // Act
        var model = StudioAction.Parse(encoding);

        // Assert
        await Assert.That((model.Kind, model.MouseAction, model.Key)).IsEqualTo((StudioKind.MouseOrKey, "scroll_up", "F4"));
    }

    [Test]
    public async Task The_mouse_capability_is_read_from_the_version_line()
    {
        // Arrange
        const string line = "#V 2 layers=4 keys=42 store=flash max=40 combos=16 caps=check,combos,defaults,mouse names=default,symbols,nav,wm";

        // Act
        var v = (StudioVersion)StudioLine.Parse(line)!;

        // Assert
        await Assert.That(v.Has("mouse")).IsTrue();
    }
}

/// <summary>The mouse keys in the emulator, on a hand-moved clock.</summary>
public class EmulatorMouseTests
{
    private const int Nav = 41, S = 14, D = 15, G = 17, E = 3, LeftThumb = 37;

    private sealed class Clock
    {
        public uint Now;
    }

    private static (EmulatedKeyboard Keyboard, Clock Clock, List<MouseReport> Reports) New()
    {
        var clock = new Clock();
        var keyboard = new EmulatedKeyboard(() => clock.Now);
        var reports = new List<MouseReport>();
        keyboard.MouseReported += (r, _) => reports.Add(r);
        return (keyboard, clock, reports);
    }

    /// <summary>Move time on in 5 ms timer ticks, as the window does.</summary>
    private static List<int> Run(EmulatedKeyboard k, Clock c, uint ms, Func<MouseState, int> sample)
    {
        var samples = new List<int>();
        for (var t = 0u; t < ms; t += 5)
        {
            c.Now += 5;
            k.Tick();
            samples.Add(sample(k.Mouse));
        }
        return samples;
    }

    [Test]
    public async Task Holding_nav_and_s_moves_the_pointer_left_smoothly()
    {
        // Arrange
        var (k, c, reports) = New();
        using var _ = k;
        k.Press(Nav);
        k.Press(S);

        // Act
        var xs = Run(k, c, 400, m => m.X);

        // Assert: always further left, in many small reports
        var steady = xs.Zip(xs.Skip(1)).All(p => p.Second <= p.First);
        await Assert.That((xs[^1] < 0, steady, reports.Count >= 30)).IsEqualTo((true, true, true));
    }

    [Test]
    public async Task Down_and_right_together_move_diagonally()
    {
        // Arrange
        var (k, c, _) = New();
        using var __ = k;
        k.Press(Nav);
        k.Press(D);
        k.Press(G);

        // Act
        Run(k, c, 300, m => m.X);

        // Assert
        var m = k.Mouse;
        await Assert.That((m.X > 0, m.Y > 0)).IsEqualTo((true, true));
    }

    [Test]
    public async Task The_left_thumb_clicks_once_the_pointer_is_on()
    {
        // Arrange
        var (k, c, reports) = New();
        using var _ = k;
        k.Press(Nav);
        k.Press(S);
        Run(k, c, 50, m => m.X);
        k.Release(S);

        // Act
        k.Press(LeftThumb);

        // Assert
        await Assert.That((k.Mouse.PointerActive, reports.Any(r => (r.Buttons & 1) != 0))).IsEqualTo((true, true));
    }

    [Test]
    public async Task E_scrolls_down_once_the_pointer_is_on()
    {
        // Arrange
        var (k, c, _) = New();
        using var __ = k;
        k.Press(Nav);
        k.Press(S);
        Run(k, c, 50, m => m.X);
        k.Release(S);
        k.Press(E);

        // Act
        Run(k, c, 400, m => m.Wheel);

        // Assert
        await Assert.That(k.Mouse.Wheel).IsLessThan(0);
    }

    [Test]
    public async Task Smooth_scrolling_counts_sixteenths()
    {
        // Arrange
        var (k, c, _) = New();
        using var __ = k;
        k.SetSmoothScrolling(true, true);
        k.Press(Nav);
        k.Press(S);
        Run(k, c, 50, m => m.X);
        k.Release(S);
        k.Press(E);

        // Act
        Run(k, c, 400, m => m.Wheel);

        // Assert
        await Assert.That((k.Mouse.Hires, k.Mouse.Wheel < 0)).IsEqualTo(((byte)3, true));
    }

    [Test]
    public async Task Without_the_pointer_the_left_thumb_is_backspace()
    {
        // Arrange
        var (k, _, reports) = New();
        using var __ = k;
        k.Press(Nav);

        // Act
        k.Press(LeftThumb);

        // Assert
        await Assert.That((reports.Count, k.LastReport.Contains(0x2A))).IsEqualTo((0, true));
    }
}

/// <summary>Mouse reports turned into a few Events lines.</summary>
public class MouseLogTests
{
    [Test]
    public async Task Motion_is_one_line_per_gesture()
    {
        // Arrange
        var log = new MouseLog();
        for (uint t = 0; t < 30; t++) log.Add(new MouseReport(0, 1, -1, 0, 0), t);

        // Act
        var lines = log.Flush(29 + MouseLog.Idle);

        // Assert
        await Assert.That(lines).IsEquivalentTo(new[] { "mouse moved (+30, −30)" });
    }

    [Test]
    public async Task Nothing_is_written_while_the_gesture_goes_on()
    {
        // Arrange
        var log = new MouseLog();
        log.Add(new MouseReport(0, 5, 0, 0, 0), 0);

        // Act
        var lines = log.Flush(100);

        // Assert
        await Assert.That(lines).IsEmpty();
    }

    [Test]
    public async Task A_press_is_a_click()
    {
        // Arrange
        var log = new MouseLog();

        // Act
        var lines = log.Add(new MouseReport(1, 0, 0, 0, 0), 0);

        // Assert
        await Assert.That(lines).IsEquivalentTo(new[] { "L click" });
    }

    [Test]
    public async Task Smooth_scrolling_is_counted_in_detents()
    {
        // Arrange
        var log = new MouseLog();
        for (uint t = 0; t < 48; t++) log.Add(new MouseReport(0, 0, 0, -1, 0), t, wheelHires: true);

        // Act
        var lines = log.Flush(47 + MouseLog.Idle);

        // Assert: 48 sixteenths down = 3 detents
        await Assert.That(lines).IsEquivalentTo(new[] { "scroll ↓ 3" });
    }

    [Test]
    public async Task Pan_right_is_an_arrow_right()
    {
        // Arrange
        var log = new MouseLog();
        log.Add(new MouseReport(0, 0, 0, 0, 2), 0);

        // Act
        var lines = log.Flush(MouseLog.Idle);

        // Assert
        await Assert.That(lines).IsEquivalentTo(new[] { "scroll → 2" });
    }
}

/// <summary>The pointer in the Emulator window and in Debug.</summary>
[NotInParallel("ui-session")]
public class MousePointerUiTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    [Test]
    public async Task Moving_right_is_one_events_line_and_moves_the_preview()
    {
        // Arrange
        var (events, preview, mouse) = await TestWindow.Run(w =>
        {
            w.K.Press(41);
            w.K.Press(17);

            // Act
            w.Advance(300);
            w.K.Release(17);
            w.Advance(600);
            return (w.Window.EventText, w.Window.Pointer.State.X, w.K.Mouse.X);
        });

        // Assert
        var moved = events.Split('\n').Count(l => l.StartsWith("mouse moved (+", StringComparison.Ordinal));
        await Assert.That((moved, preview == mouse, mouse > 0)).IsEqualTo((1, true, true));
    }

    [Test]
    public async Task Reset_puts_the_pointer_back_in_the_middle()
    {
        // Arrange
        var x = await TestWindow.Run(w =>
        {
            w.K.Press(41);
            w.K.Press(17);
            w.Advance(200);
            w.K.Release(17);

            // Act
            w.C.ResetPointer.PerformClick();
            return w.Window.Pointer.State.X;
        });

        // Assert
        await Assert.That(x).IsEqualTo(0);
    }

    [Test]
    public async Task The_smooth_scrolling_toggle_tells_the_emulator()
    {
        // Arrange
        var hires = await TestWindow.Run(w =>
        {
            // Act
            w.C.SmoothScrolling.IsChecked = true;
            return w.K.Mouse.Hires;
        });

        // Assert
        await Assert.That(hires).IsEqualTo((byte)3);
    }

    [Test]
    public async Task A_click_is_logged_in_events()
    {
        // Arrange
        var events = await TestWindow.Run(w =>
        {
            w.K.Press(41);
            w.K.Press(14);
            w.Advance(50);
            w.K.Release(14);

            // Act
            w.K.Press(37);
            w.Advance(20);
            return w.Window.EventText;
        });

        // Assert
        await Assert.That(events).Contains("L click");
    }

    [Test]
    public async Task The_mouse_chip_shows_motion()
    {
        // Arrange
        var chip = await TestWindow.Run(w =>
        {
            w.K.Press(41);
            w.K.Press(14);

            // Act
            w.Advance(100);
            return w.Window.Chips.Mouse;
        });

        // Assert
        await Assert.That(chip).IsEqualTo("moving");
    }

    [Test]
    public async Task Debug_shows_the_last_mouse_report_and_the_pointer()
    {
        // Arrange
        const string stream =
            "#S layer=2 caps=0 mods=00 out=ble profile=1 host=1 open=0 linked=1 stage=- bat=87,90 chg=1,0 wpm=0 power=active presses=0 mute=0 trace=0 lit=1 radio=4 pointer=1\r\n" +
            "#M 01d4feb004ff01\r\n";
        var (pointer, mouse, pane) = await Session.Dispatch(() =>
        {
            var view = new DebugView(_ => "base", 42, new HardwarePort(() => null));

            // Act
            view.Feed(stream);
            var r = (view.Readout("Pointer"), view.Readout("Mouse"), view.MouseText);
            view.Dispose();
            return r;
        }, CancellationToken.None);

        // Assert
        await Assert.That((pointer, mouse, pane)).IsEqualTo(("on", "L · (−300, +1200) · wheel −1 · pan +1", "L click"));
    }

    [Test]
    [Arguments("light")]
    [Arguments("dark")]
    public async Task Pointer_preview_renders(string theme)
    {
        // Arrange
        var path = Path.Combine(AppContext.BaseDirectory, $"main-window-pointer-{theme}.png");

        // Act
        var x = await TestWindow.Run(w =>
        {
            Avalonia.Application.Current!.RequestedThemeVariant = theme == "light" ? Avalonia.Styling.ThemeVariant.Light : Avalonia.Styling.ThemeVariant.Dark;
            w.K.Press(41);
            w.K.Press(17);
            w.K.Press(15);
            w.Advance(250);
            w.K.Release(15);
            w.K.Release(17);
            w.K.Press(37);
            w.Advance(30);
            Avalonia.Threading.Dispatcher.UIThread.RunJobs();
            var size = new Avalonia.PixelSize((int)w.Window.Bounds.Width, (int)w.Window.Bounds.Height);
            using var bitmap = new Avalonia.Media.Imaging.RenderTargetBitmap(size);
            bitmap.Render(w.Window);
            using (var file = File.Create(path)) bitmap.Save(file, Avalonia.Media.Imaging.PngBitmapEncoderOptions.Default);
            Avalonia.Application.Current.RequestedThemeVariant = Avalonia.Styling.ThemeVariant.Default;
            return w.Window.Pointer.State.X;
        });

        // Assert
        await Assert.That(x).IsGreaterThan(0);
    }
}

public class DebugMouseParsingTests
{
    [Test]
    public async Task Mouse_line_gives_buttons_motion_wheel_and_pan()
    {
        // Arrange
        const string line = "#M 01d4feb004ff01";

        // Act
        var e = (DebugMouse)DebugProtocol.Parse(line);

        // Assert
        await Assert.That(e.Report).IsEqualTo(new MouseReport(1, -300, 1200, -1, 1));
    }

    [Test]
    public async Task A_short_mouse_line_is_a_log_line()
    {
        // Arrange
        const string line = "#M 01d4";

        // Act
        var e = DebugProtocol.Parse(line);

        // Assert
        await Assert.That(e).IsTypeOf<DebugLog>();
    }

    [Test]
    [Arguments("radio=4 pointer=1", true)]
    [Arguments("radio=4 pointer=0", false)]
    [Arguments("radio=4", null)]
    public async Task Pointer_comes_from_the_status(string tail, bool? expected)
    {
        // Arrange
        var line = "#S layer=0 caps=0 mods=00 out=usb profile=1 host=0 open=0 linked=1 stage=- bat=80,70 chg=0,0 wpm=0 power=active presses=0 mute=0 trace=0 lit=1 " + tail;

        // Act
        var s = (DebugStatus)DebugProtocol.Parse(line);

        // Assert
        await Assert.That(s.PointerActive).IsEqualTo(expected);
    }
}

/// <summary>Studio's editor for each mouse behavior, on the emulator (which has the mouse capability).</summary>
[NotInParallel("ui-session")]
public class MouseEditorTests
{
    [Test]
    [Arguments(StudioKind.MouseMove, "up", "mm:up")]
    [Arguments(StudioKind.MouseButton, "right", "mb:right")]
    [Arguments(StudioKind.Scroll, "right", "ms:right")]
    [Arguments(StudioKind.Precision, "", "mp")]
    [Arguments(StudioKind.MouseOrKey, "scroll_up", "mk:scroll_up/F4")]
    public async Task Each_mouse_behavior_can_be_applied(StudioKind kind, string choice, string expected)
    {
        // Arrange
        var binding = await TestAppBuilder.Session.Dispatch(() =>
        {
            using var keyboard = new EmulatedKeyboard(() => 0);
            var s = new StudioView(keyboard);
            s.Activate();
            s.Pump();
            s.Pump();
            s.UnlockButton.PerformClick();
            s.Pump();
            s.Select(0, 13);
            s.EditForTest(e =>
            {
                e.Kind = kind;
                e.Direction = choice;
                e.Button = choice;
                e.MouseAction = choice;
                e.Key = "F4";
                e.Mods.Clear();
            });

            // Act
            s.Apply();
            s.Pump();
            return s.Client.Binding(0, 13);
        }, CancellationToken.None);

        // Assert
        await Assert.That(binding).IsEqualTo(expected);
    }

    [Test]
    public async Task Firmware_without_mouse_keys_hides_the_mouse_behaviors()
    {
        // Arrange
        var (supported, text) = await TestAppBuilder.Session.Dispatch(() =>
        {
            using var keyboard = new EmulatedKeyboard(() => 0);
            var debug = new DebugView(_ => "base", 42, new HardwarePort(() => null));
            var studio = new StudioView(keyboard, debug);
            studio.SelectTarget(1);

            // Act
            debug.Feed("#V 2 layers=4 keys=42 store=flash max=40 combos=16 caps=check,combos,defaults names=default,symbols,nav,wm\r\n");
            studio.Pump();
            var r = (studio.MouseSupported, studio.KeyText);
            debug.Dispose();
            return r;
        }, CancellationToken.None);

        // Assert
        await Assert.That((supported, text.Contains("Mouse move"))).IsEqualTo((false, false));
    }

    [Test]
    public async Task The_emulator_offers_the_mouse_behaviors()
    {
        // Arrange
        var text = await TestAppBuilder.Session.Dispatch(() =>
        {
            using var keyboard = new EmulatedKeyboard(() => 0);
            var s = new StudioView(keyboard);

            // Act
            s.Activate();
            s.Pump();
            s.Select(0, 13);
            return s.KeyText;
        }, CancellationToken.None);

        // Assert
        await Assert.That(text).Contains("Mouse move");
    }
}

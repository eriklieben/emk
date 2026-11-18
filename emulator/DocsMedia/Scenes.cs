namespace KbEmulator.DocsMedia;

/// <summary>
/// The keyboard scenes: the OLED animations of tools/docs-media (kb-*, the
/// same scripts) and a few more (scene-*), on the whole keyboard.
/// </summary>
internal static class Scenes
{
    private const int Sym = 36, Bksp = 37, Space = 38, Enter = 39, Nav = 41;
    private const int LCtrl = 24, RShift = 35, T = 5, Y = 6, Radio = 29;

    public static readonly Action<string>[] All =
        [Startup, WaitingForHalf, TypingRain, LayersAndMods, Pairing, RadioPower, Message, Sleep, NavMouse];

    /// <summary>A few frames, to check the whole path works (./docs-media --smoke).</summary>
    public static void Smoke(string frames)
    {
        using var s = new SceneRecorder(frames, "smoke");
        s.Tap(18);
        s.Run(200);
    }

    /// <summary>Switched on: rain, E and L, sparks, the status screens.</summary>
    private static void Startup(string frames)
    {
        using var s = new SceneRecorder(frames, "kb-startup", boot: true);
        s.Run(4000);
        s.Hold(1500);
    }

    /// <summary>The left half alone asks for the right one; its switch goes on, they link and start together.</summary>
    private static void WaitingForHalf(string frames)
    {
        using var s = new SceneRecorder(frames, "kb-waiting-for-half", controls: true);
        s.FlipPower(1);
        s.Keyboard.ReplayBoot();
        s.Run(4200);
        s.FlipPower(1);
        s.Run(6000);
        s.Hold(1200);
    }

    /// <summary>Typing: keys light up, every press drops rain on both screens.</summary>
    private static void TypingRain(string frames)
    {
        using var s = new SceneRecorder(frames, "kb-typing-rain");
        s.Run(400);
        s.Type("hello emk", 190);
        s.Run(500);
        s.Type("rain", 140);
        s.Run(1600);
        s.Hold(300);
    }

    /// <summary>SYM, NAV with the pointer moving (MOUSE), caps word, Ctrl and Shift.</summary>
    private static void LayersAndMods(string frames)
    {
        using var s = new SceneRecorder(frames, "kb-layers-and-mods");
        s.Run(600);
        s.Press(Sym);
        s.Run(1000);
        s.Release(Sym);
        s.Run(500);
        s.Press(Nav);
        s.Run(800);
        s.Press(14);
        s.Run(500);
        s.Release(14);
        s.Run(200);
        s.Press(16);
        s.Run(300);
        s.Release(16);
        s.Run(700);
        s.Release(Nav);
        s.Run(600);
        s.Press(T);
        s.Run(10);
        s.Press(Y);
        s.Run(80);
        s.Release(T);
        s.Release(Y);
        s.Run(1300);
        s.Type("emk", 200);
        s.Run(1000);
        s.Tap(Space);
        s.Run(600);
        s.Press(LCtrl);
        s.Run(500);
        s.Press(RShift);
        s.Run(1000);
        s.Release(RShift);
        s.Run(300);
        s.Release(LCtrl);
        s.Run(700);
        s.Hold(600);
    }

    /// <summary>The computer shows 418306: typed on the symbols layer, a 7 taken back, Enter.</summary>
    private static void Pairing(string frames)
    {
        using var s = new SceneRecorder(frames, "kb-pairing");
        s.Keyboard.Profile = 1;
        s.Keyboard.SetProfileOpen(true);
        s.Run(1000);
        s.Keyboard.SimulatePairing(418306);
        s.Run(900);
        s.Press(Sym);
        s.Run(150);
        foreach (var d in new[] { 4, 1, 8, 7 })
        {
            s.Tap(Digit(d));
            s.Run(330);
        }
        s.Run(200);
        s.Tap(Bksp);
        s.Run(450);
        foreach (var d in new[] { 3, 0, 6 })
        {
            s.Tap(Digit(d));
            s.Run(330);
        }
        s.Release(Sym);
        s.Run(500);
        s.Tap(Enter);
        s.Run(2400);
        s.Hold(1500);
    }

    /// <summary>nav + B three times: +8, +4, 0 dBm.</summary>
    private static void RadioPower(string frames)
    {
        using var s = new SceneRecorder(frames, "kb-radio");
        s.Run(500);
        s.Press(Nav);
        s.Run(300);
        for (var i = 0; i < 3; i++)
        {
            s.Tap(Radio, 150);
            s.Run(1000);
        }
        s.Run(500);
        s.Release(Nav);
        s.Run(1800);
        s.Hold(400);
    }

    /// <summary>A message from the computer on the right screen, then cleared.</summary>
    private static void Message(string frames)
    {
        using var s = new SceneRecorder(frames, "kb-message");
        s.Run(400);
        s.Type("ok", 160);
        s.Run(300);
        foreach (var packet in Kb.HostProtocol.NotificationPackets.Encode("Build passed on main")) s.Keyboard.RawHid(packet);
        s.Run(4200);
        s.Keyboard.RawHid(Kb.HostProtocol.NotificationPackets.Clear());
        s.Run(1500);
        s.Hold(300);
    }

    /// <summary>Idle: the screens melt down, a cursor blinks, then they go dark.</summary>
    private static void Sleep(string frames)
    {
        using var s = new SceneRecorder(frames, "kb-sleep");
        s.Keyboard.SetPowerTimeouts(TimeSpan.FromMilliseconds(1200), null);
        s.Run(5400);
        s.Hold(800);
    }

    /// <summary>
    /// Hold nav: the legends switch. G and F move the pointer (mouse mode:
    /// the MOUSE bar), then the Backspace thumb clicks.
    /// </summary>
    private static void NavMouse(string frames)
    {
        using var s = new SceneRecorder(frames, "scene-nav-mouse", pointer: true);
        s.Run(400);
        s.Press(Nav);
        s.Run(700);
        s.Press(17);
        s.Run(800);
        s.Release(17);
        s.Run(150);
        s.Press(16);
        s.Run(600);
        s.Release(16);
        s.Run(300);
        s.Tap(Bksp, 250);
        s.Run(700);
        s.Release(Nav);
        s.Run(900);
        s.Hold(400);
    }

    /// <summary>Each layer held, as one picture: legends and the layer name on the left screen.</summary>
    public static void Layers(string media)
    {
        Layer(media, "default", []);
        Layer(media, "symbols", [Sym]);
        Layer(media, "nav", [Nav]);
        Layer(media, "wm", [Sym, Nav]);
        // Mouse mode: G moved the pointer (the MOUSE bar), the Backspace thumb held is a left click.
        Layer(media, "nav-mouse", [Nav], s =>
        {
            s.Tap(17, 200);
            s.Run(200);
            s.Press(Bksp);
        });
    }

    private static void Layer(string media, string name, int[] hold, Action<SceneRecorder>? then = null)
    {
        using var s = new SceneRecorder(null, name);
        s.Run(300);
        foreach (var key in hold)
        {
            s.Press(key);
            s.Run(150);
        }
        then?.Invoke(s);
        s.Run(700);
        s.Still(Path.Combine(media, $"layer-{name}.png"));
    }

    /// <summary>
    /// One picture per combo (core/src/keymaps/corne.rs): both keys pressed
    /// together on the combo's layer, its result on the screens where it shows.
    /// </summary>
    public static void Combos(string media)
    {
        const string typing = "within 50 ms, not right after another key (150 ms idle)";
        Combo(media, "cut", "Z + X → Cut (Ctrl+X)", $"default layer · {typing}", 25, 26);
        Combo(media, "copy", "X + C → Copy (Ctrl+C)", $"default layer · {typing}", 26, 27);
        Combo(media, "paste", "C + V → Paste (Ctrl+V)", $"default layer · {typing}", 27, 28);
        Combo(media, "caps-word", "T + Y → Caps word", $"default layer · {typing}", 5, 6);
        Combo(media, "escape", "J + K → Esc", $"default layer · {typing}", 19, 20);
        Combo(media, "bt-clear", "Profile ◀ + Profile ▶ → forget the profile's computer", "nav layer (hold the right outer thumb) · within 50 ms", 25, 26, Nav);
    }

    private static void Combo(string media, string name, string title, string subtitle, int a, int b, int? layer = null)
    {
        using var s = new SceneRecorder(null, name, title: title, subtitle: subtitle);
        s.Run(400);
        if (layer is { } l)
        {
            s.Press(l);
            s.Run(300);
        }
        s.Press(a);
        s.Run(15);
        s.Press(b);
        s.Run(900);
        s.Still(Path.Combine(media, $"combo-{name}.png"), 1);
    }

    /// <summary>Digit <paramref name="d"/> on the symbols layer: 1-5 on A..G, 6-0 on Z..B.</summary>
    private static int Digit(int d) => d switch
    {
        >= 1 and <= 5 => 12 + d,
        >= 6 and <= 9 => 19 + d,
        _ => 29,
    };
}

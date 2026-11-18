using System.Globalization;

namespace KbEmulator;

/// <summary>A Studio line from the keyboard (docs/studio-protocol.md, "Lines").</summary>
public abstract record StudioLine
{
    private static readonly string[] Tags = ["V", "L", "A", "U", "D", "C", "LD", "CD"];

    /// <summary>Parse one line (without \r\n); null when it isn't a Studio line (unknown tags are skipped).</summary>
    public static StudioLine? Parse(string line)
    {
        line = line.TrimEnd('\r', '\n');
        if (line.Length < 3 || line[0] != '#') return null;
        var space = line.IndexOf(' ');
        if (space < 2) return null;
        var tag = line[1..space];
        var parts = line[(space + 1)..].Split(' ', StringSplitOptions.RemoveEmptyEntries);
        try
        {
            return tag switch
            {
                "V" => Version(parts),
                "L" or "LD" when parts.Length >= 1 => new StudioLayer(Int(parts[0]), parts.Skip(1).ToArray(), Default: tag == "LD"),
                "C" or "CD" when parts.Length >= 2 && parts[0] == "end" => new StudioComboEnd(Int(parts[1]), Default: tag == "CD"),
                "C" or "CD" when parts.Length >= 2 => new StudioComboLine(Int(parts[0]), string.Join(' ', parts.Skip(1)), Default: tag == "CD"),
                "A" => Ack(parts),
                "U" when parts.Length > 0 => Unlock(parts),
                "D" when parts.Length > 0 => new StudioDirty(parts[0] == "1"),
                _ => null,
            };
        }
        catch (FormatException)
        {
            return null;
        }
    }

    /// <summary><c>#U unlocked &lt;seconds&gt;</c> or <c>#U locked</c>.</summary>
    private static StudioUnlock Unlock(string[] parts)
    {
        if (parts[0] != "unlocked") return new StudioUnlock(false, 0);
        var seconds = parts.Length > 1 && int.TryParse(parts[1], CultureInfo.InvariantCulture, out var s) ? s : 0;
        return new StudioUnlock(true, seconds);
    }

    private static int Int(string s) => int.Parse(s, NumberStyles.Integer, CultureInfo.InvariantCulture);

    private static StudioVersion? Version(string[] parts)
    {
        if (parts.Length == 0 || !int.TryParse(parts[0], CultureInfo.InvariantCulture, out var version)) return null;
        var fields = parts.Skip(1).Select(p => p.Split('=', 2)).Where(p => p.Length == 2).ToDictionary(p => p[0], p => p[1]);
        int Field(string name, int fallback) => fields.TryGetValue(name, out var v) && int.TryParse(v, CultureInfo.InvariantCulture, out var n) ? n : fallback;
        var caps = version >= 2 && fields.TryGetValue("caps", out var c) ? c.Split(',', StringSplitOptions.RemoveEmptyEntries) : [];
        return new StudioVersion(version, Field("layers", 4), Field("keys", 42), fields.GetValueOrDefault("store", "?"), Field("max", 40),
            fields.TryGetValue("names", out var names) ? names.Split(',') : [], Field("combos", 0), caps);
    }

    /// <summary>
    /// set|check &lt;layer&gt; &lt;pos&gt; ok|err &lt;x&gt; · combo add|set|remove|check &lt;i|new&gt; ok|err &lt;x&gt;
    /// · save|revert|reset|lock ok|err &lt;why&gt; · ? err syntax.
    /// </summary>
    private static StudioAck? Ack(string[] parts)
    {
        if (parts.Length < 2) return null;
        var command = parts[0];
        int? layer = null, pos = null, combo = null;
        var i = 1;
        if (command is "set" or "check" && parts.Length >= 4 && parts[3] is "ok" or "err")
        {
            layer = Int(parts[1]);
            pos = Int(parts[2]);
            i = 3;
        }
        else if (command == "combo" && parts.Length >= 4)
        {
            command = "combo " + parts[1];
            combo = parts[2] == "new" ? null : Int(parts[2]);
            i = 3;
        }
        if (parts[i] is not ("ok" or "err")) return null;
        return new StudioAck(command, layer, pos, parts[i] == "ok", string.Join(' ', parts.Skip(i + 1))) { Combo = combo };
    }

    /// <summary>Whether a raw port line is for Studio.</summary>
    public static bool IsStudio(string line)
    {
        if (line.Length < 3 || line[0] != '#') return false;
        var space = line.IndexOf(' ');
        return space > 1 && Tags.Contains(line[1..space]);
    }
}

public sealed record StudioVersion(int Version, int Layers, int Keys, string Store, int Max, IReadOnlyList<string> Names,
    int MaxCombos = 0, IReadOnlyList<string>? Caps = null) : StudioLine
{
    /// <summary>A capability of version 2 (check, combos, defaults); version 1 has none.</summary>
    public bool Has(string capability) => Caps?.Contains(capability) == true;
}

/// <summary><c>#L</c> (live) or <c>#LD</c> (compiled) bindings of a layer.</summary>
public sealed record StudioLayer(int Layer, IReadOnlyList<string> Actions, bool Default = false) : StudioLine;

/// <summary><c>#C &lt;i&gt; &lt;combo&gt;</c> or <c>#CD …</c>.</summary>
public sealed record StudioComboLine(int Index, string Combo, bool Default = false) : StudioLine;

/// <summary><c>#C end &lt;n&gt;</c>: the list is complete.</summary>
public sealed record StudioComboEnd(int Count, bool Default = false) : StudioLine;

/// <summary>An acknowledgement: <c>#A set 0 12 ok mt:LCTRL/TAB</c>, <c>#A save err locked</c>.</summary>
public sealed record StudioAck(string Command, int? Layer, int? Pos, bool Ok, string Detail) : StudioLine
{
    /// <summary>The combo index of a <c>combo …</c> ack (null: <c>new</c>, or not a combo).</summary>
    public int? Combo { get; init; }

    /// <summary>What an error means, for the user (the reason as the keyboard sent it, explained).</summary>
    public string Explain() => Ok ? Detail : Detail switch
    {
        "locked" => "err locked: press StudioUnlock first (hold nav, press the Studio key)",
        "range" => "err range: no such layer or position",
        "action" => "err action: not an action, or mo: to a layer that doesn't exist",
        "long" => "err long: the encoding is over the length limit",
        "lockout" => "err lockout: it would leave no StudioUnlock key you can reach",
        "complex" => "err complex: the keymap is too tangled to check, so it was refused",
        "syntax" => "err syntax: the keyboard didn't understand it (older firmware, or a combo that isn't six words)",
        "positions" => "err positions: a combo needs 2 to 4 different keys",
        "name" => "err name: up to 16 of A-Z a-z 0-9 _ (or none)",
        "timing" => "err timing: timeout 1..65535 ms, idle 0..65535 ms",
        "full" => "err full: there are already as many combos as the keyboard holds",
        "storage" => "err storage: the file couldn't be written; still unsaved",
        _ => "err " + Detail,
    };
}

public sealed record StudioUnlock(bool Unlocked, int Seconds) : StudioLine;
public sealed record StudioDirty(bool Dirty) : StudioLine;

/// <summary>Where Studio lines go and come from: the emulator, or the keyboard's log port.</summary>
public interface IStudioTransport
{
    /// <summary>Send a command line (without \n); false when it can't be sent.</summary>
    bool Send(string line);

    /// <summary>Lines that came in since the last call.</summary>
    IEnumerable<string> Receive();
}

/// <summary>The emulator through <c>kb_emu_studio_command</c> / <c>next_line</c>: the same text as the keyboard.</summary>
public sealed class EmulatorStudioTransport(EmulatedKeyboard keyboard) : IStudioTransport
{
    public bool Send(string line) => keyboard.StudioCommand(line);

    public IEnumerable<string> Receive()
    {
        var lines = new List<string>();
        while (keyboard.StudioNextLine() is { } line) lines.Add(line);
        return lines;
    }
}

/// <summary>The real keyboard: lines through the Debug view's log port.</summary>
public sealed class PortStudioTransport : IStudioTransport
{
    private readonly Func<string, bool> _send;
    private readonly Queue<string> _in = new();

    public PortStudioTransport(Func<string, bool> send) => _send = send;

    public bool Send(string line) => _send(line + "\n");

    /// <summary>A line read from the port (the Debug view hands Studio lines over).</summary>
    public void Deliver(string line)
    {
        lock (_in) _in.Enqueue(line);
    }

    public IEnumerable<string> Receive()
    {
        lock (_in)
        {
            var lines = _in.ToList();
            _in.Clear();
            return lines;
        }
    }
}

/// <summary>Whether the target speaks Studio.</summary>
public enum StudioSupport
{
    Unknown,
    /// <summary><c>#studio get</c> sent, no <c>#V</c> yet.</summary>
    Asking,
    Yes,
    /// <summary>No <c>#V</c> within the timeout: this firmware has no Studio.</summary>
    No,
    /// <summary>The command couldn't be sent (no keyboard on the port).</summary>
    NotConnected,
}

/// <summary>
/// The app's side of the Studio protocol, for either target. Call
/// <see cref="Pump"/> often; it reads lines and updates the state.
/// </summary>
public sealed class StudioClient
{
    public static readonly TimeSpan VersionTimeout = TimeSpan.FromSeconds(1.2);

    private readonly IStudioTransport _transport;
    private readonly Func<DateTime> _now;
    private DateTime _askedAt;
    private DateTime? _unlockedUntil;

    public StudioClient(IStudioTransport transport, Func<DateTime>? now = null)
    {
        _transport = transport;
        _now = now ?? (() => DateTime.UtcNow);
    }

    public StudioSupport Support { get; private set; }
    public StudioVersion? Version { get; private set; }

    /// <summary>Live bindings per layer, as the keyboard sent them.</summary>
    public Dictionary<int, IReadOnlyList<string>> Layers { get; } = new();

    /// <summary>The compiled bindings (<c>#LD</c>, firmware with the defaults capability).</summary>
    public Dictionary<int, IReadOnlyList<string>> DefaultLayers { get; } = new();

    /// <summary>The live combos (latest complete <c>#C</c> list); empty without the combos capability.</summary>
    public IReadOnlyList<string> Combos { get; private set; } = [];

    /// <summary>The compiled combos (<c>#CD</c>).</summary>
    public IReadOnlyList<string> DefaultCombos { get; private set; } = [];

    private readonly List<string> _comboList = new();
    private readonly List<string> _defaultComboList = new();

    /// <summary>A capability of the firmware (check, combos, defaults); none for version 1.</summary>
    public bool Has(string capability) => Version?.Has(capability) == true;

    public bool Unlocked => _unlockedUntil is { } until && until > _now();

    /// <summary>Until the keyboard locks itself (zero when locked).</summary>
    public TimeSpan UnlockRemaining => _unlockedUntil is { } until && until > _now() ? until - _now() : TimeSpan.Zero;

    public bool Dirty { get; private set; }

    /// <summary>The last acknowledgement.</summary>
    public StudioAck? LastAck { get; private set; }

    /// <summary>Something changed (layers, lock, dirty, an ack).</summary>
    public event Action? Changed;

    public event Action<StudioAck>? Acknowledged;

    public string? Binding(int layer, int pos) =>
        Layers.TryGetValue(layer, out var actions) && pos < actions.Count ? actions[pos] : null;

    public void Get()
    {
        Support = Send("#studio get") ? StudioSupport.Asking : StudioSupport.NotConnected;
        _askedAt = _now();
        Changed?.Invoke();
    }

    public bool Status() => Send("#studio status");
    public bool Set(int layer, int pos, string action) => Send($"#studio set {layer} {pos} {action}");
    public bool Save() => Send("#studio save");
    public bool Revert() => Send("#studio revert");
    public bool Reset() => Send("#studio reset");
    public bool Lock() => Send("#studio lock");

    /// <summary>Dry run of a key change (needs the check capability).</summary>
    public bool Check(int layer, int pos, string action) => Send($"#studio check {layer} {pos} {action}");

    public bool RequestCombos() => Send("#studio combos");
    public bool RequestDefaults() => Send("#studio defaults");
    public bool ComboAdd(string combo) => Send($"#studio combo add {combo}");
    public bool ComboSet(int index, string combo) => Send($"#studio combo set {index} {combo}");
    public bool ComboRemove(int index) => Send($"#studio combo remove {index}");

    /// <summary>Dry run of a combo, as a replacement for <paramref name="index"/> or as a new one (null).</summary>
    public bool ComboCheck(int? index, string combo) => Send($"#studio combo check {(index is { } i ? i.ToString(CultureInfo.InvariantCulture) : "new")} {combo}");

    private bool Send(string line) => _transport.Send(line);

    /// <summary>Read what came in; time out the version question.</summary>
    public void Pump()
    {
        var changed = false;
        foreach (var raw in _transport.Receive())
        {
            if (StudioLine.Parse(raw) is not { } line) continue;
            Apply(line);
            changed = true;
        }
        if (Support == StudioSupport.Asking && _now() - _askedAt > VersionTimeout)
        {
            Support = StudioSupport.No;
            changed = true;
        }
        if (_unlockedUntil is { } until && until <= _now())
        {
            _unlockedUntil = null;
            changed = true;
        }
        if (changed) Changed?.Invoke();
    }

    /// <summary>Take one line into the state (tests feed lines directly).</summary>
    public void Apply(StudioLine line)
    {
        switch (line)
        {
            case StudioVersion v:
                var first = Version == null;
                Version = v;
                Support = StudioSupport.Yes;
                // The compiled keymap of this firmware, once: changes are marked against it.
                if (first && v.Has("defaults")) RequestDefaults();
                break;
            case StudioLayer { Default: true } l:
                DefaultLayers[l.Layer] = l.Actions;
                break;
            case StudioLayer l:
                Layers[l.Layer] = l.Actions;
                break;
            case StudioComboLine { Default: true } c:
                if (c.Index == 0) _defaultComboList.Clear();
                _defaultComboList.Add(c.Combo);
                break;
            case StudioComboLine c:
                if (c.Index == 0) _comboList.Clear();
                _comboList.Add(c.Combo);
                break;
            case StudioComboEnd { Default: true } e:
                DefaultCombos = _defaultComboList.Take(e.Count).ToList();
                _defaultComboList.Clear();
                break;
            case StudioComboEnd e:
                Combos = _comboList.Take(e.Count).ToList();
                _comboList.Clear();
                break;
            case StudioUnlock u:
                _unlockedUntil = u.Unlocked ? _now() + TimeSpan.FromSeconds(u.Seconds) : null;
                break;
            case StudioDirty d:
                Dirty = d.Dirty;
                break;
            case StudioAck a:
                LastAck = a;
                if (a is { Command: "set", Ok: true, Layer: { } layer, Pos: { } pos } && Layers.TryGetValue(layer, out var actions) && pos < actions.Count)
                {
                    var copy = actions.ToArray();
                    copy[pos] = a.Detail;
                    Layers[layer] = copy;
                }
                Acknowledged?.Invoke(a);
                break;
        }
    }
}

/// <summary>Behaviors the editor offers.</summary>
public enum StudioKind
{
    Key,
    ModTap,
    Layer,
    CapsWord,
    System,
    Transparent,
    None,
    /// <summary><c>mm:&lt;dir&gt;</c>: move the pointer while held.</summary>
    MouseMove,
    /// <summary><c>mb:&lt;button&gt;</c>: a mouse button.</summary>
    MouseButton,
    /// <summary><c>ms:&lt;dir&gt;</c>: scroll while held.</summary>
    Scroll,
    /// <summary><c>mp</c>: precise (slow) movement while held.</summary>
    Precision,
    /// <summary><c>mk:&lt;button|scroll_dir&gt;/&lt;key&gt;</c>: mouse while the pointer is active, else the key.</summary>
    MouseOrKey,
}

/// <summary>
/// What the editor panel edits, built into an encoding (<c>kp:LC(C)</c>,
/// <c>mt:LALT/TAB</c>, <c>mo:2</c> …) and read back from one.
/// </summary>
public sealed class StudioAction
{
    /// <summary>Modifier functions in canonical order (outermost first).</summary>
    public static readonly string[] Modifiers = ["LG", "LC", "LS", "LA", "RG", "RC", "RS", "RA"];

    /// <summary>Keys a mod-tap can hold.</summary>
    public static readonly string[] HoldKeys = ["LCTRL", "LSHFT", "LALT", "LGUI", "RCTRL", "RSHFT", "RALT", "RGUI"];

    public StudioKind Kind { get; set; } = StudioKind.Key;

    /// <summary>The key (Key: pressed; ModTap: tapped).</summary>
    public string Key { get; set; } = "A";

    /// <summary>Modifier functions around the key (LC, LS…).</summary>
    public HashSet<string> Mods { get; } = new();

    /// <summary>Mod-tap: what is held.</summary>
    public string Hold { get; set; } = "LALT";

    public int Layer { get; set; } = 1;

    public string System { get; set; } = "BtNext";

    /// <summary>Directions of mm: and ms: (left/right on ms: is the horizontal pan).</summary>
    public static readonly string[] Directions = ["left", "down", "up", "right"];

    public static readonly string[] Buttons = ["left", "right", "middle"];

    /// <summary>What an mk: key does while the pointer is active: a button or a scroll.</summary>
    public static readonly string[] MouseOrKeyChoices = ["left", "right", "middle", "scroll_left", "scroll_down", "scroll_up", "scroll_right"];

    /// <summary>The kinds that need firmware with the mouse capability.</summary>
    public static bool IsMouse(StudioKind kind) => kind is StudioKind.MouseMove or StudioKind.MouseButton or StudioKind.Scroll or StudioKind.Precision or StudioKind.MouseOrKey;

    /// <summary>mm: / ms: direction.</summary>
    public string Direction { get; set; } = "left";

    /// <summary>mb: button.</summary>
    public string Button { get; set; } = "left";

    /// <summary>mk: what the mouse does (a button, or scroll_&lt;dir&gt;); the key is <see cref="Key"/> with <see cref="Mods"/>.</summary>
    public string MouseAction { get; set; } = "left";

    public string Encode() => Kind switch
    {
        StudioKind.Key => "kp:" + Wrap(Key),
        StudioKind.ModTap => $"mt:{Hold}/{Wrap(Key)}",
        StudioKind.Layer => $"mo:{Layer}",
        StudioKind.CapsWord => "caps",
        StudioKind.System => "sys:" + System,
        StudioKind.Transparent => "trans",
        StudioKind.MouseMove => "mm:" + Direction,
        StudioKind.MouseButton => "mb:" + Button,
        StudioKind.Scroll => "ms:" + Direction,
        StudioKind.Precision => "mp",
        StudioKind.MouseOrKey => $"mk:{MouseAction}/{Wrap(Key)}",
        _ => "none",
    };

    private string Wrap(string key)
    {
        foreach (var mod in Modifiers.Reverse().Where(Mods.Contains))
            key = $"{mod}({key})";
        return key;
    }

    /// <summary>The editor model of an encoding (any case; canonical or not).</summary>
    public static StudioAction Parse(string encoding)
    {
        var e = encoding.Trim();
        var a = new StudioAction();
        var lower = e.ToLowerInvariant();
        a.Kind = lower switch
        {
            "trans" => StudioKind.Transparent,
            "none" => StudioKind.None,
            "caps" => StudioKind.CapsWord,
            "mp" => StudioKind.Precision,
            _ => ParsePrefixed(a, e, lower),
        };
        return a;
    }

    /// <summary>An encoding with a <c>kind:</c> prefix; its fields go into <paramref name="a"/>. None when unknown.</summary>
    private static StudioKind ParsePrefixed(StudioAction a, string e, string lower)
    {
        if (lower.StartsWith("mo:", StringComparison.Ordinal) && int.TryParse(e[3..], CultureInfo.InvariantCulture, out var layer))
        {
            a.Layer = layer;
            return StudioKind.Layer;
        }
        if (lower.StartsWith("sys:", StringComparison.Ordinal))
        {
            a.System = e[4..];
            return StudioKind.System;
        }
        if (lower.StartsWith("mt:", StringComparison.Ordinal) && e.IndexOf('/') is > 3 and var slash)
        {
            a.Hold = e[3..slash].ToUpperInvariant();
            a.Key = Unwrap(e[(slash + 1)..], a.Mods);
            return StudioKind.ModTap;
        }
        if (lower.StartsWith("mk:", StringComparison.Ordinal) && e.IndexOf('/') is > 3 and var mkSlash)
        {
            a.MouseAction = lower[3..mkSlash];
            a.Key = Unwrap(e[(mkSlash + 1)..], a.Mods);
            return StudioKind.MouseOrKey;
        }
        if (lower.StartsWith("kp:", StringComparison.Ordinal))
        {
            a.Key = Unwrap(e[3..], a.Mods);
            return StudioKind.Key;
        }
        return ParseMouse(a, lower);
    }

    /// <summary><c>mm:</c> move, <c>ms:</c> scroll, <c>mb:</c> button; None for anything else.</summary>
    private static StudioKind ParseMouse(StudioAction a, string lower)
    {
        if (lower.Length < 3 || lower[2] != ':') return StudioKind.None;
        var rest = lower[3..];
        switch (lower[..2])
        {
            case "mm":
                a.Direction = rest;
                return StudioKind.MouseMove;
            case "ms":
                a.Direction = rest;
                return StudioKind.Scroll;
            case "mb":
                a.Button = rest;
                return StudioKind.MouseButton;
            default:
                return StudioKind.None;
        }
    }

    private static string Unwrap(string key, HashSet<string> mods)
    {
        key = key.ToUpperInvariant();
        while (key.Length > 4 && key[2] == '(' && key.EndsWith(')') && Modifiers.Contains(key[..2]))
        {
            mods.Add(key[..2]);
            key = key[3..^1];
        }
        return key;
    }
}

/// <summary>
/// A combo as the editor edits it: <c>&lt;name|-&gt; &lt;positions&gt; &lt;layers|*&gt; &lt;timeout&gt; &lt;idle&gt; &lt;action&gt;</c>.
/// </summary>
public sealed class StudioCombo
{
    public string Name { get; set; } = "";
    public SortedSet<int> Positions { get; } = new();

    /// <summary>Layers it is active on; empty = all (<c>*</c>).</summary>
    public SortedSet<int> Layers { get; } = new();

    public int Timeout { get; set; } = 50;
    public int Idle { get; set; } = 150;
    public string Action { get; set; } = "kp:ESC";

    public string Encode() =>
        string.Join(' ',
            Name.Length == 0 ? "-" : Name,
            string.Join(',', Positions),
            Layers.Count == 0 ? "*" : string.Join(',', Layers),
            Timeout.ToString(CultureInfo.InvariantCulture),
            Idle.ToString(CultureInfo.InvariantCulture),
            Action);

    /// <summary>The editor model of a combo's six words; null when it isn't six words.</summary>
    public static StudioCombo? Parse(string text)
    {
        var w = text.Trim().Split(' ', StringSplitOptions.RemoveEmptyEntries);
        if (w.Length != 6) return null;
        var c = new StudioCombo { Name = w[0] == "-" ? "" : w[0], Action = w[5] };
        foreach (var p in w[1].Split(',', StringSplitOptions.RemoveEmptyEntries))
        {
            if (int.TryParse(p, CultureInfo.InvariantCulture, out var n)) c.Positions.Add(n);
        }
        if (w[2] != "*")
        {
            foreach (var l in w[2].Split(',', StringSplitOptions.RemoveEmptyEntries))
            {
                if (int.TryParse(l, CultureInfo.InvariantCulture, out var n)) c.Layers.Add(n);
            }
        }
        c.Timeout = int.TryParse(w[3], CultureInfo.InvariantCulture, out var t) ? t : 50;
        c.Idle = int.TryParse(w[4], CultureInfo.InvariantCulture, out var i) ? i : 0;
        return c;
    }

}

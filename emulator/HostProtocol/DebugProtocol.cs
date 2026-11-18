using System.Globalization;
using System.Text;

namespace Kb.HostProtocol;

/// <summary>Something the keyboard's log port sent (see <c>core/src/debug.rs</c>).</summary>
public abstract record DebugEvent;

/// <summary>A plain log line (or anything that isn't a valid event line).</summary>
public sealed record DebugLog(string Text) : DebugEvent;

/// <summary><c>#H 1 erik_kbd left ble</c>: the debug stream started.</summary>
public sealed record DebugHello(int Version, string Name, string Half, string Build) : DebugEvent;

/// <summary><c>#K 0000000000001001</c>: bit n set while key position n is held (both halves).</summary>
public sealed record DebugKeys(ulong Held) : DebugEvent
{
    public bool IsHeld(int pos) => pos is >= 0 and < 64 && (Held & (1UL << pos)) != 0;
}

/// <summary><c>#R 0200040000000000</c>: the boot keyboard report for the computer (mods, 0, 6 keys).</summary>
public sealed record DebugReport(byte Mods, byte[] Keys) : DebugEvent;

/// <summary><c>#F base64</c>: the left OLED frame (512 bytes, SSD1306 pages); null while the OLED is off.</summary>
public sealed record DebugFrame(byte[]? Frame) : DebugEvent;

/// <summary><c>#M</c>: the last mouse report for the computer.</summary>
public sealed record DebugMouse(MouseReport Report) : DebugEvent;

/// <summary>
/// A mouse report as the computer gets it: buttons (bit 0 left, 1 right, 2
/// middle), x right +, y down +, wheel up +, pan right +.
/// </summary>
public sealed record MouseReport(byte Buttons, short X, short Y, sbyte Wheel, sbyte Pan)
{
    /// <summary>Bytes in a report: buttons, x (i16 LE), y (i16 LE), wheel, pan.</summary>
    public const int Size = 7;

    /// <summary>The <see cref="Size"/> report bytes: buttons, x (i16 LE), y (i16 LE), wheel, pan.</summary>
    public static MouseReport? FromBytes(ReadOnlySpan<byte> b) =>
        b.Length < Size ? null : new MouseReport(b[0], (short)(b[1] | b[2] << 8), (short)(b[3] | b[4] << 8), (sbyte)b[5], (sbyte)b[6]);

    /// <summary>14 hex digits, as in <c>#M 01d4feb004ff01</c>.</summary>
    public static MouseReport? FromHex(string hex)
    {
        if (hex.Length != 2 * Size) return null;
        try
        {
            return FromBytes(Convert.FromHexString(hex));
        }
        catch (FormatException)
        {
            return null;
        }
    }

    /// <summary>Button names held: "L", "R", "M".</summary>
    public static string ButtonNames(byte buttons) =>
        string.Concat((buttons & 1) != 0 ? "L" : "", (buttons & 2) != 0 ? "R" : "", (buttons & 4) != 0 ? "M" : "");

    /// <summary>Compact: "L · (+12, −4) · wheel −1".</summary>
    public string Describe()
    {
        var parts = new List<string>();
        if (Buttons != 0) parts.Add(ButtonNames(Buttons));
        if (X != 0 || Y != 0) parts.Add($"({WithSign(X)}, {WithSign(Y)})");
        if (Wheel != 0) parts.Add($"wheel {WithSign(Wheel)}");
        if (Pan != 0) parts.Add($"pan {WithSign(Pan)}");
        return parts.Count == 0 ? "idle" : string.Join(" · ", parts);
    }

    /// <summary>The number with its sign: "+12", "−4" (a real minus sign), "0".</summary>
    public static string WithSign(int n) => n switch
    {
        > 0 => $"+{n}",
        < 0 => $"−{-n}",
        _ => "0",
    };
}

/// <summary><c>#S key=value ...</c>: the keyboard's status.</summary>
public sealed record DebugStatus : DebugEvent
{
    public int Layer { get; init; }
    public bool CapsWord { get; init; }
    public byte Mods { get; init; }
    /// <summary>"usb" or "ble".</summary>
    public string Output { get; init; } = "usb";
    /// <summary>Bluetooth profile, 1-based (as on the screen).</summary>
    public int Profile { get; init; } = 1;
    public bool HostConnected { get; init; }
    public bool ProfileOpen { get; init; }
    public bool Linked { get; init; }
    /// <summary>Split link stage; empty for none.</summary>
    public string Stage { get; init; } = "";
    public int BatteryLeft { get; init; } = 100;
    public int BatteryRight { get; init; } = 100;
    public bool ChargingLeft { get; init; }
    public bool ChargingRight { get; init; }
    public int Wpm { get; init; }
    /// <summary>"active", "idle" or "sleep".</summary>
    public string Power { get; init; } = "active";
    public int Presses { get; init; }
    public bool Mute { get; init; }
    public bool Trace { get; init; }
    public bool Lit { get; init; }
    /// <summary>Radio transmit power in dBm (RadioPower key); null from a firmware that doesn't send it.</summary>
    public int? RadioDbm { get; init; }

    /// <summary>The mouse pointer is active (MOUSE bar); null from a firmware without mouse keys.</summary>
    public bool? PointerActive { get; init; }
}

/// <summary>
/// The live debug stream on the keyboard's log port (USB serial, left half):
/// text lines, log lines as they are, event lines starting with <c>#</c>.
/// Same format as <c>core/src/debug.rs</c>; commands as <c>core/src/console.rs</c>.
/// </summary>
public static class DebugProtocol
{
    public const int Version = 1;
    public const int FrameBytes = 512;

    /// <summary>Start the debug stream (again: everything is resent).</summary>
    public const string DebugOn = "#debug 1\n";
    public const string DebugOff = "#debug 0\n";
    public const string Status = "s\n";
    public const string Trace = "t\n";
    public const string Mute = "m\n";
    public const string Dump = "d\n";
    public const string Bootloader = "boot\n";

    public static string Press(int pos) => $"p{pos}\n";
    public static string Release(int pos) => $"r{pos}\n";
    public static string Tap(int pos) => $"k{pos}\n";

    /// <summary>Parse one line (without its line ending).</summary>
    public static DebugEvent Parse(string line)
    {
        if (line.Length < 3 || line[0] != '#' || line[2] != ' ') return new DebugLog(line);
        var payload = line[3..];
        DebugEvent? parsed = line[1] switch
        {
            'H' => ParseHello(payload),
            'K' => ulong.TryParse(payload, NumberStyles.HexNumber, CultureInfo.InvariantCulture, out var held) && payload.Length == 16
                ? new DebugKeys(held)
                : null,
            'R' => ParseReport(payload),
            'F' => ParseFrame(payload),
            'S' => ParseStatus(payload),
            'M' => MouseReport.FromHex(payload) is { } mouse ? new DebugMouse(mouse) : null,
            _ => null,
        };
        return parsed ?? new DebugLog(line);
    }

    private static DebugHello? ParseHello(string payload)
    {
        var parts = payload.Split(' ');
        if (parts.Length < 4 || !int.TryParse(parts[0], CultureInfo.InvariantCulture, out var version)) return null;
        return new DebugHello(version, parts[1], parts[2], parts[3]);
    }

    private static DebugReport? ParseReport(string payload)
    {
        if (payload.Length != 16) return null;
        try
        {
            var bytes = Convert.FromHexString(payload);
            return new DebugReport(bytes[0], bytes[2..]);
        }
        catch (FormatException)
        {
            return null;
        }
    }

    private static DebugFrame? ParseFrame(string payload)
    {
        if (payload == "-") return new DebugFrame(null);
        var bytes = new byte[FrameBytes];
        return Convert.TryFromBase64String(payload, bytes, out var n) && n == FrameBytes ? new DebugFrame(bytes) : null;
    }

    private static DebugStatus? ParseStatus(string payload)
    {
        if (StatusFields.Read(payload) is not { } f) return null;
        var (batteryLeft, batteryRight) = f.Pair("bat", 100);
        var (chargingLeft, chargingRight) = f.Pair("chg", 0);
        var stage = f.Text("stage", "-");
        return new DebugStatus
        {
            Layer = f.Int("layer", 0),
            CapsWord = f.Bit("caps"),
            Mods = f.Hex("mods"),
            Output = f.Text("out", "usb"),
            Profile = f.Int("profile", 1),
            HostConnected = f.Bit("host"),
            ProfileOpen = f.Bit("open"),
            Linked = f.Bit("linked"),
            Stage = stage == "-" ? "" : stage,
            BatteryLeft = batteryLeft,
            BatteryRight = batteryRight,
            ChargingLeft = chargingLeft == 1,
            ChargingRight = chargingRight == 1,
            Wpm = f.Int("wpm", 0),
            Power = f.Text("power", "active"),
            Presses = f.Int("presses", 0),
            Mute = f.Bit("mute"),
            Trace = f.Bit("trace"),
            Lit = f.Bit("lit"),
            RadioDbm = f.SignedOrNull("radio"),
            PointerActive = f.BitOrNull("pointer"),
        };
    }

    /// <summary>The <c>key=value</c> fields of a status line.</summary>
    private sealed class StatusFields
    {
        private readonly Dictionary<string, string> _fields;

        private StatusFields(Dictionary<string, string> fields) => _fields = fields;

        /// <summary>The fields; null when a part isn't <c>key=value</c> or there are none.</summary>
        public static StatusFields? Read(string payload)
        {
            var fields = new Dictionary<string, string>();
            foreach (var part in payload.Split(' ', StringSplitOptions.RemoveEmptyEntries))
            {
                var eq = part.IndexOf('=');
                if (eq <= 0) return null;
                fields[part[..eq]] = part[(eq + 1)..];
            }
            return fields.Count == 0 ? null : new StatusFields(fields);
        }

        public string Text(string key, string fallback) => _fields.TryGetValue(key, out var v) ? v : fallback;

        public int Int(string key, int fallback) =>
            _fields.TryGetValue(key, out var v) && int.TryParse(v, CultureInfo.InvariantCulture, out var n) ? n : fallback;

        public bool Bit(string key) => Text(key, "0") == "1";

        /// <summary>Two numbers, <c>a,b</c>; both <paramref name="fallback"/> when missing or malformed.</summary>
        public (int, int) Pair(string key, int fallback)
        {
            var v = Text(key, "").Split(',');
            return v.Length == 2 && int.TryParse(v[0], CultureInfo.InvariantCulture, out var a) && int.TryParse(v[1], CultureInfo.InvariantCulture, out var b)
                ? (a, b)
                : (fallback, fallback);
        }

        /// <summary>A byte in hex (the modifier bits); 0 when missing or malformed.</summary>
        public byte Hex(string key) =>
            byte.TryParse(Text(key, "0"), NumberStyles.HexNumber, CultureInfo.InvariantCulture, out var value) ? value : (byte)0;

        /// <summary>A number with an optional sign; null when the firmware doesn't send it.</summary>
        public int? SignedOrNull(string key) =>
            _fields.TryGetValue(key, out var v) && int.TryParse(v, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out var n)
                ? n
                : null;

        /// <summary>A bit; null when the firmware doesn't send it.</summary>
        public bool? BitOrNull(string key) => _fields.TryGetValue(key, out var v) ? v == "1" : null;
    }
}

/// <summary>
/// Splits the bytes read from the port into lines (ending in <c>\n</c>,
/// with or without <c>\r</c>). A line that never ends is cut at
/// <see cref="MaxLine"/> so junk can't grow without bound.
/// </summary>
public sealed class DebugLineSplitter
{
    public const int MaxLine = 4096;

    private readonly List<byte> _partial = new();

    public List<string> Feed(ReadOnlySpan<byte> bytes)
    {
        var lines = new List<string>();
        foreach (var b in bytes)
        {
            if (b == (byte)'\n')
            {
                lines.Add(Take());
            }
            else if (b != (byte)'\r')
            {
                _partial.Add(b);
                if (_partial.Count >= MaxLine) lines.Add(Take());
            }
        }
        return lines;
    }

    /// <summary>Forget a partial line (the connection was lost).</summary>
    public void Reset() => _partial.Clear();

    private string Take()
    {
        var line = Encoding.UTF8.GetString(_partial.ToArray());
        _partial.Clear();
        return line;
    }
}

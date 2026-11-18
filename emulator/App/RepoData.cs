using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;

namespace KbEmulator;

/// <summary>Where the keyboard repository is: <c>EMK_REPO</c>, else found from the app's folder.</summary>
public static class RepoLocator
{
    /// <summary>The repo root (has <c>hil</c> and <c>emulator/</c>), or null.</summary>
    public static string? Find(string? startDirectory = null, Func<string, string?>? env = null)
    {
        env ??= Environment.GetEnvironmentVariable;
        if (env("EMK_REPO") is { Length: > 0 } configured && Directory.Exists(configured)) return Path.GetFullPath(configured);
        for (var dir = new DirectoryInfo(startDirectory ?? AppContext.BaseDirectory); dir != null; dir = dir.Parent)
        {
            if (File.Exists(Path.Combine(dir.FullName, "hil")) && Directory.Exists(Path.Combine(dir.FullName, "emulator"))) return dir.FullName;
        }
        return null;
    }
}

/// <summary>The bootloader kinds <c>./hil</c> records and finds: UF2 (a drive), or signed (serial DFU only).</summary>
internal static class BootloaderKind
{
    public const string Uf2 = "uf2";
    public const string Signed = "signed";
}

/// <summary>
/// One line of <c>.hil-halves</c>: which nice!nano (by USB serial) is which
/// half. <c>./hil flash</c> writes <c>&lt;name&gt; &lt;side&gt; &lt;serial&gt; &lt;uf2|signed&gt;</c>;
/// older lines have no bootloader column.
/// </summary>
public sealed record HalfRecord(string Name, string Side, string Serial, string? Bootloader)
{
    public const string FileName = ".hil-halves";

    public bool IsSigned => Bootloader == BootloaderKind.Signed;

    /// <summary>Lines with 3 or 4 columns; comments, blank and malformed lines are skipped.</summary>
    public static IReadOnlyList<HalfRecord> Parse(string text)
    {
        var halves = new List<HalfRecord>();
        foreach (var raw in text.Split('\n'))
        {
            var line = raw.Trim();
            if (line.Length == 0 || line.StartsWith('#')) continue;
            var cols = line.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
            if (cols.Length is < 3 or > 4 || cols[1] is not ("left" or "right")) continue;
            halves.Add(new HalfRecord(cols[0], cols[1], cols[2], cols.Length == 4 ? cols[3] : null));
        }
        return halves;
    }

    /// <summary>The repo's <c>.hil-halves</c>; empty when it is missing or unreadable.</summary>
    public static IReadOnlyList<HalfRecord> Load(string? repo)
    {
        if (repo == null) return [];
        try
        {
            var path = Path.Combine(repo, FileName);
            return File.Exists(path) ? Parse(File.ReadAllText(path)) : [];
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            return [];
        }
    }
}

/// <summary>What <c>fw-nrf/dfu-verify.py</c> said about a signed package (<c>.verify.txt</c> next to it).</summary>
public sealed record VerifyResult(bool Ok, int Checks, string Summary)
{
    public static VerifyResult Parse(string text)
    {
        var lines = text.Split('\n').Select(l => l.Trim()).ToList();
        var fails = lines.Where(l => l.StartsWith("FAIL", StringComparison.Ordinal)).ToList();
        var oks = lines.Count(l => l.StartsWith("ok", StringComparison.Ordinal));
        if (fails.Count > 0) return new VerifyResult(false, oks, fails[0]);
        return oks > 0
            ? new VerifyResult(true, oks, $"dfu-verify: {oks} checks ok")
            : new VerifyResult(false, oks, "dfu-verify: no checks in the file");
    }
}

/// <summary>A firmware package in <c>target/</c>: <c>fw-nrf-&lt;side&gt;-ble-&lt;name&gt;.uf2|.zip</c>.</summary>
public sealed partial record FirmwarePackage(string Name, string Side, string Kind, string Path, long Size, DateTime Modified, VerifyResult? Verify)
{
    public string FileName => System.IO.Path.GetFileName(Path);

    [GeneratedRegex(@"^fw-nrf-(left|right)-ble-(.+)\.(uf2|zip)$")]
    private static partial Regex Pattern();

    /// <summary>The packages in <paramref name="targetDirectory"/>; empty when it is missing.</summary>
    public static IReadOnlyList<FirmwarePackage> Scan(string? targetDirectory)
    {
        if (targetDirectory == null || !Directory.Exists(targetDirectory)) return [];
        var packages = new List<FirmwarePackage>();
        try
        {
            foreach (var file in Directory.EnumerateFiles(targetDirectory, "fw-nrf-*"))
            {
                var m = Pattern().Match(System.IO.Path.GetFileName(file));
                if (!m.Success) continue;
                var info = new FileInfo(file);
                VerifyResult? verify = null;
                var verifyPath = System.IO.Path.ChangeExtension(file, ".verify.txt");
                if (m.Groups[3].Value == "zip" && File.Exists(verifyPath)) verify = VerifyResult.Parse(File.ReadAllText(verifyPath));
                packages.Add(new FirmwarePackage(m.Groups[2].Value, m.Groups[1].Value, m.Groups[3].Value, file, info.Length, info.LastWriteTime, verify));
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // target/ changed or can't be read: show what was found so far.
        }
        return packages.OrderBy(p => p.Name).ThenBy(p => p.Side).ThenBy(p => p.Kind).ToList();
    }
}

/// <summary>
/// The firmware signing key, from its file's metadata only: the key itself
/// is never read.
/// </summary>
public sealed record SigningKeyStatus(string Path, bool Present, DateTime? Modified, bool OwnerOnly)
{
    public static string DefaultPath =>
        Environment.GetEnvironmentVariable("EMK_KEY") is { Length: > 0 } key
            ? key
            : System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".config", "emk", "keys", "emk-fw.pem");

    public static SigningKeyStatus Inspect(string? path = null)
    {
        path ??= DefaultPath;
        var info = new FileInfo(path);
        if (!info.Exists) return new SigningKeyStatus(path, false, null, false);
        var ownerOnly = OperatingSystem.IsWindows() ||
                        (info.UnixFileMode & (UnixFileMode.GroupRead | UnixFileMode.GroupWrite | UnixFileMode.OtherRead | UnixFileMode.OtherWrite)) == 0;
        return new SigningKeyStatus(path, true, info.LastWriteTime, ownerOnly);
    }

    /// <summary>The path with the home folder as ~.</summary>
    public string Display
    {
        get
        {
            var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
            return home.Length > 0 && Path.StartsWith(home, StringComparison.Ordinal) ? "~" + Path[home.Length..] : Path;
        }
    }
}

/// <summary>A nice!nano in its bootloader on USB right now (239a:00b3): UF2 (with a drive) or signed (serial only).</summary>
public sealed record UsbBootloader(string Serial, string Kind)
{
    public const string Vendor = "239a";

    /// <summary>Read <c>/sys/bus/usb/devices</c> (or <paramref name="sysRoot"/>); empty when there is none.</summary>
    public static IReadOnlyList<UsbBootloader> Scan(string sysRoot = "/sys/bus/usb/devices")
    {
        var found = new List<UsbBootloader>();
        if (!Directory.Exists(sysRoot)) return found;
        try
        {
            foreach (var device in Directory.EnumerateDirectories(sysRoot))
            {
                var vendor = Read(System.IO.Path.Combine(device, "idVendor"));
                if (vendor != Vendor) continue;
                var serial = Read(System.IO.Path.Combine(device, "serial")) ?? "";
                // Mass storage (class 08) on any interface: the UF2 drive.
                var storage = Directory.EnumerateDirectories(device)
                    .Any(i => Read(System.IO.Path.Combine(i, "bInterfaceClass")) == "08");
                found.Add(new UsbBootloader(serial, storage ? BootloaderKind.Uf2 : BootloaderKind.Signed));
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // A device went away while listing: what was found so far is still right.
        }
        return found;

        static string? Read(string path)
        {
            try
            {
                return File.Exists(path) ? File.ReadAllText(path).Trim() : null;
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                return null;
            }
        }
    }
}

/// <summary>
/// What a Debug session learned from the firmware's boot line
/// (<c>boot: bootloader 0.11.0, entry serial DFU (signed), debug port open</c>):
/// the latest one seen while the app runs.
/// </summary>
public sealed record BootFacts(string Keyboard, string Half, string Version, string Entry, string DebugPort, DateTime Seen)
{
    public bool SignedEntry => Entry.Contains("signed", StringComparison.OrdinalIgnoreCase);
}

/// <summary>The tamper alarm rules, section 6 of <c>docs/signed-bootloader.md</c>.</summary>
public static partial class TamperRules
{
    public sealed record Rule(string Lead, string Rest, bool Alarm);

    public const string Doc = "docs/signed-bootloader.md";

    /// <summary>The intro and the bullet rules; empty when the section is not there.</summary>
    public static (string Intro, IReadOnlyList<Rule> Rules) Parse(string markdown)
    {
        var lines = markdown.Replace("\r", "").Split('\n');
        var start = Array.FindIndex(lines, l => SectionHeading().IsMatch(l));
        if (start < 0) return ("", []);
        var intro = new StringBuilder();
        var rules = new List<StringBuilder>();
        for (var i = start + 1; i < lines.Length; i++)
        {
            var line = lines[i];
            if (line.StartsWith("## ", StringComparison.Ordinal) || line.Trim() == "---") break;
            if (line.StartsWith("- ", StringComparison.Ordinal)) rules.Add(new StringBuilder(line[2..].Trim()));
            else if (line.Trim().Length == 0) continue;
            else if (rules.Count > 0) rules[^1].Append(' ').Append(line.Trim());
            else intro.Append(intro.Length > 0 ? " " : "").Append(line.Trim());
        }
        return (Plain(intro.ToString()), rules.Select(r => ToRule(r.ToString())).ToList());
    }

    public static (string Intro, IReadOnlyList<Rule> Rules) Load(string? repo)
    {
        try
        {
            var path = repo == null ? null : Path.Combine(repo, Doc);
            return path != null && File.Exists(path) ? Parse(File.ReadAllText(path)) : ("", []);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            return ("", []);
        }
    }

    private static Rule ToRule(string markdown)
    {
        // The bold part leads; without one, the text up to the first full stop.
        var bold = Bold().Match(markdown);
        string lead, rest;
        if (bold.Success)
        {
            lead = bold.Groups[1].Value;
            rest = markdown.Remove(bold.Index, bold.Length).Trim();
        }
        else
        {
            var dot = markdown.IndexOf(". ", StringComparison.Ordinal);
            (lead, rest) = dot > 0 ? (markdown[..(dot + 1)], markdown[(dot + 2)..]) : (markdown, "");
        }
        var alarm = !NotAnAlarm().IsMatch(markdown);
        return new Rule(Plain(lead), Plain(rest), alarm);
    }

    /// <summary>Markdown emphasis and code marks removed.</summary>
    private static string Plain(string s) => Marks().Replace(s, "").Trim();

    [GeneratedRegex(@"^##\s+\d*\.?\s*Tamper alarm rules", RegexOptions.IgnoreCase, matchTimeoutMilliseconds: 1000)]
    private static partial Regex SectionHeading();

    [GeneratedRegex(@"\*\*(.+?)\*\*", RegexOptions.None, matchTimeoutMilliseconds: 1000)]
    private static partial Regex Bold();

    [GeneratedRegex(@"\bnot an alarm\b|only a hint", RegexOptions.IgnoreCase, matchTimeoutMilliseconds: 1000)]
    private static partial Regex NotAnAlarm();

    [GeneratedRegex(@"\*\*|`", RegexOptions.None, matchTimeoutMilliseconds: 1000)]
    private static partial Regex Marks();
}

/// <summary>Everything Flash and Security show for one keyboard.</summary>
public sealed class KeyboardFacts
{
    public required string Name { get; init; }
    public required IReadOnlyList<HalfRecord> Halves { get; init; }
    public required IReadOnlyList<FirmwarePackage> Packages { get; init; }
    public required IReadOnlyList<UsbBootloader> OnUsb { get; init; }

    /// <summary>Every recorded half, of all keyboards (to name a nice!nano on USB).</summary>
    public IReadOnlyList<HalfRecord> AllHalves { get; init; } = [];

    /// <summary>Who a serial belongs to: "erik_kbd right", or null when it isn't recorded.</summary>
    public string? Owner(string serial) =>
        AllHalves.FirstOrDefault(h => h.Serial.Equals(serial, StringComparison.OrdinalIgnoreCase)) is { } h ? $"{h.Name} {h.Side}" : null;

    /// <summary>Whether a log port path (/dev/serial/by-id/usb-erik_&lt;name&gt;-if0N) is this keyboard's.</summary>
    public bool IsOwnPort(string port) => Path.GetFileName(port).Contains($"_{Name}-", StringComparison.Ordinal);
    public BootFacts? Boot { get; init; }

    public HalfRecord? Half(string side) => Halves.FirstOrDefault(h => h.Side == side);

    public IEnumerable<FirmwarePackage> PackagesFor(string side) => Packages.Where(p => p.Side == side);

    /// <summary>The bootloader of a half on USB right now, matched by serial.</summary>
    public UsbBootloader? LiveBootloader(string side) =>
        Half(side) is { } h ? OnUsb.FirstOrDefault(b => b.Serial.Equals(h.Serial, StringComparison.OrdinalIgnoreCase)) : null;

    /// <summary>
    /// The tamper alarm <c>./hil</c> raises: a half recorded as signed shows a
    /// UF2 bootloader (on USB now, or in its boot line). Null when nothing says so.
    /// </summary>
    public string? Alarm(string side)
    {
        if (Half(side) is not { IsSigned: true } h) return null;
        if (LiveBootloader(side) is { Kind: BootloaderKind.Uf2 })
        {
            return $"ALARM: {Name} {side} ({h.Serial}) had the signed bootloader and now shows a UF2 drive.";
        }
        if (Boot is { } b && b.Keyboard == Name && b.Half == side && !b.SignedEntry)
        {
            return $"ALARM: {Name} {side} ({h.Serial}) is recorded as signed, but its boot line says entry {b.Entry}.";
        }
        return null;
    }

    /// <summary>The <c>./hil</c> command for a half, with this keyboard's name.</summary>
    public string Command(string verb, string side) => $"KB_NAME={Name} ./hil {verb} {side}";

    /// <summary>Keyboards named in <c>.hil-halves</c> or by a package, in order.</summary>
    public static IReadOnlyList<string> Names(IEnumerable<HalfRecord> halves, IEnumerable<FirmwarePackage> packages) =>
        halves.Select(h => h.Name).Concat(packages.Select(p => p.Name)).Distinct().Order(StringComparer.Ordinal).ToList();

    public static string Size(long bytes) => bytes >= 1024 * 1024
        ? (bytes / (1024.0 * 1024)).ToString("0.0", CultureInfo.InvariantCulture) + " MB"
        : (bytes / 1024.0).ToString("0", CultureInfo.InvariantCulture) + " KB";
}

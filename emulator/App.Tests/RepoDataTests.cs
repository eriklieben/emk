using System.Globalization;
using System.Text.RegularExpressions;
using Avalonia.Headless;

namespace KbEmulator.Tests;

/// <summary>A throwaway repo folder with the files Flash and Security read.</summary>
internal sealed class FakeRepo : IDisposable
{
    public FakeRepo()
    {
        Root = Path.Combine(Path.GetTempPath(), $"emk-repo-{Guid.NewGuid():N}");
        Directory.CreateDirectory(Path.Combine(Root, "target"));
        Directory.CreateDirectory(Path.Combine(Root, "emulator"));
        File.WriteAllText(Path.Combine(Root, "hil"), "#!/bin/bash\n");
    }

    public string Root { get; }

    public string Target => Path.Combine(Root, "target");

    public FakeRepo Halves(string text)
    {
        File.WriteAllText(Path.Combine(Root, HalfRecord.FileName), text);
        return this;
    }

    public FakeRepo File_(string relative, string content)
    {
        var path = Path.Combine(Root, relative);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, content);
        return this;
    }

    public void Dispose()
    {
        try
        {
            Directory.Delete(Root, recursive: true);
        }
        catch (IOException)
        {
            // A file still open somewhere: the temp folder keeps it.
        }
    }
}

public class HalfRecordTests
{
    [Test]
    public async Task Three_column_lines_have_no_bootloader()
    {
        // Arrange
        const string text = "kb_b left 0123456789ABCDEF\n";

        // Act
        var halves = HalfRecord.Parse(text);

        // Assert
        await Assert.That(halves).IsEquivalentTo(new[] { new HalfRecord("kb_b", "left", "0123456789ABCDEF", null) });
    }

    [Test]
    public async Task Four_column_lines_carry_the_bootloader_hil_recorded()
    {
        // Arrange
        const string text = "kb_b left 0123456789ABCDEF signed\nkb_b right FEDCBA9876543210 uf2\n";

        // Act
        var halves = HalfRecord.Parse(text);

        // Assert
        await Assert.That(halves.Select(h => h.Bootloader)).IsEquivalentTo(new string?[] { "signed", "uf2" });
    }

    [Test]
    public async Task Comments_blank_and_malformed_lines_are_skipped()
    {
        // Arrange
        const string text = "# halves\n\nkb_b middle 123\njust two\nkb_a right ABCDEF0123456789\n";

        // Act
        var halves = HalfRecord.Parse(text);

        // Assert
        await Assert.That(halves.Select(h => h.Serial)).IsEquivalentTo(new[] { "ABCDEF0123456789" });
    }

    [Test]
    public async Task Missing_file_means_no_halves()
    {
        // Arrange
        using var repo = new FakeRepo();

        // Act
        var halves = HalfRecord.Load(repo.Root);

        // Assert
        await Assert.That(halves).IsEmpty();
    }
}

public class FirmwarePackageTests
{
    [Test]
    public async Task Ble_packages_are_found_with_their_verify_result()
    {
        // Arrange
        using var repo = new FakeRepo()
            .File_("target/fw-nrf-left-ble-kb_b.zip", "zip")
            .File_("target/fw-nrf-left-ble-kb_b.verify.txt", "  ok    a\n  ok    b\n")
            .File_("target/fw-nrf-right-ble-kb_b.uf2", "uf2")
            .File_("target/fw-nrf-left-kb_b.uf2", "usb-only build, not BLE");

        // Act
        var packages = FirmwarePackage.Scan(repo.Target);

        // Assert
        await Assert.That(packages.Select(p => $"{p.Side} {p.Kind} {p.Verify?.Ok}"))
            .IsEquivalentTo(new[] { "left zip True", "right uf2 " });
    }

    [Test]
    public async Task A_fail_line_fails_the_verify_result()
    {
        // Arrange
        const string text = "  ok    package holds an application\n  FAIL  bootloader: ECDSA P-256 signature does not verify\n";

        // Act
        var result = VerifyResult.Parse(text);

        // Assert
        await Assert.That((result.Ok, result.Summary)).IsEqualTo((false, "FAIL  bootloader: ECDSA P-256 signature does not verify"));
    }

    [Test]
    public async Task Missing_target_folder_means_no_packages()
    {
        // Arrange
        var missing = Path.Combine(Path.GetTempPath(), $"emk-none-{Guid.NewGuid():N}");

        // Act
        var packages = FirmwarePackage.Scan(missing);

        // Assert
        await Assert.That(packages).IsEmpty();
    }
}

public class SigningKeyTests
{
    [Test]
    public async Task Missing_key_is_reported_absent()
    {
        // Arrange
        var path = Path.Combine(Path.GetTempPath(), $"emk-key-{Guid.NewGuid():N}.pem");

        // Act
        var key = SigningKeyStatus.Inspect(path);

        // Assert
        await Assert.That(key.Present).IsFalse();
    }

    [Test]
    public async Task Key_readable_by_others_is_flagged()
    {
        // Arrange
        var path = Path.Combine(Path.GetTempPath(), $"emk-key-{Guid.NewGuid():N}.pem");
        await File.WriteAllTextAsync(path, "not a key");
        if (!OperatingSystem.IsWindows()) File.SetUnixFileMode(path, UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.OtherRead);

        // Act
        var key = SigningKeyStatus.Inspect(path);

        // Assert
        File.Delete(path);
        await Assert.That((key.Present, key.OwnerOnly)).IsEqualTo((true, false));
    }
}

public class UsbBootloaderTests
{
    private static string FakeSys(params (string Serial, bool Storage)[] devices)
    {
        var root = Path.Combine(Path.GetTempPath(), $"emk-sys-{Guid.NewGuid():N}");
        for (var i = 0; i < devices.Length; i++)
        {
            var dev = Path.Combine(root, $"1-{i}");
            Directory.CreateDirectory(Path.Combine(dev, $"1-{i}:1.0"));
            File.WriteAllText(Path.Combine(dev, "idVendor"), "239a\n");
            File.WriteAllText(Path.Combine(dev, "serial"), devices[i].Serial + "\n");
            File.WriteAllText(Path.Combine(dev, $"1-{i}:1.0", "bInterfaceClass"), devices[i].Storage ? "08\n" : "02\n");
        }
        Directory.CreateDirectory(Path.Combine(root, "2-1"));
        File.WriteAllText(Path.Combine(root, "2-1", "idVendor"), "1d6b\n");
        return root;
    }

    [Test]
    public async Task Bootloaders_are_told_apart_by_their_drive()
    {
        // Arrange
        var sys = FakeSys(("0123456789ABCDEF", false), ("FEDCBA9876543210", true));

        // Act
        var found = UsbBootloader.Scan(sys);

        // Assert
        await Assert.That(found).IsEquivalentTo(new[] { new UsbBootloader("0123456789ABCDEF", "signed"), new UsbBootloader("FEDCBA9876543210", "uf2") });
    }

    [Test]
    public async Task A_signed_half_showing_a_uf2_drive_raises_the_alarm()
    {
        // Arrange
        var facts = new KeyboardFacts
        {
            Name = "kb_b",
            Halves = HalfRecord.Parse("kb_b left 0123456789ABCDEF signed\n"),
            Packages = [],
            OnUsb = UsbBootloader.Scan(FakeSys(("0123456789ABCDEF", true))),
        };

        // Act
        var alarm = facts.Alarm("left");

        // Assert
        await Assert.That(alarm).StartsWith("ALARM: kb_b left (0123456789ABCDEF) had the signed bootloader");
    }

    [Test]
    public async Task A_signed_half_in_its_signed_bootloader_is_no_alarm()
    {
        // Arrange
        var facts = new KeyboardFacts
        {
            Name = "kb_b",
            Halves = HalfRecord.Parse("kb_b left 0123456789ABCDEF signed\n"),
            Packages = [],
            OnUsb = UsbBootloader.Scan(FakeSys(("0123456789ABCDEF", false))),
        };

        // Act
        var alarm = facts.Alarm("left");

        // Assert
        await Assert.That(alarm).IsNull();
    }
}

public class KeyboardFactsTests
{
    private static KeyboardFacts Facts(string name) => new()
    {
        Name = name,
        Halves = [],
        Packages = [],
        OnUsb = [],
        AllHalves = HalfRecord.Parse("kb_a right ABCDEF0123456789\n"),
    };

    [Test]
    public async Task Another_keyboards_log_port_is_not_this_one()
    {
        // Arrange
        var facts = Facts("kb_b");

        // Act
        var (own, other) = (facts.IsOwnPort("/dev/serial/by-id/usb-erik_kb_b-if01"), facts.IsOwnPort("/dev/serial/by-id/usb-erik_kb_c-if01"));

        // Assert
        await Assert.That((own, other)).IsEqualTo((true, false));
    }

    [Test]
    public async Task A_serial_on_usb_is_named_by_its_record()
    {
        // Arrange
        var facts = Facts("kb_b");

        // Act
        var (known, unknown) = (facts.Owner("abcdef0123456789"), facts.Owner("0000111122223333"));

        // Assert
        await Assert.That((known, unknown)).IsEqualTo(("kb_a right", (string?)null));
    }
}

public class TamperRulesTests
{
    private const string Doc = """
        ## 5. Other
        text
        ## 6. Tamper alarm rules

        Once both halves are signed, the only way is a full erase.

        - **An unexpected request to pair again is an alarm.** Examples: the computer
          says so. **Don't pair it again.**
        - `./hil` refusing to flash because a package fails `dfu-verify.py` is not an alarm. Your build.

        ---

        ## 7. Verified
        """;

    [Test]
    public async Task Rules_are_read_from_section_six()
    {
        // Arrange
        var markdown = Doc;

        // Act
        var (intro, rules) = TamperRules.Parse(markdown);

        // Assert
        await Assert.That((intro, rules.Count, rules[0].Lead, rules[0].Alarm, rules[1].Alarm))
            .IsEqualTo(("Once both halves are signed, the only way is a full erase.", 2, "An unexpected request to pair again is an alarm.", true, false));
    }

    [Test]
    public async Task The_repo_doc_has_tamper_rules()
    {
        // Arrange
        var repo = RepoLocator.Find();

        // Act
        var (_, rules) = TamperRules.Load(repo);

        // Assert
        await Assert.That(rules.Count).IsGreaterThanOrEqualTo(3);
    }

    [Test]
    public async Task Missing_doc_means_no_rules()
    {
        // Arrange
        using var repo = new FakeRepo();

        // Act
        var (_, rules) = TamperRules.Load(repo.Root);

        // Assert
        await Assert.That(rules).IsEmpty();
    }
}

public class RepoLocatorTests
{
    [Test]
    public async Task Repo_is_found_by_walking_up_from_a_folder_inside_it()
    {
        // Arrange
        using var repo = new FakeRepo();
        var inside = Path.Combine(repo.Root, "emulator", "App", "bin");
        Directory.CreateDirectory(inside);

        // Act
        var found = RepoLocator.Find(inside, _ => null);

        // Assert
        await Assert.That(found).IsEqualTo(repo.Root);
    }

    [Test]
    public async Task Emk_repo_overrides_the_search()
    {
        // Arrange
        using var repo = new FakeRepo();

        // Act
        var found = RepoLocator.Find("/", name => name == "EMK_REPO" ? repo.Root : null);

        // Assert
        await Assert.That(found).IsEqualTo(repo.Root);
    }
}

/// <summary>Studio's keymap data against the firmware source.</summary>
public partial class KeymapTests
{
    [GeneratedRegex(@"combo\(""(\w+)"",\s*&\[([\d,\s]+)\],[^;]*?&\[([\d,\s]+)\]\)")]
    private static partial Regex ComboCall();

    [GeneratedRegex(@"COMBO_TIMEOUT_MS: u32 = (\d+)", RegexOptions.None, matchTimeoutMilliseconds: 1000)]
    private static partial Regex ComboTimeout();

    [GeneratedRegex(@"COMBO_PRIOR_IDLE_MS: u32 = (\d+)", RegexOptions.None, matchTimeoutMilliseconds: 1000)]
    private static partial Regex ComboPriorIdle();

    private static int[] Numbers(string s) =>
        s.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).Select(n => int.Parse(n, CultureInfo.InvariantCulture)).ToArray();

    [Test]
    public async Task Combos_match_the_keymap_source()
    {
        // Arrange
        var source = await File.ReadAllTextAsync(Path.Combine(RepoLocator.Find()!, "core", "src", "keymaps", "corne.rs"));
        var block = source[source.IndexOf("pub static COMBOS", StringComparison.Ordinal)..];
        block = block[..block.IndexOf("];", StringComparison.Ordinal)];

        // Act
        var fromSource = ComboCall().Matches(block).Select(m => $"{m.Groups[1].Value} {string.Join("+", Numbers(m.Groups[2].Value))} {string.Join(",", Numbers(m.Groups[3].Value))}").ToList();

        // Assert
        var hardcoded = Keymap.Combos.Select(c => $"{c.Name} {string.Join("+", c.Keys)} {string.Join(",", c.Layers)}").ToList();
        await Assert.That(hardcoded).IsEquivalentTo(fromSource);
    }

    [Test]
    public async Task Combo_timing_matches_the_keymap_source()
    {
        // Arrange
        var source = await File.ReadAllTextAsync(Path.Combine(RepoLocator.Find()!, "core", "src", "keymaps", "corne.rs"));

        // Act
        var timeout = int.Parse(ComboTimeout().Match(source).Groups[1].Value, CultureInfo.InvariantCulture);
        var idle = int.Parse(ComboPriorIdle().Match(source).Groups[1].Value, CultureInfo.InvariantCulture);

        // Assert: reset_bluetooth sets its own prior idle to 0 in the source
        await Assert.That((timeout, idle, Keymap.Combos.Single(c => c.Name == "reset_bluetooth").PriorIdleMs))
            .IsEqualTo((Keymap.ComboTimeoutMs, Keymap.ComboPriorIdleMs, 0));
    }

    [Test]
    public async Task Any_layer_can_be_read_without_activating_it()
    {
        // Arrange: Q (1) is ! on symbols, Profile ◀ (25) on nav

        // Act
        var labels = (Keymap.Label(0, 1), Keymap.Label(1, 1));

        // Assert
        await Assert.That(labels).IsEqualTo(("Q", "!"));
    }

    [Test]
    public async Task Tab_alt_is_a_hold_tap()
    {
        // Arrange
        const int layers = 4;

        // Act
        var holdTaps = Keymap.HoldTaps(layers);

        // Assert
        await Assert.That(holdTaps).Contains((0, 12, "Tab", "Alt"));
    }
}

/// <summary>The Studio, Flash and Security pages.</summary>
[NotInParallel("ui-session")]
public class ModePageTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    [Test]
    public async Task Studio_shows_the_chosen_layer_on_the_keyboard()
    {
        // Arrange
        var keyText = await Session.Dispatch(() =>
        {
            var studio = new StudioView(new EmulatedKeyboard(() => 0));

            // Act
            studio.Select(1, 1);
            return studio.KeyText;
        }, CancellationToken.None);

        // Assert
        await Assert.That(keyText).Contains("!");
    }

    [Test]
    public async Task Flash_without_files_explains_what_is_missing()
    {
        // Arrange
        using var repo = new FakeRepo();
        var sources = new DeviceSources { Repo = repo.Root, KeyPath = Path.Combine(repo.Root, "no.pem"), UsbRoot = repo.Root, LogPort = () => null };
        var selected = await Session.Dispatch(() =>
        {
            var flash = new FlashView(sources);

            // Act
            flash.Refresh();
            return flash.Selected;
        }, CancellationToken.None);

        // Assert
        await Assert.That(selected).IsNull();
    }

    [Test]
    public async Task Flash_picks_the_keyboard_from_hil_halves()
    {
        // Arrange
        using var repo = new FakeRepo().Halves("kb_a left 1234567890ABCDEF\nkb_a right ABCDEF0123456789\n");
        var sources = new DeviceSources { Repo = repo.Root, KeyPath = Path.Combine(repo.Root, "no.pem"), UsbRoot = repo.Root, LogPort = () => null };
        var (selected, lamps) = await Session.Dispatch(() =>
        {
            var flash = new FlashView(sources);

            // Act
            flash.Refresh();
            return (flash.Selected, flash.TopLamps.Select(l => l.Text).ToList());
        }, CancellationToken.None);

        // Assert
        await Assert.That((selected, string.Join(", ", lamps))).IsEqualTo(("kb_a", "Left not recorded, Right not recorded"));
    }

    [Test]
    public async Task Security_counts_what_is_left_to_do()
    {
        // Arrange: left signed, key present, debug port unknown
        using var repo = new FakeRepo().Halves("kb_b left 0123456789ABCDEF signed\nkb_b right FEDCBA9876543210 uf2\n");
        var key = Path.Combine(repo.Root, "emk-fw.pem");
        await File.WriteAllTextAsync(key, "");
        if (!OperatingSystem.IsWindows()) File.SetUnixFileMode(key, UnixFileMode.UserRead | UnixFileMode.UserWrite);
        var sources = new DeviceSources { Repo = repo.Root, KeyPath = key, UsbRoot = repo.Root, LogPort = () => null };
        var lamps = await Session.Dispatch(() =>
        {
            var security = new SecurityView(sources);

            // Act
            security.Refresh();
            return string.Join(", ", security.TopLamps.Select(l => l.Text));
        }, CancellationToken.None);

        // Assert: the right half and the debug port
        await Assert.That(lamps).IsEqualTo("2 items to do, No alarms");
    }

    [Test]
    public async Task A_locked_debug_port_seen_in_debug_counts_as_done()
    {
        // Arrange
        using var repo = new FakeRepo().Halves("kb_b left 0123456789ABCDEF signed\nkb_b right FEDCBA9876543210 signed\n");
        var key = Path.Combine(repo.Root, "emk-fw.pem");
        await File.WriteAllTextAsync(key, "");
        if (!OperatingSystem.IsWindows()) File.SetUnixFileMode(key, UnixFileMode.UserRead | UnixFileMode.UserWrite);
        var boot = new BootFacts("kb_b", "left", "0.11.0", "serial DFU (signed)", "locked", DateTime.Now);
        var sources = new DeviceSources { Repo = repo.Root, KeyPath = key, UsbRoot = repo.Root, LogPort = () => null, Boot = () => boot };
        var lamps = await Session.Dispatch(() =>
        {
            var security = new SecurityView(sources);

            // Act
            security.Refresh();
            return string.Join(", ", security.TopLamps.Select(l => l.Text));
        }, CancellationToken.None);

        // Assert
        await Assert.That(lamps).IsEqualTo("Nothing to do, No alarms");
    }

    [Test]
    public async Task Debug_remembers_the_boot_line_for_security()
    {
        // Arrange
        const string log = "  1100 boot: kb_b left ble (reset: pin)\r\n  1204 boot: bootloader 0.11.0, entry serial DFU (signed), debug port open\r\n";
        var boot = await Session.Dispatch(() =>
        {
            var view = new DebugView(_ => "base", 42, new HardwarePort(() => null));

            // Act
            view.Feed(log);
            var b = view.LastBoot;
            view.Dispose();
            return b;
        }, CancellationToken.None);

        // Assert
        await Assert.That((boot?.Keyboard, boot?.Half, boot?.DebugPort)).IsEqualTo(("kb_b", "left", "open"));
    }
}

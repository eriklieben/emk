using System.Text;

namespace Kb.HostProtocol.Tests;

public class DebugProtocolTests
{
    // Same lines as the tests in core/src/debug.rs.
    private const string StatusLine =
        "#S layer=2 caps=1 mods=22 out=ble profile=1 host=1 open=0 linked=1 stage=linked " +
        "bat=87,90 chg=1,0 wpm=42 power=active presses=1234 mute=0 trace=1 lit=1 radio=4";

    [Test]
    public async Task Log_lines_stay_log_lines()
    {
        // Arrange
        var line = "  12345 split: linked";

        // Act
        var parsed = DebugProtocol.Parse(line);

        // Assert
        await Assert.That(parsed).IsEqualTo(new DebugLog(line));
    }

    [Test]
    public async Task Hello_gives_version_name_half_and_build()
    {
        // Arrange
        var line = "#H 1 kb_a left ble";

        // Act
        var parsed = DebugProtocol.Parse(line);

        // Assert
        await Assert.That(parsed).IsEqualTo(new DebugHello(1, "kb_a", "left", "ble"));
    }

    [Test]
    public async Task Keys_are_a_bitmap_of_positions()
    {
        // Arrange
        var line = "#K 0000020000001001";

        // Act
        var keys = (DebugKeys)DebugProtocol.Parse(line);

        // Assert
        var held = Enumerable.Range(0, 42).Where(keys.IsHeld).ToArray();
        await Assert.That(held).IsEquivalentTo(new[] { 0, 12, 41 });
    }

    [Test]
    public async Task Report_splits_mods_and_keys()
    {
        // Arrange
        var line = "#R 0200040500000000";

        // Act
        var report = (DebugReport)DebugProtocol.Parse(line);

        // Assert
        await Assert.That(report.Mods).IsEqualTo((byte)0x02);
        await Assert.That(report.Keys).IsEquivalentTo(new byte[] { 0x04, 0x05, 0, 0, 0, 0 });
    }

    [Test]
    public async Task Status_reads_every_field()
    {
        // Arrange / Act
        var s = (DebugStatus)DebugProtocol.Parse(StatusLine);

        // Assert
        await Assert.That(s).IsEqualTo(new DebugStatus
        {
            Layer = 2,
            CapsWord = true,
            Mods = 0x22,
            Output = "ble",
            Profile = 1,
            HostConnected = true,
            ProfileOpen = false,
            Linked = true,
            Stage = "linked",
            BatteryLeft = 87,
            BatteryRight = 90,
            ChargingLeft = true,
            ChargingRight = false,
            Wpm = 42,
            Power = "active",
            Presses = 1234,
            Mute = false,
            Trace = true,
            Lit = true,
            RadioDbm = 4,
        });
    }

    [Test]
    public async Task Status_stage_dash_means_none()
    {
        // Arrange
        var line = "#S layer=0 stage=-";

        // Act
        var s = (DebugStatus)DebugProtocol.Parse(line);

        // Assert
        await Assert.That(s.Stage).IsEqualTo("");
    }

    [Test]
    public async Task Status_gives_the_radio_power()
    {
        // Arrange
        var line = StatusLine;

        // Act
        var s = (DebugStatus)DebugProtocol.Parse(line);

        // Assert
        await Assert.That(s.RadioDbm).IsEqualTo(4);
    }

    [Test]
    public async Task Status_from_an_older_firmware_has_no_radio_power()
    {
        // Arrange: before the radio field
        var line = StatusLine.Replace(" radio=4", "");

        // Act
        var s = (DebugStatus)DebugProtocol.Parse(line);

        // Assert
        await Assert.That(s.RadioDbm).IsNull();
    }

    [Test]
    public async Task Status_ignores_fields_it_does_not_know()
    {
        // Arrange: a newer firmware may add fields
        var line = "#S layer=3 future=7";

        // Act
        var s = (DebugStatus)DebugProtocol.Parse(line);

        // Assert
        await Assert.That(s.Layer).IsEqualTo(3);
    }

    [Test]
    public async Task Frame_decodes_512_bytes()
    {
        // Arrange
        var bytes = Enumerable.Range(0, 512).Select(i => (byte)(i * 7)).ToArray();
        var line = "#F " + Convert.ToBase64String(bytes);

        // Act
        var frame = (DebugFrame)DebugProtocol.Parse(line);

        // Assert
        await Assert.That(frame.Frame!).IsEquivalentTo(bytes);
    }

    [Test]
    public async Task Frame_dash_is_off()
    {
        // Arrange / Act
        var frame = DebugProtocol.Parse("#F -");

        // Assert
        await Assert.That(frame).IsEqualTo(new DebugFrame(null));
    }

    [Test]
    public async Task A_cut_off_frame_is_not_a_frame()
    {
        // Arrange: what a reader gets when it joins mid-line
        var line = "#F " + Convert.ToBase64String(new byte[300]);

        // Act
        var parsed = DebugProtocol.Parse(line);

        // Assert
        await Assert.That(parsed is DebugLog).IsTrue();
    }

    [Test]
    public async Task Malformed_keys_are_a_log_line()
    {
        // Arrange / Act
        var parsed = DebugProtocol.Parse("#K 12zz");

        // Assert
        await Assert.That(parsed is DebugLog).IsTrue();
    }

    [Test]
    public async Task Key_commands_name_the_position()
    {
        // Arrange / Act
        var commands = DebugProtocol.Press(12) + DebugProtocol.Release(12) + DebugProtocol.Tap(41);

        // Assert
        await Assert.That(commands).IsEqualTo("p12\nr12\nk41\n");
    }

    [Test]
    public async Task Splitter_joins_lines_across_reads()
    {
        // Arrange
        var splitter = new DebugLineSplitter();
        splitter.Feed("#K 00000000"u8);

        // Act
        var lines = splitter.Feed("00001001\r\n  42 boot\r\n"u8);

        // Assert
        await Assert.That(lines).IsEquivalentTo(new[] { "#K 0000000000001001", "  42 boot" });
    }

    [Test]
    public async Task Splitter_cuts_endless_lines()
    {
        // Arrange
        var splitter = new DebugLineSplitter();
        var junk = Encoding.ASCII.GetBytes(new string('x', DebugLineSplitter.MaxLine + 10));

        // Act
        var lines = splitter.Feed(junk);

        // Assert
        await Assert.That(lines.Count).IsEqualTo(1);
    }

    [Test]
    public async Task Splitter_reset_drops_the_partial_line()
    {
        // Arrange
        var splitter = new DebugLineSplitter();
        splitter.Feed("half a li"u8);
        splitter.Reset();

        // Act
        var lines = splitter.Feed("ne\n"u8);

        // Assert
        await Assert.That(lines).IsEquivalentTo(new[] { "ne" });
    }
}

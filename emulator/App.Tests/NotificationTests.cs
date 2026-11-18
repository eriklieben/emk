using Kb.HostProtocol;

namespace KbEmulator.Tests;

/// <summary>OS → left half (Raw HID) → split link → right OLED, through the Rust firmware library.</summary>
public class NotificationTests
{
    [Test]
    public async Task Notification_packets_are_shown_on_the_right_half()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        keyboard.UpdateDisplays();
        var before = keyboard.Displays[1].Version;
        var packets = NotificationPackets.Encode("Build passed on main");

        // Act
        var results = packets.Select(keyboard.RawHid).ToArray();
        keyboard.UpdateDisplays();

        // Assert
        await Assert.That(results[^1]).IsEqualTo(RawHidResult.Delivered);
        await Assert.That(keyboard.Displays[1].Version).IsNotEqualTo(before);
    }

    [Test]
    public async Task Long_notification_is_delivered_only_after_its_last_packet()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        var packets = NotificationPackets.Encode(new string('x', 60));

        // Act
        var results = packets.Select(keyboard.RawHid).ToArray();

        // Assert
        await Assert.That(results).IsEquivalentTo(new[] { RawHidResult.Pending, RawHidResult.Pending, RawHidResult.Delivered });
    }

    [Test]
    public async Task Notification_is_dropped_when_the_right_half_is_not_linked()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        keyboard.SetPeripheralConnected(false);

        // Act
        var result = keyboard.RawHid(NotificationPackets.Encode("Hi")[0]);

        // Assert
        await Assert.That(result).IsEqualTo(RawHidResult.NotLinked);
    }

    [Test]
    public async Task Clear_packet_clears_the_notification()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        keyboard.RawHid(NotificationPackets.Encode("Hi")[0]);

        // Act
        var result = keyboard.RawHid(NotificationPackets.Clear());

        // Assert
        await Assert.That(result).IsEqualTo(RawHidResult.Cleared);
    }

    [Test]
    public async Task Socket_server_passes_kbnotify_packets_on()
    {
        // Arrange
        var path = Path.Combine(Path.GetTempPath(), $"kb-emu-test-{Guid.NewGuid():N}.sock");
        var received = new TaskCompletionSource<byte[]>();
        using var server = new EmulatorSocketServer(p => received.TrySetResult(p), path);
        server.Start();
        var packet = NotificationPackets.Encode("Hi", id: 9)[0];

        // Act
        await using (var link = await EmulatorLink.ConnectAsync(path))
        {
            await link.SendAsync(packet);
        }

        // Assert
        var got = await received.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await Assert.That(got).IsEquivalentTo(packet);
    }
}

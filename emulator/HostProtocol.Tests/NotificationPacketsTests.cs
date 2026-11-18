namespace Kb.HostProtocol.Tests;

public class NotificationPacketsTests
{
    [Test]
    public async Task Short_text_is_one_packet_with_known_bytes()
    {
        // Arrange: same golden bytes as core/src/host.rs

        // Act
        var packets = NotificationPackets.Encode("Hello", id: 7);

        // Assert
        await Assert.That(packets.Count).IsEqualTo(1);
        await Assert.That(packets[0].Take(10).ToArray())
            .IsEquivalentTo(new byte[] { 0x01, 0x07, 0x00, 0x01, 0x05, (byte)'H', (byte)'e', (byte)'l', (byte)'l', (byte)'o' });
        await Assert.That(packets[0].Skip(10).All(b => b == 0)).IsTrue();
    }

    [Test]
    public async Task Long_text_is_split_into_27_byte_chunks()
    {
        // Arrange
        var text = new string('x', 60);

        // Act
        var packets = NotificationPackets.Encode(text, id: 1);

        // Assert
        var headers = packets.Select(p => (p[2], p[3], p[4])).ToArray();
        await Assert.That(headers).IsEquivalentTo(new (byte, byte, byte)[] { (0, 3, 27), (1, 3, 27), (2, 3, 6) });
    }

    [Test]
    public async Task Every_packet_is_32_bytes()
    {
        // Arrange
        var text = "Meeting in 5 minutes with the platform team";

        // Act
        var packets = NotificationPackets.Encode(text);

        // Assert
        await Assert.That(packets.All(p => p.Length == NotificationPackets.ReportSize)).IsTrue();
    }

    [Test]
    public async Task Text_over_128_bytes_is_cut_at_a_character_boundary()
    {
        // Arrange: 'é' is two bytes in UTF-8
        var text = new string('é', 100);

        // Act
        var packets = NotificationPackets.Encode(text, id: 1);

        // Assert
        var total = packets.Sum(p => p[4]);
        await Assert.That(total).IsEqualTo(128);
    }

    [Test]
    public async Task Clear_packet_has_clear_kind()
    {
        // Arrange

        // Act
        var packet = NotificationPackets.Clear();

        // Assert
        await Assert.That(packet[0]).IsEqualTo((byte)0x02);
        await Assert.That(packet.Length).IsEqualTo(32);
    }
}

using System.Text;

namespace Kb.HostProtocol;

/// <summary>
/// Raw HID packets for sending a notification to the keyboard. Same format
/// as <c>core/src/host.rs</c>; both test suites check the same bytes.
/// <code>
/// byte 0  kind         0x01 notification chunk, 0x02 clear
/// byte 1  message id   same for all chunks of one message
/// byte 2  chunk index
/// byte 3  chunk count  1..5
/// byte 4  data length  0..27
/// 5..32   UTF-8 text, zero padded
/// </code>
/// </summary>
public static class NotificationPackets
{
    public const int ReportSize = 32;
    public const int MaxText = 128;
    public const ushort UsagePage = 0xFF60;
    public const byte Usage = 0x61;

    private const int Header = 5;
    private const int ChunkData = ReportSize - Header;
    private const byte KindNotify = 0x01;
    private const byte KindClear = 0x02;

    private static int _nextId;

    /// <summary>Packets for <paramref name="text"/>; a new message id is picked when none is given.</summary>
    public static IReadOnlyList<byte[]> Encode(string text, byte? id = null)
    {
        var messageId = id ?? (byte)Interlocked.Increment(ref _nextId);
        var bytes = Truncate(Encoding.UTF8.GetBytes(text), MaxText);
        var count = Math.Max(1, (bytes.Length + ChunkData - 1) / ChunkData);
        var packets = new List<byte[]>(count);
        for (var index = 0; index < count; index++)
        {
            var start = index * ChunkData;
            var length = Math.Min(ChunkData, bytes.Length - start);
            var packet = new byte[ReportSize];
            packet[0] = KindNotify;
            packet[1] = messageId;
            packet[2] = (byte)index;
            packet[3] = (byte)count;
            packet[4] = (byte)length;
            bytes.AsSpan(start, length).CopyTo(packet.AsSpan(Header));
            packets.Add(packet);
        }
        return packets;
    }

    /// <summary>Packet that removes the notification from the screen.</summary>
    public static byte[] Clear()
    {
        var packet = new byte[ReportSize];
        packet[0] = KindClear;
        return packet;
    }

    /// <summary>Cut UTF-8 bytes to at most <paramref name="max"/> without splitting a character.</summary>
    private static byte[] Truncate(byte[] utf8, int max)
    {
        if (utf8.Length <= max) return utf8;
        var end = max;
        while (end > 0 && (utf8[end] & 0xC0) == 0x80) end--; // continuation byte: back up
        return utf8[..end];
    }
}

using Kb.Ssd1306;

namespace KbEmulator;

/// <summary>
/// The firmware's I²C writes to the two OLEDs, as <c>kb_emu_display</c> and <c>kb_mirror_display</c>
/// return them (each framed as [u16 length, little-endian][bytes]), fed into the SSD1306 models.
/// </summary>
internal static class DisplayFeed
{
    /// <summary>
    /// For each half: <paramref name="read"/> fills the buffer and returns the full length; a
    /// buffer too small is grown and read again.
    /// </summary>
    public static void Update(Ssd1306[] displays, ref byte[] buffer, Func<int, byte[], int> read)
    {
        for (var half = 0; half < displays.Length; half++)
        {
            var len = read(half, buffer);
            if (len > buffer.Length)
            {
                buffer = new byte[len];
                len = read(half, buffer);
            }
            Write(displays[half], buffer.AsSpan(0, len));
        }
    }

    private static void Write(Ssd1306 display, ReadOnlySpan<byte> writes)
    {
        while (writes.Length >= 2)
        {
            var n = writes[0] | writes[1] << 8;
            display.Write(writes.Slice(2, n));
            writes = writes[(2 + n)..];
        }
    }
}

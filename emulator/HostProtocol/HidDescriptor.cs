namespace Kb.HostProtocol;

/// <summary>The Raw HID collection as a report descriptor declares it.</summary>
/// <param name="ReportId">Report ID of its output report: 0 without report IDs (the USB
/// interface), 3 in the Bluetooth report map, next to the keyboard (1) and the mouse (2).</param>
/// <param name="OutputBytes">Bytes of the output report, without the report ID.</param>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Naming", "CA1711:Identifiers should not have incorrect suffix",
    Justification = "A HID collection, the USB HID term, not a .NET collection.")]
public sealed record RawHidCollection(byte ReportId, int OutputBytes);

/// <summary>
/// Reads HID report descriptors: where the Raw HID collection (usage page 0xFF60, usage 0x61)
/// is, and which report ID its output report has. The keyboard declares it on USB as an
/// interface of its own (no report IDs) and over Bluetooth inside the one report map, as
/// report ID 3; hidraw wants that ID as the first byte of every write.
/// </summary>
public static class HidDescriptor
{
    /// <summary>The Raw HID collection in <paramref name="descriptor"/>, or null.</summary>
    public static RawHidCollection? FindRawHid(ReadOnlySpan<byte> descriptor)
    {
        var parser = new Parser();
        var i = 0;
        while (ReadItem(descriptor, ref i) is { } item)
        {
            switch (item.Type)
            {
                case ItemType.Main when parser.Main(item.Tag, item.Data, out var found):
                    return found;
                case ItemType.Global:
                    parser.Global(item.Tag, item.Data);
                    break;
                case ItemType.Local:
                    parser.Local(item.Tag, item.Size, item.Data);
                    break;
            }
        }
        return null;
    }

    private enum ItemType
    {
        Main = 0,
        Global = 1,
        Local = 2,
        Reserved = 3,
        /// <summary>A long item: skipped.</summary>
        Long = 4,
    }

    /// <summary>One short item: its type, tag, data size in bytes and data (little-endian).</summary>
    private readonly record struct Item(ItemType Type, int Tag, int Size, uint Data);

    /// <summary>The item at <paramref name="i"/>, moving past it; null at the end or when it is cut off.</summary>
    private static Item? ReadItem(ReadOnlySpan<byte> descriptor, ref int i)
    {
        if (i >= descriptor.Length) return null;
        var prefix = descriptor[i];
        if (prefix == 0xFE) // long item: size, tag, data
        {
            if (i + 1 >= descriptor.Length) return null;
            i += 3 + descriptor[i + 1];
            return new Item(ItemType.Long, 0, 0, 0);
        }
        var size = (prefix & 0x03) == 3 ? 4 : prefix & 0x03;
        if (i + 1 + size > descriptor.Length) return null;
        var data = 0u;
        for (var k = 0; k < size; k++) data |= (uint)descriptor[i + 1 + k] << (8 * k);
        i += 1 + size;
        return new Item((ItemType)((prefix >> 2) & 0x03), prefix >> 4, size, data);
    }

    /// <summary>The descriptor's state while reading it: globals, local usages, collection depth.</summary>
    private sealed class Parser
    {
        private readonly Stack<(uint, uint, uint, byte)> _globals = new();
        private readonly List<uint> _usages = [];
        private uint _usagePage, _reportSize, _reportCount;
        private byte _reportId;
        private int _depth;
        private int _rawDepth = -1; // collection depth of the Raw HID collection while inside it
        private byte? _rawId;
        private uint _rawBits;

        /// <summary>A main item; true when the Raw HID collection ended (<paramref name="found"/> is the result).</summary>
        public bool Main(int tag, uint data, out RawHidCollection? found)
        {
            found = null;
            switch (tag)
            {
                case 0xA: // Collection
                    Collection(data);
                    break;
                case 0xC: // End Collection
                    _depth--;
                    if (_depth == _rawDepth)
                    {
                        found = _rawId is { } id ? new RawHidCollection(id, (int)(_rawBits / 8)) : null;
                        return true;
                    }
                    break;
                case 0x9 when _rawDepth >= 0: // Output, inside Raw HID
                    Output();
                    break;
            }
            _usages.Clear();
            return false;
        }

        private void Collection(uint data)
        {
            var usage = _usages.Count > 0 ? _usages[0] : 0;
            var page = usage > 0xFFFF ? usage >> 16 : _usagePage;
            var isRawHid = data == 1 && page == NotificationPackets.UsagePage && (usage & 0xFFFF) == NotificationPackets.Usage;
            if (_rawDepth < 0 && isRawHid) _rawDepth = _depth;
            _depth++;
        }

        private void Output()
        {
            if (_rawId is { } current && current != _reportId) return; // a second output report: the first one is ours
            _rawId = _reportId;
            _rawBits += _reportSize * _reportCount;
        }

        public void Global(int tag, uint data)
        {
            switch (tag)
            {
                case 0x0: _usagePage = data; break;
                case 0x7: _reportSize = data; break;
                case 0x8: _reportId = (byte)data; break;
                case 0x9: _reportCount = data; break;
                case 0xA: _globals.Push((_usagePage, _reportSize, _reportCount, _reportId)); break;
                case 0xB when _globals.Count > 0:
                    (_usagePage, _reportSize, _reportCount, _reportId) = _globals.Pop();
                    break;
            }
        }

        public void Local(int tag, int size, uint data)
        {
            if (tag == 0x0) _usages.Add(size == 4 ? data : data & 0xFFFF);
        }
    }
}

namespace Kb.Ssd1306;

/// <summary>
/// Model of the SSD1306 OLED controller as it is driven over I²C.
/// Feed it the bytes of each I²C write (without the address byte) and read
/// back what the panel shows.
///
/// Implemented: the command set needed for 128×32 / 128×64 modules
/// (addressing modes, column/page windows, start line, offset, segment and
/// COM remap, contrast, invert, entire-display-on, display on/off, charge
/// pump). Scrolling commands are parsed (so their arguments are consumed)
/// but not animated.
/// </summary>
public sealed class Ssd1306
{
    public const byte DefaultAddress = 0x3C;
    public const int Columns = 128;
    public const int Pages = 8;

    public enum AddressingMode { Horizontal = 0, Vertical = 1, Page = 2 }

    private readonly byte[] _ram = new byte[Columns * Pages];
    private readonly List<byte> _command = new();

    private int _column, _page;
    private int _columnStart, _columnEnd = Columns - 1;
    private int _pageStart, _pageEnd = Pages - 1;
    private int _pageModeColumn;

    public Ssd1306(int panelHeight = 32)
    {
        if (panelHeight is not (32 or 64)) throw new ArgumentOutOfRangeException(nameof(panelHeight));
        PanelHeight = panelHeight;
    }

    /// <summary>Rows of glass on the module (32 for the Corne's OLEDs).</summary>
    public int PanelHeight { get; }

    /// <summary>
    /// Most 128×32 modules mount the glass rotated 180° relative to the
    /// controller's SEG0/COM0, which is why drivers send A1 and C8.
    /// </summary>
    public bool MountedRotated180 { get; set; } = true;

    /// <summary>Modules powered from 3.3 V only light up when the charge pump is enabled (8D 14).</summary>
    public bool RequiresChargePump { get; set; } = true;

    public bool DisplayOn { get; private set; }
    public bool ChargePumpEnabled { get; private set; }
    public bool EntireDisplayOn { get; private set; }
    public bool Inverted { get; private set; }
    public bool SegmentRemap { get; private set; }
    public bool ComScanReversed { get; private set; }
    public byte Contrast { get; private set; } = 0x7F;
    public int Multiplex { get; private set; } = 64;
    public int StartLine { get; private set; }
    public int DisplayOffset { get; private set; }
    public AddressingMode Mode { get; private set; } = AddressingMode.Page;

    /// <summary>Increments on every change that can alter the picture.</summary>
    public int Version { get; private set; }

    public ReadOnlySpan<byte> Ram => _ram;

    /// <summary>Whether the panel emits light at all.</summary>
    public bool IsLit => Powered && DisplayOn && (ChargePumpEnabled || !RequiresChargePump);

    /// <summary>
    /// Supply voltage. The nice!nano cuts VCC to the OLEDs in deep sleep; the
    /// controller then loses its settings and RAM, and ignores the bus.
    /// </summary>
    public bool Powered
    {
        get => _powered;
        set
        {
            if (value == _powered) return;
            _powered = value;
            if (!value) Reset();
            Version++;
        }
    }

    private bool _powered = true;

    /// <summary>Power-on defaults (datasheet reset values) and cleared RAM.</summary>
    private void Reset()
    {
        Array.Clear(_ram);
        _command.Clear();
        _column = _page = _columnStart = _pageStart = _pageModeColumn = 0;
        _columnEnd = Columns - 1;
        _pageEnd = Pages - 1;
        DisplayOn = ChargePumpEnabled = EntireDisplayOn = Inverted = SegmentRemap = ComScanReversed = false;
        Contrast = 0x7F;
        Multiplex = 64;
        StartLine = DisplayOffset = 0;
        Mode = AddressingMode.Page;
    }

    /// <summary>Handle one I²C write transaction addressed to the controller.</summary>
    public void Write(ReadOnlySpan<byte> transaction)
    {
        if (!Powered) return;
        var i = 0;
        while (i < transaction.Length)
        {
            var control = transaction[i++];
            var continuation = (control & 0x80) != 0; // Co: one byte, then another control byte
            var isData = (control & 0x40) != 0;       // D/C#
            if (continuation)
            {
                if (i < transaction.Length) Accept(transaction[i++], isData);
                continue;
            }
            for (; i < transaction.Length; i++) Accept(transaction[i], isData);
        }
    }

    /// <summary>
    /// Pixel as the viewer sees it, (0,0) top-left, for x &lt; 128 and y &lt; <see cref="PanelHeight"/>.
    /// </summary>
    public bool GetPixel(int x, int y)
    {
        if ((uint)x >= Columns || (uint)y >= PanelHeight || !IsLit) return false;
        if (EntireDisplayOn) return true;

        if (MountedRotated180)
        {
            x = Columns - 1 - x;
            y = PanelHeight - 1 - y;
        }
        if (y >= Multiplex) return false; // row not driven

        var com = ComScanReversed ? Multiplex - 1 - y : y;
        var ramRow = (com + StartLine + DisplayOffset) % 64;
        var ramColumn = SegmentRemap ? Columns - 1 - x : x;
        var on = (_ram[(ramRow / 8) * Columns + ramColumn] >> (ramRow % 8) & 1) != 0;
        return on ^ Inverted;
    }

    private void Accept(byte value, bool isData)
    {
        if (isData) WriteData(value);
        else AcceptCommandByte(value);
    }

    private void WriteData(byte value)
    {
        _ram[_page * Columns + _column] = value;
        Version++;
        switch (Mode)
        {
            case AddressingMode.Horizontal:
                if (++_column > _columnEnd)
                {
                    _column = _columnStart;
                    if (++_page > _pageEnd) _page = _pageStart;
                }
                break;
            case AddressingMode.Vertical:
                if (++_page > _pageEnd)
                {
                    _page = _pageStart;
                    if (++_column > _columnEnd) _column = _columnStart;
                }
                break;
            default:
                if (++_column >= Columns) _column = _pageModeColumn;
                break;
        }
    }

    private void AcceptCommandByte(byte value)
    {
        _command.Add(value);
        if (_command.Count < 1 + ArgumentCount(_command[0])) return;
        Execute(_command[0], _command.Skip(1).ToArray());
        _command.Clear();
    }

    private static int ArgumentCount(byte command) => command switch
    {
        0x20 or 0x81 or 0x8D or 0xA8 or 0xD3 or 0xD5 or 0xD9 or 0xDA or 0xDB => 1,
        0x21 or 0x22 or 0xA3 => 2,
        0x29 or 0x2A => 5,
        0x26 or 0x27 => 6,
        _ => 0,
    };

    private void Execute(byte cmd, byte[] args)
    {
        switch (cmd)
        {
            case <= 0x0F:
                _pageModeColumn = (_pageModeColumn & 0xF0) | cmd;
                _column = _pageModeColumn;
                break;
            case <= 0x1F:
                _pageModeColumn = (_pageModeColumn & 0x0F) | ((cmd & 0x07) << 4);
                _column = _pageModeColumn;
                break;
            case 0x20:
                if ((args[0] & 0x03) != 3) Mode = (AddressingMode)(args[0] & 0x03);
                break;
            case 0x21:
                _columnStart = args[0] & 0x7F;
                _columnEnd = args[1] & 0x7F;
                _column = _columnStart;
                break;
            case 0x22:
                _pageStart = args[0] & 0x07;
                _pageEnd = args[1] & 0x07;
                _page = _pageStart;
                break;
            case >= 0x40 and <= 0x7F:
                StartLine = cmd & 0x3F;
                break;
            case 0x81:
                Contrast = args[0];
                break;
            case 0x8D:
                ChargePumpEnabled = (args[0] & 0x04) != 0;
                break;
            case 0xA0 or 0xA1:
                SegmentRemap = cmd == 0xA1;
                break;
            case 0xA4 or 0xA5:
                EntireDisplayOn = cmd == 0xA5;
                break;
            case 0xA6 or 0xA7:
                Inverted = cmd == 0xA7;
                break;
            case 0xA8:
                var mux = (args[0] & 0x3F) + 1;
                if (mux >= 16) Multiplex = mux;
                break;
            case 0xAE or 0xAF:
                DisplayOn = cmd == 0xAF;
                break;
            case >= 0xB0 and <= 0xB7:
                _page = cmd & 0x07;
                break;
            case 0xC0 or 0xC8:
                ComScanReversed = cmd == 0xC8;
                break;
            case 0xD3:
                DisplayOffset = args[0] & 0x3F;
                break;
            default:
                return; // timing, COM pins, scrolling, NOP: no visible effect in this model
        }
        Version++;
    }
}

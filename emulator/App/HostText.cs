using System.Text;

namespace KbEmulator;

/// <summary>
/// Turns HID reports into the text a host with a US layout would type.
/// Shortcuts and non-printing keys appear as tokens like <c>[⌃C]</c> or <c>[F20]</c>.
/// </summary>
public sealed class HostText
{
    private const byte Ctrl = 0x01 | 0x10, Shift = 0x02 | 0x20, Alt = 0x04, Gui = 0x08 | 0x80;

    private readonly StringBuilder _text = new();
    private readonly Func<int, string> _usageName;

    public HostText(Func<int, string> usageName) => _usageName = usageName;

    public string Text => _text.ToString();

    public void Clear() => _text.Clear();

    /// <summary>Apply one report; only newly pressed keys produce text.</summary>
    public void Apply(HidReport previous, HidReport next)
    {
        for (var usage = 0; usage < HidReport.FirstModifier; usage++)
        {
            if (next.Contains(usage) && !previous.Contains(usage)) Type(usage, next.Mods);
        }
    }

    private void Type(int usage, byte mods)
    {
        var shift = (mods & Shift) != 0;
        if ((mods & (Ctrl | Gui | Alt)) != 0)
        {
            TypeShortcut(usage, mods, shift);
            return;
        }
        switch (usage)
        {
            case 0x28: _text.Append('\n'); return;          // Enter
            case 0x2B: _text.Append('\t'); return;          // Tab
            case 0x2A: Backspace(); return;
        }
        if (Printable(usage, shift) is { } printable) _text.Append(printable);
        else _text.Append('[').Append(_usageName(usage)).Append(']');
    }

    /// <summary>A shortcut, as <c>[⌃⇧C]</c>.</summary>
    private void TypeShortcut(int usage, byte mods, bool shift)
    {
        var prefix = new StringBuilder();
        if ((mods & Ctrl) != 0) prefix.Append('⌃');
        if (shift) prefix.Append('⇧');
        if ((mods & Alt) != 0) prefix.Append('⌥');
        if ((mods & Gui) != 0) prefix.Append('◆');
        _text.Append('[').Append(prefix).Append(Printable(usage, false)?.ToUpperInvariant() ?? _usageName(usage)).Append(']');
    }

    /// <summary>Backspace takes back a typed character; after a shortcut (or nothing) it shows as [⌫].</summary>
    private void Backspace()
    {
        if (_text.Length > 0 && _text[^1] != ']') _text.Length--;
        else _text.Append("[⌫]");
    }

    /// <summary>Character for a usage on a US layout, or null for non-printing keys.</summary>
    public static string? Printable(int usage, bool shift)
    {
        if (usage is >= 0x04 and <= 0x1D)
        {
            var c = (char)('a' + usage - 0x04);
            return (shift ? char.ToUpperInvariant(c) : c).ToString();
        }
        var pair = usage switch
        {
            0x1E => "1!", 0x1F => "2@", 0x20 => "3#", 0x21 => "4$", 0x22 => "5%",
            0x23 => "6^", 0x24 => "7&", 0x25 => "8*", 0x26 => "9(", 0x27 => "0)",
            0x2C => "  ", 0x2D => "-_", 0x2E => "=+", 0x2F => "[{", 0x30 => "]}", 0x31 => "\\|",
            0x33 => ";:", 0x34 => "'\"", 0x35 => "`~", 0x36 => ",<", 0x37 => ".>", 0x38 => "/?",
            _ => null,
        };
        return pair?[shift ? 1 : 0].ToString();
    }
}

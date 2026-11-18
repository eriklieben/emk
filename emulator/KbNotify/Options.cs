namespace KbNotify;

/// <summary>The command line; anything that is not an option is a word of the text.</summary>
internal sealed record Options(bool UseEmulator, string? Device, bool Clear, bool List, bool Help, IReadOnlyList<string> Words)
{
    public const string Usage = """
        kbnotify [--emulator | --device <path>] <text...>
        kbnotify [--emulator | --device <path>] --clear
        kbnotify --list

        Shows <text> on the right half's OLED (up to 128 bytes of UTF-8; control
        characters are left out). Without --emulator/--device: the emulator if it is
        running, else the keyboard's Raw HID collection (usage page 0xFF60, usage 0x61),
        over USB if it is plugged in, else over Bluetooth.

        --device  Linux: /dev/hidrawN. Windows: a HID device path as --list shows it.
        --list    The keyboard's Raw HID devices found, in the order they are tried.
        """;

    public static Options Parse(IEnumerable<string> args)
    {
        var options = new Options(false, null, false, false, false, []);
        var words = new List<string>();
        var queue = new Queue<string>(args);
        while (queue.TryDequeue(out var arg))
        {
            switch (arg)
            {
                case "--emulator": options = options with { UseEmulator = true }; break;
                case "--device" when queue.Count > 0: options = options with { Device = queue.Dequeue() }; break;
                case "--clear": options = options with { Clear = true }; break;
                case "--list": options = options with { List = true }; break;
                case "-h" or "--help": return options with { Help = true };
                default: words.Add(arg); break;
            }
        }
        return options with { Words = words };
    }
}

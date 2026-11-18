using System.Diagnostics.CodeAnalysis;

namespace KbEmulator;

/// <summary>
/// The keymap built into the firmware (<c>core/src/keymaps/corne.rs</c>), read
/// for Studio: legends of any layer through the native lib, and the combos.
/// </summary>
public static class Keymap
{
    public const int KeyCount = 42;

    private static readonly string[] ThumbNames = ["outer", "middle", "inner"];

    /// <summary>A combo: keys pressed together within <see cref="TimeoutMs"/>.</summary>
    /// <param name="PriorIdleMs">Ignored when a key came less than this before (0: always).</param>
    public sealed record Combo(string Name, int[] Keys, string Does, int[] Layers, int TimeoutMs, int PriorIdleMs);

    public const int ComboTimeoutMs = 50;
    public const int ComboPriorIdleMs = 150;

    /// <summary>
    /// The combos of corne.rs (<c>COMBOS</c>). The native lib doesn't expose
    /// them; a test checks this list against the source.
    /// </summary>
    public static IReadOnlyList<Combo> Combos { get; } =
    [
        new("cut", [25, 26], "Ctrl+X", [0], ComboTimeoutMs, ComboPriorIdleMs),
        new("copy", [26, 27], "Ctrl+C", [0], ComboTimeoutMs, ComboPriorIdleMs),
        new("paste", [27, 28], "Ctrl+V", [0], ComboTimeoutMs, ComboPriorIdleMs),
        new("caps_word", [5, 6], "Caps word", [0], ComboTimeoutMs, ComboPriorIdleMs),
        new("escape", [19, 20], "Esc", [0], ComboTimeoutMs, ComboPriorIdleMs),
        new("reset_bluetooth", [25, 26], "Profile clear", [2], ComboTimeoutMs, 0),
    ];

    /// <summary>What layers are for (corne.rs comments); empty for an unknown name.</summary>
    public static string Describe(string layer) => layer switch
    {
        "default" => "base",
        "symbols" => "hold symbols",
        "nav" => "hold nav",
        "wm" => "symbols + nav",
        _ => "",
    };

    /// <summary>Legend of <paramref name="pos"/> with <paramref name="layer"/> active ("tap\nhold" for hold-taps).</summary>
    [SuppressMessage("Security", "S6640:Using unsafe code blocks is security-sensitive",
        Justification = "kb_mirror_label writes into a buffer Native.ReadString pins for the call.")]
    public static unsafe string Label(int layer, int pos) =>
        Native.ReadString((buf, cap) => MirrorNative.kb_mirror_label((byte)layer, (byte)pos, (byte*)buf, cap));

    /// <summary>The layers' names, from the emulator (stops at the first unknown one).</summary>
    public static IReadOnlyList<string> LayerNames(Func<int, string> layerName)
    {
        var names = new List<string>();
        for (var i = 0; i < 16; i++)
        {
            var name = layerName(i);
            if (name is "" or "?") break;
            names.Add(name);
        }
        return names;
    }

    /// <summary>
    /// Whether a key falls through to the base layer. The label API resolves
    /// transparency itself, so a key that shows its base legend counts.
    /// </summary>
    public static bool FallsThrough(int layer, int pos) => layer != 0 && Label(layer, pos) == Label(0, pos);

    /// <summary>Hold-taps: keys with a tap and a hold legend, per layer (position, tap, hold).</summary>
    public static IReadOnlyList<(int Layer, int Pos, string Tap, string Hold)> HoldTaps(int layers)
    {
        var list = new List<(int, int, string, string)>();
        for (var layer = 0; layer < layers; layer++)
        {
            for (var pos = 0; pos < KeyCount; pos++)
            {
                if (layer > 0 && FallsThrough(layer, pos)) continue;
                var parts = Label(layer, pos).Split('\n');
                if (parts.Length == 2) list.Add((layer, pos, parts[0], parts[1]));
            }
        }
        return list;
    }

    /// <summary>A thumb key (36..41): which half, and outer, middle or inner.</summary>
    internal static (bool Left, string Name) Thumb(int pos)
    {
        var left = pos <= 38;
        return (left, ThumbNames[left ? pos - 36 : 41 - pos]);
    }

    /// <summary>Where a key is: "left, row 2, column 3" or "right thumb, inner".</summary>
    public static string Where(int pos)
    {
        if (pos >= 36)
        {
            var (left, name) = Thumb(pos);
            return $"{(left ? "left" : "right")} thumb, {name}";
        }
        var row = pos / 12 + 1;
        var col = pos % 12;
        return col < 6 ? $"left, row {row}, column {col + 1}" : $"right, row {row}, column {12 - col}";
    }
}

using Avalonia.Input;

namespace KbEmulator;

/// <summary>
/// Which key on your computer's keyboard plays which Corne position. It is
/// positional (same as <c>sim/src/layout.rs</c>): Tab/Caps/Shift are the
/// outer column, the thumb keys sit around the space bar.
/// </summary>
public static class PhysicalLayout
{
    private static readonly Dictionary<PhysicalKey, int> Positions = new()
    {
        [PhysicalKey.Tab] = 0, [PhysicalKey.Q] = 1, [PhysicalKey.W] = 2, [PhysicalKey.E] = 3,
        [PhysicalKey.R] = 4, [PhysicalKey.T] = 5, [PhysicalKey.Y] = 6, [PhysicalKey.U] = 7,
        [PhysicalKey.I] = 8, [PhysicalKey.O] = 9, [PhysicalKey.P] = 10, [PhysicalKey.BracketLeft] = 11,

        [PhysicalKey.CapsLock] = 12, [PhysicalKey.A] = 13, [PhysicalKey.S] = 14, [PhysicalKey.D] = 15,
        [PhysicalKey.F] = 16, [PhysicalKey.G] = 17, [PhysicalKey.H] = 18, [PhysicalKey.J] = 19,
        [PhysicalKey.K] = 20, [PhysicalKey.L] = 21, [PhysicalKey.Semicolon] = 22, [PhysicalKey.Quote] = 23,

        [PhysicalKey.ShiftLeft] = 24, [PhysicalKey.Z] = 25, [PhysicalKey.X] = 26, [PhysicalKey.C] = 27,
        [PhysicalKey.V] = 28, [PhysicalKey.B] = 29, [PhysicalKey.N] = 30, [PhysicalKey.M] = 31,
        [PhysicalKey.Comma] = 32, [PhysicalKey.Period] = 33, [PhysicalKey.Slash] = 34, [PhysicalKey.ShiftRight] = 35,

        [PhysicalKey.MetaLeft] = 36, [PhysicalKey.AltLeft] = 37, [PhysicalKey.Space] = 38,
        [PhysicalKey.AltRight] = 39, [PhysicalKey.ContextMenu] = 40, [PhysicalKey.MetaRight] = 40,
        [PhysicalKey.ControlRight] = 41,
    };

    public static int? Position(PhysicalKey key) => Positions.TryGetValue(key, out var pos) ? pos : null;
}

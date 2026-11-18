using System.Diagnostics.CodeAnalysis;
using System.Runtime.InteropServices;
using System.Text;

namespace KbEmulator;

/// <summary>P/Invoke bindings for Studio in <c>libkb_emu.so</c> (docs/studio-protocol.md, "Emulator C ABI").</summary>
[SuppressMessage("Security", "S6640:Using unsafe code blocks is security-sensitive",
    Justification = "C ABI of libkb_emu.so: UTF-8 strings in and out as pinned byte buffers.")]
internal static unsafe class StudioNative
{
    private const string Lib = "kb_emu";

    [DllImport(Lib)] public static extern byte kb_emu_studio_layer_count();
    [DllImport(Lib)] public static extern nuint kb_emu_studio_binding(EmulatorHandle e, byte layer, byte pos, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern nuint kb_emu_studio_default_binding(EmulatorHandle e, byte layer, byte pos, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern int kb_emu_studio_check(EmulatorHandle e, byte layer, byte pos, byte* enc, nuint len);
    [DllImport(Lib)] public static extern int kb_emu_studio_set_storage_path(EmulatorHandle e, byte* path, nuint len);
    [DllImport(Lib)] public static extern uint kb_emu_studio_unlock_remaining_ms(EmulatorHandle e, uint nowMs);
    [DllImport(Lib)] public static extern uint kb_emu_studio_generation(EmulatorHandle e);

    [DllImport(Lib)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static extern bool kb_emu_studio_dirty(EmulatorHandle e);

    [DllImport(Lib)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static extern bool kb_emu_studio_command(EmulatorHandle e, byte* line, nuint len, uint nowMs);

    [DllImport(Lib)] public static extern nuint kb_emu_studio_next_line(EmulatorHandle e, uint nowMs, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern uint kb_emu_studio_line_max();
    [DllImport(Lib)] public static extern long kb_emu_studio_canonical(byte* enc, nuint len, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern nuint kb_emu_studio_label(byte* enc, nuint len, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern nuint kb_emu_studio_key_name(uint index, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern byte kb_emu_studio_combo_max();
    [DllImport(Lib)] public static extern byte kb_emu_studio_combo_count(EmulatorHandle e);
    [DllImport(Lib)] public static extern nuint kb_emu_studio_combo(EmulatorHandle e, byte index, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern byte kb_emu_studio_default_combo_count(EmulatorHandle e);
    [DllImport(Lib)] public static extern nuint kb_emu_studio_default_combo(EmulatorHandle e, byte index, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern int kb_emu_studio_combo_check(EmulatorHandle e, byte index, byte* text, nuint len);
    [DllImport(Lib)] public static extern int kb_emu_studio_combo_check_remove(EmulatorHandle e, byte index);
    [DllImport(Lib)] public static extern nuint kb_emu_studio_system_name(uint index, byte* buf, nuint cap);

    /// <summary>Call with a UTF-8 string in.</summary>
    public static T With<T>(string text, Func<IntPtr, nuint, T> call)
    {
        var bytes = Encoding.UTF8.GetBytes(text);
        fixed (byte* p = bytes) return call((IntPtr)p, (nuint)bytes.Length);
    }
}

/// <summary>Studio's encodings, through the firmware's own code (no emulator needed).</summary>
[SuppressMessage("Security", "S6640:Using unsafe code blocks is security-sensitive",
    Justification = "Calls the C ABI with pinned byte buffers (StudioNative).")]
public static unsafe class StudioCodec
{
    /// <summary>The canonical form of an encoding; null when it isn't an action.</summary>
    public static string? Canonical(string encoding) => StudioNative.With(encoding, (enc, len) =>
    {
        var buf = new byte[128];
        fixed (byte* p = buf)
        {
            var n = StudioNative.kb_emu_studio_canonical((byte*)enc, len, p, (nuint)buf.Length);
            if (n < 0) return null;
            if (n > buf.Length)
            {
                buf = new byte[n];
                fixed (byte* q = buf) StudioNative.kb_emu_studio_canonical((byte*)enc, len, q, (nuint)buf.Length);
            }
            return Encoding.UTF8.GetString(buf, 0, (int)n);
        }
    });

    /// <summary>The legend the firmware draws for an encoding ("" when invalid).</summary>
    public static string Label(string encoding) => StudioNative.With(encoding, (enc, len) =>
        Native.ReadString((buf, cap) => StudioNative.kb_emu_studio_label((byte*)enc, len, (byte*)buf, cap)));

    /// <summary>Every key name <c>kp:</c> takes (ZMK names, then the shifted aliases).</summary>
    public static IReadOnlyList<string> KeyNames { get; } = List(i =>
        Native.ReadString((buf, cap) => StudioNative.kb_emu_studio_key_name(i, (byte*)buf, cap)));

    /// <summary>Every system action <c>sys:</c> takes.</summary>
    public static IReadOnlyList<string> SystemNames { get; } = List(i =>
        Native.ReadString((buf, cap) => StudioNative.kb_emu_studio_system_name(i, (byte*)buf, cap)));

    /// <summary>A bound on a list's length, should the library never answer with an empty name.</summary>
    private const uint MaxListLength = 1000;

    private static List<string> List(Func<uint, string> item)
    {
        var list = new List<string>();
        for (uint i = 0; i < MaxListLength; i++)
        {
            var name = item(i);
            if (name.Length == 0) break;
            list.Add(name);
        }
        return list;
    }
}

using System.Diagnostics.CodeAnalysis;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace KbEmulator;

/// <summary>An emulator instance from <c>kb_emu_new</c>; released with <c>kb_emu_free</c>, exactly once.</summary>
internal sealed class EmulatorHandle() : SafeHandleZeroOrMinusOneIsInvalid(ownsHandle: true)
{
    protected override bool ReleaseHandle()
    {
        Native.kb_emu_free(handle);
        return true;
    }
}

/// <summary>
/// P/Invoke bindings for <c>libkb_emu.so</c> (the Rust crate <c>emu/</c>). The C ABI takes UTF-8
/// buffers as pointer + capacity and returns events with a fixed-size key array, so the bindings
/// are <c>unsafe</c>; every buffer passed is pinned (<c>fixed</c>) for the duration of the call only.
/// </summary>
[SuppressMessage("Security", "S6640:Using unsafe code blocks is security-sensitive",
    Justification = "C ABI of libkb_emu.so: pinned byte buffers and a fixed-size array in KbEvent.")]
internal static unsafe class Native
{
    private const string Lib = "kb_emu";

    [StructLayout(LayoutKind.Sequential)]
    public struct KbEvent
    {
        public byte Kind;
        public byte Value;
        public byte Mods;
        public byte Reserved;
        public fixed byte Keys[32];
    }

    public const byte KindNone = 0, KindReport = 1, KindLayer = 2, KindCapsWord = 3, KindSystem = 4, KindPower = 5, KindPairing = 6, KindMouse = 7;

    /// <summary>Where the mouse keys took the pointer (<c>KbMouse</c> in Rust).</summary>
    [StructLayout(LayoutKind.Sequential)]
    public struct KbMouse
    {
        public int X, Y, Wheel, Pan;
        public byte Buttons, Pointer, Moving, Hires;
    }

    [DllImport(Lib)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static extern bool kb_emu_mouse_state(EmulatorHandle e, out KbMouse state);

    [DllImport(Lib)] public static extern void kb_emu_mouse_reset_position(EmulatorHandle e);

    [DllImport(Lib)]
    public static extern void kb_emu_mouse_set_resolution(EmulatorHandle e, [MarshalAs(UnmanagedType.U1)] bool wheelHires, [MarshalAs(UnmanagedType.U1)] bool panHires);

    [DllImport(Lib)] public static extern EmulatorHandle kb_emu_new();
    [DllImport(Lib)] public static extern void kb_emu_free(IntPtr e);
    [DllImport(Lib)] public static extern uint kb_emu_key_count();
    [DllImport(Lib)] public static extern void kb_emu_press(EmulatorHandle e, byte pos, uint nowMs);
    [DllImport(Lib)] public static extern void kb_emu_release(EmulatorHandle e, byte pos, uint nowMs);
    [DllImport(Lib)] public static extern void kb_emu_tick(EmulatorHandle e, uint nowMs);
    [DllImport(Lib)] public static extern long kb_emu_next_deadline(EmulatorHandle e);

    [DllImport(Lib)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static extern bool kb_emu_poll(EmulatorHandle e, uint nowMs, out KbEvent ev);

    [DllImport(Lib)] public static extern nuint kb_emu_label(EmulatorHandle e, byte pos, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern nuint kb_emu_layer_name(EmulatorHandle e, byte layer, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern nuint kb_emu_usage_name(byte usage, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern void kb_emu_set_battery(EmulatorHandle e, byte half, byte percent);
    [DllImport(Lib)] public static extern void kb_emu_replay_boot(EmulatorHandle e);
    [DllImport(Lib)] public static extern void kb_emu_set_half_power(EmulatorHandle e, byte half, [MarshalAs(UnmanagedType.U1)] bool on, uint now);
    [DllImport(Lib)] public static extern void kb_emu_reset_half(EmulatorHandle e, byte half, uint now);
    [DllImport(Lib)] public static extern void kb_emu_set_charging(EmulatorHandle e, byte half, [MarshalAs(UnmanagedType.U1)] bool charging);

    [DllImport(Lib)]
    public static extern void kb_emu_set_peripheral_connected(EmulatorHandle e, [MarshalAs(UnmanagedType.U1)] bool connected);

    [DllImport(Lib)] public static extern void kb_emu_set_screen_style(EmulatorHandle e, byte style);

    [DllImport(Lib)]
    public static extern void kb_emu_set_profile_connected(EmulatorHandle e, [MarshalAs(UnmanagedType.U1)] bool connected);

    [DllImport(Lib)]
    public static extern void kb_emu_set_profile_open(EmulatorHandle e, [MarshalAs(UnmanagedType.U1)] bool open);

    [DllImport(Lib)] public static extern void kb_emu_set_profile(EmulatorHandle e, byte profile);
    [DllImport(Lib)] public static extern byte kb_emu_profile_count();
    [DllImport(Lib)] public static extern byte kb_emu_profile(EmulatorHandle e);

    [DllImport(Lib)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static extern bool kb_emu_profile_open(EmulatorHandle e);

    [DllImport(Lib)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static extern bool kb_emu_profile_connected(EmulatorHandle e);

    [DllImport(Lib)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static extern bool kb_emu_peripheral_connected(EmulatorHandle e);

    [DllImport(Lib)] public static extern void kb_emu_set_transport(EmulatorHandle e, byte transport);
    [DllImport(Lib)] public static extern byte kb_emu_transport(EmulatorHandle e);
    [DllImport(Lib)] public static extern byte kb_emu_link_stage_count();
    [DllImport(Lib)] public static extern nuint kb_emu_link_stage_name(byte index, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern void kb_emu_set_link_stage(EmulatorHandle e, byte index);
    [DllImport(Lib)] public static extern byte kb_emu_link_stage(EmulatorHandle e);

    [DllImport(Lib)] public static extern uint kb_emu_simulate_pairing(EmulatorHandle e, uint code, uint nowMs);

    [DllImport(Lib)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static extern bool kb_emu_pairing(EmulatorHandle e);

    [DllImport(Lib)] public static extern sbyte kb_emu_radio_dbm(EmulatorHandle e);

    [DllImport(Lib)] public static extern byte kb_emu_power_state(EmulatorHandle e);
    [DllImport(Lib)] public static extern void kb_emu_set_power_timeouts(EmulatorHandle e, uint idleMs, uint sleepMs, uint nowMs);

    [DllImport(Lib)] public static extern byte kb_emu_raw_hid(EmulatorHandle e, byte* report, nuint len, uint nowMs);

    [DllImport(Lib)] public static extern nuint kb_emu_display(EmulatorHandle e, byte half, uint nowMs, byte* buf, nuint cap);

    /// <summary>Call a "copy UTF-8 into buffer, return full length" function.</summary>
    public static string ReadString(Func<IntPtr, nuint, nuint> call)
    {
        Span<byte> buf = stackalloc byte[128];
        fixed (byte* p = buf)
        {
            var len = (int)call((IntPtr)p, (nuint)buf.Length);
            if (len <= buf.Length) return Encoding.UTF8.GetString(buf[..len]);
        }
        var big = new byte[(int)call(IntPtr.Zero, 0)];
        fixed (byte* p = big)
        {
            call((IntPtr)p, (nuint)big.Length);
        }
        return Encoding.UTF8.GetString(big);
    }
}

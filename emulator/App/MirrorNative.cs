using System.Diagnostics.CodeAnalysis;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace KbEmulator;

/// <summary>A mirror from <c>kb_mirror_new</c>; released with <c>kb_mirror_free</c>, exactly once.</summary>
internal sealed class MirrorHandle() : SafeHandleZeroOrMinusOneIsInvalid(ownsHandle: true)
{
    protected override bool ReleaseHandle()
    {
        MirrorNative.kb_mirror_free(handle);
        return true;
    }
}

/// <summary>P/Invoke bindings for the mirror of a real keyboard (<c>emu/src/mirror.rs</c>).</summary>
[SuppressMessage("Security", "S6640:Using unsafe code blocks is security-sensitive",
    Justification = "C ABI of libkb_emu.so: status, frames and labels pass as pointers to pinned buffers.")]
internal static unsafe class MirrorNative
{
    private const string Lib = "kb_emu";

    /// <summary>Same layout as <c>MirrorStatus</c> in Rust; booleans are bytes.</summary>
    [StructLayout(LayoutKind.Sequential)]
    public struct Status
    {
        public byte Layer;
        public byte CapsWord;
        public byte Mods;
        public byte Transport;
        public byte Profile;
        public byte ProfileConnected;
        public byte ProfileOpen;
        public byte Linked;
        public byte BatteryLeft;
        public byte BatteryRight;
        public byte ChargingLeft;
        public byte ChargingRight;
        public byte Power;
        public byte Reserved;
        public ushort Wpm;
        public ushort Presses;
    }

    [DllImport(Lib)] public static extern MirrorHandle kb_mirror_new();
    [DllImport(Lib)] public static extern void kb_mirror_free(IntPtr m);
    [DllImport(Lib)] public static extern void kb_mirror_set_status(MirrorHandle m, Status* status, byte* stage, nuint stageLen);
    [DllImport(Lib)] public static extern void kb_mirror_set_frame(MirrorHandle m, byte half, byte* frame);
    [DllImport(Lib)] public static extern nuint kb_mirror_display(MirrorHandle m, byte half, uint nowMs, byte* buf, nuint cap);
    [DllImport(Lib)] public static extern nuint kb_mirror_label(byte layer, byte pos, byte* buf, nuint cap);
}

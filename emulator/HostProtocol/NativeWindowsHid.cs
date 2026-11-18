using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using Microsoft.Win32.SafeHandles;

namespace Kb.HostProtocol;

/// <summary>The real Windows HID API: SetupAPI to list the collections, hid.dll to ask them.</summary>
[SupportedOSPlatform("windows")]
public sealed class NativeWindowsHid : IWindowsHidApi
{
    public IReadOnlyList<string> DevicePaths()
    {
        HidD_GetHidGuid(out var hidGuid);
        var set = SetupDiGetClassDevs(ref hidGuid, IntPtr.Zero, IntPtr.Zero, DigcfPresent | DigcfDeviceInterface);
        if (set == InvalidHandle) throw new Win32Exception(Marshal.GetLastWin32Error(), "SetupDiGetClassDevs");
        var paths = new List<string>();
        try
        {
            var data = new SpDeviceInterfaceData { CbSize = Marshal.SizeOf<SpDeviceInterfaceData>() };
            for (uint index = 0; SetupDiEnumDeviceInterfaces(set, IntPtr.Zero, ref hidGuid, index, ref data); index++)
            {
                SetupDiGetDeviceInterfaceDetail(set, ref data, IntPtr.Zero, 0, out var needed, IntPtr.Zero);
                if (needed == 0) continue;
                var detail = Marshal.AllocHGlobal((int)needed);
                try
                {
                    // SP_DEVICE_INTERFACE_DETAIL_DATA_W: cbSize (8 on 64-bit, 6 on 32-bit), then the path.
                    Marshal.WriteInt32(detail, IntPtr.Size == 8 ? 8 : 6);
                    if (SetupDiGetDeviceInterfaceDetail(set, ref data, detail, needed, out _, IntPtr.Zero)
                        && Marshal.PtrToStringUni(detail + 4) is { Length: > 0 } path)
                        paths.Add(path);
                }
                finally
                {
                    Marshal.FreeHGlobal(detail);
                }
            }
        }
        finally
        {
            SetupDiDestroyDeviceInfoList(set);
        }
        return paths;
    }

    public HidCollectionInfo? Query(string path)
    {
        // No access rights: enough to ask, and it works on collections Windows holds open
        // (keyboards, mice) too.
        using var handle = CreateFile(path, 0, FileShareRead | FileShareWrite, IntPtr.Zero, OpenExisting, 0,
            IntPtr.Zero);
        if (handle.IsInvalid) return null;
        var attributes = new HiddAttributes { Size = Marshal.SizeOf<HiddAttributes>() };
        if (!HidD_GetAttributes(handle, ref attributes)) return null;
        if (!HidD_GetPreparsedData(handle, out var preparsed)) return null;
        try
        {
            if (HidP_GetCaps(preparsed, out var caps) != HidpStatusSuccess) return null;
            byte reportId = 0;
            if (caps.NumberOutputValueCaps > 0)
            {
                var count = caps.NumberOutputValueCaps;
                var buffer = Marshal.AllocHGlobal(ValueCapsSize * count);
                try
                {
                    if (HidP_GetValueCaps(HidpOutput, buffer, ref count, preparsed) == HidpStatusSuccess && count > 0)
                        reportId = Marshal.ReadByte(buffer, 2); // HIDP_VALUE_CAPS.ReportID
                }
                finally
                {
                    Marshal.FreeHGlobal(buffer);
                }
            }
            return new HidCollectionInfo(attributes.VendorId, attributes.ProductId, caps.UsagePage, caps.Usage,
                caps.OutputReportByteLength, reportId);
        }
        finally
        {
            HidD_FreePreparsedData(preparsed);
        }
    }

    public IHidWriter OpenForWrite(string path)
    {
        SafeFileHandle? handle = null;
        try
        {
            handle = CreateFile(path, GenericWrite, FileShareRead | FileShareWrite, IntPtr.Zero, OpenExisting, 0,
                IntPtr.Zero);
            if (handle.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error(), $"can't open {path}");
            var writer = new Writer(handle);
            handle = null; // the writer owns it now
            return writer;
        }
        finally
        {
            handle?.Dispose();
        }
    }

    private sealed class Writer(SafeFileHandle handle) : IHidWriter
    {
        public void Write(byte[] report)
        {
            // The interrupt OUT pipe on USB, a GATT write over Bluetooth; SET_REPORT if that fails.
            if (WriteFile(handle, report, report.Length, out var written, IntPtr.Zero) && written == report.Length)
                return;
            var error = Marshal.GetLastWin32Error();
            if (!HidD_SetOutputReport(handle, report, report.Length))
                throw new Win32Exception(error, "writing the report failed");
        }

        public void Dispose() => handle.Dispose();

        [DllImport("hid.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.U1)]
        private static extern bool HidD_SetOutputReport(SafeFileHandle device, byte[] report, int length);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool WriteFile(SafeFileHandle file, byte[] buffer, int numberOfBytesToWrite,
            out int numberOfBytesWritten, IntPtr overlapped);
    }

    private const uint DigcfPresent = 0x02;
    private const uint DigcfDeviceInterface = 0x10;
    private const uint GenericWrite = 0x40000000;
    private const uint FileShareRead = 0x01;
    private const uint FileShareWrite = 0x02;
    private const uint OpenExisting = 3;
    private const int HidpStatusSuccess = 0x00110000;
    private const int HidpOutput = 1;
    private const int ValueCapsSize = 72; // sizeof(HIDP_VALUE_CAPS)
    private static readonly IntPtr InvalidHandle = new(-1);

    [StructLayout(LayoutKind.Sequential)]
    private struct SpDeviceInterfaceData
    {
        public int CbSize;
        public Guid InterfaceClassGuid;
        public uint Flags;
        public IntPtr Reserved;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct HiddAttributes
    {
        public int Size;
        public ushort VendorId;
        public ushort ProductId;
        public ushort VersionNumber;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct HidpCaps
    {
        public ushort Usage;
        public ushort UsagePage;
        public ushort InputReportByteLength;
        public ushort OutputReportByteLength;
        public ushort FeatureReportByteLength;
        [MarshalAs(UnmanagedType.ByValArray, SizeConst = 17)]
        public ushort[] Reserved;
        public ushort NumberLinkCollectionNodes;
        public ushort NumberInputButtonCaps;
        public ushort NumberInputValueCaps;
        public ushort NumberInputDataIndices;
        public ushort NumberOutputButtonCaps;
        public ushort NumberOutputValueCaps;
        public ushort NumberOutputDataIndices;
        public ushort NumberFeatureButtonCaps;
        public ushort NumberFeatureValueCaps;
        public ushort NumberFeatureDataIndices;
    }

    [DllImport("hid.dll")]
    private static extern void HidD_GetHidGuid(out Guid hidGuid);

    [DllImport("hid.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.U1)]
    private static extern bool HidD_GetAttributes(SafeFileHandle device, ref HiddAttributes attributes);

    [DllImport("hid.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.U1)]
    private static extern bool HidD_GetPreparsedData(SafeFileHandle device, out IntPtr preparsedData);

    [DllImport("hid.dll")]
    [return: MarshalAs(UnmanagedType.U1)]
    private static extern bool HidD_FreePreparsedData(IntPtr preparsedData);

    [DllImport("hid.dll")]
    private static extern int HidP_GetCaps(IntPtr preparsedData, out HidpCaps capabilities);

    [DllImport("hid.dll")]
    private static extern int HidP_GetValueCaps(int reportType, IntPtr valueCaps, ref ushort length,
        IntPtr preparsedData);

    [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern IntPtr SetupDiGetClassDevs(ref Guid classGuid, IntPtr enumerator, IntPtr parent,
        uint flags);

    [DllImport("setupapi.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiEnumDeviceInterfaces(IntPtr deviceInfoSet, IntPtr deviceInfoData,
        ref Guid interfaceClassGuid, uint memberIndex, ref SpDeviceInterfaceData deviceInterfaceData);

    [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr deviceInfoSet,
        ref SpDeviceInterfaceData deviceInterfaceData, IntPtr deviceInterfaceDetailData, uint detailSize,
        out uint requiredSize, IntPtr deviceInfoData);

    [DllImport("setupapi.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiDestroyDeviceInfoList(IntPtr deviceInfoSet);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern SafeFileHandle CreateFile(string fileName, uint desiredAccess, uint shareMode,
        IntPtr securityAttributes, uint creationDisposition, uint flagsAndAttributes, IntPtr templateFile);
}

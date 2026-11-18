using Kb.HostProtocol;

namespace KbNotify;

// kbnotify: show a notification on the right half of the keyboard.
//
//   kbnotify "Build passed"            emulator if it is running, else the keyboard (USB, else Bluetooth)
//   kbnotify --emulator "Build passed"
//   kbnotify --device /dev/hidraw5 "Build passed"      (Windows: a device path from --list)
//   kbnotify --list
//   kbnotify --clear
//
// The logic is in Notifier; this only wires it to the console and the real devices.
internal static class Program
{
    public static async Task<int> Main(string[] args)
    {
        var environment = new NotifierEnvironment(
            Console.Out,
            Console.Error,
            OperatingSystem.IsWindows(),
            FindKeyboards,
            path => HidrawLink.ForPath(path),
            Open,
            () => File.Exists(EmulatorLink.DefaultSocketPath),
            async () => await EmulatorLink.ConnectAsync());
        return await new Notifier(environment).RunAsync(args);
    }

    private static IReadOnlyList<RawHidDevice> FindKeyboards() =>
        OperatingSystem.IsWindows() ? WindowsHid.Find(new NativeWindowsHid()) : HidrawLink.Find();

    private static IKeyboardLink Open(RawHidDevice target) =>
        OperatingSystem.IsWindows() ? new WindowsHidLink(new NativeWindowsHid(), target) : new HidrawLink(target);
}

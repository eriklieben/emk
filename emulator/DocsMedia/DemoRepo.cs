namespace KbEmulator.DocsMedia;

/// <summary>A stand-in repository for Flash: one keyboard, demo serials, packages with a fixed date.</summary>
internal static class DemoRepo
{
    /// <summary>A stand-in signing key file (not a key): only its presence and permissions show.</summary>
    public static string Key(string dir)
    {
        Directory.CreateDirectory(dir);
        var path = Path.Combine(dir, "emk-fw.pem");
        File.WriteAllText(path, "not a key: a stand-in for the README screenshots\n");
        if (!OperatingSystem.IsWindows()) File.SetUnixFileMode(path, UnixFileMode.UserRead | UnixFileMode.UserWrite);
        File.SetLastWriteTime(path, new DateTime(2026, 11, 15, 12, 0, 0, DateTimeKind.Local));
        return path;
    }

    public static string Create(string dir)
    {
        Directory.CreateDirectory(Path.Combine(dir, "emulator"));
        Directory.CreateDirectory(Path.Combine(dir, "target"));
        File.WriteAllText(Path.Combine(dir, "hil"), "");
        File.WriteAllText(Path.Combine(dir, HalfRecord.FileName),
            "erik_kbd left 0123456789ABCDEF uf2\nerik_kbd right FEDCBA9876543210 uf2\n");
        var when = new DateTime(2026, 11, 17, 9, 30, 0, DateTimeKind.Local);
        foreach (var side in new[] { "left", "right" })
        {
            foreach (var ext in new[] { "uf2", "hex" })
            {
                var path = Path.Combine(dir, "target", $"fw-nrf-{side}-ble-erik_kbd.{ext}");
                File.WriteAllBytes(path, new byte[side == "left" ? 412_160 : 268_800]);
                File.SetLastWriteTime(path, when);
            }
        }
        return dir;
    }
}

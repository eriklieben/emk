namespace KbEmulator.Tests;

public class SizeReportTests
{
    private const string Json = """
        {
          "chip": "nRF52840", "board": "nice!nano v2", "flashTotal": 1048576,
          "flashRegions": [
            {"name": "MBR + SoftDevice area", "size": 155648},
            {"name": "Firmware", "size": 811008},
            {"name": "Settings storage", "size": 32768},
            {"name": "UF2 bootloader", "size": 49152}
          ],
          "flashUsed": 81100, "ramTotal": 262144, "ramUsed": 26214,
          "includes": "x", "excludes": "y", "generated": "2026-11-15T10:00:00+00:00"
        }
        """;

    [Test]
    public async Task Flash_fraction_is_relative_to_the_firmware_region()
    {
        // Arrange
        var report = SizeReport.Parse(Json);

        // Act
        var fraction = report.FlashFraction;

        // Assert
        await Assert.That(fraction).IsEqualTo(0.1).Within(0.0001);
    }

    [Test]
    public async Task Ram_fraction_is_relative_to_all_ram()
    {
        // Arrange
        var report = SizeReport.Parse(Json);

        // Act
        var fraction = report.RamFraction;

        // Assert
        await Assert.That(fraction).IsEqualTo(0.1).Within(0.0001);
    }

    [Test]
    public async Task Report_is_found_by_walking_up_to_the_repo_root()
    {
        // Arrange
        var root = Directory.CreateTempSubdirectory("size-report-test").FullName;
        Directory.CreateDirectory(Path.Combine(root, "target"));
        await File.WriteAllTextAsync(Path.Combine(root, "target", "size-report.json"), Json);
        var deep = Directory.CreateDirectory(Path.Combine(root, "emulator", "App", "bin")).FullName;

        // Act
        var report = SizeReport.TryLoad(deep);

        // Assert
        await Assert.That(report?.FlashUsed).IsEqualTo(81100L);
        Directory.Delete(root, recursive: true);
    }

    [Test]
    public async Task Sizes_are_shown_in_kilobytes()
    {
        // Arrange
        const long bytes = 29288;

        // Act
        var text = SizeReport.Kb(bytes);

        // Assert
        await Assert.That(text).IsEqualTo("28.6 KB");
    }
}

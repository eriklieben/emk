using System.Text.Json;

namespace KbEmulator;

/// <summary>Flash/RAM measurement written by <c>./size-report</c> (target/size-report.json).</summary>
public sealed record SizeReport(
    string Chip,
    string Board,
    long FlashTotal,
    IReadOnlyList<SizeReport.Region> FlashRegions,
    long FlashUsed,
    long RamTotal,
    long RamUsed,
    string Includes,
    string Excludes,
    string Generated)
{
    public sealed record Region(string Name, long Size);

    private static readonly JsonSerializerOptions Json = new() { PropertyNameCaseInsensitive = true };

    /// <summary>Size of the region the firmware lives in.</summary>
    public long FirmwareRegion => FlashRegions.FirstOrDefault(r => r.Name == "Firmware")?.Size ?? FlashTotal;

    public double FlashFraction => (double)FlashUsed / FirmwareRegion;

    public double RamFraction => (double)RamUsed / RamTotal;

    public static SizeReport Parse(string json) =>
        JsonSerializer.Deserialize<SizeReport>(json, Json) ?? throw new InvalidDataException("empty size report");

    /// <summary>Find target/size-report.json by walking up from the app's folder to the repo root.</summary>
    public static SizeReport? TryLoad(string? startDirectory = null)
    {
        for (var dir = new DirectoryInfo(startDirectory ?? AppContext.BaseDirectory); dir != null; dir = dir.Parent)
        {
            var path = Path.Combine(dir.FullName, "target", "size-report.json");
            if (!File.Exists(path)) continue;
            try
            {
                return Parse(File.ReadAllText(path));
            }
            catch (Exception e) when (e is JsonException or InvalidDataException or IOException)
            {
                return null;
            }
        }
        return null;
    }

    public static string Kb(long bytes) => bytes >= 1024 * 1024
        ? $"{bytes / (1024.0 * 1024):0.#} MB"
        : $"{bytes / 1024.0:0.#} KB";
}

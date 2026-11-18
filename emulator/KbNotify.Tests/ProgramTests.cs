namespace KbNotify.Tests;

/// <summary>The entry point with the real console and devices: only command lines that are done before any
/// device or the emulator is looked for.</summary>
[NotInParallel(nameof(Console))]
public class ProgramTests
{
    [Test]
    public async Task Help_exits_zero()
    {
        // Arrange
        string[] args = ["-h"];

        // Act
        var code = await Program.Main(args);

        // Assert
        await Assert.That(code).IsEqualTo(0);
    }

    [Test]
    public async Task Nothing_to_send_exits_two()
    {
        // Arrange
        string[] args = ["--emulator"];

        // Act
        var code = await Program.Main(args);

        // Assert
        await Assert.That(code).IsEqualTo(2);
    }
}

namespace KbNotify.Tests;

public class OptionsTests
{
    [Test]
    public async Task Plain_words_are_the_text()
    {
        // Arrange
        string[] args = ["Build", "passed"];

        // Act
        var options = Options.Parse(args);

        // Assert
        await Assert.That((options.UseEmulator, options.Device, options.Clear, options.List, options.Help))
            .IsEqualTo((false, (string?)null, false, false, false));
        await Assert.That(options.Words).IsEquivalentTo(new[] { "Build", "passed" });
    }

    [Test]
    public async Task Flags_are_read_anywhere_between_the_words()
    {
        // Arrange
        string[] args = ["--emulator", "Hi", "--clear", "there", "--list"];

        // Act
        var options = Options.Parse(args);

        // Assert
        await Assert.That((options.UseEmulator, options.Clear, options.List, options.Help)).IsEqualTo((true, true, true, false));
        await Assert.That(options.Words).IsEquivalentTo(new[] { "Hi", "there" });
    }

    [Test]
    public async Task Device_takes_the_next_argument_as_its_path()
    {
        // Arrange
        string[] args = ["--device", "/dev/hidraw5", "Hi"];

        // Act
        var options = Options.Parse(args);

        // Assert
        await Assert.That(options.Device).IsEqualTo("/dev/hidraw5");
        await Assert.That(options.Words).IsEquivalentTo(new[] { "Hi" });
    }

    [Test]
    public async Task Device_without_a_path_is_a_word_of_the_text()
    {
        // Arrange
        string[] args = ["Hi", "--device"];

        // Act
        var options = Options.Parse(args);

        // Assert
        await Assert.That(options.Device).IsNull();
        await Assert.That(options.Words).IsEquivalentTo(new[] { "Hi", "--device" });
    }

    [Test]
    [Arguments("-h")]
    [Arguments("--help")]
    public async Task Help_stops_reading_the_rest(string flag)
    {
        // Arrange
        string[] args = ["Hi", flag, "--clear", "more"];

        // Act
        var options = Options.Parse(args);

        // Assert: the words before it are not kept either
        await Assert.That(options.Help).IsTrue();
        await Assert.That(options.Clear).IsFalse();
        await Assert.That(options.Words).IsEmpty();
    }

    [Test]
    public async Task No_arguments_is_no_text()
    {
        // Arrange
        string[] args = [];

        // Act
        var options = Options.Parse(args);

        // Assert
        await Assert.That(options.Words).IsEmpty();
        await Assert.That(options.Device).IsNull();
    }
}

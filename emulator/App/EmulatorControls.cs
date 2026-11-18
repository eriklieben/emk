using Avalonia.Controls;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>The Emulator's controls, so tests click them as a user would.</summary>
internal sealed class EmulatorControls
{
    public Gauge[] Gauges { get; } = new Gauge[2];
    public Slider[] Sliders { get; } = new Slider[2];
    public TextBlock[] BatteryText { get; } = new TextBlock[2];
    public Pill PowerPill { get; set; } = null!;
    public Segmented PowerPreset { get; set; } = null!;
    public TextBlock PowerHint { get; set; } = null!;
    public BenchButton ReplayStartUp { get; set; } = null!;
    public List<ProfileCell> Profiles { get; } = new();
    public Toggle Connected { get; set; } = null!;
    public Toggle Open { get; set; } = null!;
    public Toggle Linked { get; set; } = null!;
    public Segmented Output { get; set; } = null!;
    public BenchButton LinkStage { get; set; } = null!;
    public List<Action> LinkStages { get; } = new();
    public BenchButton SimulatePairing { get; set; } = null!;
    public Segmented Radio { get; set; } = null!;
    public TextBlock RadioReadout { get; set; } = null!;
    public TextBox Message { get; set; } = null!;
    public BenchButton Send { get; set; } = null!;
    public BenchButton ClearMessage { get; set; } = null!;
    public Toggle Upright { get; set; } = null!;
    public BenchButton ReleaseLatched { get; set; } = null!;
    public BenchButton ClearOutput { get; set; } = null!;
    public BenchButton ResetPointer { get; set; } = null!;
    public Toggle SmoothScrolling { get; set; } = null!;
}

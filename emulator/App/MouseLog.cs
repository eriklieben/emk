using Kb.HostProtocol;

namespace KbEmulator;

/// <summary>
/// Turns the stream of mouse reports (one per millisecond of motion) into a
/// few readable lines: "mouse moved (+120, −40)" per gesture, "L click",
/// "scroll ↓ 3". A gesture ends after <see cref="Idle"/> without motion or
/// scrolling, or when a button is pressed.
/// </summary>
public sealed class MouseLog
{
    public const uint Idle = 300;

    private int _dx, _dy;
    private double _wheel, _pan;
    private bool _pending;
    private uint _last;
    private byte _buttons;

    /// <summary>A report came in at <paramref name="nowMs"/>; returns the lines to log now.</summary>
    /// <param name="wheelHires">Wheel in 1/16 detents (smooth scrolling); same for <paramref name="panHires"/>.</param>
    public IReadOnlyList<string> Add(MouseReport report, uint nowMs, bool wheelHires = false, bool panHires = false)
    {
        var lines = new List<string>();
        var pressed = (byte)(report.Buttons & ~_buttons);
        _buttons = report.Buttons;
        if (pressed != 0)
        {
            lines.AddRange(Finish());
            lines.Add($"{MouseReport.ButtonNames(pressed)} click");
        }
        if (report.X != 0 || report.Y != 0 || report.Wheel != 0 || report.Pan != 0)
        {
            _dx += report.X;
            _dy += report.Y;
            _wheel += report.Wheel / (wheelHires ? 16.0 : 1.0);
            _pan += report.Pan / (panHires ? 16.0 : 1.0);
            _pending = true;
            _last = nowMs;
        }
        return lines;
    }

    /// <summary>The lines of a gesture that has gone quiet (call on every tick).</summary>
    public IReadOnlyList<string> Flush(uint nowMs) => _pending && nowMs - _last >= Idle ? Finish() : [];

    private List<string> Finish()
    {
        var lines = new List<string>();
        if (!_pending) return lines;
        if (_dx != 0 || _dy != 0) lines.Add($"mouse moved ({MouseReport.WithSign(_dx)}, {MouseReport.WithSign(_dy)})");
        var wheel = (int)Math.Round(_wheel);
        var pan = (int)Math.Round(_pan);
        if (wheel != 0) lines.Add($"scroll {(wheel > 0 ? "↑" : "↓")} {Math.Abs(wheel)}");
        if (pan != 0) lines.Add($"scroll {(pan > 0 ? "→" : "←")} {Math.Abs(pan)}");
        (_dx, _dy, _wheel, _pan, _pending) = (0, 0, 0, 0, false);
        return lines;
    }
}

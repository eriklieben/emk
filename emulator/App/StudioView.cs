using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.LogicalTree;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>
/// Studio: edit the keymap live, on the emulator or on the real keyboard
/// (its log port), with the protocol of docs/studio-protocol.md. Layers on
/// the left, the keyboard in the tray, the selected key's editor on the
/// right. Writes need the physical StudioUnlock key. Combos are edited too on
/// firmware with the combos capability (StudioView.Combos.cs), read-only otherwise.
/// </summary>
public sealed partial class StudioView : Grid
{
    private const string ResetText = "Reset all to default…";
    private static readonly (string Text, string Tone) NotSent = ("not sent: the keyboard isn't connected", "crit");

    private readonly EmulatedKeyboard _keyboard;
    private readonly DebugView? _debug;
    private readonly StudioClient _emuClient;
    private readonly StudioClient _kbClient;
    private readonly CorneView _view = new(Keymap.KeyCount) { SelectMode = true, ShowBoardControls = false };
    private readonly IReadOnlyList<string> _defaultNames;

    // Strip.
    private readonly ContentControl _targetHost = new();
    private readonly Lamp _supportLamp = new();
    private readonly Lamp _lockLamp = new();
    private readonly Lamp _dirtyLamp = new();
    private readonly BenchButton _unlock = new("Unlock (press nav + Studio)");
    private readonly BenchButton _lock = new("Lock");
    private readonly BenchButton _save = new("Save to flash", ButtonKind.Primary);
    private readonly BenchButton _revert = new("Revert");
    private readonly BenchButton _reset = new(ResetText, ButtonKind.Danger);
    private readonly BenchButton _reload = new("Reload");
    private readonly BenchButton _poster = new("Export poster…");

    // Tray, sides.
    private readonly StackPanel _layerList = new() { Spacing = 6 };
    private readonly StackPanel _holdTaps = new() { Spacing = 6 };
    private readonly TextBlock _trayTitle = W.Lbl("");
    private readonly Chip _changedChip = new("Changed", "0");
    private readonly Chip _transparentChip = new("Falls through", "0");
    private readonly ContentControl _editor = new() { Margin = new Thickness(0, 0, 12, 0) }; // room for the scroll bar
    private readonly TextBox _search = new() { PlaceholderText = "Search keys (A, F5, LBKT, EXCL…)", FontSize = 13 };
    private readonly WrapPanel _keyResults = new() { Orientation = Orientation.Horizontal, ItemSpacing = 4, LineSpacing = 4 };

    private int _target; // 0 emulator, 1 keyboard
    private int _layer;
    private int _key = 29;
    private StudioAction _edit = new();
    private (string Text, string Tone)? _message;
    private DateTime _resetArmedAt;
    private bool _active;
    private uint _generation;

    // Combo editing: the combo being edited (index, or null for a new one).
    private readonly StackPanel _comboList = new() { Spacing = 6 };
    private StudioCombo? _combo;
    private int? _comboIndex;
    private DateTime _removeArmedAt;
    private TextBlock? _comboPreview;

    public StudioView(EmulatedKeyboard keyboard, DebugView? debug = null)
    {
        _keyboard = keyboard;
        _debug = debug;
        _defaultNames = Keymap.LayerNames(keyboard.LayerName);
        _emuClient = new StudioClient(new EmulatorStudioTransport(keyboard));
        var portTransport = new PortStudioTransport(line => _debug?.SendRaw(line) == true);
        _kbClient = new StudioClient(portTransport);
        if (_debug != null) _debug.StudioLineReceived += portTransport.Deliver;
        foreach (var client in new[] { _emuClient, _kbClient })
        {
            client.Changed += () =>
            {
                if (client != Client) return;
                Refresh();
                StateChanged?.Invoke();
            };
            client.Acknowledged += ack => { if (client == Client) OnAck(ack); };
        }

        _view.KeySelected += pos =>
        {
            if (_combo != null)
            {
                ToggleComboKey(pos);
                return;
            }
            _key = pos;
            _message = null;
            LoadEditor();
            Refresh();
        };
        _unlock.Click += () =>
        {
            _keyboard.PressStudioUnlock();
            Client.Status();
        };
        _lock.Click += () => Client.Lock();
        _save.Click += () => Client.Save();
        _revert.Click += () => Client.Revert();
        _reload.Click += () => Client.Get();
        _poster.Click += () =>
        {
            var dialog = new PosterDialog(PosterModel, PosterKeyboardName, ActualThemeVariant != Avalonia.Styling.ThemeVariant.Light);
            if (TopLevel.GetTopLevel(this) is Window owner) dialog.ShowDialog(owner);
            else dialog.Show();
        };
        _reset.Click += () =>
        {
            if (DateTime.UtcNow - _resetArmedAt < TimeSpan.FromSeconds(4))
            {
                _resetArmedAt = default;
                _reset.Text = ResetText;
                Client.Reset();
                return;
            }
            _resetArmedAt = DateTime.UtcNow;
            _reset.Text = "Click again: reset every key to the default";
        };
        _search.TextChanged += (_, _) => FillKeyResults();

        BuildLayout();
        LoadEditor();
        Refresh();
    }

    /// <summary>Lock, dirty or support changed (the top bar).</summary>
    public event Action? StateChanged;

    /// <summary>The client of the chosen target.</summary>
    internal StudioClient Client => _target == 0 ? _emuClient : _kbClient;

    internal CorneView View => _view;

    internal BenchButton UnlockButton => _unlock;
    internal BenchButton LockButton => _lock;
    internal BenchButton SaveButton => _save;
    internal BenchButton RevertButton => _revert;
    internal BenchButton ResetButton => _reset;

    /// <summary>Esc: disarm Remove or Reset all, else leave the combo editor. True if something closed.</summary>
    public bool Escape()
    {
        if (_removeArmedAt != default)
        {
            _removeArmedAt = default;
            _message = null;
            RefreshEditor();
            return true;
        }
        if (_resetArmedAt != default)
        {
            _resetArmedAt = default;
            _reset.Text = ResetText;
            return true;
        }
        if (_combo == null) return false;
        LeaveCombo();
        return true;
    }

    /// <summary>The real keyboard is the target.</summary>
    public bool TargetIsKeyboard => _target == 1;
    internal int Layer => _layer;
    internal StudioAction Edit => _edit;
    internal string? Message => _message?.Text;

    /// <summary>Change the edit as the panel's controls would (tests).</summary>
    internal void EditForTest(Action<StudioAction> change)
    {
        change(_edit);
        RefreshEditor();
    }

    /// <summary>The key panel's text (tests).</summary>
    internal string KeyText => string.Join("\n", _editor.GetLogicalDescendants().OfType<TextBlock>().Select(t => t.Text));

    /// <summary>Studio is shown: ask the target for its keymap.</summary>
    public void Activate()
    {
        _active = true;
        if (_target == 1) _debug?.EnsurePort();
        if (Client.Support is StudioSupport.Unknown or StudioSupport.NotConnected or StudioSupport.No) Client.Get();
    }

    public void Deactivate() => _active = false;

    /// <summary>Read answers; called often from the window's timer.</summary>
    public void Pump()
    {
        _emuClient.Pump();
        _kbClient.Pump();
        if (_target == 1 && _kbClient.Support == StudioSupport.NotConnected && _debug?.PortConnected == true) _kbClient.Get();
        if (_target == 0 && _keyboard.StudioGeneration != _generation)
        {
            _generation = _keyboard.StudioGeneration;
            if (_emuClient.Support == StudioSupport.Yes) Refresh();
        }
        if (_resetArmedAt != default && DateTime.UtcNow - _resetArmedAt > TimeSpan.FromSeconds(4))
        {
            _resetArmedAt = default;
            _reset.Text = ResetText;
        }
        if (_removeArmedAt != default && DateTime.UtcNow - _removeArmedAt > TimeSpan.FromSeconds(4))
        {
            _removeArmedAt = default;
            RefreshEditor();
        }
        if (_active) RefreshLock();
    }

    /// <summary>Pick the emulator (0) or the real keyboard (1).</summary>
    internal void SelectTarget(int target)
    {
        _target = target;
        _message = null;
        if (target == 1) _debug?.EnsurePort();
        Client.Get();
        LoadEditor();
        Refresh();
    }

    /// <summary>Show a layer and select a key.</summary>
    internal void Select(int layer, int key)
    {
        _layer = Math.Clamp(layer, 0, Math.Max(0, LayerNames.Count - 1));
        _key = key;
        _message = null;
        LoadEditor();
        Refresh();
    }

    private IReadOnlyList<string> LayerNames => Client.Version?.Names is { Count: > 0 } names ? names : _defaultNames;

    /// <summary>The live binding: from the target, else the emulator's compiled one.</summary>
    private string Live(int layer, int pos) => Client.Binding(layer, pos) ?? _keyboard.StudioDefault(layer, pos);

    /// <summary>
    /// The compiled binding: the target's own (<c>#LD</c>, firmware with the
    /// defaults capability), else the emulator's compiled keymap.
    /// </summary>
    private string Default(int layer, int pos) =>
        Client.Has("defaults") && Client.DefaultLayers.TryGetValue(layer, out var actions) && pos < actions.Count
            ? actions[pos]
            : _keyboard.StudioDefault(layer, pos);

    /// <summary>Whether the defaults come from the target itself.</summary>
    internal bool DefaultsFromTarget => Client.Has("defaults") && Client.DefaultLayers.Count > 0;

    private bool Changed(int layer, int pos) =>
        !string.Equals(StudioCodec.Canonical(Live(layer, pos)) ?? Live(layer, pos), Default(layer, pos), StringComparison.OrdinalIgnoreCase);

    /// <summary>The editor starts from the key's live binding.</summary>
    private void LoadEditor()
    {
        _edit = StudioAction.Parse(Live(_layer, _key));
        _search.Text = "";
    }

    // ---- actions ----------------------------------------------------------------------

    /// <summary>Dry run in the emulator: would the set be refused (lock-out, complex, invalid)?</summary>
    internal void Check()
    {
        var encoding = _edit.Encode();
        if (StudioCodec.Canonical(encoding) is null)
        {
            _message = ($"{encoding}: not an action", "crit");
        }
        else if (Client.Has("check"))
        {
            // The target's own dry run (works locked); the answer comes as #A check.
            _message = Client.Check(_layer, _key, encoding) ? ("checking…", "mut") : NotSent;
        }
        else if (_target == 1)
        {
            _message = ("This firmware can't check (Studio 1): the emulator's rules say ok; the keyboard decides when you apply.", "mut");
            var code = _keyboard.StudioCheck(_layer, _key, encoding);
            if (code != 0) _message = ($"In the emulator: {Explain(code)}", "warn");
        }
        else
        {
            var code = _keyboard.StudioCheck(_layer, _key, encoding);
            _message = code == 0 ? ($"check ok: {StudioCodec.Canonical(encoding)}", "good") : (Explain(code), "crit");
        }
        RefreshEditor();
    }

    private static string Explain(int code) => new StudioAck("check", null, null, false, code switch
    {
        1 => "locked",
        2 => "range",
        3 => "action",
        4 => "long",
        5 => "lockout",
        6 => "complex",
        7 => "storage",
        8 => "positions",
        9 => "name",
        10 => "timing",
        11 => "full",
        12 => "syntax",
        _ => $"code {code}",
    }).Explain();

    internal void Apply()
    {
        var encoding = _edit.Encode();
        _message = Client.Set(_layer, _key, encoding) ? ($"sent: set {_layer} {_key} {encoding}", "mut") : NotSent;
        RefreshEditor();
    }

    internal void RevertKey()
    {
        _edit = StudioAction.Parse(Default(_layer, _key));
        Apply();
    }

    private void OnAck(StudioAck ack)
    {
        var text = ack.Ok
            ? ack.Command switch
            {
                "set" => $"set {ack.Layer} {ack.Pos} ok: {ack.Detail}",
                "check" => $"check ok: {ack.Detail}",
                "combo add" => $"combo added as #{ack.Combo}: {ack.Detail}",
                "combo set" => $"combo #{ack.Combo} changed: {ack.Detail}",
                "combo remove" => $"combo #{ack.Combo} removed",
                "combo check" => $"combo check ok: {ack.Detail}",
                "save" => "saved to flash",
                "revert" => "reverted to what was saved",
                "reset" => "every key is back to the default",
                "lock" => "locked",
                _ => $"{ack.Command} ok",
            }
            : $"{ack.Command} {ack.Explain()}";
        _message = (text, ack.Ok ? "good" : "crit");
        if (ack is { Command: "combo add", Ok: true }) _comboIndex = ack.Combo;
        if (ack is { Command: "combo remove", Ok: true } || ack.Command is "revert" or "reset") _combo = null;
        if (ack is { Command: "set", Ok: true } || ack.Command is "revert" or "reset") LoadEditor();
        Refresh();
    }

    // ---- refresh --------------------------------------------------------------------

    private void Refresh()
    {
        RefreshStrip();
        var names = LayerNames;
        _layer = Math.Min(_layer, Math.Max(0, names.Count - 1));
        RefreshLayers(names);
        RefreshHoldTaps(names);
        RefreshCombos();
        RefreshKeyboard(names);
        RefreshEditor();
    }

    /// <summary>The target switch, the support and lock lamps, and the buttons they enable.</summary>
    private void RefreshStrip()
    {
        var client = Client;
        var keyboardName = _debug?.Mirror.Hello?.Name;
        var keyboardLabel = keyboardName != null ? $"Keyboard ({keyboardName})" : "Keyboard";
        var target = new Segmented(["Emulator", keyboardLabel], _target);
        target.SelectionChanged += SelectTarget;
        _targetHost.Content = target;

        (_supportLamp.Text, _supportLamp.State) = client.Support switch
        {
            StudioSupport.Yes => ($"Studio {client.Version!.Version} · store={client.Version.Store}", LampState.Good),
            StudioSupport.Asking => ("Asking for Studio…", LampState.Warn),
            StudioSupport.No => ("This firmware has no Studio: flash the latest build", LampState.Crit),
            StudioSupport.NotConnected => ("Keyboard not connected (left half on USB, log port)", LampState.Crit),
            _ => ("Studio: not asked yet", LampState.Off),
        };
        _unlock.IsVisible = _target == 0;
        var ready = client.Support == StudioSupport.Yes;
        _lock.Enabled = ready && client.Unlocked;
        _save.Enabled = ready && client.Unlocked && client.Dirty;
        _revert.Enabled = ready && client.Dirty;
        _reset.Enabled = ready && client.Unlocked;
        _dirtyLamp.IsVisible = ready;
        (_dirtyLamp.Text, _dirtyLamp.State) = client.Dirty
            ? ("Changes live, not saved to flash", LampState.Warn)
            : ("Saved", LampState.Good);
        RefreshLock();
    }

    private void RefreshLayers(IReadOnlyList<string> names)
    {
        _layerList.Children.Clear();
        for (var i = 0; i < names.Count; i++)
        {
            var index = i;
            var changed = Enumerable.Range(0, Keymap.KeyCount).Count(p => Changed(index, p));
            _layerList.Children.Add(LayerRow(i, names[i], changed > 0 ? $"{changed} changed" : Keymap.Describe(names[i]), i == _layer, () => Select(index, _key)));
        }
    }

    /// <summary>Hold-taps from the live bindings.</summary>
    private void RefreshHoldTaps(IReadOnlyList<string> names)
    {
        _holdTaps.Children.Clear();
        for (var l = 0; l < names.Count; l++)
        {
            for (var p = 0; p < Keymap.KeyCount; p++)
            {
                var action = StudioAction.Parse(Live(l, p));
                if (action.Kind != StudioKind.ModTap) continue;
                var (layer, pos) = (l, p);
                _holdTaps.Children.Add(ListRow(StudioCodec.Label(Live(l, p)).Replace('\n', '/'), $"{names[l]} · key {p}", () => Select(layer, pos)));
            }
        }
    }

    /// <summary>The keyboard in the tray: the layer's legends, what falls through, what changed, the combos.</summary>
    private void RefreshKeyboard(IReadOnlyList<string> names)
    {
        var labels = new string[Keymap.KeyCount];
        var through = new bool[Keymap.KeyCount];
        var changedKeys = new bool[Keymap.KeyCount];
        for (var pos = 0; pos < Keymap.KeyCount; pos++)
        {
            var live = Live(_layer, pos);
            through[pos] = _layer > 0 && live.Equals("trans", StringComparison.OrdinalIgnoreCase);
            labels[pos] = StudioCodec.Label(through[pos] ? Live(0, pos) : live);
            changedKeys[pos] = Changed(_layer, pos);
        }
        _view.SetLabels(labels);
        _view.SetTransparent(through);
        _view.SetChanged(changedKeys);
        _view.Combos = CombosOnLayer(_layer);
        _view.SelectedKey = _combo == null ? _key : null;
        _view.SetHighlighted(_combo?.Positions ?? []);
        var name = names.Count > _layer ? names[_layer] : $"layer {_layer}";
        _trayTitle.Text = (_combo != null
            ? $"{ComboTitle(_comboIndex)} · click keys to add or remove them"
            : $"{name} layer · key {_key} selected").ToUpperInvariant();
        _changedChip.Value = $"{changedKeys.Count(c => c)} keys";
        _transparentChip.Value = _layer == 0 ? "—" : $"{through.Count(t => t)} keys";
    }

    private void RefreshLock()
    {
        var client = Client;
        if (client.Support != StudioSupport.Yes)
        {
            _lockLamp.IsVisible = false;
            return;
        }
        _lockLamp.IsVisible = true;
        var left = client.UnlockRemaining;
        (_lockLamp.Text, _lockLamp.State) = client.Unlocked
            ? ($"Unlocked · locks in {(int)left.TotalMinutes}:{left.Seconds:00}", LampState.Good)
            : ("Locked: hold nav and press the Studio key on the keyboard", LampState.Warn);
        _lock.Enabled = client.Unlocked;
    }
}

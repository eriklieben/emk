using System.Globalization;
using Avalonia.Controls;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>Studio's combos: the list, and the combo editor.</summary>
public sealed partial class StudioView
{
    // ---- combos ---------------------------------------------------------------------

    /// <summary>
    /// The keymap on screen as a poster: the target's live bindings (applied,
    /// even if not saved) and combos; the built-in keymap when the target hasn't answered.
    /// </summary>
    internal PosterModel PosterModel(string keyboard)
    {
        var combos = CombosEditable ? Client.Combos : _keyboard.ReadStudioDefaultCombos();
        return KbEmulator.PosterModel.From(keyboard, LayerNames, Live, combos);
    }

    /// <summary>The keyboard's name for the poster: the connected one, else the default keyboard.</summary>
    internal string PosterKeyboardName => _target == 1 && _debug?.Mirror.Hello?.Name is { } name ? name : DeviceSources.DefaultKeyboard;

    /// <summary>Whether the target can bind mouse actions (firmware with the mouse capability).</summary>
    internal bool MouseSupported => Client.Has("mouse");

    /// <summary>Whether combos can be edited on this target (firmware with the combos capability).</summary>
    internal bool CombosEditable => Client.Has("combos");

    /// <summary>The live combos as text: the target's, or the compiled ones of a version-1 target.</summary>
    private IReadOnlyList<string> LiveCombos() => CombosEditable
        ? Client.Combos
        : Keymap.Combos.Select(c => CompiledComboText(c, "-")).ToList();

    /// <summary>A compiled combo as Studio's combo text, with <paramref name="action"/> for its action.</summary>
    private static string CompiledComboText(Keymap.Combo c, string action) =>
        $"{c.Name} {string.Join(',', c.Keys)} {string.Join(',', c.Layers)} {c.TimeoutMs} {c.PriorIdleMs} {action}";

    private static string ComboTitle(int? index) => index is { } i ? $"combo #{i}" : "new combo";

    private List<(int A, int B, string Name)> CombosOnLayer(int layer) =>
        LiveCombos().Select(StudioCombo.Parse)
            .Where(c => c != null && c.Positions.Count >= 2 && (c.Layers.Count == 0 || c.Layers.Contains(layer)))
            .Select(c => (c!.Positions.Min, c.Positions.Max, c.Name.Length == 0 ? "combo" : c.Name))
            .ToList();

    /// <summary>A combo's keys and action for the list: "Z + X → Ctrl+X".</summary>
    private string Describe(StudioCombo c, string? does = null)
    {
        var keys = string.Join(" + ", c.Positions.Select(p => StudioCodec.Label(Live(c.Layers.Count > 0 ? c.Layers.Min : 0, p)).Split('\n')[0]));
        var layers = c.Layers.Count == 0 ? "all layers" : string.Join(",", c.Layers.Select(l => LayerNames.ElementAtOrDefault(l) ?? $"{l}"));
        return $"{keys} → {does ?? StudioCodec.Label(c.Action).Replace('\n', '/')} · {layers}";
    }

    /// <summary>A combo's name in the list ("#2" without one), with a dot when it differs from the compiled one.</summary>
    private string ComboName(StudioCombo c, int i)
    {
        var defaults = Client.DefaultCombos;
        var changed = defaults.Count > 0 && (i >= defaults.Count || defaults[i] != Client.Combos[i]);
        return (c.Name.Length == 0 ? $"#{i}" : c.Name) + (changed ? " •" : "");
    }

    private void RefreshCombos()
    {
        _comboList.Children.Clear();
        if (!CombosEditable)
        {
            foreach (var c in Keymap.Combos)
            {
                var model = StudioCombo.Parse(CompiledComboText(c, "none"))!;
                _comboList.Children.Add(W.Text($"{c.Name}: {Describe(model, c.Does)}", 12.5));
            }
            _comboList.Children.Add(W.Hint(Client.Support == StudioSupport.Yes
                ? "This firmware can't edit combos (Studio 1): these are the compiled ones."
                : "Combos show once the target answers."));
            return;
        }
        var combos = Client.Combos;
        for (var i = 0; i < combos.Count; i++)
        {
            var index = i;
            if (StudioCombo.Parse(combos[i]) is not { } c) continue;
            _comboList.Children.Add(ComboRow(ComboName(c, i), Describe(c), () => EditCombo(index), selected: _combo != null && _comboIndex == i));
        }
        var max = Client.Version?.MaxCombos ?? 0;
        var add = new BenchButton("Add combo") { Enabled = max == 0 || combos.Count < max };
        add.Click += NewCombo;
        _comboList.Children.Add(W.Row(10, add, W.Hint($"{combos.Count} of {max}", 12)));
    }

    /// <summary>Edit combo <paramref name="index"/> of the latest list.</summary>
    internal void EditCombo(int index)
    {
        if (index < 0 || index >= Client.Combos.Count || StudioCombo.Parse(Client.Combos[index]) is not { } c) return;
        _combo = c;
        _comboIndex = index;
        _edit = StudioAction.Parse(c.Action);
        _message = null;
        _removeArmedAt = default;
        Refresh();
    }

    /// <summary>Start a new combo: pick its keys on the keyboard.</summary>
    internal void NewCombo()
    {
        _combo = new StudioCombo { Timeout = Keymap.ComboTimeoutMs, Idle = Keymap.ComboPriorIdleMs, Action = "kp:ESC" };
        _combo.Layers.Add(_layer);
        _comboIndex = null;
        _edit = StudioAction.Parse(_combo.Action);
        _message = null;
        Refresh();
    }

    /// <summary>Leave the combo editor.</summary>
    internal void LeaveCombo()
    {
        _combo = null;
        _comboIndex = null;
        _removeArmedAt = default;
        LoadEditor();
        Refresh();
    }

    /// <summary>A key clicked while editing a combo: in or out of its positions.</summary>
    internal void ToggleComboKey(int pos)
    {
        if (_combo == null) return;
        if (!_combo.Positions.Remove(pos)) _combo.Positions.Add(pos);
        _view.SetHighlighted(_combo.Positions);
        RefreshEditor();
    }

    internal StudioCombo? Combo => _combo;

    internal int? ComboIndex => _comboIndex;

    /// <summary>The combo text that Apply sends (its action from the action editor).</summary>
    internal string ComboText
    {
        get
        {
            _combo!.Action = _edit.Encode();
            return _combo.Encode();
        }
    }

    internal void CheckCombo()
    {
        if (_combo == null) return;
        var text = ComboText;
        _message = Client.ComboCheck(_comboIndex, text) ? ("checking…", "mut") : NotSent;
        RefreshEditor();
    }

    internal void ApplyCombo()
    {
        if (_combo == null) return;
        var text = ComboText;
        var sent = _comboIndex is { } i ? Client.ComboSet(i, text) : Client.ComboAdd(text);
        var verb = _comboIndex is { } j ? $"set {j}" : "add";
        _message = sent ? ($"sent: combo {verb} {text}", "mut") : NotSent;
        RefreshEditor();
    }

    /// <summary>Remove the combo: a second click within 4 s confirms.</summary>
    internal void RemoveCombo()
    {
        if (_comboIndex is not { } index) return;
        if (DateTime.UtcNow - _removeArmedAt < TimeSpan.FromSeconds(4))
        {
            _removeArmedAt = default;
            _message = Client.ComboRemove(index) ? ($"sent: combo remove {index}", "mut") : NotSent;
        }
        else
        {
            _removeArmedAt = DateTime.UtcNow;
            _message = ("Click Remove again to remove this combo.", "warn");
        }
        RefreshEditor();
    }

    private void RefreshComboEditor()
    {
        var combo = _combo!;
        var client = Client;
        var children = new List<Control>
        {
            W.Split(W.Lbl(_comboIndex is { } i ? $"Combo #{i}" : "New combo"), W.Lbl("Click keys on the keyboard")),
        };

        var name = new TextBox { Text = combo.Name, PlaceholderText = "name (A-Z a-z 0-9 _), or empty", FontSize = 13 };
        name.TextChanged += (_, _) =>
        {
            combo.Name = (name.Text ?? "").Trim();
            UpdateComboPreview();
        };
        children.Add(W.Lbl("Name"));
        children.Add(name);

        children.Add(W.Lbl("Keys"));
        children.Add(W.Text(combo.Positions.Count == 0
            ? "none yet: click 2 to 4 keys on the keyboard"
            : string.Join(" + ", combo.Positions.Select(p => $"{p} {StudioCodec.Label(Live(0, p)).Split('\n')[0]}")), 13));

        children.Add(W.Lbl("Active on"));
        children.Add(ComboLayers(combo));
        children.Add(W.Wrap(10, 6,
            W.Row(8, W.Lbl("Timeout ms"), NumberBox(combo.Timeout, t => combo.Timeout = t)),
            W.Row(8, W.Lbl("Prior idle ms"), NumberBox(combo.Idle, t => combo.Idle = t))));
        children.Add(W.Hint("All keys within the timeout of the first; prior idle: only if no key was typed that long before (0 = off)."));

        children.Add(Spaced(W.Lbl("Action")));
        AddActionEditor(children);

        _comboPreview = W.Mono("", 12.5);
        children.Add(Spaced(W.Lbl("Preview")));
        children.Add(_comboPreview);
        UpdateComboPreview();

        children.Add(ComboButtons(client));
        if (!client.Unlocked && client.Support == StudioSupport.Yes)
        {
            children.Add(W.Hint("Locked: Check works, but Add, Apply and Remove will be refused (err locked) until you press the Studio key."));
        }
        if (_message is { } m) children.Add(W.Text(m.Text, 13).Tok(TextBlock.ForegroundProperty, m.Tone));
        _editor.Content = W.Column(9, children.ToArray());
    }

    /// <summary>All layers, or the layers the combo is active on.</summary>
    private WrapPanel ComboLayers(StudioCombo combo)
    {
        var names = LayerNames;
        var layers = W.Wrap(5, 5);
        var all = new BenchButton("All layers", latching: true) { IsLatched = combo.Layers.Count == 0 };
        all.Click += () =>
        {
            combo.Layers.Clear();
            RefreshEditor();
        };
        layers.Children.Add(all);
        for (var l = 0; l < names.Count; l++)
        {
            var layer = l;
            var button = new BenchButton($"{l} {names[l]}", latching: true) { IsLatched = combo.Layers.Contains(l) };
            button.Click += () =>
            {
                if (!combo.Layers.Remove(layer)) combo.Layers.Add(layer);
                RefreshEditor();
            };
            layers.Children.Add(button);
        }
        return layers;
    }

    /// <summary>A box for a number of milliseconds; a valid number goes to <paramref name="set"/>.</summary>
    private TextBox NumberBox(int value, Action<int> set)
    {
        var box = new TextBox { Text = value.ToString(CultureInfo.InvariantCulture), Width = 90, FontSize = 13 };
        box.TextChanged += (_, _) =>
        {
            if (int.TryParse(box.Text, NumberStyles.Integer, CultureInfo.InvariantCulture, out var n)) set(n);
            UpdateComboPreview();
        };
        return box;
    }

    /// <summary>Check, Add or Apply, Remove (for an existing combo), Done.</summary>
    private WrapPanel ComboButtons(StudioClient client)
    {
        var ready = client.Support == StudioSupport.Yes;
        var check = new BenchButton("Check");
        check.Click += CheckCombo;
        var apply = new BenchButton(_comboIndex == null ? "Add" : "Apply", ButtonKind.Primary) { Enabled = ready };
        apply.Click += ApplyCombo;
        var row = W.Wrap(8, 8, check, apply);
        if (_comboIndex != null)
        {
            var remove = new BenchButton(_removeArmedAt != default ? "Click again: remove" : "Remove…", ButtonKind.Danger) { Enabled = ready };
            remove.Click += RemoveCombo;
            row.Children.Add(remove);
        }
        var done = new BenchButton("Done");
        done.Click += LeaveCombo;
        row.Children.Add(done);
        return row;
    }

    private void UpdateComboPreview()
    {
        if (_comboPreview == null || _combo == null) return;
        var canonical = StudioCodec.Canonical(_edit.Encode());
        _comboPreview.Text = canonical == null ? $"{_edit.Encode()}: not an action" : ComboText;
        _comboPreview.Tok(TextBlock.ForegroundProperty, canonical == null ? "crit" : "ink");
    }
}

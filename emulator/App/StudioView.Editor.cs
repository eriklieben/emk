using System.Globalization;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Layout;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>Studio's key editor: the selected key, its action editor and pickers, and the list rows.</summary>
public sealed partial class StudioView
{
    private void RefreshEditor()
    {
        if (_combo != null)
        {
            RefreshComboEditor();
            return;
        }
        var names = LayerNames;
        var live = Live(_layer, _key);
        var def = Default(_layer, _key);
        var changed = Changed(_layer, _key);
        var client = Client;

        var children = new List<Control>
        {
            W.Lbl($"Key {_key} · {(names.Count > _layer ? names[_layer] : _layer.ToString(CultureInfo.InvariantCulture))} · {Keymap.Where(_key)}"),
            W.Kv([
                ("Live", W.Row(8, W.Mono(live, 12), W.Hint(StudioCodec.Label(live).Replace('\n', '/'), 12))),
                ("Default", W.Row(8, W.Mono(def, 12, changed ? "mut" : "ink"), W.Hint(changed ? "changed" : "same", 12))),
            ], 80),
        };
        if (changed)
        {
            var revertKey = new BenchButton("Revert key to default") { Enabled = client.Unlocked };
            revertKey.Click += RevertKey;
            children.Add(revertKey);
        }

        AddActionEditor(children);

        // Preview.
        var encoding = _edit.Encode();
        var canonical = StudioCodec.Canonical(encoding);
        var legend = canonical != null ? StudioCodec.Label(canonical) : "";
        children.Add(Spaced(W.Lbl("Preview")));
        children.Add(W.Row(12, new KeyCap(legend), W.Column(2,
            W.Mono(canonical ?? $"{encoding}: not an action", 12.5, canonical != null ? "ink" : "crit"),
            W.Hint(PreviewNote(canonical, encoding, live, client)))));

        var check = new BenchButton("Check");
        check.Click += Check;
        var apply = new BenchButton("Apply", ButtonKind.Primary) { Enabled = canonical != null && client.Support == StudioSupport.Yes };
        apply.Click += Apply;
        children.Add(W.Row(8, check, apply));
        if (!client.Unlocked && client.Support == StudioSupport.Yes)
        {
            children.Add(W.Hint("Locked: Apply will be refused (err locked). Hold nav and press the Studio key on the keyboard" +
                                (_target == 0 ? ", or use Unlock above." : ".")));
        }
        if (_message is { } m) children.Add(W.Text(m.Text, 13).Tok(TextBlock.ForegroundProperty, m.Tone));
        _editor.Content = W.Column(9, children.ToArray());
    }

    /// <summary>Under the preview: whether it is the live binding already, else its length against the limit.</summary>
    private static string PreviewNote(string? canonical, string encoding, string live, StudioClient client)
    {
        if (canonical == null) return "";
        if (canonical == (StudioCodec.Canonical(live) ?? live)) return "same as the live binding";
        return $"{encoding.Length} of {client.Version?.Max ?? DefaultMaxEncoding} characters";
    }

    /// <summary>The longest encoding a target takes when it doesn't say (Studio 1).</summary>
    private const int DefaultMaxEncoding = 40;

    /// <summary>Behavior and its pickers for <c>_edit</c> (keys and combo actions share it).</summary>
    private void AddActionEditor(List<Control> children)
    {
        var names = LayerNames;
        // Behavior.
        children.Add(Spaced(W.Lbl("Behavior")));
        var kinds = new (StudioKind Kind, string Name)[]
        {
            (StudioKind.Key, "Key"), (StudioKind.ModTap, "Mod-tap"), (StudioKind.Layer, "Layer (mo)"), (StudioKind.CapsWord, "Caps word"),
            (StudioKind.System, "System"), (StudioKind.Transparent, "Transparent"), (StudioKind.None, "None"),
            (StudioKind.MouseMove, "Mouse move"), (StudioKind.MouseButton, "Mouse button"), (StudioKind.Scroll, "Scroll"),
            (StudioKind.Precision, "Precision"), (StudioKind.MouseOrKey, "Mouse in mouse mode, else key"),
        };
        // Mouse actions only for firmware that has them (caps=…,mouse).
        if (!MouseSupported) kinds = kinds.Where(k => !StudioAction.IsMouse(k.Kind) || k.Kind == _edit.Kind).ToArray();
        var kindRow = W.Wrap(5, 5);
        foreach (var (kind, label) in kinds)
        {
            var button = new BenchButton(label, latching: true) { IsLatched = _edit.Kind == kind };
            button.Click += () =>
            {
                _edit.Kind = kind;
                _message = null;
                RefreshEditor();
            };
            kindRow.Children.Add(button);
        }
        children.Add(kindRow);

        switch (_edit.Kind)
        {
            case StudioKind.Key:
                children.AddRange(KeyPicker("Key"));
                break;
            case StudioKind.ModTap:
                children.Add(W.Lbl("Hold"));
                children.Add(Choices(StudioAction.HoldKeys, _edit.Hold, v => _edit.Hold = v));
                children.AddRange(KeyPicker("Tap"));
                break;
            case StudioKind.Layer:
                children.Add(W.Lbl("Layer while held"));
                var layers = new Segmented(names.Select((n, i) => $"{i} {n}").ToArray(), _edit.Layer);
                layers.SelectionChanged += i =>
                {
                    _edit.Layer = i;
                    RefreshEditor();
                };
                children.Add(layers);
                break;
            case StudioKind.System:
                children.Add(W.Lbl("System action"));
                children.Add(Choices(StudioCodec.SystemNames, _edit.System, v => _edit.System = v));
                break;
            case StudioKind.MouseMove:
                children.Add(W.Lbl("Move the pointer"));
                children.Add(Choices(StudioAction.Directions, _edit.Direction, v => _edit.Direction = v));
                children.Add(W.Hint("While held. The first mouse-move press turns the pointer on (the MOUSE bar) until its layer is let go."));
                break;
            case StudioKind.Scroll:
                children.Add(W.Lbl("Scroll"));
                children.Add(Choices(StudioAction.Directions, _edit.Direction, v => _edit.Direction = v));
                children.Add(W.Hint("While held; left and right pan sideways."));
                break;
            case StudioKind.MouseButton:
                children.Add(W.Lbl("Button"));
                children.Add(Choices(StudioAction.Buttons, _edit.Button, v => _edit.Button = v));
                break;
            case StudioKind.Precision:
                children.Add(W.Hint("Slow, precise pointer movement while held."));
                break;
            case StudioKind.MouseOrKey:
                children.Add(W.Lbl("While the pointer is on"));
                children.Add(Choices(StudioAction.MouseOrKeyChoices, _edit.MouseAction, v => _edit.MouseAction = v));
                children.AddRange(KeyPicker("Otherwise this key"));
                break;
        }
    }

    /// <summary>Key search with results, and the modifier functions around it.</summary>
    private IEnumerable<Control> KeyPicker(string title)
    {
        yield return W.Split(W.Lbl(title), W.Mono(_edit.Key, 13));
        Responsive.Detach(_search);
        yield return _search;
        FillKeyResults();
        Responsive.Detach(_keyResults);
        yield return _keyResults;
        yield return W.Lbl("Modifiers around it");
        var mods = W.Wrap(5, 5);
        foreach (var mod in StudioAction.Modifiers)
        {
            var toggle = new BenchButton(mod, latching: true) { IsLatched = _edit.Mods.Contains(mod) };
            toggle.LatchedChanged += on =>
            {
                if (on) _edit.Mods.Add(mod);
                else _edit.Mods.Remove(mod);
                RefreshEditor();
            };
            mods.Children.Add(toggle);
        }
        yield return mods;
        yield return W.Hint("LC = left Ctrl, LS = left Shift, LA = left Alt, LG = left GUI; R… the right ones.");
    }

    private void FillKeyResults()
    {
        var query = (_search.Text ?? "").Trim().ToUpperInvariant();
        var matches = StudioCodec.KeyNames
            .Where(k => query.Length == 0 ? k.Length <= 2 || k == _edit.Key : k.Contains(query, StringComparison.Ordinal))
            .OrderBy(k => !k.StartsWith(query, StringComparison.Ordinal)).ThenBy(k => k.Length)
            .Take(48).ToList();
        if (!matches.Contains(_edit.Key)) matches.Insert(0, _edit.Key);
        _keyResults.Children.Clear();
        foreach (var key in matches)
        {
            var button = new BenchButton(key, latching: true) { IsLatched = key == _edit.Key };
            button.Click += () =>
            {
                _edit.Key = key;
                RefreshEditor();
            };
            _keyResults.Children.Add(button);
        }
    }

    private WrapPanel Choices(IReadOnlyList<string> values, string current, Action<string> pick)
    {
        var wrap = W.Wrap(5, 5);
        foreach (var value in values)
        {
            var button = new BenchButton(value, latching: true) { IsLatched = value.Equals(current, StringComparison.OrdinalIgnoreCase) };
            button.Click += () =>
            {
                pick(value);
                RefreshEditor();
            };
            wrap.Children.Add(button);
        }
        return wrap;
    }

    /// <summary>A small keycap with the preview legend.</summary>
    private sealed class KeyCap : Border
    {
        public KeyCap(string legend)
        {
            Width = 54;
            Height = 54;
            CornerRadius = new CornerRadius(4);
            BorderThickness = new Thickness(1);
            this.Tok(BackgroundProperty, "k-fill").Tok(BorderBrushProperty, "k-stroke");
            var lines = legend.Split('\n');
            var main = W.Head(lines[0], LegendSize(lines[0].Length));
            main.Tok(TextBlock.ForegroundProperty, "k-ink");
            main.HorizontalAlignment = HorizontalAlignment.Center;
            var stack = W.Column(0, main);
            if (lines.Length > 1)
            {
                var sub = W.Hint(lines[1], 9.5);
                sub.Tok(TextBlock.ForegroundProperty, "k-sub");
                sub.HorizontalAlignment = HorizontalAlignment.Center;
                stack.Children.Add(sub);
            }
            stack.VerticalAlignment = VerticalAlignment.Center;
            Child = stack;
        }

        private static double LegendSize(int length) => length switch
        {
            <= 2 => 19,
            <= 4 => 15,
            _ => 11,
        };
    }

    private const string ModeInk = "mode-ink";

    /// <summary>A layer in the list (CSS <c>.layer</c>), the chosen one dark.</summary>
    private static Border LayerRow(int index, string name, string description, bool on, Action click)
    {
        var n = W.Mono($"{index}", 11, on ? ModeInk : "mut");
        var title = W.Head(name, 14);
        title.Tok(TextBlock.ForegroundProperty, on ? ModeInk : "ink");
        var desc = W.Hint(description, 12);
        desc.Tok(TextBlock.ForegroundProperty, on ? ModeInk : "mut");
        desc.Opacity = on ? 0.75 : 1;
        desc.VerticalAlignment = VerticalAlignment.Center;
        var grid = new Grid { ColumnDefinitions = new ColumnDefinitions("Auto,*,Auto"), ColumnSpacing = 8 };
        SetColumn(title, 1);
        SetColumn(desc, 2);
        grid.Children.Add(n);
        grid.Children.Add(title);
        grid.Children.Add(desc);
        return Clickable(grid, on, click);
    }

    /// <summary>A combo in the list: its name over its keys and action (names can be long).</summary>
    private static Border ComboRow(string title, string detail, Action click, bool selected)
    {
        var head = W.Head(title, 14);
        var d = W.Hint(detail, 12);
        head.Tok(TextBlock.ForegroundProperty, selected ? ModeInk : "ink");
        d.Tok(TextBlock.ForegroundProperty, selected ? ModeInk : "mut");
        return Clickable(W.Column(1, head, d), selected, click);
    }

    private static Border ListRow(string title, string detail, Action click, bool selected = false)
    {
        var grid = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), ColumnSpacing = 8 };
        var d = W.Hint(detail, 12);
        d.VerticalAlignment = VerticalAlignment.Center;
        SetColumn(d, 1);
        var head = W.Head(title, 14);
        if (selected)
        {
            head.Tok(TextBlock.ForegroundProperty, ModeInk);
            d.Tok(TextBlock.ForegroundProperty, ModeInk);
        }
        grid.Children.Add(head);
        grid.Children.Add(d);
        return Clickable(grid, selected, click);
    }

    private static Border Clickable(Control content, bool on, Action click)
    {
        var border = new Border
        {
            Padding = new Thickness(10, 7),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(3),
            Child = content,
            Cursor = new Cursor(StandardCursorType.Hand),
        }.Tok(Border.BackgroundProperty, on ? "mode-bg" : "btn").Tok(Border.BorderBrushProperty, on ? "mode-bg" : "btn-edge");
        border.PointerPressed += (_, e) =>
        {
            if (!e.GetCurrentPoint(border).Properties.IsLeftButtonPressed) return;
            e.Handled = true;
            click();
        };
        return border;
    }
}

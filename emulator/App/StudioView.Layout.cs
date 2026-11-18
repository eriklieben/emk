using Avalonia;
using Avalonia.Controls;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>Studio's layout: strip, layers, keyboard tray and editor, wide or narrow.</summary>
public sealed partial class StudioView
{
    // ---- layout ---------------------------------------------------------------------

    private Control _strip = null!, _left = null!, _tray = null!, _right = null!;
    private int _columns = -1;

    private void BuildLayout()
    {
        var strip = W.Wrap(14, 8, _targetHost, _supportLamp, _lockLamp, _unlock, _lock, _dirtyLamp, _save, _revert, _reset, _reload, _poster);
        _strip = Strips.Conn(strip);

        _left = Strips.Side(new ScrollViewer
        {
            Content = W.Column(11,
                W.Lbl("Layers"), _layerList,
                Spaced(W.Lbl("Hold-taps")), _holdTaps,
                Spaced(W.Lbl("Combos")), _comboList),
        }, left: false);

        var header = new DockPanel();
        var chips = W.Wrap(6, 6, _changedChip, _transparentChip);
        DockPanel.SetDock(chips, Dock.Right);
        header.Children.Add(chips);
        header.Children.Add(_trayTitle);
        var hint = W.Hint("Click a key to edit it. Changed keys carry a dot; transparent keys show the layer below, muted.");
        _tray = MainWindow.KeyboardTray(header, _view, hint);

        _right = Strips.Side(new ScrollViewer { Content = _editor, HorizontalScrollBarVisibility = Avalonia.Controls.Primitives.ScrollBarVisibility.Disabled });
        Arrange(0);
    }

    private static Control Spaced(Control c)
    {
        c.Margin = new Thickness(0, 8, 0, 0);
        return c;
    }

    /// <summary>Wide: layers | keyboard | editor. Narrow: the keyboard first, then editor and layers side by side.</summary>
    public void Arrange(int narrowColumns)
    {
        var narrow = narrowColumns > 0;
        if (_columns >= 0 && (_columns > 0) == narrow) return;
        _columns = narrowColumns;
        foreach (var part in new[] { _strip, _left, _tray, _right }) Responsive.Detach(part);
        Children.Clear();
        RowDefinitions = new RowDefinitions("Auto,*");
        SetRow(_strip, 0);
        Children.Add(_strip);
        if (narrow)
        {
            ColumnDefinitions = new ColumnDefinitions("*");
            SetColumnSpan(_strip, 1);
            _tray.Height = 380;
            var columns = new Grid { ColumnDefinitions = new ColumnDefinitions("1.4*,*") };
            SetColumn(_right, 0);
            SetColumn(_left, 1);
            columns.Children.Add(_right);
            columns.Children.Add(_left);
            var below = new Border { BorderThickness = new Thickness(0, 1, 0, 0), Child = columns }.Tok(Border.BorderBrushProperty, "ln");
            var body = Responsive.Stack(_tray, below);
            SetRow(body, 1);
            Children.Add(body);
            return;
        }
        _tray.Height = double.NaN;
        ColumnDefinitions = new ColumnDefinitions("220,*,390");
        SetColumnSpan(_strip, 3);
        foreach (var (part, col) in new[] { (_left, 0), (_tray, 1), (_right, 2) })
        {
            SetRow(part, 1);
            SetColumn(part, col);
            Children.Add(part);
        }
    }
}

using Avalonia;
using Avalonia.Controls;

namespace KbEmulator.Bench;

/// <summary>
/// The window's width class: the wide layout from 1280 px (below that the
/// five rack modules and the top bar's lamps don't fit side by side); below it the
/// rack wraps into 3 columns (from 900 px) or 2, side panels move below and
/// panes stack.
/// </summary>
public static class Responsive
{
    public const double WideFrom = 1280;
    public const double ThreeColumnsFrom = 900;

    /// <summary>Rack columns in a narrow window; 0 = the wide layout.</summary>
    public static int NarrowColumns(double width) => width switch
    {
        >= WideFrom => 0,
        >= ThreeColumnsFrom => 3,
        _ => 2,
    };

    /// <summary>Take a control out of whatever holds it, to place it again.</summary>
    public static T Detach<T>(T control) where T : Control
    {
        switch (control.Parent)
        {
            case Panel panel:
                panel.Children.Remove(control);
                break;
            case Decorator decorator:
                decorator.Child = null;
                break;
            case ContentControl content:
                content.Content = null;
                break;
        }
        return control;
    }

    /// <summary>Modules in a grid of <paramref name="columns"/> with hairlines between them.</summary>
    public static Grid Wrap(IReadOnlyList<Control> modules, int columns, Thickness padding)
    {
        var grid = new Grid();
        for (var c = 0; c < columns; c++) grid.ColumnDefinitions.Add(new ColumnDefinition(GridLength.Star));
        for (var i = 0; i < modules.Count; i++)
        {
            if (i % columns == 0) grid.RowDefinitions.Add(new RowDefinition(GridLength.Auto));
            var lastInRow = i % columns == columns - 1 || i == modules.Count - 1;
            var cell = new Border
            {
                Padding = padding,
                BorderThickness = new Thickness(0, 0, lastInRow ? 0 : 1, 1),
                Child = Detach(modules[i]),
            }.Tok(Border.BorderBrushProperty, "ln");
            Grid.SetRow(cell, i / columns);
            Grid.SetColumn(cell, i % columns);
            grid.Children.Add(cell);
        }
        return grid;
    }

    /// <summary>Everything under each other, scrolling.</summary>
    public static ScrollViewer Stack(params Control[] parts)
    {
        var panel = new StackPanel();
        foreach (var p in parts) panel.Children.Add(Detach(p));
        return new ScrollViewer { Content = panel, HorizontalScrollBarVisibility = Avalonia.Controls.Primitives.ScrollBarVisibility.Disabled };
    }
}

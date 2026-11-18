//! The keymap as SVG, one picture per layer, drawn
//! from `kb_core::keymaps::corne` with the legends the app shows
//! (`kb_emu::action_label`), on the Corne outline of the app's `CorneView`
//! (same key pitch, column stagger, thumb fan and case margins). Light and
//! dark colours follow `prefers-color-scheme`, so it reads on GitHub in
//! either theme.
#![allow(
    clippy::many_single_char_names,
    reason = "drawing code: x, y, w, h, r are the clearest names"
)]

use std::fmt::Write;

use kb_core::Action;
use kb_core::keymaps::corne;

// CorneView's geometry (emulator/App/CorneView.cs).
const UNIT: f64 = 60.0;
const KEY: f64 = 54.0;
const PAD: f64 = 18.0;
const TOP: f64 = 52.0;
const OLED_X: f64 = 6.0 * UNIT + 4.0;
const OLED_Y: f64 = 4.0;
const OLED_W: f64 = 54.0;
const OLED_H: f64 = 198.0;
const HALF_WIDTH: f64 = OLED_X + OLED_W;
const GAP: f64 = 70.0;
const CASE_MARGIN: f64 = 10.0;
const STAGGER: [f64; 6] = [0.375, 0.375, 0.125, 0.0, 0.125, 0.25];
const THUMBS: [(f64, f64, f64); 3] = [(3.3, 3.42, 0.0), (4.4, 3.5, 7.0), (5.5, 3.68, 18.0)];

pub const WIDTH: f64 = 2.0 * PAD + 2.0 * HALF_WIDTH + GAP;
pub const HEIGHT: f64 = TOP + 328.0;

const STYLE: &str = r#"
  text { font-family: "Barlow Semi Condensed", "Roboto Condensed", "Arial Narrow", "Helvetica Neue", Arial, sans-serif; font-weight: 600; text-anchor: middle; dominant-baseline: central; }
  .title { fill: #1f2328; font-size: 17px; text-anchor: start; font-weight: 600; }
  .note { fill: #59636e; font-size: 13px; text-anchor: start; font-weight: 500; }
  .edge { fill: #121314; stroke: #121314; stroke-width: 3; }
  .plate { fill: #2a2c2e; }
  .bezel { fill: #141516; }
  .key { fill: #ededE7; stroke: #8f908a; stroke-width: 1; }
  .none { fill: #dcddd6; stroke: #8f908a; stroke-width: 1; stroke-dasharray: 3 3; }
  .held { fill: #2341c4; stroke: #2341c4; stroke-width: 1; }
  .ink { fill: #1a1b19; }
  .sub { fill: #6a6b65; font-weight: 500; }
  .mouse { fill: #2341c4; }
  .trans { fill: #a3a49c; }
  .held-ink { fill: #ffffff; }
  @media (prefers-color-scheme: dark) {
    .title { fill: #e6edf3; }
    .note { fill: #9198a1; }
    .edge { fill: #2e3542; stroke: #2e3542; }
    .plate { fill: #12161c; }
    .bezel { fill: #07090c; }
    .key { fill: #232934; stroke: #363e4c; }
    .none { fill: #1a1f27; stroke: #363e4c; }
    .held { fill: #a3e6f2; stroke: #a3e6f2; }
    .ink { fill: #d3d9e3; }
    .sub { fill: #6b7487; }
    .mouse { fill: #a3e6f2; }
    .trans { fill: #4f586a; }
    .held-ink { fill: #0f1318; }
  }
"#;

#[derive(Clone, Copy)]
struct KeyRect {
    x: f64,
    y: f64,
    rot: f64,
}

impl KeyRect {
    fn cx(&self) -> f64 {
        self.x + KEY / 2.0
    }
    fn cy(&self) -> f64 {
        self.y + KEY / 2.0
    }
}

fn half_origin(half: usize) -> f64 {
    PAD + if half == 0 { 0.0 } else { HALF_WIDTH + GAP }
}

/// Every key's place, as `CorneView` lays them out.
fn layout() -> [KeyRect; corne::KEYS] {
    let mut keys = [KeyRect {
        x: 0.0,
        y: 0.0,
        rot: 0.0,
    }; corne::KEYS];
    let left = |x: f64, y: f64, rot: f64| KeyRect {
        x: half_origin(0) + x,
        y: TOP + y,
        rot,
    };
    let right = |x: f64, y: f64, rot: f64| KeyRect {
        x: half_origin(1) + HALF_WIDTH - x - KEY,
        y: TOP + y,
        rot: -rot,
    };
    for row in 0..3 {
        for col in 0..6 {
            let (x, y) = (col as f64 * UNIT, (row as f64 + STAGGER[col]) * UNIT);
            keys[row * 12 + col] = left(x, y, 0.0);
            keys[row * 12 + 11 - col] = right(x, y, 0.0);
        }
    }
    for (i, &(x, y, rot)) in THUMBS.iter().enumerate() {
        keys[36 + i] = left(x * UNIT, y * UNIT, rot);
        keys[41 - i] = right(x * UNIT, y * UNIT, rot);
    }
    keys
}

fn is_left(pos: usize) -> bool {
    (36..=38).contains(&pos) || (pos < 36 && pos % 12 < 6)
}

#[allow(clippy::too_many_arguments)]
fn rect(
    out: &mut String,
    class: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
    rot: Option<(f64, f64, f64)>,
) {
    let transform = match rot {
        Some((deg, cx, cy)) if deg != 0.0 => {
            format!(r#" transform="rotate({deg} {cx:.1} {cy:.1})""#)
        }
        _ => String::new(),
    };
    let _ = writeln!(
        out,
        r#"<rect class="{class}" x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="{r}"{transform}/>"#
    );
}

/// The case of both halves: every key grown by the margin, the
/// controller/OLED block, and the board between the bottom row and the
/// thumbs; drawn once as the edge and once as the plate on top.
fn case(out: &mut String, keys: &[KeyRect]) {
    for class in ["edge", "plate"] {
        let _ = writeln!(out, r"<g>");
        for half in 0..2 {
            for (pos, k) in keys.iter().enumerate() {
                if is_left(pos) != (half == 0) {
                    continue;
                }
                rect(
                    out,
                    class,
                    k.x - CASE_MARGIN,
                    k.y - CASE_MARGIN,
                    KEY + 2.0 * CASE_MARGIN,
                    KEY + 2.0 * CASE_MARGIN,
                    8.0,
                    Some((k.rot, k.cx(), k.cy())),
                );
            }
            let (bx, by) = bezel(half);
            let (l, r) = if half == 0 {
                (6.0, CASE_MARGIN)
            } else {
                (CASE_MARGIN, 6.0)
            };
            rect(
                out,
                class,
                bx - l,
                by - CASE_MARGIN,
                OLED_W + l + r,
                OLED_H + CASE_MARGIN + 36.0,
                8.0,
                None,
            );
            let o = half_origin(half);
            let fill_x = THUMBS[0].0 * UNIT + 6.0;
            let (fw, fh) = (HALF_WIDTH - fill_x, THUMBS[0].1 * UNIT - 2.0 * UNIT - 20.0);
            let fx = if half == 0 { o + fill_x } else { o };
            rect(out, class, fx, TOP + 2.0 * UNIT + 40.0, fw, fh, 0.0, None);
        }
        let _ = writeln!(out, "</g>");
    }
    for half in 0..2 {
        let (bx, by) = bezel(half);
        rect(out, "bezel", bx, by, OLED_W, OLED_H, 4.0, None);
    }
}

fn bezel(half: usize) -> (f64, f64) {
    (
        half_origin(half) + if half == 0 { OLED_X } else { 0.0 },
        TOP + OLED_Y,
    )
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The legend of `action` and the class of its second line: hold-taps show
/// what holding does, mouse-or-key bindings what they do in mouse mode.
fn legend(action: Action) -> (String, &'static str, &'static str) {
    let label = kb_emu::action_label(action);
    match action {
        Action::Mouse(_) => (label, "mouse", "mouse"),
        Action::MouseOrKey { .. } => (label, "ink", "mouse"),
        _ => (label, "ink", "sub"),
    }
}

/// Font size for a main legend, as `CorneView` picks it, made smaller while
/// it would not fit the key in a plain sans-serif.
fn legend_size(text: &str) -> f64 {
    let n = text.chars().count();
    let mut size: f64 = match n {
        0..=2 => 19.0,
        3..=4 => 15.0,
        5..=6 => 12.5,
        _ => 10.5,
    };
    while n as f64 * size * 0.56 > KEY - 8.0 && size > 7.0 {
        size -= 0.5;
    }
    size
}

fn key(out: &mut String, k: &KeyRect, class: &str, label: &str, main: &str, sub: &str) {
    let open = if k.rot == 0.0 {
        "<g>".to_string()
    } else {
        format!(
            r#"<g transform="rotate({} {:.1} {:.1})">"#,
            k.rot,
            k.cx(),
            k.cy()
        )
    };
    let _ = writeln!(out, "{open}");
    let _ = writeln!(
        out,
        r#"<rect class="{class}" x="{:.1}" y="{:.1}" width="{KEY}" height="{KEY}" rx="4"/>"#,
        k.x + 0.5,
        k.y + 0.5
    );
    let mut lines = label.split('\n');
    let first = lines.next().unwrap_or("");
    match lines.next() {
        None if !first.is_empty() => {
            let _ = writeln!(
                out,
                r#"<text class="{main}" x="{:.1}" y="{:.1}" font-size="{}">{}</text>"#,
                k.cx(),
                k.cy(),
                legend_size(first),
                escape(first)
            );
        }
        None => {}
        Some(second) => {
            let _ = writeln!(
                out,
                r#"<text class="{main}" x="{:.1}" y="{:.1}" font-size="{}">{}</text>"#,
                k.cx(),
                k.cy() - 6.0,
                legend_size(first),
                escape(first)
            );
            let size = if second.chars().count() > 8 { 8.5 } else { 9.5 };
            let _ = writeln!(
                out,
                r#"<text class="{sub}" x="{:.1}" y="{:.1}" font-size="{size}">{}</text>"#,
                k.cx(),
                k.cy() + 12.0,
                escape(second)
            );
        }
    }
    let _ = writeln!(out, "</g>");
}

fn document(title: &str, note: &str, body: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH:.0}" height="{HEIGHT:.0}" viewBox="0 0 {WIDTH:.0} {HEIGHT:.0}" role="img" aria-label="{}">"#,
        escape(title)
    );
    let _ = writeln!(out, "<title>{}</title>", escape(title));
    let _ = writeln!(out, "<style>{STYLE}</style>");
    let _ = writeln!(
        out,
        r#"<text class="title" x="{PAD}" y="18">{}</text>"#,
        escape(title)
    );
    if !note.is_empty() {
        let _ = writeln!(
            out,
            r#"<text class="note" x="{PAD}" y="{:.0}">{}</text>"#,
            HEIGHT - 12.0,
            escape(note)
        );
    }
    out.push_str(body);
    out.push_str("</svg>\n");
    out
}

/// The keys that hold each layer: symbols 36, nav 41, wm both.
fn held_keys(layer: usize) -> &'static [usize] {
    match layer {
        1 => &[36],
        2 => &[41],
        3 => &[36, 41],
        _ => &[],
    }
}

/// The picture of `layer` (0 default, 1 symbols, 2 nav, 3 wm). Keys that
/// fall through show the default key, dimmed; keys that do nothing are dashed.
#[must_use]
pub fn layer(layer: usize) -> String {
    let keys = layout();
    let mut body = String::new();
    case(&mut body, &keys);
    for (pos, k) in keys.iter().enumerate() {
        let action = corne::LAYERS[layer][pos];
        if held_keys(layer).contains(&pos) {
            let label = kb_emu::action_label(corne::LAYERS[0][pos]);
            key(
                &mut body,
                k,
                "held",
                &format!("{label}\nheld"),
                "held-ink",
                "held-ink",
            );
            continue;
        }
        match action {
            Action::Trans => {
                let label = kb_emu::action_label(corne::LAYERS[0][pos]);
                key(&mut body, k, "key", &label, "trans", "trans");
            }
            Action::None => key(&mut body, k, "none", "", "ink", "sub"),
            _ => {
                let (label, main, sub) = legend(action);
                key(&mut body, k, "key", &label, main, sub);
            }
        }
    }
    let (title, note) = match layer {
        0 => (
            "default",
            "Second line: what the key does when held (Tab is Alt when held).",
        ),
        1 => (
            "symbols: hold the left inner thumb",
            "Dimmed keys fall through to the default layer.",
        ),
        2 => (
            "nav: hold the right outer thumb",
            "Accent legends are the mouse. Second line: what the key does in mouse mode (once the pointer moved).",
        ),
        _ => (
            "wm: hold symbols and nav together",
            "Hyprland / erikwm. WS n: go to workspace n; \u{2192} n: send the window there.",
        ),
    };
    document(title, note, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_shows_the_mouse_mode_legends_of_the_thumbs_and_scroll_keys() {
        // Arrange
        let nav = 2;

        // Act
        let svg = layer(nav);

        // Assert
        assert!(svg.contains(">Bksp</text>"));
        assert!(svg.contains(">L click</text>"));
        assert!(svg.contains("scroll ←"));
    }

    #[test]
    fn default_layer_shows_the_hold_of_the_tab_key() {
        // Arrange
        let default = 0;

        // Act
        let svg = layer(default);

        // Assert
        assert!(svg.contains(">Tab</text>"));
        assert!(svg.contains(">Alt</text>"));
    }
}

//! Status screens.
//!
//! Two styles:
//!
//! - [`Style::Zmk`]: ZMK's built-in status screen (`app/src/display/status_screen.c`),
//!   drawn landscape 128×32 like ZMK, so on the keyboard it reads sideways.
//!   Output status top-left, battery top-right, layer bottom-left, WPM bottom-right.
//!   ZMK uses LVGL's Montserrat font and symbol glyphs; these are look-alikes.
//! - [`Style::Portrait`]: upright 32×128 as seen while typing, rotated in
//!   software for each half (see [`Half`]).
//!
//! ```text
//! Portrait, left      Portrait, right
//! ┌─────┐             ┌─────┐
//! │≋1 ✓ │             │  ≋  │   ≋ = Bluetooth/link
//! │ 66% │             │ 92% │
//! │─────│             │─────│
//! │ SYM │             │     │   layer (inverted
//! │─────│             │     │   blinking: CAPS word)
//! │╎ ╎ ╷│             │  ✉  │
//! │╎╷ ╎╎│             │Build│   notification from
//! │ ╎ ╷╎│             │ok on│   the computer, word-
//! └─────┘             └─────┘   ╎ = digital rain
//! ```
#![allow(
    clippy::many_single_char_names,
    reason = "drawing code: x, y, w, h, t (ms) and the like are the clearest names"
)]

use core::fmt::Write;

use embedded_graphics::mono_font::ascii::{FONT_5X8, FONT_6X10, FONT_7X13, FONT_10X20};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};

use crate::anim::{
    Anim, CAPS_BLINK_MS, CONNECT_FLASH_MS, LAYER_SLIDE_MS, LOW_BATTERY_BLINK_MS, NOTE_CLEAR_MS,
    NOTE_DECODE_MS, PULSE_STEP_MS, SCROLL_PAUSE_MS, SCROLL_STEP_MS, battery_low, host_searching,
    link_in_progress, link_visible, mix, radio_visible, studio_visible,
};
use crate::frame::{Frame, HEIGHT, Half, PORTRAIT_HEIGHT, PORTRAIT_WIDTH, Portrait, WIDTH};
use crate::status::{Pairing, Status, Transport};
use crate::text::{StrBuf, wrap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// ZMK's built-in status screen, landscape (sideways on the keyboard).
    Zmk,
    /// Upright portrait screens for this firmware.
    Portrait,
}

// ---- icons (1-bit, `#` = lit) --------------------------------------------

#[rustfmt::skip]
const WIFI: &[&str] = &[
    "..#######..",
    ".#.......#.",
    "#..#####..#",
    "..#.....#..",
    "....###....",
    "...#...#...",
    ".....#.....",
    "....###....",
];

#[rustfmt::skip]
const CHECK: &[&str] = &[
    ".......#",
    "......##",
    ".....##.",
    "#...##..",
    "##.##...",
    ".###....",
    "..#.....",
];

#[rustfmt::skip]
const CLOSE: &[&str] = &[
    "#.....#",
    ".#...#.",
    "..#.#..",
    "...#...",
    "..#.#..",
    ".#...#.",
    "#.....#",
];

/// Open profile, ready to pair.
#[rustfmt::skip]
const PLUS: &[&str] = &[
    "...#...",
    "...#...",
    "...#...",
    "#######",
    "...#...",
    "...#...",
    "...#...",
];

/// Profile state next to its number: connected ✓, open (ready to pair) +,
/// taken by a computer that isn't connected ✗.
fn profile_mark(status: &Status) -> &'static [&'static str] {
    if status.profile_connected {
        CHECK
    } else if status.profile_open {
        PLUS
    } else {
        CLOSE
    }
}

/// On USB power (charging), in place of the `%`.
#[rustfmt::skip]
const BOLT: &[&str] = &[
    "...##",
    "..##.",
    ".##..",
    "#####",
    "..##.",
    ".##..",
    "##...",
];

/// Modifier names, in the order of the HID bits; Gui is Super, as in
/// Hyprland and erikwm. Five characters fit the 32-pixel width.
const MOD_NAMES: [&str; 4] = ["CTRL", "SHIFT", "ALT", "SUPER"];

#[rustfmt::skip]
const MAIL: &[&str] = &[
    "###########",
    "##.......##",
    "#.#.....#.#",
    "#..#...#..#",
    "#...###...#",
    "#.........#",
    "###########",
];

#[rustfmt::skip]
const KEYBOARD: &[&str] = &[
    "###########",
    "#.#.#.#.#.#",
    "#.........#",
    "#.#.#.#.#.#",
    "#.........#",
    "#..#####..#",
    "###########",
];

fn icon<D: DrawTarget<Color = BinaryColor>>(target: &mut D, x: i32, y: i32, rows: &[&str]) {
    let pixels = rows.iter().enumerate().flat_map(|(dy, row)| {
        row.bytes()
            .enumerate()
            .filter(|&(_, b)| b == b'#')
            .map(move |(dx, _)| Pixel(Point::new(x + dx as i32, y + dy as i32), BinaryColor::On))
    });
    let _ = target.draw_iter(pixels);
}

fn icon_width(rows: &[&str]) -> i32 {
    rows.first().map_or(0, |r| r.len() as i32)
}

fn text<D: DrawTarget<Color = BinaryColor>>(
    target: &mut D,
    s: &str,
    at: Point,
    font: &MonoFont<'_>,
    alignment: Alignment,
) {
    let style = MonoTextStyle::new(font, BinaryColor::On);
    let layout = TextStyleBuilder::new()
        .alignment(alignment)
        .baseline(Baseline::Top)
        .build();
    let _ = Text::with_text_style(s, at, style, layout).draw(target);
}

fn battery_text(pct: u8) -> StrBuf<5> {
    let mut s = StrBuf::new();
    let _ = write!(s, "{pct:>3}%");
    s
}

fn number(n: u16) -> StrBuf<6> {
    let mut s = StrBuf::new();
    let _ = write!(s, "{n}");
    s
}

/// A diagnostic screen: an inverted title bar and word-wrapped text, upright.
/// Used by the firmware for boot stages and to show where it crashed.
pub fn message(frame: &mut Frame, half: Half, title: &str, text: &str) {
    *frame = Frame::default();
    let mut p = Portrait { frame, half };
    let bar = Rectangle::new(Point::new(0, 0), Size::new(PORTRAIT_WIDTH as u32, 11));
    let _ = bar
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(&mut p);
    let style = MonoTextStyle::new(SMALL, BinaryColor::Off);
    let layout = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Top)
        .build();
    let _ = Text::with_text_style(title, Point::new(CENTER, 1), style, layout).draw(&mut p);
    wrap(text, NOTIFICATION_COLUMNS, 12, |i, line| {
        text_at(
            &mut p,
            line,
            Point::new(1, 14 + 9 * i as i32),
            &FONT_5X8,
            Alignment::Left,
        );
    });
}

fn text_at<D: DrawTarget<Color = BinaryColor>>(
    target: &mut D,
    s: &str,
    at: Point,
    font: &MonoFont<'_>,
    alignment: Alignment,
) {
    text(target, s, at, font, alignment);
}

/// Render `half`'s screen in `style` into a module frame, still (no animation).
pub fn render(frame: &mut Frame, half: Half, style: Style, status: &Status, layer_names: &[&str]) {
    render_animated(frame, half, style, status, layer_names, &Anim::still());
}

/// Render `half`'s screen at the moment `anim` describes (see [`crate::anim`]).
/// ZMK's look-alike screen stays still, like ZMK's.
pub fn render_animated(
    frame: &mut Frame,
    half: Half,
    style: Style,
    status: &Status,
    layer_names: &[&str],
    anim: &Anim,
) {
    *frame = Frame::default();
    match (style, half) {
        (Style::Zmk, Half::Left) => zmk_central(frame, status),
        (Style::Zmk, Half::Right) => zmk_peripheral(frame, status),
        (Style::Portrait, Half::Left) => {
            portrait_central(&mut Portrait { frame, half }, status, layer_names, anim);
        }
        (Style::Portrait, Half::Right) => {
            portrait_peripheral(&mut Portrait { frame, half }, status, anim);
        }
    }
}

// ---- ZMK built-in look-alike (landscape) -----------------------------------

const ZMK_FONT: &MonoFont<'static> = &FONT_7X13;
const ZMK_TOP: i32 = 0;
const ZMK_BOTTOM: i32 = HEIGHT as i32 - 13;
const RIGHT_EDGE: i32 = WIDTH as i32 - 1;

/// Output status widget: `≋ 1 ✓` (BLE) or `USB`.
fn zmk_output(frame: &mut Frame, status: &Status) {
    match status.transport {
        Transport::Usb => text(
            frame,
            "USB",
            Point::new(0, ZMK_TOP),
            ZMK_FONT,
            Alignment::Left,
        ),
        Transport::Ble => {
            icon(frame, 0, 3, WIFI);
            let x = icon_width(WIFI) + 3;
            text(
                frame,
                number(u16::from(status.profile) + 1).as_str(),
                Point::new(x, ZMK_TOP),
                ZMK_FONT,
                Alignment::Left,
            );
            let mark = profile_mark(status);
            let digits = number(u16::from(status.profile) + 1).as_str().len() as i32;
            icon(frame, x + 7 * digits + 3, 4, mark);
        }
    }
}

fn zmk_central(frame: &mut Frame, status: &Status) {
    if status.pairing != Pairing::Off {
        return zmk_pairing(frame, status.pairing);
    }
    zmk_output(frame, status);
    let battery = battery_text(status.battery[0]);
    text(
        frame,
        battery.as_str(),
        Point::new(RIGHT_EDGE, ZMK_TOP),
        ZMK_FONT,
        Alignment::Right,
    );

    icon(frame, 0, ZMK_BOTTOM + 4, KEYBOARD);
    let layer = number(u16::from(status.layer));
    text(
        frame,
        layer.as_str(),
        Point::new(icon_width(KEYBOARD) + 3, ZMK_BOTTOM),
        ZMK_FONT,
        Alignment::Left,
    );

    let wpm = number(status.wpm);
    text(
        frame,
        wpm.as_str(),
        Point::new(RIGHT_EDGE, ZMK_BOTTOM),
        ZMK_FONT,
        Alignment::Right,
    );
}

/// The pairing prompt on the landscape screen: `PAIR 12 4__` (typing) or
/// `PAIR 123456` (comparing) above the keys that answer it.
fn zmk_pairing(frame: &mut Frame, pairing: Pairing) {
    let digits = pair_digits(pairing);
    let mut top = StrBuf::<12>::new();
    let _ = write!(top, "PAIR {}", digits.as_str());
    text(
        frame,
        top.as_str(),
        Point::new(0, ZMK_TOP),
        ZMK_FONT,
        Alignment::Left,
    );
    let hint = match pairing {
        Pairing::Compare(_) => "SAME? ENTER / ESC",
        _ => "TYPE CODE, ENTER",
    };
    text(
        frame,
        hint,
        Point::new(0, ZMK_BOTTOM + 4),
        &FONT_5X8,
        Alignment::Left,
    );
}

/// The 6 digits of a pairing prompt: typed so far and `_` for the rest, or
/// the code to compare.
fn pair_digits(pairing: Pairing) -> StrBuf<6> {
    let mut s = StrBuf::<6>::new();
    match pairing {
        Pairing::Compare(code) => {
            let _ = write!(s, "{:06}", code % 1_000_000);
        }
        Pairing::TypeCode { typed, len } => {
            for (i, d) in typed.iter().enumerate() {
                let c = if i < len as usize {
                    (b'0' + d) as char
                } else {
                    '_'
                };
                let _ = s.write_char(c);
            }
        }
        Pairing::Off => {}
    }
    s
}

fn zmk_peripheral(frame: &mut Frame, status: &Status) {
    let link = if status.peripheral_connected {
        WIFI
    } else {
        CLOSE
    };
    icon(frame, 0, 3, link);
    let battery = battery_text(status.battery[1]);
    text(
        frame,
        battery.as_str(),
        Point::new(RIGHT_EDGE, ZMK_TOP),
        ZMK_FONT,
        Alignment::Right,
    );

    // Not in ZMK: the notification on the bottom line, as much as fits.
    if let Some(note) = &status.notification {
        icon(frame, 0, ZMK_BOTTOM + 4, MAIL);
        let mut line = StrBuf::<64>::new();
        let room = (WIDTH - 14) / 7;
        for c in note.as_str().chars().take(room) {
            let _ = line.write_char(c);
        }
        text(
            frame,
            line.as_str(),
            Point::new(14, ZMK_BOTTOM),
            ZMK_FONT,
            Alignment::Left,
        );
    }
}

// ---- upright portrait -------------------------------------------------------------

const SMALL: &MonoFont<'static> = &FONT_6X10;
const CENTER: i32 = PORTRAIT_WIDTH as i32 / 2;

fn separator<D: DrawTarget<Color = BinaryColor>>(target: &mut D, y: i32) {
    let line = Line::new(Point::new(2, y), Point::new(PORTRAIT_WIDTH as i32 - 3, y));
    let _ = line
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target);
}

/// The Bluetooth icon; while `searching` it pulses: the dot, then one arc
/// after the other.
fn wifi<D: DrawTarget<Color = BinaryColor>>(
    target: &mut D,
    x: i32,
    y: i32,
    searching: bool,
    anim: &Anim,
) {
    let first_row = if searching {
        [6, 4, 2, 0][anim.phase(PULSE_STEP_MS, 4) as usize]
    } else {
        0
    };
    icon(target, x, y + first_row as i32, &WIFI[first_row..]);
}

/// Invert rows `y0..y1` of the upright screen.
fn invert(p: &mut Portrait<'_>, y0: usize, y1: usize) {
    for y in y0..y1.min(PORTRAIT_HEIGHT) {
        for x in 0..PORTRAIT_WIDTH {
            let (mx, my) = p.half.module_xy(x, y);
            let on = p.frame.get(mx, my);
            p.frame.set(mx, my, !on);
        }
    }
}

/// Flash the connection area twice after connecting.
fn connect_flash(p: &mut Portrait<'_>, at: Option<u32>, anim: &Anim) {
    if let Some(elapsed) = anim.within(at, CONNECT_FLASH_MS)
        && (elapsed / (CONNECT_FLASH_MS / 4)).is_multiple_of(2)
    {
        invert(p, 0, 26);
    }
}

/// First three letters of a layer name, upper case: DEF / SYM / NAV.
fn layer_short(layer_names: &[&str], layer: u8) -> StrBuf<3> {
    let mut name = StrBuf::<3>::new();
    let full = layer_names.get(layer as usize).copied().unwrap_or("?");
    for c in full.chars().take(3) {
        let _ = name.write_char(c.to_ascii_uppercase());
    }
    name
}

/// This half's battery under the link icon: `66%`, or `66` and a bolt on
/// USB power. Low (15% or less, not charging), it blinks.
fn battery_line(p: &mut Portrait<'_>, status: &Status, half: usize, anim: &Anim) {
    let pct = status.battery[half];
    if battery_low(status, half) && anim.phase(LOW_BATTERY_BLINK_MS, 2) == 0 {
        return;
    }
    if status.charging[half] {
        let mut digits = StrBuf::<4>::new();
        let _ = write!(digits, "{pct:>3}");
        text(
            p,
            digits.as_str(),
            Point::new(4, 13),
            SMALL,
            Alignment::Left,
        );
        icon(p, 23, 14, BOLT);
    } else {
        text(
            p,
            battery_text(pct).as_str(),
            Point::new(CENTER, 13),
            SMALL,
            Alignment::Center,
        );
    }
}

/// Turn the pixels of rows `y0..y1` off.
fn clear_rows(p: &mut Portrait<'_>, y0: i32, y1: i32) {
    let area = Rectangle::new(
        Point::new(0, y0),
        Size::new(PORTRAIT_WIDTH as u32, (y1 - y0) as u32),
    );
    let _ = area
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
        .draw(p);
}

/// Held modifiers under the layer, over the rain: one inverted bar with
/// its name per held modifier (Ctrl, Shift, Alt, Super from top to bottom),
/// nothing when none is held. Left and right modifiers look the same.
fn modifiers(p: &mut Portrait<'_>, status: &Status) {
    let held = (status.mods | status.mods >> 4) & 0x0F;
    // The pointer first (the thumbs click while it shows), then the
    // modifiers held.
    let pointer = status.pointer_active.then_some("MOUSE");
    let mods = MOD_NAMES
        .iter()
        .enumerate()
        .filter(|&(i, _)| held & (1 << i) != 0)
        .map(|(_, &name)| name);
    let mut y = MODS_TOP;
    for name in pointer.into_iter().chain(mods) {
        // A blank line above and below, so the bar stands out from the rain.
        clear_rows(p, y - 1, y + MOD_BAR_HEIGHT + 1);
        let bar = Rectangle::new(
            Point::new(0, y),
            Size::new(PORTRAIT_WIDTH as u32, MOD_BAR_HEIGHT as u32),
        );
        let _ = bar
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(p);
        let style = MonoTextStyle::new(SMALL, BinaryColor::Off);
        let layout = TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Top)
            .build();
        let _ = Text::with_text_style(name, Point::new(CENTER, y), style, layout).draw(p);
        y += MOD_BAR_HEIGHT + 1;
    }
}

/// Height of a modifier bar (the small font is 10 pixels tall).
const MOD_BAR_HEIGHT: i32 = 10;

/// Top of the modifier row, just under the layer line.
const MODS_TOP: i32 = LAYER_LINE + 2;

/// Where the start-up animation ends, on its own clock (see [`boot_clock`]).
pub const BOOT_ART_MS: u32 = 3000;
/// Rain at least this long after power-on, also when the halves link sooner.
const BOOT_MIN_RAIN_MS: u32 = 700;
/// Without the other half, show "turn on the other half" after this long...
const BOOT_HINT_AFTER_MS: u32 = 2000;
/// ...and start anyway after this long (the left half alone still types).
pub const BOOT_WAIT_MAX_MS: u32 = 15_000;
/// Start-up phases: rain; the initial builds up out of it; it locks in and
/// throws off sparks; a scan line reveals the status screen.
const BOOT_BUILD_AT: u32 = 700;
const BOOT_LOCK_AT: u32 = 1500;
const BOOT_SPARKS_MS: u32 = 750;
const BOOT_REVEAL_AT: u32 = 2300;
const SPARK_COUNT: u32 = 16;
/// Pulls the sparks down (pixels a second, squared).
const SPARK_GRAVITY: i32 = 140;

/// Big initials (6 × 8 cells of 4 × 4 pixels: 24 × 32).
#[rustfmt::skip]
const BIG_E: [&str; 8] = [
    "######",
    "######",
    "##....",
    "#####.",
    "#####.",
    "##....",
    "######",
    "######",
];
#[rustfmt::skip]
const BIG_L: [&str; 8] = [
    "##....",
    "##....",
    "##....",
    "##....",
    "##....",
    "##....",
    "######",
    "######",
];
const BIG_CELL: i32 = 4;
const BIG_TOP: i32 = 46;

/// Spark directions (× 100), all around, 16 of them.
const SPARKS: [(i32, i32); 16] = [
    (100, 0),
    (92, 38),
    (71, 71),
    (38, 92),
    (0, 100),
    (-38, 92),
    (-71, 71),
    (-92, 38),
    (-100, 0),
    (-92, -38),
    (-71, -71),
    (-38, -92),
    (0, -100),
    (38, -92),
    (71, -71),
    (92, -38),
];

/// The start-up animation's own clock `since_power` ms after power-on, the
/// halves linked `since_linked` ms ago (`None`: not yet). `None` while it
/// still waits for the other half: rain (and a hint) until they link, so both
/// halves play the rest together. Gives up waiting after [`BOOT_WAIT_MAX_MS`].
#[must_use]
pub fn boot_clock(since_power: u32, since_linked: Option<u32>) -> Option<u32> {
    let linked_at = match since_linked {
        Some(ago) => since_power.saturating_sub(ago),
        None if since_power >= BOOT_WAIT_MAX_MS => BOOT_WAIT_MAX_MS,
        None => return None,
    };
    let start = linked_at.max(BOOT_MIN_RAIN_MS);
    since_power.checked_sub(start).map(|t| t + BOOT_BUILD_AT)
}

/// Whether the start-up animation is over (`boot_clock`'s arguments).
#[must_use]
pub fn boot_done(since_power: u32, since_linked: Option<u32>) -> bool {
    boot_clock(since_power, since_linked).is_some_and(|t| t >= BOOT_ART_MS)
}

/// The start-up screen `since_power` ms after power-on, the halves linked
/// `since_linked` ms ago (`None`: not yet): rain while waiting for the other
/// half (with a hint to switch it on), then [`boot_art`] on both together.
pub fn boot_frame(
    frame: &mut Frame,
    half: Half,
    since_power: u32,
    since_linked: Option<u32>,
    initial: &str,
    status_frame: &Frame,
) {
    if let Some(clock) = boot_clock(since_power, since_linked) {
        boot_art(frame, half, clock, since_power, initial, status_frame);
    } else {
        *frame = Frame::default();
        let mut p = Portrait { frame, half };
        full_rain(&mut p, since_power, half);
        if since_power >= BOOT_HINT_AFTER_MS {
            other_half_hint(&mut p, half, since_power);
        }
    }
}

/// "TURN ON RIGHT ->" (or "<- LEFT") in a box over the rain: the other half
/// isn't there yet.
fn other_half_hint(p: &mut Portrait<'_>, half: Half, t: u32) {
    let (top, height): (i32, u32) = (44, 40);
    clear_box(p, 0, top, PORTRAIT_WIDTH as u32, height);
    let _ = Rectangle::new(Point::new(0, top), Size::new(PORTRAIT_WIDTH as u32, height))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(p);
    let other = match half {
        Half::Left => "RIGHT",
        Half::Right => "LEFT",
    };
    for (i, line) in ["TURN", "ON", other].iter().enumerate() {
        text(
            p,
            line,
            Point::new(CENTER, top + 3 + 9 * i as i32),
            &FONT_5X8,
            Alignment::Center,
        );
    }
    // An arrow toward the other half, blinking gently.
    if (t / 500).is_multiple_of(2) {
        let y = top + height as i32 - 7;
        let (tip, back) = match half {
            Half::Left => (CENTER + 6, CENTER - 6),
            Half::Right => (CENTER - 6, CENTER + 6),
        };
        let dir = if tip > back { -1 } else { 1 };
        let _ = Line::new(Point::new(back, y), Point::new(tip, y))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(p);
        for k in 1..=3 {
            let x = tip + dir * k;
            let _ = Line::new(Point::new(x, y - k), Point::new(x, y + k))
                .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(p);
        }
    }
}

/// Start-up animation at `elapsed` ms of its own clock, `rain_ms` after
/// power-on (keeps the rain moving smoothly): dense digital rain; `initial` (E on
/// the left, L on the right) builds up out of the rain cell by cell, locks
/// in: the rain stops and sparks fly off it; then a scan line sweeping down
/// reveals `status_frame` (the first status screen) above it.
pub fn boot_art(
    frame: &mut Frame,
    half: Half,
    elapsed: u32,
    rain_ms: u32,
    initial: &str,
    status_frame: &Frame,
) {
    *frame = Frame::default();
    let mut p = Portrait { frame, half };
    // The rain stops once the letter locks in, so the sparks read clearly.
    if elapsed < BOOT_LOCK_AT {
        full_rain(&mut p, rain_ms, half);
    }

    let cells = match initial.chars().next().map(|c| c.to_ascii_uppercase()) {
        Some('L') => &BIG_L,
        _ => &BIG_E,
    };
    if elapsed >= BOOT_BUILD_AT {
        build_initial(&mut p, cells, elapsed - BOOT_BUILD_AT);
    }
    if elapsed >= BOOT_LOCK_AT {
        sparks(&mut p, elapsed - BOOT_LOCK_AT);
    }

    // The scan line reveals the status screen from the top.
    if elapsed >= BOOT_REVEAL_AT {
        let span = BOOT_ART_MS - BOOT_REVEAL_AT;
        let line =
            ((elapsed - BOOT_REVEAL_AT).min(span) * (PORTRAIT_HEIGHT as u32 + 2) / span) as usize;
        for y in 0..line.min(PORTRAIT_HEIGHT) {
            for x in 0..PORTRAIT_WIDTH {
                let (mx, my) = half.module_xy(x, y);
                p.frame.set(mx, my, status_frame.get(mx, my));
            }
        }
        for y in line..(line + 2).min(PORTRAIT_HEIGHT) {
            for x in 0..PORTRAIT_WIDTH {
                let (mx, my) = half.module_xy(x, y);
                p.frame.set(mx, my, true);
            }
        }
    }
}

/// The start-up initial (`cells`), `since` ms into its build: a dark
/// backdrop, then cell by cell out of the rain.
fn build_initial(p: &mut Portrait<'_>, cells: &[&str; 8], since: u32) {
    let x0 = (PORTRAIT_WIDTH as i32 - 6 * BIG_CELL) / 2;
    // A dark backdrop so the initial stands out of the rain.
    clear_box(
        p,
        x0 - 2,
        BIG_TOP - 2,
        6 * BIG_CELL as u32 + 4,
        8 * BIG_CELL as u32 + 4,
    );
    let build = BOOT_LOCK_AT - BOOT_BUILD_AT;
    for (r, row) in cells.iter().enumerate() {
        for (c, b) in row.bytes().enumerate() {
            if b != b'#' {
                continue;
            }
            // Each cell lands at its own moment, like a drop catching.
            let lands = mix((r * 8 + c) as u32 + 77) % build;
            if since < lands {
                continue;
            }
            let (x, y) = (x0 + c as i32 * BIG_CELL, BIG_TOP + r as i32 * BIG_CELL);
            // Just landed: a hollow outline for a moment, then solid.
            let style = if since < lands + 80 {
                PrimitiveStyle::with_stroke(BinaryColor::On, 1)
            } else {
                PrimitiveStyle::with_fill(BinaryColor::On)
            };
            let _ = Rectangle::new(
                Point::new(x, y),
                Size::new(BIG_CELL as u32, BIG_CELL as u32),
            )
            .into_styled(style)
            .draw(p);
        }
    }
}

/// Sparks and twinkles around the start-up initial, `t` ms after it locked in.
fn sparks(p: &mut Portrait<'_>, t: u32) {
    let (cx, cy) = (CENTER, BIG_TOP + 4 * BIG_CELL);
    // Sparks: glowing streaks flying off the letter's edges, arcing down
    // under gravity and dying out one by one.
    for k in 0..SPARK_COUNT {
        let h = mix(k + 991);
        let (dx, dy) = SPARKS[k as usize % SPARKS.len()];
        let life = 280 + (h >> 8) % 360; // ms
        if t >= life {
            continue;
        }
        let speed = 45 + (h % 55) as i32; // pixels a second
        let at = |ms: i32| -> Point {
            let ox = cx + dx * 13 / 100;
            let oy = cy + dy * 17 / 100;
            let run = speed * ms / 1000;
            let fall = SPARK_GRAVITY * ms * ms / 2_000_000;
            Point::new(ox + dx * run / 100, oy + dy * run / 100 + fall)
        };
        let head = at(t as i32);
        if t * 10 < life * 6 {
            // Bright: a short streak behind the head.
            let tail = at((t as i32 - 45).max(0));
            let _ = Line::new(tail, head)
                .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(p);
        } else if (t / 50 + k).is_multiple_of(2) {
            // Dying: a flickering ember.
            let _ = Pixel(head, BinaryColor::On).draw(p);
        }
    }
    // Twinkles left around the letter for a moment.
    if (150..BOOT_SPARKS_MS).contains(&t) {
        for k in 0..6u32 {
            let h = mix(k * 131 + 7);
            if !(t / 90 + k).is_multiple_of(3) {
                continue;
            }
            let x = 1 + (h % 30) as i32;
            let y = BIG_TOP - 8 + ((h >> 8) % (8 * BIG_CELL as u32 + 16)) as i32;
            let pixels = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
                .map(|(ox, oy)| Pixel(Point::new(x + ox, y + oy), BinaryColor::On));
            let _ = p.draw_iter(pixels);
        }
    }
}

/// Rain glyph `index` with its top-left at `(x, y)`; `dim` dithers it (a
/// drop's fading trail).
fn rain_glyph<D: DrawTarget<Color = BinaryColor>>(
    p: &mut D,
    x: i32,
    y: i32,
    index: usize,
    dim: bool,
) {
    let glyph = RAIN_GLYPHS[index % RAIN_GLYPHS.len()];
    let pixels = glyph.iter().enumerate().flat_map(|(dy, bits)| {
        (0..3).filter_map(move |dx| {
            let (px, py) = (x + dx, y + dy as i32);
            let on = bits & (0b100 >> dx) != 0 && (!dim || (px + py) % 2 == 0);
            on.then_some(Pixel(Point::new(px, py), BinaryColor::On))
        })
    });
    let _ = p.draw_iter(pixels);
}

/// Turn a box of pixels off.
fn clear_box<D: DrawTarget<Color = BinaryColor>>(p: &mut D, x: i32, y: i32, w: u32, h: u32) {
    let _ = Rectangle::new(Point::new(x, y), Size::new(w, h))
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
        .draw(p);
}

/// Dense digital rain over the whole screen at `t` ms: every column falls at
/// its own speed, with a dithered tail (start-up animation).
fn full_rain<D: DrawTarget<Color = BinaryColor>>(p: &mut D, t: u32, half: Half) {
    thinning_rain(p, t, half, 100);
}

/// [`full_rain`] with only `keep` percent of its glyphs (each glyph keeps
/// its place in the order, so lowering `keep` makes them go out one by one).
fn thinning_rain<D: DrawTarget<Color = BinaryColor>>(p: &mut D, t: u32, half: Half, keep: u32) {
    let rows = PORTRAIT_HEIGHT as i32 / RAIN_PITCH_Y + 1;
    let salt = if half == Half::Left { 0 } else { 0x55 };
    for c in 0..8u32 {
        let h = mix(c * 7919 + 13 + salt);
        let speed = 18 + (h % 14); // rows a second
        let trail = 6 + (h >> 8) % 8;
        let cycle = rows as u32 + trail + (h >> 12) % 6;
        let head = (t * speed / 1000 + (h >> 16) % cycle) % cycle;
        let x0 = 1 + RAIN_PITCH_X * c as i32;
        for k in 0..trail {
            let r = head as i32 - k as i32;
            if !(0..rows).contains(&r) || mix(h ^ ((r as u32) * 97)) % 100 >= keep {
                continue;
            }
            let glyph =
                RAIN_GLYPHS[(mix(h ^ (r as u32) << 8 ^ (t / 120).wrapping_add(k)) & 15) as usize];
            let dim = k > 0 && k + 3 >= trail;
            let y0 = RAIN_PITCH_Y * r;
            let pixels = glyph.iter().enumerate().flat_map(|(dy, bits)| {
                (0..3).filter_map(move |dx| {
                    let (x, y) = (x0 + dx, y0 + dy as i32);
                    let on = bits & (0b100 >> dx) != 0 && (!dim || (x + y) % 2 == 0);
                    on.then_some(Pixel(Point::new(x, y), BinaryColor::On))
                })
            });
            let _ = p.draw_iter(pixels);
        }
    }
}

/// Power-down timing (the whole takes `anim::SLEEP_ART_MS`): the columns
/// start falling within the first `SLEEP_STAGGER_MS`, everything has left
/// through the bottom by `SLEEP_FALL_END`, then the cursor blinks.
const SLEEP_STAGGER_MS: u32 = 350;
const SLEEP_FALL_END: u32 = 2000;
/// Dim rain glyphs trailing each falling edge.
const SLEEP_TRAIL: i32 = 3;

/// How far column `x` has dropped `elapsed` ms into the power-down: a
/// nudge of 25 px a second that speeds up, ~100 px after 1.55 s. Its top
/// edge, eating into it, goes half as fast again (~150 px: the screen plus
/// the rain trail), so even the last column to go is out before the cursor.
fn sleep_fall(x: usize, elapsed: u32) -> i32 {
    let delay = mix(x as u32 / 2 + 501) % SLEEP_STAGGER_MS;
    let t = i64::from(elapsed.saturating_sub(delay));
    (t / 40 + t * t / 39_000) as i32
}

/// The rain riding lane `lane`'s falling top edge `elapsed` ms into the
/// power-down: a bright glyph and a dim trail, each keeping its shape on the
/// way down. Yields the lit pixels from the head up, those past the bottom
/// included (so the trail doesn't light up again as the head leaves), those
/// still above the top not.
fn sleep_rain(lane: usize, elapsed: u32) -> impl Iterator<Item = (i32, i32)> {
    let x0 = lane as i32 * RAIN_PITCH_X + 1;
    let fall = sleep_fall(x0 as usize, elapsed);
    let edge = fall + fall / 2;
    (0..=SLEEP_TRAIL).flat_map(move |k| {
        let y0 = edge - 6 - k * RAIN_PITCH_Y;
        let glyph = RAIN_GLYPHS[(mix(x0 as u32 ^ (k as u32) << 8) & 15) as usize];
        (0..4).flat_map(move |dy| {
            (0..3).filter_map(move |dx| {
                let on = glyph[dy as usize] & (0b100 >> dx) != 0 && (k == 0 || (dx + dy) % 2 == 0);
                (on && y0 + dy >= 0).then_some((x0 + dx, y0 + dy))
            })
        })
    })
}

/// The animation before the screens switch off when the keyboard rests,
/// `elapsed` ms in: the screen `from` melts down column by column and falls
/// off the bottom, rain glyphs (with dim trails) riding each falling edge
/// out with it; nothing comes in behind, and once the screen is empty a
/// last block cursor blinks before the screen goes dark.
pub fn sleep_art(frame: &mut Frame, half: Half, elapsed: u32, from: &Frame) {
    *frame = Frame::default();
    let mut p = Portrait { frame, half };
    let h = PORTRAIT_HEIGHT as i32;
    if elapsed < SLEEP_FALL_END {
        for lane in 0..PORTRAIT_WIDTH / RAIN_PITCH_X as usize {
            // Each column drops at its own moment, speeding up as it falls,
            // while its top dissolves into the lane's rain: each pixel of
            // the rain is one taken from there, so nothing is ever added.
            let columns = lane * RAIN_PITCH_X as usize..(lane + 1) * RAIN_PITCH_X as usize;
            let mut eaten = 0;
            for x in columns.clone() {
                let fall = sleep_fall(x, elapsed);
                for y in 0..(h - fall).max(0) as usize {
                    let (mx, my) = half.module_xy(x, y);
                    if !from.get(mx, my) {
                        continue;
                    }
                    if y < fall as usize / 2 {
                        eaten += 1;
                    } else {
                        let (tx, ty) = half.module_xy(x, y + fall as usize);
                        p.frame.set(tx, ty, true);
                    }
                }
            }
            for (x, y) in sleep_rain(lane, elapsed) {
                if eaten == 0 {
                    break;
                }
                if y >= h {
                    eaten -= 1;
                    continue;
                }
                let (mx, my) = half.module_xy(x as usize, y as usize);
                if !p.frame.get(mx, my) {
                    p.frame.set(mx, my, true);
                    eaten -= 1;
                }
            }
            // What the rain has no room for yet falls on with its column.
            for x in columns {
                let fall = sleep_fall(x, elapsed);
                for y in (0..(fall as usize / 2).min((h - fall).max(0) as usize)).rev() {
                    let (mx, my) = half.module_xy(x, y);
                    if eaten > 0 && from.get(mx, my) {
                        eaten -= 1;
                        let (tx, ty) = half.module_xy(x, y + fall as usize);
                        p.frame.set(tx, ty, true);
                    }
                }
            }
        }
    } else if ((elapsed - SLEEP_FALL_END) / 150).is_multiple_of(2) {
        // The last cursor, blinking.
        let _ = Rectangle::new(Point::new(CENTER - 3, h / 2 - 5), Size::new(6, 10))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut p);
    }
}

fn portrait_central(p: &mut Portrait<'_>, status: &Status, layer_names: &[&str], anim: &Anim) {
    if status.pairing != Pairing::Off {
        return pairing(p, status.pairing);
    }
    // Connection
    match status.transport {
        Transport::Usb => text(p, "USB", Point::new(CENTER, 0), SMALL, Alignment::Center),
        Transport::Ble => {
            wifi(p, 0, 1, host_searching(status), anim);
            // Profiles 1-10: two digits need the narrow font to leave room for
            // the check mark on the 32-pixel-wide screen.
            let profile = number(u16::from(status.profile) + 1);
            let (font, width) = if profile.as_str().len() > 1 {
                (&FONT_5X8, 5)
            } else {
                (SMALL, 6)
            };
            text(
                p,
                profile.as_str(),
                Point::new(12, 1),
                font,
                Alignment::Left,
            );
            let digits = profile.as_str().len() as i32;
            icon(p, 12 + width * digits + 1, 1, profile_mark(status));
        }
    }
    battery_line(p, status, 0, anim);
    connect_flash(p, anim.connected_at, anim);
    separator(p, 26);

    // Layer between two lines; the new name slides in from the right,
    // pushing the old one out.
    let name = layer_short(layer_names, status.layer);
    let mut x = CENTER;
    if let Some(elapsed) = anim.within(anim.layer_at, LAYER_SLIDE_MS) {
        // Ease out: fast at first, settling at the end.
        let left = (LAYER_SLIDE_MS - elapsed) as i32;
        let offset = PORTRAIT_WIDTH as i32 * left * left / (LAYER_SLIDE_MS * LAYER_SLIDE_MS) as i32;
        x += offset;
        let old = layer_short(layer_names, anim.layer_from);
        text(
            p,
            old.as_str(),
            Point::new(x - PORTRAIT_WIDTH as i32, LAYER_TOP),
            &FONT_7X13,
            Alignment::Center,
        );
    }
    text(
        p,
        name.as_str(),
        Point::new(x, LAYER_TOP),
        &FONT_7X13,
        Alignment::Center,
    );

    // CAPS word: the layer row, inverted and then outlined, gently alternating.
    if status.caps_word {
        if anim.phase(CAPS_BLINK_MS, 2) == 1 {
            invert(p, LAYER_ROW.0, LAYER_ROW.1);
        } else {
            let frame = Rectangle::new(
                Point::new(0, LAYER_ROW.0 as i32),
                Size::new(PORTRAIT_WIDTH as u32, (LAYER_ROW.1 - LAYER_ROW.0) as u32),
            );
            let _ = frame
                .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(p);
        }
    }
    separator(p, LAYER_LINE);

    digital_rain(p, status, anim, RAIN_TOP);
    modifiers(p, status);
    link_detail(p, status, anim);
    radio_power(p, status, anim);
    studio_lock(p, status, anim);
}

/// Layer name, the line under it, and the rows CAPS word marks (between the lines).
const LAYER_TOP: i32 = 30;
const LAYER_LINE: i32 = 45;
const LAYER_ROW: (usize, usize) = (28, 44);

/// Made-up 3×4 glyphs for the rain (rows top to bottom, bit 2 = left pixel).
const RAIN_GLYPHS: [[u8; 4]; 16] = [
    [0b111, 0b001, 0b011, 0b001],
    [0b101, 0b111, 0b001, 0b001],
    [0b010, 0b111, 0b010, 0b011],
    [0b111, 0b100, 0b110, 0b100],
    [0b011, 0b010, 0b111, 0b010],
    [0b100, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b001, 0b011],
    [0b001, 0b011, 0b101, 0b001],
    [0b110, 0b010, 0b011, 0b110],
    [0b111, 0b010, 0b010, 0b111],
    [0b101, 0b010, 0b101, 0b010],
    [0b011, 0b100, 0b011, 0b100],
    [0b111, 0b001, 0b111, 0b001],
    [0b100, 0b110, 0b101, 0b100],
    [0b010, 0b101, 0b111, 0b101],
    [0b110, 0b101, 0b110, 0b100],
];
const RAIN_TOP: i32 = LAYER_LINE + 3;
const RAIN_PITCH_X: i32 = 4;
const RAIN_PITCH_Y: i32 = 5;

/// Digital rain from row `top` down: each recent key press drops a trail of
/// made-up glyphs that falls out of the area, its tail dithered (dimmer).
fn digital_rain(p: &mut Portrait<'_>, status: &Status, anim: &Anim, top: i32) {
    let bottom = if link_visible(status, anim) {
        116
    } else {
        PORTRAIT_HEIGHT as i32
    };
    let rows = (bottom - top + 1) / RAIN_PITCH_Y;
    // Glyphs in the trails change a few times a second.
    let epoch = anim.now / 150;
    for drop in anim.drops.iter().flatten() {
        let (c, trail) = (drop.column(), drop.trail());
        let head = drop.head(anim.now) as i32;
        let x0 = 1 + RAIN_PITCH_X * c as i32;
        for k in 0..trail {
            let r = head - k as i32;
            if !(0..rows).contains(&r) {
                continue;
            }
            let glyph = RAIN_GLYPHS
                [(mix(drop.seed ^ (r as u32) << 8 ^ epoch.wrapping_add(k)) & 15) as usize];
            let dim = k > 0 && k + 2 >= trail;
            let y0 = top + RAIN_PITCH_Y * r;
            let pixels = glyph.iter().enumerate().flat_map(|(dy, bits)| {
                (0..3).filter_map(move |dx| {
                    let (x, y) = (x0 + dx, y0 + dy as i32);
                    let on = bits & (0b100 >> dx) != 0 && (!dim || (x + y) % 2 == 0);
                    on.then_some(Pixel(Point::new(x, y), BinaryColor::On))
                })
            });
            let _ = p.draw_iter(pixels);
        }
    }
}

/// The split-link state at the bottom of the screen, if set; while the link
/// is on its way, a short line sweeps back and forth above it.
fn link_detail(p: &mut Portrait<'_>, status: &Status, anim: &Anim) {
    if !link_visible(status, anim) {
        return;
    }
    text(
        p,
        status.link_detail,
        Point::new(CENTER, 119),
        &FONT_5X8,
        Alignment::Center,
    );
    if anim.live && link_in_progress(status.link_detail) {
        const LEN: u32 = 6;
        let span = PORTRAIT_WIDTH as u32 - LEN;
        let step = (anim.now / 25) % (2 * span);
        let x = if step < span { step } else { 2 * span - step } as i32;
        let line = Line::new(Point::new(x, 117), Point::new(x + LEN as i32 - 1, 117));
        let _ = line
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(p);
    }
}

/// The pairing prompt on the left screen, instead of everything else: an
/// inverted `PAIR` title (as [`message`]), the 6 digits big in two rows of
/// three, and what to do. Typing the computer's code: the digits typed so
/// far, `_` for the rest, then Enter. Comparing: Enter if the computer shows
/// the same code, Escape if not.
fn pairing(p: &mut Portrait<'_>, pairing: Pairing) {
    let bar = Rectangle::new(Point::new(0, 0), Size::new(PORTRAIT_WIDTH as u32, 11));
    let _ = bar
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(p);
    let style = MonoTextStyle::new(SMALL, BinaryColor::Off);
    let layout = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Top)
        .build();
    let _ = Text::with_text_style("PAIR", Point::new(CENTER, 1), style, layout).draw(p);

    let comparing = matches!(pairing, Pairing::Compare(_));
    let top: &[&str] = if comparing {
        &["SAME", "CODE?"]
    } else {
        &["TYPE", "CODE"]
    };
    for (i, line) in top.iter().enumerate() {
        text(
            p,
            line,
            Point::new(CENTER, 14 + 9 * i as i32),
            &FONT_5X8,
            Alignment::Center,
        );
    }

    let digits = pair_digits(pairing);
    let (first, second) = digits.as_str().split_at(3);
    text(
        p,
        first,
        Point::new(CENTER, PAIR_DIGITS_TOP),
        &FONT_10X20,
        Alignment::Center,
    );
    text(
        p,
        second,
        Point::new(CENTER, PAIR_DIGITS_TOP + 22),
        &FONT_10X20,
        Alignment::Center,
    );
    separator(p, PAIR_HINT_TOP - 5);

    let lines = if comparing {
        ["ENTER", "= YES", "ESC", "= NO"]
    } else {
        ["ENTER", "= OK", "ESC", "= STOP"]
    };
    // Two pairs of lines, a little apart.
    for (line, dy) in lines.iter().zip([0, 9, 22, 31]) {
        text(
            p,
            line,
            Point::new(CENTER, PAIR_HINT_TOP + dy),
            &FONT_5X8,
            Alignment::Center,
        );
    }
}

/// Top of the pairing code's digits and of the hint under them.
const PAIR_DIGITS_TOP: i32 = 33;
const PAIR_HINT_TOP: i32 = 85;

/// Top of the radio power box at the bottom of the left screen.
const RADIO_TOP: i32 = 106;

/// The radio power for a moment after it changed (`RadioPower`): an inverted
/// box at the bottom, over the rain and the link stage, `RADIO` / `+4dBm`.
fn radio_power(p: &mut Portrait<'_>, status: &Status, anim: &Anim) {
    if !radio_visible(anim) {
        return;
    }
    clear_rows(p, RADIO_TOP - 1, RADIO_TOP);
    let area = Rectangle::new(
        Point::new(0, RADIO_TOP),
        Size::new(
            PORTRAIT_WIDTH as u32,
            (PORTRAIT_HEIGHT as i32 - RADIO_TOP) as u32,
        ),
    );
    let _ = area
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(p);
    let mut level = StrBuf::<6>::new();
    let _ = write!(level, "{:+}dBm", status.radio_dbm);
    let style = MonoTextStyle::new(&FONT_5X8, BinaryColor::Off);
    let layout = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Top)
        .build();
    for (i, line) in ["RADIO", level.as_str()].into_iter().enumerate() {
        let at = Point::new(CENTER, RADIO_TOP + 2 + 10 * i as i32);
        let _ = Text::with_text_style(line, at, style, layout).draw(p);
    }
}

/// Studio's lock state for a moment after it changed (the `StudioUnlock` key,
/// `#studio lock`, the timeout): an inverted box at the bottom like the radio
/// power's, `STUDIO` / `OPEN` or `LOCKED`. Drawn last, over the radio power.
fn studio_lock(p: &mut Portrait<'_>, status: &Status, anim: &Anim) {
    if !studio_visible(anim) {
        return;
    }
    clear_rows(p, RADIO_TOP - 1, RADIO_TOP);
    let area = Rectangle::new(
        Point::new(0, RADIO_TOP),
        Size::new(
            PORTRAIT_WIDTH as u32,
            (PORTRAIT_HEIGHT as i32 - RADIO_TOP) as u32,
        ),
    );
    let _ = area
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(p);
    let state = if status.studio_unlocked {
        "OPEN"
    } else {
        "LOCKED"
    };
    let style = MonoTextStyle::new(&FONT_5X8, BinaryColor::Off);
    let layout = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Top)
        .build();
    for (i, line) in ["STUDIO", state].into_iter().enumerate() {
        let at = Point::new(CENTER, RADIO_TOP + 2 + 10 * i as i32);
        let _ = Text::with_text_style(line, at, style, layout).draw(p);
    }
}

/// Characters per line and lines that fit on the right half's screen at once
/// (longer notifications scroll).
pub const NOTIFICATION_COLUMNS: usize = PORTRAIT_WIDTH / 5;
pub const NOTIFICATION_LINES: usize = 9;
const NOTIFICATION_LINE_HEIGHT: i32 = 9;
const NOTIFICATION_TOP: i32 = 42;
/// Enough for any notification ([`kb_core::host::MAX_TEXT`] characters).
const NOTIFICATION_MAX_LINES: usize = 64;

/// Bottom (exclusive) of the notification area: above the link stage when shown.
fn notification_bottom(status: &Status, anim: &Anim) -> i32 {
    if link_visible(status, anim) {
        116
    } else {
        PORTRAIT_HEIGHT as i32
    }
}

/// Pixels of the notification that don't fit on the screen (0: it all fits).
#[must_use]
pub fn notification_overflow(status: &Status, anim: &Anim) -> i32 {
    let Some(note) = &status.notification else {
        return 0;
    };
    let lines = wrap(
        note.as_str(),
        NOTIFICATION_COLUMNS,
        NOTIFICATION_MAX_LINES,
        |_, _| {},
    );
    let height = lines as i32 * NOTIFICATION_LINE_HEIGHT - 1;
    (height - (notification_bottom(status, anim) - NOTIFICATION_TOP)).max(0)
}

/// Scroll position of a long notification: rest at the top, scroll down a
/// pixel a step, rest at the end, jump back.
fn notification_scroll(overflow: i32, elapsed: u32) -> i32 {
    if overflow <= 0 {
        return 0;
    }
    let travel = overflow as u32 * SCROLL_STEP_MS;
    let t = elapsed % (2 * SCROLL_PAUSE_MS + travel);
    if t < SCROLL_PAUSE_MS {
        0
    } else {
        (((t - SCROLL_PAUSE_MS) / SCROLL_STEP_MS) as i32).min(overflow)
    }
}

/// Arriving message: the envelope builds up in this time; scrambled text
/// shows from `NOTE_TEXT_FROM_MS`, and its lines lock in one after the other
/// from the envelope's completion on, while the rain behind thins out.
const NOTE_ENVELOPE_MS: u32 = 350;
const NOTE_TEXT_FROM_MS: u32 = 250;
const NOTE_LINE_LOCK_MS: u32 = 120;
const NOTE_RAIN_MS: u32 = 700;

/// The right half's portrait screen: link and battery on top, then the
/// computer's message or the digital rain.
fn portrait_peripheral(p: &mut Portrait<'_>, status: &Status, anim: &Anim) {
    if status.peripheral_connected {
        wifi(p, CENTER - icon_width(WIFI) / 2, 1, false, anim);
    } else if anim.live {
        // Searching for the left half.
        wifi(p, CENTER - icon_width(WIFI) / 2, 1, true, anim);
    } else {
        icon(p, CENTER - icon_width(CLOSE) / 2, 1, CLOSE);
    }
    battery_line(p, status, 1, anim);
    connect_flash(p, anim.linked_at, anim);
    separator(p, 26);

    if let Some(note) = &status.notification {
        notification(p, status, anim, note.as_str());
    } else {
        digital_rain(p, status, anim, 29);
        if let (Some(e), Some(note)) = (
            anim.within(anim.cleared_at, NOTE_CLEAR_MS),
            &anim.cleared_note,
        ) {
            clearing_note(p, status, anim, note.as_str(), e);
        }
    }
    link_detail(p, status, anim);
}

/// The computer's message `note` under its envelope; arriving, the
/// envelope builds up out of the rain and the text decodes below it.
fn notification(p: &mut Portrait<'_>, status: &Status, anim: &Anim, note: &str) {
    // Arriving: the envelope builds up out of the rain, pixel by pixel,
    // then the text decodes below it (one sequence, see below).
    let arriving = anim.within(anim.note_at, NOTE_DECODE_MS);
    let mail_x = CENTER - icon_width(MAIL) / 2;
    match arriving.filter(|&e| e < NOTE_ENVELOPE_MS) {
        None => icon(p, mail_x, 31, MAIL),
        Some(e) => {
            let pixels = MAIL.iter().enumerate().flat_map(|(dy, row)| {
                row.bytes().enumerate().filter_map(move |(dx, b)| {
                    let lands = mix((dy * 16 + dx) as u32 + 313) % NOTE_ENVELOPE_MS;
                    (b == b'#' && e >= lands).then_some(Pixel(
                        Point::new(mail_x + dx as i32, 31 + dy as i32),
                        BinaryColor::On,
                    ))
                })
            });
            let _ = p.draw_iter(pixels);
        }
    }

    let elapsed = anim.since(anim.note_at).unwrap_or(0);
    let decoding = arriving;
    let slide = 0;
    let scroll = anim.since(anim.note_at).map_or(0, |_| {
        notification_scroll(notification_overflow(status, anim), elapsed)
    });
    let bottom = notification_bottom(status, anim);
    let area = Rectangle::new(
        Point::new(0, NOTIFICATION_TOP),
        Size::new(PORTRAIT_WIDTH as u32, (bottom - NOTIFICATION_TOP) as u32),
    );
    let mut clipped = p.clipped(&area);
    // A burst of rain behind the arriving message, thinning out.
    if let Some(e) = decoding.filter(|&e| e < NOTE_RAIN_MS) {
        thinning_rain(
            &mut clipped,
            e,
            Half::Right,
            100 * (NOTE_RAIN_MS - e) / NOTE_RAIN_MS,
        );
    }
    wrap(
        note,
        NOTIFICATION_COLUMNS,
        NOTIFICATION_MAX_LINES,
        |i, line| {
            let y = NOTIFICATION_TOP + NOTIFICATION_LINE_HEIGHT * i as i32 + slide - scroll;
            if y >= bottom || y + NOTIFICATION_LINE_HEIGHT <= NOTIFICATION_TOP {
                return;
            }
            let Some(e) = decoding else {
                text(
                    &mut clipped,
                    line,
                    Point::new(1, y),
                    &FONT_5X8,
                    Alignment::Left,
                );
                return;
            };
            if e < NOTE_TEXT_FROM_MS {
                return; // only rain yet, while the envelope builds
            }
            // Decoding: every character flickers through random ones and
            // locks in, line after line from the top.
            for (j, c) in line.chars().enumerate() {
                let locks =
                    NOTE_ENVELOPE_MS + NOTE_LINE_LOCK_MS * i as u32 + mix((i * 16 + j) as u32) % 90;
                let x = 1 + 5 * j as i32;
                clear_box(&mut clipped, x, y, 5, 8);
                if c == ' ' || e >= locks {
                    let mut one = StrBuf::<4>::new();
                    let _ = one.write_char(c);
                    text(
                        &mut clipped,
                        one.as_str(),
                        Point::new(x, y),
                        &FONT_5X8,
                        Alignment::Left,
                    );
                } else {
                    // Still rain: one of the rain's own glyphs, flickering.
                    let glyph = (mix((i * 16 + j) as u32 ^ (e / 45)) & 15) as usize;
                    rain_glyph(&mut clipped, x + 1, y + 2, glyph, false);
                }
            }
        },
    );
}

/// A cleared message falling away, `e` ms after it was cleared: the envelope
/// dissolves pixel by pixel; every character scrambles and drops down the
/// screen at its own moment, faster and faster.
fn clearing_note(p: &mut Portrait<'_>, status: &Status, anim: &Anim, note: &str, e: u32) {
    let mail_x = CENTER - icon_width(MAIL) / 2;
    let pixels = MAIL.iter().enumerate().flat_map(|(dy, row)| {
        row.bytes().enumerate().filter_map(move |(dx, b)| {
            let gone = mix((dy * 16 + dx) as u32 + 977) % NOTE_ENVELOPE_MS;
            (b == b'#' && e < gone).then_some(Pixel(
                Point::new(mail_x + dx as i32, 31 + dy as i32),
                BinaryColor::On,
            ))
        })
    });
    let _ = p.draw_iter(pixels);

    let bottom = notification_bottom(status, anim);
    let area = Rectangle::new(
        Point::new(0, NOTIFICATION_TOP),
        Size::new(PORTRAIT_WIDTH as u32, (bottom - NOTIFICATION_TOP) as u32),
    );
    let mut clipped = p.clipped(&area);
    wrap(
        note,
        NOTIFICATION_COLUMNS,
        NOTIFICATION_MAX_LINES,
        |i, line| {
            for (j, c) in line.chars().enumerate() {
                if c == ' ' {
                    continue;
                }
                let key = (i * 16 + j) as u32;
                let drops_at = 80 + mix(key + 4242) % 380;
                let t = i64::from(e.saturating_sub(drops_at));
                let fall = (t * t * 140 / 250_000) as i32; // ~140 px in 500 ms
                let y = NOTIFICATION_TOP + NOTIFICATION_LINE_HEIGHT * i as i32 + fall;
                if y >= bottom {
                    continue;
                }
                let x = 1 + 5 * j as i32;
                if e < drops_at {
                    // Still the letter, until it turns into rain and drops.
                    let mut one = StrBuf::<4>::new();
                    let _ = one.write_char(c);
                    text(
                        &mut clipped,
                        one.as_str(),
                        Point::new(x, y),
                        &FONT_5X8,
                        Alignment::Left,
                    );
                    continue;
                }
                // A falling rain drop: a flickering glyph with a dim trail.
                for k in 0..3 {
                    let glyph = (mix(key ^ (e / 60) ^ k) & 15) as usize;
                    rain_glyph(
                        &mut clipped,
                        x + 1,
                        y + 2 - RAIN_PITCH_Y * k as i32,
                        glyph,
                        k > 0,
                    );
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{Drop, RADIO_SHOW_MS, RAIN_DROPS, STUDIO_SHOW_MS};

    const NAMES: [&str; 3] = ["default", "symbols", "nav"];

    fn rendered(half: Half, style: Style, status: &Status) -> Frame {
        let mut f = Frame::default();
        render(&mut f, half, style, status, &NAMES);
        f
    }

    fn lit(f: &Frame) -> u32 {
        f.buf.iter().map(|b| b.count_ones()).sum()
    }

    #[test]
    fn zmk_central_screen_draws_all_four_widgets() {
        // Arrange
        let status = Status::default();

        // Act
        let frame = rendered(Half::Left, Style::Zmk, &status);

        // Assert: something in each corner region
        let any = |x0: usize, x1: usize, y0: usize, y1: usize| {
            (y0..y1).any(|y| (x0..x1).any(|x| frame.get(x, y)))
        };
        assert!(any(0, 40, 0, 16), "output status");
        assert!(any(90, 128, 0, 16), "battery");
        assert!(any(0, 40, 16, 32), "layer");
        assert!(any(100, 128, 16, 32), "wpm");
    }

    #[test]
    fn zmk_central_screen_changes_with_layer() {
        // Arrange
        let nav = Status {
            layer: 2,
            ..Status::default()
        };
        let default = rendered(Half::Left, Style::Zmk, &Status::default());

        // Act
        let frame = rendered(Half::Left, Style::Zmk, &nav);

        // Assert
        assert!(frame != default);
    }

    #[test]
    fn disconnected_profile_shows_close_instead_of_check() {
        // Arrange
        let disconnected = Status {
            transport: Transport::Ble,
            profile_connected: false,
            ..Status::default()
        };
        let connected = Status {
            transport: Transport::Ble,
            ..Status::default()
        };
        let with_check = rendered(Half::Left, Style::Zmk, &connected);

        // Act
        let frame = rendered(Half::Left, Style::Zmk, &disconnected);

        // Assert
        assert!(frame != with_check);
    }

    #[test]
    fn portrait_left_screen_is_drawn_rotated_into_the_module() {
        // Arrange
        let status = Status::default();

        // Act
        let frame = rendered(Half::Left, Style::Portrait, &status);

        // Assert: the separator at upright y=26 is a module column at x=127-26
        let column: u32 = (0..HEIGHT)
            .map(|y| u32::from(frame.get(WIDTH - 1 - 26, y)))
            .sum();
        assert!(column >= 26);
        assert!(lit(&frame) > 100);
    }

    #[test]
    fn portrait_caps_word_marks_the_layer_row() {
        // Arrange
        let caps = Status {
            caps_word: true,
            ..Status::default()
        };
        let plain = rendered(Half::Left, Style::Portrait, &Status::default());

        // Act
        let frame = rendered(Half::Left, Style::Portrait, &caps);

        // Assert
        assert!(lit(&frame) > lit(&plain) + 100);
    }

    #[test]
    fn portrait_notification_is_shown_on_the_right_half() {
        // Arrange
        let note = Status {
            notification: Some(kb_core::host::Text::new("Build ok on main")),
            ..Status::default()
        };
        let quiet = rendered(Half::Right, Style::Portrait, &Status::default());

        // Act
        let frame = rendered(Half::Right, Style::Portrait, &note);

        // Assert
        assert!(lit(&frame) > lit(&quiet) + 100);
    }

    #[test]
    fn zmk_style_notification_is_shown_on_the_right_half() {
        // Arrange
        let note = Status {
            notification: Some(kb_core::host::Text::new("Build ok")),
            ..Status::default()
        };
        let quiet = rendered(Half::Right, Style::Zmk, &Status::default());

        // Act
        let frame = rendered(Half::Right, Style::Zmk, &note);

        // Assert
        assert!(lit(&frame) > lit(&quiet) + 50);
    }

    #[test]
    fn message_screen_draws_title_bar_and_text() {
        // Arrange
        let mut frame = Frame::default();

        // Act
        message(&mut frame, Half::Left, "PANIC", "radio.rs:120");

        // Assert: a filled title bar plus text
        assert!(lit(&frame) > 250);
    }

    fn typing(typed: &[u8]) -> Status {
        let mut digits = [0; 6];
        digits[..typed.len()].copy_from_slice(typed);
        Status {
            pairing: Pairing::TypeCode {
                typed: digits,
                len: typed.len() as u8,
            },
            ..Status::default()
        }
    }

    #[test]
    fn pairing_prompt_replaces_the_left_screen() {
        // Arrange
        let plain = rendered(Half::Left, Style::Portrait, &Status::default());

        // Act
        let frame = rendered(Half::Left, Style::Portrait, &typing(&[]));

        // Assert
        assert!(frame != plain);
        assert!(
            lit_rows(&frame, Half::Left, 0, 11) > 250,
            "inverted title bar"
        );
    }

    #[test]
    fn typed_digits_fill_the_pairing_prompt() {
        // Arrange
        let empty = rendered(Half::Left, Style::Portrait, &typing(&[]));

        // Act
        let frame = rendered(Half::Left, Style::Portrait, &typing(&[1, 2, 8]));

        // Assert: digits where the placeholders were
        assert!(lit_rows(&frame, Half::Left, 33, 54) > lit_rows(&empty, Half::Left, 33, 54));
    }

    #[test]
    fn compared_code_shows_all_six_digits() {
        // Arrange
        let status = Status {
            pairing: Pairing::Compare(123_456),
            ..Status::default()
        };

        // Act
        let frame = rendered(Half::Left, Style::Portrait, &status);

        // Assert: both rows of digits are drawn
        assert!(lit_rows(&frame, Half::Left, 33, 54) > 40);
        assert!(lit_rows(&frame, Half::Left, 55, 76) > 40);
    }

    #[test]
    fn pairing_prompt_fits_on_the_screen() {
        // Arrange
        let status = typing(&[1, 2, 3, 4, 5, 6]);

        // Act
        let frame = rendered(Half::Left, Style::Portrait, &status);

        // Assert: the hint's last line ends above the bottom edge
        assert!(lit_rows(&frame, Half::Left, 112, 125) > 0);
        assert_eq!(lit_rows(&frame, Half::Left, 125, 128), 0);
    }

    #[test]
    fn zmk_style_shows_the_pairing_prompt_too() {
        // Arrange
        let plain = rendered(Half::Left, Style::Zmk, &Status::default());

        // Act
        let frame = rendered(Half::Left, Style::Zmk, &typing(&[4]));

        // Assert
        assert!(frame != plain);
    }

    #[test]
    fn radio_power_shows_in_a_box_at_the_bottom_after_a_change() {
        // Arrange
        let status = Status {
            radio_dbm: 4,
            ..Status::default()
        };
        let anim = Anim {
            radio_at: Some(900),
            ..live(1000)
        };

        // Act
        let frame = animated(Half::Left, &status, &anim);

        // Assert: an inverted box, mostly lit
        assert!(lit_rows(&frame, Half::Left, RADIO_TOP as usize, 128) > 400);
    }

    #[test]
    fn studio_unlock_shows_in_a_box_at_the_bottom() {
        // Arrange
        let status = Status {
            studio_unlocked: true,
            ..Status::default()
        };
        let anim = Anim {
            studio_at: Some(900),
            ..live(1000)
        };

        // Act
        let frame = animated(Half::Left, &status, &anim);

        // Assert: an inverted box, mostly lit
        assert!(lit_rows(&frame, Half::Left, RADIO_TOP as usize, 128) > 400);
    }

    #[test]
    fn studio_open_and_locked_look_different() {
        // Arrange
        let anim = Anim {
            studio_at: Some(900),
            ..live(1000)
        };
        let locked = animated(Half::Left, &Status::default(), &anim);
        let open = Status {
            studio_unlocked: true,
            ..Status::default()
        };

        // Act
        let frame = animated(Half::Left, &open, &anim);

        // Assert
        assert!(frame != locked);
    }

    #[test]
    fn studio_state_is_hidden_after_a_while() {
        // Arrange
        let status = Status {
            studio_unlocked: true,
            ..Status::default()
        };
        let anim = Anim {
            studio_at: Some(0),
            ..live(STUDIO_SHOW_MS)
        };

        // Act
        let frame = animated(Half::Left, &status, &anim);

        // Assert
        assert_eq!(lit_rows(&frame, Half::Left, RADIO_TOP as usize, 128), 0);
    }

    #[test]
    fn radio_power_is_hidden_after_a_while() {
        // Arrange
        let status = Status::default();
        let anim = Anim {
            radio_at: Some(0),
            ..live(RADIO_SHOW_MS)
        };

        // Act
        let frame = animated(Half::Left, &status, &anim);

        // Assert
        assert_eq!(lit_rows(&frame, Half::Left, RADIO_TOP as usize, 128), 0);
    }

    #[test]
    fn link_detail_is_shown_at_the_bottom() {
        // Arrange
        let detail = Status {
            link_detail: "pair",
            ..Status::default()
        };
        let plain = rendered(Half::Right, Style::Portrait, &Status::default());

        // Act
        let frame = rendered(Half::Right, Style::Portrait, &detail);

        // Assert
        assert!(lit(&frame) > lit(&plain) + 20);
    }

    #[test]
    fn profile_ten_fits_next_to_its_check_mark() {
        // Arrange: profile index 9 is shown as "10"
        let status = Status {
            transport: Transport::Ble,
            profile: 9,
            ..Status::default()
        };

        // Act
        let frame = rendered(Half::Left, Style::Portrait, &status);

        // Assert: nothing drawn past the right edge of the 32-pixel-wide screen
        // means the module column for upright x=31 still fits; and the check
        // mark's rightmost column (x=30) is lit somewhere in the top rows.
        let lit_at = |px: usize| {
            (0..9).any(|py| {
                let (x, y) = Half::Left.module_xy(px, py);
                frame.get(x, y)
            })
        };
        assert!(lit_at(30) || lit_at(29));
    }

    #[test]
    fn open_taken_and_connected_profiles_look_different() {
        // Arrange
        let ble = Status {
            transport: Transport::Ble,
            ..Status::default()
        };
        let connected = Status {
            profile_connected: true,
            ..ble
        };
        let open = Status {
            profile_connected: false,
            profile_open: true,
            ..ble
        };
        let taken = Status {
            profile_connected: false,
            profile_open: false,
            ..ble
        };

        // Act
        let frames = [connected, open, taken].map(|st| {
            [
                rendered(Half::Left, Style::Portrait, &st),
                rendered(Half::Left, Style::Zmk, &st),
            ]
        });

        // Assert
        let [connected, open, taken] = &frames;
        for ((c, o), t) in connected.iter().zip(open).zip(taken) {
            assert!(c != o && o != t && c != t);
        }
    }

    #[test]
    fn peripheral_screen_shows_link_and_battery() {
        // Arrange
        let status = Status::default();

        // Act
        let frame = rendered(Half::Right, Style::Portrait, &status);

        // Assert
        assert!(lit(&frame) > 30);
    }

    // ---- animations ----

    fn animated(half: Half, status: &Status, anim: &Anim) -> Frame {
        let mut f = Frame::default();
        render_animated(&mut f, half, Style::Portrait, status, &NAMES, anim);
        f
    }

    /// Lit pixels in upright rows `y0..y1`.
    fn lit_rows(f: &Frame, half: Half, y0: usize, y1: usize) -> usize {
        (y0..y1)
            .flat_map(|y| (0..PORTRAIT_WIDTH).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let (mx, my) = half.module_xy(x, y);
                f.get(mx, my)
            })
            .count()
    }

    fn live(now: u32) -> Anim {
        Anim {
            live: true,
            now,
            ..Anim::still()
        }
    }

    #[test]
    fn layer_name_is_between_old_and_new_halfway_through_the_slide() {
        // Arrange
        let nav = Status {
            layer: 2,
            ..Status::default()
        };
        let sliding = Anim {
            layer_from: 0,
            layer_at: Some(1000),
            ..live(1000 + LAYER_SLIDE_MS / 2)
        };
        let start = animated(
            Half::Left,
            &nav,
            &Anim {
                now: 1000,
                ..sliding
            },
        );
        let end = animated(Half::Left, &nav, &live(5000));

        // Act
        let middle = animated(Half::Left, &nav, &sliding);

        // Assert
        assert!(middle != start && middle != end);
    }

    #[test]
    fn computer_connecting_flashes_the_top_of_the_screen() {
        // Arrange
        let status = Status::default();
        let before = lit_rows(
            &animated(Half::Left, &status, &live(1000)),
            Half::Left,
            0,
            26,
        );
        let flashing = Anim {
            connected_at: Some(1000),
            ..live(1000)
        };

        // Act
        let frame = animated(Half::Left, &status, &flashing);

        // Assert: inverted
        assert_eq!(
            lit_rows(&frame, Half::Left, 0, 26),
            26 * PORTRAIT_WIDTH - before
        );
    }

    #[test]
    fn searching_icon_starts_with_the_dot_only() {
        // Arrange
        let searching = Status {
            profile_connected: false,
            ..Status::default()
        };
        let still = lit_rows(
            &rendered(Half::Left, Style::Portrait, &searching),
            Half::Left,
            0,
            6,
        );

        // Act
        let frame = animated(Half::Left, &searching, &live(0));

        // Assert: arcs (upper icon rows) not drawn yet, profile digit and mark still there
        assert!(lit_rows(&frame, Half::Left, 0, 6) < still);
    }

    #[test]
    fn caps_word_alternates_the_layer_row_between_inverted_and_outlined() {
        // Arrange
        let caps = Status {
            caps_word: true,
            ..Status::default()
        };
        let (top, bottom) = LAYER_ROW;
        let outlined = lit_rows(
            &animated(Half::Left, &caps, &live(0)),
            Half::Left,
            top,
            bottom,
        );

        // Act
        let filled = lit_rows(
            &animated(Half::Left, &caps, &live(CAPS_BLINK_MS)),
            Half::Left,
            top,
            bottom,
        );

        // Assert
        assert!(filled > outlined);
    }

    fn dropping(at: u32, now: u32) -> Anim {
        let mut drops = [None; RAIN_DROPS];
        for (i, d) in drops.iter_mut().enumerate() {
            *d = Some(Drop {
                at,
                seed: mix(i as u32),
            });
        }
        Anim { drops, ..live(now) }
    }

    #[test]
    fn digital_rain_is_empty_without_typing() {
        // Arrange
        let status = Status::default();

        // Act
        let frame = animated(Half::Left, &status, &live(5000));

        // Assert
        assert_eq!(
            lit_rows(&frame, Half::Left, RAIN_TOP as usize, PORTRAIT_HEIGHT),
            0
        );
    }

    #[test]
    fn digital_rain_falls_after_key_presses() {
        // Arrange
        let before = animated(Half::Left, &Status::default(), &dropping(0, 200));

        // Act
        let after = animated(Half::Left, &Status::default(), &dropping(0, 500));

        // Assert: rain in the area, moving; nothing above it changed
        assert!(lit_rows(&after, Half::Left, RAIN_TOP as usize, PORTRAIT_HEIGHT) > 0);
        assert!(after != before);
        assert_eq!(
            lit_rows(&after, Half::Left, 0, RAIN_TOP as usize),
            lit_rows(&before, Half::Left, 0, RAIN_TOP as usize)
        );
    }

    #[test]
    fn right_half_rains_without_a_notification() {
        // Arrange
        let status = Status::default();

        // Act
        let frame = animated(Half::Right, &status, &dropping(0, 300));

        // Assert
        assert!(lit_rows(&frame, Half::Right, 29, PORTRAIT_HEIGHT) > 0);
    }

    #[test]
    fn notification_replaces_the_rain_on_the_right_half() {
        // Arrange
        let status = Status {
            notification: Some(kb_core::host::Text::new("Build ok")),
            ..Status::default()
        };
        let without_rain = animated(Half::Right, &status, &live(300));

        // Act
        let frame = animated(Half::Right, &status, &dropping(0, 300));

        // Assert
        assert!(frame == without_rain);
    }

    #[test]
    fn digital_rain_leaves_room_for_the_link_stage() {
        // Arrange
        let status = Status {
            link_detail: "disc",
            ..Status::default()
        };

        // Act: wherever the rain is
        let frames: Vec<Frame> = (0..40)
            .map(|i| animated(Half::Left, &status, &dropping(0, i * 50)))
            .collect();

        // Assert: nothing between the rain and the stage text
        assert!(
            frames
                .iter()
                .all(|f| lit_rows(f, Half::Left, 116, 119) == 0)
        );
    }

    #[test]
    fn link_stage_in_progress_draws_a_sweeping_line() {
        // Arrange
        let status = Status {
            link_detail: "scan",
            ..Status::default()
        };

        // Act
        let frame = animated(Half::Left, &status, &live(0));

        // Assert
        assert_eq!(lit_rows(&frame, Half::Left, 117, 118), 6);
    }

    #[test]
    fn arriving_envelope_builds_up() {
        // Arrange
        let status = Status {
            notification: Some(kb_core::host::Text::new("Build ok")),
            ..Status::default()
        };
        let envelope = |t: u32| {
            let frame = animated(
                Half::Right,
                &status,
                &Anim {
                    note_at: Some(0),
                    ..live(t)
                },
            );
            lit_rows(&frame, Half::Right, 31, 38)
        };
        let whole = envelope(NOTE_DECODE_MS);

        // Act
        let building = envelope(NOTE_ENVELOPE_MS / 3);

        // Assert: part of it, not yet all
        assert!(building > 0 && building < whole);
    }

    #[test]
    fn long_notification_scrolls_after_resting() {
        // Arrange
        let long = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen";
        let status = Status {
            notification: Some(kb_core::host::Text::new(long)),
            ..Status::default()
        };
        let resting = animated(
            Half::Right,
            &status,
            &Anim {
                note_at: Some(0),
                ..live(1000)
            },
        );
        let later = Anim {
            note_at: Some(0),
            ..live(SCROLL_PAUSE_MS + 20 * SCROLL_STEP_MS)
        };

        // Act
        let scrolled = animated(Half::Right, &status, &later);

        // Assert
        assert!(notification_overflow(&status, &Anim::still()) > 0);
        assert!(scrolled != resting);
    }

    #[test]
    fn short_notification_does_not_overflow() {
        // Arrange
        let status = Status {
            notification: Some(kb_core::host::Text::new("Build ok")),
            ..Status::default()
        };

        // Act
        let overflow = notification_overflow(&status, &Anim::still());

        // Assert
        assert_eq!(overflow, 0);
    }

    #[test]
    fn held_shift_gets_a_bar_under_the_layer() {
        // Arrange
        let shift = Status {
            mods: 0b0000_0010,
            ..Status::default()
        };

        // Act
        let frame = animated(Half::Left, &shift, &live(0));

        // Assert: one inverted bar (mostly lit), nothing in the next bar's place
        let first = lit_rows(
            &frame,
            Half::Left,
            MODS_TOP as usize,
            (MODS_TOP + MOD_BAR_HEIGHT) as usize,
        );
        let second_top = (MODS_TOP + MOD_BAR_HEIGHT + 1) as usize;
        let second = lit_rows(
            &frame,
            Half::Left,
            second_top,
            second_top + MOD_BAR_HEIGHT as usize,
        );
        assert!(first > 200);
        assert_eq!(second, 0);
    }

    #[test]
    fn held_modifiers_stack_up() {
        // Arrange
        let ctrl_shift = Status {
            mods: 0b0000_0011,
            ..Status::default()
        };

        // Act
        let frame = animated(Half::Left, &ctrl_shift, &live(0));

        // Assert: two bars
        let second_top = (MODS_TOP + MOD_BAR_HEIGHT + 1) as usize;
        assert!(
            lit_rows(
                &frame,
                Half::Left,
                second_top,
                second_top + MOD_BAR_HEIGHT as usize
            ) > 200
        );
    }

    fn bar_rows(frame: &Frame, index: i32) -> usize {
        let top = (MODS_TOP + index * (MOD_BAR_HEIGHT + 1)) as usize;
        lit_rows(frame, Half::Left, top, top + MOD_BAR_HEIGHT as usize)
    }

    #[test]
    fn active_pointer_gets_a_mouse_bar() {
        // Arrange
        let pointer = Status {
            pointer_active: true,
            ..Status::default()
        };

        // Act
        let frame = animated(Half::Left, &pointer, &live(0));

        // Assert: one bar, where the first modifier bar goes
        assert!(bar_rows(&frame, 0) > 200);
        assert_eq!(bar_rows(&frame, 1), 0);
    }

    #[test]
    fn mouse_bar_comes_above_ctrl() {
        // Arrange
        let both = Status {
            pointer_active: true,
            mods: 0b0000_0001,
            ..Status::default()
        };
        let ctrl = Status {
            mods: 0b0000_0001,
            ..Status::default()
        };
        let ctrl_frame = animated(Half::Left, &ctrl, &live(0));

        // Act
        let frame = animated(Half::Left, &both, &live(0));

        // Assert: two bars; the second looks like CTRL's own (the first)
        assert!(bar_rows(&frame, 0) > 200);
        assert!(bar_rows(&frame, 1) > 200);
        let rows = |f: &Frame, index: i32| {
            let top = (MODS_TOP + index * (MOD_BAR_HEIGHT + 1)) as usize;
            (top..top + MOD_BAR_HEIGHT as usize)
                .map(|y| {
                    (0..PORTRAIT_WIDTH)
                        .map(|x| {
                            let (mx, my) = Half::Left.module_xy(x, y);
                            f.get(mx, my)
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(rows(&frame, 1), rows(&ctrl_frame, 0));
        assert_ne!(rows(&frame, 0), rows(&ctrl_frame, 0), "MOUSE, not CTRL");
    }

    #[test]
    fn no_mouse_bar_while_the_pointer_is_inactive() {
        // Arrange
        let plain = Status::default();

        // Act
        let frame = animated(Half::Left, &plain, &live(0));

        // Assert
        assert_eq!(bar_rows(&frame, 0), 0);
    }

    #[test]
    fn mouse_bar_is_only_on_the_left_screen() {
        // Arrange
        let pointer = Status {
            pointer_active: true,
            ..Status::default()
        };

        // Act
        let frame = animated(Half::Right, &pointer, &live(0));

        // Assert
        assert!(frame == animated(Half::Right, &Status::default(), &live(0)));
    }

    #[test]
    fn right_modifiers_look_like_left_ones() {
        // Arrange
        let left_ctrl = Status {
            mods: 0b0000_0001,
            ..Status::default()
        };
        let right_ctrl = Status {
            mods: 0b0001_0000,
            ..Status::default()
        };

        // Act
        let frame = animated(Half::Left, &right_ctrl, &live(0));

        // Assert
        assert!(frame == animated(Half::Left, &left_ctrl, &live(0)));
    }

    #[test]
    fn charging_shows_a_bolt_instead_of_the_percent_sign() {
        // Arrange
        let plugged = Status {
            charging: [true, false],
            ..Status::default()
        };

        // Act
        let frame = animated(Half::Left, &plugged, &live(0));

        // Assert
        assert!(frame != animated(Half::Left, &Status::default(), &live(0)));
    }

    #[test]
    fn low_battery_blinks() {
        // Arrange
        let low = Status {
            battery: [10, 100],
            ..Status::default()
        };
        let shown = lit_rows(
            &animated(Half::Left, &low, &live(LOW_BATTERY_BLINK_MS)),
            Half::Left,
            13,
            23,
        );

        // Act
        let hidden = lit_rows(&animated(Half::Left, &low, &live(0)), Half::Left, 13, 23);

        // Assert
        assert!(shown > 0);
        assert_eq!(hidden, 0);
    }

    #[test]
    fn charging_battery_does_not_blink_when_low() {
        // Arrange
        let low = Status {
            battery: [10, 100],
            charging: [true, false],
            ..Status::default()
        };

        // Act
        let frame = animated(Half::Left, &low, &live(0));

        // Assert
        assert!(lit_rows(&frame, Half::Left, 13, 23) > 0);
    }

    fn status_screen() -> Frame {
        let mut f = Frame::default();
        render(
            &mut f,
            Half::Left,
            Style::Portrait,
            &Status::default(),
            &NAMES,
        );
        f
    }

    #[test]
    fn power_down_starts_from_the_screen_as_it_was() {
        // Arrange
        let from = status_screen();
        let mut frame = Frame::default();

        // Act
        sleep_art(&mut frame, Half::Left, 0, &from);

        // Assert
        assert!(frame == from);
    }

    #[test]
    fn power_down_melts_the_screen_downward() {
        // Arrange
        let from = status_screen();
        let mut frame = Frame::default();

        // Act
        sleep_art(&mut frame, Half::Left, 1000, &from);

        // Assert: less at the top than before, more at the bottom
        assert!(lit_rows(&frame, Half::Left, 0, 30) < lit_rows(&from, Half::Left, 0, 30));
        assert!(lit_rows(&frame, Half::Left, 64, 128) > lit_rows(&from, Half::Left, 64, 128));
    }

    #[test]
    fn power_down_brings_nothing_back_in_at_the_top() {
        // Arrange
        let from = status_screen();
        let mut frame = Frame::default();
        let times = (600..SLEEP_FALL_END).step_by(10);

        // Act
        let top: Vec<usize> = times
            .map(|t| {
                sleep_art(&mut frame, Half::Left, t, &from);
                lit_rows(&frame, Half::Left, 0, 20)
            })
            .collect();

        // Assert
        assert!(top.windows(2).all(|w| w[1] <= w[0]), "{top:?}");
    }

    #[test]
    fn power_down_upper_half_only_empties() {
        // Arrange
        let from = status_screen();
        let mut frame = Frame::default();
        let times = (10..SLEEP_FALL_END).step_by(10);

        // Act
        let upper: Vec<usize> = times
            .map(|t| {
                sleep_art(&mut frame, Half::Left, t, &from);
                lit_rows(&frame, Half::Left, 0, PORTRAIT_HEIGHT / 2)
            })
            .collect();

        // Assert
        assert!(upper.windows(2).all(|w| w[1] <= w[0]), "{upper:?}");
    }

    #[test]
    fn power_down_is_empty_before_the_cursor() {
        // Arrange
        let from = status_screen();
        let mut frame = Frame::default();

        // Act
        sleep_art(&mut frame, Half::Left, SLEEP_FALL_END - 1, &from);

        // Assert
        assert_eq!(lit(&frame), 0);
    }

    #[test]
    fn power_down_ends_on_a_blinking_cursor() {
        // Arrange
        let from = status_screen();
        let mut frame = Frame::default();

        // Act
        sleep_art(&mut frame, Half::Left, SLEEP_FALL_END, &from);

        // Assert: just the 6 × 10 block
        assert_eq!(lit(&frame), 60);
    }

    #[test]
    fn boot_starts_with_rain_everywhere() {
        // Arrange
        let status = Frame::default();
        let mut frame = Frame::default();

        // Act
        boot_art(&mut frame, Half::Left, 300, 300, "E", &status);

        // Assert: lit pixels in the top and the bottom half
        assert!(lit_rows(&frame, Half::Left, 0, 64) > 20);
        assert!(lit_rows(&frame, Half::Left, 64, 128) > 20);
    }

    /// Lit pixels in rows `y0..y1` over the width of the initial (24 pixels).
    fn lit_in_initial(f: &Frame, half: Half, y0: i32, y1: i32) -> usize {
        let x0 = (PORTRAIT_WIDTH - 24) / 2;
        (y0 as usize..y1 as usize)
            .flat_map(|y| (x0..x0 + 24).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let (mx, my) = half.module_xy(x, y);
                f.get(mx, my)
            })
            .count()
    }

    #[test]
    fn boot_initial_stands_built_before_the_reveal() {
        // Arrange
        let status = Frame::default();
        let mut frame = Frame::default();

        // Act: after the sparks, before the scan line
        boot_art(
            &mut frame,
            Half::Left,
            BOOT_LOCK_AT + BOOT_SPARKS_MS,
            BOOT_LOCK_AT + BOOT_SPARKS_MS,
            "E",
            &status,
        );

        // Assert: the E's top bar (24 × 8 pixels) is fully lit
        assert_eq!(
            lit_in_initial(&frame, Half::Left, BIG_TOP, BIG_TOP + 2 * BIG_CELL),
            (6 * BIG_CELL * 2 * BIG_CELL) as usize
        );
    }

    #[test]
    fn boot_shows_l_on_the_right() {
        // Arrange
        let status = Frame::default();
        let t = BOOT_LOCK_AT + BOOT_SPARKS_MS;
        let (mut e, mut l) = (Frame::default(), Frame::default());
        boot_art(&mut e, Half::Right, t, t, "E", &status);

        // Act
        boot_art(&mut l, Half::Right, t, t, "L", &status);

        // Assert: L's top row has only its stem (8 wide), E's the full bar
        let top = |f: &Frame| lit_in_initial(f, Half::Right, BIG_TOP, BIG_TOP + BIG_CELL);
        assert_eq!((top(&e), top(&l)), (24 * 4, 8 * 4));
    }

    #[test]
    fn boot_ends_on_the_status_screen() {
        // Arrange
        let mut status = Frame::default();
        render(
            &mut status,
            Half::Left,
            Style::Portrait,
            &Status::default(),
            &NAMES,
        );
        let mut frame = Frame::default();

        // Act
        boot_art(
            &mut frame,
            Half::Left,
            BOOT_ART_MS,
            BOOT_ART_MS,
            "E",
            &status,
        );

        // Assert
        assert!(frame == status);
    }

    #[test]
    fn boot_waits_in_the_rain_for_the_other_half() {
        // Arrange / Act
        let clock = boot_clock(5000, None);

        // Assert
        assert_eq!(clock, None);
    }

    #[test]
    fn boot_continues_together_once_linked() {
        // Arrange: linked 100 ms ago, 5 s after power-on

        // Act
        let clock = boot_clock(5000, Some(100));

        // Assert: 100 ms into the letter phase
        assert_eq!(clock, Some(BOOT_BUILD_AT + 100));
    }

    #[test]
    fn boot_rains_a_moment_even_when_linked_at_once() {
        // Arrange / Act: linked right at power-on, 300 ms ago
        let clock = boot_clock(300, Some(300));

        // Assert
        assert_eq!(clock, None);
    }

    #[test]
    fn boot_gives_up_waiting_without_the_other_half() {
        // Arrange / Act
        let clock = boot_clock(BOOT_WAIT_MAX_MS + 200, None);

        // Assert
        assert_eq!(clock, Some(BOOT_BUILD_AT + 200));
    }

    #[test]
    fn waiting_boot_asks_to_turn_on_the_other_half() {
        // Arrange
        let status = Frame::default();
        let (mut early, mut late) = (Frame::default(), Frame::default());
        boot_frame(&mut early, Half::Left, 1000, None, "E", &status);

        // Act
        boot_frame(&mut late, Half::Left, 3000, None, "E", &status);

        // Assert: the hint's box frame (top edge, full width) appears
        assert_eq!(lit_rows(&late, Half::Left, 44, 45), PORTRAIT_WIDTH);
        assert!(lit_rows(&early, Half::Left, 44, 45) < PORTRAIT_WIDTH);
    }

    fn note_status() -> Status {
        Status {
            notification: Some(kb_core::host::Text::new("Build passed on main")),
            ..Status::default()
        }
    }

    #[test]
    fn arriving_message_is_scrambled_at_first() {
        // Arrange
        let status = note_status();
        let settled = animated(
            Half::Right,
            &status,
            &Anim {
                note_at: Some(0),
                ..live(NOTE_DECODE_MS)
            },
        );

        // Act
        let arriving = animated(
            Half::Right,
            &status,
            &Anim {
                note_at: Some(0),
                ..live(60)
            },
        );

        // Assert
        assert!(arriving != settled);
    }

    #[test]
    fn decoded_message_reads_like_the_still_one() {
        // Arrange
        let status = note_status();
        let still = rendered(Half::Right, Style::Portrait, &status);

        // Act: past the decode and the envelope bounce
        let settled = animated(
            Half::Right,
            &status,
            &Anim {
                note_at: Some(0),
                ..live(NOTE_DECODE_MS)
            },
        );

        // Assert: the text rows match
        assert_eq!(
            lit_rows(&settled, Half::Right, 42, 70),
            lit_rows(&still, Half::Right, 42, 70)
        );
    }

    #[test]
    fn cleared_message_falls_away() {
        // Arrange
        let quiet = animated(Half::Right, &Status::default(), &live(0));
        let clearing = Anim {
            cleared_note: Some(kb_core::host::Text::new("Build passed on main")),
            cleared_at: Some(0),
            ..live(NOTE_CLEAR_MS / 2)
        };

        // Act
        let frame = animated(Half::Right, &Status::default(), &clearing);

        // Assert: still something of it on the screen
        assert!(frame != quiet);
    }

    #[test]
    fn cleared_message_is_gone_after_falling() {
        // Arrange
        let quiet = animated(Half::Right, &Status::default(), &live(NOTE_CLEAR_MS));
        let cleared = Anim {
            cleared_note: Some(kb_core::host::Text::new("Build passed on main")),
            cleared_at: Some(0),
            ..live(NOTE_CLEAR_MS)
        };

        // Act
        let frame = animated(Half::Right, &Status::default(), &cleared);

        // Assert
        assert!(frame == quiet);
    }
}

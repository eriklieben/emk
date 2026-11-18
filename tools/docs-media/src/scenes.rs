//! The OLED animations, played by the emulated firmware (`kb-emu`) on a
//! simulated clock: every millisecond the firmware ticks, and every
//! [`FRAME_MS`] both displays get their I²C writes, as the app does. The
//! same script gives the same GIF every time.

use kb_core::host::{KIND_CLEAR, REPORT_SIZE, encode_notification};
use kb_core::keymaps::corne;
use kb_core::power::PowerConfig;
use kb_display::frame::{Frame, Half, PORTRAIT_WIDTH};
use kb_display::screens::{self, Style};
use kb_display::status::{Status, Transport};
use kb_emu::Emu;

use crate::gifout::Animation;
use crate::img::Indexed;
use crate::oled::{self, Panel, Show};

/// 25 frames a second.
pub const FRAME_MS: u32 = 40;
/// OLED pixels are drawn this many image pixels wide.
pub const SCALE: i32 = 4;

/// Key positions (see `core/src/keymaps/corne.rs`).
pub mod pos {
    pub const SYM: u8 = 36;
    pub const BSPC: u8 = 37;
    pub const SPACE: u8 = 38;
    pub const ENTER: u8 = 39;
    pub const NAV: u8 = 41;
    pub const LCTRL: u8 = 24;
    pub const RSHIFT: u8 = 35;
    /// T and Y: the caps word combo.
    pub const T: u8 = 5;
    pub const Y: u8 = 6;
    /// Nav: S moves the pointer left.
    pub const MOUSE_LEFT: u8 = 14;
    pub const MOUSE_UP: u8 = 16;
    /// Nav: B is the radio power key.
    pub const RADIO: u8 = 29;

    /// Where digit `d` is on the symbols layer: 1-5 on A..G, 6-0 on Z..B.
    #[must_use]
    pub const fn digit(d: u8) -> u8 {
        match d {
            1..=5 => 12 + d,
            6..=9 => 19 + d,
            _ => 29,
        }
    }
}

/// Position of a letter or space on the default layer.
///
/// # Panics
///
/// When no key on the default layer types `c`.
#[must_use]
pub fn key_of(c: char) -> u8 {
    if c == ' ' {
        return pos::SPACE;
    }
    let want = c.to_ascii_uppercase().to_string();
    (0..corne::KEYS as u8)
        .find(|&p| kb_emu::action_label(corne::LAYERS[0][p as usize]) == want)
        .unwrap_or_else(|| panic!("no key for {c:?}"))
}

/// The emulated keyboard, its two displays, and the frames recorded so far.
pub struct Sim {
    pub emu: Emu,
    pub now: u32,
    panels: [Panel; 2],
    show: Show,
    scale: i32,
    anim: Animation,
}

impl Sim {
    #[must_use]
    pub fn new(show: Show) -> Self {
        Self::with_scale(show, SCALE)
    }

    #[must_use]
    pub fn with_scale(show: Show, scale: i32) -> Self {
        Self {
            emu: Emu::new(),
            now: 0,
            panels: [Panel::default(), Panel::default()],
            show,
            scale,
            anim: Animation::new(&oled::PALETTE),
        }
    }

    fn drain(&mut self) {
        while self.emu.poll(self.now).is_some() {}
    }

    pub fn press(&mut self, pos: u8) {
        self.emu.press(pos, self.now);
        self.drain();
    }

    pub fn release(&mut self, pos: u8) {
        self.emu.release(pos, self.now);
        self.drain();
    }

    /// Press and let go 70 ms later.
    pub fn tap(&mut self, pos: u8) {
        self.press(pos);
        self.run(70);
        self.release(pos);
    }

    /// Type `text` (letters and spaces), a key every `every` ms.
    pub fn type_text(&mut self, text: &str, every: u32) {
        for c in text.chars() {
            self.tap(key_of(c));
            self.run(every.saturating_sub(70));
        }
    }

    /// Play the start-up animation, as after switching the keyboard on.
    pub fn power_on(&mut self) {
        kb_emu::kb_emu_replay_boot(&raw mut self.emu);
    }

    /// Let `ms` milliseconds pass, recording a frame every [`FRAME_MS`].
    pub fn run(&mut self, ms: u32) {
        for _ in 0..ms {
            self.now += 1;
            self.emu.tick(self.now);
            self.drain();
            if self.now.is_multiple_of(FRAME_MS) {
                self.frame();
            }
        }
    }

    fn frame(&mut self) {
        for half in 0..2 {
            for w in self.emu.display_writes(half, self.now) {
                self.panels[half].write(&w);
            }
        }
        let img = oled::compose(&self.panels, self.show, self.scale);
        self.anim.push(img, FRAME_MS);
    }

    /// A message from the computer (as `kbnotify "text"`).
    pub fn notify(&mut self, text: &str) {
        let mut reports = Vec::new();
        encode_notification(text, 1, |r| reports.push(*r));
        for r in reports {
            self.emu.raw_hid(&r, self.now);
        }
        self.drain();
    }

    /// `kbnotify --clear`.
    pub fn clear_notification(&mut self) {
        let mut r = [0u8; REPORT_SIZE];
        r[0] = KIND_CLEAR;
        self.emu.raw_hid(&r, self.now);
        self.drain();
    }

    /// The GIF, its last frame held `hold_ms` longer.
    #[must_use]
    pub fn finish(mut self, hold_ms: u32) -> Vec<u8> {
        self.anim.hold_last(hold_ms);
        self.anim.encode()
    }

    #[must_use]
    pub fn frames(&self) -> usize {
        self.anim.len()
    }
}

/// Rain, the initials E and L building up, sparks, and the scan line that
/// reveals the status screens, on both halves together.
#[must_use]
pub fn startup() -> Vec<u8> {
    let mut s = Sim::new(Show::Both);
    s.power_on();
    s.run(4000);
    s.finish(1500)
}

/// The left half on its own: rain and "TURN ON RIGHT"; then the right half
/// is switched on, the halves link and the start-up plays out.
#[must_use]
pub fn waiting_for_half() -> Vec<u8> {
    let mut s = Sim::new(Show::Left);
    s.emu.set_half_power(1, false, 0);
    s.power_on();
    s.run(4200);
    let now = s.now;
    s.emu.set_half_power(1, true, now);
    s.run(6000);
    s.finish(1200)
}

/// Typing: every key press drops a trail of digital rain on both halves.
#[must_use]
pub fn typing_rain() -> Vec<u8> {
    let mut s = Sim::new(Show::Both);
    s.run(400);
    s.type_text("hello emk", 170);
    s.run(500);
    s.type_text("rain", 120);
    s.run(1800);
    s.finish(300)
}

/// The layer name slides (DEF, SYM, NAV), the MOUSE bar once the pointer
/// moves on nav, CAPS word blinks, and the CTRL and SHIFT bars.
#[must_use]
pub fn layers_and_mods() -> Vec<u8> {
    let mut s = Sim::new(Show::Left);
    s.run(600);
    s.press(pos::SYM);
    s.run(1000);
    s.release(pos::SYM);
    s.run(500);
    s.press(pos::NAV);
    s.run(800);
    s.press(pos::MOUSE_LEFT);
    s.run(500);
    s.release(pos::MOUSE_LEFT);
    s.run(200);
    s.press(pos::MOUSE_UP);
    s.run(300);
    s.release(pos::MOUSE_UP);
    s.run(700);
    s.release(pos::NAV);
    s.run(600);
    // Caps word: T and Y together.
    s.press(pos::T);
    s.run(10);
    s.press(pos::Y);
    s.run(60);
    s.release(pos::T);
    s.release(pos::Y);
    s.run(1300);
    s.type_text("emk", 200);
    s.run(1000);
    s.tap(pos::SPACE);
    s.run(600);
    s.press(pos::LCTRL);
    s.run(500);
    s.press(pos::RSHIFT);
    s.run(1000);
    s.release(pos::RSHIFT);
    s.run(300);
    s.release(pos::LCTRL);
    s.run(700);
    s.finish(600)
}

/// The code the computer shows in the pairing pictures.
pub const PAIR_CODE: u32 = 418_306;

/// Pairing with a code: the computer shows 418306, the left screen asks for
/// it; the digits go in on the symbols layer, one wrong digit is taken back
/// with Backspace, Enter sends it, and the profile shows ✓.
#[must_use]
pub fn pairing() -> Vec<u8> {
    let mut s = Sim::new(Show::Left);
    pairing_script(&mut s);
    s.finish(1500)
}

/// The keys of the pairing pictures (also played by the app's scene).
pub fn pairing_script(s: &mut Sim) {
    s.emu.select_profile(1);
    s.emu.set_profile_open(true);
    s.run(1000);
    let now = s.now;
    s.emu.simulate_pairing(PAIR_CODE, now);
    s.run(900);
    s.press(pos::SYM);
    s.run(150);
    // 4 1 8, a 7 by mistake, Backspace, then 3 0 6.
    for d in [4, 1, 8, 7] {
        s.tap(pos::digit(d));
        s.run(330);
    }
    s.run(200);
    s.tap(pos::BSPC);
    s.run(450);
    for d in [3, 0, 6] {
        s.tap(pos::digit(d));
        s.run(330);
    }
    s.release(pos::SYM);
    s.run(500);
    s.tap(pos::ENTER);
    s.run(2400);
}

/// The radio power key on nav: +8, +4, 0 dBm, each shown for a moment.
#[must_use]
pub fn radio() -> Vec<u8> {
    let mut s = Sim::new(Show::Left);
    s.run(500);
    s.press(pos::NAV);
    s.run(300);
    for _ in 0..3 {
        s.tap(pos::RADIO);
        s.run(1100);
    }
    s.run(500);
    s.release(pos::NAV);
    s.run(1800);
    s.finish(400)
}

/// The right half: a message from the computer builds up out of the rain,
/// decodes, and falls away when it is cleared.
#[must_use]
pub fn message() -> Vec<u8> {
    let mut s = Sim::new(Show::Right);
    s.run(400);
    s.type_text("ok", 160);
    s.run(300);
    s.notify("Build passed on main");
    s.run(4200);
    s.clear_notification();
    s.run(1500);
    s.finish(300)
}

/// Resting: the screens melt down, a cursor blinks, then they go dark.
#[must_use]
pub fn sleep() -> Vec<u8> {
    let mut s = Sim::new(Show::Both);
    s.emu.set_power_config(
        PowerConfig {
            idle_ms: 1200,
            sleep_ms: None,
        },
        0,
    );
    s.run(5400);
    s.finish(800)
}

/// The top of the left screen in six states, side by side, with captions:
/// connected, open, not connected, USB, charging, low battery.
#[must_use]
pub fn profiles_strip() -> (Indexed, &'static [[u8; 3]]) {
    // The status area above the layer line.
    const ROWS: usize = 26;
    let cases: [(&str, Status); 6] = [
        ("connected", Status::default()),
        (
            "open: pair",
            Status {
                profile: 1,
                profile_open: true,
                profile_connected: false,
                ..Status::default()
            },
        ),
        (
            "not connected",
            Status {
                profile: 2,
                profile_connected: false,
                ..Status::default()
            },
        ),
        (
            "USB",
            Status {
                transport: Transport::Usb,
                ..Status::default()
            },
        ),
        (
            "charging",
            Status {
                charging: [true, false],
                battery: [64, 100],
                ..Status::default()
            },
        ),
        (
            "low battery",
            Status {
                battery: [12, 100],
                ..Status::default()
            },
        ),
    ];
    let scale = 5;
    let (gw, gh) = (PORTRAIT_WIDTH as i32 * scale, ROWS as i32 * scale);
    let (pad, gap, caption) = (8, 18, 30);
    let tile_w = gw + 2 * pad;
    let width = 16 * 2 + cases.len() as i32 * tile_w + (cases.len() as i32 - 1) * gap;
    let height = 16 * 2 + gh + 2 * pad + caption;
    let mut img = Indexed::new(width as usize, height as usize, oled::PLATE);
    for (i, (name, status)) in cases.iter().enumerate() {
        let mut frame = Frame::default();
        screens::render(
            &mut frame,
            Half::Left,
            Style::Portrait,
            status,
            corne::KEYMAP.layer_names,
        );
        let panel = Panel::showing(&frame);
        let x = 16 + i as i32 * (tile_w + gap);
        img.fill_round_rect(x, 16, tile_w, gh + 2 * pad, 5, oled::BEZEL);
        oled::draw_glass(&mut img, x + pad, 16 + pad, scale, &panel, 0..ROWS);
        img.text_centered(
            name,
            x + tile_w / 2,
            16 + gh + 2 * pad + 10,
            &embedded_graphics::mono_font::ascii::FONT_9X15,
            1,
            oled::INK,
        );
    }
    (img, &oled::PALETTE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_sit_on_the_symbols_layer_positions() {
        // Arrange
        let digits = [1, 5, 6, 0];

        // Act
        let positions = digits.map(pos::digit);

        // Assert
        assert_eq!(positions, [13, 17, 25, 29]);
    }

    #[test]
    fn letters_are_found_on_the_default_layer() {
        // Arrange
        let text = "hq ";

        // Act
        let positions: Vec<u8> = text.chars().map(key_of).collect();

        // Assert
        assert_eq!(positions, [18, 1, 38]);
    }

    #[test]
    fn a_short_start_up_records_a_looping_gif_of_both_screens() {
        // Arrange
        let mut s = Sim::with_scale(Show::Both, 1);
        s.power_on();

        // Act
        s.run(400);
        let frames = s.frames();
        let bytes = s.finish(0);

        // Assert: GIF89a, several frames, both modules wide
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options.read_info(bytes.as_slice()).expect("a gif");
        let mut decoded = 0;
        while decoder.read_next_frame().expect("a frame").is_some() {
            decoded += 1;
        }
        assert!(frames > 5);
        assert_eq!(decoded, frames);
        assert_eq!(decoder.width(), 2 * 12 + 2 * (32 + 20) + 28);
    }

    #[test]
    fn pairing_ends_with_the_profile_connected() {
        // Arrange
        let mut s = Sim::with_scale(Show::Left, 1);

        // Act
        pairing_script(&mut s);

        // Assert: the code was accepted
        assert!(s.emu.status().profile_connected);
        assert!(!s.emu.status().profile_open);
    }
}

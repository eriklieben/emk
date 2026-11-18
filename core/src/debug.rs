//! Live debug stream on the development log port (`fw-nrf` `usb-log`): what
//! the emulator's DEBUG tab reads to mirror the real keyboard.
//!
//! The port stays a plain text log: every line ends in `\r\n`, log lines
//! start with their timestamp (a space or a digit). After the computer sends
//! `#debug 1` (see [`crate::console`]), the keyboard also sends event lines,
//! each starting with `#`, a tag and a space, between log lines:
//!
//! ```text
//! #H 1 erik_kyb_blk left ble        hello: protocol version, name, half, build
//! #S layer=0 caps=0 mods=00 ...     status (key=value, see `status`), when it changes
//! #K 0000000000001001               held key positions, bit n = position n (hex u64)
//! #R 0200040000000000               last boot report for the computer: mods, 0, 6 keys
//! #M 01d4feb004ff01                 last mouse report: buttons, x, y (i16 LE), wheel, pan
//! #F <base64 of 512 bytes>          left OLED frame as drawn (SSD1306 page layout)
//! #F -                              left OLED switched off
//! ```
//!
//! Each event carries the whole state, never a delta, so a reader that
//! misses one (or connects late) is up to date with the next. `#debug 1`
//! again sends everything once more.

use core::fmt::{self, Write};

use crate::link::Output;
use crate::power::PowerState;

/// Protocol version in the hello line; bumped when a line changes meaning.
pub const VERSION: u8 = 1;

/// Bytes of an SSD1306 128×32 frame.
pub const FRAME_BYTES: usize = 512;

/// Length of a frame line: `#F `, base64, `\r\n`.
pub const FRAME_LINE: usize = 3 + FRAME_BYTES.div_ceil(3) * 4 + 2;

/// A line built in place; text that doesn't fit is cut off.
pub struct Line<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> Line<N> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    fn push(&mut self, b: u8) {
        if self.len < N {
            self.buf[self.len] = b;
            self.len += 1;
        }
    }
}

impl<const N: usize> Default for Line<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Write for Line<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            self.push(b);
        }
        Ok(())
    }
}

/// What the status line reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent fields of the status line"
)]
pub struct StatusFields<'a> {
    pub layer: u8,
    pub caps_word: bool,
    /// HID modifier bits.
    pub mods: u8,
    pub output: Output,
    /// Selected Bluetooth profile, 0-based (sent 1-based, as on the screen).
    pub profile: u8,
    /// Whether the selected profile's computer is connected.
    pub profile_connected: bool,
    /// Whether the selected profile has nothing paired yet.
    pub profile_open: bool,
    /// Whether the right half is linked.
    pub linked: bool,
    /// Split link stage (`scan`, `linked`, ...); empty for none.
    pub stage: &'a str,
    /// Battery percentage of the left and right half.
    pub battery: [u8; 2],
    /// Whether each half is on USB power. The left half doesn't know the
    /// right half's, so that one is always false.
    pub charging: [bool; 2],
    pub wpm: u16,
    pub power: PowerState,
    /// Key presses so far (wrapping), the digital rain's seed.
    pub presses: u16,
    /// Reports traced but not sent to the computer (`m`).
    pub mute: bool,
    /// Reports logged (`t`).
    pub trace: bool,
    /// Whether the left OLED is on.
    pub lit: bool,
    /// Radio transmit power in dBm (`RadioPower`). Added last, so older
    /// readers that stop at the fields they know still work.
    pub radio_dbm: i8,
    /// Whether the mouse pointer is active (the nav thumbs click). After
    /// `radio`, for the same reason.
    pub pointer: bool,
}

fn bit(b: bool) -> u8 {
    u8::from(b)
}

/// `#H 1 <name> <half> <build>\r\n`.
pub fn hello<const N: usize>(out: &mut Line<N>, name: &str, half: &str, build: &str) {
    out.clear();
    let _ = write!(out, "#H {VERSION} {name} {half} {build}\r\n");
}

/// `#S key=value ...\r\n`.
pub fn status<const N: usize>(out: &mut Line<N>, s: &StatusFields<'_>) {
    out.clear();
    let output = match s.output {
        Output::Usb => "usb",
        Output::Ble => "ble",
    };
    let power = match s.power {
        PowerState::Active => "active",
        PowerState::Idle => "idle",
        PowerState::Sleep => "sleep",
    };
    let stage = if s.stage.is_empty() { "-" } else { s.stage };
    let _ = write!(
        out,
        "#S layer={} caps={} mods={:02x} out={} profile={} host={} open={} linked={} stage={} \
         bat={},{} chg={},{} wpm={} power={} presses={} mute={} trace={} lit={} radio={} pointer={}\r\n",
        s.layer,
        bit(s.caps_word),
        s.mods,
        output,
        u16::from(s.profile) + 1,
        bit(s.profile_connected),
        bit(s.profile_open),
        bit(s.linked),
        stage,
        s.battery[0],
        s.battery[1],
        bit(s.charging[0]),
        bit(s.charging[1]),
        s.wpm,
        power,
        s.presses,
        bit(s.mute),
        bit(s.trace),
        bit(s.lit),
        s.radio_dbm,
        bit(s.pointer),
    );
}

/// `#K <16 hex digits>\r\n`: bit n set while position n is held.
pub fn keys<const N: usize>(out: &mut Line<N>, held: u64) {
    out.clear();
    let _ = write!(out, "#K {held:016x}\r\n");
}

/// `#R <16 hex digits>\r\n`: the 8-byte boot keyboard report.
pub fn report<const N: usize>(out: &mut Line<N>, report: &[u8; 8]) {
    out.clear();
    let _ = out.write_str("#R ");
    for b in report {
        let _ = write!(out, "{b:02x}");
    }
    let _ = out.write_str("\r\n");
}

/// `#M <14 hex digits>\r\n`: the 7-byte mouse report
/// (`kb_core::mouse::MouseReport::bytes`).
pub fn mouse<const N: usize>(out: &mut Line<N>, report: &[u8; crate::mouse::REPORT_LEN]) {
    out.clear();
    let _ = out.write_str("#M ");
    for b in report {
        let _ = write!(out, "{b:02x}");
    }
    let _ = out.write_str("\r\n");
}

/// `#F <base64>\r\n` for a frame that is shown, `#F -\r\n` when the display is off.
/// `N` should be at least [`FRAME_LINE`].
pub fn frame<const N: usize>(out: &mut Line<N>, frame: Option<&[u8; FRAME_BYTES]>) {
    out.clear();
    let _ = out.write_str("#F ");
    match frame {
        Some(bytes) => base64(out, bytes),
        None => out.push(b'-'),
    }
    let _ = out.write_str("\r\n");
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with `=` padding.
fn base64<const N: usize>(out: &mut Line<N>, bytes: &[u8]) {
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * i) & 63) as usize]);
            } else {
                out.push(b'=');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text<const N: usize>(line: &Line<N>) -> &str {
        core::str::from_utf8(line.as_bytes()).unwrap()
    }

    fn fields() -> StatusFields<'static> {
        StatusFields {
            layer: 2,
            caps_word: true,
            mods: 0x22,
            output: Output::Ble,
            profile: 0,
            profile_connected: true,
            profile_open: false,
            linked: true,
            stage: "linked",
            battery: [87, 90],
            charging: [true, false],
            wpm: 42,
            power: PowerState::Active,
            presses: 1234,
            mute: false,
            trace: true,
            lit: true,
            radio_dbm: 4,
            pointer: true,
        }
    }

    fn decode_base64(s: &str) -> Vec<u8> {
        let value = |c: u8| BASE64.iter().position(|&b| b == c).unwrap() as u32;
        let mut out = Vec::new();
        for quad in s.as_bytes().chunks(4) {
            #[expect(
                clippy::naive_bytecount,
                reason = "a test helper; no extra crate for it"
            )]
            let pad = quad.iter().filter(|&&c| c == b'=').count();
            let n = quad
                .iter()
                .map(|&c| if c == b'=' { 0 } else { value(c) })
                .fold(0, |n, v| n << 6 | v);
            let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
            out.extend_from_slice(&bytes[..3 - pad]);
        }
        out
    }

    #[test]
    fn hello_names_version_keyboard_and_build() {
        // Arrange
        let mut line = Line::<64>::new();

        // Act
        hello(&mut line, "erik_kyb_blk", "left", "ble");

        // Assert
        assert_eq!(text(&line), "#H 1 erik_kyb_blk left ble\r\n");
    }

    #[test]
    fn status_lists_every_field() {
        // Arrange
        let mut line = Line::<192>::new();

        // Act
        status(&mut line, &fields());

        // Assert
        assert_eq!(
            text(&line),
            "#S layer=2 caps=1 mods=22 out=ble profile=1 host=1 open=0 linked=1 stage=linked \
             bat=87,90 chg=1,0 wpm=42 power=active presses=1234 mute=0 trace=1 lit=1 radio=4 pointer=1\r\n"
        );
    }

    #[test]
    fn status_without_a_stage_says_dash() {
        // Arrange
        let mut line = Line::<192>::new();
        let s = StatusFields {
            stage: "",
            ..fields()
        };

        // Act
        status(&mut line, &s);

        // Assert
        assert!(text(&line).contains(" stage=- "));
    }

    #[test]
    fn keys_are_a_fixed_width_bitmap() {
        // Arrange
        let mut line = Line::<32>::new();

        // Act
        keys(&mut line, 1 << 41 | 1 << 12 | 1);

        // Assert
        assert_eq!(text(&line), "#K 0000020000001001\r\n");
    }

    #[test]
    fn report_is_hex_bytes_in_order() {
        // Arrange
        let mut line = Line::<32>::new();

        // Act
        report(&mut line, &[0x02, 0, 0x04, 0x05, 0, 0, 0, 0]);

        // Assert
        assert_eq!(text(&line), "#R 0200040500000000\r\n");
    }

    #[test]
    fn mouse_report_is_hex_bytes_in_order() {
        // Arrange
        let mut line = Line::<32>::new();
        let report = crate::mouse::MouseReport {
            buttons: 1,
            x: -300,
            y: 1200,
            wheel: -1,
            pan: 1,
        };

        // Act
        mouse(&mut line, &report.bytes());

        // Assert
        assert_eq!(text(&line), "#M 01d4feb004ff01\r\n");
    }

    #[test]
    fn frame_line_has_the_documented_length() {
        // Arrange
        let mut line = Line::<FRAME_LINE>::new();

        // Act
        frame(&mut line, Some(&[0xA5; FRAME_BYTES]));

        // Assert
        assert_eq!(line.as_bytes().len(), FRAME_LINE);
    }

    #[test]
    fn frame_round_trips_through_base64() {
        // Arrange
        let mut bytes = [0u8; FRAME_BYTES];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i * 7 % 256) as u8;
        }
        let mut line = Line::<FRAME_LINE>::new();

        // Act
        frame(&mut line, Some(&bytes));

        // Assert
        let payload = text(&line).strip_prefix("#F ").unwrap().trim_end();
        assert_eq!(decode_base64(payload), bytes);
    }

    #[test]
    fn frame_off_is_a_dash() {
        // Arrange
        let mut line = Line::<FRAME_LINE>::new();

        // Act
        frame(&mut line, None);

        // Assert
        assert_eq!(text(&line), "#F -\r\n");
    }

    #[test]
    fn base64_pads_short_input() {
        // Arrange
        let mut line = Line::<16>::new();

        // Act
        base64(&mut line, b"ab");

        // Assert
        assert_eq!(text(&line), "YWI=");
    }

    #[test]
    fn line_cuts_off_what_does_not_fit() {
        // Arrange
        let mut line = Line::<4>::new();

        // Act
        let _ = line.write_str("#K 0123");

        // Assert
        assert_eq!(text(&line), "#K 0");
    }
}

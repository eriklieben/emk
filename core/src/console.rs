//! Commands on the development log port (`fw-nrf` `usb-log`), so a test
//! script can drive the keyboard: one command per line, or the one-letter
//! ones on their own.
//!
//! - `s` status snapshot, `d` replay the whole log, `t` toggle report tracing,
//!   `m` toggle mute (reports are traced but not sent: tests without typing),
//!   `boot` restart into the bootloader (to flash without double-tapping reset:
//!   UF2 drive, or serial DFU on a signed-only bootloader)
//! - `p12` / `r12` press / release key position 12, `k12` tap it
//! - `#debug 1` / `#debug 0` start / stop the live debug stream for this
//!   connection (see [`crate::debug`]); `#debug 1` again resends everything
//! - `#studio <command>` the keymap editor's protocol
//!   ([`crate::studio::protocol`]), up to [`LINE_MAX`] bytes a line
//!
//! A computer that echoes what it reads (a terminal with `echo` on) sends the
//! log back: log lines start with their timestamp and the debug lines with
//! `#`, and a line too long to be a command is dropped up to its end, so
//! none of that can turn into a command.

use crate::studio::protocol::{self, StudioCommand};

/// Longest command line the firmware reads (a `#studio set` with the
/// longest action, and some room); longer lines are dropped.
pub const LINE_MAX: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Dump,
    Trace,
    Mute,
    Bootloader,
    Press(u8),
    Release(u8),
    Tap(u8),
    /// Start (`true`) or stop the live debug stream.
    Debug(bool),
    /// A Studio command.
    Studio(StudioCommand),
}

/// Parse one line (without its line ending). `None` for anything else.
pub fn parse(line: &[u8]) -> Option<Command> {
    let line = line.trim_ascii();
    match line {
        b"#debug" | b"#debug 1" => return Some(Command::Debug(true)),
        b"#debug 0" => return Some(Command::Debug(false)),
        _ => {}
    }
    if let Some(words) = line.strip_prefix(b"#studio") {
        return match words.split_first() {
            Some((b' ', words)) => Some(Command::Studio(protocol::parse(words))),
            None => Some(Command::Studio(StudioCommand::Invalid)),
            // `#studiox`: not ours.
            Some(_) => None,
        };
    }
    let (&first, rest) = line.split_first()?;
    let position = || -> Option<u8> {
        if rest.is_empty() || rest.len() > 3 || !rest.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let n = rest.iter().fold(0u16, |n, d| n * 10 + u16::from(d - b'0'));
        u8::try_from(n).ok()
    };
    match first {
        b's' if rest.is_empty() => Some(Command::Status),
        b'd' if rest.is_empty() => Some(Command::Dump),
        b't' if rest.is_empty() => Some(Command::Trace),
        b'm' if rest.is_empty() => Some(Command::Mute),
        // Two letters, so a stray `b` can't restart the keyboard.
        b'b' if rest == b"oot" => Some(Command::Bootloader),
        b'p' => position().map(Command::Press),
        b'r' => position().map(Command::Release),
        b'k' => position().map(Command::Tap),
        _ => None,
    }
}

/// Splits incoming bytes into lines; the one-letter commands also count on
/// their own, so `printf s > /dev/ttyACM0` works without a newline.
pub struct LineReader<const N: usize> {
    buf: [u8; N],
    len: usize,
    /// The current line got too long: skip to its end.
    discarding: bool,
}

impl<const N: usize> LineReader<N> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            discarding: false,
        }
    }

    /// Forget a partial line (a new connection starts afresh).
    pub fn reset(&mut self) {
        self.len = 0;
        self.discarding = false;
    }

    /// Feed bytes; calls `on_command` for every complete command.
    pub fn feed(&mut self, bytes: &[u8], mut on_command: impl FnMut(Command)) {
        for &b in bytes {
            match b {
                b'\r' | b'\n' => {
                    if !self.discarding
                        && let Some(c) = parse(&self.buf[..self.len])
                    {
                        on_command(c);
                    }
                    self.reset();
                }
                _ if self.discarding => {}
                b's' | b'd' | b't' | b'm' if self.len == 0 => {
                    if let Some(c) = parse(&[b]) {
                        on_command(c);
                    }
                }
                _ if self.len < N => {
                    self.buf[self.len] = b;
                    self.len += 1;
                }
                // Too long to be a command: drop the rest of the line.
                _ => {
                    self.len = 0;
                    self.discarding = true;
                }
            }
        }
    }
}

impl<const N: usize> Default for LineReader<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(input: &[u8]) -> Vec<Command> {
        let mut reader = LineReader::<16>::new();
        let mut out = Vec::new();
        reader.feed(input, |c| out.push(c));
        out
    }

    #[test]
    fn single_letter_works_without_a_newline() {
        // Arrange / Act
        let got = commands(b"stm");

        // Assert
        assert_eq!(got, [Command::Status, Command::Trace, Command::Mute]);
    }

    #[test]
    fn key_commands_need_a_line_ending() {
        // Arrange / Act
        let got = commands(b"k12");

        // Assert
        assert_eq!(got, []);
    }

    #[test]
    fn key_commands_parse_their_position() {
        // Arrange / Act
        let got = commands(b"p3\nr3\r\nk41\n");

        // Assert
        assert_eq!(
            got,
            [Command::Press(3), Command::Release(3), Command::Tap(41)]
        );
    }

    #[test]
    fn commands_split_over_packets_are_joined() {
        // Arrange
        let mut reader = LineReader::<16>::new();
        let mut out = Vec::new();
        reader.feed(b"k1", |c| out.push(c));

        // Act
        reader.feed(b"7\n", |c| out.push(c));

        // Assert
        assert_eq!(out, [Command::Tap(17)]);
    }

    #[test]
    fn garbage_is_ignored() {
        // Arrange / Act
        let got = commands(b"x\nk\nk1234\np-1\nkab\n");

        // Assert
        assert_eq!(got, []);
    }

    #[test]
    fn bootloader_needs_the_whole_word() {
        // Arrange / Act
        let got = commands(b"b\nbo\nboot\n");

        // Assert
        assert_eq!(got, [Command::Bootloader]);
    }

    #[test]
    fn echoed_log_lines_are_not_commands() {
        // Arrange: a log line read back, with letters after each 16 bytes
        let echo = b"  157951 host: another profile's computer; dropping\r\n   1234 split: scan, then status m\r\n";

        // Act
        let got = commands(echo);

        // Assert
        assert_eq!(got, []);
    }

    #[test]
    fn echoed_debug_lines_are_not_commands() {
        // Arrange / Act
        let got = commands(b"#S layer=0 caps=0 mods=00 out=usb\r\n#K 0000000000000000\r\n");

        // Assert
        assert_eq!(got, []);
    }

    #[test]
    fn a_command_after_a_long_line_still_works() {
        // Arrange / Act
        let got = commands(b"a line far too long for a command\ns\nk3\n");

        // Assert
        assert_eq!(got, [Command::Status, Command::Tap(3)]);
    }

    #[test]
    fn debug_switches_on_and_off() {
        // Arrange / Act
        let got = commands(b"#debug 1\n#debug 0\n#debug\n");

        // Assert
        assert_eq!(
            got,
            [
                Command::Debug(true),
                Command::Debug(false),
                Command::Debug(true)
            ]
        );
    }

    #[test]
    fn reset_drops_a_partial_line() {
        // Arrange
        let mut reader = LineReader::<16>::new();
        let mut out = Vec::new();
        reader.feed(b"k1", |c| out.push(c));
        reader.reset();

        // Act
        reader.feed(b"2\n", |c| out.push(c));

        // Assert
        assert_eq!(out, []);
    }

    #[test]
    fn studio_command_is_parsed() {
        // Arrange
        let mut reader = LineReader::<LINE_MAX>::new();
        let mut out = Vec::new();

        // Act
        reader.feed(b"#studio set 0 12 kp:LC(C)\r\n", |c| out.push(c));

        // Assert
        assert_eq!(
            out,
            [Command::Studio(StudioCommand::Set {
                layer: 0,
                pos: 12,
                action: Ok(crate::action::kp(crate::keys::lc(crate::keys::C)))
            })]
        );
    }

    #[test]
    fn bare_studio_is_an_invalid_studio_command() {
        // Arrange / Act
        let got = parse(b"#studio");

        // Assert
        assert_eq!(got, Some(Command::Studio(StudioCommand::Invalid)));
    }

    #[test]
    fn studio_needs_a_space_after_it() {
        // Arrange / Act
        let got = parse(b"#studiox get");

        // Assert
        assert_eq!(got, None);
    }

    #[test]
    fn echoed_studio_lines_are_not_commands() {
        // Arrange: what the keyboard sends, read back
        let mut reader = LineReader::<LINE_MAX>::new();
        let mut out = Vec::new();

        // Act
        reader.feed(
            b"#A set 0 1 ok kp:Z\r\n#U unlocked 300\r\n#D 1\r\n#V 1 layers=4 keys=42 store=flash\r\n",
            |c| out.push(c),
        );

        // Assert
        assert_eq!(out, []);
    }

    #[test]
    fn a_short_reader_drops_studio_set_lines() {
        // Arrange: old firmware read at most 16 bytes a line
        let mut reader = LineReader::<16>::new();
        let mut out = Vec::new();

        // Act
        reader.feed(b"#studio set 0 12 kp:A\n", |c| out.push(c));

        // Assert
        assert_eq!(out, []);
    }

    #[test]
    fn studio_line_of_the_longest_length_is_read() {
        // Arrange: a combo check padded to exactly LINE_MAX bytes
        let mut line = b"#studio combo check new abc 1,2 * 50 0 kp:A".to_vec();
        line.resize(LINE_MAX, b' ');
        line.push(b'\n');
        let mut reader = LineReader::<LINE_MAX>::new();
        let mut out = Vec::new();

        // Act
        reader.feed(&line, |c| out.push(c));

        // Assert
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn studio_line_longer_than_the_limit_is_dropped() {
        // Arrange
        let mut line = b"#studio get".to_vec();
        line.resize(LINE_MAX + 1, b' ');
        line.push(b'\n');
        let mut reader = LineReader::<LINE_MAX>::new();
        let mut out = Vec::new();

        // Act
        reader.feed(&line, |c| out.push(c));

        // Assert
        assert_eq!(out, []);
    }

    #[test]
    fn position_above_255_is_rejected() {
        // Arrange / Act
        let got = parse(b"k300");

        // Assert
        assert_eq!(got, None);
    }
}

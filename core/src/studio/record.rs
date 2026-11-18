//! The edited keymap as stored in flash: what differs from the compiled
//! keymap, in records small enough for the firmware's key-value map (every
//! item there must fit the 128-byte buffers its readers use).
//!
//! The positions of a layer are cut into blocks of [`BLOCK_KEYS`]; each
//! block that has an edit gets a record of its own (a 42-key layer: two
//! blocks, so at most 8 records for 4 layers). Saving writes only the
//! records whose block changed, which keeps flash wear down to one small
//! record per edited half-layer.
//!
//! ```text
//! byte 0     format version (FORMAT_VERSION)
//! byte 1     layer
//! byte 2     block (positions block*21 .. block*21+20)
//! bytes 3-5  which positions of the block are overridden, bit i = position block*21+i (little endian)
//! then       one action per set bit, in position order:
//!   00 trans | 01 none | 02 caps word | 03 key: code, mods | 04 mo: layer
//!   05 mod-tap: hold code, hold mods, tap code, tap mods | 06 system: SystemAction number
//!   07 mouse: kind << 4 | which (0 move, 1 button, 2 scroll: direction
//!      left down up right / button left right middle; 0x30 precision)
//!   08 mouse or key: mouse (as 07: a button or scroll), key code, key mods
//! ```
//!
//! A record with no bit set means "nothing edited here" (flash written
//! through MPSL can't drop an item, so clearing writes this instead). A
//! record of another version is skipped as a whole, so a future format can
//! migrate (or ignore) old ones. Combos, once they can be edited, get records
//! of their own under other keys with a format of their own.

use crate::action::{Action, Direction, Key, MouseAction, MouseButton, SystemAction};

/// Version of the record layout; a reader skips records of other versions.
pub const FORMAT_VERSION: u8 = 1;

/// Positions per record.
pub const BLOCK_KEYS: usize = 21;

/// Bytes of the longest action (a mod-tap).
pub const ACTION_MAX: usize = 5;

/// Bytes of the header (version, layer, block, bitmap).
const HEADER: usize = 6;

/// Bytes of the largest record: every position of a block a mod-tap.
pub const RECORD_MAX: usize = HEADER + BLOCK_KEYS * ACTION_MAX;

/// Blocks a layer of `keys` positions takes.
#[must_use]
pub const fn blocks_per_layer(keys: usize) -> usize {
    keys.div_ceil(BLOCK_KEYS)
}

/// Why a record was not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// Written by another format version (the number is in it).
    Version(u8),
    /// Too short for what its header says, or bytes left over.
    Length,
    /// A layer, block or position this keymap doesn't have.
    Range,
    /// An action this firmware doesn't know (or a layer it doesn't have).
    Action,
}

fn write_key(k: Key, out: &mut [u8]) {
    out[0] = k.code;
    out[1] = k.mods;
}

/// Write `action`; returns its length.
pub fn write_action(action: Action, out: &mut [u8; ACTION_MAX]) -> usize {
    match action {
        Action::Trans => {
            out[0] = 0;
            1
        }
        Action::None => {
            out[0] = 1;
            1
        }
        Action::CapsWord => {
            out[0] = 2;
            1
        }
        Action::Key(k) => {
            out[0] = 3;
            write_key(k, &mut out[1..]);
            3
        }
        Action::MomentaryLayer(l) => {
            out[0] = 4;
            out[1] = l;
            2
        }
        Action::ModTap { hold, tap } => {
            out[0] = 5;
            write_key(hold, &mut out[1..]);
            write_key(tap, &mut out[3..]);
            5
        }
        Action::System(s) => {
            out[0] = 6;
            out[1] = s.number();
            2
        }
        Action::Mouse(m) => {
            out[0] = 7;
            out[1] = mouse_byte(m);
            2
        }
        Action::MouseOrKey { mouse, key } => {
            out[0] = 8;
            out[1] = mouse_byte(mouse);
            write_key(key, &mut out[2..]);
            4
        }
    }
}

fn mouse_byte(m: MouseAction) -> u8 {
    match m {
        MouseAction::Move(d) => d as u8,
        MouseAction::Button(b) => 0x10 | b as u8,
        MouseAction::Scroll(d) => 0x20 | d as u8,
        MouseAction::Precision => 0x30,
    }
}

fn mouse_from_byte(b: u8) -> Option<MouseAction> {
    let low = (b & 0x0F) as usize;
    match b >> 4 {
        0 => Direction::ALL.get(low).map(|&d| MouseAction::Move(d)),
        1 => MouseButton::ALL.get(low).map(|&b| MouseAction::Button(b)),
        2 => Direction::ALL.get(low).map(|&d| MouseAction::Scroll(d)),
        3 if low == 0 => Some(MouseAction::Precision),
        _ => None,
    }
}

/// Read one action from the start of `bytes`; the action and its length.
///
/// # Errors
///
/// [`RecordError::Length`] when `bytes` end too early,
/// [`RecordError::Action`] for an action this firmware doesn't know.
pub fn read_action(bytes: &[u8]) -> Result<(Action, usize), RecordError> {
    let byte = |i: usize| bytes.get(i).copied().ok_or(RecordError::Length);
    let key = |i: usize| -> Result<Key, RecordError> {
        Ok(Key {
            code: byte(i)?,
            mods: byte(i + 1)?,
        })
    };
    Ok(match byte(0)? {
        0 => (Action::Trans, 1),
        1 => (Action::None, 1),
        2 => (Action::CapsWord, 1),
        3 => (Action::Key(key(1)?), 3),
        4 => (Action::MomentaryLayer(byte(1)?), 2),
        5 => (
            Action::ModTap {
                hold: key(1)?,
                tap: key(3)?,
            },
            5,
        ),
        6 => (
            Action::System(SystemAction::from_number(byte(1)?).ok_or(RecordError::Action)?),
            2,
        ),
        7 => (
            Action::Mouse(mouse_from_byte(byte(1)?).ok_or(RecordError::Action)?),
            2,
        ),
        8 => (
            Action::MouseOrKey {
                mouse: mouse_from_byte(byte(1)?)
                    .filter(|m| matches!(m, MouseAction::Button(_) | MouseAction::Scroll(_)))
                    .ok_or(RecordError::Action)?,
                key: key(2)?,
            },
            4,
        ),
        _ => return Err(RecordError::Action),
    })
}

/// The record for `block` of `layer`: the positions where `live` differs
/// from `default` (both the whole layer). Returns its length.
pub fn encode_block(
    layer: u8,
    block: u8,
    live: &[Action],
    default: &[Action],
    out: &mut [u8; RECORD_MAX],
) -> usize {
    out[0] = FORMAT_VERSION;
    out[1] = layer;
    out[2] = block;
    let start = block as usize * BLOCK_KEYS;
    let end = (start + BLOCK_KEYS).min(live.len());
    let mut bits = 0u32;
    let mut len = HEADER;
    for pos in start..end {
        if live[pos] != default[pos] {
            bits |= 1 << (pos - start);
            let mut action = [0u8; ACTION_MAX];
            let n = write_action(live[pos], &mut action);
            out[len..len + n].copy_from_slice(&action[..n]);
            len += n;
        }
    }
    out[3..6].copy_from_slice(&bits.to_le_bytes()[..3]);
    len
}

/// A record read back: which layer and block, and the overridden
/// positions of the block (`None`: as compiled).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub layer: u8,
    pub block: u8,
    pub actions: [Option<Action>; BLOCK_KEYS],
}

impl Block {
    /// Whether it overrides anything.
    pub fn is_empty(&self) -> bool {
        self.actions.iter().all(Option::is_none)
    }
}

/// Read a record, checked whole (a record is applied entirely or not at all).
///
/// # Errors
///
/// Why the record can't be applied.
pub fn decode_block(bytes: &[u8]) -> Result<Block, RecordError> {
    let version = *bytes.first().ok_or(RecordError::Length)?;
    if version != FORMAT_VERSION {
        return Err(RecordError::Version(version));
    }
    if bytes.len() < HEADER {
        return Err(RecordError::Length);
    }
    let bits = u32::from_le_bytes([bytes[3], bytes[4], bytes[5], 0]);
    if bits >> BLOCK_KEYS != 0 {
        return Err(RecordError::Range);
    }
    let mut block = Block {
        layer: bytes[1],
        block: bytes[2],
        actions: [None; BLOCK_KEYS],
    };
    let mut at = HEADER;
    for (i, slot) in block.actions.iter_mut().enumerate() {
        if bits & (1 << i) != 0 {
            let (action, n) = read_action(&bytes[at..])?;
            *slot = Some(action);
            at += n;
        }
    }
    if at != bytes.len() {
        return Err(RecordError::Length);
    }
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{CAPS_WORD, NONE, TRANS, kp, mo, mt, sys};
    use crate::keys::{A, B, LALT, TAB, lc};

    fn layer() -> [Action; 42] {
        [kp(A); 42]
    }

    #[test]
    fn every_kind_of_action_round_trips() {
        // Arrange
        let actions = [
            TRANS,
            NONE,
            CAPS_WORD,
            kp(lc(A)),
            mo(3),
            mt(LALT, TAB),
            sys(SystemAction::StudioUnlock),
        ];

        // Act
        let read: Vec<_> = actions
            .iter()
            .map(|&a| {
                let mut buf = [0; ACTION_MAX];
                let n = write_action(a, &mut buf);
                read_action(&buf[..n])
            })
            .collect();

        // Assert
        let expected: Vec<_> = actions
            .iter()
            .map(|&a| {
                let mut buf = [0; ACTION_MAX];
                Ok((a, write_action(a, &mut buf)))
            })
            .collect();
        assert_eq!(read, expected);
    }

    #[test]
    fn every_mouse_action_round_trips() {
        // Arrange
        let mut actions = Vec::new();
        for d in Direction::ALL {
            actions.push(Action::Mouse(MouseAction::Move(d)));
            actions.push(Action::Mouse(MouseAction::Scroll(d)));
            actions.push(Action::MouseOrKey {
                mouse: MouseAction::Scroll(d),
                key: lc(A),
            });
        }
        for b in MouseButton::ALL {
            actions.push(Action::Mouse(MouseAction::Button(b)));
            actions.push(Action::MouseOrKey {
                mouse: MouseAction::Button(b),
                key: lc(A),
            });
        }
        actions.push(Action::Mouse(MouseAction::Precision));

        // Act
        let read: Vec<_> = actions
            .iter()
            .map(|&a| {
                let mut buf = [0; ACTION_MAX];
                let n = write_action(a, &mut buf);
                read_action(&buf[..n]).map(|(a, _)| a)
            })
            .collect();

        // Assert
        assert_eq!(read, actions.into_iter().map(Ok).collect::<Vec<_>>());
    }

    #[test]
    fn unknown_mouse_bytes_are_refused() {
        // Arrange: a bad direction, button, precision; mouse-or-key with a
        // bad mouse part or a movement
        let inputs: [&[u8]; 5] = [
            &[7, 0x04],
            &[7, 0x13],
            &[7, 0x31],
            &[8, 0x13, 4, 0],
            &[8, 0x00, 4, 0],
        ];

        // Act
        let got: Vec<_> = inputs.iter().map(|b| read_action(b)).collect();

        // Assert
        assert!(
            got.iter().all(|r| *r == Err(RecordError::Action)),
            "{got:?}"
        );
    }

    #[test]
    fn unedited_block_is_just_a_header() {
        // Arrange
        let default = layer();
        let mut out = [0; RECORD_MAX];

        // Act
        let len = encode_block(1, 0, &default, &default, &mut out);

        // Assert
        assert_eq!(&out[..len], &[FORMAT_VERSION, 1, 0, 0, 0, 0]);
    }

    #[test]
    fn edited_positions_come_back_from_the_record() {
        // Arrange
        let default = layer();
        let mut live = layer();
        live[21] = kp(B);
        live[41] = mt(LALT, TAB);
        let mut out = [0; RECORD_MAX];
        let len = encode_block(2, 1, &live, &default, &mut out);

        // Act
        let block = decode_block(&out[..len]).unwrap();

        // Assert
        let mut expected = [None; BLOCK_KEYS];
        expected[0] = Some(kp(B));
        expected[20] = Some(mt(LALT, TAB));
        assert_eq!((block.layer, block.block, block.actions), (2, 1, expected));
    }

    #[test]
    fn a_block_of_mod_taps_fits_the_largest_record() {
        // Arrange
        let default = layer();
        let live = [mt(LALT, TAB); 42];
        let mut out = [0; RECORD_MAX];

        // Act
        let len = encode_block(0, 0, &live, &default, &mut out);

        // Assert
        assert_eq!(len, RECORD_MAX);
    }

    #[test]
    fn largest_record_fits_the_storage_buffer_with_its_key() {
        // Arrange: kb-ble's map readers use 128-byte buffers; one key byte,
        // rounded up to the flash's 4-byte words.
        let item = (1 + RECORD_MAX).div_ceil(4) * 4;

        // Act
        let fits = item <= 128;

        // Assert
        assert!(fits, "{item}");
    }

    #[test]
    fn record_of_another_version_is_refused() {
        // Arrange
        let bytes = [2, 0, 0, 1, 0, 0, 1];

        // Act
        let got = decode_block(&bytes);

        // Assert
        assert_eq!(got, Err(RecordError::Version(2)));
    }

    #[test]
    fn truncated_record_is_refused() {
        // Arrange: says position 0 is a key, but the key's bytes are missing
        let bytes = [FORMAT_VERSION, 0, 0, 1, 0, 0, 3, 4];

        // Act
        let got = decode_block(&bytes);

        // Assert
        assert_eq!(got, Err(RecordError::Length));
    }

    #[test]
    fn bytes_left_over_are_refused() {
        // Arrange
        let bytes = [FORMAT_VERSION, 0, 0, 0, 0, 0, 1];

        // Act
        let got = decode_block(&bytes);

        // Assert
        assert_eq!(got, Err(RecordError::Length));
    }

    #[test]
    fn unknown_action_is_refused() {
        // Arrange
        let bytes = [FORMAT_VERSION, 0, 0, 1, 0, 0, 9];

        // Act
        let got = decode_block(&bytes);

        // Assert
        assert_eq!(got, Err(RecordError::Action));
    }

    #[test]
    fn unknown_system_action_is_refused() {
        // Arrange
        let bytes = [FORMAT_VERSION, 0, 0, 1, 0, 0, 6, 99];

        // Act
        let got = decode_block(&bytes);

        // Assert
        assert_eq!(got, Err(RecordError::Action));
    }

    #[test]
    fn bits_past_the_block_are_refused() {
        // Arrange
        let bytes = [FORMAT_VERSION, 0, 0, 0, 0, 0x20];

        // Act
        let got = decode_block(&bytes);

        // Assert
        assert_eq!(got, Err(RecordError::Range));
    }

    #[test]
    fn empty_input_is_refused() {
        // Arrange
        let bytes: [u8; 0] = [];

        // Act
        let got = decode_block(&bytes);

        // Assert
        assert_eq!(got, Err(RecordError::Length));
    }
}

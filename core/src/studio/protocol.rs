//! The Studio protocol on the development log port, next to the debug
//! stream (`crate::debug`): text lines, `\r\n` at the end. The computer sends
//! `#studio ...` commands (parsed by `crate::console`), the keyboard answers
//! with `#` lines a reader of the debug stream already skips if it doesn't
//! know them. A keyboard without Studio ignores the commands.
//!
//! ```text
//! #studio get                     → #V, #L per layer, #C per combo + #C end, #U, #D
//! #studio status                  → #U, #D
//! #studio combos                  → #C per combo, #C end <count>
//! #studio defaults                → #LD per layer, #CD per combo, #CD end <count>
//! #studio set <l> <p> <a>         → #A set <l> <p> ok <a> | err <why>, #U, #D
//! #studio check <l> <p> <a>       → #A check <l> <p> ok <a> | err <why>     (changes nothing)
//! #studio combo add <combo>       → #A combo add <i> ok <combo> | #A combo add new err <why>, #C list
//! #studio combo set <i> <combo>   → #A combo set <i> ok <combo> | err <why>, #C list
//! #studio combo remove <i>        → #A combo remove <i> ok | err <why>, #C list
//! #studio combo check <i|new> <combo> → #A combo check <i|new> ok <combo> | err <why>
//! #studio save | revert | reset | lock → #A <op> ok | err <why>, ...
//! anything else after #studio     → #A ? err syntax
//!
//! #V 2 layers=4 keys=42 store=flash max=40 combos=16 caps=check,combos,defaults,mouse names=default,...
//! #L <layer> <action> × keys      #LD the same, compiled
//! #C <i> <name> <positions> <layers> <timeout> <idle> <action>    #CD the same, compiled
//! #C end <count>                  #CD end <count>
//! #U unlocked <seconds left> | #U locked
//! #D 1 | #D 0
//! ```
//!
//! After the first `#studio` command of a connection, `#U` and `#D` also come
//! by themselves whenever they change. See docs/studio-protocol.md.

use core::fmt::Write;

use super::combo::{self, ComboSet, ComboSpec};
use super::encoding::{self, DecodeError, MAX_ENCODING};
use super::{Error, Studio};
use crate::action::Action;
use crate::debug::Line;
use crate::engine::MAX_COMBOS;
use crate::queue::Queue;

/// Version in the `#V` line; bumped when lines are added or change meaning.
/// 1: keys only. 2: combos, `check`, `defaults`, mouse actions (`caps=`).
pub const VERSION: u8 = 2;

/// What this firmware's Studio can do beyond version 1 (`caps=` in `#V`).
pub const CAPS: &str = "check,combos,defaults,mouse";

/// Length of the longest `#L` / `#LD` line for `keys` positions (longer
/// than any other Studio line).
#[must_use]
pub const fn layer_line_len(keys: usize) -> usize {
    "#LD 255".len() + keys * (1 + MAX_ENCODING) + 2
}

/// A `#studio` command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioCommand {
    Get,
    Status,
    Combos,
    Defaults,
    Set {
        layer: u8,
        pos: u8,
        action: Result<Action, DecodeError>,
    },
    Check {
        layer: u8,
        pos: u8,
        action: Result<Action, DecodeError>,
    },
    ComboAdd {
        combo: Result<ComboSpec, Error>,
    },
    ComboSet {
        index: u8,
        combo: Result<ComboSpec, Error>,
    },
    ComboRemove {
        index: u8,
    },
    /// `index` `None`: as a new combo.
    ComboCheck {
        index: Option<u8>,
        combo: Result<ComboSpec, Error>,
    },
    Save,
    Revert,
    Reset,
    Lock,
    /// `#studio` with something it doesn't understand.
    Invalid,
}

fn small_number(s: Option<&str>) -> Option<u8> {
    s.filter(|s| !s.is_empty() && s.len() <= 3 && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse::<u8>().ok())
}

/// Parse what follows `#studio ` (the words of the command).
#[must_use]
pub fn parse(words: &[u8]) -> StudioCommand {
    let Ok(text) = core::str::from_utf8(words) else {
        return StudioCommand::Invalid;
    };
    let mut parts = text.split_ascii_whitespace();
    let command = match parts.next() {
        Some("get") => StudioCommand::Get,
        Some("status") => StudioCommand::Status,
        Some("combos") => StudioCommand::Combos,
        Some("defaults") => StudioCommand::Defaults,
        Some("save") => StudioCommand::Save,
        Some("revert") => StudioCommand::Revert,
        Some("reset") => StudioCommand::Reset,
        Some("lock") => StudioCommand::Lock,
        Some(word @ ("set" | "check")) => {
            let (Some(layer), Some(pos), Some(action)) = (
                small_number(parts.next()),
                small_number(parts.next()),
                parts.next(),
            ) else {
                return StudioCommand::Invalid;
            };
            let action = encoding::decode(action);
            if word == "set" {
                StudioCommand::Set { layer, pos, action }
            } else {
                StudioCommand::Check { layer, pos, action }
            }
        }
        Some("combo") => {
            return match parts.next() {
                Some("add") => StudioCommand::ComboAdd {
                    combo: combo::parse_spec(parts),
                },
                Some("set") => match small_number(parts.next()) {
                    Some(index) => StudioCommand::ComboSet {
                        index,
                        combo: combo::parse_spec(parts),
                    },
                    None => StudioCommand::Invalid,
                },
                Some("check") => {
                    let index = match parts.next() {
                        Some("new") => None,
                        other => match small_number(other) {
                            Some(i) => Some(i),
                            None => return StudioCommand::Invalid,
                        },
                    };
                    StudioCommand::ComboCheck {
                        index,
                        combo: combo::parse_spec(parts),
                    }
                }
                Some("remove") => match (small_number(parts.next()), parts.next()) {
                    (Some(index), None) => StudioCommand::ComboRemove { index },
                    _ => StudioCommand::Invalid,
                },
                _ => StudioCommand::Invalid,
            };
        }
        _ => return StudioCommand::Invalid,
    };
    if parts.next().is_some() {
        return StudioCommand::Invalid;
    }
    command
}

/// A command that changes something, for its `#A` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Save,
    Revert,
    Reset,
    Lock,
}

impl Op {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Revert => "revert",
            Self::Reset => "reset",
            Self::Lock => "lock",
        }
    }
}

/// A combo command, for its `#A` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComboOp {
    Add,
    Set,
    Remove,
    Check,
}

impl ComboOp {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Set => "set",
            Self::Remove => "remove",
            Self::Check => "check",
        }
    }
}

/// The answer to a command (an `#A` line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ack {
    /// `set`, or `check` (changed nothing).
    Set {
        check: bool,
        layer: u8,
        pos: u8,
        result: Result<Action, Error>,
    },
    /// `index` `None`: a new combo (`new`). The combo comes back canonical
    /// (none for `remove`).
    Combo {
        op: ComboOp,
        index: Option<u8>,
        result: Result<Option<ComboSpec>, Error>,
    },
    Done {
        op: Op,
        result: Result<(), Error>,
    },
    Syntax,
}

/// Where saved bindings go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    /// Flash: they survive a restart.
    Flash,
    /// RAM only (the USB-only build, which has no storage): saved until restart.
    Ram,
}

/// Acknowledgements waiting to be sent, at most (a reader that sends
/// faster than it reads stops being read until they are out: [`Outbox::backlog`]).
pub const ACKS: usize = 32;

/// A list of combos being sent: the next index (its length: the end line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Listing {
    next: u8,
}

/// What one connection still has to be sent of the Studio protocol, and
/// what it was last told. Run commands with [`command`](Self::command) and
/// take lines with [`next_line`](Self::next_line).
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent things still to send, not a state machine"
)]
pub struct Outbox {
    storage: Storage,
    active: bool,
    acks: Queue<Ack, ACKS>,
    acks_len: usize,
    version: bool,
    layers: u32,
    combos: Option<Listing>,
    default_layers: u32,
    default_combos: Option<Listing>,
    unlock: bool,
    dirty: bool,
    /// Unlock state last sent (`None`: nothing yet): `Some(None)` locked,
    /// `Some(Some(t))` unlocked since activity at `t`.
    #[allow(
        clippy::option_option,
        reason = "not sent yet, or the `Option` last sent"
    )]
    sent_unlock: Option<Option<u32>>,
    sent_dirty: Option<bool>,
}

impl Outbox {
    #[must_use]
    pub const fn new(storage: Storage) -> Self {
        Self {
            storage,
            active: false,
            acks: Queue::new(),
            acks_len: 0,
            version: false,
            layers: 0,
            combos: None,
            default_layers: 0,
            default_combos: None,
            unlock: false,
            dirty: false,
            sent_unlock: None,
            sent_dirty: None,
        }
    }

    /// A new connection: nothing sent, nothing to send.
    pub fn reset(&mut self) {
        *self = Self::new(self.storage);
    }

    /// Whether this connection has used Studio (it then gets `#U` / `#D` changes).
    #[must_use]
    pub fn active(&self) -> bool {
        self.active
    }

    /// Acknowledgements not sent yet; stop reading commands while this is
    /// more than a packet's worth (`ACKS - 24`).
    #[must_use]
    pub fn backlog(&self) -> usize {
        self.acks_len
    }

    /// Whether a line is waiting (not counting changes [`next_line`](Self::next_line) notices).
    #[must_use]
    pub fn has_lines(&self) -> bool {
        self.acks_len > 0
            || self.version
            || self.layers != 0
            || self.combos.is_some()
            || self.default_layers != 0
            || self.default_combos.is_some()
            || self.unlock
            || self.dirty
    }

    fn ack(&mut self, ack: Ack) {
        if self.acks.push(ack) {
            self.acks_len += 1;
        }
    }

    /// Answer a command that saves, reverts, resets or locks.
    fn done(&mut self, op: Op, result: Result<(), Error>) {
        self.ack(Ack::Done { op, result });
    }

    const fn all<const L: usize>() -> u32 {
        if L >= 32 { u32::MAX } else { (1 << L) - 1 }
    }

    fn list_combos(&mut self) {
        self.combos = Some(Listing { next: 0 });
    }

    /// Run `command` on `studio` and queue its answer.
    pub fn command<const N: usize, const L: usize>(
        &mut self,
        studio: &mut Studio<'_, N, L>,
        command: StudioCommand,
        now: u32,
    ) {
        self.active = true;
        match command {
            StudioCommand::Get => {
                self.version = true;
                self.layers = Self::all::<L>();
                self.list_combos();
                self.unlock = true;
                self.dirty = true;
            }
            StudioCommand::Status => {
                self.unlock = true;
                self.dirty = true;
            }
            StudioCommand::Combos => self.list_combos(),
            StudioCommand::Defaults => {
                self.default_layers = Self::all::<L>();
                self.default_combos = Some(Listing { next: 0 });
            }
            StudioCommand::Set { layer, pos, action } => {
                let result = action
                    .map_err(|_| Error::Action)
                    .and_then(|a| studio.set(layer, pos, a, now).map(|()| a));
                self.ack(Ack::Set {
                    check: false,
                    layer,
                    pos,
                    result,
                });
                self.unlock = true;
                self.dirty = true;
            }
            StudioCommand::Check { layer, pos, action } => {
                let result = action
                    .map_err(|_| Error::Action)
                    .and_then(|a| studio.check(layer, pos, a).map(|()| a));
                self.ack(Ack::Set {
                    check: true,
                    layer,
                    pos,
                    result,
                });
            }
            StudioCommand::ComboAdd { combo } => {
                let result = combo.and_then(|c| studio.add_combo(&c, now).map(|i| (i, c)));
                let (index, result) = match result {
                    Ok((i, c)) => (Some(i), Ok(Some(c))),
                    Err(e) => (None, Err(e)),
                };
                self.combo_changed(ComboOp::Add, index, result);
            }
            StudioCommand::ComboSet { index, combo } => {
                let result = combo.and_then(|c| studio.set_combo(index, &c, now).map(|()| Some(c)));
                self.combo_changed(ComboOp::Set, Some(index), result);
            }
            StudioCommand::ComboRemove { index } => {
                let result = studio.remove_combo(index, now).map(|()| None);
                self.combo_changed(ComboOp::Remove, Some(index), result);
            }
            StudioCommand::ComboCheck { index, combo } => {
                let result = combo.and_then(|c| studio.check_combo(index, &c).map(|()| Some(c)));
                self.ack(Ack::Combo {
                    op: ComboOp::Check,
                    index,
                    result,
                });
            }
            StudioCommand::Save => {
                let result = studio.save(now);
                self.done(Op::Save, result);
                self.unlock = true;
                self.dirty = true;
            }
            StudioCommand::Revert => {
                studio.revert();
                self.done(Op::Revert, Ok(()));
                self.layers = Self::all::<L>();
                self.list_combos();
                self.dirty = true;
            }
            StudioCommand::Reset => {
                let result = studio.reset(now);
                self.done(Op::Reset, result);
                if result.is_ok() {
                    self.layers = Self::all::<L>();
                    self.list_combos();
                }
                self.unlock = true;
                self.dirty = true;
            }
            StudioCommand::Lock => {
                studio.lock();
                self.done(Op::Lock, Ok(()));
                self.unlock = true;
            }
            StudioCommand::Invalid => self.ack(Ack::Syntax),
        }
        // Nothing to write the records to: saved means saved in RAM.
        if self.storage == Storage::Ram {
            studio.take_pending();
        }
    }

    fn combo_changed(
        &mut self,
        op: ComboOp,
        index: Option<u8>,
        result: Result<Option<ComboSpec>, Error>,
    ) {
        let ok = result.is_ok();
        self.ack(Ack::Combo { op, index, result });
        if ok {
            self.list_combos();
        }
        self.unlock = true;
        self.dirty = true;
    }

    /// Put the next line to send into `out` (cleared first); false when
    /// there is none. `out` needs [`layer_line_len`] bytes for `#L`.
    pub fn next_line<const N: usize, const L: usize, const M: usize>(
        &mut self,
        studio: &Studio<'_, N, L>,
        now: u32,
        out: &mut Line<M>,
    ) -> bool {
        out.clear();
        if let Some(ack) = self.acks.pop() {
            self.acks_len -= 1;
            write_ack(out, &ack);
            return true;
        }
        if core::mem::take(&mut self.version) {
            write_version(out, studio, self.storage);
            return true;
        }
        if self.layers != 0 {
            let layer = self.layers.trailing_zeros() as u8;
            self.layers &= !(1 << layer);
            if let Some(actions) = studio.layers().get(layer as usize) {
                write_layer(out, "#L", layer, actions);
                return true;
            }
        }
        if let Some(listing) = &mut self.combos {
            if !write_listing(out, "#C", studio.combos(), listing) {
                self.combos = None;
            }
            return true;
        }
        if self.default_layers != 0 {
            let layer = self.default_layers.trailing_zeros() as u8;
            self.default_layers &= !(1 << layer);
            if let Some(actions) = studio.default_layers().get(layer as usize) {
                write_layer(out, "#LD", layer, actions);
                return true;
            }
        }
        if let Some(listing) = &mut self.default_combos {
            if !write_listing(out, "#CD", &studio.default_combos(), listing) {
                self.default_combos = None;
            }
            return true;
        }
        let unlock = studio.unlocked(now).then_some(studio.active_at);
        if core::mem::take(&mut self.unlock) || (self.active && self.sent_unlock != Some(unlock)) {
            self.sent_unlock = Some(unlock);
            write_unlock(out, studio.remaining_ms(now));
            return true;
        }
        let dirty = studio.dirty();
        if core::mem::take(&mut self.dirty) || (self.active && self.sent_dirty != Some(dirty)) {
            self.sent_dirty = Some(dirty);
            write_dirty(out, dirty);
            return true;
        }
        false
    }
}

/// The next line of a combo list: a combo, or the end line (then false:
/// the list is done).
fn write_listing<const M: usize>(
    out: &mut Line<M>,
    tag: &str,
    set: &ComboSet,
    listing: &mut Listing,
) -> bool {
    if let Some(spec) = set.get(listing.next as usize) {
        write_combo(out, tag, listing.next, &spec);
        listing.next += 1;
        true
    } else {
        let _ = write!(out, "{tag} end {}\r\n", set.len());
        false
    }
}

/// `#C <index> <combo>\r\n` (or `#CD`).
pub fn write_combo<const M: usize>(out: &mut Line<M>, tag: &str, index: u8, spec: &ComboSpec) {
    let _ = write!(out, "{tag} {index} ");
    let _ = combo::write_spec(spec, out);
    let _ = out.write_str("\r\n");
}

/// `#A ...\r\n`.
pub fn write_ack<const M: usize>(out: &mut Line<M>, ack: &Ack) {
    let _ = out.write_str("#A ");
    match *ack {
        Ack::Set {
            check,
            layer,
            pos,
            result,
        } => {
            let word = if check { "check" } else { "set" };
            let _ = write!(out, "{word} {layer} {pos} ");
            match result {
                Ok(action) => {
                    let _ = out.write_str("ok ");
                    let _ = encoding::encode(action, out);
                }
                Err(e) => {
                    let _ = write!(out, "err {}", e.as_str());
                }
            }
        }
        Ack::Combo { op, index, result } => {
            let _ = write!(out, "combo {} ", op.as_str());
            let _ = match index {
                Some(i) => write!(out, "{i} "),
                None => out.write_str("new "),
            };
            match result {
                Ok(spec) => {
                    let _ = out.write_str("ok");
                    if let Some(spec) = spec {
                        let _ = out.write_char(' ');
                        let _ = combo::write_spec(&spec, out);
                    }
                }
                Err(e) => {
                    let _ = write!(out, "err {}", e.as_str());
                }
            }
        }
        Ack::Done { op, result } => {
            let _ = match result {
                Ok(()) => write!(out, "{} ok", op.as_str()),
                Err(e) => write!(out, "{} err {}", op.as_str(), e.as_str()),
            };
        }
        Ack::Syntax => {
            let _ = out.write_str("? err syntax");
        }
    }
    let _ = out.write_str("\r\n");
}

/// `#V ...\r\n`.
pub fn write_version<const N: usize, const L: usize, const M: usize>(
    out: &mut Line<M>,
    studio: &Studio<'_, N, L>,
    storage: Storage,
) {
    let store = match storage {
        Storage::Flash => "flash",
        Storage::Ram => "ram",
    };
    let _ = write!(
        out,
        "#V {VERSION} layers={L} keys={N} store={store} max={MAX_ENCODING} combos={MAX_COMBOS} caps={CAPS} names="
    );
    for (i, name) in studio.keymap().layer_names.iter().take(L).enumerate() {
        if i > 0 {
            let _ = out.write_char(',');
        }
        let _ = out.write_str(name);
    }
    let _ = out.write_str("\r\n");
}

/// `#L <layer> <action> ...\r\n` (`tag` `#L` or `#LD`).
pub fn write_layer<const M: usize>(out: &mut Line<M>, tag: &str, layer: u8, actions: &[Action]) {
    let _ = write!(out, "{tag} {layer}");
    for &action in actions {
        let _ = out.write_char(' ');
        let _ = encoding::encode(action, out);
    }
    let _ = out.write_str("\r\n");
}

/// `#U unlocked <seconds>\r\n` or `#U locked\r\n`.
pub fn write_unlock<const M: usize>(out: &mut Line<M>, remaining_ms: u32) {
    let _ = if remaining_ms > 0 {
        write!(out, "#U unlocked {}\r\n", remaining_ms.div_ceil(1000))
    } else {
        out.write_str("#U locked\r\n")
    };
}

/// `#D 0|1\r\n`.
pub fn write_dirty<const M: usize>(out: &mut Line<M>, dirty: bool) {
    let _ = write!(out, "#D {}\r\n", u8::from(dirty));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{kp, mt};
    use crate::keymaps::corne;
    use crate::keys::{LALT, TAB, Z};

    type Corne = Studio<'static, { corne::KEYS }, 4>;
    const LINE: usize = layer_line_len(corne::KEYS);

    fn lines(outbox: &mut Outbox, studio: &Corne, now: u32) -> Vec<String> {
        let mut line = Line::<LINE>::new();
        let mut out = Vec::new();
        while outbox.next_line(studio, now, &mut line) {
            out.push(
                core::str::from_utf8(line.as_bytes())
                    .unwrap()
                    .trim_end()
                    .to_string(),
            );
        }
        out
    }

    fn unlocked() -> Corne {
        let mut s = Corne::new(&corne::KEYMAP);
        s.unlock(0);
        s
    }

    #[test]
    fn set_parses_layer_position_and_action() {
        // Arrange
        let words = b"set 2 35 mt:LALT/TAB";

        // Act
        let got = parse(words);

        // Assert
        assert_eq!(
            got,
            StudioCommand::Set {
                layer: 2,
                pos: 35,
                action: Ok(mt(LALT, TAB))
            }
        );
    }

    #[test]
    fn set_with_a_bad_action_still_parses() {
        // Arrange
        let words = b"set 0 1 kp:NOPE";

        // Act
        let got = parse(words);

        // Assert
        assert_eq!(
            got,
            StudioCommand::Set {
                layer: 0,
                pos: 1,
                action: Err(DecodeError::UnknownKey)
            }
        );
    }

    #[test]
    fn plain_commands_parse() {
        // Arrange
        let inputs: [&[u8]; 6] = [b"get", b"status", b"save", b"revert", b"reset", b"lock"];

        // Act
        let got: Vec<_> = inputs.iter().map(|w| parse(w)).collect();

        // Assert
        assert_eq!(
            got,
            [
                StudioCommand::Get,
                StudioCommand::Status,
                StudioCommand::Save,
                StudioCommand::Revert,
                StudioCommand::Reset,
                StudioCommand::Lock
            ]
        );
    }

    #[test]
    fn malformed_commands_are_invalid() {
        // Arrange
        let inputs: [&[u8]; 7] = [
            b"",
            b"fly",
            b"set 1 2",
            b"set x 2 kp:A",
            b"set 1 256 kp:A",
            b"get now",
            b"set 1 2 kp:A extra",
        ];

        // Act
        let got: Vec<_> = inputs.iter().map(|w| parse(w)).collect();

        // Assert
        assert!(got.iter().all(|c| *c == StudioCommand::Invalid), "{got:?}");
    }

    #[test]
    fn get_sends_version_layers_unlock_and_dirty() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, StudioCommand::Get, 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got.len(), 14);
        assert_eq!(
            got[0],
            "#V 2 layers=4 keys=42 store=flash max=40 combos=16 caps=check,combos,defaults,mouse names=default,symbols,nav,wm"
        );
        assert!(got[1].starts_with("#L 0 kp:ESC kp:Q kp:W "), "{}", got[1]);
        assert!(
            got[3]
                .ends_with(" sys:StudioUnlock trans mk:left/BSPC mk:right/SPACE trans trans trans"),
            "{}",
            got[3]
        );
        assert_eq!(got[5], "#C 0 cut 25,26 0 50 150 kp:LC(X)");
        assert_eq!(got[11], "#C end 6");
        assert_eq!(&got[12..], ["#U locked", "#D 0"]);
    }

    #[test]
    fn layer_line_has_one_action_per_key() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);
        let mut outbox = Outbox::new(Storage::Ram);
        outbox.command(&mut studio, StudioCommand::Get, 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        for line in &got[1..5] {
            assert_eq!(line.split(' ').count(), 2 + corne::KEYS, "{line}");
        }
    }

    #[test]
    fn layer_lines_decode_back_to_the_keymap() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);
        let mut outbox = Outbox::new(Storage::Ram);
        outbox.command(&mut studio, StudioCommand::Get, 0);

        // Act
        let decoded: Vec<Vec<Action>> = lines(&mut outbox, &studio, 0)[1..5]
            .iter()
            .map(|l| {
                l.split(' ')
                    .skip(2)
                    .map(|a| encoding::decode(a).unwrap())
                    .collect()
            })
            .collect();

        // Assert
        let expected: Vec<Vec<Action>> = corne::LAYERS.iter().map(|l| l.to_vec()).collect();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn set_while_locked_answers_locked() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, parse(b"set 0 1 kp:Z"), 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got, ["#A set 0 1 err locked", "#U locked", "#D 0"]);
    }

    #[test]
    fn accepted_set_answers_with_the_canonical_action() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, parse(b"set 0 1 kp:lc(z)"), 1000);

        // Act
        let got = lines(&mut outbox, &studio, 1000);

        // Assert
        assert_eq!(got, ["#A set 0 1 ok kp:LC(Z)", "#U unlocked 300", "#D 1"]);
    }

    #[test]
    fn set_that_locks_out_answers_lockout() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, parse(b"set 2 35 none"), 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got[0], "#A set 2 35 err lockout");
    }

    #[test]
    fn set_with_a_bad_action_answers_action() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, parse(b"set 0 1 kp:NOPE"), 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got[0], "#A set 0 1 err action");
    }

    #[test]
    fn save_answers_ok_and_clean() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Ram);
        studio.set(0, 1, kp(Z), 0).unwrap();
        outbox.command(&mut studio, StudioCommand::Save, 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got, ["#A save ok", "#U unlocked 300", "#D 0"]);
    }

    #[test]
    fn save_to_flash_is_dirty_until_stored() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        studio.set(0, 1, kp(Z), 0).unwrap();
        outbox.command(&mut studio, StudioCommand::Save, 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got.last().unwrap(), "#D 1");
    }

    #[test]
    fn revert_resends_the_layers() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        studio.set(0, 1, kp(Z), 0).unwrap();
        outbox.command(&mut studio, StudioCommand::Revert, 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got[0], "#A revert ok");
        assert!(got[1].starts_with("#L 0 kp:ESC kp:Q "));
        assert_eq!(got.last().unwrap(), "#D 0");
    }

    #[test]
    fn reset_while_locked_sends_no_layers() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, StudioCommand::Reset, 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got, ["#A reset err locked", "#U locked", "#D 0"]);
    }

    #[test]
    fn lock_answers_and_reports_locked() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, StudioCommand::Lock, 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got, ["#A lock ok", "#U locked", "#D 0"]);
    }

    /// The first command of a connection also reports the state.
    #[test]
    fn invalid_command_answers_syntax() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, StudioCommand::Invalid, 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got, ["#A ? err syntax", "#U locked", "#D 0"]);
    }

    #[test]
    fn unlock_key_is_reported_by_itself_once_studio_is_used() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, StudioCommand::Status, 0);
        lines(&mut outbox, &studio, 0);
        studio.unlock(500);

        // Act
        let got = lines(&mut outbox, &studio, 500);

        // Assert
        assert_eq!(got, ["#U unlocked 300"]);
    }

    #[test]
    fn timeout_is_reported_by_itself() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, StudioCommand::Status, 0);
        lines(&mut outbox, &studio, 0);

        // Act
        let got = lines(&mut outbox, &studio, super::super::UNLOCK_IDLE_MS);

        // Assert
        assert_eq!(got, ["#U locked"]);
    }

    #[test]
    fn nothing_comes_by_itself_before_studio_is_used() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);
        let mut outbox = Outbox::new(Storage::Flash);
        studio.unlock(0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(got, Vec::<String>::new());
    }

    #[test]
    fn acks_queue_up_in_order() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, parse(b"set 0 1 kp:Z"), 0);
        outbox.command(&mut studio, parse(b"set 0 2 sys:BtNext"), 0);

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert
        assert_eq!(
            &got[..2],
            ["#A set 0 1 ok kp:Z", "#A set 0 2 ok sys:BtNext"]
        );
    }

    #[test]
    fn backlog_counts_unsent_acks() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, parse(b"set 0 1 kp:Z"), 0);

        // Act
        let backlog = outbox.backlog();

        // Assert
        assert_eq!(backlog, 1);
    }

    #[test]
    fn longest_layer_line_fits() {
        // Arrange: every key of a layer at the longest encoding allowed
        let long = Action::ModTap {
            hold: crate::Key {
                code: 0x48,
                mods: 0x03,
            },
            tap: crate::Key {
                code: 0x48,
                mods: 0x01,
            },
        };
        let encoded = encoding::encoded(long);
        assert!(
            encoded.as_str().len() <= MAX_ENCODING,
            "{}",
            encoded.as_str()
        );
        let mut line = Line::<LINE>::new();

        // Act
        write_layer(&mut line, "#LD", 255, &[long; corne::KEYS]);

        // Assert
        assert!(line.as_bytes().ends_with(b"\r\n"));
    }

    fn spec(text: &str) -> ComboSpec {
        combo::parse_spec(text.split(' ')).unwrap()
    }

    fn run(studio: &mut Corne, line: &[u8], now: u32) -> Vec<String> {
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(studio, parse(line), now);
        lines(&mut outbox, studio, now)
    }

    #[test]
    fn check_parses_like_set() {
        // Arrange / Act
        let got = parse(b"check 1 2 kp:A");

        // Assert
        assert_eq!(
            got,
            StudioCommand::Check {
                layer: 1,
                pos: 2,
                action: Ok(kp(crate::keys::A))
            }
        );
    }

    #[test]
    fn listing_commands_parse() {
        // Arrange / Act
        let got = [parse(b"combos"), parse(b"defaults")];

        // Assert
        assert_eq!(got, [StudioCommand::Combos, StudioCommand::Defaults]);
    }

    #[test]
    fn combo_add_parses_its_six_words() {
        // Arrange / Act
        let got = parse(b"combo add x 1,2 * 50 0 kp:A");

        // Assert
        assert_eq!(
            got,
            StudioCommand::ComboAdd {
                combo: Ok(spec("x 1,2 * 50 0 kp:A"))
            }
        );
    }

    #[test]
    fn combo_set_parses_index_and_combo() {
        // Arrange / Act
        let got = parse(b"combo set 3 - 1,2 0 50 0 none");

        // Assert
        assert_eq!(
            got,
            StudioCommand::ComboSet {
                index: 3,
                combo: Ok(spec("- 1,2 0 50 0 none"))
            }
        );
    }

    #[test]
    fn combo_with_bad_fields_still_parses_with_the_error() {
        // Arrange / Act
        let got = parse(b"combo add x 1,1 * 50 0 kp:A");

        // Assert
        assert_eq!(
            got,
            StudioCommand::ComboAdd {
                combo: Err(Error::Positions)
            }
        );
    }

    #[test]
    fn combo_remove_parses_its_index() {
        // Arrange / Act
        let got = parse(b"combo remove 4");

        // Assert
        assert_eq!(got, StudioCommand::ComboRemove { index: 4 });
    }

    #[test]
    fn combo_check_takes_an_index_or_new() {
        // Arrange / Act
        let got = [
            parse(b"combo check new x 1,2 * 50 0 kp:A"),
            parse(b"combo check 2 x 1,2 * 50 0 kp:A"),
        ];

        // Assert
        assert_eq!(
            got,
            [
                StudioCommand::ComboCheck {
                    index: None,
                    combo: Ok(spec("x 1,2 * 50 0 kp:A"))
                },
                StudioCommand::ComboCheck {
                    index: Some(2),
                    combo: Ok(spec("x 1,2 * 50 0 kp:A"))
                }
            ]
        );
    }

    #[test]
    fn malformed_combo_commands_are_invalid() {
        // Arrange
        let inputs: [&[u8]; 8] = [
            b"combo",
            b"combo fly",
            b"combo set x - 1,2 0 50 0 none",
            b"combo set",
            b"combo remove",
            b"combo remove 1 2",
            b"combo check maybe x 1,2 * 50 0 kp:A",
            b"check 1 kp:A",
        ];

        // Act
        let got: Vec<_> = inputs.iter().map(|w| parse(w)).collect();

        // Assert
        assert!(got.iter().all(|c| *c == StudioCommand::Invalid), "{got:?}");
    }

    #[test]
    fn combo_with_too_few_words_is_a_syntax_error() {
        // Arrange / Act
        let got = parse(b"combo add x 1,2 * 50");

        // Assert
        assert_eq!(
            got,
            StudioCommand::ComboAdd {
                combo: Err(Error::Syntax)
            }
        );
    }

    #[test]
    fn check_answers_ok_without_changing_anything() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);

        // Act
        let got = run(&mut studio, b"check 0 1 kp:lc(z)", 0);

        // Assert
        assert_eq!(got[0], "#A check 0 1 ok kp:LC(Z)");
        assert!(!studio.dirty());
        assert_eq!(studio.binding(0, 1), corne::LAYERS[0].get(1).copied());
    }

    #[test]
    fn check_answers_lockout() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);

        // Act
        let got = run(&mut studio, b"check 2 35 trans", 0);

        // Assert
        assert_eq!(got[0], "#A check 2 35 err lockout");
    }

    #[test]
    fn check_answers_range_and_action() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);

        // Act
        let got = [
            run(&mut studio, b"check 9 0 kp:A", 0).remove(0),
            run(&mut studio, b"check 0 0 kp:NOPE", 0).remove(0),
            run(&mut studio, b"check 0 0 mo:4", 0).remove(0),
        ];

        // Assert
        assert_eq!(
            got,
            [
                "#A check 9 0 err range",
                "#A check 0 0 err action",
                "#A check 0 0 err action"
            ]
        );
    }

    #[test]
    fn combos_lists_every_combo_and_ends() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);

        // Act
        let got = run(&mut studio, b"combos", 0);

        // Assert
        assert_eq!(
            &got[..7],
            [
                "#C 0 cut 25,26 0 50 150 kp:LC(X)",
                "#C 1 copy 26,27 0 50 150 kp:LC(C)",
                "#C 2 paste 27,28 0 50 150 kp:LC(V)",
                "#C 3 caps_word 5,6 0 50 150 caps",
                "#C 4 escape 19,20 0 50 150 kp:ESC",
                "#C 5 reset_bluetooth 25,26 2 50 0 sys:BtClear",
                "#C end 6"
            ]
        );
    }

    #[test]
    fn defaults_sends_the_compiled_keymap_even_when_edited() {
        // Arrange
        let mut studio = unlocked();
        studio.set(0, 0, kp(Z), 0).unwrap();
        studio.remove_combo(0, 0).unwrap();

        // Act
        let got = run(&mut studio, b"defaults", 0);

        // Assert
        assert_eq!(got.len(), 4 + 7 + 2);
        assert!(got[0].starts_with("#LD 0 kp:ESC kp:Q "), "{}", got[0]);
        assert!(got[3].starts_with("#LD 3 "));
        assert_eq!(got[4], "#CD 0 cut 25,26 0 50 150 kp:LC(X)");
        assert_eq!(got[10], "#CD end 6");
    }

    #[test]
    fn combo_add_answers_its_index_and_lists_the_combos() {
        // Arrange
        let mut studio = unlocked();

        // Act
        let got = run(&mut studio, b"combo add x 2,1 0,1 40 0 kp:A", 0);

        // Assert
        assert_eq!(got[0], "#A combo add 6 ok x 1,2 0,1 40 0 kp:A");
        assert_eq!(got[7], "#C 6 x 1,2 0,1 40 0 kp:A");
        assert_eq!(got[8], "#C end 7");
        assert_eq!(&got[9..], ["#U unlocked 300", "#D 1"]);
    }

    #[test]
    fn combo_add_while_locked_answers_locked_and_lists_nothing() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);

        // Act
        let got = run(&mut studio, b"combo add x 1,2 0 40 0 kp:A", 0);

        // Assert
        assert_eq!(got, ["#A combo add new err locked", "#U locked", "#D 0"]);
    }

    #[test]
    fn combo_errors_have_their_words() {
        // Arrange
        let mut studio = unlocked();
        let inputs: [&[u8]; 7] = [
            b"combo add x 1,1 0 40 0 kp:A",
            b"combo add a-b 1,2 0 40 0 kp:A",
            b"combo add x 1,2 0 0 0 kp:A",
            b"combo add x 1,2 0 40 0 kp:NOPE",
            b"combo add x 1,2 0",
            b"combo set 9 x 1,2 0 40 0 kp:A",
            b"combo remove 9",
        ];

        // Act
        let got: Vec<_> = inputs
            .iter()
            .map(|i| run(&mut studio, i, 0).remove(0))
            .collect();

        // Assert
        assert_eq!(
            got,
            [
                "#A combo add new err positions",
                "#A combo add new err name",
                "#A combo add new err timing",
                "#A combo add new err action",
                "#A combo add new err syntax",
                "#A combo set 9 err range",
                "#A combo remove 9 err range",
            ]
        );
    }

    #[test]
    fn combo_add_when_full_answers_full() {
        // Arrange
        let mut studio = unlocked();
        for i in 0..10u8 {
            studio
                .add_combo(&spec(&format!("x {},{} 0 50 0 kp:A", i, i + 1)), 0)
                .unwrap();
        }

        // Act
        let got = run(&mut studio, b"combo add x 30,31 0 40 0 kp:A", 0);

        // Assert
        assert_eq!(got[0], "#A combo add new err full");
    }

    #[test]
    fn combo_set_answers_the_canonical_combo() {
        // Arrange
        let mut studio = unlocked();

        // Act
        let got = run(&mut studio, b"combo set 0 cut 26,25 0 60 150 kp:lc(x)", 0);

        // Assert
        assert_eq!(got[0], "#A combo set 0 ok cut 25,26 0 60 150 kp:LC(X)");
        assert_eq!(got[1], "#C 0 cut 25,26 0 60 150 kp:LC(X)");
    }

    #[test]
    fn combo_remove_answers_and_lists_the_rest() {
        // Arrange
        let mut studio = unlocked();

        // Act
        let got = run(&mut studio, b"combo remove 0", 0);

        // Assert
        assert_eq!(got[0], "#A combo remove 0 ok");
        assert_eq!(got[1], "#C 0 copy 26,27 0 50 150 kp:LC(C)");
        assert_eq!(got[6], "#C end 5");
    }

    #[test]
    fn combo_check_answers_without_changing_anything() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);

        // Act
        let got = run(&mut studio, b"combo check new x 1,2 0 40 0 kp:A", 0);

        // Assert
        assert_eq!(got[0], "#A combo check new ok x 1,2 0 40 0 kp:A");
        assert_eq!(studio.combos().len(), 6);
        assert!(!got.iter().any(|l| l.starts_with("#C ")));
    }

    #[test]
    fn combo_check_answers_lockout() {
        // Arrange: the unlock key only reachable through combo 6
        let mut studio = unlocked();
        studio
            .add_combo(&spec("u 0,11 0 50 0 sys:StudioUnlock"), 0)
            .unwrap();
        studio.set(2, 35, crate::action::TRANS, 0).unwrap();

        // Act
        let got = run(&mut studio, b"combo check 6 u 0,11 0 50 0 kp:A", 0);

        // Assert
        assert_eq!(got[0], "#A combo check 6 err lockout");
    }

    #[test]
    fn revert_resends_the_combos() {
        // Arrange
        let mut studio = unlocked();
        studio.remove_combo(0, 0).unwrap();

        // Act
        let got = run(&mut studio, b"revert", 0);

        // Assert
        assert!(got.contains(&"#C end 6".to_string()), "{got:?}");
    }

    #[test]
    fn reset_resends_layers_and_combos() {
        // Arrange
        let mut studio = unlocked();

        // Act
        let got = run(&mut studio, b"reset", 0);

        // Assert
        assert_eq!(got[0], "#A reset ok");
        assert!(got[1].starts_with("#L 0 "));
        assert_eq!(got[5], "#C 0 cut 25,26 0 50 150 kp:LC(X)");
        assert_eq!(got[11], "#C end 6");
    }

    #[test]
    fn save_while_locked_answers_locked() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);

        // Act
        let got = run(&mut studio, b"save", 0);

        // Assert
        assert_eq!(got[0], "#A save err locked");
    }

    #[test]
    fn status_answers_unlock_and_dirty() {
        // Arrange
        let mut studio = unlocked();

        // Act
        let got = run(&mut studio, b"status", 1000);

        // Assert
        assert_eq!(got, ["#U unlocked 299", "#D 0"]);
    }

    #[test]
    fn every_error_has_its_own_word() {
        // Arrange
        let all = [
            Error::Locked,
            Error::Range,
            Error::Action,
            Error::TooLong,
            Error::Lockout,
            Error::TooComplex,
            Error::Positions,
            Error::Name,
            Error::Timing,
            Error::Full,
            Error::Syntax,
        ];

        // Act
        let mut words: Vec<_> = all.iter().map(|e| e.as_str()).collect();
        words.sort_unstable();
        words.dedup();

        // Assert
        assert_eq!(words.len(), all.len());
        assert!(words.iter().all(|w| !w.contains(' ')));
    }

    #[test]
    fn longest_combo_line_fits() {
        // Arrange
        let long = crate::Key {
            code: 0x48,
            mods: 0x03,
        };
        let combo = ComboSpec {
            name: combo::Name::new("abcdefghijklmnop").unwrap(),
            def: crate::ComboDef {
                keys: 0b1111 << 38,
                action: Action::ModTap {
                    hold: long,
                    tap: crate::Key {
                        code: 0x48,
                        mods: 0x01,
                    },
                },
                layers: 0xFF,
                timeout_ms: 65535,
                require_prior_idle_ms: 65535,
            },
        };
        let mut line = Line::<LINE>::new();

        // Act
        write_combo(&mut line, "#CD", 15, &combo);

        // Assert
        assert!(line.as_bytes().ends_with(b"\r\n"), "not cut off");
    }

    #[test]
    fn longest_combo_command_fits_the_command_line() {
        // Arrange
        let command = b"#studio combo check new abcdefghijklmnop 38,39,40,41 0,1,2,3,4,5,6,7 65535 65535 mt:LC(LS(PAUSE_BREAK))/LC(PAUSE_BREAK)";

        // Act
        let fits = command.len() <= crate::console::LINE_MAX;

        // Assert
        assert!(fits, "{}", command.len());
    }

    #[test]
    fn set_with_too_long_an_action_answers_long() {
        // Arrange
        let mut studio = unlocked();

        // Act
        let got = run(
            &mut studio,
            b"set 0 0 kp:LC(LS(LA(LG(RC(RS(RA(RG(PAUSE_BREAK))))))))",
            0,
        );

        // Assert
        assert_eq!(got[0], "#A set 0 0 err long");
    }

    #[test]
    fn too_complex_answers_complex() {
        // Arrange
        let mut line = Line::<64>::new();

        // Act
        write_ack(
            &mut line,
            &Ack::Set {
                check: false,
                layer: 1,
                pos: 2,
                result: Err(Error::TooComplex),
            },
        );

        // Assert
        assert_eq!(line.as_bytes(), b"#A set 1 2 err complex\r\n");
    }

    #[test]
    fn get_while_unlocked_reports_the_time_left() {
        // Arrange
        let mut studio = unlocked();

        // Act
        let got = run(&mut studio, b"get", 30_000);

        // Assert
        assert_eq!(got[12], "#U unlocked 270");
    }

    #[test]
    fn outbox_reset_forgets_everything() {
        // Arrange
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, StudioCommand::Get, 0);

        // Act
        outbox.reset();

        // Assert
        assert!(!outbox.active());
        assert!(!outbox.has_lines());
        assert_eq!(lines(&mut outbox, &studio, 0), Vec::<String>::new());
    }

    #[test]
    fn has_lines_until_everything_is_sent() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, StudioCommand::Combos, 0);
        let before = outbox.has_lines();

        // Act
        lines(&mut outbox, &studio, 0);

        // Assert
        assert!(before);
        assert!(!outbox.has_lines());
    }

    #[test]
    fn dirty_change_is_reported_by_itself() {
        // Arrange: a reader that saw #D 0, then an edit from elsewhere
        let mut studio = unlocked();
        let mut outbox = Outbox::new(Storage::Flash);
        outbox.command(&mut studio, StudioCommand::Status, 0);
        lines(&mut outbox, &studio, 0);
        studio.set(0, 1, kp(Z), 0).unwrap();

        // Act
        let got = lines(&mut outbox, &studio, 0);

        // Assert: the unlock window didn't move (same moment), only dirty did
        assert_eq!(got, ["#D 1"]);
    }

    #[test]
    fn full_ack_queue_drops_the_newest() {
        // Arrange
        let mut studio = Corne::new(&corne::KEYMAP);
        let mut outbox = Outbox::new(Storage::Flash);
        for _ in 0..ACKS + 5 {
            outbox.command(&mut studio, StudioCommand::Invalid, 0);
        }

        // Act
        let backlog = outbox.backlog();

        // Assert
        assert_eq!(backlog, ACKS);
    }
}

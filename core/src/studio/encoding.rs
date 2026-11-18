//! Actions as short, readable text: what the Studio protocol sends and the
//! emulator's file keeps. One token, no spaces:
//!
//! ```text
//! trans                 &trans: the binding of the next active layer below
//! none                  &none
//! caps                  &caps_word
//! kp:A  kp:EXCL         &kp: a key; ZMK's key names (keys::name) and aliases
//! kp:LC(C)  kp:LG(LS(N1))   with modifiers, ZMK's LC() LS() LA() LG() RC() RS() RA() RG()
//! kp:0x87               a HID usage without a name
//! mo:2                  &mo: layer 2 while held
//! mt:LALT/TAB           &mt: hold / tap
//! sys:BtNext            a system action (SystemAction's names)
//! mm:left               move the mouse pointer (left down up right)
//! mb:left               a mouse button (left right middle)
//! ms:down               scroll (left down up right: left/right pan)
//! mp                    precise (slow) mouse movement while held
//! mk:left/BSPC          the left button once the pointer is active, else Backspace
//! mk:scroll_left/F2     scroll left once the pointer is active, else F2
//! ```
//!
//! Reading ignores case and accepts a few of ZMK's longer names (`ENTER`,
//! `BACKSPACE`, ...). Writing is canonical: lower-case kinds, upper-case
//! key names, modifiers outermost first in the order LG LC LS LA RG RC RS RA,
//! a shifted-symbol alias (`EXCL`, `LPAR`, ...) only for exactly Left Shift
//! plus its key, `MEH` for Ctrl+Shift+Alt. `decode(encode(a)) == a` for every
//! action.

use core::fmt::{self, Write};

use crate::action::{Action, Direction, Key, MouseAction, MouseButton, SystemAction};
#[allow(
    clippy::wildcard_imports,
    reason = "the encoding names every key and modifier"
)]
use crate::keys::{self, *};

/// Longest encoding Studio accepts for a binding (the line buffers are
/// sized for it); the compiled keymap stays well below it.
pub const MAX_ENCODING: usize = 40;

/// Why a text is not an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Not one of `trans none caps kp: mo: mt: sys:`.
    UnknownKind,
    /// A key name (or modifier function) nobody knows.
    UnknownKey,
    /// Brackets that don't match, a missing `/`, a bad number.
    Syntax,
    /// A system action nobody knows.
    UnknownSystem,
    /// A mouse direction or button nobody knows.
    UnknownMouse,
}

/// Modifier functions, in the order they are written (outermost first).
const MOD_FUNCTIONS: [(&str, u8); 8] = [
    ("LG", MOD_LGUI),
    ("LC", MOD_LCTL),
    ("LS", MOD_LSFT),
    ("LA", MOD_LALT),
    ("RG", MOD_RGUI),
    ("RC", MOD_RCTL),
    ("RS", MOD_RSFT),
    ("RA", MOD_RALT),
];

/// Names for a key with Left Shift (US layout, ZMK's names), and MEH.
pub const ALIASES: [(&str, Key); 22] = [
    ("EXCL", EXCL),
    ("AT", AT),
    ("HASH", HASH),
    ("DLLR", DLLR),
    ("PRCNT", PRCNT),
    ("CARET", CARET),
    ("AMPS", AMPS),
    ("ASTRK", ASTRK),
    ("LPAR", LPAR),
    ("RPAR", RPAR),
    ("UNDER", UNDER),
    ("PLUS", PLUS),
    ("LBRC", LBRC),
    ("RBRC", RBRC),
    ("PIPE", PIPE),
    ("TILDE", TILDE),
    ("COLON", ls(SEMI)),
    ("DQT", ls(SQT)),
    ("LT", ls(COMMA)),
    ("GT", ls(DOT)),
    ("QMARK", ls(FSLH)),
    ("MEH", MEH),
];

/// Other spellings that are read but never written (ZMK's long names).
const SPELLINGS: [(&str, u8); 12] = [
    ("ENTER", 0x28),
    ("RETURN", 0x28),
    ("ESCAPE", 0x29),
    ("BACKSPACE", 0x2A),
    ("SPC", 0x2C),
    ("DELETE", 0x4C),
    ("LCTL", 0xE0),
    ("LSHIFT", 0xE1),
    ("LCMD", 0xE3),
    ("RCTL", 0xE4),
    ("RSHIFT", 0xE5),
    ("RCMD", 0xE7),
];

/// Every key name that [`decode`] reads after `kp:` and writes itself: the
/// named HID usages in usage order, then [`ALIASES`]. For a key picker.
pub fn key_names() -> impl Iterator<Item = &'static str> {
    (0..=255u8)
        .map(keys::name)
        .filter(|&n| n != "?")
        .chain(ALIASES.iter().map(|&(n, _)| n))
}

/// Write `action` in its canonical form.
///
/// # Errors
///
/// Whatever `out` returns when it can't take more text.
pub fn encode<W: Write>(action: Action, out: &mut W) -> fmt::Result {
    match action {
        Action::Trans => out.write_str("trans"),
        Action::None => out.write_str("none"),
        Action::CapsWord => out.write_str("caps"),
        Action::Key(k) => {
            out.write_str("kp:")?;
            encode_key(k, out)
        }
        Action::MomentaryLayer(l) => write!(out, "mo:{l}"),
        Action::ModTap { hold, tap } => {
            out.write_str("mt:")?;
            encode_key(hold, out)?;
            out.write_char('/')?;
            encode_key(tap, out)
        }
        Action::System(s) => {
            out.write_str("sys:")?;
            out.write_str(s.name())
        }
        Action::Mouse(MouseAction::Move(d)) => write!(out, "mm:{}", d.name()),
        Action::Mouse(MouseAction::Button(b)) => write!(out, "mb:{}", b.name()),
        Action::Mouse(MouseAction::Scroll(d)) => write!(out, "ms:{}", d.name()),
        Action::Mouse(MouseAction::Precision) => out.write_str("mp"),
        Action::MouseOrKey { mouse, key } => {
            out.write_str("mk:")?;
            match mouse {
                MouseAction::Button(b) => out.write_str(b.name())?,
                MouseAction::Scroll(d) => write!(out, "scroll_{}", d.name())?,
                // Never made by `decode` (only buttons and scrolling).
                MouseAction::Move(d) => write!(out, "move_{}", d.name())?,
                MouseAction::Precision => out.write_str("precision")?,
            }
            out.write_char('/')?;
            encode_key(key, out)
        }
    }
}

fn direction(text: &str) -> Result<Direction, DecodeError> {
    Direction::ALL
        .into_iter()
        .find(|d| d.name().eq_ignore_ascii_case(text))
        .ok_or(DecodeError::UnknownMouse)
}

fn button(text: &str) -> Result<MouseButton, DecodeError> {
    MouseButton::ALL
        .into_iter()
        .find(|b| b.name().eq_ignore_ascii_case(text))
        .ok_or(DecodeError::UnknownMouse)
}

/// Write `k` the way `kp:` and `mt:` take it.
///
/// # Errors
///
/// Whatever `out` returns when it can't take more text.
pub fn encode_key<W: Write>(k: Key, out: &mut W) -> fmt::Result {
    if k == MEH {
        return out.write_str("MEH");
    }
    let alias = (k.mods == MOD_LSFT)
        .then(|| {
            ALIASES
                .iter()
                .find(|(_, a)| a.code == k.code && a.mods == MOD_LSFT)
        })
        .flatten();
    let mods = if alias.is_some() { 0 } else { k.mods };
    let mut open = 0;
    for (name, bit) in MOD_FUNCTIONS {
        if mods & bit != 0 {
            out.write_str(name)?;
            out.write_char('(')?;
            open += 1;
        }
    }
    match (alias, keys::name(k.code)) {
        (Some((name, _)), _) => out.write_str(name)?,
        (None, "?") => write!(out, "0x{:02X}", k.code)?,
        (None, name) => out.write_str(name)?,
    }
    for _ in 0..open {
        out.write_char(')')?;
    }
    Ok(())
}

/// Read an action (see the module documentation).
///
/// # Errors
///
/// Why `text` is not an action.
pub fn decode(text: &str) -> Result<Action, DecodeError> {
    let (kind, arg) = match text.split_once(':') {
        Some((kind, arg)) => (kind, Some(arg)),
        None => (text, None),
    };
    let is = |name: &str| kind.eq_ignore_ascii_case(name);
    match arg {
        None if is("trans") => Ok(Action::Trans),
        None if is("none") => Ok(Action::None),
        None if is("caps") => Ok(Action::CapsWord),
        None if is("mp") => Ok(Action::Mouse(MouseAction::Precision)),
        Some(arg) if is("mm") => direction(arg).map(|d| Action::Mouse(MouseAction::Move(d))),
        Some(arg) if is("ms") => direction(arg).map(|d| Action::Mouse(MouseAction::Scroll(d))),
        Some(arg) if is("mb") => button(arg).map(|b| Action::Mouse(MouseAction::Button(b))),
        Some(arg) if is("mk") => {
            let (m, key) = arg.split_once('/').ok_or(DecodeError::Syntax)?;
            let mouse = match m.get(..7) {
                Some(p) if p.eq_ignore_ascii_case("scroll_") => {
                    MouseAction::Scroll(direction(&m[7..])?)
                }
                _ => MouseAction::Button(button(m)?),
            };
            Ok(Action::MouseOrKey {
                mouse,
                key: decode_key(key)?,
            })
        }
        Some(arg) if is("kp") => decode_key(arg).map(Action::Key),
        Some(arg) if is("mo") => decode_number(arg)
            .map(Action::MomentaryLayer)
            .ok_or(DecodeError::Syntax),
        Some(arg) if is("mt") => {
            let (hold, tap) = arg.split_once('/').ok_or(DecodeError::Syntax)?;
            Ok(Action::ModTap {
                hold: decode_key(hold)?,
                tap: decode_key(tap)?,
            })
        }
        Some(arg) if is("sys") => SystemAction::ALL
            .iter()
            .find(|s| s.name().eq_ignore_ascii_case(arg))
            .map(|&s| Action::System(s))
            .ok_or(DecodeError::UnknownSystem),
        _ => Err(DecodeError::UnknownKind),
    }
}

/// Read a key: a name, `0x..`, or a modifier function around a key.
///
/// # Errors
///
/// Why `text` is not a key.
pub fn decode_key(text: &str) -> Result<Key, DecodeError> {
    if let Some((function, rest)) = text.split_once('(') {
        let inner = rest.strip_suffix(')').ok_or(DecodeError::Syntax)?;
        let (_, bit) = MOD_FUNCTIONS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(function))
            .ok_or(DecodeError::UnknownKey)?;
        return Ok(decode_key(inner)?.with_mods(*bit));
    }
    if text.contains(')') || text.is_empty() {
        return Err(DecodeError::Syntax);
    }
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        if hex.is_empty() || hex.len() > 2 {
            return Err(DecodeError::Syntax);
        }
        return u8::from_str_radix(hex, 16)
            .map(Key::new)
            .map_err(|_| DecodeError::Syntax);
    }
    if let Some(code) = (0..=255u8).find(|&c| {
        let name = keys::name(c);
        name != "?" && name.eq_ignore_ascii_case(text)
    }) {
        return Ok(Key::new(code));
    }
    ALIASES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
        .map(|&(_, k)| k)
        .or_else(|| {
            SPELLINGS
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(text))
                .map(|&(_, code)| Key::new(code))
        })
        .ok_or(DecodeError::UnknownKey)
}

/// A decimal `u8` without sign or leading `+`.
fn decode_number(text: &str) -> Option<u8> {
    if text.is_empty() || text.len() > 3 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// An encoding in a buffer of its own (big enough for any action).
#[derive(Clone, Copy)]
pub struct Encoded {
    buf: [u8; 96],
    len: u8,
}

impl Encoded {
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Only ASCII is ever written.
        core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }
}

impl Write for Encoded {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let at = self.len as usize;
        let end = at + s.len();
        if end > self.buf.len() {
            return Err(fmt::Error);
        }
        self.buf[at..end].copy_from_slice(s.as_bytes());
        self.len = end as u8;
        Ok(())
    }
}

/// `action`'s canonical encoding.
#[must_use]
pub fn encoded(action: Action) -> Encoded {
    let mut e = Encoded {
        buf: [0; 96],
        len: 0,
    };
    // Can't fail: the longest action (two keys with all eight modifiers) fits.
    let _ = encode(action, &mut e);
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{CAPS_WORD, NONE, TRANS, kp, mo, mt, sys};
    use crate::keymaps::corne;

    fn text(action: Action) -> String {
        encoded(action).as_str().to_string()
    }

    #[test]
    fn plain_key_is_kp_and_its_name() {
        // Arrange
        let action = kp(A);

        // Act
        let got = text(action);

        // Assert
        assert_eq!(got, "kp:A");
    }

    #[test]
    fn key_with_ctrl_is_wrapped_in_lc() {
        // Arrange
        let action = kp(lc(C));

        // Act
        let got = text(action);

        // Assert
        assert_eq!(got, "kp:LC(C)");
    }

    #[test]
    fn modifiers_are_written_in_canonical_order() {
        // Arrange: built in another order
        let action = kp(ls(lc(lg(LEFT))));

        // Act
        let got = text(action);

        // Assert
        assert_eq!(got, "kp:LG(LC(LS(LEFT)))");
    }

    #[test]
    fn shifted_symbol_uses_its_alias() {
        // Arrange
        let action = kp(EXCL);

        // Act
        let got = text(action);

        // Assert
        assert_eq!(got, "kp:EXCL");
    }

    #[test]
    fn shift_plus_another_modifier_keeps_the_base_name() {
        // Arrange
        let action = kp(lg(ls(N1)));

        // Act
        let got = text(action);

        // Assert
        assert_eq!(got, "kp:LG(LS(N1))");
    }

    #[test]
    fn meh_has_its_own_name() {
        // Arrange
        let action = kp(MEH);

        // Act
        let got = text(action);

        // Assert
        assert_eq!(got, "kp:MEH");
    }

    #[test]
    fn mod_tap_is_hold_slash_tap() {
        // Arrange
        let action = mt(LALT, TAB);

        // Act
        let got = text(action);

        // Assert
        assert_eq!(got, "mt:LALT/TAB");
    }

    #[test]
    fn simple_actions_have_simple_names() {
        // Arrange
        let actions = [TRANS, NONE, CAPS_WORD, mo(2), sys(SystemAction::BtNext)];

        // Act
        let got: Vec<_> = actions.into_iter().map(text).collect();

        // Assert
        assert_eq!(got, ["trans", "none", "caps", "mo:2", "sys:BtNext"]);
    }

    #[test]
    fn usage_without_a_name_is_hex() {
        // Arrange
        let action = kp(Key::new(0x87));

        // Act
        let got = text(action);

        // Assert
        assert_eq!(got, "kp:0x87");
    }

    #[test]
    fn every_action_of_the_corne_keymap_round_trips() {
        // Arrange
        let actions: Vec<Action> = corne::LAYERS.iter().flatten().copied().collect();

        // Act
        let decoded: Vec<Action> = actions
            .iter()
            .map(|&a| decode(encoded(a).as_str()).unwrap())
            .collect();

        // Assert
        assert_eq!(decoded, actions);
    }

    #[test]
    fn every_usage_with_every_modifier_set_round_trips() {
        // Arrange
        let keys: Vec<Key> = (0..=255u8)
            .flat_map(|code| (0..=255u8).map(move |mods| Key { code, mods }))
            .collect();

        // Act
        let bad: Vec<Key> = keys
            .iter()
            .copied()
            .filter(|&k| decode(encoded(kp(k)).as_str()) != Ok(kp(k)))
            .collect();

        // Assert
        assert_eq!(bad, []);
    }

    #[test]
    fn every_system_action_round_trips() {
        // Arrange
        let actions: Vec<Action> = SystemAction::ALL.iter().map(|&s| sys(s)).collect();

        // Act
        let decoded: Vec<_> = actions.iter().map(|&a| decode(&text(a))).collect();

        // Assert
        assert_eq!(decoded, actions.into_iter().map(Ok).collect::<Vec<_>>());
    }

    #[test]
    fn every_layer_number_round_trips() {
        // Arrange
        let actions: Vec<Action> = (0..=255).map(mo).collect();

        // Act
        let decoded: Vec<_> = actions.iter().map(|&a| decode(&text(a))).collect();

        // Assert
        assert_eq!(decoded, actions.into_iter().map(Ok).collect::<Vec<_>>());
    }

    #[test]
    fn longest_action_fits_the_buffer() {
        // Arrange: every modifier on both keys of a mod-tap with long names
        let long = Key {
            code: 0x48,
            mods: 0xFF,
        };
        let action = mt(long, long);

        // Act
        let round = decode(encoded(action).as_str());

        // Assert
        assert_eq!(round, Ok(action));
    }

    #[test]
    fn corne_keymap_encodings_are_short_enough_for_studio() {
        // Arrange
        let actions = corne::LAYERS.iter().flatten();

        // Act
        let longest = actions.map(|&a| text(a).len()).max().unwrap();

        // Assert
        assert!(longest <= MAX_ENCODING, "{longest}");
    }

    #[test]
    fn mouse_actions_have_short_names() {
        // Arrange
        use crate::action::{MOUSE_PRECISION, click, mkey, mkp, mmv, msc, scroll};
        let actions = [
            mmv(Direction::Left),
            mkp(MouseButton::Middle),
            msc(Direction::Down),
            MOUSE_PRECISION,
            mkey(click(MouseButton::Left), BSPC),
            mkey(scroll(Direction::Left), F2),
        ];

        // Act
        let got: Vec<_> = actions.into_iter().map(text).collect();

        // Assert
        assert_eq!(
            got,
            [
                "mm:left",
                "mb:middle",
                "ms:down",
                "mp",
                "mk:left/BSPC",
                "mk:scroll_left/F2"
            ]
        );
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
                key: F5,
            });
        }
        for b in MouseButton::ALL {
            actions.push(Action::Mouse(MouseAction::Button(b)));
            actions.push(Action::MouseOrKey {
                mouse: MouseAction::Button(b),
                key: lc(SPACE),
            });
        }
        actions.push(Action::Mouse(MouseAction::Precision));

        // Act
        let decoded: Vec<_> = actions.iter().map(|&a| decode(&text(a))).collect();

        // Assert
        assert_eq!(decoded, actions.into_iter().map(Ok).collect::<Vec<_>>());
    }

    #[test]
    fn unknown_mouse_words_are_rejected() {
        // Arrange
        let inputs = [
            "mm:sideways",
            "mb:fourth",
            "ms:",
            "mk:left",
            "mk:scroll_in/A",
            "mk:left/NOPE",
            "mk:move_left/A",
        ];

        // Act
        let got: Vec<_> = inputs.iter().map(|i| decode(i)).collect();

        // Assert
        assert_eq!(
            got,
            [
                Err(DecodeError::UnknownMouse),
                Err(DecodeError::UnknownMouse),
                Err(DecodeError::UnknownMouse),
                Err(DecodeError::Syntax),
                Err(DecodeError::UnknownMouse),
                Err(DecodeError::UnknownKey),
                Err(DecodeError::UnknownMouse)
            ]
        );
    }

    #[test]
    fn reading_ignores_case() {
        // Arrange
        let input = "MT:lalt/tab";

        // Act
        let got = decode(input);

        // Assert
        assert_eq!(got, Ok(mt(LALT, TAB)));
    }

    #[test]
    fn long_zmk_names_are_read() {
        // Arrange
        let input = "kp:LC(BACKSPACE)";

        // Act
        let got = decode(input);

        // Assert
        assert_eq!(got, Ok(kp(lc(BSPC))));
    }

    #[test]
    fn aliases_take_more_modifiers() {
        // Arrange
        let input = "kp:LG(EXCL)";

        // Act
        let got = decode(input);

        // Assert
        assert_eq!(got, Ok(kp(lg(ls(N1)))));
    }

    #[test]
    fn unknown_key_is_rejected() {
        // Arrange
        let input = "kp:NOPE";

        // Act
        let got = decode(input);

        // Assert
        assert_eq!(got, Err(DecodeError::UnknownKey));
    }

    #[test]
    fn unknown_modifier_function_is_rejected() {
        // Arrange
        let input = "kp:XX(A)";

        // Act
        let got = decode(input);

        // Assert
        assert_eq!(got, Err(DecodeError::UnknownKey));
    }

    #[test]
    fn unbalanced_brackets_are_rejected() {
        // Arrange
        let inputs = ["kp:LC(A", "kp:A)", "kp:LC(LS(A)", "kp:LC()"];

        // Act
        let got: Vec<_> = inputs.iter().map(|i| decode(i)).collect();

        // Assert
        assert!(got.iter().all(Result::is_err), "{got:?}");
    }

    #[test]
    fn unknown_kind_is_rejected() {
        // Arrange
        let inputs = ["", "kp", "trans:1", "lt:1/A", "&kp A"];

        // Act
        let got: Vec<_> = inputs.iter().map(|i| decode(i)).collect();

        // Assert
        assert!(
            got.iter().all(|r| *r == Err(DecodeError::UnknownKind)),
            "{got:?}"
        );
    }

    #[test]
    fn bad_layer_numbers_are_rejected() {
        // Arrange
        let inputs = ["mo:", "mo:256", "mo:-1", "mo:+1", "mo:1a"];

        // Act
        let got: Vec<_> = inputs.iter().map(|i| decode(i)).collect();

        // Assert
        assert!(
            got.iter().all(|r| *r == Err(DecodeError::Syntax)),
            "{got:?}"
        );
    }

    #[test]
    fn mod_tap_without_a_slash_is_rejected() {
        // Arrange
        let input = "mt:LALT";

        // Act
        let got = decode(input);

        // Assert
        assert_eq!(got, Err(DecodeError::Syntax));
    }

    #[test]
    fn unknown_system_action_is_rejected() {
        // Arrange
        let input = "sys:SelfDestruct";

        // Act
        let got = decode(input);

        // Assert
        assert_eq!(got, Err(DecodeError::UnknownSystem));
    }

    #[test]
    fn hex_needs_one_or_two_digits() {
        // Arrange
        let inputs = ["kp:0x", "kp:0x123", "kp:0xZZ"];

        // Act
        let got: Vec<_> = inputs.iter().map(|i| decode(i)).collect();

        // Assert
        assert!(got.iter().all(Result::is_err), "{got:?}");
    }

    #[test]
    fn key_names_are_all_readable() {
        // Arrange
        let names: Vec<_> = key_names().collect();

        // Act
        let unreadable: Vec<_> = names.iter().filter(|n| decode_key(n).is_err()).collect();

        // Assert
        assert!(names.len() > 100);
        assert_eq!(unreadable, Vec::<&&str>::new());
    }
}

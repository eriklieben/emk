//! What a key does when it is pressed (ZMK calls these "behaviors").

/// A HID keyboard usage plus modifiers to send with it, e.g. `LS(N1)` for `!`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// HID usage ID (keyboard page 0x07).
    pub code: u8,
    /// Modifier bits sent along with the key (`keys::MOD_*`).
    pub mods: u8,
}

impl Key {
    #[must_use]
    pub const fn new(code: u8) -> Self {
        Self { code, mods: 0 }
    }

    #[must_use]
    pub const fn with_mods(self, mods: u8) -> Self {
        Self {
            code: self.code,
            mods: self.mods | mods,
        }
    }

    /// Left/right Ctrl, Shift, Alt or GUI (usages 0xE0..=0xE7).
    #[must_use]
    pub const fn is_modifier(self) -> bool {
        self.code >= 0xE0 && self.code <= 0xE7
    }

    /// For a modifier key: every modifier bit it holds, including its own.
    /// `MEH` (`LS(LC(LALT))`) gives Ctrl|Shift|Alt.
    #[must_use]
    pub const fn modifier_bits(self) -> u8 {
        if self.is_modifier() {
            self.mods | (1 << (self.code - 0xE0))
        } else {
            self.mods
        }
    }
}

/// Actions handled outside the HID report (Bluetooth, output selection, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAction {
    BtPrev,
    BtNext,
    BtClear,
    OutToggle,
    StudioUnlock,
    /// Show the link between the halves at the bottom of the screens again
    /// for a moment (once linked, it hides itself). Not in ZMK.
    ShowLink,
    /// Step the radio's transmit power: +8 → +4 → 0 → +8 dBm
    /// (`kb_core::link::next_radio_dbm`). Not in ZMK.
    RadioPower,
}

impl SystemAction {
    /// Every system action, in the order of their numbers (`kb_emu_poll`,
    /// the stored keymap format): the index is the number.
    pub const ALL: [Self; 7] = [
        Self::BtPrev,
        Self::BtNext,
        Self::BtClear,
        Self::OutToggle,
        Self::StudioUnlock,
        Self::ShowLink,
        Self::RadioPower,
    ];

    /// Its number: the index in [`ALL`](Self::ALL).
    #[must_use]
    pub const fn number(self) -> u8 {
        self as u8
    }

    /// The action with number `n`.
    #[must_use]
    pub const fn from_number(n: u8) -> Option<Self> {
        if (n as usize) < Self::ALL.len() {
            Some(Self::ALL[n as usize])
        } else {
            None
        }
    }

    /// Its name, as in the source (`BtNext`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BtPrev => "BtPrev",
            Self::BtNext => "BtNext",
            Self::BtClear => "BtClear",
            Self::OutToggle => "OutToggle",
            Self::StudioUnlock => "StudioUnlock",
            Self::ShowLink => "ShowLink",
            Self::RadioPower => "RadioPower",
        }
    }
}

/// A direction for mouse movement and scrolling (the order of HJKL / SDFG).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Down,
    Up,
    Right,
}

impl Direction {
    pub const ALL: [Self; 4] = [Self::Left, Self::Down, Self::Up, Self::Right];

    /// Its name in the Studio encoding (`left`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Down => "down",
            Self::Up => "up",
            Self::Right => "right",
        }
    }
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub const ALL: [Self; 3] = [Self::Left, Self::Right, Self::Middle];

    /// Its bit in a HID mouse report.
    #[must_use]
    pub const fn bit(self) -> u8 {
        match self {
            Self::Left => 0x01,
            Self::Right => 0x02,
            Self::Middle => 0x04,
        }
    }

    /// Its name in the Studio encoding (`left`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
        }
    }
}

/// What a mouse key does (`kb_core::mouse` turns these into motion).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    /// Move the pointer while held (accelerating).
    Move(Direction),
    /// A button, down while held.
    Button(MouseButton),
    /// Scroll while held (vertical wheel or horizontal pan).
    Scroll(Direction),
    /// Slow, precise movement while held.
    Precision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `&trans`: use the binding of the next active layer below.
    Trans,
    /// `&none`: do nothing.
    None,
    /// `&kp`: press a key (with optional modifiers).
    Key(Key),
    /// `&mo`: layer active while held.
    MomentaryLayer(u8),
    /// `&mt`: modifier when held, key when tapped ("hold-preferred" flavor).
    ModTap { hold: Key, tap: Key },
    /// `&caps_word`: shift letters until a word-breaking key is pressed.
    CapsWord,
    /// `&bt`, `&out`, `&studio_unlock`.
    System(SystemAction),
    /// A mouse key (ZMK `&mmv`, `&mkp`, `&msc`).
    Mouse(MouseAction),
    /// `mouse` (a button or a scroll direction) once the pointer is active
    /// (a mouse-move key was pressed since the layer it was pressed on came
    /// up), `key` otherwise. Decided at press; the release always matches.
    /// Not in ZMK.
    MouseOrKey { mouse: MouseAction, key: Key },
}

pub const TRANS: Action = Action::Trans;
pub const NONE: Action = Action::None;
pub const CAPS_WORD: Action = Action::CapsWord;

#[must_use]
pub const fn kp(k: Key) -> Action {
    Action::Key(k)
}

#[must_use]
pub const fn mo(layer: u8) -> Action {
    Action::MomentaryLayer(layer)
}

#[must_use]
pub const fn mt(hold: Key, tap: Key) -> Action {
    Action::ModTap { hold, tap }
}

#[must_use]
pub const fn sys(s: SystemAction) -> Action {
    Action::System(s)
}

/// Move the mouse pointer.
#[must_use]
pub const fn mmv(d: Direction) -> Action {
    Action::Mouse(MouseAction::Move(d))
}

/// A mouse button.
#[must_use]
pub const fn mkp(b: MouseButton) -> Action {
    Action::Mouse(MouseAction::Button(b))
}

/// Scroll.
#[must_use]
pub const fn msc(d: Direction) -> Action {
    Action::Mouse(MouseAction::Scroll(d))
}

/// Precise (slow) mouse movement while held.
pub const MOUSE_PRECISION: Action = Action::Mouse(MouseAction::Precision);

/// `mouse` (a button or scroll) once the pointer is active, `key` otherwise.
#[must_use]
pub const fn mkey(mouse: MouseAction, key: Key) -> Action {
    Action::MouseOrKey { mouse, key }
}

/// A button, for [`mkey`].
#[must_use]
pub const fn click(b: MouseButton) -> MouseAction {
    MouseAction::Button(b)
}

/// Scrolling, for [`mkey`].
#[must_use]
pub const fn scroll(d: Direction) -> MouseAction {
    MouseAction::Scroll(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_action_numbers_are_their_index() {
        // Arrange
        let all = SystemAction::ALL;

        // Act
        let numbers: Vec<u8> = all.iter().map(|s| s.number()).collect();

        // Assert
        assert_eq!(numbers, (0..all.len() as u8).collect::<Vec<_>>());
    }

    #[test]
    fn system_action_comes_back_from_its_number() {
        // Arrange
        let all = SystemAction::ALL;

        // Act
        let back: Vec<_> = all
            .iter()
            .map(|s| SystemAction::from_number(s.number()))
            .collect();

        // Assert
        assert_eq!(back, all.map(Some));
        assert_eq!(SystemAction::from_number(7), None);
    }
}

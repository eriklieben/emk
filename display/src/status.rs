//! Keyboard state shown on the displays: connection, battery, layer, WPM.

use kb_core::SystemAction;
use kb_core::host::Text;
use kb_core::link::{PASSKEY_DIGITS, PairPrompt, PairRequest};

/// Bluetooth profiles (paired computers), like ZMK with `BT_MAX_PAIRED=11` on a split.
pub const BT_PROFILES: u8 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Ble,
}

/// The pairing prompt on the left half's screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pairing {
    #[default]
    Off,
    /// Type the code the computer shows (passkey entry): the digits so far.
    TypeCode {
        typed: [u8; PASSKEY_DIGITS],
        len: u8,
    },
    /// Enter if the computer shows this code, Escape if not.
    Compare(u32),
}

impl Pairing {
    /// The prompt for `request`, with what the key loop's `prompt` took in.
    #[must_use]
    pub fn of(request: Option<PairRequest>, prompt: &PairPrompt) -> Self {
        match request {
            None => Self::Off,
            Some(PairRequest::Compare(code)) => Self::Compare(code),
            Some(PairRequest::TypeCode) => {
                let mut typed = [0; PASSKEY_DIGITS];
                let digits = prompt.typed();
                typed[..digits.len()].copy_from_slice(digits);
                Self::TypeCode {
                    typed,
                    len: digits.len() as u8,
                }
            }
        }
    }
}

/// Everything the status screens show, as the key loop last saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent things on the screen, not a state machine"
)]
pub struct Status {
    pub layer: u8,
    pub caps_word: bool,
    pub transport: Transport,
    /// Selected Bluetooth profile, 0-based.
    pub profile: u8,
    /// Battery percentage of the left (central) and right (peripheral) half.
    pub battery: [u8; 2],
    pub wpm: u16,
    /// Modifiers held (HID bits: left Ctrl Shift Alt Gui, then right).
    pub mods: u8,
    /// Whether each half runs on USB power (charging its battery).
    pub charging: [bool; 2],
    /// Key presses so far (wrapping): each one drops digital rain.
    pub key_presses: u16,
    /// Whether the selected Bluetooth profile has a connected host.
    pub profile_connected: bool,
    /// Whether the selected Bluetooth profile is open: no computer paired on
    /// it yet, so it is ready to pair (shown as `+`; taken but not connected `✗`).
    pub profile_open: bool,
    /// Whether the right half is connected to the left.
    pub peripheral_connected: bool,
    /// Notification from the computer, shown on the right half.
    pub notification: Option<Text>,
    /// Short state of the link between the halves (`scan`, `pair`, `linked`, ...),
    /// shown at the bottom of the screen while bringing up Bluetooth. Empty: hidden.
    pub link_detail: &'static str,
    /// Times the link was asked to be shown (`ShowLink`, wrapping).
    pub link_requests: u8,
    /// A computer pairing with a code: the prompt on the left half's screen.
    pub pairing: Pairing,
    /// The radio's transmit power in dBm (`RadioPower`).
    pub radio_dbm: i8,
    /// Times the radio power changed (wrapping): the level shows for a moment.
    pub radio_changes: u8,
    /// Whether Studio (live keymap editing) is unlocked; a change shows on
    /// the left screen for a moment.
    pub studio_unlocked: bool,
    /// Whether the mouse pointer is active (moved since nav came up): the
    /// nav thumbs click. A `MOUSE` bar on the left screen, above the
    /// modifier bars.
    pub pointer_active: bool,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            layer: 0,
            caps_word: false,
            transport: Transport::Ble,
            profile: 0,
            battery: [100, 100],
            wpm: 0,
            mods: 0,
            charging: [false, false],
            key_presses: 0,
            profile_connected: true,
            profile_open: false,
            peripheral_connected: true,
            notification: None,
            link_detail: "",
            link_requests: 0,
            pairing: Pairing::Off,
            radio_dbm: kb_core::link::RADIO_DEFAULT_DBM,
            radio_changes: 0,
            studio_unlocked: false,
            pointer_active: false,
        }
    }
}

impl Status {
    /// Update for a `&bt` / `&out` action.
    pub fn apply(&mut self, action: SystemAction) {
        match action {
            SystemAction::BtNext => self.profile = (self.profile + 1) % BT_PROFILES,
            SystemAction::BtPrev => self.profile = (self.profile + BT_PROFILES - 1) % BT_PROFILES,
            SystemAction::OutToggle => {
                self.transport = match self.transport {
                    Transport::Usb => Transport::Ble,
                    Transport::Ble => Transport::Usb,
                }
            }
            SystemAction::ShowLink => self.link_requests = self.link_requests.wrapping_add(1),
            SystemAction::RadioPower => {
                self.radio_dbm = kb_core::link::next_radio_dbm(self.radio_dbm);
                self.radio_changes = self.radio_changes.wrapping_add(1);
            }
            // Clearing a bond or unlocking Studio doesn't change what's shown.
            SystemAction::BtClear | SystemAction::StudioUnlock => {}
        }
    }
}

const BUCKET_MS: u32 = 5_000;
const BUCKETS: usize = 12; // 12 × 5 s = one-minute window

/// Words per minute over the last minute (one word = 5 key presses), like ZMK's WPM widget.
pub struct Wpm {
    buckets: [u16; BUCKETS],
    current: usize,
    bucket_start: u32,
}

impl Default for Wpm {
    fn default() -> Self {
        Self::new()
    }
}

impl Wpm {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buckets: [0; BUCKETS],
            current: 0,
            bucket_start: 0,
        }
    }

    pub fn key_pressed(&mut self, now: u32) {
        self.advance(now);
        self.buckets[self.current] = self.buckets[self.current].saturating_add(1);
    }

    /// Move the window forward to `now`, dropping old buckets.
    pub fn advance(&mut self, now: u32) {
        let elapsed = now.wrapping_sub(self.bucket_start);
        if elapsed >= BUCKET_MS * BUCKETS as u32 {
            self.buckets = [0; BUCKETS];
            self.bucket_start = now - now % BUCKET_MS;
            return;
        }
        while now.wrapping_sub(self.bucket_start) >= BUCKET_MS {
            self.current = (self.current + 1) % BUCKETS;
            self.buckets[self.current] = 0;
            self.bucket_start = self.bucket_start.wrapping_add(BUCKET_MS);
        }
    }

    #[must_use]
    pub fn wpm(&self) -> u16 {
        self.buckets.iter().sum::<u16>() / 5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bt_prev_from_first_profile_wraps_to_last() {
        // Arrange
        let mut s = Status::default();

        // Act
        s.apply(SystemAction::BtPrev);

        // Assert
        assert_eq!(s.profile, BT_PROFILES - 1);
    }

    #[test]
    fn bt_next_from_last_profile_wraps_to_first() {
        // Arrange
        let mut s = Status {
            profile: BT_PROFILES - 1,
            ..Status::default()
        };

        // Act
        s.apply(SystemAction::BtNext);

        // Assert
        assert_eq!(s.profile, 0);
    }

    #[test]
    fn out_toggle_switches_ble_to_usb() {
        // Arrange
        let mut s = Status::default();

        // Act
        s.apply(SystemAction::OutToggle);

        // Assert
        assert_eq!(s.transport, Transport::Usb);
    }

    #[test]
    fn radio_power_steps_up_from_the_default_and_counts_the_change() {
        // Arrange
        let mut s = Status::default();

        // Act
        s.apply(SystemAction::RadioPower);

        // Assert
        assert_eq!((s.radio_dbm, s.radio_changes), (8, 1));
    }

    #[test]
    fn wpm_counts_five_presses_as_one_word() {
        // Arrange
        let mut w = Wpm::new();

        // Act: 250 presses over 50 s
        for i in 0..250 {
            w.key_pressed(i * 200);
        }

        // Assert
        assert_eq!(w.wpm(), 50);
    }

    #[test]
    fn wpm_drops_presses_older_than_a_minute() {
        // Arrange
        let mut w = Wpm::new();
        for i in 0..250 {
            w.key_pressed(i * 200);
        }

        // Act
        w.advance(110_000);

        // Assert
        assert_eq!(w.wpm(), 0);
    }
}

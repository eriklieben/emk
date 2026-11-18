//! Bluetooth profiles as the firmware keeps them: which of the
//! [`BT_PROFILES`] slots have a computer paired, which one is selected,
//! and which (if any) has its computer connected.

use kb_display::status::{BT_PROFILES, Status};

/// Stages of the split link shown at the bottom of the OLEDs, in the
/// firmware's order (`fw-nrf/src/ble/split.rs`). Index 0 hides it.
pub const LINK_STAGES: [&str; 20] = [
    "", "start", "scan", "wait", "found", "adv", "conn", "pair", "sub", "linked", "disc", "c-host",
    "p-err", "d-err", "s-err", "a-adv", "c-ctl", "p-tmo", "a-acc", "a-srv",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profiles {
    /// Whether a computer is paired on each profile.
    taken: [bool; BT_PROFILES as usize],
    active: u8,
    /// The profile whose computer is connected. A keyboard only connects on
    /// the selected profile, so selecting another one drops it.
    connected: Option<u8>,
}

impl Default for Profiles {
    /// Profile 1 paired and connected, the others open.
    fn default() -> Self {
        let mut taken = [false; BT_PROFILES as usize];
        taken[0] = true;
        Self {
            taken,
            active: 0,
            connected: Some(0),
        }
    }
}

impl Profiles {
    #[must_use]
    pub fn active(&self) -> u8 {
        self.active
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.taken[self.active as usize]
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected == Some(self.active)
    }

    /// Select `profile` (0-based; out of range is ignored).
    pub fn select(&mut self, profile: u8) {
        if profile < BT_PROFILES && profile != self.active {
            self.active = profile;
            self.connected = None;
        }
    }

    pub fn next(&mut self) {
        self.select((self.active + 1) % BT_PROFILES);
    }

    pub fn prev(&mut self) {
        self.select((self.active + BT_PROFILES - 1) % BT_PROFILES);
    }

    /// `&bt BT_CLR`: forget the selected profile's computer.
    pub fn clear(&mut self) {
        self.set_open(true);
    }

    /// Open: no computer paired (which also means none connected).
    pub fn set_open(&mut self, open: bool) {
        self.taken[self.active as usize] = !open;
        if open && self.is_connected() {
            self.connected = None;
        }
    }

    /// Connecting pairs the selected profile if it was open.
    pub fn set_connected(&mut self, connected: bool) {
        if connected {
            self.taken[self.active as usize] = true;
            self.connected = Some(self.active);
        } else if self.is_connected() {
            self.connected = None;
        }
    }

    /// Copy the selected profile's state into what the screens show.
    pub fn show(&self, status: &mut Status) {
        status.profile = self.active;
        status.profile_open = self.is_open();
        status.profile_connected = self.is_connected();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_first_profile_connected() {
        // Arrange
        let p = Profiles::default();

        // Act
        let state = (p.active(), p.is_open(), p.is_connected());

        // Assert
        assert_eq!(state, (0, false, true));
    }

    #[test]
    fn next_selects_an_open_profile() {
        // Arrange
        let mut p = Profiles::default();

        // Act
        p.next();

        // Assert
        assert_eq!(
            (p.active(), p.is_open(), p.is_connected()),
            (1, true, false)
        );
    }

    #[test]
    fn prev_from_first_profile_wraps_to_last() {
        // Arrange
        let mut p = Profiles::default();

        // Act
        p.prev();

        // Assert
        assert_eq!(p.active(), BT_PROFILES - 1);
    }

    #[test]
    fn returning_to_a_taken_profile_shows_it_not_connected() {
        // Arrange
        let mut p = Profiles::default();
        p.next();

        // Act
        p.prev();

        // Assert
        assert_eq!(
            (p.active(), p.is_open(), p.is_connected()),
            (0, false, false)
        );
    }

    #[test]
    fn selecting_the_same_profile_keeps_the_connection() {
        // Arrange
        let mut p = Profiles::default();

        // Act
        p.select(0);

        // Assert
        assert!(p.is_connected());
    }

    #[test]
    fn selecting_out_of_range_is_ignored() {
        // Arrange
        let mut p = Profiles::default();

        // Act
        p.select(BT_PROFILES);

        // Assert
        assert_eq!(p.active(), 0);
    }

    #[test]
    fn clear_makes_the_profile_open_and_disconnected() {
        // Arrange
        let mut p = Profiles::default();

        // Act
        p.clear();

        // Assert
        assert_eq!((p.is_open(), p.is_connected()), (true, false));
    }

    #[test]
    fn clear_leaves_other_profiles_taken() {
        // Arrange
        let mut p = Profiles::default();
        p.select(2);
        p.set_connected(true);
        p.select(0);
        p.clear();

        // Act
        p.select(2);

        // Assert
        assert!(!p.is_open());
    }

    #[test]
    fn connecting_an_open_profile_takes_it() {
        // Arrange
        let mut p = Profiles::default();
        p.select(4);

        // Act
        p.set_connected(true);

        // Assert
        assert_eq!((p.is_open(), p.is_connected()), (false, true));
    }

    #[test]
    fn disconnecting_keeps_the_profile_taken() {
        // Arrange
        let mut p = Profiles::default();

        // Act
        p.set_connected(false);

        // Assert
        assert_eq!((p.is_open(), p.is_connected()), (false, false));
    }

    #[test]
    fn marking_a_profile_taken_does_not_connect_it() {
        // Arrange
        let mut p = Profiles::default();
        p.select(3);

        // Act
        p.set_open(false);

        // Assert
        assert_eq!((p.is_open(), p.is_connected()), (false, false));
    }

    #[test]
    fn show_copies_the_selected_profile_into_status() {
        // Arrange
        let mut p = Profiles::default();
        p.select(5);
        let mut status = Status::default();

        // Act
        p.show(&mut status);

        // Assert
        assert_eq!(
            (
                status.profile,
                status.profile_open,
                status.profile_connected
            ),
            (5, true, false)
        );
    }

    #[test]
    fn link_stages_start_with_hidden_then_firmware_names() {
        // Arrange
        let stages = LINK_STAGES;

        // Act
        let (first, last) = (stages[0], stages[stages.len() - 1]);

        // Assert
        assert_eq!((first, stages[1], last), ("", "start", "a-srv"));
    }
}

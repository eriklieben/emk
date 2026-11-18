//! The keyboard's Bluetooth, on the `trouble` host, for any controller:
//! Nordic's `SoftDevice` Controller in the firmware (`fw-nrf`), the in-memory
//! `blesim` air in the host tests.
//!
//! The split link: the right half is a peripheral that notifies its held
//! keys; the left half is the central that bonds with it once and feeds those
//! keys into the engine. The left half is also the computer's Bluetooth
//! keyboard (`hid`), with 10 profiles; `left` runs both links on one stack.
//!
//! Both halves' code is always compiled; the firmware calls the one it is.
//! Everything the rest of the firmware shares with a half is in its state,
//! [`RightState`] or [`LeftState`], which the firmware keeps in a `static`.
#![no_std]

use core::ops::Range;

use embassy_time::Duration;
use kb_core::link::{self, Pace};
use trouble_host::prelude::Address;

/// Write a line into a [`LogRing`]: `ring_log!(state.log, "split: {}", stage)`.
macro_rules! ring_log {
    ($ring:expr, $($arg:tt)*) => {
        $ring.write_line(format_args!($($arg)*))
    };
}

pub mod hid;
pub mod keymap;
pub mod left;
pub mod log;
pub mod notify;
pub mod radio;
pub mod split;
pub mod storage;
#[cfg(test)]
extern crate std;
#[cfg(test)]
mod test_flash;

pub use hid::{Command, HidState};
pub use left::LeftState;
pub use log::LogRing;
pub use split::{RightState, SplitLink, SplitServer};
pub use {bt_hci, trouble_host};

/// What a half needs from the firmware besides its controller and flash.
pub struct Params {
    /// This half's own (static random) address.
    pub address: Address,
    /// Flash addresses of the bond storage region.
    pub storage: Range<u32>,
    /// The keyboard's name, as the computer sees it.
    pub name: &'static str,
    pub timing: Timing,
    /// Sets the controller's transmit power for a role or link (left half,
    /// `RadioPower`); [`no_tx_power`] where the controller can't.
    pub set_tx_power: SetTxPower,
}

/// What a transmit power applies to (the Zephyr / Nordic vendor command's
/// handle types).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxTarget {
    /// Legacy advertising (to the computer).
    Advertising,
    /// Scanning and connecting (to the right half).
    ScanInit,
    /// The link with this connection handle.
    Connection(u16),
}

/// Set the transmit power of `target` to `dbm`: the level the controller
/// selected, or its HCI error status.
pub type SetTxPower = fn(target: TxTarget, dbm: i8) -> Result<i8, u8>;

/// Milliseconds since `since` for [`Timing::pace`], stopping at `u32::MAX`
/// (49 days) instead of wrapping back to the fast pace.
fn elapsed_ms(since: embassy_time::Instant) -> u32 {
    u32::try_from(since.elapsed().as_millis()).unwrap_or(u32::MAX)
}

/// A [`SetTxPower`] for a controller without per-role power: does nothing.
///
/// # Errors
///
/// Never: it reports `dbm` as selected.
pub fn no_tx_power(_: TxTarget, dbm: i8) -> Result<i8, u8> {
    Ok(dbm)
}

/// The controller's default transmit power (fw-nrf builds the `SoftDevice`
/// Controller with it): 0 dBm, `kb_core::link::RADIO_DEFAULT_DBM`. The left
/// half then sets the level it keeps in flash (the `RadioPower` key: +8, +4 or
/// 0 dBm, see [`radio`]) on its advertising, scanning and links; the right
/// half stays at this.
pub const TX_POWER: trouble_host::prelude::TxPower = tx_power(kb_core::link::RADIO_DEFAULT_DBM);

/// trouble's name for a [`kb_core::link::RADIO_DBM`] level (others: 0 dBm).
#[must_use]
pub const fn tx_power(dbm: i8) -> trouble_host::prelude::TxPower {
    use trouble_host::prelude::TxPower;
    match dbm {
        8 => TxPower::Plus8dBm,
        4 => TxPower::Plus4dBm,
        _ => TxPower::ZerodBm,
    }
}

/// The links' timeouts.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Right half: how long a connection has to prove it's the left half.
    pub split_verify: Duration,
    /// Right half: how long after the left half is verified the battery level
    /// and the log go out (time to subscribe).
    pub right_forward_delay: Duration,
    /// Right half: the held keys are notified at least this often.
    pub notify_heartbeat: Duration,
    /// Left half: how long the right half has to finish pairing or encrypting.
    pub pairing_timeout: Duration,
    /// Left half: how long a computer has to secure its link for its profile
    /// (a pairing code on the screen holds it off; it has its own timeout).
    pub host_secure_timeout: Duration,
    /// Left half: how long a pairing code waits to be typed (or answered)
    /// before the pairing is rejected.
    pub pair_timeout: Duration,
    /// How long to look for a link fast before slowing down
    /// (`kb_core::link::pace`); at most `kb_core::link::FAST_PHASE_MS`.
    pub fast_phase_ms: u32,
}

impl Timing {
    /// The firmware's timeouts.
    pub const FIRMWARE: Self = Self {
        split_verify: Duration::from_millis(link::SPLIT_VERIFY_MS as u64),
        right_forward_delay: Duration::from_secs(2),
        notify_heartbeat: Duration::from_secs(1),
        pairing_timeout: Duration::from_secs(10),
        host_secure_timeout: Duration::from_secs(30),
        pair_timeout: Duration::from_secs(60),
        fast_phase_ms: link::FAST_PHASE_MS,
    };

    /// `kb_core::link::pace` with this fast phase: the pace `elapsed_ms` after
    /// starting to look, and how long until it changes.
    #[must_use]
    pub fn pace(&self, elapsed_ms: u32) -> (Pace, Option<u32>) {
        let skipped = link::FAST_PHASE_MS.saturating_sub(self.fast_phase_ms);
        link::pace(elapsed_ms.saturating_add(skipped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trouble_host::prelude::TxPower;

    #[test]
    fn radio_levels_map_to_trouble_power() {
        // Arrange
        let levels = [8, 4, 0];

        // Act
        let powers = levels.map(tx_power);

        // Assert
        assert!(matches!(
            powers,
            [TxPower::Plus8dBm, TxPower::Plus4dBm, TxPower::ZerodBm]
        ));
    }

    #[test]
    fn level_without_a_name_is_zero_dbm() {
        // Arrange
        let dbm = -20;

        // Act
        let power = tx_power(dbm);

        // Assert
        assert!(matches!(power, TxPower::ZerodBm));
    }

    #[test]
    fn controller_without_per_role_power_reports_the_level_as_selected() {
        // Arrange
        let target = TxTarget::Connection(7);

        // Act
        let selected = no_tx_power(target, 4);

        // Assert
        assert_eq!(selected, Ok(4));
    }

    #[test]
    fn shorter_fast_phase_slows_down_sooner() {
        // Arrange
        let timing = Timing {
            fast_phase_ms: 2_000,
            ..Timing::FIRMWARE
        };

        // Act
        let (pace, _) = timing.pace(2_000);

        // Assert
        assert_eq!(pace, Timing::FIRMWARE.pace(link::FAST_PHASE_MS).0);
        assert_ne!(pace, Timing::FIRMWARE.pace(0).0);
    }
}

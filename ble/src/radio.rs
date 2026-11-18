//! The left half's radio transmit power, stepped with the `RadioPower` key
//! (+8 → +4 → 0 → +8 dBm, `kb_core::link::next_radio_dbm`), kept in flash
//! and applied at once: to advertising, to scanning / connecting, and to the
//! links that are up (the computer's and the right half's), through the
//! controller's per-role power command ([`SetTxPower`]).
//!
//! The right half's own transmitter stays at its controller default: it has
//! no key for this and the level isn't sent over the split link.

use core::cell::Cell;
use core::sync::atomic::{AtomicI8, AtomicU8, AtomicU16, Ordering};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embedded_storage_async::nor_flash::NorFlash;
use kb_core::link::{RADIO_DEFAULT_DBM, next_radio_dbm, valid_radio_dbm};

use crate::log::LogRing;
use crate::split::Store;
use crate::storage::{RADIO_POWER, StoredRadioPower};
use crate::{SetTxPower, TxTarget};

/// A link whose power follows the level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Split = 0,
    Host = 1,
}

const NO_HANDLE: u16 = 0xFFFF;

/// The level and the links it applies to.
pub struct Radio {
    dbm: AtomicI8,
    /// Times the level changed (wrapping), for the screen.
    changes: AtomicU8,
    step: Signal<CriticalSectionRawMutex, ()>,
    handles: [AtomicU16; 2],
    set: Mutex<CriticalSectionRawMutex, Cell<Option<SetTxPower>>>,
}

impl Default for Radio {
    fn default() -> Self {
        Self::new()
    }
}

impl Radio {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            dbm: AtomicI8::new(RADIO_DEFAULT_DBM),
            changes: AtomicU8::new(0),
            step: Signal::new(),
            handles: [AtomicU16::new(NO_HANDLE), AtomicU16::new(NO_HANDLE)],
            set: Mutex::new(Cell::new(None)),
        }
    }

    /// The level now, in dBm.
    pub fn dbm(&self) -> i8 {
        self.dbm.load(Ordering::Relaxed)
    }

    /// Times the level changed so far (wrapping).
    pub fn changes(&self) -> u8 {
        self.changes.load(Ordering::Relaxed)
    }

    /// The `RadioPower` key: step to the next level (the Bluetooth task does it).
    pub fn step(&self) {
        self.step.signal(());
    }

    fn apply(&self, target: TxTarget, log: &LogRing) {
        let Some(set) = self.set.lock(Cell::get) else {
            return;
        };
        let dbm = self.dbm();
        match set(target, dbm) {
            Ok(selected) if selected != dbm => {
                ring_log!(
                    log,
                    "radio: {:?} at {:+} dBm, not {:+}",
                    target,
                    selected,
                    dbm
                );
            }
            Ok(_) => {}
            Err(status) => ring_log!(log, "radio: {:?} power not set: 0x{:02x}", target, status),
        }
    }

    fn apply_all(&self, log: &LogRing) {
        self.apply(TxTarget::Advertising, log);
        self.apply(TxTarget::ScanInit, log);
        for h in &self.handles {
            let handle = h.load(Ordering::Relaxed);
            if handle != NO_HANDLE {
                self.apply(TxTarget::Connection(handle), log);
            }
        }
    }

    /// A link came up on `handle`: it gets the level now.
    pub(crate) fn connected(&self, link: Link, handle: u16, log: &LogRing) {
        self.handles[link as usize].store(handle, Ordering::Relaxed);
        self.apply(TxTarget::Connection(handle), log);
    }

    /// The link is down.
    pub(crate) fn disconnected(&self, link: Link) {
        self.handles[link as usize].store(NO_HANDLE, Ordering::Relaxed);
    }

    /// Before advertising: the advertiser gets the level.
    pub(crate) fn advertising(&self, log: &LogRing) {
        self.apply(TxTarget::Advertising, log);
    }
}

/// Load the level from flash (the default without one), apply it, then
/// follow the key forever: step, store, apply, log.
pub(crate) async fn run<S: NorFlash>(
    radio: &Radio,
    set: SetTxPower,
    store: &Store<S>,
    log: &LogRing,
) -> ! {
    radio.set.lock(|c| c.set(Some(set)));
    // As large as the largest item in the map (a bond): fetching reads
    // through the others.
    let mut buf = [0u8; 128];
    let stored = {
        let mut map = store.lock().await;
        map.fetch_item::<StoredRadioPower>(&mut buf, &RADIO_POWER)
            .await
    };
    let dbm = match stored {
        Ok(Some(StoredRadioPower(dbm))) => valid_radio_dbm(dbm),
        Ok(None) => RADIO_DEFAULT_DBM,
        Err(e) => {
            ring_log!(log, "storage: radio power not read: {:?}", e);
            RADIO_DEFAULT_DBM
        }
    };
    radio.dbm.store(dbm, Ordering::Relaxed);
    ring_log!(log, "radio: {:+} dBm", dbm);
    radio.apply_all(log);
    loop {
        radio.step.wait().await;
        let dbm = next_radio_dbm(radio.dbm());
        radio.dbm.store(dbm, Ordering::Relaxed);
        radio.changes.fetch_add(1, Ordering::Relaxed);
        radio.apply_all(log);
        ring_log!(log, "radio: now {:+} dBm", dbm);
        let mut map = store.lock().await;
        if let Err(e) = map
            .store_item(&mut buf, &RADIO_POWER, &StoredRadioPower(dbm))
            .await
        {
            ring_log!(log, "storage: radio power not stored: {:?}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_flash::{log_text, store};
    // The host's critical section, for the blocking mutexes.
    use critical_section as _;
    use embassy_futures::select::select;
    use embassy_futures::{block_on, yield_now};

    #[expect(
        clippy::unnecessary_wraps,
        reason = "a `SetTxPower`, which can fail on a real controller"
    )]
    fn selects_four(_: TxTarget, _: i8) -> Result<i8, u8> {
        Ok(4)
    }

    fn refuses(_: TxTarget, _: i8) -> Result<i8, u8> {
        Err(0x12)
    }

    /// Run `radio::run` until it waits (RAM flash answers at once).
    fn run_until_idle(
        radio: &Radio,
        set: SetTxPower,
        store: &Store<crate::test_flash::TestFlash>,
        log: &LogRing,
    ) {
        block_on(select(run(radio, set, store, log), async {
            for _ in 0..16 {
                yield_now().await;
            }
        }));
    }

    #[test]
    fn new_radio_is_at_the_default_level_unchanged() {
        // Arrange / Act
        let radio = Radio::default();

        // Assert
        assert_eq!((radio.dbm(), radio.changes()), (RADIO_DEFAULT_DBM, 0));
    }

    #[test]
    fn level_before_the_task_runs_goes_nowhere() {
        // Arrange
        let radio = Radio::new();
        let log = LogRing::new();

        // Act
        radio.advertising(&log);

        // Assert
        assert_eq!(log_text(&log), "");
    }

    #[test]
    fn controller_choosing_another_level_goes_to_the_log() {
        // Arrange
        let radio = Radio::new();
        let log = LogRing::new();
        radio.set.lock(|c| c.set(Some(selects_four)));

        // Act
        radio.connected(Link::Host, 3, &log);

        // Assert
        assert!(
            log_text(&log).contains("radio: Connection(3) at +4 dBm, not +0"),
            "{}",
            log_text(&log)
        );
    }

    #[test]
    fn controller_refusing_the_level_goes_to_the_log() {
        // Arrange
        let radio = Radio::new();
        let log = LogRing::new();
        radio.set.lock(|c| c.set(Some(refuses)));

        // Act
        radio.advertising(&log);

        // Assert
        assert!(
            log_text(&log).contains("radio: Advertising power not set: 0x12"),
            "{}",
            log_text(&log)
        );
    }

    #[test]
    fn level_that_cannot_be_read_falls_back_to_the_default() {
        // Arrange
        let radio = Radio::new();
        let log = LogRing::new();
        let store = store();
        block_on(store.lock()).flash().fail_reads = true;
        radio.dbm.store(8, Ordering::Relaxed);

        // Act
        run_until_idle(&radio, crate::no_tx_power, &store, &log);

        // Assert
        assert_eq!(radio.dbm(), RADIO_DEFAULT_DBM);
        assert!(log_text(&log).contains("storage: radio power not read"));
    }

    #[test]
    fn step_that_cannot_be_stored_still_applies() {
        // Arrange
        let radio = Radio::new();
        let log = LogRing::new();
        let store = store();
        block_on(store.lock()).flash().fail_writes = true;
        radio.step();

        // Act
        run_until_idle(&radio, crate::no_tx_power, &store, &log);

        // Assert
        assert_eq!((radio.dbm(), radio.changes()), (8, 1));
        assert!(log_text(&log).contains("storage: radio power not stored"));
    }

    #[test]
    fn link_that_went_down_no_longer_follows_the_level() {
        // Arrange
        let radio = Radio::new();
        let log = LogRing::new();
        radio.connected(Link::Split, 5, &log);

        // Act
        radio.disconnected(Link::Split);

        // Assert
        assert!(
            radio
                .handles
                .iter()
                .all(|h| h.load(Ordering::Relaxed) == NO_HANDLE)
        );
    }
}

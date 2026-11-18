//! Bonds in the firmware's own flash region, as a key-value map
//! (`sequential-storage`). The flash itself (on the nRF52840 written through
//! MPSL, so flash operations never disturb the radio) and the region's
//! addresses come from the firmware.

use embedded_storage_async::nor_flash::NorFlash;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{MapStorage, PostcardValue};
use serde::{Deserialize, Serialize};
use trouble_host::prelude::BondInformation;

use crate::log::LogRing;

/// Map key of the bond between the two halves.
pub const SPLIT_BOND: u8 = 0;

/// Map key of Bluetooth profile `i`'s bond with a computer (0-based, up to 10).
#[must_use]
pub const fn profile_bond(i: usize) -> u8 {
    0x10 + i as u8
}

/// Map key of what profile `i`'s computer subscribed to (its CCCD values):
/// a bonded computer may count on the keyboard remembering it across
/// restarts (Windows does; it doesn't subscribe again when it reconnects).
#[must_use]
pub const fn profile_subscriptions(i: usize) -> u8 {
    0x30 + i as u8
}

/// A computer's subscriptions as stored in flash: trouble's client attribute
/// table, raw (at most 64 bytes, in two halves for serde).
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct StoredSubscriptions {
    len: u8,
    first: [u8; 32],
    second: [u8; 32],
}

impl PostcardValue<'_> for StoredSubscriptions {}

impl StoredSubscriptions {
    /// `raw` (cut at 64 bytes).
    #[must_use]
    pub fn new(raw: &[u8]) -> Self {
        let len = raw.len().min(64);
        let mut bytes = [0u8; 64];
        bytes[..len].copy_from_slice(&raw[..len]);
        let (mut first, mut second) = ([0u8; 32], [0u8; 32]);
        first.copy_from_slice(&bytes[..32]);
        second.copy_from_slice(&bytes[32..]);
        Self {
            len: len as u8,
            first,
            second,
        }
    }

    /// The raw table, into `out`; returns its length.
    pub fn raw(&self, out: &mut [u8; 64]) -> usize {
        out[..32].copy_from_slice(&self.first);
        out[32..].copy_from_slice(&self.second);
        self.len as usize
    }
}

/// Map key of the selected Bluetooth profile.
pub const ACTIVE_PROFILE: u8 = 0x20;

/// The selected Bluetooth profile as stored in flash.
#[derive(Serialize, Deserialize)]
pub struct StoredProfile(pub u8);

impl PostcardValue<'_> for StoredProfile {}

/// The key-value store over the storage region.
pub type Map<S> = MapStorage<u8, S, NoCache>;

/// A stored bond, or `None` if there is none (or it was forgotten).
pub async fn load_bond<S: NorFlash>(
    map: &mut Map<S>,
    buf: &mut [u8],
    key: u8,
) -> Option<BondInformation> {
    match map.fetch_item::<StoredBond>(buf, &key).await {
        Ok(Some(StoredBond(bond))) => bond,
        _ => None,
    }
}

/// Store a bond under `key` (`None` forgets it); a failure goes to `log`.
pub async fn store_bond<S: NorFlash>(
    map: &mut Map<S>,
    buf: &mut [u8],
    key: u8,
    bond: Option<BondInformation>,
    log: &LogRing,
) {
    if let Err(e) = map.store_item(buf, &key, &StoredBond(bond)).await {
        ring_log!(log, "storage: writing key {} failed: {:?}", key, e);
    }
}

/// A bond as stored in flash. `None` is a forgotten bond: MPSL's flash can't
/// rewrite in place, so forgetting stores an empty entry instead of removing it.
#[derive(Serialize, Deserialize)]
pub struct StoredBond(pub Option<BondInformation>);

impl PostcardValue<'_> for StoredBond {}

/// Map key of the left half's radio transmit power (`RadioPower`).
pub const RADIO_POWER: u8 = 0x40;

/// The radio transmit power as stored in flash, in dBm.
#[derive(Serialize, Deserialize)]
pub struct StoredRadioPower(pub i8);

impl PostcardValue<'_> for StoredRadioPower {}

/// Map keys 0x50..=0x6F (key blocks) and 0x70..=0x77 (combos): Studio's
/// edited keymap, see [`crate::keymap`].
pub use crate::keymap::{KEYMAP_RECORDS, keymap_record};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_flash::{log_text, store};
    // The host's critical section, for the log's blocking mutex.
    use critical_section as _;
    use embassy_futures::block_on;
    use trouble_host::prelude::{Address, Identity, LongTermKey, SecurityLevel};

    fn bond(last: u8) -> BondInformation {
        BondInformation {
            ltk: LongTermKey(0x1234_5678),
            identity: Identity {
                addr: Address::random([1, 2, 3, 4, 5, last]),
                irk: None,
            },
            is_bonded: true,
            security_level: SecurityLevel::Encrypted,
        }
    }

    #[test]
    fn subscriptions_come_back_as_stored() {
        // Arrange
        let table = [7u8, 0, 1, 0, 9, 0, 2, 0];
        let stored = StoredSubscriptions::new(&table);
        let mut out = [0u8; 64];

        // Act
        let len = stored.raw(&mut out);

        // Assert
        assert_eq!(&out[..len], &table);
    }

    #[test]
    fn subscriptions_longer_than_64_bytes_are_cut() {
        // Arrange
        let table: [u8; 70] = core::array::from_fn(|i| i as u8);
        let stored = StoredSubscriptions::new(&table);
        let mut out = [0u8; 64];

        // Act
        let len = stored.raw(&mut out);

        // Assert
        assert_eq!(len, 64);
        assert_eq!(out[..], table[..64]);
    }

    #[test]
    fn profile_keys_do_not_overlap_the_other_keys() {
        // Arrange
        let profiles = 0..crate::hid::PROFILES;

        // Act
        let keys: std::vec::Vec<u8> = profiles
            .clone()
            .map(profile_bond)
            .chain(profiles.map(profile_subscriptions))
            .collect();

        // Assert
        for key in [SPLIT_BOND, ACTIVE_PROFILE, RADIO_POWER] {
            assert!(!keys.contains(&key), "key {key:#x}");
        }
        assert!(keys.iter().all(|&k| k < keymap_record(0)));
    }

    #[test]
    fn stored_bond_loads_back() {
        // Arrange
        let store = store();
        let log = LogRing::new();
        let mut buf = [0u8; 128];
        let mut map = block_on(store.lock());
        block_on(store_bond(
            &mut map,
            &mut buf,
            profile_bond(3),
            Some(bond(3)),
            &log,
        ));

        // Act
        let loaded = block_on(load_bond(&mut map, &mut buf, profile_bond(3)));

        // Assert
        assert_eq!(loaded.map(|b| b.identity), Some(bond(3).identity));
    }

    #[test]
    fn forgotten_bond_loads_as_none() {
        // Arrange
        let store = store();
        let log = LogRing::new();
        let mut buf = [0u8; 128];
        let mut map = block_on(store.lock());
        block_on(store_bond(
            &mut map,
            &mut buf,
            SPLIT_BOND,
            Some(bond(0)),
            &log,
        ));
        block_on(store_bond(&mut map, &mut buf, SPLIT_BOND, None, &log));

        // Act
        let loaded = block_on(load_bond(&mut map, &mut buf, SPLIT_BOND));

        // Assert
        assert!(loaded.is_none());
    }

    #[test]
    fn bond_that_cannot_be_read_loads_as_none() {
        // Arrange
        let store = store();
        let log = LogRing::new();
        let mut buf = [0u8; 128];
        let mut map = block_on(store.lock());
        block_on(store_bond(
            &mut map,
            &mut buf,
            SPLIT_BOND,
            Some(bond(0)),
            &log,
        ));
        map.flash().fail_reads = true;

        // Act
        let loaded = block_on(load_bond(&mut map, &mut buf, SPLIT_BOND));

        // Assert
        assert!(loaded.is_none());
    }

    #[test]
    fn failed_bond_write_goes_to_the_log() {
        // Arrange
        let store = store();
        let log = LogRing::new();
        let mut buf = [0u8; 128];
        let mut map = block_on(store.lock());
        map.flash().fail_writes = true;

        // Act
        block_on(store_bond(
            &mut map,
            &mut buf,
            profile_bond(2),
            Some(bond(2)),
            &log,
        ));

        // Assert
        assert!(
            log_text(&log).contains("storage: writing key 18 failed"),
            "{}",
            log_text(&log)
        );
    }
}

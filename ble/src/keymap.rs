//! Studio's edited keymap in flash (left half): the key block records of
//! `kb_core::studio::record` and the combo records of
//! `kb_core::studio::combo` under the map keys [`keymap_record`], loaded once
//! at start, written whenever Studio saves or resets.
//!
//! The firmware owns the keymap (a [`SharedStudio`] in a static, also used
//! by the key loop and the log port) and attaches it to the left half's
//! state ([`KeymapSlot::attach`]) before the Bluetooth task starts; this
//! module only moves records between it and flash, behind the
//! [`KeymapStore`] trait so the half's state needs no keymap sizes.

use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embedded_storage_async::nor_flash::NorFlash;
use kb_core::Keymap;
use kb_core::studio::record::{RECORD_MAX, RecordError};
use kb_core::studio::{COMBO_RECORD_BASE, LoadReport, Studio};

use crate::log::LogRing;
use crate::split::Store;

/// Map key of Studio record index `i`: key blocks (indices 0..32) at
/// 0x50..=0x6F, combo records (indices 32..40) at 0x70..=0x77.
#[must_use]
pub const fn keymap_record(i: usize) -> u8 {
    if i < COMBO_RECORD_BASE {
        0x50 + i as u8
    } else {
        0x70 + (i - COMBO_RECORD_BASE) as u8
    }
}

/// Most key block records (map keys 0x50..=0x6F).
pub const KEYMAP_RECORDS: usize = 32;

/// A keymap whose records go to flash; see [`SharedStudio`].
pub trait KeymapStore: Sync {
    /// Whether it has record index `i` (`i` < 64).
    fn has_record(&self, i: usize) -> bool;
    /// Record index `i`, read at start (in index order).
    ///
    /// # Errors
    ///
    /// Why the record can't be applied (the record stays unused).
    fn load(&self, i: usize, record: &[u8]) -> Result<(), RecordError>;
    /// All records are in.
    fn loaded(&self) -> LoadReport;
    /// Records to write now (bit i = record index i), each handed out once.
    fn take_pending(&self) -> u64;
    /// Record `index` as it should be stored; its length.
    fn record(&self, index: usize, out: &mut [u8; RECORD_MAX]) -> usize;
    /// Record `index` was written (or not).
    fn stored(&self, index: usize, ok: bool);
    /// Signalled when there are records to write.
    fn changed(&self) -> &Signal<CriticalSectionRawMutex, ()>;
}

/// Studio shared between the key loop, the log port and the flash.
pub struct SharedStudio<const N: usize, const L: usize> {
    studio: Mutex<CriticalSectionRawMutex, RefCell<Studio<'static, N, L>>>,
    /// The live bindings' generation, readable without the lock (the key
    /// loop checks it every scan and copies the bindings when it changed).
    generation: AtomicU32,
    changed: Signal<CriticalSectionRawMutex, ()>,
}

impl<const N: usize, const L: usize> SharedStudio<N, L> {
    #[must_use]
    pub const fn new(keymap: &'static Keymap<'static, N>) -> Self {
        Self {
            studio: Mutex::new(RefCell::new(Studio::new(keymap))),
            generation: AtomicU32::new(0),
            changed: Signal::new(),
        }
    }

    /// Use Studio. Afterwards the generation is published and the flash
    /// task woken if there are records to write.
    pub fn with<R>(&self, f: impl FnOnce(&mut Studio<'static, N, L>) -> R) -> R {
        let (result, generation, pending) = self.studio.lock(|s| {
            let mut s = s.borrow_mut();
            let result = f(&mut s);
            (result, s.generation(), s.pending() != 0)
        });
        self.generation.store(generation, Ordering::Relaxed);
        if pending {
            self.changed.signal(());
        }
        result
    }

    /// The live bindings' generation (cheap: no lock).
    pub fn generation(&self) -> u32 {
        self.generation.load(Ordering::Relaxed)
    }
}

impl<const N: usize, const L: usize> KeymapStore for SharedStudio<N, L> {
    fn has_record(&self, i: usize) -> bool {
        Studio::<'static, N, L>::record_indices().any(|r| r == i)
    }

    fn load(&self, i: usize, record: &[u8]) -> Result<(), RecordError> {
        self.with(|s| s.load(i, record))
    }

    fn loaded(&self) -> LoadReport {
        self.with(kb_core::studio::Studio::loaded)
    }

    fn take_pending(&self) -> u64 {
        self.with(kb_core::studio::Studio::take_pending)
    }

    fn record(&self, index: usize, out: &mut [u8; RECORD_MAX]) -> usize {
        self.with(|s| s.record(index, out))
    }

    fn stored(&self, index: usize, ok: bool) {
        self.with(|s| s.stored(index, ok));
    }

    fn changed(&self) -> &Signal<CriticalSectionRawMutex, ()> {
        &self.changed
    }
}

/// Where the left half's state finds the keymap to keep in flash.
pub struct KeymapSlot<'a> {
    store: Mutex<CriticalSectionRawMutex, Cell<Option<&'a dyn KeymapStore>>>,
}

impl Default for KeymapSlot<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> KeymapSlot<'a> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            store: Mutex::new(Cell::new(None)),
        }
    }

    /// Keep `store`'s records in flash (before the Bluetooth task starts).
    pub fn attach(&self, store: &'a dyn KeymapStore) {
        self.store.lock(|c| c.set(Some(store)));
    }

    fn get(&self) -> Option<&'a dyn KeymapStore> {
        self.store.lock(Cell::get)
    }
}

/// Load the records into the attached keymap, then write what it asks to,
/// forever. Without a keymap attached, does nothing.
pub(crate) async fn run<S: NorFlash>(slot: &KeymapSlot<'_>, store: &Store<S>, log: &LogRing) -> ! {
    let Some(keymap) = slot.get() else {
        core::future::pending().await
    };
    // As large as the largest item in the map (a bond): fetching reads
    // through the others. A keymap record is at most RECORD_MAX + 1.
    let mut buf = [0u8; 128];
    let mut found = 0;
    {
        let mut map = store.lock().await;
        for i in (0..64).filter(|&i| keymap.has_record(i)) {
            match map.fetch_item::<&[u8]>(&mut buf, &keymap_record(i)).await {
                Ok(Some(record)) => match keymap.load(i, record) {
                    Ok(()) => found += 1,
                    Err(e) => ring_log!(log, "keymap: record {} skipped: {:?}", i, e),
                },
                Ok(None) => {}
                Err(e) => ring_log!(log, "storage: keymap record {} not read: {:?}", i, e),
            }
        }
    }
    let report = keymap.loaded();
    if let Some(e) = report.combos {
        ring_log!(
            log,
            "keymap: stored combos not used ({:?}); compiled combos",
            e
        );
    }
    if report.ok {
        ring_log!(log, "keymap: {} studio record(s) read", found);
    } else {
        ring_log!(
            log,
            "keymap: stored edits would lock Studio out; back to the compiled keymap"
        );
    }
    loop {
        let pending = keymap.take_pending();
        if pending == 0 {
            keymap.changed().wait().await;
            continue;
        }
        let mut map = store.lock().await;
        let mut written = 0;
        for i in (0..64).filter(|i| pending & (1 << i) != 0) {
            let mut record = [0u8; RECORD_MAX];
            let n = keymap.record(i, &mut record);
            let result = map
                .store_item(&mut buf, &keymap_record(i), &&record[..n])
                .await;
            if let Err(e) = &result {
                ring_log!(log, "storage: keymap record {} not stored: {:?}", i, e);
            } else {
                written += 1;
            }
            keymap.stored(i, result.is_ok());
        }
        ring_log!(log, "keymap: {} record(s) stored", written);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The host's critical section, for the blocking mutex.
    use critical_section as _;

    #[test]
    fn key_blocks_and_combos_get_their_own_map_keys() {
        // Arrange
        let indices = [0, 31, COMBO_RECORD_BASE, COMBO_RECORD_BASE + 7];

        // Act
        let keys = indices.map(keymap_record);

        // Assert
        assert_eq!(keys, [0x50, 0x6F, 0x70, 0x77]);
    }

    use kb_core::action::kp;
    use kb_core::keymaps::corne;
    use kb_core::keys::Z;

    type CorneStudio = SharedStudio<{ corne::KEYS }, { corne::LAYER_COUNT }>;

    #[test]
    fn edit_publishes_the_generation() {
        // Arrange
        let shared = CorneStudio::new(&corne::KEYMAP);
        let before = shared.generation();

        // Act
        shared.with(|s| {
            s.unlock(0);
            s.set(0, 1, kp(Z), 0).unwrap();
        });

        // Assert
        assert_ne!(shared.generation(), before);
    }

    #[test]
    fn save_signals_the_flash_task() {
        // Arrange
        let shared = CorneStudio::new(&corne::KEYMAP);
        shared.with(|s| {
            s.unlock(0);
            s.set(0, 1, kp(Z), 0).unwrap();
        });
        let before = KeymapStore::changed(&shared).signaled();

        // Act
        shared.with(|s| s.save(0).unwrap());

        // Assert
        assert!(!before);
        assert!(KeymapStore::changed(&shared).signaled());
    }

    #[test]
    fn store_has_key_blocks_and_combo_records() {
        // Arrange
        let shared = CorneStudio::new(&corne::KEYMAP);

        // Act
        let count = (0..64).filter(|&i| shared.has_record(i)).count();

        // Assert
        assert_eq!(count, 8 + 8);
        assert!(shared.has_record(7) && !shared.has_record(8));
        assert!(shared.has_record(COMBO_RECORD_BASE));
    }

    #[test]
    fn store_hands_out_the_saved_record() {
        // Arrange
        let shared = CorneStudio::new(&corne::KEYMAP);
        shared.with(|s| {
            s.unlock(0);
            s.set(0, 1, kp(Z), 0).unwrap();
            s.save(0).unwrap();
        });

        // Act
        let pending = shared.take_pending();
        let mut out = [0; RECORD_MAX];
        let n = shared.record(0, &mut out);
        shared.stored(0, true);

        // Assert
        assert_eq!(pending, 1);
        assert_eq!(&out[..3], &[1, 0, 0]);
        assert!(n > 6);
        assert!(!shared.with(|s| s.dirty()));
    }

    #[test]
    fn slot_without_a_keymap_has_none() {
        // Arrange
        let slot = KeymapSlot::new();

        // Act
        let got = slot.get().is_none();

        // Assert
        assert!(got);
    }

    // ---- the flash task (`run`), with a keymap that records what it's told ----

    use crate::test_flash::{TestFlash, log_text, store};
    use core::sync::atomic::AtomicU64;
    use embassy_futures::select::select;
    use embassy_futures::{block_on, yield_now};
    use kb_core::studio::combo::ComboRecordError;

    /// Records 0 and 1 (key blocks); `reject` can't be applied.
    struct FakeKeymap {
        reject: Option<usize>,
        report: LoadReport,
        pending: AtomicU64,
        loaded: std::sync::Mutex<std::vec::Vec<usize>>,
        stored: std::sync::Mutex<std::vec::Vec<(usize, bool)>>,
        changed: Signal<CriticalSectionRawMutex, ()>,
    }

    impl FakeKeymap {
        fn new() -> Self {
            Self {
                reject: None,
                report: LoadReport {
                    ok: true,
                    combos: None,
                },
                pending: AtomicU64::new(0),
                loaded: std::sync::Mutex::new(std::vec::Vec::new()),
                stored: std::sync::Mutex::new(std::vec::Vec::new()),
                changed: Signal::new(),
            }
        }
    }

    impl KeymapStore for FakeKeymap {
        fn has_record(&self, i: usize) -> bool {
            i < 2
        }
        fn load(&self, i: usize, _: &[u8]) -> Result<(), RecordError> {
            if self.reject == Some(i) {
                return Err(RecordError::Length);
            }
            self.loaded.lock().unwrap().push(i);
            Ok(())
        }
        fn loaded(&self) -> LoadReport {
            self.report
        }
        fn take_pending(&self) -> u64 {
            self.pending.swap(0, Ordering::Relaxed)
        }
        fn record(&self, index: usize, out: &mut [u8; RECORD_MAX]) -> usize {
            out[..3].copy_from_slice(&[1, index as u8, 0xAB]);
            3
        }
        fn stored(&self, index: usize, ok: bool) {
            self.stored.lock().unwrap().push((index, ok));
        }
        fn changed(&self) -> &Signal<CriticalSectionRawMutex, ()> {
            &self.changed
        }
    }

    /// Records 0 and 1 in the flash.
    fn store_with_records() -> Store<TestFlash> {
        let store = store();
        let mut buf = [0u8; 128];
        let mut map = block_on(store.lock());
        for i in 0..2u8 {
            block_on(map.store_item(&mut buf, &keymap_record(i.into()), &&[1, i, 0][..])).unwrap();
        }
        drop(map);
        store
    }

    /// Run the flash task until it waits (RAM flash answers at once).
    fn run_until_idle(keymap: &FakeKeymap, store: &Store<TestFlash>, log: &LogRing) {
        let slot = KeymapSlot::default();
        slot.attach(keymap);
        block_on(select(run(&slot, store, log), async {
            for _ in 0..16 {
                yield_now().await;
            }
        }));
    }

    #[test]
    fn stored_records_are_loaded_at_start() {
        // Arrange
        let keymap = FakeKeymap::new();
        let (store, log) = (store_with_records(), LogRing::new());

        // Act
        run_until_idle(&keymap, &store, &log);

        // Assert
        assert_eq!(*keymap.loaded.lock().unwrap(), [0, 1]);
        assert!(log_text(&log).contains("keymap: 2 studio record(s) read"));
    }

    #[test]
    fn record_that_does_not_apply_is_skipped() {
        // Arrange
        let mut keymap = FakeKeymap::new();
        keymap.reject = Some(1);
        let (store, log) = (store_with_records(), LogRing::new());

        // Act
        run_until_idle(&keymap, &store, &log);

        // Assert
        let text = log_text(&log);
        assert!(text.contains("keymap: record 1 skipped: Length"), "{text}");
        assert!(text.contains("keymap: 1 studio record(s) read"), "{text}");
    }

    #[test]
    fn unreadable_records_are_logged_and_left_out() {
        // Arrange
        let keymap = FakeKeymap::new();
        let (store, log) = (store_with_records(), LogRing::new());
        block_on(store.lock()).flash().fail_reads = true;

        // Act
        run_until_idle(&keymap, &store, &log);

        // Assert
        assert!(keymap.loaded.lock().unwrap().is_empty());
        assert!(log_text(&log).contains("storage: keymap record 0 not read"));
    }

    #[test]
    fn edits_that_would_lock_studio_out_are_reported() {
        // Arrange
        let mut keymap = FakeKeymap::new();
        keymap.report = LoadReport {
            ok: false,
            combos: Some(ComboRecordError::Mixed),
        };
        let (store, log) = (store_with_records(), LogRing::new());

        // Act
        run_until_idle(&keymap, &store, &log);

        // Assert
        let text = log_text(&log);
        assert!(
            text.contains("keymap: stored combos not used (Mixed)"),
            "{text}"
        );
        assert!(text.contains("would lock Studio out"), "{text}");
        assert!(!text.contains("studio record(s) read"), "{text}");
    }

    #[test]
    fn pending_record_is_written_to_its_map_key() {
        // Arrange
        let keymap = FakeKeymap::new();
        keymap.pending.store(0b10, Ordering::Relaxed);
        let (store, log) = (store(), LogRing::new());

        // Act
        run_until_idle(&keymap, &store, &log);

        // Assert
        let mut buf = [0u8; 128];
        let mut map = block_on(store.lock());
        let stored = block_on(map.fetch_item::<&[u8]>(&mut buf, &keymap_record(1)))
            .unwrap()
            .map(<[u8]>::to_vec);
        assert_eq!(stored.as_deref(), Some(&[1, 1, 0xAB][..]));
        assert_eq!(*keymap.stored.lock().unwrap(), [(1, true)]);
        assert!(log_text(&log).contains("keymap: 1 record(s) stored"));
    }

    #[test]
    fn record_that_cannot_be_written_stays_unsaved() {
        // Arrange
        let keymap = FakeKeymap::new();
        keymap.pending.store(0b01, Ordering::Relaxed);
        let (store, log) = (store(), LogRing::new());
        block_on(store.lock()).flash().fail_writes = true;

        // Act
        run_until_idle(&keymap, &store, &log);

        // Assert
        assert_eq!(*keymap.stored.lock().unwrap(), [(0, false)]);
        let text = log_text(&log);
        assert!(
            text.contains("storage: keymap record 0 not stored"),
            "{text}"
        );
        assert!(text.contains("keymap: 0 record(s) stored"), "{text}");
    }

    #[test]
    fn without_a_keymap_the_task_does_nothing() {
        // Arrange
        let slot = KeymapSlot::new();
        let (store, log) = (store_with_records(), LogRing::new());

        // Act
        block_on(select(run(&slot, &store, &log), yield_now()));

        // Assert
        assert_eq!(log_text(&log), "");
    }
}

//! Studio: the keymap edited live, from the computer (EMK's own studio app).
//!
//! [`Studio`] holds the bindings and the combos three times: as compiled
//! (the default), as last saved, and live (what the engine types with; the
//! caller copies them into [`Engine::set_layers`](crate::Engine::set_layers)
//! and [`Engine::set_combos`](crate::Engine::set_combos) whenever
//! [`generation`](Studio::generation) changes). It decides what may change:
//!
//! - reading is always allowed;
//! - edits (`set`, combo `set` / `add` / `remove`), `save` and `reset` only
//!   while unlocked: the physical `StudioUnlock` key unlocks for
//!   [`UNLOCK_IDLE_MS`] after the last unlock or accepted write; `lock` (or a
//!   restart) locks at once. `revert` only goes back to what was saved
//!   (already allowed once), so it works locked; checks change nothing and
//!   work locked too;
//! - an edit is only applied if a `StudioUnlock` key stays reachable
//!   ([`guard`]; combos count), so the keyboard can always be unlocked and reset.
//!
//! What is saved goes to flash as records: the positions that differ from
//! the compiled keymap ([`record`], record indices below
//! [`COMBO_RECORD_BASE`]) and the combo set if it differs from the compiled
//! one ([`combo`], indices from [`COMBO_RECORD_BASE`] on).
//! [`take_pending`](Studio::take_pending) says which records to write. The
//! text protocol is in [`protocol`], the action encoding in [`encoding`].

pub mod combo;
pub mod encoding;
pub mod guard;
pub mod protocol;
pub mod record;

use crate::action::Action;
use crate::engine::Keymap;
use combo::{COMBO_RECORDS, ComboLoad, ComboRecordError, ComboSet, ComboSpec};
use guard::Reach;
use record::{BLOCK_KEYS, RECORD_MAX, RecordError, blocks_per_layer};

/// Unlocked Studio locks itself after this long without an unlock press or
/// an accepted write (5 minutes).
pub const UNLOCK_IDLE_MS: u32 = 5 * 60 * 1000;

/// Record index of combo record 0 (combo record `j` is index
/// `COMBO_RECORD_BASE + j`); key blocks are the indices below.
pub const COMBO_RECORD_BASE: usize = 32;

/// Why an edit was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Needs the `StudioUnlock` key first.
    Locked,
    /// No such layer, position or combo.
    Range,
    /// Not an action (or a `mo` to a layer that doesn't exist).
    Action,
    /// Encodes longer than [`encoding::MAX_ENCODING`].
    TooLong,
    /// No `StudioUnlock` key could be pressed afterwards.
    Lockout,
    /// Too tangled to check for a lockout: refused to be safe.
    TooComplex,
    /// Combo positions: not 2 to 4 distinct positions of this keyboard.
    Positions,
    /// Combo name: not up to 16 of `A-Z a-z 0-9 _`.
    Name,
    /// Combo timeout (1..=65535 ms) or prior idle (0..=65535 ms) out of range.
    Timing,
    /// No room for another combo.
    Full,
    /// The command's words don't fit it.
    Syntax,
}

impl Error {
    /// Its word in the protocol (`#A ... err <word>`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::Range => "range",
            Self::Action => "action",
            Self::TooLong => "long",
            Self::Lockout => "lockout",
            Self::TooComplex => "complex",
            Self::Positions => "positions",
            Self::Name => "name",
            Self::Timing => "timing",
            Self::Full => "full",
            Self::Syntax => "syntax",
        }
    }
}

/// What reading the records at start gave ([`Studio::loaded`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadReport {
    /// False: the edits would have locked Studio out and were dropped (the
    /// compiled keymap and combos; storage is cleared).
    pub ok: bool,
    /// Why stored combo records were not used (compiled combos), if they weren't.
    pub combos: Option<ComboRecordError>,
}

/// The editable keymap of `L` layers × `N` positions; see the module documentation.
#[derive(Clone)]
pub struct Studio<'a, const N: usize, const L: usize> {
    keymap: &'a Keymap<'a, N>,
    live: [[Action; N]; L],
    saved: [[Action; N]; L],
    live_combos: ComboSet,
    saved_combos: ComboSet,
    unlocked: bool,
    /// Last unlock press or accepted write.
    active_at: u32,
    generation: u32,
    /// Records to write: bit = record index.
    pending: u64,
    /// Key block records in storage that override something.
    stored: u32,
    /// Storage holds a combo set other than the compiled one (or might).
    combos_stored: bool,
    /// Set id of the combo records stored last.
    combo_set_id: u8,
    combo_load: ComboLoad,
    /// The last write to storage failed: saved isn't really saved.
    store_failed: bool,
}

impl<'a, const N: usize, const L: usize> Studio<'a, N, L> {
    /// Blocks (storage records) per layer.
    pub const BLOCKS_PER_LAYER: usize = blocks_per_layer(N);
    /// Key block records in all (indices `0..BLOCKS`).
    pub const BLOCKS: usize = L * Self::BLOCKS_PER_LAYER;

    /// The compiled `keymap`, unedited and locked. It must have `L` layers.
    ///
    /// # Panics
    ///
    /// When the keymap doesn't have `L` layers, or its key blocks don't fit
    /// below the combo records (at compile time where it is a `const`).
    #[must_use]
    pub const fn new(keymap: &'a Keymap<'a, N>) -> Self {
        assert!(keymap.layers.len() == L, "the keymap must have L layers");
        assert!(
            Self::BLOCKS <= COMBO_RECORD_BASE,
            "at most 32 key block records"
        );
        let mut layers = [[Action::Trans; N]; L];
        let mut i = 0;
        while i < L {
            layers[i] = keymap.layers[i];
            i += 1;
        }
        let combos = ComboSet::compiled(keymap);
        Self {
            keymap,
            live: layers,
            saved: layers,
            live_combos: combos,
            saved_combos: combos,
            unlocked: false,
            active_at: 0,
            generation: 0,
            pending: 0,
            stored: 0,
            combos_stored: false,
            combo_set_id: 0,
            combo_load: ComboLoad::new(),
            store_failed: false,
        }
    }

    /// Every record index storage may hold, key blocks then combo records.
    pub fn record_indices() -> impl Iterator<Item = usize> {
        (0..Self::BLOCKS).chain(COMBO_RECORD_BASE..COMBO_RECORD_BASE + COMBO_RECORDS)
    }

    #[must_use]
    pub fn keymap(&self) -> &'a Keymap<'a, N> {
        self.keymap
    }

    /// The live bindings (what the engine should use).
    #[must_use]
    pub fn layers(&self) -> &[[Action; N]; L] {
        &self.live
    }

    /// The saved bindings (what storage has, or is about to get).
    #[must_use]
    pub fn saved_layers(&self) -> &[[Action; N]; L] {
        &self.saved
    }

    /// The compiled bindings.
    #[must_use]
    pub fn default_layers(&self) -> &'a [[Action; N]] {
        self.keymap.layers
    }

    /// The live binding of `pos` on `layer` (no `&trans` fall-through).
    #[must_use]
    pub fn binding(&self, layer: u8, pos: u8) -> Option<Action> {
        self.live.get(layer as usize)?.get(pos as usize).copied()
    }

    /// The compiled binding of `pos` on `layer`.
    #[must_use]
    pub fn default_binding(&self, layer: u8, pos: u8) -> Option<Action> {
        self.keymap
            .layers
            .get(layer as usize)?
            .get(pos as usize)
            .copied()
    }

    /// The live combos (what the engine should use: `combos().defs()`).
    #[must_use]
    pub fn combos(&self) -> &ComboSet {
        &self.live_combos
    }

    /// The saved combos.
    #[must_use]
    pub fn saved_combos(&self) -> &ComboSet {
        &self.saved_combos
    }

    /// The compiled combos.
    #[must_use]
    pub fn default_combos(&self) -> ComboSet {
        ComboSet::compiled(self.keymap)
    }

    /// Counts changes of the live bindings or combos (wrapping).
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation
    }

    fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Whether the live bindings or combos differ from what is stored.
    #[must_use]
    pub fn dirty(&self) -> bool {
        self.live != self.saved
            || self.live_combos != self.saved_combos
            || self.pending != 0
            || self.store_failed
    }

    // ---- lock ----------------------------------------------------------

    /// The `StudioUnlock` key was pressed.
    pub fn unlock(&mut self, now: u32) {
        self.unlocked = true;
        self.active_at = now;
    }

    /// Lock now (`#studio lock`).
    pub fn lock(&mut self) {
        self.unlocked = false;
    }

    /// Whether writes are allowed at `now`.
    #[must_use]
    pub fn unlocked(&self, now: u32) -> bool {
        self.remaining_ms(now) > 0
    }

    /// Milliseconds until it locks itself; 0 when locked.
    #[must_use]
    pub fn remaining_ms(&self, now: u32) -> u32 {
        if !self.unlocked {
            return 0;
        }
        UNLOCK_IDLE_MS.saturating_sub(now.wrapping_sub(self.active_at))
    }

    /// Lock if the time is up; whether it just did (the screen shows it).
    pub fn expire(&mut self, now: u32) -> bool {
        if self.unlocked && self.remaining_ms(now) == 0 {
            self.unlocked = false;
            return true;
        }
        false
    }

    fn write_access(&mut self, now: u32) -> Result<(), Error> {
        if !self.unlocked(now) {
            self.unlocked = false;
            return Err(Error::Locked);
        }
        self.active_at = now;
        Ok(())
    }

    // ---- edits -----------------------------------------------------------

    /// Whether `layers` with `combos` keep a `StudioUnlock` key reachable.
    fn guard(&self, layers: &[[Action; N]; L], combos: &ComboSet) -> Result<(), Error> {
        match guard::unlock_reachable(layers, self.keymap.conditional_layers, combos.defs()) {
            Reach::Yes => Ok(()),
            Reach::No => Err(Error::Lockout),
            Reach::TooComplex => Err(Error::TooComplex),
        }
    }

    /// Check `action` for `pos` on `layer` without applying it (no lock check).
    ///
    /// # Errors
    ///
    /// [`Error::Range`], [`Error::Action`] or [`Error::TooLong`] for a binding
    /// that doesn't fit, [`Error::Lockout`] or [`Error::TooComplex`] when no
    /// `StudioUnlock` key would stay reachable.
    pub fn check(&self, layer: u8, pos: u8, action: Action) -> Result<(), Error> {
        if layer as usize >= L || pos as usize >= N {
            return Err(Error::Range);
        }
        if matches!(action, Action::MomentaryLayer(l) if l as usize >= L) {
            return Err(Error::Action);
        }
        if encoding::encoded(action).as_str().len() > encoding::MAX_ENCODING {
            return Err(Error::TooLong);
        }
        let mut next = self.live;
        next[layer as usize][pos as usize] = action;
        self.guard(&next, &self.live_combos)
    }

    /// Bind `action` to `pos` on `layer` (live, not saved yet).
    ///
    /// # Errors
    ///
    /// [`Error::Locked`] without the `StudioUnlock` key, otherwise as
    /// [`check`](Self::check).
    pub fn set(&mut self, layer: u8, pos: u8, action: Action, now: u32) -> Result<(), Error> {
        self.check(layer, pos, action)?;
        self.write_access(now)?;
        let slot = &mut self.live[layer as usize][pos as usize];
        if *slot != action {
            *slot = action;
            self.changed();
        }
        Ok(())
    }

    /// The combos after replacing combo `index` with `spec` (`None`: adding it), checked.
    fn combos_with(&self, index: Option<u8>, spec: &ComboSpec) -> Result<ComboSet, Error> {
        combo::validate(spec, N, L)?;
        let mut next = self.live_combos;
        match index {
            Some(i) if !next.set(i as usize, *spec) => return Err(Error::Range),
            Some(_) => {}
            None => {
                next.push(*spec).ok_or(Error::Full)?;
            }
        }
        self.guard(&self.live, &next)?;
        Ok(next)
    }

    /// Check combo `spec` as combo `index` (`None`: as a new one) without
    /// applying it (no lock check).
    ///
    /// # Errors
    ///
    /// What [`combo::validate`] finds, [`Error::Range`] or [`Error::Full`] for
    /// the index, [`Error::Lockout`] or [`Error::TooComplex`] when no
    /// `StudioUnlock` key would stay reachable.
    pub fn check_combo(&self, index: Option<u8>, spec: &ComboSpec) -> Result<(), Error> {
        self.combos_with(index, spec).map(|_| ())
    }

    /// Replace combo `index` (live).
    ///
    /// # Errors
    ///
    /// [`Error::Locked`] without the `StudioUnlock` key, otherwise as
    /// [`check_combo`](Self::check_combo).
    pub fn set_combo(&mut self, index: u8, spec: &ComboSpec, now: u32) -> Result<(), Error> {
        let next = self.combos_with(Some(index), spec)?;
        self.write_access(now)?;
        self.apply_combos(&next);
        Ok(())
    }

    /// Add a combo at the end (live); its index.
    ///
    /// # Errors
    ///
    /// [`Error::Locked`] without the `StudioUnlock` key, otherwise as
    /// [`check_combo`](Self::check_combo).
    pub fn add_combo(&mut self, spec: &ComboSpec, now: u32) -> Result<u8, Error> {
        let next = self.combos_with(None, spec)?;
        self.write_access(now)?;
        self.apply_combos(&next);
        Ok(self.live_combos.len() as u8 - 1)
    }

    /// Check removing combo `index` (no lock check).
    ///
    /// # Errors
    ///
    /// [`Error::Range`] for no such combo, [`Error::Lockout`] or
    /// [`Error::TooComplex`] when no `StudioUnlock` key would stay reachable.
    pub fn check_remove_combo(&self, index: u8) -> Result<(), Error> {
        let mut next = self.live_combos;
        if !next.remove(index as usize) {
            return Err(Error::Range);
        }
        self.guard(&self.live, &next)
    }

    /// Remove combo `index` (live; the later ones move down).
    ///
    /// # Errors
    ///
    /// As [`check_remove_combo`](Self::check_remove_combo), then
    /// [`Error::Locked`] without the `StudioUnlock` key.
    pub fn remove_combo(&mut self, index: u8, now: u32) -> Result<(), Error> {
        self.check_remove_combo(index)?;
        self.write_access(now)?;
        let mut next = self.live_combos;
        next.remove(index as usize);
        self.apply_combos(&next);
        Ok(())
    }

    fn apply_combos(&mut self, next: &ComboSet) {
        if *next != self.live_combos {
            self.live_combos = *next;
            self.changed();
        }
    }

    /// The combo records to write for the saved combos.
    fn combo_records(&self) -> u64 {
        let compiled = self.saved_combos == self.default_combos();
        let (_, records) = combo::plan((!compiled).then_some(&self.saved_combos));
        ((1u64 << records) - 1) << COMBO_RECORD_BASE
    }

    /// Keep the live bindings and combos: in RAM now, in storage once the
    /// records from [`take_pending`](Self::take_pending) are written.
    ///
    /// # Errors
    ///
    /// [`Error::Locked`] without the `StudioUnlock` key.
    pub fn save(&mut self, now: u32) -> Result<(), Error> {
        self.write_access(now)?;
        for layer in 0..L {
            for block in 0..Self::BLOCKS_PER_LAYER {
                let range = block_range::<N>(block);
                if self.live[layer][range.clone()] != self.saved[layer][range] {
                    self.pending |= 1 << (layer * Self::BLOCKS_PER_LAYER + block);
                }
            }
        }
        let combos_changed = self.live_combos != self.saved_combos;
        if self.store_failed {
            // Write everything edited again.
            self.pending |= u64::from(self.stored | self.edited_blocks(&self.live));
        }
        self.saved = self.live;
        self.saved_combos = self.live_combos;
        let compiled = self.saved_combos == self.default_combos();
        if (combos_changed || self.store_failed) && (!compiled || self.combos_stored) {
            self.combo_set_id = self.combo_set_id.wrapping_add(1);
            self.pending |= self.combo_records();
        }
        self.store_failed = false;
        Ok(())
    }

    /// Back to what was saved (allowed locked: it can only undo).
    pub fn revert(&mut self) {
        if self.live != self.saved || self.live_combos != self.saved_combos {
            self.live = self.saved;
            self.live_combos = self.saved_combos;
            self.changed();
        }
    }

    /// Back to the compiled keymap and combos, live and in storage.
    ///
    /// # Errors
    ///
    /// [`Error::Locked`] without the `StudioUnlock` key.
    pub fn reset(&mut self, now: u32) -> Result<(), Error> {
        self.write_access(now)?;
        self.pending |= u64::from(self.stored | self.edited_blocks(&self.saved));
        for (layer, compiled) in self.live.iter_mut().zip(self.keymap.layers) {
            *layer = *compiled;
        }
        self.saved = self.live;
        let compiled = self.default_combos();
        if self.combos_stored || self.saved_combos != compiled {
            self.combo_set_id = self.combo_set_id.wrapping_add(1);
            self.pending |= 1 << COMBO_RECORD_BASE;
        }
        self.live_combos = compiled;
        self.saved_combos = compiled;
        self.changed();
        Ok(())
    }

    /// A restart: unsaved edits are gone, Studio is locked.
    pub fn restart(&mut self) {
        self.revert();
        self.unlocked = false;
    }

    /// Blocks where `layers` differ from the compiled keymap.
    fn edited_blocks(&self, layers: &[[Action; N]; L]) -> u32 {
        let mut mask = 0;
        for (layer, (edited, compiled)) in layers.iter().zip(self.keymap.layers).enumerate() {
            for block in 0..Self::BLOCKS_PER_LAYER {
                let range = block_range::<N>(block);
                if edited[range.clone()] != compiled[range] {
                    mask |= 1 << (layer * Self::BLOCKS_PER_LAYER + block);
                }
            }
        }
        mask
    }

    // ---- storage ---------------------------------------------------------

    /// At start: apply a key block record read from storage (to saved and
    /// live). Call [`loaded`](Self::loaded) once all records are in.
    ///
    /// # Errors
    ///
    /// Why the record can't be applied; nothing of it is then.
    pub fn load_record(&mut self, bytes: &[u8]) -> Result<(), RecordError> {
        let block = record::decode_block(bytes)?;
        let (layer, index) = (block.layer as usize, block.block as usize);
        if layer >= L || index >= Self::BLOCKS_PER_LAYER {
            return Err(RecordError::Range);
        }
        let start = index * BLOCK_KEYS;
        for (i, action) in block.actions.iter().enumerate() {
            match action {
                Some(_) if start + i >= N => return Err(RecordError::Range),
                Some(Action::MomentaryLayer(l)) if *l as usize >= L => {
                    return Err(RecordError::Action);
                }
                _ => {}
            }
        }
        for (i, action) in block.actions.iter().enumerate() {
            if let Some(action) = action {
                self.saved[layer][start + i] = *action;
            }
        }
        if !block.is_empty() {
            self.stored |= 1 << (layer * Self::BLOCKS_PER_LAYER + index);
        }
        self.live = self.saved;
        self.changed();
        Ok(())
    }

    /// At start: combo record `index` (0-based, in order) read from storage.
    pub fn load_combo_record(&mut self, index: usize, bytes: &[u8]) {
        self.combo_load.record(&mut self.saved_combos, index, bytes);
    }

    /// At start: a record read from storage, by record index (key blocks
    /// and combo records alike). Combo record errors come with `loaded`.
    ///
    /// # Errors
    ///
    /// As [`load_record`](Self::load_record), for key block records.
    pub fn load(&mut self, index: usize, bytes: &[u8]) -> Result<(), RecordError> {
        if index >= COMBO_RECORD_BASE {
            self.load_combo_record(index - COMBO_RECORD_BASE, bytes);
            Ok(())
        } else {
            self.load_record(bytes)
        }
    }

    /// All records are in. Combos that came back whole replace the compiled
    /// ones. If what the records make up would lock Studio out (a compiled
    /// keymap changed under them), all of it is dropped, back to compiled,
    /// and cleared in storage.
    pub fn loaded(&mut self) -> LoadReport {
        let mut report = LoadReport {
            ok: true,
            combos: None,
        };
        let compiled = self.default_combos();
        match self.combo_load.finish(&self.saved_combos) {
            Ok(None) => self.saved_combos = compiled,
            Ok(Some(id)) => {
                let set = self.saved_combos;
                let valid = (0..set.len()).all(|i| {
                    set.get(i)
                        .is_some_and(|s| combo::validate(&s, N, L).is_ok())
                });
                if valid {
                    self.combo_set_id = id;
                } else {
                    report.combos = Some(ComboRecordError::Corrupt);
                    self.saved_combos = compiled;
                }
                self.combos_stored = true;
            }
            Err(e) => {
                report.combos = Some(e);
                self.saved_combos = compiled;
                // Something is there: the next save writes the combos whole.
                self.combos_stored = true;
            }
        }
        self.combo_load = ComboLoad::new();
        if self.guard(&self.saved, &self.saved_combos).is_err() {
            report.ok = false;
            self.pending |= u64::from(self.stored);
            if self.combos_stored {
                self.combo_set_id = self.combo_set_id.wrapping_add(1);
                self.pending |= 1 << COMBO_RECORD_BASE;
            }
            for (layer, compiled) in self.saved.iter_mut().zip(self.keymap.layers) {
                *layer = *compiled;
            }
            self.saved_combos = self.default_combos();
        }
        self.live = self.saved;
        self.live_combos = self.saved_combos;
        self.changed();
        report
    }

    /// The records waiting to be written (without taking them).
    #[must_use]
    pub fn pending(&self) -> u64 {
        self.pending
    }

    /// The records to write now (bit = record index); cleared here, so each
    /// is handed out once.
    pub fn take_pending(&mut self) -> u64 {
        core::mem::take(&mut self.pending)
    }

    /// Record `index` (a key block below [`COMBO_RECORD_BASE`], a combo
    /// record from it on) as it should be stored. Returns its length.
    pub fn record(&self, index: usize, out: &mut [u8; RECORD_MAX]) -> usize {
        if index >= COMBO_RECORD_BASE {
            let compiled = self.saved_combos == self.default_combos();
            return combo::encode_record(
                (!compiled).then_some(&self.saved_combos),
                index - COMBO_RECORD_BASE,
                self.combo_set_id,
                out,
            );
        }
        let (layer, block) = (
            index / Self::BLOCKS_PER_LAYER,
            index % Self::BLOCKS_PER_LAYER,
        );
        let layer = layer.min(L - 1);
        record::encode_block(
            layer as u8,
            block as u8,
            &self.saved[layer],
            &self.keymap.layers[layer],
            out,
        )
    }

    /// Storage wrote record `index` (or failed to).
    pub fn stored(&mut self, index: usize, ok: bool) {
        if !ok {
            self.store_failed = true;
            return;
        }
        if index >= COMBO_RECORD_BASE {
            self.combos_stored = self.saved_combos != self.default_combos();
            return;
        }
        let bit = 1u32 << index;
        if self.edited_blocks(&self.saved) & bit != 0 {
            self.stored |= bit;
        } else {
            self.stored &= !bit;
        }
    }
}

/// Positions of `block` in a layer of `N`.
fn block_range<const N: usize>(block: usize) -> core::ops::Range<usize> {
    let start = (block * BLOCK_KEYS).min(N);
    start..(start + BLOCK_KEYS).min(N)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{SystemAction, TRANS, kp, mo};
    use crate::keymaps::corne;
    use crate::keys::{A, B, Z};

    type Corne = Studio<'static, { corne::KEYS }, 4>;

    fn unlocked() -> Corne {
        let mut s = Corne::new(&corne::KEYMAP);
        s.unlock(1000);
        s
    }

    /// A studio that saved `kp(Z)` on layer 1 position 3, and its records.
    fn saved_records() -> Vec<Vec<u8>> {
        let mut s = unlocked();
        s.set(1, 3, kp(Z), 1000).unwrap();
        s.save(1000).unwrap();
        let pending = s.take_pending();
        (0..Corne::BLOCKS)
            .filter(|i| pending & (1 << i) != 0)
            .map(|i| {
                let mut out = [0; RECORD_MAX];
                let n = s.record(i, &mut out);
                out[..n].to_vec()
            })
            .collect()
    }

    #[test]
    fn starts_with_the_compiled_keymap() {
        // Arrange
        let s = Corne::new(&corne::KEYMAP);

        // Act
        let layers = s.layers();

        // Assert
        assert_eq!(&layers[..], &corne::LAYERS[..]);
    }

    #[test]
    fn starts_locked() {
        // Arrange
        let s = Corne::new(&corne::KEYMAP);

        // Act
        let open = s.unlocked(0);

        // Assert
        assert!(!open);
    }

    #[test]
    fn set_while_locked_is_refused() {
        // Arrange
        let mut s = Corne::new(&corne::KEYMAP);

        // Act
        let got = s.set(0, 1, kp(Z), 0);

        // Assert
        assert_eq!(got, Err(Error::Locked));
        assert_eq!(s.binding(0, 1), corne::LAYERS[0].get(1).copied());
    }

    #[test]
    fn set_while_unlocked_changes_the_live_binding() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = s.set(0, 1, kp(Z), 2000);

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(s.binding(0, 1), Some(kp(Z)));
    }

    #[test]
    fn set_bumps_the_generation() {
        // Arrange
        let mut s = unlocked();
        let before = s.generation();

        // Act
        s.set(0, 1, kp(Z), 2000).unwrap();

        // Assert
        assert_ne!(s.generation(), before);
    }

    #[test]
    fn edit_makes_it_dirty() {
        // Arrange
        let mut s = unlocked();

        // Act
        s.set(0, 1, kp(Z), 2000).unwrap();

        // Assert
        assert!(s.dirty());
    }

    #[test]
    fn unlock_lasts_five_idle_minutes() {
        // Arrange
        let s = unlocked();

        // Act
        let (before, after) = (
            s.unlocked(1000 + UNLOCK_IDLE_MS - 1),
            s.unlocked(1000 + UNLOCK_IDLE_MS),
        );

        // Assert
        assert!(before);
        assert!(!after);
    }

    #[test]
    fn accepted_write_extends_the_unlock() {
        // Arrange
        let mut s = unlocked();
        s.set(0, 1, kp(Z), 1000 + UNLOCK_IDLE_MS - 10).unwrap();

        // Act
        let open = s.unlocked(1000 + UNLOCK_IDLE_MS + 10);

        // Assert
        assert!(open);
    }

    #[test]
    fn refused_write_does_not_extend_the_unlock() {
        // Arrange
        let mut s = unlocked();
        let _ = s.set(9, 1, kp(Z), 1000 + UNLOCK_IDLE_MS - 10);

        // Act
        let open = s.unlocked(1000 + UNLOCK_IDLE_MS);

        // Assert
        assert!(!open);
    }

    #[test]
    fn set_after_the_timeout_is_refused() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = s.set(0, 1, kp(Z), 1000 + UNLOCK_IDLE_MS);

        // Assert
        assert_eq!(got, Err(Error::Locked));
    }

    #[test]
    fn expire_reports_the_moment_it_locks() {
        // Arrange
        let mut s = unlocked();

        // Act
        let locked = s.expire(1000 + UNLOCK_IDLE_MS);

        // Assert
        assert!(locked);
        assert!(!s.expire(1000 + UNLOCK_IDLE_MS + 1));
    }

    #[test]
    fn remaining_time_counts_down() {
        // Arrange
        let s = unlocked();

        // Act
        let left = s.remaining_ms(61_000);

        // Assert
        assert_eq!(left, UNLOCK_IDLE_MS - 60_000);
    }

    #[test]
    fn unlock_survives_the_clock_wrapping() {
        // Arrange
        let mut s = Corne::new(&corne::KEYMAP);
        s.unlock(u32::MAX - 100);

        // Act
        let open = s.unlocked(100);

        // Assert
        assert!(open);
    }

    #[test]
    fn lock_ends_the_unlock() {
        // Arrange
        let mut s = unlocked();

        // Act
        s.lock();

        // Assert
        assert!(!s.unlocked(1001));
    }

    #[test]
    fn set_out_of_range_is_refused() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = [s.set(4, 0, kp(A), 1000), s.set(0, 42, kp(A), 1000)];

        // Assert
        assert_eq!(got, [Err(Error::Range), Err(Error::Range)]);
    }

    #[test]
    fn mo_to_a_missing_layer_is_refused() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = s.set(0, 0, mo(4), 1000);

        // Assert
        assert_eq!(got, Err(Error::Action));
    }

    #[test]
    fn too_long_an_encoding_is_refused() {
        // Arrange
        let mut s = unlocked();
        let long = crate::Key {
            code: 0x48,
            mods: 0xFF,
        };

        // Act
        let got = s.set(0, 0, kp(long), 1000);

        // Assert
        assert_eq!(got, Err(Error::TooLong));
    }

    #[test]
    fn removing_the_unlock_key_is_refused() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = s.set(2, 35, TRANS, 1000);

        // Assert
        assert_eq!(got, Err(Error::Lockout));
        assert_eq!(
            s.binding(2, 35),
            Some(Action::System(SystemAction::StudioUnlock))
        );
    }

    #[test]
    fn removing_the_nav_key_is_refused() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = s.set(0, 41, kp(A), 1000);

        // Assert
        assert_eq!(got, Err(Error::Lockout));
    }

    #[test]
    fn moving_the_nav_key_in_two_steps_works() {
        // Arrange: a second way to nav first
        let mut s = unlocked();
        s.set(0, 39, mo(2), 1000).unwrap();

        // Act
        let got = s.set(0, 41, kp(A), 1000);

        // Assert
        assert_eq!(got, Ok(()));
    }

    #[test]
    fn revert_drops_unsaved_edits() {
        // Arrange
        let mut s = unlocked();
        s.set(0, 1, kp(Z), 1000).unwrap();
        s.save(1000).unwrap();
        s.set(0, 2, kp(Z), 1000).unwrap();

        // Act
        s.revert();

        // Assert
        assert_eq!(
            (s.binding(0, 1), s.binding(0, 2)),
            (Some(kp(Z)), corne::LAYERS[0].get(2).copied())
        );
    }

    #[test]
    fn revert_works_locked() {
        // Arrange
        let mut s = unlocked();
        s.set(0, 1, kp(Z), 1000).unwrap();
        s.lock();

        // Act
        s.revert();

        // Assert
        assert!(!s.dirty());
    }

    #[test]
    fn save_while_locked_is_refused() {
        // Arrange
        let mut s = unlocked();
        s.set(0, 1, kp(Z), 1000).unwrap();
        s.lock();

        // Act
        let got = s.save(2000);

        // Assert
        assert_eq!(got, Err(Error::Locked));
        assert_eq!(s.take_pending(), 0);
    }

    #[test]
    fn save_asks_to_write_only_the_edited_block() {
        // Arrange
        let mut s = unlocked();
        s.set(1, 30, kp(Z), 1000).unwrap();

        // Act
        s.save(1000).unwrap();

        // Assert: layer 1, second block
        assert_eq!(s.take_pending(), 1 << 3);
    }

    #[test]
    fn pending_records_are_handed_out_once() {
        // Arrange
        let mut s = unlocked();
        s.set(1, 30, kp(Z), 1000).unwrap();
        s.save(1000).unwrap();
        s.take_pending();

        // Act
        let again = s.take_pending();

        // Assert
        assert_eq!(again, 0);
    }

    #[test]
    fn saving_nothing_new_writes_nothing() {
        // Arrange
        let mut s = unlocked();
        s.set(1, 30, kp(Z), 1000).unwrap();
        s.save(1000).unwrap();
        s.take_pending();

        // Act
        s.save(1000).unwrap();

        // Assert
        assert_eq!(s.take_pending(), 0);
    }

    #[test]
    fn saved_keymap_is_clean_once_stored() {
        // Arrange
        let mut s = unlocked();
        s.set(1, 30, kp(Z), 1000).unwrap();
        s.save(1000).unwrap();
        s.take_pending();

        // Act
        s.stored(3, true);

        // Assert
        assert!(!s.dirty());
    }

    #[test]
    fn failed_store_leaves_it_dirty() {
        // Arrange
        let mut s = unlocked();
        s.set(1, 30, kp(Z), 1000).unwrap();
        s.save(1000).unwrap();
        s.take_pending();

        // Act
        s.stored(3, false);

        // Assert
        assert!(s.dirty());
    }

    #[test]
    fn saving_again_after_a_failed_store_rewrites_the_records() {
        // Arrange
        let mut s = unlocked();
        s.set(1, 30, kp(Z), 1000).unwrap();
        s.save(1000).unwrap();
        s.take_pending();
        s.stored(3, false);

        // Act
        s.save(1000).unwrap();

        // Assert
        assert_eq!(s.take_pending(), 1 << 3);
    }

    #[test]
    fn records_restore_the_saved_keymap_after_a_restart() {
        // Arrange
        let records = saved_records();
        let mut fresh = Corne::new(&corne::KEYMAP);

        // Act
        for r in &records {
            fresh.load_record(r).unwrap();
        }

        // Assert
        assert!(fresh.loaded().ok);
        assert_eq!(fresh.binding(1, 3), Some(kp(Z)));
        assert!(!fresh.dirty());
    }

    #[test]
    fn reset_restores_the_compiled_keymap() {
        // Arrange
        let mut s = unlocked();
        s.set(0, 1, kp(Z), 1000).unwrap();
        s.save(1000).unwrap();

        // Act
        s.reset(1000).unwrap();

        // Assert
        assert_eq!(&s.layers()[..], &corne::LAYERS[..]);
    }

    #[test]
    fn reset_clears_the_stored_records() {
        // Arrange: a keyboard started with a stored edit
        let mut s = Corne::new(&corne::KEYMAP);
        for r in &saved_records() {
            s.load_record(r).unwrap();
        }
        s.loaded();
        s.unlock(1000);

        // Act
        s.reset(1000).unwrap();

        // Assert: layer 1, first block, written again (empty)
        let pending = s.take_pending();
        assert_eq!(pending, 1 << 2);
        let mut out = [0; RECORD_MAX];
        let n = s.record(2, &mut out);
        assert!(record::decode_block(&out[..n]).unwrap().is_empty());
    }

    #[test]
    fn reset_while_locked_is_refused() {
        // Arrange
        let mut s = Corne::new(&corne::KEYMAP);

        // Act
        let got = s.reset(0);

        // Assert
        assert_eq!(got, Err(Error::Locked));
    }

    #[test]
    fn record_of_another_version_is_ignored() {
        // Arrange
        let mut s = Corne::new(&corne::KEYMAP);
        let mut record = saved_records().remove(0);
        record[0] = record::FORMAT_VERSION + 1;

        // Act
        let got = s.load_record(&record);

        // Assert
        assert_eq!(got, Err(RecordError::Version(record::FORMAT_VERSION + 1)));
        assert_eq!(&s.layers()[..], &corne::LAYERS[..]);
    }

    #[test]
    fn record_for_a_missing_layer_is_ignored() {
        // Arrange
        let mut s = Corne::new(&corne::KEYMAP);
        let record = [record::FORMAT_VERSION, 7, 0, 1, 0, 0, 1];

        // Act
        let got = s.load_record(&record);

        // Assert
        assert_eq!(got, Err(RecordError::Range));
    }

    #[test]
    fn record_with_a_mo_to_a_missing_layer_is_ignored() {
        // Arrange
        let mut s = Corne::new(&corne::KEYMAP);
        let record = [record::FORMAT_VERSION, 0, 0, 1, 0, 0, 4, 9];

        // Act
        let got = s.load_record(&record);

        // Assert
        assert_eq!(got, Err(RecordError::Action));
        assert_eq!(&s.layers()[..], &corne::LAYERS[..]);
    }

    #[test]
    fn stored_records_that_lock_out_are_dropped() {
        // Arrange: a record taking the nav key away (as if the compiled
        // keymap had changed under it)
        let mut s = Corne::new(&corne::KEYMAP);
        let mut live = corne::LAYERS[0];
        live[41] = kp(B);
        let mut out = [0; RECORD_MAX];
        let n = record::encode_block(0, 1, &live, &corne::LAYERS[0], &mut out);
        s.load_record(&out[..n]).unwrap();

        // Act
        let ok = s.loaded().ok;

        // Assert: back to compiled, and the record gets cleared
        assert!(!ok);
        assert_eq!(&s.layers()[..], &corne::LAYERS[..]);
        assert_eq!(s.take_pending(), 1 << 1);
    }

    #[test]
    fn restart_drops_unsaved_edits_and_locks() {
        // Arrange
        let mut s = unlocked();
        s.set(0, 1, kp(Z), 1000).unwrap();

        // Act
        s.restart();

        // Assert
        assert!(!s.unlocked(1000));
        assert_eq!(&s.layers()[..], &corne::LAYERS[..]);
    }

    // ---- combos ----------------------------------------------------------

    fn combo(text: &str) -> ComboSpec {
        combo::parse_spec(text.split(' ')).unwrap()
    }

    /// The records a save asked for, as storage would write them.
    fn written(s: &mut Corne) -> Vec<(usize, Vec<u8>)> {
        let pending = s.take_pending();
        let out: Vec<_> = Corne::record_indices()
            .filter(|i| pending & (1 << i) != 0)
            .map(|i| {
                let mut out = [0; RECORD_MAX];
                let n = s.record(i, &mut out);
                (i, out[..n].to_vec())
            })
            .collect();
        for (i, _) in &out {
            s.stored(*i, true);
        }
        out
    }

    fn restarted(records: &[(usize, Vec<u8>)]) -> (Corne, LoadReport) {
        let mut s = Corne::new(&corne::KEYMAP);
        for (i, r) in records {
            let _ = s.load(*i, r);
        }
        let report = s.loaded();
        (s, report)
    }

    #[test]
    fn starts_with_the_compiled_combos() {
        // Arrange
        let s = Corne::new(&corne::KEYMAP);

        // Act
        let combos = s.combos();

        // Assert
        assert_eq!(*combos, ComboSet::compiled(&corne::KEYMAP));
        assert_eq!(combos.len(), corne::COMBOS.len());
    }

    #[test]
    fn add_combo_while_locked_is_refused() {
        // Arrange
        let mut s = Corne::new(&corne::KEYMAP);

        // Act
        let got = s.add_combo(&combo("x 1,2 0 50 0 kp:A"), 0);

        // Assert
        assert_eq!(got, Err(Error::Locked));
        assert_eq!(s.combos().len(), 6);
    }

    #[test]
    fn add_combo_appends_it() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = s.add_combo(&combo("x 1,2 0 50 0 kp:A"), 1000);

        // Assert
        assert_eq!(got, Ok(6));
        assert_eq!(s.combos().get(6), Some(combo("x 1,2 0 50 0 kp:A")));
        assert!(s.dirty());
    }

    #[test]
    fn add_combo_bumps_the_generation() {
        // Arrange
        let mut s = unlocked();
        let before = s.generation();

        // Act
        s.add_combo(&combo("x 1,2 0 50 0 kp:A"), 1000).unwrap();

        // Assert
        assert_ne!(s.generation(), before);
    }

    #[test]
    fn add_combo_when_full_is_refused() {
        // Arrange
        let mut s = unlocked();
        for i in 0..10u8 {
            s.add_combo(&combo(&format!("x {},{} 0 50 0 kp:A", i, i + 1)), 1000)
                .unwrap();
        }

        // Act
        let got = s.add_combo(&combo("x 30,31 0 50 0 kp:A"), 1000);

        // Assert
        assert_eq!(got, Err(Error::Full));
    }

    #[test]
    fn invalid_combo_is_refused() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = [
            s.add_combo(&combo("x 1 0 50 0 kp:A"), 1000),
            s.add_combo(&combo("x 1,42 0 50 0 kp:A"), 1000),
            s.add_combo(&combo("x 1,2 5 50 0 kp:A"), 1000),
            s.add_combo(&combo("x 1,2 0 0 0 kp:A"), 1000),
            s.add_combo(&combo("x 1,2 0 50 0 mo:7"), 1000),
        ];

        // Assert
        assert_eq!(
            got,
            [
                Err(Error::Positions),
                Err(Error::Positions),
                Err(Error::Range),
                Err(Error::Timing),
                Err(Error::Action)
            ]
        );
    }

    #[test]
    fn set_combo_replaces_it() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = s.set_combo(0, &combo("cut 25,26 0 60 0 kp:LC(X)"), 1000);

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(s.combos().get(0).unwrap().def.timeout_ms, 60);
    }

    #[test]
    fn set_missing_combo_is_refused() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = s.set_combo(6, &combo("x 1,2 0 50 0 kp:A"), 1000);

        // Assert
        assert_eq!(got, Err(Error::Range));
    }

    #[test]
    fn remove_combo_moves_the_later_ones_down() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = s.remove_combo(0, 1000);

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(s.combos().get(0).unwrap().name.as_str(), "copy");
        assert_eq!(s.combos().len(), 5);
    }

    #[test]
    fn remove_missing_combo_is_refused() {
        // Arrange
        let mut s = unlocked();

        // Act
        let got = s.remove_combo(6, 1000);

        // Assert
        assert_eq!(got, Err(Error::Range));
    }

    #[test]
    fn remove_combo_while_locked_is_refused() {
        // Arrange
        let mut s = Corne::new(&corne::KEYMAP);

        // Act
        let got = s.remove_combo(0, 0);

        // Assert
        assert_eq!(got, Err(Error::Locked));
    }

    #[test]
    fn unlock_combo_lets_the_unlock_key_go() {
        // Arrange: an unlock combo on the base layer first
        let mut s = unlocked();
        s.add_combo(&combo("u 0,11 0 50 0 sys:StudioUnlock"), 1000)
            .unwrap();

        // Act
        let got = s.set(2, 35, TRANS, 1000);

        // Assert
        assert_eq!(got, Ok(()));
    }

    #[test]
    fn removing_the_last_way_to_unlock_through_a_combo_is_refused() {
        // Arrange: the unlock key gone, only a combo left
        let mut s = unlocked();
        s.add_combo(&combo("u 0,11 0 50 0 sys:StudioUnlock"), 1000)
            .unwrap();
        s.set(2, 35, TRANS, 1000).unwrap();

        // Act
        let got = s.remove_combo(6, 1000);

        // Assert
        assert_eq!(got, Err(Error::Lockout));
        assert_eq!(s.combos().len(), 7);
    }

    #[test]
    fn changing_the_last_unlock_combo_is_refused() {
        // Arrange
        let mut s = unlocked();
        s.add_combo(&combo("u 0,11 0 50 0 sys:StudioUnlock"), 1000)
            .unwrap();
        s.set(2, 35, TRANS, 1000).unwrap();

        // Act
        let got = s.set_combo(6, &combo("u 0,11 0 50 0 kp:A"), 1000);

        // Assert
        assert_eq!(got, Err(Error::Lockout));
    }

    #[test]
    fn check_combo_changes_nothing_and_ignores_the_lock() {
        // Arrange
        let s = Corne::new(&corne::KEYMAP);

        // Act
        let got = s.check_combo(None, &combo("x 1,2 0 50 0 kp:A"));

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(s.combos().len(), 6);
    }

    #[test]
    fn check_remove_combo_reports_a_missing_one() {
        // Arrange
        let s = Corne::new(&corne::KEYMAP);

        // Act
        let got = s.check_remove_combo(9);

        // Assert
        assert_eq!(got, Err(Error::Range));
    }

    #[test]
    fn revert_restores_the_combos() {
        // Arrange
        let mut s = unlocked();
        s.remove_combo(0, 1000).unwrap();

        // Act
        s.revert();

        // Assert
        assert_eq!(*s.combos(), ComboSet::compiled(&corne::KEYMAP));
        assert!(!s.dirty());
    }

    #[test]
    fn unsaved_combo_edit_is_gone_after_a_restart() {
        // Arrange
        let mut s = unlocked();
        s.remove_combo(0, 1000).unwrap();

        // Act
        s.restart();

        // Assert
        assert_eq!(s.combos().len(), 6);
    }

    #[test]
    fn saving_combos_asks_for_combo_records() {
        // Arrange
        let mut s = unlocked();
        s.remove_combo(0, 1000).unwrap();

        // Act
        s.save(1000).unwrap();

        // Assert: the set (5 combos) in one record, no key blocks
        assert_eq!(s.take_pending(), 1 << COMBO_RECORD_BASE);
    }

    #[test]
    fn saving_compiled_combos_writes_nothing() {
        // Arrange
        let mut s = unlocked();
        s.set(0, 1, kp(Z), 1000).unwrap();

        // Act
        s.save(1000).unwrap();

        // Assert: only the key block
        assert_eq!(s.take_pending(), 1);
    }

    #[test]
    fn saved_combos_come_back_after_a_restart() {
        // Arrange
        let mut s = unlocked();
        s.add_combo(&combo("x 1,2 0 50 0 kp:A"), 1000).unwrap();
        s.save(1000).unwrap();
        let records = written(&mut s);

        // Act
        let (back, report) = restarted(&records);

        // Assert
        assert_eq!(
            report,
            LoadReport {
                ok: true,
                combos: None
            }
        );
        assert_eq!(back.combos(), s.combos());
        assert!(!back.dirty());
    }

    #[test]
    fn empty_combo_set_comes_back_empty() {
        // Arrange
        let mut s = unlocked();
        for _ in 0..6 {
            s.remove_combo(0, 1000).unwrap();
        }
        s.save(1000).unwrap();
        let records = written(&mut s);

        // Act
        let (back, _) = restarted(&records);

        // Assert
        assert!(back.combos().is_empty());
    }

    #[test]
    fn saving_back_to_compiled_combos_writes_the_marker() {
        // Arrange: a saved edit, then undone and saved again
        let mut s = unlocked();
        s.remove_combo(0, 1000).unwrap();
        s.save(1000).unwrap();
        let mut records = written(&mut s);
        s.add_combo(&s.default_combos().get(0).unwrap(), 1000)
            .unwrap();
        s.set_combo(5, &s.default_combos().get(0).unwrap(), 1000)
            .ok();
        s.reset(1000).unwrap();
        let marker = written(&mut s);
        records.retain(|(i, _)| !marker.iter().any(|(m, _)| m == i));
        records.extend(marker);
        records.sort();

        // Act
        let (back, report) = restarted(&records);

        // Assert
        assert_eq!(report.combos, None);
        assert_eq!(*back.combos(), ComboSet::compiled(&corne::KEYMAP));
    }

    #[test]
    fn reset_clears_the_stored_combos() {
        // Arrange: a keyboard started with a stored combo edit
        let mut s = unlocked();
        s.remove_combo(0, 1000).unwrap();
        s.save(1000).unwrap();
        let records = written(&mut s);
        let (mut back, _) = restarted(&records);
        back.unlock(0);

        // Act
        back.reset(0).unwrap();

        // Assert: combo record 0 is written again, as the marker
        let pending = back.take_pending();
        assert_eq!(pending, 1 << COMBO_RECORD_BASE);
        let mut out = [0; RECORD_MAX];
        let n = back.record(COMBO_RECORD_BASE, &mut out);
        assert_eq!(out[3], 0xFF);
        assert_eq!(n, 5);
    }

    #[test]
    fn reset_restores_the_compiled_combos() {
        // Arrange
        let mut s = unlocked();
        s.remove_combo(0, 1000).unwrap();

        // Act
        s.reset(1000).unwrap();

        // Assert
        assert_eq!(*s.combos(), ComboSet::compiled(&corne::KEYMAP));
    }

    #[test]
    fn corrupt_combo_records_mean_compiled_combos() {
        // Arrange
        let mut s = unlocked();
        s.remove_combo(0, 1000).unwrap();
        s.save(1000).unwrap();
        let mut records = written(&mut s);
        records[0].1.pop();

        // Act
        let (back, report) = restarted(&records);

        // Assert
        assert_eq!(report.combos, Some(ComboRecordError::Corrupt));
        assert_eq!(*back.combos(), ComboSet::compiled(&corne::KEYMAP));
    }

    #[test]
    fn combo_records_from_two_saves_mean_compiled_combos() {
        // Arrange: a set over several records; the second save stopped
        // after its first record
        let mut s = unlocked();
        for i in 0..10u8 {
            s.add_combo(
                &combo(&format!("long_combo_name {},{} 0 50 0 kp:A", i, i + 1)),
                1000,
            )
            .unwrap();
        }
        s.save(1000).unwrap();
        let first = written(&mut s);
        s.remove_combo(0, 1000).unwrap();
        s.save(1000).unwrap();
        let mut second = written(&mut s);
        assert!(second.len() > 1);
        second[1] = first[1].clone();

        // Act
        let (back, report) = restarted(&second);

        // Assert
        assert_eq!(report.combos, Some(ComboRecordError::Mixed));
        assert_eq!(back.combos().len(), 6);
    }

    #[test]
    fn stored_combo_with_a_missing_layer_is_refused() {
        // Arrange: a combo record of a keyboard with more layers
        let mut set = ComboSet::compiled(&corne::KEYMAP);
        set.push(combo("x 1,2 0 50 0 mo:6"));
        let mut out = [0; RECORD_MAX];
        let (_, n) = combo::plan(Some(&set));
        let records: Vec<_> = (0..n)
            .map(|j| {
                let len = combo::encode_record(Some(&set), j, 1, &mut out);
                (COMBO_RECORD_BASE + j, out[..len].to_vec())
            })
            .collect();

        // Act
        let (back, report) = restarted(&records);

        // Assert
        assert_eq!(report.combos, Some(ComboRecordError::Corrupt));
        assert_eq!(back.combos().len(), 6);
    }

    #[test]
    fn stored_edits_that_lock_out_drop_the_combos_too() {
        // Arrange: the unlock key moved to a combo, and the combo record lost
        let mut s = unlocked();
        s.add_combo(&combo("u 0,11 0 50 0 sys:StudioUnlock"), 1000)
            .unwrap();
        s.set(2, 35, TRANS, 1000).unwrap();
        s.save(1000).unwrap();
        let records: Vec<_> = written(&mut s)
            .into_iter()
            .filter(|(i, _)| *i < COMBO_RECORD_BASE)
            .collect();

        // Act
        let (mut back, report) = restarted(&records);

        // Assert
        assert!(!report.ok);
        assert_eq!(&back.layers()[..], &corne::LAYERS[..]);
        assert_ne!(back.take_pending(), 0, "the records are cleared");
    }

    #[test]
    fn failed_combo_store_leaves_it_dirty() {
        // Arrange
        let mut s = unlocked();
        s.remove_combo(0, 1000).unwrap();
        s.save(1000).unwrap();
        s.take_pending();

        // Act
        s.stored(COMBO_RECORD_BASE, false);

        // Assert
        assert!(s.dirty());
    }

    #[test]
    fn saving_again_after_a_failed_store_rewrites_the_combos() {
        // Arrange
        let mut s = unlocked();
        s.remove_combo(0, 1000).unwrap();
        s.save(1000).unwrap();
        s.take_pending();
        s.stored(COMBO_RECORD_BASE, false);

        // Act
        s.save(1000).unwrap();

        // Assert
        assert_ne!(s.take_pending() >> COMBO_RECORD_BASE, 0);
    }

    #[test]
    fn every_save_gets_a_new_set_id() {
        // Arrange
        let mut s = unlocked();
        s.remove_combo(0, 1000).unwrap();
        s.save(1000).unwrap();
        let first = written(&mut s)[0].1[2];
        s.remove_combo(0, 1000).unwrap();
        s.save(1000).unwrap();

        // Act
        let second = written(&mut s)[0].1[2];

        // Assert
        assert_ne!(first, second);
    }

    #[test]
    fn record_indices_cover_blocks_and_combo_records() {
        // Arrange / Act
        let indices: Vec<_> = Corne::record_indices().collect();

        // Assert
        assert_eq!(indices.len(), 8 + combo::COMBO_RECORDS);
        assert_eq!(indices[8], COMBO_RECORD_BASE);
    }

    // ---- unlock edges ------------------------------------------------------

    #[test]
    fn expire_while_locked_reports_nothing() {
        // Arrange
        let mut s = Corne::new(&corne::KEYMAP);

        // Act
        let locked = s.expire(UNLOCK_IDLE_MS * 2);

        // Assert
        assert!(!locked);
    }

    #[test]
    fn pressing_unlock_again_restarts_the_time() {
        // Arrange
        let mut s = unlocked();
        s.unlock(200_000);

        // Act
        let open = s.unlocked(1000 + UNLOCK_IDLE_MS + 1000);

        // Assert
        assert!(open);
        assert_eq!(s.remaining_ms(200_000), UNLOCK_IDLE_MS);
    }

    #[test]
    fn unlock_after_lock_opens_again() {
        // Arrange
        let mut s = unlocked();
        s.lock();

        // Act
        s.unlock(5000);

        // Assert
        assert!(s.unlocked(5000));
    }

    #[test]
    fn reads_and_checks_work_locked() {
        // Arrange
        let s = Corne::new(&corne::KEYMAP);

        // Act
        let got = (s.binding(0, 0), s.check(0, 1, kp(Z)));

        // Assert
        assert_eq!(got, (corne::LAYERS[0].first().copied(), Ok(())));
    }

    #[test]
    fn write_after_the_timeout_locks_for_good() {
        // Arrange: timed out, then a refused write
        let mut s = unlocked();
        let _ = s.save(1000 + UNLOCK_IDLE_MS);

        // Act: the clock wraps far around
        let open = s.unlocked(1000);

        // Assert
        assert!(!open);
    }

    #[test]
    fn save_extends_the_unlock() {
        // Arrange
        let mut s = unlocked();
        s.save(200_000).unwrap();

        // Act
        let left = s.remaining_ms(200_000);

        // Assert
        assert_eq!(left, UNLOCK_IDLE_MS);
    }

    #[test]
    fn check_does_not_extend_the_unlock() {
        // Arrange
        let s = unlocked();
        s.check(0, 1, kp(Z)).unwrap();

        // Act
        let left = s.remaining_ms(1000 + 60_000);

        // Assert
        assert_eq!(left, UNLOCK_IDLE_MS - 60_000);
    }
}

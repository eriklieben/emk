//! Combos in Studio: the set as Studio keeps it ([`ComboSet`]: what the
//! engine uses plus a name each), how one is written as text in the protocol
//! ([`write_spec`], [`parse_spec`]) and how the set is stored in flash
//! ([`encode_record`], [`ComboLoad`]).
//!
//! Text, six words:
//!
//! ```text
//! <name> <positions> <layers> <timeout> <idle> <action>
//! copy   26,27       0        50        150    kp:LC(C)
//! -      25,26       *        50        0      sys:BtClear
//! ```
//!
//! `name`: up to [`NAME_MAX`] of `A-Z a-z 0-9 _`, `-` for none. `positions`:
//! 2 to [`MAX_KEYS`] distinct positions, comma separated (written ascending).
//! `layers`: comma separated, or `*` for all. `timeout`: ms within which all
//! keys must be down (1..=65535). `idle`: ZMK's `require-prior-idle-ms`
//! (0..=65535, 0 = off). `action`: as `encoding`.
//!
//! Records (map keys 0x70 + i, at most [`COMBO_RECORDS`]): the whole set,
//! split over as many records as needed, each at most `record::RECORD_MAX`
//! bytes:
//!
//! ```text
//! byte 0  COMBO_FORMAT_VERSION
//! byte 1  record index i
//! byte 2  set id: the same in every record of one save (a save cut short
//!         leaves records of two ids, and the set is not used)
//! byte 3  combos in the whole set; 0xFF in record 0 = "as compiled"
//! byte 4  combos in this record
//! then per combo: key count, positions..., action (record::write_action),
//!         layers (bitmap), timeout (u16 LE), idle (u16 LE), name length, name
//! ```

use core::fmt::{self, Write};

use super::Error;
use super::encoding::{self, MAX_ENCODING};
use super::record::{self, ACTION_MAX, RECORD_MAX};
use crate::action::Action;
use crate::engine::{ComboDef, Keymap, MAX_COMBOS};

/// Longest combo name.
pub const NAME_MAX: usize = 16;

/// Most keys in a combo Studio makes (the compiled ones may have more).
pub const MAX_KEYS: usize = 4;

/// Version of the combo record layout.
pub const COMBO_FORMAT_VERSION: u8 = 1;

/// Most combo records.
pub const COMBO_RECORDS: usize = 8;

/// `total` in record 0 for "the compiled combos".
const COMPILED: u8 = 0xFF;

const HEADER: usize = 5;

/// Bytes of the largest combo in a record.
pub const ENTRY_MAX: usize = 1 + MAX_KEYS + ACTION_MAX + 1 + 2 + 2 + 1 + NAME_MAX;

// Every record holds at least one combo, and all of them fit.
const _: () = assert!(HEADER + ENTRY_MAX <= RECORD_MAX);
const _: () = assert!(MAX_COMBOS.div_ceil((RECORD_MAX - HEADER) / ENTRY_MAX) <= COMBO_RECORDS);

/// A combo's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Name {
    bytes: [u8; NAME_MAX],
    len: u8,
}

const fn name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Name {
    pub const EMPTY: Self = Self {
        bytes: [0; NAME_MAX],
        len: 0,
    };

    /// `text` as a name, if it is one (empty is allowed).
    #[must_use]
    pub const fn new(text: &str) -> Option<Self> {
        let b = text.as_bytes();
        if b.len() > NAME_MAX {
            return None;
        }
        let mut name = Self::EMPTY;
        let mut i = 0;
        while i < b.len() {
            if !name_byte(b[i]) {
                return None;
            }
            name.bytes[i] = b[i];
            i += 1;
        }
        name.len = b.len() as u8;
        Some(name)
    }

    /// A compiled name: cut to [`NAME_MAX`], other characters as `_`.
    #[must_use]
    pub const fn lossy(text: &str) -> Self {
        let b = text.as_bytes();
        let mut name = Self::EMPTY;
        let mut i = 0;
        while i < b.len() && i < NAME_MAX {
            name.bytes[i] = if name_byte(b[i]) { b[i] } else { b'_' };
            i += 1;
        }
        name.len = i as u8;
        name
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// One combo: what it does and its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComboSpec {
    pub name: Name,
    pub def: ComboDef,
}

/// The combos, at most [`MAX_COMBOS`].
#[derive(Clone, Copy, Debug)]
pub struct ComboSet {
    defs: [ComboDef; MAX_COMBOS],
    names: [Name; MAX_COMBOS],
    len: u8,
}

impl PartialEq for ComboSet {
    fn eq(&self, other: &Self) -> bool {
        self.defs() == other.defs() && self.names() == other.names()
    }
}

impl Eq for ComboSet {}

impl ComboSet {
    pub const EMPTY: Self = Self {
        defs: [ComboDef::NONE; MAX_COMBOS],
        names: [Name::EMPTY; MAX_COMBOS],
        len: 0,
    };

    /// The compiled combos of `keymap` (at most [`MAX_COMBOS`]).
    #[must_use]
    pub const fn compiled<const N: usize>(keymap: &Keymap<'_, N>) -> Self {
        let mut set = Self::EMPTY;
        let mut i = 0;
        while i < keymap.combos.len() && i < MAX_COMBOS {
            set.defs[i] = keymap.combos[i].def();
            set.names[i] = Name::lossy(keymap.combos[i].name);
            i += 1;
        }
        set.len = i as u8;
        set
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// What the engine uses ([`Engine::set_combos`](crate::Engine::set_combos)).
    #[must_use]
    pub fn defs(&self) -> &[ComboDef] {
        &self.defs[..self.len as usize]
    }

    fn names(&self) -> &[Name] {
        &self.names[..self.len as usize]
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<ComboSpec> {
        (index < self.len()).then(|| ComboSpec {
            name: self.names[index],
            def: self.defs[index],
        })
    }

    /// Replace combo `index`; false if there is none.
    pub fn set(&mut self, index: usize, spec: ComboSpec) -> bool {
        if index >= self.len() {
            return false;
        }
        self.defs[index] = spec.def;
        self.names[index] = spec.name;
        true
    }

    /// Add at the end; its index, or `None` when full.
    pub fn push(&mut self, spec: ComboSpec) -> Option<usize> {
        let index = self.len();
        if index >= MAX_COMBOS {
            return None;
        }
        self.len += 1;
        self.set(index, spec);
        Some(index)
    }

    /// Remove combo `index` (the later ones move down); false if there is none.
    pub fn remove(&mut self, index: usize) -> bool {
        let len = self.len();
        if index >= len {
            return false;
        }
        self.defs.copy_within(index + 1..len, index);
        self.names.copy_within(index + 1..len, index);
        self.defs[len - 1] = ComboDef::NONE;
        self.names[len - 1] = Name::EMPTY;
        self.len -= 1;
        true
    }
}

/// Check a combo for a keymap of `keys` positions and `layers` layers.
///
/// # Errors
///
/// [`Error::Positions`], [`Error::Timing`], [`Error::Action`] or
/// [`Error::TooLong`] for the first part of the combo that doesn't fit.
pub fn validate(spec: &ComboSpec, keys: usize, layers: usize) -> Result<(), Error> {
    let d = &spec.def;
    let count = d.keys.count_ones() as usize;
    let positions = if keys >= 64 {
        u64::MAX
    } else {
        (1u64 << keys) - 1
    };
    if !(2..=MAX_KEYS).contains(&count) || d.keys & !positions != 0 {
        return Err(Error::Positions);
    }
    let all_layers = if layers >= 32 {
        u32::MAX
    } else {
        (1u32 << layers) - 1
    };
    if d.layers & !all_layers != 0 {
        return Err(Error::Range);
    }
    if d.timeout_ms == 0
        || d.timeout_ms > u32::from(u16::MAX)
        || d.require_prior_idle_ms > u32::from(u16::MAX)
    {
        return Err(Error::Timing);
    }
    if matches!(d.action, Action::MomentaryLayer(l) if l as usize >= layers) {
        return Err(Error::Action);
    }
    if encoding::encoded(d.action).as_str().len() > MAX_ENCODING {
        return Err(Error::TooLong);
    }
    Ok(())
}

// ---- text ------------------------------------------------------------------

/// Write `spec` as its six words.
///
/// # Errors
///
/// Whatever `out` returns when it can't take more text.
pub fn write_spec<W: Write>(spec: &ComboSpec, out: &mut W) -> fmt::Result {
    let d = &spec.def;
    out.write_str(if spec.name.is_empty() {
        "-"
    } else {
        spec.name.as_str()
    })?;
    out.write_char(' ')?;
    write_bits(d.keys, out)?;
    out.write_char(' ')?;
    if d.layers == 0 {
        out.write_char('*')?;
    } else {
        write_bits(u64::from(d.layers), out)?;
    }
    write!(out, " {} {} ", d.timeout_ms, d.require_prior_idle_ms)?;
    encoding::encode(d.action, out)
}

/// `0,3,7` for bits 0, 3 and 7.
fn write_bits<W: Write>(bits: u64, out: &mut W) -> fmt::Result {
    let mut first = true;
    for i in (0..64).filter(|i| bits & (1 << i) != 0) {
        if !first {
            out.write_char(',')?;
        }
        first = false;
        write!(out, "{i}")?;
    }
    Ok(())
}

fn number(text: &str, max: u32) -> Option<u32> {
    if text.is_empty() || text.len() > 5 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|&n| n <= max)
}

/// Read the six words of a combo from `words` (exactly six).
///
/// # Errors
///
/// [`Error::Syntax`] for the wrong number of words, otherwise the error of
/// the first word that doesn't read.
pub fn parse_spec<'t>(mut words: impl Iterator<Item = &'t str>) -> Result<ComboSpec, Error> {
    let (Some(name), Some(positions), Some(layers), Some(timeout), Some(idle), Some(action), None) = (
        words.next(),
        words.next(),
        words.next(),
        words.next(),
        words.next(),
        words.next(),
        words.next(),
    ) else {
        return Err(Error::Syntax);
    };
    let name = if name == "-" {
        Name::EMPTY
    } else {
        Name::new(name)
            .filter(|n| !n.is_empty())
            .ok_or(Error::Name)?
    };
    let mut keys = 0u64;
    let mut count = 0;
    for p in positions.split(',') {
        let p = number(p, 63).ok_or(Error::Positions)?;
        if keys & (1 << p) != 0 {
            return Err(Error::Positions);
        }
        keys |= 1 << p;
        count += 1;
    }
    if count > MAX_KEYS {
        return Err(Error::Positions);
    }
    let layers = if layers == "*" {
        0
    } else {
        let mut mask = 0u32;
        for l in layers.split(',') {
            mask |= 1 << number(l, 31).ok_or(Error::Range)?;
        }
        mask
    };
    let timeout_ms = number(timeout, u32::from(u16::MAX)).ok_or(Error::Timing)?;
    let require_prior_idle_ms = number(idle, u32::from(u16::MAX)).ok_or(Error::Timing)?;
    let action = encoding::decode(action).map_err(|_| Error::Action)?;
    Ok(ComboSpec {
        name,
        def: ComboDef {
            keys,
            action,
            layers,
            timeout_ms,
            require_prior_idle_ms,
        },
    })
}

// ---- records -----------------------------------------------------------------

/// Bytes `spec` takes in a record.
fn entry_len(spec: &ComboSpec) -> usize {
    let mut action = [0; ACTION_MAX];
    1 + spec.def.keys.count_ones() as usize
        + record::write_action(spec.def.action, &mut action)
        + 1
        + 2
        + 2
        + 1
        + spec.name.len as usize
}

/// Which combos go in which record: `(first, count)` per record, and how
/// many records. The compiled set (`None`) is one record: the marker.
#[must_use]
pub fn plan(set: Option<&ComboSet>) -> ([(u8, u8); COMBO_RECORDS], usize) {
    let mut out = [(0u8, 0u8); COMBO_RECORDS];
    let Some(set) = set else {
        return (out, 1);
    };
    let mut records = 1;
    let mut used = HEADER;
    for i in 0..set.len() {
        let len = entry_len(&set.get(i).unwrap_or(ComboSpec {
            name: Name::EMPTY,
            def: ComboDef::NONE,
        }));
        if used + len > RECORD_MAX {
            records += 1;
            used = HEADER;
            out[records - 1].0 = i as u8;
        }
        used += len;
        out[records - 1].1 += 1;
    }
    (out, records)
}

/// Record `index` of `set` (`None`: the compiled combos), saved as `set_id`.
/// Returns its length.
pub fn encode_record(
    set: Option<&ComboSet>,
    index: usize,
    set_id: u8,
    out: &mut [u8; RECORD_MAX],
) -> usize {
    let (plan, _) = plan(set);
    out[0] = COMBO_FORMAT_VERSION;
    out[1] = index as u8;
    out[2] = set_id;
    let Some(set) = set else {
        out[3] = COMPILED;
        out[4] = 0;
        return HEADER;
    };
    out[3] = set.len() as u8;
    let (first, count) = plan.get(index).copied().unwrap_or((0, 0));
    out[4] = count;
    let mut at = HEADER;
    for i in first as usize..(first + count) as usize {
        let Some(spec) = set.get(i) else { break };
        let d = &spec.def;
        out[at] = d.keys.count_ones() as u8;
        at += 1;
        for p in (0..64u8).filter(|p| d.keys & (1 << p) != 0) {
            out[at] = p;
            at += 1;
        }
        let mut action = [0; ACTION_MAX];
        let n = record::write_action(d.action, &mut action);
        out[at..at + n].copy_from_slice(&action[..n]);
        at += n;
        out[at] = d.layers as u8;
        out[at + 1..at + 3].copy_from_slice(&(d.timeout_ms as u16).to_le_bytes());
        out[at + 3..at + 5].copy_from_slice(&(d.require_prior_idle_ms as u16).to_le_bytes());
        out[at + 5] = spec.name.len;
        at += 6;
        let name = &spec.name.bytes[..spec.name.len as usize];
        out[at..at + name.len()].copy_from_slice(name);
        at += name.len();
    }
    at
}

/// Why combo records were not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComboRecordError {
    /// Another format version.
    Version(u8),
    /// Cut short, bytes left over, or a field out of range.
    Corrupt,
    /// Records of different saves, missing or out of order.
    Mixed,
}

/// Combo records being read at start, in index order, straight into the
/// set they make up (so no second set is kept for it).
#[derive(Clone, Copy, Debug)]
pub struct ComboLoad {
    /// Set id and total from record 0, once read.
    header: Option<(u8, u8)>,
    next: u8,
    failed: Option<ComboRecordError>,
}

impl Default for ComboLoad {
    fn default() -> Self {
        Self::new()
    }
}

impl ComboLoad {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            header: None,
            next: 0,
            failed: None,
        }
    }

    /// Take record `index` into `set` (records must come in index order;
    /// ones after the set is complete are leftovers of a larger set and
    /// ignored). `set` is emptied by record 0 of a stored set.
    pub fn record(&mut self, set: &mut ComboSet, index: usize, bytes: &[u8]) {
        if self.failed.is_some() {
            return;
        }
        if let Some((_, total)) = self.header
            && (total == COMPILED || set.len() >= total as usize)
        {
            return; // complete: a leftover
        }
        if let Err(e) = self.take(set, index, bytes) {
            self.failed = Some(e);
        }
    }

    /// No more records: `Ok(None)` for the compiled combos (no records, or
    /// the marker), `Ok(Some(set id))` when `set` holds the stored set.
    ///
    /// # Errors
    ///
    /// The first problem the records had; the compiled combos apply then.
    pub fn finish(&self, set: &ComboSet) -> Result<Option<u8>, ComboRecordError> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        match self.header {
            None | Some((_, COMPILED)) => Ok(None),
            Some((id, total)) if set.len() == total as usize => Ok(Some(id)),
            Some(_) => Err(ComboRecordError::Mixed),
        }
    }

    fn take(
        &mut self,
        set: &mut ComboSet,
        index: usize,
        bytes: &[u8],
    ) -> Result<(), ComboRecordError> {
        let byte = |i: usize| bytes.get(i).copied().ok_or(ComboRecordError::Corrupt);
        let version = byte(0)?;
        if version != COMBO_FORMAT_VERSION {
            return Err(ComboRecordError::Version(version));
        }
        let (rec_index, set_id, total, count) = (byte(1)?, byte(2)?, byte(3)?, byte(4)?);
        if rec_index as usize != index || index != self.next as usize {
            return Err(ComboRecordError::Mixed);
        }
        match self.header {
            None => {
                if total != COMPILED && total as usize > MAX_COMBOS {
                    return Err(ComboRecordError::Corrupt);
                }
                self.header = Some((set_id, total));
                if total != COMPILED {
                    *set = ComboSet::EMPTY;
                }
            }
            Some(h) if h != (set_id, total) => return Err(ComboRecordError::Mixed),
            Some(_) => {}
        }
        self.next += 1;
        if total == COMPILED {
            return if count == 0 && bytes.len() == HEADER {
                Ok(())
            } else {
                Err(ComboRecordError::Corrupt)
            };
        }
        let mut at = HEADER;
        for _ in 0..count {
            let (spec, next) = read_combo(bytes, at)?;
            at = next;
            set.push(spec).ok_or(ComboRecordError::Corrupt)?;
        }
        if at != bytes.len() || set.len() > total as usize {
            return Err(ComboRecordError::Corrupt);
        }
        Ok(())
    }
}

/// The combo stored in `bytes` from `at` on (key count, keys, action,
/// layers, timeout, prior idle, name), and where the next one starts.
fn read_combo(bytes: &[u8], mut at: usize) -> Result<(ComboSpec, usize), ComboRecordError> {
    let byte = |i: usize| bytes.get(i).copied().ok_or(ComboRecordError::Corrupt);
    let keys = byte(at)? as usize;
    at += 1;
    if keys == 0 || keys > 8 {
        return Err(ComboRecordError::Corrupt);
    }
    let mut mask = 0u64;
    for _ in 0..keys {
        let p = byte(at)?;
        if p >= 64 {
            return Err(ComboRecordError::Corrupt);
        }
        mask |= 1 << p;
        at += 1;
    }
    let (action, n) = record::read_action(&bytes[at.min(bytes.len())..])
        .map_err(|_| ComboRecordError::Corrupt)?;
    at += n;
    let layers = u32::from(byte(at)?);
    let timeout = u32::from(u16::from_le_bytes([byte(at + 1)?, byte(at + 2)?]));
    let idle = u32::from(u16::from_le_bytes([byte(at + 3)?, byte(at + 4)?]));
    let name_len = byte(at + 5)? as usize;
    at += 6;
    let name = bytes
        .get(at..at + name_len)
        .ok_or(ComboRecordError::Corrupt)?;
    at += name_len;
    let name = core::str::from_utf8(name)
        .ok()
        .and_then(Name::new)
        .ok_or(ComboRecordError::Corrupt)?;
    let spec = ComboSpec {
        name,
        def: ComboDef {
            keys: mask,
            action,
            layers,
            timeout_ms: timeout,
            require_prior_idle_ms: idle,
        },
    };
    Ok((spec, at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SystemAction;
    use crate::action::{kp, mo, mt, sys};
    use crate::keymaps::corne;
    use crate::keys::{A, B, C, LCTRL, PAUSE_BREAK, lc, ls};

    fn spec(text: &str) -> Result<ComboSpec, Error> {
        parse_spec(text.split(' '))
    }

    fn text(spec: &ComboSpec) -> String {
        let mut out = String::new();
        write_spec(spec, &mut out).unwrap();
        out
    }

    fn records(set: Option<&ComboSet>, id: u8) -> Vec<Vec<u8>> {
        let (_, n) = plan(set);
        (0..n)
            .map(|i| {
                let mut out = [0; RECORD_MAX];
                let len = encode_record(set, i, id, &mut out);
                out[..len].to_vec()
            })
            .collect()
    }

    /// What the records load as: `Ok(None)` compiled, `Ok(Some((set, id)))`.
    fn load(records: &[Vec<u8>]) -> Result<Option<(ComboSet, u8)>, ComboRecordError> {
        let mut l = ComboLoad::new();
        let mut set = ComboSet::compiled(&corne::KEYMAP);
        for (i, r) in records.iter().enumerate() {
            l.record(&mut set, i, r);
        }
        l.finish(&set).map(|id| id.map(|id| (set, id)))
    }

    /// 16 combos with long names and mod-tap actions: the most bytes.
    fn biggest_set() -> ComboSet {
        let mut set = ComboSet::EMPTY;
        for i in 0..MAX_COMBOS {
            let long = crate::Key {
                code: PAUSE_BREAK.code,
                mods: 0xFF,
            };
            set.push(ComboSpec {
                name: Name::new("abcdefghijklmnop").unwrap(),
                def: ComboDef {
                    keys: 0b1111 << i,
                    action: mt(long, long),
                    layers: 0xFF,
                    timeout_ms: 65535,
                    require_prior_idle_ms: 65535,
                },
            })
            .unwrap();
        }
        set
    }

    #[test]
    fn combo_text_has_six_words() {
        // Arrange
        let combo = spec("copy 27,26 0 50 150 kp:LC(C)").unwrap();

        // Act
        let got = text(&combo);

        // Assert: positions ascending
        assert_eq!(got, "copy 26,27 0 50 150 kp:LC(C)");
    }

    #[test]
    fn combo_text_round_trips_for_every_compiled_combo() {
        // Arrange
        let set = ComboSet::compiled(&corne::KEYMAP);

        // Act
        let back: Vec<_> = (0..set.len())
            .map(|i| spec(&text(&set.get(i).unwrap())))
            .collect();

        // Assert
        let expected: Vec<_> = (0..set.len()).map(|i| Ok(set.get(i).unwrap())).collect();
        assert_eq!(back, expected);
    }

    #[test]
    fn unnamed_combo_on_all_layers() {
        // Arrange
        let input = "- 1,2 * 40 0 caps";

        // Act
        let got = spec(input).unwrap();

        // Assert
        assert_eq!(got.name, Name::EMPTY);
        assert_eq!(got.def.layers, 0);
        assert_eq!(text(&got), input);
    }

    #[test]
    fn several_layers_are_a_bitmap() {
        // Arrange
        let input = "x 1,2 0,2 40 0 none";

        // Act
        let got = spec(input).unwrap();

        // Assert
        assert_eq!(got.def.layers, 0b101);
    }

    #[test]
    fn compiled_reset_bluetooth_keeps_its_semantics() {
        // Arrange
        let set = ComboSet::compiled(&corne::KEYMAP);

        // Act
        let reset = set.get(5).unwrap();

        // Assert: nav only, no prior idle, BT clear on 25+26
        assert_eq!(text(&reset), "reset_bluetooth 25,26 2 50 0 sys:BtClear");
    }

    #[test]
    fn repeated_position_is_refused() {
        // Arrange / Act
        let got = spec("x 1,1 * 50 0 none");

        // Assert
        assert_eq!(got, Err(Error::Positions));
    }

    #[test]
    fn five_positions_are_refused() {
        // Arrange / Act
        let got = spec("x 1,2,3,4,5 * 50 0 none");

        // Assert
        assert_eq!(got, Err(Error::Positions));
    }

    #[test]
    fn bad_position_text_is_refused() {
        // Arrange
        let inputs = [
            "x 1,a * 50 0 none",
            "x 1, * 50 0 none",
            "x 64,1 * 50 0 none",
        ];

        // Act
        let got: Vec<_> = inputs.iter().map(|i| spec(i)).collect();

        // Assert
        assert!(got.iter().all(|r| *r == Err(Error::Positions)), "{got:?}");
    }

    #[test]
    fn bad_names_are_refused() {
        // Arrange
        let inputs = [
            "a-b 1,2 * 50 0 none",
            "abcdefghijklmnopq 1,2 * 50 0 none",
            "é 1,2 * 50 0 none",
        ];

        // Act
        let got: Vec<_> = inputs.iter().map(|i| spec(i)).collect();

        // Assert
        assert!(got.iter().all(|r| *r == Err(Error::Name)), "{got:?}");
    }

    #[test]
    fn bad_layers_are_refused() {
        // Arrange / Act
        let got = spec("x 1,2 0,x 50 0 none");

        // Assert
        assert_eq!(got, Err(Error::Range));
    }

    #[test]
    fn bad_timing_is_refused() {
        // Arrange
        let inputs = [
            "x 1,2 * 65536 0 none",
            "x 1,2 * 50 -1 none",
            "x 1,2 * fast 0 none",
        ];

        // Act
        let got: Vec<_> = inputs.iter().map(|i| spec(i)).collect();

        // Assert
        assert!(got.iter().all(|r| *r == Err(Error::Timing)), "{got:?}");
    }

    #[test]
    fn bad_action_is_refused() {
        // Arrange / Act
        let got = spec("x 1,2 * 50 0 kp:NOPE");

        // Assert
        assert_eq!(got, Err(Error::Action));
    }

    #[test]
    fn wrong_word_count_is_syntax() {
        // Arrange
        let inputs = ["x 1,2 * 50 0", "x 1,2 * 50 0 none extra"];

        // Act
        let got: Vec<_> = inputs.iter().map(|i| spec(i)).collect();

        // Assert
        assert!(got.iter().all(|r| *r == Err(Error::Syntax)), "{got:?}");
    }

    #[test]
    fn validate_accepts_a_plain_combo() {
        // Arrange
        let combo = spec("x 1,2 0 50 0 kp:A").unwrap();

        // Act
        let got = validate(&combo, 42, 4);

        // Assert
        assert_eq!(got, Ok(()));
    }

    #[test]
    fn validate_needs_two_keys() {
        // Arrange
        let combo = spec("x 1 0 50 0 kp:A").unwrap();

        // Act
        let got = validate(&combo, 42, 4);

        // Assert
        assert_eq!(got, Err(Error::Positions));
    }

    #[test]
    fn validate_refuses_positions_past_the_keyboard() {
        // Arrange
        let combo = spec("x 1,42 0 50 0 kp:A").unwrap();

        // Act
        let got = validate(&combo, 42, 4);

        // Assert
        assert_eq!(got, Err(Error::Positions));
    }

    #[test]
    fn validate_refuses_missing_layers() {
        // Arrange
        let combo = spec("x 1,2 4 50 0 kp:A").unwrap();

        // Act
        let got = validate(&combo, 42, 4);

        // Assert
        assert_eq!(got, Err(Error::Range));
    }

    #[test]
    fn validate_refuses_zero_timeout() {
        // Arrange
        let combo = spec("x 1,2 0 0 0 kp:A").unwrap();

        // Act
        let got = validate(&combo, 42, 4);

        // Assert
        assert_eq!(got, Err(Error::Timing));
    }

    #[test]
    fn validate_refuses_mo_to_a_missing_layer() {
        // Arrange
        let combo = spec("x 1,2 0 50 0 mo:4").unwrap();

        // Act
        let got = validate(&combo, 42, 4);

        // Assert
        assert_eq!(got, Err(Error::Action));
    }

    #[test]
    fn validate_refuses_too_long_an_action() {
        // Arrange
        let mut combo = spec("x 1,2 0 50 0 none").unwrap();
        let long = crate::Key {
            code: PAUSE_BREAK.code,
            mods: 0xFF,
        };
        combo.def.action = kp(long);

        // Act
        let got = validate(&combo, 42, 4);

        // Assert
        assert_eq!(got, Err(Error::TooLong));
    }

    #[test]
    fn set_push_and_remove_keep_the_order() {
        // Arrange
        let mut set = ComboSet::EMPTY;
        for name in ["a", "b", "c"] {
            set.push(ComboSpec {
                name: Name::new(name).unwrap(),
                def: ComboDef::NONE,
            });
        }

        // Act
        set.remove(1);

        // Assert
        let names: Vec<_> = (0..set.len())
            .map(|i| set.get(i).unwrap().name.as_str().to_string())
            .collect();
        assert_eq!(names, ["a", "c"]);
    }

    #[test]
    fn full_set_takes_no_more() {
        // Arrange
        let mut set = biggest_set();

        // Act
        let got = set.push(ComboSpec {
            name: Name::EMPTY,
            def: ComboDef::NONE,
        });

        // Assert
        assert_eq!(got, None);
    }

    #[test]
    fn missing_index_is_refused() {
        // Arrange
        let mut set = ComboSet::EMPTY;
        let spec = ComboSpec {
            name: Name::EMPTY,
            def: ComboDef::NONE,
        };

        // Act
        let got = (set.remove(0), set.set(0, spec), set.get(0));

        // Assert
        assert_eq!(got, (false, false, None));
    }

    #[test]
    fn removed_slots_do_not_count_for_equality() {
        // Arrange
        let a = ComboSet::compiled(&corne::KEYMAP);
        let mut b = a;
        b.push(a.get(0).unwrap());

        // Act
        b.remove(6);

        // Assert
        assert_eq!(a, b);
    }

    #[test]
    fn compiled_set_is_stored_as_the_marker() {
        // Arrange / Act
        let got = records(None, 3);

        // Assert
        assert_eq!(got, [vec![COMBO_FORMAT_VERSION, 0, 3, COMPILED, 0]]);
    }

    #[test]
    fn compiled_combos_round_trip_through_records() {
        // Arrange
        let set = ComboSet::compiled(&corne::KEYMAP);
        let recs = records(Some(&set), 7);

        // Act
        let got = load(&recs);

        // Assert
        assert_eq!(got, Ok(Some((set, 7))));
    }

    #[test]
    fn biggest_set_fits_in_the_records() {
        // Arrange
        let set = biggest_set();

        // Act
        let recs = records(Some(&set), 1);

        // Assert
        assert!(recs.len() <= COMBO_RECORDS);
        assert!(recs.iter().all(|r| r.len() <= RECORD_MAX));
        assert_eq!(load(&recs), Ok(Some((set, 1))));
    }

    #[test]
    fn empty_set_is_not_the_compiled_one() {
        // Arrange
        let recs = records(Some(&ComboSet::EMPTY), 2);

        // Act
        let got = load(&recs);

        // Assert
        assert_eq!(got, Ok(Some((ComboSet::EMPTY, 2))));
    }

    #[test]
    fn no_records_mean_compiled() {
        // Arrange / Act
        let got = load(&[]);

        // Assert
        assert_eq!(got, Ok(None));
    }

    #[test]
    fn marker_record_means_compiled() {
        // Arrange
        let recs = records(None, 0);

        // Act
        let got = load(&recs);

        // Assert
        assert_eq!(got, Ok(None));
    }

    #[test]
    fn leftover_records_of_a_bigger_set_are_ignored() {
        // Arrange: a big set saved, then a small one over its first record
        let mut recs = records(Some(&biggest_set()), 1);
        let small = ComboSet::compiled(&corne::KEYMAP);
        let fresh = records(Some(&small), 2);
        assert!(fresh.len() < recs.len());
        recs[..fresh.len()].clone_from_slice(&fresh);

        // Act
        let got = load(&recs);

        // Assert
        assert_eq!(got, Ok(Some((small, 2))));
    }

    #[test]
    fn records_of_two_saves_are_refused() {
        // Arrange: a save cut short after its first record
        let mut recs = records(Some(&biggest_set()), 1);
        recs[0] = records(Some(&biggest_set()), 2).remove(0);

        // Act
        let got = load(&recs);

        // Assert
        assert_eq!(got, Err(ComboRecordError::Mixed));
    }

    #[test]
    fn missing_record_is_refused() {
        // Arrange
        let mut recs = records(Some(&biggest_set()), 1);
        recs.truncate(2);

        // Act
        let got = load(&recs);

        // Assert
        assert_eq!(got, Err(ComboRecordError::Mixed));
    }

    #[test]
    fn record_of_another_version_is_refused() {
        // Arrange
        let mut recs = records(Some(&ComboSet::compiled(&corne::KEYMAP)), 1);
        recs[0][0] = COMBO_FORMAT_VERSION + 1;

        // Act
        let got = load(&recs);

        // Assert
        assert_eq!(
            got,
            Err(ComboRecordError::Version(COMBO_FORMAT_VERSION + 1))
        );
    }

    #[test]
    fn truncated_record_is_refused() {
        // Arrange
        let mut recs = records(Some(&ComboSet::compiled(&corne::KEYMAP)), 1);
        recs[0].pop();

        // Act
        let got = load(&recs);

        // Assert
        assert_eq!(got, Err(ComboRecordError::Corrupt));
    }

    #[test]
    fn record_with_bytes_left_over_is_refused() {
        // Arrange
        let mut recs = records(Some(&ComboSet::compiled(&corne::KEYMAP)), 1);
        recs[0].push(0);

        // Act
        let got = load(&recs);

        // Assert
        assert_eq!(got, Err(ComboRecordError::Corrupt));
    }

    #[test]
    fn record_with_a_bad_action_is_refused() {
        // Arrange: one combo, its action byte made unknown
        let mut set = ComboSet::EMPTY;
        set.push(spec("x 1,2 * 50 0 caps").unwrap());
        let mut recs = records(Some(&set), 1);
        recs[0][HEADER + 3] = 0x77;

        // Act
        let got = load(&recs);

        // Assert
        assert_eq!(got, Err(ComboRecordError::Corrupt));
    }

    #[test]
    fn record_out_of_order_is_refused() {
        // Arrange
        let recs = records(Some(&biggest_set()), 1);
        let mut l = ComboLoad::new();
        let mut set = ComboSet::EMPTY;
        l.record(&mut set, 0, &recs[0]);

        // Act
        l.record(&mut set, 2, &recs[2]);

        // Assert
        assert_eq!(l.finish(&set), Err(ComboRecordError::Mixed));
    }

    #[test]
    fn every_action_kind_survives_a_record() {
        // Arrange
        let actions = [
            kp(lc(C)),
            mo(2),
            mt(LCTRL, A),
            sys(SystemAction::StudioUnlock),
            crate::action::CAPS_WORD,
            crate::action::NONE,
            crate::action::TRANS,
            kp(ls(B)),
        ];
        let mut set = ComboSet::EMPTY;
        for (i, &a) in actions.iter().enumerate() {
            set.push(ComboSpec {
                name: Name::EMPTY,
                def: ComboDef {
                    keys: 0b11 << i,
                    action: a,
                    layers: 1 << (i % 4),
                    timeout_ms: 50 + i as u32,
                    require_prior_idle_ms: i as u32,
                },
            });
        }

        // Act
        let got = load(&records(Some(&set), 9));

        // Assert
        assert_eq!(got, Ok(Some((set, 9))));
    }
}

//! Studio in the emulator: the same editable keymap, lock and protocol as
//! the firmware (`kb_core::studio`), with the "flash" either in memory
//! (lasts while the emulator runs, across emulated restarts) or in a text
//! file the app names (`kb_emu_studio_set_storage_path`), e.g.
//! `~/.config/emk/emulator-keymap.txt`:
//!
//! ```text
//! # EMK emulator keymap: Studio's edits over the compiled keymap
//! emk-keymap 2
//! 0 12 mt:LCTRL/TAB
//! 2 3 kp:LC(LS(B))
//! combos 2
//! combo copy 26,27 0 50 150 kp:LC(C)
//! combo - 1,2 * 40 0 caps
//! ```
//!
//! One `<layer> <position> <action>` line per binding that differs from
//! the compiled keymap; if the combos differ from the compiled ones, a
//! `combos <count>` line and the whole set, one `combo <name> <positions>
//! <layers> <timeout> <idle> <action>` line each (no `combos` line: the
//! compiled combos). Version 1 files (no combos) are read too; a file of
//! another version is ignored (compiled keymap). Read back through the
//! firmware's own record path, so the lockout guard applies at start
//! exactly as on the keyboard.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kb_core::Action;
use kb_core::debug::Line;
use kb_core::keymaps::corne;
use kb_core::studio::combo::{self, ComboSet, ComboSpec};
use kb_core::studio::protocol::{self, Outbox, Storage, StudioCommand};
use kb_core::studio::record::{self, BLOCK_KEYS, RECORD_MAX};
use kb_core::studio::{COMBO_RECORD_BASE, Error, Studio, encoding};

pub type CorneStudio = Studio<'static, { corne::KEYS }, { corne::LAYER_COUNT }>;

/// First line of the file after comments: format and version.
pub const FILE_MAGIC: &str = "emk-keymap";
pub const FILE_VERSION: u32 = 2;

/// The longest line [`EmuStudio::next_line`] gives.
pub const LINE_MAX: usize = protocol::layer_line_len(corne::KEYS);

/// What reading the file gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Loaded {
    /// Applied (or there is no file yet: compiled keymap).
    Ok = 0,
    /// Unreadable, another version or a bad line: ignored, compiled keymap.
    Ignored = 1,
    /// The edits would lock Studio out: dropped, compiled keymap.
    Dropped = 2,
}

pub struct EmuStudio {
    pub studio: CorneStudio,
    file: Option<PathBuf>,
    outbox: Outbox,
    /// A protocol line taken from the outbox but not yet read by the app.
    line: Option<String>,
}

impl Default for EmuStudio {
    fn default() -> Self {
        Self::new()
    }
}

impl EmuStudio {
    #[must_use]
    pub fn new() -> Self {
        Self {
            studio: Studio::new(&corne::KEYMAP),
            file: None,
            outbox: Outbox::new(Storage::Ram),
            line: None,
        }
    }

    fn storage(&self) -> Storage {
        if self.file.is_some() {
            Storage::Flash
        } else {
            Storage::Ram
        }
    }

    /// Keep the edits in `path` (`None`: in memory), reading what it has
    /// now: like a keyboard starting with that flash, unsaved edits are gone.
    pub fn set_storage_path(&mut self, path: Option<PathBuf>) -> Loaded {
        self.studio = Studio::new(&corne::KEYMAP);
        self.file = path;
        self.outbox = Outbox::new(self.storage());
        self.line = None;
        let Some(path) = &self.file else {
            return Loaded::Ok;
        };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::Ok,
            Err(_) => return Loaded::Ignored,
        };
        let Some((layers, combos)) = parse_file(&text) else {
            return Loaded::Ignored;
        };
        for (layer, (edited, compiled)) in layers.iter().zip(&corne::LAYERS).enumerate() {
            for block in 0..CorneStudio::BLOCKS_PER_LAYER {
                let mut out = [0u8; RECORD_MAX];
                let n = record::encode_block(layer as u8, block as u8, edited, compiled, &mut out);
                if self.studio.load_record(&out[..n]).is_err() {
                    return Loaded::Ignored;
                }
            }
        }
        if let Some(set) = combos {
            let (_, records) = combo::plan(Some(&set));
            for j in 0..records {
                let mut out = [0u8; RECORD_MAX];
                let n = combo::encode_record(Some(&set), j, 1, &mut out);
                let _ = self.studio.load(COMBO_RECORD_BASE + j, &out[..n]);
            }
        }
        let report = self.studio.loaded();
        if !report.ok {
            // Clear the file too, as the firmware clears its records.
            let _ = self.store();
            Loaded::Dropped
        } else if report.combos.is_some() {
            Loaded::Ignored
        } else {
            Loaded::Ok
        }
    }

    /// Write what Studio asks to (the file, or nothing in memory). False
    /// if writing the file failed (Studio stays dirty).
    pub fn store(&mut self) -> bool {
        let pending = self.studio.take_pending();
        if pending == 0 {
            return true;
        }
        let ok = match &self.file {
            Some(path) => {
                write_file(path, self.studio.saved_layers(), self.studio.saved_combos()).is_ok()
            }
            None => true,
        };
        for i in (0..64).filter(|i| pending & (1 << i) != 0) {
            self.studio.stored(i, ok);
        }
        ok
    }

    /// Bind the action written as `text` to `pos` on `layer` (live).
    ///
    /// # Errors
    ///
    /// [`Error::Action`] when `text` is not an action, otherwise as
    /// [`Studio::check`](kb_core::studio::Studio::check) and
    /// [`Error::Locked`].
    pub fn set(&mut self, layer: u8, pos: u8, text: &str, now: u32) -> Result<(), Error> {
        let action = encoding::decode(text).map_err(|_| Error::Action)?;
        self.studio.set(layer, pos, action, now)
    }

    /// Check the action written as `text` for `pos` on `layer`.
    ///
    /// # Errors
    ///
    /// [`Error::Action`] when `text` is not an action, otherwise as
    /// [`Studio::check`](kb_core::studio::Studio::check).
    pub fn check(&self, layer: u8, pos: u8, text: &str) -> Result<(), Error> {
        let action = encoding::decode(text).map_err(|_| Error::Action)?;
        self.studio.check(layer, pos, action)
    }

    /// Add the combo written as `text` (live); its index.
    ///
    /// # Errors
    ///
    /// [`Error::Syntax`] or the first word's error when `text` is not a combo,
    /// otherwise as [`Studio::check_combo`](kb_core::studio::Studio::check_combo)
    /// and [`Error::Locked`].
    pub fn combo_add(&mut self, text: &str, now: u32) -> Result<u8, Error> {
        let spec = parse_combo(text)?;
        self.studio.add_combo(&spec, now)
    }

    /// Replace combo `index` with the one written as `text` (live).
    ///
    /// # Errors
    ///
    /// [`Error::Syntax`] or the first word's error when `text` is not a combo,
    /// otherwise as [`Studio::check_combo`](kb_core::studio::Studio::check_combo)
    /// and [`Error::Locked`].
    pub fn combo_set(&mut self, index: u8, text: &str, now: u32) -> Result<(), Error> {
        let spec = parse_combo(text)?;
        self.studio.set_combo(index, &spec, now)
    }

    /// Check the combo written as `text` as combo `index` (`None`: a new one).
    ///
    /// # Errors
    ///
    /// [`Error::Syntax`] or the first word's error when `text` is not a combo,
    /// otherwise as [`Studio::check_combo`](kb_core::studio::Studio::check_combo).
    pub fn combo_check(&self, index: Option<u8>, text: &str) -> Result<(), Error> {
        let spec = parse_combo(text)?;
        self.studio.check_combo(index, &spec)
    }

    /// Save; `Ok(false)` if the file couldn't be written.
    ///
    /// # Errors
    ///
    /// [`Error::Locked`] without the `StudioUnlock` key.
    pub fn save(&mut self, now: u32) -> Result<bool, Error> {
        self.studio.save(now)?;
        Ok(self.store())
    }

    /// Back to the compiled keymap and combos; `Ok(false)` if the file
    /// couldn't be written.
    ///
    /// # Errors
    ///
    /// [`Error::Locked`] without the `StudioUnlock` key.
    pub fn reset(&mut self, now: u32) -> Result<bool, Error> {
        self.studio.reset(now)?;
        Ok(self.store())
    }

    /// Run one protocol line (`#studio ...`, no line end); false if it
    /// isn't a Studio command (nothing happens then).
    pub fn command(&mut self, line: &[u8], now: u32) -> bool {
        let Some(kb_core::console::Command::Studio(command)) = kb_core::console::parse(line) else {
            return false;
        };
        self.outbox.command(&mut self.studio, command, now);
        if matches!(command, StudioCommand::Save | StudioCommand::Reset) {
            self.store();
        }
        true
    }

    /// The next protocol line (with its `\r\n`), as the keyboard would send it.
    pub fn next_line(&mut self, now: u32) -> Option<&str> {
        if self.line.is_none() {
            let mut line = Line::<LINE_MAX>::new();
            if self.outbox.next_line(&self.studio, now, &mut line) {
                self.line = Some(String::from_utf8_lossy(line.as_bytes()).into_owned());
            }
        }
        self.line.as_deref()
    }

    /// The line from [`next_line`](Self::next_line) was read.
    pub fn line_taken(&mut self) {
        self.line = None;
    }
}

/// A combo's six words (`<name> <positions> <layers> <timeout> <idle> <action>`).
///
/// # Errors
///
/// [`Error::Syntax`] for the wrong number of words, otherwise the error of
/// the first word that doesn't read.
pub fn parse_combo(text: &str) -> Result<ComboSpec, Error> {
    combo::parse_spec(text.split_ascii_whitespace())
}

/// A combo as its six words.
#[must_use]
pub fn combo_text(spec: &ComboSpec) -> String {
    let mut out = String::new();
    let _ = combo::write_spec(spec, &mut out);
    out
}

type Layers = [[Action; corne::KEYS]; corne::LAYER_COUNT];

/// The bindings a file describes (the compiled keymap plus its lines), and
/// its combos (`None`: compiled).
fn parse_file(text: &str) -> Option<(Layers, Option<ComboSet>)> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let mut header = lines.next()?.split_ascii_whitespace();
    if header.next() != Some(FILE_MAGIC) {
        return None;
    }
    let version = header.next().and_then(|v| v.parse::<u32>().ok())?;
    if !(1..=FILE_VERSION).contains(&version) {
        return None;
    }
    let mut layers = corne::LAYERS;
    let mut combos: Option<(usize, ComboSet)> = None;
    for line in lines {
        let mut words = line.split_ascii_whitespace();
        let first = words.next()?;
        if version >= 2 && first == "combos" {
            let count: usize = words.next()?.parse().ok()?;
            if words.next().is_some() || combos.is_some() {
                return None;
            }
            combos = Some((count, ComboSet::EMPTY));
            continue;
        }
        if version >= 2 && first == "combo" {
            let (_, set) = combos.as_mut()?;
            set.push(combo::parse_spec(words).ok()?)?;
            continue;
        }
        let layer: usize = first.parse().ok()?;
        let pos: usize = words.next()?.parse().ok()?;
        let action = encoding::decode(words.next()?).ok()?;
        if words.next().is_some() || layer >= corne::LAYER_COUNT || pos >= corne::KEYS {
            return None;
        }
        layers[layer][pos] = action;
    }
    let combos = match combos {
        Some((count, set)) if set.len() == count => Some(set),
        Some(_) => return None,
        None => None,
    };
    Some((layers, combos))
}

/// The file for `layers` and `combos`: one line per binding that differs
/// from compiled, and the combos if they differ.
#[must_use]
pub fn file_text(layers: &Layers, combos: &ComboSet) -> String {
    let mut text = format!(
        "# EMK emulator keymap: Studio's edits over the compiled keymap\n{FILE_MAGIC} {FILE_VERSION}\n"
    );
    for (l, (layer, compiled)) in layers.iter().zip(&corne::LAYERS).enumerate() {
        for (p, (&action, &original)) in layer.iter().zip(compiled).enumerate() {
            if action != original {
                let _ = writeln!(text, "{l} {p} {}", encoding::encoded(action).as_str());
            }
        }
    }
    if *combos != ComboSet::compiled(&corne::KEYMAP) {
        let _ = writeln!(text, "combos {}", combos.len());
        for i in 0..combos.len() {
            if let Some(spec) = combos.get(i) {
                let _ = writeln!(text, "combo {}", combo_text(&spec));
            }
        }
    }
    text
}

/// Write the file whole: to a temporary name first, then renamed over it.
fn write_file(path: &Path, layers: &Layers, combos: &ComboSet) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, file_text(layers, combos))?;
    std::fs::rename(&tmp, path)
}

// Blocks are what the firmware writes; the file keeps whole layers, so it
// doesn't care, but both must cover every position.
const _: () = assert!(CorneStudio::BLOCKS_PER_LAYER * BLOCK_KEYS >= corne::KEYS);

#[cfg(test)]
mod tests {
    use super::*;
    use kb_core::action::kp;
    use kb_core::keys::Z;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kb-emu-studio-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("keymap.txt")
    }

    fn unlocked() -> EmuStudio {
        let mut s = EmuStudio::new();
        s.studio.unlock(0);
        s
    }

    #[test]
    fn saved_edit_is_written_to_the_file() {
        // Arrange
        let path = temp_file("write");
        let mut s = EmuStudio::new();
        s.set_storage_path(Some(path.clone()));
        s.studio.unlock(0);
        s.set(0, 1, "kp:Z", 0).unwrap();

        // Act
        let ok = s.save(0);

        // Assert
        assert_eq!(ok, Ok(true));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("emk-keymap 2\n0 1 kp:Z\n"), "{text}");
        assert!(!s.studio.dirty());
    }

    #[test]
    fn file_is_read_back_on_start() {
        // Arrange
        let path = temp_file("read");
        let mut first = EmuStudio::new();
        first.set_storage_path(Some(path.clone()));
        first.studio.unlock(0);
        first.set(1, 3, "kp:Z", 0).unwrap();
        first.save(0).unwrap();
        let mut second = EmuStudio::new();

        // Act
        let loaded = second.set_storage_path(Some(path));

        // Assert
        assert_eq!(loaded, Loaded::Ok);
        assert_eq!(second.studio.binding(1, 3), Some(kp(Z)));
        assert!(!second.studio.dirty());
    }

    #[test]
    fn missing_file_means_the_compiled_keymap() {
        // Arrange
        let mut s = EmuStudio::new();

        // Act
        let loaded = s.set_storage_path(Some(temp_file("missing")));

        // Assert
        assert_eq!(loaded, Loaded::Ok);
        assert_eq!(&s.studio.layers()[..], &corne::LAYERS[..]);
    }

    #[test]
    fn file_of_another_version_is_ignored() {
        // Arrange
        let path = temp_file("version");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "emk-keymap 3\n0 1 kp:Z\n").unwrap();
        let mut s = EmuStudio::new();

        // Act
        let loaded = s.set_storage_path(Some(path));

        // Assert
        assert_eq!(loaded, Loaded::Ignored);
        assert_eq!(&s.studio.layers()[..], &corne::LAYERS[..]);
    }

    #[test]
    fn file_that_locks_out_is_dropped_and_cleared() {
        // Arrange: the nav key taken away
        let path = temp_file("lockout");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "emk-keymap 1\n0 41 kp:A\n").unwrap();
        let mut s = EmuStudio::new();

        // Act
        let loaded = s.set_storage_path(Some(path.clone()));

        // Assert
        assert_eq!(loaded, Loaded::Dropped);
        assert_eq!(&s.studio.layers()[..], &corne::LAYERS[..]);
        assert!(!std::fs::read_to_string(&path).unwrap().contains("0 41"));
    }

    #[test]
    fn reset_empties_the_file() {
        // Arrange
        let path = temp_file("reset");
        let mut s = EmuStudio::new();
        s.set_storage_path(Some(path.clone()));
        s.studio.unlock(0);
        s.set(0, 1, "kp:Z", 0).unwrap();
        s.save(0).unwrap();

        // Act
        let ok = s.reset(0);

        // Assert
        assert_eq!(ok, Ok(true));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("emk-keymap 2\n"), "{text}");
    }

    #[test]
    fn protocol_get_answers_like_the_keyboard() {
        // Arrange
        let mut s = EmuStudio::new();
        s.command(b"#studio get", 0);

        // Act
        let mut lines = Vec::new();
        while let Some(line) = s.next_line(0) {
            lines.push(line.to_string());
            s.line_taken();
        }

        // Assert
        assert_eq!(lines.len(), 14);
        assert!(lines[0].starts_with("#V 2 layers=4 keys=42 store=ram "));
        assert!(lines.iter().all(|l| l.ends_with("\r\n")));
    }

    #[test]
    fn protocol_save_writes_the_file() {
        // Arrange
        let path = temp_file("protocol");
        let mut s = EmuStudio::new();
        s.set_storage_path(Some(path.clone()));
        s.studio.unlock(0);
        s.command(b"#studio set 0 1 kp:Z", 0);

        // Act
        s.command(b"#studio save", 0);

        // Assert
        assert!(std::fs::read_to_string(&path).unwrap().contains("0 1 kp:Z"));
    }

    #[test]
    fn line_stays_until_taken() {
        // Arrange
        let mut s = unlocked();
        s.command(b"#studio lock", 0);
        let first = s.next_line(0).map(str::to_string);

        // Act
        let again = s.next_line(0).map(str::to_string);

        // Assert
        assert_eq!(first, again);
        assert_eq!(first.as_deref(), Some("#A lock ok\r\n"));
    }

    #[test]
    fn other_lines_are_not_studio_commands() {
        // Arrange
        let mut s = EmuStudio::new();

        // Act
        let taken = s.command(b"#debug 1", 0);

        // Assert
        assert!(!taken);
        assert_eq!(s.next_line(0), None);
    }

    #[test]
    fn version_1_file_is_still_read() {
        // Arrange
        let path = temp_file("v1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "emk-keymap 1\n0 1 kp:Z\n").unwrap();
        let mut s = EmuStudio::new();

        // Act
        let loaded = s.set_storage_path(Some(path));

        // Assert
        assert_eq!(loaded, Loaded::Ok);
        assert_eq!(s.studio.binding(0, 1), Some(kp(Z)));
        assert_eq!(s.studio.combos().len(), 6);
    }

    #[test]
    fn saved_combos_are_written_to_the_file() {
        // Arrange
        let path = temp_file("combos-write");
        let mut s = EmuStudio::new();
        s.set_storage_path(Some(path.clone()));
        s.studio.unlock(0);
        s.combo_add("x 1,2 * 40 0 caps", 0).unwrap();

        // Act
        s.save(0).unwrap();

        // Assert
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("combos 7\ncombo cut 25,26 0 50 150 kp:LC(X)\n"),
            "{text}"
        );
        assert!(text.ends_with("combo x 1,2 * 40 0 caps\n"), "{text}");
    }

    #[test]
    fn combos_come_back_from_the_file() {
        // Arrange
        let path = temp_file("combos-read");
        let mut first = EmuStudio::new();
        first.set_storage_path(Some(path.clone()));
        first.studio.unlock(0);
        first.studio.remove_combo(0, 0).unwrap();
        first
            .combo_set(0, "copy 26,27 0 70 150 kp:LC(C)", 0)
            .unwrap();
        first.save(0).unwrap();
        let mut second = EmuStudio::new();

        // Act
        let loaded = second.set_storage_path(Some(path));

        // Assert
        assert_eq!(loaded, Loaded::Ok);
        assert_eq!(second.studio.combos(), first.studio.combos());
        assert_eq!(second.studio.combos().get(0).unwrap().def.timeout_ms, 70);
    }

    #[test]
    fn compiled_combos_write_no_combo_lines() {
        // Arrange
        let path = temp_file("combos-none");
        let mut s = EmuStudio::new();
        s.set_storage_path(Some(path.clone()));
        s.studio.unlock(0);
        s.set(0, 1, "kp:Z", 0).unwrap();

        // Act
        s.save(0).unwrap();

        // Assert
        assert!(!std::fs::read_to_string(&path).unwrap().contains("combo"));
    }

    #[test]
    fn combo_count_that_does_not_match_is_ignored() {
        // Arrange
        let path = temp_file("combos-count");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "emk-keymap 2\ncombos 2\ncombo x 1,2 * 40 0 caps\n").unwrap();
        let mut s = EmuStudio::new();

        // Act
        let loaded = s.set_storage_path(Some(path));

        // Assert
        assert_eq!(loaded, Loaded::Ignored);
        assert_eq!(s.studio.combos().len(), 6);
    }

    #[test]
    fn bad_combo_line_is_ignored() {
        // Arrange
        let path = temp_file("combos-bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "emk-keymap 2\ncombos 1\ncombo x 1,1 * 40 0 caps\n").unwrap();
        let mut s = EmuStudio::new();

        // Act
        let loaded = s.set_storage_path(Some(path));

        // Assert
        assert_eq!(loaded, Loaded::Ignored);
    }

    #[test]
    fn combo_with_a_missing_layer_in_the_file_is_ignored() {
        // Arrange
        let path = temp_file("combos-layer");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "emk-keymap 2\ncombos 1\ncombo x 1,2 6 40 0 caps\n").unwrap();
        let mut s = EmuStudio::new();

        // Act
        let loaded = s.set_storage_path(Some(path));

        // Assert
        assert_eq!(loaded, Loaded::Ignored);
        assert_eq!(s.studio.combos().len(), 6);
    }

    #[test]
    fn empty_combo_set_survives_the_file() {
        // Arrange
        let path = temp_file("combos-empty");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "emk-keymap 2\ncombos 0\n").unwrap();
        let mut s = EmuStudio::new();

        // Act
        let loaded = s.set_storage_path(Some(path));

        // Assert
        assert_eq!(loaded, Loaded::Ok);
        assert!(s.studio.combos().is_empty());
    }

    #[test]
    fn unreadable_file_is_ignored() {
        // Arrange: a directory where the file should be
        let path = temp_file("dir");
        std::fs::create_dir_all(&path).unwrap();
        let mut s = EmuStudio::new();

        // Act
        let loaded = s.set_storage_path(Some(path));

        // Assert
        assert_eq!(loaded, Loaded::Ignored);
    }

    #[test]
    fn unwritable_file_leaves_it_dirty() {
        // Arrange: the file's place is taken by a directory
        let path = temp_file("unwritable");
        let mut s = EmuStudio::new();
        s.set_storage_path(Some(path.clone()));
        std::fs::create_dir_all(&path).unwrap();
        s.studio.unlock(0);
        s.set(0, 1, "kp:Z", 0).unwrap();

        // Act
        let saved = s.save(0);

        // Assert
        assert_eq!(saved, Ok(false));
        assert!(s.studio.dirty());
    }

    #[test]
    fn bad_layer_line_is_ignored() {
        // Arrange
        let path = temp_file("badline");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "emk-keymap 2\n9 1 kp:Z\n").unwrap();
        let mut s = EmuStudio::new();

        // Act
        let loaded = s.set_storage_path(Some(path));

        // Assert
        assert_eq!(loaded, Loaded::Ignored);
    }

    #[test]
    fn back_to_memory_drops_the_file() {
        // Arrange
        let path = temp_file("memory");
        let mut s = EmuStudio::new();
        s.set_storage_path(Some(path.clone()));

        // Act
        let loaded = s.set_storage_path(None);

        // Assert
        assert_eq!(loaded, Loaded::Ok);
        s.studio.unlock(0);
        s.set(0, 1, "kp:Z", 0).unwrap();
        s.save(0).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn protocol_combo_add_saves_to_the_file() {
        // Arrange
        let path = temp_file("protocol-combo");
        let mut s = EmuStudio::new();
        s.set_storage_path(Some(path.clone()));
        s.studio.unlock(0);
        s.command(b"#studio combo add x 1,2 * 40 0 caps", 0);

        // Act
        s.command(b"#studio save", 0);

        // Assert
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("combo x 1,2 * 40 0 caps")
        );
    }
}

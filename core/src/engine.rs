//! The keymap engine.
//!
//! Events flow through three stages, the same order ZMK uses:
//!
//! 1. **Combos** buffer presses of keys that may start a combo and either
//!    fire the combo or let the presses through unchanged.
//! 2. **Layer lookup** resolves a position to an [`Action`] at press time and
//!    remembers it, so the release always matches the press.
//! 3. **Behaviors** (key press, layer, mod-tap, caps word, ...) update the
//!    HID state, which is emitted as a [`Report`] whenever it changes.
//!
//! Time is a free-running `u32` millisecond counter supplied by the caller.
//! Call [`Engine::tick`] at [`Engine::next_deadline`] so timeouts fire on time.

use crate::action::{Action, Key, MouseAction, SystemAction};
use crate::keys::{self, MOD_LSFT};
use crate::queue::Queue;
use crate::report::Report;

pub const MAX_COMBO_KEYS: usize = 8;
const MAX_ACTIVE_COMBOS: usize = 4;
const MAX_HELD_HOLD_TAPS: usize = 16;
const OUTPUT_QUEUE: usize = 64;

/// Most layers a keymap may have: the engine keeps its own copy of the
/// bindings in RAM ([`Engine::set_binding`], Studio), this many layers big.
pub const MAX_LAYERS: usize = 8;

/// ZMK `combos { ... }` entry.
#[derive(Clone, Copy, Debug)]
pub struct Combo<'a> {
    pub name: &'a str,
    pub keys: &'a [u8],
    pub action: Action,
    /// Layers on which the combo is active; empty means all layers.
    /// Checked against the highest active layer, like ZMK.
    pub layers: &'a [u8],
    /// All keys must be pressed within this time of the first one (ZMK default 50 ms).
    pub timeout_ms: u32,
    /// ZMK `require-prior-idle-ms`: only a candidate when no key was typed in
    /// this time before the first combo key, so fast typing never triggers it.
    /// 0 = off. Only keycodes count, not layer keys or modifiers (as in ZMK).
    pub require_prior_idle_ms: u32,
}

impl Combo<'_> {
    /// Its keys as a position bitmap.
    #[must_use]
    pub const fn mask(&self) -> u64 {
        let mut mask = 0;
        let mut i = 0;
        while i < self.keys.len() {
            if self.keys[i] < 64 {
                mask |= bit(self.keys[i]);
            }
            i += 1;
        }
        mask
    }

    /// The combo as the engine keeps it in RAM.
    #[must_use]
    pub const fn def(&self) -> ComboDef {
        let mut layers = 0u32;
        let mut i = 0;
        while i < self.layers.len() {
            if self.layers[i] < 32 {
                layers |= 1 << self.layers[i];
            }
            i += 1;
        }
        ComboDef {
            keys: self.mask(),
            action: self.action,
            layers,
            timeout_ms: self.timeout_ms,
            require_prior_idle_ms: self.require_prior_idle_ms,
        }
    }
}

/// Most combos the engine holds ([`Engine::set_combos`], Studio).
pub const MAX_COMBOS: usize = 16;

/// A combo as the engine uses it: like [`Combo`], with its keys and layers
/// as bitmaps, so a set can live in RAM and be edited (Studio).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComboDef {
    /// Positions, bit n = position n.
    pub keys: u64,
    pub action: Action,
    /// Layers on which it is active, bit n = layer n; 0 means all layers.
    /// Checked against the highest active layer, like ZMK.
    pub layers: u32,
    pub timeout_ms: u32,
    pub require_prior_idle_ms: u32,
}

impl ComboDef {
    /// An empty slot (never matches: no keys).
    pub const NONE: Self = Self {
        keys: 0,
        action: Action::None,
        layers: 0,
        timeout_ms: 0,
        require_prior_idle_ms: 0,
    };

    /// Whether it is active with `highest` as the highest active layer.
    #[must_use]
    pub const fn enabled_on(&self, highest: u8) -> bool {
        self.layers == 0 || (highest < 32 && self.layers & (1 << highest) != 0)
    }
}

pub struct Keymap<'a, const N: usize> {
    pub layers: &'a [[Action; N]],
    pub layer_names: &'a [&'a str],
    pub combos: &'a [Combo<'a>],
    pub conditional_layers: &'a [ConditionalLayer<'a>],
}

/// ZMK `zmk,conditional-layers` entry: `then_layer` is active exactly while
/// all of `if_layers` are (the "tri-layer" pattern).
#[derive(Clone, Copy, Debug)]
pub struct ConditionalLayer<'a> {
    pub if_layers: &'a [u8],
    pub then_layer: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Mod-tap: hold longer than this to get the modifier (ZMK `&mt` default 200 ms).
    pub tapping_term_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tapping_term_ms: 200,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// The HID state changed; send this report.
    Report(Report),
    /// The highest active layer changed.
    Layer(u8),
    CapsWord(bool),
    System(SystemAction),
    /// A mouse key went down (`true`) or up; `kb_core::mouse` makes motion of it.
    Mouse(MouseAction, bool),
    /// The pointer became active (a mouse-move key) or inactive (its layer
    /// was released): `MouseOrKey` keys are clicks while it is.
    Pointer(bool),
}

/// Who pressed an action: a key position, or a fired combo (by slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    Pos(u8),
    Combo(u8),
}

#[derive(Clone, Copy)]
struct PendingHoldTap {
    owner: Owner,
    hold: Key,
    tap: Key,
    deadline: u32,
}

#[derive(Clone, Copy)]
struct ActiveCombo {
    /// What it pressed (kept, so editing the combos can't change the release).
    action: Action,
    /// Positions of this combo that are still physically held.
    held: u64,
    /// The combo's action is released when the first of its keys is released.
    released: bool,
}

pub struct Engine<'a, const N: usize> {
    keymap: &'a Keymap<'a, N>,
    config: Config,

    /// The bindings presses resolve to: the keymap's layers, copied into RAM
    /// so they can be edited at runtime (Studio). Only the first
    /// `layer_count` are used.
    bindings: [[Action; N]; MAX_LAYERS],
    layer_count: usize,
    /// The combos, in RAM like the bindings; the first `combo_count` are used.
    combos: [ComboDef; MAX_COMBOS],
    combo_count: usize,

    /// Active layers: base, momentary and conditional.
    layers: u32,
    /// Layers switched on by keys (`&mo`), always including the base layer.
    momentary: u32,
    resolved: [Option<Action>; N],

    combo_buf: [u8; MAX_COMBO_KEYS],
    combo_len: usize,
    combo_start: u32,
    active_combos: [Option<ActiveCombo>; MAX_ACTIVE_COMBOS],

    pending: Option<PendingHoldTap>,
    held_hold_taps: [Option<(Owner, Key)>; MAX_HELD_HOLD_TAPS],

    mod_counts: [u8; 8],
    key_counts: [u8; 256],
    /// Keys whose count is above zero, kept in step with `key_counts`.
    report: Report,
    /// Modifier bits whose count is above zero, kept in step with `mod_counts`.
    explicit_mods: u8,
    /// Time of the event being handled.
    now: u32,
    /// When a (non-modifier) keycode was last pressed, for `require_prior_idle_ms`.
    last_keycode_press: Option<u32>,
    /// Quiet time before the first key of the combo being collected.
    combo_prior_idle: u32,
    /// Modifiers that belong to the most recently pressed key (e.g. Shift for `!`).
    implicit_mods: u8,
    implicit_owner: Option<Owner>,
    caps_word: bool,
    /// The layer the pointer became active on (a mouse-move press), while
    /// that layer is still active.
    pointer_layer: Option<u8>,

    last_report: Report,
    last_layer: u8,
    out: Queue<Output, OUTPUT_QUEUE>,
}

const fn bit(pos: u8) -> u64 {
    1 << pos
}

/// `true` when `now` is at or past `deadline`, robust to `u32` wraparound.
fn due(now: u32, deadline: u32) -> bool {
    (now.wrapping_sub(deadline) as i32) >= 0
}

impl<'a, const N: usize> Engine<'a, N> {
    /// An engine typing with `keymap`, nothing held, on its base layer.
    ///
    /// # Panics
    ///
    /// When `N` is above 64, or the keymap has no layers, more than
    /// `MAX_LAYERS` layers or more than `MAX_COMBOS` combos.
    #[must_use]
    pub fn new(keymap: &'a Keymap<'a, N>, config: Config) -> Self {
        assert!(N <= 64, "at most 64 key positions are supported");
        assert!(!keymap.layers.is_empty() && keymap.layers.len() <= MAX_LAYERS);
        assert!(keymap.combos.len() <= MAX_COMBOS);
        let mut bindings = [[Action::Trans; N]; MAX_LAYERS];
        bindings[..keymap.layers.len()].copy_from_slice(keymap.layers);
        let mut combos = [ComboDef::NONE; MAX_COMBOS];
        for (slot, c) in combos.iter_mut().zip(keymap.combos) {
            *slot = c.def();
        }
        Self {
            keymap,
            config,
            bindings,
            layer_count: keymap.layers.len(),
            combos,
            combo_count: keymap.combos.len(),
            layers: 1,
            momentary: 1,
            resolved: [None; N],
            combo_buf: [0; MAX_COMBO_KEYS],
            combo_len: 0,
            combo_start: 0,
            active_combos: [None; MAX_ACTIVE_COMBOS],
            pending: None,
            held_hold_taps: [None; MAX_HELD_HOLD_TAPS],
            mod_counts: [0; 8],
            key_counts: [0; 256],
            report: Report::default(),
            explicit_mods: 0,
            now: 0,
            last_keycode_press: None,
            combo_prior_idle: u32::MAX,
            implicit_mods: 0,
            implicit_owner: None,
            caps_word: false,
            pointer_layer: None,
            last_report: Report::default(),
            last_layer: 0,
            out: Queue::new(),
        }
    }

    pub fn press(&mut self, pos: u8, now: u32) {
        if (pos as usize) < N {
            self.tick(now);
            self.now = now;
            self.combo_press(pos, now);
        }
    }

    pub fn release(&mut self, pos: u8, now: u32) {
        if (pos as usize) < N {
            self.tick(now);
            self.now = now;
            self.combo_release(pos, now);
        }
    }

    /// Fire any timeouts that are due at `now`.
    pub fn tick(&mut self, now: u32) {
        loop {
            let combo = self.combo_deadline();
            let hold_tap = self.pending.map(|p| p.deadline);
            let due_at = match (combo, hold_tap) {
                (Some(c), Some(h)) if due(now, c) && due(now, h) => {
                    Some((due(h, c), if due(h, c) { c } else { h }))
                }
                (Some(c), _) if due(now, c) => Some((true, c)),
                (_, Some(h)) if due(now, h) => Some((false, h)),
                _ => None,
            };
            let Some((is_combo, at)) = due_at else { break };
            self.now = at;
            if is_combo {
                self.resolve_combo_buffer(at);
            } else {
                self.decide_hold();
            }
        }
    }

    /// When the caller should call [`tick`](Self::tick) next, if anything is waiting on a timeout.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u32> {
        match (self.combo_deadline(), self.pending.map(|p| p.deadline)) {
            (Some(c), Some(h)) => Some(if due(h, c) { c } else { h }),
            (c, h) => c.or(h),
        }
    }

    /// Next pending output, oldest first.
    pub fn poll(&mut self) -> Option<Output> {
        self.out.pop()
    }

    #[must_use]
    pub fn highest_layer(&self) -> u8 {
        31 - self.layers.leading_zeros() as u8
    }

    #[must_use]
    pub fn layer_name(&self, layer: u8) -> &'a str {
        self.keymap
            .layer_names
            .get(layer as usize)
            .copied()
            .unwrap_or("?")
    }

    /// The action a press of `pos` would trigger right now (after `&trans` fall-through).
    #[must_use]
    pub fn binding(&self, pos: u8) -> Action {
        if (pos as usize) < N {
            self.lookup(pos)
        } else {
            Action::None
        }
    }

    #[must_use]
    pub fn caps_word_active(&self) -> bool {
        self.caps_word
    }

    /// Whether the pointer is active: a mouse-move key was pressed since the
    /// layer it was on came up, so `MouseOrKey` keys are mouse buttons.
    #[must_use]
    pub fn pointer_active(&self) -> bool {
        self.pointer_layer.is_some()
    }

    /// Number of layers in the keymap.
    #[must_use]
    pub fn layer_count(&self) -> usize {
        self.layer_count
    }

    /// The bindings of `layer` as presses resolve them now (`None` past the last layer).
    #[must_use]
    pub fn layer_bindings(&self, layer: u8) -> Option<&[Action; N]> {
        self.bindings[..self.layer_count].get(layer as usize)
    }

    /// Change what `pos` does on `layer` (Studio). A key held now keeps the
    /// action it was pressed with until it is released. Returns false (and
    /// changes nothing) for a layer or position out of range, or a
    /// `MomentaryLayer` to a layer that doesn't exist.
    pub fn set_binding(&mut self, layer: u8, pos: u8, action: Action) -> bool {
        if (layer as usize) >= self.layer_count
            || (pos as usize) >= N
            || matches!(action, Action::MomentaryLayer(l) if l as usize >= self.layer_count)
        {
            return false;
        }
        self.bindings[layer as usize][pos as usize] = action;
        true
    }

    /// Replace all bindings at once (Studio: a whole keymap applied, saved
    /// or reverted). Needs exactly [`layer_count`](Self::layer_count) layers;
    /// returns false otherwise, or when an action names a missing layer.
    pub fn set_layers(&mut self, layers: &[[Action; N]]) -> bool {
        let count = self.layer_count;
        let valid = layers.len() == count
            && layers
                .iter()
                .flatten()
                .all(|a| !matches!(a, Action::MomentaryLayer(l) if *l as usize >= count));
        if valid {
            self.bindings[..count].copy_from_slice(layers);
        }
        valid
    }

    /// The combos presses are checked against now.
    #[must_use]
    pub fn combos(&self) -> &[ComboDef] {
        &self.combos[..self.combo_count]
    }

    /// Replace the combos (Studio). Refused (false, nothing changes) for more
    /// than [`MAX_COMBOS`], a combo without keys or with a position past the
    /// last, or a `MomentaryLayer` to a missing layer. A combo held now
    /// releases what it pressed; keys waiting to become a combo are settled
    /// against the new set.
    pub fn set_combos(&mut self, combos: &[ComboDef]) -> bool {
        let positions = if N >= 64 { u64::MAX } else { (1u64 << N) - 1 };
        let count = self.layer_count;
        let valid = combos.len() <= MAX_COMBOS
            && combos.iter().all(|c| {
                c.keys != 0
                    && c.keys & !positions == 0
                    && !matches!(c.action, Action::MomentaryLayer(l) if l as usize >= count)
            });
        if valid {
            self.combos[..combos.len()].copy_from_slice(combos);
            self.combo_count = combos.len();
        }
        valid
    }

    // ---- stage 1: combos -------------------------------------------------

    fn combo_enabled(&self, c: &ComboDef) -> bool {
        c.enabled_on(self.highest_layer())
    }

    fn combo_buf_mask(&self) -> u64 {
        self.combo_buf[..self.combo_len]
            .iter()
            .fold(0, |m, &p| m | bit(p))
    }

    /// Combos still possible for the keys in `mask`, pressed starting at `start`.
    fn candidates(&self, mask: u64, now: u32) -> impl Iterator<Item = (usize, ComboDef)> + '_ {
        let start = self.combo_start;
        let fresh = self.combo_len == 0;
        // Quiet time before the combo's first key: measured now for a fresh
        // attempt, remembered from its start otherwise.
        let prior_idle = if fresh {
            self.prior_idle(now)
        } else {
            self.combo_prior_idle
        };
        self.combos()
            .iter()
            .copied()
            .enumerate()
            .filter(move |(_, c)| {
                self.combo_enabled(c)
                    && c.keys & mask == mask
                    && (fresh || now.wrapping_sub(start) < c.timeout_ms)
                    && prior_idle >= c.require_prior_idle_ms
            })
    }

    /// Milliseconds since a keycode was last pressed (`u32::MAX` if never).
    fn prior_idle(&self, now: u32) -> u32 {
        self.last_keycode_press
            .map_or(u32::MAX, |t| now.wrapping_sub(t))
    }

    fn combo_deadline(&self) -> Option<u32> {
        if self.combo_len == 0 {
            return None;
        }
        let mask = self.combo_buf_mask();
        let longest = self
            .combos()
            .iter()
            .filter(|c| self.combo_enabled(c) && c.keys & mask == mask)
            .map(|c| c.timeout_ms)
            .max()
            .unwrap_or(0);
        Some(self.combo_start.wrapping_add(longest))
    }

    fn combo_press(&mut self, pos: u8, now: u32) {
        let mask = self.combo_buf_mask() | bit(pos);
        if self.candidates(mask, now).next().is_none() || self.combo_len == MAX_COMBO_KEYS {
            if self.combo_len > 0 {
                // This key can't extend any combo: settle the buffered keys first.
                self.resolve_combo_buffer(now);
                self.combo_press(pos, now);
            } else {
                self.behavior_press(pos, now);
            }
            return;
        }
        if self.combo_len == 0 {
            self.combo_start = now;
            self.combo_prior_idle = self.prior_idle(now);
        }
        self.combo_buf[self.combo_len] = pos;
        self.combo_len += 1;

        // Fire right away when exactly one combo is possible and it is complete.
        let unique = {
            let mut cands = self.candidates(mask, now);
            match (cands.next(), cands.next()) {
                (Some((idx, c)), None) if c.keys == mask => Some(idx),
                _ => None,
            }
        };
        if let Some(idx) = unique {
            self.combo_len = 0;
            self.fire_combo(idx, mask, now);
        }
    }

    /// Fire the combo that exactly matches the buffered keys, or let them through as normal presses.
    fn resolve_combo_buffer(&mut self, now: u32) {
        let mask = self.combo_buf_mask();
        let len = self.combo_len;
        self.combo_len = 0;
        let exact = self
            .combos()
            .iter()
            .position(|c| self.combo_enabled(c) && c.keys == mask);
        match exact {
            Some(idx) => self.fire_combo(idx, mask, now),
            None => {
                for i in 0..len {
                    self.behavior_press(self.combo_buf[i], now);
                }
            }
        }
    }

    fn fire_combo(&mut self, idx: usize, mask: u64, now: u32) {
        let Some(slot) = self.active_combos.iter().position(Option::is_none) else {
            // No free slot: treat the keys as normal presses.
            for pos in 0..N as u8 {
                if mask & bit(pos) != 0 {
                    self.behavior_press(pos, now);
                }
            }
            return;
        };
        let action = self.concrete(self.combos[idx].action);
        self.active_combos[slot] = Some(ActiveCombo {
            action,
            held: mask,
            released: false,
        });
        self.action_press(action, Owner::Combo(slot as u8), now);
    }

    fn combo_release(&mut self, pos: u8, now: u32) {
        if self.combo_len > 0 {
            // Keep event order: buffered presses happened before this release.
            self.resolve_combo_buffer(now);
        }
        for slot in 0..MAX_ACTIVE_COMBOS {
            let Some(mut ac) = self.active_combos[slot] else {
                continue;
            };
            if ac.held & bit(pos) == 0 {
                continue;
            }
            let release_now = !ac.released;
            ac.held &= !bit(pos);
            ac.released = true;
            self.active_combos[slot] = if ac.held == 0 { None } else { Some(ac) };
            if release_now {
                self.action_release(ac.action, Owner::Combo(slot as u8), now);
            }
            return;
        }
        self.behavior_release(pos, now);
    }

    // ---- stage 2: layer lookup -------------------------------------------

    fn lookup(&self, pos: u8) -> Action {
        for layer in (0..self.layer_count).rev() {
            if self.layers & (1 << layer) != 0 {
                let action = self.bindings[layer][pos as usize];
                if action != Action::Trans {
                    return action;
                }
            }
        }
        Action::None
    }

    /// What `action` does when pressed now: a `MouseOrKey` becomes its
    /// button or its key (and stays that until released).
    fn concrete(&self, action: Action) -> Action {
        match action {
            Action::MouseOrKey { mouse, key } => {
                if self.pointer_active() {
                    Action::Mouse(mouse)
                } else {
                    Action::Key(key)
                }
            }
            other => other,
        }
    }

    fn behavior_press(&mut self, pos: u8, now: u32) {
        let action = self.concrete(self.lookup(pos));
        self.resolved[pos as usize] = Some(action);
        self.action_press(action, Owner::Pos(pos), now);
    }

    fn behavior_release(&mut self, pos: u8, now: u32) {
        if let Some(action) = self.resolved[pos as usize].take() {
            self.action_release(action, Owner::Pos(pos), now);
        }
    }

    // ---- stage 3: behaviors ----------------------------------------------

    fn action_press(&mut self, action: Action, owner: Owner, now: u32) {
        // Hold-preferred mod-tap: pressing any other key while undecided means "hold".
        if self.pending.is_some() {
            self.decide_hold();
        }
        match action {
            Action::Key(k) => self.key_down(k, owner),
            Action::MomentaryLayer(layer) => {
                self.momentary |= 1 << layer;
                self.update_layers();
            }
            Action::ModTap { hold, tap } => {
                let deadline = now.wrapping_add(self.config.tapping_term_ms);
                self.pending = Some(PendingHoldTap {
                    owner,
                    hold,
                    tap,
                    deadline,
                });
            }
            Action::CapsWord => self.set_caps_word(!self.caps_word),
            Action::System(s) => {
                self.out.push(Output::System(s));
            }
            Action::Mouse(m) => {
                if matches!(m, MouseAction::Move(_)) && self.pointer_layer.is_none() {
                    self.pointer_layer = Some(self.highest_layer());
                    self.out.push(Output::Pointer(true));
                }
                self.out.push(Output::Mouse(m, true));
            }
            // Resolved at press (`concrete`); a bare one does nothing.
            Action::MouseOrKey { .. } | Action::Trans | Action::None => {}
        }
    }

    fn action_release(&mut self, action: Action, owner: Owner, _now: u32) {
        match action {
            Action::Key(k) => self.key_up(k, owner),
            Action::MomentaryLayer(layer) => {
                self.momentary &= !(1 << layer);
                self.momentary |= 1;
                self.update_layers();
            }
            Action::ModTap { tap, .. } => {
                if self.pending.is_some_and(|p| p.owner == owner) {
                    // Released before the tapping term and before any other press: a tap.
                    self.pending = None;
                    self.key_down(tap, owner);
                    self.key_up(tap, owner);
                } else if let Some((_, hold)) = self
                    .held_hold_taps
                    .iter_mut()
                    .find(|h| h.is_some_and(|(o, _)| o == owner))
                    .and_then(Option::take)
                {
                    self.key_up(hold, owner);
                }
            }
            Action::Mouse(m) => {
                self.out.push(Output::Mouse(m, false));
            }
            Action::CapsWord
            | Action::System(_)
            | Action::MouseOrKey { .. }
            | Action::Trans
            | Action::None => {}
        }
    }

    fn decide_hold(&mut self) {
        let Some(p) = self.pending.take() else { return };
        if let Some(slot) = self.held_hold_taps.iter_mut().find(|h| h.is_none()) {
            *slot = Some((p.owner, p.hold));
            self.key_down(p.hold, p.owner);
        } else {
            // No slot to remember the hold in, so its release couldn't undo it:
            // send the tap instead, which never leaves anything held.
            self.key_down(p.tap, p.owner);
            self.key_up(p.tap, p.owner);
        }
    }

    fn key_down(&mut self, k: Key, owner: Owner) {
        if k.is_modifier() {
            self.add_mods(k.modifier_bits(), 1);
        } else {
            let mut implicit = k.mods;
            if self.caps_word {
                if keys::is_alpha(k.code) {
                    implicit |= MOD_LSFT;
                } else if !caps_word_continues(k) {
                    self.set_caps_word(false);
                }
            }
            let count = &mut self.key_counts[k.code as usize];
            *count = count.saturating_add(1);
            if *count == 1 {
                self.report.set(k.code, true);
            }
            self.implicit_mods = implicit;
            self.implicit_owner = Some(owner);
            self.last_keycode_press = Some(self.now);
        }
        self.emit_report();
    }

    fn key_up(&mut self, k: Key, owner: Owner) {
        if k.is_modifier() {
            self.add_mods(k.modifier_bits(), -1);
        } else {
            let count = &mut self.key_counts[k.code as usize];
            let was_down = *count > 0;
            *count = count.saturating_sub(1);
            if was_down && *count == 0 {
                self.report.set(k.code, false);
            }
            if self.implicit_owner == Some(owner) {
                self.implicit_mods = 0;
                self.implicit_owner = None;
            }
        }
        self.emit_report();
    }

    fn add_mods(&mut self, bits: u8, delta: i8) {
        for (i, count) in self.mod_counts.iter_mut().enumerate() {
            if bits & (1 << i) != 0 {
                *count = count.saturating_add_signed(delta);
                if *count > 0 {
                    self.explicit_mods |= 1 << i;
                } else {
                    self.explicit_mods &= !(1 << i);
                }
            }
        }
    }

    /// The report is kept up to date per key (`report`, `explicit_mods`), so
    /// this is a copy and a compare rather than a walk over 256 key codes.
    fn emit_report(&mut self) {
        let mut report = self.report;
        report.mods = self.explicit_mods | self.implicit_mods;
        if report != self.last_report {
            self.last_report = report;
            self.out.push(Output::Report(report));
        }
    }

    /// Recompute the active layers from the momentary ones plus the
    /// conditional layers whose conditions hold.
    fn update_layers(&mut self) {
        let mut layers = self.momentary;
        for c in self.keymap.conditional_layers {
            let needed = c.if_layers.iter().fold(0u32, |m, &l| m | 1 << l);
            if needed != 0 && self.momentary & needed == needed {
                layers |= 1 << c.then_layer;
            }
        }
        self.layers = layers;
        // The pointer ends with the layer it started on (nav let go).
        if let Some(l) = self.pointer_layer
            && self.layers & (1 << l) == 0
        {
            self.pointer_layer = None;
            self.out.push(Output::Pointer(false));
        }
        self.layer_changed();
    }

    fn layer_changed(&mut self) {
        let highest = self.highest_layer();
        if highest != self.last_layer {
            self.last_layer = highest;
            self.out.push(Output::Layer(highest));
        }
    }

    fn set_caps_word(&mut self, on: bool) {
        if on != self.caps_word {
            self.caps_word = on;
            self.out.push(Output::CapsWord(on));
        }
    }
}

/// ZMK's default caps-word continue list: letters, digits, `_`, Backspace, Delete.
fn caps_word_continues(k: Key) -> bool {
    keys::is_alpha(k.code)
        || keys::is_digit(k.code)
        || k == keys::UNDER
        || k == keys::BSPC
        || k == keys::DEL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{kp, mo, mt};
    use crate::keys::{A, B, C, LSHFT};

    const MANY: usize = 20;
    static ALL_MOD_TAPS: [[Action; MANY]; 1] = [[mt(LSHFT, A); MANY]];
    static KEYMAP: Keymap<'static, MANY> = Keymap {
        layers: &ALL_MOD_TAPS,
        layer_names: &["test"],
        combos: &[],
        conditional_layers: &[],
    };

    fn last_report(e: &mut Engine<'_, MANY>) -> Option<Report> {
        let mut last = None;
        while let Some(out) = e.poll() {
            if let Output::Report(r) = out {
                last = Some(r);
            }
        }
        last
    }

    #[test]
    fn more_held_mod_taps_than_slots_never_leave_a_modifier_stuck() {
        // Arrange: hold more mod-taps than there are hold slots
        let mut e = Engine::new(&KEYMAP, Config::default());
        for pos in 0..MANY as u8 {
            e.press(pos, u32::from(pos) * 300);
            e.tick(u32::from(pos) * 300 + 200);
        }
        let _ = last_report(&mut e);

        // Act
        for pos in 0..MANY as u8 {
            e.release(pos, 10_000 + u32::from(pos));
        }

        // Assert
        let report = last_report(&mut e).expect("releases change the report");
        assert_eq!(report.mods, 0);
        assert_eq!(report.keys().count(), 0);
    }

    static TWO_LAYERS: [[Action; 3]; 2] =
        [[kp(A), kp(B), mo(1)], [kp(C), Action::Trans, Action::Trans]];
    static SMALL: Keymap<'static, 3> = Keymap {
        layers: &TWO_LAYERS,
        layer_names: &["base", "upper"],
        combos: &[],
        conditional_layers: &[],
    };

    fn keys_of(e: &mut Engine<'_, 3>) -> Vec<u8> {
        let mut last = None;
        while let Some(out) = e.poll() {
            if let Output::Report(r) = out {
                last = Some(r);
            }
        }
        last.map(|r| r.keys().collect()).unwrap_or_default()
    }

    #[test]
    fn edited_binding_is_what_a_press_sends() {
        // Arrange
        let mut e = Engine::new(&SMALL, Config::default());
        e.set_binding(0, 0, kp(C));

        // Act
        e.press(0, 0);

        // Assert
        assert_eq!(keys_of(&mut e), [C.code]);
    }

    #[test]
    fn key_held_while_edited_releases_what_it_pressed() {
        // Arrange
        let mut e = Engine::new(&SMALL, Config::default());
        e.press(0, 0);
        let _ = keys_of(&mut e);
        e.set_binding(0, 0, kp(C));

        // Act
        e.release(0, 10);

        // Assert: A went up, C never went down
        assert_eq!(keys_of(&mut e), Vec::<u8>::new());
    }

    #[test]
    fn edited_layer_key_switches_the_layer() {
        // Arrange: position 1 becomes the layer key
        let mut e = Engine::new(&SMALL, Config::default());
        e.set_binding(0, 1, mo(1));
        e.press(1, 0);

        // Act
        e.press(0, 10);

        // Assert
        assert_eq!(keys_of(&mut e), [C.code]);
    }

    #[test]
    fn set_layers_replaces_every_binding() {
        // Arrange
        let mut e = Engine::new(&SMALL, Config::default());
        let edited = [[kp(B), kp(B), mo(1)], [kp(A), Action::Trans, Action::Trans]];

        // Act
        let ok = e.set_layers(&edited);

        // Assert
        assert!(ok);
        assert_eq!(e.layer_bindings(0), Some(&edited[0]));
        assert_eq!(e.layer_bindings(1), Some(&edited[1]));
    }

    #[test]
    fn set_layers_needs_every_layer() {
        // Arrange
        let mut e = Engine::new(&SMALL, Config::default());

        // Act
        let ok = e.set_layers(&[[kp(B); 3]]);

        // Assert
        assert!(!ok);
        assert_eq!(e.layer_bindings(0), Some(&TWO_LAYERS[0]));
    }

    #[test]
    fn binding_out_of_range_is_refused() {
        // Arrange
        let mut e = Engine::new(&SMALL, Config::default());

        // Act
        let refused = [
            e.set_binding(2, 0, kp(A)),
            e.set_binding(0, 3, kp(A)),
            e.set_binding(0, 0, mo(2)),
        ];

        // Assert
        assert_eq!(refused, [false; 3]);
        assert_eq!(e.layer_bindings(0), Some(&TWO_LAYERS[0]));
    }

    #[test]
    fn no_layer_past_the_last_one() {
        // Arrange
        let e = Engine::new(&SMALL, Config::default());

        // Act
        let past = e.layer_bindings(2);

        // Assert
        assert_eq!(past, None);
        assert_eq!(e.layer_count(), 2);
    }

    static COMBO_LAYERS: [[Action; 4]; 2] = [
        [kp(A), kp(B), kp(C), mo(1)],
        [Action::Trans, Action::Trans, Action::Trans, Action::Trans],
    ];
    static COMBO_KEYS: [u8; 2] = [0, 1];
    static WITH_COMBO: Keymap<'static, 4> = Keymap {
        layers: &COMBO_LAYERS,
        layer_names: &["base", "upper"],
        combos: &[Combo {
            name: "ab",
            keys: &COMBO_KEYS,
            action: kp(C),
            layers: &[],
            timeout_ms: 50,
            require_prior_idle_ms: 0,
        }],
        conditional_layers: &[],
    };

    fn combo_def(keys: u64, action: Action) -> ComboDef {
        ComboDef {
            keys,
            action,
            layers: 0,
            timeout_ms: 50,
            require_prior_idle_ms: 0,
        }
    }

    fn reports(e: &mut Engine<'_, 4>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(o) = e.poll() {
            if let Output::Report(r) = o {
                out.push(r.keys().collect());
            }
        }
        out
    }

    #[test]
    fn engine_starts_with_the_compiled_combos() {
        // Arrange
        let e = Engine::new(&WITH_COMBO, Config::default());

        // Act
        let combos = e.combos();

        // Assert
        assert_eq!(combos, [combo_def(0b11, kp(C))]);
    }

    #[test]
    fn edited_combo_fires_its_new_action() {
        // Arrange
        let mut e = Engine::new(&WITH_COMBO, Config::default());
        e.set_combos(&[combo_def(0b11, kp(B))]);

        // Act
        e.press(0, 0);
        e.press(1, 5);

        // Assert
        assert_eq!(reports(&mut e), [vec![B.code]]);
    }

    #[test]
    fn added_combo_fires() {
        // Arrange
        let mut e = Engine::new(&WITH_COMBO, Config::default());
        e.set_combos(&[combo_def(0b11, kp(C)), combo_def(0b110, kp(A))]);

        // Act
        e.press(1, 0);
        e.press(2, 5);

        // Assert
        assert_eq!(reports(&mut e), [vec![A.code]]);
    }

    #[test]
    fn removed_combo_lets_the_keys_through() {
        // Arrange
        let mut e = Engine::new(&WITH_COMBO, Config::default());
        e.set_combos(&[]);

        // Act
        e.press(0, 0);
        e.press(1, 5);

        // Assert
        assert_eq!(reports(&mut e), [vec![A.code], vec![A.code, B.code]]);
    }

    #[test]
    fn combo_held_while_edited_releases_what_it_pressed() {
        // Arrange
        let mut e = Engine::new(&WITH_COMBO, Config::default());
        e.press(0, 0);
        e.press(1, 5);
        let _ = reports(&mut e);
        e.set_combos(&[combo_def(0b11, kp(B))]);

        // Act
        e.release(0, 20);

        // Assert: C went up, nothing left down
        assert_eq!(reports(&mut e), [Vec::<u8>::new()]);
    }

    #[test]
    fn combo_layers_bitmap_limits_it() {
        // Arrange: the combo only on layer 1
        let mut e = Engine::new(&WITH_COMBO, Config::default());
        e.set_combos(&[ComboDef {
            layers: 1 << 1,
            ..combo_def(0b11, kp(C))
        }]);

        // Act
        e.press(0, 0);
        e.press(1, 5);

        // Assert: on the base layer the keys go through
        assert_eq!(reports(&mut e), [vec![A.code], vec![A.code, B.code]]);
    }

    #[test]
    fn invalid_combo_sets_are_refused() {
        // Arrange
        let mut e = Engine::new(&WITH_COMBO, Config::default());
        let too_many = [combo_def(0b11, kp(A)); MAX_COMBOS + 1];

        // Act
        let refused = [
            e.set_combos(&too_many),
            e.set_combos(&[combo_def(0, kp(A))]),
            e.set_combos(&[combo_def(1 << 4, kp(A))]),
            e.set_combos(&[combo_def(0b11, mo(2))]),
        ];

        // Assert
        assert_eq!(refused, [false; 4]);
        assert_eq!(e.combos(), [combo_def(0b11, kp(C))]);
    }

    #[test]
    fn compiled_combo_converts_keys_and_layers_to_bitmaps() {
        // Arrange
        let combo = Combo {
            name: "x",
            keys: &[3, 40],
            action: kp(A),
            layers: &[0, 2],
            timeout_ms: 30,
            require_prior_idle_ms: 7,
        };

        // Act
        let def = combo.def();

        // Assert
        assert_eq!(
            def,
            ComboDef {
                keys: 1 << 3 | 1 << 40,
                action: kp(A),
                layers: 0b101,
                timeout_ms: 30,
                require_prior_idle_ms: 7
            }
        );
    }

    #[test]
    fn edited_binding_while_held_on_another_layer_keeps_the_press() {
        // Arrange: hold the layer key, press A's position, then edit the upper layer
        let mut e = Engine::new(&WITH_COMBO, Config::default());
        e.press(3, 0);
        e.press(2, 100);
        let _ = reports(&mut e);
        e.set_binding(1, 2, kp(A));

        // Act
        e.release(2, 200);

        // Assert: the C that was pressed goes up
        assert_eq!(reports(&mut e), [Vec::<u8>::new()]);
    }
}

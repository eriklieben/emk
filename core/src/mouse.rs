//! Mouse keys: the pointer, the buttons and the wheel from keys, made to
//! feel like a pointer, not a game.
//!
//! The engine reports mouse keys going down and up ([`Output::Mouse`]);
//! [`Mouse`] turns them into motion, integrated in 1 ms steps
//! ([`Mouse::tick`]) with sub-pixel accumulation in 16.16 fixed point, and
//! hands out HID reports ([`Mouse::take_report`]): on USB every millisecond
//! there is motion, on Bluetooth at most every [`MouseConfig::ble_interval_ms`].
//!
//! How a movement key behaves (all constants in [`MouseConfig::DEFAULT`]):
//!
//! ```text
//! speed (px/s)
//! 1800 |                                   ___________  v_max
//!      |                              __---
//!      |                          _--'        smoothstep from v_start to v_max
//!      |                       _-'            over ramp_ms (600 ms)
//!  300 |_____________________.'               v_start
//!      |                  ./
//!    0 +-------------...'---------------------------------> held (ms)
//!      0 nudge     start_ms (80)          start + ramp (680)
//! ```
//!
//! - **press**: the pointer moves [`MouseConfig::nudge_px`] (1 px) at once, so
//!   a short tap is a precise nudge (a 50 ms tap moves about 2 px);
//! - **hold**: speed eases in from 0 to `v_start` (cubic) over `start_ms`, then
//!   from `v_start` to `v_max` along a smoothstep over `ramp_ms`: no step
//!   anywhere, no "game" constant speed, but fast across a 4K screen
//!   (3840 px in about 2.5 s) once you hold;
//! - **diagonals**: the speed is that of the vector, so two keys go exactly
//!   as fast as one (each axis × 1/√2), not 41% faster;
//! - **turning**: each axis' velocity follows its target with a time constant
//!   of `follow_ms` (40 ms), and the hold time carries on while any movement
//!   key stays down: changing or adding a direction bends the path smoothly
//!   instead of jerking or starting over;
//! - **release**: the last movement key up stops the pointer at once and
//!   drops the sub-pixel rest: no drift, no overshoot;
//! - **precision** (held): `precision_pct` (28%) of the speed, capped at
//!   `precision_max` (250 px/s), so it stays slow however long you hold;
//! - **scrolling** ramps like the pointer, in 1/16 detents: a tap is
//!   exactly one detent (at press; nothing more before `scroll_delay_ms`);
//!   held, it eases in from standstill to `scroll_v_start` and on along a
//!   smoothstep to `scroll_v_max` over `scroll_ramp_ms`; release stops at
//!   once (no glide); precision keeps it at `scroll_precision_max`. With the
//!   HID Resolution Multiplier set by the computer (Windows, Linux:
//!   [`Mouse::set_resolution`]) the wheel and pan move in 1/16 detents and
//!   scroll smoothly; without it (macOS, BIOS) the same ramp comes out as
//!   whole detents that repeat faster and faster.
//!   Left/right scroll is the horizontal pan (AC Pan).
//!
//! [`Output::Mouse`]: crate::Output::Mouse

use crate::action::{Direction, MouseAction};

/// One pixel in 16.16 fixed point.
const ONE: i32 = 1 << 16;

/// 1/√2 in 16.16 (0.70711: error 0.0005%).
const INV_SQRT2: i64 = 46341;

/// Everything that sets how the mouse keys feel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseConfig {
    /// Pixels the pointer moves the moment a movement key goes down.
    pub nudge_px: i32,
    /// Time to ease in from standstill to `v_start` (cubic: gentle at first).
    pub start_ms: u32,
    /// Speed at the end of the ease-in, px/s.
    pub v_start: i32,
    /// Time from `v_start` to `v_max` (smoothstep).
    pub ramp_ms: u32,
    /// Top speed, px/s.
    pub v_max: i32,
    /// Time constant with which each axis' velocity follows its target.
    pub follow_ms: i32,
    /// Precision mode: this percentage of the speed...
    pub precision_pct: i32,
    /// ... and never faster than this, px/s.
    pub precision_max: i32,
    /// Scrolling: time from the detent at press to the start of the ramp
    /// (a tap shorter than this is exactly one detent).
    pub scroll_delay_ms: u32,
    /// Scrolling: ease-in from standstill to `scroll_v_start`.
    pub scroll_start_ms: u32,
    /// Scrolling speed after the ease-in, 1/16 detents per second.
    pub scroll_v_start: i32,
    /// Time from `scroll_v_start` to `scroll_v_max` (smoothstep).
    pub scroll_ramp_ms: u32,
    /// Top scrolling speed, 1/16 detents per second.
    pub scroll_v_max: i32,
    /// Precision mode: scrolling at most this fast, 1/16 detents per second.
    pub scroll_precision_max: i32,
    /// Least time between two motion reports on Bluetooth (a connection
    /// interval is 7.5-15 ms; more would only queue up).
    pub ble_interval_ms: u32,
}

impl MouseConfig {
    pub const DEFAULT: Self = Self {
        nudge_px: 1,
        start_ms: 80,
        v_start: 300,
        ramp_ms: 600,
        v_max: 1800,
        follow_ms: 40,
        precision_pct: 28,
        precision_max: 250,
        scroll_delay_ms: 250,
        scroll_start_ms: 150,
        scroll_v_start: 4 * 16,
        scroll_ramp_ms: 1200,
        scroll_v_max: 30 * 16,
        scroll_precision_max: 3 * 16,
        ble_interval_ms: 8,
    };

    /// Speed (px/s) after a movement key has been held `held_ms`, at full
    /// speed (`precision` false) or in precision mode.
    #[must_use]
    pub fn speed(&self, held_ms: u32, precision: bool) -> i32 {
        let v = ramp(
            held_ms,
            self.start_ms,
            self.v_start,
            self.ramp_ms,
            self.v_max,
        );
        if precision {
            (v * self.precision_pct / 100).min(self.precision_max)
        } else {
            v
        }
    }

    /// Scrolling speed (1/16 detents per second) after a scroll key has
    /// been held `held_ms` (0 until `scroll_delay_ms`: the tap's detent).
    #[must_use]
    pub fn scroll_speed(&self, held_ms: u32, precision: bool) -> i32 {
        let Some(t) = held_ms.checked_sub(self.scroll_delay_ms) else {
            return 0;
        };
        let v = ramp(
            t,
            self.scroll_start_ms,
            self.scroll_v_start,
            self.scroll_ramp_ms,
            self.scroll_v_max,
        );
        if precision {
            (v * self.precision_pct / 100).min(self.scroll_precision_max)
        } else {
            v
        }
    }
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The speed curve: a cubic ease-in from 0 to `v_start` over `start_ms`,
/// then a smoothstep from `v_start` to `v_max` over `ramp_ms`.
fn ramp(t: u32, start_ms: u32, v_start: i32, ramp_ms: u32, v_max: i32) -> i32 {
    if t < start_ms {
        let u = frac(t, start_ms);
        mul(
            i64::from(v_start),
            u * u / i64::from(ONE) * u / i64::from(ONE),
        )
    } else {
        let u = frac(t - start_ms, ramp_ms);
        let smooth = u * u / i64::from(ONE) * (3 * i64::from(ONE) - 2 * u) / i64::from(ONE);
        v_start + mul(i64::from(v_max - v_start), smooth)
    }
}

/// Wheel / pan units per detent with the Resolution Multiplier set.
pub const HIRES: i32 = 16;

/// What the computer set in the mouse's feature report (the Resolution
/// Multipliers): whether the wheel and the pan are high resolution.
#[must_use]
pub fn resolution_from_feature(byte: u8) -> (bool, bool) {
    (byte & 0x03 == 1, (byte >> 2) & 0x03 == 1)
}

/// The feature report for `wheel` / `pan` high resolution (`GET_REPORT`).
#[must_use]
pub fn feature_byte(wheel: bool, pan: bool) -> u8 {
    u8::from(wheel) | u8::from(pan) << 2
}

/// `n / d` in 16.16, clamped to 0..=1.
fn frac(n: u32, d: u32) -> i64 {
    if d == 0 || n >= d {
        i64::from(ONE)
    } else {
        i64::from(n) * i64::from(ONE) / i64::from(d)
    }
}

/// `v × f` for a 16.16 fraction `f`.
fn mul(v: i64, f: i64) -> i32 {
    (v * f / i64::from(ONE)) as i32
}

/// A HID mouse report: buttons (bit 0 left, 1 right, 2 middle), relative X
/// (right +) and Y (down +), wheel (up +) and pan (right +).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseReport {
    pub buttons: u8,
    pub x: i16,
    pub y: i16,
    pub wheel: i8,
    pub pan: i8,
}

/// Bytes of a mouse report (without report ID).
pub const REPORT_LEN: usize = 7;

impl MouseReport {
    /// As sent: buttons, X and Y (i16 little endian), wheel, pan.
    #[must_use]
    pub fn bytes(&self) -> [u8; REPORT_LEN] {
        let x = self.x.to_le_bytes();
        let y = self.y.to_le_bytes();
        [
            self.buttons,
            x[0],
            x[1],
            y[0],
            y[1],
            self.wheel as u8,
            self.pan as u8,
        ]
    }

    #[must_use]
    pub fn from_bytes(b: &[u8; REPORT_LEN]) -> Self {
        Self {
            buttons: b[0],
            x: i16::from_le_bytes([b[1], b[2]]),
            y: i16::from_le_bytes([b[3], b[4]]),
            wheel: b[5] as i8,
            pan: b[6] as i8,
        }
    }
}

/// The mouse's HID report descriptor, without a report ID (USB: an
/// interface of its own). [`REPORT_LEN`] bytes: 5 buttons + 3 padding bits,
/// X and Y as i16, wheel, AC Pan.
pub const REPORT_DESCRIPTOR: [u8; 123] = mouse_descriptor::<123>(None);

/// The same with report ID `id` (Bluetooth: next to the keyboard).
///
/// # Panics
///
/// At compile time, when `N` is not the descriptor's length (123 bytes, 125
/// with a report ID).
#[must_use]
pub const fn mouse_descriptor<const N: usize>(id: Option<u8>) -> [u8; N] {
    const HEAD: [u8; 6] = [0x05, 0x01, 0x09, 0x02, 0xA1, 0x01];
    const BODY: [u8; 117] = [
        0x09, 0x01, // Usage (Pointer)
        0xA1, 0x00, // Collection (Physical)
        0x05, 0x09, //   Usage Page (Button)
        0x19, 0x01, //   Usage Minimum (1)
        0x29, 0x05, //   Usage Maximum (5)
        0x15, 0x00, //   Logical Minimum (0)
        0x25, 0x01, //   Logical Maximum (1)
        0x95, 0x05, //   Report Count (5)
        0x75, 0x01, //   Report Size (1)
        0x81, 0x02, //   Input (Data, Var, Abs)
        0x95, 0x01, //   Report Count (1)
        0x75, 0x03, //   Report Size (3)
        0x81, 0x03, //   Input (Const)
        0x05, 0x01, //   Usage Page (Generic Desktop)
        0x09, 0x30, //   Usage (X)
        0x09, 0x31, //   Usage (Y)
        0x16, 0x01, 0x80, //   Logical Minimum (-32767)
        0x26, 0xFF, 0x7F, //   Logical Maximum (32767)
        0x75, 0x10, //   Report Size (16)
        0x95, 0x02, //   Report Count (2)
        0x81, 0x06, //   Input (Data, Var, Rel)
        0xA1, 0x02, //   Collection (Logical): the wheel and its multiplier
        0x09, 0x48, //     Usage (Resolution Multiplier)
        0x15, 0x00, //     Logical Minimum (0)
        0x25, 0x01, //     Logical Maximum (1)
        0x35, 0x01, //     Physical Minimum (1)
        0x45, 0x10, //     Physical Maximum (16)
        0x75, 0x02, //     Report Size (2)
        0x95, 0x01, //     Report Count (1)
        0xB1, 0x02, //     Feature (Data, Var, Abs)
        0x35, 0x00, //     Physical Minimum (0)
        0x45, 0x00, //     Physical Maximum (0)
        0x09, 0x38, //     Usage (Wheel)
        0x15, 0x81, //     Logical Minimum (-127)
        0x25, 0x7F, //     Logical Maximum (127)
        0x75, 0x08, //     Report Size (8)
        0x81, 0x06, //     Input (Data, Var, Rel)
        0xC0, //   End Collection
        0xA1, 0x02, //   Collection (Logical): the pan and its multiplier
        0x09, 0x48, //     Usage (Resolution Multiplier)
        0x15, 0x00, //     Logical Minimum (0)
        0x25, 0x01, //     Logical Maximum (1)
        0x35, 0x01, //     Physical Minimum (1)
        0x45, 0x10, //     Physical Maximum (16)
        0x75, 0x02, //     Report Size (2)
        0xB1, 0x02, //     Feature (Data, Var, Abs)
        0x35, 0x00, //     Physical Minimum (0)
        0x45, 0x00, //     Physical Maximum (0)
        0x05, 0x0C, //     Usage Page (Consumer)
        0x0A, 0x38, 0x02, //     Usage (AC Pan)
        0x15, 0x81, //     Logical Minimum (-127)
        0x25, 0x7F, //     Logical Maximum (127)
        0x75, 0x08, //     Report Size (8)
        0x81, 0x06, //     Input (Data, Var, Rel)
        0xC0, //   End Collection
        0x75, 0x04, //   Report Size (4)
        0xB1, 0x03, //   Feature (Const): pads the feature report to a byte
        0xC0, // End Collection
        0xC0, // End Collection
    ];
    let mut out = [0u8; N];
    let mut at = 0;
    let mut i = 0;
    while i < HEAD.len() {
        out[at] = HEAD[i];
        at += 1;
        i += 1;
    }
    if let Some(id) = id {
        out[at] = 0x85;
        out[at + 1] = id;
        at += 2;
    }
    let mut i = 0;
    while i < BODY.len() {
        out[at] = BODY[i];
        at += 1;
        i += 1;
    }
    assert!(at == N, "descriptor length");
    out
}

/// Bytes of the input report `id` (0: reports without an ID) that
/// `descriptor` declares, without the ID byte. Walks the short items
/// (Report Size / Count / ID, Input), enough to check our descriptors.
#[must_use]
pub fn input_report_len(descriptor: &[u8], id: u8) -> usize {
    report_len(descriptor, id, 0x80)
}

/// Bytes of the feature report `id`, as [`input_report_len`].
#[must_use]
pub fn feature_report_len(descriptor: &[u8], id: u8) -> usize {
    report_len(descriptor, id, 0xB0)
}

/// Bytes of the output report `id`, as [`input_report_len`].
#[must_use]
pub fn output_report_len(descriptor: &[u8], id: u8) -> usize {
    report_len(descriptor, id, 0x90)
}

fn report_len(descriptor: &[u8], id: u8, main: u8) -> usize {
    let (mut size, mut count, mut current, mut bits) = (0u32, 0u32, 0u8, 0u32);
    let mut i = 0;
    while i < descriptor.len() {
        let prefix = descriptor[i];
        let len = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        let mut data = 0u32;
        for k in 0..len {
            data |= u32::from(*descriptor.get(i + 1 + k).unwrap_or(&0)) << (8 * k);
        }
        match prefix & 0xFC {
            0x74 => size = data,
            0x94 => count = data,
            0x84 => current = data as u8,
            m if m == main && current == id => bits += size * count,
            _ => {}
        }
        i += 1 + len;
    }
    bits.div_ceil(8) as usize
}

/// Bit of a direction in a held-keys mask.
const fn dir_bit(d: Direction) -> u8 {
    match d {
        Direction::Left => 1,
        Direction::Down => 2,
        Direction::Up => 4,
        Direction::Right => 8,
    }
}

/// -1, 0 or 1 along X and Y for the directions in `mask`.
fn axes(mask: u8) -> (i64, i64) {
    let on = |d| i64::from(mask & dir_bit(d) != 0);
    (
        on(Direction::Right) - on(Direction::Left),
        on(Direction::Down) - on(Direction::Up),
    )
}

/// Most 1 ms steps one tick catches up on (after the loop slept).
const MAX_STEPS: u32 = 100;

/// The mouse keys' state and motion; see the module documentation.
#[derive(Clone, Debug)]
pub struct Mouse {
    config: MouseConfig,
    moves: u8,
    scrolls: u8,
    buttons: u8,
    precision: bool,
    /// When the movement keys went down (none held before).
    move_since: u32,
    /// Velocity per axis, 16.16 px per ms.
    v: [i64; 2],
    /// Sub-pixel position not reported yet, 16.16.
    rest: [i64; 2],
    /// Whole motion waiting for a report: x, y, wheel, pan.
    pending: [i32; 4],
    sent_buttons: u8,
    scroll_since: u32,
    /// Sub-unit scrolling not reported yet, 16.16 of 1/16 detents: wheel, pan.
    scroll_rest: [i64; 2],
    /// Wheel and pan units per detent: 1, or [`HIRES`] once the computer set
    /// the Resolution Multiplier.
    resolution: [i32; 2],
    last_step: Option<u32>,
    last_report: Option<u32>,
}

impl Default for Mouse {
    fn default() -> Self {
        Self::new(MouseConfig::DEFAULT)
    }
}

impl Mouse {
    #[must_use]
    pub const fn new(config: MouseConfig) -> Self {
        Self {
            config,
            moves: 0,
            scrolls: 0,
            buttons: 0,
            precision: false,
            move_since: 0,
            v: [0; 2],
            rest: [0; 2],
            pending: [0; 4],
            sent_buttons: 0,
            scroll_since: 0,
            scroll_rest: [0; 2],
            resolution: [1, 1],
            last_step: None,
            last_report: None,
        }
    }

    #[must_use]
    pub fn config(&self) -> &MouseConfig {
        &self.config
    }

    /// The buttons held now (report bits).
    #[must_use]
    pub fn buttons(&self) -> u8 {
        self.buttons
    }

    /// A mouse key went down.
    pub fn press(&mut self, action: MouseAction, now: u32) {
        self.tick(now);
        match action {
            MouseAction::Move(d) => {
                if self.moves == 0 {
                    self.move_since = now;
                    self.v = [0; 2];
                    self.rest = [0; 2];
                }
                if self.moves & dir_bit(d) == 0 {
                    // The nudge: this key's direction, at once.
                    let (x, y) = axes(dir_bit(d));
                    self.pending[0] += x as i32 * self.config.nudge_px;
                    self.pending[1] += y as i32 * self.config.nudge_px;
                }
                self.moves |= dir_bit(d);
            }
            MouseAction::Scroll(d) => {
                if self.scrolls == 0 {
                    self.scroll_since = now;
                    self.scroll_rest = [0; 2];
                }
                if self.scrolls & dir_bit(d) == 0 {
                    // A tap: exactly one detent, at once.
                    self.notch(dir_bit(d));
                }
                self.scrolls |= dir_bit(d);
            }
            MouseAction::Button(b) => self.buttons |= b.bit(),
            MouseAction::Precision => self.precision = true,
        }
    }

    /// A mouse key went up.
    pub fn release(&mut self, action: MouseAction, now: u32) {
        self.tick(now);
        match action {
            MouseAction::Move(d) => {
                self.moves &= !dir_bit(d);
                if self.moves == 0 {
                    // Stop at once: no drift, no overshoot.
                    self.v = [0; 2];
                    self.rest = [0; 2];
                }
            }
            MouseAction::Scroll(d) => {
                self.scrolls &= !dir_bit(d);
                if self.scrolls == 0 {
                    // Stop at once: no glide.
                    self.scroll_rest = [0; 2];
                }
            }
            MouseAction::Button(b) => self.buttons &= !b.bit(),
            MouseAction::Precision => self.precision = false,
        }
    }

    /// One detent of wheel / pan for the directions in `mask`.
    fn notch(&mut self, mask: u8) {
        let (x, y) = axes(mask);
        // Up scrolls up: wheel +; right pans right: pan +.
        self.pending[2] -= y as i32 * self.resolution[0];
        self.pending[3] += x as i32 * self.resolution[1];
    }

    /// The computer set (or cleared) the Resolution Multipliers: wheel and
    /// pan in 1/16 detents (`true`) or whole detents.
    pub fn set_resolution(&mut self, wheel: bool, pan: bool) {
        let units = |hires| if hires { HIRES } else { 1 };
        self.resolution = [units(wheel), units(pan)];
    }

    /// Wheel and pan units per detent now (1 or [`HIRES`]).
    #[must_use]
    pub fn resolution(&self) -> [i32; 2] {
        self.resolution
    }

    /// Integrate the motion up to `now` in 1 ms steps.
    pub fn tick(&mut self, now: u32) {
        let Some(last) = self.last_step else {
            self.last_step = Some(now);
            return;
        };
        let steps = now.wrapping_sub(last);
        if steps as i32 <= 0 {
            return;
        }
        let first = if steps > MAX_STEPS {
            now.wrapping_sub(MAX_STEPS)
        } else {
            last
        };
        let mut t = first;
        while t != now {
            t = t.wrapping_add(1);
            self.step(t);
        }
        self.last_step = Some(now);
    }

    fn step(&mut self, t: u32) {
        if self.moves != 0 {
            let speed = i64::from(
                self.config
                    .speed(t.wrapping_sub(self.move_since), self.precision),
            );
            // px/s → 16.16 px per ms.
            let per_ms = speed * i64::from(ONE) / 1000;
            let (x, y) = axes(self.moves);
            let scale = if x != 0 && y != 0 {
                INV_SQRT2
            } else {
                i64::from(ONE)
            };
            let target = [
                x * per_ms * scale / i64::from(ONE),
                y * per_ms * scale / i64::from(ONE),
            ];
            let follow = i64::from(self.config.follow_ms.max(1));
            for (axis, target) in target.into_iter().enumerate() {
                self.v[axis] += (target - self.v[axis]) / follow;
                self.rest[axis] += self.v[axis];
                let whole = self.rest[axis] / i64::from(ONE);
                self.rest[axis] -= whole * i64::from(ONE);
                self.pending[axis] = self.pending[axis].saturating_add(whole as i32);
            }
        }
        if self.scrolls != 0 {
            let speed = i64::from(
                self.config
                    .scroll_speed(t.wrapping_sub(self.scroll_since), self.precision),
            );
            // 1/16 detents per second → 16.16 of 1/16 detents per ms.
            let per_ms = speed * i64::from(ONE) / 1000;
            let (x, y) = axes(self.scrolls);
            // Wheel up is +, the down key is +y.
            for (axis, dir) in [-y, x].into_iter().enumerate() {
                self.scroll_rest[axis] += dir * per_ms;
                // One unit is 16 / resolution sixteenths.
                let unit = i64::from(ONE) * i64::from(HIRES / self.resolution[axis]);
                let whole = self.scroll_rest[axis] / unit;
                self.scroll_rest[axis] -= whole * unit;
                self.pending[2 + axis] = self.pending[2 + axis].saturating_add(whole as i32);
            }
        }
    }

    /// Whether anything still moves or waits to be reported (the key loop
    /// keeps ticking while it does).
    #[must_use]
    pub fn active(&self) -> bool {
        self.moves != 0
            || self.scrolls != 0
            || self.pending != [0; 4]
            || self.buttons != self.sent_buttons
    }

    /// The report to send now, if any: at once when the buttons changed,
    /// otherwise when there is motion and `min_interval_ms` has passed since
    /// the last report (0 on USB). Motion that doesn't fit a report (more
    /// than ±32767 px, ±127 notches) stays for the next one.
    pub fn take_report(&mut self, now: u32, min_interval_ms: u32) -> Option<MouseReport> {
        let buttons_changed = self.buttons != self.sent_buttons;
        let motion = self.pending != [0; 4];
        let waited = self
            .last_report
            .is_none_or(|t| now.wrapping_sub(t) >= min_interval_ms);
        if !(buttons_changed || (motion && waited)) {
            return None;
        }
        let take = |v: &mut i32, limit: i32| {
            let part = (*v).clamp(-limit, limit);
            *v -= part;
            part
        };
        let report = MouseReport {
            buttons: self.buttons,
            x: take(&mut self.pending[0], i32::from(i16::MAX)) as i16,
            y: take(&mut self.pending[1], i32::from(i16::MAX)) as i16,
            wheel: take(&mut self.pending[2], 127) as i8,
            pan: take(&mut self.pending[3], 127) as i8,
        };
        self.sent_buttons = self.buttons;
        self.last_report = Some(now);
        Some(report)
    }

    /// Release everything (a new output, a restart): buttons up, no motion.
    pub fn reset(&mut self) {
        *self = Self {
            sent_buttons: self.sent_buttons,
            ..Self::new(self.config)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::MouseButton;

    const RIGHT: MouseAction = MouseAction::Move(Direction::Right);
    const LEFT: MouseAction = MouseAction::Move(Direction::Left);
    const DOWN: MouseAction = MouseAction::Move(Direction::Down);
    const UP: MouseAction = MouseAction::Move(Direction::Up);

    /// Run `mouse` from `from` to `to` ms, taking USB reports every ms;
    /// the motion summed (x, y, wheel, pan) and the reports.
    fn run(mouse: &mut Mouse, from: u32, to: u32) -> ([i32; 4], Vec<MouseReport>) {
        let mut sum = [0; 4];
        let mut reports = Vec::new();
        for t in from..=to {
            mouse.tick(t);
            if let Some(r) = mouse.take_report(t, 0) {
                sum[0] += i32::from(r.x);
                sum[1] += i32::from(r.y);
                sum[2] += i32::from(r.wheel);
                sum[3] += i32::from(r.pan);
                reports.push(r);
            }
        }
        (sum, reports)
    }

    fn started() -> Mouse {
        let mut m = Mouse::default();
        m.tick(0);
        m
    }

    fn length(v: [i32; 4]) -> f64 {
        (f64::from(v[0]).powi(2) + f64::from(v[1]).powi(2)).sqrt()
    }

    #[test]
    fn press_nudges_one_pixel_at_once() {
        // Arrange
        let mut m = started();

        // Act
        m.press(RIGHT, 0);

        // Assert
        assert_eq!(m.take_report(0, 0).map(|r| r.x), Some(1));
    }

    #[test]
    fn short_tap_is_a_precise_nudge() {
        // Arrange
        let mut m = started();
        m.press(RIGHT, 0);

        // Act: a 50 ms tap
        let (moved, _) = run(&mut m, 0, 50);
        m.release(RIGHT, 50);
        let (after, _) = run(&mut m, 51, 300);

        // Assert
        assert!((1..=3).contains(&moved[0]), "{moved:?}");
        assert_eq!(after, [0; 4]);
    }

    #[test]
    fn speed_ramp_never_goes_down() {
        // Arrange
        let c = MouseConfig::DEFAULT;

        // Act
        let speeds: Vec<i32> = (0..1000).map(|t| c.speed(t, false)).collect();

        // Assert
        assert!(speeds.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(speeds[0], 0);
    }

    #[test]
    fn speed_ramp_has_no_steps() {
        // Arrange
        let c = MouseConfig::DEFAULT;

        // Act: the biggest change from one ms to the next
        let jump = (0..1000)
            .map(|t| c.speed(t + 1, false) - c.speed(t, false))
            .max()
            .unwrap();

        // Assert: under 1% of top speed per ms
        assert!(jump <= c.v_max / 100, "{jump}");
    }

    #[test]
    fn speed_ramp_reaches_its_points() {
        // Arrange
        let c = MouseConfig::DEFAULT;

        // Act
        let points = [
            c.speed(c.start_ms, false),
            c.speed(c.start_ms + c.ramp_ms / 2, false),
            c.speed(c.start_ms + c.ramp_ms, false),
            c.speed(10_000, false),
        ];

        // Assert
        assert_eq!(points, [300, 1050, 1800, 1800]);
    }

    #[test]
    fn holding_reaches_top_speed() {
        // Arrange
        let mut m = started();
        m.press(RIGHT, 0);
        run(&mut m, 0, 1000);

        // Act: one more second at top speed
        let (moved, _) = run(&mut m, 1001, 2000);

        // Assert: 1800 px/s, give or take rounding
        assert!((1790..=1800).contains(&moved[0]), "{moved:?}");
    }

    #[test]
    fn velocity_rises_smoothly_while_held() {
        // Arrange
        let mut m = started();
        m.press(RIGHT, 0);
        run(&mut m, 0, 5);

        // Act: pixels per 10 ms over the first second
        let per_10ms: Vec<i32> = (0..99)
            .map(|i| run(&mut m, 6 + i * 10, 15 + i * 10).0[0])
            .collect();

        // Assert: never slower than 1 px below the window before (rounding)
        assert!(
            per_10ms.windows(2).all(|w| w[1] >= w[0] - 1),
            "{per_10ms:?}"
        );
    }

    #[test]
    fn diagonal_is_as_fast_as_straight() {
        // Arrange
        let mut straight = started();
        straight.press(RIGHT, 0);
        let mut diagonal = started();
        diagonal.press(RIGHT, 0);
        diagonal.press(DOWN, 0);

        // Act: a second and a half of each
        let (s, _) = run(&mut straight, 0, 1500);
        let (d, _) = run(&mut diagonal, 0, 1500);

        // Assert: within 2%
        let ratio = length(d) / length(s);
        assert!((0.98..=1.02).contains(&ratio), "{ratio} {s:?} {d:?}");
        assert_eq!(d[0], d[1]);
    }

    #[test]
    fn opposite_keys_cancel() {
        // Arrange
        let mut m = started();
        m.press(LEFT, 0);
        m.press(RIGHT, 0);
        let _ = m.take_report(0, 0);

        // Act
        let (moved, _) = run(&mut m, 1, 500);

        // Assert
        assert_eq!(moved, [0; 4]);
    }

    #[test]
    fn reversing_turns_smoothly_without_starting_over() {
        // Arrange: full speed right, then left pressed and right let go
        let mut m = started();
        m.press(RIGHT, 0);
        run(&mut m, 0, 1000);
        m.press(LEFT, 1000);
        m.release(RIGHT, 1000);
        let _ = m.take_report(1000, 0);

        // Act: x per ms after the turn
        let per_ms: Vec<i32> = (1001..=1200).map(|t| run(&mut m, t, t).0[0]).collect();

        // Assert: no jump bigger than a few px between ms, through zero,
        // and back at (left) top speed without a new ramp
        assert!(
            per_ms.windows(2).all(|w| (w[1] - w[0]).abs() <= 3),
            "{per_ms:?}"
        );
        assert!(per_ms[0] > 0);
        let end: i32 = per_ms[150..].iter().sum();
        assert!(end <= -85, "{end}");
    }

    #[test]
    fn adding_a_direction_bends_the_path() {
        // Arrange: right at speed, then down added
        let mut m = started();
        m.press(RIGHT, 0);
        run(&mut m, 0, 800);
        m.press(DOWN, 800);
        let nudge = m.take_report(800, 0);

        // Act
        let (moved, _) = run(&mut m, 801, 1300);

        // Assert: the new direction nudges, then the path bends (y grows,
        // x keeps going: no restart)
        assert_eq!(nudge.map(|r| r.y), Some(1));
        assert!(moved[1] > 400, "{moved:?}");
        assert!(moved[0] > moved[1], "{moved:?}");
    }

    #[test]
    fn release_stops_at_once() {
        // Arrange: at top speed
        let mut m = started();
        m.press(UP, 0);
        run(&mut m, 0, 1000);

        // Act
        m.release(UP, 1000);
        let _ = m.take_report(1000, 0);
        let (after, reports) = run(&mut m, 1001, 2000);

        // Assert
        assert_eq!(after, [0; 4]);
        assert_eq!(reports, [] as [MouseReport; 0]);
        assert!(!m.active());
    }

    #[test]
    fn up_is_negative_y() {
        // Arrange
        let mut m = started();

        // Act
        m.press(UP, 0);

        // Assert
        assert_eq!(m.take_report(0, 0).map(|r| r.y), Some(-1));
    }

    #[test]
    fn precision_mode_stays_slow() {
        // Arrange
        let mut m = started();
        m.press(MouseAction::Precision, 0);
        m.press(RIGHT, 0);
        run(&mut m, 0, 2000);

        // Act: a second at the precision cap
        let (moved, _) = run(&mut m, 2001, 3000);

        // Assert
        assert!((240..=250).contains(&moved[0]), "{moved:?}");
    }

    #[test]
    fn precision_speed_is_a_fraction_of_normal() {
        // Arrange
        let c = MouseConfig::DEFAULT;

        // Act
        let (normal, precise) = (c.speed(200, false), c.speed(200, true));

        // Assert
        assert_eq!(precise, normal * 28 / 100);
    }

    #[test]
    fn slow_motion_adds_up_from_sub_pixels() {
        // Arrange: precision at the start of the ramp: well under 1 px/ms
        let mut m = started();
        m.press(MouseAction::Precision, 0);
        m.press(RIGHT, 0);
        let _ = m.take_report(0, 0);

        // Act
        let (moved, reports) = run(&mut m, 1, 400);

        // Assert: it moves, a pixel at a time, never more per ms
        assert!(moved[0] > 5, "{moved:?}");
        assert!(reports.iter().all(|r| r.x == 1));
    }

    #[test]
    fn scroll_notches_at_once() {
        // Arrange
        let mut m = started();

        // Act
        m.press(MouseAction::Scroll(Direction::Down), 0);

        // Assert
        assert_eq!(m.take_report(0, 0).map(|r| r.wheel), Some(-1));
    }

    #[test]
    fn scroll_left_and_right_pan() {
        // Arrange
        let mut m = started();

        // Act
        m.press(MouseAction::Scroll(Direction::Left), 0);
        let left = m.take_report(0, 0).map(|r| (r.pan, r.wheel));
        m.press(MouseAction::Scroll(Direction::Right), 1);
        let right = m.take_report(1, 0).map(|r| (r.pan, r.wheel));

        // Assert
        assert_eq!((left, right), (Some((-1, 0)), Some((1, 0))));
    }

    #[test]
    fn button_is_reported_at_once_and_released() {
        // Arrange
        let mut m = started();
        m.press(MouseAction::Button(MouseButton::Left), 0);
        let down = m.take_report(0, 8);

        // Act
        m.release(MouseAction::Button(MouseButton::Left), 1);
        let up = m.take_report(1, 8);

        // Assert: buttons ignore the report interval
        assert_eq!(down.map(|r| r.buttons), Some(1));
        assert_eq!(up.map(|r| r.buttons), Some(0));
    }

    #[test]
    fn buttons_have_their_bits() {
        // Arrange
        let mut m = started();

        // Act
        m.press(MouseAction::Button(MouseButton::Right), 0);
        m.press(MouseAction::Button(MouseButton::Middle), 0);

        // Assert
        assert_eq!(m.take_report(0, 0).map(|r| r.buttons), Some(0b110));
    }

    #[test]
    fn bluetooth_interval_collects_motion() {
        // Arrange: same motion, USB vs every 8 ms
        let mut usb = started();
        usb.press(RIGHT, 0);
        let mut ble = started();
        ble.press(RIGHT, 0);

        // Act
        let (usb_sum, usb_reports) = run(&mut usb, 0, 1000);
        let mut ble_sum = 0;
        let mut ble_reports = 0;
        for t in 0..=1000 {
            ble.tick(t);
            if let Some(r) = ble.take_report(t, 8) {
                ble_sum += i32::from(r.x);
                ble_reports += 1;
            }
        }
        // What came after the last report goes out with the next one.
        if let Some(r) = ble.take_report(1000, 0) {
            ble_sum += i32::from(r.x);
            ble_reports += 1;
        }

        // Assert: the same distance in an eighth of the reports
        assert_eq!(ble_sum, usb_sum[0]);
        assert!(ble_reports <= 1000 / 8 + 2, "{ble_reports}");
        assert!(usb_reports.len() > 500);
    }

    #[test]
    fn nothing_to_report_without_keys() {
        // Arrange
        let mut m = started();

        // Act
        let (sum, reports) = run(&mut m, 0, 100);

        // Assert
        assert_eq!(sum, [0; 4]);
        assert_eq!(reports, [] as [MouseReport; 0]);
        assert!(!m.active());
    }

    #[test]
    fn long_gap_catches_up_at_most_100_ms() {
        // Arrange: held, then the loop didn't tick for 5 s
        let mut m = started();
        m.press(RIGHT, 0);
        m.tick(1000);
        let _ = m.take_report(1000, 0);

        // Act
        m.tick(6000);
        let r = m.take_report(6000, 0).unwrap();

        // Assert: 100 ms of top speed, not 5 s
        assert!((170..=190).contains(&r.x), "{}", r.x);
    }

    #[test]
    fn huge_motion_is_split_over_reports() {
        // Arrange
        let mut m = started();
        m.pending = [40_000, 0, 200, 0];

        // Act
        let first = m.take_report(0, 0).unwrap();
        let second = m.take_report(1, 0).unwrap();

        // Assert
        assert_eq!((first.x, first.wheel), (32767, 127));
        assert_eq!((second.x, second.wheel), (7233, 73));
    }

    #[test]
    fn report_bytes_round_trip() {
        // Arrange
        let r = MouseReport {
            buttons: 5,
            x: -300,
            y: 1200,
            wheel: -1,
            pan: 1,
        };

        // Act
        let back = MouseReport::from_bytes(&r.bytes());

        // Assert
        assert_eq!(back, r);
        assert_eq!(r.bytes(), [5, 0xD4, 0xFE, 0xB0, 0x04, 0xFF, 0x01]);
    }

    #[test]
    fn descriptor_declares_the_seven_byte_report() {
        // Arrange / Act
        let len = input_report_len(&REPORT_DESCRIPTOR, 0);

        // Assert
        assert_eq!(len, REPORT_LEN);
    }

    #[test]
    fn descriptor_with_an_id_declares_it_for_that_id() {
        // Arrange
        let with_id: [u8; 125] = mouse_descriptor(Some(2));

        // Act
        let (two, none) = (input_report_len(&with_id, 2), input_report_len(&with_id, 0));

        // Assert
        assert_eq!((two, none), (REPORT_LEN, 0));
    }

    #[test]
    fn reset_lets_go_of_everything() {
        // Arrange
        let mut m = started();
        m.press(RIGHT, 0);
        m.press(MouseAction::Button(MouseButton::Left), 0);
        let _ = m.take_report(0, 0);

        // Act
        m.reset();
        let r = m.take_report(1, 0);

        // Assert: one report with the button up, then quiet
        assert_eq!(r.map(|r| r.buttons), Some(0));
        let (sum, _) = run(&mut m, 2, 200);
        assert_eq!(sum, [0; 4]);
    }

    /// Wheel units over a hold of `ms` (pressed at 0, released at `ms`),
    /// and how many reports carried them.
    fn scroll_for(ms: u32, hires: bool, precision: bool) -> (i32, usize) {
        let mut m = started();
        m.set_resolution(hires, hires);
        if precision {
            m.press(MouseAction::Precision, 0);
        }
        m.press(MouseAction::Scroll(Direction::Up), 0);
        let (held, reports) = run(&mut m, 0, ms);
        m.release(MouseAction::Scroll(Direction::Up), ms);
        let (after, _) = run(&mut m, ms + 1, ms + 500);
        assert_eq!(after, [0; 4], "no glide after release");
        (held[2], reports.len())
    }

    #[test]
    fn scroll_tap_is_exactly_one_detent() {
        // Arrange / Act: taps up to the delay
        let taps: Vec<_> = [10, 100, 249]
            .iter()
            .map(|&ms| scroll_for(ms, false, false).0)
            .collect();

        // Assert
        assert_eq!(taps, [1, 1, 1]);
    }

    #[test]
    fn hires_scroll_tap_is_exactly_one_detent_too() {
        // Arrange / Act
        let (units, _) = scroll_for(200, true, false);

        // Assert: 16 sixteenths
        assert_eq!(units, HIRES);
    }

    #[test]
    fn scroll_speed_starts_slow_and_never_drops() {
        // Arrange
        let c = MouseConfig::DEFAULT;

        // Act
        let speeds: Vec<i32> = (0..3000).map(|t| c.scroll_speed(t, false)).collect();

        // Assert
        assert!(speeds.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(
            speeds[c.scroll_delay_ms as usize], 0,
            "eases in from standstill"
        );
        assert_eq!(speeds[2999], c.scroll_v_max);
    }

    #[test]
    fn scroll_speed_has_no_steps() {
        // Arrange
        let c = MouseConfig::DEFAULT;

        // Act
        let jump = (0..3000)
            .map(|t| c.scroll_speed(t + 1, false) - c.scroll_speed(t, false))
            .max()
            .unwrap();

        // Assert: under 2% of top speed per ms
        assert!(jump <= c.scroll_v_max / 50, "{jump}");
    }

    #[test]
    fn held_scroll_reaches_top_speed() {
        // Arrange: up to speed
        let mut m = started();
        m.press(MouseAction::Scroll(Direction::Down), 0);
        run(&mut m, 0, 2000);

        // Act: a second at top speed
        let (moved, _) = run(&mut m, 2001, 3000);

        // Assert: 30 detents a second, down
        assert!((-30..=-29).contains(&moved[2]), "{moved:?}");
    }

    #[test]
    fn hires_scroll_comes_in_small_steps() {
        // Arrange
        let mut m = started();
        m.set_resolution(true, true);
        m.press(MouseAction::Scroll(Direction::Down), 0);
        run(&mut m, 0, 2000);

        // Act
        let (moved, reports) = run(&mut m, 2001, 3000);

        // Assert: the same 30 detents, as hundreds of reports of 1/16
        assert!((-480..=-470).contains(&moved[2]), "{moved:?}");
        assert!(reports.len() > 400, "{}", reports.len());
        assert!(reports.iter().all(|r| r.wheel == -1));
    }

    #[test]
    fn scroll_without_hires_comes_as_whole_detents_faster_and_faster() {
        // Arrange
        let mut m = started();
        m.press(MouseAction::Scroll(Direction::Down), 0);

        // Act: when each detent came
        let mut times = Vec::new();
        for t in 0..=2000 {
            m.tick(t);
            if let Some(r) = m.take_report(t, 0) {
                assert_eq!(r.wheel, -1);
                times.push(t);
            }
        }

        // Assert
        let gaps: Vec<u32> = times.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps[0] >= 250, "nothing before the delay: {gaps:?}");
        assert!(gaps[1..].windows(2).all(|w| w[1] <= w[0] + 1), "{gaps:?}");
        assert!(*gaps.last().unwrap() <= 34, "{gaps:?}");
    }

    #[test]
    fn precision_keeps_scrolling_slow() {
        // Arrange / Act: two seconds held in precision mode
        let (units, _) = scroll_for(3000, false, true);

        // Assert: at most 3 detents a second after the delay, plus the tap
        assert!(units <= 1 + 3 * 3, "{units}");
        assert!(units > 2, "{units}");
    }

    #[test]
    fn scroll_release_stops_at_once() {
        // Arrange / Act
        let (units, _) = scroll_for(1500, true, false);

        // Assert (the helper checks nothing comes after the release)
        assert!(units > HIRES, "{units}");
    }

    #[test]
    fn feature_byte_sets_both_multipliers() {
        // Arrange
        let mut m = started();
        let (wheel, pan) = resolution_from_feature(0x05);

        // Act
        m.set_resolution(wheel, pan);

        // Assert
        assert_eq!(m.resolution(), [HIRES, HIRES]);
        assert_eq!(feature_byte(wheel, pan), 0x05);
    }

    #[test]
    fn feature_byte_zero_means_whole_detents() {
        // Arrange / Act
        let got = resolution_from_feature(0x00);

        // Assert
        assert_eq!(got, (false, false));
    }

    #[test]
    fn pan_and_wheel_have_their_own_multiplier() {
        // Arrange / Act
        let got = [resolution_from_feature(0x01), resolution_from_feature(0x04)];

        // Assert
        assert_eq!(got, [(true, false), (false, true)]);
    }

    #[test]
    fn descriptor_declares_a_one_byte_feature_report() {
        // Arrange / Act
        let len = feature_report_len(&REPORT_DESCRIPTOR, 0);

        // Assert: two 2-bit multipliers and padding
        assert_eq!(len, 1);
    }
}

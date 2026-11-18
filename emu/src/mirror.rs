//! Mirror of a real keyboard, for the emulator's DEBUG tab: the state the
//! left half sends over its log port (`kb_core::debug`) turned into the same
//! SSD1306 transactions the emulated displays get.
//!
//! The left screen is the frame the hardware drew, as it is. The left half
//! doesn't have the right half's frame, so the right screen is rendered here
//! from the forwarded status with the firmware's own screen code. It shows
//! what the right half would draw for that status; its animations run on
//! this computer's clock, and its battery charging mark is unknown.

use kb_core::keymaps::corne;
use kb_core::power::PowerState;
use kb_core::{Action, debug::FRAME_BYTES};
use kb_display::anim::{Animator, SLEEP_ART_MS};
use kb_display::frame::{Frame, Half};
use kb_display::screens::{self, Style};
use kb_display::ssd1306;
use kb_display::status::{Status, Transport};

use crate::bt::LINK_STAGES;

/// The status fields the debug stream carries, as the .NET side passes them.
/// Booleans are bytes (0 or 1).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MirrorStatus {
    pub layer: u8,
    pub caps_word: u8,
    pub mods: u8,
    /// 0 USB, 1 Bluetooth.
    pub transport: u8,
    /// 0-based.
    pub profile: u8,
    pub profile_connected: u8,
    pub profile_open: u8,
    pub linked: u8,
    pub battery_left: u8,
    pub battery_right: u8,
    pub charging_left: u8,
    pub charging_right: u8,
    /// 0 active, 1 idle, 2 sleep.
    pub power: u8,
    pub reserved: u8,
    pub wpm: u16,
    pub presses: u16,
}

/// What a display should show.
enum Target {
    /// Exactly this frame, or off.
    Frame(Option<Box<Frame>>),
    /// Rendered from the status.
    Rendered,
}

pub struct Mirror {
    status: Status,
    power: PowerState,
    targets: [Target; 2],
    shown: [Option<Frame>; 2],
    lit: [bool; 2],
    idle_at: [Option<u32>; 2],
    animators: [Animator; 2],
}

impl Default for Mirror {
    fn default() -> Self {
        Self::new()
    }
}

impl Mirror {
    #[must_use]
    pub fn new() -> Self {
        Self {
            status: Status::default(),
            power: PowerState::Active,
            targets: [Target::Frame(None), Target::Rendered],
            shown: [None, None],
            lit: [false, false],
            idle_at: [None, None],
            animators: [Animator::new(), Animator::new()],
        }
    }

    pub fn set_status(&mut self, s: &MirrorStatus, stage: &str) {
        let on = |b: u8| b != 0;
        self.status = Status {
            layer: s.layer,
            caps_word: on(s.caps_word),
            transport: if s.transport == 0 {
                Transport::Usb
            } else {
                Transport::Ble
            },
            profile: s.profile,
            battery: [s.battery_left, s.battery_right],
            wpm: s.wpm,
            mods: s.mods,
            charging: [on(s.charging_left), on(s.charging_right)],
            key_presses: s.presses,
            profile_connected: on(s.profile_connected),
            profile_open: on(s.profile_open),
            peripheral_connected: on(s.linked),
            notification: None,
            // The screens need a 'static name: the known stages.
            link_detail: LINK_STAGES
                .iter()
                .find(|&&known| known == stage)
                .copied()
                .unwrap_or(""),
            link_requests: self.status.link_requests,
            // Left-screen only (the left screen is the hardware's frame).
            pairing: kb_display::status::Pairing::Off,
            radio_dbm: self.status.radio_dbm,
            radio_changes: self.status.radio_changes,
            // Left-screen only too.
            studio_unlocked: false,
            pointer_active: false,
        };
        self.power = match s.power {
            1 => PowerState::Idle,
            2 => PowerState::Sleep,
            _ => PowerState::Active,
        };
    }

    #[must_use]
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Show exactly `frame` on display `half` (`None`: switched off).
    pub fn set_frame(&mut self, half: usize, frame: Option<&[u8; FRAME_BYTES]>) {
        self.targets[half.min(1)] = Target::Frame(frame.map(|bytes| {
            let mut f = Box::<Frame>::default();
            f.buf.copy_from_slice(bytes);
            f
        }));
    }

    /// Transactions bringing display `half` up to date.
    pub fn display_writes(&mut self, half: usize, now: u32) -> Vec<Vec<u8>> {
        let half = half.min(1);
        let mut writes = Vec::new();
        let frame = match &self.targets[half] {
            Target::Frame(None) => {
                if self.lit[half] {
                    self.lit[half] = false;
                    ssd1306::display_off(|w| writes.push(w.to_vec()));
                }
                return writes;
            }
            Target::Frame(Some(frame)) => (**frame).clone(),
            Target::Rendered => {
                if let Some(frame) = self.rendered(half, now) {
                    frame
                } else {
                    if self.lit[half] {
                        self.lit[half] = false;
                        ssd1306::display_off(|w| writes.push(w.to_vec()));
                    }
                    return writes;
                }
            }
        };
        if self.shown[half].is_none() {
            ssd1306::init(|w| writes.push(w.to_vec())); // ends with display on
        } else if !self.lit[half] {
            ssd1306::display_on(|w| writes.push(w.to_vec()));
        }
        self.lit[half] = true;
        if self.shown[half].as_ref() != Some(&frame) {
            ssd1306::flush(&frame, |w| writes.push(w.to_vec()));
            self.shown[half] = Some(frame);
        }
        writes
    }

    /// What the firmware would draw on `half` for the status; `None` while off.
    fn rendered(&mut self, half: usize, now: u32) -> Option<Frame> {
        let side = if half == 0 { Half::Left } else { Half::Right };
        let mut frame = Frame::default();
        match self.power {
            PowerState::Sleep => {
                self.idle_at[half] = None;
                return None;
            }
            PowerState::Idle => {
                let elapsed = now.wrapping_sub(*self.idle_at[half].get_or_insert(now));
                if !self.lit[half] || elapsed >= SLEEP_ART_MS {
                    return None;
                }
                let mut from = Frame::default();
                screens::render(
                    &mut from,
                    side,
                    Style::Portrait,
                    &self.status,
                    corne::KEYMAP.layer_names,
                );
                screens::sleep_art(&mut frame, side, elapsed, &from);
                return Some(frame);
            }
            PowerState::Active => self.idle_at[half] = None,
        }
        let anim = self.animators[half].update(&self.status, now);
        screens::render_animated(
            &mut frame,
            side,
            Style::Portrait,
            &self.status,
            corne::KEYMAP.layer_names,
            &anim,
        );
        Some(frame)
    }
}

/// What key `pos` does with `layer` on top of the base layer (`&trans` falls
/// through to the base layer).
#[must_use]
pub fn layer_binding(layer: u8, pos: u8) -> Action {
    let layers = corne::KEYMAP.layers;
    if pos as usize >= corne::KEYS {
        return Action::None;
    }
    for l in [layer as usize, 0] {
        if let Some(keys) = layers.get(l)
            && keys[pos as usize] != Action::Trans
        {
            return keys[pos as usize];
        }
    }
    Action::None
}

// ---- C ABI ------------------------------------------------------------------

/// The mirror behind `m`, or `None` for null.
///
/// # Safety
///
/// `m` is null or a mirror from `kb_mirror_new` that is not freed yet, and
/// not otherwise borrowed while the reference lives.
unsafe fn mirror<'a>(m: *mut Mirror) -> Option<&'a mut Mirror> {
    // SAFETY: null or live and unaliased, as this function requires.
    unsafe { m.as_mut() }
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_mirror_new() -> *mut Mirror {
    Box::into_raw(Box::new(Mirror::new()))
}

/// # Safety
///
/// `m` is null or a mirror from [`kb_mirror_new`] that is not freed yet; it is
/// invalid afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kb_mirror_free(m: *mut Mirror) {
    if !m.is_null() {
        // SAFETY: from `kb_mirror_new`, freed once.
        drop(unsafe { Box::from_raw(m) });
    }
}

/// The status from a `#S` line; `stage` is `stage_len` bytes of UTF-8.
///
/// # Safety
///
/// `m` is null or a mirror from [`kb_mirror_new`] that is not freed yet;
/// `status` is null or points to a valid [`MirrorStatus`]; `stage` is null or
/// points to `stage_len` readable bytes. Nothing is kept past the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kb_mirror_set_status(
    m: *mut Mirror,
    status: *const MirrorStatus,
    stage: *const u8,
    stage_len: usize,
) {
    // SAFETY: `m` is null or live, as this function requires.
    let Some(m) = (unsafe { mirror(m) }) else {
        return;
    };
    // SAFETY: the caller passes a valid status (or null, checked here).
    let Some(status) = (unsafe { status.as_ref() }) else {
        return;
    };
    let stage = if stage.is_null() {
        &[][..]
    } else {
        // SAFETY: non-null here, and the caller guarantees `stage_len`
        // readable bytes behind it for the duration of the call.
        unsafe { std::slice::from_raw_parts(stage, stage_len) }
    };
    let stage = std::str::from_utf8(stage).unwrap_or("");
    m.set_status(status, stage);
}

/// The frame from a `#F` line for display `half` (512 bytes), or null: off.
///
/// # Safety
///
/// `m` is null or a mirror from [`kb_mirror_new`] that is not freed yet;
/// `frame` is null or points to `FRAME_BYTES` (512) readable bytes. Nothing is
/// kept past the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kb_mirror_set_frame(m: *mut Mirror, half: u8, frame: *const u8) {
    // SAFETY: `m` is null or live, as this function requires.
    let Some(m) = (unsafe { mirror(m) }) else {
        return;
    };
    // SAFETY: the caller passes FRAME_BYTES readable bytes (or null).
    let frame = unsafe { frame.cast::<[u8; FRAME_BYTES]>().as_ref() };
    m.set_frame(half as usize, frame);
}

/// I²C writes for display `half`, framed like `kb_emu_display`.
///
/// # Safety
///
/// `m` is null or a mirror from [`kb_mirror_new`] that is not freed yet; `buf`
/// is null or valid for writing `cap` bytes. Nothing is kept past the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kb_mirror_display(
    m: *mut Mirror,
    half: u8,
    now_ms: u32,
    buf: *mut u8,
    cap: usize,
) -> usize {
    // SAFETY: `m` is null or live, as this function requires.
    let Some(m) = (unsafe { mirror(m) }) else {
        return 0;
    };
    let half = (half as usize).min(1);
    let snapshot = (m.shown[half].clone(), m.lit[half], m.idle_at[half]);
    let mut framed = Vec::new();
    for w in m.display_writes(half, now_ms) {
        framed.extend_from_slice(&(w.len() as u16).to_le_bytes());
        framed.extend_from_slice(&w);
    }
    if framed.len() > cap || buf.is_null() {
        // Not delivered: resend next time.
        (m.shown[half], m.lit[half], m.idle_at[half]) = snapshot;
        return framed.len();
    }
    // SAFETY: `buf` is null or has room for `cap` bytes, as this function requires.
    unsafe { crate::copy_out(&framed, buf, cap) }
}

/// Label of key `pos` with `layer` active (as `kb_emu_label`).
///
/// # Safety
///
/// `buf` is null or valid for writing `cap` bytes. Nothing is kept past the
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kb_mirror_label(layer: u8, pos: u8, buf: *mut u8, cap: usize) -> usize {
    // SAFETY: `buf` is null or has room for `cap` bytes, as this function requires.
    unsafe {
        crate::copy_out(
            crate::action_label(layer_binding(layer, pos)).as_bytes(),
            buf,
            cap,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> MirrorStatus {
        MirrorStatus {
            battery_left: 80,
            battery_right: 70,
            linked: 1,
            transport: 1,
            profile_connected: 1,
            ..MirrorStatus::default()
        }
    }

    #[test]
    fn status_fields_map_onto_the_screen_status() {
        // Arrange
        let mut m = Mirror::new();
        let s = MirrorStatus {
            layer: 2,
            caps_word: 1,
            ..status()
        };

        // Act
        m.set_status(&s, "linked");

        // Assert
        let st = m.status();
        assert_eq!(
            (st.layer, st.caps_word, st.battery, st.link_detail),
            (2, true, [80, 70], "linked")
        );
    }

    #[test]
    fn an_unknown_stage_is_hidden() {
        // Arrange
        let mut m = Mirror::new();

        // Act
        m.set_status(&status(), "warp-drive");

        // Assert
        assert_eq!(m.status().link_detail, "");
    }

    #[test]
    fn first_frame_initializes_the_display() {
        // Arrange
        let mut m = Mirror::new();
        m.set_frame(0, Some(&[0xFF; FRAME_BYTES]));

        // Act
        let writes = m.display_writes(0, 0);

        // Assert: the init sequence, then header and data
        let data = writes.last().unwrap();
        assert_eq!(
            (writes.len(), data.len(), data[1]),
            (
                ssd1306::INIT.len() + 2,
                1 + FRAME_BYTES,
                ssd1306::frame_data(&Frame {
                    buf: [0xFF; FRAME_BYTES]
                })[1]
            )
        );
    }

    #[test]
    fn an_unchanged_frame_sends_nothing() {
        // Arrange
        let mut m = Mirror::new();
        m.set_frame(0, Some(&[0x0F; FRAME_BYTES]));
        m.display_writes(0, 0);
        m.set_frame(0, Some(&[0x0F; FRAME_BYTES]));

        // Act
        let writes = m.display_writes(0, 10);

        // Assert
        assert_eq!(writes, [] as [std::vec::Vec<u8>; 0]);
    }

    #[test]
    fn frame_off_switches_the_display_off() {
        // Arrange
        let mut m = Mirror::new();
        m.set_frame(0, Some(&[1; FRAME_BYTES]));
        m.display_writes(0, 0);
        m.set_frame(0, None);

        // Act
        let writes = m.display_writes(0, 10);

        // Assert
        assert_eq!(writes, [vec![0x00, 0xAE]]);
    }

    #[test]
    fn right_screen_is_rendered_from_the_status() {
        // Arrange
        let mut m = Mirror::new();
        m.set_status(&status(), "");
        let mut expected = Frame::default();
        let anim = Animator::new().update(m.status(), 0);
        screens::render_animated(
            &mut expected,
            Half::Right,
            Style::Portrait,
            m.status(),
            corne::KEYMAP.layer_names,
            &anim,
        );

        // Act
        let writes = m.display_writes(1, 0);

        // Assert
        assert_eq!(*writes.last().unwrap(), ssd1306::frame_data(&expected));
    }

    #[test]
    fn right_screen_goes_dark_in_deep_sleep() {
        // Arrange
        let mut m = Mirror::new();
        m.set_status(&status(), "");
        m.display_writes(1, 0);
        m.set_status(
            &MirrorStatus {
                power: 2,
                ..status()
            },
            "",
        );

        // Act
        let writes = m.display_writes(1, 10);

        // Assert
        assert_eq!(writes, [vec![0x00, 0xAE]]);
    }

    #[test]
    fn layer_binding_falls_through_to_the_base_layer() {
        // Arrange: Tab/Alt (12) is transparent on symbols (1)

        // Act
        let (on_symbols, on_base) = (layer_binding(1, 12), layer_binding(0, 12));

        // Assert
        assert_eq!(on_symbols, on_base);
    }

    #[test]
    fn layer_binding_uses_the_layer_when_set() {
        // Arrange: Q (1) is `!` on symbols

        // Act
        let label = crate::action_label(layer_binding(1, 1));

        // Assert
        assert_eq!(label, "!");
    }
}

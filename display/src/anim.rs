//! Screen animations.
//!
//! The screens stay pure functions of what they show: [`Animator`] remembers
//! what changed when (layer, connections, notification) and keeps a short
//! the digital rain drops; it hands [`crate::screens::render_animated`] an [`Anim`]
//! with the time, so any moment of a transition can be drawn (and tested).
//! [`Animator::next_frame`] tells the display task when to draw again; `None`
//! means the screen is still and only needs drawing when the status changes.
//!
//! - Connection: the Bluetooth icon pulses while searching (no computer on
//!   the profile, or the halves not linked), a scanner line sweeps over the
//!   link stage while the halves connect, and the top of the screen flashes
//!   when a computer connects or the halves link.
//! - Layer and caps: the layer name slides in, the CAPS badge blinks gently,
//!   and below them every key press drops a trail of digital rain
//!   (made-up glyphs, never the keys typed); without typing it is empty.
//!   The right half rains too while it has no notification, for the keys
//!   pressed on it (it doesn't see the left half's keys).
//! - Notification: the envelope builds up out of the rain and the text decodes; text longer
//!   than the screen scrolls through and back.

use crate::frame::Half;
use crate::status::{Status, Transport};

/// Redraw interval while something moves (20 frames a second).
pub const FRAME_MS: u32 = 50;
/// One step of the Bluetooth pulse (four steps a cycle).
pub const PULSE_STEP_MS: u32 = 250;
/// Layer name slide.
pub const LAYER_SLIDE_MS: u32 = 240;
/// Flash of the connection area after connecting.
pub const CONNECT_FLASH_MS: u32 = 600;
/// A cleared message falling away.
pub const NOTE_CLEAR_MS: u32 = 900;
/// Notification text decoding out of the rain (up to 11 lines on screen).
pub const NOTE_DECODE_MS: u32 = 1800;
/// A long notification rests this long at the top and the bottom.
pub const SCROLL_PAUSE_MS: u32 = 2000;
/// Scrolling speed of a long notification: one pixel per step.
pub const SCROLL_STEP_MS: u32 = 100;
/// CAPS badge: filled, then outlined, each this long.
pub const CAPS_BLINK_MS: u32 = 600;
/// How long the link stage stays at the bottom once linked (after linking,
/// or after `ShowLink`).
pub const LINK_SHOW_MS: u32 = 4000;
/// How long the radio power shows at the bottom of the left screen after
/// it changed (`RadioPower`).
pub const RADIO_SHOW_MS: u32 = 2500;
/// How long Studio's lock state shows at the bottom of the left screen
/// after it changed (`STUDIO` / `OPEN` or `LOCKED`).
pub const STUDIO_SHOW_MS: u32 = 2500;
/// Low battery: the percentage blinks, this long on and off.
pub const LOW_BATTERY_BLINK_MS: u32 = 500;
/// Low battery: at or below this percentage (and not charging).
pub const LOW_BATTERY_PCT: u8 = 15;
/// How long the sleep picture shows before the screens switch off.
pub const SLEEP_ART_MS: u32 = 3000;
/// Redraw interval of the sleep picture.
pub const SLEEP_FRAME_MS: u32 = 100;
/// Redraw interval while digital rain falls.
pub const RAIN_FRAME_MS: u32 = 60;
/// Drops falling at once; more key presses than this replace the oldest.
pub const RAIN_DROPS: usize = 10;
/// Glyph rows a drop needs to fall out of sight (head and trail), at most.
pub const RAIN_SPAN_ROWS: u32 = 20 + 7;

/// A trail of digital rain dropped by a key press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drop {
    /// When it was dropped.
    pub at: u32,
    /// Picks its column, speed, trail length and glyphs.
    pub seed: u32,
}

impl Drop {
    /// Column 0..8.
    #[must_use]
    pub fn column(&self) -> u32 {
        self.seed % 8
    }

    /// Glyph rows a second: 12 to 23.
    #[must_use]
    pub fn speed(&self) -> u32 {
        12 + (self.seed >> 3) % 12
    }

    /// Glyphs in the trail, the head included: 3 to 7.
    #[must_use]
    pub fn trail(&self) -> u32 {
        3 + (self.seed >> 7) % 5
    }

    /// Row of the head (it starts above the area and falls).
    #[must_use]
    pub fn head(&self, now: u32) -> u32 {
        now.wrapping_sub(self.at) * self.speed() / 1000
    }
}

/// Spread bits (for drop seeds and rain glyphs).
#[must_use]
pub fn mix(mut x: u32) -> u32 {
    x = x.wrapping_mul(0x9E37_79B1);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85EB_CA77);
    x ^ (x >> 13)
}

/// Everything a screen needs to draw a moment of its animations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anim {
    /// False: draw the still picture (no pulse phase, no transitions).
    pub live: bool,
    /// Milliseconds, wrapping.
    pub now: u32,
    /// Layer shown before the last layer change, and when it changed.
    pub layer_from: u8,
    pub layer_at: Option<u32>,
    /// When the selected profile's computer connected.
    pub connected_at: Option<u32>,
    /// When the halves linked.
    pub linked_at: Option<u32>,
    /// When the current notification arrived.
    pub note_at: Option<u32>,
    /// The message that was cleared last, and when (it falls away).
    pub cleared_note: Option<kb_core::host::Text>,
    pub cleared_at: Option<u32>,
    /// When the link stage last changed or was asked for.
    pub link_at: Option<u32>,
    /// When the radio power last changed.
    pub radio_at: Option<u32>,
    /// When Studio was last unlocked or locked.
    pub studio_at: Option<u32>,
    /// Digital rain falling, one drop per recent key press.
    pub drops: [Option<Drop>; RAIN_DROPS],
}

impl Default for Anim {
    fn default() -> Self {
        Self::still()
    }
}

impl Anim {
    /// No animation: the screens look as they always did.
    #[must_use]
    pub const fn still() -> Self {
        Self {
            live: false,
            now: 0,
            layer_from: 0,
            layer_at: None,
            connected_at: None,
            linked_at: None,
            note_at: None,
            cleared_note: None,
            cleared_at: None,
            link_at: None,
            radio_at: None,
            studio_at: None,
            drops: [None; RAIN_DROPS],
        }
    }

    /// Milliseconds since `at`, if that is less than `duration` ago.
    #[must_use]
    pub fn within(&self, at: Option<u32>, duration: u32) -> Option<u32> {
        let elapsed = self.now.wrapping_sub(at?);
        (self.live && elapsed < duration).then_some(elapsed)
    }

    /// Milliseconds since `at` (no limit).
    #[must_use]
    pub fn since(&self, at: Option<u32>) -> Option<u32> {
        at.filter(|_| self.live).map(|t| self.now.wrapping_sub(t))
    }

    /// Step `0..steps` of a cycle of `steps × step_ms`.
    #[must_use]
    pub fn phase(&self, step_ms: u32, steps: u32) -> u32 {
        if self.live {
            (self.now / step_ms) % steps
        } else {
            steps - 1
        }
    }
}

/// Whether `half`'s battery is low (and not charging): its percentage blinks.
#[must_use]
pub fn battery_low(status: &Status, half: usize) -> bool {
    status.battery[half] <= LOW_BATTERY_PCT && !status.charging[half]
}

/// Whether the left half's Bluetooth icon pulses: on Bluetooth with no
/// computer connected on the selected profile.
#[must_use]
pub fn host_searching(status: &Status) -> bool {
    status.transport == Transport::Ble && !status.profile_connected
}

/// Whether the link stage is shown at the bottom of the screens: while the
/// link is on its way or failed, and for [`LINK_SHOW_MS`] once linked (or
/// asked for with `ShowLink`). Still pictures always show it.
#[must_use]
pub fn link_visible(status: &Status, anim: &Anim) -> bool {
    match status.link_detail {
        "" => false,
        "linked" => !anim.live || anim.within(anim.link_at, LINK_SHOW_MS).is_some(),
        _ => true,
    }
}

/// Whether the radio power shows at the bottom of the left screen: for
/// [`RADIO_SHOW_MS`] after it changed. Still pictures don't show it.
#[must_use]
pub fn radio_visible(anim: &Anim) -> bool {
    anim.within(anim.radio_at, RADIO_SHOW_MS).is_some()
}

/// Whether Studio's lock state shows at the bottom of the left screen: for
/// [`STUDIO_SHOW_MS`] after it changed. Still pictures don't show it.
#[must_use]
pub fn studio_visible(anim: &Anim) -> bool {
    anim.within(anim.studio_at, STUDIO_SHOW_MS).is_some()
}

/// Whether the link stage is a step on the way (scanning, connecting,
/// pairing...) rather than a result (`linked`, `disc`, errors with a `-`).
#[must_use]
pub fn link_in_progress(stage: &str) -> bool {
    !stage.is_empty() && stage != "linked" && stage != "disc" && !stage.contains('-')
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Seen {
    layer: u8,
    connected: bool,
    linked: bool,
    notification: Option<kb_core::host::Text>,
    link_detail: &'static str,
    link_requests: u8,
    radio_changes: u8,
    studio_unlocked: bool,
}

impl Seen {
    fn of(status: &Status) -> Self {
        Self {
            layer: status.layer,
            connected: status.transport == Transport::Ble && status.profile_connected,
            linked: status.peripheral_connected,
            notification: status.notification,
            link_detail: status.link_detail,
            link_requests: status.link_requests,
            radio_changes: status.radio_changes,
            studio_unlocked: status.studio_unlocked,
        }
    }
}

/// Follows the status from frame to frame and times the transitions.
#[derive(Clone)]
pub struct Animator {
    anim: Anim,
    seen: Option<Seen>,
    presses: Option<u16>,
}

impl Default for Animator {
    fn default() -> Self {
        Self::new()
    }
}

impl Animator {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            anim: Anim::still(),
            seen: None,
            presses: None,
        }
    }

    /// Note what changed since the last call and return the moment to draw.
    /// The first call only takes the status in (nothing animates in from boot).
    pub fn update(&mut self, status: &Status, now: u32) -> Anim {
        let a = &mut self.anim;
        a.live = true;
        a.now = now;
        let seen = Seen::of(status);
        if let Some(before) = self.seen {
            if seen.layer != before.layer {
                a.layer_from = before.layer;
                // One frame in already, so the change shows on the first frame.
                a.layer_at = Some(now.wrapping_sub(FRAME_MS));
            }
            if seen.connected && !before.connected {
                a.connected_at = Some(now);
            }
            if seen.linked && !before.linked {
                a.linked_at = Some(now);
            }
            if seen.link_detail != before.link_detail || seen.link_requests != before.link_requests
            {
                a.link_at = Some(now);
            }
            if seen.radio_changes != before.radio_changes {
                a.radio_at = Some(now);
            }
            if seen.studio_unlocked != before.studio_unlocked {
                a.studio_at = Some(now);
            }
            if seen.notification != before.notification {
                a.note_at = seen.notification.map(|_| now);
                // Cleared: keep the old message to let it fall away.
                if seen.notification.is_none() {
                    a.cleared_note = before.notification;
                    a.cleared_at = Some(now);
                }
            }
        } else {
            if seen.notification.is_some() {
                a.note_at = Some(now);
            }
            a.link_at = Some(now);
        }
        self.seen = Some(seen);

        // Digital rain: a drop per new key press (a few at most per frame).
        let pressed = self
            .presses
            .map_or(0, |p| status.key_presses.wrapping_sub(p));
        self.presses = Some(status.key_presses);
        for d in &mut a.drops {
            if d.is_some_and(|d| d.head(now) >= RAIN_SPAN_ROWS) {
                *d = None;
            }
        }
        for i in 0..pressed.min(4) {
            let seed = mix(now ^ u32::from(status.key_presses.wrapping_sub(i)) << 16);
            let drop = Drop { at: now, seed };
            // A free slot, or the oldest drop.
            let slot = a.drops.iter().position(Option::is_none).unwrap_or_else(|| {
                (0..RAIN_DROPS)
                    .min_by_key(|&k| {
                        a.drops[k]
                            .map_or(0, |d| now.wrapping_sub(d.at))
                            .wrapping_neg()
                    })
                    .unwrap_or(0)
            });
            a.drops[slot] = Some(drop);
        }
        *a
    }

    /// Milliseconds until the screen of `half` changes by itself, or `None`
    /// when it is still.
    /// While the start-up animation plays: the halves linking is what the
    /// start-up itself shows, so the `linked` stage doesn't show again once
    /// the status screen is revealed. Stages on the way and errors still do.
    pub fn starting_up(&mut self) -> Anim {
        self.anim.link_at = None;
        self.anim
    }

    pub fn next_frame(&self, half: Half, status: &Status) -> Option<u32> {
        let a = &self.anim;
        let mut next: Option<u32> = None;
        let mut want = |ms: u32| next = Some(next.map_or(ms, |n| n.min(ms)));

        let flash_at = match half {
            Half::Left => a.connected_at,
            Half::Right => a.linked_at,
        };
        if a.within(flash_at, CONNECT_FLASH_MS).is_some() {
            want(FRAME_MS);
        }
        if link_in_progress(status.link_detail) {
            want(FRAME_MS);
        }
        // Redraw when the link stage hides itself.
        if status.link_detail == "linked"
            && let Some(elapsed) = a.within(a.link_at, LINK_SHOW_MS)
        {
            want(LINK_SHOW_MS - elapsed);
        }
        let index = match half {
            Half::Left => 0,
            Half::Right => 1,
        };
        if battery_low(status, index) {
            want(LOW_BATTERY_BLINK_MS - a.now % LOW_BATTERY_BLINK_MS);
        }
        match half {
            Half::Left => {
                if a.within(a.layer_at, LAYER_SLIDE_MS).is_some() {
                    want(FRAME_MS);
                }
                // Redraw when the radio power hides itself.
                if let Some(elapsed) = a.within(a.radio_at, RADIO_SHOW_MS) {
                    want(RADIO_SHOW_MS - elapsed);
                }
                // ... and when Studio's lock state does.
                if let Some(elapsed) = a.within(a.studio_at, STUDIO_SHOW_MS) {
                    want(STUDIO_SHOW_MS - elapsed);
                }
                if host_searching(status) {
                    want(PULSE_STEP_MS - a.now % PULSE_STEP_MS);
                }
                if status.caps_word {
                    want(CAPS_BLINK_MS - a.now % CAPS_BLINK_MS);
                }
                if a.drops.iter().any(Option::is_some) {
                    want(RAIN_FRAME_MS);
                }
            }
            Half::Right => {
                if !status.peripheral_connected {
                    want(PULSE_STEP_MS - a.now % PULSE_STEP_MS);
                }
                if status.notification.is_none() && a.within(a.cleared_at, NOTE_CLEAR_MS).is_some()
                {
                    want(FRAME_MS);
                }
                if status.notification.is_none() && a.drops.iter().any(Option::is_some) {
                    want(RAIN_FRAME_MS);
                }
                if status.notification.is_some() {
                    if a.within(a.note_at, NOTE_DECODE_MS).is_some() {
                        want(FRAME_MS);
                    } else if crate::screens::notification_overflow(status, a) > 0 {
                        want(SCROLL_STEP_MS);
                    }
                }
            }
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status::default()
    }

    #[test]
    fn first_update_starts_no_transition() {
        // Arrange
        let mut animator = Animator::new();

        // Act
        let anim = animator.update(&status(), 1000);

        // Assert
        assert_eq!(
            (anim.layer_at, anim.connected_at, anim.linked_at),
            (None, None, None)
        );
    }

    #[test]
    fn layer_change_is_timed_from_the_old_layer() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);

        // Act
        let anim = animator.update(
            &Status {
                layer: 2,
                ..status()
            },
            1050,
        );

        // Assert
        assert_eq!((anim.layer_from, anim.layer_at), (0, Some(1050 - FRAME_MS)));
    }

    #[test]
    fn computer_connecting_starts_the_flash() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(
            &Status {
                profile_connected: false,
                ..status()
            },
            1000,
        );

        // Act
        let anim = animator.update(&status(), 1200);

        // Assert
        assert_eq!(anim.connected_at, Some(1200));
    }

    #[test]
    fn halves_linking_starts_the_flash() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(
            &Status {
                peripheral_connected: false,
                ..status()
            },
            1000,
        );

        // Act
        let anim = animator.update(&status(), 1300);

        // Assert
        assert_eq!(anim.linked_at, Some(1300));
    }

    #[test]
    fn new_notification_is_timed() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        let note = Status {
            notification: Some(kb_core::host::Text::new("hi")),
            ..status()
        };

        // Act
        let anim = animator.update(&note, 1100);

        // Assert
        assert_eq!(anim.note_at, Some(1100));
    }

    #[test]
    fn key_press_drops_rain() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        let typed = Status {
            key_presses: 1,
            ..status()
        };

        // Act
        let anim = animator.update(&typed, 1050);

        // Assert
        assert_eq!(anim.drops.iter().flatten().count(), 1);
    }

    #[test]
    fn rain_drops_fall_out_of_sight() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        let typed = Status {
            key_presses: 1,
            ..status()
        };
        animator.update(&typed, 1000);

        // Act: slowest drop, 12 rows a second
        let anim = animator.update(&typed, 1000 + RAIN_SPAN_ROWS * 1000 / 12 + 1);

        // Assert
        assert_eq!(anim.drops.iter().flatten().count(), 0);
    }

    #[test]
    fn still_screen_needs_no_frames() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);

        // Act
        let next = animator.next_frame(Half::Left, &status());

        // Assert
        assert_eq!(next, None);
    }

    #[test]
    fn searching_for_the_left_half_pulses() {
        // Arrange
        let mut animator = Animator::new();
        let searching = Status {
            peripheral_connected: false,
            ..status()
        };
        animator.update(&searching, 1010);

        // Act
        let next = animator.next_frame(Half::Right, &searching);

        // Assert: the next pulse step
        assert_eq!(next, Some(PULSE_STEP_MS - 10));
    }

    #[test]
    fn falling_rain_needs_frames() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        animator.update(
            &Status {
                key_presses: 1,
                ..status()
            },
            1050,
        );

        // Act
        let next = animator.next_frame(Half::Left, &status());

        // Assert
        assert_eq!(next, Some(RAIN_FRAME_MS));
    }

    #[test]
    fn layer_slide_runs_at_the_frame_rate() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        let nav = Status {
            layer: 2,
            ..status()
        };
        animator.update(&nav, 1000);

        // Act
        let next = animator.next_frame(Half::Left, &nav);

        // Assert
        assert_eq!(next, Some(FRAME_MS));
    }

    fn linked() -> Status {
        Status {
            link_detail: "linked",
            ..status()
        }
    }

    #[test]
    fn linked_stage_hides_itself_after_a_while() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(
            &Status {
                link_detail: "sub",
                ..status()
            },
            1000,
        );
        animator.update(&linked(), 2000);

        // Act
        let anim = animator.update(&linked(), 2000 + LINK_SHOW_MS);

        // Assert
        assert!(!link_visible(&linked(), &anim));
    }

    #[test]
    fn linked_stage_shows_right_after_linking() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(
            &Status {
                link_detail: "sub",
                ..status()
            },
            1000,
        );

        // Act
        let anim = animator.update(&linked(), 2000);

        // Assert
        assert!(link_visible(&linked(), &anim));
    }

    #[test]
    fn linked_during_start_up_is_not_shown_after_it() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(
            &Status {
                link_detail: "sub",
                ..status()
            },
            1000,
        );
        animator.update(&linked(), 2000);
        animator.starting_up();

        // Act
        let anim = animator.update(&linked(), 2500);

        // Assert
        assert!(!link_visible(&linked(), &anim));
    }

    #[test]
    fn link_error_during_start_up_still_shows() {
        // Arrange
        let failed = Status {
            link_detail: "sec-fail",
            ..status()
        };
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        animator.update(&failed, 2000);
        animator.starting_up();

        // Act
        let anim = animator.update(&failed, 2500);

        // Assert
        assert!(link_visible(&failed, &anim));
    }

    #[test]
    fn show_link_brings_the_linked_stage_back() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&linked(), 0);
        animator.update(&linked(), LINK_SHOW_MS + 1000);
        let mut asked = linked();
        asked.apply(kb_core::SystemAction::ShowLink);

        // Act
        let anim = animator.update(&asked, LINK_SHOW_MS + 2000);

        // Assert
        assert!(link_visible(&asked, &anim));
    }

    fn studio_unlocked() -> Status {
        Status {
            studio_unlocked: true,
            ..status()
        }
    }

    #[test]
    fn studio_unlock_shows_for_a_moment() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);

        // Act
        let anim = animator.update(&studio_unlocked(), 2000);

        // Assert
        assert!(studio_visible(&anim));
    }

    #[test]
    fn studio_lock_state_hides_itself_after_a_while() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        animator.update(&studio_unlocked(), 2000);

        // Act
        let anim = animator.update(&studio_unlocked(), 2000 + STUDIO_SHOW_MS);

        // Assert
        assert!(!studio_visible(&anim));
    }

    #[test]
    fn studio_locking_again_shows_too() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        animator.update(&studio_unlocked(), 2000);
        animator.update(&studio_unlocked(), 2000 + STUDIO_SHOW_MS);

        // Act
        let anim = animator.update(&status(), 10_000);

        // Assert
        assert!(studio_visible(&anim));
    }

    #[test]
    fn hiding_the_studio_state_needs_a_frame() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        animator.update(&studio_unlocked(), 1000);

        // Act
        let next = animator.next_frame(Half::Left, &studio_unlocked());

        // Assert
        assert_eq!(next, Some(STUDIO_SHOW_MS));
    }

    fn radio_changed(status: &Status) -> Status {
        let mut changed = *status;
        changed.apply(kb_core::SystemAction::RadioPower);
        changed
    }

    #[test]
    fn radio_power_shows_after_it_changed() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);

        // Act
        let anim = animator.update(&radio_changed(&status()), 2000);

        // Assert
        assert!(radio_visible(&anim));
    }

    #[test]
    fn radio_power_hides_itself_after_a_while() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        let changed = radio_changed(&status());
        animator.update(&changed, 2000);

        // Act
        let anim = animator.update(&changed, 2000 + RADIO_SHOW_MS);

        // Assert
        assert!(!radio_visible(&anim));
    }

    #[test]
    fn radio_power_is_not_shown_at_boot() {
        // Arrange
        let mut animator = Animator::new();

        // Act
        let anim = animator.update(&radio_changed(&status()), 1000);

        // Assert
        assert!(!radio_visible(&anim));
    }

    #[test]
    fn hiding_the_radio_power_needs_a_frame() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&status(), 1000);
        let changed = radio_changed(&status());
        animator.update(&changed, 1000);

        // Act
        let next = animator.next_frame(Half::Left, &changed);

        // Assert
        assert_eq!(next, Some(RADIO_SHOW_MS));
    }

    #[test]
    fn failed_link_stage_stays() {
        // Arrange
        let mut animator = Animator::new();
        let failed = Status {
            link_detail: "c-ctl",
            ..status()
        };
        animator.update(&failed, 0);

        // Act
        let anim = animator.update(&failed, 60_000);

        // Assert
        assert!(link_visible(&failed, &anim));
    }

    #[test]
    fn hiding_the_linked_stage_needs_a_frame() {
        // Arrange
        let mut animator = Animator::new();
        animator.update(&linked(), 1000);

        // Act
        let next = animator.next_frame(Half::Right, &linked());

        // Assert
        assert_eq!(next, Some(LINK_SHOW_MS));
    }

    #[test]
    fn link_stage_progress_is_told_from_results() {
        // Arrange
        let stages = ["scan", "pair", "linked", "disc", "c-ctl", ""];

        // Act
        let progress = stages.map(link_in_progress);

        // Assert
        assert_eq!(progress, [true, true, false, false, false, false]);
    }

    #[test]
    fn still_anim_reports_no_transition() {
        // Arrange
        let anim = Anim {
            layer_at: Some(0),
            ..Anim::still()
        };

        // Act
        let elapsed = anim.within(anim.layer_at, LAYER_SLIDE_MS);

        // Assert
        assert_eq!(elapsed, None);
    }

    #[test]
    fn low_battery_needs_frames_to_blink() {
        // Arrange
        let mut animator = Animator::new();
        let low = Status {
            battery: [100, 12],
            ..status()
        };
        animator.update(&low, 1100);

        // Act
        let next = animator.next_frame(Half::Right, &low);

        // Assert
        assert_eq!(next, Some(LOW_BATTERY_BLINK_MS - 100));
    }
}

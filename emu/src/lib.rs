//! The keyboard's firmware logic behind a C ABI, for the .NET emulator.
//!
//! Everything a real Corne would do minus the hardware: the keymap engine,
//! Bluetooth profile / output state, WPM, and the OLED screens rendered all
//! the way down to SSD1306 I²C transactions.
//!
//! The caller owns time (milliseconds since start) and must call
//! [`kb_emu_tick`] regularly, ideally at [`kb_emu_next_deadline`].

// The kb_emu_* functions are the C boundary: pointers come from the .NET
// side, are null-checked, and must otherwise be valid as documented.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

pub mod bt;
pub mod mirror;
pub mod studio;

use std::collections::VecDeque;

use kb_core::host::{HostMessage, Reassembler, SPLIT_MAX, SplitMessage};
use kb_core::keymaps::corne;
use kb_core::keys::{
    self, MOD_LALT, MOD_LCTL, MOD_LGUI, MOD_LSFT, MOD_RALT, MOD_RCTL, MOD_RGUI, MOD_RSFT,
};
use kb_core::mouse::{Mouse, MouseReport};
use kb_core::power::{Power, PowerConfig, PowerState};
use kb_core::{
    Action, Config, Direction, Engine, Key, MouseAction, MouseButton, Output, Report, SystemAction,
};
use kb_display::anim::Animator;
use kb_display::frame::{Frame, Half};
use kb_display::screens::Style;
use kb_display::status::{Status, Transport, Wpm};

use bt::{LINK_STAGES, Profiles};
use kb_display::{screens, ssd1306};

pub const KEYS: usize = corne::KEYS;

/// How long a notification stays on the right screen (as the firmware's).
pub const NOTIFICATION_MS: u32 = kb_core::host::NOTIFICATION_MS;

/// What happened to a Raw HID report from the computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RawHidResult {
    /// Not a complete message yet (or ignored).
    Pending = 0,
    /// Forwarded over the split link and shown on the right half.
    Delivered = 1,
    /// The right half is not linked; the message was dropped.
    NotLinked = 2,
    /// Notification cleared.
    Cleared = 3,
    /// The keyboard is in deep sleep (radio off); nothing arrives.
    Asleep = 4,
}

/// How long a pairing prompt waits for the code (as the firmware's
/// `Timing::FIRMWARE.pair_timeout`).
pub const PAIR_TIMEOUT_MS: u32 = 60_000;

/// How a simulated pairing ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PairResult {
    /// The code typed matched: the computer is paired on the profile.
    Paired = 0,
    /// Another code was typed: the pairing failed.
    WrongCode = 1,
    /// Escape on the keyboard.
    Rejected = 2,
    /// Nothing typed in time.
    TimedOut = 3,
}

/// Something the emulated firmware did that the UI may show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Output(Output),
    Power(PowerState),
    Pairing(PairResult),
    /// A mouse report for the computer (at the output's pace: every ms of
    /// motion on USB, every 8 ms on Bluetooth).
    Mouse(MouseReport),
}

/// Where the mouse reports so far have taken the pointer (summed), for the
/// app to draw it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseTotals {
    pub x: i32,
    pub y: i32,
    pub wheel: i32,
    pub pan: i32,
    pub buttons: u8,
}

/// A computer pairing with a code (passkey entry), simulated.
#[derive(Clone, Copy, Debug)]
struct Pairing {
    /// The code the computer shows.
    code: u32,
    until: u32,
}

pub struct Emu {
    engine: Engine<'static, KEYS>,
    status: Status,
    wpm: Wpm,
    host: Report,
    /// The engine's last report (before the pairing prompt takes keys out).
    engine_report: Report,
    /// Last frame sent to each display; `None` until the display is initialized.
    shown: [Option<Frame>; 2],
    style: Style,
    raw_hid: Reassembler,
    notification_until: Option<u32>,
    power: Power,
    power_events: VecDeque<PowerState>,
    /// Whether each display is switched on (`AF`) as far as the firmware knows.
    lit: [bool; 2],
    /// Animation timing per display.
    animators: [Animator; 2],
    /// Key presses on the right half, as its own firmware counts them.
    right_presses: u16,
    /// When each display started showing the sleep picture.
    idle_at: [Option<u32>; 2],
    /// When each display started its start-up animation (first frame), and
    /// whether one is due (`kb_emu_replay_boot`; the app plays it on start).
    boot_at: [Option<u32>; 2],
    boot_due: [bool; 2],
    /// Each half's power switch.
    half_on: [bool; 2],
    /// When the halves link again after a half (re)started.
    relink_at: Option<u32>,
    /// When the halves last linked (the start-up animation waits for it).
    linked_at: Option<u32>,
    /// Bluetooth profiles; what the selected one looks like is mirrored in `status`.
    bt: Profiles,
    /// A simulated computer pairing with a code, and the key loop's prompt.
    pairing: Option<Pairing>,
    pair_prompt: kb_core::link::PairPrompt,
    pair_events: VecDeque<PairResult>,
    /// Studio: the editable keymap (kept across emulated restarts, like flash).
    pub studio: studio::EmuStudio,
    /// Studio's generation the engine has.
    studio_generation: u32,
    /// Mouse keys, as the firmware's key loop runs them.
    mouse: Mouse,
    mouse_reports: VecDeque<MouseReport>,
    pub mouse_totals: MouseTotals,
    /// The time of the last tick (the mouse needs one every ms while it moves).
    last_now: u32,
}

impl Default for Emu {
    fn default() -> Self {
        Self::new()
    }
}

impl Emu {
    #[must_use]
    pub fn new() -> Self {
        Self {
            engine: Engine::new(&corne::KEYMAP, Config::default()),
            status: Status::default(),
            wpm: Wpm::new(),
            host: Report::default(),
            engine_report: Report::default(),
            shown: [None, None],
            style: Style::Portrait,
            raw_hid: Reassembler::new(),
            notification_until: None,
            power: Power::new(PowerConfig::ZMK_CONFIG, 0),
            power_events: VecDeque::new(),
            lit: [false, false],
            animators: [Animator::new(), Animator::new()],
            right_presses: 0,
            idle_at: [None, None],
            boot_at: [None, None],
            boot_due: [false, false],
            half_on: [true, true],
            relink_at: None,
            linked_at: None,
            bt: Profiles::default(),
            pairing: None,
            pair_prompt: kb_core::link::PairPrompt::new(),
            pair_events: VecDeque::new(),
            studio: studio::EmuStudio::new(),
            studio_generation: 0,
            mouse: Mouse::default(),
            mouse_reports: VecDeque::new(),
            mouse_totals: MouseTotals::default(),
            last_now: 0,
        }
    }

    /// Mouse motion up to `now`, and a report when the output takes one.
    fn mouse_tick(&mut self, now: u32) {
        self.mouse.tick(now);
        let interval = match self.status.transport {
            Transport::Usb => 0,
            Transport::Ble => self.mouse.config().ble_interval_ms,
        };
        if let Some(r) = self.mouse.take_report(now, interval) {
            let t = &mut self.mouse_totals;
            t.x += i32::from(r.x);
            t.y += i32::from(r.y);
            t.wheel += i32::from(r.wheel);
            t.pan += i32::from(r.pan);
            t.buttons = r.buttons;
            self.mouse_reports.push_back(r);
        }
    }

    /// Give the engine Studio's bindings when they changed (as the
    /// firmware's key loop does before every scan).
    pub fn sync_studio(&mut self) {
        let generation = self.studio.studio.generation();
        if generation != self.studio_generation {
            self.studio_generation = generation;
            self.engine.set_layers(self.studio.studio.layers());
            self.engine.set_combos(self.studio.studio.combos().defs());
        }
    }

    /// Studio's lock as the left screen shows it.
    fn studio_status(&mut self, now: u32) {
        self.studio.studio.expire(now);
        self.status.studio_unlocked = self.studio.studio.unlocked(now);
    }

    /// A computer starts pairing with a code on the selected profile, as
    /// with the firmware's passkey entry: the computer shows `code` (6
    /// digits), the left screen asks for it; typing it and Enter pairs,
    /// Escape or another code doesn't, and after [`PAIR_TIMEOUT_MS`] it's
    /// rejected. Returns the code.
    pub fn simulate_pairing(&mut self, code: u32, now: u32) -> u32 {
        let code = code % 1_000_000;
        self.pairing = Some(Pairing {
            code,
            until: now.wrapping_add(PAIR_TIMEOUT_MS),
        });
        self.activity(now);
        self.refilter();
        code
    }

    /// The prompt for the current pairing.
    fn request(&self) -> Option<kb_core::link::PairRequest> {
        self.pairing.map(|_| kb_core::link::PairRequest::TypeCode)
    }

    /// The prompt came or went: run the last report through it again.
    fn refilter(&mut self) {
        let request = self.request();
        let (_, _) = self.pair_prompt.filter(request, &self.engine_report);
        self.status.pairing = kb_display::status::Pairing::of(request, &self.pair_prompt);
    }

    fn end_pairing(&mut self, result: PairResult) {
        self.pairing = None;
        if result == PairResult::Paired {
            self.bt.set_connected(true);
            self.bt.show(&mut self.status);
        }
        self.pair_events.push_back(result);
        self.refilter();
    }

    pub fn press(&mut self, pos: u8, now: u32) {
        // A switched-off half has no keys; without the left half nothing works.
        if !self.half_on[0] || (on_right_half(pos) && !self.half_on[1]) {
            return;
        }
        if self.power.state() == PowerState::Sleep {
            // The chip wakes from System OFF by rebooting; this key is lost.
            self.reboot(now);
            return;
        }
        self.activity(now);
        if on_right_half(pos) {
            // The right half only sees its own keys (digital rain on its screen).
            self.right_presses = self.right_presses.wrapping_add(1);
        }
        self.sync_studio();
        self.engine.press(pos, now);
    }

    pub fn release(&mut self, pos: u8, now: u32) {
        if !self.half_on[0] || (on_right_half(pos) && !self.half_on[1]) {
            return;
        }
        if self.power.state() == PowerState::Sleep {
            return;
        }
        self.activity(now);
        self.engine.release(pos, now);
    }

    #[must_use]
    pub fn power_state(&self) -> PowerState {
        self.power.state()
    }

    pub fn set_power_config(&mut self, config: PowerConfig, now: u32) {
        if let Some(state) = self.power.set_config(config, now) {
            self.power_events.push_back(state);
        }
    }

    fn activity(&mut self, now: u32) {
        if let Some(state) = self.power.activity(now) {
            self.power_events.push_back(state);
        }
    }

    /// Wake from deep sleep: like a reset. Settings stored in flash (BT
    /// profile, output) survive; everything else starts fresh.
    fn reboot(&mut self, now: u32) {
        let kept = Status {
            transport: self.status.transport,
            radio_dbm: self.status.radio_dbm,
            battery: self.status.battery,
            peripheral_connected: self.status.peripheral_connected,
            link_detail: self.status.link_detail,
            ..Status::default()
        };
        let (style, config, bt) = (self.style, self.power.config(), self.bt);
        // Studio's saved keymap is in flash; unsaved edits and the unlock aren't.
        let mut studio = std::mem::take(&mut self.studio);
        studio.studio.restart();
        *self = Self::new();
        self.studio = studio;
        self.studio_generation = self.studio.studio.generation().wrapping_sub(1);
        self.sync_studio();
        self.status = kept;
        self.bt = bt;
        self.bt.show(&mut self.status);
        self.style = style;
        self.power = Power::new(config, now);
        self.power_events.push_back(PowerState::Active);
    }

    /// A half's power switch. Switching on starts it like the firmware: the
    /// start-up animation, then the halves link again.
    pub fn set_half_power(&mut self, half: usize, on: bool, now: u32) {
        let half = half.min(1);
        if self.half_on[half] == on {
            return;
        }
        if half == 0 && on {
            self.reboot(now);
        }
        self.half_on[half] = on;
        self.unlink(now);
        if on {
            self.boot_due[half] = true;
        }
    }

    /// The reset button of a half: it restarts, the halves link again.
    pub fn reset_half(&mut self, half: usize, now: u32) {
        let half = half.min(1);
        if !self.half_on[half] {
            return;
        }
        if half == 0 {
            self.reboot(now);
        }
        self.unlink(now);
        self.boot_due[half] = true;
    }

    /// The link between the halves drops; it comes back after the start-up
    /// if both halves are on.
    fn unlink(&mut self, now: u32) {
        if self.status.peripheral_connected {
            // The left half lets go of whatever the right half held.
            for pos in (0..42).filter(|&p| on_right_half(p)) {
                self.engine.release(pos, now);
            }
        }
        self.status.peripheral_connected = false;
        self.relink_at = (self.half_on == [true, true]).then(|| now.wrapping_add(RELINK_MS));
    }

    pub fn tick(&mut self, now: u32) {
        if self
            .relink_at
            .is_some_and(|t| now.wrapping_sub(t) as i32 >= 0)
        {
            self.relink_at = None;
            self.status.peripheral_connected = true;
            self.linked_at = Some(now);
        }
        self.last_now = now;
        self.engine.tick(now);
        self.mouse_tick(now);
        self.studio_status(now);
        self.wpm.advance(now);
        if self
            .notification_until
            .is_some_and(|t| now.wrapping_sub(t) as i32 >= 0)
        {
            self.notification_until = None;
            self.status.notification = None;
        }
        if let Some(state) = self.power.tick(now) {
            self.power_events.push_back(state);
        }
        if self
            .pairing
            .is_some_and(|p| now.wrapping_sub(p.until) as i32 >= 0)
        {
            self.end_pairing(PairResult::TimedOut);
        }
    }

    /// A Raw HID report from the computer arrives at the left (central)
    /// half. Complete messages go over the split link to the right half.
    pub fn raw_hid(&mut self, report: &[u8], now: u32) -> RawHidResult {
        if self.power.state() == PowerState::Sleep {
            return RawHidResult::Asleep;
        }
        let Some(message) = self.raw_hid.feed(report) else {
            return RawHidResult::Pending;
        };
        let split = match message {
            HostMessage::Notify(text) => SplitMessage::Notify(text),
            HostMessage::Clear => SplitMessage::Clear,
        };
        if !self.status.peripheral_connected {
            return RawHidResult::NotLinked;
        }
        // Central encodes, the bytes cross the (simulated) split link,
        // the peripheral decodes them.
        let mut link = [0u8; SPLIT_MAX];
        let len = split.encode(&mut link);
        self.peripheral_receive(&link[..len], now)
    }

    fn peripheral_receive(&mut self, bytes: &[u8], now: u32) -> RawHidResult {
        match SplitMessage::decode(bytes) {
            Some(SplitMessage::Notify(text)) => {
                self.status.notification = Some(text);
                self.notification_until = Some(now.wrapping_add(NOTIFICATION_MS));
                // A notification wakes the screens so it can be read.
                self.activity(now);
                RawHidResult::Delivered
            }
            Some(SplitMessage::Clear) => {
                self.status.notification = None;
                self.notification_until = None;
                RawHidResult::Cleared
            }
            None => RawHidResult::Pending,
        }
    }

    /// Earliest time [`tick`](Self::tick) must run (key timers, idle/sleep, notification expiry).
    #[must_use]
    pub fn next_deadline(&self) -> Option<u32> {
        [
            self.engine.next_deadline(),
            self.power.next_deadline(),
            self.notification_until,
            self.pairing.map(|p| p.until),
            // The mouse moves in 1 ms steps.
            self.mouse.active().then(|| self.last_now.wrapping_add(1)),
        ]
        .into_iter()
        .flatten()
        .reduce(|a, b| if (a.wrapping_sub(b) as i32) < 0 { a } else { b })
    }

    /// Next event: power changes first, then engine outputs (after updating
    /// the firmware state they affect).
    pub fn poll(&mut self, now: u32) -> Option<Event> {
        if let Some(state) = self.power_events.pop_front() {
            return Some(Event::Power(state));
        }
        if let Some(result) = self.pair_events.pop_front() {
            return Some(Event::Pairing(result));
        }
        if let Some(report) = self.mouse_reports.pop_front() {
            return Some(Event::Mouse(report));
        }
        let mut out = self.engine.poll()?;
        if let Output::Report(r) = out {
            // A pairing prompt takes the keys: nothing reaches the computer.
            self.engine_report = r;
            let (filtered, reply) = self.pair_prompt.filter(self.request(), &r);
            if let (Some(reply), Some(pairing)) = (reply, self.pairing) {
                use kb_core::link::PairReply;
                self.end_pairing(match reply {
                    PairReply::Passkey(code) if code == pairing.code => PairResult::Paired,
                    PairReply::Passkey(_) | PairReply::Confirm => PairResult::WrongCode,
                    PairReply::Reject => PairResult::Rejected,
                });
            }
            self.status.pairing =
                kb_display::status::Pairing::of(self.request(), &self.pair_prompt);
            out = Output::Report(filtered);
        }
        match out {
            Output::Report(r) => {
                for code in r.keys() {
                    if !self.host.contains(code) {
                        self.wpm.key_pressed(now);
                        self.status.key_presses = self.status.key_presses.wrapping_add(1);
                    }
                }
                self.host = r;
                self.status.mods = r.mods;
            }
            Output::Layer(l) => self.status.layer = l,
            Output::CapsWord(on) => self.status.caps_word = on,
            Output::System(s) => self.system(s, now),
            Output::Mouse(action, true) => {
                self.mouse.press(action, now);
                self.mouse_tick(now);
            }
            Output::Mouse(action, false) => {
                self.mouse.release(action, now);
                self.mouse_tick(now);
            }
            Output::Pointer(on) => self.status.pointer_active = on,
        }
        Some(Event::Output(out))
    }

    /// `&bt` / `&out` the way the firmware handles them.
    fn system(&mut self, action: SystemAction, now: u32) {
        match action {
            // Profile ◀ / ▶ step through BT 1-10 and USB as one ring.
            SystemAction::BtNext | SystemAction::BtPrev => {
                use kb_core::link::Slot;
                let n = kb_display::status::BT_PROFILES as usize;
                let here = match self.status.transport {
                    Transport::Usb => Slot::Usb,
                    Transport::Ble => Slot::Ble(self.bt.active() as usize),
                };
                let there = if action == SystemAction::BtNext {
                    here.next(n)
                } else {
                    here.prev(n)
                };
                match there {
                    Slot::Usb => self.status.transport = Transport::Usb,
                    Slot::Ble(i) => {
                        self.status.transport = Transport::Ble;
                        self.bt.select(i as u8);
                    }
                }
            }
            // On the USB slot there is no computer to forget.
            SystemAction::BtClear => {
                if self.status.transport == Transport::Ble {
                    self.bt.clear();
                }
            }
            SystemAction::StudioUnlock => {
                self.studio.studio.unlock(now);
                self.studio_status(now);
            }
            SystemAction::OutToggle | SystemAction::ShowLink | SystemAction::RadioPower => {
                self.status.apply(action);
            }
        }
        self.bt.show(&mut self.status);
    }

    /// Select Bluetooth profile `profile` (0-based; out of range is ignored).
    pub fn select_profile(&mut self, profile: u8) {
        self.bt.select(profile);
        self.bt.show(&mut self.status);
    }

    /// Mark the selected profile open (no computer paired) or taken.
    pub fn set_profile_open(&mut self, open: bool) {
        self.bt.set_open(open);
        self.bt.show(&mut self.status);
    }

    /// Whether the selected profile's computer is connected.
    pub fn set_profile_connected(&mut self, connected: bool) {
        self.bt.set_connected(connected);
        self.bt.show(&mut self.status);
    }

    pub fn set_transport(&mut self, transport: Transport) {
        self.status.transport = transport;
    }

    /// Split link stage by index into [`LINK_STAGES`]; out of range hides it.
    pub fn set_link_stage(&mut self, index: usize) {
        self.status.link_detail = LINK_STAGES.get(index).copied().unwrap_or("");
    }

    #[must_use]
    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn set_style(&mut self, style: Style) {
        self.style = style;
    }

    pub fn status_mut(&mut self) -> &mut Status {
        &mut self.status
    }

    /// Key legend for `pos` on the current layers (two lines for hold-taps).
    #[must_use]
    pub fn label(&self, pos: u8) -> String {
        action_label(self.engine.binding(pos))
    }

    #[must_use]
    pub fn layer_name(&self, layer: u8) -> &'static str {
        corne::KEYMAP
            .layer_names
            .get(layer as usize)
            .copied()
            .unwrap_or("?")
    }

    /// I²C transactions to bring display `half` (0 = left, 1 = right) up to
    /// date: the init sequence the first time, then a frame when it changed.
    /// Idle switches the display off (`AE`, like ZMK's blanking); deep sleep
    /// cuts its power, so it needs the full init again afterwards.
    pub fn display_writes(&mut self, half: usize, now: u32) -> Vec<Vec<u8>> {
        let half = half.min(1);
        let mut writes = Vec::new();
        // Switched off: no power, no screen.
        if !self.half_on[half] {
            if self.shown[half].is_some() || self.lit[half] {
                ssd1306::display_off(|w| writes.push(w.to_vec()));
            }
            self.shown[half] = None;
            self.lit[half] = false;
            self.boot_at[half] = None;
            return writes;
        }
        match self.power.state() {
            PowerState::Sleep => {
                self.shown[half] = None;
                self.lit[half] = false;
                self.idle_at[half] = None;
                return writes;
            }
            PowerState::Idle => {
                self.idle_writes(half, now, &mut writes);
                return writes;
            }
            PowerState::Active => self.idle_at[half] = None,
        }

        let frame = self.active_frame(half, now);
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

    /// Display `half` while the keyboard rests: the sleep picture for a
    /// moment (as the firmware), then off.
    fn idle_writes(&mut self, half: usize, now: u32, writes: &mut Vec<Vec<u8>>) {
        if !self.lit[half] {
            return;
        }
        // The sleep picture for a moment, then off (as the firmware).
        let elapsed = now.wrapping_sub(*self.idle_at[half].get_or_insert(now));
        if elapsed < kb_display::anim::SLEEP_ART_MS {
            let side = if half == 0 { Half::Left } else { Half::Right };
            let mut frame = Frame::default();
            let mut from = Frame::default();
            screens::render(
                &mut from,
                side,
                self.style,
                &self.status,
                corne::KEYMAP.layer_names,
            );
            screens::sleep_art(&mut frame, side, elapsed, &from);
            if self.shown[half].as_ref() != Some(&frame) {
                ssd1306::flush(&frame, |w| writes.push(w.to_vec()));
                self.shown[half] = Some(frame);
            }
            return;
        }
        self.lit[half] = false;
        ssd1306::display_off(|w| writes.push(w.to_vec()));
    }

    /// The frame display `half` shows at `now` while awake: its status
    /// screen, animated, with the start-up animation over it at first.
    fn active_frame(&mut self, half: usize, now: u32) -> Frame {
        self.wpm.advance(now);
        self.status.wpm = self.wpm.wpm();
        self.studio_status(now);
        let side = if half == 0 { Half::Left } else { Half::Right };
        // Each half has its own status: the right half counts its own keys,
        // and the left half's over the link while linked.
        let mut status = self.status;
        if side == Half::Right {
            status.key_presses = if status.peripheral_connected {
                self.status.key_presses
            } else {
                self.right_presses
            };
        }
        let mut anim = self.animators[half].update(&status, now);
        if self.boot_due[half] || self.boot_at[half].is_some() {
            anim = self.animators[half].starting_up();
        }
        let mut frame = Frame::default();
        screens::render_animated(
            &mut frame,
            side,
            self.style,
            &status,
            corne::KEYMAP.layer_names,
            &anim,
        );
        // Start-up animation first, ending on this status screen.
        if self.boot_due[half] {
            self.boot_due[half] = false;
            self.boot_at[half] = Some(now);
        }
        // Rain until the halves are linked, then the rest on both together.
        if let Some(start) = self.boot_at[half].filter(|_| self.style == Style::Portrait) {
            let since = now.wrapping_sub(start);
            // Linked before this half started: as if at power-on.
            let since_linked = self.status.peripheral_connected.then(|| {
                self.linked_at
                    .map_or(since, |t| now.wrapping_sub(t).min(since))
            });
            if screens::boot_done(since, since_linked) {
                self.boot_at[half] = None;
            } else {
                let initial = if half == 0 { "E" } else { "L" };
                let status_frame = frame.clone();
                screens::boot_frame(
                    &mut frame,
                    side,
                    since,
                    since_linked,
                    initial,
                    &status_frame,
                );
            }
        }
        frame
    }
}

/// How long after a half (re)starts the halves are linked again: the
/// start-up animation, then the link (as the firmware, roughly).
const RELINK_MS: u32 = 2500;

/// Whether key position `pos` is on the right half (columns 6-11, right thumbs).
fn on_right_half(pos: u8) -> bool {
    (pos < 36 && pos % 12 >= 6) || (39..42).contains(&pos)
}

// ---- key legends -----------------------------------------------------------

fn shifted_symbol(code: u8) -> Option<&'static str> {
    Some(match code {
        0x1E => "!",
        0x1F => "@",
        0x20 => "#",
        0x21 => "$",
        0x22 => "%",
        0x23 => "^",
        0x24 => "&",
        0x25 => "*",
        0x26 => "(",
        0x27 => ")",
        0x2D => "_",
        0x2E => "+",
        0x2F => "{",
        0x30 => "}",
        0x31 => "|",
        0x33 => ":",
        0x34 => "\"",
        0x35 => "~",
        0x36 => "<",
        0x37 => ">",
        0x38 => "?",
        _ => return None,
    })
}

fn base_label(code: u8) -> String {
    let s = match code {
        0x04..=0x1D => return ((b'A' + code - 0x04) as char).to_string(),
        0x1E..=0x26 => return ((b'1' + code - 0x1E) as char).to_string(),
        0x27 => "0",
        0x28 => "Enter",
        0x29 => "Esc",
        0x2A => "Bksp",
        0x2B => "Tab",
        0x2C => "Space",
        0x2D => "-",
        0x2E => "=",
        0x2F => "[",
        0x30 => "]",
        0x31 => "\\",
        0x33 => ";",
        0x34 => "'",
        0x35 => "`",
        0x36 => ",",
        0x37 => ".",
        0x38 => "/",
        0x39 => "Caps",
        0x46 => "PrtSc",
        0x49 => "Ins",
        0x4A => "Home",
        0x4B => "PgUp",
        0x4C => "Del",
        0x4D => "End",
        0x4E => "PgDn",
        0x4F => "→",
        0x50 => "←",
        0x51 => "↓",
        0x52 => "↑",
        0xE0 | 0xE4 => "Ctrl",
        0xE1 | 0xE5 => "Shift",
        0xE2 => "Alt",
        0xE6 => "AltGr",
        0xE3 | 0xE7 => "Gui",
        _ => keys::name(code),
    };
    s.to_string()
}

/// Window-manager shortcuts (Super + ...) as Hyprland/erikwm actions.
fn wm_label(k: Key) -> Option<String> {
    if k.mods & MOD_LGUI == 0 || k.is_modifier() {
        return None;
    }
    let others = k.mods & !MOD_LGUI;
    let digit = match k.code {
        0x1E..=0x26 => Some(k.code - 0x1E + 1),
        0x27 => Some(10),
        _ => None,
    };
    Some(match (others, digit, k.code) {
        (0, Some(n), _) => format!("WS {n}"),
        (MOD_LSFT, Some(n), _) => format!("→ {n}"),
        (MOD_LCTL, None, 0x50) => "WS ◀".into(),
        (MOD_LCTL, None, 0x4F) => "WS ▶".into(),
        (m, None, 0x50) if m == MOD_LCTL | MOD_LSFT => "→ WS◀".into(),
        (m, None, 0x4F) if m == MOD_LCTL | MOD_LSFT => "→ WS▶".into(),
        (0, None, 0x36) => "Col ◀".into(),
        (0, None, 0x37) => "Col ▶".into(),
        (MOD_LSFT, None, 0x0D) => "Win ▼".into(),
        (MOD_LSFT, None, 0x0E) => "Win ▲".into(),
        _ => return None,
    })
}

fn key_label(k: Key) -> String {
    if k == keys::MEH {
        return "Meh".into();
    }
    if let Some(label) = wm_label(k) {
        return label;
    }
    if k.mods == MOD_LSFT
        && let Some(sym) = shifted_symbol(k.code)
    {
        return sym.into();
    }
    let mut out = String::new();
    for (bit, sign) in [
        (MOD_LCTL | MOD_RCTL, "⌃"),
        (MOD_LSFT | MOD_RSFT, "⇧"),
        (MOD_LALT | MOD_RALT, "⌥"),
        (MOD_LGUI | MOD_RGUI, "◆"),
    ] {
        if k.mods & bit != 0 {
            out.push_str(sign);
        }
    }
    out.push_str(&base_label(k.code));
    out
}

fn arrow(d: Direction) -> &'static str {
    match d {
        Direction::Left => "←",
        Direction::Down => "↓",
        Direction::Up => "↑",
        Direction::Right => "→",
    }
}

fn click_label(b: MouseButton) -> &'static str {
    match b {
        MouseButton::Left => "L click",
        MouseButton::Right => "R click",
        MouseButton::Middle => "M click",
    }
}

#[must_use]
pub fn action_label(action: Action) -> String {
    match action {
        Action::Mouse(MouseAction::Move(d)) => {
            let head = match d {
                Direction::Left => "◀",
                Direction::Down => "▼",
                Direction::Up => "▲",
                Direction::Right => "▶",
            };
            format!("{head} mouse")
        }
        Action::Mouse(MouseAction::Button(b)) => click_label(b).into(),
        Action::Mouse(MouseAction::Scroll(d)) => format!("scroll {}", arrow(d)),
        Action::Mouse(MouseAction::Precision) => "Precise".into(),
        // Two lines, like a mod-tap: the key first, what it does in mouse
        // mode (the pointer moved) under it.
        Action::MouseOrKey { mouse, key } => {
            let mouse = match mouse {
                MouseAction::Button(b) => click_label(b).to_string(),
                other => action_label(Action::Mouse(other)),
            };
            format!("{}\n{mouse}", key_label(key))
        }
        Action::Trans | Action::None => String::new(),
        Action::Key(k) => key_label(k),
        Action::MomentaryLayer(l) => corne::KEYMAP
            .layer_names
            .get(l as usize)
            .copied()
            .unwrap_or("?")
            .to_string(),
        Action::ModTap { hold, tap } => format!("{}\n{}", key_label(tap), key_label(hold)),
        Action::CapsWord => "Caps\nWord".into(),
        Action::System(s) => match s {
            SystemAction::BtPrev => "Profile ◀",
            SystemAction::BtNext => "Profile ▶",
            SystemAction::BtClear => "Profile clear",
            SystemAction::OutToggle => "USB/BT",
            SystemAction::StudioUnlock => "Studio",
            SystemAction::ShowLink => "Link?",
            SystemAction::RadioPower => "Radio",
        }
        .into(),
    }
}

// ---- C ABI -------------------------------------------------------------------

/// One engine output. `kind`: 1 = HID report (`mods`, `keys` = NKRO bitmap),
/// 2 = layer changed (`value` = layer), 3 = caps word (`value` = 0/1),
/// 4 = system action (`value`: 0 `BtPrev`, 1 `BtNext`, 2 `BtClear`, 3 `OutToggle`, 4 `StudioUnlock`, 5 `ShowLink`,
/// 6 `RadioPower`), 5 = power state (`value`: 0 active, 1 idle, 2 deep sleep),
/// 6 = pairing ended (`value`: [`PairResult`]: 0 paired, 1 wrong code, 2 rejected, 3 timed out),
/// 7 = mouse report for the computer (`mods` = buttons: bit 0 left, 1 right,
/// 2 middle; `keys[0..7]` = the report: buttons, x i16 LE, y i16 LE (down +),
/// wheel i8 (up +), pan i8 (right +)), 0 = nothing to show (skip it).
#[repr(C)]
pub struct KbEvent {
    pub kind: u8,
    pub value: u8,
    pub mods: u8,
    pub reserved: u8,
    pub keys: [u8; 32],
}

impl KbEvent {
    /// The C form of `event`.
    fn of(event: Event) -> Self {
        let mut ev = Self {
            kind: 0,
            value: 0,
            mods: 0,
            reserved: 0,
            keys: [0; 32],
        };
        let o = match event {
            Event::Output(o) => o,
            Event::Power(state) => {
                ev.kind = 5;
                ev.value = power_code(state);
                return ev;
            }
            Event::Pairing(result) => {
                ev.kind = 6;
                ev.value = result as u8;
                return ev;
            }
            Event::Mouse(report) => {
                ev.kind = 7;
                ev.mods = report.buttons;
                ev.keys[..kb_core::mouse::REPORT_LEN].copy_from_slice(&report.bytes());
                return ev;
            }
        };
        match o {
            Output::Report(r) => {
                ev.kind = 1;
                ev.mods = r.mods;
                ev.keys = *r.nkro_bitmap();
            }
            Output::Layer(l) => {
                ev.kind = 2;
                ev.value = l;
            }
            Output::CapsWord(on) => {
                ev.kind = 3;
                ev.value = u8::from(on);
            }
            Output::Mouse(..) | Output::Pointer(_) => {
                // Mouse keys come out as kind 7 reports; the pointer state is
                // `kb_emu_mouse_state`. Reported as nothing: kind 0.
            }
            Output::System(s) => {
                ev.kind = 4;
                ev.value = match s {
                    SystemAction::BtPrev => 0,
                    SystemAction::BtNext => 1,
                    SystemAction::BtClear => 2,
                    SystemAction::OutToggle => 3,
                    SystemAction::StudioUnlock => 4,
                    SystemAction::ShowLink => 5,
                    SystemAction::RadioPower => 6,
                };
            }
        }
        ev
    }
}

fn emu<'a>(e: *mut Emu) -> Option<&'a mut Emu> {
    // SAFETY: the caller passes a pointer from kb_emu_new, not yet freed.
    unsafe { e.as_mut() }
}

/// Copy `s` into `buf` (not NUL-terminated); returns the full length in bytes.
fn copy_out(s: &[u8], buf: *mut u8, cap: usize) -> usize {
    if !buf.is_null() {
        let n = s.len().min(cap);
        // SAFETY: the caller guarantees `buf` has room for `cap` bytes.
        unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), buf, n) };
    }
    s.len()
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_new() -> *mut Emu {
    Box::into_raw(Box::new(Emu::new()))
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_free(e: *mut Emu) {
    if !e.is_null() {
        // SAFETY: pointer came from Box::into_raw in kb_emu_new.
        drop(unsafe { Box::from_raw(e) });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_key_count() -> u32 {
    KEYS as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_press(e: *mut Emu, pos: u8, now_ms: u32) {
    if let Some(e) = emu(e) {
        e.press(pos, now_ms);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_release(e: *mut Emu, pos: u8, now_ms: u32) {
    if let Some(e) = emu(e) {
        e.release(pos, now_ms);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_tick(e: *mut Emu, now_ms: u32) {
    if let Some(e) = emu(e) {
        e.tick(now_ms);
    }
}

/// Next time `kb_emu_tick` must be called, or -1 when nothing is waiting.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_next_deadline(e: *mut Emu) -> i64 {
    emu(e).and_then(|e| e.next_deadline()).map_or(-1, i64::from)
}

/// Fill `out` with the next output. Returns false when there is none.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_poll(e: *mut Emu, now_ms: u32, out: *mut KbEvent) -> bool {
    let (Some(e), false) = (emu(e), out.is_null()) else {
        return false;
    };
    let Some(event) = e.poll(now_ms) else {
        return false;
    };
    // SAFETY: checked non-null above; the caller provides room for a KbEvent.
    unsafe { out.write(KbEvent::of(event)) };
    true
}

fn power_code(state: PowerState) -> u8 {
    match state {
        PowerState::Active => 0,
        PowerState::Idle => 1,
        PowerState::Sleep => 2,
    }
}

/// A computer starts pairing with `code` (6 digits, shown by the computer):
/// the left screen asks for it. Returns the code (`code % 1000000`).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_simulate_pairing(e: *mut Emu, code: u32, now_ms: u32) -> u32 {
    emu(e).map_or(0, |e| e.simulate_pairing(code, now_ms))
}

/// Whether a pairing prompt is up.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_pairing(e: *mut Emu) -> bool {
    emu(e).is_some_and(|e| e.pairing.is_some())
}

/// The radio's transmit power in dBm (`RadioPower` key).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_radio_dbm(e: *mut Emu) -> i8 {
    emu(e).map_or(0, |e| e.status.radio_dbm)
}

/// 0 = active, 1 = idle (OLEDs off), 2 = deep sleep.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_power_state(e: *mut Emu) -> u8 {
    emu(e).map_or(0, |e| power_code(e.power_state()))
}

/// Idle and deep-sleep timeouts in ms; `sleep_ms` 0 disables deep sleep.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_power_timeouts(e: *mut Emu, idle_ms: u32, sleep_ms: u32, now_ms: u32) {
    if let Some(e) = emu(e) {
        let sleep_ms = (sleep_ms != 0).then_some(sleep_ms);
        e.set_power_config(PowerConfig { idle_ms, sleep_ms }, now_ms);
    }
}

/// UTF-8 legend of `pos` on the current layers. Returns the full length.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_label(e: *mut Emu, pos: u8, buf: *mut u8, cap: usize) -> usize {
    emu(e).map_or(0, |e| {
        e.sync_studio();
        copy_out(e.label(pos).as_bytes(), buf, cap)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_layer_name(e: *mut Emu, layer: u8, buf: *mut u8, cap: usize) -> usize {
    emu(e).map_or(0, |e| copy_out(e.layer_name(layer).as_bytes(), buf, cap))
}

/// Short name of a HID usage (`A`, `LSHFT`, `F20`, ...). Returns the full length.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_usage_name(usage: u8, buf: *mut u8, cap: usize) -> usize {
    copy_out(keys::name(usage).as_bytes(), buf, cap)
}

/// Simulated battery level for `half` (0 = left, 1 = right).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_battery(e: *mut Emu, half: u8, percent: u8) {
    if let Some(e) = emu(e) {
        e.status_mut().battery[(half as usize).min(1)] = percent.min(100);
    }
}

/// Switch half `half` (0 left, 1 right) on or off.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_half_power(e: *mut Emu, half: u8, on: bool, now: u32) {
    if let Some(e) = emu(e) {
        e.set_half_power(half as usize, on, now);
    }
}

/// Press half `half`'s reset button.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_reset_half(e: *mut Emu, half: u8, now: u32) {
    if let Some(e) = emu(e) {
        e.reset_half(half as usize, now);
    }
}

/// Play the start-up animation again (as after switching the keyboard on).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_replay_boot(e: *mut Emu) {
    if let Some(e) = emu(e) {
        e.boot_due = [true, true];
    }
}

/// Whether half `half` runs on USB power (charging).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_charging(e: *mut Emu, half: u8, charging: bool) {
    if let Some(e) = emu(e) {
        e.status_mut().charging[(half as usize).min(1)] = charging;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_peripheral_connected(e: *mut Emu, connected: bool) {
    if let Some(e) = emu(e) {
        e.status_mut().peripheral_connected = connected;
    }
}

/// Screen style: 0 = ZMK's built-in screen (landscape), 1 = upright portrait.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_screen_style(e: *mut Emu, style: u8) {
    if let Some(e) = emu(e) {
        e.set_style(if style == 0 {
            Style::Zmk
        } else {
            Style::Portrait
        });
    }
}

/// Whether the selected Bluetooth profile's computer is connected
/// (connecting an open profile pairs it).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_profile_connected(e: *mut Emu, connected: bool) {
    if let Some(e) = emu(e) {
        e.set_profile_connected(connected);
    }
}

/// Mark the selected Bluetooth profile open (nothing paired, `+`) or taken.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_profile_open(e: *mut Emu, open: bool) {
    if let Some(e) = emu(e) {
        e.set_profile_open(open);
    }
}

/// Select Bluetooth profile `profile` (0-based, below `kb_emu_profile_count`).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_profile(e: *mut Emu, profile: u8) {
    if let Some(e) = emu(e) {
        e.select_profile(profile);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_profile_count() -> u8 {
    kb_display::status::BT_PROFILES
}

/// Selected Bluetooth profile, 0-based.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_profile(e: *mut Emu) -> u8 {
    emu(e).map_or(0, |e| e.status().profile)
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_profile_open(e: *mut Emu) -> bool {
    emu(e).is_some_and(|e| e.status().profile_open)
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_profile_connected(e: *mut Emu) -> bool {
    emu(e).is_some_and(|e| e.status().profile_connected)
}

#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_peripheral_connected(e: *mut Emu) -> bool {
    emu(e).is_some_and(|e| e.status().peripheral_connected)
}

/// Output transport: 0 = USB, 1 = Bluetooth.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_transport(e: *mut Emu, transport: u8) {
    if let Some(e) = emu(e) {
        e.set_transport(if transport == 0 {
            Transport::Usb
        } else {
            Transport::Ble
        });
    }
}

/// 0 = USB, 1 = Bluetooth.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_transport(e: *mut Emu) -> u8 {
    emu(e).map_or(1, |e| match e.status().transport {
        Transport::Usb => 0,
        Transport::Ble => 1,
    })
}

/// Number of split link stages; index 0 is "hidden".
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_link_stage_count() -> u8 {
    LINK_STAGES.len() as u8
}

/// Name of link stage `index` (empty for 0 or out of range). Returns the full length.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_link_stage_name(index: u8, buf: *mut u8, cap: usize) -> usize {
    let name = LINK_STAGES.get(index as usize).copied().unwrap_or("");
    copy_out(name.as_bytes(), buf, cap)
}

/// Show split link stage `index` at the bottom of the OLEDs (0 or out of range hides it).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_set_link_stage(e: *mut Emu, index: u8) {
    if let Some(e) = emu(e) {
        e.set_link_stage(index as usize);
    }
}

/// Index of the link stage shown (0 = hidden).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_link_stage(e: *mut Emu) -> u8 {
    emu(e).map_or(0, |e| {
        LINK_STAGES
            .iter()
            .position(|&s| s == e.status().link_detail)
            .unwrap_or(0) as u8
    })
}

/// Feed one Raw HID report (32 bytes) from the computer. Returns a
/// [`RawHidResult`]: 0 pending, 1 delivered, 2 right half not linked, 3 cleared, 4 asleep.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_raw_hid(e: *mut Emu, report: *const u8, len: usize, now_ms: u32) -> u8 {
    let Some(e) = emu(e) else {
        return RawHidResult::Pending as u8;
    };
    if report.is_null() {
        return RawHidResult::Pending as u8;
    }
    // SAFETY: the caller passes `len` readable bytes.
    let report = unsafe { std::slice::from_raw_parts(report, len) };
    e.raw_hid(report, now_ms) as u8
}

/// I²C writes for display `half`, framed as `[len: u16 LE][bytes...]` per
/// transaction, all to address 0x3C. Returns the total length; if it is
/// larger than `cap` nothing is written and the call should be repeated
/// with a bigger buffer (the writes are kept until then).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_display(
    e: *mut Emu,
    half: u8,
    now_ms: u32,
    buf: *mut u8,
    cap: usize,
) -> usize {
    let Some(e) = emu(e) else { return 0 };
    let half = (half as usize).min(1);
    let snapshot = e.shown[half].clone();
    let writes = e.display_writes(half, now_ms);
    let mut framed = Vec::new();
    for w in &writes {
        framed.extend_from_slice(&(w.len() as u16).to_le_bytes());
        framed.extend_from_slice(w);
    }
    if framed.len() > cap || buf.is_null() {
        e.shown[half] = snapshot; // not delivered: resend next time
        return framed.len();
    }
    copy_out(&framed, buf, cap)
}

// ---- Mouse --------------------------------------------------------------------

/// Where the mouse keys have taken the pointer, for drawing it.
#[repr(C)]
pub struct KbMouse {
    /// Sum of all mouse reports' X / Y since start (or the last reset), px.
    pub x: i32,
    pub y: i32,
    /// Sum of wheel notches (up +) and pan notches (right +).
    pub wheel: i32,
    pub pan: i32,
    /// Buttons held as last reported (bit 0 left, 1 right, 2 middle).
    pub buttons: u8,
    /// 1 while the pointer is active (the nav thumbs click; the MOUSE bar).
    pub pointer: u8,
    /// 1 while mouse keys still move or scroll (keep ticking every ms).
    pub moving: u8,
    /// Smooth scrolling as the emulated computer set it
    /// (`kb_emu_mouse_set_resolution`): bit 0 wheel, bit 1 pan. Then
    /// `wheel` / `pan` count 1/16 detents, otherwise whole detents.
    pub hires: u8,
}

/// Fill `out` with the mouse state. False for a null pointer.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_mouse_state(e: *mut Emu, out: *mut KbMouse) -> bool {
    let (Some(e), false) = (emu(e), out.is_null()) else {
        return false;
    };
    let t = e.mouse_totals;
    let state = KbMouse {
        x: t.x,
        y: t.y,
        wheel: t.wheel,
        pan: t.pan,
        buttons: t.buttons,
        pointer: u8::from(e.status.pointer_active),
        moving: u8::from(e.mouse.active()),
        hires: {
            let [w, p] = e.mouse.resolution();
            u8::from(w > 1) | u8::from(p > 1) << 1
        },
    };
    // SAFETY: checked non-null; the caller provides a valid KbMouse.
    unsafe { out.write(state) };
    true
}

/// The emulated computer sets (or clears) the Resolution Multipliers, as
/// Windows and Linux do: then the wheel and pan move in 1/16 detents
/// (smooth scrolling). A new emulator starts with whole detents (as macOS).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_mouse_set_resolution(e: *mut Emu, wheel_hires: bool, pan_hires: bool) {
    if let Some(e) = emu(e) {
        e.mouse.set_resolution(wheel_hires, pan_hires);
    }
}

/// Set the summed pointer position (and wheel / pan) back to 0.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_mouse_reset_position(e: *mut Emu) {
    if let Some(e) = emu(e) {
        let buttons = e.mouse_totals.buttons;
        e.mouse_totals = MouseTotals {
            buttons,
            ..MouseTotals::default()
        };
    }
}

// ---- Studio: live keymap editing --------------------------------------------
//
// The same editable keymap, lock and protocol as the firmware
// (`kb_core::studio`, docs/studio-protocol.md). Strings go in as
// (pointer, length in bytes) UTF-8 without a NUL; strings come out through
// (buf, cap) like the other kb_emu_* text functions: the full length is
// returned, at most `cap` bytes are copied, no NUL is added.
//
// Actions are encoded as in the protocol: `kp:A`, `kp:LC(C)`, `mt:LALT/TAB`,
// `mo:1`, `trans`, `none`, `caps`, `sys:BtNext`.
//
// Result codes (i32) of the functions that change something:
//   0 ok, 1 locked (press StudioUnlock first: nav layer, position 35),
//   2 range (no such layer/position), 3 action (not an action, or `mo` to a
//   missing layer), 4 long (encoding over 40 characters), 5 lockout (no
//   StudioUnlock key would be reachable), 6 complex (too tangled to check),
//   7 storage (the file could not be written; still dirty), 8 positions
//   (combo: not 2-4 distinct positions), 9 name (combo name), 10 timing (combo
//   timeout 1..=65535 / idle 0..=65535), 11 full (16 combos), 12 syntax
//   (combo text isn't six words), -1 bad arguments.
//
// Combos are text of six words, as in the protocol:
//   `<name|-> <positions> <layers|*> <timeout ms> <idle ms> <action>`,
//   e.g. `copy 26,27 0 50 150 kp:LC(C)`.
//
// Unlocking: pressing the StudioUnlock key (`kb_emu_press` / `kb_emu_release`
// of position 35 while nav is held) unlocks for 5 minutes after the last
// unlock or accepted write; `kb_emu_studio_lock` or an emulated restart locks.
// Reads work locked; `revert` works locked too (it only goes back to saved).
//
// Storage: in memory (default: survives emulated restarts, not the app) or a
// text file (`kb_emu_studio_set_storage_path`).

/// Text from the caller: `len` bytes at `ptr`, UTF-8.
fn text_in<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return (len == 0).then_some("");
    }
    // SAFETY: the caller passes `len` readable bytes at `ptr`.
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len) }).ok()
}

fn studio_code(result: Result<(), kb_core::studio::Error>) -> i32 {
    use kb_core::studio::Error;
    match result {
        Ok(()) => 0,
        Err(Error::Locked) => 1,
        Err(Error::Range) => 2,
        Err(Error::Action) => 3,
        Err(Error::TooLong) => 4,
        Err(Error::Lockout) => 5,
        Err(Error::TooComplex) => 6,
        Err(Error::Positions) => 8,
        Err(Error::Name) => 9,
        Err(Error::Timing) => 10,
        Err(Error::Full) => 11,
        Err(Error::Syntax) => 12,
    }
}

/// 0, or 7 when the storage file couldn't be written.
fn stored_code(result: Result<bool, kb_core::studio::Error>) -> i32 {
    match result {
        Ok(true) => 0,
        Ok(false) => 7,
        Err(e) => studio_code(Err(e)),
    }
}

/// Number of layers (4).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_layer_count() -> u8 {
    corne::LAYER_COUNT as u8
}

/// The live binding of `pos` on `layer`, encoded (no `trans` fall-through).
/// 0 for a layer or position out of range.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_binding(
    e: *mut Emu,
    layer: u8,
    pos: u8,
    buf: *mut u8,
    cap: usize,
) -> usize {
    emu(e)
        .and_then(|e| e.studio.studio.binding(layer, pos))
        .map_or(0, |a| {
            copy_out(
                kb_core::studio::encoding::encoded(a).as_str().as_bytes(),
                buf,
                cap,
            )
        })
}

/// The compiled (default) binding of `pos` on `layer`, encoded.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_default_binding(
    e: *mut Emu,
    layer: u8,
    pos: u8,
    buf: *mut u8,
    cap: usize,
) -> usize {
    emu(e)
        .and_then(|e| e.studio.studio.default_binding(layer, pos))
        .map_or(0, |a| {
            copy_out(
                kb_core::studio::encoding::encoded(a).as_str().as_bytes(),
                buf,
                cap,
            )
        })
}

/// All live bindings of `layer`, encoded, separated by single spaces (the
/// payload of a `#L` line). 0 for a layer out of range.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_layer(e: *mut Emu, layer: u8, buf: *mut u8, cap: usize) -> usize {
    let Some(actions) = emu(e).and_then(|e| e.studio.studio.layers().get(layer as usize).copied())
    else {
        return 0;
    };
    let text: Vec<String> = actions
        .iter()
        .map(|&a| kb_core::studio::encoding::encoded(a).as_str().to_string())
        .collect();
    copy_out(text.join(" ").as_bytes(), buf, cap)
}

/// Bind the action `enc` (`len` bytes) to `pos` on `layer`, live. Result code.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_set(
    e: *mut Emu,
    layer: u8,
    pos: u8,
    enc: *const u8,
    len: usize,
    now_ms: u32,
) -> i32 {
    let (Some(e), Some(text)) = (emu(e), text_in(enc, len)) else {
        return -1;
    };
    let result = e.studio.set(layer, pos, text, now_ms);
    e.sync_studio();
    studio_code(result)
}

/// Whether `enc` could be bound to `pos` on `layer` (all checks but the
/// lock; nothing changes). Result code (never 1).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_check(
    e: *mut Emu,
    layer: u8,
    pos: u8,
    enc: *const u8,
    len: usize,
) -> i32 {
    let (Some(e), Some(text)) = (emu(e), text_in(enc, len)) else {
        return -1;
    };
    studio_code(e.studio.check(layer, pos, text))
}

/// Keep the live bindings (to the storage file, if one is set). Result code.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_save(e: *mut Emu, now_ms: u32) -> i32 {
    emu(e).map_or(-1, |e| stored_code(e.studio.save(now_ms)))
}

/// Drop unsaved edits (back to the saved bindings). Works locked. Result code (0).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_revert(e: *mut Emu) -> i32 {
    emu(e).map_or(-1, |e| {
        e.studio.studio.revert();
        e.sync_studio();
        0
    })
}

/// Back to the compiled keymap, live and in storage. Result code.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_reset(e: *mut Emu, now_ms: u32) -> i32 {
    emu(e).map_or(-1, |e| {
        let result = e.studio.reset(now_ms);
        e.sync_studio();
        stored_code(result)
    })
}

/// Lock Studio now.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_lock(e: *mut Emu, now_ms: u32) {
    if let Some(e) = emu(e) {
        e.studio.studio.lock();
        e.studio_status(now_ms);
    }
}

/// Milliseconds until Studio locks itself; 0 = locked.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_unlock_remaining_ms(e: *mut Emu, now_ms: u32) -> u32 {
    emu(e).map_or(0, |e| e.studio.studio.remaining_ms(now_ms))
}

/// Whether the live bindings differ from what is saved.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_dirty(e: *mut Emu) -> bool {
    emu(e).is_some_and(|e| e.studio.studio.dirty())
}

/// Counts changes of the live bindings (wrapping): refresh when it changes.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_generation(e: *mut Emu) -> u32 {
    emu(e).map_or(0, |e| e.studio.studio.generation())
}

/// Keep Studio's edits in the file at `path` (`len` bytes, UTF-8; null or
/// 0 = in memory), and read it now: unsaved edits are dropped and Studio
/// locks, as when a keyboard starts. Returns 0 applied (or no file yet),
/// 1 ignored (unreadable, other version, bad line: compiled keymap),
/// 2 dropped (it would lock Studio out: compiled keymap, file cleared),
/// -1 bad arguments.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_set_storage_path(e: *mut Emu, path: *const u8, len: usize) -> i32 {
    let (Some(e), Some(path)) = (emu(e), text_in(path, len)) else {
        return -1;
    };
    let path = (!path.is_empty()).then(|| std::path::PathBuf::from(path));
    let loaded = e.studio.set_storage_path(path);
    e.sync_studio();
    loaded as i32
}

/// Run one line of the text protocol, exactly as the keyboard's log port
/// would (`#studio get`, `#studio set 0 12 kp:A`, ...; no line end needed).
/// The answers come from `kb_emu_studio_next_line`. Returns false (and does
/// nothing) for a line that isn't a `#studio` command.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_command(
    e: *mut Emu,
    line: *const u8,
    len: usize,
    now_ms: u32,
) -> bool {
    let (Some(e), Some(line)) = (emu(e), text_in(line, len)) else {
        return false;
    };
    let taken = e.studio.command(line.as_bytes(), now_ms);
    e.sync_studio();
    e.studio_status(now_ms);
    taken
}

/// The next protocol line the keyboard would send (with its `\r\n`):
/// answers to commands, and `#U` / `#D` changes once a command was sent.
/// Returns its length, 0 when there is none. The line is only consumed when
/// it fits in `cap`; otherwise call again with a buffer of at least the
/// returned length (`kb_emu_studio_line_max` always fits).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_next_line(
    e: *mut Emu,
    now_ms: u32,
    buf: *mut u8,
    cap: usize,
) -> usize {
    let Some(e) = emu(e) else { return 0 };
    let Some(line) = e.studio.next_line(now_ms) else {
        return 0;
    };
    let len = line.len();
    if len <= cap && !buf.is_null() {
        copy_out(line.as_bytes(), buf, cap);
        e.studio.line_taken();
    }
    len
}

/// Longest line `kb_emu_studio_next_line` gives.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_line_max() -> u32 {
    studio::LINE_MAX as u32
}

/// Most combos (16).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_combo_max() -> u8 {
    kb_core::engine::MAX_COMBOS as u8
}

/// Number of live combos.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_combo_count(e: *mut Emu) -> u8 {
    emu(e).map_or(0, |e| e.studio.studio.combos().len() as u8)
}

/// Live combo `index` as its six words; 0 past the last.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_combo(e: *mut Emu, index: u8, buf: *mut u8, cap: usize) -> usize {
    emu(e)
        .and_then(|e| e.studio.studio.combos().get(index as usize))
        .map_or(0, |c| copy_out(studio::combo_text(&c).as_bytes(), buf, cap))
}

/// Number of compiled combos.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_default_combo_count(e: *mut Emu) -> u8 {
    emu(e).map_or(0, |e| e.studio.studio.default_combos().len() as u8)
}

/// Compiled combo `index` as its six words; 0 past the last.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_default_combo(
    e: *mut Emu,
    index: u8,
    buf: *mut u8,
    cap: usize,
) -> usize {
    emu(e)
        .and_then(|e| e.studio.studio.default_combos().get(index as usize))
        .map_or(0, |c| copy_out(studio::combo_text(&c).as_bytes(), buf, cap))
}

/// Add the combo `text` (six words) at the end, live. Result code; on 0
/// its index is `kb_emu_studio_combo_count() - 1`.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_combo_add(
    e: *mut Emu,
    text: *const u8,
    len: usize,
    now_ms: u32,
) -> i32 {
    let (Some(e), Some(text)) = (emu(e), text_in(text, len)) else {
        return -1;
    };
    let result = e.studio.combo_add(text, now_ms).map(|_| ());
    e.sync_studio();
    studio_code(result)
}

/// Replace combo `index` with `text` (six words), live. Result code.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_combo_set(
    e: *mut Emu,
    index: u8,
    text: *const u8,
    len: usize,
    now_ms: u32,
) -> i32 {
    let (Some(e), Some(text)) = (emu(e), text_in(text, len)) else {
        return -1;
    };
    let result = e.studio.combo_set(index, text, now_ms);
    e.sync_studio();
    studio_code(result)
}

/// Remove combo `index` (the later ones move down), live. Result code.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_combo_remove(e: *mut Emu, index: u8, now_ms: u32) -> i32 {
    emu(e).map_or(-1, |e| {
        let result = e.studio.studio.remove_combo(index, now_ms);
        e.sync_studio();
        studio_code(result)
    })
}

/// Whether `text` could be combo `index` (255: added as a new one): all
/// checks but the lock, nothing changes. Result code (never 1).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_combo_check(
    e: *mut Emu,
    index: u8,
    text: *const u8,
    len: usize,
) -> i32 {
    let (Some(e), Some(text)) = (emu(e), text_in(text, len)) else {
        return -1;
    };
    let index = (index != 255).then_some(index);
    studio_code(e.studio.combo_check(index, text))
}

/// Whether combo `index` could be removed (all checks but the lock).
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_combo_check_remove(e: *mut Emu, index: u8) -> i32 {
    emu(e).map_or(-1, |e| {
        studio_code(e.studio.studio.check_remove_combo(index))
    })
}

/// The canonical encoding of `enc` (e.g. `MT:lalt/tab` → `mt:LALT/TAB`).
/// Returns its length, or -1 if `enc` is not an action.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_canonical(
    enc: *const u8,
    len: usize,
    buf: *mut u8,
    cap: usize,
) -> i64 {
    let Some(action) = text_in(enc, len).and_then(|t| kb_core::studio::encoding::decode(t).ok())
    else {
        return -1;
    };
    copy_out(
        kb_core::studio::encoding::encoded(action)
            .as_str()
            .as_bytes(),
        buf,
        cap,
    ) as i64
}

/// The key legend for the action `enc` (as `kb_emu_label` draws it: two
/// lines `tap\nhold` for a mod-tap, the layer name for `mo`). Empty for
/// `trans`, `none` and anything that is not an action.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_label(
    enc: *const u8,
    len: usize,
    buf: *mut u8,
    cap: usize,
) -> usize {
    let label = text_in(enc, len)
        .and_then(|t| kb_core::studio::encoding::decode(t).ok())
        .map(action_label)
        .unwrap_or_default();
    copy_out(label.as_bytes(), buf, cap)
}

/// Key name number `index` for a picker (`A`, ..., `RGUI`, then the
/// aliases `EXCL`, ..., `MEH`): what `kp:` / `mt:` take. 0 past the last.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_key_name(index: u32, buf: *mut u8, cap: usize) -> usize {
    kb_core::studio::encoding::key_names()
        .nth(index as usize)
        .map_or(0, |n| copy_out(n.as_bytes(), buf, cap))
}

/// System action name number `index` (`BtPrev`, ..., what `sys:` takes);
/// 0 past the last.
#[unsafe(no_mangle)]
pub extern "C" fn kb_emu_studio_system_name(index: u32, buf: *mut u8, cap: usize) -> usize {
    SystemAction::ALL
        .get(index as usize)
        .map_or(0, |s| copy_out(s.name().as_bytes(), buf, cap))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(e: &mut Emu, now: u32) {
        while e.poll(now).is_some() {}
    }

    #[test]
    fn label_of_plain_key_is_its_letter() {
        // Arrange
        let e = Emu::new();

        // Act
        let label = e.label(1);

        // Assert
        assert_eq!(label, "Q");
    }

    #[test]
    fn label_of_mod_tap_shows_tap_over_hold() {
        // Arrange
        let e = Emu::new();

        // Act
        let labels = [e.label(12), e.label(37), e.label(40)];

        // Assert
        assert_eq!(labels, ["Tab\nAlt", "Bksp", "Del"]);
    }

    #[test]
    fn label_of_layer_key_is_layer_name() {
        // Arrange
        let e = Emu::new();

        // Act
        let label = e.label(36);

        // Assert
        assert_eq!(label, "symbols");
    }

    #[test]
    fn labels_follow_the_active_layer() {
        // Arrange
        let mut e = Emu::new();
        e.press(36, 0);
        drain(&mut e, 0);

        // Act
        let labels = [e.label(1), e.label(12)];

        // Assert: Q is `!` on symbols, Tab/Alt is transparent
        assert_eq!(labels, ["!", "Tab\nAlt"]);
    }

    #[test]
    fn layer_change_updates_status() {
        // Arrange
        let mut e = Emu::new();
        e.press(36, 0);

        // Act
        drain(&mut e, 0);

        // Assert
        assert_eq!(e.status().layer, 1);
    }

    #[test]
    fn bt_next_on_nav_selects_next_profile() {
        // Arrange
        let mut e = Emu::new();

        // Act: Profile ▶ (a combo key: it acts when released alone)
        tap_on_nav(&mut e, BT_NEXT, 0);

        // Assert
        assert_eq!(e.status().profile, 1);
    }

    const NAV: u8 = 41;
    const BT_PREV: u8 = 25;
    const BT_NEXT: u8 = 26;

    fn tap_on_nav(e: &mut Emu, pos: u8, now: u32) {
        e.press(NAV, now);
        e.press(pos, now);
        e.release(pos, now);
        e.release(NAV, now);
        drain(e, now);
    }

    #[test]
    fn bt_next_shows_the_next_profile_as_open() {
        // Arrange
        let mut e = Emu::new();

        // Act
        tap_on_nav(&mut e, BT_NEXT, 0);

        // Assert
        let s = e.status();
        assert_eq!(
            (s.profile, s.profile_open, s.profile_connected),
            (1, true, false)
        );
    }

    #[test]
    fn bt_prev_back_to_a_paired_profile_shows_it_not_connected() {
        // Arrange
        let mut e = Emu::new();
        tap_on_nav(&mut e, BT_NEXT, 0);

        // Act
        tap_on_nav(&mut e, BT_PREV, 10);

        // Assert
        let s = e.status();
        assert_eq!(
            (s.profile, s.profile_open, s.profile_connected),
            (0, false, false)
        );
    }

    #[test]
    fn bt_clear_combo_makes_the_profile_open() {
        // Arrange
        let mut e = Emu::new();
        e.press(NAV, 0);
        e.press(BT_PREV, 0);

        // Act: both profile keys
        e.press(BT_NEXT, 5);
        drain(&mut e, 5);

        // Assert
        let s = e.status();
        assert_eq!((s.profile_open, s.profile_connected), (true, false));
    }

    #[test]
    fn bt_prev_from_the_first_profile_selects_usb() {
        // Arrange
        let mut e = Emu::new();

        // Act
        tap_on_nav(&mut e, BT_PREV, 0);

        // Assert
        assert_eq!(e.status().transport, Transport::Usb);
    }

    #[test]
    fn link_stage_index_selects_the_firmware_name() {
        // Arrange
        let mut e = Emu::new();

        // Act
        e.set_link_stage(7);

        // Assert
        assert_eq!(e.status().link_detail, "pair");
    }

    #[test]
    fn link_stage_out_of_range_hides_it() {
        // Arrange
        let mut e = Emu::new();
        e.set_link_stage(7);

        // Act
        e.set_link_stage(LINK_STAGES.len());

        // Assert
        assert_eq!(e.status().link_detail, "");
    }

    #[test]
    fn link_stage_ffi_round_trips_its_index() {
        // Arrange
        let e = kb_emu_new();

        // Act
        kb_emu_set_link_stage(e, 11);

        // Assert
        assert_eq!(kb_emu_link_stage(e), 11);
        kb_emu_free(e);
    }

    #[test]
    fn profile_state_survives_deep_sleep() {
        // Arrange
        let mut e = Emu::new();
        fast_power(&mut e);
        e.select_profile(4);
        e.set_profile_connected(true);
        e.tick(1_000);

        // Act
        e.press(13, 5_000);

        // Assert
        let s = e.status();
        assert_eq!(
            (s.profile, s.profile_open, s.profile_connected),
            (4, false, true)
        );
    }

    #[test]
    fn first_display_update_sends_init_and_frame() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let writes = e.display_writes(0, 0);

        // Assert
        assert_eq!(writes[..ssd1306::INIT.len()], *ssd1306::INIT);
        assert_eq!(writes.len(), ssd1306::INIT.len() + 2);
    }

    #[test]
    fn unchanged_display_sends_nothing() {
        // Arrange
        let mut e = Emu::new();
        e.display_writes(0, 0);

        // Act
        let writes = e.display_writes(0, 10);

        // Assert
        assert_eq!(writes, [] as [std::vec::Vec<u8>; 0]);
    }

    #[test]
    fn layer_change_sends_new_frame_without_init() {
        // Arrange
        let mut e = Emu::new();
        e.display_writes(0, 0);
        e.press(36, 20);
        drain(&mut e, 20);
        e.display_writes(0, 30); // the slide starts from the old name

        // Act: after the layer name has slid in
        let writes = e.display_writes(0, 30 + kb_display::anim::LAYER_SLIDE_MS);

        // Assert
        assert_eq!(writes.len(), 2);
    }

    #[test]
    fn switched_off_right_half_ignores_its_keys() {
        // Arrange
        let mut e = Emu::new();
        e.set_half_power(1, false, 0);

        // Act: J, on the right half
        e.press(19, 10);

        // Assert: nothing for the engine to report
        assert!(e.poll(10).is_none());
    }

    #[test]
    fn right_half_links_again_after_switching_on() {
        // Arrange
        let mut e = Emu::new();
        e.set_half_power(1, false, 0);
        e.set_half_power(1, true, 100);

        // Act
        e.tick(100 + RELINK_MS);

        // Assert
        assert!(e.status().peripheral_connected);
    }

    #[test]
    fn switching_a_half_off_drops_the_link() {
        // Arrange
        let mut e = Emu::new();

        // Act
        e.set_half_power(1, false, 0);

        // Assert
        assert!(!e.status().peripheral_connected);
    }

    #[test]
    fn switched_off_half_has_a_dark_screen() {
        // Arrange
        let mut e = Emu::new();
        e.display_writes(1, 0);
        e.set_half_power(1, false, 10);

        // Act
        let writes = e.display_writes(1, 20);

        // Assert
        assert_eq!(writes, [vec![0x00, 0xAE]]);
    }

    #[test]
    fn reset_plays_the_start_up_animation() {
        // Arrange
        let mut e = Emu::new();
        let still = e.display_writes(1, 0);
        e.reset_half(1, 10);

        // Act
        let writes = e.display_writes(1, 20);

        // Assert: a new frame (the animation), not the still status screen
        assert!(!still.is_empty() && !writes.is_empty());
    }

    fn send_notification(e: &mut Emu, text: &str, now: u32) -> RawHidResult {
        let mut result = RawHidResult::Pending;
        kb_core::host::encode_notification(text, 1, |r| result = e.raw_hid(r, now));
        result
    }

    #[test]
    fn notification_from_host_reaches_the_right_half() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let result = send_notification(&mut e, "Build passed", 0);

        // Assert
        assert_eq!(result, RawHidResult::Delivered);
        assert_eq!(
            e.status().notification.map(|t| t.as_str().to_string()),
            Some("Build passed".into())
        );
    }

    #[test]
    fn notification_is_dropped_when_right_half_is_not_linked() {
        // Arrange
        let mut e = Emu::new();
        e.status_mut().peripheral_connected = false;

        // Act
        let result = send_notification(&mut e, "Build passed", 0);

        // Assert
        assert_eq!(result, RawHidResult::NotLinked);
        assert!(e.status().notification.is_none());
    }

    #[test]
    fn notification_disappears_after_timeout() {
        // Arrange
        let mut e = Emu::new();
        send_notification(&mut e, "Build passed", 1_000);

        // Act
        e.tick(1_000 + NOTIFICATION_MS);

        // Assert
        assert!(e.status().notification.is_none());
    }

    #[test]
    fn notification_redraws_the_right_display() {
        // Arrange
        let mut e = Emu::new();
        e.display_writes(1, 0);
        send_notification(&mut e, "Build passed", 10);

        // Act
        let writes = e.display_writes(1, 20);

        // Assert
        assert_eq!(writes.len(), 2);
    }

    fn fast_power(e: &mut Emu) {
        e.set_power_config(
            PowerConfig {
                idle_ms: 100,
                sleep_ms: Some(1_000),
            },
            0,
        );
    }

    #[test]
    fn idle_shows_the_sleep_picture_first() {
        // Arrange
        let mut e = Emu::new();
        fast_power(&mut e);
        e.display_writes(0, 0);
        e.tick(100);
        e.display_writes(0, 100); // starts from the screen as it was

        // Act: the screen melting down
        let writes = e.display_writes(0, 600);

        // Assert: a new frame, the display still on
        assert_ne!(writes, [] as [std::vec::Vec<u8>; 0]);
        assert!(!writes.contains(&vec![0x00, 0xAE]));
    }

    #[test]
    fn idle_switches_displays_off_after_the_sleep_picture() {
        // Arrange
        let mut e = Emu::new();
        fast_power(&mut e);
        e.display_writes(0, 0);
        e.tick(100);
        e.display_writes(0, 100);

        // Act
        let writes = e.display_writes(0, 100 + kb_display::anim::SLEEP_ART_MS);

        // Assert
        assert_eq!(writes, [vec![0x00, 0xAE]]);
    }

    #[test]
    fn key_press_after_idle_switches_display_back_on() {
        // Arrange
        let mut e = Emu::new();
        fast_power(&mut e);
        e.display_writes(0, 0);
        e.tick(100);
        e.display_writes(0, 100);
        let off = 100 + kb_display::anim::SLEEP_ART_MS;
        e.display_writes(0, off);
        e.press(13, off + 100);

        // Act
        let writes = e.display_writes(0, off + 110);

        // Assert
        assert_eq!(writes[0], [0x00, 0xAF]);
    }

    #[test]
    fn idle_and_sleep_are_reported_as_events() {
        // Arrange
        let mut e = Emu::new();
        fast_power(&mut e);
        e.tick(100);
        e.tick(1_000);

        // Act
        let events: Vec<_> = std::iter::from_fn(|| e.poll(1_000)).collect();

        // Assert
        assert_eq!(
            events,
            [
                Event::Power(PowerState::Idle),
                Event::Power(PowerState::Sleep)
            ]
        );
    }

    #[test]
    fn key_that_wakes_from_deep_sleep_is_lost() {
        // Arrange
        let mut e = Emu::new();
        fast_power(&mut e);
        e.tick(1_000);
        drain(&mut e, 1_000);

        // Act
        e.press(13, 5_000);
        let events: Vec<_> = std::iter::from_fn(|| e.poll(5_000)).collect();

        // Assert: woke up, but no key report
        assert_eq!(events, [Event::Power(PowerState::Active)]);
    }

    #[test]
    fn wake_from_deep_sleep_reinitializes_the_displays() {
        // Arrange
        let mut e = Emu::new();
        fast_power(&mut e);
        e.display_writes(0, 0);
        e.tick(1_000);
        e.display_writes(0, 1_000);
        e.press(13, 5_000);

        // Act
        let writes = e.display_writes(0, 5_010);

        // Assert
        assert_eq!(writes[..ssd1306::INIT.len()], *ssd1306::INIT);
    }

    #[test]
    fn bluetooth_profile_survives_deep_sleep() {
        // Arrange
        let mut e = Emu::new();
        fast_power(&mut e);
        e.select_profile(3);
        e.tick(1_000);

        // Act
        e.press(13, 5_000);

        // Assert
        assert_eq!(e.status().profile, 3);
    }

    #[test]
    fn notification_cannot_arrive_during_deep_sleep() {
        // Arrange
        let mut e = Emu::new();
        fast_power(&mut e);
        e.tick(1_000);

        // Act
        let result = send_notification(&mut e, "Hi", 1_100);

        // Assert
        assert_eq!(result, RawHidResult::Asleep);
    }

    #[test]
    fn notification_wakes_idle_screens() {
        // Arrange
        let mut e = Emu::new();
        fast_power(&mut e);
        e.tick(100);

        // Act
        send_notification(&mut e, "Hi", 150);

        // Assert
        assert_eq!(e.power_state(), PowerState::Active);
    }

    #[test]
    fn wm_layer_keys_are_labelled_as_window_manager_actions() {
        // Arrange
        let mut e = Emu::new();
        e.press(36, 0);
        e.press(41, 0);
        drain(&mut e, 0);

        // Act
        let labels = [
            e.label(13),
            e.label(1),
            e.label(29),
            e.label(18),
            e.label(19),
            e.label(30),
        ];

        // Assert: digit positions go, shifted-digit positions send, arrows reorder
        assert_eq!(labels, ["WS 1", "→ 1", "WS 10", "Col ◀", "Win ▼", "WS ◀"]);
    }

    #[test]
    fn display_ffi_reports_size_and_keeps_writes_when_buffer_is_too_small() {
        // Arrange
        let e = kb_emu_new();
        let needed = kb_emu_display(e, 0, 0, std::ptr::null_mut(), 0);
        let mut buf = vec![0u8; needed];

        // Act
        let written = kb_emu_display(e, 0, 0, buf.as_mut_ptr(), buf.len());

        // Assert
        assert_eq!(written, needed);
        let first_len = u16::from_le_bytes([buf[0], buf[1]]) as usize;
        assert_eq!(first_len, ssd1306::INIT[0].len());
        kb_emu_free(e);
    }

    // ---- pairing with a code ------------------------------------------------------

    const LOWER: u8 = 36;
    const ENTER: u8 = 39;
    const POS_J: u8 = 19;
    const POS_K: u8 = 20;

    /// Positions of the digits on the symbols layer: 1..5, then 6..9, 0.
    fn digit_pos(d: u8) -> u8 {
        match d {
            1..=5 => 12 + d,
            6..=9 => 19 + d,
            _ => 29,
        }
    }

    /// Everything the emulator reported while `act` ran.
    fn events(e: &mut Emu, now: u32, act: impl FnOnce(&mut Emu)) -> Vec<Event> {
        drain(e, now);
        act(e);
        std::iter::from_fn(|| e.poll(now)).collect()
    }

    fn type_code(e: &mut Emu, code: &str, now: u32) {
        e.press(LOWER, now);
        for d in code.bytes().map(|b| b - b'0') {
            e.press(digit_pos(d), now);
            e.release(digit_pos(d), now);
        }
        e.release(LOWER, now);
    }

    fn pairing_results(events: &[Event]) -> Vec<PairResult> {
        events
            .iter()
            .filter_map(|ev| match ev {
                Event::Pairing(r) => Some(*r),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn simulated_pairing_asks_for_the_code_on_the_left_screen() {
        // Arrange
        let mut e = Emu::new();

        // Act
        e.simulate_pairing(123_456, 0);

        // Assert
        assert!(matches!(
            e.status().pairing,
            kb_display::status::Pairing::TypeCode { len: 0, .. }
        ));
    }

    #[test]
    fn typing_the_code_and_enter_pairs() {
        // Arrange
        let mut e = Emu::new();
        e.set_profile_open(true);
        e.simulate_pairing(120_456, 0);
        type_code(&mut e, "120456", 10);

        // Act
        let seen = events(&mut e, 20, |e| {
            e.press(ENTER, 20);
            e.release(ENTER, 20);
        });

        // Assert
        assert_eq!(pairing_results(&seen), [PairResult::Paired]);
        assert!(e.status().profile_connected);
        assert_eq!(e.status().pairing, kb_display::status::Pairing::Off);
    }

    #[test]
    fn typed_code_does_not_reach_the_computer() {
        // Arrange
        let mut e = Emu::new();
        e.simulate_pairing(111_111, 0);

        // Act
        let seen = events(&mut e, 10, |e| type_code(e, "111", 10));

        // Assert
        assert!(seen.iter().all(|ev| match ev {
            Event::Output(Output::Report(r)) => r.keys().next().is_none(),
            _ => true,
        }));
    }

    #[test]
    fn a_wrong_code_fails_the_pairing() {
        // Arrange
        let mut e = Emu::new();
        e.simulate_pairing(111_111, 0);
        type_code(&mut e, "222222", 10);

        // Act
        let seen = events(&mut e, 20, |e| {
            e.press(ENTER, 20);
            e.release(ENTER, 20);
        });

        // Assert
        assert_eq!(pairing_results(&seen), [PairResult::WrongCode]);
    }

    #[test]
    fn escape_rejects_the_pairing() {
        // Arrange
        let mut e = Emu::new();
        e.simulate_pairing(111_111, 0);

        // Act: Escape is the J + K combo
        let seen = events(&mut e, 10, |e| {
            e.press(POS_J, 10);
            e.press(POS_K, 15);
            e.release(POS_J, 30);
            e.release(POS_K, 30);
        });

        // Assert
        assert_eq!(pairing_results(&seen), [PairResult::Rejected]);
    }

    #[test]
    fn untyped_pairing_times_out() {
        // Arrange
        let mut e = Emu::new();
        e.simulate_pairing(111_111, 0);

        // Act
        let seen = events(&mut e, PAIR_TIMEOUT_MS, |e| e.tick(PAIR_TIMEOUT_MS));

        // Assert
        assert_eq!(pairing_results(&seen), [PairResult::TimedOut]);
    }

    // ---- radio power ----------------------------------------------------------------

    const RADIO: u8 = 29;

    #[test]
    fn radio_key_on_nav_steps_the_level() {
        // Arrange
        let mut e = Emu::new();

        // Act
        tap_on_nav(&mut e, RADIO, 0);

        // Assert
        assert_eq!((e.status().radio_dbm, e.status().radio_changes), (8, 1));
    }

    #[test]
    fn radio_key_is_labelled_radio() {
        // Arrange
        let mut e = Emu::new();
        e.press(NAV, 0);
        drain(&mut e, 0);

        // Act
        let label = e.label(RADIO);

        // Assert
        assert_eq!(label, "Radio");
    }

    /// Hold nav (position 41) and tap `StudioUnlock` (35) at `now`.
    fn press_studio_unlock(e: &mut Emu, now: u32) {
        e.press(41, now);
        e.press(35, now + 10);
        e.release(35, now + 20);
        e.release(41, now + 30);
        drain(e, now + 30);
    }

    fn studio_lines(e: &mut Emu, now: u32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut buf = vec![0u8; kb_emu_studio_line_max() as usize];
        loop {
            let n = kb_emu_studio_next_line(e, now, buf.as_mut_ptr(), buf.len());
            if n == 0 {
                return lines;
            }
            lines.push(String::from_utf8_lossy(&buf[..n]).trim_end().to_string());
        }
    }

    fn set(e: &mut Emu, layer: u8, pos: u8, enc: &str, now: u32) -> i32 {
        kb_emu_studio_set(e, layer, pos, enc.as_ptr(), enc.len(), now)
    }

    #[test]
    fn studio_unlock_key_unlocks_studio() {
        // Arrange
        let mut e = Emu::new();

        // Act
        press_studio_unlock(&mut e, 0);

        // Assert
        let remaining = kb_emu_studio_unlock_remaining_ms(&raw mut e, 100);
        assert!(
            remaining > kb_core::studio::UNLOCK_IDLE_MS - 1000,
            "{remaining}"
        );
        assert!(e.status().studio_unlocked);
    }

    #[test]
    fn studio_set_is_refused_without_the_unlock_key() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let code = set(&mut e, 0, 1, "kp:Z", 0);

        // Assert
        assert_eq!(code, 1);
        assert_eq!(e.label(1), "Q");
    }

    #[test]
    fn studio_edit_changes_what_the_key_types() {
        // Arrange
        let mut e = Emu::new();
        press_studio_unlock(&mut e, 0);
        assert_eq!(set(&mut e, 0, 1, "kp:Z", 100), 0);

        // Act
        e.press(1, 200);

        // Assert
        let mut typed = Vec::new();
        while let Some(event) = e.poll(200) {
            if let Event::Output(Output::Report(r)) = event {
                typed.extend(r.keys());
            }
        }
        assert_eq!(typed, [keys::Z.code]);
    }

    #[test]
    fn studio_edit_changes_the_label() {
        // Arrange
        let mut e = Emu::new();
        press_studio_unlock(&mut e, 0);
        set(&mut e, 0, 1, "mt:LCTRL/ESC", 100);

        // Act
        let mut buf = [0u8; 64];
        let n = kb_emu_label(&raw mut e, 1, buf.as_mut_ptr(), buf.len());

        // Assert
        assert_eq!(std::str::from_utf8(&buf[..n]).unwrap(), "Esc\nCtrl");
    }

    #[test]
    fn studio_guard_refuses_removing_the_unlock_key() {
        // Arrange
        let mut e = Emu::new();
        press_studio_unlock(&mut e, 0);

        // Act
        let code = set(&mut e, 2, 35, "trans", 100);

        // Assert
        assert_eq!(code, 5);
    }

    #[test]
    fn studio_saved_edit_survives_an_emulated_restart() {
        // Arrange
        let mut e = Emu::new();
        press_studio_unlock(&mut e, 0);
        set(&mut e, 0, 1, "kp:Z", 100);
        assert_eq!(kb_emu_studio_save(&raw mut e, 100), 0);
        set(&mut e, 0, 2, "kp:Z", 100);

        // Act
        kb_emu_reset_half(&raw mut e, 0, 200);

        // Assert: saved stays, unsaved is gone, locked again
        assert_eq!((e.label(1).as_str(), e.label(2).as_str()), ("Z", "W"));
        assert_eq!(kb_emu_studio_unlock_remaining_ms(&raw mut e, 200), 0);
    }

    #[test]
    fn studio_protocol_works_through_the_c_abi() {
        // Arrange
        let mut e = Emu::new();
        press_studio_unlock(&mut e, 0);
        let line = "#studio set 0 1 kp:lc(z)";
        kb_emu_studio_command(&raw mut e, line.as_ptr(), line.len(), 100);

        // Act
        let lines = studio_lines(&mut e, 100);

        // Assert
        assert_eq!(lines, ["#A set 0 1 ok kp:LC(Z)", "#U unlocked 300", "#D 1"]);
    }

    #[test]
    fn studio_line_too_long_for_the_buffer_is_kept() {
        // Arrange
        let mut e = Emu::new();
        let line = "#studio get";
        kb_emu_studio_command(&raw mut e, line.as_ptr(), line.len(), 0);
        let mut small = [0u8; 4];
        let needed = kb_emu_studio_next_line(&raw mut e, 0, small.as_mut_ptr(), small.len());

        // Act
        let mut big = vec![0u8; needed];
        let got = kb_emu_studio_next_line(&raw mut e, 0, big.as_mut_ptr(), big.len());

        // Assert
        assert_eq!(got, needed);
        assert!(big.starts_with(b"#V 2 "));
    }

    #[test]
    fn studio_canonical_and_label_of_an_encoding() {
        // Arrange
        let enc = "MT:lalt/tab";
        let mut buf = [0u8; 64];

        // Act
        let n = kb_emu_studio_canonical(enc.as_ptr(), enc.len(), buf.as_mut_ptr(), buf.len());
        let canonical = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
        let m = kb_emu_studio_label(enc.as_ptr(), enc.len(), buf.as_mut_ptr(), buf.len());

        // Assert
        assert_eq!(canonical, "mt:LALT/TAB");
        assert_eq!(std::str::from_utf8(&buf[..m]).unwrap(), "Tab\nAlt");
    }

    #[test]
    fn studio_invalid_encoding_has_no_canonical_form() {
        // Arrange
        let enc = "kp:NOPE";

        // Act
        let n = kb_emu_studio_canonical(enc.as_ptr(), enc.len(), std::ptr::null_mut(), 0);

        // Assert
        assert_eq!(n, -1);
    }

    #[test]
    fn studio_key_names_end_with_the_aliases() {
        // Arrange
        let mut buf = [0u8; 16];
        let mut names = Vec::new();

        // Act
        for i in 0.. {
            let n = kb_emu_studio_key_name(i, buf.as_mut_ptr(), buf.len());
            if n == 0 {
                break;
            }
            names.push(String::from_utf8_lossy(&buf[..n]).into_owned());
        }

        // Assert
        assert_eq!(names.first().map(String::as_str), Some("A"));
        assert_eq!(names.last().map(String::as_str), Some("MEH"));
    }

    fn text_of(mut f: impl FnMut(*mut u8, usize) -> usize) -> String {
        let mut buf = vec![0u8; 4096];
        let n = f(buf.as_mut_ptr(), buf.len());
        String::from_utf8_lossy(&buf[..n.min(buf.len())]).into_owned()
    }

    fn unlocked_emu() -> Emu {
        let mut e = Emu::new();
        press_studio_unlock(&mut e, 0);
        e
    }

    #[test]
    fn studio_layer_count_is_four() {
        // Arrange / Act
        let n = kb_emu_studio_layer_count();

        // Assert
        assert_eq!(n, 4);
    }

    #[test]
    fn studio_binding_is_the_raw_encoding() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let got = [
            text_of(|b, c| kb_emu_studio_binding(&raw mut e, 0, 12, b, c)),
            text_of(|b, c| kb_emu_studio_binding(&raw mut e, 1, 12, b, c)),
            text_of(|b, c| kb_emu_studio_binding(&raw mut e, 4, 0, b, c)),
        ];

        // Assert
        assert_eq!(got, ["mt:LALT/TAB", "trans", ""]);
    }

    #[test]
    fn studio_default_binding_stays_compiled() {
        // Arrange
        let mut e = unlocked_emu();
        set(&mut e, 0, 1, "kp:Z", 100);

        // Act
        let got = [
            text_of(|b, c| kb_emu_studio_default_binding(&raw mut e, 0, 1, b, c)),
            text_of(|b, c| kb_emu_studio_binding(&raw mut e, 0, 1, b, c)),
        ];

        // Assert
        assert_eq!(got, ["kp:Q", "kp:Z"]);
    }

    #[test]
    fn studio_layer_lists_every_position() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let layer = text_of(|b, c| kb_emu_studio_layer(&raw mut e, 2, b, c));

        // Assert
        let words: Vec<_> = layer.split(' ').collect();
        assert_eq!(words.len(), 42);
        assert_eq!(words[35], "sys:StudioUnlock");
        assert_eq!(text_of(|b, c| kb_emu_studio_layer(&raw mut e, 4, b, c)), "");
    }

    #[test]
    fn studio_check_works_locked_and_changes_nothing() {
        // Arrange
        let mut e = Emu::new();
        let (ok, bad) = ("kp:Z", "trans");

        // Act
        let codes = [
            kb_emu_studio_check(&raw mut e, 0, 1, ok.as_ptr(), ok.len()),
            kb_emu_studio_check(&raw mut e, 2, 35, bad.as_ptr(), bad.len()),
        ];

        // Assert
        assert_eq!(codes, [0, 5]);
        assert_eq!(e.label(1), "Q");
    }

    #[test]
    fn studio_result_codes_for_bad_input() {
        // Arrange
        let mut e = unlocked_emu();

        // Act
        let codes = [
            set(&mut e, 9, 0, "kp:A", 100),
            set(&mut e, 0, 0, "kp:NOPE", 100),
            set(
                &mut e,
                0,
                0,
                "kp:LC(LS(LA(LG(RC(RS(RA(RG(PAUSE_BREAK))))))))",
                100,
            ),
            kb_emu_studio_set(&raw mut e, 0, 0, [0xFFu8].as_ptr(), 1, 100),
            kb_emu_studio_set(std::ptr::null_mut(), 0, 0, std::ptr::null(), 0, 100),
        ];

        // Assert
        assert_eq!(codes, [2, 3, 4, -1, -1]);
    }

    #[test]
    fn studio_revert_drops_the_edit() {
        // Arrange
        let mut e = unlocked_emu();
        set(&mut e, 0, 1, "kp:Z", 100);

        // Act
        let code = kb_emu_studio_revert(&raw mut e);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(e.label(1), "Q");
        assert!(!kb_emu_studio_dirty(&raw mut e));
    }

    #[test]
    fn studio_reset_needs_the_unlock() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let code = kb_emu_studio_reset(&raw mut e, 0);

        // Assert
        assert_eq!(code, 1);
    }

    #[test]
    fn studio_reset_back_to_compiled() {
        // Arrange
        let mut e = unlocked_emu();
        set(&mut e, 0, 1, "kp:Z", 100);
        kb_emu_studio_save(&raw mut e, 100);

        // Act
        let code = kb_emu_studio_reset(&raw mut e, 100);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(e.label(1), "Q");
    }

    #[test]
    fn studio_lock_locks_and_shows_it() {
        // Arrange
        let mut e = unlocked_emu();

        // Act
        kb_emu_studio_lock(&raw mut e, 100);

        // Assert
        assert_eq!(kb_emu_studio_unlock_remaining_ms(&raw mut e, 100), 0);
        assert!(!e.status().studio_unlocked);
    }

    #[test]
    fn studio_unlock_times_out_on_tick() {
        // Arrange
        let mut e = unlocked_emu();

        // Act
        e.tick(kb_core::studio::UNLOCK_IDLE_MS + 1000);

        // Assert
        assert!(!e.status().studio_unlocked);
    }

    #[test]
    fn studio_dirty_follows_edits_and_saves() {
        // Arrange
        let mut e = unlocked_emu();
        set(&mut e, 0, 1, "kp:Z", 100);
        let before = kb_emu_studio_dirty(&raw mut e);

        // Act
        kb_emu_studio_save(&raw mut e, 100);

        // Assert
        assert!(before);
        assert!(!kb_emu_studio_dirty(&raw mut e));
    }

    #[test]
    fn studio_generation_changes_with_an_edit() {
        // Arrange
        let mut e = unlocked_emu();
        let before = kb_emu_studio_generation(&raw mut e);

        // Act
        set(&mut e, 0, 1, "kp:Z", 100);

        // Assert
        assert_ne!(kb_emu_studio_generation(&raw mut e), before);
    }

    #[test]
    fn studio_storage_path_reads_the_file() {
        // Arrange
        let dir = std::env::temp_dir().join(format!("kb-emu-abi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keymap.txt");
        std::fs::write(&path, "emk-keymap 2\n0 1 kp:Z\n").unwrap();
        let path = path.to_str().unwrap().to_string();
        let mut e = Emu::new();

        // Act
        let code = kb_emu_studio_set_storage_path(&raw mut e, path.as_ptr(), path.len());

        // Assert
        assert_eq!(code, 0);
        assert_eq!(e.label(1), "Z");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn studio_storage_path_empty_means_memory() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let code = kb_emu_studio_set_storage_path(&raw mut e, std::ptr::null(), 0);

        // Assert
        assert_eq!(code, 0);
    }

    #[test]
    fn studio_system_names_are_the_sys_names() {
        // Arrange / Act
        let names: Vec<_> = (0..8)
            .map(|i| text_of(|b, c| kb_emu_studio_system_name(i, b, c)))
            .collect();

        // Assert
        assert_eq!(names[0], "BtPrev");
        assert_eq!(names[4], "StudioUnlock");
        assert_eq!(names[7], "");
    }

    #[test]
    fn studio_command_ignores_other_lines() {
        // Arrange
        let mut e = Emu::new();
        let line = "s";

        // Act
        let taken = kb_emu_studio_command(&raw mut e, line.as_ptr(), line.len(), 0);

        // Assert
        assert!(!taken);
        assert_eq!(studio_lines(&mut e, 0), Vec::<String>::new());
    }

    #[test]
    fn studio_line_max_fits_a_layer_line() {
        // Arrange / Act
        let max = kb_emu_studio_line_max();

        // Assert
        assert!(max as usize >= kb_core::studio::protocol::layer_line_len(42));
    }

    // ---- combos ----------------------------------------------------------

    fn combo_add(e: &mut Emu, text: &str, now: u32) -> i32 {
        kb_emu_studio_combo_add(e, text.as_ptr(), text.len(), now)
    }

    #[test]
    fn studio_combo_max_is_sixteen() {
        // Arrange / Act
        let max = kb_emu_studio_combo_max();

        // Assert
        assert_eq!(max, 16);
    }

    #[test]
    fn studio_combos_start_compiled() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let (count, first, default_count, default_last) = (
            kb_emu_studio_combo_count(&raw mut e),
            text_of(|b, c| kb_emu_studio_combo(&raw mut e, 0, b, c)),
            kb_emu_studio_default_combo_count(&raw mut e),
            text_of(|b, c| kb_emu_studio_default_combo(&raw mut e, 5, b, c)),
        );

        // Assert
        assert_eq!(count, 6);
        assert_eq!(first, "cut 25,26 0 50 150 kp:LC(X)");
        assert_eq!(default_count, 6);
        assert_eq!(default_last, "reset_bluetooth 25,26 2 50 0 sys:BtClear");
        assert_eq!(text_of(|b, c| kb_emu_studio_combo(&raw mut e, 6, b, c)), "");
    }

    #[test]
    fn studio_combo_add_needs_the_unlock() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let code = combo_add(&mut e, "x 1,2 * 40 0 kp:A", 0);

        // Assert
        assert_eq!(code, 1);
    }

    #[test]
    fn studio_added_combo_types() {
        // Arrange: Q + W together type A
        let mut e = unlocked_emu();
        assert_eq!(combo_add(&mut e, "qw 1,2 0 50 0 kp:A", 100), 0);

        // Act
        e.press(1, 300);
        e.press(2, 310);

        // Assert
        let mut typed = Vec::new();
        while let Some(event) = e.poll(310) {
            if let Event::Output(Output::Report(r)) = event {
                typed.extend(r.keys());
            }
        }
        assert_eq!(typed, [keys::A.code]);
        assert_eq!(kb_emu_studio_combo_count(&raw mut e), 7);
    }

    #[test]
    fn studio_combo_set_changes_it() {
        // Arrange
        let mut e = unlocked_emu();
        let text = "cut 25,26 0 80 150 kp:LC(X)";

        // Act
        let code = kb_emu_studio_combo_set(&raw mut e, 0, text.as_ptr(), text.len(), 100);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(
            text_of(|b, c| kb_emu_studio_combo(&raw mut e, 0, b, c)),
            text
        );
    }

    #[test]
    fn studio_combo_remove_drops_it() {
        // Arrange
        let mut e = unlocked_emu();

        // Act
        let code = kb_emu_studio_combo_remove(&raw mut e, 0, 100);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(kb_emu_studio_combo_count(&raw mut e), 5);
        assert_eq!(kb_emu_studio_combo_remove(&raw mut e, 9, 100), 2);
    }

    #[test]
    fn studio_combo_result_codes() {
        // Arrange
        let mut e = unlocked_emu();

        // Act
        let codes = [
            combo_add(&mut e, "x 1,1 * 40 0 kp:A", 100),
            combo_add(&mut e, "a-b 1,2 * 40 0 kp:A", 100),
            combo_add(&mut e, "x 1,2 * 0 0 kp:A", 100),
            combo_add(&mut e, "x 1,2 *", 100),
            combo_add(&mut e, "x 1,2 * 40 0 kp:NOPE", 100),
        ];

        // Assert
        assert_eq!(codes, [8, 9, 10, 12, 3]);
    }

    #[test]
    fn studio_combo_full_is_code_11() {
        // Arrange
        let mut e = unlocked_emu();
        for i in 0..10 {
            assert_eq!(
                combo_add(&mut e, &format!("x {},{} 0 50 0 kp:A", i, i + 1), 100),
                0
            );
        }

        // Act
        let code = combo_add(&mut e, "x 30,31 0 50 0 kp:A", 100);

        // Assert
        assert_eq!(code, 11);
    }

    #[test]
    fn studio_combo_check_new_and_existing() {
        // Arrange
        let mut e = Emu::new();
        let (good, bad) = ("x 1,2 * 40 0 kp:A", "x 1,2 * 40 0 mo:9");

        // Act
        let codes = [
            kb_emu_studio_combo_check(&raw mut e, 255, good.as_ptr(), good.len()),
            kb_emu_studio_combo_check(&raw mut e, 0, good.as_ptr(), good.len()),
            kb_emu_studio_combo_check(&raw mut e, 9, good.as_ptr(), good.len()),
            kb_emu_studio_combo_check(&raw mut e, 255, bad.as_ptr(), bad.len()),
        ];

        // Assert
        assert_eq!(codes, [0, 0, 2, 3]);
        assert_eq!(kb_emu_studio_combo_count(&raw mut e), 6);
    }

    #[test]
    fn studio_combo_check_remove() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let codes = [
            kb_emu_studio_combo_check_remove(&raw mut e, 0),
            kb_emu_studio_combo_check_remove(&raw mut e, 6),
        ];

        // Assert
        assert_eq!(codes, [0, 2]);
    }

    #[test]
    fn studio_reset_bluetooth_combo_still_clears() {
        // Arrange: on nav, after an unrelated combo edit
        let mut e = unlocked_emu();
        combo_add(&mut e, "x 1,2 0 50 0 kp:A", 100);
        e.press(41, 1000);
        drain(&mut e, 1000);

        // Act
        e.press(25, 1100);
        e.press(26, 1110);

        // Assert
        let mut systems = Vec::new();
        while let Some(event) = e.poll(1110) {
            if let Event::Output(Output::System(s)) = event {
                systems.push(s);
            }
        }
        assert_eq!(systems, [SystemAction::BtClear]);
    }

    #[test]
    fn studio_saved_combo_survives_an_emulated_restart() {
        // Arrange
        let mut e = unlocked_emu();
        kb_emu_studio_combo_remove(&raw mut e, 0, 100);
        kb_emu_studio_save(&raw mut e, 100);

        // Act
        kb_emu_reset_half(&raw mut e, 0, 200);

        // Assert
        assert_eq!(kb_emu_studio_combo_count(&raw mut e), 5);
    }

    #[test]
    fn studio_protocol_defaults_through_the_c_abi() {
        // Arrange
        let mut e = Emu::new();
        let line = "#studio defaults";
        kb_emu_studio_command(&raw mut e, line.as_ptr(), line.len(), 0);

        // Act
        let lines = studio_lines(&mut e, 0);

        // Assert
        assert!(lines[0].starts_with("#LD 0 "));
        assert!(lines.contains(&"#CD end 6".to_string()));
    }

    // ---- mouse -------------------------------------------------------------

    /// Run the emulator from `from` to `to` ms like the app: tick every ms,
    /// poll everything; the mouse reports seen.
    fn mouse_run(e: &mut Emu, from: u32, to: u32) -> Vec<MouseReport> {
        let mut reports = Vec::new();
        for t in from..=to {
            e.tick(t);
            while let Some(event) = e.poll(t) {
                if let Event::Mouse(r) = event {
                    reports.push(r);
                }
            }
        }
        reports
    }

    fn mouse_state(e: &mut Emu) -> KbMouse {
        let mut out = KbMouse {
            x: 0,
            y: 0,
            wheel: 0,
            pan: 0,
            buttons: 0,
            pointer: 0,
            moving: 0,
            hires: 0,
        };
        assert!(kb_emu_mouse_state(e, &raw mut out));
        out
    }

    #[test]
    fn nav_mouse_keys_have_legends() {
        // Arrange
        let mut e = Emu::new();
        e.press(41, 0);
        drain(&mut e, 0);

        // Act
        let labels: Vec<_> = [14, 15, 16, 17, 2, 3, 4, 5, 13, 12, 37, 38]
            .iter()
            .map(|&p| e.label(p))
            .collect();

        // Assert
        assert_eq!(
            labels,
            [
                "◀ mouse",
                "▼ mouse",
                "▲ mouse",
                "▶ mouse",
                "F2\nscroll ←",
                "F3\nscroll ↓",
                "F4\nscroll ↑",
                "F5\nscroll →",
                "M click",
                "Precise",
                "Bksp\nL click",
                "Space\nR click"
            ]
        );
    }

    #[test]
    fn mouse_key_moves_the_pointer() {
        // Arrange: nav, then G (→) held for half a second
        let mut e = Emu::new();
        e.press(41, 0);
        e.press(17, 10);

        // Act
        let reports = mouse_run(&mut e, 10, 510);

        // Assert
        let state = mouse_state(&mut e);
        assert!(state.x > 50, "{}", state.x);
        assert_eq!(state.y, 0);
        assert_eq!(state.x, reports.iter().map(|r| i32::from(r.x)).sum::<i32>());
        assert_eq!(state.pointer, 1);
        assert_eq!(state.moving, 1);
    }

    #[test]
    fn mouse_reports_come_as_kind_7_events() {
        // Arrange
        let mut e = Emu::new();
        e.press(41, 0);
        drain(&mut e, 0);
        e.press(15, 10);

        // Act
        let mut ev = KbEvent {
            kind: 0,
            value: 0,
            mods: 0,
            reserved: 0,
            keys: [0; 32],
        };
        let mut seen = None;
        while kb_emu_poll(&raw mut e, 10, &raw mut ev) {
            if ev.kind == 7 {
                seen = Some(MouseReport::from_bytes(ev.keys[..7].try_into().unwrap()));
            }
        }

        // Assert: the nudge, 1 px down
        assert_eq!(seen.map(|r| (r.x, r.y)), Some((0, 1)));
    }

    #[test]
    fn moving_mouse_asks_for_a_tick_every_ms() {
        // Arrange
        let mut e = Emu::new();
        e.press(41, 0);
        e.press(17, 10);
        mouse_run(&mut e, 10, 20);

        // Act
        let next = kb_emu_next_deadline(&raw mut e);

        // Assert
        assert_eq!(next, 21);
    }

    #[test]
    fn releasing_the_mouse_key_stops_it() {
        // Arrange
        let mut e = Emu::new();
        e.press(41, 0);
        e.press(17, 10);
        mouse_run(&mut e, 10, 500);
        e.release(17, 500);
        mouse_run(&mut e, 500, 501);
        let at_release = mouse_state(&mut e).x;

        // Act
        mouse_run(&mut e, 502, 1500);

        // Assert
        assert_eq!(mouse_state(&mut e).x, at_release);
        assert_eq!(mouse_state(&mut e).moving, 0);
    }

    #[test]
    fn bluetooth_reports_come_every_8_ms() {
        // Arrange
        let mut e = Emu::new();
        e.set_transport(Transport::Ble);
        e.press(41, 0);
        e.press(17, 10);

        // Act
        let reports = mouse_run(&mut e, 10, 1010);

        // Assert: at most one per 8 ms (a few windows at the slow start
        // have no whole pixel yet)
        assert!(reports.len() <= 1000 / 8 + 2, "{}", reports.len());
        assert!(reports.len() >= 100, "{}", reports.len());
    }

    #[test]
    fn thumb_clicks_once_the_pointer_moved() {
        // Arrange
        let mut e = Emu::new();
        e.press(41, 0);
        e.press(17, 10);
        mouse_run(&mut e, 10, 50);
        e.release(17, 50);
        mouse_run(&mut e, 50, 60);

        // Act
        e.press(37, 100);
        mouse_run(&mut e, 100, 100);

        // Assert
        assert_eq!(mouse_state(&mut e).buttons, 1);
        assert!(e.host.keys().next().is_none(), "no Backspace typed");
    }

    #[test]
    fn thumb_types_backspace_without_a_move() {
        // Arrange
        let mut e = Emu::new();
        e.press(41, 0);
        drain(&mut e, 0);

        // Act
        e.press(37, 100);
        drain(&mut e, 100);

        // Assert
        assert_eq!(e.host.keys().collect::<Vec<_>>(), [keys::BSPC.code]);
        assert_eq!(mouse_state(&mut e).buttons, 0);
    }

    #[test]
    fn pointer_shows_the_mouse_bar_until_nav_goes() {
        // Arrange
        let mut e = Emu::new();
        e.press(41, 0);
        e.press(17, 10);
        mouse_run(&mut e, 10, 50);
        e.release(17, 50);
        mouse_run(&mut e, 50, 60);
        let during = e.status().pointer_active;

        // Act
        e.release(41, 100);
        drain(&mut e, 100);

        // Assert
        assert!(during);
        assert!(!e.status().pointer_active);
        assert_eq!(mouse_state(&mut e).pointer, 0);
    }

    #[test]
    fn scroll_key_turns_the_wheel_in_mouse_mode() {
        // Arrange: nav, a move, then R (scroll up in mouse mode)
        let mut e = Emu::new();
        e.press(41, 0);
        e.press(17, 10);
        mouse_run(&mut e, 10, 20);
        e.release(17, 20);
        mouse_run(&mut e, 20, 30);
        e.press(4, 30);

        // Act
        mouse_run(&mut e, 30, 100);

        // Assert: R is scroll up: one notch at once
        assert_eq!(mouse_state(&mut e).wheel, 1);
    }

    #[test]
    fn hires_scrolling_counts_sixteenths() {
        // Arrange: mouse mode, hi-res set by the "computer"
        let mut e = Emu::new();
        kb_emu_mouse_set_resolution(&raw mut e, true, true);
        e.press(41, 0);
        e.press(17, 10);
        mouse_run(&mut e, 10, 20);
        e.release(17, 20);
        mouse_run(&mut e, 20, 30);
        e.press(4, 30);

        // Act: a tap
        mouse_run(&mut e, 30, 100);

        // Assert: one detent of 16
        let state = mouse_state(&mut e);
        assert_eq!((state.wheel, state.hires), (16, 0b11));
    }

    #[test]
    fn reset_position_keeps_the_buttons() {
        // Arrange
        let mut e = Emu::new();
        e.press(41, 0);
        e.press(13, 10);
        mouse_run(&mut e, 10, 10);
        e.press(17, 20);
        mouse_run(&mut e, 20, 200);

        // Act
        kb_emu_mouse_reset_position(&raw mut e);

        // Assert
        let state = mouse_state(&mut e);
        assert_eq!((state.x, state.y, state.buttons), (0, 0, 4));
    }

    #[test]
    fn mouse_state_needs_a_place_to_write() {
        // Arrange
        let mut e = Emu::new();

        // Act
        let ok = kb_emu_mouse_state(&raw mut e, std::ptr::null_mut());

        // Assert
        assert!(!ok);
    }

    #[test]
    fn studio_can_bind_mouse_actions() {
        // Arrange
        let mut e = unlocked_emu();

        // Act
        let code = set(&mut e, 0, 0, "mk:left/ESC", 100);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(e.label(0), "Esc\nL click");
    }

    #[test]
    fn mouse_action_labels_through_studio() {
        // Arrange
        let enc = "ms:right";
        let mut buf = [0u8; 32];

        // Act
        let n = kb_emu_studio_label(enc.as_ptr(), enc.len(), buf.as_mut_ptr(), buf.len());

        // Assert
        assert_eq!(std::str::from_utf8(&buf[..n]).unwrap(), "scroll →");
    }
}

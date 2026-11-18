//! The development log port: the left half's USB serial port (`usb-log`).
//!
//! A reader opening the port (which raises DTR) starts a session: it gets
//! the whole log ring from its oldest complete line, then new lines as they
//! come. Commands (`kb_core::console`) are read all the time, also while
//! output waits for a slow reader or nobody reads at all. With `#debug 1` the
//! session also carries the live debug stream (`kb_core::debug`): held keys,
//! the report for the computer, the status and the OLED frame.
//!
//! `#studio ...` commands edit the keymap live (`kb_core::studio`): their
//! answers go out between log lines like the debug stream's, also without
//! `#debug 1`.
//!
//! When the reader goes away (DTR drops) the session ends: keys it pressed
//! are released and mute is switched off again, so a crashed tool can't
//! leave the keyboard holding a key or not typing.

use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicU8, Ordering::Relaxed};

use embassy_futures::select::{Either4, select3, select4};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex as Cs;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use kb_core::console::{Command, LINE_MAX, LineReader};
use kb_core::debug::{self, FRAME_BYTES, FRAME_LINE, Line, StatusFields};
use kb_core::keymaps::corne;
use kb_core::studio::protocol::{self as studio, Outbox, Storage, StudioCommand};
use kb_display::frame::Frame;
use kb_display::status::Transport;

use crate::log;

/// Something the debug stream shows changed.
static POKE: Signal<Cs, ()> = Signal::new();
/// The longest line a session sends: an OLED frame or a Studio layer.
const LINE: usize = if FRAME_LINE > studio::layer_line_len(corne::KEYS) {
    FRAME_LINE
} else {
    studio::layer_line_len(corne::KEYS)
};

/// Where Studio saves: flash with Bluetooth (its storage), RAM only without.
const STORAGE: Storage = if cfg!(feature = "ble") {
    Storage::Flash
} else {
    Storage::Ram
};

/// Keys held on both halves, bit n = position n.
static HELD: Mutex<Cs, Cell<u64>> = Mutex::new(Cell::new(0));
/// The last boot report for the computer (also when muted).
static REPORT: Mutex<Cs, Cell<[u8; 8]>> = Mutex::new(Cell::new([0; 8]));
/// The last mouse report for the computer (also when muted).
static MOUSE: Mutex<Cs, Cell<[u8; kb_core::mouse::REPORT_LEN]>> =
    Mutex::new(Cell::new([0; kb_core::mouse::REPORT_LEN]));
/// This half's battery, as last measured.
static BATTERY: AtomicU8 = AtomicU8::new(100);

/// What the OLED shows; `seq` counts changes.
struct Oled {
    seq: u32,
    on: bool,
    frame: [u8; FRAME_BYTES],
}

static OLED: Mutex<Cs, RefCell<Oled>> = Mutex::new(RefCell::new(Oled {
    seq: 0,
    on: false,
    frame: [0; FRAME_BYTES],
}));

/// Studio's lock changed (the unlock key): a Studio session says so.
pub fn poke() {
    POKE.signal(());
}

/// The keys held now (from the key loop).
pub fn set_held(held: u64) {
    if HELD.lock(|c| c.replace(held)) != held {
        POKE.signal(());
    }
}

/// The report just queued for the computer.
pub fn set_report(report: [u8; 8]) {
    REPORT.lock(|c| c.set(report));
    POKE.signal(());
}

/// The mouse report just queued for the computer.
pub fn set_mouse(report: [u8; kb_core::mouse::REPORT_LEN]) {
    MOUSE.lock(|c| c.set(report));
    POKE.signal(());
}

/// This half's battery level.
pub fn set_battery(pct: u8) {
    BATTERY.store(pct, Relaxed);
    POKE.signal(());
}

/// What the OLED shows now: a frame, or `None` while it's off.
pub fn show_frame(frame: Option<&Frame>) {
    let changed = OLED.lock(|o| {
        let mut o = o.borrow_mut();
        let same = match frame {
            None => !o.on,
            Some(f) => o.on && o.frame == f.buf,
        };
        if !same {
            o.on = frame.is_some();
            if let Some(f) = frame {
                o.frame = f.buf;
            }
            o.seq = o.seq.wrapping_add(1);
        }
        !same
    });
    if changed {
        POKE.signal(());
    }
}

/// The status as the debug stream reports it.
fn status_line(out: &mut Line<STATUS_LINE>) {
    let Some(screen) = crate::SNAPSHOT.lock(core::cell::Cell::get) else {
        out.clear();
        return;
    };
    let st = screen.status;
    let fields = StatusFields {
        layer: st.layer,
        caps_word: st.caps_word,
        mods: st.mods,
        output: match st.transport {
            Transport::Usb => kb_core::link::Output::Usb,
            Transport::Ble => kb_core::link::Output::Ble,
        },
        profile: st.profile,
        profile_connected: st.profile_connected,
        profile_open: st.profile_open,
        linked: st.peripheral_connected,
        stage: st.link_detail,
        battery: [BATTERY.load(Relaxed), st.battery[1]],
        charging: [crate::sleep::usb_powered(), false],
        wpm: st.wpm,
        power: screen.power,
        presses: st.key_presses,
        mute: log::MUTE.load(Relaxed),
        trace: log::TRACE.load(Relaxed),
        lit: OLED.lock(|o| o.borrow().on),
        radio_dbm: st.radio_dbm,
        pointer: st.pointer_active,
    };
    debug::status(out, &fields);
}

const STATUS_LINE: usize = 224;

/// What `boot` leaves in GPREGRET before the reset. The Adafruit nRF52
/// bootloader stays in DFU mode when it finds one of its magic values there:
/// 0x57 the UF2 drive plus serial DFU, 0x4E serial DFU only. A signed-only
/// bootloader (`SIGNED_FW`) has no drive and treats both as serial.
const DFU_MAGIC: u8 = if cfg!(feature = "signed-boot") {
    0x4E
} else {
    0x57
};

/// What one reader's session has sent so far.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags of one session, not a state machine"
)]
struct Session {
    /// Next byte of the log ring to send.
    pos: u32,
    /// Drop log bytes up to the next line end (the ring starts mid-line).
    skip_partial: bool,
    /// The last log byte sent wasn't a line end: no event line may follow yet.
    mid_line: bool,
    /// Send a line end first: ends whatever a previous reader left in the
    /// USB buffer, so the first line starts clean.
    greet: bool,
    debug: bool,
    sent_hello: bool,
    sent_keys: Option<u64>,
    sent_report: Option<[u8; 8]>,
    sent_mouse: Option<[u8; kb_core::mouse::REPORT_LEN]>,
    sent_frame: Option<u32>,
    sent_status: Line<STATUS_LINE>,
    /// Studio answers and changes still to send.
    studio: Outbox,
    /// The event line being sent, and how much of it is out.
    line: Line<LINE>,
    line_at: usize,
}

impl Session {
    const fn new() -> Self {
        Self {
            pos: 0,
            skip_partial: false,
            mid_line: false,
            greet: false,
            debug: false,
            sent_hello: false,
            sent_keys: None,
            sent_report: None,
            sent_mouse: None,
            sent_frame: None,
            sent_status: Line::new(),
            studio: Outbox::new(STORAGE),
            line: Line::new(),
            line_at: 0,
        }
    }

    /// A new reader: the whole log from its oldest complete line, no debug stream.
    fn start(&mut self) {
        *self = Self::new();
        self.replay();
    }

    /// Send the log again from its oldest complete line (`d`).
    fn replay(&mut self) {
        self.pos = log::ring().oldest();
        // Once the ring has wrapped, its oldest byte is mid-line.
        self.skip_partial = self.pos > 0;
        self.greet = true;
        self.mid_line = false;
    }

    /// Start (or restart: everything is sent again) or stop the debug stream.
    fn set_debug(&mut self, on: bool) {
        self.debug = on;
        self.sent_hello = false;
        self.sent_keys = None;
        self.sent_report = None;
        self.sent_mouse = None;
        self.sent_frame = None;
        self.sent_status.clear();
    }

    /// The next packet to send (at most 63 bytes: a full 64-byte packet would
    /// leave the computer waiting for more), or 0 when there is nothing.
    fn next_packet(&mut self, out: &mut [u8; 64]) -> usize {
        if self.line_at < self.line.as_bytes().len() {
            return self.line_packet(out);
        }
        if self.greet {
            self.greet = false;
            out[..2].copy_from_slice(b"\r\n");
            return 2;
        }
        if !self.mid_line && self.next_studio_line() {
            return self.line_packet(out);
        }
        if self.debug && !self.mid_line && self.next_event() {
            return self.line_packet(out);
        }
        loop {
            let mut n = log::ring().read(&mut self.pos, &mut out[..63]);
            if n == 0 {
                return 0;
            }
            if self.skip_partial {
                match out[..n].iter().position(|&b| b == b'\n') {
                    Some(end) => {
                        self.skip_partial = false;
                        out.copy_within(end + 1..n, 0);
                        n -= end + 1;
                        if n == 0 {
                            continue;
                        }
                    }
                    None => continue,
                }
            }
            self.mid_line = out[n - 1] != b'\n';
            return n;
        }
    }

    fn line_packet(&mut self, out: &mut [u8; 64]) -> usize {
        let rest = &self.line.as_bytes()[self.line_at..];
        let n = rest.len().min(63);
        out[..n].copy_from_slice(&rest[..n]);
        self.line_at += n;
        n
    }

    /// Put the next Studio line into `line`; false if there is none.
    fn next_studio_line(&mut self) -> bool {
        // A reader that never used Studio gets nothing from it.
        if !self.studio.active() {
            return false;
        }
        self.line_at = 0;
        let now = Instant::now().as_millis() as u32;
        crate::STUDIO.with(|s| self.studio.next_line(s, now, &mut self.line))
    }

    /// Run a Studio command; its answers go out with the next packets.
    fn studio_command(&mut self, command: StudioCommand) {
        let now = Instant::now().as_millis() as u32;
        crate::STUDIO.with(|s| self.studio.command(s, command, now));
        match command {
            StudioCommand::Save | StudioCommand::Reset | StudioCommand::Lock => {
                dlog!("studio: {:?}", command);
            }
            StudioCommand::ComboAdd { .. }
            | StudioCommand::ComboSet { .. }
            | StudioCommand::ComboRemove { .. } => dlog!("studio: combo edit"),
            _ => {}
        }
    }

    /// Put the next changed piece of state into `line`; false if none changed.
    /// Keys first: they change fastest and matter most.
    fn next_event(&mut self) -> bool {
        self.line_at = 0;
        if !self.sent_hello {
            self.sent_hello = true;
            let half = if cfg!(feature = "right") {
                "right"
            } else {
                "left"
            };
            let build = if cfg!(feature = "ble") {
                "ble"
            } else {
                "usb-only"
            };
            debug::hello(&mut self.line, crate::KEYBOARD_NAME, half, build);
            return true;
        }
        let held = HELD.lock(core::cell::Cell::get);
        if self.sent_keys != Some(held) {
            self.sent_keys = Some(held);
            debug::keys(&mut self.line, held);
            return true;
        }
        let report = REPORT.lock(core::cell::Cell::get);
        if self.sent_report != Some(report) {
            self.sent_report = Some(report);
            debug::report(&mut self.line, &report);
            return true;
        }
        let mouse = MOUSE.lock(core::cell::Cell::get);
        if self.sent_mouse != Some(mouse) {
            self.sent_mouse = Some(mouse);
            debug::mouse(&mut self.line, &mouse);
            return true;
        }
        let mut status = Line::<STATUS_LINE>::new();
        status_line(&mut status);
        if !status.as_bytes().is_empty() && status.as_bytes() != self.sent_status.as_bytes() {
            self.sent_status = status;
            self.line.clear();
            let _ = core::fmt::Write::write_str(
                &mut self.line,
                core::str::from_utf8(self.sent_status.as_bytes()).unwrap_or(""),
            );
            return true;
        }
        let (seq, on, frame) = OLED.lock(|o| {
            let o = o.borrow();
            (o.seq, o.on, o.frame)
        });
        if self.sent_frame != Some(seq) {
            self.sent_frame = Some(seq);
            debug::frame(&mut self.line, on.then_some(&frame));
            return true;
        }
        self.line.clear();
        false
    }
}

/// Serves the log port; see the module documentation.
#[cfg(not(all(feature = "ble", feature = "right")))]
#[embassy_executor::task]
pub async fn log_task(
    port: embassy_usb::class::cdc_acm::CdcAcmClass<'static, crate::UsbDriver>,
) -> ! {
    let (mut tx, mut rx, control) = port.split_with_control();
    let mut session = Session::new();
    let mut lines = LineReader::<LINE_MAX>::new();
    let mut input = [0u8; 64];
    let mut out = [0u8; 64];
    // A packet taken from the session but not yet accepted by USB: kept
    // across select rounds, so a cancelled write loses nothing.
    let mut out_len = 0;
    let mut open = false;
    // Hosts that never set DTR still get output once they send something.
    let mut dtr_seen = false;
    let mut rx_seen = false;
    // Keys pressed through this port and not yet released.
    let mut injected = 0u64;
    rx.wait_connection().await;
    loop {
        dtr_seen |= control.dtr();
        let reader = control.dtr() || (!dtr_seen && rx_seen);
        if reader != open {
            open = reader;
            out_len = 0;
            reader_changed(open, &mut session, &mut lines, &mut injected).await;
        }
        if open && out_len == 0 {
            out_len = session.next_packet(&mut out);
        }
        // Studio watches the unlock key and its timeout like the debug stream.
        let (writing, idle, debug) = (
            open && out_len > 0,
            open && out_len == 0,
            session.debug || session.studio.active(),
        );
        // A reader sending Studio commands faster than it reads the answers
        // waits until they are out.
        let reading = session.studio.backlog() <= studio::ACKS - 24;
        let event = select4(
            only_if(writing, tx.write_packet(&out[..out_len])),
            only_if(reading, rx.read_packet(&mut input)),
            control.control_changed(),
            more_to_send(idle, debug),
        )
        .await;
        match event {
            Either4::First(Ok(())) => out_len = 0,
            Either4::First(Err(_)) | Either4::Second(Err(_)) => {
                // USB unconfigured (unplugged, reset): wait for the next setup.
                out_len = 0;
                open = false;
                dtr_seen = false;
                rx_seen = false;
                end_session(&mut injected).await;
                rx.wait_connection().await;
            }
            Either4::Second(Ok(n)) => {
                rx_seen = true;
                take_input(&input[..n], &mut lines, &mut session, &mut injected).await;
            }
            Either4::Third(()) | Either4::Fourth(()) => {}
        }
    }
}

/// `future`'s output, or never when `wanted` is false (`future` is then
/// dropped without being polled).
async fn only_if<F: core::future::Future>(wanted: bool, future: F) -> F::Output {
    if wanted {
        future.await
    } else {
        core::future::pending().await
    }
}

/// Returns once there may be more to send to an `idle` reader: a new log
/// line, or with the `debug` stream (or Studio) on, a status change.
async fn more_to_send(idle: bool, debug: bool) {
    match (idle, debug) {
        // Not every status change pokes (the link stage, say):
        // look again now and then.
        (true, true) => {
            select3(
                log::ring().wait_new(),
                POKE.wait(),
                Timer::after(Duration::from_millis(250)),
            )
            .await;
        }
        (true, false) => log::ring().wait_new().await,
        _ => core::future::pending().await,
    }
}

/// A reader came (`open`) or went: end the old session and start afresh.
async fn reader_changed(
    open: bool,
    session: &mut Session,
    lines: &mut LineReader<LINE_MAX>,
    injected: &mut u64,
) {
    end_session(injected).await;
    if open {
        session.start();
        lines.reset();
        dlog!("log port: reader connected");
    } else {
        session.set_debug(false);
        dlog!("log port: reader gone");
    }
}

/// Bytes the reader sent: run the commands of the lines they complete (up
/// to 24 per packet).
async fn take_input(
    input: &[u8],
    lines: &mut LineReader<LINE_MAX>,
    session: &mut Session,
    injected: &mut u64,
) {
    let mut commands = [None; 24];
    let mut count = 0;
    lines.feed(input, |c| {
        if let Some(slot) = commands.get_mut(count) {
            *slot = Some(c);
            count += 1;
        }
    });
    for command in commands.into_iter().flatten() {
        run(command, session, injected).await;
    }
}

/// A reader went away (or a new one came): release what it held, unmute.
async fn end_session(injected: &mut u64) {
    for pos in 0..64u8 {
        if *injected & (1 << pos) != 0 {
            log::INJECT.send((pos, false)).await;
        }
    }
    *injected = 0;
    if log::MUTE.swap(false, Relaxed) {
        dlog!("mute: off");
    }
}

async fn run(command: Command, session: &mut Session, injected: &mut u64) {
    let key = |p: u8| (p as usize) < corne::KEYS;
    match command {
        Command::Status => log_status(),
        Command::Dump => session.replay(),
        Command::Trace => {
            let on = !log::TRACE.load(Relaxed);
            log::TRACE.store(on, Relaxed);
            dlog!("trace: {}", if on { "on" } else { "off" });
        }
        Command::Mute => {
            let on = !log::MUTE.load(Relaxed);
            log::MUTE.store(on, Relaxed);
            dlog!("mute: {}", if on { "on" } else { "off" });
        }
        Command::Debug(on) => {
            session.set_debug(on);
            dlog!("debug stream: {}", if on { "on" } else { "off" });
        }
        Command::Studio(command) => session.studio_command(command),
        Command::Bootloader => {
            dlog!(
                "restarting into the bootloader ({})",
                if cfg!(feature = "signed-boot") {
                    "serial DFU"
                } else {
                    "UF2"
                }
            );
            Timer::after(Duration::from_millis(100)).await;
            embassy_nrf::pac::POWER
                .gpregret()
                .write(|w| w.set_gpregret(DFU_MAGIC));
            cortex_m::peripheral::SCB::sys_reset();
        }
        Command::Press(p) if key(p) => {
            *injected |= 1 << p;
            log::INJECT.send((p, true)).await;
        }
        Command::Release(p) if key(p) => {
            *injected &= !(1 << p);
            log::INJECT.send((p, false)).await;
        }
        // A tap is a press, then a release a little later (no combo, hold or
        // bounce window cares about 30 ms).
        Command::Tap(p) if key(p) => {
            log::INJECT.send((p, true)).await;
            Timer::after(Duration::from_millis(30)).await;
            log::INJECT.send((p, false)).await;
            Timer::after(Duration::from_millis(30)).await;
        }
        Command::Press(_) | Command::Release(_) | Command::Tap(_) => {}
    }
}

fn log_status() {
    match crate::SNAPSHOT.lock(core::cell::Cell::get) {
        Some(screen) => {
            let st = screen.status;
            dlog!(
                "status: power {:?}, layer {}, caps {}, {:?} profile {}, battery {}% (right {}%), wpm {}, radio {:+} dBm",
                screen.power,
                st.layer,
                st.caps_word,
                st.transport,
                st.profile + 1,
                BATTERY.load(Relaxed),
                st.battery[1],
                st.wpm,
                st.radio_dbm
            );
        }
        None => dlog!("status: no screen yet"),
    }
    #[cfg(feature = "ble")]
    dlog!(
        "status: split {} (linked: {})",
        crate::ble::link().stage_name(),
        crate::ble::link().linked.load(Relaxed)
    );
}

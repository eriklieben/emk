//! Diagnostic build (feature `stage-led`, `STAGE_LED=1 ./fw-nrf/build-uf2`):
//! the board's LED (P0.15, the one the bootloader pulses) shows how far the
//! start got, for boards without a screen or a working USB port.
//!
//! - `n` blinks, then a pause: step `n` was the last one reached (`main.rs`
//!   numbers them with [`reached`]). A hang keeps blinking that number, since
//!   the blinking runs from the `SysTick` exception, not from the executor.
//! - On all the time: a crash report from the previous run is waiting.
//! - A crash in this run: the step it crashed in, blinked the same way; a hard
//!   fault adds a long flash (1.5 s on) after the pause, a panic doesn't.

use core::sync::atomic::{AtomicU8, Ordering};

use cortex_m::peripheral::syst::SystClkSource;
use cortex_m_rt::{ExceptionFrame, exception};
use embassy_nrf::pac;

/// The LED's pin on port 0.
const LED: usize = 15;
/// `SysTick` rate: 100 per second on the 64 MHz core clock.
const TICKS_PER_SECOND: u32 = 100;
const CORE_HZ: u32 = 64_000_000;
/// One blink: 250 ms on, 250 ms off.
const BLINK: u32 = TICKS_PER_SECOND / 2;
/// Dark between two rounds of blinks.
const PAUSE: u32 = TICKS_PER_SECOND * 3 / 2;
/// [`STAGE`] value for "a crash report is waiting": LED on.
const SOLID: u8 = u8::MAX;

static STAGE: AtomicU8 = AtomicU8::new(0);

/// Takes the LED and starts the blinking at step 1. Call first in `main`.
pub fn init() {
    pac::P0
        .pin_cnf(LED)
        .write(|w| w.set_dir(pac::gpio::vals::Dir::Output));
    led(false);
    // SAFETY: SysTick is used by nothing else in this firmware (embassy's
    // timer runs on RTC1, MPSL on RTC0 and TIMER0); this is its only owner.
    let mut core = unsafe { cortex_m::Peripherals::steal() };
    core.SYST.set_clock_source(SystClkSource::Core);
    core.SYST.set_reload(CORE_HZ / TICKS_PER_SECOND - 1);
    core.SYST.clear_current();
    core.SYST.enable_interrupt();
    core.SYST.enable_counter();
    reached(1);
}

/// Step `n` of the start was reached.
pub fn reached(n: u8) {
    STAGE.store(n, Ordering::Relaxed);
}

/// A crash report from the previous run is waiting: LED on.
pub fn crash_waiting() {
    STAGE.store(SOLID, Ordering::Relaxed);
}

fn led(on: bool) {
    if on {
        pac::P0.outset().write(|w| w.set_pin(LED, true));
    } else {
        pac::P0.outclr().write(|w| w.set_pin(LED, true));
    }
}

/// What crashed: a Rust panic, or a fault of the processor itself.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Crash {
    Panic,
    Fault,
}

/// Wait `ticks` × 10 ms on the `SysTick` counter itself, so it works where
/// the `SysTick` exception can't run (inside a fault or a panic).
fn wait(ticks: u32) {
    // SAFETY: only reads the counter's wrap flag; nothing else owns SysTick.
    let mut core = unsafe { cortex_m::Peripherals::steal() };
    for _ in 0..ticks {
        while !core.SYST.has_wrapped() {}
    }
}

/// Blink the step the crash happened in, forever, without interrupts.
pub fn crashed(kind: Crash) -> ! {
    cortex_m::interrupt::disable();
    let stage = STAGE.load(Ordering::Relaxed);
    loop {
        for _ in 0..stage {
            led(true);
            wait(BLINK / 2);
            led(false);
            wait(BLINK / 2);
        }
        wait(PAUSE);
        if kind == Crash::Fault {
            led(true);
            wait(PAUSE);
            led(false);
            wait(BLINK);
        }
    }
}

/// Reload value that feeds a watchdog reload register.
const WDT_RELOAD: u32 = 0x6E52_4635;

/// A watchdog left running by the bootloader resets us unless it is fed:
/// feed every enabled reload register on every tick.
fn feed_watchdog() {
    let wdt = pac::WDT;
    if wdt.runstatus().read().runstatus() {
        let enabled = wdt.rren().read().0;
        for i in 0..8 {
            if enabled & (1 << i) != 0 {
                wdt.rr(i).write_value(pac::wdt::regs::Rr(WDT_RELOAD));
            }
        }
    }
}

#[exception]
fn SysTick() {
    static mut TICK: u32 = 0;
    feed_watchdog();
    *TICK = TICK.wrapping_add(1);
    let stage = STAGE.load(Ordering::Relaxed);
    if stage == SOLID {
        led(true);
        return;
    }
    let blinks = u32::from(stage) * BLINK;
    let t = *TICK % (blinks + PAUSE);
    led(t < blinks && t % BLINK < BLINK / 2);
}

#[exception]
unsafe fn HardFault(_frame: &ExceptionFrame) -> ! {
    crashed(Crash::Fault)
}

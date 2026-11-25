//! Diagnostics without a debug probe, shown on the OLED.
//!
//! - **Crash screen:** the panic handler writes the panic location and message
//!   to a RAM area that survives the reset (`.uninit`), then resets. The next
//!   boot finds it, shows it on the OLED and stops there instead of looping,
//!   so it can be read. Pressing reset once clears it and boots normally;
//!   double-tap reset still enters the bootloader.
//! - **Boot stages:** `stage("radio")` shows the step about to run, so a hang
//!   leaves its name on the screen.

use core::fmt::Write;
use core::mem::MaybeUninit;
use core::panic::PanicInfo;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};

const MAGIC: u32 = 0x5041_4e43; // "PANC"
const LEN: usize = 120;

#[repr(C)]
struct Record {
    magic: u32,
    len: u32,
    text: [u8; LEN],
}

#[unsafe(link_section = ".uninit.crash")]
static mut RECORD: MaybeUninit<Record> = MaybeUninit::uninit();

/// Text to show instead of the status screen: (title, text).
pub static MESSAGE: Signal<CriticalSectionRawMutex, (&str, Text)> = Signal::new();

/// A short fixed-capacity string.
#[derive(Clone, Copy)]
pub struct Text {
    buf: [u8; LEN],
    len: usize,
}

impl Text {
    pub fn new(s: &str) -> Self {
        let mut t = Self {
            buf: [0; LEN],
            len: 0,
        };
        let _ = t.write_str(s);
        t
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
    }
}

impl Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.chars() {
            let mut tmp = [0u8; 4];
            let bytes = c.encode_utf8(&mut tmp).as_bytes();
            if self.len + bytes.len() > LEN {
                return Ok(());
            }
            self.buf[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            self.len += bytes.len();
        }
        Ok(())
    }
}

/// Show a boot stage and give the display task a moment to draw it.
pub async fn stage(name: &'static str) {
    crate::dlog!("stage: {}", name);
    MESSAGE.signal(("BOOT", Text::new(name)));
    Timer::after(Duration::from_millis(30)).await;
}

/// Record the panic (<file:line> and message) for the next boot, then reset.
#[cfg_attr(
    feature = "stage-led",
    allow(dead_code, reason = "the diagnostic build blinks the LED instead")
)]
pub fn record_and_reset(info: &PanicInfo) -> ! {
    let mut text = Text::new("");
    if let Some(loc) = info.location() {
        let file = loc.file().rsplit('/').next().unwrap_or(loc.file());
        let _ = write!(text, "{}:{} ", file, loc.line());
    }
    let _ = write!(text, "{}", info.message());
    store_and_reset(&text)
}

/// Record a fault of the processor itself (where it happened: the program
/// counter and the return address) for the next boot, then reset. Look the
/// addresses up with `addr2line -e fw-nrf/target/thumbv7em-none-eabihf/release/fw-nrf`.
#[cfg_attr(
    feature = "stage-led",
    allow(dead_code, reason = "the diagnostic build blinks the LED instead")
)]
pub fn record_fault_and_reset(pc: u32, lr: u32) -> ! {
    let mut text = Text::new("");
    let _ = write!(text, "hard fault at pc=0x{pc:08x} lr=0x{lr:08x}");
    store_and_reset(&text)
}

fn store_and_reset(text: &Text) -> ! {
    // SAFETY: single core, interrupts are irrelevant this close to reset; the
    // record is plain bytes.
    unsafe {
        let r = (*core::ptr::addr_of_mut!(RECORD)).as_mut_ptr();
        (*r).len = text.len as u32;
        (*r).text = text.buf;
        (*r).magic = MAGIC;
    }
    cortex_m::peripheral::SCB::sys_reset()
}

/// The crash recorded before the last reset, if any (cleared on read).
pub fn take_crash() -> Option<Text> {
    // SAFETY: read once at boot before anything else touches the record.
    unsafe {
        let r = (*core::ptr::addr_of_mut!(RECORD)).as_mut_ptr();
        if (*r).magic != MAGIC {
            return None;
        }
        (*r).magic = 0;
        let record = core::ptr::read(r);
        let len = (record.len as usize).min(LEN);
        let mut text = Text::new("");
        text.buf[..len].copy_from_slice(&record.text[..len]);
        text.len = len;
        Some(text)
    }
}

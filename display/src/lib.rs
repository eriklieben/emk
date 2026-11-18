//! What the keyboard shows on its OLEDs, and how it gets there.
//!
//! ```text
//! Status ──screens::render(half, style)──► Frame (128×32, 1 bit/pixel)
//!        ──ssd1306::init/flush──────────► I²C writes to address 0x3C
//! ```
//!
//! The I²C bytes are exactly what firmware would put on the bus, so the
//! .NET emulator decodes them with a model of the SSD1306 controller rather
//! than reading the frame buffer directly.
#![cfg_attr(not(test), no_std)]

pub mod anim;
pub mod frame;
pub mod screens;
pub mod ssd1306;
pub mod status;
mod text;

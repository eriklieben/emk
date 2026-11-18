//! Hardware-independent keyboard logic.
//!
//! The engine turns key position events (with millisecond timestamps) into
//! HID reports. It has no knowledge of GPIO, USB or Bluetooth, so the same
//! code runs in unit tests, in the desktop simulator (`kb-sim`) and on the
//! keyboard itself.
//!
//! ```text
//! matrix scan → debounce → Engine::press/release/tick → Output (HID report, layer, ...)
//! ```
#![cfg_attr(not(test), no_std)]

pub mod action;
pub mod battery;
pub mod console;
pub mod debounce;
pub mod debug;
pub mod engine;
pub mod host;
pub mod keymaps;
pub mod keys;
pub mod link;
pub mod mouse;
pub mod power;
mod queue;
pub mod report;
pub mod studio;

pub use action::{Action, Direction, Key, MouseAction, MouseButton, SystemAction};
pub use engine::{Combo, ComboDef, ConditionalLayer, Config, Engine, Keymap, Output};
pub use report::Report;

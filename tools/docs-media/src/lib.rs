//! The README's pictures, made from the firmware's own code: the OLED
//! animations are played by `kb-emu` on a simulated clock and decoded from
//! the SSD1306 bytes; the keymap pictures come from
//! `kb_core::keymaps::corne`. Run by `./docs-media` (which also has the
//! .NET app render its keyboard scenes and screenshots).

pub mod assemble;
pub mod gifout;
pub mod img;
pub mod keymap_svg;
pub mod oled;
pub mod scenes;

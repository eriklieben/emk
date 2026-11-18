//! SSD1306 driver for the Corne's 128×32 OLED modules, sending the same
//! I²C bytes as ZMK.
//!
//! The sequence is copied from the Zephyr driver ZMK ships
//! (`zmkfirmware/zephyr` `v4.1.0+zmk-fixes`, `drivers/display/ssd1306.c`)
//! with the Corne shield's devicetree settings (`corne.dtsi`):
//! `segment-remap`, `com-invdir`, `com-sequential`, `inversion-on`,
//! `multiplex-ratio = <31>`, `prechargep = <0x22>`.
//!
//! Every function produces I²C write transactions (without the address
//! byte). Each transaction starts with a control byte: `0x00` = commands
//! follow, `0x40` = display data follows.

use crate::frame::{Frame, PAGES, WIDTH};

/// 7-bit I²C address of the module (SA0 low).
pub const ADDRESS: u8 = 0x3C;

const COMMANDS: u8 = 0x00;
const DATA: u8 = 0x40;

/// `inversion-on`: the controller inverts the picture (`A7`), so the frame
/// is sent with inverted bits and the net result is lit pixels for `true`.
pub const INVERSION_ON: bool = true;

/// Power-up transactions, in the order `ssd1306_init_device` sends them.
#[rustfmt::skip]
pub const INIT: &[&[u8]] = &[
    &[COMMANDS, 0xAE],                                     // display off
    &[COMMANDS, 0xD5, 0x80, 0xD9, 0x22, 0xDB, 0x20],       // clock 8/1, precharge 0x22, VCOMH 0x20
    &[COMMANDS, 0x40, 0xD3, 0x00, 0xDA, 0x02, 0xA8, 0x1F], // start line 0, offset 0, sequential COM, 32 rows
    &[COMMANDS, 0xA1, 0xC8],                               // segment remap, COM scan flipped
    &[COMMANDS, 0x8D, 0x14, 0x33],                         // charge pump on (+ SH1106 pump voltage byte)
    &[COMMANDS, 0xA4, if INVERSION_ON { 0xA7 } else { 0xA6 }], // show RAM, reverse display
    &[COMMANDS, 0x81, 0x80],                               // contrast 128 (CONFIG_SSD1306_DEFAULT_CONTRAST)
    &[COMMANDS, 0xAF],                                     // display on
];

/// Switches the panel off, keeping its RAM.
pub const DISPLAY_OFF: [u8; 2] = [COMMANDS, 0xAE];

/// Switches the panel back on.
pub const DISPLAY_ON: [u8; 2] = [COMMANDS, 0xAF];

/// Send the power-up sequence.
pub fn init(mut write: impl FnMut(&[u8])) {
    for transaction in INIT {
        write(transaction);
    }
}

/// Switch the panel off, keeping its RAM (ZMK blanking on idle: `ssd1306_suspend`).
pub fn display_off(mut write: impl FnMut(&[u8])) {
    write(&DISPLAY_OFF);
}

/// Switch the panel back on (`ssd1306_resume`).
pub fn display_on(mut write: impl FnMut(&[u8])) {
    write(&DISPLAY_ON);
}

/// Address-window header written before every frame.
pub const FLUSH_HEADER: [u8; 9] = [
    COMMANDS,
    0x20,
    0x00, // horizontal addressing
    0x21,
    0,
    (WIDTH - 1) as u8, // columns 0..127
    0x22,
    0,
    (PAGES - 1) as u8, // pages 0..3
];

/// The data transaction for `frame` (control byte + 512 pixel bytes), for
/// async drivers that send transactions themselves.
#[must_use]
pub fn frame_data(frame: &Frame) -> [u8; 1 + WIDTH * PAGES] {
    let mut data = [0u8; 1 + WIDTH * PAGES];
    data[0] = DATA;
    for (out, &byte) in data[1..].iter_mut().zip(frame.buf.iter()) {
        *out = if INVERSION_ON { !byte } else { byte };
    }
    data
}

/// Send a whole frame like `ssd1306_write_default`: one command
/// transaction for the address window, one data transaction for the pixels.
pub fn flush(frame: &Frame, mut write: impl FnMut(&[u8])) {
    write(&FLUSH_HEADER);
    write(&frame_data(frame));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flushed(frame: &Frame) -> Vec<Vec<u8>> {
        let mut writes = Vec::new();
        flush(frame, |w| writes.push(w.to_vec()));
        writes
    }

    #[test]
    fn init_sends_zmk_sequence_ending_with_display_on() {
        // Arrange
        let mut writes = Vec::new();

        // Act
        init(|w| writes.push(w.to_vec()));

        // Assert
        assert_eq!(writes.len(), 8);
        assert_eq!(writes[0], [COMMANDS, 0xAE]);
        assert_eq!(writes[7], [COMMANDS, 0xAF]);
    }

    #[test]
    fn flush_sets_window_then_sends_all_pixels_in_one_write() {
        // Arrange
        let frame = Frame::default();

        // Act
        let writes = flushed(&frame);

        // Assert
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0], [0x00, 0x20, 0x00, 0x21, 0, 127, 0x22, 0, 3]);
        assert_eq!(writes[1].len(), 1 + WIDTH * PAGES);
        assert_eq!(writes[1][0], DATA);
    }

    #[test]
    fn flush_sends_inverted_bits_because_of_inversion_on() {
        // Arrange
        let mut frame = Frame::default();
        frame.set(0, 0, true);

        // Act
        let writes = flushed(&frame);

        // Assert
        assert_eq!(writes[1][1], 0xFE);
        assert_eq!(writes[1][2], 0xFF);
    }
}

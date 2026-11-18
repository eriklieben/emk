//! HID keyboard usage → Linux input key code, for the virtual keyboard.
//! Same table as the kernel's `hid_keyboard[]` in `drivers/hid/hid-input.c`.

pub fn to_linux(usage: u8) -> Option<u16> {
    // Letters A..Z (0x04..0x1D) are not in alphabetical order in Linux codes.
    const LETTERS: [u16; 26] = [
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17,
        45, 21, 44,
    ];
    Some(match usage {
        0x04..=0x1D => LETTERS[(usage - 0x04) as usize],
        0x1E..=0x26 => u16::from(usage - 0x1E) + 2, // 1..9
        0x27 => 11,                                 // 0
        0x28 => 28,                                 // Enter
        0x29 => 1,                                  // Esc
        0x2A => 14,                                 // Backspace
        0x2B => 15,                                 // Tab
        0x2C => 57,                                 // Space
        0x2D => 12,                                 // -
        0x2E => 13,                                 // =
        0x2F => 26,                                 // [
        0x30 => 27,                                 // ]
        0x31 | 0x32 => 43,                          // \ and non-US #
        0x33 => 39,                                 // ;
        0x34 => 40,                                 // '
        0x35 => 41,                                 // `
        0x36 => 51,                                 // ,
        0x37 => 52,                                 // .
        0x38 => 53,                                 // /
        0x39 => 58,                                 // Caps Lock
        0x3A..=0x43 => u16::from(usage - 0x3A) + 59, // F1..F10
        0x44 => 87,                                 // F11
        0x45 => 88,                                 // F12
        0x68..=0x73 => u16::from(usage - 0x68) + 183, // F13..F24
        0x46 => 99,                                 // Print Screen
        0x47 => 70,                                 // Scroll Lock
        0x48 => 119,                                // Pause
        0x49 => 110,                                // Insert
        0x4A => 102,                                // Home
        0x4B => 104,                                // Page Up
        0x4C => 111,                                // Delete
        0x4D => 107,                                // End
        0x4E => 109,                                // Page Down
        0x4F => 106,                                // Right
        0x50 => 105,                                // Left
        0x51 => 108,                                // Down
        0x52 => 103,                                // Up
        0xE0 => 29,                                 // Left Ctrl
        0xE1 => 42,                                 // Left Shift
        0xE2 => 56,                                 // Left Alt
        0xE3 => 125,                                // Left GUI
        0xE4 => 97,                                 // Right Ctrl
        0xE5 => 54,                                 // Right Shift
        0xE6 => 100,                                // Right Alt
        0xE7 => 126,                                // Right GUI
        _ => return None,
    })
}

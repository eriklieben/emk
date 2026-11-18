//! Where the bonds are kept: this firmware's own flash region (`STORAGE` in
//! memory.x), written through MPSL so flash operations never disturb the
//! radio. The key-value map on it is `kb_ble::storage`.

use core::ops::Range;

unsafe extern "C" {
    static __storage_start: u8;
    static __storage_end: u8;
}

/// Flash addresses of the storage region, from the linker script.
pub fn range() -> Range<u32> {
    // Only the addresses of these linker symbols are taken, never their values.
    let start = &raw const __storage_start as u32;
    let end = &raw const __storage_end as u32;
    start..end
}

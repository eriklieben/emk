//! Host tests only: NOR flash in RAM with the nRF52840's geometry, which can
//! be told to fail its reads or writes (a worn or busy flash), and a store
//! over it as the halves build theirs.

use core::ops::Range;

use embassy_sync::mutex::Mutex;
use embedded_storage_async::nor_flash::{
    ErrorType, MultiwriteNorFlash, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};
use sequential_storage::cache::NoCache;
use sequential_storage::map::{MapConfig, MapStorage};

use crate::split::Store;

const SIZE: usize = 0x2000;

/// Erased RAM flash (two 4 KB pages).
pub struct TestFlash {
    bytes: [u8; SIZE],
    /// Every read fails while set.
    pub fail_reads: bool,
    /// Every write and erase fails while set.
    pub fail_writes: bool,
}

#[derive(Debug)]
pub struct Failed;

impl NorFlashError for Failed {
    fn kind(&self) -> NorFlashErrorKind {
        NorFlashErrorKind::Other
    }
}

impl ErrorType for TestFlash {
    type Error = Failed;
}

impl TestFlash {
    pub const fn new() -> Self {
        Self {
            bytes: [0xFF; SIZE],
            fail_reads: false,
            fail_writes: false,
        }
    }

    fn range(offset: u32, len: usize) -> Result<Range<usize>, Failed> {
        let start = offset as usize;
        let end = start.checked_add(len).ok_or(Failed)?;
        if end > SIZE {
            return Err(Failed);
        }
        Ok(start..end)
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the trait's methods are async; RAM answers at once"
)]
impl ReadNorFlash for TestFlash {
    const READ_SIZE: usize = 1;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        if self.fail_reads {
            return Err(Failed);
        }
        bytes.copy_from_slice(&self.bytes[Self::range(offset, bytes.len())?]);
        Ok(())
    }

    fn capacity(&self) -> usize {
        SIZE
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the trait's methods are async; RAM answers at once"
)]
impl NorFlash for TestFlash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        if self.fail_writes || to < from {
            return Err(Failed);
        }
        self.bytes[Self::range(from, (to - from) as usize)?].fill(0xFF);
        Ok(())
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        if self.fail_writes {
            return Err(Failed);
        }
        for (cell, &b) in self.bytes[Self::range(offset, bytes.len())?]
            .iter_mut()
            .zip(bytes)
        {
            *cell &= b;
        }
        Ok(())
    }
}

impl MultiwriteNorFlash for TestFlash {}

/// A store over a fresh [`TestFlash`], as `left::run` builds one.
pub fn store() -> Store<TestFlash> {
    Mutex::new(MapStorage::new(
        TestFlash::new(),
        MapConfig::new(0..SIZE as u32),
        NoCache::new(),
    ))
}

/// Everything in `log`, as text.
pub fn log_text(log: &crate::LogRing) -> std::string::String {
    let mut pos = log.oldest();
    let mut out = std::vec::Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let n = log.read(&mut pos, &mut chunk);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..n]);
    }
    std::string::String::from_utf8_lossy(&out).into_owned()
}

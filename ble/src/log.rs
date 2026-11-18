//! Development log: timestamped lines in a RAM ring buffer. The firmware
//! reads it over a USB serial port next to the keyboard; the right half also
//! sends its own to the left half over the split link. Lines written before
//! anyone reads are kept (the last 4 KB), so boot messages can be read
//! afterwards.

use core::cell::RefCell;
use core::fmt::{self, Write};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Instant;

const SIZE: usize = 4096;

struct Ring {
    buf: [u8; SIZE],
    /// Bytes written since boot; the newest byte is at `(written - 1) % SIZE`.
    written: u32,
}

/// The log: lines of `{ms:>7} text\r\n`.
pub struct LogRing {
    ring: Mutex<CriticalSectionRawMutex, RefCell<Ring>>,
    /// Signalled whenever a line is added.
    new: Signal<CriticalSectionRawMutex, ()>,
}

impl Default for LogRing {
    fn default() -> Self {
        Self::new()
    }
}

struct Line {
    buf: [u8; 160],
    len: usize,
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = self.buf.len() - self.len;
        let n = s.len().min(room);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

impl LogRing {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ring: Mutex::new(RefCell::new(Ring {
                buf: [0; SIZE],
                written: 0,
            })),
            new: Signal::new(),
        }
    }

    /// Add a line, with the time since boot in front.
    pub fn write_line(&self, args: fmt::Arguments<'_>) {
        let mut line = Line {
            buf: [0; 160],
            len: 0,
        };
        let _ = write!(line, "{:>7} ", Instant::now().as_millis());
        let _ = line.write_fmt(args);
        let _ = line.write_str("\r\n");
        self.ring.lock(|r| {
            let mut r = r.borrow_mut();
            for &b in &line.buf[..line.len] {
                let at = r.written as usize % SIZE;
                r.buf[at] = b;
                r.written = r.written.wrapping_add(1);
            }
        });
        self.new.signal(());
    }

    /// Position of the oldest byte still in the ring.
    pub fn oldest(&self) -> u32 {
        self.ring.lock(|r| {
            let w = r.borrow().written;
            w.saturating_sub(SIZE as u32)
        })
    }

    /// Copy bytes from `pos` on into `out`, advancing `pos`. Returns how many.
    /// If the reader fell behind by more than the ring holds, it skips ahead.
    pub fn read(&self, pos: &mut u32, out: &mut [u8]) -> usize {
        self.ring.lock(|r| {
            let r = r.borrow();
            if r.written.wrapping_sub(*pos) > SIZE as u32 {
                *pos = r.written - SIZE as u32;
            }
            let available = r.written.wrapping_sub(*pos) as usize;
            let n = available.min(out.len());
            for (i, o) in out[..n].iter_mut().enumerate() {
                *o = r.buf[(*pos as usize + i) % SIZE];
            }
            *pos = pos.wrapping_add(n as u32);
            n
        })
    }

    /// Wait until a line is added (since the last wait returned).
    pub async fn wait_new(&self) {
        self.new.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_flash::log_text;
    // The host's critical section, for the blocking mutex.
    use critical_section as _;

    #[test]
    fn new_log_is_empty() {
        // Arrange
        let log = LogRing::default();
        let mut pos = log.oldest();

        // Act
        let n = log.read(&mut pos, &mut [0u8; 16]);

        // Assert
        assert_eq!(n, 0);
    }

    #[test]
    fn line_ends_with_crlf_after_the_time() {
        // Arrange
        let log = LogRing::new();

        // Act
        ring_log!(log, "split: {}", "linked");

        // Assert
        let text = log_text(&log);
        assert!(text.ends_with(" split: linked\r\n"), "{text:?}");
        assert!(text[..7].trim_start().parse::<u64>().is_ok(), "{text:?}");
        assert_eq!(text.as_bytes()[7], b' ');
    }

    #[test]
    fn long_line_is_cut_to_160_bytes() {
        // Arrange
        let log = LogRing::new();
        let long = [b'x'; 300];

        // Act
        ring_log!(log, "{}", core::str::from_utf8(&long).unwrap());

        // Assert
        let text = log_text(&log);
        assert_eq!(text.len(), 160);
        assert!(!text.ends_with("\r\n"), "no room left for the line end");
    }

    #[test]
    fn reader_that_fell_behind_skips_to_the_oldest_kept_byte() {
        // Arrange
        let log = LogRing::new();
        for i in 0..100 {
            ring_log!(log, "line {:03} with some padding to fill the ring up", i);
        }
        let mut pos = 0;
        let mut out = [0u8; 8];

        // Act
        log.read(&mut pos, &mut out);

        // Assert
        assert_eq!(pos, log.oldest() + 8);
        assert!(log.oldest() > 0, "the ring wrapped");
    }

    #[test]
    fn read_in_chunks_gets_everything_in_order() {
        // Arrange
        let log = LogRing::new();
        ring_log!(log, "first");
        ring_log!(log, "second");
        let mut pos = log.oldest();
        let mut text = std::vec::Vec::new();

        // Act
        loop {
            let mut chunk = [0u8; 5];
            let n = log.read(&mut pos, &mut chunk);
            if n == 0 {
                break;
            }
            text.extend_from_slice(&chunk[..n]);
        }

        // Assert
        let text = std::string::String::from_utf8(text).unwrap();
        let first = text.find("first").unwrap();
        assert!(text[first..].contains("second"), "{text:?}");
    }
}

//! Looping GIFs from indexed frames: one global palette, and every frame
//! after the first only as the rectangle that changed, with the unchanged
//! pixels in it transparent (index 0 is reserved for that).

use std::borrow::Cow;

use crate::img::Indexed;

/// Palette index that means "keep the pixel of the frame before".
pub const KEEP: u8 = 0;

pub struct Animation {
    palette: Vec<[u8; 3]>,
    /// Frames and how long each shows (ms); equal frames in a row are merged.
    frames: Vec<(Indexed, u32)>,
}

impl Animation {
    /// `palette[0]` is never drawn (it is the transparent [`KEEP`]).
    ///
    /// # Panics
    ///
    /// With more than 256 colours.
    #[must_use]
    pub fn new(palette: &[[u8; 3]]) -> Self {
        assert!(
            palette.len() <= 256,
            "a GIF palette has 256 colours at most"
        );
        Self {
            palette: palette.to_vec(),
            frames: Vec::new(),
        }
    }

    pub fn push(&mut self, frame: Indexed, ms: u32) {
        match self.frames.last_mut() {
            Some((last, t)) if *last == frame => *t += ms,
            _ => self.frames.push((frame, ms)),
        }
    }

    /// Show the last frame longer before the loop starts again.
    pub fn hold_last(&mut self, ms: u32) {
        if let Some((_, t)) = self.frames.last_mut() {
            *t += ms;
        }
    }

    /// Frames so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether there are no frames yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The looping GIF of all frames; empty without frames.
    ///
    /// # Panics
    ///
    /// When a frame is larger than GIF allows (65535 pixels a side).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let Some((first, _)) = self.frames.first() else {
            return Vec::new();
        };
        let (w, h) = (first.width, first.height);
        // The palette is padded to a power of two, as GIF stores it.
        let size = self.palette.len().next_power_of_two().max(4);
        let mut flat: Vec<u8> = self.palette.iter().flatten().copied().collect();
        flat.resize(size * 3, 0);

        let mut out = Vec::new();
        {
            let mut enc =
                gif::Encoder::new(&mut out, w as u16, h as u16, &flat).expect("gif header");
            enc.set_repeat(gif::Repeat::Infinite).expect("gif loop");
            let mut prev: Option<&Indexed> = None;
            // Delays in centiseconds, rounded on the running total so the
            // whole keeps its length.
            let mut elapsed_ms = 0u32;
            for (img, ms) in &self.frames {
                let start_cs = (elapsed_ms + 5) / 10;
                elapsed_ms += ms;
                let delay = ((elapsed_ms + 5) / 10 - start_cs).max(2) as u16;
                let (x0, y0, x1, y1) = match prev {
                    None => (0, 0, w, h),
                    Some(p) => changed_box(p, img).unwrap_or((0, 0, 1, 1)),
                };
                let mut buf = Vec::with_capacity((x1 - x0) * (y1 - y0));
                for y in y0..y1 {
                    for x in x0..x1 {
                        let v = img.get(x, y);
                        buf.push(match prev {
                            Some(p) if p.get(x, y) == v => KEEP,
                            _ => v,
                        });
                    }
                }
                let frame = gif::Frame {
                    left: x0 as u16,
                    top: y0 as u16,
                    width: (x1 - x0) as u16,
                    height: (y1 - y0) as u16,
                    buffer: Cow::Owned(buf),
                    delay,
                    transparent: Some(KEEP),
                    dispose: gif::DisposalMethod::Keep,
                    ..gif::Frame::default()
                };
                enc.write_frame(&frame).expect("gif frame");
                prev = Some(img);
            }
        }
        out
    }
}

/// The smallest rectangle (x0, y0, x1, y1; ends exclusive) holding every
/// pixel that differs, or `None` when the frames are the same.
fn changed_box(a: &Indexed, b: &Indexed) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..a.height {
        let row = y * a.width;
        for x in 0..a.width {
            if a.px[row + x] != b.px[row + x] {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > 0).then_some((x0, y0, x1, y1))
}

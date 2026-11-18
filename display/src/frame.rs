//! 128×32 monochrome frame buffer in SSD1306 memory layout: 4 pages of
//! 128 columns, each byte a vertical strip of 8 pixels (bit 0 = top).

use core::convert::Infallible;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;

pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 32;
pub const PAGES: usize = HEIGHT / 8;

#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    pub buf: [u8; WIDTH * PAGES],
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            buf: [0; WIDTH * PAGES],
        }
    }
}

impl Frame {
    pub fn set(&mut self, x: usize, y: usize, on: bool) {
        if x < WIDTH && y < HEIGHT {
            let byte = &mut self.buf[(y / 8) * WIDTH + x];
            if on {
                *byte |= 1 << (y % 8);
            } else {
                *byte &= !(1 << (y % 8));
            }
        }
    }

    #[must_use]
    pub fn get(&self, x: usize, y: usize) -> bool {
        x < WIDTH && y < HEIGHT && self.buf[(y / 8) * WIDTH + x] & (1 << (y % 8)) != 0
    }

    #[must_use]
    pub fn page(&self, page: usize) -> &[u8] {
        &self.buf[page * WIDTH..(page + 1) * WIDTH]
    }
}

impl OriginDimensions for Frame {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl DrawTarget for Frame {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(p, color) in pixels {
            if p.x >= 0 && p.y >= 0 {
                self.set(p.x as usize, p.y as usize, color.is_on());
            }
        }
        Ok(())
    }
}

/// Which half a display sits on. The module is mounted vertically beside
/// the inner column. On the gray Corne both modules turned out to face the
/// same way (the right half's screen was upside down with a 180° rotation),
/// so landscape content reads bottom to top on both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Half {
    /// Left (central).
    Left,
    /// Right (peripheral).
    Right,
}

pub const PORTRAIT_WIDTH: usize = HEIGHT;
pub const PORTRAIT_HEIGHT: usize = WIDTH;

impl Half {
    /// Module pixel for a pixel of the upright 32×128 view on this half.
    #[must_use]
    pub const fn module_xy(self, px: usize, py: usize) -> (usize, usize) {
        match self {
            Self::Left | Self::Right => (WIDTH - 1 - py, px),
        }
    }
}

/// Draw upright (32 wide × 128 tall, as seen while typing) into a module
/// frame; the rotation for the half is done in software.
pub struct Portrait<'a> {
    pub frame: &'a mut Frame,
    pub half: Half,
}

impl OriginDimensions for Portrait<'_> {
    fn size(&self) -> Size {
        Size::new(PORTRAIT_WIDTH as u32, PORTRAIT_HEIGHT as u32)
    }
}

impl DrawTarget for Portrait<'_> {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(p, color) in pixels {
            if p.x >= 0
                && p.y >= 0
                && (p.x as usize) < PORTRAIT_WIDTH
                && (p.y as usize) < PORTRAIT_HEIGHT
            {
                let (x, y) = self.half.module_xy(p.x as usize, p.y as usize);
                self.frame.set(x, y, color.is_on());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_pixel_uses_ssd1306_page_layout() {
        // Arrange
        let mut f = Frame::default();

        // Act: y = 9 is page 1, bit 1
        f.set(3, 9, true);

        // Assert
        assert_eq!(f.buf[WIDTH + 3], 0b10);
        assert!(f.get(3, 9));
    }

    #[test]
    fn portrait_top_left_on_left_half_is_module_bottom_right_end() {
        // Arrange
        let mut f = Frame::default();

        // Act
        let _ = Pixel(Point::new(0, 0), BinaryColor::On).draw(&mut Portrait {
            frame: &mut f,
            half: Half::Left,
        });

        // Assert: left half reads bottom to top, so upright y=0 is module x=127
        assert!(f.get(WIDTH - 1, 0));
    }

    #[test]
    fn portrait_top_left_on_right_half_matches_left_half() {
        // Arrange
        let mut f = Frame::default();

        // Act
        let _ = Pixel(Point::new(0, 0), BinaryColor::On).draw(&mut Portrait {
            frame: &mut f,
            half: Half::Right,
        });

        // Assert: same orientation as the left half (checked on hardware)
        assert!(f.get(WIDTH - 1, 0));
    }

    #[test]
    fn clearing_a_pixel_turns_it_off() {
        // Arrange
        let mut f = Frame::default();
        f.set(3, 9, true);

        // Act
        f.set(3, 9, false);

        // Assert
        assert!(!f.get(3, 9));
    }
}

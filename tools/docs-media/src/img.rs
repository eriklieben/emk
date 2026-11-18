//! Pictures as palette indices (what a GIF stores), and a few ways to draw on them.
#![allow(
    clippy::many_single_char_names,
    reason = "drawing code: x, y, w, h, r are the clearest names"
)]

use core::convert::Infallible;
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};

/// An image of palette indices, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indexed {
    pub width: usize,
    pub height: usize,
    pub px: Vec<u8>,
}

impl Indexed {
    #[must_use]
    pub fn new(width: usize, height: usize, fill: u8) -> Self {
        Self {
            width,
            height,
            px: vec![fill; width * height],
        }
    }

    #[must_use]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.px[y * self.width + x]
    }

    /// Fill a rectangle, clipped to the image.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u8) {
        let (x0, y0) = (x.max(0), y.max(0));
        let x1 = (x + w).min(self.width as i32);
        let y1 = (y + h).min(self.height as i32);
        for yy in y0..y1 {
            let row = yy as usize * self.width;
            for xx in x0..x1 {
                self.px[row + xx as usize] = color;
            }
        }
    }

    /// Fill a rectangle with corners rounded by `r` pixels.
    pub fn fill_round_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, color: u8) {
        for yy in 0..h {
            for xx in 0..w {
                // Distance into the nearest corner's square.
                let dx = if xx < r {
                    r - xx
                } else if xx >= w - r {
                    xx - (w - r - 1)
                } else {
                    0
                };
                let dy = if yy < r {
                    r - yy
                } else if yy >= h - r {
                    yy - (h - r - 1)
                } else {
                    0
                };
                if dx > 0 && dy > 0 && dx * dx + dy * dy > r * r {
                    continue;
                }
                let (px, py) = (x + xx, y + yy);
                if px >= 0 && py >= 0 && (px as usize) < self.width && (py as usize) < self.height {
                    self.px[py as usize * self.width + px as usize] = color;
                }
            }
        }
    }

    /// Text in an embedded-graphics mono font, `scale` image pixels per font
    /// pixel, centred on `cx` with its top at `y`.
    pub fn text_centered(
        &mut self,
        s: &str,
        cx: i32,
        y: i32,
        font: &MonoFont<'_>,
        scale: i32,
        color: u8,
    ) {
        let mut pen = Pen {
            img: self,
            x: cx,
            y,
            scale,
            color,
        };
        let style = MonoTextStyle::new(font, BinaryColor::On);
        let layout = TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Top)
            .build();
        let _ = Text::with_text_style(s, Point::zero(), style, layout).draw(&mut pen);
    }

    /// RGB bytes through `palette`.
    #[must_use]
    pub fn to_rgb(&self, palette: &[[u8; 3]]) -> Vec<u8> {
        self.px.iter().flat_map(|&i| palette[i as usize]).collect()
    }
}

/// Draws font pixels as `scale`-sized squares around an origin.
struct Pen<'a> {
    img: &'a mut Indexed,
    x: i32,
    y: i32,
    scale: i32,
    color: u8,
}

impl OriginDimensions for Pen<'_> {
    fn size(&self) -> Size {
        Size::new(u32::MAX / 4, u32::MAX / 4)
    }
}

impl DrawTarget for Pen<'_> {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(p, c) in pixels {
            if c.is_on() {
                let s = self.scale;
                self.img
                    .fill_rect(self.x + p.x * s, self.y + p.y * s, s, s, self.color);
            }
        }
        Ok(())
    }
}

/// Write an RGB PNG.
///
/// # Panics
///
/// When `rgb` is not `width × height` pixels.
#[must_use]
pub fn png_rgb(width: usize, height: usize, rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width as u32, height as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Best);
        let mut writer = enc.write_header().expect("png header");
        writer.write_image_data(rgb).expect("png data");
    }
    out
}

/// An indexed PNG (smallest for a few flat colours).
///
/// # Panics
///
/// When the image doesn't match its size or has more than 256 colours.
#[must_use]
pub fn png_indexed(img: &Indexed, palette: &[[u8; 3]]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width as u32, img.height as u32);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(palette.iter().flatten().copied().collect::<Vec<_>>());
        enc.set_compression(png::Compression::Best);
        let mut writer = enc.write_header().expect("png header");
        writer.write_image_data(&img.px).expect("png data");
    }
    out
}

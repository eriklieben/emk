//! The OLED modules as the app's `CorneView` draws them: each 32 × 128
//! upright, a dot per pixel with a faint dot matrix on the unlit glass, in
//! a dark bezel, scaled up pixel-sharp.

use kb_display::frame::{Frame, Half, PORTRAIT_HEIGHT, PORTRAIT_WIDTH, WIDTH};

use crate::img::Indexed;

/// The palette of every OLED picture. Index 0 is the GIF's "unchanged".
pub const PALETTE: [[u8; 3]; 8] = [
    [0, 0, 0],
    // Plate around the bezels (the app's dark `plate`).
    [0x12, 0x16, 0x1c],
    // Bezel (`bezel`).
    [0x07, 0x09, 0x0c],
    // Glass between the dots (CorneView's #030506).
    [0x03, 0x05, 0x06],
    // An unlit dot: the OLED colour at 7 % over the glass.
    [0x0d, 0x15, 0x17],
    // A lit dot: #8fe6ff at the brightness of contrast 128 (0.45 + 0.55 × 128/255).
    [0x68, 0xa5, 0xbb],
    // Caption text.
    [0xd3, 0xd9, 0xe3],
    // Dim caption text.
    [0x6b, 0x74, 0x87],
];

pub const PLATE: u8 = 1;
pub const BEZEL: u8 = 2;
pub const GLASS: u8 = 3;
pub const DOT_OFF: u8 = 4;
pub const DOT_ON: u8 = 5;
pub const INK: u8 = 6;
pub const INK_DIM: u8 = 7;

/// An SSD1306 as far as the firmware's I²C writes go: on/off and its RAM.
/// The firmware only ever writes whole frames (the window is always all
/// 128 × 4 pages), and the display runs inverted (`A7`), so a lit pixel is
/// a 0 bit in RAM.
#[derive(Clone)]
pub struct Panel {
    on: bool,
    ram: [u8; WIDTH * 4],
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            on: false,
            ram: [0xFF; WIDTH * 4],
        }
    }
}

impl Panel {
    /// One I²C transaction: control byte `00` (commands) or `40` (data).
    pub fn write(&mut self, bytes: &[u8]) {
        match bytes.split_first() {
            Some((0x00, cmds)) => {
                // None of the parameters the firmware sends is AE or AF.
                for &c in cmds {
                    match c {
                        0xAE => self.on = false,
                        0xAF => self.on = true,
                        _ => {}
                    }
                }
            }
            Some((0x40, data)) => {
                let n = data.len().min(self.ram.len());
                self.ram[..n].copy_from_slice(&data[..n]);
            }
            _ => {}
        }
    }

    /// Whether module pixel (x, y) of the 128 × 32 panel lights.
    #[must_use]
    pub fn lit(&self, x: usize, y: usize) -> bool {
        self.on && self.ram[(y / 8) * WIDTH + x] & (1 << (y % 8)) == 0
    }

    /// Whether pixel (px, py) of the upright 32 × 128 view lights (both
    /// modules are mounted the same way: see [`Half::module_xy`]).
    #[must_use]
    pub fn lit_upright(&self, px: usize, py: usize) -> bool {
        let (x, y) = Half::Left.module_xy(px, py);
        self.lit(x, y)
    }

    /// A panel showing `frame`, switched on.
    #[must_use]
    pub fn showing(frame: &Frame) -> Self {
        let mut p = Self {
            on: true,
            ..Self::default()
        };
        for (r, b) in p.ram.iter_mut().zip(frame.buf.iter()) {
            *r = !b;
        }
        p
    }
}

/// Draw upright pixels `rows` of a panel at (x, y), `scale` image pixels per
/// OLED pixel: a dot of `scale - 1` with a 1-pixel gap of glass.
pub fn draw_glass(
    img: &mut Indexed,
    x: i32,
    y: i32,
    scale: i32,
    panel: &Panel,
    rows: core::ops::Range<usize>,
) {
    let h = rows.len() as i32;
    img.fill_rect(x, y, PORTRAIT_WIDTH as i32 * scale, h * scale, GLASS);
    let dot = (scale - 1).max(1);
    for (row, py) in rows.enumerate() {
        for px in 0..PORTRAIT_WIDTH {
            let c = if panel.lit_upright(px, py) {
                DOT_ON
            } else {
                DOT_OFF
            };
            img.fill_rect(x + px as i32 * scale, y + row as i32 * scale, dot, dot, c);
        }
    }
}

/// Which modules a picture shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    Left,
    Right,
    Both,
}

const MARGIN: i32 = 12;
const BEZEL_PAD: i32 = 10;
const GAP: i32 = 28;

/// The whole picture: the modules in their bezels on the plate.
#[must_use]
pub fn compose(panels: &[Panel; 2], show: Show, scale: i32) -> Indexed {
    let halves: &[usize] = match show {
        Show::Left => &[0],
        Show::Right => &[1],
        Show::Both => &[0, 1],
    };
    let (gw, gh) = (
        PORTRAIT_WIDTH as i32 * scale,
        PORTRAIT_HEIGHT as i32 * scale,
    );
    let bw = gw + 2 * BEZEL_PAD;
    let n = halves.len() as i32;
    let width = 2 * MARGIN + n * bw + (n - 1) * GAP;
    let height = 2 * MARGIN + gh + 2 * BEZEL_PAD;
    let mut img = Indexed::new(width as usize, height as usize, PLATE);
    for (i, &half) in halves.iter().enumerate() {
        let bx = MARGIN + i as i32 * (bw + GAP);
        img.fill_round_rect(bx, MARGIN, bw, gh + 2 * BEZEL_PAD, 6, BEZEL);
        draw_glass(
            &mut img,
            bx + BEZEL_PAD,
            MARGIN + BEZEL_PAD,
            scale,
            &panels[half],
            0..PORTRAIT_HEIGHT,
        );
    }
    img
}

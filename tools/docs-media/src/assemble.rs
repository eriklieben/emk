//! PNG frames (the app's keyboard scenes, rendered by emulator/DocsMedia)
//! into one looping GIF. One palette for all frames: the most used colours,
//! skipping near-duplicates, and every other colour mapped to the nearest
//! of them. Flat UI colours stay exact; only anti-aliased edges round.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::gifout::Animation;
use crate::img::Indexed;

/// Colours a frame may use (index 0 is the GIF's "unchanged").
const COLORS: usize = 255;
/// Colours closer than this (squared, per channel summed) count as one.
const NEAR: u32 = 3 * 5 * 5;

/// A frame as RGB, decoded from a PNG.
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    pub px: Vec<u32>,
}

/// Read and decode the PNG at `path`.
///
/// # Errors
///
/// What went wrong reading or decoding the file.
pub fn read_png(path: &Path) -> Result<Rgb, String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let channels = info.color_type.samples();
    let px = buf[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|c| match channels {
            // Over black (the scenes have an opaque background anyway).
            4 => {
                let a = u32::from(c[3]);
                let f = |v: u8| (u32::from(v) * a / 255) & 0xFF;
                f(c[0]) << 16 | f(c[1]) << 8 | f(c[2])
            }
            3 => u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]),
            _ => u32::from(c[0]) * 0x01_01_01,
        })
        .collect();
    Ok(Rgb {
        width: info.width as usize,
        height: info.height as usize,
        px,
    })
}

fn dist(a: u32, b: u32) -> u32 {
    let d = |s: u32| {
        let (x, y) = ((a >> s) & 0xFF, (b >> s) & 0xFF);
        x.abs_diff(y).pow(2)
    };
    d(16) + d(8) + d(0)
}

/// The palette for `frames`: index 0 unused, then up to 255 colours.
#[must_use]
pub fn palette(frames: &[Rgb]) -> Vec<u32> {
    let mut counts: HashMap<u32, u64> = HashMap::new();
    for f in frames {
        for &p in &f.px {
            *counts.entry(p).or_default() += 1;
        }
    }
    let mut by_use: Vec<(u32, u64)> = counts.into_iter().collect();
    by_use.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut chosen: Vec<u32> = Vec::with_capacity(COLORS);
    let exact = by_use.len() <= COLORS;
    for &(c, _) in &by_use {
        if chosen.len() == COLORS {
            break;
        }
        if exact || chosen.iter().all(|&k| dist(k, c) > NEAR) {
            chosen.push(c);
        }
    }
    chosen
}

/// The frames of `dir` (`*.png`, in name order) as a GIF, each shown
/// `frame_ms`, the last one `hold_ms` longer.
///
/// # Errors
///
/// When `dir` can't be read, has no PNGs, or one of them doesn't decode.
pub fn assemble(dir: &Path, frame_ms: u32, hold_ms: u32) -> Result<Vec<u8>, String> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("{}: no frames", dir.display()));
    }
    let frames = paths
        .iter()
        .map(|p| read_png(p))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(encode(&frames, frame_ms, hold_ms))
}

/// A looping GIF of `frames`, `frame_ms` each, the last one `hold_ms` longer.
///
/// # Panics
///
/// When `frames` have no colours at all (no pixels).
#[must_use]
pub fn encode(frames: &[Rgb], frame_ms: u32, hold_ms: u32) -> Vec<u8> {
    let colors = palette(frames);
    let mut rgb: Vec<[u8; 3]> = vec![[0, 0, 0]];
    rgb.extend(
        colors
            .iter()
            .map(|&c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]),
    );
    let mut nearest: HashMap<u32, u8> = HashMap::new();
    let mut anim = Animation::new(&rgb);
    for f in frames {
        let mut img = Indexed::new(f.width, f.height, 1);
        for (out, &p) in img.px.iter_mut().zip(&f.px) {
            *out = *nearest.entry(p).or_insert_with(|| {
                let (i, _) = colors
                    .iter()
                    .enumerate()
                    .min_by_key(|&(_, &c)| dist(c, p))
                    .expect("a colour");
                (i + 1) as u8
            });
        }
        anim.push(img, frame_ms);
    }
    anim.hold_last(hold_ms);
    anim.encode()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: usize, height: usize, color: u32) -> Rgb {
        Rgb {
            width,
            height,
            px: vec![color; width * height],
        }
    }

    #[test]
    fn few_colours_are_kept_exactly() {
        // Arrange
        let mut b = flat(4, 4, 0x12_34_56);
        b.px[5] = 0xA3_E6_F2;
        let frames = [flat(4, 4, 0x12_34_56), b];

        // Act
        let colors = palette(&frames);

        // Assert
        assert_eq!(colors, [0x12_34_56, 0xA3_E6_F2]);
    }

    #[test]
    fn two_frames_make_a_two_frame_gif() {
        // Arrange
        let mut b = flat(8, 8, 0x10_10_10);
        b.px[9] = 0xFF_FF_FF;
        let frames = [flat(8, 8, 0x10_10_10), b];

        // Act
        let bytes = encode(&frames, 50, 0);

        // Assert
        let mut decoder = gif::DecodeOptions::new()
            .read_info(bytes.as_slice())
            .expect("a gif");
        let mut n = 0;
        while decoder.read_next_frame().expect("a frame").is_some() {
            n += 1;
        }
        assert_eq!(n, 2);
    }
}

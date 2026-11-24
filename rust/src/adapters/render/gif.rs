//! Encoding frames as the small looping GIF the device stores.
//!
//! One palette for all the frames (so colours that only appear later, a bolt or sparkles, survive), and every frame after
//! the first stored as just the rectangle that changed: the screens are static except for a mascot, so the GIFs shrink
//! 5-10x. The device takes ~36 ms per KB to store an upload, so that is most of the time an update used to take.

use std::collections::HashMap;

use color_quant::NeuQuant;
use gif::{DisposalMethod, Encoder, Frame, Repeat};

use super::canvas::Canvas;

pub const FRAME_DELAY_CS: u16 = 13; // 130 ms
const COLORS: usize = 96;
const EXACT: usize = 32; // the most frequent colours are kept exactly (the background, the text, the accents)

fn pack(r: u8, g: u8, b: u8) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

fn unpack(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

/// The palette for all frames: every colour if there are few, else the commonest ones exactly plus a trained rest.
fn build_palette(histogram: &HashMap<u32, u32>) -> Vec<[u8; 3]> {
    let mut by_count: Vec<(u32, u32)> = histogram.iter().map(|(c, n)| (*c, *n)).collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0))); // (most frequent first, ties by colour: deterministic)
    if by_count.len() <= COLORS {
        return by_count.into_iter().map(|(c, _)| unpack(c)).collect();
    }
    let mut palette: Vec<[u8; 3]> = by_count.iter().take(EXACT).map(|(c, _)| unpack(*c)).collect();
    // train on the rest, each colour repeated by how common it is (roughly: log2 of its count)
    let mut training: Vec<u8> = Vec::new();
    for (c, n) in by_count.iter().skip(EXACT) {
        let [r, g, b] = unpack(*c);
        for _ in 0..(32 - n.leading_zeros()).max(1) {
            training.extend_from_slice(&[r, g, b, 255]);
        }
    }
    let mut i = 0;
    let base = training.len();
    while training.len() < 4 * 2048 && base > 0 {
        let pixel = [training[i % base], training[(i + 1) % base], training[(i + 2) % base], 255];
        training.extend_from_slice(&pixel);
        i += 4;
    }
    let nq = NeuQuant::new(1, COLORS - EXACT, &training);
    for rgb in nq.color_map_rgb().chunks_exact(3) {
        palette.push([rgb[0], rgb[1], rgb[2]]);
    }
    palette
}

fn nearest(palette: &[[u8; 3]], c: u32) -> u8 {
    let [r, g, b] = unpack(c);
    let mut best = (u32::MAX, 0usize);
    for (i, p) in palette.iter().enumerate() {
        let d = (i32::from(p[0]) - i32::from(r)).pow(2) + (i32::from(p[1]) - i32::from(g)).pow(2) + (i32::from(p[2]) - i32::from(b)).pow(2);
        if (d as u32) < best.0 {
            best = (d as u32, i);
        }
    }
    best.1 as u8
}

/// The looping GIF of `frames` (all the same size).
pub fn encode_gif(frames: &[Canvas]) -> Result<Vec<u8>, String> {
    let first = frames.first().ok_or("no frames to encode")?;
    let (w, h) = (first.w, first.h);
    let mut histogram: HashMap<u32, u32> = HashMap::new();
    for frame in frames {
        for p in frame.px.chunks_exact(3) {
            *histogram.entry(pack(p[0], p[1], p[2])).or_insert(0) += 1;
        }
    }
    let palette = build_palette(&histogram);
    let lookup: HashMap<u32, u8> = histogram.keys().map(|c| (*c, nearest(&palette, *c))).collect();
    let mut flat: Vec<u8> = palette.iter().flatten().copied().collect();
    flat.resize(128 * 3, 0); // (a palette of 2^n entries)

    let indexed: Vec<Vec<u8>> = frames
        .iter()
        .map(|f| f.px.chunks_exact(3).map(|p| lookup[&pack(p[0], p[1], p[2])]).collect())
        .collect();

    let mut out = Vec::new();
    {
        let mut enc = Encoder::new(&mut out, w as u16, h as u16, &flat).map_err(|e| e.to_string())?;
        enc.set_repeat(Repeat::Infinite).map_err(|e| e.to_string())?;
        for (n, pixels) in indexed.iter().enumerate() {
            // the rectangle that differs from the frame before it (the first frame is whole)
            let (left, top, right, bottom) = match n {
                0 => (0, 0, w, h),
                _ => changed_box(&indexed[n - 1], pixels, w, h),
            };
            let (rw, rh) = (right - left, bottom - top);
            let mut part = Vec::with_capacity(rw * rh);
            for y in top..bottom {
                part.extend_from_slice(&pixels[y * w + left..y * w + right]);
            }
            let mut frame = Frame::from_indexed_pixels(rw as u16, rh as u16, part, None);
            frame.left = left as u16;
            frame.top = top as u16;
            frame.delay = FRAME_DELAY_CS;
            frame.dispose = DisposalMethod::Keep; // leave the frame in place: the next one is only what changed
            enc.write_frame(&frame).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

/// (left, top, right, bottom) of what differs between two frames; one pixel if nothing does.
fn changed_box(before: &[u8], after: &[u8], w: usize, h: usize) -> (usize, usize, usize, usize) {
    let (mut l, mut t, mut r, mut b) = (w, h, 0, 0);
    for y in 0..h {
        let row = y * w;
        if before[row..row + w] == after[row..row + w] {
            continue;
        }
        t = t.min(y);
        b = y + 1;
        for x in 0..w {
            if before[row + x] != after[row + x] {
                l = l.min(x);
                r = r.max(x + 1);
            }
        }
    }
    if l >= r || t >= b {
        return (0, 0, 1, 1);
    }
    (l, t, r, b)
}

/// The first frame of a GIF as RGB (what the screen shows first), with its size.
pub fn first_frame_rgb(gif_bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(gif_bytes).map_err(|e| e.to_string())?;
    let (w, h) = (u32::from(decoder.width()), u32::from(decoder.height()));
    let frame = decoder.read_next_frame().map_err(|e| e.to_string())?.ok_or("the GIF has no frames")?;
    let mut canvas = vec![0u8; (w * h * 3) as usize];
    for y in 0..u32::from(frame.height) {
        for x in 0..u32::from(frame.width) {
            let src = ((y * u32::from(frame.width) + x) * 4) as usize;
            let dst = (((y + u32::from(frame.top)) * w + x + u32::from(frame.left)) * 3) as usize;
            if dst + 3 <= canvas.len() {
                canvas[dst..dst + 3].copy_from_slice(&frame.buffer[src..src + 3]);
            }
        }
    }
    Ok((w, h, canvas))
}

/// Every frame of a GIF as a complete RGB picture (the later frames are stored as just the rectangle that changed, which is
/// laid over the one before: the "leave in place" disposal the encoder above uses), with the picture's size.
pub fn all_frames_rgb(gif_bytes: &[u8]) -> Result<(u32, u32, Vec<Vec<u8>>), String> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(gif_bytes).map_err(|e| e.to_string())?;
    let (w, h) = (u32::from(decoder.width()), u32::from(decoder.height()));
    let mut canvas = vec![0u8; (w * h * 3) as usize];
    let mut frames = Vec::new();
    while let Some(frame) = decoder.read_next_frame().map_err(|e| e.to_string())? {
        for y in 0..u32::from(frame.height) {
            for x in 0..u32::from(frame.width) {
                let src = ((y * u32::from(frame.width) + x) * 4) as usize;
                let dst = (((y + u32::from(frame.top)) * w + x + u32::from(frame.left)) * 3) as usize;
                if dst + 3 <= canvas.len() {
                    canvas[dst..dst + 3].copy_from_slice(&frame.buffer[src..src + 3]);
                }
            }
        }
        frames.push(canvas.clone());
    }
    if frames.is_empty() {
        return Err("the GIF has no frames".into());
    }
    Ok((w, h, frames))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::themes::Rgb;

    #[test]
    fn every_frame_comes_out_complete() {
        let frames = vec![
            Canvas::new(240, 240, Rgb(24, 22, 15)),
            {
                let mut c = Canvas::new(240, 240, Rgb(24, 22, 15));
                c.rect(100, 100, 110, 110, Rgb(250, 84, 7));
                c
            },
            Canvas::new(240, 240, Rgb(24, 22, 15)),
        ];
        let (w, h, decoded) = all_frames_rgb(&encode_gif(&frames).unwrap()).unwrap();
        assert_eq!((w, h, decoded.len()), (240, 240, 3));
        let at = |frame: &Vec<u8>, x: usize, y: usize| frame[(y * 240 + x) * 3..(y * 240 + x) * 3 + 3].to_vec();
        assert_eq!(at(&decoded[0], 105, 105), [24, 22, 15]);
        assert_eq!(at(&decoded[1], 105, 105), [250, 84, 7], "the changed rectangle is laid over the first frame");
        assert_eq!(at(&decoded[1], 5, 5), [24, 22, 15], "and the rest of the picture is still there");
        assert_eq!(at(&decoded[2], 105, 105), [24, 22, 15]);
        assert!(all_frames_rgb(b"not a gif").is_err());
    }

    fn frame(dot: Option<(i32, i32)>) -> Canvas {
        let mut c = Canvas::new(240, 240, Rgb(24, 22, 15));
        c.rect(10, 10, 100, 40, Rgb(250, 84, 7));
        if let Some((x, y)) = dot {
            c.rect(x, y, x + 5, y + 5, Rgb(255, 217, 102));
        }
        c
    }

    #[test]
    fn round_trips_and_stores_only_what_changed() {
        let frames = vec![frame(None), frame(Some((150, 150))), frame(Some((160, 150)))];
        let gif = encode_gif(&frames).unwrap();
        assert_eq!(&gif[..6], b"GIF89a");
        let mut decoder = gif::DecodeOptions::new().read_info(&gif[..]).unwrap();
        let mut sizes = Vec::new();
        while let Some(f) = decoder.read_next_frame().unwrap() {
            sizes.push((f.width, f.height));
        }
        assert_eq!(sizes[0], (240, 240));
        assert!(sizes[1].0 < 20 && sizes[1].1 < 20, "a later frame is just the changed rectangle: {sizes:?}");
        let (w, h, rgb) = first_frame_rgb(&gif).unwrap();
        assert_eq!((w, h), (240, 240));
        assert_eq!(&rgb[..3], &[24, 22, 15], "the background survives exactly");
        assert!(gif.len() < 4000, "flat colours compress hard: {} bytes", gif.len());
    }

    #[test]
    fn many_colours_still_encode_with_exact_dominant_ones() {
        let mut c = frame(None);
        for i in 0..200 {
            c.rect(120 + (i % 100), 100 + i / 2, 121 + (i % 100), 100 + i / 2, Rgb(i as u8, (i * 3) as u8, (255 - i) as u8));
        }
        let gif = encode_gif(&[c]).unwrap();
        let (_, _, rgb) = first_frame_rgb(&gif).unwrap();
        assert_eq!(&rgb[..3], &[24, 22, 15]);
    }

    #[test]
    fn changed_box_of_identical_frames_is_one_pixel() {
        let a = vec![0u8; 16];
        assert_eq!(changed_box(&a, &a, 4, 4), (0, 0, 1, 1));
    }
}

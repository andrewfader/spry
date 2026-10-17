//! PNG and GIF, in and out.
//!
//! Exports are always nearest-neighbour scaled. A pixel art export that
//! blurs is a bug. GIFs honour every frame's duration and keep full
//! transparency, clearing each frame before the next so animated
//! silhouettes never smear.

use crate::editor::playback::play_order;
use crate::editor::{Pixels, Sprite, Tag};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::Cursor;

pub fn encode_png(pixels: &Pixels) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_raw(pixels.width(), pixels.height(), pixels.to_rgba_bytes())
        .context("image buffer size mismatch")?
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .context("encoding PNG")?;
    Ok(bytes)
}

/// Decodes any PNG or GIF (first frame) into pixels.
pub fn decode_image(bytes: &[u8]) -> Result<Pixels> {
    let decoded = image::load_from_memory(bytes).context("that file isn't a PNG or GIF image")?.to_rgba8();
    Pixels::from_rgba_bytes(decoded.width(), decoded.height(), decoded.as_raw()).context("decoded image size mismatch")
}

/// The frames of an animation as `(image, delay_ms)`, played in tag order.
pub fn animation_frames(sprite: &Sprite, tag: Option<&Tag>, scale: u32) -> Vec<(Pixels, u32)> {
    play_order(sprite, tag)
        .into_iter()
        .map(|frame| (sprite.composite(frame).scaled_up(scale), sprite.frames[frame].duration_ms))
        .collect()
}

pub fn encode_gif(frames: &[(Pixels, u32)]) -> Result<Vec<u8>> {
    let Some((first, _)) = frames.first() else { bail!("an animation needs at least one frame") };
    let (width, height) = (first.width(), first.height());
    if width > u16::MAX as u32 || height > u16::MAX as u32 {
        bail!("GIFs can be at most 65535 pixels on a side");
    }
    let palette = GifPalette::for_frames(frames);
    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, width as u16, height as u16, &palette.rgb_table())?;
        encoder.set_repeat(gif::Repeat::Infinite)?;
        for (image, delay_ms) in frames {
            let mut frame = gif::Frame {
                width: width as u16,
                height: height as u16,
                // GIF delays are in hundredths of a second.
                delay: (delay_ms / 10).max(1) as u16,
                dispose: gif::DisposalMethod::Background,
                transparent: Some(GifPalette::TRANSPARENT_INDEX),
                ..gif::Frame::default()
            };
            frame.buffer = std::borrow::Cow::Owned(palette.indices_of(image));
            encoder.write_frame(&frame)?;
        }
    }
    Ok(bytes)
}

/// One global colour table shared by every frame, with index 0 kept for
/// transparency. Pixel art rarely has more than 255 colours. When it does,
/// the rarest colours map to their nearest kept neighbour.
struct GifPalette {
    colours: Vec<[u8; 3]>,
    index_of: HashMap<[u8; 3], u8>,
}

impl GifPalette {
    const TRANSPARENT_INDEX: u8 = 0;

    fn for_frames(frames: &[(Pixels, u32)]) -> GifPalette {
        let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
        for (image, _) in frames {
            for colour in image.colours().iter().filter(|c| c.a() >= 128) {
                *counts.entry([colour.r(), colour.g(), colour.b()]).or_default() += 1;
            }
        }
        let mut by_popularity: Vec<([u8; 3], usize)> = counts.into_iter().collect();
        by_popularity.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let kept: Vec<[u8; 3]> = by_popularity.iter().take(255).map(|(c, _)| *c).collect();
        let mut index_of = HashMap::new();
        for (i, colour) in kept.iter().enumerate() {
            index_of.insert(*colour, i as u8 + 1);
        }
        for (colour, _) in by_popularity.iter().skip(255) {
            let nearest = kept
                .iter()
                .enumerate()
                .min_by_key(|(_, k)| k.iter().zip(colour).map(|(a, b)| (*a as i32 - *b as i32).pow(2)).sum::<i32>());
            index_of.insert(*colour, nearest.map_or(1, |(i, _)| i as u8 + 1));
        }
        let mut colours = vec![[0, 0, 0]];
        colours.extend(kept);
        GifPalette { colours, index_of }
    }

    fn rgb_table(&self) -> Vec<u8> {
        // GIF tables must hold a power of two entries.
        let size = self.colours.len().next_power_of_two().max(2);
        let mut table: Vec<u8> = self.colours.iter().flatten().copied().collect();
        table.resize(size * 3, 0);
        table
    }

    fn indices_of(&self, image: &Pixels) -> Vec<u8> {
        image
            .colours()
            .iter()
            .map(|c| if c.a() < 128 { Self::TRANSPARENT_INDEX } else { self.index_of[&[c.r(), c.g(), c.b()]] })
            .collect()
    }
}

/// Decodes every frame of a GIF as `(image, delay_ms)`. Used by tests and
/// by "import GIF as animation".
pub fn decode_gif(bytes: &[u8]) -> Result<Vec<(Pixels, u32)>> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(Cursor::new(bytes)).context("that file isn't a GIF")?;
    let (width, height) = (decoder.width() as u32, decoder.height() as u32);
    let mut canvas = Pixels::new(width, height);
    let mut frames = Vec::new();
    while let Some(frame) = decoder.read_next_frame()? {
        let patch = Pixels::from_rgba_bytes(frame.width as u32, frame.height as u32, &frame.buffer)
            .context("GIF frame size mismatch")?;
        let mut shown = canvas.clone();
        shown.stamp(&patch, frame.left as i32, frame.top as i32, 255);
        frames.push((shown.clone(), frame.delay as u32 * 10));
        canvas = match frame.dispose {
            gif::DisposalMethod::Background => Pixels::new(width, height),
            gif::DisposalMethod::Previous => canvas,
            _ => shown,
        };
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Rgba;

    #[test]
    fn png_round_trips_including_transparency() {
        let mut image = Pixels::new(3, 2);
        image.set(1, 1, Rgba::rgba(10, 20, 30, 128));
        assert_eq!(decode_image(&encode_png(&image).unwrap()).unwrap(), image);
    }

    #[test]
    fn gif_frames_keep_their_pixels_delays_and_holes() {
        let mut a = Pixels::new(4, 4);
        a.set(0, 0, Rgba::rgb(255, 0, 77));
        let mut b = Pixels::new(4, 4);
        b.set(3, 3, Rgba::rgb(0, 228, 54));
        let decoded = decode_gif(&encode_gif(&[(a.clone(), 100), (b.clone(), 250)]).unwrap()).unwrap();
        assert_eq!(decoded, vec![(a, 100), (b, 250)]);
    }

    #[test]
    fn gifs_with_too_many_colours_still_encode() {
        let mut image = Pixels::new(20, 20);
        for (i, (x, y)) in image.bounds().points().collect::<Vec<_>>().into_iter().enumerate() {
            image.set(x, y, Rgba::rgb(i as u8, (i / 2) as u8, 3));
        }
        let decoded = decode_gif(&encode_gif(&[(image, 100)]).unwrap()).unwrap();
        assert_eq!(decoded.len(), 1);
    }

    #[test]
    fn non_images_are_rejected_with_a_reason() {
        assert!(decode_image(b"hello").unwrap_err().to_string().contains("PNG"));
    }
}

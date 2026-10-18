//! PNG, APNG, GIF and WebP, in and out.
//!
//! APNG is the better animated format for pixel art: full 8-bit alpha and
//! delays to the millisecond, where GIF has one-bit transparency and 10 ms
//! steps. WebP is written lossless, as one picture or as an animation with
//! the same exact delays.
//!
//! Exports are always nearest-neighbour scaled. A pixel art export that
//! blurs is a bug. GIFs honour every frame's duration and keep full
//! transparency, clearing each frame before the next so animated
//! silhouettes never smear.

use crate::editor::playback::play_order;
use crate::editor::{Pixels, Sprite, Tag};
use anyhow::{bail, ensure, Context, Result};
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
        let delays = centisecond_delays(frames.iter().map(|(_, delay_ms)| *delay_ms));
        for ((image, _), delay) in frames.iter().zip(delays) {
            let mut frame = gif::Frame {
                width: width as u16,
                height: height as u16,
                delay,
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

/// GIF delays count hundredths of a second. Rounding each frame on its own
/// drifts (three 15 ms frames would play as 60 ms or 30 ms), so each
/// frame ends at its true time rounded, and the loop as a whole keeps its
/// real length.
fn centisecond_delays(delays_ms: impl Iterator<Item = u32>) -> Vec<u16> {
    let mut elapsed_ms: u64 = 0;
    let mut shown_cs: u64 = 0;
    delays_ms
        .map(|delay| {
            elapsed_ms += u64::from(delay);
            let ends_cs = ((elapsed_ms + 5) / 10).max(shown_cs + 1);
            let delay_cs = (ends_cs - shown_cs).min(u64::from(u16::MAX));
            shown_cs += delay_cs;
            delay_cs as u16
        })
        .collect()
}

/// An animated PNG that loops forever, each frame shown for exactly its
/// delay in milliseconds.
pub fn encode_apng(frames: &[(Pixels, u32)]) -> Result<Vec<u8>> {
    let Some((first, _)) = frames.first() else { bail!("an animation needs at least one frame") };
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, first.width(), first.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_animated(frames.len() as u32, 0).context("setting up the APNG")?;
        let mut writer = encoder.write_header().context("writing the APNG header")?;
        for (image, delay_ms) in frames {
            writer.set_frame_delay((*delay_ms).clamp(1, u32::from(u16::MAX)) as u16, 1000)?;
            writer.write_image_data(&image.to_rgba_bytes()).context("writing an APNG frame")?;
        }
        writer.finish().context("finishing the APNG")?;
    }
    Ok(bytes)
}

/// Every frame of an APNG as `(image, delay_ms)`. Spry writes whole
/// frames, so this only needs to read whole frames back.
pub fn decode_apng(bytes: &[u8]) -> Result<Vec<(Pixels, u32)>> {
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().context("that file isn't a PNG")?;
    let count = reader.info().animation_control().map_or(1, |control| control.num_frames);
    let mut frames = Vec::new();
    let mut buffer = vec![0; reader.output_buffer_size().context("the PNG is too large")?];
    for _ in 0..count {
        let info = reader.next_frame(&mut buffer).context("reading an APNG frame")?;
        let delay = reader.info().frame_control().map_or(0, |control| {
            let denominator = if control.delay_den == 0 { 100 } else { u32::from(control.delay_den) };
            u32::from(control.delay_num) * 1000 / denominator
        });
        let image = Pixels::from_rgba_bytes(info.width, info.height, &buffer[..info.buffer_size()])
            .context("APNG frame size mismatch")?;
        frames.push((image, delay));
    }
    Ok(frames)
}

/// A lossless WebP of one picture.
pub fn encode_webp(pixels: &Pixels) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_raw(pixels.width(), pixels.height(), pixels.to_rgba_bytes())
        .context("image buffer size mismatch")?
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::WebP)
        .context("encoding WebP")?;
    Ok(bytes)
}

/// An animated, lossless WebP that loops forever.
///
/// The encoder Spry uses only writes single pictures, so each frame is
/// encoded on its own and its `VP8L` bitstream is wrapped in the animation
/// container from the WebP spec: a `VP8X` header with the animation flag, an
/// `ANIM` chunk (loop forever), and one `ANMF` chunk per frame. Every frame
/// covers the whole canvas and replaces the last one (no blending, no
/// disposal), so what you drew is exactly what plays.
pub fn encode_animated_webp(frames: &[(Pixels, u32)]) -> Result<Vec<u8>> {
    let Some((first, _)) = frames.first() else { bail!("an animation needs at least one frame") };
    let (width, height) = (first.width(), first.height());
    ensure!(width <= 1 << 24 && height <= 1 << 24, "WebP canvases are at most 16777216 pixels on a side");
    let mut body = Vec::new();
    body.extend_from_slice(b"WEBP");
    let mut header = vec![VP8X_ANIMATION | VP8X_ALPHA, 0, 0, 0];
    header.extend_from_slice(&u24(width - 1));
    header.extend_from_slice(&u24(height - 1));
    push_chunk(&mut body, b"VP8X", &header);
    // Background colour (unused, since every frame covers the canvas), then
    // a loop count of 0, meaning forever.
    push_chunk(&mut body, b"ANIM", &[0, 0, 0, 0, 0, 0]);
    for (image, delay_ms) in frames {
        let mut frame = Vec::new();
        frame.extend_from_slice(&u24(0)); // x / 2
        frame.extend_from_slice(&u24(0)); // y / 2
        frame.extend_from_slice(&u24(image.width() - 1));
        frame.extend_from_slice(&u24(image.height() - 1));
        frame.extend_from_slice(&u24((*delay_ms).clamp(1, (1 << 24) - 1)));
        frame.push(ANMF_NO_BLENDING);
        push_chunk(&mut frame, b"VP8L", &lossless_bitstream(image)?);
        push_chunk(&mut body, b"ANMF", &frame);
    }
    let mut bytes = Vec::with_capacity(body.len() + 8);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&body);
    Ok(bytes)
}

const VP8X_ANIMATION: u8 = 1 << 1;
const VP8X_ALPHA: u8 = 1 << 4;
const ANMF_NO_BLENDING: u8 = 1 << 1;

fn u24(value: u32) -> [u8; 3] {
    let [a, b, c, _] = value.to_le_bytes();
    [a, b, c]
}

/// A RIFF chunk: four-letter name, little-endian length, data, and a pad
/// byte to keep the next chunk on an even offset.
fn push_chunk(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(name);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

/// The raw `VP8L` data of one frame, lifted out of the simple single-image
/// file the encoder writes (`RIFF`, size, `WEBP`, `VP8L`, size, data).
fn lossless_bitstream(image: &Pixels) -> Result<Vec<u8>> {
    let mut file = Vec::new();
    image_webp::WebPEncoder::new(&mut file)
        .encode(&image.to_rgba_bytes(), image.width(), image.height(), image_webp::ColorType::Rgba8)
        .context("encoding a WebP frame")?;
    ensure!(file.len() >= 20 && &file[12..16] == b"VP8L", "the WebP encoder wrote an unexpected layout");
    let length = u32::from_le_bytes([file[16], file[17], file[18], file[19]]) as usize;
    file.get(20..20 + length).map(<[u8]>::to_vec).context("the WebP frame is shorter than it says")
}

/// Every frame of an animated WebP as `(image, delay_ms)`.
pub fn decode_animated_webp(bytes: &[u8]) -> Result<Vec<(Pixels, u32)>> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).context("that file isn't a WebP")?;
    ensure!(decoder.is_animated(), "that WebP isn't animated");
    let (width, height) = decoder.dimensions();
    let mut buffer = vec![0; decoder.output_buffer_size().context("the WebP is too large")?];
    let mut frames = Vec::new();
    for _ in 0..decoder.num_frames() {
        let delay = decoder.read_frame(&mut buffer).context("reading a WebP frame")?;
        let image = if decoder.has_alpha() {
            Pixels::from_rgba_bytes(width, height, &buffer)
        } else {
            let rgba: Vec<u8> = buffer.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect();
            Pixels::from_rgba_bytes(width, height, &rgba)
        };
        frames.push((image.context("WebP frame size mismatch")?, delay));
    }
    Ok(frames)
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
    fn gif_delays_round_without_drifting() {
        assert_eq!(centisecond_delays([15, 15, 15, 15].into_iter()), vec![2, 1, 2, 1]);
        assert_eq!(centisecond_delays([105, 100].into_iter()), vec![11, 10]);
        assert_eq!(centisecond_delays([1, 1].into_iter()), vec![1, 1], "every frame shows for at least 10 ms");
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
    fn apng_keeps_every_frame_alpha_and_millisecond_delay() {
        let mut a = Pixels::new(3, 3);
        a.set(0, 0, Rgba::rgba(255, 0, 77, 100));
        let mut b = Pixels::new(3, 3);
        b.set(2, 2, Rgba::rgb(0, 228, 54));
        let frames = vec![(a, 15), (b, 105)];
        assert_eq!(decode_apng(&encode_apng(&frames).unwrap()).unwrap(), frames);
    }

    #[test]
    fn animated_webp_keeps_every_frame_alpha_and_millisecond_delay() {
        let mut a = Pixels::new(3, 2);
        a.set(0, 0, Rgba::rgba(255, 0, 77, 100));
        let mut b = Pixels::new(3, 2);
        b.set(2, 1, Rgba::rgb(0, 228, 54));
        let frames = vec![(a, 15), (b, 1234)];
        assert_eq!(decode_animated_webp(&encode_animated_webp(&frames).unwrap()).unwrap(), frames);
    }

    #[test]
    fn odd_sized_frames_keep_the_chunks_aligned() {
        let frames: Vec<(Pixels, u32)> =
            (0..3u8).map(|i| (Pixels::filled(5, 3, Rgba::rgb(i * 80, 7, 9)), 100)).collect();
        assert_eq!(decode_animated_webp(&encode_animated_webp(&frames).unwrap()).unwrap(), frames);
    }

    #[test]
    fn webp_is_lossless() {
        let mut image = Pixels::new(5, 4);
        image.set(1, 2, Rgba::rgba(10, 200, 30, 128));
        image.set(4, 0, Rgba::rgb(255, 0, 77));
        assert_eq!(decode_image(&encode_webp(&image).unwrap()).unwrap(), image);
    }

    #[test]
    fn non_images_are_rejected_with_a_reason() {
        assert!(decode_image(b"hello").unwrap_err().to_string().contains("PNG"));
    }
}

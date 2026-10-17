//! Turning arbitrary pictures into honest pixel art.
//!
//! Diffusion models, even "pixel art" ones, paint soft, oversized images
//! with thousands of colours. This pipeline turns them into real sprites:
//!
//! 1. **Downscale by dominant colour.** Each target pixel takes the most
//!    common colour in its block, never the average, so edges stay crisp
//!    and no new in-between colours appear.
//! 2. **Remove the background** by flooding from the corners, since
//!    generators usually paint subjects on a flat backdrop.
//! 3. **Quantise to the palette**, optionally with ordered dither so
//!    gradients survive as patterns instead of banding.
//!
//! The same quantiser backs the "snap layer to palette" command.

use super::color::Rgba;
use super::dither::bayer_threshold;
use super::palette::Palette;
use super::pixels::Pixels;
use super::raster::contiguous_region;
use std::collections::HashMap;

/// Shrinks `image` to `width × height`, giving each output pixel the colour
/// that covers most of its source block. Near-identical colours are pooled
/// (to 5 bits per channel) so JPEG-ish noise doesn't split a vote.
pub fn downscale_dominant(image: &Pixels, width: u32, height: u32) -> Pixels {
    let mut out = Pixels::new(width, height);
    let scale_x = image.width() as f32 / width as f32;
    let scale_y = image.height() as f32 / height as f32;
    for (x, y) in out.bounds().points() {
        let left = (x as f32 * scale_x) as i32;
        let top = (y as f32 * scale_y) as i32;
        let right = (((x + 1) as f32 * scale_x).ceil() as i32).max(left + 1);
        let bottom = (((y + 1) as f32 * scale_y).ceil() as i32).max(top + 1);
        let mut votes: HashMap<[u8; 4], (u32, Rgba)> = HashMap::new();
        for sy in top..bottom {
            for sx in left..right {
                let colour = image.get(sx, sy);
                let key = if colour.a() < 128 {
                    [0, 0, 0, 0]
                } else {
                    [colour.r() >> 3, colour.g() >> 3, colour.b() >> 3, 255]
                };
                let entry = votes.entry(key).or_insert((0, colour));
                entry.0 += 1;
            }
        }
        let winner = votes.into_values().max_by_key(|(count, _)| *count).map(|(_, c)| c);
        let colour = winner.unwrap_or(Rgba::TRANSPARENT);
        out.set(x, y, if colour.a() < 128 { Rgba::TRANSPARENT } else { colour.with_alpha(255) });
    }
    out
}

/// Clears the flat backdrop: every region touching a corner whose colour is
/// within `tolerance` (Oklab distance) of that corner becomes transparent.
pub fn remove_background(image: &Pixels, tolerance: f32) -> Pixels {
    let mut out = image.clone();
    let (w, h) = (image.width() as i32, image.height() as i32);
    for corner in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        let backdrop = out.get(corner.0, corner.1);
        if backdrop.is_transparent() {
            continue;
        }
        let similar = snap_similar(&out, backdrop, tolerance);
        for (x, y) in contiguous_region(&similar, corner) {
            out.set(x, y, Rgba::TRANSPARENT);
        }
    }
    out
}

/// A copy where every colour close to `target` is replaced by `target`
/// exactly, so an exact-match flood fill can follow a slightly noisy
/// backdrop.
fn snap_similar(image: &Pixels, target: Rgba, tolerance: f32) -> Pixels {
    let mut snapped = image.clone();
    for pixel in snapped.colours_mut() {
        if !pixel.is_transparent() && pixel.perceptual_distance(target) <= tolerance {
            *pixel = target;
        }
    }
    snapped
}

/// Maps every visible pixel to a palette colour. With `dither`, each pixel
/// picks between its two nearest palette colours using a 4×4 Bayer
/// threshold, weighted by how close it sits to each.
pub fn quantise(image: &Pixels, palette: &Palette, dither: bool) -> Pixels {
    let mut out = image.clone();
    let mut nearest_cache: HashMap<Rgba, (Rgba, Rgba, f32)> = HashMap::new();
    for (x, y) in image.bounds().points() {
        let colour = image.get(x, y);
        if colour.is_transparent() {
            continue;
        }
        let (first, second, blend) = *nearest_cache.entry(colour).or_insert_with(|| two_nearest(palette, colour));
        let chosen = if dither && blend > 0.0 {
            let threshold = (bayer_threshold(4, x, y) as f32 + 0.5) / 16.0;
            if blend > threshold {
                second
            } else {
                first
            }
        } else {
            first
        };
        out.set(x, y, chosen.with_alpha(255));
    }
    out
}

/// The nearest and second-nearest palette colours, and how far toward the
/// second one the colour sits (0 = exactly the first, 0.5 = halfway). Only
/// a colour truly between the two gets dithered: past a 0.35 share it
/// becomes a pattern, below that it stays solid.
fn two_nearest(palette: &Palette, colour: Rgba) -> (Rgba, Rgba, f32) {
    let opaque = colour.with_alpha(255);
    let mut ranked: Vec<(f32, Rgba)> = palette.colours.iter().map(|&c| (c.perceptual_distance(opaque), c)).collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
    match ranked[..] {
        [] => (colour, colour, 0.0),
        [(_, only)] => (only, only, 0.0),
        [(d1, c1), (d2, c2), ..] => {
            let share = if d1 + d2 == 0.0 { 0.0 } else { d1 / (d1 + d2) };
            let blend = if share < 0.35 { 0.0 } else { share };
            (c1, c2, blend)
        }
    }
}

/// The whole pipeline, from a generated picture to a sprite-ready layer.
pub fn pixelate(
    image: &Pixels,
    width: u32,
    height: u32,
    palette: &Palette,
    clear_background: bool,
    dither: bool,
) -> Pixels {
    let small = downscale_dominant(image, width, height);
    let isolated = if clear_background { remove_background(&small, 0.08) } else { small };
    quantise(&isolated, palette, dither)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::pixels::Rect;

    fn two_tone(size: u32, inner: Rgba, outer: Rgba) -> Pixels {
        let mut image = Pixels::filled(size, size, outer);
        let quarter = (size / 4) as i32;
        for (x, y) in Rect::new(quarter, quarter, size / 2, size / 2).points() {
            image.set(x, y, inner);
        }
        image
    }

    #[test]
    fn downscaling_keeps_the_dominant_colour_of_each_block() {
        let big = two_tone(64, Rgba::BLACK, Rgba::WHITE);
        let small = downscale_dominant(&big, 4, 4);
        assert_eq!(small.get(0, 0), Rgba::WHITE);
        assert_eq!(small.get(1, 1), Rgba::BLACK);
        assert_eq!(small.get(2, 2), Rgba::BLACK);
        assert_eq!(small.get(3, 3), Rgba::WHITE);
    }

    #[test]
    fn downscaling_never_invents_in_between_colours() {
        let big = two_tone(30, Rgba::BLACK, Rgba::WHITE);
        let small = downscale_dominant(&big, 7, 7);
        assert!(small.colours().iter().all(|&c| c == Rgba::BLACK || c == Rgba::WHITE));
    }

    #[test]
    fn background_removal_clears_the_backdrop_but_not_the_subject() {
        let image = two_tone(8, Rgba::BLACK, Rgba::WHITE);
        let cut_out = remove_background(&image, 0.05);
        assert_eq!(cut_out.get(0, 0), Rgba::TRANSPARENT);
        assert_eq!(cut_out.get(4, 4), Rgba::BLACK);
    }

    #[test]
    fn quantising_without_dither_uses_only_palette_colours() {
        let palette = Palette::pico8();
        let mut image = Pixels::new(16, 1);
        for x in 0..16 {
            image.set(x, 0, Rgba::rgb((x * 16) as u8, 100, 200));
        }
        let snapped = quantise(&image, &palette, false);
        assert!(snapped.colours().iter().all(|c| palette.colours.contains(c)));
    }

    #[test]
    fn dithering_a_midtone_mixes_its_two_neighbours() {
        let palette = Palette { name: "bw".into(), colours: vec![Rgba::BLACK, Rgba::WHITE] };
        let grey = Pixels::filled(4, 4, Rgba::rgb(119, 119, 119));
        let dithered = quantise(&grey, &palette, true);
        let whites = dithered.colours().iter().filter(|&&c| c == Rgba::WHITE).count();
        assert!((4..=12).contains(&whites), "expected a mix, got {whites} whites");
    }

    #[test]
    fn colours_already_in_the_palette_are_never_dithered() {
        let palette = Palette::pico8();
        let flat = Pixels::filled(4, 4, palette.colours[8]);
        assert_eq!(quantise(&flat, &palette, true), flat);
    }

    #[test]
    fn the_pipeline_lands_on_the_grid_and_the_palette() {
        let palette = Palette::pico8();
        let generated = two_tone(256, Rgba::rgb(250, 10, 70), Rgba::rgb(30, 40, 90));
        let sprite = pixelate(&generated, 32, 32, &palette, true, false);
        assert_eq!((sprite.width(), sprite.height()), (32, 32));
        assert_eq!(sprite.get(0, 0), Rgba::TRANSPARENT);
        assert!(sprite.colours().iter().all(|c| c.is_transparent() || palette.colours.contains(c)));
    }
}

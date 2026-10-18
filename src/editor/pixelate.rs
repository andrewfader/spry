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
//! With [`Fit::Subject`] the backdrop is cut away first, at full size, and
//! only the subject is shrunk, keeping its proportions.
//!
//! The same quantiser backs the "snap layer to palette" command.

use super::color::Rgba;
use super::dither::bayer_threshold;
use super::palette::Palette;
use super::pixels::Pixels;
use super::raster::contiguous_region;
use serde::{Deserialize, Serialize};
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
        // Ties go to the lowest colour key, not to HashMap order, so the
        // same picture always gives the same sprite.
        let winner =
            votes.into_iter().max_by(|a, b| a.1 .0.cmp(&b.1 .0).then_with(|| b.0.cmp(&a.0))).map(|(_, (_, c))| c);
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

/// How close to a corner's colour a pixel must be to count as backdrop.
const BACKDROP_TOLERANCE: f32 = 0.08;

/// How a picture is laid onto the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// The whole picture, stretched to the canvas.
    #[default]
    Canvas,
    /// Only the subject, as large as fits with its proportions kept,
    /// centred and standing on the bottom edge (where a sprite's feet go).
    /// Always clears the backdrop, since that's how the subject is found.
    Subject,
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
    let isolated = if clear_background { remove_background(&small, BACKDROP_TOLERANCE) } else { small };
    quantise(&isolated, palette, dither)
}

/// The pipeline for [`Fit::Subject`]. The backdrop is cleared before
/// shrinking because at full size its edge is sharp; shrunk first, the
/// subject's outline would be voted into the backdrop.
pub fn pixelate_subject(image: &Pixels, width: u32, height: u32, palette: &Palette, dither: bool) -> Pixels {
    let cut_out = remove_background(image, BACKDROP_TOLERANCE);
    let mut canvas = Pixels::new(width, height);
    let Some(bounds) = cut_out.opaque_bounds() else { return canvas };
    let (fit_width, fit_height) = fit_inside((bounds.width, bounds.height), (width, height));
    let subject = downscale_dominant(&cut_out.crop(bounds), fit_width, fit_height);
    canvas.paste(&subject, ((width - fit_width) / 2) as i32, (height - fit_height) as i32);
    quantise(&canvas, palette, dither)
}

/// The largest size with `subject`'s proportions that fits in `frame`,
/// never smaller than one pixel a side.
fn fit_inside(subject: (u32, u32), frame: (u32, u32)) -> (u32, u32) {
    let scale = (frame.0 as f32 / subject.0 as f32).min(frame.1 as f32 / subject.1 as f32);
    let side = |length: u32, limit: u32| ((length as f32 * scale).round() as u32).clamp(1, limit);
    (side(subject.0, frame.0), side(subject.1, frame.1))
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
    fn a_tied_vote_always_goes_the_same_way() {
        let mut checker = Pixels::new(2, 2);
        checker.set(0, 0, Rgba::BLACK);
        checker.set(1, 1, Rgba::BLACK);
        checker.set(1, 0, Rgba::WHITE);
        checker.set(0, 1, Rgba::WHITE);
        let first = downscale_dominant(&checker, 1, 1);
        assert!((0..50).all(|_| downscale_dominant(&checker, 1, 1) == first));
        assert_eq!(first.get(0, 0), Rgba::BLACK);
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

    #[test]
    fn a_tall_subject_fits_the_height_and_is_centred() {
        assert_eq!(fit_inside((50, 200), (32, 32)), (8, 32));
    }

    #[test]
    fn a_hairline_subject_keeps_at_least_one_pixel() {
        assert_eq!(fit_inside((1000, 1), (16, 16)), (16, 1));
    }

    #[test]
    fn a_picture_that_is_all_backdrop_lands_as_an_empty_layer() {
        let blank = Pixels::filled(64, 64, Rgba::WHITE);
        assert!(pixelate_subject(&blank, 8, 8, &Palette::pico8(), false).is_blank());
    }
}

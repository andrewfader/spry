//! Layer blend modes: how a layer's colours mix with what's below it.
//!
//! These are the nineteen modes Aseprite offers, so files move between the
//! two without changing how they look. The maths is the W3C Compositing
//! and Blending spec's: a blend function `B(backdrop, source)` mixes the
//! colours, and the result is then laid over the backdrop by ordinary
//! source-over alpha compositing. Normal mode skips all of that and uses
//! the exact integer path in `Rgba::over`, so plain pixel art stays
//! bit-exact.

use super::color::Rgba;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    Addition,
    Subtract,
    Divide,
}

impl BlendMode {
    /// In Aseprite's order, which is also the numbering its files use.
    pub const ALL: [BlendMode; 19] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::Addition,
        BlendMode::Subtract,
        BlendMode::Divide,
    ];

    /// The mode an Aseprite file means by its number.
    pub fn from_aseprite(number: u16) -> Option<BlendMode> {
        BlendMode::ALL.get(number as usize).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
            BlendMode::ColorDodge => "Color dodge",
            BlendMode::ColorBurn => "Color burn",
            BlendMode::HardLight => "Hard light",
            BlendMode::SoftLight => "Soft light",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Color => "Color",
            BlendMode::Luminosity => "Luminosity",
            BlendMode::Addition => "Addition",
            BlendMode::Subtract => "Subtract",
            BlendMode::Divide => "Divide",
        }
    }
}

/// Lays `source` over `below` at `opacity` (0–255) using `mode`.
pub fn blend_over(source: Rgba, below: Rgba, opacity: u8, mode: BlendMode) -> Rgba {
    if mode == BlendMode::Normal || below.is_transparent() {
        // Over nothing there's nothing to mix with, and every mode reduces
        // to plain painting.
        return source.over(below, opacity);
    }
    let source_alpha = f32::from(source.a()) / 255.0 * f32::from(opacity) / 255.0;
    if source_alpha <= 0.0 {
        return below;
    }
    let backdrop_alpha = f32::from(below.a()) / 255.0;
    let (backdrop, colour) = (unit_rgb(below), unit_rgb(source));
    let mixed = mix(mode, backdrop, colour);
    let out_alpha = source_alpha + backdrop_alpha * (1.0 - source_alpha);
    let channel = |i: usize| {
        // Where the backdrop is translucent, the source shows through
        // unmixed in proportion (the spec's Cs' = (1 - αb)·Cs + αb·B).
        let blended = (1.0 - backdrop_alpha) * colour[i] + backdrop_alpha * mixed[i];
        let value = source_alpha * blended + backdrop_alpha * backdrop[i] * (1.0 - source_alpha);
        to_byte(value / out_alpha)
    };
    Rgba::rgba(channel(0), channel(1), channel(2), to_byte(out_alpha))
}

type Rgb = [f32; 3];

fn unit_rgb(colour: Rgba) -> Rgb {
    [colour.r(), colour.g(), colour.b()].map(|c| f32::from(c) / 255.0)
}

fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn mix(mode: BlendMode, backdrop: Rgb, source: Rgb) -> Rgb {
    let each = |f: fn(f32, f32) -> f32| [0, 1, 2].map(|i| f(backdrop[i], source[i]));
    match mode {
        BlendMode::Normal => source,
        BlendMode::Multiply => each(|b, s| b * s),
        BlendMode::Screen => each(screen),
        BlendMode::Overlay => each(|b, s| hard_light(s, b)),
        BlendMode::Darken => each(f32::min),
        BlendMode::Lighten => each(f32::max),
        BlendMode::ColorDodge => each(colour_dodge),
        BlendMode::ColorBurn => each(colour_burn),
        BlendMode::HardLight => each(hard_light),
        BlendMode::SoftLight => each(soft_light),
        BlendMode::Difference => each(|b, s| (b - s).abs()),
        BlendMode::Exclusion => each(|b, s| b + s - 2.0 * b * s),
        BlendMode::Hue => set_luminosity(set_saturation(source, saturation(backdrop)), luminosity(backdrop)),
        BlendMode::Saturation => set_luminosity(set_saturation(backdrop, saturation(source)), luminosity(backdrop)),
        BlendMode::Color => set_luminosity(source, luminosity(backdrop)),
        BlendMode::Luminosity => set_luminosity(backdrop, luminosity(source)),
        BlendMode::Addition => each(|b, s| (b + s).min(1.0)),
        BlendMode::Subtract => each(|b, s| (b - s).max(0.0)),
        BlendMode::Divide => each(|b, s| if s <= 0.0 { 1.0 } else { (b / s).min(1.0) }),
    }
}

fn screen(b: f32, s: f32) -> f32 {
    b + s - b * s
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b * 2.0 * s
    } else {
        screen(b, 2.0 * s - 1.0)
    }
}

fn colour_dodge(b: f32, s: f32) -> f32 {
    match (b, s) {
        (b, _) if b <= 0.0 => 0.0,
        (_, s) if s >= 1.0 => 1.0,
        (b, s) => (b / (1.0 - s)).min(1.0),
    }
}

fn colour_burn(b: f32, s: f32) -> f32 {
    match (b, s) {
        (b, _) if b >= 1.0 => 1.0,
        (_, s) if s <= 0.0 => 0.0,
        (b, s) => 1.0 - ((1.0 - b) / s).min(1.0),
    }
}

fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b - (1.0 - 2.0 * s) * b * (1.0 - b)
    } else {
        let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
        b + (2.0 * s - 1.0) * (d - b)
    }
}

// The non-separable modes work on luminosity and saturation as the spec
// defines them, rather than through HSL, so hue survives exactly.

fn luminosity([r, g, b]: Rgb) -> f32 {
    0.3 * r + 0.59 * g + 0.11 * b
}

fn clip_colour(colour: Rgb) -> Rgb {
    let lum = luminosity(colour);
    let (low, high) =
        (colour.iter().copied().fold(f32::MAX, f32::min), colour.iter().copied().fold(f32::MIN, f32::max));
    colour.map(|c| {
        let c = if low < 0.0 { lum + (c - lum) * lum / (lum - low) } else { c };
        if high > 1.0 {
            lum + (c - lum) * (1.0 - lum) / (high - lum)
        } else {
            c
        }
    })
}

fn set_luminosity(colour: Rgb, lum: f32) -> Rgb {
    let shift = lum - luminosity(colour);
    clip_colour(colour.map(|c| c + shift))
}

fn saturation(colour: Rgb) -> f32 {
    colour.iter().copied().fold(f32::MIN, f32::max) - colour.iter().copied().fold(f32::MAX, f32::min)
}

fn set_saturation(colour: Rgb, sat: f32) -> Rgb {
    let (high, low) =
        (colour.iter().copied().fold(f32::MIN, f32::max), colour.iter().copied().fold(f32::MAX, f32::min));
    if high <= low {
        return [0.0; 3];
    }
    colour.map(|c| (c - low) * sat / (high - low))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREY: Rgba = Rgba::rgb(128, 128, 128);

    fn opaque(r: u8, g: u8, b: u8) -> Rgba {
        Rgba::rgb(r, g, b)
    }

    #[test]
    fn normal_mode_is_exactly_ordinary_painting() {
        let paint = Rgba::rgba(200, 10, 40, 100);
        assert_eq!(blend_over(paint, GREY, 200, BlendMode::Normal), paint.over(GREY, 200));
    }

    #[test]
    fn multiply_darkens_and_screen_lightens() {
        assert_eq!(blend_over(GREY, opaque(255, 128, 0), 255, BlendMode::Multiply), opaque(128, 64, 0));
        assert_eq!(blend_over(GREY, opaque(255, 128, 0), 255, BlendMode::Screen), opaque(255, 192, 128));
    }

    #[test]
    fn white_multiplies_to_no_change_and_black_screens_to_no_change() {
        let backdrop = opaque(30, 140, 220);
        assert_eq!(blend_over(Rgba::WHITE, backdrop, 255, BlendMode::Multiply), backdrop);
        assert_eq!(blend_over(Rgba::BLACK, backdrop, 255, BlendMode::Screen), backdrop);
    }

    #[test]
    fn difference_of_a_colour_with_itself_is_black() {
        let colour = opaque(90, 180, 30);
        assert_eq!(blend_over(colour, colour, 255, BlendMode::Difference), Rgba::BLACK);
    }

    #[test]
    fn addition_and_subtract_clamp_at_the_ends() {
        assert_eq!(
            blend_over(opaque(200, 0, 100), opaque(100, 50, 100), 255, BlendMode::Addition),
            opaque(255, 50, 200)
        );
        assert_eq!(blend_over(opaque(200, 0, 100), opaque(100, 50, 100), 255, BlendMode::Subtract), opaque(0, 50, 0));
    }

    #[test]
    fn half_opacity_lands_halfway_to_the_blend() {
        let mixed = blend_over(Rgba::BLACK, Rgba::WHITE, 128, BlendMode::Multiply);
        assert!((126..=129).contains(&mixed.r()), "{mixed}");
        assert_eq!(mixed.a(), 255);
    }

    #[test]
    fn over_transparency_every_mode_just_paints() {
        let paint = opaque(10, 200, 90);
        for mode in BlendMode::ALL {
            assert_eq!(blend_over(paint, Rgba::TRANSPARENT, 255, mode), paint, "{mode:?}");
        }
    }

    #[test]
    fn luminosity_mode_keeps_the_backdrop_hue() {
        let red = opaque(200, 0, 0);
        let bright = blend_over(Rgba::rgb(160, 160, 160), red, 255, BlendMode::Luminosity);
        assert!(bright.r() > bright.g() && bright.g() == bright.b(), "still a red: {bright}");
        assert!(luminosity(unit_rgb(bright)) > luminosity(unit_rgb(red)));
    }

    #[test]
    fn aseprite_blend_numbers_map_in_order() {
        assert_eq!(BlendMode::from_aseprite(1), Some(BlendMode::Multiply));
        assert_eq!(BlendMode::from_aseprite(18), Some(BlendMode::Divide));
        assert_eq!(BlendMode::from_aseprite(19), None);
    }
}

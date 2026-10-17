//! Colours as pixel artists think of them: exact 8-bit RGBA values.
//!
//! Spry never blends a colour that the artist didn't ask for, so `Rgba` is a
//! plain value type. The only maths here is what painting needs: alpha
//! compositing for layer opacity, and Oklab distance for snapping an
//! arbitrary colour (from a diffusion model, say) to the nearest palette
//! entry the way a human eye would.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Serialises as a hex string (`"#ff004d"`), so files and MCP arguments
/// read the way artists write colours.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rgba(pub [u8; 4]);

impl From<Rgba> for String {
    fn from(colour: Rgba) -> String {
        colour.to_hex()
    }
}

impl TryFrom<String> for Rgba {
    type Error = String;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        Rgba::from_hex(&text).ok_or_else(|| format!("`{text}` is not a colour; use #rrggbb or #rrggbbaa"))
    }
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba([0, 0, 0, 0]);
    pub const BLACK: Rgba = Rgba([0, 0, 0, 255]);
    pub const WHITE: Rgba = Rgba([255, 255, 255, 255]);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba([r, g, b, 255])
    }

    // `Rgba::rgb` and `Rgba::rgba` read as the colour notation they mirror.
    #[allow(clippy::self_named_constructors)]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Rgba([r, g, b, a])
    }

    pub fn r(self) -> u8 {
        self.0[0]
    }
    pub fn g(self) -> u8 {
        self.0[1]
    }
    pub fn b(self) -> u8 {
        self.0[2]
    }
    pub fn a(self) -> u8 {
        self.0[3]
    }

    pub fn is_transparent(self) -> bool {
        self.a() == 0
    }

    pub fn with_alpha(self, alpha: u8) -> Self {
        Rgba([self.r(), self.g(), self.b(), alpha])
    }

    /// Parses `#rgb`, `#rrggbb` or `#rrggbbaa`, with or without the hash.
    pub fn from_hex(text: &str) -> Option<Self> {
        let digits = text.trim().trim_start_matches('#');
        let byte_at = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
        match digits.len() {
            3 => {
                let nibble = |i: usize| u8::from_str_radix(digits.get(i..i + 1)?, 16).ok();
                Some(Rgba::rgb(nibble(0)? * 17, nibble(1)? * 17, nibble(2)? * 17))
            }
            6 => Some(Rgba::rgb(byte_at(0)?, byte_at(2)?, byte_at(4)?)),
            8 => Some(Rgba::rgba(byte_at(0)?, byte_at(2)?, byte_at(4)?, byte_at(6)?)),
            _ => None,
        }
    }

    /// `#rrggbb` for opaque colours and `#rrggbbaa` otherwise, so the common
    /// case reads the way artists write it.
    pub fn to_hex(self) -> String {
        if self.a() == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r(), self.g(), self.b())
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r(), self.g(), self.b(), self.a())
        }
    }

    /// Porter-Duff "source over": paints `self` on top of `below`, with the
    /// source's alpha already scaled by `opacity` (0–255).
    pub fn over(self, below: Rgba, opacity: u8) -> Rgba {
        let source_alpha = self.a() as u32 * opacity as u32 / 255;
        if source_alpha == 0 {
            return below;
        }
        if source_alpha == 255 {
            return self.with_alpha(255);
        }
        let below_alpha = below.a() as u32;
        let out_alpha = source_alpha + below_alpha * (255 - source_alpha) / 255;
        let channel = |s: u8, d: u8| {
            let weighted = s as u32 * source_alpha + d as u32 * below_alpha * (255 - source_alpha) / 255;
            (weighted / out_alpha.max(1)) as u8
        };
        Rgba([
            channel(self.r(), below.r()),
            channel(self.g(), below.g()),
            channel(self.b(), below.b()),
            out_alpha as u8,
        ])
    }

    /// Converts to the perceptual Oklab space (Björn Ottosson, 2020), where
    /// straight-line distance matches how different two colours look.
    pub fn to_oklab(self) -> [f32; 3] {
        let linear = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (linear(self.r()), linear(self.g()), linear(self.b()));
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        [
            0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        ]
    }

    pub fn perceptual_distance(self, other: Rgba) -> f32 {
        let (a, b) = (self.to_oklab(), other.to_oklab());
        let alpha_gap = (self.a() as f32 - other.a() as f32) / 255.0;
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2) + alpha_gap.powi(2)).sqrt()
    }

    /// Hue (0–360), saturation and value (0–1), for the colour picker.
    pub fn to_hsv(self) -> [f32; 3] {
        let (r, g, b) = (self.r() as f32 / 255.0, self.g() as f32 / 255.0, self.b() as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let hue = if delta == 0.0 {
            0.0
        } else if max == r {
            60.0 * ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        let saturation = if max == 0.0 { 0.0 } else { delta / max };
        [hue, saturation, max]
    }

    pub fn from_hsv(hue: f32, saturation: f32, value: f32, alpha: u8) -> Rgba {
        let chroma = value * saturation;
        let sector = (hue.rem_euclid(360.0)) / 60.0;
        let x = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
        let (r, g, b) = match sector as u32 {
            0 => (chroma, x, 0.0),
            1 => (x, chroma, 0.0),
            2 => (0.0, chroma, x),
            3 => (0.0, x, chroma),
            4 => (x, 0.0, chroma),
            _ => (chroma, 0.0, x),
        };
        let lift = value - chroma;
        let to_byte = |c: f32| ((c + lift) * 255.0).round().clamp(0.0, 255.0) as u8;
        Rgba([to_byte(r), to_byte(g), to_byte(b), alpha])
    }
}

impl fmt::Debug for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_in_every_supported_length() {
        assert_eq!(Rgba::from_hex("#f80"), Some(Rgba::rgb(255, 136, 0)));
        assert_eq!(Rgba::from_hex("29adff"), Some(Rgba::rgb(0x29, 0xad, 0xff)));
        assert_eq!(Rgba::from_hex("#11223380"), Some(Rgba::rgba(0x11, 0x22, 0x33, 0x80)));
        assert_eq!(Rgba::rgb(1, 2, 3).to_hex(), "#010203");
        assert_eq!(Rgba::rgba(1, 2, 3, 4).to_hex(), "#01020304");
    }

    #[test]
    fn malformed_hex_is_rejected_rather_than_guessed() {
        assert_eq!(Rgba::from_hex("#12345"), None);
        assert_eq!(Rgba::from_hex("zzzzzz"), None);
        assert_eq!(Rgba::from_hex(""), None);
    }

    #[test]
    fn serialises_as_a_hex_string() {
        assert_eq!(serde_json::to_string(&Rgba::rgb(255, 0, 77)).unwrap(), "\"#ff004d\"");
        assert_eq!(serde_json::from_str::<Rgba>("\"#ff004d\"").unwrap(), Rgba::rgb(255, 0, 77));
        assert!(serde_json::from_str::<Rgba>("\"red\"").is_err());
    }

    #[test]
    fn opaque_paint_replaces_what_is_below() {
        assert_eq!(Rgba::WHITE.over(Rgba::BLACK, 255), Rgba::WHITE);
    }

    #[test]
    fn transparent_paint_leaves_what_is_below() {
        assert_eq!(Rgba::TRANSPARENT.over(Rgba::BLACK, 255), Rgba::BLACK);
        assert_eq!(Rgba::WHITE.over(Rgba::BLACK, 0), Rgba::BLACK);
    }

    #[test]
    fn half_opacity_blends_halfway() {
        let blended = Rgba::WHITE.over(Rgba::BLACK, 128);
        assert!((126..=130).contains(&blended.r()), "{blended:?}");
        assert_eq!(blended.a(), 255);
    }

    #[test]
    fn painting_onto_nothing_keeps_the_paint_colour() {
        let blended = Rgba::rgb(200, 10, 10).over(Rgba::TRANSPARENT, 128);
        assert_eq!((blended.r(), blended.g()), (200, 10));
        assert_eq!(blended.a(), 128);
    }

    #[test]
    fn perceptual_distance_ranks_similar_colours_closer() {
        let red = Rgba::rgb(255, 0, 0);
        let dark_red = Rgba::rgb(200, 0, 0);
        let blue = Rgba::rgb(0, 0, 255);
        assert!(red.perceptual_distance(dark_red) < red.perceptual_distance(blue));
        assert_eq!(red.perceptual_distance(red), 0.0);
    }

    #[test]
    fn hsv_round_trips_primary_and_mixed_colours() {
        for colour in [Rgba::rgb(255, 0, 0), Rgba::rgb(18, 200, 77), Rgba::rgb(90, 90, 90)] {
            let [h, s, v] = colour.to_hsv();
            assert_eq!(Rgba::from_hsv(h, s, v, 255), colour);
        }
    }
}

//! Palettes: the ordered colour lists pixel art is built from.
//!
//! A palette is more than a swatch strip. Its order defines *ramps* (runs
//! from dark to light), which the shading tool walks along, and its colours
//! are the targets for snapping diffusion output, so AI art ends up drawn
//! in the artist's colours.

use super::color::Rgba;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub name: String,
    pub colours: Vec<Rgba>,
}

fn from_hex_list(name: &str, hexes: &[&str]) -> Palette {
    let colours = hexes.iter().map(|hex| Rgba::from_hex(hex).expect("built-in palettes are valid hex")).collect();
    Palette { name: name.to_string(), colours }
}

impl Palette {
    pub fn pico8() -> Palette {
        from_hex_list(
            "PICO-8",
            &[
                "000000", "1d2b53", "7e2553", "008751", "ab5236", "5f574f", "c2c3c7", "fff1e8", "ff004d", "ffa300",
                "ffec27", "00e436", "29adff", "83769c", "ff77a8", "ffccaa",
            ],
        )
    }

    pub fn db16() -> Palette {
        from_hex_list(
            "DawnBringer 16",
            &[
                "140c1c", "442434", "30346d", "4e4a4e", "854c30", "346524", "d04648", "757161", "597dce", "d27d2c",
                "8595a1", "6daa2c", "d2aa99", "6dc2ca", "dad45e", "deeed6",
            ],
        )
    }

    pub fn db32() -> Palette {
        from_hex_list(
            "DawnBringer 32",
            &[
                "000000", "222034", "45283c", "663931", "8f563b", "df7126", "d9a066", "eec39a", "fbf236", "99e550",
                "6abe30", "37946e", "4b692f", "524b24", "323c39", "3f3f74", "306082", "5b6ee1", "639bff", "5fcde4",
                "cbdbfc", "ffffff", "9badb7", "847e87", "696a6a", "595652", "76428a", "ac3232", "d95763", "d77bba",
                "8f974a", "8a6f30",
            ],
        )
    }

    pub fn endesga32() -> Palette {
        from_hex_list(
            "Endesga 32",
            &[
                "be4a2f", "d77643", "ead4aa", "e4a672", "b86f50", "733e39", "3e2731", "a22633", "e43b44", "f77622",
                "feae34", "fee761", "63c74d", "3e8948", "265c42", "193c3e", "124e89", "0099db", "2ce8f5", "ffffff",
                "c0cbdc", "8b9bb4", "5a6988", "3a4466", "262b44", "181425", "ff0044", "68386c", "b55088", "f6757a",
                "e8b796", "c28569",
            ],
        )
    }

    pub fn game_boy() -> Palette {
        from_hex_list("Game Boy", &["0f380f", "306230", "8bac0f", "9bbc0f"])
    }

    pub fn sweetie16() -> Palette {
        from_hex_list(
            "Sweetie 16",
            &[
                "1a1c2c", "5d275d", "b13e53", "ef7d57", "ffcd75", "a7f070", "38b764", "257179", "29366f", "3b5dc9",
                "41a6f6", "73eff7", "f4f4f4", "94b0c2", "566c86", "333c57",
            ],
        )
    }

    pub fn nes() -> Palette {
        from_hex_list(
            "NES",
            &[
                "7c7c7c", "0000fc", "0000bc", "4428bc", "940084", "a80020", "a81000", "881400", "503000", "007800",
                "006800", "005800", "004058", "000000", "bcbcbc", "0078f8", "0058f8", "6844fc", "d800cc", "e40058",
                "f83800", "e45c10", "ac7c00", "00b800", "00a800", "00a844", "008888", "f8f8f8", "3cbcfc", "6888fc",
                "9878f8", "f878f8", "f85898", "f87858", "fca044", "f8b800", "b8f818", "58d854", "58f898", "00e8d8",
                "787878", "fcfcfc", "a4e4fc", "b8b8f8", "d8b8f8", "f8b8f8", "f8a4c0", "f0d0b0", "fce0a8", "f8d878",
                "d8f878", "b8f8b8", "b8f8d8", "00fcfc",
            ],
        )
    }

    /// Every built-in palette, in the order the palette menu lists them.
    pub fn built_ins() -> Vec<Palette> {
        vec![
            Palette::pico8(),
            Palette::db16(),
            Palette::db32(),
            Palette::endesga32(),
            Palette::sweetie16(),
            Palette::game_boy(),
            Palette::nes(),
        ]
    }

    pub fn built_in_named(name: &str) -> Option<Palette> {
        let wanted = name.to_ascii_lowercase().replace([' ', '-', '_'], "");
        Palette::built_ins().into_iter().find(|p| p.name.to_ascii_lowercase().replace([' ', '-', '_'], "") == wanted)
    }

    /// The palette entry that looks closest to `colour`. Fully transparent
    /// input stays transparent, since a palette describes ink, not holes.
    pub fn nearest(&self, colour: Rgba) -> Rgba {
        if colour.is_transparent() || self.colours.is_empty() {
            return colour;
        }
        let opaque = colour.with_alpha(255);
        self.colours
            .iter()
            .copied()
            .min_by(|a, b| a.perceptual_distance(opaque).total_cmp(&b.perceptual_distance(opaque)))
            .unwrap_or(colour)
    }

    /// The colour `steps` places further along `colour`'s ramp. Positive
    /// steps go lighter, negative go darker. The ramp is the run of palette
    /// entries sorted by Oklab lightness among colours with a similar hue,
    /// so it works on palettes that weren't ordered with ramps in mind.
    pub fn shade(&self, colour: Rgba, steps: i32) -> Rgba {
        let Some(position) = self.colours.iter().position(|&c| c == colour.with_alpha(255)) else {
            return colour;
        };
        let ramp = self.ramp_through(position);
        let place = ramp.iter().position(|&c| c == self.colours[position]).unwrap_or(0) as i32;
        let target = (place + steps).clamp(0, ramp.len() as i32 - 1) as usize;
        ramp[target].with_alpha(colour.a())
    }

    fn ramp_through(&self, index: usize) -> Vec<Rgba> {
        let anchor = self.colours[index];
        let [_, anchor_a, anchor_b] = anchor.to_oklab();
        let anchor_chroma = (anchor_a * anchor_a + anchor_b * anchor_b).sqrt();
        let mut ramp: Vec<Rgba> = self
            .colours
            .iter()
            .copied()
            .filter(|c| {
                let [_, a, b] = c.to_oklab();
                let chroma = (a * a + b * b).sqrt();
                let both_greyish = chroma < 0.04 && anchor_chroma < 0.04;
                let hue_gap = ((a - anchor_a).powi(2) + (b - anchor_b).powi(2)).sqrt();
                both_greyish || hue_gap < 0.12
            })
            .collect();
        ramp.sort_by(|a, b| a.to_oklab()[0].total_cmp(&b.to_oklab()[0]));
        ramp.dedup();
        ramp
    }

    /// Reads a palette file: Paint.NET/Lospec `.hex` (one hex per line) or
    /// GIMP `.gpl`.
    pub fn parse(name: &str, text: &str) -> Result<Palette, String> {
        let colours = if text.trim_start().starts_with("GIMP Palette") {
            parse_gpl(text)?
        } else {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with(';') && !line.starts_with("//"))
                .map(|line| Rgba::from_hex(line).ok_or_else(|| format!("`{line}` is not a hex colour")))
                .collect::<Result<Vec<_>, _>>()?
        };
        if colours.is_empty() {
            return Err("the palette file has no colours".into());
        }
        Ok(Palette { name: name.to_string(), colours })
    }

    pub fn to_hex_file(&self) -> String {
        self.colours.iter().map(|c| c.to_hex().trim_start_matches('#').to_string() + "\n").collect()
    }

    pub fn to_gpl_file(&self) -> String {
        let mut text = format!("GIMP Palette\nName: {}\nColumns: 8\n#\n", self.name);
        for colour in &self.colours {
            text += &format!("{:3} {:3} {:3}\t{}\n", colour.r(), colour.g(), colour.b(), colour.to_hex());
        }
        text
    }
}

fn parse_gpl(text: &str) -> Result<Vec<Rgba>, String> {
    text.lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.contains(':'))
        .map(|line| {
            let channels: Vec<u8> = line.split_whitespace().take(3).filter_map(|n| n.parse().ok()).collect();
            match channels[..] {
                [r, g, b] => Ok(Rgba::rgb(r, g, b)),
                _ => Err(format!("`{line}` is not an `R G B` palette line")),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_palettes_have_their_canonical_sizes() {
        assert_eq!(Palette::pico8().colours.len(), 16);
        assert_eq!(Palette::db32().colours.len(), 32);
        assert_eq!(Palette::endesga32().colours.len(), 32);
        assert_eq!(Palette::game_boy().colours.len(), 4);
        assert_eq!(Palette::nes().colours.len(), 54);
    }

    #[test]
    fn built_ins_are_found_by_forgiving_names() {
        assert_eq!(Palette::built_in_named("pico8").map(|p| p.name), Some("PICO-8".into()));
        assert_eq!(Palette::built_in_named("game boy").map(|p| p.colours.len()), Some(4));
        assert!(Palette::built_in_named("vaporwave").is_none());
    }

    #[test]
    fn nearest_snaps_to_the_closest_looking_entry() {
        let gb = Palette::game_boy();
        assert_eq!(gb.nearest(Rgba::rgb(0, 0, 0)), Rgba::from_hex("0f380f").unwrap());
        assert_eq!(gb.nearest(Rgba::rgb(250, 255, 250)), Rgba::from_hex("9bbc0f").unwrap());
        assert_eq!(gb.nearest(Rgba::TRANSPARENT), Rgba::TRANSPARENT);
    }

    #[test]
    fn shading_walks_a_green_ramp_both_ways_and_stops_at_the_ends() {
        let gb = Palette::game_boy();
        let [darkest, dark, light, lightest] = [0, 1, 2, 3].map(|i| gb.colours[i]);
        assert_eq!(gb.shade(dark, 1), light);
        assert_eq!(gb.shade(light, -1), dark);
        assert_eq!(gb.shade(lightest, 5), lightest);
        assert_eq!(gb.shade(darkest, -1), darkest);
    }

    #[test]
    fn shading_an_off_palette_colour_leaves_it_alone() {
        assert_eq!(Palette::pico8().shade(Rgba::rgb(1, 2, 3), 1), Rgba::rgb(1, 2, 3));
    }

    #[test]
    fn hex_files_round_trip() {
        let palette = Palette::sweetie16();
        let parsed = Palette::parse("Sweetie 16", &palette.to_hex_file()).unwrap();
        assert_eq!(parsed, palette);
    }

    #[test]
    fn gpl_files_round_trip() {
        let palette = Palette::db16();
        assert_eq!(Palette::parse("DawnBringer 16", &palette.to_gpl_file()).unwrap(), palette);
    }

    #[test]
    fn broken_palette_files_say_which_line_is_wrong() {
        assert!(Palette::parse("x", "ff0000\nbanana\n").unwrap_err().contains("banana"));
        assert!(Palette::parse("x", "\n\n").is_err());
    }
}

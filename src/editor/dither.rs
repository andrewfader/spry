//! Dither patterns: the MS Paint "pattern fill" family and ordered Bayer
//! levels.
//!
//! A pattern answers one question: at canvas pixel `(x, y)`, does the
//! primary colour show (on) or the secondary (off)? Patterns are anchored to
//! canvas coordinates rather than the stroke, so two strokes painted side by
//! side tile seamlessly, just as Paint's fills did.
//!
//! Patterns travel as short names (`checker`, `bricks`, `bayer4:6`) so
//! humans, menus and LLMs over MCP all spell them the same way.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Pattern {
    Solid,
    Checker,
    Sparse25,
    Dense75,
    Dots,
    Horizontal,
    Vertical,
    DiagonalDown,
    DiagonalUp,
    CrossHatch,
    Bricks,
    /// An ordered-dither level: `size` is 2, 4 or 8, and `level` runs from 0
    /// (all secondary) to `size²` (all primary).
    Bayer {
        size: u8,
        level: u8,
    },
}

/// The fixed Paint-style patterns in menu order.
pub const CLASSIC_PATTERNS: [Pattern; 11] = [
    Pattern::Solid,
    Pattern::Checker,
    Pattern::Sparse25,
    Pattern::Dense75,
    Pattern::Dots,
    Pattern::Horizontal,
    Pattern::Vertical,
    Pattern::DiagonalDown,
    Pattern::DiagonalUp,
    Pattern::CrossHatch,
    Pattern::Bricks,
];

const BAYER_8: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// The Bayer threshold at `(x, y)` for a matrix of `size`, from 0 to
/// `size² - 1`. The 8×8 matrix is built recursively, so sampling it every
/// `8 / size` cells gives exactly the smaller Bayer matrix, with values
/// already in `0..size²`.
pub fn bayer_threshold(size: u8, x: i32, y: i32) -> u8 {
    let size = size.clamp(2, 8) as i32;
    let stride = (8 / size) as usize;
    let (cx, cy) = (x.rem_euclid(size) as usize, y.rem_euclid(size) as usize);
    BAYER_8[cy * stride][cx * stride]
}

impl Pattern {
    pub fn is_on(self, x: i32, y: i32) -> bool {
        let (mx, my) = (x.rem_euclid(8), y.rem_euclid(8));
        match self {
            Pattern::Solid => true,
            Pattern::Checker => (x + y).rem_euclid(2) == 0,
            Pattern::Sparse25 => x.rem_euclid(2) == 0 && y.rem_euclid(2) == 0,
            Pattern::Dense75 => !(x.rem_euclid(2) == 1 && y.rem_euclid(2) == 1),
            Pattern::Dots => mx % 4 == 0 && my % 4 == 0 || (mx % 4 == 2 && my % 4 == 2),
            Pattern::Horizontal => y.rem_euclid(2) == 0,
            Pattern::Vertical => x.rem_euclid(2) == 0,
            Pattern::DiagonalDown => (x - y).rem_euclid(4) == 0,
            Pattern::DiagonalUp => (x + y).rem_euclid(4) == 0,
            Pattern::CrossHatch => (x - y).rem_euclid(4) == 0 || (x + y).rem_euclid(4) == 0,
            Pattern::Bricks => {
                let mortar_row = my % 4 == 3;
                let joint_column = if my < 4 { mx == 0 } else { mx == 4 };
                mortar_row || joint_column
            }
            Pattern::Bayer { size, level } => bayer_threshold(size, x, y) < level,
        }
    }

    /// The share of pixels that are on, from 0 to 1. Patterns are sorted
    /// and labelled by this.
    pub fn coverage(self) -> f32 {
        let on = (0..8).flat_map(|y| (0..8).map(move |x| (x, y))).filter(|&(x, y)| self.is_on(x, y));
        on.count() as f32 / 64.0
    }

    pub fn name(self) -> String {
        self.into()
    }
}

/// Ordered-dither gradient: at `(x, y)` with blend position `t` (0 =
/// secondary, 1 = primary), is the primary shown? Uses the 8×8 matrix for
/// 65 smooth steps.
pub fn gradient_is_primary(x: i32, y: i32, t: f32) -> bool {
    let level = (t.clamp(0.0, 1.0) * 64.0).round() as u8;
    bayer_threshold(8, x, y) < level
}

impl From<Pattern> for String {
    fn from(pattern: Pattern) -> String {
        match pattern {
            Pattern::Solid => "solid".into(),
            Pattern::Checker => "checker".into(),
            Pattern::Sparse25 => "sparse25".into(),
            Pattern::Dense75 => "dense75".into(),
            Pattern::Dots => "dots".into(),
            Pattern::Horizontal => "horizontal".into(),
            Pattern::Vertical => "vertical".into(),
            Pattern::DiagonalDown => "diagonal_down".into(),
            Pattern::DiagonalUp => "diagonal_up".into(),
            Pattern::CrossHatch => "cross_hatch".into(),
            Pattern::Bricks => "bricks".into(),
            Pattern::Bayer { size, level } => format!("bayer{size}:{level}"),
        }
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from(*self))
    }
}

impl FromStr for Pattern {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim().to_ascii_lowercase();
        if let Some(classic) = CLASSIC_PATTERNS.iter().find(|p| p.name() == text) {
            return Ok(*classic);
        }
        let unknown = || {
            format!(
                "unknown dither pattern `{text}`; use one of {} or bayer2:N / bayer4:N / bayer8:N",
                CLASSIC_PATTERNS.map(|p| p.name()).join(", ")
            )
        };
        let (matrix, level) = text.strip_prefix("bayer").and_then(|rest| rest.split_once(':')).ok_or_else(unknown)?;
        let size: u8 = matrix.parse().map_err(|_| unknown())?;
        let level: u8 = level.parse().map_err(|_| unknown())?;
        if ![2, 4, 8].contains(&size) || level as u32 > (size as u32).pow(2) {
            return Err(format!("bayer{size}:{level} is out of range; size is 2, 4 or 8 and level is 0..=size²"));
        }
        Ok(Pattern::Bayer { size, level })
    }
}

impl TryFrom<String> for Pattern {
    type Error = String;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checker_is_on_where_x_plus_y_is_even() {
        assert!(Pattern::Checker.is_on(0, 0));
        assert!(!Pattern::Checker.is_on(1, 0));
        assert!(Pattern::Checker.is_on(1, 1));
        assert!(Pattern::Checker.is_on(-1, -1), "patterns tile into negative space");
    }

    #[test]
    fn classic_coverages_match_their_names() {
        assert_eq!(Pattern::Solid.coverage(), 1.0);
        assert_eq!(Pattern::Checker.coverage(), 0.5);
        assert_eq!(Pattern::Sparse25.coverage(), 0.25);
        assert_eq!(Pattern::Dense75.coverage(), 0.75);
        assert_eq!(Pattern::Horizontal.coverage(), 0.5);
    }

    #[test]
    fn every_bayer_level_lights_exactly_level_cells_per_tile() {
        for size in [2u8, 4, 8] {
            for level in 0..=size * size {
                let tile = (0..size as i32).flat_map(|y| (0..size as i32).map(move |x| (x, y)));
                let lit = tile.filter(|&(x, y)| Pattern::Bayer { size, level }.is_on(x, y)).count();
                assert_eq!(lit, level as usize, "bayer{size}:{level}");
            }
        }
    }

    #[test]
    fn bayer_levels_are_nested_so_gradients_never_flicker() {
        for level in 0..64u8 {
            for (x, y) in (0..8).flat_map(|y| (0..8).map(move |x| (x, y))) {
                if (Pattern::Bayer { size: 8, level }).is_on(x, y) {
                    assert!((Pattern::Bayer { size: 8, level: level + 1 }).is_on(x, y));
                }
            }
        }
    }

    #[test]
    fn patterns_round_trip_through_their_names() {
        for pattern in CLASSIC_PATTERNS.into_iter().chain([Pattern::Bayer { size: 4, level: 9 }]) {
            assert_eq!(pattern.name().parse::<Pattern>(), Ok(pattern));
        }
    }

    #[test]
    fn unknown_or_out_of_range_names_explain_themselves() {
        assert!("plaid".parse::<Pattern>().unwrap_err().contains("checker"));
        assert!("bayer4:17".parse::<Pattern>().unwrap_err().contains("out of range"));
        assert!("bayer3:1".parse::<Pattern>().is_err());
    }

    #[test]
    fn gradient_runs_from_all_secondary_to_all_primary() {
        let tile = || (0..8).flat_map(|y| (0..8).map(move |x| (x, y)));
        assert!(tile().all(|(x, y)| !gradient_is_primary(x, y, 0.0)));
        assert!(tile().all(|(x, y)| gradient_is_primary(x, y, 1.0)));
        assert_eq!(tile().filter(|&(x, y)| gradient_is_primary(x, y, 0.5)).count(), 32);
    }
}

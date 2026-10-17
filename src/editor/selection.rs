//! Selections and floating pixels.
//!
//! A `Selection` is a per-pixel mask over the canvas. Masks rather than
//! rectangles are what make the magic wand and "invert" possible, and they
//! clip painting exactly. A `Floating` is a lifted piece of art that
//! hovers above the cel while you move it, until it's anchored back down.

use super::pixels::{Pixels, Rect};
use super::raster::Point;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    width: u32,
    height: u32,
    mask: Vec<bool>,
}

impl Selection {
    pub fn empty(width: u32, height: u32) -> Selection {
        Selection { width, height, mask: vec![false; (width * height) as usize] }
    }

    pub fn all(width: u32, height: u32) -> Selection {
        Selection { width, height, mask: vec![true; (width * height) as usize] }
    }

    pub fn from_rect(width: u32, height: u32, rect: Rect) -> Selection {
        Selection::from_points(width, height, rect.points())
    }

    pub fn from_points(width: u32, height: u32, points: impl IntoIterator<Item = Point>) -> Selection {
        let mut selection = Selection::empty(width, height);
        for (x, y) in points {
            selection.set(x, y, true);
        }
        selection
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height)
            .then(|| (y as u32 * self.width + x as u32) as usize)
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|i| self.mask[i])
    }

    pub fn set(&mut self, x: i32, y: i32, selected: bool) {
        if let Some(i) = self.index(x, y) {
            self.mask[i] = selected;
        }
    }

    pub fn inverted(&self) -> Selection {
        Selection { width: self.width, height: self.height, mask: self.mask.iter().map(|s| !s).collect() }
    }

    /// Adds another selection to this one (Shift-select).
    pub fn union(&self, other: &Selection) -> Selection {
        let mask = self.mask.iter().zip(&other.mask).map(|(a, b)| *a || *b).collect();
        Selection { width: self.width, height: self.height, mask }
    }

    pub fn is_empty(&self) -> bool {
        !self.mask.iter().any(|&s| s)
    }

    pub fn bounds(&self) -> Option<Rect> {
        let mut extent: Option<(i32, i32, i32, i32)> = None;
        for (x, y) in Rect::new(0, 0, self.width, self.height).points() {
            if self.contains(x, y) {
                let (l, t, r, b) = extent.unwrap_or((x, y, x, y));
                extent = Some((l.min(x), t.min(y), r.max(x), b.max(y)));
            }
        }
        extent.map(|(l, t, r, b)| Rect::spanning((l, t), (r, b)))
    }

    pub fn points(&self) -> impl Iterator<Item = Point> + '_ {
        Rect::new(0, 0, self.width, self.height).points().filter(|&(x, y)| self.contains(x, y))
    }

    /// Copies the selected pixels out of `image` into an image the size of
    /// the selection's bounds. Unselected pixels come out transparent.
    pub fn extract(&self, image: &Pixels) -> Option<(Pixels, Rect)> {
        let bounds = self.bounds()?;
        let mut piece = Pixels::new(bounds.width, bounds.height);
        for (x, y) in bounds.points() {
            if self.contains(x, y) {
                piece.set(x - bounds.x, y - bounds.y, image.get(x, y));
            }
        }
        Some((piece, bounds))
    }

    /// Selection edges as unit line segments in pixel-corner coordinates,
    /// for drawing "marching ants" around any mask shape.
    pub fn outline_segments(&self) -> Vec<((i32, i32), (i32, i32))> {
        let mut segments = Vec::new();
        for (x, y) in self.points() {
            if !self.contains(x, y - 1) {
                segments.push(((x, y), (x + 1, y)));
            }
            if !self.contains(x, y + 1) {
                segments.push(((x, y + 1), (x + 1, y + 1)));
            }
            if !self.contains(x - 1, y) {
                segments.push(((x, y), (x, y + 1)));
            }
            if !self.contains(x + 1, y) {
                segments.push(((x + 1, y), (x + 1, y + 1)));
            }
        }
        segments
    }
}

/// Pixels lifted off a cel and hovering at `(x, y)` until anchored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Floating {
    pub pixels: Pixels,
    pub x: i32,
    pub y: i32,
}

impl Floating {
    pub fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, self.pixels.width(), self.pixels.height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::color::Rgba;

    #[test]
    fn rectangle_selections_contain_exactly_their_rect() {
        let selection = Selection::from_rect(8, 8, Rect::new(2, 2, 3, 3));
        assert!(selection.contains(2, 2));
        assert!(selection.contains(4, 4));
        assert!(!selection.contains(5, 5));
        assert_eq!(selection.bounds(), Some(Rect::new(2, 2, 3, 3)));
    }

    #[test]
    fn inverting_swaps_inside_and_outside() {
        let selection = Selection::from_rect(4, 4, Rect::new(0, 0, 2, 2)).inverted();
        assert!(!selection.contains(0, 0));
        assert!(selection.contains(3, 3));
        assert_eq!(selection.points().count(), 12);
    }

    #[test]
    fn union_combines_two_selections() {
        let a = Selection::from_rect(4, 1, Rect::new(0, 0, 1, 1));
        let b = Selection::from_rect(4, 1, Rect::new(3, 0, 1, 1));
        assert_eq!(a.union(&b).points().collect::<Vec<_>>(), vec![(0, 0), (3, 0)]);
    }

    #[test]
    fn extracting_keeps_only_selected_pixels() {
        let image = Pixels::filled(4, 4, Rgba::BLACK);
        let selection = Selection::from_points(4, 4, [(1, 1), (2, 2)]);
        let (piece, at) = selection.extract(&image).unwrap();
        assert_eq!(at, Rect::new(1, 1, 2, 2));
        assert_eq!(piece.get(0, 0), Rgba::BLACK);
        assert_eq!(piece.get(1, 0), Rgba::TRANSPARENT);
    }

    #[test]
    fn a_single_pixel_selection_has_four_outline_edges() {
        assert_eq!(Selection::from_points(3, 3, [(1, 1)]).outline_segments().len(), 4);
    }

    #[test]
    fn empty_selections_have_no_bounds() {
        assert!(Selection::empty(4, 4).is_empty());
        assert_eq!(Selection::empty(4, 4).bounds(), None);
    }
}

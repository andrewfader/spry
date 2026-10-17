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

    /// A freehand lasso: the pixels whose centres fall inside the closed
    /// outline (even-odd rule, so a figure-eight leaves its crossing out),
    /// plus the pixels the outline itself runs through, so a thin stroke
    /// still selects what it traced.
    pub fn from_polygon(width: u32, height: u32, outline: &[Point]) -> Selection {
        let mut selection = Selection::empty(width, height);
        let Some(&first) = outline.first() else { return selection };
        let mut closed = outline.to_vec();
        closed.push(first);
        for point in super::raster::connect(&closed) {
            selection.set(point.0, point.1, true);
        }
        let (low_y, high_y) = outline.iter().fold((i32::MAX, i32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
        for y in low_y.max(0)..=high_y.min(height as i32 - 1) {
            for x in crossings_inside(&closed, y as f32 + 0.5, width) {
                selection.set(x, y, true);
            }
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

/// The pixel columns on row `centre_y` whose centres are inside a closed
/// outline: sort where the edges cross the row, then fill between pairs.
fn crossings_inside(closed: &[Point], centre_y: f32, width: u32) -> Vec<i32> {
    let mut crossings: Vec<f32> = closed
        .windows(2)
        .filter_map(|edge| {
            let ((x0, y0), (x1, y1)) =
                ((edge[0].0 as f32 + 0.5, edge[0].1 as f32 + 0.5), (edge[1].0 as f32 + 0.5, edge[1].1 as f32 + 0.5));
            let spans_row = (y0 <= centre_y && centre_y < y1) || (y1 <= centre_y && centre_y < y0);
            spans_row.then(|| x0 + (centre_y - y0) / (y1 - y0) * (x1 - x0))
        })
        .collect();
    crossings.sort_by(f32::total_cmp);
    crossings
        .chunks_exact(2)
        .flat_map(|pair| {
            let from = (pair[0] - 0.5).ceil().max(0.0) as i32;
            let to = ((pair[1] - 0.5).floor() as i32).min(width as i32 - 1);
            from..=to
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_lasso_selects_the_inside_of_its_outline() {
        // A triangle with its corner at the top left.
        let lasso = super::Selection::from_polygon(10, 10, &[(0, 0), (8, 0), (0, 8)]);
        assert!(lasso.contains(1, 1), "inside");
        assert!(lasso.contains(4, 0) && lasso.contains(0, 4), "on the outline");
        assert!(!lasso.contains(7, 7), "past the diagonal");
        assert!(!lasso.contains(9, 9));
    }

    #[test]
    fn an_hourglass_lasso_selects_its_two_lobes_and_not_the_gaps() {
        // Edges cross in the middle: lobes to the left and right, open
        // wedges above and below.
        let lasso = super::Selection::from_polygon(10, 10, &[(0, 0), (9, 9), (9, 0), (0, 9)]);
        assert!(lasso.contains(1, 4) && lasso.contains(8, 4), "both lobes");
        assert!(!lasso.contains(4, 1) && !lasso.contains(4, 8), "neither wedge");
    }

    #[test]
    fn an_empty_lasso_selects_nothing() {
        let lasso = super::Selection::from_polygon(4, 4, &[]);
        assert!(lasso.bounds().is_none());
    }

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

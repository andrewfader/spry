//! One-click layer effects that pixel artists otherwise do by hand: an
//! outline around a sprite, and a hard drop shadow.
//!
//! These are real edits to the pixels, not render-time filters, so what you
//! see is exactly what exports. And like everything else, they undo.

use super::color::Rgba;
use super::pixels::Pixels;

/// Paints `colour` on every transparent pixel next to an opaque one. With
/// `diagonals`, corner neighbours count too, which gives the thicker
/// "8-way" outline style.
pub fn outline(image: &Pixels, colour: Rgba, diagonals: bool) -> Pixels {
    let mut out = image.clone();
    let orthogonal = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let diagonal = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
    for (x, y) in image.bounds().points() {
        if !image.get(x, y).is_transparent() {
            continue;
        }
        let touches =
            |offsets: &[(i32, i32)]| offsets.iter().any(|(dx, dy)| !image.get(x + dx, y + dy).is_transparent());
        if touches(&orthogonal) || (diagonals && touches(&diagonal)) {
            out.set(x, y, colour);
        }
    }
    out
}

/// Adds a solid shadow of the sprite's silhouette, offset by `(dx, dy)`,
/// behind the existing pixels.
pub fn drop_shadow(image: &Pixels, colour: Rgba, dx: i32, dy: i32) -> Pixels {
    let mut out = image.clone();
    for (x, y) in image.bounds().points() {
        if !image.get(x, y).is_transparent() && image.get(x + dx, y + dy).is_transparent() {
            out.set(x + dx, y + dy, colour);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = Rgba::rgb(255, 0, 0);

    fn dot() -> Pixels {
        let mut image = Pixels::new(5, 5);
        image.set(2, 2, RED);
        image
    }

    #[test]
    fn outline_rings_a_dot_with_four_pixels() {
        let outlined = outline(&dot(), Rgba::BLACK, false);
        let blacks = outlined.colours().iter().filter(|&&c| c == Rgba::BLACK).count();
        assert_eq!(blacks, 4);
        assert_eq!(outlined.get(2, 2), RED, "the sprite itself is untouched");
    }

    #[test]
    fn eight_way_outline_also_fills_corners() {
        let outlined = outline(&dot(), Rgba::BLACK, true);
        assert_eq!(outlined.colours().iter().filter(|&&c| c == Rgba::BLACK).count(), 8);
    }

    #[test]
    fn drop_shadow_sits_behind_and_offset() {
        let shadowed = drop_shadow(&dot(), Rgba::BLACK, 1, 1);
        assert_eq!(shadowed.get(3, 3), Rgba::BLACK);
        assert_eq!(shadowed.get(2, 2), RED);
    }
}

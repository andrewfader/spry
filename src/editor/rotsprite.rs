//! Rotsprite: rotation by any angle that still looks hand-drawn.
//!
//! Rotating pixel art with plain nearest-neighbour sampling chews up lines
//! and leaves holes. Rotsprite (Xenowhirl, 2007) first enlarges the image
//! 8× with Scale2x three times. Scale2x rounds off staircases, so diagonal
//! lines stay lines. It then rotates that big picture with nearest
//! neighbour and samples it back down to 1×. No colour is ever invented,
//! so the result stays in the palette.

use super::color::Rgba;
use super::pixels::Pixels;

/// The largest piece Rotsprite will turn. Working at 8×, a 512² piece is
/// already a 4096² intermediate.
pub const MAX_ROTSPRITE_SIDE: u32 = 512;

const UPSCALE: i32 = 8;

/// Rotates `image` clockwise by `degrees` about its centre. The result is
/// sized to hold the whole rotated picture, centred on the same point.
pub fn rotsprite(image: &Pixels, degrees: f32) -> Pixels {
    let quarter_turns = degrees / 90.0;
    if (quarter_turns - quarter_turns.round()).abs() < 1e-4 {
        return exact_quarter_turns(image, quarter_turns.round() as i32);
    }
    let big = scale2x(&scale2x(&scale2x(image)));
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (width, height) = rotated_size(image.width(), image.height(), sin, cos);
    let mut out = Pixels::new(width, height);
    let source_centre = (image.width() as f32 / 2.0, image.height() as f32 / 2.0);
    let target_centre = (width as f32 / 2.0, height as f32 / 2.0);
    for (x, y) in out.bounds().points() {
        // Sample the middle of each output pixel, turned back into the
        // source, at 8× (inverse mapping, so every output pixel is filled
        // exactly once).
        let (dx, dy) = (x as f32 + 0.5 - target_centre.0, y as f32 + 0.5 - target_centre.1);
        let source_x = dx * cos + dy * sin + source_centre.0;
        let source_y = -dx * sin + dy * cos + source_centre.1;
        let sample = big.get((source_x * UPSCALE as f32).floor() as i32, (source_y * UPSCALE as f32).floor() as i32);
        out.set(x, y, sample);
    }
    out
}

fn exact_quarter_turns(image: &Pixels, turns: i32) -> Pixels {
    (0..turns.rem_euclid(4)).fold(image.clone(), |turned, _| turned.rotated_clockwise())
}

/// The bounding box of a `width × height` rectangle turned by the angle.
fn rotated_size(width: u32, height: u32, sin: f32, cos: f32) -> (u32, u32) {
    let (w, h) = (width as f32, height as f32);
    let new_width = (w * cos.abs() + h * sin.abs()).round().max(1.0);
    let new_height = (w * sin.abs() + h * cos.abs()).round().max(1.0);
    (new_width as u32, new_height as u32)
}

/// Scale2x (also called EPX): each pixel becomes four, and a corner takes
/// a neighbour's colour when that neighbour forms a clean diagonal edge.
pub fn scale2x(image: &Pixels) -> Pixels {
    let mut out = Pixels::new(image.width() * 2, image.height() * 2);
    // Outside the image counts as the pixel itself, so edges don't grow
    // phantom corners.
    let at = |x: i32, y: i32, fallback: Rgba| if image.contains(x, y) { image.get(x, y) } else { fallback };
    for (x, y) in image.bounds().points() {
        let p = image.get(x, y);
        let (a, b, c, d) = (at(x, y - 1, p), at(x + 1, y, p), at(x - 1, y, p), at(x, y + 1, p));
        let top_left = if c == a && c != d && a != b { a } else { p };
        let top_right = if a == b && a != c && b != d { b } else { p };
        let bottom_left = if d == c && d != b && c != a { c } else { p };
        let bottom_right = if b == d && b != a && d != c { d } else { p };
        out.set(x * 2, y * 2, top_left);
        out.set(x * 2 + 1, y * 2, top_right);
        out.set(x * 2, y * 2 + 1, bottom_left);
        out.set(x * 2 + 1, y * 2 + 1, bottom_right);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: Rgba = Rgba::rgb(20, 20, 30);
    const SKIN: Rgba = Rgba::rgb(240, 180, 140);

    fn diagonal(size: u32) -> Pixels {
        let mut image = Pixels::new(size, size);
        for i in 0..size as i32 {
            image.set(i, i, INK);
        }
        image
    }

    #[test]
    fn scale2x_rounds_a_staircase_into_a_line() {
        let big = scale2x(&diagonal(3));
        // The corners between steps fill in, so the diagonal has no gaps.
        assert_eq!(big.get(1, 0), INK);
        assert_eq!(big.get(0, 1), INK);
        assert_eq!(big.get(3, 0), Rgba::TRANSPARENT, "but it doesn't spread");
    }

    #[test]
    fn quarter_turns_are_exact() {
        let image = diagonal(5);
        assert_eq!(rotsprite(&image, 90.0), image.rotated_clockwise());
        assert_eq!(rotsprite(&image, 360.0), image);
        assert_eq!(rotsprite(&image, -90.0), image.rotated_clockwise().rotated_clockwise().rotated_clockwise());
    }

    #[test]
    fn rotating_never_invents_colours() {
        let mut image = Pixels::filled(12, 8, SKIN);
        for x in 0..12 {
            image.set(x, 0, INK);
        }
        let turned = rotsprite(&image, 30.0);
        assert!(turned.colours().iter().all(|c| [INK, SKIN, Rgba::TRANSPARENT].contains(c)));
        assert!(turned.colours().contains(&INK), "the outline survives");
    }

    #[test]
    fn the_result_is_big_enough_for_the_whole_turned_picture() {
        let turned = rotsprite(&Pixels::filled(10, 10, SKIN), 45.0);
        assert_eq!((turned.width(), turned.height()), (14, 14));
        assert_eq!(turned.get(7, 7), SKIN, "the middle is still filled");
        assert_eq!(turned.get(0, 0), Rgba::TRANSPARENT, "the new corners are empty");
    }

    #[test]
    fn a_line_turned_fifteen_degrees_stays_one_connected_line() {
        let mut image = Pixels::new(16, 3);
        for x in 0..16 {
            image.set(x, 1, INK);
        }
        let turned = rotsprite(&image, 15.0);
        let inked: Vec<(i32, i32)> = turned.bounds().points().filter(|&(x, y)| turned.get(x, y) == INK).collect();
        let connected = inked.iter().all(|&(x, y)| {
            inked.len() == 1
                || inked.iter().any(|&(ox, oy)| (ox, oy) != (x, y) && (ox - x).abs() <= 1 && (oy - y).abs() <= 1)
        });
        assert!(inked.len() >= 14, "the line keeps its length: {}", inked.len());
        assert!(connected, "no pixel of the line floats loose");
    }
}

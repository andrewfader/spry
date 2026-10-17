//! Rasterising: turns shapes into exact lists of pixel coordinates.
//!
//! Every function here is pure. It answers "which pixels?" and leaves "what
//! colour?" to inks and dithers. Keeping the two apart is what lets any
//! shape be drawn with any ink, pattern or symmetry.

use super::pixels::{Pixels, Rect};
use std::collections::HashSet;

pub type Point = (i32, i32);

/// Bresenham's line, both end points included. Steps are 8-connected, so a
/// line never has gaps and never doubles up on a step.
pub fn line(from: Point, to: Point) -> Vec<Point> {
    let (mut x, mut y) = from;
    let dx = (to.0 - x).abs();
    let dy = -(to.1 - y).abs();
    let step_x = if x < to.0 { 1 } else { -1 };
    let step_y = if y < to.1 { 1 } else { -1 };
    let mut error = dx + dy;
    let mut points = Vec::with_capacity((dx.max(-dy) + 1) as usize);
    loop {
        points.push((x, y));
        if (x, y) == to {
            return points;
        }
        let doubled = 2 * error;
        if doubled >= dy {
            error += dy;
            x += step_x;
        }
        if doubled <= dx {
            error += dx;
            y += step_y;
        }
    }
}

pub fn rectangle_outline(corner_a: Point, corner_b: Point) -> Vec<Point> {
    let rect = Rect::spanning(corner_a, corner_b);
    let (left, top, right, bottom) = (rect.x, rect.y, rect.right() - 1, rect.bottom() - 1);
    let mut points: Vec<Point> = (left..=right).flat_map(|x| [(x, top), (x, bottom)]).collect();
    points.extend((top + 1..bottom).flat_map(|y| [(left, y), (right, y)]));
    deduplicated(points)
}

pub fn rectangle_filled(corner_a: Point, corner_b: Point) -> Vec<Point> {
    Rect::spanning(corner_a, corner_b).points().collect()
}

/// The ellipse that fits inside the rectangle spanned by two corners, drawn
/// with Alois Zingl's integer algorithm. It's symmetric for both even and
/// odd sizes, so small sprites get round eyes rather than lopsided ones.
pub fn ellipse_outline(corner_a: Point, corner_b: Point) -> Vec<Point> {
    let (mut x0, mut y0, mut x1, mut y1) = (corner_a.0 as i64, corner_a.1 as i64, corner_b.0 as i64, corner_b.1 as i64);
    let mut a = (x1 - x0).abs();
    let b = (y1 - y0).abs();
    let mut b1 = b & 1;
    let mut dx = 4 * (1 - a) * b * b;
    let mut dy = 4 * (b1 + 1) * a * a;
    let mut error = dx + dy + b1 * a * a;
    if x0 > x1 {
        x0 = x1;
        x1 += a;
    }
    if y0 > y1 {
        y0 = y1;
    }
    y0 += (b + 1) / 2;
    y1 = y0 - b1;
    a = 8 * a * a;
    b1 = 8 * b * b;

    let mut points = Vec::new();
    let mut plot = |x: i64, y: i64| points.push((x as i32, y as i32));
    loop {
        plot(x1, y0);
        plot(x0, y0);
        plot(x0, y1);
        plot(x1, y1);
        let doubled = 2 * error;
        if doubled <= dy {
            y0 += 1;
            y1 -= 1;
            dy += a;
            error += dy;
        }
        if doubled >= dx || 2 * error > dy {
            x0 += 1;
            x1 -= 1;
            dx += b1;
            error += dx;
        }
        if x0 > x1 {
            break;
        }
    }
    // Very flat ellipses finish their tips here.
    while y0 - y1 <= b {
        plot(x0 - 1, y0);
        plot(x1 + 1, y0);
        y0 += 1;
        plot(x0 - 1, y1);
        plot(x1 + 1, y1);
        y1 -= 1;
    }
    let rect = Rect::spanning(corner_a, corner_b);
    deduplicated(points.into_iter().filter(|&(x, y)| rect.contains(x, y)).collect())
}

/// Fills every row of the outline between its leftmost and rightmost pixel.
pub fn ellipse_filled(corner_a: Point, corner_b: Point) -> Vec<Point> {
    let mut extent_by_row: std::collections::BTreeMap<i32, (i32, i32)> = Default::default();
    for (x, y) in ellipse_outline(corner_a, corner_b) {
        let span = extent_by_row.entry(y).or_insert((x, x));
        *span = (span.0.min(x), span.1.max(x));
    }
    extent_by_row.into_iter().flat_map(|(y, (left, right))| (left..=right).map(move |x| (x, y))).collect()
}

/// Joins sampled mouse positions into a continuous path, since the mouse
/// can skip pixels between frames.
pub fn connect(samples: &[Point]) -> Vec<Point> {
    let mut path: Vec<Point> = Vec::new();
    for window in samples.windows(2) {
        let segment = line(window[0], window[1]);
        let skip_shared_start = usize::from(!path.is_empty());
        path.extend(segment.into_iter().skip(skip_shared_start));
    }
    if path.is_empty() {
        path.extend(samples.first().copied());
    }
    path
}

/// Pixel-perfect strokes: drops the inner pixel of any "L" corner, so a
/// freehand curve stays one pixel thin, the way a pixel artist would clean
/// it up by hand.
pub fn pixel_perfect(path: &[Point]) -> Vec<Point> {
    let mut kept: Vec<Point> = Vec::with_capacity(path.len());
    for (i, &point) in path.iter().enumerate() {
        let next = path.get(i + 1);
        let forms_an_l_corner = match (kept.last(), next) {
            (Some(&previous), Some(&next)) => {
                let touches_previous_orthogonally = previous.0 == point.0 || previous.1 == point.1;
                let touches_next_orthogonally = next.0 == point.0 || next.1 == point.1;
                let ends_are_diagonal = previous.0 != next.0 && previous.1 != next.1;
                touches_previous_orthogonally && touches_next_orthogonally && ends_are_diagonal
            }
            _ => false,
        };
        if !forms_an_l_corner {
            kept.push(point);
        }
    }
    kept
}

/// Every pixel 4-connected to `start` that shares its exact colour. This is
/// the classic bucket fill and the magic wand.
pub fn contiguous_region(image: &Pixels, start: Point) -> Vec<Point> {
    if !image.contains(start.0, start.1) {
        return Vec::new();
    }
    let target = image.get(start.0, start.1);
    let mut seen = vec![false; (image.width() * image.height()) as usize];
    let index = |(x, y): Point| (y as u32 * image.width() + x as u32) as usize;
    let mut region = Vec::new();
    let mut frontier = vec![start];
    seen[index(start)] = true;
    while let Some((x, y)) = frontier.pop() {
        region.push((x, y));
        for neighbour in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            if image.contains(neighbour.0, neighbour.1)
                && !seen[index(neighbour)]
                && image.get(neighbour.0, neighbour.1) == target
            {
                seen[index(neighbour)] = true;
                frontier.push(neighbour);
            }
        }
    }
    region
}

/// Every pixel anywhere in the image with the same colour as `start`
/// ("fill all matching", Aseprite's non-contiguous bucket).
pub fn matching_colour(image: &Pixels, start: Point) -> Vec<Point> {
    if !image.contains(start.0, start.1) {
        return Vec::new();
    }
    let target = image.get(start.0, start.1);
    image.bounds().points().filter(|&(x, y)| image.get(x, y) == target).collect()
}

/// A square brush footprint of `size` pixels, centred the way Aseprite
/// centres it: even sizes lean up and to the left.
pub fn square_brush(centre: Point, size: u32) -> Vec<Point> {
    let size = size.max(1) as i32;
    let start = -(size / 2);
    (0..size).flat_map(|dy| (0..size).map(move |dx| (centre.0 + start + dx, centre.1 + start + dy))).collect()
}

/// A round brush footprint, for sizes above 2.
pub fn round_brush(centre: Point, size: u32) -> Vec<Point> {
    if size <= 2 {
        return square_brush(centre, size);
    }
    let radius = size as f32 / 2.0;
    square_brush(centre, size)
        .into_iter()
        .filter(|&(x, y)| {
            let (dx, dy) = ((x - centre.0) as f32, (y - centre.1) as f32);
            dx * dx + dy * dy <= radius * radius
        })
        .collect()
}

/// A spray-can burst: `count` points scattered evenly over a disc. It's
/// seeded so the same stroke always gives the same speckle, which keeps
/// undo/redo and tests deterministic.
pub fn spray(centre: Point, radius: u32, count: u32, seed: u64) -> Vec<Point> {
    let mut rng = fastrand::Rng::with_seed(seed);
    let radius = radius.max(1) as f32;
    (0..count)
        .map(move |_| {
            let angle = rng.f32() * std::f32::consts::TAU;
            let distance = rng.f32().sqrt() * radius;
            (centre.0 + (angle.cos() * distance).round() as i32, centre.1 + (angle.sin() * distance).round() as i32)
        })
        .collect()
}

pub fn deduplicated(points: Vec<Point>) -> Vec<Point> {
    let mut seen = HashSet::with_capacity(points.len());
    points.into_iter().filter(|p| seen.insert(*p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::color::Rgba;

    fn sorted(mut points: Vec<Point>) -> Vec<Point> {
        points.sort();
        points
    }

    #[test]
    fn a_shallow_line_steps_like_bresenham() {
        assert_eq!(line((0, 0), (7, 3)), vec![(0, 0), (1, 0), (2, 1), (3, 1), (4, 2), (5, 2), (6, 3), (7, 3)]);
    }

    #[test]
    fn lines_are_the_same_pixels_drawn_either_way() {
        assert_eq!(sorted(line((0, 0), (5, 9))).len(), sorted(line((5, 9), (0, 0))).len());
    }

    #[test]
    fn a_single_point_line_is_one_pixel() {
        assert_eq!(line((4, 4), (4, 4)), vec![(4, 4)]);
    }

    #[test]
    fn rectangle_outline_has_a_hollow_middle() {
        let outline = rectangle_outline((0, 0), (3, 2));
        assert_eq!(outline.len(), 10);
        assert!(!outline.contains(&(1, 1)));
    }

    #[test]
    fn ellipse_stays_inside_its_rectangle_and_is_symmetric() {
        let points = ellipse_outline((0, 0), (9, 5));
        assert!(points.iter().all(|&(x, y)| (0..=9).contains(&x) && (0..=5).contains(&y)));
        for &(x, y) in &points {
            assert!(points.contains(&(9 - x, y)), "missing mirror of {:?}", (x, y));
            assert!(points.contains(&(x, 5 - y)), "missing mirror of {:?}", (x, y));
        }
    }

    #[test]
    fn ellipse_touches_all_four_sides_of_its_rectangle() {
        let points = ellipse_outline((2, 3), (12, 9));
        assert!(points.iter().any(|p| p.0 == 2));
        assert!(points.iter().any(|p| p.0 == 12));
        assert!(points.iter().any(|p| p.1 == 3));
        assert!(points.iter().any(|p| p.1 == 9));
    }

    #[test]
    fn filled_ellipse_includes_its_centre_and_outline() {
        let filled = ellipse_filled((0, 0), (6, 6));
        assert!(filled.contains(&(3, 3)));
        for point in ellipse_outline((0, 0), (6, 6)) {
            assert!(filled.contains(&point));
        }
    }

    #[test]
    fn tiny_ellipses_degenerate_to_their_rectangle() {
        assert_eq!(sorted(ellipse_outline((0, 0), (1, 1))), vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
        assert_eq!(ellipse_outline((3, 3), (3, 3)), vec![(3, 3)]);
    }

    #[test]
    fn pixel_perfect_turns_a_jagged_staircase_into_a_clean_diagonal() {
        let staircase = vec![(0, 0), (1, 0), (1, 1), (2, 1), (2, 2)];
        assert_eq!(pixel_perfect(&staircase), vec![(0, 0), (1, 1), (2, 2)]);
    }

    #[test]
    fn pixel_perfect_keeps_straight_runs() {
        let straight = vec![(0, 0), (1, 0), (2, 0), (3, 0)];
        assert_eq!(pixel_perfect(&straight), straight);
    }

    #[test]
    fn connecting_samples_fills_gaps_without_repeating_joints() {
        let path = connect(&[(0, 0), (3, 0), (3, 2)]);
        assert_eq!(path, vec![(0, 0), (1, 0), (2, 0), (3, 0), (3, 1), (3, 2)]);
    }

    #[test]
    fn contiguous_region_stops_at_a_different_colour() {
        let mut image = Pixels::new(5, 1);
        image.set(2, 0, Rgba::BLACK);
        assert_eq!(sorted(contiguous_region(&image, (0, 0))), vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn matching_colour_jumps_over_barriers() {
        let mut image = Pixels::new(5, 1);
        image.set(2, 0, Rgba::BLACK);
        assert_eq!(matching_colour(&image, (0, 0)).len(), 4);
    }

    #[test]
    fn region_from_outside_the_image_is_empty() {
        assert!(contiguous_region(&Pixels::new(2, 2), (5, 5)).is_empty());
    }

    #[test]
    fn odd_square_brushes_centre_on_the_pixel() {
        assert_eq!(sorted(square_brush((5, 5), 3)), sorted(Rect::new(4, 4, 3, 3).points().collect()));
        assert_eq!(square_brush((5, 5), 1), vec![(5, 5)]);
    }

    #[test]
    fn even_square_brushes_lean_up_and_left() {
        assert_eq!(sorted(square_brush((5, 5), 2)), vec![(4, 4), (4, 5), (5, 4), (5, 5)]);
    }

    #[test]
    fn round_brushes_cut_the_corners() {
        let brush = round_brush((10, 10), 5);
        assert!(brush.contains(&(10, 10)));
        assert!(!brush.contains(&(8, 8)));
    }

    #[test]
    fn spray_is_deterministic_and_stays_within_its_radius() {
        let burst = spray((0, 0), 4, 30, 7);
        assert_eq!(burst, spray((0, 0), 4, 30, 7));
        assert!(burst.iter().all(|&(x, y)| x * x + y * y <= 16 + 8));
    }
}

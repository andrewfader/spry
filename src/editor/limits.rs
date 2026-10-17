//! The sizes Spry will work with.
//!
//! Commands come from people, but also from LLMs and scripts that can ask
//! for a 2-billion-pixel line or a brush wider than the canvas. Every
//! command passes `check` before it runs, so a bad request is a polite
//! error rather than a panic or a machine-wide out-of-memory kill. The
//! limits are generous for pixel art: Aseprite users rarely go past 512².

use super::command::{Brush, Command, Ink};
use super::raster::Point;

pub const MAX_CANVAS_SIDE: u32 = 4096;
pub const MAX_BRUSH_SIZE: u32 = 64;
pub const MAX_SPRAY_RADIUS: u32 = 64;
pub const MAX_SPRAY_DENSITY: u32 = 1024;
/// How far outside the canvas a shape's corner may sit. Shapes may overhang
/// so an ellipse can be cut by the edge, but not by miles.
pub const MAX_OVERHANG: i32 = 256;
/// The longest side of any exported image, scale included.
pub const MAX_EXPORT_SIDE: u64 = 16384;

pub fn check_canvas_size(width: u32, height: u32) -> Result<(), String> {
    let fits = |side: u32| (1..=MAX_CANVAS_SIDE).contains(&side);
    if fits(width) && fits(height) {
        Ok(())
    } else {
        Err(format!("canvas sizes run from 1 to {MAX_CANVAS_SIDE} pixels a side, not {width}×{height}"))
    }
}

pub fn check_export_size(width: u32, height: u32, scale: u32) -> Result<(), String> {
    let side = u64::from(width.max(height)) * u64::from(scale.max(1));
    if side <= MAX_EXPORT_SIDE {
        Ok(())
    } else {
        Err(format!("at {scale}× the image would be {side} pixels across; the most is {MAX_EXPORT_SIDE}"))
    }
}

pub fn check_brush(brush: &Brush) -> Result<(), String> {
    if (1..=MAX_BRUSH_SIZE).contains(&brush.size) {
        Ok(())
    } else {
        Err(format!("brush sizes run from 1 to {MAX_BRUSH_SIZE}, not {}", brush.size))
    }
}

pub fn check_spray(radius: u32, density: u32) -> Result<(), String> {
    if radius > MAX_SPRAY_RADIUS || density > MAX_SPRAY_DENSITY {
        return Err(format!(
            "spray radius goes up to {MAX_SPRAY_RADIUS} and density up to {MAX_SPRAY_DENSITY}, not {radius} and {density}"
        ));
    }
    Ok(())
}

/// Checks a command against the canvas it is about to change.
pub fn check(command: &Command, canvas_width: u32, canvas_height: u32) -> Result<(), String> {
    let reach = Reach { width: canvas_width as i32, height: canvas_height as i32 };
    match command {
        Command::Paint { points, ink, brush, .. } => {
            check_brush(brush)?;
            check_ink(ink)?;
            points.iter().try_for_each(|&point| reach.check(point))
        }
        Command::DrawLine { from, to, ink, brush }
        | Command::DrawRect { from, to, ink, brush, .. }
        | Command::DrawEllipse { from, to, ink, brush, .. } => {
            check_brush(brush)?;
            check_ink(ink)?;
            reach.check(*from)?;
            reach.check(*to)
        }
        Command::Fill { ink, .. } => check_ink(ink),
        Command::Spray { at, ink, radius, density, .. } => {
            check_spray(*radius, *density)?;
            check_ink(ink)?;
            reach.check(*at)
        }
        Command::MoveFloating { dx, dy } | Command::DropShadow { dx, dy, .. } => check_offset(*dx, *dy),
        Command::Paste { at: Some(at) } => reach.check(*at),
        Command::SelectLasso { points, .. } => points.iter().try_for_each(|&point| reach.check(point)),
        Command::ImportLayer { image, .. } => check_canvas_size(image.width(), image.height()),
        Command::ResizeCanvas { width, height, offset_x, offset_y } => {
            check_canvas_size(*width, *height)?;
            check_offset(*offset_x, *offset_y)
        }
        _ => Ok(()),
    }
}

fn check_ink(ink: &Ink) -> Result<(), String> {
    match ink {
        Ink::Clone { dx, dy } => check_offset(*dx, *dy),
        _ => Ok(()),
    }
}

fn check_offset(dx: i32, dy: i32) -> Result<(), String> {
    let limit = MAX_CANVAS_SIDE;
    if dx.unsigned_abs() <= limit && dy.unsigned_abs() <= limit {
        Ok(())
    } else {
        Err(format!("offsets go up to {limit} pixels each way, not ({dx}, {dy})"))
    }
}

struct Reach {
    width: i32,
    height: i32,
}

impl Reach {
    fn check(&self, (x, y): Point) -> Result<(), String> {
        let within = |value: i32, side: i32| (-MAX_OVERHANG..side + MAX_OVERHANG).contains(&value);
        if within(x, self.width) && within(y, self.height) {
            Ok(())
        } else {
            Err(format!("({x}, {y}) is too far off a {}×{} canvas", self.width, self.height))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Rgba;

    fn line_to(to: Point) -> Command {
        Command::DrawLine { from: (0, 0), to, ink: Ink::colour(Rgba::BLACK), brush: Brush::default() }
    }

    #[test]
    fn shapes_may_overhang_the_canvas_a_little_but_not_a_lot() {
        assert!(check(&line_to((40, -10)), 32, 32).is_ok());
        assert!(check(&line_to((i32::MAX, 0)), 32, 32).unwrap_err().contains("too far off"));
    }

    #[test]
    fn brushes_and_sprays_have_sane_sizes() {
        let huge = Brush { size: 100_000, ..Brush::default() };
        let paint = Command::Paint { points: vec![(1, 1)], ink: Ink::Erase, brush: huge, pixel_perfect: false };
        assert!(check(&paint, 8, 8).unwrap_err().contains("brush sizes"));
        let spray = Command::Spray { at: (1, 1), ink: Ink::Erase, radius: 4, density: u32::MAX, seed: 0 };
        assert!(check(&spray, 8, 8).unwrap_err().contains("density"));
    }

    #[test]
    fn offsets_cannot_overflow_coordinates() {
        assert!(check(&Command::MoveFloating { dx: i32::MAX, dy: 0 }, 8, 8).is_err());
        let clone = Command::Fill { at: (0, 0), ink: Ink::Clone { dx: i32::MIN, dy: 0 }, contiguous: true };
        assert!(check(&clone, 8, 8).is_err());
    }

    #[test]
    fn exports_stop_short_of_absurd_sizes() {
        assert!(check_export_size(4096, 4096, 4).is_ok());
        assert!(check_export_size(4096, 64, 32).unwrap_err().contains("131072"));
        assert!(check_export_size(u32::MAX, 1, u32::MAX).is_err());
    }

    #[test]
    fn canvases_run_from_one_pixel_to_the_maximum() {
        assert!(check_canvas_size(1, MAX_CANVAS_SIDE).is_ok());
        assert!(check_canvas_size(0, 8).is_err());
        assert!(check_canvas_size(65536, 65536).is_err());
    }
}

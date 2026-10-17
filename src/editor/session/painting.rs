//! Painting: brushes and inks, shapes, fills and the effects that redraw
//! a cel.

use super::*;
use crate::editor::dither::gradient_is_primary;
use crate::editor::effects;
use crate::editor::pixelate::quantise;

impl Editor {
    pub(super) fn paint(&mut self, points: &[Point], ink: Ink, brush: Brush, pixel_perfect: bool) -> EditResult {
        self.ensure_drawable()?;
        let connected = raster::connect(points);
        let path = if pixel_perfect { raster::pixel_perfect(&connected) } else { connected };
        self.draw(&path, ink, brush, "painted")
    }

    pub(super) fn spray(&mut self, at: Point, ink: Ink, radius: u32, density: u32, seed: u64) -> EditResult {
        self.draw(&raster::spray(at, radius, density, seed), ink, Brush::default(), "sprayed")
    }

    pub(super) fn replace_colour(&mut self, from: Rgba, to: Rgba) -> EditResult {
        let count = self.document.sprite.swap_colour(from, to);
        Ok(format!("replaced {count} pixels of {from} with {to}"))
    }

    pub(super) fn outline(&mut self, colour: Rgba, diagonals: bool) -> EditResult {
        self.transform_cel(|image| effects::outline(image, colour, diagonals))?;
        Ok("outlined the layer".into())
    }

    pub(super) fn drop_shadow(&mut self, colour: Rgba, dx: i32, dy: i32) -> EditResult {
        self.transform_cel(|image| effects::drop_shadow(image, colour, dx, dy))?;
        Ok("added a drop shadow".into())
    }

    pub(super) fn snap_to_palette(&mut self, dither: bool) -> EditResult {
        let palette = self.sprite().palette.clone();
        self.transform_cel(|image| quantise(image, &palette, dither))?;
        Ok(format!("snapped the cel to {}", palette.name))
    }

    pub fn footprint(&self, path: &[Point], brush: Brush) -> Vec<Point> {
        let (w, h) = (self.sprite().width as i32, self.sprite().height as i32);
        let mut points = Vec::with_capacity(path.len() * (brush.size * brush.size) as usize);
        for &point in path {
            let stamp = if brush.round {
                raster::round_brush(point, brush.size)
            } else {
                raster::square_brush(point, brush.size)
            };
            for (x, y) in stamp {
                points.push((x, y));
                if brush.mirror_x {
                    points.push((w - 1 - x, y));
                }
                if brush.mirror_y {
                    points.push((x, h - 1 - y));
                }
                if brush.mirror_x && brush.mirror_y {
                    points.push((w - 1 - x, h - 1 - y));
                }
            }
        }
        raster::deduplicated(points)
    }

    /// What a line, rectangle or ellipse command would paint on the
    /// current cel, without painting it: each pixel it would change, with
    /// its new colour. Shape tools preview with this, so the preview runs
    /// the same code as the result and can never disagree with it, whether
    /// the ink is a pattern, shade or clone, or there's a selection.
    pub fn preview(&self, command: &Command) -> Vec<(Point, Rgba)> {
        let (Some(path), Some((ink, brush))) = (shape_path(command), shape_ink(command)) else {
            return Vec::new();
        };
        let points = raster::deduplicated(self.footprint(&path, brush));
        let before = self.current_image_or_blank();
        let mut after = before.clone();
        self.paint_points(&mut after, &before, &points, ink);
        points
            .into_iter()
            .filter(|&(x, y)| after.get(x, y) != before.get(x, y))
            .map(|p| (p, after.get(p.0, p.1)))
            .collect()
    }

    pub(super) fn draw(&mut self, path: &[Point], ink: Ink, brush: Brush, summary: &str) -> EditResult {
        self.ensure_drawable()?;
        let points = self.footprint(path, brush);
        let before = self.current_image_or_blank();
        let mut image = before.clone();
        self.paint_points(&mut image, &before, &points, ink);
        self.write_current_cel(image);
        Ok(summary.to_string())
    }

    /// Applies `ink` to each point, reading the "before" image for inks that
    /// depend on existing pixels (shade, clone). Points outside the
    /// selection, when there is one, are skipped.
    pub(super) fn paint_points(&self, image: &mut Pixels, before: &Pixels, points: &[Point], ink: Ink) {
        let palette = &self.sprite().palette;
        for &(x, y) in points {
            if !image.contains(x, y) || self.selection.as_ref().is_some_and(|s| !s.contains(x, y)) {
                continue;
            }
            let existing = before.get(x, y);
            let painted = match ink {
                Ink::Colour { colour } => Some(colour.over(existing, 255)),
                Ink::Erase => Some(Rgba::TRANSPARENT),
                Ink::Pattern { pattern, primary, secondary } => {
                    if pattern.is_on(x, y) {
                        Some(primary)
                    } else {
                        secondary
                    }
                }
                Ink::Shade { steps } => (!existing.is_transparent()).then(|| palette.shade(existing, steps)),
                Ink::Clone { dx, dy } => Some(before.get(x + dx, y + dy)),
            };
            if let Some(colour) = painted {
                image.set(x, y, colour);
            }
        }
    }

    pub(super) fn set_pixels(&mut self, pixels: &[PixelValue]) -> EditResult {
        self.ensure_drawable()?;
        let mut written = 0;
        self.edit_current_cel(|image| {
            for pixel in pixels {
                if image.contains(pixel.x, pixel.y) {
                    image.set(pixel.x, pixel.y, pixel.colour);
                    written += 1;
                }
            }
        });
        Ok(format!("set {written} pixels"))
    }

    pub(super) fn fill(&mut self, at: Point, ink: Ink, contiguous: bool) -> EditResult {
        self.ensure_drawable()?;
        let image = self.current_image_or_blank();
        if !image.contains(at.0, at.1) {
            return Err(EditError::Invalid(format!("({}, {}) is outside the canvas", at.0, at.1)));
        }
        let region =
            if contiguous { raster::contiguous_region(&image, at) } else { raster::matching_colour(&image, at) };
        let count = region.len();
        let mut painted = image.clone();
        self.paint_points(&mut painted, &image, &region, ink);
        self.write_current_cel(painted);
        Ok(format!("filled {count} pixels"))
    }

    pub(super) fn gradient_fill(&mut self, from: Point, to: Point, primary: Rgba, secondary: Rgba) -> EditResult {
        self.ensure_drawable()?;
        let image = self.current_image_or_blank();
        let region: Vec<Point> = match &self.selection {
            Some(selection) => selection.points().collect(),
            None => raster::contiguous_region(&image, from),
        };
        let (gx, gy) = ((to.0 - from.0) as f32, (to.1 - from.1) as f32);
        let length_squared = (gx * gx + gy * gy).max(1.0);
        let mut painted = image;
        for &(x, y) in &region {
            let t = (((x - from.0) as f32 * gx) + ((y - from.1) as f32 * gy)) / length_squared;
            painted.set(x, y, if gradient_is_primary(x, y, t) { primary } else { secondary });
        }
        self.write_current_cel(painted);
        Ok(format!("gradient-filled {} pixels", region.len()))
    }

    /// Replaces the current cel with a transformed copy, limited to the
    /// selection when there is one.
    pub(super) fn transform_cel(&mut self, transform: impl Fn(&Pixels) -> Pixels) -> Result<(), EditError> {
        self.ensure_drawable()?;
        let before = self.current_image_or_blank();
        let after = transform(&before);
        let merged = match &self.selection {
            Some(selection) => {
                let mut merged = before.clone();
                for (x, y) in selection.points() {
                    merged.set(x, y, after.get(x, y));
                }
                merged
            }
            None => after,
        };
        self.write_current_cel(merged);
        Ok(())
    }

    pub(super) fn import_layer(&mut self, name: &str, image: Pixels) -> EditResult {
        let (w, h) = (self.sprite().width, self.sprite().height);
        let fitted = if (image.width(), image.height()) == (w, h) { image } else { image.resized_canvas(w, h, 0, 0) };
        let id = self.document.sprite.insert_layer(self.layer + 1, name);
        self.layer = self.sprite().layer_index(id).unwrap_or(self.layer);
        self.write_current_cel(fitted);
        Ok(format!("added layer \"{name}\""))
    }
}

/// The path a shape command traces, for previews.
fn shape_path(command: &Command) -> Option<Vec<Point>> {
    match *command {
        Command::DrawLine { from, to, .. } => Some(raster::line(from, to)),
        Command::DrawRect { from, to, filled, .. } => Some(rectangle_path(from, to, filled)),
        Command::DrawEllipse { from, to, filled, .. } => Some(ellipse_path(from, to, filled)),
        _ => None,
    }
}

pub(super) fn rectangle_path(from: Point, to: Point, filled: bool) -> Vec<Point> {
    if filled {
        raster::rectangle_filled(from, to)
    } else {
        raster::rectangle_outline(from, to)
    }
}

pub(super) fn ellipse_path(from: Point, to: Point, filled: bool) -> Vec<Point> {
    if filled {
        raster::ellipse_filled(from, to)
    } else {
        raster::ellipse_outline(from, to)
    }
}

fn shape_ink(command: &Command) -> Option<(Ink, Brush)> {
    match *command {
        Command::DrawLine { ink, brush, .. }
        | Command::DrawRect { ink, brush, .. }
        | Command::DrawEllipse { ink, brush, .. } => Some((ink, brush)),
        _ => None,
    }
}

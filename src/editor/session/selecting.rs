//! Selecting, the clipboard, and floating pixels: lifting, moving, turning
//! and anchoring them.

use super::*;
use crate::editor::rotsprite::{rotsprite, MAX_ROTSPRITE_SIDE};

impl Editor {
    pub(super) fn select_rect(&mut self, rect: Rect, add: bool) -> EditResult {
        let picked = Selection::from_rect(self.sprite().width, self.sprite().height, rect);
        self.select(picked, add)
    }

    pub(super) fn select_lasso(&mut self, outline: &[Point], add: bool) -> EditResult {
        let picked = Selection::from_polygon(self.sprite().width, self.sprite().height, outline);
        self.select(picked, add)
    }

    pub(super) fn select_wand(&mut self, at: Point, contiguous: bool, add: bool) -> EditResult {
        let image = self.current_image_or_blank();
        let points =
            if contiguous { raster::contiguous_region(&image, at) } else { raster::matching_colour(&image, at) };
        let picked = Selection::from_points(self.sprite().width, self.sprite().height, points);
        self.select(picked, add)
    }

    pub(super) fn select_all(&mut self) -> EditResult {
        self.anchor_floating();
        self.selection = Some(Selection::all(self.sprite().width, self.sprite().height));
        Ok("selected everything".into())
    }

    pub(super) fn select_none(&mut self) -> EditResult {
        self.anchor_floating();
        self.selection = None;
        Ok("deselected".into())
    }

    pub(super) fn invert_selection(&mut self) -> EditResult {
        self.anchor_floating();
        let (w, h) = (self.sprite().width, self.sprite().height);
        self.selection = Some(self.selection.as_ref().map_or_else(|| Selection::all(w, h), Selection::inverted));
        Ok("inverted the selection".into())
    }

    pub(super) fn cut(&mut self) -> EditResult {
        self.copy()?;
        self.delete_selection()
    }

    pub(super) fn move_floating(&mut self, dx: i32, dy: i32) -> EditResult {
        if self.floating.is_none() {
            self.lift_selection()?;
        }
        let floating = self.floating.as_mut().ok_or(EditError::NothingSelected)?;
        floating.x += dx;
        floating.y += dy;
        Ok(format!("moved the selection by ({dx}, {dy})"))
    }

    pub(super) fn anchor(&mut self) -> EditResult {
        if self.anchor_floating() {
            Ok("anchored".into())
        } else {
            Err(EditError::NothingSelected)
        }
    }

    pub(super) fn select(&mut self, picked: Selection, add: bool) -> EditResult {
        self.anchor_floating();
        let combined = match (&self.selection, add) {
            (Some(existing), true) => existing.union(&picked),
            _ => picked,
        };
        let count = combined.points().count();
        self.selection = (!combined.is_empty()).then_some(combined);
        Ok(format!("selected {count} pixels"))
    }

    pub(super) fn copy(&mut self) -> EditResult {
        let source = match &self.floating {
            Some(floating) => floating.pixels.clone(),
            None => {
                let image = self.current_image_or_blank();
                match &self.selection {
                    Some(selection) => selection.extract(&image).ok_or(EditError::NothingSelected)?.0,
                    None => image,
                }
            }
        };
        let summary = format!("copied {}×{}", source.width(), source.height());
        self.clipboard = Some(source);
        Ok(summary)
    }

    pub fn clipboard(&self) -> Option<&Pixels> {
        self.clipboard.as_ref()
    }

    pub(super) fn delete_selection(&mut self) -> EditResult {
        self.ensure_drawable()?;
        if self.floating.take().is_some() {
            return Ok("deleted the floating pixels".into());
        }
        let selection = self.selection.clone().ok_or(EditError::NothingSelected)?;
        let image = self.document.sprite.cel_image_mut(self.layer, self.frame);
        for (x, y) in selection.points() {
            image.set(x, y, Rgba::TRANSPARENT);
        }
        Ok("cleared the selection".into())
    }

    pub(super) fn paste(&mut self, at: Option<Point>) -> EditResult {
        self.ensure_drawable()?;
        let pixels = self.clipboard.clone().ok_or_else(|| EditError::Invalid("the clipboard is empty".into()))?;
        self.anchor_floating();
        let origin =
            at.or_else(|| self.selection.as_ref().and_then(Selection::bounds).map(|b| (b.x, b.y))).unwrap_or((0, 0));
        let summary = format!("pasted {}×{} at ({}, {})", pixels.width(), pixels.height(), origin.0, origin.1);
        self.floating = Some(Floating { pixels, x: origin.0, y: origin.1 });
        self.selection = None;
        Ok(summary)
    }

    /// Lifts the selected pixels off the cel into a floating piece, leaving
    /// a transparent hole, the way a move tool picks art up.
    pub(super) fn lift_selection(&mut self) -> EditResult {
        self.ensure_drawable()?;
        if self.floating.is_some() {
            return Ok("already floating".into());
        }
        let selection = self.selection.clone().ok_or(EditError::NothingSelected)?;
        let image = self.current_image_or_blank();
        let (pixels, bounds) = selection.extract(&image).ok_or(EditError::NothingSelected)?;
        let cel = self.document.sprite.cel_image_mut(self.layer, self.frame);
        for (x, y) in selection.points() {
            cel.set(x, y, Rgba::TRANSPARENT);
        }
        self.floating = Some(Floating { pixels, x: bounds.x, y: bounds.y });
        self.selection = None;
        Ok("lifted the selection".into())
    }

    /// Stamps floating pixels down onto the current cel. The anchored area
    /// becomes the new selection, so you can keep working on it. Returns
    /// whether anything was floating.
    pub fn anchor_floating(&mut self) -> bool {
        let Some(floating) = self.floating.take() else { return false };
        let opacity = 255;
        self.document.sprite.cel_image_mut(self.layer, self.frame).stamp(
            &floating.pixels,
            floating.x,
            floating.y,
            opacity,
        );
        let landed = floating.bounds().intersect(&self.sprite().bounds());
        self.selection =
            (!landed.is_empty()).then(|| Selection::from_rect(self.sprite().width, self.sprite().height, landed));
        true
    }

    /// Flips or rotates the floating pixels, or the selection (lifting it
    /// first), or else the whole cel about its centre.
    pub(super) fn rotate_by(&mut self, degrees: f32) -> EditResult {
        if !degrees.is_finite() {
            return Err(EditError::Invalid("an angle has to be a number of degrees".into()));
        }
        let piece = self
            .floating
            .as_ref()
            .map(|floating| (floating.pixels.width(), floating.pixels.height()))
            .or_else(|| self.selection.as_ref().and_then(Selection::bounds).map(|b| (b.width, b.height)))
            .unwrap_or((self.sprite().width, self.sprite().height));
        if piece.0.max(piece.1) > MAX_ROTSPRITE_SIDE {
            return Err(EditError::Invalid(format!(
                "Rotsprite turns pieces up to {MAX_ROTSPRITE_SIDE} pixels across; select a smaller area"
            )));
        }
        self.reorient(move |image| rotsprite(image, degrees), &format!("rotated {degrees}°"))
    }

    pub(super) fn reorient(&mut self, turn: impl Fn(&Pixels) -> Pixels, summary: &str) -> EditResult {
        self.ensure_drawable()?;
        if self.floating.is_none() && self.selection.is_some() {
            self.lift_selection()?;
        }
        if let Some(floating) = self.floating.as_mut() {
            let centre =
                (floating.x * 2 + floating.pixels.width() as i32, floating.y * 2 + floating.pixels.height() as i32);
            floating.pixels = turn(&floating.pixels);
            floating.x = (centre.0 - floating.pixels.width() as i32) / 2;
            floating.y = (centre.1 - floating.pixels.height() as i32) / 2;
            return Ok(summary.to_string());
        }
        let image = self.current_image_or_blank();
        let turned = turn(&image);
        let offset_x = (image.width() as i32 - turned.width() as i32) / 2;
        let offset_y = (image.height() as i32 - turned.height() as i32) / 2;
        *self.document.sprite.cel_image_mut(self.layer, self.frame) =
            turned.resized_canvas(image.width(), image.height(), offset_x, offset_y);
        Ok(summary.to_string())
    }
}

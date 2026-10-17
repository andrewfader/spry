//! Edits to the sprite as a whole: palette, canvas size, notes, pins,
//! slices and versions.

use super::*;
use crate::editor::palette::Palette;
use crate::editor::sprite::{Pin, Slice};

impl Editor {
    pub(super) fn set_palette(&mut self, palette: Palette) -> EditResult {
        let summary = format!("using palette {} ({} colours)", palette.name, palette.colours.len());
        self.document.sprite.palette = palette;
        Ok(summary)
    }

    pub(super) fn use_built_in_palette(&mut self, name: &str) -> EditResult {
        let palette = Palette::built_in_named(name).ok_or_else(|| {
            let known: Vec<String> = Palette::built_ins().into_iter().map(|p| p.name).collect();
            EditError::NotFound(format!("palette \"{name}\" (built-ins are {})", known.join(", ")))
        })?;
        self.set_palette(palette)
    }

    pub(super) fn add_palette_colour(&mut self, colour: Rgba) -> EditResult {
        if self.sprite().palette.colours.contains(&colour) {
            return Ok(format!("{colour} is already in the palette"));
        }
        self.document.sprite.palette.colours.push(colour);
        Ok(format!("added {colour} to the palette"))
    }

    pub(super) fn set_palette_colour(&mut self, index: usize, colour: Rgba) -> EditResult {
        let slot = self
            .document
            .sprite
            .palette
            .colours
            .get_mut(index)
            .ok_or_else(|| EditError::NotFound(format!("palette entry {index}")))?;
        *slot = colour;
        Ok(format!("palette entry {index} is now {colour}"))
    }

    pub(super) fn remove_palette_colour(&mut self, index: usize) -> EditResult {
        if index >= self.sprite().palette.colours.len() {
            return Err(EditError::NotFound(format!("palette entry {index}")));
        }
        let removed = self.document.sprite.palette.colours.remove(index);
        Ok(format!("removed {removed} from the palette"))
    }

    pub(super) fn extract_palette(&mut self) -> EditResult {
        let colours = self.sprite().used_colours();
        if colours.is_empty() {
            return Err(EditError::Invalid("the sprite has no colours to extract yet".into()));
        }
        let count = colours.len();
        self.document.sprite.palette = Palette { name: "Extracted".into(), colours };
        Ok(format!("extracted {count} colours"))
    }

    pub(super) fn resize_canvas(&mut self, width: u32, height: u32, offset_x: i32, offset_y: i32) -> EditResult {
        self.anchor_floating();
        self.selection = None;
        self.document.sprite.resize_canvas(width, height, offset_x, offset_y);
        Ok(format!("canvas is now {width}×{height}"))
    }

    pub(super) fn set_notes(&mut self, text: String) -> EditResult {
        self.document.sprite.notes = text;
        Ok("updated the notes".into())
    }

    pub(super) fn set_frame_note(&mut self, frame: usize, text: String) -> EditResult {
        let frame = self.check_frame_index(frame)?;
        self.document.sprite.frames[frame].note = text;
        Ok(format!("updated the note on frame {}", frame + 1))
    }

    pub(super) fn add_pin(&mut self, x: i32, y: i32, text: String) -> EditResult {
        self.document.sprite.pins.push(Pin { x, y, text });
        Ok(format!("pinned a note at ({x}, {y})"))
    }

    pub(super) fn remove_pin(&mut self, index: usize) -> EditResult {
        if index >= self.sprite().pins.len() {
            return Err(EditError::NotFound(format!("pin {index}")));
        }
        self.document.sprite.pins.remove(index);
        Ok("removed the pin".into())
    }

    /// Adds a slice, replacing any slice of the same name.
    pub(super) fn add_slice(&mut self, name: String, bounds: Rect) -> EditResult {
        if bounds.is_empty() {
            return Err(EditError::Invalid("slices need a width and height".into()));
        }
        self.document.sprite.slices.retain(|s| s.name != name);
        self.document.sprite.slices.push(Slice { name: name.clone(), bounds });
        Ok(format!("added slice \"{name}\""))
    }

    pub(super) fn remove_slice(&mut self, name: &str) -> EditResult {
        let before = self.sprite().slices.len();
        self.document.sprite.slices.retain(|s| s.name != name);
        if before == self.sprite().slices.len() {
            return Err(EditError::NotFound(format!("slice \"{name}\"")));
        }
        Ok(format!("removed slice \"{name}\""))
    }

    pub(super) fn commit_version(&mut self, message: &str) -> EditResult {
        self.anchor_floating();
        let now = (self.clock)();
        match self.document.commit_version(message, now) {
            Some(id) => {
                Ok(format!("saved version {id}: {}", self.document.versions.last().map_or("", |v| v.message.as_str())))
            }
            None => Ok("nothing changed since the last version".into()),
        }
    }

    pub(super) fn restore_version(&mut self, id: u32) -> EditResult {
        let version = self.document.version(id).ok_or_else(|| EditError::NotFound(format!("version {id}")))?;
        let (sprite, message) = (version.sprite.clone(), version.message.clone());
        self.document.sprite = sprite;
        self.selection = None;
        self.floating = None;
        self.layer = self.layer.min(self.sprite().layers.len() - 1);
        self.frame = self.frame.min(self.sprite().frame_count() - 1);
        Ok(format!("restored version {id}: {message}"))
    }
}

//! Frames, cels and clones, and tags.

use super::*;
use crate::editor::sprite::{LoopDirection, Tag};

impl Editor {
    pub(super) fn add_frame(&mut self) -> EditResult {
        let duration = self.sprite().frames[self.frame].duration_ms;
        self.document.sprite.insert_frame(self.frame + 1, duration);
        self.frame += 1;
        Ok(format!("added frame {}", self.frame + 1))
    }

    pub(super) fn duplicate_current_frame(&mut self, as_clone: bool) -> EditResult {
        self.frame = self.document.sprite.duplicate_frame(self.frame, as_clone);
        Ok(if as_clone { "cloned the frame (linked cels)" } else { "duplicated the frame" }.into())
    }

    pub(super) fn remove_current_frame(&mut self) -> EditResult {
        if !self.document.sprite.remove_frame(self.frame) {
            return Err(EditError::Invalid("a sprite needs at least one frame".into()));
        }
        self.frame = self.frame.min(self.sprite().frame_count() - 1);
        Ok("removed the frame".into())
    }

    pub(super) fn move_current_frame(&mut self, to: usize) -> EditResult {
        let to = self.check_frame_index(to)?;
        self.document.sprite.move_frame(self.frame, to);
        self.frame = to;
        Ok(format!("moved the frame to position {}", to + 1))
    }

    pub(super) fn set_frame_duration(&mut self, duration_ms: u32, all_frames: bool) -> EditResult {
        if duration_ms == 0 {
            return Err(EditError::Invalid("frames need a duration of at least 1 ms".into()));
        }
        let frame = self.frame;
        for (index, f) in self.document.sprite.frames.iter_mut().enumerate() {
            if all_frames || index == frame {
                f.duration_ms = duration_ms;
            }
        }
        Ok(format!("set duration to {duration_ms} ms"))
    }

    pub(super) fn link_cel(&mut self, to_frame: usize) -> EditResult {
        let to_frame = self.check_frame_index(to_frame)?;
        if self.sprite().cel_image(LayerIndex(self.layer), FrameIndex(self.frame)).is_none() {
            return Err(EditError::Invalid("an empty cel has nothing to link".into()));
        }
        self.document.sprite.link_cel(LayerIndex(self.layer), FrameIndex(self.frame), FrameIndex(to_frame));
        Ok(format!("linked frame {} to frame {}", to_frame + 1, self.frame + 1))
    }

    pub(super) fn unlink_cel(&mut self) -> EditResult {
        if !self.document.sprite.unlink_cel(LayerIndex(self.layer), FrameIndex(self.frame)) {
            return Err(EditError::Invalid("this cel isn't linked".into()));
        }
        Ok("unlinked the cel".into())
    }

    pub(super) fn clear_cel(&mut self) -> EditResult {
        self.ensure_drawable()?;
        self.document.sprite.clear_cel(LayerIndex(self.layer), FrameIndex(self.frame));
        Ok("cleared the cel".into())
    }

    /// Adds a tag, replacing any tag of the same name.
    pub(super) fn add_tag(&mut self, name: String, from: usize, to: usize, direction: LoopDirection) -> EditResult {
        let (from, to) = (self.check_frame_index(from)?, self.check_frame_index(to)?);
        if name.trim().is_empty() {
            return Err(EditError::Invalid("tags need a name".into()));
        }
        let (from, to) = (from.min(to), from.max(to));
        self.document.sprite.tags.retain(|t| t.name != name);
        self.document.sprite.tags.push(Tag { name: name.clone(), from, to, direction });
        Ok(format!("tagged frames {}–{} as \"{name}\"", from + 1, to + 1))
    }

    pub(super) fn remove_tag(&mut self, name: &str) -> EditResult {
        let before = self.sprite().tags.len();
        self.document.sprite.tags.retain(|t| t.name != name);
        if self.sprite().tags.len() == before {
            return Err(EditError::NotFound(format!("tag \"{name}\"")));
        }
        Ok(format!("removed tag \"{name}\""))
    }
}

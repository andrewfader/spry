//! The document: a sprite plus its version history. This is what a `.spry`
//! file holds.
//!
//! Versions are the long-term memory that undo isn't: named snapshots you
//! choose to keep, saved inside the file, that you can browse, compare and
//! restore weeks later. Sprites share unchanged images through `Arc`, so a
//! hundred versions of a sprite cost little more than one.

use super::color::Rgba;
use super::pixels::Pixels;
use super::raster::Point;
use super::sprite::Sprite;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub id: u32,
    pub message: String,
    /// Seconds since the Unix epoch.
    pub created_at: u64,
    pub sprite: Sprite,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub sprite: Sprite,
    pub versions: Vec<Version>,
    next_version_id: u32,
}

impl Document {
    /// The current sprite and every version must be sound. See
    /// `Sprite::validate`.
    pub fn validate(&self) -> Result<(), String> {
        self.sprite.validate()?;
        for version in &self.versions {
            version.sprite.validate().map_err(|why| format!("version \"{}\": {why}", version.message))?;
        }
        if self.versions.iter().any(|v| v.id >= self.next_version_id) {
            return Err("a version has an id from the future".into());
        }
        Ok(())
    }

    pub fn new(sprite: Sprite) -> Document {
        Document { sprite, versions: Vec::new(), next_version_id: 1 }
    }

    /// Keeps the current sprite as a named version, unless it's identical to
    /// the latest one. Returns the new version's id.
    pub fn commit_version(&mut self, message: &str, created_at: u64) -> Option<u32> {
        if self.versions.last().is_some_and(|latest| latest.sprite == self.sprite) {
            return None;
        }
        let id = self.next_version_id;
        self.next_version_id += 1;
        let message = if message.trim().is_empty() { format!("Version {id}") } else { message.trim().to_string() };
        self.versions.push(Version { id, message, created_at, sprite: self.sprite.clone() });
        Some(id)
    }

    pub fn version(&self, id: u32) -> Option<&Version> {
        self.versions.iter().find(|v| v.id == id)
    }

    /// A new document that starts from version `id`, with the history up
    /// to and including it, so the branch remembers where it came from.
    pub fn branch(&self, id: u32) -> Option<Document> {
        let position = self.versions.iter().position(|v| v.id == id)?;
        let versions = self.versions[..=position].to_vec();
        Some(Document { sprite: versions[position].sprite.clone(), versions, next_version_id: id + 1 })
    }
}

/// A picture of what changed from `before` to `after`: changed pixels show
/// their new colour, pixels that were erased show in magenta, and
/// everything that stayed the same is a faint grey, so the eye goes
/// straight to the edits.
pub fn difference_overlay(before: &Pixels, after: &Pixels) -> Pixels {
    const UNCHANGED: Rgba = Rgba::rgba(128, 128, 128, 40);
    const ERASED: Rgba = Rgba::rgb(255, 0, 255);
    let mut overlay = Pixels::filled(before.width().max(after.width()), before.height().max(after.height()), UNCHANGED);
    for (x, y) in changed_pixels(before, after) {
        let now = after.get(x, y);
        overlay.set(x, y, if now.is_transparent() { ERASED } else { now.with_alpha(255) });
    }
    overlay
}

/// The pixels that differ between two frames, for the visual version diff.
pub fn changed_pixels(before: &Pixels, after: &Pixels) -> Vec<Point> {
    let width = before.width().max(after.width()) as i32;
    let height = before.height().max(after.height()) as i32;
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .filter(|&(x, y)| before.get(x, y) != after.get(x, y))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::color::Rgba;
    use crate::editor::{FrameIndex, LayerIndex};

    #[test]
    fn identical_content_does_not_make_a_new_version() {
        let mut document = Document::new(Sprite::new(4, 4));
        assert_eq!(document.commit_version("base", 1), Some(1));
        assert_eq!(document.commit_version("again", 2), None);
        document.sprite.cel_image_mut(LayerIndex(0), FrameIndex(0)).set(0, 0, Rgba::BLACK);
        assert_eq!(document.commit_version("ink", 3), Some(2));
    }

    #[test]
    fn versions_without_a_message_get_a_numbered_name() {
        let mut document = Document::new(Sprite::new(4, 4));
        document.commit_version("  ", 1);
        assert_eq!(document.versions[0].message, "Version 1");
    }

    #[test]
    fn changed_pixels_lists_exactly_the_differences() {
        let before = Pixels::new(3, 3);
        let mut after = before.clone();
        after.set(2, 1, Rgba::BLACK);
        assert_eq!(changed_pixels(&before, &after), vec![(2, 1)]);
    }
}

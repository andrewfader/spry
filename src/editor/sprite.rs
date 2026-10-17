//! The sprite: everything the artist makes, and nothing about how the
//! editor is being used right now.
//!
//! A sprite is a canvas size, a palette, a stack of layers, a strip of
//! frames, and the cels where they meet. Cels don't own pixels. They point
//! at an image in a shared pool, so a *clone* (a linked cel) is just two
//! cels pointing at the same image, and painting one paints both.
//!
//! Layers can be grouped. A group holds no pixels of its own. Its
//! children are composited together first, and the result is laid down
//! with the group's opacity and blend mode, the way Aseprite does it. In
//! the flat `layers` list a group's children always sit directly below it
//! (see `groups.rs`).
//!
//! Images are held in `Arc`s, which makes a whole sprite cheap to copy.
//! Undo history and version snapshots are plain clones of the sprite, and an
//! image is only actually copied when someone paints on it
//! (`Arc::make_mut`).

use super::blend::BlendMode;
use super::color::Rgba;
use super::palette::Palette;
use super::pixels::{Pixels, Rect};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ImageId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LayerId(pub u32);

/// Where a layer meets a frame: which pooled image it shows, and how
/// strongly. Linked cels (clones) share an image but each keeps its own
/// opacity, as in Aseprite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(from = "CelOnDisk")]
pub struct Cel {
    pub image: ImageId,
    pub opacity: u8,
}

impl Cel {
    pub fn showing(image: ImageId) -> Cel {
        Cel { image, opacity: 255 }
    }
}

/// Files from before cel opacity stored a cel as its bare image id.
#[derive(Deserialize)]
#[serde(untagged)]
enum CelOnDisk {
    BareImage(ImageId),
    Full { image: ImageId, opacity: u8 },
}

impl From<CelOnDisk> for Cel {
    fn from(cel: CelOnDisk) -> Cel {
        match cel {
            CelOnDisk::BareImage(image) => Cel::showing(image),
            CelOnDisk::Full { image, opacity } => Cel { image, opacity },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    #[default]
    Image,
    /// Holds other layers (those whose `parent` is this one) and no pixels.
    Group,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: u8,
    #[serde(default)]
    pub blend: BlendMode,
    #[serde(default)]
    pub kind: LayerKind,
    /// The group this layer sits in, or `None` at the top level.
    #[serde(default)]
    pub parent: Option<LayerId>,
    /// One entry per frame. `None` is an empty cel. A group's cels are
    /// always empty.
    pub cels: Vec<Option<Cel>>,
}

impl Layer {
    pub fn is_group(&self) -> bool {
        self.kind == LayerKind::Group
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub duration_ms: u32,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopDirection {
    Forward,
    Reverse,
    PingPong,
}

/// A named run of frames (inclusive, zero-based) played as one animation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub from: usize,
    pub to: usize,
    pub direction: LoopDirection,
}

/// A note pinned to a canvas pixel: art direction, feedback, reminders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    pub x: i32,
    pub y: i32,
    pub text: String,
}

/// A named region exported with the sheet (hitboxes, pivots, 9-slices).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slice {
    pub name: String,
    pub bounds: Rect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sprite {
    pub width: u32,
    pub height: u32,
    pub palette: Palette,
    pub layers: Vec<Layer>,
    pub frames: Vec<Frame>,
    pub images: BTreeMap<ImageId, Arc<Pixels>>,
    pub tags: Vec<Tag>,
    pub slices: Vec<Slice>,
    pub notes: String,
    pub pins: Vec<Pin>,
    next_image_id: u32,
    next_layer_id: u32,
}

pub const DEFAULT_FRAME_MS: u32 = 100;

/// Two 0–255 opacities applied one after the other.
pub fn mul_opacity(a: u8, b: u8) -> u8 {
    ((u16::from(a) * u16::from(b) + 127) / 255) as u8
}

impl Sprite {
    /// A blank sprite: one empty layer called "Layer 1", one 100 ms frame,
    /// and the PICO-8 palette.
    pub fn new(width: u32, height: u32) -> Sprite {
        let mut sprite = Sprite {
            width,
            height,
            palette: Palette::pico8(),
            layers: Vec::new(),
            frames: vec![Frame { duration_ms: DEFAULT_FRAME_MS, note: String::new() }],
            images: BTreeMap::new(),
            tags: Vec::new(),
            slices: Vec::new(),
            notes: String::new(),
            pins: Vec::new(),
            next_image_id: 1,
            next_layer_id: 1,
        };
        sprite.insert_layer(0, "Layer 1");
        sprite
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Checks the promises the rest of the editor relies on: every layer
    /// has one cel per frame, every cel points at a pooled image the size
    /// of the canvas, tags stay inside the timeline, and new ids won't
    /// collide with old ones. A sprite read from disk must pass this
    /// before anything indexes into it.
    pub fn validate(&self) -> Result<(), String> {
        super::limits::check_canvas_size(self.width, self.height)?;
        if self.layers.is_empty() || self.frames.is_empty() {
            return Err("a sprite needs at least one layer and one frame".into());
        }
        for layer in &self.layers {
            if layer.cels.len() != self.frames.len() {
                return Err(format!(
                    "layer \"{}\" has {} cels for {} frames",
                    layer.name,
                    layer.cels.len(),
                    self.frames.len()
                ));
            }
            if layer.id.0 >= self.next_layer_id {
                return Err(format!("layer \"{}\" has an id from the future", layer.name));
            }
            if let Some(missing) = layer.cels.iter().flatten().find(|cel| !self.images.contains_key(&cel.image)) {
                return Err(format!("layer \"{}\" points at missing image {}", layer.name, missing.image.0));
            }
            if layer.is_group() && layer.cels.iter().any(Option::is_some) {
                return Err(format!("group \"{}\" has pixels of its own", layer.name));
            }
        }
        for (id, image) in &self.images {
            if (image.width(), image.height()) != (self.width, self.height) {
                return Err(format!(
                    "image {} is {}×{} on a {}×{} canvas",
                    id.0,
                    image.width(),
                    image.height(),
                    self.width,
                    self.height
                ));
            }
            if id.0 >= self.next_image_id {
                return Err(format!("image {} has an id from the future", id.0));
            }
        }
        if let Some(tag) = self.tags.iter().find(|t| t.from > t.to || t.to >= self.frames.len()) {
            return Err(format!("tag \"{}\" runs past the last frame", tag.name));
        }
        self.validate_groups()
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn layer_index(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|layer| layer.id == id)
    }

    // ----- layers -------------------------------------------------------

    /// Inserts an empty layer at `index` (0 is the bottom of the stack).
    pub fn insert_layer(&mut self, index: usize, name: &str) -> LayerId {
        let id = LayerId(self.next_layer_id);
        self.next_layer_id += 1;
        let layer = Layer {
            id,
            name: name.to_string(),
            visible: true,
            locked: false,
            opacity: 255,
            blend: BlendMode::Normal,
            kind: LayerKind::Image,
            parent: None,
            cels: vec![None; self.frames.len()],
        };
        self.layers.insert(index.min(self.layers.len()), layer);
        id
    }

    /// The next free "Layer N" name, so new layers never reuse a name.
    pub fn next_layer_name(&self) -> String {
        (1..)
            .map(|n| format!("Layer {n}"))
            .find(|name| self.layers.iter().all(|layer| &layer.name != name))
            .unwrap_or_else(|| "Layer".into())
    }

    /// Copies an image layer, with independent pixels, just above itself,
    /// in the same group. (Groups are copied whole by `duplicate_group`.)
    pub fn duplicate_layer(&mut self, index: usize) -> usize {
        let source = self.layers[index].clone();
        let id = self.insert_layer(index + 1, &format!("{} copy", source.name));
        let copy_index = self.layer_index(id).unwrap_or(index + 1);
        let cels = source
            .cels
            .iter()
            .map(|cel| {
                cel.and_then(|cel| {
                    let image = self.images.get(&cel.image).cloned()?;
                    Some(Cel { image: self.add_image((*image).clone()), ..cel })
                })
            })
            .collect();
        self.layers[copy_index] = Layer { id, name: format!("{} copy", source.name), cels, ..source };
        copy_index
    }

    /// Composites `index` onto the layer below it, keeping the lower layer's
    /// name, and removes the upper one. A hidden layer is dropped unpainted,
    /// matching what was on screen. Both must be image layers in the same
    /// group.
    pub fn merge_down(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.layers.len() {
            return false;
        }
        let (upper, lower) = (&self.layers[index], &self.layers[index - 1]);
        if upper.is_group() || lower.is_group() || upper.parent != lower.parent {
            return false;
        }
        let upper = self.layers.remove(index);
        if upper.visible {
            self.stamp_layer_onto(&upper, index - 1);
        }
        self.collect_unused_images();
        true
    }

    /// Paints every cel of `upper` onto layer `lower`, with the upper
    /// layer's blend mode and its layer and cel opacity. Each merged cel gets
    /// a fresh image, because the lower cel may be a clone shared with
    /// frames whose upper art differs. Frames that shared both their lower
    /// and their upper cel still look alike after the merge, so they stay
    /// linked. The lower cel's own opacity is baked into the merged pixels,
    /// so the result shows at full cel opacity.
    fn stamp_layer_onto(&mut self, upper: &Layer, lower: usize) {
        let mut merged: BTreeMap<(Option<Cel>, Cel), ImageId> = BTreeMap::new();
        for frame in 0..self.frames.len() {
            let Some(top) = upper.cels[frame] else { continue };
            let bottom = self.layers[lower].cels[frame];
            let id = match merged.get(&(bottom, top)) {
                Some(&id) => id,
                None => {
                    let mut image = Pixels::new(self.width, self.height);
                    if let Some(bottom) = bottom {
                        image.stamp(&self.images[&bottom.image], 0, 0, bottom.opacity);
                    }
                    let opacity = mul_opacity(upper.opacity, top.opacity);
                    image.stamp_blended(&self.images[&top.image], 0, 0, opacity, upper.blend);
                    let id = self.add_image(image);
                    merged.insert((bottom, top), id);
                    id
                }
            };
            self.layers[lower].cels[frame] = Some(Cel::showing(id));
        }
    }

    // ----- frames -------------------------------------------------------

    pub fn insert_frame(&mut self, index: usize, duration_ms: u32) {
        let index = index.min(self.frames.len());
        self.frames.insert(index, Frame { duration_ms, note: String::new() });
        for layer in &mut self.layers {
            layer.cels.insert(index, None);
        }
        for tag in &mut self.tags {
            if tag.from >= index {
                tag.from += 1;
            }
            if tag.to >= index {
                tag.to += 1;
            }
        }
    }

    /// Copies a frame just after itself. With `as_clone`, every cel in the
    /// copy is linked to the original's image instead of copied.
    pub fn duplicate_frame(&mut self, index: usize, as_clone: bool) -> usize {
        let duration = self.frames[index].duration_ms;
        self.insert_frame(index + 1, duration);
        for layer_index in 0..self.layers.len() {
            let Some(source) = self.layers[layer_index].cels[index] else { continue };
            let target = if as_clone {
                source
            } else {
                let pixels = (*self.images[&source.image]).clone();
                Cel { image: self.add_image(pixels), ..source }
            };
            self.layers[layer_index].cels[index + 1] = Some(target);
        }
        index + 1
    }

    pub fn remove_frame(&mut self, index: usize) -> bool {
        if self.frames.len() <= 1 || index >= self.frames.len() {
            return false;
        }
        self.frames.remove(index);
        for layer in &mut self.layers {
            layer.cels.remove(index);
        }
        let last = self.frames.len() - 1;
        self.tags.retain_mut(|tag| {
            let shift = |f: usize| if f > index { f - 1 } else { f };
            let covered_only_the_removed_frame = tag.from == index && tag.to == index;
            tag.from = shift(tag.from).min(last);
            tag.to = shift(tag.to).min(last);
            !covered_only_the_removed_frame
        });
        self.collect_unused_images();
        true
    }

    /// Moves a frame to a new position, carrying its cels with it.
    pub fn move_frame(&mut self, from: usize, to: usize) {
        if from >= self.frames.len() || to >= self.frames.len() || from == to {
            return;
        }
        let frame = self.frames.remove(from);
        self.frames.insert(to, frame);
        for layer in &mut self.layers {
            let cel = layer.cels.remove(from);
            layer.cels.insert(to, cel);
        }
    }

    // ----- cels and clones ---------------------------------------------

    pub fn add_image(&mut self, pixels: Pixels) -> ImageId {
        let id = ImageId(self.next_image_id);
        self.next_image_id += 1;
        self.images.insert(id, Arc::new(pixels));
        id
    }

    pub fn cel(&self, layer: usize, frame: usize) -> Option<Cel> {
        *self.layers.get(layer)?.cels.get(frame)?
    }

    pub fn cel_image(&self, layer: usize, frame: usize) -> Option<&Pixels> {
        let cel = self.cel(layer, frame)?;
        self.images.get(&cel.image).map(|image| &**image)
    }

    /// The image at a cel, ready to paint. An empty cel gets a fresh
    /// transparent image. An image shared with an undo snapshot is copied
    /// first (copy-on-write). An image shared with *linked cels* is not
    /// copied: they're meant to change together.
    pub fn cel_image_mut(&mut self, layer: usize, frame: usize) -> &mut Pixels {
        let id = match self.layers[layer].cels[frame] {
            Some(cel) => cel.image,
            None => {
                let id = self.add_image(Pixels::new(self.width, self.height));
                self.layers[layer].cels[frame] = Some(Cel::showing(id));
                id
            }
        };
        let image = self.images.get_mut(&id).expect("every cel points at a pooled image");
        Arc::make_mut(image)
    }

    /// Every (layer, frame) that shares the image at this cel, itself
    /// included.
    pub fn linked_cels_of(&self, layer: usize, frame: usize) -> Vec<(usize, usize)> {
        let Some(target) = self.cel(layer, frame).map(|cel| cel.image) else {
            return Vec::new();
        };
        self.layers
            .iter()
            .enumerate()
            .flat_map(|(li, l)| l.cels.iter().enumerate().map(move |(fi, cel)| (li, fi, *cel)))
            .filter(|&(_, _, cel)| cel.is_some_and(|cel| cel.image == target))
            .map(|(li, fi, _)| (li, fi))
            .collect()
    }

    pub fn is_linked(&self, layer: usize, frame: usize) -> bool {
        self.linked_cels_of(layer, frame).len() > 1
    }

    /// Gives a linked cel its own private copy of the pixels.
    pub fn unlink_cel(&mut self, layer: usize, frame: usize) -> bool {
        if !self.is_linked(layer, frame) {
            return false;
        }
        let (Some(cel), Some(pixels)) = (self.cel(layer, frame), self.cel_image(layer, frame).cloned()) else {
            return false;
        };
        let image = self.add_image(pixels);
        self.layers[layer].cels[frame] = Some(Cel { image, ..cel });
        true
    }

    /// Makes `(layer, to_frame)` a clone of `(layer, from_frame)`.
    pub fn link_cel(&mut self, layer: usize, from_frame: usize, to_frame: usize) {
        self.layers[layer].cels[to_frame] = self.layers[layer].cels[from_frame];
        self.collect_unused_images();
    }

    pub fn clear_cel(&mut self, layer: usize, frame: usize) {
        self.layers[layer].cels[frame] = None;
        self.collect_unused_images();
    }

    /// Drops pool images no cel points at any more.
    pub fn collect_unused_images(&mut self) {
        let used: std::collections::HashSet<ImageId> =
            self.layers.iter().flat_map(|l| l.cels.iter().flatten().map(|cel| cel.image)).collect();
        self.images.retain(|id, _| used.contains(id));
    }

    // ----- canvas -------------------------------------------------------

    /// Changes the canvas size. Every image keeps its pixels, shifted by the
    /// offset. Anything that falls outside is cropped.
    pub fn resize_canvas(&mut self, width: u32, height: u32, offset_x: i32, offset_y: i32) {
        for image in self.images.values_mut() {
            *image = Arc::new(image.resized_canvas(width, height, offset_x, offset_y));
        }
        for pin in &mut self.pins {
            pin.x += offset_x;
            pin.y += offset_y;
        }
        for slice in &mut self.slices {
            slice.bounds.x += offset_x;
            slice.bounds.y += offset_y;
        }
        self.width = width;
        self.height = height;
    }

    /// Flattens every visible layer of a frame into one image.
    pub fn composite(&self, frame: usize) -> Pixels {
        self.composite_with(frame, None)
    }

    /// Like `composite`, but showing `stand_in` in place of one layer's
    /// cel. The editor uses this to show floating pixels before they're
    /// anchored.
    pub fn composite_with(&self, frame: usize, stand_in: Option<(usize, &Pixels)>) -> Pixels {
        self.composite_children(frame, None, stand_in)
    }

    /// Composites the visible layers directly inside `parent` (the top
    /// level when `None`), bottom to top. A group is composited on its own
    /// first, then laid down as one picture.
    fn composite_children(&self, frame: usize, parent: Option<LayerId>, stand_in: Option<(usize, &Pixels)>) -> Pixels {
        let mut out = Pixels::new(self.width, self.height);
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.parent != parent || !layer.visible {
                continue;
            }
            if layer.is_group() {
                let inside = self.composite_children(frame, Some(layer.id), stand_in);
                out.stamp_blended(&inside, 0, 0, layer.opacity, layer.blend);
                continue;
            }
            let cel_opacity = self.cel(index, frame).map_or(255, |cel| cel.opacity);
            let image = match stand_in {
                Some((stand_in_layer, image)) if stand_in_layer == index => Some(image),
                _ => self.cel_image(index, frame),
            };
            if let Some(image) = image {
                out.stamp_blended(image, 0, 0, mul_opacity(layer.opacity, cel_opacity), layer.blend);
            }
        }
        out
    }

    /// Every distinct visible colour across the whole sprite, in order of
    /// first appearance. This is "extract palette".
    pub fn used_colours(&self) -> Vec<Rgba> {
        let mut seen = std::collections::HashSet::new();
        let mut colours = Vec::new();
        for image in self.images.values() {
            for &colour in image.colours() {
                if !colour.is_transparent() && seen.insert(colour) {
                    colours.push(colour);
                }
            }
        }
        colours
    }

    /// Replaces one exact colour with another in every image.
    pub fn swap_colour(&mut self, from: Rgba, to: Rgba) -> usize {
        let mut changed = 0;
        for image in self.images.values_mut() {
            if !image.colours().contains(&from) {
                continue;
            }
            for pixel in Arc::make_mut(image).colours_mut() {
                if *pixel == from {
                    *pixel = to;
                    changed += 1;
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = Rgba::rgb(255, 0, 0);

    #[test]
    fn a_new_sprite_has_one_layer_one_frame_and_no_pixels() {
        let sprite = Sprite::new(32, 32);
        assert_eq!(sprite.layers.len(), 1);
        assert_eq!(sprite.layers[0].name, "Layer 1");
        assert_eq!(sprite.frames, vec![Frame { duration_ms: 100, note: String::new() }]);
        assert!(sprite.cel_image(0, 0).is_none());
        assert_eq!(sprite.palette.name, "PICO-8");
    }

    #[test]
    fn painting_an_empty_cel_creates_its_image() {
        let mut sprite = Sprite::new(4, 4);
        sprite.cel_image_mut(0, 0).set(1, 1, RED);
        assert_eq!(sprite.cel_image(0, 0).unwrap().get(1, 1), RED);
    }

    #[test]
    fn merging_down_onto_clones_gives_each_frame_its_own_art() {
        let mut sprite = Sprite::new(4, 4);
        sprite.cel_image_mut(0, 0).set(0, 0, RED);
        sprite.insert_frame(1, 100);
        sprite.link_cel(0, 0, 1);
        sprite.insert_layer(1, "Top");
        sprite.cel_image_mut(1, 0).set(1, 0, Rgba::BLACK);
        sprite.cel_image_mut(1, 1).set(2, 0, Rgba::BLACK);
        assert!(sprite.merge_down(1));
        let (first, second) = (sprite.cel_image(0, 0).unwrap(), sprite.cel_image(0, 1).unwrap());
        assert_eq!((first.get(0, 0), first.get(1, 0), first.get(2, 0)), (RED, Rgba::BLACK, Rgba::TRANSPARENT));
        assert_eq!((second.get(0, 0), second.get(1, 0), second.get(2, 0)), (RED, Rgba::TRANSPARENT, Rgba::BLACK));
    }

    #[test]
    fn merging_down_keeps_clones_that_stay_identical_linked() {
        let mut sprite = Sprite::new(4, 4);
        sprite.cel_image_mut(0, 0).set(0, 0, RED);
        sprite.insert_frame(1, 100);
        sprite.link_cel(0, 0, 1);
        sprite.insert_layer(1, "Top");
        sprite.cel_image_mut(1, 0).set(1, 0, Rgba::BLACK);
        sprite.link_cel(1, 0, 1);
        sprite.merge_down(1);
        assert!(sprite.is_linked(0, 0));
    }

    #[test]
    fn merging_down_a_hidden_layer_drops_it_without_painting() {
        let mut sprite = Sprite::new(4, 4);
        sprite.insert_layer(1, "Hidden");
        sprite.cel_image_mut(1, 0).set(1, 0, RED);
        sprite.layers[1].visible = false;
        sprite.merge_down(1);
        assert_eq!(sprite.layers.len(), 1);
        assert!(sprite.cel_image(0, 0).is_none_or(|image| image.get(1, 0).is_transparent()));
    }

    #[test]
    fn resizing_the_canvas_carries_slices_with_the_art() {
        let mut sprite = Sprite::new(4, 4);
        sprite.slices.push(Slice { name: "hand".into(), bounds: Rect::new(1, 1, 2, 2) });
        sprite.resize_canvas(8, 8, 2, 3);
        assert_eq!(sprite.slices[0].bounds, Rect::new(3, 4, 2, 2));
    }

    #[test]
    fn a_fresh_sprite_is_valid() {
        let mut sprite = Sprite::new(4, 4);
        sprite.cel_image_mut(0, 0).set(0, 0, RED);
        sprite.insert_frame(1, 100);
        assert_eq!(sprite.validate(), Ok(()));
    }

    #[test]
    fn validation_catches_every_broken_promise() {
        let broken: [fn(&mut Sprite); 5] = [
            |s| s.layers[0].cels.push(None),
            |s| s.layers[0].cels[0] = Some(Cel::showing(ImageId(99))),
            |s| s.width = 0,
            |s| s.tags.push(Tag { name: "walk".into(), from: 0, to: 5, direction: LoopDirection::Forward }),
            |s| {
                s.cel_image_mut(0, 0);
                s.width = 8;
            },
        ];
        for (index, breaking) in broken.iter().enumerate() {
            let mut sprite = Sprite::new(4, 4);
            breaking(&mut sprite);
            assert!(sprite.validate().is_err(), "case {index} slipped through");
        }
    }

    #[test]
    fn snapshots_do_not_see_later_paint() {
        let mut sprite = Sprite::new(4, 4);
        sprite.cel_image_mut(0, 0).set(0, 0, RED);
        let snapshot = sprite.clone();
        sprite.cel_image_mut(0, 0).set(0, 0, Rgba::BLACK);
        assert_eq!(snapshot.cel_image(0, 0).unwrap().get(0, 0), RED);
    }

    #[test]
    fn cloned_frames_share_paint_until_unlinked() {
        let mut sprite = Sprite::new(4, 4);
        sprite.cel_image_mut(0, 0).set(0, 0, RED);
        sprite.duplicate_frame(0, true);
        sprite.cel_image_mut(0, 1).set(3, 3, RED);
        assert_eq!(sprite.cel_image(0, 0).unwrap().get(3, 3), RED);
        assert_eq!(sprite.linked_cels_of(0, 0), vec![(0, 0), (0, 1)]);

        assert!(sprite.unlink_cel(0, 1));
        sprite.cel_image_mut(0, 1).set(2, 2, RED);
        assert_eq!(sprite.cel_image(0, 0).unwrap().get(2, 2), Rgba::TRANSPARENT);
        assert!(!sprite.is_linked(0, 0));
    }

    #[test]
    fn duplicated_frames_without_cloning_are_independent() {
        let mut sprite = Sprite::new(4, 4);
        sprite.cel_image_mut(0, 0).set(0, 0, RED);
        sprite.duplicate_frame(0, false);
        sprite.cel_image_mut(0, 1).set(1, 1, RED);
        assert_eq!(sprite.cel_image(0, 0).unwrap().get(1, 1), Rgba::TRANSPARENT);
    }

    #[test]
    fn inserting_frames_shifts_tags_after_the_insertion_point() {
        let mut sprite = Sprite::new(4, 4);
        sprite.insert_frame(1, 100);
        sprite.insert_frame(2, 100);
        sprite.tags.push(Tag { name: "walk".into(), from: 1, to: 2, direction: LoopDirection::Forward });
        sprite.insert_frame(0, 100);
        assert_eq!((sprite.tags[0].from, sprite.tags[0].to), (2, 3));
    }

    #[test]
    fn removing_a_frame_drops_tags_that_covered_only_it() {
        let mut sprite = Sprite::new(4, 4);
        sprite.insert_frame(1, 100);
        sprite.tags.push(Tag { name: "blink".into(), from: 1, to: 1, direction: LoopDirection::Forward });
        assert!(sprite.remove_frame(1));
        assert!(sprite.tags.is_empty());
    }

    #[test]
    fn the_last_frame_cannot_be_removed() {
        assert!(!Sprite::new(4, 4).remove_frame(0));
    }

    #[test]
    fn moving_a_frame_carries_its_cels() {
        let mut sprite = Sprite::new(4, 4);
        sprite.insert_frame(1, 250);
        sprite.cel_image_mut(0, 1).set(0, 0, RED);
        sprite.move_frame(1, 0);
        assert_eq!(sprite.frames[0].duration_ms, 250);
        assert_eq!(sprite.cel_image(0, 0).unwrap().get(0, 0), RED);
    }

    #[test]
    fn composite_respects_visibility_and_opacity() {
        let mut sprite = Sprite::new(1, 1);
        sprite.cel_image_mut(0, 0).set(0, 0, Rgba::BLACK);
        sprite.insert_layer(1, "top");
        sprite.cel_image_mut(1, 0).set(0, 0, Rgba::WHITE);
        sprite.layers[1].opacity = 128;
        let mid = sprite.composite(0).get(0, 0);
        assert!((126..=130).contains(&mid.r()), "{mid:?}");
        sprite.layers[1].visible = false;
        assert_eq!(sprite.composite(0).get(0, 0), Rgba::BLACK);
    }

    #[test]
    fn merge_down_flattens_into_the_lower_layer() {
        let mut sprite = Sprite::new(2, 1);
        sprite.cel_image_mut(0, 0).set(0, 0, Rgba::BLACK);
        sprite.insert_layer(1, "top");
        sprite.cel_image_mut(1, 0).set(1, 0, RED);
        assert!(sprite.merge_down(1));
        assert_eq!(sprite.layers.len(), 1);
        assert_eq!(sprite.cel_image(0, 0).unwrap().get(1, 0), RED);
        assert!(!sprite.merge_down(0));
    }

    #[test]
    fn duplicated_layers_copy_pixels_independently() {
        let mut sprite = Sprite::new(2, 2);
        sprite.cel_image_mut(0, 0).set(0, 0, RED);
        let copy = sprite.duplicate_layer(0);
        assert_eq!(sprite.layers[copy].name, "Layer 1 copy");
        sprite.cel_image_mut(copy, 0).set(1, 1, RED);
        assert_eq!(sprite.cel_image(0, 0).unwrap().get(1, 1), Rgba::TRANSPARENT);
    }

    #[test]
    fn layer_names_are_never_reused() {
        let mut sprite = Sprite::new(2, 2);
        assert_eq!(sprite.next_layer_name(), "Layer 2");
        sprite.insert_layer(1, "Layer 2");
        assert_eq!(sprite.next_layer_name(), "Layer 3");
    }

    #[test]
    fn resizing_the_canvas_keeps_pixels_at_their_offset() {
        let mut sprite = Sprite::new(4, 4);
        sprite.cel_image_mut(0, 0).set(1, 1, RED);
        sprite.resize_canvas(8, 2, 2, 0);
        let image = sprite.cel_image(0, 0).unwrap();
        assert_eq!((image.width(), image.height()), (8, 2));
        assert_eq!(image.get(3, 1), RED);
    }

    #[test]
    fn swapping_a_colour_reaches_every_frame() {
        let mut sprite = Sprite::new(2, 2);
        sprite.cel_image_mut(0, 0).set(0, 0, RED);
        sprite.insert_frame(1, 100);
        sprite.cel_image_mut(0, 1).set(1, 1, RED);
        assert_eq!(sprite.swap_colour(RED, Rgba::BLACK), 2);
        assert_eq!(sprite.used_colours(), vec![Rgba::BLACK]);
    }

    #[test]
    fn unused_images_are_collected() {
        let mut sprite = Sprite::new(2, 2);
        sprite.cel_image_mut(0, 0).set(0, 0, RED);
        sprite.clear_cel(0, 0);
        assert!(sprite.images.is_empty());
    }
}

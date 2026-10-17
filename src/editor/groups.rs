//! Layer groups.
//!
//! Layers stay one flat list, bottom to top, so everything that walks
//! layers by index keeps working. A layer's `parent` says which group it
//! belongs to. After any change to the structure, `tidy_layers` puts the
//! list back into its canonical shape: each group's children sit directly
//! below it, in the order they had. That shape is what the Layers panel
//! shows (children indented under their group) and what Aseprite files
//! use, so a moved group always carries its children along.

use super::sprite::{Layer, LayerId, LayerKind, Sprite};
use std::collections::HashMap;

impl Sprite {
    /// Adds an empty group at `index`, inside `parent`.
    pub fn insert_group(&mut self, index: usize, name: &str, parent: Option<LayerId>) -> LayerId {
        let id = self.insert_layer(index, name);
        let position = self.layer_index(id).expect("just inserted");
        let group = &mut self.layers[position];
        group.kind = LayerKind::Group;
        group.parent = parent;
        self.tidy_layers();
        id
    }

    /// How many groups a layer is inside.
    pub fn depth(&self, index: usize) -> usize {
        self.ancestors(index).len()
    }

    /// The groups containing a layer, innermost first. Stops rather than
    /// loop if a damaged file makes a cycle.
    fn ancestors(&self, index: usize) -> Vec<LayerId> {
        let mut found = Vec::new();
        let mut parent = self.layers.get(index).and_then(|layer| layer.parent);
        while let Some(id) = parent {
            if found.contains(&id) || found.len() > self.layers.len() {
                break;
            }
            found.push(id);
            parent = self.layer_index(id).and_then(|i| self.layers[i].parent);
        }
        found
    }

    /// Whether a layer actually shows: it and every group around it are
    /// visible.
    pub fn is_shown(&self, index: usize) -> bool {
        self.layers.get(index).is_some_and(|layer| layer.visible)
            && self.ancestors(index).iter().all(|id| self.layer_index(*id).is_some_and(|i| self.layers[i].visible))
    }

    /// Every layer inside `group`, at any depth, bottom to top.
    pub fn descendants(&self, group: LayerId) -> Vec<LayerId> {
        let mut found = Vec::new();
        for layer in self.layers.iter().filter(|layer| layer.parent == Some(group)) {
            if layer.is_group() {
                found.extend(self.descendants(layer.id));
            }
            found.push(layer.id);
        }
        found
    }

    /// Moves the layer at `index` into `group` (on top of what's already
    /// there), or out to the top level with `None`. A layer can't go into
    /// something that isn't a group, or into itself or its own children.
    pub fn set_parent(&mut self, index: usize, group: Option<LayerId>) -> Result<(), String> {
        let layer = self.layers.get(index).ok_or("there's no such layer")?;
        let id = layer.id;
        let old_parent = layer.parent;
        if let Some(group) = group {
            let target = self.layer_index(group).ok_or("there's no such group")?;
            if !self.layers[target].is_group() {
                return Err(format!("\"{}\" is a layer, not a group", self.layers[target].name));
            }
            if group == id || self.descendants(id).contains(&group) {
                return Err("a group can't go inside itself".into());
            }
        }
        let moving = self.layers.remove(index);
        // Into a group: just below it, which is the top of its children.
        // Out of one: just above the group it left.
        let landing = match (group, old_parent) {
            (Some(group), _) => self.layer_index(group).unwrap_or(0),
            (None, Some(old)) => self.layer_index(old).map_or(self.layers.len(), |i| i + 1),
            (None, None) => index.min(self.layers.len()),
        };
        self.layers.insert(landing, Layer { parent: group, ..moving });
        self.tidy_layers();
        Ok(())
    }

    /// Removes a layer, and everything inside it if it's a group. Returns
    /// how many layers went.
    pub fn remove_layer(&mut self, index: usize) -> usize {
        let Some(layer) = self.layers.get(index) else { return 0 };
        let mut doomed = self.descendants(layer.id);
        doomed.push(layer.id);
        self.layers.retain(|layer| !doomed.contains(&layer.id));
        self.collect_unused_images();
        doomed.len()
    }

    /// Copies a group and everything in it, with independent pixels, just
    /// above the original. Returns the copy's index.
    pub fn duplicate_group(&mut self, index: usize) -> usize {
        let group = self.layers[index].id;
        let mut subtree = self.descendants(group);
        subtree.push(group);
        let mut copies: HashMap<LayerId, LayerId> = HashMap::new();
        for original in subtree {
            let at = self.layer_index(original).expect("the subtree is in the sprite");
            let copy_index = if self.layers[at].is_group() {
                let source = self.layers[at].clone();
                let id = self.insert_layer(at + 1, &format!("{} copy", source.name));
                let copy_index = self.layer_index(id).expect("just inserted");
                self.layers[copy_index] = Layer { id, name: format!("{} copy", source.name), ..source };
                copy_index
            } else {
                self.duplicate_layer(at)
            };
            copies.insert(original, self.layers[copy_index].id);
        }
        for layer in &mut self.layers {
            if copies.values().any(|copy| *copy == layer.id) {
                layer.parent = layer.parent.map(|parent| copies.get(&parent).copied().unwrap_or(parent));
            }
        }
        self.tidy_layers();
        self.layer_index(copies[&group]).expect("the copy is in the sprite")
    }

    /// Puts layers back in canonical order: siblings keep their order, and
    /// each group's children come directly below it. A layer whose parent
    /// is missing or isn't a group moves to the top level.
    pub fn tidy_layers(&mut self) {
        let groups: Vec<LayerId> = self.layers.iter().filter(|l| l.is_group()).map(|l| l.id).collect();
        for layer in &mut self.layers {
            if layer.parent.is_some_and(|parent| !groups.contains(&parent) || parent == layer.id) {
                layer.parent = None;
            }
        }
        let mut order = Vec::with_capacity(self.layers.len());
        self.emit_in_order(None, &mut order, 0);
        // A cycle in a damaged file leaves some layers unreached; keep them.
        for layer in &self.layers {
            if !order.contains(&layer.id) {
                order.push(layer.id);
            }
        }
        let mut by_id: HashMap<LayerId, Layer> = self.layers.drain(..).map(|layer| (layer.id, layer)).collect();
        self.layers = order.into_iter().filter_map(|id| by_id.remove(&id)).collect();
    }

    fn emit_in_order(&self, parent: Option<LayerId>, order: &mut Vec<LayerId>, depth: usize) {
        if depth > self.layers.len() {
            return;
        }
        for layer in self.layers.iter().filter(|layer| layer.parent == parent) {
            if layer.is_group() {
                self.emit_in_order(Some(layer.id), order, depth + 1);
            }
            order.push(layer.id);
        }
    }

    /// Every parent is a group in this sprite, and no group contains
    /// itself.
    pub(super) fn validate_groups(&self) -> Result<(), String> {
        for (index, layer) in self.layers.iter().enumerate() {
            let Some(parent) = layer.parent else { continue };
            let group = self.layer_index(parent).map(|i| &self.layers[i]);
            if !group.is_some_and(Layer::is_group) {
                return Err(format!("layer \"{}\" is in a group that doesn't exist", layer.name));
            }
            if self.ancestors(index).contains(&layer.id) {
                return Err(format!("group \"{}\" contains itself", layer.name));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{BlendMode, Rgba};
    use crate::editor::{FrameIndex, LayerIndex};

    const RED: Rgba = Rgba::rgb(255, 0, 0);

    /// Bottom to top: "Body", then group "Head" holding "Face" and "Hair".
    fn head_and_body() -> Sprite {
        let mut sprite = Sprite::new(4, 4);
        sprite.layers[0].name = "Body".into();
        let head = sprite.insert_group(1, "Head", None);
        for name in ["Face", "Hair"] {
            let id = sprite.insert_layer(sprite.layers.len(), name);
            let index = sprite.layer_index(id).unwrap();
            sprite.set_parent(index, Some(head)).unwrap();
        }
        sprite
    }

    fn names(sprite: &Sprite) -> Vec<&str> {
        sprite.layers.iter().map(|l| l.name.as_str()).collect()
    }

    #[test]
    fn children_sit_directly_below_their_group() {
        let sprite = head_and_body();
        assert_eq!(names(&sprite), vec!["Body", "Face", "Hair", "Head"]);
        assert_eq!((sprite.depth(1), sprite.depth(3)), (1, 0));
    }

    #[test]
    fn a_group_can_not_go_inside_itself_or_a_plain_layer() {
        let mut sprite = head_and_body();
        let head = sprite.layers[3].id;
        let face_index = 1;
        assert!(sprite.set_parent(3, Some(head)).unwrap_err().contains("itself"));
        let face = sprite.layers[face_index].id;
        assert!(sprite.set_parent(0, Some(face)).unwrap_err().contains("not a group"));
    }

    #[test]
    fn moving_out_of_a_group_lands_just_above_it() {
        let mut sprite = head_and_body();
        sprite.set_parent(1, None).unwrap();
        assert_eq!(names(&sprite), vec!["Body", "Hair", "Head", "Face"]);
    }

    #[test]
    fn hiding_a_group_hides_everything_in_it() {
        let mut sprite = head_and_body();
        sprite.cel_image_mut(LayerIndex(1), FrameIndex(0)).set(0, 0, RED);
        assert_eq!(sprite.composite(0).get(0, 0), RED);
        sprite.layers[3].visible = false;
        assert!(sprite.composite(0).get(0, 0).is_transparent());
        assert!(!sprite.is_shown(1));
    }

    #[test]
    fn a_group_blends_its_children_as_one_picture() {
        let mut sprite = head_and_body();
        sprite.cel_image_mut(LayerIndex(0), FrameIndex(0)).set(0, 0, Rgba::rgb(200, 200, 200));
        sprite.cel_image_mut(LayerIndex(1), FrameIndex(0)).set(0, 0, Rgba::rgb(255, 0, 0));
        sprite.cel_image_mut(LayerIndex(2), FrameIndex(0)).set(0, 0, Rgba::rgb(0, 0, 255));
        sprite.layers[3].blend = BlendMode::Multiply;
        // Inside the group Hair covers Face; the group's result (blue)
        // multiplies onto the body.
        assert_eq!(sprite.composite(0).get(0, 0), Rgba::rgb(0, 0, 200));
    }

    #[test]
    fn removing_a_group_removes_its_children() {
        let mut sprite = head_and_body();
        assert_eq!(sprite.remove_layer(3), 3);
        assert_eq!(names(&sprite), vec!["Body"]);
    }

    #[test]
    fn duplicating_a_group_copies_the_whole_tree_with_its_own_pixels() {
        let mut sprite = head_and_body();
        sprite.cel_image_mut(LayerIndex(1), FrameIndex(0)).set(0, 0, RED);
        let copy = sprite.duplicate_group(3);
        assert_eq!(names(&sprite), vec!["Body", "Face", "Hair", "Head", "Face copy", "Hair copy", "Head copy"]);
        assert_eq!(copy, 6);
        assert_eq!(sprite.layers[4].parent, Some(sprite.layers[6].id));
        sprite.cel_image_mut(LayerIndex(4), FrameIndex(0)).set(0, 0, Rgba::BLACK);
        assert_eq!(
            sprite.cel_image(LayerIndex(1), FrameIndex(0)).unwrap().get(0, 0),
            RED,
            "the original keeps its pixels"
        );
        assert_eq!(sprite.validate(), Ok(()));
    }

    #[test]
    fn validation_catches_orphans_and_cycles() {
        let mut orphan = head_and_body();
        orphan.layers[1].parent = Some(LayerId(999));
        assert!(orphan.validate().unwrap_err().contains("doesn't exist"));
        let mut cycle = head_and_body();
        let head = cycle.layers[3].id;
        let inner = cycle.insert_group(0, "Inner", Some(head));
        let head_index = cycle.layer_index(head).unwrap();
        cycle.layers[head_index].parent = Some(inner);
        assert!(cycle.validate().unwrap_err().contains("itself"));
    }
}

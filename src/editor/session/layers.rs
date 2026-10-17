//! Layers and groups.

use super::*;
use crate::editor::sprite::LayerId;
use crate::editor::BlendMode;

impl Editor {
    pub(super) fn add_layer(&mut self, name: Option<String>) -> EditResult {
        let name = name.unwrap_or_else(|| self.sprite().next_layer_name());
        let id = self.insert_beside_current(|sprite, index, parent| {
            let id = sprite.insert_layer(index, &name);
            let position = sprite.layer_index(id).expect("just inserted");
            sprite.layers[position].parent = parent;
            id
        });
        Ok(format!("added layer \"{name}\"{}", self.inside_which_group(id)))
    }

    pub(super) fn add_group(&mut self, name: Option<String>) -> EditResult {
        let name = name.unwrap_or_else(|| "Group".into());
        let id = self.insert_beside_current(|sprite, index, parent| sprite.insert_group(index, &name, parent));
        Ok(format!("added group \"{name}\"{}", self.inside_which_group(id)))
    }

    pub(super) fn set_layer_parent(&mut self, group: Option<usize>) -> EditResult {
        let id = self.sprite().layers[self.layer].id;
        let group = group.map(|index| self.check_layer_index(index).map(|i| self.sprite().layers[i].id)).transpose()?;
        self.document.sprite.set_parent(self.layer, group).map_err(EditError::Invalid)?;
        self.layer = self.sprite().layer_index(id).unwrap_or(0);
        Ok(match group {
            Some(_) => format!("moved the layer{}", self.inside_which_group(id)),
            None => "moved the layer out to the top level".into(),
        })
    }

    pub(super) fn remove_current_layer(&mut self) -> EditResult {
        let doomed = &self.sprite().layers[self.layer];
        let (name, id) = (doomed.name.clone(), doomed.id);
        if self.sprite().descendants(id).len() + 1 >= self.sprite().layers.len() {
            return Err(EditError::Invalid("a sprite needs at least one layer".into()));
        }
        let removed = self.document.sprite.remove_layer(self.layer);
        self.layer = self.layer.saturating_sub(removed - 1).min(self.sprite().layers.len() - 1);
        Ok(if removed > 1 {
            format!("removed group \"{name}\" and the {} layers in it", removed - 1)
        } else {
            format!("removed layer \"{name}\"")
        })
    }

    pub(super) fn duplicate_current_layer(&mut self) -> EditResult {
        self.layer = if self.sprite().layers[self.layer].is_group() {
            self.document.sprite.duplicate_group(self.layer)
        } else {
            self.document.sprite.duplicate_layer(self.layer)
        };
        Ok(format!("duplicated into \"{}\"", self.sprite().layers[self.layer].name))
    }

    pub(super) fn merge_current_layer_down(&mut self) -> EditResult {
        if !self.document.sprite.merge_down(self.layer) {
            return Err(EditError::Invalid(
                "merge down needs an image layer with another image layer below it, in the same group".into(),
            ));
        }
        self.layer -= 1;
        Ok("merged down".into())
    }

    pub(super) fn rename_layer(&mut self, name: &str) -> EditResult {
        if name.trim().is_empty() {
            return Err(EditError::Invalid("layer names can't be blank".into()));
        }
        self.document.sprite.layers[self.layer].name = name.trim().to_string();
        Ok(format!("renamed the layer to \"{}\"", name.trim()))
    }

    /// Reorders among siblings. A group moves with its children, and
    /// nothing changes group (that's `set_layer_parent`).
    pub(super) fn move_layer(&mut self, to: usize) -> EditResult {
        let to = self.check_layer_index(to)?;
        let layer = self.document.sprite.layers.remove(self.layer);
        let id = layer.id;
        self.document.sprite.layers.insert(to, layer);
        self.document.sprite.tidy_layers();
        self.layer = self.sprite().layer_index(id).unwrap_or(to);
        Ok(format!("moved the layer to position {}", self.layer))
    }

    pub(super) fn set_layer_visible(&mut self, visible: bool) -> EditResult {
        self.document.sprite.layers[self.layer].visible = visible;
        Ok(if visible { "showed the layer" } else { "hid the layer" }.into())
    }

    pub(super) fn set_layer_locked(&mut self, locked: bool) -> EditResult {
        self.document.sprite.layers[self.layer].locked = locked;
        Ok(if locked { "locked the layer" } else { "unlocked the layer" }.into())
    }

    pub(super) fn set_layer_opacity(&mut self, opacity: u8) -> EditResult {
        self.document.sprite.layers[self.layer].opacity = opacity;
        Ok(format!("set layer opacity to {opacity}"))
    }

    pub(super) fn set_layer_blend(&mut self, blend: BlendMode) -> EditResult {
        self.document.sprite.layers[self.layer].blend = blend;
        Ok(format!("the layer now blends with {}", blend.name()))
    }

    pub(super) fn set_cel_opacity(&mut self, opacity: u8) -> EditResult {
        let (layer, frame) = (self.layer, self.frame);
        let cel = self.document.sprite.layers[layer].cels[frame]
            .as_mut()
            .ok_or_else(|| EditError::Invalid("this cel is empty, so there's nothing to fade".into()))?;
        cel.opacity = opacity;
        Ok(format!("set cel opacity to {opacity}"))
    }

    /// Inserts a new layer or group just above the current one, in the
    /// same group (a group's own level when a group is current), then
    /// selects it.
    pub(super) fn insert_beside_current(
        &mut self,
        insert: impl FnOnce(&mut Sprite, usize, Option<LayerId>) -> LayerId,
    ) -> LayerId {
        let parent = self.sprite().layers[self.layer].parent;
        let id = insert(&mut self.document.sprite, self.layer + 1, parent);
        self.document.sprite.tidy_layers();
        self.layer = self.sprite().layer_index(id).unwrap_or(self.layer);
        id
    }

    pub(super) fn inside_which_group(&self, id: LayerId) -> String {
        let sprite = self.sprite();
        let parent = sprite.layer_index(id).and_then(|i| sprite.layers[i].parent);
        parent
            .and_then(|p| sprite.layer_index(p))
            .map_or(String::new(), |g| format!(" in \"{}\"", sprite.layers[g].name))
    }
}

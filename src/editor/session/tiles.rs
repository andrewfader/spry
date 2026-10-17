//! Tilesets and tilemap layers.

use super::*;
use crate::editor::sprite::LayerKind;
use crate::editor::tilemap::{TileRef, TilesetId};

impl Editor {
    pub(super) fn add_tilemap_layer(
        &mut self,
        name: Option<String>,
        tileset: Option<u32>,
        tile_width: u32,
        tile_height: u32,
    ) -> EditResult {
        let name = name.unwrap_or_else(|| self.sprite().next_layer_name());
        let tileset = match tileset {
            Some(id) => {
                let id = TilesetId(id);
                self.sprite().tileset(id).ok_or_else(|| EditError::NotFound(format!("tileset {}", id.0)))?;
                id
            }
            None => self.document.sprite.add_tileset(&format!("{name} tiles"), tile_width, tile_height),
        };
        self.add_layer(Some(name.clone()))?;
        self.document.sprite.layers[self.layer].kind = LayerKind::Tilemap { tileset };
        Ok(format!("added tilemap layer \"{name}\" using tileset {}", tileset.0))
    }

    pub(super) fn convert_to_tilemap(&mut self, tile_width: u32, tile_height: u32) -> EditResult {
        self.ensure_drawable()?;
        if self.sprite().layers[self.layer].kind != LayerKind::Image {
            return Err(EditError::Invalid("only an image layer can be cut into tiles".into()));
        }
        self.anchor_floating();
        let id = self.document.sprite.convert_to_tilemap(LayerIndex(self.layer), tile_width, tile_height);
        let distinct = self.sprite().tileset(id).map_or(0, |t| t.tiles.len() - 1);
        Ok(format!("cut the layer into {distinct} distinct {tile_width}×{tile_height} tiles (tileset {})", id.0))
    }

    pub(super) fn place_tile(&mut self, at: Point, tile: u32, flip_x: bool, flip_y: bool) -> EditResult {
        self.ensure_drawable()?;
        let layer = LayerIndex(self.layer);
        let tileset = self
            .sprite()
            .tilemap_of(layer)
            .and_then(|id| self.sprite().tileset(id))
            .ok_or_else(|| EditError::Invalid("tiles go on a tilemap layer; this is an image layer".into()))?;
        if tile as usize >= tileset.tiles.len() {
            return Err(EditError::NotFound(format!("tile {tile} (the tileset has 0 to {})", tileset.tiles.len() - 1)));
        }
        if !self.sprite().bounds().contains(at.0, at.1) {
            return Err(EditError::Invalid(format!("({}, {}) is off the canvas", at.0, at.1)));
        }
        let (column, row) = (at.0 as u32 / tileset.tile_width, at.1 as u32 / tileset.tile_height);
        self.document.sprite.set_tile(layer, FrameIndex(self.frame), column, row, TileRef::new(tile, flip_x, flip_y));
        Ok(if tile == 0 {
            format!("cleared cell ({column}, {row})")
        } else {
            format!("placed tile {tile} in cell ({column}, {row})")
        })
    }
}

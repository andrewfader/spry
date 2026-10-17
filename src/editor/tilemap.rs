//! Tilesets and tilemap layers.
//!
//! A *tileset* is a list of same-sized tiles; tile 0 is always the empty
//! tile. A *tilemap layer* paints with them: each of its cels is a grid
//! saying which tile sits in each cell, possibly flipped. As in Aseprite,
//! that grid is stored as a small image in the ordinary image pool, one
//! 32-bit tile reference per "pixel", so clones, undo, versions and the
//! `.spry` format handle tilemaps with no extra machinery.
//!
//! Everything else in the editor reads and writes a cel's pixels through
//! `cel_pixels` and `write_cel_pixels`. For a tilemap, reading draws the
//! tiles; writing works out which tiles the new pixels changed and edits
//! those tiles in the tileset, so every placement of a tile updates at
//! once. Painting into an empty cell makes a new tile, or reuses an
//! identical one.

use super::color::Rgba;
use super::pixels::{Pixels, Rect};
use super::sprite::{Cel, FrameIndex, LayerIndex, LayerKind, Sprite};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TilesetId(pub u32);

pub const MAX_TILE_SIDE: u32 = 256;
pub const MAX_TILES: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tileset {
    pub id: TilesetId,
    pub name: String,
    pub tile_width: u32,
    pub tile_height: u32,
    /// Tile 0 is the empty tile and is never edited.
    pub tiles: Vec<Arc<Pixels>>,
}

impl Tileset {
    pub fn new(id: TilesetId, name: &str, tile_width: u32, tile_height: u32) -> Tileset {
        let empty = Arc::new(Pixels::new(tile_width, tile_height));
        Tileset { id, name: name.to_string(), tile_width, tile_height, tiles: vec![empty] }
    }

    /// The index of a tile with exactly these pixels, adding it if new.
    /// A blank tile is always tile 0.
    pub fn find_or_add(&mut self, pixels: Pixels) -> u32 {
        if pixels.is_blank() {
            return 0;
        }
        if let Some(index) = self.tiles.iter().position(|tile| **tile == pixels) {
            return index as u32;
        }
        self.tiles.push(Arc::new(pixels));
        (self.tiles.len() - 1) as u32
    }
}

/// One cell of a tilemap: which tile, and how it's flipped. Stored in 32
/// bits as Aseprite does: the tile index in the low 29, then the flips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TileRef(pub u32);

impl TileRef {
    pub const FLIP_X: u32 = 1 << 31;
    pub const FLIP_Y: u32 = 1 << 30;
    pub const DIAGONAL: u32 = 1 << 29;
    const INDEX: u32 = (1 << 29) - 1;
    pub const EMPTY: TileRef = TileRef(0);

    pub fn new(index: u32, flip_x: bool, flip_y: bool) -> TileRef {
        let flags = if flip_x { Self::FLIP_X } else { 0 } | if flip_y { Self::FLIP_Y } else { 0 };
        TileRef((index & Self::INDEX) | flags)
    }

    pub fn index(self) -> usize {
        (self.0 & Self::INDEX) as usize
    }

    pub fn is_empty(self) -> bool {
        self.index() == 0
    }

    /// The tile as it appears in this cell: diagonal flip (a transpose)
    /// first, then x, then y.
    pub fn orient(self, tile: &Pixels) -> Pixels {
        let mut shown = tile.clone();
        if self.0 & Self::DIAGONAL != 0 {
            shown = shown.rotated_clockwise().flipped_horizontally();
        }
        if self.0 & Self::FLIP_X != 0 {
            shown = shown.flipped_horizontally();
        }
        if self.0 & Self::FLIP_Y != 0 {
            shown = shown.flipped_vertically();
        }
        shown
    }

    /// Undoes `orient`, to turn edited on-screen pixels back into the tile.
    pub fn unorient(self, shown: &Pixels) -> Pixels {
        let mut tile = shown.clone();
        if self.0 & Self::FLIP_Y != 0 {
            tile = tile.flipped_vertically();
        }
        if self.0 & Self::FLIP_X != 0 {
            tile = tile.flipped_horizontally();
        }
        if self.0 & Self::DIAGONAL != 0 {
            tile = tile.rotated_clockwise().flipped_horizontally();
        }
        tile
    }

    fn to_colour(self) -> Rgba {
        Rgba(self.0.to_le_bytes())
    }

    fn from_colour(colour: Rgba) -> TileRef {
        TileRef(u32::from_le_bytes(colour.0))
    }
}

impl Sprite {
    pub fn add_tileset(&mut self, name: &str, tile_width: u32, tile_height: u32) -> TilesetId {
        let id = TilesetId(self.tilesets.iter().map(|t| t.id.0 + 1).max().unwrap_or(1));
        self.tilesets.push(Tileset::new(id, name, tile_width, tile_height));
        id
    }

    pub fn tileset(&self, id: TilesetId) -> Option<&Tileset> {
        self.tilesets.iter().find(|t| t.id == id)
    }

    fn tileset_mut(&mut self, id: TilesetId) -> Option<&mut Tileset> {
        self.tilesets.iter_mut().find(|t| t.id == id)
    }

    /// The tileset a layer paints with, if it's a tilemap layer.
    pub fn tilemap_of(&self, LayerIndex(layer): LayerIndex) -> Option<TilesetId> {
        match self.layers.get(layer)?.kind {
            LayerKind::Tilemap { tileset } => Some(tileset),
            _ => None,
        }
    }

    /// How many cells across and down a tilemap needs to cover the canvas.
    pub fn grid_size(&self, tileset: &Tileset) -> (u32, u32) {
        (self.width.div_ceil(tileset.tile_width), self.height.div_ceil(tileset.tile_height))
    }

    pub fn tile_at(&self, layer: LayerIndex, frame: FrameIndex, column: u32, row: u32) -> TileRef {
        self.cel_image(layer, frame)
            .map_or(TileRef::EMPTY, |grid| TileRef::from_colour(grid.get(column as i32, row as i32)))
    }

    /// Puts a tile in a cell of a tilemap layer.
    pub fn set_tile(&mut self, layer: LayerIndex, frame: FrameIndex, column: u32, row: u32, tile: TileRef) {
        let Some(tileset) = self.tilemap_of(layer).and_then(|id| self.tileset(id)) else { return };
        let (columns, rows) = self.grid_size(tileset);
        self.grid_mut(layer, frame, columns, rows).set(column as i32, row as i32, tile.to_colour());
    }

    /// A tilemap cel's grid, made empty if the cel was.
    fn grid_mut(
        &mut self,
        LayerIndex(layer): LayerIndex,
        FrameIndex(frame): FrameIndex,
        columns: u32,
        rows: u32,
    ) -> &mut Pixels {
        let id = match self.layers[layer].cels[frame] {
            Some(cel) => cel.image,
            None => {
                let id = self.add_image(Pixels::new(columns, rows));
                self.layers[layer].cels[frame] = Some(Cel::showing(id));
                id
            }
        };
        Arc::make_mut(self.images.get_mut(&id).expect("every cel points at a pooled image"))
    }

    /// A tilemap cel drawn out as pixels at canvas size.
    fn render_tilemap(&self, layer: LayerIndex, frame: FrameIndex, tileset: &Tileset) -> Pixels {
        let mut out = Pixels::new(self.width, self.height);
        let (columns, rows) = self.grid_size(tileset);
        for row in 0..rows {
            for column in 0..columns {
                let reference = self.tile_at(layer, frame, column, row);
                if let Some(tile) = tileset.tiles.get(reference.index()).filter(|_| !reference.is_empty()) {
                    let (x, y) = ((column * tileset.tile_width) as i32, (row * tileset.tile_height) as i32);
                    out.paste(&reference.orient(tile), x, y);
                }
            }
        }
        out
    }

    /// What a cel shows, at canvas size: its image, or for a tilemap its
    /// tiles drawn out. An empty cel is transparent.
    pub fn cel_pixels(&self, layer: LayerIndex, frame: FrameIndex) -> Pixels {
        match self.tilemap_of(layer).and_then(|id| self.tileset(id)) {
            Some(tileset) => self.render_tilemap(layer, frame, tileset),
            None => self.cel_image(layer, frame).cloned().unwrap_or_else(|| Pixels::new(self.width, self.height)),
        }
    }

    /// Makes a cel show `pixels`. For a tilemap, each cell whose pixels
    /// changed edits its tile (every placement of it changes too), and a
    /// painted empty cell gets a new or matching tile.
    pub fn write_cel_pixels(&mut self, layer: LayerIndex, frame: FrameIndex, pixels: Pixels) {
        let Some(id) = self.tilemap_of(layer) else {
            *self.cel_image_mut(layer, frame) = pixels;
            return;
        };
        let Some(tileset) = self.tileset(id).cloned() else { return };
        let before = self.render_tilemap(layer, frame, &tileset);
        let (columns, rows) = self.grid_size(&tileset);
        for row in 0..rows {
            for column in 0..columns {
                let cell = Rect::new(
                    (column * tileset.tile_width) as i32,
                    (row * tileset.tile_height) as i32,
                    tileset.tile_width,
                    tileset.tile_height,
                );
                // Only the on-canvas part of an edge cell can have changed.
                let visible = cell.intersect(&self.bounds());
                let now = pixels.crop(visible);
                if now == before.crop(visible) {
                    continue;
                }
                self.write_cell(layer, frame, (column, row), &now, &tileset);
            }
        }
    }

    fn write_cell(
        &mut self,
        layer: LayerIndex,
        frame: FrameIndex,
        (column, row): (u32, u32),
        now: &Pixels,
        shape: &Tileset,
    ) {
        let reference = self.tile_at(layer, frame, column, row);
        let id = shape.id;
        // The cell may hang off the canvas edge: keep the off-canvas part
        // of the tile and replace only what's visible.
        let mut shown = match self.tileset(id).and_then(|t| t.tiles.get(reference.index())) {
            Some(tile) if !reference.is_empty() => reference.orient(tile),
            _ => Pixels::new(shape.tile_width, shape.tile_height),
        };
        shown.paste(now, 0, 0);
        if shown.is_blank() {
            self.set_tile(layer, frame, column, row, TileRef::EMPTY);
            return;
        }
        let Some(tileset) = self.tileset_mut(id) else { return };
        if reference.is_empty() {
            let index = tileset.find_or_add(shown);
            self.set_tile(layer, frame, column, row, TileRef::new(index, false, false));
        } else {
            tileset.tiles[reference.index()] = Arc::new(reference.unorient(&shown));
        }
    }

    /// Turns an image layer into a tilemap layer with a new tileset, cut
    /// from its art: identical tiles are stored once. Clones stay clones.
    pub fn convert_to_tilemap(&mut self, layer: LayerIndex, tile_width: u32, tile_height: u32) -> TilesetId {
        let name = format!("{} tiles", self.layers[layer.0].name);
        let id = self.add_tileset(&name, tile_width, tile_height);
        let mut tileset = self.tileset(id).cloned().expect("just added");
        let (columns, rows) = self.grid_size(&tileset);
        let mut grids = std::collections::BTreeMap::new();
        for frame in 0..self.frames.len() {
            let Some(cel) = self.layers[layer.0].cels[frame] else { continue };
            if grids.contains_key(&cel.image) {
                continue;
            }
            let art = self.images[&cel.image].clone();
            let mut grid = Pixels::new(columns, rows);
            for (column, row) in Rect::new(0, 0, columns, rows).points() {
                let cell = Rect::new(column * tile_width as i32, row * tile_height as i32, tile_width, tile_height);
                let index = tileset.find_or_add(art.crop(cell));
                grid.set(column, row, TileRef::new(index, false, false).to_colour());
            }
            grids.insert(cel.image, self.add_image(grid));
        }
        *self.tileset_mut(id).expect("just added") = tileset;
        let target = &mut self.layers[layer.0];
        target.kind = LayerKind::Tilemap { tileset: id };
        for cel in target.cels.iter_mut().flatten() {
            cel.image = grids[&cel.image];
        }
        self.collect_unused_images();
        id
    }

    /// The pool images that are tilemap grids rather than pictures, which
    /// whole-sprite pixel operations must leave alone.
    pub fn grid_images(&self) -> HashSet<super::sprite::ImageId> {
        self.layers
            .iter()
            .filter(|layer| matches!(layer.kind, LayerKind::Tilemap { .. }))
            .flat_map(|layer| layer.cels.iter().flatten().map(|cel| cel.image))
            .collect()
    }

    /// Fits every tilemap's grid to a resized canvas. Offsets move the map
    /// by whole cells (rounded down), since tiles can't sit between cells.
    pub(super) fn resize_tilemaps(&mut self, offset_x: i32, offset_y: i32) {
        for layer in 0..self.layers.len() {
            let Some(tileset) = self.tilemap_of(LayerIndex(layer)).and_then(|id| self.tileset(id)).cloned() else {
                continue;
            };
            let (columns, rows) = self.grid_size(&tileset);
            let shift =
                (offset_x.div_euclid(tileset.tile_width as i32), offset_y.div_euclid(tileset.tile_height as i32));
            let images: HashSet<_> = self.layers[layer].cels.iter().flatten().map(|cel| cel.image).collect();
            for id in images {
                let grid = &self.images[&id];
                self.images.insert(id, Arc::new(grid.resized_canvas(columns, rows, shift.0, shift.1)));
            }
        }
    }

    pub(super) fn validate_tilemaps(&self) -> Result<(), String> {
        for tileset in &self.tilesets {
            if !(1..=MAX_TILE_SIDE).contains(&tileset.tile_width) || !(1..=MAX_TILE_SIDE).contains(&tileset.tile_height)
            {
                return Err(format!("tileset \"{}\" has an impossible tile size", tileset.name));
            }
            let wrong =
                tileset.tiles.iter().any(|t| (t.width(), t.height()) != (tileset.tile_width, tileset.tile_height));
            if tileset.tiles.is_empty() || wrong || tileset.tiles.len() > MAX_TILES {
                return Err(format!("tileset \"{}\" has tiles of the wrong size", tileset.name));
            }
        }
        for (index, layer) in self.layers.iter().enumerate() {
            let LayerKind::Tilemap { tileset } = layer.kind else { continue };
            let tileset =
                self.tileset(tileset).ok_or_else(|| format!("layer \"{}\" uses a missing tileset", layer.name))?;
            let size = self.grid_size(tileset);
            for frame in 0..self.frames.len() {
                let Some(grid) = self.cel_image(LayerIndex(index), FrameIndex(frame)) else { continue };
                if (grid.width(), grid.height()) != size {
                    return Err(format!("layer \"{}\" has a tile grid of the wrong size", layer.name));
                }
                if grid.colours().iter().any(|&c| TileRef::from_colour(c).index() >= tileset.tiles.len()) {
                    return Err(format!("layer \"{}\" uses a tile its tileset doesn't have", layer.name));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = Rgba::rgb(255, 0, 0);
    const BLUE: Rgba = Rgba::rgb(0, 0, 255);

    /// An 8×4 image layer filled red: as 4×4 tiles, two identical ones.
    fn two_red_tiles() -> Sprite {
        let mut sprite = Sprite::new(8, 4);
        let art = sprite.cel_image_mut(LayerIndex(0), FrameIndex(0));
        for (x, y) in Rect::new(0, 0, 8, 4).points() {
            art.set(x, y, RED);
        }
        sprite
    }

    #[test]
    fn converting_art_to_a_tilemap_stores_each_distinct_tile_once() {
        let mut sprite = two_red_tiles();
        let id = sprite.convert_to_tilemap(LayerIndex(0), 4, 4);
        assert_eq!(sprite.tileset(id).unwrap().tiles.len(), 2, "the empty tile and one red tile");
        assert_eq!(sprite.tile_at(LayerIndex(0), FrameIndex(0), 1, 0).index(), 1);
        assert_eq!(sprite.cel_pixels(LayerIndex(0), FrameIndex(0)).get(6, 2), RED, "it still looks the same");
        assert_eq!(sprite.validate(), Ok(()));
    }

    #[test]
    fn painting_one_placement_of_a_tile_repaints_every_placement() {
        let mut sprite = two_red_tiles();
        sprite.convert_to_tilemap(LayerIndex(0), 4, 4);
        let mut edited = sprite.cel_pixels(LayerIndex(0), FrameIndex(0));
        edited.set(1, 1, BLUE);
        sprite.write_cel_pixels(LayerIndex(0), FrameIndex(0), edited);
        let shown = sprite.cel_pixels(LayerIndex(0), FrameIndex(0));
        assert_eq!((shown.get(1, 1), shown.get(5, 1)), (BLUE, BLUE), "both copies of the tile changed");
    }

    #[test]
    fn painting_an_empty_cell_makes_a_new_tile_and_erasing_it_clears_the_cell() {
        let mut sprite = Sprite::new(8, 4);
        let id = sprite.add_tileset("ground", 4, 4);
        sprite.layers[0].kind = LayerKind::Tilemap { tileset: id };
        let mut art = sprite.cel_pixels(LayerIndex(0), FrameIndex(0));
        art.set(6, 0, BLUE);
        sprite.write_cel_pixels(LayerIndex(0), FrameIndex(0), art);
        assert_eq!(sprite.tileset(id).unwrap().tiles.len(), 2);
        assert_eq!(sprite.tile_at(LayerIndex(0), FrameIndex(0), 1, 0).index(), 1);
        sprite.write_cel_pixels(LayerIndex(0), FrameIndex(0), Pixels::new(8, 4));
        assert!(sprite.tile_at(LayerIndex(0), FrameIndex(0), 1, 0).is_empty());
    }

    #[test]
    fn flipped_tiles_draw_mirrored_and_edits_go_back_unflipped() {
        let mut sprite = Sprite::new(8, 4);
        let id = sprite.add_tileset("arrows", 4, 4);
        sprite.layers[0].kind = LayerKind::Tilemap { tileset: id };
        let mut arrow = Pixels::new(4, 4);
        arrow.set(0, 0, RED);
        let index = sprite.tileset_mut(id).unwrap().find_or_add(arrow);
        sprite.set_tile(LayerIndex(0), FrameIndex(0), 0, 0, TileRef::new(index, false, false));
        sprite.set_tile(LayerIndex(0), FrameIndex(0), 1, 0, TileRef::new(index, true, false));
        let shown = sprite.cel_pixels(LayerIndex(0), FrameIndex(0));
        assert_eq!((shown.get(0, 0), shown.get(7, 0)), (RED, RED), "the second copy is mirrored");
        let mut edited = shown;
        edited.set(7, 3, BLUE); // bottom-right of the mirrored copy
        sprite.write_cel_pixels(LayerIndex(0), FrameIndex(0), edited);
        assert_eq!(sprite.tileset(id).unwrap().tiles[index as usize].get(0, 3), BLUE, "stored at the tile's own left");
        assert_eq!(sprite.cel_pixels(LayerIndex(0), FrameIndex(0)).get(3, 3), Rgba::TRANSPARENT);
        assert_eq!(sprite.cel_pixels(LayerIndex(0), FrameIndex(0)).get(0, 3), BLUE);
    }

    #[test]
    fn orient_and_unorient_undo_each_other_for_every_flip() {
        let mut tile = Pixels::new(3, 3);
        tile.set(0, 0, RED);
        tile.set(2, 1, BLUE);
        for flags in 0..8u32 {
            let reference = TileRef((flags << 29) | 1);
            assert_eq!(reference.unorient(&reference.orient(&tile)), tile, "flags {flags:03b}");
        }
    }

    #[test]
    fn a_partial_cell_at_the_canvas_edge_keeps_its_hidden_part() {
        let mut sprite = Sprite::new(6, 4); // the second column of 4-wide cells is cut off
        let id = sprite.add_tileset("edge", 4, 4);
        sprite.layers[0].kind = LayerKind::Tilemap { tileset: id };
        let mut tile = Pixels::new(4, 4);
        tile.set(3, 0, RED); // off-canvas when placed in the second column
        tile.set(0, 0, RED);
        let index = sprite.tileset_mut(id).unwrap().find_or_add(tile);
        sprite.set_tile(LayerIndex(0), FrameIndex(0), 1, 0, TileRef::new(index, false, false));
        let mut art = sprite.cel_pixels(LayerIndex(0), FrameIndex(0));
        art.set(5, 1, BLUE);
        sprite.write_cel_pixels(LayerIndex(0), FrameIndex(0), art);
        let stored = &sprite.tileset(id).unwrap().tiles[index as usize];
        assert_eq!((stored.get(1, 1), stored.get(3, 0)), (BLUE, RED));
    }

    #[test]
    fn tilemaps_follow_a_resized_canvas_by_whole_cells() {
        let mut sprite = two_red_tiles();
        sprite.convert_to_tilemap(LayerIndex(0), 4, 4);
        sprite.resize_canvas(12, 8, 4, 4);
        assert_eq!(sprite.validate(), Ok(()));
        let shown = sprite.cel_pixels(LayerIndex(0), FrameIndex(0));
        assert_eq!((shown.get(4, 4), shown.get(0, 0)), (RED, Rgba::TRANSPARENT));
    }

    #[test]
    fn validation_catches_a_tile_the_tileset_lacks() {
        let mut sprite = two_red_tiles();
        sprite.convert_to_tilemap(LayerIndex(0), 4, 4);
        sprite.set_tile(LayerIndex(0), FrameIndex(0), 0, 0, TileRef::new(40, false, false));
        assert!(sprite.validate().unwrap_err().contains("doesn't have"));
    }
}

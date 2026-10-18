//! Exporting a tileset and the maps that use it.
//!
//! The image holds every tile except the empty one, in rows of up to 16.
//! The JSON gives each tilemap layer's grid per frame. Its numbers follow
//! Tiled's convention: 0 is an empty cell, 1 is the first tile in the
//! image, and the top three bits are the horizontal, vertical and diagonal
//! flips (the same bits Aseprite and Spry use). So a map drops straight
//! into most level editors and engines.

use crate::editor::sprite::LayerKind;
use crate::editor::tilemap::{TileRef, TilesetId};
use crate::editor::{FrameIndex, LayerIndex, Pixels, Sprite};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::Path;

const COLUMNS: u32 = 16;

pub struct TilesetExport {
    pub image: Pixels,
    pub json: Value,
}

pub fn build(sprite: &Sprite, id: TilesetId, image_name: &str) -> Result<TilesetExport> {
    let tileset = sprite.tileset(id).with_context(|| format!("there's no tileset {}", id.0))?;
    let tiles = &tileset.tiles[1..];
    let columns = (tiles.len() as u32).clamp(1, COLUMNS);
    let rows = (tiles.len() as u32).div_ceil(columns).max(1);
    let mut image = Pixels::new(columns * tileset.tile_width, rows * tileset.tile_height);
    for (index, tile) in tiles.iter().enumerate() {
        let (column, row) = (index as u32 % columns, index as u32 / columns);
        image.paste(tile, (column * tileset.tile_width) as i32, (row * tileset.tile_height) as i32);
    }
    let (grid_columns, grid_rows) = sprite.grid_size(tileset);
    let layers: Vec<Value> = sprite
        .layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| layer.kind == LayerKind::Tilemap { tileset: id })
        .map(|(index, layer)| {
            let frames: Vec<Value> = (0..sprite.frame_count())
                .map(|frame| {
                    let data: Vec<u32> = (0..grid_rows)
                        .flat_map(|row| (0..grid_columns).map(move |column| (column, row)))
                        .map(|(column, row)| sprite.tile_at(LayerIndex(index), FrameIndex(frame), column, row))
                        .map(|TileRef(bits)| bits)
                        .collect();
                    json!({ "frame": frame, "data": data })
                })
                .collect();
            json!({ "name": layer.name, "columns": grid_columns, "rows": grid_rows, "frames": frames })
        })
        .collect();
    let json = json!({
        "tileset": {
            "name": tileset.name,
            "image": image_name,
            "tile_width": tileset.tile_width,
            "tile_height": tileset.tile_height,
            "tile_count": tiles.len(),
            "columns": columns,
        },
        "layers": layers,
        "meta": { "app": "Spry", "numbering": "0 = empty, 1 = first tile in the image; bits 31/30/29 flip x/y/diagonal" },
    });
    Ok(TilesetExport { image, json })
}

/// Writes `<stem>.png` and `<stem>.json`. With no `id`, the tileset of the
/// first tilemap layer.
pub fn export(png_path: &Path, sprite: &Sprite, id: Option<TilesetId>) -> Result<TilesetExport> {
    let id = id
        .or_else(|| {
            sprite.layers.iter().find_map(|layer| match layer.kind {
                LayerKind::Tilemap { tileset } => Some(tileset),
                _ => None,
            })
        })
        .context("this sprite has no tilemap layer, so there's no tileset to export")?;
    let image_name = png_path.file_name().and_then(|n| n.to_str()).unwrap_or("tiles.png");
    let built = build(sprite, id, image_name)?;
    std::fs::write(png_path, super::encode_png(&built.image)?)
        .with_context(|| format!("writing {}", png_path.display()))?;
    let json_path = png_path.with_extension("json");
    std::fs::write(&json_path, serde_json::to_string_pretty(&built.json)?)
        .with_context(|| format!("writing {}", json_path.display()))?;
    Ok(built)
}

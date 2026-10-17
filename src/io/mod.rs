//! Files: the `.spry` format, image formats, and every export. This is the
//! only place Spry touches the filesystem for documents.

pub mod raster_formats;
pub mod sheet;
pub mod spry_file;

use crate::editor::{Document, Pixels, Sprite};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub use raster_formats::{decode_gif, decode_image, encode_gif, encode_png};
pub use sheet::{SheetLayout, SheetOptions, SpriteSheet};

/// Opens anything Spry can read: a `.spry` document, or a PNG/GIF, which
/// becomes a new document (one frame per GIF frame).
pub fn open(path: &Path) -> Result<Document> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    match extension_of(path).as_str() {
        "spry" => spry_file::decode(&bytes).with_context(|| format!("opening {}", path.display())),
        "gif" => document_from_animation(decode_gif(&bytes)?),
        _ => Ok(document_from_image(decode_image(&bytes)?)),
    }
}

pub fn save(path: &Path, document: &Document) -> Result<()> {
    spry_file::save(path, document)
}

fn extension_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

pub fn document_from_image(image: Pixels) -> Document {
    let mut sprite = Sprite::new(image.width(), image.height());
    *sprite.cel_image_mut(0, 0) = image;
    Document::new(sprite)
}

pub fn document_from_animation(frames: Vec<(Pixels, u32)>) -> Result<Document> {
    let (first, _) = frames.first().context("the animation has no frames")?;
    let mut sprite = Sprite::new(first.width(), first.height());
    for (index, (image, delay)) in frames.into_iter().enumerate() {
        if index > 0 {
            sprite.insert_frame(index, delay.max(10));
        }
        sprite.frames[index].duration_ms = delay.max(10);
        *sprite.cel_image_mut(0, index) = image;
    }
    Ok(Document::new(sprite))
}

/// A sprite sheet cut into frames becomes a one-layer animation.
pub fn document_from_sheet(sheet: &Pixels, cell_width: u32, cell_height: u32) -> Result<Document> {
    let cells = sheet::slice_grid(sheet, cell_width, cell_height);
    anyhow::ensure!(
        !cells.is_empty(),
        "a {cell_width}×{cell_height} grid doesn't fit this {}×{} image",
        sheet.width(),
        sheet.height()
    );
    document_from_animation(cells.into_iter().map(|cell| (cell, crate::editor::sprite::DEFAULT_FRAME_MS)).collect())
}

pub fn export_png(path: &Path, sprite: &Sprite, frame: usize, scale: u32) -> Result<()> {
    let bytes = encode_png(&sprite.composite(frame).scaled_up(scale))?;
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

pub fn export_gif(path: &Path, sprite: &Sprite, tag: Option<&str>, scale: u32) -> Result<()> {
    let tag = match tag {
        Some(name) => Some(
            sprite.tags.iter().find(|t| t.name == name).with_context(|| format!("there's no tag called \"{name}\""))?,
        ),
        None => None,
    };
    let frames = raster_formats::animation_frames(sprite, tag, scale);
    std::fs::write(path, encode_gif(&frames)?).with_context(|| format!("writing {}", path.display()))
}

/// Writes `name_000.png`, `name_001.png`, … into `directory` and returns
/// the paths.
pub fn export_png_sequence(directory: &Path, name: &str, sprite: &Sprite, scale: u32) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(directory).with_context(|| format!("creating {}", directory.display()))?;
    (0..sprite.frame_count())
        .map(|frame| {
            let path = directory.join(format!("{name}_{frame:03}.png"));
            export_png(&path, sprite, frame, scale)?;
            Ok(path)
        })
        .collect()
}

/// Writes `<stem>.png` and `<stem>.json` side by side. `png_path` names the
/// image, and the JSON goes next to it.
pub fn export_sheet(png_path: &Path, sprite: &Sprite, title: &str, options: &SheetOptions) -> Result<SpriteSheet> {
    let image_name = png_path.file_name().and_then(|n| n.to_str()).unwrap_or("sheet.png").to_string();
    let options = SheetOptions { image_name, ..options.clone() };
    let built = sheet::build(sprite, title, &options).map_err(anyhow::Error::msg)?;
    std::fs::write(png_path, encode_png(&built.image)?).with_context(|| format!("writing {}", png_path.display()))?;
    let json_path = png_path.with_extension("json");
    std::fs::write(&json_path, serde_json::to_string_pretty(&built.json)?)
        .with_context(|| format!("writing {}", json_path.display()))?;
    Ok(built)
}

//! Files: the `.spry` format, image formats, and every export. This is the
//! only place Spry touches the filesystem for documents.

pub mod aseprite;
pub mod raster_formats;
pub mod sheet;
pub mod spry_file;

use crate::editor::{limits, Document, Palette, Pixels, Sprite};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub use raster_formats::{decode_apng, decode_gif, decode_image, encode_apng, encode_gif, encode_png, encode_webp};
pub use sheet::{SheetLayout, SheetOptions, SpriteSheet};

/// Opens anything Spry can read: a `.spry` document, or an Aseprite file,
/// PNG or GIF, which becomes a new document (one frame per GIF frame).
pub fn open(path: &Path) -> Result<Document> {
    open_with_notices(path).map(|(document, _)| document)
}

/// Like `open`, plus notes on anything that couldn't come across exactly
/// (so far only Aseprite files have any).
pub fn open_with_notices(path: &Path) -> Result<(Document, Vec<String>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let opening = || format!("opening {}", path.display());
    match extension_of(path).as_str() {
        "spry" => Ok((spry_file::decode(&bytes).with_context(opening)?, Vec::new())),
        "aseprite" | "ase" => {
            let import = aseprite::decode(&bytes).with_context(opening)?;
            Ok((import.document, import.notices))
        }
        "gif" => Ok((document_from_animation(decode_gif(&bytes)?)?, Vec::new())),
        _ => Ok((document_from_image(decode_image(&bytes)?)?, Vec::new())),
    }
}

pub fn save(path: &Path, document: &Document) -> Result<()> {
    spry_file::save(path, document)
}

fn extension_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

pub fn document_from_image(image: Pixels) -> Result<Document> {
    limits::check_canvas_size(image.width(), image.height()).map_err(anyhow::Error::msg)?;
    let mut sprite = Sprite::new(image.width(), image.height());
    *sprite.cel_image_mut(0, 0) = image;
    Ok(Document::new(sprite))
}

pub fn document_from_animation(frames: Vec<(Pixels, u32)>) -> Result<Document> {
    let (first, _) = frames.first().context("the animation has no frames")?;
    limits::check_canvas_size(first.width(), first.height()).map_err(anyhow::Error::msg)?;
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

/// Reads a `.hex` or `.gpl` palette, named after its file.
pub fn load_palette(path: &Path) -> Result<Palette> {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Palette").to_string();
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Palette::parse(&name, &text).map_err(anyhow::Error::msg)
}

/// Writes a palette as GIMP `.gpl` when the path says so, otherwise as a
/// Lospec-style `.hex` list.
pub fn save_palette(path: &Path, palette: &Palette) -> Result<()> {
    let text = if extension_of(path) == "gpl" { palette.to_gpl_file() } else { palette.to_hex_file() };
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// Opens a sprite sheet image as an animation, finding the cell size from
/// the transparent gutters between frames, or using `fallback_cell` when
/// there are none. Returns the document and the cell size it used.
pub fn import_sheet(path: &Path, fallback_cell: (u32, u32)) -> Result<(Document, (u32, u32))> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let sheet = decode_image(&bytes)?;
    let cell = sheet::detect_grid(&sheet).unwrap_or(fallback_cell);
    Ok((document_from_sheet(&sheet, cell.0, cell.1)?, cell))
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
    limits::check_export_size(sprite.width, sprite.height, scale).map_err(anyhow::Error::msg)?;
    let bytes = encode_png(&sprite.composite(frame).scaled_up(scale))?;
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

pub fn export_gif(path: &Path, sprite: &Sprite, tag: Option<&str>, scale: u32) -> Result<()> {
    limits::check_export_size(sprite.width, sprite.height, scale).map_err(anyhow::Error::msg)?;
    let frames = raster_formats::animation_frames(sprite, find_tag(sprite, tag)?, scale);
    std::fs::write(path, encode_gif(&frames)?).with_context(|| format!("writing {}", path.display()))
}

pub fn export_apng(path: &Path, sprite: &Sprite, tag: Option<&str>, scale: u32) -> Result<()> {
    limits::check_export_size(sprite.width, sprite.height, scale).map_err(anyhow::Error::msg)?;
    let frames = raster_formats::animation_frames(sprite, find_tag(sprite, tag)?, scale);
    std::fs::write(path, encode_apng(&frames)?).with_context(|| format!("writing {}", path.display()))
}

pub fn export_webp(path: &Path, sprite: &Sprite, frame: usize, scale: u32) -> Result<()> {
    limits::check_export_size(sprite.width, sprite.height, scale).map_err(anyhow::Error::msg)?;
    let bytes = encode_webp(&sprite.composite(frame).scaled_up(scale))?;
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn find_tag<'a>(sprite: &'a Sprite, name: Option<&str>) -> Result<Option<&'a crate::editor::Tag>> {
    name.map(|name| {
        sprite.tags.iter().find(|t| t.name == name).with_context(|| format!("there's no tag called \"{name}\""))
    })
    .transpose()
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

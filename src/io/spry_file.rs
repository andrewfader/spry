//! The `.spry` file: gzip-compressed JSON.
//!
//! JSON keeps the format open, so anyone can `zcat` a file and see layers,
//! tags, notes and versions. One problem with that is version history: every
//! version holds a whole sprite, and written naively the same unchanged
//! image would be saved once per version. So images are pooled. Each
//! distinct image is stored once, in `pool`, and sprites refer to pool
//! entries by index.

use crate::editor::Document;
use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{Read, Write};

pub const FORMAT: &str = "spry";
pub const FORMAT_VERSION: u64 = 1;

pub fn encode(document: &Document) -> Result<Vec<u8>> {
    let mut tree = serde_json::to_value(document).context("serialising the document")?;
    let mut pool = ImagePool::default();
    for sprite in sprites_in(&mut tree) {
        pool.lift_images_out_of(sprite);
    }
    let file = json!({ "format": FORMAT, "format_version": FORMAT_VERSION, "pool": pool.images, "document": tree });
    let mut gzip = GzEncoder::new(Vec::new(), flate2::Compression::default());
    serde_json::to_writer(&mut gzip, &file)?;
    Ok(gzip.finish()?)
}

/// The largest a `.spry` file may grow when unzipped. Real documents are a
/// tiny fraction of this. The cap stops a crafted "zip bomb" from filling
/// memory.
const MAX_UNZIPPED_BYTES: u64 = 1 << 30;

pub fn decode(bytes: &[u8]) -> Result<Document> {
    let mut text = String::new();
    GzDecoder::new(bytes)
        .take(MAX_UNZIPPED_BYTES)
        .read_to_string(&mut text)
        .context("this is not a .spry file (it isn't gzip-compressed)")?;
    let mut file: Value = serde_json::from_str(&text).context("the .spry file is damaged (invalid JSON)")?;
    if file["format"] != FORMAT {
        bail!("this is not a .spry file");
    }
    let version = file["format_version"].as_u64().unwrap_or(0);
    if version > FORMAT_VERSION {
        bail!("this file was made by a newer Spry (format {version}); please update");
    }
    let pool = file["pool"].as_array().cloned().unwrap_or_default();
    let mut tree = file["document"].take();
    for sprite in sprites_in(&mut tree) {
        put_images_back(sprite, &pool)?;
    }
    let document: Document =
        serde_json::from_value(tree).context("the .spry file is damaged (unexpected structure)")?;
    document.validate().map_err(|why| anyhow::anyhow!("the .spry file is damaged: {why}"))?;
    Ok(document)
}

pub fn save(path: &std::path::Path, document: &Document) -> Result<()> {
    let bytes = encode(document)?;
    // Write beside the target, then rename, so a crash mid-save can never
    // destroy the previous good copy.
    let temporary = path.with_extension("spry.saving");
    std::fs::File::create(&temporary)
        .and_then(|mut file| file.write_all(&bytes).and_then(|_| file.sync_all()))
        .with_context(|| format!("writing {}", temporary.display()))?;
    std::fs::rename(&temporary, path).with_context(|| format!("saving {}", path.display()))
}

pub fn load(path: &std::path::Path) -> Result<Document> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    decode(&bytes).with_context(|| format!("opening {}", path.display()))
}

/// The current sprite and every version's sprite, as JSON objects.
fn sprites_in(tree: &mut Value) -> Vec<&mut Map<String, Value>> {
    let Some(document) = tree.as_object_mut() else { return Vec::new() };
    let mut sprites = Vec::new();
    let (sprite, versions) = {
        let mut sprite = None;
        let mut versions = None;
        for (key, value) in document.iter_mut() {
            match key.as_str() {
                "sprite" => sprite = value.as_object_mut(),
                "versions" => versions = value.as_array_mut(),
                _ => {}
            }
        }
        (sprite, versions)
    };
    sprites.extend(sprite);
    for version in versions.into_iter().flatten() {
        sprites.extend(version.get_mut("sprite").and_then(Value::as_object_mut));
    }
    sprites
}

#[derive(Default)]
struct ImagePool {
    images: Vec<Value>,
    index_of: HashMap<String, usize>,
}

impl ImagePool {
    fn lift_images_out_of(&mut self, sprite: &mut Map<String, Value>) {
        let Some(images) = sprite.get_mut("images").and_then(Value::as_object_mut) else { return };
        for image in images.values_mut() {
            let key = image.to_string();
            let next = self.images.len();
            let index = *self.index_of.entry(key).or_insert(next);
            if index == next {
                self.images.push(image.take());
            }
            *image = json!(index);
        }
    }
}

fn put_images_back(sprite: &mut Map<String, Value>, pool: &[Value]) -> Result<()> {
    let Some(images) = sprite.get_mut("images").and_then(Value::as_object_mut) else { return Ok(()) };
    for image in images.values_mut() {
        let index = image.as_u64().context("an image reference is not a pool index")? as usize;
        *image = pool.get(index).cloned().with_context(|| format!("image {index} is missing from the pool"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Command, Editor, Ink, Rgba};

    #[test]
    fn documents_round_trip_exactly() {
        let mut editor = Editor::new(8, 8);
        editor.execute(Command::Fill { at: (0, 0), ink: Ink::colour(Rgba::rgb(9, 8, 7)), contiguous: true }).unwrap();
        editor.execute(Command::CommitVersion { message: "base".into() }).unwrap();
        editor.execute(Command::SetNotes { text: "hero idle".into() }).unwrap();
        let bytes = encode(&editor.document).unwrap();
        assert_eq!(decode(&bytes).unwrap(), editor.document);
    }

    #[test]
    fn unchanged_images_are_stored_once_across_versions() {
        let mut editor = Editor::new(16, 16);
        editor.execute(Command::Fill { at: (0, 0), ink: Ink::colour(Rgba::BLACK), contiguous: true }).unwrap();
        for n in 0..5 {
            editor.execute(Command::SetNotes { text: format!("v{n}") }).unwrap();
            editor.execute(Command::CommitVersion { message: format!("v{n}") }).unwrap();
        }
        let mut text = String::new();
        GzDecoder::new(&encode(&editor.document).unwrap()[..]).read_to_string(&mut text).unwrap();
        let file: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(file["pool"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn rejects_files_that_are_not_spry() {
        assert!(decode(b"PNG...").is_err());
        let mut gzip = GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(br#"{"format":"aseprite"}"#).unwrap();
        assert!(decode(&gzip.finish().unwrap()).unwrap_err().to_string().contains("not a .spry"));
    }
}

//! Automatic sprite sheets, out and in.
//!
//! **Out:** every frame is composited, optionally trimmed to its visible
//! pixels, deduplicated, laid out (strip, grid or packed), and described in
//! Aseprite's JSON "hash" format, which Godot, Unity, Phaser, Defold and
//! Love2D importers already understand. Spry adds `meta.notes`, so art
//! direction travels with the asset.
//!
//! **In:** a sheet PNG is sliced back into frames, by a cell size you give or
//! by finding the transparent gutters between sprites.

use crate::editor::limits;
use crate::editor::playback::play_order;
use crate::editor::{LoopDirection, Pixels, Rect, Sprite};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetLayout {
    Horizontal,
    Vertical,
    /// A grid with a fixed column count, or roughly square when `None`.
    Grid {
        columns: Option<u32>,
    },
    /// Shelf-packed by height for the smallest sheet. Pairs well with trim.
    Packed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SheetOptions {
    pub layout: SheetLayout,
    /// Empty pixels between neighbouring frames.
    pub padding: u32,
    /// Empty pixels around the sheet's edge.
    pub border: u32,
    pub trim: bool,
    pub merge_duplicates: bool,
    /// Only frames in this tag, or every frame.
    pub tag: Option<String>,
    pub scale: u32,
    /// The image filename written into `meta.image`.
    pub image_name: String,
}

impl Default for SheetOptions {
    fn default() -> Self {
        SheetOptions {
            layout: SheetLayout::Grid { columns: None },
            padding: 0,
            border: 0,
            trim: false,
            merge_duplicates: false,
            tag: None,
            scale: 1,
            image_name: "sheet.png".into(),
        }
    }
}

/// Where one animation frame landed on the sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedFrame {
    pub frame: usize,
    /// The rectangle on the sheet.
    pub sheet_rect: Rect,
    /// Which part of the full frame it holds (smaller than the frame when
    /// trimmed).
    pub source_rect: Rect,
    pub duration_ms: u32,
}

pub struct SpriteSheet {
    pub image: Pixels,
    pub frames: Vec<PlacedFrame>,
    pub json: Value,
}

impl SpriteSheet {
    /// How many distinct images the sheet holds (fewer than frames when
    /// duplicates were merged).
    pub fn distinct_images(&self) -> usize {
        let mut rects: Vec<Rect> = self.frames.iter().map(|f| f.sheet_rect).collect();
        rects.sort_by_key(|r| (r.x, r.y));
        rects.dedup();
        rects.len()
    }
}

struct Cutout {
    pixels: Pixels,
    source_rect: Rect,
}

pub const MAX_PADDING: u32 = 64;

pub fn build(sprite: &Sprite, title: &str, options: &SheetOptions) -> Result<SpriteSheet, String> {
    let scale = options.scale.max(1);
    limits::check_export_size(sprite.width, sprite.height, scale)?;
    if options.padding > MAX_PADDING {
        return Err(format!("padding goes up to {MAX_PADDING} pixels, not {}", options.padding));
    }
    let frames = frames_to_export(sprite, options.tag.as_deref())?;

    let mut cutouts: Vec<Cutout> = Vec::new();
    let mut cutout_of_frame: Vec<usize> = Vec::new();
    let mut seen: HashMap<Pixels, usize> = HashMap::new();
    for &frame in &frames {
        let full = sprite.composite(frame).scaled_up(scale);
        let cutout = if options.trim {
            let bounds = full.opaque_bounds().unwrap_or(Rect::new(0, 0, 1, 1));
            Cutout { pixels: full.crop(bounds), source_rect: bounds }
        } else {
            Cutout { source_rect: full.bounds(), pixels: full }
        };
        let existing = options.merge_duplicates.then(|| seen.get(&cutout.pixels).copied()).flatten();
        let index = existing.unwrap_or_else(|| {
            seen.insert(cutout.pixels.clone(), cutouts.len());
            cutouts.push(cutout);
            cutouts.len() - 1
        });
        cutout_of_frame.push(index);
    }

    let sizes: Vec<(u32, u32)> = cutouts.iter().map(|c| (c.pixels.width(), c.pixels.height())).collect();
    let (positions, sheet_width, sheet_height) = lay_out(&sizes, options);
    limits::check_export_size(sheet_width, sheet_height, 1).map_err(|_| {
        format!("a {sheet_width}×{sheet_height} sheet is too big; try a grid layout or a smaller scale")
    })?;
    let mut image = Pixels::new(sheet_width, sheet_height);
    for (cutout, &(x, y)) in cutouts.iter().zip(&positions) {
        image.paste(&cutout.pixels, x, y);
    }

    let placed: Vec<PlacedFrame> = frames
        .iter()
        .zip(&cutout_of_frame)
        .map(|(&frame, &index)| {
            let (x, y) = positions[index];
            let cutout = &cutouts[index];
            PlacedFrame {
                frame,
                sheet_rect: Rect::new(x, y, cutout.pixels.width(), cutout.pixels.height()),
                source_rect: cutout.source_rect,
                duration_ms: sprite.frames[frame].duration_ms,
            }
        })
        .collect();
    let json = aseprite_json(sprite, title, options, &placed, (sheet_width, sheet_height));
    Ok(SpriteSheet { image, frames: placed, json })
}

fn frames_to_export(sprite: &Sprite, tag: Option<&str>) -> Result<Vec<usize>, String> {
    match tag {
        None => Ok((0..sprite.frame_count()).collect()),
        Some(name) => {
            let tag = sprite
                .tags
                .iter()
                .find(|t| t.name == name)
                .ok_or_else(|| format!("there's no tag called \"{name}\""))?;
            let mut frames = play_order(sprite, Some(tag));
            frames.sort_unstable();
            frames.dedup();
            Ok(frames)
        }
    }
}

/// Places items of the given sizes. Returns each item's top-left corner and
/// the sheet size.
fn lay_out(sizes: &[(u32, u32)], options: &SheetOptions) -> (Vec<(i32, i32)>, u32, u32) {
    if sizes.is_empty() {
        return (Vec::new(), 1, 1);
    }
    let (pad, border) = (options.padding, options.border);
    let cell_w = sizes.iter().map(|s| s.0).max().unwrap_or(1);
    let cell_h = sizes.iter().map(|s| s.1).max().unwrap_or(1);
    let count = sizes.len() as u32;
    let columns = match options.layout {
        SheetLayout::Horizontal => count,
        SheetLayout::Vertical => 1,
        SheetLayout::Grid { columns: Some(columns) } => columns.clamp(1, count),
        SheetLayout::Grid { columns: None } => (count as f32).sqrt().ceil() as u32,
        SheetLayout::Packed => return shelf_pack(sizes, pad, border),
    };
    let rows = count.div_ceil(columns);
    let positions = (0..count)
        .map(|i| {
            let (column, row) = (i % columns, i / columns);
            ((border + column * (cell_w + pad)) as i32, (border + row * (cell_h + pad)) as i32)
        })
        .collect();
    let width = 2 * border + columns * cell_w + (columns - 1) * pad;
    let height = 2 * border + rows * cell_h + (rows - 1) * pad;
    (positions, width, height)
}

/// Shelf packing: tallest items first, filling rows left to right up to a
/// width chosen to make the sheet roughly square.
fn shelf_pack(sizes: &[(u32, u32)], pad: u32, border: u32) -> (Vec<(i32, i32)>, u32, u32) {
    let area: u64 = sizes.iter().map(|&(w, h)| ((w + pad) * (h + pad)) as u64).sum();
    let widest = sizes.iter().map(|s| s.0).max().unwrap_or(1);
    let target_width = ((area as f64).sqrt().ceil() as u32).max(widest);

    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(sizes[i].1), std::cmp::Reverse(sizes[i].0), i));

    let mut positions = vec![(0, 0); sizes.len()];
    let (mut x, mut y, mut shelf_height, mut used_width) = (0u32, 0u32, 0u32, 0u32);
    for index in order {
        let (w, h) = sizes[index];
        if x > 0 && x + w > target_width {
            y += shelf_height + pad;
            x = 0;
            shelf_height = 0;
        }
        positions[index] = ((border + x) as i32, (border + y) as i32);
        used_width = used_width.max(x + w);
        x += w + pad;
        shelf_height = shelf_height.max(h);
    }
    (positions, used_width + 2 * border, y + shelf_height + 2 * border)
}

fn rect_json(rect: Rect) -> Value {
    json!({ "x": rect.x, "y": rect.y, "w": rect.width, "h": rect.height })
}

fn aseprite_json(
    sprite: &Sprite,
    title: &str,
    options: &SheetOptions,
    placed: &[PlacedFrame],
    size: (u32, u32),
) -> Value {
    let scale = options.scale.max(1);
    let full = Rect::new(0, 0, sprite.width * scale, sprite.height * scale);
    let mut frames = Map::new();
    for frame in placed {
        frames.insert(
            format!("{title} {}.aseprite", frame.frame),
            json!({
                "frame": rect_json(frame.sheet_rect),
                "rotated": false,
                "trimmed": frame.source_rect != full,
                "spriteSourceSize": rect_json(frame.source_rect),
                "sourceSize": { "w": full.width, "h": full.height },
                "duration": frame.duration_ms,
            }),
        );
    }
    let tags: Vec<Value> = sprite
        .tags
        .iter()
        .map(|tag| {
            let direction = match tag.direction {
                LoopDirection::Forward => "forward",
                LoopDirection::Reverse => "reverse",
                LoopDirection::PingPong => "pingpong",
            };
            json!({ "name": tag.name, "from": tag.from, "to": tag.to, "direction": direction })
        })
        .collect();
    let layers: Vec<Value> =
        sprite.layers.iter().map(|l| json!({ "name": l.name, "opacity": l.opacity, "blendMode": "normal" })).collect();
    let slices: Vec<Value> = sprite
        .slices
        .iter()
        .map(|s| json!({ "name": s.name, "color": "#0000ffff", "keys": [{ "frame": 0, "bounds": rect_json(s.bounds) }] }))
        .collect();
    let frame_notes: Map<String, Value> = sprite
        .frames
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.note.is_empty())
        .map(|(i, f)| (i.to_string(), json!(f.note)))
        .collect();
    json!({
        "frames": frames,
        "meta": {
            "app": "https://github.com/spry-editor/spry",
            "version": env!("CARGO_PKG_VERSION"),
            "image": options.image_name,
            "format": "RGBA8888",
            "size": { "w": size.0, "h": size.1 },
            "scale": scale.to_string(),
            "frameTags": tags,
            "layers": layers,
            "slices": slices,
            "notes": {
                "document": sprite.notes,
                "frames": frame_notes,
                "pins": sprite.pins.iter().map(|p| json!({ "x": p.x, "y": p.y, "text": p.text })).collect::<Vec<_>>(),
            },
        }
    })
}

// ----- importing ---------------------------------------------------------

/// Cuts a sheet into `cell_width × cell_height` frames, left to right and top
/// to bottom. Blank cells at the end (an unfilled last row) are dropped.
pub fn slice_grid(sheet: &Pixels, cell_width: u32, cell_height: u32) -> Vec<Pixels> {
    if cell_width == 0 || cell_height == 0 {
        return Vec::new();
    }
    let columns = sheet.width() / cell_width;
    let rows = sheet.height() / cell_height;
    let mut cells: Vec<Pixels> = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .map(|(column, row)| {
            sheet.crop(Rect::new((column * cell_width) as i32, (row * cell_height) as i32, cell_width, cell_height))
        })
        .collect();
    while cells.len() > 1 && cells.last().is_some_and(Pixels::is_blank) {
        cells.pop();
    }
    cells
}

/// Guesses the cell size from transparent gutters: the distance between
/// the starts of runs of non-empty columns (and rows), if they're evenly
/// spaced.
pub fn detect_grid(sheet: &Pixels) -> Option<(u32, u32)> {
    let column_has_ink = |x: i32| (0..sheet.height() as i32).any(|y| !sheet.get(x, y).is_transparent());
    let row_has_ink = |y: i32| (0..sheet.width() as i32).any(|x| !sheet.get(x, y).is_transparent());
    let cell_width = even_spacing((0..sheet.width() as i32).map(column_has_ink).collect(), sheet.width())?;
    let cell_height = even_spacing((0..sheet.height() as i32).map(row_has_ink).collect(), sheet.height())?;
    Some((cell_width, cell_height))
}

fn even_spacing(ink: Vec<bool>, length: u32) -> Option<u32> {
    let starts: Vec<u32> = (0..ink.len()).filter(|&i| ink[i] && (i == 0 || !ink[i - 1])).map(|i| i as u32).collect();
    match starts.len() {
        0 => None,
        1 => Some(length),
        _ => {
            let spacing = starts[1] - starts[0];
            let evenly_spaced = starts.windows(2).all(|w| w[1] - w[0] == spacing);
            (evenly_spaced && length.is_multiple_of(spacing)).then_some(spacing)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Rgba;

    const RED: Rgba = Rgba::rgb(255, 0, 0);

    fn four_frame_sprite() -> Sprite {
        let mut sprite = Sprite::new(8, 8);
        for frame in 1..4 {
            sprite.insert_frame(frame, 100);
        }
        for frame in 0..4 {
            sprite.cel_image_mut(0, frame).set(frame as i32, 0, RED);
        }
        sprite
    }

    #[test]
    fn a_horizontal_strip_is_one_frame_tall() {
        let options = SheetOptions { layout: SheetLayout::Horizontal, ..Default::default() };
        let sheet = build(&four_frame_sprite(), "hero", &options).unwrap();
        assert_eq!((sheet.image.width(), sheet.image.height()), (32, 8));
        assert_eq!(sheet.frames[3].sheet_rect, Rect::new(24, 0, 8, 8));
    }

    #[test]
    fn grids_default_to_roughly_square() {
        let sheet = build(&four_frame_sprite(), "hero", &SheetOptions::default()).unwrap();
        assert_eq!((sheet.image.width(), sheet.image.height()), (16, 16));
    }

    #[test]
    fn padding_and_border_space_out_the_cells() {
        let options = SheetOptions { layout: SheetLayout::Horizontal, padding: 2, border: 1, ..Default::default() };
        let sheet = build(&four_frame_sprite(), "hero", &options).unwrap();
        assert_eq!(sheet.image.width(), 1 + 8 * 4 + 2 * 3 + 1);
        assert_eq!(sheet.frames[1].sheet_rect.x, 11);
    }

    #[test]
    fn trimming_records_where_the_pixels_came_from() {
        let options = SheetOptions { trim: true, layout: SheetLayout::Packed, ..Default::default() };
        let sheet = build(&four_frame_sprite(), "hero", &options).unwrap();
        assert_eq!(sheet.frames[2].source_rect, Rect::new(2, 0, 1, 1));
        assert_eq!(sheet.json["frames"]["hero 2.aseprite"]["trimmed"], true);
        assert_eq!(sheet.json["frames"]["hero 2.aseprite"]["spriteSourceSize"]["x"], 2);
    }

    #[test]
    fn packing_never_overlaps_frames() {
        let options = SheetOptions { trim: true, layout: SheetLayout::Packed, padding: 1, ..Default::default() };
        let mut sprite = four_frame_sprite();
        sprite.cel_image_mut(0, 1).set(7, 7, RED);
        let sheet = build(&sprite, "x", &options).unwrap();
        for (i, a) in sheet.frames.iter().enumerate() {
            for b in &sheet.frames[i + 1..] {
                assert!(a.sheet_rect.intersect(&b.sheet_rect).is_empty(), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn merging_duplicates_points_identical_frames_at_one_image() {
        let mut sprite = four_frame_sprite();
        sprite.clear_cel(0, 3);
        sprite.link_cel(0, 1, 3);
        let options = SheetOptions { merge_duplicates: true, ..Default::default() };
        let sheet = build(&sprite, "hero", &options).unwrap();
        assert_eq!(sheet.distinct_images(), 3);
        assert_eq!(sheet.frames[1].sheet_rect, sheet.frames[3].sheet_rect);
    }

    #[test]
    fn a_tag_limits_the_sheet_to_its_frames() {
        let mut sprite = four_frame_sprite();
        sprite.tags.push(crate::editor::Tag {
            name: "walk".into(),
            from: 1,
            to: 2,
            direction: LoopDirection::PingPong,
        });
        let options = SheetOptions { tag: Some("walk".into()), ..Default::default() };
        let sheet = build(&sprite, "hero", &options).unwrap();
        assert_eq!(sheet.frames.iter().map(|f| f.frame).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(sheet.json["meta"]["frameTags"][0]["direction"], "pingpong");
        assert!(build(&sprite, "hero", &SheetOptions { tag: Some("run".into()), ..Default::default() }).is_err());
    }

    #[test]
    fn scaling_multiplies_every_rectangle() {
        let options = SheetOptions { scale: 3, layout: SheetLayout::Vertical, ..Default::default() };
        let sheet = build(&four_frame_sprite(), "hero", &options).unwrap();
        assert_eq!((sheet.image.width(), sheet.image.height()), (24, 96));
        assert_eq!(sheet.json["meta"]["scale"], "3");
    }

    #[test]
    fn slicing_a_sheet_recovers_its_frames() {
        let sprite = four_frame_sprite();
        let sheet =
            build(&sprite, "hero", &SheetOptions { layout: SheetLayout::Horizontal, ..Default::default() }).unwrap();
        let frames = slice_grid(&sheet.image, 8, 8);
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[2], sprite.composite(2));
    }

    #[test]
    fn grid_detection_finds_cells_between_gutters() {
        let mut sheet = Pixels::new(30, 10);
        for left in [1, 11, 21] {
            for (x, y) in Rect::new(left, 3, 6, 5).points() {
                sheet.set(x, y, RED);
            }
        }
        assert_eq!(detect_grid(&sheet), Some((10, 10)));
        assert_eq!(detect_grid(&Pixels::new(4, 4)), None);
    }
}

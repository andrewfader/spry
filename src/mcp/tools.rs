//! The MCP tool set: Spry's editor verbs, phrased for language models.
//!
//! The same tools serve every LLM: external agents over stdio or HTTP, and
//! the built-in AI chat panel. Each tool is a thin translation from
//! friendly JSON arguments (hex colours, pattern names, optional
//! layer/frame) into editor `Command`s, so an LLM edit is exactly as
//! undoable as a human one.

use super::Workspace;
use crate::editor::BlendMode;
use crate::editor::{Brush, Command, Ink, LoopDirection, Palette, Pattern, PixelValue, Rect, Rgba};
use crate::io::{self, SheetLayout, SheetOptions};
use crate::studio::Studio;
use base64::Engine;
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

/// What a tool hands back: text for the model, and optionally a picture.
#[derive(Debug, Default, PartialEq)]
pub struct ToolOutput {
    pub text: String,
    pub png: Option<Vec<u8>>,
}

impl ToolOutput {
    fn text(text: impl Into<String>) -> ToolOutput {
        ToolOutput { text: text.into(), png: None }
    }
}

pub type ToolResult = Result<ToolOutput, String>;

fn point_schema(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2, "description": description })
}

fn colour_schema() -> Value {
    json!({ "type": "string", "description": "Hex colour, #rrggbb or #rrggbbaa" })
}

/// Arguments every drawing tool accepts, so the model can describe its ink
/// in the same words for every tool.
fn ink_properties() -> serde_json::Map<String, Value> {
    let mut properties = serde_json::Map::new();
    properties.insert("colour".into(), colour_schema());
    properties.insert(
        "erase".into(),
        json!({ "type": "boolean", "description": "Erase to transparent instead of painting" }),
    );
    properties.insert(
        "pattern".into(),
        json!({ "type": "string", "description": "Optional MS Paint-style dither: solid, checker, sparse25, dense75, dots, horizontal, vertical, diagonal_down, diagonal_up, cross_hatch, bricks, or bayerN:L (N=2,4,8; L=0..N²)" }),
    );
    properties.insert(
        "secondary".into(),
        json!({ "type": "string", "description": "Colour for the pattern's off pixels; omit to leave them untouched" }),
    );
    properties.insert(
        "layer".into(),
        json!({ "type": "integer", "description": "Layer index (0 = bottom); defaults to the current layer" }),
    );
    properties.insert(
        "frame".into(),
        json!({ "type": "integer", "description": "Frame index (0-based); defaults to the current frame" }),
    );
    properties.insert(
        "size".into(),
        json!({ "type": "integer", "minimum": 1, "description": "Brush size in pixels (default 1)" }),
    );
    properties
}

fn drawing_schema(extra: Value, required: &[&str]) -> Value {
    let mut properties = ink_properties();
    if let Value::Object(more) = extra {
        properties.extend(more);
    }
    json!({ "type": "object", "properties": properties, "required": required })
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

pub fn catalogue() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "document_info",
            description: "Describe the open sprite: size, layers, frames, tags, palette, notes and the current layer/frame. Call this first.",
            input_schema: object_schema(json!({}), &[]),
        },
        ToolSpec {
            name: "new_document",
            description: "Start a new blank sprite, replacing the open one.",
            input_schema: object_schema(
                json!({ "width": { "type": "integer", "minimum": 1, "maximum": 4096 }, "height": { "type": "integer", "minimum": 1, "maximum": 4096 }, "palette": { "type": "string", "description": "Built-in palette name, e.g. PICO-8, DB32, Endesga 32, Game Boy" } }),
                &["width", "height"],
            ),
        },
        ToolSpec {
            name: "open_document",
            description: "Open a .spry, .png or .gif file.",
            input_schema: object_schema(json!({ "path": { "type": "string" } }), &["path"]),
        },
        ToolSpec {
            name: "save_document",
            description: "Save as .spry (also records a version). Path is optional if the document was saved before.",
            input_schema: object_schema(json!({ "path": { "type": "string" } }), &[]),
        },
        ToolSpec {
            name: "render_png",
            description: "Look at the sprite: returns a PNG of one frame, enlarged for visibility.",
            input_schema: object_schema(json!({ "frame": { "type": "integer" }, "scale": { "type": "integer", "minimum": 1, "maximum": 32 } }), &[]),
        },
        ToolSpec {
            name: "get_pixels",
            description: "Read pixels of one cel (or the composite) as a character grid with a colour legend. '.' is transparent.",
            input_schema: object_schema(
                json!({ "x": { "type": "integer" }, "y": { "type": "integer" }, "width": { "type": "integer" }, "height": { "type": "integer" }, "frame": { "type": "integer" }, "layer": { "type": "integer", "description": "Omit for the flattened image" } }),
                &[],
            ),
        },
        ToolSpec {
            name: "set_pixels",
            description: "Set individual pixels on the current (or given) layer/frame.",
            input_schema: object_schema(
                json!({
                    "pixels": { "type": "array", "items": { "type": "object", "properties": { "x": { "type": "integer" }, "y": { "type": "integer" }, "colour": colour_schema() }, "required": ["x", "y", "colour"] } },
                    "layer": { "type": "integer" }, "frame": { "type": "integer" }
                }),
                &["pixels"],
            ),
        },
        ToolSpec {
            name: "draw_line",
            description: "Draw a pixel-exact (Bresenham) line between two points, ends included.",
            input_schema: drawing_schema(json!({ "from": point_schema("[x, y]"), "to": point_schema("[x, y]") }), &["from", "to"]),
        },
        ToolSpec {
            name: "draw_rect",
            description: "Draw a rectangle spanning two corners (inclusive), outlined or filled.",
            input_schema: drawing_schema(json!({ "from": point_schema("corner [x, y]"), "to": point_schema("opposite corner [x, y]"), "filled": { "type": "boolean" } }), &["from", "to"]),
        },
        ToolSpec {
            name: "draw_ellipse",
            description: "Draw the ellipse inscribed in the rectangle spanning two corners, outlined or filled.",
            input_schema: drawing_schema(json!({ "from": point_schema("corner [x, y]"), "to": point_schema("opposite corner [x, y]"), "filled": { "type": "boolean" } }), &["from", "to"]),
        },
        ToolSpec {
            name: "fill",
            description: "Bucket fill from a point. contiguous=false fills every pixel of that colour.",
            input_schema: drawing_schema(json!({ "at": point_schema("[x, y]"), "contiguous": { "type": "boolean" } }), &["at"]),
        },
        ToolSpec {
            name: "dither_fill",
            description: "Ordered-dither gradient from `secondary` at `from` to `colour` at `to`, over the selection or the region under `from`.",
            input_schema: drawing_schema(json!({ "from": point_schema("[x, y]"), "to": point_schema("[x, y]") }), &["from", "to", "colour", "secondary"]),
        },
        ToolSpec {
            name: "outline",
            description: "Add a 1px outline around everything on the layer.",
            input_schema: object_schema(json!({ "colour": colour_schema(), "diagonals": { "type": "boolean" }, "layer": { "type": "integer" }, "frame": { "type": "integer" } }), &["colour"]),
        },
        ToolSpec {
            name: "add_layer",
            description: "Add an empty layer above the current one and make it current.",
            input_schema: object_schema(json!({ "name": { "type": "string" } }), &[]),
        },
        ToolSpec {
            name: "add_group",
            description: "Add an empty layer group above the current layer and make it current. Move layers into it with set_layer's `group`.",
            input_schema: object_schema(json!({ "name": { "type": "string" } }), &[]),
        },
        ToolSpec {
            name: "set_layer",
            description: "Change a layer's properties (the current layer unless `layer` is given). Only the fields you pass change. `group` is the index of a group to move the layer into, or -1 for the top level. `cel_opacity` fades just the current frame's cel.",
            input_schema: object_schema(
                json!({
                    "layer": { "type": "integer" },
                    "name": { "type": "string" },
                    "visible": { "type": "boolean" },
                    "locked": { "type": "boolean" },
                    "opacity": { "type": "integer", "minimum": 0, "maximum": 255 },
                    "blend": { "type": "string", "enum": BlendMode::ALL.iter().map(|m| serde_json::to_value(m).unwrap_or_default()).collect::<Vec<_>>() },
                    "group": { "type": "integer" },
                    "cel_opacity": { "type": "integer", "minimum": 0, "maximum": 255 }
                }),
                &[],
            ),
        },
        ToolSpec {
            name: "select_layer",
            description: "Make a layer current.",
            input_schema: object_schema(json!({ "index": { "type": "integer" } }), &["index"]),
        },
        ToolSpec {
            name: "add_frame",
            description: "Add a frame after the current one (empty, or a copy) and make it current.",
            input_schema: object_schema(json!({ "copy": { "type": "boolean", "description": "Copy the current frame's pixels" }, "duration_ms": { "type": "integer", "minimum": 1 } }), &[]),
        },
        ToolSpec {
            name: "clone_frame",
            description: "Duplicate the current frame as linked cels (clones): painting either frame changes both.",
            input_schema: object_schema(json!({}), &[]),
        },
        ToolSpec {
            name: "select_frame",
            description: "Make a frame current.",
            input_schema: object_schema(json!({ "index": { "type": "integer" } }), &["index"]),
        },
        ToolSpec {
            name: "set_palette",
            description: "Switch to a built-in palette by name, or to a custom list of hex colours.",
            input_schema: object_schema(json!({ "name": { "type": "string" }, "colours": { "type": "array", "items": colour_schema() } }), &[]),
        },
        ToolSpec {
            name: "add_tag",
            description: "Name a frame range as an animation (frames 0-based, inclusive).",
            input_schema: object_schema(
                json!({ "name": { "type": "string" }, "from": { "type": "integer" }, "to": { "type": "integer" }, "direction": { "type": "string", "enum": ["forward", "reverse", "ping_pong"] } }),
                &["name", "from", "to"],
            ),
        },
        ToolSpec {
            name: "add_note",
            description: "Leave a note: on the document (default), on a frame, or pinned to a pixel.",
            input_schema: object_schema(json!({ "text": { "type": "string" }, "frame": { "type": "integer" }, "x": { "type": "integer" }, "y": { "type": "integer" } }), &["text"]),
        },
        ToolSpec {
            name: "list_notes",
            description: "Read the document notes, frame notes and pinned notes.",
            input_schema: object_schema(json!({}), &[]),
        },
        ToolSpec {
            name: "commit_version",
            description: "Save a named snapshot in the document's version history.",
            input_schema: object_schema(json!({ "message": { "type": "string" } }), &["message"]),
        },
        ToolSpec {
            name: "list_versions",
            description: "List saved versions, newest last.",
            input_schema: object_schema(json!({}), &[]),
        },
        ToolSpec {
            name: "restore_version",
            description: "Bring back a saved version (undoable).",
            input_schema: object_schema(json!({ "id": { "type": "integer" } }), &["id"]),
        },
        ToolSpec {
            name: "export_png",
            description: "Write one frame as a PNG.",
            input_schema: object_schema(json!({ "path": { "type": "string" }, "frame": { "type": "integer" }, "scale": { "type": "integer", "minimum": 1 } }), &["path"]),
        },
        ToolSpec {
            name: "export_gif",
            description: "Write the animation (or one tag) as an animated GIF.",
            input_schema: object_schema(json!({ "path": { "type": "string" }, "tag": { "type": "string" }, "scale": { "type": "integer", "minimum": 1 } }), &["path"]),
        },
        ToolSpec {
            name: "export_sheet",
            description: "Write a sprite sheet PNG plus Aseprite-format JSON beside it.",
            input_schema: object_schema(
                json!({
                    "path": { "type": "string", "description": "Path of the .png; the .json goes next to it" },
                    "layout": { "type": "string", "enum": ["horizontal", "vertical", "grid", "packed"] },
                    "columns": { "type": "integer" }, "padding": { "type": "integer" }, "trim": { "type": "boolean" },
                    "merge_duplicates": { "type": "boolean" }, "tag": { "type": "string" }, "scale": { "type": "integer" }
                }),
                &["path"],
            ),
        },
        ToolSpec { name: "undo", description: "Undo the last change.", input_schema: object_schema(json!({}), &[]) },
        ToolSpec { name: "redo", description: "Redo the last undone change.", input_schema: object_schema(json!({}), &[]) },
        ToolSpec {
            name: "run_command",
            description: "Run any Spry editor command as JSON, e.g. {\"command\":\"flip_horizontal\"}, {\"command\":\"drop_shadow\",\"colour\":\"#000000\",\"dx\":1,\"dy\":1}, {\"command\":\"select_rect\",\"rect\":{\"x\":0,\"y\":0,\"width\":4,\"height\":4}}, {\"command\":\"set_frame_duration\",\"duration_ms\":80}. Use when no dedicated tool fits.",
            input_schema: object_schema(json!({ "command": { "type": "object", "description": "A command object with a \"command\" field naming it" } }), &["command"]),
        },
    ]
}

// ----- argument helpers --------------------------------------------------

fn integer(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

fn required_integer(args: &Value, key: &str) -> Result<i64, String> {
    integer(args, key).ok_or_else(|| format!("`{key}` is required and must be an integer"))
}

fn string<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    string(args, key).ok_or_else(|| format!("`{key}` is required and must be a string"))
}

fn colour(args: &Value, key: &str) -> Result<Option<Rgba>, String> {
    string(args, key)
        .map(|text| Rgba::from_hex(text).ok_or_else(|| format!("`{key}`: `{text}` is not a hex colour")))
        .transpose()
}

fn point(args: &Value, key: &str) -> Result<(i32, i32), String> {
    match args.get(key).and_then(Value::as_array).map(|a| a.iter().map(Value::as_i64).collect::<Vec<_>>()) {
        Some(v) if v.len() == 2 && v.iter().all(Option::is_some) => {
            Ok((v[0].unwrap_or(0) as i32, v[1].unwrap_or(0) as i32))
        }
        _ => Err(format!("`{key}` must be an [x, y] pair of integers")),
    }
}

fn path(args: &Value, key: &str) -> Result<PathBuf, String> {
    required_string(args, key).map(PathBuf::from)
}

/// An export path with its extension forced to `extension`, so a model can
/// only ever write the kind of file it asked to export, never over a
/// script or a config file.
fn export_path(args: &Value, extension: &str) -> Result<PathBuf, String> {
    path(args, "path").map(|path| path.with_extension(extension))
}

fn ink(args: &Value) -> Result<Ink, String> {
    if args.get("erase").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(Ink::Erase);
    }
    let primary = colour(args, "colour")?.ok_or("`colour` is required unless `erase` is true")?;
    match string(args, "pattern") {
        None | Some("solid") => Ok(Ink::colour(primary)),
        Some(name) => {
            Ok(Ink::Pattern { pattern: name.parse::<Pattern>()?, primary, secondary: colour(args, "secondary")? })
        }
    }
}

fn brush(args: &Value) -> Brush {
    Brush { size: integer(args, "size").unwrap_or(1).clamp(1, 64) as u32, ..Brush::default() }
}

fn scale_arg(args: &Value) -> u32 {
    integer(args, "scale").unwrap_or(1).clamp(1, 64) as u32
}

// ----- dispatch -------------------------------------------------------------

fn run(studio: &mut Studio, command: Command) -> ToolResult {
    studio.editor.execute(command).map(ToolOutput::text).map_err(|e| e.to_string())
}

/// Points the editor at the `layer`/`frame` named in the arguments, if any.
fn target(studio: &mut Studio, args: &Value) -> Result<(), String> {
    if let Some(layer) = integer(args, "layer") {
        studio.editor.execute(Command::SelectLayer { index: layer.max(0) as usize }).map_err(|e| e.to_string())?;
    }
    if let Some(frame) = integer(args, "frame") {
        studio.editor.execute(Command::SelectFrame { index: frame.max(0) as usize }).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn call(workspace: &mut impl Workspace, name: &str, args: &Value) -> ToolResult {
    match name {
        "new_document" => {
            let (width, height) = (required_integer(args, "width")?, required_integer(args, "height")?);
            let (width, height) = (width.clamp(0, u32::MAX as i64) as u32, height.clamp(0, u32::MAX as i64) as u32);
            crate::editor::limits::check_canvas_size(width, height)?;
            let mut studio = Studio::new(width, height);
            if let Some(palette) = string(args, "palette") {
                run(&mut studio, Command::UseBuiltInPalette { name: palette.into() })?;
            }
            workspace.open(studio);
            Ok(ToolOutput::text(format!("new {width}×{height} sprite")))
        }
        "open_document" => {
            let path = path(args, "path")?;
            workspace.open(Studio::open(&path).map_err(|e| format!("{e:#}"))?);
            Ok(ToolOutput::text(format!("opened {}\n\n{}", path.display(), describe(workspace.studio()))))
        }
        _ => call_on_studio(workspace.studio(), name, args),
    }
}

fn call_on_studio(studio: &mut Studio, name: &str, args: &Value) -> ToolResult {
    match name {
        "document_info" => Ok(ToolOutput::text(describe(studio))),
        "save_document" => {
            let requested = string(args, "path").map(PathBuf::from);
            let saved = studio.save(requested.as_deref()).map_err(|e| format!("{e:#}"))?;
            Ok(ToolOutput::text(format!("saved {}", saved.display())))
        }
        "render_png" => {
            let frame = integer(args, "frame").map_or(studio.editor.frame, |f| f.max(0) as usize);
            if frame >= studio.editor.sprite().frame_count() {
                return Err(format!("there is no frame {frame}"));
            }
            let sprite = studio.editor.sprite();
            // A preview never needs to be wider than 1024 pixels.
            let widest_scale = (1024 / sprite.width.max(sprite.height)).clamp(1, 32);
            let auto_scale = (256 / sprite.width.max(sprite.height)).clamp(1, 32);
            let scale = integer(args, "scale").map_or(auto_scale, |s| s.clamp(1, 32) as u32).min(widest_scale);
            let image = studio.editor.composite_for_display(frame).scaled_up(scale);
            let png = io::encode_png(&image).map_err(|e| e.to_string())?;
            Ok(ToolOutput { text: format!("frame {frame} at {scale}×"), png: Some(png) })
        }
        "get_pixels" => pixel_grid(studio, args),
        "set_pixels" => {
            target(studio, args)?;
            let pixels = args
                .get("pixels")
                .and_then(Value::as_array)
                .ok_or("`pixels` must be an array of {x, y, colour}")?
                .iter()
                .map(|p| {
                    Ok(PixelValue {
                        x: required_integer(p, "x")? as i32,
                        y: required_integer(p, "y")? as i32,
                        colour: colour(p, "colour")?.ok_or("each pixel needs a `colour`")?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            run(studio, Command::SetPixels { pixels })
        }
        "draw_line" => {
            target(studio, args)?;
            run(
                studio,
                Command::DrawLine {
                    from: point(args, "from")?,
                    to: point(args, "to")?,
                    ink: ink(args)?,
                    brush: brush(args),
                },
            )
        }
        "draw_rect" | "draw_ellipse" => {
            target(studio, args)?;
            let (from, to, ink, brush) = (point(args, "from")?, point(args, "to")?, ink(args)?, brush(args));
            let filled = args.get("filled").and_then(Value::as_bool).unwrap_or(false);
            let command = if name == "draw_rect" {
                Command::DrawRect { from, to, ink, filled, brush }
            } else {
                Command::DrawEllipse { from, to, ink, filled, brush }
            };
            run(studio, command)
        }
        "fill" => {
            target(studio, args)?;
            let contiguous = args.get("contiguous").and_then(Value::as_bool).unwrap_or(true);
            run(studio, Command::Fill { at: point(args, "at")?, ink: ink(args)?, contiguous })
        }
        "dither_fill" => {
            target(studio, args)?;
            let primary = colour(args, "colour")?.ok_or("`colour` is required")?;
            let secondary = colour(args, "secondary")?.ok_or("`secondary` is required")?;
            run(
                studio,
                Command::GradientFill { from: point(args, "from")?, to: point(args, "to")?, primary, secondary },
            )
        }
        "outline" => {
            target(studio, args)?;
            let colour = colour(args, "colour")?.ok_or("`colour` is required")?;
            run(
                studio,
                Command::Outline { colour, diagonals: args.get("diagonals").and_then(Value::as_bool).unwrap_or(false) },
            )
        }
        "add_layer" => run(studio, Command::AddLayer { name: string(args, "name").map(str::to_string) }),
        "add_group" => run(studio, Command::AddGroup { name: string(args, "name").map(str::to_string) }),
        "set_layer" => set_layer(studio, args),
        "select_layer" => run(studio, Command::SelectLayer { index: required_integer(args, "index")?.max(0) as usize }),
        "add_frame" => {
            let copy = args.get("copy").and_then(Value::as_bool).unwrap_or(false);
            let summary =
                run(studio, if copy { Command::DuplicateFrame { as_clone: false } } else { Command::AddFrame })?;
            if let Some(duration) = integer(args, "duration_ms") {
                run(studio, Command::SetFrameDuration { duration_ms: duration.max(1) as u32, all_frames: false })?;
            }
            Ok(summary)
        }
        "clone_frame" => run(studio, Command::DuplicateFrame { as_clone: true }),
        "select_frame" => run(studio, Command::SelectFrame { index: required_integer(args, "index")?.max(0) as usize }),
        "set_palette" => match (string(args, "name"), args.get("colours").and_then(Value::as_array)) {
            (_, Some(colours)) => {
                let colours = colours
                    .iter()
                    .map(|c| c.as_str().and_then(Rgba::from_hex).ok_or_else(|| format!("`{c}` is not a hex colour")))
                    .collect::<Result<Vec<_>, _>>()?;
                let name = string(args, "name").unwrap_or("Custom").to_string();
                run(studio, Command::SetPalette { palette: Palette { name, colours } })
            }
            (Some(name), None) => run(studio, Command::UseBuiltInPalette { name: name.into() }),
            (None, None) => Err("give a built-in palette `name` or a list of `colours`".into()),
        },
        "add_tag" => {
            let direction = match string(args, "direction").unwrap_or("forward") {
                "forward" => LoopDirection::Forward,
                "reverse" => LoopDirection::Reverse,
                "ping_pong" | "pingpong" => LoopDirection::PingPong,
                other => return Err(format!("unknown direction `{other}`; use forward, reverse or ping_pong")),
            };
            run(
                studio,
                Command::AddTag {
                    name: required_string(args, "name")?.into(),
                    from: required_integer(args, "from")?.max(0) as usize,
                    to: required_integer(args, "to")?.max(0) as usize,
                    direction,
                },
            )
        }
        "add_note" => {
            let text = required_string(args, "text")?.to_string();
            match (integer(args, "x"), integer(args, "y"), integer(args, "frame")) {
                (Some(x), Some(y), _) => run(studio, Command::AddPin { x: x as i32, y: y as i32, text }),
                (_, _, Some(frame)) => run(studio, Command::SetFrameNote { frame: frame.max(0) as usize, text }),
                _ => {
                    let existing = &studio.editor.sprite().notes;
                    let combined = if existing.is_empty() { text } else { format!("{existing}\n{text}") };
                    run(studio, Command::SetNotes { text: combined })
                }
            }
        }
        "list_notes" => Ok(ToolOutput::text(notes_text(studio))),
        "commit_version" => run(studio, Command::CommitVersion { message: required_string(args, "message")?.into() }),
        "list_versions" => {
            let versions = &studio.editor.document.versions;
            if versions.is_empty() {
                return Ok(ToolOutput::text("no versions yet"));
            }
            let lines: Vec<String> =
                versions.iter().map(|v| format!("{}: {} (at {})", v.id, v.message, v.created_at)).collect();
            Ok(ToolOutput::text(lines.join("\n")))
        }
        "restore_version" => run(studio, Command::RestoreVersion { id: required_integer(args, "id")?.max(0) as u32 }),
        "export_png" => {
            let target_path = export_path(args, "png")?;
            let frame = integer(args, "frame").map_or(studio.editor.frame, |f| f.max(0) as usize);
            if frame >= studio.editor.sprite().frame_count() {
                return Err(format!("there is no frame {frame}"));
            }
            io::export_png(&target_path, studio.editor.sprite(), frame, scale_arg(args))
                .map_err(|e| format!("{e:#}"))?;
            Ok(ToolOutput::text(format!("wrote {}", target_path.display())))
        }
        "export_gif" => {
            let target_path = export_path(args, "gif")?;
            io::export_gif(&target_path, studio.editor.sprite(), string(args, "tag"), scale_arg(args))
                .map_err(|e| format!("{e:#}"))?;
            Ok(ToolOutput::text(format!("wrote {}", target_path.display())))
        }
        "export_sheet" => {
            let target_path = export_path(args, "png")?;
            let layout = match string(args, "layout").unwrap_or("grid") {
                "horizontal" => SheetLayout::Horizontal,
                "vertical" => SheetLayout::Vertical,
                "packed" => SheetLayout::Packed,
                "grid" => SheetLayout::Grid { columns: integer(args, "columns").map(|c| c.max(1) as u32) },
                other => return Err(format!("unknown layout `{other}`")),
            };
            let options = SheetOptions {
                layout,
                padding: integer(args, "padding").unwrap_or(0).clamp(0, 64) as u32,
                trim: args.get("trim").and_then(Value::as_bool).unwrap_or(false),
                merge_duplicates: args.get("merge_duplicates").and_then(Value::as_bool).unwrap_or(false),
                tag: string(args, "tag").map(str::to_string),
                scale: scale_arg(args),
                ..SheetOptions::default()
            };
            let sheet = io::export_sheet(&target_path, studio.editor.sprite(), &studio.title(), &options)
                .map_err(|e| format!("{e:#}"))?;
            Ok(ToolOutput::text(format!(
                "wrote {} ({}×{}, {} frames, {} distinct) and its .json",
                target_path.display(),
                sheet.image.width(),
                sheet.image.height(),
                sheet.frames.len(),
                sheet.distinct_images()
            )))
        }
        "undo" => run(studio, Command::Undo),
        "redo" => run(studio, Command::Redo),
        "run_command" => {
            let command: Command = serde_json::from_value(args.get("command").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("not a valid Spry command: {e}"))?;
            run(studio, command)
        }
        unknown => Err(format!("there is no tool called `{unknown}`")),
    }
}

/// Applies each property `set_layer` was given as its own command, in a
/// fixed order. A bad field stops there with an error; the fields before
/// it have already been applied (and each undoes on its own).
fn set_layer(studio: &mut Studio, args: &Value) -> ToolResult {
    if let Some(layer) = integer(args, "layer") {
        run(studio, Command::SelectLayer { index: layer.max(0) as usize })?;
    }
    let mut commands = Vec::new();
    if let Some(name) = string(args, "name") {
        commands.push(Command::RenameLayer { name: name.into() });
    }
    if let Some(visible) = args.get("visible").and_then(Value::as_bool) {
        commands.push(Command::SetLayerVisible { visible });
    }
    if let Some(locked) = args.get("locked").and_then(Value::as_bool) {
        commands.push(Command::SetLayerLocked { locked });
    }
    if let Some(opacity) = integer(args, "opacity") {
        commands.push(Command::SetLayerOpacity { opacity: opacity.clamp(0, 255) as u8 });
    }
    if let Some(blend) = args.get("blend") {
        let blend: BlendMode =
            serde_json::from_value(blend.clone()).map_err(|_| format!("`{blend}` isn't a blend mode"))?;
        commands.push(Command::SetLayerBlend { blend });
    }
    if let Some(group) = integer(args, "group") {
        commands.push(Command::SetLayerParent { group: (group >= 0).then_some(group as usize) });
    }
    if let Some(opacity) = integer(args, "cel_opacity") {
        commands.push(Command::SetCelOpacity { opacity: opacity.clamp(0, 255) as u8 });
    }
    if commands.is_empty() {
        return Err("set_layer needs at least one property to change".into());
    }
    let summaries =
        commands.into_iter().map(|command| run(studio, command).map(|o| o.text)).collect::<Result<Vec<_>, _>>()?;
    Ok(ToolOutput::text(summaries.join("; ")))
}

fn describe(studio: &Studio) -> String {
    let editor = &studio.editor;
    let sprite = editor.sprite();
    let layers: Vec<String> = sprite
        .layers
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let flags = [(!l.visible).then_some("hidden"), l.locked.then_some("locked")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let marker = if i == editor.layer { " ← current" } else { "" };
            let flags = if flags.is_empty() { String::new() } else { format!(" ({})", flags.join(", ")) };
            let kind = if l.is_group() { "group " } else { "" };
            let indent = "  ".repeat(sprite.depth(i));
            let blend = if l.blend == BlendMode::Normal { String::new() } else { format!(", {}", l.blend.name()) };
            format!("  {indent}{i}: {kind}{}{flags}, opacity {}{blend}{marker}", l.name, l.opacity)
        })
        .collect();
    let frames: Vec<String> = sprite.frames.iter().map(|f| f.duration_ms.to_string()).collect();
    let tags: Vec<String> =
        sprite.tags.iter().map(|t| format!("{} [{}–{}, {:?}]", t.name, t.from, t.to, t.direction)).collect();
    let palette: Vec<String> = sprite.palette.colours.iter().map(|c| c.to_hex()).collect();
    format!(
        "\"{}\": {}×{} pixels\nlayers (0 = bottom):\n{}\nframes: {} (durations ms: {}), current frame {}\ntags: {}\npalette {}: {}\nversions: {}\nnotes: {}",
        studio.title(),
        sprite.width,
        sprite.height,
        layers.join("\n"),
        sprite.frame_count(),
        frames.join(", "),
        editor.frame,
        if tags.is_empty() { "none".into() } else { tags.join(", ") },
        sprite.palette.name,
        palette.join(" "),
        studio.editor.document.versions.len(),
        if sprite.notes.is_empty() { "none" } else { &sprite.notes },
    )
}

fn notes_text(studio: &Studio) -> String {
    let sprite = studio.editor.sprite();
    let mut lines = vec![format!("document: {}", if sprite.notes.is_empty() { "(none)" } else { &sprite.notes })];
    for (index, frame) in sprite.frames.iter().enumerate().filter(|(_, f)| !f.note.is_empty()) {
        lines.push(format!("frame {index}: {}", frame.note));
    }
    for (index, pin) in sprite.pins.iter().enumerate() {
        lines.push(format!("pin {index} at ({}, {}): {}", pin.x, pin.y, pin.text));
    }
    lines.join("\n")
}

/// A text picture of the pixels, one character per pixel with a legend,
/// which lets a model reason about exact coordinates even without vision.
fn pixel_grid(studio: &Studio, args: &Value) -> ToolResult {
    let sprite = studio.editor.sprite();
    let frame = integer(args, "frame").map_or(studio.editor.frame, |f| f.max(0) as usize);
    if frame >= sprite.frame_count() {
        return Err(format!("there is no frame {frame}"));
    }
    let image = match integer(args, "layer") {
        Some(layer) => sprite
            .cel_image(layer.max(0) as usize, frame)
            .cloned()
            .unwrap_or_else(|| crate::editor::Pixels::new(sprite.width, sprite.height)),
        None => studio.editor.composite_for_display(frame),
    };
    let region = Rect::new(
        integer(args, "x").unwrap_or(0) as i32,
        integer(args, "y").unwrap_or(0) as i32,
        integer(args, "width").unwrap_or(sprite.width as i64).clamp(1, 128) as u32,
        integer(args, "height").unwrap_or(sprite.height as i64).clamp(1, 128) as u32,
    )
    .intersect(&image.bounds());
    const SYMBOLS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789#@$%&*+=?";
    let mut legend: Vec<Rgba> = Vec::new();
    let mut rows = Vec::new();
    for y in region.y..region.bottom() {
        let mut row = String::new();
        for x in region.x..region.right() {
            let colour = image.get(x, y);
            if colour.is_transparent() {
                row.push('.');
                continue;
            }
            let index = legend.iter().position(|&c| c == colour).unwrap_or_else(|| {
                legend.push(colour);
                legend.len() - 1
            });
            row.push(SYMBOLS.chars().nth(index).unwrap_or('~'));
        }
        rows.push(row);
    }
    let key: Vec<String> =
        legend.iter().enumerate().map(|(i, c)| format!("{}={}", SYMBOLS.chars().nth(i).unwrap_or('~'), c)).collect();
    Ok(ToolOutput::text(format!(
        "region x={} y={} {}×{} (frame {frame})\n{}\nlegend: . = transparent{}{}",
        region.x,
        region.y,
        region.width,
        region.height,
        rows.join("\n"),
        if key.is_empty() { "" } else { ", " },
        key.join(", ")
    )))
}

/// MCP content blocks for a tool result.
pub fn content_blocks(output: &ToolOutput) -> Vec<Value> {
    let mut blocks = vec![json!({ "type": "text", "text": output.text })];
    if let Some(png) = &output.png {
        blocks.push(json!({ "type": "image", "mimeType": "image/png", "data": base64::engine::general_purpose::STANDARD.encode(png) }));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn studio() -> Studio {
        Studio::new(8, 8)
    }

    #[test]
    fn every_tool_has_an_object_schema_and_a_unique_name() {
        let tools = catalogue();
        let mut names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), tools.len());
        assert!(tools.iter().all(|t| t.input_schema["type"] == "object"));
    }

    #[test]
    fn every_catalogued_tool_is_dispatched() {
        for tool in catalogue() {
            let result = call(&mut studio(), tool.name, &json!({}));
            if let Err(message) = result {
                assert!(!message.starts_with("there is no tool"), "{} is not dispatched", tool.name);
            }
        }
    }

    #[test]
    fn drawing_tools_accept_patterns_by_name() {
        let mut studio = studio();
        call(
            &mut studio,
            "draw_rect",
            &json!({ "from": [0, 0], "to": [3, 3], "filled": true, "colour": "#000000", "pattern": "checker" }),
        )
        .unwrap();
        let cel = studio.editor.sprite().cel_image(0, 0).unwrap();
        assert_eq!(cel.get(0, 0), Rgba::BLACK);
        assert_eq!(cel.get(1, 0), Rgba::TRANSPARENT);
    }

    #[test]
    fn get_pixels_draws_a_legend_grid() {
        let mut studio = studio();
        call(&mut studio, "set_pixels", &json!({ "pixels": [{ "x": 1, "y": 0, "colour": "#ff004d" }] })).unwrap();
        let grid = call(&mut studio, "get_pixels", &json!({ "width": 3, "height": 1 })).unwrap().text;
        assert!(grid.contains("\n.A.\n"), "{grid}");
        assert!(grid.contains("A=#ff004d"));
    }

    #[test]
    fn bad_arguments_explain_what_was_expected() {
        let error =
            call(&mut studio(), "draw_line", &json!({ "from": [0], "to": [1, 1], "colour": "#fff" })).unwrap_err();
        assert!(error.contains("[x, y]"));
        let error = call(&mut studio(), "fill", &json!({ "at": [0, 0] })).unwrap_err();
        assert!(error.contains("colour"));
    }

    #[test]
    fn run_command_reaches_every_editor_verb() {
        let mut studio = studio();
        call(&mut studio, "run_command", &json!({ "command": { "command": "add_layer", "name": "ink" } })).unwrap();
        assert_eq!(studio.editor.sprite().layers[1].name, "ink");
        assert!(call(&mut studio, "run_command", &json!({ "command": { "command": "explode" } })).is_err());
    }

    #[test]
    fn render_png_returns_an_enlarged_image() {
        let output = call(&mut studio(), "render_png", &json!({})).unwrap();
        let image = io::decode_image(&output.png.unwrap()).unwrap();
        assert_eq!(image.width(), 256);
    }
}

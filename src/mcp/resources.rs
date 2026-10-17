//! MCP resources: the document as things a client can read without calling
//! a tool. A summary of the document, and each frame as a PNG at its real
//! size. The PNGs are exact pixels, unlike `render_png`, which enlarges
//! for a model's eyes.

use super::tools::describe;
use crate::io;
use crate::studio::Studio;
use base64::Engine;
use serde_json::{json, Value};

const DOCUMENT_URI: &str = "spry://document";
const FRAME_PREFIX: &str = "spry://frames/";

pub fn list(studio: &Studio) -> Value {
    let mut resources = vec![json!({
        "uri": DOCUMENT_URI,
        "name": "document",
        "title": format!("{}: layers, frames, tags, palette", studio.title()),
        "mimeType": "text/plain",
    })];
    resources.extend((0..studio.editor.sprite().frame_count()).map(|frame| {
        json!({
            "uri": frame_uri(frame),
            "name": format!("frame {frame}"),
            "title": format!("Frame {} at its real size", frame + 1),
            "mimeType": "image/png",
        })
    }));
    json!({ "resources": resources })
}

pub fn templates() -> Value {
    json!({ "resourceTemplates": [{
        "uriTemplate": format!("{FRAME_PREFIX}{{frame}}.png"),
        "name": "frame",
        "title": "One frame, flattened, at its real size (frames count from 0)",
        "mimeType": "image/png",
    }] })
}

pub fn read(studio: &Studio, uri: &str) -> Result<Value, String> {
    if uri == DOCUMENT_URI {
        return Ok(json!({ "contents": [{ "uri": uri, "mimeType": "text/plain", "text": describe(studio) }] }));
    }
    let frame = uri
        .strip_prefix(FRAME_PREFIX)
        .and_then(|rest| rest.strip_suffix(".png"))
        .and_then(|number| number.parse::<usize>().ok())
        .ok_or_else(|| format!("there's no resource at {uri}"))?;
    let sprite = studio.editor.sprite();
    if frame >= sprite.frame_count() {
        return Err(format!("there's no frame {frame}; frames run from 0 to {}", sprite.frame_count() - 1));
    }
    let png = io::encode_png(&studio.editor.composite_for_display(frame)).map_err(|e| e.to_string())?;
    let blob = base64::engine::general_purpose::STANDARD.encode(png);
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": "image/png", "blob": blob }] }))
}

fn frame_uri(frame: usize) -> String {
    format!("{FRAME_PREFIX}{frame}.png")
}

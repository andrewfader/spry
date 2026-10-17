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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Command, Ink, Rgba};

    fn red_studio() -> Studio {
        let mut studio = Studio::new(4, 2);
        studio
            .editor
            .execute(Command::Fill { at: (0, 0), ink: Ink::colour(Rgba::rgb(255, 0, 0)), contiguous: true })
            .unwrap();
        studio
    }

    #[test]
    fn the_document_and_each_frame_are_listed() {
        let mut studio = red_studio();
        studio.editor.execute(Command::AddFrame).unwrap();
        let uris: Vec<String> = list(&studio)["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["uri"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(uris, vec!["spry://document", "spry://frames/0.png", "spry://frames/1.png"]);
    }

    #[test]
    fn a_frame_reads_back_as_its_exact_pixels() {
        let studio = red_studio();
        let read = read(&studio, "spry://frames/0.png").unwrap();
        let blob = read["contents"][0]["blob"].as_str().unwrap();
        let png = base64::engine::general_purpose::STANDARD.decode(blob).unwrap();
        assert_eq!(io::decode_image(&png).unwrap(), studio.editor.sprite().composite(0));
    }

    #[test]
    fn the_document_reads_as_its_summary() {
        let studio = red_studio();
        let text = read(&studio, DOCUMENT_URI).unwrap()["contents"][0]["text"].as_str().unwrap().to_string();
        assert!(text.contains("4×2"), "{text}");
    }

    #[test]
    fn unknown_and_missing_resources_say_what_exists() {
        let studio = red_studio();
        assert!(read(&studio, "spry://frames/3.png").unwrap_err().contains("frames run from 0 to 0"));
        assert!(read(&studio, "spry://elsewhere").unwrap_err().contains("no resource"));
        assert!(read(&studio, "spry://frames/x.png").is_err());
    }

    #[test]
    fn frames_are_offered_as_a_template() {
        assert_eq!(templates()["resourceTemplates"][0]["uriTemplate"], "spry://frames/{frame}.png");
    }
}

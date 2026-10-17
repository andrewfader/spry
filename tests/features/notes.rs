//! Feature 11: notes.

use crate::support::*;
use serde_json::json;
use spry::editor::Command;
use spry::io::{self, SheetOptions};
use spry::studio::Studio;

#[test]
fn notes_on_the_document_frames_and_pixels_survive_save_and_load() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario
        .when(Command::SetNotes { text: "# Knight\nKeep the cape two-tone.".into() })
        .and(Command::SetFrameNote { frame: 0, text: "anticipation pose".into() })
        .and(Command::AddPin { x: 2, y: 3, text: "fix this hand".into() });
    let path = scenario.path("notes.spry");
    scenario.studio.save(Some(&path)).unwrap();
    let sprite = Studio::open(&path).unwrap().editor.sprite().clone();
    assert_eq!(sprite.notes, "# Knight\nKeep the cape two-tone.");
    assert_eq!(sprite.frames[0].note, "anticipation pose");
    assert_eq!((sprite.pins[0].x, sprite.pins[0].y, sprite.pins[0].text.as_str()), (2, 3, "fix this hand"));
}

#[test]
fn notes_travel_inside_the_sprite_sheet_json() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::SetNotes { text: "palette: PICO-8 only".into() }).and(Command::AddPin {
        x: 1,
        y: 1,
        text: "pivot".into(),
    });
    let sheet = io::sheet::build(scenario.sprite(), "knight", &SheetOptions::default()).unwrap();
    assert_eq!(sheet.json["meta"]["notes"]["document"], "palette: PICO-8 only");
    assert_eq!(sheet.json["meta"]["notes"]["pins"][0]["text"], "pivot");
}

#[test]
fn an_llm_can_leave_and_read_notes_over_mcp() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.tool("add_note", json!({ "text": "needs a darker outline" })).unwrap();
    scenario.tool("add_note", json!({ "text": "blink here", "frame": 0 })).unwrap();
    scenario.tool("add_note", json!({ "text": "sword tip", "x": 6, "y": 1 })).unwrap();
    let notes = scenario.tool("list_notes", json!({})).unwrap().text;
    assert!(notes.contains("needs a darker outline"));
    assert!(notes.contains("frame 0: blink here"));
    assert!(notes.contains("(6, 1): sword tip"));
}

#[test]
fn pins_can_be_removed() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::AddPin { x: 0, y: 0, text: "a".into() }).and(Command::RemovePin { index: 0 });
    assert!(scenario.sprite().pins.is_empty());
}

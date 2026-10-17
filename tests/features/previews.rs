//! Feature 14: previews. The UI's previews show exactly what these
//! functions produce, so testing the functions tests what you see.

use crate::support::*;
use serde_json::json;
use spry::editor::playback::frame_at;
use spry::editor::{Command, Ink, LoopDirection, PixelValue, Rect};

#[test]
fn the_preview_includes_floating_pixels_before_they_land() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: RED }] });
    scenario
        .when(Command::SelectRect { rect: Rect::new(0, 0, 1, 1), add: false })
        .and(Command::MoveFloating { dx: 4, dy: 4 });
    let shown = scenario.studio.editor.composite_for_display(0);
    assert_eq!(shown.get(4, 4), RED);
    assert_eq!(shown.get(0, 0), CLEAR);
}

#[test]
fn the_preview_follows_the_chosen_tag() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::AddFrame).and(Command::AddFrame).and(Command::AddFrame);
    scenario.when(Command::AddTag { name: "tail".into(), from: 2, to: 3, direction: LoopDirection::Forward });
    let tag = scenario.sprite().tags[0].clone();
    let shown: Vec<usize> = [0, 100, 200, 300].iter().map(|&t| frame_at(scenario.sprite(), Some(&tag), t)).collect();
    assert_eq!(shown, vec![2, 3, 2, 3]);
}

#[test]
fn an_llm_preview_is_a_crisp_enlargement() {
    let mut scenario = given_a_blank_sprite(16, 16);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(GREEN), contiguous: true });
    let output = scenario.tool("render_png", json!({ "scale": 4 })).unwrap();
    let image = spry::io::decode_image(&output.png.unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (64, 64));
    assert!(image.colours().iter().all(|&c| c == GREEN));
}

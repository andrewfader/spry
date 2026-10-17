//! Feature 1: canvas & documents.

use crate::support::*;
use spry::editor::{Command, Ink, LoopDirection, Rect};
use spry::studio::Studio;

#[test]
fn a_new_document_starts_with_one_empty_layer_and_frame() {
    let scenario = given_a_blank_sprite(32, 32);
    let sprite = scenario.sprite();
    assert_eq!((sprite.width, sprite.height), (32, 32));
    assert_eq!(sprite.layers.len(), 1);
    assert_eq!(sprite.layers[0].name, "Layer 1");
    assert_eq!(sprite.frame_count(), 1);
    assert_eq!(sprite.frames[0].duration_ms, 100);
    assert!(sprite.cel_image(0, 0).is_none(), "the first cel is transparent");
    assert_eq!(sprite.palette.name, "PICO-8");
}

#[test]
fn resizing_anchored_top_left_keeps_pixels_where_they_were() {
    let mut scenario = given_a_blank_sprite(32, 32);
    scenario.when(Command::DrawLine { from: (0, 0), to: (5, 5), ink: Ink::colour(RED), brush: Default::default() });
    scenario.when(Command::ResizeCanvas { width: 48, height: 16, offset_x: 0, offset_y: 0 });
    assert_eq!((scenario.sprite().width, scenario.sprite().height), (48, 16));
    assert_eq!(scenario.pixel(5, 5), RED);
    assert_eq!(scenario.pixel(40, 10), CLEAR, "new area is transparent");
}

#[test]
fn a_saved_file_reopens_with_everything_identical() {
    let mut scenario = given_a_blank_sprite(16, 16);
    scenario
        .when(Command::Fill { at: (0, 0), ink: Ink::colour(BLUE), contiguous: true })
        .and(Command::AddLayer { name: Some("ink".into()) })
        .and(Command::DrawRect {
            from: (2, 2),
            to: (6, 6),
            ink: Ink::colour(RED),
            filled: false,
            brush: Default::default(),
        })
        .and(Command::DuplicateFrame { as_clone: true })
        .and(Command::AddTag { name: "idle".into(), from: 0, to: 1, direction: LoopDirection::PingPong })
        .and(Command::SetNotes { text: "hero, facing right".into() })
        .and(Command::AddPin { x: 3, y: 3, text: "eye goes here".into() })
        .and(Command::AddSlice { name: "hitbox".into(), bounds: Rect::new(2, 2, 5, 5) });
    let path = scenario.path("hero.spry");
    scenario.studio.save(Some(&path)).unwrap();

    let reopened = Studio::open(&path).unwrap();
    assert_eq!(reopened.editor.document, scenario.studio.editor.document);
    assert!(reopened.editor.sprite().is_linked(1, 0), "clones stay linked after a round trip");
}

#[test]
fn pngs_and_gifs_open_as_new_documents() {
    let scenario = given_a_blank_sprite(4, 4);
    let image = picture(&["r..", ".g."], &[('r', RED), ('g', GREEN)]);
    let path = scenario.path("art.png");
    std::fs::write(&path, spry::io::encode_png(&image).unwrap()).unwrap();
    let opened = Studio::open(&path).unwrap();
    assert_eq!(opened.editor.sprite().composite(0), image);
    assert!(opened.path.is_none(), "an imported image saves as a new .spry");
}

#[test]
fn opening_a_damaged_file_says_so() {
    let scenario = given_a_blank_sprite(4, 4);
    let path = scenario.path("broken.spry");
    std::fs::write(&path, b"definitely not gzip").unwrap();
    let error = Studio::open(&path).err().expect("a damaged file should not open");
    assert!(format!("{error:#}").contains("not a .spry file"));
}

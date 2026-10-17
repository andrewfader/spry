//! Feature 13: automatic sprite sheets.

use crate::support::*;
use spry::editor::{Brush, Command, Ink, PixelValue};
use spry::io::{self, SheetLayout, SheetOptions};

/// Four 8×8 frames where frames 2 and 4 (indices 1 and 3) are identical.
fn four_frames_with_a_repeat() -> Scenario {
    let mut scenario = given_a_blank_sprite(8, 8);
    let marks = [(0, 0, RED), (5, 5, GREEN), (2, 6, BLUE), (5, 5, GREEN)];
    for (frame, (x, y, colour)) in marks.into_iter().enumerate() {
        if frame > 0 {
            scenario.when(Command::AddFrame);
        }
        scenario.when(Command::SetPixels { pixels: vec![PixelValue { x, y, colour }] });
    }
    scenario
}

#[test]
fn merging_duplicates_stores_three_images_for_four_frames() {
    let scenario = four_frames_with_a_repeat();
    let options = SheetOptions { merge_duplicates: true, ..SheetOptions::default() };
    let sheet = io::sheet::build(scenario.sprite(), "hero", &options).unwrap();
    assert_eq!(sheet.distinct_images(), 3);
    assert_eq!(sheet.frames[1].sheet_rect, sheet.frames[3].sheet_rect, "frames 2 and 4 share a rect");
}

#[test]
fn the_sheet_and_its_aseprite_json_are_written_side_by_side() {
    let mut scenario = four_frames_with_a_repeat();
    scenario.when(Command::AddTag {
        name: "walk".into(),
        from: 0,
        to: 3,
        direction: spry::editor::LoopDirection::Forward,
    });
    let png = scenario.path("hero-sheet.png");
    let options = SheetOptions { layout: SheetLayout::Horizontal, padding: 1, ..SheetOptions::default() };
    io::export_sheet(&png, scenario.sprite(), "hero", &options).unwrap();

    let image = io::decode_image(&std::fs::read(&png).unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (8 * 4 + 3, 8));
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(json["meta"]["image"], "hero-sheet.png");
    assert_eq!(json["meta"]["size"]["w"], 35);
    assert_eq!(json["meta"]["frameTags"][0]["name"], "walk");
    let frame_keys: Vec<&String> = json["frames"].as_object().unwrap().keys().collect();
    assert_eq!(
        frame_keys,
        vec!["hero 0.aseprite", "hero 1.aseprite", "hero 2.aseprite", "hero 3.aseprite"],
        "frames are listed in order"
    );
    assert_eq!(json["frames"]["hero 1.aseprite"]["frame"]["x"], 9);
    assert_eq!(json["frames"]["hero 1.aseprite"]["duration"], 100);
}

#[test]
fn packed_and_trimmed_sheets_are_smaller_than_strips() {
    let scenario = four_frames_with_a_repeat();
    let strip = io::sheet::build(
        scenario.sprite(),
        "x",
        &SheetOptions { layout: SheetLayout::Horizontal, ..SheetOptions::default() },
    )
    .unwrap();
    let packed = io::sheet::build(
        scenario.sprite(),
        "x",
        &SheetOptions { layout: SheetLayout::Packed, trim: true, merge_duplicates: true, ..SheetOptions::default() },
    )
    .unwrap();
    let area = |p: &spry::editor::Pixels| p.width() * p.height();
    assert!(area(&packed.image) < area(&strip.image));
}

#[test]
fn a_sheet_slices_back_into_the_same_animation() {
    let scenario = four_frames_with_a_repeat();
    let sheet = io::sheet::build(
        scenario.sprite(),
        "x",
        &SheetOptions { layout: SheetLayout::Grid { columns: Some(2) }, ..SheetOptions::default() },
    )
    .unwrap();
    let document = io::document_from_sheet(&sheet.image, 8, 8).unwrap();
    assert_eq!(document.sprite.frame_count(), 4);
    for frame in 0..4 {
        assert_eq!(document.sprite.composite(frame), scenario.sprite().composite(frame));
    }
}

#[test]
fn importing_a_sheet_file_finds_its_cells_from_the_gutters() {
    let mut scenario = given_a_blank_sprite(8, 8);
    for (frame, colour) in [RED, GREEN, BLUE].into_iter().enumerate() {
        if frame > 0 {
            scenario.when(Command::AddFrame);
        }
        scenario.when(Command::DrawRect {
            from: (1, 1),
            to: (6, 6),
            ink: Ink::colour(colour),
            filled: true,
            brush: Brush::default(),
        });
    }
    let path = scenario.path("walk.png");
    let options = SheetOptions { layout: SheetLayout::Horizontal, ..SheetOptions::default() };
    io::export_sheet(&path, scenario.sprite(), "walk", &options).unwrap();

    let (document, cell) = io::import_sheet(&path, (16, 16)).unwrap();
    assert_eq!(cell, (8, 8), "the transparent border around each frame gives the grid away");
    assert_eq!(document.sprite.frame_count(), 3);
    assert_eq!(document.sprite.composite(2).get(3, 3), BLUE);
}

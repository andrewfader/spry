//! Feature 7: clones (linked cels) and the clone stamp.

use crate::support::*;
use spry::editor::{Brush, Command, Ink, PixelValue, StrokeKind};

fn paint(x: i32, y: i32, colour: spry::editor::Rgba) -> Command {
    Command::SetPixels { pixels: vec![PixelValue { x, y, colour }] }
}

#[test]
fn painting_a_cloned_frame_paints_the_original_too() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(paint(0, 0, RED)).and(Command::DuplicateFrame { as_clone: true });
    assert_eq!(scenario.studio.editor.frame, 1);
    scenario.when(paint(3, 3, BLUE));
    assert_eq!(scenario.cel_pixel(0, 0, 3, 3), BLUE, "frame 1 shows paint from frame 2");
}

#[test]
fn an_unlinked_cel_paints_on_its_own() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(paint(0, 0, RED)).and(Command::DuplicateFrame { as_clone: true }).and(Command::UnlinkCel);
    scenario.when(paint(3, 3, BLUE));
    assert_eq!(scenario.cel_pixel(0, 0, 3, 3), CLEAR, "the original frame is unchanged");
    assert_eq!(scenario.cel_pixel(0, 0, 0, 0), RED);
}

#[test]
fn any_cel_can_be_linked_to_another_frame() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario
        .when(Command::AddFrame)
        .and(Command::AddFrame)
        .and(Command::SelectFrame { index: 0 })
        .and(paint(1, 1, GREEN));
    scenario.when(Command::LinkCel { to_frame: 2 });
    assert!(scenario.sprite().is_linked(0, 2));
    assert_eq!(scenario.cel_pixel(0, 2, 1, 1), GREEN);
    assert_eq!(scenario.cel_pixel(0, 1, 1, 1), CLEAR);
}

#[test]
fn undo_restores_a_link_that_was_broken() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario
        .when(paint(0, 0, RED))
        .and(Command::DuplicateFrame { as_clone: true })
        .and(Command::UnlinkCel)
        .and(Command::Undo);
    assert!(scenario.sprite().is_linked(0, 1));
}

#[test]
fn the_clone_stamp_copies_from_its_source_offset() {
    let mut scenario = given_a_blank_sprite(16, 16);
    scenario.when(paint(1, 1, RED)).and(paint(2, 1, BLUE));
    let editor = &mut scenario.studio.editor;
    editor
        .begin_stroke(
            (9, 9),
            Ink::Clone { dx: -8, dy: -8 },
            Brush::default(),
            StrokeKind::Freehand { pixel_perfect: false },
        )
        .unwrap();
    editor.continue_stroke((10, 9));
    editor.finish_stroke();
    assert_eq!(scenario.pixel(9, 9), RED);
    assert_eq!(scenario.pixel(10, 9), BLUE);
}

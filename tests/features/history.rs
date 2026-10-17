//! Feature 9: history.

use crate::support::*;
use spry::editor::{Brush, Command, Ink, StrokeKind};

fn stroke(scenario: &mut Scenario, row: i32) {
    let editor = &mut scenario.studio.editor;
    editor
        .begin_stroke((0, row), Ink::colour(RED), Brush::default(), StrokeKind::Freehand { pixel_perfect: false })
        .unwrap();
    editor.continue_stroke((3, row));
    editor.continue_stroke((7, row));
    editor.finish_stroke();
}

#[test]
fn undo_twice_and_redo_once_leaves_the_first_two_strokes() {
    let mut scenario = given_a_blank_sprite(8, 8);
    for row in [1, 2, 3] {
        stroke(&mut scenario, row);
    }
    scenario.when(Command::Undo).and(Command::Undo).and(Command::Redo);
    assert_eq!(scenario.pixel(5, 1), RED);
    assert_eq!(scenario.pixel(5, 2), RED);
    assert_eq!(scenario.pixel(5, 3), CLEAR);
}

#[test]
fn a_whole_drag_is_one_undo_step() {
    let mut scenario = given_a_blank_sprite(8, 8);
    stroke(&mut scenario, 4);
    scenario.when(Command::Undo);
    assert!(scenario.painted_pixels().is_empty());
    assert!(!scenario.studio.editor.can_undo());
}

#[test]
fn undo_with_nothing_to_undo_says_so() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when_trying(Command::Undo);
    scenario.then_it_failed_saying("nothing to undo");
}

//! Feature 4: selection & clipboard.

use crate::support::*;
use spry::editor::{Command, Ink, PixelValue, Rect};

fn with_a_red_dot_at_origin() -> Scenario {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: RED }] });
    scenario
}

#[test]
fn copy_paste_and_move_then_stamp_down() {
    let mut scenario = with_a_red_dot_at_origin();
    scenario
        .when(Command::SelectRect { rect: Rect::new(0, 0, 1, 1), add: false })
        .and(Command::Copy)
        .and(Command::Paste { at: Some((2, 2)) })
        .and(Command::MoveFloating { dx: 1, dy: 1 })
        .and(Command::Anchor);
    assert_eq!(scenario.pixel(0, 0), RED, "copying keeps the original");
    assert_eq!(scenario.pixel(3, 3), RED);
}

#[test]
fn cut_removes_the_pixels_and_paste_brings_them_back() {
    let mut scenario = with_a_red_dot_at_origin();
    scenario.when(Command::SelectRect { rect: Rect::new(0, 0, 2, 2), add: false }).and(Command::Cut);
    assert_eq!(scenario.pixel(0, 0), CLEAR);
    scenario.when(Command::Paste { at: Some((5, 5)) }).and(Command::Anchor);
    assert_eq!(scenario.pixel(5, 5), RED);
}

#[test]
fn the_magic_wand_selects_a_contiguous_colour_region() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::DrawRect {
        from: (0, 0),
        to: (2, 2),
        ink: Ink::colour(RED),
        filled: true,
        brush: Default::default(),
    });
    scenario.when(Command::SelectWand { at: (1, 1), contiguous: true, add: false });
    let selection = scenario.studio.editor.selection.clone().unwrap();
    assert_eq!(selection.points().count(), 9);
    assert_eq!(selection.bounds(), Some(Rect::new(0, 0, 3, 3)));
}

#[test]
fn invert_selection_selects_everything_else() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::SelectRect { rect: Rect::new(0, 0, 2, 4), add: false }).and(Command::InvertSelection);
    let selection = scenario.studio.editor.selection.clone().unwrap();
    assert!(!selection.contains(0, 0));
    assert!(selection.contains(3, 0));
}

#[test]
fn flipping_a_selection_only_flips_that_region() {
    let mut scenario = given_a_blank_sprite(8, 1);
    scenario.when(Command::SetPixels {
        pixels: vec![PixelValue { x: 0, y: 0, colour: RED }, PixelValue { x: 7, y: 0, colour: GREEN }],
    });
    scenario
        .when(Command::SelectRect { rect: Rect::new(0, 0, 4, 1), add: false })
        .and(Command::FlipHorizontal)
        .and(Command::Anchor);
    assert_eq!(scenario.pixel(3, 0), RED);
    assert_eq!(scenario.pixel(0, 0), CLEAR);
    assert_eq!(scenario.pixel(7, 0), GREEN, "outside the selection is untouched");
}

#[test]
fn outlining_rings_the_art_in_the_given_colour() {
    let mut scenario = given_a_blank_sprite(5, 5);
    scenario.when(Command::SetPixels { pixels: vec![PixelValue { x: 2, y: 2, colour: RED }] });
    scenario.when(Command::Outline { colour: BLACK, diagonals: false });
    scenario.then_pixels(&[(1, 2), (3, 2), (2, 1), (2, 3)], BLACK);
    assert_eq!(scenario.pixel(1, 1), CLEAR);
}

#[test]
fn nudging_needs_something_selected() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when_trying(Command::MoveFloating { dx: 1, dy: 0 });
    scenario.then_it_failed_saying("nothing is selected");
}

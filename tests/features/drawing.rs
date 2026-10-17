//! Feature 2: drawing tools.

use crate::support::*;
use spry::editor::raster;
use spry::editor::{Brush, Command, Ink, StrokeKind};

#[test]
fn a_line_paints_exactly_the_bresenham_pixels() {
    let mut scenario = given_a_blank_sprite(16, 16);
    scenario.when(Command::DrawLine { from: (0, 0), to: (7, 3), ink: Ink::colour(RED), brush: Brush::default() });
    let mut painted = scenario.painted_pixels();
    painted.sort();
    let mut expected = raster::line((0, 0), (7, 3));
    expected.sort();
    assert_eq!(painted, expected);
}

#[test]
fn pixel_perfect_strokes_leave_no_double_pixels_on_corners() {
    let mut scenario = given_a_blank_sprite(16, 16);
    let editor = &mut scenario.studio.editor;
    editor
        .begin_stroke((0, 0), Ink::colour(RED), Brush::default(), StrokeKind::Freehand { pixel_perfect: true })
        .unwrap();
    for point in [(1, 0), (1, 1), (2, 1), (2, 2), (3, 2), (3, 3)] {
        editor.continue_stroke(point);
    }
    editor.finish_stroke();
    let painted = scenario.painted_pixels();
    for &(x, y) in &painted {
        let neighbours =
            [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)].iter().filter(|p| painted.contains(p)).count();
        assert!(neighbours <= 1, "({x}, {y}) has {neighbours} orthogonal neighbours: an L corner survived");
    }
    assert_eq!(scenario.pixel(0, 0), RED);
    assert_eq!(scenario.pixel(3, 3), RED);
}

#[test]
fn horizontal_symmetry_mirrors_across_the_centre() {
    let mut scenario = given_a_blank_sprite(16, 16);
    let brush = Brush { mirror_x: true, ..Brush::default() };
    scenario.when(Command::Paint { points: vec![(2, 5)], ink: Ink::colour(RED), brush, pixel_perfect: false });
    scenario.then_pixels(&[(2, 5), (13, 5)], RED);
    assert_eq!(scenario.painted_pixels().len(), 2);
}

#[test]
fn a_size_three_brush_paints_a_square_centred_on_the_click() {
    let mut scenario = given_a_blank_sprite(16, 16);
    let brush = Brush { size: 3, ..Brush::default() };
    scenario.when(Command::Paint { points: vec![(5, 5)], ink: Ink::colour(RED), brush, pixel_perfect: false });
    let mut painted = scenario.painted_pixels();
    painted.sort();
    let expected: Vec<(i32, i32)> = (4..=6).flat_map(|x| (4..=6).map(move |y| (x, y))).collect();
    assert_eq!(painted, expected);
}

#[test]
fn drawing_on_a_locked_layer_changes_nothing_and_says_why() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::SetLayerLocked { locked: true });
    scenario.when_trying(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    scenario.then_it_failed_saying("locked");
    assert!(scenario.painted_pixels().is_empty());
}

#[test]
fn bucket_fill_stops_at_a_different_colour() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::DrawLine { from: (4, 0), to: (4, 7), ink: Ink::colour(BLACK), brush: Brush::default() });
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    assert_eq!(scenario.pixel(3, 3), RED);
    assert_eq!(scenario.pixel(5, 3), CLEAR, "the wall stops the fill");
}

#[test]
fn filled_and_outlined_shapes_differ_only_inside() {
    let mut scenario = given_a_blank_sprite(16, 16);
    scenario.when(Command::DrawEllipse {
        from: (0, 0),
        to: (8, 8),
        ink: Ink::colour(RED),
        filled: false,
        brush: Brush::default(),
    });
    assert_eq!(scenario.pixel(4, 4), CLEAR);
    scenario.when(Command::DrawEllipse {
        from: (0, 0),
        to: (8, 8),
        ink: Ink::colour(RED),
        filled: true,
        brush: Brush::default(),
    });
    assert_eq!(scenario.pixel(4, 4), RED);
}

#[test]
fn the_eraser_clears_to_transparent() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    scenario.when(Command::DrawLine { from: (0, 0), to: (7, 0), ink: Ink::Erase, brush: Brush::default() });
    assert_eq!(scenario.pixel(3, 0), CLEAR);
    assert_eq!(scenario.pixel(3, 1), RED);
}

#[test]
fn spray_scatters_pixels_within_its_radius_repeatably() {
    let mut first = given_a_blank_sprite(32, 32);
    let mut second = given_a_blank_sprite(32, 32);
    for scenario in [&mut first, &mut second] {
        scenario.when(Command::Spray { at: (16, 16), ink: Ink::colour(GREEN), radius: 5, density: 20, seed: 42 });
    }
    assert_eq!(first.painted_pixels(), second.painted_pixels());
    assert!(first.painted_pixels().iter().all(|&(x, y)| (x - 16).pow(2) + (y - 16).pow(2) <= 36));
}

#[test]
fn replace_colour_swaps_one_colour_everywhere() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    scenario.when(Command::AddFrame).and(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    scenario.when(Command::ReplaceColour { from: RED, to: BLUE });
    assert_eq!(scenario.composite_pixel(0, 1, 1), BLUE);
    assert_eq!(scenario.composite_pixel(1, 1, 1), BLUE);
}

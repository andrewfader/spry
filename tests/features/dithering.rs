//! Feature 3: MS Paint-style dithers.

use crate::support::*;
use spry::editor::{Command, Ink, Pattern, Rect};

fn checker_ink(secondary: Option<spry::editor::Rgba>) -> Ink {
    Ink::Pattern { pattern: Pattern::Checker, primary: BLACK, secondary }
}

#[test]
fn a_checker_fill_paints_where_x_plus_y_is_even_and_leaves_the_rest() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::DrawRect {
        from: (0, 0),
        to: (3, 3),
        ink: checker_ink(None),
        filled: true,
        brush: Default::default(),
    });
    for (x, y) in Rect::new(0, 0, 4, 4).points() {
        let expected = if (x + y) % 2 == 0 { BLACK } else { CLEAR };
        assert_eq!(scenario.pixel(x, y), expected, "({x}, {y})");
    }
}

#[test]
fn neighbouring_pattern_strokes_tile_without_a_seam() {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::DrawRect {
        from: (0, 0),
        to: (2, 3),
        ink: checker_ink(Some(RED)),
        filled: true,
        brush: Default::default(),
    });
    scenario.when(Command::DrawRect {
        from: (3, 0),
        to: (5, 3),
        ink: checker_ink(Some(RED)),
        filled: true,
        brush: Default::default(),
    });
    for y in 0..4 {
        assert_ne!(scenario.pixel(2, y), scenario.pixel(3, y), "the checker continues across the join on row {y}");
    }
}

#[test]
fn the_dither_brush_only_changes_on_pixels_over_existing_art() {
    let mut scenario = given_a_blank_sprite(4, 1);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(GREEN), contiguous: true });
    scenario.when(Command::DrawLine { from: (0, 0), to: (3, 0), ink: checker_ink(None), brush: Default::default() });
    assert_eq!([0, 1, 2, 3].map(|x| scenario.pixel(x, 0)), [BLACK, GREEN, BLACK, GREEN]);
}

#[test]
fn two_colour_mode_paints_the_off_pixels_too() {
    let mut scenario = given_a_blank_sprite(4, 1);
    scenario.when(Command::DrawLine {
        from: (0, 0),
        to: (3, 0),
        ink: checker_ink(Some(RED)),
        brush: Default::default(),
    });
    assert_eq!([0, 1, 2, 3].map(|x| scenario.pixel(x, 0)), [BLACK, RED, BLACK, RED]);
}

#[test]
fn a_gradient_fill_dithers_from_secondary_to_primary_across_the_selection() {
    let mut scenario = given_a_blank_sprite(32, 4);
    scenario.when(Command::SelectAll);
    scenario.when(Command::GradientFill { from: (0, 0), to: (31, 0), primary: BLACK, secondary: RED });
    let blacks_in_column = |x: i32| (0..4).filter(|&y| scenario.pixel(x, y) == BLACK).count();
    assert_eq!(blacks_in_column(0), 0, "starts as the secondary colour");
    assert_eq!(blacks_in_column(31), 4, "ends as the primary colour");
    let left_half: usize = (0..16).map(blacks_in_column).sum();
    let right_half: usize = (16..32).map(blacks_in_column).sum();
    assert!(left_half < right_half);
}

#[test]
fn bayer_levels_are_available_by_name() {
    let mut scenario = given_a_blank_sprite(4, 4);
    let pattern: Pattern = "bayer4:4".parse().unwrap();
    scenario.when(Command::Fill {
        at: (0, 0),
        ink: Ink::Pattern { pattern, primary: BLACK, secondary: None },
        contiguous: true,
    });
    assert_eq!(scenario.painted_pixels().len(), 4, "level 4 of 16 lights a quarter of each tile");
}

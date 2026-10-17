//! Feature 8: palette & colour.

use crate::support::*;
use spry::editor::{Command, Ink, Palette, PixelValue};

#[test]
fn built_in_palettes_are_one_command_away() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::UseBuiltInPalette { name: "endesga 32".into() });
    assert_eq!(scenario.sprite().palette.colours.len(), 32);
    scenario.when_trying(Command::UseBuiltInPalette { name: "vaporwave".into() });
    scenario.then_it_failed_saying("built-ins are");
}

#[test]
fn shading_lightens_along_the_palette_ramp() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::UseBuiltInPalette { name: "Game Boy".into() });
    let ramp = scenario.sprite().palette.colours.clone();
    scenario.when(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: ramp[1] }] });
    scenario.when(Command::Paint {
        points: vec![(0, 0)],
        ink: Ink::Shade { steps: 1 },
        brush: Default::default(),
        pixel_perfect: false,
    });
    assert_eq!(scenario.pixel(0, 0), ramp[2], "mid became light");
}

#[test]
fn extracting_a_palette_collects_the_colours_in_use() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::SetPixels {
        pixels: vec![PixelValue { x: 0, y: 0, colour: RED }, PixelValue { x: 1, y: 0, colour: BLUE }],
    });
    scenario.when(Command::ExtractPalette);
    assert_eq!(scenario.sprite().palette.colours, vec![RED, BLUE]);
}

#[test]
fn palette_files_load_and_save_in_hex_and_gpl() {
    let scenario = given_a_blank_sprite(4, 4);
    let palette = Palette::db16();
    for name in ["db.hex", "db.gpl"] {
        let path = scenario.path(name);
        spry::io::save_palette(&path, &palette).unwrap();
        let loaded = spry::io::load_palette(&path).unwrap();
        assert_eq!(loaded.colours, palette.colours, "{name}");
        assert_eq!(loaded.name, "db", "a palette is named after its file");
    }
    let gpl = std::fs::read_to_string(scenario.path("db.gpl")).unwrap();
    assert!(gpl.starts_with("GIMP Palette"), "a .gpl path gets GIMP's format");
    assert!(spry::io::load_palette(&scenario.path("missing.hex")).is_err());
}

#[test]
fn colours_can_be_added_edited_and_removed() {
    let mut scenario = given_a_blank_sprite(4, 4);
    let before = scenario.sprite().palette.colours.len();
    scenario.when(Command::AddPaletteColour { colour: spry::editor::Rgba::rgb(1, 2, 3) });
    assert_eq!(scenario.sprite().palette.colours.len(), before + 1);
    scenario.when(Command::SetPaletteColour { index: 0, colour: RED });
    assert_eq!(scenario.sprite().palette.colours[0], RED);
    scenario.when(Command::RemovePaletteColour { index: 0 });
    assert_eq!(scenario.sprite().palette.colours.len(), before);
}

#[test]
fn snapping_a_layer_to_the_palette_leaves_only_palette_colours() {
    let mut scenario = given_a_blank_sprite(8, 1);
    let off_palette: Vec<PixelValue> =
        (0..8).map(|x| PixelValue { x, y: 0, colour: spry::editor::Rgba::rgb((x * 30) as u8, 77, 150) }).collect();
    scenario.when(Command::SetPixels { pixels: off_palette });
    scenario.when(Command::SnapToPalette { dither: false });
    let palette = scenario.sprite().palette.colours.clone();
    assert!((0..8).all(|x| palette.contains(&scenario.pixel(x, 0))));
}

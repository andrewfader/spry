//! Feature 21: tilesets and tilemaps.

use crate::support::*;
use spry::editor::tilemap::TilesetId;
use spry::editor::{Command, Ink, LayerKind, PixelValue};

/// A 16×8 sprite with the same red-and-blue 8×8 tile drawn twice.
fn two_identical_tiles() -> Scenario {
    let mut scenario = given_a_blank_sprite(16, 8);
    for left in [0, 8] {
        scenario.when(Command::DrawRect {
            from: (left, 0),
            to: (left + 7, 7),
            ink: Ink::colour(RED),
            filled: true,
            brush: Default::default(),
        });
        scenario.when(Command::SetPixels { pixels: vec![PixelValue { x: left + 2, y: 2, colour: BLUE }] });
    }
    scenario
}

fn tiles_in(scenario: &Scenario, layer: usize) -> usize {
    let LayerKind::Tilemap { tileset } = scenario.sprite().layers[layer].kind else { panic!("not a tilemap layer") };
    scenario.sprite().tileset(tileset).unwrap().tiles.len()
}

#[test]
fn converting_art_into_tiles_stores_each_distinct_tile_once_and_looks_the_same() {
    let mut scenario = two_identical_tiles();
    let before = scenario.sprite().composite(0);
    scenario.when(Command::ConvertToTilemap { tile_width: 8, tile_height: 8 });
    assert_eq!(tiles_in(&scenario, 0), 2, "the empty tile and the one red-and-blue tile");
    assert_eq!(scenario.sprite().composite(0), before);
}

#[test]
fn painting_one_copy_of_a_tile_repaints_every_copy_and_one_undo_restores_both() {
    let mut scenario = two_identical_tiles();
    scenario.when(Command::ConvertToTilemap { tile_width: 8, tile_height: 8 });
    scenario.when(Command::DrawLine { from: (0, 7), to: (7, 7), ink: Ink::colour(GREEN), brush: Default::default() });
    assert_eq!(scenario.composite_pixel(0, 12, 7), GREEN, "the second copy changed with the first");
    scenario.when(Command::Undo);
    assert_eq!(scenario.composite_pixel(0, 12, 7), RED);
    assert_eq!(scenario.composite_pixel(0, 4, 7), RED);
}

#[test]
fn tiles_are_placed_by_cell_and_cleared_with_tile_zero() {
    let mut scenario = two_identical_tiles();
    scenario.when(Command::ConvertToTilemap { tile_width: 8, tile_height: 8 });
    scenario.when(Command::PlaceTile { at: (12, 3), tile: 0, flip_x: false, flip_y: false });
    assert_eq!(scenario.composite_pixel(0, 12, 3), CLEAR, "cell (1, 0) is empty now");
    scenario.when(Command::PlaceTile { at: (9, 1), tile: 1, flip_x: true, flip_y: false });
    assert_eq!(scenario.composite_pixel(0, 13, 2), BLUE, "the tile came back mirrored: its blue dot moved right");
}

#[test]
fn a_new_tilemap_layer_grows_its_tileset_as_you_draw() {
    let mut scenario = given_a_blank_sprite(16, 16);
    scenario.when(Command::AddTilemapLayer {
        name: Some("Ground".into()),
        tileset: None,
        tile_width: 8,
        tile_height: 8,
    });
    scenario.when(Command::SetPixels {
        pixels: vec![PixelValue { x: 1, y: 1, colour: RED }, PixelValue { x: 9, y: 9, colour: RED }],
    });
    assert_eq!(tiles_in(&scenario, 1), 2, "two cells drawn alike share one new tile");
    scenario.when(Command::DrawRect {
        from: (8, 8),
        to: (15, 15),
        ink: Ink::Erase,
        filled: true,
        brush: Default::default(),
    });
    assert_eq!(scenario.composite_pixel(0, 9, 9), CLEAR, "erasing a whole cell clears that cell");
    assert_eq!(scenario.composite_pixel(0, 1, 1), RED, "but the tile itself, and its other copy, stay");
}

#[test]
fn two_layers_can_share_one_tileset() {
    let mut scenario = two_identical_tiles();
    scenario.when(Command::ConvertToTilemap { tile_width: 8, tile_height: 8 });
    let LayerKind::Tilemap { tileset: TilesetId(id) } = scenario.sprite().layers[0].kind else { panic!() };
    scenario.when(Command::AddTilemapLayer {
        name: Some("Overlay".into()),
        tileset: Some(id),
        tile_width: 8,
        tile_height: 8,
    });
    scenario.when(Command::PlaceTile { at: (0, 0), tile: 1, flip_x: false, flip_y: false });
    assert_eq!(scenario.cel_pixel(1, 0, 2, 2), BLUE);
}

#[test]
fn tile_mistakes_are_explained() {
    let mut scenario = two_identical_tiles();
    scenario.when_trying(Command::PlaceTile { at: (0, 0), tile: 1, flip_x: false, flip_y: false });
    scenario.then_it_failed_saying("tilemap layer");
    scenario.when(Command::ConvertToTilemap { tile_width: 8, tile_height: 8 });
    scenario.when_trying(Command::PlaceTile { at: (0, 0), tile: 9, flip_x: false, flip_y: false });
    scenario.then_it_failed_saying("tile 9");
    scenario.when_trying(Command::ConvertToTilemap { tile_width: 8, tile_height: 8 });
    scenario.then_it_failed_saying("only an image layer");
    scenario.when_trying(Command::AddTilemapLayer { name: None, tileset: None, tile_width: 0, tile_height: 8 });
    scenario.then_it_failed_saying("tiles run from 1");
}

#[test]
fn tilemaps_survive_saving_and_reopening() {
    let mut scenario = two_identical_tiles();
    scenario.when(Command::ConvertToTilemap { tile_width: 8, tile_height: 8 });
    let path = scenario.path("level.spry");
    scenario.studio.save(Some(&path)).unwrap();
    let reopened = spry::studio::Studio::open(&path).unwrap();
    assert_eq!(reopened.editor.sprite(), scenario.sprite());
    assert_eq!(reopened.editor.sprite().composite(0).get(10, 2), BLUE);
}

#[test]
fn resizing_the_canvas_moves_tilemaps_by_whole_cells() {
    let mut scenario = two_identical_tiles();
    scenario.when(Command::ConvertToTilemap { tile_width: 8, tile_height: 8 });
    scenario.when(Command::ResizeCanvas { width: 24, height: 16, offset_x: 8, offset_y: 8 });
    assert_eq!(scenario.composite_pixel(0, 10, 10), BLUE);
    assert_eq!(scenario.sprite().validate(), Ok(()));
}

#[test]
fn a_tileset_exports_as_an_image_and_tiled_style_maps() {
    let mut scenario = two_identical_tiles();
    scenario.when(Command::ConvertToTilemap { tile_width: 8, tile_height: 8 });
    scenario.when(Command::PlaceTile { at: (8, 0), tile: 1, flip_x: true, flip_y: false });
    let path = scenario.path("tiles.png");
    spry::io::tileset::export(&path, scenario.sprite(), None).unwrap();

    let image = spry::io::decode_image(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (8, 8), "one tile; the empty tile isn't drawn");
    assert_eq!(image.get(2, 2), BLUE);
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path.with_extension("json")).unwrap()).unwrap();
    assert_eq!(json["tileset"]["tile_count"], 1);
    assert_eq!(
        json["layers"][0]["frames"][0]["data"],
        serde_json::json!([1, 0x8000_0001u32]),
        "1 = first tile; bit 31 = mirrored"
    );
}

#[test]
fn exporting_tiles_from_a_sprite_without_a_tilemap_says_so() {
    let scenario = two_identical_tiles();
    let error = spry::io::tileset::export(&scenario.path("tiles.png"), scenario.sprite(), None).err().unwrap();
    assert!(error.to_string().contains("no tilemap layer"));
}

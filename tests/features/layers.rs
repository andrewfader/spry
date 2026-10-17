//! Feature 5: layers.

use crate::support::*;
use spry::editor::{Command, Ink, PixelValue};

fn dot(colour: spry::editor::Rgba) -> Command {
    Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour }] }
}

#[test]
fn a_half_opaque_top_layer_blends_over_the_one_below() {
    let mut scenario = given_a_blank_sprite(1, 1);
    scenario.when(dot(BLACK)).and(Command::AddLayer { name: Some("glaze".into()) }).and(dot(spry::editor::Rgba::WHITE));
    scenario.when(Command::SetLayerOpacity { opacity: 128 });
    let blended = scenario.composite_pixel(0, 0, 0);
    assert!((126..=130).contains(&blended.r()), "got {blended}");
}

#[test]
fn hidden_layers_drop_out_of_the_composite() {
    let mut scenario = given_a_blank_sprite(1, 1);
    scenario.when(dot(RED)).and(Command::AddLayer { name: None }).and(dot(BLUE));
    scenario.when(Command::SetLayerVisible { visible: false });
    assert_eq!(scenario.composite_pixel(0, 0, 0), RED);
}

#[test]
fn layers_can_be_renamed_reordered_duplicated_merged_and_removed() {
    let mut scenario = given_a_blank_sprite(2, 1);
    scenario.when(dot(RED)).and(Command::AddLayer { name: None }).and(Command::RenameLayer { name: "line art".into() });
    scenario.when(Command::SetPixels { pixels: vec![PixelValue { x: 1, y: 0, colour: BLUE }] });
    scenario.when(Command::MoveLayer { to: 0 });
    assert_eq!(scenario.sprite().layers[0].name, "line art");
    scenario.when(Command::DuplicateLayer);
    assert_eq!(scenario.sprite().layers.len(), 3);
    assert_eq!(scenario.sprite().layers[1].name, "line art copy");
    scenario.when(Command::RemoveLayer);
    assert_eq!(scenario.sprite().layers.len(), 2);
    scenario.when(Command::SelectLayer { index: 1 }).and(Command::MergeDown);
    assert_eq!(scenario.sprite().layers.len(), 1);
    assert_eq!(scenario.composite_pixel(0, 0, 0), RED);
    assert_eq!(scenario.composite_pixel(0, 1, 0), BLUE);
}

#[test]
fn the_last_layer_cannot_be_removed() {
    let mut scenario = given_a_blank_sprite(2, 2);
    scenario.when_trying(Command::RemoveLayer);
    scenario.then_it_failed_saying("at least one layer");
}

#[test]
fn new_layers_go_above_the_current_one_and_become_current() {
    let mut scenario = given_a_blank_sprite(2, 2);
    scenario.when(Command::AddLayer { name: None });
    assert_eq!(scenario.studio.editor.layer, 1);
    assert_eq!(scenario.sprite().layers[1].name, "Layer 2");
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    assert!(scenario.sprite().cel_image(0, 0).is_none(), "the layer below is untouched");
}

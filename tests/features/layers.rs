//! Feature 5: layers.

use crate::support::*;
use spry::editor::{BlendMode, Command, Ink, PixelValue};

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

#[test]
fn a_multiply_layer_darkens_what_is_below_it() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::Fill {
        at: (0, 0),
        ink: Ink::colour(spry::editor::Rgba::rgb(255, 128, 0)),
        contiguous: true,
    });
    scenario.when(Command::AddLayer { name: Some("Shadow".into()) });
    scenario.when(Command::Fill {
        at: (0, 0),
        ink: Ink::colour(spry::editor::Rgba::rgb(128, 128, 128)),
        contiguous: true,
    });
    scenario.when(Command::SetLayerBlend { blend: BlendMode::Multiply });
    assert_eq!(scenario.composite_pixel(0, 1, 1), spry::editor::Rgba::rgb(128, 64, 0));
}

#[test]
fn a_cel_can_fade_on_its_own_frame_only() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    scenario.when(Command::DuplicateFrame { as_clone: true });
    scenario.when(Command::SetCelOpacity { opacity: 64 });
    assert_eq!(scenario.composite_pixel(1, 0, 0).a(), 64, "the clone on frame 2 fades");
    assert_eq!(scenario.composite_pixel(0, 0, 0).a(), 255, "the original on frame 1 doesn't");
    scenario.when(Command::AddFrame);
    scenario.when_trying(Command::SetCelOpacity { opacity: 10 });
    scenario.then_it_failed_saying("empty");
}

#[test]
fn layers_go_into_groups_and_hiding_the_group_hides_them() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::AddGroup { name: Some("Head".into()) });
    let group = scenario.studio.editor.layer;
    scenario.when(Command::AddLayer { name: Some("Face".into()) }).and(Command::SelectLayer { index: 2 });
    scenario.when(Command::SetLayerParent { group: Some(group) });
    let face = scenario.studio.editor.layer;
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(BLUE), contiguous: true });
    let names: Vec<&str> = scenario.sprite().layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, vec!["Layer 1", "Face", "Head"]);
    assert_eq!(scenario.sprite().depth(face), 1);
    assert_eq!(scenario.composite_pixel(0, 0, 0), BLUE);

    let head = scenario.sprite().layer_index(scenario.sprite().layers[2].id).unwrap();
    scenario.when(Command::SelectLayer { index: head }).and(Command::SetLayerVisible { visible: false });
    assert_eq!(scenario.composite_pixel(0, 0, 0), CLEAR);
}

#[test]
fn a_group_holds_layers_not_pixels() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::AddGroup { name: None });
    scenario.when_trying(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    scenario.then_it_failed_saying("is a group");
}

#[test]
fn deleting_a_group_takes_its_layers_and_undo_brings_them_back() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::AddGroup { name: Some("Props".into()) });
    let group = scenario.studio.editor.layer;
    scenario.when(Command::AddLayer { name: Some("Sword".into()) }).and(Command::SetLayerParent { group: Some(group) });
    let props = scenario.sprite().layers.iter().position(|l| l.name == "Props").unwrap();
    scenario.when(Command::SelectLayer { index: props }).and(Command::RemoveLayer);
    assert_eq!(scenario.sprite().layers.len(), 1);
    scenario.when(Command::Undo);
    assert_eq!(scenario.sprite().layers.len(), 3);
}

#[test]
fn groups_blends_and_cel_opacity_survive_saving() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    scenario.when(Command::SetCelOpacity { opacity: 77 }).and(Command::SetLayerBlend { blend: BlendMode::Screen });
    scenario.when(Command::AddGroup { name: Some("Group".into()) });
    let path = scenario.path("grouped.spry");
    scenario.studio.save(Some(&path)).unwrap();
    let reopened = spry::studio::Studio::open(&path).unwrap();
    assert_eq!(reopened.editor.sprite(), scenario.sprite());
}

#[test]
fn files_from_before_cel_opacity_still_open() {
    let mut sprite = serde_json::to_value(spry::editor::Sprite::new(2, 2)).unwrap();
    sprite["images"] = serde_json::json!({ "1": { "w": 2, "h": 2, "rgba": base64_of_red_square() } });
    sprite["layers"][0]["cels"] = serde_json::json!([1]);
    sprite["next_image_id"] = serde_json::json!(2);
    for key in ["blend", "kind", "parent"] {
        sprite["layers"][0].as_object_mut().unwrap().remove(key);
    }
    let sprite: spry::editor::Sprite = serde_json::from_value(sprite).unwrap();
    assert_eq!(sprite.cel(0, 0).unwrap().opacity, 255);
    assert_eq!(sprite.composite(0).get(1, 1), RED);
}

fn base64_of_red_square() -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(RED.0.repeat(4))
}

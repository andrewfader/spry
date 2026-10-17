//! Feature 10: versioning.

use crate::support::*;
use spry::editor::document::changed_pixels;
use spry::editor::{Command, Ink, PixelValue};

fn with_two_versions() -> Scenario {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.studio.editor.set_clock(|| 1_760_000_000);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(BLUE), contiguous: true });
    scenario.when(Command::CommitVersion { message: "base".into() });
    scenario.when(Command::SetPixels { pixels: vec![PixelValue { x: 4, y: 4, colour: RED }] });
    scenario.when(Command::CommitVersion { message: "armour".into() });
    scenario
}

#[test]
fn restoring_v1_brings_back_its_canvas_and_keeps_v2() {
    let mut scenario = with_two_versions();
    scenario.when(Command::RestoreVersion { id: 1 });
    assert_eq!(scenario.pixel(4, 4), BLUE);
    let messages: Vec<&str> = scenario.studio.editor.document.versions.iter().map(|v| v.message.as_str()).collect();
    assert_eq!(messages, vec!["base", "armour"]);
}

#[test]
fn restoring_is_undoable() {
    let mut scenario = with_two_versions();
    scenario.when(Command::RestoreVersion { id: 1 }).and(Command::Undo);
    assert_eq!(scenario.pixel(4, 4), RED);
}

#[test]
fn versions_record_when_they_were_made() {
    let scenario = with_two_versions();
    assert!(scenario.studio.editor.document.versions.iter().all(|v| v.created_at == 1_760_000_000));
}

#[test]
fn saving_records_a_version_unless_nothing_changed() {
    let mut scenario = given_a_blank_sprite(8, 8);
    let path = scenario.path("knight.spry");
    scenario.studio.save(Some(&path)).unwrap();
    scenario.studio.save(None).unwrap();
    assert_eq!(scenario.studio.editor.document.versions.len(), 1);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    scenario.studio.save(None).unwrap();
    assert_eq!(scenario.studio.editor.document.versions.len(), 2);
}

#[test]
fn the_visual_diff_highlights_exactly_the_changed_pixels() {
    let scenario = with_two_versions();
    let versions = &scenario.studio.editor.document.versions;
    let before = versions[0].sprite.composite(0);
    let after = versions[1].sprite.composite(0);
    assert_eq!(changed_pixels(&before, &after), vec![(4, 4)]);
}

#[test]
fn restoring_a_missing_version_is_refused() {
    let mut scenario = with_two_versions();
    scenario.when_trying(Command::RestoreVersion { id: 99 });
    scenario.then_it_failed_saying("version 99");
}

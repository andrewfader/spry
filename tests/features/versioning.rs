//! Feature 10: versioning.

use crate::support::*;
use spry::editor::document::{changed_pixels, difference_overlay};
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

#[test]
fn branching_a_version_opens_it_as_a_new_document_with_its_history() {
    let scenario = with_two_versions();
    let branch = scenario.studio.editor.document.branch(1).expect("version 1 exists");
    assert_eq!(branch.sprite.cel_image(0, 0).unwrap().get(4, 4), BLUE);
    let messages: Vec<&str> = branch.versions.iter().map(|v| v.message.as_str()).collect();
    assert_eq!(messages, vec!["base"], "the branch forgets what came after it");
    assert!(scenario.studio.editor.document.branch(99).is_none());
}

#[test]
fn a_branch_numbers_its_new_versions_after_the_one_it_came_from() {
    let scenario = with_two_versions();
    let mut branch = scenario.studio.editor.document.branch(1).unwrap();
    branch.sprite.notes = "a new direction".into();
    assert_eq!(branch.commit_version("fork", 0), Some(2));
    assert_eq!(branch.validate(), Ok(()));
}

#[test]
fn the_visual_diff_highlights_only_what_changed() {
    let scenario = with_two_versions();
    let document = &scenario.studio.editor.document;
    let (base, armour) =
        (document.version(1).unwrap().sprite.composite(0), document.version(2).unwrap().sprite.composite(0));
    let overlay = difference_overlay(&base, &armour);
    assert_eq!(overlay.get(4, 4), RED, "the new armour pixel shows in its colour");
    assert!(overlay.get(0, 0).a() < 64, "unchanged pixels fade into the background");
    let erased = difference_overlay(&armour, &spry::editor::Pixels::new(8, 8));
    assert_eq!(erased.get(4, 4), spry::editor::Rgba::rgb(255, 0, 255), "erased pixels show in magenta");
}

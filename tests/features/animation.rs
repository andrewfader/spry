//! Feature 6: frames, animation & onion skin.

use crate::support::*;
use spry::editor::playback::{frame_at, play_order};
use spry::editor::{Command, Ink, LoopDirection};

fn three_frames(durations: [u32; 3]) -> Scenario {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::SetFrameDuration { duration_ms: durations[0], all_frames: false });
    for &duration in &durations[1..] {
        scenario.when(Command::AddFrame).and(Command::SetFrameDuration { duration_ms: duration, all_frames: false });
    }
    scenario
}

#[test]
fn playback_shows_the_frame_whose_time_it_is() {
    let scenario = three_frames([100, 200, 100]);
    assert_eq!(frame_at(scenario.sprite(), None, 250), 1, "250 ms in, frame 2 is showing");
    assert_eq!(frame_at(scenario.sprite(), None, 320), 2);
    assert_eq!(frame_at(scenario.sprite(), None, 420), 0, "and it loops");
}

#[test]
fn a_ping_pong_tag_bounces_between_its_ends() {
    let mut scenario = three_frames([100, 100, 100]);
    scenario.when(Command::AddTag { name: "bob".into(), from: 0, to: 2, direction: LoopDirection::PingPong });
    let tag = &scenario.sprite().tags[0];
    assert_eq!(play_order(scenario.sprite(), Some(tag)), vec![0, 1, 2, 1]);
    let shown: Vec<usize> = (0..6).map(|step| frame_at(scenario.sprite(), Some(tag), step * 100)).collect();
    assert_eq!(shown, vec![0, 1, 2, 1, 0, 1]);
}

#[test]
fn frames_can_be_added_duplicated_moved_and_removed() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    scenario.when(Command::DuplicateFrame { as_clone: false }).and(Command::Fill {
        at: (0, 0),
        ink: Ink::colour(BLUE),
        contiguous: true,
    });
    scenario.when(Command::MoveFrame { to: 0 });
    assert_eq!(scenario.composite_pixel(0, 0, 0), BLUE);
    assert_eq!(scenario.composite_pixel(1, 0, 0), RED);
    scenario.when(Command::RemoveFrame);
    assert_eq!(scenario.sprite().frame_count(), 1);
    assert_eq!(scenario.composite_pixel(0, 0, 0), RED);
}

#[test]
fn every_frame_can_share_one_duration() {
    let mut scenario = three_frames([50, 60, 70]);
    scenario.when(Command::SetFrameDuration { duration_ms: 80, all_frames: true });
    assert!(scenario.sprite().frames.iter().all(|f| f.duration_ms == 80));
    scenario.when_trying(Command::SetFrameDuration { duration_ms: 0, all_frames: false });
    scenario.then_it_failed_saying("at least 1 ms");
}

#[test]
fn tags_must_name_frames_that_exist() {
    let mut scenario = three_frames([100, 100, 100]);
    scenario.when_trying(Command::AddTag { name: "run".into(), from: 0, to: 9, direction: LoopDirection::Forward });
    scenario.then_it_failed_saying("frame 9");
}

#[test]
fn re_adding_a_tag_by_name_changes_its_direction_without_duplicating_it() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::AddFrame).and(Command::AddFrame);
    let tag = |direction| Command::AddTag { name: "walk".into(), from: 0, to: 2, direction };
    scenario.when(tag(LoopDirection::Forward)).and(tag(LoopDirection::PingPong));
    let tags = &scenario.sprite().tags;
    assert_eq!(tags.len(), 1);
    assert_eq!((tags[0].from, tags[0].to, tags[0].direction), (0, 2, LoopDirection::PingPong));
}

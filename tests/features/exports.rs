//! Feature 12: exports.

use crate::support::*;
use spry::editor::{Command, Ink, LoopDirection, PixelValue};
use spry::io;

fn three_frame_animation() -> Scenario {
    let mut scenario = given_a_blank_sprite(8, 8);
    let colours = [RED, GREEN, BLUE];
    for (frame, colour) in colours.into_iter().enumerate() {
        if frame > 0 {
            scenario.when(Command::AddFrame);
        }
        scenario.when(Command::SetPixels { pixels: vec![PixelValue { x: frame as i32, y: frame as i32, colour }] });
        scenario.when(Command::SetFrameDuration { duration_ms: 100 + 50 * frame as u32, all_frames: false });
    }
    scenario
}

#[test]
fn a_gif_decodes_to_the_same_frames_pixels_and_delays() {
    let scenario = three_frame_animation();
    let path = scenario.path("anim.gif");
    io::export_gif(&path, scenario.sprite(), None, 1).unwrap();
    let decoded = io::decode_gif(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(decoded.len(), 3);
    for (frame, (image, delay)) in decoded.iter().enumerate() {
        assert_eq!(image, &scenario.sprite().composite(frame), "frame {frame}");
        assert_eq!(*delay, scenario.sprite().frames[frame].duration_ms);
    }
}

#[test]
fn a_gif_of_a_tag_plays_only_that_tag() {
    let mut scenario = three_frame_animation();
    scenario.when(Command::AddTag { name: "end".into(), from: 1, to: 2, direction: LoopDirection::Reverse });
    let path = scenario.path("end.gif");
    io::export_gif(&path, scenario.sprite(), Some("end"), 1).unwrap();
    let decoded = io::decode_gif(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        decoded.iter().map(|(image, _)| image.clone()).collect::<Vec<_>>(),
        vec![scenario.sprite().composite(2), scenario.sprite().composite(1)]
    );
}

#[test]
fn a_png_export_scales_with_nearest_neighbour_only() {
    let mut scenario = given_a_blank_sprite(4, 4);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true });
    scenario.when(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: BLUE }] });
    let path = scenario.path("big.png");
    io::export_png(&path, scenario.sprite(), 0, 4).unwrap();
    let image = io::decode_image(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (16, 16));
    assert_eq!(image.get(3, 3), BLUE);
    assert_eq!(image.get(4, 4), RED, "no blending at the block edge");
    let mut colours: Vec<_> = image.colours().to_vec();
    colours.sort_by_key(|c| c.to_hex());
    colours.dedup();
    assert_eq!(colours.len(), 2);
}

#[test]
fn a_png_sequence_writes_one_numbered_file_per_frame() {
    let scenario = three_frame_animation();
    let folder = scenario.path("frames");
    let files = io::export_png_sequence(&folder, "knight", scenario.sprite(), 1).unwrap();
    let names: Vec<String> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["knight_000.png", "knight_001.png", "knight_002.png"]);
    let second = io::decode_image(&std::fs::read(&files[1]).unwrap()).unwrap();
    assert_eq!(second, scenario.sprite().composite(1));
}

#[test]
fn exporting_a_missing_tag_is_an_error() {
    let scenario = three_frame_animation();
    let error = io::export_gif(&scenario.path("x.gif"), scenario.sprite(), Some("jump"), 1).unwrap_err();
    assert!(error.to_string().contains("no tag called \"jump\""));
}

#[test]
fn an_apng_keeps_every_frame_and_its_exact_delay() {
    let mut scenario = three_frame_animation();
    scenario.when(Command::SetFrameDuration { duration_ms: 15, all_frames: true });
    let path = scenario.path("walk.png");
    io::export_apng(&path, scenario.sprite(), None, 1).unwrap();
    let frames = io::decode_apng(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(frames.len(), 3);
    for (index, (image, delay)) in frames.iter().enumerate() {
        assert_eq!(image, &scenario.sprite().composite(index));
        assert_eq!(*delay, 15, "APNG keeps the 15 ms a GIF would round");
    }
}

#[test]
fn a_frame_exports_as_lossless_webp() {
    let scenario = three_frame_animation();
    let path = scenario.path("frame.webp");
    io::export_webp(&path, scenario.sprite(), 1, 2).unwrap();
    let image = io::decode_image(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(image, scenario.sprite().composite(1).scaled_up(2));
}

#[test]
fn an_animated_webp_keeps_every_frame_and_its_exact_delay() {
    let mut scenario = three_frame_animation();
    scenario.when(Command::SetFrameDuration { duration_ms: 15, all_frames: true });
    let path = scenario.path("walk.webp");
    io::export_animated_webp(&path, scenario.sprite(), None, 2).unwrap();
    let frames = io::decode_animated_webp(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(frames.len(), 3);
    for (index, (image, delay)) in frames.iter().enumerate() {
        assert_eq!(image, &scenario.sprite().composite(index).scaled_up(2));
        assert_eq!(*delay, 15);
    }
}

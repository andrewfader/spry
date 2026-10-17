//! The `spry` command line: headless info and exports for build pipelines.

use crate::support::*;
use spry::editor::{Command, Ink};
use std::process::Command as Process;

fn spry(arguments: &[&str]) -> std::process::Output {
    Process::new(env!("CARGO_BIN_EXE_spry")).args(arguments).output().expect("spry runs")
}

fn saved_sprite() -> (Scenario, std::path::PathBuf) {
    let mut scenario = given_a_blank_sprite(8, 8);
    scenario.when(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true }).and(Command::AddFrame);
    let path = scenario.path("cli.spry");
    scenario.studio.save(Some(&path)).unwrap();
    (scenario, path)
}

#[test]
fn spry_export_writes_a_sheet_without_a_window() {
    let (scenario, source) = saved_sprite();
    let sheet = scenario.path("cli-sheet.png");
    let output = spry(&[
        "export",
        source.to_str().unwrap(),
        "--sheet",
        sheet.to_str().unwrap(),
        "--layout",
        "horizontal",
        "--scale",
        "2",
    ]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let image = spry::io::decode_image(&std::fs::read(&sheet).unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (32, 16));
    assert!(sheet.with_extension("json").exists());
}

#[test]
fn spry_info_describes_a_sprite() {
    let (_scenario, source) = saved_sprite();
    let output = spry(&["info", source.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("8×8") && text.contains("frames: 2"), "{text}");
}

#[test]
fn mistakes_exit_with_a_usage_hint() {
    let output = spry(&["export", "nowhere.spry"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--png"));
    let help = spry(&["--help"]);
    assert!(String::from_utf8_lossy(&help.stdout).contains("spry mcp"));
}

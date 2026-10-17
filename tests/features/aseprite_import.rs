//! Feature 20: opening Aseprite files.
//!
//! The files are built byte by byte from the published Aseprite file spec
//! by the small writer below, then opened through the real `Studio::open`.

use crate::support::*;
use spry::editor::{LoopDirection, Rgba};
use spry::studio::Studio;
use std::io::Write;

const LAYER: u16 = 0x2004;
const CEL: u16 = 0x2005;
const TAGS: u16 = 0x2018;
const PALETTE: u16 = 0x2019;
const SLICE: u16 = 0x2022;

/// Little-endian byte building, the way the spec lays fields out.
#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn byte(mut self, value: u8) -> Self {
        self.0.push(value);
        self
    }
    fn word(mut self, value: u16) -> Self {
        self.0.extend(value.to_le_bytes());
        self
    }
    fn short(mut self, value: i16) -> Self {
        self.0.extend(value.to_le_bytes());
        self
    }
    fn dword(mut self, value: u32) -> Self {
        self.0.extend(value.to_le_bytes());
        self
    }
    fn long(mut self, value: i32) -> Self {
        self.0.extend(value.to_le_bytes());
        self
    }
    fn zeros(mut self, count: usize) -> Self {
        self.0.extend(std::iter::repeat_n(0, count));
        self
    }
    fn string(self, text: &str) -> Self {
        let mut this = self.word(text.len() as u16);
        this.0.extend(text.as_bytes());
        this
    }
    fn raw(mut self, bytes: &[u8]) -> Self {
        self.0.extend(bytes);
        self
    }
}

struct AsepriteFile {
    width: u16,
    height: u16,
    depth: u16,
    transparent_index: u8,
    /// One list of chunks per frame, with the frame's duration.
    frames: Vec<(u16, Vec<Vec<u8>>)>,
}

impl AsepriteFile {
    fn rgba(width: u16, height: u16) -> Self {
        AsepriteFile { width, height, depth: 32, transparent_index: 0, frames: Vec::new() }
    }

    fn frame(mut self, duration: u16, chunks: Vec<Vec<u8>>) -> Self {
        self.frames.push((duration, chunks));
        self
    }

    fn bytes(&self) -> Vec<u8> {
        let frames: Vec<u8> = self
            .frames
            .iter()
            .flat_map(|(duration, chunks)| {
                let body: Vec<u8> = chunks.concat();
                Bytes::default()
                    .dword(16 + body.len() as u32)
                    .word(0xF1FA)
                    .word(chunks.len() as u16)
                    .word(*duration)
                    .zeros(2)
                    .dword(chunks.len() as u32)
                    .raw(&body)
                    .0
            })
            .collect();
        Bytes::default()
            .dword(128 + frames.len() as u32)
            .word(0xA5E0)
            .word(self.frames.len() as u16)
            .word(self.width)
            .word(self.height)
            .word(self.depth)
            .dword(1) // layer opacity is valid
            .word(100)
            .dword(0)
            .dword(0)
            .byte(self.transparent_index)
            .zeros(3)
            .word(0)
            .byte(1)
            .byte(1)
            .short(0)
            .short(0)
            .word(16)
            .word(16)
            .zeros(84)
            .raw(&frames)
            .0
    }

    fn save_in(&self, scenario: &Scenario, name: &str) -> std::path::PathBuf {
        let path = scenario.path(name);
        std::fs::write(&path, self.bytes()).unwrap();
        path
    }
}

fn chunk(kind: u16, body: Bytes) -> Vec<u8> {
    Bytes::default().dword(6 + body.0.len() as u32).word(kind).raw(&body.0).0
}

/// `flags`: 1 visible, 2 editable. `kind`: 0 image, 1 group.
fn layer(name: &str, flags: u16, kind: u16, blend: u16, opacity: u8) -> Vec<u8> {
    let body = Bytes::default().word(flags).word(kind).word(0).word(0).word(0).word(blend).byte(opacity).zeros(3);
    chunk(LAYER, body.string(name))
}

fn cel_header(layer: u16, x: i16, y: i16, opacity: u8, kind: u16) -> Bytes {
    Bytes::default().word(layer).short(x).short(y).byte(opacity).word(kind).short(0).zeros(5)
}

fn raw_cel(layer: u16, (x, y): (i16, i16), opacity: u8, size: (u16, u16), pixels: &[u8]) -> Vec<u8> {
    chunk(CEL, cel_header(layer, x, y, opacity, 0).word(size.0).word(size.1).raw(pixels))
}

fn compressed_cel(layer: u16, (x, y): (i16, i16), size: (u16, u16), pixels: &[u8]) -> Vec<u8> {
    let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    zlib.write_all(pixels).unwrap();
    chunk(CEL, cel_header(layer, x, y, 255, 2).word(size.0).word(size.1).raw(&zlib.finish().unwrap()))
}

fn linked_cel(layer: u16, to_frame: u16) -> Vec<u8> {
    chunk(CEL, cel_header(layer, 0, 0, 255, 1).word(to_frame))
}

fn tag(name: &str, from: u16, to: u16, direction: u8) -> Vec<u8> {
    let body = Bytes::default().word(1).zeros(8).word(from).word(to).byte(direction).word(0).zeros(6).zeros(4);
    chunk(TAGS, body.string(name))
}

fn palette(colours: &[[u8; 4]]) -> Vec<u8> {
    let mut body = Bytes::default().dword(colours.len() as u32).dword(0).dword(colours.len() as u32 - 1).zeros(8);
    for [r, g, b, a] in colours {
        body = body.word(0).byte(*r).byte(*g).byte(*b).byte(*a);
    }
    chunk(PALETTE, body)
}

fn slice(name: &str, x: i32, y: i32, w: u32, h: u32) -> Vec<u8> {
    let body = Bytes::default().dword(1).dword(0).dword(0).string(name).dword(0).long(x).long(y).dword(w).dword(h);
    chunk(SLICE, body)
}

const RED_PX: [u8; 4] = [255, 0, 77, 255];
const BLUE_PX: [u8; 4] = [41, 173, 255, 255];

fn knight() -> AsepriteFile {
    let red_square: Vec<u8> = RED_PX.repeat(4);
    AsepriteFile::rgba(8, 8)
        .frame(
            100,
            vec![
                layer("Body", 3, 0, 0, 255),
                layer("Eyes", 2, 0, 0, 128),
                compressed_cel(0, (2, 3), (2, 2), &red_square),
                raw_cel(1, (0, 0), 255, (1, 1), &BLUE_PX),
                tag("walk", 0, 1, 2),
                slice("hand", 1, 2, 3, 4),
            ],
        )
        .frame(150, vec![linked_cel(0, 0)])
}

#[test]
fn an_aseprite_file_opens_with_its_layers_cels_clones_and_tags() {
    let scenario = given_a_blank_sprite(1, 1);
    let studio = Studio::open(&knight().save_in(&scenario, "knight.aseprite")).unwrap();
    let sprite = studio.editor.sprite();

    assert_eq!((sprite.width, sprite.height), (8, 8));
    let names: Vec<&str> = sprite.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, vec!["Body", "Eyes"]);
    assert!(!sprite.layers[1].visible, "Eyes was hidden in Aseprite");
    assert_eq!(sprite.layers[1].opacity, 128);
    let body = sprite.cel_image(0, 0).unwrap();
    assert_eq!((body.get(2, 3), body.get(3, 4)), (RED, RED), "the cel sits at its offset");
    assert_eq!(body.get(1, 3), CLEAR);
    assert!(sprite.is_linked(0, 1), "the linked cel stays a clone");
    assert_eq!(sprite.cel_image(1, 0).unwrap().get(0, 0), BLUE);
    let durations: Vec<u32> = sprite.frames.iter().map(|f| f.duration_ms).collect();
    assert_eq!(durations, vec![100, 150]);
    assert_eq!((sprite.tags[0].name.as_str(), sprite.tags[0].direction), ("walk", LoopDirection::PingPong));
    assert_eq!(sprite.slices[0].bounds, spry::editor::Rect::new(1, 2, 3, 4));
    assert!(studio.path.is_none(), "it opens as a new document, so saving never overwrites the .aseprite");
}

#[test]
fn indexed_colour_files_use_their_palette_and_transparent_index() {
    let scenario = given_a_blank_sprite(1, 1);
    let mut file = AsepriteFile { depth: 8, transparent_index: 0, ..AsepriteFile::rgba(2, 2) };
    file = file.frame(
        100,
        vec![
            palette(&[[0, 0, 0, 255], RED_PX, BLUE_PX]),
            layer("Ink", 1, 0, 0, 255),
            raw_cel(0, (0, 0), 255, (2, 2), &[0, 1, 2, 1]),
        ],
    );
    let studio = Studio::open(&file.save_in(&scenario, "tiny.ase")).unwrap();
    let image = studio.editor.sprite().cel_image(0, 0).unwrap();
    assert_eq!([image.get(0, 0), image.get(1, 0), image.get(0, 1)], [CLEAR, RED, BLUE]);
    assert!(studio.editor.sprite().palette.colours.contains(&RED));
}

#[test]
fn cel_opacity_is_baked_into_the_pixels() {
    let scenario = given_a_blank_sprite(1, 1);
    let file = AsepriteFile::rgba(2, 2)
        .frame(100, vec![layer("Glow", 1, 0, 0, 255), raw_cel(0, (0, 0), 128, (1, 1), &RED_PX)]);
    let studio = Studio::open(&file.save_in(&scenario, "glow.aseprite")).unwrap();
    assert_eq!(studio.editor.sprite().cel_image(0, 0).unwrap().get(0, 0), Rgba::rgba(255, 0, 77, 128));
}

#[test]
fn what_spry_cannot_represent_is_named_not_silently_dropped() {
    let scenario = given_a_blank_sprite(1, 1);
    let file = AsepriteFile::rgba(4, 4).frame(
        100,
        vec![
            layer("Arm", 1, 1, 0, 255),
            layer("Sleeve", 1, 0, 0, 255),
            layer("Shine", 1, 0, 9, 255),
            raw_cel(1, (0, 0), 255, (1, 1), &RED_PX),
        ],
    );
    let studio = Studio::open(&file.save_in(&scenario, "arm.aseprite")).unwrap();
    let names: Vec<&str> = studio.editor.sprite().layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, vec!["Sleeve", "Shine"], "the group is gone but its children stay");
    assert_eq!(studio.editor.sprite().cel_image(0, 0).unwrap().get(0, 0), RED, "cels still find their layer");
    let report = studio.notices.join("\n");
    assert!(report.contains("group") && report.contains("Arm"), "{report}");
    assert!(report.contains("blend") && report.contains("Shine"), "{report}");
}

#[test]
fn damaged_aseprite_files_are_refused_with_a_reason() {
    let scenario = given_a_blank_sprite(1, 1);
    let whole = knight().bytes();
    for (name, bytes) in [
        ("truncated.aseprite", whole[..whole.len() - 7].to_vec()),
        ("garbage.aseprite", b"definitely not an aseprite file at all, not even close".to_vec()),
        ("huge.aseprite", {
            let mut huge = whole.clone();
            huge[8..12].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]); // 65535 × 65535 canvas
            huge
        }),
    ] {
        let path = scenario.path(name);
        std::fs::write(&path, bytes).unwrap();
        let error = Studio::open(&path).err().unwrap_or_else(|| panic!("{name} should be refused"));
        assert!(!format!("{error:#}").is_empty());
    }
}

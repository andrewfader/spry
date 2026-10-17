//! Reading `.aseprite` / `.ase` files, following Aseprite's published file
//! spec (`docs/ase-file-specs.md` in the Aseprite repository).
//!
//! An Aseprite file is a 128-byte header, then one block per frame, each
//! holding typed chunks: layers, cels, tags, palette, slices and more.
//! Spry keeps what it can represent faithfully and names everything else in
//! the import's `notices`, so a group or blend mode never disappears
//! without a word.
//!
//! Every length in the file is checked before it's trusted: a truncated or
//! hostile file is an error, never a panic or a huge allocation.

use crate::editor::limits::check_canvas_size;
use crate::editor::palette::Palette;
use crate::editor::sprite::{LoopDirection, Slice, Tag};
use crate::editor::{Document, Pixels, Rect, Rgba, Sprite};
use anyhow::{bail, ensure, Context, Result};
use std::io::Read;

const HEADER_MAGIC: u16 = 0xA5E0;
const FRAME_MAGIC: u16 = 0xF1FA;
const HEADER_SIZE: usize = 128;

const CHUNK_LAYER: u16 = 0x2004;
const CHUNK_CEL: u16 = 0x2005;
const CHUNK_TAGS: u16 = 0x2018;
const CHUNK_PALETTE: u16 = 0x2019;
const CHUNK_SLICE: u16 = 0x2022;

const LAYER_VISIBLE: u16 = 1;
const LAYER_EDITABLE: u16 = 2;
const HEADER_LAYER_OPACITY_VALID: u32 = 1;

/// A decoded file, plus plain-English notes about anything that couldn't
/// come across exactly.
pub struct Import {
    pub document: Document,
    pub notices: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum ColourDepth {
    Rgba,
    Grayscale,
    Indexed { transparent: u8 },
}

impl ColourDepth {
    fn bytes_per_pixel(self) -> usize {
        match self {
            ColourDepth::Rgba => 4,
            ColourDepth::Grayscale => 2,
            ColourDepth::Indexed { .. } => 1,
        }
    }
}

/// Where each Aseprite layer went: Spry has no groups, so their indexes
/// shift once groups are dropped.
enum LayerSlot {
    Spry(usize),
    Group,
}

struct Decoder {
    depth: ColourDepth,
    layer_opacity_valid: bool,
    sprite: Sprite,
    layer_slots: Vec<LayerSlot>,
    palette: Vec<Rgba>,
    /// Tags name frame ranges, so they wait until every frame exists.
    /// Added earlier, inserting later frames would shift them.
    tags: Vec<Tag>,
    notices: Vec<String>,
}

pub fn decode(bytes: &[u8]) -> Result<Import> {
    let mut file = Reader::new(bytes);
    let header = Header::read(&mut file)?;
    check_canvas_size(header.width, header.height).map_err(anyhow::Error::msg)?;
    let mut decoder = Decoder {
        depth: header.depth,
        layer_opacity_valid: header.flags & HEADER_LAYER_OPACITY_VALID != 0,
        sprite: Sprite::new(header.width, header.height),
        layer_slots: Vec::new(),
        palette: Vec::new(),
        tags: Vec::new(),
        notices: Vec::new(),
    };
    decoder.sprite.layers.clear();
    for index in 0..header.frames {
        decoder.read_frame(&mut file, index as usize).with_context(|| format!("reading frame {}", index + 1))?;
    }
    decoder.finish()
}

struct Header {
    frames: u16,
    width: u32,
    height: u32,
    depth: ColourDepth,
    flags: u32,
}

impl Header {
    fn read(file: &mut Reader) -> Result<Header> {
        let header = file.take(HEADER_SIZE).context("this file is too short to be an Aseprite file")?;
        let mut header = Reader::new(header);
        let _file_size = header.dword()?;
        ensure!(header.word()? == HEADER_MAGIC, "this isn't an Aseprite file (the magic number is wrong)");
        let frames = header.word()?;
        let (width, height) = (u32::from(header.word()?), u32::from(header.word()?));
        let depth = header.word()?;
        let flags = header.dword()?;
        header.skip(2 + 4 + 4)?;
        let transparent = header.byte()?;
        let depth = match depth {
            32 => ColourDepth::Rgba,
            16 => ColourDepth::Grayscale,
            8 => ColourDepth::Indexed { transparent },
            other => bail!("Aseprite colour depth {other} isn't one Spry knows (32, 16 or 8 bits)"),
        };
        ensure!(frames > 0, "the file has no frames");
        Ok(Header { frames, width, height, depth, flags })
    }
}

impl Decoder {
    fn read_frame(&mut self, file: &mut Reader, index: usize) -> Result<()> {
        let size = file.dword()? as usize;
        ensure!(size >= 16, "a frame header is damaged");
        let mut frame = Reader::new(file.take(size - 4).context("the file ends in the middle of a frame")?);
        ensure!(frame.word()? == FRAME_MAGIC, "a frame doesn't start where it should");
        let old_chunk_count = frame.word()?;
        let duration = u32::from(frame.word()?).max(1);
        frame.skip(2)?;
        let new_chunk_count = frame.dword()?;
        let chunks = if new_chunk_count == 0 { u32::from(old_chunk_count) } else { new_chunk_count };

        if index > 0 {
            self.sprite.insert_frame(index, duration);
        }
        self.sprite.frames[index].duration_ms = duration;
        for _ in 0..chunks {
            let size = frame.dword()? as usize;
            ensure!(size >= 6, "a chunk header is damaged");
            let kind = frame.word()?;
            let mut chunk = Reader::new(frame.take(size - 6).context("the file ends in the middle of a chunk")?);
            self.read_chunk(kind, &mut chunk, index)?;
        }
        Ok(())
    }

    fn read_chunk(&mut self, kind: u16, chunk: &mut Reader, frame: usize) -> Result<()> {
        match kind {
            CHUNK_LAYER => self.read_layer(chunk),
            CHUNK_CEL => self.read_cel(chunk, frame),
            CHUNK_TAGS => self.read_tags(chunk),
            CHUNK_PALETTE => self.read_palette(chunk),
            CHUNK_SLICE => self.read_slice(chunk),
            // Colour profiles, user data, external files, the old palette
            // chunks and the rest carry nothing Spry draws with.
            _ => Ok(()),
        }
    }

    fn read_layer(&mut self, chunk: &mut Reader) -> Result<()> {
        let flags = chunk.word()?;
        let layer_type = chunk.word()?;
        chunk.skip(2 + 2 + 2)?;
        let blend_mode = chunk.word()?;
        let opacity = chunk.byte()?;
        chunk.skip(3)?;
        let name = chunk.string()?;
        match layer_type {
            0 => {}
            1 => {
                self.notices.push(format!("group \"{name}\" was flattened: its layers came in as ordinary layers"));
                self.layer_slots.push(LayerSlot::Group);
                return Ok(());
            }
            _ => {
                self.notices.push(format!("tilemap layer \"{name}\" came in empty, since Spry has no tilemaps"));
            }
        }
        if blend_mode != 0 {
            self.notices
                .push(format!("layer \"{name}\" uses a blend mode Spry doesn't have yet, so it blends normally"));
        }
        let index = self.sprite.layers.len();
        self.sprite.insert_layer(index, &name);
        let layer = &mut self.sprite.layers[index];
        layer.visible = flags & LAYER_VISIBLE != 0;
        layer.locked = flags & LAYER_EDITABLE == 0;
        layer.opacity = if self.layer_opacity_valid { opacity } else { 255 };
        self.layer_slots.push(LayerSlot::Spry(index));
        Ok(())
    }

    fn read_cel(&mut self, chunk: &mut Reader, frame: usize) -> Result<()> {
        let aseprite_layer = chunk.word()? as usize;
        let (x, y) = (i32::from(chunk.short()?), i32::from(chunk.short()?));
        let opacity = chunk.byte()?;
        let cel_type = chunk.word()?;
        chunk.skip(2 + 5)?;
        let layer = match self.layer_slots.get(aseprite_layer) {
            Some(LayerSlot::Spry(index)) => *index,
            Some(LayerSlot::Group) => return Ok(()),
            None => bail!("a cel refers to layer {aseprite_layer}, which doesn't exist"),
        };
        match cel_type {
            0 | 2 => {
                let (width, height) = (u32::from(chunk.word()?), u32::from(chunk.word()?));
                let data = if cel_type == 0 {
                    chunk.rest().to_vec()
                } else {
                    let expected = width as usize * height as usize * self.depth.bytes_per_pixel();
                    inflate(chunk.rest(), expected)?
                };
                let patch = self.pixels_from(&data, width, height, opacity)?;
                let mut image = Pixels::new(self.sprite.width, self.sprite.height);
                image.stamp(&patch, x, y, 255);
                let id = self.sprite.add_image(image);
                self.sprite.layers[layer].cels[frame] = Some(id);
            }
            1 => {
                let source = chunk.word()? as usize;
                ensure!(source < frame, "a linked cel points at frame {source}, which comes later");
                self.sprite.layers[layer].cels[frame] = self.sprite.layers[layer].cels[source];
            }
            3 => {} // Tilemap cels: already reported with their layer.
            other => bail!("unknown cel type {other}"),
        }
        Ok(())
    }

    /// Turns a cel's raw bytes into pixels in Spry's RGBA, baking in the
    /// cel's opacity since Spry cels don't carry one.
    fn pixels_from(&self, data: &[u8], width: u32, height: u32, opacity: u8) -> Result<Pixels> {
        check_canvas_size(width.max(1), height.max(1)).map_err(anyhow::Error::msg)?;
        let needed = width as usize * height as usize * self.depth.bytes_per_pixel();
        ensure!(data.len() >= needed, "a cel has fewer pixels than its size says");
        let mut pixels = Pixels::new(width, height);
        let chunks = data[..needed].chunks_exact(self.depth.bytes_per_pixel());
        for ((x, y), raw) in Rect::new(0, 0, width, height).points().zip(chunks) {
            let colour = match self.depth {
                ColourDepth::Rgba => Rgba::rgba(raw[0], raw[1], raw[2], raw[3]),
                ColourDepth::Grayscale => Rgba::rgba(raw[0], raw[0], raw[0], raw[1]),
                ColourDepth::Indexed { transparent } if raw[0] == transparent => Rgba::TRANSPARENT,
                ColourDepth::Indexed { .. } => self.palette.get(raw[0] as usize).copied().unwrap_or(Rgba::TRANSPARENT),
            };
            let alpha = (u16::from(colour.a()) * u16::from(opacity) / 255) as u8;
            pixels.set(x, y, if colour.is_transparent() { colour } else { colour.with_alpha(alpha) });
        }
        Ok(pixels)
    }

    fn read_tags(&mut self, chunk: &mut Reader) -> Result<()> {
        let count = chunk.word()?;
        chunk.skip(8)?;
        for _ in 0..count {
            let (from, to) = (chunk.word()? as usize, chunk.word()? as usize);
            let direction = match chunk.byte()? {
                1 => LoopDirection::Reverse,
                2 | 3 => LoopDirection::PingPong,
                _ => LoopDirection::Forward,
            };
            chunk.skip(2 + 6 + 3 + 1)?;
            let name = chunk.string()?;
            self.tags.push(Tag { name, from, to, direction });
        }
        Ok(())
    }

    fn read_palette(&mut self, chunk: &mut Reader) -> Result<()> {
        let size = chunk.dword()? as usize;
        let (first, last) = (chunk.dword()? as usize, chunk.dword()? as usize);
        chunk.skip(8)?;
        ensure!(first <= last && last < size.max(1) && size <= 256, "the palette chunk is damaged");
        self.palette.resize(size, Rgba::TRANSPARENT);
        for entry in first..=last {
            let flags = chunk.word()?;
            let colour = Rgba::rgba(chunk.byte()?, chunk.byte()?, chunk.byte()?, chunk.byte()?);
            if flags & 1 != 0 {
                chunk.string()?;
            }
            self.palette[entry] = colour;
        }
        Ok(())
    }

    fn read_slice(&mut self, chunk: &mut Reader) -> Result<()> {
        let keys = chunk.dword()?;
        let flags = chunk.dword()?;
        chunk.skip(4)?;
        let name = chunk.string()?;
        ensure!(keys > 0, "slice \"{name}\" has no keys");
        let _frame = chunk.dword()?;
        let (x, y) = (chunk.long()?, chunk.long()?);
        let (width, height) = (chunk.dword()?, chunk.dword()?);
        if keys > 1 || flags != 0 {
            self.notices.push(format!("slice \"{name}\" kept its first key's bounds only"));
        }
        self.sprite.slices.push(Slice { name, bounds: Rect::new(x, y, width, height) });
        Ok(())
    }

    fn finish(mut self) -> Result<Import> {
        self.sprite.tags = std::mem::take(&mut self.tags);
        if self.sprite.layers.is_empty() {
            self.sprite.insert_layer(0, "Layer 1");
        }
        let opaque_palette: Vec<Rgba> = self.palette.iter().copied().filter(|c| !c.is_transparent()).collect();
        if !opaque_palette.is_empty() {
            self.sprite.palette = Palette { name: "Imported".into(), colours: opaque_palette };
        }
        self.sprite.collect_unused_images();
        self.sprite.validate().map_err(|why| anyhow::anyhow!("the Aseprite file doesn't add up: {why}"))?;
        Ok(Import { document: Document::new(self.sprite), notices: self.notices })
    }
}

/// Unzips a compressed cel, refusing to produce more than the cel's size
/// says it holds.
fn inflate(compressed: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expected);
    flate2::read::ZlibDecoder::new(compressed)
        .take(expected as u64)
        .read_to_end(&mut out)
        .context("a compressed cel is damaged")?;
    Ok(out)
}

/// A cursor over little-endian bytes where running off the end is an
/// error, not a panic.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        ensure!(count <= self.bytes.len(), "the file ends sooner than it says");
        let (taken, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(taken)
    }

    fn skip(&mut self, count: usize) -> Result<()> {
        self.take(count).map(|_| ())
    }

    fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut array = [0; N];
        array.copy_from_slice(self.take(N)?);
        Ok(array)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn word(&mut self) -> Result<u16> {
        self.array().map(u16::from_le_bytes)
    }

    fn short(&mut self) -> Result<i16> {
        self.array().map(i16::from_le_bytes)
    }

    fn dword(&mut self) -> Result<u32> {
        self.array().map(u32::from_le_bytes)
    }

    fn long(&mut self) -> Result<i32> {
        self.array().map(i32::from_le_bytes)
    }

    fn string(&mut self) -> Result<String> {
        let length = self.word()? as usize;
        Ok(String::from_utf8_lossy(self.take(length)?).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_past_the_end_is_an_error_not_a_panic() {
        let mut reader = Reader::new(&[1, 2, 3]);
        assert_eq!(reader.word().unwrap(), 0x0201);
        assert!(reader.dword().is_err());
    }

    #[test]
    fn inflating_stops_at_the_promised_size() {
        use std::io::Write;
        let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        zlib.write_all(&[7; 4096]).unwrap();
        assert_eq!(inflate(&zlib.finish().unwrap(), 16).unwrap().len(), 16);
    }
}

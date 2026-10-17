//! The headless editor: Spry's whole document model and every editing
//! operation, with no UI, network or filesystem in it.
//!
//! Everything outside this module (the glass UI, the MCP server, the AI
//! agent, file I/O) is an adapter that reads a `Sprite` and sends
//! `Command`s to an `Editor`.

pub mod blend;
pub mod color;
pub mod command;
pub mod dither;
pub mod document;
pub mod effects;
pub mod groups;
pub mod limits;
pub mod palette;
pub mod pixelate;
pub mod pixels;
pub mod playback;
pub mod raster;
pub mod selection;
pub mod session;
pub mod sprite;

pub use blend::BlendMode;
pub use color::Rgba;
pub use command::{Brush, Command, Ink, PixelValue};
pub use dither::Pattern;
pub use document::{Document, Version};
pub use palette::Palette;
pub use pixels::{Pixels, Rect};
pub use session::{EditError, EditResult, Editor, StrokeKind};
pub use sprite::{Cel, Frame, Layer, LayerKind, LoopDirection, Sprite, Tag};

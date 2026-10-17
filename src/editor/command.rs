//! The verbs of Spry.
//!
//! Every change to a document is a `Command`: clicks and drags, the command
//! palette, MCP tool calls and the AI agent all go through here. The enum
//! is serde-tagged (`{"command": "draw_rect", ...}`), so commands can be
//! logged, scripted and replayed as plain JSON.

use super::blend::BlendMode;
use super::color::Rgba;
use super::dither::Pattern;
use super::palette::Palette;
use super::pixels::{Pixels, Rect};
use super::raster::Point;
use super::sprite::LoopDirection;
use serde::{Deserialize, Serialize};

/// What paint does to each pixel a tool touches.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Ink {
    /// Paints a colour. Translucent colours composite over what's there.
    Colour {
        colour: Rgba,
    },
    Erase,
    /// MS Paint-style pattern fill: `primary` where the pattern is on,
    /// `secondary` (or untouched, if none) where it's off.
    Pattern {
        pattern: Pattern,
        primary: Rgba,
        #[serde(default)]
        secondary: Option<Rgba>,
    },
    /// Moves each pixel `steps` along its palette ramp: positive lightens,
    /// negative darkens.
    Shade {
        steps: i32,
    },
    /// Clone stamp: copies the pixel found `(dx, dy)` away, as it was before
    /// the stroke began.
    Clone {
        dx: i32,
        dy: i32,
    },
}

impl Ink {
    pub fn colour(colour: Rgba) -> Ink {
        Ink::Colour { colour }
    }
}

/// How each touched point grows into a footprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Brush {
    pub size: u32,
    pub round: bool,
    /// Mirror across the vertical centre line (left↔right).
    pub mirror_x: bool,
    /// Mirror across the horizontal centre line (top↔bottom).
    pub mirror_y: bool,
}

impl Default for Brush {
    fn default() -> Self {
        Brush { size: 1, round: false, mirror_x: false, mirror_y: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelValue {
    pub x: i32,
    pub y: i32,
    pub colour: Rgba,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    // ----- drawing on the current cel ----------------------------------
    Paint {
        points: Vec<Point>,
        ink: Ink,
        #[serde(default)]
        brush: Brush,
        #[serde(default)]
        pixel_perfect: bool,
    },
    SetPixels {
        pixels: Vec<PixelValue>,
    },
    DrawLine {
        from: Point,
        to: Point,
        ink: Ink,
        #[serde(default)]
        brush: Brush,
    },
    DrawRect {
        from: Point,
        to: Point,
        ink: Ink,
        #[serde(default)]
        filled: bool,
        #[serde(default)]
        brush: Brush,
    },
    DrawEllipse {
        from: Point,
        to: Point,
        ink: Ink,
        #[serde(default)]
        filled: bool,
        #[serde(default)]
        brush: Brush,
    },
    Fill {
        at: Point,
        ink: Ink,
        #[serde(default = "yes")]
        contiguous: bool,
    },
    /// Ordered-dither gradient from `secondary` at `from` to `primary` at
    /// `to`, over the selection (or the region under `from`).
    GradientFill {
        from: Point,
        to: Point,
        primary: Rgba,
        secondary: Rgba,
    },
    Spray {
        at: Point,
        ink: Ink,
        radius: u32,
        density: u32,
        #[serde(default)]
        seed: u64,
    },
    ReplaceColour {
        from: Rgba,
        to: Rgba,
    },
    Outline {
        colour: Rgba,
        #[serde(default)]
        diagonals: bool,
    },
    DropShadow {
        colour: Rgba,
        dx: i32,
        dy: i32,
    },
    SnapToPalette {
        #[serde(default)]
        dither: bool,
    },
    /// Adds a new layer above the current one holding `image` in the
    /// current frame. This is how generated art arrives.
    ImportLayer {
        name: String,
        image: Pixels,
    },

    // ----- selection & clipboard ---------------------------------------
    SelectRect {
        rect: Rect,
        #[serde(default)]
        add: bool,
    },
    SelectWand {
        at: Point,
        #[serde(default = "yes")]
        contiguous: bool,
        #[serde(default)]
        add: bool,
    },
    SelectAll,
    SelectNone,
    InvertSelection,
    Copy,
    Cut,
    Paste {
        #[serde(default)]
        at: Option<Point>,
    },
    LiftSelection,
    MoveFloating {
        dx: i32,
        dy: i32,
    },
    Anchor,
    DeleteSelection,
    FlipHorizontal,
    FlipVertical,
    RotateClockwise,
    RotateCounterClockwise,

    // ----- layers -------------------------------------------------------
    AddLayer {
        #[serde(default)]
        name: Option<String>,
    },
    RemoveLayer,
    DuplicateLayer,
    MergeDown,
    RenameLayer {
        name: String,
    },
    MoveLayer {
        to: usize,
    },
    SetLayerVisible {
        visible: bool,
    },
    SetLayerLocked {
        locked: bool,
    },
    SetLayerOpacity {
        opacity: u8,
    },
    SetLayerBlend {
        blend: BlendMode,
    },
    /// Adds an empty group above the current layer, at the same level.
    AddGroup {
        #[serde(default)]
        name: Option<String>,
    },
    /// Moves the current layer into the group at layer index `group`, or
    /// out to the top level when `group` is omitted.
    SetLayerParent {
        #[serde(default)]
        group: Option<usize>,
    },
    /// How strongly the current cel shows, on top of its layer's opacity.
    SetCelOpacity {
        opacity: u8,
    },
    SelectLayer {
        index: usize,
    },

    // ----- frames, clones and tags --------------------------------------
    AddFrame,
    DuplicateFrame {
        #[serde(default)]
        as_clone: bool,
    },
    RemoveFrame,
    MoveFrame {
        to: usize,
    },
    SetFrameDuration {
        duration_ms: u32,
        #[serde(default)]
        all_frames: bool,
    },
    SelectFrame {
        index: usize,
    },
    LinkCel {
        to_frame: usize,
    },
    UnlinkCel,
    ClearCel,
    AddTag {
        name: String,
        from: usize,
        to: usize,
        direction: LoopDirection,
    },
    RemoveTag {
        name: String,
    },

    // ----- palette ------------------------------------------------------
    SetPalette {
        palette: Palette,
    },
    UseBuiltInPalette {
        name: String,
    },
    AddPaletteColour {
        colour: Rgba,
    },
    SetPaletteColour {
        index: usize,
        colour: Rgba,
    },
    RemovePaletteColour {
        index: usize,
    },
    ExtractPalette,

    // ----- canvas, notes, slices ------------------------------------------
    ResizeCanvas {
        width: u32,
        height: u32,
        #[serde(default)]
        offset_x: i32,
        #[serde(default)]
        offset_y: i32,
    },
    SetNotes {
        text: String,
    },
    SetFrameNote {
        frame: usize,
        text: String,
    },
    AddPin {
        x: i32,
        y: i32,
        text: String,
    },
    RemovePin {
        index: usize,
    },
    AddSlice {
        name: String,
        bounds: Rect,
    },
    RemoveSlice {
        name: String,
    },

    // ----- history & versions -----------------------------------------
    Undo,
    Redo,
    CommitVersion {
        #[serde(default)]
        message: String,
    },
    RestoreVersion {
        id: u32,
    },
}

fn yes() -> bool {
    true
}

impl Command {
    /// Commands that only move the editor's attention rather than change
    /// the art. These don't get an undo step.
    pub fn is_navigation(&self) -> bool {
        matches!(self, Command::SelectLayer { .. } | Command::SelectFrame { .. } | Command::Copy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_read_as_plain_json() {
        let json = r##"{"command":"draw_rect","from":[1,1],"to":[4,4],"filled":true,
                        "ink":{"type":"colour","colour":"#ff004d"}}"##;
        let command: Command = serde_json::from_str(json).unwrap();
        assert_eq!(
            command,
            Command::DrawRect {
                from: (1, 1),
                to: (4, 4),
                ink: Ink::colour(Rgba::rgb(255, 0, 77)),
                filled: true,
                brush: Brush::default(),
            }
        );
    }

    #[test]
    fn pattern_inks_name_their_pattern() {
        let json = r##"{"type":"pattern","pattern":"checker","primary":"#000000"}"##;
        let ink: Ink = serde_json::from_str(json).unwrap();
        assert_eq!(ink, Ink::Pattern { pattern: Pattern::Checker, primary: Rgba::BLACK, secondary: None });
    }

    #[test]
    fn fills_are_contiguous_unless_told_otherwise() {
        let json = r##"{"command":"fill","at":[0,0],"ink":{"type":"erase"}}"##;
        assert!(matches!(serde_json::from_str(json).unwrap(), Command::Fill { contiguous: true, .. }));
    }
}

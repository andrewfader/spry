//! `Editor`: one open document and everything about working on it, such as
//! the current layer and frame, the selection, the clipboard, undo, and the
//! stroke being drawn right now.
//!
//! `Editor::execute` is the one door that every change goes through. It
//! takes an undo snapshot, runs the command, and either keeps the result or
//! rolls it back if the command failed, so a failed command never leaves a
//! half-finished edit or an empty undo step.
//!
//! Live strokes (pencil drags) are the one exception to "one command, one
//! call". They're drawn incrementally for responsiveness, but they still
//! land in history as a single undo step.

use super::color::Rgba;
use super::command::{Brush, Command, Ink, PixelValue};
use super::dither::gradient_is_primary;
use super::document::Document;
use super::effects;
use super::limits;
use super::palette::Palette;
use super::pixelate::quantise;
use super::pixels::{Pixels, Rect};
use super::raster::{self, Point};
use super::rotsprite::{rotsprite, MAX_ROTSPRITE_SIDE};
use super::selection::{Floating, Selection};
use super::sprite::{Sprite, Tag};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    LayerLocked(String),
    NothingSelected,
    NothingToUndo,
    NothingToRedo,
    NotFound(String),
    Invalid(String),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::LayerLocked(name) => write!(f, "layer \"{name}\" is locked; unlock it to draw"),
            EditError::NothingSelected => f.write_str("nothing is selected"),
            EditError::NothingToUndo => f.write_str("nothing to undo"),
            EditError::NothingToRedo => f.write_str("nothing to redo"),
            EditError::NotFound(what) => write!(f, "{what} not found"),
            EditError::Invalid(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for EditError {}

pub type EditResult = Result<String, EditError>;

/// Everything undo restores. The selection and floating pixels are part of
/// it, so undoing a cut brings back both the pixels and the selection.
#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    sprite: Sprite,
    selection: Option<Selection>,
    floating: Option<Floating>,
    layer: usize,
    frame: usize,
}

const UNDO_LIMIT: usize = 256;

struct UndoGroup {
    has_snapshot: bool,
    /// Groups nest (the move tool opens one inside an AI turn), and only
    /// the outermost `end_undo_group` closes it.
    depth: u32,
}

/// Edits that arrive as a continuous stream (a keystroke per note change,
/// a step per pixel of a dragged duration) and so merge into the previous
/// undo step while the same stream continues.
#[derive(Clone, Copy, Debug, PartialEq)]
enum EditStream {
    Notes,
    FrameNote(usize),
    FrameDuration { frame: usize, all_frames: bool },
}

impl EditStream {
    fn of(command: &Command, current_frame: usize) -> Option<EditStream> {
        match *command {
            Command::SetNotes { .. } => Some(EditStream::Notes),
            Command::SetFrameNote { frame, .. } => Some(EditStream::FrameNote(frame)),
            Command::SetFrameDuration { all_frames, .. } => {
                Some(EditStream::FrameDuration { frame: current_frame, all_frames })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StrokeKind {
    Freehand { pixel_perfect: bool },
    Spray { radius: u32, density: u32 },
}

struct Stroke {
    before: Pixels,
    /// Redo survives a stroke that turns out to change nothing.
    redo_before: Vec<Snapshot>,
    /// Whether this stroke took its own undo snapshot (it doesn't inside an
    /// undo group that already has one).
    pushed_undo: bool,
    samples: Vec<Point>,
    sprayed: Vec<Point>,
    ink: Ink,
    brush: Brush,
    kind: StrokeKind,
}

pub struct Editor {
    pub document: Document,
    pub layer: usize,
    pub frame: usize,
    pub selection: Option<Selection>,
    pub floating: Option<Floating>,
    clipboard: Option<Pixels>,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    stroke: Option<Stroke>,
    /// While a group is open, only its first change takes an undo snapshot,
    /// so a whole AI turn (many tool calls) undoes in one step.
    undo_group: Option<UndoGroup>,
    /// The stream the newest undo step belongs to, if any.
    last_stream: Option<EditStream>,
    /// Bumped on every change, so views can cache textures cheaply.
    pub revision: u64,
    saved_revision: u64,
    clock: Box<dyn Fn() -> u64 + Send>,
}

fn system_clock() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Editor {
    pub fn new(width: u32, height: u32) -> Editor {
        Editor::with_document(Document::new(Sprite::new(width, height)))
    }

    pub fn with_document(document: Document) -> Editor {
        Editor {
            document,
            layer: 0,
            frame: 0,
            selection: None,
            floating: None,
            clipboard: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            stroke: None,
            undo_group: None,
            last_stream: None,
            revision: 0,
            saved_revision: 0,
            clock: Box::new(system_clock),
        }
    }

    /// Replaces the wall clock. Tests use this to make version timestamps
    /// predictable.
    pub fn set_clock(&mut self, clock: impl Fn() -> u64 + Send + 'static) {
        self.clock = Box::new(clock);
    }

    pub fn sprite(&self) -> &Sprite {
        &self.document.sprite
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    pub fn current_tag(&self) -> Option<&Tag> {
        self.sprite().tags.iter().find(|t| (t.from..=t.to).contains(&self.frame))
    }

    // ----- the single door ----------------------------------------------

    /// Runs a command. On success it returns a short human-readable summary
    /// (for the status bar and MCP replies), and on failure the document is
    /// exactly as it was.
    pub fn execute(&mut self, command: Command) -> EditResult {
        self.finish_stroke();
        limits::check(&command, self.sprite().width, self.sprite().height).map_err(EditError::Invalid)?;
        match command {
            Command::Undo => return self.undo(),
            Command::Redo => return self.redo(),
            Command::Copy => return self.copy(),
            // Moving away from floating pixels anchors them where they are,
            // which is an edit, so only plain navigation skips the snapshot.
            Command::SelectLayer { index } if self.floating.is_none() => return self.select_layer(index),
            Command::SelectFrame { index } if self.floating.is_none() => return self.select_frame(index),
            _ => {}
        }
        let stream = EditStream::of(&command, self.frame);
        let before = self.snapshot();
        match self.apply(command) {
            Ok(summary) => {
                if self.snapshot() != before {
                    let continues_the_stream = stream.is_some() && stream == self.last_stream && self.can_undo();
                    if continues_the_stream {
                        self.redo_stack.clear();
                    } else {
                        self.push_undo(before);
                    }
                    self.last_stream = stream;
                    self.revision += 1;
                }
                Ok(summary)
            }
            Err(error) => {
                self.restore(before);
                Err(error)
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            sprite: self.document.sprite.clone(),
            selection: self.selection.clone(),
            floating: self.floating.clone(),
            layer: self.layer,
            frame: self.frame,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.document.sprite = snapshot.sprite;
        self.selection = snapshot.selection;
        self.floating = snapshot.floating;
        self.layer = snapshot.layer.min(self.document.sprite.layers.len().saturating_sub(1));
        self.frame = snapshot.frame.min(self.document.sprite.frame_count().saturating_sub(1));
    }

    /// Opens an undo group: until `end_undo_group`, every change merges
    /// into one undo step.
    pub fn begin_undo_group(&mut self) {
        self.finish_stroke();
        match self.undo_group.as_mut() {
            Some(group) => group.depth += 1,
            None => self.undo_group = Some(UndoGroup { has_snapshot: false, depth: 1 }),
        }
    }

    pub fn end_undo_group(&mut self) {
        self.finish_stroke();
        if let Some(group) = self.undo_group.as_mut() {
            group.depth -= 1;
            if group.depth == 0 {
                self.undo_group = None;
            }
        }
    }

    /// Records an undo step, unless an open group already has one. Returns
    /// whether a step was recorded.
    fn push_undo(&mut self, snapshot: Snapshot) -> bool {
        self.redo_stack.clear();
        self.last_stream = None;
        if let Some(group) = self.undo_group.as_mut() {
            if group.has_snapshot {
                return false;
            }
            group.has_snapshot = true;
        }
        self.undo_stack.push(snapshot);
        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }
        true
    }

    fn undo(&mut self) -> EditResult {
        let previous = self.undo_stack.pop().ok_or(EditError::NothingToUndo)?;
        self.last_stream = None;
        if let Some(group) = self.undo_group.as_mut() {
            group.has_snapshot = false;
        }
        self.redo_stack.push(self.snapshot());
        self.restore(previous);
        self.revision += 1;
        Ok("undone".into())
    }

    fn redo(&mut self) -> EditResult {
        let next = self.redo_stack.pop().ok_or(EditError::NothingToRedo)?;
        self.last_stream = None;
        self.undo_stack.push(self.snapshot());
        self.restore(next);
        self.revision += 1;
        Ok("redone".into())
    }

    // ----- live strokes ---------------------------------------------------

    /// Starts a drag-painted stroke at `at`. Until `finish_stroke`, each
    /// `continue_stroke` redraws the whole stroke from the pre-stroke
    /// pixels, which is what lets pixel-perfect cleanup work on the full
    /// path as it grows.
    pub fn begin_stroke(&mut self, at: Point, ink: Ink, brush: Brush, kind: StrokeKind) -> Result<(), EditError> {
        self.finish_stroke();
        self.ensure_drawable()?;
        limits::check_brush(&brush).map_err(EditError::Invalid)?;
        if let StrokeKind::Spray { radius, density } = kind {
            limits::check_spray(radius, density).map_err(EditError::Invalid)?;
        }
        let before_snapshot = self.snapshot();
        let before = self.current_image_or_blank();
        let redo_before = self.redo_stack.clone();
        let pushed_undo = self.push_undo(before_snapshot);
        self.stroke = Some(Stroke {
            before,
            redo_before,
            pushed_undo,
            samples: Vec::new(),
            sprayed: Vec::new(),
            ink,
            brush,
            kind,
        });
        self.continue_stroke(at);
        Ok(())
    }

    pub fn continue_stroke(&mut self, at: Point) {
        let Some(stroke) = self.stroke.as_mut() else { return };
        if stroke.samples.last() == Some(&at) {
            return;
        }
        stroke.samples.push(at);
        let path = match stroke.kind {
            StrokeKind::Freehand { pixel_perfect } => {
                let connected = raster::connect(&stroke.samples);
                if pixel_perfect {
                    raster::pixel_perfect(&connected)
                } else {
                    connected
                }
            }
            StrokeKind::Spray { radius, density } => {
                let seed = stroke.samples.len() as u64;
                stroke.sprayed.extend(raster::spray(at, radius, density, seed));
                stroke.sprayed.clone()
            }
        };
        let (ink, brush, before) = (stroke.ink, stroke.brush, stroke.before.clone());
        let points = self.footprint(&path, brush);
        let mut image = before.clone();
        self.paint_points(&mut image, &before, &points, ink);
        *self.document.sprite.cel_image_mut(self.layer, self.frame) = image;
        self.revision += 1;
    }

    pub fn is_stroking(&self) -> bool {
        self.stroke.is_some()
    }

    /// Ends the live stroke. A stroke that changed nothing leaves no undo
    /// step behind.
    pub fn finish_stroke(&mut self) {
        let Some(stroke) = self.stroke.take() else { return };
        let unchanged = self.sprite().cel_image(self.layer, self.frame) == Some(&stroke.before)
            || (stroke.before.is_blank()
                && self.sprite().cel_image(self.layer, self.frame).is_none_or(Pixels::is_blank));
        if unchanged {
            self.redo_stack = stroke.redo_before;
            if stroke.pushed_undo {
                if let Some(previous) = self.undo_stack.pop() {
                    self.restore(previous);
                }
                if let Some(group) = self.undo_group.as_mut() {
                    group.has_snapshot = false;
                }
            }
        } else {
            self.revision += 1;
        }
    }

    // ----- what commands do ----------------------------------------------

    fn apply(&mut self, command: Command) -> EditResult {
        match command {
            Command::Paint { points, ink, brush, pixel_perfect } => {
                self.ensure_drawable()?;
                let connected = raster::connect(&points);
                let path = if pixel_perfect { raster::pixel_perfect(&connected) } else { connected };
                self.draw(&path, ink, brush, "painted")
            }
            Command::SetPixels { pixels } => self.set_pixels(&pixels),
            Command::DrawLine { ink, brush, .. }
            | Command::DrawRect { ink, brush, .. }
            | Command::DrawEllipse { ink, brush, .. } => {
                let (path, summary) = shape_path(&command).expect("these commands are shapes");
                self.draw(&path, ink, brush, summary)
            }
            Command::Fill { at, ink, contiguous } => self.fill(at, ink, contiguous),
            Command::GradientFill { from, to, primary, secondary } => self.gradient_fill(from, to, primary, secondary),
            Command::Spray { at, ink, radius, density, seed } => {
                let points = raster::spray(at, radius, density, seed);
                self.draw(&points, ink, Brush::default(), "sprayed")
            }
            Command::ReplaceColour { from, to } => {
                let count = self.document.sprite.swap_colour(from, to);
                Ok(format!("replaced {count} pixels of {from} with {to}"))
            }
            Command::Outline { colour, diagonals } => {
                self.transform_cel(|image| effects::outline(image, colour, diagonals))?;
                Ok("outlined the layer".into())
            }
            Command::DropShadow { colour, dx, dy } => {
                self.transform_cel(|image| effects::drop_shadow(image, colour, dx, dy))?;
                Ok("added a drop shadow".into())
            }
            Command::SnapToPalette { dither } => {
                let palette = self.sprite().palette.clone();
                self.transform_cel(|image| quantise(image, &palette, dither))?;
                Ok(format!("snapped the cel to {}", palette.name))
            }
            Command::ImportLayer { name, image } => self.import_layer(&name, image),

            Command::SelectRect { rect, add } => {
                let picked = Selection::from_rect(self.sprite().width, self.sprite().height, rect);
                self.select(picked, add)
            }
            Command::SelectLasso { points, add } => {
                let picked = Selection::from_polygon(self.sprite().width, self.sprite().height, &points);
                self.select(picked, add)
            }
            Command::SelectWand { at, contiguous, add } => {
                let image = self.current_image_or_blank();
                let points = if contiguous {
                    raster::contiguous_region(&image, at)
                } else {
                    raster::matching_colour(&image, at)
                };
                let picked = Selection::from_points(self.sprite().width, self.sprite().height, points);
                self.select(picked, add)
            }
            Command::SelectAll => {
                self.anchor_floating();
                self.selection = Some(Selection::all(self.sprite().width, self.sprite().height));
                Ok("selected everything".into())
            }
            Command::SelectNone => {
                self.anchor_floating();
                self.selection = None;
                Ok("deselected".into())
            }
            Command::InvertSelection => {
                self.anchor_floating();
                let (w, h) = (self.sprite().width, self.sprite().height);
                self.selection =
                    Some(self.selection.as_ref().map_or_else(|| Selection::all(w, h), Selection::inverted));
                Ok("inverted the selection".into())
            }
            Command::Cut => {
                self.copy()?;
                self.delete_selection()
            }
            Command::Paste { at } => self.paste(at),
            Command::LiftSelection => self.lift_selection(),
            Command::MoveFloating { dx, dy } => {
                if self.floating.is_none() {
                    self.lift_selection()?;
                }
                let floating = self.floating.as_mut().ok_or(EditError::NothingSelected)?;
                floating.x += dx;
                floating.y += dy;
                Ok(format!("moved the selection by ({dx}, {dy})"))
            }
            Command::Anchor => {
                if self.anchor_floating() {
                    Ok("anchored".into())
                } else {
                    Err(EditError::NothingSelected)
                }
            }
            Command::DeleteSelection => self.delete_selection(),
            Command::FlipHorizontal => self.reorient(Pixels::flipped_horizontally, "flipped horizontally"),
            Command::FlipVertical => self.reorient(Pixels::flipped_vertically, "flipped vertically"),
            Command::RotateClockwise => self.reorient(Pixels::rotated_clockwise, "rotated 90° clockwise"),
            Command::RotateCounterClockwise => {
                self.reorient(Pixels::rotated_counter_clockwise, "rotated 90° counter-clockwise")
            }
            Command::Rotate { degrees } => self.rotate_by(degrees),

            Command::AddLayer { name } => {
                let name = name.unwrap_or_else(|| self.sprite().next_layer_name());
                let id = self.insert_beside_current(|sprite, index, parent| {
                    let id = sprite.insert_layer(index, &name);
                    let position = sprite.layer_index(id).expect("just inserted");
                    sprite.layers[position].parent = parent;
                    id
                });
                Ok(format!("added layer \"{name}\"{}", self.inside_which_group(id)))
            }
            Command::AddGroup { name } => {
                let name = name.unwrap_or_else(|| "Group".into());
                let id = self.insert_beside_current(|sprite, index, parent| sprite.insert_group(index, &name, parent));
                Ok(format!("added group \"{name}\"{}", self.inside_which_group(id)))
            }
            Command::SetLayerParent { group } => {
                let id = self.sprite().layers[self.layer].id;
                let group =
                    group.map(|index| self.check_layer_index(index).map(|i| self.sprite().layers[i].id)).transpose()?;
                self.document.sprite.set_parent(self.layer, group).map_err(EditError::Invalid)?;
                self.layer = self.sprite().layer_index(id).unwrap_or(0);
                Ok(match group {
                    Some(_) => format!("moved the layer{}", self.inside_which_group(id)),
                    None => "moved the layer out to the top level".into(),
                })
            }
            Command::RemoveLayer => {
                let doomed = &self.sprite().layers[self.layer];
                let (name, id) = (doomed.name.clone(), doomed.id);
                if self.sprite().descendants(id).len() + 1 >= self.sprite().layers.len() {
                    return Err(EditError::Invalid("a sprite needs at least one layer".into()));
                }
                let removed = self.document.sprite.remove_layer(self.layer);
                self.layer = self.layer.saturating_sub(removed - 1).min(self.sprite().layers.len() - 1);
                Ok(if removed > 1 {
                    format!("removed group \"{name}\" and the {} layers in it", removed - 1)
                } else {
                    format!("removed layer \"{name}\"")
                })
            }
            Command::DuplicateLayer => {
                self.layer = if self.sprite().layers[self.layer].is_group() {
                    self.document.sprite.duplicate_group(self.layer)
                } else {
                    self.document.sprite.duplicate_layer(self.layer)
                };
                Ok(format!("duplicated into \"{}\"", self.sprite().layers[self.layer].name))
            }
            Command::MergeDown => {
                if !self.document.sprite.merge_down(self.layer) {
                    return Err(EditError::Invalid(
                        "merge down needs an image layer with another image layer below it, in the same group".into(),
                    ));
                }
                self.layer -= 1;
                Ok("merged down".into())
            }
            Command::RenameLayer { name } => {
                if name.trim().is_empty() {
                    return Err(EditError::Invalid("layer names can't be blank".into()));
                }
                self.document.sprite.layers[self.layer].name = name.trim().to_string();
                Ok(format!("renamed the layer to \"{}\"", name.trim()))
            }
            Command::MoveLayer { to } => {
                // Reorders among siblings. A group moves with its children,
                // and nothing changes group (that's SetLayerParent).
                let to = self.check_layer_index(to)?;
                let layer = self.document.sprite.layers.remove(self.layer);
                let id = layer.id;
                self.document.sprite.layers.insert(to, layer);
                self.document.sprite.tidy_layers();
                self.layer = self.sprite().layer_index(id).unwrap_or(to);
                Ok(format!("moved the layer to position {}", self.layer))
            }
            Command::SetLayerVisible { visible } => {
                self.document.sprite.layers[self.layer].visible = visible;
                Ok(if visible { "showed the layer" } else { "hid the layer" }.into())
            }
            Command::SetLayerLocked { locked } => {
                self.document.sprite.layers[self.layer].locked = locked;
                Ok(if locked { "locked the layer" } else { "unlocked the layer" }.into())
            }
            Command::SetLayerOpacity { opacity } => {
                self.document.sprite.layers[self.layer].opacity = opacity;
                Ok(format!("set layer opacity to {opacity}"))
            }
            Command::SetLayerBlend { blend } => {
                self.document.sprite.layers[self.layer].blend = blend;
                Ok(format!("the layer now blends with {}", blend.name()))
            }
            Command::SetCelOpacity { opacity } => {
                let (layer, frame) = (self.layer, self.frame);
                let cel = self.document.sprite.layers[layer].cels[frame]
                    .as_mut()
                    .ok_or_else(|| EditError::Invalid("this cel is empty, so there's nothing to fade".into()))?;
                cel.opacity = opacity;
                Ok(format!("set cel opacity to {opacity}"))
            }
            Command::SelectLayer { index } => self.select_layer(index),
            Command::SelectFrame { index } => self.select_frame(index),
            Command::Copy | Command::Undo | Command::Redo => {
                unreachable!("handled before an undo snapshot is taken")
            }

            Command::AddFrame => {
                let duration = self.sprite().frames[self.frame].duration_ms;
                self.document.sprite.insert_frame(self.frame + 1, duration);
                self.frame += 1;
                Ok(format!("added frame {}", self.frame + 1))
            }
            Command::DuplicateFrame { as_clone } => {
                self.frame = self.document.sprite.duplicate_frame(self.frame, as_clone);
                Ok(if as_clone { "cloned the frame (linked cels)" } else { "duplicated the frame" }.into())
            }
            Command::RemoveFrame => {
                if !self.document.sprite.remove_frame(self.frame) {
                    return Err(EditError::Invalid("a sprite needs at least one frame".into()));
                }
                self.frame = self.frame.min(self.sprite().frame_count() - 1);
                Ok("removed the frame".into())
            }
            Command::MoveFrame { to } => {
                let to = self.check_frame_index(to)?;
                self.document.sprite.move_frame(self.frame, to);
                self.frame = to;
                Ok(format!("moved the frame to position {}", to + 1))
            }
            Command::SetFrameDuration { duration_ms, all_frames } => {
                if duration_ms == 0 {
                    return Err(EditError::Invalid("frames need a duration of at least 1 ms".into()));
                }
                let frame = self.frame;
                for (index, f) in self.document.sprite.frames.iter_mut().enumerate() {
                    if all_frames || index == frame {
                        f.duration_ms = duration_ms;
                    }
                }
                Ok(format!("set duration to {duration_ms} ms"))
            }
            Command::LinkCel { to_frame } => {
                let to_frame = self.check_frame_index(to_frame)?;
                if self.sprite().cel_image(self.layer, self.frame).is_none() {
                    return Err(EditError::Invalid("an empty cel has nothing to link".into()));
                }
                self.document.sprite.link_cel(self.layer, self.frame, to_frame);
                Ok(format!("linked frame {} to frame {}", to_frame + 1, self.frame + 1))
            }
            Command::UnlinkCel => {
                if !self.document.sprite.unlink_cel(self.layer, self.frame) {
                    return Err(EditError::Invalid("this cel isn't linked".into()));
                }
                Ok("unlinked the cel".into())
            }
            Command::ClearCel => {
                self.ensure_drawable()?;
                self.document.sprite.clear_cel(self.layer, self.frame);
                Ok("cleared the cel".into())
            }
            Command::AddTag { name, from, to, direction } => {
                let (from, to) = (self.check_frame_index(from)?, self.check_frame_index(to)?);
                if name.trim().is_empty() {
                    return Err(EditError::Invalid("tags need a name".into()));
                }
                self.document.sprite.tags.retain(|t| t.name != name);
                self.document.sprite.tags.push(Tag {
                    name: name.clone(),
                    from: from.min(to),
                    to: from.max(to),
                    direction,
                });
                Ok(format!("tagged frames {}–{} as \"{name}\"", from.min(to) + 1, from.max(to) + 1))
            }
            Command::RemoveTag { name } => {
                let before = self.sprite().tags.len();
                self.document.sprite.tags.retain(|t| t.name != name);
                if self.sprite().tags.len() == before {
                    return Err(EditError::NotFound(format!("tag \"{name}\"")));
                }
                Ok(format!("removed tag \"{name}\""))
            }

            Command::SetPalette { palette } => {
                let summary = format!("using palette {} ({} colours)", palette.name, palette.colours.len());
                self.document.sprite.palette = palette;
                Ok(summary)
            }
            Command::UseBuiltInPalette { name } => {
                let palette = Palette::built_in_named(&name).ok_or_else(|| {
                    let known: Vec<String> = Palette::built_ins().into_iter().map(|p| p.name).collect();
                    EditError::NotFound(format!("palette \"{name}\" (built-ins are {})", known.join(", ")))
                })?;
                self.apply(Command::SetPalette { palette })
            }
            Command::AddPaletteColour { colour } => {
                if self.sprite().palette.colours.contains(&colour) {
                    return Ok(format!("{colour} is already in the palette"));
                }
                self.document.sprite.palette.colours.push(colour);
                Ok(format!("added {colour} to the palette"))
            }
            Command::SetPaletteColour { index, colour } => {
                let slot = self
                    .document
                    .sprite
                    .palette
                    .colours
                    .get_mut(index)
                    .ok_or_else(|| EditError::NotFound(format!("palette entry {index}")))?;
                *slot = colour;
                Ok(format!("palette entry {index} is now {colour}"))
            }
            Command::RemovePaletteColour { index } => {
                if index >= self.sprite().palette.colours.len() {
                    return Err(EditError::NotFound(format!("palette entry {index}")));
                }
                let removed = self.document.sprite.palette.colours.remove(index);
                Ok(format!("removed {removed} from the palette"))
            }
            Command::ExtractPalette => {
                let colours = self.sprite().used_colours();
                if colours.is_empty() {
                    return Err(EditError::Invalid("the sprite has no colours to extract yet".into()));
                }
                let count = colours.len();
                self.document.sprite.palette = Palette { name: "Extracted".into(), colours };
                Ok(format!("extracted {count} colours"))
            }

            Command::ResizeCanvas { width, height, offset_x, offset_y } => {
                self.anchor_floating();
                self.selection = None;
                self.document.sprite.resize_canvas(width, height, offset_x, offset_y);
                Ok(format!("canvas is now {width}×{height}"))
            }
            Command::SetNotes { text } => {
                self.document.sprite.notes = text;
                Ok("updated the notes".into())
            }
            Command::SetFrameNote { frame, text } => {
                let frame = self.check_frame_index(frame)?;
                self.document.sprite.frames[frame].note = text;
                Ok(format!("updated the note on frame {}", frame + 1))
            }
            Command::AddPin { x, y, text } => {
                self.document.sprite.pins.push(super::sprite::Pin { x, y, text });
                Ok(format!("pinned a note at ({x}, {y})"))
            }
            Command::RemovePin { index } => {
                if index >= self.sprite().pins.len() {
                    return Err(EditError::NotFound(format!("pin {index}")));
                }
                self.document.sprite.pins.remove(index);
                Ok("removed the pin".into())
            }
            Command::AddSlice { name, bounds } => {
                if bounds.is_empty() {
                    return Err(EditError::Invalid("slices need a width and height".into()));
                }
                self.document.sprite.slices.retain(|s| s.name != name);
                self.document.sprite.slices.push(super::sprite::Slice { name: name.clone(), bounds });
                Ok(format!("added slice \"{name}\""))
            }
            Command::RemoveSlice { name } => {
                let before = self.sprite().slices.len();
                self.document.sprite.slices.retain(|s| s.name != name);
                if before == self.sprite().slices.len() {
                    return Err(EditError::NotFound(format!("slice \"{name}\"")));
                }
                Ok(format!("removed slice \"{name}\""))
            }

            Command::CommitVersion { message } => {
                self.anchor_floating();
                let now = (self.clock)();
                match self.document.commit_version(&message, now) {
                    Some(id) => Ok(format!(
                        "saved version {id}: {}",
                        self.document.versions.last().map_or("", |v| v.message.as_str())
                    )),
                    None => Ok("nothing changed since the last version".into()),
                }
            }
            Command::RestoreVersion { id } => {
                let version = self.document.version(id).ok_or_else(|| EditError::NotFound(format!("version {id}")))?;
                let (sprite, message) = (version.sprite.clone(), version.message.clone());
                self.document.sprite = sprite;
                self.selection = None;
                self.floating = None;
                self.layer = self.layer.min(self.sprite().layers.len() - 1);
                self.frame = self.frame.min(self.sprite().frame_count() - 1);
                Ok(format!("restored version {id}: {message}"))
            }
        }
    }

    // ----- navigation ---------------------------------------------------

    fn select_layer(&mut self, index: usize) -> EditResult {
        self.anchor_floating();
        self.layer = self.check_layer_index(index)?;
        Ok(format!("layer \"{}\"", self.sprite().layers[self.layer].name))
    }

    fn select_frame(&mut self, index: usize) -> EditResult {
        self.anchor_floating();
        self.frame = self.check_frame_index(index)?;
        Ok(format!("frame {}", self.frame + 1))
    }

    fn check_layer_index(&self, index: usize) -> Result<usize, EditError> {
        (index < self.sprite().layers.len())
            .then_some(index)
            .ok_or_else(|| EditError::NotFound(format!("layer {index} (there are {})", self.sprite().layers.len())))
    }

    fn check_frame_index(&self, index: usize) -> Result<usize, EditError> {
        (index < self.sprite().frame_count()).then_some(index).ok_or_else(|| {
            EditError::NotFound(format!(
                "frame {index} (frames are numbered from 0 to {})",
                self.sprite().frame_count() - 1
            ))
        })
    }

    // ----- painting helpers -----------------------------------------------

    fn ensure_drawable(&self) -> Result<(), EditError> {
        let layer = &self.sprite().layers[self.layer];
        if layer.is_group() {
            return Err(EditError::Invalid(format!(
                "\"{}\" is a group, which holds layers rather than pixels; pick a layer inside it",
                layer.name
            )));
        }
        if layer.locked {
            return Err(EditError::LayerLocked(layer.name.clone()));
        }
        Ok(())
    }

    /// Inserts a new layer or group just above the current one, in the
    /// same group (a group's own level when a group is current), then
    /// selects it.
    fn insert_beside_current(
        &mut self,
        insert: impl FnOnce(&mut Sprite, usize, Option<super::sprite::LayerId>) -> super::sprite::LayerId,
    ) -> super::sprite::LayerId {
        let parent = self.sprite().layers[self.layer].parent;
        let id = insert(&mut self.document.sprite, self.layer + 1, parent);
        self.document.sprite.tidy_layers();
        self.layer = self.sprite().layer_index(id).unwrap_or(self.layer);
        id
    }

    fn inside_which_group(&self, id: super::sprite::LayerId) -> String {
        let sprite = self.sprite();
        let parent = sprite.layer_index(id).and_then(|i| sprite.layers[i].parent);
        parent
            .and_then(|p| sprite.layer_index(p))
            .map_or(String::new(), |g| format!(" in \"{}\"", sprite.layers[g].name))
    }

    fn current_image_or_blank(&self) -> Pixels {
        self.sprite()
            .cel_image(self.layer, self.frame)
            .cloned()
            .unwrap_or_else(|| Pixels::new(self.sprite().width, self.sprite().height))
    }

    /// Grows a path into the brush footprint, mirrored by the symmetry
    /// settings.
    pub fn footprint(&self, path: &[Point], brush: Brush) -> Vec<Point> {
        let (w, h) = (self.sprite().width as i32, self.sprite().height as i32);
        let mut points = Vec::with_capacity(path.len() * (brush.size * brush.size) as usize);
        for &point in path {
            let stamp = if brush.round {
                raster::round_brush(point, brush.size)
            } else {
                raster::square_brush(point, brush.size)
            };
            for (x, y) in stamp {
                points.push((x, y));
                if brush.mirror_x {
                    points.push((w - 1 - x, y));
                }
                if brush.mirror_y {
                    points.push((x, h - 1 - y));
                }
                if brush.mirror_x && brush.mirror_y {
                    points.push((w - 1 - x, h - 1 - y));
                }
            }
        }
        raster::deduplicated(points)
    }

    /// What a line, rectangle or ellipse command would paint on the
    /// current cel, without painting it: each pixel it would change, with
    /// its new colour. Shape tools preview with this, so the preview runs
    /// the same code as the result and can never disagree with it, whether
    /// the ink is a pattern, shade or clone, or there's a selection.
    pub fn preview(&self, command: &Command) -> Vec<(Point, Rgba)> {
        let (Some((path, _)), Some((ink, brush))) = (shape_path(command), shape_ink(command)) else {
            return Vec::new();
        };
        let points = raster::deduplicated(self.footprint(&path, brush));
        let before = self.current_image_or_blank();
        let mut after = before.clone();
        self.paint_points(&mut after, &before, &points, ink);
        points
            .into_iter()
            .filter(|&(x, y)| after.get(x, y) != before.get(x, y))
            .map(|p| (p, after.get(p.0, p.1)))
            .collect()
    }

    fn draw(&mut self, path: &[Point], ink: Ink, brush: Brush, summary: &str) -> EditResult {
        self.ensure_drawable()?;
        let points = self.footprint(path, brush);
        let before = self.current_image_or_blank();
        let mut image = before.clone();
        self.paint_points(&mut image, &before, &points, ink);
        *self.document.sprite.cel_image_mut(self.layer, self.frame) = image;
        Ok(summary.to_string())
    }

    /// Applies `ink` to each point, reading the "before" image for inks that
    /// depend on existing pixels (shade, clone). Points outside the
    /// selection, when there is one, are skipped.
    fn paint_points(&self, image: &mut Pixels, before: &Pixels, points: &[Point], ink: Ink) {
        let palette = &self.sprite().palette;
        for &(x, y) in points {
            if !image.contains(x, y) || self.selection.as_ref().is_some_and(|s| !s.contains(x, y)) {
                continue;
            }
            let existing = before.get(x, y);
            let painted = match ink {
                Ink::Colour { colour } => Some(colour.over(existing, 255)),
                Ink::Erase => Some(Rgba::TRANSPARENT),
                Ink::Pattern { pattern, primary, secondary } => {
                    if pattern.is_on(x, y) {
                        Some(primary)
                    } else {
                        secondary
                    }
                }
                Ink::Shade { steps } => (!existing.is_transparent()).then(|| palette.shade(existing, steps)),
                Ink::Clone { dx, dy } => Some(before.get(x + dx, y + dy)),
            };
            if let Some(colour) = painted {
                image.set(x, y, colour);
            }
        }
    }

    fn set_pixels(&mut self, pixels: &[PixelValue]) -> EditResult {
        self.ensure_drawable()?;
        let image = self.document.sprite.cel_image_mut(self.layer, self.frame);
        let mut written = 0;
        for pixel in pixels {
            if image.contains(pixel.x, pixel.y) {
                image.set(pixel.x, pixel.y, pixel.colour);
                written += 1;
            }
        }
        Ok(format!("set {written} pixels"))
    }

    fn fill(&mut self, at: Point, ink: Ink, contiguous: bool) -> EditResult {
        self.ensure_drawable()?;
        let image = self.current_image_or_blank();
        if !image.contains(at.0, at.1) {
            return Err(EditError::Invalid(format!("({}, {}) is outside the canvas", at.0, at.1)));
        }
        let region =
            if contiguous { raster::contiguous_region(&image, at) } else { raster::matching_colour(&image, at) };
        let count = region.len();
        let mut painted = image.clone();
        self.paint_points(&mut painted, &image, &region, ink);
        *self.document.sprite.cel_image_mut(self.layer, self.frame) = painted;
        Ok(format!("filled {count} pixels"))
    }

    fn gradient_fill(&mut self, from: Point, to: Point, primary: Rgba, secondary: Rgba) -> EditResult {
        self.ensure_drawable()?;
        let image = self.current_image_or_blank();
        let region: Vec<Point> = match &self.selection {
            Some(selection) => selection.points().collect(),
            None => raster::contiguous_region(&image, from),
        };
        let (gx, gy) = ((to.0 - from.0) as f32, (to.1 - from.1) as f32);
        let length_squared = (gx * gx + gy * gy).max(1.0);
        let mut painted = image;
        for &(x, y) in &region {
            let t = (((x - from.0) as f32 * gx) + ((y - from.1) as f32 * gy)) / length_squared;
            painted.set(x, y, if gradient_is_primary(x, y, t) { primary } else { secondary });
        }
        *self.document.sprite.cel_image_mut(self.layer, self.frame) = painted;
        Ok(format!("gradient-filled {} pixels", region.len()))
    }

    /// Replaces the current cel with a transformed copy, limited to the
    /// selection when there is one.
    fn transform_cel(&mut self, transform: impl Fn(&Pixels) -> Pixels) -> Result<(), EditError> {
        self.ensure_drawable()?;
        let before = self.current_image_or_blank();
        let after = transform(&before);
        let merged = match &self.selection {
            Some(selection) => {
                let mut merged = before.clone();
                for (x, y) in selection.points() {
                    merged.set(x, y, after.get(x, y));
                }
                merged
            }
            None => after,
        };
        *self.document.sprite.cel_image_mut(self.layer, self.frame) = merged;
        Ok(())
    }

    fn import_layer(&mut self, name: &str, image: Pixels) -> EditResult {
        let (w, h) = (self.sprite().width, self.sprite().height);
        let fitted = if (image.width(), image.height()) == (w, h) { image } else { image.resized_canvas(w, h, 0, 0) };
        let id = self.document.sprite.insert_layer(self.layer + 1, name);
        self.layer = self.sprite().layer_index(id).unwrap_or(self.layer);
        *self.document.sprite.cel_image_mut(self.layer, self.frame) = fitted;
        Ok(format!("added layer \"{name}\""))
    }

    // ----- selection helpers ----------------------------------------------

    fn select(&mut self, picked: Selection, add: bool) -> EditResult {
        self.anchor_floating();
        let combined = match (&self.selection, add) {
            (Some(existing), true) => existing.union(&picked),
            _ => picked,
        };
        let count = combined.points().count();
        self.selection = (!combined.is_empty()).then_some(combined);
        Ok(format!("selected {count} pixels"))
    }

    fn copy(&mut self) -> EditResult {
        let source = match &self.floating {
            Some(floating) => floating.pixels.clone(),
            None => {
                let image = self.current_image_or_blank();
                match &self.selection {
                    Some(selection) => selection.extract(&image).ok_or(EditError::NothingSelected)?.0,
                    None => image,
                }
            }
        };
        let summary = format!("copied {}×{}", source.width(), source.height());
        self.clipboard = Some(source);
        Ok(summary)
    }

    pub fn clipboard(&self) -> Option<&Pixels> {
        self.clipboard.as_ref()
    }

    fn delete_selection(&mut self) -> EditResult {
        self.ensure_drawable()?;
        if self.floating.take().is_some() {
            return Ok("deleted the floating pixels".into());
        }
        let selection = self.selection.clone().ok_or(EditError::NothingSelected)?;
        let image = self.document.sprite.cel_image_mut(self.layer, self.frame);
        for (x, y) in selection.points() {
            image.set(x, y, Rgba::TRANSPARENT);
        }
        Ok("cleared the selection".into())
    }

    fn paste(&mut self, at: Option<Point>) -> EditResult {
        self.ensure_drawable()?;
        let pixels = self.clipboard.clone().ok_or_else(|| EditError::Invalid("the clipboard is empty".into()))?;
        self.anchor_floating();
        let origin =
            at.or_else(|| self.selection.as_ref().and_then(Selection::bounds).map(|b| (b.x, b.y))).unwrap_or((0, 0));
        let summary = format!("pasted {}×{} at ({}, {})", pixels.width(), pixels.height(), origin.0, origin.1);
        self.floating = Some(Floating { pixels, x: origin.0, y: origin.1 });
        self.selection = None;
        Ok(summary)
    }

    /// Lifts the selected pixels off the cel into a floating piece, leaving
    /// a transparent hole, the way a move tool picks art up.
    fn lift_selection(&mut self) -> EditResult {
        self.ensure_drawable()?;
        if self.floating.is_some() {
            return Ok("already floating".into());
        }
        let selection = self.selection.clone().ok_or(EditError::NothingSelected)?;
        let image = self.current_image_or_blank();
        let (pixels, bounds) = selection.extract(&image).ok_or(EditError::NothingSelected)?;
        let cel = self.document.sprite.cel_image_mut(self.layer, self.frame);
        for (x, y) in selection.points() {
            cel.set(x, y, Rgba::TRANSPARENT);
        }
        self.floating = Some(Floating { pixels, x: bounds.x, y: bounds.y });
        self.selection = None;
        Ok("lifted the selection".into())
    }

    /// Stamps floating pixels down onto the current cel. The anchored area
    /// becomes the new selection, so you can keep working on it. Returns
    /// whether anything was floating.
    pub fn anchor_floating(&mut self) -> bool {
        let Some(floating) = self.floating.take() else { return false };
        let opacity = 255;
        self.document.sprite.cel_image_mut(self.layer, self.frame).stamp(
            &floating.pixels,
            floating.x,
            floating.y,
            opacity,
        );
        let landed = floating.bounds().intersect(&self.sprite().bounds());
        self.selection =
            (!landed.is_empty()).then(|| Selection::from_rect(self.sprite().width, self.sprite().height, landed));
        true
    }

    /// Flips or rotates the floating pixels, or the selection (lifting it
    /// first), or else the whole cel about its centre.
    fn rotate_by(&mut self, degrees: f32) -> EditResult {
        if !degrees.is_finite() {
            return Err(EditError::Invalid("an angle has to be a number of degrees".into()));
        }
        let piece = self
            .floating
            .as_ref()
            .map(|floating| (floating.pixels.width(), floating.pixels.height()))
            .or_else(|| self.selection.as_ref().and_then(Selection::bounds).map(|b| (b.width, b.height)))
            .unwrap_or((self.sprite().width, self.sprite().height));
        if piece.0.max(piece.1) > MAX_ROTSPRITE_SIDE {
            return Err(EditError::Invalid(format!(
                "Rotsprite turns pieces up to {MAX_ROTSPRITE_SIDE} pixels across; select a smaller area"
            )));
        }
        self.reorient(move |image| rotsprite(image, degrees), &format!("rotated {degrees}°"))
    }

    fn reorient(&mut self, turn: impl Fn(&Pixels) -> Pixels, summary: &str) -> EditResult {
        self.ensure_drawable()?;
        if self.floating.is_none() && self.selection.is_some() {
            self.lift_selection()?;
        }
        if let Some(floating) = self.floating.as_mut() {
            let centre =
                (floating.x * 2 + floating.pixels.width() as i32, floating.y * 2 + floating.pixels.height() as i32);
            floating.pixels = turn(&floating.pixels);
            floating.x = (centre.0 - floating.pixels.width() as i32) / 2;
            floating.y = (centre.1 - floating.pixels.height() as i32) / 2;
            return Ok(summary.to_string());
        }
        let image = self.current_image_or_blank();
        let turned = turn(&image);
        let offset_x = (image.width() as i32 - turned.width() as i32) / 2;
        let offset_y = (image.height() as i32 - turned.height() as i32) / 2;
        *self.document.sprite.cel_image_mut(self.layer, self.frame) =
            turned.resized_canvas(image.width(), image.height(), offset_x, offset_y);
        Ok(summary.to_string())
    }

    /// The current frame as shown on screen, with floating pixels
    /// composited onto their layer.
    pub fn composite_for_display(&self, frame: usize) -> Pixels {
        let sprite = self.sprite();
        match self.floating.as_ref().filter(|_| frame == self.frame) {
            Some(floating) => {
                let mut with_floating = sprite
                    .cel_image(self.layer, frame)
                    .cloned()
                    .unwrap_or_else(|| Pixels::new(sprite.width, sprite.height));
                with_floating.stamp(&floating.pixels, floating.x, floating.y, 255);
                sprite.composite_with(frame, Some((self.layer, &with_floating)))
            }
            None => sprite.composite(frame),
        }
    }

    /// The selection's bounding box, for the UI's handles.
    pub fn selection_bounds(&self) -> Option<Rect> {
        self.floating.as_ref().map(Floating::bounds).or_else(|| self.selection.as_ref().and_then(Selection::bounds))
    }
}

/// The path a shape command traces, and how to describe it.
fn shape_path(command: &Command) -> Option<(Vec<Point>, &'static str)> {
    match *command {
        Command::DrawLine { from, to, .. } => Some((raster::line(from, to), "drew a line")),
        Command::DrawRect { from, to, filled: true, .. } => {
            Some((raster::rectangle_filled(from, to), "drew a rectangle"))
        }
        Command::DrawRect { from, to, .. } => Some((raster::rectangle_outline(from, to), "drew a rectangle")),
        Command::DrawEllipse { from, to, filled: true, .. } => {
            Some((raster::ellipse_filled(from, to), "drew an ellipse"))
        }
        Command::DrawEllipse { from, to, .. } => Some((raster::ellipse_outline(from, to), "drew an ellipse")),
        _ => None,
    }
}

fn shape_ink(command: &Command) -> Option<(Ink, Brush)> {
    match *command {
        Command::DrawLine { ink, brush, .. }
        | Command::DrawRect { ink, brush, .. }
        | Command::DrawEllipse { ink, brush, .. } => Some((ink, brush)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shape_preview_shows_exactly_what_the_command_paints() {
        let mut editor = editor();
        editor.execute(Command::SelectRect { rect: Rect::new(0, 0, 4, 8), add: false }).unwrap();
        let ink = Ink::Pattern { pattern: crate::editor::Pattern::Checker, primary: RED, secondary: None };
        let rect = Command::DrawRect { from: (1, 1), to: (6, 6), ink, filled: true, brush: Brush::default() };
        let mut previewed = editor.preview(&rect);
        editor.execute(rect).unwrap();
        let image = editor.sprite().cel_image(0, 0).unwrap();
        let mut painted: Vec<(Point, Rgba)> = image
            .bounds()
            .points()
            .filter(|&(x, y)| !image.get(x, y).is_transparent())
            .map(|p| (p, image.get(p.0, p.1)))
            .collect();
        previewed.sort_by_key(|(p, _)| *p);
        painted.sort_by_key(|(p, _)| *p);
        assert!(!painted.is_empty());
        assert_eq!(previewed, painted, "clipped to the selection and patterned, just like the real thing");
    }

    const RED: Rgba = Rgba::rgb(255, 0, 0);

    fn editor() -> Editor {
        Editor::new(8, 8)
    }

    fn pixel(editor: &Editor, x: i32, y: i32) -> Rgba {
        editor.sprite().cel_image(editor.layer, editor.frame).map_or(Rgba::TRANSPARENT, |i| i.get(x, y))
    }

    #[test]
    fn a_failed_command_leaves_no_trace() {
        let mut editor = editor();
        editor.execute(Command::SetLayerLocked { locked: true }).unwrap();
        let undo_depth = editor.undo_stack.len();
        let error = editor.execute(Command::DrawLine {
            from: (0, 0),
            to: (3, 3),
            ink: Ink::colour(RED),
            brush: Brush::default(),
        });
        assert_eq!(error, Err(EditError::LayerLocked("Layer 1".into())));
        assert_eq!(editor.undo_stack.len(), undo_depth);
    }

    #[test]
    fn commands_that_change_nothing_add_no_undo_step() {
        let mut editor = editor();
        editor.execute(Command::SetLayerVisible { visible: true }).unwrap();
        assert!(!editor.can_undo());
    }

    #[test]
    fn a_live_stroke_is_one_undo_step() {
        let mut editor = editor();
        editor
            .begin_stroke((0, 0), Ink::colour(RED), Brush::default(), StrokeKind::Freehand { pixel_perfect: false })
            .unwrap();
        editor.continue_stroke((3, 0));
        editor.continue_stroke((3, 3));
        editor.finish_stroke();
        assert_eq!(pixel(&editor, 2, 0), RED);
        assert_eq!(pixel(&editor, 3, 2), RED);
        editor.execute(Command::Undo).unwrap();
        assert_eq!(pixel(&editor, 2, 0), Rgba::TRANSPARENT);
        assert!(!editor.can_undo());
    }

    #[test]
    fn an_eraser_stroke_on_an_empty_cel_leaves_no_undo_step() {
        let mut editor = editor();
        editor
            .begin_stroke((1, 1), Ink::Erase, Brush::default(), StrokeKind::Freehand { pixel_perfect: false })
            .unwrap();
        editor.finish_stroke();
        assert!(!editor.can_undo());
    }

    #[test]
    fn mirror_brushes_paint_all_four_quadrants() {
        let mut editor = editor();
        let brush = Brush { mirror_x: true, mirror_y: true, ..Brush::default() };
        editor
            .execute(Command::Paint { points: vec![(1, 2)], ink: Ink::colour(RED), brush, pixel_perfect: false })
            .unwrap();
        for (x, y) in [(1, 2), (6, 2), (1, 5), (6, 5)] {
            assert_eq!(pixel(&editor, x, y), RED, "at {x},{y}");
        }
    }

    #[test]
    fn painting_is_clipped_to_the_selection() {
        let mut editor = editor();
        editor.execute(Command::SelectRect { rect: Rect::new(0, 0, 2, 8), add: false }).unwrap();
        editor
            .execute(Command::DrawLine { from: (0, 0), to: (7, 0), ink: Ink::colour(RED), brush: Brush::default() })
            .unwrap();
        assert_eq!(pixel(&editor, 1, 0), RED);
        assert_eq!(pixel(&editor, 2, 0), Rgba::TRANSPARENT);
    }

    #[test]
    fn cutting_and_undoing_brings_back_pixels_and_selection() {
        let mut editor = editor();
        editor.execute(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true }).unwrap();
        editor.execute(Command::SelectRect { rect: Rect::new(0, 0, 2, 2), add: false }).unwrap();
        editor.execute(Command::Cut).unwrap();
        assert_eq!(pixel(&editor, 0, 0), Rgba::TRANSPARENT);
        editor.execute(Command::Undo).unwrap();
        assert_eq!(pixel(&editor, 0, 0), RED);
        assert!(editor.selection.as_ref().is_some_and(|s| s.contains(1, 1)));
    }

    #[test]
    fn moving_a_selection_lifts_and_anchors_it() {
        let mut editor = editor();
        editor.execute(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: RED }] }).unwrap();
        editor.execute(Command::SelectRect { rect: Rect::new(0, 0, 1, 1), add: false }).unwrap();
        editor.execute(Command::MoveFloating { dx: 3, dy: 2 }).unwrap();
        editor.execute(Command::Anchor).unwrap();
        assert_eq!(pixel(&editor, 0, 0), Rgba::TRANSPARENT);
        assert_eq!(pixel(&editor, 3, 2), RED);
    }

    #[test]
    fn switching_layers_anchors_floating_pixels_on_the_layer_they_came_from() {
        let mut editor = editor();
        editor.execute(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: RED }] }).unwrap();
        editor.execute(Command::AddLayer { name: None }).unwrap();
        editor.execute(Command::SelectLayer { index: 0 }).unwrap();
        editor.execute(Command::SelectRect { rect: Rect::new(0, 0, 1, 1), add: false }).unwrap();
        editor.execute(Command::MoveFloating { dx: 2, dy: 0 }).unwrap();
        editor.execute(Command::SelectLayer { index: 1 }).unwrap();
        assert_eq!(editor.sprite().cel_image(0, 0).unwrap().get(2, 0), RED);
        assert!(editor.sprite().cel_image(1, 0).is_none_or(|image| image.get(2, 0).is_transparent()));
    }

    #[test]
    fn anchoring_by_switching_frames_is_undoable() {
        let mut editor = editor();
        editor.execute(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: RED }] }).unwrap();
        editor.execute(Command::AddFrame).unwrap();
        editor.execute(Command::SelectFrame { index: 0 }).unwrap();
        editor.execute(Command::SelectRect { rect: Rect::new(0, 0, 1, 1), add: false }).unwrap();
        editor.execute(Command::MoveFloating { dx: 2, dy: 0 }).unwrap();
        editor.execute(Command::SelectFrame { index: 1 }).unwrap();
        editor.execute(Command::Undo).unwrap();
        assert!(editor.floating.is_some(), "undo puts the piece back in the air");
        assert_eq!(editor.frame, 0);
    }

    #[test]
    fn the_floating_piece_shows_in_the_display_composite_before_anchoring() {
        let mut editor = editor();
        editor.execute(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: RED }] }).unwrap();
        editor.execute(Command::SelectAll).unwrap();
        editor.execute(Command::Copy).unwrap();
        editor.execute(Command::Paste { at: Some((5, 5)) }).unwrap();
        assert_eq!(editor.composite_for_display(0).get(5, 5), RED);
        assert_eq!(pixel(&editor, 5, 5), Rgba::TRANSPARENT);
    }

    #[test]
    fn rotating_the_whole_cel_turns_about_the_centre() {
        let mut editor = editor();
        editor.execute(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: RED }] }).unwrap();
        editor.execute(Command::RotateClockwise).unwrap();
        assert_eq!(pixel(&editor, 7, 0), RED);
    }

    #[test]
    fn shade_ink_only_touches_existing_pixels() {
        let mut editor = editor();
        let palette = editor.sprite().palette.clone();
        let mid = palette.colours[8];
        editor.execute(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: mid }] }).unwrap();
        editor
            .execute(Command::DrawLine {
                from: (0, 0),
                to: (1, 0),
                ink: Ink::Shade { steps: 1 },
                brush: Brush::default(),
            })
            .unwrap();
        assert_eq!(pixel(&editor, 0, 0), palette.shade(mid, 1));
        assert_eq!(pixel(&editor, 1, 0), Rgba::TRANSPARENT);
    }

    #[test]
    fn clone_ink_copies_from_the_offset() {
        let mut editor = editor();
        editor.execute(Command::SetPixels { pixels: vec![PixelValue { x: 0, y: 0, colour: RED }] }).unwrap();
        editor
            .execute(Command::Paint {
                points: vec![(4, 4)],
                ink: Ink::Clone { dx: -4, dy: -4 },
                brush: Brush::default(),
                pixel_perfect: false,
            })
            .unwrap();
        assert_eq!(pixel(&editor, 4, 4), RED);
    }

    #[test]
    fn restoring_a_version_is_itself_undoable() {
        let mut editor = editor();
        editor.set_clock(|| 1_700_000_000);
        editor.execute(Command::CommitVersion { message: "blank".into() }).unwrap();
        editor.execute(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true }).unwrap();
        editor.execute(Command::RestoreVersion { id: 1 }).unwrap();
        assert_eq!(pixel(&editor, 0, 0), Rgba::TRANSPARENT);
        editor.execute(Command::Undo).unwrap();
        assert_eq!(pixel(&editor, 0, 0), RED);
        assert_eq!(editor.document.versions[0].created_at, 1_700_000_000);
    }

    #[test]
    fn unknown_palettes_list_the_known_ones() {
        let error = editor().execute(Command::UseBuiltInPalette { name: "neon".into() }).unwrap_err();
        assert!(error.to_string().contains("PICO-8"));
    }

    #[test]
    fn an_undo_group_undoes_in_one_step() {
        let mut editor = editor();
        editor.begin_undo_group();
        editor.execute(Command::AddLayer { name: None }).unwrap();
        editor.execute(Command::Fill { at: (0, 0), ink: Ink::colour(RED), contiguous: true }).unwrap();
        editor.execute(Command::AddFrame).unwrap();
        editor.end_undo_group();
        editor.execute(Command::Undo).unwrap();
        assert_eq!(editor.sprite().layers.len(), 1);
        assert_eq!(editor.sprite().frame_count(), 1);
        assert!(!editor.can_undo());
    }

    #[test]
    fn redo_is_cleared_by_a_new_change() {
        let mut editor = editor();
        editor.execute(Command::AddLayer { name: None }).unwrap();
        editor.execute(Command::Undo).unwrap();
        editor.execute(Command::AddFrame).unwrap();
        assert_eq!(editor.execute(Command::Redo), Err(EditError::NothingToRedo));
    }

    #[test]
    fn a_click_that_paints_nothing_keeps_redo_alive() {
        let mut editor = editor();
        editor.execute(Command::AddLayer { name: None }).unwrap();
        editor.execute(Command::Undo).unwrap();
        editor
            .begin_stroke((0, 0), Ink::Erase, Brush::default(), StrokeKind::Freehand { pixel_perfect: false })
            .unwrap();
        editor.finish_stroke();
        assert!(editor.can_redo());
    }

    #[test]
    fn nested_undo_groups_stay_one_step_until_the_outer_one_closes() {
        let mut editor = editor();
        editor.begin_undo_group();
        editor.execute(Command::AddLayer { name: None }).unwrap();
        editor.begin_undo_group();
        editor.execute(Command::AddFrame).unwrap();
        editor.end_undo_group();
        editor.execute(Command::AddFrame).unwrap();
        editor.end_undo_group();
        editor.execute(Command::Undo).unwrap();
        assert_eq!((editor.sprite().layers.len(), editor.sprite().frame_count()), (1, 1));
        assert!(!editor.can_undo());
    }

    #[test]
    fn typing_a_note_is_one_undo_step_not_one_per_keystroke() {
        let mut editor = editor();
        editor.execute(Command::AddLayer { name: None }).unwrap();
        for text in ["h", "hi", "hi!"] {
            editor.execute(Command::SetNotes { text: text.into() }).unwrap();
        }
        editor.execute(Command::Undo).unwrap();
        assert_eq!(editor.sprite().notes, "");
        assert_eq!(editor.sprite().layers.len(), 2, "the drawing history underneath survives");
    }

    #[test]
    fn dragging_a_frame_duration_is_one_undo_step() {
        let mut editor = editor();
        for duration_ms in [110, 120, 130] {
            editor.execute(Command::SetFrameDuration { duration_ms, all_frames: false }).unwrap();
        }
        editor.execute(Command::Undo).unwrap();
        assert_eq!(editor.sprite().frames[0].duration_ms, 100);
    }

    #[test]
    fn a_different_edit_in_between_starts_a_new_note_step() {
        let mut editor = editor();
        editor.execute(Command::SetNotes { text: "a".into() }).unwrap();
        editor.execute(Command::AddFrame).unwrap();
        editor.execute(Command::SetNotes { text: "ab".into() }).unwrap();
        editor.execute(Command::Undo).unwrap();
        assert_eq!(editor.sprite().notes, "a");
    }
}

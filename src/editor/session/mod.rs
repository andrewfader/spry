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

mod frames;
mod layers;
mod painting;
mod selecting;
mod whole_sprite;

use super::color::Rgba;
use super::command::{Brush, Command, Ink, PixelValue};
use super::document::Document;
use super::limits;
use super::pixels::{Pixels, Rect};
use super::raster::{self, Point};
use super::selection::{Floating, Selection};
use super::sprite::{Sprite, Tag};
use painting::{ellipse_path, rectangle_path};
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
            Command::Paint { points, ink, brush, pixel_perfect } => self.paint(&points, ink, brush, pixel_perfect),
            Command::SetPixels { pixels } => self.set_pixels(&pixels),
            Command::DrawLine { from, to, ink, brush } => self.draw(&raster::line(from, to), ink, brush, "drew a line"),
            Command::DrawRect { from, to, ink, filled, brush } => {
                self.draw(&rectangle_path(from, to, filled), ink, brush, "drew a rectangle")
            }
            Command::DrawEllipse { from, to, ink, filled, brush } => {
                self.draw(&ellipse_path(from, to, filled), ink, brush, "drew an ellipse")
            }
            Command::Fill { at, ink, contiguous } => self.fill(at, ink, contiguous),
            Command::GradientFill { from, to, primary, secondary } => self.gradient_fill(from, to, primary, secondary),
            Command::Spray { at, ink, radius, density, seed } => self.spray(at, ink, radius, density, seed),
            Command::ReplaceColour { from, to } => self.replace_colour(from, to),
            Command::Outline { colour, diagonals } => self.outline(colour, diagonals),
            Command::DropShadow { colour, dx, dy } => self.drop_shadow(colour, dx, dy),
            Command::SnapToPalette { dither } => self.snap_to_palette(dither),
            Command::ImportLayer { name, image } => self.import_layer(&name, image),

            Command::SelectRect { rect, add } => self.select_rect(rect, add),
            Command::SelectLasso { points, add } => self.select_lasso(&points, add),
            Command::SelectWand { at, contiguous, add } => self.select_wand(at, contiguous, add),
            Command::SelectAll => self.select_all(),
            Command::SelectNone => self.select_none(),
            Command::InvertSelection => self.invert_selection(),
            Command::Copy => self.copy(),
            Command::Cut => self.cut(),
            Command::Paste { at } => self.paste(at),
            Command::LiftSelection => self.lift_selection(),
            Command::MoveFloating { dx, dy } => self.move_floating(dx, dy),
            Command::Anchor => self.anchor(),
            Command::DeleteSelection => self.delete_selection(),
            Command::FlipHorizontal => self.reorient(Pixels::flipped_horizontally, "flipped horizontally"),
            Command::FlipVertical => self.reorient(Pixels::flipped_vertically, "flipped vertically"),
            Command::RotateClockwise => self.reorient(Pixels::rotated_clockwise, "rotated 90° clockwise"),
            Command::RotateCounterClockwise => {
                self.reorient(Pixels::rotated_counter_clockwise, "rotated 90° counter-clockwise")
            }
            Command::Rotate { degrees } => self.rotate_by(degrees),

            Command::AddLayer { name } => self.add_layer(name),
            Command::AddGroup { name } => self.add_group(name),
            Command::SetLayerParent { group } => self.set_layer_parent(group),
            Command::RemoveLayer => self.remove_current_layer(),
            Command::DuplicateLayer => self.duplicate_current_layer(),
            Command::MergeDown => self.merge_current_layer_down(),
            Command::RenameLayer { name } => self.rename_layer(&name),
            Command::MoveLayer { to } => self.move_layer(to),
            Command::SetLayerVisible { visible } => self.set_layer_visible(visible),
            Command::SetLayerLocked { locked } => self.set_layer_locked(locked),
            Command::SetLayerOpacity { opacity } => self.set_layer_opacity(opacity),
            Command::SetLayerBlend { blend } => self.set_layer_blend(blend),
            Command::SetCelOpacity { opacity } => self.set_cel_opacity(opacity),
            Command::SelectLayer { index } => self.select_layer(index),
            Command::SelectFrame { index } => self.select_frame(index),

            Command::AddFrame => self.add_frame(),
            Command::DuplicateFrame { as_clone } => self.duplicate_current_frame(as_clone),
            Command::RemoveFrame => self.remove_current_frame(),
            Command::MoveFrame { to } => self.move_current_frame(to),
            Command::SetFrameDuration { duration_ms, all_frames } => self.set_frame_duration(duration_ms, all_frames),
            Command::LinkCel { to_frame } => self.link_cel(to_frame),
            Command::UnlinkCel => self.unlink_cel(),
            Command::ClearCel => self.clear_cel(),
            Command::AddTag { name, from, to, direction } => self.add_tag(name, from, to, direction),
            Command::RemoveTag { name } => self.remove_tag(&name),

            Command::SetPalette { palette } => self.set_palette(palette),
            Command::UseBuiltInPalette { name } => self.use_built_in_palette(&name),
            Command::AddPaletteColour { colour } => self.add_palette_colour(colour),
            Command::SetPaletteColour { index, colour } => self.set_palette_colour(index, colour),
            Command::RemovePaletteColour { index } => self.remove_palette_colour(index),
            Command::ExtractPalette => self.extract_palette(),
            Command::ResizeCanvas { width, height, offset_x, offset_y } => {
                self.resize_canvas(width, height, offset_x, offset_y)
            }
            Command::SetNotes { text } => self.set_notes(text),
            Command::SetFrameNote { frame, text } => self.set_frame_note(frame, text),
            Command::AddPin { x, y, text } => self.add_pin(x, y, text),
            Command::RemovePin { index } => self.remove_pin(index),
            Command::AddSlice { name, bounds } => self.add_slice(name, bounds),
            Command::RemoveSlice { name } => self.remove_slice(&name),
            Command::CommitVersion { message } => self.commit_version(&message),
            Command::RestoreVersion { id } => self.restore_version(id),

            // `execute` handles these before taking a snapshot, but they
            // are real operations, so they work here too.
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
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

    fn current_image_or_blank(&self) -> Pixels {
        self.sprite()
            .cel_image(self.layer, self.frame)
            .cloned()
            .unwrap_or_else(|| Pixels::new(self.sprite().width, self.sprite().height))
    }

    /// Grows a path into the brush footprint, mirrored by the symmetry
    /// settings.
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

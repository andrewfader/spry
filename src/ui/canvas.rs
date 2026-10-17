//! The canvas: where pixels are shown and drawn.
//!
//! The canvas turns pointer gestures into editor calls. Stroke tools paint
//! live through `begin_stroke`/`continue_stroke`. Shapes and selections
//! preview while you drag, then commit one `Command` on release. Everything
//! drawn on top (grid, onion skin, marching ants, pins) is display only and
//! never touches the document.

use super::textures::TextureCache;
use super::theme::{self, Appearance};
use super::tools::{Tool, ToolSettings};
use crate::editor::raster::Point;
use crate::editor::{Command, Rect, StrokeKind};
use crate::studio::Studio;
use egui::{Color32, Pos2, Sense, Stroke, Vec2};

pub struct CanvasView {
    pub zoom: f32,
    /// How far the canvas centre sits from the viewport centre, in screen
    /// points.
    pub offset: Vec2,
    pub show_grid: bool,
    pub onion_skin: bool,
    pub onion_before: usize,
    pub onion_after: usize,
    /// How strongly the nearest ghost shows. Ghosts further away fade out.
    pub onion_opacity: u8,
    pub tiled: bool,
    fitted: bool,
    gesture: Option<Gesture>,
    /// The outline traced so far by the lasso.
    lasso: Vec<Point>,
    /// The pixel under the pointer, for the status bar.
    pub hovered: Option<Point>,
}

impl Default for CanvasView {
    fn default() -> Self {
        CanvasView {
            zoom: 8.0,
            offset: Vec2::ZERO,
            show_grid: true,
            onion_skin: false,
            onion_before: 1,
            onion_after: 1,
            onion_opacity: 100,
            tiled: false,
            fitted: false,
            gesture: None,
            lasso: Vec::new(),
            hovered: None,
        }
    }
}

struct Gesture {
    tool: Tool,
    start: Point,
    current: Point,
    secondary: bool,
}

/// Something the canvas wants the app to do that needs the app's help (a
/// dialog, a status message).
pub enum CanvasRequest {
    Status(String),
    PinNoteAt(Point),
}

pub struct CanvasInput<'a> {
    pub studio: &'a mut Studio,
    pub view: &'a mut CanvasView,
    pub settings: &'a mut ToolSettings,
    pub textures: &'a mut TextureCache,
    pub appearance: Appearance,
    pub texture_prefix: String,
}

const ZOOM_STEPS: [f32; 14] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0, 20.0, 24.0, 32.0, 48.0];

impl CanvasView {
    pub fn zoom_in(&mut self) {
        self.zoom = ZOOM_STEPS.iter().copied().find(|&z| z > self.zoom).unwrap_or(64.0);
    }

    pub fn zoom_out(&mut self) {
        self.zoom = ZOOM_STEPS.iter().rev().copied().find(|&z| z < self.zoom).unwrap_or(1.0);
    }

    /// Fits the sprite to the viewport, leaving room for the floating
    /// panels.
    pub fn fit(&mut self, viewport: egui::Rect, width: u32, height: u32) {
        let usable = viewport.shrink2(Vec2::new(300.0, 160.0));
        let fit = (usable.width() / width as f32).min(usable.height() / height as f32).floor();
        self.zoom = fit.clamp(1.0, 48.0);
        self.offset = Vec2::new(-20.0, -40.0);
        self.fitted = true;
    }

    pub fn refit(&mut self) {
        self.fitted = false;
    }
}

pub fn show(ui: &mut egui::Ui, input: CanvasInput<'_>) -> Vec<CanvasRequest> {
    let CanvasInput { studio, view, settings, textures, appearance, texture_prefix } = input;
    let mut requests = Vec::new();
    let viewport = ui.max_rect();
    let response = ui.allocate_rect(viewport, Sense::click_and_drag());
    let (width, height) = (studio.editor.sprite().width, studio.editor.sprite().height);
    if !view.fitted {
        view.fit(viewport, width, height);
    }

    handle_navigation(ui, &response, view, viewport, (width, height));

    let zoom = view.zoom;
    let canvas_size = Vec2::new(width as f32, height as f32) * zoom;
    let canvas_rect = egui::Rect::from_center_size(viewport.center() + view.offset, canvas_size);
    let to_pixel = move |pos: Pos2| -> Point {
        let local = (pos - canvas_rect.min) / zoom;
        (local.x.floor() as i32, local.y.floor() as i32)
    };
    let to_screen = move |(x, y): Point| canvas_rect.min + Vec2::new(x as f32, y as f32) * zoom;

    view.hovered =
        response.hover_pos().map(to_pixel).filter(|&(x, y)| x >= 0 && y >= 0 && x < width as i32 && y < height as i32);

    paint_sprite(ui, studio, view, textures, appearance, &texture_prefix, canvas_rect);
    let panning = ui.input(|i| i.key_down(egui::Key::Space) || i.pointer.middle_down());
    if !panning {
        handle_tool(ui, &response, studio, view, settings, to_pixel, &mut requests);
    }
    paint_overlays(ui, studio, view, settings, canvas_rect, to_screen, appearance);
    requests
}

fn handle_navigation(
    ui: &egui::Ui,
    response: &egui::Response,
    view: &mut CanvasView,
    viewport: egui::Rect,
    size: (u32, u32),
) {
    let (scroll, pinch, pointer) = ui.input(|i| (i.raw_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
    if response.hovered() && (scroll.y != 0.0 || pinch != 1.0) {
        let before = view.zoom;
        if pinch > 1.0 || scroll.y > 0.0 {
            view.zoom_in();
        } else {
            view.zoom_out();
        }
        // Keep the pixel under the pointer where it is.
        if let Some(pointer) = pointer {
            let centre = viewport.center() + view.offset;
            let from_centre = pointer - centre;
            view.offset -= from_centre * (view.zoom / before - 1.0);
        }
    }
    let panning = ui.input(|i| i.key_down(egui::Key::Space) || i.pointer.middle_down());
    if panning && response.dragged() {
        view.offset += response.drag_delta();
    }
    // Never let the sprite drift entirely off screen.
    let limit = Vec2::new(
        viewport.width() / 2.0 + size.0 as f32 * view.zoom / 2.0,
        viewport.height() / 2.0 + size.1 as f32 * view.zoom / 2.0,
    ) - Vec2::splat(40.0);
    view.offset = view.offset.clamp(-limit, limit);
}

fn paint_sprite(
    ui: &egui::Ui,
    studio: &Studio,
    view: &CanvasView,
    textures: &mut TextureCache,
    appearance: Appearance,
    prefix: &str,
    canvas_rect: egui::Rect,
) {
    let painter = ui.painter();
    let colours = theme::palette(appearance);
    let editor = &studio.editor;
    let revision = editor.revision;
    let full_uv = egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

    // Soft shadow under the "sheet", like artwork resting on glass.
    painter.rect_filled(canvas_rect.translate(Vec2::new(0.0, 8.0)).expand(4.0), 6.0, Color32::from_black_alpha(50));
    let checker = textures.checkerboard(ui.ctx(), colours.checker_light, colours.checker_dark).id();
    let cells = canvas_rect.size() / 16.0;
    painter.image(checker, canvas_rect, egui::Rect::from_min_size(Pos2::ZERO, cells), Color32::WHITE);

    let frame = editor.frame;
    let current = textures
        .get(ui.ctx(), &format!("{prefix}composite"), revision ^ (frame as u64) << 48, || {
            editor.composite_for_display(frame)
        })
        .id();

    if view.tiled {
        for dy in -1..=1 {
            for dx in -1..=1 {
                if (dx, dy) != (0, 0) {
                    let tile = canvas_rect.translate(Vec2::new(dx as f32, dy as f32) * canvas_rect.size());
                    painter.image(current, tile, full_uv, Color32::from_white_alpha(150));
                }
            }
        }
    }

    if view.onion_skin {
        let frames = editor.sprite().frame_count();
        let fade = |step: usize, reach: usize| {
            (view.onion_opacity as usize * (reach + 1 - step) / reach.max(1)).min(255) as u8
        };
        let tint_before = |step| Color32::from_rgba_unmultiplied(255, 90, 90, fade(step, view.onion_before));
        let tint_after = |step| Color32::from_rgba_unmultiplied(90, 150, 255, fade(step, view.onion_after));
        let mut ghost = |other: usize, tint: Color32| {
            let id = textures
                .get(ui.ctx(), &format!("{prefix}onion{other}"), revision, || editor.sprite().composite(other))
                .id();
            painter.image(id, canvas_rect, full_uv, tint);
        };
        // Furthest first, so the nearest ghost sits on top.
        for step in (1..=view.onion_before).rev() {
            if let Some(other) = frame.checked_sub(step) {
                ghost(other, tint_before(step));
            }
        }
        for step in (1..=view.onion_after).rev() {
            if frame + step < frames {
                ghost(frame + step, tint_after(step));
            }
        }
    }
    painter.image(current, canvas_rect, full_uv, Color32::WHITE);
}

#[allow(clippy::too_many_arguments)]
fn handle_tool(
    ui: &egui::Ui,
    response: &egui::Response,
    studio: &mut Studio,
    view: &mut CanvasView,
    settings: &mut ToolSettings,
    to_pixel: impl Fn(Pos2) -> Point,
    requests: &mut Vec<CanvasRequest>,
) {
    let (pressed_primary, pressed_secondary, alt, shift) = ui.input(|i| {
        (
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.button_pressed(egui::PointerButton::Secondary),
            i.modifiers.alt,
            i.modifiers.shift,
        )
    });
    let pointer = ui.input(|i| i.pointer.interact_pos()).map(&to_pixel);
    let down = ui.input(|i| i.pointer.primary_down() || i.pointer.secondary_down());
    let editor = &mut studio.editor;

    if response.hovered() && (pressed_primary || pressed_secondary) {
        let Some(at) = pointer else { return };
        let secondary = pressed_secondary;
        let tool = settings.tool;
        if tool == Tool::CloneStamp && alt {
            settings.clone_anchor = Some(at);
            settings.clone_offset = None;
            requests.push(CanvasRequest::Status(format!("clone source set at ({}, {})", at.0, at.1)));
            return;
        }
        if tool == Tool::Eyedropper || alt {
            let picked = editor.composite_for_display(editor.frame).get(at.0, at.1);
            if !picked.is_transparent() {
                if secondary {
                    settings.secondary = picked
                } else {
                    settings.primary = picked
                }
            }
            return;
        }
        match tool {
            Tool::Fill => report(
                requests,
                editor.execute(Command::Fill {
                    at,
                    ink: ink_for(settings, secondary),
                    contiguous: settings.contiguous,
                }),
            ),
            Tool::Wand => report(
                requests,
                editor.execute(Command::SelectWand { at, contiguous: settings.contiguous, add: shift }),
            ),
            Tool::Pin => requests.push(CanvasRequest::PinNoteAt(at)),
            Tool::Move => {
                editor.begin_undo_group();
                if editor.floating.is_none() {
                    let lifted = if editor.selection.is_some() { Command::LiftSelection } else { Command::SelectAll };
                    report(requests, editor.execute(lifted));
                    if editor.floating.is_none() {
                        report(requests, editor.execute(Command::LiftSelection));
                    }
                }
                view.gesture = Some(Gesture { tool, start: at, current: at, secondary });
            }
            stroke_tool if stroke_tool.is_stroke() => {
                if stroke_tool == Tool::CloneStamp && settings.clone_offset.is_none() {
                    match settings.clone_anchor {
                        Some(anchor) => settings.clone_offset = Some((anchor.0 - at.0, anchor.1 - at.1)),
                        None => {
                            requests.push(CanvasRequest::Status("Alt-click to choose where to clone from".into()));
                            return;
                        }
                    }
                }
                let kind = match stroke_tool {
                    Tool::Spray => StrokeKind::Spray { radius: settings.spray_radius, density: settings.spray_density },
                    _ => StrokeKind::Freehand { pixel_perfect: settings.pixel_perfect && settings.brush.size == 1 },
                };
                match editor.begin_stroke(at, ink_for(settings, secondary), settings.brush, kind) {
                    Ok(()) => view.gesture = Some(Gesture { tool, start: at, current: at, secondary }),
                    Err(error) => requests.push(CanvasRequest::Status(error.to_string())),
                }
            }
            Tool::Lasso => {
                view.lasso = vec![at];
                view.gesture = Some(Gesture { tool, start: at, current: at, secondary });
            }
            _ => view.gesture = Some(Gesture { tool, start: at, current: at, secondary }),
        }
        return;
    }

    let Some(gesture) = view.gesture.as_mut() else { return };
    if let Some(at) = pointer {
        if down && at != gesture.current {
            if gesture.tool.is_stroke() {
                editor.continue_stroke(at);
            } else if gesture.tool == Tool::Move {
                let (dx, dy) = (at.0 - gesture.current.0, at.1 - gesture.current.1);
                report(requests, editor.execute(Command::MoveFloating { dx, dy }));
            } else if gesture.tool == Tool::Lasso {
                view.lasso.push(at);
            }
            gesture.current = at;
        }
    }
    if !down {
        let gesture = view.gesture.take().expect("checked above");
        let outline = std::mem::take(&mut view.lasso);
        match gesture.tool {
            // A click without a drag clears the selection, as with the
            // rectangle.
            Tool::Lasso if outline.len() < 3 && !shift => report(requests, editor.execute(Command::SelectNone)),
            Tool::Lasso => report(requests, editor.execute(Command::SelectLasso { points: outline, add: shift })),
            _ => finish_gesture(editor, settings, gesture, shift, requests),
        }
    }
}

fn finish_gesture(
    editor: &mut crate::editor::Editor,
    settings: &ToolSettings,
    gesture: Gesture,
    shift: bool,
    requests: &mut Vec<CanvasRequest>,
) {
    let Gesture { tool, start, current: end, secondary } = gesture;
    if let Some(shape) = shape_command(&gesture, settings, shift) {
        report(requests, editor.execute(shape));
        return;
    }
    let outcome = match tool {
        stroke if stroke.is_stroke() => {
            editor.finish_stroke();
            return;
        }
        Tool::Move => {
            editor.end_undo_group();
            return;
        }
        Tool::Gradient => {
            let (primary, secondary_colour) =
                if secondary { (settings.secondary, settings.primary) } else { (settings.primary, settings.secondary) };
            editor.execute(Command::GradientFill { from: start, to: end, primary, secondary: secondary_colour })
        }
        Tool::Select if start == end && !shift => editor.execute(Command::SelectNone),
        Tool::Select => editor.execute(Command::SelectRect { rect: Rect::spanning(start, end), add: shift }),
        _ => return,
    };
    report(requests, outcome);
}

fn report(requests: &mut Vec<CanvasRequest>, outcome: crate::editor::EditResult) {
    if let Err(error) = outcome {
        requests.push(CanvasRequest::Status(error.to_string()));
    }
}

/// The ink for a press, where a right-click paints with the secondary
/// colour.
fn ink_for(settings: &ToolSettings, secondary: bool) -> crate::editor::Ink {
    if !secondary {
        return settings.ink();
    }
    let swapped = ToolSettings { primary: settings.secondary, secondary: settings.primary, ..settings.clone() };
    swapped.ink()
}

/// Shift-drag makes squares and circles.
/// The command a line, rectangle or ellipse drag stands for. Both the live
/// preview and the release use it, so what you see is what you get. Shift
/// makes rectangles square and ellipses round.
fn shape_command(gesture: &Gesture, settings: &ToolSettings, shift: bool) -> Option<Command> {
    let (from, ink, brush, filled) =
        (gesture.start, ink_for(settings, gesture.secondary), settings.brush, settings.filled);
    let squared = if shift { square_up(gesture.start, gesture.current) } else { gesture.current };
    match gesture.tool {
        Tool::Line => Some(Command::DrawLine { from, to: gesture.current, ink, brush }),
        Tool::Rectangle => Some(Command::DrawRect { from, to: squared, ink, filled, brush }),
        Tool::Ellipse => Some(Command::DrawEllipse { from, to: squared, ink, filled, brush }),
        _ => None,
    }
}

fn square_up(start: Point, end: Point) -> Point {
    let side = (end.0 - start.0).abs().max((end.1 - start.1).abs());
    (start.0 + side * (end.0 - start.0).signum(), start.1 + side * (end.1 - start.1).signum())
}

fn paint_overlays(
    ui: &egui::Ui,
    studio: &Studio,
    view: &CanvasView,
    settings: &ToolSettings,
    canvas_rect: egui::Rect,
    to_screen: impl Fn(Point) -> Pos2,
    appearance: Appearance,
) {
    let painter = ui.painter_at(ui.max_rect());
    let colours = theme::palette(appearance);
    let editor = &studio.editor;
    let sprite = editor.sprite();
    let zoom = view.zoom;
    let pixel_rect = |p: Point| egui::Rect::from_min_size(to_screen(p), Vec2::splat(zoom));

    if view.show_grid && zoom >= 6.0 {
        let grid = Stroke::new(1.0_f32, colours.edge.gamma_multiply(0.35));
        for x in 1..sprite.width {
            let sx = canvas_rect.left() + x as f32 * zoom;
            painter.line_segment([Pos2::new(sx, canvas_rect.top()), Pos2::new(sx, canvas_rect.bottom())], grid);
        }
        for y in 1..sprite.height {
            let sy = canvas_rect.top() + y as f32 * zoom;
            painter.line_segment([Pos2::new(canvas_rect.left(), sy), Pos2::new(canvas_rect.right(), sy)], grid);
        }
    }
    painter.rect_stroke(canvas_rect, 0.0, Stroke::new(1.0_f32, colours.edge), egui::StrokeKind::Outside);

    for slice in &sprite.slices {
        let b = slice.bounds;
        let rect = egui::Rect::from_min_max(to_screen((b.x, b.y)), to_screen((b.right(), b.bottom())));
        painter.rect_stroke(rect, 0.0, Stroke::new(1.5_f32, Color32::from_rgb(90, 140, 255)), egui::StrokeKind::Middle);
        painter.text(
            rect.left_top() + Vec2::new(2.0, -14.0),
            egui::Align2::LEFT_TOP,
            &slice.name,
            egui::FontId::proportional(11.0),
            Color32::from_rgb(140, 180, 255),
        );
    }

    // Shape previews: exactly the pixels the command will paint, in the
    // colours it will paint them.
    if let Some(gesture) = &view.gesture {
        let shift = ui.input(|i| i.modifiers.shift);
        if let Some(shape) = shape_command(gesture, settings, shift) {
            for (point, colour) in editor.preview(&shape) {
                let ghost = Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), colour.a().min(220));
                painter.rect_filled(pixel_rect(point), 0.0, ghost);
            }
        }
        let end = gesture.current;
        match gesture.tool {
            Tool::Gradient => {
                painter.line_segment(
                    [to_screen(gesture.start) + Vec2::splat(zoom / 2.0), to_screen(end) + Vec2::splat(zoom / 2.0)],
                    Stroke::new(2.0_f32, colours.accent),
                );
            }
            Tool::Lasso => {
                let centre = Vec2::splat(zoom / 2.0);
                let trail: Vec<Pos2> = view.lasso.iter().map(|&p| to_screen(p) + centre).collect();
                painter.add(egui::Shape::line(trail, Stroke::new(1.5_f32, Color32::WHITE)));
            }
            Tool::Select => {
                let r = Rect::spanning(gesture.start, end);
                let rect = egui::Rect::from_min_max(to_screen((r.x, r.y)), to_screen((r.right(), r.bottom())));
                painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, Color32::WHITE), egui::StrokeKind::Middle);
            }
            _ => {}
        }
    }

    // Marching ants: alternate dark and light edges, shifting over time.
    let time = ui.input(|i| i.time);
    let march = ((time * 6.0) as i32) & 1;
    if let Some(selection) = &editor.selection {
        for ((x0, y0), (x1, y1)) in selection.outline_segments() {
            let colour = if (x0 + y0 + march) % 2 == 0 { Color32::WHITE } else { Color32::BLACK };
            painter.line_segment([to_screen((x0, y0)), to_screen((x1, y1))], Stroke::new(1.5_f32, colour));
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(160));
    }
    if let Some(floating) = &editor.floating {
        let b = floating.bounds();
        let rect = egui::Rect::from_min_max(to_screen((b.x, b.y)), to_screen((b.right(), b.bottom())));
        painter.rect_stroke(rect, 0.0, Stroke::new(1.5_f32, colours.accent), egui::StrokeKind::Middle);
    }

    for (index, pin) in sprite.pins.iter().enumerate() {
        let centre = to_screen((pin.x, pin.y)) + Vec2::splat(zoom / 2.0);
        painter.circle_filled(centre + Vec2::new(0.0, 2.0), 7.0, Color32::from_black_alpha(80));
        painter.circle_filled(centre, 7.0, colours.glass_strong);
        painter.circle_stroke(centre, 7.0, Stroke::new(1.5_f32, Color32::from_rgb(255, 196, 64)));
        painter.text(
            centre,
            egui::Align2::CENTER_CENTER,
            (index + 1).to_string(),
            egui::FontId::proportional(9.0),
            colours.text,
        );
        let hover = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| p.distance(centre) < 9.0);
        if hover {
            let galley = painter.layout(pin.text.clone(), egui::FontId::proportional(13.0), colours.text, 240.0);
            let at = centre + Vec2::new(12.0, -galley.size().y / 2.0);
            let bubble = egui::Rect::from_min_size(at, galley.size()).expand(8.0);
            painter.rect_filled(bubble, 10.0, colours.glass_strong);
            painter.rect_stroke(bubble, 10.0, Stroke::new(1.0_f32, colours.edge), egui::StrokeKind::Inside);
            painter.galley(at, galley, colours.text);
        }
    }

    // The brush footprint under the pointer.
    if let (Some(hovered), true) = (view.hovered, view.gesture.is_none()) {
        let footprint = if settings.tool.is_stroke() || settings.tool.is_shape() {
            editor.footprint(&[hovered], settings.brush)
        } else {
            vec![hovered]
        };
        for point in footprint {
            painter.rect_stroke(
                pixel_rect(point),
                0.0,
                Stroke::new(1.0_f32, Color32::from_white_alpha(160)),
                egui::StrokeKind::Inside,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_climb_and_descend_through_the_ladder() {
        let mut view = CanvasView { zoom: 8.0, ..CanvasView::default() };
        view.zoom_in();
        assert_eq!(view.zoom, 10.0);
        view.zoom_out();
        view.zoom_out();
        assert_eq!(view.zoom, 6.0);
        view.zoom = 1.0;
        view.zoom_out();
        assert_eq!(view.zoom, 1.0);
    }

    #[test]
    fn shift_constrains_shapes_to_squares_in_any_direction() {
        assert_eq!(square_up((0, 0), (5, 2)), (5, 5));
        assert_eq!(square_up((10, 10), (7, 12)), (7, 13));
    }
}

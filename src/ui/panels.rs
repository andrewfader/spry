//! The floating glass panels. Each one is a view over `App` state that
//! sends `Action`s and editor `Command`s. None of them edits a document
//! directly.

use super::app::{Action, App, Panel, ResizeDraft};
use super::settings::ChatProvider;
use super::theme::{self, Appearance};
use super::tools::Tool;
use super::workers::{ChatLine, DiffusionJob};
use crate::ai::diffusion::{self, Generation, Landing};
use crate::editor::dither::CLASSIC_PATTERNS;
use crate::editor::limits::MAX_CANVAS_SIDE;
use crate::editor::playback;
use crate::editor::{BlendMode, Command, LoopDirection, Palette, Pattern, Rgba};
use crate::io::{SheetLayout, SheetOptions};
use crate::studio::Studio;
use egui::{Align2, Color32, RichText, Sense, Vec2};

pub struct GenerateForm {
    pub prompt: String,
    pub preset: usize,
    pub seed: i64,
    pub random_seed: bool,
    pub from_current_frame: bool,
    pub strength: f32,
    pub clear_background: bool,
    pub dither: bool,
    pub use_lora: bool,
    pub lora_name: String,
    pub lora_weight: f32,
}

impl Default for GenerateForm {
    fn default() -> Self {
        GenerateForm {
            prompt: "a small knight with a blue cape, side view".into(),
            preset: 0,
            seed: 1,
            random_seed: true,
            from_current_frame: false,
            strength: 0.6,
            clear_background: true,
            dither: false,
            use_lora: false,
            lora_name: String::new(),
            lora_weight: 1.0,
        }
    }
}

fn to_colour32(colour: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), colour.a())
}

fn from_colour32(colour: Color32) -> Rgba {
    let [r, g, b, a] = colour.to_srgba_unmultiplied();
    Rgba::rgba(r, g, b, a)
}

/// A clickable colour chip. A primary click picks it, a secondary click
/// picks it as the secondary colour.
fn swatch(ui: &mut egui::Ui, colour: Rgba, size: f32, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 5.0, to_colour32(colour));
    let edge = if selected {
        egui::Stroke::new(2.0_f32, Color32::WHITE)
    } else {
        egui::Stroke::new(1.0_f32, Color32::from_white_alpha(40))
    };
    painter.rect_stroke(rect, 5.0, edge, egui::StrokeKind::Inside);
    response.on_hover_text(colour.to_hex())
}

/// Lays out a panel the first time it appears, relative to the screen.
fn window(app: &App, panel: Panel, pos: egui::Pos2, width: f32) -> egui::Window<'static> {
    egui::Window::new(panel.title())
        .id(egui::Id::new(("panel", panel.title())))
        .frame(app.glass())
        .default_pos(pos)
        .default_width(width)
        .collapsible(true)
        .resizable(true)
}

impl App {
    pub fn show_panels(&mut self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();
        self.top_bar(ctx);
        self.tool_dock(ctx);
        self.tool_inspector(ctx);
        self.status_bar(ctx);

        let right = screen.right() - 290.0;
        let mut toggles: Vec<Panel> = Vec::new();
        for panel in [
            Panel::Layers,
            Panel::Palette,
            Panel::Preview,
            Panel::Timeline,
            Panel::Versions,
            Panel::Notes,
            Panel::Assistant,
            Panel::Generate,
            Panel::Export,
            Panel::Settings,
            Panel::NewDocument,
            Panel::ResizeCanvas,
        ] {
            if !self.is_open(panel) {
                continue;
            }
            let mut open = true;
            let (pos, width) = match panel {
                Panel::Layers => (egui::pos2(right, 70.0), 270.0),
                Panel::Palette => (egui::pos2(right, 330.0), 270.0),
                Panel::Preview => (egui::pos2(right - 210.0, 70.0), 190.0),
                Panel::Timeline => (egui::pos2(160.0, screen.bottom() - 190.0), (screen.width() - 470.0).max(400.0)),
                Panel::Versions | Panel::Notes => (egui::pos2(right - 330.0, 260.0), 310.0),
                Panel::Assistant | Panel::Generate => (egui::pos2(170.0, 120.0), 360.0),
                _ => (screen.center() - Vec2::new(200.0, 160.0), 400.0),
            };
            let shown = window(self, panel, pos, width).open(&mut open).show(ctx, |ui| match panel {
                Panel::Layers => self.layers_panel(ui),
                Panel::Palette => self.palette_panel(ui),
                Panel::Preview => self.preview_panel(ui),
                Panel::Timeline => self.timeline_panel(ctx, ui),
                Panel::Versions => self.versions_panel(ctx, ui),
                Panel::Notes => self.notes_panel(ui),
                Panel::Assistant => self.assistant_panel(ctx, ui),
                Panel::Generate => self.generate_panel(ctx, ui),
                Panel::Export => self.export_panel(ctx, ui),
                Panel::Settings => self.settings_panel(ctx, ui),
                Panel::NewDocument => self.new_document_panel(ui),
                Panel::ResizeCanvas => self.resize_panel(ui),
            });
            if !open || shown.is_none() {
                toggles.push(panel);
            }
        }
        for panel in toggles {
            self.open_panels.remove(&panel);
        }
        self.pin_editor(ctx);
        if self.palette_open {
            self.command_palette(ctx);
        }
    }

    // ----- chrome -------------------------------------------------------

    fn top_bar(&mut self, ctx: &egui::Context) {
        let mut action: Option<Action> = None;
        egui::Area::new(egui::Id::new("top-bar")).anchor(Align2::LEFT_TOP, [12.0, 10.0]).show(ctx, |ui| {
            theme::pill_frame(self.appearance()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("◆ Spry").strong().color(theme::palette(self.appearance()).accent));
                    ui.menu_button("File", |ui| {
                        for (label, item) in [
                            ("New sprite…   Ctrl+N", Action::New),
                            ("Open…   Ctrl+O", Action::Open),
                            ("Save   Ctrl+S", Action::Save),
                            ("Save as…   Ctrl+Shift+S", Action::SaveAs),
                            ("Import sprite sheet…", Action::ImportSheet),
                            ("Close   Ctrl+W", Action::Close),
                        ] {
                            if ui.button(label).clicked() {
                                action = Some(item);
                                ui.close_menu();
                            }
                        }
                        let recent = self.settings.recent_files.clone();
                        if !recent.is_empty() {
                            ui.separator();
                            ui.weak("Recent");
                            for path in recent {
                                let label = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
                                if ui.button(label).on_hover_text(path.display().to_string()).clicked() {
                                    self.open_path(&path);
                                    ui.close_menu();
                                }
                            }
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        for (label, item) in [
                            ("Undo   Ctrl+Z", Action::Edit(Command::Undo)),
                            ("Redo   Ctrl+Y", Action::Edit(Command::Redo)),
                            ("Cut   Ctrl+X", Action::Edit(Command::Cut)),
                            ("Copy   Ctrl+C", Action::Edit(Command::Copy)),
                            ("Paste   Ctrl+V", Action::Edit(Command::Paste { at: None })),
                            ("Select all   Ctrl+A", Action::Edit(Command::SelectAll)),
                            ("Deselect   Ctrl+D", Action::Edit(Command::SelectNone)),
                            ("Invert selection   Ctrl+Shift+I", Action::Edit(Command::InvertSelection)),
                            ("Flip horizontal   Shift+H", Action::Edit(Command::FlipHorizontal)),
                            ("Flip vertical   Shift+V", Action::Edit(Command::FlipVertical)),
                            ("Rotate 90° clockwise   Shift+R", Action::Edit(Command::RotateClockwise)),
                            ("Rotate 90° counter-clockwise", Action::Edit(Command::RotateCounterClockwise)),
                            ("Canvas size…", Action::Toggle(Panel::ResizeCanvas)),
                        ] {
                            if ui.button(label).clicked() {
                                action = Some(item);
                                ui.close_menu();
                            }
                        }
                    });
                    ui.menu_button("Effects", |ui| {
                        let primary = self.tools.primary;
                        for (label, item) in [
                            ("Outline (primary colour)", Command::Outline { colour: primary, diagonals: false }),
                            ("Thick outline (8-way)", Command::Outline { colour: primary, diagonals: true }),
                            ("Drop shadow ↘", Command::DropShadow { colour: primary, dx: 1, dy: 1 }),
                            ("Snap to palette", Command::SnapToPalette { dither: false }),
                            ("Snap to palette with dither", Command::SnapToPalette { dither: true }),
                            (
                                "Replace primary with secondary everywhere",
                                Command::ReplaceColour { from: primary, to: self.tools.secondary },
                            ),
                        ] {
                            if ui.button(label).clicked() {
                                action = Some(Action::Edit(item));
                                ui.close_menu();
                            }
                        }
                    });
                    ui.menu_button("View", |ui| {
                        for (label, item) in [
                            ("Zoom in   Ctrl++", Action::ZoomIn),
                            ("Zoom out   Ctrl+-", Action::ZoomOut),
                            ("Fit to window   Ctrl+0", Action::ZoomToFit),
                            ("Pixel grid", Action::ToggleGrid),
                            ("Onion skin   Tab", Action::ToggleOnionSkin),
                            ("Tiled preview (seamless check)", Action::ToggleTiledPreview),
                            ("Light / dark glass", Action::ToggleAppearance),
                        ] {
                            if ui.button(label).clicked() {
                                action = Some(item);
                                ui.close_menu();
                            }
                        }
                    });
                    ui.separator();
                    for panel in [
                        Panel::Layers,
                        Panel::Timeline,
                        Panel::Palette,
                        Panel::Preview,
                        Panel::Versions,
                        Panel::Notes,
                        Panel::Export,
                    ] {
                        if ui.selectable_label(self.is_open(panel), panel.title()).clicked() {
                            action = Some(Action::Toggle(panel));
                        }
                    }
                    ui.separator();
                    if ui.selectable_label(self.is_open(Panel::Assistant), "🤖 Assistant").clicked() {
                        action = Some(Action::Toggle(Panel::Assistant));
                    }
                    if ui.selectable_label(self.is_open(Panel::Generate), "✨ Generate").clicked() {
                        action = Some(Action::Toggle(Panel::Generate));
                    }
                    if ui.button("⚙").on_hover_text("Settings").clicked() {
                        action = Some(Action::Toggle(Panel::Settings));
                    }
                    if ui.button("⌘K").on_hover_text("Command palette").clicked() {
                        action = Some(Action::CommandPalette);
                    }
                });
            });
        });
        // Document tabs, centred under the bar.
        if self.documents.len() > 1 {
            egui::Area::new(egui::Id::new("tabs")).anchor(Align2::CENTER_TOP, [0.0, 58.0]).show(ctx, |ui| {
                theme::pill_frame(self.appearance()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for index in 0..self.documents.len() {
                            let studio = &self.documents[index].studio;
                            let dirty = if studio.editor.has_unsaved_changes() { " •" } else { "" };
                            if ui.selectable_label(index == self.active, format!("{}{dirty}", studio.title())).clicked()
                            {
                                self.active = index;
                            }
                        }
                    });
                });
            });
        }
        if let Some(action) = action {
            self.run(ctx, action);
        }
    }

    fn tool_dock(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("tool-dock")).anchor(Align2::LEFT_CENTER, [12.0, 0.0]).show(ctx, |ui| {
            theme::pill_frame(self.appearance()).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for tool in Tool::ALL {
                        let hint = match tool.shortcut() {
                            Some(key) => format!("{} ({})", tool.name(), key.name()),
                            None => tool.name().to_string(),
                        };
                        let button = egui::Button::new(RichText::new(tool.icon()).size(17.0))
                            .min_size(Vec2::splat(34.0))
                            .selected(self.tools.tool == tool);
                        if ui.add(button).on_hover_text(hint).clicked() {
                            self.tools.tool = tool;
                        }
                    }
                    ui.add_space(6.0);
                    // Primary over secondary, like every paint program.
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
                    let painter = ui.painter();
                    let back = egui::Rect::from_min_size(rect.min + Vec2::splat(12.0), Vec2::splat(22.0));
                    let front = egui::Rect::from_min_size(rect.min, Vec2::splat(22.0));
                    painter.rect_filled(back, 5.0, to_colour32(self.tools.secondary));
                    painter.rect_stroke(
                        back,
                        5.0,
                        egui::Stroke::new(1.0_f32, Color32::from_white_alpha(80)),
                        egui::StrokeKind::Inside,
                    );
                    painter.rect_filled(front, 5.0, to_colour32(self.tools.primary));
                    painter.rect_stroke(
                        front,
                        5.0,
                        egui::Stroke::new(1.5_f32, Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                    if response.on_hover_text("Swap colours (X)").clicked() {
                        self.tools.swap_colours();
                    }
                });
            });
        });
    }

    fn tool_inspector(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("tool-inspector"))
            .anchor(Align2::CENTER_TOP, [0.0, if self.documents.len() > 1 { 104.0 } else { 60.0 }])
            .show(ctx, |ui| {
                theme::pill_frame(self.appearance()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let tool = self.tools.tool;
                        ui.label(RichText::new(format!("{} {}", tool.icon(), tool.name())).strong());
                        ui.separator();
                        let mut primary = to_colour32(self.tools.primary);
                        if egui::color_picker::color_edit_button_srgba(
                            ui,
                            &mut primary,
                            egui::color_picker::Alpha::OnlyBlend,
                        )
                        .changed()
                        {
                            self.tools.primary = from_colour32(primary);
                        }
                        let mut secondary = to_colour32(self.tools.secondary);
                        if egui::color_picker::color_edit_button_srgba(
                            ui,
                            &mut secondary,
                            egui::color_picker::Alpha::OnlyBlend,
                        )
                        .changed()
                        {
                            self.tools.secondary = from_colour32(secondary);
                        }
                        if tool.is_stroke() || tool.is_shape() {
                            ui.separator();
                            ui.label("Size");
                            ui.add(egui::DragValue::new(&mut self.tools.brush.size).range(1..=64));
                            ui.checkbox(&mut self.tools.brush.round, "Round");
                            ui.toggle_value(&mut self.tools.brush.mirror_x, "⇆").on_hover_text("Mirror left ↔ right");
                            ui.toggle_value(&mut self.tools.brush.mirror_y, "⇅").on_hover_text("Mirror top ↕ bottom");
                        }
                        match tool {
                            Tool::Pencil => {
                                ui.checkbox(&mut self.tools.pixel_perfect, "Pixel-perfect");
                            }
                            Tool::Rectangle | Tool::Ellipse => {
                                ui.checkbox(&mut self.tools.filled, "Filled");
                                self.pattern_picker(ui);
                            }
                            Tool::Dither => self.pattern_picker(ui),
                            Tool::Fill => {
                                ui.checkbox(&mut self.tools.contiguous, "Contiguous");
                                self.pattern_picker(ui);
                            }
                            Tool::Wand => {
                                ui.checkbox(&mut self.tools.contiguous, "Contiguous");
                            }
                            Tool::Shade => {
                                ui.radio_value(&mut self.tools.shade_lightens, true, "Lighten");
                                ui.radio_value(&mut self.tools.shade_lightens, false, "Darken");
                            }
                            Tool::Spray => {
                                ui.label("Radius");
                                ui.add(egui::DragValue::new(&mut self.tools.spray_radius).range(1..=32));
                                ui.label("Density");
                                ui.add(egui::DragValue::new(&mut self.tools.spray_density).range(1..=64));
                            }
                            Tool::CloneStamp => {
                                let source = match self.tools.clone_anchor {
                                    Some((x, y)) => format!("source ({x}, {y})"),
                                    None => "Alt-click to pick a source".into(),
                                };
                                ui.weak(source);
                            }
                            Tool::Gradient => {
                                ui.weak("Drag from secondary to primary");
                            }
                            Tool::Select | Tool::Move => {
                                ui.weak("Shift adds · arrows nudge · Esc drops");
                            }
                            Tool::Pin => {
                                ui.weak("Click the canvas to pin a note");
                            }
                            _ => {}
                        }
                    });
                });
            });
    }

    fn pattern_picker(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        egui::ComboBox::from_id_salt("pattern").selected_text(format!("▦ {}", self.tools.pattern.name())).show_ui(
            ui,
            |ui| {
                for pattern in CLASSIC_PATTERNS {
                    ui.selectable_value(
                        &mut self.tools.pattern,
                        pattern,
                        format!("{} ({:.0}%)", pattern.name(), pattern.coverage() * 100.0),
                    );
                }
                ui.separator();
                for size in [2u8, 4, 8] {
                    ui.menu_button(format!("Bayer {size}×{size}"), |ui| {
                        for level in 0..=size * size {
                            let pattern = Pattern::Bayer { size, level };
                            ui.selectable_value(&mut self.tools.pattern, pattern, pattern.name());
                        }
                    });
                }
            },
        );
        ui.checkbox(&mut self.tools.pattern_two_colour, "Two-colour");
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("status")).anchor(Align2::LEFT_BOTTOM, [12.0, -10.0]).show(ctx, |ui| {
            theme::pill_frame(self.appearance()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let document = self.document();
                    let sprite = document.studio.editor.sprite();
                    ui.weak(format!("{}×{}", sprite.width, sprite.height));
                    ui.weak(format!("{:.0}×", document.view.zoom));
                    if let Some((x, y)) = document.view.hovered {
                        ui.weak(format!("({x}, {y})"));
                    }
                    if let Some(bridge) = &self.mcp {
                        ui.weak(format!("● MCP {}", bridge.address)).on_hover_text(bridge.url());
                    }
                    if self.agent.busy {
                        ui.spinner();
                        ui.weak("assistant working…");
                    }
                    if let Some(job) = &self.diffusion {
                        ui.spinner();
                        ui.weak(format!("generating “{}”", job.prompt));
                    }
                    ui.separator();
                    ui.label(&self.status);
                });
            });
        });
    }

    // ----- layers -------------------------------------------------------

    fn layers_panel(&mut self, ui: &mut egui::Ui) {
        let editor = &self.studio().editor;
        let sprite = editor.sprite();
        let rows: Vec<LayerRow> = (0..sprite.layers.len()).map(|index| LayerRow::of(sprite, index)).collect();
        let current = editor.layer;
        let cel_opacity = sprite.cel(current, editor.frame).map(|cel| cel.opacity);
        let mut commands: Vec<Command> = Vec::new();
        ui.horizontal(|ui| {
            if ui.button("➕").on_hover_text("New layer").clicked() {
                commands.push(Command::AddLayer { name: None });
            }
            if ui.button("📁").on_hover_text("New group").clicked() {
                commands.push(Command::AddGroup { name: None });
            }
            if ui.button("⧉").on_hover_text("Duplicate layer (a group copies everything in it)").clicked() {
                commands.push(Command::DuplicateLayer);
            }
            if ui.button("⤓").on_hover_text("Merge down").clicked() {
                commands.push(Command::MergeDown);
            }
            if ui.button("⏶").on_hover_text("Move up").clicked() && current + 1 < rows.len() {
                commands.push(Command::MoveLayer { to: current + 1 });
            }
            if ui.button("⏷").on_hover_text("Move down").clicked() && current > 0 {
                commands.push(Command::MoveLayer { to: current - 1 });
            }
            if ui.button("🗑").on_hover_text("Delete layer (a group goes with everything in it)").clicked() {
                commands.push(Command::RemoveLayer);
            }
        });
        ui.separator();
        // Top of the stack first, as artists expect, with children indented
        // under their group.
        for index in (0..rows.len()).rev() {
            let row = &rows[index];
            ui.horizontal(|ui| {
                let (visible, locked) = (row.visible, row.locked);
                if ui.selectable_label(visible, if visible { "👁" } else { "–" }).on_hover_text("Show / hide").clicked()
                {
                    commands.extend(on_layer(index, current, Command::SetLayerVisible { visible: !visible }));
                }
                if ui
                    .selectable_label(locked, if locked { "🔒" } else { "🔓" })
                    .on_hover_text("Lock / unlock")
                    .clicked()
                {
                    commands.extend(on_layer(index, current, Command::SetLayerLocked { locked: !locked }));
                }
                ui.add_space(row.depth as f32 * 14.0);
                let label = if row.is_group { format!("📁 {}", row.name) } else { row.name.clone() };
                let width = (150.0 - row.depth as f32 * 14.0).max(60.0);
                let response = ui.add_sized([width, 24.0], egui::SelectableLabel::new(index == current, label));
                if response.clicked() {
                    commands.push(Command::SelectLayer { index });
                }
                response.context_menu(|ui| {
                    ui.label(RichText::new("Move into group").weak());
                    for group in rows.iter().enumerate().filter(|(g, r)| r.is_group && *g != index) {
                        if ui.button(format!("📁 {}", group.1.name)).clicked() {
                            // Indexes shift as the layer moves, so stay with it.
                            commands.extend([
                                Command::SelectLayer { index },
                                Command::SetLayerParent { group: Some(group.0) },
                            ]);
                            ui.close_menu();
                        }
                    }
                    if row.depth > 0 && ui.button("Out to the top level").clicked() {
                        commands.extend([Command::SelectLayer { index }, Command::SetLayerParent { group: None }]);
                        ui.close_menu();
                    }
                });
            });
        }
        ui.separator();
        let LayerRow { name: saved_name, mut opacity, mut blend, .. } = rows[current].clone();
        // The draft lives across frames; reading the name back from the
        // document each frame would throw away every keystroke.
        let mut name = match &self.layer_name_draft {
            Some((layer, draft)) if *layer == current => draft.clone(),
            _ => saved_name.clone(),
        };
        ui.horizontal(|ui| {
            ui.label("Name");
            let response = ui.text_edit_singleline(&mut name);
            if response.changed() {
                self.layer_name_draft = Some((current, name.clone()));
            }
            if response.lost_focus() {
                if name != saved_name && !name.trim().is_empty() {
                    commands.push(Command::RenameLayer { name: name.clone() });
                }
                self.layer_name_draft = None;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Opacity");
            if ui.add(egui::Slider::new(&mut opacity, 0..=255)).drag_stopped() {
                commands.push(Command::SetLayerOpacity { opacity });
            }
        });
        ui.horizontal(|ui| {
            ui.label("Blend");
            egui::ComboBox::from_id_salt("layer-blend").selected_text(blend.name()).show_ui(ui, |ui| {
                for mode in BlendMode::ALL {
                    if ui.selectable_value(&mut blend, mode, mode.name()).clicked() {
                        commands.push(Command::SetLayerBlend { blend: mode });
                    }
                }
            });
        });
        if let Some(mut cel_opacity) = cel_opacity {
            ui.horizontal(|ui| {
                ui.label("Cel");
                let slider = egui::Slider::new(&mut cel_opacity, 0..=255).text("opacity on this frame");
                if ui.add(slider).drag_stopped() {
                    commands.push(Command::SetCelOpacity { opacity: cel_opacity });
                }
            });
        }
        for command in commands {
            self.edit(command);
        }
    }

    // ----- palette ------------------------------------------------------

    fn palette_panel(&mut self, ui: &mut egui::Ui) {
        let palette = self.studio().editor.sprite().palette.clone();
        let mut commands = Vec::new();
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("palette-choice").selected_text(&palette.name).show_ui(ui, |ui| {
                for built_in in Palette::built_ins() {
                    if ui
                        .selectable_label(
                            built_in.name == palette.name,
                            format!("{} ({})", built_in.name, built_in.colours.len()),
                        )
                        .clicked()
                    {
                        commands.push(Command::SetPalette { palette: built_in });
                    }
                }
            });
            if ui.button("⤒").on_hover_text("Extract palette from the sprite").clicked() {
                commands.push(Command::ExtractPalette);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(4.0);
            for (index, &colour) in palette.colours.iter().enumerate() {
                let response = swatch(ui, colour, 22.0, colour == self.tools.primary);
                if response.clicked() {
                    self.tools.primary = colour;
                }
                if response.secondary_clicked() {
                    self.tools.secondary = colour;
                }
                response.context_menu(|ui| {
                    if ui.button("Set to primary colour").clicked() {
                        commands.push(Command::SetPaletteColour { index, colour: self.tools.primary });
                        ui.close_menu();
                    }
                    if ui.button("Remove").clicked() {
                        commands.push(Command::RemovePaletteColour { index });
                        ui.close_menu();
                    }
                });
            }
        });
        ui.horizontal(|ui| {
            if ui.button("➕ Add primary").clicked() {
                commands.push(Command::AddPaletteColour { colour: self.tools.primary });
            }
            if ui.button("Load…").clicked() {
                self.run(ui.ctx(), Action::LoadPalette);
            }
            if ui.button("Save…").clicked() {
                self.run(ui.ctx(), Action::SavePalette);
            }
        });
        ui.weak("Left-click: primary · right-click: secondary · right-click menu to edit");
        for command in commands {
            self.edit(command);
        }
    }

    // ----- preview ------------------------------------------------------

    fn preview_panel(&mut self, ui: &mut egui::Ui) {
        let document = &self.documents[self.active];
        let sprite = document.studio.editor.sprite();
        let revision = document.studio.editor.revision;
        let prefix = document.texture_prefix();
        ui.horizontal(|ui| {
            let selected = self.preview_tag.clone().unwrap_or_else(|| "All frames".into());
            egui::ComboBox::from_id_salt("preview-tag").selected_text(selected).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.preview_tag, None, "All frames");
                for tag in &sprite.tags {
                    ui.selectable_value(&mut self.preview_tag, Some(tag.name.clone()), &tag.name);
                }
            });
        });
        let tag = self.preview_tag.as_deref().and_then(|name| sprite.tags.iter().find(|t| t.name == name));
        let elapsed = (ui.input(|i| i.time) * 1000.0) as u64;
        let frame = playback::frame_at(sprite, tag, elapsed);
        let texture =
            self.textures.get(ui.ctx(), &format!("{prefix}frame{frame}"), revision, || sprite.composite(frame)).id();
        let longest = sprite.width.max(sprite.height) as f32;
        for scale in [1.0f32, 2.0] {
            let size = Vec2::new(sprite.width as f32, sprite.height as f32) * scale;
            let size = if longest * scale > 170.0 { size * (170.0 / (longest * scale)) } else { size };
            ui.add(egui::Image::new((texture, size)));
        }
        ui.weak(format!("frame {} · {} ms", frame + 1, sprite.frames[frame].duration_ms));
        if let Some(wait) = playback::ms_until_next_frame(sprite, tag, elapsed) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(wait));
        }
    }

    // ----- timeline -----------------------------------------------------

    fn timeline_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mut commands = Vec::new();
        let mut actions = Vec::new();
        let document = &self.documents[self.active];
        let editor = &document.studio.editor;
        let sprite = editor.sprite();
        let prefix = document.texture_prefix();
        let revision = editor.revision;
        let current = editor.frame;
        let layer = editor.layer;
        ui.horizontal(|ui| {
            let play = if self.playing.is_some() { "⏸" } else { "▶" };
            if ui.button(play).on_hover_text("Play / pause (Enter)").clicked() {
                actions.push(Action::PlayPause);
            }
            if ui.button("⏮").clicked() {
                actions.push(Action::PreviousFrame);
            }
            if ui.button("⏭").clicked() {
                actions.push(Action::NextFrame);
            }
            ui.separator();
            if ui.button("➕ Frame").clicked() {
                commands.push(Command::AddFrame);
            }
            if ui.button("⧉ Copy").on_hover_text("Duplicate with independent pixels").clicked() {
                commands.push(Command::DuplicateFrame { as_clone: false });
            }
            if ui.button("🔗 Clone").on_hover_text("Duplicate as linked cels: painting one paints both").clicked() {
                commands.push(Command::DuplicateFrame { as_clone: true });
            }
            if editor.sprite().is_linked(layer, current) && ui.button("✂ Unlink").clicked() {
                commands.push(Command::UnlinkCel);
            }
            if ui.button("◀").on_hover_text("Move frame earlier").clicked() && current > 0 {
                commands.push(Command::MoveFrame { to: current - 1 });
            }
            if ui.button("▶").on_hover_text("Move frame later").clicked() && current + 1 < sprite.frame_count() {
                commands.push(Command::MoveFrame { to: current + 1 });
            }
            if ui.button("🗑").on_hover_text("Delete frame").clicked() {
                commands.push(Command::RemoveFrame);
            }
            ui.separator();
            let mut duration = sprite.frames[current].duration_ms;
            ui.label("ms");
            if ui.add(egui::DragValue::new(&mut duration).range(1..=10_000).speed(5)).changed() {
                commands.push(Command::SetFrameDuration { duration_ms: duration, all_frames: false });
            }
            let mut onion = document.view.onion_skin;
            if ui.toggle_value(&mut onion, "🧅 Onion").on_hover_text("Onion skin (Tab)").clicked() {
                actions.push(Action::ToggleOnionSkin);
            }
            if onion {
                let view = &document.view;
                let (mut before, mut after, mut opacity) = (view.onion_before, view.onion_after, view.onion_opacity);
                let mut changed = ui.add(egui::DragValue::new(&mut before).range(0..=8).prefix("◀ ")).changed();
                changed |= ui.add(egui::DragValue::new(&mut after).range(0..=8).prefix("▶ ")).changed();
                changed |= ui.add(egui::Slider::new(&mut opacity, 20..=220).show_value(false)).changed();
                if changed {
                    actions.push(Action::OnionSkin { before, after, opacity });
                }
            }
        });
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for frame in 0..sprite.frame_count() {
                    let texture = self
                        .textures
                        .get(ctx, &format!("{prefix}frame{frame}"), revision, || sprite.composite(frame))
                        .id();
                    let longest = sprite.width.max(sprite.height) as f32;
                    let size = Vec2::new(sprite.width as f32, sprite.height as f32) * (44.0 / longest);
                    ui.vertical(|ui| {
                        let response = ui
                            .add(egui::ImageButton::new((texture, size)).selected(frame == current).corner_radius(6.0));
                        let tags: Vec<&str> = sprite
                            .tags
                            .iter()
                            .filter(|t| (t.from..=t.to).contains(&frame))
                            .map(|t| t.name.as_str())
                            .collect();
                        let link = if sprite.is_linked(layer, frame) { "🔗" } else { "" };
                        ui.small(format!("{}{link}", frame + 1));
                        if !tags.is_empty() {
                            ui.small(
                                RichText::new(tags.join(",")).color(theme::palette(self.settings.appearance).accent),
                            );
                        }
                        if response.clicked() {
                            commands.push(Command::SelectFrame { index: frame });
                        }
                        if !sprite.frames[frame].note.is_empty() {
                            response.on_hover_text(&sprite.frames[frame].note);
                        }
                    });
                }
            });
            cel_grid(ui, sprite, layer, current, &mut commands);
        });
        ui.horizontal(|ui| {
            ui.weak("Tags:");
            for tag in &sprite.tags {
                let label = format!(
                    "{} {}–{} {}",
                    tag.name,
                    tag.from + 1,
                    tag.to + 1,
                    match tag.direction {
                        LoopDirection::Forward => "→",
                        LoopDirection::Reverse => "←",
                        LoopDirection::PingPong => "↔",
                    }
                );
                ui.label(label).context_menu(|ui| {
                    for (direction, name) in [
                        (LoopDirection::Forward, "Play forward →"),
                        (LoopDirection::Reverse, "Play in reverse ←"),
                        (LoopDirection::PingPong, "Ping-pong ↔"),
                    ] {
                        if ui.radio(tag.direction == direction, name).clicked() {
                            // Adding a tag under an existing name replaces it.
                            let (name, from, to) = (tag.name.clone(), tag.from, tag.to);
                            commands.push(Command::AddTag { name, from, to, direction });
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("Remove tag").clicked() {
                        commands.push(Command::RemoveTag { name: tag.name.clone() });
                        ui.close_menu();
                    }
                });
            }
            ui.menu_button("➕ Tag", |ui| {
                let id = ui.id().with("new-tag");
                let mut form: (String, usize, usize, LoopDirection) = ui.data_mut(|d| d.get_temp(id)).unwrap_or((
                    "walk".into(),
                    0,
                    sprite.frame_count() - 1,
                    LoopDirection::Forward,
                ));
                ui.text_edit_singleline(&mut form.0);
                let last = sprite.frame_count() - 1;
                ui.horizontal(|ui| {
                    ui.label("from");
                    ui.add(
                        egui::DragValue::new(&mut form.1)
                            .range(0..=last)
                            .custom_formatter(|n, _| format!("{}", n as usize + 1)),
                    );
                    ui.label("to");
                    ui.add(
                        egui::DragValue::new(&mut form.2)
                            .range(0..=last)
                            .custom_formatter(|n, _| format!("{}", n as usize + 1)),
                    );
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut form.3, LoopDirection::Forward, "→");
                    ui.radio_value(&mut form.3, LoopDirection::Reverse, "←");
                    ui.radio_value(&mut form.3, LoopDirection::PingPong, "↔ ping-pong");
                });
                if ui.button("Add tag").clicked() {
                    commands.push(Command::AddTag {
                        name: form.0.clone(),
                        from: form.1,
                        to: form.2,
                        direction: form.3,
                    });
                    ui.close_menu();
                }
                ui.data_mut(|d| d.insert_temp(id, form));
            });
        });
        for command in commands {
            self.edit(command);
        }
        for action in actions {
            self.run(ctx, action);
        }
    }

    // ----- versions -------------------------------------------------------

    fn versions_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mut commands = Vec::new();
        let mut branch: Option<u32> = None;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.version_message).hint_text("What changed?").desired_width(180.0),
            );
            if ui.button("Commit").clicked() {
                commands.push(Command::CommitVersion { message: std::mem::take(&mut self.version_message) });
            }
        });
        ui.weak("Saving also records a version automatically.");
        ui.separator();
        let document = &self.documents[self.active];
        let prefix = document.texture_prefix();
        let editor = &document.studio.editor;
        let versions = &editor.document.versions;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let frame_of = |sprite: &crate::editor::Sprite| editor.frame.min(sprite.frame_count() - 1);
        if let Some(version) = self.selected_version.and_then(|id| editor.document.version(id)) {
            // Only the chosen version is compared, and only when the
            // document changes, so a long history costs nothing per frame.
            let key = (document.id, version.id, editor.revision);
            let old = || version.sprite.composite(frame_of(&version.sprite));
            let changed = match self.version_diff {
                Some((cached_key, count)) if cached_key == key => count,
                _ => {
                    let count =
                        crate::editor::document::changed_pixels(&old(), &editor.composite_for_display(editor.frame))
                            .len();
                    self.version_diff = Some((key, count));
                    count
                }
            };
            let overlay = self
                .textures
                .get(ctx, &format!("{prefix}diff{}", version.id), editor.revision, || {
                    crate::editor::document::difference_overlay(&old(), &editor.composite_for_display(editor.frame))
                })
                .id();
            let thumbnail =
                self.textures.get(ctx, &format!("{prefix}version{}", version.id), version.id as u64, old).id();
            let size = Vec2::new(version.sprite.width as f32, version.sprite.height as f32)
                * (110.0 / version.sprite.width.max(version.sprite.height) as f32);
            ui.label(RichText::new(format!("Version {} · {}", version.id, version.message)).strong());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.add(egui::Image::new((thumbnail, size)));
                    ui.weak("then");
                });
                ui.vertical(|ui| {
                    ui.add(egui::Image::new((overlay, size)));
                    ui.weak(if changed == 0 {
                        "same as now".to_string()
                    } else {
                        format!("{changed} px changed since")
                    });
                });
            });
            ui.separator();
        }
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            if versions.is_empty() {
                ui.weak("No versions yet.");
            }
            for version in versions.iter().rev() {
                let texture = self
                    .textures
                    .get(ctx, &format!("{prefix}version{}", version.id), version.id as u64, || {
                        version.sprite.composite(frame_of(&version.sprite))
                    })
                    .id();
                ui.horizontal(|ui| {
                    let longest = version.sprite.width.max(version.sprite.height) as f32;
                    ui.add(egui::Image::new((
                        texture,
                        Vec2::new(version.sprite.width as f32, version.sprite.height as f32) * (40.0 / longest),
                    )));
                    ui.vertical(|ui| {
                        let chosen = self.selected_version == Some(version.id);
                        let title = RichText::new(format!("{} · {}", version.id, version.message)).strong();
                        if ui.selectable_label(chosen, title).on_hover_text("Compare with now").clicked() {
                            self.selected_version = if chosen { None } else { Some(version.id) };
                        }
                        ui.weak(time_ago(now.saturating_sub(version.created_at)));
                        ui.horizontal(|ui| {
                            if ui.small_button("Restore").clicked() {
                                commands.push(Command::RestoreVersion { id: version.id });
                            }
                            if ui.small_button("Branch").on_hover_text("Open this version as a new document").clicked()
                            {
                                branch = Some(version.id);
                            }
                        });
                    });
                });
                ui.separator();
            }
        });
        for command in commands {
            self.edit(command);
        }
        if let Some(id) = branch {
            if let Some(document) = self.studio().editor.document.branch(id) {
                self.add_document(Studio::from_document(document));
                self.status = format!("Branched version {id} into a new document");
            }
        }
    }

    // ----- notes ----------------------------------------------------------

    fn notes_panel(&mut self, ui: &mut egui::Ui) {
        let mut commands = Vec::new();
        let editor = &self.studio().editor;
        let sprite = editor.sprite();
        let frame = editor.frame;
        ui.label(RichText::new("Document").strong());
        let mut notes = sprite.notes.clone();
        if ui
            .add(
                egui::TextEdit::multiline(&mut notes)
                    .hint_text("Art direction, references, TODOs…")
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            )
            .changed()
        {
            commands.push(Command::SetNotes { text: notes });
        }
        ui.label(RichText::new(format!("Frame {}", frame + 1)).strong());
        let mut frame_note = sprite.frames[frame].note.clone();
        if ui
            .add(
                egui::TextEdit::multiline(&mut frame_note)
                    .hint_text("Timing, poses, effects…")
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            )
            .changed()
        {
            commands.push(Command::SetFrameNote { frame, text: frame_note });
        }
        ui.label(RichText::new("Pins").strong());
        if sprite.pins.is_empty() {
            ui.weak("Use the 📌 tool to pin a note to a pixel.");
        }
        for (index, pin) in sprite.pins.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{} ({}, {})", index + 1, pin.x, pin.y));
                ui.label(&pin.text);
                if ui.small_button("✕").clicked() {
                    commands.push(Command::RemovePin { index });
                }
            });
        }
        for command in commands {
            self.edit(command);
        }
    }

    fn pin_editor(&mut self, ctx: &egui::Context) {
        let Some((x, y, mut text)) = self.pending_pin.take() else { return };
        let mut keep = true;
        let mut commit = false;
        egui::Window::new("Pin a note")
            .frame(self.glass())
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("At pixel ({x}, {y})"));
                let response = ui.add(
                    egui::TextEdit::multiline(&mut text).desired_rows(3).hint_text("What should someone know here?"),
                );
                response.request_focus();
                ui.horizontal(|ui| {
                    commit = ui.button("Pin").clicked();
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
        if commit && !text.trim().is_empty() {
            self.edit(Command::AddPin { x, y, text: text.trim().to_string() });
        } else if keep && !commit {
            self.pending_pin = Some((x, y, text));
        }
    }

    // ----- AI -------------------------------------------------------------

    fn assistant_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.weak(self.agent.provider_label());
            if ui.small_button("New chat").clicked() {
                let provider = self.settings.provider();
                self.agent.reset(provider);
            }
        });
        egui::ScrollArea::vertical().max_height(300.0).stick_to_bottom(true).auto_shrink([false, true]).show(ui, |ui| {
            if self.agent.lines.is_empty() {
                ui.weak("Ask for a change, like “add a 1px dark outline”, “make a 4-frame idle bob”, or “shade the sword with the palette ramp”. The assistant sees the canvas and edits with Spry's own tools, and one Ctrl+Z undoes its whole turn.");
            }
            let accent = theme::palette(self.appearance()).accent;
            for line in &self.agent.lines {
                match line {
                    ChatLine::User(text) => {
                        ui.label(RichText::new(format!("You: {text}")).strong());
                    }
                    ChatLine::Assistant(text) => {
                        ui.label(text);
                    }
                    ChatLine::Tool { name, ok, summary } => {
                        let mark = if *ok { "✓" } else { "✗" };
                        ui.small(RichText::new(format!("{mark} {name}: {summary}")).color(accent));
                    }
                    ChatLine::Error(error) => {
                        ui.colored_label(Color32::from_rgb(255, 120, 120), error);
                    }
                }
            }
        });
        ui.separator();
        let response = ui.add(
            egui::TextEdit::multiline(&mut self.chat_draft)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text("Describe a change… (Ctrl+Enter sends)"),
        );
        let send_shortcut =
            response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
        ui.horizontal(|ui| {
            let can_send = !self.agent.busy && !self.chat_draft.trim().is_empty();
            if (ui.add_enabled(can_send, egui::Button::new("Send")).clicked() || (send_shortcut && can_send))
                && can_send
            {
                self.send_chat(ctx);
            }
            if self.agent.busy {
                ui.spinner();
            }
        });
    }

    fn generate_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let presets = diffusion::presets();
        let form = &mut self.generate;
        ui.add(
            egui::TextEdit::multiline(&mut form.prompt)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text("What should it draw?"),
        );
        egui::ComboBox::from_label("Model preset")
            .selected_text(&presets[form.preset.min(presets.len() - 1)].name)
            .show_ui(ui, |ui| {
                for (index, preset) in presets.iter().enumerate() {
                    if ui.selectable_value(&mut form.preset, index, &preset.name).clicked() {
                        if let Some(suggested) = &preset.suggested_lora {
                            (form.use_lora, form.lora_name) = (true, suggested.clone());
                        }
                    }
                }
            });
        ui.horizontal(|ui| {
            ui.checkbox(&mut form.use_lora, "LoRA");
            ui.add_enabled(
                form.use_lora,
                egui::TextEdit::singleline(&mut form.lora_name).hint_text("e.g. pixel-art-xl").desired_width(140.0),
            );
            ui.add_enabled(form.use_lora, egui::DragValue::new(&mut form.lora_weight).range(0.0..=2.0).speed(0.05));
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut form.random_seed, "Random seed");
            ui.add_enabled(!form.random_seed, egui::DragValue::new(&mut form.seed));
        });
        ui.checkbox(&mut form.from_current_frame, "Restyle the current frame (img2img)");
        if form.from_current_frame {
            ui.add(egui::Slider::new(&mut form.strength, 0.1..=1.0).text("change"));
        }
        ui.checkbox(&mut form.clear_background, "Remove flat background");
        ui.checkbox(&mut form.dither, "Dither to the palette");
        let backend = self.settings.diffusion_backend();
        let backend_label = match &backend {
            Ok(diffusion::Backend::Automatic1111 { url }) => format!("Automatic1111 / Forge at {url}"),
            Ok(diffusion::Backend::ComfyUi { url, checkpoint, .. }) => format!("ComfyUI at {url} ({checkpoint})"),
            Err(error) => error.clone(),
        };
        ui.weak(format!("Backend: {backend_label} (change in ⚙ Settings)"));
        let busy = self.diffusion.is_some();
        let ready = !busy && backend.is_ok();
        if let (true, Ok(backend)) =
            (ui.add_enabled(ready, egui::Button::new("✨ Generate as new layer")).clicked(), backend)
        {
            let form = &mut self.generate;
            if form.random_seed {
                form.seed = fastrand::i64(0..i32::MAX as i64);
            }
            let editor = &self.documents[self.active].studio.editor;
            let sprite = editor.sprite();
            let generation = Generation {
                prompt: form.prompt.clone(),
                preset: presets[form.preset.min(presets.len() - 1)].clone(),
                seed: form.seed,
                init_image: form
                    .from_current_frame
                    .then(|| (editor.composite_for_display(editor.frame), form.strength)),
                canvas: (sprite.width, sprite.height),
                lora: (form.use_lora && !form.lora_name.trim().is_empty())
                    .then(|| diffusion::Lora { name: form.lora_name.trim().to_string(), weight: form.lora_weight }),
            };
            let landing = Landing {
                width: sprite.width,
                height: sprite.height,
                palette: sprite.palette.clone(),
                clear_background: form.clear_background,
                dither: form.dither,
            };
            let wake = ctx.clone();
            let document = self.document().id;
            self.diffusion =
                Some(DiffusionJob::start(document, backend, generation, landing, move || wake.request_repaint()));
            self.status = "Generating… the result arrives as a new layer, snapped to your palette".into();
        }
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Generating…");
            });
        }
    }

    // ----- export ---------------------------------------------------------

    fn export_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            ui.label("Scale");
            ui.add(egui::Slider::new(&mut self.export_scale, 1..=16).suffix("×"));
        });
        ui.horizontal(|ui| {
            if ui.button("PNG (frame)…").clicked() {
                actions.push(Action::ExportPng);
            }
            if ui.button("GIF…").on_hover_text("Uses the preview's tag selection").clicked() {
                actions.push(Action::ExportGif);
            }
            if ui.button("PNG sequence…").clicked() {
                actions.push(Action::ExportSequence);
            }
        });
        ui.separator();
        ui.label(RichText::new("Sprite sheet").strong());
        let tags: Vec<String> = self.studio().editor.sprite().tags.iter().map(|t| t.name.clone()).collect();
        let options: &mut SheetOptions = &mut self.sheet_options;
        let layout_name = match options.layout {
            SheetLayout::Horizontal => "Horizontal strip",
            SheetLayout::Vertical => "Vertical strip",
            SheetLayout::Grid { .. } => "Grid",
            SheetLayout::Packed => "Packed",
        };
        egui::ComboBox::from_label("Layout").selected_text(layout_name).show_ui(ui, |ui| {
            ui.selectable_value(&mut options.layout, SheetLayout::Horizontal, "Horizontal strip");
            ui.selectable_value(&mut options.layout, SheetLayout::Vertical, "Vertical strip");
            ui.selectable_value(&mut options.layout, SheetLayout::Grid { columns: None }, "Grid");
            ui.selectable_value(&mut options.layout, SheetLayout::Packed, "Packed");
        });
        if let SheetLayout::Grid { columns } = &mut options.layout {
            let mut fixed = columns.is_some();
            ui.horizontal(|ui| {
                ui.checkbox(&mut fixed, "Fixed columns");
                let mut count = columns.unwrap_or(4);
                if fixed {
                    ui.add(egui::DragValue::new(&mut count).range(1..=64));
                }
                *columns = fixed.then_some(count);
            });
        }
        ui.horizontal(|ui| {
            ui.label("Padding");
            ui.add(egui::DragValue::new(&mut options.padding).range(0..=32));
            ui.label("Border");
            ui.add(egui::DragValue::new(&mut options.border).range(0..=32));
        });
        ui.checkbox(&mut options.trim, "Trim empty edges");
        ui.checkbox(&mut options.merge_duplicates, "Merge duplicate frames");
        egui::ComboBox::from_label("Frames")
            .selected_text(options.tag.clone().unwrap_or_else(|| "All".into()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut options.tag, None, "All");
                for tag in tags {
                    ui.selectable_value(&mut options.tag, Some(tag.clone()), tag);
                }
            });
        if ui.button("Export sheet + Aseprite JSON…").clicked() {
            actions.push(Action::ExportSheet);
        }
        ui.separator();
        ui.label(RichText::new("Import").strong());
        ui.horizontal(|ui| {
            ui.label("Cell");
            ui.add(egui::DragValue::new(&mut self.sheet_cell.0).range(1..=1024));
            ui.label("×");
            ui.add(egui::DragValue::new(&mut self.sheet_cell.1).range(1..=1024));
            if ui
                .button("Import sheet…")
                .on_hover_text("The grid is detected from transparent gutters when possible")
                .clicked()
            {
                actions.push(Action::ImportSheet);
            }
        });
        for action in actions {
            self.run(ctx, action);
        }
    }

    // ----- settings -------------------------------------------------------

    fn settings_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let before = self.settings.clone();
        ui.label(RichText::new("Appearance").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.settings.appearance, Appearance::Dark, "Dark glass");
            ui.radio_value(&mut self.settings.appearance, Appearance::Light, "Light glass");
        });
        ui.separator();
        ui.label(RichText::new("Assistant").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.settings.chat_provider, ChatProvider::Claude, "Claude");
            ui.radio_value(&mut self.settings.chat_provider, ChatProvider::Ollama, "Ollama (local)");
        });
        match self.settings.chat_provider {
            ChatProvider::Claude => {
                ui.horizontal(|ui| {
                    ui.label("API key");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.anthropic_api_key)
                            .password(true)
                            .hint_text("or set ANTHROPIC_API_KEY"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Model");
                    ui.text_edit_singleline(&mut self.settings.claude_model);
                });
                if let Some(path) = super::settings::Settings::location() {
                    ui.weak(format!("Stored in {}", path.display()));
                }
            }
            ChatProvider::Ollama => {
                ui.horizontal(|ui| {
                    ui.label("URL");
                    ui.text_edit_singleline(&mut self.settings.ollama_url);
                });
                ui.horizontal(|ui| {
                    ui.label("Model");
                    ui.text_edit_singleline(&mut self.settings.ollama_model);
                });
                ui.weak("Pick a model with tool calling (and vision, to see the canvas).");
            }
        }
        ui.separator();
        ui.label(RichText::new("Diffusion").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.settings.use_comfy, false, "Automatic1111 / Forge");
            ui.radio_value(&mut self.settings.use_comfy, true, "ComfyUI");
        });
        if self.settings.use_comfy {
            ui.horizontal(|ui| {
                ui.label("URL");
                ui.text_edit_singleline(&mut self.settings.comfy_url);
            });
            ui.horizontal(|ui| {
                ui.label("Checkpoint");
                ui.text_edit_singleline(&mut self.settings.comfy_checkpoint);
            });
            ui.horizontal(|ui| {
                ui.label("Workflow");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.comfy_workflow)
                        .hint_text("built-in (or a path to an API-format .json)"),
                );
            });
        } else {
            ui.horizontal(|ui| {
                ui.label("URL");
                ui.text_edit_singleline(&mut self.settings.a1111_url);
            });
            ui.weak("Start it with --api.");
        }
        ui.separator();
        ui.label(RichText::new("MCP server").strong());
        let mut enabled = self.mcp.is_some();
        ui.horizontal(|ui| {
            ui.checkbox(&mut enabled, "Let LLM agents edit the open document over HTTP");
            ui.add_enabled(self.mcp.is_none(), egui::DragValue::new(&mut self.settings.mcp_port).range(1024..=65535));
        });
        if enabled != self.mcp.is_some() {
            self.settings.mcp_http_enabled = enabled;
            if enabled {
                self.start_mcp(ctx)
            } else {
                self.stop_mcp()
            }
        }
        if let Some(error) = &self.mcp_error {
            ui.colored_label(Color32::from_rgb(255, 120, 120), error);
        }
        let url =
            self.mcp.as_ref().map_or_else(|| format!("http://127.0.0.1:{}/mcp", self.settings.mcp_port), |b| b.url());
        ui.weak("Connect Claude Code to this window:");
        ui.code(format!("claude mcp add --transport http spry {url}"));
        ui.weak("Or run Spry headless for any agent:");
        ui.code("claude mcp add spry -- spry mcp path/to/sprite.spry");

        if self.settings != before {
            if self.settings.appearance != before.appearance {
                theme::apply(ctx, self.settings.appearance);
            }
            let provider_changed = self.settings.chat_provider != before.chat_provider
                || self.settings.anthropic_api_key != before.anthropic_api_key
                || self.settings.claude_model != before.claude_model
                || self.settings.ollama_url != before.ollama_url
                || self.settings.ollama_model != before.ollama_model;
            if provider_changed {
                let provider = self.settings.provider();
                self.agent.reset(provider);
            }
            self.persist_settings();
        }
    }

    fn new_document_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Width");
            ui.add(egui::DragValue::new(&mut self.new_document.0).range(1..=4096));
            ui.label("Height");
            ui.add(egui::DragValue::new(&mut self.new_document.1).range(1..=4096));
        });
        ui.horizontal_wrapped(|ui| {
            for size in [8u32, 16, 24, 32, 48, 64, 128, 256] {
                if ui.small_button(format!("{size}²")).clicked() {
                    self.new_document = (size, size);
                }
            }
        });
        if ui.button("Create").clicked() {
            let (w, h) = self.new_document;
            self.add_document(Studio::new(w, h));
            self.open_panels.remove(&Panel::NewDocument);
            self.status = format!("New {w}×{h} sprite");
        }
    }

    fn resize_panel(&mut self, ui: &mut egui::Ui) {
        let (old_w, old_h) = {
            let sprite = self.studio().editor.sprite();
            (sprite.width, sprite.height)
        };
        let draft = self.resize.get_or_insert(ResizeDraft { width: old_w, height: old_h, column: 0, row: 0 });
        ui.horizontal(|ui| {
            ui.label("Width");
            ui.add(egui::DragValue::new(&mut draft.width).range(1..=MAX_CANVAS_SIDE));
            ui.label("Height");
            ui.add(egui::DragValue::new(&mut draft.height).range(1..=MAX_CANVAS_SIDE));
        });
        ui.label("Anchor");
        egui::Grid::new("anchor").show(ui, |ui| {
            for row in 0..3 {
                for column in 0..3 {
                    let chosen = (draft.column, draft.row) == (column, row);
                    if ui.selectable_label(chosen, "●").clicked() {
                        (draft.column, draft.row) = (column, row);
                    }
                }
                ui.end_row();
            }
        });
        if ui.button("Resize").clicked() {
            let (offset_x, offset_y) = draft.offsets_from(old_w, old_h);
            let (width, height) = (draft.width, draft.height);
            self.edit(Command::ResizeCanvas { width, height, offset_x, offset_y });
            self.document_mut().view.refit();
            self.open_panels.remove(&Panel::ResizeCanvas);
        }
    }

    // ----- command palette --------------------------------------------------

    fn command_palette(&mut self, ctx: &egui::Context) {
        let entries = palette_entries(self);
        let query = self.palette_query.to_lowercase();
        let matches: Vec<&(String, Action)> =
            entries.iter().filter(|(label, _)| fuzzy_match(&query, &label.to_lowercase())).take(14).collect();
        let mut chosen: Option<Action> = None;
        let mut close = false;
        egui::Area::new(egui::Id::new("command-palette"))
            .anchor(Align2::CENTER_TOP, [0.0, 120.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                theme::glass_frame(self.appearance()).fill(theme::palette(self.appearance()).glass_strong).show(
                    ui,
                    |ui| {
                        ui.set_width(460.0);
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.palette_query)
                                .hint_text("Type an action…")
                                .desired_width(f32::INFINITY),
                        );
                        response.request_focus();
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            close = true;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            chosen = matches.first().map(|(_, action)| action.clone());
                        }
                        ui.separator();
                        for (label, action) in &matches {
                            if ui.add(egui::Button::new(label.as_str()).frame(false)).clicked() {
                                chosen = Some(action.clone());
                            }
                        }
                        if matches.is_empty() {
                            ui.weak("Nothing matches.");
                        }
                    },
                );
            });
        if let Some(action) = chosen {
            self.palette_open = false;
            self.run(ctx, action);
        } else if close {
            self.palette_open = false;
        }
    }
}

/// Every action the palette can run, labelled for searching.
fn palette_entries(app: &App) -> Vec<(String, Action)> {
    let mut entries: Vec<(String, Action)> = vec![
        ("New sprite".into(), Action::New),
        ("Open file".into(), Action::Open),
        ("Save".into(), Action::Save),
        ("Save as".into(), Action::SaveAs),
        ("Close document".into(), Action::Close),
        ("Export PNG".into(), Action::ExportPng),
        ("Export GIF".into(), Action::ExportGif),
        ("Export sprite sheet".into(), Action::ExportSheet),
        ("Export PNG sequence".into(), Action::ExportSequence),
        ("Import sprite sheet".into(), Action::ImportSheet),
        ("Load palette file".into(), Action::LoadPalette),
        ("Save palette file".into(), Action::SavePalette),
        ("Play / pause animation".into(), Action::PlayPause),
        ("Swap primary and secondary colours".into(), Action::SwapColours),
        ("Zoom in".into(), Action::ZoomIn),
        ("Zoom out".into(), Action::ZoomOut),
        ("Zoom to fit".into(), Action::ZoomToFit),
        ("Toggle pixel grid".into(), Action::ToggleGrid),
        ("Toggle onion skin".into(), Action::ToggleOnionSkin),
        ("Toggle tiled preview".into(), Action::ToggleTiledPreview),
        ("Toggle light / dark glass".into(), Action::ToggleAppearance),
        ("Previous frame".into(), Action::PreviousFrame),
        ("Next frame".into(), Action::NextFrame),
    ];
    for panel in [
        Panel::Layers,
        Panel::Timeline,
        Panel::Palette,
        Panel::Preview,
        Panel::Versions,
        Panel::Notes,
        Panel::Assistant,
        Panel::Generate,
        Panel::Export,
        Panel::Settings,
        Panel::ResizeCanvas,
    ] {
        entries.push((format!("Show / hide {}", panel.title()), Action::Toggle(panel)));
    }
    for tool in Tool::ALL {
        entries.push((format!("Tool: {}", tool.name()), Action::UseTool(tool)));
    }
    let primary = app.tools.primary;
    let commands = [
        ("Undo", Command::Undo),
        ("Redo", Command::Redo),
        ("Select all", Command::SelectAll),
        ("Deselect", Command::SelectNone),
        ("Invert selection", Command::InvertSelection),
        ("Copy", Command::Copy),
        ("Cut", Command::Cut),
        ("Paste", Command::Paste { at: None }),
        ("Delete selection", Command::DeleteSelection),
        ("Flip horizontal", Command::FlipHorizontal),
        ("Flip vertical", Command::FlipVertical),
        ("Rotate 90° clockwise", Command::RotateClockwise),
        ("Rotate 90° counter-clockwise", Command::RotateCounterClockwise),
        ("Outline with primary colour", Command::Outline { colour: primary, diagonals: false }),
        ("Thick outline with primary colour", Command::Outline { colour: primary, diagonals: true }),
        ("Drop shadow with primary colour", Command::DropShadow { colour: primary, dx: 1, dy: 1 }),
        ("Snap layer to palette", Command::SnapToPalette { dither: false }),
        ("Snap layer to palette with dither", Command::SnapToPalette { dither: true }),
        ("Extract palette from sprite", Command::ExtractPalette),
        ("Add layer", Command::AddLayer { name: None }),
        ("Duplicate layer", Command::DuplicateLayer),
        ("Merge layer down", Command::MergeDown),
        ("Delete layer", Command::RemoveLayer),
        ("Add frame", Command::AddFrame),
        ("Duplicate frame", Command::DuplicateFrame { as_clone: false }),
        ("Clone frame (linked cels)", Command::DuplicateFrame { as_clone: true }),
        ("Unlink cel", Command::UnlinkCel),
        ("Clear cel", Command::ClearCel),
        ("Delete frame", Command::RemoveFrame),
        ("Commit version", Command::CommitVersion { message: String::new() }),
    ];
    for (label, command) in commands {
        entries.push((label.into(), Action::Edit(command)));
    }
    for palette in Palette::built_ins() {
        entries.push((format!("Palette: {}", palette.name), Action::Edit(Command::SetPalette { palette })));
    }
    entries
}

/// True when every character of `query` appears in `text` in order, so
/// "exgf" finds "Export GIF".
pub fn fuzzy_match(query: &str, text: &str) -> bool {
    let mut remaining = text.chars();
    query.chars().filter(|c| !c.is_whitespace()).all(|wanted| remaining.any(|c| c == wanted))
}

fn time_ago(seconds: u64) -> String {
    match seconds {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

/// What the Layers panel shows for one layer.
#[derive(Clone)]
struct LayerRow {
    name: String,
    visible: bool,
    locked: bool,
    opacity: u8,
    blend: BlendMode,
    depth: usize,
    is_group: bool,
}

impl LayerRow {
    fn of(sprite: &crate::editor::Sprite, index: usize) -> LayerRow {
        let layer = &sprite.layers[index];
        LayerRow {
            name: layer.name.clone(),
            visible: layer.visible,
            locked: layer.locked,
            opacity: layer.opacity,
            blend: layer.blend,
            depth: sprite.depth(index),
            is_group: layer.is_group(),
        }
    }
}

/// Runs `command` on another layer and comes back to the current one.
fn on_layer(index: usize, current: usize, command: Command) -> [Command; 3] {
    [Command::SelectLayer { index }, command, Command::SelectLayer { index: current }]
}

/// Aseprite's cel grid: one row per layer (top layer first), one column
/// per frame. ■ is a cel with art, ◆ a cel linked to other frames (a
/// clone), · an empty cel. Clicking a cell picks that layer and frame.
fn cel_grid(
    ui: &mut egui::Ui,
    sprite: &crate::editor::Sprite,
    current_layer: usize,
    current_frame: usize,
    commands: &mut Vec<Command>,
) {
    egui::Grid::new("cel-grid").spacing([2.0, 2.0]).show(ui, |ui| {
        for layer in (0..sprite.layers.len()).rev() {
            let name: String = sprite.layers[layer].name.chars().take(10).collect();
            ui.label(RichText::new(name).small().strong());
            for frame in 0..sprite.frame_count() {
                let has_art = sprite.cel_image(layer, frame).is_some_and(|image| !image.is_blank());
                let symbol = match (has_art, sprite.is_linked(layer, frame)) {
                    (true, true) => "◆",
                    (true, false) => "■",
                    (false, _) => "·",
                };
                let here = layer == current_layer && frame == current_frame;
                let cell = ui.add_sized([18.0, 18.0], egui::SelectableLabel::new(here, symbol));
                if cell.on_hover_text(format!("{}, frame {}", sprite.layers[layer].name, frame + 1)).clicked() {
                    commands.push(Command::SelectLayer { index: layer });
                    commands.push(Command::SelectFrame { index: frame });
                }
            }
            ui.end_row();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matching_finds_letters_in_order() {
        assert!(fuzzy_match("exgif", "export gif"));
        assert!(fuzzy_match("", "anything"));
        assert!(!fuzzy_match("gifex", "export gif"));
    }

    #[test]
    fn time_ago_reads_naturally() {
        assert_eq!(time_ago(5), "just now");
        assert_eq!(time_ago(125), "2 min ago");
        assert_eq!(time_ago(7200), "2 h ago");
        assert_eq!(time_ago(3 * 86_400), "3 days ago");
    }

    #[test]
    fn opaque_colours_survive_the_trip_through_egui_exactly() {
        let colour = Rgba::rgb(10, 200, 30);
        assert_eq!(from_colour32(to_colour32(colour)), colour);
    }

    #[test]
    fn translucent_colours_survive_within_premultiplied_rounding() {
        let colour = Rgba::rgba(10, 200, 30, 128);
        let back = from_colour32(to_colour32(colour));
        let close = |a: u8, b: u8| a.abs_diff(b) <= 2;
        assert!(close(back.r(), 10) && close(back.g(), 200) && close(back.b(), 30) && back.a() == 128, "{back}");
    }
}

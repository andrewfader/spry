//! The Spry window: open documents, floating glass panels, and the plumbing
//! that connects them to MCP and AI workers.
//!
//! Every menu item, shortcut and command-palette entry is an `Action`, so
//! all of them are found, named and run in one place. Panels live in
//! `panels.rs` and read and write the state here.

use super::canvas::{self, CanvasInput, CanvasRequest, CanvasView};
use super::file_browser::{BrowserOutcome, FileBrowser, Purpose};
use super::settings::Settings;
use super::textures::TextureCache;
use super::theme::{self, Appearance};
use super::tools::{Tool, ToolSettings};
use super::workers::{AgentWorker, DiffusionJob};
use crate::editor::playback;
use crate::editor::Command;
use crate::io::{self, SheetOptions};
use crate::mcp::http::HttpBridge;
use crate::mcp::Workspace;
use crate::studio::Studio;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct OpenDocument {
    pub id: u64,
    pub studio: Studio,
    pub view: CanvasView,
}

impl OpenDocument {
    pub fn texture_prefix(&self) -> String {
        format!("doc{}:", self.id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    Layers,
    Timeline,
    Palette,
    Preview,
    Versions,
    Notes,
    Assistant,
    Generate,
    Export,
    Settings,
    NewDocument,
    ResizeCanvas,
}

impl Panel {
    pub fn title(self) -> &'static str {
        match self {
            Panel::Layers => "Layers",
            Panel::Timeline => "Timeline",
            Panel::Palette => "Palette",
            Panel::Preview => "Preview",
            Panel::Versions => "Versions",
            Panel::Notes => "Notes",
            Panel::Assistant => "AI Assistant",
            Panel::Generate => "Generate",
            Panel::Export => "Export",
            Panel::Settings => "Settings",
            Panel::NewDocument => "New sprite",
            Panel::ResizeCanvas => "Canvas size",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    New,
    Open,
    Save,
    SaveAs,
    Close,
    ExportPng,
    ExportGif,
    ExportSheet,
    ExportSequence,
    ImportSheet,
    LoadPalette,
    SavePalette,
    Edit(Command),
    Toggle(Panel),
    UseTool(Tool),
    SwapColours,
    PlayPause,
    ZoomIn,
    ZoomOut,
    ZoomToFit,
    ToggleGrid,
    ToggleOnionSkin,
    OnionSkin { before: usize, after: usize, opacity: u8 },
    ToggleTiledPreview,
    ToggleAppearance,
    PreviousFrame,
    NextFrame,
    CommandPalette,
}

/// The new size, and which of the nine anchor cells (column and row, 0 to
/// 2) the old picture keeps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizeDraft {
    pub width: u32,
    pub height: u32,
    pub column: i32,
    pub row: i32,
}

impl ResizeDraft {
    /// Where the old picture lands in the new canvas. Worked out at the
    /// moment of resizing, so it always matches the final width and height.
    pub fn offsets_from(&self, old_width: u32, old_height: u32) -> (i32, i32) {
        let spare_x = self.width as i32 - old_width as i32;
        let spare_y = self.height as i32 - old_height as i32;
        (spare_x * self.column / 2, spare_y * self.row / 2)
    }
}

/// What's waiting on an answer about unsaved work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closing {
    Tab(u64),
    Window,
}

pub struct App {
    pub documents: Vec<OpenDocument>,
    pub active: usize,
    next_id: u64,
    pub settings: Settings,
    pub tools: ToolSettings,
    pub textures: TextureCache,
    pub open_panels: std::collections::HashSet<Panel>,
    pub browser: Option<FileBrowser>,
    pub status: String,
    pub playing: Option<Instant>,
    pub preview_tag: Option<String>,
    pub mcp: Option<HttpBridge>,
    pub mcp_error: Option<String>,
    pub agent: AgentWorker,
    /// The document the assistant's current turn is working on, if a turn
    /// is under way.
    pub agent_turn: Option<u64>,
    pub chat_draft: String,
    pub diffusion: Option<DiffusionJob>,
    pub generate: super::panels::GenerateForm,
    pub sheet_options: SheetOptions,
    pub export_scale: u32,
    pub new_document: (u32, u32),
    /// The Canvas Size dialog's choices while it is open.
    pub resize: Option<ResizeDraft>,
    pub layer_name_draft: Option<(usize, String)>,
    pub palette_query: String,
    pub palette_open: bool,
    pub pending_pin: Option<(i32, i32, String)>,
    pub version_message: String,
    pub sheet_cell: (u32, u32),
    pub closing: Option<Closing>,
    /// The version the Versions panel is comparing with now.
    pub selected_version: Option<u32>,
    /// The last pixel-diff count, keyed by (document, version, revision).
    pub version_diff: Option<((u64, u32, u64), usize)>,
    /// Set once the user has chosen to quit and lose unsaved changes.
    quit_confirmed: bool,
}

impl App {
    pub fn new(ctx: &egui::Context, initial: Option<PathBuf>) -> App {
        let settings = Settings::load();
        theme::apply(ctx, settings.appearance);
        let agent = AgentWorker::new(settings.provider());
        let mut app = App {
            documents: Vec::new(),
            active: 0,
            next_id: 1,
            settings,
            tools: ToolSettings::default(),
            textures: TextureCache::default(),
            open_panels: [Panel::Layers, Panel::Timeline, Panel::Palette, Panel::Preview].into_iter().collect(),
            browser: None,
            status: "Welcome to Spry. Press Ctrl+K to find any action.".into(),
            playing: None,
            preview_tag: None,
            mcp: None,
            mcp_error: None,
            agent,
            agent_turn: None,
            chat_draft: String::new(),
            diffusion: None,
            generate: super::panels::GenerateForm::default(),
            sheet_options: SheetOptions::default(),
            export_scale: 1,
            new_document: (32, 32),
            resize: None,
            layer_name_draft: None,
            palette_query: String::new(),
            palette_open: false,
            pending_pin: None,
            version_message: String::new(),
            sheet_cell: (16, 16),
            closing: None,
            selected_version: None,
            version_diff: None,
            quit_confirmed: false,
        };
        match initial.map(|path| Studio::open(&path).map(|studio| (studio, path))) {
            Some(Ok((studio, path))) => {
                app.settings.remember_file(&path);
                app.add_document(studio);
            }
            Some(Err(error)) => {
                app.status = format!("Couldn't open that file: {error:#}");
                app.add_document(Studio::new(32, 32));
            }
            None => app.add_document(Studio::new(32, 32)),
        }
        if app.settings.mcp_http_enabled {
            app.start_mcp(ctx);
        }
        app
    }

    pub fn add_document(&mut self, studio: Studio) {
        self.documents.push(OpenDocument { id: self.next_id, studio, view: CanvasView::default() });
        self.next_id += 1;
        self.active = self.documents.len() - 1;
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.documents.iter().position(|document| document.id == id)
    }

    pub fn document(&self) -> &OpenDocument {
        &self.documents[self.active]
    }

    pub fn document_mut(&mut self) -> &mut OpenDocument {
        &mut self.documents[self.active]
    }

    pub fn studio(&self) -> &Studio {
        &self.document().studio
    }

    pub fn studio_mut(&mut self) -> &mut Studio {
        &mut self.document_mut().studio
    }

    pub fn appearance(&self) -> Appearance {
        self.settings.appearance
    }

    pub fn glass(&self) -> egui::Frame {
        theme::glass_frame(self.appearance())
    }

    pub fn is_open(&self, panel: Panel) -> bool {
        self.open_panels.contains(&panel)
    }

    pub fn toggle(&mut self, panel: Panel) {
        if !self.open_panels.remove(&panel) {
            self.open_panels.insert(panel);
        }
    }

    // ----- actions ------------------------------------------------------

    pub fn run(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::New => {
                let (w, h) = (self.studio().editor.sprite().width, self.studio().editor.sprite().height);
                self.new_document = (w, h);
                self.open_panels.insert(Panel::NewDocument);
            }
            Action::Open => self.browse(Purpose::Open, ""),
            Action::Save => match self.studio().path.clone() {
                Some(_) => self.save_to(None),
                None => self.browse(Purpose::SaveDocument, &format!("{}.spry", self.studio().title())),
            },
            Action::SaveAs => self.run_save_as(),
            Action::Close => self.close_active(),
            Action::ExportPng => self.browse(Purpose::ExportPng, &format!("{}.png", self.studio().title())),
            Action::ExportGif => self.browse(Purpose::ExportGif, &format!("{}.gif", self.studio().title())),
            Action::ExportSheet => self.browse(Purpose::ExportSheet, &format!("{}-sheet.png", self.studio().title())),
            Action::ExportSequence => self.browse(Purpose::ExportSequence, ""),
            Action::ImportSheet => self.browse(Purpose::ImportSheet, ""),
            Action::LoadPalette => self.browse(Purpose::LoadPalette, ""),
            Action::SavePalette => {
                self.browse(Purpose::SavePalette, &format!("{}.hex", self.studio().editor.sprite().palette.name))
            }
            Action::Edit(command) => self.edit(command),
            Action::Toggle(panel) => self.toggle(panel),
            Action::UseTool(tool) => {
                self.tools.tool = tool;
                self.status = tool.name().into();
            }
            Action::SwapColours => self.tools.swap_colours(),
            Action::PlayPause => self.playing = if self.playing.is_some() { None } else { Some(Instant::now()) },
            Action::ZoomIn => self.document_mut().view.zoom_in(),
            Action::ZoomOut => self.document_mut().view.zoom_out(),
            Action::ZoomToFit => self.document_mut().view.refit(),
            Action::ToggleGrid => flip(&mut self.document_mut().view.show_grid),
            Action::ToggleOnionSkin => flip(&mut self.document_mut().view.onion_skin),
            Action::OnionSkin { before, after, opacity } => {
                let view = &mut self.document_mut().view;
                (view.onion_before, view.onion_after, view.onion_opacity) = (before, after, opacity);
            }
            Action::ToggleTiledPreview => flip(&mut self.document_mut().view.tiled),
            Action::ToggleAppearance => {
                self.settings.appearance = match self.settings.appearance {
                    Appearance::Dark => Appearance::Light,
                    Appearance::Light => Appearance::Dark,
                };
                theme::apply(ctx, self.settings.appearance);
                self.persist_settings();
            }
            Action::PreviousFrame | Action::NextFrame => {
                let editor = &mut self.studio_mut().editor;
                let count = editor.sprite().frame_count();
                let step = if action == Action::NextFrame { 1 } else { count - 1 };
                let target = (editor.frame + step) % count;
                let _ = editor.execute(Command::SelectFrame { index: target });
            }
            Action::CommandPalette => {
                self.palette_open = !self.palette_open;
                self.palette_query.clear();
            }
        }
    }

    pub fn edit(&mut self, command: Command) {
        match self.studio_mut().editor.execute(command) {
            Ok(summary) => self.status = capitalise(&summary),
            Err(error) => self.status = capitalise(&error.to_string()),
        }
    }

    fn browse(&mut self, purpose: Purpose, suggested: &str) {
        let directory = self
            .studio()
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| self.settings.last_directory.clone())
            .or_else(dirs::picture_dir)
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        self.browser = Some(FileBrowser::new(purpose, directory, suggested));
    }

    /// Closes the front tab, first asking about unsaved changes.
    fn close_active(&mut self) {
        let document = self.document();
        if document.studio.editor.has_unsaved_changes() {
            self.closing = Some(Closing::Tab(document.id));
        } else {
            self.close_tab(document.id);
        }
    }

    fn close_tab(&mut self, id: u64) {
        let Some(index) = self.index_of(id) else { return };
        let prefix = self.documents[index].texture_prefix();
        self.textures.forget(&prefix);
        self.documents.remove(index);
        if self.documents.is_empty() {
            self.add_document(Studio::new(32, 32));
        }
        self.active = self.active.min(self.documents.len() - 1);
    }

    fn unsaved_titles(&self) -> Vec<String> {
        self.documents
            .iter()
            .filter(|document| document.studio.editor.has_unsaved_changes())
            .map(|document| document.studio.title())
            .collect()
    }

    /// Stops the window closing while unsaved work would be lost, and asks
    /// instead.
    fn guard_window_close(&mut self, ctx: &egui::Context) {
        let wants_to_close = ctx.input(|input| input.viewport().close_requested());
        if wants_to_close && !self.quit_confirmed && !self.unsaved_titles().is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = Some(Closing::Window);
        }
    }

    fn show_close_dialog(&mut self, ctx: &egui::Context) {
        let Some(closing) = self.closing else { return };
        let question = match closing {
            Closing::Tab(id) => match self.index_of(id) {
                Some(index) => format!("Save changes to \"{}\" before closing?", self.documents[index].studio.title()),
                None => {
                    self.closing = None;
                    return;
                }
            },
            Closing::Window => format!("Unsaved changes in: {}", self.unsaved_titles().join(", ")),
        };
        let mut choice = None;
        egui::Window::new("Unsaved changes")
            .frame(self.glass())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(question);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if closing != Closing::Window && ui.button("Save").clicked() {
                        choice = Some(CloseChoice::Save);
                    }
                    if ui.button("Discard changes").clicked() {
                        choice = Some(CloseChoice::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(CloseChoice::Cancel);
                    }
                });
            });
        let Some(choice) = choice else { return };
        self.closing = None;
        match (closing, choice) {
            (_, CloseChoice::Cancel) => {}
            (Closing::Tab(id), CloseChoice::Discard) => self.close_tab(id),
            (Closing::Tab(id), CloseChoice::Save) => self.save_then_close(id),
            (Closing::Window, _) => {
                self.quit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Saves a tab that has a file and closes it. A tab that has never been
    /// saved opens Save As instead, and stays open.
    fn save_then_close(&mut self, id: u64) {
        let Some(index) = self.index_of(id) else { return };
        self.active = index;
        if self.studio().path.is_none() {
            self.run_save_as();
            return;
        }
        self.save_to(None);
        if !self.studio().editor.has_unsaved_changes() {
            self.close_tab(id);
        }
    }

    fn run_save_as(&mut self) {
        self.browse(Purpose::SaveDocument, &format!("{}.spry", self.studio().title()));
    }

    fn save_to(&mut self, path: Option<&Path>) {
        match self.studio_mut().save(path) {
            Ok(saved) => {
                self.status = format!("Saved {} (version recorded)", saved.display());
                self.settings.remember_file(&saved);
                self.persist_settings();
            }
            Err(error) => self.status = format!("Couldn't save: {error:#}"),
        }
    }

    pub fn persist_settings(&mut self) {
        if let Err(error) = self.settings.save() {
            self.status = format!("Couldn't save settings: {error}");
        }
    }

    /// Opens a file in a new tab, or switches to it if it's already open.
    pub fn open_path(&mut self, path: &Path) {
        if let Some(index) = self.documents.iter().position(|d| d.studio.path.as_deref() == Some(path)) {
            self.active = index;
            return;
        }
        match Studio::open(path) {
            Ok(studio) => {
                self.settings.remember_file(path);
                self.persist_settings();
                self.status = match studio.notices.as_slice() {
                    [] => format!("Opened {}", path.display()),
                    notices => format!("Opened {}. Note: {}", path.display(), notices.join("; ")),
                };
                self.add_document(studio);
            }
            Err(error) => self.status = format!("Couldn't open {}: {error:#}", path.display()),
        }
    }

    fn finish_browsing(&mut self, purpose: Purpose, path: PathBuf) {
        let title = self.studio().title();
        let sprite = self.studio().editor.sprite().clone();
        let frame = self.studio().editor.frame;
        let scale = self.export_scale;
        let report = |result: anyhow::Result<String>| match result {
            Ok(message) => message,
            Err(error) => format!("{error:#}"),
        };
        self.status = match purpose {
            Purpose::Open => {
                self.open_path(&path);
                return;
            }
            Purpose::SaveDocument => {
                self.save_to(Some(&path));
                return;
            }
            Purpose::ExportPng => {
                report(io::export_png(&path, &sprite, frame, scale).map(|_| format!("Exported {}", path.display())))
            }
            Purpose::ExportGif => report(
                io::export_gif(&path, &sprite, self.preview_tag.as_deref(), scale)
                    .map(|_| format!("Exported {}", path.display())),
            ),
            Purpose::ExportSheet => {
                let options = SheetOptions { scale, ..self.sheet_options.clone() };
                report(io::export_sheet(&path, &sprite, &title, &options).map(|sheet| {
                    format!(
                        "Exported {} ({}×{}, {} distinct images) with its .json",
                        path.display(),
                        sheet.image.width(),
                        sheet.image.height(),
                        sheet.distinct_images()
                    )
                }))
            }
            Purpose::ExportSequence => report(
                io::export_png_sequence(&path, &title, &sprite, scale)
                    .map(|files| format!("Exported {} frames into {}", files.len(), path.display())),
            ),
            Purpose::ImportSheet => match io::import_sheet(&path, self.sheet_cell) {
                Ok((document, (w, h))) => {
                    let frames = document.sprite.frame_count();
                    self.add_document(Studio::from_document(document));
                    format!("Imported {frames} frames of {w}×{h} from {}", path.display())
                }
                Err(error) => format!("Couldn't import the sheet: {error:#}"),
            },
            Purpose::LoadPalette => match io::load_palette(&path) {
                Ok(palette) => {
                    self.edit(Command::SetPalette { palette });
                    return;
                }
                Err(error) => format!("Couldn't load the palette: {error:#}"),
            },
            Purpose::SavePalette => {
                report(io::save_palette(&path, &sprite.palette).map(|_| format!("Saved palette to {}", path.display())))
            }
        };
    }

    // ----- MCP ----------------------------------------------------------

    pub fn start_mcp(&mut self, ctx: &egui::Context) {
        let wake = ctx.clone();
        match HttpBridge::start(self.settings.mcp_port, move || wake.request_repaint()) {
            Ok(bridge) => {
                self.status = format!("MCP server listening at {}", bridge.url());
                self.mcp = Some(bridge);
                self.mcp_error = None;
            }
            Err(error) => self.mcp_error = Some(format!("{error:#}")),
        }
    }

    pub fn stop_mcp(&mut self) {
        self.mcp = None;
    }

    /// External MCP clients work on whichever document is in front.
    fn answer_mcp(&mut self) {
        let Some(bridge) = &self.mcp else { return };
        let pending: Vec<_> = bridge.messages.try_iter().collect();
        for message in pending {
            let target = self.document().id;
            let reply = crate::mcp::handle(&mut Tabs { app: self, target, in_undo_group: false }, &message.message);
            message.answer(reply);
        }
    }

    // ----- AI -----------------------------------------------------------

    pub fn send_chat(&mut self, ctx: &egui::Context) {
        let text = std::mem::take(&mut self.chat_draft);
        let editor = &self.studio().editor;
        let sprite = editor.sprite();
        let scale = (512 / sprite.width.max(sprite.height)).clamp(1, 16);
        let canvas = io::encode_png(&editor.composite_for_display(editor.frame).scaled_up(scale)).ok();
        self.studio_mut().editor.begin_undo_group();
        self.agent_turn = Some(self.document().id);
        let wake = ctx.clone();
        self.agent.send(text, canvas, move || wake.request_repaint());
    }

    /// The assistant's tools work on the document its turn started on, even
    /// if you switch tabs while it thinks.
    fn run_agent_tools(&mut self) {
        for request in self.agent.drain() {
            let Some(target) = self.agent_turn.filter(|&id| self.index_of(id).is_some()) else {
                request.answer(Err("the document this turn was working on has been closed".into()));
                continue;
            };
            let mut tabs = Tabs { app: self, target, in_undo_group: true };
            let result = crate::mcp::tools::call(&mut tabs, &request.name, &request.arguments);
            self.agent_turn = Some(tabs.target);
            request.answer(result);
        }
        if let Some(target) = self.agent_turn.filter(|_| self.agent.is_idle()) {
            if let Some(index) = self.index_of(target) {
                self.documents[index].studio.editor.end_undo_group();
            }
            self.agent_turn = None;
            self.status = "The assistant finished; Ctrl+Z undoes its whole turn".into();
        }
    }

    fn collect_generation(&mut self) {
        let Some(job) = &self.diffusion else { return };
        let Some(result) = job.poll() else { return };
        let (prompt, target) = (job.prompt.clone(), job.document);
        self.diffusion = None;
        let Some(index) = self.index_of(target) else {
            self.status = "A generation finished, but its document has been closed".into();
            return;
        };
        match result {
            Ok(image) => {
                let name: String = format!("✨ {}", prompt.chars().take(24).collect::<String>());
                let outcome = self.documents[index].studio.editor.execute(Command::ImportLayer { name, image });
                self.status = match outcome {
                    Ok(summary) => capitalise(&summary),
                    Err(error) => capitalise(&error.to_string()),
                };
            }
            Err(error) => self.status = format!("Generation failed: {error}"),
        }
    }

    // ----- keyboard -------------------------------------------------------

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let command = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let command_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let bindings: Vec<(KeyboardShortcut, Action)> = vec![
            (command_shift(Key::Z), Action::Edit(Command::Redo)),
            (command(Key::Z), Action::Edit(Command::Undo)),
            (command(Key::Y), Action::Edit(Command::Redo)),
            (command(Key::K), Action::CommandPalette),
            (command_shift(Key::S), Action::SaveAs),
            (command(Key::S), Action::Save),
            (command(Key::O), Action::Open),
            (command(Key::N), Action::New),
            (command(Key::W), Action::Close),
            (command(Key::C), Action::Edit(Command::Copy)),
            (command(Key::X), Action::Edit(Command::Cut)),
            (command(Key::V), Action::Edit(Command::Paste { at: None })),
            (command(Key::A), Action::Edit(Command::SelectAll)),
            (command(Key::D), Action::Edit(Command::SelectNone)),
            (command_shift(Key::I), Action::Edit(Command::InvertSelection)),
            (command(Key::E), Action::Toggle(Panel::Export)),
            (command(Key::Plus), Action::ZoomIn),
            (command(Key::Equals), Action::ZoomIn),
            (command(Key::Minus), Action::ZoomOut),
            (command(Key::Num0), Action::ZoomToFit),
            (KeyboardShortcut::new(Modifiers::ALT, Key::OpenBracket), Action::PreviousFrame),
            (KeyboardShortcut::new(Modifiers::ALT, Key::CloseBracket), Action::NextFrame),
        ];
        let mut triggered = Vec::new();
        ctx.input_mut(|input| {
            for (shortcut, action) in &bindings {
                if input.consume_shortcut(shortcut) {
                    triggered.push(action.clone());
                }
            }
        });

        // Single keys only when no text field has focus.
        if !ctx.wants_keyboard_input() {
            ctx.input(|input| {
                // Whole-cel transforms are easy to hit by accident, so like
                // Aseprite they need Shift.
                if input.modifiers.matches_exact(Modifiers::SHIFT) {
                    let shifted = [
                        (Key::H, Action::Edit(Command::FlipHorizontal)),
                        (Key::V, Action::Edit(Command::FlipVertical)),
                        (Key::R, Action::Edit(Command::RotateClockwise)),
                    ];
                    for (key, action) in shifted {
                        if input.key_pressed(key) {
                            triggered.push(action);
                        }
                    }
                }
                if input.modifiers.any() {
                    return;
                }
                for tool in Tool::ALL {
                    if tool.shortcut().is_some_and(|key| input.key_pressed(key)) {
                        triggered.push(Action::UseTool(tool));
                    }
                }
                let singles = [
                    (Key::X, Action::SwapColours),
                    (Key::Enter, Action::PlayPause),
                    (Key::Delete, Action::Edit(Command::DeleteSelection)),
                    (Key::Backspace, Action::Edit(Command::DeleteSelection)),
                    (Key::Escape, Action::Edit(Command::Anchor)),
                    (Key::Comma, Action::PreviousFrame),
                    (Key::Period, Action::NextFrame),
                    (Key::Tab, Action::ToggleOnionSkin),
                ];
                for (key, action) in singles {
                    if input.key_pressed(key) {
                        triggered.push(action);
                    }
                }
                let nudges =
                    [(Key::ArrowLeft, -1, 0), (Key::ArrowRight, 1, 0), (Key::ArrowUp, 0, -1), (Key::ArrowDown, 0, 1)];
                for (key, dx, dy) in nudges {
                    if input.key_pressed(key) {
                        triggered.push(Action::Edit(Command::MoveFloating { dx, dy }));
                    }
                }
            });
        }
        for action in triggered {
            // Escape and arrows only mean something with a selection.
            let quiet = matches!(action, Action::Edit(Command::Anchor) | Action::Edit(Command::MoveFloating { .. }));
            let has_selection = self.studio().editor.selection.is_some() || self.studio().editor.floating.is_some();
            if quiet && !has_selection {
                continue;
            }
            self.run(ctx, action);
        }
    }

    // ----- frame ----------------------------------------------------------

    fn animate(&mut self, ctx: &egui::Context) {
        let Some(started) = self.playing else { return };
        let elapsed = started.elapsed().as_millis() as u64;
        let tag_name = self.preview_tag.clone();
        let editor = &mut self.documents[self.active].studio.editor;
        let tag = tag_name.as_deref().and_then(|name| editor.sprite().tags.iter().find(|t| t.name == name));
        let frame = playback::frame_at(editor.sprite(), tag, elapsed);
        let wait = playback::ms_until_next_frame(editor.sprite(), tag, elapsed);
        if frame != editor.frame {
            let _ = editor.execute(Command::SelectFrame { index: frame });
        }
        if let Some(wait) = wait {
            ctx.request_repaint_after(std::time::Duration::from_millis(wait));
        }
    }

    pub fn window_title(&self) -> String {
        let studio = self.studio();
        let dirty = if studio.editor.has_unsaved_changes() { " •" } else { "" };
        format!("{}{dirty} — Spry", studio.title())
    }
}

#[derive(Clone, Copy)]
enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

/// The app's tabs as an MCP workspace. Tools work on the `target`
/// document, and a document an AI creates or opens arrives in a new tab
/// instead of replacing the one you were working on.
struct Tabs<'a> {
    app: &'a mut App,
    target: u64,
    /// Whether the target is inside an AI turn's undo group, which then
    /// moves with it to the new document.
    in_undo_group: bool,
}

impl Workspace for Tabs<'_> {
    fn studio(&mut self) -> &mut Studio {
        // Callers check the target is still open before each call.
        let index = self.app.index_of(self.target).unwrap_or(self.app.active);
        &mut self.app.documents[index].studio
    }

    fn open(&mut self, studio: Studio) {
        if self.in_undo_group {
            self.studio().editor.end_undo_group();
        }
        self.app.add_document(studio);
        self.target = self.app.document().id;
        if self.in_undo_group {
            self.studio().editor.begin_undo_group();
        }
    }
}

fn flip(flag: &mut bool) {
    *flag = !*flag;
}

pub fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().collect::<String>() + chars.as_str())
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.guard_window_close(ctx);
        self.answer_mcp();
        self.run_agent_tools();
        self.collect_generation();
        self.animate(ctx);
        self.handle_shortcuts(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));

        let appearance = self.appearance();
        let time = ctx.input(|i| i.time);
        theme::paint_backdrop(&ctx.layer_painter(egui::LayerId::background()), ctx.screen_rect(), appearance, time);

        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let index = self.active;
            let prefix = self.documents[index].texture_prefix();
            let document = &mut self.documents[index];
            let requests = canvas::show(
                ui,
                CanvasInput {
                    studio: &mut document.studio,
                    view: &mut document.view,
                    settings: &mut self.tools,
                    textures: &mut self.textures,
                    appearance,
                    texture_prefix: prefix,
                },
            );
            for request in requests {
                match request {
                    CanvasRequest::Status(text) => self.status = capitalise(&text),
                    CanvasRequest::PinNoteAt((x, y)) => self.pending_pin = Some((x, y, String::new())),
                }
            }
        });

        self.show_panels(ctx);
        self.show_close_dialog(ctx);
        if !self.is_open(Panel::ResizeCanvas) {
            self.resize = None;
        }

        if let Some(browser) = &mut self.browser {
            let purpose = browser.purpose;
            match browser.show(ctx, theme::glass_frame(appearance)) {
                BrowserOutcome::Open => {}
                BrowserOutcome::Cancelled => self.browser = None,
                BrowserOutcome::Chosen(path) => {
                    self.browser = None;
                    self.finish_browsing(purpose, path);
                }
            }
        }
        // Workers wake the UI when they finish; this only keeps spinners
        // turning, so a slow tick is plenty on a machine doing other work.
        if self.agent.busy || self.diffusion.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_settings();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_centred_anchor_splits_the_new_space_evenly() {
        let draft = ResizeDraft { width: 48, height: 16, column: 1, row: 1 };
        assert_eq!(draft.offsets_from(32, 32), (8, -8));
    }

    #[test]
    fn the_anchor_follows_a_size_typed_after_it_was_chosen() {
        let mut draft = ResizeDraft { width: 32, height: 32, column: 2, row: 2 };
        draft.width = 40;
        assert_eq!(draft.offsets_from(32, 32), (8, 0));
    }

    #[test]
    fn status_messages_start_with_a_capital() {
        assert_eq!(capitalise("drew a line"), "Drew a line");
        assert_eq!(capitalise(""), "");
    }
}

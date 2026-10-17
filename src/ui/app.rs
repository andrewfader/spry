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
use crate::editor::{Command, Palette};
use crate::io::{self, SheetOptions};
use crate::mcp::http::HttpBridge;
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
    ToggleTiledPreview,
    ToggleAppearance,
    PreviousFrame,
    NextFrame,
    CommandPalette,
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
    pub agent_turn_open: bool,
    pub chat_draft: String,
    pub diffusion: Option<DiffusionJob>,
    pub generate: super::panels::GenerateForm,
    pub sheet_options: SheetOptions,
    pub export_scale: u32,
    pub new_document: (u32, u32),
    pub resize: (u32, u32, i32, i32),
    pub palette_query: String,
    pub palette_open: bool,
    pub pending_pin: Option<(i32, i32, String)>,
    pub version_message: String,
    pub sheet_cell: (u32, u32),
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
            agent_turn_open: false,
            chat_draft: String::new(),
            diffusion: None,
            generate: super::panels::GenerateForm::default(),
            sheet_options: SheetOptions::default(),
            export_scale: 1,
            new_document: (32, 32),
            resize: (32, 32, 0, 0),
            palette_query: String::new(),
            palette_open: false,
            pending_pin: None,
            version_message: String::new(),
            sheet_cell: (16, 16),
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
            Action::SaveAs => self.browse(Purpose::SaveDocument, &format!("{}.spry", self.studio().title())),
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

    fn close_active(&mut self) {
        let prefix = self.document().texture_prefix();
        self.textures.forget(&prefix);
        self.documents.remove(self.active);
        if self.documents.is_empty() {
            self.add_document(Studio::new(32, 32));
        }
        self.active = self.active.min(self.documents.len() - 1);
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
                self.add_document(studio);
                self.status = format!("Opened {}", path.display());
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
            Purpose::ImportSheet => {
                let imported = std::fs::read(&path)
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| io::decode_image(&bytes))
                    .and_then(|sheet| {
                        let (w, h) = crate::io::sheet::detect_grid(&sheet).unwrap_or(self.sheet_cell);
                        io::document_from_sheet(&sheet, w, h).map(|doc| (doc, w, h))
                    });
                match imported {
                    Ok((document, w, h)) => {
                        let frames = document.sprite.frame_count();
                        self.add_document(Studio::from_document(document));
                        format!("Imported {frames} frames of {w}×{h} from {}", path.display())
                    }
                    Err(error) => format!("Couldn't import the sheet: {error:#}"),
                }
            }
            Purpose::LoadPalette => {
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Palette").to_string();
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|text| Palette::parse(&name, &text))
                {
                    Ok(palette) => {
                        self.edit(Command::SetPalette { palette });
                        return;
                    }
                    Err(error) => format!("Couldn't load the palette: {error}"),
                }
            }
            Purpose::SavePalette => {
                let palette = &sprite.palette;
                let is_gpl = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gpl"));
                let text = if is_gpl { palette.to_gpl_file() } else { palette.to_hex_file() };
                report(
                    std::fs::write(&path, text)
                        .map(|_| format!("Saved palette to {}", path.display()))
                        .map_err(Into::into),
                )
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

    fn answer_mcp(&mut self) {
        let Some(bridge) = &self.mcp else { return };
        let pending: Vec<_> = bridge.messages.try_iter().collect();
        for message in pending {
            let reply = crate::mcp::handle(&mut self.documents[self.active].studio, &message.message);
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
        self.agent_turn_open = true;
        let wake = ctx.clone();
        self.agent.send(text, canvas, move || wake.request_repaint());
    }

    fn run_agent_tools(&mut self) {
        for request in self.agent.drain() {
            let result =
                crate::mcp::tools::call(&mut self.documents[self.active].studio, &request.name, &request.arguments);
            request.answer(result);
        }
        if self.agent_turn_open && self.agent.is_idle() {
            self.studio_mut().editor.end_undo_group();
            self.agent_turn_open = false;
            self.status = "The assistant finished; Ctrl+Z undoes its whole turn".into();
        }
    }

    fn collect_generation(&mut self) {
        let Some(job) = &self.diffusion else { return };
        let Some(result) = job.poll() else { return };
        let prompt = job.prompt.clone();
        self.diffusion = None;
        match result {
            Ok(image) => {
                let name: String = format!("✨ {}", prompt.chars().take(24).collect::<String>());
                self.edit(Command::ImportLayer { name, image });
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
                    (Key::H, Action::Edit(Command::FlipHorizontal)),
                    (Key::R, Action::Edit(Command::RotateClockwise)),
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
        if frame != editor.frame {
            let _ = editor.execute(Command::SelectFrame { index: frame });
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(10));
    }

    pub fn window_title(&self) -> String {
        let studio = self.studio();
        let dirty = if studio.editor.has_unsaved_changes() { " •" } else { "" };
        format!("{}{dirty} — Spry", studio.title())
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
        if ctx.input(|i| i.pointer.any_down()) || self.agent.busy || self.diffusion.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
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
    fn status_messages_start_with_a_capital() {
        assert_eq!(capitalise("drew a line"), "Drew a line");
        assert_eq!(capitalise(""), "");
    }
}

//! A small glass file browser for opening, saving and exporting. It works
//! the same on every platform and needs no native dialog libraries.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Open,
    SaveDocument,
    ExportPng,
    ExportGif,
    ExportSheet,
    ExportSequence,
    ImportSheet,
    LoadPalette,
    SavePalette,
}

impl Purpose {
    pub fn title(self) -> &'static str {
        match self {
            Purpose::Open => "Open",
            Purpose::SaveDocument => "Save as",
            Purpose::ExportPng => "Export PNG",
            Purpose::ExportGif => "Export GIF",
            Purpose::ExportSheet => "Export sprite sheet",
            Purpose::ExportSequence => "Export PNG sequence (choose a folder)",
            Purpose::ImportSheet => "Import sprite sheet",
            Purpose::LoadPalette => "Load palette",
            Purpose::SavePalette => "Save palette",
        }
    }

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Purpose::Open => &["spry", "png", "gif"],
            Purpose::SaveDocument => &["spry"],
            Purpose::ExportPng | Purpose::ExportSheet | Purpose::ImportSheet => &["png"],
            Purpose::ExportGif => &["gif"],
            Purpose::ExportSequence => &[],
            Purpose::LoadPalette => &["hex", "gpl"],
            Purpose::SavePalette => &["hex", "gpl"],
        }
    }

    pub fn is_saving(self) -> bool {
        !matches!(self, Purpose::Open | Purpose::ImportSheet | Purpose::LoadPalette)
    }
}

pub struct FileBrowser {
    pub purpose: Purpose,
    pub directory: PathBuf,
    pub file_name: String,
    entries: Vec<(String, bool)>,
    error: Option<String>,
}

pub enum BrowserOutcome {
    Open,
    Chosen(PathBuf),
    Cancelled,
}

impl FileBrowser {
    pub fn new(purpose: Purpose, directory: PathBuf, suggested_name: &str) -> FileBrowser {
        let mut browser =
            FileBrowser { purpose, directory, file_name: suggested_name.to_string(), entries: Vec::new(), error: None };
        browser.refresh();
        browser
    }

    fn refresh(&mut self) {
        self.entries = list_directory(&self.directory, self.purpose.extensions());
    }

    pub fn show(&mut self, ctx: &egui::Context, frame: egui::Frame) -> BrowserOutcome {
        let mut outcome = BrowserOutcome::Open;
        let mut navigate_to: Option<PathBuf> = None;
        egui::Window::new(self.purpose.title())
            .frame(frame)
            .collapsible(false)
            .resizable(true)
            .default_size([520.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("⬆").on_hover_text("Parent folder").clicked() {
                        navigate_to = self.directory.parent().map(Path::to_path_buf);
                    }
                    if ui.button("🏠").on_hover_text("Home").clicked() {
                        navigate_to = dirs::home_dir();
                    }
                    let mut typed = self.directory.display().to_string();
                    if ui.add(egui::TextEdit::singleline(&mut typed).desired_width(f32::INFINITY)).changed() {
                        let candidate = PathBuf::from(&typed);
                        if candidate.is_dir() {
                            navigate_to = Some(candidate);
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, false]).show(ui, |ui| {
                    for (name, is_dir) in &self.entries {
                        let label = if *is_dir { format!("📁 {name}") } else { format!("🖼 {name}") };
                        let response = ui.selectable_label(!is_dir && *name == self.file_name, label);
                        if response.clicked() {
                            if *is_dir {
                                navigate_to = Some(self.directory.join(name));
                            } else {
                                self.file_name = name.clone();
                            }
                        }
                        if response.double_clicked() && !is_dir {
                            outcome = BrowserOutcome::Chosen(self.directory.join(name));
                        }
                    }
                    if self.entries.is_empty() {
                        ui.weak("No matching files here.");
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if self.purpose != Purpose::ExportSequence {
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut self.file_name).desired_width(260.0));
                    }
                    let verb = if self.purpose.is_saving() { "Save" } else { "Open" };
                    if ui.button(verb).clicked() {
                        match self.resolve() {
                            Ok(path) => outcome = BrowserOutcome::Chosen(path),
                            Err(error) => self.error = Some(error),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = BrowserOutcome::Cancelled;
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(255, 120, 120), error);
                }
            });
        if let Some(directory) = navigate_to {
            self.directory = directory;
            self.refresh();
        }
        outcome
    }

    /// The chosen path, with the expected extension added when saving.
    fn resolve(&self) -> Result<PathBuf, String> {
        if self.purpose == Purpose::ExportSequence {
            return Ok(self.directory.clone());
        }
        let name = self.file_name.trim();
        if name.is_empty() {
            return Err("type a file name".into());
        }
        let mut path = self.directory.join(name);
        let wanted = self.purpose.extensions();
        let has_wanted = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| wanted.contains(&e.to_ascii_lowercase().as_str()));
        if self.purpose.is_saving() && !has_wanted {
            if let Some(first) = wanted.first() {
                path.set_extension(first);
            }
        }
        if !self.purpose.is_saving() && !path.is_file() {
            return Err(format!("{} doesn't exist", path.display()));
        }
        Ok(path)
    }
}

/// Folders first, then files with a matching extension, each sorted by
/// name. Hidden entries are skipped.
pub fn list_directory(directory: &Path, extensions: &[&str]) -> Vec<(String, bool)> {
    let Ok(read) = std::fs::read_dir(directory) else { return Vec::new() };
    let mut entries: Vec<(String, bool)> = read
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            let is_dir = entry.file_type().ok()?.is_dir();
            let matches =
                is_dir || extensions.iter().any(|ext| name.to_ascii_lowercase().ends_with(&format!(".{ext}")));
            (!name.starts_with('.') && matches).then_some((name, is_dir))
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listings_show_folders_first_and_only_matching_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("zeta")).unwrap();
        for name in ["b.png", "a.spry", "notes.txt", ".hidden.png"] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        let listed = list_directory(dir.path(), &["spry", "png"]);
        assert_eq!(listed, vec![("zeta".into(), true), ("a.spry".into(), false), ("b.png".into(), false)]);
    }

    #[test]
    fn saving_adds_the_expected_extension() {
        let dir = tempfile::tempdir().unwrap();
        let browser = FileBrowser::new(Purpose::ExportGif, dir.path().to_path_buf(), "walk");
        assert_eq!(browser.resolve().unwrap(), dir.path().join("walk.gif"));
    }

    #[test]
    fn opening_a_missing_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let browser = FileBrowser::new(Purpose::Open, dir.path().to_path_buf(), "ghost.spry");
        assert!(browser.resolve().unwrap_err().contains("doesn't exist"));
    }
}

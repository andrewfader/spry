//! A `Studio` is one open document as the outside world sees it: an
//! `Editor` plus where it lives on disk. The glass UI, the MCP server and
//! the AI agent all work through a studio, so "save" means the same thing
//! everywhere, including the automatic version recorded on every save.

use crate::editor::{Command, Document, Editor};
use crate::io;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct Studio {
    pub editor: Editor,
    pub path: Option<PathBuf>,
}

impl Studio {
    pub fn new(width: u32, height: u32) -> Studio {
        Studio { editor: Editor::new(width, height), path: None }
    }

    pub fn open(path: &Path) -> Result<Studio> {
        let document = io::open(path)?;
        // Images open as new, unsaved documents. Only .spry files keep the path.
        let is_spry = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("spry"));
        let mut studio = Studio { editor: Editor::with_document(document), path: is_spry.then(|| path.to_path_buf()) };
        studio.editor.mark_saved();
        Ok(studio)
    }

    pub fn from_document(document: Document) -> Studio {
        Studio { editor: Editor::with_document(document), path: None }
    }

    /// The name shown in tabs and window titles.
    pub fn title(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_stem)
            .and_then(|s| s.to_str())
            .map_or_else(|| "Untitled".to_string(), str::to_string)
    }

    /// Saves to the current path, or to `path` (which becomes the current
    /// one). Every save records a version, unless nothing changed since the
    /// last one.
    pub fn save(&mut self, path: Option<&Path>) -> Result<PathBuf> {
        let target = path
            .map(Path::to_path_buf)
            .or_else(|| self.path.clone())
            .context("this document hasn't been saved yet, so give it a file name")?;
        let target = if target.extension().is_none() { target.with_extension("spry") } else { target };
        self.editor.anchor_floating();
        let stamp = format!("Saved {}", target.file_name().and_then(|n| n.to_str()).unwrap_or("document"));
        self.editor.execute(Command::CommitVersion { message: stamp }).map_err(anyhow::Error::msg)?;
        io::save(&target, &self.editor.document)?;
        self.path = Some(target.clone());
        self.editor.mark_saved();
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Ink, Rgba};

    #[test]
    fn saving_records_a_version_and_reopens_identically() {
        let folder = tempfile::tempdir().unwrap();
        let mut studio = Studio::new(8, 8);
        studio.editor.execute(Command::Fill { at: (0, 0), ink: Ink::colour(Rgba::BLACK), contiguous: true }).unwrap();
        let saved = studio.save(Some(&folder.path().join("hero"))).unwrap();
        assert_eq!(saved.extension().unwrap(), "spry");
        assert_eq!(studio.editor.document.versions.len(), 1);
        assert!(!studio.editor.has_unsaved_changes());

        let reopened = Studio::open(&saved).unwrap();
        assert_eq!(reopened.editor.document, studio.editor.document);
        assert_eq!(reopened.title(), "hero");
    }

    #[test]
    fn saving_twice_without_changes_keeps_one_version() {
        let folder = tempfile::tempdir().unwrap();
        let mut studio = Studio::new(4, 4);
        studio.save(Some(&folder.path().join("a.spry"))).unwrap();
        studio.save(None).unwrap();
        assert_eq!(studio.editor.document.versions.len(), 1);
    }

    #[test]
    fn an_unsaved_document_needs_a_name() {
        assert!(Studio::new(4, 4).save(None).unwrap_err().to_string().contains("file name"));
    }
}

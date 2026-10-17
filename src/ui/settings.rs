//! App preferences, kept as JSON in the platform config directory
//! (`~/.config/spry/settings.json` on Linux).

use super::theme::Appearance;
use crate::ai::agent::{Provider, CLAUDE_URL, DEFAULT_CLAUDE_MODEL};
use crate::ai::diffusion::Backend;
use crate::mcp::http::DEFAULT_PORT;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatProvider {
    Claude,
    Ollama,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub chat_provider: ChatProvider,
    /// Empty means "use ANTHROPIC_API_KEY".
    pub anthropic_api_key: String,
    pub claude_model: String,
    pub ollama_url: String,
    pub ollama_model: String,
    pub comfy_url: String,
    pub comfy_checkpoint: String,
    /// A workflow exported from ComfyUI with "Save (API format)". Empty
    /// means Spry's built-in graphs.
    pub comfy_workflow: String,
    pub a1111_url: String,
    pub use_comfy: bool,
    pub mcp_http_enabled: bool,
    pub mcp_port: u16,
    pub recent_files: Vec<PathBuf>,
    pub last_directory: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            appearance: Appearance::Dark,
            chat_provider: ChatProvider::Claude,
            anthropic_api_key: String::new(),
            claude_model: DEFAULT_CLAUDE_MODEL.into(),
            ollama_url: "http://127.0.0.1:11434".into(),
            ollama_model: "qwen3".into(),
            comfy_url: "http://127.0.0.1:8188".into(),
            comfy_checkpoint: "v1-5-pruned-emaonly.safetensors".into(),
            comfy_workflow: String::new(),
            a1111_url: "http://127.0.0.1:7860".into(),
            use_comfy: false,
            mcp_http_enabled: false,
            mcp_port: DEFAULT_PORT,
            recent_files: Vec::new(),
            last_directory: None,
        }
    }
}

impl Settings {
    pub fn location() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("spry").join("settings.json"))
    }

    pub fn load() -> Settings {
        Settings::location()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Settings::location() else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap_or_default())
    }

    pub fn provider(&self) -> Provider {
        match self.chat_provider {
            ChatProvider::Claude => {
                let key =
                    (!self.anthropic_api_key.trim().is_empty()).then(|| self.anthropic_api_key.trim().to_string());
                match Provider::claude(key) {
                    Provider::Claude { api_key, .. } => {
                        Provider::Claude { api_key, model: self.claude_model.clone(), base_url: CLAUDE_URL.into() }
                    }
                    other => other,
                }
            }
            ChatProvider::Ollama => Provider::Ollama { url: self.ollama_url.clone(), model: self.ollama_model.clone() },
        }
    }

    pub fn diffusion_backend(&self) -> Result<Backend, String> {
        if !self.use_comfy {
            return Ok(Backend::Automatic1111 { url: self.a1111_url.clone() });
        }
        let path = self.comfy_workflow.trim();
        let workflow = if path.is_empty() {
            None
        } else {
            Some(std::fs::read_to_string(path).map_err(|e| format!("couldn't read the workflow {path}: {e}"))?)
        };
        Ok(Backend::ComfyUi { url: self.comfy_url.clone(), workflow, checkpoint: self.comfy_checkpoint.clone() })
    }

    pub fn remember_file(&mut self, path: &std::path::Path) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_path_buf());
        self.recent_files.truncate(10);
        self.last_directory = path.parent().map(|p| p.to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_json_round_trip() {
        let mut settings = Settings { ollama_model: "llava".into(), ..Settings::default() };
        settings.remember_file(std::path::Path::new("/art/hero.spry"));
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), settings);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"ollama_model":"phi"}"#).unwrap();
        assert_eq!(settings.ollama_model, "phi");
        assert_eq!(settings.mcp_port, DEFAULT_PORT);
    }

    #[test]
    fn recent_files_are_unique_and_newest_first() {
        let mut settings = Settings::default();
        for name in ["a", "b", "a"] {
            settings.remember_file(std::path::Path::new(name));
        }
        assert_eq!(settings.recent_files, vec![PathBuf::from("a"), PathBuf::from("b")]);
    }

    #[test]
    fn the_chosen_backend_follows_the_comfy_toggle() {
        let settings = Settings { use_comfy: true, ..Settings::default() };
        assert!(matches!(settings.diffusion_backend(), Ok(Backend::ComfyUi { workflow: None, .. })));
    }

    #[test]
    fn a_custom_comfy_workflow_is_read_from_its_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("flow.json");
        std::fs::write(&path, "{}").unwrap();
        let settings = Settings { use_comfy: true, comfy_workflow: path.display().to_string(), ..Settings::default() };
        assert!(matches!(settings.diffusion_backend(), Ok(Backend::ComfyUi { workflow: Some(w), .. }) if w == "{}"));
        let missing = Settings { comfy_workflow: "/no/such/flow.json".into(), ..settings };
        assert!(missing.diffusion_backend().unwrap_err().contains("couldn't read"));
    }
}

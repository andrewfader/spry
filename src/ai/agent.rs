//! The AI pixel agent: describe a change in words, and an LLM makes it with
//! Spry's own tools.
//!
//! The agent uses exactly the MCP tool catalogue (`mcp::tools`), so what a
//! model can do in the chat panel and over MCP is the same, and both are
//! undoable editor commands. Each user message starts a turn. The model sees
//! the canvas as an image, calls tools, sees their results, and loops until
//! it answers in plain text.
//!
//! Providers:
//! - **Claude** via the Messages API (raw HTTPS; Rust has no official SDK),
//!   defaulting to `claude-opus-5-5`, with server-side refusal fallback on.
//! - **Ollama** via `/api/chat`, for fully local models.
//!
//! The agent never touches the document directly. It calls the `execute`
//! closure it's given, so the UI can run tools on its own thread and tests
//! can run them inline.

use crate::mcp::tools::{self, ToolOutput, ToolResult};
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

pub const DEFAULT_CLAUDE_MODEL: &str = "claude-opus-5-5";
pub const CLAUDE_URL: &str = "https://api.anthropic.com";
const MAX_TOOL_ROUNDS: usize = 40;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provider {
    Claude { api_key: String, model: String, base_url: String },
    Ollama { url: String, model: String },
}

impl Provider {
    /// Claude, with the key from `ANTHROPIC_API_KEY` when one isn't given.
    pub fn claude(api_key: Option<String>) -> Provider {
        Provider::Claude {
            api_key: api_key.or_else(|| std::env::var("ANTHROPIC_API_KEY").ok()).unwrap_or_default(),
            model: DEFAULT_CLAUDE_MODEL.into(),
            base_url: CLAUDE_URL.into(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Provider::Claude { model, .. } => format!("Claude ({model})"),
            Provider::Ollama { model, .. } => format!("Ollama ({model})"),
        }
    }
}

const SYSTEM_PROMPT: &str = "You are a pixel artist working inside Spry, a sprite and animation editor. \
You edit the open sprite only through the provided tools. Coordinates are integer pixels from (0,0) at the top-left; \
layers count from 0 at the bottom; frames from 0. Use the document's palette unless asked otherwise, keep shapes crisp \
(no anti-aliasing), and check your work with render_png or get_pixels before you finish. When done, reply with a short \
summary of what you changed.";

/// Something the UI may want to show while a turn runs.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    Thinking,
    Said(String),
    UsedTool { name: String, ok: bool, summary: String },
}

/// One chat with one provider. The transcript is kept in the provider's own
/// message format and only ever appended to, as the APIs expect.
pub struct Conversation {
    pub provider: Provider,
    transcript: Vec<Value>,
}

impl Conversation {
    pub fn new(provider: Provider) -> Conversation {
        Conversation { provider, transcript: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.transcript.is_empty()
    }

    /// Runs one user turn to completion and returns the model's final text.
    /// `canvas` is a PNG of the current frame, so the model can see it.
    pub fn run_turn(
        &mut self,
        user_text: &str,
        canvas: Option<&[u8]>,
        execute: &mut dyn FnMut(&str, &Value) -> ToolResult,
        on_event: &mut dyn FnMut(AgentEvent),
    ) -> Result<String> {
        match self.provider.clone() {
            Provider::Claude { api_key, model, base_url } => {
                if api_key.trim().is_empty() {
                    bail!("add an Anthropic API key in Settings (or set ANTHROPIC_API_KEY) to use Claude");
                }
                self.claude_turn(&api_key, &model, &base_url, user_text, canvas, execute, on_event)
            }
            Provider::Ollama { url, model } => self.ollama_turn(&url, &model, user_text, canvas, execute, on_event),
        }
    }

    // ----- Claude ---------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn claude_turn(
        &mut self,
        api_key: &str,
        model: &str,
        base_url: &str,
        user_text: &str,
        canvas: Option<&[u8]>,
        execute: &mut dyn FnMut(&str, &Value) -> ToolResult,
        on_event: &mut dyn FnMut(AgentEvent),
    ) -> Result<String> {
        let mut content = Vec::new();
        if let Some(png) = canvas {
            content.push(claude_image(png));
        }
        content.push(json!({ "type": "text", "text": user_text }));
        self.transcript.push(json!({ "role": "user", "content": content }));

        let tool_definitions: Vec<Value> = tools::catalogue()
            .into_iter()
            .map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.input_schema }))
            .collect();
        let http = http_agent();
        for _ in 0..MAX_TOOL_ROUNDS {
            on_event(AgentEvent::Thinking);
            let body = json!({
                "model": model,
                "max_tokens": 16000,
                "system": SYSTEM_PROMPT,
                "tools": tool_definitions,
                "messages": self.transcript,
                // If a safety classifier declines, the API retries on a
                // suitable fallback model instead of failing the turn.
                "fallbacks": "default",
            });
            let mut response = http
                .post(&format!("{}/v1/messages", base_url.trim_end_matches('/')))
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("anthropic-beta", "server-side-fallback-2026-07-01")
                .config()
                .http_status_as_error(false)
                .build()
                .send_json(&body)
                .context("reaching the Claude API")?;
            let status = response.status().as_u16();
            let reply: Value =
                response.body_mut().read_json().context("the Claude API sent something that isn't JSON")?;
            if status >= 400 {
                let message = reply["error"]["message"].as_str().unwrap_or("unknown error");
                bail!("Claude API error {status}: {message}");
            }

            let blocks = reply["content"].as_array().cloned().unwrap_or_default();
            // The whole content goes back into the transcript unchanged,
            // thinking blocks included, as the API requires.
            self.transcript.push(json!({ "role": "assistant", "content": blocks }));
            let said = text_of_blocks(&blocks);
            if !said.is_empty() {
                on_event(AgentEvent::Said(said.clone()));
            }
            match reply["stop_reason"].as_str() {
                Some("tool_use") => {
                    let results: Vec<Value> = blocks
                        .iter()
                        .filter(|b| b["type"] == "tool_use")
                        .map(|call| {
                            let name = call["name"].as_str().unwrap_or_default();
                            let outcome = execute(name, &call["input"]);
                            report(on_event, name, &outcome);
                            claude_tool_result(call["id"].as_str().unwrap_or_default(), outcome)
                        })
                        .collect();
                    self.transcript.push(json!({ "role": "user", "content": results }));
                }
                Some("refusal") => bail!("Claude declined this request"),
                Some("pause_turn") => continue,
                _ => return Ok(said),
            }
        }
        bail!("the agent used {MAX_TOOL_ROUNDS} rounds of tools without finishing; try a smaller request")
    }

    // ----- Ollama ---------------------------------------------------------

    fn ollama_turn(
        &mut self,
        url: &str,
        model: &str,
        user_text: &str,
        canvas: Option<&[u8]>,
        execute: &mut dyn FnMut(&str, &Value) -> ToolResult,
        on_event: &mut dyn FnMut(AgentEvent),
    ) -> Result<String> {
        if self.transcript.is_empty() {
            self.transcript.push(json!({ "role": "system", "content": SYSTEM_PROMPT }));
        }
        let mut message = json!({ "role": "user", "content": user_text });
        if let Some(png) = canvas {
            message["images"] = json!([base64::engine::general_purpose::STANDARD.encode(png)]);
        }
        self.transcript.push(message);
        let tool_definitions: Vec<Value> = tools::catalogue()
            .into_iter()
            .map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.input_schema } }))
            .collect();
        let http = http_agent();
        for _ in 0..MAX_TOOL_ROUNDS {
            on_event(AgentEvent::Thinking);
            let body =
                json!({ "model": model, "messages": self.transcript, "tools": tool_definitions, "stream": false });
            let reply: Value = http
                .post(&format!("{}/api/chat", url.trim_end_matches('/')))
                .send_json(&body)
                .with_context(|| format!("reaching Ollama at {url} (is `ollama serve` running and `{model}` pulled?)"))?
                .body_mut()
                .read_json()?;
            let message = reply["message"].clone();
            self.transcript.push(message.clone());
            let said = message["content"].as_str().unwrap_or_default().trim().to_string();
            if !said.is_empty() {
                on_event(AgentEvent::Said(said.clone()));
            }
            let calls = message["tool_calls"].as_array().cloned().unwrap_or_default();
            if calls.is_empty() {
                return Ok(said);
            }
            for call in calls {
                let name = call["function"]["name"].as_str().unwrap_or_default();
                // Some models send arguments as a JSON string, others as an object.
                let arguments = match &call["function"]["arguments"] {
                    Value::String(text) => serde_json::from_str(text).unwrap_or(json!({})),
                    other => other.clone(),
                };
                let outcome = execute(name, &arguments);
                report(on_event, name, &outcome);
                let content = match &outcome {
                    Ok(output) => output.text.clone(),
                    Err(error) => format!("error: {error}"),
                };
                let mut tool_message = json!({ "role": "tool", "content": content, "tool_name": name });
                if let Ok(ToolOutput { png: Some(png), .. }) = &outcome {
                    tool_message["images"] = json!([base64::engine::general_purpose::STANDARD.encode(png)]);
                }
                self.transcript.push(tool_message);
            }
        }
        bail!("the agent used {MAX_TOOL_ROUNDS} rounds of tools without finishing; try a smaller request")
    }
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).build().into()
}

fn report(on_event: &mut dyn FnMut(AgentEvent), name: &str, outcome: &ToolResult) {
    let (ok, summary) = match outcome {
        Ok(output) => (true, output.text.lines().next().unwrap_or_default().to_string()),
        Err(error) => (false, error.clone()),
    };
    on_event(AgentEvent::UsedTool { name: name.to_string(), ok, summary });
}

fn claude_image(png: &[u8]) -> Value {
    json!({ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": base64::engine::general_purpose::STANDARD.encode(png) } })
}

fn claude_tool_result(tool_use_id: &str, outcome: ToolResult) -> Value {
    match outcome {
        Ok(output) => {
            let mut content = vec![json!({ "type": "text", "text": output.text })];
            if let Some(png) = &output.png {
                content.push(claude_image(png));
            }
            json!({ "type": "tool_result", "tool_use_id": tool_use_id, "content": content })
        }
        Err(error) => json!({ "type": "tool_result", "tool_use_id": tool_use_id, "content": error, "is_error": true }),
    }
}

fn text_of_blocks(blocks: &[Value]) -> String {
    blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

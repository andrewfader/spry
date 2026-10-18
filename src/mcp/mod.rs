//! Model Context Protocol server: lets any LLM agent drive Spry.
//!
//! The protocol layer here is transport-agnostic: `Server::handle` takes
//! one JSON-RPC message and returns the reply. `serve_stdio` wires it to
//! stdin/stdout for `spry mcp`, and `http` serves it from the running app,
//! so an agent can paint while you watch.

pub mod http;
pub mod resources;
pub mod tools;

/// Where MCP tools do their work. Most tools edit one studio. Two of them,
/// `new_document` and `open_document`, bring a whole new studio into the
/// world, and what that means depends on the host. Alone on stdio, the new
/// studio simply takes over. In the app, it opens in a new tab, and the
/// document you were working on stays exactly as it was.
pub trait Workspace {
    /// The studio the next tool call works on.
    fn studio(&mut self) -> &mut Studio;
    /// Brings in a new studio, which becomes the one tools work on.
    fn open(&mut self, studio: Studio);

    /// Runs a diffusion generation that lands as a new layer, and says
    /// what happened. Alone on stdio there's nothing else to do, so this
    /// waits for the picture. The app generates in the background instead,
    /// so a slow model never freezes the window.
    fn generate(&mut self, request: Request) -> Result<String, String> {
        let image = request.run().map_err(|e| format!("{e:#}"))?;
        let seed = request.generation.seed;
        let outcome = self.studio().editor.execute(Command::ImportLayer { name: request.layer_name, image });
        outcome.map(|summary| format!("{summary} (seed {seed})")).map_err(|e| e.to_string())
    }
}

impl Workspace for Studio {
    fn studio(&mut self) -> &mut Studio {
        self
    }

    fn open(&mut self, studio: Studio) {
        *self = studio;
    }
}

use crate::ai::diffusion::Request;
use crate::editor::Command;
use crate::studio::Studio;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

const INSTRUCTIONS: &str = "Spry is a pixel art editor. Coordinates are integer pixels with (0,0) at the top-left; \
layers are numbered from 0 at the bottom; frames from 0. Start with document_info, look with render_png or get_pixels, \
and prefer the document's palette colours. Every change is undoable.";

/// Handles one JSON-RPC message against a studio. Notifications (messages
/// without an `id`) get no reply.
pub fn handle(workspace: &mut impl Workspace, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(json!({}));
    let id = id?;
    let outcome = match method {
        "initialize" => Ok(json!({
            // We implement one revision and always answer with it. The
            // spec leaves it to the client whether to carry on.
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false }, "resources": { "listChanged": false, "subscribe": false } },
            "serverInfo": { "name": "spry", "title": "Spry pixel editor", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_list() })),
        "tools/call" => call_tool(workspace, &params),
        "resources/list" => Ok(resources::list(workspace.studio())),
        "resources/templates/list" => Ok(resources::templates()),
        "resources/read" => match params.get("uri").and_then(Value::as_str) {
            // -32002 is the spec's "resource not found".
            Some(uri) => resources::read(workspace.studio(), uri).map_err(|message| (-32002, message)),
            None => Err((-32602, "resources/read needs a `uri`".to_string())),
        },
        _ => Err((-32601, format!("method `{method}` is not supported"))),
    };
    Some(match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

fn call_tool(workspace: &mut impl Workspace, params: &Value) -> Result<Value, (i64, String)> {
    let name =
        params.get("name").and_then(Value::as_str).ok_or((-32602, "tools/call needs a tool `name`".to_string()))?;
    if !tools::catalogue().iter().any(|t| t.name == name) {
        return Err((-32602, format!("unknown tool `{name}`")));
    }
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    // Tool failures are results, not protocol errors, so the model can
    // read the message and correct itself.
    Ok(match tools::call(workspace, name, &arguments) {
        Ok(output) => json!({ "content": tools::content_blocks(&output), "isError": false }),
        Err(message) => json!({ "content": [{ "type": "text", "text": message }], "isError": true }),
    })
}

pub fn tool_list() -> Vec<Value> {
    tools::catalogue()
        .into_iter()
        .map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": t.input_schema }))
        .collect()
}

/// Serves MCP over newline-delimited JSON on the given streams until the
/// input closes.
pub fn serve(studio: &mut Studio, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(studio, &message),
            Err(error) => Some(
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {error}") } }),
            ),
        };
        if let Some(reply) = reply {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

pub fn serve_stdio(mut studio: Studio) -> std::io::Result<()> {
    serve(&mut studio, std::io::stdin().lock(), std::io::stdout().lock())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: u64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn initialize_announces_tools_and_the_protocol_version() {
        let reply =
            handle(&mut Studio::new(8, 8), &request(1, "initialize", json!({ "protocolVersion": PROTOCOL_VERSION })))
                .unwrap();
        assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(reply["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn notifications_get_no_reply() {
        let notification = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle(&mut Studio::new(8, 8), &notification).is_none());
    }

    #[test]
    fn tool_failures_come_back_as_error_results() {
        let reply = handle(
            &mut Studio::new(8, 8),
            &request(2, "tools/call", json!({ "name": "fill", "arguments": { "at": [0, 0] } })),
        )
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
    }

    #[test]
    fn unknown_methods_are_protocol_errors() {
        let reply = handle(&mut Studio::new(8, 8), &request(3, "sampling/createMessage", json!({}))).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[test]
    fn the_stream_server_answers_line_by_line_and_reports_bad_json() {
        let input = format!("{}\nnot json\n", request(1, "ping", json!({})));
        let mut output = Vec::new();
        serve(&mut Studio::new(4, 4), input.as_bytes(), &mut output).unwrap();
        let lines: Vec<Value> =
            String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["result"], json!({}));
        assert_eq!(lines[1]["error"]["code"], -32700);
    }
}

//! MCP over local HTTP, for the running app.
//!
//! The UI thread owns the open documents, so the HTTP thread never touches
//! one. It forwards each JSON-RPC message through a channel, wakes the UI,
//! and waits for the reply. The UI answers between frames, so an agent's
//! edits appear live, and they undo like your own.
//!
//! This follows MCP's "streamable HTTP" transport in its plain-JSON form:
//! `POST /mcp` with one message, and a JSON reply (or `202 Accepted` for a
//! notification). It only binds to loopback and refuses cross-site
//! `Origin`s, which is the DNS-rebinding guard the spec asks for.

use anyhow::{Context, Result};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 7477;
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// One message waiting for whoever owns the studio to answer it.
pub struct PendingMessage {
    pub message: Value,
    reply: Sender<Option<Value>>,
}

impl PendingMessage {
    pub fn answer(self, reply: Option<Value>) {
        // The HTTP thread may have timed out and left; that's fine.
        let _ = self.reply.send(reply);
    }
}

pub struct HttpBridge {
    pub address: SocketAddr,
    pub messages: Receiver<PendingMessage>,
    server: Arc<tiny_http::Server>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl HttpBridge {
    /// Starts listening on `127.0.0.1:port` (0 picks a free port). `wake`
    /// is called whenever a message arrives, so a sleeping UI can repaint
    /// and answer it.
    pub fn start(port: u16, wake: impl Fn() + Send + 'static) -> Result<HttpBridge> {
        let server = tiny_http::Server::http(("127.0.0.1", port))
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("starting the MCP server on port {port}"))?;
        let address = server.server_addr().to_ip().context("the MCP server has no IP address")?;
        let server = Arc::new(server);
        let (sender, messages) = mpsc::channel();
        let listener = Arc::clone(&server);
        let worker = std::thread::Builder::new()
            .name("spry-mcp-http".into())
            .spawn(move || {
                for request in listener.incoming_requests() {
                    respond(request, &sender, &wake);
                }
            })
            .context("starting the MCP server thread")?;
        Ok(HttpBridge { address, messages, server, worker: Some(worker) })
    }

    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.address)
    }
}

impl Drop for HttpBridge {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn respond(mut request: tiny_http::Request, sender: &Sender<PendingMessage>, wake: &impl Fn()) {
    let reply_with = |request: tiny_http::Request, status: u16, body: String| {
        let json = tiny_http::Header::from_bytes("Content-Type", "application/json").expect("static header");
        let _ = request.respond(tiny_http::Response::from_string(body).with_status_code(status).with_header(json));
    };
    if request.url().split('?').next() != Some("/mcp") {
        return reply_with(request, 404, r#"{"error":"MCP lives at /mcp"}"#.into());
    }
    if *request.method() != tiny_http::Method::Post {
        return reply_with(request, 405, r#"{"error":"POST JSON-RPC messages to /mcp"}"#.into());
    }
    if !origin_is_local(&request) {
        return reply_with(request, 403, r#"{"error":"cross-origin requests are refused"}"#.into());
    }
    let mut body = String::new();
    if std::io::Read::read_to_string(request.as_reader(), &mut body).is_err() {
        return reply_with(request, 400, r#"{"error":"unreadable body"}"#.into());
    }
    let message: Value = match serde_json::from_str(&body) {
        Ok(message) => message,
        Err(error) => {
            let reply = serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {error}") } });
            return reply_with(request, 400, reply.to_string());
        }
    };
    let (reply_sender, reply_receiver) = mpsc::channel();
    if sender.send(PendingMessage { message, reply: reply_sender }).is_err() {
        return reply_with(request, 503, r#"{"error":"Spry is shutting down"}"#.into());
    }
    wake();
    match reply_receiver.recv_timeout(REPLY_TIMEOUT) {
        Ok(Some(reply)) => reply_with(request, 200, reply.to_string()),
        Ok(None) => {
            let _ = request.respond(tiny_http::Response::empty(202));
        }
        Err(_) => reply_with(request, 504, r#"{"error":"Spry did not answer in time"}"#.into()),
    }
}

fn origin_is_local(request: &tiny_http::Request) -> bool {
    request.headers().iter().filter(|h| h.field.equiv("Origin")).all(|origin| {
        let value = origin.value.as_str();
        ["http://localhost", "http://127.0.0.1", "https://localhost", "https://127.0.0.1"]
            .iter()
            .any(|local| value == *local || value.starts_with(&format!("{local}:")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::Studio;
    use serde_json::json;

    /// Answers bridge messages on a background thread, the way the UI does
    /// between frames.
    fn answer_in_background(bridge: HttpBridge) -> (String, std::thread::JoinHandle<()>) {
        let url = bridge.url();
        let handle = std::thread::spawn(move || {
            let mut studio = Studio::new(8, 8);
            for pending in bridge.messages.iter().take(2) {
                let reply = crate::mcp::handle(&mut studio, &pending.message);
                pending.answer(reply);
            }
        });
        (url, handle)
    }

    #[test]
    fn json_rpc_round_trips_over_http() {
        let bridge = HttpBridge::start(0, || {}).unwrap();
        let (url, worker) = answer_in_background(bridge);
        let reply: Value = ureq::post(&url)
            .send_json(json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" }))
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(reply["id"], 7);
        let accepted =
            ureq::post(&url).send_json(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).unwrap();
        assert_eq!(accepted.status(), 202);
        worker.join().unwrap();
    }

    #[test]
    fn cross_site_origins_are_refused() {
        let bridge = HttpBridge::start(0, || {}).unwrap();
        let refused = ureq::post(&bridge.url())
            .header("Origin", "https://evil.example")
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }))
            .unwrap();
        assert_eq!(refused.status(), 403);
    }
}

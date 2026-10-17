//! The scenario vocabulary: Given/When/Then helpers that let each feature
//! file read like the acceptance criteria in `docs/BIBLE.md`.
//!
//! Scenarios drive the real headless studio, the real file formats, the
//! real `spry` binary, and real local HTTP servers standing in for
//! diffusion and LLM backends. Nothing of ours is mocked.

#![allow(dead_code)]

use serde_json::Value;
use spry::editor::{Command, EditResult, Pixels, Rgba, Sprite};
pub use spry::editor::{FrameIndex, LayerIndex};
use spry::studio::Studio;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const RED: Rgba = Rgba::rgb(255, 0, 77);
pub const GREEN: Rgba = Rgba::rgb(0, 228, 54);
pub const BLUE: Rgba = Rgba::rgb(41, 173, 255);
pub const BLACK: Rgba = Rgba::BLACK;
pub const CLEAR: Rgba = Rgba::TRANSPARENT;

pub struct Scenario {
    pub studio: Studio,
    pub folder: tempfile::TempDir,
    pub last_outcome: Option<EditResult>,
}

pub fn given_a_blank_sprite(width: u32, height: u32) -> Scenario {
    Scenario { studio: Studio::new(width, height), folder: tempfile::tempdir().expect("temp dir"), last_outcome: None }
}

impl Scenario {
    pub fn sprite(&self) -> &Sprite {
        self.studio.editor.sprite()
    }

    /// Runs a command, insisting it succeeds.
    pub fn when(&mut self, command: Command) -> &mut Self {
        let outcome = self.studio.editor.execute(command.clone());
        assert!(outcome.is_ok(), "{command:?} failed: {outcome:?}");
        self.last_outcome = Some(outcome);
        self
    }

    /// Runs a command that may fail, keeping the outcome for `then_it_*`.
    pub fn when_trying(&mut self, command: Command) -> &mut Self {
        self.last_outcome = Some(self.studio.editor.execute(command));
        self
    }

    pub fn and(&mut self, command: Command) -> &mut Self {
        self.when(command)
    }

    pub fn then_it_failed_saying(&self, words: &str) {
        match &self.last_outcome {
            Some(Err(error)) => assert!(error.to_string().contains(words), "error was: {error}"),
            other => panic!("expected a failure mentioning `{words}`, got {other:?}"),
        }
    }

    /// The pixel in the current layer and frame.
    pub fn pixel(&self, x: i32, y: i32) -> Rgba {
        let editor = &self.studio.editor;
        self.cel_pixel(editor.layer, editor.frame, x, y)
    }

    /// What a cel shows at (x, y): a tilemap's tiles are drawn out.
    pub fn cel_pixel(&self, layer: usize, frame: usize, x: i32, y: i32) -> Rgba {
        self.sprite().cel_pixels(LayerIndex(layer), FrameIndex(frame)).get(x, y)
    }

    pub fn composite_pixel(&self, frame: usize, x: i32, y: i32) -> Rgba {
        self.sprite().composite(frame).get(x, y)
    }

    pub fn then_pixels(&self, expected: &[(i32, i32)], colour: Rgba) {
        for &(x, y) in expected {
            assert_eq!(self.pixel(x, y), colour, "pixel ({x}, {y})");
        }
    }

    /// Every painted pixel on the current cel.
    pub fn painted_pixels(&self) -> Vec<(i32, i32)> {
        let Some(image) =
            self.sprite().cel_image(LayerIndex(self.studio.editor.layer), FrameIndex(self.studio.editor.frame))
        else {
            return Vec::new();
        };
        image.bounds().points().filter(|&(x, y)| !image.get(x, y).is_transparent()).collect()
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.folder.path().join(name)
    }

    pub fn tool(&mut self, name: &str, arguments: Value) -> Result<spry::mcp::tools::ToolOutput, String> {
        spry::mcp::tools::call(&mut self.studio, name, &arguments)
    }
}

/// A picture made from rows of characters, one per pixel, for writing
/// expected art inline: `.` is transparent and other letters map through
/// `key`.
pub fn picture(rows: &[&str], key: &[(char, Rgba)]) -> Pixels {
    let mut image = Pixels::new(rows[0].len() as u32, rows.len() as u32);
    for (y, row) in rows.iter().enumerate() {
        for (x, symbol) in row.chars().enumerate() {
            let colour = key.iter().find(|(k, _)| *k == symbol).map_or(CLEAR, |(_, c)| *c);
            image.set(x as i32, y as i32, colour);
        }
    }
    image
}

/// A real HTTP server on a free loopback port, answering with a scripted
/// handler and recording every request it gets.
pub struct FakeServer {
    pub url: String,
    pub requests: Arc<Mutex<Vec<RecordedRequest>>>,
    server: Arc<tiny_http::Server>,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl RecordedRequest {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(value: Value) -> Reply {
        Reply { status: 200, content_type: "application/json", body: value.to_string().into_bytes() }
    }

    pub fn png(bytes: Vec<u8>) -> Reply {
        Reply { status: 200, content_type: "image/png", body: bytes }
    }
}

impl FakeServer {
    pub fn start(handler: impl Fn(&RecordedRequest, usize) -> Reply + Send + 'static) -> FakeServer {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("bind a loopback port"));
        let url = format!("http://{}", server.server_addr().to_ip().expect("ip address"));
        let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::default();
        let (listener, log) = (Arc::clone(&server), Arc::clone(&requests));
        let worker = std::thread::spawn(move || {
            for mut request in listener.incoming_requests() {
                let mut body = String::new();
                let _ = std::io::Read::read_to_string(request.as_reader(), &mut body);
                let recorded = RecordedRequest {
                    method: request.method().to_string(),
                    path: request.url().to_string(),
                    headers: request.headers().iter().map(|h| (h.field.to_string(), h.value.to_string())).collect(),
                    body,
                };
                let index = {
                    let mut log = log.lock().expect("request log");
                    log.push(recorded.clone());
                    log.len() - 1
                };
                let reply = handler(&recorded, index);
                let header = tiny_http::Header::from_bytes("Content-Type", reply.content_type).expect("header");
                let _ = request.respond(
                    tiny_http::Response::from_data(reply.body).with_status_code(reply.status).with_header(header),
                );
            }
        });
        FakeServer { url, requests, server, worker: Some(worker) }
    }

    pub fn recorded(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("request log").clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

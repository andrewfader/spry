//! Background work that mustn't freeze the canvas: LLM turns and diffusion
//! generations.
//!
//! Network calls run on worker threads, but documents live on the UI thread.
//! So the agent's tool calls come back to the UI as `ToolRequest`s, get run
//! between frames against the active document, and their results are sent
//! back. This is the same arrangement as the MCP HTTP bridge.

use crate::ai::agent::{AgentEvent, Conversation, Provider};
use crate::ai::diffusion::{self, Backend, Generation, Landing};
use crate::editor::Pixels;
use crate::mcp::tools::ToolResult;
use serde_json::Value;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

pub struct ToolRequest {
    pub name: String,
    pub arguments: Value,
    reply: Sender<ToolResult>,
}

impl ToolRequest {
    pub fn answer(self, result: ToolResult) {
        let _ = self.reply.send(result);
    }
}

pub enum AgentMessage {
    Event(AgentEvent),
    Tool(ToolRequest),
    Finished(Result<String, String>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChatLine {
    User(String),
    Assistant(String),
    Tool { name: String, ok: bool, summary: String },
    Error(String),
}

pub struct AgentWorker {
    conversation: Arc<Mutex<Conversation>>,
    inbox: Option<Receiver<AgentMessage>>,
    pub lines: Vec<ChatLine>,
    pub busy: bool,
}

impl AgentWorker {
    pub fn new(provider: Provider) -> AgentWorker {
        AgentWorker {
            conversation: Arc::new(Mutex::new(Conversation::new(provider))),
            inbox: None,
            lines: Vec::new(),
            busy: false,
        }
    }

    /// Starts a fresh conversation, for instance after the provider changed.
    pub fn reset(&mut self, provider: Provider) {
        if !self.busy {
            *self = AgentWorker::new(provider);
        }
    }

    pub fn provider_label(&self) -> String {
        self.conversation.lock().map(|c| c.provider.label()).unwrap_or_default()
    }

    pub fn send(&mut self, text: String, canvas_png: Option<Vec<u8>>, wake: impl Fn() + Send + 'static) {
        if self.busy || text.trim().is_empty() {
            return;
        }
        self.lines.push(ChatLine::User(text.clone()));
        self.busy = true;
        let (outbox, inbox) = mpsc::channel();
        self.inbox = Some(inbox);
        let conversation = Arc::clone(&self.conversation);
        std::thread::spawn(move || {
            let notify = |message: AgentMessage, outbox: &Sender<AgentMessage>| {
                let _ = outbox.send(message);
                wake();
            };
            let mut execute = |name: &str, arguments: &Value| -> ToolResult {
                let (reply, answer) = mpsc::channel();
                notify(
                    AgentMessage::Tool(ToolRequest { name: name.to_string(), arguments: arguments.clone(), reply }),
                    &outbox,
                );
                answer.recv().unwrap_or_else(|_| Err("Spry closed before the tool ran".into()))
            };
            let mut on_event = |event: AgentEvent| notify(AgentMessage::Event(event), &outbox);
            let outcome = match conversation.lock() {
                Ok(mut conversation) => conversation
                    .run_turn(&text, canvas_png.as_deref(), &mut execute, &mut on_event)
                    .map_err(|e| format!("{e:#}")),
                Err(_) => Err("the previous turn crashed; start a new chat".into()),
            };
            notify(AgentMessage::Finished(outcome), &outbox);
        });
    }

    /// Takes everything the worker has sent since the last frame. Tool
    /// requests are handed back for the caller to run.
    pub fn drain(&mut self) -> Vec<ToolRequest> {
        let mut requests = Vec::new();
        let Some(inbox) = &self.inbox else { return requests };
        let mut finished = false;
        for message in inbox.try_iter() {
            match message {
                AgentMessage::Tool(request) => requests.push(request),
                AgentMessage::Event(AgentEvent::Said(text)) => self.lines.push(ChatLine::Assistant(text)),
                AgentMessage::Event(AgentEvent::UsedTool { name, ok, summary }) => {
                    self.lines.push(ChatLine::Tool { name, ok, summary })
                }
                AgentMessage::Event(AgentEvent::Thinking) => {}
                AgentMessage::Finished(outcome) => {
                    if let Err(error) = outcome {
                        self.lines.push(ChatLine::Error(error));
                    }
                    finished = true;
                }
            }
        }
        if finished {
            self.busy = false;
            self.inbox = None;
        }
        requests
    }

    /// Whether the worker finished a turn since `drain` was last called
    /// (the UI closes the turn's undo group then).
    pub fn is_idle(&self) -> bool {
        !self.busy
    }
}

/// One diffusion generation in flight.
pub struct DiffusionJob {
    inbox: Receiver<Result<Pixels, String>>,
    pub prompt: String,
}

impl DiffusionJob {
    pub fn start(
        backend: Backend,
        generation: Generation,
        landing: Landing,
        wake: impl Fn() + Send + 'static,
    ) -> DiffusionJob {
        let (outbox, inbox) = mpsc::channel();
        let prompt = generation.prompt.clone();
        std::thread::spawn(move || {
            let result = diffusion::generate_sprite(&backend, &generation, &landing).map_err(|e| format!("{e:#}"));
            let _ = outbox.send(result);
            wake();
        });
        DiffusionJob { inbox, prompt }
    }

    pub fn poll(&self) -> Option<Result<Pixels, String>> {
        self.inbox.try_recv().ok()
    }
}

//! Feature 16: the AI pixel agent, against scripted local LLM servers that
//! speak the real Claude Messages and Ollama chat formats.

use crate::support::*;
use serde_json::{json, Value};
use spry::ai::agent::{AgentEvent, Conversation, Provider};
use spry::editor::Command;

fn claude_reply(content: Value, stop_reason: &str) -> Reply {
    Reply::json(
        json!({ "id": "msg_1", "type": "message", "role": "assistant", "content": content, "stop_reason": stop_reason }),
    )
}

/// A Claude stand-in: first asks to draw a rectangle, then says it's done.
fn scripted_claude() -> FakeServer {
    FakeServer::start(|_, index| match index {
        0 => claude_reply(
            json!([
                { "type": "text", "text": "I'll add a red box." },
                { "type": "tool_use", "id": "toolu_1", "name": "draw_rect", "input": { "from": [1, 1], "to": [4, 4], "filled": true, "colour": "#ff004d" } }
            ]),
            "tool_use",
        ),
        _ => claude_reply(json!([{ "type": "text", "text": "Added a 4×4 red box." }]), "end_turn"),
    })
}

fn run(
    scenario: &mut Scenario,
    conversation: &mut Conversation,
    prompt: &str,
) -> (anyhow::Result<String>, Vec<AgentEvent>) {
    let mut events = Vec::new();
    scenario.studio.editor.begin_undo_group();
    let studio = &mut scenario.studio;
    let outcome = conversation.run_turn(
        prompt,
        Some(b"fake png"),
        &mut |name, args| spry::mcp::tools::call(studio, name, args),
        &mut |event| events.push(event),
    );
    scenario.studio.editor.end_undo_group();
    (outcome, events)
}

#[test]
fn a_claude_tool_call_paints_the_canvas_and_one_undo_removes_the_turn() {
    let server = scripted_claude();
    let mut scenario = given_a_blank_sprite(8, 8);
    let mut conversation = Conversation::new(Provider::Claude {
        api_key: "test-key".into(),
        model: "claude-opus-5-5".into(),
        base_url: server.url.clone(),
    });
    let (outcome, events) = run(&mut scenario, &mut conversation, "add a red box");

    assert_eq!(outcome.unwrap(), "Added a 4×4 red box.");
    assert_eq!(scenario.pixel(2, 2), RED);
    assert!(events.iter().any(|e| matches!(e, AgentEvent::UsedTool { name, ok: true, .. } if name == "draw_rect")));
    scenario.when(Command::Undo);
    assert!(scenario.painted_pixels().is_empty(), "one undo removes the whole turn");
}

#[test]
fn claude_requests_carry_the_key_tools_canvas_and_tool_results() {
    let server = scripted_claude();
    let mut scenario = given_a_blank_sprite(8, 8);
    let mut conversation = Conversation::new(Provider::Claude {
        api_key: "test-key".into(),
        model: "claude-opus-5-5".into(),
        base_url: server.url.clone(),
    });
    run(&mut scenario, &mut conversation, "add a red box").0.unwrap();

    let requests = server.recorded();
    assert_eq!(requests.len(), 2);
    let first = &requests[0];
    assert_eq!(first.path, "/v1/messages");
    assert_eq!(first.header("x-api-key"), Some("test-key"));
    assert_eq!(first.header("anthropic-version"), Some("2023-06-01"));
    let body = first.json();
    assert_eq!(body["model"], "claude-opus-5-5");
    assert!(body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "draw_rect" && t["input_schema"]["type"] == "object"));
    assert_eq!(body["messages"][0]["content"][0]["type"], "image", "the model sees the canvas");

    let second = requests[1].json();
    let turns = second["messages"].as_array().unwrap();
    assert_eq!(turns[1]["role"], "assistant");
    assert_eq!(turns[1]["content"][1]["type"], "tool_use", "the assistant turn is passed back unchanged");
    assert_eq!(turns[2]["content"][0]["type"], "tool_result");
    assert_eq!(turns[2]["content"][0]["tool_use_id"], "toolu_1");
}

#[test]
fn claude_api_errors_are_reported_plainly() {
    let server = FakeServer::start(|_, _| Reply {
        status: 401,
        content_type: "application/json",
        body: br#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.to_vec(),
    });
    let mut scenario = given_a_blank_sprite(8, 8);
    let mut conversation = Conversation::new(Provider::Claude {
        api_key: "bad".into(),
        model: "claude-opus-5-5".into(),
        base_url: server.url.clone(),
    });
    let error = run(&mut scenario, &mut conversation, "hi").0.unwrap_err();
    assert!(error.to_string().contains("401") && error.to_string().contains("invalid x-api-key"), "{error}");
}

#[test]
fn claude_without_a_key_explains_how_to_add_one() {
    let mut scenario = given_a_blank_sprite(8, 8);
    let mut conversation = Conversation::new(Provider::Claude {
        api_key: " ".into(),
        model: "claude-opus-5-5".into(),
        base_url: "http://127.0.0.1:9".into(),
    });
    let error = run(&mut scenario, &mut conversation, "hi").0.unwrap_err();
    assert!(error.to_string().contains("API key"));
}

#[test]
fn a_local_ollama_model_uses_the_same_tools() {
    let server = FakeServer::start(|_, index| match index {
        0 => Reply::json(
            json!({ "message": { "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": "fill", "arguments": { "at": [0, 0], "colour": "#00e436" } } }] } }),
        ),
        _ => Reply::json(json!({ "message": { "role": "assistant", "content": "Filled it green." } })),
    });
    let mut scenario = given_a_blank_sprite(4, 4);
    let mut conversation = Conversation::new(Provider::Ollama { url: server.url.clone(), model: "qwen3".into() });
    let (outcome, _) = run(&mut scenario, &mut conversation, "make it green");
    assert_eq!(outcome.unwrap(), "Filled it green.");
    assert_eq!(scenario.pixel(3, 3), GREEN);
    let second = server.recorded()[1].json();
    let last = second["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["role"], "tool");
    assert_eq!(server.recorded()[0].path, "/api/chat");
}

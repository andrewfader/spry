//! Feature 15: the MCP server, driven end to end through the real `spry`
//! binary over stdio, and through the HTTP bridge the app uses.

use crate::support::*;
use base64::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// A wedged binary fails the scenario instead of hanging the whole suite.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

struct McpSession {
    child: Child,
    input: ChildStdin,
    /// Lines from the server's stdout, read on their own thread so a
    /// missing reply can time out.
    replies: Receiver<String>,
    next_id: u64,
}

impl McpSession {
    fn launch(extra: &[&str]) -> McpSession {
        let mut child = Command::new(env!("CARGO_BIN_EXE_spry"))
            .arg("mcp")
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the spry binary starts");
        let input = child.stdin.take().expect("stdin");
        let output = BufReader::new(child.stdout.take().expect("stdout"));
        let (sender, replies) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in output.lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        McpSession { child, input, replies, next_id: 1 }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.input, "{}", json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).unwrap();
        let line = self.replies.recv_timeout(REPLY_TIMEOUT).unwrap_or_else(|_| panic!("no reply to {method}"));
        let reply: Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("not JSON: {line}"));
        assert_eq!(reply["id"], id);
        reply
    }

    fn notify(&mut self, method: &str) {
        writeln!(self.input, "{}", json!({ "jsonrpc": "2.0", "method": method })).unwrap();
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({ "name": tool, "arguments": arguments }))["result"].clone()
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn initialize_then_list_every_tool_with_a_schema() {
    let mut session = McpSession::launch(&[]);
    let init = session.request("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }));
    assert_eq!(init["result"]["serverInfo"]["name"], "spry");
    session.notify("notifications/initialized");
    let listed = session.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "document_info",
        "new_document",
        "open_document",
        "save_document",
        "set_pixels",
        "get_pixels",
        "draw_line",
        "draw_rect",
        "draw_ellipse",
        "fill",
        "dither_fill",
        "render_png",
        "add_layer",
        "add_frame",
        "clone_frame",
        "set_palette",
        "add_tag",
        "add_note",
        "list_notes",
        "commit_version",
        "list_versions",
        "restore_version",
        "export_png",
        "export_gif",
        "export_sheet",
        "undo",
        "redo",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
    assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
}

#[test]
fn a_rectangle_drawn_over_mcp_shows_up_in_the_rendered_png() {
    let mut session = McpSession::launch(&[]);
    session.request("initialize", json!({ "protocolVersion": "2025-06-18" }));
    session.call("new_document", json!({ "width": 16, "height": 16 }));
    let drawn = session.call("draw_rect", json!({ "from": [2, 2], "to": [9, 9], "filled": true, "colour": "#ff004d" }));
    assert_eq!(drawn["isError"], false);
    let rendered = session.call("render_png", json!({ "scale": 1 }));
    let image_block =
        rendered["content"].as_array().unwrap().iter().find(|b| b["type"] == "image").expect("an image block");
    assert_eq!(image_block["mimeType"], "image/png");
    let png = base64::engine::general_purpose::STANDARD.decode(image_block["data"].as_str().unwrap()).unwrap();
    let image = spry::io::decode_image(&png).unwrap();
    assert_eq!(image.get(5, 5), RED);
    assert_eq!(image.get(12, 12), CLEAR);
}

#[test]
fn an_agent_can_build_save_and_export_a_whole_animation() {
    let folder = tempfile::tempdir().unwrap();
    let document = folder.path().join("bob.spry");
    let sheet = folder.path().join("bob.png");
    let mut session = McpSession::launch(&[document.to_str().unwrap()]);
    session.request("initialize", json!({}));
    session.call("set_palette", json!({ "name": "Game Boy" }));
    session.call("draw_ellipse", json!({ "from": [2, 4], "to": [12, 14], "filled": true, "colour": "#306230" }));
    session.call("clone_frame", json!({}));
    session.call("add_frame", json!({ "copy": true, "duration_ms": 150 }));
    session.call("run_command", json!({ "command": { "command": "move_floating", "dx": 0, "dy": -1 } }));
    session.call("add_tag", json!({ "name": "bob", "from": 0, "to": 2, "direction": "ping_pong" }));
    let saved = session.call("save_document", json!({}));
    assert_eq!(saved["isError"], false, "{saved}");
    let exported = session.call(
        "export_sheet",
        json!({ "path": sheet.to_str().unwrap(), "layout": "horizontal", "merge_duplicates": true }),
    );
    assert_eq!(exported["isError"], false, "{exported}");

    let reopened = spry::studio::Studio::open(&document).unwrap();
    assert_eq!(reopened.editor.sprite().frame_count(), 3);
    assert_eq!(reopened.editor.sprite().tags[0].name, "bob");
    assert!(sheet.with_extension("json").exists());
}

#[test]
fn tool_mistakes_come_back_as_readable_errors_not_crashes() {
    let mut session = McpSession::launch(&[]);
    session.request("initialize", json!({}));
    let result = session.call("draw_line", json!({ "from": [0, 0], "to": [3, 3], "colour": "not-a-colour" }));
    assert_eq!(result["isError"], true);
    assert!(result["content"][0]["text"].as_str().unwrap().contains("not a hex colour"));
    let still_alive = session.request("ping", json!({}));
    assert_eq!(still_alive["result"], json!({}));
}

#[test]
fn absurd_sizes_from_a_model_are_refused_without_taking_the_machine_down() {
    let mut session = McpSession::launch(&[]);
    session.request("initialize", json!({}));
    let refusals = [
        ("new_document", json!({ "width": 65536, "height": 65536 }), "canvas sizes"),
        ("draw_line", json!({ "from": [0, 0], "to": [2147483647, 2147483647], "colour": "#000000" }), "too far off"),
        (
            "run_command",
            json!({ "command": { "command": "spray", "at": [1, 1], "ink": { "type": "erase" }, "radius": 4, "density": 4294967295u64 } }),
            "density",
        ),
        (
            "run_command",
            json!({ "command": { "command": "resize_canvas", "width": 100000, "height": 4 } }),
            "canvas sizes",
        ),
    ];
    for (tool, arguments, reason) in refusals {
        let result = session.call(tool, arguments.clone());
        assert_eq!(result["isError"], true, "{tool} {arguments} should be refused, got {result}");
        let said = result["content"][0]["text"].as_str().unwrap_or_default();
        assert!(said.contains(reason), "{tool} was refused for the wrong reason: {said}");
    }
    let still_alive = session.request("ping", json!({}));
    assert_eq!(still_alive["result"], json!({}));
}

#[test]
fn the_http_bridge_serves_the_same_tools_to_the_running_app() {
    let bridge = spry::mcp::http::HttpBridge::start(0, || {}).unwrap();
    let url = bridge.url();
    let app = std::thread::spawn(move || {
        let mut studio = spry::studio::Studio::new(8, 8);
        for pending in bridge.messages.iter().take(2) {
            let reply = spry::mcp::handle(&mut studio, &pending.message);
            pending.answer(reply);
        }
        studio
    });
    let post = |body: Value| -> Value { ureq::post(&url).send_json(body).unwrap().body_mut().read_json().unwrap() };
    post(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "fill", "arguments": { "at": [0, 0], "colour": "#29adff" } } }),
    );
    let info = post(
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "document_info", "arguments": {} } }),
    );
    assert!(info["result"]["content"][0]["text"].as_str().unwrap().contains("8×8"));
    let studio = app.join().unwrap();
    assert_eq!(studio.editor.sprite().composite(0).get(7, 7), BLUE, "the app's document changed");
}

#[test]
fn exports_over_mcp_only_ever_write_image_files() {
    let folder = tempfile::tempdir().unwrap();
    let script = folder.path().join("deploy.sh");
    std::fs::write(&script, "echo keep me").unwrap();
    let mut session = McpSession::launch(&[]);
    session.request("initialize", json!({}));
    let result = session.call("export_png", json!({ "path": script.to_str().unwrap() }));
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(std::fs::read_to_string(&script).unwrap(), "echo keep me");
    assert!(folder.path().join("deploy.png").exists());
}

#[test]
fn a_model_can_work_across_layers_and_frames_with_tools_alone() {
    let mut scenario = given_a_blank_sprite(8, 8);
    let ok = |result: Result<spry::mcp::tools::ToolOutput, String>| result.unwrap_or_else(|e| panic!("{e}")).text;

    ok(scenario.tool("fill", json!({ "at": [0, 0], "colour": "#ff004d" })));
    ok(scenario.tool("add_layer", json!({ "name": "Ink" })));
    ok(scenario.tool("set_pixels", json!({ "pixels": [{ "x": 3, "y": 3, "colour": "#000000" }] })));
    ok(scenario.tool("outline", json!({ "colour": "#29adff", "layer": 1 })));
    assert_eq!(scenario.cel_pixel(1, 0, 2, 3), BLUE, "the outline went round the ink, on the ink layer");
    assert_eq!(scenario.cel_pixel(0, 0, 2, 3), RED, "the layer below is untouched");

    ok(scenario.tool("add_frame", json!({ "copy": true, "duration_ms": 250 })));
    assert_eq!(scenario.sprite().frames[1].duration_ms, 250);
    assert_eq!(scenario.cel_pixel(1, 1, 3, 3), BLACK, "a copied frame carries the art");
    ok(scenario.tool("select_frame", json!({ "index": 0 })));
    ok(scenario.tool("select_layer", json!({ "index": 0 })));
    assert_eq!((scenario.studio.editor.layer, scenario.studio.editor.frame), (0, 0));

    ok(scenario.tool("set_palette", json!({ "name": "Duo", "colours": ["#000000", "#ffffff"] })));
    assert_eq!(scenario.sprite().palette.colours.len(), 2);
    let refused = scenario.tool("set_palette", json!({ "colours": ["#000000", "chartreuse"] })).err().unwrap();
    assert!(refused.contains("not a hex colour"), "{refused}");
    let missing = scenario.tool("select_layer", json!({ "index": 9 })).err().unwrap();
    assert!(missing.contains("layer 9"), "{missing}");
}

#[test]
fn a_model_can_group_layers_and_set_blend_modes() {
    let mut scenario = given_a_blank_sprite(8, 8);
    let ok = |result: Result<spry::mcp::tools::ToolOutput, String>| result.unwrap_or_else(|e| panic!("{e}")).text;
    ok(scenario.tool("add_group", json!({ "name": "Lights" })));
    ok(scenario.tool("add_layer", json!({ "name": "Glow" })));
    ok(scenario.tool("set_layer", json!({ "group": 1, "blend": "screen", "opacity": 200 })));
    let glow = &scenario.sprite().layers[1];
    assert_eq!((glow.name.as_str(), glow.blend, glow.opacity), ("Glow", spry::editor::BlendMode::Screen, 200));
    assert_eq!(glow.parent, Some(scenario.sprite().layers[2].id));
    let info = ok(scenario.tool("document_info", json!({})));
    assert!(info.contains("2: group Lights") && info.contains("    1: Glow"), "{info}");
    let refused = scenario.tool("set_layer", json!({ "blend": "sparkle" })).err().unwrap();
    assert!(refused.contains("isn't a blend mode"), "{refused}");
}

#[test]
fn the_document_and_its_frames_can_be_read_as_resources() {
    let mut session = McpSession::launch(&[]);
    let hello = session.request("initialize", json!({}));
    assert!(hello["result"]["capabilities"]["resources"].is_object());
    session.call("fill", json!({ "at": [0, 0], "colour": "#ff004d" }));
    session.call("add_frame", json!({}));

    let listed = session.request("resources/list", json!({}));
    let uris: Vec<&str> =
        listed["result"]["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert_eq!(uris, vec!["spry://document", "spry://frames/0.png", "spry://frames/1.png"]);

    let summary = session.request("resources/read", json!({ "uri": "spry://document" }));
    assert!(summary["result"]["contents"][0]["text"].as_str().unwrap().contains("frames: 2"));

    let frame = session.request("resources/read", json!({ "uri": "spry://frames/0.png" }));
    let blob = frame["result"]["contents"][0]["blob"].as_str().unwrap();
    let image = spry::io::decode_image(&base64::engine::general_purpose::STANDARD.decode(blob).unwrap()).unwrap();
    assert_eq!((image.width(), image.get(3, 3)), (32, RED), "exact pixels at the real size");

    let missing = session.request("resources/read", json!({ "uri": "spry://frames/9.png" }));
    assert_eq!(missing["error"]["code"], -32002);
    let templates = session.request("resources/templates/list", json!({}));
    assert_eq!(templates["result"]["resourceTemplates"][0]["uriTemplate"], "spry://frames/{frame}.png");
}

#[test]
fn a_model_can_build_a_level_from_tiles() {
    let mut scenario = given_a_blank_sprite(32, 16);
    let ok = |result: Result<spry::mcp::tools::ToolOutput, String>| result.unwrap_or_else(|e| panic!("{e}")).text;
    ok(scenario.tool("add_tilemap_layer", json!({ "name": "Ground", "tile_width": 8, "tile_height": 8 })));
    ok(scenario.tool("draw_rect", json!({ "from": [0, 8], "to": [7, 15], "colour": "#008751", "filled": true })));
    for x in [8, 16, 24] {
        ok(scenario.tool("place_tile", json!({ "x": x, "y": 8, "tile": 1 })));
    }
    assert_eq!(
        scenario.composite_pixel(0, 30, 12),
        spry::editor::Rgba::rgb(0, 135, 81),
        "the grass tile runs along the bottom"
    );
    let info = ok(scenario.tool("document_info", json!({})));
    assert!(info.contains("tilemap (tileset 1) Ground") && info.contains("1 tiles of 8×8"), "{info}");
    let path = scenario.path("level.png");
    let wrote = ok(scenario.tool("export_tileset", json!({ "path": path.to_str().unwrap() })));
    assert!(wrote.contains("1 tiles"), "{wrote}");
    let refused = scenario.tool("place_tile", json!({ "x": 0, "y": 0, "tile": 5 })).err().unwrap();
    assert!(refused.contains("tile 5"), "{refused}");
}

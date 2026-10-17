//! Feature 17: diffusion with micro pixel-art models, against scripted
//! local servers that speak the Automatic1111 and ComfyUI APIs.

use crate::support::*;
use base64::Engine;
use serde_json::json;
use spry::ai::diffusion::{self, Backend, Generation, Landing};
use spry::editor::{Command, Palette, Pixels, Rect, Rgba};

/// What a model might paint: a pink blob on a pale backdrop, 256×256.
fn generated_picture() -> Pixels {
    let mut image = Pixels::filled(256, 256, Rgba::rgb(235, 238, 240));
    for (x, y) in Rect::new(64, 64, 128, 128).points() {
        image.set(x, y, Rgba::rgb(250, 30, 90));
    }
    image
}

fn generation(prompt: &str) -> Generation {
    Generation { prompt: prompt.into(), preset: diffusion::presets()[0].clone(), seed: 7, init_image: None }
}

fn pico8_landing(size: u32) -> Landing {
    Landing { width: size, height: size, palette: Palette::pico8(), clear_background: true, dither: false }
}

#[test]
fn an_automatic1111_image_lands_as_a_pico8_layer_on_the_grid() {
    let png = spry::io::encode_png(&generated_picture()).unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    let server = FakeServer::start(move |_, _| Reply::json(json!({ "images": [encoded.clone()] })));

    let mut scenario = given_a_blank_sprite(32, 32);
    let backend = Backend::Automatic1111 { url: server.url.clone() };
    let image = diffusion::generate_sprite(&backend, &generation("a pink slime"), &pico8_landing(32)).unwrap();
    scenario.when(Command::ImportLayer { name: "slime".into(), image });

    assert_eq!(scenario.sprite().layers.len(), 2);
    assert_eq!(scenario.sprite().layers[1].name, "slime");
    let palette = Palette::pico8().colours;
    let layer = scenario.sprite().cel_image(1, 0).unwrap();
    assert!(
        layer.colours().iter().all(|c| c.is_transparent() || palette.contains(c)),
        "every pixel is a PICO-8 colour"
    );
    assert_eq!(layer.get(0, 0), CLEAR, "the backdrop was removed");
    assert!(!layer.get(16, 16).is_transparent(), "the subject survived");

    let request = server.recorded()[0].json();
    assert_eq!(server.recorded()[0].path, "/sdapi/v1/txt2img");
    assert!(request["prompt"].as_str().unwrap().starts_with("a pink slime, pixel art"));
    assert_eq!(request["seed"], 7);
    assert_eq!(request["width"], 512);
}

#[test]
fn img2img_sends_the_current_frame_enlarged() {
    let png = spry::io::encode_png(&generated_picture()).unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    let server = FakeServer::start(move |_, _| Reply::json(json!({ "images": [encoded.clone()] })));
    let backend = Backend::Automatic1111 { url: server.url.clone() };
    let mut request = generation("restyle");
    request.init_image = Some((Pixels::filled(16, 16, RED), 0.5));
    diffusion::generate_sprite(&backend, &request, &pico8_landing(16)).unwrap();
    let sent = server.recorded()[0].clone();
    assert_eq!(sent.path, "/sdapi/v1/img2img");
    let body = sent.json();
    assert_eq!(body["denoising_strength"], 0.5);
    let init = base64::engine::general_purpose::STANDARD.decode(body["init_images"][0].as_str().unwrap()).unwrap();
    assert_eq!(spry::io::decode_image(&init).unwrap().width(), 512);
}

#[test]
fn comfyui_queues_the_workflow_waits_for_history_and_fetches_the_image() {
    let png = spry::io::encode_png(&generated_picture()).unwrap();
    let server = FakeServer::start(move |request, _| {
        if request.path == "/prompt" {
            Reply::json(json!({ "prompt_id": "abc" }))
        } else if request.path.starts_with("/history/abc") {
            Reply::json(
                json!({ "abc": { "outputs": { "9": { "images": [{ "filename": "spry_00001_.png", "subfolder": "", "type": "output" }] } } } }),
            )
        } else {
            Reply::png(png.clone())
        }
    });
    let backend = Backend::ComfyUi { url: server.url.clone(), workflow: None, checkpoint: "pixel.safetensors".into() };
    let image = diffusion::generate_sprite(&backend, &generation("a mushroom"), &pico8_landing(24)).unwrap();
    assert_eq!((image.width(), image.height()), (24, 24));

    let requests = server.recorded();
    let queued = requests[0].json();
    assert_eq!(queued["prompt"]["4"]["inputs"]["ckpt_name"], "pixel.safetensors");
    assert_eq!(queued["prompt"]["3"]["inputs"]["seed"], 7);
    assert!(requests.last().unwrap().path.starts_with("/view?filename=spry_00001_.png"));
}

#[test]
fn an_unreachable_backend_says_what_to_check() {
    let backend = Backend::Automatic1111 { url: "http://127.0.0.1:9".into() };
    let error = diffusion::generate_sprite(&backend, &generation("x"), &pico8_landing(8)).unwrap_err();
    assert!(format!("{error:#}").contains("--api"));
}

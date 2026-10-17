//! Image generation through local Stable Diffusion servers, landed as
//! pixel art.
//!
//! Spry doesn't bundle a model. It talks to the servers pixel artists
//! already run: **Automatic1111 / Forge** (`/sdapi/v1/txt2img`, `/img2img`)
//! and **ComfyUI** (`/prompt`, `/history`, `/view`). Presets carry the
//! prompt wording, step counts and native sizes that suit small pixel-art
//! checkpoints and LoRAs. Whatever comes back goes through
//! `editor::pixelate`, so it lands on your grid and in your palette.

use crate::editor::pixelate::pixelate;
use crate::editor::{Palette, Pixels};
use crate::io::{decode_image, encode_png};
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Backend {
    Automatic1111 {
        url: String,
    },
    ComfyUi {
        url: String,
        /// An API-format workflow with `{{prompt}}`, `{{negative}}`,
        /// `{{seed}}`, `{{steps}}`, `{{cfg}}`, `{{width}}`, `{{height}}` and
        /// `{{checkpoint}}` placeholders. `None` uses Spry's built-in
        /// txt2img graph.
        workflow: Option<String>,
        checkpoint: String,
    },
}

impl Default for Backend {
    fn default() -> Self {
        Backend::Automatic1111 { url: "http://127.0.0.1:7860".into() }
    }
}

/// Generation settings that suit a style of small pixel-art model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub prompt_suffix: String,
    pub negative: String,
    pub steps: u32,
    pub cfg: f32,
    /// The size the model was trained at. Spry generates at this size and
    /// pixelates down to the canvas.
    pub native_size: u32,
}

pub fn presets() -> Vec<Preset> {
    let negative =
        "blurry, soft, anti-aliased, gradient, photo, realistic, 3d render, noise, jpeg artifacts, text, watermark";
    vec![
        Preset {
            name: "Pixel sprite (SD 1.5 pixel checkpoints)".into(),
            prompt_suffix:
                "pixel art, game sprite, 16-bit, crisp pixels, flat colours, clean outline, plain white background"
                    .into(),
            negative: negative.into(),
            steps: 20,
            cfg: 7.0,
            native_size: 512,
        },
        Preset {
            name: "Pixel Art XL LoRA (SDXL)".into(),
            prompt_suffix: "pixel art, <lora:pixel-art-xl:1.0>, game asset, flat shading, plain background".into(),
            negative: negative.into(),
            steps: 24,
            cfg: 6.0,
            native_size: 1024,
        },
        Preset {
            name: "Fast micro model (LCM / Turbo)".into(),
            prompt_suffix: "pixel art sprite, limited palette, plain background".into(),
            negative: negative.into(),
            steps: 6,
            cfg: 1.5,
            native_size: 512,
        },
        Preset {
            name: "Item icon".into(),
            prompt_suffix:
                "pixel art item icon, single object, centred, RPG inventory icon, bold outline, plain background".into(),
            negative: negative.into(),
            steps: 20,
            cfg: 7.0,
            native_size: 512,
        },
        Preset {
            name: "Tileable texture".into(),
            prompt_suffix: "seamless tileable pixel art texture, top-down game tile, flat colours".into(),
            negative: negative.into(),
            steps: 20,
            cfg: 7.0,
            native_size: 512,
        },
    ]
}

#[derive(Clone, Debug, PartialEq)]
pub struct Generation {
    pub prompt: String,
    pub preset: Preset,
    pub seed: i64,
    /// For img2img: the current frame, and how far to move away from it
    /// (0 = keep, 1 = ignore).
    pub init_image: Option<(Pixels, f32)>,
}

/// What to do with the generated picture once it arrives.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    pub width: u32,
    pub height: u32,
    pub palette: Palette,
    pub clear_background: bool,
    pub dither: bool,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).build().into()
}

fn base64_png(pixels: &Pixels) -> Result<String> {
    Ok(base64::engine::general_purpose::STANDARD.encode(encode_png(pixels)?))
}

/// Asks the backend for a picture and returns it as generated, at the
/// model's native size.
pub fn generate_raw(backend: &Backend, generation: &Generation) -> Result<Pixels> {
    match backend {
        Backend::Automatic1111 { url } => automatic1111(url.trim_end_matches('/'), generation),
        Backend::ComfyUi { url, workflow, checkpoint } => {
            comfyui(url.trim_end_matches('/'), workflow.as_deref(), checkpoint, generation)
        }
    }
}

/// Generates and pixelates in one go: the result is ready to become a
/// layer.
pub fn generate_sprite(backend: &Backend, generation: &Generation, landing: &Landing) -> Result<Pixels> {
    let raw = generate_raw(backend, generation)?;
    Ok(pixelate(&raw, landing.width, landing.height, &landing.palette, landing.clear_background, landing.dither))
}

fn full_prompt(generation: &Generation) -> String {
    format!("{}, {}", generation.prompt.trim(), generation.preset.prompt_suffix)
}

/// The size to generate at: the preset's native size, keeping the
/// canvas's aspect ratio, rounded to multiples of 64 as the models expect.
pub fn generation_size(preset: &Preset, canvas_width: u32, canvas_height: u32) -> (u32, u32) {
    let longest = canvas_width.max(canvas_height).max(1) as f32;
    let fit = |side: u32| (((side as f32 / longest) * preset.native_size as f32 / 64.0).round() as u32).max(1) * 64;
    (fit(canvas_width), fit(canvas_height))
}

fn automatic1111(url: &str, generation: &Generation) -> Result<Pixels> {
    let preset = &generation.preset;
    let (endpoint, mut body) = match &generation.init_image {
        Some((init, strength)) => {
            let (width, height) = generation_size(preset, init.width(), init.height());
            let enlarged = init.scaled_up((width / init.width()).max(1));
            let body = json!({ "init_images": [base64_png(&enlarged)?], "denoising_strength": strength, "width": width, "height": height });
            ("img2img", body)
        }
        None => ("txt2img", json!({ "width": preset.native_size, "height": preset.native_size })),
    };
    body["prompt"] = json!(full_prompt(generation));
    body["negative_prompt"] = json!(preset.negative);
    body["steps"] = json!(preset.steps);
    body["cfg_scale"] = json!(preset.cfg);
    body["seed"] = json!(generation.seed);
    let reply: Value = agent()
        .post(&format!("{url}/sdapi/v1/{endpoint}"))
        .send_json(&body)
        .with_context(|| format!("asking Automatic1111/Forge at {url} for an image (is it running with --api?)"))?
        .body_mut()
        .read_json()
        .context("Automatic1111 replied with something that isn't JSON")?;
    let encoded = reply["images"][0].as_str().context("Automatic1111 returned no image")?;
    decode_base64_image(encoded)
}

fn decode_base64_image(encoded: &str) -> Result<Pixels> {
    // Some servers prefix a data URL header.
    let payload = encoded.split_once("base64,").map_or(encoded, |(_, data)| data);
    let bytes =
        base64::engine::general_purpose::STANDARD.decode(payload.trim()).context("the image wasn't valid base64")?;
    decode_image(&bytes)
}

/// Spry's built-in ComfyUI graph: checkpoint → prompts → empty latent →
/// KSampler → VAE decode → save.
pub const DEFAULT_COMFY_WORKFLOW: &str = r#"{
  "4": { "class_type": "CheckpointLoaderSimple", "inputs": { "ckpt_name": "{{checkpoint}}" } },
  "6": { "class_type": "CLIPTextEncode", "inputs": { "text": "{{prompt}}", "clip": ["4", 1] } },
  "7": { "class_type": "CLIPTextEncode", "inputs": { "text": "{{negative}}", "clip": ["4", 1] } },
  "5": { "class_type": "EmptyLatentImage", "inputs": { "width": "{{width}}", "height": "{{height}}", "batch_size": 1 } },
  "3": { "class_type": "KSampler", "inputs": { "seed": "{{seed}}", "steps": "{{steps}}", "cfg": "{{cfg}}", "sampler_name": "euler", "scheduler": "normal", "denoise": 1.0, "model": ["4", 0], "positive": ["6", 0], "negative": ["7", 0], "latent_image": ["5", 0] } },
  "8": { "class_type": "VAEDecode", "inputs": { "samples": ["3", 0], "vae": ["4", 2] } },
  "9": { "class_type": "SaveImage", "inputs": { "filename_prefix": "spry", "images": ["8", 0] } }
}"#;

/// Fills a workflow's placeholders. A string that is exactly a
/// placeholder becomes a typed JSON value (so `"{{steps}}"` becomes the
/// number 20), and placeholders inside longer text are substituted as
/// text.
pub fn fill_workflow(template: &str, values: &[(&str, Value)]) -> Result<Value> {
    let mut workflow: Value = serde_json::from_str(template)
        .context("the ComfyUI workflow isn't valid JSON (export it with \"Save (API format)\")")?;
    fill_placeholders(&mut workflow, values);
    Ok(workflow)
}

fn fill_placeholders(node: &mut Value, values: &[(&str, Value)]) {
    match node {
        Value::String(text) => {
            if let Some((_, value)) = values.iter().find(|(key, _)| *text == format!("{{{{{key}}}}}")) {
                *node = value.clone();
                return;
            }
            for (key, value) in values {
                let as_text = value.as_str().map_or_else(|| value.to_string(), str::to_string);
                *text = text.replace(&format!("{{{{{key}}}}}"), &as_text);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| fill_placeholders(item, values)),
        Value::Object(fields) => fields.values_mut().for_each(|field| fill_placeholders(field, values)),
        _ => {}
    }
}

fn comfyui(url: &str, workflow: Option<&str>, checkpoint: &str, generation: &Generation) -> Result<Pixels> {
    let preset = &generation.preset;
    let values = [
        ("prompt", json!(full_prompt(generation))),
        ("negative", json!(preset.negative)),
        ("seed", json!(generation.seed)),
        ("steps", json!(preset.steps)),
        ("cfg", json!(preset.cfg)),
        ("width", json!(preset.native_size)),
        ("height", json!(preset.native_size)),
        ("checkpoint", json!(checkpoint)),
    ];
    let graph = fill_workflow(workflow.unwrap_or(DEFAULT_COMFY_WORKFLOW), &values)?;
    let http = agent();
    let queued: Value = http
        .post(&format!("{url}/prompt"))
        .send_json(json!({ "prompt": graph, "client_id": "spry" }))
        .with_context(|| format!("queueing the workflow on ComfyUI at {url}"))?
        .body_mut()
        .read_json()?;
    let prompt_id = queued["prompt_id"].as_str().context("ComfyUI didn't return a prompt_id")?.to_string();

    let started = Instant::now();
    let image_ref = loop {
        let history: Value = http.get(&format!("{url}/history/{prompt_id}")).call()?.body_mut().read_json()?;
        if let Some(found) = first_output_image(&history[&prompt_id]) {
            break found;
        }
        if started.elapsed() > Duration::from_secs(600) {
            bail!("ComfyUI didn't finish within 10 minutes");
        }
        std::thread::sleep(Duration::from_millis(400));
    };
    let bytes = http
        .get(&format!("{url}/view"))
        .query("filename", &image_ref.0)
        .query("subfolder", &image_ref.1)
        .query("type", &image_ref.2)
        .call()?
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_vec()?;
    decode_image(&bytes)
}

/// The (filename, subfolder, type) of the first image in a ComfyUI
/// history entry, once the run has finished.
fn first_output_image(entry: &Value) -> Option<(String, String, String)> {
    entry["outputs"].as_object()?.values().find_map(|output| {
        let image = output["images"].as_array()?.first()?;
        Some((
            image["filename"].as_str()?.to_string(),
            image["subfolder"].as_str().unwrap_or("").to_string(),
            image["type"].as_str().unwrap_or("output").to_string(),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_size_keeps_aspect_in_multiples_of_64() {
        let preset = &presets()[0];
        assert_eq!(generation_size(preset, 32, 32), (512, 512));
        assert_eq!(generation_size(preset, 64, 32), (512, 256));
    }

    #[test]
    fn workflow_placeholders_become_typed_values() {
        let workflow = fill_workflow(
            r#"{"a":{"inputs":{"steps":"{{steps}}","text":"cat, {{prompt}}"}}}"#,
            &[("steps", json!(12)), ("prompt", json!("knight"))],
        )
        .unwrap();
        assert_eq!(workflow["a"]["inputs"]["steps"], 12);
        assert_eq!(workflow["a"]["inputs"]["text"], "cat, knight");
    }

    #[test]
    fn the_default_workflow_is_valid_once_filled() {
        let workflow = fill_workflow(DEFAULT_COMFY_WORKFLOW, &[("seed", json!(5)), ("width", json!(512))]).unwrap();
        assert_eq!(workflow["3"]["inputs"]["seed"], 5);
        assert_eq!(workflow["5"]["inputs"]["width"], 512);
    }

    #[test]
    fn finished_comfy_history_yields_its_image() {
        let entry = json!({ "outputs": { "9": { "images": [{ "filename": "spry_0001.png", "subfolder": "", "type": "output" }] } } });
        assert_eq!(first_output_image(&entry), Some(("spry_0001.png".into(), "".into(), "output".into())));
        assert_eq!(first_output_image(&json!({})), None);
    }

    #[test]
    fn data_url_prefixes_are_tolerated() {
        let png = encode_png(&Pixels::new(2, 2)).unwrap();
        let encoded = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png));
        assert_eq!(decode_base64_image(&encoded).unwrap(), Pixels::new(2, 2));
    }
}

//! Image generation through local Stable Diffusion servers, landed as
//! pixel art.
//!
//! Spry doesn't bundle a model. It talks to the servers pixel artists
//! already run: **Automatic1111 / Forge** (`/sdapi/v1/txt2img`, `/img2img`)
//! and **ComfyUI** (`/prompt`, `/history`, `/view`). Presets carry the
//! prompt wording, step counts and native sizes that suit small pixel-art
//! checkpoints and LoRAs. Whatever comes back goes through
//! `editor::pixelate`, so it lands on your grid and in your palette.

use crate::editor::pixelate::{pixelate, pixelate_subject, Fit};
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
        /// `{{checkpoint}}` placeholders, plus `{{init_image}}` and
        /// `{{denoise}}` if it can restyle a frame, and `{{lora}}` and
        /// `{{lora_weight}}` if it loads a LoRA. `None` uses Spry's
        /// built-in graphs.
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
    /// The LoRA this preset is meant for, offered as the form's default.
    #[serde(default)]
    pub suggested_lora: Option<String>,
}

/// A LoRA (a small add-on network, which is how most pixel-art styles
/// ship) and how strongly to apply it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lora {
    pub name: String,
    pub weight: f32,
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
            suggested_lora: None,
        },
        Preset {
            name: "Pixel Art XL LoRA (SDXL)".into(),
            prompt_suffix: "pixel art, game asset, flat shading, plain background".into(),
            negative: negative.into(),
            steps: 24,
            cfg: 6.0,
            native_size: 1024,
            suggested_lora: Some("pixel-art-xl".into()),
        },
        Preset {
            name: "Fast micro model (LCM / Turbo)".into(),
            prompt_suffix: "pixel art sprite, limited palette, plain background".into(),
            negative: negative.into(),
            steps: 6,
            cfg: 1.5,
            native_size: 512,
            suggested_lora: None,
        },
        Preset {
            name: "Item icon".into(),
            prompt_suffix:
                "pixel art item icon, single object, centred, RPG inventory icon, bold outline, plain background".into(),
            negative: negative.into(),
            steps: 20,
            cfg: 7.0,
            native_size: 512,
            suggested_lora: None,
        },
        Preset {
            name: "Tileable texture".into(),
            prompt_suffix: "seamless tileable pixel art texture, top-down game tile, flat colours".into(),
            negative: negative.into(),
            steps: 20,
            cfg: 7.0,
            native_size: 512,
            suggested_lora: None,
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
    /// The canvas the result will land on, so the model paints the same
    /// shape instead of a square that gets squashed.
    pub canvas: (u32, u32),
    pub lora: Option<Lora>,
}

/// What to do with the generated picture once it arrives.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    pub width: u32,
    pub height: u32,
    pub palette: Palette,
    pub clear_background: bool,
    pub dither: bool,
    pub fit: Fit,
}

impl Landing {
    /// Runs a picture through the pixelise pipeline onto the grid and
    /// palette, which is how both generated and imported pictures arrive.
    pub fn land(&self, picture: &Pixels) -> Pixels {
        match self.fit {
            Fit::Canvas => {
                pixelate(picture, self.width, self.height, &self.palette, self.clear_background, self.dither)
            }
            Fit::Subject => pixelate_subject(picture, self.width, self.height, &self.palette, self.dither),
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).build().into()
}

/// An `f32` setting as the decimal the user typed. Widening 0.8f32 to JSON
/// directly would send 0.800000011920929.
fn tidy(value: f32) -> f64 {
    (f64::from(value) * 1000.0).round() / 1000.0
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
    Ok(landing.land(&raw))
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
    let (width, height) = generation_size(preset, generation.canvas.0, generation.canvas.1);
    let (endpoint, mut body) = match &generation.init_image {
        Some((init, strength)) => {
            let (width, height) = generation_size(preset, init.width(), init.height());
            let enlarged = init.scaled_up((width / init.width()).max(1));
            let body = json!({ "init_images": [base64_png(&enlarged)?], "denoising_strength": tidy(*strength), "width": width, "height": height });
            ("img2img", body)
        }
        None => ("txt2img", json!({ "width": width, "height": height })),
    };
    // Automatic1111 loads LoRAs from a tag inside the prompt.
    let lora_tag = generation.lora.as_ref().map(|lora| format!(", <lora:{}:{}>", lora.name, tidy(lora.weight)));
    body["prompt"] = json!(full_prompt(generation) + lora_tag.as_deref().unwrap_or(""));
    body["negative_prompt"] = json!(preset.negative);
    body["steps"] = json!(preset.steps);
    body["cfg_scale"] = json!(tidy(preset.cfg));
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

/// Spry's built-in ComfyUI txt2img graph: checkpoint → prompts → empty
/// latent → KSampler → VAE decode → save.
pub const DEFAULT_COMFY_WORKFLOW: &str = r#"{
  "4": { "class_type": "CheckpointLoaderSimple", "inputs": { "ckpt_name": "{{checkpoint}}" } },
  "6": { "class_type": "CLIPTextEncode", "inputs": { "text": "{{prompt}}", "clip": ["4", 1] } },
  "7": { "class_type": "CLIPTextEncode", "inputs": { "text": "{{negative}}", "clip": ["4", 1] } },
  "5": { "class_type": "EmptyLatentImage", "inputs": { "width": "{{width}}", "height": "{{height}}", "batch_size": 1 } },
  "3": { "class_type": "KSampler", "inputs": { "seed": "{{seed}}", "steps": "{{steps}}", "cfg": "{{cfg}}", "sampler_name": "euler", "scheduler": "normal", "denoise": 1.0, "model": ["4", 0], "positive": ["6", 0], "negative": ["7", 0], "latent_image": ["5", 0] } },
  "8": { "class_type": "VAEDecode", "inputs": { "samples": ["3", 0], "vae": ["4", 2] } },
  "9": { "class_type": "SaveImage", "inputs": { "filename_prefix": "spry", "images": ["8", 0] } }
}"#;

/// The built-in img2img graph: the uploaded frame is encoded into the
/// latent the sampler starts from, and `denoise` sets how far it strays.
pub const DEFAULT_COMFY_IMG2IMG_WORKFLOW: &str = r#"{
  "4": { "class_type": "CheckpointLoaderSimple", "inputs": { "ckpt_name": "{{checkpoint}}" } },
  "6": { "class_type": "CLIPTextEncode", "inputs": { "text": "{{prompt}}", "clip": ["4", 1] } },
  "7": { "class_type": "CLIPTextEncode", "inputs": { "text": "{{negative}}", "clip": ["4", 1] } },
  "11": { "class_type": "LoadImage", "inputs": { "image": "{{init_image}}" } },
  "12": { "class_type": "VAEEncode", "inputs": { "pixels": ["11", 0], "vae": ["4", 2] } },
  "3": { "class_type": "KSampler", "inputs": { "seed": "{{seed}}", "steps": "{{steps}}", "cfg": "{{cfg}}", "sampler_name": "euler", "scheduler": "normal", "denoise": "{{denoise}}", "model": ["4", 0], "positive": ["6", 0], "negative": ["7", 0], "latent_image": ["12", 0] } },
  "8": { "class_type": "VAEDecode", "inputs": { "samples": ["3", 0], "vae": ["4", 2] } },
  "9": { "class_type": "SaveImage", "inputs": { "filename_prefix": "spry", "images": ["8", 0] } }
}"#;

/// Puts a LoraLoader between the checkpoint (node "4") and everything
/// that read its model or CLIP, which is how ComfyUI applies a LoRA. Only
/// used on Spry's own graphs, whose node ids are known.
fn insert_lora(graph: &mut Value, lora: &Lora) {
    const LORA_NODE: &str = "10";
    let Some(nodes) = graph.as_object_mut() else { return };
    for node in nodes.values_mut() {
        let Some(inputs) = node["inputs"].as_object_mut() else { continue };
        for (name, input) in inputs.iter_mut() {
            let reads_checkpoint = input[0] == "4" && (input[1] == 0 || input[1] == 1);
            if reads_checkpoint && (name == "model" || name == "clip") {
                input[0] = json!(LORA_NODE);
            }
        }
    }
    nodes.insert(
        LORA_NODE.into(),
        json!({ "class_type": "LoraLoader", "inputs": {
            "lora_name": lora.name, "strength_model": tidy(lora.weight), "strength_clip": tidy(lora.weight),
            "model": ["4", 0], "clip": ["4", 1]
        } }),
    );
}

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
    let http = agent();
    let graph = comfy_graph(&http, url, workflow, checkpoint, generation)?;
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

/// The filled-in graph to queue: the user's workflow or Spry's own, with
/// the frame uploaded first when restyling.
fn comfy_graph(
    http: &ureq::Agent,
    url: &str,
    workflow: Option<&str>,
    checkpoint: &str,
    generation: &Generation,
) -> Result<Value> {
    let preset = &generation.preset;
    let (width, height) = generation_size(preset, generation.canvas.0, generation.canvas.1);
    let mut values = vec![
        ("prompt", json!(full_prompt(generation))),
        ("negative", json!(preset.negative)),
        ("seed", json!(generation.seed)),
        ("steps", json!(preset.steps)),
        ("cfg", json!(tidy(preset.cfg))),
        ("width", json!(width)),
        ("height", json!(height)),
        ("checkpoint", json!(checkpoint)),
    ];
    if let Some(lora) = &generation.lora {
        values.extend([("lora", json!(lora.name)), ("lora_weight", json!(tidy(lora.weight)))]);
    }
    let template = match (workflow, &generation.init_image) {
        (Some(custom), Some(_)) if !custom.contains("{{init_image}}") => {
            bail!("your ComfyUI workflow has no {{{{init_image}}}} placeholder, so it can't restyle a frame")
        }
        (Some(custom), _) => custom,
        (None, Some(_)) => DEFAULT_COMFY_IMG2IMG_WORKFLOW,
        (None, None) => DEFAULT_COMFY_WORKFLOW,
    };
    if let Some((frame, strength)) = &generation.init_image {
        let enlarged = frame.scaled_up((width / frame.width().max(1)).max(1));
        values.extend([
            ("init_image", json!(upload_to_comfyui(http, url, &enlarged)?)),
            ("denoise", json!(tidy(*strength))),
        ]);
    }
    let mut graph = fill_workflow(template, &values)?;
    if let (None, Some(lora)) = (workflow, &generation.lora) {
        insert_lora(&mut graph, lora);
    }
    Ok(graph)
}

/// Uploads a picture into ComfyUI's input folder (a multipart form post,
/// written out by hand since it's one small file) and returns the name
/// `LoadImage` should ask for.
fn upload_to_comfyui(http: &ureq::Agent, url: &str, image: &Pixels) -> Result<String> {
    const BOUNDARY: &str = "spry-upload-7c1f9a";
    let mut body = Vec::new();
    body.extend_from_slice(format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"spry-init.png\"\r\nContent-Type: image/png\r\n\r\n"
    ).as_bytes());
    body.extend_from_slice(&encode_png(image)?);
    body.extend_from_slice(
        format!(
            "\r\n--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"overwrite\"\r\n\r\ntrue\r\n--{BOUNDARY}--\r\n"
        )
        .as_bytes(),
    );
    let reply: Value = http
        .post(&format!("{url}/upload/image"))
        .header("Content-Type", &format!("multipart/form-data; boundary={BOUNDARY}"))
        .send(&body[..])
        .with_context(|| format!("uploading the frame to ComfyUI at {url}"))?
        .body_mut()
        .read_json()
        .context("ComfyUI's upload reply wasn't JSON")?;
    let name = reply["name"].as_str().context("ComfyUI didn't say what it called the upload")?;
    let subfolder = reply["subfolder"].as_str().unwrap_or("");
    Ok(if subfolder.is_empty() { name.to_string() } else { format!("{subfolder}/{name}") })
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

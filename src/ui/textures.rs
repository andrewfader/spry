//! GPU textures for sprite images, re-uploaded only when the image actually
//! changed. Each texture is keyed by name and tagged with the editor
//! revision it was made from.

use crate::editor::{Pixels, Rgba};
use egui::{ColorImage, TextureHandle, TextureOptions, TextureWrapMode};
use std::collections::HashMap;

#[derive(Default)]
pub struct TextureCache {
    entries: HashMap<String, (u64, TextureHandle)>,
}

pub fn pixel_options() -> TextureOptions {
    TextureOptions {
        magnification: egui::TextureFilter::Nearest,
        minification: egui::TextureFilter::Nearest,
        ..TextureOptions::NEAREST
    }
}

pub fn to_color_image(pixels: &Pixels) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([pixels.width() as usize, pixels.height() as usize], &pixels.to_rgba_bytes())
}

impl TextureCache {
    /// The texture called `key`, rebuilt by `make` when `revision` moved on.
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        revision: u64,
        make: impl FnOnce() -> Pixels,
    ) -> &TextureHandle {
        let stale = self.entries.get(key).is_none_or(|(made_at, _)| *made_at != revision);
        if stale {
            let image = to_color_image(&make());
            match self.entries.get_mut(key) {
                Some((made_at, handle)) => {
                    handle.set(image, pixel_options());
                    *made_at = revision;
                }
                None => {
                    let handle = ctx.load_texture(key, image, pixel_options());
                    self.entries.insert(key.to_string(), (revision, handle));
                }
            }
        }
        &self.entries[key].1
    }

    /// Drops textures whose key starts with `prefix`, e.g. when a document
    /// closes.
    pub fn forget(&mut self, prefix: &str) {
        self.entries.retain(|key, _| !key.starts_with(prefix));
    }

    /// A tiny repeating checkerboard, drawn behind transparent pixels.
    pub fn checkerboard(&mut self, ctx: &egui::Context, light: egui::Color32, dark: egui::Color32) -> &TextureHandle {
        let key = format!("checker:{light:?}:{dark:?}");
        if !self.entries.contains_key(&key) {
            let as_rgba = |c: egui::Color32| Rgba::rgb(c.r(), c.g(), c.b());
            let mut tile = Pixels::filled(2, 2, as_rgba(light));
            tile.set(1, 0, as_rgba(dark));
            tile.set(0, 1, as_rgba(dark));
            let options = TextureOptions { wrap_mode: TextureWrapMode::Repeat, ..pixel_options() };
            let handle = ctx.load_texture(&key, to_color_image(&tile), options);
            self.entries.insert(key.clone(), (0, handle));
        }
        &self.entries[&key].1
    }
}

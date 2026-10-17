//! Frosted glass: a real blur of whatever is behind each panel.
//!
//! egui draws flat translucent shapes and can't blur. So every glass panel
//! starts with a paint callback that runs on the GPU in the middle of
//! egui's frame, after everything beneath the panel has been drawn. It
//! copies that region of the framebuffer, blurs it at half resolution with
//! a separable Gaussian (two passes, run twice for a soft, wide blur),
//! and draws it back clipped to the panel's rounded corners. The panel's
//! translucent tint and border then draw on top, as usual. That is the
//! Apple-style "material": the canvas and backdrop show through, softened.
//!
//! All of this needs GLSL 1.40 / ES 3.00. On older drivers the callback
//! does nothing and panels stay plain translucent glass.

use eframe::egui_glow::{self, glow, ShaderVersion};
use glow::HasContext as _;
use std::sync::{Arc, Mutex};

/// Shared GPU objects, made on first use inside a callback (the only place
/// a GL context is available) and reused every frame.
#[derive(Clone, Default)]
pub struct GlassBlur {
    renderer: Arc<Mutex<Option<Renderer>>>,
}

/// A rectangle of the framebuffer in physical pixels, measured from the
/// bottom-left as OpenGL does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRegion {
    pub left: i32,
    pub bottom: i32,
    pub width: i32,
    pub height: i32,
}

impl PixelRegion {
    /// The part of a panel's rect (in points) that's actually on screen.
    pub fn of(rect: egui::Rect, pixels_per_point: f32, screen_px: [u32; 2]) -> Option<PixelRegion> {
        let [screen_width, screen_height] = screen_px.map(|side| side as f32);
        let left = (rect.min.x * pixels_per_point).round().max(0.0);
        let right = (rect.max.x * pixels_per_point).round().min(screen_width);
        let top = (rect.min.y * pixels_per_point).round().max(0.0);
        let bottom_edge = (rect.max.y * pixels_per_point).round().min(screen_height);
        let (width, height) = (right - left, bottom_edge - top);
        (width >= 2.0 && height >= 2.0).then_some(PixelRegion {
            left: left as i32,
            bottom: (screen_height - bottom_edge) as i32,
            width: width as i32,
            height: height as i32,
        })
    }

    /// The half-size working buffer: blurring there is four times cheaper
    /// and widens the blur for free.
    fn half(self) -> (i32, i32) {
        ((self.width / 2).max(1), (self.height / 2).max(1))
    }
}

/// Weights and offsets of a 9-tap Gaussian folded into 5 linear-filtered
/// reads (each off-centre read lands between two texels and the GPU's
/// bilinear filter mixes them in the right proportion).
pub const GAUSSIAN_WEIGHTS: [f32; 3] = [0.227_027_03, 0.316_216_22, 0.070_270_27];
pub const GAUSSIAN_OFFSETS: [f32; 3] = [0.0, 1.384_615_4, 3.230_769_2];

impl GlassBlur {
    /// A shape to put first in a panel's layer: the blurred backdrop of
    /// `rect`, with corners rounded by `radius` points.
    pub fn backdrop(&self, rect: egui::Rect, radius: f32) -> egui::Shape {
        let renderer = Arc::clone(&self.renderer);
        let callback = egui_glow::CallbackFn::new(move |info, painter| {
            let gl = painter.gl();
            let Some(region) = PixelRegion::of(rect, info.pixels_per_point, info.screen_size_px) else { return };
            let Ok(mut slot) = renderer.lock() else { return };
            if slot.is_none() {
                *slot = Renderer::new(gl);
            }
            if let Some(renderer) = slot.as_mut() {
                let clip = PixelRegion::of(info.clip_rect, info.pixels_per_point, info.screen_size_px);
                // SAFETY: called by egui_glow on the GL thread with the
                // context current; egui restores its own state afterwards.
                unsafe { renderer.draw(gl, region, clip, radius * info.pixels_per_point) };
            }
        });
        egui::Shape::Callback(egui::PaintCallback { rect, callback: Arc::new(callback) })
    }
}

struct Renderer {
    blur: glow::Program,
    composite: glow::Program,
    vertex_array: glow::VertexArray,
    capture: glow::Texture,
    ping: (glow::Framebuffer, glow::Texture),
    pong: (glow::Framebuffer, glow::Texture),
    working_size: (i32, i32),
}

const VERTEX: &str = r#"
const vec2 CORNERS[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
out vec2 v_uv;
void main() {
    vec2 p = CORNERS[gl_VertexID];
    v_uv = p * 0.5 + 0.5;
    gl_Position = vec4(p, 0.0, 1.0);
}
"#;

/// The blur pass, written from the weights above so the two can't drift.
fn blur_shader() -> String {
    let [w0, w1, w2] = GAUSSIAN_WEIGHTS;
    let [_, o1, o2] = GAUSSIAN_OFFSETS;
    format!(
        "uniform sampler2D u_image;
uniform vec2 u_step;
in vec2 v_uv;
out vec4 colour;
void main() {{
    colour = texture(u_image, v_uv) * {w0:.8};
    colour += (texture(u_image, v_uv + u_step * {o1:.7}) + texture(u_image, v_uv - u_step * {o1:.7})) * {w1:.8};
    colour += (texture(u_image, v_uv + u_step * {o2:.7}) + texture(u_image, v_uv - u_step * {o2:.7})) * {w2:.8};
}}
"
    )
}

const COMPOSITE: &str = r#"
uniform sampler2D u_image;
uniform vec2 u_size;
uniform float u_radius;
in vec2 v_uv;
out vec4 colour;
// Signed distance to a rounded rectangle centred on the origin.
float rounded_box(vec2 p, vec2 half_size, float radius) {
    vec2 q = abs(p) - half_size + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}
void main() {
    float distance = rounded_box((v_uv - 0.5) * u_size, u_size * 0.5, u_radius);
    float coverage = clamp(0.5 - distance, 0.0, 1.0);
    colour = vec4(texture(u_image, v_uv).rgb * coverage, coverage);
}
"#;

impl Renderer {
    fn new(gl: &glow::Context) -> Option<Renderer> {
        let version = ShaderVersion::get(gl);
        if !version.is_new_shader_interface() {
            eprintln!("glass blur needs GLSL 1.40 or ES 3.00; panels stay plain glass");
            return None;
        }
        // SAFETY: the context is current (we're inside a paint callback).
        unsafe {
            let blur = compile(gl, version, &blur_shader())?;
            let composite = compile(gl, version, COMPOSITE)?;
            Some(Renderer {
                blur,
                composite,
                vertex_array: gl.create_vertex_array().ok()?,
                capture: linear_texture(gl)?,
                ping: target(gl)?,
                pong: target(gl)?,
                working_size: (0, 0),
            })
        }
    }

    unsafe fn draw(&mut self, gl: &glow::Context, region: PixelRegion, clip: Option<PixelRegion>, radius_px: f32) {
        let PixelRegion { left, bottom, width, height } = region;
        let drawing_into = current_framebuffer(gl);
        unsafe {
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(Some(self.vertex_array));

            // 1. Copy what's behind the panel.
            gl.bind_texture(glow::TEXTURE_2D, Some(self.capture));
            gl.copy_tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8, left, bottom, width, height, 0);

            // 2. Blur at half size: across, down, across, down.
            let (half_width, half_height) = region.half();
            self.resize_working(gl, (half_width, half_height));
            gl.viewport(0, 0, half_width, half_height);
            gl.use_program(Some(self.blur));
            let step = gl.get_uniform_location(self.blur, "u_step");
            let image = gl.get_uniform_location(self.blur, "u_image");
            gl.uniform_1_i32(image.as_ref(), 0);
            let (texel_x, texel_y) = (1.0 / half_width as f32, 1.0 / half_height as f32);
            let mut source = self.capture;
            for round in 0..2 {
                let spread = 1.0 + round as f32;
                for (target, direction) in [(self.ping, (texel_x * spread, 0.0)), (self.pong, (0.0, texel_y * spread))]
                {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.0));
                    gl.bind_texture(glow::TEXTURE_2D, Some(source));
                    gl.uniform_2_f32(step.as_ref(), direction.0, direction.1);
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                    source = target.1;
                }
            }

            // 3. Draw it back into the panel, rounded, inside its clip.
            gl.bind_framebuffer(glow::FRAMEBUFFER, drawing_into);
            gl.viewport(left, bottom, width, height);
            if let Some(clip) = clip {
                gl.enable(glow::SCISSOR_TEST);
                gl.scissor(clip.left, clip.bottom, clip.width, clip.height);
            }
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.use_program(Some(self.composite));
            gl.uniform_1_i32(gl.get_uniform_location(self.composite, "u_image").as_ref(), 0);
            gl.uniform_2_f32(gl.get_uniform_location(self.composite, "u_size").as_ref(), width as f32, height as f32);
            gl.uniform_1_f32(gl.get_uniform_location(self.composite, "u_radius").as_ref(), radius_px);
            gl.bind_texture(glow::TEXTURE_2D, Some(source));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
        }
    }

    unsafe fn resize_working(&mut self, gl: &glow::Context, size: (i32, i32)) {
        if self.working_size == size {
            return;
        }
        for (_, texture) in [self.ping, self.pong] {
            unsafe {
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    size.0,
                    size.1,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
            }
        }
        self.working_size = size;
    }
}

/// The framebuffer egui is drawing into, so we can go back to it.
unsafe fn current_framebuffer(gl: &glow::Context) -> Option<glow::Framebuffer> {
    let id = unsafe { gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING) };
    std::num::NonZeroU32::new(id as u32).map(glow::NativeFramebuffer)
}

unsafe fn compile(gl: &glow::Context, version: ShaderVersion, fragment: &str) -> Option<glow::Program> {
    let precision = if version.is_embedded() { "precision mediump float;\n" } else { "" };
    let header = format!("{}{precision}", version.version_declaration());
    unsafe {
        let program = gl.create_program().ok()?;
        let mut shaders = Vec::new();
        for (kind, body) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, fragment)] {
            let shader = gl.create_shader(kind).ok()?;
            gl.shader_source(shader, &format!("{header}{body}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                eprintln!("glass blur shader didn't compile: {}", gl.get_shader_info_log(shader));
                return None;
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            eprintln!("glass blur shader didn't link: {}", gl.get_program_info_log(program));
            return None;
        }
        Some(program)
    }
}

unsafe fn linear_texture(gl: &glow::Context) -> Option<glow::Texture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        for (parameter, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, value as i32);
        }
        Some(texture)
    }
}

/// A texture with a framebuffer drawing into it.
unsafe fn target(gl: &glow::Context) -> Option<(glow::Framebuffer, glow::Texture)> {
    unsafe {
        let texture = linear_texture(gl)?;
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            1,
            1,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        let framebuffer = gl.create_framebuffer().ok()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), 0);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        Some((framebuffer, texture))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_maps_to_framebuffer_pixels_from_the_bottom() {
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 70.0));
        let region = PixelRegion::of(rect, 2.0, [800, 600]).unwrap();
        assert_eq!(region, PixelRegion { left: 20, bottom: 600 - 140, width: 200, height: 100 });
    }

    #[test]
    fn panels_hanging_off_screen_are_clipped_and_vanished_ones_skipped() {
        let half_off = egui::Rect::from_min_max(egui::pos2(-50.0, -10.0), egui::pos2(50.0, 40.0));
        assert_eq!(
            PixelRegion::of(half_off, 1.0, [300, 200]),
            Some(PixelRegion { left: 0, bottom: 160, width: 50, height: 40 })
        );
        let gone = egui::Rect::from_min_max(egui::pos2(400.0, 10.0), egui::pos2(500.0, 40.0));
        assert_eq!(PixelRegion::of(gone, 1.0, [300, 200]), None);
    }

    #[test]
    fn the_gaussian_keeps_brightness() {
        let total = GAUSSIAN_WEIGHTS[0] + 2.0 * (GAUSSIAN_WEIGHTS[1] + GAUSSIAN_WEIGHTS[2]);
        assert!((total - 1.0).abs() < 1e-5, "weights sum to {total}");
    }

    #[test]
    fn the_blur_shader_uses_those_weights_and_offsets() {
        let shader = blur_shader();
        for value in [GAUSSIAN_WEIGHTS[0], GAUSSIAN_WEIGHTS[1], GAUSSIAN_WEIGHTS[2]] {
            assert!(shader.contains(&format!("{value:.8}")), "{shader}");
        }
        assert!(shader.contains(&format!("{:.7}", GAUSSIAN_OFFSETS[2])));
    }
}

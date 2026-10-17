//! The glass look: translucent rounded panels floating over a soft,
//! luminous backdrop, in the spirit of Apple's "Liquid Glass".
//!
//! egui can't blur what sits behind a panel, so the effect comes from three
//! things that read as glass: low-opacity fills over a colourful gradient,
//! a hairline light border that catches the edge, and a wide soft shadow
//! that lifts the panel off the canvas.

use egui::{epaint, Color32, CornerRadius, Frame, Margin, Painter, Pos2, Rect, Shadow, Stroke, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Appearance {
    Dark,
    Light,
}

pub struct Palette {
    pub glass: Color32,
    pub glass_strong: Color32,
    pub edge: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub backdrop_top: Color32,
    pub backdrop_bottom: Color32,
    pub glows: [Color32; 3],
    pub checker_light: Color32,
    pub checker_dark: Color32,
}

pub fn palette(appearance: Appearance) -> Palette {
    match appearance {
        Appearance::Dark => Palette {
            glass: Color32::from_rgba_unmultiplied(30, 32, 44, 168),
            glass_strong: Color32::from_rgba_unmultiplied(36, 38, 52, 220),
            edge: Color32::from_rgba_unmultiplied(255, 255, 255, 34),
            text: Color32::from_rgb(236, 238, 245),
            text_dim: Color32::from_rgb(160, 166, 184),
            accent: Color32::from_rgb(120, 140, 255),
            backdrop_top: Color32::from_rgb(18, 20, 34),
            backdrop_bottom: Color32::from_rgb(10, 26, 38),
            glows: [
                Color32::from_rgba_unmultiplied(120, 80, 255, 20),
                Color32::from_rgba_unmultiplied(0, 190, 220, 16),
                Color32::from_rgba_unmultiplied(255, 90, 160, 12),
            ],
            checker_light: Color32::from_rgb(64, 66, 78),
            checker_dark: Color32::from_rgb(48, 50, 60),
        },
        Appearance::Light => Palette {
            glass: Color32::from_rgba_unmultiplied(255, 255, 255, 150),
            glass_strong: Color32::from_rgba_unmultiplied(255, 255, 255, 215),
            edge: Color32::from_rgba_unmultiplied(255, 255, 255, 210),
            text: Color32::from_rgb(28, 30, 38),
            text_dim: Color32::from_rgb(96, 102, 118),
            accent: Color32::from_rgb(64, 96, 240),
            backdrop_top: Color32::from_rgb(214, 222, 246),
            backdrop_bottom: Color32::from_rgb(236, 226, 244),
            glows: [
                Color32::from_rgba_unmultiplied(150, 120, 255, 26),
                Color32::from_rgba_unmultiplied(80, 210, 240, 22),
                Color32::from_rgba_unmultiplied(255, 150, 190, 20),
            ],
            checker_light: Color32::from_rgb(250, 250, 252),
            checker_dark: Color32::from_rgb(222, 224, 232),
        },
    }
}

pub const PANEL_RADIUS: u8 = 16;
pub const CONTROL_RADIUS: u8 = 9;

/// Applies the glass styling to every egui widget.
pub fn apply(ctx: &egui::Context, appearance: Appearance) {
    let colours = palette(appearance);
    let mut visuals = match appearance {
        Appearance::Dark => egui::Visuals::dark(),
        Appearance::Light => egui::Visuals::light(),
    };
    visuals.override_text_color = Some(colours.text);
    visuals.window_fill = colours.glass;
    visuals.panel_fill = Color32::TRANSPARENT;
    visuals.window_stroke = Stroke::new(1.0_f32, colours.edge);
    visuals.window_corner_radius = CornerRadius::same(PANEL_RADIUS);
    visuals.window_shadow = Shadow { offset: [0, 10], blur: 32, spread: 0, color: Color32::from_black_alpha(70) };
    visuals.popup_shadow = Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(60) };
    visuals.menu_corner_radius = CornerRadius::same(CONTROL_RADIUS + 3);
    visuals.selection.bg_fill = colours.accent.gamma_multiply(0.75);
    visuals.selection.stroke = Stroke::new(1.0_f32, colours.text);
    visuals.hyperlink_color = colours.accent;
    visuals.extreme_bg_color = colours.glass_strong.gamma_multiply(0.6);
    visuals.faint_bg_color = colours.glass.gamma_multiply(0.5);

    let glassy = |alpha: f32| colours.glass_strong.gamma_multiply(alpha);
    for (state, fill, stroke_alpha) in [
        (&mut visuals.widgets.noninteractive, Color32::TRANSPARENT, 0.0),
        (&mut visuals.widgets.inactive, glassy(0.55), 0.5),
        (&mut visuals.widgets.hovered, glassy(0.85), 1.0),
        (&mut visuals.widgets.active, colours.accent.gamma_multiply(0.55), 1.0),
        (&mut visuals.widgets.open, glassy(0.9), 1.0),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.corner_radius = CornerRadius::same(CONTROL_RADIUS);
        state.bg_stroke = Stroke::new(1.0_f32, colours.edge.gamma_multiply(stroke_alpha));
    }
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, colours.text_dim);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, colours.text);
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.spacing.window_margin = Margin::same(12);
        style.spacing.interact_size.y = 26.0;
        style.spacing.slider_width = 140.0;
    });
}

/// The frame for floating panels.
pub fn glass_frame(appearance: Appearance) -> Frame {
    let colours = palette(appearance);
    Frame::new()
        .fill(colours.glass)
        .stroke(Stroke::new(1.0_f32, colours.edge))
        .corner_radius(CornerRadius::same(PANEL_RADIUS))
        .shadow(Shadow { offset: [0, 10], blur: 32, spread: 0, color: Color32::from_black_alpha(64) })
        .inner_margin(Margin::same(10))
}

/// Shows `frame` with the blurred backdrop behind it when `blur` is on.
/// The backdrop's spot in the layer is taken before the frame draws, so it
/// sits underneath the frame's tint, border and contents.
pub fn show_glass<R>(
    ui: &mut egui::Ui,
    frame: Frame,
    blur: Option<super::glass_blur::GlassBlur>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let slot = blur.as_ref().map(|_| ui.painter().add(egui::Shape::Noop));
    let shown = frame.show(ui, add_contents);
    if let (Some(slot), Some(blur)) = (slot, blur) {
        ui.painter().set(slot, blur.backdrop(shown.response.rect, f32::from(frame.corner_radius.nw)));
    }
    shown
}

/// A tighter, pill-shaped frame for toolbars and docks.
pub fn pill_frame(appearance: Appearance) -> Frame {
    glass_frame(appearance).corner_radius(CornerRadius::same(22)).inner_margin(Margin::symmetric(8, 6))
}

/// Paints the luminous backdrop: a vertical gradient with soft coloured
/// glows that give the glass something to refract.
pub fn paint_backdrop(painter: &Painter, rect: Rect, appearance: Appearance, time: f64) {
    let colours = palette(appearance);
    let mut mesh = epaint::Mesh::default();
    mesh.colored_vertex(rect.left_top(), colours.backdrop_top);
    mesh.colored_vertex(rect.right_top(), colours.backdrop_top);
    mesh.colored_vertex(rect.left_bottom(), colours.backdrop_bottom);
    mesh.colored_vertex(rect.right_bottom(), colours.backdrop_bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(egui::Shape::mesh(mesh));

    // Glows drift very slowly, enough to feel alive but never distracting.
    let drift = (time * 0.05).sin() as f32;
    let anchors = [(0.18 + 0.02 * drift, 0.22), (0.82, 0.30 - 0.03 * drift), (0.55 - 0.02 * drift, 0.88)];
    let radius = rect.width().max(rect.height()) * 0.42;
    for (glow, (fx, fy)) in colours.glows.iter().zip(anchors) {
        let centre = Pos2::new(rect.left() + rect.width() * fx, rect.top() + rect.height() * fy);
        soft_disc(painter, centre, radius, *glow);
    }
}

/// A radial glow built from stacked translucent circles, brightest at the
/// centre.
fn soft_disc(painter: &Painter, centre: Pos2, radius: f32, colour: Color32) {
    const RINGS: usize = 14;
    for ring in 0..RINGS {
        let t = 1.0 - ring as f32 / RINGS as f32;
        painter.circle_filled(centre, radius * t, colour);
    }
}

#[cfg(test)]
mod tests {
    /// Every character in a string literal of the UI's source, so a test
    /// can check the bundled fonts can draw them. Comments don't count.
    fn characters_in_string_literals(source: &str) -> String {
        let mut found = String::new();
        let (mut inside, mut escaped) = (false, false);
        for c in source.chars() {
            match (inside, escaped, c) {
                (true, true, _) => escaped = false,
                (true, false, '\\') => escaped = true,
                (_, false, '"') => inside = !inside,
                (true, false, _) => found.push(c),
                _ => {}
            }
        }
        found
    }

    #[test]
    fn every_icon_and_symbol_in_the_ui_has_a_glyph_in_the_bundled_fonts() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let font = egui::FontId::proportional(14.0);
        let sources = [
            ("app.rs", include_str!("app.rs")),
            ("canvas.rs", include_str!("canvas.rs")),
            ("file_browser.rs", include_str!("file_browser.rs")),
            ("panels.rs", include_str!("panels.rs")),
            ("tools.rs", include_str!("tools.rs")),
            ("theme.rs", include_str!("theme.rs")),
        ];
        for (file, source) in sources {
            let missing: String = characters_in_string_literals(source)
                .chars()
                .filter(|c| !c.is_ascii() && !ctx.fonts(|fonts| fonts.has_glyph(&font, *c)))
                .collect();
            assert!(missing.is_empty(), "{file} uses characters the fonts can't draw, which show as □: {missing}");
        }
    }

    #[test]
    fn string_literal_scanning_reads_only_inside_quotes_and_skips_escapes() {
        let source = "// → not a string\nlet a = \"x\\ny\";";
        assert_eq!(characters_in_string_literals(source), "xy");
    }
}

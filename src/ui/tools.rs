//! The tool dock: which tool is in hand, and how its settings become an
//! editor `Ink` and `Brush`.

use crate::editor::{Brush, Ink, Pattern, Rgba};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Pencil,
    Eraser,
    Dither,
    Shade,
    Spray,
    CloneStamp,
    Line,
    Rectangle,
    Ellipse,
    Fill,
    Gradient,
    Eyedropper,
    Select,
    Lasso,
    Wand,
    Move,
    Pin,
}

impl Tool {
    pub const ALL: [Tool; 17] = [
        Tool::Pencil,
        Tool::Eraser,
        Tool::Dither,
        Tool::Shade,
        Tool::Spray,
        Tool::CloneStamp,
        Tool::Line,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Fill,
        Tool::Gradient,
        Tool::Eyedropper,
        Tool::Select,
        Tool::Lasso,
        Tool::Wand,
        Tool::Move,
        Tool::Pin,
    ];

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Pencil => "✏",
            Tool::Eraser => "⌫",
            Tool::Dither => "▦",
            Tool::Shade => "◐",
            Tool::Spray => "⁘",
            Tool::CloneStamp => "⎘",
            Tool::Line => "╱",
            Tool::Rectangle => "▭",
            Tool::Ellipse => "◯",
            Tool::Fill => "🌢",
            Tool::Gradient => "▤",
            Tool::Eyedropper => "💉",
            Tool::Select => "⬚",
            Tool::Lasso => "➰",
            Tool::Wand => "✨",
            Tool::Move => "✥",
            Tool::Pin => "📌",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::Pencil => "Pencil",
            Tool::Eraser => "Eraser",
            Tool::Dither => "Dither brush",
            Tool::Shade => "Shade along ramp",
            Tool::Spray => "Spray",
            Tool::CloneStamp => "Clone stamp",
            Tool::Line => "Line",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Fill => "Bucket fill",
            Tool::Gradient => "Dither gradient",
            Tool::Eyedropper => "Eyedropper",
            Tool::Select => "Rectangle select",
            Tool::Lasso => "Lasso select",
            Tool::Wand => "Magic wand",
            Tool::Move => "Move selection",
            Tool::Pin => "Pin a note",
        }
    }

    /// Aseprite-style single-key shortcuts.
    pub fn shortcut(self) -> Option<egui::Key> {
        use egui::Key;
        Some(match self {
            Tool::Pencil => Key::B,
            Tool::Eraser => Key::E,
            Tool::Dither => Key::D,
            Tool::Shade => Key::S,
            Tool::Spray => Key::P,
            Tool::CloneStamp => Key::C,
            Tool::Line => Key::L,
            Tool::Rectangle => Key::U,
            Tool::Ellipse => Key::O,
            Tool::Fill => Key::G,
            Tool::Eyedropper => Key::I,
            Tool::Select => Key::M,
            Tool::Lasso => Key::Q,
            Tool::Wand => Key::W,
            Tool::Move => Key::V,
            Tool::Gradient | Tool::Pin => return None,
        })
    }

    /// Tools painted live along a drag, as opposed to shapes previewed and
    /// committed on release.
    pub fn is_stroke(self) -> bool {
        matches!(self, Tool::Pencil | Tool::Eraser | Tool::Dither | Tool::Shade | Tool::Spray | Tool::CloneStamp)
    }

    pub fn is_shape(self) -> bool {
        matches!(self, Tool::Line | Tool::Rectangle | Tool::Ellipse | Tool::Gradient | Tool::Select)
    }
}

/// Everything the dock and its inspector let you set.
#[derive(Clone)]
pub struct ToolSettings {
    pub tool: Tool,
    pub primary: Rgba,
    pub secondary: Rgba,
    pub brush: Brush,
    pub pixel_perfect: bool,
    pub filled: bool,
    pub contiguous: bool,
    pub pattern: Pattern,
    pub pattern_two_colour: bool,
    pub spray_radius: u32,
    pub spray_density: u32,
    pub shade_lightens: bool,
    /// The clone stamp's source offset, set with Alt-click.
    pub clone_offset: Option<(i32, i32)>,
    pub clone_anchor: Option<(i32, i32)>,
}

impl Default for ToolSettings {
    fn default() -> Self {
        ToolSettings {
            tool: Tool::Pencil,
            primary: Rgba::from_hex("#1d2b53").unwrap_or(Rgba::BLACK),
            secondary: Rgba::from_hex("#fff1e8").unwrap_or(Rgba::WHITE),
            brush: Brush::default(),
            pixel_perfect: true,
            filled: false,
            contiguous: true,
            pattern: Pattern::Checker,
            pattern_two_colour: false,
            spray_radius: 4,
            spray_density: 6,
            shade_lightens: true,
            clone_offset: None,
            clone_anchor: None,
        }
    }
}

impl ToolSettings {
    /// The ink the current tool paints with. Shape tools use the dither
    /// pattern too when it isn't solid, which is how MS Paint's pattern
    /// fills worked.
    pub fn ink(&self) -> Ink {
        let pattern_ink = || Ink::Pattern {
            pattern: self.pattern,
            primary: self.primary,
            secondary: self.pattern_two_colour.then_some(self.secondary),
        };
        match self.tool {
            Tool::Eraser => Ink::Erase,
            Tool::Dither => pattern_ink(),
            Tool::Shade => Ink::Shade { steps: if self.shade_lightens { 1 } else { -1 } },
            Tool::CloneStamp => {
                let (dx, dy) = self.clone_offset.unwrap_or((0, 0));
                Ink::Clone { dx, dy }
            }
            Tool::Rectangle | Tool::Ellipse | Tool::Fill if self.pattern != Pattern::Solid => pattern_ink(),
            _ => Ink::colour(self.primary),
        }
    }

    pub fn swap_colours(&mut self) {
        std::mem::swap(&mut self.primary, &mut self.secondary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_distinct_shortcut() {
        let mut keys: Vec<egui::Key> = Tool::ALL.iter().filter_map(|t| t.shortcut()).collect();
        let count = keys.len();
        keys.sort_by_key(|k| k.name());
        keys.dedup();
        assert_eq!(keys.len(), count);
    }

    #[test]
    fn shape_tools_pick_up_the_dither_pattern() {
        let settings = ToolSettings { tool: Tool::Rectangle, pattern: Pattern::Bricks, ..ToolSettings::default() };
        assert!(matches!(settings.ink(), Ink::Pattern { pattern: Pattern::Bricks, .. }));
        let solid = ToolSettings { tool: Tool::Rectangle, pattern: Pattern::Solid, ..ToolSettings::default() };
        assert_eq!(solid.ink(), Ink::colour(solid.primary));
    }

    #[test]
    fn the_pencil_ignores_the_pattern() {
        let settings = ToolSettings { tool: Tool::Pencil, pattern: Pattern::Checker, ..ToolSettings::default() };
        assert_eq!(settings.ink(), Ink::colour(settings.primary));
    }

    #[test]
    fn swapping_colours_exchanges_primary_and_secondary() {
        let mut settings = ToolSettings::default();
        let (a, b) = (settings.primary, settings.secondary);
        settings.swap_colours();
        assert_eq!((settings.primary, settings.secondary), (b, a));
    }
}

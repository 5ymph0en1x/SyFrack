use egui::{Color32, FontFamily, FontId, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(0x10, 0x16, 0x1b);
pub const PANEL: Color32 = Color32::from_rgb(0x19, 0x22, 0x29);
pub const FIELD: Color32 = Color32::from_rgb(0x22, 0x2e, 0x37);
pub const SEPARATOR: Color32 = Color32::from_rgb(0x30, 0x3e, 0x47);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xed, 0xf0);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xa2, 0xb0, 0xb9);
pub const ACCENT: Color32 = Color32::from_rgb(0x96, 0xcb, 0xb6);
pub const SELECTED: Color32 = Color32::from_rgb(0x29, 0x47, 0x40);
pub const ERROR: Color32 = Color32::from_rgb(0xf2, 0x8b, 0x82);
pub const GRID: f32 = 8.0;
/// Clear colour of the swapchain, in linear light (the surface is sRGB).
pub const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.0052,
    g: 0.0080,
    b: 0.0110,
    a: 1.0,
};

/// The studio's symbols missing from egui's fonts (« → », « ● »), drawn by
/// `assets/make_symbols.py`.
const SYMBOLS_TTF: &[u8] = include_bytes!("../../assets/symbols.ttf");

/// egui's fonts with the studio symbols as the last fallback of both families.
fn fonts() -> egui::FontDefinitions {
    let mut defs = egui::FontDefinitions::default();
    defs.font_data.insert(
        "syfrack-symbols".into(),
        std::sync::Arc::new(egui::FontData::from_static(SYMBOLS_TTF)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        defs.families
            .entry(family)
            .or_default()
            .push("syfrack-symbols".into());
    }
    defs
}

/// Slate surfaces, a sage accent and readable text on an 8 px grid.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    let mut style = (*ctx.global_style()).clone();
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = BG;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = SELECTED;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, SEPARATOR);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    v.widgets.inactive.bg_fill = FIELD;
    v.widgets.inactive.weak_bg_fill = FIELD;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, SEPARATOR);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_fill = SEPARATOR;
    v.widgets.hovered.weak_bg_fill = SEPARATOR;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, TEXT_DIM);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.active.bg_fill = FIELD;
    v.widgets.active.weak_bg_fill = FIELD;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    // egui also uses this foreground for RichText::strong headings.
    v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    v.weak_text_color = Some(TEXT_DIM);
    v.window_corner_radius = 8.0.into();
    v.menu_corner_radius = 6.0.into();
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(18.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(GRID, 6.0);
    style.spacing.window_margin = egui::Margin::same(GRID as i8);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.combo_width = 200.0;
    style.spacing.slider_width = 160.0;
    ctx.set_global_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_installs_the_palette_and_three_text_sizes() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let style = ctx.global_style();
        assert_eq!(style.visuals.panel_fill, PANEL);
        assert_eq!(style.visuals.window_fill, PANEL);
        assert_eq!(style.visuals.strong_text_color(), TEXT);
        assert_eq!(style.visuals.selection.bg_fill, SELECTED);
        assert_eq!(
            style.visuals.widgets.noninteractive.bg_stroke.color,
            SEPARATOR
        );
        let size = |ts: egui::TextStyle| style.text_styles[&ts].size;
        assert_eq!(size(egui::TextStyle::Heading), 18.0);
        assert_eq!(size(egui::TextStyle::Body), 14.0);
        assert_eq!(size(egui::TextStyle::Monospace), 13.0);
        assert_eq!(style.spacing.item_spacing, egui::vec2(GRID, 6.0));
        assert!(style.visuals.dark_mode, "dark theme");
    }

    #[test]
    fn the_studio_symbols_are_in_the_text_font() {
        let ctx = egui::Context::default();
        apply(&ctx);
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        // The proportional family draws every label and banner. (egui's
        // `has_glyph` answers false in the monospace family whatever the
        // fallback, through its replacement-face heuristic.)
        for c in ['→', '●', '⟳'] {
            let font = FontId::proportional(13.0);
            assert!(ctx.fonts_mut(|f| f.has_glyph(&font, c)), "{c}");
        }
    }

    #[test]
    fn text_contrast_is_readable_on_panels() {
        let luminance = |c: Color32| {
            let rgb = c.to_array();
            let linear = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
        };
        for ink in [TEXT, TEXT_DIM, ACCENT, ERROR] {
            assert!((luminance(ink) + 0.05) / (luminance(PANEL) + 0.05) >= 4.5);
        }
    }
}

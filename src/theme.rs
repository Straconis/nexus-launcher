//! Straconis palette and the bundled Orbitron header face.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontFamily, Stroke, Vec2};

pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x24, 0x4e);
pub const ACCENT_DEEP: Color32 = Color32::from_rgb(0xc4, 0x12, 0x36);
pub const CHROME: Color32 = Color32::from_rgb(0x18, 0x19, 0x20);
pub const BG: Color32 = Color32::from_rgb(0x07, 0x08, 0x0b);
pub const PANEL: Color32 = Color32::from_rgb(0x10, 0x11, 0x18);
pub const TEXT: Color32 = Color32::from_rgb(0xf5, 0xf5, 0xfa);
pub const MUTED: Color32 = Color32::from_rgb(0xb4, 0xb4, 0xc5);
pub const BORDER: Color32 = Color32::from_rgb(0x64, 0x20, 0x3c);

pub fn orbitron() -> FontFamily {
    FontFamily::Name("Orbitron".into())
}

pub fn apply(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // Unmodified SIL OFL variable font. Reserved Font Name: Orbitron.
    fonts.font_data.insert(
        "Orbitron".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Orbitron[wght].ttf"
        ))),
    );
    fonts
        .families
        .entry(FontFamily::Name("Orbitron".into()))
        .or_default()
        .insert(0, "Orbitron".to_owned());
    ctx.set_fonts(fonts);

    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = BG;
    visuals.panel_fill = BG;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = PANEL;
    visuals.code_bg_color = PANEL;
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(0xff, 0x24, 0x4e, 80);
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.warn_fg_color = ACCENT;
    visuals.error_fg_color = ACCENT;
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
    ] {
        widget.bg_fill = PANEL;
        widget.weak_bg_fill = PANEL;
        widget.fg_stroke = Stroke::new(1.0_f32, TEXT);
        widget.bg_stroke = Stroke::new(1.0_f32, BORDER);
    }
    visuals.widgets.hovered.bg_fill = ACCENT;
    visuals.widgets.hovered.weak_bg_fill = ACCENT;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.bg_fill = ACCENT_DEEP;
    visuals.widgets.active.weak_bg_fill = ACCENT_DEEP;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT_DEEP);
    visuals.widgets.open.bg_fill = PANEL;
    visuals.widgets.open.weak_bg_fill = PANEL;
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, BORDER);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);
    ctx.set_style(style);
}

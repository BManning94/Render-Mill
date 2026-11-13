//! Look & feel modelled on Lumion 2025: a thin dark title bar, a narrow icon
//! sidebar, and flat charcoal panels floating over a full-screen viewport.

use egui::{Color32, FontFamily, FontId, Rounding, Stroke, TextStyle};

pub const ACCENT: Color32 = Color32::from_rgb(30, 136, 229);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(44, 82, 140);
/// Selected tool / tab background (Lumion's muted blue).
pub const SELECTED: Color32 = Color32::from_rgb(52, 86, 140);
pub const TITLE_BAR: Color32 = Color32::from_rgb(22, 22, 23);
pub const SIDEBAR: Color32 = Color32::from_rgb(28, 28, 30);
pub const PANEL: Color32 = Color32::from_rgb(30, 31, 34);
pub const PANEL_LIGHT: Color32 = Color32::from_rgba_premultiplied(40, 41, 45, 248);
pub const PHOTO_BG: Color32 = Color32::from_rgb(36, 37, 40);
pub const BAR_BG: Color32 = Color32::from_rgb(40, 41, 44);
pub const BAR_FILL: Color32 = Color32::from_rgb(78, 79, 83);
pub const TILE: Color32 = Color32::from_rgb(44, 45, 49);
pub const TEXT: Color32 = Color32::from_rgb(232, 233, 236);
pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 152, 158);
pub const GOOD: Color32 = Color32::from_rgb(46, 204, 64);
pub const WARN: Color32 = Color32::from_rgb(250, 190, 70);
pub const DANGER: Color32 = Color32::from_rgb(150, 30, 42);
pub const HOVER_YELLOW: Color32 = Color32::from_rgb(255, 214, 10);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(15.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(12.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(12.5, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.slider_width = 150.0;
    style.spacing.interact_size.y = 24.0;

    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0_f32, Color32::from_white_alpha(12));
    v.window_rounding = Rounding::same(6.0);
    v.menu_rounding = Rounding::same(4.0);
    v.extreme_bg_color = Color32::from_rgb(24, 24, 26);
    v.faint_bg_color = Color32::from_rgb(36, 37, 40);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.hyperlink_color = ACCENT;
    v.override_text_color = Some(TEXT);
    let r = Rounding::same(4.0);
    v.widgets.noninteractive.bg_fill = PANEL_LIGHT;
    v.widgets.noninteractive.rounding = r;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_white_alpha(12));
    v.widgets.inactive.bg_fill = Color32::from_rgb(48, 49, 53);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(48, 49, 53);
    v.widgets.inactive.rounding = r;
    v.widgets.hovered.bg_fill = Color32::from_rgb(62, 63, 68);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(62, 63, 68);
    v.widgets.hovered.rounding = r;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_white_alpha(40));
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.active.weak_bg_fill = ACCENT_DIM;
    v.widgets.active.rounding = r;
    v.slider_trailing_fill = true;
    style.visuals = v;
    ctx.set_style(style);
}

/// Frame for a floating panel (flat, lightly rounded, like Lumion's cards).
pub fn panel_frame() -> egui::Frame {
    egui::Frame::none().fill(PANEL).rounding(Rounding::same(4.0)).inner_margin(egui::Margin::same(12.0)).shadow(egui::epaint::Shadow {
        offset: egui::vec2(0.0, 2.0),
        blur: 10.0,
        spread: 0.0,
        color: Color32::from_black_alpha(70),
    })
}

/// Tight frame for compact cards and docks.
pub fn card_frame() -> egui::Frame {
    panel_frame().inner_margin(egui::Margin::same(8.0))
}

pub fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.5).color(TEXT).strong());
}

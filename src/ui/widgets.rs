//! Custom widgets in the Lumion style: flat tool buttons, value bars with the
//! label inside, swatches, toggles, key chips, library tiles, the sun dial.

use super::icons::{self, Icon};
use super::theme::{self, ACCENT, BAR_BG, BAR_FILL, SELECTED, TEXT, TEXT_DIM, TILE};
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Response, Sense, Stroke, Ui};

/// Flat square tool button (Lumion's toolbar buttons): transparent until
/// hovered, muted blue when selected.
pub fn tool_button(ui: &mut Ui, icon: Icon, size: egui::Vec2, selected: bool, tooltip: &str) -> Response {
    tool_button_colored(ui, icon, size, selected, None, tooltip)
}

pub fn tool_button_colored(ui: &mut Ui, icon: Icon, size: egui::Vec2, selected: bool, fill: Option<Color32>, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let bg = if let Some(f) = fill {
        f
    } else if selected {
        SELECTED
    } else if resp.hovered() {
        Color32::from_rgb(52, 53, 58)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 3.0, bg);
    let col = if selected || resp.hovered() || fill.is_some() { Color32::WHITE } else { Color32::from_rgb(205, 207, 212) };
    let s = size.x.min(size.y);
    icons::draw(ui.painter(), icon, Rect::from_center_size(rect.center(), vec2(s * 0.46, s * 0.46)), col);
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}

/// Backwards-compatible square icon button.
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32, selected: bool, tooltip: &str) -> Response {
    tool_button(ui, icon, vec2(size, size), selected, tooltip)
}

/// Lumion-style value bar: a dark track, a grey fill showing the value, the
/// label inside on the left and the value on the right. Drag anywhere on it.
/// `frac`/`unfrac` map between the value and 0..1 along the bar.
pub fn value_bar(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    width: f32,
    frac: impl Fn(f32) -> f32,
    unfrac: impl Fn(f32) -> f32,
    display: impl Fn(f32) -> String,
) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click_and_drag());
    if resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let t = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let nv = unfrac(t);
            if (nv - *value).abs() > f32::EPSILON {
                *value = nv;
                resp.mark_changed();
            }
        }
    }
    let active = resp.dragged() || resp.hovered();
    let p = ui.painter();
    p.rect_filled(rect, 2.0, BAR_BG);
    let t = frac(*value).clamp(0.0, 1.0);
    let fill = Rect::from_min_size(rect.min, vec2(rect.width() * t, rect.height()));
    p.rect_filled(fill, 2.0, if active { ACCENT } else { BAR_FILL });
    p.text(pos2(rect.left() + 9.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(12.0), TEXT);
    if active || resp.has_focus() {
        p.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, display(*value), FontId::proportional(11.5), Color32::from_white_alpha(170));
    }
    resp
}

/// Linear value bar.
pub fn bar(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32, display: impl Fn(f32) -> String) -> Response {
    let (lo, hi) = (*range.start(), *range.end());
    value_bar(ui, label, value, width, move |v| (v - lo) / (hi - lo), move |t| lo + t * (hi - lo), display)
}

/// Logarithmic value bar (sizes, distances).
pub fn log_bar(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32, display: impl Fn(f32) -> String) -> Response {
    let (lo, hi) = (range.start().ln(), range.end().ln());
    value_bar(ui, label, value, width, move |v| (v.max(1e-6).ln() - lo) / (hi - lo), move |t| (lo + t * (hi - lo)).exp(), display)
}

pub fn pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

/// Small square colour/texture swatch; returns the click response.
pub fn swatch(ui: &mut Ui, color: Option<Color32>, texture: Option<&egui::TextureHandle>, size: f32, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let p = ui.painter();
    if let Some(t) = texture {
        p.image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else if let Some(c) = color {
        p.rect_filled(rect, 1.0, c);
    } else {
        // checker = "no map"
        let h = size / 2.0;
        p.rect_filled(rect, 1.0, Color32::from_rgb(70, 70, 74));
        p.rect_filled(Rect::from_min_size(rect.min, vec2(h, h)), 0.0, Color32::from_rgb(120, 120, 124));
        p.rect_filled(Rect::from_min_size(rect.min + vec2(h, h), vec2(h, h)), 0.0, Color32::from_rgb(120, 120, 124));
    }
    if resp.hovered() {
        p.rect_stroke(rect, 1.0, Stroke::new(1.5_f32, Color32::WHITE));
    }
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}

/// Lumion-style on/off switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(28.0, 14.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let p = ui.painter();
    p.rect_filled(rect, 3.0, if *on { ACCENT } else { Color32::from_rgb(70, 71, 76) });
    let knob = if *on { rect.right() - 7.0 } else { rect.left() + 7.0 };
    p.rect_filled(Rect::from_center_size(pos2(knob, rect.center().y), vec2(10.0, 10.0)), 2.0, Color32::WHITE);
    resp
}

/// A key cap like Lumion's shortcut hints ("SHIFT", "WASD").
pub fn key_chip(p: &egui::Painter, right_center: egui::Pos2, key: &str) -> Rect {
    let galley = p.layout_no_wrap(key.to_string(), FontId::proportional(10.5), Color32::WHITE);
    let w = galley.size().x + 10.0;
    let r = Rect::from_min_size(pos2(right_center.x - w, right_center.y - 9.0), vec2(w, 18.0));
    p.rect_filled(r, 2.0, Color32::from_black_alpha(170));
    p.rect_stroke(r, 2.0, Stroke::new(1.0_f32, Color32::from_white_alpha(200)));
    p.galley(pos2(r.left() + 5.0, r.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    r
}

/// Text label with dark background on the left of a key chip.
pub fn chip_label(p: &egui::Painter, right_center: egui::Pos2, text: &str) -> Rect {
    let galley = p.layout_no_wrap(text.to_string(), FontId::proportional(10.5), Color32::WHITE);
    let w = galley.size().x + 10.0;
    let r = Rect::from_min_size(pos2(right_center.x - w, right_center.y - 9.0), vec2(w, 18.0));
    p.rect_filled(r, 2.0, Color32::from_black_alpha(190));
    p.galley(pos2(r.left() + 5.0, r.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    r
}

/// Library tile: thumbnail (or icon), optional caption underneath.
pub fn tile(ui: &mut Ui, thumb: Option<&egui::TextureHandle>, fallback: Icon, caption: &str, selected: bool, size: f32) -> Response {
    let cap_h = if caption.is_empty() { 0.0 } else { 16.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size + cap_h), Sense::click());
    let img_rect = Rect::from_min_size(rect.min, vec2(size, size));
    ui.painter().rect_filled(img_rect, 2.0, TILE);
    if let Some(t) = thumb {
        ui.painter().image(t.id(), img_rect.shrink(1.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else {
        icons::draw(ui.painter(), fallback, img_rect.shrink(size * 0.3), TEXT_DIM);
    }
    let stroke = if selected {
        Stroke::new(2.0_f32, ACCENT)
    } else if resp.hovered() {
        Stroke::new(1.5_f32, Color32::from_white_alpha(160))
    } else {
        Stroke::NONE
    };
    ui.painter().rect_stroke(img_rect, 2.0, stroke);
    if !caption.is_empty() {
        let galley = ui.painter().layout(caption.to_string(), FontId::proportional(10.5), if selected { Color32::WHITE } else { TEXT_DIM }, size);
        let text_pos = pos2(rect.center().x - galley.size().x.min(size) / 2.0, img_rect.bottom() + 2.0);
        ui.painter().with_clip_rect(rect).galley(text_pos, galley, TEXT);
    }
    resp
}

/// Lumion-style sun dial: drag the sun around the compass for heading, toward
/// the centre for a higher sun. Returns true when changed.
pub fn sun_dial(ui: &mut Ui, heading: &mut f32, height: &mut f32, size: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let c = rect.center();
    let r = size * 0.4;
    let p = ui.painter();
    p.circle_filled(c, r, Color32::from_rgb(24, 26, 30));
    for k in [0.33, 0.66] {
        p.circle_stroke(c, r * k, Stroke::new(1.0_f32, Color32::from_white_alpha(25)));
    }
    p.circle_stroke(c, r, Stroke::new(1.5_f32, Color32::from_white_alpha(60)));
    for (label, ang) in [("N", 0.0f32), ("E", 90.0), ("S", 180.0), ("W", 270.0)] {
        let a = ang.to_radians();
        let d = vec2(a.sin(), -a.cos());
        p.text(c + d * (r + 8.0), Align2::CENTER_CENTER, label, FontId::proportional(9.5), TEXT_DIM);
    }
    let mut changed = false;
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let d = pos - c;
            *heading = (d.x.atan2(-d.y).to_degrees() + 360.0) % 360.0;
            let t = (d.length() / r).clamp(0.0, 1.15);
            *height = 90.0 - t * 90.0;
            changed = true;
        }
    }
    let a = heading.to_radians();
    let t = ((90.0 - *height) / 90.0).clamp(0.0, 1.15);
    let sun = c + vec2(a.sin(), -a.cos()) * r * t;
    let day = *height > 0.0;
    p.line_segment([c, sun], Stroke::new(1.0_f32, Color32::from_white_alpha(70)));
    p.circle_filled(sun, 7.0, if day { Color32::from_rgb(255, 205, 90) } else { Color32::from_rgb(120, 140, 200) });
    resp.on_hover_text("Drag the sun: around = direction, toward centre = higher");
    changed
}

/// Labelled classic slider row (used in a few secondary panels).
pub fn slider(ui: &mut Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) -> Response {
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 20.0], egui::Label::new(egui::RichText::new(label).color(TEXT_DIM)).truncate());
        ui.add(egui::Slider::new(v, range).show_value(true))
    })
    .inner
}

/// Text button styled like Lumion's flat buttons (optionally highlighted).
pub fn flat_button(ui: &mut Ui, icon: Option<Icon>, text: &str, primary: bool, width: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let bg = if primary {
        if resp.hovered() {
            Color32::from_rgb(60, 104, 170)
        } else {
            SELECTED
        }
    } else if resp.hovered() {
        Color32::from_rgb(58, 59, 64)
    } else {
        Color32::from_rgb(44, 45, 49)
    };
    let p = ui.painter();
    p.rect_filled(rect, 3.0, bg);
    let galley = p.layout_no_wrap(text.to_string(), FontId::proportional(12.0), Color32::WHITE);
    let icon_w = if icon.is_some() { 20.0 } else { 0.0 };
    let total = galley.size().x + icon_w;
    let x0 = rect.center().x - total / 2.0;
    if let Some(i) = icon {
        icons::draw(p, i, Rect::from_center_size(pos2(x0 + 7.0, rect.center().y), vec2(13.0, 13.0)), Color32::WHITE);
    }
    p.galley(pos2(x0 + icon_w, rect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    resp
}

/// Row with a label inside a dark bar and a widget area on the right
/// (Lumion's "Color" row).
pub fn label_row(ui: &mut Ui, label: &str, width: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    ui.painter().rect_filled(rect, 2.0, BAR_BG);
    ui.painter().text(pos2(rect.left() + 9.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(12.0), TEXT);
    rect
}

/// Hover preview card shown beside a library (big image + name + tags).
pub fn preview_card(ctx: &egui::Context, id: &str, at: egui::Pos2, title: &str, tags: &[&str], image: Option<&egui::TextureHandle>, image_size: egui::Vec2) {
    egui::Area::new(egui::Id::new(id)).fixed_pos(at).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
        theme::panel_frame().show(ui, |ui| {
            ui.set_width(image_size.x.max(220.0));
            ui.label(egui::RichText::new(title).size(13.0).strong());
            if !tags.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for t in tags {
                        egui::Frame::none().fill(Color32::from_rgb(52, 53, 58)).rounding(2.0).inner_margin(egui::Margin::symmetric(5.0, 2.0)).show(ui, |ui| {
                            ui.label(egui::RichText::new(*t).size(11.0));
                        });
                    }
                });
            }
            if let Some(t) = image {
                let (r, _) = ui.allocate_exact_size(image_size, Sense::hover());
                ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
        });
    });
}

/// Left-aligned, truncated label in a fixed-size box.
pub fn left_label(ui: &mut Ui, size: egui::Vec2, text: egui::RichText) -> Response {
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(size);
        ui.add(egui::Label::new(text).truncate())
    })
    .inner
}

/// "⋮" button that opens a small menu below it.
pub fn kebab(ui: &mut Ui, id: impl std::hash::Hash, tooltip: &str, add_contents: impl FnOnce(&mut Ui)) {
    let resp = tool_button(ui, Icon::Kebab, vec2(22.0, 26.0), false, tooltip);
    let popup = ui.make_persistent_id(id);
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup));
    }
    egui::popup_below_widget(ui, popup, &resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(180.0);
        add_contents(ui);
    });
}

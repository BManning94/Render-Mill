//! Photo mode, laid out like Lumion: effect list on the left, a framed
//! preview in the middle with a camera bar under it, the photo-set filmstrip
//! along the bottom and a big render button.

use super::*;
use crate::env::{EffectKind, Environment, PhotoEffect};
use crate::ui::icons::{self, Icon};
use crate::ui::widgets;
use egui::{pos2, Rect, Sense, Stroke, UiBuilder};

const STRIP_H: f32 = 118.0;
const LEFT_W: f32 = 290.0;
const CAMBAR_H: f32 = 40.0;

type StyleFn = fn(&mut Environment);

/// Photo styles: each sets an effect stack and its values.
fn styles() -> Vec<(&'static str, StyleFn)> {
    fn set(e: &mut Environment, kinds: &[EffectKind]) {
        e.effects = kinds.iter().map(|&kind| PhotoEffect { kind, enabled: true }).collect();
    }
    use EffectKind::*;
    vec![
        ("Realistic", |e| {
            let d = Environment::default();
            (e.exposure, e.contrast, e.saturation, e.bloom, e.ao, e.fog) = (d.exposure, d.contrast, d.saturation, d.bloom, d.ao, d.fog);
            set(e, &[ColorCorrection, SkyLight, Fog, Shadow, Reflection, Bloom]);
        }),
        ("Interior", |e| {
            (e.exposure, e.contrast, e.saturation, e.bloom, e.vignette, e.ao) = (1.35, 1.0, 1.0, 0.5, 0.2, 1.6);
            set(e, &[ColorCorrection, SkyLight, Shadow, Reflection, Bloom, Vignette]);
        }),
        ("Golden", |e| {
            (e.exposure, e.contrast, e.saturation, e.bloom, e.vignette, e.ao) = (1.15, 1.1, 1.18, 0.9, 0.3, 1.0);
            set(e, &[ColorCorrection, SkyLight, Fog, Shadow, Reflection, Bloom, Vignette]);
        }),
        ("Dramatic", |e| {
            (e.exposure, e.contrast, e.saturation, e.bloom, e.vignette, e.ao, e.sharpen) = (1.0, 1.35, 0.9, 0.6, 0.6, 1.5, 0.3);
            set(e, &[ColorCorrection, SkyLight, Fog, Shadow, Reflection, Bloom, Vignette, Sharpen]);
        }),
        ("Clean", |e| {
            (e.exposure, e.contrast, e.saturation, e.ao) = (1.1, 0.95, 0.95, 0.8);
            set(e, &[ColorCorrection, SkyLight, Shadow, Reflection]);
        }),
        ("Black & white", |e| {
            (e.saturation, e.contrast) = (0.0, 1.2);
            set(e, &[ColorCorrection, SkyLight, Shadow, Reflection, Vignette]);
        }),
    ]
}

const ASPECTS: [((u32, u32), &str); 7] = [
    ((1920, 1080), "16:9"),
    ((1600, 1200), "4:3"),
    ((1800, 1200), "3:2"),
    ((1500, 1500), "1:1"),
    ((2520, 1080), "21:9"),
    ((1080, 1920), "9:16"),
    ((2480, 3508), "A4"),
];

impl LumenApp {
    pub(super) fn photo_mode(&mut self, ui: &mut egui::Ui) {
        let area = ui.max_rect();
        let strip = Rect::from_min_max(pos2(area.left(), area.bottom() - STRIP_H), area.max);
        let left = Rect::from_min_max(area.min + vec2(8.0, 8.0), pos2(area.left() + 8.0 + LEFT_W, strip.top() - 8.0));
        let stage = Rect::from_min_max(pos2(left.right() + 12.0, area.top() + 8.0), pos2(area.right() - 12.0, strip.top() - 8.0));

        // Preview frame with the photo's aspect ratio, centred in the stage.
        let aspect = self.photo_res.0 as f32 / self.photo_res.1 as f32;
        let avail = Rect::from_min_max(stage.min + vec2(20.0, 10.0), stage.max - vec2(20.0, CAMBAR_H + 14.0));
        let (pw, ph) =
            if avail.width() / avail.height() > aspect { (avail.height() * aspect, avail.height()) } else { (avail.width(), avail.width() / aspect) };
        let preview = Rect::from_center_size(avail.center(), vec2(pw.max(32.0), ph.max(32.0)));
        ui.painter().rect_filled(stage, 4.0, Color32::from_rgb(28, 29, 31));
        self.viewport(ui, preview);
        ui.painter().rect_stroke(preview, 0.0, Stroke::new(1.0_f32, Color32::from_white_alpha(30)));
        ui.painter().text(
            preview.left_bottom() + vec2(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            if self.photo_lock { "Camera locked." } else { "Click inside the preview window to update.  Right-drag to look around, WASD to move." },
            egui::FontId::proportional(11.0),
            Color32::from_white_alpha(200),
        );

        // Camera bar under the preview.
        let bar = Rect::from_center_size(pos2(preview.center().x, preview.bottom() + 8.0 + CAMBAR_H / 2.0), vec2(700.0, CAMBAR_H));
        ui.allocate_new_ui(UiBuilder::new().max_rect(bar), |ui| self.camera_bar(ui));

        ui.allocate_new_ui(UiBuilder::new().max_rect(left), |ui| {
            theme::panel_frame().show(ui, |ui| {
                ui.set_width(LEFT_W - 24.0);
                ui.set_min_height(left.height() - 24.0);
                self.effects_panel(ui);
            });
        });
        ui.allocate_new_ui(UiBuilder::new().max_rect(strip), |ui| self.filmstrip(ui, strip));
    }

    fn camera_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            icons::draw(ui.painter(), Icon::Photo, r, theme::TEXT);
            // Focal length from the vertical field of view (24 mm-high sensor).
            let cam = &mut self.world.camera;
            let mut focal = 12.0 / (cam.fov.to_radians() * 0.5).tan();
            let r = ui.add(egui::DragValue::new(&mut focal).range(8.0..=300.0).speed(0.2).max_decimals(1).suffix(" mm"));
            if r.changed() {
                cam.fov = (2.0 * (12.0 / focal).atan()).to_degrees().clamp(5.0, 120.0);
            }
            if widgets::log_bar(ui, "Focal length", &mut focal, 8.0..=300.0, 150.0, |v| format!("{v:.0} mm")).changed() {
                cam.fov = (2.0 * (12.0 / focal).atan()).to_degrees().clamp(5.0, 120.0);
            }
            let mut heading = (-cam.yaw.to_degrees()).rem_euclid(360.0);
            if widgets::bar(ui, "Heading", &mut heading, 0.0..=360.0, 120.0, |v| format!("{v:.0}°")).changed() {
                cam.yaw = -heading.to_radians();
            }
            let mut pitch = cam.pitch.to_degrees();
            if widgets::bar(ui, "Pitch", &mut pitch, -89.0..=89.0, 120.0, |v| format!("{v:.0}°")).changed() {
                cam.pitch = pitch.to_radians();
            }
            let cur = ASPECTS.iter().find(|(r, _)| *r == self.photo_res).map(|(_, n)| *n).unwrap_or("Custom");
            egui::ComboBox::from_id_salt("aspect").width(64.0).selected_text(cur).show_ui(ui, |ui| {
                for (r, n) in ASPECTS {
                    if ui.selectable_label(self.photo_res == r, n).clicked() {
                        self.photo_res = r;
                        self.slot_thumbs.clear();
                    }
                }
            });
            let lock = self.photo_lock;
            if widgets::tool_button(ui, if lock { Icon::Lock } else { Icon::Unlock }, vec2(28.0, 28.0), lock, "Lock the camera").clicked() {
                self.photo_lock = !lock;
            }
            let live = self.rt.live;
            if widgets::tool_button(
                ui,
                Icon::Render,
                vec2(28.0, 28.0),
                live,
                "Ray-traced preview: refine the preview with ray tracing + AI denoising while the camera is still",
            )
            .clicked()
            {
                self.rt.live = !live;
                self.rt.live_job = None;
                self.last_rendered = None;
            }
            if widgets::tool_button(ui, Icon::Menu, vec2(28.0, 28.0), self.render_popup, "Render settings").clicked() {
                self.render_popup = !self.render_popup;
            }
        });
    }

    fn effects_panel(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.label(RichText::new(&self.world.env.style).size(11.5).color(theme::TEXT_DIM));
        ui.horizontal(|ui| {
            let r = ui.menu_button(RichText::new("  Custom style  ").size(12.0), |ui| {
                for (name, f) in styles() {
                    if ui.button(name).clicked() {
                        f(&mut self.world.env);
                        self.world.env.style = name.to_string();
                        self.slot_thumbs.clear();
                        ui.close_menu();
                    }
                }
            });
            r.response.on_hover_text("Apply a style: a ready-made set of effects");
            let missing: Vec<EffectKind> = EffectKind::ALL.iter().copied().filter(|k| !self.world.env.effects.iter().any(|e| e.kind == *k)).collect();
            ui.add_enabled_ui(!missing.is_empty(), |ui| {
                ui.menu_button(RichText::new("  + Add effect  ").size(12.0), |ui| {
                    for k in &missing {
                        if ui.button(k.label()).clicked() {
                            self.world.env.effects.push(PhotoEffect { kind: *k, enabled: true });
                            self.effect_open = Some(*k);
                            self.world.env.style = "Custom style".into();
                            ui.close_menu();
                        }
                    }
                });
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Effect list").size(12.5).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::toggle(ui, &mut self.world.env.effects_on).on_hover_text("Turn all effects on or off");
            });
        });
        let mut remove = None;
        egui::ScrollArea::vertical().id_salt("effects").show(ui, |ui| {
            let n = self.world.env.effects.len();
            for idx in 0..n {
                let PhotoEffect { kind, enabled } = self.world.env.effects[idx].clone();
                let open = self.effect_open == Some(kind);
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                ui.painter().rect_filled(
                    r,
                    2.0,
                    if open {
                        Color32::from_rgb(46, 47, 52)
                    } else if resp.hovered() {
                        Color32::from_rgb(40, 41, 45)
                    } else {
                        Color32::from_rgb(36, 37, 40)
                    },
                );
                ui.painter().text(
                    pos2(r.left() + 10.0, r.center().y),
                    Align2::LEFT_CENTER,
                    kind.label(),
                    egui::FontId::proportional(12.5),
                    if enabled { Color32::WHITE } else { theme::TEXT_DIM },
                );
                let eye = Rect::from_center_size(pos2(r.right() - 18.0, r.center().y), vec2(24.0, 24.0));
                let eye_resp = ui.interact(eye, egui::Id::new(("eye", idx)), Sense::click());
                icons::draw(
                    ui.painter(),
                    if enabled { Icon::Eye } else { Icon::EyeOff },
                    eye.shrink(4.0),
                    if eye_resp.hovered() { Color32::WHITE } else { theme::TEXT_DIM },
                );
                if eye_resp.on_hover_text("Show / hide this effect").clicked() {
                    self.world.env.effects[idx].enabled = !enabled;
                } else if resp.clicked() {
                    self.effect_open = if open { None } else { Some(kind) };
                }
                resp.context_menu(|ui| {
                    if ui.button("Remove effect").clicked() {
                        remove = Some(idx);
                        ui.close_menu();
                    }
                });
                if open {
                    egui::Frame::none().inner_margin(egui::Margin { left: 8.0, right: 0.0, top: 2.0, bottom: 6.0 }).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        let w = ui.available_width();
                        let e = &mut self.world.env;
                        match kind {
                            EffectKind::ColorCorrection => {
                                widgets::log_bar(ui, "Exposure", &mut e.exposure, 0.1..=6.0, w, |v| format!("{v:.2}"));
                                widgets::bar(ui, "Contrast", &mut e.contrast, 0.5..=1.6, w, |v| format!("{v:.2}"));
                                widgets::bar(ui, "Saturation", &mut e.saturation, 0.0..=2.0, w, widgets::pct);
                                ui.checkbox(&mut e.filmic, "Filmic tone curve (camera-like highlights)");
                            }
                            EffectKind::SkyLight => {
                                widgets::bar(ui, "Brightness", &mut e.ambient, 0.0..=3.0, w, |v| format!("{v:.2}"));
                                widgets::bar(ui, "Occlusion", &mut e.ao, 0.0..=2.5, w, |v| format!("{v:.2}"));
                            }
                            EffectKind::Fog => {
                                widgets::bar(ui, "Density", &mut e.fog, 0.0..=2.0, w, |v| format!("{v:.2}"));
                                widgets::bar(ui, "Haze", &mut e.haze, 0.0..=1.0, w, widgets::pct);
                            }
                            EffectKind::Shadow => {
                                widgets::bar(ui, "Softness", &mut e.shadow_softness, 0.0..=2.0, w, |v| format!("{v:.2}"));
                                widgets::bar(ui, "Sun brightness", &mut e.sun_intensity, 0.0..=8.0, w, |v| format!("{v:.1}"));
                            }
                            EffectKind::Reflection => {
                                widgets::bar(ui, "Strength", &mut e.reflections, 0.0..=2.0, w, |v| format!("{v:.2}"));
                            }
                            EffectKind::Bloom => {
                                widgets::bar(ui, "Amount", &mut e.bloom, 0.0..=3.0, w, |v| format!("{v:.2}"));
                            }
                            EffectKind::Vignette => {
                                widgets::bar(ui, "Amount", &mut e.vignette, 0.0..=1.0, w, widgets::pct);
                            }
                            EffectKind::Sharpen => {
                                widgets::bar(ui, "Amount", &mut e.sharpen, 0.0..=1.0, w, widgets::pct);
                            }
                            EffectKind::DepthOfField => {
                                widgets::bar(ui, "Blur amount", &mut e.dof, 0.0..=2.0, w, |v| format!("{v:.2}"));
                                widgets::bar(ui, "Focus distance", &mut e.dof_focus, 0.0..=100.0, w, |v| {
                                    if v < 0.05 {
                                        "Auto".into()
                                    } else {
                                        format!("{v:.1} m")
                                    }
                                });
                                ui.label(RichText::new("Focus at 0 = auto-focus on the centre of the photo.").size(10.5).color(theme::TEXT_DIM));
                            }
                            EffectKind::ChromaticAberration => {
                                widgets::bar(ui, "Amount", &mut e.chroma, 0.0..=2.0, w, |v| format!("{v:.2}"));
                            }
                            EffectKind::FilmGrain => {
                                widgets::bar(ui, "Amount", &mut e.grain, 0.0..=1.0, w, widgets::pct);
                            }
                            EffectKind::AutoExposure => {
                                ui.label(RichText::new("Adjusts exposure to the brightness of the view, like a camera.").size(10.5).color(theme::TEXT_DIM));
                            }
                            EffectKind::Reflections => {
                                widgets::bar(ui, "Strength", &mut e.ssr, 0.0..=1.5, w, widgets::pct);
                                ui.label(RichText::new("Reflects the scene in glass, water and glossy floors.").size(10.5).color(theme::TEXT_DIM));
                            }
                        }
                    });
                }
            }
        });
        if let Some(i) = remove {
            self.world.env.effects.remove(i);
            self.world.env.style = "Custom style".into();
        }
    }

    fn filmstrip(&mut self, ui: &mut egui::Ui, strip: Rect) {
        ui.painter().rect_filled(strip, 0.0, Color32::from_rgb(28, 29, 31));
        let slot = vec2(150.0, 84.0);
        let y = strip.center().y - slot.y / 2.0;
        // Render button (big blue square on the right).
        let rb = Rect::from_min_size(pos2(strip.right() - 96.0, strip.center().y - 42.0), vec2(84.0, 84.0));
        let rresp = ui.interact(rb, egui::Id::new("render_btn"), Sense::click());
        ui.painter().rect_filled(rb, 2.0, if rresp.hovered() { Color32::from_rgb(52, 152, 240) } else { theme::ACCENT });
        icons::draw(ui.painter(), Icon::Render, Rect::from_center_size(rb.center(), vec2(36.0, 36.0)), Color32::WHITE);
        if rresp.on_hover_text("Render").clicked() {
            self.render_popup = !self.render_popup;
        }
        // Photo-set icon on the left.
        let set_r = Rect::from_min_size(pos2(strip.left() + 14.0, strip.center().y - 14.0), vec2(28.0, 28.0));
        icons::draw(ui.painter(), Icon::Film, set_r, theme::TEXT_DIM);
        let mut x = strip.left() + 56.0;
        let mut action: Option<(usize, u8)> = None; // 0 go, 1 save, 2 delete
        for i in 0..PHOTO_SLOTS {
            if x + slot.x > rb.left() - 12.0 {
                break;
            }
            let r = Rect::from_min_size(pos2(x, y), slot);
            let filled = i < self.world.views.len();
            if filled && !self.slot_thumbs.contains_key(&i) {
                self.slot_thumbnail(i);
            }
            let resp = ui.interact(r, egui::Id::new(("slot", i)), Sense::click());
            ui.painter().rect_filled(r, 2.0, Color32::from_rgb(44, 45, 49));
            if let Some(t) = self.slot_thumbs.get(&i) {
                ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                icons::draw(ui.painter(), Icon::Photo, Rect::from_center_size(r.center(), vec2(24.0, 24.0)), Color32::from_white_alpha(70));
            }
            let current = filled && self.world.views[i].camera == self.world.camera;
            let stroke = if current {
                Stroke::new(2.0_f32, Color32::WHITE)
            } else if resp.hovered() {
                Stroke::new(1.5_f32, Color32::from_white_alpha(150))
            } else {
                Stroke::NONE
            };
            ui.painter().rect_stroke(r, 2.0, stroke);
            ui.painter().text(r.left_top() + vec2(5.0, 4.0), Align2::LEFT_TOP, format!("{}", i + 1), egui::FontId::proportional(10.5), Color32::WHITE);
            let tip = if filled { "Click: go to this photo · right-click: update / delete" } else { "Click: save the current camera here (Ctrl+1..9)" };
            if resp.clone().on_hover_text(tip).clicked() {
                action = Some((i, if filled { 0 } else { 1 }));
            }
            resp.context_menu(|ui| {
                if ui.button("Update with current view").clicked() {
                    action = Some((i, 1));
                    ui.close_menu();
                }
                if filled && ui.button("Delete").clicked() {
                    action = Some((i, 2));
                    ui.close_menu();
                }
            });
            x += slot.x + 8.0;
        }
        match action {
            Some((i, 0)) => self.world.camera = self.world.views[i].camera.clone(),
            Some((i, 1)) => self.save_slot(i.min(self.world.views.len())),
            Some((i, 2)) => {
                self.world.views.remove(i);
                self.slot_thumbs.clear();
            }
            _ => {}
        }
    }

    pub(super) fn photo_overlays(&mut self, ctx: &egui::Context) {
        if !self.render_popup {
            return;
        }
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("render_popup"))
            .fixed_pos(pos2(screen.right() - 360.0, screen.bottom() - STRIP_H - 370.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                theme::panel_frame().show(ui, |ui| {
                    ui.set_width(320.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Render photo").size(14.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::tool_button(ui, Icon::Close, vec2(22.0, 22.0), false, "Close").clicked() {
                                self.render_popup = false;
                            }
                        });
                    });
                    let aspect = self.photo_res.0 as f32 / self.photo_res.1 as f32;
                    ui.label(RichText::new("Size").color(theme::TEXT_DIM));
                    ui.horizontal_wrapped(|ui| {
                        for (long, label) in [(1280u32, "HD"), (1920, "Full HD"), (2560, "QHD"), (3840, "4K"), (7680, "8K")] {
                            let r = if aspect >= 1.0 { (long, (long as f32 / aspect).round() as u32) } else { ((long as f32 * aspect).round() as u32, long) };
                            if ui.selectable_label(self.photo_res == r, label).on_hover_text(format!("{} x {}", r.0, r.1)).clicked() {
                                self.photo_res = r;
                            }
                        }
                    });
                    ui.label(
                        RichText::new(format!("{} x {} px · rendered at the highest quality", self.photo_res.0, self.photo_res.1))
                            .size(11.0)
                            .color(theme::TEXT_DIM),
                    );
                    ui.add_space(4.0);
                    ui.label(RichText::new("Ray tracing").color(theme::TEXT_DIM))
                        .on_hover_text("Path-traced global illumination, soft shadows, true reflections and refraction.\nOff = fast rasterized render.");
                    ui.horizontal(|ui| {
                        for (i, label) in super::raytrace::LEVELS.iter().enumerate() {
                            let tip = match crate::pathtrace::PtOpts::preset(i.saturating_sub(1) as u32) {
                                o if i > 0 => format!("{} samples per pixel, {} bounces", o.spp, o.bounces),
                                _ => "Rasterized (no ray tracing)".to_string(),
                            };
                            if ui.selectable_label(self.rt.level == i as u32, *label).on_hover_text(tip).clicked() {
                                self.rt.level = i as u32;
                            }
                        }
                    });
                    ui.add_enabled_ui(self.rt.level > 0, |ui| {
                        let ai = crate::denoise::ai_available();
                        let label = if ai { "AI denoiser (Open Image Denoise)" } else { "Denoiser (AI module not found, uses wavelet filter)" };
                        ui.checkbox(&mut self.rt.denoise, label)
                            .on_hover_text("Removes the remaining grain from the ray-traced image with a neural network trained on path-traced renders.");
                    });
                    // GPU limits for the selected size (checked before rendering starts).
                    let (pw, ph) = self.photo_res;
                    let issue = match crate::pathtrace::check_resolution(&self.renderer.device.limits(), pw, ph, self.rt.level > 0) {
                        Err(e) => Some(e),
                        // An out-of-memory failure from the last attempt at this size.
                        Ok(()) => self.render_issue.clone().filter(|e| e.requested == (pw, ph)),
                    };
                    if let Some(e) = issue {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!("{} x {} can't be rendered on this graphics card: {}.", pw, ph, e.reason)).size(11.0).color(theme::WARN),
                        );
                        let (mw, mh) = e.max_supported;
                        if widgets::flat_button(ui, None, &format!("Use {mw} x {mh} (highest supported)"), false, 296.0).clicked() {
                            self.photo_res = (mw, mh);
                            self.render_issue = None;
                        }
                    }
                    // Texture budget: say when maps are shown below their own resolution.
                    for n in self.texture_notes() {
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("{n}. Raise it under Settings › Texture memory if the card has room.")).size(11.0).color(theme::WARN));
                    }
                    ui.add_space(6.0);
                    if widgets::flat_button(ui, Some(Icon::Render), "Render this photo", true, 296.0).clicked() {
                        self.render_popup = false;
                        self.render_photo();
                    }
                    let n = self.world.views.len();
                    ui.add_enabled_ui(n > 0, |ui| {
                        if widgets::flat_button(ui, Some(Icon::Film), &format!("Render photo set ({n})"), false, 296.0).clicked() {
                            self.render_popup = false;
                            self.render_photo_set();
                        }
                    });
                });
            });
    }
}

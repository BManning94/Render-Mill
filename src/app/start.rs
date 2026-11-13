//! Start screen: Create new (landscape templates), Examples, Load, recent
//! projects - shown at launch and from File › New project.

use super::*;
use crate::landscape::TEMPLATES;
use crate::ui::icons::Icon;
use crate::ui::widgets;
use egui::{pos2, Rect, Sense, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum StartPage {
    Home,
    Templates,
}

/// Thumbnails rendered with `lumencraft --render --template:<id>` and
/// embedded in the exe.
fn thumb_bytes(id: &str) -> Option<&'static [u8]> {
    Some(match id {
        "meadow" => include_bytes!("../../assets/templates/meadow.jpg"),
        "valley" => include_bytes!("../../assets/templates/valley.jpg"),
        "lakeside" => include_bytes!("../../assets/templates/lakeside.jpg"),
        "forest" => include_bytes!("../../assets/templates/forest.jpg"),
        "coast" => include_bytes!("../../assets/templates/coast.jpg"),
        "desert" => include_bytes!("../../assets/templates/desert.jpg"),
        "winter" => include_bytes!("../../assets/templates/winter.jpg"),
        "evening" => include_bytes!("../../assets/templates/evening.jpg"),
        "flat" => include_bytes!("../../assets/templates/flat.jpg"),
        _ => return None,
    })
}

impl LumenApp {
    fn template_thumb(&mut self, ctx: &egui::Context, id: &str) -> Option<egui::TextureHandle> {
        let key = format!("template:{id}");
        if let Some(t) = self.start_thumbs.get(&key) {
            return Some(t.clone());
        }
        let img = image::load_from_memory(thumb_bytes(id)?).ok()?.to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], &img);
        let t = ctx.load_texture(key.clone(), ci, egui::TextureOptions::LINEAR);
        self.start_thumbs.insert(key, t.clone());
        Some(t)
    }

    /// Draws the start screen; returns true while it covers the app.
    pub(super) fn start_screen(&mut self, ctx: &egui::Context) -> bool {
        let Some(page) = self.start_page else { return false };
        // Building a template takes a moment: show "Loading" for one frame first.
        if let Some(id) = self.pending_template.take() {
            if self.start_loading_drawn {
                self.start_loading_drawn = false;
                self.create_from_template(&id);
                self.start_page = None;
                return false;
            }
            self.pending_template = Some(id);
            self.start_loading_drawn = true;
        }
        let loading = self.pending_template.clone();
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::PHOTO_BG)).show(ctx, |ui| {
            let full = ui.max_rect();
            // Soft diagonal bands in the background, like Lumion's start screen.
            let p = ui.painter();
            for k in 0..5 {
                let y = full.top() + full.height() * (0.45 + k as f32 * 0.12);
                let pts: Vec<egui::Pos2> = (0..=40)
                    .map(|i| {
                        let t = i as f32 / 40.0;
                        pos2(full.left() + t * full.width(), y + (t * 5.0 + k as f32).sin() * 40.0 - t * 80.0)
                    })
                    .collect();
                p.add(egui::Shape::line(pts, Stroke::new(50.0_f32, Color32::from_rgb(39, 40, 44))));
            }
            if let Some(id) = &loading {
                let name = TEMPLATES.iter().find(|t| t.id == id).map_or("project", |t| t.name);
                p.text(full.center() + vec2(0.0, -12.0), Align2::CENTER_CENTER, crate::APP_NAME, egui::FontId::proportional(34.0), theme::TEXT);
                p.text(
                    full.center() + vec2(0.0, 26.0),
                    Align2::CENTER_CENTER,
                    format!("Creating {name}..."),
                    egui::FontId::proportional(13.0),
                    theme::TEXT_DIM,
                );
                ctx.request_repaint();
                return;
            }
            let card_w = if page == StartPage::Home { 440.0 } else { 640.0 };
            egui::Area::new(egui::Id::new("start_card")).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(crate::APP_NAME).size(34.0).color(theme::TEXT));
                    ui.add_space(14.0);
                });
                theme::panel_frame().inner_margin(egui::Margin::same(16.0)).show(ui, |ui| {
                    ui.set_width(card_w);
                    match page {
                        StartPage::Home => self.start_home(ui),
                        StartPage::Templates => self.start_templates(ui),
                    }
                });
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            match page {
                StartPage::Templates => self.start_page = Some(StartPage::Home),
                // Back to the open project (if there is one to go back to).
                StartPage::Home if self.start_can_close => self.start_page = None,
                _ => {}
            }
        }
        true
    }

    fn start_home(&mut self, ui: &mut egui::Ui) {
        let w = (ui.available_width() - 16.0) / 3.0;
        ui.horizontal(|ui| {
            if widgets::flat_button(ui, Some(Icon::Plus), "Create new", true, w).clicked() {
                self.start_page = Some(StartPage::Templates);
            }
            if widgets::flat_button(ui, Some(Icon::Grid), "Example", false, w).on_hover_text("The demo house on a flat lawn").clicked() {
                self.load_demo();
                self.start_page = None;
            }
            if widgets::flat_button(ui, Some(Icon::Folder), "Load", false, w).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Render Mill project", &["lcproj"]).pick_file() {
                    self.open_project(p);
                    self.start_page = None;
                }
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if widgets::flat_button(ui, Some(Icon::Sync), "Start from a Revit / 3D model...", false, ui.available_width()).clicked() {
                self.start_page = None;
                self.new_project();
                self.open_model_dialog();
            }
        });
        let recent: Vec<PathBuf> = self.settings.recent_projects.iter().filter(|p| p.exists()).take(6).cloned().collect();
        if !recent.is_empty() {
            ui.add_space(12.0);
            theme::section_title(ui, "Recent projects");
            ui.add_space(4.0);
            for p in recent {
                let name = p.file_stem().map_or(String::new(), |s| s.to_string_lossy().to_string());
                let r = ui
                    .add(egui::Button::new(RichText::new(name).size(12.5)).frame(false).min_size(vec2(ui.available_width(), 22.0)))
                    .on_hover_text(p.display().to_string());
                if r.clicked() {
                    self.open_project(p);
                    self.start_page = None;
                }
            }
        }
        if self.start_can_close {
            ui.add_space(10.0);
            if ui.small_button("Back to the current project").clicked() {
                self.start_page = None;
            }
        }
    }

    fn start_templates(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("<").on_hover_text("Back").clicked() {
                self.start_page = Some(StartPage::Home);
            }
            ui.add_space(ui.available_width() / 2.0 - 80.0);
            ui.label(RichText::new("Create new project").size(15.0).color(theme::TEXT));
        });
        ui.add_space(10.0);
        let ctx = ui.ctx().clone();
        let cols = 3;
        let gap = 8.0;
        let tw = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
        let th = tw * 9.0 / 16.0;
        for row in TEMPLATES.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(gap, 0.0);
                for t in row {
                    let thumb = self.template_thumb(&ctx, t.id);
                    let (rect, resp) = ui.allocate_exact_size(vec2(tw, th + 20.0), Sense::click());
                    let img = Rect::from_min_size(rect.min, vec2(tw, th));
                    let p = ui.painter();
                    match &thumb {
                        Some(tex) => {
                            p.image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        None => {
                            p.rect_filled(img, 3.0, theme::TILE);
                        }
                    }
                    if resp.hovered() {
                        p.rect_stroke(img, 2.0, Stroke::new(2.0_f32, theme::ACCENT));
                    }
                    p.text(pos2(img.left(), img.bottom() + 4.0), Align2::LEFT_TOP, t.name, egui::FontId::proportional(12.0), theme::TEXT);
                    let resp = resp.on_hover_text(t.description);
                    if resp.clicked() {
                        self.pending_template = Some(t.id.to_string());
                    }
                }
            });
            ui.add_space(gap);
        }
    }

    pub(super) fn create_from_template(&mut self, id: &str) {
        let t0 = Instant::now();
        self.new_project();
        if self.world.apply_template(id) {
            let name = TEMPLATES.iter().find(|t| t.id == id).map_or(id, |t| t.name);
            self.push_log(format!("New project from the {name} template ({:.1}s)", t0.elapsed().as_secs_f32()));
        }
        self.last_rendered = None;
    }

    /// File › New project: the template gallery.
    pub(super) fn show_new_project_page(&mut self) {
        self.start_can_close = true;
        self.start_page = Some(StartPage::Templates);
    }
}

//! Window chrome: custom title bar, left icon sidebar, slide-out panels
//! (file, activity, settings, help, LiveSync), the top-right cards, the view
//! pill and the shortcut hints on the right edge.

use super::*;
use crate::ui::icons::{self, Icon};
use crate::ui::widgets;
use egui::{pos2, Rect, Sense, Stroke};

impl LumenApp {
    pub(super) fn fps(&self) -> f32 {
        if self.frame_times.is_empty() {
            0.0
        } else {
            self.frame_times.len() as f32 / self.frame_times.iter().sum::<f32>()
        }
    }

    /// Borderless window: let the edges resize it.
    pub(super) fn window_resize_edges(&mut self, ctx: &egui::Context) {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if maximized {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
        let r = ctx.screen_rect();
        let m = 5.0;
        let (l, rt, t, b) = (pos.x < r.left() + m, pos.x > r.right() - m, pos.y < r.top() + m, pos.y > r.bottom() - m);
        use egui::viewport::ResizeDirection as D;
        let dir = match (l, rt, t, b) {
            (true, _, true, _) => Some(D::NorthWest),
            (_, true, true, _) => Some(D::NorthEast),
            (true, _, _, true) => Some(D::SouthWest),
            (_, true, _, true) => Some(D::SouthEast),
            (true, _, _, _) => Some(D::West),
            (_, true, _, _) => Some(D::East),
            (_, _, true, _) => Some(D::North),
            (_, _, _, true) => Some(D::South),
            _ => None,
        };
        if let Some(d) = dir {
            ctx.set_cursor_icon(match d {
                D::North | D::South => egui::CursorIcon::ResizeVertical,
                D::East | D::West => egui::CursorIcon::ResizeHorizontal,
                D::NorthWest | D::SouthEast => egui::CursorIcon::ResizeNwSe,
                D::NorthEast | D::SouthWest => egui::CursorIcon::ResizeNeSw,
            });
            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(d));
            }
        }
    }

    pub(super) fn title_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("titlebar").exact_height(TITLE_H).frame(egui::Frame::none().fill(theme::TITLE_BAR)).show(ctx, |ui| {
            let rect = ui.max_rect();
            // Drag the window by its title bar; double-click toggles maximize.
            let bar = ui.interact(rect, egui::Id::new("titlebar_drag"), Sense::click_and_drag());
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            if bar.double_clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            } else if bar.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            let p = ui.painter();
            icons::draw(p, Icon::Logo, Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(18.0, 18.0)), Color32::WHITE);
            let project = self.project_path.as_ref().and_then(|p| p.file_stem()).map(|s| format!("  —  {}", s.to_string_lossy())).unwrap_or_default();
            p.text(
                pos2(rect.left() + 44.0, rect.center().y),
                Align2::LEFT_CENTER,
                format!("{} {}{project}", crate::APP_NAME, env!("CARGO_PKG_VERSION")),
                egui::FontId::proportional(12.5),
                theme::TEXT,
            );

            // Right side: window buttons, quality stars, frame-rate gauge.
            let mut x = rect.right();
            let bw = 44.0;
            let win_btn = |ui: &mut egui::Ui, x: f32, icon: Icon, danger: bool| -> bool {
                let r = Rect::from_min_size(pos2(x - bw, rect.top()), vec2(bw, rect.height()));
                let resp = ui.interact(r, egui::Id::new(("winbtn", icon as u8)), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, if danger { Color32::from_rgb(196, 43, 28) } else { Color32::from_rgb(52, 52, 56) });
                }
                icons::draw(ui.painter(), icon, Rect::from_center_size(r.center(), vec2(14.0, 14.0)), Color32::WHITE);
                resp.clicked()
            };
            if win_btn(ui, x, Icon::Close, true) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            x -= bw;
            if win_btn(ui, x, if maximized { Icon::Restore } else { Icon::Maximize }, false) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            x -= bw;
            if win_btn(ui, x, Icon::Minimize, false) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            x -= bw + 14.0;

            // Quality stars (F1-F4), like Lumion's render-quality indicator.
            let mut new_q = None;
            for s in (1..=4u8).rev() {
                let r = Rect::from_center_size(pos2(x - 8.0, rect.center().y), vec2(16.0, 16.0));
                let resp = ui.interact(r, egui::Id::new(("qstar", s)), Sense::click());
                let col = if s <= self.quality { Color32::from_rgb(255, 196, 70) } else { Color32::from_white_alpha(45) };
                icons::draw(ui.painter(), Icon::StarFilled, r.shrink(1.0), col);
                if resp.on_hover_text(format!("Viewport quality {s} (F{s}). Photos always render at full quality.")).clicked() {
                    new_q = Some(s);
                }
                x -= 17.0;
            }
            x -= 14.0;

            // Frame-rate gauge: green arc + number.
            let fps = self.fps();
            let p = ui.painter();
            let num = format!("{:.0}", fps);
            let g = p.layout_no_wrap(num, egui::FontId::proportional(13.0), theme::TEXT);
            let tx = x - g.size().x;
            p.galley(pos2(tx, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
            let c = pos2(tx - 20.0, rect.center().y + 5.0);
            let col = if fps >= 40.0 {
                theme::GOOD
            } else if fps >= 20.0 {
                theme::WARN
            } else {
                Color32::from_rgb(230, 70, 60)
            };
            let arc: Vec<egui::Pos2> = (0..=16)
                .map(|i| {
                    let a = std::f32::consts::PI + std::f32::consts::PI * i as f32 / 16.0;
                    c + vec2(a.cos(), a.sin()) * 11.0
                })
                .collect();
            p.add(egui::Shape::line(arc, Stroke::new(3.5_f32, col)));
            let t = (fps / 90.0).clamp(0.0, 1.0);
            let a = std::f32::consts::PI * (1.0 + t);
            p.line_segment([c, c + vec2(a.cos(), a.sin()) * 9.0], Stroke::new(1.6_f32, Color32::WHITE));
            let gauge = Rect::from_center_size(c - vec2(0.0, 4.0), vec2(48.0, 22.0));
            ui.interact(gauge.union(Rect::from_min_size(pos2(tx, rect.top()), vec2(30.0, rect.height()))), egui::Id::new("fpsgauge"), Sense::hover())
                .on_hover_text(format!(
                    "{fps:.0} fps · {:.1} ms GPU frame{} · {} triangles",
                    self.frame_ms,
                    if self.showing_low { format!(" · {}% resolution while moving", (self.move_scale * 100.0) as u32) } else { String::new() },
                    self.world.scene.triangle_count()
                ));
            if let Some(q) = new_q {
                self.set_quality(q);
            }
        });
    }

    pub(super) fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar").exact_width(SIDEBAR_W).resizable(false).frame(egui::Frame::none().fill(theme::SIDEBAR)).show(ctx, |ui| {
            let rect = ui.max_rect();
            let mut y = rect.top() + 4.0;
            let item = |ui: &mut egui::Ui, y: f32, icon: Icon, on: bool, tip: &str| -> bool {
                let r = Rect::from_min_size(pos2(rect.left(), y), vec2(SIDEBAR_W, 38.0));
                let resp = ui.interact(r, egui::Id::new(("side", icon as u8)), Sense::click());
                if on {
                    ui.painter().rect_filled(r, 0.0, theme::SELECTED);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, Color32::from_rgb(46, 46, 50));
                }
                icons::draw(
                    ui.painter(),
                    icon,
                    Rect::from_center_size(r.center(), vec2(18.0, 18.0)),
                    if on || resp.hovered() { Color32::WHITE } else { Color32::from_rgb(190, 192, 198) },
                );
                resp.on_hover_text(tip).clicked()
            };
            if item(ui, y, Icon::Save, self.side == Some(SidePanel::File), "File: save, open, import") {
                self.toggle_side(SidePanel::File);
            }
            y += 42.0;
            if item(ui, y, Icon::Walker, self.mode == Mode::Build, "Build mode") {
                self.mode = Mode::Build;
            }
            y += 38.0;
            if item(ui, y, Icon::Photo, self.mode == Mode::Photo, "Photo mode") {
                self.mode = Mode::Photo;
                self.drag = None;
                self.obj_cat = None;
            }
            y += 38.0;
            if item(ui, y, Icon::Film, self.mode == Mode::Movie, "Movie mode") {
                self.mode = Mode::Movie;
                self.drag = None;
                self.obj_cat = None;
            }
            y += 38.0;
            if item(ui, y, Icon::Sync, self.side == Some(SidePanel::LiveSync), "Imported model & LiveSync") {
                self.toggle_side(SidePanel::LiveSync);
            }
            let mut yb = rect.bottom() - 42.0;
            if item(ui, yb, Icon::Help, self.side == Some(SidePanel::Help), "Keyboard & mouse shortcuts") {
                self.toggle_side(SidePanel::Help);
            }
            yb -= 40.0;
            if item(ui, yb, Icon::Gear, self.side == Some(SidePanel::Settings), "Settings") {
                self.toggle_side(SidePanel::Settings);
            }
            yb -= 40.0;
            if item(ui, yb, Icon::Chat, self.side == Some(SidePanel::Log), "Activity log") {
                self.toggle_side(SidePanel::Log);
            }
        });
    }

    pub(super) fn toggle_side(&mut self, p: SidePanel) {
        self.side = if self.side == Some(p) { None } else { Some(p) };
    }

    /// Slide-out panels next to the sidebar (and the LiveSync panel on the right).
    pub(super) fn side_panels(&mut self, ctx: &egui::Context) {
        let Some(panel) = self.side else { return };
        let top = TITLE_H + 6.0;
        if panel == SidePanel::LiveSync {
            let screen = ctx.screen_rect();
            egui::Area::new(egui::Id::new("livesync_panel")).fixed_pos(pos2(screen.right() - 334.0, TITLE_H + 100.0)).show(ctx, |ui| {
                theme::panel_frame().show(ui, |ui| {
                    ui.set_width(300.0);
                    self.livesync_panel(ui);
                });
            });
            return;
        }
        egui::Area::new(egui::Id::new("side_panel")).fixed_pos(pos2(SIDEBAR_W + 6.0, top)).show(ctx, |ui| {
            theme::panel_frame().show(ui, |ui| match panel {
                SidePanel::File => self.file_panel(ui, ctx),
                SidePanel::Log => {
                    ui.set_width(380.0);
                    theme::section_title(ui, "Activity");
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        for (t, l) in &self.log {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(format!("{:>4}s", t.elapsed().as_secs())).monospace().size(10.5).color(theme::TEXT_DIM));
                                ui.label(RichText::new(l).size(11.5));
                            });
                        }
                    });
                }
                SidePanel::Settings => self.settings_panel(ui),
                SidePanel::Help => {
                    ui.set_width(330.0);
                    theme::section_title(ui, "Navigation");
                    for (k, d) in [
                        ("W A S D / arrows", "Move"),
                        ("Q / E", "Down / up"),
                        ("Right mouse drag", "Look around"),
                        ("O + right drag", "Orbit around the screen centre"),
                        ("Middle mouse drag", "Pan"),
                        ("Scroll", "Zoom in / out"),
                        ("Shift / Space / both", "Fast / slow / extra fast"),
                        ("F", "Frame the model"),
                    ] {
                        Self::help_row(ui, k, d);
                    }
                    ui.add_space(6.0);
                    theme::section_title(ui, "Objects");
                    for (k, d) in [
                        ("M / H / R / L", "Move / height / rotate / scale"),
                        ("Ctrl + click", "Add to selection"),
                        ("Ctrl + A", "Select all objects"),
                        ("Delete", "Delete selection"),
                        ("Esc", "Deselect"),
                        ("Ctrl + Z / Ctrl + Y", "Undo / redo"),
                    ] {
                        Self::help_row(ui, k, d);
                    }
                    ui.add_space(6.0);
                    theme::section_title(ui, "Views & quality");
                    for (k, d) in [
                        ("Ctrl + 1..9", "Save camera to photo slot"),
                        ("Shift + 1..9", "Go to photo slot"),
                        ("F1 - F4", "Viewport quality"),
                        ("Ctrl + S", "Save project"),
                    ] {
                        Self::help_row(ui, k, d);
                    }
                }
                SidePanel::LiveSync => {}
            });
        });
    }

    fn help_row(ui: &mut egui::Ui, key: &str, desc: &str) {
        ui.horizontal(|ui| {
            ui.add_sized([130.0, 18.0], egui::Label::new(RichText::new(key).size(11.5).strong()));
            ui.label(RichText::new(desc).size(11.5).color(theme::TEXT_DIM));
        });
    }

    fn file_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.set_width(250.0);
        let mut close = false;
        let mut item = |ui: &mut egui::Ui, icon: Icon, text: &str| -> bool {
            let (r, resp) = ui.allocate_exact_size(vec2(250.0, 30.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, 2.0, Color32::from_rgb(50, 51, 56));
            }
            icons::draw(ui.painter(), icon, Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(15.0, 15.0)), theme::TEXT);
            ui.painter().text(pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, text, egui::FontId::proportional(12.5), theme::TEXT);
            if resp.clicked() {
                close = true;
            }
            resp.clicked()
        };
        if item(ui, Icon::Plus, "New project...") {
            self.show_new_project_page();
        }
        if item(ui, Icon::Folder, "Open project...") {
            if let Some(p) = rfd::FileDialog::new()
                .add_filter("Render Mill or Lumion project", &["lcproj", "lsf", "lqf"])
                .add_filter("Render Mill project", &["lcproj"])
                .add_filter("Lumion scene", &["lsf", "lqf"])
                .pick_file()
            {
                self.open_any(p);
            }
        }
        if item(ui, Icon::Save, "Save project   Ctrl+S") {
            self.save_project(false);
        }
        if item(ui, Icon::Save, "Save project as...") {
            self.save_project(true);
        }
        ui.separator();
        if item(ui, Icon::Import, "Import model (FBX, glTF, OBJ)...") {
            self.open_model_dialog();
        }
        if item(ui, Icon::Photo, "Match materials from a photo...") {
            self.start_photo_match();
        }
        if item(ui, Icon::Materials, "Import textures...") {
            self.import_textures_dialog();
        }
        if item(ui, Icon::Home, "Load demo scene") {
            self.load_demo();
        }
        ui.separator();
        if item(ui, Icon::Undo, "Undo   Ctrl+Z") {
            self.undo();
        }
        if item(ui, Icon::Close, "Quit") {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if close {
            self.side = None;
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.set_width(320.0);
        theme::section_title(ui, "Viewport");
        ui.horizontal(|ui| {
            ui.label(RichText::new("Quality").color(theme::TEXT_DIM));
            for q in 1..=4u8 {
                if ui.selectable_label(self.quality == q, format!("{q}")).on_hover_text(format!("F{q}")).clicked() {
                    self.set_quality(q);
                }
            }
        });
        let mut a = self.adaptive_res;
        ui.horizontal(|ui| {
            if widgets::toggle(ui, &mut a).changed() {
                self.adaptive_res = a;
                self.last_rendered = None;
            }
            ui.label("Lower resolution while moving if the frame rate drops");
        });
        ui.horizontal(|ui| {
            widgets::toggle(ui, &mut self.world.env.animate);
            ui.label("Animate clouds, water and plants");
        });
        widgets::log_bar(ui, "Walk speed", &mut self.move_speed, 0.5..=200.0, 300.0, |v| format!("{v:.1} m/s"));
        ui.add_space(8.0);
        theme::section_title(ui, "Graphics card");
        ui.label(RichText::new(format!("In use: {}", self.gpu_in_use)).size(11.5));
        ui.label(
            RichText::new("Pick the card Render Mill renders on (e.g. your eGPU or the laptop GPU). Takes effect after a restart.")
                .size(11.0)
                .color(theme::TEXT_DIM),
        );
        let mut pick: Option<Option<String>> = None;
        if ui.selectable_label(self.settings.gpu.is_none(), "Automatic (Windows' high-performance GPU)").clicked() {
            pick = Some(None);
        }
        for g in self.gpus.clone() {
            if ui.selectable_label(self.settings.gpu.as_deref() == Some(g.as_str()), &g).clicked() {
                pick = Some(Some(g));
            }
        }
        if let Some(p) = pick {
            if p != self.settings.gpu {
                self.settings.gpu = p;
                self.settings.save();
                self.gpu_changed = true;
            }
        }
        if self.gpu_changed {
            ui.label(RichText::new("Restart Render Mill to switch graphics card.").size(11.0).color(theme::WARN));
        }
        ui.add_space(8.0);
        theme::section_title(ui, "Texture memory");
        ui.label(
            RichText::new("Large projects: when the maps don't fit this budget, the largest are shown at lower resolution (down to 128 px) instead of running the graphics card out of memory.")
                .size(11.0)
                .color(theme::TEXT_DIM),
        );
        let mut auto = self.settings.texture_budget_mb.is_none();
        ui.horizontal(|ui| {
            if widgets::toggle(ui, &mut auto).changed() {
                self.settings.texture_budget_mb = if auto { None } else { Some(2048) };
                self.settings.save();
            }
            ui.label("Automatic (2 GB viewport, 3 GB ray tracing)");
        });
        let mut dragging = false;
        if let Some(mb) = self.settings.texture_budget_mb {
            let mut v = mb as f32;
            let r = widgets::log_bar(ui, "Budget", &mut v, 256.0..=16384.0, 300.0, |v| format!("{:.0} MB", v));
            let nv = ((v / 64.0).round() * 64.0) as u32;
            if nv != mb {
                self.settings.texture_budget_mb = Some(nv);
            }
            dragging = r.dragged();
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                self.settings.save();
            }
        }
        let st = self.renderer.texture_stats();
        ui.label(RichText::new(format!("Viewport textures: {} MB of {} MB", st.bytes.div_ceil(1 << 20), st.budget >> 20)).size(11.5));
        if let Some(t) = &self.rt.tracer {
            let m = t.memory();
            ui.label(RichText::new(format!("Ray tracing textures: {} MB of {} MB", m.textures.div_ceil(1 << 20), m.budget >> 20)).size(11.5));
        }
        for n in self.texture_notes() {
            ui.label(RichText::new(n).size(11.0).color(theme::WARN));
        }
        if !dragging {
            // Re-fitting re-uploads textures: only once the slider is released.
            let mut tracer = self.rt.tracer.take();
            self.apply_texture_budget(tracer.as_mut());
            self.rt.tracer = tracer;
        }
        ui.add_space(8.0);
        theme::section_title(ui, "Material library folders");
        ui.label(
            RichText::new("Folders of your own textures, browsable under Material editor › Library › My folders. Files stay where they are.")
                .size(11.0)
                .color(theme::TEXT_DIM),
        );
        let mut remove = None;
        for (i, f) in self.settings.library_folders.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(f.display().to_string()).size(11.0)).truncate()).on_hover_text(f.display().to_string());
                if ui.small_button("x").clicked() {
                    remove = Some(i);
                }
            });
        }
        let mut changed = false;
        if let Some(i) = remove {
            self.settings.library_folders.remove(i);
            changed = true;
        }
        ui.horizontal(|ui| {
            if ui.small_button("+ Add library folder...").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    if !self.settings.library_folders.contains(&d) {
                        self.settings.library_folders.push(d);
                        changed = true;
                    }
                }
            }
            if ui.small_button("Rescan").clicked() {
                changed = true;
            }
        });
        if changed {
            self.settings.save();
            self.rescan_library();
        }
        ui.add_space(8.0);
        theme::section_title(ui, "LiveSync server (BimCraft protocol)");
        match &self.server {
            Some(s) => ui.label(RichText::new(format!("Listening on {}", s.addr)).size(11.5).color(theme::GOOD)),
            None => ui.label(RichText::new("Stopped").color(theme::TEXT_DIM)),
        };
        let mut restart = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Port").color(theme::TEXT_DIM));
            restart |= ui.add(egui::DragValue::new(&mut self.server_port).range(1024..=65535)).lost_focus();
        });
        ui.horizontal(|ui| {
            restart |= widgets::toggle(ui, &mut self.server_lan).changed();
            ui.label("Allow other computers");
        });
        if self.server_lan {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Pairing code").color(theme::TEXT_DIM));
                ui.label(RichText::new(&self.pair_token).monospace().strong());
                if ui.small_button("New").clicked() {
                    self.pair_token = crate::sync::new_token();
                    restart = true;
                }
            });
        }
        ui.horizontal(|ui| {
            widgets::toggle(ui, &mut self.follow_host_camera);
            ui.label("Follow the host application's camera");
        });
        if restart {
            self.start_server();
        }
    }

    /// Imported model + LiveSync: the panel under the LiveSync card.
    pub(super) fn livesync_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::section_title(ui, "Imported model");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::tool_button(ui, Icon::Close, vec2(22.0, 22.0), false, "Close").clicked() {
                    self.side = None;
                }
            });
        });
        ui.horizontal(|ui| {
            if widgets::flat_button(ui, Some(Icon::Import), "Import model...", true, 140.0)
                .on_hover_text("Revit: File > Export > FBX. Also glTF/GLB and OBJ.")
                .clicked()
            {
                self.open_model_dialog();
            }
            if widgets::flat_button(ui, Some(Icon::Folder), "Watch folder...", false, 140.0)
                .on_hover_text("Always show the newest model exported into a folder")
                .clicked()
            {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.start_file_sync(p);
                }
            }
        });
        match &self.file_sync {
            Some(fs) => {
                ui.label(RichText::new(format!("Live: {}", fs.target.display())).size(11.0).color(theme::GOOD));
                if ui.small_button("Reload now").clicked() {
                    fs.reload();
                }
            }
            None => {
                ui.label(RichText::new("No file linked. Export from Revit to FBX, then import it here.").size(11.0).color(theme::TEXT_DIM));
            }
        }
        if let Some(s) = &self.world.last_sync {
            ui.label(RichText::new(s).size(11.0).color(theme::TEXT_DIM));
        }
        if !self.world.missing_textures.is_empty() {
            ui.label(RichText::new(format!("{} texture(s) not found:", self.world.missing_textures.len())).color(theme::WARN));
            egui::ScrollArea::vertical().id_salt("missing").max_height(60.0).show(ui, |ui| {
                for m in &self.world.missing_textures {
                    ui.label(RichText::new(m).size(10.5).color(theme::TEXT_DIM));
                }
            });
        }
        ui.add_space(6.0);
        theme::section_title(ui, "Import settings");
        let mut s = self.world.import.clone().unwrap_or_else(|| self.import_settings.lock().unwrap().clone());
        let before = s.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Up axis").color(theme::TEXT_DIM));
            ui.selectable_value(&mut s.up, UpAxis::FromFile, "From file");
            ui.selectable_value(&mut s.up, UpAxis::Y, "Y");
            ui.selectable_value(&mut s.up, UpAxis::Z, "Z");
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Units").color(theme::TEXT_DIM));
            egui::ComboBox::from_id_salt("units").selected_text(s.units.label()).show_ui(ui, |ui| {
                for u in Units::ALL {
                    ui.selectable_value(&mut s.units, u, u.label());
                }
            });
        });
        ui.horizontal(|ui| {
            widgets::toggle(ui, &mut s.recenter);
            ui.label("Place at origin");
        });
        ui.label(RichText::new("Texture folders").color(theme::TEXT_DIM));
        let mut remove = None;
        for (i, d) in s.texture_dirs.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(d.display().to_string()).size(11.0));
                if ui.small_button("x").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            s.texture_dirs.remove(i);
        }
        if ui.small_button("+ Add texture folder...").on_hover_text("Searched (with sub-folders) for textures the model can't find").clicked() {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                s.texture_dirs.push(d);
            }
        }
        if s != before {
            if s.up != before.up || s.units != before.units {
                s.offset = None;
            }
            self.world.import = Some(s.clone());
            *self.import_settings.lock().unwrap() = s;
            if let Some(fs) = &self.file_sync {
                fs.reload();
            }
        }
    }

    /// Top-right: LiveSync status card, then either the imported-model card
    /// or (Delete tool) the selected-objects card.
    pub(super) fn top_right_cards(&mut self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();
        let w = 322.0;
        let x = screen.right() - w - 12.0;
        let mut y = TITLE_H + 40.0;
        let live = self.file_sync.is_some() || self.world.scene.meshes.keys().any(|k| k.starts_with("net:"));
        let name = self.model_name();
        egui::Area::new(egui::Id::new("livesync_card")).fixed_pos(pos2(x, y)).show(ctx, |ui| {
            theme::card_frame().inner_margin(egui::Margin::symmetric(14.0, 10.0)).show(ui, |ui| {
                ui.set_width(w - 28.0);
                ui.horizontal(|ui| {
                    let r = ui.label(RichText::new("LiveSync").size(12.5).strong()).rect;
                    let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, if live { theme::GOOD } else { Color32::from_rgb(110, 110, 116) });
                    let _ = r;
                    ui.add_space(8.0);
                    let label = name.clone().unwrap_or_else(|| "No model linked".into());
                    let resp = ui.add(egui::Label::new(RichText::new(label).size(12.0).color(theme::TEXT)).truncate().sense(Sense::click()));
                    if resp.on_hover_text("Imported model & LiveSync settings").clicked() {
                        self.toggle_side(SidePanel::LiveSync);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let tip = if self.file_sync.is_some() { "Pause LiveSync" } else { "Start LiveSync: import a model" };
                        if widgets::tool_button(ui, Icon::Power, vec2(22.0, 22.0), false, tip).clicked() {
                            if self.file_sync.is_some() {
                                self.stop_file_sync();
                            } else if let Some(src) = self.world.source.clone() {
                                self.start_file_sync(src);
                            } else {
                                self.open_model_dialog();
                            }
                        }
                    });
                });
            });
        });
        y += 58.0;
        if self.build_tab != BuildTab::Home || self.side == Some(SidePanel::LiveSync) {
            return;
        }
        if self.tool == Tool::Delete {
            let n = self.delete_marks.len();
            egui::Area::new(egui::Id::new("sel_card")).fixed_pos(pos2(x, y)).show(ctx, |ui| {
                theme::card_frame().inner_margin(egui::Margin::symmetric(14.0, 10.0)).show(ui, |ui| {
                    ui.set_width(w - 28.0);
                    ui.label(RichText::new("Selected objects").size(12.5).strong());
                    ui.horizontal(|ui| {
                        let r = widgets::tool_button_colored(
                            ui,
                            Icon::Delete,
                            vec2(58.0, 58.0),
                            false,
                            Some(theme::DANGER),
                            "Delete the selected objects (Delete)",
                        );
                        if r.clicked() {
                            let marks = std::mem::take(&mut self.delete_marks);
                            self.delete_objects(marks);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("{n}")).size(30.0).strong());
                        });
                    });
                });
            });
            return;
        }
        let Some(name) = name else { return };
        let thumb = self.model_thumb.as_ref().map(|(_, t)| t.clone());
        egui::Area::new(egui::Id::new("model_card")).fixed_pos(pos2(x, y)).show(ctx, |ui| {
            theme::card_frame().inner_margin(egui::Margin::symmetric(14.0, 10.0)).show(ui, |ui| {
                ui.set_width(w - 28.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                            icons::draw(ui.painter(), Icon::Home, r, theme::TEXT);
                            let stem = name.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or(name.clone());
                            ui.add(egui::Label::new(RichText::new(stem).size(12.5).strong()).truncate());
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if widgets::tool_button(ui, Icon::Folder, vec2(28.0, 28.0), false, "Import a different model").clicked() {
                                self.open_model_dialog();
                            }
                            if widgets::tool_button(ui, Icon::Sync, vec2(28.0, 28.0), false, "Reload the model now").clicked() {
                                if let Some(fs) = &self.file_sync {
                                    fs.reload();
                                } else if let Some(src) = self.world.source.clone() {
                                    self.start_file_sync(src);
                                }
                            }
                            if widgets::tool_button(ui, Icon::Gear, vec2(28.0, 28.0), false, "Import settings").clicked() {
                                self.toggle_side(SidePanel::LiveSync);
                            }
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(t) = &thumb {
                            let (r, resp) = ui.allocate_exact_size(vec2(112.0, 63.0), Sense::click());
                            ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            if resp.on_hover_text("Frame the model (F)").clicked() {
                                self.frame_all();
                            }
                        }
                    });
                });
            });
        });
    }

    /// Lumion's "Main" pill (views) and the round search button.
    pub(super) fn view_pill(&mut self, ctx: &egui::Context) {
        let x = SIDEBAR_W + 6.0;
        let y = TITLE_H + 6.0;
        egui::Area::new(egui::Id::new("view_pill")).fixed_pos(pos2(x, y)).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(56.0, 26.0), Sense::click());
            let fill = if resp.hovered() { Color32::from_rgb(245, 245, 247) } else { Color32::from_rgb(228, 228, 232) };
            ui.painter().rect_filled(r, 3.0, fill);
            icons::draw(ui.painter(), Icon::Eye, Rect::from_center_size(pos2(r.left() + 13.0, r.center().y), vec2(13.0, 13.0)), Color32::from_rgb(40, 40, 44));
            ui.painter().text(
                pos2(r.left() + 23.0, r.center().y),
                Align2::LEFT_CENTER,
                "Main",
                egui::FontId::proportional(11.5),
                Color32::from_rgb(30, 30, 34),
            );
            let popup = egui::Id::new("views_popup");
            if resp.on_hover_text("Camera views").clicked() {
                ui.memory_mut(|m| m.toggle_popup(popup));
            }
            egui::popup_below_widget(ui, popup, &ui.interact(r, egui::Id::new("vp_anchor"), Sense::hover()), egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(200.0);
                if ui.button("Frame the model   F").clicked() {
                    self.frame_all();
                }
                if ui.button("Save view to next photo slot").clicked() {
                    let n = self.world.views.len().min(PHOTO_SLOTS - 1);
                    self.save_slot(n);
                }
                if !self.world.views.is_empty() {
                    ui.separator();
                }
                let mut go = None;
                for (i, v) in self.world.views.iter().enumerate() {
                    if ui.button(format!("{}   Shift+{}", v.name, i + 1)).clicked() {
                        go = Some(i);
                    }
                }
                if let Some(i) = go {
                    self.world.camera = self.world.views[i].camera.clone();
                }
            });
        });
        // Search button (opens the object library with the search box focused).
        let show_search = self.build_tab == BuildTab::Home && self.obj_cat.is_none() && self.mat.is_none();
        if show_search && self.side.is_none() {
            egui::Area::new(egui::Id::new("search_btn")).fixed_pos(pos2(x + 12.0, y + 40.0)).show(ctx, |ui| {
                let (r, resp) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::click());
                ui.painter().circle_filled(r.center(), 26.0, if resp.hovered() { Color32::from_rgb(40, 40, 44) } else { Color32::from_rgb(20, 20, 22) });
                icons::draw(ui.painter(), Icon::Search, Rect::from_center_size(r.center(), vec2(18.0, 18.0)), Color32::WHITE);
                if resp.on_hover_text("Search the object library").clicked() {
                    self.obj_cat = Some(ObjCat::All);
                    self.obj_sub = 0;
                    self.focus_search = true;
                    self.tool = Tool::Place;
                }
            });
        }
    }

    /// Key-cap hints on the right edge; labels show while navigating.
    pub(super) fn shortcut_hints(&mut self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();
        let right = screen.right() - 14.0;
        let mut y = screen.bottom() - 196.0;
        let labels = self.last_nav.elapsed().as_secs_f32() < 2.5;
        if labels {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
        let mut rows: Vec<(&[&str], &str)> = vec![
            (&["SCROLL"], "Zoom in/out"),
            (&["MMB"], "Pan camera"),
            (&["RMB"], "Rotate camera"),
            (&["O"], "Orbit (hold with RMB)"),
            (&["QE"], "Down / up"),
            (&["WASD"], "Move"),
        ];
        if labels {
            rows = vec![
                (&["SCROLL"], "Zoom in/out"),
                (&["RMB"], "Rotate camera"),
                (&["SHIFT", "SPACE"], "Speed up 2x"),
                (&["SPACE"], "Slow speed"),
                (&["SHIFT"], "Speed up"),
            ];
        } else if self.build_tab == BuildTab::Home && !self.selection.is_empty() {
            rows.extend_from_slice(&[(&["L"], "Scale"), (&["R"], "Rotate"), (&["H"], "Height"), (&["M"], "Move"), (&["DEL"], "Delete")]);
        } else if self.build_tab == BuildTab::Home && self.tool == Tool::Select {
            rows.push((&["CTRL"], "Add to selection"));
        }
        let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("hints"));
        let p = ctx.layer_painter(layer);
        for (keys, label) in rows {
            let mut x = right;
            for k in keys.iter().rev() {
                let r = widgets::key_chip(&p, pos2(x, y), k);
                x = r.left() - 4.0;
            }
            if labels {
                widgets::chip_label(&p, pos2(x, y), &format!("Shortcut: {label}"));
            }
            y -= 23.0;
        }
    }
}

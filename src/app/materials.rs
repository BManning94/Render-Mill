//! Lumion-style material editor: a left panel with Library / Settings tabs,
//! value bars with map swatches, placement and weathering sections, and the
//! ✓ / ✕ buttons in the bottom-right to keep or discard the changes.

use super::*;
use crate::texlib::MapKind;
use crate::ui::icons::{self, Icon};
use crate::ui::widgets;
use egui::{pos2, Rect, Sense};

const BAR_W: f32 = 300.0;

impl LumenApp {
    pub(super) fn open_material(&mut self, name: &str) {
        if self.mat.as_ref().is_some_and(|m| m.name == name) {
            return;
        }
        if self.mat.is_some() {
            self.commit_material();
        }
        self.build_tab = BuildTab::Materials;
        self.mat = Some(MatSession {
            name: name.to_string(),
            before: self.world.scene.overrides.get(name).cloned(),
            tab: MatTab::Settings,
            section: MatSection::Placement,
            show_more: true,
            lib: MatLib::Materials,
            lib_cat: 0,
        });
    }

    /// ✓ Save changes.
    pub(super) fn commit_material(&mut self) {
        if let Some(m) = self.mat.take() {
            if self.world.scene.overrides.get(&m.name) != m.before.as_ref() {
                self.push_log(format!("Saved changes to {}", m.name));
            }
        }
    }

    /// ✕ Discard the changes made since the editor opened.
    fn cancel_material(&mut self) {
        if let Some(m) = self.mat.take() {
            if self.world.scene.overrides.get(&m.name) != m.before.as_ref() {
                self.snapshot();
                match m.before {
                    Some(p) => {
                        self.world.scene.overrides.insert(m.name, p);
                    }
                    None => {
                        self.world.scene.overrides.remove(&m.name);
                    }
                }
            }
        }
    }

    pub(super) fn material_ui(&mut self, ctx: &egui::Context) {
        let Some(name) = self.mat.as_ref().map(|m| m.name.clone()) else { return };
        let x = SIDEBAR_W + 6.0;
        let top = TITLE_H + 46.0;
        let mut preview: Option<(egui::TextureHandle, String)> = None;
        let panel = egui::Area::new(egui::Id::new("mateditor")).fixed_pos(pos2(x, top)).show(ctx, |ui| {
            theme::panel_frame().inner_margin(egui::Margin::same(0.0)).show(ui, |ui| {
                ui.set_width(372.0);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                // Tabs
                ui.horizontal(|ui| {
                    let tab = self.mat.as_ref().unwrap().tab;
                    for (t, icon, label) in [(MatTab::Library, Icon::Grid, "Library"), (MatTab::Settings, Icon::Pencil, "Settings")] {
                        let (r, resp) = ui.allocate_exact_size(vec2(100.0, 40.0), Sense::click());
                        let on = tab == t;
                        if on {
                            ui.painter().rect_filled(r, 0.0, Color32::from_rgb(40, 41, 45));
                        }
                        icons::draw(
                            ui.painter(),
                            icon,
                            Rect::from_center_size(pos2(r.left() + 28.0, r.center().y), vec2(14.0, 14.0)),
                            if on { Color32::WHITE } else { theme::TEXT_DIM },
                        );
                        ui.painter().text(
                            pos2(r.left() + 44.0, r.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(12.5),
                            if on { Color32::WHITE } else { theme::TEXT_DIM },
                        );
                        if resp.clicked() {
                            self.mat.as_mut().unwrap().tab = t;
                        }
                    }
                });
                egui::Frame::none().inner_margin(egui::Margin::symmetric(14.0, 10.0)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    self.material_header(ui, &name);
                    ui.separator();
                    match self.mat.as_ref().map(|m| m.tab) {
                        Some(MatTab::Settings) => self.material_settings(ui, &name, &mut preview),
                        Some(MatTab::Library) => self.material_library(ui, ctx, &name),
                        None => {}
                    }
                });
            });
        });
        if let Some((tex, title)) = preview {
            let at = pos2(panel.response.rect.right() + 6.0, top);
            widgets::preview_card(ctx, "mapcard", at, &title, &[], Some(&tex), vec2(360.0, 360.0));
        }
        // ✓ / ✕ in the bottom-right corner.
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("mat_save")).fixed_pos(pos2(screen.right() - 150.0, screen.bottom() - 110.0)).show(ctx, |ui| {
            theme::card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, resp) = ui.allocate_exact_size(vec2(66.0, 66.0), Sense::click());
                    ui.painter().rect_filled(r, 2.0, if resp.hovered() { Color32::from_rgb(52, 152, 240) } else { theme::ACCENT });
                    icons::draw(ui.painter(), Icon::Check, Rect::from_center_size(r.center(), vec2(30.0, 30.0)), Color32::WHITE);
                    if resp.on_hover_text("Save changes").clicked() {
                        self.commit_material();
                    }
                    ui.vertical(|ui| {
                        if widgets::tool_button(ui, Icon::Close, vec2(30.0, 30.0), false, "Discard changes").clicked() {
                            self.cancel_material();
                        }
                        if widgets::tool_button(
                            ui,
                            Icon::Help,
                            vec2(30.0, 30.0),
                            false,
                            "Click surfaces in the view to switch material. Drag files onto the window to add textures.",
                        )
                        .clicked()
                        {
                            self.side = Some(SidePanel::Help);
                        }
                    });
                });
            });
        });
    }

    fn material_header(&mut self, ui: &mut egui::Ui, name: &str) {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            let c = self.world.scene.effective(name).base_color;
            ui.painter().rect_filled(r, 1.0, egui::Rgba::from_rgb(c[0].sqrt(), c[1].sqrt(), c[2].sqrt()));
            ui.add_space(6.0);
            widgets::left_label(ui, vec2(214.0, 22.0), RichText::new(format!("Surface: {name}")).size(12.0)).on_hover_text(name);
            if widgets::tool_button(ui, Icon::Scan, vec2(26.0, 26.0), false, "Show every surface with this material").clicked() {
                self.frame_material(name);
            }
            if widgets::tool_button(ui, Icon::Copy, vec2(26.0, 26.0), false, "Copy material").clicked() {
                self.mat_clipboard = Some(self.world.scene.effective(name));
                self.push_log(format!("Copied {name}"));
            }
            if widgets::tool_button(ui, Icon::Paste, vec2(26.0, 26.0), false, "Paste material").clicked() {
                if let Some(p) = self.mat_clipboard.clone() {
                    self.snapshot();
                    self.world.scene.overrides.insert(name.to_string(), p);
                }
            }
            widgets::kebab(ui, "mat_more", "More", |ui| {
                if ui.button("Imported material (reset)").clicked() {
                    self.snapshot();
                    self.world.scene.overrides.remove(name);
                    ui.memory_mut(|m| m.close_popup());
                }
                let mut p = self.world.scene.effective(name);
                if ui.button(if p.invisible { "Make visible" } else { "Invisible" }).clicked() {
                    self.snapshot();
                    p.invisible = !p.invisible;
                    self.world.scene.overrides.insert(name.to_string(), p);
                    ui.memory_mut(|m| m.close_popup());
                }
            });
        });
    }

    /// One "map" row: value bar + swatch showing the map (click = load map).
    #[allow(clippy::too_many_arguments)]
    fn map_row(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        kind: MapKind,
        bar: impl FnOnce(&mut egui::Ui, &mut MaterialParams) -> egui::Response,
        p: &mut MaterialParams,
        st: &mut (bool, bool),
        preview: &mut Option<(egui::TextureHandle, String)>,
    ) {
        ui.horizontal(|ui| {
            track(st, bar(ui, p));
            ui.add_space(10.0);
            let imported = self.world.scene.imported_materials.get(name);
            let user_path = match kind {
                MapKind::Color => p.color_map.clone(),
                MapKind::Normal => p.normal_map.clone(),
                MapKind::Roughness => p.roughness_map.clone(),
            };
            let model_tex = imported.and_then(|m| match kind {
                MapKind::Color => m.texture.clone(),
                MapKind::Normal => m.normal_texture.clone(),
                MapKind::Roughness => m.roughness_texture.clone(),
            });
            let current = match &user_path {
                Some(path) => self.world.scene.user_textures.get(&crate::scene::user_texture_key(path, kind)).cloned(),
                None if kind == MapKind::Color => model_tex.clone().filter(|_| p.use_texture),
                None => model_tex.clone(),
            };
            let thumb = current.as_ref().map(|t| self.thumbs.texture_preview(ui.ctx(), t));
            let fallback = match kind {
                MapKind::Color => None,
                MapKind::Normal => Some(Color32::from_rgb(128, 128, 255)),
                MapKind::Roughness => Some(Color32::from_rgb(150, 150, 150)),
            };
            let title = match kind {
                MapKind::Color => "Color map",
                MapKind::Normal => "Normal map",
                MapKind::Roughness => "Roughness map",
            };
            let resp = widgets::swatch(ui, fallback.filter(|_| thumb.is_none()), thumb.as_ref(), 22.0, "");
            if resp.hovered() {
                if let Some(t) = &current {
                    let big = self.thumbs.texture_preview_sized(ui.ctx(), t, 512);
                    let src = user_path
                        .as_deref()
                        .map(|s| {
                            std::path::Path::new(s.split('#').next().unwrap_or(s)).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
                        })
                        .unwrap_or_else(|| "from the imported model".into());
                    *preview = Some((big, format!("{title} - {}x{}\n{src}", t.width, t.height)));
                }
            }
            let mut load = resp.on_hover_text(format!("{title}: click to load an image")).clicked();
            let mut remove = false;
            let mut use_model = false;
            widgets::kebab(ui, ("map_more", kind as u8), "Map options", |ui| {
                if ui.button("Load map...").clicked() {
                    load = true;
                    ui.memory_mut(|m| m.close_popup());
                }
                if current.is_some() && ui.button("Remove map").clicked() {
                    remove = true;
                    ui.memory_mut(|m| m.close_popup());
                }
                if model_tex.is_some() && user_path.is_some() && ui.button("Use the model's map").clicked() {
                    use_model = true;
                    ui.memory_mut(|m| m.close_popup());
                }
            });
            if load {
                if let Some(f) = rfd::FileDialog::new().add_filter("Image", IMAGE_EXTS).pick_file() {
                    let path = f.to_string_lossy().to_string();
                    if self.world.load_user_texture(&path, kind) {
                        *st = (true, true);
                        match kind {
                            MapKind::Color => {
                                p.color_map = Some(path);
                                p.use_texture = true;
                                p.tex_influence = 1.0;
                                p.base_color = [1.0, 1.0, 1.0, p.base_color[3]];
                                self.add_to_library(crate::texlib::group(&[f]));
                            }
                            MapKind::Normal => {
                                p.normal_map = Some(path);
                                p.normal_strength = p.normal_strength.max(1.0);
                            }
                            MapKind::Roughness => {
                                p.roughness_map = Some(path);
                                p.roughness = 0.5;
                            }
                        }
                    } else {
                        self.push_log(format!("Could not read image {}", f.display()));
                    }
                }
            }
            if remove {
                *st = (true, true);
                match kind {
                    MapKind::Color => {
                        p.color_map = None;
                        p.use_texture = false;
                    }
                    MapKind::Normal => {
                        p.normal_map = None;
                        p.normal_strength = 0.0;
                    }
                    MapKind::Roughness => p.roughness_map = None,
                }
            }
            if use_model {
                *st = (true, true);
                match kind {
                    MapKind::Color => {
                        p.color_map = None;
                        p.use_texture = true;
                    }
                    MapKind::Normal => p.normal_map = None,
                    MapKind::Roughness => p.roughness_map = None,
                }
            }
        });
    }

    fn material_settings(&mut self, ui: &mut egui::Ui, name: &str, preview: &mut Option<(egui::TextureHandle, String)>) {
        // Only write an override when the user actually changes something.
        let mut p = self.world.scene.effective(name);
        let mut st = (false, false); // (changed, gesture started)
        let show_more = self.mat.as_ref().is_none_or(|m| m.show_more);

        // Color
        ui.horizontal(|ui| {
            widgets::label_row(ui, "Color", BAR_W);
            ui.add_space(10.0);
            let mut rgb = [p.base_color[0], p.base_color[1], p.base_color[2]];
            let r = ui.color_edit_button_rgb(&mut rgb).on_hover_text("Colour (multiplies the texture)");
            if r.changed() {
                p.base_color[..3].copy_from_slice(&rgb);
            }
            track(&mut st, r);
        });
        self.map_row(
            ui,
            name,
            MapKind::Color,
            |ui, p| widgets::bar(ui, "Texture", &mut p.tex_influence, 0.0..=1.0, BAR_W, widgets::pct),
            &mut p,
            &mut st,
            preview,
        );
        ui.horizontal(|ui| {
            let mut size = 1.0 / p.uv_scale.max(0.001);
            let r = widgets::log_bar(ui, "Map scale - custom", &mut size, 0.05..=20.0, BAR_W, |v| format!("{v:.2} m"));
            if r.changed() {
                p.uv_scale = 1.0 / size;
            }
            track(&mut st, r.on_hover_text("Real-world size of one texture tile"));
        });
        if show_more {
            self.map_row(
                ui,
                name,
                MapKind::Normal,
                |ui, p| widgets::bar(ui, "Relief", &mut p.normal_strength, 0.0..=3.0, BAR_W, widgets::pct),
                &mut p,
                &mut st,
                preview,
            );
            self.map_row(
                ui,
                name,
                MapKind::Roughness,
                |ui, p| widgets::bar(ui, "Roughness", &mut p.roughness, 0.0..=1.0, BAR_W, widgets::pct),
                &mut p,
                &mut st,
                preview,
            );
            ui.horizontal(|ui| {
                track(&mut st, widgets::bar(ui, "Reflectivity", &mut p.reflectivity, 0.0..=1.0, BAR_W, widgets::pct));
            });
            ui.horizontal(|ui| {
                track(&mut st, widgets::bar(ui, "Metalness", &mut p.metallic, 0.0..=1.0, BAR_W, widgets::pct));
            });
        }
        ui.vertical_centered(|ui| {
            let label = if show_more { "Show less" } else { "Show more" };
            let (r, resp) = ui.allocate_exact_size(vec2(110.0, 24.0), Sense::click());
            ui.painter().text(
                r.center() - vec2(8.0, 0.0),
                Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(12.0),
                if resp.hovered() { Color32::WHITE } else { theme::TEXT },
            );
            icons::draw(
                ui.painter(),
                if show_more { Icon::ChevronUp } else { Icon::ChevronDown },
                Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(11.0, 11.0)),
                theme::TEXT,
            );
            if resp.clicked() {
                if let Some(m) = &mut self.mat {
                    m.show_more = !m.show_more;
                }
            }
        });
        // Section tabs.
        let section = self.mat.as_ref().map_or(MatSection::Placement, |m| m.section);
        ui.horizontal(|ui| {
            for (s, icon, tip) in [
                (MatSection::Advanced, Icon::Sliders, "Type, transparency & glow"),
                (MatSection::Placement, Icon::Move, "Texture placement"),
                (MatSection::Weathering, Icon::Leaf, "Weathering"),
                (MatSection::Imported, Icon::History, "Imported material"),
            ] {
                if widgets::tool_button(ui, icon, vec2(54.0, 30.0), section == s, tip).clicked() {
                    if let Some(m) = &mut self.mat {
                        m.section = s;
                    }
                }
            }
        });
        match section {
            MatSection::Placement => {
                ui.horizontal(|ui| {
                    widgets::left_label(ui, vec2(BAR_W - 40.0, 22.0), RichText::new("Real-world mapping").size(11.5)).on_hover_text(
                        "Place the texture by size in metres on every face, ignoring the model's own UVs.\nUse this when a texture looks stretched, smeared, tiny or as a flat colour (common with Revit exports).",
                    );
                    track(&mut st, widgets::toggle(ui, &mut p.box_mapping));
                });
                track(&mut st, widgets::bar(ui, "X offset", &mut p.uv_offset[0], -1.0..=1.0, BAR_W, |v| format!("{v:.2}")));
                track(&mut st, widgets::bar(ui, "Y offset", &mut p.uv_offset[1], -1.0..=1.0, BAR_W, |v| format!("{v:.2}")));
                track(&mut st, widgets::bar(ui, "Heading", &mut p.uv_rotation, 0.0..=360.0, BAR_W, |v| format!("{v:.0}°")));
                ui.horizontal(|ui| {
                    widgets::left_label(ui, vec2(BAR_W - 40.0, 22.0), RichText::new("Enable landscape tiling").size(11.5))
                        .on_hover_text("Break up visible repetition of the texture on large surfaces");
                    track(&mut st, widgets::toggle(ui, &mut p.anti_tiling));
                });
            }
            MatSection::Advanced => {
                ui.horizontal(|ui| {
                    widgets::label_row(ui, "Material type", BAR_W - 130.0);
                    let kind_before = p.kind;
                    egui::ComboBox::from_id_salt("mtype").width(120.0).selected_text(p.kind.label()).show_ui(ui, |ui| {
                        for k in MaterialKind::ALL {
                            ui.selectable_value(&mut p.kind, k, k.label());
                        }
                    });
                    if p.kind != kind_before {
                        st = (true, true);
                    }
                });
                track(&mut st, widgets::bar(ui, "Opacity", &mut p.base_color[3], 0.0..=1.0, BAR_W, widgets::pct));
                ui.horizontal(|ui| {
                    track(&mut st, widgets::log_bar(ui, "Glow", &mut p.emissive_strength, 0.01..=50.0, BAR_W, |v| format!("{v:.1}")));
                    ui.add_space(10.0);
                    track(&mut st, ui.color_edit_button_rgb(&mut p.emissive));
                });
                ui.horizontal(|ui| {
                    widgets::left_label(ui, vec2(BAR_W - 40.0, 22.0), RichText::new("Cut out transparent parts of the colour map").size(11.5));
                    track(&mut st, widgets::toggle(ui, &mut p.cutout));
                });
            }
            MatSection::Weathering => {
                track(&mut st, widgets::bar(ui, "Weathering", &mut p.variation, 0.0..=1.0, BAR_W, widgets::pct));
                ui.label(
                    RichText::new("Adds dirt, wear and colour variation so flat colours don't look computer-generated.").size(11.0).color(theme::TEXT_DIM),
                );
            }
            MatSection::Imported => {
                let edited = self.world.scene.overrides.contains_key(name);
                ui.label(RichText::new(if edited { "This surface uses an edited material." } else { "This surface uses the imported material." }).size(11.5));
                ui.horizontal(|ui| {
                    if widgets::flat_button(ui, Some(Icon::History), "Imported material", false, 160.0).on_hover_text("Throw away all edits").clicked()
                        && edited
                    {
                        self.snapshot();
                        self.world.scene.overrides.remove(name);
                        return;
                    }
                    let mut inv = p.invisible;
                    if widgets::flat_button(ui, Some(if inv { Icon::Eye } else { Icon::EyeOff }), if inv { "Visible" } else { "Invisible" }, false, 130.0)
                        .clicked()
                    {
                        inv = !inv;
                        p.invisible = inv;
                        st = (true, true);
                    }
                });
            }
        }
        if st.0 {
            if st.1 {
                self.snapshot(); // one undo step per gesture
            }
            if p.emissive_strength > 0.011 && p.emissive == [0.0; 3] {
                p.emissive = [1.0, 0.85, 0.6];
            }
            self.world.scene.overrides.insert(name.to_string(), p);
        }
    }

    fn material_library(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, name: &str) {
        let (lib, lib_cat) = self.mat.as_ref().map(|m| (m.lib, m.lib_cat)).unwrap_or((MatLib::Materials, 0));
        ui.horizontal(|ui| {
            let new = ui.menu_button(RichText::new("+ New").size(12.0), |ui| {
                for k in MaterialKind::ALL {
                    if ui.button(k.label()).clicked() {
                        self.snapshot();
                        let old = self.world.scene.effective(name);
                        let mut p = MaterialParams { kind: k, base_color: old.base_color, uv_transform: old.uv_transform, ..Default::default() };
                        if k.is_transparent() {
                            p.base_color[3] = 0.4;
                            p.roughness = 0.05;
                        }
                        if k == MaterialKind::Emissive {
                            p.emissive = [1.0, 0.85, 0.6];
                            p.emissive_strength = 8.0;
                        }
                        self.world.scene.overrides.insert(name.to_string(), p);
                        if let Some(m) = &mut self.mat {
                            m.tab = MatTab::Settings;
                        }
                        ui.close_menu();
                    }
                }
            });
            new.response.on_hover_text("Start from a blank material of a type");
            for (l, label) in [(MatLib::Materials, "Materials"), (MatLib::Folders, "My folders"), (MatLib::Custom, "Custom")] {
                if ui.selectable_label(lib == l, label).clicked() {
                    if let Some(m) = &mut self.mat {
                        m.lib = l;
                        m.lib_cat = 0;
                    }
                }
            }
        });
        if lib != MatLib::Custom {
            ui.add(egui::TextEdit::singleline(&mut self.lib_filter).hint_text("Search materials").desired_width(f32::INFINITY));
        }
        ui.add_space(4.0);
        let mut hovered: Option<(egui::TextureHandle, String)> = None;
        match lib {
            MatLib::Materials | MatLib::Folders => {
                let builtin = lib == MatLib::Materials;
                let want = |m: &crate::library::LibMaterial| (m.source == crate::library::Source::Builtin) == builtin;
                let mut cats: Vec<String> = vec![if builtin { "Basic".to_string() } else { "All".to_string() }];
                for m in self.library.iter().filter(|m| want(m)) {
                    if !cats.contains(&m.category) {
                        cats.push(m.category.clone());
                    }
                }
                if self.library_rx.is_some() {
                    ui.label(RichText::new("Indexing material libraries...").size(11.5).color(theme::TEXT_DIM));
                } else if !builtin && cats.len() == 1 {
                    ui.label(
                        RichText::new("No library folders yet. Add one in Settings (gear, bottom left) to browse your own textures here.")
                            .size(11.5)
                            .color(theme::TEXT_DIM),
                    );
                }
                let cat = lib_cat.min(cats.len() - 1);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for (k, c) in cats.iter().enumerate() {
                        if ui.selectable_label(cat == k, RichText::new(c.as_str()).size(11.5)).clicked() {
                            if let Some(m) = &mut self.mat {
                                m.lib_cat = k;
                            }
                        }
                    }
                });
                let filter = self.lib_filter.to_lowercase();
                let cat_name = cats[cat].clone();
                ui.label(RichText::new(format!("{}  ›  {}", if builtin { "Materials" } else { "My folders" }, cat_name)).size(11.5).color(theme::TEXT_DIM));
                let basic = builtin && cat == 0 && filter.is_empty();
                egui::ScrollArea::vertical().id_salt(("libgrid", builtin)).max_height(430.0).min_scrolled_height(400.0).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.set_max_width(344.0);
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        if basic {
                            let all = presets();
                            for (n, p) in &all {
                                let th = self.thumbs.get(&format!("mat:{n}")).cloned();
                                let resp = widgets::tile(ui, th.as_ref(), Icon::Materials, n, false, 76.0);
                                if resp.hovered() {
                                    if let Some(t) = th.clone() {
                                        hovered = Some((t, n.to_string()));
                                    }
                                }
                                if resp.clicked() {
                                    self.snapshot();
                                    let old = self.world.scene.effective(name);
                                    let mut np = p.clone();
                                    np.uv_transform = old.uv_transform;
                                    np.use_texture = false;
                                    self.world.scene.overrides.insert(name.to_string(), np);
                                    if let Some(m) = &mut self.mat {
                                        m.tab = MatTab::Settings;
                                    }
                                }
                            }
                            return;
                        }
                        let items: Vec<usize> = (0..self.library.len())
                            .filter(|&i| {
                                let m = &self.library[i];
                                want(m)
                                    && (if filter.is_empty() {
                                        (!builtin && cat == 0) || m.category == cat_name
                                    } else {
                                        m.name.to_lowercase().contains(&filter) || m.tags.iter().any(|t| t.contains(&filter))
                                    })
                            })
                            .collect();
                        let mut apply = None;
                        for i in items {
                            let m = &self.library[i];
                            let key = Self::set_key(&m.set);
                            let th = self.thumbs.get(&key).cloned();
                            let resp = widgets::tile(ui, th.as_ref(), Icon::Materials, &m.name, false, 76.0);
                            if th.is_none() && ui.is_rect_visible(resp.rect) {
                                self.thumbs.request(&key, Job::TextureSet(m.set.clone()));
                            }
                            if resp.hovered() {
                                if let Some(t) = th.clone() {
                                    hovered = Some((t, format!("{}\n{} · {:.1} m tile", m.name, m.category, m.set.size_m)));
                                }
                            }
                            if resp.clicked() {
                                apply = Some(i);
                            }
                        }
                        if let Some(i) = apply {
                            let lm = self.library[i].clone();
                            self.apply_texture_set(name, &lm.set);
                            if lm.metallic {
                                let mut p = self.world.scene.effective(name);
                                p.metallic = 1.0;
                                self.world.scene.overrides.insert(name.to_string(), p);
                            }
                            if let Some(m) = &mut self.mat {
                                m.tab = MatTab::Settings;
                            }
                        }
                    });
                });
            }
            MatLib::Custom => {
                ui.horizontal(|ui| {
                    if widgets::flat_button(ui, Some(Icon::Plus), "Images...", false, 110.0)
                        .on_hover_text("Colour/normal/roughness maps with matching names become one material")
                        .clicked()
                    {
                        self.import_textures_dialog();
                    }
                    if widgets::flat_button(ui, Some(Icon::Folder), "Folder...", false, 110.0)
                        .on_hover_text("Import every texture in a folder (Poly Haven, ambientCG...)")
                        .clicked()
                    {
                        self.import_texture_folder_dialog();
                    }
                });
                if self.world.texture_library.is_empty() {
                    ui.label(RichText::new("Import your own textures, or drag image files and folders onto the window.").size(11.5).color(theme::TEXT_DIM));
                }
                let mut apply = None;
                let mut remove = None;
                let mut resize: Option<(usize, f32)> = None;
                egui::ScrollArea::vertical().id_salt("customlib").max_height(360.0).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for (i, set) in self.world.texture_library.iter().enumerate() {
                            let key = Self::set_key(set);
                            let th = self.thumbs.get(&key).cloned();
                            if th.is_none() {
                                self.thumbs.request(&key, Job::TextureSet(set.clone()));
                            }
                            let resp = widgets::tile(ui, th.as_ref(), Icon::Materials, &set.name, false, 76.0);
                            if resp.hovered() {
                                if let Some(t) = th.clone() {
                                    let maps = [(set.color.is_some(), "colour"), (set.normal.is_some(), "normal"), (set.roughness.is_some(), "roughness")];
                                    let list: Vec<&str> = maps.iter().filter(|m| m.0).map(|m| m.1).collect();
                                    hovered = Some((t, format!("{}\n{} · {:.2} m tile", set.name, list.join(" + "), set.size_m)));
                                }
                            }
                            if resp.clicked() {
                                apply = Some(i);
                            }
                            resp.context_menu(|ui| {
                                let mut size = set.size_m;
                                ui.horizontal(|ui| {
                                    ui.label("Tile size");
                                    if ui.add(egui::DragValue::new(&mut size).range(0.05..=50.0).speed(0.05).suffix(" m")).changed() {
                                        resize = Some((i, size));
                                    }
                                });
                                if ui.button("Remove from library").clicked() {
                                    remove = Some(i);
                                    ui.close_menu();
                                }
                            });
                        }
                    });
                });
                if let Some(i) = apply {
                    let set = self.world.texture_library[i].clone();
                    self.apply_texture_set(name, &set);
                    if let Some(m) = &mut self.mat {
                        m.tab = MatTab::Settings;
                    }
                }
                if let Some((i, size)) = resize {
                    self.world.texture_library[i].size_m = size;
                }
                if let Some(i) = remove {
                    self.world.texture_library.remove(i);
                }
            }
        }
        if let Some((t, title)) = hovered {
            let at = pos2(SIDEBAR_W + 6.0 + 372.0 + 6.0, TITLE_H + 46.0);
            widgets::preview_card(ctx, "matcard", at, &title, &[], Some(&t), vec2(220.0, 220.0));
        }
    }
}

fn track(st: &mut (bool, bool), r: egui::Response) {
    st.0 |= r.changed();
    st.1 |= r.drag_started() || (r.changed() && !r.dragged());
}

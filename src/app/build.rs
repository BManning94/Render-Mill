//! Build mode: the compact dock in the bottom-left (tabs, category grid,
//! tools), the object library panel with hover previews, object properties,
//! and the landscape / weather tabs.

use super::*;
use crate::ui::icons::{self, Icon};
use crate::ui::widgets;
use egui::{pos2, Rect, Sense, Stroke};

const DOCK_BODY_H: f32 = 128.0;
const DOCK_TAB_H: f32 = 42.0;

impl ObjCat {
    const GRID: [ObjCat; 8] =
        [ObjCat::Nature, ObjCat::People, ObjCat::Transport, ObjCat::Interior, ObjCat::Exterior, ObjCat::Lights, ObjCat::Favourites, ObjCat::All];
    fn info(self) -> (Icon, &'static str) {
        match self {
            ObjCat::Nature => (Icon::Tree, "Nature"),
            ObjCat::People => (Icon::Person, "People and Animals"),
            ObjCat::Transport => (Icon::Car, "Transport"),
            ObjCat::Interior => (Icon::Chair, "Interior"),
            ObjCat::Exterior => (Icon::Bench, "Exterior"),
            ObjCat::Lights => (Icon::Light, "Lights"),
            ObjCat::Favourites => (Icon::Star, "Favourites"),
            ObjCat::All => (Icon::Grid, "All objects"),
        }
    }
}

impl LumenApp {
    /// Sub-categories of a library category: (icon, name, items).
    fn lib_subs(&self, cat: ObjCat) -> Vec<(Icon, String, Vec<LibItem>)> {
        use ObjectKind::*;
        let kinds = |ks: &[ObjectKind]| ks.iter().map(|k| LibItem::Kind(*k)).collect::<Vec<_>>();
        let models = |cats: &[&str], filter: &dyn Fn(&str) -> bool| {
            self.models.iter().filter(|m| cats.contains(&m.category.as_str()) && filter(&m.key)).map(|m| LibItem::Model(m.key.clone())).collect::<Vec<_>>()
        };
        let any = |_: &str| true;
        let outdoor = |k: &str| {
            k.contains("street_lamp")
                || k.contains("picnic")
                || k.contains("outdoor")
                || k.contains("lantern")
                || k.contains("fire_pit")
                || k.contains("aircon")
        };
        let join = |mut a: Vec<LibItem>, b: Vec<LibItem>| {
            a.extend(b);
            a
        };
        match cat {
            ObjCat::Nature => vec![
                (Icon::Tree, "Trees".into(), join(kinds(&[Tree, Conifer]), models(&["Trees"], &any))),
                (Icon::Leaf, "Plants".into(), join(kinds(&[Shrub, Planter]), models(&["Plants"], &any))),
                (Icon::Landscape, "Rocks".into(), join(kinds(&[Boulder]), models(&["Rocks"], &any))),
            ],
            ObjCat::People => vec![(Icon::Person, "People".into(), kinds(&[Person]))],
            ObjCat::Transport => vec![(Icon::Car, "Cars".into(), kinds(&[Car]))],
            ObjCat::Interior => vec![
                (Icon::Chair, "Seating".into(), join(kinds(&[ArmChair, Sofa, Chair]), models(&["Seating"], &any))),
                (Icon::Outdoor, "Tables".into(), join(kinds(&[Table]), models(&["Tables"], &|k: &str| !outdoor(k)))),
                (Icon::Grid, "Storage & beds".into(), models(&["Storage & beds"], &any)),
                (Icon::Light, "Lamps".into(), models(&["Lighting"], &|k: &str| !outdoor(k))),
                (Icon::Effects, "Decor".into(), models(&["Decor"], &|k: &str| !outdoor(k))),
            ],
            ObjCat::Exterior => vec![
                (Icon::Bench, "Furniture".into(), join(kinds(&[Bench, Planter]), models(&["Tables", "Decor"], &outdoor))),
                (Icon::Light, "Lighting".into(), join(kinds(&[StreetLamp]), models(&["Lighting"], &outdoor))),
                (Icon::Landscape, "Rocks".into(), join(kinds(&[Boulder]), models(&["Rocks"], &any))),
            ],
            ObjCat::Lights => vec![(Icon::Light, "Lights".into(), kinds(&[PointLight, SpotLight, StreetLamp]))],
            ObjCat::Favourites => {
                let mut all = kinds(&ObjectKind::ALL);
                all.extend(self.models.iter().map(|m| LibItem::Model(m.key.clone())));
                all.retain(|i| self.favourites.contains(&i.fav_key()));
                vec![(Icon::Star, "Favourites".into(), all)]
            }
            ObjCat::All => {
                let mut all = kinds(&ObjectKind::ALL);
                all.extend(self.models.iter().filter(|m| m.category != "Imported").map(|m| LibItem::Model(m.key.clone())));
                vec![(Icon::Grid, "All".into(), all), (Icon::Import, "Imported".into(), models(&["Imported"], &any))]
            }
        }
    }

    fn item_thumb_key(item: &LibItem) -> String {
        match item {
            LibItem::Kind(k) => format!("obj:{k:?}"),
            LibItem::Model(m) => format!("model:{m}"),
        }
    }

    fn item_name(&self, item: &LibItem) -> String {
        match item {
            LibItem::Kind(k) => k.label().to_string(),
            LibItem::Model(m) => self.model_entry(m).map(|e| e.name.clone()).unwrap_or_else(|| m.clone()),
        }
    }

    fn item_tags(&self, item: &LibItem) -> Vec<String> {
        match item {
            LibItem::Kind(k) => k.tags().iter().map(|s| s.to_string()).collect(),
            LibItem::Model(m) => {
                let mut t: Vec<String> = self.model_entry(m).map(|e| e.tags.iter().take(4).cloned().collect()).unwrap_or_default();
                if let Some(e) = self.model_entry(m) {
                    if e.key.starts_with("builtin:") {
                        t.push("CC0 scan".into());
                    }
                    if e.triangles > 0 {
                        t.push(format!("{}k tris", (e.triangles + 500) / 1000));
                    }
                }
                t
            }
        }
    }

    pub(super) fn build_overlays(&mut self, ctx: &egui::Context) {
        self.view_pill(ctx);
        self.top_right_cards(ctx);
        self.shortcut_hints(ctx);
        let dock = self.dock(ctx);
        match self.build_tab {
            BuildTab::Home => {
                if self.obj_cat.is_some() {
                    self.object_library(ctx, dock);
                }
                self.tool_options(ctx, dock);
            }
            BuildTab::Materials => self.material_ui(ctx),
            _ => {}
        }
    }

    /// The bottom-left dock. Returns its screen rect.
    fn dock(&mut self, ctx: &egui::Context) -> Rect {
        let screen = ctx.screen_rect();
        let x = SIDEBAR_W + 14.0;
        let bottom = screen.bottom() - 14.0;
        let top = bottom - DOCK_BODY_H - DOCK_TAB_H;
        let mut dock_rect = Rect::from_min_max(pos2(x, top), pos2(x, bottom));
        egui::Area::new(egui::Id::new("dock")).fixed_pos(pos2(x, top)).show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            // Tab strip (with Lumion's notched tab shape).
            let (tabs_rect, _) = ui.allocate_exact_size(vec2(236.0, DOCK_TAB_H), Sense::hover());
            let p = ui.painter();
            let notch = vec![
                tabs_rect.left_bottom(),
                tabs_rect.left_top() + vec2(0.0, 6.0),
                tabs_rect.left_top() + vec2(6.0, 0.0),
                tabs_rect.right_top() - vec2(30.0, 0.0),
                tabs_rect.right_bottom(),
            ];
            p.add(egui::Shape::convex_polygon(notch, theme::PANEL, Stroke::NONE));
            for (k, (tab, icon, tip)) in [
                (BuildTab::Home, Icon::Home, "Objects & imported model"),
                (BuildTab::Materials, Icon::Brush, "Materials"),
                (BuildTab::Landscape, Icon::Landscape, "Landscape"),
                (BuildTab::Weather, Icon::WeatherCloud, "Weather"),
            ]
            .into_iter()
            .enumerate()
            {
                let r = Rect::from_min_size(tabs_rect.min + vec2(10.0 + k as f32 * 52.0, 4.0), vec2(46.0, DOCK_TAB_H - 6.0));
                let resp = ui.interact(r, egui::Id::new(("docktab", k)), Sense::click());
                let on = self.build_tab == tab;
                if resp.hovered() && !on {
                    ui.painter().rect_filled(r.shrink(2.0), 3.0, Color32::from_rgb(48, 49, 54));
                }
                icons::draw(
                    ui.painter(),
                    icon,
                    Rect::from_center_size(r.center() - vec2(0.0, 2.0), vec2(19.0, 19.0)),
                    if on { Color32::WHITE } else { theme::TEXT_DIM },
                );
                if on {
                    ui.painter()
                        .line_segment([pos2(r.left() + 6.0, r.bottom() - 2.0), pos2(r.right() - 6.0, r.bottom() - 2.0)], Stroke::new(2.0_f32, Color32::WHITE));
                }
                if resp.on_hover_text(tip).clicked() {
                    self.set_build_tab(tab);
                }
            }
            // Body
            let body = ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                match self.build_tab {
                    BuildTab::Home => self.dock_home(ui),
                    BuildTab::Materials => self.dock_materials(ui),
                    BuildTab::Landscape => self.dock_landscape(ui),
                    BuildTab::Weather => self.dock_weather(ui),
                }
            });
            dock_rect = tabs_rect.union(body.response.rect);
        });
        dock_rect
    }

    fn set_build_tab(&mut self, tab: BuildTab) {
        if self.build_tab == tab {
            return;
        }
        if tab != BuildTab::Materials && self.mat.is_some() {
            self.commit_material();
        }
        if tab != BuildTab::Home {
            self.obj_cat = None;
            self.drag = None;
        }
        self.build_tab = tab;
    }

    /// A dark block in the dock body.
    fn dock_block(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
        egui::Frame::none().fill(theme::PANEL).rounding(egui::Rounding { nw: 0.0, ne: 4.0, sw: 4.0, se: 4.0 }).inner_margin(egui::Margin::same(10.0)).show(
            ui,
            |ui| {
                ui.set_width(width);
                ui.set_height(DOCK_BODY_H - 20.0);
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                ui.vertical(|ui| add(ui));
            },
        );
    }

    fn dock_home(&mut self, ui: &mut egui::Ui) {
        // Block 1: imported model + category grid + select-all.
        Self::dock_block(ui, 322.0, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                let open = self.side == Some(SidePanel::LiveSync);
                if widgets::tool_button(ui, Icon::Build, vec2(48.0, 104.0), open, "Imported model: LiveSync & import settings").clicked() {
                    self.toggle_side(SidePanel::LiveSync);
                }
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for row in ObjCat::GRID.chunks(4) {
                        ui.horizontal(|ui| {
                            for &cat in row {
                                let (icon, name) = cat.info();
                                let on = self.obj_cat == Some(cat);
                                if widgets::tool_button(ui, icon, vec2(50.0, 50.0), on, name).clicked() {
                                    if on {
                                        self.obj_cat = None;
                                    } else {
                                        self.obj_cat = Some(cat);
                                        self.obj_sub = 0;
                                        self.obj_search.clear();
                                        self.tool = Tool::Place;
                                    }
                                }
                            }
                        });
                    }
                });
                ui.add_space(4.0);
                if widgets::tool_button(ui, Icon::SelectBox, vec2(42.0, 104.0), false, "Select all objects (Ctrl+A)").clicked() {
                    self.tool = Tool::Select;
                    self.selection = (0..self.world.objects.len()).collect();
                }
            });
        });
        // Block 2: tools.
        Self::dock_block(ui, 154.0, |ui| {
            ui.horizontal(|ui| {
                for (t, icon, tip) in [
                    (Tool::Place, Icon::PlaceDown, "Place object"),
                    (Tool::Select, Icon::Select, "Select and edit objects"),
                    (Tool::Delete, Icon::Delete, "Delete objects"),
                ] {
                    let fill = (t == Tool::Delete && self.tool == Tool::Delete).then_some(theme::DANGER);
                    if widgets::tool_button_colored(ui, icon, vec2(48.0, 48.0), self.tool == t, fill, tip).clicked() {
                        self.tool = t;
                        self.drag = None;
                        if t != Tool::Delete {
                            self.delete_marks.clear();
                        }
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| match self.tool {
                Tool::Place => {
                    for (m, icon, tip) in [
                        (PlaceMode::Single, Icon::Dot, "Place one object per click"),
                        (PlaceMode::Paint, Icon::Pencil, "Paint: drag to place objects along a path"),
                        (PlaceMode::Mass, Icon::Circle, "Mass place: scatter many objects in a circle"),
                    ] {
                        if widgets::tool_button(ui, icon, vec2(36.0, 36.0), self.place_mode == m, tip).clicked() {
                            self.place_mode = m;
                        }
                    }
                }
                Tool::Select => {
                    for (m, icon, tip) in [
                        (EditMode::Move, Icon::Move, "Move (M)"),
                        (EditMode::Height, Icon::Height, "Height (H)"),
                        (EditMode::Rotate, Icon::Rotate, "Rotate (R)"),
                        (EditMode::Scale, Icon::Scale, "Scale (L)"),
                    ] {
                        if widgets::tool_button(ui, icon, vec2(34.0, 36.0), self.edit_mode == m, tip).clicked() {
                            self.edit_mode = m;
                        }
                    }
                }
                Tool::Delete => {
                    for (m, icon, tip) in [
                        (DeleteMode::Single, Icon::Dot, "Click objects to mark them"),
                        (DeleteMode::Area, Icon::Circle, "Drag over an area to mark everything in it"),
                    ] {
                        if widgets::tool_button(ui, icon, vec2(36.0, 36.0), self.delete_mode == m, tip).clicked() {
                            self.delete_mode = m;
                        }
                    }
                }
            });
        });
        // Block 3 (Place): randomise.
        if self.tool == Tool::Place {
            Self::dock_block(ui, 48.0, |ui| {
                if widgets::tool_button(ui, Icon::Random, vec2(48.0, 48.0), self.place_random, "Random rotation and size").clicked() {
                    self.place_random = !self.place_random;
                }
            });
        }
        // Block 4: undo / deselect.
        Self::dock_block(ui, 48.0, |ui| {
            let can_undo = !self.undo.is_empty();
            if widgets::tool_button(ui, Icon::Undo, vec2(48.0, 48.0), false, "Undo (Ctrl+Z)").clicked() && can_undo {
                self.undo();
            }
            ui.add_space(6.0);
            if widgets::tool_button(ui, Icon::Deselect, vec2(48.0, 48.0), false, "Deselect (Esc)").clicked() {
                self.selection.clear();
                self.delete_marks.clear();
            }
        });
    }

    fn dock_materials(&mut self, ui: &mut egui::Ui) {
        Self::dock_block(ui, 560.0, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(180.0);
                    ui.label(RichText::new("Material editor").strong());
                    ui.label(RichText::new("Click on an imported model to modify its material.").size(11.5).color(theme::TEXT_DIM));
                    ui.add_space(4.0);
                    if widgets::flat_button(ui, Some(Icon::Plus), "Import textures...", false, 170.0).clicked() {
                        self.import_textures_dialog();
                    }
                });
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.set_width(350.0);
                    ui.add(egui::TextEdit::singleline(&mut self.mat_filter).hint_text("Search model materials").desired_width(350.0));
                    let filter = self.mat_filter.to_lowercase();
                    let mut used: Vec<String> =
                        self.world.scene.meshes.iter().filter(|(k, _)| !k.starts_with("env:")).map(|(_, m)| m.data.material.clone()).collect();
                    used.sort();
                    used.dedup();
                    let mut open = None;
                    egui::ScrollArea::vertical().id_salt("matchips").max_height(72.0).max_width(350.0).auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        for name in used.iter().filter(|n| filter.is_empty() || n.to_lowercase().contains(&filter)) {
                            let p = self.world.scene.effective(name);
                            let edited = self.world.scene.overrides.contains_key(name);
                            let on = self.mat.as_ref().is_some_and(|m| &m.name == name);
                            let (r, resp) = ui.allocate_exact_size(vec2(340.0, 20.0), Sense::click());
                            let bg = if on {
                                theme::SELECTED
                            } else if resp.hovered() {
                                Color32::from_rgb(52, 53, 58)
                            } else {
                                Color32::from_rgb(40, 41, 45)
                            };
                            ui.painter().rect_filled(r, 2.0, bg);
                            let c = p.base_color;
                            ui.painter().rect_filled(
                                Rect::from_center_size(pos2(r.left() + 11.0, r.center().y), vec2(10.0, 10.0)),
                                1.0,
                                egui::Rgba::from_rgb(c[0].sqrt(), c[1].sqrt(), c[2].sqrt()),
                            );
                            ui.painter().text(
                                pos2(r.left() + 22.0, r.center().y),
                                Align2::LEFT_CENTER,
                                name,
                                egui::FontId::proportional(11.0),
                                if edited { theme::ACCENT } else { theme::TEXT },
                            );
                            if resp.clicked() {
                                open = Some(name.clone());
                            }
                        }
                    });
                    if let Some(n) = open {
                        self.open_material(&n);
                    }
                });
            });
        });
    }

    fn dock_landscape(&mut self, ui: &mut egui::Ui) {
        Self::dock_block(ui, 250.0, |ui| {
            ui.label(RichText::new("Landscape").strong());
            ui.horizontal(|ui| {
                widgets::toggle(ui, &mut self.world.env.ground_enabled);
                ui.label("Show ground");
            });
            ui.horizontal(|ui| {
                widgets::toggle(ui, &mut self.world.env.backdrop);
                ui.label("Mountains");
                ui.add_space(10.0);
                widgets::toggle(ui, &mut self.world.env.grass);
                ui.label("3D grass");
            });
            widgets::bar(ui, "Grass height", &mut self.world.env.grass_height, 0.04..=0.5, 230.0, |v| format!("{:.0} cm", v * 100.0));
            let mut p = self.world.scene.effective(crate::objects::MAT_GROUND);
            if widgets::bar(ui, "Variation", &mut p.variation, 0.0..=1.0, 230.0, widgets::pct).changed() {
                self.world.scene.overrides.insert(crate::objects::MAT_GROUND.into(), p);
            }
        });
        Self::dock_block(ui, 620.0, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Ground type").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Edit ground material").clicked() {
                        self.build_tab = BuildTab::Materials;
                        self.open_material(crate::objects::MAT_GROUND);
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                let current = self.world.scene.effective(crate::objects::MAT_GROUND);
                // Photo-textured landscapes from the built-in library.
                for (name, id, size) in [
                    ("Lawn", "Grass001", 2.5),
                    ("Meadow", "Grass004", 2.5),
                    ("Dry grass", "Ground003", 2.5),
                    ("Forest floor", "Ground037", 2.5),
                    ("Gravel", "Gravel022", 1.5),
                    ("Sand", "Ground033", 2.5),
                ] {
                    let Some(mut set) = World::builtin_set(id) else { continue };
                    set.size_m = size;
                    let key = Self::set_key(&set);
                    let th = self.thumbs.get(&key).cloned();
                    if th.is_none() {
                        self.thumbs.request(&key, Job::TextureSet(set.clone()));
                    }
                    let sel = current.color_map == set.color;
                    if widgets::tile(ui, th.as_ref(), Icon::Landscape, name, sel, 70.0).clicked() {
                        self.snapshot();
                        self.world.set_ground_from_library(id, size);
                    }
                }
                for (name, p) in Self::ground_types().into_iter().filter(|(n, _)| matches!(*n, "Concrete" | "Lake water")) {
                    let th = self.thumbs.get(&format!("ground:{name}")).cloned();
                    let sel = current.base_color == p.base_color && current.kind == p.kind;
                    if widgets::tile(ui, th.as_ref(), Icon::Landscape, name, sel, 70.0).clicked() {
                        self.snapshot();
                        self.world.scene.overrides.insert(crate::objects::MAT_GROUND.into(), p);
                    }
                }
            });
        });
    }

    fn dock_weather(&mut self, ui: &mut egui::Ui) {
        Self::dock_block(ui, 380.0, |ui| {
            ui.horizontal_top(|ui| {
                let e = &mut self.world.env;
                widgets::sun_dial(ui, &mut e.sun_heading, &mut e.sun_height, 104.0);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
                    let e = &mut self.world.env;
                    widgets::bar(ui, "Sun height", &mut e.sun_height, -30.0..=90.0, 250.0, |v| format!("{v:.0}°"));
                    widgets::bar(ui, "Sun heading", &mut e.sun_heading, 0.0..=360.0, 250.0, |v| format!("{v:.0}°"));
                    widgets::bar(ui, "Sun brightness", &mut e.sun_intensity, 0.0..=8.0, 250.0, |v| format!("{v:.1}"));
                    widgets::bar(ui, "Cloud amount", &mut e.cloudiness, 0.0..=1.0, 250.0, widgets::pct);
                });
            });
        });
        Self::dock_block(ui, 470.0, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Weather presets").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Animate").size(11.5).color(theme::TEXT_DIM));
                    widgets::toggle(ui, &mut self.world.env.animate);
                });
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                for (name, preset) in crate::env::Environment::presets() {
                    let th = self.thumbs.get(&format!("sky:{name}")).cloned();
                    if widgets::tile(ui, th.as_ref(), Icon::Sun, name, false, 70.0).clicked() {
                        self.world.env.apply_weather(&preset);
                    }
                }
            });
        });
    }

    /// The object library panel (left), with a hover preview card.
    fn object_library(&mut self, ctx: &egui::Context, dock: Rect) {
        let Some(cat) = self.obj_cat else { return };
        let top = TITLE_H + 46.0;
        let height = (dock.top() - top - 12.0).max(200.0);
        let x = SIDEBAR_W + 14.0;
        let mut hovered: Option<LibItem> = None;
        let mut import_clicked = false;
        let mut remove_model: Option<String> = None;
        let panel = egui::Area::new(egui::Id::new("objlib")).fixed_pos(pos2(x, top)).show(ctx, |ui| {
            theme::panel_frame().show(ui, |ui| {
                ui.set_width(344.0);
                ui.set_height(height - 24.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(cat.info().1).size(14.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::tool_button(ui, Icon::Close, vec2(22.0, 22.0), false, "Close").clicked() {
                            self.obj_cat = None;
                        }
                    });
                });
                let search = egui::TextEdit::singleline(&mut self.obj_search).hint_text("Search").desired_width(f32::INFINITY);
                let sresp = ui.add(search);
                if self.focus_search {
                    sresp.request_focus();
                    self.focus_search = false;
                }
                ui.add_space(4.0);
                // Category icons (switch category without going back to the dock).
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                    for c in ObjCat::GRID {
                        let (icon, name) = c.info();
                        if widgets::tool_button(ui, icon, vec2(38.0, 36.0), c == cat, name).clicked() {
                            self.obj_cat = Some(c);
                            self.obj_sub = 0;
                        }
                    }
                });
                let subs = self.lib_subs(cat);
                if self.obj_sub >= subs.len() {
                    self.obj_sub = 0;
                }
                ui.horizontal(|ui| {
                    if subs.len() > 1 {
                        for (k, (icon, name, _)) in subs.iter().enumerate() {
                            if widgets::tool_button(ui, *icon, vec2(34.0, 30.0), self.obj_sub == k, name).clicked() {
                                self.obj_sub = k;
                            }
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::tool_button(ui, Icon::Import, vec2(30.0, 30.0), false, "Import 3D model (FBX, OBJ, glTF) as a library object").clicked() {
                            import_clicked = true;
                        }
                    });
                });
                let (_, sub_name, items) = &subs[self.obj_sub];
                let q = self.obj_search.to_lowercase();
                let items: Vec<LibItem> = if !q.is_empty() {
                    let mut all: Vec<LibItem> = ObjectKind::ALL.iter().map(|k| LibItem::Kind(*k)).collect();
                    all.extend(self.models.iter().map(|m| LibItem::Model(m.key.clone())));
                    all.into_iter()
                        .filter(|i| self.item_name(i).to_lowercase().contains(&q) || self.item_tags(i).iter().any(|t| t.to_lowercase().contains(&q)))
                        .collect()
                } else {
                    items.clone()
                };
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(cat.info().1.split(' ').next().unwrap_or("")).size(11.5).strong());
                    ui.label(
                        RichText::new(format!("›  {}", if q.is_empty() { sub_name.to_string() } else { format!("\"{}\"", self.obj_search) }))
                            .size(11.5)
                            .color(theme::TEXT_DIM),
                    );
                });
                ui.add_space(4.0);
                if items.is_empty() {
                    let msg = if cat == ObjCat::Favourites {
                        "Star objects in the preview card to collect them here."
                    } else if sub_name == "Imported" {
                        "Import your own FBX, OBJ or glTF models with the button above. They stay in this list for every project."
                    } else {
                        "Nothing found."
                    };
                    ui.label(RichText::new(msg).size(11.5).color(theme::TEXT_DIM));
                }
                egui::ScrollArea::vertical().id_salt("objgrid").show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for item in items {
                            let tkey = Self::item_thumb_key(&item);
                            let th = self.thumbs.get(&tkey).cloned();
                            let sel = self.tool == Tool::Place
                                && match &item {
                                    LibItem::Kind(k) => self.place_kind == *k,
                                    LibItem::Model(m) => self.place_kind == ObjectKind::Model && self.place_model.as_deref() == Some(m.as_str()),
                                };
                            let resp = widgets::tile(ui, th.as_ref(), Icon::Objects, "", sel, 76.0);
                            if th.is_none() && ui.is_rect_visible(resp.rect) {
                                if let LibItem::Model(m) = &item {
                                    self.thumbs.request(&tkey, Job::Model(m.clone()));
                                }
                            }
                            if resp.hovered() {
                                hovered = Some(item.clone());
                            }
                            if resp.clicked() {
                                self.select_item(&item);
                            }
                            if let LibItem::Model(m) = &item {
                                if m.starts_with("builtin:") {
                                    continue;
                                }
                                let key = m.clone();
                                resp.context_menu(|ui| {
                                    if ui.button("Remove from library").clicked() {
                                        remove_model = Some(key.clone());
                                        ui.close_menu();
                                    }
                                });
                            }
                        }
                    });
                });
            });
        });
        if import_clicked {
            self.ask_import(true);
        }
        if let Some(k) = remove_model {
            self.settings.imported_models.retain(|p| p.to_string_lossy() != k);
            self.settings.save();
            self.models = crate::library::scan_models_labeled(&self.settings.imported_models, &self.settings.model_labels);
        }
        // Hover card to the right of the panel (with a favourite star).
        let card_x = panel.response.rect.right() + 6.0;
        let over_card = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| p.x > card_x && p.x < card_x + 360.0 && p.y < top + 190.0);
        let show = hovered.or(self.lib_hover_item.clone().filter(|_| over_card));
        self.lib_hover_item = show.clone();
        if let Some(item) = show {
            let th = self.thumbs.get(&Self::item_thumb_key(&item)).cloned();
            let name = self.item_name(&item);
            let tags = self.item_tags(&item);
            let fav_key = item.fav_key();
            egui::Area::new(egui::Id::new("objcard")).fixed_pos(pos2(card_x, top)).show(ctx, |ui| {
                theme::panel_frame().show(ui, |ui| {
                    ui.set_width(340.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&name).size(13.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let fav = self.favourites.contains(&fav_key);
                            let icon = if fav { Icon::StarFilled } else { Icon::Star };
                            if widgets::tool_button(ui, icon, vec2(24.0, 24.0), false, if fav { "Remove from favourites" } else { "Add to favourites" })
                                .clicked()
                            {
                                if fav {
                                    self.favourites.remove(&fav_key);
                                } else {
                                    self.favourites.insert(fav_key.clone());
                                }
                                self.settings.favourites = self.favourites.iter().cloned().collect();
                                self.settings.save();
                            }
                        });
                    });
                    ui.horizontal_top(|ui| {
                        if let Some(t) = &th {
                            let (r, _) = ui.allocate_exact_size(vec2(120.0, 120.0), Sense::hover());
                            ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        ui.vertical(|ui| {
                            ui.set_width(200.0);
                            ui.horizontal_wrapped(|ui| {
                                for t in &tags {
                                    egui::Frame::none().fill(Color32::from_rgb(52, 53, 58)).rounding(2.0).inner_margin(egui::Margin::symmetric(5.0, 2.0)).show(
                                        ui,
                                        |ui| {
                                            ui.label(RichText::new(t.as_str()).size(11.0));
                                        },
                                    );
                                }
                            });
                        });
                    });
                });
            });
        }
    }

    /// Options shown to the right of the dock: placement settings or the
    /// properties of the selected object (Lumion's Position / Orientation panel).
    fn tool_options(&mut self, ctx: &egui::Context, dock: Rect) {
        let pos = pos2(dock.right() + 10.0, dock.bottom() - DOCK_BODY_H);
        match self.tool {
            Tool::Place => {
                let light = self.place_kind.is_light();
                egui::Area::new(egui::Id::new("place_opts")).fixed_pos(pos).show(ctx, |ui| {
                    Self::dock_block(ui, 250.0, |ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
                        ui.label(RichText::new(self.place_label()).strong());
                        if light {
                            widgets::bar(ui, "Height", &mut self.light_height, 0.0..=10.0, 240.0, |v| format!("{v:.1} m"));
                            widgets::log_bar(ui, "Brightness", &mut self.light_intensity, 1.0..=500.0, 240.0, |v| format!("{v:.0}"));
                            ui.horizontal(|ui| {
                                ui.color_edit_button_rgb(&mut self.light_color);
                                ui.label(RichText::new("Colour").color(theme::TEXT_DIM));
                            });
                        } else {
                            widgets::bar(ui, "Size", &mut self.place_scale, 0.3..=3.0, 240.0, |v| format!("{v:.2}×"));
                            match self.place_mode {
                                PlaceMode::Mass => {
                                    let mut n = self.mass_count as f32;
                                    if widgets::bar(ui, "Count", &mut n, 1.0..=100.0, 240.0, |v| format!("{:.0}", v)).changed() {
                                        self.mass_count = n.round() as u32;
                                    }
                                    widgets::bar(ui, "Radius", &mut self.mass_radius, 1.0..=60.0, 240.0, |v| format!("{v:.0} m"));
                                }
                                PlaceMode::Paint => {
                                    widgets::bar(ui, "Spacing", &mut self.paint_spacing, 0.5..=20.0, 240.0, |v| format!("{v:.1} m"));
                                }
                                PlaceMode::Single => {
                                    ui.label(RichText::new("Click in the view to place.").size(11.5).color(theme::TEXT_DIM));
                                }
                            }
                        }
                    });
                });
            }
            Tool::Delete if self.delete_mode == DeleteMode::Area => {
                egui::Area::new(egui::Id::new("del_opts")).fixed_pos(pos).show(ctx, |ui| {
                    Self::dock_block(ui, 250.0, |ui| {
                        ui.label(RichText::new("Area delete").strong());
                        widgets::bar(ui, "Radius", &mut self.mass_radius, 1.0..=60.0, 240.0, |v| format!("{v:.0} m"));
                        ui.label(RichText::new("Drag over objects to mark them, then press the bin.").size(11.5).color(theme::TEXT_DIM));
                    });
                });
            }
            Tool::Select => {
                let Some(&i) = self.selection.last() else { return };
                if i >= self.world.objects.len() {
                    return;
                }
                egui::Area::new(egui::Id::new("obj_props")).fixed_pos(pos).show(ctx, |ui| {
                    Self::dock_block(ui, 520.0, |ui| self.object_properties(ui, i));
                });
            }
            _ => {}
        }
    }

    fn object_properties(&mut self, ui: &mut egui::Ui, i: usize) {
        let n = self.selection.len();
        let before = self.world.objects[i].clone();
        let mut o = before.clone();
        ui.horizontal(|ui| {
            let oname = self.object_name(&o);
            ui.label(RichText::new(if n > 1 { format!("{oname} (+{} more)", n - 1) } else { oname }).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::tool_button_colored(ui, Icon::Delete, vec2(24.0, 24.0), false, None, "Delete (Del)").clicked() {
                    let sel = std::mem::take(&mut self.selection);
                    self.delete_objects(sel);
                }
            });
        });
        if i >= self.world.objects.len() {
            return;
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 3.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
                ui.label(RichText::new("Position").size(11.5).color(theme::TEXT_DIM));
                for (k, l) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized([14.0, 22.0], egui::Label::new(RichText::new(*l).color(theme::TEXT_DIM)));
                        ui.add_sized([110.0, 22.0], egui::DragValue::new(&mut o.position[k]).speed(0.05).max_decimals(2).suffix(" m"));
                    });
                }
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
                ui.label(RichText::new(if o.kind.is_light() { "Light" } else { "Orientation & size" }).size(11.5).color(theme::TEXT_DIM));
                if o.kind.is_light() {
                    widgets::log_bar(ui, "Brightness", &mut o.intensity, 1.0..=500.0, 330.0, |v| format!("{v:.0}"));
                    widgets::bar(ui, "Range", &mut o.range, 1.0..=100.0, 330.0, |v| format!("{v:.0} m"));
                    if o.kind == ObjectKind::SpotLight {
                        widgets::bar(ui, "Cone", &mut o.cone, 10.0..=170.0, 330.0, |v| format!("{v:.0}°"));
                    }
                    ui.horizontal(|ui| {
                        ui.color_edit_button_rgb(&mut o.color);
                        ui.label(RichText::new("Colour").color(theme::TEXT_DIM));
                    });
                } else {
                    let mut deg = o.rotation.to_degrees();
                    if widgets::bar(ui, "Heading", &mut deg, 0.0..=360.0, 330.0, |v| format!("{v:.0}°")).changed() {
                        o.rotation = deg.to_radians();
                    }
                    widgets::log_bar(ui, "Size", &mut o.scale, 0.05..=20.0, 330.0, |v| format!("{v:.2}×"));
                    if o.kind == ObjectKind::StreetLamp {
                        widgets::log_bar(ui, "Brightness", &mut o.intensity, 1.0..=500.0, 330.0, |v| format!("{v:.0}"));
                    }
                }
            });
        });
        let changed = o != before;
        if changed {
            let first_change = ui.input(|inp| inp.pointer.any_pressed()) || !ui.input(|inp| inp.pointer.any_down());
            if first_change {
                self.snapshot();
            }
            self.world.objects[i] = o;
            crate::objects::rebuild_one(&mut self.world.scene, &self.world.objects, i);
        }
    }
}

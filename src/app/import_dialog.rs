//! "Import model" dialog (Lumion-style): pick how a model file comes in
//! before it is loaded - as the live-synced building or as a placeable
//! library object - with name, category, units and axis.

use super::*;
use crate::import::{ImportSettings, Units, UpAxis};
use crate::ui::icons::Icon;
use crate::ui::widgets;

pub(super) struct ImportDialog {
    pub path: PathBuf,
    pub name: String,
    pub category: String,
    /// true = library object; false = the live-synced building model.
    pub as_object: bool,
    pub settings: ImportSettings,
}

impl ImportDialog {
    pub fn new(path: PathBuf, as_object: bool) -> Self {
        let name = path.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ")).unwrap_or_default();
        let settings = ImportSettings::default_for(&path);
        Self { path, name, category: "Imported".into(), as_object, settings }
    }
}

impl LumenApp {
    /// Pick a model file, then show the import dialog for it.
    pub(super) fn ask_import(&mut self, as_object: bool) {
        let dlg = rfd::FileDialog::new().add_filter("3D model", &["fbx", "gltf", "glb", "obj"]);
        if as_object {
            // Library objects: several files can be added in one go.
            let Some(mut files) = dlg.pick_files() else { return };
            if files.len() > 1 {
                self.import_model_files(files);
            } else if let Some(p) = files.pop() {
                self.import_dialog = Some(ImportDialog::new(p, true));
            }
        } else if let Some(p) = dlg.pick_file() {
            self.import_dialog = Some(ImportDialog::new(p, false));
        }
    }

    pub(super) fn import_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.import_dialog.take() else { return };
        let mut done: Option<bool> = None; // Some(true) = OK, Some(false) = cancel
        egui::Area::new(egui::Id::new("import_dialog")).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            theme::panel_frame().inner_margin(egui::Margin::same(18.0)).show(ui, |ui| {
                ui.set_width(440.0);
                ui.horizontal(|ui| {
                    if ui.button("<").on_hover_text("Cancel").clicked() {
                        done = Some(false);
                    }
                    ui.add_space(120.0);
                    ui.label(RichText::new("Import model").size(15.0));
                });
                ui.add_space(10.0);
                egui::Grid::new("import_grid").num_columns(2).spacing(vec2(16.0, 10.0)).show(ui, |ui| {
                    ui.label("File name");
                    ui.add(egui::Label::new(RichText::new(d.path.display().to_string()).size(11.0).color(theme::TEXT_DIM)).truncate())
                        .on_hover_text(d.path.display().to_string());
                    ui.end_row();
                    ui.label("Import as");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut d.as_object, false, "Building (LiveSync)")
                            .on_hover_text("The project's main model. Re-exports from Revit update it automatically.");
                        ui.selectable_value(&mut d.as_object, true, "Library object")
                            .on_hover_text("Placeable as many times as you like, like trees or furniture.");
                    });
                    ui.end_row();
                    if d.as_object {
                        ui.label("Model name");
                        ui.text_edit_singleline(&mut d.name);
                        ui.end_row();
                        ui.label("Category folder");
                        egui::ComboBox::from_id_salt("import_cat").selected_text(d.category.clone()).show_ui(ui, |ui| {
                            let mut cats: Vec<String> = self.models.iter().map(|m| m.category.clone()).collect();
                            cats.push("Imported".into());
                            cats.sort();
                            cats.dedup();
                            for c in cats {
                                ui.selectable_value(&mut d.category, c.clone(), c);
                            }
                        });
                        ui.end_row();
                    } else {
                        ui.label("Units");
                        egui::ComboBox::from_id_salt("import_units").selected_text(d.settings.units.label()).show_ui(ui, |ui| {
                            for u in Units::ALL {
                                ui.selectable_value(&mut d.settings.units, u, u.label());
                            }
                        });
                        ui.end_row();
                        ui.label("Up axis");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut d.settings.up, UpAxis::FromFile, "From file");
                            ui.selectable_value(&mut d.settings.up, UpAxis::Y, "Y");
                            ui.selectable_value(&mut d.settings.up, UpAxis::Z, "Z");
                        });
                        ui.end_row();
                        ui.label("Place on the plot");
                        ui.checkbox(&mut d.settings.recenter, "Move to the centre of the landscape, on the ground");
                        ui.end_row();
                    }
                });
                ui.add_space(8.0);
                let note = if d.as_object {
                    "Then click in the view to place it."
                } else if self.world.source.is_some() {
                    "Replaces the current building model and keeps watching this file for changes."
                } else {
                    "Render Mill keeps watching this file: export again from Revit and the scene updates."
                };
                ui.label(RichText::new(note).size(11.0).color(theme::TEXT_DIM));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(ui.available_width() - 120.0);
                    if widgets::flat_button(ui, Some(Icon::Check), "OK", true, 120.0).clicked() {
                        done = Some(true);
                    }
                });
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = Some(false);
        }
        match done {
            None => self.import_dialog = Some(d),
            Some(false) => {}
            Some(true) => self.finish_import(d),
        }
    }

    fn finish_import(&mut self, d: ImportDialog) {
        if d.as_object {
            if !self.settings.imported_models.contains(&d.path) {
                self.settings.imported_models.push(d.path.clone());
            }
            let key = d.path.to_string_lossy().to_string();
            self.settings.model_labels.insert(key.clone(), crate::library::ModelLabel { name: d.name.trim().to_string(), category: d.category.clone() });
            self.settings.save();
            self.models = crate::library::scan_models_labeled(&self.settings.imported_models, &self.settings.model_labels);
            if crate::objects::model_asset(&key).is_some() {
                self.select_item(&LibItem::Model(key));
                self.obj_cat = Some(ObjCat::All);
                self.obj_sub = 1;
            } else {
                self.push_log(format!("Could not read {}", d.path.display()));
            }
        } else {
            // A fresh building: these settings, not the ones of a previous model.
            self.world.source = None;
            let mut s = d.settings.clone();
            s.offset = None;
            s.texture_dirs = self.world.import.as_ref().map(|i| i.texture_dirs.clone()).unwrap_or_default();
            self.world.import = Some(s);
            self.start_file_sync_with(d.path, true);
        }
    }
}

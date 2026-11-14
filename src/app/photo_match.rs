//! "Match from photo": pick a photo of a house, see its materials split into
//! regions, the closest textures from the library for each, and apply them
//! to the model's materials in one click.

use super::*;
use crate::photomatch::{self, Analysis, DescCache, RegionKind, PALETTE};
use crate::ui::icons::Icon;
use crate::ui::widgets;
use egui::{pos2, Rect, Sense};
use std::sync::mpsc::{channel, Receiver};

enum Msg {
    Progress(String),
    Done(Box<MatchResult>),
    Failed(String),
}

struct MatchResult {
    analysis: Analysis,
    /// Per region: (index into `LumenApp::library`, similarity), best first.
    matches: Vec<Vec<(usize, f32)>>,
}

pub(super) struct PhotoMatch {
    path: PathBuf,
    rx: Option<Receiver<Msg>>,
    status: String,
    result: Option<MatchResult>,
    selected: usize,
    /// Per region: chosen match (index into its `matches`) and target material.
    choice: Vec<usize>,
    target: Vec<Option<String>>,
    applied: Vec<bool>,
    photo_tex: Option<(Option<usize>, egui::TextureHandle)>,
}

fn cache_path() -> Option<PathBuf> {
    crate::library::data_dir().map(|d| d.join("material-match-cache.json"))
}

impl LumenApp {
    pub(super) fn start_photo_match(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Photo", &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff"]).pick_file() else { return };
        self.photo_match_file(path);
    }

    pub(super) fn photo_match_file(&mut self, path: PathBuf) {
        self.photo_match = Some(PhotoMatch {
            path,
            rx: None,
            status: "Waiting for the material library...".into(),
            result: None,
            selected: 0,
            choice: Vec::new(),
            target: Vec::new(),
            applied: Vec::new(),
            photo_tex: None,
        });
    }

    /// Start the analysis once the library has been indexed.
    fn photo_match_launch(&mut self) {
        let Some(pm) = self.photo_match.as_mut() else { return };
        if pm.rx.is_some() || pm.result.is_some() || self.library_rx.is_some() {
            return;
        }
        let items: Vec<(usize, PathBuf)> = self.library.iter().enumerate().filter_map(|(i, m)| m.set.color.as_ref().map(|c| (i, PathBuf::from(c)))).collect();
        let (tx, rx) = channel();
        pm.rx = Some(rx);
        pm.status = "Reading the photo...".into();
        let photo = pm.path.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let img = match image::open(&photo) {
                Ok(i) => i,
                Err(e) => {
                    let _ = tx.send(Msg::Failed(format!("Could not read {}: {e}", photo.display())));
                    ctx.request_repaint();
                    return;
                }
            };
            let cp = cache_path();
            let mut cache = cp.as_deref().map(DescCache::load).unwrap_or_default();
            let paths: Vec<PathBuf> = items.iter().map(|(_, p)| p.clone()).collect();
            let ptx = tx.clone();
            let pctx = ctx.clone();
            let descs = photomatch::describe_library(&paths, &mut cache, &move |n, total| {
                if n % 8 == 0 || n == total {
                    let _ = ptx.send(Msg::Progress(format!("Learning the library textures ({n} of {total}; only the first time)...")));
                    pctx.request_repaint();
                }
            });
            if let Some(p) = &cp {
                cache.save(p);
            }
            let _ = tx.send(Msg::Progress("Analysing the photo...".into()));
            ctx.request_repaint();
            let analysis = photomatch::analyze(&img, 8);
            let matches =
                analysis.regions.iter().map(|r| photomatch::best_matches(&r.desc, &descs, 6).into_iter().map(|(j, s)| (items[j].0, s)).collect()).collect();
            let _ = tx.send(Msg::Done(Box::new(MatchResult { analysis, matches })));
            ctx.request_repaint();
        });
    }

    fn model_materials(&self) -> Vec<String> {
        let mut used: Vec<String> =
            self.world.scene.meshes.iter().filter(|(k, _)| !k.starts_with("env:") && !k.starts_with("obj:")).map(|(_, m)| m.data.material.clone()).collect();
        used.sort();
        used.dedup();
        used
    }

    pub(super) fn photo_match_ui(&mut self, ctx: &egui::Context) {
        if self.photo_match.is_none() {
            return;
        }
        self.photo_match_launch();
        // Collect messages.
        let mut done = None;
        if let Some(pm) = self.photo_match.as_mut() {
            if let Some(rx) = &pm.rx {
                while let Ok(m) = rx.try_recv() {
                    match m {
                        Msg::Progress(s) => pm.status = s,
                        Msg::Failed(s) => {
                            pm.status = s;
                            pm.rx = None;
                            break;
                        }
                        Msg::Done(r) => {
                            done = Some(r);
                            break;
                        }
                    }
                }
            }
        }
        if let Some(r) = done {
            let mats = self.model_materials();
            let pm = self.photo_match.as_mut().unwrap();
            pm.rx = None;
            let n = r.analysis.regions.len();
            pm.choice = vec![0; n];
            pm.applied = vec![false; n];
            pm.target = (0..n)
                .map(|i| {
                    let reg = &r.analysis.regions[i];
                    if reg.kind != RegionKind::Surface {
                        return None;
                    }
                    let best = r.matches[i].first()?;
                    photomatch::suggest_target(&self.library[best.0].category, &mats).cloned()
                })
                .collect();
            // Start on the biggest building surface.
            pm.selected = (0..n)
                .filter(|&i| r.analysis.regions[i].kind == RegionKind::Surface)
                .max_by(|&a, &b| r.analysis.regions[a].share.total_cmp(&r.analysis.regions[b].share))
                .unwrap_or(0);
            pm.result = Some(*r);
            pm.photo_tex = None;
        }

        let mut close = false;
        let mut apply: Vec<usize> = Vec::new();
        let mats = self.model_materials();
        let mut open = true;
        let mut retired = Vec::new();
        egui::Window::new("Material match from photo")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(940.0)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let Some(pm) = self.photo_match.as_mut() else { return };
                ui.label(RichText::new(pm.path.display().to_string()).size(11.0).color(theme::TEXT_DIM));
                let Some(res) = &pm.result else {
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(&pm.status);
                    });
                    ui.add_space(20.0);
                    return;
                };
                let a = &res.analysis;
                ui.horizontal_top(|ui| {
                    // Photo with the regions tinted; click to select one.
                    ui.vertical(|ui| {
                        let want = Some(pm.selected);
                        if pm.photo_tex.as_ref().map(|t| t.0) != Some(want) {
                            let img = photomatch::overlay(a, want, 0.35);
                            let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], &img);
                            let new = (want, ui.ctx().load_texture("photo_match", ci, egui::TextureOptions::LINEAR));
                            if let Some((_, old)) = pm.photo_tex.replace(new) {
                                retired.push(old);
                            }
                        }
                        let (iw, ih) = (a.image.width() as f32, a.image.height() as f32);
                        let scale = (520.0 / iw).min(460.0 / ih);
                        let (rect, resp) = ui.allocate_exact_size(vec2(iw * scale, ih * scale), Sense::click());
                        if let Some((_, t)) = &pm.photo_tex {
                            ui.painter().image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.clicked()) {
                            let x = ((p.x - rect.left()) / scale) as usize;
                            let y = ((p.y - rect.top()) / scale) as usize;
                            if x < a.image.width() as usize && y < a.image.height() as usize {
                                pm.selected = a.labels[y * a.image.width() as usize + x] as usize;
                            }
                        }
                        ui.label(RichText::new("Click a part of the photo to pick that material.").size(11.0).color(theme::TEXT_DIM));
                    });
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.set_width(380.0);
                        theme::section_title(ui, "Materials found in the photo");
                        for (i, r) in a.regions.iter().enumerate() {
                            let best = res.matches[i].get(pm.choice[i]).map(|(j, _)| self.library[*j].name.clone()).unwrap_or_default();
                            let what = match r.kind {
                                RegionKind::Sky => "sky".to_string(),
                                RegionKind::Plants => "plants".to_string(),
                                RegionKind::Surface => best,
                            };
                            let (row, resp) = ui.allocate_exact_size(vec2(370.0, 22.0), Sense::click());
                            let bg = if pm.selected == i { theme::SELECTED } else if resp.hovered() { Color32::from_rgb(52, 53, 58) } else { Color32::TRANSPARENT };
                            let p = ui.painter();
                            p.rect_filled(row, 2.0, bg);
                            let pc = PALETTE[i % PALETTE.len()];
                            p.rect_filled(Rect::from_min_size(row.min + vec2(4.0, 5.0), vec2(6.0, 12.0)), 1.0, Color32::from_rgb(pc[0], pc[1], pc[2]));
                            p.rect_filled(Rect::from_min_size(row.min + vec2(12.0, 5.0), vec2(12.0, 12.0)), 2.0, Color32::from_rgb(r.color[0], r.color[1], r.color[2]));
                            let mut line = format!("{:.0}%  {what}", r.share * 100.0);
                            if let Some(t) = &pm.target[i] {
                                line += &format!("  ›  {t}");
                            }
                            if pm.applied[i] {
                                line += "  (applied)";
                            }
                            let col = if r.kind == RegionKind::Surface { theme::TEXT } else { theme::TEXT_DIM };
                            p.text(row.min + vec2(30.0, 11.0), Align2::LEFT_CENTER, line, egui::FontId::proportional(11.5), col);
                            if resp.clicked() {
                                pm.selected = i;
                            }
                        }
                        ui.add_space(8.0);
                        let i = pm.selected.min(a.regions.len().saturating_sub(1));
                        theme::section_title(ui, "Closest library textures");
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                            for (k, (j, score)) in res.matches[i].iter().enumerate() {
                                let m = &self.library[*j];
                                let key = Self::set_key(&m.set);
                                let th = self.thumbs.get(&key).cloned();
                                if th.is_none() {
                                    self.thumbs.request(&key, Job::TextureSet(m.set.clone()));
                                }
                                let r = widgets::tile(ui, th.as_ref(), Icon::Materials, &format!("{:.0}%", score * 100.0), pm.choice[i] == k, 56.0)
                                    .on_hover_text(format!("{}\n{} · {:.0}% similar", m.name, m.category, score * 100.0));
                                if r.clicked() {
                                    pm.choice[i] = k;
                                    pm.applied[i] = false;
                                }
                            }
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label("Apply to");
                            let cur = pm.target[i].clone().unwrap_or_else(|| "(choose a model material)".into());
                            egui::ComboBox::from_id_salt("pm_target").width(230.0).selected_text(cur).show_ui(ui, |ui| {
                                if ui.selectable_label(pm.target[i].is_none(), "(none)").clicked() {
                                    pm.target[i] = None;
                                }
                                if let Some(m) = &self.mat {
                                    if ui.selectable_label(false, format!("{} (being edited)", m.name)).clicked() {
                                        pm.target[i] = Some(m.name.clone());
                                    }
                                }
                                for m in &mats {
                                    if ui.selectable_label(pm.target[i].as_deref() == Some(m.as_str()), m).clicked() {
                                        pm.target[i] = Some(m.clone());
                                        pm.applied[i] = false;
                                    }
                                }
                            });
                        });
                        if mats.is_empty() {
                            ui.label(RichText::new("Import or LiveSync a model first: matches are applied to its materials.").size(11.0).color(theme::WARN));
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if widgets::flat_button(ui, Some(Icon::Check), "Apply", true, 120.0).clicked() && pm.target[i].is_some() {
                                apply.push(i);
                            }
                            let n = (0..a.regions.len()).filter(|&k| pm.target[k].is_some() && !pm.applied[k]).count();
                            if widgets::flat_button(ui, Some(Icon::Materials), &format!("Apply all ({n})"), false, 130.0).clicked() {
                                apply.extend((0..a.regions.len()).filter(|&k| pm.target[k].is_some() && !pm.applied[k]));
                            }
                            if widgets::flat_button(ui, Some(Icon::Folder), "Other photo...", false, 110.0).clicked() {
                                close = true;
                            }
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("On-device image analysis (colour and texture). Matches are only as good as the photo and the textures in your library - check them before rendering.")
                                .size(10.5)
                                .color(theme::TEXT_DIM),
                        );
                    });
                });
            });
        self.retired_textures.extend(retired);
        let mut log = Vec::new();
        if let Some(pm) = self.photo_match.as_mut() {
            if let Some(res) = &pm.result {
                for &i in &apply {
                    if let (Some(t), Some((j, _))) = (pm.target[i].clone(), res.matches[i].get(pm.choice[i])) {
                        log.push((t, *j, i));
                    }
                }
            }
        }
        for (target, j, i) in log {
            let lm = self.library[j].clone();
            self.apply_texture_set(&target, &lm.set);
            if lm.metallic {
                let mut p = self.world.scene.effective(&target);
                p.metallic = 1.0;
                self.world.scene.overrides.insert(target.clone(), p);
            }
            if let Some(pm) = self.photo_match.as_mut() {
                pm.applied[i] = true;
            }
        }
        if close || !open {
            if let Some((_, t)) = self.photo_match.as_mut().and_then(|pm| pm.photo_tex.take()) {
                self.retired_textures.push(t);
            }
        }
        if close {
            self.photo_match = None;
            self.start_photo_match();
            return;
        }
        if !open {
            self.photo_match = None;
        }
    }
}

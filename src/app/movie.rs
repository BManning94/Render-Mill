//! Movie mode (Lumion's video editor, simplified): clips are camera paths
//! through keyframes. Preview them, then render to a PNG sequence and, when
//! ffmpeg is available, an MP4.

use super::*;
use crate::env::Clip;
use crate::ui::icons::{self, Icon};
use crate::ui::widgets;
use egui::{pos2, Rect, Sense, Stroke, UiBuilder};
use std::path::Path;

const STRIP_H: f32 = 118.0;
const LEFT_W: f32 = 290.0;
const TIMELINE_H: f32 = 132.0;

/// Movie-mode state that isn't saved with the project.
pub(super) struct MovieState {
    pub clip: usize,
    pub sel_key: Option<usize>,
    pub playing: bool,
    pub t: f32,
    pub last_tick: Instant,
    pub key_thumbs: HashMap<(usize, usize), (Camera, egui::TextureHandle)>,
    pub popup: bool,
    pub fps: u32,
    pub res: (u32, u32),
    pub job: Option<MovieJob>,
    /// Ray tracing level for movie frames (0 = off).
    pub raytrace: u32,
}

impl Default for MovieState {
    fn default() -> Self {
        Self {
            clip: 0,
            sel_key: None,
            playing: false,
            t: 0.0,
            last_tick: Instant::now(),
            key_thumbs: HashMap::new(),
            popup: false,
            raytrace: 0,
            fps: 30,
            res: (1920, 1080),
            job: None,
        }
    }
}

/// A movie render in progress: one frame is rendered per UI update so the
/// app stays responsive and can show progress / be cancelled.
pub(super) struct MovieJob {
    clips: Vec<usize>,
    cur: usize,
    frame: u32,
    out_dir: PathBuf,
    targets: Option<Targets>,
    viewport_q: Quality,
    started: Instant,
    done_dirs: Vec<(PathBuf, String)>,
    pt: Option<(crate::pathtrace::PtJob, FrameParams)>,
}

impl MovieJob {
    fn frames_of(clip: &Clip, fps: u32) -> u32 {
        ((clip.duration * fps as f32).round() as u32).max(1)
    }
}

/// ffmpeg on PATH or next to the executable.
pub fn find_ffmpeg() -> Option<PathBuf> {
    static FOUND: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    FOUND.get_or_init(probe_ffmpeg).clone()
}

fn probe_ffmpeg() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let p = d.join(exe);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    hide_console(&mut cmd);
    cmd.status().ok().filter(|s| s.success()).map(|_| PathBuf::from(exe))
}

fn hide_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
    let n = n.trim().to_string();
    if n.is_empty() {
        "clip".into()
    } else {
        n
    }
}

impl LumenApp {
    fn ensure_clip(&mut self) {
        if self.world.clips.is_empty() {
            self.world.clips.push(Clip::new("Clip 1".into()));
        }
        if self.movie.clip >= self.world.clips.len() {
            self.movie.clip = self.world.clips.len() - 1;
        }
    }

    /// Advance playback and drive the camera from the clip while playing.
    fn movie_tick(&mut self, ctx: &egui::Context) {
        let dt = self.movie.last_tick.elapsed().as_secs_f32().min(0.2);
        self.movie.last_tick = Instant::now();
        if !self.movie.playing {
            return;
        }
        let clip = &self.world.clips[self.movie.clip];
        if clip.keys.len() < 2 {
            self.movie.playing = false;
            return;
        }
        self.movie.t += dt;
        if self.movie.t >= clip.duration {
            self.movie.t = clip.duration;
            self.movie.playing = false;
        }
        if let Some(c) = clip.camera_at(self.movie.t) {
            self.world.camera = c;
        }
        ctx.request_repaint();
    }

    pub(super) fn movie_mode(&mut self, ui: &mut egui::Ui) {
        self.ensure_clip();
        self.movie_tick(ui.ctx());
        let area = ui.max_rect();
        let strip = Rect::from_min_max(pos2(area.left(), area.bottom() - STRIP_H), area.max);
        let left = Rect::from_min_max(area.min + vec2(8.0, 8.0), pos2(area.left() + 8.0 + LEFT_W, strip.top() - 8.0));
        let stage = Rect::from_min_max(pos2(left.right() + 12.0, area.top() + 8.0), pos2(area.right() - 12.0, strip.top() - 8.0));
        let aspect = self.movie.res.0 as f32 / self.movie.res.1 as f32;
        let avail = Rect::from_min_max(stage.min + vec2(20.0, 10.0), stage.max - vec2(20.0, TIMELINE_H + 10.0));
        let (pw, ph) =
            if avail.width() / avail.height() > aspect { (avail.height() * aspect, avail.height()) } else { (avail.width(), avail.width() / aspect) };
        let preview = Rect::from_center_size(avail.center(), vec2(pw.max(32.0), ph.max(32.0)));
        ui.painter().rect_filled(stage, 4.0, Color32::from_rgb(28, 29, 31));
        self.viewport(ui, preview);
        ui.painter().rect_stroke(preview, 0.0, Stroke::new(1.0_f32, Color32::from_white_alpha(30)));
        let n_keys = self.world.clips[self.movie.clip].keys.len();
        let hint = if n_keys == 0 {
            "Move the camera to the first shot, then press + Keyframe. Add more keyframes to make a camera path."
        } else if n_keys == 1 {
            "Move to the next shot and add another keyframe."
        } else {
            "Press play to preview the clip, or drag the Time bar to scrub through it."
        };
        ui.painter().text(preview.left_bottom() + vec2(8.0, -8.0), Align2::LEFT_BOTTOM, hint, egui::FontId::proportional(11.0), Color32::from_white_alpha(200));
        let tl = Rect::from_min_size(pos2(preview.center().x - 380.0, preview.bottom() + 8.0), vec2(760.0, TIMELINE_H - 8.0));
        ui.allocate_new_ui(UiBuilder::new().max_rect(tl), |ui| self.movie_timeline(ui));
        ui.allocate_new_ui(UiBuilder::new().max_rect(left), |ui| {
            theme::panel_frame().show(ui, |ui| {
                ui.set_width(LEFT_W - 24.0);
                ui.set_min_height(left.height() - 24.0);
                self.movie_left_panel(ui);
            });
        });
        ui.allocate_new_ui(UiBuilder::new().max_rect(strip), |ui| self.movie_strip(ui, strip));
    }

    fn movie_left_panel(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Clip").size(13.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::tool_button(ui, Icon::Delete, vec2(26.0, 26.0), false, "Delete this clip").clicked() && self.world.clips.len() > 1 {
                    self.world.clips.remove(self.movie.clip);
                    self.movie.clip = self.movie.clip.saturating_sub(1);
                    self.movie.key_thumbs.clear();
                    self.movie.sel_key = None;
                }
            });
        });
        let c = self.movie.clip;
        ui.add(egui::TextEdit::singleline(&mut self.world.clips[c].name).desired_width(f32::INFINITY));
        let clip = &mut self.world.clips[c];
        widgets::bar(ui, "Length", &mut clip.duration, 2.0..=120.0, LEFT_W - 24.0, |v| format!("{v:.1} s"));
        ui.label(RichText::new(format!("{} keyframe(s)", clip.keys.len())).size(11.5).color(theme::TEXT_DIM));
        ui.separator();
        ui.label(RichText::new("Effects").size(12.5).strong());
        ui.label(
            RichText::new("Movies use the same effect list and style as Photo mode (sun, fog, depth of field, film grain...). Clouds, water and plants animate over the clip.")
                .size(11.0)
                .color(theme::TEXT_DIM),
        );
        if widgets::flat_button(ui, Some(Icon::Photo), "Edit effects in Photo mode", false, LEFT_W - 24.0).clicked() {
            self.mode = Mode::Photo;
        }
        ui.horizontal(|ui| {
            widgets::toggle(ui, &mut self.world.env.animate);
            ui.label("Animate clouds, water & plants");
        });
        ui.separator();
        ui.label(RichText::new("Output").size(12.5).strong());
        match find_ffmpeg() {
            Some(_) => ui.label(RichText::new("ffmpeg found: clips are also saved as MP4.").size(11.0).color(theme::GOOD)),
            None => ui.label(
                RichText::new("Clips render to numbered PNG frames. Put ffmpeg.exe next to lumencraft.exe (or on PATH) to also get MP4 files.")
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            ),
        };
    }

    fn movie_timeline(&mut self, ui: &mut egui::Ui) {
        let c = self.movie.clip;
        let duration = self.world.clips[c].duration;
        let n = self.world.clips[c].keys.len();
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        ui.horizontal(|ui| {
            let playing = self.movie.playing;
            let (r, resp) = ui.allocate_exact_size(vec2(34.0, 30.0), Sense::click());
            ui.painter().rect_filled(r, 3.0, if resp.hovered() { theme::SELECTED } else { Color32::from_rgb(48, 49, 54) });
            if playing {
                for x in [-5.0, 5.0] {
                    ui.painter().rect_filled(Rect::from_center_size(r.center() + vec2(x, 0.0), vec2(4.0, 14.0)), 1.0, Color32::WHITE);
                }
            } else {
                let cpt = r.center();
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![cpt + vec2(-5.0, -8.0), cpt + vec2(8.0, 0.0), cpt + vec2(-5.0, 8.0)],
                    Color32::WHITE,
                    Stroke::NONE,
                ));
            }
            if resp.on_hover_text(if playing { "Pause" } else { "Play the clip" }).clicked() {
                if playing {
                    self.movie.playing = false;
                } else if n >= 2 {
                    if self.movie.t >= duration - 1e-3 {
                        self.movie.t = 0.0;
                    }
                    self.movie.playing = true;
                    self.movie.last_tick = Instant::now();
                } else {
                    self.push_log("Add at least two keyframes to play a clip");
                }
            }
            let mut t = self.movie.t;
            if widgets::bar(ui, "Time", &mut t, 0.0..=duration, 420.0, |v| format!("{v:.1} s")).changed() {
                self.movie.t = t;
                self.movie.playing = false;
                if let Some(cam) = self.world.clips[c].camera_at(t) {
                    self.world.camera = cam;
                }
            }
            ui.label(RichText::new(format!("{:.1} / {:.0} s", self.movie.t, duration)).size(11.5).color(theme::TEXT_DIM));
            if widgets::flat_button(ui, Some(Icon::Plus), "Keyframe", true, 110.0).on_hover_text("Add the current camera as the next keyframe").clicked() {
                let at = self.movie.sel_key.map(|k| k + 1).unwrap_or(n);
                self.world.clips[c].keys.insert(at, self.world.camera.clone());
                self.movie.sel_key = Some(at);
                self.movie.key_thumbs.clear();
            }
        });
        // Keyframe thumbnails along the clip.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
            let mut action: Option<(usize, u8)> = None; // 0 go, 1 update, 2 delete
            egui::ScrollArea::horizontal().id_salt("keys").max_width(760.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    for k in 0..n {
                        let cam = self.world.clips[c].keys[k].clone();
                        let stale = self.movie.key_thumbs.get(&(c, k)).is_none_or(|(kc, _)| *kc != cam);
                        if stale {
                            let ci = self.render_small(&cam);
                            let h = self.ctx.load_texture(format!("key{c}_{k}"), ci, egui::TextureOptions::LINEAR);
                            self.movie.key_thumbs.insert((c, k), (cam.clone(), h));
                        }
                        let (r, resp) = ui.allocate_exact_size(vec2(128.0, 72.0), Sense::click());
                        if let Some((_, t)) = self.movie.key_thumbs.get(&(c, k)) {
                            ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        let sel = self.movie.sel_key == Some(k);
                        ui.painter().rect_stroke(
                            r,
                            2.0,
                            if sel {
                                Stroke::new(2.0_f32, theme::ACCENT)
                            } else if resp.hovered() {
                                Stroke::new(1.5_f32, Color32::WHITE)
                            } else {
                                Stroke::NONE
                            },
                        );
                        ui.painter().text(
                            r.left_top() + vec2(5.0, 4.0),
                            Align2::LEFT_TOP,
                            format!("{}", k + 1),
                            egui::FontId::proportional(10.5),
                            Color32::WHITE,
                        );
                        if resp.clone().on_hover_text("Click: go to this keyframe · right-click: update / delete").clicked() {
                            action = Some((k, 0));
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Update with current camera").clicked() {
                                action = Some((k, 1));
                                ui.close_menu();
                            }
                            if ui.button("Delete keyframe").clicked() {
                                action = Some((k, 2));
                                ui.close_menu();
                            }
                        });
                    }
                    if n == 0 {
                        ui.label(RichText::new("No keyframes yet.").size(11.5).color(theme::TEXT_DIM));
                    }
                });
            });
            match action {
                Some((k, 0)) => {
                    self.world.camera = self.world.clips[c].keys[k].clone();
                    self.movie.sel_key = Some(k);
                    self.movie.playing = false;
                    if n > 1 {
                        self.movie.t = duration * k as f32 / (n - 1) as f32;
                    }
                }
                Some((k, 1)) => {
                    self.world.clips[c].keys[k] = self.world.camera.clone();
                }
                Some((k, 2)) => {
                    self.world.clips[c].keys.remove(k);
                    self.movie.key_thumbs.clear();
                    self.movie.sel_key = None;
                }
                _ => {}
            }
        });
    }

    fn movie_strip(&mut self, ui: &mut egui::Ui, strip: Rect) {
        ui.painter().rect_filled(strip, 0.0, Color32::from_rgb(28, 29, 31));
        let slot = vec2(150.0, 84.0);
        let y = strip.center().y - slot.y / 2.0;
        let rb = Rect::from_min_size(pos2(strip.right() - 96.0, strip.center().y - 42.0), vec2(84.0, 84.0));
        let rresp = ui.interact(rb, egui::Id::new("movie_render_btn"), Sense::click());
        ui.painter().rect_filled(rb, 2.0, if rresp.hovered() { Color32::from_rgb(52, 152, 240) } else { theme::ACCENT });
        icons::draw(ui.painter(), Icon::Film, Rect::from_center_size(rb.center(), vec2(36.0, 36.0)), Color32::WHITE);
        if rresp.on_hover_text("Render movie").clicked() {
            self.movie.popup = !self.movie.popup;
        }
        let mut x = strip.left() + 14.0;
        let mut select = None;
        for i in 0..self.world.clips.len() {
            if x + slot.x > rb.left() - 170.0 {
                break;
            }
            let r = Rect::from_min_size(pos2(x, y), slot);
            let resp = ui.interact(r, egui::Id::new(("clip", i)), Sense::click());
            ui.painter().rect_filled(r, 2.0, Color32::from_rgb(44, 45, 49));
            if let Some((_, t)) = self.movie.key_thumbs.get(&(i, 0)) {
                ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else if let Some(cam) = self.world.clips[i].keys.first().cloned() {
                let ci = self.render_small(&cam);
                let h = self.ctx.load_texture(format!("key{i}_0"), ci, egui::TextureOptions::LINEAR);
                self.movie.key_thumbs.insert((i, 0), (cam, h));
            } else {
                icons::draw(ui.painter(), Icon::Film, Rect::from_center_size(r.center(), vec2(24.0, 24.0)), Color32::from_white_alpha(70));
            }
            let on = i == self.movie.clip;
            ui.painter().rect_stroke(
                r,
                2.0,
                if on {
                    Stroke::new(2.0_f32, Color32::WHITE)
                } else if resp.hovered() {
                    Stroke::new(1.5_f32, Color32::from_white_alpha(150))
                } else {
                    Stroke::NONE
                },
            );
            let label = format!("{}  ·  {:.0} s", self.world.clips[i].name, self.world.clips[i].duration);
            let g = ui.painter().layout_no_wrap(label, egui::FontId::proportional(10.5), Color32::WHITE);
            let lr = Rect::from_min_size(r.left_bottom() - vec2(0.0, 18.0), vec2(r.width(), 18.0));
            ui.painter().rect_filled(lr, 0.0, Color32::from_black_alpha(150));
            ui.painter().galley(lr.left_top() + vec2(5.0, 2.0), g, Color32::WHITE);
            if resp.clicked() {
                select = Some(i);
            }
            x += slot.x + 8.0;
        }
        // "+ New clip" slot
        let r = Rect::from_min_size(pos2(x, y), slot);
        let resp = ui.interact(r, egui::Id::new("new_clip"), Sense::click());
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0_f32, if resp.hovered() { Color32::WHITE } else { Color32::from_white_alpha(60) }));
        icons::draw(ui.painter(), Icon::Plus, Rect::from_center_size(r.center() - vec2(0.0, 8.0), vec2(22.0, 22.0)), Color32::from_white_alpha(180));
        ui.painter().text(r.center() + vec2(0.0, 18.0), Align2::CENTER_CENTER, "New clip", egui::FontId::proportional(11.0), theme::TEXT_DIM);
        if resp.clicked() {
            let n = self.world.clips.len() + 1;
            self.world.clips.push(Clip::new(format!("Clip {n}")));
            select = Some(self.world.clips.len() - 1);
        }
        if let Some(i) = select {
            self.movie.clip = i;
            self.movie.sel_key = None;
            self.movie.playing = false;
            self.movie.t = 0.0;
            if let Some(cam) = self.world.clips[i].keys.first() {
                self.world.camera = cam.clone();
            }
        }
    }

    pub(super) fn movie_overlays(&mut self, ctx: &egui::Context) {
        if self.movie.job.is_some() {
            self.movie_render_step(ctx);
            return;
        }
        if !self.movie.popup {
            return;
        }
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("movie_popup"))
            .fixed_pos(pos2(screen.right() - 360.0, screen.bottom() - STRIP_H - 340.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                theme::panel_frame().show(ui, |ui| {
                    ui.set_width(320.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Render movie").size(14.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::tool_button(ui, Icon::Close, vec2(22.0, 22.0), false, "Close").clicked() {
                                self.movie.popup = false;
                            }
                        });
                    });
                    ui.label(RichText::new("Size").color(theme::TEXT_DIM));
                    ui.horizontal(|ui| {
                        for (r, label) in [((1280, 720), "HD 720p"), ((1920, 1080), "Full HD"), ((3840, 2160), "4K")] {
                            if ui.selectable_label(self.movie.res == r, label).clicked() {
                                self.movie.res = r;
                            }
                        }
                    });
                    ui.label(RichText::new("Frame rate").color(theme::TEXT_DIM));
                    ui.horizontal(|ui| {
                        for f in [24u32, 30, 60] {
                            if ui.selectable_label(self.movie.fps == f, format!("{f} fps")).clicked() {
                                self.movie.fps = f;
                            }
                        }
                    });
                    let clip = &self.world.clips[self.movie.clip];
                    let frames = MovieJob::frames_of(clip, self.movie.fps);
                    ui.label(RichText::new(format!("{frames} frames · rendered at the highest quality")).size(11.0).color(theme::TEXT_DIM));
                    ui.label(RichText::new("Ray tracing").color(theme::TEXT_DIM))
                        .on_hover_text("Path trace every frame (with denoising). Much slower than rasterized frames.");
                    ui.horizontal(|ui| {
                        for (i, label) in super::raytrace::LEVELS.iter().enumerate() {
                            if ui.selectable_label(self.movie.raytrace == i as u32, *label).clicked() {
                                self.movie.raytrace = i as u32;
                            }
                        }
                    });
                    ui.add_space(6.0);
                    let ok = clip.keys.len() >= 2;
                    ui.add_enabled_ui(ok, |ui| {
                        if widgets::flat_button(ui, Some(Icon::Film), "Render this clip", true, 296.0).clicked() {
                            self.start_movie_render(vec![self.movie.clip]);
                        }
                        let all: Vec<usize> = (0..self.world.clips.len()).filter(|&i| self.world.clips[i].keys.len() >= 2).collect();
                        if all.len() > 1 && widgets::flat_button(ui, Some(Icon::Film), &format!("Render all clips ({})", all.len()), false, 296.0).clicked() {
                            self.start_movie_render(all);
                        }
                    });
                    if !ok {
                        ui.label(RichText::new("This clip needs at least two keyframes.").size(11.0).color(theme::WARN));
                    }
                });
            });
    }

    fn start_movie_render(&mut self, clips: Vec<usize>) {
        let (w, h) = self.movie.res;
        if let Err(e) = crate::pathtrace::check_resolution(&self.renderer.device.limits(), w, h, self.movie.raytrace > 0) {
            self.push_log(format!("Movie: {e}"));
            self.movie.res = e.max_supported;
            self.push_log(format!("Movie size set to {} x {}; press Render again to use it.", e.max_supported.0, e.max_supported.1));
            return;
        }
        let Some(dir) = rfd::FileDialog::new().set_title("Folder for the movie").pick_folder() else { return };
        self.movie.popup = false;
        self.movie.playing = false;
        let viewport_q = self.renderer.quality;
        self.renderer.set_quality(Quality::preset(4));
        self.renderer.invalidate_shadows();
        self.renderer.set_highlight(None, None);
        self.movie.job =
            Some(MovieJob { clips, cur: 0, frame: 0, out_dir: dir, targets: None, viewport_q, started: Instant::now(), done_dirs: Vec::new(), pt: None });
        if self.movie.raytrace > 0 {
            self.tracer();
        }
    }

    fn movie_render_step(&mut self, ctx: &egui::Context) {
        let Some(mut job) = self.movie.job.take() else { return };
        let (w, h) = self.movie.res;
        let fps = self.movie.fps;
        let mut cancel = false;
        if job.cur < job.clips.len() {
            let clip = self.world.clips[job.clips[job.cur]].clone();
            let frames = MovieJob::frames_of(&clip, fps);
            let frame_dir = job.out_dir.join(format!("{} frames", safe_name(&clip.name)));
            if job.frame == 0 {
                let _ = std::fs::create_dir_all(&frame_dir);
            }
            let targets = job.targets.get_or_insert_with(|| self.renderer.create_targets(w, h));
            let t = job.frame as f32 / fps as f32;
            let rt_opts = self.rt.movie_opts(self.movie.raytrace);
            let mut frame_done = true;
            if let (Some(opts), Some(cam)) = (rt_opts, clip.camera_at(t)) {
                // Ray-traced frame, accumulated over several UI updates.
                if job.pt.is_none() {
                    let saved = std::mem::replace(&mut self.world.camera, cam);
                    let fp = self.world.frame_params(w as f32 / h as f32, 10.0 + t);
                    self.world.camera = saved;
                    self.renderer.sync_scene(&self.world.scene);
                    self.renderer.settle = true;
                    let mut tracer = self.rt.tracer.take().unwrap_or_else(|| crate::pathtrace::PathTracer::new(&self.renderer.device));
                    self.apply_texture_budget(Some(&mut tracer));
                    let mut pj = tracer.begin(&mut self.renderer, &self.world.scene, &fp, targets, opts);
                    pj.target_ms = 80.0;
                    self.rt.tracer = Some(tracer);
                    job.pt = Some((pj, fp));
                }
                let (pj, fp) = job.pt.as_mut().unwrap();
                let tracer = self.rt.tracer.as_mut().expect("tracer");
                tracer.step(&self.renderer, pj, std::time::Duration::from_millis(120));
                if pj.finished() {
                    let (img, _) = tracer.finish(&mut self.renderer, &self.world.scene, fp, targets, pj);
                    if let Some((pj, _)) = job.pt.take() {
                        tracer.recycle(pj);
                    }
                    let path = frame_dir.join(format!("frame_{:05}.png", job.frame));
                    if let Err(e) = img.save(&path) {
                        self.push_log(format!("Could not save {}: {e}", path.display()));
                        cancel = true;
                    }
                } else {
                    frame_done = false;
                }
            } else if let Some(cam) = clip.camera_at(t) {
                let saved = std::mem::replace(&mut self.world.camera, cam);
                let fp = self.world.frame_params(w as f32 / h as f32, 10.0 + t);
                self.world.camera = saved;
                self.renderer.sync_scene(&self.world.scene);
                self.renderer.settle = true;
                self.renderer.render(&self.world.scene, &fp, targets);
                let img = self.renderer.read_output(targets);
                let path = frame_dir.join(format!("frame_{:05}.png", job.frame));
                if let Err(e) = img.save(&path) {
                    self.push_log(format!("Could not save {}: {e}", path.display()));
                    cancel = true;
                }
            }
            if frame_done {
                job.frame += 1;
            }
            if job.frame >= frames {
                job.done_dirs.push((frame_dir, safe_name(&clip.name)));
                job.cur += 1;
                job.frame = 0;
            }
        }
        // Progress card
        let total: u32 = job.clips.iter().map(|&i| MovieJob::frames_of(&self.world.clips[i], fps)).sum();
        let done: u32 = job.clips[..job.cur.min(job.clips.len())].iter().map(|&i| MovieJob::frames_of(&self.world.clips[i], fps)).sum::<u32>() + job.frame;
        let frac = done as f32 / total.max(1) as f32;
        let el = job.started.elapsed().as_secs_f32();
        let eta = if frac > 0.01 { el / frac - el } else { 0.0 };
        egui::Area::new(egui::Id::new("movie_progress")).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            theme::panel_frame().show(ui, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new("Rendering movie").size(14.0).strong());
                ui.add(egui::ProgressBar::new(frac).show_percentage());
                ui.label(RichText::new(format!("Frame {done} of {total} · about {:.0} min left", eta / 60.0)).size(11.5).color(theme::TEXT_DIM));
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        ctx.request_repaint();
        if cancel || job.cur >= job.clips.len() {
            self.renderer.set_quality(job.viewport_q);
            self.renderer.invalidate_shadows();
            self.last_rendered = None;
            if cancel {
                self.push_log("Movie render cancelled");
                return;
            }
            self.finish_movie(job);
            return;
        }
        self.movie.job = Some(job);
    }

    /// Encode the frames to MP4 in the background if ffmpeg is available.
    fn finish_movie(&mut self, job: MovieJob) {
        let fps = self.movie.fps;
        match find_ffmpeg() {
            Some(ff) => {
                let out = job.out_dir.clone();
                let dirs = job.done_dirs.clone();
                self.push_log(format!("Frames rendered. Encoding {} MP4 file(s) into {} with ffmpeg...", dirs.len(), out.display()));
                std::thread::spawn(move || {
                    for (dir, name) in dirs {
                        let mp4 = out.join(format!("{name}.mp4"));
                        let mut cmd = std::process::Command::new(&ff);
                        cmd.args(["-y", "-framerate", &fps.to_string(), "-i"])
                            .arg(dir.join("frame_%05d.png"))
                            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "17", "-preset", "slow"])
                            .arg(&mp4)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null());
                        hide_console(&mut cmd);
                        match cmd.status() {
                            Ok(s) if s.success() => log::info!("wrote {}", mp4.display()),
                            Ok(s) => log::warn!("ffmpeg failed ({s}) for {}", mp4.display()),
                            Err(e) => log::warn!("ffmpeg: {e}"),
                        }
                    }
                });
            }
            None => self.push_log(format!("Movie frames saved in {} (add ffmpeg to also get MP4 files)", job.out_dir.display())),
        }
    }
}

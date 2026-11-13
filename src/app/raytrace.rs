//! Ray-traced photo rendering: path tracing + AI denoising, stepped a little
//! every UI update so the app stays responsive and shows progress.

use super::*;
use crate::pathtrace::{PathTracer, PtJob, PtOpts};
use std::time::Duration;

pub(super) const LEVELS: [&str; 5] = ["Off", "Fast", "High", "Ultra", "Max"];

pub(super) struct RtState {
    /// 0 = rasterized photos, 1..=4 = path traced (Fast..Max).
    pub level: u32,
    pub denoise: bool,
    pub tracer: Option<PathTracer>,
    pub job: Option<PhotoJob>,
    /// Photo mode: keep refining a ray-traced preview while the camera is still.
    pub live: bool,
    pub live_job: Option<LivePt>,
    pub live_tex: Option<egui::TextureId>,
}

pub(super) struct LivePt {
    key: (FrameParams, u64, (u32, u32)),
    pub preview: crate::preview::LivePreview,
}

impl LivePt {
    pub fn samples(&self) -> u32 {
        self.preview.samples()
    }
    pub fn finished(&self) -> bool {
        self.preview.finished()
    }
    pub fn has_image(&self) -> bool {
        self.preview.has_image
    }
}

impl Default for RtState {
    fn default() -> Self {
        Self { level: 2, denoise: true, tracer: None, job: None, live: false, live_job: None, live_tex: None }
    }
}

impl RtState {
    pub fn opts(&self) -> Option<PtOpts> {
        (self.level > 0).then(|| {
            let mut o = PtOpts::preset(self.level - 1);
            o.denoise = self.denoise;
            o
        })
    }
    /// Lighter settings per frame for movies.
    pub fn movie_opts(&self, level: u32) -> Option<PtOpts> {
        (level > 0).then(|| {
            let mut o = PtOpts::preset(level - 1);
            o.spp = (o.spp / 4).max(8);
            o.denoise = self.denoise;
            o
        })
    }
}

pub(super) struct PhotoJob {
    items: VecDeque<(Camera, PathBuf)>,
    total: usize,
    cur: Option<(PtJob, Targets, FrameParams, PathBuf)>,
    viewport_q: Quality,
    started: Instant,
    opts: PtOpts,
    method: Option<crate::denoise::Method>,
}

pub(super) fn method_label(m: Option<crate::denoise::Method>) -> &'static str {
    match m {
        Some(crate::denoise::Method::Ai) => "AI denoiser",
        Some(crate::denoise::Method::Wavelet) => "wavelet denoiser",
        None => "no denoiser",
    }
}

impl LumenApp {
    pub(super) fn tracer(&mut self) -> &mut PathTracer {
        let device = self.renderer.device.clone();
        let mut t = self.rt.tracer.take().unwrap_or_else(|| PathTracer::new(&device));
        self.apply_texture_budget(Some(&mut t));
        self.rt.tracer.insert(t)
    }

    /// Queue ray-traced renders of these views.
    pub(super) fn start_raytraced(&mut self, items: Vec<(Camera, PathBuf)>) {
        let Some(opts) = self.rt.opts() else { return };
        if items.is_empty() {
            return;
        }
        let viewport_q = self.renderer.quality;
        self.renderer.set_quality(Quality::preset(4));
        self.renderer.invalidate_shadows();
        self.renderer.set_highlight(None, None);
        self.tracer();
        let total = items.len();
        self.rt.job = Some(PhotoJob { items: items.into(), total, cur: None, viewport_q, started: Instant::now(), opts, method: None });
    }

    pub(super) fn raytrace_step(&mut self, ctx: &egui::Context) {
        let Some(mut job) = self.rt.job.take() else { return };
        let (w, h) = self.photo_res;
        if job.cur.is_none() {
            if let Some((cam, path)) = job.items.pop_front() {
                let saved = std::mem::replace(&mut self.world.camera, cam);
                let fp = self.world.frame_params(w as f32 / h as f32, self.time());
                self.world.camera = saved;
                self.renderer.sync_scene(&self.world.scene);
                let targets = self.renderer.create_targets(w, h);
                self.renderer.settle = true;
                let mut tracer = self.rt.tracer.take().unwrap_or_else(|| PathTracer::new(&self.renderer.device));
                self.apply_texture_budget(Some(&mut tracer));
                let started = tracer.try_begin(&mut self.renderer, &self.world.scene, &fp, &targets, job.opts);
                self.rt.tracer = Some(tracer);
                match started {
                    Ok(mut pj) => {
                        pj.target_ms = 80.0;
                        job.cur = Some((pj, targets, fp, path));
                    }
                    Err(e) => {
                        // The card ran out of memory for this size: stop, explain,
                        // and offer the smaller size in the render popup.
                        self.renderer.set_quality(job.viewport_q);
                        self.renderer.invalidate_shadows();
                        self.last_rendered = None;
                        self.push_log(e.to_string());
                        self.render_issue = Some(e);
                        self.render_popup = true;
                        return;
                    }
                }
            }
        }
        let mut cancel = false;
        let mut finished_one = None;
        if let Some((pj, targets, fp, path)) = job.cur.as_mut() {
            let tracer = self.rt.tracer.as_mut().expect("tracer");
            tracer.step(&self.renderer, pj, Duration::from_millis(120));
            if pj.finished() {
                let (img, m) = tracer.finish(&mut self.renderer, &self.world.scene, fp, targets, pj);
                job.method = m;
                finished_one = Some((img.save(&*path).map_err(|e| e.to_string()), path.clone(), pj.started.elapsed().as_secs_f32()));
            }
        }
        if let Some((res, path, secs)) = finished_one {
            if let (Some((pj, ..)), Some(t)) = (job.cur.take(), self.rt.tracer.as_mut()) {
                t.recycle(pj);
            }
            match res {
                Ok(()) => self.push_log(format!(
                    "Ray traced {}x{} ({} samples, {}) to {} in {:.1}s",
                    w,
                    h,
                    job.opts.spp,
                    method_label(job.method),
                    path.display(),
                    secs
                )),
                Err(e) => self.push_log(format!("Could not save {}: {e}", path.display())),
            }
        }
        // Progress card
        let done_imgs = job.total - job.items.len() - job.cur.is_some() as usize;
        let cur_frac = job.cur.as_ref().map_or(0.0, |c| c.0.progress());
        let frac = (done_imgs as f32 + cur_frac) / job.total as f32;
        let el = job.started.elapsed().as_secs_f32();
        let eta = if frac > 0.02 { el / frac - el } else { 0.0 };
        let ai = crate::denoise::ai_available();
        egui::Area::new(egui::Id::new("rt_progress")).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            theme::panel_frame().show(ui, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new("Ray tracing photo").size(14.0).strong());
                ui.add(egui::ProgressBar::new(frac).show_percentage());
                let spp = job.cur.as_ref().map_or(0, |c| c.0.done_spp);
                let mut line = format!("{} samples of {}", spp, job.opts.spp);
                if job.total > 1 {
                    line = format!("Photo {} of {} · {line}", (done_imgs + 1).min(job.total), job.total);
                }
                if eta > 1.0 {
                    line += &format!(" · about {} left", fmt_secs(eta));
                }
                ui.label(RichText::new(line).size(11.5).color(theme::TEXT_DIM));
                let dn = if !job.opts.denoise {
                    "Denoiser off"
                } else if ai {
                    "Finishes with the AI denoiser (Open Image Denoise)"
                } else {
                    "Finishes with the wavelet denoiser (AI denoiser not installed)"
                };
                ui.label(RichText::new(dn).size(11.0).color(theme::TEXT_DIM));
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        ctx.request_repaint();
        if cancel || (job.cur.is_none() && job.items.is_empty()) {
            self.renderer.set_quality(job.viewport_q);
            self.renderer.invalidate_shadows();
            self.last_rendered = None;
            if cancel {
                self.push_log("Ray tracing cancelled");
            } else if job.total > 1 {
                self.push_log(format!("Ray traced {} photos in {}", job.total, fmt_secs(el)));
            }
            return;
        }
        self.rt.job = Some(job);
    }
}

impl LumenApp {
    /// Push the Settings texture budget to the viewport and the path tracer.
    pub(super) fn apply_texture_budget(&mut self, tracer: Option<&mut PathTracer>) {
        let mb = self.settings.texture_budget_mb;
        self.renderer.set_texture_budget(mb.map_or(crate::renderer::DEFAULT_VIEWPORT_TEXTURE_MB, |m| m as u64) << 20);
        if let (Some(t), Some(m)) = (tracer, mb) {
            t.set_texture_budget((m as u64) << 20);
        }
    }

    /// Texture-quality reductions currently in force, for the UI.
    pub(super) fn texture_notes(&self) -> Vec<String> {
        let mut v: Vec<String> = self.renderer.texture_stats().reduction_note().into_iter().collect();
        if let Some(t) = &self.rt.tracer {
            v.extend(t.memory().reduction_note());
        }
        v
    }

    /// Progressive ray-traced preview: restarts when anything changes,
    /// refines while still, and shows a denoised image at 1, 2, 4, 8 …
    /// samples. Returns the texture to show once there is one.
    pub(super) fn live_raytrace(&mut self, fp: &FrameParams, size: (u32, u32), still: bool) -> Option<egui::TextureId> {
        if !self.rt.live || self.rt.job.is_some() || self.movie.job.is_some() {
            self.rt.live_job = None;
            return None;
        }
        let mut k = fp.clone();
        k.time = 0.0;
        let key = (k, self.renderer.content_version(), size);
        if self.rt.live_job.as_ref().is_some_and(|l| l.key != key) {
            // Changed view or scene: start over, reusing the GPU buffers.
            if let (Some(old), Some(t)) = (self.rt.live_job.take(), self.rt.tracer.as_mut()) {
                old.preview.recycle(t);
            }
        }
        if !still {
            return None;
        }
        if self.rt.live_job.is_none() {
            let mut tracer = self.rt.tracer.take().unwrap_or_else(|| PathTracer::new(&self.renderer.device));
            self.apply_texture_budget(Some(&mut tracer));
            let preview = crate::preview::LivePreview::start(&mut tracer, &mut self.renderer, &self.world.scene, fp, size, self.rt.denoise, true);
            self.rt.tracer = Some(tracer);
            self.rt.live_job = Some(LivePt { key, preview });
        }
        let mut live = self.rt.live_job.take()?;
        let tracer = self.rt.tracer.as_mut().expect("tracer");
        // Asynchronous: never waits on read-back or denoising; the previous
        // image stays on screen until the next one is ready.
        if live.preview.frame(tracer, &mut self.renderer, &self.world.scene, Duration::from_millis(12)) {
            let mut er = self.egui_renderer.write();
            match self.rt.live_tex {
                Some(id) => er.update_egui_texture_from_wgpu_texture(&self.renderer.device, &live.preview.targets.output_view, wgpu::FilterMode::Linear, id),
                None => self.rt.live_tex = Some(er.register_native_texture(&self.renderer.device, &live.preview.targets.output_view, wgpu::FilterMode::Linear)),
            }
        }
        if !live.finished() {
            self.ctx.request_repaint();
        }
        let tex = if live.has_image() { self.rt.live_tex } else { None };
        self.rt.live_job = Some(live);
        tex
    }
}

pub(super) fn fmt_secs(s: f32) -> String {
    if s >= 90.0 {
        format!("{:.0} min", s / 60.0)
    } else {
        format!("{:.0} s", s.max(1.0))
    }
}

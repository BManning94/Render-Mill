//! Live ray-traced preview (Photo mode): keeps refining while the camera is
//! still and shows a denoised image at 1, 2, 4, 8 ... samples.
//!
//! Two update strategies, kept side by side so they can be measured on the
//! same GPU (`lumencraft --bench-preview`):
//!
//! * **Blocking** (v0.11): at each update the UI thread waits for the GPU,
//!   reads the image back, denoises it and renders a full raster frame for
//!   post-processing - the UI is frozen for all of that.
//! * **Asynchronous** (v0.12, default): the snapshot is queued behind the
//!   passes already on the GPU, read back through a non-blocking map,
//!   denoised on a worker thread, then only the post-processing passes run.
//!   The UI thread never waits; the previous image stays on screen.

use crate::pathtrace::{PathTracer, PtJob, PtOpts};
use crate::renderer::{FrameParams, Renderer, Targets};
use crate::scene::Scene;
use std::time::{Duration, Instant};

/// Timing of one preview update.
#[derive(Clone, Copy, Debug, Default)]
pub struct UpdateStat {
    pub spp: u32,
    /// UI-thread time spent on this update (the visible stall).
    pub blocked_ms: f32,
    /// Request until the new image was on screen.
    pub latency_ms: f32,
    pub denoise_ms: f32,
}

pub struct LivePreview {
    pub job: PtJob,
    pub targets: Targets,
    pub fp: FrameParams,
    pub shown_spp: u32,
    pub has_image: bool,
    pub asynchronous: bool,
    pub updates: Vec<UpdateStat>,
    pub started: Instant,
    /// Time from start until each new image (for "time to N samples").
    pub image_times: Vec<(u32, f32)>,
}

impl LivePreview {
    /// Begin a preview job for this view at `size`.
    pub fn start(tracer: &mut PathTracer, r: &mut Renderer, scene: &Scene, fp: &FrameParams, size: (u32, u32), denoise: bool, asynchronous: bool) -> Self {
        let targets = r.create_targets(size.0, size.1);
        let opts = PtOpts { spp: 2048, bounces: 8, denoise };
        let mut job = tracer.begin(r, scene, fp, &targets, opts);
        // Short passes: the viewport and UI share the GPU.
        job.target_ms = 20.0;
        Self {
            job,
            targets,
            fp: fp.clone(),
            shown_spp: 0,
            has_image: false,
            asynchronous,
            updates: Vec::new(),
            started: Instant::now(),
            image_times: Vec::new(),
        }
    }

    pub fn finished(&self) -> bool {
        self.job.finished() && self.shown_spp >= self.job.done_spp
    }

    pub fn samples(&self) -> u32 {
        self.shown_spp
    }

    /// One UI frame of work. Returns true when a new image is in `targets`.
    pub fn frame(&mut self, tracer: &mut PathTracer, r: &mut Renderer, scene: &Scene, budget: Duration) -> bool {
        if !self.job.finished() {
            tracer.step(r, &mut self.job, budget);
        }
        let next = if self.shown_spp == 0 { 1 } else { self.shown_spp * 2 };
        let due = self.job.done_spp >= next || (self.job.finished() && self.shown_spp < self.job.done_spp);
        if !self.asynchronous {
            if !due {
                return false;
            }
            let t = Instant::now();
            tracer.compose(r, scene, &self.fp, &self.targets, &mut self.job);
            let ms = t.elapsed().as_secs_f32() * 1000.0;
            self.shown_spp = self.job.done_spp;
            self.updates.push(UpdateStat { spp: self.shown_spp, blocked_ms: ms, latency_ms: ms, denoise_ms: 0.0 });
            return self.new_image();
        }
        let t = Instant::now();
        if due && !tracer.preview_pending(&self.job) {
            tracer.request_preview(r, &mut self.job);
        }
        if let Some(img) = tracer.poll_preview(r, &mut self.job) {
            r.set_hdr_override_packed(self.job.width, self.job.height, &img.hdr);
            let fp = PathTracer::post_params(&self.job, &self.fp);
            r.render_post_only(&fp, &self.targets);
            self.shown_spp = img.spp;
            let blocked = t.elapsed().as_secs_f32() * 1000.0;
            self.updates.push(UpdateStat { spp: img.spp, blocked_ms: blocked, latency_ms: img.latency_ms, denoise_ms: img.denoise_ms });
            return self.new_image();
        }
        false
    }

    fn new_image(&mut self) -> bool {
        self.has_image = true;
        self.image_times.push((self.shown_spp, self.started.elapsed().as_secs_f32()));
        true
    }

    pub fn recycle(self, tracer: &mut PathTracer) {
        tracer.recycle(self.job);
    }
}

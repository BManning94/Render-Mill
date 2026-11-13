//! Library thumbnails rendered by the engine itself (like Lumion's content
//! library): objects, material spheres, ground swatches, weather presets.

use crate::env::{Camera, Environment};
use crate::objects::{self, ObjectKind, PlacedObject};
use crate::renderer::{LightU, Renderer, Targets};
use crate::scene::{Material, MaterialParams, MeshData, Scene, TextureData};
use glam::{Vec2, Vec3};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

pub const THUMB: u32 = 144;

#[derive(Clone)]
pub enum Job {
    Object(ObjectKind),
    Material(MaterialParams),
    Ground(MaterialParams),
    Weather(Environment),
    /// A texture set from the user's library, shown on a sphere.
    TextureSet(crate::texlib::TextureSet),
    /// A 3D model from the object library (loaded in the background first).
    Model(String),
}

type Loaded = (String, crate::texlib::TextureSet, Vec<(String, crate::texlib::MapKind, TextureData)>);

pub struct Thumbs {
    loader_tx: std::sync::mpsc::Sender<(String, crate::texlib::TextureSet)>,
    loaded_rx: std::sync::mpsc::Receiver<Loaded>,
    loading: usize,
    model_tx: std::sync::mpsc::Sender<String>,
    model_rx: std::sync::mpsc::Receiver<String>,
    renderer: Renderer,
    targets: Targets,
    tex: HashMap<String, egui::TextureHandle>,
    queued: HashSet<String>,
    queue: VecDeque<(String, Job)>,
    previews: HashMap<(u64, u32), egui::TextureHandle>,
}

pub fn sphere(radius: f32, rings: u32, segs: u32, uv_tiles: f32) -> MeshData {
    let mut m = MeshData::default();
    for r in 0..=rings {
        let v = r as f32 / rings as f32;
        let th = v * std::f32::consts::PI;
        for s in 0..=segs {
            let u = s as f32 / segs as f32;
            let ph = u * std::f32::consts::TAU;
            let d = Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin());
            m.positions.push(d * radius);
            m.normals.push(d);
            m.uvs.push(Vec2::new(u * uv_tiles * 2.0, v * uv_tiles));
        }
    }
    for r in 0..rings {
        for s in 0..segs {
            let a = r * (segs + 1) + s;
            let b = a + segs + 1;
            m.indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    m
}

fn floor(size: f32, y: f32, material: &str) -> MeshData {
    let h = size / 2.0;
    MeshData {
        name: "floor".into(),
        material: material.into(),
        positions: vec![Vec3::new(-h, y, -h), Vec3::new(h, y, -h), Vec3::new(h, y, h), Vec3::new(-h, y, h)],
        normals: vec![Vec3::Y; 4],
        uvs: vec![Vec2::new(0.0, 0.0), Vec2::new(size, 0.0), Vec2::new(size, size), Vec2::new(0.0, size)],
        indices: vec![0, 2, 1, 0, 3, 2],
    }
}

impl Thumbs {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        let renderer = Renderer::new(device, queue);
        let targets = renderer.create_targets(THUMB, THUMB);
        // Texture-set thumbnails decode big images; do that off the UI thread.
        let (loader_tx, job_rx) = std::sync::mpsc::channel::<(String, crate::texlib::TextureSet)>();
        let (done_tx, loaded_rx) = std::sync::mpsc::channel::<Loaded>();
        let job_rx = Arc::new(std::sync::Mutex::new(job_rx));
        for _ in 0..3 {
            let rx = job_rx.clone();
            let tx = done_tx.clone();
            std::thread::spawn(move || loop {
                let job = { rx.lock().unwrap().recv() };
                let Ok((key, set)) = job else { return };
                use crate::texlib::{load_map, MapKind};
                let mut maps = Vec::new();
                for (spec, kind) in [(&set.color, MapKind::Color), (&set.normal, MapKind::Normal), (&set.roughness, MapKind::Roughness)] {
                    if let Some(spec) = spec {
                        if let Some(t) = load_map(spec, kind) {
                            let img = image::RgbaImage::from_raw(t.width, t.height, t.rgba.to_vec()).unwrap_or_default();
                            let small = image::imageops::thumbnail(&img, 256, 256);
                            let (w, h) = small.dimensions();
                            maps.push((spec.clone(), kind, TextureData::new(w, h, small.into_raw())));
                        }
                    }
                }
                if tx.send((key, set, maps)).is_err() {
                    return;
                }
            });
        }
        let (model_tx, model_rx) = std::sync::mpsc::channel();
        Self {
            loader_tx,
            loaded_rx,
            loading: 0,
            model_tx,
            model_rx,
            renderer,
            targets,
            tex: HashMap::new(),
            queued: HashSet::new(),
            queue: VecDeque::new(),
            previews: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&egui::TextureHandle> {
        self.tex.get(key)
    }

    /// Ask for a thumbnail; it appears a frame or two later.
    pub fn request(&mut self, key: &str, job: Job) {
        if !self.tex.contains_key(key) && self.queued.insert(key.to_string()) {
            if let Job::TextureSet(set) = &job {
                if self.loader_tx.send((key.to_string(), set.clone())).is_ok() {
                    self.loading += 1;
                }
                return;
            }
            if let Job::Model(m) = &job {
                // Load the file off the UI thread; render when it's ready.
                let (m, tx) = (m.clone(), self.model_tx.clone());
                self.loading += 1;
                std::thread::spawn(move || {
                    let _ = objects::model_asset(&m);
                    let _ = tx.send(m);
                });
                return;
            }
            self.queue.push_back((key.to_string(), job));
        }
    }

    pub fn pending(&self) -> bool {
        !self.queue.is_empty() || self.loading > 0
    }

    /// Render up to `max` queued thumbnails.
    pub fn step(&mut self, ctx: &egui::Context, max: usize) {
        while let Ok(m) = self.model_rx.try_recv() {
            self.loading -= 1;
            self.queue.push_back((format!("model:{m}"), Job::Model(m)));
        }
        for _ in 0..max {
            let Ok((key, set, maps)) = self.loaded_rx.try_recv() else { break };
            self.loading -= 1;
            let img = self.render_set(&set, maps);
            let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
            self.tex.insert(key.clone(), ctx.load_texture(format!("thumb:{key}"), ci, egui::TextureOptions::LINEAR));
            self.queued.remove(&key);
        }
        for _ in 0..max {
            let Some((key, job)) = self.queue.pop_front() else { return };
            let img = self.render(&job);
            let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
            self.tex.insert(key.clone(), ctx.load_texture(format!("thumb:{key}"), ci, egui::TextureOptions::LINEAR));
            self.queued.remove(&key);
        }
    }

    fn render(&mut self, job: &Job) -> image::RgbaImage {
        let mut scene = Scene::default();
        objects::register_library_materials(&mut scene);
        let floor_mat = "Thumb floor";
        scene.set_material(Material {
            name: floor_mat.into(),
            params: MaterialParams { base_color: [0.5, 0.47, 0.43, 1.0], roughness: 0.85, ..Default::default() },
            texture: None,
            normal_texture: None,
            roughness_texture: None,
        });
        let mut env =
            Environment { vignette: 0.0, fog: 0.0, ground_enabled: false, sun_height: 40.0, sun_heading: 230.0, cloudiness: 0.25, ..Default::default() };
        let mut cam = Camera { fov: 34.0, ..Default::default() };
        let mut lights: Vec<LightU> = Vec::new();
        match job {
            Job::Object(kind) => {
                let o = PlacedObject::new(*kind, [0.0, 0.0, 0.0], 0.7, 1.0, 11);
                if kind.is_light() {
                    // Lights have no geometry: show a glowing bulb lighting a floor at dusk.
                    scene.set_material(Material {
                        name: "Thumb bulb".into(),
                        params: MaterialParams { kind: crate::scene::MaterialKind::Emissive, emissive: o.color, emissive_strength: 5.0, ..Default::default() },
                        texture: None,
                        normal_texture: None,
                        roughness_texture: None,
                    });
                    let mut b = sphere(0.18, 12, 16, 1.0);
                    b.material = "Thumb bulb".into();
                    for p in &mut b.positions {
                        *p += Vec3::Y * 1.2;
                    }
                    scene.upsert_mesh("bulb".into(), b);
                    env = Environment { sun_height: -12.0, exposure: 1.4, bloom: 0.6, ..env };
                    let mut l = o.clone();
                    l.position = [0.0, 1.2, 0.0];
                    l.range = 8.0;
                    l.intensity = 12.0;
                    l.cone = 90.0;
                    lights.extend(l.light());
                } else {
                    for (i, m) in objects::build(&o).into_iter().enumerate() {
                        scene.upsert_mesh(format!("o{i}"), m);
                    }
                    lights.extend(o.light());
                }
                if kind.is_light() {
                    scene.upsert_mesh("floor".into(), floor(12.0, 0.0, floor_mat));
                    cam.position = [2.6, 1.7, 3.4];
                    cam.look_at(Vec3::new(0.0, 0.6, 0.0));
                } else {
                    let (lo, hi) = scene.model_bounds().unwrap_or((Vec3::splat(-1.0), Vec3::splat(1.0)));
                    let size = (hi - lo).max_element().max(1.0);
                    scene.upsert_mesh("floor".into(), floor(size * 6.0, 0.0, floor_mat));
                    let c = (lo + hi) * 0.5;
                    let dir = Vec3::new(0.65, 0.35, 0.75).normalize();
                    cam.position = (c + dir * (hi - lo).length() * 1.45).to_array();
                    cam.look_at(c);
                }
            }
            Job::Material(p) => {
                scene.set_material(Material { name: "m".into(), params: p.clone(), texture: None, normal_texture: None, roughness_texture: None });
                let mut s = sphere(1.0, 32, 48, 2.0);
                s.material = "m".into();
                scene.upsert_mesh("sphere".into(), s);
                scene.upsert_mesh("floor".into(), floor(12.0, -1.0, floor_mat));
                cam.position = [0.0, 0.9, 3.5];
                cam.look_at(Vec3::new(0.0, -0.2, 0.0));
            }
            Job::TextureSet(set) => return self.render_set(set, Vec::new()),
            Job::Model(key) => {
                let mut o = PlacedObject::new(ObjectKind::Model, [0.0, 0.0, 0.0], 0.6, 1.0, 1);
                o.model = Some(key.clone());
                objects::register_model_materials(&mut scene, &o);
                for (i, m) in objects::build(&o).into_iter().enumerate() {
                    scene.upsert_mesh(format!("o{i}"), m);
                }
                let (lo, hi) = scene.model_bounds().unwrap_or((Vec3::splat(-1.0), Vec3::splat(1.0)));
                let size = (hi - lo).max_element().max(0.2);
                scene.upsert_mesh("floor".into(), floor(size * 6.0, 0.0, floor_mat));
                let c = (lo + hi) * 0.5;
                let dir = Vec3::new(0.65, 0.35, 0.75).normalize();
                cam.position = (c + dir * (hi - lo).length() * 1.45).to_array();
                cam.look_at(c);
            }
            Job::Ground(p) => {
                scene.set_material(Material { name: "g".into(), params: p.clone(), texture: None, normal_texture: None, roughness_texture: None });
                scene.upsert_mesh("floor".into(), floor(40.0, 0.0, "g"));
                cam.position = [0.0, 3.0, 6.0];
                cam.look_at(Vec3::new(0.0, 0.0, -2.0));
            }
            Job::Weather(e) => {
                env = Environment { vignette: 0.0, ground_enabled: false, ..e.clone() };
                scene.set_material(Material {
                    name: "w".into(),
                    params: MaterialParams { base_color: [0.8, 0.8, 0.78, 1.0], roughness: 0.5, ..Default::default() },
                    texture: None,
                    normal_texture: None,
                    roughness_texture: None,
                });
                let mut s = sphere(0.8, 24, 32, 1.0);
                s.material = "w".into();
                for p in &mut s.positions {
                    *p += Vec3::new(0.6, 0.8, 0.0);
                }
                scene.upsert_mesh("sphere".into(), s);
                scene.upsert_mesh("floor".into(), floor(60.0, 0.0, crate::objects::MAT_GROUND));
                cam.position = [-1.5, 1.4, 4.2];
                cam.look_at(Vec3::new(0.4, 1.6, 0.0));
                cam.fov = 60.0;
            }
        }
        let mut fp = env.frame_params(&cam, 1.0, 8.0);
        fp.lights = lights;
        // Every thumbnail is a fresh mini scene whose revision counters restart.
        self.renderer.reset_cache();
        self.renderer.sync_scene(&scene);
        self.renderer.render(&scene, &fp, &self.targets);
        self.renderer.read_output(&self.targets)
    }

    /// A texture set on a sphere, with its maps already decoded (small).
    fn render_set(&mut self, set: &crate::texlib::TextureSet, maps: Vec<(String, crate::texlib::MapKind, TextureData)>) -> image::RgbaImage {
        let mut scene = Scene::default();
        objects::register_library_materials(&mut scene);
        let floor_mat = "Thumb floor";
        scene.set_material(Material {
            name: floor_mat.into(),
            params: MaterialParams { base_color: [0.5, 0.47, 0.43, 1.0], roughness: 0.85, ..Default::default() },
            texture: None,
            normal_texture: None,
            roughness_texture: None,
        });
        for (spec, kind, t) in maps {
            scene.user_textures.insert(crate::scene::user_texture_key(&spec, kind), t);
        }
        let mut p = crate::texlib::apply(set, &MaterialParams::default());
        p.uv_scale = 1.5;
        scene.set_material(Material { name: "m".into(), params: p, texture: None, normal_texture: None, roughness_texture: None });
        let mut s = sphere(1.0, 32, 48, 1.0);
        s.material = "m".into();
        scene.upsert_mesh("sphere".into(), s);
        scene.upsert_mesh("floor".into(), floor(12.0, -1.0, floor_mat));
        let env = Environment { vignette: 0.0, fog: 0.0, ground_enabled: false, sun_height: 40.0, sun_heading: 230.0, cloudiness: 0.25, ..Default::default() };
        let mut cam = Camera { fov: 34.0, ..Default::default() };
        cam.position = [0.0, 0.9, 3.5];
        cam.look_at(Vec3::new(0.0, -0.2, 0.0));
        let fp = env.frame_params(&cam, 1.0, 8.0);
        self.renderer.reset_cache();
        self.renderer.sync_scene(&scene);
        self.renderer.render(&scene, &fp, &self.targets);
        self.renderer.read_output(&self.targets)
    }

    /// Small preview of an imported/assigned texture for the material editor.
    pub fn texture_preview(&mut self, ctx: &egui::Context, t: &TextureData) -> egui::TextureHandle {
        self.texture_preview_sized(ctx, t, 72)
    }

    pub fn texture_preview_sized(&mut self, ctx: &egui::Context, t: &TextureData, size: u32) -> egui::TextureHandle {
        if let Some(h) = self.previews.get(&(t.key, size)) {
            return h.clone();
        }
        let img = image::RgbaImage::from_raw(t.width, t.height, t.rgba.to_vec()).unwrap_or_default();
        let small = image::imageops::thumbnail(&img, size, size);
        let ci = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw());
        let h = ctx.load_texture(format!("preview:{}:{size}", t.key), ci, egui::TextureOptions::LINEAR);
        self.previews.insert((t.key, size), h.clone());
        h
    }
}

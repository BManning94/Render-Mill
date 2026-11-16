//! Project file (.lcproj, JSON) and the "world" that ties scene + settings together.

use crate::env::{Camera, Environment};
use crate::import::{self, ImportSettings};
use crate::objects::{self, PlacedObject};
use crate::scene::{MaterialParams, Scene};
use crate::sync::SyncEvent;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedView {
    pub name: String,
    pub camera: Camera,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    /// Model file (or folder) that is live-synced.
    pub source: Option<PathBuf>,
    pub import: Option<ImportSettings>,
    pub overrides: BTreeMap<String, MaterialParams>,
    pub objects: Vec<PlacedObject>,
    pub environment: Environment,
    pub camera: Camera,
    pub views: Vec<SavedView>,
    /// Textures the user imported into this project's library.
    #[serde(default)]
    pub texture_library: Vec<crate::texlib::TextureSet>,
    /// Movie clips (camera paths).
    #[serde(default)]
    pub clips: Vec<crate::env::Clip>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: 2,
            source: None,
            import: None,
            overrides: BTreeMap::new(),
            objects: Vec::new(),
            environment: Environment::default(),
            camera: Camera::default(),
            views: Vec::new(),
            clips: Vec::new(),
            texture_library: Vec::new(),
        }
    }
}

impl Project {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        let mut p: Project = serde_json::from_str(&s)?;
        if p.version < 2 {
            // Before 0.10 library/folder textures assumed the model's UVs were
            // in metres; they now use real-world mapping, which is what was meant.
            for m in p.overrides.values_mut() {
                if m.color_map.is_some() {
                    m.box_mapping = true;
                }
            }
            p.version = 2;
        }
        Ok(p)
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Live state: the scene plus everything the user has set up around it.
/// What came across from a Lumion scene.
#[derive(Debug, Default)]
pub struct LumionReport {
    pub models: usize,
    pub instances: usize,
    pub objects: usize,
    pub skipped: Vec<(String, usize)>,
    pub views: usize,
    /// (model material, Lumion library material name) to match in the library.
    pub library_hints: Vec<(String, String)>,
}

impl LumionReport {
    pub fn summary(&self) -> String {
        let skipped: usize = self.skipped.iter().map(|s| s.1).sum();
        let mut s = format!(
            "Lumion scene: {} model(s) placed {} time(s), {} Lumion object(s) replaced with Render Mill's, {} camera view(s)",
            self.models, self.instances, self.objects, self.views
        );
        if skipped > 0 {
            s += &format!(", {skipped} left out (grass clumps, landscape paint, effects and other items Render Mill draws its own way)");
        }
        s
    }
}

pub struct World {
    pub scene: Scene,
    pub objects: Vec<PlacedObject>,
    pub env: Environment,
    pub camera: Camera,
    pub views: Vec<SavedView>,
    pub source: Option<PathBuf>,
    pub import: Option<ImportSettings>,
    pub last_sync: Option<String>,
    pub ground_size: f32,
    pub ground_on: bool,
    /// Textures the last synced model referenced but we couldn't find.
    pub missing_textures: Vec<String>,
    pub texture_library: Vec<crate::texlib::TextureSet>,
    pub clips: Vec<crate::env::Clip>,
    grass_key: u64,
    landscape_key: u64,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// (Re)grow 3D grass around the model when the model, ground or settings change.
    fn update_grass(&mut self) {
        let ground = self.scene.effective(objects::MAT_GROUND);
        let c = ground.base_color;
        let textured_grass = ground.color_map.as_deref().is_some_and(|m| {
            let l = m.to_lowercase();
            l.contains("grass") || l.contains("lawn") || l.contains("moss")
        });
        let grassy = ground.kind == crate::scene::MaterialKind::Ground && ((c[1] > c[0] * 1.2 && c[1] > c[2] * 1.2) || textured_grass);
        let want = self.env.grass && self.env.ground_enabled && grassy;
        let mut key: u64 = if want { 1 } else { 0 };
        if want {
            for (k, m) in &self.scene.meshes {
                if !k.starts_with("env:") && !k.starts_with("obj:") {
                    key = key.wrapping_mul(1_000_003).wrapping_add(m.rev).wrapping_add(k.len() as u64);
                }
            }
            key = key.wrapping_mul(31).wrapping_add((self.env.grass_height * 1000.0) as u64);
        }
        if key == self.grass_key {
            return;
        }
        self.grass_key = key;
        self.scene.remove_prefix("env:grass:");
        if !want {
            return;
        }
        let model: Vec<&crate::scene::SceneMesh> =
            self.scene.meshes.iter().filter(|(k, _)| !k.starts_with("env:") && !k.starts_with("obj:")).map(|(_, m)| m).collect();
        let bounds = model.iter().map(|m| m.bounds).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        let (center, radius) = match bounds {
            Some((lo, hi)) => ((lo + hi) * 0.5, ((hi - lo).length() * 0.5 * 1.8).clamp(25.0, 75.0)),
            None => (glam::Vec3::ZERO, 40.0),
        };
        // Occupancy grid (0.5 m) of everything the model has near the ground.
        let cell = 0.5f32;
        let n = (radius * 2.0 / cell).ceil() as i32 + 2;
        let origin = glam::Vec2::new(center.x - radius - cell, center.z - radius - cell);
        let mut occ = vec![false; (n * n) as usize];
        for m in &model {
            if m.bounds.0.y > 1.5 {
                continue;
            }
            let p = &m.data.positions;
            for t in m.data.indices.chunks_exact(3) {
                let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
                if a.y.min(b.y).min(c.y) > 1.0 {
                    continue;
                }
                let lo = a.min(b).min(c);
                let hi = a.max(b).max(c);
                let (i0, i1) = (((lo.x - origin.x) / cell).floor() as i32, ((hi.x - origin.x) / cell).floor() as i32);
                let (j0, j1) = (((lo.z - origin.y) / cell).floor() as i32, ((hi.z - origin.y) / cell).floor() as i32);
                if i1 - i0 > 400 || j1 - j0 > 400 {
                    continue; // huge triangles (terrain): don't block everything
                }
                for j in j0.max(0)..=j1.min(n - 1) {
                    for i in i0.max(0)..=i1.min(n - 1) {
                        occ[(j * n + i) as usize] = true;
                    }
                }
            }
        }
        let blocked = |x: f32, z: f32| {
            let i = ((x - origin.x) / cell).floor() as i32;
            let j = ((z - origin.y) / cell).floor() as i32;
            i >= 0 && j >= 0 && i < n && j < n && occ[(j * n + i) as usize]
        };
        let h = self.env.grass_height;
        let mut tiles = objects::grass_meshes(center, radius, h, &blocked);
        if let Some(l) = &self.env.landscape {
            // Follow the terrain.
            for (_, m) in &mut tiles {
                for p in &mut m.positions {
                    p.y += l.height(p.x, p.z);
                }
            }
        }
        let _ = textured_grass;
        if let Some(g) = self.scene.imported_materials.get_mut(objects::MAT_GRASS) {
            // Match the blades to the ground photo's average colour so the lawn
            // blends into the landscape instead of reading as a darker patch.
            let avg =
                ground.color_map.as_deref().and_then(|m| self.scene.user_textures.get(&crate::scene::user_texture_key(m, crate::texlib::MapKind::Color))).map(
                    |t| {
                        let px = t.rgba.len() / 4;
                        let step = (px / 4096).max(1);
                        let mut acc = [0.0f32; 3];
                        let mut n = 0.0;
                        for i in (0..px).step_by(step) {
                            for (k, a) in acc.iter_mut().enumerate() {
                                *a += (t.rgba[i * 4 + k] as f32 / 255.0).powf(2.2);
                            }
                            n += 1.0;
                        }
                        [acc[0] / n, acc[1] / n, acc[2] / n]
                    },
                );
            g.params.base_color = match (textured_grass, avg) {
                (true, Some(a)) => [a[0] * 0.7, a[1] * 0.78, a[2] * 0.55, 1.0],
                (true, None) => [0.075, 0.13, 0.03, 1.0],
                _ => [c[0] * 1.15, c[1] * 1.2, c[2] * 1.05, 1.0],
            };
            self.scene.material_rev += 1;
        }
        for (id, m) in tiles {
            self.scene.upsert_mesh(id, m);
        }
    }

    pub fn new() -> Self {
        let mut w = Self {
            scene: Scene::default(),
            objects: Vec::new(),
            env: Environment::default(),
            camera: Camera::default(),
            views: Vec::new(),
            source: None,
            import: None,
            last_sync: None,
            ground_size: 0.0,
            ground_on: false,
            missing_textures: Vec::new(),
            texture_library: Vec::new(),
            clips: Vec::new(),
            grass_key: 0,
            landscape_key: 0,
        };
        objects::register_library_materials(&mut w.scene);
        w.set_ground_from_library("Grass001", 2.5);
        w.update_ground();
        w
    }

    pub fn from_project(p: Project) -> Self {
        let mut w = Self::new();
        w.scene.overrides = p.overrides;
        w.objects = p.objects;
        w.env = p.environment;
        w.camera = p.camera;
        w.views = p.views;
        w.texture_library = p.texture_library;
        w.clips = p.clips;
        w.source = p.source;
        w.import = p.import;
        objects::sync_objects(&mut w.scene, &w.objects);
        w.update_ground();
        w.load_user_textures();
        w
    }

    pub fn to_project(&self) -> Project {
        Project {
            version: 2,
            source: self.source.clone(),
            import: self.import.clone(),
            overrides: self.scene.overrides.clone(),
            objects: self.objects.clone(),
            environment: self.env.clone(),
            camera: self.camera.clone(),
            views: self.views.clone(),
            texture_library: self.texture_library.clone(),
            clips: self.clips.clone(),
        }
    }

    /// Start a new project from a landscape template (see `landscape::TEMPLATES`).
    pub fn apply_template(&mut self, id: &str) -> bool {
        let Some((l, env)) = crate::landscape::template(id) else { return false };
        self.env = env;
        let ground = l.as_ref().map_or("Grass001".to_string(), |l| l.ground.clone());
        self.env.landscape = l;
        self.set_ground_from_library(&ground, 2.5);
        self.camera = crate::landscape::start_camera(self.env.landscape.as_ref());
        self.update_ground();
        true
    }

    /// Terrain height under (x, z): the landscape's, or 0 on flat ground.
    pub fn terrain_height(&self, x: f32, z: f32) -> f32 {
        self.env.landscape.as_ref().filter(|_| self.env.ground_enabled).map_or(0.0, |l| l.height(x, z))
    }

    pub fn update_ground(&mut self) {
        // Landscape template: terrain tiles, water and vegetation.
        let land = self.env.landscape.as_ref().filter(|_| self.env.ground_enabled);
        let lkey = land.map_or(0, |l| l.key());
        if lkey != self.landscape_key {
            self.landscape_key = lkey;
            crate::landscape::apply(&mut self.scene, land);
            self.grass_key = u64::MAX; // regrow on the new terrain
            self.ground_size = -1.0; // re-evaluate the flat ground below
        }
        let want_backdrop = self.env.backdrop && self.env.ground_enabled;
        let has_backdrop = self.scene.meshes.contains_key("env:mountains");
        if want_backdrop != has_backdrop {
            if want_backdrop {
                objects::register_library_materials(&mut self.scene);
                self.scene.upsert_mesh("env:mountains".into(), objects::mountains_mesh(1800.0, 7500.0));
            } else {
                self.scene.remove_mesh("env:mountains");
            }
        }
        self.update_grass();
        if self.env.ground_enabled == self.ground_on && self.env.ground_size == self.ground_size {
            return;
        }
        self.ground_on = self.env.ground_enabled;
        self.ground_size = self.env.ground_size;
        // The terrain replaces the flat ground plane.
        if self.ground_on && self.env.landscape.is_none() {
            self.scene.upsert_mesh("env:ground".into(), objects::ground_mesh(self.ground_size));
        } else {
            self.scene.remove_mesh("env:ground");
        }
    }

    /// All local lights in the scene (placed lights + street lamps).
    pub fn lights(&self) -> Vec<crate::renderer::LightU> {
        self.objects.iter().filter_map(|o| o.light()).collect()
    }

    /// Renderer parameters for the current camera, environment and lights.
    pub fn frame_params(&self, aspect: f32, time: f32) -> crate::renderer::FrameParams {
        let mut fp = self.env.frame_params(&self.camera, aspect, time);
        fp.lights = self.lights();
        fp
    }

    /// Load images assigned in the material editor (after opening a project).
    pub fn load_user_textures(&mut self) {
        use crate::texlib::MapKind;
        let paths: Vec<(String, MapKind)> = self
            .scene
            .overrides
            .values()
            .flat_map(|p| {
                [
                    p.color_map.clone().map(|c| (c, MapKind::Color)),
                    p.normal_map.clone().map(|n| (n, MapKind::Normal)),
                    p.roughness_map.clone().map(|r| (r, MapKind::Roughness)),
                ]
            })
            .flatten()
            .collect();
        for (path, kind) in paths {
            self.load_user_texture(&path, kind);
        }
    }

    /// Load (once) an image for a material map slot.
    pub fn load_user_texture(&mut self, spec: &str, kind: crate::texlib::MapKind) -> bool {
        let key = crate::scene::user_texture_key(spec, kind);
        if self.scene.user_textures.contains_key(&key) {
            return true;
        }
        match crate::texlib::load_map(spec, kind) {
            Some(t) => {
                self.scene.user_textures.insert(key, t);
                self.scene.material_rev += 1;
                true
            }
            None => false,
        }
    }

    /// Load every map of a library set; false if its colour/normal image is unreadable.
    /// Find a material of the built-in CC0 library by its id (e.g. "Grass001").
    pub fn builtin_set(id: &str) -> Option<crate::texlib::TextureSet> {
        let root = crate::library::builtin_dir()?;
        for cat in std::fs::read_dir(&root).ok()?.flatten() {
            let d = cat.path().join(id);
            if d.is_dir() {
                let files: Vec<PathBuf> = std::fs::read_dir(&d).ok()?.flatten().map(|e| e.path()).collect();
                return crate::texlib::group(&files).into_iter().next();
            }
        }
        None
    }

    /// Give the landscape a real photo texture from the built-in library
    /// (with tiling break-up), like Lumion's landscape materials.
    pub fn set_ground_from_library(&mut self, id: &str, size_m: f32) -> bool {
        let Some(mut set) = Self::builtin_set(id) else { return false };
        set.size_m = size_m;
        if !self.load_texture_set(&set) {
            return false;
        }
        let old = crate::scene::MaterialParams { kind: crate::scene::MaterialKind::Ground, roughness: 0.95, ..Default::default() };
        let mut p = crate::texlib::apply(&set, &old);
        p.kind = crate::scene::MaterialKind::Ground;
        p.variation = 0.55;
        p.anti_tiling = true;
        p.reflectivity = 0.3;
        self.scene.overrides.insert(objects::MAT_GROUND.into(), p);
        true
    }

    pub fn load_texture_set(&mut self, set: &crate::texlib::TextureSet) -> bool {
        use crate::texlib::MapKind;
        let mut ok = true;
        if let Some(c) = &set.color {
            ok &= self.load_user_texture(c, MapKind::Color);
        }
        if let Some(n) = &set.normal {
            ok &= self.load_user_texture(n, MapKind::Normal);
        }
        if let Some(r) = &set.roughness {
            self.load_user_texture(r, MapKind::Roughness);
        }
        ok
    }

    pub fn rebuild_objects(&mut self) {
        objects::sync_objects(&mut self.scene, &self.objects);
    }

    /// Load a model synchronously (headless mode / tests).
    /// Open a Lumion scene (.lsf/.lqf): the imported models where they stood,
    /// Lumion's library objects swapped for Render Mill's, camera views and
    /// the sun. Returns what came across, for the activity log.
    pub fn open_lumion_scene(&mut self, path: &Path) -> anyhow::Result<LumionReport> {
        let scene = crate::lumion::read_scene(path)?;
        let settings = self.import.clone().unwrap_or_else(|| ImportSettings::default_for(path));
        let mut report = LumionReport {
            models: scene.models.len(),
            instances: scene.instances.len(),
            objects: scene.objects.len(),
            skipped: scene.skipped.clone(),
            views: scene.cameras.len(),
            library_hints: scene.models.iter().flat_map(|m| m.library_hints.clone()).collect(),
        };
        if !scene.instances.is_empty() {
            let (parts, mats) = crate::lumion::scene_meshes(&scene);
            let hints = import::FileHints { up: Some(import::UpAxis::Y), scale_to_m: Some(1.0) };
            let model = import::finish(parts, mats, hints, &settings, Vec::new());
            self.import = Some(settings);
            self.source = Some(path.to_path_buf());
            self.apply(SyncEvent::FileLoaded { path: path.to_path_buf(), model, millis: 0 });
        }
        self.objects.extend(scene.objects);
        self.rebuild_objects();
        for c in &scene.cameras {
            self.views.push(SavedView { name: c.name.clone(), camera: c.camera.clone() });
        }
        // Start on the first saved photo view (else the first clip).
        if let Some(c) = scene.cameras.iter().find(|c| c.name.starts_with("Lumion view")).or(scene.cameras.first()) {
            self.camera = c.camera.clone();
        }
        if let Some((h, p)) = scene.sun {
            self.env.sun_heading = h;
            self.env.sun_height = p;
        }
        report.models = report.models.max(scene.models.len());
        Ok(report)
    }

    pub fn load_model_now(&mut self, path: &Path) -> anyhow::Result<()> {
        let settings = self.import.clone().unwrap_or_else(|| ImportSettings::default_for(path));
        let model = import::load(path, &settings)?;
        self.source = Some(path.to_path_buf());
        self.apply(SyncEvent::FileLoaded { path: path.to_path_buf(), model, millis: 0 });
        Ok(())
    }

    /// Apply a live-sync event. Returns a camera request if the host sent one.
    pub fn apply(&mut self, ev: SyncEvent) -> Option<String> {
        match ev {
            SyncEvent::FileLoaded { path, model, millis } => {
                let mut s = self.import.clone().unwrap_or_else(|| ImportSettings::default_for(&path));
                if s.recenter && s.offset.is_none() {
                    s.offset = Some(model.offset.to_array());
                }
                self.import = Some(s);
                self.missing_textures = model.missing_textures.clone();
                for m in model.materials {
                    self.scene.set_material(m);
                }
                // Incremental: drop chunks that disappeared, re-upload only changed ones.
                let new_ids: std::collections::HashSet<&String> = model.meshes.iter().map(|(id, _)| id).collect();
                let stale: Vec<String> = self.scene.meshes.keys().filter(|k| k.starts_with("file:") && !new_ids.contains(k)).cloned().collect();
                for k in &stale {
                    self.scene.remove_mesh(k);
                }
                let total = model.meshes.len();
                let mut changed = 0;
                for (id, m) in model.meshes {
                    if self.scene.upsert_mesh(id, m) {
                        changed += 1;
                    }
                }
                let msg = format!(
                    "Synced {} in {} ms: {} of {} chunks changed, {} removed ({} triangles)",
                    path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
                    millis,
                    changed,
                    total,
                    stale.len(),
                    self.scene.triangle_count(),
                );
                let msg = if self.missing_textures.is_empty() {
                    msg
                } else {
                    format!(
                        "{msg}. {} texture(s) not found: {} (add their folder in LiveSync > Texture folders)",
                        self.missing_textures.len(),
                        self.missing_textures.iter().take(4).cloned().collect::<Vec<_>>().join(", ")
                    )
                };
                self.last_sync = Some(msg.clone());
                Some(msg)
            }
            SyncEvent::FileError(e) => Some(format!("Sync error: {e}")),
            SyncEvent::NetClear { client } => {
                self.scene.remove_prefix("net:");
                Some(format!("LiveSync: {client} started a full sync"))
            }
            SyncEvent::NetMesh { id, data } => {
                self.scene.upsert_mesh(id, data);
                None
            }
            SyncEvent::NetRemove { id } => {
                self.scene.remove_mesh(&id);
                None
            }
            SyncEvent::NetMaterial(m) => {
                self.scene.set_material(m);
                None
            }
            SyncEvent::NetCamera { position, target, fov } => {
                self.camera.position = position.to_array();
                self.camera.look_at(target);
                if let Some(f) = fov {
                    self.camera.fov = f;
                }
                None
            }
            SyncEvent::Status(s) => Some(s),
        }
    }
}

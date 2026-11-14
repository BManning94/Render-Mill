//! The desktop application, laid out like Lumion 2025: a thin title bar, a
//! narrow icon sidebar, a full-screen viewport with a compact Build dock in the
//! bottom-left, a LiveSync card top-right, and a separate Photo mode with a
//! framed preview, effect list and photo-set filmstrip.

mod build;
mod chrome;
mod materials;
mod movie;
pub use movie::find_ffmpeg;
mod import_dialog;
mod photo;
mod photo_match;
mod raytrace;
mod start;

use crate::env::Camera;
use crate::import::{ImportSettings, Units, UpAxis};
use crate::objects::{ObjectKind, PlacedObject};
use crate::project::{Project, SavedView, World};
use crate::protocol::DEFAULT_PORT;
use crate::renderer::{FrameParams, Quality, Renderer, Targets};
use crate::scene::{presets, MaterialKind, MaterialParams};
use crate::sync::{self, FileSync, NetServer, SyncEvent};
use crate::texlib::TextureSet;
use crate::ui::theme;
use crate::ui::thumbs::{Job, Thumbs};
use egui::{vec2, Align2, Color32, RichText};
use glam::Vec3;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub(crate) const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff", "webp", "dds", "gif"];
const PHOTO_SLOTS: usize = 10;
pub(crate) const TITLE_H: f32 = 34.0;
pub(crate) const SIDEBAR_W: f32 = 40.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Build,
    Photo,
    Movie,
}

/// The four tabs on top of the Build dock (Lumion: home, materials, landscape, weather).
#[derive(Clone, Copy, PartialEq, Eq)]
enum BuildTab {
    Home,
    Materials,
    Landscape,
    Weather,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Place,
    Select,
    Delete,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlaceMode {
    Single,
    Paint,
    Mass,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditMode {
    Move,
    Height,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeleteMode {
    Single,
    Area,
}

/// Content-library categories (the 2×4 grid in the Build dock).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ObjCat {
    Nature,
    People,
    Transport,
    Interior,
    Exterior,
    Lights,
    Favourites,
    All,
}

/// Panels that slide out next to the sidebar or under the LiveSync card.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SidePanel {
    File,
    Log,
    Settings,
    Help,
    LiveSync,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MatTab {
    Library,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MatSection {
    Advanced,
    Placement,
    Weathering,
    Imported,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MatLib {
    Materials,
    Folders,
    Custom,
}

/// An open material-editor session: what was there before, so ✕ can revert.
struct MatSession {
    name: String,
    before: Option<MaterialParams>,
    tab: MatTab,
    section: MatSection,
    show_more: bool,
    lib: MatLib,
    lib_cat: usize,
}

/// One entry in the object library: a built-in procedural object or a 3D model.
#[derive(Clone, PartialEq, Debug)]
enum LibItem {
    Kind(ObjectKind),
    Model(String),
}

impl LibItem {
    fn fav_key(&self) -> String {
        match self {
            LibItem::Kind(k) => format!("kind:{k:?}"),
            LibItem::Model(m) => m.clone(),
        }
    }
}

#[derive(Clone)]
struct Snapshot {
    objects: Vec<PlacedObject>,
    overrides: BTreeMap<String, MaterialParams>,
}

struct DragState {
    grab: Vec3,
    anchor: usize,
}

pub struct LumenApp {
    world: World,
    renderer: Renderer,
    targets: Option<Targets>,
    tex_id: Option<egui::TextureId>,
    egui_renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    thumbs: Thumbs,
    slot_targets: Targets,
    slot_thumbs: HashMap<usize, egui::TextureHandle>,
    model_thumb: Option<(u64, egui::TextureHandle)>,
    events_tx: Sender<SyncEvent>,
    events_rx: Receiver<SyncEvent>,
    file_sync: Option<FileSync>,
    import_settings: Arc<Mutex<ImportSettings>>,
    server: Option<NetServer>,
    server_lan: bool,
    server_port: u16,
    follow_host_camera: bool,
    pair_token: String,
    // UI state
    mode: Mode,
    build_tab: BuildTab,
    tool: Tool,
    place_mode: PlaceMode,
    edit_mode: EditMode,
    delete_mode: DeleteMode,
    obj_cat: Option<ObjCat>,
    obj_sub: usize,
    obj_search: String,
    focus_search: bool,
    favourites: HashSet<String>,
    models: Vec<crate::library::ModelEntry>,
    place_model: Option<String>,
    lib_hover_item: Option<LibItem>,
    side: Option<SidePanel>,
    mat: Option<MatSession>,
    mat_hover: Option<String>,
    mat_filter: String,
    mat_clipboard: Option<MaterialParams>,
    settings: crate::library::AppSettings,
    /// Graphics cards on this computer, and the one in use.
    gpus: Vec<String>,
    gpu_in_use: String,
    gpu_changed: bool,
    library: Vec<crate::library::LibMaterial>,
    library_rx: Option<Receiver<Vec<crate::library::LibMaterial>>>,
    lib_filter: String,
    last_mat_pick: Instant,
    selection: Vec<usize>,
    delete_marks: Vec<usize>,
    hovered_object: Option<usize>,
    drag: Option<DragState>,
    paint_last: Option<Vec3>,
    place_kind: ObjectKind,
    place_scale: f32,
    place_random: bool,
    mass_count: u32,
    mass_radius: f32,
    paint_spacing: f32,
    light_height: f32,
    light_color: [f32; 3],
    light_intensity: f32,
    light_range: f32,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    log: VecDeque<(Instant, String)>,
    project_path: Option<PathBuf>,
    start: Instant,
    paused_time: f32,
    move_speed: f32,
    /// Smoothed camera motion (Lumion-style glide): current velocity and the
    /// remaining mouse-wheel dolly speed.
    cam_vel: Vec3,
    dolly_vel: f32,
    photo_res: (u32, u32),
    /// Why the selected photo size can't be rendered on this GPU (shown in the render popup).
    render_issue: Option<crate::pathtrace::ResolutionLimit>,
    /// Texture-quality reductions last reported to the activity log.
    tex_notes: Vec<String>,
    /// Start screen (launch / File › New project).
    start_page: Option<start::StartPage>,
    start_thumbs: HashMap<String, egui::TextureHandle>,
    pending_template: Option<String>,
    start_loading_drawn: bool,
    /// A project is open behind the start screen (Esc goes back to it).
    start_can_close: bool,
    import_dialog: Option<import_dialog::ImportDialog>,
    photo_match: Option<photo_match::PhotoMatch>,
    /// Textures dropped this frame, freed next frame (after they were drawn).
    retired_textures: Vec<egui::TextureHandle>,
    photo_lock: bool,
    movie: movie::MovieState,
    rt: raytrace::RtState,
    render_popup: bool,
    effect_open: Option<crate::env::EffectKind>,
    frame_times: VecDeque<f32>,
    last_frame: Instant,
    viewport_rect: egui::Rect,
    ctx: egui::Context,
    orbit_pivot: Option<Vec3>,
    last_nav: Instant,
    // performance
    quality: u8,
    adaptive_res: bool,
    move_scale: f32,
    last_move: Instant,
    low_targets: Option<Targets>,
    showing_low: bool,
    last_rendered: Option<(FrameParams, u64, (u32, u32), bool)>,
    last_camera: Camera,
    last_hover_pos: Option<egui::Pos2>,
    frame_ms: f32,
    last_render_at: Instant,
}

const MOVE_TAU: f32 = 0.12;
const DOLLY_TAU: f32 = 0.15;

impl LumenApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        let rs = cc.wgpu_render_state.as_ref().expect("Render Mill needs the wgpu backend");
        let renderer = Renderer::new(rs.device.clone(), rs.queue.clone());
        let slot_targets = renderer.create_targets(256, 144);
        let thumbs = Thumbs::new(rs.device.clone(), rs.queue.clone());
        let (tx, rx) = channel();
        theme::apply(&cc.egui_ctx);

        let mut app = Self {
            world: World::new(),
            renderer,
            targets: None,
            tex_id: None,
            egui_renderer: rs.renderer.clone(),
            thumbs,
            slot_targets,
            slot_thumbs: HashMap::new(),
            model_thumb: None,
            events_tx: tx,
            events_rx: rx,
            file_sync: None,
            import_settings: Arc::new(Mutex::new(ImportSettings {
                up: UpAxis::Y,
                units: Units::Meters,
                recenter: true,
                offset: None,
                texture_dirs: Vec::new(),
            })),
            server: None,
            server_lan: false,
            server_port: DEFAULT_PORT,
            follow_host_camera: false,
            pair_token: crate::sync::new_token(),
            mode: Mode::Build,
            build_tab: BuildTab::Home,
            tool: Tool::Select,
            place_mode: PlaceMode::Single,
            edit_mode: EditMode::Move,
            delete_mode: DeleteMode::Single,
            obj_cat: None,
            obj_sub: 0,
            obj_search: String::new(),
            focus_search: false,
            favourites: HashSet::new(),
            models: Vec::new(),
            place_model: None,
            lib_hover_item: None,
            side: None,
            mat: None,
            mat_hover: None,
            mat_filter: String::new(),
            mat_clipboard: None,
            settings: crate::library::AppSettings::load(),
            gpus: {
                let mut v: Vec<String> = rs
                    .available_adapters
                    .iter()
                    .map(|a| a.get_info())
                    .filter(|i| i.device_type != wgpu::DeviceType::Cpu && i.backend != wgpu::Backend::Gl)
                    .map(|i| i.name)
                    .collect();
                v.sort();
                v.dedup();
                v
            },
            gpu_in_use: {
                let i = rs.adapter.get_info();
                format!("{} ({:?})", i.name, i.backend)
            },
            gpu_changed: false,
            library: Vec::new(),
            library_rx: None,
            lib_filter: String::new(),
            last_mat_pick: Instant::now(),
            selection: Vec::new(),
            delete_marks: Vec::new(),
            hovered_object: None,
            drag: None,
            paint_last: None,
            place_kind: ObjectKind::Tree,
            place_scale: 1.0,
            place_random: true,
            mass_count: 12,
            mass_radius: 8.0,
            paint_spacing: 4.0,
            light_height: 2.6,
            light_color: [1.0, 0.82, 0.62],
            light_intensity: 40.0,
            light_range: 18.0,
            undo: Vec::new(),
            redo: Vec::new(),
            log: VecDeque::new(),
            project_path: None,
            start: Instant::now(),
            paused_time: 0.0,
            move_speed: 8.0,
            cam_vel: Vec3::ZERO,
            dolly_vel: 0.0,
            photo_res: (1920, 1080),
            render_issue: None,
            tex_notes: Vec::new(),
            start_page: None,
            start_thumbs: HashMap::new(),
            pending_template: None,
            start_loading_drawn: false,
            start_can_close: false,
            import_dialog: None,
            photo_match: None,
            retired_textures: Vec::new(),
            photo_lock: false,
            movie: movie::MovieState::default(),
            rt: raytrace::RtState::default(),
            render_popup: false,
            effect_open: None,
            frame_times: VecDeque::new(),
            last_frame: Instant::now(),
            viewport_rect: egui::Rect::NOTHING,
            ctx: cc.egui_ctx.clone(),
            orbit_pivot: None,
            last_nav: Instant::now() - std::time::Duration::from_secs(60),
            // Full quality on a dedicated graphics card (desktop, eGPU, gaming laptop).
            quality: if rs.adapter.get_info().device_type == wgpu::DeviceType::DiscreteGpu { 4 } else { 3 },
            adaptive_res: true,
            move_scale: 1.0,
            last_move: Instant::now(),
            low_targets: None,
            showing_low: false,
            last_rendered: None,
            last_camera: Camera::default(),
            last_hover_pos: None,
            frame_ms: 0.0,
            last_render_at: Instant::now(),
        };
        app.renderer.set_quality(Quality::preset(app.quality));
        app.apply_texture_budget(None);
        app.queue_library_thumbnails();
        app.rescan_library();
        app.favourites = app.settings.favourites.iter().cloned().collect();
        app.models = crate::library::scan_models_labeled(&app.settings.imported_models, &app.settings.model_labels);
        app.start_server();
        match open {
            Some(p) if p.extension().is_some_and(|e| e == "lcproj") => app.open_project(p),
            // A photo: start screen, then the material match for it.
            Some(p) if crate::import::is_image(&p) => {
                app.start_page = Some(start::StartPage::Home);
                app.photo_match_file(p);
            }
            Some(p) => app.start_file_sync(p),
            None => {
                // Lumion-style start screen: pick a landscape template, the
                // example, or open a project.
                app.start_page = Some(start::StartPage::Home);
            }
        }
        app
    }

    fn queue_library_thumbnails(&mut self) {
        for k in ObjectKind::ALL {
            self.thumbs.request(&format!("obj:{k:?}"), Job::Object(k));
        }
        for (name, p) in presets() {
            self.thumbs.request(&format!("mat:{name}"), Job::Material(p.clone()));
        }
        for (name, p) in Self::ground_types() {
            self.thumbs.request(&format!("ground:{name}"), Job::Ground(p));
        }
        for (name, e) in crate::env::Environment::presets() {
            self.thumbs.request(&format!("sky:{name}"), Job::Weather(e));
        }
    }

    /// (Re)index the built-in library and the user's library folders.
    fn rescan_library(&mut self) {
        let ctx = self.ctx.clone();
        self.library_rx = Some(crate::library::scan_all_async(self.settings.library_folders.clone(), move || ctx.request_repaint()));
    }

    fn poll_library(&mut self) {
        if let Some(rx) = &self.library_rx {
            if let Ok(lib) = rx.try_recv() {
                let n_user = lib.iter().filter(|m| m.source != crate::library::Source::Builtin).count();
                self.push_log(format!("Material library: {} built-in, {} from your folders", lib.len() - n_user, n_user));
                self.library = lib;
                self.library_rx = None;
            }
        }
    }

    fn model_entry(&self, key: &str) -> Option<&crate::library::ModelEntry> {
        self.models.iter().find(|m| m.key == key)
    }

    /// Display name of a placed object.
    fn object_name(&self, o: &PlacedObject) -> String {
        match (&o.kind, &o.model) {
            (ObjectKind::Model, Some(k)) => self
                .model_entry(k)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| std::path::Path::new(k).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "3D model".into())),
            _ => o.kind.label().to_string(),
        }
    }

    fn place_label(&self) -> String {
        match (&self.place_kind, &self.place_model) {
            (ObjectKind::Model, Some(k)) => self.model_entry(k).map(|m| m.name.clone()).unwrap_or_else(|| "3D model".into()),
            _ => self.place_kind.label().to_string(),
        }
    }

    fn select_item(&mut self, item: &LibItem) {
        match item {
            LibItem::Kind(k) => {
                self.place_kind = *k;
                self.place_model = None;
            }
            LibItem::Model(m) => {
                self.place_kind = ObjectKind::Model;
                self.place_model = Some(m.clone());
                self.place_scale = 1.0;
            }
        }
        self.tool = Tool::Place;
        self.push_log(format!("{}: click in the view to place it", self.place_label()));
    }

    /// Import FBX/OBJ/glTF files as placeable library objects.
    /// (Several files at once skip the import dialog.)
    fn import_model_files(&mut self, files: Vec<PathBuf>) {
        let mut first = None;
        for f in files {
            if !self.settings.imported_models.contains(&f) {
                self.settings.imported_models.push(f.clone());
            }
            first.get_or_insert(f);
        }
        self.settings.save();
        self.models = crate::library::scan_models_labeled(&self.settings.imported_models, &self.settings.model_labels);
        if let Some(f) = first {
            let key = f.to_string_lossy().to_string();
            if crate::objects::model_asset(&key).is_some() {
                self.select_item(&LibItem::Model(key));
            } else {
                self.push_log(format!("Could not read {}", f.display()));
            }
        }
        self.obj_cat = Some(ObjCat::All);
        self.obj_sub = 1;
    }

    fn set_key(set: &TextureSet) -> String {
        format!("set:{}|{}|{}", set.color.as_deref().unwrap_or(""), set.normal.as_deref().unwrap_or(""), set.size_m)
    }

    fn add_to_library(&mut self, sets: Vec<TextureSet>) -> usize {
        let mut added = 0;
        for set in sets {
            let dup = self.world.texture_library.iter().any(|s| s.color == set.color && s.normal == set.normal);
            if dup {
                continue;
            }
            self.thumbs.request(&Self::set_key(&set), Job::TextureSet(set.clone()));
            self.world.texture_library.push(set);
            added += 1;
        }
        if added > 0 {
            self.push_log(format!("Added {added} texture set(s) to Custom materials."));
        }
        added
    }

    fn import_textures_dialog(&mut self) {
        if let Some(files) = rfd::FileDialog::new().add_filter("Images", IMAGE_EXTS).pick_files() {
            let sets = crate::texlib::group(&files);
            if sets.is_empty() {
                self.push_log("No usable images found (colour or normal maps)");
            }
            self.add_to_library(sets);
        }
    }

    fn import_texture_folder_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            let files = crate::texlib::scan_folder(&dir);
            let sets = crate::texlib::group(&files);
            self.push_log(format!("Found {} images in {}", files.len(), dir.display()));
            self.add_to_library(sets);
        }
    }

    /// Put a library texture set on a material.
    fn apply_texture_set(&mut self, material: &str, set: &TextureSet) {
        if !self.world.load_texture_set(set) {
            self.push_log(format!("Could not read the images of {}", set.name));
            return;
        }
        self.snapshot();
        let old = self.world.scene.effective(material);
        self.world.scene.overrides.insert(material.to_string(), crate::texlib::apply(set, &old));
        self.push_log(format!("Applied {} to {}", set.name, material));
    }

    /// Files dragged onto the window: models are imported, images go to the
    /// texture library (and onto the material being edited, if any).
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let (dropped, hovering) = ctx.input(|i| (i.raw.dropped_files.clone(), !i.raw.hovered_files.is_empty()));
        if hovering {
            egui::Area::new(egui::Id::new("dropzone")).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).interactable(false).show(ctx, |ui| {
                theme::panel_frame().show(ui, |ui| {
                    ui.label(RichText::new("Drop to import").heading());
                    let target = self.mat.as_ref().map(|m| format!("Images are added to Custom materials and applied to \"{}\".", m.name));
                    ui.label(target.unwrap_or_else(|| "Images are added to Custom materials. FBX/glTF/OBJ files are imported as the model.".into()));
                });
            });
        }
        if dropped.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = dropped.iter().filter_map(|f| f.path.clone()).collect();
        let mut images = Vec::new();
        for p in &paths {
            if p.is_dir() {
                images.extend(crate::texlib::scan_folder(p));
            } else if crate::import::is_image(p) {
                images.push(p.clone());
            } else if crate::import::is_supported(p) {
                self.start_file_sync(p.clone());
            }
        }
        if images.is_empty() {
            return;
        }
        let sets = crate::texlib::group(&images);
        self.add_to_library(sets.clone());
        self.mode = Mode::Build;
        self.build_tab = BuildTab::Materials;
        if let (Some(m), Some(first)) = (self.mat.as_ref().map(|m| m.name.clone()), sets.first()) {
            self.apply_texture_set(&m, first);
        }
        if let Some(m) = &mut self.mat {
            m.tab = MatTab::Library;
            m.lib = MatLib::Custom;
        }
    }

    fn ground_types() -> Vec<(&'static str, MaterialParams)> {
        presets()
            .into_iter()
            .filter(|(n, _)| matches!(*n, "Grass" | "Gravel" | "Sand" | "Concrete" | "Asphalt" | "Lake water"))
            .map(|(n, mut p)| {
                if p.kind == MaterialKind::Standard {
                    p.kind = MaterialKind::Ground;
                }
                (n, p)
            })
            .collect()
    }

    fn push_log(&mut self, s: impl Into<String>) {
        let s = s.into();
        log::info!("{s}");
        self.log.push_front((Instant::now(), s));
        self.log.truncate(120);
    }

    // ------------------------------------------------------------- undo
    fn snapshot(&mut self) {
        self.undo.push(Snapshot { objects: self.world.objects.clone(), overrides: self.world.scene.overrides.clone() });
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn restore(&mut self, s: Snapshot) {
        self.world.objects = s.objects;
        self.world.scene.overrides = s.overrides;
        self.world.rebuild_objects();
        self.selection.clear();
        self.delete_marks.clear();
    }

    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(Snapshot { objects: self.world.objects.clone(), overrides: self.world.scene.overrides.clone() });
            self.restore(s);
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(Snapshot { objects: self.world.objects.clone(), overrides: self.world.scene.overrides.clone() });
            self.restore(s);
        }
    }

    /// Remove several objects at once (indices may be in any order).
    fn delete_objects(&mut self, mut idx: Vec<usize>) {
        idx.sort_unstable();
        idx.dedup();
        idx.retain(|i| *i < self.world.objects.len());
        if idx.is_empty() {
            return;
        }
        self.snapshot();
        for i in idx.iter().rev() {
            self.world.objects.remove(*i);
        }
        self.world.rebuild_objects();
        self.push_log(format!("Deleted {} object(s)", idx.len()));
        self.selection.clear();
        self.delete_marks.clear();
    }

    // ------------------------------------------------------------- io
    fn start_server(&mut self) {
        self.server = None;
        let bind = format!("{}:{}", if self.server_lan { "0.0.0.0" } else { "127.0.0.1" }, self.server_port);
        let mut result = Err(anyhow::anyhow!("not started"));
        for _ in 0..20 {
            let c2 = self.ctx.clone();
            result = NetServer::start(&bind, self.pair_token.clone(), self.events_tx.clone(), Arc::new(move || c2.request_repaint()));
            if result.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        match result {
            Ok(s) => {
                self.push_log(format!("LiveSync server listening on {}", s.addr));
                self.server = Some(s);
            }
            Err(e) => self.push_log(format!("LiveSync server failed on {bind}: {e}")),
        }
    }

    fn start_file_sync(&mut self, path: PathBuf) {
        self.start_file_sync_with(path, false);
    }

    /// `keep_settings`: use `world.import` as set by the import dialog.
    fn start_file_sync_with(&mut self, path: PathBuf, keep_settings: bool) {
        self.file_sync = None;
        if keep_settings && self.world.import.is_some() {
            self.world.scene.remove_prefix("demo:");
            if self.world.source.is_none() {
                self.world.objects.clear();
                self.world.rebuild_objects();
            }
        } else if self.world.source.as_ref() != Some(&path) || self.world.import.is_none() {
            let probe = if path.is_dir() {
                std::fs::read_dir(&path)
                    .ok()
                    .and_then(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| crate::import::is_supported(p)))
                    .unwrap_or_else(|| path.join("x.fbx"))
            } else {
                path.clone()
            };
            let dirs = self.world.import.as_ref().map(|i| i.texture_dirs.clone()).unwrap_or_default();
            let mut s = ImportSettings::default_for(&probe);
            s.texture_dirs = dirs;
            self.world.import = Some(s);
            self.world.scene.remove_prefix("demo:");
            if self.world.source.is_none() {
                self.world.objects.clear();
                self.world.rebuild_objects();
            }
        }
        *self.import_settings.lock().unwrap() = self.world.import.clone().unwrap();
        self.world.source = Some(path.clone());
        let ctx = self.ctx.clone();
        match FileSync::start(path.clone(), self.import_settings.clone(), self.events_tx.clone(), move || ctx.request_repaint()) {
            Ok(fs) => {
                self.push_log(format!("LiveSync: watching {} for changes", path.display()));
                self.file_sync = Some(fs);
            }
            Err(e) => self.push_log(format!("Could not watch {}: {e}", path.display())),
        }
    }

    fn stop_file_sync(&mut self) {
        if self.file_sync.take().is_some() {
            self.push_log("LiveSync paused. The model stays in the scene.");
        }
    }

    fn open_project(&mut self, path: PathBuf) {
        match Project::load(&path) {
            Ok(p) => {
                self.world = World::from_project(p);
                for set in self.world.texture_library.clone() {
                    self.thumbs.request(&Self::set_key(&set), Job::TextureSet(set));
                }
                self.slot_thumbs.clear();
                self.model_thumb = None;
                self.undo.clear();
                self.mat = None;
                self.selection.clear();
                self.project_path = Some(path.clone());
                self.remember_recent(&path);
                self.push_log(format!("Opened project {}", path.display()));
                if let Some(src) = self.world.source.clone() {
                    self.start_file_sync(src);
                }
            }
            Err(e) => self.push_log(format!("Could not open project: {e}")),
        }
    }

    fn save_project(&mut self, ask: bool) {
        let path = if ask || self.project_path.is_none() {
            rfd::FileDialog::new().add_filter("Render Mill project", &["lcproj"]).set_file_name("scene.lcproj").save_file()
        } else {
            self.project_path.clone()
        };
        if let Some(p) = path {
            match self.world.to_project().save(&p) {
                Ok(()) => {
                    self.push_log(format!("Saved {}", p.display()));
                    self.remember_recent(&p);
                    self.project_path = Some(p);
                }
                Err(e) => self.push_log(format!("Save failed: {e}")),
            }
        }
    }

    fn remember_recent(&mut self, p: &std::path::Path) {
        let p = p.to_path_buf();
        self.settings.recent_projects.retain(|q| *q != p);
        self.settings.recent_projects.insert(0, p);
        self.settings.recent_projects.truncate(10);
        self.settings.save();
    }

    fn new_project(&mut self) {
        self.file_sync = None;
        self.world = World::new();
        self.project_path = None;
        self.slot_thumbs.clear();
        self.model_thumb = None;
        self.undo.clear();
        self.mat = None;
        self.selection.clear();
    }

    fn load_demo(&mut self) {
        self.new_project();
        crate::demo::build_demo(&mut self.world);
    }

    fn open_model_dialog(&mut self) {
        self.ask_import(false);
    }

    fn time(&self) -> f32 {
        if self.world.env.animate {
            self.start.elapsed().as_secs_f32()
        } else {
            self.paused_time
        }
    }

    fn render_to_file(&mut self, cam: &Camera, path: &std::path::Path) -> anyhow::Result<f32> {
        let (w, h) = self.photo_res;
        let t0 = Instant::now();
        // Photos always render at the highest quality, whatever the viewport uses.
        let viewport_q = self.renderer.quality;
        self.renderer.set_quality(Quality::preset(4));
        self.renderer.invalidate_shadows();
        self.renderer.set_highlight(None, None);
        let saved = std::mem::replace(&mut self.world.camera, cam.clone());
        let fp = self.world.frame_params(w as f32 / h as f32, self.time());
        self.world.camera = saved;
        self.renderer.sync_scene(&self.world.scene);
        let img = crate::headless::render_supersampled(&mut self.renderer, &self.world.scene, &fp, w, h);
        self.renderer.set_quality(viewport_q);
        self.renderer.invalidate_shadows();
        self.last_rendered = None;
        img.save(path)?;
        Ok(t0.elapsed().as_secs_f32())
    }

    /// Check the photo size against the selected GPU's limits before any
    /// rendering starts; on failure explain why in the render popup, which
    /// offers the highest supported size.
    pub(super) fn photo_size_ok(&mut self) -> bool {
        let (w, h) = self.photo_res;
        match crate::pathtrace::check_resolution(&self.renderer.device.limits(), w, h, self.rt.level > 0) {
            Ok(()) => {
                self.render_issue = None;
                true
            }
            Err(e) => {
                self.push_log(e.to_string());
                self.render_issue = Some(e);
                self.render_popup = true;
                false
            }
        }
    }

    fn render_photo(&mut self) {
        if !self.photo_size_ok() {
            return;
        }
        // LC_SAVE_PATH skips the dialog (automated testing).
        let pick = std::env::var("LC_SAVE_PATH").ok().map(PathBuf::from);
        let Some(path) = pick.or_else(|| rfd::FileDialog::new().add_filter("PNG image", &["png"]).set_file_name("render.png").save_file()) else {
            return;
        };
        let cam = self.world.camera.clone();
        if self.rt.level > 0 {
            self.start_raytraced(vec![(cam, path)]);
            return;
        }
        match self.render_to_file(&cam, &path) {
            Ok(t) => self.push_log(format!("Rendered {}x{} to {} in {:.1}s", self.photo_res.0, self.photo_res.1, path.display(), t)),
            Err(e) => self.push_log(format!("Could not save image: {e}")),
        }
    }

    fn render_photo_set(&mut self) {
        if !self.photo_size_ok() {
            return;
        }
        let Some(dir) = rfd::FileDialog::new().pick_folder() else { return };
        let views = self.world.views.clone();
        if self.rt.level > 0 {
            let items = views
                .iter()
                .enumerate()
                .map(|(i, v)| (v.camera.clone(), dir.join(format!("{} {:02}.png", v.name.replace(['/', '\\', ':'], "-"), i + 1))))
                .collect();
            self.start_raytraced(items);
            return;
        }
        for (i, v) in views.iter().enumerate() {
            let path = dir.join(format!("{} {:02}.png", v.name.replace(['/', '\\', ':'], "-"), i + 1));
            if let Err(e) = self.render_to_file(&v.camera, &path) {
                self.push_log(format!("Could not save {}: {e}", path.display()));
            }
        }
        self.push_log(format!("Rendered {} photos to {}", views.len(), dir.display()));
    }

    fn render_small(&mut self, cam: &Camera) -> egui::ColorImage {
        let (w, h) = self.slot_targets.size;
        let saved = std::mem::replace(&mut self.world.camera, cam.clone());
        let fp = self.world.frame_params(w as f32 / h as f32, self.time());
        self.world.camera = saved;
        let hl = (self.mat_hover.clone(), self.mat.as_ref().map(|m| m.name.clone()));
        self.renderer.set_highlight(None, None);
        self.renderer.sync_scene(&self.world.scene);
        self.renderer.render(&self.world.scene, &fp, &self.slot_targets);
        self.renderer.set_highlight(hl.0.as_deref(), hl.1.as_deref());
        self.last_rendered = None;
        let img = self.renderer.read_output(&self.slot_targets);
        egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
    }

    fn slot_thumbnail(&mut self, i: usize) {
        let Some(v) = self.world.views.get(i).cloned() else { return };
        let ci = self.render_small(&v.camera);
        self.slot_thumbs.insert(i, self.ctx.load_texture(format!("slot{i}"), ci, egui::TextureOptions::LINEAR));
    }

    /// Thumbnail of the imported model for the LiveSync card (re-rendered when the model changes).
    fn update_model_thumb(&mut self) {
        let Some((lo, hi)) = self.model_bounds() else {
            self.model_thumb = None;
            return;
        };
        let key = self
            .world
            .scene
            .meshes
            .iter()
            .filter(|(k, _)| k.starts_with("file:") || k.starts_with("net:") || k.starts_with("demo:"))
            .fold(0u64, |a, (_, m)| a.wrapping_mul(31).wrapping_add(m.rev));
        if self.model_thumb.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let mut cam = Camera { fov: 40.0, ..Default::default() };
        cam.frame_bounds(lo, hi);
        let ci = self.render_small(&cam);
        self.model_thumb = Some((key, self.ctx.load_texture("model_thumb", ci, egui::TextureOptions::LINEAR)));
    }

    fn model_bounds(&self) -> Option<(Vec3, Vec3)> {
        self.world
            .scene
            .meshes
            .iter()
            .filter(|(k, _)| k.starts_with("file:") || k.starts_with("net:") || k.starts_with("demo:"))
            .map(|(_, m)| m.bounds)
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }

    fn model_name(&self) -> Option<String> {
        if let Some(fs) = &self.file_sync {
            return fs.target.file_name().map(|n| n.to_string_lossy().to_string());
        }
        if let Some(src) = &self.world.source {
            return src.file_name().map(|n| n.to_string_lossy().to_string());
        }
        if self.world.scene.meshes.keys().any(|k| k.starts_with("net:")) {
            return Some("BimCraft LiveSync".into());
        }
        if self.world.scene.meshes.keys().any(|k| k.starts_with("demo:")) {
            return Some("Demo house".into());
        }
        None
    }

    fn save_slot(&mut self, i: usize) {
        let view = SavedView { name: format!("Photo {}", i + 1), camera: self.world.camera.clone() };
        if i < self.world.views.len() {
            self.world.views[i].camera = view.camera;
        } else {
            self.world.views.push(view);
        }
        let idx = i.min(self.world.views.len() - 1);
        self.slot_thumbnail(idx);
        self.push_log(format!("Saved camera to photo {}", idx + 1));
    }

    fn handle_events(&mut self) {
        for ev in sync::drain(&self.events_rx, 20000) {
            if matches!(ev, SyncEvent::NetCamera { .. }) && !self.follow_host_camera {
                continue;
            }
            let first_file = matches!(ev, SyncEvent::FileLoaded { .. }) && self.world.last_sync.is_none();
            if let Some(msg) = self.world.apply(ev) {
                self.push_log(msg);
            }
            if let Some(s) = &self.world.import {
                let mut shared = self.import_settings.lock().unwrap();
                if shared.offset.is_none() && s.offset.is_some() {
                    *shared = s.clone();
                }
            }
            if first_file {
                self.frame_all();
            }
        }
    }

    fn frame_all(&mut self) {
        let b = self
            .world
            .scene
            .meshes
            .iter()
            .filter(|(k, _)| !k.starts_with("env:") && !k.starts_with("obj:"))
            .map(|(_, m)| m.bounds)
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        if let Some((lo, hi)) = b.or_else(|| self.world.scene.model_bounds()) {
            self.world.camera.frame_bounds(lo, hi);
            self.move_speed = ((hi - lo).length() * 0.25).clamp(2.0, 200.0);
        }
    }

    /// Point the camera at every surface that uses a material.
    fn frame_material(&mut self, name: &str) {
        let b = self.world.scene.meshes.values().filter(|m| m.data.material == name).map(|m| m.bounds).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        if let Some((lo, hi)) = b {
            self.world.camera.frame_bounds(lo, hi);
        }
    }

    // ------------------------------------------------------------- viewport
    fn project(&self, p: Vec3) -> Option<egui::Pos2> {
        let rect = self.viewport_rect;
        let vp = self.world.camera.proj(rect.width() / rect.height()) * self.world.camera.view();
        let c = vp * p.extend(1.0);
        if c.w <= 0.0 {
            return None;
        }
        let ndc = c.truncate() / c.w;
        Some(egui::pos2(rect.left() + (ndc.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - ndc.y * 0.5) * rect.height()))
    }

    fn ray_at(&self, pos: egui::Pos2) -> (Vec3, Vec3) {
        let rect = self.viewport_rect;
        self.world.camera.ray((pos.x - rect.left()) / rect.width(), (pos.y - rect.top()) / rect.height(), rect.width() / rect.height())
    }

    fn light_marker_at(&self, pos: egui::Pos2) -> Option<usize> {
        self.world
            .objects
            .iter()
            .enumerate()
            .find_map(|(i, o)| (o.kind.is_light() && self.project(Vec3::from(o.position)).is_some_and(|p| p.distance(pos) < 14.0)).then_some(i))
    }

    /// Object under the cursor: geometry via ray cast (the model can occlude
    /// it), lights via their marker.
    fn pick_object(&self, pos: egui::Pos2) -> Option<usize> {
        if let Some(i) = self.light_marker_at(pos) {
            return Some(i);
        }
        let (o, d) = self.ray_at(pos);
        let hit = self.world.scene.raycast(o, d)?;
        let rest = hit.1.strip_prefix("obj:")?;
        rest.split(':').next()?.parse().ok()
    }

    /// Like `pick_object`, but ignores the model (cheap enough for hover).
    fn pick_object_fast(&self, pos: egui::Pos2) -> Option<usize> {
        if let Some(i) = self.light_marker_at(pos) {
            return Some(i);
        }
        let (o, d) = self.ray_at(pos);
        let hit = self.world.scene.raycast_filtered(o, d, |id| id.starts_with("obj:"))?;
        hit.1.strip_prefix("obj:")?.split(':').next()?.parse().ok()
    }

    /// Point on the model/terrain under the cursor (library objects are
    /// ignored, so trees don't end up stacked on each other).
    fn surface_hit(&self, pos: egui::Pos2) -> Option<Vec3> {
        let (o, d) = self.ray_at(pos);
        self.world
            .scene
            .raycast_filtered(o, d, |id| !id.starts_with("obj:") && !id.starts_with("env:veg:") && !id.starts_with("env:grass:"))
            .map(|(t, _, _)| o + d * t)
            .or_else(|| (d.y < -1e-4).then(|| o + d * (-o.y / d.y)))
    }

    fn ground_height_at(&self, x: f32, z: f32) -> f32 {
        let from = Vec3::new(x, 2000.0, z);
        self.world
            .scene
            .raycast_filtered(from, -Vec3::Y, |id| !id.starts_with("obj:") && !id.starts_with("env:veg:") && !id.starts_with("env:grass:"))
            .map_or(0.0, |(t, _, _)| from.y - t)
    }

    fn show_targets(&mut self, low: bool) {
        let t = if low { self.low_targets.as_ref() } else { self.targets.as_ref() };
        let Some(t) = t else { return };
        let mut er = self.egui_renderer.write();
        match self.tex_id {
            Some(id) => er.update_egui_texture_from_wgpu_texture(&self.renderer.device, &t.output_view, wgpu::FilterMode::Linear, id),
            None => self.tex_id = Some(er.register_native_texture(&self.renderer.device, &t.output_view, wgpu::FilterMode::Linear)),
        }
        self.showing_low = low;
    }

    /// Render the 3D view into `rect` and handle navigation + tools on it.
    fn viewport(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let ppp = ui.ctx().pixels_per_point();
        let w = (rect.width() * ppp).round().max(16.0) as u32;
        let h = (rect.height() * ppp).round().max(16.0) as u32;
        let samples = if self.renderer.quality.msaa > 1 { 4 } else { 1 };
        if self.targets.as_ref().is_none_or(|t| t.size != (w, h) || t.samples != samples) {
            self.targets = Some(self.renderer.create_targets(w, h));
            self.low_targets = None;
            self.show_targets(false);
            self.last_rendered = None;
        }
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        self.viewport_rect = rect;
        if !(self.mode == Mode::Photo && self.photo_lock) && !(self.mode == Mode::Movie && (self.movie.playing || self.movie.job.is_some())) {
            self.camera_input(ui, &resp);
        }
        if self.mode == Mode::Build {
            self.viewport_tools(ui, &resp);
        } else if resp.clicked() {
            // Photo mode: a click refreshes the preview at full quality.
            self.last_rendered = None;
            self.renderer.invalidate_shadows();
        }
        self.update_highlight(&resp);

        self.world.update_ground();
        self.renderer.sync_scene(&self.world.scene);

        // Adaptive resolution: while the camera or an object is moving and the
        // frame rate drops, render smaller; snap back to full res when still.
        let moving = self.world.camera != self.last_camera || self.drag.is_some();
        self.last_camera = self.world.camera.clone();
        if moving {
            self.last_move = Instant::now();
            if self.adaptive_res {
                let fps = if self.frame_times.len() < 5 { 60.0 } else { 1.0 / (self.frame_times.iter().rev().take(5).sum::<f32>() / 5.0) };
                if fps < 28.0 {
                    self.move_scale = (self.move_scale - 0.125).max(0.5);
                } else if fps > 50.0 {
                    self.move_scale = (self.move_scale + 0.125).min(1.0);
                }
            }
        }
        let still = self.last_move.elapsed().as_secs_f32() > 0.25;
        let use_low = self.adaptive_res && !still && self.move_scale < 0.99;
        if use_low {
            let (lw, lh) = (((w as f32) * self.move_scale) as u32, ((h as f32) * self.move_scale) as u32);
            if self.low_targets.as_ref().is_none_or(|t| t.size != (lw, lh) || t.samples != samples) {
                self.low_targets = Some(self.renderer.create_targets(lw, lh));
                self.last_rendered = None;
            }
        }
        if use_low != self.showing_low {
            self.show_targets(use_low);
            self.last_rendered = None;
        }
        if still && self.showing_low {
            self.show_targets(false);
        }

        // Skip the GPU entirely when nothing that affects the image changed.
        let fp = self.world.frame_params(w as f32 / h as f32, self.time());
        let target = if self.showing_low { self.low_targets.as_ref() } else { self.targets.as_ref() }.unwrap();
        let key = (fp.clone(), self.renderer.content_version(), target.size, self.showing_low);
        let mut unchanged = self.last_rendered.as_ref().is_some_and(|k| *k == key) && self.renderer.stats.shadow_updates == 0;
        // Only clouds/water are animating: 30 fps is plenty and saves the GPU.
        if !unchanged && still && self.world.env.animate {
            if let Some(k) = &self.last_rendered {
                let mut same = k.0.clone();
                same.time = fp.time;
                let only_time = same == fp && k.1 == key.1 && k.2 == key.2 && self.renderer.stats.shadow_updates == 0;
                if only_time && self.last_render_at.elapsed().as_secs_f32() < 1.0 / 30.0 {
                    unchanged = true;
                }
            }
        }
        self.renderer.settle = still;
        if !unchanged {
            let t0 = Instant::now();
            let target = if self.showing_low { self.low_targets.take() } else { self.targets.take() }.unwrap();
            self.renderer.render(&self.world.scene, &fp, &target);
            if self.showing_low {
                self.low_targets = Some(target);
            } else {
                self.targets = Some(target);
            }
            self.frame_ms = t0.elapsed().as_secs_f32() * 1000.0;
            self.last_render_at = Instant::now();
            self.last_rendered = Some(key);
        } else {
            self.renderer.stats.shadow_updates = 0;
        }
        let painter = ui.painter_at(rect);
        let live = if self.mode == Mode::Photo && !self.showing_low { self.live_raytrace(&fp, (w, h), still) } else { None };
        painter.image(live.or(self.tex_id).unwrap(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        if let Some(l) = self.rt.live_job.as_ref().filter(|l| l.has_image() && live.is_some()) {
            let txt = if l.finished() { format!("Ray traced · {} samples", l.samples()) } else { format!("Ray tracing… {} samples", l.samples()) };
            painter.text(rect.right_bottom() + vec2(-8.0, -8.0), Align2::RIGHT_BOTTOM, txt, egui::FontId::proportional(11.0), Color32::from_white_alpha(220));
        }
        // Texture budget: tell the user when maps are being shown smaller.
        let notes = self.texture_notes();
        if notes != self.tex_notes {
            let new: Vec<String> = notes.iter().filter(|n| !self.tex_notes.contains(n)).cloned().collect();
            for n in new {
                self.push_log(n);
            }
            self.tex_notes = notes;
        }
        if !self.tex_notes.is_empty() {
            painter.text(
                rect.left_bottom() + vec2(8.0, -8.0),
                Align2::LEFT_BOTTOM,
                "Reduced texture quality (texture memory budget) - see Settings",
                egui::FontId::proportional(11.0),
                theme::WARN,
            );
        }
        if self.mode == Mode::Build {
            self.draw_overlays(&painter);
        }
    }

    /// Material picker: hovering a surface tints it yellow, the one being
    /// edited blue (only while the brush tab is active).
    fn update_highlight(&mut self, resp: &egui::Response) {
        let picking = self.mode == Mode::Build && self.build_tab == BuildTab::Materials;
        if !picking {
            self.mat_hover = None;
            self.renderer.set_highlight(None, None);
            return;
        }
        let pointer = resp.hover_pos().filter(|_| self.drag.is_none() && !resp.dragged());
        if let Some(pos) = pointer {
            if pointer != self.last_hover_pos && self.last_mat_pick.elapsed().as_secs_f32() > 0.04 {
                self.last_hover_pos = pointer;
                self.last_mat_pick = Instant::now();
                let (o, d) = self.ray_at(pos);
                self.mat_hover = self.world.scene.raycast_filtered(o, d, |id| !id.starts_with("env:sky")).map(|h| h.2);
            }
        } else {
            self.mat_hover = None;
        }
        let sel = self.mat.as_ref().map(|m| m.name.clone());
        self.renderer.set_highlight(self.mat_hover.as_deref(), sel.as_deref());
    }

    fn draw_overlays(&self, painter: &egui::Painter) {
        let home = self.build_tab == BuildTab::Home;
        // Light markers.
        if home {
            for (i, o) in self.world.objects.iter().enumerate() {
                if o.kind.is_light() {
                    if let Some(p) = self.project(Vec3::from(o.position)) {
                        let col = egui::Rgba::from_rgb(o.color[0], o.color[1], o.color[2]);
                        let hl = Some(i) == self.hovered_object || self.selection.contains(&i);
                        painter.circle(p, if hl { 9.0 } else { 7.0 }, col, egui::Stroke::new(2.0_f32, if hl { theme::ACCENT } else { Color32::BLACK }));
                        if o.kind == ObjectKind::SpotLight {
                            painter.line_segment([p, p + vec2(0.0, 18.0)], egui::Stroke::new(2.0_f32, Color32::from(col)));
                        }
                    }
                }
            }
        }
        let bbox = |i: usize| -> Option<egui::Rect> {
            let (lo, hi) = self.world.scene.bounds_with_prefix(&format!("obj:{i}:")).or_else(|| {
                let o = self.world.objects.get(i)?;
                let p = Vec3::from(o.position);
                Some((p - Vec3::splat(0.2), p + Vec3::splat(0.2)))
            })?;
            let mut r = egui::Rect::NOTHING;
            for k in 0..8 {
                let c = Vec3::new(if k & 1 == 0 { lo.x } else { hi.x }, if k & 2 == 0 { lo.y } else { hi.y }, if k & 4 == 0 { lo.z } else { hi.z });
                r.extend_with(self.project(c)?);
            }
            r.is_positive().then_some(r)
        };
        if home {
            if let Some(i) = self.hovered_object {
                let col = if self.tool == Tool::Delete { Color32::from_rgb(235, 60, 60) } else { theme::HOVER_YELLOW };
                if let Some(r) = bbox(i) {
                    painter.rect_stroke(r.expand(3.0), 2.0, egui::Stroke::new(1.5_f32, col));
                }
            }
            for &i in &self.delete_marks {
                if let Some(r) = bbox(i) {
                    painter.rect_filled(r.expand(3.0), 2.0, Color32::from_rgba_unmultiplied(220, 40, 40, 30));
                    painter.rect_stroke(r.expand(3.0), 2.0, egui::Stroke::new(2.0_f32, Color32::from_rgb(235, 60, 60)));
                }
            }
            for &i in &self.selection {
                if let Some(r) = bbox(i) {
                    painter.rect_stroke(r.expand(3.0), 2.0, egui::Stroke::new(1.5_f32, theme::HOVER_YELLOW));
                }
                // Lumion-style axis gizmo at the object's base.
                if let Some(o) = self.world.objects.get(i) {
                    let base = Vec3::from(o.position);
                    let len = (self.world.camera.pos().distance(base) * 0.08).clamp(0.4, 6.0);
                    if let Some(p0) = self.project(base) {
                        for (dir, col) in
                            [(Vec3::X, Color32::from_rgb(230, 50, 50)), (Vec3::Y, Color32::from_rgb(60, 210, 70)), (Vec3::Z, Color32::from_rgb(60, 110, 240))]
                        {
                            if let Some(p1) = self.project(base + dir * len) {
                                painter.line_segment([p0, p1], egui::Stroke::new(2.0_f32, col));
                            }
                        }
                    }
                }
            }
            // Mass-place / area-delete radius on the ground.
            let radius = match (self.tool, self.place_mode, self.delete_mode) {
                (Tool::Place, PlaceMode::Mass, _) => Some(self.mass_radius),
                (Tool::Delete, _, DeleteMode::Area) => Some(self.mass_radius),
                _ => None,
            };
            if let (Some(rad), Some(pos)) = (radius, self.last_hover_pos.or(self.ctx.pointer_hover_pos())) {
                if self.viewport_rect.contains(pos) {
                    if let Some(c) = self.surface_hit(pos) {
                        let pts: Vec<egui::Pos2> = (0..=48)
                            .filter_map(|k| {
                                let a = k as f32 / 48.0 * std::f32::consts::TAU;
                                self.project(c + Vec3::new(a.cos() * rad, 0.05, a.sin() * rad))
                            })
                            .collect();
                        let col = if self.tool == Tool::Delete { Color32::from_rgb(235, 60, 60) } else { theme::ACCENT };
                        painter.add(egui::Shape::line(pts, egui::Stroke::new(2.0_f32, col)));
                    }
                }
            }
        }
        // Material hover label (Lumion shows the surface name next to the cursor).
        if self.build_tab == BuildTab::Materials {
            if let (Some(name), Some(pos)) = (&self.mat_hover, self.ctx.pointer_hover_pos()) {
                if self.viewport_rect.contains(pos) {
                    let g = painter.layout_no_wrap(name.clone(), egui::FontId::proportional(12.0), Color32::BLACK);
                    let r = egui::Rect::from_min_size(pos + vec2(14.0, 14.0), g.size() + vec2(12.0, 6.0));
                    painter.rect_filled(r, 2.0, theme::HOVER_YELLOW);
                    painter.galley(r.min + vec2(6.0, 3.0), g, Color32::BLACK);
                }
            }
        }
    }

    fn viewport_tools(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        let pointer = resp.hover_pos();
        let home = self.build_tab == BuildTab::Home;
        // Hover highlight: only re-test when the pointer moved, and only
        // against placed objects (never the whole imported model).
        if !(home && self.tool != Tool::Place && self.drag.is_none()) {
            self.hovered_object = None;
            if self.build_tab != BuildTab::Materials {
                self.last_hover_pos = pointer;
            }
        } else if pointer != self.last_hover_pos {
            self.last_hover_pos = pointer;
            self.hovered_object = pointer.and_then(|p| self.pick_object_fast(p));
        }
        let ctrl = ui.input(|i| i.modifiers.command);

        if home {
            match self.tool {
                Tool::Select => self.drag_edit(resp),
                Tool::Place if self.place_mode == PlaceMode::Paint => {
                    if resp.drag_started_by(egui::PointerButton::Primary) {
                        self.snapshot();
                        self.paint_last = None;
                    }
                    if resp.dragged_by(egui::PointerButton::Primary) {
                        if let Some(h) = resp.interact_pointer_pos().and_then(|p| self.surface_hit(p)) {
                            let spacing = self.paint_spacing * self.place_scale;
                            if self.paint_last.is_none_or(|l| l.distance(h) >= spacing) {
                                self.paint_last = Some(h);
                                self.place_one(h);
                            }
                        }
                    }
                }
                Tool::Delete if self.delete_mode == DeleteMode::Area && (resp.dragged_by(egui::PointerButton::Primary) || resp.clicked()) => {
                    if let Some(h) = resp.interact_pointer_pos().and_then(|p| self.surface_hit(p)) {
                        for (i, o) in self.world.objects.iter().enumerate() {
                            let d = Vec3::from(o.position) - h;
                            if Vec3::new(d.x, 0.0, d.z).length() <= self.mass_radius && !self.delete_marks.contains(&i) {
                                self.delete_marks.push(i);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if resp.clicked() {
            let Some(pos) = resp.interact_pointer_pos() else { return };
            match self.build_tab {
                BuildTab::Materials => {
                    let (o, d) = self.ray_at(pos);
                    if let Some((_, _, mat)) = self.world.scene.raycast_filtered(o, d, |id| !id.starts_with("env:sky")) {
                        self.open_material(&mat);
                    }
                }
                BuildTab::Home => match self.tool {
                    Tool::Place => match self.place_mode {
                        PlaceMode::Single => {
                            if let Some(h) = self.surface_hit(pos) {
                                self.snapshot();
                                self.place_one(h);
                            }
                        }
                        PlaceMode::Mass => self.place_mass(pos),
                        PlaceMode::Paint => {}
                    },
                    Tool::Select => {
                        let hit = self.pick_object(pos);
                        match (hit, ctrl) {
                            (Some(i), true) => {
                                if let Some(k) = self.selection.iter().position(|s| *s == i) {
                                    self.selection.remove(k);
                                } else {
                                    self.selection.push(i);
                                }
                            }
                            (Some(i), false) => self.selection = vec![i],
                            (None, false) => self.selection.clear(),
                            (None, true) => {}
                        }
                    }
                    Tool::Delete => {
                        if self.delete_mode == DeleteMode::Single {
                            if let Some(i) = self.pick_object(pos) {
                                if let Some(k) = self.delete_marks.iter().position(|s| *s == i) {
                                    self.delete_marks.remove(k);
                                } else {
                                    self.delete_marks.push(i);
                                }
                            }
                        }
                    }
                },
                _ => {}
            }
        }
    }

    /// Move / height / rotate / scale by dragging (applies to the whole selection).
    fn drag_edit(&mut self, resp: &egui::Response) {
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                if let Some(i) = self.pick_object(p) {
                    if !self.selection.contains(&i) {
                        self.selection = vec![i];
                    }
                    self.snapshot();
                    let pos = Vec3::from(self.world.objects[i].position);
                    let hit = self.surface_hit(p).unwrap_or(pos);
                    self.drag = Some(DragState { grab: hit - pos, anchor: i });
                }
            }
        }
        let Some(d) = &self.drag else { return };
        let (anchor, grab) = (d.anchor, d.grab);
        if resp.dragged_by(egui::PointerButton::Primary) && anchor < self.world.objects.len() {
            let delta = resp.drag_delta();
            let apos = Vec3::from(self.world.objects[anchor].position);
            let dist = self.world.camera.pos().distance(apos).max(1.0);
            let mut shift = Vec3::ZERO;
            if self.edit_mode == EditMode::Move {
                if let Some(p) = resp.interact_pointer_pos() {
                    if self.world.objects[anchor].kind.is_light() {
                        let (ro, rd) = self.ray_at(p);
                        if rd.y.abs() > 1e-4 {
                            let t = (apos.y - ro.y) / rd.y;
                            if t > 0.0 {
                                let h = ro + rd * t;
                                shift = Vec3::new(h.x - apos.x, 0.0, h.z - apos.z);
                            }
                        }
                    } else if let Some(h) = self.surface_hit(p) {
                        let target = h - Vec3::new(grab.x, 0.0, grab.z);
                        shift = Vec3::new(target.x - apos.x, h.y - apos.y, target.z - apos.z);
                    }
                }
            }
            let sel = self.selection.clone();
            for &i in &sel {
                let Some(o) = self.world.objects.get_mut(i) else { continue };
                match self.edit_mode {
                    EditMode::Move => {
                        let p = Vec3::from(o.position) + shift;
                        o.position = p.to_array();
                    }
                    EditMode::Rotate => o.rotation = (o.rotation + delta.x * 0.012).rem_euclid(std::f32::consts::TAU),
                    EditMode::Height => o.position[1] -= delta.y * dist * 0.0025,
                    EditMode::Scale => o.scale = (o.scale * (1.0 - delta.y * 0.006)).clamp(0.05, 20.0),
                }
            }
            // Other selected objects follow the terrain when moved.
            if self.edit_mode == EditMode::Move && sel.len() > 1 {
                for &i in &sel {
                    if i != anchor && !self.world.objects[i].kind.is_light() {
                        let p = self.world.objects[i].position;
                        self.world.objects[i].position[1] = self.ground_height_at(p[0], p[2]);
                    }
                }
            }
            for &i in &sel {
                crate::objects::rebuild_one(&mut self.world.scene, &self.world.objects, i);
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
    }

    fn rng(&self) -> impl FnMut() -> f32 {
        let mut seed = (self.start.elapsed().as_nanos() as u32) | 1;
        move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed & 0xFFFF) as f32 / 65535.0
        }
    }

    fn new_object(&self, pos: Vec3, rnd: &mut impl FnMut() -> f32) -> PlacedObject {
        let (rot, scale) = if self.place_random { (rnd() * std::f32::consts::TAU, self.place_scale * (0.8 + rnd() * 0.4)) } else { (0.0, self.place_scale) };
        let mut o = PlacedObject::new(self.place_kind, pos.to_array(), rot, scale, (rnd() * 1e6) as u32 + 1);
        if self.place_kind == ObjectKind::Model {
            o.model = self.place_model.clone();
        }
        if self.place_kind.is_light() {
            o.position[1] += self.light_height;
            o.color = self.light_color;
            o.intensity = self.light_intensity;
            o.range = self.light_range;
            o.scale = 1.0;
        }
        o
    }

    /// Place one object (no undo snapshot; callers take one per gesture).
    fn place_one(&mut self, at: Vec3) {
        let mut rnd = self.rng();
        let o = self.new_object(at, &mut rnd);
        self.world.objects.push(o);
        let i = self.world.objects.len() - 1;
        crate::objects::rebuild_one(&mut self.world.scene, &self.world.objects, i);
    }

    fn place_mass(&mut self, pos: egui::Pos2) {
        let Some(p) = self.surface_hit(pos) else { return };
        self.snapshot();
        let mut rnd = self.rng();
        let n0 = self.world.objects.len();
        for _ in 0..self.mass_count.max(1) {
            let a = rnd() * std::f32::consts::TAU;
            let r = rnd().sqrt() * self.mass_radius;
            let mut q = p + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
            q.y = self.ground_height_at(q.x, q.z);
            let o = self.new_object(q, &mut rnd);
            self.world.objects.push(o);
        }
        for i in n0..self.world.objects.len() {
            crate::objects::rebuild_one(&mut self.world.scene, &self.world.objects, i);
        }
        self.push_log(format!("Placed {} × {}", self.world.objects.len() - n0, self.place_label()));
    }

    fn camera_input(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let orbit_key = !ui.ctx().wants_keyboard_input() && ui.input(|i| i.key_down(egui::Key::O));
        if resp.drag_started_by(egui::PointerButton::Secondary) {
            self.orbit_pivot = if orbit_key {
                let c = self.viewport_rect.center();
                self.surface_hit(c).or_else(|| self.model_bounds().map(|(lo, hi)| (lo + hi) * 0.5))
            } else {
                None
            };
        }
        let cam = &mut self.world.camera;
        let mut nav = false;
        if resp.dragged_by(egui::PointerButton::Secondary) {
            let d = resp.drag_delta();
            nav = true;
            match self.orbit_pivot.filter(|_| orbit_key) {
                Some(pivot) => {
                    // Orbit around the point in the middle of the screen.
                    let off = cam.pos() - pivot;
                    let yaw = glam::Quat::from_rotation_y(-d.x * 0.005);
                    let right = cam.right();
                    let pitch = glam::Quat::from_axis_angle(right, -d.y * 0.005);
                    let new_off = pitch * (yaw * off);
                    if new_off.normalize().y.abs() < 0.98 {
                        cam.position = (pivot + new_off).to_array();
                    } else {
                        cam.position = (pivot + yaw * off).to_array();
                    }
                    cam.look_at(pivot);
                }
                None => {
                    cam.yaw -= d.x * 0.0035;
                    cam.pitch = (cam.pitch - d.y * 0.0035).clamp(-1.55, 1.55);
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Middle) {
            let d = resp.drag_delta();
            let s = self.move_speed * 0.004;
            cam.position = (cam.pos() - cam.right() * d.x * s + Vec3::Y * d.y * s).to_array();
            nav = true;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                // Glide instead of jumping: the same total distance, spread
                // over ~0.15 s (integral of v0 * exp(-t / tau) = v0 * tau).
                self.dolly_vel += scroll * 0.01 * self.move_speed / DOLLY_TAU;
                nav = true;
            }
        }
        if ui.ctx().wants_keyboard_input() {
            if nav {
                self.last_nav = Instant::now();
            }
            return;
        }
        let (mut mv, fast, slow, frame) = ui.input(|i| {
            let k = |key| if i.key_down(key) && !i.modifiers.command { 1.0 } else { 0.0 };
            let f = k(egui::Key::W) + k(egui::Key::ArrowUp) - k(egui::Key::S) - k(egui::Key::ArrowDown);
            let r = k(egui::Key::D) + k(egui::Key::ArrowRight) - k(egui::Key::A) - k(egui::Key::ArrowLeft);
            let u = k(egui::Key::E) - k(egui::Key::Q);
            (Vec3::new(r, u, f), i.modifiers.shift, i.key_down(egui::Key::Space), i.key_pressed(egui::Key::F) && !i.modifiers.command)
        });
        if frame {
            self.frame_all();
            return;
        }
        let cam = &mut self.world.camera;
        if self.dolly_vel.abs() > 1e-3 {
            cam.position = (cam.pos() + cam.forward() * self.dolly_vel * dt).to_array();
            self.dolly_vel *= (-dt / DOLLY_TAU).exp();
            ui.ctx().request_repaint();
            nav = true;
        } else {
            self.dolly_vel = 0.0;
        }
        let mut target = Vec3::ZERO;
        if mv != Vec3::ZERO {
            // Lumion: Shift = fast, Space = slow, both = extra fast.
            let speed = self.move_speed
                * if fast && slow {
                    8.0
                } else if fast {
                    4.0
                } else if slow {
                    0.25
                } else {
                    1.0
                };
            let fwd = cam.forward();
            let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
            let fwd_dir = if resp.dragged_by(egui::PointerButton::Secondary) { fwd } else { flat };
            mv = cam.right() * mv.x + Vec3::Y * mv.y + fwd_dir * mv.z;
            target = mv * speed;
        }
        // Ease in and out of movement (about 0.12 s), like Lumion's camera.
        let k = 1.0 - (-dt / MOVE_TAU).exp();
        self.cam_vel += (target - self.cam_vel) * k;
        if self.cam_vel.length() > 1e-3 * self.move_speed.max(1.0) || target != Vec3::ZERO {
            cam.position = (cam.pos() + self.cam_vel * dt).to_array();
            ui.ctx().request_repaint();
            nav = true;
        } else {
            self.cam_vel = Vec3::ZERO;
        }
        if nav {
            self.last_nav = Instant::now();
        }
    }

    fn set_quality(&mut self, q: u8) {
        self.quality = q.clamp(1, 4);
        self.renderer.set_quality(Quality::preset(self.quality));
        self.last_rendered = None;
        self.push_log(format!("Viewport quality: {} of 4 (F1-F4)", self.quality));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let fkey = ctx.input(|i| [egui::Key::F1, egui::Key::F2, egui::Key::F3, egui::Key::F4].iter().position(|k| i.key_pressed(*k)));
        if let Some(q) = fkey {
            self.set_quality(q as u8 + 1);
        }
        let (undo, redo, save, del, esc, select_all, tool_keys, slots) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            let mut slots = Vec::new();
            for (n, key) in [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
            ]
            .into_iter()
            .enumerate()
            {
                if i.key_pressed(key) {
                    slots.push((n, cmd, i.modifiers.shift));
                }
            }
            let tool = if cmd {
                None
            } else if i.key_pressed(egui::Key::M) {
                Some(EditMode::Move)
            } else if i.key_pressed(egui::Key::H) {
                Some(EditMode::Height)
            } else if i.key_pressed(egui::Key::R) {
                Some(EditMode::Rotate)
            } else if i.key_pressed(egui::Key::L) {
                Some(EditMode::Scale)
            } else {
                None
            };
            (
                cmd && i.key_pressed(egui::Key::Z) && !i.modifiers.shift,
                cmd && (i.key_pressed(egui::Key::Y) || (i.key_pressed(egui::Key::Z) && i.modifiers.shift)),
                cmd && i.key_pressed(egui::Key::S),
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::Escape),
                cmd && i.key_pressed(egui::Key::A),
                tool,
                slots,
            )
        });
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if save {
            self.save_project(false);
        }
        if del {
            let mut idx = std::mem::take(&mut self.selection);
            idx.append(&mut self.delete_marks);
            self.delete_objects(idx);
        }
        if esc {
            self.selection.clear();
            self.delete_marks.clear();
            self.obj_cat = None;
            self.side = None;
            self.render_popup = false;
        }
        if select_all && self.mode == Mode::Build {
            self.build_tab = BuildTab::Home;
            self.tool = Tool::Select;
            self.selection = (0..self.world.objects.len()).collect();
        }
        if let Some(m) = tool_keys {
            if self.mode == Mode::Build {
                self.build_tab = BuildTab::Home;
                self.tool = Tool::Select;
                self.edit_mode = m;
            }
        }
        // Lumion: Ctrl+1..9 stores a camera, Shift+1..9 recalls it.
        for (n, cmd, shift) in slots {
            if cmd {
                self.save_slot(n.min(self.world.views.len()));
            } else if shift {
                if let Some(v) = self.world.views.get(n) {
                    self.world.camera = v.camera.clone();
                }
            }
        }
    }

    fn toast(&mut self, ctx: &egui::Context) {
        if let Some((t, msg)) = self.log.front().cloned() {
            let age = t.elapsed().as_secs_f32();
            if age < 5.0 {
                let alpha = (5.0 - age).clamp(0.0, 1.0);
                egui::Area::new(egui::Id::new("toast")).anchor(Align2::CENTER_TOP, vec2(0.0, TITLE_H + 14.0)).interactable(false).show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(Color32::from_black_alpha((190.0 * alpha) as u8))
                        .rounding(3.0)
                        .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                        .show(ui, |ui| {
                            ui.set_max_width(640.0);
                            ui.label(RichText::new(msg).size(12.0).color(Color32::from_white_alpha((235.0 * alpha) as u8)));
                        });
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }
}

impl eframe::App for LumenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.retired_textures.clear();
        let now = Instant::now();
        self.frame_times.push_back((now - self.last_frame).as_secs_f32());
        self.last_frame = now;
        if self.frame_times.len() > 60 {
            self.frame_times.pop_front();
        }
        if !self.world.env.animate && self.paused_time == 0.0 {
            self.paused_time = self.start.elapsed().as_secs_f32();
        } else if self.world.env.animate {
            self.paused_time = 0.0;
        }
        self.selection.retain(|i| *i < self.world.objects.len());
        self.delete_marks.retain(|i| *i < self.world.objects.len());
        if self.tool != Tool::Delete || self.build_tab != BuildTab::Home {
            self.delete_marks.clear();
        }

        self.handle_events();
        self.poll_library();
        self.shortcuts(ctx);
        self.handle_dropped_files(ctx);
        if self.thumbs.pending() {
            self.thumbs.step(ctx, 2);
            ctx.request_repaint();
        }
        if self.model_thumb.is_none() || self.world.last_sync.is_some() {
            self.update_model_thumb();
        }

        self.window_resize_edges(ctx);
        self.title_bar(ctx);
        if self.start_screen(ctx) {
            self.toast(ctx);
            return;
        }
        self.start_can_close = true;
        self.sidebar(ctx);

        match self.mode {
            Mode::Build => {
                egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::BLACK)).show(ctx, |ui| {
                    let rect = ui.max_rect();
                    self.viewport(ui, rect);
                });
                self.build_overlays(ctx);
            }
            Mode::Photo => {
                egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::PHOTO_BG)).show(ctx, |ui| self.photo_mode(ui));
                self.photo_overlays(ctx);
            }
            Mode::Movie => {
                egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::PHOTO_BG)).show(ctx, |ui| self.movie_mode(ui));
                self.movie_overlays(ctx);
            }
        }
        if self.rt.job.is_some() {
            self.raytrace_step(ctx);
        }
        self.side_panels(ctx);
        self.import_dialog_ui(ctx);
        self.photo_match_ui(ctx);
        self.toast(ctx);

        if self.world.env.animate {
            ctx.request_repaint();
        }
    }
}

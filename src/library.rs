//! Material libraries that live on disk rather than in a project:
//!
//! * the built-in CC0 library shipped next to the executable (`library/`,
//!   one folder per material with a `meta.json`), and
//! * the user's own library folders (for example a folder of manufacturer
//!   textures), indexed in place — nothing is copied.
//!
//! Scanning only lists file names, so even very large folders index quickly;
//! images are decoded when a thumbnail is shown or a material is applied.

use crate::texlib::{self, TextureSet};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Builtin,
    Folder(PathBuf),
}

#[derive(Clone, Debug)]
pub struct LibMaterial {
    pub name: String,
    pub category: String,
    pub set: TextureSet,
    pub metallic: bool,
    pub tags: Vec<String>,
    pub source: Source,
}

#[derive(Deserialize)]
struct Meta {
    name: String,
    category: String,
    size_m: f32,
    #[serde(default)]
    metallic: bool,
    #[serde(default)]
    tags: Vec<String>,
}

/// Where the built-in library is installed (next to the exe, or in the
/// source tree when running from cargo).
pub fn builtin_dir() -> Option<PathBuf> {
    let mut cands = Vec::new();
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        cands.push(exe_dir.join("library"));
        cands.push(exe_dir.join("..").join("library"));
        cands.push(exe_dir.join("..").join("..").join("library"));
    }
    if let Ok(v) = std::env::var("LUMENCRAFT_LIBRARY") {
        cands.insert(0, PathBuf::from(v));
    }
    cands.into_iter().find(|p| p.is_dir())
}

/// Read the built-in CC0 library (`<category>/<id>/meta.json` + maps).
pub fn scan_builtin(root: &Path) -> Vec<LibMaterial> {
    let mut out = Vec::new();
    let Ok(cats) = std::fs::read_dir(root) else { return out };
    for cat in cats.flatten() {
        let Ok(items) = std::fs::read_dir(cat.path()) else { continue };
        for item in items.flatten() {
            let dir = item.path();
            let Ok(text) = std::fs::read_to_string(dir.join("meta.json")) else { continue };
            let Ok(meta) = serde_json::from_str::<Meta>(&text) else { continue };
            let files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            let Some(mut set) = texlib::group(&files).into_iter().next() else { continue };
            set.name = meta.name.clone();
            set.size_m = meta.size_m.max(0.05);
            out.push(LibMaterial { name: meta.name, category: meta.category, set, metallic: meta.metallic, tags: meta.tags, source: Source::Builtin });
        }
    }
    out.sort_by(|a, b| a.category.cmp(&b.category).then(natural_key(&a.name).cmp(&natural_key(&b.name))));
    out
}

/// Guess a library category from a material name.
pub fn guess_category(name: &str) -> &'static str {
    let l = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| l.contains(w));
    if has(&["roof", "shingle", "shake"]) && !has(&["siding"]) {
        "Roofing"
    } else if has(&["siding", "board & batten", "board and batten", "batten", "lap ", "clapboard", "hardie", "cladding"]) {
        "Siding"
    } else if has(&["brick"]) {
        "Brick"
    } else if has(&["paver", "paving", "cobble", "sidewalk", "flagstone"]) {
        "Paving"
    } else if has(&["masonry", "masonary", "stone", "limestone", "ledge", "ashlar", "split face", "rock", "granite", "slate", "quartzite", "fieldstone"]) {
        "Stone"
    } else if has(&["marble", "travertine", "terrazzo", "onyx", "quartz"]) {
        "Marble & stone"
    } else if has(&["tile", "zellige", "mosaic", "ceramic", "porcelain"]) {
        "Tiles"
    } else if has(&["floor", "parquet", "herringbone", "laminate", "hardwood"]) {
        "Wood floor"
    } else if has(&["wood", "oak", "pine", "walnut", "cedar", "ipe", "maple", "plank", "deck", "spruce", "hickory", "mdf", "veneer", "cherry", "birch", "teak"])
    {
        "Wood"
    } else if has(&["metal", "matel", "steel", "copper", "brass", "aluminum", "aluminium", "iron", "zinc", "corrugated", "anodized"]) {
        "Metal"
    } else if has(&["concrete", "cement", "block", "cmu", "stucco"]) {
        "Concrete"
    } else if has(&["grass", "gravel", "mulch", "soil", "dirt", "sand", "ground", "leaves", "moss", "lawn"]) {
        "Ground"
    } else if has(&["asphalt", "road", "tarmac"]) {
        "Asphalt"
    } else if has(&["plaster", "paint", "drywall", "wallpaper"]) {
        "Plaster"
    } else if has(&["fabric", "carpet", "rug", "leather", "linen", "wool", "velvet"]) {
        "Fabric"
    } else if has(&["glass", "glazing"]) {
        "Glass"
    } else {
        "Other"
    }
}

/// Index a folder of the user's textures. Each sub-folder is one material
/// (named after the folder); loose images next to the folders become
/// colour-only materials unless a folder of the same name already has maps.
pub fn scan_user_folder(root: &Path) -> Vec<LibMaterial> {
    let mut out: Vec<LibMaterial> = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else { return out };
    let mut dirs = Vec::new();
    let mut loose = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            dirs.push(p);
        } else if crate::import::is_image(&p) {
            loose.push(p);
        }
    }
    dirs.sort();
    loose.sort();
    let src = Source::Folder(root.to_path_buf());
    let mut covered = std::collections::HashSet::new();
    for d in &dirs {
        let folder = d.file_name().map(|s| s.to_string_lossy().trim().to_string()).unwrap_or_default();
        let files = texlib::scan_folder(d);
        let sets: Vec<TextureSet> = texlib::group(&files).into_iter().filter(|s| s.color.is_some()).collect();
        let n = sets.len();
        for mut set in sets {
            let name = if n == 1 { folder.clone() } else { format!("{folder} - {}", set.name) };
            set.name = name.clone();
            set.size_m = default_size(&name);
            covered.insert(folder.to_lowercase());
            out.push(LibMaterial { category: guess_category(&name).into(), name, set, metallic: false, tags: Vec::new(), source: src.clone() });
        }
    }
    for p in loose {
        let stem = p.file_stem().map(|s| s.to_string_lossy().trim().to_string()).unwrap_or_default();
        if covered.contains(&stem.to_lowercase()) {
            continue;
        }
        let (_, role) = texlib::classify(&p);
        if role != texlib::Role::Color {
            continue;
        }
        let set = TextureSet { name: stem.clone(), color: Some(p.to_string_lossy().to_string()), normal: None, roughness: None, size_m: default_size(&stem) };
        out.push(LibMaterial { category: guess_category(&stem).into(), name: stem, set, metallic: false, tags: Vec::new(), source: src.clone() });
    }
    out.sort_by_key(|a| natural_key(&a.name));
    out
}

/// Sensible real-world tile size (metres) from the kind of material, used
/// until the user sets Map scale. Architectural texture packs usually cover
/// about a sheet of material per image.
pub fn default_size(name: &str) -> f32 {
    match guess_category(name) {
        "Siding" | "Roofing" => 2.4,
        "Brick" | "Stone" | "Concrete" | "Paving" => 1.8,
        "Tiles" | "Marble & stone" => 1.2,
        "Wood floor" | "Wood" => 2.0,
        "Ground" | "Asphalt" => 3.0,
        "Fabric" => 0.8,
        _ => 1.5,
    }
}

fn natural_key(s: &str) -> String {
    s.to_lowercase()
}

/// Persistent app settings (not per project): library folders and so on.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub library_folders: Vec<PathBuf>,
    /// 3D model files the user imported into the object library.
    #[serde(default)]
    pub imported_models: Vec<PathBuf>,
    /// Favourite library objects ("kind:Tree" or a model key).
    #[serde(default)]
    pub favourites: Vec<String>,
    /// Set once the default folders have been offered, so removing one sticks.
    #[serde(default)]
    pub defaults_applied: bool,
    /// Graphics card to render on (part of its name); None = Windows' choice.
    #[serde(default)]
    pub gpu: Option<String>,
    /// Texture memory budget (MB) for the viewport and ray tracing; None =
    /// automatic (2 GB viewport, 3 GB ray tracing).
    #[serde(default)]
    pub texture_budget_mb: Option<u32>,
    /// Most recently opened/saved projects, newest first.
    #[serde(default)]
    pub recent_projects: Vec<PathBuf>,
    /// Names and category folders given to imported library models (by path).
    #[serde(default)]
    pub model_labels: std::collections::BTreeMap<String, ModelLabel>,
}

fn settings_base() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// Folder for Render Mill's own data (settings, caches).
pub fn data_dir() -> Option<PathBuf> {
    Some(settings_base()?.join("Render Mill"))
}

fn settings_path() -> Option<PathBuf> {
    Some(settings_base()?.join("Render Mill").join("settings.json"))
}

/// Settings saved by LumenCraft (the app's name before 0.13.1).
fn legacy_settings_path() -> Option<PathBuf> {
    Some(settings_base()?.join("LumenCraft").join("settings.json"))
}

impl AppSettings {
    pub fn load() -> Self {
        // Fall back to the LumenCraft settings, so library folders, GPU choice
        // and recent projects carry over after the rename.
        let read = |p: Option<PathBuf>| p.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<AppSettings>(&t).ok());
        let mut s: AppSettings = match read(settings_path()) {
            Some(s) => s,
            None => match read(legacy_settings_path()) {
                Some(s) => {
                    s.save();
                    s
                }
                None => AppSettings::default(),
            },
        };
        if !s.defaults_applied {
            // The user's texture collection, if it's on this machine.
            if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
                let p = PathBuf::from(home).join("OneDrive").join("Documents").join("Work").join("WHD").join("Rendering").join("1. Lumion Textures");
                if p.is_dir() && !s.library_folders.contains(&p) {
                    s.library_folders.push(p);
                }
            }
            s.defaults_applied = true;
            s.save();
        }
        s
    }

    pub fn save(&self) {
        if let Some(p) = settings_path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(t) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, t);
            }
        }
    }
}

/// Scan every library in the background; results arrive on the receiver.
pub fn scan_all_async(folders: Vec<PathBuf>, done: impl Fn() + Send + 'static) -> std::sync::mpsc::Receiver<Vec<LibMaterial>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut all = Vec::new();
        if let Some(b) = builtin_dir() {
            all.extend(scan_builtin(&b));
        }
        for f in &folders {
            all.extend(scan_user_folder(f));
        }
        let _ = tx.send(all);
        done();
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        assert_eq!(guess_category("Board & Batten - Espresso"), "Siding");
        assert_eq!(guess_category("Brick Veneer Kingston"), "Brick");
        assert_eq!(guess_category("Masonary - Arbor Granite"), "Stone");
        assert_eq!(guess_category("Metal - Aged Brass"), "Metal");
        assert_eq!(guess_category("Hardie Siding - Shake Shingles - Grey"), "Siding");
        assert_eq!(guess_category("Ground Mulch Large Chips"), "Ground");
    }

    #[test]
    fn manufacturer_pack_layout() {
        let root = std::env::temp_dir().join(format!("lc_lib_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = root.join("Masonry Norway");
        std::fs::create_dir_all(&d).unwrap();
        for f in [
            "Masonry-Norway-001-COLOR_WITH_SHADOWS-2K.png",
            "Masonry-Norway-001-COLOR-2K.png",
            "Masonry-Norway-001-DISPLACEMENT-2K.png",
            "Masonry-Norway-001-GLOSS-2K.png",
            "Masonry-Norway-001-NORMAL-2K.png",
            "Masonry-Norway-001-REFLECTIVITY-2K.png",
            "Masonry-Norway-001-ROUGHNESS-2K.png",
            "Masonry-Norway-sphere-preview-plane.png",
        ] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        std::fs::write(root.join("Masonry Norway.png"), b"x").unwrap();
        std::fs::write(root.join("Ground Gravel.jpg"), b"x").unwrap();
        let mats = scan_user_folder(&root);
        assert_eq!(mats.len(), 2, "{mats:?}");
        let m = mats.iter().find(|m| m.name == "Masonry Norway").unwrap();
        assert!(m.set.color.as_ref().unwrap().ends_with("COLOR-2K.png"));
        assert!(m.set.normal.as_ref().unwrap().ends_with("NORMAL-2K.png"));
        assert!(m.set.roughness.as_ref().unwrap().ends_with("ROUGHNESS-2K.png"));
        assert_eq!(m.category, "Stone");
        let g = mats.iter().find(|m| m.name == "Ground Gravel").unwrap();
        assert_eq!(g.category, "Ground");
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// A 3D model available in the object library.
#[derive(Clone, Debug)]
pub struct ModelEntry {
    /// "builtin:<Category>/<id>" or the file path.
    pub key: String,
    pub name: String,
    /// Built-in category ("Trees", "Seating", ...) or "Imported".
    pub category: String,
    pub tags: Vec<String>,
    pub triangles: u32,
}

#[derive(Deserialize)]
struct ModelMeta {
    name: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    triangles: u32,
}

/// The built-in CC0 models plus the user's imported model files.
/// Name and category folder the user gave an imported model.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelLabel {
    pub name: String,
    pub category: String,
}

/// `scan_models` with the user's names and category folders applied.
pub fn scan_models_labeled(imported: &[PathBuf], labels: &std::collections::BTreeMap<String, ModelLabel>) -> Vec<ModelEntry> {
    let mut v = scan_models(imported);
    for m in &mut v {
        if let Some(l) = labels.get(&m.key) {
            if !l.name.is_empty() {
                m.name = l.name.clone();
            }
            if !l.category.is_empty() {
                m.category = l.category.clone();
            }
        }
    }
    v
}

pub fn scan_models(imported: &[PathBuf]) -> Vec<ModelEntry> {
    let mut out = Vec::new();
    if let Some(root) = crate::objects::builtin_models_dir() {
        if let Ok(cats) = std::fs::read_dir(&root) {
            for cat in cats.flatten() {
                let cat_dir = cat.file_name().to_string_lossy().to_string();
                let Ok(items) = std::fs::read_dir(cat.path()) else { continue };
                for it in items.flatten() {
                    let id = it.file_name().to_string_lossy().to_string();
                    let Ok(t) = std::fs::read_to_string(it.path().join("meta.json")) else { continue };
                    let Ok(m) = serde_json::from_str::<ModelMeta>(&t) else { continue };
                    out.push(ModelEntry {
                        key: format!("builtin:{cat_dir}/{id}"),
                        name: m.name,
                        category: cat_dir.replace("_and_", " & ").replace('_', " "),
                        tags: m.tags,
                        triangles: m.triangles,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
    for p in imported {
        if p.is_file() {
            let name = p.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ")).unwrap_or_default();
            out.push(ModelEntry { key: p.to_string_lossy().to_string(), name, category: "Imported".into(), tags: vec!["imported".into()], triangles: 0 });
        }
    }
    out
}

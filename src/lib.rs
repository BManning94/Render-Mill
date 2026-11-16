//! LumenCraft — real-time architectural visualization with live model sync.
pub mod app;
pub mod bench;
pub mod bvh;
pub mod collada;
pub mod demo;
pub mod denoise;
pub mod env;
pub mod fbx;
pub mod headless;
pub mod import;
pub mod landscape;
pub mod lumion;
pub mod photomatch;

/// The app's name as shown to people (renamed from LumenCraft in 0.13.1;
/// the crate, `.lcproj` files and the LiveSync protocol keep their names).
pub const APP_NAME: &str = "Render Mill";
pub mod library;
pub mod mips;
pub mod objects;
pub mod pathtrace;
pub mod preview;
pub mod project;
pub mod protocol;
pub mod renderer;
pub mod scene;
pub mod sync;
pub mod texlib;
pub mod ui;

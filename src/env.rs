//! Camera, environment (weather/sun/effects) and conversions to renderer params.

use crate::renderer::FrameParams;
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Camera {
    pub position: [f32; 3],
    /// Radians, 0 = looking down -Z.
    pub yaw: f32,
    pub pitch: f32,
    /// Vertical field of view in degrees.
    pub fov: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { position: [18.0, 6.0, 22.0], yaw: 0.68, pitch: -0.18, fov: 55.0 }
    }
}

impl Camera {
    pub fn pos(&self) -> Vec3 {
        Vec3::from(self.position)
    }
    pub fn forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }
    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).normalize_or_zero()
    }
    pub fn view(&self) -> Mat4 {
        Mat4::look_to_rh(self.pos(), self.forward(), Vec3::Y)
    }
    pub fn proj(&self, aspect: f32) -> Mat4 {
        // Reverse-Z with an infinite far plane: full depth precision at any
        // distance, so large sites and far terrain don't flicker.
        Mat4::perspective_infinite_reverse_rh(self.fov.to_radians(), aspect.max(0.01), 0.05)
    }
    pub fn look_at(&mut self, target: Vec3) {
        let d = (target - self.pos()).normalize_or_zero();
        self.pitch = d.y.clamp(-0.999, 0.999).asin();
        self.yaw = (-d.x).atan2(-d.z);
    }
    /// Ray through a pixel (0..1 normalized viewport coords, y down).
    pub fn ray(&self, nx: f32, ny: f32, aspect: f32) -> (Vec3, Vec3) {
        let inv = (self.proj(aspect) * self.view()).inverse();
        let ndc = glam::Vec2::new(nx * 2.0 - 1.0, 1.0 - ny * 2.0);
        let a = inv.project_point3(ndc.extend(1.0)); // near plane
        let b = inv.project_point3(ndc.extend(0.5)); // 2x near
        (a, (b - a).normalize())
    }
    /// Frame a bounding box (Lumion "F" / zoom extents).
    pub fn frame_bounds(&mut self, lo: Vec3, hi: Vec3) {
        let c = (lo + hi) * 0.5;
        let r = ((hi - lo).length() * 0.5).max(1.0);
        let dist = r / (self.fov.to_radians() * 0.5).tan() * 1.1;
        let dir = Vec3::new(0.65, 0.35, 0.75).normalize();
        self.position = (c + dir * dist).to_array();
        self.look_at(c);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Environment {
    /// Sun compass heading in degrees (0 = north/-Z).
    pub sun_heading: f32,
    /// Sun height above horizon in degrees (-20..90).
    pub sun_height: f32,
    pub sun_intensity: f32,
    pub ambient: f32,
    pub cloudiness: f32,
    pub haze: f32,
    pub fog: f32,
    pub wind: f32,
    pub shadow_softness: f32,
    pub reflections: f32,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub vignette: f32,
    pub bloom: f32,
    pub sharpen: f32,
    #[serde(default = "one")]
    pub ao: f32,
    pub ground_enabled: bool,
    pub ground_size: f32,
    pub animate: bool,
    /// Photo effect stack (Lumion's "Effect list"). Effects that are not in the
    /// list, or are hidden, fall back to neutral values.
    #[serde(default = "all_effects")]
    pub effects: Vec<PhotoEffect>,
    /// Master switch for the whole effect list.
    #[serde(default = "yes")]
    pub effects_on: bool,
    /// Name of the style the effects came from (shown in Photo mode).
    #[serde(default = "custom_style")]
    pub style: String,
    /// Depth of field: blur amount and focus distance in metres (0 = auto focus).
    #[serde(default = "half")]
    pub dof: f32,
    /// Filmic (AgX) tone curve: natural highlights and colour like a camera.
    #[serde(default = "yes")]
    pub filmic: bool,
    #[serde(default)]
    pub dof_focus: f32,
    #[serde(default = "half")]
    pub chroma: f32,
    #[serde(default = "grain_default")]
    pub grain: f32,
    /// Screen-space reflections of the scene on glass, water and glossy floors.
    #[serde(default = "one")]
    pub ssr: f32,
    /// Distant mountains and forest around the site.
    #[serde(default = "yes")]
    pub backdrop: bool,
    /// 3D grass on landscape near the model.
    #[serde(default = "yes")]
    pub grass: bool,
    #[serde(default = "grass_height")]
    pub grass_height: f32,
    /// Terrain, water and vegetation from a landscape template (None = flat ground).
    #[serde(default)]
    pub landscape: Option<crate::landscape::Landscape>,
}

fn half() -> f32 {
    0.5
}
fn grain_default() -> f32 {
    0.25
}
fn grass_height() -> f32 {
    0.12
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EffectKind {
    ColorCorrection,
    SkyLight,
    Fog,
    Shadow,
    Reflection,
    Bloom,
    Vignette,
    Sharpen,
    DepthOfField,
    ChromaticAberration,
    FilmGrain,
    AutoExposure,
    Reflections,
}

impl EffectKind {
    pub const ALL: [EffectKind; 13] = [
        EffectKind::ColorCorrection,
        EffectKind::SkyLight,
        EffectKind::Fog,
        EffectKind::Shadow,
        EffectKind::Reflection,
        EffectKind::Bloom,
        EffectKind::Vignette,
        EffectKind::Sharpen,
        EffectKind::DepthOfField,
        EffectKind::ChromaticAberration,
        EffectKind::FilmGrain,
        EffectKind::AutoExposure,
        EffectKind::Reflections,
    ];
    pub fn label(self) -> &'static str {
        match self {
            EffectKind::ColorCorrection => "Color correction",
            EffectKind::SkyLight => "Sky light",
            EffectKind::Fog => "Fog",
            EffectKind::Shadow => "Shadow",
            EffectKind::Reflection => "Reflection",
            EffectKind::Bloom => "Bloom",
            EffectKind::Vignette => "Vignette",
            EffectKind::Sharpen => "Sharpen",
            EffectKind::DepthOfField => "Depth of field",
            EffectKind::ChromaticAberration => "Chromatic aberration",
            EffectKind::FilmGrain => "Film grain",
            EffectKind::AutoExposure => "Auto exposure",
            EffectKind::Reflections => "Screen-space reflections",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PhotoEffect {
    pub kind: EffectKind,
    pub enabled: bool,
}

fn all_effects() -> Vec<PhotoEffect> {
    EffectKind::ALL[..8].iter().chain([EffectKind::Reflections].iter()).map(|&kind| PhotoEffect { kind, enabled: true }).collect()
}
fn default_effects() -> Vec<PhotoEffect> {
    [EffectKind::ColorCorrection, EffectKind::SkyLight, EffectKind::Fog, EffectKind::Shadow, EffectKind::Reflection, EffectKind::Reflections, EffectKind::Bloom]
        .iter()
        .map(|&kind| PhotoEffect { kind, enabled: true })
        .collect()
}
fn yes() -> bool {
    true
}
fn custom_style() -> String {
    "Custom style".into()
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sun_heading: 215.0,
            sun_height: 38.0,
            sun_intensity: 3.2,
            ambient: 1.0,
            cloudiness: 0.35,
            haze: 0.25,
            fog: 0.05,
            wind: 1.0,
            shadow_softness: 0.3,
            reflections: 1.0,
            exposure: 1.0,
            contrast: 1.05,
            saturation: 1.05,
            vignette: 0.25,
            bloom: 0.4,
            sharpen: 0.15,
            ao: 1.0,
            ground_enabled: true,
            ground_size: 50000.0,
            animate: true,
            effects: default_effects(),
            effects_on: true,
            style: "Realistic".into(),
            dof: 0.3,
            filmic: true,
            dof_focus: 0.0,
            chroma: 0.5,
            grain: 0.25,
            ssr: 1.0,
            backdrop: true,
            grass: true,
            grass_height: 0.12,
            landscape: None,
        }
    }
}

impl Environment {
    pub fn sun_dir(&self) -> Vec3 {
        let h = self.sun_height.to_radians();
        let a = self.sun_heading.to_radians();
        Vec3::new(a.sin() * h.cos(), h.sin(), -a.cos() * h.cos()).normalize()
    }

    pub fn sun_color(&self) -> Vec3 {
        let h = self.sun_dir().y;
        let warm = Vec3::new(1.0, 0.42, 0.16);
        let noon = Vec3::new(1.0, 0.95, 0.88);
        let t = smoothstep(-0.02, 0.45, h);
        warm.lerp(noon, t) * (1.0 - self.cloudiness * 0.55)
    }

    pub fn effect_on(&self, k: EffectKind) -> bool {
        self.effects_on && self.effects.iter().any(|e| e.kind == k && e.enabled)
    }

    /// Copy only the weather (sun, sky, light) from a preset, keeping effects,
    /// landscape and animation settings.
    pub fn apply_weather(&mut self, w: &Environment) {
        self.sun_heading = w.sun_heading;
        self.sun_height = w.sun_height;
        self.sun_intensity = w.sun_intensity;
        self.ambient = w.ambient;
        self.cloudiness = w.cloudiness;
        self.haze = w.haze;
        self.shadow_softness = w.shadow_softness;
        self.exposure = w.exposure;
        self.bloom = w.bloom;
        self.saturation = w.saturation;
    }

    /// The values the renderer actually uses once the effect list is applied.
    pub fn effective(&self) -> Environment {
        let mut e = self.clone();
        let d = Environment::default();
        if !self.effect_on(EffectKind::ColorCorrection) {
            e.contrast = 1.0;
            e.saturation = 1.0;
        }
        if !self.effect_on(EffectKind::SkyLight) {
            e.ao = 0.0;
            e.ambient *= 0.8;
        }
        if !self.effect_on(EffectKind::Fog) {
            e.fog = 0.0;
        }
        if !self.effect_on(EffectKind::Shadow) {
            e.shadow_softness = d.shadow_softness;
        }
        if !self.effect_on(EffectKind::Reflection) {
            e.reflections = 0.6;
        }
        if !self.effect_on(EffectKind::Bloom) {
            e.bloom = 0.0;
        }
        if !self.effect_on(EffectKind::Vignette) {
            e.vignette = 0.0;
        }
        if !self.effect_on(EffectKind::Sharpen) {
            e.sharpen = 0.0;
        }
        if !self.effect_on(EffectKind::DepthOfField) {
            e.dof = 0.0;
        }
        if !self.effect_on(EffectKind::ChromaticAberration) {
            e.chroma = 0.0;
        }
        if !self.effect_on(EffectKind::FilmGrain) {
            e.grain = 0.0;
        }
        if !self.effect_on(EffectKind::Reflections) {
            e.ssr = 0.0;
        }
        e
    }

    pub fn frame_params(&self, cam: &Camera, aspect: f32, time: f32) -> FrameParams {
        let e = self.effective();
        e.frame_params_raw(cam, aspect, time)
    }

    fn frame_params_raw(&self, cam: &Camera, aspect: f32, time: f32) -> FrameParams {
        FrameParams {
            view: cam.view(),
            proj: cam.proj(aspect),
            cam_pos: cam.pos(),
            time,
            sun_dir: self.sun_dir(),
            sun_color: self.sun_color(),
            sun_intensity: self.sun_intensity,
            ambient: self.ambient,
            cloudiness: self.cloudiness,
            fog: self.fog,
            haze: self.haze,
            shadow_softness: self.shadow_softness + self.cloudiness * 0.6,
            wind: self.wind,
            reflections: self.reflections,
            exposure: self.exposure,
            contrast: self.contrast,
            saturation: self.saturation,
            vignette: self.vignette,
            bloom: self.bloom,
            sharpen: self.sharpen,
            ao: self.ao,
            ao_radius: 0.8,
            ssr: self.ssr,
            lens: [self.dof, self.dof_focus, self.chroma, self.grain],
            filmic: self.filmic,
            auto_exposure: self.effect_on(EffectKind::AutoExposure),
            lights: Vec::new(),
        }
    }

    /// Lumion-style style presets.
    pub fn presets() -> Vec<(&'static str, Environment)> {
        let d = Environment::default();
        vec![
            ("Realistic midday", d.clone()),
            ("Golden hour", Environment { sun_height: 7.0, sun_heading: 250.0, haze: 0.5, exposure: 1.25, bloom: 0.8, cloudiness: 0.25, ..d.clone() }),
            ("Overcast", Environment { cloudiness: 0.9, sun_intensity: 2.0, haze: 0.6, ambient: 1.3, shadow_softness: 1.0, saturation: 0.9, ..d.clone() }),
            ("Clear morning", Environment { sun_height: 18.0, sun_heading: 110.0, cloudiness: 0.1, haze: 0.15, ..d.clone() }),
            ("Dusk", Environment { sun_height: -4.0, sun_heading: 260.0, exposure: 2.2, bloom: 1.0, cloudiness: 0.3, ..d.clone() }),
            ("Night", Environment { sun_height: -25.0, exposure: 3.0, ambient: 1.0, cloudiness: 0.1, bloom: 1.2, ..d.clone() }),
        ]
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn one() -> f32 {
    1.0
}

/// A movie clip: a camera path through keyframes (Lumion's movie clips).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Clip {
    pub name: String,
    pub keys: Vec<Camera>,
    /// Length in seconds.
    pub duration: f32,
}

impl Clip {
    pub fn new(name: String) -> Self {
        Self { name, keys: Vec::new(), duration: 10.0 }
    }

    /// Camera at time `t` (seconds). Smooth Catmull-Rom path through the
    /// keyframes, with the camera easing in and out at the ends.
    pub fn camera_at(&self, t: f32) -> Option<Camera> {
        let n = self.keys.len();
        if n == 0 {
            return None;
        }
        if n == 1 {
            return Some(self.keys[0].clone());
        }
        let u = (t / self.duration.max(0.01)).clamp(0.0, 1.0);
        // ease in/out
        let u = u * u * (3.0 - 2.0 * u) * 0.3 + u * 0.7;
        let f = u * (n - 1) as f32;
        let i = (f.floor() as usize).min(n - 2);
        let s = f - i as f32;
        let k = |j: isize| &self.keys[j.clamp(0, n as isize - 1) as usize];
        let (p0, p1, p2, p3) = (k(i as isize - 1), k(i as isize), k(i as isize + 1), k(i as isize + 2));
        let cr = |a: f32, b: f32, c: f32, d: f32| {
            0.5 * ((2.0 * b) + (-a + c) * s + (2.0 * a - 5.0 * b + 4.0 * c - d) * s * s + (-a + 3.0 * b - 3.0 * c + d) * s * s * s)
        };
        // Unwrap yaw so the camera turns the short way round.
        let unwrap = |base: f32, y: f32| {
            let mut d = y - base;
            while d > std::f32::consts::PI {
                d -= std::f32::consts::TAU;
            }
            while d < -std::f32::consts::PI {
                d += std::f32::consts::TAU;
            }
            base + d
        };
        let y1 = p1.yaw;
        let y0 = unwrap(y1, p0.yaw);
        let y2 = unwrap(y1, p2.yaw);
        let y3 = unwrap(y2, p3.yaw);
        let pos = [0, 1, 2].map(|c| cr(p0.position[c], p1.position[c], p2.position[c], p3.position[c]));
        Some(Camera {
            position: pos,
            yaw: cr(y0, y1, y2, y3),
            pitch: cr(p0.pitch, p1.pitch, p2.pitch, p3.pitch).clamp(-1.55, 1.55),
            fov: p1.fov + (p2.fov - p1.fov) * s,
        })
    }
}

#[cfg(test)]
mod clip_tests {
    use super::*;

    #[test]
    fn path_hits_keyframes_and_turns_short_way() {
        let mut c = Clip::new("A".into());
        c.keys.push(Camera { position: [0.0, 2.0, 0.0], yaw: 3.0, ..Default::default() });
        c.keys.push(Camera { position: [10.0, 2.0, 0.0], yaw: -3.0, ..Default::default() });
        let a = c.camera_at(0.0).unwrap();
        let b = c.camera_at(c.duration).unwrap();
        assert!((a.position[0]).abs() < 1e-3 && (b.position[0] - 10.0).abs() < 1e-3);
        let mid = c.camera_at(c.duration * 0.5).unwrap();
        // 3.0 -> -3.0 is 0.28 rad the short way; the middle must be near pi, not 0.
        assert!(mid.yaw.abs() > 3.0, "yaw {}", mid.yaw);
    }
}

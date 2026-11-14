//! Vector icons drawn with the painter (no dependency on emoji fonts).

use egui::{epaint::PathShape, pos2, vec2, Color32, Painter, Pos2, Rect, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Sun,
    Landscape,
    Materials,
    Objects,
    Sync,
    Build,
    Photo,
    Save,
    Menu,
    Select,
    Place,
    Mass,
    Move,
    Rotate,
    Height,
    Scale,
    Delete,
    Light,
    Person,
    Tree,
    Outdoor,
    Eye,
    Plus,
    Folder,
    Undo,
    Render,
    Effects,
    Home,
    Brush,
    WeatherCloud,
    Car,
    Chair,
    Group,
    Grid,
    SelectBox,
    PlaceDown,
    Dot,
    Pencil,
    Circle,
    Deselect,
    Search,
    EyeOff,
    Walker,
    Film,
    Chat,
    Gear,
    Help,
    Power,
    Copy,
    Paste,
    Kebab,
    Scan,
    Sliders,
    History,
    Leaf,
    Check,
    Close,
    Star,
    StarFilled,
    Lock,
    Unlock,
    Minimize,
    Maximize,
    Restore,
    ArrowUp,
    ChevronUp,
    ChevronDown,
    ChevronRight,
    Logo,
    Bench,
    Random,
    Import,
}

fn poly(p: &Painter, pts: Vec<Pos2>, closed: bool, s: Stroke) {
    if closed {
        p.add(PathShape::closed_line(pts, s));
    } else {
        p.add(PathShape::line(pts, s));
    }
}

fn arrow_head(p: &Painter, tip: Pos2, dir: egui::Vec2, size: f32, s: Stroke) {
    let d = dir.normalized();
    let n = vec2(-d.y, d.x);
    poly(p, vec![tip - d * size + n * size * 0.7, tip, tip - d * size - n * size * 0.7], false, s);
}

pub fn draw(p: &Painter, icon: Icon, rect: Rect, color: Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    let w = (r * 0.13).max(1.4);
    let s = Stroke::new(w, color);
    match icon {
        Icon::Sun => {
            p.circle_stroke(c, r * 0.36, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * r * 0.55, c + d * r * 0.85], s);
            }
        }
        Icon::Landscape => {
            poly(
                p,
                vec![
                    c + vec2(-r * 0.9, r * 0.55),
                    c + vec2(-r * 0.3, -r * 0.4),
                    c + vec2(0.05 * r, r * 0.1),
                    c + vec2(r * 0.35, -r * 0.2),
                    c + vec2(r * 0.9, r * 0.55),
                ],
                true,
                s,
            );
            p.circle_stroke(c + vec2(r * 0.5, -r * 0.6), r * 0.16, s);
        }
        Icon::Materials => {
            p.circle_stroke(c, r * 0.75, s);
            p.circle_filled(c + vec2(-r * 0.28, -r * 0.28), r * 0.17, color);
            poly(
                p,
                vec![c + vec2(-r * 0.75, 0.0), c + vec2(-r * 0.2, r * 0.15), c + vec2(r * 0.4, r * 0.05), c + vec2(r * 0.75, -r * 0.15)],
                false,
                Stroke::new(w * 0.7, color),
            );
        }
        Icon::Objects | Icon::Tree => {
            p.circle_stroke(c + vec2(0.0, -r * 0.25), r * 0.5, s);
            p.line_segment([c + vec2(0.0, r * 0.2), c + vec2(0.0, r * 0.85)], s);
            p.line_segment([c + vec2(-r * 0.4, r * 0.85), c + vec2(r * 0.4, r * 0.85)], s);
        }
        Icon::Sync => {
            let arc = |from: f32, to: f32| -> Vec<Pos2> {
                (0..=12)
                    .map(|i| {
                        let a = from + (to - from) * i as f32 / 12.0;
                        c + vec2(a.cos(), a.sin()) * r * 0.62
                    })
                    .collect()
            };
            let a1 = arc(3.6, 6.0);
            let a2 = arc(0.45, 2.85);
            let (t1, t2) = (*a1.last().unwrap(), *a2.last().unwrap());
            poly(p, a1, false, s);
            poly(p, a2, false, s);
            arrow_head(p, t1, vec2(-(6.0f32).sin(), 6.0f32.cos()), r * 0.28, s);
            arrow_head(p, t2, vec2(-(2.85f32).sin(), 2.85f32.cos()), r * 0.28, s);
        }
        Icon::Build => {
            poly(p, vec![c + vec2(-r * 0.75, -r * 0.05), c + vec2(0.0, -r * 0.75), c + vec2(r * 0.75, -r * 0.05)], false, s);
            poly(p, vec![c + vec2(-r * 0.55, -r * 0.2), c + vec2(-r * 0.55, r * 0.75), c + vec2(r * 0.55, r * 0.75), c + vec2(r * 0.55, -r * 0.2)], false, s);
            p.rect_stroke(Rect::from_center_size(c + vec2(0.0, r * 0.42), vec2(r * 0.4, r * 0.66)), 0.0, s);
        }
        Icon::Photo | Icon::Render => {
            p.rect_stroke(Rect::from_center_size(c + vec2(0.0, r * 0.1), vec2(r * 1.6, r * 1.1)), r * 0.15, s);
            p.circle_stroke(c + vec2(0.0, r * 0.12), r * 0.32, s);
            p.line_segment([c + vec2(-r * 0.4, -r * 0.45), c + vec2(-r * 0.15, -r * 0.7)], s);
            p.line_segment([c + vec2(-r * 0.15, -r * 0.7), c + vec2(r * 0.25, -r * 0.7)], s);
            if icon == Icon::Render {
                p.circle_filled(c + vec2(r * 0.6, -r * 0.25), r * 0.1, color);
            }
        }
        Icon::Save => {
            let b = Rect::from_center_size(c, vec2(r * 1.5, r * 1.5));
            poly(p, vec![b.left_top(), b.right_top() - vec2(r * 0.35, 0.0), b.right_top() + vec2(0.0, r * 0.35), b.right_bottom(), b.left_bottom()], true, s);
            p.rect_stroke(Rect::from_min_max(b.left_top() + vec2(r * 0.3, 0.0), b.left_top() + vec2(r * 1.0, r * 0.5)), 0.0, s);
            p.rect_stroke(Rect::from_min_max(b.left_bottom() + vec2(r * 0.25, -r * 0.65), b.right_bottom() - vec2(r * 0.25, 0.0)), 0.0, s);
        }
        Icon::Menu => {
            for k in [-0.45, 0.0, 0.45] {
                p.line_segment([c + vec2(-r * 0.7, r * k), c + vec2(r * 0.7, r * k)], s);
            }
        }
        Icon::Select => {
            poly(
                p,
                vec![
                    c + vec2(-r * 0.5, -r * 0.8),
                    c + vec2(-r * 0.5, r * 0.55),
                    c + vec2(-r * 0.15, r * 0.2),
                    c + vec2(r * 0.15, r * 0.8),
                    c + vec2(r * 0.35, r * 0.7),
                    c + vec2(r * 0.05, r * 0.1),
                    c + vec2(r * 0.5, r * 0.1),
                ],
                true,
                s,
            );
        }
        Icon::Place | Icon::Plus => {
            p.line_segment([c + vec2(-r * 0.6, 0.0), c + vec2(r * 0.6, 0.0)], s);
            p.line_segment([c + vec2(0.0, -r * 0.6), c + vec2(0.0, r * 0.6)], s);
            if icon == Icon::Place {
                p.circle_stroke(c, r * 0.85, Stroke::new(w * 0.6, color));
            }
        }
        Icon::Mass => {
            for (x, y) in [(-0.45, -0.35), (0.3, -0.5), (0.5, 0.25), (-0.2, 0.45), (0.0, -0.02), (-0.6, 0.2)] {
                p.circle_filled(c + vec2(r * x, r * y), r * 0.13, color);
            }
        }
        Icon::Move => {
            for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
                p.line_segment([c, c + d * r * 0.8], s);
                arrow_head(p, c + d * r * 0.8, d, r * 0.25, s);
            }
        }
        Icon::Rotate => {
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = 0.6 + 4.9 * i as f32 / 16.0;
                    c + vec2(a.cos(), a.sin()) * r * 0.62
                })
                .collect();
            let tip = *pts.last().unwrap();
            poly(p, pts, false, s);
            arrow_head(p, tip, vec2(-(5.5f32).sin(), 5.5f32.cos()), r * 0.3, s);
        }
        Icon::Height => {
            p.line_segment([c + vec2(0.0, -r * 0.8), c + vec2(0.0, r * 0.8)], s);
            arrow_head(p, c + vec2(0.0, -r * 0.8), vec2(0.0, -1.0), r * 0.3, s);
            arrow_head(p, c + vec2(0.0, r * 0.8), vec2(0.0, 1.0), r * 0.3, s);
            p.line_segment([c + vec2(-r * 0.6, r * 0.85), c + vec2(r * 0.6, r * 0.85)], Stroke::new(w * 0.7, color));
        }
        Icon::Scale => {
            p.rect_stroke(Rect::from_center_size(c + vec2(-r * 0.2, r * 0.2), vec2(r * 0.8, r * 0.8)), 0.0, s);
            p.line_segment([c + vec2(0.0, 0.0), c + vec2(r * 0.75, -r * 0.75)], s);
            arrow_head(p, c + vec2(r * 0.75, -r * 0.75), vec2(1.0, -1.0), r * 0.3, s);
        }
        Icon::Delete => {
            p.line_segment([c + vec2(-r * 0.7, -r * 0.55), c + vec2(r * 0.7, -r * 0.55)], s);
            p.line_segment([c + vec2(-r * 0.2, -r * 0.75), c + vec2(r * 0.2, -r * 0.75)], s);
            poly(p, vec![c + vec2(-r * 0.5, -r * 0.45), c + vec2(-r * 0.4, r * 0.8), c + vec2(r * 0.4, r * 0.8), c + vec2(r * 0.5, -r * 0.45)], false, s);
            for x in [-0.18, 0.18] {
                p.line_segment([c + vec2(r * x, -r * 0.25), c + vec2(r * x, r * 0.55)], Stroke::new(w * 0.7, color));
            }
        }
        Icon::Light => {
            p.circle_stroke(c + vec2(0.0, -r * 0.2), r * 0.48, s);
            p.line_segment([c + vec2(-r * 0.22, r * 0.45), c + vec2(r * 0.22, r * 0.45)], s);
            p.line_segment([c + vec2(-r * 0.16, r * 0.68), c + vec2(r * 0.16, r * 0.68)], s);
        }
        Icon::Person => {
            p.circle_stroke(c + vec2(0.0, -r * 0.5), r * 0.25, s);
            poly(p, vec![c + vec2(-r * 0.5, r * 0.85), c + vec2(-r * 0.45, r * 0.05), c + vec2(r * 0.45, r * 0.05), c + vec2(r * 0.5, r * 0.85)], false, s);
        }
        Icon::Outdoor => {
            p.line_segment([c + vec2(-r * 0.8, -r * 0.05), c + vec2(r * 0.8, -r * 0.05)], s);
            p.line_segment([c + vec2(-r * 0.8, r * 0.25), c + vec2(r * 0.8, r * 0.25)], s);
            for x in [-0.6, 0.6] {
                p.line_segment([c + vec2(r * x, -r * 0.45), c + vec2(r * x, r * 0.8)], s);
            }
        }
        Icon::Eye => {
            let top: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    c + vec2((t - 0.5) * r * 1.7, -(t * std::f32::consts::PI).sin() * r * 0.55)
                })
                .collect();
            let bot: Vec<Pos2> = top.iter().map(|q| pos2(q.x, c.y + (c.y - q.y))).collect();
            poly(p, top, false, s);
            poly(p, bot, false, s);
            p.circle_filled(c, r * 0.22, color);
        }
        Icon::Folder => {
            poly(
                p,
                vec![
                    c + vec2(-r * 0.8, -r * 0.5),
                    c + vec2(-r * 0.3, -r * 0.5),
                    c + vec2(-r * 0.15, -r * 0.3),
                    c + vec2(r * 0.8, -r * 0.3),
                    c + vec2(r * 0.8, r * 0.6),
                    c + vec2(-r * 0.8, r * 0.6),
                ],
                true,
                s,
            );
        }
        Icon::Undo => {
            let pts: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = -2.6 + 3.4 * i as f32 / 12.0;
                    c + vec2(a.cos(), a.sin()) * r * 0.55 + vec2(0.0, r * 0.15)
                })
                .collect();
            let start = pts[0];
            poly(p, pts, false, s);
            arrow_head(p, start, vec2(-0.3, 1.0), r * 0.3, s);
        }
        Icon::Effects => {
            // sparkle
            for (d, l) in [(vec2(0.0, 1.0), 0.8), (vec2(1.0, 0.0), 0.8), (vec2(0.7, 0.7), 0.4), (vec2(-0.7, 0.7), 0.4)] {
                p.line_segment([c - d * r * l, c + d * r * l], s);
            }
        }
        Icon::Home => {
            poly(p, vec![c + vec2(-r * 0.8, -r * 0.05), c + vec2(0.0, -r * 0.8), c + vec2(r * 0.8, -r * 0.05)], false, s);
            poly(p, vec![c + vec2(-r * 0.6, -r * 0.2), c + vec2(-r * 0.6, r * 0.75), c + vec2(r * 0.6, r * 0.75), c + vec2(r * 0.6, -r * 0.2)], false, s);
        }
        Icon::Brush => {
            poly(p, vec![c + vec2(r * 0.75, -r * 0.8), c + vec2(r * 0.85, -r * 0.7), c + vec2(-r * 0.1, r * 0.3), c + vec2(-r * 0.3, r * 0.1)], true, s);
            poly(
                p,
                vec![
                    c + vec2(-r * 0.3, r * 0.1),
                    c + vec2(-r * 0.75, r * 0.4),
                    c + vec2(-r * 0.85, r * 0.85),
                    c + vec2(-r * 0.4, r * 0.75),
                    c + vec2(-r * 0.1, r * 0.3),
                ],
                false,
                s,
            );
        }
        Icon::WeatherCloud => {
            p.circle_stroke(c + vec2(-r * 0.2, -r * 0.25), r * 0.3, s);
            for i in 0..5 {
                let a = -2.6 + i as f32 * 0.75;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + vec2(-r * 0.2, -r * 0.25) + d * r * 0.45, c + vec2(-r * 0.2, -r * 0.25) + d * r * 0.65], s);
            }
            let cl: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0;
                    let a = std::f32::consts::PI * (1.0 + t);
                    c + vec2(r * 0.3, r * 0.35) + vec2(a.cos() * r * 0.55, a.sin() * r * 0.38)
                })
                .collect();
            poly(p, cl, false, s);
            p.line_segment([c + vec2(-r * 0.25, r * 0.35), c + vec2(r * 0.85, r * 0.35)], s);
        }
        Icon::Car => {
            poly(
                p,
                vec![
                    c + vec2(-r * 0.85, r * 0.35),
                    c + vec2(-r * 0.85, -r * 0.05),
                    c + vec2(-r * 0.55, -r * 0.15),
                    c + vec2(-r * 0.35, -r * 0.55),
                    c + vec2(r * 0.35, -r * 0.55),
                    c + vec2(r * 0.55, -r * 0.15),
                    c + vec2(r * 0.85, -r * 0.05),
                    c + vec2(r * 0.85, r * 0.35),
                ],
                true,
                s,
            );
            p.circle_stroke(c + vec2(-r * 0.45, r * 0.45), r * 0.17, s);
            p.circle_stroke(c + vec2(r * 0.45, r * 0.45), r * 0.17, s);
        }
        Icon::Chair => {
            poly(p, vec![c + vec2(-r * 0.4, -r * 0.8), c + vec2(-r * 0.4, r * 0.1), c + vec2(r * 0.45, r * 0.1), c + vec2(r * 0.45, r * 0.8)], false, s);
            p.line_segment([c + vec2(-r * 0.4, r * 0.1), c + vec2(-r * 0.4, r * 0.8)], s);
            p.line_segment([c + vec2(-r * 0.4, -r * 0.35), c + vec2(r * 0.2, -r * 0.35)], s);
        }
        Icon::Bench => {
            p.line_segment([c + vec2(-r * 0.85, -r * 0.05), c + vec2(r * 0.85, -r * 0.05)], s);
            p.line_segment([c + vec2(-r * 0.85, -r * 0.45), c + vec2(r * 0.85, -r * 0.45)], s);
            for x in [-0.6, 0.6] {
                p.line_segment([c + vec2(r * x, -r * 0.45), c + vec2(r * x, r * 0.7)], s);
            }
        }
        Icon::Group => {
            p.rect_stroke(Rect::from_center_size(c, vec2(r * 1.4, r * 1.4)), r * 0.15, s);
            p.circle_filled(c, r * 0.18, color);
        }
        Icon::Grid => {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                p.rect_stroke(Rect::from_center_size(c + vec2(r * 0.38 * x, r * 0.38 * y), vec2(r * 0.6, r * 0.6)), r * 0.08, s);
            }
        }
        Icon::SelectBox => {
            let b = Rect::from_center_size(c, vec2(r * 1.6, r * 1.6));
            for (a, d1, d2) in [
                (b.left_top(), vec2(1.0, 0.0), vec2(0.0, 1.0)),
                (b.right_top(), vec2(-1.0, 0.0), vec2(0.0, 1.0)),
                (b.left_bottom(), vec2(1.0, 0.0), vec2(0.0, -1.0)),
                (b.right_bottom(), vec2(-1.0, 0.0), vec2(0.0, -1.0)),
            ] {
                poly(p, vec![a + d1 * r * 0.45, a, a + d2 * r * 0.45], false, s);
            }
            p.line_segment([c + vec2(-r * 0.3, -r * 0.3), c + vec2(r * 0.25, r * 0.25)], s);
            arrow_head(p, c + vec2(-r * 0.3, -r * 0.3), vec2(-1.0, -1.0), r * 0.25, s);
        }
        Icon::PlaceDown | Icon::Import => {
            p.line_segment([c + vec2(0.0, -r * 0.85), c + vec2(0.0, r * 0.25)], s);
            arrow_head(p, c + vec2(0.0, r * 0.25), vec2(0.0, 1.0), r * 0.35, s);
            if icon == Icon::PlaceDown {
                poly(
                    p,
                    vec![c + vec2(-r * 0.85, r * 0.45), c + vec2(-r * 0.55, r * 0.75), c + vec2(r * 0.55, r * 0.75), c + vec2(r * 0.85, r * 0.45)],
                    false,
                    s,
                );
            } else {
                poly(p, vec![c + vec2(-r * 0.8, r * 0.3), c + vec2(-r * 0.8, r * 0.8), c + vec2(r * 0.8, r * 0.8), c + vec2(r * 0.8, r * 0.3)], false, s);
            }
        }
        Icon::Dot => {
            p.circle_filled(c, r * 0.2, color);
        }
        Icon::Pencil => {
            poly(
                p,
                vec![
                    c + vec2(r * 0.55, -r * 0.75),
                    c + vec2(r * 0.75, -r * 0.55),
                    c + vec2(-r * 0.45, r * 0.65),
                    c + vec2(-r * 0.75, r * 0.75),
                    c + vec2(-r * 0.65, r * 0.45),
                ],
                true,
                s,
            );
        }
        Icon::Circle => {
            p.circle_stroke(c, r * 0.55, s);
            p.circle_filled(c, r * 0.3, color);
        }
        Icon::Deselect => {
            let b = Rect::from_center_size(c, vec2(r * 1.5, r * 1.5));
            let dash = Stroke::new(w * 0.8, color);
            for i in 0..4 {
                let t0 = i as f32 / 4.0;
                let t1 = t0 + 0.14;
                for (a, bb) in
                    [(b.left_top(), b.right_top()), (b.right_top(), b.right_bottom()), (b.right_bottom(), b.left_bottom()), (b.left_bottom(), b.left_top())]
                {
                    p.line_segment([a + (bb - a) * t0, a + (bb - a) * t1], dash);
                }
            }
            p.line_segment([c + vec2(-r * 0.3, -r * 0.3), c + vec2(r * 0.3, r * 0.3)], s);
            p.line_segment([c + vec2(r * 0.3, -r * 0.3), c + vec2(-r * 0.3, r * 0.3)], s);
        }
        Icon::Search => {
            p.circle_stroke(c + vec2(-r * 0.15, -r * 0.15), r * 0.45, s);
            p.line_segment([c + vec2(r * 0.18, r * 0.18), c + vec2(r * 0.7, r * 0.7)], Stroke::new(w * 1.3, color));
        }
        Icon::EyeOff => {
            draw(p, Icon::Eye, rect, color);
            p.line_segment([c + vec2(-r * 0.75, -r * 0.6), c + vec2(r * 0.75, r * 0.6)], s);
        }
        Icon::Walker => {
            p.circle_stroke(c + vec2(-r * 0.25, -r * 0.6), r * 0.18, s);
            poly(
                p,
                vec![
                    c + vec2(-r * 0.7, r * 0.85),
                    c + vec2(-r * 0.35, r * 0.25),
                    c + vec2(-r * 0.2, -r * 0.3),
                    c + vec2(r * 0.05, r * 0.25),
                    c + vec2(r * 0.1, r * 0.85),
                ],
                false,
                s,
            );
            p.line_segment([c + vec2(-r * 0.2, -r * 0.2), c + vec2(r * 0.35, r * 0.0)], s);
            p.rect_stroke(Rect::from_center_size(c + vec2(r * 0.6, r * 0.35), vec2(r * 0.45, r * 0.7)), r * 0.05, s);
        }
        Icon::Film => {
            let b = Rect::from_center_size(c, vec2(r * 1.6, r * 1.3));
            p.rect_stroke(b, r * 0.1, s);
            p.line_segment([b.left_top() + vec2(r * 0.4, 0.0), b.left_bottom() + vec2(r * 0.4, 0.0)], s);
            p.line_segment([b.right_top() - vec2(r * 0.4, 0.0), b.right_bottom() - vec2(r * 0.4, 0.0)], s);
            p.line_segment([c + vec2(-r * 0.4, 0.0), c + vec2(r * 0.4, 0.0)], s);
        }
        Icon::Chat => {
            poly(
                p,
                vec![
                    c + vec2(-r * 0.8, -r * 0.6),
                    c + vec2(r * 0.8, -r * 0.6),
                    c + vec2(r * 0.8, r * 0.35),
                    c + vec2(-r * 0.2, r * 0.35),
                    c + vec2(-r * 0.55, r * 0.75),
                    c + vec2(-r * 0.55, r * 0.35),
                    c + vec2(-r * 0.8, r * 0.35),
                ],
                true,
                s,
            );
            for x in [-0.35, 0.0, 0.35] {
                p.circle_filled(c + vec2(r * x, -r * 0.12), r * 0.07, color);
            }
        }
        Icon::Gear => {
            p.circle_stroke(c, r * 0.25, s);
            let pts: Vec<Pos2> = (0..=48)
                .map(|i| {
                    let a = i as f32 / 48.0 * std::f32::consts::TAU;
                    let tooth = if (i / 3) % 2 == 0 { 0.78 } else { 0.58 };
                    c + vec2(a.cos(), a.sin()) * r * tooth
                })
                .collect();
            poly(p, pts, true, s);
        }
        Icon::Help => {
            p.circle_stroke(c, r * 0.8, s);
            let q: Vec<Pos2> = (0..=10)
                .map(|i| {
                    let a = -3.3 + 3.9 * i as f32 / 10.0;
                    c + vec2(0.0, -r * 0.2) + vec2(a.cos(), a.sin()) * r * 0.25
                })
                .collect();
            let end = *q.last().unwrap();
            poly(p, q, false, s);
            p.line_segment([end, c + vec2(0.0, r * 0.2)], s);
            p.circle_filled(c + vec2(0.0, r * 0.45), r * 0.07, color);
        }
        Icon::Power => {
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = -1.1 + (std::f32::consts::TAU - 1.0) * i as f32 / 20.0 - std::f32::consts::FRAC_PI_2 + 0.6;
                    c + vec2(a.cos(), a.sin()) * r * 0.62
                })
                .collect();
            poly(p, pts, false, s);
            p.line_segment([c + vec2(0.0, -r * 0.8), c + vec2(0.0, -r * 0.1)], s);
        }
        Icon::Copy | Icon::Paste => {
            p.rect_stroke(Rect::from_center_size(c + vec2(r * 0.15, r * 0.15), vec2(r * 1.0, r * 1.2)), r * 0.12, s);
            poly(p, vec![c + vec2(-r * 0.6, r * 0.45), c + vec2(-r * 0.6, -r * 0.7), c + vec2(r * 0.35, -r * 0.7)], false, s);
            if icon == Icon::Paste {
                p.line_segment([c + vec2(-r * 0.1, r * 0.15), c + vec2(r * 0.4, r * 0.15)], s);
            }
        }
        Icon::Kebab => {
            for y in [-0.5, 0.0, 0.5] {
                p.circle_filled(c + vec2(0.0, r * y), r * 0.11, color);
            }
        }
        Icon::Scan => {
            let b = Rect::from_center_size(c, vec2(r * 1.6, r * 1.6));
            for (a, d1, d2) in [
                (b.left_top(), vec2(1.0, 0.0), vec2(0.0, 1.0)),
                (b.right_top(), vec2(-1.0, 0.0), vec2(0.0, 1.0)),
                (b.left_bottom(), vec2(1.0, 0.0), vec2(0.0, -1.0)),
                (b.right_bottom(), vec2(-1.0, 0.0), vec2(0.0, -1.0)),
            ] {
                poly(p, vec![a + d1 * r * 0.4, a, a + d2 * r * 0.4], false, s);
            }
            p.circle_stroke(c + vec2(-r * 0.05, -r * 0.05), r * 0.3, s);
            p.line_segment([c + vec2(r * 0.18, r * 0.18), c + vec2(r * 0.45, r * 0.45)], s);
        }
        Icon::Sliders => {
            for (x, k) in [(-0.5, -0.2), (0.0, 0.35), (0.5, -0.4)] {
                p.line_segment([c + vec2(r * x, -r * 0.75), c + vec2(r * x, r * 0.75)], s);
                p.line_segment([c + vec2(r * x - r * 0.18, r * k), c + vec2(r * x + r * 0.18, r * k)], Stroke::new(w * 1.6, color));
            }
        }
        Icon::History => {
            let pts: Vec<Pos2> = (0..=18)
                .map(|i| {
                    let a = -2.4 + 5.2 * i as f32 / 18.0;
                    c + vec2(a.cos(), a.sin()) * r * 0.65
                })
                .collect();
            let st = pts[0];
            poly(p, pts, false, s);
            arrow_head(p, st, vec2(0.5, -1.0), r * 0.25, s);
            poly(p, vec![c + vec2(0.0, -r * 0.35), c, c + vec2(r * 0.3, r * 0.2)], false, s);
        }
        Icon::Leaf => {
            let a: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    let x = -0.7 + 1.4 * t;
                    c + vec2(r * x, -(r * x)) + vec2(-1.0, -1.0) * (t * std::f32::consts::PI).sin() * r * 0.35
                })
                .collect();
            let a2: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    let x = -0.7 + 1.4 * t;
                    c + vec2(r * x, -(r * x)) + vec2(1.0, 1.0) * (t * std::f32::consts::PI).sin() * r * 0.35
                })
                .collect();
            poly(p, a, false, s);
            poly(p, a2, false, s);
            p.line_segment([c + vec2(-r * 0.85, r * 0.85), c + vec2(r * 0.3, -r * 0.3)], s);
        }
        Icon::Check => {
            poly(p, vec![c + vec2(-r * 0.65, 0.0), c + vec2(-r * 0.2, r * 0.45), c + vec2(r * 0.7, -r * 0.45)], false, Stroke::new(w * 1.3, color));
        }
        Icon::Close => {
            p.line_segment([c + vec2(-r * 0.55, -r * 0.55), c + vec2(r * 0.55, r * 0.55)], s);
            p.line_segment([c + vec2(r * 0.55, -r * 0.55), c + vec2(-r * 0.55, r * 0.55)], s);
        }
        Icon::Star | Icon::StarFilled => {
            let pts: Vec<Pos2> = (0..10)
                .map(|k| {
                    let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
                    let rad = if k % 2 == 0 { r * 0.85 } else { r * 0.38 };
                    c + vec2(a.cos(), a.sin()) * rad
                })
                .collect();
            if icon == Icon::StarFilled {
                p.add(egui::Shape::convex_polygon(pts.clone(), color, Stroke::NONE));
                // convex_polygon fills the hull; redraw the outline for a star look
            }
            poly(p, pts, true, s);
        }
        Icon::Lock | Icon::Unlock => {
            p.rect_stroke(Rect::from_center_size(c + vec2(0.0, r * 0.3), vec2(r * 1.1, r * 0.85)), r * 0.12, s);
            let off = if icon == Icon::Unlock { r * 0.35 } else { 0.0 };
            let arc: Vec<Pos2> = (0..=10)
                .map(|i| {
                    let a = std::f32::consts::PI + std::f32::consts::PI * i as f32 / 10.0;
                    c + vec2(off, -r * 0.2) + vec2(a.cos(), a.sin()) * r * 0.35
                })
                .collect();
            let first = arc[0];
            let last = *arc.last().unwrap();
            poly(p, arc, false, s);
            p.line_segment([first, first + vec2(0.0, r * 0.15)], s);
            if icon == Icon::Lock {
                p.line_segment([last, last + vec2(0.0, r * 0.15)], s);
            }
        }
        Icon::Minimize => {
            p.line_segment([c + vec2(-r * 0.5, 0.0), c + vec2(r * 0.5, 0.0)], Stroke::new(1.2_f32, color));
        }
        Icon::Maximize => {
            p.rect_stroke(Rect::from_center_size(c, vec2(r * 1.0, r * 1.0)), 0.0, Stroke::new(1.2_f32, color));
        }
        Icon::Restore => {
            p.rect_stroke(Rect::from_center_size(c + vec2(-r * 0.12, r * 0.12), vec2(r * 0.8, r * 0.8)), 0.0, Stroke::new(1.2_f32, color));
            poly(
                p,
                vec![
                    c + vec2(-r * 0.25, -r * 0.3),
                    c + vec2(-r * 0.25, -r * 0.5),
                    c + vec2(r * 0.5, -r * 0.5),
                    c + vec2(r * 0.5, r * 0.25),
                    c + vec2(r * 0.3, r * 0.25),
                ],
                false,
                Stroke::new(1.2_f32, color),
            );
        }
        Icon::ArrowUp => {
            poly(p, vec![c + vec2(r * 0.6, r * 0.7), c + vec2(-r * 0.4, r * 0.7), c + vec2(-r * 0.4, -r * 0.6)], false, s);
            arrow_head(p, c + vec2(-r * 0.4, -r * 0.6), vec2(0.0, -1.0), r * 0.3, s);
        }
        Icon::ChevronUp => poly(p, vec![c + vec2(-r * 0.5, r * 0.25), c + vec2(0.0, -r * 0.25), c + vec2(r * 0.5, r * 0.25)], false, s),
        Icon::ChevronDown => poly(p, vec![c + vec2(-r * 0.5, -r * 0.25), c + vec2(0.0, r * 0.25), c + vec2(r * 0.5, -r * 0.25)], false, s),
        Icon::ChevronRight => poly(p, vec![c + vec2(-r * 0.25, -r * 0.5), c + vec2(r * 0.25, 0.0), c + vec2(-r * 0.25, r * 0.5)], false, s),
        Icon::Logo => {
            // Render Mill: a windmill - tapered tower and four sails.
            let hub = c + vec2(0.0, -r * 0.2);
            poly(
                p,
                vec![c + vec2(-r * 0.32, r * 0.9), hub + vec2(-r * 0.14, 0.0), hub + vec2(r * 0.14, 0.0), c + vec2(r * 0.32, r * 0.9)],
                true,
                Stroke::new(w * 1.2, color),
            );
            for k in 0..4 {
                let a = std::f32::consts::FRAC_PI_4 + k as f32 * std::f32::consts::FRAC_PI_2;
                let d = vec2(a.cos(), a.sin());
                let n = vec2(-d.y, d.x);
                p.line_segment([hub, hub + d * r * 0.85], Stroke::new(w * 1.3, color));
                // Sail cloth along one side of each arm.
                p.line_segment([hub + d * r * 0.3 + n * r * 0.18, hub + d * r * 0.85 + n * r * 0.18], Stroke::new(w, color));
            }
        }
        Icon::Random => {
            // magic wand with sparkles
            p.line_segment([c + vec2(-r * 0.75, r * 0.75), c + vec2(r * 0.25, -r * 0.25)], Stroke::new(w * 1.3, color));
            for (pt, l) in [(vec2(0.45, -0.6), 0.22), (vec2(0.7, -0.05), 0.15), (vec2(-0.05, -0.75), 0.15)] {
                let q = c + pt * r;
                p.line_segment([q - vec2(0.0, r * l), q + vec2(0.0, r * l)], s);
                p.line_segment([q - vec2(r * l, 0.0), q + vec2(r * l, 0.0)], s);
            }
        }
    }
}

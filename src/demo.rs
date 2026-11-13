//! A small modern house so the app has something to show before a model is synced.

use crate::objects::{ObjectKind, PlacedObject};
use crate::project::World;
use crate::scene::{presets, Material, MeshData};
use glam::{Vec2, Vec3};

pub fn add_box(m: &mut MeshData, lo: Vec3, hi: Vec3) {
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, -Vec3::Z, Vec3::Y),
        (-Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (-Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (-Vec3::Z, -Vec3::X, Vec3::Y),
    ];
    let c = (lo + hi) * 0.5;
    let h = (hi - lo) * 0.5;
    for (n, u, v) in faces {
        let base = m.positions.len() as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = c + n * h + u * h * a + v * h * b;
            m.positions.push(p);
            m.normals.push(n);
            let uv = if n.y.abs() > 0.5 {
                Vec2::new(p.x, p.z)
            } else if n.x.abs() > 0.5 {
                Vec2::new(p.z, p.y)
            } else {
                Vec2::new(p.x, p.y)
            };
            m.uvs.push(uv);
        }
        m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

pub fn build_demo(world: &mut World) {
    let p = presets();
    let get = |n: &str| p.iter().find(|(k, _)| *k == n).unwrap().1.clone();
    let mats = [
        ("Concrete - Cast-in-Place", get("Concrete")),
        ("Plaster - White", get("White plaster")),
        ("Glass - Clear Glazing", get("Clear glass")),
        ("Wood - Cladding Oak", get("Oak wood")),
        ("Water - Pool", get("Pool water")),
        ("Metal - Black Frames", get("Black metal")),
        ("Paving - Stone", {
            let mut s = get("Concrete");
            s.base_color = [0.55, 0.52, 0.47, 1.0];
            s
        }),
    ];
    for (n, params) in mats {
        world.scene.set_material(Material { name: n.into(), params, texture: None, normal_texture: None, roughness_texture: None });
    }
    let mut meshes: Vec<(&str, MeshData)> = Vec::new();
    let mk = |mat: &str| MeshData { name: mat.into(), material: mat.into(), ..Default::default() };
    let v = Vec3::new;

    let mut concrete = mk("Concrete - Cast-in-Place");
    add_box(&mut concrete, v(-9.0, 0.0, -6.0), v(9.0, 0.35, 6.0)); // slab
    add_box(&mut concrete, v(-9.5, 6.4, -6.5), v(9.5, 6.8, 6.5)); // roof
    add_box(&mut concrete, v(1.0, 3.2, -6.0), v(9.0, 3.5, 6.0)); // first floor slab
    add_box(&mut concrete, v(-2.0, 0.0, 7.5), v(-1.4, 1.0, 14.0)); // garden wall

    let mut plaster = mk("Plaster - White");
    add_box(&mut plaster, v(-9.0, 0.35, -6.0), v(-8.7, 6.4, 6.0));
    add_box(&mut plaster, v(-9.0, 0.35, -6.0), v(9.0, 6.4, -5.7));
    add_box(&mut plaster, v(1.0, 3.5, 5.7), v(9.0, 6.4, 6.0));
    add_box(&mut plaster, v(8.7, 0.35, -6.0), v(9.0, 6.4, 1.0));

    let mut wood = mk("Wood - Cladding Oak");
    for i in 0..18 {
        let x = -8.6 + i as f32 * 0.5;
        add_box(&mut wood, v(x, 3.5, 5.8), v(x + 0.12, 6.4, 6.05));
    }
    add_box(&mut wood, v(-12.0, 0.0, 6.5), v(-3.0, 0.25, 14.5)); // deck

    let mut glass = mk("Glass - Clear Glazing");
    add_box(&mut glass, v(-8.7, 0.35, 5.9), v(1.0, 3.2, 5.95));
    add_box(&mut glass, v(8.95, 0.35, 1.0), v(9.0, 3.2, 6.0));
    add_box(&mut glass, v(-8.7, 3.5, 5.9), v(-8.65, 6.4, 5.95));

    let mut frames = mk("Metal - Black Frames");
    for i in 0..6 {
        let x = -8.7 + i as f32 * 1.94;
        add_box(&mut frames, v(x, 0.35, 5.85), v(x + 0.06, 3.2, 6.0));
    }
    add_box(&mut frames, v(-8.7, 3.1, 5.85), v(1.0, 3.2, 6.0));

    let mut paving = mk("Paving - Stone");
    add_box(&mut paving, v(-1.0, 0.0, 6.0), v(14.0, 0.12, 16.0));
    let mut pool_edge = mk("Concrete - Cast-in-Place");
    add_box(&mut pool_edge, v(1.5, 0.0, 8.5), v(12.5, 0.2, 8.8));
    add_box(&mut pool_edge, v(1.5, 0.0, 13.2), v(12.5, 0.2, 13.5));
    add_box(&mut pool_edge, v(1.5, 0.0, 8.5), v(1.8, 0.2, 13.5));
    add_box(&mut pool_edge, v(12.2, 0.0, 8.5), v(12.5, 0.2, 13.5));
    let mut water = mk("Water - Pool");
    add_box(&mut water, v(1.8, 0.1, 8.8), v(12.2, 0.14, 13.2));

    meshes.push(("concrete", concrete));
    meshes.push(("concrete2", pool_edge));
    meshes.push(("plaster", plaster));
    meshes.push(("wood", wood));
    meshes.push(("glass", glass));
    meshes.push(("frames", frames));
    meshes.push(("paving", paving));
    meshes.push(("water", water));
    for (id, m) in meshes {
        world.scene.upsert_mesh(format!("demo:{id}"), m);
    }

    let mut seed = 7;
    let mut obj = |kind, x: f32, z: f32, rot: f32, scale: f32| {
        seed += 13;
        PlacedObject::new(kind, [x, 0.0, z], rot, scale, seed)
    };
    world.objects = vec![
        obj(ObjectKind::Tree, -16.0, 4.0, 0.3, 1.1),
        obj(ObjectKind::Tree, 16.0, -4.0, 1.3, 1.0),
        obj(ObjectKind::Tree, -14.0, 18.0, 2.0, 0.9),
        obj(ObjectKind::Conifer, -18.0, -10.0, 0.0, 1.2),
        obj(ObjectKind::Conifer, -12.0, -14.0, 0.5, 1.0),
        obj(ObjectKind::Conifer, 14.0, -14.0, 0.9, 1.3),
        obj(ObjectKind::Shrub, -5.0, 16.5, 0.0, 1.0),
        obj(ObjectKind::Shrub, -3.5, 17.0, 1.0, 0.8),
        obj(ObjectKind::Shrub, 15.0, 8.0, 0.4, 1.1),
        obj(ObjectKind::Person, 3.0, 15.0, 2.6, 1.0),
        obj(ObjectKind::Person, -6.0, 9.0, 0.8, 1.0),
        obj(ObjectKind::StreetLamp, 14.5, 16.5, 0.0, 1.0),
        obj(ObjectKind::Boulder, 13.0, 3.0, 0.7, 1.0),
        obj(ObjectKind::Car, -11.0, 9.5, 0.5, 1.0),
        obj(ObjectKind::Bench, -8.0, 14.0, 0.2, 1.0),
        obj(ObjectKind::Planter, -2.0, 12.0, 0.0, 1.0),
    ];
    world.rebuild_objects();
    world.camera.position = [24.0, 5.0, 30.0];
    world.camera.look_at(Vec3::new(0.0, 2.5, 3.0));
}

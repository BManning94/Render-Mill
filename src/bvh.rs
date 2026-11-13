//! Bounding volume hierarchy for the path tracer: binned SAH build on the
//! CPU, flattened into a GPU-friendly node array.

use glam::Vec3;

/// 32-byte node: `lo.xyz, a` and `hi.xyz, b`. Interior: `a` = left child
/// (right = left + 1), `b` = 0. Leaf: `a` = first triangle, `b` = count.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Debug)]
pub struct Node {
    pub lo: [f32; 3],
    pub a: u32,
    pub hi: [f32; 3],
    pub b: u32,
}

pub struct Bvh {
    pub nodes: Vec<Node>,
    /// Triangle order: `order[i]` is the original index of the i-th leaf triangle.
    pub order: Vec<u32>,
    /// Deepest leaf (root = 1). Always <= `MAX_DEPTH`.
    pub depth: u32,
}

/// Levels split by the surface area heuristic; deeper subtrees are split at
/// the object median, which halves the triangle count every level. That
/// bounds the tree depth to `SAH_DEPTH + log2(triangles / LEAF)` - at most
/// 62 for 2^32 triangles - so the shader's fixed traversal stack
/// (`STACK_SIZE` in pathtrace.wgsl, must be >= MAX_DEPTH) can never overflow.
pub const SAH_DEPTH: u32 = 32;
pub const MAX_DEPTH: u32 = 63;

#[derive(Clone, Copy)]
struct Aabb {
    lo: Vec3,
    hi: Vec3,
}

impl Aabb {
    const EMPTY: Aabb = Aabb { lo: Vec3::splat(f32::MAX), hi: Vec3::splat(f32::MIN) };
    fn grow(&mut self, p: Vec3) {
        self.lo = self.lo.min(p);
        self.hi = self.hi.max(p);
    }
    fn merge(&mut self, o: &Aabb) {
        self.lo = self.lo.min(o.lo);
        self.hi = self.hi.max(o.hi);
    }
    fn area(&self) -> f32 {
        let d = (self.hi - self.lo).max(Vec3::ZERO);
        2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
    }
}

const BINS: usize = 16;
const LEAF: usize = 4;

/// Build over triangles given as three corner positions each.
pub fn build(tris: &[[Vec3; 3]]) -> Bvh {
    build_with(tris, SAH_DEPTH)
}

/// As `build`, with SAH used only for the first `sah_depth` levels.
pub fn build_with(tris: &[[Vec3; 3]], sah_depth: u32) -> Bvh {
    let n = tris.len();
    let mut boxes = Vec::with_capacity(n);
    let mut cents = Vec::with_capacity(n);
    for t in tris {
        let mut b = Aabb::EMPTY;
        for p in t {
            b.grow(*p);
        }
        cents.push((b.lo + b.hi) * 0.5);
        boxes.push(b);
    }
    let mut order: Vec<u32> = (0..n as u32).collect();
    let mut nodes: Vec<Node> = Vec::with_capacity(2 * n / LEAF + 1);
    nodes.push(Node { lo: [0.0; 3], a: 0, hi: [0.0; 3], b: 0 });
    // (node index, start, end, depth)
    let mut stack = vec![(0usize, 0usize, n, 1u32)];
    let mut max_depth = 1u32;
    while let Some((ni, s, e, depth)) = stack.pop() {
        max_depth = max_depth.max(depth);
        let mut bb = Aabb::EMPTY;
        let mut cb = Aabb::EMPTY;
        for &i in &order[s..e] {
            bb.merge(&boxes[i as usize]);
            cb.grow(cents[i as usize]);
        }
        let count = e - s;
        let make_leaf = |nodes: &mut Vec<Node>| {
            nodes[ni] = Node { lo: bb.lo.to_array(), a: s as u32, hi: bb.hi.to_array(), b: count as u32 };
        };
        if count <= LEAF || depth >= MAX_DEPTH {
            make_leaf(&mut nodes);
            continue;
        }
        // Binned SAH over the widest centroid axis.
        let ext = cb.hi - cb.lo;
        let axis = if ext.x >= ext.y && ext.x >= ext.z {
            0
        } else if ext.y >= ext.z {
            1
        } else {
            2
        };
        let (lo_c, w) = (cb.lo[axis], ext[axis]);
        let mut split = None;
        if depth >= sah_depth {
            // Object median on the widest axis: balanced, so depth stays bounded.
            let mid = s + count / 2;
            order[s..e].select_nth_unstable_by(count / 2, |&a, &b| cents[a as usize][axis].total_cmp(&cents[b as usize][axis]));
            let left = nodes.len();
            nodes.push(Node { lo: [0.0; 3], a: 0, hi: [0.0; 3], b: 0 });
            nodes.push(Node { lo: [0.0; 3], a: 0, hi: [0.0; 3], b: 0 });
            nodes[ni] = Node { lo: bb.lo.to_array(), a: left as u32, hi: bb.hi.to_array(), b: 0 };
            stack.push((left + 1, mid, e, depth + 1));
            stack.push((left, s, mid, depth + 1));
            continue;
        }
        if w > 1e-9 {
            let mut bins = [(Aabb::EMPTY, 0usize); BINS];
            let k = BINS as f32 / w;
            for &i in &order[s..e] {
                let b = (((cents[i as usize][axis] - lo_c) * k) as usize).min(BINS - 1);
                bins[b].0.merge(&boxes[i as usize]);
                bins[b].1 += 1;
            }
            let mut right_area = [0f32; BINS];
            let mut right_count = [0usize; BINS];
            let mut acc = Aabb::EMPTY;
            let mut cnt = 0;
            for b in (1..BINS).rev() {
                acc.merge(&bins[b].0);
                cnt += bins[b].1;
                right_area[b] = acc.area();
                right_count[b] = cnt;
            }
            let mut acc = Aabb::EMPTY;
            let mut cnt = 0;
            let mut best = (f32::MAX, 0usize);
            for b in 0..BINS - 1 {
                acc.merge(&bins[b].0);
                cnt += bins[b].1;
                if cnt == 0 || right_count[b + 1] == 0 {
                    continue;
                }
                let cost = acc.area() * cnt as f32 + right_area[b + 1] * right_count[b + 1] as f32;
                if cost < best.0 {
                    best = (cost, b + 1);
                }
            }
            let leaf_cost = bb.area() * count as f32;
            if (best.0 < leaf_cost || count > 16) && best.0 < f32::MAX {
                split = Some(lo_c + best.1 as f32 / k);
            }
        }
        let mid = match split {
            Some(pos) => {
                let seg = &mut order[s..e];
                let mut l = 0;
                for r in 0..seg.len() {
                    if cents[seg[r] as usize][axis] < pos {
                        seg.swap(l, r);
                        l += 1;
                    }
                }
                s + l
            }
            None => s + count / 2,
        };
        let mid = if mid == s || mid == e { s + count / 2 } else { mid };
        if split.is_none() && count <= 16 {
            make_leaf(&mut nodes);
            continue;
        }
        let left = nodes.len();
        nodes.push(Node { lo: [0.0; 3], a: 0, hi: [0.0; 3], b: 0 });
        nodes.push(Node { lo: [0.0; 3], a: 0, hi: [0.0; 3], b: 0 });
        nodes[ni] = Node { lo: bb.lo.to_array(), a: left as u32, hi: bb.hi.to_array(), b: 0 };
        stack.push((left + 1, mid, e, depth + 1));
        stack.push((left, s, mid, depth + 1));
    }
    Bvh { nodes, order, depth: max_depth }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn ray_tri(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<f32> {
        let e1 = t[1] - t[0];
        let e2 = t[2] - t[0];
        let p = d.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            return None;
        }
        let inv = 1.0 / det;
        let s = o - t[0];
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(e1);
        let v = d.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = e2.dot(q) * inv;
        (t > 1e-6).then_some(t)
    }

    pub(super) fn traverse(b: &Bvh, tris: &[[Vec3; 3]], o: Vec3, d: Vec3) -> Option<f32> {
        let mut best: Option<f32> = None;
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let nd = &b.nodes[i];
            let (lo, hi) = (Vec3::from(nd.lo), Vec3::from(nd.hi));
            let inv = d.recip();
            let t0 = (lo - o) * inv;
            let t1 = (hi - o) * inv;
            let tn = t0.min(t1).max_element();
            let tf = t0.max(t1).min_element();
            if tf < tn.max(0.0) || best.is_some_and(|b| tn > b) {
                continue;
            }
            if nd.b > 0 {
                for k in nd.a..nd.a + nd.b {
                    if let Some(t) = ray_tri(o, d, &tris[b.order[k as usize] as usize]) {
                        if best.is_none_or(|bb| t < bb) {
                            best = Some(t);
                        }
                    }
                }
            } else {
                stack.push(nd.a as usize);
                stack.push(nd.a as usize + 1);
            }
        }
        best
    }

    #[test]
    fn bvh_matches_brute_force() {
        let mut seed = 7u32;
        let mut r = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed & 0xFFFFFF) as f32 / 16777216.0
        };
        let tris: Vec<[Vec3; 3]> = (0..5000)
            .map(|_| {
                let c = Vec3::new(r() * 50.0, r() * 10.0, r() * 50.0);
                [c, c + Vec3::new(r(), r(), r()), c + Vec3::new(r(), r(), r())]
            })
            .collect();
        let b = build(&tris);
        assert_eq!(b.order.len(), tris.len());
        let mut sorted = b.order.clone();
        sorted.sort();
        assert!(sorted.iter().enumerate().all(|(i, v)| *v == i as u32));
        for _ in 0..300 {
            let o = Vec3::new(r() * 60.0 - 5.0, 20.0, r() * 60.0 - 5.0);
            let d = Vec3::new(r() - 0.5, -1.0, r() - 0.5).normalize();
            let brute = tris.iter().filter_map(|t| ray_tri(o, d, t)).fold(None, |a: Option<f32>, t| Some(a.map_or(t, |a| a.min(t))));
            let fast = traverse(&b, &tris, o, d);
            assert_eq!(brute.is_some(), fast.is_some());
            if let (Some(a), Some(f)) = (brute, fast) {
                assert!((a - f).abs() < 1e-4);
            }
        }
    }
}

#[cfg(test)]
mod depth_tests {
    use super::*;

    /// Max depth by walking the tree (independent of `Bvh::depth`).
    fn walk_depth(b: &Bvh) -> u32 {
        let mut best = 0;
        let mut st = vec![(0usize, 1u32)];
        while let Some((i, d)) = st.pop() {
            best = best.max(d);
            let n = &b.nodes[i];
            if n.b == 0 {
                st.push((n.a as usize, d + 1));
                st.push((n.a as usize + 1, d + 1));
            }
        }
        best
    }

    /// Every triangle is referenced exactly once and every child bound
    /// is contained in its parent's.
    fn check_structure(b: &Bvh, tris: &[[Vec3; 3]]) {
        let mut seen = vec![0u8; tris.len()];
        let mut st = vec![0usize];
        while let Some(i) = st.pop() {
            let n = b.nodes[i];
            if n.b > 0 {
                for k in n.a..n.a + n.b {
                    let t = b.order[k as usize] as usize;
                    seen[t] += 1;
                    for p in tris[t] {
                        for ax in 0..3 {
                            assert!(p[ax] >= n.lo[ax] - 1e-4 && p[ax] <= n.hi[ax] + 1e-4, "triangle outside its leaf box");
                        }
                    }
                }
            } else {
                for c in [n.a as usize, n.a as usize + 1] {
                    let ch = b.nodes[c];
                    for ax in 0..3 {
                        assert!(ch.lo[ax] >= n.lo[ax] - 1e-4 && ch.hi[ax] <= n.hi[ax] + 1e-4, "child box outside parent");
                    }
                    st.push(c);
                }
            }
        }
        assert!(seen.iter().all(|&c| c == 1), "triangles missing or duplicated");
    }

    #[test]
    fn adversarial_inputs_stay_within_the_shader_stack() {
        // Exponentially spaced tiny triangles.
        let exp: Vec<[Vec3; 3]> = (0..100_000)
            .map(|i| {
                let c = Vec3::new(1.0001f32.powi(i) - 1.0, 0.0, 0.0);
                [c, c + Vec3::new(1e-4, 1e-4, 0.0), c + Vec3::new(0.0, 1e-4, 1e-4)]
            })
            .collect();
        // Clusters at geometrically growing distances (detail, house, site, terrain).
        let mut geo = Vec::new();
        for k in 0..90 {
            let base = 1.4f32.powi(k) * 1e-3;
            for j in 0..40 {
                let c = Vec3::new(base, j as f32 * base * 1e-3, 0.0);
                let s = base * 1e-3 + 1e-6;
                geo.push([c, c + Vec3::new(s, 0.0, 0.0), c + Vec3::new(0.0, s, s)]);
            }
        }
        // Many triangles with an identical centroid (stacked duplicate geometry).
        let dup: Vec<[Vec3; 3]> = (0..5000).map(|_| [Vec3::ZERO, Vec3::X, Vec3::Y]).collect();
        // One huge ground triangle plus fine detail.
        let mut big = vec![[Vec3::new(-1e4, 0.0, -1e4), Vec3::new(1e4, 0.0, -1e4), Vec3::new(0.0, 0.0, 1e4)]];
        for i in 0..50_000 {
            let c = Vec3::new((i % 300) as f32 * 0.01, (i / 300) as f32 * 0.01, 0.0);
            big.push([c, c + Vec3::new(0.005, 0.0, 0.0), c + Vec3::new(0.0, 0.005, 0.0)]);
        }
        for (name, tris) in [("exp", exp), ("geometric", geo), ("duplicates", dup), ("big+detail", big)] {
            let b = build(&tris);
            let d = walk_depth(&b);
            assert_eq!(d, b.depth, "{name}: reported depth");
            assert!(d <= MAX_DEPTH, "{name}: depth {d}");
            check_structure(&b, &tris);
        }
    }

    #[test]
    fn forced_median_splits_keep_the_tree_correct() {
        // A long chain that would be deep under SAH alone: verify the
        // median-split path builds a valid tree that still finds every hit.
        let tris: Vec<[Vec3; 3]> = (0..20_000)
            .map(|i| {
                let c = Vec3::new(i as f32 * 0.37 % 50.0, (i / 137) as f32 * 0.21, (i * 7 % 31) as f32);
                [c, c + Vec3::new(0.3, 0.0, 0.0), c + Vec3::new(0.0, 0.3, 0.1)]
            })
            .collect();
        let mut seed = 11u32;
        let mut r = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed & 0xFFFFFF) as f32 / 16777216.0
        };
        for sah_depth in [0, 3, 8] {
            let b = build_with(&tris, sah_depth);
            check_structure(&b, &tris);
            assert!(b.depth <= MAX_DEPTH);
            for _ in 0..400 {
                let o = Vec3::new(r() * 60.0 - 5.0, r() * 40.0, -10.0);
                let d = Vec3::new(r() - 0.5, r() - 0.5, 1.0).normalize();
                let brute = tris.iter().filter_map(|t| super::tests::ray_tri(o, d, t)).fold(None, |a: Option<f32>, t| Some(a.map_or(t, |a| a.min(t))));
                let fast = super::tests::traverse(&b, &tris, o, d);
                assert_eq!(brute.is_some(), fast.is_some(), "sah_depth {sah_depth}");
                if let (Some(a), Some(f)) = (brute, fast) {
                    assert!((a - f).abs() < 1e-4);
                }
            }
        }
    }
}

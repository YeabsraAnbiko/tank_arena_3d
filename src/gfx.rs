//! 3D rendering with macroquad. All models are built from shaded boxes and
//! spheres, so there are no model files to ship. The ground texture is
//! generated in code (pixel-art style, nearest-neighbour filtered).

use macroquad::prelude::*;

use crate::sim::*;

/// Simulation units per world unit.
pub const UNIT: f32 = 40.0;

pub fn w3(x: f32, y: f32, h: f32) -> Vec3 {
    vec3((x - ARENA_W / 2.0) / UNIT, h, (y - ARENA_H / 2.0) / UNIT)
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393)
        ^ (y as u32).wrapping_mul(668_265_263)
        ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0xFFFF) as f32 / 65535.0
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash(xi, yi, seed);
    let b = hash(xi + 1, yi, seed);
    let c = hash(xi, yi + 1, seed);
    let d = hash(xi + 1, yi + 1, seed);
    (a + (b - a) * sx) + ((c + (d - c) * sx) - (a + (b - a) * sx)) * sy
}

/// Pixel-art meadow: grass with mottled shading, dirt patches, tufts, flowers.
pub fn make_ground_texture() -> Texture2D {
    let (w, h) = ((ARENA_W / 2.0) as u16, (ARENA_H / 2.0) as u16);
    let mut img = Image::gen_image_color(w, h, GREEN);
    for py in 0..h as i32 {
        for px in 0..w as i32 {
            let (fx, fy) = (px as f32, py as f32);
            let grass = value_noise(fx / 9.0, fy / 9.0, 1) * 0.6 + value_noise(fx / 3.0, fy / 3.0, 2) * 0.4;
            let dirt = value_noise(fx / 28.0, fy / 28.0, 7);
            let speck = hash(px, py, 3);
            let mut c = if dirt > 0.66 {
                let t = (dirt - 0.66) / 0.34;
                let base = 0.40 + 0.08 * grass + 0.06 * speck;
                Color::new(base + 0.15 * (1.0 - t) * 0.0 + 0.10, base * 0.78 + 0.02, base * 0.45, 1.0)
            } else {
                let g = 0.34 + 0.22 * grass + 0.05 * speck;
                Color::new(g * 0.52, g, g * 0.38, 1.0)
            };
            // tufts and flowers
            if dirt <= 0.66 {
                let t = hash(px, py, 11);
                if t > 0.992 {
                    c = Color::new(0.16, 0.42, 0.14, 1.0);
                } else if t < 0.0016 {
                    c = Color::new(0.95, 0.92, 0.5, 1.0);
                } else if t > 0.9985 - 0.0004 && t < 0.9985 {
                    c = Color::new(0.9, 0.5, 0.65, 1.0);
                }
            }
            img.set_pixel(px as u32, py as u32, c);
        }
    }
    let tex = Texture2D::from_image(&img);
    tex.set_filter(FilterMode::Nearest);
    tex
}

// ------------------------------------------------------------ primitives ----

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

fn light(n: Vec3) -> f32 {
    let l = vec3(-0.35, 0.8, 0.5).normalize();
    (0.5 + 0.6 * n.normalize_or_zero().dot(l).max(0.0)).min(1.0)
}

fn shade(c: Color, f: f32) -> Color {
    Color::new((c.r * f).min(1.0), (c.g * f).min(1.0), (c.b * f).min(1.0), c.a)
}

/// Collects lit triangles from many shapes and draws them in a few calls.
pub struct Batch {
    v: Vec<Vertex>,
    i: Vec<u16>,
}

impl Batch {
    pub fn new() -> Batch {
        Batch { v: Vec::with_capacity(8192), i: Vec::with_capacity(16384) }
    }

    pub fn flush(&mut self) {
        if self.v.is_empty() {
            return;
        }
        draw_mesh(&Mesh { vertices: std::mem::take(&mut self.v), indices: std::mem::take(&mut self.i), texture: None });
    }

    /// macroquad silently truncates a single mesh beyond ~10k vertices /
    /// ~5k indices, so flush well before that.
    fn room(&mut self, extra: usize) {
        if self.v.len() + extra > 8000 || self.i.len() + extra * 3 > 4700 {
            self.flush();
        }
    }

    fn quad(&mut self, p: [Vec3; 4], col: Color) {
        self.room(4);
        let base = self.v.len() as u16;
        for (k, pt) in p.iter().enumerate() {
            let uv = [vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)][k];
            self.v.push(Vertex::new2(*pt, uv, col));
        }
        self.i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Box with local +X forward, +Y up, +Z right, oriented by `q`.
    pub fn box_q(&mut self, c: Vec3, size: Vec3, q: Quat, col: Color) {
        let h = size * 0.5;
        let (ax, ay, az) = (q * Vec3::X, q * Vec3::Y, q * Vec3::Z);
        let p = |sx: f32, sy: f32, sz: f32| c + ax * (sx * h.x) + ay * (sy * h.y) + az * (sz * h.z);
        let faces: [(Vec3, [Vec3; 4]); 6] = [
            (ay, [p(-1., 1., -1.), p(1., 1., -1.), p(1., 1., 1.), p(-1., 1., 1.)]),
            (-ay, [p(-1., -1., 1.), p(1., -1., 1.), p(1., -1., -1.), p(-1., -1., -1.)]),
            (ax, [p(1., -1., -1.), p(1., -1., 1.), p(1., 1., 1.), p(1., 1., -1.)]),
            (-ax, [p(-1., -1., 1.), p(-1., -1., -1.), p(-1., 1., -1.), p(-1., 1., 1.)]),
            (az, [p(1., -1., 1.), p(-1., -1., 1.), p(-1., 1., 1.), p(1., 1., 1.)]),
            (-az, [p(-1., -1., -1.), p(1., -1., -1.), p(1., 1., -1.), p(-1., 1., -1.)]),
        ];
        for (n, corners) in faces.iter() {
            self.quad(*corners, shade(col, light(*n)));
        }
    }

    /// Box rotated only around the vertical axis (`yaw` as in the simulation).
    pub fn box_y(&mut self, c: Vec3, size: Vec3, yaw: f32, col: Color) {
        self.box_q(c, size, Quat::from_rotation_y(-yaw), col);
    }

    /// Cylinder / wheel whose axis is local Z, `n` rim stripes that alternate
    /// between `a` and `b`. `rot` turns the stripes around the axis, which is
    /// what makes a rolling wheel visibly roll.
    pub fn disc(&mut self, c: Vec3, r: f32, thick: f32, q: Quat, a: Color, b: Color, rot: f32, n: usize) {
        self.room(n * 6 + 4);
        let az = q * Vec3::Z;
        let pt = |ang: f32, z: f32| c + q * vec3(r * ang.cos(), r * ang.sin(), z);
        // caps
        for (sign, col) in [(1.0f32, shade(a, light(az))), (-1.0, shade(a, light(-az)))] {
            let center = self.v.len() as u16;
            self.v.push(Vertex::new2(c + az * (sign * thick / 2.0), vec2(0.5, 0.5), col));
            for j in 0..n {
                let ang = rot + j as f32 * TAU / n as f32;
                self.v.push(Vertex::new2(pt(ang, sign * thick / 2.0), vec2(0.0, 0.0), col));
            }
            for j in 0..n {
                self.i.extend_from_slice(&[center, center + 1 + j as u16, center + 1 + ((j + 1) % n) as u16]);
            }
        }
        // rim
        for j in 0..n {
            let a0 = rot + j as f32 * TAU / n as f32;
            let a1 = rot + (j + 1) as f32 * TAU / n as f32;
            let mid = (a0 + a1) / 2.0;
            let normal = q * vec3(mid.cos(), mid.sin(), 0.0);
            let col = shade(if j % 2 == 0 { a } else { b }, light(normal));
            self.quad([pt(a0, -thick / 2.0), pt(a1, -thick / 2.0), pt(a1, thick / 2.0), pt(a0, thick / 2.0)], col);
        }
    }

    /// Upright cylinder (turret base, hatch, exhaust pipe).
    pub fn cyl(&mut self, c: Vec3, r: f32, h: f32, col: Color) {
        self.disc(c, r, h, Quat::from_rotation_x(FRAC_PI_2), col, shade(col, 0.9), 0.0, 14);
    }

    /// Smooth-shaded ellipsoid (turret dome, bullets). `sy` squashes it vertically.
    pub fn sphere(&mut self, c: Vec3, r: f32, sy: f32, col: Color) {
        let (lat, lon) = (6usize, 10usize);
        self.room((lat + 1) * (lon + 1));
        let base = self.v.len() as u16;
        for i in 0..=lat {
            let phi = -FRAC_PI_2 + PI * i as f32 / lat as f32;
            for j in 0..=lon {
                let th = TAU * j as f32 / lon as f32;
                let n = vec3(phi.cos() * th.cos(), phi.sin(), phi.cos() * th.sin());
                let pos = c + vec3(n.x * r, n.y * r * sy, n.z * r);
                self.v.push(Vertex::new2(pos, vec2(0., 0.), shade(col, light(n))));
            }
        }
        for i in 0..lat {
            for j in 0..lon {
                let a = base + (i * (lon + 1) + j) as u16;
                let b = a + (lon + 1) as u16;
                self.i.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
            }
        }
    }

    /// Flat soft shadow on the ground.
    pub fn shadow(&mut self, c: Vec3, rx: f32, rz: f32, yaw: f32) {
        self.room(20);
        let col = Color::new(0.0, 0.0, 0.0, 0.32);
        let center = self.v.len() as u16;
        self.v.push(Vertex::new2(c, vec2(0., 0.), col));
        let n = 14usize;
        let (s, co) = yaw.sin_cos();
        for j in 0..n {
            let a = TAU * j as f32 / n as f32;
            let (lx, lz) = (a.cos() * rx, a.sin() * rz);
            self.v.push(Vertex::new2(c + vec3(co * lx - s * lz, 0.0, s * lx + co * lz), vec2(0., 0.), col));
        }
        for j in 0..n {
            self.i.extend_from_slice(&[center, center + 1 + j as u16, center + 1 + ((j + 1) % n) as u16]);
        }
    }
}

pub fn team_color(team: u8, index_in_team: usize) -> Color {
    let k = 1.0 - 0.14 * index_in_team as f32;
    if team == 0 {
        Color::new(0.20 * k, 0.45 * k, 0.95 * k, 1.0)
    } else {
        Color::new(0.92 * k, 0.25 * k, 0.22 * k, 1.0)
    }
}

pub fn index_in_team(world: &World, id: u32) -> usize {
    let Some(t) = world.tank(id) else { return 0 };
    world.tanks.iter().filter(|o| o.team == t.team).position(|o| o.id == id).unwrap_or(0)
}

// ------------------------------------------------------------ track anim ----

/// How far each side's track has travelled, derived purely from how the
/// tank's position and heading change on screen. Because it only looks at
/// what is drawn, it works the same for local, AI and networked tanks:
/// driving rolls both tracks the same way, spinning in place rolls them in
/// opposite directions.
#[derive(Default)]
pub struct TrackAnim {
    map: HashMap<u32, TrackState>,
}

struct TrackState {
    x: f32,
    y: f32,
    a: f32,
    left: f32,
    right: f32,
}

const TRACK_HALF_GAP: f32 = 0.34;

fn wrap_pi(mut d: f32) -> f32 {
    while d > PI {
        d -= TAU;
    }
    while d < -PI {
        d += TAU;
    }
    d
}

impl TrackAnim {
    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn update(&mut self, world: &World) {
        for t in &world.tanks {
            match self.map.get_mut(&t.id) {
                Some(s) => {
                    if t.alive {
                        let (dx, dy) = (t.x - s.x, t.y - s.y);
                        let dth = wrap_pi(t.angle - s.a);
                        if dx.hypot(dy) < 80.0 {
                            let am = s.a + dth / 2.0;
                            let d = (dx * am.cos() + dy * am.sin()) / UNIT;
                            s.left += d + dth * TRACK_HALF_GAP;
                            s.right += d - dth * TRACK_HALF_GAP;
                        }
                    }
                    s.x = t.x;
                    s.y = t.y;
                    s.a = t.angle;
                }
                None => {
                    self.map.insert(t.id, TrackState { x: t.x, y: t.y, a: t.angle, left: 0.0, right: 0.0 });
                }
            }
        }
    }

    fn get(&self, id: u32) -> (f32, f32) {
        self.map.get(&id).map(|s| (s.left, s.right)).unwrap_or((0.0, 0.0))
    }
}

// ----------------------------------------------------------------- scene ----

pub struct Scene {
    pub ground: Texture2D,
}

impl Scene {
    pub fn new() -> Scene {
        Scene { ground: make_ground_texture() }
    }

    pub fn draw_world(&self, world: &World, time: f32, tracks: &TrackAnim, aim: Option<&(Vec<(f32, f32)>, bool)>, hide_pips_of: Option<u32>) {
        // ground
        draw_plane(vec3(0.0, -0.02, 0.0), vec2(40.0, 40.0), None, Color::new(0.10, 0.2, 0.09, 1.0));
        draw_plane(vec3(0.0, 0.0, 0.0), vec2(ARENA_W / UNIT / 2.0, ARENA_H / UNIT / 2.0), Some(&self.ground), WHITE);

        let mut b = Batch::new();
        self.draw_walls(&mut b);
        for (i, o) in world.obstacles.iter().enumerate() {
            self.draw_obstacle(&mut b, o, i);
        }
        for p in &world.pickups {
            self.draw_pickup(&mut b, p, time);
        }
        for t in world.tanks.iter() {
            let k = index_in_team(world, t.id);
            self.draw_tank(&mut b, t, k, time, tracks.get(t.id), hide_pips_of != Some(t.id));
        }
        for bl in &world.bullets {
            let pos = w3(bl.x, bl.y, 0.6);
            b.sphere(pos, 0.11, 1.0, Color::new(1.0, 0.85, 0.3, 1.0));
            let back = vec3(-bl.vx, 0.0, -bl.vy).normalize_or_zero() * 0.28;
            b.sphere(pos + back, 0.07, 1.0, Color::new(1.0, 0.6, 0.2, 1.0));
        }
        if let Some((pts, hit)) = aim {
            self.draw_aim_line(&mut b, pts, *hit);
        }
        b.flush();

        // translucent things last
        for p in &world.pickups {
            let (r, g, bl) = p.kind.rgb();
            draw_sphere(w3(p.x, p.y, 0.02), 0.5, None, Color::new(r, g, bl, 0.18));
        }
        for t in world.tanks.iter().filter(|t| t.alive && t.shield_t > 0.0) {
            let pulse = 0.22 + 0.08 * (time * 8.0).sin();
            draw_sphere(w3(t.x, t.y, 0.4), 0.78, None, Color::new(0.35, 0.75, 1.0, pulse));
        }
    }

    /// Dotted trajectory of the next shot, fading with each bounce, ending in
    /// a marker that turns red when the shot would land on an enemy.
    fn draw_aim_line(&self, b: &mut Batch, pts: &[(f32, f32)], hit: bool) {
        let mut carry = 0.0f32;
        for (leg, w) in pts.windows(2).enumerate() {
            let ((ax, ay), (bx, by)) = (w[0], w[1]);
            let len = (bx - ax).hypot(by - ay);
            let alpha = [0.95, 0.6, 0.4, 0.3][leg.min(3)];
            let mut d = carry;
            while d < len {
                let t = d / len;
                b.box_y(w3(ax + (bx - ax) * t, ay + (by - ay) * t, 0.62), vec3(0.06, 0.06, 0.06), 0.0, Color::new(1.0, 0.95, 0.5, alpha));
                d += 16.0;
            }
            carry = d - len;
        }
        if let Some(&(x, y)) = pts.last() {
            let col = if hit { Color::new(1.0, 0.25, 0.2, 1.0) } else { Color::new(1.0, 1.0, 1.0, 0.8) };
            b.sphere(w3(x, y, 0.62), if hit { 0.17 } else { 0.11 }, 1.0, col);
        }
    }

    fn draw_walls(&self, b: &mut Batch) {
        let (hw, hh) = (ARENA_W / UNIT / 2.0, ARENA_H / UNIT / 2.0);
        let c = Color::new(0.45, 0.33, 0.22, 1.0);
        let c2 = Color::new(0.38, 0.28, 0.19, 1.0);
        let t = 0.5;
        b.box_y(vec3(0.0, 0.3, -hh - t / 2.0), vec3(hw * 2.0 + t * 2.0, 0.6, t), 0.0, c);
        b.box_y(vec3(0.0, 0.3, hh + t / 2.0), vec3(hw * 2.0 + t * 2.0, 0.6, t), 0.0, c);
        b.box_y(vec3(-hw - t / 2.0, 0.3, 0.0), vec3(t, 0.6, hh * 2.0), 0.0, c2);
        b.box_y(vec3(hw + t / 2.0, 0.3, 0.0), vec3(t, 0.6, hh * 2.0), 0.0, c2);
    }

    fn draw_obstacle(&self, b: &mut Batch, o: &Obstacle, idx: usize) {
        let (cx, cy) = (o.x + o.w / 2.0, o.y + o.h / 2.0);
        let (w, d) = (o.w / UNIT, o.h / UNIT);
        match o.kind {
            ObstacleKind::Rock => {
                let shade_k = 0.85 + 0.2 * hash(idx as i32, 1, 5);
                let base = Color::new(0.50 * shade_k, 0.50 * shade_k, 0.53 * shade_k, 1.0);
                b.box_y(w3(cx, cy, 0.3), vec3(w, 0.6, d), 0.0, base);
                b.box_y(w3(cx, cy, 0.68), vec3(w * 0.78, 0.3, d * 0.78), 0.0, Color::new(0.58 * shade_k, 0.58 * shade_k, 0.6 * shade_k, 1.0));
                b.box_y(w3(cx - o.w * 0.12, cy + o.h * 0.1, 0.95), vec3(w * 0.4, 0.26, d * 0.42), 0.35, Color::new(0.64 * shade_k, 0.64 * shade_k, 0.66 * shade_k, 1.0));
                for k in 0..3 {
                    let a = hash(idx as i32, k, 9) * 6.28;
                    let r = 0.14 + 0.1 * hash(idx as i32, k, 4);
                    let px = cx + a.cos() * (o.w / 2.0 + 4.0);
                    let py = cy + a.sin() * (o.h / 2.0 + 4.0);
                    b.box_y(w3(px, py, r * 0.5), vec3(r * 1.6, r, r * 1.3), a, Color::new(0.42, 0.42, 0.45, 1.0));
                }
            }
            ObstacleKind::Sandbag => {
                let long_x = w >= d;
                let (len, thick) = if long_x { (w, d) } else { (d, w) };
                let n = ((len / 0.5).round() as i32).max(1);
                let cell = len / n as f32;
                for row in 0..2 {
                    let off = if row == 1 { cell / 2.0 } else { 0.0 };
                    let count = if row == 1 { n - 1 } else { n };
                    for i in 0..count.max(0) {
                        let along = -len / 2.0 + cell * (i as f32 + 0.5) + off;
                        let sh = 0.92 + 0.12 * hash(i, row + idx as i32 * 7, 6);
                        let col = Color::new(0.78 * sh, 0.66 * sh, 0.42 * sh, 1.0);
                        let (sx, sz) = if long_x { (cell - 0.04, thick - 0.04) } else { (thick - 0.04, cell - 0.04) };
                        let (px, pz) = if long_x { (along, 0.0) } else { (0.0, along) };
                        let base = w3(cx, cy, 0.0);
                        b.box_y(vec3(base.x + px, 0.12 + row as f32 * 0.22, base.z + pz), vec3(sx, 0.22, sz), 0.0, col);
                    }
                }
            }
        }
    }

    fn draw_pickup(&self, b: &mut Batch, p: &Pickup, time: f32) {
        let (r, g, bl) = p.kind.rgb();
        let col = Color::new(r, g, bl, 1.0);
        let bob = 0.1 * (time * 3.0 + p.id as f32).sin();
        b.box_y(w3(p.x, p.y, 0.5 + bob), vec3(0.38, 0.38, 0.38), time * 1.6, col);
        b.box_y(w3(p.x, p.y, 0.5 + bob), vec3(0.22, 0.46, 0.22), time * 1.6 + 0.8, Color::new(1.0, 1.0, 1.0, 1.0));
    }

    /// A chunky cartoon tank: rolling tracks on real wheels, sloped hull,
    /// fenders, dome turret with a muzzle-braked cannon, headlights,
    /// exhaust, and a waving team flag on the antenna.
    fn draw_tank(&self, b: &mut Batch, t: &Tank, k: usize, time: f32, tr: (f32, f32), pips: bool) {
        const S: f32 = 1.1; // model scale; collision radius stays 0.45
        let q = Quat::from_rotation_y(-t.angle);
        let base = w3(t.x, t.y, 0.0);
        let at = |l: Vec3| base + q * (l * S);
        let sz = |v: Vec3| v * S;

        b.shadow(base + vec3(0.0, 0.012, 0.0), 0.62 * S, 0.5 * S, t.angle);

        let body = team_color(t.team, k);
        let dark = Color::new(0.13, 0.13, 0.14, 1.0);
        let metal = Color::new(0.30, 0.31, 0.34, 1.0);
        let belt_a = Color::new(0.16, 0.16, 0.17, 1.0);
        let belt_b = Color::new(0.24, 0.24, 0.25, 1.0);

        if !t.alive {
            // burnt-out wreck: no animation, no team colour
            let char_ = Color::new(0.14, 0.14, 0.15, 1.0);
            b.box_q(at(vec3(0.0, 0.2, 0.0)), sz(vec3(0.95, 0.3, 0.7)), q, char_);
            b.box_q(at(vec3(0.1, 0.42, 0.0)), sz(vec3(0.4, 0.14, 0.34)), q * Quat::from_rotation_y(0.5), Color::new(0.2, 0.2, 0.2, 1.0));
            let flicker = 0.5 + 0.5 * (time * 12.0 + t.id as f32).sin();
            b.sphere(at(vec3(0.0, 0.55, 0.0)), 0.09 + 0.05 * flicker, 1.0, Color::new(1.0, 0.45, 0.1, 1.0));
            return;
        }

        // ---- tracks: wheels and belt, one side at a time ----
        let (half_len, wheel_r, belt_r) = (0.33f32, 0.15f32, 0.18f32);
        let straight = half_len * 2.0;
        let perim = 2.0 * straight + TAU * belt_r;
        let segs = 26usize;
        for (side, phase) in [(-1.0f32, tr.0), (1.0f32, tr.1)] {
            let z = side * TRACK_HALF_GAP;
            let spin = -phase / wheel_r; // forward travel rolls the wheel clockwise
            // belt links riding around the stadium path
            for j in 0..segs {
                let s = (j as f32 * perim / segs as f32 + phase).rem_euclid(perim);
                let (px, py, pitch) = if s < straight {
                    (-half_len + s, 2.0 * belt_r, 0.0) // top run, moving forward
                } else if s < straight + PI * belt_r {
                    let a = FRAC_PI_2 - (s - straight) / belt_r; // front arc, over the top and down
                    (half_len + belt_r * a.cos(), belt_r + belt_r * a.sin(), (-a.cos()).atan2(a.sin()))
                } else if s < 2.0 * straight + PI * belt_r {
                    (half_len - (s - straight - PI * belt_r), 0.0, 0.0) // bottom run, moving backward
                } else {
                    let a = -FRAC_PI_2 - (s - 2.0 * straight - PI * belt_r) / belt_r; // rear arc
                    (-half_len + belt_r * a.cos(), belt_r + belt_r * a.sin(), (-a.cos()).atan2(a.sin()))
                };
                let col = if j % 2 == 0 { belt_a } else { belt_b };
                let seg_len = perim / segs as f32 * 0.86;
                b.box_q(at(vec3(px, py, z)), sz(vec3(seg_len, 0.04, 0.2)), q * Quat::from_rotation_z(pitch), col);
            }
            // drive wheels (front/back) and small road wheels in between
            let mut wheels: Vec<(f32, f32, f32)> = vec![(half_len, belt_r, wheel_r), (-half_len, belt_r, wheel_r)];
            for x in [-0.165f32, 0.0, 0.165] {
                wheels.push((x, 0.105, 0.095));
            }
            for (wx, wy, r) in wheels {
                let ang = -phase / r;
                let wq = q * Quat::from_rotation_z(0.0);
                b.disc(at(vec3(wx, wy, z)), r * S, 0.17 * S, wq, Color::new(0.1, 0.1, 0.11, 1.0), Color::new(0.2, 0.2, 0.21, 1.0), ang, 10);
                // outer hub plate with three spokes that turn with the wheel
                let hz = z + side * 0.105;
                b.disc(at(vec3(wx, wy, hz)), r * 0.7 * S, 0.02 * S, wq, Color::new(0.68, 0.69, 0.72, 1.0), Color::new(0.58, 0.59, 0.62, 1.0), ang, 8);
                for sp in 0..3 {
                    let sq = q * Quat::from_rotation_z(ang + sp as f32 * PI / 3.0);
                    b.box_q(at(vec3(wx, wy, hz + side * 0.012)), sz(vec3(r * 1.35, 0.022, 0.016)), sq, Color::new(0.4, 0.41, 0.44, 1.0));
                }
            }
            let _ = spin;
            // fender over the track
            b.box_q(at(vec3(0.0, 0.405, z)), sz(vec3(0.96, 0.03, 0.24)), q, shade(body, 0.7));
        }

        // ---- hull ----
        b.box_q(at(vec3(0.0, 0.27, 0.0)), sz(vec3(0.86, 0.2, 0.46)), q, shade(body, 0.82));
        b.box_q(at(vec3(-0.04, 0.42, 0.0)), sz(vec3(0.7, 0.14, 0.42)), q, body);
        b.box_q(at(vec3(0.42, 0.37, 0.0)), sz(vec3(0.24, 0.12, 0.44)), q * Quat::from_rotation_z(-0.5), shade(body, 0.92)); // sloped nose
        b.box_q(at(vec3(-0.38, 0.5, 0.0)), sz(vec3(0.2, 0.05, 0.36)), q, shade(body, 0.7)); // engine deck
        for gz in [-0.1f32, 0.0, 0.1] {
            b.box_q(at(vec3(-0.38, 0.532, gz)), sz(vec3(0.15, 0.012, 0.03)), q, dark); // grille slats
        }
        b.box_q(at(vec3(-0.05, 0.495, 0.0)), sz(vec3(0.5, 0.012, 0.07)), q, Color::new(0.97, 0.97, 0.97, 1.0)); // racing stripe
        for lz in [-0.15f32, 0.15] {
            b.box_q(at(vec3(0.52, 0.31, lz)), sz(vec3(0.05, 0.07, 0.08)), q, Color::new(1.0, 0.95, 0.5, 1.0)); // headlights
        }
        b.cyl(at(vec3(-0.46, 0.5, -0.17)), 0.035 * S, 0.12 * S, dark); // exhaust

        // ---- turret and cannon (rotates on its own with the aim) ----
        let qt = Quat::from_rotation_y(-t.turret);
        let att = |l: Vec3| base + qt * (l * S);
        b.cyl(att(vec3(0.0, 0.52, 0.0)), 0.25 * S, 0.1 * S, shade(body, 0.85));
        b.sphere(att(vec3(-0.02, 0.57, 0.0)), 0.21 * S, 0.7, body);
        b.cyl(att(vec3(-0.1, 0.69, 0.08)), 0.065 * S, 0.035 * S, dark); // hatch
        b.box_q(att(vec3(0.12, 0.66, -0.07)), sz(vec3(0.06, 0.04, 0.04)), qt, Color::new(0.5, 0.8, 1.0, 1.0)); // periscope glass
        b.box_q(att(vec3(0.31, 0.6, 0.0)), sz(vec3(0.42, 0.085, 0.085)), qt, metal); // barrel
        b.box_q(att(vec3(0.55, 0.6, 0.0)), sz(vec3(0.08, 0.125, 0.125)), qt, dark); // muzzle brake

        // ---- antenna and team flag ----
        b.box_q(at(vec3(-0.42, 0.78, 0.2)), sz(vec3(0.015, 0.56, 0.015)), q, metal);
        let wave = (time * 7.0 + t.id as f32).sin() * 0.35;
        b.box_q(at(vec3(-0.42, 0.96, 0.2) + vec3(0.0, 0.0, 0.0)), sz(vec3(0.02, 0.1, 0.16)), q * Quat::from_rotation_y(wave), body);

        // ---- hit points as pips above the tank ----
        for i in 0..if pips { t.hp.max(0) } else { 0 } {
            let off = (i as f32 - (TANK_HP as f32 - 1.0) / 2.0) * 0.2;
            let pos = w3(t.x, t.y, 1.15) + vec3(off, 0.0, 0.0);
            b.box_y(pos, vec3(0.13, 0.13, 0.13), 0.0, Color::new(0.4, 1.0, 0.45, 1.0));
        }
    }
}

// --------------------------------------------------------------- particles ----

pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub life: f32,
    pub max: f32,
    pub color: Color,
    pub size: f32,
}

fn burst(out: &mut Vec<Particle>, at: Vec3, n: usize, speed: (f32, f32), life: (f32, f32), color: Color, size: f32, up: f32) {
    for _ in 0..n {
        let a = rand::gen_range(0.0, std::f32::consts::TAU);
        let s = rand::gen_range(speed.0, speed.1);
        let l = rand::gen_range(life.0, life.1);
        out.push(Particle {
            pos: at,
            vel: vec3(a.cos() * s, rand::gen_range(0.0, up), a.sin() * s),
            life: l,
            max: l,
            color,
            size,
        });
    }
}

pub fn spawn_fx(out: &mut Vec<Particle>, e: &Event) {
    match e.kind {
        EventKind::Fire => {
            let at = w3(e.x, e.y, 0.6);
            for _ in 0..7 {
                let a = e.a + rand::gen_range(-0.35, 0.35);
                let s = rand::gen_range(2.0, 4.5);
                let l = rand::gen_range(0.15, 0.3);
                out.push(Particle { pos: at, vel: vec3(a.cos() * s, rand::gen_range(0.0, 0.8), a.sin() * s), life: l, max: l, color: Color::new(1.0, 0.8, 0.3, 1.0), size: 0.07 });
            }
        }
        EventKind::Bounce => burst(out, w3(e.x, e.y, 0.5), 6, (1.0, 3.0), (0.15, 0.3), Color::new(1.0, 1.0, 0.6, 1.0), 0.05, 2.0),
        EventKind::Hit => burst(out, w3(e.x, e.y, 0.55), 12, (1.5, 4.0), (0.2, 0.45), Color::new(1.0, 0.55, 0.2, 1.0), 0.07, 3.0),
        EventKind::Explode => {
            let at = w3(e.x, e.y, 0.5);
            burst(out, at, 28, (1.0, 6.0), (0.4, 0.9), Color::new(1.0, 0.5, 0.15, 1.0), 0.14, 5.0);
            burst(out, at, 14, (0.5, 2.5), (0.8, 1.5), Color::new(0.25, 0.25, 0.25, 1.0), 0.18, 3.0);
        }
        EventKind::Pickup => {
            let kind = PickupKind::from_code(e.a as u8).unwrap_or(PickupKind::Heal);
            let (r, g, b) = kind.rgb();
            burst(out, w3(e.x, e.y, 0.5), 18, (1.0, 3.0), (0.4, 0.8), Color::new(r, g, b, 1.0), 0.08, 4.0);
        }
    }
}

pub fn update_particles(ps: &mut Vec<Particle>, dt: f32) {
    for p in ps.iter_mut() {
        p.vel.y -= 9.0 * dt;
        p.pos += p.vel * dt;
        if p.pos.y < 0.03 {
            p.pos.y = 0.03;
            p.vel.y = 0.0;
            p.vel.x *= 0.9;
            p.vel.z *= 0.9;
        }
        p.life -= dt;
    }
    ps.retain(|p| p.life > 0.0);
}

pub fn draw_particles(ps: &[Particle]) {
    for p in ps {
        let a = (p.life / p.max).clamp(0.0, 1.0);
        let s = p.size * (0.4 + a);
        draw_cube(p.pos, vec3(s, s, s), None, Color::new(p.color.r, p.color.g, p.color.b, a));
    }
}

// ------------------------------------------------------------------ camera ----

pub struct CamState {
    pub chase: bool,
    pub yaw: f32,
}

impl CamState {
    pub fn new(chase: bool) -> Self {
        CamState { chase, yaw: 0.0 }
    }

    pub fn camera(&mut self, world: &World, my_id: u32, _dt: f32) -> Camera3D {
        let aspect = screen_width() / screen_height();
        let me = world.tank(my_id).filter(|t| t.alive);
        if self.chase {
            if let Some(t) = me {
                // Over-the-shoulder: the camera looks where the turret aims
                // (`self.yaw` is the aim angle, set by the input code).
                let (sn, c) = self.yaw.sin_cos();
                let tp = w3(t.x, t.y, 0.0);
                let right = vec3(-sn, 0.0, c) * 0.7;
                // Camera collision: pull the boom in when cover is in the way.
                let at = |d: f32| tp + vec3(-c * d, 0.0, -sn * d) + right * (d / 4.4);
                let mut dist = 4.4f32;
                for i in 1..=22 {
                    let d = i as f32 * 0.2;
                    let p = at(d);
                    let (sx, sy) = (p.x * UNIT + ARENA_W / 2.0, p.z * UNIT + ARENA_H / 2.0);
                    let blocked = world
                        .obstacles
                        .iter()
                        .any(|o| sx > o.x - 18.0 && sx < o.x + o.w + 18.0 && sy > o.y - 18.0 && sy < o.y + o.h + 18.0);
                    if blocked {
                        dist = ((i - 1) as f32 * 0.2).max(2.0);
                        break;
                    }
                }
                let base = at(dist);
                let height = 1.9 + 1.2 * (dist / 4.4); // always above the cover
                return Camera3D {
                    position: vec3(base.x, height, base.z),
                    target: tp + vec3(c * 4.0, 0.2, sn * 4.0) + right,
                    up: vec3(0.0, 1.0, 0.0),
                    fovy: 55f32.to_radians(),
                    ..Default::default()
                };
            }
        }
        // Overview: tilted view that fits the whole arena for any aspect ratio.
        let fovy = 45f32.to_radians();
        let tan = (fovy / 2.0).tan();
        let tilt = 58f32.to_radians();
        let dist = (12.6 / (tan * aspect)).max(10.2 / tan).max(12.0);
        Camera3D {
            position: vec3(0.0, tilt.sin() * dist, tilt.cos() * dist),
            target: vec3(0.0, 0.0, 0.3),
            up: vec3(0.0, 1.0, 0.0),
            fovy,
            ..Default::default()
        }
    }
}

/// Project a world position to screen pixels. None if behind the camera.
pub fn project(cam: &Camera3D, p: Vec3) -> Option<Vec2> {
    let aspect = screen_width() / screen_height();
    let proj = Mat4::perspective_rh_gl(cam.fovy, aspect, 0.01, 10000.0);
    let view = Mat4::look_at_rh(cam.position, cam.target, cam.up);
    let c = proj * view * vec4(p.x, p.y, p.z, 1.0);
    if c.w <= 0.0 {
        return None;
    }
    let (nx, ny) = (c.x / c.w, c.y / c.w);
    Some(vec2((nx + 1.0) * 0.5 * screen_width(), (1.0 - ny) * 0.5 * screen_height()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        World::new_match(&[PlayerSpec { id: 0, name: "a".into(), team: 0, ai: false }], 1)
    }

    #[test]
    fn tracks_roll_together_when_driving_forward() {
        let mut w = world();
        let mut tr = TrackAnim::default();
        tr.update(&w);
        for _ in 0..30 {
            let t = w.tank_mut(0).unwrap();
            t.x += t.angle.cos() * 2.0;
            t.y += t.angle.sin() * 2.0;
            tr.update(&w);
        }
        let (l, r) = tr.get(0);
        assert!(l > 0.5 && (l - r).abs() < 1e-3, "l={l} r={r}");
    }

    #[test]
    fn tracks_counter_roll_when_spinning_in_place() {
        let mut w = world();
        let mut tr = TrackAnim::default();
        tr.update(&w);
        for _ in 0..30 {
            w.tank_mut(0).unwrap().angle += 0.05; // clockwise turn, no travel
            tr.update(&w);
        }
        let (l, r) = tr.get(0);
        assert!(l > 0.1 && r < -0.1 && (l + r).abs() < 1e-3, "l={l} r={r}");
    }

    #[test]
    fn tracks_stand_still_when_parked_or_dead() {
        let mut w = world();
        let mut tr = TrackAnim::default();
        tr.update(&w);
        tr.update(&w);
        assert_eq!(tr.get(0), (0.0, 0.0));
        w.tank_mut(0).unwrap().alive = false;
        w.tank_mut(0).unwrap().x += 30.0;
        tr.update(&w);
        assert_eq!(tr.get(0), (0.0, 0.0));
    }
}

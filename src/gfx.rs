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

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Value noise that tiles every `pw` x `ph` lattice cells.
fn tile_noise(x: f32, y: f32, pw: i32, ph: i32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (smooth(x - xi as f32), smooth(y - yi as f32));
    let h = |a: i32, b: i32| hash(a.rem_euclid(pw), b.rem_euclid(ph), seed);
    let top = h(xi, yi) + (h(xi + 1, yi) - h(xi, yi)) * fx;
    let bot = h(xi, yi + 1) + (h(xi + 1, yi + 1) - h(xi, yi + 1)) * fx;
    top + (bot - top) * fy
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::new(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1.0)
}

/// Paints a meadow into `img`. Everything wraps at the image edge when
/// `tileable`, so the same image can be repeated across the outer field.
fn paint_meadow(w: u32, h: u32, scale: f32, tileable: bool, seed: u32) -> Image {
    let mut img = Image::gen_image_color(w as u16, h as u16, GREEN);
    let dark = Color::new(0.10, 0.27, 0.07, 1.0);
    let mid = Color::new(0.22, 0.43, 0.11, 1.0);
    let light = Color::new(0.43, 0.60, 0.19, 1.0);
    let dry = Color::new(0.56, 0.55, 0.24, 1.0);
    let dirt_a = Color::new(0.42, 0.30, 0.18, 1.0);
    let dirt_b = Color::new(0.56, 0.43, 0.27, 1.0);
    let (wi, hi) = (w as i32, h as i32);
    // noise lattice sizes in pixels, scaled with the texture resolution
    let cell = |px: f32| (px * scale).max(2.0);
    for py in 0..hi {
        for px in 0..wi {
            let n = |c: f32, sd: u32| {
                let c = cell(c);
                let (pw, ph) = (((w as f32 / c).round() as i32).max(1), ((h as f32 / c).round() as i32).max(1));
                // stretch so the lattice divides the image exactly
                let (sx, sy) = (w as f32 / pw as f32, h as f32 / ph as f32);
                if tileable {
                    tile_noise(px as f32 / sx, py as f32 / sy, pw, ph, seed + sd)
                } else {
                    value_noise(px as f32 / c, py as f32 / c, seed + sd)
                }
            };
            let big = n(70.0, 1);
            let medium = n(24.0, 2);
            let fine = n(7.0, 3);
            let fleck = hash(px, py, seed + 4);
            let t = 0.45 * big + 0.3 * medium + 0.25 * fine;
            let mut c = if t < 0.5 { mix(dark, mid, t * 2.0) } else { mix(mid, light, (t - 0.5) * 2.0) };
            // sun-bleached patches
            c = mix(c, dry, ((n(55.0, 5) - 0.62) * 4.0).clamp(0.0, 0.7));
            // pixel level speckle for blade-like grain
            let g = 0.9 + 0.2 * fleck;
            c = Color::new(c.r * g, c.g * g, c.b * g, 1.0);
            // dirt patches with soft edges and pebbles
            let dm = 0.65 * n(90.0, 7) + 0.35 * n(30.0, 8);
            if dm > 0.6 {
                let k = ((dm - 0.6) * 12.0).clamp(0.0, 1.0);
                let mut d = mix(dirt_a, dirt_b, 0.5 * n(9.0, 9) + 0.5 * fleck);
                if fleck > 0.985 {
                    d = Color::new(0.62, 0.6, 0.57, 1.0);
                } else if fleck < 0.03 {
                    d = Color::new(0.28, 0.2, 0.12, 1.0);
                }
                c = mix(c, d, k);
            }
            if !tileable {
                // soft shade along the walls
                let e = (px.min(wi - 1 - px).min(py).min(hi - 1 - py)) as f32 / (scale * 26.0);
                let ao = 0.72 + 0.28 * smooth(e.clamp(0.0, 1.0));
                c = Color::new(c.r * ao, c.g * ao, c.b * ao, 1.0);
            }
            img.set_pixel(px as u32, py as u32, c);
        }
    }
    // blades of grass: short bright/dark strokes
    let blades = (w * h) as f32 / (scale * scale * 26.0);
    for k in 0..blades as i32 {
        let bx = (hash(k, 1, seed + 20) * w as f32) as i32;
        let by = (hash(k, 2, seed + 20) * h as f32) as i32;
        let lean = (hash(k, 3, seed + 20) - 0.5) * 0.8;
        let len = (3.0 + 4.0 * hash(k, 4, seed + 20)) * scale.max(0.6);
        let tone = hash(k, 5, seed + 20);
        let base = img.get_pixel(bx as u32 % w, by as u32 % h);
        let g = if tone > 0.45 { 1.3 + 0.3 * tone } else { 0.62 };
        let col = Color::new((base.r * g * 0.9).min(1.0), (base.g * g).min(1.0), (base.b * g * 0.8).min(1.0), 1.0);
        for s in 0..len as i32 {
            let (x, y) = (bx as f32 + lean * s as f32, by as f32 - s as f32);
            let (xi, yi) = (x as i32, y as i32);
            if tileable {
                img.set_pixel(xi.rem_euclid(wi) as u32, yi.rem_euclid(hi) as u32, col);
            } else if xi >= 0 && yi >= 0 && xi < wi && yi < hi {
                img.set_pixel(xi as u32, yi as u32, col);
            }
        }
    }
    // wild flowers
    for k in 0..(w * h / (900.0 * scale * scale) as u32) as i32 {
        let x = (hash(k, 7, seed + 30) * w as f32) as u32;
        let y = (hash(k, 8, seed + 30) * h as f32) as u32;
        let base = img.get_pixel(x, y);
        if base.g < base.r * 1.2 {
            continue; // not on bare dirt
        }
        let col = match (hash(k, 9, seed + 30) * 4.0) as u32 {
            0 => Color::new(0.97, 0.96, 0.9, 1.0),
            1 => Color::new(0.98, 0.85, 0.25, 1.0),
            2 => Color::new(0.9, 0.45, 0.6, 1.0),
            _ => Color::new(0.6, 0.55, 0.95, 1.0),
        };
        img.set_pixel(x, y, col);
        if scale >= 1.0 && x + 1 < w {
            img.set_pixel(x + 1, y, col);
        }
    }
    img
}

/// Detailed meadow for the arena floor (40 px per world unit).
pub fn make_ground_texture() -> Texture2D {
    let img = paint_meadow(ARENA_W as u32, ARENA_H as u32, 1.0, false, 1);
    let tex = Texture2D::from_image(&img);
    tex.set_filter(FilterMode::Linear);
    tex
}

/// Seamless grass that is repeated across the field outside the arena.
pub fn make_outer_texture() -> Texture2D {
    let img = paint_meadow(256, 256, 0.8, true, 50);
    let tex = Texture2D::from_image(&img);
    tex.set_filter(FilterMode::Linear);
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

    /// Single triangle with per-corner colours (grass blades).
    pub fn tri(&mut self, p: [Vec3; 3], c: [Color; 3]) {
        self.room(3);
        let base = self.v.len() as u16;
        for k in 0..3 {
            self.v.push(Vertex::new2(p[k], vec2(0., 0.), c[k]));
        }
        self.i.extend_from_slice(&[base, base + 1, base + 2]);
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

pub fn brass_c() -> Color {
    Color::new(0.85, 0.7, 0.25, 1.0)
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
    pub outer: Texture2D,
}

const HAZE: Color = Color::new(0.80, 0.88, 0.92, 1.0);
const OUTER_HALF: f32 = 96.0;
const OUTER_TILE: f32 = 8.0;

impl Scene {
    pub fn new() -> Scene {
        Scene { ground: make_ground_texture(), outer: make_outer_texture() }
    }

    pub fn draw_world(&self, world: &World, time: f32, tracks: &TrackAnim, aim: Option<&(Vec<(f32, f32)>, bool)>, hide_pips_of: Option<u32>) {
        self.draw_sky();
        self.draw_outer_field();
        draw_plane(vec3(0.0, 0.0, 0.0), vec2(ARENA_W / UNIT / 2.0, ARENA_H / UNIT / 2.0), Some(&self.ground), WHITE);

        let mut b = Batch::new();
        self.draw_scenery(&mut b, time);
        self.draw_grass(&mut b, world, time);
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

    /// Brick wall around the arena: staggered bricks over dark mortar, stone cap.
    fn draw_walls(&self, b: &mut Batch) {
        let (hw, hh) = (ARENA_W / UNIT / 2.0, ARENA_H / UNIT / 2.0);
        let t = 0.5;
        let (rows, rh) = (3, 0.26);
        let mortar = Color::new(0.24, 0.19, 0.15, 1.0);
        let cap = Color::new(0.58, 0.56, 0.52, 1.0);
        // (centre x, centre z, length, along_x)
        let sides = [(0.0, -hh - t / 2.0, hw * 2.0 + t * 2.0, true), (0.0, hh + t / 2.0, hw * 2.0 + t * 2.0, true), (-hw - t / 2.0, 0.0, hh * 2.0, false), (hw + t / 2.0, 0.0, hh * 2.0, false)];
        for (si, &(cx, cz, len, along_x)) in sides.iter().enumerate() {
            let dims = |l: f32, th: f32| if along_x { vec3(l, rh, th) } else { vec3(th, rh, l) };
            // mortar core
            b.box_y(vec3(cx, rows as f32 * rh / 2.0 - 0.02, cz), dims(len, t - 0.06) * vec3(1.0, rows as f32, 1.0), 0.0, mortar);
            let n = (len / 0.95).round() as i32;
            let cell = len / n as f32;
            for r in 0..rows {
                let off = if r % 2 == 1 { cell / 2.0 } else { 0.0 };
                let count = if r % 2 == 1 { n - 1 } else { n };
                for i in 0..count {
                    let pos = -len / 2.0 + cell * (i as f32 + 0.5) + off;
                    let k = 0.82 + 0.3 * hash(i + si as i32 * 97, r, 31);
                    let tint = hash(i, r + si as i32 * 13, 32);
                    let col = Color::new((0.60 + 0.08 * tint) * k, (0.36 + 0.04 * tint) * k, 0.25 * k, 1.0);
                    let (px, pz) = if along_x { (cx + pos, cz) } else { (cx, cz + pos) };
                    b.box_y(vec3(px, rh * (r as f32 + 0.5) - 0.02, pz), dims(cell - 0.05, t + 0.02) * vec3(1.0, 0.9, 1.0), 0.0, col);
                }
            }
            b.box_y(vec3(cx, rows as f32 * rh + 0.04, cz), dims(len + if along_x { 0.0 } else { 0.0 }, t + 0.14) * vec3(1.0, 0.7, 1.0), 0.0, cap);
        }
    }

    /// Gradient sky dome with a warm glow around the sun.
    fn draw_sky(&self) {
        let sun = vec3(-0.35, 0.8, 0.5).normalize();
        let zenith = Color::new(0.20, 0.42, 0.78, 1.0);
        let elev = [-0.35f32, 0.0, 0.06, 0.16, 0.32, 0.55, 0.85, 1.15, 1.45, FRAC_PI_2];
        let segs = 36usize;
        let r = 400.0;
        let mut v = Vec::new();
        for &e in &elev {
            for j in 0..=segs {
                let a = TAU * j as f32 / segs as f32;
                let dir = vec3(e.cos() * a.cos(), e.sin(), e.cos() * a.sin());
                let up = e.sin().max(0.0).powf(0.6);
                let mut c = mix(HAZE, zenith, up);
                let glow = dir.dot(sun).max(0.0).powi(12) * 0.35;
                c = Color::new((c.r + glow).min(1.0), (c.g + glow * 0.85).min(1.0), (c.b + glow * 0.5).min(1.0), 1.0);
                v.push(Vertex::new2(dir * r, vec2(0., 0.), c));
            }
        }
        let mut idx = Vec::new();
        for i in 0..elev.len() - 1 {
            for j in 0..segs {
                let a = (i * (segs + 1) + j) as u16;
                let b = a + (segs + 1) as u16;
                idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        draw_mesh(&Mesh { vertices: v, indices: idx, texture: None });
    }

    /// Endless-looking meadow around the arena: a repeated seamless texture
    /// that fades into the haze at the horizon.
    fn draw_outer_field(&self) {
        let n = (OUTER_HALF * 2.0 / OUTER_TILE) as i32;
        let fog = |x: f32, z: f32| {
            let d = x.hypot(z);
            let t = ((d - 26.0) / (OUTER_HALF - 26.0)).clamp(0.0, 1.0);
            let c = mix(WHITE, Color::new(0.74, 0.86, 0.92, 1.0), t * t);
            c
        };
        let mut v = Vec::with_capacity((n * n * 4) as usize);
        let mut idx = Vec::with_capacity((n * n * 6) as usize);
        for iz in 0..n {
            for ix in 0..n {
                let (x0, z0) = (-OUTER_HALF + ix as f32 * OUTER_TILE, -OUTER_HALF + iz as f32 * OUTER_TILE);
                let base = v.len() as u16;
                for (dx, dz, u, w) in [(0.0, 0.0, 0.0, 0.0), (1.0, 0.0, 1.0, 0.0), (1.0, 1.0, 1.0, 1.0), (0.0, 1.0, 0.0, 1.0)] {
                    let (x, z) = (x0 + dx * OUTER_TILE, z0 + dz * OUTER_TILE);
                    v.push(Vertex::new2(vec3(x, -0.04, z), vec2(u, w), fog(x, z)));
                }
                idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
        // stay under macroquad's per-mesh limits
        let per = 200usize;
        for chunk in 0..(v.len() / 4 + per - 1) / per {
            let (a, e) = (chunk * per, ((chunk + 1) * per).min(v.len() / 4));
            let verts = v[a * 4..e * 4].to_vec();
            let ind: Vec<u16> = (0..(e - a) as u16).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
            draw_mesh(&Mesh { vertices: verts, indices: ind, texture: Some(self.outer.clone()) });
        }
    }

    /// Hills on the horizon, trees, bushes, boulders and drifting clouds.
    fn draw_scenery(&self, b: &mut Batch, time: f32) {
        // hills hide the edge of the meadow
        for k in 0..30 {
            let a = TAU * (k as f32 + 0.4 * hash(k, 1, 60)) / 30.0;
            let rad = OUTER_HALF + 6.0 + 22.0 * hash(k, 2, 60);
            let r = 30.0 + 26.0 * hash(k, 3, 60);
            let tone = 0.85 + 0.2 * hash(k, 4, 60);
            let col = Color::new(0.38 * tone, 0.55 * tone, 0.42 * tone, 1.0);
            b.sphere(vec3(a.cos() * rad, -r * 0.18, a.sin() * rad), r, 0.42 + 0.2 * hash(k, 5, 60), col);
        }
        // trees (rounded and pine-like)
        for k in 0..64 {
            let x = (hash(k, 1, 61) - 0.5) * 2.0 * 78.0;
            let z = (hash(k, 2, 61) - 0.5) * 2.0 * 64.0;
            if x.abs() < ARENA_W / UNIT / 2.0 + 4.5 && z.abs() < ARENA_H / UNIT / 2.0 + 4.5 {
                continue;
            }
            let s = 0.8 + 0.7 * hash(k, 3, 61);
            let tone = 0.8 + 0.4 * hash(k, 4, 61);
            b.shadow(vec3(x + 0.5 * s, 0.01, z + 0.4 * s), 1.4 * s, 1.1 * s, 0.0);
            b.box_y(vec3(x, 0.9 * s, z), vec3(0.34 * s, 1.8 * s, 0.34 * s), 0.0, Color::new(0.36, 0.25, 0.16, 1.0));
            if hash(k, 5, 61) > 0.45 {
                for (i, (dx, dy, dz, rr)) in [(0.0, 2.5, 0.0, 1.25), (0.7, 2.2, 0.3, 0.9), (-0.6, 2.3, -0.4, 0.95)].iter().enumerate() {
                    let g = tone * (0.95 + 0.1 * i as f32);
                    b.sphere(vec3(x + dx * s, dy * s, z + dz * s), rr * s, 0.85, Color::new(0.14 * g, 0.40 * g, 0.12 * g, 1.0));
                }
            } else {
                for (dy, rr) in [(1.7, 1.15), (2.6, 0.9), (3.4, 0.6)] {
                    b.sphere(vec3(x, dy * s, z), rr * s, 1.25, Color::new(0.08 * tone, 0.30 * tone, 0.16 * tone, 1.0));
                }
            }
        }
        // bushes and boulders near the walls
        for k in 0..46 {
            let x = (hash(k, 1, 62) - 0.5) * 2.0 * 46.0;
            let z = (hash(k, 2, 62) - 0.5) * 2.0 * 30.0;
            if x.abs() < ARENA_W / UNIT / 2.0 + 1.6 && z.abs() < ARENA_H / UNIT / 2.0 + 1.6 {
                continue;
            }
            let s = 0.4 + 0.5 * hash(k, 3, 62);
            if hash(k, 4, 62) > 0.35 {
                let g = 0.8 + 0.4 * hash(k, 5, 62);
                b.shadow(vec3(x + 0.2, 0.01, z + 0.2), 0.9 * s, 0.8 * s, 0.0);
                b.sphere(vec3(x, 0.35 * s, z), 0.8 * s, 0.7, Color::new(0.16 * g, 0.38 * g, 0.12 * g, 1.0));
                b.sphere(vec3(x + 0.5 * s, 0.28 * s, z + 0.2 * s), 0.55 * s, 0.7, Color::new(0.2 * g, 0.44 * g, 0.14 * g, 1.0));
            } else {
                let g = 0.8 + 0.3 * hash(k, 5, 62);
                b.box_y(vec3(x, 0.25 * s, z), vec3(1.1 * s, 0.5 * s, 0.8 * s), hash(k, 6, 62) * 3.0, Color::new(0.5 * g, 0.5 * g, 0.52 * g, 1.0));
            }
        }
        // clouds
        for k in 0..14 {
            let a = TAU * hash(k, 1, 63);
            let rad = 120.0 + 200.0 * hash(k, 2, 63);
            let drift = time * (0.5 + 0.5 * hash(k, 3, 63));
            let (cx, cz) = (a.cos() * rad + drift, a.sin() * rad);
            let cx = (cx + 330.0).rem_euclid(660.0) - 330.0;
            let h = 70.0 + 50.0 * hash(k, 4, 63);
            let r = 16.0 + 14.0 * hash(k, 5, 63);
            for (dx, dz, f) in [(0.0, 0.0, 1.0), (r * 0.9, r * 0.2, 0.7), (-r * 0.8, -r * 0.1, 0.75)] {
                b.sphere(vec3(cx + dx, h, cz + dz), r * f, 0.28, Color::new(1.0, 1.0, 1.0, 1.0));
            }
        }
    }

    /// Swaying tufts of grass scattered over the arena floor.
    fn draw_grass(&self, b: &mut Batch, world: &World, time: f32) {
        let base = Color::new(0.10, 0.27, 0.07, 1.0);
        for k in 0..420 {
            let x = 18.0 + hash(k, 1, 70) * (ARENA_W - 36.0);
            let y = 18.0 + hash(k, 2, 70) * (ARENA_H - 36.0);
            if world.obstacles.iter().any(|o| x > o.x - 14.0 && x < o.x + o.w + 14.0 && y > o.y - 14.0 && y < o.y + o.h + 14.0) {
                continue;
            }
            let p = w3(x, y, 0.0);
            let sway = (time * 1.6 + x * 0.05 + y * 0.03).sin() * 0.05;
            let tone = 0.85 + 0.3 * hash(k, 3, 70);
            let tip = Color::new(0.42 * tone, 0.64 * tone, 0.20 * tone, 1.0);
            for bl in 0..4 {
                let a = bl as f32 * 1.57 + hash(k, 4, 70) * 6.0;
                let (dx, dz) = (a.cos() * 0.05, a.sin() * 0.05);
                let hgt = 0.16 + 0.12 * hash(k, 5 + bl, 70);
                let lean = vec3(a.cos() * 0.07 + sway, hgt, a.sin() * 0.07);
                let side = vec3(-a.sin() * 0.025, 0.0, a.cos() * 0.025);
                b.tri([p + vec3(dx, 0.0, dz) - side, p + vec3(dx, 0.0, dz) + side, p + lean], [base, base, tip]);
            }
        }
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
            ObstacleKind::Crate => {
                let n = ((w.min(d)) / 0.75).round().max(1.0) as i32;
                let c = w3(cx, cy, 0.0);
                for i in 0..n {
                    for j in 0..n {
                        let sh = 0.85 + 0.2 * hash(i + idx as i32 * 5, j, 21);
                        let col = Color::new(0.62 * sh, 0.44 * sh, 0.24 * sh, 1.0);
                        let (px, pz) = (c.x + (i as f32 - (n - 1) as f32 / 2.0) * (w / n as f32), c.z + (j as f32 - (n - 1) as f32 / 2.0) * (d / n as f32));
                        let (cw, cd) = (w / n as f32 - 0.03, d / n as f32 - 0.03);
                        b.box_y(vec3(px, 0.3, pz), vec3(cw, 0.6, cd), hash(i, j, 22) * 0.1, col);
                        b.box_y(vec3(px, 0.61, pz), vec3(cw * 0.96, 0.03, cd * 0.96), 0.0, Color::new(0.45 * sh, 0.32 * sh, 0.18 * sh, 1.0));
                        b.box_y(vec3(px, 0.3, pz), vec3(cw + 0.02, 0.07, cd + 0.02), 0.0, Color::new(0.4, 0.28, 0.16, 1.0)); // straps
                    }
                }
                // one crate stacked on top
                b.box_y(vec3(c.x, 0.9, c.z), vec3(w * 0.5, 0.5, d * 0.5), 0.3, Color::new(0.7, 0.5, 0.28, 1.0));
            }
            ObstacleKind::Pond => {
                let c = w3(cx, cy, 0.0);
                b.box_y(vec3(c.x, 0.005, c.z), vec3(w + 0.14, 0.012, d + 0.14), 0.0, Color::new(0.45, 0.36, 0.22, 1.0)); // muddy bank
                b.box_y(vec3(c.x, 0.03, c.z), vec3(w, 0.02, d), 0.0, Color::new(0.16, 0.38, 0.62, 1.0));
                b.box_y(vec3(c.x, 0.045, c.z), vec3(w * 0.7, 0.01, d * 0.7), 0.0, Color::new(0.22, 0.5, 0.74, 1.0));
                for k in 0..4 {
                    let a = idx as f32 * 1.3 + k as f32 * 1.7;
                    b.box_y(vec3(c.x + a.cos() * w * 0.25, 0.058, c.z + a.sin() * d * 0.25), vec3(0.28, 0.006, 0.05), a, Color::new(0.75, 0.88, 0.95, 1.0)); // ripples
                }
                for k in 0..7 {
                    let a = k as f32 * 0.9 + idx as f32;
                    let (rx, rz) = (c.x + a.cos() * (w / 2.0 + 0.05), c.z + a.sin() * (d / 2.0 + 0.05));
                    b.box_y(vec3(rx, 0.1, rz), vec3(0.025, 0.2, 0.025), 0.0, Color::new(0.35, 0.5, 0.2, 1.0)); // reeds
                }
            }
            ObstacleKind::Mud => {
                let c = w3(cx, cy, 0.0);
                b.box_y(vec3(c.x, 0.004, c.z), vec3(w, 0.01, d), 0.0, Color::new(0.30, 0.21, 0.13, 1.0));
                b.box_y(vec3(c.x, 0.009, c.z), vec3(w * 0.78, 0.01, d * 0.72), 0.3, Color::new(0.24, 0.17, 0.1, 1.0));
                for k in 0..6 {
                    let a = idx as f32 * 2.1 + k as f32 * 1.1;
                    b.box_y(vec3(c.x + a.cos() * w * 0.3, 0.016, c.z + a.sin() * d * 0.3), vec3(0.3, 0.008, 0.18), a, Color::new(0.4, 0.3, 0.2, 1.0)); // puddle sheen
                }
            }
            ObstacleKind::Tree => {
                let c = w3(cx, cy, 0.0);
                let g = 0.85 + 0.3 * hash(idx as i32, 3, 24);
                b.shadow(vec3(c.x + 0.3, 0.01, c.z + 0.25), 0.8, 0.65, 0.0);
                b.box_y(vec3(c.x, 0.5, c.z), vec3(0.3, 1.0, 0.3), 0.0, Color::new(0.38, 0.26, 0.16, 1.0));
                b.sphere(vec3(c.x, 1.45, c.z), 0.95, 0.9, Color::new(0.14 * g, 0.4 * g, 0.12 * g, 1.0));
                b.sphere(vec3(c.x + 0.4, 1.2, c.z + 0.15), 0.65, 0.9, Color::new(0.18 * g, 0.46 * g, 0.14 * g, 1.0));
                b.sphere(vec3(c.x - 0.35, 1.25, c.z - 0.2), 0.6, 0.9, Color::new(0.12 * g, 0.36 * g, 0.1 * g, 1.0));
            }
            ObstacleKind::Block => {
                let c = w3(cx, cy, 0.0);
                b.box_y(vec3(c.x, 0.3, c.z), vec3(w, 0.6, d), 0.0, Color::new(0.62, 0.62, 0.6, 1.0));
                b.box_y(vec3(c.x, 0.64, c.z), vec3(w * 0.96, 0.08, d * 0.9), 0.0, Color::new(0.7, 0.7, 0.68, 1.0));
                for k in 0..(w / 0.5).max(1.0) as i32 {
                    let (px, _) = (c.x - w / 2.0 + 0.25 + k as f32 * 0.5, 0);
                    b.box_y(vec3(px, 0.3, c.z + d / 2.0 + 0.003), vec3(0.2, 0.2, 0.01), 0.0, Color::new(0.9, 0.75, 0.1, 1.0)); // hazard stripes
                }
            }
            ObstacleKind::Tower => {
                let c = w3(cx, cy, 0.0);
                let wood = Color::new(0.46, 0.32, 0.2, 1.0);
                for (dx, dz) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    b.box_y(vec3(c.x + dx * w * 0.4, 1.15, c.z + dz * d * 0.4), vec3(0.14, 2.3, 0.14), 0.0, wood);
                }
                for (y, ex) in [(0.6f32, 1.0f32), (1.4, 0.85)] {
                    b.box_y(vec3(c.x, y, c.z - d * 0.4), vec3(w * ex * 0.8, 0.07, 0.05), 0.0, shade(wood, 0.85));
                    b.box_y(vec3(c.x, y, c.z + d * 0.4), vec3(w * ex * 0.8, 0.07, 0.05), 0.0, shade(wood, 0.85));
                }
                b.box_y(vec3(c.x, 2.35, c.z), vec3(w * 1.05, 0.1, d * 1.05), 0.0, shade(wood, 0.9)); // platform
                for sd in [-1.0f32, 1.0] {
                    b.box_y(vec3(c.x, 2.55, c.z + sd * d * 0.5), vec3(w * 1.05, 0.3, 0.05), 0.0, wood);
                    b.box_y(vec3(c.x + sd * w * 0.5, 2.55, c.z), vec3(0.05, 0.3, d * 1.05), 0.0, wood);
                }
                b.box_y(vec3(c.x, 3.05, c.z), vec3(w * 1.2, 0.08, d * 1.2), 0.0, Color::new(0.6, 0.2, 0.15, 1.0)); // roof
                b.box_y(vec3(c.x, 3.2, c.z), vec3(w * 0.7, 0.14, d * 0.7), 0.0, Color::new(0.68, 0.24, 0.18, 1.0));
                for (dx, dz) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    b.box_y(vec3(c.x + dx * w * 0.5, 2.7, c.z + dz * d * 0.5), vec3(0.06, 0.7, 0.06), 0.0, wood);
                }
                b.shadow(vec3(c.x + 0.6, 0.01, c.z + 0.5), w * 0.8, d * 0.8, 0.0);
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
    pub fn draw_tank(&self, b: &mut Batch, t: &Tank, k: usize, time: f32, tr: (f32, f32), pips: bool) {
        // model scale per class; collision radius stays 0.45
        let S: f32 = match t.kind { 1 => 0.96, 2 => 1.2, _ => 1.1 };
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

        // ---- hull (shared plan, per-class variations) ----
        let camo = shade(body, 0.6);
        let steel = Color::new(0.22, 0.23, 0.25, 1.0);
        let rubber = Color::new(0.09, 0.09, 0.1, 1.0);
        let glass = Color::new(0.5, 0.8, 1.0, 1.0);
        let kind = t.kind;
        let deck = match kind { 1 => 0.45f32, 2 => 0.52, _ => 0.49 };
        b.box_q(at(vec3(0.0, 0.27, 0.0)), sz(vec3(0.9, 0.2, 0.5)), q, shade(body, 0.8)); // lower hull
        b.box_q(at(vec3(-0.04, deck - 0.07, 0.0)), sz(vec3(0.7, 0.14, 0.46)), q, body); // upper hull
        b.box_q(at(vec3(0.44, deck - 0.12, 0.0)), sz(vec3(0.26, 0.12, 0.46)), q * Quat::from_rotation_z(-0.5), shade(body, 0.93)); // glacis
        b.box_q(at(vec3(0.5, 0.2, 0.0)), sz(vec3(0.06, 0.16, 0.5)), q * Quat::from_rotation_z(0.35), shade(body, 0.72)); // lower nose plate
        b.box_q(at(vec3(-0.39, deck + 0.01, 0.0)), sz(vec3(0.2, 0.05, 0.38)), q, shade(body, 0.68)); // engine deck
        for gz in [-0.12f32, -0.04, 0.04, 0.12] {
            b.box_q(at(vec3(-0.39, deck + 0.04, gz)), sz(vec3(0.15, 0.012, 0.025)), q, dark); // louvres
        }
        // camouflage patches
        for (cx, cz, w, d, yw) in [(0.0f32, -0.1f32, 0.24f32, 0.16f32, 0.4f32), (0.18, 0.12, 0.2, 0.14, -0.3), (-0.2, 0.08, 0.18, 0.18, 0.7)] {
            b.box_q(at(vec3(cx, deck + 0.004, cz)), sz(vec3(w, 0.006, d)), q * Quat::from_rotation_y(yw), camo);
        }
        // driver hatch and vision slit on the glacis
        b.cyl(at(vec3(0.33, deck + 0.015, -0.11)), 0.055 * S, 0.03 * S, steel);
        b.box_q(at(vec3(0.5, deck - 0.1, -0.11)), sz(vec3(0.02, 0.025, 0.09)), q * Quat::from_rotation_z(-0.5), dark);
        // lights, mudflaps, tow hooks
        for lz in [-0.17f32, 0.17] {
            b.box_q(at(vec3(0.51, 0.33, lz)), sz(vec3(0.05, 0.065, 0.08)), q, Color::new(1.0, 0.95, 0.55, 1.0));
            b.box_q(at(vec3(-0.47, 0.34, lz)), sz(vec3(0.03, 0.05, 0.07)), q, Color::new(0.9, 0.12, 0.1, 1.0));
            b.box_q(at(vec3(0.5, 0.09, lz * 1.9)), sz(vec3(0.02, 0.1, 0.12)), q, rubber);
        }
        for hz in [-0.12f32, 0.12] {
            b.box_q(at(vec3(0.54, 0.16, hz)), sz(vec3(0.05, 0.03, 0.03)), q, steel);
        }
        // exhaust with muffler, jerry cans, tow cable and stowage on the fenders
        b.cyl(at(vec3(-0.47, 0.46, -0.17)), 0.05 * S, 0.16 * S, steel);
        b.cyl(at(vec3(-0.47, 0.56, -0.17)), 0.032 * S, 0.06 * S, dark);
        for (jz, jc) in [(0.15f32, Color::new(0.35, 0.4, 0.2, 1.0)), (0.25, Color::new(0.5, 0.2, 0.15, 1.0))] {
            b.box_q(at(vec3(-0.42, deck + 0.1, jz - 0.05)), sz(vec3(0.07, 0.12, 0.12)), q, jc);
        }
        for sz_ in [-1.0f32, 1.0] {
            b.box_q(at(vec3(0.0, 0.44, sz_ * 0.3)), sz(vec3(0.74, 0.018, 0.018)), q, Color::new(0.1, 0.09, 0.08, 1.0)); // tow cable
            b.box_q(at(vec3(-0.18, 0.46, sz_ * 0.36)), sz(vec3(0.22, 0.07, 0.07)), q, shade(body, 0.7)); // fender stowage box
        }
        // return rollers above the tracks (the tracks themselves keep animating)
        for rz in [-1.0f32, 1.0] {
            for rx in [-0.15f32, 0.15] {
                b.disc(at(vec3(rx, 0.37, rz * TRACK_HALF_GAP)), 0.035 * S, 0.06 * S, q, steel, shade(steel, 1.2), 0.0, 8);
            }
        }

        // ---- class-specific armour ----
        match kind {
            2 => {
                // heavy: appliqué plates, side skirts, spare track links
                b.box_q(at(vec3(0.36, deck - 0.01, 0.0)), sz(vec3(0.16, 0.05, 0.46)), q * Quat::from_rotation_z(-0.5), shade(body, 0.86));
                for sd in [-1.0f32, 1.0] {
                    b.box_q(at(vec3(0.0, 0.38, sd * 0.45)), sz(vec3(0.9, 0.1, 0.03)), q, shade(body, 0.65));
                    for i in 0..5 {
                        b.box_q(at(vec3(-0.3 + i as f32 * 0.15, 0.38, sd * 0.468)), sz(vec3(0.012, 0.1, 0.012)), q, dark);
                    }
                }
                for i in 0..4 {
                    b.box_q(at(vec3(0.43 - i as f32 * 0.015, deck - 0.14 + i as f32 * 0.04, 0.0)), sz(vec3(0.03, 0.03, 0.34)), q * Quat::from_rotation_z(-0.5), steel);
                }
            }
            1 => {
                // scout: rear stowage rack and a searchlight
                b.box_q(at(vec3(-0.5, deck + 0.02, 0.0)), sz(vec3(0.03, 0.12, 0.34)), q, steel);
                b.box_q(at(vec3(-0.43, deck + 0.04, 0.0)), sz(vec3(0.14, 0.03, 0.3)), q, steel);
                b.box_q(at(vec3(0.3, deck + 0.05, 0.14)), sz(vec3(0.07, 0.07, 0.07)), q, Color::new(1.0, 0.95, 0.7, 1.0));
            }
            3 => {
                // gunner: rocket pods on the fenders
                for sd in [-1.0f32, 1.0] {
                    b.box_q(at(vec3(-0.05, 0.47, sd * 0.36)), sz(vec3(0.36, 0.12, 0.1)), q, shade(body, 0.6));
                    for i in 0..3 {
                        b.box_q(at(vec3(0.14, 0.47 + (i as f32 - 1.0) * 0.032, sd * 0.36)), sz(vec3(0.02, 0.025, 0.07)), q, dark);
                    }
                }
            }
            _ => {}
        }

        // ---- turret and cannon (rotates on its own with the aim) ----
        let qt = Quat::from_rotation_y(-t.turret);
        let att = |l: Vec3| base + qt * (l * S);
        let ty = deck + 0.03; // turret ring height
        b.cyl(att(vec3(0.0, ty, 0.0)), 0.26 * S, 0.06 * S, steel);
        match kind {
            1 => {
                // scout: low wedge turret, long slim gun
                b.box_q(att(vec3(-0.02, ty + 0.08, 0.0)), sz(vec3(0.4, 0.12, 0.34)), qt, body);
                b.box_q(att(vec3(0.2, ty + 0.07, 0.0)), sz(vec3(0.2, 0.1, 0.3)), qt * Quat::from_rotation_z(-0.45), shade(body, 0.92));
                b.box_q(att(vec3(-0.22, ty + 0.1, 0.0)), sz(vec3(0.1, 0.12, 0.3)), qt, shade(body, 0.8));
                b.cyl(att(vec3(-0.08, ty + 0.17, 0.09)), 0.05 * S, 0.03 * S, steel);
                b.box_q(att(vec3(0.07, ty + 0.15, -0.08)), sz(vec3(0.06, 0.035, 0.05)), qt, glass);
                b.box_q(att(vec3(0.4, ty + 0.08, 0.0)), sz(vec3(0.52, 0.055, 0.055)), qt, steel);
                b.box_q(att(vec3(0.68, ty + 0.08, 0.0)), sz(vec3(0.07, 0.085, 0.085)), qt, dark);
                b.box_q(att(vec3(0.15, ty + 0.12, 0.1)), sz(vec3(0.16, 0.025, 0.025)), qt, dark); // coax MG
            }
            2 => {
                // heavy: slab turret with bustle, big mantlet, thick gun
                b.box_q(att(vec3(0.0, ty + 0.12, 0.0)), sz(vec3(0.5, 0.2, 0.5)), qt, body);
                b.box_q(att(vec3(0.27, ty + 0.12, 0.0)), sz(vec3(0.18, 0.2, 0.48)), qt * Quat::from_rotation_z(-0.5), shade(body, 0.9));
                b.box_q(att(vec3(-0.34, ty + 0.1, 0.0)), sz(vec3(0.22, 0.16, 0.44)), qt, shade(body, 0.8)); // bustle
                b.box_q(att(vec3(-0.35, ty + 0.2, 0.0)), sz(vec3(0.2, 0.012, 0.3)), qt, camo);
                b.box_q(att(vec3(0.33, ty + 0.12, 0.0)), sz(vec3(0.1, 0.16, 0.22)), qt, steel); // mantlet
                b.box_q(att(vec3(0.38, ty + 0.12, 0.0)), sz(vec3(0.34, 0.085, 0.085)), qt, steel);
                b.box_q(att(vec3(0.5, ty + 0.12, 0.0)), sz(vec3(0.07, 0.115, 0.115)), qt, shade(steel, 0.8)); // bore evacuator
                b.box_q(att(vec3(0.56, ty + 0.12, 0.0)), sz(vec3(0.1, 0.13, 0.13)), qt, dark); // muzzle brake
                b.cyl(att(vec3(-0.1, ty + 0.25, 0.14)), 0.075 * S, 0.07 * S, steel); // commander cupola
                b.box_q(att(vec3(-0.1, ty + 0.31, 0.14)), sz(vec3(0.2, 0.02, 0.02)), qt, dark); // MG
                b.cyl(att(vec3(-0.12, ty + 0.23, -0.14)), 0.06 * S, 0.03 * S, steel);
                for sd in [-1.0f32, 1.0] {
                    for i in 0..3 {
                        b.box_q(att(vec3(0.18, ty + 0.17, sd * (0.24 + i as f32 * 0.0))), sz(vec3(0.03, 0.03, 0.03)), qt, dark); // smoke launchers
                    }
                }
            }
            3 => {
                // gunner: boxy turret, twin autocannons, drums and a sensor
                b.box_q(att(vec3(0.0, ty + 0.1, 0.0)), sz(vec3(0.42, 0.15, 0.4)), qt, body);
                b.box_q(att(vec3(0.2, ty + 0.1, 0.0)), sz(vec3(0.14, 0.15, 0.38)), qt * Quat::from_rotation_z(-0.4), shade(body, 0.9));
                b.box_q(att(vec3(-0.12, ty + 0.2, 0.0)), sz(vec3(0.2, 0.02, 0.24)), qt, camo);
                for gz in [-0.075f32, 0.075] {
                    b.box_q(att(vec3(0.38, ty + 0.1, gz)), sz(vec3(0.44, 0.05, 0.05)), qt, steel);
                    b.box_q(att(vec3(0.58, ty + 0.1, gz)), sz(vec3(0.06, 0.07, 0.07)), qt, dark);
                    b.box_q(att(vec3(0.3, ty + 0.1, gz)), sz(vec3(0.12, 0.08, 0.08)), qt, shade(steel, 0.8)); // cooling jacket
                }
                for sd in [-1.0f32, 1.0] {
                    b.disc(att(vec3(-0.02, ty + 0.1, sd * 0.25)), 0.1 * S, 0.1 * S, qt, shade(body, 0.7), shade(body, 0.55), 0.0, 10); // ammo drum
                    b.box_q(att(vec3(0.12, ty + 0.1, sd * 0.22)), sz(vec3(0.16, 0.03, 0.03)), qt, brass_c()); // ammo feed
                }
                b.sphere(att(vec3(-0.1, ty + 0.26, -0.1)), 0.07 * S, 0.9, Color::new(0.35, 0.55, 0.7, 1.0)); // sensor ball
                b.cyl(att(vec3(-0.12, ty + 0.23, 0.12)), 0.06 * S, 0.04 * S, steel);
            }
            _ => {
                // standard: cast turret with cupola, mantlet and bore evacuator
                b.sphere(att(vec3(-0.02, ty + 0.06, 0.0)), 0.23 * S, 0.62, body);
                b.box_q(att(vec3(-0.17, ty + 0.1, 0.0)), sz(vec3(0.16, 0.14, 0.34)), qt, shade(body, 0.85)); // bustle bin
                b.box_q(att(vec3(0.0, ty + 0.15, -0.05)), sz(vec3(0.24, 0.006, 0.16)), qt * Quat::from_rotation_y(0.5), camo);
                b.cyl(att(vec3(-0.1, ty + 0.17, 0.09)), 0.065 * S, 0.05 * S, steel); // commander cupola
                b.box_q(att(vec3(-0.06, ty + 0.21, 0.09)), sz(vec3(0.16, 0.02, 0.02)), qt, dark); // MG
                b.cyl(att(vec3(-0.12, ty + 0.15, -0.1)), 0.05 * S, 0.03 * S, steel); // loader hatch
                b.box_q(att(vec3(0.1, ty + 0.14, -0.08)), sz(vec3(0.06, 0.035, 0.045)), qt, glass); // periscope
                b.box_q(att(vec3(0.2, ty + 0.07, 0.0)), sz(vec3(0.08, 0.13, 0.2)), qt, steel); // mantlet
                b.box_q(att(vec3(0.34, ty + 0.07, 0.0)), sz(vec3(0.42, 0.075, 0.075)), qt, steel);
                b.box_q(att(vec3(0.38, ty + 0.07, 0.0)), sz(vec3(0.08, 0.1, 0.1)), qt, shade(steel, 0.8)); // bore evacuator
                b.box_q(att(vec3(0.56, ty + 0.07, 0.0)), sz(vec3(0.08, 0.115, 0.115)), qt, dark); // muzzle brake
                b.box_q(att(vec3(0.17, ty + 0.07, 0.1)), sz(vec3(0.14, 0.025, 0.025)), qt, dark); // coax MG
                for sd in [-1.0f32, 1.0] {
                    b.box_q(att(vec3(0.1, ty + 0.1, sd * 0.2)), sz(vec3(0.04, 0.04, 0.04)), qt, dark); // smoke dischargers
                }
            }
        }

        // ---- antenna and team flag ----
        let (ax, ah) = match kind { 1 => (-0.45, 0.95), 2 => (-0.5, 0.7), _ => (-0.44, 0.78) };
        b.box_q(at(vec3(ax, deck + ah / 2.0, 0.2)), sz(vec3(0.015, ah, 0.015)), q, steel);
        let wave = (time * 7.0 + t.id as f32).sin() * 0.35;
        b.box_q(at(vec3(ax, deck + ah - 0.06, 0.2)), sz(vec3(0.02, 0.1, 0.16)), q * Quat::from_rotation_y(wave), body);

        // ---- hit points as pips above the tank ----
        for i in 0..if pips { t.hp.max(0) } else { 0 } {
            let off = (i as f32 - (t.max_hp() as f32 - 1.0) / 2.0) * 0.2;
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
                        .filter(|o| o.tall())
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
        let dist = ((ARENA_W / UNIT / 2.0 + 0.8) / (tan * aspect)).max((ARENA_H / UNIT / 2.0 + 2.4) / tan).max(12.0);
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
        World::new_match(&[PlayerSpec { id: 0, name: "a".into(), team: 0, ai: false, kind: 0 }], 1)
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

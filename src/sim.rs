//! Headless game simulation. No rendering, no networking, no macroquad.
//!
//! The simulation lives on a flat 960x640 plane (x, y). The renderer maps
//! it onto the XZ ground plane in 3D. Keeping the rules here makes them
//! unit-testable and lets the LAN host and the offline mode share them.

use std::f32::consts::PI;

pub const ARENA_W: f32 = 960.0;
pub const ARENA_H: f32 = 640.0;

pub const TANK_R: f32 = 18.0;
pub const TANK_SPEED: f32 = 110.0;
pub const TANK_ROT: f32 = 1.6;
pub const TANK_HP: i32 = 3;
pub const BULLET_SPEED: f32 = 480.0;
pub const BULLET_R: f32 = 4.0;
pub const MAX_BOUNCES: u32 = 2;
pub const MAX_AMMO: i32 = 5;
pub const FIRE_COOLDOWN: f32 = 0.18;
pub const RELOAD_TIME: f32 = 1.6;
pub const TAP_THRESHOLD: f32 = 0.16;
pub const PICKUP_R: f32 = 14.0;
pub const MAX_PICKUPS: usize = 3;
pub const SPEED_MULT: f32 = 1.6;
pub const RAPID_MULT: f32 = 0.35;
pub const BUFF_TIME: f32 = 6.0;
pub const SHIELD_TIME: f32 = 5.0;
// Drive (chase-camera) controls
pub const DRIVE_TURN: f32 = 2.3;
pub const REVERSE_MULT: f32 = 0.6;
pub const BOOST_MULT: f32 = 1.7;
/// Stamina is 0..1; these are per-second rates.
pub const BOOST_DRAIN: f32 = 1.0 / 1.3;
pub const BOOST_REGEN: f32 = 1.0 / 3.5;
pub const TURRET_RATE: f32 = 7.0;
/// Delay between the last kill and the game-over screen so the explosion plays.
pub const END_DELAY: f32 = 1.5;

// ---------------------------------------------------------------- rng ----

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u32(&mut self) -> u32 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }
    /// Uniform in [0, 1).
    pub fn f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f32()
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u32() as usize) % n.max(1)
    }
}

// ------------------------------------------------------------- pickups ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickupKind {
    Heal,
    Ammo,
    Speed,
    Rapid,
    Shield,
}

impl PickupKind {
    pub const ALL: [PickupKind; 5] = [
        PickupKind::Heal,
        PickupKind::Ammo,
        PickupKind::Speed,
        PickupKind::Rapid,
        PickupKind::Shield,
    ];
    pub fn code(self) -> u8 {
        self as u8
    }
    pub fn from_code(c: u8) -> Option<Self> {
        Self::ALL.get(c as usize).copied()
    }
    pub fn label(self) -> &'static str {
        match self {
            PickupKind::Heal => "REPAIR",
            PickupKind::Ammo => "RELOAD",
            PickupKind::Speed => "NITRO",
            PickupKind::Rapid => "RAPID",
            PickupKind::Shield => "SHIELD",
        }
    }
    /// RGB colour used by the renderer and HUD.
    pub fn rgb(self) -> (f32, f32, f32) {
        match self {
            PickupKind::Heal => (0.35, 0.9, 0.4),
            PickupKind::Ammo => (0.9, 0.85, 0.3),
            PickupKind::Speed => (1.0, 0.65, 0.15),
            PickupKind::Rapid => (0.95, 0.35, 0.25),
            PickupKind::Shield => (0.35, 0.75, 1.0),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pickup {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub kind: PickupKind,
}

// ----------------------------------------------------------- obstacles ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObstacleKind {
    Rock,
    Sandbag,
}

#[derive(Clone, Debug)]
pub struct Obstacle {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub kind: ObstacleKind,
}

/// Fixed, left/right symmetric layout. Spawn lanes (x < 150 and x > 810) stay
/// clear. Because it is fixed, it never has to be sent over the network.
pub fn default_obstacles() -> Vec<Obstacle> {
    use ObstacleKind::*;
    let o = |x, y, w, h, kind| Obstacle { x, y, w, h, kind };
    vec![
        o(440.0, 280.0, 80.0, 80.0, Rock),
        o(250.0, 120.0, 40.0, 100.0, Sandbag),
        o(670.0, 120.0, 40.0, 100.0, Sandbag),
        o(250.0, 420.0, 40.0, 100.0, Sandbag),
        o(670.0, 420.0, 40.0, 100.0, Sandbag),
        o(330.0, 300.0, 50.0, 40.0, Rock),
        o(580.0, 300.0, 50.0, 40.0, Rock),
        o(440.0, 80.0, 80.0, 30.0, Sandbag),
        o(440.0, 530.0, 80.0, 30.0, Sandbag),
    ]
}

// --------------------------------------------------------------- tanks ----

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AiMode {
    Idle,
    Tap(f32),
    Drive(f32),
}

#[derive(Clone, Debug)]
pub struct Ai {
    pub mode: AiMode,
    /// Seconds until the AI may fire again.
    pub fire_wait: f32,
    /// Seconds until the AI considers driving.
    pub drive_wait: f32,
}

/// One player's input for a frame. `drive == false` is the classic scheme
/// (only `held` matters: tap = fire, hold = drive). `drive == true` is the
/// chase-camera scheme: steer/throttle drive the hull, `aim` is the absolute
/// turret angle (the camera yaw), `fire` is held for auto-fire.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    pub drive: bool,
    pub held: bool,
    pub steer: f32,
    pub throttle: f32,
    pub fire: bool,
    pub boost: bool,
    pub reload: bool,
    pub aim: f32,
}

impl Default for Control {
    fn default() -> Self {
        Control { drive: false, held: false, steer: 0.0, throttle: 0.0, fire: false, boost: false, reload: false, aim: 0.0 }
    }
}

impl Control {
    pub fn classic(held: bool) -> Control {
        Control { held, ..Default::default() }
    }

    pub fn encode(&self) -> String {
        format!(
            "{},{},{:.2},{:.2},{},{},{},{:.2}",
            self.drive as u8, self.held as u8, self.steer, self.throttle, self.fire as u8, self.boost as u8, self.reload as u8, self.aim
        )
    }

    /// Accepts the full form and the legacy "0"/"1" classic form.
    pub fn decode(s: &str) -> Option<Control> {
        if s == "0" || s == "1" {
            return Some(Control::classic(s == "1"));
        }
        let f: Vec<&str> = s.split(',').collect();
        if f.len() != 8 {
            return None;
        }
        let b = |x: &str| x == "1";
        let n = |x: &str| x.parse::<f32>().ok().filter(|v| v.is_finite());
        Some(Control {
            drive: b(f[0]),
            held: b(f[1]),
            steer: n(f[2])?.clamp(-1.0, 1.0),
            throttle: n(f[3])?.clamp(-1.0, 1.0),
            fire: b(f[4]),
            boost: b(f[5]),
            reload: b(f[6]),
            aim: n(f[7])?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Tank {
    pub id: u32,
    pub team: u8,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub angle: f32,
    pub hp: i32,
    pub alive: bool,
    pub ammo: i32,
    /// Seconds of reload left; 0 means not reloading.
    pub reload: f32,
    pub cooldown: f32,
    pub rot_dir: f32,
    pub held: bool,
    pub was_held: bool,
    pub hold_timer: f32,
    pub moving: bool,
    pub speed_t: f32,
    pub rapid_t: f32,
    pub shield_t: f32,
    /// Absolute turret angle. Equals `angle` in the classic scheme.
    pub turret: f32,
    /// Boost stamina, 0..1.
    pub stamina: f32,
    boost_lock: bool,
    pub ctrl: Control,
    pub ai: Option<Ai>,
}

impl Tank {
    pub fn new(id: u32, team: u8, name: &str, x: f32, y: f32, angle: f32, ai: bool) -> Self {
        Tank {
            id,
            team,
            name: name.to_string(),
            x,
            y,
            angle,
            hp: TANK_HP,
            alive: true,
            ammo: MAX_AMMO,
            reload: 0.0,
            cooldown: 0.0,
            rot_dir: 1.0,
            held: false,
            was_held: false,
            hold_timer: 0.0,
            moving: false,
            speed_t: 0.0,
            rapid_t: 0.0,
            shield_t: 0.0,
            turret: angle,
            stamina: 1.0,
            boost_lock: false,
            ctrl: Control::default(),
            ai: if ai {
                Some(Ai { mode: AiMode::Idle, fire_wait: 1.0, drive_wait: 1.5 })
            } else {
                None
            },
        }
    }

    pub fn reloading(&self) -> bool {
        self.reload > 0.0
    }
    pub fn speed(&self) -> f32 {
        TANK_SPEED * if self.speed_t > 0.0 { SPEED_MULT } else { 1.0 }
    }
    pub fn cooldown_len(&self) -> f32 {
        FIRE_COOLDOWN * if self.rapid_t > 0.0 { RAPID_MULT } else { 1.0 }
    }
    pub fn can_fire(&self) -> bool {
        self.alive && self.ammo > 0 && !self.reloading() && self.cooldown <= 0.0
    }
}

// --------------------------------------------------------------- misc ----

#[derive(Clone, Debug)]
pub struct Bullet {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub bounces: u32,
    pub owner: u32,
    pub team: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Fire,
    Bounce,
    Hit,
    Explode,
    Pickup,
}

impl EventKind {
    pub fn code(self) -> char {
        match self {
            EventKind::Fire => 'F',
            EventKind::Bounce => 'B',
            EventKind::Hit => 'H',
            EventKind::Explode => 'X',
            EventKind::Pickup => 'P',
        }
    }
    pub fn from_code(c: &str) -> Option<Self> {
        Some(match c {
            "F" => EventKind::Fire,
            "B" => EventKind::Bounce,
            "H" => EventKind::Hit,
            "X" => EventKind::Explode,
            "P" => EventKind::Pickup,
            _ => return None,
        })
    }
}

/// A one-shot happening used for particles and sound. `a` is the angle for
/// muzzle flashes and the pickup kind code for pickup events.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub kind: EventKind,
    pub x: f32,
    pub y: f32,
    pub a: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Playing,
    GameOver,
}

#[derive(Clone, Debug)]
pub struct PlayerSpec {
    pub id: u32,
    pub name: String,
    pub team: u8,
    pub ai: bool,
}

// -------------------------------------------------------------- world ----

pub struct World {
    pub tanks: Vec<Tank>,
    pub bullets: Vec<Bullet>,
    pub obstacles: Vec<Obstacle>,
    pub pickups: Vec<Pickup>,
    pub events: Vec<Event>,
    pub phase: Phase,
    /// Winning team, or None for a draw. Only meaningful in GameOver.
    pub winner: Option<u8>,
    pub spawn_timer: f32,
    pub end_timer: Option<f32>,
    pub time: f32,
    rng: Rng,
    next_pickup_id: u32,
}

fn wrap_angle(mut a: f32) -> f32 {
    while a > PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}

/// If a circle overlaps the rect, return the pushed-out centre and the
/// contact normal.
fn circle_rect(x: f32, y: f32, r: f32, o: &Obstacle) -> Option<(f32, f32, f32, f32)> {
    let cx = x.clamp(o.x, o.x + o.w);
    let cy = y.clamp(o.y, o.y + o.h);
    let dx = x - cx;
    let dy = y - cy;
    let d2 = dx * dx + dy * dy;
    if d2 >= r * r {
        return None;
    }
    if d2 > 1e-6 {
        let d = d2.sqrt();
        let (nx, ny) = (dx / d, dy / d);
        return Some((cx + nx * r, cy + ny * r, nx, ny));
    }
    // Centre is inside the rect: leave through the nearest face.
    let left = x - o.x;
    let right = o.x + o.w - x;
    let top = y - o.y;
    let bottom = o.y + o.h - y;
    let m = left.min(right).min(top).min(bottom);
    if m == left {
        Some((o.x - r, y, -1.0, 0.0))
    } else if m == right {
        Some((o.x + o.w + r, y, 1.0, 0.0))
    } else if m == top {
        Some((x, o.y - r, 0.0, -1.0))
    } else {
        Some((x, o.y + o.h + r, 0.0, 1.0))
    }
}

impl World {
    pub fn new_match(specs: &[PlayerSpec], seed: u64) -> World {
        let mut tanks = Vec::new();
        for team in 0..2u8 {
            let members: Vec<&PlayerSpec> = specs.iter().filter(|s| s.team == team).collect();
            let n = members.len();
            for (k, s) in members.iter().enumerate() {
                let y = ARENA_H * (k as f32 + 1.0) / (n as f32 + 1.0);
                let (x, angle) = if team == 0 { (90.0, 0.0) } else { (ARENA_W - 90.0, PI) };
                tanks.push(Tank::new(s.id, s.team, &s.name, x, y, angle, s.ai));
            }
        }
        World {
            tanks,
            bullets: Vec::new(),
            obstacles: default_obstacles(),
            pickups: Vec::new(),
            events: Vec::new(),
            phase: Phase::Playing,
            winner: None,
            spawn_timer: 5.0,
            end_timer: None,
            time: 0.0,
            rng: Rng::new(seed),
            next_pickup_id: 1,
        }
    }

    pub fn tank(&self, id: u32) -> Option<&Tank> {
        self.tanks.iter().find(|t| t.id == id)
    }
    pub fn tank_mut(&mut self, id: u32) -> Option<&mut Tank> {
        self.tanks.iter_mut().find(|t| t.id == id)
    }

    /// Remove a player mid-game (disconnect). Counts as being destroyed.
    pub fn kill_player(&mut self, id: u32) {
        if let Some(t) = self.tank_mut(id) {
            if t.alive {
                t.alive = false;
                t.hp = 0;
                let (x, y) = (t.x, t.y);
                self.events.push(Event { kind: EventKind::Explode, x, y, a: 0.0 });
            }
        }
    }

    fn alive_teams(&self) -> Vec<u8> {
        let mut teams: Vec<u8> = Vec::new();
        for t in self.tanks.iter().filter(|t| t.alive) {
            if !teams.contains(&t.team) {
                teams.push(t.team);
            }
        }
        teams
    }

    pub fn step(&mut self, dt: f32) {
        let dt = dt.min(0.05);
        self.time += dt;
        if self.phase == Phase::GameOver {
            // Bullets and particles may keep flying; tanks stop.
            self.update_bullets(dt);
            return;
        }

        self.run_ai(dt);

        let mut new_bullets = Vec::new();
        for t in self.tanks.iter_mut() {
            Self::update_tank(t, dt, &mut new_bullets, &mut self.events);
        }
        self.bullets.extend(new_bullets);

        self.collide_tanks();
        self.update_bullets(dt);
        self.update_pickups(dt);

        if self.tanks.len() >= 2 {
            if self.alive_teams().len() <= 1 {
                let t = self.end_timer.get_or_insert(END_DELAY);
                *t -= dt;
                if *t <= 0.0 {
                    self.phase = Phase::GameOver;
                    self.winner = self.alive_teams().first().copied();
                }
            } else {
                self.end_timer = None;
            }
        }
    }

    // ---- tank logic (shared by humans, remote players, and AI) ----------

    fn update_tank(t: &mut Tank, dt: f32, out: &mut Vec<Bullet>, events: &mut Vec<Event>) {
        if !t.alive {
            return;
        }
        // combat timers
        t.cooldown = (t.cooldown - dt).max(0.0);
        if t.reload > 0.0 {
            t.reload -= dt;
            if t.reload <= 0.0 {
                t.reload = 0.0;
                t.ammo = MAX_AMMO;
            }
        }
        t.speed_t = (t.speed_t - dt).max(0.0);
        t.rapid_t = (t.rapid_t - dt).max(0.0);
        t.shield_t = (t.shield_t - dt).max(0.0);

        if t.ctrl.drive {
            Self::update_drive(t, dt, out, events);
            return;
        }
        t.turret = t.angle;

        // Tap = fire and reverse spin. Hold = stop spinning and drive.
        if t.held {
            if !t.was_held {
                t.hold_timer = 0.0;
            }
            t.hold_timer += dt;
            if !t.moving && t.hold_timer >= TAP_THRESHOLD {
                t.moving = true;
            }
        } else if t.was_held {
            if !t.moving {
                Self::fire(t, out, events);
                t.rot_dir = -t.rot_dir;
            } else {
                t.moving = false;
            }
            t.hold_timer = 0.0;
        }
        t.was_held = t.held;

        if t.moving {
            let s = t.speed() * dt;
            t.x += t.angle.cos() * s;
            t.y += t.angle.sin() * s;
        } else {
            t.angle = wrap_angle(t.angle + t.rot_dir * TANK_ROT * dt);
        }
        t.turret = t.angle;
    }

    /// Chase-camera scheme: steer/throttle drive the hull, the turret slews
    /// toward `ctrl.aim`, `fire` auto-fires within the magazine, Shift boost
    /// burns stamina, and `reload` tops up the magazine on demand.
    fn update_drive(t: &mut Tank, dt: f32, out: &mut Vec<Bullet>, events: &mut Vec<Event>) {
        let c = t.ctrl;
        // leave the classic state machine clean in case the scheme switches
        t.moving = false;
        t.was_held = false;
        t.hold_timer = 0.0;

        t.angle = wrap_angle(t.angle + c.steer.clamp(-1.0, 1.0) * DRIVE_TURN * dt);
        let th = c.throttle.clamp(-1.0, 1.0);
        let mut speed = t.speed() * th.abs() * if th < 0.0 { REVERSE_MULT } else { 1.0 };

        if t.boost_lock && t.stamina >= 0.3 {
            t.boost_lock = false;
        }
        if c.boost && th != 0.0 && t.stamina > 0.0 && !t.boost_lock {
            speed *= BOOST_MULT;
            t.stamina = (t.stamina - BOOST_DRAIN * dt).max(0.0);
            if t.stamina <= 0.0 {
                t.boost_lock = true; // must recover a bit before boosting again
            }
        } else {
            t.stamina = (t.stamina + BOOST_REGEN * dt).min(1.0);
        }
        let dir = if th < 0.0 { -1.0 } else { 1.0 };
        t.x += t.angle.cos() * speed * dir * dt;
        t.y += t.angle.sin() * speed * dir * dt;

        let d = wrap_angle(c.aim - t.turret);
        let step = TURRET_RATE * dt;
        t.turret = wrap_angle(t.turret + d.clamp(-step, step));

        if c.reload && t.ammo < MAX_AMMO && !t.reloading() {
            t.reload = RELOAD_TIME;
        }
        if c.fire {
            Self::fire(t, out, events);
        }
    }

    fn fire(t: &mut Tank, out: &mut Vec<Bullet>, events: &mut Vec<Event>) -> bool {
        if !t.can_fire() {
            return false;
        }
        let (c, s) = (t.turret.cos(), t.turret.sin());
        let muzzle = TANK_R + 8.0;
        let (bx, by) = (t.x + c * muzzle, t.y + s * muzzle);
        out.push(Bullet {
            x: bx,
            y: by,
            vx: c * BULLET_SPEED,
            vy: s * BULLET_SPEED,
            bounces: 0,
            owner: t.id,
            team: t.team,
        });
        events.push(Event { kind: EventKind::Fire, x: bx, y: by, a: t.turret });
        t.ammo -= 1;
        t.cooldown = t.cooldown_len();
        if t.ammo <= 0 {
            t.reload = RELOAD_TIME;
        }
        true
    }

    fn collide_tanks(&mut self) {
        for t in self.tanks.iter_mut().filter(|t| t.alive) {
            for o in &self.obstacles {
                if let Some((nx, ny, _, _)) = circle_rect(t.x, t.y, TANK_R, o) {
                    t.x = nx;
                    t.y = ny;
                }
            }
            t.x = t.x.clamp(TANK_R, ARENA_W - TANK_R);
            t.y = t.y.clamp(TANK_R, ARENA_H - TANK_R);
        }
        let n = self.tanks.len();
        for i in 0..n {
            for j in (i + 1)..n {
                if !self.tanks[i].alive || !self.tanks[j].alive {
                    continue;
                }
                let dx = self.tanks[j].x - self.tanks[i].x;
                let dy = self.tanks[j].y - self.tanks[i].y;
                let d = (dx * dx + dy * dy).sqrt();
                let min = TANK_R * 2.0;
                if d < min && d > 1e-4 {
                    let push = (min - d) / 2.0;
                    let (ux, uy) = (dx / d, dy / d);
                    self.tanks[i].x -= ux * push;
                    self.tanks[i].y -= uy * push;
                    self.tanks[j].x += ux * push;
                    self.tanks[j].y += uy * push;
                }
            }
        }
    }

    fn update_bullets(&mut self, dt: f32) {
        let mut bullets = std::mem::take(&mut self.bullets);
        bullets.retain_mut(|b| self.step_bullet(b, dt));
        self.bullets = bullets;
    }

    /// Returns false when the bullet is spent.
    fn step_bullet(&mut self, b: &mut Bullet, dt: f32) -> bool {
        b.x += b.vx * dt;
        b.y += b.vy * dt;

        // Tanks. Teammates are never hurt (friendly-fire shield).
        for t in self.tanks.iter_mut() {
            if !t.alive || t.team == b.team {
                continue;
            }
            let dx = t.x - b.x;
            let dy = t.y - b.y;
            let r = TANK_R + BULLET_R;
            if dx * dx + dy * dy < r * r {
                self.events.push(Event { kind: EventKind::Hit, x: b.x, y: b.y, a: 0.0 });
                if t.shield_t <= 0.0 {
                    t.hp -= 1;
                    if t.hp <= 0 {
                        t.alive = false;
                        t.hp = 0;
                        self.events.push(Event { kind: EventKind::Explode, x: t.x, y: t.y, a: 0.0 });
                    }
                }
                return false;
            }
        }

        // Walls and obstacles: reflect, then die after MAX_BOUNCES.
        let bounced = Self::bounce(&self.obstacles, b);
        if bounced {
            self.events.push(Event { kind: EventKind::Bounce, x: b.x, y: b.y, a: 0.0 });
            b.bounces += 1;
            if b.bounces > MAX_BOUNCES {
                return false;
            }
        }
        true
    }

    /// Reflect a bullet off the arena walls and obstacles. True if it bounced.
    fn bounce(obstacles: &[Obstacle], b: &mut Bullet) -> bool {
        let mut bounced = false;
        if b.x < BULLET_R {
            b.x = BULLET_R;
            b.vx = b.vx.abs();
            bounced = true;
        } else if b.x > ARENA_W - BULLET_R {
            b.x = ARENA_W - BULLET_R;
            b.vx = -b.vx.abs();
            bounced = true;
        }
        if b.y < BULLET_R {
            b.y = BULLET_R;
            b.vy = b.vy.abs();
            bounced = true;
        } else if b.y > ARENA_H - BULLET_R {
            b.y = ARENA_H - BULLET_R;
            b.vy = -b.vy.abs();
            bounced = true;
        }
        for o in obstacles {
            if let Some((nx, ny, ux, uy)) = circle_rect(b.x, b.y, BULLET_R, o) {
                b.x = nx;
                b.y = ny;
                let dot = b.vx * ux + b.vy * uy;
                if dot < 0.0 {
                    b.vx -= 2.0 * dot * ux;
                    b.vy -= 2.0 * dot * uy;
                }
                bounced = true;
            }
        }
        bounced
    }

    /// Predict a shot fired from a tank at (x, y) along `angle`: the points
    /// where its path starts, bounces and ends, plus whether it ends on an
    /// enemy of `team`. Used for the aim line.
    pub fn trace_shot(&self, x: f32, y: f32, angle: f32, team: u8) -> (Vec<(f32, f32)>, bool) {
        let (c, s) = (angle.cos(), angle.sin());
        let muzzle = TANK_R + 8.0;
        let mut b = Bullet { x: x + c * muzzle, y: y + s * muzzle, vx: c * BULLET_SPEED, vy: s * BULLET_SPEED, bounces: 0, owner: 0, team };
        let mut pts = vec![(b.x, b.y)];
        let dt = 6.0 / BULLET_SPEED; // 6 units per step
        for _ in 0..500 {
            b.x += b.vx * dt;
            b.y += b.vy * dt;
            let r = TANK_R + BULLET_R;
            if self.tanks.iter().any(|t| t.alive && t.team != team && (t.x - b.x).hypot(t.y - b.y) < r) {
                pts.push((b.x, b.y));
                return (pts, true);
            }
            if Self::bounce(&self.obstacles, &mut b) {
                pts.push((b.x, b.y));
                b.bounces += 1;
                if b.bounces > MAX_BOUNCES {
                    return (pts, false);
                }
            }
        }
        pts.push((b.x, b.y));
        (pts, false)
    }

    // ---- pickups --------------------------------------------------------

    fn update_pickups(&mut self, dt: f32) {
        self.spawn_timer -= dt;
        if self.spawn_timer <= 0.0 {
            self.spawn_timer = self.rng.range(7.0, 13.0);
            if self.pickups.len() < MAX_PICKUPS {
                self.spawn_pickup();
            }
        }
        let mut taken: Vec<(usize, usize)> = Vec::new();
        for (pi, p) in self.pickups.iter().enumerate() {
            for (ti, t) in self.tanks.iter().enumerate() {
                if !t.alive {
                    continue;
                }
                let (dx, dy) = (t.x - p.x, t.y - p.y);
                let r = TANK_R + PICKUP_R;
                if dx * dx + dy * dy < r * r {
                    taken.push((pi, ti));
                    break;
                }
            }
        }
        for &(pi, ti) in taken.iter().rev() {
            // Index-safe: `taken` has one entry per pickup, ascending.
            let p = self.pickups.remove(pi);
            Self::apply_pickup(&mut self.tanks[ti], p.kind);
            self.events.push(Event { kind: EventKind::Pickup, x: p.x, y: p.y, a: p.kind.code() as f32 });
        }
    }

    pub fn apply_pickup(t: &mut Tank, kind: PickupKind) {
        match kind {
            PickupKind::Heal => t.hp = (t.hp + 1).min(TANK_HP),
            PickupKind::Ammo => {
                t.ammo = MAX_AMMO;
                t.reload = 0.0;
            }
            PickupKind::Speed => t.speed_t = BUFF_TIME,
            PickupKind::Rapid => t.rapid_t = BUFF_TIME,
            PickupKind::Shield => t.shield_t = SHIELD_TIME,
        }
    }

    fn spawn_pickup(&mut self) {
        for _ in 0..30 {
            let x = self.rng.range(70.0, ARENA_W - 70.0);
            let y = self.rng.range(70.0, ARENA_H - 70.0);
            let blocked = self.obstacles.iter().any(|o| circle_rect(x, y, PICKUP_R + 14.0, o).is_some());
            let near_tank = self.tanks.iter().any(|t| t.alive && (t.x - x).hypot(t.y - y) < 60.0);
            let near_pickup = self.pickups.iter().any(|p| (p.x - x).hypot(p.y - y) < 60.0);
            if !blocked && !near_tank && !near_pickup {
                let kind = PickupKind::ALL[self.rng.below(PickupKind::ALL.len())];
                let id = self.next_pickup_id;
                self.next_pickup_id += 1;
                self.pickups.push(Pickup { id, x, y, kind });
                return;
            }
        }
    }

    // ---- AI -------------------------------------------------------------

    fn run_ai(&mut self, dt: f32) {
        let snapshot: Vec<(u32, u8, f32, f32, bool)> =
            self.tanks.iter().map(|t| (t.id, t.team, t.x, t.y, t.alive)).collect();
        for i in 0..self.tanks.len() {
            let t = &mut self.tanks[i];
            let can_fire = t.can_fire();
            let Some(ai) = t.ai.as_mut() else { continue };
            if !t.alive {
                t.held = false;
                continue;
            }
            ai.fire_wait -= dt;
            ai.drive_wait -= dt;
            match ai.mode {
                AiMode::Tap(left) => {
                    t.held = true;
                    let left = left - dt;
                    ai.mode = if left <= 0.0 { AiMode::Idle } else { AiMode::Tap(left) };
                    if left <= 0.0 {
                        t.held = false;
                    }
                }
                AiMode::Drive(left) => {
                    t.held = true;
                    let left = left - dt;
                    if left <= 0.0 {
                        t.held = false;
                        ai.mode = AiMode::Idle;
                        ai.drive_wait = self.rng.range(1.2, 3.0);
                    } else {
                        ai.mode = AiMode::Drive(left);
                    }
                }
                AiMode::Idle => {
                    t.held = false;
                    // nearest enemy
                    let target = snapshot
                        .iter()
                        .filter(|s| s.4 && s.1 != t.team)
                        .min_by(|a, b| {
                            let da = (a.2 - t.x).hypot(a.3 - t.y);
                            let db = (b.2 - t.x).hypot(b.3 - t.y);
                            da.partial_cmp(&db).unwrap()
                        });
                    if let Some(&(_, _, tx, ty, _)) = target {
                        let diff = wrap_angle((ty - t.y).atan2(tx - t.x) - t.angle);
                        if diff.abs() < 0.07 && ai.fire_wait <= 0.0 && can_fire {
                            ai.mode = AiMode::Tap(0.07);
                            ai.fire_wait = self.rng.range(0.7, 1.6);
                            continue;
                        }
                    }
                    if ai.drive_wait <= 0.0 {
                        ai.mode = AiMode::Drive(self.rng.range(0.45, 1.1));
                    }
                }
            }
        }
    }

    // ---- network snapshots ---------------------------------------------

    pub fn encode_state(&self, events: &[Event]) -> String {
        let phase = if self.phase == Phase::Playing { 'P' } else { 'G' };
        let winner = self.winner.map(|w| w as i32).unwrap_or(-1);
        let tanks: Vec<String> = self
            .tanks
            .iter()
            .map(|t| {
                format!(
                    "{},{:.1},{:.1},{:.2},{},{},{},{:.1},{:.1},{:.1},{:.1},{:.2},{:.2}",
                    t.id,
                    t.x,
                    t.y,
                    t.angle,
                    t.hp,
                    t.alive as u8,
                    t.ammo,
                    t.reload,
                    t.speed_t,
                    t.rapid_t,
                    t.shield_t,
                    t.turret,
                    t.stamina
                )
            })
            .collect();
        let bullets: Vec<String> = self
            .bullets
            .iter()
            .map(|b| format!("{:.1},{:.1},{:.2}", b.x, b.y, b.vy.atan2(b.vx)))
            .collect();
        let pickups: Vec<String> =
            self.pickups.iter().map(|p| format!("{},{:.0},{:.0},{}", p.id, p.x, p.y, p.kind.code())).collect();
        let evs: Vec<String> =
            events.iter().map(|e| format!("{},{:.0},{:.0},{:.2}", e.kind.code(), e.x, e.y, e.a)).collect();
        format!("STATE:{}:{}:{}:{}:{}:{}", phase, winner, tanks.join(";"), bullets.join(";"), pickups.join(";"), evs.join(";"))
    }

    /// Apply a snapshot produced by `encode_state`. Returns the events it
    /// carried, or None if the packet is malformed.
    pub fn apply_state(&mut self, msg: &str) -> Option<Vec<Event>> {
        let parts: Vec<&str> = msg.splitn(7, ':').collect();
        if parts.len() != 7 || parts[0] != "STATE" {
            return None;
        }
        let pf = |s: &str| s.parse::<f32>().ok();
        self.phase = if parts[1] == "G" { Phase::GameOver } else { Phase::Playing };
        let w: i32 = parts[2].parse().ok()?;
        self.winner = if w >= 0 { Some(w as u8) } else { None };

        for rec in parts[3].split(';').filter(|s| !s.is_empty()) {
            let f: Vec<&str> = rec.split(',').collect();
            if f.len() != 13 {
                return None;
            }
            let id: u32 = f[0].parse().ok()?;
            if let Some(t) = self.tank_mut(id) {
                t.x = pf(f[1])?;
                t.y = pf(f[2])?;
                t.angle = pf(f[3])?;
                t.hp = f[4].parse().ok()?;
                t.alive = f[5] == "1";
                t.ammo = f[6].parse().ok()?;
                t.reload = pf(f[7])?;
                t.speed_t = pf(f[8])?;
                t.rapid_t = pf(f[9])?;
                t.shield_t = pf(f[10])?;
                t.turret = pf(f[11])?;
                t.stamina = pf(f[12])?;
            }
        }
        self.bullets.clear();
        for rec in parts[4].split(';').filter(|s| !s.is_empty()) {
            let f: Vec<&str> = rec.split(',').collect();
            if f.len() != 3 {
                return None;
            }
            let ang = pf(f[2])?;
            self.bullets.push(Bullet {
                x: pf(f[0])?,
                y: pf(f[1])?,
                vx: ang.cos() * BULLET_SPEED,
                vy: ang.sin() * BULLET_SPEED,
                bounces: 0,
                owner: 0,
                team: 0,
            });
        }
        self.pickups.clear();
        for rec in parts[5].split(';').filter(|s| !s.is_empty()) {
            let f: Vec<&str> = rec.split(',').collect();
            if f.len() != 4 {
                return None;
            }
            self.pickups.push(Pickup {
                id: f[0].parse().ok()?,
                x: pf(f[1])?,
                y: pf(f[2])?,
                kind: PickupKind::from_code(f[3].parse().ok()?)?,
            });
        }
        let mut events = Vec::new();
        for rec in parts[6].split(';').filter(|s| !s.is_empty()) {
            let f: Vec<&str> = rec.split(',').collect();
            if f.len() != 4 {
                continue;
            }
            if let (Some(kind), Some(x), Some(y), Some(a)) = (EventKind::from_code(f[0]), pf(f[1]), pf(f[2]), pf(f[3])) {
                events.push(Event { kind, x, y, a });
            }
        }
        Some(events)
    }
}

// -------------------------------------------------------------- tests ----

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: u32, team: u8, ai: bool) -> PlayerSpec {
        PlayerSpec { id, name: format!("P{id}"), team, ai }
    }

    fn duel() -> World {
        World::new_match(&[spec(0, 0, false), spec(1, 1, false)], 7)
    }

    /// Simulate a quick tap: held for 0.08s then released.
    fn tap(w: &mut World, id: u32) {
        w.tank_mut(id).unwrap().held = true;
        for _ in 0..5 {
            w.step(0.016);
        }
        w.tank_mut(id).unwrap().held = false;
        w.step(0.016);
    }

    #[test]
    fn tap_fires_and_reverses_spin() {
        let mut w = duel();
        let before = w.tank(0).unwrap().rot_dir;
        tap(&mut w, 0);
        assert_eq!(w.bullets.len(), 1);
        assert_eq!(w.tank(0).unwrap().ammo, MAX_AMMO - 1);
        assert_eq!(w.tank(0).unwrap().rot_dir, -before);
    }

    #[test]
    fn hold_drives_and_release_does_not_fire() {
        let mut w = duel();
        let x0 = w.tank(0).unwrap().x;
        w.tank_mut(0).unwrap().held = true;
        for _ in 0..60 {
            w.step(0.016);
        }
        w.tank_mut(0).unwrap().held = false;
        w.step(0.016);
        assert!(w.tank(0).unwrap().x != x0, "tank should have moved");
        assert!(w.bullets.is_empty(), "driving must not fire");
        assert!(!w.tank(0).unwrap().moving);
    }

    #[test]
    fn magazine_empties_then_reloads() {
        let mut w = duel();
        w.tank_mut(0).unwrap().cooldown = 0.0;
        for _ in 0..MAX_AMMO {
            tap(&mut w, 0);
            for _ in 0..20 {
                w.step(0.016); // let the cooldown pass
            }
            // keep the bullets from killing anyone: remove them
            w.bullets.clear();
        }
        let t = w.tank(0).unwrap();
        assert_eq!(t.ammo, 0);
        assert!(t.reloading());
        tap(&mut w, 0);
        assert!(w.bullets.is_empty(), "cannot fire while reloading");
        for _ in 0..120 {
            w.step(0.016);
        }
        let t = w.tank(0).unwrap();
        assert_eq!(t.ammo, MAX_AMMO);
        assert!(!t.reloading());
    }

    #[test]
    fn bullet_bounces_twice_then_dies() {
        let mut w = duel();
        // Free lane at the top, fired straight at the right wall.
        w.tanks.clear();
        w.bullets.push(Bullet { x: 500.0, y: 30.0, vx: BULLET_SPEED, vy: 0.0, bounces: 0, owner: 9, team: 0 });
        let mut bounces = 0;
        for _ in 0..600 {
            w.step(0.016);
            bounces += w.events.iter().filter(|e| e.kind == EventKind::Bounce).count();
            w.events.clear();
            if w.bullets.is_empty() {
                break;
            }
        }
        assert!(w.bullets.is_empty());
        assert_eq!(bounces as u32, MAX_BOUNCES + 1);
    }

    #[test]
    fn bullet_hits_enemy_but_not_teammate() {
        let specs = [spec(0, 0, false), spec(1, 0, false), spec(2, 1, false)];
        let mut w = World::new_match(&specs, 3);
        w.tank_mut(0).unwrap().x = 100.0;
        w.tank_mut(0).unwrap().y = 30.0;
        w.tank_mut(1).unwrap().x = 200.0;
        w.tank_mut(1).unwrap().y = 30.0;
        w.tank_mut(2).unwrap().x = 800.0;
        w.tank_mut(2).unwrap().y = 600.0;
        w.bullets.push(Bullet { x: 120.0, y: 30.0, vx: BULLET_SPEED, vy: 0.0, bounces: 0, owner: 0, team: 0 });
        for _ in 0..30 {
            w.step(0.016);
        }
        assert_eq!(w.tank(1).unwrap().hp, TANK_HP, "teammate must be immune");

        // Same lane, but the target is on the other team.
        let mut w = World::new_match(&specs, 3);
        w.tank_mut(0).unwrap().x = 100.0;
        w.tank_mut(0).unwrap().y = 30.0;
        w.tank_mut(1).unwrap().x = 800.0;
        w.tank_mut(1).unwrap().y = 600.0;
        w.tank_mut(2).unwrap().x = 200.0;
        w.tank_mut(2).unwrap().y = 30.0;
        w.bullets.push(Bullet { x: 120.0, y: 30.0, vx: BULLET_SPEED, vy: 0.0, bounces: 0, owner: 0, team: 0 });
        for _ in 0..30 {
            w.step(0.016);
        }
        assert_eq!(w.tank(2).unwrap().hp, TANK_HP - 1);
    }

    #[test]
    fn shield_blocks_damage() {
        let mut w = duel();
        w.tank_mut(1).unwrap().x = 200.0;
        w.tank_mut(1).unwrap().y = 30.0;
        w.tank_mut(1).unwrap().shield_t = 5.0;
        w.tank_mut(0).unwrap().y = 600.0;
        w.bullets.push(Bullet { x: 120.0, y: 30.0, vx: BULLET_SPEED, vy: 0.0, bounces: 0, owner: 0, team: 0 });
        for _ in 0..30 {
            w.step(0.016);
        }
        assert_eq!(w.tank(1).unwrap().hp, TANK_HP);
        assert!(w.bullets.is_empty(), "shield still absorbs the bullet");
    }

    #[test]
    fn pickups_apply_effects() {
        let mut t = Tank::new(0, 0, "x", 0.0, 0.0, 0.0, false);
        t.hp = 1;
        t.ammo = 0;
        t.reload = 1.0;
        World::apply_pickup(&mut t, PickupKind::Heal);
        World::apply_pickup(&mut t, PickupKind::Ammo);
        World::apply_pickup(&mut t, PickupKind::Speed);
        World::apply_pickup(&mut t, PickupKind::Rapid);
        World::apply_pickup(&mut t, PickupKind::Shield);
        assert_eq!(t.hp, 2);
        assert_eq!(t.ammo, MAX_AMMO);
        assert!(!t.reloading());
        assert!(t.speed() > TANK_SPEED);
        assert!(t.cooldown_len() < FIRE_COOLDOWN);
        assert!(t.shield_t > 0.0);
    }

    #[test]
    fn pickups_spawn_on_free_ground_and_are_collected() {
        let mut w = duel();
        for _ in 0..(20 * 60) {
            w.step(0.016);
            w.events.clear();
            // keep the match alive and tanks parked
            for t in w.tanks.iter_mut() {
                t.held = false;
            }
            assert!(w.pickups.len() <= MAX_PICKUPS);
            for p in &w.pickups {
                assert!(!w.obstacles.iter().any(|o| circle_rect(p.x, p.y, PICKUP_R, o).is_some()));
            }
        }
        assert!(!w.pickups.is_empty(), "something should have spawned");
        let p = w.pickups[0].clone();
        w.tank_mut(0).unwrap().x = p.x;
        w.tank_mut(0).unwrap().y = p.y;
        w.step(0.016);
        assert!(!w.pickups.iter().any(|q| q.id == p.id), "driving over it collects it");
        assert!(w.events.iter().any(|e| e.kind == EventKind::Pickup));
    }

    #[test]
    fn tanks_cannot_enter_obstacles_or_leave_arena() {
        let mut w = duel();
        let o = w.obstacles[0].clone();
        w.tank_mut(0).unwrap().x = o.x + o.w / 2.0;
        w.tank_mut(0).unwrap().y = o.y + o.h / 2.0;
        w.step(0.016);
        let t = w.tank(0).unwrap();
        assert!(circle_rect(t.x, t.y, TANK_R - 0.5, &o).is_none());
        w.tank_mut(0).unwrap().x = -500.0;
        w.step(0.016);
        assert!(w.tank(0).unwrap().x >= TANK_R);
    }

    #[test]
    fn game_ends_when_one_team_remains() {
        let mut w = duel();
        w.tank_mut(1).unwrap().alive = false;
        assert_eq!(w.phase, Phase::Playing);
        for _ in 0..200 {
            w.step(0.016);
        }
        assert_eq!(w.phase, Phase::GameOver);
        assert_eq!(w.winner, Some(0));
    }

    #[test]
    fn solo_world_does_not_end_immediately() {
        let mut w = World::new_match(&[spec(0, 0, false)], 1);
        for _ in 0..200 {
            w.step(0.016);
        }
        assert_eq!(w.phase, Phase::Playing);
    }

    #[test]
    fn ai_fights_and_game_terminates_or_stays_valid() {
        let specs = [spec(0, 0, true), spec(1, 1, true), spec(2, 1, true)];
        let mut w = World::new_match(&specs, 42);
        let mut shots = 0;
        for _ in 0..(90 * 60) {
            w.step(0.016);
            shots += w.events.iter().filter(|e| e.kind == EventKind::Fire).count();
            w.events.clear();
            for t in &w.tanks {
                assert!(t.x.is_finite() && t.y.is_finite());
                assert!(t.x >= 0.0 && t.x <= ARENA_W && t.y >= 0.0 && t.y <= ARENA_H);
            }
            if w.phase == Phase::GameOver {
                break;
            }
        }
        assert!(shots > 5, "AI should shoot (shots={shots})");
    }

    #[test]
    fn snapshot_roundtrip() {
        let specs = [spec(0, 0, false), spec(1, 1, false)];
        let mut a = World::new_match(&specs, 5);
        a.tank_mut(0).unwrap().hp = 2;
        a.tank_mut(0).unwrap().shield_t = 3.2;
        a.tank_mut(1).unwrap().alive = false;
        a.bullets.push(Bullet { x: 12.5, y: 99.0, vx: 0.0, vy: BULLET_SPEED, bounces: 1, owner: 0, team: 0 });
        a.pickups.push(Pickup { id: 4, x: 300.0, y: 200.0, kind: PickupKind::Rapid });
        let evs = [Event { kind: EventKind::Explode, x: 10.0, y: 20.0, a: 0.0 }];
        let msg = a.encode_state(&evs);

        let mut b = World::new_match(&specs, 99);
        let got = b.apply_state(&msg).expect("parse");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, EventKind::Explode);
        assert_eq!(b.tank(0).unwrap().hp, 2);
        assert!((b.tank(0).unwrap().shield_t - 3.2).abs() < 0.11);
        assert!(!b.tank(1).unwrap().alive);
        assert_eq!(b.bullets.len(), 1);
        assert!((b.bullets[0].vy - BULLET_SPEED).abs() < 1.0);
        assert_eq!(b.pickups[0].kind, PickupKind::Rapid);
        assert!(b.apply_state("STATE:garbage").is_none());
    }

    #[test]
    fn team_spawns_are_on_their_own_side() {
        let specs = [spec(0, 0, false), spec(1, 0, false), spec(2, 1, false), spec(3, 1, false)];
        let w = World::new_match(&specs, 1);
        for t in &w.tanks {
            if t.team == 0 {
                assert!(t.x < 150.0);
            } else {
                assert!(t.x > 810.0);
            }
        }
    }

    fn drive_ctrl(throttle: f32, steer: f32, aim: f32) -> Control {
        Control { drive: true, throttle, steer, aim, ..Default::default() }
    }

    #[test]
    fn drive_throttle_and_steer_move_the_hull() {
        let mut w = duel();
        let (x0, a0) = (w.tank(0).unwrap().x, w.tank(0).unwrap().angle);
        w.tank_mut(0).unwrap().ctrl = drive_ctrl(1.0, 0.0, 0.0);
        for _ in 0..30 {
            w.step(0.016);
        }
        assert!(w.tank(0).unwrap().x > x0 + 20.0, "W drives forward");
        assert!((w.tank(0).unwrap().angle - a0).abs() < 1e-4, "no steering, no turning");
        let x1 = w.tank(0).unwrap().x;
        w.tank_mut(0).unwrap().ctrl = drive_ctrl(-1.0, 0.0, 0.0);
        for _ in 0..30 {
            w.step(0.016);
        }
        assert!(w.tank(0).unwrap().x < x1, "S reverses");
        let a1 = w.tank(0).unwrap().angle;
        w.tank_mut(0).unwrap().ctrl = drive_ctrl(0.0, 1.0, 0.0);
        for _ in 0..30 {
            w.step(0.016);
        }
        assert!(w.tank(0).unwrap().angle > a1 + 0.5, "steer turns the hull");
    }

    #[test]
    fn drive_reverse_is_slower_than_forward() {
        let mut f = duel();
        let mut r = duel();
        f.tank_mut(0).unwrap().ctrl = drive_ctrl(1.0, 0.0, 0.0);
        r.tank_mut(0).unwrap().ctrl = drive_ctrl(-1.0, 0.0, 0.0);
        for w in [&mut f, &mut r] {
            w.tank_mut(0).unwrap().x = 500.0;
            w.tank_mut(0).unwrap().y = 600.0; // clear lane, away from the central rock
        }
        for _ in 0..20 {
            f.step(0.016);
            r.step(0.016);
        }
        let fwd = f.tank(0).unwrap().x - 500.0;
        let back = 500.0 - r.tank(0).unwrap().x;
        assert!(fwd > 0.0 && back > 0.0, "forward moves +x, reverse moves -x (fwd={fwd}, back={back})");
        assert!(fwd > back * 1.4, "reverse is slower (fwd={fwd}, back={back})");
    }

    #[test]
    fn turret_aims_independently_of_hull_and_fires_that_way() {
        let mut w = duel();
        w.tank_mut(0).unwrap().ctrl = Control { drive: true, aim: 1.0, ..Default::default() };
        for _ in 0..60 {
            w.step(0.016);
        }
        let t = w.tank(0).unwrap();
        assert!((t.turret - 1.0).abs() < 0.01, "turret slewed to the aim angle");
        assert!(t.angle.abs() < 1e-4, "hull did not move");
        w.tank_mut(0).unwrap().ctrl.fire = true;
        w.step(0.016);
        let b = &w.bullets[0];
        assert!((b.vy.atan2(b.vx) - 1.0).abs() < 0.01, "bullet follows the turret, not the hull");
    }

    #[test]
    fn turret_slews_at_a_limited_rate() {
        let mut w = duel();
        w.tank_mut(0).unwrap().ctrl = Control { drive: true, aim: 3.0, ..Default::default() };
        w.step(0.016);
        assert!(w.tank(0).unwrap().turret < 0.2, "cannot snap instantly");
    }

    #[test]
    fn held_fire_autofires_until_the_magazine_is_empty_then_reloads() {
        let mut w = duel();
        w.tank_mut(0).unwrap().y = 600.0; // keep clear of the other tank's lane
        w.tank_mut(1).unwrap().y = 40.0;
        w.tank_mut(0).unwrap().ctrl = Control { drive: true, fire: true, aim: 0.0, ..Default::default() };
        let mut shots = 0;
        for _ in 0..90 {
            w.step(0.016);
            shots += w.events.iter().filter(|e| e.kind == EventKind::Fire).count();
            w.events.clear();
            w.bullets.clear();
        }
        assert_eq!(shots, MAX_AMMO as usize, "exactly one magazine before the reload starts");
        assert!(w.tank(0).unwrap().reloading());
    }

    #[test]
    fn boost_is_faster_burns_stamina_and_locks_when_empty() {
        let mut a = duel();
        let mut b = duel();
        a.tank_mut(0).unwrap().ctrl = drive_ctrl(1.0, 0.0, 0.0);
        b.tank_mut(0).unwrap().ctrl = Control { boost: true, ..drive_ctrl(1.0, 0.0, 0.0) };
        for _ in 0..20 {
            a.step(0.016);
            b.step(0.016);
        }
        assert!(b.tank(0).unwrap().x > a.tank(0).unwrap().x + 10.0, "boost is faster");
        assert!(b.tank(0).unwrap().stamina < 1.0);

        // hold boost until empty, in a roomy lane
        let mut w = duel();
        w.tank_mut(0).unwrap().y = 600.0;
        w.tank_mut(0).unwrap().angle = 0.0;
        w.tank_mut(0).unwrap().ctrl = Control { boost: true, steer: 0.0, ..drive_ctrl(1.0, 0.0, 0.0) };
        let mut min = 1.0f32;
        for _ in 0..80 {
            w.step(0.016);
            let t = w.tank_mut(0).unwrap();
            if t.x > 800.0 {
                t.x = 100.0;
            }
            min = min.min(t.stamina);
        }
        assert!(min < 0.05, "stamina ran out (min={min})");
        assert!(w.tank(0).unwrap().stamina < 0.5, "and is not instantly boosting again");
        w.tank_mut(0).unwrap().ctrl = drive_ctrl(0.0, 0.0, 0.0);
        for _ in 0..400 {
            w.step(0.016);
        }
        assert!(w.tank(0).unwrap().stamina > 0.99, "stamina regenerates");
    }

    #[test]
    fn manual_reload_refills_the_magazine() {
        let mut w = duel();
        w.tank_mut(0).unwrap().ammo = 2;
        w.tank_mut(0).unwrap().ctrl = Control { drive: true, reload: true, ..Default::default() };
        w.step(0.016);
        assert!(w.tank(0).unwrap().reloading());
        w.tank_mut(0).unwrap().ctrl.reload = false;
        for _ in 0..120 {
            w.step(0.016);
        }
        assert_eq!(w.tank(0).unwrap().ammo, MAX_AMMO);
    }

    #[test]
    fn switching_back_to_classic_snaps_the_turret_to_the_hull() {
        let mut w = duel();
        w.tank_mut(0).unwrap().ctrl = Control { drive: true, aim: 2.0, ..Default::default() };
        for _ in 0..60 {
            w.step(0.016);
        }
        w.tank_mut(0).unwrap().ctrl = Control::default();
        w.step(0.016);
        let t = w.tank(0).unwrap();
        assert!((t.turret - t.angle).abs() < 1e-4);
    }

    #[test]
    fn control_roundtrips_and_rejects_garbage() {
        let c = Control { drive: true, held: false, steer: -1.0, throttle: 0.5, fire: true, boost: true, reload: false, aim: 2.25 };
        assert_eq!(Control::decode(&c.encode()), Some(c));
        assert_eq!(Control::decode("1"), Some(Control::classic(true)));
        assert_eq!(Control::decode("0"), Some(Control::classic(false)));
        assert_eq!(Control::decode("1,0,9.0,-9.0,0,0,0,0.0").map(|c| (c.steer, c.throttle)), Some((1.0, -1.0)), "clamped");
        assert!(Control::decode("nonsense").is_none());
        assert!(Control::decode("1,0,x,0,0,0,0,0").is_none());
        assert!(Control::decode("1,0,NaN,0,0,0,0,0").is_none());
    }

    #[test]
    fn trace_shot_follows_bounces_and_flags_enemy_hits() {
        let w = duel();
        // Straight along the top lane: bounces off the far wall and comes back.
        let (pts, hit) = w.trace_shot(100.0, 20.0, 0.0, 0);
        assert!(!hit);
        assert!(pts.len() >= 3, "start, bounce(s), end: {pts:?}");
        assert!(pts.iter().any(|p| p.0 > ARENA_W - 10.0), "reaches the far wall");
        // Aimed straight at the enemy tank along a clear lane (the middle is
        // blocked by the central rock).
        let mut w = duel();
        w.tank_mut(0).unwrap().y = 600.0;
        w.tank_mut(1).unwrap().y = 600.0;
        let (pts, hit) = w.trace_shot(90.0, 600.0, 0.0, 0);
        assert!(hit, "line ends on the enemy");
        assert_eq!(pts.len(), 2, "no bounces on the way: {pts:?}");
        assert!(pts.last().unwrap().0 > 800.0);
        // The same shot is not flagged when the target is a teammate.
        w.tank_mut(1).unwrap().team = 0;
        let (_, hit) = w.trace_shot(90.0, 600.0, 0.0, 0);
        assert!(!hit, "teammates are passed through");
    }

    #[test]
    fn snapshot_carries_turret_and_stamina() {
        let specs = [spec(0, 0, false), spec(1, 1, false)];
        let mut a = World::new_match(&specs, 5);
        a.tank_mut(0).unwrap().turret = 1.25;
        a.tank_mut(0).unwrap().stamina = 0.4;
        let msg = a.encode_state(&[]);
        let mut b = World::new_match(&specs, 9);
        b.apply_state(&msg).expect("parse");
        assert!((b.tank(0).unwrap().turret - 1.25).abs() < 0.01);
        assert!((b.tank(0).unwrap().stamina - 0.4).abs() < 0.01);
    }
}

//! Capture the Flag tournament rules, layered on the shared `World`.
//!
//! Each team owns a flag at its home base. Carry the enemy flag back to your
//! own base while your flag is safe at home to score. Destroyed tanks respawn
//! after a few seconds. Mystery boxes give one secret item at a time (bomb,
//! shield or spread shot), used with the item key. Scores stay hidden until
//! the tournament clock runs out.

use std::collections::VecDeque;

use crate::sim::*;

/// Length of one tournament match.
pub const TOURNAMENT_SECS: f32 = 240.0;
pub const RESPAWN_SECS: f32 = 4.0;
pub const SPAWN_SHIELD: f32 = 2.0;
pub const FLAG_PICK_R: f32 = 30.0;
pub const FLAG_HOME_R: f32 = 40.0;
pub const FLAG_RETURN_SECS: f32 = 12.0;
pub const MINE_ARM: f32 = 1.0;
pub const MINE_LIFE: f32 = 25.0;
pub const MINE_TRIGGER: f32 = 30.0;
pub const MINE_BLAST: f32 = 70.0;
pub const MINE_DAMAGE: i32 = 2;
pub const SPREAD_TIME: f32 = 8.0;
/// Bot ids start here so they never clash with humans.
pub const BOT_ID_BASE: u32 = 100;

// ---- held items ---------------------------------------------------------

pub const ITEM_NONE: u8 = 0;
pub const ITEM_BOMB: u8 = 1;
pub const ITEM_SHIELD: u8 = 2;
pub const ITEM_SPREAD: u8 = 3;

pub fn item_label(code: u8) -> &'static str {
    match code {
        ITEM_BOMB => "BOMB",
        ITEM_SHIELD => "SHIELD",
        ITEM_SPREAD => "SPREAD",
        _ => "",
    }
}

pub fn item_rgb(code: u8) -> (f32, f32, f32) {
    match code {
        ITEM_BOMB => (1.0, 0.45, 0.2),
        ITEM_SHIELD => (0.35, 0.75, 1.0),
        ITEM_SPREAD => (0.95, 0.85, 0.3),
        _ => (0.7, 0.7, 0.7),
    }
}

// ---- state --------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Flag {
    pub home: (f32, f32),
    pub x: f32,
    pub y: f32,
    pub carrier: Option<u32>,
    /// Seconds the flag has lain dropped away from home.
    pub away: f32,
}

impl Flag {
    fn new(home: (f32, f32)) -> Flag {
        Flag { home, x: home.0, y: home.1, carrier: None, away: 0.0 }
    }
    pub fn at_home(&self) -> bool {
        self.carrier.is_none() && (self.x - self.home.0).abs() < 0.5 && (self.y - self.home.1).abs() < 0.5
    }
    fn reset(&mut self) {
        self.x = self.home.0;
        self.y = self.home.1;
        self.carrier = None;
        self.away = 0.0;
    }
}

#[derive(Clone, Debug)]
pub struct Mine {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub team: u8,
    pub owner: u32,
    pub arm: f32,
    pub life: f32,
}

impl Mine {
    pub fn armed(&self) -> bool {
        self.arm <= 0.0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PStat {
    pub id: u32,
    pub kills: u32,
    pub deaths: u32,
    pub caps: u32,
    pub returns: u32,
}

/// Cheap bot navigation: a coarse grid of cells a tank can stand on.
#[derive(Clone, Debug)]
struct Nav {
    cols: usize,
    rows: usize,
    blocked: Vec<bool>,
}

const CELL: f32 = 20.0;

impl Nav {
    fn build(obstacles: &[Obstacle]) -> Nav {
        let (cols, rows) = ((ARENA_W / CELL) as usize, (ARENA_H / CELL) as usize);
        let mut blocked = vec![false; cols * rows];
        for r in 0..rows {
            for c in 0..cols {
                let (x, y) = ((c as f32 + 0.5) * CELL, (r as f32 + 0.5) * CELL);
                let edge = x < TANK_R + 2.0 || y < TANK_R + 2.0 || x > ARENA_W - TANK_R - 2.0 || y > ARENA_H - TANK_R - 2.0;
                blocked[r * cols + c] = edge || obstacles.iter().any(|o| o.solid_tank() && circle_rect(x, y, TANK_R + 3.0, o).is_some());
            }
        }
        Nav { cols, rows, blocked }
    }

    fn cell(&self, x: f32, y: f32) -> (usize, usize) {
        (((x / CELL) as usize).min(self.cols - 1), ((y / CELL) as usize).min(self.rows - 1))
    }

    fn free(&self, c: usize, r: usize) -> bool {
        !self.blocked[r * self.cols + c]
    }

    /// Nearest unblocked cell to (c, r), searching outwards.
    fn nearest_free(&self, c: usize, r: usize) -> (usize, usize) {
        if self.free(c, r) {
            return (c, r);
        }
        for d in 1..12i32 {
            for dr in -d..=d {
                for dc in -d..=d {
                    if dr.abs().max(dc.abs()) != d {
                        continue;
                    }
                    let (nc, nr) = (c as i32 + dc, r as i32 + dr);
                    if nc >= 0 && nr >= 0 && (nc as usize) < self.cols && (nr as usize) < self.rows && self.free(nc as usize, nr as usize) {
                        return (nc as usize, nr as usize);
                    }
                }
            }
        }
        (c, r)
    }

    /// Breadth-first path from `from` to `to` as cell-centre waypoints.
    fn path(&self, from: (f32, f32), to: (f32, f32)) -> Vec<(f32, f32)> {
        let s = self.cell(from.0, from.1);
        let g = self.nearest_free(self.cell(to.0, to.1).0, self.cell(to.0, to.1).1);
        let mut prev = vec![usize::MAX; self.cols * self.rows];
        let idx = |c: usize, r: usize| r * self.cols + c;
        let mut q = VecDeque::new();
        prev[idx(s.0, s.1)] = idx(s.0, s.1);
        q.push_back(s);
        let mut found = false;
        while let Some((c, r)) = q.pop_front() {
            if (c, r) == g {
                found = true;
                break;
            }
            for (dc, dr) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let (nc, nr) = (c as i32 + dc, r as i32 + dr);
                if nc < 0 || nr < 0 || nc as usize >= self.cols || nr as usize >= self.rows {
                    continue;
                }
                let (nc, nr) = (nc as usize, nr as usize);
                if !self.free(nc, nr) || prev[idx(nc, nr)] != usize::MAX {
                    continue;
                }
                // no cutting corners around blocked cells
                if dc != 0 && dr != 0 && (!self.free(c, nr) || !self.free(nc, r)) {
                    continue;
                }
                prev[idx(nc, nr)] = idx(c, r);
                q.push_back((nc, nr));
            }
        }
        if !found {
            return vec![to];
        }
        let mut out = vec![to];
        let mut cur = idx(g.0, g.1);
        while cur != idx(s.0, s.1) {
            out.push((((cur % self.cols) as f32 + 0.5) * CELL, ((cur / self.cols) as f32 + 0.5) * CELL));
            cur = prev[cur];
        }
        out.reverse();
        out
    }
}

#[derive(Clone, Debug, Default)]
struct Bot {
    id: u32,
    path: Vec<(f32, f32)>,
    repath: f32,
    last_pos: (f32, f32),
    stuck: f32,
    unstick: f32,
    seen_for: f32,
}

#[derive(Clone, Debug)]
pub struct Ctf {
    pub flags: [Flag; 2],
    pub mines: Vec<Mine>,
    /// Captures per team. Hidden from clients until the match ends.
    pub scores: [u32; 2],
    pub time_left: f32,
    pub stats: Vec<PStat>,
    next_mine: u32,
    nav: Nav,
    bots: Vec<Bot>,
}

impl Ctf {
    pub fn new(w: &World) -> Ctf {
        Ctf {
            flags: [Flag::new((46.0, ARENA_H / 2.0)), Flag::new((ARENA_W - 46.0, ARENA_H / 2.0))],
            mines: Vec::new(),
            scores: [0, 0],
            time_left: TOURNAMENT_SECS,
            stats: w.tanks.iter().map(|t| PStat { id: t.id, ..Default::default() }).collect(),
            next_mine: 1,
            nav: Nav::build(&w.obstacles),
            bots: Vec::new(),
        }
    }

    pub fn stat(&self, id: u32) -> PStat {
        self.stats.iter().find(|s| s.id == id).cloned().unwrap_or(PStat { id, ..Default::default() })
    }

    fn stat_mut(&mut self, id: u32) -> &mut PStat {
        if let Some(i) = self.stats.iter().position(|s| s.id == id) {
            return &mut self.stats[i];
        }
        self.stats.push(PStat { id, ..Default::default() });
        self.stats.last_mut().unwrap()
    }

    /// The tournament section appended to snapshots. Scores and stats are only
    /// included once the match is over, so they stay a surprise.
    pub fn encode(&self, w: &World) -> String {
        let fl: Vec<String> = self
            .flags
            .iter()
            .map(|f| format!("{:.1},{:.1},{}", f.x, f.y, f.carrier.map(|c| c as i64).unwrap_or(-1)))
            .collect();
        let mines: Vec<String> =
            self.mines.iter().map(|m| format!("{},{:.0},{:.0},{},{}", m.id, m.x, m.y, m.team, m.armed() as u8)).collect();
        let tx: Vec<String> = w.tanks.iter().map(|t| format!("{},{},{:.1},{:.1}", t.id, t.item, t.spread_t, t.respawn_t)).collect();
        let fin = if w.phase == Phase::GameOver {
            let st: Vec<String> = self.stats.iter().map(|s| format!("{},{},{},{},{}", s.id, s.kills, s.deaths, s.caps, s.returns)).collect();
            format!("{},{}~{}", self.scores[0], self.scores[1], st.join(";"))
        } else {
            String::new()
        };
        let gates: Vec<String> = w.gates.iter().map(|g| format!("{:.2}", g.timer)).collect();
        format!("{:.1}|{}|{}|{}|{}|{}", self.time_left, fl.join(";"), mines.join(";"), tx.join(";"), fin, gates.join(";"))
    }
}

/// Apply the tournament section of a snapshot on a client.
pub fn apply(w: &mut World, sec: &str) {
    let parts: Vec<&str> = sec.split('|').collect();
    if parts.len() < 5 {
        return;
    }
    let pf = |s: &str| s.parse::<f32>().ok();
    // per-tank extras first (needs &mut tanks)
    for rec in parts[3].split(';').filter(|s| !s.is_empty()) {
        let f: Vec<&str> = rec.split(',').collect();
        if f.len() != 4 {
            continue;
        }
        if let (Ok(id), Ok(item), Some(sp), Some(rs)) = (f[0].parse::<u32>(), f[1].parse::<u8>(), pf(f[2]), pf(f[3])) {
            if let Some(t) = w.tank_mut(id) {
                t.item = item;
                t.spread_t = sp;
                t.respawn_t = rs;
            }
        }
    }
    if let Some(g) = parts.get(5) {
        for (i, v) in g.split(';').filter(|s| !s.is_empty()).enumerate() {
            if let (Some(gate), Some(t)) = (w.gates.get_mut(i), pf(v)) {
                gate.timer = t;
            }
        }
    }
    let Some(c) = w.ctf.as_mut() else { return };
    if let Some(t) = pf(parts[0]) {
        c.time_left = t;
    }
    for (i, rec) in parts[1].split(';').enumerate().take(2) {
        let f: Vec<&str> = rec.split(',').collect();
        if f.len() == 3 {
            if let (Some(x), Some(y), Ok(car)) = (pf(f[0]), pf(f[1]), f[2].parse::<i64>()) {
                c.flags[i].x = x;
                c.flags[i].y = y;
                c.flags[i].carrier = if car >= 0 { Some(car as u32) } else { None };
            }
        }
    }
    c.mines.clear();
    for rec in parts[2].split(';').filter(|s| !s.is_empty()) {
        let f: Vec<&str> = rec.split(',').collect();
        if f.len() == 5 {
            if let (Ok(id), Some(x), Some(y), Ok(team), armed) = (f[0].parse::<u32>(), pf(f[1]), pf(f[2]), f[3].parse::<u8>(), f[4] == "1") {
                c.mines.push(Mine { id, x, y, team, owner: 0, arm: if armed { 0.0 } else { 0.5 }, life: 10.0 });
            }
        }
    }
    if let Some((sc, st)) = parts[4].split_once('~') {
        if let Some((a, b)) = sc.split_once(',') {
            if let (Ok(a), Ok(b)) = (a.parse(), b.parse()) {
                c.scores = [a, b];
            }
        }
        c.stats.clear();
        for rec in st.split(';').filter(|s| !s.is_empty()) {
            let f: Vec<u32> = rec.split(',').filter_map(|x| x.parse().ok()).collect();
            if f.len() == 5 {
                c.stats.push(PStat { id: f[0], kills: f[1], deaths: f[2], caps: f[3], returns: f[4] });
            }
        }
    }
}

// ---- world integration --------------------------------------------------

impl World {
    /// A capture-the-flag match for the given players.
    pub fn new_ctf(specs: &[PlayerSpec], seed: u64) -> World {
        World::build(specs, seed, true, 1)
    }

    /// Any mode in any arena: `ctf` picks capture the flag, `arena` the map.
    pub fn build(specs: &[PlayerSpec], seed: u64, ctf: bool, arena: u8) -> World {
        let mut w = World::new_match(specs, seed);
        w.set_arena(arena);
        if ctf {
            w.ctf = Some(Ctf::new(&w));
            w.spawn_timer = 2.0;
        }
        w
    }

    /// Per-step tournament rules: item use, deaths and respawns, flags,
    /// mines and the clock.
    pub(crate) fn ctf_update(&mut self, dt: f32) {
        self.ctf_items();
        self.ctf_deaths(dt);
        self.ctf_flags(dt);
        self.ctf_mines(dt);
        let Some(c) = self.ctf.as_mut() else { return };
        c.time_left -= dt;
        if c.time_left <= 0.0 {
            c.time_left = 0.0;
            let (a, b) = (c.scores[0], c.scores[1]);
            let kills = |team: u8, w: &World, c: &Ctf| -> u32 { w.tanks.iter().filter(|t| t.team == team).map(|t| c.stat(t.id).kills).sum() };
            let c2 = c.clone();
            self.winner = if a != b {
                Some(if a > b { 0 } else { 1 })
            } else {
                // tie-break on kills, then call it a draw
                let (ka, kb) = (kills(0, self, &c2), kills(1, self, &c2));
                if ka != kb {
                    Some(if ka > kb { 0 } else { 1 })
                } else {
                    None
                }
            };
            self.phase = Phase::GameOver;
        }
    }

    fn ctf_items(&mut self) {
        let mut new_mines: Vec<Mine> = Vec::new();
        let mut evs: Vec<Event> = Vec::new();
        for t in self.tanks.iter_mut() {
            let edge = t.ctrl.use_item && !t.item_prev;
            t.item_prev = t.ctrl.use_item;
            if !edge || !t.alive || t.item == ITEM_NONE {
                continue;
            }
            match t.item {
                ITEM_BOMB => {
                    let (bx, by) = (t.x - t.angle.cos() * TANK_R * 1.7, t.y - t.angle.sin() * TANK_R * 1.7);
                    new_mines.push(Mine { id: 0, x: bx, y: by, team: t.team, owner: t.id, arm: MINE_ARM, life: MINE_LIFE });
                }
                ITEM_SHIELD => t.shield_t = t.shield_t.max(SHIELD_TIME),
                ITEM_SPREAD => t.spread_t = SPREAD_TIME,
                _ => {}
            }
            evs.push(Event { kind: EventKind::Pickup, x: t.x, y: t.y, a: 5.0 });
            t.item = ITEM_NONE;
        }
        self.events.extend(evs);
        if let Some(c) = self.ctf.as_mut() {
            for mut m in new_mines {
                m.id = c.next_mine;
                c.next_mine += 1;
                c.mines.push(m);
            }
        }
    }

    fn ctf_deaths(&mut self, dt: f32) {
        let mut respawn: Vec<usize> = Vec::new();
        let mut dropped: Vec<(usize, u32)> = Vec::new();
        let mut kills: Vec<u32> = Vec::new();
        let mut deaths: Vec<u32> = Vec::new();
        for (i, t) in self.tanks.iter_mut().enumerate() {
            if t.alive {
                continue;
            }
            if !t.dead_seen {
                t.dead_seen = true;
                t.respawn_t = RESPAWN_SECS;
                t.item = ITEM_NONE;
                t.spread_t = 0.0;
                deaths.push(t.id);
                if let Some(k) = t.last_hit.take() {
                    if k != t.id && k != GATE_OWNER {
                        kills.push(k);
                    }
                }
                dropped.push((i, t.id));
            } else if !t.left {
                t.respawn_t -= dt;
                if t.respawn_t <= 0.0 {
                    respawn.push(i);
                }
            }
        }
        for i in respawn {
            let t = &mut self.tanks[i];
            t.alive = true;
            t.hp = t.max_hp();
            t.ammo = t.max_ammo();
            t.reload = 0.0;
            t.cooldown = 0.0;
            t.x = t.spawn.0;
            t.y = t.spawn.1;
            t.angle = t.spawn.2;
            t.turret = t.spawn.2;
            t.stamina = 1.0;
            t.shield_t = SPAWN_SHIELD;
            t.respawn_t = 0.0;
            t.dead_seen = false;
            t.held = false;
            t.moving = false;
        }
        let Some(c) = self.ctf.as_mut() else { return };
        for id in deaths {
            c.stat_mut(id).deaths += 1;
        }
        for id in kills {
            c.stat_mut(id).kills += 1;
        }
        for (_, id) in dropped {
            for f in c.flags.iter_mut() {
                if f.carrier == Some(id) {
                    f.carrier = None;
                    f.away = 0.0;
                }
            }
        }
    }

    fn ctf_flags(&mut self, dt: f32) {
        let Some(mut c) = self.ctf.take() else { return };
        let mut evs: Vec<Event> = Vec::new();
        for team in 0..2usize {
            // carried flags follow their carrier
            if let Some(id) = c.flags[team].carrier {
                match self.tank(id) {
                    Some(t) if t.alive => {
                        c.flags[team].x = t.x;
                        c.flags[team].y = t.y;
                    }
                    _ => {
                        c.flags[team].carrier = None;
                        c.flags[team].away = 0.0;
                    }
                }
                continue;
            }
            let (fx, fy) = (c.flags[team].x, c.flags[team].y);
            let near = |t: &Tank| t.alive && (t.x - fx).hypot(t.y - fy) < FLAG_PICK_R;
            if !c.flags[team].at_home() {
                c.flags[team].away += dt;
                // a teammate touching it sends it home
                if let Some(t) = self.tanks.iter().find(|t| t.team as usize == team && near(t)) {
                    let id = t.id;
                    c.flags[team].reset();
                    c.stat_mut(id).returns += 1;
                    evs.push(Event { kind: EventKind::Pickup, x: fx, y: fy, a: 5.0 });
                    continue;
                }
                if c.flags[team].away >= FLAG_RETURN_SECS {
                    c.flags[team].reset();
                    continue;
                }
            }
            // an enemy touching it picks it up
            if let Some(t) = self.tanks.iter().find(|t| t.team as usize != team && near(t)) {
                c.flags[team].carrier = Some(t.id);
                c.flags[team].away = 0.0;
                evs.push(Event { kind: EventKind::Pickup, x: fx, y: fy, a: 5.0 });
            }
        }
        // capture: carrier reaches home while its own flag is safe there
        for team in 0..2usize {
            let enemy = 1 - team;
            let Some(id) = c.flags[enemy].carrier else { continue };
            let Some(t) = self.tank(id) else { continue };
            let home = c.flags[team].home;
            if t.alive && c.flags[team].at_home() && (t.x - home.0).hypot(t.y - home.1) < FLAG_HOME_R {
                c.scores[team] += 1;
                c.flags[enemy].reset();
                c.stat_mut(id).caps += 1;
                evs.push(Event { kind: EventKind::Pickup, x: home.0, y: home.1, a: 5.0 });
            }
        }
        self.events.extend(evs);
        self.ctf = Some(c);
    }

    fn ctf_mines(&mut self, dt: f32) {
        let Some(mut c) = self.ctf.take() else { return };
        let mut boom: Vec<usize> = Vec::new();
        for (i, m) in c.mines.iter_mut().enumerate() {
            m.life -= dt;
            m.arm = (m.arm - dt).max(0.0);
            if m.life <= 0.0 {
                boom.push(i);
                continue;
            }
            if m.armed() && self.tanks.iter().any(|t| t.alive && t.team != m.team && (t.x - m.x).hypot(t.y - m.y) < MINE_TRIGGER) {
                boom.push(i);
            }
        }
        let mut owner_of: Vec<(f32, f32, u8, u32, bool)> = Vec::new();
        for &i in boom.iter().rev() {
            let m = c.mines.remove(i);
            owner_of.push((m.x, m.y, m.team, m.owner, m.life > 0.0));
        }
        for (x, y, team, owner, live) in owner_of {
            if !live {
                continue; // timed out quietly
            }
            self.events.push(Event { kind: EventKind::Explode, x, y, a: 0.0 });
            for t in self.tanks.iter_mut() {
                if t.alive && t.team != team && (t.x - x).hypot(t.y - y) < MINE_BLAST && t.shield_t <= 0.0 {
                    t.hp -= MINE_DAMAGE;
                    t.last_hit = Some(owner);
                    if t.hp <= 0 {
                        t.hp = 0;
                        t.alive = false;
                        self.events.push(Event { kind: EventKind::Explode, x: t.x, y: t.y, a: 0.0 });
                    }
                }
            }
        }
        self.ctf = Some(c);
    }

    // ---- bots ----------------------------------------------------------

    fn line_clear(&self, a: (f32, f32), b: (f32, f32)) -> bool {
        let d = (b.0 - a.0).hypot(b.1 - a.1);
        let n = (d / 8.0).ceil().max(1.0) as i32;
        for i in 1..n {
            let f = i as f32 / n as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            if self.obstacles.iter().any(|o| o.solid_bullet() && x > o.x && x < o.x + o.w && y > o.y && y < o.y + o.h) {
                return false;
            }
        }
        true
    }

    /// Objective-driven bots: they path-find to flags, escort and return,
    /// shoot what they can see and use their items.
    pub(crate) fn ctf_ai(&mut self, dt: f32) {
        let Some(mut c) = self.ctf.take() else { return };
        let snapshot: Vec<(u32, u8, f32, f32, bool)> = self.tanks.iter().map(|t| (t.id, t.team, t.x, t.y, t.alive)).collect();
        let ids: Vec<(usize, u32)> = self.tanks.iter().enumerate().filter(|(_, t)| t.ai.is_some()).map(|(i, t)| (i, t.id)).collect();
        // bots of each team in order, so every second one defends
        let is_ai: Vec<u32> = ids.iter().map(|&(_, id)| id).collect();
        let rank = |id: u32, team: u8| snapshot.iter().filter(|s| s.1 == team && s.0 < id && is_ai.contains(&s.0)).count();
        for (i, id) in ids {
            let (team, alive) = (self.tanks[i].team, self.tanks[i].alive);
            if !c.bots.iter().any(|b| b.id == id) {
                c.bots.push(Bot { id, repath: 0.0, ..Default::default() });
            }
            let bi = c.bots.iter().position(|b| b.id == id).unwrap();
            if !alive {
                self.tanks[i].ctrl = Control { drive: true, ..Default::default() };
                c.bots[bi].path.clear();
                continue;
            }
            let (mx, my) = (self.tanks[i].x, self.tanks[i].y);
            let enemy = 1 - team as usize;
            let own = team as usize;
            let team_size = snapshot.iter().filter(|s| s.1 == team).count();
            let defender = team_size >= 3 && rank(id, team) % 2 == 1;
            let carrying = c.flags[enemy].carrier == Some(id);

            // ---- choose a goal
            let home = c.flags[own].home;
            let mut goal = (c.flags[enemy].x, c.flags[enemy].y);
            if carrying {
                goal = home;
            } else if let Some(cid) = c.flags[own].carrier {
                // my flag was stolen: run the thief down
                if let Some(s) = snapshot.iter().find(|s| s.0 == cid) {
                    goal = (s.2, s.3);
                }
            } else if !c.flags[own].at_home() {
                goal = (c.flags[own].x, c.flags[own].y);
            } else if let Some(cid) = c.flags[enemy].carrier {
                // a teammate has the flag: stay close and clear the way
                if let Some(s) = snapshot.iter().find(|s| s.0 == cid) {
                    goal = (s.2, s.3);
                }
            } else if defender {
                let dir = if own == 0 { 1.0 } else { -1.0 };
                goal = (home.0 + dir * 110.0, ARENA_H / 2.0 + ((self.time * 0.5 + id as f32).sin()) * 150.0);
            }
            // grab a nearby mystery box when idle
            if !carrying && self.tanks[i].item == ITEM_NONE {
                if let Some(p) = self
                    .pickups
                    .iter()
                    .filter(|p| p.kind == PickupKind::Mystery && (p.x - mx).hypot(p.y - my) < 240.0)
                    .min_by(|a, b| (a.x - mx).hypot(a.y - my).partial_cmp(&(b.x - mx).hypot(b.y - my)).unwrap())
                {
                    goal = (p.x, p.y);
                }
            }

            // ---- path to the goal
            let bot = &mut c.bots[bi];
            bot.repath -= dt;
            if bot.repath <= 0.0 {
                bot.repath = 0.45;
                bot.path = c.nav.path((mx, my), goal);
            }
            let bot = &mut c.bots[bi];
            while bot.path.len() > 1 && (bot.path[0].0 - mx).hypot(bot.path[0].1 - my) < 26.0 {
                bot.path.remove(0);
            }
            // aim at a waypoint a little ahead so turns stay smooth
            let wp = *bot.path.get(1).or(bot.path.first()).unwrap_or(&goal);
            let wp = if bot.path.len() > 1 && self.line_clear((mx, my), wp) { wp } else { *bot.path.first().unwrap_or(&goal) };
            let heading = (wp.1 - my).atan2(wp.0 - mx);
            let diff = wrap_angle(heading - self.tanks[i].angle);
            let mut steer = (diff * 3.0).clamp(-1.0, 1.0);
            let mut throttle = if diff.abs() < 1.3 { diff.cos().max(0.2) } else { 0.0 };
            if (goal.0 - mx).hypot(goal.1 - my) < 14.0 && !carrying {
                throttle = 0.0; // hold position at a defensive spot
            }

            // ---- stuck handling
            let bot = &mut c.bots[bi];
            if (bot.last_pos.0 - mx).hypot(bot.last_pos.1 - my) < 3.0 && throttle > 0.0 {
                bot.stuck += dt;
            } else {
                bot.stuck = 0.0;
                bot.last_pos = (mx, my);
            }
            if bot.stuck > 1.2 {
                bot.unstick = 0.7;
                bot.stuck = 0.0;
                bot.repath = 0.0;
                bot.last_pos = (mx, my);
            }
            if bot.unstick > 0.0 {
                bot.unstick -= dt;
                throttle = -1.0;
                steer = if id % 2 == 0 { 1.0 } else { -1.0 };
            }

            // ---- combat
            let mut aim = self.tanks[i].turret;
            let mut fire = false;
            let target = snapshot
                .iter()
                .filter(|s| s.4 && s.1 != team && (s.2 - mx).hypot(s.3 - my) < 380.0 && self.line_clear((mx, my), (s.2, s.3)))
                .min_by(|a, b| (a.2 - mx).hypot(a.3 - my).partial_cmp(&(b.2 - mx).hypot(b.3 - my)).unwrap())
                .copied();
            let bot = &mut c.bots[bi];
            if let Some((_, _, tx, ty, _)) = target {
                bot.seen_for += dt;
                let jitter = self.rng.range(-0.2, 0.2);
                aim = (ty - my).atan2(tx - mx) + jitter;
                let aligned = wrap_angle(aim - self.tanks[i].turret).abs() < 0.14;
                fire = bot.seen_for > 0.55 && aligned;
            } else {
                bot.seen_for = 0.0;
                aim = heading;
            }

            // ---- items
            let t = &self.tanks[i];
            let near_enemy = snapshot.iter().any(|s| s.4 && s.1 != team && (s.2 - mx).hypot(s.3 - my) < 170.0);
            let use_item = match t.item {
                ITEM_SHIELD => near_enemy || (carrying && target.is_some()),
                ITEM_BOMB => near_enemy || (defender && (home.0 - mx).hypot(home.1 - my) < 200.0),
                ITEM_SPREAD => target.is_some(),
                _ => false,
            };
            let boost = carrying && t.stamina > 0.15;
            self.tanks[i].ctrl = Control { drive: true, steer, throttle, fire, boost, aim, use_item, ..Default::default() };
        }
        self.ctf = Some(c);
    }
}

// ---- tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: u32, team: u8, ai: bool) -> PlayerSpec {
        PlayerSpec { id, name: format!("P{id}"), team, ai, kind: 0 }
    }

    fn duel() -> World {
        World::new_ctf(&[spec(0, 0, false), spec(1, 1, false)], 5)
    }

    fn idle_ctrl() -> Control {
        Control { drive: true, ..Default::default() }
    }

    #[test]
    fn flag_homes_are_clear_and_on_each_side() {
        let w = duel();
        let c = w.ctf.as_ref().unwrap();
        for f in &c.flags {
            assert!(w.obstacles.iter().all(|o| circle_rect(f.home.0, f.home.1, 40.0, o).is_none()), "flag base must be open ground");
        }
        assert!(c.flags[0].home.0 < ARENA_W / 2.0 && c.flags[1].home.0 > ARENA_W / 2.0);
        assert!(c.flags.iter().all(|f| f.at_home()));
    }

    #[test]
    fn stealing_and_capturing_scores_a_point() {
        let mut w = duel();
        let (h0, h1) = {
            let c = w.ctf.as_ref().unwrap();
            (c.flags[0].home, c.flags[1].home)
        };
        // blue (team 0) tank walks onto red's flag
        {
            let t = w.tank_mut(0).unwrap();
            t.x = h1.0;
            t.y = h1.1;
        }
        w.tank_mut(1).unwrap().x = 700.0; // red out of the way
        w.tank_mut(1).unwrap().y = 60.0;
        w.step(0.02);
        assert_eq!(w.ctf.as_ref().unwrap().flags[1].carrier, Some(0), "flag picked up");
        // the flag follows the carrier
        w.tank_mut(0).unwrap().x = 500.0;
        w.tank_mut(0).unwrap().y = 400.0;
        w.step(0.02);
        assert!((w.ctf.as_ref().unwrap().flags[1].x - 500.0).abs() < 30.0);
        // bring it home
        w.tank_mut(0).unwrap().x = h0.0 + 10.0;
        w.tank_mut(0).unwrap().y = h0.1;
        w.step(0.02);
        let c = w.ctf.as_ref().unwrap();
        assert_eq!(c.scores, [1, 0]);
        assert!(c.flags[1].at_home(), "stolen flag resets");
        assert_eq!(c.stat(0).caps, 1);
    }

    #[test]
    fn cannot_capture_while_own_flag_is_taken() {
        let mut w = duel();
        {
            let c = w.ctf.as_mut().unwrap();
            c.flags[1].carrier = Some(0);
            c.flags[0].carrier = Some(1); // red holds blue's flag
        }
        let h0 = w.ctf.as_ref().unwrap().flags[0].home;
        w.tank_mut(1).unwrap().x = 900.0;
        w.tank_mut(1).unwrap().y = 100.0;
        let t = w.tank_mut(0).unwrap();
        t.x = h0.0 + 5.0;
        t.y = h0.1;
        w.step(0.02);
        assert_eq!(w.ctf.as_ref().unwrap().scores, [0, 0]);
    }

    #[test]
    fn carrier_dies_drops_flag_and_teammate_returns_it() {
        let mut w = World::new_ctf(&[spec(0, 0, false), spec(2, 0, false), spec(1, 1, false)], 3);
        w.ctf.as_mut().unwrap().flags[1].carrier = Some(0);
        {
            let t = w.tank_mut(0).unwrap();
            t.x = 400.0;
            t.y = 380.0;
            t.hp = 1;
            t.last_hit = Some(1);
            t.alive = false;
        }
        w.tank_mut(2).unwrap().x = 100.0;
        w.step(0.02);
        let c = w.ctf.as_ref().unwrap();
        assert_eq!(c.flags[1].carrier, None, "dropped on death");
        assert_eq!(c.stat(0).deaths, 1);
        assert_eq!(c.stat(1).kills, 1, "killer is credited");
        assert!(w.tank(0).unwrap().respawn_t > 3.0);
    }

    #[test]
    fn dropped_flag_returns_on_touch_or_timeout() {
        let mut w = duel();
        {
            let c = w.ctf.as_mut().unwrap();
            c.flags[0].x = 600.0;
            c.flags[0].y = 380.0;
        }
        // an enemy touching it picks it up again; the owner touching it returns it
        w.tank_mut(0).unwrap().x = 600.0;
        w.tank_mut(0).unwrap().y = 380.0;
        w.tank_mut(1).unwrap().x = 1300.0;
        w.step(0.02);
        assert!(w.ctf.as_ref().unwrap().flags[0].at_home(), "owner touch sends it home");
        // timeout
        {
            let c = w.ctf.as_mut().unwrap();
            c.flags[0].x = 700.0;
            c.flags[0].y = 100.0;
            c.flags[0].away = FLAG_RETURN_SECS - 0.01;
        }
        w.tank_mut(0).unwrap().x = 200.0;
        w.step(0.05);
        assert!(w.ctf.as_ref().unwrap().flags[0].at_home(), "auto return after the timeout");
    }

    #[test]
    fn destroyed_tanks_respawn_at_their_base() {
        let mut w = duel();
        {
            let t = w.tank_mut(0).unwrap();
            t.x = 700.0;
            t.alive = false;
            t.hp = 0;
        }
        for _ in 0..40 {
            w.step(0.05);
        }
        assert!(!w.tank(0).unwrap().alive, "still waiting");
        for _ in 0..60 {
            w.step(0.05);
        }
        let t = w.tank(0).unwrap();
        assert!(t.alive && t.hp == t.max_hp());
        assert!((t.x - t.spawn.0).abs() < 5.0);
        assert!(t.shield_t > 0.0 || w.time > 5.0);
    }

    #[test]
    fn departed_players_do_not_respawn() {
        let mut w = duel();
        w.kill_player(1);
        for _ in 0..140 {
            w.step(0.05);
        }
        assert!(!w.tank(1).unwrap().alive);
    }

    #[test]
    fn mystery_box_gives_one_item_and_items_work() {
        let mut w = duel();
        w.pickups.push(Pickup { id: 90, x: 720.0, y: 250.0, kind: PickupKind::Mystery });
        let t = w.tank_mut(0).unwrap();
        t.x = 720.0;
        t.y = 250.0;
        w.tank_mut(1).unwrap().x = 1300.0;
        w.step(0.02);
        let item = w.tank(0).unwrap().item;
        assert!((1..=3).contains(&item), "got an item");
        // a second box is left alone while holding one
        w.pickups.push(Pickup { id: 91, x: 720.0, y: 250.0, kind: PickupKind::Mystery });
        w.step(0.02);
        assert_eq!(w.tank(0).unwrap().item, item);
        assert_eq!(w.pickups.iter().filter(|p| p.kind == PickupKind::Mystery).count(), 1);
        // use it (the leftover box is cleared so the tank cannot pick a new item up at once)
        w.pickups.retain(|p| p.kind != PickupKind::Mystery);
        w.tank_mut(0).unwrap().ctrl = Control { use_item: true, ..idle_ctrl() };
        w.step(0.02);
        let t = w.tank(0).unwrap();
        assert_eq!(t.item, 0, "consumed");
        match item {
            ITEM_BOMB => assert_eq!(w.ctf.as_ref().unwrap().mines.len(), 1),
            ITEM_SHIELD => assert!(t.shield_t > 4.0),
            _ => assert!(t.spread_t > 5.0),
        }
    }

    #[test]
    fn bomb_hits_enemies_but_not_shielded_or_friends() {
        let mut w = World::new_ctf(&[spec(0, 0, false), spec(2, 0, false), spec(1, 1, false)], 3);
        w.tank_mut(0).unwrap().x = 100.0;
        w.tank_mut(2).unwrap().x = 100.0;
        w.tank_mut(2).unwrap().y = 720.0;
        {
            let t = w.tank_mut(1).unwrap();
            t.x = 720.0;
            t.y = 250.0;
            t.ctrl = idle_ctrl();
        }
        w.ctf.as_mut().unwrap().mines.push(Mine { id: 1, x: 710.0, y: 250.0, team: 0, owner: 0, arm: 0.0, life: 20.0 });
        w.tank_mut(1).unwrap().shield_t = 3.0;
        w.step(0.02);
        assert_eq!(w.tank(1).unwrap().hp, w.tank(1).unwrap().max_hp(), "shield blocks the blast");
        assert!(w.ctf.as_ref().unwrap().mines.is_empty(), "mine is spent");
        w.ctf.as_mut().unwrap().mines.push(Mine { id: 2, x: 720.0, y: 250.0, team: 0, owner: 0, arm: 0.0, life: 20.0 });
        w.tank_mut(1).unwrap().shield_t = 0.0;
        w.step(0.02);
        assert!(w.tank(1).unwrap().hp < w.tank(1).unwrap().max_hp(), "enemy is hurt");
        // a friendly tank on a friendly mine is fine
        w.ctf.as_mut().unwrap().mines.push(Mine { id: 3, x: 100.0, y: 720.0, team: 0, owner: 0, arm: 0.0, life: 20.0 });
        w.step(0.02);
        assert_eq!(w.tank(2).unwrap().hp, w.tank(2).unwrap().max_hp());
        assert_eq!(w.ctf.as_ref().unwrap().mines.len(), 1, "friendly mine stays armed");
    }

    #[test]
    fn spread_shot_fires_three_bullets() {
        let mut w = duel();
        w.tank_mut(0).unwrap().spread_t = 5.0;
        w.tank_mut(0).unwrap().ctrl = Control { fire: true, aim: 0.0, ..idle_ctrl() };
        w.step(0.02);
        assert_eq!(w.bullets.len(), 3);
    }

    #[test]
    fn clock_ends_the_match_and_winner_is_the_higher_score() {
        let mut w = duel();
        {
            let c = w.ctf.as_mut().unwrap();
            c.scores = [0, 2];
            c.time_left = 0.03;
        }
        w.step(0.05);
        assert_eq!(w.phase, Phase::GameOver);
        assert_eq!(w.winner, Some(1));
        // a tie with no kills is a draw
        let mut w = duel();
        w.ctf.as_mut().unwrap().time_left = 0.01;
        w.step(0.05);
        assert_eq!(w.winner, None);
    }

    #[test]
    fn scores_stay_hidden_until_the_end() {
        let mut w = duel();
        w.ctf.as_mut().unwrap().scores = [3, 1];
        let mid = w.encode_state(&[]);
        let mut client = duel();
        client.apply_state(&mid).unwrap();
        assert_eq!(client.ctf.as_ref().unwrap().scores, [0, 0], "no score leaks mid-match");
        w.phase = Phase::GameOver;
        w.winner = Some(0);
        let end = w.encode_state(&[]);
        client.apply_state(&end).unwrap();
        assert_eq!(client.ctf.as_ref().unwrap().scores, [3, 1], "revealed at the end");
    }

    #[test]
    fn ctf_snapshot_roundtrip() {
        let mut w = duel();
        {
            let c = w.ctf.as_mut().unwrap();
            c.flags[1].carrier = Some(0);
            c.flags[1].x = 300.0;
            c.flags[1].y = 222.0;
            c.mines.push(Mine { id: 4, x: 10.0, y: 20.0, team: 1, owner: 1, arm: 0.0, life: 9.0 });
            c.time_left = 123.4;
        }
        w.tank_mut(0).unwrap().item = ITEM_SPREAD;
        w.tank_mut(1).unwrap().respawn_t = 2.5;
        let s = w.encode_state(&[]);
        let mut cl = duel();
        assert!(cl.apply_state(&s).is_some());
        let c = cl.ctf.as_ref().unwrap();
        assert_eq!(c.flags[1].carrier, Some(0));
        assert!((c.flags[1].y - 222.0).abs() < 0.2);
        assert_eq!(c.mines.len(), 1);
        assert!((c.time_left - 123.4).abs() < 0.2);
        assert_eq!(cl.tank(0).unwrap().item, ITEM_SPREAD);
        assert!((cl.tank(1).unwrap().respawn_t - 2.5).abs() < 0.2);
    }

    #[test]
    fn nav_finds_a_way_around_obstacles() {
        let w = duel();
        let c = w.ctf.as_ref().unwrap();
        let path = c.nav.path((90.0, 400.0), (1350.0, 400.0));
        assert!(path.len() > 10, "a real route, not a straight shot");
        assert!(path.iter().all(|p| w.obstacles.iter().all(|o| !o.solid_tank() || circle_rect(p.0, p.1, TANK_R - 2.0, o).is_none())));
    }

    #[test]
    fn bots_play_full_matches_and_sometimes_score() {
        let specs: Vec<PlayerSpec> = (0..4).map(|i| spec(BOT_ID_BASE + i, (i % 2) as u8, true)).collect();
        let (mut caps, mut kills) = (0, 0);
        for seed in 20..28u64 {
            let mut w = World::new_ctf(&specs, seed);
            let mut steps = 0;
            while w.phase != Phase::GameOver {
                w.step(1.0 / 30.0);
                steps += 1;
                assert!(w.tanks.iter().all(|t| t.x.is_finite() && t.y.is_finite()));
                assert!(steps < 30 * 300, "the clock must end the match");
            }
            let c = w.ctf.as_ref().unwrap();
            caps += c.scores.iter().sum::<u32>();
            kills += c.stats.iter().map(|s| s.kills).sum::<u32>();
        }
        assert!(kills > 20, "bots fight (kills {kills})");
        assert!(caps > 0, "bots manage to capture flags (caps {caps})");
    }

    #[test]
    fn every_arena_is_playable_for_ctf() {
        for arena in 0..ARENAS {
            let specs: Vec<PlayerSpec> = (0..8).map(|i| spec(i, (i % 2) as u8, false)).collect();
            let w = World::build(&specs, 4, true, arena);
            let name = arena_name(arena);
            // spawn columns and flag bases are open ground
            for t in &w.tanks {
                assert!(w.obstacles.iter().all(|o| !o.solid_tank() || circle_rect(t.spawn.0, t.spawn.1, TANK_R + 4.0, o).is_none()), "{name}: spawn of tank {} is clear", t.id);
            }
            let c = w.ctf.as_ref().unwrap();
            for f in &c.flags {
                assert!(w.obstacles.iter().all(|o| !o.solid_tank() || circle_rect(f.home.0, f.home.1, 40.0, o).is_none()), "{name}: flag base clear");
            }
            // gates sit on housings with a clear muzzle
            for g in &w.gates {
                let (mx, my) = (g.x + g.angle.cos() * 34.0, g.y + g.angle.sin() * 34.0);
                assert!(w.obstacles.iter().all(|o| !(mx > o.x && mx < o.x + o.w && my > o.y && my < o.y + o.h && o.solid_bullet())), "{name}: muzzle clear");
            }
            // solid obstacles never overlap one another
            let solid: Vec<_> = w.obstacles.iter().filter(|o| o.solid_tank()).collect();
            for (i, a) in solid.iter().enumerate() {
                for b in solid.iter().skip(i + 1) {
                    assert!(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y, "{name}: {a:?} overlaps {b:?}");
                }
            }
            // each base can reach the other flag, and bots can path there
            let (a, b) = (c.flags[0].home, c.flags[1].home);
            let path = c.nav.path((a.0 + 40.0, a.1), b);
            assert!(path.len() > 20, "{name}: a real route to the enemy flag ({} steps)", path.len());
            // left-right mirror
            for o in &w.obstacles {
                let m = (ARENA_W - o.x - o.w, o.y);
                assert!(w.obstacles.iter().any(|q| q.kind == o.kind && (q.x - m.0).abs() < 0.5 && (q.y - m.1).abs() < 0.5 && (q.w - o.w).abs() < 0.5), "{name}: mirror of {o:?}");
            }
        }
    }
}

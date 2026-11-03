//! LAN multiplayer over UDP. The host is authoritative: it simulates the
//! match and broadcasts snapshots; clients send a single "held" boolean.
//!
//! Wire protocol (UTF-8 text, fields split on ':'):
//!   client -> host   JOIN:<name>   INPUT:<control csv>   LEAVE   DISCOVER
//!                    (control csv = sim::Control::encode; legacy INPUT:0|1 still works)
//!   host -> client   WELCOME:<id>  REJECT:<reason>
//!                    HELLO:<host name>:<players>:<capacity>:<match type>:<in game 0|1>
//!                    LOBBY:<match type>:<id,name,team;...>
//!                    STATE:...  (see sim::World::encode_state)
//!
//! Only std::net is used, so it behaves the same on Linux, Windows, Android.

use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::sim::{Control, Event, PlayerSpec, World};

pub const PORT: u16 = 22122;
pub const TIMEOUT: Duration = Duration::from_secs(6);
const STATE_INTERVAL: Duration = Duration::from_millis(50);
const LOBBY_INTERVAL: Duration = Duration::from_millis(250);
const INPUT_INTERVAL: Duration = Duration::from_millis(50);

/// Keep names safe for the wire format and for display.
pub fn sanitize_name(s: &str) -> String {
    let cleaned: String =
        s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '\'')).take(12).collect();
    let t = cleaned.trim().to_string();
    if t.is_empty() {
        "Player".to_string()
    } else {
        t
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RosterEntry {
    pub id: u32,
    pub name: String,
    pub team: u8,
}

pub fn roster_to_specs(r: &[RosterEntry]) -> Vec<PlayerSpec> {
    r.iter().map(|e| PlayerSpec { id: e.id, name: e.name.clone(), team: e.team, ai: false }).collect()
}

/// Players per team for a match type: 1v1, 2v2, 4v4.
pub fn capacity(match_type: u8) -> usize {
    match_type as usize * 2
}

fn encode_roster(r: &[RosterEntry]) -> String {
    r.iter().map(|e| format!("{},{},{}", e.id, e.name, e.team)).collect::<Vec<_>>().join(";")
}

fn decode_roster(s: &str) -> Vec<RosterEntry> {
    s.split(';')
        .filter_map(|rec| {
            let f: Vec<&str> = rec.split(',').collect();
            if f.len() != 3 {
                return None;
            }
            Some(RosterEntry { id: f[0].parse().ok()?, name: f[1].to_string(), team: f[2].parse().ok()? })
        })
        .collect()
}

// ------------------------------------------------------------------ host ----

struct ClientInfo {
    id: u32,
    last_seen: Instant,
    ctrl: Control,
}

pub struct Host {
    sock: UdpSocket,
    clients: HashMap<SocketAddr, ClientInfo>,
    pub roster: Vec<RosterEntry>,
    pub match_type: u8,
    pub in_game: bool,
    host_name: String,
    next_id: u32,
    last_state: Instant,
    last_lobby: Instant,
}

impl Host {
    pub fn new(port: u16, name: &str, match_type: u8) -> io::Result<Host> {
        let sock = UdpSocket::bind(("0.0.0.0", port))?;
        sock.set_nonblocking(true)?;
        let name = sanitize_name(name);
        Ok(Host {
            sock,
            clients: HashMap::new(),
            roster: vec![RosterEntry { id: 0, name: name.clone(), team: 0 }],
            match_type,
            in_game: false,
            host_name: name,
            next_id: 1,
            last_state: Instant::now(),
            last_lobby: Instant::now(),
        })
    }

    pub fn port(&self) -> u16 {
        self.sock.local_addr().map(|a| a.port()).unwrap_or(PORT)
    }

    fn team_count(&self, team: u8) -> usize {
        self.roster.iter().filter(|e| e.team == team).count()
    }

    /// Cycle 1v1 -> 2v2 -> 4v4, skipping sizes that cannot hold the roster.
    pub fn cycle_match_type(&mut self) {
        let order = [1u8, 2, 4];
        let start = order.iter().position(|&m| m == self.match_type).unwrap_or(0);
        for step in 1..=order.len() {
            let m = order[(start + step) % order.len()];
            if self.team_count(0) <= m as usize && self.team_count(1) <= m as usize {
                self.match_type = m;
                return;
            }
        }
    }

    /// Latest input received from a client (neutral if unknown).
    pub fn input(&self, id: u32) -> Control {
        self.clients.values().find(|c| c.id == id).map(|c| c.ctrl).unwrap_or_default()
    }

    /// Process incoming packets. Returns ids of clients that left or timed out.
    pub fn poll(&mut self) -> Vec<u32> {
        let mut gone = Vec::new();
        let mut buf = [0u8; 2048];
        loop {
            match self.sock.recv_from(&mut buf) {
                Ok((n, addr)) => {
                    let msg = String::from_utf8_lossy(&buf[..n]).to_string();
                    self.handle(&msg, addr, &mut gone);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break, // e.g. ICMP "connection reset" on Windows
            }
        }
        let now = Instant::now();
        let stale: Vec<SocketAddr> =
            self.clients.iter().filter(|(_, c)| now.duration_since(c.last_seen) > TIMEOUT).map(|(a, _)| *a).collect();
        for a in stale {
            if let Some(c) = self.clients.remove(&a) {
                self.roster.retain(|e| e.id != c.id);
                gone.push(c.id);
            }
        }
        gone
    }

    fn send(&self, msg: &str, to: SocketAddr) {
        let _ = self.sock.send_to(msg.as_bytes(), to);
    }

    fn handle(&mut self, msg: &str, addr: SocketAddr, gone: &mut Vec<u32>) {
        let mut parts = msg.splitn(2, ':');
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("");
        match cmd {
            "DISCOVER" => {
                let hello = format!(
                    "HELLO:{}:{}:{}:{}:{}",
                    self.host_name,
                    self.roster.len(),
                    capacity(self.match_type),
                    self.match_type,
                    self.in_game as u8
                );
                self.send(&hello, addr);
            }
            "JOIN" => {
                if let Some(c) = self.clients.get_mut(&addr) {
                    c.last_seen = Instant::now();
                    let id = c.id;
                    self.send(&format!("WELCOME:{id}"), addr);
                    return;
                }
                if self.in_game {
                    self.send("REJECT:Game already running", addr);
                    return;
                }
                if self.roster.len() >= capacity(self.match_type) {
                    self.send("REJECT:Lobby is full", addr);
                    return;
                }
                // Auto-balance: join the smaller team (ties go to team 0).
                let team = if self.team_count(0) <= self.team_count(1) { 0 } else { 1 };
                let id = self.next_id;
                self.next_id += 1;
                self.roster.push(RosterEntry { id, name: sanitize_name(arg), team });
                self.clients.insert(addr, ClientInfo { id, last_seen: Instant::now(), ctrl: Control::default() });
                self.send(&format!("WELCOME:{id}"), addr);
                self.last_lobby = Instant::now() - LOBBY_INTERVAL; // push roster right away
            }
            "INPUT" => {
                if let Some(c) = self.clients.get_mut(&addr) {
                    c.last_seen = Instant::now();
                    if let Some(ctrl) = Control::decode(arg) {
                        c.ctrl = ctrl;
                    }
                }
            }
            "LEAVE" => {
                if let Some(c) = self.clients.remove(&addr) {
                    self.roster.retain(|e| e.id != c.id);
                    gone.push(c.id);
                }
            }
            _ => {}
        }
    }

    /// Rebalance so team sizes differ by at most one (used before starting).
    pub fn rebalance(&mut self) {
        loop {
            let (a, b) = (self.team_count(0), self.team_count(1));
            if a > b + 1 {
                if let Some(e) = self.roster.iter_mut().rev().find(|e| e.team == 0 && e.id != 0) {
                    e.team = 1;
                    continue;
                }
            } else if b > a + 1 {
                if let Some(e) = self.roster.iter_mut().rev().find(|e| e.team == 1) {
                    e.team = 0;
                    continue;
                }
            }
            break;
        }
    }

    pub fn can_start(&self) -> bool {
        self.roster.len() >= 2 && self.team_count(0) >= 1 && self.team_count(1) >= 1
    }

    pub fn send_lobby(&mut self) {
        if self.last_lobby.elapsed() < LOBBY_INTERVAL {
            return;
        }
        self.last_lobby = Instant::now();
        let msg = format!("LOBBY:{}:{}", self.match_type, encode_roster(&self.roster));
        for a in self.clients.keys() {
            self.send(&msg, *a);
        }
    }

    /// Broadcast a snapshot at ~20 Hz. `force` bypasses the rate limit (used
    /// for the final game-over packet). Returns true when it was sent.
    pub fn send_state(&mut self, world: &World, events: &[Event], force: bool) -> bool {
        if !force && self.last_state.elapsed() < STATE_INTERVAL {
            return false;
        }
        self.last_state = Instant::now();
        let msg = world.encode_state(events);
        for a in self.clients.keys() {
            self.send(&msg, *a);
        }
        true
    }

    /// Back to the lobby on the *same* sockets and roster (rematch).
    pub fn return_to_lobby(&mut self) {
        self.in_game = false;
        for c in self.clients.values_mut() {
            c.ctrl = Control::default();
            c.last_seen = Instant::now();
        }
        self.last_lobby = Instant::now() - LOBBY_INTERVAL;
        self.send_lobby();
    }

    pub fn shutdown(&mut self) {
        for a in self.clients.keys() {
            self.send("REJECT:Host closed the game", *a);
        }
        self.clients.clear();
    }
}

// ---------------------------------------------------------------- client ----

#[derive(Clone, Debug)]
pub struct HostInfo {
    pub addr: SocketAddr,
    pub name: String,
    pub players: usize,
    pub capacity: usize,
    pub in_game: bool,
}

#[derive(Debug)]
pub enum ClientMsg {
    Hello(HostInfo),
    Welcome(u32),
    Reject(String),
    Lobby { match_type: u8, roster: Vec<RosterEntry> },
    State(String),
}

pub struct Client {
    sock: UdpSocket,
    port: u16,
    pub host: Option<SocketAddr>,
    pub my_id: Option<u32>,
    name: String,
    last_recv: Instant,
    last_input: Instant,
    last_ctrl: Control,
    last_join: Instant,
    last_discover: Instant,
}

impl Client {
    pub fn new(port: u16, name: &str) -> io::Result<Client> {
        let sock = UdpSocket::bind(("0.0.0.0", 0))?;
        sock.set_nonblocking(true)?;
        let _ = sock.set_broadcast(true);
        let ago = Instant::now() - Duration::from_secs(10);
        Ok(Client {
            sock,
            port,
            host: None,
            my_id: None,
            name: sanitize_name(name),
            last_recv: Instant::now(),
            last_input: ago,
            last_ctrl: Control::default(),
            last_join: ago,
            last_discover: ago,
        })
    }

    /// Ping the LAN every second. The broadcast reaches a host on the same
    /// WiFi or phone hotspot; loopback is also tried for same-machine play.
    pub fn discover(&mut self) {
        if self.last_discover.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_discover = Instant::now();
        let _ = self.sock.send_to(b"DISCOVER", ("255.255.255.255", self.port));
        let _ = self.sock.send_to(b"DISCOVER", ("127.0.0.1", self.port));
    }

    pub fn connect(&mut self, addr: SocketAddr) {
        self.host = Some(addr);
        self.my_id = None;
        self.last_recv = Instant::now();
        self.last_join = Instant::now() - Duration::from_secs(10);
    }

    /// Re-sends JOIN until WELCOME arrives; sends input; reads packets.
    pub fn poll(&mut self, ctrl: Control) -> Vec<ClientMsg> {
        if let Some(host) = self.host {
            if self.my_id.is_none() && self.last_join.elapsed() > Duration::from_millis(500) {
                self.last_join = Instant::now();
                let _ = self.sock.send_to(format!("JOIN:{}", self.name).as_bytes(), host);
            }
            // Send changes promptly (but at most ~60/s) and a keep-alive at 20 Hz.
            let since = self.last_input.elapsed();
            let changed = ctrl != self.last_ctrl && since > Duration::from_millis(15);
            if self.my_id.is_some() && (changed || since > INPUT_INTERVAL) {
                self.last_input = Instant::now();
                self.last_ctrl = ctrl;
                let _ = self.sock.send_to(format!("INPUT:{}", ctrl.encode()).as_bytes(), host);
            }
        }
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match self.sock.recv_from(&mut buf) {
                Ok((n, from)) => {
                    let msg = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Some(m) = self.parse(&msg, from) {
                        out.push(m);
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        out
    }

    fn parse(&mut self, msg: &str, from: SocketAddr) -> Option<ClientMsg> {
        let mut it = msg.splitn(2, ':');
        let cmd = it.next()?;
        let arg = it.next().unwrap_or("");
        if cmd == "HELLO" {
            let f: Vec<&str> = arg.split(':').collect();
            if f.len() < 5 {
                return None;
            }
            // Reply came from the host's real address (even for broadcasts).
            return Some(ClientMsg::Hello(HostInfo {
                addr: from,
                name: f[0].to_string(),
                players: f[1].parse().ok()?,
                capacity: f[2].parse().ok()?,
                in_game: f[4] == "1",
            }));
        }
        // Everything else must come from our host.
        if Some(from) != self.host {
            return None;
        }
        self.last_recv = Instant::now();
        match cmd {
            "WELCOME" => {
                let id = arg.parse().ok()?;
                self.my_id = Some(id);
                Some(ClientMsg::Welcome(id))
            }
            "REJECT" => Some(ClientMsg::Reject(arg.to_string())),
            "LOBBY" => {
                let mut p = arg.splitn(2, ':');
                let mt = p.next()?.parse().ok()?;
                Some(ClientMsg::Lobby { match_type: mt, roster: decode_roster(p.next().unwrap_or("")) })
            }
            "STATE" => Some(ClientMsg::State(msg.to_string())),
            _ => None,
        }
    }

    pub fn host_timed_out(&self) -> bool {
        self.host.is_some() && self.last_recv.elapsed() > TIMEOUT
    }

    pub fn leave(&mut self) {
        if let Some(h) = self.host {
            let _ = self.sock.send_to(b"LEAVE", h);
        }
        self.host = None;
        self.my_id = None;
    }
}

// ----------------------------------------------------------------- tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Phase, World};
    use std::thread::sleep;

    fn pump(host: &mut Host, clients: &mut [&mut Client], ms: u64) -> Vec<Vec<ClientMsg>> {
        let mut all: Vec<Vec<ClientMsg>> = clients.iter().map(|_| Vec::new()).collect();
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            host.poll();
            host.send_lobby();
            for (i, c) in clients.iter_mut().enumerate() {
                all[i].extend(c.poll(Control::default()));
            }
            sleep(Duration::from_millis(5));
        }
        all
    }

    fn join(port: u16, host: &mut Host, name: &str) -> Client {
        let mut c = Client::new(port, name).unwrap();
        c.connect(format!("127.0.0.1:{port}").parse().unwrap());
        for _ in 0..100 {
            pump(host, &mut [&mut c], 20);
            if c.my_id.is_some() {
                break;
            }
        }
        c
    }

    #[test]
    fn names_are_sanitized() {
        assert_eq!(sanitize_name("Al:ice;,1"), "Alice1");
        assert_eq!(sanitize_name("   "), "Player");
        assert_eq!(sanitize_name("ABCDEFGHIJKLMNOP").len(), 12);
    }

    #[test]
    fn discovery_finds_host() {
        let mut host = Host::new(23101, "Hosty", 2).unwrap();
        let mut c = Client::new(23101, "x").unwrap();
        let mut found = None;
        for _ in 0..100 {
            c.discover();
            host.poll();
            for m in c.poll(Control::default()) {
                if let ClientMsg::Hello(h) = m {
                    found = Some(h);
                }
            }
            if found.is_some() {
                break;
            }
            sleep(Duration::from_millis(10));
        }
        let h = found.expect("host discovered");
        assert_eq!(h.name, "Hosty");
        assert_eq!(h.capacity, 4);
        assert_eq!(h.port_ok(23101), true);
    }

    impl HostInfo {
        fn port_ok(&self, p: u16) -> bool {
            self.addr.port() == p
        }
    }

    #[test]
    fn join_balances_teams_and_respects_capacity() {
        let mut host = Host::new(23102, "H", 1).unwrap();
        let c1 = join(23102, &mut host, "Bob");
        assert_eq!(c1.my_id, Some(1));
        assert_eq!(host.roster.len(), 2);
        assert_eq!(host.roster[0].team, 0);
        assert_eq!(host.roster[1].team, 1, "second player goes to the other team");
        // 1v1 is full now
        let mut c2 = Client::new(23102, "Eve").unwrap();
        c2.connect("127.0.0.1:23102".parse().unwrap());
        let mut rejected = false;
        for _ in 0..50 {
            for m in pump(&mut host, &mut [&mut c2], 20).remove(0) {
                if let ClientMsg::Reject(r) = m {
                    assert!(r.contains("full"));
                    rejected = true;
                }
            }
            if rejected {
                break;
            }
        }
        assert!(rejected);
        assert_eq!(host.roster.len(), 2);
    }

    #[test]
    fn four_players_two_v_two() {
        let mut host = Host::new(23103, "H", 2).unwrap();
        let mut cs: Vec<Client> = (0..3).map(|i| join(23103, &mut host, &format!("P{i}"))).collect();
        assert_eq!(host.roster.len(), 4);
        let t0 = host.roster.iter().filter(|e| e.team == 0).count();
        assert_eq!(t0, 2);
        // clients learn the roster
        let mut refs: Vec<&mut Client> = cs.iter_mut().collect();
        let got = pump(&mut host, &mut refs, 400);
        let lobby = got[0].iter().rev().find_map(|m| if let ClientMsg::Lobby { roster, .. } = m { Some(roster.clone()) } else { None });
        assert_eq!(lobby.expect("lobby").len(), 4);
    }

    #[test]
    fn state_flows_input_flows_and_rematch_keeps_connection() {
        let mut host = Host::new(23104, "H", 1).unwrap();
        let mut c = join(23104, &mut host, "Bob");
        let id = c.my_id.unwrap();
        assert!(host.can_start());

        // start a match
        host.in_game = true;
        let mut world = World::new_match(&roster_to_specs(&host.roster), 1);

        // client holds input -> host sees it
        let mut saw_input = false;
        let mut got_state = false;
        for _ in 0..200 {
            host.poll();
            host.send_state(&world, &[], false);
            for m in c.poll(Control::classic(true)) {
                if let ClientMsg::State(s) = m {
                    let mut w2 = World::new_match(&roster_to_specs(&host.roster), 2);
                    assert!(w2.apply_state(&s).is_some());
                    got_state = true;
                }
            }
            if host.input(id).held {
                saw_input = true;
            }
            if saw_input && got_state {
                break;
            }
            sleep(Duration::from_millis(10));
        }
        assert!(saw_input, "host received the client's held input");
        assert!(got_state, "client received a snapshot");

        // game over, then rematch on the same sockets
        world.phase = Phase::GameOver;
        host.send_state(&world, &[], true);
        host.return_to_lobby();
        let mut saw_lobby = false;
        for _ in 0..100 {
            host.poll();
            for m in c.poll(Control::default()) {
                if matches!(m, ClientMsg::Lobby { .. }) {
                    saw_lobby = true;
                }
            }
            if saw_lobby {
                break;
            }
            sleep(Duration::from_millis(10));
        }
        assert!(saw_lobby, "client returned to the lobby, not disconnected");
        assert_eq!(host.roster.len(), 2, "client is still on the roster");
        // and a second match can start
        host.in_game = true;
        assert!(host.roster.len() >= 2);
    }

    #[test]
    fn leave_and_ingame_reject() {
        let mut host = Host::new(23105, "H", 2).unwrap();
        let mut c = join(23105, &mut host, "A");
        c.leave();
        for _ in 0..20 {
            host.poll();
            sleep(Duration::from_millis(5));
        }
        assert_eq!(host.roster.len(), 1, "leaver removed");
        host.in_game = true;
        let mut late = Client::new(23105, "Late").unwrap();
        late.connect("127.0.0.1:23105".parse().unwrap());
        let mut rejected = false;
        for _ in 0..50 {
            for m in pump(&mut host, &mut [&mut late], 20).remove(0) {
                if matches!(m, ClientMsg::Reject(_)) {
                    rejected = true;
                }
            }
            if rejected {
                break;
            }
        }
        assert!(rejected);
    }

    #[test]
    fn match_type_cycles_and_skips_too_small() {
        let mut host = Host::new(23106, "H", 4).unwrap();
        host.cycle_match_type();
        assert_eq!(host.match_type, 1);
        host.cycle_match_type();
        assert_eq!(host.match_type, 2);
        // 3 players on team 0 would not fit 1v1 or 2v2... simulate
        for i in 0..3 {
            host.roster.push(RosterEntry { id: 10 + i, name: "x".into(), team: 0 });
        }
        host.cycle_match_type();
        assert_eq!(host.match_type, 4, "skips 1 (too small)");
    }

    #[test]
    fn rebalance_evens_teams() {
        let mut host = Host::new(23107, "H", 4).unwrap();
        for i in 1..5 {
            host.roster.push(RosterEntry { id: i, name: "x".into(), team: 0 });
        }
        host.rebalance();
        let a = host.roster.iter().filter(|e| e.team == 0).count();
        let b = host.roster.iter().filter(|e| e.team == 1).count();
        assert!((a as i32 - b as i32).abs() <= 1, "{a} vs {b}");
    }

    #[test]
    fn drive_input_reaches_the_host_and_resets_on_rematch() {
        let mut host = Host::new(23108, "H", 1).unwrap();
        let mut c = join(23108, &mut host, "Bob");
        let id = c.my_id.unwrap();
        let want = Control { drive: true, steer: -1.0, throttle: 1.0, fire: true, boost: true, reload: false, held: false, aim: 1.57 };
        let mut got = None;
        for _ in 0..200 {
            host.poll();
            c.poll(want);
            let seen = host.input(id);
            if seen.drive {
                got = Some(seen);
                break;
            }
            sleep(Duration::from_millis(10));
        }
        let got = got.expect("host saw drive input");
        assert_eq!(got.steer, -1.0);
        assert_eq!(got.throttle, 1.0);
        assert!(got.fire && got.boost);
        assert!((got.aim - 1.57).abs() < 0.01);
        host.return_to_lobby();
        assert_eq!(host.input(id), Control::default(), "no stuck keys after a rematch");
    }

    #[test]
    fn garbage_and_legacy_input_do_not_crash_the_host() {
        let mut host = Host::new(23109, "H", 1).unwrap();
        let mut c = join(23109, &mut host, "Bob");
        let id = c.my_id.unwrap();
        let sock = UdpSocket::bind("0.0.0.0:0").unwrap();
        // a stranger's packets are ignored
        sock.send_to(b"INPUT:1,1,0,0,1,1,1,0", "127.0.0.1:23109").unwrap();
        sock.send_to(b"\xff\xfe junk", "127.0.0.1:23109").unwrap();
        for _ in 0..10 {
            host.poll();
            c.poll(Control::default());
            sleep(Duration::from_millis(5));
        }
        assert_eq!(host.input(id), Control::default());
        assert_eq!(host.roster.len(), 2);
    }
}

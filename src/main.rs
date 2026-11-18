mod audio;
mod ctf;
mod gfx;
mod net;
mod settings;
mod sim;
mod ui;

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};

use macroquad::prelude::*;

use audio::Audio;
use gfx::*;
use net::*;
use settings::Settings;
use sim::*;
use ui::*;

/// Window / taskbar icon (raw RGBA generated from assets/icon).
fn window_icon() -> macroquad::miniquad::conf::Icon {
    macroquad::miniquad::conf::Icon {
        small: include_bytes!("../assets/icon/16.rgba").to_owned(),
        medium: include_bytes!("../assets/icon/32.rgba").to_owned(),
        big: include_bytes!("../assets/icon/64.rgba").to_owned(),
    }
}

fn conf() -> Conf {
    Conf {
        window_title: "Tank Arena 3D".to_string(),
        window_width: 1280,
        window_height: 720,
        high_dpi: true,
        sample_count: 4,
        icon: Some(window_icon()),
        ..Default::default()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum KbTarget {
    Name,
    Ip,
}

enum Screen {
    Menu,
    Settings,
    HowTo,
    Garage,
    Keyboard { target: KbTarget, text: String },
    Discovering,
    HostLobby,
    ClientLobby,
    Play,
}

enum Mode {
    Offline,
    Host(Host),
    Client(Client),
}

struct App {
    screen: Screen,
    mode: Mode,
    settings: Settings,
    world: World,
    demo: World,
    my_id: u32,
    particles: Vec<Particle>,
    pending_events: Vec<Event>,
    cam: CamState,
    scene: Scene,
    tracks: TrackAnim,
    demo_tracks: TrackAnim,
    ui: Ui,
    audio: Audio,
    notice: Option<(String, f32)>,
    roster: Vec<RosterEntry>,
    lobby_match_type: u8,
    lobby_ctf: bool,
    lobby_arena: u8,
    /// keeps the item press alive long enough to survive a dropped packet
    item_hold: f32,
    status: String,
    play_time: f32,
    targets: HashMap<u32, (f32, f32, f32, f32)>,
    quit: bool,
    final_sent: bool,
    /// Camera / turret aim angle in the chase view.
    aim: f32,
    touch: TouchCtl,
    touch_mode: bool,
    paused: bool,
    help_page: usize,
    garage_sel: u8,
    grabbed: bool,
    fire_armed: bool,
    last_mouse: Vec2,
    muted: bool,
    fullscreen: bool,
    forced_drive: Vec<(u64, u64, f32, f32, bool, f32)>,
    test_pause: Option<u64>,
}

/// Touch-screen twin-stick state (virtual coordinates).
#[derive(Default)]
struct TouchCtl {
    stick: Option<(u64, Vec2)>,
    look: Option<(u64, Vec2)>,
    /// where the aim stick was first touched, and its current offset
    look_origin: Vec2,
    look_knob: Vec2,
    knob: Vec2,
    /// (fire, boost, reload) buttons currently held
    flags: (bool, bool, bool),
    /// (drive, reverse) buttons currently held in the aim + drive layout
    drive_flags: (bool, bool),
    /// time and travel of the current aim touch, for tap-to-fire
    look_t0: f32,
    look_moved: f32,
    boost_grace: f32,
    tap_pending: bool,
    item_pressed: bool,
}

fn demo_world(seed: u64) -> World {
    let specs: Vec<PlayerSpec> = (0..4)
        .map(|i| PlayerSpec { id: i, name: format!("Bot {}", i + 1), team: (i % 2) as u8, ai: true, kind: ((i as u64 + seed) % KINDS as u64) as u8 })
        .collect();
    World::new_match(&specs, seed)
}

fn local_ip() -> Option<String> {
    // "Connecting" a UDP socket sends nothing; it only picks the LAN interface.
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

fn wrap(mut d: f32) -> f32 {
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}

impl App {
    fn note(&mut self, s: &str) {
        self.notice = Some((s.to_string(), 4.0));
    }

    /// Where Settings/Garage return to: the lobby we came from, or the menu.
    fn back_target(&self) -> Screen {
        match self.mode {
            Mode::Host(_) => Screen::HostLobby,
            Mode::Client(_) => Screen::ClientLobby,
            Mode::Offline => Screen::Menu,
        }
    }

    /// Push the current name and tank to the lobby so others see changes made there.
    fn sync_profile(&mut self) {
        let (name, kind) = (self.settings.name.clone(), self.settings.tank);
        match &mut self.mode {
            Mode::Host(h) => h.set_profile(0, &name, kind),
            Mode::Client(c) => c.update_profile(&name, kind),
            Mode::Offline => {}
        }
    }

    fn leave_to_menu(&mut self) {
        match &mut self.mode {
            Mode::Host(h) => h.shutdown(),
            Mode::Client(c) => c.leave(),
            Mode::Offline => {}
        }
        self.mode = Mode::Offline;
        self.screen = Screen::Menu;
        self.particles.clear();
        self.release_mouse();
    }

    fn release_mouse(&mut self) {
        if self.grabbed {
            set_cursor_grab(false);
            show_mouse(true);
            self.grabbed = false;
        }
    }

    fn start_practice(&mut self) {
        let mut specs = vec![PlayerSpec { id: 0, name: self.settings.name.clone(), team: 0, ai: false, kind: self.settings.tank }];
        let pick = (get_time() * 1000.0) as u32;
        for i in 0..3 {
            specs.push(PlayerSpec { id: 1 + i, name: format!("Bot {}", i + 1), team: 1, ai: true, kind: ((pick / 7 + i * 2 + i / 2) % KINDS as u32) as u8 });
        }
        self.begin_match(World::build(&specs, (get_time() * 1000.0) as u64 + 1, false, self.settings.arena), 0);
        self.mode = Mode::Offline;
    }

    /// Solo capture-the-flag tournament: you plus bots on both sides.
    fn start_ctf_practice(&mut self) {
        let mt = self.settings.match_type.max(1) as u32;
        let mut specs = vec![PlayerSpec { id: 0, name: self.settings.name.clone(), team: 0, ai: false, kind: self.settings.tank }];
        let mut next = crate::ctf::BOT_ID_BASE;
        for team in 0..2u8 {
            let have = if team == 0 { 1 } else { 0 };
            for k in have..mt {
                specs.push(PlayerSpec { id: next, name: format!("Bot {}", k + 1), team, ai: true, kind: (next % KINDS as u32) as u8 });
                next += 1;
            }
        }
        self.mode = Mode::Offline;
        let mut w = World::build(&specs, (get_time() * 1000.0) as u64 + 3, true, self.settings.arena);
        // test hook: shorten the tournament for automated screenshots
        if let (Ok(t), Some(c)) = (std::env::var("TA_CTF_TIME"), w.ctf.as_mut()) {
            c.time_left = t.parse().unwrap_or(c.time_left);
        }
        self.begin_match(w, 0);
    }

    fn begin_match(&mut self, world: World, my_id: u32) {
        self.world = world;
        self.my_id = my_id;
        self.particles.clear();
        self.pending_events.clear();
        self.targets.clear();
        self.tracks.clear();
        self.play_time = 0.0;
        self.final_sent = false;
        self.cam = CamState::new(self.settings.chase_cam);
        self.aim = self.world.tank(my_id).map(|t| t.angle).unwrap_or(0.0);
        self.paused = false;
        self.fire_armed = false;
        self.touch = TouchCtl::default();
        self.screen = Screen::Play;
    }

    fn start_host(&mut self) {
        match Host::new(PORT, &self.settings.name, self.settings.match_type) {
            Ok(mut h) => {
                h.roster[0].kind = self.settings.tank;
                h.arena = self.settings.arena;
                self.mode = Mode::Host(h);
                self.screen = Screen::HostLobby;
            }
            Err(e) => self.note(&format!("Cannot host: {e}")),
        }
    }

    fn start_join(&mut self) {
        match Client::new(PORT, &self.settings.name) {
            Ok(mut c) => {
                c.kind = self.settings.tank;
                self.mode = Mode::Client(c);
                self.screen = Screen::Discovering;
                self.status = "Searching for a host on your WiFi...".into();
            }
            Err(e) => self.note(&format!("Cannot join: {e}")),
        }
    }

    fn host_begin(&mut self) {
        if let Mode::Host(h) = &mut self.mode {
            if !h.can_start() {
                return;
            }
            h.in_game = true;
            let specs = h.match_specs();
            let seed = (get_time() * 1000.0) as u64 + 7;
            let w = World::build(&specs, seed, h.ctf, h.arena);
            self.begin_match(w, 0);
        }
    }

    fn fx(&mut self, events: &[Event]) {
        for e in events {
            spawn_fx(&mut self.particles, e);
            self.audio.play_event(e);
        }
    }

    // ----------------------------------------------------------- input ----

    /// "Is the fire/drive control held?" Mouse, touch, or space bar. A press
    /// that started on a HUD button does not count.
    fn local_held(&self, start_on_button: bool) -> bool {
        (self.ui.down && !start_on_button) || is_key_down(KeyCode::Space)
    }
}

struct HoldTracker {
    active: bool,
}

#[macroquad::main(conf)]
async fn main() {
    let settings = Settings::load();
    let mut font = load_ttf_font_from_bytes(include_bytes!("../assets/fonts/font.ttf")).ok();
    if let Some(f) = font.as_mut() {
        f.set_filter(FilterMode::Nearest);
    }
    let audio = Audio::load(settings.music, settings.sfx).await;
    let mut ui = Ui::new(font);

    // Test hooks (used by the automated smoke tests; harmless otherwise):
    //   TA_CLICKS="frame:x:y,..."  scripted taps    TA_SHOTS="frame:path,..."  screenshots
    if let Ok(s) = std::env::var("TA_CLICKS") {
        for item in s.split(',') {
            let p: Vec<&str> = item.split(':').collect();
            if p.len() == 3 {
                if let (Ok(f), Ok(x), Ok(y)) = (p[0].parse(), p[1].parse(), p[2].parse()) {
                    ui.script.push((f, x, y));
                }
            }
        }
    }
    let shots: Vec<(u64, String)> = std::env::var("TA_SHOTS")
        .map(|s| {
            s.split(',')
                .filter_map(|i| i.split_once(':').and_then(|(f, p)| Some((f.parse().ok()?, p.to_string()))))
                .collect()
        })
        .unwrap_or_default();
    let prof_start = std::time::Instant::now();
    let exit_after: Option<u64> = std::env::var("TA_EXIT").ok().and_then(|s| s.parse().ok());
    let force_hold: Option<(u64, u64)> = std::env::var("TA_HOLD").ok().and_then(|s| {
        let (a, b) = s.split_once(':')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    });

    // TA_DRIVE="start:len:throttle:steer:fire:aimrate,..." scripts chase-view input.
    let forced_drive: Vec<(u64, u64, f32, f32, bool, f32)> = std::env::var("TA_DRIVE")
        .map(|s| {
            s.split(',')
                .filter_map(|seg| {
                    let p: Vec<&str> = seg.split(':').collect();
                    if p.len() != 6 {
                        return None;
                    }
                    Some((p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?, p[3].parse().ok()?, p[4] == "1", p[5].parse().ok()?))
                })
                .collect()
        })
        .unwrap_or_default();

    let mut app = App {
        screen: Screen::Menu,
        mode: Mode::Offline,
        cam: CamState::new(settings.chase_cam),
        settings,
        world: World::new_match(&[], 1),
        demo: demo_world(1),
        my_id: 0,
        particles: Vec::new(),
        pending_events: Vec::new(),
        scene: Scene::new(),
        tracks: TrackAnim::default(),
        demo_tracks: TrackAnim::default(),
        ui,
        audio,
        notice: None,
        roster: Vec::new(),
        lobby_match_type: 2,
        lobby_ctf: false,
        lobby_arena: 0,
        item_hold: 0.0,
        status: String::new(),
        play_time: 0.0,
        targets: HashMap::new(),
        quit: false,
        final_sent: false,
        aim: 0.0,
        touch: TouchCtl::default(),
        touch_mode: cfg!(target_os = "android") || std::env::var("TA_TOUCH").is_ok(),
        paused: false,
        help_page: 0,
        garage_sel: 0,
        grabbed: false,
        fire_armed: false,
        last_mouse: Vec2::ZERO,
        muted: false,
        fullscreen: false,
        forced_drive,
        test_pause: std::env::var("TA_PAUSE").ok().and_then(|v| v.parse().ok()),
    };
    let mut hold = HoldTracker { active: false };
    let mut demo_seed = 2u64;
    let mut frame: u64 = 0;
    let mut chase_toggle_hint = true;
    let _ = &mut chase_toggle_hint;

    loop {
        frame += 1;
        let dt = get_frame_time().min(1.0 / 20.0);
        app.ui.begin_frame();
        let expired = match app.notice.as_mut() {
            Some((_, t)) => {
                *t -= dt;
                *t <= 0.0
            }
            None => false,
        };
        if expired {
            app.notice = None;
        }

        if !app.touch_mode && !touches().is_empty() {
            app.touch_mode = true;
        }
        // global keys (not while typing a name / IP)
        if !matches!(app.screen, Screen::Keyboard { .. }) {
            if is_key_pressed(KeyCode::F) || is_key_pressed(KeyCode::F11) {
                app.fullscreen = !app.fullscreen;
                set_fullscreen(app.fullscreen);
            }
            if is_key_pressed(KeyCode::M) {
                app.muted = !app.muted;
                let v = if app.muted { 0.0 } else { app.settings.music };
                app.audio.set_music(v);
                app.note(if app.muted { "Music off (M)" } else { "Music on (M)" });
            }
        }

        clear_background(Color::new(0.04, 0.06, 0.1, 1.0));
        let in_play = matches!(app.screen, Screen::Play);

        if in_play {
            run_play(&mut app, dt, &mut hold, force_hold.map_or(false, |(s, l)| frame >= s && frame < s + l), frame);
        } else {
            app.release_mouse();
            if matches!(app.screen, Screen::Garage) {
                run_menus(&mut app);
            } else {
            // 3D bot battle as the menu backdrop
            app.demo.step(dt);
            app.demo.events.clear();
            if app.demo.phase == Phase::GameOver {
                demo_seed += 1;
                app.demo = demo_world(demo_seed);
            }
            let cam = app.cam.camera(&app.demo, 99, dt);
            set_camera(&cam);
            app.demo_tracks.update(&app.demo);
            app.scene.draw_world(&app.demo, get_time() as f32, &app.demo_tracks, None, None, cam.position);
            set_default_camera();
            draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.03, 0.05, 0.1, 0.62));
            run_menus(&mut app);
            }
        }

        // toast
        if let Some((msg, _)) = &app.notice {
            let w = app.ui.text_width(msg, 26.0) + 50.0;
            app.ui.fill(VW / 2.0 - w / 2.0, 640.0, w, 50.0, Color::new(0.5, 0.15, 0.15, 0.95));
            app.ui.text(msg, VW / 2.0, 665.0, 26.0, TEXT, Align::Center);
        }

        for (f, path) in &shots {
            if *f == frame {
                get_screen_data().export_png(path);
            }
        }
        if app.quit || exit_after.map_or(false, |n| frame >= n) {
            if std::env::var("TA_PROF").is_ok() {
                use std::sync::atomic::Ordering::Relaxed;
                let f = frame.max(1) as f64;
                println!(
                    "PROF frames={} cpu_draw_world={:.3} ms/frame verts={:.0}/frame meshes={:.1}/frame wall_frame={:.2} ms",
                    frame,
                    gfx::PROF_NS.load(Relaxed) as f64 / 1e6 / f,
                    gfx::PROF_VERTS.load(Relaxed) as f64 / f,
                    gfx::PROF_CALLS.load(Relaxed) as f64 / f,
                    prof_start.elapsed().as_secs_f64() * 1000.0 / f
                );
            }
            if let Mode::Host(h) = &mut app.mode {
                h.shutdown();
            }
            if let Mode::Client(c) = &mut app.mode {
                c.leave();
            }
            break;
        }
        next_frame().await;
    }
}

/// Keep a joined client talking to the host while it sits in the lobby (or in
/// Settings/Garage opened from it). Returns true when the screen changed
/// (kicked, lost the host, or the match began).
fn client_lobby_poll(app: &mut App) -> bool {
    let mut left: Option<String> = None;
    let mut start = false;
    if let Mode::Client(c) = &mut app.mode {
        for m in c.poll(Control::default()) {
            match m {
                ClientMsg::Lobby { match_type, ctf, arena, roster } => {
                    app.lobby_match_type = match_type;
                    app.lobby_ctf = ctf;
                    app.lobby_arena = arena;
                    app.roster = roster;
                }
                ClientMsg::Reject(r) => left = Some(r),
                ClientMsg::State(s) => {
                    if s.starts_with("STATE:P") {
                        start = true;
                        let specs = roster_to_specs(&app.roster);
                        let mut w = World::build(&specs, 0, app.lobby_ctf, app.lobby_arena);
                        w.apply_state(&s);
                        app.world = w;
                    }
                }
                _ => {}
            }
        }
        if c.host_timed_out() {
            left = Some("Lost connection to the host".into());
        }
        app.my_id = c.my_id.unwrap_or(u32::MAX);
    }
    if let Some(r) = left {
        app.leave_to_menu();
        app.note(&r);
        return true;
    }
    if start {
        let w = std::mem::replace(&mut app.world, World::new_match(&[], 1));
        let id = app.my_id;
        app.begin_match(w, id);
        return true;
    }
    false
}

// ------------------------------------------------------------------ menus ----

fn run_menus(app: &mut App) {
    let cx = VW / 2.0;
    // networking housekeeping while in lobbies
    match &mut app.mode {
        Mode::Host(h) if !matches!(app.screen, Screen::Play) => {
            h.poll();
            h.send_lobby();
        }
        Mode::Client(c) if c.my_id.is_some() && matches!(app.screen, Screen::Settings | Screen::Garage | Screen::Keyboard { target: KbTarget::Name, .. }) => {
            if client_lobby_poll(app) {
                return;
            }
        }
        _ => {}
    }
    if is_key_pressed(KeyCode::Escape) && !matches!(app.screen, Screen::Keyboard { .. }) {
        match app.screen {
            Screen::Menu => app.quit = cfg!(not(target_os = "android")),
            Screen::Settings => {
                app.settings.save();
                app.sync_profile();
                app.screen = app.back_target();
            }
            Screen::Garage => {
                app.sync_profile();
                app.screen = app.back_target();
            }
            Screen::HowTo => {
                app.help_page = 0;
                app.screen = Screen::Menu;
            }
            _ => app.leave_to_menu(),
        }
    }

    // take the screen out so we can mutate app while matching
    let mut screen = std::mem::replace(&mut app.screen, Screen::Menu);
    match &mut screen {
        Screen::Menu => {
            app.ui.title("TANK ARENA 3D", 110.0);
            app.ui.text("tap to fire  ·  hold to drive  ·  bounce your shots", cx, 175.0, 22.0, DIM, Align::Center);
            app.ui.text(&format!("Playing as {}", app.settings.name), cx, 205.0, 22.0, ACCENT, Align::Center);
            let (w, h) = (460.0, 58.0);
            let x = cx - w / 2.0;
            let half0 = (w - 12.0) / 2.0;
            if app.ui.button(x, 235.0, half0, h, "PRACTICE", Style::Primary) {
                app.start_practice();
            }
            if app.ui.button(x + half0 + 12.0, 235.0, half0, h, "CTF VS BOTS", Style::Primary) {
                app.start_ctf_practice();
                return;
            }
            if app.ui.button(x, 300.0, w, h, "HOST LAN GAME", Style::Normal) {
                app.start_host();
            }
            if app.ui.button(x, 365.0, w, h, "JOIN LAN GAME", Style::Normal) {
                app.start_join();
            }
            let garage_label = format!("GARAGE: {}", stats(app.settings.tank).name);
            if app.ui.button(x, 430.0, w, h, &garage_label, Style::Normal) {
                app.garage_sel = app.settings.tank;
                app.screen = Screen::Garage;
                return;
            }
            let half = (w - 12.0) / 2.0;
            if app.ui.button(x, 495.0, half, h, "SETTINGS", Style::Normal) {
                app.screen = Screen::Settings;
                return;
            }
            if app.ui.button(x + half + 12.0, 495.0, half, h, "HOW TO PLAY", Style::Normal) {
                app.help_page = 0;
                app.screen = Screen::HowTo;
                return;
            }
            let arena_label = format!("ARENA: {}", arena_name(app.settings.arena));
            if app.ui.button(x, 560.0, w, 50.0, &arena_label, Style::Normal) {
                app.settings.arena = (app.settings.arena + 1) % ARENAS;
                app.settings.save();
            }
            if cfg!(not(target_os = "android")) && app.ui.button(x, 622.0, w, 46.0, "QUIT", Style::Danger) {
                app.quit = true;
            }
            if !app.touch_mode {
                app.ui.text("1 PRACTICE  2 HOST  3 JOIN  4 CTF  A ARENA  G GARAGE  S SETTINGS  H HELP", cx, 696.0, 18.0, DIM, Align::Center);
                if is_key_pressed(KeyCode::Key1) || is_key_pressed(KeyCode::Enter) {
                    app.start_practice();
                } else if is_key_pressed(KeyCode::Key4) {
                    app.start_ctf_practice();
                    return;
                } else if is_key_pressed(KeyCode::Key2) {
                    app.start_host();
                } else if is_key_pressed(KeyCode::Key3) {
                    app.start_join();
                } else if is_key_pressed(KeyCode::A) {
                    app.settings.arena = (app.settings.arena + 1) % ARENAS;
                    app.settings.save();
                } else if is_key_pressed(KeyCode::G) {
                    app.garage_sel = app.settings.tank;
                    app.screen = Screen::Garage;
                    return;
                } else if is_key_pressed(KeyCode::S) {
                    app.screen = Screen::Settings;
                    return;
                } else if is_key_pressed(KeyCode::H) {
                    app.help_page = 0;
                    app.screen = Screen::HowTo;
                    return;
                }
            }
            if !matches!(app.screen, Screen::Menu) {
                return; // an action switched screens
            }
        }
        Screen::Settings => {
            app.ui.title("SETTINGS", 64.0);
            app.ui.panel(300.0, 104.0, 680.0, 556.0);
            let before = app.settings.serialize();
            let (bh, step) = (54.0, 62.0);
            let row = |i: f32| 122.0 + i * step;
            // name
            app.ui.text("Name", 340.0, row(0.0) + 27.0, 26.0, DIM, Align::Left);
            let name = app.settings.name.clone();
            if app.ui.button(560.0, row(0.0), 380.0, bh, &name, Style::Normal) {
                screen = Screen::Keyboard { target: KbTarget::Name, text: String::new() };
                app.screen = screen;
                return;
            }
            // volumes
            for (i, label) in ["Music", "Effects"].iter().enumerate() {
                let y = row(1.0 + i as f32);
                let v = if i == 0 { app.settings.music } else { app.settings.sfx };
                app.ui.text(label, 340.0, y + 27.0, 26.0, DIM, Align::Left);
                let mut nv = v;
                if app.ui.button(560.0, y, 70.0, bh, "-", Style::Normal) {
                    nv = ((v * 10.0).round() - 1.0).max(0.0) / 10.0;
                }
                if app.ui.button(870.0, y, 70.0, bh, "+", Style::Normal) {
                    nv = ((v * 10.0).round() + 1.0).min(10.0) / 10.0;
                }
                app.ui.fill(645.0, y + 19.0, 210.0, 16.0, Color::new(0.02, 0.03, 0.06, 1.0));
                app.ui.fill(645.0, y + 19.0, 210.0 * v, 16.0, ACCENT);
                if nv != v {
                    if i == 0 {
                        app.settings.music = nv;
                        app.muted = false;
                        app.audio.set_music(nv);
                    } else {
                        app.settings.sfx = nv;
                        app.audio.set_sfx(nv);
                        app.audio.blip();
                    }
                }
            }
            // start view: MAP = tap/hold controls, CHASE = drive controls
            app.ui.text("View", 340.0, row(3.0) + 27.0, 26.0, DIM, Align::Left);
            let cam_label = if app.settings.chase_cam { "CHASE (DRIVE)" } else { "MAP (TAP/HOLD)" };
            if app.ui.button(560.0, row(3.0), 380.0, bh, cam_label, Style::Normal) {
                app.settings.chase_cam = !app.settings.chase_cam;
            }
            // look sensitivity
            app.ui.text("Aim", 340.0, row(4.0) + 27.0, 26.0, DIM, Align::Left);
            let sv = app.settings.sens;
            if app.ui.button(560.0, row(4.0), 70.0, bh, "-", Style::Normal) {
                app.settings.sens = (((sv * 10.0).round() - 1.0) / 10.0).max(0.3);
            }
            if app.ui.button(870.0, row(4.0), 70.0, bh, "+", Style::Normal) {
                app.settings.sens = (((sv * 10.0).round() + 1.0) / 10.0).min(3.0);
            }
            app.ui.text(&format!("{:.1}x", sv), 750.0, row(4.0) + 27.0, 28.0, TEXT, Align::Center);
            let mut next = 5.0;
            if app.touch_mode {
                app.ui.text("Touch aim", 340.0, row(5.0) + 27.0, 26.0, DIM, Align::Left);
                let aim_label = if app.settings.aim_stick { "RIGHT STICK" } else { "DRAG RIGHT SIDE" };
                if app.ui.button(560.0, row(5.0), 380.0, bh, aim_label, Style::Normal) {
                    app.settings.aim_stick = !app.settings.aim_stick;
                }
                app.ui.text("Touch move", 340.0, row(6.0) + 27.0, 26.0, DIM, Align::Left);
                let mv_label = if app.settings.aim_drive { "AIM + DRIVE BUTTON" } else { "LEFT STICK" };
                if app.ui.button(560.0, row(6.0), 380.0, bh, mv_label, Style::Normal) {
                    app.settings.aim_drive = !app.settings.aim_drive;
                }
                app.ui.text("Tap to fire", 340.0, row(7.0) + 27.0, 26.0, DIM, Align::Left);
                let tf_label = if app.settings.tap_fire { "ON" } else { "OFF" };
                if app.ui.button(560.0, row(7.0), 380.0, bh, tf_label, Style::Normal) {
                    app.settings.tap_fire = !app.settings.tap_fire;
                }
                next = 8.0;
            }
            if app.ui.button(cx - 150.0, row(next) + 6.0, 300.0, 50.0, "BACK", Style::Primary) {
                app.settings.save();
                app.sync_profile();
                app.screen = app.back_target();
                return;
            }
            // save as soon as anything changes, in case the app is closed from Android
            if app.settings.serialize() != before {
                app.settings.save();
            }
        }
        Screen::HowTo => {
            // Paged so nothing can run off the panel. Text shrinks if a page is still too tall.
            let pages: [(&str, Vec<(String, String)>); 5] = [
                (
                    "CONTROLS",
                    vec![
                        ("MAP VIEW".into(), "Tap anywhere to fire (your spin reverses). Hold to drive, release to spin again.".into()),
                        ("CHASE".into(), "W A S D or arrows move the tank the way you press, relative to the camera. Mouse or Q E aims. Click or Space fires.".into()),
                        ("MORE".into(), "Shift (or Ctrl/X) boosts, R reloads, C switches view, Esc pauses, M music, F fullscreen.".into()),
                        ("TOUCH".into(), "Left stick drives, drag the right side to aim, hold FIRE. Settings can switch to a right aim stick, or to AIM + DRIVE: the tank goes where you aim while you hold DRIVE.".into()),
                    ],
                ),
                (
                    "BATTLE",
                    vec![
                        ("SHOTS".into(), "A magazine, then a reload. Bullets ricochet twice. The dotted line previews your bounce.".into()),
                        ("TEAMS".into(), "Teammates cannot hurt each other. The last team standing wins.".into()),
                        ("PICKUPS".into(), "Drive over the glowing cubes: repair, reload, nitro, rapid fire, shield.".into()),
                        ("LAN".into(), "One player hosts, the others tap JOIN on the same WiFi. Pick your tank in the garage first.".into()),
                    ],
                ),
                (
                    "THE FIELD",
                    vec![
                        ("WALLS".into(), "Bricks, crates, rocks, trees, sandbags, concrete blocks and the tower are solid. Shots bounce off them.".into()),
                        ("WATER".into(), "Ponds stop tanks, but bullets fly right over them.".into()),
                        ("MUD".into(), "Mud patches slow you down. Bullets ignore them, so cross quickly.".into()),
                    ],
                ),
                (
                    "TANKS",
                    (0..KINDS as u8)
                        .map(|k| {
                            let st = stats(k);
                            (st.name.to_string(), format!("{} - armour {}, {} rounds.", st.blurb, st.hp, st.ammo))
                        })
                        .collect(),
                ),
                (
                    "CAPTURE THE FLAG",
                    vec![
                        ("GOAL".into(), "Steal the enemy flag and carry it to your own base. You score only while your own flag is safe at home.".into()),
                        ("FLAGS".into(), "Shoot the carrier and the flag drops. Touch your dropped flag to send it home, or it returns by itself after 12 seconds.".into()),
                        ("RESPAWN".into(), "Destroyed tanks come back at their base after 4 seconds. Bots fill every empty slot.".into()),
                        ("ITEMS".into(), "Purple boxes hold one secret item: a bomb mine, a shield or a spread shot. Use it with V or the ITEM button.".into()),
                        ("GATES".into(), "Machine gates guard each base and fire single shots on a rhythm, straight along the lane. The barrel glows just before. Their shots never hurt their own team, and a shield blocks them.".into()),
                        ("ARENAS".into(), "Pick the arena on the menu (A) or in the host lobby: Meadow, Foundry (slag pit) or Facility (steel bunker with a grated corridor). Gates shoot straight in the last two. No walls, only the edge line.".into()),
                        ("SCORE".into(), "The score is hidden during the 4 minute tournament and revealed at the end.".into()),
                    ],
                ),
            ];
            let np = pages.len();
            app.help_page = app.help_page.min(np - 1);
            let (head, lines) = &pages[app.help_page];
            app.ui.title("HOW TO PLAY", 70.0);
            let (px, py, pw, ph) = (140.0, 118.0, 1000.0, 478.0);
            app.ui.panel(px, py, pw, ph);
            app.ui.text(&format!("{}  {}/{}", head, app.help_page + 1, np), cx, py + 30.0, 28.0, TEXT, Align::Center);
            let label_w = 230.0;
            let text_w = pw - label_w - 70.0;
            let avail = ph - 80.0;
            // pick the biggest text size whose wrapped lines fit the panel
            let mut size = 26.0f32;
            let layout = loop {
                let lh = size * 0.95;
                let mut total = 0.0;
                let mut rows = Vec::new();
                for (k, v) in lines.iter() {
                    let wrapped = app.ui.wrap(v, size, text_w);
                    total += wrapped.len().max(1) as f32 * lh + 16.0;
                    rows.push((k.clone(), wrapped));
                }
                if total <= avail || size <= 16.0 {
                    break (rows, lh);
                }
                size -= 2.0;
            };
            let mut y = py + 78.0;
            for (k, wrapped) in layout.0.iter() {
                app.ui.text_fit(k, px + 30.0, y, size, ACCENT, Align::Left, label_w - 10.0);
                for (li, l) in wrapped.iter().enumerate() {
                    app.ui.text_fit(l, px + label_w + 30.0, y + li as f32 * layout.1, size, TEXT, Align::Left, text_w + 20.0);
                }
                y += wrapped.len().max(1) as f32 * layout.1 + 16.0;
            }
            let by = 616.0;
            if app.ui.button(px, by, 220.0, 54.0, "< PREV", if app.help_page > 0 { Style::Normal } else { Style::Disabled }) && app.help_page > 0 {
                app.help_page -= 1;
            }
            if app.ui.button(px + pw - 220.0, by, 220.0, 54.0, "NEXT >", if app.help_page + 1 < np { Style::Normal } else { Style::Disabled }) && app.help_page + 1 < np {
                app.help_page += 1;
            }
            if app.ui.button(cx - 130.0, by, 260.0, 54.0, "BACK", Style::Primary) {
                app.help_page = 0;
                app.screen = Screen::Menu;
                return;
            }
            if is_key_pressed(KeyCode::Right) && app.help_page + 1 < np {
                app.help_page += 1;
            }
            if is_key_pressed(KeyCode::Left) && app.help_page > 0 {
                app.help_page -= 1;
            }
        }
        Screen::Garage => {
            let t = get_time() as f32;
            // backdrop
            let (sw, sh) = (screen_width(), screen_height());
            draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.07, 0.10, 0.16, 1.0));
            draw_rectangle(0.0, sh * 0.62, sw, sh * 0.38, Color::new(0.10, 0.15, 0.17, 1.0));
            let sel = app.garage_sel.min(KINDS as u8 - 1);
            let cam = Camera3D {
                position: vec3(2.9, 2.2, 4.4),
                target: vec3(1.7, 0.3, 0.0),
                up: vec3(0.0, 1.0, 0.0),
                fovy: 38f32.to_radians(),
                ..Default::default()
            };
            set_camera(&cam);
            let mut b = Batch::new();
            b.cyl(vec3(0.0, -0.05, 0.0), 1.5, 0.1, Color::new(0.2, 0.22, 0.26, 1.0));
            b.cyl(vec3(0.0, -0.01, 0.0), 1.38, 0.04, Color::new(0.3, 0.32, 0.37, 1.0));
            let mut tank = Tank::new(0, 0, "", ARENA_W / 2.0, ARENA_H / 2.0, t * 0.55, false);
            tank.set_kind(sel);
            tank.turret = tank.angle + (t * 0.8).sin() * 0.5;
            app.scene.draw_tank(&mut b, &tank, 0, t, (t * 0.6, t * 0.6), false, true);
            b.flush();
            set_default_camera();

            app.ui.title("GARAGE", 62.0);
            let st = stats(sel);
            let (px, py, pw, ph) = (700.0, 130.0, 500.0, 470.0);
            app.ui.panel(px, py, pw, ph);
            app.ui.text_fit(st.name, px + pw / 2.0, py + 40.0, 44.0, ACCENT, Align::Center, pw - 40.0);
            for (i, l) in app.ui.wrap(st.blurb, 24.0, pw - 60.0).iter().enumerate() {
                app.ui.text(l, px + pw / 2.0, py + 90.0 + i as f32 * 26.0, 24.0, TEXT, Align::Center);
            }
            let bars = [
                ("ARMOUR", st.hp as f32 / 5.0, format!("{}", st.hp)),
                ("SPEED", st.speed / 145.0, format!("{:.0}", st.speed)),
                ("FIRE RATE", (1.0 / st.cooldown) / 11.2, format!("{:.1}/s", 1.0 / st.cooldown)),
                ("MAGAZINE", st.ammo as f32 / 8.0, format!("{}", st.ammo)),
                ("RELOAD", 1.0 - (st.reload - 1.0) / 1.6, format!("{:.1}s", st.reload)),
            ];
            for (i, (name, f, val)) in bars.iter().enumerate() {
                let y = py + 150.0 + i as f32 * 50.0;
                app.ui.text(name, px + 30.0, y + 12.0, 22.0, DIM, Align::Left);
                app.ui.fill(px + 190.0, y, 190.0, 22.0, Color::new(0.2, 0.22, 0.28, 1.0));
                app.ui.fill(px + 190.0, y, 190.0 * f.clamp(0.04, 1.0), 22.0, Color::new(0.95, 0.7, 0.2, 1.0));
                app.ui.text(val, px + 392.0, y + 12.0, 20.0, TEXT, Align::Left);
            }
            let by = py + ph - 62.0;
            if app.ui.button(px + 20.0, by, 90.0, 50.0, "<", Style::Normal) || is_key_pressed(KeyCode::Left) {
                app.garage_sel = (sel + KINDS as u8 - 1) % KINDS as u8;
            }
            if app.ui.button(px + pw - 110.0, by, 90.0, 50.0, ">", Style::Normal) || is_key_pressed(KeyCode::Right) {
                app.garage_sel = (sel + 1) % KINDS as u8;
            }
            let chosen = app.settings.tank == sel;
            let lab = if chosen { "SELECTED" } else { "SELECT" };
            if app.ui.button(px + 130.0, by, pw - 260.0, 50.0, lab, if chosen { Style::Disabled } else { Style::Primary }) && !chosen || (is_key_pressed(KeyCode::Enter) && !chosen) {
                app.settings.tank = sel;
                app.settings.save();
            }
            if app.ui.button(60.0, 636.0, 260.0, 54.0, "BACK", Style::Normal) {
                app.sync_profile();
                app.screen = app.back_target();
                return;
            }
        }
        Screen::Keyboard { target, text } => {
            let (title, numeric, max) = match target {
                KbTarget::Name => ("Enter your name", false, 12),
                KbTarget::Ip => ("Enter host IP address", true, 15),
            };
            match app.ui.keyboard(title, text, max, numeric) {
                Kb::Editing => {}
                Kb::Cancel => {
                    app.screen = match target {
                        KbTarget::Name => Screen::Settings,
                        KbTarget::Ip => Screen::Discovering,
                    };
                    return;
                }
                Kb::Done => {
                    match target {
                        KbTarget::Name => {
                            app.settings.name = sanitize_name(text);
                            app.settings.save();
                            app.screen = Screen::Settings;
                        }
                        KbTarget::Ip => {
                            let addr: Result<SocketAddr, _> = format!("{}:{}", text.trim(), PORT).parse();
                            match (addr, &mut app.mode) {
                                (Ok(a), Mode::Client(c)) => {
                                    c.connect(a);
                                    app.status = format!("Connecting to {a}...");
                                }
                                _ => app.note("That is not a valid IP address"),
                            }
                            app.screen = Screen::Discovering;
                        }
                    }
                    return;
                }
            }
        }
        Screen::Discovering => {
            app.ui.title("JOIN LAN GAME", 100.0);
            app.ui.panel(300.0, 170.0, 680.0, 300.0);
            let dots = ".".repeat(((get_time() * 2.0) as usize) % 4);
            app.ui.text(&format!("{}{}", app.status.trim_end_matches('.'), dots), cx, 230.0, 26.0, TEXT, Align::Center);
            app.ui.text("Joins automatically when found.", cx, 275.0, 20.0, DIM, Align::Center);
            let mut manual = false;
            let mut found: Option<HostInfo> = None;
            let mut welcomed = false;
            let mut rejected: Option<String> = None;
            let mut lobby = None;
            if let Mode::Client(c) = &mut app.mode {
                if c.host.is_none() {
                    c.discover();
                }
                for m in c.poll(Control::default()) {
                    match m {
                        ClientMsg::Hello(h) => {
                            if c.host.is_none() && !h.in_game && h.players < h.capacity {
                                found = Some(h);
                            }
                        }
                        ClientMsg::Welcome(_) => welcomed = true,
                        ClientMsg::Reject(r) => rejected = Some(r),
                        ClientMsg::Lobby { match_type, roster, .. } => lobby = Some((match_type, roster)),
                        ClientMsg::State(_) => {}
                    }
                }
                if let Some(h) = found {
                    app.status = format!("Found {} - joining", h.name);
                    c.connect(h.addr);
                }
                if c.host.is_some() && c.host_timed_out() {
                    c.host = None;
                    rejected = Some("Could not reach that host".into());
                }
            }
            if let Some((mt, r)) = lobby {
                app.lobby_match_type = mt;
                app.roster = r;
            }
            if let Some(r) = rejected {
                if let Mode::Client(c) = &mut app.mode {
                    c.host = None;
                }
                app.status = "Searching for a host on your WiFi...".into();
                app.note(&r);
            }
            if welcomed {
                app.screen = Screen::ClientLobby;
                return;
            }
            if app.ui.button(cx - 230.0, 330.0, 460.0, 60.0, "ENTER IP MANUALLY", Style::Normal) {
                manual = true;
            }
            if app.ui.button(cx - 230.0, 405.0, 460.0, 52.0, "BACK", Style::Danger) {
                app.leave_to_menu();
                return;
            }
            if manual {
                app.screen = Screen::Keyboard { target: KbTarget::Ip, text: String::new() };
                return;
            }
        }
        Screen::HostLobby => {
            app.ui.title("HOST GAME", 70.0);
            if app.ui.button(30.0, 20.0, 220.0, 50.0, "SETTINGS", Style::Normal) {
                app.screen = Screen::Settings;
                return;
            }
            if app.ui.button(1030.0, 20.0, 220.0, 50.0, "GARAGE", Style::Normal) {
                app.screen = Screen::Garage;
                return;
            }
            let ip = local_ip().unwrap_or_else(|| "?".into());
            let (roster, mt, can_start) = match &app.mode {
                Mode::Host(h) => (h.full_roster(), h.match_type, h.can_start()),
                _ => (Vec::new(), 2, false),
            };
            let ctf_now = matches!(&app.mode, Mode::Host(h) if h.ctf);
            app.ui.text(&format!("Your IP: {}", ip), cx, 128.0, 24.0, DIM, Align::Center);
            draw_roster(&app.ui, &roster, mt, 0, ctf_now);
            if let Some(t) = team_tap(&app.ui, &roster, mt, 0) {
                if let Mode::Host(h) = &mut app.mode {
                    h.swap_team(0, t);
                }
            }
            let mode_label = format!("MODE: {}v{}", mt, mt);
            let ctf = matches!(&app.mode, Mode::Host(h) if h.ctf);
            let bw = 172.0;
            let bx = cx - (bw * 4.0 + 36.0) / 2.0;
            if app.ui.button(bx, 585.0, bw, 60.0, &mode_label, Style::Normal) {
                if let Mode::Host(h) = &mut app.mode {
                    h.cycle_match_type();
                    app.settings.match_type = h.match_type;
                    app.settings.save();
                }
            }
            let arena_now = if let Mode::Host(h) = &app.mode { h.arena } else { 0 };
            if app.ui.button(VW - 330.0, 100.0, 300.0, 46.0, &format!("ARENA: {}", arena_name(arena_now)), Style::Normal) {
                if let Mode::Host(h) = &mut app.mode {
                    h.cycle_arena();
                    app.settings.arena = h.arena;
                    app.settings.save();
                }
            }
            let game_label = if ctf { "GAME: CTF" } else { "GAME: BATTLE" };
            if app.ui.button(bx + bw + 12.0, 585.0, bw, 60.0, game_label, if ctf { Style::Primary } else { Style::Normal }) {
                if let Mode::Host(h) = &mut app.mode {
                    h.toggle_ctf();
                }
            }
            let st = if can_start { Style::Primary } else { Style::Disabled };
            if (app.ui.button(bx + (bw + 12.0) * 2.0, 585.0, bw, 60.0, "START", st) || is_key_pressed(KeyCode::Enter)) && can_start {
                app.screen = Screen::Menu;
                app.host_begin();
                return;
            }
            if app.ui.button(bx + (bw + 12.0) * 3.0, 585.0, bw, 60.0, "BACK", Style::Danger) {
                app.leave_to_menu();
                return;
            }
            if !can_start {
                app.ui.text("Waiting for at least one more player...", cx, 560.0, 20.0, DIM, Align::Center);
            }
        }
        Screen::ClientLobby => {
            app.ui.title("LOBBY", 70.0);
            if app.ui.button(30.0, 20.0, 220.0, 50.0, "SETTINGS", Style::Normal) {
                app.screen = Screen::Settings;
                return;
            }
            if app.ui.button(1030.0, 20.0, 220.0, 50.0, "GARAGE", Style::Normal) {
                app.screen = Screen::Garage;
                return;
            }
            if client_lobby_poll(app) {
                return;
            }
            draw_roster(&app.ui, &app.roster, app.lobby_match_type, app.my_id, app.lobby_ctf);
            if let Some(t) = team_tap(&app.ui, &app.roster, app.lobby_match_type, app.my_id) {
                if let Mode::Client(c) = &mut app.mode {
                    c.request_team(t);
                }
                let me = app.my_id;
                let room = app.roster.iter().filter(|e| e.team == t).count() < app.lobby_match_type as usize && !app.lobby_ctf;
                if room {
                    if let Some(e) = app.roster.iter_mut().find(|e| e.id == me) {
                        e.team = t; // shows instantly; the host's roster confirms it
                    }
                }
            }
            app.ui.text("Waiting for the host to start...", cx, 560.0, 24.0, ACCENT, Align::Center);
            if app.ui.button(cx - 150.0, 595.0, 300.0, 56.0, "LEAVE", Style::Danger) {
                app.leave_to_menu();
                return;
            }
        }
        Screen::Play => {}
    }
    // keep the screen unless something already replaced it
    if matches!(app.screen, Screen::Menu) {
        app.screen = screen;
    } else if matches!(app.screen, Screen::Play) {
        // begin_match set it; nothing to restore
    }
}

fn draw_roster(ui: &Ui, roster: &[RosterEntry], mt: u8, me: u32, ctf: bool) {
    let humans = roster.iter().filter(|e| e.id < crate::ctf::BOT_ID_BASE).count();
    let kind = if ctf { "CTF tournament" } else { "match" };
    ui.text(&format!("{}v{} {}   {}/{} players", mt, mt, kind, humans, capacity(mt)), VW / 2.0, 160.0, 24.0, TEXT, Align::Center);
    let my_team = roster.iter().find(|e| e.id == me).map(|e| e.team);
    for team in 0..2u8 {
        let x = 200.0 + team as f32 * 460.0;
        let count = roster.iter().filter(|e| e.team == team).count();
        let (name, col) = if team == 0 { ("BLUE TEAM", TEAM_BLUE) } else { ("RED TEAM", TEAM_RED) };
        ui.panel(x, 190.0, 420.0, 350.0);
        if my_team == Some(team) {
            let r = ui.r(x, 190.0, 420.0, 350.0);
            draw_rectangle_lines(r.x, r.y, r.w, r.h, 5.0 * ui.u, col);
        }
        ui.text(name, x + 210.0, 220.0, 28.0, col, Align::Center);
        for (i, e) in roster.iter().filter(|e| e.team == team).enumerate() {
            let you = if e.id == me { " (you)" } else if e.id >= crate::ctf::BOT_ID_BASE { " (bot)" } else { "" };
            ui.text_fit(&format!("{}{} [{}]", e.name, you, stats(e.kind).name), x + 30.0, 275.0 + i as f32 * 46.0, 24.0, TEXT, Align::Left, 370.0);
        }
        let foot = if my_team == Some(team) {
            "YOUR TEAM"
        } else if !ctf && count >= mt as usize {
            "FULL"
        } else {
            "TAP HERE TO JOIN"
        };
        ui.text(foot, x + 210.0, 512.0, 22.0, if my_team == Some(team) { col } else { ACCENT }, Align::Center);
    }
}

/// A tap (or click) on a team panel that this player can move to.
fn team_tap(ui: &Ui, roster: &[RosterEntry], mt: u8, me: u32) -> Option<u8> {
    let click = ui.click?;
    let my_team = roster.iter().find(|e| e.id == me).map(|e| e.team);
    for team in 0..2u8 {
        let x = 200.0 + team as f32 * 460.0;
        let r = ui.r(x, 190.0, 420.0, 350.0);
        let _ = mt; // a full side is fine: the host swaps places
        if r.contains(click) && my_team != Some(team) {
            return Some(team);
        }
    }
    None
}

// ------------------------------------------------------------------- play ----

// Touch layout in virtual 1280x720 coordinates: (centre x, centre y, radius).
const FIRE_BTN: (f32, f32, f32) = (1130.0, 560.0, 90.0);
const BOOST_BTN: (f32, f32, f32) = (975.0, 625.0, 58.0);
const RELOAD_BTN: (f32, f32, f32) = (1190.0, 410.0, 46.0);
const ITEM_BTN: (f32, f32, f32) = (1050.0, 420.0, 50.0);
const STICK_R: f32 = 95.0;
// "aim + drive button" touch layout
const DRIVE_BTN: (f32, f32, f32) = (190.0, 540.0, 105.0);
const REV_BTN: (f32, f32, f32) = (390.0, 640.0, 46.0);

/// Keyboard + mouse (+ test script) input for the chase-camera scheme.
/// W/S throttle, A/D steer (or arrows), mouse or Q/E aim, Space or left
/// click fire, Shift boost, R reload.
fn gather_drive(app: &mut App, dt: f32, frame: u64) -> Control {
    let kd = is_key_down;
    let mut c = Control { drive: true, ..Default::default() };
    // car-style: W/A/S/D mean "go that way on screen" (relative to the camera).
    let mut right = 0.0f32;
    let mut fwd = 0.0f32;
    if kd(KeyCode::D) || kd(KeyCode::Right) {
        right += 1.0;
    }
    if kd(KeyCode::A) || kd(KeyCode::Left) {
        right -= 1.0;
    }
    if kd(KeyCode::W) || kd(KeyCode::Up) {
        fwd += 1.0;
    }
    if kd(KeyCode::S) || kd(KeyCode::Down) {
        fwd -= 1.0;
    }
    let hull = app.world.tank(app.my_id).map(|t| t.angle).unwrap_or(0.0);
    let (mut steer, mut throttle) = if right != 0.0 || fwd != 0.0 {
        screen_drive(app.aim, hull, right, fwd, 1.0)
    } else {
        (0.0, 0.0)
    };
    c.fire = kd(KeyCode::Space);
    // Shift (either), Ctrl or X boost; a short grace keeps boost alive across the
    // key-event hiccups some keyboards/platforms produce when arrows and Shift mix.
    let boost_now = kd(KeyCode::LeftShift) || kd(KeyCode::RightShift) || kd(KeyCode::LeftControl) || kd(KeyCode::RightControl) || kd(KeyCode::X);
    app.touch.boost_grace = if boost_now { 0.12 } else { (app.touch.boost_grace - dt).max(0.0) };
    c.boost = boost_now || app.touch.boost_grace > 0.0;
    c.reload = kd(KeyCode::R);

    let sens = app.settings.sens;
    app.aim += (kd(KeyCode::E) as i32 - kd(KeyCode::Q) as i32) as f32 * 2.6 * dt * sens;

    // mouse look (the cursor is grabbed while playing, so position changes are raw motion)
    let mp = Vec2::from(mouse_position());
    if app.grabbed {
        app.aim += (mp.x - app.last_mouse.x) * 0.0035 * sens;
        if is_mouse_button_down(MouseButton::Left) {
            c.fire |= app.fire_armed; // ignore the click that started the match
        } else {
            app.fire_armed = true;
        }
    }
    app.last_mouse = mp;

    if app.touch_mode {
        let (st, th, fire, boost, reload) = touch_input(app);
        if st != 0.0 || th != 0.0 {
            steer = st;
            throttle = th;
        }
        c.fire |= fire;
        c.boost |= boost;
        c.reload |= reload;
    }

    // scripted input for automated screenshots/tests
    for &(start, len, th, st, fire, aim_rate) in &app.forced_drive {
        if frame >= start && frame < start + len {
            throttle = th;
            steer = st;
            c.fire |= fire;
            app.aim += aim_rate * dt;
        }
    }

    app.aim = wrap(app.aim);
    c.steer = steer.clamp(-1.0, 1.0);
    c.throttle = throttle.clamp(-1.0, 1.0);
    c.aim = app.aim;
    c
}

/// Turn a screen-relative direction (right, forward) into hull steer/throttle.
/// The tank turns toward that direction and drives; if the direction is mostly
/// behind it, it reverses toward it instead of making a U-turn.
fn screen_drive(aim: f32, hull: f32, right: f32, fwd: f32, mag: f32) -> (f32, f32) {
    let target = aim + right.atan2(fwd);
    let diff = wrap(target - hull);
    let (err, sign) = if diff.abs() > 1.9 { (wrap(diff + std::f32::consts::PI), -1.0) } else { (diff, 1.0) };
    let steer = (err * 3.0).clamp(-1.0, 1.0);
    let go = if err.abs() < 1.2 { err.cos().max(0.0) } else { 0.0 };
    (steer, sign * mag * go.max(0.1))
}

/// "Aim + drive button" controls: the hull turns to the aim direction and the
/// DRIVE button moves it forward along that direction; REV backs straight up.
fn aim_drive_move(aim: f32, hull: f32, drive: bool, rev: bool) -> (f32, f32) {
    let diff = wrap(aim - hull);
    if drive {
        ((diff * 3.0).clamp(-1.0, 1.0), if diff.abs() < 1.3 { diff.cos().max(0.25) } else { 0.0 })
    } else if rev {
        (0.0, -1.0)
    } else {
        (0.0, 0.0)
    }
}

/// Twin-stick touch input: a floating stick on the left drives (push toward
/// where you want to go, relative to the camera), dragging the right side
/// aims, and FIRE / BOOST / R are held buttons. Returns
/// (steer, throttle, fire, boost, reload).
fn touch_input(app: &mut App) -> (f32, f32, bool, bool, bool) {
    let sens = app.settings.sens;
    let hull = app.world.tank(app.my_id).map(|t| t.angle).unwrap_or(0.0);
    let ts = touches();
    let live = |id: u64| ts.iter().any(|t| t.id == id && !matches!(t.phase, TouchPhase::Ended | TouchPhase::Cancelled));
    if let Some((id, _)) = app.touch.stick {
        if !live(id) {
            app.touch.stick = None;
        }
    }
    if let Some((id, _)) = app.touch.look {
        if !live(id) {
            if app.settings.tap_fire && app.touch.look_moved < 18.0 && get_time() as f32 - app.touch.look_t0 < 0.3 {
                app.touch.tap_pending = true;
            }
            app.touch.look = None;
        }
    }
    let (mut fire, mut boost, mut reload) = (false, false, false);
    if std::mem::take(&mut app.touch.tap_pending) {
        fire = true;
    }
    let (mut drive_btn, mut rev_btn) = (false, false);
    let aim_drive = app.settings.aim_drive;
    let mut stick_delta: Option<Vec2> = None;
    for t in &ts {
        if matches!(t.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            continue;
        }
        let v = app.ui.to_virtual(t.position);
        if let Some((id, origin)) = app.touch.stick {
            if id == t.id {
                stick_delta = Some(v - origin);
                continue;
            }
        }
        if let Some((id, last)) = app.touch.look {
            if id == t.id {
                if app.settings.aim_stick {
                    // right stick: sideways deflection turns the camera/turret at a steady rate
                    let d = v - app.touch.look_origin;
                    app.touch.look_knob = d;
                    let m = (d.x / STICK_R).clamp(-1.0, 1.0);
                    let m = if m.abs() < 0.12 { 0.0 } else { m };
                    app.aim += m * 2.8 * sens * get_frame_time();
                } else {
                    app.aim += (v.x - last.x) * 0.006 * sens;
                }
                app.touch.look_moved = app.touch.look_moved.max((v - app.touch.look_origin).length());
                app.touch.look = Some((id, v));
                continue;
            }
        }
        let inside = |b: (f32, f32, f32)| (v.x - b.0).hypot(v.y - b.1) <= b.2;
        if inside(FIRE_BTN) {
            fire = true;
        } else if inside(BOOST_BTN) {
            boost = true;
        } else if inside(RELOAD_BTN) {
            reload = true;
        } else if app.world.ctf.is_some() && inside(ITEM_BTN) {
            if t.phase == TouchPhase::Started {
                app.touch.item_pressed = true;
            }
        } else if aim_drive && inside(DRIVE_BTN) {
            drive_btn = true;
        } else if aim_drive && inside(REV_BTN) {
            rev_btn = true;
        } else if t.phase == TouchPhase::Started {
            if !aim_drive && v.x < 560.0 && v.y > 160.0 && app.touch.stick.is_none() {
                app.touch.stick = Some((t.id, v));
                stick_delta = Some(Vec2::ZERO);
            } else if v.x >= 560.0 && v.y > 90.0 && app.touch.look.is_none() {
                app.touch.look = Some((t.id, v));
                app.touch.look_origin = v;
                app.touch.look_t0 = get_time() as f32;
                app.touch.look_moved = 0.0;
                app.touch.look_knob = Vec2::ZERO;
            }
        }
    }
    if app.touch.look.is_none() {
        app.touch.look_knob = Vec2::ZERO;
    }
    app.touch.flags = (fire, boost, reload);
    app.touch.drive_flags = (drive_btn, rev_btn);
    let (mut steer, mut throttle) = (0.0, 0.0);
    if aim_drive {
        // the tank always heads where the camera aims; DRIVE moves it forward
        let (st, th) = aim_drive_move(app.aim, hull, drive_btn, rev_btn);
        steer = st;
        throttle = th;
        return (steer, throttle, fire, boost, reload);
    }
    if let Some(d) = stick_delta {
        app.touch.knob = d;
        let m = (d.length() / STICK_R).min(1.0);
        if m > 0.18 {
            let r = screen_drive(app.aim, hull, d.x, -d.y, m);
            steer = r.0;
            throttle = r.1;
        }
    } else {
        app.touch.knob = Vec2::ZERO;
    }
    (steer, throttle, fire, boost, reload)
}

fn draw_touch_controls(app: &App) {
    let u = app.ui.u;
    let at = |x: f32, y: f32| {
        let r = app.ui.r(x, y, 0.0, 0.0);
        (r.x, r.y)
    };
    let (fire, boost, reload) = app.touch.flags;
    for (b, label, on, col) in [
        (FIRE_BTN, "FIRE", fire, Color::new(1.0, 0.35, 0.3, 1.0)),
        (BOOST_BTN, "BOOST", boost, Color::new(1.0, 0.75, 0.2, 1.0)),
        (RELOAD_BTN, "R", reload, Color::new(0.5, 0.8, 1.0, 1.0)),
    ] {
        let (x, y) = at(b.0, b.1);
        draw_circle(x, y, b.2 * u, Color::new(col.r, col.g, col.b, if on { 0.55 } else { 0.22 }));
        draw_circle_lines(x, y, b.2 * u, 3.0 * u, Color::new(col.r, col.g, col.b, 0.9));
        app.ui.text(label, b.0, b.1, if b.2 > 80.0 { 30.0 } else { 24.0 }, TEXT, Align::Center);
    }
    if let Some(me) = app.world.tank(app.my_id).filter(|_| app.world.ctf.is_some()) {
        let col = if me.item == 0 { Color::new(0.6, 0.6, 0.65, 1.0) } else { let c = crate::ctf::item_rgb(me.item); Color::new(c.0, c.1, c.2, 1.0) };
        let (x, y) = at(ITEM_BTN.0, ITEM_BTN.1);
        draw_circle(x, y, ITEM_BTN.2 * u, Color::new(col.r, col.g, col.b, if me.item == 0 { 0.12 } else { 0.4 }));
        draw_circle_lines(x, y, ITEM_BTN.2 * u, 3.0 * u, Color::new(col.r, col.g, col.b, 0.9));
        let label = if me.item == 0 { "ITEM" } else { crate::ctf::item_label(me.item) };
        app.ui.text(label, ITEM_BTN.0, ITEM_BTN.1, 20.0, TEXT, Align::Center);
    }
    if app.settings.aim_drive {
        let (d, r) = app.touch.drive_flags;
        for (b, label, on, col) in [
            (DRIVE_BTN, "DRIVE", d, Color::new(0.4, 0.9, 0.5, 1.0)),
            (REV_BTN, "REV", r, Color::new(0.8, 0.8, 0.9, 1.0)),
        ] {
            let (x, y) = at(b.0, b.1);
            draw_circle(x, y, b.2 * u, Color::new(col.r, col.g, col.b, if on { 0.55 } else { 0.22 }));
            draw_circle_lines(x, y, b.2 * u, 3.0 * u, Color::new(col.r, col.g, col.b, 0.9));
            app.ui.text(label, b.0, b.1, if b.2 > 80.0 { 32.0 } else { 22.0 }, TEXT, Align::Center);
        }
    }
    // floating stick: base where the thumb landed, or a hint at rest
    let (ox, oy, active) = match app.touch.stick {
        Some((_, o)) => (o.x, o.y, true),
        None => (190.0, 540.0, false),
    };
    if !app.settings.aim_drive {
        let (x, y) = at(ox, oy);
        draw_circle(x, y, STICK_R * u, Color::new(1.0, 1.0, 1.0, if active { 0.14 } else { 0.07 }));
        draw_circle_lines(x, y, STICK_R * u, 3.0 * u, Color::new(1.0, 1.0, 1.0, if active { 0.6 } else { 0.3 }));
        let k = if active { app.touch.knob.clamp_length_max(STICK_R) } else { Vec2::ZERO };
        let (kx, ky) = at(ox + k.x, oy + k.y);
        draw_circle(kx, ky, 38.0 * u, Color::new(1.0, 1.0, 1.0, if active { 0.5 } else { 0.2 }));
    }
    if app.settings.aim_stick {
        // right aim stick: base where the thumb landed, or a hint at rest
        let (ox, oy, active) = match app.touch.look {
            Some(_) => (app.touch.look_origin.x, app.touch.look_origin.y, true),
            None => (800.0, 540.0, false),
        };
        let (x, y) = at(ox, oy);
        let tint = Color::new(0.6, 0.85, 1.0, 1.0);
        draw_circle(x, y, STICK_R * u, Color::new(tint.r, tint.g, tint.b, if active { 0.14 } else { 0.07 }));
        draw_circle_lines(x, y, STICK_R * u, 3.0 * u, Color::new(tint.r, tint.g, tint.b, if active { 0.6 } else { 0.3 }));
        let k = if active { app.touch.look_knob.clamp_length_max(STICK_R) } else { Vec2::ZERO };
        let (kx, ky) = at(ox + k.x, oy + k.y);
        draw_circle(kx, ky, 38.0 * u, Color::new(tint.r, tint.g, tint.b, if active { 0.5 } else { 0.2 }));
        if !active {
            app.ui.text("AIM", ox, oy, 22.0, DIM, Align::Center);
        }
    }
}

fn run_play(app: &mut App, dt: f32, hold: &mut HoldTracker, forced: bool, frame: u64) {
    app.play_time += dt;
    let mut quit_pressed = false;
    let mut cam_toggle = false;
    let over = app.world.phase == Phase::GameOver;
    let drive = app.cam.chase;

    if !app.touch_mode && !touches().is_empty() {
        app.touch_mode = true;
    }

    if app.test_pause == Some(frame) {
        app.paused = true;
    }
    // --- pause / camera keys ---
    if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::P) {
        if over {
            quit_pressed = true;
        } else {
            app.paused = !app.paused;
        }
    }
    if is_key_pressed(KeyCode::C) && !app.paused {
        cam_toggle = true;
    }

    // --- mouse capture: only while actually driving with mouse + keyboard ---
    let want_grab = drive && !app.touch_mode && !app.paused && !over && cfg!(not(target_os = "android"));
    if want_grab != app.grabbed {
        set_cursor_grab(want_grab);
        show_mouse(!want_grab);
        app.grabbed = want_grab;
        app.fire_armed = false;
        app.last_mouse = Vec2::from(mouse_position());
    }
    if app.grabbed {
        // the invisible cursor must never "click" HUD buttons
        app.ui.click = None;
    }

    // --- input ---
    let quit_r = app.ui.r(16.0, 14.0, 130.0, 50.0);
    let cam_r = app.ui.r(VW - 276.0, 14.0, 260.0, 50.0);
    let ctf = app.world.ctf.is_some();
    let item_btn_shown = ctf && !(drive && app.touch_mode);
    let item_r = app.ui.r(VW - 216.0, 72.0, 200.0, 50.0);
    let item_click = item_btn_shown && app.ui.click.map_or(false, |c| item_r.contains(c));
    if let Some(c) = app.ui.click {
        hold.active = !quit_r.contains(c) && !cam_r.contains(c) && !(item_btn_shown && item_r.contains(c)) && !over && !app.paused;
    }
    if !app.ui.down {
        hold.active = false;
    }
    let held = !app.paused && ((app.ui.down && hold.active) || is_key_down(KeyCode::Space) || forced);
    let mut ctrl = if app.paused || over {
        Control { drive, aim: app.aim, ..Default::default() }
    } else if drive {
        gather_drive(app, dt, frame)
    } else {
        Control::classic(held)
    };
    // use the held mystery item: V, the ITEM button, or the touch ITEM circle
    if ctf && !app.paused && !over {
        if is_key_pressed(KeyCode::V) || item_click || std::mem::take(&mut app.touch.item_pressed) {
            app.item_hold = 0.25;
        } else {
            app.item_hold = (app.item_hold - dt).max(0.0);
        }
        ctrl.use_item = app.item_hold > 0.0;
    }

    // --- simulation per mode ---
    let mut back_to_lobby_client = false;
    let mut client_msg_leave: Option<String> = None;
    let my_id = app.my_id;
    match &mut app.mode {
        Mode::Offline => {
            if let Some(t) = app.world.tank_mut(0) {
                t.ctrl = ctrl;
                t.held = ctrl.held;
            }
            if !app.paused {
                app.world.step(dt);
            }
            let evs = std::mem::take(&mut app.world.events);
            for e in &evs {
                spawn_fx(&mut app.particles, e);
                app.audio.play_event(e);
            }
        }
        Mode::Host(h) => {
            let gone = h.poll();
            for id in gone {
                app.world.kill_player(id);
            }
            for t in app.world.tanks.iter_mut() {
                if t.ai.is_some() {
                    continue; // bots steer themselves
                }
                let c = if t.id == 0 { ctrl } else { h.input(t.id) };
                t.ctrl = c;
                t.held = c.held;
            }
            app.world.step(dt);
            let evs = std::mem::take(&mut app.world.events);
            for e in &evs {
                spawn_fx(&mut app.particles, e);
                app.audio.play_event(e);
            }
            app.pending_events.extend(evs);
            let over_now = app.world.phase == Phase::GameOver;
            let force = over_now && !app.final_sent;
            if h.send_state(&app.world, &app.pending_events, force) {
                app.pending_events.clear();
                if force {
                    app.final_sent = true;
                }
            }
        }
        Mode::Client(c) => {
            let mut evs_all = Vec::new();
            for m in c.poll(ctrl) {
                match m {
                    ClientMsg::State(s) => {
                        let prev: HashMap<u32, (f32, f32, f32, f32)> =
                            app.world.tanks.iter().map(|t| (t.id, (t.x, t.y, t.angle, t.turret))).collect();
                        if let Some(evs) = app.world.apply_state(&s) {
                            evs_all.extend(evs);
                            for t in app.world.tanks.iter_mut() {
                                app.targets.insert(t.id, (t.x, t.y, t.angle, t.turret));
                                if let Some(&(px, py, pa, pt)) = prev.get(&t.id) {
                                    if (px - t.x).hypot(py - t.y) < 150.0 {
                                        t.x = px;
                                        t.y = py;
                                        t.angle = pa;
                                        t.turret = pt;
                                    }
                                }
                            }
                        }
                    }
                    ClientMsg::Lobby { match_type, ctf, arena, roster } => {
                        app.lobby_match_type = match_type;
                        app.lobby_ctf = ctf;
                        app.lobby_arena = arena;
                        app.roster = roster;
                        back_to_lobby_client = true;
                    }
                    ClientMsg::Reject(r) => client_msg_leave = Some(r),
                    _ => {}
                }
            }
            // smooth tanks toward the latest snapshot; coast bullets
            let k = 1.0 - (-dt * 18.0).exp();
            for t in app.world.tanks.iter_mut() {
                if let Some(&(tx, ty, ta, tt)) = app.targets.get(&t.id) {
                    t.x += (tx - t.x) * k;
                    t.y += (ty - t.y) * k;
                    t.angle += wrap(ta - t.angle) * k;
                    t.turret += wrap(tt - t.turret) * k;
                }
            }
            for b in app.world.bullets.iter_mut() {
                b.x += b.vx * dt;
                b.y += b.vy * dt;
            }
            for e in &evs_all {
                spawn_fx(&mut app.particles, e);
                app.audio.play_event(e);
            }
            if c.host_timed_out() {
                client_msg_leave = Some("Lost connection to the host".into());
            }
        }
    }
    update_particles(&mut app.particles, dt);

    // --- 3D scene ---
    let me = app.world.tank(my_id).filter(|t| t.alive);
    let me_alive = me.is_some();
    // where the next shot will go (drive view only)
    let aim_line = if drive && !over { me.map(|t| app.world.trace_shot(t.x, t.y, if app.cam.chase { app.aim } else { t.turret }, t.team)) } else { None };
    if drive {
        app.cam.yaw = app.aim;
    }
    let cam = app.cam.camera(&app.world, my_id, dt);
    set_camera(&cam);
    app.tracks.update(&app.world);
    app.scene.draw_world(&app.world, app.play_time, &app.tracks, aim_line.as_ref(), if drive { Some(my_id) } else { None }, cam.position);
    draw_particles(&app.particles);
    set_default_camera();

    // --- labels over tanks ---
    for t in &app.world.tanks {
        let Some(p) = project(&cam, w3(t.x, t.y, 1.45)) else { continue };
        let mine = t.id == my_id;
        let col = if !t.alive { Color::new(0.5, 0.5, 0.5, 0.8) } else if t.team == 0 { TEAM_BLUE } else { TEAM_RED };
        let label = if mine { format!("> {} <", t.name) } else { t.name.clone() };
        let v = app.ui.to_virtual(p);
        app.ui.text(&label, v.x, v.y, 24.0, col, Align::Center);
    }
    for p in &app.world.pickups {
        if let Some(sp) = project(&cam, w3(p.x, p.y, 1.0)) {
            let v = app.ui.to_virtual(sp);
            let (r, g, b) = p.kind.rgb();
            app.ui.text(p.kind.label(), v.x, v.y, 24.0, Color::new(r, g, b, 1.0), Align::Center);
        }
    }

    // crosshair where the cannon points
    if drive && me_alive && !over {
        if let Some(t) = app.world.tank(my_id) {
            let p3 = w3(t.x, t.y, 0.6) + vec3(t.turret.cos(), 0.0, t.turret.sin()) * 6.0;
            if let Some(sp) = project(&cam, p3) {
                let u = app.ui.u;
                let hit = aim_line.as_ref().map_or(false, |a| a.1);
                let col = if hit { Color::new(1.0, 0.3, 0.25, 1.0) } else { Color::new(1.0, 1.0, 1.0, 0.9) };
                let (g, l) = (7.0 * u, 15.0 * u);
                for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                    draw_line(sp.x + dx * g, sp.y + dy * g, sp.x + dx * (g + l), sp.y + dy * (g + l), 3.0 * u, col);
                }
                draw_circle(sp.x, sp.y, 2.5 * u, col);
            }
        }
    }

    draw_hud(app, me_alive);
    if drive && app.touch_mode && !over && !app.paused {
        draw_touch_controls(app);
    }
    if app.ui.button(16.0, 14.0, 130.0, 50.0, "QUIT", Style::Danger) {
        quit_pressed = true;
    }
    if app.ui.button(VW - 276.0, 14.0, 260.0, 50.0, if app.cam.chase { "VIEW: CHASE" } else { "VIEW: MAP" }, Style::Normal) {
        cam_toggle = true;
    }

    // --- pause overlay ---
    if app.paused && !over {
        app.ui.fill(0.0, 0.0, VW, VH, Color::new(0.0, 0.0, 0.0, 0.5));
        app.ui.panel(340.0, 130.0, 600.0, 470.0);
        app.ui.text("PAUSED", VW / 2.0, 185.0, 64.0, ACCENT, Align::Center);
        if !matches!(app.mode, Mode::Offline) {
            app.ui.text("The match keeps running", VW / 2.0, 232.0, 22.0, DIM, Align::Center);
        }
        if app.ui.button(420.0, 262.0, 440.0, 60.0, "RESUME", Style::Primary) {
            app.paused = false;
        }
        if app.ui.button(420.0, 336.0, 440.0, 60.0, "QUIT TO MENU", Style::Danger) {
            quit_pressed = true;
        }
        let keys = ["WASD / ARROWS  drive", "MOUSE or Q E  aim", "CLICK or SPACE  fire", "SHIFT/X boost   R reload", "C switch view   ESC pause"];
        for (i, k) in keys.iter().enumerate() {
            app.ui.text_fit(k, VW / 2.0, 435.0 + i as f32 * 30.0, 22.0, DIM, Align::Center, 540.0);
        }
    }

    // --- game over ---
    let mut next_action = 0; // 1 = rematch/play again, 2 = menu
    let ctf_end = over && app.world.ctf.is_some();
    if over && !ctf_end {
        app.ui.panel(380.0, 190.0, 520.0, 300.0);
        let my_team = app.world.tank(my_id).map(|t| t.team);
        let (msg, col) = match (app.world.winner, my_team) {
            (None, _) => ("DRAW".to_string(), ACCENT),
            (Some(w), Some(mt)) if w == mt => ("VICTORY!".to_string(), Color::new(0.4, 1.0, 0.5, 1.0)),
            (Some(w), None) => (format!("{} TEAM WINS", if w == 0 { "BLUE" } else { "RED" }), ACCENT),
            (Some(_), _) => ("DEFEAT".to_string(), TEAM_RED),
        };
        app.ui.text(&msg, VW / 2.0, 250.0, if msg.len() > 10 { 56.0 } else { 80.0 }, col, Align::Center);
        if let Some(w) = app.world.winner {
            app.ui.text(&format!("{} team wins the round", if w == 0 { "Blue" } else { "Red" }), VW / 2.0, 305.0, 22.0, DIM, Align::Center);
        }
        match &app.mode {
            Mode::Client(_) => {
                app.ui.text("Waiting for host rematch...", VW / 2.0, 360.0, 20.0, ACCENT, Align::Center);
            }
            Mode::Host(_) => {
                if app.ui.button(420.0, 345.0, 440.0, 60.0, "REMATCH (LOBBY)", Style::Primary) {
                    next_action = 1;
                }
            }
            Mode::Offline => {
                if app.ui.button(420.0, 345.0, 440.0, 60.0, "PLAY AGAIN", Style::Primary) {
                    next_action = 1;
                }
            }
        }
    }
    if over && !ctf_end && app.ui.button(420.0, 420.0, 440.0, 50.0, "MAIN MENU", Style::Normal) {
        next_action = 2;
    }
    if ctf_end {
        next_action = draw_ctf_end(app, my_id);
    }

    // --- transitions ---
    if cam_toggle {
        app.cam.chase = !app.cam.chase;
        app.settings.chase_cam = app.cam.chase;
        app.settings.save();
        app.aim = app.world.tank(my_id).map(|t| t.turret).unwrap_or(app.aim);
    }
    if let Some(r) = client_msg_leave {
        app.leave_to_menu();
        app.note(&r);
        return;
    }
    if back_to_lobby_client {
        app.screen = Screen::ClientLobby;
        app.particles.clear();
        return;
    }
    if quit_pressed || next_action == 2 {
        app.leave_to_menu();
        return;
    }
    if next_action == 1 {
        match &mut app.mode {
            Mode::Offline => {
                if app.world.ctf.is_some() {
                    app.start_ctf_practice();
                } else {
                    app.start_practice();
                }
            }
            Mode::Host(h) => {
                h.return_to_lobby();
                app.screen = Screen::HostLobby;
                app.particles.clear();
            }
            Mode::Client(_) => {}
        }
    }
}

/// The big reveal at the end of a capture-the-flag tournament: final score,
/// winner and everybody's numbers. Returns 1 for rematch, 2 for menu, else 0.
fn draw_ctf_end(app: &mut App, my_id: u32) -> i32 {
    let mut action = 0;
    let Some(c) = app.world.ctf.clone() else { return 0 };
    app.ui.panel(170.0, 40.0, 940.0, 640.0);
    let my_team = app.world.tank(my_id).map(|t| t.team);
    let (msg, col) = match (app.world.winner, my_team) {
        (None, _) => ("DRAW".to_string(), ACCENT),
        (Some(w), Some(mt)) if w == mt => ("VICTORY!".to_string(), Color::new(0.4, 1.0, 0.5, 1.0)),
        (Some(w), None) => (format!("{} WINS", if w == 0 { "BLUE" } else { "RED" }), ACCENT),
        (Some(_), _) => ("DEFEAT".to_string(), TEAM_RED),
    };
    app.ui.text(&msg, VW / 2.0, 100.0, 60.0, col, Align::Center);
    app.ui.text("FINAL SCORE", VW / 2.0, 150.0, 18.0, DIM, Align::Center);
    app.ui.text(&format!("{}", c.scores[0]), VW / 2.0 - 90.0, 200.0, 60.0, TEAM_BLUE, Align::Center);
    app.ui.text("-", VW / 2.0, 200.0, 48.0, TEXT, Align::Center);
    app.ui.text(&format!("{}", c.scores[1]), VW / 2.0 + 90.0, 200.0, 60.0, TEAM_RED, Align::Center);
    if app.world.winner.is_some() && c.scores[0] == c.scores[1] {
        app.ui.text("won on kills", VW / 2.0, 240.0, 18.0, DIM, Align::Center);
    }

    // MVP: most captures, then kills and returns
    let points = |id: u32| {
        let s = c.stat(id);
        s.caps * 5 + s.kills + s.returns * 2
    };
    let mvp = app.world.tanks.iter().map(|t| t.id).max_by_key(|&id| (points(id), std::cmp::Reverse(id)));
    let cols = [(780.0, "KILLS"), (870.0, "DEATHS"), (960.0, "FLAGS"), (1050.0, "SAVES")];
    let mut y = 270.0;
    for team in 0..2u8 {
        let (name, tc) = if team == 0 { ("BLUE TEAM", TEAM_BLUE) } else { ("RED TEAM", TEAM_RED) };
        app.ui.text(name, 210.0, y, 22.0, tc, Align::Left);
        if team == 0 {
            for (x, h) in cols {
                app.ui.text(h, x, y, 15.0, DIM, Align::Right);
            }
        }
        y += 28.0;
        let mut members: Vec<&Tank> = app.world.tanks.iter().filter(|t| t.team == team).collect();
        members.sort_by_key(|t| std::cmp::Reverse(points(t.id)));
        for t in members {
            let st = c.stat(t.id);
            let you = if t.id == my_id { " (you)" } else { "" };
            let tag = if Some(t.id) == mvp && points(t.id) > 0 { "  MVP" } else { "" };
            let rc = if Some(t.id) == mvp && points(t.id) > 0 { ACCENT } else { TEXT };
            app.ui.text_fit(&format!("{}{}{}", t.name, you, tag), 230.0, y, 20.0, rc, Align::Left, 440.0);
            for (x, v) in [(780.0, st.kills), (870.0, st.deaths), (960.0, st.caps), (1050.0, st.returns)] {
                app.ui.text(&v.to_string(), x, y, 20.0, rc, Align::Right);
            }
            y += 26.0;
        }
        y += 12.0;
    }
    let by = 612.0;
    match &app.mode {
        Mode::Client(_) => app.ui.text("Waiting for host rematch...", 440.0, by + 27.0, 20.0, ACCENT, Align::Center),
        Mode::Host(_) => {
            if app.ui.button(210.0, by, 460.0, 54.0, "REMATCH (LOBBY)", Style::Primary) {
                action = 1;
            }
        }
        Mode::Offline => {
            if app.ui.button(210.0, by, 460.0, 54.0, "PLAY AGAIN", Style::Primary) {
                action = 1;
            }
        }
    }
    if app.ui.button(700.0, by, 370.0, 54.0, "MAIN MENU", Style::Normal) {
        action = 2;
    }
    action
}

fn draw_hud(app: &mut App, me_alive: bool) {
    let w = &app.world;
    let ui = &app.ui;
    let drive = app.cam.chase;
    // team status, top centre
    let alive = |team: u8| w.tanks.iter().filter(|t| t.team == team && t.alive).count();
    let total = |team: u8| w.tanks.iter().filter(|t| t.team == team).count();
    if let Some(c) = w.ctf.as_ref() {
        // tournament clock and flag status; the score stays secret until the end
        let secs = c.time_left.ceil() as i32;
        ui.fill(VW / 2.0 - 230.0, 14.0, 460.0, 74.0, Color::new(0.0, 0.0, 0.0, 0.5));
        let low = secs <= 30;
        ui.text(&format!("{}:{:02}", secs / 60, secs % 60), VW / 2.0, 40.0, 36.0, if low { Color::new(1.0, 0.45, 0.35, 1.0) } else { TEXT }, Align::Center);
        let state = |team: usize| {
            let f = &c.flags[team];
            if f.carrier.is_some() {
                "TAKEN"
            } else if f.at_home() {
                "SAFE"
            } else {
                "DROPPED"
            }
        };
        ui.text(&format!("BLUE FLAG {}", state(0)), VW / 2.0 - 220.0, 72.0, 17.0, TEAM_BLUE, Align::Left);
        ui.text(&format!("RED FLAG {}", state(1)), VW / 2.0 + 220.0, 72.0, 17.0, TEAM_RED, Align::Right);
        ui.text("SCORES REVEALED AT THE END", VW / 2.0, 100.0, 15.0, DIM, Align::Center);
    } else {
        ui.fill(VW / 2.0 - 170.0, 14.0, 340.0, 46.0, Color::new(0.0, 0.0, 0.0, 0.45));
        ui.text(&format!("BLUE {}/{}", alive(0), total(0)), VW / 2.0 - 160.0, 37.0, 24.0, TEAM_BLUE, Align::Left);
        ui.text(&format!("{}/{} RED", alive(1), total(1)), VW / 2.0 + 160.0, 37.0, 24.0, TEAM_RED, Align::Right);
    }

    if let Some(t) = w.tank(app.my_id) {
        if t.alive {
            // bottom-left status plate (moves up on touch so the stick has room)
            let py = if drive && app.touch_mode { 20.0 + 70.0 } else { 600.0 };
            let (px, ph) = if drive { (16.0, 112.0) } else { (16.0, 104.0) };
            ui.fill(px, py, 430.0, ph, Color::new(0.0, 0.0, 0.0, 0.5));
            for i in 0..t.max_hp() {
                let c = if i < t.hp { Color::new(0.4, 1.0, 0.45, 1.0) } else { Color::new(0.25, 0.3, 0.25, 1.0) };
                ui.fill(30.0 + i as f32 * 40.0, py + 14.0, 32.0, 24.0, c);
            }
            if t.reloading() {
                let f = 1.0 - t.reload / t.reload_time();
                ui.fill(30.0, py + 52.0, 220.0, 20.0, Color::new(0.2, 0.2, 0.2, 1.0));
                ui.fill(30.0, py + 52.0, 220.0 * f, 20.0, Color::new(1.0, 0.7, 0.2, 1.0));
                ui.text("RELOADING", 262.0, py + 62.0, 20.0, ACCENT, Align::Left);
            } else {
                for i in 0..t.max_ammo() {
                    let c = if i < t.ammo { Color::new(1.0, 0.85, 0.3, 1.0) } else { Color::new(0.3, 0.3, 0.3, 1.0) };
                    ui.fill(30.0 + i as f32 * 36.0, py + 52.0, 26.0, 20.0, c);
                }
            }
            if drive {
                let locked = t.stamina < 0.3;
                let col = if locked { Color::new(0.9, 0.35, 0.3, 1.0) } else { Color::new(0.4, 0.8, 1.0, 1.0) };
                ui.fill(30.0, py + 86.0, 220.0, 12.0, Color::new(0.2, 0.2, 0.2, 1.0));
                ui.fill(30.0, py + 86.0, 220.0 * t.stamina.clamp(0.0, 1.0), 12.0, col);
                ui.text("BOOST", 262.0, py + 92.0, 18.0, DIM, Align::Left);
            }
            let mut buffs = Vec::new();
            if t.speed_t > 0.0 {
                buffs.push((format!("NITRO {:.0}", t.speed_t.ceil()), PickupKind::Speed));
            }
            if t.rapid_t > 0.0 {
                buffs.push((format!("RAPID {:.0}", t.rapid_t.ceil()), PickupKind::Rapid));
            }
            if t.shield_t > 0.0 {
                buffs.push((format!("SHIELD {:.0}", t.shield_t.ceil()), PickupKind::Shield));
            }
            if w.ctf.is_some() {
                let (label, col) = if t.item == 0 {
                    ("ITEM: NONE".to_string(), DIM)
                } else {
                    let c = crate::ctf::item_rgb(t.item);
                    (format!("ITEM: {}{}", crate::ctf::item_label(t.item), if app.touch_mode { "" } else { "  [V]" }), Color::new(c.0, c.1, c.2, 1.0))
                };
                ui.text(&label, 460.0, py + 20.0 + buffs.len() as f32 * 26.0, 20.0, col, Align::Left);
                if t.spread_t > 0.0 {
                    ui.text(&format!("SPREAD {:.0}", t.spread_t.ceil()), 460.0, py + 20.0 + (buffs.len() as f32 + 1.0) * 26.0, 20.0, Color::new(0.95, 0.85, 0.3, 1.0), Align::Left);
                }
            }
            for (i, (label, kind)) in buffs.iter().enumerate() {
                let (r, g, b) = kind.rgb();
                ui.text(label, 460.0, py + 20.0 + i as f32 * 26.0, 20.0, Color::new(r, g, b, 1.0), Align::Left);
            }
        }
    }
    if !me_alive && w.phase == Phase::Playing && w.tank(app.my_id).is_some() {
        ui.fill(VW / 2.0 - 300.0, 120.0, 600.0, 56.0, Color::new(0.5, 0.1, 0.1, 0.8));
        if w.ctf.is_some() {
            let r = w.tank(app.my_id).map(|t| t.respawn_t).unwrap_or(0.0).ceil().max(1.0);
            ui.text(&format!("DESTROYED - back in {}", r as i32), VW / 2.0, 148.0, 28.0, TEXT, Align::Center);
        } else {
            ui.text("ELIMINATED - spectating", VW / 2.0, 148.0, 28.0, TEXT, Align::Center);
        }
    }
    if let (Some(c), true) = (w.ctf.as_ref(), me_alive) {
        if let Some(me) = w.tank(app.my_id) {
            let mine = me.team as usize;
            let msg = if c.flags[1 - mine].carrier == Some(me.id) {
                if c.flags[mine].at_home() { Some(("YOU HAVE THE FLAG - GET HOME!", Color::new(1.0, 0.9, 0.3, 1.0))) } else { Some(("YOUR FLAG IS TAKEN - GET IT BACK FIRST!", Color::new(1.0, 0.55, 0.3, 1.0))) }
            } else if c.flags[mine].carrier.is_some() {
                Some(("ENEMY HAS YOUR FLAG - HUNT THE CARRIER!", Color::new(1.0, 0.45, 0.4, 1.0)))
            } else if !c.flags[mine].at_home() {
                Some(("YOUR FLAG IS DOWN - TOUCH IT TO RETURN", Color::new(1.0, 0.8, 0.4, 1.0)))
            } else {
                None
            };
            if let Some((m, col)) = msg {
                ui.fill(VW / 2.0 - 340.0, 124.0, 680.0, 40.0, Color::new(0.0, 0.0, 0.0, 0.55));
                ui.text_fit(m, VW / 2.0, 144.0, 22.0, col, Align::Center, 660.0);
            }
        }
    }
    if w.ctf.is_some() && !(drive && app.touch_mode) && me_alive {
        // desktop / map view: a clickable ITEM button
        let held = w.tank(app.my_id).map(|t| t.item).unwrap_or(0);
        let label = if held == 0 { "NO ITEM".to_string() } else { format!("USE {} [V]", crate::ctf::item_label(held)) };
        ui.fill(VW - 216.0, 72.0, 200.0, 50.0, Color::new(0.0, 0.0, 0.0, if held == 0 { 0.35 } else { 0.6 }));
        let c = crate::ctf::item_rgb(held);
        ui.text_fit(&label, VW - 116.0, 98.0, 20.0, if held == 0 { DIM } else { Color::new(c.0, c.1, c.2, 1.0) }, Align::Center, 190.0);
    }
    if app.play_time < 10.0 && me_alive && !app.paused && w.ctf.is_some() {
        ui.text_fit("GRAB THE ENEMY FLAG AND BRING IT HOME   V = USE ITEM", VW / 2.0, 190.0, 24.0, Color::new(1.0, 1.0, 1.0, (10.0 - app.play_time).min(1.0)), Align::Center, 1200.0);
    }
    if app.play_time < 8.0 && me_alive && !app.paused {
        let hint = match (drive, app.touch_mode) {
            (false, _) => "TAP = FIRE     HOLD = DRIVE",
            (true, false) => "WASD DRIVE  MOUSE AIM  CLICK FIRE  SHIFT BOOST",
            (true, true) if app.settings.aim_drive => "AIM WITH THE RIGHT SIDE   HOLD DRIVE TO MOVE",
            (true, true) if app.settings.aim_stick => "LEFT STICK DRIVE   RIGHT STICK AIM",
            (true, true) => "LEFT STICK DRIVE   DRAG RIGHT TO AIM",
        };
        ui.text_fit(hint, VW / 2.0, if drive && app.touch_mode { 250.0 } else if w.ctf.is_some() { 150.0 } else { 120.0 }, 28.0, Color::new(1.0, 1.0, 1.0, (8.0 - app.play_time).min(1.0)), Align::Center, 1200.0);
    }
}

#[cfg(test)]
mod drive_tests {
    use super::*;
    #[test]
    fn aim_drive_buttons() {
        // aiming straight ahead: DRIVE goes full speed with no steering
        let (s, t) = aim_drive_move(0.0, 0.0, true, false);
        assert!(s.abs() < 1e-4 && t > 0.99);
        // aim to the right of the hull: turn right and still creep forward
        let (s, t) = aim_drive_move(0.8, 0.0, true, false);
        assert!(s > 0.9 && t > 0.2 && t < 0.8);
        // aim behind the hull: turn in place, no forward motion
        let (_, t) = aim_drive_move(3.0, 0.0, true, false);
        assert_eq!(t, 0.0);
        // not holding DRIVE: stand still while the camera turns
        assert_eq!(aim_drive_move(1.0, 0.0, false, false), (0.0, 0.0));
        assert_eq!(aim_drive_move(1.0, 0.0, false, true), (0.0, -1.0));
    }

    #[test]
    fn screen_relative() {
        // facing the camera's forward: W drives forward, no steering
        let (s, t) = screen_drive(0.0, 0.0, 0.0, 1.0, 1.0);
        assert!(s.abs() < 1e-3 && t > 0.9);
        // D (screen right) while facing forward: steer right (positive angle), still moving
        let (s, t) = screen_drive(0.0, 0.0, 1.0, 0.0, 1.0);
        assert!(s > 0.5 && t > 0.0 && t < 0.5);
        // A: steer left
        assert!(screen_drive(0.0, 0.0, -1.0, 0.0, 1.0).0 < -0.5);
        // S while facing forward: reverse straight back, no spin
        let (s, t) = screen_drive(0.0, 0.0, 0.0, -1.0, 1.0);
        assert!(s.abs() < 1e-3 && t < -0.9);
    }
}

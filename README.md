# Tank Arena 3D

A Rust + [macroquad](https://macroquad.rs) 3D port of the LÖVE2D Tank Arena.
Runs on Linux, Windows and Android with the same code.

**Controls**

Chase view (default) - "drive" scheme:
| Action | Keyboard / mouse | Touch |
|---|---|---|
| Drive | W/S forward/back, A/D turn (or arrows) | left stick |
| Aim turret + camera | mouse, or Q/E | drag right side |
| Fire | left click / Space (auto-fire, 5-round magazine) | FIRE button |
| Boost | Shift (stamina bar) | BOOST button |
| Reload | R | RELOAD button |
| Pause | Esc / P | - |
Dotted line previews your shot including bounces.

Map view (classic): tap = fire, hold = drive.
Other keys: C toggle view, M mute, F fullscreen; menu 1/2/3/S/H/Enter.
Settings has an aim-sensitivity slider.
and drive · release = spin again. Desktop extras: Space = tap/hold, C = switch
camera, Esc = back.

**Features:** 3D arena with chase or map camera · bouncing bullets (2 bounces) ·
5-round magazine with reload · friendly-fire immunity between teammates ·
random crates (repair, reload, nitro, rapid fire, shield) · practice vs 3 bots ·
LAN multiplayer 1v1 / 2v2 / 4v4 with auto-discovery on WiFi or a phone hotspot,
auto team balance, rematch without reconnecting · player names shown in-game ·
volume settings · How to Play · elimination banner and spectating ·
fully touch-driven UI including an on-screen keyboard (no physical keyboard needed).

## Build: Linux

```bash
sudo apt install build-essential pkg-config libasound2-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev libgl1-mesa-dev
cargo run --release
# binary: target/release/tank_arena_3d
```
(Fedora: `sudo dnf install alsa-lib-devel libX11-devel libXi-devel mesa-libGL-devel`.)

## Build: Windows

Easiest is natively on Windows: install Rust (rustup, MSVC toolchain), then

```powershell
cargo build --release
# target\release\tank_arena_3d.exe  (single file, nothing else to ship)
```

Cross-compiling from Linux/zsh instead:

```bash
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
# target/x86_64-pc-windows-gnu/release/tank_arena_3d.exe
```
Allow the game through Windows Firewall (private networks) the first time you host.

## Build: Android (APK)

Using the Docker image maintained for macroquad (no local Android SDK needed):

```bash
docker run --rm -v "$(pwd)":/root/src -w /root/src notfl3/cargo-apk \
  cargo quad-apk build --release
# APK: target/android-artifacts/release/apk/tank_arena_3d.apk
adb install -r target/android-artifacts/release/apk/tank_arena_3d.apk
```
The release APK is debug-signed, which is fine for sideloading. For the Play
Store, sign it with your own keystore (see the cargo-quad-apk docs).

## Playing over LAN

1. Everyone joins the same WiFi (or the host's phone hotspot).
2. One player taps **HOST LAN GAME**. Others tap **JOIN LAN GAME**: the host is
   found automatically and you land in the lobby. The host can switch 1v1/2v2/4v4
   and tap **START**.
3. If auto-discovery is blocked by the network, tap **ENTER IP MANUALLY** and type
   the IP shown on the host's lobby screen. UDP port **22122** must be reachable.

## Tests

```bash
cargo test        # simulation rules + LAN protocol over loopback + settings
```

## Layout

| file | what |
|---|---|
| `src/sim.rs` | headless rules: tanks, bullets, pickups, AI, snapshots |
| `src/net.rs` | UDP host/client, discovery, teams, rematch |
| `src/gfx.rs` | 3D models, procedural pixel-art ground, particles, cameras |
| `src/ui.rs` | touch UI + on-screen keyboard |
| `src/audio.rs` | embedded sound effects and music |
| `src/settings.rs` | name / volume / camera settings (file next to the game) |
| `src/main.rs` | screens and game loop |

Settings are saved to `tank_arena_3d_settings.txt` in the working directory when
writable (on Android the defaults are used if the app folder is read-only).

Font: `assets/fonts/font.ttf` is Press Start 2P (SIL Open Font License 1.1, license text embedded in the font file).

//! Tiny key=value settings file. Failing to read or write it is never fatal
//! (on Android the working directory may be read-only; defaults are used).

use crate::net::sanitize_name;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub name: String,
    pub music: f32,
    pub sfx: f32,
    pub match_type: u8,
    pub chase_cam: bool,
    /// Mouse / touch look sensitivity multiplier.
    pub sens: f32,
    /// Chosen tank class (index into sim::stats).
    pub tank: u8,
    /// Touch: aim with a floating right stick instead of dragging.
    pub aim_stick: bool,
    /// Touch chase view where aim = move direction and a DRIVE button moves the tank.
    pub aim_drive: bool,
    pub tap_fire: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { name: "Player".into(), music: 0.5, sfx: 0.8, match_type: 2, chase_cam: false, sens: 1.0, tank: 0, aim_stick: false, aim_drive: false, tap_fire: false }
    }
}

const FILE: &str = "tank_arena_3d_settings.txt";

/// Where the settings live. On a desktop that is the working directory. On
/// Android the working directory is read-only, so use the app's private
/// storage (/data/data/<package>/files), found through the process name.
fn file_path() -> std::path::PathBuf {
    #[cfg(target_os = "android")]
    {
        if let Ok(cmd) = std::fs::read("/proc/self/cmdline") {
            let name = cmd.split(|b| *b == 0).next().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
            if !name.is_empty() && !name.contains('/') {
                for root in ["/data/data", "/data/user/0"] {
                    let dir = std::path::PathBuf::from(format!("{root}/{name}/files"));
                    if std::fs::create_dir_all(&dir).is_ok() {
                        return dir.join(FILE);
                    }
                }
            }
        }
    }
    std::path::PathBuf::from(FILE)
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "name" => s.name = sanitize_name(v),
                "music" => s.music = v.parse::<f32>().unwrap_or(s.music).clamp(0.0, 1.0),
                "sfx" => s.sfx = v.parse::<f32>().unwrap_or(s.sfx).clamp(0.0, 1.0),
                "matchtype" => {
                    if let Ok(m) = v.parse::<u8>() {
                        if [1, 2, 4].contains(&m) {
                            s.match_type = m;
                        }
                    }
                }
                "chase" => s.chase_cam = v == "1" || v == "true",
                "sens" => s.sens = v.parse::<f32>().unwrap_or(s.sens).clamp(0.3, 3.0),
                "aimstick" => s.aim_stick = v == "1",
                "aimdrive" => s.aim_drive = v == "1",
                "tapfire" => s.tap_fire = v == "1",
                "tank" => {
                    if let Ok(t) = v.parse::<u8>() {
                        s.tank = t.min(crate::sim::KINDS as u8 - 1);
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn serialize(&self) -> String {
        format!(
            "name={}\nmusic={:.2}\nsfx={:.2}\nmatchtype={}\nchase={}\nsens={:.1}\ntank={}\naimstick={}\naimdrive={}\ntapfire={}\n",
            self.name, self.music, self.sfx, self.match_type, self.chase_cam as u8, self.sens, self.tank, self.aim_stick as u8, self.aim_drive as u8, self.tap_fire as u8
        )
    }

    pub fn load() -> Settings {
        std::fs::read_to_string(file_path()).map(|t| Settings::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = std::fs::write(file_path(), self.serialize());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Settings { name: "Zed".into(), music: 0.3, sfx: 0.0, match_type: 4, chase_cam: true, sens: 1.5, tank: 3, aim_stick: true, aim_drive: true, tap_fire: true };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn garbage_falls_back_to_defaults() {
        let s = Settings::parse("music=loud\nmatchtype=7\nname=a:b;c\nnonsense");
        assert_eq!(s.match_type, 2);
        assert_eq!(s.music, Settings::default().music);
        assert_eq!(s.name, "abc");
    }
}

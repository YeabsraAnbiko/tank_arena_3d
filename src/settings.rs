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
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            name: "Player".into(),
            music: 0.5,
            sfx: 0.8,
            match_type: 2,
            chase_cam: false,
            sens: 1.0,
            tank: 0,
            aim_stick: false,
        }
    }
}

const FILE: &str = "tank_arena_3d_settings.txt";

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
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
            "name={}\nmusic={:.2}\nsfx={:.2}\nmatchtype={}\nchase={}\nsens={:.1}\ntank={}\naimstick={}\n",
            self.name, self.music, self.sfx, self.match_type, self.chase_cam as u8, self.sens, self.tank, self.aim_stick as u8
        )
    }

    pub fn load() -> Settings {
        std::fs::read_to_string(FILE)
            .map(|t| Settings::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = std::fs::write(FILE, self.serialize());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Settings {
            name: "Zed".into(),
            music: 0.3,
            sfx: 0.0,
            match_type: 4,
            chase_cam: true,
            sens: 1.5,
            tank: 3,
            aim_stick: true,
        };
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

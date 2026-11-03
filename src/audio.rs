//! Sound effects and background music. All audio is embedded in the binary
//! so Android needs no asset folder. If audio cannot start, the game runs
//! silently.

use macroquad::audio::{load_sound_from_bytes, play_sound, play_sound_once, set_sound_volume, stop_sound, PlaySoundParams, Sound};

use crate::sim::{Event, EventKind};

pub struct Audio {
    sounds: Option<Sounds>,
    pub music_vol: f32,
    pub sfx_vol: f32,
    music_playing: bool,
}

struct Sounds {
    shoot: Sound,
    bounce: Sound,
    hit: Sound,
    explosion: Sound,
    powerup: Sound,
    music: Sound,
}

impl Audio {
    pub async fn load(music_vol: f32, sfx_vol: f32) -> Audio {
        let sounds = async {
            Some(Sounds {
                shoot: load_sound_from_bytes(include_bytes!("../assets/audio/sfx_shoot.wav")).await.ok()?,
                bounce: load_sound_from_bytes(include_bytes!("../assets/audio/sfx_bounce.wav")).await.ok()?,
                hit: load_sound_from_bytes(include_bytes!("../assets/audio/sfx_hit.wav")).await.ok()?,
                explosion: load_sound_from_bytes(include_bytes!("../assets/audio/sfx_explosion.wav")).await.ok()?,
                powerup: load_sound_from_bytes(include_bytes!("../assets/audio/sfx_powerup.wav")).await.ok()?,
                music: load_sound_from_bytes(include_bytes!("../assets/audio/music_classical.wav")).await.ok()?,
            })
        }
        .await;
        let mut a = Audio { sounds, music_vol, sfx_vol, music_playing: false };
        a.apply_music();
        a
    }

    pub fn set_music(&mut self, v: f32) {
        self.music_vol = v.clamp(0.0, 1.0);
        self.apply_music();
    }

    pub fn set_sfx(&mut self, v: f32) {
        self.sfx_vol = v.clamp(0.0, 1.0);
    }

    /// Start, retune or stop the looping music to match the volume.
    fn apply_music(&mut self) {
        let Some(s) = &self.sounds else { return };
        if self.music_vol <= 0.0 {
            if self.music_playing {
                stop_sound(&s.music);
                self.music_playing = false;
            }
        } else if !self.music_playing {
            play_sound(&s.music, PlaySoundParams { looped: true, volume: self.music_vol * 0.6 });
            self.music_playing = true;
        } else {
            set_sound_volume(&s.music, self.music_vol * 0.6);
        }
    }

    pub fn play_event(&self, e: &Event) {
        let Some(s) = &self.sounds else { return };
        if self.sfx_vol <= 0.0 {
            return;
        }
        let snd = match e.kind {
            EventKind::Fire => &s.shoot,
            EventKind::Bounce => &s.bounce,
            EventKind::Hit => &s.hit,
            EventKind::Explode => &s.explosion,
            EventKind::Pickup => &s.powerup,
        };
        play_sound(snd, PlaySoundParams { looped: false, volume: self.sfx_vol });
    }

    #[allow(dead_code)]
    pub fn blip(&self) {
        if let Some(s) = &self.sounds {
            if self.sfx_vol > 0.0 {
                play_sound_once(&s.bounce);
            }
        }
    }
}

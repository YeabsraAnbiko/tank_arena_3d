//! Immediate-mode, touch-first UI on a fixed 1280x720 virtual canvas that is
//! scaled and letterboxed to any screen. Every action is a tappable button,
//! and text entry uses an on-screen keyboard (macroquad has no Android soft
//! keyboard), so the whole game is playable without a physical keyboard.

use macroquad::prelude::*;

pub const VW: f32 = 1280.0;
pub const VH: f32 = 720.0;

pub const BG: Color = Color::new(0.05, 0.07, 0.12, 1.0);
pub const PANEL: Color = Color::new(0.10, 0.14, 0.22, 0.92);
pub const ACCENT: Color = Color::new(0.98, 0.72, 0.22, 1.0);
pub const TEXT: Color = Color::new(0.92, 0.94, 0.98, 1.0);
pub const DIM: Color = Color::new(0.62, 0.68, 0.78, 1.0);
pub const TEAM_BLUE: Color = Color::new(0.35, 0.6, 1.0, 1.0);
pub const TEAM_RED: Color = Color::new(1.0, 0.4, 0.38, 1.0);

#[derive(Clone, Copy, PartialEq)]
pub enum Style {
    Normal,
    Primary,
    Danger,
    Disabled,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(PartialEq, Debug)]
pub enum Kb {
    Editing,
    Done,
    Cancel,
}

pub struct Ui {
    pub font: Option<Font>,
    pub u: f32,
    ox: f32,
    oy: f32,
    /// Screen-space position of a tap/click that started this frame.
    pub click: Option<Vec2>,
    pub down: bool,
    /// Screen-space rects of buttons drawn this frame (to keep taps on
    /// buttons from also counting as "hold to drive").
    pub rects: Vec<Rect>,
    shift: bool,
    /// Scripted taps for automated UI tests: (frame, screen x, screen y).
    pub script: Vec<(u64, f32, f32)>,
    pub frame: u64,
}

impl Ui {
    pub fn new(font: Option<Font>) -> Ui {
        Ui { font, u: 1.0, ox: 0.0, oy: 0.0, click: None, down: false, rects: Vec::new(), shift: true, script: Vec::new(), frame: 0 }
    }

    pub fn begin_frame(&mut self) {
        let (sw, sh) = (screen_width(), screen_height());
        self.u = (sw / VW).min(sh / VH);
        self.ox = (sw - VW * self.u) / 2.0;
        self.oy = (sh - VH * self.u) / 2.0;
        self.rects.clear();
        self.frame += 1;

        let mut click = None;
        if is_mouse_button_pressed(MouseButton::Left) {
            click = Some(Vec2::from(mouse_position()));
        }
        // Touch: macroquad also emulates the mouse, but handle raw touches in
        // case that is switched off on some platform.
        for t in touches() {
            if t.phase == TouchPhase::Started {
                click = Some(t.position);
            }
        }
        if let Some(&(_, x, y)) = self.script.iter().find(|s| s.0 == self.frame) {
            click = Some(vec2(x, y));
        }
        self.click = click;
        self.down = is_mouse_button_down(MouseButton::Left) || touches().iter().any(|t| t.phase != TouchPhase::Ended && t.phase != TouchPhase::Cancelled);
    }

    /// Screen pixels -> virtual (1280x720) coordinates.
    pub fn to_virtual(&self, p: Vec2) -> Vec2 {
        vec2((p.x - self.ox) / self.u, (p.y - self.oy) / self.u)
    }

    /// Virtual -> screen rectangle.
    pub fn r(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(self.ox + x * self.u, self.oy + y * self.u, w * self.u, h * self.u)
    }

    /// Pixel size to rasterise at. The font is a wide 8x8-grid pixel font, so
    /// sizes are scaled down and snapped to multiples of 8 to stay crisp.
    fn px(&self, size: f32) -> f32 {
        ((size * 0.62 * self.u) / 8.0).round().max(1.0) * 8.0
    }

    fn width_px(&self, s: &str, px: f32) -> f32 {
        measure_text(s, self.font.as_ref(), px as u16, 1.0).width
    }

    pub fn text_width(&self, s: &str, size: f32) -> f32 {
        self.width_px(s, self.px(size)) / self.u
    }

    /// Largest snapped size (<= `size`) at which `s` fits in `max_w` virtual units.
    fn fit_px(&self, s: &str, size: f32, max_w: f32) -> f32 {
        let mut px = self.px(size);
        while px > 8.0 && self.width_px(s, px) / self.u > max_w {
            px -= 8.0;
        }
        px
    }

    /// Draw text with its vertical centre at `y` (virtual coordinates).
    pub fn text(&self, s: &str, x: f32, y: f32, size: f32, color: Color, align: Align) {
        self.text_fit(s, x, y, size, color, align, f32::MAX);
    }

    /// Like `text`, but shrinks to fit inside `max_w`.
    pub fn text_fit(&self, s: &str, x: f32, y: f32, size: f32, color: Color, align: Align, max_w: f32) {
        let px = self.fit_px(s, size, max_w);
        let w = self.width_px(s, px) / self.u;
        let x = match align {
            Align::Left => x,
            Align::Center => x - w / 2.0,
            Align::Right => x - w,
        };
        let params = TextParams { font: self.font.as_ref(), font_size: px as u16, color, ..Default::default() };
        draw_text_ex(s, self.ox + x * self.u, self.oy + y * self.u + px * 0.4, params);
    }

    /// Greedy word wrap into lines no wider than `max_w`.
    pub fn wrap(&self, s: &str, size: f32, max_w: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut cur = String::new();
        for word in s.split_whitespace() {
            let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if !cur.is_empty() && self.text_width(&trial, size) > max_w {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            } else {
                cur = trial;
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    }

    pub fn panel(&self, x: f32, y: f32, w: f32, h: f32) {
        let r = self.r(x, y, w, h);
        draw_rectangle(r.x, r.y, r.w, r.h, PANEL);
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0 * self.u, Color::new(1.0, 1.0, 1.0, 0.12));
    }

    pub fn fill(&self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        let r = self.r(x, y, w, h);
        draw_rectangle(r.x, r.y, r.w, r.h, c);
    }

    /// A tappable button; true on the frame it is tapped.
    pub fn button(&mut self, x: f32, y: f32, w: f32, h: f32, label: &str, style: Style) -> bool {
        let r = self.r(x, y, w, h);
        let hovered = !matches!(style, Style::Disabled) && self.click.map_or(false, |c| r.contains(c));
        let (fill, edge, tc) = match style {
            Style::Normal => (Color::new(0.16, 0.23, 0.37, 1.0), Color::new(0.45, 0.6, 0.9, 0.6), TEXT),
            Style::Primary => (Color::new(0.80, 0.55, 0.12, 1.0), Color::new(1.0, 0.85, 0.4, 0.9), Color::new(0.1, 0.08, 0.04, 1.0)),
            Style::Danger => (Color::new(0.5, 0.15, 0.15, 1.0), Color::new(1.0, 0.45, 0.4, 0.7), TEXT),
            Style::Disabled => (Color::new(0.12, 0.14, 0.2, 1.0), Color::new(0.3, 0.35, 0.45, 0.5), Color::new(0.45, 0.5, 0.6, 1.0)),
        };
        // pressed look while the pointer is held on it
        let pressed = self.down && !matches!(style, Style::Disabled) && Vec2::from(mouse_position()).x >= r.x && r.contains(Vec2::from(mouse_position()));
        let fill = if pressed { Color::new(fill.r * 1.25, fill.g * 1.25, fill.b * 1.25, 1.0) } else { fill };
        draw_rectangle(r.x, r.y, r.w, r.h, fill);
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 3.0 * self.u, edge);
        self.rects.push(r);
        let size = (h * 0.46).min(34.0);
        self.text_fit(label, x + w / 2.0, y + h / 2.0, size, tc, Align::Center, w - 24.0);
        hovered
    }

    pub fn title(&self, s: &str, y: f32) {
        self.text(s, VW / 2.0 + 4.0, y + 4.0, 80.0, Color::new(0.0, 0.0, 0.0, 0.6), Align::Center);
        self.text(s, VW / 2.0, y, 80.0, ACCENT, Align::Center);
    }

    /// True if the last click landed on any button drawn this frame.
    pub fn over_button(&self, p: Vec2) -> bool {
        self.rects.iter().any(|r| r.contains(p))
    }

    /// On-screen keyboard that edits `text`. `numeric` shows digits and '.'.
    pub fn keyboard(&mut self, title: &str, text: &mut String, max: usize, numeric: bool) -> Kb {
        let mut result = Kb::Editing;
        self.panel(140.0, 60.0, 1000.0, 600.0);
        self.text(title, VW / 2.0, 105.0, 30.0, DIM, Align::Center);
        // text box
        self.fill(190.0, 140.0, 900.0, 70.0, Color::new(0.02, 0.03, 0.06, 1.0));
        let caret = if (get_time() * 2.0) as i64 % 2 == 0 { "_" } else { " " };
        self.text(&format!("{}{}", text, caret), 215.0, 175.0, 36.0, TEXT, Align::Left);

        // physical keyboard still works on desktop
        while let Some(c) = get_char_pressed() {
            if !c.is_control() && text.chars().count() < max {
                if !numeric || c.is_ascii_digit() || c == '.' {
                    text.push(c);
                }
            }
        }
        if is_key_pressed(KeyCode::Backspace) {
            text.pop();
        }
        if is_key_pressed(KeyCode::Enter) {
            result = Kb::Done;
        }

        let rows: Vec<&str> = if numeric {
            vec!["123", "456", "789", ".0"]
        } else {
            vec!["1234567890", "QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM-_"]
        };
        let (kw, kh, gap) = if numeric { (130.0, 70.0, 12.0) } else { (78.0, 64.0, 8.0) };
        let mut y = 232.0;
        for row in &rows {
            let n = row.chars().count() as f32;
            let total = n * kw + (n - 1.0) * gap;
            let mut x = VW / 2.0 - total / 2.0;
            for ch in row.chars() {
                let shown = if ch.is_ascii_alphabetic() && !self.shift { ch.to_ascii_lowercase() } else { ch };
                if self.button(x, y, kw, kh, &shown.to_string(), Style::Normal) && text.chars().count() < max {
                    text.push(shown);
                    self.shift = false;
                }
                x += kw + gap;
            }
            y += kh + gap;
        }
        // bottom row
        let by = y + 6.0;
        if !numeric {
            if self.button(190.0, by, 150.0, 66.0, if self.shift { "ABC" } else { "abc" }, Style::Normal) {
                self.shift = !self.shift;
            }
            if self.button(352.0, by, 330.0, 66.0, "SPACE", Style::Normal) && text.chars().count() < max {
                text.push(' ');
            }
        }
        if self.button(if numeric { 190.0 } else { 694.0 }, by, 150.0, 66.0, "DEL", Style::Normal) {
            text.pop();
        }
        if self.button(852.0, by, 118.0, 66.0, "OK", Style::Primary) {
            result = Kb::Done;
        }
        if self.button(982.0, by, 108.0, 66.0, "X", Style::Danger) {
            result = Kb::Cancel;
        }
        if is_key_pressed(KeyCode::Escape) {
            result = Kb::Cancel;
        }
        result
    }
}

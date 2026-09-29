//! Named colors and color arithmetic shared by every view.
//!
//! Views never spell out `Color::Rgb(...)` literals; they pick a name from
//! here so the palette can be tuned in one place.

use ratatui::style::Color;

// Neutral scale, brightest first.
pub const BRIGHT: Color = Color::Rgb(220, 220, 235);
pub const TEXT: Color = Color::Rgb(180, 180, 195);
pub const MUTED: Color = Color::Rgb(130, 130, 150);
pub const MUTED_DARK: Color = Color::Rgb(110, 110, 130);
pub const SUBTLE: Color = Color::Rgb(80, 80, 100);
pub const DIM: Color = Color::Rgb(70, 70, 85);
pub const FAINT: Color = Color::Rgb(55, 55, 65);
pub const RULE: Color = Color::Rgb(35, 35, 45);
pub const GHOST: Color = Color::Rgb(30, 30, 38);
pub const BG: Color = Color::Rgb(16, 16, 22);

// Accent scale.
pub const ACCENT: Color = Color::Rgb(59, 130, 246);
pub const ACCENT_LIGHT: Color = Color::Rgb(96, 165, 250);
pub const ACCENT_PALE: Color = Color::Rgb(147, 197, 253);
pub const HIGHLIGHT_BG: Color = Color::Rgb(40, 45, 65);

// Semantic colors.
pub const SUCCESS: Color = Color::Rgb(34, 197, 94);
pub const WARNING: Color = Color::Rgb(234, 179, 8);
pub const ERROR: Color = Color::Rgb(239, 68, 68);

// Hues used for tagging values.
pub const ORANGE: Color = Color::Rgb(249, 115, 22);
pub const AMBER: Color = Color::Rgb(251, 191, 36);
pub const YELLOW: Color = Color::Rgb(253, 224, 71);
pub const LIME: Color = Color::Rgb(190, 242, 100);
pub const PURPLE: Color = Color::Rgb(168, 85, 247);
pub const CYAN: Color = Color::Rgb(6, 182, 212);
pub const POPUP_BORDER: Color = Color::Rgb(245, 166, 35);

// Search-hit backgrounds.
pub const MATCH_BG: Color = Color::Rgb(60, 50, 20);
pub const MATCH_ACTIVE_BG: Color = Color::Rgb(100, 80, 10);

/// A color ramp through RGB stops at ascending positions in `0..=1`.
pub struct Gradient(pub &'static [(f64, (u8, u8, u8))]);

impl Gradient {
    /// Color at position `t`, clamped to `0..=1`.
    pub fn at(&self, t: f64) -> Color {
        let t = t.clamp(0.0, 1.0);
        let (r, g, b) = self
            .0
            .windows(2)
            .find(|w| t <= w[1].0)
            .map(|w| {
                let ((t0, c0), (t1, c1)) = (w[0], w[1]);
                let s = if (t1 - t0).abs() < f64::EPSILON {
                    0.0
                } else {
                    (t - t0) / (t1 - t0)
                };
                (
                    lerp(c0.0, c1.0, s),
                    lerp(c0.1, c1.1, s),
                    lerp(c0.2, c1.2, s),
                )
            })
            .unwrap_or_else(|| self.0.last().map_or((0, 0, 0), |s| s.1));
        Color::Rgb(r, g, b)
    }
}

/// Arithmetic on RGB colors. Non-RGB colors pass through unchanged.
pub trait ColorExt: Sized {
    fn lighten(self, amount: u8) -> Color;
    fn darken(self, amount: u8) -> Color;
    /// Mix toward `other` by `t` in `0..=1`.
    fn blend(self, other: Color, t: f64) -> Color;
    /// Near-black or near-white text that stays readable on `self`.
    fn contrast_fg(self) -> Color;
}

impl ColorExt for Color {
    fn lighten(self, amount: u8) -> Color {
        map_rgb(self, |v| v.saturating_add(amount))
    }

    fn darken(self, amount: u8) -> Color {
        map_rgb(self, |v| v.saturating_sub(amount))
    }

    fn blend(self, other: Color, t: f64) -> Color {
        match (self, other) {
            (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
                Color::Rgb(lerp(r1, r2, t), lerp(g1, g2, t), lerp(b1, b2, t))
            }
            _ => self,
        }
    }

    fn contrast_fg(self) -> Color {
        match self {
            Color::Rgb(r, g, b) => {
                let luma = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
                if luma > 160.0 {
                    Color::Rgb(20, 18, 15)
                } else {
                    Color::Rgb(250, 248, 245)
                }
            }
            _ => Color::White,
        }
    }
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    ((1.0 - t) * a as f64 + t * b as f64).round() as u8
}

fn map_rgb(c: Color, f: impl Fn(u8) -> u8) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(f(r), f(g), f(b)),
        _ => c,
    }
}

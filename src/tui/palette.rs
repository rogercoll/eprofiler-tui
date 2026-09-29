//! Flamegraph frame colors.
//!
//! A frame's color is a pure function of what the frame is, never of where it
//! sits in the graph, so colors stay put across updates, re-sorts and zooms:
//!
//! - **hue** comes from the runtime (Go is cyan, JVM green, ...), with a small
//!   per-name offset so adjacent siblings stay distinguishable;
//! - **tone** separates application code (vivid) from runtime, stdlib and
//!   system code (a pastel tint of the same hue). Kernel frames stay vivid so
//!   kernel time stands out;
//! - **lightness** grows with self time, so frames that burn CPU themselves
//!   glow and pass-through frames stay dark. Width already shows total time.

use ratatui::style::Color;

use crate::frame::{FrameKind, Origin, Runtime};

const LIGHT_INLINED: f64 = 0.07;
const LIGHT_JITTER: f64 = 0.025;

/// Saturation and lightness range for one group of frames.
#[derive(Clone, Copy)]
struct Tone {
    sat: f64,
    /// Lightness of a frame with no self time.
    light_min: f64,
    /// Extra lightness for a frame that is all self time.
    light_range: f64,
}

/// How one runtime's frames are colored.
#[derive(Clone, Copy)]
struct Band {
    /// Hue center in degrees.
    center: f64,
    /// Per-name hue offset, up to this many degrees either way.
    spread: f64,
    application: Tone,
    runtime: Tone,
}

/// Runtime, stdlib and system frames: a pastel tint of the runtime's hue.
/// Mixing toward white rather than grey keeps them pleasant, while vivid
/// color stays reserved for application code.
const PASTEL: Tone = Tone {
    sat: 0.80,
    light_min: 0.76,
    light_range: 0.08,
};

impl Band {
    const fn around(center: f64) -> Self {
        Self {
            center,
            spread: 7.0,
            application: Tone {
                sat: 0.85,
                light_min: 0.42,
                light_range: 0.24,
            },
            runtime: PASTEL,
        }
    }
}

/// Brendan Gregg's "hot" scheme: each name lands somewhere between red and
/// yellow, at full saturation, since dim orange and yellow read as brown.
/// Runtime frames become salmon, peach and cream.
const NATIVE: Band = Band {
    center: 27.0,
    spread: 25.0,
    application: Tone {
        sat: 1.0,
        light_min: 0.50,
        light_range: 0.16,
    },
    runtime: PASTEL,
};

/// The band for a runtime, or `None` for grey (non-code rows and unknown frames).
/// Every non-native hue sits outside native's red-to-yellow range.
fn band(runtime: Runtime) -> Option<Band> {
    let center = match runtime {
        Runtime::Native => return Some(NATIVE),
        Runtime::Js => 75.0,
        Runtime::Jvm => 115.0,
        Runtime::Perl => 145.0,
        Runtime::Kernel => 172.0,
        Runtime::Go => 197.0,
        Runtime::Python => 222.0,
        Runtime::Php => 248.0,
        Runtime::Dotnet => 272.0,
        Runtime::Beam => 300.0,
        Runtime::Ruby => 330.0,
        Runtime::Unknown | Runtime::Thread => return None,
    };
    Some(Band::around(center))
}

/// Color of a frame with the given kind, label and self-time ratio.
pub fn frame_color(kind: FrameKind, name: &str, self_ratio: f64) -> Color {
    let hash = fnv1a(name);
    // Two independent values in -1.0..=1.0 from different hash bits.
    let j1 = ((hash & 0xffff) as f64 / 32767.5) - 1.0;
    let j2 = (((hash >> 16) & 0xffff) as f64 / 32767.5) - 1.0;
    shade(kind, self_ratio, j1, j2 * LIGHT_JITTER)
}

/// Representative color for the legend: medium self time, no jitter.
pub fn swatch(runtime: Runtime, origin: Origin) -> Color {
    let kind = FrameKind {
        runtime,
        origin,
        inlined: false,
    };
    shade(kind, 0.5, 0.0, 0.0)
}

/// `hue_jitter` is in -1.0..=1.0 and is scaled by the band's spread.
fn shade(kind: FrameKind, self_ratio: f64, hue_jitter: f64, light_offset: f64) -> Color {
    let band = band(kind.runtime);
    // Grey rows use the default lightness so they sit with the other frames.
    let b = band.unwrap_or(Band::around(0.0));
    let muted = kind.origin == Origin::Runtime && kind.runtime != Runtime::Kernel;
    let tone = if muted { b.runtime } else { b.application };

    let heat = self_ratio.clamp(0.0, 1.0).sqrt();
    let mut light = tone.light_min + tone.light_range * heat + light_offset;
    if kind.inlined {
        light += LIGHT_INLINED;
    }
    let sat = if band.is_some() { tone.sat } else { 0.0 };
    hsl(b.center + hue_jitter * b.spread, sat, light.clamp(0.0, 1.0))
}

fn hsl(h: f64, s: f64, l: f64) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_u8 = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::Rgb(to_u8(r), to_u8(g), to_u8(b))
}

/// Stable across runs and platforms, unlike `DefaultHasher`.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(runtime: Runtime, origin: Origin) -> FrameKind {
        FrameKind {
            runtime,
            origin,
            inlined: false,
        }
    }

    fn rgb(c: Color) -> (u8, u8, u8) {
        match c {
            Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("expected rgb, got {other:?}"),
        }
    }

    fn luma(c: Color) -> f64 {
        let (r, g, b) = rgb(c);
        0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64
    }

    fn chroma(c: Color) -> u8 {
        let (r, g, b) = rgb(c);
        r.max(g).max(b) - r.min(g).min(b)
    }

    #[test]
    fn hsl_primaries() {
        assert_eq!(hsl(0.0, 1.0, 0.5), Color::Rgb(255, 0, 0));
        assert_eq!(hsl(120.0, 1.0, 0.5), Color::Rgb(0, 255, 0));
        assert_eq!(hsl(240.0, 1.0, 0.5), Color::Rgb(0, 0, 255));
        assert_eq!(hsl(0.0, 0.0, 0.5), Color::Rgb(128, 128, 128));
    }

    #[test]
    fn same_frame_same_color() {
        let k = kind(Runtime::Go, Origin::Application);
        assert_eq!(
            frame_color(k, "main.work", 0.3),
            frame_color(k, "main.work", 0.3)
        );
    }

    #[test]
    fn hotter_frames_are_brighter() {
        let k = kind(Runtime::Python, Origin::Application);
        assert!(luma(frame_color(k, "f", 0.9)) > luma(frame_color(k, "f", 0.0)));
    }

    #[test]
    fn runtime_frames_are_pastel_except_kernel() {
        for runtime in Runtime::ALL.into_iter().filter(|r| band(*r).is_some()) {
            let app = swatch(runtime, Origin::Application);
            let rt = swatch(runtime, Origin::Runtime);
            if runtime == Runtime::Kernel {
                assert_eq!(app, rt, "kernel stays vivid");
                continue;
            }
            // Pastel: lighter and softer than the vivid tone, yet clearly colored.
            assert!(
                luma(rt) > luma(app),
                "{runtime:?}: {rt:?} not lighter than {app:?}"
            );
            assert!(
                chroma(rt) < chroma(app),
                "{runtime:?}: {rt:?} not softer than {app:?}"
            );
            assert!(chroma(rt) > 50, "{runtime:?}: {rt:?} is greyish");
        }
    }

    #[test]
    fn thread_rows_are_grey() {
        assert_eq!(chroma(frame_color(FrameKind::THREAD, "worker-1", 0.0)), 0);
    }

    #[test]
    fn every_runtime_hue_is_distinct() {
        let colors: std::collections::HashSet<_> = Runtime::ALL
            .iter()
            .filter(|r| band(**r).is_some())
            .map(|r| rgb(swatch(*r, Origin::Application)))
            .collect();
        assert_eq!(colors.len(), 11);
    }

    #[test]
    fn native_frames_span_red_to_yellow() {
        let k = kind(Runtime::Native, Origin::Application);
        let greens: Vec<u8> = (0..200)
            .map(|i| rgb(frame_color(k, &format!("fn_{i}"), 0.0)))
            .inspect(|&(r, _, b)| assert!(r > b, "native stays warm"))
            .map(|(_, g, _)| g)
            .collect();
        let (lo, hi) = (greens.iter().min().unwrap(), greens.iter().max().unwrap());
        assert!(*lo < 60, "some frames are red, min green {lo}");
        assert!(*hi > 150, "some frames are yellow, max green {hi}");
    }

    #[test]
    fn native_colors_are_never_brown_and_runtime_is_pastel() {
        let app = kind(Runtime::Native, Origin::Application);
        let rt = kind(Runtime::Native, Origin::Runtime);
        for i in 0..200 {
            let name = format!("fn_{i}");
            let vivid = frame_color(app, &name, 0.0);
            let pastel = frame_color(rt, &name, 0.0);
            // Brown is a warm hue with a dim top channel; both tones keep red high.
            for c in [vivid, pastel] {
                assert!(rgb(c).0 >= 230, "{name}: {c:?} reads as brown");
            }
            // Pastel: lighter and softer than the vivid tone, yet clearly colored.
            assert!(luma(pastel) > luma(vivid), "{name}: {pastel:?} not lighter");
            assert!(
                chroma(pastel) < chroma(vivid),
                "{name}: {pastel:?} not softer"
            );
            assert!(chroma(pastel) > 50, "{name}: {pastel:?} is greyish");
        }
    }
}

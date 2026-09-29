//! Text formatting shared by the views.

/// Cut `s` to at most `max` characters, ending in `…` when truncated.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max <= 1 {
        s.chars().take(max).collect()
    } else {
        s.chars()
            .take(max - 1)
            .chain(std::iter::once('…'))
            .collect()
    }
}

/// Compact sample count: `1234567` is `1.2M`, `1234` is `1.2K`.
pub fn format_count(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1_000_000.0),
        n if n >= 1_000 => format!("{:.1}K", n as f64 / 1_000.0),
        n => n.to_string(),
    }
}

/// Duration with a unit that keeps three significant digits.
pub fn format_duration(nanos: u64) -> String {
    match nanos {
        n if n >= 1_000_000_000 => format!("{:.3}s", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.3}ms", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.1}µs", n as f64 / 1e3),
        n => format!("{n}ns"),
    }
}

/// Unix timestamp as `seconds.millis`.
pub fn format_timestamp(nanos: u64) -> String {
    let (secs, ms) = (nanos / 1_000_000_000, (nanos % 1_000_000_000) / 1_000_000);
    format!("{secs}.{ms:03}s (epoch)")
}

pub fn format_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello", 4), "hel…");
        assert_eq!(truncate("hello", 1), "h");
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn format_count_scales() {
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_500), "1.5K");
        assert_eq!(format_count(2_500_000), "2.5M");
    }

    #[test]
    fn format_duration_picks_a_unit() {
        assert_eq!(format_duration(999), "999ns");
        assert_eq!(format_duration(1_500), "1.5µs");
        assert_eq!(format_duration(2_000_000_000), "2.000s");
    }
}

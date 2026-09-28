//! Small buffer-level drawing and text helpers shared by all views.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Widget},
};

use super::theme;

/// Paint every cell in `area` with `style` and a blank glyph.
pub fn fill(buf: &mut Buffer, area: Rect, style: Style) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_char(' ');
                c.set_style(style);
            }
        }
    }
}

/// Draw a horizontal run of `ch` from `x0` (inclusive) to `x1` (exclusive).
pub fn hline(buf: &mut Buffer, y: u16, x0: u16, x1: u16, ch: char, style: Style) {
    for x in x0..x1 {
        if let Some(c) = buf.cell_mut((x, y)) {
            c.set_char(ch);
            c.set_style(style);
        }
    }
}

/// Write `text` centered horizontally in `area` on row `y`, if `y` is inside `area`.
pub fn center(buf: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) {
    if y >= area.bottom() {
        return;
    }
    let width = text.chars().count() as u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    buf.set_string(x, y, text, style);
}

/// Clear `area` and draw a rounded, titled border around it.
pub fn popup_frame(buf: &mut Buffer, area: Rect, title: &str, border: Color) {
    Clear.render(area, buf);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::reset().fg(border))
        .style(Style::reset())
        .title(Span::styled(
            title,
            theme::bold(theme::BRIGHT).bg(Color::Reset),
        ))
        .render(area, buf);
}

/// Footer line of `[key] description` pairs.
pub fn key_hints(hints: &[(&str, &str)]) -> Line<'static> {
    let spans = hints
        .iter()
        .enumerate()
        .flat_map(|(i, (key, desc))| {
            let prefix = if i == 0 { " " } else { "" };
            [
                format!("{prefix}{key}").fg(theme::SUBTLE),
                desc.to_string().fg(theme::FAINT),
            ]
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

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

/// `1234567` -> `1.2M`, `1234` -> `1.2K`.
pub fn format_count(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1_000_000.0),
        n if n >= 1_000 => format!("{:.1}K", n as f64 / 1_000.0),
        n => n.to_string(),
    }
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
}

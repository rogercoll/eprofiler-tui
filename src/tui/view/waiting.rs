//! Splash screen shown until the first profile arrives.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::error::Error;
use crate::tui::canvas::BufferExt;
use crate::tui::theme;

/// Logo, listen address and either a waiting message or why the server is down.
pub struct Waiting<'a> {
    pub listen_addr: &'a str,
    pub error: Option<&'a Error>,
}

impl Waiting<'_> {
    const LOGO: [&'static str; 6] = [
        " ███████╗██████╗ ██████╗  ██████╗ ███████╗██╗██╗     ███████╗██████╗       ████████╗██╗   ██╗██╗",
        " ██╔════╝██╔══██╗██╔══██╗██╔═══██╗██╔════╝██║██║     ██╔════╝██╔══██╗      ╚══██╔══╝██║   ██║██║",
        " █████╗  ██████╔╝██████╔╝██║   ██║█████╗  ██║██║     █████╗  ██████╔╝█████╗   ██║   ██║   ██║██║",
        " ██╔══╝  ██╔═══╝ ██╔══██╗██║   ██║██╔══╝  ██║██║     ██╔══╝  ██╔══██╗╚════╝   ██║   ██║   ██║██║",
        " ███████╗██║     ██║  ██║╚██████╔╝██║     ██║███████╗███████╗██║  ██║         ██║   ╚██████╔╝██║",
        " ╚══════╝╚═╝     ╚═╝  ╚═╝ ╚═════╝ ╚═╝     ╚═╝╚══════╝╚══════╝╚═╝  ╚═╝         ╚═╝    ╚═════╝ ╚═╝",
    ];
    /// One color per logo line, top to bottom.
    const LOGO_COLORS: [Color; 6] = [
        Color::Rgb(168, 50, 160),
        Color::Rgb(200, 40, 80),
        Color::Rgb(220, 50, 32),
        Color::Rgb(240, 100, 18),
        Color::Rgb(250, 170, 30),
        theme::YELLOW,
    ];
    const SUBTITLE: &'static str = "OTLP Profile Flamegraph Viewer";
    /// Lines below the logo: subtitle, rule, address, waiting message.
    const TEXT_H: u16 = 5;
}

impl Widget for Waiting<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let logo_h = Self::LOGO.len() as u16;
        let logo_w = Self::LOGO
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0) as u16;
        let total_h = logo_h + Self::TEXT_H;
        let top = area.y + area.height.saturating_sub(total_h) / 2;

        if area.width >= logo_w + 2 && area.height >= total_h {
            let x = area.x + area.width.saturating_sub(logo_w) / 2;
            for (i, (line, color)) in Self::LOGO.iter().zip(Self::LOGO_COLORS).enumerate() {
                buf.set_string(x, top + i as u16, line, Style::new().fg(color));
            }
        } else {
            let style = Style::new().fg(Self::LOGO_COLORS[4]).bold();
            buf.center(area, top + 1, "◆ eprofiler-tui", style);
        }

        let y = top + logo_h + 1;
        buf.center(
            area,
            y,
            Self::SUBTITLE,
            Style::new().fg(theme::BRIGHT).bold(),
        );
        let rule_w = Self::SUBTITLE.len() as u16;
        let rule_x = area.x + area.width.saturating_sub(rule_w) / 2;
        let rule = Style::new().fg(theme::RULE);
        buf.hline(y + 1, rule_x, rule_x + rule_w, '─', rule);
        let (status, status_color, note) = match self.error {
            None => (
                format!("Listening on {}", self.listen_addr),
                theme::MUTED,
                "Waiting for profiles...",
            ),
            Some(error) => (error.to_string(), theme::ERROR, "Not receiving profiles"),
        };
        buf.center(area, y + 2, &status, Style::new().fg(status_color));
        buf.center(area, y + 3, note, Style::new().fg(theme::DIM).italic());
    }
}

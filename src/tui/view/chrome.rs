//! UI pieces shared by every tab: header, footer, detail lines and placeholders.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Tabs, Widget},
};

use crate::tui::canvas::BufferExt;
use crate::tui::state::{ActiveTab, State};
use crate::tui::text::format_count;
use crate::tui::theme;

/// `(key, description)` pairs for the footer.
pub type Keys = &'static [(&'static str, &'static str)];

/// A row of `│`-separated groups, as used by the header and detail bars.
#[derive(Default)]
pub struct DetailLine(Vec<Span<'static>>);

impl DetailLine {
    /// Append a group, separated from the previous one.
    pub fn group(mut self, spans: impl IntoIterator<Item = Span<'static>>) -> Self {
        if !self.0.is_empty() {
            self.0.push(" │ ".fg(theme::FAINT));
        }
        self.0.extend(spans);
        self
    }

    /// Append a `label: value` group.
    pub fn field(self, label: &str, value: impl Into<String>, color: Color) -> Self {
        self.group([format!("{label}: ").fg(theme::DIM), value.into().fg(color)])
    }

    /// Append a highlighted `▸ text` group naming the current selection.
    pub fn selection(self, text: impl Into<String>) -> Self {
        self.group([
            " ▸ ".fg(theme::ACCENT).bold(),
            text.into().fg(theme::BRIGHT).bold(),
        ])
    }
}

impl From<DetailLine> for Line<'static> {
    fn from(line: DetailLine) -> Self {
        Line::from(line.0)
    }
}

/// App name, listen address and totals on the left; tabs and the live/frozen
/// indicator on the right.
pub struct Header<'a>(pub &'a State);

impl Header<'_> {
    const TABS: [(&'static str, ActiveTab); 3] = [
        ("Flamegraph", ActiveTab::Flamegraph),
        ("Flamescope", ActiveTab::Flamescope),
        ("Executables", ActiveTab::Executables),
    ];
}

impl Widget for Header<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let state = self.0;
        Line::from(
            DetailLine::default()
                .group([
                    " ◆ ".fg(theme::ACCENT),
                    "eprofiler-tui".fg(theme::BRIGHT).bold(),
                ])
                .group([state.listen_addr.clone().fg(theme::MUTED)])
                .group([format!("{} profiles", state.fg.profiles_received).fg(theme::MUTED_DARK)])
                .group([
                    format!("{} samples", format_count(state.fg.samples_received))
                        .fg(theme::MUTED_DARK),
                ]),
        )
        .render(area, buf);

        let (indicator, color) = if state.fg.frozen {
            (" ⏸ FROZEN ", theme::WARNING)
        } else {
            (" ▶ LIVE ", theme::SUCCESS)
        };
        let indicator_x = area
            .right()
            .saturating_sub(indicator.chars().count() as u16);
        buf.set_string(
            indicator_x,
            area.y,
            indicator,
            Style::new().fg(color).bold(),
        );

        // Right-aligned tab strip just left of the indicator: each title is
        // padded by one column on both sides, with a one-column divider between.
        let titles = Self::TABS.map(|(label, _)| label);
        let width = titles.iter().map(|t| t.len() as u16 + 3).sum::<u16>() - 1;
        Tabs::new(titles)
            .select(
                Self::TABS
                    .iter()
                    .position(|(_, tab)| *tab == state.active_tab),
            )
            .style(Style::new().fg(theme::DIM))
            .highlight_style(Style::new().fg(theme::BRIGHT).bold())
            .divider(Span::styled("│", Style::new().fg(theme::FAINT)))
            .render(
                Rect::new(indicator_x.saturating_sub(width), area.y, width, 1),
                buf,
            );
    }
}

/// Footer line of `[key] description` pairs.
pub struct KeyHints(pub Keys);

impl Widget for KeyHints {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let spans: Vec<Span> = self
            .0
            .iter()
            .enumerate()
            .flat_map(|(i, (key, desc))| {
                let prefix = if i == 0 { " " } else { "" };
                [
                    format!("{prefix}{key}").fg(theme::SUBTLE),
                    desc.fg(theme::FAINT),
                ]
            })
            .collect();
        Line::from(spans).render(area, buf);
    }
}

/// Centered italic message for views with nothing to show yet.
pub struct Placeholder<'a>(pub &'a str);

impl Widget for Placeholder<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let y = area.y + area.height / 2;
        buf.center(area, y, self.0, Style::new().fg(theme::SUBTLE).italic());
    }
}

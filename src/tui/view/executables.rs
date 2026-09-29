//! Discovered executables and their loaded symbols.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint::Length, Rect},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, Cell, Padding, Row, StatefulWidget, Table, TableState},
};

use super::TabView;
use super::chrome::Keys;
use crate::tui::canvas::BufferExt;
use crate::tui::state::{ExeEntry, ExecutablesTab};
use crate::tui::text::{format_count, truncate};
use crate::tui::theme;

impl TabView for ExecutablesTab {
    const KEYS: Keys = &[
        ("[Tab]", " switch "),
        ("[j/k]", " navigate "),
        ("[Enter]", " symbolize "),
        ("[r]", " remove "),
        ("[/]", " add new "),
        ("[q]", " quit "),
    ];

    fn render_body(&mut self, area: Rect, buf: &mut Buffer) {
        if area.height < 2 {
            return;
        }
        let id_w = ID_MAX_W.min(area.width / 3);
        let name_w = area.width.saturating_sub(id_w + SYMBOLS_W + 4);
        let rows = self
            .list
            .iter()
            .enumerate()
            .map(|(i, entry)| entry.table_row(i == self.cursor.index, name_w));
        let header = Row::new(["File ID", "Name", "Symbols"])
            .style(Style::new().fg(theme::DIM).bold())
            .bottom_margin(1);
        let widths = [Length(id_w), Length(name_w), Length(SYMBOLS_W)];
        let table = Table::new(rows, widths)
            .header(header)
            .column_spacing(0)
            .row_highlight_style(Style::new().bg(theme::HIGHLIGHT_BG))
            .block(Block::new().padding(Padding::left(1)));

        let mut state = TableState::new()
            .with_offset(self.cursor.offset)
            .with_selected(Some(self.cursor.index));
        StatefulWidget::render(table, area, buf, &mut state);
        self.cursor.offset = state.offset();

        // Rule in the header's bottom margin.
        buf.hline(
            area.y + 1,
            area.left(),
            area.right(),
            '─',
            Style::new().fg(theme::RULE),
        );
        if self.list.is_empty() {
            let msg = "No executables discovered. Waiting for profiles...";
            buf.set_string(
                area.x + 2,
                area.y + 3,
                msg,
                Style::new().fg(theme::DIM).italic(),
            );
        }
    }

    /// The outcome of the last load or remove, colored by success.
    fn detail(&self) -> Line<'static> {
        let Some(status) = &self.status else {
            return Line::default();
        };
        let is_pending = status.starts_with("Loading") || status.starts_with("Removing");
        let (text, color) = if is_pending {
            (format!("{status}..."), theme::WARNING)
        } else if status.starts_with("Error") {
            (status.clone(), theme::ERROR)
        } else {
            (status.clone(), theme::SUCCESS)
        };
        Line::from(vec![" ".into(), text.fg(color)])
    }
}

const ID_MAX_W: u16 = 34;
const SYMBOLS_W: u16 = 12;

impl ExeEntry {
    /// File ID, name and symbol count. Entries without symbols are dimmed.
    fn table_row(&self, selected: bool, name_w: u16) -> Row<'static> {
        let symbolized = self.num_ranges.is_some();
        let fg = |color: Color| Style::new().fg(color);

        let id = self
            .file_id
            .map_or_else(|| "N/A".into(), |id| id.format_hex());
        let id_fg = if symbolized {
            theme::MUTED_DARK
        } else {
            theme::SUBTLE
        };

        let prefix = if selected { "▸ " } else { "  " };
        let name = truncate(&self.name, (name_w as usize).saturating_sub(3));
        let name_style = match (selected, symbolized) {
            (true, _) => fg(theme::BRIGHT).bold(),
            (false, true) => fg(theme::TEXT),
            (false, false) => fg(theme::MUTED),
        };

        let (symbols, symbols_fg) = match self.num_ranges {
            Some(n) => (format_count(n as u64), theme::SUCCESS),
            None => ("N/A".to_string(), theme::SUBTLE),
        };
        Row::new([
            Cell::new(id).style(fg(id_fg)),
            Cell::new(format!("{prefix}{name}")).style(name_style),
            Cell::new(symbols).style(fg(symbols_fg)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyCode;

    use crate::tui::view::testing::{key, populated_state, screen};

    #[test]
    fn table_lists_discovered_mappings() {
        let mut state = populated_state();
        state.handle_event(key(KeyCode::Tab));
        state.handle_event(key(KeyCode::Tab));
        let shown = screen(&mut state, 100, 20);
        assert!(
            shown.contains("File ID") && shown.contains("Symbols"),
            "{shown}"
        );
        assert!(shown.contains("▸ libc.so.6"), "{shown}");
        assert!(shown.contains("app"), "{shown}");
    }
}

//! Subsecond-offset heatmap: one column per second, one row per slice of
//! that second, colored by how many samples landed there.

mod layout;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style, Stylize},
    text::Line,
    widgets::Widget,
};

use super::TabView;
use super::chrome::{DetailLine, Keys, Placeholder};
use crate::tui::state::FlamescopeTab;
use crate::tui::theme::{self, ColorExt, Gradient};
use layout::{CELL_W, FlamescopeLayout};

impl TabView for FlamescopeTab {
    const KEYS: Keys = &[
        ("[Tab]", " switch "),
        ("[q]", " quit "),
        ("[h/← l/→]", " time "),
        ("[j/↓ k/↑]", " offset "),
        ("[/]", " filter "),
        ("[Esc]", " unfilter "),
        ("[G]", " latest "),
        ("[r]", " reset "),
    ];

    fn render_body(&mut self, area: Rect, buf: &mut Buffer) {
        if self.visible_columns().is_empty() {
            let msg = if self.is_empty() {
                "No profile data yet"
            } else {
                "No data for this thread"
            };
            Placeholder(msg).render(area, buf);
            return;
        }
        let Some(layout) = FlamescopeLayout::new(area) else {
            return;
        };
        // Resolution and horizontal scroll depend on the terminal size.
        self.set_rows(layout.rows);
        self.col.scroll_to_fit(layout.cols);
        Heatmap { tab: self, layout }.render(area, buf);
    }

    fn detail(&self) -> Line<'static> {
        if self.visible_columns().is_empty() {
            return Line::default();
        }
        let mut line = DetailLine::default();
        if let Some(filter) = &self.filter {
            line = line.group([format!(" filtered: {filter} ").fg(theme::ACCENT).bold()]);
        }
        let (sec, ms_start, ms_end) = self.selected_time();
        line.selection(format!("{sec}s + {ms_start}\u{2013}{ms_end}ms"))
            .field("samples", self.selected_value().to_string(), theme::ORANGE)
            .field("peak", self.visible_peak().to_string(), theme::WARNING)
            .field(
                "duration",
                format!("{}s", self.total_seconds()),
                theme::MUTED,
            )
            .into()
    }
}

/// The grid, its row labels and the seconds axis, for an already fitted tab.
struct Heatmap<'a> {
    tab: &'a FlamescopeTab,
    layout: FlamescopeLayout,
}

impl Heatmap<'_> {
    /// Sequential ramp from deep blue (few samples) to pale yellow (peak).
    const RAMP: Gradient = Gradient(&[
        (0.00, (13, 8, 135)),
        (0.25, (126, 3, 168)),
        (0.50, (204, 71, 120)),
        (0.75, (249, 149, 64)),
        (1.00, (252, 255, 164)),
    ]);
    /// Faint fill for cells with no samples, so the grid reads as a grid.
    const EMPTY: Color = Color::Rgb(32, 32, 42);
    /// Seven-eighths block: fills a cell but leaves a thin line on top, which
    /// separates rows without spending a whole text line on borders. Gaps show
    /// the terminal background, like the rest of the UI.
    const GLYPH: &'static str = "▇";
    /// Seconds between labels on the time axis.
    const AXIS_STEP: usize = 5;
    /// Milliseconds between row labels.
    const TICK_MS: usize = 100;

    /// Heat for a per-bucket density relative to the busiest cell. The square
    /// root keeps sparse cells visible next to hot ones.
    fn color(density: f64, peak: f64) -> Color {
        if density > 0.0 {
            Self::RAMP.at((density / peak).sqrt())
        } else {
            Self::EMPTY
        }
    }

    /// Tick at the row holding each 100ms boundary; the cursor row shows its own start.
    fn row_label(&self, buf: &mut Buffer, row: usize) {
        let start = self.tab.row_start_ms(row);
        let end = self.tab.row_start_ms(row + 1);
        let tick = start.div_ceil(Self::TICK_MS) * Self::TICK_MS;
        let label = if row == self.tab.row {
            Some((start, theme::ACCENT))
        } else {
            (tick < end).then_some((tick, theme::DIM))
        };
        if let Some((ms, color)) = label {
            let y = self.layout.row_y(row);
            buf.set_string(
                self.layout.label_x(),
                y,
                format!("{ms:>3}ms "),
                Style::new().fg(color),
            );
        }
    }

    fn row_cells(&self, buf: &mut Buffer, row: usize, peak: f64) {
        let data = self.tab.visible_columns();
        let cell = Self::GLYPH.repeat(CELL_W as usize);
        let visible = self.tab.col.visible(self.layout.cols, data.len());
        for (offset, col) in visible.enumerate() {
            let mut color = Self::color(self.tab.density(&data[col], row), peak);
            if col == self.tab.col.index && row == self.tab.row {
                color = color.lighten(50);
            }
            let x = self.layout.cell_x(offset);
            buf.set_string(x, self.layout.row_y(row), &cell, Style::new().fg(color));
        }
    }

    /// Label every `AXIS_STEP` seconds where the text fits.
    fn axis(&self, buf: &mut Buffer, right: u16) {
        let visible = self
            .tab
            .col
            .visible(self.layout.cols, self.tab.total_seconds());
        let mut free_x = self.layout.cell_x(0);
        for (offset, sec) in visible.enumerate() {
            let x = self.layout.cell_x(offset);
            let label = format!("{sec}s");
            let end = x + label.len() as u16;
            if sec % Self::AXIS_STEP == 0 && x >= free_x && end <= right {
                buf.set_string(x, self.layout.axis_y(), &label, Style::new().fg(theme::DIM));
                free_x = end + 1;
            }
        }
    }
}

impl Widget for Heatmap<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let peak = self.tab.peak_density();
        for row in 0..self.layout.rows {
            self.row_label(buf, row);
            self.row_cells(buf, row, peak);
        }
        self.axis(buf, area.right());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::flamegraph::FlameGraph;
    use crate::tui::event::Event;
    use crate::tui::state::{ActiveTab, State};
    use crate::tui::view::testing::render;

    /// Busy for the first 200ms of every second, idle otherwise.
    fn busy_start_state(seconds: u64) -> State {
        let ts = (0..seconds)
            .flat_map(|s| {
                (0..200)
                    .step_by(4)
                    .map(move |ms| (s * 1000 + ms) * 1_000_000)
            })
            .collect();
        let mut state = State::new("addr".into(), vec![]);
        state.handle_event(Event::ProfileUpdate {
            flamegraph: FlameGraph::new(),
            samples: 0,
            timestamps: HashMap::from([("t".into(), ts)]),
        });
        state.active_tab = ActiveTab::Flamescope;
        state
    }

    #[test]
    fn grid_fills_the_height_with_terminal_background() {
        let mut state = busy_start_state(10);
        let buf = render(&mut state, 100, 60);
        // Body is 57 lines: 56 rows plus the seconds axis.
        let body = Rect::new(0, 2, 100, 57);
        let layout = FlamescopeLayout::new(body).unwrap();
        assert_eq!(layout.rows, 56);
        assert_eq!(layout.axis_y(), body.bottom() - 1);
        let x = layout.cell_x(0);
        for row in 0..layout.rows {
            let y = layout.row_y(row);
            let cell = &buf[(x, y)];
            assert_eq!(cell.symbol(), Heatmap::GLYPH);
            let start = state.fs.row_start_ms(row);
            assert_eq!(
                cell.fg != Heatmap::EMPTY,
                start < 200,
                "row {row} ({start}ms)"
            );
            // Cells and the gaps between them keep the terminal background.
            assert_eq!(cell.bg, Color::Reset);
            assert_eq!(buf[(x + CELL_W, y)].bg, Color::Reset);
        }
        // Each 100ms tick sits on the row containing that boundary.
        let label = |row: usize| -> String {
            (0..6)
                .map(|x| buf[(x, layout.row_y(row))].symbol())
                .collect()
        };
        for tick in (100..1000).step_by(100) {
            let row = (0..layout.rows)
                .find(|&r| state.fs.row_start_ms(r + 1) > tick)
                .unwrap();
            assert_eq!(label(row).trim(), format!("{tick}ms"));
        }
        // The first frame's detail bar already reports the grid's resolution.
        let detail: String = (0..100).map(|x| buf[(x, 1)].symbol()).collect();
        let (_, start, end) = state.fs.selected_time();
        assert!(
            detail.contains(&format!("{start}\u{2013}{end}ms")),
            "{detail}"
        );
        assert_eq!(end - start, 10);
    }

    #[test]
    fn wider_terminals_show_more_seconds() {
        let mut state = busy_start_state(120);
        let mut seconds_shown = |w| {
            render(&mut state, w, 30);
            FlamescopeLayout::new(Rect::new(0, 2, w, 27)).unwrap().cols
        };
        let narrow = seconds_shown(80);
        let wide = seconds_shown(200);
        assert!(wide > narrow * 2, "{narrow} vs {wide}");
        // Auto-scroll keeps the newest second on screen at either width.
        assert_eq!(state.fs.col.index, 119);
        assert!(state.fs.col.visible(wide, 120).contains(&119));
    }
}

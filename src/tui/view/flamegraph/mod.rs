//! Icicle-style flamegraph: the root on top, one row per stack depth.

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
use crate::tui::canvas::BufferExt;
use crate::tui::state::FlamegraphTab;
use crate::tui::text::{format_count, truncate};
use crate::tui::theme::{self, ColorExt};
use crate::tui::widgets::Cursor;
use layout::{FlameLayout, FrameRect};

impl TabView for FlamegraphTab {
    const KEYS: Keys = &[
        ("[Tab]", " switch "),
        ("[q]", " quit "),
        ("[f/Space]", " freeze "),
        ("[j/↓ k/↑]", " depth "),
        ("[h/← l/→]", " frame "),
        ("[Enter]", " zoom "),
        ("[Esc]", " back "),
        ("[/]", " search "),
        ("[r]", " reset "),
    ];

    fn render_body(&mut self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 2 {
            return;
        }
        // Borrow fields, not `self.zoom_root()`, so `scroll_y` stays assignable.
        let root = self.graph.root.follow_path(&self.zoom_path);
        if root.total_value <= 0 {
            Placeholder("No profile data yet").render(area, buf);
            return;
        }

        let layout = FlameLayout::new(root, area.width);
        let frames = layout.frames();
        let max_depth = frames.iter().map(|f| f.depth).max().unwrap_or(0);
        let viewport = area.height as usize;

        // Keep the cursor's depth on screen without scrolling past the deepest row.
        let mut depth = Cursor {
            index: self.cursor_path.len(),
            offset: self.scroll_y,
        };
        self.scroll_y = depth
            .scroll_to_fit(viewport)
            .min(max_depth.saturating_sub(viewport.saturating_sub(1)));

        let painter = Painter {
            area,
            scroll_y: self.scroll_y,
            root_total: root.total_value,
        };
        let cursor = layout.rect_at(&self.cursor_path);
        for frame in &frames {
            let is_cursor = cursor
                .as_ref()
                .is_some_and(|c| c.depth == frame.depth && c.x == frame.x);
            painter.frame(buf, frame, is_cursor);
        }
        for depth in painter.rows().filter(|&d| d <= max_depth) {
            if !frames.iter().any(|f| f.depth == depth) {
                painter.empty_row(buf, depth);
            }
        }
    }

    fn detail(&self) -> Line<'static> {
        let mut line = DetailLine::default();
        if let Some(zoomed) = self.zoom_path.last() {
            line = line.group([format!(" zoomed: {zoomed} ").fg(theme::ACCENT).bold()]);
        }
        let Some(selected) = self.selected() else {
            return line.into();
        };

        let node = selected.node;
        let root_total = self.zoom_root().total_value.max(1) as f64;
        let stat = |v: i64| {
            let pct = v as f64 / root_total * 100.0;
            format!("{} ({pct:.1}%)", format_count(v as u64))
        };
        let swatch = node.kind.color(&node.name, node.self_ratio());
        line.selection(truncate(&node.name, 40))
            .group(["■ ".fg(swatch), node.kind.to_string().fg(theme::MUTED)])
            .field("self", stat(node.self_value), theme::ORANGE)
            .field("total", stat(node.total_value), theme::WARNING)
            .field("depth", selected.depth.to_string(), theme::MUTED)
            .into()
    }
}

/// Draws laid-out frames into the visible depth rows of `area`.
struct Painter {
    area: Rect,
    /// First visible depth.
    scroll_y: usize,
    /// Samples under the zoom root, for percentage labels.
    root_total: i64,
}

impl Painter {
    /// Frames at least this wide show their share of the root.
    const PCT_MIN_WIDTH: u16 = 14;

    /// Depths currently on screen.
    fn rows(&self) -> std::ops::Range<usize> {
        self.scroll_y..self.scroll_y + self.area.height as usize
    }

    fn row_y(&self, depth: usize) -> Option<u16> {
        self.rows()
            .contains(&depth)
            .then(|| self.area.y + (depth - self.scroll_y) as u16)
    }

    /// A colored bar with edge ticks, a centered name and, when wide
    /// enough, its percentage of the root.
    fn frame(&self, buf: &mut Buffer, frame: &FrameRect, is_cursor: bool) {
        let Some(y) = self.row_y(frame.depth) else {
            return;
        };
        let node = frame.node;
        let mut bg = node.kind.color(&node.name, node.self_ratio());
        if is_cursor {
            bg = bg.lighten(45);
        }
        let fg = bg.contrast_fg();
        let on_bg = |color: Color| Style::new().fg(color).bg(bg);

        let x0 = self.area.x + frame.x;
        let x1 = (x0 + frame.width).min(self.area.right());
        buf.fill(
            Rect::new(x0, y, x1.saturating_sub(x0), 1),
            Style::new().bg(bg),
        );
        for x in [x0, x1.saturating_sub(1)] {
            buf.hline(y, x, x + 1, '▏', on_bg(bg.darken(55)));
        }

        let inner = frame.width.saturating_sub(2) as usize;
        if inner >= 3 {
            let name = truncate(&node.name, inner);
            let pad = inner.saturating_sub(name.chars().count()) / 2;
            let style = if is_cursor {
                on_bg(fg).bold()
            } else {
                on_bg(fg)
            };
            buf.set_string(x0 + 1 + pad as u16, y, &name, style);
        }

        if frame.width >= Self::PCT_MIN_WIDTH && self.root_total > 0 {
            let pct = node.total_value as f64 / self.root_total as f64 * 100.0;
            let label = format!("{pct:.1}%");
            let label_x = x0 + frame.width - label.len() as u16 - 2;
            if pct >= 0.1 && label_x > x0 + 2 {
                buf.set_string(label_x, y, &label, on_bg(fg.blend(bg, 0.45)));
            }
        }

        if is_cursor && frame.width >= 3 {
            buf.hline(y, x0 + 1, x0 + 2, '▸', on_bg(Color::White).bold());
        }
    }

    /// Dotted filler for a depth whose frames are all too narrow to draw.
    fn empty_row(&self, buf: &mut Buffer, depth: usize) {
        if let Some(y) = self.row_y(depth) {
            let style = Style::new().fg(theme::GHOST);
            buf.hline(y, self.area.left(), self.area.right(), '·', style);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ratatui::crossterm::event::KeyCode;

    use super::*;
    use crate::flamegraph::FlameGraph;
    use crate::tui::event::Event;
    use crate::tui::state::State;
    use crate::tui::view::testing::{key, populated_state, render, screen};

    /// Background of the rendered cell inside the frame at `path`, where
    /// `path` is relative to the current zoom root.
    fn bg_of(state: &mut State, path: &[&str]) -> Color {
        let buf = render(state, 100, 20);
        let root = state.fg.zoom_root();
        let names: Vec<String> = path.iter().map(|s| s.to_string()).collect();
        let target = root.follow_path(&names);
        let frame = FlameLayout::new(root, 100)
            .frames()
            .into_iter()
            .find(|f| std::ptr::eq(f.node, target))
            .expect("frame is laid out");
        // Header and detail bar sit above the graph.
        let y = 2 + (frame.depth - state.fg.scroll_y) as u16;
        buf[(frame.x + 1, y)].bg
    }

    #[test]
    fn frame_colors_survive_reorder_and_zoom() {
        let mut state = populated_state();
        let before = bg_of(&mut state, &["worker-2", "main"]);

        // worker-2 overtakes worker-1, flipping the top-level order.
        let mut fg = FlameGraph::new();
        fg.add_stack(&["worker-2".into(), "other".into()], 100);
        state.handle_event(Event::ProfileUpdate {
            flamegraph: fg,
            samples: 100,
            timestamps: HashMap::new(),
        });
        assert_eq!(&*state.fg.graph.root.children[0].name, "worker-2");
        assert_eq!(bg_of(&mut state, &["worker-2", "main"]), before);

        state.fg.zoom_path = vec!["worker-2".into()];
        assert_eq!(bg_of(&mut state, &["main"]), before);
    }

    #[test]
    fn detail_bar_follows_cursor() {
        let mut state = populated_state();
        state.handle_event(key(KeyCode::Down));
        state.handle_event(key(KeyCode::Down));
        let screen = screen(&mut state, 100, 20);
        assert!(screen.contains("depth: 2"), "{screen}");
        assert!(screen.contains("▸ main"), "{screen}");
    }

    #[test]
    fn detail_bar_shows_frame_kind() {
        let mut state = populated_state();
        state.handle_event(key(KeyCode::Down));
        assert!(screen(&mut state, 120, 20).contains("■ Unknown"));
    }
}

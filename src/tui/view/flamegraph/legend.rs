//! Runtime color key, toggled with `?` on the flamegraph tab.

use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};

use crate::frame::{FrameKind, Origin, Runtime};
use crate::tui::canvas::BufferExt;
use crate::tui::palette::Paint;
use crate::tui::theme;

/// Anchored to the top-right corner of the area it is rendered into.
pub struct Legend;

impl Legend {
    const WIDTH: u16 = 32;
    const NOTES: [&'static str; 2] = ["brighter = more self time", "lighter = inlined"];
    const SWATCH: &'static str = "   ";
}

impl Widget for Legend {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let runtimes: Vec<Runtime> = Runtime::ALL
            .into_iter()
            .filter(|r| *r != Runtime::Thread)
            .collect();
        let rows = runtimes.len() as u16;
        let height = rows + Self::NOTES.len() as u16 + 5;
        if area.width < Self::WIDTH + 2 || area.height < height {
            return;
        }
        let popup = Rect::new(area.right() - Self::WIDTH - 1, area.y, Self::WIDTH, height);
        buf.popup(popup, " colors ", theme::ACCENT);

        let x = popup.x + 2;
        let header = Style::reset().fg(theme::DIM).italic();
        buf.set_string(x, popup.y + 1, "app runtime", header);
        for (i, runtime) in runtimes.into_iter().enumerate() {
            let y = popup.y + 2 + i as u16;
            let swatch = |origin| Style::reset().bg(FrameKind::new(runtime, origin).swatch());
            buf.set_string(x, y, Self::SWATCH, swatch(Origin::Application));
            if runtime != Runtime::Unknown {
                buf.set_string(x + 4, y, Self::SWATCH, swatch(Origin::Runtime));
            }
            buf.set_string(x + 12, y, runtime.label(), Style::reset().fg(theme::TEXT));
        }
        for (i, note) in Self::NOTES.iter().enumerate() {
            let y = popup.y + 3 + rows + i as u16;
            buf.set_string(x, y, note, Style::reset().fg(theme::MUTED));
        }
    }
}

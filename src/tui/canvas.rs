//! Low-level drawing on a ratatui [`Buffer`], for views that place cells
//! directly instead of composing widgets.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Span,
    widgets::{Block, BorderType, Clear, Widget},
};

use super::theme;

pub trait BufferExt {
    /// Paint every cell in `area` with `style` and a blank glyph.
    fn fill(&mut self, area: Rect, style: Style);
    /// Draw `ch` across `x0..x1` on row `y`.
    fn hline(&mut self, y: u16, x0: u16, x1: u16, ch: char, style: Style);
    /// Write `text` centered horizontally in `area` on row `y`, if that row is inside it.
    fn center(&mut self, area: Rect, y: u16, text: &str, style: Style);
    /// Clear `area` and draw a rounded, titled border around it.
    fn popup(&mut self, area: Rect, title: &str, border: Color);
}

impl BufferExt for Buffer {
    fn fill(&mut self, area: Rect, style: Style) {
        for y in area.top()..area.bottom() {
            self.hline(y, area.left(), area.right(), ' ', style);
        }
    }

    fn hline(&mut self, y: u16, x0: u16, x1: u16, ch: char, style: Style) {
        for x in x0..x1 {
            if let Some(cell) = self.cell_mut((x, y)) {
                cell.set_char(ch);
                cell.set_style(style);
            }
        }
    }

    fn center(&mut self, area: Rect, y: u16, text: &str, style: Style) {
        if y >= area.bottom() {
            return;
        }
        let width = text.chars().count() as u16;
        let x = area.x + area.width.saturating_sub(width) / 2;
        self.set_string(x, y, text, style);
    }

    fn popup(&mut self, area: Rect, title: &str, border: Color) {
        Clear.render(area, self);
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::reset().fg(border))
            .style(Style::reset())
            .title(Span::styled(title, Style::reset().fg(theme::BRIGHT).bold()))
            .render(area, self);
    }
}

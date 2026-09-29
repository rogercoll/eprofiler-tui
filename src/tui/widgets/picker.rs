//! A filterable list prompt: text input on top, matching candidates below.
//!
//! Used for the thread search on the flamegraph and flamescope tabs and for
//! the path prompt on the executables tab. The owner supplies candidates via
//! [`Picker::refresh`] and decides what to do on [`PickerEvent::Submit`].

use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent},
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};

use super::Cursor;
use crate::tui::canvas::BufferExt;
use crate::tui::text::truncate;
use crate::tui::theme;

/// Static presentation and key-hint data for one kind of picker.
pub struct PickerStyle {
    pub title: &'static str,
    /// Shown in the list when the input is empty.
    pub placeholder: &'static str,
    pub border: Color,
    pub width: u16,
    pub max_visible: usize,
    /// Footer hints while this picker is open.
    pub keys: &'static [(&'static str, &'static str)],
}

pub struct Picker {
    pub style: &'static PickerStyle,
    pub input: String,
    pub items: Vec<String>,
    pub cursor: Cursor,
}

/// What the owner must react to after a key press.
#[derive(Debug, PartialEq, Eq)]
pub enum PickerEvent {
    /// The input changed; call [`Picker::refresh`] with new candidates.
    Changed,
    Cancel,
    Submit,
}

impl Picker {
    pub fn new(style: &'static PickerStyle) -> Self {
        Self {
            style,
            input: String::new(),
            items: Vec::new(),
            cursor: Cursor::default(),
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.items.get(self.cursor.index).map(String::as_str)
    }

    /// Replace the candidate list with `names` matching the current input
    /// case-insensitively, and move the cursor to the top.
    pub fn refresh<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        let query = self.input.to_lowercase();
        self.items = names
            .into_iter()
            .filter(|n| query.is_empty() || n.to_lowercase().contains(&query))
            .map(str::to_owned)
            .collect();
        self.cursor.reset();
    }

    /// Replace the candidate list verbatim and move the cursor to the top.
    pub fn set_items(&mut self, items: Vec<String>) {
        self.items = items;
        self.cursor.reset();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<PickerEvent> {
        match key.code {
            KeyCode::Esc => Some(PickerEvent::Cancel),
            KeyCode::Enter => Some(PickerEvent::Submit),
            KeyCode::Backspace => {
                self.input.pop();
                Some(PickerEvent::Changed)
            }
            KeyCode::Tab => {
                self.input = self.selected()?.to_owned();
                Some(PickerEvent::Changed)
            }
            KeyCode::Up => {
                self.cursor.prev();
                None
            }
            KeyCode::Down => {
                self.cursor.next(self.items.len());
                None
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                Some(PickerEvent::Changed)
            }
            _ => None,
        }
    }
}

impl Widget for &Picker {
    /// Draw as a popup anchored to the bottom of `area`.
    fn render(self, area: Rect, buf: &mut Buffer) {
        let width = self.style.width.min(area.width.saturating_sub(4));
        let shown = self.items.len().min(self.style.max_visible);
        let height = (shown as u16 + 4).min(area.height.saturating_sub(2));
        if width < 10 || height < 4 {
            return;
        }
        let popup = Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height),
            width,
            height,
        );
        buf.popup(popup, self.style.title, self.style.border);

        let inner_w = popup.width.saturating_sub(2) as usize;
        let prompt = format!(" / {}█", truncate(&self.input, inner_w.saturating_sub(5)));
        buf.set_string(
            popup.x + 1,
            popup.y + 1,
            truncate(&prompt, inner_w),
            Style::reset().fg(theme::BRIGHT),
        );
        buf.hline(
            popup.y + 2,
            popup.x + 1,
            popup.right() - 1,
            '─',
            Style::reset().fg(theme::FAINT),
        );

        let list_y = popup.y + 3;
        if self.items.is_empty() {
            let hint = if self.input.is_empty() {
                self.style.placeholder
            } else {
                "no matches"
            };
            buf.set_string(
                popup.x + 2,
                list_y,
                hint,
                Style::reset()
                    .fg(theme::SUBTLE)
                    .add_modifier(Modifier::ITALIC),
            );
            return;
        }

        let visible = shown.min(popup.height.saturating_sub(4) as usize);
        let first = self.cursor.index.saturating_sub(visible.saturating_sub(1));
        for (row, (idx, item)) in self
            .items
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
            .enumerate()
        {
            let y = list_y + row as u16;
            let selected = idx == self.cursor.index;
            let is_dir = item.ends_with('/');
            let fg = match (selected, is_dir) {
                (true, _) => theme::BRIGHT,
                (false, true) => theme::ACCENT_LIGHT,
                (false, false) => theme::TEXT,
            };
            let mut style = Style::reset().fg(fg);
            if selected {
                style = style.bg(theme::HIGHLIGHT_BG).add_modifier(Modifier::BOLD);
                buf.fill(
                    Rect::new(popup.x + 1, y, popup.width - 2, 1),
                    Style::reset().bg(theme::HIGHLIGHT_BG),
                );
            }
            let prefix = if selected { " ▸ " } else { "   " };
            let text = format!("{prefix}{}", truncate(item, inner_w.saturating_sub(3)));
            buf.set_string(popup.x + 1, y, text, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    static STYLE: PickerStyle = PickerStyle {
        title: " test ",
        placeholder: "type...",
        border: Color::White,
        width: 40,
        max_visible: 3,
        keys: &[],
    };

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn refresh_filters_case_insensitively() {
        let mut p = Picker::new(&STYLE);
        p.input = "WORK".into();
        p.refresh(["worker-1", "main", "Workqueue"]);
        assert_eq!(p.items, vec!["worker-1", "Workqueue"]);
        assert_eq!(p.selected(), Some("worker-1"));
    }

    #[test]
    fn typing_and_navigation_emit_expected_events() {
        let mut p = Picker::new(&STYLE);
        assert_eq!(
            p.handle_key(key(KeyCode::Char('a'))),
            Some(PickerEvent::Changed)
        );
        assert_eq!(p.input, "a");
        p.set_items(vec!["a1".into(), "a2".into()]);
        assert_eq!(p.handle_key(key(KeyCode::Down)), None);
        assert_eq!(p.selected(), Some("a2"));
        assert_eq!(p.handle_key(key(KeyCode::Tab)), Some(PickerEvent::Changed));
        assert_eq!(p.input, "a2");
        assert_eq!(p.handle_key(key(KeyCode::Enter)), Some(PickerEvent::Submit));
        assert_eq!(p.handle_key(key(KeyCode::Esc)), Some(PickerEvent::Cancel));
    }

    #[test]
    fn tab_without_selection_is_ignored() {
        let mut p = Picker::new(&STYLE);
        assert_eq!(p.handle_key(key(KeyCode::Tab)), None);
    }
}

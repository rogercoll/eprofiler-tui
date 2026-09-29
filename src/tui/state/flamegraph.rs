use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::flamegraph::{FlameGraph, FlameNode};
use crate::tui::theme;
use crate::tui::widgets::{Picker, PickerEvent, PickerStyle};

pub const SEARCH_KEYS: &[(&str, &str)] = &[
    ("[Esc]", " cancel "),
    ("[Enter]", " select "),
    ("[↑↓]", " navigate "),
];

pub static THREAD_PICKER: PickerStyle = PickerStyle {
    title: " thread.name ",
    placeholder: "type to filter threads...",
    border: theme::POPUP_BORDER,
    width: 50,
    max_visible: 3,
    keys: SEARCH_KEYS,
};

/// The frame under the cursor, derived from the graph on demand.
pub struct Selected<'a> {
    pub node: &'a FlameNode,
    pub depth: usize,
}

pub struct FlamegraphTab {
    pub graph: FlameGraph,
    pub frozen: bool,
    pub profiles_received: u64,
    pub samples_received: u64,
    /// First visible depth row.
    pub scroll_y: usize,
    /// Child indices from the zoom root down to the cursor.
    pub cursor_path: Vec<usize>,
    /// Frame names from the real root down to the zoom root.
    pub zoom_path: Vec<String>,
    pub picker: Option<Picker>,
    /// Show the color legend overlay.
    pub show_legend: bool,
}

impl Default for FlamegraphTab {
    fn default() -> Self {
        Self {
            graph: FlameGraph::new(),
            frozen: false,
            profiles_received: 0,
            samples_received: 0,
            scroll_y: 0,
            cursor_path: Vec::new(),
            zoom_path: Vec::new(),
            picker: None,
            show_legend: false,
        }
    }
}

impl FlamegraphTab {
    pub fn merge(&mut self, new_fg: FlameGraph, samples: u64) {
        if self.frozen {
            return;
        }
        self.graph.root.merge(new_fg.root);
        self.graph.root.sort_recursive();
        self.profiles_received += 1;
        self.samples_received += samples;
    }

    pub fn zoom_root(&self) -> &FlameNode {
        self.graph.root.follow_path(&self.zoom_path)
    }

    pub fn selected(&self) -> Option<Selected<'_>> {
        let node = self.zoom_root().descend(&self.cursor_path)?;
        Some(Selected {
            node,
            depth: self.cursor_path.len(),
        })
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.picker.is_some() {
            return self.handle_picker_key(key);
        }
        match key.code {
            KeyCode::Char('f') | KeyCode::Char(' ') => self.frozen = !self.frozen,
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::Left | KeyCode::Char('h') => self.move_left(),
            KeyCode::Right | KeyCode::Char('l') => self.move_right(),
            KeyCode::Enter => self.zoom_in(),
            KeyCode::Esc | KeyCode::Backspace => self.zoom_out(),
            KeyCode::Char('r') => self.reset(),
            KeyCode::Char('/') => self.open_search(),
            KeyCode::Char('?') => self.show_legend = !self.show_legend,
            _ => {}
        };
    }

    fn open_search(&mut self) {
        let mut picker = Picker::new(&THREAD_PICKER);
        picker.refresh(self.thread_names());
        self.picker = Some(picker);
    }

    fn thread_names(&self) -> impl Iterator<Item = &str> {
        self.graph.root.children.iter().map(|c| c.name.as_str())
    }

    fn handle_picker_key(&mut self, key: KeyEvent) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        match picker.handle_key(key) {
            Some(PickerEvent::Changed) => {
                let names = self.graph.root.children.iter().map(|c| c.name.as_str());
                picker.refresh(names);
            }
            Some(PickerEvent::Cancel) => self.picker = None,
            Some(PickerEvent::Submit) => {
                let picked = self
                    .picker
                    .take()
                    .and_then(|p| p.selected().map(str::to_owned));
                if let Some(name) = picked {
                    self.zoom_path = vec![name];
                    self.reset_cursor();
                }
            }
            None => {}
        }
    }

    fn cursor_node(&self) -> &FlameNode {
        self.zoom_root().follow_indices(&self.cursor_path)
    }

    fn move_down(&mut self) {
        if !self.cursor_node().children.is_empty() {
            self.cursor_path.push(0);
        }
    }

    fn move_up(&mut self) {
        self.cursor_path.pop();
    }

    fn move_left(&mut self) {
        if let Some(last) = self.cursor_path.last_mut() {
            *last = last.saturating_sub(1);
        }
    }

    fn move_right(&mut self) {
        let Some(depth) = self.cursor_path.len().checked_sub(1) else {
            return;
        };
        let siblings = self
            .zoom_root()
            .follow_indices(&self.cursor_path[..depth])
            .children
            .len();
        if let Some(last) = self.cursor_path.last_mut()
            && *last + 1 < siblings
        {
            *last += 1;
        }
    }

    fn zoom_in(&mut self) {
        if self.cursor_path.is_empty() {
            return;
        }
        let names = self.zoom_root().names_along(&self.cursor_path);
        self.zoom_path.extend(names);
        self.reset_cursor();
    }

    fn zoom_out(&mut self) {
        if self.zoom_path.pop().is_some() {
            self.reset_cursor();
        }
    }

    fn reset_cursor(&mut self) {
        self.cursor_path.clear();
        self.scroll_y = 0;
    }

    fn reset(&mut self) {
        *self = Self {
            frozen: self.frozen,
            show_legend: self.show_legend,
            ..Self::default()
        };
    }
}

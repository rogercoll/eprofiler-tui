use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::flamegraph::{FlameGraph, FlameNode, get_node, get_zoom_node};

#[derive(Default)]
pub struct Selection {
    pub name: String,
    pub self_value: i64,
    pub total_value: i64,
    pub pct: f64,
    pub depth: usize,
}

#[derive(Default)]
pub struct SearchOverlay {
    pub active: bool,
    pub input: String,
    pub matches: Vec<(String, usize)>,
    pub cursor: usize,
}

impl SearchOverlay {
    pub fn open(&mut self) {
        *self = Self {
            active: true,
            ..Default::default()
        };
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }
}

pub struct FlamegraphTab {
    pub graph: FlameGraph,
    pub frozen: bool,
    pub profiles_received: u64,
    pub samples_received: u64,
    pub scroll_y: usize,
    pub cursor_path: Vec<usize>,
    pub zoom_path: Vec<String>,
    pub selection: Selection,
    pub search: SearchOverlay,
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
            selection: Selection::default(),
            search: SearchOverlay::default(),
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

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.search.active {
            return self.handle_search_key(key);
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
            KeyCode::Char('/') => {
                self.search.open();
                self.refresh_search();
            }
            _ => {}
        };
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.search.close(),
            KeyCode::Enter => {
                if let Some((name, _)) = self.search.matches.get(self.search.cursor).cloned() {
                    self.zoom_path = vec![name];
                    self.cursor_path.clear();
                    self.scroll_y = 0;
                }
                self.search.close();
            }
            KeyCode::Backspace => {
                self.search.input.pop();
                self.search.cursor = 0;
                self.refresh_search();
            }
            KeyCode::Up => self.search.cursor = self.search.cursor.saturating_sub(1),
            KeyCode::Down => {
                if self.search.cursor + 1 < self.search.matches.len() {
                    self.search.cursor += 1;
                }
            }
            KeyCode::Char(c) => {
                self.search.input.push(c);
                self.search.cursor = 0;
                self.refresh_search();
            }
            _ => {}
        }
    }

    fn refresh_search(&mut self) {
        let query = self.search.input.to_lowercase();
        self.search.matches = self
            .graph
            .root
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| query.is_empty() || c.name.to_lowercase().contains(&query))
            .map(|(i, c)| (c.name.clone(), i))
            .collect();
    }

    fn move_down(&mut self) {
        let has_children = {
            let zr = get_zoom_node(&self.graph.root, &self.zoom_path);
            !get_node(zr, &self.cursor_path).children.is_empty()
        };
        if has_children {
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
        let sibling_count = {
            if self.cursor_path.is_empty() {
                return;
            }
            let zr = get_zoom_node(&self.graph.root, &self.zoom_path);
            get_node(zr, &self.cursor_path[..self.cursor_path.len() - 1])
                .children
                .len()
        };
        if let Some(last) = self.cursor_path.last_mut() {
            if *last + 1 < sibling_count {
                *last += 1;
            }
        }
    }

    fn zoom_in(&mut self) {
        if self.cursor_path.is_empty() {
            return;
        }
        let names = {
            let zr = get_zoom_node(&self.graph.root, &self.zoom_path);
            collect_path_names(zr, &self.cursor_path)
        };
        self.zoom_path.extend(names);
        self.cursor_path.clear();
        self.scroll_y = 0;
    }

    fn zoom_out(&mut self) {
        if self.zoom_path.pop().is_some() {
            self.cursor_path.clear();
            self.scroll_y = 0;
        }
    }

    fn reset(&mut self) {
        self.graph = FlameGraph::new();
        self.profiles_received = 0;
        self.samples_received = 0;
        self.zoom_path.clear();
        self.cursor_path.clear();
        self.scroll_y = 0;
    }
}

fn collect_path_names(root: &FlameNode, index_path: &[usize]) -> Vec<String> {
    index_path
        .iter()
        .scan(root, |node, &idx| {
            let child = node.children.get(idx)?;
            *node = child;
            Some(child.name.clone())
        })
        .collect()
}

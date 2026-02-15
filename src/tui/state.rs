use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::flamegraph::{FlameGraph, FlameNode, get_node, get_zoom_node};

pub struct State {
    pub running: bool,
    pub listen_addr: String,
    pub frozen: bool,
    pub flamegraph: FlameGraph,
    pub profiles_received: u64,
    pub samples_received: u64,
    pub scroll_y: usize,
    pub cursor_path: Vec<usize>,
    pub zoom_path: Vec<String>,
    pub selected_name: String,
    pub selected_self: i64,
    pub selected_total: i64,
    pub selected_pct: f64,
    pub selected_depth: usize,
    pub search_active: bool,
    pub search_input: String,
    pub search_matches: Vec<(String, usize)>,
    pub search_cursor: usize,
}

impl State {
    pub fn new(listen_addr: String) -> Self {
        Self {
            running: true,
            listen_addr,
            frozen: false,
            flamegraph: FlameGraph::new(),
            profiles_received: 0,
            samples_received: 0,
            scroll_y: 0,
            cursor_path: Vec::new(),
            zoom_path: Vec::new(),
            selected_name: String::new(),
            selected_self: 0,
            selected_total: 0,
            selected_pct: 0.0,
            selected_depth: 0,
            search_active: false,
            search_input: String::new(),
            search_matches: Vec::new(),
            search_cursor: 0,
        }
    }

    pub fn merge_flamegraph(&mut self, new_fg: FlameGraph, samples: u64) {
        if self.frozen {
            return;
        }
        self.flamegraph.root.merge(new_fg.root);
        self.flamegraph.root.sort_recursive();
        self.profiles_received += 1;
        self.samples_received += samples;
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.running = false;
            return;
        }

        if self.search_active {
            self.handle_search_key(key);
            return;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.running = false,
            KeyCode::Char('f') | KeyCode::Char(' ') => self.frozen = !self.frozen,
            KeyCode::Down | KeyCode::Char('j') => self.move_down_depth(),
            KeyCode::Up | KeyCode::Char('k') => self.move_up_depth(),
            KeyCode::Left | KeyCode::Char('h') => self.move_left(),
            KeyCode::Right | KeyCode::Char('l') => self.move_right(),
            KeyCode::Enter => self.zoom_in(),
            KeyCode::Esc | KeyCode::Backspace => self.zoom_out(),
            KeyCode::Char('r') => self.reset(),
            KeyCode::Char('/') => self.open_search(),
            _ => {}
        }
    }

    fn open_search(&mut self) {
        self.search_active = true;
        self.search_input.clear();
        self.search_cursor = 0;
        self.update_search_matches();
    }

    fn close_search(&mut self) {
        self.search_active = false;
        self.search_input.clear();
        self.search_matches.clear();
        self.search_cursor = 0;
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.close_search(),
            KeyCode::Enter => self.confirm_search(),
            KeyCode::Backspace => {
                self.search_input.pop();
                self.search_cursor = 0;
                self.update_search_matches();
            }
            KeyCode::Up => {
                if self.search_cursor > 0 {
                    self.search_cursor -= 1;
                }
            }
            KeyCode::Down => {
                if !self.search_matches.is_empty()
                    && self.search_cursor + 1 < self.search_matches.len()
                {
                    self.search_cursor += 1;
                }
            }
            KeyCode::Char(c) => {
                self.search_input.push(c);
                self.search_cursor = 0;
                self.update_search_matches();
            }
            _ => {}
        }
    }

    fn update_search_matches(&mut self) {
        let query = self.search_input.to_lowercase();
        self.search_matches = self
            .flamegraph
            .root
            .children
            .iter()
            .enumerate()
            .filter(|(_, child)| query.is_empty() || child.name.to_lowercase().contains(&query))
            .map(|(idx, child)| (child.name.clone(), idx))
            .collect();
    }

    fn confirm_search(&mut self) {
        if let Some((name, _idx)) = self.search_matches.get(self.search_cursor).cloned() {
            self.zoom_path.clear();
            self.zoom_path.push(name);
            self.cursor_path.clear();
            self.scroll_y = 0;
        }
        self.close_search();
    }

    fn move_down_depth(&mut self) {
        let has_children = {
            let zr = get_zoom_node(&self.flamegraph.root, &self.zoom_path);
            let node = get_node(zr, &self.cursor_path);
            !node.children.is_empty()
        };
        if has_children {
            self.cursor_path.push(0);
        }
    }

    fn move_up_depth(&mut self) {
        self.cursor_path.pop();
    }

    fn move_left(&mut self) {
        if let Some(last) = self.cursor_path.last_mut() {
            *last = last.saturating_sub(1);
        }
    }

    fn move_right(&mut self) {
        let num_siblings = {
            let zr = get_zoom_node(&self.flamegraph.root, &self.zoom_path);
            if self.cursor_path.is_empty() {
                return;
            }
            let parent = get_node(zr, &self.cursor_path[..self.cursor_path.len() - 1]);
            parent.children.len()
        };
        if let Some(last) = self.cursor_path.last_mut() {
            if *last + 1 < num_siblings {
                *last += 1;
            }
        }
    }

    fn zoom_in(&mut self) {
        if self.cursor_path.is_empty() {
            return;
        }
        let new_names = {
            let zr = get_zoom_node(&self.flamegraph.root, &self.zoom_path);
            collect_path_names(zr, &self.cursor_path)
        };
        self.zoom_path.extend(new_names);
        self.cursor_path.clear();
        self.scroll_y = 0;
    }

    fn zoom_out(&mut self) {
        if !self.zoom_path.is_empty() {
            self.zoom_path.pop();
            self.cursor_path.clear();
            self.scroll_y = 0;
        }
    }

    fn reset(&mut self) {
        self.flamegraph = FlameGraph::new();
        self.profiles_received = 0;
        self.samples_received = 0;
        self.zoom_path.clear();
        self.cursor_path.clear();
        self.scroll_y = 0;
    }
}

fn collect_path_names(root: &FlameNode, index_path: &[usize]) -> Vec<String> {
    let mut names = Vec::new();
    let mut node = root;
    for &idx in index_path {
        if idx < node.children.len() {
            names.push(node.children[idx].name.clone());
            node = &node.children[idx];
        }
    }
    names
}

use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::flamegraph::{FlameGraph, FlameNode, get_node, get_zoom_node};
use crate::storage::{ExecutableInfo, FileId};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Flamegraph,
    Executables,
}

#[derive(Clone)]
pub struct ExeEntry {
    pub name: String,
    pub file_id: Option<FileId>,
    pub num_ranges: Option<u32>,
}

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

    pub active_tab: ActiveTab,
    pub exe_cursor: usize,
    pub exe_scroll: usize,
    pub exe_input: String,
    pub exe_input_active: bool,
    pub exe_input_target: Option<String>,
    pub exe_completions: Vec<String>,
    pub exe_completion_cursor: usize,
    pub exe_list: Vec<ExeEntry>,
    pub exe_status: Option<String>,
}

pub enum Action {
    LoadSymbols(PathBuf, Option<String>),
    RemoveSymbols(String, FileId),
    None,
}

impl State {
    pub fn new(listen_addr: String, initial_exes: Vec<ExecutableInfo>) -> Self {
        let exe_list: Vec<ExeEntry> = initial_exes
            .into_iter()
            .map(|info| ExeEntry {
                name: info.file_name,
                file_id: Some(info.file_id),
                num_ranges: Some(info.num_ranges),
            })
            .collect();

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

            active_tab: ActiveTab::Flamegraph,
            exe_cursor: 0,
            exe_scroll: 0,
            exe_input: String::new(),
            exe_input_active: false,
            exe_input_target: None,
            exe_completions: Vec::new(),
            exe_completion_cursor: 0,
            exe_list,
            exe_status: None,
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

    pub fn merge_discovered_mappings(&mut self, names: Vec<String>) {
        for name in names {
            if !self.exe_list.iter().any(|e| e.name == name) {
                self.exe_list.push(ExeEntry {
                    name,
                    file_id: None,
                    num_ranges: None,
                });
            }
        }
        self.sort_exe_list();
    }

    pub fn update_symbolized(&mut self, target_name: String, info: ExecutableInfo) {
        if let Some(entry) = self.exe_list.iter_mut().find(|e| e.name == target_name) {
            entry.file_id = Some(info.file_id);
            entry.num_ranges = Some(info.num_ranges);
        } else {
            self.exe_list.push(ExeEntry {
                name: info.file_name,
                file_id: Some(info.file_id),
                num_ranges: Some(info.num_ranges),
            });
        }
        self.sort_exe_list();
    }

    pub fn clear_symbols(&mut self, name: &str) {
        if let Some(entry) = self.exe_list.iter_mut().find(|e| e.name == name) {
            entry.file_id = None;
            entry.num_ranges = None;
        }
        self.sort_exe_list();
    }

    pub fn sort_exe_list(&mut self) {
        let current_name = self.exe_list.get(self.exe_cursor).map(|e| e.name.clone());

        self.exe_list.sort_by(|a, b| {
            let a_sym = a.num_ranges.is_some();
            let b_sym = b.num_ranges.is_some();
            b_sym.cmp(&a_sym).then(a.name.cmp(&b.name))
        });

        if let Some(name) = current_name {
            if let Some(pos) = self.exe_list.iter().position(|e| e.name == name) {
                self.exe_cursor = pos;
            }
        }
        self.clamp_exe_cursor();
    }

    fn clamp_exe_cursor(&mut self) {
        if self.exe_list.is_empty() {
            self.exe_cursor = 0;
            self.exe_scroll = 0;
        } else {
            self.exe_cursor = self.exe_cursor.min(self.exe_list.len() - 1);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.running = false;
            return Action::None;
        }

        if key.code == KeyCode::Tab && !self.exe_input_active && !self.search_active {
            self.active_tab = match self.active_tab {
                ActiveTab::Flamegraph => ActiveTab::Executables,
                ActiveTab::Executables => ActiveTab::Flamegraph,
            };
            return Action::None;
        }

        match self.active_tab {
            ActiveTab::Flamegraph => self.handle_flamegraph_key(key),
            ActiveTab::Executables => self.handle_executables_key(key),
        }
    }

    fn handle_flamegraph_key(&mut self, key: KeyEvent) -> Action {
        if self.search_active {
            self.handle_search_key(key);
            return Action::None;
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
        };
        return Action::None;
    }

    fn handle_executables_key(&mut self, key: KeyEvent) -> Action {
        if self.exe_input_active {
            return self.handle_exe_input_key(key);
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.running = false,
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.exe_list.is_empty() && self.exe_cursor + 1 < self.exe_list.len() {
                    self.exe_cursor += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.exe_cursor > 0 {
                    self.exe_cursor -= 1;
                }
            }
            KeyCode::Enter => {
                if let Some(entry) = self.exe_list.get(self.exe_cursor) {
                    let target = entry.name.clone();
                    self.open_exe_input(Some(target));
                }
            }
            KeyCode::Char('r') => {
                if let Some(entry) = self.exe_list.get(self.exe_cursor) {
                    if let Some(file_id) = entry.file_id {
                        let name = entry.name.clone();
                        return Action::RemoveSymbols(name, file_id);
                    }
                }
            }
            KeyCode::Char('/') => self.open_exe_input(None),
            _ => {}
        };
        Action::None
    }

    fn open_exe_input(&mut self, target: Option<String>) {
        self.exe_input_active = true;
        self.exe_input.clear();
        self.exe_input_target = target;
        self.exe_completions.clear();
        self.exe_completion_cursor = 0;
    }

    fn close_exe_input(&mut self) {
        self.exe_input_active = false;
        self.exe_input.clear();
        self.exe_input_target = None;
        self.exe_completions.clear();
        self.exe_completion_cursor = 0;
    }

    fn handle_exe_input_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => self.close_exe_input(),
            KeyCode::Enter => {
                let path = self.exe_input.trim().to_string();
                if !path.is_empty() {
                    let target = self.exe_input_target.take();
                    let display = target.as_deref().unwrap_or(&path);
                    self.exe_status = Some(format!("Loading {}", display));
                    self.close_exe_input();
                    return Action::LoadSymbols(PathBuf::from(&path), target);
                }
                self.close_exe_input();
            }
            KeyCode::Backspace => {
                self.exe_input.pop();
                self.refresh_completions();
            }
            KeyCode::Tab => self.apply_completion(),
            KeyCode::Up => {
                if self.exe_completion_cursor > 0 {
                    self.exe_completion_cursor -= 1;
                }
            }
            KeyCode::Down => {
                if !self.exe_completions.is_empty()
                    && self.exe_completion_cursor + 1 < self.exe_completions.len()
                {
                    self.exe_completion_cursor += 1;
                }
            }
            KeyCode::Char(c) => {
                self.exe_input.push(c);
                self.refresh_completions();
            }
            _ => {}
        };
        Action::None
    }

    fn refresh_completions(&mut self) {
        self.exe_completions = compute_path_completions(&self.exe_input);
        self.exe_completion_cursor = 0;
    }

    fn apply_completion(&mut self) {
        if let Some(selected) = self
            .exe_completions
            .get(self.exe_completion_cursor)
            .cloned()
        {
            self.exe_input = selected;
            self.refresh_completions();
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

fn compute_path_completions(input: &str) -> Vec<String> {
    if input.is_empty() {
        return list_dir_entries(Path::new("."), "");
    }

    let path = Path::new(input);

    if input.ends_with('/') {
        return list_dir_entries(path, "");
    }

    let parent = path.parent().unwrap_or(Path::new("."));
    let prefix = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    list_dir_entries(parent, &prefix)
}

fn list_dir_entries(dir: &Path, prefix: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };

    let prefix_lower = prefix.to_lowercase();
    let mut results = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !prefix_lower.is_empty() && !name.to_lowercase().starts_with(&prefix_lower) {
            continue;
        }
        if name.starts_with('.') && prefix.is_empty() {
            continue;
        }
        let full = entry.path().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            results.push(format!("{}/", full));
        } else {
            results.push(full);
        }
    }
    results.sort();
    results
}

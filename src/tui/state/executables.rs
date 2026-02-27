use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::Action;
use crate::storage::{ExecutableInfo, FileId};

#[derive(Clone)]
pub struct ExeEntry {
    pub name: String,
    pub file_id: Option<FileId>,
    pub num_ranges: Option<u32>,
}

#[derive(Default)]
pub struct PathInput {
    pub active: bool,
    pub input: String,
    pub target: Option<String>,
    pub completions: Vec<String>,
    pub completion_cursor: usize,
}

impl PathInput {
    pub fn open(&mut self, target: Option<String>) {
        *self = Self {
            active: true,
            target,
            ..Default::default()
        };
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }

    fn refresh_completions(&mut self) {
        self.completions = compute_path_completions(&self.input);
        self.completion_cursor = 0;
    }

    fn apply_completion(&mut self) {
        if let Some(selected) = self.completions.get(self.completion_cursor).cloned() {
            self.input = selected;
            self.refresh_completions();
        }
    }
}

pub struct ExecutablesTab {
    pub cursor: usize,
    pub scroll: usize,
    pub list: Vec<ExeEntry>,
    pub status: Option<String>,
    pub path_input: PathInput,
}

impl From<Vec<ExecutableInfo>> for ExecutablesTab {
    fn from(exes: Vec<ExecutableInfo>) -> Self {
        Self {
            list: exes
                .into_iter()
                .map(|info| ExeEntry {
                    name: info.file_name,
                    file_id: Some(info.file_id),
                    num_ranges: Some(info.num_ranges),
                })
                .collect(),
            cursor: 0,
            scroll: 0,
            status: None,
            path_input: PathInput::default(),
        }
    }
}

impl ExecutablesTab {
    pub fn merge_discovered_mappings(&mut self, names: Vec<String>) {
        for name in names {
            if !self.list.iter().any(|e| e.name == name) {
                self.list.push(ExeEntry {
                    name,
                    file_id: None,
                    num_ranges: None,
                });
            }
        }
        self.sort_list();
    }

    pub fn update_symbolized(&mut self, target_name: String, info: ExecutableInfo) {
        if let Some(entry) = self.list.iter_mut().find(|e| e.name == target_name) {
            entry.file_id = Some(info.file_id);
            entry.num_ranges = Some(info.num_ranges);
        } else {
            self.list.push(ExeEntry {
                name: info.file_name,
                file_id: Some(info.file_id),
                num_ranges: Some(info.num_ranges),
            });
        }
        self.sort_list();
    }

    pub fn clear_symbols(&mut self, name: &str) {
        if let Some(entry) = self.list.iter_mut().find(|e| e.name == name) {
            entry.file_id = None;
            entry.num_ranges = None;
        }
        self.sort_list();
    }

    fn sort_list(&mut self) {
        let current_name = self.list.get(self.cursor).map(|e| e.name.clone());
        self.list.sort_by(|a, b| {
            let a_sym = a.num_ranges.is_some();
            let b_sym = b.num_ranges.is_some();
            b_sym.cmp(&a_sym).then(a.name.cmp(&b.name))
        });
        if let Some(name) = current_name {
            self.cursor = self
                .list
                .iter()
                .position(|e| e.name == name)
                .unwrap_or(self.cursor);
        }
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        if self.list.is_empty() {
            self.cursor = 0;
            self.scroll = 0;
        } else {
            self.cursor = self.cursor.min(self.list.len() - 1);
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Action {
        if self.path_input.active {
            return self.handle_path_input_key(key);
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < self.list.len() {
                    self.cursor += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some(entry) = self.list.get(self.cursor) {
                    self.path_input.open(Some(entry.name.clone()));
                }
            }
            KeyCode::Char('r') => {
                if let Some(entry) = self.list.get(self.cursor) {
                    if let Some(file_id) = entry.file_id {
                        return Action::RemoveSymbols(entry.name.clone(), file_id);
                    }
                }
            }
            KeyCode::Char('/') => self.path_input.open(None),
            _ => {}
        };
        Action::None
    }

    fn handle_path_input_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => self.path_input.close(),
            KeyCode::Enter => {
                let path = self.path_input.input.trim().to_string();
                if !path.is_empty() {
                    let target = self.path_input.target.take();
                    let display = target.as_deref().unwrap_or(&path);
                    self.status = Some(format!("Loading {}", display));
                    self.path_input.close();
                    return Action::LoadSymbols(PathBuf::from(&path), target);
                }
                self.path_input.close();
            }
            KeyCode::Backspace => {
                self.path_input.input.pop();
                self.path_input.refresh_completions();
            }
            KeyCode::Tab => self.path_input.apply_completion(),
            KeyCode::Up => {
                self.path_input.completion_cursor =
                    self.path_input.completion_cursor.saturating_sub(1);
            }
            KeyCode::Down => {
                if self.path_input.completion_cursor + 1 < self.path_input.completions.len() {
                    self.path_input.completion_cursor += 1;
                }
            }
            KeyCode::Char(c) => {
                self.path_input.input.push(c);
                self.path_input.refresh_completions();
            }
            _ => {}
        };
        Action::None
    }
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
    let mut results: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !prefix_lower.is_empty() && !name.to_lowercase().starts_with(&prefix_lower) {
                return None;
            }
            if name.starts_with('.') && prefix.is_empty() {
                return None;
            }
            let full = entry.path().to_string_lossy().into_owned();
            Some(if entry.path().is_dir() {
                format!("{full}/")
            } else {
                full
            })
        })
        .collect();
    results.sort();
    results
}

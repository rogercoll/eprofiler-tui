use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::Action;
use crate::storage::{ExecutableInfo, FileId};
use crate::tui::theme;
use crate::tui::widgets::{Cursor, Picker, PickerEvent, PickerStyle};

pub const PATH_KEYS: &[(&str, &str)] = &[
    ("[Esc]", " cancel "),
    ("[Tab]", " complete "),
    ("[↑↓]", " navigate "),
    ("[Enter]", " load "),
];

static PATH_PICKER: PickerStyle = PickerStyle {
    title: " executable path ",
    placeholder: "type a path...",
    border: theme::ACCENT,
    width: 60,
    max_visible: 5,
    keys: PATH_KEYS,
};

#[derive(Clone, Default)]
pub struct ExeEntry {
    pub name: String,
    pub file_id: Option<FileId>,
    pub num_ranges: Option<u32>,
}

impl From<ExecutableInfo> for ExeEntry {
    fn from(info: ExecutableInfo) -> Self {
        Self {
            name: info.file_name,
            file_id: Some(info.file_id),
            num_ranges: Some(info.num_ranges),
        }
    }
}

impl ExeEntry {
    fn symbolized(&self) -> bool {
        self.num_ranges.is_some()
    }
}

/// Path prompt plus the discovered mapping it will symbolize, if any.
struct PathPrompt {
    picker: Picker,
    target: Option<String>,
}

impl PathPrompt {
    /// Filesystem entries completing `input`, directories suffixed with `/`.
    fn completions(input: &str) -> Vec<String> {
        if input.is_empty() {
            return Self::entries(Path::new("."), "");
        }
        let path = Path::new(input);
        if input.ends_with('/') {
            return Self::entries(path, "");
        }
        let parent = path.parent().unwrap_or(Path::new("."));
        let prefix = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self::entries(parent, &prefix)
    }

    /// Entries of `dir` whose names start with `prefix`, case-insensitively,
    /// sorted. Hidden entries are listed only once the prefix is typed.
    fn entries(dir: &Path, prefix: &str) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let prefix_lower = prefix.to_lowercase();
        let mut results: Vec<String> = entries
            .flatten()
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                let hidden = name.starts_with('.') && prefix.is_empty();
                !hidden && name.starts_with(&prefix_lower)
            })
            .map(|entry| {
                let full = entry.path().to_string_lossy().into_owned();
                if entry.path().is_dir() {
                    format!("{full}/")
                } else {
                    full
                }
            })
            .collect();
        results.sort();
        results
    }
}

pub struct ExecutablesTab {
    pub cursor: Cursor,
    pub list: Vec<ExeEntry>,
    pub status: Option<String>,
    prompt: Option<PathPrompt>,
}

impl From<Vec<ExecutableInfo>> for ExecutablesTab {
    fn from(exes: Vec<ExecutableInfo>) -> Self {
        Self {
            list: exes.into_iter().map(ExeEntry::from).collect(),
            cursor: Cursor::default(),
            status: None,
            prompt: None,
        }
    }
}

impl ExecutablesTab {
    pub fn picker(&self) -> Option<&Picker> {
        self.prompt.as_ref().map(|p| &p.picker)
    }

    pub fn merge_discovered_mappings(&mut self, names: Vec<String>) {
        for name in names {
            if !self.list.iter().any(|e| e.name == name) {
                self.list.push(ExeEntry {
                    name,
                    ..Default::default()
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
            self.list.push(info.into());
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

    pub fn handle_symbols_loaded(
        &mut self,
        target_name: String,
        info: crate::error::Result<ExecutableInfo>,
    ) {
        match info {
            Ok(info) => {
                self.status = Some(format!(
                    "Loaded {} symbols for {}",
                    info.num_ranges, target_name
                ));
                self.update_symbolized(target_name, info);
            }
            Err(err) => {
                self.status = Some(format!("Error loading {target_name}: {err}"));
            }
        }
    }

    pub fn handle_symbols_removed(&mut self, name: String, error: Option<crate::error::Error>) {
        self.status = Some(
            error
                .map(|err| format!("Error removing {name}: {err}"))
                .unwrap_or_else(|| format!("Removed symbols for {name}")),
        );
        self.clear_symbols(&name);
    }

    /// Symbolized entries first, then by name; the cursor follows its entry.
    fn sort_list(&mut self) {
        let current_name = self.list.get(self.cursor.index).map(|e| e.name.clone());
        self.list.sort_by(|a, b| {
            b.symbolized()
                .cmp(&a.symbolized())
                .then(a.name.cmp(&b.name))
        });
        if let Some(pos) = current_name.and_then(|n| self.list.iter().position(|e| e.name == n)) {
            self.cursor.index = pos;
        }
        self.cursor.clamp(self.list.len());
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if self.prompt.is_some() {
            return self.handle_prompt_key(key);
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.cursor.next(self.list.len()),
            KeyCode::Up | KeyCode::Char('k') => self.cursor.prev(),
            KeyCode::Enter => {
                let target = self.list.get(self.cursor.index).map(|e| e.name.clone());
                if target.is_some() {
                    self.open_prompt(target);
                }
            }
            KeyCode::Char('r') => {
                let entry = self.list.get(self.cursor.index)?;
                let (name, file_id) = (entry.name.clone(), entry.file_id?);
                self.status = Some(format!("Removing {name}"));
                return Some(Action::RemoveSymbols(name, file_id));
            }
            KeyCode::Char('/') => self.open_prompt(None),
            _ => {}
        };
        None
    }

    fn open_prompt(&mut self, target: Option<String>) {
        let mut picker = Picker::new(&PATH_PICKER);
        picker.set_items(PathPrompt::completions(""));
        self.prompt = Some(PathPrompt { picker, target });
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) -> Option<Action> {
        let prompt = self.prompt.as_mut()?;
        match prompt.picker.handle_key(key)? {
            PickerEvent::Changed => {
                let items = PathPrompt::completions(&prompt.picker.input);
                prompt.picker.set_items(items);
                None
            }
            PickerEvent::Cancel => {
                self.prompt = None;
                None
            }
            PickerEvent::Submit => {
                let PathPrompt { picker, target } = self.prompt.take()?;
                let path = picker.input.trim();
                if path.is_empty() {
                    return None;
                }
                self.status = Some(format!("Loading {}", target.as_deref().unwrap_or(path)));
                Some(Action::LoadSymbols(PathBuf::from(path), target))
            }
        }
    }
}

use std::collections::HashMap;

use ratatui::crossterm::event::{KeyCode, KeyEvent};

const SUBSECOND_ROWS: usize = 10;
const NS_PER_SEC: u64 = 1_000_000_000;
const NS_PER_ROW: u64 = NS_PER_SEC / SUBSECOND_ROWS as u64;

#[derive(Default)]
pub struct ThreadSearch {
    pub active: bool,
    pub input: String,
    pub matches: Vec<String>,
    pub cursor: usize,
}

impl ThreadSearch {
    fn open(&mut self) {
        *self = Self {
            active: true,
            ..Default::default()
        };
    }

    fn close(&mut self) {
        *self = Self::default();
    }
}

pub struct FlamescopeTab {
    epoch_ns: Option<u64>,
    columns: Vec<[u64; SUBSECOND_ROWS]>,
    threads: HashMap<String, Vec<[u64; SUBSECOND_ROWS]>>,
    thread_names: Vec<String>,
    pub filter: Option<String>,
    pub search: ThreadSearch,
    pub auto_scroll: bool,
    pub scroll_x: usize,
    pub cursor_col: usize,
    pub cursor_row: usize,
}

impl Default for FlamescopeTab {
    fn default() -> Self {
        Self {
            epoch_ns: None,
            columns: Vec::new(),
            threads: HashMap::new(),
            thread_names: Vec::new(),
            filter: None,
            search: ThreadSearch::default(),
            auto_scroll: true,
            scroll_x: 0,
            cursor_col: 0,
            cursor_row: 0,
        }
    }
}

impl FlamescopeTab {
    pub const ROWS: usize = SUBSECOND_ROWS;

    pub fn record_timestamps(&mut self, entries: &HashMap<String, Vec<u64>>) {
        for (thread, timestamps) in entries {
            if !self.threads.contains_key(thread) {
                let pos = self
                    .thread_names
                    .binary_search(thread)
                    .unwrap_or_else(|e| e);
                self.thread_names.insert(pos, thread.clone());
            }

            for &ts in timestamps {
                let epoch = *self.epoch_ns.get_or_insert(ts);
                let offset = ts.saturating_sub(epoch);
                let col = (offset / NS_PER_SEC) as usize;
                let row = ((offset % NS_PER_SEC) / NS_PER_ROW) as usize;
                let row = row.min(SUBSECOND_ROWS - 1);

                while self.columns.len() <= col {
                    self.columns.push([0; SUBSECOND_ROWS]);
                }
                self.columns[col][row] += 1;

                let thread_cols = self.threads.entry(thread.clone()).or_default();
                while thread_cols.len() <= col {
                    thread_cols.push([0; SUBSECOND_ROWS]);
                }
                thread_cols[col][row] += 1;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    pub fn visible_columns(&self) -> &[[u64; SUBSECOND_ROWS]] {
        match &self.filter {
            Some(name) => self.threads.get(name).map_or(&[], |v| v.as_slice()),
            None => &self.columns,
        }
    }

    pub fn selected_value(&self) -> u64 {
        self.visible_columns()
            .get(self.cursor_col)
            .map_or(0, |col| col[self.cursor_row])
    }

    pub fn selected_time(&self) -> (usize, usize, usize) {
        let ms_start = (self.cursor_row * 1000) / SUBSECOND_ROWS;
        let ms_end = ((self.cursor_row + 1) * 1000) / SUBSECOND_ROWS;
        (self.cursor_col, ms_start, ms_end)
    }

    pub fn visible_peak(&self) -> u64 {
        self.visible_columns()
            .iter()
            .flat_map(|col| col.iter())
            .copied()
            .max()
            .unwrap_or(0)
    }

    pub fn total_seconds(&self) -> usize {
        self.visible_columns().len()
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.search.active {
            return self.handle_search_key(key);
        }
        match key.code {
            KeyCode::Right | KeyCode::Char('l') => {
                self.auto_scroll = false;
                let total = self.visible_columns().len();
                if total > 0 && self.cursor_col + 1 < total {
                    self.cursor_col += 1;
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.auto_scroll = false;
                self.cursor_col = self.cursor_col.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor_row + 1 < SUBSECOND_ROWS {
                    self.cursor_row += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor_row = self.cursor_row.saturating_sub(1);
            }
            KeyCode::Char('/') => {
                self.search.open();
                self.refresh_search();
            }
            KeyCode::Esc => {
                self.filter = None;
                self.auto_scroll = true;
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.auto_scroll = true;
            }
            KeyCode::Char('r') => *self = Self::default(),
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.search.close(),
            KeyCode::Enter => {
                if let Some(name) = self.search.matches.get(self.search.cursor).cloned() {
                    self.filter = Some(name);
                    self.cursor_col = 0;
                    self.scroll_x = 0;
                    self.auto_scroll = true;
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
            .thread_names
            .iter()
            .filter(|name| query.is_empty() || name.to_lowercase().contains(&query))
            .cloned()
            .collect();
    }
}

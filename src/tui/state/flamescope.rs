use std::collections::HashMap;

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::flamegraph::THREAD_PICKER;
use crate::tui::widgets::{Cursor, Picker, PickerEvent};

const SUBSECOND_ROWS: usize = 10;
const NS_PER_SEC: u64 = 1_000_000_000;
const NS_PER_ROW: u64 = NS_PER_SEC / SUBSECOND_ROWS as u64;

/// Number of one-second columns visible at once.
pub const VISIBLE_COLS: usize = 30;

pub type Column = [u64; SUBSECOND_ROWS];

pub struct FlamescopeTab {
    epoch_ns: Option<u64>,
    columns: Vec<Column>,
    threads: HashMap<String, Vec<Column>>,
    thread_names: Vec<String>,
    pub filter: Option<String>,
    pub picker: Option<Picker>,
    /// Follow the newest column as data arrives.
    pub auto_scroll: bool,
    /// Selected second (`index`) and first visible second (`offset`).
    pub col: Cursor,
    /// Selected subsecond row.
    pub row: usize,
}

impl Default for FlamescopeTab {
    fn default() -> Self {
        Self {
            epoch_ns: None,
            columns: Vec::new(),
            threads: HashMap::new(),
            thread_names: Vec::new(),
            filter: None,
            picker: None,
            auto_scroll: true,
            col: Cursor::default(),
            row: 0,
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
            let thread_cols = self.threads.entry(thread.clone()).or_default();

            for &ts in timestamps {
                let epoch = *self.epoch_ns.get_or_insert(ts);
                let offset = ts.saturating_sub(epoch);
                let col = (offset / NS_PER_SEC) as usize;
                let row =
                    ((offset % NS_PER_SEC) / NS_PER_ROW).min(SUBSECOND_ROWS as u64 - 1) as usize;

                bump(&mut self.columns, col, row);
                bump(thread_cols, col, row);
            }
        }
        self.sync_cursor();
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    pub fn visible_columns(&self) -> &[Column] {
        match &self.filter {
            Some(name) => self.threads.get(name).map_or(&[], Vec::as_slice),
            None => &self.columns,
        }
    }

    pub fn selected_value(&self) -> u64 {
        self.visible_columns()
            .get(self.col.index)
            .map_or(0, |col| col[self.row])
    }

    /// `(second, ms_start, ms_end)` of the selected cell.
    pub fn selected_time(&self) -> (usize, usize, usize) {
        let ms_start = (self.row * 1000) / SUBSECOND_ROWS;
        let ms_end = ((self.row + 1) * 1000) / SUBSECOND_ROWS;
        (self.col.index, ms_start, ms_end)
    }

    pub fn visible_peak(&self) -> u64 {
        self.visible_columns()
            .iter()
            .flatten()
            .copied()
            .max()
            .unwrap_or(0)
    }

    pub fn total_seconds(&self) -> usize {
        self.visible_columns().len()
    }

    /// Clamp the column cursor to the data, snap to the newest column when
    /// auto-scrolling, and keep it inside the fixed-width viewport.
    fn sync_cursor(&mut self) {
        let len = self.visible_columns().len();
        self.col.clamp(len);
        if self.auto_scroll {
            self.col.last(len);
        }
        self.col.scroll_to_fit(VISIBLE_COLS);
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.picker.is_some() {
            return self.handle_picker_key(key);
        }
        match key.code {
            KeyCode::Right | KeyCode::Char('l') => {
                self.auto_scroll = false;
                self.col.next(self.visible_columns().len());
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.auto_scroll = false;
                self.col.prev();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.row + 1 < SUBSECOND_ROWS {
                    self.row += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.row = self.row.saturating_sub(1),
            KeyCode::Char('/') => {
                let mut picker = Picker::new(&THREAD_PICKER);
                picker.refresh(self.thread_names.iter().map(String::as_str));
                self.picker = Some(picker);
            }
            KeyCode::Esc => {
                self.filter = None;
                self.auto_scroll = true;
            }
            KeyCode::Char('G') | KeyCode::End => self.auto_scroll = true,
            KeyCode::Char('r') => *self = Self::default(),
            _ => {}
        }
        self.sync_cursor();
    }

    fn handle_picker_key(&mut self, key: KeyEvent) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        match picker.handle_key(key) {
            Some(PickerEvent::Changed) => {
                picker.refresh(self.thread_names.iter().map(String::as_str))
            }
            Some(PickerEvent::Cancel) => self.picker = None,
            Some(PickerEvent::Submit) => {
                let picked = self
                    .picker
                    .take()
                    .and_then(|p| p.selected().map(str::to_owned));
                if let Some(name) = picked {
                    self.filter = Some(name);
                    self.col.reset();
                    self.auto_scroll = true;
                    self.sync_cursor();
                }
            }
            None => {}
        }
    }
}

/// Increment `cols[col][row]`, growing `cols` with empty columns as needed.
fn bump(cols: &mut Vec<Column>, col: usize, row: usize) {
    if cols.len() <= col {
        cols.resize(col + 1, [0; SUBSECOND_ROWS]);
    }
    cols[col][row] += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab_with(ts: &[u64]) -> FlamescopeTab {
        let mut tab = FlamescopeTab::default();
        tab.record_timestamps(&HashMap::from([("t".to_string(), ts.to_vec())]));
        tab
    }

    #[test]
    fn timestamps_bucket_into_seconds_and_rows() {
        let tab = tab_with(&[0, 150_000_000, NS_PER_SEC * 2 + 950_000_000]);
        let cols = tab.visible_columns();
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[0][0], 1);
        assert_eq!(cols[0][1], 1);
        assert_eq!(cols[2][9], 1);
    }

    #[test]
    fn auto_scroll_follows_newest_column() {
        let tab = tab_with(&[0, NS_PER_SEC * 40]);
        assert_eq!(tab.col.index, 40);
        assert_eq!(tab.col.offset, 40 + 1 - VISIBLE_COLS);
    }

    #[test]
    fn filter_without_data_yields_empty_view() {
        let mut tab = tab_with(&[0]);
        tab.filter = Some("other".into());
        assert!(tab.visible_columns().is_empty());
        assert_eq!(tab.selected_value(), 0);
    }
}

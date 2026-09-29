use std::collections::HashMap;
use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::flamegraph::THREAD_PICKER;
use crate::tui::widgets::{Cursor, Picker, PickerEvent};

/// Samples are bucketed at 10ms. Display rows group whole buckets, so this
/// is also the finest resolution a tall terminal can show.
const BUCKETS: usize = 100;
const NS_PER_SEC: u64 = 1_000_000_000;
const NS_PER_BUCKET: u64 = NS_PER_SEC / BUCKETS as u64;
const MS_PER_BUCKET: usize = 1000 / BUCKETS;

/// One second of samples, one counter per 10ms bucket.
pub type Column = [u32; BUCKETS];

pub struct FlamescopeTab {
    epoch_ns: Option<u64>,
    all: Timeline,
    threads: HashMap<String, Timeline>,
    thread_names: Vec<String>,
    pub filter: Option<String>,
    pub picker: Option<Picker>,
    /// Follow the newest column as data arrives.
    pub auto_scroll: bool,
    /// Selected second (`index`) and first visible second (`offset`).
    pub col: Cursor,
    /// Selected display row, in `0..rows`.
    pub row: usize,
    /// Display rows per second, chosen by the view to fill the terminal height.
    rows: usize,
}

impl Default for FlamescopeTab {
    fn default() -> Self {
        Self {
            epoch_ns: None,
            all: Timeline::default(),
            threads: HashMap::new(),
            thread_names: Vec::new(),
            filter: None,
            picker: None,
            auto_scroll: true,
            col: Cursor::default(),
            row: 0,
            rows: 10,
        }
    }
}

impl FlamescopeTab {
    /// Most display rows per second: one per bucket.
    pub const MAX_ROWS: usize = BUCKETS;

    /// Switch display resolution, keeping the cursor at the same time offset.
    pub fn set_rows(&mut self, rows: usize) {
        let rows = rows.clamp(1, Self::MAX_ROWS);
        if rows != self.rows {
            self.row = (self.row * rows / self.rows).min(rows - 1);
            self.rows = rows;
        }
    }

    /// Buckets covered by display row `row`. When `rows` does not divide
    /// [`BUCKETS`], spans differ by at most one bucket.
    fn row_buckets(&self, row: usize) -> Range<usize> {
        row * BUCKETS / self.rows..(row + 1) * BUCKETS / self.rows
    }

    /// Start of display row `row` within its second, in milliseconds.
    pub fn row_start_ms(&self, row: usize) -> usize {
        self.row_buckets(row).start * MS_PER_BUCKET
    }

    /// Samples in display row `row` of `col`.
    pub fn cell(&self, col: &Column, row: usize) -> u64 {
        col[self.row_buckets(row)].iter().map(|&n| n as u64).sum()
    }

    /// Samples per bucket in display row `row` of `col`. Colors use this so
    /// rows spanning one bucket more than their neighbors do not look hotter.
    pub fn density(&self, col: &Column, row: usize) -> f64 {
        self.cell(col, row) as f64 / self.row_buckets(row).len() as f64
    }

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
                let bucket = ((offset % NS_PER_SEC) / NS_PER_BUCKET) as usize;

                self.all.record(col, bucket);
                thread_cols.record(col, bucket);
            }
        }
        self.sync_cursor();
    }

    pub fn is_empty(&self) -> bool {
        self.all.0.is_empty()
    }

    pub fn visible_columns(&self) -> &[Column] {
        match &self.filter {
            Some(name) => self.threads.get(name).map_or(&[], |t| &t.0),
            None => &self.all.0,
        }
    }

    pub fn selected_value(&self) -> u64 {
        self.visible_columns()
            .get(self.col.index)
            .map_or(0, |col| self.cell(col, self.row))
    }

    /// `(second, ms_start, ms_end)` of the selected cell.
    pub fn selected_time(&self) -> (usize, usize, usize) {
        let buckets = self.row_buckets(self.row);
        (
            self.col.index,
            buckets.start * MS_PER_BUCKET,
            buckets.end * MS_PER_BUCKET,
        )
    }

    /// Most samples in any visible display cell.
    pub fn visible_peak(&self) -> u64 {
        self.cells()
            .map(|(col, row)| self.cell(col, row))
            .max()
            .unwrap_or(0)
    }

    /// Highest per-bucket density of any visible display cell; the heatmap's scale.
    pub fn peak_density(&self) -> f64 {
        self.cells()
            .map(|(col, row)| self.density(col, row))
            .fold(0.0, f64::max)
    }

    fn cells(&self) -> impl Iterator<Item = (&Column, usize)> {
        self.visible_columns()
            .iter()
            .flat_map(move |col| (0..self.rows).map(move |row| (col, row)))
    }

    pub fn total_seconds(&self) -> usize {
        self.visible_columns().len()
    }

    /// Clamp the column cursor to the data and snap it to the newest column
    /// when auto-scrolling. The view scrolls it into its viewport, whose
    /// width depends on the terminal.
    fn sync_cursor(&mut self) {
        let len = self.visible_columns().len();
        self.col.clamp(len);
        if self.auto_scroll {
            self.col.last(len);
        }
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
                if self.row + 1 < self.rows {
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

/// One-second columns of bucketed sample counts, indexed by seconds since
/// the first sample.
#[derive(Default)]
struct Timeline(Vec<Column>);

impl Timeline {
    /// Count one sample, growing the timeline with empty seconds as needed.
    fn record(&mut self, second: usize, bucket: usize) {
        if self.0.len() <= second {
            self.0.resize(second + 1, [0; BUCKETS]);
        }
        self.0[second][bucket] += 1;
    }
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
    fn timestamps_bucket_into_seconds_and_10ms_buckets() {
        let tab = tab_with(&[0, 150_000_000, NS_PER_SEC * 2 + 995_000_000]);
        let cols = tab.visible_columns();
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[0][0], 1);
        assert_eq!(cols[0][15], 1, "150ms lands in the 150-160ms bucket");
        assert_eq!(cols[2][99], 1, "995ms lands in the last bucket");
    }

    #[test]
    fn uneven_rows_are_colored_by_density() {
        // 3 rows over 100 buckets span 33, 33 and 34 buckets.
        let mut tab = tab_with(&[0]);
        tab.set_rows(3);
        assert_eq!(tab.row_buckets(0), 0..33);
        assert_eq!(tab.row_buckets(2), 66..100);
        assert_eq!(tab.row_start_ms(2), 660);
        // One sample in every bucket: equal density everywhere, despite the
        // last row holding one more sample.
        let ts: Vec<u64> = (0..100).map(|b| b * NS_PER_BUCKET).collect();
        let mut tab = tab_with(&ts);
        tab.set_rows(3);
        let col = tab.visible_columns()[0];
        assert_eq!((tab.cell(&col, 0), tab.cell(&col, 2)), (33, 34));
        assert_eq!(tab.density(&col, 0), tab.density(&col, 2));
        assert_eq!(tab.peak_density(), 1.0);
    }

    #[test]
    fn display_rows_sum_whole_buckets() {
        // Three samples in the first 100ms, one at 500ms.
        let mut tab = tab_with(&[0, 30_000_000, 90_000_000, 500_000_000]);
        tab.set_rows(10);
        let col = tab.visible_columns()[0];
        assert_eq!(tab.cell(&col, 0), 3);
        assert_eq!(tab.cell(&col, 5), 1);
        assert_eq!(tab.visible_peak(), 3);
        tab.set_rows(50);
        assert_eq!(tab.visible_peak(), 1, "finer rows split the burst");
    }

    #[test]
    fn changing_resolution_keeps_the_cursor_time() {
        let mut tab = tab_with(&[0]);
        tab.set_rows(10);
        tab.row = 5;
        assert_eq!(tab.selected_time(), (0, 500, 600));
        tab.set_rows(50);
        assert_eq!(tab.row, 25);
        assert_eq!(tab.selected_time(), (0, 500, 520));
        tab.set_rows(5);
        assert_eq!(tab.selected_time(), (0, 400, 600));
    }

    #[test]
    fn auto_scroll_follows_newest_column() {
        let tab = tab_with(&[0, NS_PER_SEC * 40]);
        assert_eq!(tab.col.index, 40);
    }

    #[test]
    fn filter_without_data_yields_empty_view() {
        let mut tab = tab_with(&[0]);
        tab.filter = Some("other".into());
        assert!(tab.visible_columns().is_empty());
        assert_eq!(tab.selected_value(), 0);
    }
}

use ratatui::layout::Rect;

use crate::tui::state::FlamescopeTab;

const GRID_COLS: usize = 30;
const GRID_ROWS: usize = FlamescopeTab::ROWS;
const LABEL_W: u16 = 6;

pub struct FlamescopeLayout {
    pub area: Rect,
    pub cell_w: u16,
    pub cell_h: u16,
    pub visible_cols: usize,
    pad_top: u16,
    pad_left: u16,
}

impl FlamescopeLayout {
    pub fn new(area: Rect) -> Option<Self> {
        if area.width < 10 || area.height < 2 {
            return None;
        }

        let avail_h = area.height as usize;
        let avail_w = area.width.saturating_sub(LABEL_W) as usize;

        let cell_h = (avail_h / GRID_ROWS).max(1) as u16;
        let cell_w = (avail_w / GRID_COLS).max(2) as u16;

        let used_h = cell_h * GRID_ROWS as u16;
        let used_w = LABEL_W + cell_w * GRID_COLS as u16;
        let pad_top = (area.height.saturating_sub(used_h)) / 2;
        let pad_left = (area.width.saturating_sub(used_w)) / 2;

        Some(Self {
            area,
            cell_w,
            cell_h,
            visible_cols: GRID_COLS,
            pad_top,
            pad_left,
        })
    }

    pub const fn num_rows(&self) -> usize {
        GRID_ROWS
    }

    pub fn row_y(&self, row: usize) -> u16 {
        self.area.y + self.pad_top + row as u16 * self.cell_h
    }

    pub fn cell_x(&self, col_offset: usize) -> u16 {
        self.area.x + self.pad_left + LABEL_W + col_offset as u16 * self.cell_w
    }

    pub fn label_x(&self) -> u16 {
        self.area.x + self.pad_left
    }

    pub fn ms_label(&self, row: usize) -> usize {
        (row * 1000) / GRID_ROWS
    }

    pub fn bottom(&self) -> u16 {
        self.area.y + self.area.height
    }

    pub fn right(&self) -> u16 {
        self.area.x + self.area.width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_rows_same_height() {
        for h in 10u16..=120 {
            let area = Rect::new(0, 0, 200, h);
            let lay = FlamescopeLayout::new(area).unwrap();
            assert!(lay.cell_h >= 1);
            for row in 1..GRID_ROWS {
                assert_eq!(lay.row_y(row) - lay.row_y(row - 1), lay.cell_h);
            }
        }
    }

    #[test]
    fn grid_stays_within_area() {
        for h in 10u16..=120 {
            let area = Rect::new(0, 5, 200, h);
            let lay = FlamescopeLayout::new(area).unwrap();
            let last_bottom = lay.row_y(GRID_ROWS - 1) + lay.cell_h;
            assert!(last_bottom <= lay.bottom(), "height {h}: grid overflows");
        }
    }

    #[test]
    fn grid_centered_vertically() {
        let area = Rect::new(0, 0, 200, 47);
        let lay = FlamescopeLayout::new(area).unwrap();
        let used = lay.cell_h * GRID_ROWS as u16;
        let leftover = 47 - used;
        assert_eq!(lay.pad_top, leftover / 2);
    }

    #[test]
    fn grid_centered_horizontally() {
        let area = Rect::new(0, 0, 200, 50);
        let lay = FlamescopeLayout::new(area).unwrap();
        let used_w = LABEL_W + lay.cell_w * GRID_COLS as u16;
        let leftover = 200 - used_w;
        assert_eq!(lay.pad_left, leftover / 2);
    }

    #[test]
    fn visible_cols_always_30() {
        let area = Rect::new(0, 0, 200, 50);
        let lay = FlamescopeLayout::new(area).unwrap();
        assert_eq!(lay.visible_cols, 30);
    }
}

//! Geometry of the flamescope grid.
//!
//! Cells are one line tall and a fixed width, roughly square, so the grid
//! never stretches with the terminal. The grid fills the whole height with
//! one subsecond row per line, and the width with as many seconds as fit, so
//! a bigger terminal (or a smaller font) shows more cells in both directions.

use ratatui::layout::Rect;

use crate::tui::state::FlamescopeTab;

/// Width of the `980ms ` row labels.
pub const LABEL_W: u16 = 6;
/// Columns drawn per cell. Two terminal columns are about as wide as one
/// line is tall, so cells come out roughly square.
pub const CELL_W: u16 = 2;
/// Blank columns between cells.
pub const GAP_W: u16 = 1;
const PITCH: u16 = CELL_W + GAP_W;
/// Line reserved under the grid for the seconds axis.
const AXIS_H: u16 = 1;
/// Below this many rows a second is too coarse to be useful.
const MIN_ROWS: usize = 5;

pub struct FlamescopeLayout {
    /// Subsecond rows per column: one per available line.
    pub rows: usize,
    /// Seconds visible at once.
    pub cols: usize,
    top: u16,
    left: u16,
    label_x: u16,
}

impl FlamescopeLayout {
    /// `None` when `area` cannot fit [`MIN_ROWS`] rows and one column.
    pub fn new(area: Rect) -> Option<Self> {
        let rows = (area.height.saturating_sub(AXIS_H) as usize).min(FlamescopeTab::MAX_ROWS);
        let grid_w = area.width.saturating_sub(LABEL_W);
        let cols = ((grid_w + GAP_W) / PITCH) as usize;
        if rows < MIN_ROWS || cols == 0 {
            return None;
        }
        // Only a terminal taller than MAX_ROWS lines leaves spare height.
        let spare = area.height - (rows as u16 + AXIS_H);
        Some(Self {
            rows,
            cols,
            top: area.y + spare / 2,
            left: area.x + LABEL_W,
            label_x: area.x,
        })
    }

    pub fn row_y(&self, row: usize) -> u16 {
        self.top + row as u16
    }

    pub fn cell_x(&self, col_offset: usize) -> u16 {
        self.left + col_offset as u16 * PITCH
    }

    pub fn label_x(&self) -> u16 {
        self.label_x
    }

    pub fn axis_y(&self) -> u16 {
        self.top + self.rows as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_and_axis_fill_the_height() {
        for h in [6u16, 24, 37, 57, 101] {
            let area = Rect::new(0, 3, 100, h);
            let lay = FlamescopeLayout::new(area).unwrap();
            assert_eq!(lay.rows, h as usize - 1, "height {h}");
            assert_eq!(lay.row_y(0), area.y);
            assert_eq!(lay.axis_y(), area.bottom() - 1);
        }
    }

    #[test]
    fn very_tall_areas_cap_at_one_row_per_bucket() {
        let area = Rect::new(0, 0, 100, 140);
        let lay = FlamescopeLayout::new(area).unwrap();
        assert_eq!(lay.rows, FlamescopeTab::MAX_ROWS);
        assert!(lay.axis_y() < area.bottom());
    }

    #[test]
    fn too_small_areas_render_nothing() {
        assert!(FlamescopeLayout::new(Rect::new(0, 0, 100, 5)).is_none());
        assert!(FlamescopeLayout::new(Rect::new(0, 0, LABEL_W + 1, 30)).is_none());
    }

    #[test]
    fn more_width_shows_more_seconds_not_wider_cells() {
        let cols_for = |w| FlamescopeLayout::new(Rect::new(0, 0, w, 30)).unwrap().cols;
        assert_eq!(cols_for(LABEL_W + 2), 1);
        assert_eq!(cols_for(LABEL_W + 5), 2);
        assert_eq!(cols_for(200), ((200 - LABEL_W + GAP_W) / PITCH) as usize);
    }

    #[test]
    fn grid_stays_inside_the_area() {
        for (w, h) in [(40, 7), (80, 24), (120, 40), (200, 60), (300, 120)] {
            let area = Rect::new(3, 5, w, h);
            let lay = FlamescopeLayout::new(area).unwrap();
            let last_x = lay.cell_x(lay.cols - 1) + CELL_W;
            assert!(last_x <= area.right(), "{w}x{h}: grid overflows right");
            assert!(
                lay.axis_y() < area.bottom(),
                "{w}x{h}: axis overflows bottom"
            );
        }
    }
}

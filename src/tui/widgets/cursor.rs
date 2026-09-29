use std::ops::Range;

/// A selection index plus the viewport offset needed to keep it on screen.
///
/// Works for any linear list: table rows, heatmap columns, picker matches.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Selected item.
    pub index: usize,
    /// First visible item.
    pub offset: usize,
}

impl Cursor {
    pub fn next(&mut self, len: usize) {
        if self.index + 1 < len {
            self.index += 1;
        }
    }

    pub fn prev(&mut self) {
        self.index = self.index.saturating_sub(1);
    }

    pub fn last(&mut self, len: usize) {
        self.index = len.saturating_sub(1);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Keep `index` valid after the underlying list changed size.
    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.reset();
        } else {
            self.index = self.index.min(len - 1);
        }
    }

    /// Move `offset` the minimum distance so `index` is inside a viewport of
    /// `size` items. Returns the new offset.
    pub fn scroll_to_fit(&mut self, size: usize) -> usize {
        self.offset = Self::fit_offset(self.offset, self.index, size);
        self.offset
    }

    /// Smallest change to `offset` that brings `index` into `offset..offset + size`.
    fn fit_offset(offset: usize, index: usize, size: usize) -> usize {
        if size == 0 || index < offset {
            index.min(offset)
        } else if index >= offset + size {
            index + 1 - size
        } else {
            offset
        }
    }

    /// Indices shown by a viewport of `size` items over a list of `len`.
    pub fn visible(&self, size: usize, len: usize) -> Range<usize> {
        self.offset..(self.offset + size).min(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_stops_at_end() {
        let mut c = Cursor::default();
        c.next(2);
        c.next(2);
        c.next(2);
        assert_eq!(c.index, 1);
        c.next(0);
        assert_eq!(c.index, 1);
    }

    #[test]
    fn clamp_handles_shrink_and_empty() {
        let mut c = Cursor {
            index: 5,
            offset: 3,
        };
        c.clamp(3);
        assert_eq!(c.index, 2);
        c.clamp(0);
        assert_eq!(c, Cursor::default());
    }

    #[test]
    fn fit_offset_scrolls_both_ways() {
        assert_eq!(Cursor::fit_offset(0, 3, 10), 0, "already visible");
        assert_eq!(
            Cursor::fit_offset(0, 10, 5),
            6,
            "scroll down to show index 10"
        );
        assert_eq!(Cursor::fit_offset(6, 2, 5), 2, "scroll up to show index 2");
        assert_eq!(Cursor::fit_offset(4, 7, 0), 4, "zero viewport is a no-op");
    }

    #[test]
    fn visible_range_is_clipped_to_len() {
        let c = Cursor {
            index: 0,
            offset: 8,
        };
        assert_eq!(c.visible(5, 10), 8..10);
    }
}

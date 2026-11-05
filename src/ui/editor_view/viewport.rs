/// The scroll offset of an editor pane, in lines and display columns.
///
/// Persisted between frames so the view only scrolls when the cursor would
/// otherwise leave the screen (or enter the scroll-off margin).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub top: usize,
    pub left: usize,
}

/// The cursor and buffer dimensions needed to compute scrolling.
#[derive(Debug, Clone, Copy)]
pub struct ScrollTarget {
    pub row: usize,
    /// Cursor column in display cells.
    pub col: usize,
    pub line_count: usize,
}

impl Viewport {
    /// Adjusts the offset so the cursor is visible in a `width` x `height`
    /// area, keeping `scroll_off` lines of context above and below it where
    /// the buffer allows.
    pub fn scroll_to(
        &mut self,
        target: ScrollTarget,
        width: usize,
        height: usize,
        scroll_off: usize,
    ) {
        if height > 0 {
            let margin = scroll_off.min((height - 1) / 2);
            let above = margin.min(target.row);
            let below = margin.min(target.line_count.saturating_sub(target.row + 1));

            if target.row < self.top + above {
                self.top = target.row - above;
            } else if target.row + below >= self.top + height {
                self.top = target.row + below + 1 - height;
            }
        }

        if width > 0 {
            if target.col < self.left {
                self.left = target.col;
            } else if target.col >= self.left + width {
                self.left = target.col + 1 - width;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(row: usize, col: usize) -> ScrollTarget {
        ScrollTarget {
            row,
            col,
            line_count: 100,
        }
    }

    #[test]
    fn scrolls_down_when_cursor_below_view() {
        let mut v = Viewport::default();
        v.scroll_to(target(10, 0), 80, 5, 0);
        assert_eq!(v.top, 6);
    }

    #[test]
    fn scrolls_up_when_cursor_above_view() {
        let mut v = Viewport { top: 10, left: 0 };
        v.scroll_to(target(3, 0), 80, 5, 0);
        assert_eq!(v.top, 3);
    }

    #[test]
    fn does_not_scroll_when_cursor_visible() {
        let mut v = Viewport { top: 2, left: 0 };
        v.scroll_to(target(4, 0), 80, 5, 0);
        assert_eq!(v.top, 2);
    }

    #[test]
    fn keeps_scroll_off_margin() {
        let mut v = Viewport::default();
        v.scroll_to(target(8, 0), 80, 10, 3);
        assert_eq!(v.top, 2, "row 8 should have 3 lines below it visible");

        let mut v = Viewport { top: 20, left: 0 };
        v.scroll_to(target(21, 0), 80, 10, 3);
        assert_eq!(v.top, 18);
    }

    #[test]
    fn scroll_off_is_limited_by_buffer_end() {
        let mut v = Viewport::default();
        let at_end = ScrollTarget {
            row: 9,
            col: 0,
            line_count: 10,
        };
        v.scroll_to(at_end, 80, 5, 3);
        assert_eq!(v.top, 5, "no blank lines should be shown past the end");
    }

    #[test]
    fn scroll_off_is_capped_to_half_the_height() {
        let mut v = Viewport::default();
        // Height 4 allows a margin of 1: rows 1..=4 are shown for row 3.
        v.scroll_to(target(3, 0), 80, 4, 100);
        assert_eq!(v.top, 1);
    }

    #[test]
    fn scrolls_horizontally() {
        let mut v = Viewport::default();
        v.scroll_to(target(0, 20), 10, 5, 0);
        assert_eq!(v.left, 11);
        v.scroll_to(target(0, 3), 10, 5, 0);
        assert_eq!(v.left, 3);
    }
}

//! The window layout: a tree of splits, and its geometry.
//!
//! Geometry is computed here, with no UI types, so the editor can move
//! between windows by direction (`<C-w>h`) without knowing the screen size:
//! directions only depend on the windows' relative positions, which are the
//! same in any area.

use super::WindowId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// Windows stacked top to bottom (`:split`).
    Horizontal,
    /// Windows side by side (`:vsplit`).
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// A rectangle in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Area {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Area {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn right(&self) -> u16 {
        self.x + self.width
    }

    fn bottom(&self) -> u16 {
        self.y + self.height
    }
}

/// A separator line between two windows: vertical (`│`) or horizontal (`─`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Separator {
    pub area: Area,
    pub vertical: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    Window(WindowId),
    /// Children share the space equally, separated by one-cell lines.
    Split {
        direction: SplitDirection,
        children: Vec<Layout>,
    },
}

impl Layout {
    /// Window ids in reading order (left to right, top to bottom).
    pub fn windows(&self) -> Vec<WindowId> {
        match self {
            Layout::Window(id) => vec![*id],
            Layout::Split { children, .. } => children.iter().flat_map(Layout::windows).collect(),
        }
    }

    /// Splits window `target`, placing `new` after it. A split in the same
    /// direction as the parent joins the parent, so three `:vsplit`s give
    /// three equal columns rather than nested halves.
    pub fn split(&mut self, target: WindowId, new: WindowId, direction: SplitDirection) -> bool {
        match self {
            Layout::Window(id) if *id == target => {
                *self = Layout::Split {
                    direction,
                    children: vec![Layout::Window(target), Layout::Window(new)],
                };
                true
            }
            Layout::Window(_) => false,
            Layout::Split {
                direction: own,
                children,
            } => {
                if *own == direction
                    && let Some(i) = children.iter().position(|c| *c == Layout::Window(target))
                {
                    children.insert(i + 1, Layout::Window(new));
                    return true;
                }
                children.iter_mut().any(|c| c.split(target, new, direction))
            }
        }
    }

    /// Removes window `target`, collapsing splits left with one child.
    pub fn remove(&mut self, target: WindowId) -> bool {
        let Layout::Split { children, .. } = self else {
            return false;
        };
        let removed = if let Some(i) = children.iter().position(|c| *c == Layout::Window(target)) {
            children.remove(i);
            true
        } else {
            children.iter_mut().any(|c| c.remove(target))
        };
        if let Layout::Split { children, .. } = self
            && children.len() == 1
        {
            *self = children.remove(0);
        }
        removed
    }

    /// Each window's area, and the separators between them, within `area`.
    pub fn arrange(&self, area: Area) -> (Vec<(WindowId, Area)>, Vec<Separator>) {
        let mut windows = Vec::new();
        let mut separators = Vec::new();
        self.arrange_into(area, &mut windows, &mut separators);
        (windows, separators)
    }

    fn arrange_into(
        &self,
        area: Area,
        windows: &mut Vec<(WindowId, Area)>,
        separators: &mut Vec<Separator>,
    ) {
        let (direction, children) = match self {
            Layout::Window(id) => {
                windows.push((*id, area));
                return;
            }
            Layout::Split {
                direction,
                children,
            } => (*direction, children),
        };
        let vertical = direction == SplitDirection::Vertical;
        let total = if vertical { area.width } else { area.height };
        let n = children.len() as u16;
        let gaps = n.saturating_sub(1);
        let available = total.saturating_sub(gaps);

        // `offset` is where the next child starts. With too little room for
        // every separator, later windows get no space; nothing is ever
        // placed outside `area`.
        let mut offset: u16 = 0;
        for (i, child) in children.iter().enumerate() {
            let i = i as u16;
            // Share the space evenly, giving any remainder to the first ones.
            let size = (available / n + u16::from(i < available % n)).min(total - offset);
            let child_area = if vertical {
                Area::new(area.x + offset, area.y, size, area.height)
            } else {
                Area::new(area.x, area.y + offset, area.width, size)
            };
            child.arrange_into(child_area, windows, separators);
            offset += size;

            if i + 1 < n && offset < total {
                let line = if vertical {
                    Area::new(area.x + offset, area.y, 1, area.height)
                } else {
                    Area::new(area.x, area.y + offset, area.width, 1)
                };
                separators.push(Separator {
                    area: line,
                    vertical,
                });
                offset += 1;
            }
        }
    }

    /// The window next to `from` in `direction`: the nearest one on that
    /// side that overlaps it, preferring the one most in line with it.
    pub fn neighbor(&self, from: WindowId, direction: Direction) -> Option<WindowId> {
        // Any area works; a large one avoids rounding windows to nothing.
        let (windows, _) = self.arrange(Area::new(0, 0, 1000, 1000));
        let (_, here) = *windows.iter().find(|(id, _)| *id == from)?;

        windows
            .iter()
            .filter(|(id, _)| *id != from)
            .filter_map(|&(id, there)| {
                let (gap, overlap_start, overlap_end, offset) = match direction {
                    Direction::Left if there.right() <= here.x => (
                        here.x - there.right(),
                        here.y.max(there.y),
                        here.bottom().min(there.bottom()),
                        there.y,
                    ),
                    Direction::Right if there.x >= here.right() => (
                        there.x - here.right(),
                        here.y.max(there.y),
                        here.bottom().min(there.bottom()),
                        there.y,
                    ),
                    Direction::Up if there.bottom() <= here.y => (
                        here.y - there.bottom(),
                        here.x.max(there.x),
                        here.right().min(there.right()),
                        there.x,
                    ),
                    Direction::Down if there.y >= here.bottom() => (
                        there.y - here.bottom(),
                        here.x.max(there.x),
                        here.right().min(there.right()),
                        there.x,
                    ),
                    _ => return None,
                };
                (overlap_start < overlap_end).then_some((gap, offset, id))
            })
            .min()
            .map(|(_, _, id)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(n: u64) -> WindowId {
        WindowId(n)
    }

    fn leaf(n: u64) -> Layout {
        Layout::Window(w(n))
    }

    #[test]
    fn split_replaces_a_window_with_a_pair() {
        let mut layout = leaf(0);
        assert!(layout.split(w(0), w(1), SplitDirection::Vertical));
        assert_eq!(
            layout,
            Layout::Split {
                direction: SplitDirection::Vertical,
                children: vec![leaf(0), leaf(1)]
            }
        );
        assert!(
            !layout.split(w(9), w(2), SplitDirection::Vertical),
            "no such window"
        );
    }

    #[test]
    fn same_direction_splits_join_the_parent() {
        let mut layout = leaf(0);
        layout.split(w(0), w(1), SplitDirection::Vertical);
        layout.split(w(0), w(2), SplitDirection::Vertical);
        assert_eq!(layout.windows(), [w(0), w(2), w(1)]);
        let Layout::Split { children, .. } = &layout else {
            panic!()
        };
        assert_eq!(children.len(), 3);
    }

    #[test]
    fn other_direction_splits_nest() {
        let mut layout = leaf(0);
        layout.split(w(0), w(1), SplitDirection::Vertical);
        layout.split(w(1), w(2), SplitDirection::Horizontal);
        assert_eq!(layout.windows(), [w(0), w(1), w(2)]);
    }

    #[test]
    fn remove_collapses_single_children() {
        let mut layout = leaf(0);
        layout.split(w(0), w(1), SplitDirection::Vertical);
        layout.split(w(1), w(2), SplitDirection::Horizontal);
        assert!(layout.remove(w(2)));
        assert_eq!(layout.windows(), [w(0), w(1)]);
        assert!(layout.remove(w(0)));
        assert_eq!(layout, leaf(1));
        assert!(!layout.remove(w(1)), "the last window can't be removed");
    }

    #[test]
    fn arrange_shares_space_with_separators() {
        let mut layout = leaf(0);
        layout.split(w(0), w(1), SplitDirection::Vertical);
        let (windows, separators) = layout.arrange(Area::new(0, 0, 21, 10));
        assert_eq!(
            windows,
            [
                (w(0), Area::new(0, 0, 10, 10)),
                (w(1), Area::new(11, 0, 10, 10))
            ]
        );
        assert_eq!(
            separators,
            [Separator {
                area: Area::new(10, 0, 1, 10),
                vertical: true
            }]
        );
    }

    #[test]
    fn arrange_gives_remainders_to_the_first_windows() {
        let mut layout = leaf(0);
        layout.split(w(0), w(1), SplitDirection::Horizontal);
        layout.split(w(1), w(2), SplitDirection::Horizontal);
        let (windows, separators) = layout.arrange(Area::new(0, 0, 10, 12));
        let heights: Vec<u16> = windows.iter().map(|(_, a)| a.height).collect();
        assert_eq!(heights, [4, 3, 3], "12 rows minus 2 separators");
        assert_eq!(separators.len(), 2);
        assert!(!separators[0].vertical);
    }

    #[test]
    fn arrange_never_places_anything_outside_the_area() {
        let mut layout = leaf(0);
        layout.split(w(0), w(1), SplitDirection::Vertical);
        layout.split(w(1), w(2), SplitDirection::Vertical);
        let area = Area::new(5, 0, 1, 3);
        let (windows, separators) = layout.arrange(area);
        for (_, a) in &windows {
            assert!(a.x >= 5 && a.right() <= 6, "{a:?}");
        }
        for s in &separators {
            assert!(s.area.right() <= 6, "{s:?}");
        }
        assert_eq!(windows.len(), 3, "every window is still listed");
    }

    #[test]
    fn neighbors_by_direction() {
        // [0 | 1]
        // [  | 2]
        let mut layout = leaf(0);
        layout.split(w(0), w(1), SplitDirection::Vertical);
        layout.split(w(1), w(2), SplitDirection::Horizontal);
        assert_eq!(
            layout.neighbor(w(0), Direction::Right),
            Some(w(1)),
            "top-most when tied"
        );
        assert_eq!(layout.neighbor(w(2), Direction::Left), Some(w(0)));
        assert_eq!(layout.neighbor(w(1), Direction::Down), Some(w(2)));
        assert_eq!(layout.neighbor(w(2), Direction::Up), Some(w(1)));
        assert_eq!(layout.neighbor(w(0), Direction::Left), None);
        assert_eq!(layout.neighbor(w(0), Direction::Down), None);
    }
}

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
    /// Children share the space in proportion to their `weights` (one per
    /// child), separated by one-cell lines. Equal weights share equally;
    /// resizing sets them from cell sizes, so proportions survive the
    /// terminal being resized.
    Split {
        direction: SplitDirection,
        children: Vec<Layout>,
        weights: Vec<u32>,
    },
}

/// Splits `available` cells in proportion to `weights`, giving cells left
/// over from rounding to the first children.
fn share(available: u16, weights: &[u32]) -> Vec<u16> {
    let total: u64 = weights.iter().map(|&w| u64::from(w.max(1))).sum();
    let mut sizes: Vec<u16> = weights
        .iter()
        .map(|&w| (u64::from(available) * u64::from(w.max(1)) / total.max(1)) as u16)
        .collect();
    let mut left = available - sizes.iter().sum::<u16>();
    for size in &mut sizes {
        if left == 0 {
            break;
        }
        *size += 1;
        left -= 1;
    }
    sizes
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
                    weights: vec![1, 1],
                };
                true
            }
            Layout::Window(_) => false,
            Layout::Split {
                direction: own,
                children,
                weights,
            } => {
                if *own == direction
                    && let Some(i) = children.iter().position(|c| *c == Layout::Window(target))
                {
                    // Like Vim's `equalalways`, the windows share equally again.
                    children.insert(i + 1, Layout::Window(new));
                    *weights = vec![1; children.len()];
                    return true;
                }
                children.iter_mut().any(|c| c.split(target, new, direction))
            }
        }
    }

    /// Removes window `target`, collapsing splits left with one child.
    pub fn remove(&mut self, target: WindowId) -> bool {
        let Layout::Split {
            children, weights, ..
        } = self
        else {
            return false;
        };
        let removed = if let Some(i) = children.iter().position(|c| *c == Layout::Window(target)) {
            children.remove(i);
            *weights = vec![1; children.len()];
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
        let Layout::Split {
            direction,
            children,
            ..
        } = self
        else {
            if let Layout::Window(id) = self {
                windows.push((*id, area));
            }
            return;
        };
        let vertical = *direction == SplitDirection::Vertical;
        let (areas, lines) = self.child_areas(area);
        for (child, child_area) in children.iter().zip(areas) {
            child.arrange_into(child_area, windows, separators);
        }
        separators.extend(lines.into_iter().map(|area| Separator { area, vertical }));
    }

    /// A split's children's areas within `area`, and the separator lines
    /// between them. With too little room for every separator, later
    /// windows get no space; nothing is ever placed outside `area`.
    fn child_areas(&self, area: Area) -> (Vec<Area>, Vec<Area>) {
        let Layout::Split {
            direction,
            children,
            weights,
        } = self
        else {
            return (vec![area], Vec::new());
        };
        let vertical = *direction == SplitDirection::Vertical;
        let total = if vertical { area.width } else { area.height };
        let n = children.len();
        let available = total.saturating_sub(n.saturating_sub(1) as u16);
        let sizes = share(available, weights);

        let mut areas = Vec::with_capacity(n);
        let mut lines = Vec::new();
        // Where the next child starts.
        let mut offset: u16 = 0;
        for (i, &size) in sizes.iter().enumerate() {
            let size = size.min(total - offset);
            areas.push(if vertical {
                Area::new(area.x + offset, area.y, size, area.height)
            } else {
                Area::new(area.x, area.y + offset, area.width, size)
            });
            offset += size;
            if i + 1 < n && offset < total {
                lines.push(if vertical {
                    Area::new(area.x + offset, area.y, 1, area.height)
                } else {
                    Area::new(area.x, area.y + offset, area.width, 1)
                });
                offset += 1;
            }
        }
        (areas, lines)
    }

    /// Resizes window `target` along `axis` (`Horizontal`: its height,
    /// `Vertical`: its width), in the innermost split of that direction
    /// holding it. `want` gets the current size and the most it can have,
    /// and returns the size wanted. Growing takes cells from the windows
    /// after it first, then the ones before; shrinking gives them to the
    /// next window (or the previous, for the last). Returns `false` if no
    /// split in that direction holds the window.
    pub fn resize(
        &mut self,
        target: WindowId,
        axis: SplitDirection,
        area: Area,
        want: &dyn Fn(u16, u16) -> u16,
    ) -> bool {
        self.resize_in(target, axis, area, want) == Some(true)
    }

    /// `None`: `target` isn't in this subtree. `Some(done)`: it is, and
    /// whether a split has resized it yet.
    fn resize_in(
        &mut self,
        target: WindowId,
        axis: SplitDirection,
        area: Area,
        want: &dyn Fn(u16, u16) -> u16,
    ) -> Option<bool> {
        let (areas, _) = self.child_areas(area);
        let Layout::Split {
            direction,
            children,
            weights,
        } = self
        else {
            return (*self == Layout::Window(target)).then_some(false);
        };
        for (i, (child, child_area)) in children.iter_mut().zip(&areas).enumerate() {
            let Some(done) = child.resize_in(target, axis, *child_area, want) else {
                continue;
            };
            if done || *direction != axis {
                return Some(done);
            }
            let size_of = |a: &Area| {
                if axis == SplitDirection::Vertical {
                    a.width
                } else {
                    a.height
                }
            };
            let mut sizes: Vec<u16> = areas.iter().map(size_of).collect();
            let others = sizes.len() as u16 - 1;
            let total: u16 = sizes.iter().sum();
            let most = total.saturating_sub(others).max(1);
            let wanted = want(sizes[i], most).clamp(1, most);
            resize_child(&mut sizes, i, wanted);
            *weights = sizes.iter().map(|&s| u32::from(s.max(1))).collect();
            return Some(true);
        }
        None
    }

    /// The split directly holding window `target`.
    fn parent_mut(&mut self, target: WindowId) -> Option<&mut Layout> {
        let holds = matches!(self, Layout::Split { children, .. }
            if children.contains(&Layout::Window(target)));
        if holds {
            return Some(self);
        }
        match self {
            Layout::Window(_) => None,
            Layout::Split { children, .. } => {
                children.iter_mut().find_map(|c| c.parent_mut(target))
            }
        }
    }

    /// `<C-w>x`: swaps window `target` with the next window in its row or
    /// column (the previous one, for the last). Sizes stay where they are.
    /// Returns the other window, or `None` if the neighbour is a split.
    pub fn exchange(&mut self, target: WindowId) -> Option<WindowId> {
        let Layout::Split { children, .. } = self.parent_mut(target)? else {
            return None;
        };
        let i = children.iter().position(|c| *c == Layout::Window(target))?;
        let j = if i + 1 < children.len() { i + 1 } else { i - 1 };
        let Layout::Window(other) = children[j] else {
            return None;
        };
        children.swap(i, j);
        Some(other)
    }

    /// `<C-w>r` (`forward`) / `<C-w>R`: rotates the windows in `target`'s
    /// row or column one place. Only works when they're all windows, as in
    /// Vim. Sizes stay where they are.
    pub fn rotate(&mut self, target: WindowId, forward: bool) -> bool {
        let Some(Layout::Split { children, .. }) = self.parent_mut(target) else {
            return false;
        };
        if children.iter().any(|c| matches!(c, Layout::Split { .. })) {
            return false;
        }
        if forward {
            children.rotate_right(1);
        } else {
            children.rotate_left(1);
        }
        true
    }

    /// `<C-w>H`/`J`/`K`/`L`: moves window `target` to that edge of the whole
    /// layout, full height (left, right) or full width (top, bottom).
    pub fn move_to_edge(&mut self, target: WindowId, edge: Direction) -> bool {
        if *self == Layout::Window(target) || !self.remove(target) {
            return false;
        }
        let (direction, first) = match edge {
            Direction::Left => (SplitDirection::Vertical, true),
            Direction::Right => (SplitDirection::Vertical, false),
            Direction::Up => (SplitDirection::Horizontal, true),
            Direction::Down => (SplitDirection::Horizontal, false),
        };
        let rest = std::mem::replace(self, Layout::Window(target));
        // A split already running that way takes the window as a child.
        let mut children = match rest {
            Layout::Split {
                direction: own,
                children,
                ..
            } if own == direction => children,
            other => vec![other],
        };
        if first {
            children.insert(0, Layout::Window(target));
        } else {
            children.push(Layout::Window(target));
        }
        *self = Layout::Split {
            direction,
            weights: vec![1; children.len()],
            children,
        };
        true
    }

    /// `<C-w>=`: every split shares its space equally again.
    pub fn equalize(&mut self) {
        if let Layout::Split {
            children, weights, ..
        } = self
        {
            *weights = vec![1; children.len()];
            children.iter_mut().for_each(Layout::equalize);
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

/// Makes `sizes[i]` equal `wanted`, taking cells from (or giving them to)
/// the others, which keep at least one cell each.
fn resize_child(sizes: &mut [u16], i: usize, wanted: u16) {
    if wanted > sizes[i] {
        let mut need = wanted - sizes[i];
        let order: Vec<usize> = (i + 1..sizes.len()).chain((0..i).rev()).collect();
        for j in order {
            let spare = sizes[j].saturating_sub(1).min(need);
            sizes[j] -= spare;
            need -= spare;
        }
        sizes[i] = wanted - need;
    } else {
        let spare = sizes[i] - wanted;
        let receiver = if i + 1 < sizes.len() { i + 1 } else { i - 1 };
        sizes[receiver] += spare;
        sizes[i] = wanted;
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
                children: vec![leaf(0), leaf(1)],
                weights: vec![1, 1],
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

    fn heights(layout: &Layout, area: Area) -> Vec<u16> {
        layout
            .arrange(area)
            .0
            .iter()
            .map(|(_, a)| a.height)
            .collect()
    }

    fn stack(n: u64) -> Layout {
        let mut layout = leaf(0);
        for i in 1..n {
            layout.split(w(i - 1), w(i), SplitDirection::Horizontal);
        }
        layout
    }

    #[test]
    fn share_splits_in_proportion() {
        assert_eq!(share(10, &[1, 1]), [5, 5]);
        assert_eq!(share(10, &[1, 1, 1]), [4, 3, 3]);
        assert_eq!(share(12, &[1, 2]), [4, 8]);
        assert_eq!(share(5, &[3, 0]), [4, 1], "a zero weight counts as one");
    }

    #[test]
    fn growing_takes_from_the_next_window_first() {
        let mut layout = stack(3);
        let area = Area::new(0, 0, 10, 32); // 30 rows: 10 each
        assert!(layout.resize(w(0), SplitDirection::Horizontal, area, &|h, _| h + 5));
        assert_eq!(heights(&layout, area), [15, 5, 10]);
        // More than the next has: the rest comes from the one after.
        assert!(layout.resize(w(0), SplitDirection::Horizontal, area, &|h, _| h + 6));
        assert_eq!(heights(&layout, area), [21, 1, 8]);
    }

    #[test]
    fn the_last_window_grows_from_the_one_before() {
        let mut layout = stack(2);
        let area = Area::new(0, 0, 10, 21);
        layout.resize(w(1), SplitDirection::Horizontal, area, &|h, _| h + 4);
        assert_eq!(heights(&layout, area), [6, 14]);
    }

    #[test]
    fn shrinking_gives_to_the_next_window_and_keeps_one_row() {
        let mut layout = stack(2);
        let area = Area::new(0, 0, 10, 21);
        layout.resize(w(0), SplitDirection::Horizontal, area, &|h, _| {
            h.saturating_sub(100)
        });
        assert_eq!(heights(&layout, area), [1, 19]);
        layout.resize(w(0), SplitDirection::Horizontal, area, &|_, most| most);
        assert_eq!(
            heights(&layout, area),
            [19, 1],
            "maximised: the other keeps one row"
        );
    }

    #[test]
    fn proportions_survive_a_new_area() {
        let mut layout = stack(2);
        layout.resize(
            w(0),
            SplitDirection::Horizontal,
            Area::new(0, 0, 10, 11),
            &|_, _| 3,
        );
        assert_eq!(heights(&layout, Area::new(0, 0, 10, 11)), [3, 7]);
        assert_eq!(heights(&layout, Area::new(0, 0, 10, 21)), [6, 14]);
    }

    #[test]
    fn resizing_needs_a_split_in_that_direction() {
        let mut layout = stack(2);
        let area = Area::new(0, 0, 10, 21);
        assert!(!layout.resize(w(0), SplitDirection::Vertical, area, &|x, _| x + 1));
        assert!(!leaf(0).resize(w(0), SplitDirection::Horizontal, area, &|x, _| x + 1));
    }

    #[test]
    fn the_innermost_matching_split_is_resized() {
        // [0 | 1]
        // [  | 2]  — resizing 2's width changes the outer split.
        let mut layout = leaf(0);
        layout.split(w(0), w(1), SplitDirection::Vertical);
        layout.split(w(1), w(2), SplitDirection::Horizontal);
        let area = Area::new(0, 0, 21, 21);
        layout.resize(w(2), SplitDirection::Vertical, area, &|x, _| x + 4);
        let widths: Vec<u16> = layout
            .arrange(area)
            .0
            .iter()
            .map(|(_, a)| a.width)
            .collect();
        assert_eq!(widths, [6, 14, 14]);
        layout.resize(w(2), SplitDirection::Horizontal, area, &|_, _| 5);
        assert_eq!(heights(&layout, area), [21, 15, 5]);
    }

    #[test]
    fn equalize_and_new_windows_share_equally() {
        let mut layout = stack(2);
        let area = Area::new(0, 0, 10, 21);
        layout.resize(w(0), SplitDirection::Horizontal, area, &|_, _| 3);
        layout.equalize();
        assert_eq!(heights(&layout, area), [10, 10]);

        layout.resize(w(0), SplitDirection::Horizontal, area, &|_, _| 3);
        layout.split(w(1), w(2), SplitDirection::Horizontal);
        assert_eq!(heights(&layout, Area::new(0, 0, 10, 32)), [10, 10, 10]);
    }

    #[test]
    fn exchange_swaps_with_the_next_or_previous_window() {
        let mut layout = stack(3);
        assert_eq!(layout.exchange(w(0)), Some(w(1)));
        assert_eq!(layout.windows(), [w(1), w(0), w(2)]);
        assert_eq!(
            layout.exchange(w(2)),
            Some(w(0)),
            "the last swaps backwards"
        );
        assert_eq!(layout.windows(), [w(1), w(2), w(0)]);
        assert_eq!(leaf(0).exchange(w(0)), None, "nothing to swap with");
    }

    #[test]
    fn exchange_wont_swap_with_a_split() {
        // [0 | 1/2]
        let mut layout = leaf(0);
        layout.split(w(0), w(1), SplitDirection::Vertical);
        layout.split(w(1), w(2), SplitDirection::Horizontal);
        assert_eq!(layout.exchange(w(0)), None);
        assert_eq!(layout.exchange(w(1)), Some(w(2)), "within the column");
    }

    #[test]
    fn rotate_both_ways_but_not_across_nested_splits() {
        let mut layout = stack(3);
        assert!(layout.rotate(w(0), true));
        assert_eq!(layout.windows(), [w(2), w(0), w(1)]);
        assert!(layout.rotate(w(0), false));
        assert_eq!(layout.windows(), [w(0), w(1), w(2)]);

        let mut nested = leaf(0);
        nested.split(w(0), w(1), SplitDirection::Vertical);
        nested.split(w(1), w(2), SplitDirection::Horizontal);
        assert!(!nested.rotate(w(0), true));
    }

    #[test]
    fn move_to_edges() {
        // Stacked 0/1/2: move 2 to the far left as a full-height column.
        let mut layout = stack(3);
        assert!(layout.move_to_edge(w(2), Direction::Left));
        let area = Area::new(0, 0, 21, 21);
        let (windows, _) = layout.arrange(area);
        assert_eq!(windows[0], (w(2), Area::new(0, 0, 10, 21)));
        assert_eq!(layout.windows(), [w(2), w(0), w(1)]);

        // Then to the bottom: a full-width row below the rest.
        assert!(layout.move_to_edge(w(2), Direction::Down));
        assert_eq!(layout.windows(), [w(0), w(1), w(2)]);
        let Layout::Split {
            direction,
            children,
            ..
        } = &layout
        else {
            panic!()
        };
        assert_eq!(*direction, SplitDirection::Horizontal);
        assert_eq!(children.len(), 3, "joined the existing stack");

        assert!(
            !leaf(0).move_to_edge(w(0), Direction::Left),
            "a lone window"
        );
    }
}

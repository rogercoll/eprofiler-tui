//! Horizontal placement of flamegraph nodes: each node is as wide as its
//! share of the root's samples.

use crate::flamegraph::FlameNode;

/// A node placed on screen: horizontal extent in cells and its depth row.
pub struct FrameRect<'a> {
    pub x: u16,
    pub width: u16,
    pub depth: usize,
    pub node: &'a FlameNode,
}

/// Maps sample counts under `root` onto `width` terminal columns.
pub struct FlameLayout<'a> {
    root: &'a FlameNode,
    /// Columns per sample.
    scale: f64,
}

impl<'a> FlameLayout<'a> {
    pub fn new(root: &'a FlameNode, width: u16) -> Self {
        let scale = if root.total_value > 0 {
            width as f64 / root.total_value as f64
        } else {
            0.0
        };
        Self { root, scale }
    }

    /// Every node at least one column wide, parents before children.
    pub fn frames(&self) -> Vec<FrameRect<'a>> {
        let mut frames = Vec::new();
        self.place(self.root, 0.0, 0, &mut frames);
        frames
    }

    fn place(&self, node: &'a FlameNode, x: f64, depth: usize, out: &mut Vec<FrameRect<'a>>) {
        let start = x.round() as u16;
        let end = (x + node.total_value as f64 * self.scale).round() as u16;
        let width = end.saturating_sub(start);
        if width == 0 {
            return;
        }
        out.push(FrameRect {
            x: start,
            width,
            depth,
            node,
        });

        let mut child_x = x;
        for child in &node.children {
            self.place(child, child_x, depth + 1, out);
            child_x += child.total_value as f64 * self.scale;
        }
    }

    /// The node at `path` below the root, even if narrower than one column.
    pub fn rect_at(&self, path: &[usize]) -> Option<FrameRect<'a>> {
        if self.root.total_value <= 0 {
            return None;
        }
        let mut node = self.root;
        let mut x = 0.0;
        for &idx in path {
            let preceding: i64 = node
                .children
                .get(..idx)?
                .iter()
                .map(|c| c.total_value)
                .sum();
            x += preceding as f64 * self.scale;
            node = node.children.get(idx)?;
        }
        Some(FrameRect {
            x: x.round() as u16,
            width: (node.total_value as f64 * self.scale).round().max(1.0) as u16,
            depth: path.len(),
            node,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flamegraph::FlameGraph;

    fn graph() -> FlameGraph {
        let mut fg = FlameGraph::new();
        fg.add_stack(&["a".into(), "x".into()], 75);
        fg.add_stack(&["b".into()], 25);
        fg
    }

    #[test]
    fn widths_are_proportional_to_samples() {
        let fg = graph();
        let frames = FlameLayout::new(&fg.root, 100).frames();
        let spans: Vec<_> = frames
            .iter()
            .map(|f| (f.node.name.as_str(), f.depth, f.x, f.width))
            .collect();
        assert_eq!(
            spans,
            vec![
                ("all", 0, 0, 100),
                ("a", 1, 0, 75),
                ("x", 2, 0, 75),
                ("b", 1, 75, 25)
            ]
        );
    }

    #[test]
    fn rect_at_matches_the_laid_out_frame() {
        let fg = graph();
        let layout = FlameLayout::new(&fg.root, 100);
        let rect = layout.rect_at(&[1]).unwrap();
        assert_eq!(
            (rect.node.name.as_str(), rect.x, rect.width, rect.depth),
            ("b", 75, 25, 1)
        );
        assert!(layout.rect_at(&[5]).is_none());
    }

    #[test]
    fn tiny_nodes_are_skipped_but_still_selectable() {
        let mut fg = FlameGraph::new();
        fg.add_stack(&["big".into()], 1000);
        fg.add_stack(&["tiny".into()], 1);
        let layout = FlameLayout::new(&fg.root, 10);
        assert!(layout.frames().iter().all(|f| f.node.name != "tiny"));
        assert_eq!(layout.rect_at(&[1]).unwrap().width, 1);
    }
}

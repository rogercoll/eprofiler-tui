//! Horizontal placement of flamegraph nodes: each node is as wide as its
//! share of the root's samples.

use crate::flamegraph::{FlameGraph, FlameNode, NodeId};

/// A node placed on screen: horizontal extent in cells and its depth row.
pub struct FrameRect<'a> {
    pub x: u16,
    pub width: u16,
    pub depth: usize,
    pub id: NodeId,
    pub node: &'a FlameNode,
}

/// Maps sample counts under `root` onto `width` terminal columns.
pub struct FlameLayout<'a> {
    graph: &'a FlameGraph,
    root: NodeId,
    /// Columns per sample.
    scale: f64,
}

impl<'a> FlameLayout<'a> {
    pub fn new(graph: &'a FlameGraph, root: NodeId, width: u16) -> Self {
        let total = graph[root].total_value;
        let scale = if total > 0 {
            width as f64 / total as f64
        } else {
            0.0
        };
        Self { graph, root, scale }
    }

    /// Every node at least one column wide, parents before children.
    pub fn frames(&self) -> Vec<FrameRect<'a>> {
        let mut frames = Vec::new();
        self.place(self.root, 0.0, 0, &mut frames);
        frames
    }

    fn place(&self, id: NodeId, x: f64, depth: usize, out: &mut Vec<FrameRect<'a>>) {
        let node = &self.graph[id];
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
            id,
            node,
        });

        let mut child_x = x;
        for &child in &node.children {
            self.place(child, child_x, depth + 1, out);
            child_x += self.graph[child].total_value as f64 * self.scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(stacks: &[(&[&str], i64)]) -> FlameGraph {
        let mut graph = FlameGraph::new();
        for (names, weight) in stacks {
            let frames: Vec<_> = names.iter().map(|&n| n.into()).collect();
            graph.add_stack(&frames, *weight);
        }
        graph
    }

    #[test]
    fn widths_are_proportional_to_samples() {
        let graph = graph(&[(&["a", "x"], 75), (&["b"], 25)]);
        let frames = FlameLayout::new(&graph, FlameGraph::ROOT, 100).frames();
        let spans: Vec<_> = frames
            .iter()
            .map(|f| (&*f.node.name, f.depth, f.x, f.width))
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
    fn a_zoomed_layout_starts_at_its_root() {
        let graph = graph(&[(&["a", "x"], 75), (&["b"], 25)]);
        let a = graph.child(FlameGraph::ROOT, "a").unwrap();
        let frames = FlameLayout::new(&graph, a, 10).frames();
        assert_eq!(frames[0].id, a);
        assert_eq!((frames[0].x, frames[0].width, frames[0].depth), (0, 10, 0));
    }

    #[test]
    fn nodes_narrower_than_a_column_are_skipped() {
        let graph = graph(&[(&["big"], 1000), (&["tiny"], 1)]);
        let frames = FlameLayout::new(&graph, FlameGraph::ROOT, 10).frames();
        assert!(frames.iter().all(|f| &*f.node.name != "tiny"));
    }
}

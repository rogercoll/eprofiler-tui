use std::collections::HashMap;
use std::sync::Arc;

use crate::frame::{Frame, FrameKind};

/// One frame in the call tree. Children are kept heaviest first.
#[derive(Debug)]
pub struct FlameNode {
    /// Shared with the decoded frames and the parent's index: cloning it
    /// bumps a reference count instead of copying the text.
    pub name: Arc<str>,
    pub kind: FrameKind,
    pub total_value: i64,
    pub self_value: i64,
    pub children: Vec<FlameNode>,
    /// Position of each child in `children`, by name.
    child_index: HashMap<Arc<str>, usize>,
}

impl FlameNode {
    pub fn new(name: Arc<str>, kind: FrameKind) -> Self {
        Self {
            name,
            kind,
            total_value: 0,
            self_value: 0,
            children: Vec::new(),
            child_index: HashMap::new(),
        }
    }

    /// O(1) child lookup by name (returns reference).
    pub fn child_by_name(&self, name: &str) -> Option<&FlameNode> {
        self.child_index.get(name).map(|&idx| &self.children[idx])
    }

    /// Share of this node's time spent in itself rather than in callees.
    pub fn self_ratio(&self) -> f64 {
        if self.total_value > 0 {
            self.self_value as f64 / self.total_value as f64
        } else {
            0.0
        }
    }

    pub fn add_stack(&mut self, stack: &[Frame], value: i64) {
        self.total_value += value;
        let Some((frame, rest)) = stack.split_first() else {
            self.self_value += value;
            return;
        };
        let idx = match self.child_index.get(&frame.name) {
            Some(&idx) => idx,
            None => self.push_child(FlameNode::new(Arc::clone(&frame.name), frame.kind)),
        };
        self.children[idx].add_stack(rest, value);
        self.promote(idx);
    }

    fn push_child(&mut self, child: FlameNode) -> usize {
        let idx = self.children.len();
        self.child_index.insert(Arc::clone(&child.name), idx);
        self.children.push(child);
        idx
    }

    /// Restore heaviest-first order after the child at `idx` gained weight.
    /// Weights only grow, so the child can only move toward the front, past
    /// strictly lighter siblings; equal weights keep their order.
    fn promote(&mut self, mut idx: usize) {
        while idx > 0 && self.children[idx].total_value > self.children[idx - 1].total_value {
            self.children.swap(idx, idx - 1);
            for i in [idx - 1, idx] {
                *self
                    .child_index
                    .get_mut(&self.children[i].name)
                    .expect("every child is indexed") = i;
            }
            idx -= 1;
        }
    }

    /// Walk down child names, stopping if a name is missing.
    pub fn follow_path(&self, names: &[String]) -> &FlameNode {
        names
            .iter()
            .fold(self, |node, name| node.child_by_name(name).unwrap_or(node))
    }

    /// Walk down child indices, stopping if an index is out of bounds.
    pub fn follow_indices(&self, indices: &[usize]) -> &FlameNode {
        indices
            .iter()
            .fold(self, |node, &idx| node.children.get(idx).unwrap_or(node))
    }

    /// Names of the nodes along `indices` below this one, stopping at the
    /// first index that is out of bounds.
    pub fn names_along(&self, indices: &[usize]) -> Vec<String> {
        indices
            .iter()
            .scan(self, |node, &idx| {
                *node = node.children.get(idx)?;
                Some(node.name.to_string())
            })
            .collect()
    }

    /// Walk down child indices; `None` if any index is out of bounds.
    pub fn descend(&self, indices: &[usize]) -> Option<&FlameNode> {
        indices
            .iter()
            .try_fold(self, |node, &idx| node.children.get(idx))
    }
}

/// One distinct call stack from a profile and the samples it received.
#[derive(Debug)]
pub struct SampledStack {
    /// Root first: the thread, then its frames.
    pub frames: Vec<Frame>,
    pub weight: i64,
}

#[cfg(test)]
impl SampledStack {
    pub fn from_names(names: &[&str], weight: i64) -> Self {
        Self {
            frames: names.iter().map(|&n| n.into()).collect(),
            weight,
        }
    }
}

#[derive(Debug)]
pub struct FlameGraph {
    pub root: FlameNode,
}

impl FlameGraph {
    pub fn new() -> Self {
        Self {
            root: FlameNode::new("all".into(), FrameKind::THREAD),
        }
    }

    pub fn add_stack(&mut self, stack: &[Frame], value: i64) {
        self.root.add_stack(stack, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(node: &FlameNode) -> Vec<&str> {
        node.children.iter().map(|c| &*c.name).collect()
    }

    /// Children must be heaviest first and every index entry must point at
    /// the child with that name, at every level.
    fn assert_consistent(node: &FlameNode) {
        let totals: Vec<i64> = node.children.iter().map(|c| c.total_value).collect();
        assert!(totals.windows(2).all(|w| w[0] >= w[1]), "{:?}", names(node));
        assert_eq!(node.child_index.len(), node.children.len());
        for (i, child) in node.children.iter().enumerate() {
            assert_eq!(node.child_index[&child.name], i);
            assert_consistent(child);
        }
    }

    #[test]
    fn a_growing_child_overtakes_lighter_siblings() {
        let mut graph = FlameGraph::new();
        for (name, weight) in [("a", 5), ("b", 3), ("c", 1)] {
            graph.add_stack(&[name.into()], weight);
        }
        assert_eq!(names(&graph.root), ["a", "b", "c"]);

        graph.add_stack(&["c".into(), "leaf".into()], 10);
        assert_eq!(names(&graph.root), ["c", "a", "b"]);
        assert_eq!(graph.root.child_by_name("c").unwrap().total_value, 11);
        assert_consistent(&graph.root);
    }

    #[test]
    fn equal_weights_keep_arrival_order() {
        let mut graph = FlameGraph::new();
        for name in ["x", "y", "z"] {
            graph.add_stack(&[name.into()], 2);
        }
        assert_eq!(names(&graph.root), ["x", "y", "z"]);
    }
}

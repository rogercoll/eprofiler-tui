use std::collections::HashMap;

use crate::frame::{Frame, FrameKind};

#[derive(Clone, Debug)]
pub struct FlameNode {
    pub name: String,
    pub kind: FrameKind,
    pub total_value: i64,
    pub self_value: i64,
    pub children: Vec<FlameNode>,
    child_index: HashMap<String, usize>,
}

impl FlameNode {
    pub fn new(name: String, kind: FrameKind) -> Self {
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
            None => {
                let idx = self.children.len();
                self.children
                    .push(FlameNode::new(frame.name.clone(), frame.kind));
                self.child_index.insert(frame.name.clone(), idx);
                idx
            }
        };
        self.children[idx].add_stack(rest, value);
    }

    pub fn merge(&mut self, other: FlameNode) {
        self.total_value += other.total_value;
        self.self_value += other.self_value;
        for other_child in other.children {
            if let Some(&idx) = self.child_index.get(&other_child.name) {
                self.children[idx].merge(other_child);
            } else {
                let idx = self.children.len();
                self.child_index.insert(other_child.name.clone(), idx);
                self.children.push(other_child);
            }
        }
    }

    pub fn sort_recursive(&mut self) {
        self.children
            .sort_by_key(|c| std::cmp::Reverse(c.total_value));
        self.rebuild_index();
        for child in &mut self.children {
            child.sort_recursive();
        }
    }

    fn rebuild_index(&mut self) {
        self.child_index.clear();
        for (i, child) in self.children.iter().enumerate() {
            self.child_index.insert(child.name.clone(), i);
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
                Some(node.name.clone())
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

#[derive(Clone, Debug)]
pub struct FlameGraph {
    pub root: FlameNode,
}

impl FlameGraph {
    pub fn new() -> Self {
        Self {
            root: FlameNode::new("all".to_string(), FrameKind::THREAD),
        }
    }

    pub fn add_stack(&mut self, stack: &[Frame], value: i64) {
        self.root.add_stack(stack, value);
    }
}

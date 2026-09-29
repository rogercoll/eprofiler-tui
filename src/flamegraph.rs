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

/// A node placed on screen: horizontal extent in cells and its depth row.
pub struct FrameRect<'a> {
    pub x: u16,
    pub width: u16,
    pub depth: usize,
    pub node: &'a FlameNode,
}

/// Place every node under `root` that is at least one cell wide.
pub fn layout_frames(root: &FlameNode, area_width: u16) -> Vec<FrameRect<'_>> {
    if root.total_value <= 0 {
        return Vec::new();
    }
    let scale = area_width as f64 / root.total_value as f64;
    let mut frames = Vec::new();
    layout_recursive(root, 0.0, 0, scale, &mut frames);
    frames
}

fn layout_recursive<'a>(
    node: &'a FlameNode,
    x_float: f64,
    depth: usize,
    scale: f64,
    frames: &mut Vec<FrameRect<'a>>,
) {
    let x_end = x_float + node.total_value as f64 * scale;
    let x = x_float.round() as u16;
    let width = (x_end.round() as u16).saturating_sub(x);
    if width == 0 {
        return;
    }
    frames.push(FrameRect {
        x,
        width,
        depth,
        node,
    });

    let mut child_x = x_float;
    for child in &node.children {
        layout_recursive(child, child_x, depth + 1, scale, frames);
        child_x += child.total_value as f64 * scale;
    }
}

/// Screen extent of the node at `cursor_path` below `root`, even if it is
/// narrower than one cell.
pub fn cursor_frame_rect<'a>(
    root: &'a FlameNode,
    cursor_path: &[usize],
    area_width: u16,
) -> Option<FrameRect<'a>> {
    if root.total_value <= 0 {
        return None;
    }
    let scale = area_width as f64 / root.total_value as f64;
    let mut node = root;
    let mut x_acc = 0.0;

    for &idx in cursor_path {
        let preceding: i64 = node
            .children
            .get(..idx)?
            .iter()
            .map(|c| c.total_value)
            .sum();
        x_acc += preceding as f64 * scale;
        node = node.children.get(idx)?;
    }

    Some(FrameRect {
        x: x_acc.round() as u16,
        width: (node.total_value as f64 * scale).round().max(1.0) as u16,
        depth: cursor_path.len(),
        node,
    })
}

//! The call tree behind the flamegraph, stored flat.
//!
//! Nodes live in one `Vec` and refer to each other by [`NodeId`]. A single
//! graph-wide map finds a node's child by label, so adding a node costs no
//! per-node maps, and ids stay valid as siblings are reordered.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Index;
use std::sync::Arc;

use crate::frame::{Frame, FrameKind};

/// A node's position in its [`FlameGraph`], stable for the graph's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

/// A frame label interned by the graph, so child lookups hash an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LabelId(u32);

/// Hasher for keys made of the graph's own integer ids. The default SipHash
/// resists crafted keys but is slow on small integers; these ids come from
/// the graph itself, so a multiply-rotate mix (as in rustc's `FxHasher`)
/// is enough. Labels, which come from the network, keep the default hasher.
#[derive(Default)]
struct IdHasher(u64);

impl Hasher for IdHasher {
    fn write(&mut self, bytes: &[u8]) {
        bytes.iter().for_each(|&b| self.write_u32(b as u32));
    }

    fn write_u32(&mut self, n: u32) {
        self.0 = (self.0.rotate_left(5) ^ n as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// One frame in the call tree.
#[derive(Debug)]
pub struct FlameNode {
    /// Shared with the graph's label table: cloning bumps a reference count.
    pub name: Arc<str>,
    pub kind: FrameKind,
    pub total_value: i64,
    pub self_value: i64,
    /// `None` only for the root.
    pub parent: Option<NodeId>,
    /// Heaviest first.
    pub children: Vec<NodeId>,
    /// Index of this node in its parent's `children`.
    position: usize,
}

impl FlameNode {
    /// Share of this node's time spent in itself rather than in callees.
    pub fn self_ratio(&self) -> f64 {
        if self.total_value > 0 {
            self.self_value as f64 / self.total_value as f64
        } else {
            0.0
        }
    }

    /// Index of this node among its siblings, heaviest first.
    pub fn position(&self) -> usize {
        self.position
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
    nodes: Vec<FlameNode>,
    labels: HashMap<Arc<str>, LabelId>,
    /// Child of `parent` with a given label, for every node in the graph.
    children: HashMap<(NodeId, LabelId), NodeId, BuildHasherDefault<IdHasher>>,
}

impl Index<NodeId> for FlameGraph {
    type Output = FlameNode;

    fn index(&self, id: NodeId) -> &FlameNode {
        &self.nodes[id.0 as usize]
    }
}

impl FlameGraph {
    /// The synthetic node above every thread.
    pub const ROOT: NodeId = NodeId(0);

    pub fn new() -> Self {
        Self {
            nodes: vec![FlameNode {
                name: "all".into(),
                kind: FrameKind::THREAD,
                total_value: 0,
                self_value: 0,
                parent: None,
                children: Vec::new(),
                position: 0,
            }],
            labels: HashMap::new(),
            children: HashMap::default(),
        }
    }

    /// Every node, root first.
    #[cfg(test)]
    pub fn nodes(&self) -> &[FlameNode] {
        &self.nodes
    }

    /// Add `weight` samples along `stack`, root first.
    pub fn add_stack(&mut self, stack: &[Frame], weight: i64) {
        let mut id = Self::ROOT;
        self.node_mut(id).total_value += weight;
        for frame in stack {
            id = self.child_or_insert(id, frame);
            self.node_mut(id).total_value += weight;
            self.promote(id);
        }
        self.node_mut(id).self_value += weight;
    }

    /// Child of `parent` named `name`, if it has one.
    pub fn child(&self, parent: NodeId, name: &str) -> Option<NodeId> {
        let label = *self.labels.get(name)?;
        self.children.get(&(parent, label)).copied()
    }

    /// Number of steps from `ancestor` down to `id`, or `None` if `id` is not
    /// in `ancestor`'s subtree.
    pub fn depth(&self, id: NodeId, ancestor: NodeId) -> Option<usize> {
        self.ancestors(id).position(|a| a == ancestor)
    }

    /// `id`, then its parent, and so on up to the root.
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(Some(id), |&a| self[a].parent)
    }

    fn node_mut(&mut self, id: NodeId) -> &mut FlameNode {
        &mut self.nodes[id.0 as usize]
    }

    fn child_or_insert(&mut self, parent: NodeId, frame: &Frame) -> NodeId {
        let label = self.intern(&frame.name);
        if let Some(&child) = self.children.get(&(parent, label)) {
            return child;
        }
        let id = NodeId(self.nodes.len() as u32);
        let position = self[parent].children.len();
        self.nodes.push(FlameNode {
            name: Arc::clone(&frame.name),
            kind: frame.kind,
            total_value: 0,
            self_value: 0,
            parent: Some(parent),
            children: Vec::new(),
            position,
        });
        self.node_mut(parent).children.push(id);
        self.children.insert((parent, label), id);
        id
    }

    fn intern(&mut self, name: &Arc<str>) -> LabelId {
        if let Some(&label) = self.labels.get(name) {
            return label;
        }
        let label = LabelId(self.labels.len() as u32);
        self.labels.insert(Arc::clone(name), label);
        label
    }

    /// Restore heaviest-first order after `id` gained weight. Weights only
    /// grow, so `id` can only move toward the front, past strictly lighter
    /// siblings; equal weights keep their order.
    fn promote(&mut self, id: NodeId) {
        let Some(parent) = self[id].parent else {
            return;
        };
        let total = self[id].total_value;
        let mut pos = self[id].position;
        while pos > 0 {
            let prev = self[parent].children[pos - 1];
            if self[prev].total_value >= total {
                break;
            }
            self.node_mut(parent).children.swap(pos - 1, pos);
            self.node_mut(prev).position = pos;
            pos -= 1;
        }
        self.node_mut(id).position = pos;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(graph: &FlameGraph, id: NodeId) -> Vec<&str> {
        graph[id]
            .children
            .iter()
            .map(|&c| &*graph[c].name)
            .collect()
    }

    fn path(graph: &FlameGraph, names: &[&str]) -> NodeId {
        names.iter().fold(FlameGraph::ROOT, |id, name| {
            graph.child(id, name).expect("child exists")
        })
    }

    /// Siblings heaviest first, positions and parents consistent, and every
    /// child reachable through the graph-wide map.
    fn assert_consistent(graph: &FlameGraph) {
        for i in 0..graph.nodes().len() {
            let id = NodeId(i as u32);
            let node = &graph[id];
            let totals: Vec<i64> = node
                .children
                .iter()
                .map(|&c| graph[c].total_value)
                .collect();
            assert!(
                totals.windows(2).all(|w| w[0] >= w[1]),
                "{:?}",
                names(graph, id)
            );
            for (pos, &child) in node.children.iter().enumerate() {
                assert_eq!(graph[child].position, pos);
                assert_eq!(graph[child].parent, Some(id));
                assert_eq!(graph.child(id, &graph[child].name), Some(child));
            }
        }
    }

    #[test]
    fn a_growing_child_overtakes_lighter_siblings() {
        let mut graph = FlameGraph::new();
        for (name, weight) in [("a", 5), ("b", 3), ("c", 1)] {
            graph.add_stack(&[name.into()], weight);
        }
        assert_eq!(names(&graph, FlameGraph::ROOT), ["a", "b", "c"]);

        let c = path(&graph, &["c"]);
        graph.add_stack(&["c".into(), "leaf".into()], 10);
        assert_eq!(names(&graph, FlameGraph::ROOT), ["c", "a", "b"]);
        assert_eq!(path(&graph, &["c"]), c, "ids survive reordering");
        assert_eq!(graph[c].total_value, 11);
        assert_consistent(&graph);
    }

    #[test]
    fn equal_weights_keep_arrival_order() {
        let mut graph = FlameGraph::new();
        for name in ["x", "y", "z"] {
            graph.add_stack(&[name.into()], 2);
        }
        assert_eq!(names(&graph, FlameGraph::ROOT), ["x", "y", "z"]);
    }

    #[test]
    fn totals_and_self_time_accumulate_along_the_stack() {
        let mut graph = FlameGraph::new();
        graph.add_stack(&["t".into(), "main".into(), "work".into()], 4);
        graph.add_stack(&["t".into(), "main".into()], 1);
        let main = path(&graph, &["t", "main"]);
        assert_eq!((graph[main].total_value, graph[main].self_value), (5, 1));
        assert_eq!(graph[FlameGraph::ROOT].total_value, 5);
        assert_eq!(graph.nodes().len(), 4);
    }

    #[test]
    fn labels_are_scoped_by_parent() {
        let mut graph = FlameGraph::new();
        graph.add_stack(&["t1".into(), "main".into()], 1);
        graph.add_stack(&["t2".into(), "main".into()], 1);
        let (m1, m2) = (path(&graph, &["t1", "main"]), path(&graph, &["t2", "main"]));
        assert_ne!(m1, m2);
        assert_eq!(graph.labels.len(), 3, "\"main\" is interned once");
        assert_consistent(&graph);
    }

    #[test]
    fn depth_counts_steps_to_an_ancestor() {
        let mut graph = FlameGraph::new();
        graph.add_stack(&["t".into(), "a".into(), "b".into()], 1);
        let (t, b) = (path(&graph, &["t"]), path(&graph, &["t", "a", "b"]));
        assert_eq!(graph.depth(b, FlameGraph::ROOT), Some(3));
        assert_eq!(graph.depth(b, t), Some(2));
        assert_eq!(graph.depth(t, b), None);
    }
}

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::flamegraph::{FlameGraph, FlameNode, NodeId, SampleType, SampledStack, TypedStacks};
use crate::tui::theme;
use crate::tui::widgets::{Picker, PickerEvent, PickerStyle};

pub const SEARCH_KEYS: &[(&str, &str)] = &[
    ("[Esc]", " cancel "),
    ("[Enter]", " select "),
    ("[↑↓]", " navigate "),
];

pub static PROCESS_PICKER: PickerStyle = PickerStyle {
    title: " process ",
    placeholder: "type to filter processes...",
    border: theme::POPUP_BORDER,
    width: 50,
    max_visible: 3,
    keys: SEARCH_KEYS,
};

/// The frame under the cursor, derived from the graph on demand.
pub struct Selected<'a> {
    pub node: &'a FlameNode,
    /// Steps below the zoom root.
    pub depth: usize,
}

pub struct FlamegraphTab {
    /// The shown graph, of `sample_type`.
    pub graph: FlameGraph,
    pub sample_type: Option<SampleType>,
    pub frozen: bool,
    pub profiles_received: u64,
    /// Weight received for `sample_type`.
    pub samples_received: u64,
    /// The other sample types' graphs and weights, switched in with `t`.
    others: Vec<(SampleType, FlameGraph, u64)>,
    /// First visible depth row.
    pub scroll_y: usize,
    /// Top of the visible subtree.
    pub zoom: NodeId,
    /// Selected frame; always in the zoom root's subtree. Node ids are stable,
    /// so the selection stays on its frame when updates reorder siblings.
    pub cursor: NodeId,
    pub picker: Option<Picker>,
}

impl Default for FlamegraphTab {
    fn default() -> Self {
        Self {
            graph: FlameGraph::new(),
            sample_type: None,
            frozen: false,
            profiles_received: 0,
            samples_received: 0,
            others: Vec::new(),
            scroll_y: 0,
            zoom: FlameGraph::ROOT,
            cursor: FlameGraph::ROOT,
            picker: None,
        }
    }
}

impl FlamegraphTab {
    /// Add one request's profiles to their sample type's graph, unless frozen.
    pub fn ingest(&mut self, profiles: &[TypedStacks]) {
        if self.frozen {
            return;
        }
        for p in profiles {
            // The first sample type seen is shown
            let shown = self
                .sample_type
                .get_or_insert_with(|| p.sample_type.clone());
            if *shown == p.sample_type {
                add_stacks(&mut self.graph, &p.stacks);
                self.samples_received += p.samples;
                continue;
            }
            let other = match self.others.iter().position(|(t, ..)| *t == p.sample_type) {
                Some(i) => &mut self.others[i],
                None => {
                    self.others
                        .push((p.sample_type.clone(), FlameGraph::new(), 0));
                    self.others.last_mut().expect("just pushed")
                }
            };
            add_stacks(&mut other.1, &p.stacks);
            other.2 += p.samples;
        }
        self.profiles_received += 1;
    }

    /// Show the next sample type's graph (`t`); the view starts at its root.
    fn next_sample_type(&mut self) {
        if self.others.is_empty() {
            return;
        }
        let (sample_type, graph, samples) = self.others.remove(0);
        let shown_type = self.sample_type.replace(sample_type);
        let shown_graph = std::mem::replace(&mut self.graph, graph);
        let shown_samples = std::mem::replace(&mut self.samples_received, samples);
        if let Some(shown_type) = shown_type {
            self.others.push((shown_type, shown_graph, shown_samples));
        }
        self.scroll_y = 0;
        self.zoom = FlameGraph::ROOT;
        self.cursor = FlameGraph::ROOT;
    }

    pub fn zoom_root(&self) -> &FlameNode {
        &self.graph[self.zoom]
    }

    pub fn selected(&self) -> Selected<'_> {
        Selected {
            node: &self.graph[self.cursor],
            depth: self.cursor_depth(),
        }
    }

    /// Steps from the zoom root down to the cursor.
    pub fn cursor_depth(&self) -> usize {
        self.graph.depth(self.cursor, self.zoom).unwrap_or(0)
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.picker.is_some() {
            return self.handle_picker_key(key);
        }
        match key.code {
            KeyCode::Char('f') | KeyCode::Char(' ') => self.frozen = !self.frozen,
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::Left | KeyCode::Char('h') => self.move_sideways(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_sideways(1),
            KeyCode::Enter => self.zoom_in(),
            KeyCode::Esc | KeyCode::Backspace => self.zoom_out(),
            KeyCode::Char('r') => self.reset(),
            KeyCode::Char('t') => self.next_sample_type(),
            KeyCode::Char('/') => self.open_search(),
            _ => {}
        };
    }

    fn open_search(&mut self) {
        let mut picker = Picker::new(&PROCESS_PICKER);
        picker.refresh(self.process_names());
        self.picker = Some(picker);
    }

    fn process_names(&self) -> impl Iterator<Item = &str> {
        let root = &self.graph[FlameGraph::ROOT];
        root.children.iter().map(|&p| &*self.graph[p].name)
    }

    fn handle_picker_key(&mut self, key: KeyEvent) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        match picker.handle_key(key) {
            Some(PickerEvent::Changed) => {
                let root = &self.graph[FlameGraph::ROOT];
                picker.refresh(root.children.iter().map(|&p| &*self.graph[p].name));
            }
            Some(PickerEvent::Cancel) => self.picker = None,
            Some(PickerEvent::Submit) => {
                let picked = self.picker.take();
                let process = picked
                    .as_ref()
                    .and_then(|p| p.selected())
                    .and_then(|name| self.graph.child(FlameGraph::ROOT, name));
                if let Some(process) = process {
                    self.zoom_to(process);
                }
            }
            None => {}
        }
    }

    fn move_down(&mut self) {
        if let Some(&first) = self.graph[self.cursor].children.first() {
            self.cursor = first;
        }
    }

    fn move_up(&mut self) {
        if self.cursor != self.zoom
            && let Some(parent) = self.graph[self.cursor].parent
        {
            self.cursor = parent;
        }
    }

    /// Move to the previous (`-1`) or next (`1`) sibling, if there is one.
    fn move_sideways(&mut self, step: isize) {
        if self.cursor == self.zoom {
            return;
        }
        let node = &self.graph[self.cursor];
        let Some(parent) = node.parent else {
            return;
        };
        let siblings = &self.graph[parent].children;
        if let Some(&sibling) = node
            .position()
            .checked_add_signed(step)
            .and_then(|pos| siblings.get(pos))
        {
            self.cursor = sibling;
        }
    }

    fn zoom_in(&mut self) {
        if self.cursor != self.zoom {
            self.zoom_to(self.cursor);
        }
    }

    fn zoom_out(&mut self) {
        if let Some(parent) = self.graph[self.zoom].parent {
            self.zoom_to(parent);
        }
    }

    fn zoom_to(&mut self, id: NodeId) {
        self.zoom = id;
        self.cursor = id;
        self.scroll_y = 0;
    }

    fn reset(&mut self) {
        *self = Self {
            frozen: self.frozen,
            ..Self::default()
        };
    }
}

fn add_stacks(graph: &mut FlameGraph, stacks: &[SampledStack]) {
    for stack in stacks {
        graph.add_stack(&stack.frames, stack.weight);
    }
}

#[derive(Clone, Debug)]
pub struct FlameNode {
    pub name: String,
    pub total_value: i64,
    pub self_value: i64,
    pub children: Vec<FlameNode>,
}

impl FlameNode {
    pub fn new(name: String) -> Self {
        Self {
            name,
            total_value: 0,
            self_value: 0,
            children: Vec::new(),
        }
    }

    pub fn add_stack(&mut self, stack: &[String], value: i64) {
        self.total_value += value;
        if stack.is_empty() {
            self.self_value += value;
            return;
        }
        let pos = self.children.iter().position(|c| c.name == stack[0]);
        let child = if let Some(pos) = pos {
            &mut self.children[pos]
        } else {
            self.children.push(FlameNode::new(stack[0].clone()));
            self.children.last_mut().unwrap()
        };
        child.add_stack(&stack[1..], value);
    }

    pub fn merge(&mut self, other: FlameNode) {
        self.total_value += other.total_value;
        self.self_value += other.self_value;
        for other_child in other.children {
            let pos = self
                .children
                .iter()
                .position(|c| c.name == other_child.name);
            if let Some(pos) = pos {
                self.children[pos].merge(other_child);
            } else {
                self.children.push(other_child);
            }
        }
    }

    pub fn sort_recursive(&mut self) {
        self.children
            .sort_by(|a, b| b.total_value.cmp(&a.total_value));
        for child in &mut self.children {
            child.sort_recursive();
        }
    }

    #[allow(dead_code)]
    pub fn max_depth(&self) -> usize {
        if self.children.is_empty() {
            0
        } else {
            1 + self
                .children
                .iter()
                .map(|c| c.max_depth())
                .max()
                .unwrap_or(0)
        }
    }
}

#[derive(Clone, Debug)]
pub struct FlameGraph {
    pub root: FlameNode,
}

impl FlameGraph {
    pub fn new() -> Self {
        Self {
            root: FlameNode::new("all".to_string()),
        }
    }

    pub fn add_stack(&mut self, stack: &[String], value: i64) {
        self.root.add_stack(stack, value);
    }
}

pub fn get_zoom_node<'a>(root: &'a FlameNode, zoom_path: &[String]) -> &'a FlameNode {
    let mut node = root;
    for name in zoom_path {
        if let Some(child) = node.children.iter().find(|c| c.name == *name) {
            node = child;
        }
    }
    node
}

pub fn get_node<'a>(root: &'a FlameNode, index_path: &[usize]) -> &'a FlameNode {
    let mut node = root;
    for &idx in index_path {
        if idx < node.children.len() {
            node = &node.children[idx];
        }
    }
    node
}

pub struct FrameRect {
    pub x: u16,
    pub width: u16,
    pub depth: usize,
    pub name: String,
    pub self_value: i64,
    pub total_value: i64,
    pub palette_index: usize,
}

pub fn thread_rank(root: &FlameNode, thread_name: &str) -> usize {
    root.children
        .iter()
        .position(|c| c.name == thread_name)
        .unwrap_or(0)
}

pub fn layout_frames(
    node: &FlameNode,
    area_width: u16,
    forced_palette: Option<usize>,
) -> Vec<FrameRect> {
    if node.total_value <= 0 {
        return Vec::new();
    }
    let scale = area_width as f64 / node.total_value as f64;
    let mut frames = Vec::new();
    layout_recursive(node, 0.0, 0, scale, forced_palette, &mut frames);
    frames
}

fn layout_recursive(
    node: &FlameNode,
    x_float: f64,
    depth: usize,
    scale: f64,
    palette: Option<usize>,
    frames: &mut Vec<FrameRect>,
) {
    let x_end = x_float + node.total_value as f64 * scale;
    let x = x_float.round() as u16;
    let width = (x_end.round() as u16).saturating_sub(x);

    if width == 0 {
        return;
    }

    let palette_index = palette.unwrap_or(0);

    frames.push(FrameRect {
        x,
        width,
        depth,
        name: node.name.clone(),
        self_value: node.self_value,
        total_value: node.total_value,
        palette_index,
    });

    let mut child_x = x_float;
    for (i, child) in node.children.iter().enumerate() {
        let child_palette = if depth == 0 && palette.is_none() {
            Some(i)
        } else {
            Some(palette_index)
        };
        layout_recursive(child, child_x, depth + 1, scale, child_palette, frames);
        child_x += child.total_value as f64 * scale;
    }
}

pub fn cursor_frame_rect(
    zoom_root: &FlameNode,
    cursor_path: &[usize],
    area_width: u16,
    forced_palette: Option<usize>,
) -> Option<FrameRect> {
    if zoom_root.total_value <= 0 {
        return None;
    }
    let scale = area_width as f64 / zoom_root.total_value as f64;
    let mut node = zoom_root;
    let mut x_acc = 0.0;
    let mut palette_index = forced_palette.unwrap_or(0);

    for (step, &idx) in cursor_path.iter().enumerate() {
        for i in 0..idx.min(node.children.len()) {
            x_acc += node.children[i].total_value as f64 * scale;
        }
        if idx < node.children.len() {
            if step == 0 && forced_palette.is_none() {
                palette_index = idx;
            }
            node = &node.children[idx];
        } else {
            return None;
        }
    }

    let x = x_acc.round() as u16;
    let width = (node.total_value as f64 * scale).round().max(1.0) as u16;

    Some(FrameRect {
        x,
        width,
        depth: cursor_path.len(),
        name: node.name.clone(),
        self_value: node.self_value,
        total_value: node.total_value,
        palette_index,
    })
}

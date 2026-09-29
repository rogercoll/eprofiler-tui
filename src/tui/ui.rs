use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::draw::{self, center, fill, format_count, hline, truncate};
use super::flamescope_layout::FlamescopeLayout;
use super::state::{ActiveTab, ExecutablesTab, FlamegraphTab, FlamescopeTab, State};
use super::theme::{self, Gradient, blend, bold, contrast_fg, darken, gradient, italic, lighten};
use super::widgets::fit_offset;
use crate::flamegraph::{cursor_frame_rect, layout_frames};

pub fn render(state: &mut State, frame: &mut Frame) {
    let area = frame.area();

    if state.fg.graph.root.total_value == 0 && state.active_tab == ActiveTab::Flamegraph {
        render_waiting(frame, area, &state.listen_addr);
        return;
    }

    let [header, detail, body, footer] = Layout::new(
        Direction::Vertical,
        [
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ],
    )
    .areas(area);

    render_header(state, frame, header);

    match state.active_tab {
        ActiveTab::Flamegraph => {
            render_detail_bar(&state.fg, frame, detail);
            render_flamegraph(&mut state.fg, frame, body);
        }
        ActiveTab::Flamescope => {
            render_flamescope_detail_bar(&state.fs, frame, detail);
            render_flamescope(&state.fs, frame, body);
        }
        ActiveTab::Executables => {
            render_exe_status_bar(state.exe.status.as_deref(), frame, detail);
            render_exe_table(&mut state.exe, frame, body);
        }
    }

    let hints = state
        .active_picker()
        .map_or(tab_keys(state.active_tab), |p| p.style.keys);
    frame.render_widget(Paragraph::new(draw::key_hints(hints)), footer);

    if let Some(picker) = state.active_picker() {
        frame.render_widget(picker, body);
    }
}

// ---------------------------------------------------------------------------
// Waiting screen
// ---------------------------------------------------------------------------

const LOGO: &[&str] = &[
    " ███████╗██████╗ ██████╗  ██████╗ ███████╗██╗██╗     ███████╗██████╗       ████████╗██╗   ██╗██╗",
    " ██╔════╝██╔══██╗██╔══██╗██╔═══██╗██╔════╝██║██║     ██╔════╝██╔══██╗      ╚══██╔══╝██║   ██║██║",
    " █████╗  ██████╔╝██████╔╝██║   ██║█████╗  ██║██║     █████╗  ██████╔╝█████╗   ██║   ██║   ██║██║",
    " ██╔══╝  ██╔═══╝ ██╔══██╗██║   ██║██╔══╝  ██║██║     ██╔══╝  ██╔══██╗╚════╝   ██║   ██║   ██║██║",
    " ███████╗██║     ██║  ██║╚██████╔╝██║     ██║███████╗███████╗██║  ██║         ██║   ╚██████╔╝██║",
    " ╚══════╝╚═╝     ╚═╝  ╚═╝ ╚═════╝ ╚═╝     ╚═╝╚══════╝╚══════╝╚═╝  ╚═╝         ╚═╝    ╚═════╝ ╚═╝",
];

const LOGO_GRADIENT: &[Color] = &[
    Color::Rgb(168, 50, 160),
    Color::Rgb(200, 40, 80),
    Color::Rgb(220, 50, 32),
    Color::Rgb(240, 100, 18),
    Color::Rgb(250, 170, 30),
    theme::YELLOW,
];

fn render_waiting(frame: &mut Frame, area: Rect, listen_addr: &str) {
    let buf = frame.buffer_mut();
    let on_bg = |style: Style| style.bg(theme::BG);
    fill(buf, area, on_bg(Style::reset()));

    let art_h = LOGO.len() as u16;
    let art_w = LOGO.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
    let total_h = art_h + 5;
    let top = area.y + area.height.saturating_sub(total_h) / 2;

    if area.width >= art_w + 2 && area.height >= total_h {
        let x = area.x + area.width.saturating_sub(art_w) / 2;
        for (i, line) in LOGO.iter().enumerate() {
            let color = LOGO_GRADIENT[i % LOGO_GRADIENT.len()];
            buf.set_string(x, top + i as u16, line, on_bg(Style::default().fg(color)));
        }
    } else {
        center(
            buf,
            area,
            top + 1,
            "◆ eprofiler-tui",
            on_bg(bold(LOGO_GRADIENT[4])),
        );
    }

    let subtitle = "OTLP Profile Flamegraph Viewer";
    let base_y = top + art_h + 1;
    center(buf, area, base_y, subtitle, on_bg(bold(theme::BRIGHT)));
    let rule_w = subtitle.len() as u16;
    let rule_x = area.x + area.width.saturating_sub(rule_w) / 2;
    hline(
        buf,
        base_y + 1,
        rule_x,
        rule_x + rule_w,
        '─',
        on_bg(Style::default().fg(theme::RULE)),
    );
    center(
        buf,
        area,
        base_y + 2,
        &format!("Listening on {listen_addr}"),
        on_bg(Style::default().fg(theme::MUTED)),
    );
    center(
        buf,
        area,
        base_y + 3,
        "Waiting for profiles...",
        on_bg(italic(theme::DIM)),
    );
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

const TABS: &[(&str, ActiveTab)] = &[
    ("Flamegraph", ActiveTab::Flamegraph),
    ("Flamescope", ActiveTab::Flamescope),
    ("Executables", ActiveTab::Executables),
];

fn sep() -> Span<'static> {
    " │ ".fg(theme::FAINT)
}

fn render_header(state: &State, frame: &mut Frame, area: Rect) {
    let left = Line::from(vec![
        " ◆ ".fg(theme::ACCENT),
        "eprofiler-tui".fg(theme::BRIGHT).bold(),
        sep(),
        state.listen_addr.clone().fg(theme::MUTED),
        sep(),
        format!("{} profiles", state.fg.profiles_received).fg(theme::MUTED_DARK),
        sep(),
        format!("{} samples", format_count(state.fg.samples_received)).fg(theme::MUTED_DARK),
    ]);
    frame.render_widget(Paragraph::new(left), area);

    let buf = frame.buffer_mut();

    let (indicator, color) = if state.fg.frozen {
        (" ⏸ FROZEN ", theme::WARNING)
    } else {
        (" ▶ LIVE ", theme::SUCCESS)
    };
    let indicator_x = area
        .right()
        .saturating_sub(indicator.chars().count() as u16);
    buf.set_string(indicator_x, area.y, indicator, bold(color));

    // Right-aligned tab strip, just left of the indicator. Widths are in
    // columns, not bytes: the separator glyph is multi-byte.
    let tab_sep = " │ ";
    let sep_w = tab_sep.chars().count() as u16;
    let tabs_width: u16 =
        TABS.iter().map(|(l, _)| l.len() as u16).sum::<u16>() + sep_w * (TABS.len() as u16 - 1) + 2;
    let mut x = indicator_x.saturating_sub(tabs_width) + 1;
    for (i, &(label, tab)) in TABS.iter().enumerate() {
        if i > 0 {
            buf.set_string(x, area.y, tab_sep, Style::default().fg(theme::FAINT));
            x += sep_w;
        }
        let style = if state.active_tab == tab {
            bold(theme::BRIGHT)
        } else {
            Style::default().fg(theme::DIM)
        };
        buf.set_string(x, area.y, label, style);
        x += label.len() as u16;
    }
}

/// Centered, italic placeholder for views with nothing to show yet.
fn render_empty(buf: &mut Buffer, area: Rect, msg: &str) {
    center(
        buf,
        area,
        area.y + area.height / 2,
        msg,
        italic(theme::SUBTLE),
    );
}

// ---------------------------------------------------------------------------
// Flamegraph
// ---------------------------------------------------------------------------

fn render_detail_bar(fg: &FlamegraphTab, frame: &mut Frame, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();

    if let Some(zoomed) = fg.zoom_path.last() {
        spans.push(format!(" zoomed: {zoomed} ").fg(theme::ACCENT).bold());
    }

    if let Some(sel) = fg.selected() {
        if !spans.is_empty() {
            spans.push(sep());
        }
        let root_total = fg.zoom_root().total_value;
        let pct = |v: i64| {
            if root_total > 0 {
                v as f64 / root_total as f64 * 100.0
            } else {
                0.0
            }
        };
        let stat = |v: i64| format!("{} ({:.1}%)", format_count(v as u64), pct(v));

        spans.extend([
            " ▸ ".fg(theme::ACCENT).bold(),
            truncate(&sel.node.name, 40).fg(theme::BRIGHT).bold(),
            sep(),
            "self: ".fg(theme::DIM),
            stat(sel.node.self_value).fg(theme::ORANGE),
            sep(),
            "total: ".fg(theme::DIM),
            stat(sel.node.total_value).fg(theme::WARNING),
            sep(),
            "depth: ".fg(theme::DIM),
            sel.depth.to_string().fg(theme::MUTED),
        ]);
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_flamegraph(fg: &mut FlamegraphTab, frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();

    if area.width < 4 || area.height < 2 {
        return;
    }

    // Field path rather than `fg.zoom_root()` so `fg.scroll_y` stays assignable below.
    let zoom_root = fg.graph.root.follow_path(&fg.zoom_path);
    if zoom_root.total_value <= 0 {
        render_empty(buf, area, "No profile data yet");
        return;
    }

    let forced_palette = fg
        .zoom_path
        .first()
        .and_then(|name| fg.graph.root.child_position(name));

    let frames = layout_frames(zoom_root, area.width, forced_palette);
    let max_depth = frames.iter().map(|f| f.depth).max().unwrap_or(0);
    let viewport = area.height as usize;
    let root_total = zoom_root.total_value;

    // Keep the cursor's depth row on screen, then avoid trailing blank rows.
    fg.scroll_y = fit_offset(fg.scroll_y, fg.cursor_path.len(), viewport)
        .min(max_depth.saturating_sub(viewport.saturating_sub(1)));
    let scroll_y = fg.scroll_y;

    let cursor_rect = cursor_frame_rect(zoom_root, &fg.cursor_path, area.width, forced_palette);

    for fr in frames
        .iter()
        .filter(|f| (scroll_y..scroll_y + viewport).contains(&f.depth))
    {
        let screen_y = area.y + (fr.depth - scroll_y) as u16;
        let is_cursor = cursor_rect
            .as_ref()
            .is_some_and(|cr| cr.depth == fr.depth && cr.x == fr.x);

        let heat = if fr.total_value > 0 {
            fr.self_value as f64 / fr.total_value as f64
        } else {
            0.0
        };
        let mut bg = flame_color(&fr.name, heat, fr.palette_index);
        if is_cursor {
            bg = lighten(bg, 45);
        }
        let fg_color = contrast_fg(bg);

        let x_start = area.x + fr.x;
        let x_end = (x_start + fr.width).min(area.right());
        fill(
            buf,
            Rect::new(x_start, screen_y, x_end.saturating_sub(x_start), 1),
            Style::default().bg(bg),
        );
        for x in [x_start, x_end.saturating_sub(1)] {
            if let Some(cell) = buf.cell_mut((x, screen_y)) {
                cell.set_char('▏');
                cell.set_style(Style::default().fg(darken(bg, 55)).bg(bg));
            }
        }

        let inner_width = fr.width.saturating_sub(2) as usize;
        if inner_width >= 3 {
            let name = truncate(&fr.name, inner_width);
            let pad = inner_width.saturating_sub(name.chars().count()) / 2;
            let mut style = Style::default().fg(fg_color).bg(bg);
            if is_cursor {
                style = style.add_modifier(Modifier::BOLD);
            }
            buf.set_string(x_start + 1 + pad as u16, screen_y, &name, style);
        }

        if fr.width >= 14 && root_total > 0 {
            let pct = fr.total_value as f64 / root_total as f64 * 100.0;
            let pct_str = format!("{pct:.1}%");
            let pct_x = x_start + fr.width - pct_str.len() as u16 - 2;
            if pct >= 0.1 && pct_x > x_start + 2 {
                let dim_fg = blend(fg_color, bg, 0.45);
                buf.set_string(
                    pct_x,
                    screen_y,
                    &pct_str,
                    Style::default().fg(dim_fg).bg(bg),
                );
            }
        }

        if is_cursor
            && fr.width >= 3
            && let Some(cell) = buf.cell_mut((x_start + 1, screen_y))
        {
            cell.set_char('▸');
            cell.set_style(bold(Color::White).bg(bg));
        }
    }

    // Dotted filler for depth rows that have no frame wide enough to draw.
    for vis_d in 0..viewport {
        let depth = scroll_y + vis_d;
        if depth <= max_depth && !frames.iter().any(|f| f.depth == depth) {
            let y = area.y + vis_d as u16;
            hline(
                buf,
                y,
                area.left(),
                area.right(),
                '·',
                Style::default().fg(theme::GHOST),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Flamescope (subsecond-offset heatmap)
// ---------------------------------------------------------------------------

const HEATMAP_STOPS: &Gradient = &[
    (0.00, (13, 8, 135)),
    (0.25, (126, 3, 168)),
    (0.50, (204, 71, 120)),
    (0.75, (249, 149, 64)),
    (1.00, (252, 255, 164)),
];

fn heatmap_color(value: u64, max: u64) -> Color {
    let t = ((value as f64).sqrt() / (max as f64).sqrt()).clamp(0.0, 1.0);
    let (r, g, b) = gradient(t, HEATMAP_STOPS);
    Color::Rgb(r, g, b)
}

fn render_flamescope_detail_bar(fs: &FlamescopeTab, frame: &mut Frame, area: Rect) {
    if fs.visible_columns().is_empty() {
        return;
    }

    let (sec, ms_start, ms_end) = fs.selected_time();
    let mut spans: Vec<Span> = Vec::new();

    if let Some(filter) = &fs.filter {
        spans.push(format!(" filtered: {filter} ").fg(theme::ACCENT).bold());
        spans.push(sep());
    }

    spans.extend([
        " ▸ ".fg(theme::ACCENT).bold(),
        format!("{sec}s + {ms_start}\u{2013}{ms_end}ms")
            .fg(theme::BRIGHT)
            .bold(),
        sep(),
        "samples: ".fg(theme::DIM),
        fs.selected_value().to_string().fg(theme::ORANGE),
        sep(),
        "peak: ".fg(theme::DIM),
        fs.visible_peak().to_string().fg(theme::WARNING),
        sep(),
        "duration: ".fg(theme::DIM),
        format!("{}s", fs.total_seconds()).fg(theme::MUTED),
    ]);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_flamescope(fs: &FlamescopeTab, frame: &mut Frame, area: Rect) {
    let Some(lay) = FlamescopeLayout::new(area) else {
        return;
    };
    let buf = frame.buffer_mut();

    let data = fs.visible_columns();
    if data.is_empty() {
        let msg = if fs.is_empty() {
            "No profile data yet"
        } else {
            "No data for this thread"
        };
        render_empty(buf, area, msg);
        return;
    }
    let peak = fs.visible_peak();

    for row in 0..lay.num_rows() {
        let y_start = lay.row_y(row);
        if y_start >= lay.bottom() {
            break;
        }

        let label_y = y_start + lay.cell_h / 2;
        if label_y < lay.bottom() {
            let label = format!("{:>3}ms ", lay.ms_label(row));
            buf.set_string(
                lay.label_x(),
                label_y,
                label,
                Style::default().fg(theme::DIM),
            );
        }

        for col_off in 0..lay.visible_cols {
            let col = fs.col.offset + col_off;
            let value = data.get(col).map_or(0, |c| c[row]);
            let is_cursor = col == fs.col.index && row == fs.row;

            let bg = match (value > 0 && peak > 0, is_cursor) {
                (true, true) => lighten(heatmap_color(value, peak), 50),
                (true, false) => heatmap_color(value, peak),
                (false, true) => theme::HIGHLIGHT_BG,
                (false, false) => continue,
            };

            let cell =
                Rect::new(lay.cell_x(col_off), y_start, lay.cell_w, lay.cell_h).intersection(area);
            fill(buf, cell, Style::default().bg(bg));
        }
    }
}

// ---------------------------------------------------------------------------
// Executables
// ---------------------------------------------------------------------------

fn render_exe_status_bar(status: Option<&str>, frame: &mut Frame, area: Rect) {
    let Some(status) = status else { return };

    let is_loading = status.starts_with("Loading") || status.starts_with("Removing");
    let (text, color) = if is_loading {
        (format!("{status}..."), theme::WARNING)
    } else if status.starts_with("Error") {
        (status.to_owned(), theme::ERROR)
    } else {
        (status.to_owned(), theme::SUCCESS)
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![" ".into(), text.fg(color)])),
        area,
    );
}

fn render_exe_table(exe: &mut ExecutablesTab, frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();

    if area.height < 2 {
        return;
    }

    let col_id_w = 34u16.min(area.width / 3);
    let col_sym_w = 12u16;
    let col_name_w = area.width.saturating_sub(col_id_w + col_sym_w + 4);
    let (id_x, name_x, sym_x) = (
        area.x + 1,
        area.x + 1 + col_id_w,
        area.x + 1 + col_id_w + col_name_w,
    );

    let hdr = bold(theme::DIM);
    buf.set_string(id_x, area.y, "File ID", hdr);
    buf.set_string(name_x, area.y, "Name", hdr);
    buf.set_string(sym_x, area.y, "Symbols", hdr);

    let sep_y = area.y + 1;
    hline(
        buf,
        sep_y,
        area.left(),
        area.right(),
        '─',
        Style::default().fg(theme::RULE),
    );

    let rows_y = sep_y + 1;
    let visible_rows = area.bottom().saturating_sub(rows_y) as usize;
    if visible_rows == 0 {
        return;
    }

    if exe.list.is_empty() {
        buf.set_string(
            area.x + 2,
            rows_y + 1,
            "No executables discovered. Waiting for profiles...",
            italic(theme::DIM),
        );
        return;
    }

    exe.cursor.scroll_to_fit(visible_rows);
    for (vis_row, idx) in exe.cursor.visible(visible_rows, exe.list.len()).enumerate() {
        let entry = &exe.list[idx];
        let y = rows_y + vis_row as u16;
        let is_cursor = idx == exe.cursor.index;
        let is_sym = entry.num_ranges.is_some();
        let row_bg = if is_cursor {
            theme::HIGHLIGHT_BG
        } else {
            Color::Reset
        };
        let on_row = |fg: Color| Style::default().fg(fg).bg(row_bg);

        if is_cursor {
            fill(
                buf,
                Rect::new(area.x, y, area.width, 1),
                Style::default().bg(row_bg),
            );
        }

        let id_str = entry.file_id.map_or_else(
            || "N/A".to_string(),
            |id| truncate(&id.format_hex(), col_id_w as usize - 1),
        );
        let id_fg = if is_sym {
            theme::MUTED_DARK
        } else {
            theme::SUBTLE
        };
        buf.set_string(id_x, y, &id_str, on_row(id_fg));

        let prefix = if is_cursor { "▸ " } else { "  " };
        let name = truncate(
            &entry.name,
            (col_name_w as usize).saturating_sub(prefix.len() + 1),
        );
        let name_fg = match (is_cursor, is_sym) {
            (true, _) => theme::BRIGHT,
            (false, true) => theme::TEXT,
            (false, false) => theme::MUTED,
        };
        let mut name_style = on_row(name_fg);
        if is_cursor {
            name_style = name_style.add_modifier(Modifier::BOLD);
        }
        buf.set_string(name_x, y, format!("{prefix}{name}"), name_style);

        let sym_str = entry
            .num_ranges
            .map_or("N/A".to_string(), |n| format_count(n as u64));
        let sym_fg = if is_sym {
            theme::SUCCESS
        } else {
            theme::SUBTLE
        };
        buf.set_string(sym_x, y, &sym_str, on_row(sym_fg));
    }
}

// ---------------------------------------------------------------------------
// Footer key hints
// ---------------------------------------------------------------------------

const FLAMEGRAPH_KEYS: &[(&str, &str)] = &[
    ("[Tab]", " switch "),
    ("[q]", " quit "),
    ("[f/Space]", " freeze "),
    ("[j/↓ k/↑]", " depth "),
    ("[h/← l/→]", " frame "),
    ("[Enter]", " zoom "),
    ("[Esc]", " back "),
    ("[/]", " search "),
    ("[r]", " reset "),
];

const FLAMESCOPE_KEYS: &[(&str, &str)] = &[
    ("[Tab]", " switch "),
    ("[q]", " quit "),
    ("[h/← l/→]", " time "),
    ("[j/↓ k/↑]", " offset "),
    ("[/]", " filter "),
    ("[Esc]", " unfilter "),
    ("[G]", " latest "),
    ("[r]", " reset "),
];

const EXE_KEYS: &[(&str, &str)] = &[
    ("[Tab]", " switch "),
    ("[j/k]", " navigate "),
    ("[Enter]", " symbolize "),
    ("[r]", " remove "),
    ("[/]", " add new "),
    ("[q]", " quit "),
];

fn tab_keys(tab: ActiveTab) -> &'static [(&'static str, &'static str)] {
    match tab {
        ActiveTab::Flamegraph => FLAMEGRAPH_KEYS,
        ActiveTab::Flamescope => FLAMESCOPE_KEYS,
        ActiveTab::Executables => EXE_KEYS,
    }
}

// ---------------------------------------------------------------------------
// Flame colors
// ---------------------------------------------------------------------------

const PALETTES: &[&Gradient] = &[
    &[
        (0.00, (253, 224, 71)),
        (0.25, (251, 191, 36)),
        (0.45, (249, 115, 22)),
        (0.65, (234, 88, 12)),
        (0.80, (220, 38, 38)),
        (1.00, (185, 28, 28)),
    ],
    &[
        (0.00, (252, 211, 77)),
        (0.25, (245, 158, 11)),
        (0.45, (217, 119, 6)),
        (0.65, (180, 83, 9)),
        (0.80, (146, 64, 14)),
        (1.00, (120, 53, 15)),
    ],
    &[
        (0.00, (253, 164, 175)),
        (0.25, (251, 113, 133)),
        (0.45, (244, 63, 94)),
        (0.65, (225, 29, 72)),
        (0.80, (190, 18, 60)),
        (1.00, (136, 19, 55)),
    ],
    &[
        (0.00, (190, 242, 100)),
        (0.25, (163, 230, 53)),
        (0.45, (132, 204, 22)),
        (0.65, (101, 163, 13)),
        (0.80, (77, 124, 15)),
        (1.00, (54, 83, 20)),
    ],
    &[
        (0.00, (153, 246, 228)),
        (0.25, (94, 234, 212)),
        (0.45, (20, 184, 166)),
        (0.65, (13, 148, 136)),
        (0.80, (15, 118, 110)),
        (1.00, (19, 78, 74)),
    ],
    &[
        (0.00, (147, 197, 253)),
        (0.25, (96, 165, 250)),
        (0.45, (59, 130, 246)),
        (0.65, (37, 99, 235)),
        (0.80, (29, 78, 216)),
        (1.00, (30, 58, 138)),
    ],
    &[
        (0.00, (165, 180, 252)),
        (0.25, (129, 140, 248)),
        (0.45, (99, 102, 241)),
        (0.65, (79, 70, 229)),
        (0.80, (67, 56, 202)),
        (1.00, (55, 48, 163)),
    ],
    &[
        (0.00, (216, 180, 254)),
        (0.25, (192, 132, 252)),
        (0.45, (168, 85, 247)),
        (0.65, (147, 51, 234)),
        (0.80, (126, 34, 206)),
        (1.00, (88, 28, 135)),
    ],
];

/// Palette color for `heat` (self/total ratio), jittered per frame name so
/// adjacent frames with equal heat stay distinguishable.
fn flame_color(name: &str, heat: f64, palette_index: usize) -> Color {
    let hash = name.bytes().fold(0u64, |h, b| {
        h.wrapping_mul(2654435761).wrapping_add(b as u64)
    });

    let stops = PALETTES[palette_index % PALETTES.len()];
    let (r, g, b) = gradient(heat, stops);

    let rv = ((hash % 18) as i16 - 9).clamp(-12, 12);
    let gv = (((hash >> 5) % 14) as i16 - 7).clamp(-10, 10);

    Color::Rgb(
        (r as i16 + rv).clamp(25, 255) as u8,
        (g as i16 + gv).clamp(20, 255) as u8,
        b,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    };

    use super::*;
    use crate::flamegraph::FlameGraph;
    use crate::tui::event::Event;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn populated_state() -> State {
        let mut state = State::new("0.0.0.0:4317".into(), vec![]);
        let mut fg = FlameGraph::new();
        fg.add_stack(&["worker-1".into(), "main".into(), "do_work".into()], 30);
        fg.add_stack(&["worker-2".into(), "main".into()], 10);
        let timestamps = HashMap::from([("worker-1".to_string(), vec![0u64, 1_500_000_000])]);
        state.handle_event(Event::ProfileUpdate {
            flamegraph: fg,
            samples: 40,
            timestamps,
        });
        state.handle_event(Event::MappingsDiscovered(vec![
            "libc.so.6".into(),
            "app".into(),
        ]));
        state
    }

    fn draw(state: &mut State, width: u16, height: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| render(state, f)).unwrap();
        term.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn waiting_screen_before_data() {
        let mut state = State::new("0.0.0.0:4317".into(), vec![]);
        assert!(draw(&mut state, 100, 20).contains("Listening on 0.0.0.0:4317"));
    }

    #[test]
    fn every_tab_renders_with_and_without_picker_at_awkward_sizes() {
        let mut state = populated_state();
        for (w, h) in [(120, 40), (40, 8), (12, 3)] {
            for _ in 0..3 {
                draw(&mut state, w, h);
                state.handle_event(key(KeyCode::Char('/')));
                assert!(state.active_picker().is_some());
                draw(&mut state, w, h);
                state.handle_event(key(KeyCode::Esc));
                assert!(state.active_picker().is_none());
                state.handle_event(key(KeyCode::Tab));
            }
        }
    }

    #[test]
    fn detail_bar_follows_cursor() {
        let mut state = populated_state();
        state.handle_event(key(KeyCode::Down));
        state.handle_event(key(KeyCode::Down));
        let screen = draw(&mut state, 100, 20);
        assert!(screen.contains("depth: 2"), "{screen}");
        assert!(screen.contains("▸ main"), "{screen}");
    }

    #[test]
    fn executables_table_lists_discovered_mappings() {
        let mut state = populated_state();
        state.handle_event(key(KeyCode::Tab));
        state.handle_event(key(KeyCode::Tab));
        let screen = draw(&mut state, 100, 20);
        assert!(screen.contains("libc.so.6"), "{screen}");
        assert!(screen.contains("▸ libc.so.6"), "{screen}");
    }
}

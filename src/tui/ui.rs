use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::state::{ActiveTab, State};
use crate::flamegraph::{cursor_frame_rect, get_zoom_node, layout_frames, thread_rank};

const BG: Color = Color::Rgb(16, 16, 22);
const ACCENT: Color = Color::Rgb(59, 130, 246);
const DIM: Color = Color::Rgb(70, 70, 85);
const BRIGHT: Color = Color::Rgb(220, 220, 235);
const SEP_COLOR: Color = Color::Rgb(35, 35, 45);

pub fn render(state: &mut State, frame: &mut Frame) {
    let area = frame.area();

    if state.flamegraph.root.total_value == 0 && state.active_tab == ActiveTab::Flamegraph {
        render_waiting(frame, area, &state.listen_addr);
        return;
    }

    let chunks = Layout::new(
        Direction::Vertical,
        [
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ],
    )
    .split(area);

    render_header(state, frame, chunks[0]);

    match state.active_tab {
        ActiveTab::Flamegraph => {
            render_detail_bar(state, frame, chunks[1]);
            render_flamegraph(state, frame, chunks[2]);
            render_flamegraph_footer(state, frame, chunks[3]);

            if state.search_active {
                render_search_overlay(state, frame, chunks[2]);
            }
        }
        ActiveTab::Executables => {
            render_exe_status_bar(state, frame, chunks[1]);
            render_exe_table(state, frame, chunks[2]);
            render_exe_footer(state, frame, chunks[3]);

            if state.exe_input_active {
                render_exe_input_overlay(state, frame, chunks[2]);
            }
        }
    }
}

fn render_waiting(frame: &mut Frame, area: Rect, listen_addr: &str) {
    let buf = frame.buffer_mut();
    fill(buf, area, BG);

    let art: &[&str] = &[
        " ███████╗██████╗ ██████╗  ██████╗ ███████╗██╗██╗     ███████╗██████╗       ████████╗██╗   ██╗██╗",
        " ██╔════╝██╔══██╗██╔══██╗██╔═══██╗██╔════╝██║██║     ██╔════╝██╔══██╗      ╚══██╔══╝██║   ██║██║",
        " █████╗  ██████╔╝██████╔╝██║   ██║█████╗  ██║██║     █████╗  ██████╔╝█████╗   ██║   ██║   ██║██║",
        " ██╔══╝  ██╔═══╝ ██╔══██╗██║   ██║██╔══╝  ██║██║     ██╔══╝  ██╔══██╗╚════╝   ██║   ██║   ██║██║",
        " ███████╗██║     ██║  ██║╚██████╔╝██║     ██║███████╗███████╗██║  ██║         ██║   ╚██████╔╝██║",
        " ╚══════╝╚═╝     ╚═╝  ╚═╝ ╚═════╝ ╚═╝     ╚═╝╚══════╝╚══════╝╚═╝  ╚═╝         ╚═╝    ╚═════╝ ╚═╝",
    ];

    let art_h = art.len() as u16;
    let art_w = art.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;

    let subtitle = "OTLP Profile Flamegraph Viewer";
    let listening = format!("Listening on {listen_addr}");
    let waiting = "Waiting for profiles...";

    let total_h = art_h + 5;
    let v_off = area.height.saturating_sub(total_h) / 2;

    let flame_gradient: &[Color] = &[
        Color::Rgb(168, 50, 160),
        Color::Rgb(200, 40, 80),
        Color::Rgb(220, 50, 32),
        Color::Rgb(240, 100, 18),
        Color::Rgb(250, 170, 30),
        Color::Rgb(253, 224, 71),
    ];

    if area.width >= art_w + 2 && area.height >= total_h {
        let h_off = (area.width.saturating_sub(art_w)) / 2;
        for (i, line) in art.iter().enumerate() {
            let color = flame_gradient[i % flame_gradient.len()];
            buf.set_string(
                area.x + h_off,
                area.y + v_off + i as u16,
                line,
                Style::default().fg(color).bg(BG),
            );
        }
    } else {
        let name = "◆ eprofiler-tui";
        let hx = area.x + (area.width.saturating_sub(name.len() as u16)) / 2;
        buf.set_string(
            hx,
            area.y + v_off + 1,
            name,
            Style::default()
                .fg(Color::Rgb(250, 170, 30))
                .bg(BG)
                .add_modifier(Modifier::BOLD),
        );
    }

    let base_y = area.y + v_off + art_h + 1;

    center_str(
        buf,
        area,
        base_y,
        subtitle,
        Style::default()
            .fg(BRIGHT)
            .bg(BG)
            .add_modifier(Modifier::BOLD),
    );

    let sep_len = subtitle.len() as u16;
    let sep_x = area.x + (area.width.saturating_sub(sep_len)) / 2;
    buf.set_string(
        sep_x,
        base_y + 1,
        &"─".repeat(sep_len as usize),
        Style::default().fg(SEP_COLOR).bg(BG),
    );

    center_str(
        buf,
        area,
        base_y + 2,
        &listening,
        Style::default().fg(Color::Rgb(130, 130, 150)).bg(BG),
    );
    center_str(
        buf,
        area,
        base_y + 3,
        waiting,
        Style::default()
            .fg(DIM)
            .bg(BG)
            .add_modifier(Modifier::ITALIC),
    );
}

fn center_str(buf: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) {
    if y >= area.y + area.height {
        return;
    }
    let x = area.x + (area.width.saturating_sub(text.len() as u16)) / 2;
    buf.set_string(x, y, text, style);
}

fn render_header(state: &State, frame: &mut Frame, area: Rect) {
    let sep = " │ ".fg(Color::Rgb(55, 55, 65));

    let left_spans: Vec<Span> = vec![
        Span::styled(" ◆ ", Style::default().fg(ACCENT)),
        Span::styled(
            "eprofiler-tui",
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
        sep.clone(),
        Span::styled(
            state.listen_addr.clone(),
            Style::default().fg(Color::Rgb(130, 130, 150)),
        ),
        sep.clone(),
        format!("{} profiles", state.profiles_received).fg(Color::Rgb(110, 110, 130)),
        sep,
        format!("{} samples", format_count(state.samples_received)).fg(Color::Rgb(110, 110, 130)),
    ];
    frame.render_widget(Paragraph::new(Line::from(left_spans)), area);

    let buf = frame.buffer_mut();

    // Right side: [tabs] [status indicator]
    let (icon, label, status_color) = if state.frozen {
        ("⏸ ", "FROZEN", Color::Rgb(234, 179, 8))
    } else {
        ("▶ ", "LIVE", Color::Rgb(34, 197, 94))
    };
    let indicator = format!(" {icon}{label} ");
    let indicator_len = indicator.len() as u16;
    let ix = area.x + area.width.saturating_sub(indicator_len);
    buf.set_string(
        ix,
        area.y,
        &indicator,
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD),
    );

    let fg_active = state.active_tab == ActiveTab::Flamegraph;
    let fg_style = if fg_active {
        Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(DIM)
    };
    let exe_style = if !fg_active {
        Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(DIM)
    };

    let tab_sep = " │ ";
    let tab_str_fg = "Flamegraph";
    let tab_str_exe = "Executables";
    let tabs_width = tab_str_fg.len() + tab_sep.len() + tab_str_exe.len() + 2;
    let tabs_x = ix.saturating_sub(tabs_width as u16);

    buf.set_string(tabs_x, area.y, " ", Style::default());
    buf.set_string(tabs_x + 1, area.y, tab_str_fg, fg_style);
    buf.set_string(
        tabs_x + 1 + tab_str_fg.len() as u16,
        area.y,
        tab_sep,
        Style::default().fg(Color::Rgb(55, 55, 65)),
    );
    buf.set_string(
        tabs_x + 1 + tab_str_fg.len() as u16 + tab_sep.len() as u16,
        area.y,
        tab_str_exe,
        exe_style,
    );
}

fn render_exe_status_bar(state: &State, frame: &mut Frame, area: Rect) {
    let Some(ref status) = state.exe_status else {
        return;
    };

    let is_loading = status.starts_with("Loading") || status.starts_with("Removing");
    let is_error = status.starts_with("Error");

    let display = if is_loading {
        format!("{}{}", status, "...")
    } else {
        status.clone()
    };

    let color = if is_loading {
        Color::Rgb(234, 179, 8)
    } else if is_error {
        Color::Rgb(239, 68, 68)
    } else {
        Color::Rgb(34, 197, 94)
    };

    let spans = vec![
        " ".into(),
        Span::styled(display, Style::default().fg(color)),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_exe_table(state: &mut State, frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();

    if area.height < 2 {
        return;
    }

    let header_y = area.y;
    let col_id_w = 34u16.min(area.width / 3);
    let col_sym_w = 12u16;
    let col_name_w = area.width.saturating_sub(col_id_w + col_sym_w + 4);

    let hdr_style = Style::default().fg(DIM).add_modifier(Modifier::BOLD);
    buf.set_string(area.x + 1, header_y, "File ID", hdr_style);
    buf.set_string(area.x + 1 + col_id_w, header_y, "Name", hdr_style);
    buf.set_string(
        area.x + 1 + col_id_w + col_name_w,
        header_y,
        "Symbols",
        hdr_style,
    );

    let sep_y = header_y + 1;
    if sep_y >= area.y + area.height {
        return;
    }
    for x in area.x..area.x + area.width {
        if let Some(c) = buf.cell_mut((x, sep_y)) {
            c.set_char('─');
            c.set_style(Style::default().fg(SEP_COLOR));
        }
    }

    let visible_rows = (area.y + area.height).saturating_sub(sep_y + 1) as usize;
    if visible_rows == 0 {
        return;
    }

    // Keep cursor in view.
    if state.exe_cursor < state.exe_scroll {
        state.exe_scroll = state.exe_cursor;
    }
    if state.exe_cursor >= state.exe_scroll + visible_rows {
        state.exe_scroll = state.exe_cursor + 1 - visible_rows;
    }

    let cursor_bg = Color::Rgb(40, 45, 65);
    let na_fg = Color::Rgb(80, 80, 100);

    for vis_row in 0..visible_rows {
        let idx = state.exe_scroll + vis_row;
        if idx >= state.exe_list.len() {
            break;
        }
        let entry = &state.exe_list[idx];
        let y = sep_y + 1 + vis_row as u16;

        let is_cursor = idx == state.exe_cursor;
        let is_sym = entry.num_ranges.is_some();

        if is_cursor {
            for x in area.x..area.x + area.width {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.set_char(' ');
                    c.set_style(Style::default().bg(cursor_bg));
                }
            }
        }

        let row_bg = if is_cursor { cursor_bg } else { Color::Reset };

        let id_str = if let Some(file_id) = entry.file_id {
            let hex = file_id.format_hex();
            if hex.len() > col_id_w as usize - 1 {
                format!("{}…", &hex[..col_id_w as usize - 2])
            } else {
                hex
            }
        } else {
            "N/A".to_string()
        };
        let id_fg = if is_sym {
            Color::Rgb(100, 100, 120)
        } else {
            na_fg
        };
        buf.set_string(
            area.x + 1,
            y,
            &id_str,
            Style::default().fg(id_fg).bg(row_bg),
        );

        let prefix = if is_cursor { "▸ " } else { "  " };
        let max_name = (col_name_w as usize).saturating_sub(prefix.len() + 1);
        let name = truncate(&entry.name, max_name);
        let name_fg = if is_cursor {
            BRIGHT
        } else if is_sym {
            Color::Rgb(180, 180, 195)
        } else {
            Color::Rgb(120, 120, 140)
        };
        let name_style = if is_cursor {
            Style::default()
                .fg(name_fg)
                .bg(row_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(name_fg).bg(row_bg)
        };
        buf.set_string(
            area.x + 1 + col_id_w,
            y,
            &format!("{}{}", prefix, name),
            name_style,
        );

        let sym_str = if let Some(num) = entry.num_ranges {
            format_count(num as u64)
        } else {
            "N/A".to_string()
        };
        let sym_fg = if is_sym {
            Color::Rgb(34, 197, 94)
        } else {
            na_fg
        };
        buf.set_string(
            area.x + 1 + col_id_w + col_name_w,
            y,
            &sym_str,
            Style::default().fg(sym_fg).bg(row_bg),
        );
    }

    if state.exe_list.is_empty() && !state.exe_input_active {
        let msg = "No executables discovered. Waiting for profiles...";
        let y = sep_y + 2;
        if y < area.y + area.height {
            buf.set_string(
                area.x + 2,
                y,
                msg,
                Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
            );
        }
    }
}

fn render_exe_input_overlay(state: &State, frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();

    let popup_w = 60u16.min(area.width.saturating_sub(4));
    if popup_w < 10 {
        return;
    }

    let max_visible = 5usize;
    let match_count = state.exe_completions.len().min(max_visible);
    let popup_h = (match_count as u16 + 4).min(area.height.saturating_sub(2));
    if popup_h < 4 {
        return;
    }

    let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let popup_y = area.y + area.height.saturating_sub(popup_h);
    let popup = Rect::new(popup_x, popup_y, popup_w, popup_h);

    let border_fg = ACCENT;
    let input_fg = BRIGHT;
    let match_fg = Color::Rgb(180, 180, 195);
    let highlight_bg = Color::Rgb(40, 45, 65);
    let dim_fg = Color::Rgb(80, 80, 100);
    let dir_fg = Color::Rgb(96, 165, 250);

    // Clear popup area.
    for y in popup.y..popup.y + popup.height {
        for x in popup.x..popup.x + popup.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_char(' ');
                c.set_style(Style::reset());
            }
        }
    }

    // Top border.
    let top_y = popup.y;
    for x in popup.x..popup.x + popup.width {
        if let Some(c) = buf.cell_mut((x, top_y)) {
            let ch = if x == popup.x {
                '╭'
            } else if x == popup.x + popup.width - 1 {
                '╮'
            } else {
                '─'
            };
            c.set_char(ch);
            c.set_style(Style::reset().fg(border_fg));
        }
    }

    let title = " executable path ";
    let title_x = popup.x + 2;
    if title.len() + 3 <= popup.width as usize {
        buf.set_string(
            title_x,
            top_y,
            title,
            Style::reset().fg(BRIGHT).add_modifier(Modifier::BOLD),
        );
    }

    // Bottom border.
    let bot_y = popup.y + popup.height - 1;
    for x in popup.x..popup.x + popup.width {
        if let Some(c) = buf.cell_mut((x, bot_y)) {
            let ch = if x == popup.x {
                '╰'
            } else if x == popup.x + popup.width - 1 {
                '╯'
            } else {
                '─'
            };
            c.set_char(ch);
            c.set_style(Style::reset().fg(border_fg));
        }
    }

    // Side borders.
    for y in (popup.y + 1)..bot_y {
        if let Some(c) = buf.cell_mut((popup.x, y)) {
            c.set_char('│');
            c.set_style(Style::reset().fg(border_fg));
        }
        if let Some(c) = buf.cell_mut((popup.x + popup.width - 1, y)) {
            c.set_char('│');
            c.set_style(Style::reset().fg(border_fg));
        }
    }

    // Input line.
    let input_y = popup.y + 1;
    let inner_w = popup.width.saturating_sub(2) as usize;
    let prompt = format!(
        " / {}█",
        truncate(&state.exe_input, inner_w.saturating_sub(5))
    );
    buf.set_string(
        popup.x + 1,
        input_y,
        &truncate(&prompt, inner_w),
        Style::reset().fg(input_fg),
    );

    // Separator.
    let sep_y = popup.y + 2;
    for x in (popup.x + 1)..(popup.x + popup.width - 1) {
        if let Some(c) = buf.cell_mut((x, sep_y)) {
            c.set_char('─');
            c.set_style(Style::reset().fg(Color::Rgb(60, 60, 75)));
        }
    }

    // Completion list.
    let list_start_y = popup.y + 3;
    let visible = match_count.min((popup.height.saturating_sub(4)) as usize);

    let scroll_off = if state.exe_completion_cursor >= visible {
        state.exe_completion_cursor - visible + 1
    } else {
        0
    };

    if state.exe_completions.is_empty() {
        let msg = if state.exe_input.is_empty() {
            "type a path..."
        } else {
            "no matches"
        };
        buf.set_string(
            popup.x + 2,
            list_start_y,
            msg,
            Style::reset().fg(dim_fg).add_modifier(Modifier::ITALIC),
        );
    } else {
        for i in 0..visible {
            let match_idx = scroll_off + i;
            if match_idx >= state.exe_completions.len() {
                break;
            }
            let entry = &state.exe_completions[match_idx];
            let y = list_start_y + i as u16;
            let is_selected = match_idx == state.exe_completion_cursor;
            let is_dir = entry.ends_with('/');

            let row_fg = if is_selected {
                BRIGHT
            } else if is_dir {
                dir_fg
            } else {
                match_fg
            };

            if is_selected {
                for x in (popup.x + 1)..(popup.x + popup.width - 1) {
                    if let Some(c) = buf.cell_mut((x, y)) {
                        c.set_char(' ');
                        c.set_style(Style::reset().bg(highlight_bg));
                    }
                }
            }

            let prefix = if is_selected { " ▸ " } else { "   " };
            let display = format!("{}{}", prefix, truncate(entry, inner_w.saturating_sub(3)));

            let style = if is_selected {
                Style::reset()
                    .fg(row_fg)
                    .bg(highlight_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::reset().fg(row_fg)
            };

            buf.set_string(popup.x + 1, y, &display, style);
        }
    }
}

fn render_exe_footer(state: &State, frame: &mut Frame, area: Rect) {
    let key = Color::Rgb(80, 80, 100);
    let desc = Color::Rgb(55, 55, 65);

    let spans: Vec<Span> = if state.exe_input_active {
        vec![
            " [Esc]".fg(key),
            " cancel ".fg(desc),
            "[Tab]".fg(key),
            " complete ".fg(desc),
            "[↑↓]".fg(key),
            " navigate ".fg(desc),
            "[Enter]".fg(key),
            " load ".fg(desc),
        ]
    } else {
        vec![
            " [Tab]".fg(key),
            " switch ".fg(desc),
            "[j/k]".fg(key),
            " navigate ".fg(desc),
            "[Enter]".fg(key),
            " symbolize ".fg(desc),
            "[r]".fg(key),
            " remove ".fg(desc),
            "[/]".fg(key),
            " add new ".fg(desc),
            "[q]".fg(key),
            " quit ".fg(desc),
        ]
    };
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        area,
    );
}

fn render_detail_bar(state: &State, frame: &mut Frame, area: Rect) {
    if state.selected_name.is_empty() && state.zoom_path.is_empty() {
        return;
    }

    let sep = " │ ".fg(Color::Rgb(55, 55, 65));
    let root_total = {
        let zr = get_zoom_node(&state.flamegraph.root, &state.zoom_path);
        zr.total_value
    };

    let mut spans: Vec<Span> = Vec::new();

    if !state.zoom_path.is_empty() {
        spans.push(
            format!(
                " zoomed: {} ",
                state.zoom_path.last().unwrap_or(&String::new())
            )
            .fg(ACCENT)
            .bold(),
        );
        if !state.selected_name.is_empty() {
            spans.push(sep.clone());
        }
    }

    if !state.selected_name.is_empty() {
        let self_pct = if root_total > 0 {
            state.selected_self as f64 / root_total as f64 * 100.0
        } else {
            0.0
        };
        let total_pct = if root_total > 0 {
            state.selected_total as f64 / root_total as f64 * 100.0
        } else {
            0.0
        };

        spans.push(" ▸ ".fg(ACCENT).bold());
        spans.push(Span::styled(
            truncate(&state.selected_name, 40),
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ));
        spans.push(sep.clone());
        spans.push("self: ".fg(DIM));
        spans.push(
            format!(
                "{} ({:.1}%)",
                format_count(state.selected_self as u64),
                self_pct
            )
            .fg(Color::Rgb(249, 115, 22)),
        );
        spans.push(sep.clone());
        spans.push("total: ".fg(DIM));
        spans.push(
            format!(
                "{} ({:.1}%)",
                format_count(state.selected_total as u64),
                total_pct
            )
            .fg(Color::Rgb(234, 179, 8)),
        );
        spans.push(sep);
        spans.push("depth: ".fg(DIM));
        spans.push(
            state
                .selected_depth
                .to_string()
                .fg(Color::Rgb(130, 130, 150)),
        );
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_flamegraph(state: &mut State, frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();

    if area.width < 4 || area.height < 2 {
        return;
    }

    let zoom_root = get_zoom_node(&state.flamegraph.root, &state.zoom_path);
    if zoom_root.total_value <= 0 {
        render_empty_fg(buf, area);
        return;
    }

    let forced_palette = state
        .zoom_path
        .first()
        .map(|thread_name| thread_rank(&state.flamegraph.root, thread_name));

    let frames = layout_frames(zoom_root, area.width, forced_palette);
    let max_depth = frames.iter().map(|f| f.depth).max().unwrap_or(0);
    let viewport_height = area.height as usize;
    let root_total = zoom_root.total_value;

    let cursor_depth = state.cursor_path.len();
    if viewport_height > 0 {
        if cursor_depth < state.scroll_y {
            state.scroll_y = cursor_depth;
        }
        if cursor_depth >= state.scroll_y + viewport_height {
            state.scroll_y = cursor_depth - viewport_height + 1;
        }
    }
    let max_scroll = max_depth.saturating_sub(viewport_height.saturating_sub(1));
    if state.scroll_y > max_scroll {
        state.scroll_y = max_scroll;
    }

    let cursor_rect = cursor_frame_rect(zoom_root, &state.cursor_path, area.width, forced_palette);

    if let Some(ref cr) = cursor_rect {
        state.selected_name = cr.name.clone();
        state.selected_self = cr.self_value;
        state.selected_total = cr.total_value;
        state.selected_pct = if root_total > 0 {
            cr.total_value as f64 / root_total as f64 * 100.0
        } else {
            0.0
        };
        state.selected_depth = cr.depth;
    }

    for fr in &frames {
        if fr.depth < state.scroll_y {
            continue;
        }
        let vis_depth = fr.depth - state.scroll_y;
        if vis_depth >= viewport_height {
            continue;
        }

        let screen_y = area.y + vis_depth as u16;
        if screen_y >= area.y + area.height {
            continue;
        }

        let is_cursor = cursor_rect
            .as_ref()
            .is_some_and(|cr| cr.depth == fr.depth && cr.x == fr.x);

        let heat = if fr.total_value > 0 {
            fr.self_value as f64 / fr.total_value as f64
        } else {
            0.0
        };

        let bg = if is_cursor {
            lighten(flame_color(&fr.name, heat, fr.palette_index), 45)
        } else {
            flame_color(&fr.name, heat, fr.palette_index)
        };
        let fg = contrast_fg(bg);

        let x_start = area.x + fr.x;
        let x_end = (area.x + fr.x + fr.width).min(area.x + area.width);
        let border_color = darken(bg, 55);

        for x in x_start..x_end {
            if let Some(cell) = buf.cell_mut((x, screen_y)) {
                if x == x_start || x == x_end.saturating_sub(1) {
                    cell.set_char('▏');
                    cell.set_style(Style::default().fg(border_color).bg(bg));
                } else {
                    cell.set_char(' ');
                    cell.set_style(Style::default().bg(bg));
                }
            }
        }

        let inner_width = fr.width.saturating_sub(2);
        if inner_width >= 3 {
            let max_chars = inner_width as usize;
            let name = truncate(&fr.name, max_chars);
            let pad = (inner_width as usize).saturating_sub(name.len()) / 2;
            let name_x = area.x + fr.x + 1 + pad as u16;

            let style = if is_cursor {
                Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg).bg(bg)
            };
            buf.set_string(name_x, screen_y, &name, style);
        }

        if fr.width >= 14 && root_total > 0 {
            let pct = fr.total_value as f64 / root_total as f64 * 100.0;
            if pct >= 0.1 {
                let pct_str = format!("{:.1}%", pct);
                let pct_x = area.x + fr.x + fr.width - pct_str.len() as u16 - 2;
                if pct_x > area.x + fr.x + 2 {
                    let dim_fg = blend(fg, bg, 0.45);
                    buf.set_string(
                        pct_x,
                        screen_y,
                        &pct_str,
                        Style::default().fg(dim_fg).bg(bg),
                    );
                }
            }
        }

        if is_cursor && fr.width >= 3 {
            if let Some(cell) = buf.cell_mut((area.x + fr.x + 1, screen_y)) {
                cell.set_char('▸');
                cell.set_style(
                    Style::default()
                        .fg(Color::White)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
    }

    for vis_d in 0..viewport_height {
        let screen_y = area.y + vis_d as u16;
        if screen_y >= area.y + area.height {
            break;
        }
        let depth = state.scroll_y + vis_d;
        let has_frame = frames.iter().any(|f| f.depth == depth);
        if !has_frame && depth <= max_depth {
            for x in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, screen_y)) {
                    cell.set_char('·');
                    cell.set_style(Style::default().fg(Color::Rgb(30, 30, 38)));
                }
            }
        }
    }
}

fn render_empty_fg(buf: &mut Buffer, area: Rect) {
    let msg = "No profile data yet";
    let y = area.y + area.height / 2;
    let x = area.x + (area.width.saturating_sub(msg.len() as u16)) / 2;
    buf.set_string(
        x,
        y,
        msg,
        Style::default()
            .fg(Color::Rgb(90, 90, 110))
            .add_modifier(Modifier::ITALIC),
    );
}

fn render_flamegraph_footer(state: &State, frame: &mut Frame, area: Rect) {
    let key = Color::Rgb(80, 80, 100);
    let desc = Color::Rgb(55, 55, 65);

    if state.search_active {
        let spans: Vec<Span> = vec![
            " [Esc]".fg(key),
            " cancel ".fg(desc),
            "[Enter]".fg(key),
            " select ".fg(desc),
            "[↑↓]".fg(key),
            " navigate ".fg(desc),
        ];
        frame.render_widget(
            Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
            area,
        );
    } else {
        let spans: Vec<Span> = vec![
            " [Tab]".fg(key),
            " switch ".fg(desc),
            "[q]".fg(key),
            " quit ".fg(desc),
            "[f/Space]".fg(key),
            " freeze ".fg(desc),
            "[j/↓ k/↑]".fg(key),
            " depth ".fg(desc),
            "[h/← l/→]".fg(key),
            " frame ".fg(desc),
            "[Enter]".fg(key),
            " zoom ".fg(desc),
            "[Esc]".fg(key),
            " back ".fg(desc),
            "[/]".fg(key),
            " search ".fg(desc),
            "[r]".fg(key),
            " reset ".fg(desc),
        ];
        frame.render_widget(
            Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
            area,
        );
    }
}

fn render_search_overlay(state: &State, frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();

    let popup_w = 50u16.min(area.width.saturating_sub(4));
    if popup_w < 10 {
        return;
    }

    let max_visible = 3usize;
    let match_count = state.search_matches.len().min(max_visible);
    let popup_h = (match_count as u16 + 4).min(area.height.saturating_sub(2));
    if popup_h < 4 {
        return;
    }

    let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let popup_y = area.y + area.height.saturating_sub(popup_h);

    let popup = Rect::new(popup_x, popup_y, popup_w, popup_h);

    let border_fg = Color::Rgb(245, 166, 35);
    let input_fg = BRIGHT;
    let match_fg = Color::Rgb(180, 180, 195);
    let highlight_bg = Color::Rgb(40, 45, 65);
    let dim_fg = Color::Rgb(80, 80, 100);

    for y in popup.y..popup.y + popup.height {
        for x in popup.x..popup.x + popup.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_char(' ');
                c.set_style(Style::reset());
            }
        }
    }

    let top_y = popup.y;
    for x in popup.x..popup.x + popup.width {
        if let Some(c) = buf.cell_mut((x, top_y)) {
            let ch = if x == popup.x {
                '╭'
            } else if x == popup.x + popup.width - 1 {
                '╮'
            } else {
                '─'
            };
            c.set_char(ch);
            c.set_style(Style::reset().fg(border_fg));
        }
    }

    let title = " thread.name ";
    let title_x = popup.x + 2;
    if title.len() + 3 <= popup.width as usize {
        buf.set_string(
            title_x,
            top_y,
            title,
            Style::reset().fg(BRIGHT).add_modifier(Modifier::BOLD),
        );
    }

    let bot_y = popup.y + popup.height - 1;
    for x in popup.x..popup.x + popup.width {
        if let Some(c) = buf.cell_mut((x, bot_y)) {
            let ch = if x == popup.x {
                '╰'
            } else if x == popup.x + popup.width - 1 {
                '╯'
            } else {
                '─'
            };
            c.set_char(ch);
            c.set_style(Style::reset().fg(border_fg));
        }
    }

    for y in (popup.y + 1)..bot_y {
        if let Some(c) = buf.cell_mut((popup.x, y)) {
            c.set_char('│');
            c.set_style(Style::reset().fg(border_fg));
        }
        if let Some(c) = buf.cell_mut((popup.x + popup.width - 1, y)) {
            c.set_char('│');
            c.set_style(Style::reset().fg(border_fg));
        }
    }

    let input_y = popup.y + 1;
    let inner_w = popup.width.saturating_sub(2) as usize;
    let prompt = format!(
        " / {}█",
        truncate(&state.search_input, inner_w.saturating_sub(5))
    );
    buf.set_string(
        popup.x + 1,
        input_y,
        &truncate(&prompt, inner_w),
        Style::reset().fg(input_fg),
    );

    let sep_y = popup.y + 2;
    for x in (popup.x + 1)..(popup.x + popup.width - 1) {
        if let Some(c) = buf.cell_mut((x, sep_y)) {
            c.set_char('─');
            c.set_style(Style::reset().fg(Color::Rgb(60, 60, 75)));
        }
    }

    let list_start_y = popup.y + 3;
    let visible = match_count.min((popup.height.saturating_sub(4)) as usize);

    let scroll_off = if state.search_cursor >= visible {
        state.search_cursor - visible + 1
    } else {
        0
    };

    if state.search_matches.is_empty() {
        let msg = if state.search_input.is_empty() {
            "type to filter threads..."
        } else {
            "no matches"
        };
        buf.set_string(
            popup.x + 2,
            list_start_y,
            msg,
            Style::reset().fg(dim_fg).add_modifier(Modifier::ITALIC),
        );
    } else {
        for i in 0..visible {
            let match_idx = scroll_off + i;
            if match_idx >= state.search_matches.len() {
                break;
            }
            let (ref name, _) = state.search_matches[match_idx];
            let y = list_start_y + i as u16;
            let is_selected = match_idx == state.search_cursor;

            let row_fg = if is_selected { BRIGHT } else { match_fg };

            if is_selected {
                for x in (popup.x + 1)..(popup.x + popup.width - 1) {
                    if let Some(c) = buf.cell_mut((x, y)) {
                        c.set_char(' ');
                        c.set_style(Style::reset().bg(highlight_bg));
                    }
                }
            }

            let prefix = if is_selected { " ▸ " } else { "   " };
            let display = format!("{}{}", prefix, truncate(name, inner_w.saturating_sub(3)));

            let style = if is_selected {
                Style::reset()
                    .fg(row_fg)
                    .bg(highlight_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::reset().fg(row_fg)
            };

            buf.set_string(popup.x + 1, y, &display, style);
        }
    }
}

const PALETTES: &[&[(f64, (u8, u8, u8))]] = &[
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

fn flame_color(name: &str, heat: f64, palette_index: usize) -> Color {
    let hash = name.bytes().fold(0u64, |h, b| {
        h.wrapping_mul(2654435761).wrapping_add(b as u64)
    });

    let h = heat.clamp(0.0, 1.0);

    let stops = PALETTES[palette_index % PALETTES.len()];
    let (r, g, b) = gradient(h, stops);

    let rv = ((hash % 18) as i16 - 9).clamp(-12, 12);
    let gv = (((hash >> 5) % 14) as i16 - 7).clamp(-10, 10);

    Color::Rgb(
        (r as i16 + rv).clamp(25, 255) as u8,
        (g as i16 + gv).clamp(20, 255) as u8,
        b,
    )
}

fn gradient(t: f64, stops: &[(f64, (u8, u8, u8))]) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    for i in 0..stops.len() - 1 {
        let (t0, c0) = stops[i];
        let (t1, c1) = stops[i + 1];
        if t <= t1 {
            let s = if (t1 - t0).abs() < f64::EPSILON {
                0.0
            } else {
                (t - t0) / (t1 - t0)
            };
            return (
                lerp_u8(c0.0, c1.0, s),
                lerp_u8(c0.1, c1.1, s),
                lerp_u8(c0.2, c1.2, s),
            );
        }
    }
    stops.last().unwrap().1
}

fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    ((1.0 - t) * a as f64 + t * b as f64).round() as u8
}

fn contrast_fg(bg: Color) -> Color {
    match bg {
        Color::Rgb(r, g, b) => {
            let lum = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
            if lum > 160.0 {
                Color::Rgb(20, 18, 15)
            } else {
                Color::Rgb(250, 248, 245)
            }
        }
        _ => Color::White,
    }
}

fn lighten(c: Color, amount: u8) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            r.saturating_add(amount),
            g.saturating_add(amount),
            b.saturating_add(amount),
        ),
        _ => c,
    }
}

fn darken(c: Color, amount: u8) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            r.saturating_sub(amount),
            g.saturating_sub(amount),
            b.saturating_sub(amount),
        ),
        _ => c,
    }
}

fn blend(c1: Color, c2: Color, t: f64) -> Color {
    match (c1, c2) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            Color::Rgb(lerp_u8(r1, r2, t), lerp_u8(g1, g2, t), lerp_u8(b1, b2, t))
        }
        _ => c1,
    }
}

fn fill(buf: &mut Buffer, r: Rect, color: Color) {
    let style = Style::reset().bg(color);
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_char(' ');
                c.set_style(style);
            }
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max <= 1 {
        s.chars().take(max).collect()
    } else {
        s.chars()
            .take(max - 1)
            .chain(std::iter::once('…'))
            .collect()
    }
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::Paragraph,
};

use eprofiler_proto::opentelemetry::proto::collector::profiles::v1development::ExportProfilesServiceRequest;
use eprofiler_proto::opentelemetry::proto::common::v1 as common;
use eprofiler_proto::opentelemetry::proto::profiles::v1development as profiles;

use super::DebugState;

use crate::tui::canvas::BufferExt;
use crate::tui::text::{format_duration, format_hex, format_timestamp, truncate};
use crate::tui::theme::{
    ACCENT, ACCENT_LIGHT, ACCENT_PALE, AMBER, BRIGHT, CYAN, DIM, ERROR, FAINT, LIME,
    MATCH_ACTIVE_BG, MATCH_BG, MUTED, MUTED_DARK, ORANGE, POPUP_BORDER, PURPLE, RULE, SUCCESS,
    TEXT, YELLOW,
};
use crate::tui::view::KeyHints;

const KEYS: &[(&str, &str)] = &[
    ("[h/←]", " prev "),
    ("[l/→]", " next "),
    ("[j/k]", " scroll "),
    ("[d/u]", " page "),
    ("[/]", " search "),
    ("[g/G]", " first/last "),
    ("[q]", " quit "),
];

const SEARCH_KEYS: &[(&str, &str)] = &[("[Esc]", " cancel "), ("[Enter]", " confirm ")];

fn dim(s: &str) -> Span<'static> {
    s.to_owned().fg(DIM)
}

fn fmt_any_val(v: Option<&common::AnyValue>) -> String {
    match v.and_then(|v| v.value.as_ref()) {
        Some(common::any_value::Value::StringValue(s)) => format!("\"{s}\""),
        Some(common::any_value::Value::BoolValue(b)) => b.to_string(),
        Some(common::any_value::Value::IntValue(i)) => i.to_string(),
        Some(common::any_value::Value::DoubleValue(d)) => d.to_string(),
        Some(common::any_value::Value::BytesValue(b)) => format!("<{} bytes>", b.len()),
        Some(common::any_value::Value::ArrayValue(a)) => format!("[{} items]", a.values.len()),
        Some(common::any_value::Value::KvlistValue(kv)) => format!("{{{} pairs}}", kv.values.len()),
        Some(common::any_value::Value::StringValueStrindex(i)) => format!("strindex({i})"),
        None => String::new(),
    }
}

struct Dict<'a>(&'a profiles::ProfilesDictionary);

impl Dict<'_> {
    fn str(&self, idx: i32) -> String {
        self.0
            .string_table
            .get(idx as usize)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_default()
    }

    fn get_attr(&self, idx: i32) -> Option<&profiles::KeyValueAndUnit> {
        (idx > 0)
            .then(|| self.0.attribute_table.get(idx as usize))
            .flatten()
    }

    fn frame_info(&self, loc: &profiles::Location) -> (String, Color) {
        let raw = loc
            .attribute_indices
            .iter()
            .filter_map(|&ai| {
                let attr = self.0.attribute_table.get(ai as usize).filter(|_| ai > 0)?;
                let key = self.0.string_table.get(attr.key_strindex as usize)?;
                (key == "profile.frame.type").then_some(attr)
            })
            .find_map(|attr| match attr.value.as_ref()?.value.as_ref()? {
                common::any_value::Value::StringValue(s) => Some(s.clone()),
                _ => None,
            });

        match raw.as_deref() {
            Some("native") => ("Native".into(), SUCCESS),
            Some("kernel") => ("Kernel".into(), ERROR),
            Some("jvm") => ("JVM".into(), ORANGE),
            Some("cpython") => ("Python".into(), YELLOW),
            Some("php" | "phpjit") => ("PHP".into(), PURPLE),
            Some("ruby") => ("Ruby".into(), ERROR),
            Some("perl") => ("Perl".into(), ACCENT_LIGHT),
            Some("v8js") => ("JS".into(), YELLOW),
            Some("dotnet") => (".NET".into(), ACCENT_LIGHT),
            Some("beam") => ("Beam".into(), PURPLE),
            Some("go") => ("Go".into(), CYAN),
            Some(other) => (other.to_string(), MUTED_DARK),
            None => ("Unknown".into(), MUTED_DARK),
        }
    }

    fn mapping_name(&self, loc: &profiles::Location) -> String {
        self.0
            .mapping_table
            .get(loc.mapping_index as usize)
            .filter(|_| loc.mapping_index > 0)
            .map(|m| self.str(m.filename_strindex))
            .filter(|n| !n.is_empty())
            .map(|n| n.rsplit('/').next().unwrap_or(&n).to_string())
            .unwrap_or_else(|| "[unknown]".into())
    }

    fn func_name(&self, line: &profiles::Line) -> String {
        self.0
            .function_table
            .get(line.function_index as usize)
            .filter(|_| line.function_index > 0)
            .map(|f| self.str(f.name_strindex))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "[unknown]".into())
    }

    fn value_type(&self, vt: &profiles::ValueType) -> String {
        format!(
            "{} / {}",
            self.str(vt.type_strindex),
            self.str(vt.unit_strindex)
        )
    }

    fn any_val(&self, v: Option<&common::AnyValue>) -> String {
        match v.and_then(|v| v.value.as_ref()) {
            Some(common::any_value::Value::StringValueStrindex(i)) => {
                let s = self.str(*i);
                if s.is_empty() {
                    format!("strindex({i})")
                } else {
                    format!("\"{s}\"")
                }
            }
            _ => fmt_any_val(v),
        }
    }
}

struct Doc(Vec<Line<'static>>);

impl From<Doc> for Vec<Line<'static>> {
    fn from(doc: Doc) -> Self {
        doc.0
    }
}

impl Doc {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn from_request(req: &ExportProfilesServiceRequest) -> Self {
        let mut doc = Self::new();
        let dict = req.dictionary.as_ref().map(Dict);
        if let Some(ref d) = dict {
            doc.dictionary(d);
        }
        for (i, rp) in req.resource_profiles.iter().enumerate() {
            doc.resource(rp, i, req.resource_profiles.len(), dict.as_ref());
        }
        if doc.is_empty() {
            doc.0
                .push(Line::from("  <empty request>".to_owned().fg(DIM).italic()));
        }
        doc
    }

    fn section(&mut self, title: &str) {
        self.0.push(Line::from(vec![
            format!("  ── {title} ").fg(ACCENT_LIGHT).bold(),
            "─".repeat(60usize.saturating_sub(title.len() + 5)).fg(RULE),
        ]));
    }

    fn subsection(&mut self, title: &str) {
        self.0
            .push(Line::from(format!("  ╌╌ {title}").fg(ACCENT_PALE)));
    }

    fn table(&mut self, title: &str, non_empty: bool, f: impl FnOnce(&mut Self)) {
        if non_empty {
            self.subsection(title);
            f(self);
            self.blank();
        }
    }

    fn blank(&mut self) {
        self.0.push(Line::default());
    }
    fn row(&mut self, spans: Vec<Span<'static>>) {
        self.0.push(Line::from(spans));
    }

    fn kv(&mut self, label: &str, value: &str) {
        self.row(vec![dim(label), value.to_owned().fg(BRIGHT)]);
    }

    fn kv_ne(&mut self, label: &str, value: &str) {
        if !value.is_empty() {
            self.kv(label, value);
        }
    }

    fn attr(&mut self, idx: i32, d: &Dict, prefix: Span<'static>) {
        let Some(a) = d.get_attr(idx) else { return };
        self.row(vec![
            prefix,
            d.str(a.key_strindex).fg(YELLOW),
            dim(" = "),
            d.any_val(a.value.as_ref()).fg(LIME),
        ]);
    }

    fn dictionary(&mut self, d: &Dict) {
        let p = d.0;
        self.section("Dictionary");
        self.row(vec![
            dim("  strings: "),
            p.string_table.len().to_string().fg(BRIGHT),
            dim("  locations: "),
            p.location_table.len().to_string().fg(BRIGHT),
            dim("  functions: "),
            p.function_table.len().to_string().fg(BRIGHT),
        ]);
        self.row(vec![
            dim("  mappings: "),
            p.mapping_table.len().to_string().fg(BRIGHT),
            dim("  stacks: "),
            p.stack_table.len().to_string().fg(BRIGHT),
            dim("  attributes: "),
            p.attribute_table.len().to_string().fg(BRIGHT),
            dim("  links: "),
            p.link_table.len().to_string().fg(BRIGHT),
        ]);
        self.blank();

        self.table("String Table", p.string_table.len() > 1, |doc| {
            let mut shown = 0usize;
            for (i, s) in p.string_table.iter().enumerate() {
                if s.is_empty() {
                    continue;
                }
                doc.row(vec![dim(&format!("  [{i:>4}] ")), s.clone().fg(TEXT)]);
                shown += 1;
                if shown >= 200 {
                    doc.row(vec![dim(&format!(
                        "  … truncated ({} total)",
                        p.string_table.len()
                    ))]);
                    break;
                }
            }
        });

        self.table("Mapping Table", p.mapping_table.len() > 1, |doc| {
            for (i, m) in p.mapping_table.iter().enumerate().skip(1) {
                doc.row(vec![
                    dim(&format!("  [{i}] ")),
                    d.str(m.filename_strindex).fg(ACCENT_PALE),
                ]);
                doc.row(vec![
                    dim("      mem: "),
                    format!("0x{:x}", m.memory_start).fg(AMBER),
                    dim(".."),
                    format!("0x{:x}", m.memory_limit).fg(AMBER),
                    dim("  offset: "),
                    format!("0x{:x}", m.file_offset).fg(AMBER),
                ]);
                for &ai in &m.attribute_indices {
                    doc.attr(ai, d, dim("      "));
                }
            }
        });

        self.table("Attribute Table", p.attribute_table.len() > 1, |doc| {
            for (i, attr) in p.attribute_table.iter().enumerate().skip(1) {
                doc.row(vec![
                    dim(&format!("  [{i:>3}] ")),
                    d.str(attr.key_strindex).fg(YELLOW),
                    dim(" = "),
                    d.any_val(attr.value.as_ref()).fg(LIME),
                ]);
            }
        });

        self.table("Function Table", p.function_table.len() > 1, |doc| {
            for (i, f) in p.function_table.iter().enumerate().skip(1) {
                let mut spans = vec![
                    dim(&format!("  [{i:>3}] ")),
                    d.str(f.name_strindex).fg(BRIGHT),
                ];
                let sys = d.str(f.system_name_strindex);
                if !sys.is_empty() {
                    spans.extend([dim("  sys="), sys.fg(TEXT)]);
                }
                let file = d.str(f.filename_strindex);
                if !file.is_empty() {
                    spans.extend([dim("  file="), file.fg(MUTED)]);
                }
                if f.start_line > 0 {
                    spans.push(dim(&format!(":{}", f.start_line)));
                }
                doc.row(spans);
            }
        });
    }

    fn resource(
        &mut self,
        rp: &profiles::ResourceProfiles,
        idx: usize,
        total: usize,
        dict: Option<&Dict>,
    ) {
        self.section(&format!("Resource {}/{total}", idx + 1));
        if let Some(res) = &rp.resource {
            for kv in &res.attributes {
                self.row(vec![
                    dim("  "),
                    kv.key.clone().fg(YELLOW),
                    dim(" = "),
                    fmt_any_val(kv.value.as_ref()).fg(LIME),
                ]);
            }
        }
        self.kv_ne("  schema_url: ", &rp.schema_url);
        self.blank();

        for sp in &rp.scope_profiles {
            if let Some(scope) = &sp.scope {
                self.subsection("Scope");
                self.kv_ne("  name: ", &scope.name);
                self.kv_ne("  version: ", &scope.version);
                self.blank();
            }
            for (i, p) in sp.profiles.iter().enumerate() {
                self.profile(p, i, sp.profiles.len(), dict);
            }
        }
    }

    fn profile(&mut self, p: &profiles::Profile, idx: usize, total: usize, dict: Option<&Dict>) {
        self.section(&format!("Profile {}/{total}", idx + 1));
        if !p.profile_id.is_empty() {
            self.row(vec![dim("  id: "), format_hex(&p.profile_id).fg(PURPLE)]);
        }
        if p.time_unix_nano > 0 {
            self.kv("  time: ", &format_timestamp(p.time_unix_nano));
        }
        if p.duration_nano > 0 {
            self.kv("  duration: ", &format_duration(p.duration_nano));
        }
        if let Some(d) = dict {
            for (label, vt) in [
                ("sample_type", &p.sample_type),
                ("period_type", &p.period_type),
            ] {
                if let Some(vt) = vt {
                    self.kv(&format!("  {label}: "), &d.value_type(vt));
                }
            }
        }
        if p.period != 0 {
            self.kv("  period: ", &p.period.to_string());
        }
        self.kv("  samples: ", &p.samples.len().to_string());
        if p.dropped_attributes_count > 0 {
            self.row(vec![
                dim("  dropped_attributes: "),
                p.dropped_attributes_count.to_string().fg(ERROR),
            ]);
        }
        if !p.original_payload_format.is_empty() {
            self.kv("  original_format: ", &p.original_payload_format);
            self.kv(
                "  original_payload: ",
                &format!("{} bytes", p.original_payload.len()),
            );
        }
        if let Some(d) = dict {
            p.attribute_indices
                .iter()
                .for_each(|&ai| self.attr(ai, d, dim("  ")));
        }
        self.blank();
        if !p.samples.is_empty() {
            self.subsection("Samples");
            self.samples(&p.samples, dict);
        }
    }

    fn samples(&mut self, samples: &[profiles::Sample], dict: Option<&Dict>) {
        for (sample_idx, sample) in samples.iter().enumerate() {
            let mut hdr = vec![
                format!("  ┌ #{sample_idx}").fg(ACCENT),
                dim(&format!("  stack[{}]", sample.stack_index)),
            ];
            if !sample.values.is_empty() {
                hdr.extend([dim("  values="), format!("{:?}", sample.values).fg(ORANGE)]);
            }
            if !sample.timestamps_unix_nano.is_empty() {
                hdr.push(dim(&format!(
                    "  ts_count={}",
                    sample.timestamps_unix_nano.len()
                )));
            }
            if sample.link_index != 0 {
                hdr.push(dim(&format!("  link[{}]", sample.link_index)));
            }
            self.row(hdr);

            let Some(d) = dict else {
                self.blank();
                continue;
            };
            for &ai in &sample.attribute_indices {
                self.attr(ai, d, "  │  ".to_owned().fg(ACCENT));
            }

            let stack_idx = sample.stack_index as usize;
            if stack_idx > 0 && stack_idx < d.0.stack_table.len() {
                for (fi, &loc_idx) in d.0.stack_table[stack_idx]
                    .location_indices
                    .iter()
                    .enumerate()
                    .rev()
                {
                    let is_leaf = fi == 0;
                    let conn = if is_leaf { "  └  " } else { "  │  " };
                    let Some(loc) =
                        d.0.location_table
                            .get(loc_idx as usize)
                            .filter(|_| loc_idx > 0)
                    else {
                        self.row(vec![conn.to_owned().fg(ACCENT), dim("<invalid>")]);
                        continue;
                    };

                    let (ft, ftc) = d.frame_info(loc);
                    let tag = format!(" [{ft}]").fg(ftc);

                    if loc.lines.is_empty() {
                        self.row(vec![
                            conn.to_owned().fg(ACCENT),
                            format!("{}+0x{:x}", d.mapping_name(loc), loc.address).fg(AMBER),
                            tag,
                        ]);
                    } else {
                        for (li, info) in loc.lines.iter().enumerate() {
                            let pfx = match li {
                                0 => conn,
                                _ if is_leaf => "     ",
                                _ => "  │  ",
                            };
                            let mut spans = vec![
                                pfx.to_owned().fg(ACCENT),
                                d.func_name(info).fg(BRIGHT),
                                tag.clone(),
                            ];
                            if li > 0 {
                                spans.push(" [inline]".to_owned().fg(PURPLE));
                            }
                            if info.line > 0 {
                                spans.push(dim(&format!(" :{}", info.line)));
                            }
                            self.row(spans);
                        }
                    }
                }
            }
            self.blank();
        }
    }
}

impl DebugState {
    pub(super) fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if self.requests.is_empty() {
            self.render_waiting(frame, area);
            return;
        }
        let chunks = Layout::new(
            Direction::Vertical,
            [
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ],
        )
        .split(area);

        self.render_header(frame, chunks[0]);
        self.render_body(frame, chunks[1]);
        self.render_footer(frame, chunks[2]);
        if self.search.active {
            self.render_search_overlay(frame, chunks[1]);
        }
    }

    pub(super) fn recompute_hits(&mut self) {
        self.search.hit_cursor = 0;
        self.search.hits = self
            .requests
            .get(self.current)
            .filter(|_| !self.search.pattern.is_empty())
            .map(|req| {
                let pat = self.search.pattern.to_lowercase();
                let lines: Vec<Line> = Doc::from_request(req).into();
                lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| {
                        line.spans
                            .iter()
                            .map(|s| s.content.as_ref())
                            .collect::<String>()
                            .to_lowercase()
                            .contains(&pat)
                    })
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default();
    }

    fn render_waiting(&self, frame: &mut Frame, area: Rect) {
        let buf = frame.buffer_mut();
        let text = Style::default();
        let cy = area.y + area.height / 2;
        buf.center(
            area,
            cy.saturating_sub(2),
            "◆ eprofiler-tui debug",
            text.fg(ACCENT).add_modifier(Modifier::BOLD),
        );
        buf.center(
            area,
            cy,
            &format!("Listening on {}", self.listen_addr),
            text.fg(BRIGHT),
        );
        buf.center(
            area,
            cy + 1,
            "Waiting for profiles...",
            text.fg(DIM).add_modifier(Modifier::ITALIC),
        );
        buf.center(
            area,
            cy + 3,
            "Send OTLP profiles to inspect them",
            text.fg(MUTED_DARK),
        );
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let (cur, total) = (self.current + 1, self.requests.len());
        let sep = " │ ".fg(FAINT);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                " ◆ ".fg(ACCENT),
                "debug".fg(BRIGHT).bold(),
                sep.clone(),
                self.listen_addr.clone().fg(MUTED),
                sep.clone(),
                format!("Request {cur} of {total}").fg(BRIGHT).bold(),
                sep,
                format!("{total} queued").fg(MUTED_DARK),
            ])),
            area,
        );
    }

    fn render_body(&mut self, frame: &mut Frame, area: Rect) {
        let Some(req) = self.requests.get(self.current) else {
            return;
        };
        let lines: Vec<Line> = Doc::from_request(req).into();
        self.scroll_y = self
            .scroll_y
            .min(lines.len().saturating_sub(area.height as usize));

        let active_hit = self.search.hits.get(self.search.hit_cursor).copied();
        let has_pattern = !self.search.pattern.is_empty();

        let visible: Vec<Line> = lines
            .into_iter()
            .enumerate()
            .skip(self.scroll_y)
            .take(area.height as usize)
            .map(|(idx, line)| {
                if !has_pattern || !self.search.hits.contains(&idx) {
                    return line;
                }
                let bg = if active_hit == Some(idx) {
                    MATCH_ACTIVE_BG
                } else {
                    MATCH_BG
                };
                Line::from(
                    line.spans
                        .into_iter()
                        .map(|s| Span::styled(s.content, s.style.bg(bg)))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        frame.render_widget(Paragraph::new(visible), area);
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        if !self.search.active && !self.search.pattern.is_empty() {
            let total = self.search.hits.len();
            let cur = if total > 0 {
                self.search.hit_cursor + 1
            } else {
                0
            };
            let left = format!(" /{}", self.search.pattern).fg(BRIGHT);
            let right = format!("[{cur}/{total}] ").fg(MUTED);
            let right_len = right.width() as u16;

            frame.render_widget(Paragraph::new(Line::from(vec![left])), area);
            let buf = frame.buffer_mut();
            let rx = area.x + area.width.saturating_sub(right_len);
            buf.set_string(
                rx,
                area.y,
                right.content.as_ref(),
                Style::default().fg(MUTED),
            );
            return;
        }

        let hints = if self.search.active {
            SEARCH_KEYS
        } else {
            KEYS
        };
        frame.render_widget(KeyHints(hints), area);
    }

    fn render_search_overlay(&self, frame: &mut Frame, area: Rect) {
        let (pw, ph) = (
            50u16.min(area.width.saturating_sub(4)),
            3u16.min(area.height.saturating_sub(2)),
        );
        if pw < 10 || ph < 3 {
            return;
        }
        let popup = Rect::new(
            area.x + (area.width.saturating_sub(pw)) / 2,
            area.y + area.height.saturating_sub(ph),
            pw,
            ph,
        );
        let buf = frame.buffer_mut();
        buf.popup(popup, " search ", POPUP_BORDER);
        let iw = popup.width.saturating_sub(2) as usize;
        buf.set_string(
            popup.x + 1,
            popup.y + 1,
            format!(" / {}█", truncate(&self.search.input, iw.saturating_sub(5))),
            Style::reset().fg(BRIGHT),
        );
    }
}

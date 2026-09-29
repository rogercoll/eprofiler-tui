//! Presentation: turns [`State`] into terminal cells.
//!
//! [`Screen`] lays out the header, the active tab and the footer. Each tab's
//! state implements [`TabView`], which owns the tab's body, its one-line
//! detail bar and its key hints.

mod chrome;
mod executables;
mod flamegraph;
mod flamescope;
mod waiting;

pub use chrome::{KeyHints, Keys};

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{StatefulWidget, Widget},
};

use super::state::{ActiveTab, State};
use chrome::Header;
use waiting::Waiting;

/// The whole terminal: header, detail bar, active tab, footer and any open picker.
pub struct Screen;

impl StatefulWidget for Screen {
    type State = State;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut State) {
        if state.active_tab == ActiveTab::Flamegraph && state.fg.graph.root.total_value == 0 {
            Waiting {
                listen_addr: &state.listen_addr,
                error: state.server_error.as_ref(),
            }
            .render(area, buf);
            return;
        }

        let [header, detail, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(area);

        Header(state).render(header, buf);
        let tab_keys = match state.active_tab {
            ActiveTab::Flamegraph => state.fg.render(detail, body, buf),
            ActiveTab::Flamescope => state.fs.render(detail, body, buf),
            ActiveTab::Executables => state.exe.render(detail, body, buf),
        };

        let picker = state.active_picker();
        KeyHints(picker.map_or(tab_keys, |p| p.style.keys)).render(footer, buf);
        if let Some(picker) = picker {
            picker.render(body, buf);
        }
    }
}

/// How one tab is drawn.
trait TabView {
    /// Footer key hints while no picker is open.
    const KEYS: Keys;

    /// Draw the tab body. May update viewport-dependent state, such as
    /// scroll offsets or resolution, that [`Self::detail`] reports.
    fn render_body(&mut self, area: Rect, buf: &mut Buffer);

    /// One-line summary shown above the body.
    fn detail(&self) -> Line<'static>;

    /// Draw body then detail bar, and return the footer key hints.
    fn render(&mut self, detail: Rect, body: Rect, buf: &mut Buffer) -> Keys {
        self.render_body(body, buf);
        self.detail().render(detail, buf);
        Self::KEYS
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Helpers shared by the view tests.

    use std::collections::HashMap;

    use ratatui::{
        Terminal,
        backend::TestBackend,
        buffer::Buffer,
        crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    };

    use super::Screen;
    use crate::flamegraph::FlameGraph;
    use crate::tui::event::Event;
    use crate::tui::state::State;

    pub fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// Two threads, a flamescope sample and two discovered executables.
    pub fn populated_state() -> State {
        let mut state = State::new("0.0.0.0:4317".into(), vec![]);
        let mut fg = FlameGraph::new();
        fg.add_stack(&["worker-1".into(), "main".into(), "do_work".into()], 30);
        fg.add_stack(&["worker-2".into(), "main".into()], 10);
        state.handle_event(Event::ProfileUpdate {
            flamegraph: fg,
            samples: 40,
            timestamps: HashMap::from([("worker-1".into(), vec![0u64, 1_500_000_000])]),
        });
        state.handle_event(Event::MappingsDiscovered(vec![
            "libc.so.6".into(),
            "app".into(),
        ]));
        state
    }

    pub fn render(state: &mut State, width: u16, height: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| f.render_stateful_widget(Screen, f.area(), state))
            .unwrap();
        term.backend().buffer().clone()
    }

    /// The rendered screen as one string of glyphs, for `contains` checks.
    pub fn screen(state: &mut State, width: u16, height: u16) -> String {
        render(state, width, height)
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyCode;

    use ratatui::style::Color;

    use super::testing::{key, populated_state, render, screen};
    use crate::error::Error;
    use crate::tui::state::State;

    #[test]
    fn waiting_screen_before_data() {
        let mut state = State::new("0.0.0.0:4317".into(), vec![]);
        assert!(screen(&mut state, 100, 20).contains("Listening on 0.0.0.0:4317"));
        // Like the tabs, the landing page keeps the terminal background.
        let buf = render(&mut state, 120, 30);
        assert!(buf.content.iter().all(|cell| cell.bg == Color::Reset));
    }

    #[test]
    fn landing_page_explains_why_the_server_is_down() {
        let mut state = State::new("0.0.0.0:4317".into(), vec![]);
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = taken.local_addr().unwrap();
        let source = std::net::TcpListener::bind(addr).unwrap_err();
        state.server_error = Some(Error::Bind { addr, source });

        let shown = screen(&mut state, 120, 30);
        assert!(
            shown.contains(&format!("cannot listen on {addr}")),
            "{shown}"
        );
        assert!(shown.contains("Not receiving profiles"), "{shown}");
        assert!(!shown.contains("Waiting for profiles"), "{shown}");
    }

    #[test]
    fn every_tab_renders_with_and_without_picker_at_awkward_sizes() {
        let mut state = populated_state();
        for (w, h) in [(120, 40), (40, 8), (12, 3)] {
            for _ in 0..3 {
                screen(&mut state, w, h);
                state.handle_event(key(KeyCode::Char('/')));
                assert!(state.active_picker().is_some());
                screen(&mut state, w, h);
                state.handle_event(key(KeyCode::Esc));
                assert!(state.active_picker().is_none());
                state.handle_event(key(KeyCode::Tab));
            }
        }
    }

    #[test]
    fn footer_shows_picker_keys_while_it_is_open() {
        let mut state = populated_state();
        assert!(screen(&mut state, 150, 20).contains("[f/Space] freeze"));
        state.handle_event(key(KeyCode::Char('/')));
        let open = screen(&mut state, 150, 20);
        assert!(open.contains("[Enter] select") && !open.contains("[f/Space] freeze"));
    }
}

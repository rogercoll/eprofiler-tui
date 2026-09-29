mod executables;
mod flamegraph;
mod flamescope;

pub use executables::{ExeEntry, ExecutablesTab};
pub use flamegraph::FlamegraphTab;
pub use flamescope::FlamescopeTab;

use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::error::Error;
use crate::storage::{ExecutableInfo, FileId};
use crate::tui::event::Event;
use crate::tui::widgets::Picker;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTab {
    Flamegraph,
    Flamescope,
    Executables,
}

impl ActiveTab {
    fn next(self) -> Self {
        match self {
            Self::Flamegraph => Self::Flamescope,
            Self::Flamescope => Self::Executables,
            Self::Executables => Self::Flamegraph,
        }
    }
}

/// Side effect requested by the state; executed by the caller.
pub enum Action {
    LoadSymbols(PathBuf, Option<String>),
    RemoveSymbols(String, FileId),
}

pub struct State {
    pub running: bool,
    pub listen_addr: String,
    /// Why the OTLP receiver is not running, if it is not.
    pub server_error: Option<Error>,
    pub active_tab: ActiveTab,
    pub fg: FlamegraphTab,
    pub fs: FlamescopeTab,
    pub exe: ExecutablesTab,
}

impl State {
    pub fn new(listen_addr: String, initial_exes: Vec<ExecutableInfo>) -> Self {
        Self {
            running: true,
            listen_addr,
            server_error: None,
            active_tab: ActiveTab::Flamegraph,
            fg: FlamegraphTab::default(),
            fs: FlamescopeTab::default(),
            exe: ExecutablesTab::from(initial_exes),
        }
    }

    /// The picker open on the active tab, if any.
    pub fn active_picker(&self) -> Option<&Picker> {
        match self.active_tab {
            ActiveTab::Flamegraph => self.fg.picker.as_ref(),
            ActiveTab::Flamescope => self.fs.picker.as_ref(),
            ActiveTab::Executables => self.exe.picker(),
        }
    }

    /// Central event handler: mutates state and returns any side effect to run.
    pub fn handle_event(&mut self, event: Event) -> Option<Action> {
        match event {
            Event::Tick | Event::Resize => None,
            Event::Key(key) => self.handle_key(key),
            Event::ProfileUpdate {
                flamegraph,
                samples,
                timestamps,
            } => {
                if !self.fg.frozen {
                    self.fs.record_timestamps(&timestamps);
                }
                self.fg.merge(flamegraph, samples);
                None
            }
            Event::MappingsDiscovered(names) => {
                self.exe.merge_discovered_mappings(names);
                None
            }
            Event::SymbolsLoaded { target_name, info } => {
                self.exe.handle_symbols_loaded(target_name, info);
                None
            }
            Event::SymbolsRemoved { name, error } => {
                self.exe.handle_symbols_removed(name, error);
                None
            }
            Event::ServerFailed(error) => {
                self.server_error = Some(error);
                None
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        let picker_open = self.active_picker().is_some();

        match key.code {
            _ if ctrl_c => self.running = false,
            KeyCode::Char('q') | KeyCode::Char('Q') if !picker_open => self.running = false,
            KeyCode::Tab if !picker_open => self.active_tab = self.active_tab.next(),
            _ => {
                return match self.active_tab {
                    ActiveTab::Flamegraph => {
                        self.fg.handle_key(key);
                        None
                    }
                    ActiveTab::Flamescope => {
                        self.fs.handle_key(key);
                        None
                    }
                    ActiveTab::Executables => self.exe.handle_key(key),
                };
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::flamegraph::FlameGraph;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_str(state: &mut State, s: &str) {
        for c in s.chars() {
            state.handle_event(key(KeyCode::Char(c)));
        }
    }

    fn populated_state() -> State {
        let mut state = State::new("addr".into(), vec![]);
        let mut fg = FlameGraph::new();
        fg.add_stack(&["worker-1".into(), "main".into()], 5);
        fg.add_stack(&["other".into(), "main".into()], 5);
        let timestamps = HashMap::from([("worker-1".into(), vec![0u64])]);
        state.handle_event(Event::ProfileUpdate {
            flamegraph: fg,
            samples: 10,
            timestamps,
        });
        state.handle_event(Event::MappingsDiscovered(vec!["app".into()]));
        state
    }

    #[test]
    fn quit_keys_are_swallowed_while_a_picker_is_open() {
        let mut state = populated_state();
        state.handle_event(key(KeyCode::Char('/')));
        state.handle_event(key(KeyCode::Char('q')));
        state.handle_event(key(KeyCode::Tab));
        assert!(state.running);
        assert_eq!(state.active_tab, ActiveTab::Flamegraph);
        assert_eq!(state.fg.picker.as_ref().unwrap().input, "q");
    }

    #[test]
    fn flamegraph_search_zooms_into_the_picked_thread() {
        let mut state = populated_state();
        state.handle_event(key(KeyCode::Char('/')));
        type_str(&mut state, "WORK");
        state.handle_event(key(KeyCode::Enter));
        assert!(state.fg.picker.is_none());
        assert_eq!(state.fg.zoom_path, vec!["worker-1".to_string()]);
    }

    #[test]
    fn flamescope_search_filters_by_thread() {
        let mut state = populated_state();
        state.handle_event(key(KeyCode::Tab));
        state.handle_event(key(KeyCode::Char('/')));
        type_str(&mut state, "worker");
        state.handle_event(key(KeyCode::Enter));
        assert_eq!(state.fs.filter.as_deref(), Some("worker-1"));
        assert!(state.fs.auto_scroll);
    }

    #[test]
    fn executables_prompt_returns_a_load_action_with_target() {
        let mut state = populated_state();
        state.handle_event(key(KeyCode::Tab));
        state.handle_event(key(KeyCode::Tab));
        assert!(state.handle_event(key(KeyCode::Enter)).is_none());
        type_str(&mut state, "/bin/ls");
        let action = state.handle_event(key(KeyCode::Enter));
        match action {
            Some(Action::LoadSymbols(path, target)) => {
                assert_eq!(path, PathBuf::from("/bin/ls"));
                assert_eq!(target.as_deref(), Some("app"));
            }
            _ => panic!("expected LoadSymbols"),
        }
        assert_eq!(state.exe.status.as_deref(), Some("Loading app"));
        assert!(state.exe.picker().is_none());
    }

    #[test]
    fn empty_path_submission_is_a_cancel() {
        let mut state = populated_state();
        state.handle_event(key(KeyCode::Tab));
        state.handle_event(key(KeyCode::Tab));
        state.handle_event(key(KeyCode::Char('/')));
        assert!(state.handle_event(key(KeyCode::Enter)).is_none());
        assert!(state.exe.picker().is_none());
    }

    #[test]
    fn server_failure_is_kept_for_the_landing_page() {
        let mut state = State::new("addr".into(), vec![]);
        let source = std::io::Error::from(std::io::ErrorKind::AddrInUse);
        let addr = "0.0.0.0:4317".parse().unwrap();
        assert!(
            state
                .handle_event(Event::ServerFailed(Error::Bind { addr, source }))
                .is_none()
        );
        assert!(matches!(state.server_error, Some(Error::Bind { .. })));
        assert!(state.running, "the UI keeps running");
    }
}

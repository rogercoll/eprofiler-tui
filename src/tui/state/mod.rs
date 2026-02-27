mod executables;
mod flamegraph;
mod flamescope;

pub use executables::ExecutablesTab;
pub use flamegraph::FlamegraphTab;
pub use flamescope::FlamescopeTab;

use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::storage::{ExecutableInfo, FileId};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Flamegraph,
    Flamescope,
    Executables,
}

pub enum Action {
    LoadSymbols(PathBuf, Option<String>),
    RemoveSymbols(String, FileId),
    None,
}

pub struct State {
    pub running: bool,
    pub listen_addr: String,
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
            active_tab: ActiveTab::Flamegraph,
            fg: FlamegraphTab::default(),
            fs: FlamescopeTab::default(),
            exe: ExecutablesTab::from(initial_exes),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.running = false;
            return Action::None;
        }

        let overlay_active =
            self.fg.search.active || self.fs.search.active || self.exe.path_input.active;

        if key.code == KeyCode::Tab && !overlay_active {
            self.active_tab = match self.active_tab {
                ActiveTab::Flamegraph => ActiveTab::Flamescope,
                ActiveTab::Flamescope => ActiveTab::Executables,
                ActiveTab::Executables => ActiveTab::Flamegraph,
            };
            return Action::None;
        }

        if matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q')) && !overlay_active {
            self.running = false;
            return Action::None;
        }

        match self.active_tab {
            ActiveTab::Flamegraph => {
                self.fg.handle_key(key);
                Action::None
            }
            ActiveTab::Flamescope => {
                self.fs.handle_key(key);
                Action::None
            }
            ActiveTab::Executables => self.exe.handle_key(key),
        }
    }
}

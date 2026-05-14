mod executables;
mod flamegraph;
mod flamescope;

pub use executables::ExecutablesTab;
pub use flamegraph::FlamegraphTab;
pub use flamescope::FlamescopeTab;

use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::storage::{ExecutableInfo, FileId};
use crate::tui::event::Event;

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

/// Shared search overlay used by multiple tabs.
#[derive(Default)]
pub struct SearchOverlay {
    pub active: bool,
    pub input: String,
    pub matches: Vec<String>,
    pub cursor: usize,
}

pub enum SearchAction {
    None,
    Closed,
    Selected(Option<String>),
    Refresh,
}

impl SearchOverlay {
    pub fn open(&mut self) { *self = Self { active: true, ..Default::default() }; }

    pub fn close(&mut self) { *self = Self::default(); }

    pub fn handle_key(&mut self, key: KeyEvent) -> SearchAction {
        match key.code {
            KeyCode::Esc => {
                self.close();
                SearchAction::Closed
            }
            KeyCode::Enter => {
                let selected = self.matches.get(self.cursor).cloned();
                self.close();
                SearchAction::Selected(selected)
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.cursor = 0;
                SearchAction::Refresh
            }
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                SearchAction::None
            }
            KeyCode::Down => {
                if self.cursor + 1 < self.matches.len() {
                    self.cursor += 1;
                }
                SearchAction::None
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.cursor = 0;
                SearchAction::Refresh
            }
            _ => SearchAction::None,
        }
    }
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

    /// Central event handler: mutates state and returns an Action for side effects.
    pub fn handle_event(&mut self, event: Event) -> Action {
        match event {
            Event::Tick | Event::Resize => Action::None,
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
                Action::None
            }
            Event::MappingsDiscovered(names) => {
                self.exe.merge_discovered_mappings(names);
                Action::None
            }
            Event::SymbolsLoaded { target_name, info } => {
                self.exe.handle_symbols_loaded(target_name, info);
                Action::None
            }
            Event::SymbolsRemoved { name, error } => {
                self.exe.handle_symbols_removed(name, error);
                Action::None
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Action {
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
            ActiveTab::Flamegraph => { self.fg.handle_key(key); Action::None }
            ActiveTab::Flamescope => { self.fs.handle_key(key); Action::None }
            ActiveTab::Executables => self.exe.handle_key(key),
        }
    }
}

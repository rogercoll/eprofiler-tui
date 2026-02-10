use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use state::State;
use std::{io, panic};

use crate::error::Result;

pub(crate) mod event;
pub(crate) mod state;

mod ui;

type Backend = CrosstermBackend<io::Stderr>;

#[derive(Debug)]
pub struct Tui {
    terminal: Terminal<Backend>,
    pub events: event::EventHandler,
}

impl Tui {
    pub fn new(terminal: Terminal<Backend>, events: event::EventHandler) -> Self {
        Self { terminal, events }
    }

    pub fn init(&mut self) -> Result<()> {
        terminal::enable_raw_mode()?;
        ratatui::crossterm::execute!(io::stderr(), EnterAlternateScreen)?;

        panic::set_hook(Box::new(move |panic| {
            Self::reset().expect("failed to reset terminal");
            better_panic::Settings::auto()
                .most_recent_first(false)
                .lineno_suffix(true)
                .create_panic_handler()(panic);
            std::process::exit(1);
        }));

        self.terminal.hide_cursor()?;
        self.terminal.clear()?;
        Ok(())
    }

    pub fn draw(&mut self, state: &mut State) -> Result<()> {
        self.terminal.draw(|frame| ui::render(state, frame))?;
        Ok(())
    }

    pub fn reset() -> Result<()> {
        terminal::disable_raw_mode()?;
        ratatui::crossterm::execute!(io::stderr(), LeaveAlternateScreen)?;
        Terminal::new(CrosstermBackend::new(io::stderr()))?.show_cursor()?;
        Ok(())
    }

    pub fn exit(&mut self) -> Result<()> {
        terminal::disable_raw_mode()?;
        ratatui::crossterm::execute!(io::stderr(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        self.events.stop();
        Ok(())
    }
}

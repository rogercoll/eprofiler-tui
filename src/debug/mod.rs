mod server;
mod ui;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind};
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};

use crate::error::Result;
use eprofiler_proto::opentelemetry::proto::collector::profiles::v1development::ExportProfilesServiceRequest;

pub(crate) enum DebugEvent {
    Key(KeyEvent),
    NewRequest(ExportProfilesServiceRequest),
    Tick,
}

#[derive(Default)]
pub(crate) struct Search {
    pub active: bool,
    pub input: String,
    pub pattern: String,
    pub hits: Vec<usize>,
    pub hit_cursor: usize,
}

impl Search {
    fn cancel(&mut self) {
        self.active = false;
        self.input.clear();
    }

    fn confirm(&mut self) {
        self.active = false;
        self.pattern = std::mem::take(&mut self.input);
    }

    fn jump(&mut self, forward: bool) -> Option<usize> {
        if self.hits.is_empty() {
            return None;
        }
        self.hit_cursor = if forward {
            (self.hit_cursor + 1) % self.hits.len()
        } else {
            self.hit_cursor
                .checked_sub(1)
                .unwrap_or(self.hits.len() - 1)
        };
        Some(self.hits[self.hit_cursor])
    }
}

#[derive(Default)]
pub(crate) struct DebugState {
    pub requests: Vec<ExportProfilesServiceRequest>,
    pub current: usize,
    pub scroll_y: usize,
    pub running: bool,
    pub listen_addr: String,
    pub search: Search,
}

impl DebugState {
    fn navigate(&mut self, idx: usize) {
        self.current = idx;
        self.scroll_y = 0;
        if !self.search.pattern.is_empty() {
            self.recompute_hits();
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.search.active {
            match key.code {
                KeyCode::Esc => self.search.cancel(),
                KeyCode::Enter => {
                    self.search.confirm();
                    self.recompute_hits();
                    if let Some(&line) = self.search.hits.first() {
                        self.scroll_y = line;
                    }
                }
                KeyCode::Backspace => {
                    self.search.input.pop();
                }
                KeyCode::Char(c) => self.search.input.push(c),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Esc => self.search = Search::default(),
            KeyCode::Char('/') => {
                self.search.active = true;
                self.search.input.clear();
            }
            KeyCode::Char('n') if !self.search.pattern.is_empty() => {
                if let Some(line) = self.search.jump(true) {
                    self.scroll_y = line;
                }
            }
            KeyCode::Char('N') if !self.search.pattern.is_empty() => {
                if let Some(line) = self.search.jump(false) {
                    self.scroll_y = line;
                }
            }
            KeyCode::Char('l') | KeyCode::Right if self.current + 1 < self.requests.len() => {
                self.navigate(self.current + 1);
            }
            KeyCode::Char('h') | KeyCode::Left if self.current > 0 => {
                self.navigate(self.current - 1);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_y = self.scroll_y.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_y = self.scroll_y.saturating_sub(1);
            }
            KeyCode::Char('G') if !self.requests.is_empty() => {
                self.navigate(self.requests.len() - 1);
            }
            KeyCode::Char('g') if !self.requests.is_empty() => self.navigate(0),
            KeyCode::Char('d') | KeyCode::PageDown => {
                self.scroll_y = self.scroll_y.saturating_add(20);
            }
            KeyCode::Char('u') | KeyCode::PageUp => {
                self.scroll_y = self.scroll_y.saturating_sub(20);
            }
            _ => {}
        }
    }
}

pub fn run(port: u16) -> Result<()> {
    let listen_addr = format!("0.0.0.0:{port}");
    let (tx, rx) = mpsc::channel();

    std::thread::spawn({
        let addr = listen_addr.clone();
        let tx = tx.clone();
        move || {
            tokio::runtime::Runtime::new()
                .expect("tokio runtime")
                .block_on(async {
                    if let Err(e) = server::start(tx, &addr).await {
                        eprintln!("gRPC error: {e}");
                    }
                });
        }
    });

    std::thread::spawn(move || {
        let tick = Duration::from_millis(100);
        let mut last = Instant::now();
        loop {
            let timeout = tick.checked_sub(last.elapsed()).unwrap_or(tick);
            if event::poll(timeout).unwrap_or(false)
                && let Ok(CrosstermEvent::Key(k)) = event::read()
                && k.kind == KeyEventKind::Press
            {
                let _ = tx.send(DebugEvent::Key(k));
            }
            if last.elapsed() >= tick {
                let _ = tx.send(DebugEvent::Tick);
                last = Instant::now();
            }
        }
    });

    terminal::enable_raw_mode()?;
    ratatui::crossterm::execute!(std::io::stderr(), EnterAlternateScreen)?;

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        let _ = ratatui::crossterm::execute!(std::io::stderr(), LeaveAlternateScreen);
        hook(info);
    }));

    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stderr()))?;
    terminal.hide_cursor()?;
    terminal.clear()?;

    let mut state = DebugState {
        running: true,
        listen_addr,
        ..Default::default()
    };

    while state.running {
        terminal.draw(|f| state.render(f))?;
        match rx.recv()? {
            DebugEvent::Key(k) => state.handle_key(k),
            DebugEvent::NewRequest(req) => state.requests.push(req),
            DebugEvent::Tick => {}
        }
    }

    terminal::disable_raw_mode()?;
    ratatui::crossterm::execute!(std::io::stderr(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

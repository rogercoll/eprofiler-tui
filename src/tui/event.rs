use ratatui::crossterm::event::{self, Event as CrosstermEvent, KeyEvent, KeyEventKind};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::flamegraph::FlameGraph;

pub enum Event {
    Tick,
    Key(KeyEvent),
    Resize,
    ProfileUpdate {
        flamegraph: FlameGraph,
        samples: u64,
    },
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct EventHandler {
    pub sender: mpsc::Sender<Event>,
    receiver: mpsc::Receiver<Event>,
    handler: thread::JoinHandle<()>,
    running: Arc<AtomicBool>,
}

impl EventHandler {
    pub fn new(tick_rate: u64) -> Self {
        let tick_rate = Duration::from_millis(tick_rate);
        let (sender, receiver) = mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));

        let handler = {
            let sender = sender.clone();
            let running = running.clone();
            thread::spawn(move || {
                let mut last_tick = Instant::now();
                while running.load(Ordering::Relaxed) {
                    let timeout = tick_rate
                        .checked_sub(last_tick.elapsed())
                        .unwrap_or(tick_rate);

                    if event::poll(timeout).expect("event poll failed") {
                        match event::read().expect("event read failed") {
                            CrosstermEvent::Key(e) if e.kind == KeyEventKind::Press => {
                                let _ = sender.send(Event::Key(e));
                            }
                            CrosstermEvent::Resize(_, _) => {
                                let _ = sender.send(Event::Resize);
                            }
                            _ => {}
                        }
                    }

                    if last_tick.elapsed() >= tick_rate {
                        let _ = sender.send(Event::Tick);
                        last_tick = Instant::now();
                    }
                }
            })
        };

        Self {
            sender,
            receiver,
            handler,
            running,
        }
    }

    pub fn next(&self) -> Result<Event> {
        Ok(self.receiver.recv()?)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

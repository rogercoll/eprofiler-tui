use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

mod error;
mod flamegraph;
mod grpc;
mod proto;
mod tui;

use error::Result;
use tui::event::{Event, EventHandler};
use tui::state::State;
use tui::Tui;

fn main() -> Result<()> {
    let events = EventHandler::new(100);

    let grpc_tx = events.sender.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        rt.block_on(async {
            if let Err(e) = grpc::start_server(grpc_tx, "0.0.0.0:4317").await {
                eprintln!("gRPC server error: {e}");
            }
        });
    });

    let backend = CrosstermBackend::new(std::io::stderr());
    let terminal = Terminal::new(backend)?;
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    let mut state = State::new();

    while state.running {
        tui.draw(&mut state)?;

        match tui.events.next()? {
            Event::Tick => {}
            Event::Key(key_event) => state.handle_key(key_event),
            Event::Resize => {}
            Event::ProfileUpdate {
                flamegraph,
                samples,
            } => {
                state.merge_flamegraph(flamegraph, samples);
            }
        }
    }

    tui.exit()?;
    Ok(())
}

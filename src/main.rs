use clap::Parser;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

mod error;
mod flamegraph;
mod grpc;
mod symbolizer;
mod tui;

use error::Result;
use tui::Tui;
use tui::event::{Event, EventHandler};
use tui::state::State;

#[derive(Parser)]
#[command(
    name = "eprofiler-tui",
    about = "Terminal-based OTLP flamegraph viewer"
)]
struct Cli {
    #[arg(short, long, default_value_t = 4317)]
    port: u16,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let listen_addr = format!("0.0.0.0:{}", cli.port);

    let events = EventHandler::new(100);

    let grpc_tx = events.sender.clone();
    let addr = listen_addr.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        rt.block_on(async {
            if let Err(e) = grpc::start_server(grpc_tx, &addr).await {
                eprintln!("gRPC server error: {e}");
            }
        });
    });

    let backend = CrosstermBackend::new(std::io::stderr());
    let terminal = Terminal::new(backend)?;
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    let mut state = State::new(listen_addr);

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

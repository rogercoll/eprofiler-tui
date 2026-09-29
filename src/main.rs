use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

mod debug;
mod error;
mod flamegraph;
mod frame;
mod grpc;
mod otlp;
mod services;
mod storage;
mod symbolizer;
mod tui;

use error::{Error, Result};
use services::Services;
use storage::SymbolStore;
use tui::Tui;
use tui::event::EventHandler;
use tui::state::State;

/// Milliseconds between UI ticks.
const TICK_RATE_MS: u64 = 100;

#[derive(Parser)]
#[command(
    name = "eprofiler-tui",
    about = "Terminal-based OTLP flamegraph viewer"
)]
struct Cli {
    #[arg(short, long, default_value_t = 4317)]
    port: u16,
    /// Symbol store directory (default: $XDG_DATA_HOME/eprofiler-tui,
    /// typically ~/.local/share/eprofiler-tui on Linux)
    #[arg(short = 'd', long = "data-dir", value_name = "PATH")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Inspect raw OTLP ResourceProfiles one by one
    Debug {
        /// Port to listen on (overrides --port)
        #[arg(short, long)]
        port: Option<u16>,
    },
}

impl Cli {
    fn listen_addr(&self) -> SocketAddr {
        SocketAddr::from(([0, 0, 0, 0], self.port))
    }

    /// The symbol store directory, created if missing.
    fn storage_path(&self) -> Result<PathBuf> {
        let path = match &self.data_dir {
            Some(path) => path.clone(),
            None => ProjectDirs::from("", "", "eprofiler-tui")
                .ok_or(Error::NoHomeDir)?
                .data_local_dir()
                .to_path_buf(),
        };
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(Commands::Debug { port }) = cli.command {
        return debug::run(port.unwrap_or(cli.port));
    }

    let addr = cli.listen_addr();
    let store = Arc::new(SymbolStore::open(cli.storage_path()?)?);
    let events = EventHandler::new(TICK_RATE_MS);
    let services = Services::new(Arc::clone(&store), events.sender.clone());
    services.serve(addr);

    let mut tui = Tui::new(
        Terminal::new(CrosstermBackend::new(std::io::stderr()))?,
        events,
    );
    tui.init()?;

    let mut state = State::new(addr.to_string(), store.list_files()?);
    while state.running {
        tui.draw(&mut state)?;
        if let Some(action) = state.handle_event(tui.events.next()?) {
            services.run(action);
        }
    }

    tui.exit()
}

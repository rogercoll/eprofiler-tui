use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

mod debug;
mod error;
mod flamegraph;
mod grpc;
mod storage;
mod symbolizer;
mod tui;

use error::Result;
use storage::SymbolStore;
use tui::Tui;
use tui::event::{Event, EventHandler};
use tui::state::{Action, State};

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

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(Commands::Debug { port }) = cli.command {
        return debug::run(port.unwrap_or(cli.port));
    }

    let listen_addr = format!("0.0.0.0:{}", cli.port);
    let storage_path = resolve_storage_path(cli.data_dir)?;
    let store = Arc::new(SymbolStore::open(&storage_path)?);
    let events = EventHandler::new(100);

    spawn_grpc_server(
        Arc::clone(&store),
        listen_addr.clone(),
        events.sender.clone(),
    );

    let backend = CrosstermBackend::new(std::io::stderr());
    let terminal = Terminal::new(backend)?;
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    let mut state = State::new(listen_addr, store.list_files()?);

    while state.running {
        tui.draw(&mut state)?;

        match state.handle_event(tui.events.next()?) {
            Action::None => {}
            Action::LoadSymbols(path, target_name) => {
                spawn_symbol_load(
                    Arc::clone(&store),
                    tui.events.sender.clone(),
                    path,
                    target_name,
                );
            }
            Action::RemoveSymbols(name, file_id) => {
                state.exe.status = Some(format!("Removing {name}"));
                spawn_symbol_remove(Arc::clone(&store), tui.events.sender.clone(), name, file_id);
            }
        }
    }

    tui.exit()?;
    Ok(())
}

fn resolve_storage_path(data_dir: Option<PathBuf>) -> Result<PathBuf> {
    let path = match data_dir {
        Some(p) => p,
        None => ProjectDirs::from("", "", "eprofiler-tui")
            .expect("Could not determine the user's home directory!")
            .data_local_dir()
            .to_path_buf(),
    };
    if !path.exists() {
        std::fs::create_dir_all(&path)
            .expect("Failed to create the storage directory. Check permissions.");
    }
    Ok(path)
}

fn spawn_grpc_server(
    store: Arc<SymbolStore>,
    listen_addr: String,
    event_tx: std::sync::mpsc::Sender<Event>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        rt.block_on(async {
            if let Err(e) = grpc::start_server(event_tx, &listen_addr, store).await {
                eprintln!("gRPC server error: {e}");
            }
        });
    });
}

fn spawn_symbol_load(
    store: Arc<SymbolStore>,
    sender: std::sync::mpsc::Sender<Event>,
    path: PathBuf,
    target_name: Option<String>,
) {
    std::thread::spawn(move || {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());

        let _ = sender.send(Event::SymbolsLoaded {
            target_name: target_name.unwrap_or_else(|| file_name.clone()),
            info: symbolizer::extract_symbols(&path).and_then(|file_sym| {
                let info = storage::ExecutableInfo {
                    file_id: file_sym.file_id,
                    file_name,
                    num_ranges: file_sym.ranges.len() as u32,
                };
                store.store_file_symbols(&file_sym, &path)?;
                Ok(info)
            }),
        });
    });
}

fn spawn_symbol_remove(
    store: Arc<SymbolStore>,
    sender: std::sync::mpsc::Sender<Event>,
    name: String,
    file_id: storage::FileId,
) {
    std::thread::spawn(move || {
        let _ = sender.send(Event::SymbolsRemoved {
            name,
            error: store.remove_file_symbols(file_id).err(),
        });
    });
}

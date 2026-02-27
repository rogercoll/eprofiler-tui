use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use directories::ProjectDirs;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

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
use tui::state::State;

use crate::tui::state::Action;

#[derive(Parser)]
#[command(
    name = "eprofiler-tui",
    about = "Terminal-based OTLP flamegraph viewer"
)]
struct Cli {
    #[arg(short, long, default_value_t = 4317)]
    port: u16,
    #[arg(short = 'd', long = "data-dir", value_name = "PATH")]
    data_dir: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let listen_addr = format!("0.0.0.0:{}", cli.port);

    let storage_path: PathBuf = match cli.data_dir {
        Some(custom_path) => custom_path,
        None => {
            // ProjectDirs::from takes (qualifier, organization, application_name)
            let proj_dirs = ProjectDirs::from("", "", "eprofiler-tui")
                .expect("Could not determine the user's home directory!");
            proj_dirs.data_local_dir().to_path_buf()
        }
    };

    if !storage_path.exists() {
        std::fs::create_dir_all(&storage_path)
            .expect("Failed to create the storage directory. Check permissions.");
    }

    let store = Arc::new(SymbolStore::open(storage_path)?);
    let events = EventHandler::new(100);

    std::thread::spawn({
        let store = Arc::clone(&store);
        let listen_addr = listen_addr.clone();
        let events = events.sender.clone();
        move || {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            rt.block_on(async {
                if let Err(e) = grpc::start_server(events, &listen_addr, Some(store)).await {
                    eprintln!("gRPC server error: {e}");
                }
            });
        }
    });

    let backend = CrosstermBackend::new(std::io::stderr());
    let terminal = Terminal::new(backend)?;
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    let mut state = State::new(listen_addr, store.list_files()?);

    while state.running {
        tui.draw(&mut state)?;

        match tui.events.next()? {
            Event::Tick => {}
            Event::Key(key_event) => match state.handle_key(key_event) {
                Action::None => {}
                Action::LoadSymbols(path, target_name) => {
                    std::thread::spawn({
                        let store = Arc::clone(&store);
                        let sender = tui.events.sender.clone();
                        move || {
                            let file_name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.display().to_string());

                            let _ = sender.send(Event::SymbolsLoaded {
                                target_name: target_name.unwrap_or(file_name.clone()),
                                info: symbolizer::extract_symbols(&path).and_then(|file_sym| {
                                    let info = storage::ExecutableInfo {
                                        file_id: file_sym.file_id,
                                        file_name: file_name,
                                        num_ranges: file_sym.ranges.len() as u32,
                                    };
                                    store.store_file_symbols(&file_sym, &path)?;
                                    Ok(info)
                                }),
                            });
                        }
                    });
                }
                Action::RemoveSymbols(name, file_id) => {
                    state.exe.status = Some(format!("Removing {}", name));
                    std::thread::spawn({
                        let sender = tui.events.sender.clone();
                        let store = Arc::clone(&store);
                        move || {
                            sender.send(Event::SymbolsRemoved {
                                name: name,
                                error: store.remove_file_symbols(file_id).err(),
                            })
                        }
                    });
                }
            },
            Event::Resize => {}
            Event::ProfileUpdate {
                flamegraph,
                samples,
            } => {
                state.fg.merge(flamegraph, samples);
            }
            Event::MappingsDiscovered(names) => {
                state.exe.merge_discovered_mappings(names);
            }
            Event::SymbolsLoaded { target_name, info } => {
                match info {
                    Ok(info) => {
                        state.exe.status = Some(format!(
                            "Loaded {} symbols for {}",
                            info.num_ranges, target_name
                        ));
                        state.exe.update_symbolized(target_name, info);
                    }
                    Err(err) => {
                        state.exe.status =
                            Some(format!("Error loading {}: {}", target_name, err))
                    }
                };
            }
            Event::SymbolsRemoved { name, error } => {
                state.exe.status = Some(
                    error
                        .map(|err| format!("Error removing {}: {}", name, err))
                        .unwrap_or(format!("Removed symbols for {}", name)),
                );
                state.exe.clear_symbols(&name);
            }
        }
    }

    tui.exit()?;
    Ok(())
}

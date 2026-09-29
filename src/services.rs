//! Background services for the UI: the OTLP receiver and symbol loading. Each
//! reports back to the UI loop as an [`Event`].

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;

use crate::grpc::ProfilesServer;
use crate::storage::{ExecutableInfo, FileId, SymbolStore};
use crate::symbolizer::FileSym;
use crate::tui::event::Event;
use crate::tui::state::Action;

pub struct Services {
    store: Arc<SymbolStore>,
    events: mpsc::Sender<Event>,
}

impl Services {
    pub fn new(store: Arc<SymbolStore>, events: mpsc::Sender<Event>) -> Self {
        Self { store, events }
    }

    /// Run the OTLP receiver on its own thread and Tokio runtime.
    pub fn serve(&self, addr: SocketAddr) {
        let server = ProfilesServer::new(self.events.clone(), Arc::clone(&self.store));
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            if let Err(e) = runtime.block_on(server.serve(addr)) {
                eprintln!("gRPC server error: {e}");
            }
        });
    }

    pub fn run(&self, action: Action) {
        match action {
            Action::LoadSymbols(path, target) => self.load_symbols(path, target),
            Action::RemoveSymbols(name, file_id) => self.remove_symbols(name, file_id),
        }
    }

    /// Extract symbols from the executable at `path` and store them. `target`
    /// is the discovered mapping they belong to, if the user picked one.
    fn load_symbols(&self, path: PathBuf, target: Option<String>) {
        let (store, events) = (Arc::clone(&self.store), self.events.clone());
        thread::spawn(move || {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let info = FileSym::extract(&path).and_then(|symbols| {
                store.store_file_symbols(&symbols, &path)?;
                Ok(ExecutableInfo {
                    file_id: symbols.file_id,
                    num_ranges: symbols.ranges.len() as u32,
                    file_name: file_name.clone(),
                })
            });
            let _ = events.send(Event::SymbolsLoaded {
                target_name: target.unwrap_or(file_name),
                info,
            });
        });
    }

    fn remove_symbols(&self, name: String, file_id: FileId) {
        let (store, events) = (Arc::clone(&self.store), self.events.clone());
        thread::spawn(move || {
            let error = store.remove_file_symbols(file_id).err();
            let _ = events.send(Event::SymbolsRemoved { name, error });
        });
    }
}

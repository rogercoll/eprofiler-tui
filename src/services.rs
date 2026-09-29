//! Background services for the UI: the OTLP receiver and symbol loading. Each
//! reports back to the UI loop as an [`Event`].

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;

use tonic::transport::server::TcpIncoming;

use crate::error::{Error, Result};
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

    /// Bind `addr` and run the OTLP receiver on its own thread and Tokio
    /// runtime. Binding happens here, so a busy port or missing permission is
    /// returned; a failure after that arrives as [`Event::ServerFailed`].
    pub fn serve(&self, addr: SocketAddr) -> Result<()> {
        let runtime = tokio::runtime::Runtime::new()?;
        // The listener registers with the runtime's reactor, so bind inside it.
        let incoming = {
            let _guard = runtime.enter();
            TcpIncoming::bind(addr).map_err(|source| Error::Bind { addr, source })?
        };
        let server = ProfilesServer::new(self.events.clone(), Arc::clone(&self.store));
        let events = self.events.clone();
        thread::spawn(move || {
            if let Err(e) = runtime.block_on(server.serve(incoming)) {
                let _ = events.send(Event::ServerFailed(e.into()));
            }
        });
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn services() -> (Services, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(SymbolStore::open(tmp.path()).unwrap());
        (Services::new(store, mpsc::channel().0), tmp)
    }

    #[test]
    fn serving_on_a_busy_port_reports_the_address() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = taken.local_addr().unwrap();
        let (services, _tmp) = services();

        let error = services.serve(addr).unwrap_err();
        assert!(matches!(error, Error::Bind { addr: a, .. } if a == addr));
        assert!(
            error
                .to_string()
                .starts_with(&format!("cannot listen on {addr}: "))
        );
    }

    #[test]
    fn serving_on_a_free_port_succeeds() {
        let (services, _tmp) = services();
        assert!(services.serve("127.0.0.1:0".parse().unwrap()).is_ok());
    }
}

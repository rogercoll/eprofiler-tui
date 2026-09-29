use std::net::SocketAddr;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Channel receive error: {0}")]
    Recv(#[from] std::sync::mpsc::RecvError),
    #[error("cannot listen on {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
    #[error("gRPC server stopped: {0}")]
    Grpc(#[from] tonic::transport::Error),
    #[error("symbolization parsing error: {0}")]
    SymParsing(#[from] symblib::objfile::Error),
    #[error("symbolization dwarf parsing error: {0}")]
    SymDwarf(#[from] symblib::dwarf::Error),
    #[error("symbolization error: {0}")]
    SymMulti(#[from] symblib::symbconv::multi::Error),
    #[error("multi symbolization multi error: {0}")]
    SymConv(#[from] symblib::symbconv::Error),
    #[error("storage error: {0}")]
    Storage(#[from] fjall::Error),
    #[error("incompatible storage format at `{}`: delete the directory and restart", .0.display())]
    StorageVersionMismatch(PathBuf),
    #[error("cannot determine the home directory; pass --data-dir")]
    NoHomeDir,
}

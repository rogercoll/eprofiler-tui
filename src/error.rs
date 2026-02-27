pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Channel receive error: {0}")]
    Recv(#[from] std::sync::mpsc::RecvError),
    #[error("gRPC transport error: {0}")]
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
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Channel receive error: {0}")]
    Recv(#[from] std::sync::mpsc::RecvError),
    #[error("gRPC transport error: {0}")]
    Grpc(#[from] tonic::transport::Error),
}

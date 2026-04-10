use std::sync::mpsc;

use tonic::{Request, Response, Status};

use eprofiler_proto::opentelemetry::proto::collector::profiles::v1development as collector;

use super::DebugEvent;

struct Server {
    tx: mpsc::Sender<DebugEvent>,
}

#[tonic::async_trait]
impl collector::profiles_service_server::ProfilesService for Server {
    async fn export(
        &self,
        request: Request<collector::ExportProfilesServiceRequest>,
    ) -> Result<Response<collector::ExportProfilesServiceResponse>, Status> {
        let _ = self.tx.send(DebugEvent::NewRequest(request.into_inner()));
        Ok(Response::new(collector::ExportProfilesServiceResponse {
            partial_success: None,
        }))
    }
}

pub async fn start(
    tx: mpsc::Sender<DebugEvent>,
    addr: &str,
) -> Result<(), tonic::transport::Error> {
    let addr = addr.parse().expect("invalid gRPC listen address");
    let server = Server { tx };

    tonic::transport::Server::builder()
        .add_service(
            collector::profiles_service_server::ProfilesServiceServer::new(server)
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                .send_compressed(tonic::codec::CompressionEncoding::Gzip),
        )
        .serve(addr)
        .await
}

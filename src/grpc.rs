//! OTLP profiles gRPC receiver: accepts export requests and forwards the
//! decoded data to the UI as events.

use std::sync::{Arc, mpsc};

use eprofiler_proto::opentelemetry::proto::collector::profiles::v1development as collector;
use tonic::codec::CompressionEncoding;
use tonic::transport::server::TcpIncoming;
use tonic::{Request, Response, Status};

use crate::otlp::{Decoder, KnownMappings};
use crate::storage::SymbolStore;
use crate::tui::event::Event;

pub struct ProfilesServer {
    events: mpsc::Sender<Event>,
    store: Arc<SymbolStore>,
    known: Arc<KnownMappings>,
}

impl ProfilesServer {
    pub fn new(events: mpsc::Sender<Event>, store: Arc<SymbolStore>) -> Self {
        Self {
            events,
            store,
            known: Arc::default(),
        }
    }

    /// Serve connections from `incoming` until the transport fails.
    pub async fn serve(self, incoming: TcpIncoming) -> Result<(), tonic::transport::Error> {
        tonic::transport::Server::builder()
            .add_service(
                collector::profiles_service_server::ProfilesServiceServer::new(self)
                    .accept_compressed(CompressionEncoding::Gzip)
                    .send_compressed(CompressionEncoding::Gzip),
            )
            .serve_with_incoming(incoming)
            .await
    }

    /// Decode on the blocking pool (symbol lookups hit disk) and publish.
    fn publish(&self, request: collector::ExportProfilesServiceRequest) {
        let (events, store, known) = (
            self.events.clone(),
            Arc::clone(&self.store),
            Arc::clone(&self.known),
        );
        tokio::task::spawn_blocking(move || {
            let Some(batch) = Decoder::decode(&request, &store, &known) else {
                return;
            };
            if !batch.new_mappings.is_empty() {
                let _ = events.send(Event::MappingsDiscovered(batch.new_mappings));
            }
            let _ = events.send(Event::ProfileUpdate {
                flamegraph: batch.flamegraph,
                samples: batch.samples,
                timestamps: batch.timestamps,
            });
        });
    }
}

#[tonic::async_trait]
impl collector::profiles_service_server::ProfilesService for ProfilesServer {
    async fn export(
        &self,
        request: Request<collector::ExportProfilesServiceRequest>,
    ) -> Result<Response<collector::ExportProfilesServiceResponse>, Status> {
        self.publish(request.into_inner());
        Ok(Response::new(collector::ExportProfilesServiceResponse {
            partial_success: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::frame::{FrameKind, Runtime};
    use eprofiler_proto::opentelemetry::proto::common::v1 as common;
    use eprofiler_proto::opentelemetry::proto::profiles::v1development as profiles;

    use collector::ExportProfilesServiceRequest;
    use collector::profiles_service_client::ProfilesServiceClient;
    use common::AnyValue;
    use common::any_value;
    use profiles::{
        Function, KeyValueAndUnit, Line, Location, Profile, ProfilesDictionary, ResourceProfiles,
        Sample, ScopeProfiles, Stack,
    };

    async fn setup_server(tx: mpsc::Sender<Event>) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::storage::SymbolStore::open(tmp.path()).unwrap());
        tokio::spawn(async move {
            let _tmp = tmp;
            let server = ProfilesServer::new(tx, store);
            tonic::transport::Server::builder()
                .add_service(collector::profiles_service_server::ProfilesServiceServer::new(server))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        port
    }

    fn build_dictionary() -> ProfilesDictionary {
        ProfilesDictionary {
            string_table: vec![
                "".into(),
                "thread.name".into(),
                "worker-1".into(),
                "do_work".into(),
                "main".into(),
            ],
            attribute_table: vec![
                KeyValueAndUnit::default(),
                KeyValueAndUnit {
                    key_strindex: 1,
                    value: Some(AnyValue {
                        value: Some(any_value::Value::StringValue("worker-1".into())),
                    }),
                    unit_strindex: 0,
                },
            ],
            function_table: vec![
                Function::default(),
                Function {
                    name_strindex: 3,
                    ..Default::default()
                },
                Function {
                    name_strindex: 4,
                    ..Default::default()
                },
            ],
            location_table: vec![
                Location::default(),
                Location {
                    lines: vec![Line {
                        function_index: 1,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Location {
                    lines: vec![Line {
                        function_index: 2,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            stack_table: vec![
                Stack::default(),
                Stack {
                    location_indices: vec![1, 2],
                },
            ],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_export_with_values() {
        let (tx, rx) = mpsc::channel();
        let port = setup_server(tx).await;

        let mut client = ProfilesServiceClient::connect(format!("http://127.0.0.1:{port}"))
            .await
            .unwrap();

        let sample = Sample {
            stack_index: 1,
            values: vec![10],
            attribute_indices: vec![1],
            ..Default::default()
        };
        let req = ExportProfilesServiceRequest {
            dictionary: Some(build_dictionary()),
            resource_profiles: vec![ResourceProfiles {
                scope_profiles: vec![ScopeProfiles {
                    profiles: vec![Profile {
                        samples: vec![sample],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };

        client.export(req).await.unwrap();

        let event = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        match event {
            Event::ProfileUpdate {
                flamegraph,
                samples,
                timestamps,
            } => {
                assert_eq!(samples, 10);
                assert!(timestamps.is_empty());
                let thread = &flamegraph.root.children[0];
                assert_eq!(thread.name, "worker-1");
                assert_eq!(thread.total_value, 10);
                assert_eq!(thread.kind, FrameKind::THREAD);
                assert_eq!(thread.children[0].name, "main");
                assert_eq!(thread.children[0].kind.runtime, Runtime::Unknown);
                assert_eq!(thread.children[0].children[0].name, "do_work");
            }
            _ => panic!("expected ProfileUpdate event"),
        }
    }

    #[tokio::test]
    async fn test_export_timestamps_take_priority() {
        let (tx, rx) = mpsc::channel();
        let port = setup_server(tx).await;

        let mut client = ProfilesServiceClient::connect(format!("http://127.0.0.1:{port}"))
            .await
            .unwrap();

        let sample = Sample {
            stack_index: 1,
            values: vec![1],
            timestamps_unix_nano: vec![100, 200, 300, 400, 500],
            attribute_indices: vec![1],
            ..Default::default()
        };
        let req = ExportProfilesServiceRequest {
            dictionary: Some(build_dictionary()),
            resource_profiles: vec![ResourceProfiles {
                scope_profiles: vec![ScopeProfiles {
                    profiles: vec![Profile {
                        samples: vec![sample],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };

        client.export(req).await.unwrap();

        let event = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        match event {
            Event::ProfileUpdate {
                flamegraph,
                samples,
                timestamps,
            } => {
                assert_eq!(samples, 5);
                assert_eq!(
                    timestamps.get("worker-1").unwrap(),
                    &vec![100, 200, 300, 400, 500]
                );
                let thread = &flamegraph.root.children[0];
                assert_eq!(thread.total_value, 5);
            }
            _ => panic!("expected ProfileUpdate event"),
        }
    }
}

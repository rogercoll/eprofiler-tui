use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock, mpsc};
use tonic::{Request, Response, Status};

use crate::flamegraph::FlameGraph;
use crate::storage::SymbolStore;
use crate::tui::event::Event;
use eprofiler_proto::opentelemetry::proto::collector::profiles::v1development as collector;
use eprofiler_proto::opentelemetry::proto::common::v1 as common;
use eprofiler_proto::opentelemetry::proto::profiles::v1development as profiles;

pub struct ProfilesServer {
    event_tx: mpsc::Sender<Event>,
    store: Arc<SymbolStore>,
    known_basenames: Arc<RwLock<HashSet<String>>>,
}

impl ProfilesServer {
    pub fn new(event_tx: mpsc::Sender<Event>, store: Arc<SymbolStore>) -> Self {
        Self {
            event_tx,
            store,
            known_basenames: Arc::new(RwLock::new(HashSet::new())),
        }
    }
}

/// Thin wrapper around `ProfilesDictionary` for ergonomic lookups.
struct Dict<'a> {
    d: &'a profiles::ProfilesDictionary,
}

impl<'a> Dict<'a> {
    fn new(d: &'a profiles::ProfilesDictionary) -> Self {
        Self { d }
    }

    fn str(&self, idx: i32) -> Option<&'a str> {
        self.d
            .string_table
            .get(idx as usize)
            .filter(|s| !s.is_empty())
            .map(String::as_str)
    }

    fn func_name(&self, line: &profiles::Line) -> &'a str {
        self.d
            .function_table
            .get(line.function_index as usize)
            .filter(|_| line.function_index > 0)
            .and_then(|f| self.str(f.name_strindex))
            .unwrap_or("[unknown]")
    }

    fn mapping_basename(&self, location: &profiles::Location) -> &'a str {
        self.d
            .mapping_table
            .get(location.mapping_index as usize)
            .filter(|_| location.mapping_index > 0)
            .and_then(|m| self.str(m.filename_strindex))
            .map(|full| full.rsplit('/').next().unwrap_or(full))
            .unwrap_or("[unknown]")
    }

    fn frame_type(&self, location: &profiles::Location) -> &str {
        self.find_attr_value(&location.attribute_indices, "profile.frame.type")
            .map(|s| match s {
                "native" => "Native",
                "kernel" => "Kernel",
                "jvm" => "JVM",
                "cpython" => "Python",
                "php" | "phpjit" => "PHP",
                "ruby" => "Ruby",
                "perl" => "Perl",
                "v8js" => "JS",
                "dotnet" => ".NET",
                "beam" => "Beam",
                "go" => "Go",
                other => other,
            })
            .unwrap_or("Unknown")
    }

    fn thread_name(&self, sample: &profiles::Sample) -> &'a str {
        self.find_attr_value(&sample.attribute_indices, "thread.name")
            .unwrap_or("[unknown]")
    }

    fn find_attr_value(&self, indices: &[i32], key: &str) -> Option<&'a str> {
        indices.iter().find_map(|&idx| {
            let attr = self
                .d
                .attribute_table
                .get(idx as usize)
                .filter(|_| idx > 0)?;
            let k = self.str(attr.key_strindex)?;
            if k != key {
                return None;
            }
            match attr.value.as_ref()?.value.as_ref()? {
                common::any_value::Value::StringValue(s) if !s.is_empty() => Some(s.as_str()),
                _ => None,
            }
        })
    }

    fn unknown_basenames(&self, known: &RwLock<HashSet<String>>) -> Vec<String> {
        self.d
            .mapping_table
            .iter()
            .skip(1)
            .filter_map(|mapping| {
                let full_path = self.str(mapping.filename_strindex)?;
                if known.read().ok()?.contains(full_path) {
                    return None;
                }
                let basename = full_path.rsplit('/').next().unwrap_or(full_path);
                if basename.is_empty() || basename.starts_with('[') {
                    return None;
                }
                known.write().ok()?.insert(full_path.to_string());
                Some(basename.to_string())
            })
            .collect()
    }
}

/// Pre-resolves the location table into human-readable strings.
fn pre_resolve_locations(dict: &Dict, store: &SymbolStore) -> Vec<String> {
    dict.d
        .location_table
        .iter()
        .map(|location| {
            let tag = dict.frame_type(location);
            if location.lines.is_empty() {
                if tag == "Native"
                    && let Some(names) = symbolize_native(store, location, dict)
                {
                    return names
                        .iter()
                        .enumerate()
                        .map(|(i, n)| {
                            format!("{n} [Native]{}", if i > 0 { " [Inline]" } else { "" })
                        })
                        .collect::<Vec<_>>()
                        .join(" / ");
                }
                let basename = dict.mapping_basename(location);
                format!("{basename}+0x{:016x} [{tag}]", location.address)
            } else {
                location
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(i, line)| {
                        format!(
                            "{} [{tag}]{}",
                            dict.func_name(line),
                            if i > 0 { " [Inline]" } else { "" }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" / ")
            }
        })
        .collect()
}

fn process_export(
    req: collector::ExportProfilesServiceRequest,
    store: &SymbolStore,
    known: &RwLock<HashSet<String>>,
    event_tx: &mpsc::Sender<Event>,
) {
    let Some(raw_dict) = req.dictionary.as_ref() else {
        return;
    };
    let dict = Dict::new(raw_dict);

    let mut flamegraph = FlameGraph::new();
    let mut stack_cache: HashMap<i32, Vec<String>> = HashMap::new();
    let location_cache = pre_resolve_locations(&dict, store);
    let mut sample_count: u64 = 0;
    let mut thread_timestamps: HashMap<String, Vec<u64>> = HashMap::new();

    let samples = req
        .resource_profiles
        .iter()
        .flat_map(|rp| &rp.scope_profiles)
        .flat_map(|sp| &sp.profiles)
        .flat_map(|p| &p.samples);

    for sample in samples {
        let stack = stack_cache.entry(sample.stack_index).or_insert_with(|| {
            let idx = sample.stack_index as usize;
            if idx == 0 || idx >= dict.d.stack_table.len() {
                return Vec::new();
            }

            let mut frames: Vec<String> = dict.d.stack_table[idx]
                .location_indices
                .iter()
                .filter_map(|&loc_idx| location_cache.get(loc_idx as usize).cloned())
                .collect();
            frames.reverse();

            let comm = dict.thread_name(sample).to_string();
            let mut result = Vec::with_capacity(frames.len() + 1);
            result.push(comm);
            result.extend(frames);
            result
        });

        if stack.is_empty() {
            continue;
        }

        let value = if !sample.timestamps_unix_nano.is_empty() {
            thread_timestamps
                .entry(stack[0].clone())
                .or_default()
                .extend_from_slice(&sample.timestamps_unix_nano);
            sample.timestamps_unix_nano.len() as i64
        } else if !sample.values.is_empty() {
            sample.values.iter().sum::<i64>().max(1)
        } else {
            1
        };

        flamegraph.add_stack(stack, value);
        sample_count += value as u64;
    }

    let basenames = dict.unknown_basenames(known);
    if !basenames.is_empty() {
        let _ = event_tx.send(Event::MappingsDiscovered(basenames));
    }
    let _ = event_tx.send(Event::ProfileUpdate {
        flamegraph,
        samples: sample_count,
        timestamps: thread_timestamps,
    });
}

#[tonic::async_trait]
impl collector::profiles_service_server::ProfilesService for ProfilesServer {
    async fn export(
        &self,
        request: Request<collector::ExportProfilesServiceRequest>,
    ) -> Result<Response<collector::ExportProfilesServiceResponse>, Status> {
        tokio::task::spawn_blocking({
            let store = self.store.clone();
            let known_basenames = Arc::clone(&self.known_basenames);
            let event_tx = self.event_tx.clone();
            move || {
                process_export(
                    request.into_inner(),
                    store.as_ref(),
                    &known_basenames,
                    &event_tx,
                );
            }
        });

        Ok(Response::new(collector::ExportProfilesServiceResponse {
            partial_success: None,
        }))
    }
}

fn symbolize_native(
    store: &SymbolStore,
    location: &profiles::Location,
    dict: &Dict,
) -> Option<Vec<String>> {
    let resolved = store
        .lookup(
            store.file_id_for_basename(dict.mapping_basename(location))?,
            location.address,
        )
        .ok()?;
    if resolved.is_empty() {
        return None;
    }
    Some(resolved.into_iter().map(|f| f.func).collect())
}

pub async fn start_server(
    event_tx: mpsc::Sender<Event>,
    addr: &str,
    store: Arc<SymbolStore>,
) -> Result<(), tonic::transport::Error> {
    let addr = addr.parse().expect("invalid gRPC listen address");
    let server = ProfilesServer::new(event_tx, store);

    tonic::transport::Server::builder()
        .add_service(
            collector::profiles_service_server::ProfilesServiceServer::new(server)
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                .send_compressed(tonic::codec::CompressionEncoding::Gzip),
        )
        .serve(addr)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
                assert_eq!(thread.children[0].name, "main [Unknown]");
                assert_eq!(thread.children[0].children[0].name, "do_work [Unknown]");
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

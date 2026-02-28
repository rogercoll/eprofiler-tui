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

fn unknown_basenames(
    known: &RwLock<HashSet<String>>,
    dict: &profiles::ProfilesDictionary,
) -> Vec<String> {
    dict.mapping_table
        .iter()
        .skip(1)
        .fold(Vec::new(), |mut names, mapping| {
            let name_idx = mapping.filename_strindex as usize;
            if name_idx != 0 && name_idx < dict.string_table.len() {
                let full_path = &dict.string_table[name_idx];
                if !full_path.is_empty() && !known.read().unwrap().contains(full_path) {
                    let basename = full_path.rsplit('/').next().unwrap_or(full_path);
                    if !basename.is_empty() && !basename.starts_with('[') {
                        known.write().unwrap().insert(full_path.to_string());
                        names.push(basename.to_string());
                    }
                }
            }
            names
        })
}

/// Pre-resolves the location table into human-readable strings.
/// This turns a complex Protobuf traversal into a simple O(1) vector lookup.
fn pre_resolve_locations(dict: &profiles::ProfilesDictionary, store: &SymbolStore) -> Vec<String> {
    dict.location_table
        .iter()
        .map(|location| {
            let frame_tag = resolve_frame_type(location, dict);
            if location.lines.is_empty() {
                // Try native symbolication
                if frame_tag == "Native" {
                    if let Some(names) = symbolize_native(store, location, dict) {
                        // Join inlined native frames into one string for the cache
                        return names
                            .iter()
                            .enumerate()
                            .map(|(i, n)| {
                                format!("{} [Native]{}", n, if i > 0 { " [Inline]" } else { "" })
                            })
                            .collect::<Vec<_>>()
                            .join(" / ");
                    }
                }
                format_with_tag(&resolve_unsymbolized_label(location, dict), &frame_tag)
            } else {
                // Resolve known lines
                location
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(i, line)| {
                        let func_name = resolve_function_name(line, dict);
                        format!(
                            "{} [{}]{}",
                            func_name,
                            frame_tag,
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
    let mut flamegraph = FlameGraph::new();
    let Some(dict) = req.dictionary.as_ref() else {
        // return, no string data can be referenced
        return;
    };

    let mut stack_cache: HashMap<i32, Vec<String>> = HashMap::new();
    let location_cache = pre_resolve_locations(dict, store);

    let mut sample_count: u64 = 0;
    let mut thread_timestamps: HashMap<String, Vec<u64>> = HashMap::new();

    for resource_profiles in &req.resource_profiles {
        for scope_profiles in &resource_profiles.scope_profiles {
            for profile in &scope_profiles.profiles {
                for sample in &profile.samples {
                    let stack = stack_cache.entry(sample.stack_index).or_insert_with(|| {
                        let idx = sample.stack_index as usize;
                        if idx == 0 || idx >= dict.stack_table.len() {
                            return Vec::new();
                        }

                        let mut frames = Vec::new();
                        for &loc_idx in &dict.stack_table[idx].location_indices {
                            let loc_idx = loc_idx as usize;
                            if loc_idx < location_cache.len() {
                                frames.push(location_cache[loc_idx].clone());
                            }
                        }
                        frames.reverse(); // Standard pprof leaf-to-root reversal

                        let comm = resolve_thread_name(sample, dict);
                        let mut result = Vec::with_capacity(frames.len() + 1);
                        result.push(comm);
                        result.extend(frames);
                        result
                    });

                    if !stack.is_empty() {
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

                        flamegraph.add_stack(&stack, value);
                        sample_count += value as u64;
                    }
                }
            }
        }
    }

    let basenames = unknown_basenames(known, dict);
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

/// Try to symbolize a native frame via the local symbol store.
fn symbolize_native(
    store: &SymbolStore,
    location: &profiles::Location,
    dict: &profiles::ProfilesDictionary,
) -> Option<Vec<String>> {
    let resolved = store
        .lookup(
            store.file_id_for_basename(&resolve_mapping_filename(location, dict))?,
            location.address,
        )
        .ok()?;
    if resolved.is_empty() {
        return None;
    }
    Some(resolved.into_iter().map(|f| f.func).collect())
}

fn resolve_function_name(line: &profiles::Line, dict: &profiles::ProfilesDictionary) -> String {
    let func_idx = line.function_index as usize;
    if func_idx == 0 || func_idx >= dict.function_table.len() {
        return "[unknown]".to_string();
    }
    let func = &dict.function_table[func_idx];
    let name_idx = func.name_strindex as usize;
    if name_idx < dict.string_table.len() && !dict.string_table[name_idx].is_empty() {
        dict.string_table[name_idx].clone()
    } else {
        "[unknown]".to_string()
    }
}

fn resolve_unsymbolized_label(
    location: &profiles::Location,
    dict: &profiles::ProfilesDictionary,
) -> String {
    let mapping_name = resolve_mapping_filename(location, dict);
    format!("{}+0x{:016x}", mapping_name, location.address)
}

fn resolve_mapping_filename(
    location: &profiles::Location,
    dict: &profiles::ProfilesDictionary,
) -> String {
    let mapping_idx = location.mapping_index as usize;
    if mapping_idx == 0 || mapping_idx >= dict.mapping_table.len() {
        return "[unknown]".to_string();
    }
    let mapping = &dict.mapping_table[mapping_idx];
    let name_idx = mapping.filename_strindex as usize;
    if name_idx < dict.string_table.len() && !dict.string_table[name_idx].is_empty() {
        let full_path = &dict.string_table[name_idx];
        full_path
            .rsplit('/')
            .next()
            .unwrap_or(full_path)
            .to_string()
    } else {
        "[unknown]".to_string()
    }
}

fn resolve_frame_type(
    location: &profiles::Location,
    dict: &profiles::ProfilesDictionary,
) -> String {
    for &attr_idx in &location.attribute_indices {
        let attr_idx = attr_idx as usize;
        if attr_idx == 0 || attr_idx >= dict.attribute_table.len() {
            continue;
        }
        let attr = &dict.attribute_table[attr_idx];
        let key_idx = attr.key_strindex as usize;
        if key_idx >= dict.string_table.len() {
            continue;
        }
        if dict.string_table[key_idx] != "profile.frame.type" {
            continue;
        }
        if let Some(ref value) = attr.value {
            if let Some(common::any_value::Value::StringValue(ref s)) = value.value {
                return match s.as_str() {
                    "native" => "Native".to_string(),
                    "kernel" => "Kernel".to_string(),
                    "jvm" => "JVM".to_string(),
                    "cpython" => "Python".to_string(),
                    "php" | "phpjit" => "PHP".to_string(),
                    "ruby" => "Ruby".to_string(),
                    "perl" => "Perl".to_string(),
                    "v8js" => "JS".to_string(),
                    "dotnet" => ".NET".to_string(),
                    "beam" => "Beam".to_string(),
                    "go" => "Go".to_string(),
                    other => other.to_string(),
                };
            }
        }
    }
    String::from("Unknown")
}

fn format_with_tag(label: &str, tag: &str) -> String {
    if tag.is_empty() {
        label.to_string()
    } else {
        format!("{} [{}]", label, tag)
    }
}

fn resolve_thread_name(sample: &profiles::Sample, dict: &profiles::ProfilesDictionary) -> String {
    for &attr_idx in &sample.attribute_indices {
        let attr_idx = attr_idx as usize;
        if attr_idx == 0 || attr_idx >= dict.attribute_table.len() {
            continue;
        }
        let attr = &dict.attribute_table[attr_idx];
        let key_idx = attr.key_strindex as usize;
        if key_idx >= dict.string_table.len() {
            continue;
        }
        let key = &dict.string_table[key_idx];
        if key == "thread.name" {
            if let Some(ref value) = attr.value {
                if let Some(common::any_value::Value::StringValue(ref s)) = value.value {
                    if !s.is_empty() {
                        return s.clone();
                    }
                }
            }
        }
    }
    "[unknown]".to_string()
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
            let _tmp = tmp; // keep tempdir alive for the server's lifetime
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

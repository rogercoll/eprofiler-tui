use std::sync::mpsc;
use tonic::{Request, Response, Status};

use crate::flamegraph::FlameGraph;
use crate::proto::opentelemetry::proto::collector::profiles::v1development as collector;
use crate::proto::opentelemetry::proto::common::v1 as common;
use crate::proto::opentelemetry::proto::profiles::v1development as profiles;
use crate::tui::event::Event;

pub struct ProfilesServer {
    event_tx: mpsc::Sender<Event>,
}

impl ProfilesServer {
    pub fn new(event_tx: mpsc::Sender<Event>) -> Self {
        Self { event_tx }
    }
}

#[tonic::async_trait]
impl collector::profiles_service_server::ProfilesService for ProfilesServer {
    async fn export(
        &self,
        request: Request<collector::ExportProfilesServiceRequest>,
    ) -> Result<Response<collector::ExportProfilesServiceResponse>, Status> {
        let req = request.into_inner();
        let mut flamegraph = FlameGraph::new();
        let dict = req.dictionary.as_ref();

        let mut sample_count: u64 = 0;

        for resource_profiles in &req.resource_profiles {
            for scope_profiles in &resource_profiles.scope_profiles {
                for profile in &scope_profiles.profiles {
                    for sample in &profile.samples {
                        let stack = resolve_stack(sample, dict);
                        if stack.is_empty() {
                            continue;
                        }

                        let value = if !sample.values.is_empty() {
                            sample.values.iter().sum::<i64>().max(1)
                        } else if !sample.timestamps_unix_nano.is_empty() {
                            sample.timestamps_unix_nano.len() as i64
                        } else {
                            1
                        };

                        flamegraph.add_stack(&stack, value);
                        sample_count += 1;
                    }
                }
            }
        }

        flamegraph.root.sort_recursive();

        let _ = self.event_tx.send(Event::ProfileUpdate {
            flamegraph,
            samples: sample_count,
        });

        Ok(Response::new(collector::ExportProfilesServiceResponse {
            partial_success: None,
        }))
    }
}

fn resolve_stack(
    sample: &profiles::Sample,
    dict: Option<&profiles::ProfilesDictionary>,
) -> Vec<String> {
    let Some(dict) = dict else {
        return vec![];
    };

    let stack_idx = sample.stack_index as usize;
    if stack_idx == 0 || stack_idx >= dict.stack_table.len() {
        return vec![];
    }

    let stack = &dict.stack_table[stack_idx];
    let mut frames: Vec<String> = Vec::new();

    for &loc_idx in &stack.location_indices {
        let loc_idx = loc_idx as usize;
        if loc_idx == 0 || loc_idx >= dict.location_table.len() {
            continue;
        }
        let location = &dict.location_table[loc_idx];
        let frame_tag = resolve_frame_type(location, dict);

        if location.lines.is_empty() {
            let label = resolve_unsymbolized_label(location, dict);
            frames.push(format_with_tag(&label, &frame_tag));
        } else {
            for (i, line) in location.lines.iter().enumerate() {
                let func_name = resolve_function_name(line, dict);
                let inline_suffix = if i > 0 { " [Inline]" } else { "" };
                frames.push(format!(
                    "{}{}{}",
                    func_name,
                    if frame_tag.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", frame_tag)
                    },
                    inline_suffix,
                ));
            }
        }
    }

    frames.reverse();

    let comm = resolve_thread_name(sample, dict);
    let mut result = Vec::with_capacity(frames.len() + 1);
    result.push(comm);
    result.extend(frames);
    result
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
    String::new()
}

fn format_with_tag(label: &str, tag: &str) -> String {
    if tag.is_empty() {
        label.to_string()
    } else {
        format!("{} [{}]", label, tag)
    }
}

fn resolve_thread_name(
    sample: &profiles::Sample,
    dict: &profiles::ProfilesDictionary,
) -> String {
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
) -> Result<(), tonic::transport::Error> {
    let addr = addr.parse().expect("invalid gRPC listen address");
    let server = ProfilesServer::new(event_tx);

    tonic::transport::Server::builder()
        .add_service(
            collector::profiles_service_server::ProfilesServiceServer::new(server)
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                .send_compressed(tonic::codec::CompressionEncoding::Gzip),
        )
        .serve(addr)
        .await
}

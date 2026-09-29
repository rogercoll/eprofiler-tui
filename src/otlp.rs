//! Decoding of OTLP profile export requests into flamegraph data.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use eprofiler_proto::opentelemetry::proto::collector::profiles::v1development::ExportProfilesServiceRequest;
use eprofiler_proto::opentelemetry::proto::common::v1 as common;
use eprofiler_proto::opentelemetry::proto::profiles::v1development as profiles;

use crate::flamegraph::SampledStack;
use crate::frame::{Frame, FrameInfo, FrameKind, Runtime};
use crate::storage::SymbolStore;

/// Everything one export request contributes to the UI.
pub struct ProfileBatch {
    /// Each distinct stack in the request, once, with its summed weight.
    pub stacks: Vec<SampledStack>,
    /// Total weight across `stacks`.
    pub samples: u64,
    /// Sample timestamps per process, for the flamescope.
    pub timestamps: HashMap<Arc<str>, Vec<u64>>,
    /// Basenames of executables seen for the first time.
    pub new_mappings: Vec<String>,
}

/// Mapping paths already reported, shared across requests so each
/// executable is announced once.
#[derive(Default)]
pub struct KnownMappings(Mutex<HashSet<String>>);

impl KnownMappings {
    /// Record `paths` and return the basenames of those not seen before,
    /// skipping pseudo-mappings such as `[vdso]`.
    fn record<'a>(&self, paths: impl Iterator<Item = &'a str>) -> Vec<String> {
        let Ok(mut known) = self.0.lock() else {
            return Vec::new();
        };
        paths
            .filter(|path| known.insert((*path).to_owned()))
            .map(|path| path.rsplit('/').next().unwrap_or(path))
            .filter(|base| !base.is_empty() && !base.starts_with('['))
            .map(str::to_owned)
            .collect()
    }
}

/// Resolves one request's locations and samples into frames.
pub struct Decoder<'a> {
    dict: Dict<'a>,
    store: &'a SymbolStore,
}

impl<'a> Decoder<'a> {
    /// Decode `req`; `None` when it carries no dictionary.
    pub fn decode(
        req: &'a ExportProfilesServiceRequest,
        store: &'a SymbolStore,
        known: &KnownMappings,
    ) -> Option<ProfileBatch> {
        let decoder = Self {
            dict: Dict::new(req.dictionary.as_ref()?),
            store,
        };
        let locations = decoder.resolve_locations();

        let mut batch = ProfileBatch {
            stacks: Vec::new(),
            samples: 0,
            timestamps: HashMap::new(),
            new_mappings: known.record(decoder.dict.mapping_paths()),
        };
        // Position in `batch.stacks` per stack, process and thread; `None` for
        // samples that point at no stack. The dictionary is shared by every
        // process in the request and the thread comes from each sample, so
        // the same stack index can belong to several processes and threads.
        let mut slots: HashMap<(i32, &str, &str), Option<usize>> = HashMap::new();

        // The agent sends one resource per process.
        let samples = req.resource_profiles.iter().flat_map(|rp| {
            let process = decoder.process_label(rp, &locations);
            Self::samples_of(rp).map(move |sample| (process, sample))
        });
        for (process, sample) in samples {
            let thread = decoder.dict.thread_name(sample);
            let slot = *slots
                .entry((sample.stack_index, process, thread))
                .or_insert_with(|| {
                    let frames = decoder.stack(process, thread, sample, &locations);
                    (!frames.is_empty()).then(|| {
                        batch.stacks.push(SampledStack { frames, weight: 0 });
                        batch.stacks.len() - 1
                    })
                });
            let Some(slot) = slot else {
                continue;
            };
            let stack = &mut batch.stacks[slot];

            let value = if !sample.timestamps_unix_nano.is_empty() {
                batch
                    .timestamps
                    .entry(Arc::clone(&stack.frames[0].name))
                    .or_default()
                    .extend_from_slice(&sample.timestamps_unix_nano);
                sample.timestamps_unix_nano.len() as i64
            } else if !sample.values.is_empty() {
                sample.values.iter().sum::<i64>().max(1)
            } else {
                1
            };

            stack.weight += value;
            batch.samples += value as u64;
        }
        Some(batch)
    }

    fn samples_of(
        rp: &'a profiles::ResourceProfiles,
    ) -> impl Iterator<Item = &'a profiles::Sample> {
        rp.scope_profiles
            .iter()
            .flat_map(|sp| &sp.profiles)
            .flat_map(|p| &p.samples)
    }

    /// Label of a resource's process row: its `process.executable.name`.
    /// Without one it is `[kernel]` when every sampled stack is kernel code,
    /// as for kernel threads, which have no executable; otherwise `[unknown]`.
    fn process_label(&self, rp: &'a profiles::ResourceProfiles, locations: &[Frame]) -> &'a str {
        let executable = rp
            .resource
            .iter()
            .flat_map(|r| &r.attributes)
            .find(|kv| kv.key == "process.executable.name")
            .and_then(|kv| match kv.value.as_ref()?.value.as_ref()? {
                common::any_value::Value::StringValue(s) if !s.is_empty() => Some(s.as_str()),
                _ => None,
            });
        if let Some(name) = executable {
            return name;
        }
        let kernel_only = Self::samples_of(rp)
            .filter_map(|sample| self.stack_of(sample))
            .flat_map(|stack| &stack.location_indices)
            .all(|&idx| {
                locations
                    .get(idx as usize)
                    .is_some_and(|frame| frame.kind.runtime == Runtime::Kernel)
            });
        if kernel_only { "[kernel]" } else { "[unknown]" }
    }

    /// The stack `sample` points at; `None` for the null stack.
    fn stack_of(&self, sample: &profiles::Sample) -> Option<&'a profiles::Stack> {
        self.dict
            .d
            .stack_table
            .get(sample.stack_index as usize)
            .filter(|_| sample.stack_index > 0)
    }

    /// Root-first stack for `sample`: its process row, its thread row unless
    /// that just repeats the process name, then its frames. Empty when the
    /// sample points at no stack.
    fn stack(
        &self,
        process: &str,
        thread: &str,
        sample: &profiles::Sample,
        locations: &[Frame],
    ) -> Vec<Frame> {
        let Some(stack) = self.stack_of(sample) else {
            return Vec::new();
        };
        let frames = stack
            .location_indices
            .iter()
            .rev()
            .filter_map(|&idx| locations.get(idx as usize).cloned());
        let thread = (!Self::is_main_thread(thread, process)).then(|| Frame::thread(thread));
        std::iter::once(Frame::process(process))
            .chain(thread)
            .chain(frames)
            .collect()
    }

    /// Whether `thread` is named after `process`, as a main thread is. Linux
    /// truncates thread names to 15 bytes, so a full-length thread name that
    /// prefixes the executable name counts too.
    fn is_main_thread(thread: &str, process: &str) -> bool {
        const THREAD_NAME_MAX: usize = 15;
        thread == process || (thread.len() == THREAD_NAME_MAX && process.starts_with(thread))
    }

    /// Every entry of the location table as a labeled, classified frame.
    fn resolve_locations(&self) -> Vec<Frame> {
        self.dict
            .d
            .location_table
            .iter()
            .map(|location| self.resolve_location(location))
            .collect()
    }

    /// A location with several lines (or several symbolized inline levels)
    /// is an inline chain; it becomes one frame whose label joins the
    /// functions with ` / ` and whose kind is classified from the first one.
    fn resolve_location(&self, location: &profiles::Location) -> Frame {
        let runtime = self.dict.runtime(location);
        let mapping = self.dict.mapping_basename(location);

        let (names, file): (Vec<String>, Option<&str>) = if !location.lines.is_empty() {
            let names = location
                .lines
                .iter()
                .map(|l| self.dict.func_name(l).to_owned())
                .collect();
            (names, self.dict.func_file(&location.lines[0]))
        } else if runtime == Runtime::Native
            && let Some(names) = self.symbolize_native(location)
        {
            (names, None)
        } else {
            (vec![format!("{mapping}+0x{:016x}", location.address)], None)
        };

        let info = FrameInfo {
            function: &names[0],
            file,
            mapping: Some(mapping),
        };
        Frame {
            kind: FrameKind {
                runtime,
                origin: info.origin(runtime),
                inlined: names.len() > 1,
            },
            name: names.join(" / ").into(),
        }
    }

    /// Function names for an unsymbolized native address, outermost first,
    /// from symbols loaded on the executables tab.
    fn symbolize_native(&self, location: &profiles::Location) -> Option<Vec<String>> {
        let file_id = self
            .store
            .file_id_for_basename(self.dict.mapping_basename(location))?;
        let resolved = self.store.lookup(file_id, location.address).ok()?;
        (!resolved.is_empty()).then(|| resolved.into_iter().map(|f| f.func).collect())
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
        self.function(line)
            .and_then(|f| self.str(f.name_strindex))
            .unwrap_or("[unknown]")
    }

    fn func_file(&self, line: &profiles::Line) -> Option<&'a str> {
        self.function(line)
            .and_then(|f| self.str(f.filename_strindex))
    }

    fn function(&self, line: &profiles::Line) -> Option<&'a profiles::Function> {
        self.d
            .function_table
            .get(line.function_index as usize)
            .filter(|_| line.function_index > 0)
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

    fn runtime(&self, location: &profiles::Location) -> Runtime {
        self.find_attr_value(&location.attribute_indices, "profile.frame.type")
            .map_or(Runtime::Unknown, Runtime::from_otlp)
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

    /// Full paths of every mapping in the request.
    fn mapping_paths(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.d
            .mapping_table
            .iter()
            .skip(1)
            .filter_map(|m| self.str(m.filename_strindex))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{AnyValue, any_value};
    use eprofiler_proto::opentelemetry::proto::resource::v1::Resource;
    use profiles::{Function, KeyValueAndUnit, Line, Location, ProfilesDictionary};

    #[test]
    fn locations_resolve_to_classified_frames() {
        let attr = |key: i32, value: &str| KeyValueAndUnit {
            key_strindex: key,
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(value.into())),
            }),
            unit_strindex: 0,
        };
        let func = |name: i32| Function {
            name_strindex: name,
            ..Default::default()
        };
        let line = |function_index: i32| Line {
            function_index,
            ..Default::default()
        };
        let dict = ProfilesDictionary {
            string_table: vec![
                "".into(),
                "profile.frame.type".into(),
                "main.work".into(),
                "runtime.mallocgc".into(),
                "/usr/lib/x86_64-linux-gnu/libc.so.6".into(),
            ],
            attribute_table: vec![KeyValueAndUnit::default(), attr(1, "go"), attr(1, "native")],
            function_table: vec![Function::default(), func(2), func(3)],
            mapping_table: vec![
                profiles::Mapping::default(),
                profiles::Mapping {
                    filename_strindex: 4,
                    ..Default::default()
                },
            ],
            location_table: vec![
                Location::default(),
                Location {
                    lines: vec![line(1)],
                    attribute_indices: vec![1],
                    ..Default::default()
                },
                Location {
                    lines: vec![line(2), line(1)],
                    attribute_indices: vec![1],
                    ..Default::default()
                },
                Location {
                    mapping_index: 1,
                    address: 0x1234,
                    attribute_indices: vec![2],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        let store = SymbolStore::open(tmp.path()).unwrap();

        let decoder = Decoder {
            dict: Dict::new(&dict),
            store: &store,
        };
        let frames = decoder.resolve_locations();
        let kinds: Vec<_> = frames[1..]
            .iter()
            .map(|f| (&*f.name, f.kind.runtime, f.kind.origin, f.kind.inlined))
            .collect();
        use crate::frame::Origin::{Application as App, Runtime as Rt};
        assert_eq!(
            kinds,
            vec![
                ("main.work", Runtime::Go, App, false),
                ("runtime.mallocgc / main.work", Runtime::Go, Rt, true),
                ("libc.so.6+0x0000000000001234", Runtime::Native, Rt, false),
            ]
        );
    }

    #[test]
    fn new_mappings_are_reported_once() {
        let dict = ProfilesDictionary {
            string_table: vec!["".into(), "/usr/bin/app".into(), "[vdso]".into()],
            mapping_table: vec![
                profiles::Mapping::default(),
                profiles::Mapping {
                    filename_strindex: 1,
                    ..Default::default()
                },
                profiles::Mapping {
                    filename_strindex: 2,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let req = ExportProfilesServiceRequest {
            dictionary: Some(dict),
            ..Default::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        let store = SymbolStore::open(tmp.path()).unwrap();
        let known = KnownMappings::default();

        let first = Decoder::decode(&req, &store, &known).unwrap();
        assert_eq!(first.new_mappings, vec!["app".to_string()]);
        let second = Decoder::decode(&req, &store, &known).unwrap();
        assert!(second.new_mappings.is_empty());
    }

    #[test]
    fn requests_without_a_dictionary_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SymbolStore::open(tmp.path()).unwrap();
        let req = ExportProfilesServiceRequest::default();
        assert!(Decoder::decode(&req, &store, &KnownMappings::default()).is_none());
    }

    /// Functions `a` and `b`, one stack each (indices 1 and 2), and
    /// `thread.name` attributes for threads `t` (index 1) and `u` (index 2).
    fn two_stack_dictionary() -> ProfilesDictionary {
        let thread = |name: &str| KeyValueAndUnit {
            key_strindex: 1,
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(name.into())),
            }),
            unit_strindex: 0,
        };
        ProfilesDictionary {
            string_table: ["", "thread.name", "a", "b"].map(str::to_string).to_vec(),
            attribute_table: vec![KeyValueAndUnit::default(), thread("t"), thread("u")],
            function_table: [0, 2, 3]
                .map(|name_strindex| Function {
                    name_strindex,
                    ..Default::default()
                })
                .to_vec(),
            location_table: [0, 1, 2]
                .map(|function_index| Location {
                    lines: vec![Line {
                        function_index,
                        ..Default::default()
                    }],
                    ..Default::default()
                })
                .to_vec(),
            stack_table: [vec![], vec![1], vec![2]]
                .map(|location_indices| profiles::Stack { location_indices })
                .to_vec(),
            ..Default::default()
        }
    }

    /// `(stack index, thread attribute index, weight)`.
    type TestSample = (i32, i32, i64);

    /// Decode one resource per `(process, samples)` and return each emitted
    /// stack as frame names plus weight. A `None` process has no
    /// `process.executable.name` attribute.
    fn decode_processes(
        processes: &[(Option<&str>, &[TestSample])],
    ) -> (Vec<(Vec<String>, i64)>, u64) {
        decode_processes_with(two_stack_dictionary(), processes)
    }

    /// Like [`decode_processes`], with a custom dictionary.
    fn decode_processes_with(
        dict: ProfilesDictionary,
        processes: &[(Option<&str>, &[TestSample])],
    ) -> (Vec<(Vec<String>, i64)>, u64) {
        let resource = |(process, samples): &(Option<&str>, &[TestSample])| {
            let samples = samples
                .iter()
                .map(|&(stack_index, thread, weight)| profiles::Sample {
                    stack_index,
                    values: vec![weight],
                    attribute_indices: vec![thread],
                    ..Default::default()
                })
                .collect();
            let attributes = process
                .map(|name| common::KeyValue {
                    key: "process.executable.name".into(),
                    value: Some(AnyValue {
                        value: Some(any_value::Value::StringValue(name.into())),
                    }),
                    ..Default::default()
                })
                .into_iter()
                .collect();
            profiles::ResourceProfiles {
                resource: Some(Resource {
                    attributes,
                    ..Default::default()
                }),
                scope_profiles: vec![profiles::ScopeProfiles {
                    profiles: vec![profiles::Profile {
                        samples,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }
        };
        let req = ExportProfilesServiceRequest {
            dictionary: Some(dict),
            resource_profiles: processes.iter().map(resource).collect(),
        };
        let tmp = tempfile::tempdir().unwrap();
        let store = SymbolStore::open(tmp.path()).unwrap();
        let batch = Decoder::decode(&req, &store, &KnownMappings::default()).unwrap();
        let stacks = batch
            .stacks
            .iter()
            .map(|s| {
                (
                    s.frames.iter().map(|f| f.name.to_string()).collect(),
                    s.weight,
                )
            })
            .collect();
        (stacks, batch.samples)
    }

    /// All `samples` from one process named `app`.
    fn decode_samples(samples: &[TestSample]) -> (Vec<(Vec<String>, i64)>, u64) {
        decode_processes(&[(Some("app"), samples)])
    }

    fn stack(names: &[&str], weight: i64) -> (Vec<String>, i64) {
        (names.iter().map(|n| n.to_string()).collect(), weight)
    }

    #[test]
    fn samples_of_the_same_stack_collapse_into_one_weighted_entry() {
        // Stack 0 is the null stack and is skipped.
        let (stacks, samples) = decode_samples(&[(1, 1, 2), (2, 1, 1), (1, 1, 3), (0, 1, 9)]);
        assert_eq!(
            stacks,
            [stack(&["app", "t", "a"], 5), stack(&["app", "t", "b"], 1)]
        );
        assert_eq!(samples, 6);
    }

    #[test]
    fn threads_sharing_a_stack_stay_separate() {
        let (stacks, samples) = decode_samples(&[(1, 1, 2), (1, 2, 7), (1, 1, 1)]);
        assert_eq!(
            stacks,
            [stack(&["app", "t", "a"], 3), stack(&["app", "u", "a"], 7)]
        );
        assert_eq!(samples, 10);
    }

    #[test]
    fn processes_sharing_a_stack_and_thread_name_stay_separate() {
        let (stacks, _) =
            decode_processes(&[(Some("api"), &[(1, 1, 2)]), (Some("worker"), &[(1, 1, 5)])]);
        assert_eq!(
            stacks,
            [
                stack(&["api", "t", "a"], 2),
                stack(&["worker", "t", "a"], 5)
            ]
        );
    }

    #[test]
    fn processes_without_an_executable_name_are_unknown() {
        let (stacks, _) = decode_processes(&[(None, &[(1, 1, 1)])]);
        assert_eq!(stacks, [stack(&["[unknown]", "t", "a"], 1)]);
    }

    #[test]
    fn main_threads_named_after_their_process_get_no_thread_row() {
        let mut dict = two_stack_dictionary();
        for name in ["app", "opentelemetry-c"] {
            dict.attribute_table.push(KeyValueAndUnit {
                key_strindex: 1,
                value: Some(AnyValue {
                    value: Some(any_value::Value::StringValue(name.into())),
                }),
                unit_strindex: 0,
            });
        }
        // Attribute 3 is thread "app", attribute 4 is the 15-byte truncation
        // of "opentelemetry-collector"; attribute 1 is an ordinary thread "t".
        let (stacks, _) = decode_processes_with(
            dict,
            &[
                (Some("app"), &[(1, 3, 1), (1, 1, 2)]),
                (Some("opentelemetry-collector"), &[(2, 4, 3)]),
            ],
        );
        assert_eq!(
            stacks,
            [
                stack(&["app", "a"], 1),
                stack(&["app", "t", "a"], 2),
                stack(&["opentelemetry-collector", "b"], 3),
            ]
        );
    }

    #[test]
    fn only_full_length_thread_names_count_as_truncated() {
        assert!(Decoder::is_main_thread("app", "app"));
        assert!(Decoder::is_main_thread(
            "opentelemetry-c",
            "opentelemetry-collector"
        ));
        assert!(!Decoder::is_main_thread("open", "opentelemetry-collector"));
        assert!(!Decoder::is_main_thread("tokio-rt-worker", "eprofiler-tui"));
    }

    #[test]
    fn nameless_kernel_only_processes_are_labeled_kernel() {
        // Stack 1's only frame is kernel code; stack 2's is not.
        let mut dict = two_stack_dictionary();
        dict.string_table
            .extend(["profile.frame.type", "kernel"].map(str::to_string));
        dict.attribute_table.push(KeyValueAndUnit {
            key_strindex: dict.string_table.len() as i32 - 2,
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue("kernel".into())),
            }),
            unit_strindex: 0,
        });
        dict.location_table[1].attribute_indices = vec![dict.attribute_table.len() as i32 - 1];

        let (stacks, _) = decode_processes_with(
            dict,
            &[(None, &[(1, 1, 4)]), (None, &[(1, 2, 1), (2, 2, 1)])],
        );
        assert_eq!(
            stacks,
            [
                stack(&["[kernel]", "t", "a"], 4),
                stack(&["[unknown]", "u", "a"], 1),
                stack(&["[unknown]", "u", "b"], 1),
            ]
        );
    }
}

//! Decoding of OTLP profile export requests into flamegraph data.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use eprofiler_proto::opentelemetry::proto::collector::profiles::v1development::ExportProfilesServiceRequest;
use eprofiler_proto::opentelemetry::proto::common::v1 as common;
use eprofiler_proto::opentelemetry::proto::profiles::v1development as profiles;

use crate::flamegraph::FlameGraph;
use crate::frame::{Frame, FrameInfo, FrameKind, Runtime};
use crate::storage::SymbolStore;

/// Everything one export request contributes to the UI.
pub struct ProfileBatch {
    pub flamegraph: FlameGraph,
    /// Total sample weight added to `flamegraph`.
    pub samples: u64,
    /// Sample timestamps per thread, for the flamescope.
    pub timestamps: HashMap<String, Vec<u64>>,
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
            flamegraph: FlameGraph::new(),
            samples: 0,
            timestamps: HashMap::new(),
            new_mappings: known.record(decoder.dict.mapping_paths()),
        };
        let mut stacks: HashMap<i32, Vec<Frame>> = HashMap::new();

        let samples = req
            .resource_profiles
            .iter()
            .flat_map(|rp| &rp.scope_profiles)
            .flat_map(|sp| &sp.profiles)
            .flat_map(|p| &p.samples);
        for sample in samples {
            let stack = stacks
                .entry(sample.stack_index)
                .or_insert_with(|| decoder.stack(sample, &locations));
            let Some(thread) = stack.first() else {
                continue;
            };

            let value = if !sample.timestamps_unix_nano.is_empty() {
                batch
                    .timestamps
                    .entry(thread.name.clone())
                    .or_default()
                    .extend_from_slice(&sample.timestamps_unix_nano);
                sample.timestamps_unix_nano.len() as i64
            } else if !sample.values.is_empty() {
                sample.values.iter().sum::<i64>().max(1)
            } else {
                1
            };

            batch.flamegraph.add_stack(stack, value);
            batch.samples += value as u64;
        }
        Some(batch)
    }

    /// Root-first stack for `sample`: its thread row, then its frames.
    /// Empty when the sample points at no stack.
    fn stack(&self, sample: &profiles::Sample, locations: &[Frame]) -> Vec<Frame> {
        let Some(stack) = self
            .dict
            .d
            .stack_table
            .get(sample.stack_index as usize)
            .filter(|_| sample.stack_index > 0)
        else {
            return Vec::new();
        };
        let frames = stack
            .location_indices
            .iter()
            .rev()
            .filter_map(|&idx| locations.get(idx as usize).cloned());
        std::iter::once(Frame::thread(self.dict.thread_name(sample)))
            .chain(frames)
            .collect()
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
            name: names.join(" / "),
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
            .map(|f| {
                (
                    f.name.as_str(),
                    f.kind.runtime,
                    f.kind.origin,
                    f.kind.inlined,
                )
            })
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
}

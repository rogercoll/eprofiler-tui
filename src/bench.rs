//! Flamegraph ingest and render benchmark.
//!
//! ```sh
//! cargo test --release bench -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Feeds synthetic OTLP requests through the real decode and merge path, then
//! renders the flamegraph tab. Reports wall time (best of [`RUNS`]) and heap
//! allocations, counted by a test-only global allocator. Allocation counts are
//! deterministic, so they compare cleanly across branches; run with one test
//! thread so no other test allocates concurrently.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use eprofiler_proto::opentelemetry::proto::collector::profiles::v1development::ExportProfilesServiceRequest;
use eprofiler_proto::opentelemetry::proto::common::v1::{AnyValue, any_value};
use eprofiler_proto::opentelemetry::proto::profiles::v1development::{
    Function, KeyValueAndUnit, Line, Location, Profile, ProfilesDictionary, ResourceProfiles,
    Sample, ScopeProfiles, Stack,
};
use ratatui::{Terminal, backend::TestBackend};

use crate::flamegraph::FlameNode;
use crate::otlp::{Decoder, KnownMappings};
use crate::storage::SymbolStore;
use crate::tui::event::Event;
use crate::tui::state::State;
use crate::tui::view::Screen;

const RUNS: usize = 3;
const THREADS: u64 = 8;
/// Distinct function names.
const VOCABULARY: u64 = 3_000;
/// Distinct call stacks the samples are drawn from.
const STACK_POOL: usize = 20_000;
/// Callees per frame when growing the synthetic call tree.
const FANOUT: u64 = 3;
const DEPTH: std::ops::Range<u64> = 8..48;
const REQUESTS: usize = 120;
const SAMPLES_PER_REQUEST: usize = 800;
const RENDER_FRAMES: usize = 100;
const SCREEN: (u16, u16) = (200, 60);

#[test]
#[ignore = "benchmark: cargo test --release bench -- --ignored --nocapture --test-threads=1"]
fn flamegraph_ingest_and_render() {
    let requests = Workload::new().requests();
    let reports: Vec<Report> = (0..RUNS).map(|_| Report::measure(&requests)).collect();
    let best = |f: fn(&Report) -> Duration| reports.iter().map(f).min().unwrap();
    let r = &reports[0];

    println!();
    println!("flamegraph benchmark ({RUNS} runs, best time; allocations from run 1)");
    println!(
        "  workload: {REQUESTS} requests x {SAMPLES_PER_REQUEST} samples, {} tree nodes",
        r.nodes
    );
    println!(
        "  {:<22} {:>10} {:>14} {:>12}",
        "phase", "time", "allocations", "alloc MiB"
    );
    for (name, time, heap) in [
        ("decode", best(|r| r.decode), r.decode_heap),
        ("merge into UI tree", best(|r| r.merge), r.merge_heap),
        (
            "render (per frame)",
            best(|r| r.render) / RENDER_FRAMES as u32,
            r.render_heap / RENDER_FRAMES as u64,
        ),
    ] {
        println!(
            "  {name:<22} {:>8.2}ms {:>14} {:>12.1}",
            time.as_secs_f64() * 1e3,
            heap.count,
            heap.bytes as f64 / (1024.0 * 1024.0)
        );
    }
}

/// One full run: ingest every request, then render.
struct Report {
    nodes: usize,
    decode: Duration,
    merge: Duration,
    render: Duration,
    decode_heap: Heap,
    merge_heap: Heap,
    render_heap: Heap,
}

impl Report {
    fn measure(requests: &[ExportProfilesServiceRequest]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let store = SymbolStore::open(tmp.path()).unwrap();
        let known = KnownMappings::default();
        let mut state = State::new("bench".into(), vec![]);
        let mut report = Self {
            nodes: 0,
            decode: Duration::ZERO,
            merge: Duration::ZERO,
            render: Duration::ZERO,
            decode_heap: Heap::default(),
            merge_heap: Heap::default(),
            render_heap: Heap::default(),
        };

        for req in requests {
            let (batch, time, heap) = measure(|| Decoder::decode(req, &store, &known).unwrap());
            report.decode += time;
            report.decode_heap += heap;
            let event = Event::ProfileUpdate {
                stacks: batch.stacks,
                samples: batch.samples,
                timestamps: batch.timestamps,
            };
            let ((), time, heap) = measure(|| {
                state.handle_event(event);
            });
            report.merge += time;
            report.merge_heap += heap;
        }
        report.nodes = count_nodes(&state.fg.graph.root);
        assert_heaviest_first(&state.fg.graph.root);

        let mut term = Terminal::new(TestBackend::new(SCREEN.0, SCREEN.1)).unwrap();
        let ((), time, heap) = measure(|| {
            for _ in 0..RENDER_FRAMES {
                term.draw(|f| f.render_stateful_widget(Screen, f.area(), &mut state))
                    .unwrap();
            }
        });
        report.render = time;
        report.render_heap = heap;
        report
    }
}

fn count_nodes(node: &FlameNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

/// Guards against a faster merge that stops keeping siblings in order.
fn assert_heaviest_first(node: &FlameNode) {
    let totals = node.children.iter().map(|c| c.total_value);
    assert!(totals.clone().zip(totals.skip(1)).all(|(a, b)| a >= b));
    node.children.iter().for_each(assert_heaviest_first);
}

/// Run `f`, returning its result, elapsed time and heap allocations.
fn measure<T>(f: impl FnOnce() -> T) -> (T, Duration, Heap) {
    let before = Heap::now();
    let start = Instant::now();
    let out = f();
    let time = start.elapsed();
    (out, time, Heap::now() - before)
}

/// Deterministic synthetic profiles: a call tree grown by random walks, a
/// fixed pool of stacks from it, and skewed sampling so a few stacks are hot.
struct Workload {
    rng: Rng,
    /// Root-first function indices, plus the thread each stack runs on.
    stacks: Vec<(u64, Vec<u64>)>,
}

impl Workload {
    fn new() -> Self {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let stacks = (0..STACK_POOL)
            .map(|_| {
                let thread = rng.below(THREADS);
                let depth = DEPTH.start + rng.below(DEPTH.end - DEPTH.start);
                let mut node = thread;
                let frames = (0..depth)
                    .map(|_| {
                        node = Rng::mix(node ^ rng.below(FANOUT));
                        node % VOCABULARY
                    })
                    .collect();
                (thread, frames)
            })
            .collect();
        Self { rng, stacks }
    }

    fn requests(mut self) -> Vec<ExportProfilesServiceRequest> {
        (0..REQUESTS).map(|_| self.request()).collect()
    }

    /// One request with its own dictionary, as the eBPF agent sends them.
    fn request(&mut self) -> ExportProfilesServiceRequest {
        let mut dict = Dictionary::default();
        let samples = (0..SAMPLES_PER_REQUEST)
            .map(|_| {
                // Squaring skews picks toward the front of the pool.
                let r = self.rng.below(STACK_POOL as u64) as f64 / STACK_POOL as f64;
                let (thread, frames) = &self.stacks[(r * r * STACK_POOL as f64) as usize];
                Sample {
                    stack_index: dict.stack(frames),
                    values: vec![1],
                    attribute_indices: vec![dict.thread(*thread)],
                    ..Default::default()
                }
            })
            .collect();
        ExportProfilesServiceRequest {
            dictionary: Some(dict.finish()),
            resource_profiles: vec![ResourceProfiles {
                scope_profiles: vec![ScopeProfiles {
                    profiles: vec![Profile {
                        samples,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }
}

/// Builds a request's interned tables, handing out indices on first use.
struct Dictionary {
    table: ProfilesDictionary,
    strings: HashMap<String, i32>,
    locations: HashMap<u64, i32>,
    stacks: HashMap<Vec<u64>, i32>,
    threads: HashMap<u64, i32>,
}

impl Default for Dictionary {
    fn default() -> Self {
        Self {
            // Index 0 of every table is the null entry.
            table: ProfilesDictionary {
                string_table: vec![String::new()],
                attribute_table: vec![KeyValueAndUnit::default()],
                function_table: vec![Function::default()],
                location_table: vec![Location::default()],
                stack_table: vec![Stack::default()],
                ..Default::default()
            },
            strings: HashMap::new(),
            locations: HashMap::new(),
            stacks: HashMap::new(),
            threads: HashMap::new(),
        }
    }
}

impl Dictionary {
    fn string(&mut self, s: String) -> i32 {
        let table = &mut self.table.string_table;
        *self.strings.entry(s.clone()).or_insert_with(|| {
            table.push(s);
            table.len() as i32 - 1
        })
    }

    fn location(&mut self, function: u64) -> i32 {
        if let Some(&idx) = self.locations.get(&function) {
            return idx;
        }
        let name = format!(
            "app::module_{:03}::Type{}::method_{function}",
            function % 97,
            function % 31
        );
        let name_strindex = self.string(name);
        self.table.function_table.push(Function {
            name_strindex,
            ..Default::default()
        });
        self.table.location_table.push(Location {
            lines: vec![Line {
                function_index: self.table.function_table.len() as i32 - 1,
                ..Default::default()
            }],
            ..Default::default()
        });
        let idx = self.table.location_table.len() as i32 - 1;
        self.locations.insert(function, idx);
        idx
    }

    /// `frames` is root-first; OTLP lists locations leaf-first.
    fn stack(&mut self, frames: &[u64]) -> i32 {
        if let Some(&idx) = self.stacks.get(frames) {
            return idx;
        }
        let location_indices = frames.iter().rev().map(|&f| self.location(f)).collect();
        self.table.stack_table.push(Stack { location_indices });
        let idx = self.table.stack_table.len() as i32 - 1;
        self.stacks.insert(frames.to_vec(), idx);
        idx
    }

    fn thread(&mut self, thread: u64) -> i32 {
        if let Some(&idx) = self.threads.get(&thread) {
            return idx;
        }
        let key_strindex = self.string("thread.name".into());
        self.table.attribute_table.push(KeyValueAndUnit {
            key_strindex,
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(format!("worker-{thread}"))),
            }),
            unit_strindex: 0,
        });
        let idx = self.table.attribute_table.len() as i32 - 1;
        self.threads.insert(thread, idx);
        idx
    }

    fn finish(self) -> ProfilesDictionary {
        self.table
    }
}

/// xorshift64*: fast, deterministic, good enough for synthetic data.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) % n
    }

    /// Scramble a call-tree node key into its child's key.
    fn mix(mut x: u64) -> u64 {
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^ (x >> 31)
    }
}

/// Heap allocation totals since process start.
#[derive(Default, Clone, Copy)]
struct Heap {
    count: u64,
    bytes: u64,
}

impl Heap {
    fn now() -> Self {
        Self {
            count: ALLOCATIONS.load(Relaxed),
            bytes: ALLOCATED_BYTES.load(Relaxed),
        }
    }
}

impl std::ops::Sub for Heap {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            count: self.count - rhs.count,
            bytes: self.bytes - rhs.bytes,
        }
    }
}

impl std::ops::AddAssign for Heap {
    fn add_assign(&mut self, rhs: Self) {
        self.count += rhs.count;
        self.bytes += rhs.bytes;
    }
}

impl std::ops::Div<u64> for Heap {
    type Output = Self;
    fn div(self, n: u64) -> Self {
        Self {
            count: self.count / n,
            bytes: self.bytes / n,
        }
    }
}

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

/// Counts every allocation, then defers to the system allocator. Only
/// compiled into the test binary.
struct CountingAllocator;

// SAFETY: forwards every call unchanged to `System`, only adding counters.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Relaxed);
        ALLOCATED_BYTES.fetch_add(layout.size() as u64, Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Relaxed);
        ALLOCATED_BYTES.fetch_add(new_size as u64, Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

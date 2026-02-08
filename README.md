# eprofiler-tui

A terminal-based flamegraph viewer that receives profiling data via an OTLP gRPC endpoint and renders a live, interactive flamegraph in the terminal.

## Features

- OTLP gRPC profiles receiver (listens on `0.0.0.0:4317`)
- Live icicle-style flamegraph with hot/warm color scheme
- Frame type annotations (`[Native]`, `[Kernel]`, `[JVM]`, etc.)
- Thread/process grouping via `thread.name` sample attribute
- Keyboard-driven navigation and zoom
- Thread search (`/`) with fuzzy filtering

## Building

Requires Rust 2024 edition and protobuf definitions from the `opentelemetry-proto` submodule.

```
git submodule update --init
cargo build --release
```

## Usage

```
cargo run --release
```

The TUI will start and listen for OTLP profile data on port 4317. Point your OpenTelemetry profiling agent (e.g. `opentelemetry-ebpf-profiler`) at `localhost:4317`.

## Keybindings

| Key | Action |
|-----|--------|
| `j` / `↓` | Move deeper into the stack |
| `k` / `↑` | Move shallower |
| `h` / `←` | Previous sibling frame |
| `l` / `→` | Next sibling frame |
| `Enter` | Zoom into selected frame |
| `Esc` | Zoom out one level |
| `/` | Search/filter by thread name |
| `r` | Reset view |
| `c` | Clear all data |
| `q` | Quit |

//! What a stack frame *is*: which runtime produced it and whether it belongs
//! to the application or to the runtime/system underneath it.
//!
//! Everything here is derived from the frame itself (its type attribute,
//! name, source file and mapping), never from its position in a graph, so it
//! is stable across updates and zoom levels.

/// Runtime that produced a frame, from the OTLP `profile.frame.type` attribute.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Runtime {
    Native,
    Kernel,
    Jvm,
    Go,
    Python,
    Js,
    Ruby,
    Php,
    Dotnet,
    Beam,
    Perl,
    #[default]
    Unknown,
    /// Synthetic rows that are not code: the graph root and per-thread rows.
    Thread,
}

impl Runtime {
    /// Every runtime, in legend order.
    pub const ALL: [Runtime; 13] = [
        Self::Native,
        Self::Kernel,
        Self::Jvm,
        Self::Go,
        Self::Python,
        Self::Js,
        Self::Ruby,
        Self::Php,
        Self::Dotnet,
        Self::Beam,
        Self::Perl,
        Self::Unknown,
        Self::Thread,
    ];

    pub fn from_otlp(frame_type: &str) -> Self {
        match frame_type {
            "native" => Self::Native,
            "kernel" => Self::Kernel,
            "jvm" => Self::Jvm,
            "go" => Self::Go,
            "cpython" => Self::Python,
            "v8js" => Self::Js,
            "ruby" => Self::Ruby,
            "php" | "phpjit" => Self::Php,
            "dotnet" => Self::Dotnet,
            "beam" => Self::Beam,
            "perl" => Self::Perl,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "Native",
            Self::Kernel => "Kernel",
            Self::Jvm => "JVM",
            Self::Go => "Go",
            Self::Python => "Python",
            Self::Js => "JS",
            Self::Ruby => "Ruby",
            Self::Php => "PHP",
            Self::Dotnet => ".NET",
            Self::Beam => "Beam",
            Self::Perl => "Perl",
            Self::Unknown => "Unknown",
            Self::Thread => "Thread",
        }
    }
}

/// Whether a frame is the profiled program's own code or the runtime,
/// standard library or system beneath it. Third-party libraries count as
/// application.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    #[default]
    Application,
    Runtime,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Self::Application => "application",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameKind {
    pub runtime: Runtime,
    pub origin: Origin,
    /// The frame's label is an inline chain rather than a single function.
    pub inlined: bool,
}

impl FrameKind {
    pub const THREAD: Self = Self {
        runtime: Runtime::Thread,
        origin: Origin::Application,
        inlined: false,
    };
}

/// One element of a stack: a label plus what kind of code it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub name: String,
    pub kind: FrameKind,
}

impl Frame {
    pub fn thread(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: FrameKind::THREAD,
        }
    }
}

impl From<&str> for Frame {
    fn from(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            kind: FrameKind::default(),
        }
    }
}

/// What is known about a frame when it is decoded.
#[derive(Debug, Default, Clone, Copy)]
pub struct FrameInfo<'a> {
    pub function: &'a str,
    /// Source file of the function, when the profiler reports one.
    pub file: Option<&'a str>,
    /// Basename of the executable or library the frame's address maps to.
    pub mapping: Option<&'a str>,
}

/// Decide whether a frame belongs to the application or to the runtime.
/// Runtimes without a rule default to application.
pub fn classify(runtime: Runtime, info: FrameInfo) -> Origin {
    let is_runtime = match runtime {
        Runtime::Kernel => true,
        Runtime::Native => native_is_runtime(info),
        Runtime::Go => go_is_stdlib(info.function),
        Runtime::Jvm => has_prefix(
            info.function,
            &[
                "java.", "javax.", "jdk.", "sun.", "com.sun.", "kotlin.", "scala.",
            ],
        ),
        Runtime::Python => info.file.is_some_and(|f| {
            f.starts_with("<frozen ") || (f.contains("/lib/python") && !is_third_party_dir(f))
        }),
        Runtime::Js => info
            .file
            .is_some_and(|f| f.starts_with("node:") || f.starts_with("internal/")),
        Runtime::Ruby => info.file.is_some_and(|f| {
            f.starts_with("<internal:") || (f.contains("/lib/ruby/") && !f.contains("/gems/"))
        }),
        Runtime::Dotnet => has_prefix(info.function, &["System.", "Microsoft."]),
        _ => false,
    };
    if is_runtime {
        Origin::Runtime
    } else {
        Origin::Application
    }
}

fn has_prefix(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.starts_with(p))
}

fn is_third_party_dir(path: &str) -> bool {
    path.contains("/site-packages/") || path.contains("/dist-packages/")
}

/// System libraries, the dynamic loader, the vDSO, interpreter/VM binaries,
/// and Rust/C++ standard library symbols.
fn native_is_runtime(info: FrameInfo) -> bool {
    let system_mapping = info.mapping.is_some_and(|m| {
        (m.starts_with("lib") && m.contains(".so"))
            || m.starts_with("ld-")
            || m.contains("vdso")
            || has_prefix(
                m,
                &[
                    "python", "node", "java", "ruby", "php", "perl", "dotnet", "beam",
                ],
            )
    });
    let std_symbol = has_prefix(
        info.function,
        &[
            "std::",
            "core::",
            "alloc::",
            "<std::",
            "<core::",
            "<alloc::",
            "__gnu_cxx::",
        ],
    );
    system_mapping || std_symbol
}

/// Go stdlib import paths have no dot in their first element
/// (`net/http`, `runtime`), unlike module paths (`github.com/...`).
fn go_is_stdlib(function: &str) -> bool {
    let name = function.split('[').next().unwrap_or(function);
    let seg_start = name.rfind('/').map_or(0, |i| i + 1);
    let Some(dot) = name[seg_start..].find('.') else {
        return false;
    };
    let package = &name[..seg_start + dot];
    let first = package.split('/').next().unwrap_or(package);
    package != "main" && !first.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(
        runtime: Runtime,
        function: &str,
        file: Option<&str>,
        mapping: Option<&str>,
    ) -> Origin {
        classify(
            runtime,
            FrameInfo {
                function,
                file,
                mapping,
            },
        )
    }

    #[test]
    fn classification_table() {
        use Origin::{Application as App, Runtime as Rt};
        // (runtime, function, file, mapping, expected)
        type Case = (
            Runtime,
            &'static str,
            Option<&'static str>,
            Option<&'static str>,
            Origin,
        );
        let cases: &[Case] = &[
            (Runtime::Kernel, "do_syscall_64", None, None, Rt),
            (Runtime::Native, "do_work", None, Some("myapp"), App),
            (Runtime::Native, "malloc", None, Some("libc.so.6"), Rt),
            (
                Runtime::Native,
                "_dl_start",
                None,
                Some("ld-linux-x86-64.so.2"),
                Rt,
            ),
            (
                Runtime::Native,
                "_PyEval_EvalFrameDefault",
                None,
                Some("python3.12"),
                Rt,
            ),
            (
                Runtime::Native,
                "std::rt::lang_start",
                None,
                Some("myapp"),
                Rt,
            ),
            (
                Runtime::Native,
                "<alloc::vec::Vec<T> as Drop>::drop",
                None,
                Some("myapp"),
                Rt,
            ),
            (Runtime::Go, "main.main", None, None, App),
            (Runtime::Go, "main.main.func1", None, None, App),
            (Runtime::Go, "runtime.mallocgc", None, None, Rt),
            (Runtime::Go, "net/http.(*conn).serve", None, None, Rt),
            (Runtime::Go, "internal/poll.(*FD).Read", None, None, Rt),
            (
                Runtime::Go,
                "github.com/foo/bar.(*Baz).Run",
                None,
                None,
                App,
            ),
            (Runtime::Go, "slices.Sort[go.shape.int]", None, None, Rt),
            (Runtime::Jvm, "java.lang.Thread.run", None, None, Rt),
            (Runtime::Jvm, "com.example.Service.handle", None, None, App),
            (
                Runtime::Python,
                "json.loads",
                Some("/usr/lib/python3.12/json/__init__.py"),
                None,
                Rt,
            ),
            (
                Runtime::Python,
                "_find_and_load",
                Some("<frozen importlib._bootstrap>"),
                None,
                Rt,
            ),
            (
                Runtime::Python,
                "get",
                Some("/usr/lib/python3.12/site-packages/requests/api.py"),
                None,
                App,
            ),
            (Runtime::Python, "main", Some("/srv/app/main.py"), None, App),
            (Runtime::Python, "main", None, None, App),
            (
                Runtime::Js,
                "listOnTimeout",
                Some("node:internal/timers"),
                None,
                Rt,
            ),
            (Runtime::Js, "handler", Some("/srv/app/index.js"), None, App),
            (Runtime::Ruby, "each", Some("<internal:array>"), None, Rt),
            (
                Runtime::Ruby,
                "call",
                Some("/usr/lib/ruby/gems/3.3/rack.rb"),
                None,
                App,
            ),
            (
                Runtime::Dotnet,
                "System.Threading.Thread.Start",
                None,
                None,
                Rt,
            ),
            (Runtime::Php, "anything", None, None, App),
            (Runtime::Unknown, "[unknown]", None, None, App),
        ];
        for &(runtime, function, file, mapping, expected) in cases {
            assert_eq!(
                origin(runtime, function, file, mapping),
                expected,
                "{runtime:?} {function} file={file:?} mapping={mapping:?}"
            );
        }
    }

    #[test]
    fn otlp_frame_types_round_trip_through_labels() {
        assert_eq!(Runtime::from_otlp("cpython"), Runtime::Python);
        assert_eq!(Runtime::from_otlp("phpjit"), Runtime::Php);
        assert_eq!(Runtime::from_otlp("something-new"), Runtime::Unknown);
        assert_eq!(Runtime::Dotnet.label(), ".NET");
    }
}

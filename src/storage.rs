use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use fjall::{Keyspace, PartitionHandle};
use symblib::VirtAddr;
pub use symblib::fileid::FileId;
use zerocopy::byteorder::{BigEndian, U16, U32, U64, U128};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

use crate::symbolizer::{FileSym, SymRange};

const NONE_REF: u32 = u32::MAX;

/// Big-endian key for the ranges LSM partition.
///
/// Byte-level lexicographic ordering matches semantic ordering, so a
/// reverse-range scan from `(file_id, addr, u16::MAX)` efficiently locates
/// the nearest range whose `va_start <= addr`.
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct RangeKey {
    file_id: U128<BigEndian>,
    va_start: U64<BigEndian>,
    depth: U16<BigEndian>,
}

impl RangeKey {
    fn new(file_id: u128, va_start: u64, depth: u16) -> Self {
        Self {
            file_id: U128::new(file_id),
            va_start: U64::new(va_start),
            depth: U16::new(depth),
        }
    }

    fn va_start(&self) -> u64 {
        self.va_start.get()
    }

    fn depth(&self) -> u16 {
        self.depth.get()
    }
}

/// Fixed-size value stored alongside each [`RangeKey`].
///
/// Optional fields use sentinels (`NONE_REF` / `0`) to avoid variable-size
/// encoding while staying zerocopy-friendly.
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct RangeValue {
    length: U32<BigEndian>,
    func_ref: U32<BigEndian>,
    file_ref: U32<BigEndian>,
    call_file_ref: U32<BigEndian>,
    call_line: U32<BigEndian>,
}

impl RangeValue {
    fn from_range(r: &SymRange) -> Self {
        Self {
            length: U32::new(r.length),
            func_ref: U32::new(r.func.0),
            file_ref: U32::new(r.file.map_or(NONE_REF, |s| s.0)),
            call_file_ref: U32::new(r.call_file.map_or(NONE_REF, |s| s.0)),
            call_line: U32::new(r.call_line.unwrap_or(0)),
        }
    }

    fn length(&self) -> u32 {
        self.length.get()
    }

    fn func_ref(&self) -> u32 {
        self.func_ref.get()
    }
}

/// Key for the per-file interned string table.
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct StringKey {
    file_id: U128<BigEndian>,
    idx: U32<BigEndian>,
}

impl StringKey {
    fn new(file_id: u128, idx: u32) -> Self {
        Self {
            file_id: U128::new(file_id),
            idx: U32::new(idx),
        }
    }
}

/// Resolved symbol information for a single inline depth level.
pub struct ResolvedFrame {
    pub func: String,
    pub depth: u16,
}

/// Metadata for a stored executable.
#[derive(Clone)]
pub struct ExecutableInfo {
    pub file_id: FileId,
    pub file_name: String,
    pub num_ranges: u32,
}

/// Persistent symbol store backed by fjall (LSM-tree).
///
/// Three partitions:
///   - **ranges**: `RangeKey -> RangeValue` (fixed 26-byte key, 20-byte value)
///   - **strings**: `StringKey -> raw UTF-8` (fixed 20-byte key, variable value)
///   - **files**: `U128<BE> -> num_ranges(4) + filename` (executable metadata)
pub struct SymbolStore {
    keyspace: Keyspace,
    ranges: PartitionHandle,
    strings: PartitionHandle,
    files: PartitionHandle,
    basename_index: RwLock<HashMap<String, FileId>>,
}

impl SymbolStore {
    pub fn open(path: impl AsRef<Path>) -> crate::Result<Self> {
        let keyspace = fjall::Config::new(path).open()?;
        let ranges = keyspace.open_partition("ranges", Default::default())?;
        let strings = keyspace.open_partition("strings", Default::default())?;
        let files = keyspace.open_partition("files", Default::default())?;

        let store = Self {
            keyspace,
            ranges,
            strings,
            files,
            basename_index: RwLock::new(HashMap::new()),
        };

        // Rebuild in-memory basename index from persisted metadata.
        for info in store.list_files()? {
            let basename = basename_of(&info.file_name);
            store
                .basename_index
                .write()
                .unwrap()
                .insert(basename, info.file_id);
        }

        Ok(store)
    }

    /// Atomically persist all ranges, interned strings, and file metadata.
    pub fn store_file_symbols(&self, file_sym: &FileSym, path: &Path) -> crate::Result<()> {
        let fid: u128 = file_sym.file_id.into();
        let mut batch = self.keyspace.batch();

        for (idx, s) in file_sym.strings.iter().enumerate() {
            batch.insert(
                &self.strings,
                StringKey::new(fid, idx as u32).as_bytes(),
                s.as_bytes(),
            );
        }

        for r in &file_sym.ranges {
            batch.insert(
                &self.ranges,
                RangeKey::new(fid, r.va_start, r.depth).as_bytes(),
                RangeValue::from_range(r).as_bytes(),
            );
        }

        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let num_ranges = file_sym.ranges.len() as u32;
        let mut meta_val = num_ranges.to_be_bytes().to_vec();
        meta_val.extend_from_slice(file_name.as_bytes());
        let fid_key = U128::<BigEndian>::new(fid);
        batch.insert(&self.files, fid_key.as_bytes(), &meta_val);

        batch.commit()?;

        let bname = basename_of(&file_name);
        self.basename_index
            .write()
            .unwrap()
            .insert(bname, file_sym.file_id);

        Ok(())
    }

    /// Find all symbol frames covering `addr` in the given file.
    ///
    /// Scans backwards from `(file_id, addr, MAX_DEPTH)` until a depth-0
    /// containing range is found, collecting inline frames along the way.
    /// Returns frames sorted by depth (outermost first).
    pub fn lookup(&self, file_id: FileId, addr: VirtAddr) -> crate::Result<Vec<ResolvedFrame>> {
        let fid: u128 = file_id.into();
        let lower = RangeKey::new(fid, 0, 0);
        let upper = RangeKey::new(fid, addr, u16::MAX);

        let mut frames = Vec::new();

        for item in self.ranges.range(lower.as_bytes()..=upper.as_bytes()).rev() {
            let kv = item?;
            let Ok(key) = RangeKey::ref_from_bytes(&kv.0) else {
                continue;
            };
            let Ok(val) = RangeValue::ref_from_bytes(&kv.1) else {
                continue;
            };

            let start = key.va_start();
            let end = start.saturating_add(val.length() as u64);

            if addr >= start && addr < end {
                frames.push(ResolvedFrame {
                    func: self.resolve_string(fid, val.func_ref())?,
                    depth: key.depth(),
                });
            }
            if key.depth() == 0 {
                break;
            }
        }

        frames.sort_unstable_by_key(|f| f.depth);
        Ok(frames)
    }

    fn resolve_string(&self, file_id: u128, idx: u32) -> crate::Result<String> {
        let key = StringKey::new(file_id, idx);
        match self.strings.get(key.as_bytes())? {
            Some(v) => Ok(String::from_utf8_lossy(&v).into_owned()),
            None => Ok("[unknown]".into()),
        }
    }

    /// Resolve a mapping basename to a stored FileId.
    pub fn file_id_for_basename(&self, basename: &str) -> Option<FileId> {
        self.basename_index
            .read()
            .ok()
            .map(|base| base.get(basename).copied())?
    }

    /// List all stored executables.
    pub fn list_files(&self) -> crate::Result<Vec<ExecutableInfo>> {
        let mut result = Vec::new();
        for item in self.files.range::<Vec<u8>, _>(..) {
            let (kb, vb) = item?;
            if let Some(info) = parse_file_meta(&kb, &vb) {
                result.push(info);
            }
        }
        Ok(result)
    }

    /// Remove all stored symbols for a given file.
    pub fn remove_file_symbols(&self, file_id: FileId) -> crate::Result<()> {
        let fid: u128 = file_id.into();
        let prefix = U128::<BigEndian>::new(fid);
        let prefix_bytes = prefix.as_bytes();

        let mut batch = self.keyspace.batch();

        for item in self.ranges.prefix(prefix_bytes) {
            batch.remove(&self.ranges, item?.0);
        }
        for item in self.strings.prefix(prefix_bytes) {
            batch.remove(&self.strings, item?.0);
        }
        batch.remove(&self.files, prefix_bytes);
        batch.commit()?;

        self.basename_index
            .write()
            .unwrap()
            .retain(|_, v| *v != file_id);

        Ok(())
    }
}

fn parse_file_meta(kb: &[u8], vb: &[u8]) -> Option<ExecutableInfo> {
    let fid_key = U128::<BigEndian>::ref_from_bytes(kb).ok()?;
    if vb.len() < 4 {
        return None;
    }
    let num_ranges = u32::from_be_bytes(vb[..4].try_into().ok()?);
    let file_name = String::from_utf8_lossy(&vb[4..]).into_owned();
    Some(ExecutableInfo {
        file_id: FileId::from(fid_key.get()),
        file_name,
        num_ranges,
    })
}

fn basename_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

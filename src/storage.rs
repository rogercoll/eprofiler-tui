use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use fjall::{Database, Keyspace, KeyspaceCreateOptions};
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

impl ExecutableInfo {
    /// Value in the `files` partition: big-endian range count, then the file name.
    fn encode_value(&self) -> Vec<u8> {
        let mut value = self.num_ranges.to_be_bytes().to_vec();
        value.extend_from_slice(self.file_name.as_bytes());
        value
    }

    /// Inverse of [`Self::encode_value`], with the file ID taken from the key.
    fn decode(key: &[u8], value: &[u8]) -> Option<Self> {
        let file_id = U128::<BigEndian>::ref_from_bytes(key).ok()?.get();
        let (count, name) = value.split_first_chunk::<4>()?;
        Some(Self {
            file_id: FileId::from(file_id),
            file_name: String::from_utf8_lossy(name).into_owned(),
            num_ranges: u32::from_be_bytes(*count),
        })
    }
}

/// Persistent symbol store backed by fjall (LSM-tree).
///
/// Three partitions:
///   - **ranges**: `RangeKey -> RangeValue` (fixed 26-byte key, 20-byte value)
///   - **strings**: `StringKey -> raw UTF-8` (fixed 20-byte key, variable value)
///   - **files**: `U128<BE> -> num_ranges(4) + filename` (executable metadata)
pub struct SymbolStore {
    db: Database,
    ranges: Keyspace,
    strings: Keyspace,
    files: Keyspace,
    basename_index: RwLock<HashMap<String, FileId>>,
}

impl SymbolStore {
    pub fn open(path: impl AsRef<Path>) -> crate::Result<Self> {
        let db = Database::builder(path.as_ref())
            .open()
            .map_err(|e| match e {
                fjall::Error::InvalidVersion(_) => {
                    crate::error::Error::StorageVersionMismatch(path.as_ref().to_path_buf())
                }
                other => other.into(),
            })?;
        let ranges = db.keyspace("ranges", KeyspaceCreateOptions::default)?;
        let strings = db.keyspace("strings", KeyspaceCreateOptions::default)?;
        let files = db.keyspace("files", KeyspaceCreateOptions::default)?;

        let store = Self {
            db,
            ranges,
            strings,
            files,
            basename_index: RwLock::new(HashMap::new()),
        };

        // Rebuild in-memory basename index from persisted metadata.
        for info in store.list_files()? {
            store
                .basename_index
                .write()
                .unwrap()
                .insert(info.file_name, info.file_id);
        }

        Ok(store)
    }

    /// Atomically persist all ranges, interned strings, and file metadata.
    pub fn store_file_symbols(&self, file_sym: &FileSym, path: &Path) -> crate::Result<()> {
        let fid: u128 = file_sym.file_id.into();
        let mut batch = self.db.batch();

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

        let info = ExecutableInfo {
            file_id: file_sym.file_id,
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            num_ranges: file_sym.ranges.len() as u32,
        };
        batch.insert(
            &self.files,
            U128::<BigEndian>::new(fid).as_bytes(),
            info.encode_value(),
        );
        batch.commit()?;

        self.basename_index
            .write()
            .unwrap()
            .insert(info.file_name, info.file_id);

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

        for guard in self.ranges.range(lower.as_bytes()..=upper.as_bytes()).rev() {
            let (kb, vb) = guard.into_inner()?;
            let Ok(key) = RangeKey::ref_from_bytes(&kb) else {
                continue;
            };
            let Ok(val) = RangeValue::ref_from_bytes(&vb) else {
                continue;
            };

            let start = key.va_start.get();
            let end = start.saturating_add(val.length.get() as u64);

            if addr >= start && addr < end {
                frames.push(ResolvedFrame {
                    func: self.resolve_string(fid, val.func_ref.get())?,
                    depth: key.depth.get(),
                });
            }
            if key.depth.get() == 0 {
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
        for guard in self.files.range::<Vec<u8>, _>(..) {
            let (kb, vb) = guard.into_inner()?;
            if let Some(info) = ExecutableInfo::decode(&kb, &vb) {
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

        let mut batch = self.db.batch();

        for guard in self.ranges.prefix(prefix_bytes) {
            batch.remove(&self.ranges, guard.key()?);
        }
        for guard in self.strings.prefix(prefix_bytes) {
            batch.remove(&self.strings, guard.key()?);
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

#[cfg(test)]
mod tests {
    use indexmap::IndexSet;

    use super::*;
    use crate::symbolizer::{StringRef, SymRange};

    fn symbols(file_id: u128) -> FileSym {
        let range = |va_start, length, func, depth| SymRange {
            va_start,
            length,
            func: StringRef(func),
            file: None,
            call_file: None,
            call_line: None,
            depth,
        };
        FileSym {
            file_id: FileId::from(file_id),
            // `outer` spans 0x1000..0x1100; `inner` is inlined at 0x1010..0x1020.
            ranges: vec![range(0x1000, 0x100, 0, 0), range(0x1010, 0x10, 1, 1)],
            strings: IndexSet::from(["outer".to_string(), "inner".to_string()]),
        }
    }

    #[test]
    fn metadata_round_trips_through_the_files_partition() {
        let info = ExecutableInfo {
            file_id: FileId::from(42u128),
            file_name: "app".into(),
            num_ranges: 7,
        };
        let key = U128::<BigEndian>::new(42);
        let decoded = ExecutableInfo::decode(key.as_bytes(), &info.encode_value()).unwrap();
        assert_eq!(
            (decoded.file_id, decoded.file_name, decoded.num_ranges),
            (info.file_id, info.file_name, info.num_ranges)
        );
        assert!(ExecutableInfo::decode(key.as_bytes(), &[0, 1]).is_none());
    }

    #[test]
    fn store_lookup_and_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SymbolStore::open(tmp.path()).unwrap();
        store
            .store_file_symbols(&symbols(7), Path::new("/usr/bin/app"))
            .unwrap();

        let files = store.list_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            (files[0].file_name.as_str(), files[0].num_ranges),
            ("app", 2)
        );
        let id = store.file_id_for_basename("app").unwrap();

        let funcs = |addr| -> Vec<String> {
            store
                .lookup(id, addr)
                .unwrap()
                .into_iter()
                .map(|f| f.func)
                .collect()
        };
        assert_eq!(funcs(0x1004), ["outer"]);
        assert_eq!(funcs(0x1014), ["outer", "inner"]);
        assert!(funcs(0x2000).is_empty());

        store.remove_file_symbols(id).unwrap();
        assert!(store.list_files().unwrap().is_empty());
        assert!(store.file_id_for_basename("app").is_none());
    }

    #[test]
    fn reopening_restores_the_basename_index() {
        let tmp = tempfile::tempdir().unwrap();
        let path = Path::new("/opt/svc");
        SymbolStore::open(tmp.path())
            .unwrap()
            .store_file_symbols(&symbols(9), path)
            .unwrap();
        let reopened = SymbolStore::open(tmp.path()).unwrap();
        assert_eq!(
            reopened.file_id_for_basename("svc"),
            Some(FileId::from(9u128))
        );
    }
}

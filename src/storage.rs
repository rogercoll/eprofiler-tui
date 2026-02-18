use std::path::Path;

use fjall::{Keyspace, PartitionHandle};
use symblib::VirtAddr;
use symblib::fileid::FileId;
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

impl From<&SymRange> for RangeValue {
    fn from(r: &SymRange) -> Self {
        Self {
            length: U32::new(r.length),
            func_ref: U32::new(r.func.0),
            file_ref: U32::new(r.file.map_or(NONE_REF, |s| s.0)),
            call_file_ref: U32::new(r.call_file.map_or(NONE_REF, |s| s.0)),
            call_line: U32::new(r.call_line.unwrap_or(0)),
        }
    }
}

impl RangeValue {
    fn file_ref(&self) -> Option<u32> {
        let v = self.file_ref.get();
        (v != NONE_REF).then_some(v)
    }

    fn call_file_ref(&self) -> Option<u32> {
        let v = self.call_file_ref.get();
        (v != NONE_REF).then_some(v)
    }

    fn call_line(&self) -> Option<u32> {
        let v = self.call_line.get();
        (v != 0).then_some(v)
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
    pub file: Option<String>,
    pub call_file: Option<String>,
    pub call_line: Option<u32>,
    pub depth: u16,
}

/// Persistent symbol store backed by fjall (LSM-tree).
///
/// Two partitions:
///   - **ranges**: `RangeKey -> RangeValue` (fixed 26-byte key, 20-byte value)
///   - **strings**: `StringKey -> raw UTF-8` (fixed 20-byte key, variable value)
pub struct SymbolStore {
    keyspace: Keyspace,
    ranges: PartitionHandle,
    strings: PartitionHandle,
}

impl SymbolStore {
    pub fn open(path: impl AsRef<Path>) -> crate::Result<Self> {
        let keyspace = fjall::Config::new(path).open()?;
        let ranges = keyspace.open_partition("ranges", Default::default())?;
        let strings = keyspace.open_partition("strings", Default::default())?;
        Ok(Self {
            keyspace,
            ranges,
            strings,
        })
    }

    /// Atomically persist all ranges and interned strings for one binary.
    pub fn store_file_symbols(&self, file_sym: &FileSym) -> crate::Result<()> {
        let fid: u128 = file_sym.file_id.into();
        let mut batch = self.keyspace.batch();

        for (idx, s) in file_sym.strings.iter().enumerate() {
            let key = StringKey::new(fid, idx as u32);
            batch.insert(&self.strings, key.as_bytes(), s.as_bytes());
        }

        for r in &file_sym.ranges {
            let key = RangeKey::new(fid, r.va_start, r.depth);
            let val = RangeValue::from(r);
            batch.insert(&self.ranges, key.as_bytes(), val.as_bytes());
        }

        batch.commit()?;
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

            let start = key.va_start.get();
            let end = start.saturating_add(val.length.get() as u64);

            if addr >= start && addr < end {
                frames.push(ResolvedFrame {
                    func: self.resolve_string(fid, val.func_ref.get())?,
                    file: val
                        .file_ref()
                        .map(|i| self.resolve_string(fid, i))
                        .transpose()?,
                    call_file: val
                        .call_file_ref()
                        .map(|i| self.resolve_string(fid, i))
                        .transpose()?,
                    call_line: val.call_line(),
                    depth: key.depth.get(),
                });

                if key.depth.get() == 0 {
                    break;
                }
            } else if key.depth.get() == 0 {
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
}

use indexmap::IndexSet;
use std::ops::Range;
use std::path::PathBuf;
use symblib::fileid::FileId;
use symblib::symbconv::RangeExtractor;

use symblib::{VirtAddr, symbconv};

#[derive(Debug)]
struct Symbolizer {}

pub struct StringRef(pub usize);

struct FileSym {
    file_id: FileId,
    ranges: Vec<SymRange>,
}

struct SymRange {
    range: Range<VirtAddr>,
    func: StringRef,
    file: Option<StringRef>,
    call_file: Option<StringRef>,
    call_line: Option<u32>,
    depth: u16,
}

impl Symbolizer {
    fn new() -> Self {
        todo!();
    }

    fn extract_symbols(path: PathBuf) -> crate::Result<FileSym> {
        let file_id = FileId::from_path(&path)?;
        let obj = symblib::objfile::File::load(&path)?;
        let obj = obj.parse()?;
        let dwarf_secs = symblib::dwarf::Sections::load(&obj)?;

        let mut multi_extractor = symbconv::multi::Extractor::new(&obj)?;
        multi_extractor.add("dwarf", symbconv::dwarf::Extractor::new(&dwarf_secs));
        multi_extractor.add("go", symbconv::go::Extractor::new(&obj));
        multi_extractor.add(
            "dbg-obj-sym",
            symbconv::obj::Extractor::new(&obj, symblib::objfile::SymbolSource::Debug),
        );
        multi_extractor.add(
            "dyn-obj-sym",
            symbconv::obj::Extractor::new(&obj, symblib::objfile::SymbolSource::Dynamic),
        );

        let mut strings = IndexSet::with_capacity(1024);
        let mut ranges = Vec::with_capacity(1024);
        multi_extractor.extract(&mut |range| {
            // Read and convert range to database format.
            ranges.push(SymRange {
                range: range.va_range(),
                func: StringRef(strings.insert_full(range.func).0),
                file: range.file.map(|f| StringRef(strings.insert_full(f).0)),
                call_file: range
                    .call_file
                    .map(|cf| StringRef(strings.insert_full(cf).0)),
                call_line: range.call_line,
                depth: range.depth as u16,
            });

            Ok(())
        })?;
        Ok(FileSym { file_id, ranges })
    }
}

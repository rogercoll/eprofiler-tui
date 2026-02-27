use indexmap::IndexSet;
use std::path::Path;
use symblib::symbconv::RangeExtractor;
use symblib::{VirtAddr, symbconv};

#[derive(Debug, Clone, Copy)]
pub struct StringRef(pub u32);

pub struct FileSym {
    pub file_id: symblib::fileid::FileId,
    pub ranges: Vec<SymRange>,
    pub strings: IndexSet<String>,
}

pub struct SymRange {
    pub va_start: VirtAddr,
    pub length: u32,
    pub func: StringRef,
    pub file: Option<StringRef>,
    pub call_file: Option<StringRef>,
    pub call_line: Option<u32>,
    pub depth: u16,
}

pub fn extract_symbols(path: &Path) -> crate::Result<FileSym> {
    let file_id = symblib::fileid::FileId::from_path(path)?;
    let obj = symblib::objfile::File::load(path)?;
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
        let (func_idx, _) = strings.insert_full(range.func);
        ranges.push(SymRange {
            va_start: range.elf_va,
            length: range.length,
            func: StringRef(func_idx as u32),
            file: range.file.map(|f| {
                let (i, _) = strings.insert_full(f);
                StringRef(i as u32)
            }),
            call_file: range.call_file.map(|cf| {
                let (i, _) = strings.insert_full(cf);
                StringRef(i as u32)
            }),
            call_line: range.call_line,
            depth: range.depth as u16,
        });
        Ok(())
    })?;

    ranges.sort_unstable_by(|a, b| a.va_start.cmp(&b.va_start).then(a.depth.cmp(&b.depth)));

    Ok(FileSym {
        file_id,
        ranges,
        strings,
    })
}

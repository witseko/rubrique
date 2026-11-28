//! Read the sheet names and one sheet's cells from an Excel `.xls`
//! workbook: Excel 97-2003 (BIFF8) or Excel 5.0/95 (BIFF5).
//!
//! The file is an OLE compound file: a small file system of fixed-size
//! sectors, chained through a file allocation table (FAT), with a directory
//! of named streams. Small streams live in a "mini stream" of 64-byte
//! sectors with their own mini FAT. The `Workbook` stream holds BIFF8
//! records: a type, a length and data. The workbook globals list the sheets
//! (`BOUNDSHEET`) and the shared strings (`SST`); each sheet's records then
//! hold its cells. BIFF5 has the same records, except that text is stored
//! as bytes in the workbook's code page (its `CODEPAGE` record) and there
//! are no shared strings.

use super::codepage;
use super::table::Table;
use super::zip_archive::{read_u16, read_u32, read_u64};

/// The file signature of a compound file.
const SIGNATURE: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";
/// FAT value at the end of a sector chain.
const END_OF_CHAIN: u32 = 0xffff_fffe;
/// Directory entry type of a stream.
const STREAM: u8 = 2;
/// Directory entry ID that means "none".
const NO_ENTRY: u32 = 0xffff_ffff;
/// Size of one directory entry.
const DIRECTORY_ENTRY_BYTES: usize = 128;
/// Most directory entries read.
const MAX_DIRECTORY_ENTRIES: usize = 10_000;

/// BIFF record types read here.
mod record {
    pub const BOF: u16 = 0x0809;
    pub const CODEPAGE: u16 = 0x0042;
    pub const EOF: u16 = 0x000a;
    pub const FILEPASS: u16 = 0x002f;
    pub const CONTINUE: u16 = 0x003c;
    pub const BOUNDSHEET: u16 = 0x0085;
    pub const SST: u16 = 0x00fc;
    pub const LABELSST: u16 = 0x00fd;
    pub const LABEL: u16 = 0x0204;
    pub const RSTRING: u16 = 0x00d6;
    pub const NUMBER: u16 = 0x0203;
    pub const RK: u16 = 0x027e;
    pub const MULRK: u16 = 0x00bd;
    pub const BOOLERR: u16 = 0x0205;
    pub const FORMULA: u16 = 0x0006;
    pub const STRING: u16 = 0x0207;
}

/// The sheet names and the cells of sheet number `selected`.
pub(super) fn sheet(bytes: &[u8], selected: usize) -> Result<(Vec<String>, Table), String> {
    let workbook = CompoundFile::new(bytes)?.workbook_stream()?;
    read_workbook(&workbook, selected)
}

/// The sheet names and the cells of sheet number `selected` in a workbook
/// stream.
fn read_workbook(workbook: &[u8], selected: usize) -> Result<(Vec<String>, Table), String> {
    let globals = read_globals(Records::new(workbook))?;
    let names = globals.sheets.iter().map(|(name, _)| name.clone()).collect();
    let (_, offset) = globals.sheets.get(selected).ok_or("This workbook has no sheet to display.")?;
    let start = usize::try_from(*offset).map_err(|_| damaged())?;
    let sheet_records = Records::new(workbook.get(start..).ok_or_else(damaged)?);
    let table = read_cells(sheet_records, &globals);
    Ok((names, table))
}

fn damaged() -> String {
    "This Excel workbook is damaged.".into()
}

/// The parts of a compound file needed to read its streams.
struct CompoundFile<'a> {
    bytes: &'a [u8],
    sector_size: usize,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    /// The mini stream: the root entry's stream, holding small streams.
    mini_stream: Vec<u8>,
    mini_cutoff: u64,
    directory: Vec<u8>,
}

impl<'a> CompoundFile<'a> {
    /// Read the header, the FAT, the directory and the mini stream.
    fn new(bytes: &'a [u8]) -> Result<Self, String> {
        if !bytes.starts_with(SIGNATURE) {
            return Err("This file is not an Excel workbook.".into());
        }
        let header16 = |offset| read_u16(bytes, offset).ok_or_else(damaged);
        let header32 = |offset| read_u32(bytes, offset).ok_or_else(damaged);
        let sector_size = match header16(30)? {
            9 => 512,
            12 => 4096,
            _ => return Err(damaged()),
        };
        let mut file = Self {
            bytes,
            sector_size,
            fat: Vec::new(),
            mini_fat: Vec::new(),
            mini_stream: Vec::new(),
            mini_cutoff: u64::from(header32(56)?),
            directory: Vec::new(),
        };
        file.fat = file.read_fat(header32(44)?, header32(68)?)?;
        file.directory = file.chain(header32(48)?, usize::MAX)?;
        file.mini_fat = file
            .chain(header32(60)?, usize::MAX)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|value| u32::from_le_bytes(*value))
            .collect();
        if let Some(root) = file.directory_entry(0) {
            let size = usize::try_from(root.size).map_err(|_| damaged())?;
            file.mini_stream = file.chain(root.start, size)?;
        }
        Ok(file)
    }

    /// The FAT: its sectors are listed in the header's 109 DIFAT slots,
    /// then in a chain of DIFAT sectors.
    fn read_fat(&self, fat_sectors: u32, first_difat: u32) -> Result<Vec<u32>, String> {
        let wanted = usize::try_from(fat_sectors).map_err(|_| damaged())?;
        if wanted > self.sector_count() {
            return Err(damaged());
        }
        let mut sectors: Vec<u32> = (0..109).filter_map(|slot| read_u32(self.bytes, 76 + slot * 4)).collect();
        let mut difat = first_difat;
        let mut visited = 0;
        while difat < END_OF_CHAIN && sectors.len() < wanted {
            visited += 1;
            if visited > self.sector_count() {
                return Err(damaged());
            }
            let sector = self.sector(difat).ok_or_else(damaged)?;
            let (entries, next) = sector.split_at(self.sector_size - 4);
            sectors.extend(entries.as_chunks::<4>().0.iter().map(|value| u32::from_le_bytes(*value)));
            difat = read_u32(next, 0).ok_or_else(damaged)?;
        }
        sectors.truncate(wanted);
        let mut fat = Vec::with_capacity(wanted * self.sector_size / 4);
        for sector in sectors {
            let data = self.sector(sector).ok_or_else(damaged)?;
            fat.extend(data.as_chunks::<4>().0.iter().map(|value| u32::from_le_bytes(*value)));
        }
        Ok(fat)
    }

    /// The number of whole sectors after the header.
    fn sector_count(&self) -> usize {
        (self.bytes.len() / self.sector_size).saturating_sub(1)
    }

    /// The bytes of sector `index`.
    fn sector(&self, index: u32) -> Option<&'a [u8]> {
        let start = (usize::try_from(index).ok()? + 1).checked_mul(self.sector_size)?;
        self.bytes.get(start..start.checked_add(self.sector_size)?)
    }

    /// Follow a FAT chain from `start` and join its sectors, up to `limit`
    /// bytes. A chain longer than the file's sector count loops, so the
    /// result is never larger than the file.
    fn chain(&self, start: u32, limit: usize) -> Result<Vec<u8>, String> {
        let mut data = Vec::new();
        let mut sector = start;
        let mut visited = 0;
        while sector < END_OF_CHAIN && data.len() < limit {
            visited += 1;
            if visited > self.sector_count() {
                return Err(damaged());
            }
            data.extend_from_slice(self.sector(sector).ok_or_else(damaged)?);
            sector = *self.fat.get(usize::try_from(sector).map_err(|_| damaged())?).ok_or_else(damaged)?;
        }
        data.truncate(limit);
        Ok(data)
    }

    /// Follow a mini FAT chain and join its 64-byte sectors, up to `limit`
    /// bytes. A chain longer than the mini stream's sector count loops.
    fn mini_chain(&self, start: u32, limit: usize) -> Result<Vec<u8>, String> {
        let mut data = Vec::new();
        let mut sector = start;
        let mut visited = 0;
        while sector < END_OF_CHAIN && data.len() < limit {
            visited += 1;
            if visited > self.mini_stream.len() / 64 {
                return Err(damaged());
            }
            let index = usize::try_from(sector).map_err(|_| damaged())?;
            let begin = index.checked_mul(64).ok_or_else(damaged)?;
            data.extend_from_slice(self.mini_stream.get(begin..begin + 64).ok_or_else(damaged)?);
            sector = *self.mini_fat.get(index).ok_or_else(damaged)?;
        }
        data.truncate(limit);
        Ok(data)
    }

    /// Directory entry number `index`.
    fn directory_entry(&self, index: u32) -> Option<DirectoryEntry> {
        let start = usize::try_from(index).ok()?.checked_mul(DIRECTORY_ENTRY_BYTES)?;
        let entry = self.directory.get(start..start + DIRECTORY_ENTRY_BYTES)?;
        let name_bytes = usize::from(read_u16(entry, 64)?).min(64);
        let units: Vec<u16> = entry[..name_bytes]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
            .take_while(|unit| *unit != 0)
            .collect();
        Some(DirectoryEntry {
            name: String::from_utf16_lossy(&units),
            kind: entry[66],
            left: read_u32(entry, 68)?,
            right: read_u32(entry, 72)?,
            child: read_u32(entry, 76)?,
            start: read_u32(entry, 116)?,
            // Version 3 files may leave garbage in the high half.
            size: if self.sector_size == 512 { u64::from(read_u32(entry, 120)?) } else { read_u64(entry, 120)? },
        })
    }

    /// The `Workbook` stream at the top level, or the Excel 5.0/95 `Book`
    /// stream when there is no `Workbook` (a file saved for both versions
    /// has both).
    fn workbook_stream(&self) -> Result<Vec<u8>, String> {
        let root = self.directory_entry(0).ok_or_else(damaged)?;
        let mut pending = vec![root.child];
        let mut book = None;
        let mut visited = 0;
        while let Some(index) = pending.pop() {
            if index == NO_ENTRY {
                continue;
            }
            visited += 1;
            if visited > MAX_DIRECTORY_ENTRIES {
                return Err(damaged());
            }
            let entry = self.directory_entry(index).ok_or_else(damaged)?;
            if entry.kind == STREAM && entry.name.eq_ignore_ascii_case("Workbook") {
                return self.stream(&entry);
            }
            if entry.kind == STREAM && entry.name.eq_ignore_ascii_case("Book") {
                book = Some(entry.clone());
            }
            pending.extend([entry.left, entry.right]);
        }
        match book {
            Some(entry) => self.stream(&entry),
            None => Err("This file has no Excel workbook inside.".into()),
        }
    }

    /// The bytes of a stream, from the mini stream when it is small.
    fn stream(&self, entry: &DirectoryEntry) -> Result<Vec<u8>, String> {
        let size = usize::try_from(entry.size).map_err(|_| damaged())?;
        if size > self.bytes.len() {
            return Err(damaged());
        }
        if entry.size < self.mini_cutoff { self.mini_chain(entry.start, size) } else { self.chain(entry.start, size) }
    }
}

/// The fields of a directory entry read here.
#[derive(Clone)]
struct DirectoryEntry {
    name: String,
    kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
}

/// BIFF records in order: (type, data).
struct Records<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Records<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// The type of the next record, without moving past it.
    fn peek_kind(&self) -> Option<u16> {
        read_u16(self.bytes, self.position)
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = (u16, &'a [u8]);

    /// The next record. A record cut off by the end of the stream ends the
    /// iteration, so callers that peek first cannot see it again.
    fn next(&mut self) -> Option<Self::Item> {
        let kind = read_u16(self.bytes, self.position)?;
        let start = self.position + 4;
        let data = read_u16(self.bytes, self.position + 2)
            .and_then(|length| self.bytes.get(start..start + usize::from(length)));
        let Some(data) = data else {
            self.position = self.bytes.len();
            return None;
        };
        self.position = start + data.len();
        Some((kind, data))
    }
}

/// How a workbook stores its text.
#[derive(Clone, Copy)]
enum Version {
    /// Excel 97-2003: Unicode strings that start with an option byte.
    Biff8,
    /// Excel 5.0/95: bytes in a Windows code page.
    Biff5 { codepage: u16 },
}

impl Version {
    /// A string of `length` characters stored in `data`, after its length
    /// field.
    fn text(self, data: &[u8], length: usize) -> String {
        match self {
            Self::Biff8 => ContinuedData::new(vec![data]).string(length),
            Self::Biff5 { codepage } => {
                data.iter().take(length).map(|byte| codepage::character(*byte, codepage)).collect()
            }
        }
    }
}

/// The Windows code page for the value of a `CODEPAGE` record, which uses
/// two numbers of its own for Mac Roman and Windows Latin 1.
fn windows_codepage(value: u16) -> u16 {
    match value {
        0x8000 => 10000,
        0x8001 => 1252,
        other => other,
    }
}

/// What the workbook globals say.
struct Globals {
    version: Version,
    /// Each worksheet's name and the stream offset of its `BOF` record.
    sheets: Vec<(String, u32)>,
    strings: Vec<String>,
}

/// Read the globals up to their `EOF` record.
fn read_globals(mut records: Records<'_>) -> Result<Globals, String> {
    let version = match records.next() {
        Some((record::BOF, data)) => match read_u16(data, 0) {
            Some(0x0600) => Version::Biff8,
            Some(0x0500) => Version::Biff5 { codepage: 1252 },
            _ => return Err("Excel workbooks older than Excel 5.0 are not supported.".into()),
        },
        _ => return Err(damaged()),
    };
    let mut globals = Globals { version, sheets: Vec::new(), strings: Vec::new() };
    while let Some((kind, data)) = records.next() {
        match kind {
            record::EOF => break,
            record::FILEPASS => return Err("This workbook is password protected.".into()),
            record::CODEPAGE => {
                if let (Version::Biff5 { .. }, Some(value)) = (globals.version, read_u16(data, 0)) {
                    globals.version = Version::Biff5 { codepage: windows_codepage(value) };
                }
            }
            record::BOUNDSHEET => {
                // Sheet type 0 is a worksheet (others are charts and macros).
                if data.get(5) == Some(&0)
                    && let (Some(offset), Some(length), Some(name)) = (read_u32(data, 0), data.get(6), data.get(7..))
                {
                    globals.sheets.push((globals.version.text(name, usize::from(*length)), offset));
                }
            }
            record::SST => {
                let mut parts = vec![data];
                while records.peek_kind() == Some(record::CONTINUE) {
                    parts.extend(records.next().map(|(_, data)| data));
                }
                globals.strings = shared_strings(ContinuedData::new(parts));
            }
            _ => {}
        }
    }
    Ok(globals)
}

/// Read the shared string table.
fn shared_strings(mut data: ContinuedData<'_>) -> Vec<String> {
    data.skip(4);
    let count = data.u32();
    let mut strings = Vec::new();
    while strings.len() < usize::try_from(count).unwrap_or(0) && !data.at_end() {
        let length = usize::from(data.u16());
        strings.push(data.string(length));
    }
    strings
}

/// Record data that may run on in `CONTINUE` records. A string's characters
/// that cross into the next record start it with a fresh option byte (one
/// byte or two bytes per character).
struct ContinuedData<'a> {
    parts: Vec<&'a [u8]>,
    part: usize,
    position: usize,
}

impl<'a> ContinuedData<'a> {
    fn new(parts: Vec<&'a [u8]>) -> Self {
        Self { parts, part: 0, position: 0 }
    }

    fn at_end(&mut self) -> bool {
        self.next_part_if_done();
        self.part >= self.parts.len()
    }

    /// Move to the next record when the current one is used up.
    fn next_part_if_done(&mut self) -> bool {
        let mut moved = false;
        while self.parts.get(self.part).is_some_and(|part| self.position >= part.len()) {
            self.part += 1;
            self.position = 0;
            moved = true;
        }
        moved
    }

    fn u8(&mut self) -> u8 {
        self.next_part_if_done();
        let byte = self.parts.get(self.part).and_then(|part| part.get(self.position)).copied().unwrap_or(0);
        self.position += 1;
        byte
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.u8(), self.u8()])
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes([self.u8(), self.u8(), self.u8(), self.u8()])
    }

    fn skip(&mut self, count: usize) {
        for _ in 0..count {
            self.u8();
        }
    }

    /// Read an `XLUnicodeRichExtendedString` body of `length` characters,
    /// after its length field: the option byte, run and extension counts,
    /// the characters, then the rich text runs and extension data skipped.
    fn string(&mut self, length: usize) -> String {
        let options = self.u8();
        let runs = if options & 0x08 != 0 { usize::from(self.u16()) } else { 0 };
        let extension = if options & 0x04 != 0 { usize::try_from(self.u32()).unwrap_or(0) } else { 0 };
        let mut wide = options & 0x01 != 0;
        let mut units = Vec::with_capacity(length);
        while units.len() < length && !self.at_end() {
            if self.position == 0 && self.part > 0 && !units.is_empty() {
                wide = self.u8() & 0x01 != 0;
            }
            let unit = if wide { self.u16() } else { u16::from(self.u8()) };
            units.push(unit);
            // A character never straddles two records, so check here.
            let _ = self.next_part_if_done();
        }
        self.skip(runs * 4 + extension.min(1 << 20));
        String::from_utf16_lossy(&units)
    }
}

/// Read a sheet's cells up to its `EOF` record.
fn read_cells(mut records: Records<'_>, globals: &Globals) -> Table {
    let mut table = Table::default();
    if !matches!(records.next(), Some((record::BOF, _))) {
        return table;
    }
    while let Some((kind, data)) = records.next() {
        match kind {
            record::EOF => break,
            record::LABELSST => {
                let text = read_u32(data, 6).and_then(|index| globals.strings.get(usize::try_from(index).ok()?));
                set_cell(&mut table, data, text.map_or("", String::as_str));
            }
            record::LABEL | record::RSTRING => {
                let length = usize::from(read_u16(data, 6).unwrap_or(0));
                let text = globals.version.text(data.get(8..).unwrap_or_default(), length);
                set_cell(&mut table, data, &text);
            }
            record::NUMBER => {
                if let Some(value) = read_u64(data, 6) {
                    set_cell(&mut table, data, &f64::from_bits(value).to_string());
                }
            }
            record::RK => {
                if let Some(value) = read_u32(data, 6) {
                    set_cell(&mut table, data, &rk_number(value).to_string());
                }
            }
            record::MULRK => mul_rk(&mut table, data),
            record::BOOLERR => {
                let text = match (data.get(6), data.get(7)) {
                    (Some(value), Some(0)) => {
                        if *value == 0 {
                            "false"
                        } else {
                            "true"
                        }
                    }
                    (Some(code), Some(_)) => error_text(*code),
                    _ => "",
                };
                set_cell(&mut table, data, text);
            }
            record::FORMULA => {
                let text = formula_result(data, &mut records, globals.version);
                set_cell(&mut table, data, &text);
            }
            _ => {}
        }
    }
    table
}

/// Put `text` in the cell whose row and column start the record `data`.
fn set_cell(table: &mut Table, data: &[u8], text: &str) {
    if let (Some(row), Some(column)) = (read_u16(data, 0), read_u16(data, 2))
        && !text.is_empty()
    {
        table.set(usize::from(row), usize::from(column), text);
    }
}

/// Put each value of a `MULRK` record (several RK numbers in one row).
fn mul_rk(table: &mut Table, data: &[u8]) {
    let (Some(row), Some(first_column)) = (read_u16(data, 0), read_u16(data, 2)) else { return };
    let values = data.get(4..data.len().saturating_sub(2)).unwrap_or_default();
    for (offset, value) in values.as_chunks::<6>().0.iter().enumerate() {
        if let Some(number) = read_u32(value, 2) {
            table.set(usize::from(row), usize::from(first_column) + offset, &rk_number(number).to_string());
        }
    }
}

/// The cached result of a `FORMULA` record; a text result is in the
/// `STRING` record that follows.
fn formula_result(data: &[u8], records: &mut Records<'_>, version: Version) -> String {
    let Some(result) = data.get(6..14) else { return String::new() };
    if result[6..8] != [0xff, 0xff] {
        return read_u64(result, 0).map(|bits| f64::from_bits(bits).to_string()).unwrap_or_default();
    }
    match result[0] {
        0 if records.peek_kind() == Some(record::STRING) => {
            let Some((_, string)) = records.next() else { return String::new() };
            let length = usize::from(read_u16(string, 0).unwrap_or(0));
            version.text(string.get(2..).unwrap_or_default(), length)
        }
        1 => if result[2] == 0 { "false" } else { "true" }.to_string(),
        2 => error_text(result[2]).to_string(),
        _ => String::new(),
    }
}

/// Decode an RK number: a 30-bit integer or the high bits of a double,
/// optionally divided by 100.
fn rk_number(value: u32) -> f64 {
    let number = if value & 0x02 != 0 {
        f64::from(value.cast_signed() >> 2)
    } else {
        f64::from_bits(u64::from(value & 0xffff_fffc) << 32)
    };
    if value & 0x01 != 0 { number / 100.0 } else { number }
}

/// The text Excel shows for an error code.
fn error_text(code: u8) -> &'static str {
    match code {
        0x00 => "#NULL!",
        0x07 => "#DIV/0!",
        0x0f => "#VALUE!",
        0x17 => "#REF!",
        0x1d => "#NAME?",
        0x24 => "#NUM!",
        0x2a => "#N/A",
        _ => "#ERROR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_rk_numbers() {
        assert_eq!(rk_number(42 << 2 | 0x02), 42.0);
        assert_eq!(rk_number((-3_i32 << 2).cast_unsigned() | 0x02), -3.0);
        assert_eq!(rk_number(1234 << 2 | 0x03), 12.34);
        assert_eq!(rk_number(0x3ff0_0000), 1.0);
    }

    #[test]
    fn reads_strings_that_continue_in_the_next_record() {
        // Two strings: "ab" (one byte per character), then "cd" split over two
        // records, switching to two bytes per character in the second.
        let first: &[u8] = &[0, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, b'a', b'b', 2, 0, 0, b'c'];
        let second: &[u8] = &[1, b'd', 0];
        assert_eq!(shared_strings(ContinuedData::new(vec![first, second])), ["ab", "cd"]);
    }

    #[test]
    fn rejects_files_that_are_not_compound_files_and_broken_chains() {
        assert!(sheet(b"plain text", 0).is_err());
        let mut broken = include_bytes!("fixtures/two-sheets.xls").to_vec();
        // Point the directory chain at a sector far past the end.
        broken[48..52].copy_from_slice(&0x00ff_ffff_u32.to_le_bytes());
        assert!(sheet(&broken, 0).is_err());
    }

    #[test]
    fn rejects_a_sector_chain_that_points_back_to_itself() {
        let mut looped = include_bytes!("fixtures/two-sheets.xls").to_vec();
        let sector_size = 1 << read_u16(&looped, 30).unwrap();
        let first_fat_sector = read_u32(&looped, 76).unwrap() as usize;
        let directory_start = read_u32(&looped, 48).unwrap();
        let entry = (first_fat_sector + 1) * sector_size + directory_start as usize * 4;
        looped[entry..entry + 4].copy_from_slice(&directory_start.to_le_bytes());
        assert!(sheet(&looped, 0).is_err());
    }

    /// A BIFF record: type, length, data.
    fn biff_record(kind: u16, data: &[u8]) -> Vec<u8> {
        let length = u16::try_from(data.len()).unwrap();
        [&kind.to_le_bytes()[..], &length.to_le_bytes(), data].concat()
    }

    #[test]
    fn reads_excel_95_text_in_the_workbook_code_page() {
        let bof = biff_record(record::BOF, &[0x00, 0x05, 0x10, 0x00]);
        let sheet_bof = biff_record(record::BOF, &[0x00, 0x05, 0x10, 0x00]);
        let eof = biff_record(record::EOF, &[]);
        // Code page 1251: 0xcf 0xf0 0xe8 is "При", 0xe9 is "й".
        let codepage = biff_record(record::CODEPAGE, &1251_u16.to_le_bytes());
        let label = biff_record(record::LABEL, &[0, 0, 0, 0, 0, 0, 3, 0, 0xcf, 0xf0, 0xe8]);
        let number = biff_record(record::RK, &[0, 0, 1, 0, 0, 0, 0x0a, 0, 0, 0]); // 2
        // The sheet starts after the globals; its BOUNDSHEET record is 13 bytes.
        let sheet_offset = u32::try_from(bof.len() + codepage.len() + 13 + eof.len()).unwrap();
        let boundsheet =
            biff_record(record::BOUNDSHEET, &[&sheet_offset.to_le_bytes()[..], &[0, 0, 2, b'L', 0xe9]].concat());
        let workbook = [bof, codepage, boundsheet, eof.clone(), sheet_bof, label, number, eof].concat();
        let (names, table) = read_workbook(&workbook, 0).unwrap();
        assert_eq!(names, ["Lй"]);
        assert_eq!(table.rows, [vec!["При", "2"]]);
    }

    #[test]
    fn reads_a_workbook_stored_in_a_book_stream() {
        let mut renamed = include_bytes!("fixtures/two-sheets.xls").to_vec();
        let name: Vec<u8> = "Workbook".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let at = renamed.windows(name.len()).position(|window| window == name).unwrap();
        let book: Vec<u8> = "Book\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        renamed[at..at + name.len()].fill(0);
        renamed[at..at + book.len()].copy_from_slice(&book);
        renamed[at + 64..at + 66].copy_from_slice(&10_u16.to_le_bytes());
        let (names, _) = sheet(&renamed, 0).unwrap();
        assert_eq!(names, sheet(include_bytes!("fixtures/two-sheets.xls"), 0).unwrap().0);
    }

    #[test]
    fn stops_at_a_continue_record_cut_off_by_the_end_of_the_stream() {
        let workbook: &[u8] = &[
            0x09, 0x08, 4, 0, 0x00, 0x06, 0x10, 0x00, // BOF, BIFF8 globals
            0xfc, 0x00, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, // SST with no strings
            0x3c, 0x00, 0xff, 0xff, // CONTINUE longer than what is left
        ];
        let globals = read_globals(Records::new(workbook)).unwrap();
        assert_eq!(globals.sheets, []);
    }
}

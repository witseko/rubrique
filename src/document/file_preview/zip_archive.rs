//! Read named entries from a ZIP archive in memory (the container of Office
//! and `OpenDocument` files), with size limits checked before inflating.
//!
//! The archive ends with an "end of central directory" record that points to
//! the central directory: one header per entry with its name, sizes and the
//! offset of its local header, which precedes the entry's data.

/// Most entries read from one archive.
const MAX_ENTRIES: usize = 20_000;
/// Signature of the end of central directory record.
const END_SIGNATURE: u32 = 0x0605_4b50;
/// Signature of the ZIP64 end of central directory locator.
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
/// Signature of the ZIP64 end of central directory record.
const ZIP64_END_SIGNATURE: u32 = 0x0606_4b50;
/// Signature of a central directory header.
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
/// Signature of a local file header.
const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
/// Size of the end of central directory record without its comment.
const END_RECORD_BYTES: usize = 22;
/// Compression method: stored as is.
const STORED: u16 = 0;
/// Compression method: deflate.
const DEFLATED: u16 = 8;

/// One file in the archive.
#[derive(Debug)]
struct Entry {
    name: String,
    method: u16,
    encrypted: bool,
    crc32: u32,
    compressed_size: u64,
    size: u64,
    local_header: u64,
}

/// An archive's entries, read from its central directory.
#[derive(Debug)]
pub(super) struct ZipArchive<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

impl<'a> ZipArchive<'a> {
    /// Read the central directory of `bytes`.
    pub(super) fn new(bytes: &'a [u8]) -> Result<Self, String> {
        let (directory_offset, directory_size) = central_directory(bytes)?;
        let start = usize::try_from(directory_offset).map_err(|_| broken())?;
        let end = start.checked_add(usize::try_from(directory_size).map_err(|_| broken())?).ok_or_else(broken)?;
        let mut directory = bytes.get(start..end).ok_or_else(broken)?;
        let mut entries = Vec::new();
        while directory.len() >= 4 && read_u32(directory, 0) == Some(CENTRAL_SIGNATURE) {
            if entries.len() == MAX_ENTRIES {
                return Err("This archive has too many entries to preview.".into());
            }
            let (entry, length) = central_entry(directory)?;
            entries.push(entry);
            directory = &directory[length..];
        }
        Ok(Self { bytes, entries })
    }

    /// The sum of the uncompressed sizes the entries declare.
    pub(super) fn declared_size(&self) -> u64 {
        self.entries.iter().fold(0, |total, entry| total.saturating_add(entry.size))
    }

    /// The uncompressed bytes of entry `name`, refused when it declares or
    /// inflates to more than `limit` bytes.
    pub(super) fn read(&self, name: &str, limit: u64) -> Result<Vec<u8>, String> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| format!("Cannot read {name}: it is missing."))?;
        let too_large = || "This document part exceeds the preview limit.".to_string();
        if entry.size > limit {
            return Err(too_large());
        }
        if entry.encrypted {
            return Err(format!("Cannot read {name}: it is encrypted."));
        }
        let data = self.entry_data(entry).ok_or_else(|| format!("Cannot read {name}: the archive is damaged."))?;
        let size = usize::try_from(entry.size).map_err(|_| too_large())?;
        let contents = match entry.method {
            STORED => data.to_vec(),
            DEFLATED => miniz_oxide::inflate::decompress_to_vec_with_limit(data, size)
                .map_err(|_| format!("Cannot read {name}: its compressed data is damaged or larger than it says."))?,
            _ => return Err(format!("Cannot read {name}: it uses an unsupported compression method.")),
        };
        if contents.len() != size || crc32(&contents) != entry.crc32 {
            return Err(format!("Cannot read {name}: its data does not match its checksum."));
        }
        Ok(contents)
    }

    /// The compressed bytes of an entry, after its local header.
    fn entry_data(&self, entry: &Entry) -> Option<&'a [u8]> {
        let header = usize::try_from(entry.local_header).ok()?;
        let local = self.bytes.get(header..)?;
        if read_u32(local, 0)? != LOCAL_SIGNATURE {
            return None;
        }
        let name_length = usize::from(read_u16(local, 26)?);
        let extra_length = usize::from(read_u16(local, 28)?);
        let start = 30 + name_length + extra_length;
        let end = start.checked_add(usize::try_from(entry.compressed_size).ok()?)?;
        local.get(start..end)
    }
}

/// The offset and size of the central directory, from the end record (or
/// its ZIP64 version when the end record holds placeholder values).
fn central_directory(bytes: &[u8]) -> Result<(u64, u64), String> {
    let last_start =
        bytes.len().checked_sub(END_RECORD_BYTES).ok_or_else(|| "This file is not a ZIP archive.".to_string())?;
    let earliest = last_start.saturating_sub(usize::from(u16::MAX));
    let end = (earliest..=last_start)
        .rev()
        .find(|&start| read_u32(bytes, start) == Some(END_SIGNATURE))
        .ok_or_else(|| "This file is not a ZIP archive.".to_string())?;
    let size = read_u32(bytes, end + 12).ok_or_else(broken)?;
    let offset = read_u32(bytes, end + 16).ok_or_else(broken)?;
    if size != u32::MAX && offset != u32::MAX {
        return Ok((u64::from(offset), u64::from(size)));
    }
    let locator = end.checked_sub(20).ok_or_else(broken)?;
    if read_u32(bytes, locator) != Some(ZIP64_LOCATOR_SIGNATURE) {
        return Err(broken());
    }
    let zip64_end = usize::try_from(read_u64(bytes, locator + 8).ok_or_else(broken)?).map_err(|_| broken())?;
    if read_u32(bytes, zip64_end) != Some(ZIP64_END_SIGNATURE) {
        return Err(broken());
    }
    let size = read_u64(bytes, zip64_end + 40).ok_or_else(broken)?;
    let offset = read_u64(bytes, zip64_end + 48).ok_or_else(broken)?;
    Ok((offset, size))
}

/// Read one central directory header; returns the entry and the header's
/// length.
fn central_entry(header: &[u8]) -> Result<(Entry, usize), String> {
    let field16 = |offset| read_u16(header, offset).ok_or_else(broken);
    let field32 = |offset| read_u32(header, offset).ok_or_else(broken);
    let flags = field16(8)?;
    let name_length = usize::from(field16(28)?);
    let extra_length = usize::from(field16(30)?);
    let comment_length = usize::from(field16(32)?);
    let name_end = 46 + name_length;
    let extra_end = name_end + extra_length;
    let length = extra_end + comment_length;
    let name = header.get(46..name_end).ok_or_else(broken)?;
    let extra = header.get(name_end..extra_end).ok_or_else(broken)?;
    if header.len() < length {
        return Err(broken());
    }
    let mut entry = Entry {
        name: String::from_utf8_lossy(name).into_owned(),
        method: field16(10)?,
        encrypted: flags & 1 != 0,
        crc32: field32(16)?,
        compressed_size: u64::from(field32(20)?),
        size: u64::from(field32(24)?),
        local_header: u64::from(field32(42)?),
    };
    apply_zip64_extra(&mut entry, extra);
    Ok((entry, length))
}

/// Replace placeholder sizes and offset with the 64-bit values of the ZIP64
/// extra field, in the order the format lists them.
fn apply_zip64_extra(entry: &mut Entry, mut extra: &[u8]) {
    while let (Some(id), Some(length)) = (read_u16(extra, 0), read_u16(extra, 2)) {
        let length = usize::from(length);
        let Some(data) = extra.get(4..4 + length) else { return };
        if id == 1 {
            let mut values = data.as_chunks::<8>().0.iter().map(|value| u64::from_le_bytes(*value));
            for field in [&mut entry.size, &mut entry.compressed_size, &mut entry.local_header] {
                if *field == u64::from(u32::MAX) {
                    let Some(value) = values.next() else { return };
                    *field = value;
                }
            }
            return;
        }
        extra = &extra[4 + length..];
    }
}

/// The message for a damaged archive.
fn broken() -> String {
    "This ZIP archive is damaged.".into()
}

/// Little-endian 16-bit number at `offset`.
pub(super) fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?))
}

/// Little-endian 32-bit number at `offset`.
pub(super) fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?))
}

/// Little-endian 64-bit number at `offset`.
pub(super) fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?))
}

/// The CRC-32 lookup table (reflected polynomial 0xEDB88320).
const CRC_TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut index = 0;
    let mut value_of_index: u32 = 0;
    while index < 256 {
        let mut value = value_of_index;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 { 0xedb8_8320 ^ (value >> 1) } else { value >> 1 };
            bit += 1;
        }
        table[index] = value;
        index += 1;
        value_of_index += 1;
    }
    table
};

/// The CRC-32 that ZIP stores for each entry.
fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(u32::MAX, |crc, byte| CRC_TABLE[usize::from(crc.to_le_bytes()[0] ^ byte)] ^ (crc >> 8))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A ZIP archive with `files` stored uncompressed, for tests.
    pub(in crate::document::file_preview) fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut archive = Vec::new();
        let mut directory = Vec::new();
        for (name, contents) in files {
            let offset = u32::try_from(archive.len()).unwrap();
            let size = u32::try_from(contents.len()).unwrap();
            let name_length = u16::try_from(name.len()).unwrap();
            let crc = crc32(contents);
            archive.extend_from_slice(&LOCAL_SIGNATURE.to_le_bytes());
            archive.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            for value in [crc, size, size] {
                archive.extend_from_slice(&value.to_le_bytes());
            }
            archive.extend_from_slice(&name_length.to_le_bytes());
            archive.extend_from_slice(&[0, 0]);
            archive.extend_from_slice(name.as_bytes());
            archive.extend_from_slice(contents);
            directory.extend_from_slice(&CENTRAL_SIGNATURE.to_le_bytes());
            directory.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            for value in [crc, size, size] {
                directory.extend_from_slice(&value.to_le_bytes());
            }
            directory.extend_from_slice(&name_length.to_le_bytes());
            directory.extend_from_slice(&[0; 12]);
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
        }
        let directory_offset = u32::try_from(archive.len()).unwrap();
        let count = u16::try_from(files.len()).unwrap();
        archive.extend_from_slice(&directory);
        archive.extend_from_slice(&END_SIGNATURE.to_le_bytes());
        archive.extend_from_slice(&[0, 0, 0, 0]);
        archive.extend_from_slice(&count.to_le_bytes());
        archive.extend_from_slice(&count.to_le_bytes());
        archive.extend_from_slice(&u32::try_from(directory.len()).unwrap().to_le_bytes());
        archive.extend_from_slice(&directory_offset.to_le_bytes());
        archive.extend_from_slice(&[0, 0]);
        archive
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let archive = stored_zip(&[("a.txt", b"hello"), ("b.txt", b"")]);
        let zip = ZipArchive::new(&archive).unwrap();
        assert_eq!(zip.read("a.txt", 100).unwrap(), b"hello");
        assert_eq!(zip.read("b.txt", 100).unwrap(), b"");
        assert_eq!(zip.declared_size(), 5);
        assert!(zip.read("c.txt", 100).is_err());
        assert!(zip.read("a.txt", 4).is_err());

        let fixture = include_bytes!("fixtures/text.odt");
        let zip = ZipArchive::new(fixture).unwrap();
        assert!(String::from_utf8(zip.read("content.xml", 1 << 20).unwrap()).unwrap().contains("Heading"));
    }

    #[test]
    fn rejects_damaged_archives_and_size_lies() {
        assert!(ZipArchive::new(b"not a zip").is_err());
        let archive = stored_zip(&[("a.txt", b"hello")]);
        assert!(ZipArchive::new(&archive[..archive.len() - 1]).is_err());

        // An entry whose data is longer than its declared size.
        let mut lying = stored_zip(&[("a.txt", b"hello")]);
        let central = lying.windows(4).position(|window| window == CENTRAL_SIGNATURE.to_le_bytes()).unwrap();
        lying[central + 24..central + 28].copy_from_slice(&2u32.to_le_bytes());
        assert!(ZipArchive::new(&lying).unwrap().read("a.txt", 100).is_err());

        // Changed data no longer matches its checksum.
        let mut changed = stored_zip(&[("a.txt", b"hello")]);
        let data = changed.windows(5).position(|window| window == b"hello").unwrap();
        changed[data] = b'j';
        assert!(ZipArchive::new(&changed).unwrap().read("a.txt", 100).is_err());
    }

    #[test]
    fn computes_the_standard_crc32() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }
}

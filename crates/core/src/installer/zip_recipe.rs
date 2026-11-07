//! Read ZIP members in container order and retain their relative structure.
use super::{invalid, Result};
use crate::operation;
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use zip::{HasZipMetadata, ZipArchive};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub index: usize,
    pub source: String,
    pub destination: String,
    pub size: u64,
    pub expected_sha256: Option<String>,
}
/// zip 6 indexes entries by name and loses duplicates. Keep the raw central
/// records in order, presenting each record through an in-memory metadata view
/// while all payload reads still come directly from the original file.
pub struct Archive {
    file: File,
    records: Vec<Vec<u8>>,
    central_start: u64,
    archive_offset: u64,
}
pub fn open(path: &Path) -> Result<Archive> {
    from_file(File::open(path)?)
}
fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn read_at<const N: usize>(file: &mut File, position: u64) -> Result<[u8; N]> {
    file.seek(SeekFrom::Start(position))?;
    let mut bytes = [0u8; N];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn zip64_footer(file: &mut File, locator: u64, declared: u64) -> Result<(u64, [u8; 56])> {
    let header = read_at::<56>(file, declared)?;
    if &header[..4] == b"PK\x06\x06"
        && declared
            .checked_add(12)
            .and_then(|p| p.checked_add(u64_at(&header, 4)))
            == Some(locator)
    {
        return Ok((declared, header));
    }
    // Prepended executable data can shift the physical ZIP64 record while its
    // disk-relative locator remains unchanged. Its size field identifies the
    // actual record ending at the locator; scan metadata without a size limit.
    let mut end = locator;
    while end > 0 {
        operation::check()?;
        let start = end.saturating_sub(64 * 1024);
        let read_end = end.saturating_add(11).min(locator);
        let mut bytes = vec![0u8; (read_end - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut bytes)?;
        for index in (0..(end - start) as usize).rev() {
            if index + 12 <= bytes.len() && &bytes[index..index + 4] == b"PK\x06\x06" {
                let position = start + index as u64;
                if u64_at(&bytes, index + 4) >= 44
                    && position
                        .checked_add(12)
                        .and_then(|p| p.checked_add(u64_at(&bytes, index + 4)))
                        == Some(locator)
                {
                    return Ok((position, read_at(file, position)?));
                }
            }
        }
        end = start;
    }
    Err(invalid("invalid_zip64_footer"))
}

/// Locate the directory using ZIP/ZIP64 footer fields. The comment's 16-bit
/// width bounds this search; no payload size or member-count policy is applied.
fn directory(file: &mut File) -> Result<(u64, u64, u64, u64)> {
    let end = file.seek(SeekFrom::End(0))?;
    let start = end.saturating_sub(22 + u16::MAX as u64);
    let mut tail = vec![0u8; (end - start) as usize];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut tail)?;
    let at = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|at| {
            &tail[*at..*at + 4] == b"PK\x05\x06"
                && *at + 22 + u16_at(&tail, *at + 20) as usize == tail.len()
        })
        .ok_or_else(|| invalid("invalid_zip_footer"))?;
    let eocd = &tail[at..];
    let position = start + at as u64;
    if u16_at(eocd, 4) != 0 || u16_at(eocd, 6) != 0 {
        return Err(invalid("zip_split_archive_unsupported"));
    }
    let mut count = u16_at(eocd, 10) as u64;
    let mut size = u32_at(eocd, 12) as u64;
    let mut offset = u32_at(eocd, 16) as u64;
    let mut directory_end = position;
    if position >= 20 {
        let locator = read_at::<20>(file, position - 20)?;
        if &locator[..4] == b"PK\x06\x07" {
            if u32_at(&locator, 4) != 0 || u32_at(&locator, 16) != 1 {
                return Err(invalid("zip_split_archive_unsupported"));
            }
            let (physical, header) = zip64_footer(file, position - 20, u64_at(&locator, 8))?;
            if u32_at(&header, 16) != 0
                || u32_at(&header, 20) != 0
                || u64_at(&header, 24) != u64_at(&header, 32)
            {
                return Err(invalid("zip_split_archive_unsupported"));
            }
            count = u64_at(&header, 32);
            size = u64_at(&header, 40);
            offset = u64_at(&header, 48);
            directory_end = physical;
        } else if count == u16::MAX as u64 || size == u32::MAX as u64 || offset == u32::MAX as u64 {
            return Err(invalid("missing_zip64_footer"));
        }
    }
    let central_start = directory_end
        .checked_sub(size)
        .ok_or_else(|| invalid("invalid_zip_directory"))?;
    let archive_offset = central_start
        .checked_sub(offset)
        .ok_or_else(|| invalid("invalid_zip_offset"))?;
    Ok((central_start, archive_offset, count, directory_end))
}

/// Keep only metadata consumed by the decoder or filename reader. ZIP comments,
/// timestamps and other descriptive extra fields never gate payload extraction.
fn extraction_record(mut record: Vec<u8>) -> Result<Vec<u8>> {
    let name_end = 46 + u16_at(&record, 28) as usize;
    let extra_end = name_end + u16_at(&record, 30) as usize;
    let mut extra = Vec::new();
    let mut at = name_end;
    while at + 4 <= extra_end {
        let tag = u16_at(&record, at);
        let size = u16_at(&record, at + 2) as usize;
        let consumed = matches!(tag, 0x0001 | 0x7075 | 0x9901);
        if at + 4 + size > extra_end {
            if consumed {
                return Err(invalid("invalid_zip_extraction_metadata"));
            }
            break;
        }
        if consumed {
            extra.extend_from_slice(&record[at..at + 4 + size]);
        }
        at += 4 + size;
    }
    record.truncate(name_end);
    record[30..32].copy_from_slice(&(extra.len() as u16).to_le_bytes());
    record[32..34].copy_from_slice(&0u16.to_le_bytes());
    record.extend(extra);
    Ok(record)
}

pub fn from_file(mut file: File) -> Result<Archive> {
    let (central_start, archive_offset, count, directory_end) = directory(&mut file)?;
    file.seek(SeekFrom::Start(central_start))?;
    let mut records = Vec::new();
    for _ in 0..count {
        operation::check()?;
        if file
            .stream_position()?
            .checked_add(46)
            .is_none_or(|end| end > directory_end)
        {
            return Err(invalid("invalid_zip_directory"));
        }
        let mut record = vec![0u8; 46];
        file.read_exact(&mut record)?;
        if &record[..4] != b"PK\x01\x02" {
            return Err(invalid("invalid_zip_directory"));
        }
        let variable: usize = [28, 30, 32]
            .into_iter()
            .map(|at| u16_at(&record, at) as usize)
            .sum();
        if file
            .stream_position()?
            .checked_add(variable as u64)
            .is_none_or(|end| end > directory_end)
        {
            return Err(invalid("invalid_zip_directory"));
        }
        record.resize(46 + variable, 0);
        file.read_exact(&mut record[46..])?;
        records.push(extraction_record(record)?);
    }
    // A directory digital signature is metadata rather than a file member.
    let position = file.stream_position()?;
    if position != directory_end {
        let signature = read_at::<6>(&mut file, position)?;
        if &signature[..4] != b"PK\x05\x05"
            || file
                .stream_position()?
                .checked_add(u16_at(&signature, 4) as u64)
                != Some(directory_end)
        {
            return Err(invalid("invalid_zip_directory"));
        }
    }
    Ok(Archive {
        file,
        records,
        central_start: central_start - archive_offset,
        archive_offset,
    })
}
impl Archive {
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub(crate) fn entry(&mut self, index: usize) -> Result<ZipArchive<EntryReader<'_>>> {
        let mut tail = self
            .records
            .get(index)
            .ok_or_else(|| invalid("zip_member_missing"))?
            .clone();
        let size = tail.len() as u64;
        let zip64_start = self
            .central_start
            .checked_add(size)
            .ok_or_else(|| invalid("zip_size_overflow"))?;
        // A ZIP64 footer also represents small ZIPs and avoids narrowing an
        // existing large archive offset in this one-record metadata view.
        tail.extend_from_slice(b"PK\x06\x06");
        tail.extend_from_slice(&44u64.to_le_bytes());
        tail.extend_from_slice(&45u16.to_le_bytes());
        tail.extend_from_slice(&45u16.to_le_bytes());
        tail.extend_from_slice(&0u32.to_le_bytes());
        tail.extend_from_slice(&0u32.to_le_bytes());
        tail.extend_from_slice(&1u64.to_le_bytes());
        tail.extend_from_slice(&1u64.to_le_bytes());
        tail.extend_from_slice(&size.to_le_bytes());
        tail.extend_from_slice(&self.central_start.to_le_bytes());
        tail.extend_from_slice(b"PK\x06\x07");
        tail.extend_from_slice(&0u32.to_le_bytes());
        tail.extend_from_slice(&zip64_start.to_le_bytes());
        tail.extend_from_slice(&1u32.to_le_bytes());
        tail.extend_from_slice(b"PK\x05\x06");
        tail.extend_from_slice(&[0u8; 4]);
        tail.extend_from_slice(&1u16.to_le_bytes());
        tail.extend_from_slice(&1u16.to_le_bytes());
        tail.extend_from_slice(&u32::MAX.to_le_bytes());
        tail.extend_from_slice(&u32::MAX.to_le_bytes());
        tail.extend_from_slice(&0u16.to_le_bytes());
        let reader = EntryReader {
            file: &mut self.file,
            tail,
            central_start: self.central_start,
            archive_offset: self.archive_offset,
            position: 0,
        };
        ZipArchive::new(reader).map_err(|e| invalid(format!("invalid_zip:{e}")))
    }
}
pub(crate) struct EntryReader<'a> {
    file: &'a mut File,
    tail: Vec<u8>,
    central_start: u64,
    archive_offset: u64,
    position: u64,
}
impl Read for EntryReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = if self.position < self.central_start {
            let limit = (self.central_start - self.position).min(buffer.len() as u64) as usize;
            self.file.seek(SeekFrom::Start(
                self.archive_offset
                    .checked_add(self.position)
                    .ok_or_else(|| invalid("zip_size_overflow"))?,
            ))?;
            self.file.read(&mut buffer[..limit])?
        } else {
            let offset = usize::try_from(self.position - self.central_start)
                .map_err(|_| invalid("zip_size_overflow"))?;
            if offset >= self.tail.len() {
                return Ok(0);
            }
            let count = buffer.len().min(self.tail.len() - offset);
            buffer[..count].copy_from_slice(&self.tail[offset..offset + count]);
            count
        };
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| invalid("zip_size_overflow"))?;
        Ok(count)
    }
}
impl Seek for EntryReader<'_> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::End(delta) => {
                i128::from(self.central_start) + self.tail.len() as i128 + i128::from(delta)
            }
            SeekFrom::Current(delta) => i128::from(self.position) + i128::from(delta),
        };
        self.position = u64::try_from(position).map_err(|_| invalid("invalid_zip_seek"))?;
        Ok(self.position)
    }
}
/// Filename support otherwise follows the target filesystem.
pub(crate) fn relative(name: &str) -> Result<()> {
    // A colon names an alternate data stream on Windows, rather than a
    // regular archive member. Linux permits colons in ordinary filenames.
    #[cfg(windows)]
    if name.contains(':') {
        return Err(invalid("unsafe_relative_path"));
    }
    if name.is_empty()
        || name.contains('\0')
        || name.starts_with(['/', '\\'])
        || (name.as_bytes().get(1) == Some(&b':') && name.as_bytes()[0].is_ascii_alphabetic())
        || name.split(['/', '\\']).any(|p| p == "..")
        || !name.split(['/', '\\']).any(|p| !p.is_empty() && p != ".")
    {
        return Err(invalid("unsafe_relative_path"));
    }
    Ok(())
}
pub fn plan(zip: &mut Archive, root: &Path, cancel: &AtomicBool) -> Result<Vec<Member>> {
    if !root.is_absolute() || !root.is_dir() || !no_links(root) {
        return Err(invalid("unsafe_game_root"));
    }
    let mut members = Vec::new();
    for index in 0..zip.len() {
        operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("cancelled"));
        }
        let mut entry = zip.entry(index)?;
        let file = entry
            .by_index(0)
            .map_err(|e| invalid(format!("invalid_zip:{e}")))?;
        if file.get_metadata().is_utf8 && std::str::from_utf8(file.name_raw()).is_err() {
            return Err(invalid("zip_name_encoding"));
        }
        let source = file.name().trim_end_matches(['/', '\\']);
        if file.is_dir() && source.split(['/', '\\']).all(|part| part == ".") {
            continue;
        }
        let destination = source.replace('\\', "/");
        relative(&destination)?;
        let destination = destination
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect::<Vec<_>>()
            .join("/");
        let mode = file.unix_mode().unwrap_or(0) & 0o170000;
        if (file.is_dir() && mode != 0 && mode != 0o040000)
            || (!file.is_dir() && mode != 0 && mode != 0o100000)
        {
            return Err(invalid("zip_link_or_special_file"));
        }
        if !file.is_dir() {
            members.push(Member {
                index,
                source: source.into(),
                destination,
                size: file.size(),
                expected_sha256: None,
            });
        }
    }
    if members.is_empty() {
        return Err(invalid("empty_recipe"));
    }
    Ok(members)
}

/// `no_links` port: true when neither `path` nor any ancestor is a symlink /
/// reparse point. Missing components are not links.
pub(crate) fn no_links(path: &Path) -> bool {
    path.ancestors().all(|ancestor| {
        if ancestor.as_os_str().is_empty() {
            return true;
        }
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => !linked(&metadata),
            Err(_) => true,
        }
    })
}

fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{write::SimpleFileOptions, ZipWriter};
    #[test]
    fn preserves_wrappers_extras_case_and_unicode() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("input.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        for name in [
            "wrapper/patch.xp3",
            "wrapper/readme.txt",
            "A.txt",
            "a.txt",
            "内容.txt",
        ] {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"x").unwrap();
        }
        writer.finish().unwrap();
        let members = plan(
            &mut open(&path).unwrap(),
            temp.path(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            members
                .iter()
                .map(|m| m.destination.as_str())
                .collect::<Vec<_>>(),
            [
                "wrapper/patch.xp3",
                "wrapper/readme.txt",
                "A.txt",
                "a.txt",
                "内容.txt"
            ]
        );
    }
    #[test]
    fn normalizes_relative_root_prefix_and_directory_separators() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("relative.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer
            .add_directory("./", SimpleFileOptions::default())
            .unwrap();
        writer
            .start_file("./dir//file.bin", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"payload").unwrap();
        writer.finish().unwrap();
        let members = plan(
            &mut open(&path).unwrap(),
            temp.path(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].destination, "dir/file.bin");
    }
    fn zip_bytes(zip64: bool) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut bytes);
        writer
            .start_file("file.bin", SimpleFileOptions::default().large_file(zip64))
            .unwrap();
        writer.write_all(b"payload").unwrap();
        writer.finish().unwrap();
        bytes.into_inner()
    }

    #[test]
    fn unused_unicode_comment_and_malformed_descriptive_extras_do_not_block_extraction() {
        let bytes = zip_bytes(false);
        let start = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let end = bytes.len() - 22;
        let mut record = bytes[start..end].to_vec();
        let name_end = 46 + u16_at(&record, 28) as usize;
        // 0x6375 is a valid Unicode comment using CRC32 of its raw CP437
        // comment byte. zip 6 instead compares the decoded UTF-8 comment.
        let mut extra = vec![0x75, 0x63, 7, 0, 1];
        extra.extend_from_slice(&0xd1b40d81u32.to_le_bytes());
        extra.extend_from_slice("é".as_bytes());
        // The decoder does not consume a malformed timestamp or unknown tail.
        extra.extend_from_slice(&[0x55, 0x54, 0, 0, 0x34, 0x12, 0xff, 0xff]);
        record.truncate(name_end);
        record[30..32].copy_from_slice(&(extra.len() as u16).to_le_bytes());
        record[32..34].copy_from_slice(&1u16.to_le_bytes());
        record.extend(extra);
        record.push(0x82);
        let mut footer = bytes[end..].to_vec();
        footer[12..16].copy_from_slice(&(record.len() as u32).to_le_bytes());
        let mut bytes = bytes[..start].to_vec();
        bytes.extend(record);
        bytes.extend(footer);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.zip");
        fs::write(&path, bytes).unwrap();
        let mut archive = open(&path).unwrap();
        let members = plan(&mut archive, temp.path(), &AtomicBool::new(false)).unwrap();
        assert_eq!(members[0].destination, "file.bin");
        let mut entry = archive.entry(0).unwrap();
        let mut file = entry.by_index(0).unwrap();
        let mut output = Vec::new();
        file.read_to_end(&mut output).unwrap();
        assert_eq!(output, b"payload");
    }

    #[test]
    fn zip64_member_and_prepended_archive_decode_from_original_file() {
        for zip64 in [false, true] {
            for prefix in [b"".as_slice(), b"MZ inert synthetic prefix".as_slice()] {
                let temp = tempfile::tempdir().unwrap();
                let path = temp.path().join("archive.zip");
                let mut bytes = prefix.to_vec();
                bytes.extend(zip_bytes(zip64));
                fs::write(&path, bytes).unwrap();
                let mut archive = open(&path).unwrap();
                let members = plan(&mut archive, temp.path(), &AtomicBool::new(false)).unwrap();
                assert_eq!(members[0].size, 7);
                let mut entry = archive.entry(0).unwrap();
                let mut file = entry.by_index(0).unwrap();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                assert_eq!(bytes, b"payload");
            }
        }
    }

    #[test]
    fn zip64_footer_with_extensible_data_and_prepended_bytes_is_located() {
        let original = zip_bytes(true);
        let end = original.len() - 22;
        for extension in [b"".as_slice(), b"unconsumed metadata".as_slice()] {
            let mut body = original[..end].to_vec();
            body.extend_from_slice(b"PK\x06\x06");
            body.extend_from_slice(&(44 + extension.len() as u64).to_le_bytes());
            body.extend_from_slice(&[45, 0, 45, 0]);
            body.extend_from_slice(&[0; 8]);
            body.extend_from_slice(&1u64.to_le_bytes());
            body.extend_from_slice(&1u64.to_le_bytes());
            body.extend_from_slice(&(u32_at(&original, end + 12) as u64).to_le_bytes());
            body.extend_from_slice(&(u32_at(&original, end + 16) as u64).to_le_bytes());
            body.extend_from_slice(extension);
            body.extend_from_slice(b"PK\x06\x07");
            body.extend_from_slice(&0u32.to_le_bytes());
            body.extend_from_slice(&(end as u64).to_le_bytes());
            body.extend_from_slice(&1u32.to_le_bytes());
            let mut footer = original[end..].to_vec();
            footer[8..12].fill(0xff);
            footer[12..20].fill(0xff);
            body.extend(footer);
            for prefix in [b"".as_slice(), b"MZ inert prefix".as_slice()] {
                let temp = tempfile::tempdir().unwrap();
                let path = temp.path().join("zip64.zip");
                let mut bytes = prefix.to_vec();
                bytes.extend_from_slice(&body);
                fs::write(&path, bytes).unwrap();
                let mut archive = open(&path).unwrap();
                let mut entry = archive.entry(0).unwrap();
                let mut file = entry.by_index(0).unwrap();
                let mut output = Vec::new();
                file.read_to_end(&mut output).unwrap();
                assert_eq!(output, b"payload");
            }
        }
    }

    #[test]
    fn relative_boundaries() {
        for path in [
            "../outside",
            "/absolute",
            "a/../b",
            "C:/outside",
            "a\\..\\b",
        ] {
            assert!(relative(path).is_err());
        }
        for path in ["内容.txt", "A.txt", "a.txt", "dir/trailing.", "./dir//file"] {
            relative(path).unwrap();
        }
        #[cfg(windows)]
        assert!(relative("file:stream").is_err());
        #[cfg(unix)]
        relative("file:stream").unwrap();
    }
}

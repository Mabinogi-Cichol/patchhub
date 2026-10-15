//! Bounded ZIP central-directory inspection before ZipArchive allocates entries.
use crate::operation;
use std::io::{self, Read, Seek, SeekFrom};

/// Upper bound on the central-directory/metadata bytes inspected before
/// `zip::ZipArchive` allocates per-member entries.
///
/// Protection object: the memory a caller must hold while planning/installing an
/// archive (central directory plus per-member bookkeeping), not an arbitrary
/// accepted payload size. Basis: a byte budget rather than a member-count line,
/// so an ordinary ZIP is admitted while `cd_size + count * RECORD_ESTIMATE`
/// stays within this value (roughly 62.7k members before the estimate
/// dominates). Checked in `exceeds_budget`, reached from `preflight` and
/// `inspect` for every candidate EOCD/ZIP64 record. Over-limit archives fail
/// with the explicit `zip_metadata_limit` error; members are never silently
/// truncated or dropped.
pub const MAX_METADATA_BYTES: u64 = 64 * 1024 * 1024;
/// Conservative per-member estimate for the plan/authorization bookkeeping that
/// accompanies each central-directory record, used only inside
/// `exceeds_budget` to convert a member count into budgeted bytes.
const RECORD_ESTIMATE: u64 = 1024;

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "zip_metadata_invalid")
}
fn candidate_invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "zip_metadata_candidate_invalid")
}
fn metadata_limit() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "zip_metadata_limit")
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

/// Checks ZIP/ZIP64 directory bounds and record framing without allocating
/// per-member metadata. The caller's stream position is restored on all paths.
pub fn preflight<R: Read + Seek>(reader: &mut R) -> io::Result<()> {
    let saved = reader.stream_position()?;
    let result = inspect(reader);
    let restored = reader.seek(SeekFrom::Start(saved));
    match (result, restored) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(_)) => Ok(()),
    }
}

/// zip 6 retries earlier EOCD signatures when the latest central header fails.
/// Scan every candidate the parser could reach, including comments that end
/// before EOF, so an earlier oversized directory cannot bypass the budget.
fn inspect<R: Read + Seek>(reader: &mut R) -> io::Result<()> {
    let end = reader.seek(SeekFrom::End(0))?;
    if end < 22 {
        return Err(invalid());
    }
    const SCAN_BYTES: u64 = 1024 * 1024;
    let mut scan_end = end;
    let mut valid_candidate = false;
    while scan_end > 0 {
        operation::check()?;
        let start = scan_end.saturating_sub(SCAN_BYTES);
        // Include three bytes from the next chunk for signatures on a boundary.
        let read_end = scan_end.saturating_add(3).min(end);
        let mut chunk = vec![0u8; (read_end - start) as usize];
        reader.seek(SeekFrom::Start(start))?;
        reader.read_exact(&mut chunk)?;
        for index in (0..(scan_end - start) as usize).rev() {
            if index + 4 > chunk.len() || chunk[index..index + 4] != *b"PK\x05\x06" {
                continue;
            }
            operation::check()?;
            let position = start + index as u64;
            if position > end - 22 {
                continue;
            }
            match inspect_candidate(reader, end, position) {
                Ok(()) => valid_candidate = true,
                Err(error) if error.to_string() == "zip_metadata_limit" => return Err(error),
                Err(error) if error.to_string() == "zip_metadata_candidate_invalid" => {}
                Err(error) => return Err(error),
            }
        }
        scan_end = start;
    }
    if valid_candidate {
        Ok(())
    } else {
        Err(invalid())
    }
}

fn inspect_candidate<R: Read + Seek>(reader: &mut R, end: u64, eocd_pos: u64) -> io::Result<()> {
    reader.seek(SeekFrom::Start(eocd_pos))?;
    let mut eocd = [0; 22];
    reader.read_exact(&mut eocd)?;
    if eocd[..4] != *b"PK\x05\x06"
        || eocd_pos
            .checked_add(22 + u16_at(&eocd, 20) as u64)
            .is_none_or(|comment_end| comment_end > end)
    {
        return Err(candidate_invalid());
    }
    let zip32_count = u16_at(&eocd, 8) as u64;
    let mut count = zip32_count;
    let mut cd_size = u32_at(&eocd, 12) as u64;
    let mut cd_offset = u32_at(&eocd, 16) as u64;
    let mut cd_end = eocd_pos;
    // ZIP64 is present when a classic field is saturated (zip 6's
    // `may_be_zip64`) OR when a ZIP64 EOCD locator sits immediately before the
    // classic EOCD. The second condition is required for real ZIP64 archives
    // whose values still fit in 32 bits: CPython (and this project's writer,
    // frozen to it) only saturates on overflow, so a central-directory offset
    // in [2 GiB, 4 GiB) is written raw while a ZIP64 EOCD is still emitted.
    // Without this the reader mis-locates the central directory and rejects a
    // valid archive as `zip_metadata_invalid`.
    let saturated = u16_at(&eocd, 10) == u16::MAX || cd_offset == u32::MAX as u64;
    let has_locator = eocd_pos >= 20 && {
        let mut signature = [0u8; 4];
        reader.seek(SeekFrom::Start(eocd_pos - 20))?;
        reader.read_exact(&mut signature).is_ok() && signature == *b"PK\x06\x07"
    };
    let zip64_needed = saturated || has_locator;
    if !zip64_needed && u16_at(&eocd, 4) != u16_at(&eocd, 6) {
        return Err(candidate_invalid());
    }
    let mut footer_bytes = 0;
    if zip64_needed {
        let zip64_result = read_zip64(reader, eocd_pos);
        match zip64_result {
            Ok((zip64_count, zip64_size, zip64_offset, zip64_pos, extensible_bytes)) => {
                count = zip64_count;
                cd_size = zip64_size;
                cd_offset = zip64_offset;
                cd_end = zip64_pos;
                footer_bytes = extensible_bytes;
            }
            Err(error) => {
                // zip 6 may treat an absent or malformed locator as ZIP32.
                // In that case these same fields must still be bounded.
                if exceeds_budget(cd_size, zip32_count.max(u16_at(&eocd, 10) as u64)) {
                    return Err(metadata_limit());
                }
                return Err(error);
            }
        }
    }
    if exceeds_budget(cd_size.saturating_add(footer_bytes), count) {
        return Err(metadata_limit());
    }
    if !zip64_needed && u16_at(&eocd, 10) as u64 != zip32_count {
        return Err(invalid());
    }
    let cd_start = cd_end.checked_sub(cd_size).ok_or_else(invalid)?;
    if cd_offset > cd_start || count > cd_size / 46 {
        return Err(invalid());
    }
    reader.seek(SeekFrom::Start(cd_start))?;
    let mut position = cd_start;
    for _ in 0..count {
        operation::check()?;
        if cd_end - position < 46 {
            return Err(invalid());
        }
        let mut header = [0; 46];
        reader.read_exact(&mut header)?;
        if header[..4] != *b"PK\x01\x02" {
            return Err(invalid());
        }
        let variable =
            u16_at(&header, 28) as u64 + u16_at(&header, 30) as u64 + u16_at(&header, 32) as u64;
        position = position.checked_add(46 + variable).ok_or_else(invalid)?;
        if position > cd_end {
            return Err(invalid());
        }
        reader.seek(SeekFrom::Start(position))?;
    }
    if position != cd_end {
        return Err(invalid());
    }
    Ok(())
}

fn exceeds_budget(cd_size: u64, count: u64) -> bool {
    cd_size
        .checked_add(count.saturating_mul(RECORD_ESTIMATE))
        .is_none_or(|estimate| estimate > MAX_METADATA_BYTES)
}

fn read_zip64<R: Read + Seek>(
    reader: &mut R,
    eocd_pos: u64,
) -> io::Result<(u64, u64, u64, u64, u64)> {
    let locator_pos = eocd_pos.checked_sub(20).ok_or_else(invalid)?;
    reader.seek(SeekFrom::Start(locator_pos))?;
    let mut locator = [0; 20];
    reader.read_exact(&mut locator)?;
    if locator[..4] != *b"PK\x06\x07" || u32_at(&locator, 4) != 0 || u32_at(&locator, 16) != 1 {
        return Err(invalid());
    }
    let zip64_pos = u64_at(&locator, 8);
    if zip64_pos > locator_pos.saturating_sub(56) {
        return Err(invalid());
    }
    reader.seek(SeekFrom::Start(zip64_pos))?;
    let mut record = [0; 56];
    reader.read_exact(&mut record)?;
    if u64_at(&record, 4) > MAX_METADATA_BYTES {
        return Err(metadata_limit());
    }
    let record_end = zip64_pos
        .checked_add(12)
        .and_then(|position| position.checked_add(u64_at(&record, 4)))
        .ok_or_else(invalid)?;
    if record_end != locator_pos
        || record[..4] != *b"PK\x06\x06"
        || u64_at(&record, 4) < 44
        || u32_at(&record, 16) != 0
        || u32_at(&record, 20) != 0
        || u64_at(&record, 24) != u64_at(&record, 32)
    {
        return Err(invalid());
    }
    Ok((
        u64_at(&record, 32),
        u64_at(&record, 40),
        u64_at(&record, 48),
        zip64_pos,
        u64_at(&record, 4) - 44,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn one_file_zip() -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("payload.bin", SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut writer, b"data").unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn validates_directory_and_restores_reader_position() {
        let mut reader = Cursor::new(one_file_zip());
        reader.set_position(3);
        preflight(&mut reader).unwrap();
        assert_eq!(reader.position(), 3);
    }

    #[test]
    fn rejects_invalid_record_and_impossible_count_before_archive_parse() {
        let mut broken = one_file_zip();
        let eocd = broken.len() - 22;
        let cd_start = u32_at(&broken[eocd..], 16) as usize;
        broken[cd_start] = 0;
        let mut reader = Cursor::new(broken);
        reader.set_position(2);
        assert_eq!(
            preflight(&mut reader).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(reader.position(), 2);

        let mut broken = one_file_zip();
        let eocd = broken.len() - 22;
        broken[eocd + 10..eocd + 12].copy_from_slice(&9000u16.to_le_bytes());
        assert_eq!(
            preflight(&mut Cursor::new(broken)).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn accepts_zip64_footer_and_rejects_oversized_directory() {
        let mut bytes = one_file_zip();
        let eocd = bytes.len() - 22;
        let cd_size = u32_at(&bytes[eocd..], 12) as u64;
        let cd_offset = u32_at(&bytes[eocd..], 16) as u64;
        bytes.truncate(eocd);
        let zip64_pos = bytes.len() as u64;
        bytes.extend_from_slice(b"PK\x06\x06");
        bytes.extend_from_slice(&44u64.to_le_bytes());
        bytes.extend_from_slice(&45u16.to_le_bytes());
        bytes.extend_from_slice(&45u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&cd_size.to_le_bytes());
        bytes.extend_from_slice(&cd_offset.to_le_bytes());
        bytes.extend_from_slice(b"PK\x06\x07");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&zip64_pos.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(b"PK\x05\x06");
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&u16::MAX.to_le_bytes());
        bytes.extend_from_slice(&u16::MAX.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        preflight(&mut Cursor::new(bytes.clone())).unwrap();

        let mut bad_framing = bytes.clone();
        bad_framing[zip64_pos as usize + 4..zip64_pos as usize + 12]
            .copy_from_slice(&45u64.to_le_bytes());
        assert_eq!(
            preflight(&mut Cursor::new(bad_framing))
                .unwrap_err()
                .to_string(),
            "zip_metadata_limit"
        );

        let eocd = bytes.len() - 22;
        bytes[eocd - 20 - 56 + 40..eocd - 20 - 56 + 48]
            .copy_from_slice(&(MAX_METADATA_BYTES + 1).to_le_bytes());
        assert_eq!(
            preflight(&mut Cursor::new(bytes)).unwrap_err().to_string(),
            "zip_metadata_limit"
        );
    }

    #[test]
    fn accepts_zip64_with_unsaturated_classic_fields() {
        // A real ZIP64 archive whose values fit in 32 bits: CPython (and this
        // project's writer, frozen to it) writes a ZIP64 EOCD but leaves the
        // classic count/size/offset raw, never saturating to 0xFFFF/0xFFFFFFFF.
        // A central-directory offset in [2 GiB, 4 GiB) is the common case for a
        // large package. The reader must detect ZIP64 from the locator.
        let mut bytes = one_file_zip();
        let eocd = bytes.len() - 22;
        let cd_size = u32_at(&bytes[eocd..], 12) as u64;
        let cd_offset = u32_at(&bytes[eocd..], 16) as u64;
        bytes.truncate(eocd);
        let zip64_pos = bytes.len() as u64;
        bytes.extend_from_slice(b"PK\x06\x06");
        bytes.extend_from_slice(&44u64.to_le_bytes());
        bytes.extend_from_slice(&45u16.to_le_bytes());
        bytes.extend_from_slice(&45u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&cd_size.to_le_bytes());
        bytes.extend_from_slice(&cd_offset.to_le_bytes());
        bytes.extend_from_slice(b"PK\x06\x07");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&zip64_pos.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(b"PK\x05\x06");
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&(cd_size as u32).to_le_bytes());
        bytes.extend_from_slice(&(cd_offset as u32).to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        preflight(&mut Cursor::new(bytes)).unwrap();
    }

    #[test]
    fn rejects_oversized_directory_hidden_behind_failing_last_eocd() {
        // zip 6 accepts the final EOCD, then fails its AES central member and
        // retries the earlier EOCD. The preflight must budget that earlier one.
        let count = 65_000u16;
        let mut bytes = Vec::new();
        for index in 0..count {
            let mut header = [0u8; 46];
            header[..4].copy_from_slice(b"PK\x01\x02");
            header[4..6].copy_from_slice(&20u16.to_le_bytes());
            header[6..8].copy_from_slice(&20u16.to_le_bytes());
            header[28..30].copy_from_slice(&5u16.to_le_bytes());
            bytes.extend_from_slice(&header);
            bytes.extend_from_slice(format!("{index:05}").as_bytes());
        }
        let first_cd_size = bytes.len() as u32;
        bytes.extend_from_slice(b"PK\x05\x06");
        bytes.extend_from_slice(&[0u8; 4]);
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&first_cd_size.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        bytes.resize(bytes.len() + 70_000, 0);
        let fake_offset = bytes.len() as u32;
        let mut fake_header = [0u8; 46];
        fake_header[..4].copy_from_slice(b"PK\x01\x02");
        fake_header[4..6].copy_from_slice(&20u16.to_le_bytes());
        fake_header[6..8].copy_from_slice(&20u16.to_le_bytes());
        fake_header[10..12].copy_from_slice(&99u16.to_le_bytes()); // AES without extra
        fake_header[28..30].copy_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&fake_header);
        bytes.extend_from_slice(b"trap");
        bytes.extend_from_slice(b"PK\x05\x06");
        bytes.extend_from_slice(&[0u8; 4]);
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&50u32.to_le_bytes());
        bytes.extend_from_slice(&fake_offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        assert_eq!(archive.len(), count as usize, "zip 6 must really fall back");
        assert_eq!(
            preflight(&mut Cursor::new(&bytes)).unwrap_err().to_string(),
            "zip_metadata_limit"
        );
    }

    #[test]
    fn scans_eocd_across_chunk_boundary_with_trailing_bytes() {
        let mut bytes = one_file_zip();
        // The EOCD signature crosses the boundary between backward scan chunks.
        bytes.resize(bytes.len() + 1024 * 1024 - 20, 0);
        assert_eq!(zip::ZipArchive::new(Cursor::new(&bytes)).unwrap().len(), 1);
        preflight(&mut Cursor::new(bytes)).unwrap();
    }

    #[test]
    fn rejects_size_framing_lie_even_when_zip_crate_accepts_it() {
        let mut bytes = one_file_zip();
        let eocd = bytes.len() - 22;
        bytes[eocd + 12..eocd + 16].copy_from_slice(&0u32.to_le_bytes());
        // zip 6 ignores declared CD size while reading central entries.
        assert_eq!(zip::ZipArchive::new(Cursor::new(&bytes)).unwrap().len(), 1);
        assert_eq!(
            preflight(&mut Cursor::new(bytes)).unwrap_err().to_string(),
            "zip_metadata_invalid"
        );
    }
}

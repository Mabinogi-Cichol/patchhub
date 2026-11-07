//! Static Setup Factory member reader and streaming decoder.
//! File table framing derived from CybercentreCanada/sfextract
//! 21f5c5595a70e9284fff83476c0e307d5b92f573 (licenses/sfextract-MIT.txt).
use crate::{require, Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read, Write};
pub const SIGNATURE: [u8; 16] = [
    0xe0, 0xe0, 0xe1, 0xe1, 0xe2, 0xe2, 0xe3, 0xe3, 0xe4, 0xe4, 0xe5, 0xe5, 0xe6, 0xe6, 0xe7, 0xe7,
];
const CHUNK: usize = 64 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub path: String,
    pub size: u64,
    pub crc32: u32,
}
struct Payload {
    offset: usize,
    packed: u64,
    compressed: bool,
}
pub struct Archive {
    members: Vec<Member>,
    payloads: Vec<Payload>,
}
impl Archive {
    pub fn members(&self) -> &[Member] {
        &self.members
    }
    pub fn extract(&self, data: &[u8], index: usize, sink: &mut dyn Write) -> Result<PayloadInfo> {
        let member = self
            .members
            .get(index)
            .ok_or_else(|| Error("invalid_member_index".into()))?;
        let payload = &self.payloads[index];
        decode(
            data,
            payload.offset,
            payload.packed,
            Some(member.size),
            Some(member.crc32),
            payload.compressed,
            sink,
        )
    }
}
/// Remove the known application-root marker; other relative directories remain
/// literal paths. The destination writer enforces its filesystem boundary.
pub fn relative_path(directory: &str, basename: &str) -> Result<String> {
    let directory = directory.replace('\\', "/");
    let directory = if directory == "%AppFolder%" {
        ""
    } else {
        directory.strip_prefix("%AppFolder%/").unwrap_or(&directory)
    };
    let name = if directory.is_empty() {
        basename.to_owned()
    } else {
        format!("{directory}/{basename}")
    };
    let name = name.replace('\\', "/");
    require(
        !name.is_empty()
            && !name.starts_with('/')
            && !name.contains('\0')
            && !name.split('/').any(|part| part == "..")
            && name.as_bytes().get(1).is_none_or(|b| *b != b':'),
        "unsafe_target_path",
    )?;
    Ok(name)
}
pub struct Reader<'a> {
    data: &'a [u8],
    pub pos: usize,
    end: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            end: data.len(),
        }
    }
    pub fn at(data: &'a [u8], start: usize, end: usize) -> Result<Self> {
        require(start <= end && end <= data.len(), "invalid_range")?;
        Ok(Self {
            data,
            pos: start,
            end,
        })
    }
    pub fn read(&mut self, size: usize) -> Result<&'a [u8]> {
        require(
            size <= self.end.saturating_sub(self.pos),
            "truncated_record",
        )?;
        let start = self.pos;
        self.pos += size;
        Ok(&self.data[start..self.pos])
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.read(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.read(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.read(8)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.read(8)?.try_into().unwrap()))
    }
    fn string_bytes(&mut self) -> Result<(&'a [u8], bool)> {
        let mut length = self.u8()? as usize;
        let mut wide = false;
        if length == 0xff {
            length = self.u16()? as usize;
            if length == 0xfffe {
                wide = true;
                length = self.u8()? as usize;
                if length == 0xff {
                    length = self.u16()? as usize;
                }
            }
            if length == 0xffff {
                length = self.u32()? as usize;
                require(length != u32::MAX as usize, "unsupported_string_length")?;
            }
        }
        let size = length
            .checked_mul(if wide { 2 } else { 1 })
            .ok_or_else(|| Error("string_size_overflow".into()))?;
        Ok((self.read(size)?, wide))
    }
    pub fn skip_string(&mut self) -> Result<()> {
        self.string_bytes().map(|_| ())
    }
    pub fn string(&mut self) -> Result<String> {
        let (data, wide) = self.string_bytes()?;
        let value = if wide {
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            String::from_utf16(&units).map_err(|_| Error("invalid_utf16_name".into()))?
        } else if let Ok(value) = std::str::from_utf8(data) {
            value.to_owned()
        } else {
            // SF ANSI strings carry no code-page tag. Existing UTF-8 packages
            // remain UTF-8; otherwise use strict CP936, as used by the Chinese
            // publisher samples. Never replace undecodable filename bytes.
            encoding_rs::GBK
                .decode_without_bom_handling_and_without_replacement(data)
                .ok_or_else(|| Error("invalid_ansi_name".into()))?
                .into_owned()
        };
        let value = value.trim_end_matches('\0').to_owned();
        require(!value.contains('\0'), "embedded_nul")?;
        Ok(value)
    }
}
fn at_u32(data: &[u8], at: usize) -> Result<u32> {
    Reader::at(data, at, data.len())?.u32()
}
type PackedSizes = Vec<(u64, bool)>;
fn file_table(metadata: &[u8], version: &str) -> Result<(Vec<Member>, PackedSizes)> {
    let marker = b"\xff\xff\x01\x00\x0e\x00CSetupFileData";
    let mut found = metadata
        .windows(marker.len())
        .enumerate()
        .filter(|(_, b)| *b == marker)
        .map(|(i, _)| i);
    let start = found
        .next()
        .filter(|i| *i >= 2)
        .ok_or_else(|| Error("missing_file_table".into()))?
        - 2;
    require(found.next().is_none(), "ambiguous_file_table")?;
    let mut r = Reader::at(metadata, start, metadata.len())?;
    let count = r.u16()? as usize;
    require(count > 0, "no_file_members")?;
    r.read(marker.len() + 5)?;
    let mut members = Vec::with_capacity(count);
    let mut sizes = Vec::with_capacity(count);
    for index in 0..count {
        r.skip_string()?; // author source path
        let basename = r.string()?;
        for _ in 0..4 {
            r.skip_string()?;
        }
        r.read(2)?;
        let unpacked = r.i64()?;
        require(unpacked >= 0, "invalid_unpacked_size")?;
        r.read(1 + 4 + 8 + 16 + 8 + 25)?;
        let directory = r.string()?;
        r.read(11)?;
        for _ in 0..5 {
            r.skip_string()?;
        }
        r.read(1)?;
        r.skip_string()?;
        r.read(8)?;
        r.skip_string()?;
        r.read(if version == "9.7" { 4 } else { 3 })?;
        let compressed = r.u8()?;
        require(compressed <= 1, "invalid_compression_flag")?;
        r.read(2 + 10)?;
        let wide_size = r.u16()? as usize * 2;
        r.read(wide_size)?;
        r.skip_string()?;
        r.read(2)?;
        r.skip_string()?;
        r.skip_string()?;
        let packages = r.u16()?;
        for _ in 0..packages {
            r.skip_string()?;
        }
        r.skip_string()?;
        let packed = r.i64()?;
        let crc32 = r.u32()?;
        require(packed >= 0, "invalid_packed_size")?;
        members.push(Member {
            path: relative_path(&directory, &basename)?,
            size: unpacked as u64,
            crc32,
        });
        sizes.push((packed as u64, compressed != 0));
        if index + 1 < count {
            r.read(8)?;
        }
    }
    Ok((members, sizes))
}
#[derive(Clone, Copy)]
struct Section {
    rva: u64,
    raw_offset: usize,
    raw_size: usize,
}
fn resource_resolve(sections: &[Section], rva: u32, size: u32) -> Result<usize> {
    let rva = rva as u64;
    let size = size as u64;
    let matches: Vec<_> = sections
        .iter()
        .filter(|s| s.rva <= rva && rva + size <= s.rva + s.raw_size as u64)
        .map(|s| s.raw_offset + (rva - s.rva) as usize)
        .collect();
    require(matches.len() == 1, "invalid_resource_rva")?;
    Ok(matches[0])
}
fn resource_directory(
    data: &[u8],
    resource: usize,
    size: usize,
    relative: u32,
) -> Result<Vec<(u32, u32)>> {
    let mut r = Reader::at(data, resource + relative as usize, resource + size)?;
    r.read(12)?;
    let count = r.u16()? as usize + r.u16()? as usize;
    (0..count).map(|_| Ok((r.u32()?, r.u32()?))).collect()
}
fn manifest_version(manifest: &[u8]) -> Result<String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(manifest);
    let mut depth = 0usize;
    loop {
        let event = reader
            .read_event()
            .map_err(|_| Error("invalid_manifest_xml".into()))?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e)
                if depth == 1 && e.local_name().as_ref() == b"assemblyIdentity" =>
            {
                for attr in e.attributes() {
                    let attr = attr.map_err(|_| Error("invalid_manifest_xml".into()))?;
                    if attr.key.as_ref() == b"version" {
                        let version = attr
                            .decode_and_unescape_value(reader.decoder())
                            .map_err(|_| Error("invalid_manifest_xml".into()))?
                            .into_owned();
                        let parts: Vec<_> = version.split('.').collect();
                        require(
                            parts.len() == 4
                                && parts.iter().all(|p| {
                                    !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())
                                })
                                && parts[0] == "9"
                                && ["5", "6", "7"].contains(&parts[1]),
                            "unsupported_installer_version",
                        )?;
                        return Ok(format!("{}.{}", parts[0], parts[1]));
                    }
                }
            }
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => break,
            _ => {}
        }
    }
    Err(Error("unsupported_installer_version".into()))
}
fn pe_overlay(data: &[u8]) -> Result<(usize, usize, String, bool)> {
    require(data.len() >= 64 && &data[..2] == b"MZ", "not_pe")?;
    let pe = at_u32(data, 60)? as usize;
    let mut h = Reader::at(data, pe, data.len())?;
    require(h.read(4)? == b"PE\0\0", "not_pe")?;
    h.read(2)?;
    let count = h.u16()? as usize;
    h.read(12)?;
    let optional_size = h.u16()? as usize;
    h.read(2)?;
    require(count > 0 && optional_size >= 96, "invalid_pe_header")?;
    let optional_start = h.pos;
    let mut optional = Reader::at(data, optional_start, optional_start + optional_size)?;
    let magic = optional.u16()?;
    require(
        [0x10b, 0x20b].contains(&magic),
        "invalid_pe_optional_header",
    )?;
    let directory_offset = if magic == 0x10b { 96 } else { 112 };
    require(
        optional_size >= directory_offset + 40,
        "invalid_pe_directories",
    )?;
    let number_dirs = at_u32(data, optional_start + directory_offset - 4)?;
    require(number_dirs >= 5, "invalid_pe_directories")?;
    let size_headers = at_u32(data, optional_start + 60)? as usize;
    let mut table = Reader::at(data, optional_start + optional_size, data.len())?;
    require(
        size_headers >= table.pos + count * 40 && size_headers <= data.len(),
        "invalid_pe_headers_size",
    )?;
    let mut sections = Vec::with_capacity(count);
    for _ in 0..count {
        let record = table.read(40)?;
        let mut r = Reader::at(record, 8, 40)?;
        r.u32()?;
        let rva = r.u32()? as u64;
        let raw_size = r.u32()? as usize;
        let raw_offset = r.u32()? as usize;
        require(
            raw_offset as u64 + raw_size as u64 <= data.len() as u64,
            "truncated_pe_section",
        )?;
        require(
            raw_size == 0 || raw_offset >= size_headers,
            "overlapping_pe_headers",
        )?;
        sections.push(Section {
            rva,
            raw_size,
            raw_offset,
        });
    }
    let mut intervals: Vec<_> = sections
        .iter()
        .filter(|s| s.raw_size > 0)
        .map(|s| (s.raw_offset, s.raw_offset + s.raw_size))
        .collect();
    intervals.sort();
    require(
        intervals.windows(2).all(|a| a[0].1 <= a[1].0),
        "overlapping_pe_sections",
    )?;
    let overlay = sections
        .iter()
        .map(|s| s.raw_offset + s.raw_size)
        .chain([size_headers])
        .max()
        .unwrap();
    let resource_rva = at_u32(data, optional_start + directory_offset + 16)?;
    let resource_size = at_u32(data, optional_start + directory_offset + 20)?;
    let cert_offset = at_u32(data, optional_start + directory_offset + 32)? as usize;
    let cert_size = at_u32(data, optional_start + directory_offset + 36)? as usize;
    let mut certificate = false;
    let mut overlay_end = data.len();
    if cert_offset != 0 || cert_size != 0 {
        require(
            cert_offset >= overlay
                && cert_offset.is_multiple_of(8)
                && cert_size >= 8
                && cert_offset as u64 + cert_size as u64 == data.len() as u64,
            "invalid_pe_certificate_range",
        )?;
        // Certificate contents are not extraction inputs. Only their file
        // range separates the SF overlay from the appended signature table.
        certificate = true;
        overlay_end = cert_offset;
    }
    require(resource_size > 0, "invalid_resource_size")?;
    let resource = resource_resolve(&sections, resource_rva, resource_size)?;
    let mut versions = Vec::new();
    for (kind, node) in resource_directory(data, resource, resource_size as usize, 0)? {
        if kind != 24 {
            continue;
        }
        require(node & 0x80000000 != 0, "invalid_manifest_directory")?;
        for (_, language) in
            resource_directory(data, resource, resource_size as usize, node & 0x7fffffff)?
        {
            require(language & 0x80000000 != 0, "invalid_manifest_directory")?;
            for (_, leaf) in resource_directory(
                data,
                resource,
                resource_size as usize,
                language & 0x7fffffff,
            )? {
                require(leaf & 0x80000000 == 0, "invalid_manifest_leaf")?;
                let mut entry = Reader::at(
                    data,
                    resource + leaf as usize,
                    resource + resource_size as usize,
                )?;
                let rva = entry.u32()?;
                let size = entry.u32()?;
                require(size > 0, "manifest_size_limit")?;
                let offset = resource_resolve(&sections, rva, size)?;
                versions.push(manifest_version(&data[offset..offset + size as usize])?);
            }
        }
    }
    let version = versions
        .first()
        .ok_or_else(|| Error("missing_manifest_version".into()))?
        .to_owned();
    require(
        versions.iter().all(|v| *v == version),
        "ambiguous_manifest_version",
    )?;
    require(
        data.get(overlay..overlay + 16) == Some(SIGNATURE.as_slice()),
        "unsupported_overlay",
    )?;
    Ok((overlay, overlay_end, version, certificate))
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut v = i as u32;
        let mut bit = 0;
        while bit < 8 {
            v = if v & 1 != 0 {
                0xedb88320 ^ (v >> 1)
            } else {
                v >> 1
            };
            bit += 1;
        }
        table[i] = v;
        i += 1;
    }
    table
}
const CRC_TABLE: [u32; 256] = crc_table();
fn crc_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for b in bytes {
        crc = CRC_TABLE[((crc ^ *b as u32) & 255) as usize] ^ (crc >> 8);
    }
    crc
}
#[derive(Debug, Serialize, Deserialize)]
pub struct PayloadInfo {
    pub size: u64,
    pub crc32: u32,
    pub sha256: String,
}
#[allow(clippy::too_many_arguments)]
fn decode(
    data: &[u8],
    offset: usize,
    packed: u64,
    expected: Option<u64>,
    crc: Option<u32>,
    compressed: bool,
    sink: &mut dyn Write,
) -> Result<PayloadInfo> {
    require(
        offset <= data.len() && packed <= data.len().saturating_sub(offset) as u64,
        "truncated_payload",
    )?;
    let bytes = &data[offset..offset + packed as usize];
    let mut expected = expected;
    let mut input = Cursor::new(bytes);
    let mut lzma = None;
    if compressed {
        require(packed >= 10, "truncated_lzma2_header")?;
        let prop = bytes[0];
        require(prop <= 40, "unsupported_compression")?;
        let dict = if prop < 40 {
            (2u32 | (prop as u32 & 1)) << (prop as u32 / 2 + 11)
        } else {
            u32::MAX
        };
        let header = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
        require(expected.is_none_or(|n| n == header), "lzma2_size_mismatch")?;
        expected = Some(header);
        input.set_position(9);
        lzma = Some(lzma_rust2::Lzma2Reader::new(input, dict, None));
        input = Cursor::new(&[]);
    }
    let mut digest = Sha256::new();
    let mut checksum = u32::MAX;
    let mut size = 0u64;
    let mut chunk = [0u8; CHUNK];
    loop {
        let available = expected
            .map_or(CHUNK as u64, |n| n.saturating_sub(size).saturating_add(1))
            .min(CHUNK as u64) as usize;
        let count = if let Some(decoder) = lzma.as_mut() {
            decoder.read(&mut chunk[..available]).map_err(|e| {
                Error(
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        "truncated_lzma2_stream"
                    } else {
                        "invalid_lzma2_stream"
                    }
                    .into(),
                )
            })?
        } else {
            input.read(&mut chunk[..available])?
        };
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| Error("payload_size_overflow".into()))?;
        require(expected.is_none_or(|n| size <= n), "payload_size_mismatch")?;
        digest.update(&chunk[..count]);
        checksum = crc_update(checksum, &chunk[..count]);
        sink.write_all(&chunk[..count])?;
    }
    if let Some(decoder) = lzma {
        require(
            decoder.into_inner().position() == packed,
            "trailing_compressed_data",
        )?;
    }
    require(expected.is_none_or(|n| size == n), "payload_size_mismatch")?;
    let checksum = !checksum;
    require(crc.is_none_or(|n| n == checksum), "payload_crc_mismatch")?;
    Ok(PayloadInfo {
        size,
        crc32: checksum,
        sha256: format!("{:x}", digest.finalize()),
    })
}
pub fn read(data: &[u8]) -> Result<Archive> {
    let (overlay, end, version, certificate) = pe_overlay(data)?;
    let mut r = Reader::at(data, overlay, end)?;
    r.read(16 + 10)?;
    // Stored runtime executables are container resources, never game members.
    for _ in 0..2 {
        let size = r.i64()?;
        require(size >= 0, "invalid_runtime_size")?;
        r.read(usize::try_from(size).map_err(|_| Error("invalid_runtime_size".into()))?)?;
    }
    let count = r.u32()?;
    let mut metadata = None;
    for _ in 0..count {
        let raw_name = r.read(264)?;
        let packed = r.i64()?;
        let crc = r.u32()?;
        r.read(4)?;
        require(packed >= 0, "invalid_packed_size")?;
        let packed = packed as u64;
        let offset = r.pos;
        r.read(usize::try_from(packed).map_err(|_| Error("payload_size_overflow".into()))?)?;
        if raw_name.split(|b| *b == 0).next() == Some(b"irsetup.dat".as_slice()) {
            require(metadata.is_none(), "ambiguous_metadata")?;
            let mut content = Vec::new();
            decode(data, offset, packed, None, Some(crc), true, &mut content)?;
            metadata = Some(content);
        }
    }
    let metadata = metadata.ok_or_else(|| Error("missing_metadata".into()))?;
    let (members, sizes) = file_table(&metadata, &version)?;
    let mut payloads = Vec::with_capacity(members.len());
    for (packed, compressed) in sizes {
        payloads.push(Payload {
            offset: r.pos,
            packed,
            compressed,
        });
        r.read(usize::try_from(packed).map_err(|_| Error("payload_size_overflow".into()))?)?;
    }
    let padding = if certificate { (8 - r.pos % 8) % 8 } else { 0 };
    require(
        r.read(padding)?.iter().all(|b| *b == 0) && r.pos == end,
        "overlay_size_or_tail_mismatch",
    )?;
    Ok(Archive { members, payloads })
}

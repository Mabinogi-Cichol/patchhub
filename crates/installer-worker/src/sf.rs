//! Fixed-action SF worker protocol. The caller must launch this code in the platform sandbox.
//! This module never authorizes installation or opens a game/state directory.
use memmap2::MmapOptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::Path;
use steam_patch_installer_format::{audit, require, Error, Result};

pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "action", deny_unknown_fields)]
pub enum Descriptor {
    #[serde(rename = "analyze")]
    Analyze { exe_sha256: String, exe_size: u64 },
    #[serde(rename = "analyze_behavior")]
    AnalyzeBehavior { exe_sha256: String, exe_size: u64 },
    #[serde(rename = "convert_behavior")]
    ConvertBehavior {
        exe_sha256: String,
        exe_size: u64,
        metadata_sha256: String,
        payload: Vec<Payload>,
        behavior_sha256: String,
    },
}
fn is_sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Descriptor {
    fn binding(&self) -> (&str, u64) {
        match self {
            Self::Analyze {
                exe_sha256,
                exe_size,
            }
            | Self::AnalyzeBehavior {
                exe_sha256,
                exe_size,
            }
            | Self::ConvertBehavior {
                exe_sha256,
                exe_size,
                ..
            } => (exe_sha256, *exe_size),
        }
    }
    fn validate(&self) -> Result<()> {
        let (hash, size) = self.binding();
        require(is_sha(hash) && size > 0, "invalid_descriptor")?;
        if let Self::ConvertBehavior {
            metadata_sha256,
            behavior_sha256,
            payload,
            ..
        } = self
        {
            require(
                is_sha(metadata_sha256)
                    && is_sha(behavior_sha256)
                    && !payload.is_empty()
                    && payload.len() <= audit::MAX_ENTRIES,
                "invalid_descriptor",
            )?;
            // Validate every declared member and keep the running total
            // overflow-checked; collection to a real artifact is bounded by
            // the actual input and disk at conversion time.
            let _total = payload.iter().try_fold(0u64, |total, p| {
                require(is_sha(&p.sha256), "invalid_descriptor")?;
                total
                    .checked_add(p.size)
                    .ok_or_else(|| Error("invalid_descriptor".into()))
            })?;
        }
        Ok(())
    }
}
pub fn parse_descriptor(raw: &[u8]) -> Result<Descriptor> {
    require(raw.len() <= MAX_MESSAGE, "descriptor_size_limit")?;
    let descriptor: Descriptor =
        serde_json::from_slice(raw).map_err(|_| Error("invalid_descriptor".into()))?;
    descriptor.validate()?;
    Ok(descriptor)
}

#[derive(PartialEq, Eq)]
struct Identity {
    len: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    native: Vec<u64>,
}
fn identity(file: &File) -> Result<Identity> {
    let m = file.metadata()?;
    #[cfg(unix)]
    let native = {
        use std::os::unix::fs::MetadataExt;
        vec![
            m.dev(),
            m.ino(),
            m.mtime() as u64,
            m.mtime_nsec() as u64,
            m.ctime() as u64,
            m.ctime_nsec() as u64,
        ]
    };
    #[cfg(not(unix))]
    let native = Vec::new();
    Ok(Identity {
        len: m.len(),
        modified: m.modified().ok(),
        created: m.created().ok(),
        native,
    })
}
fn no_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        let m = fs::symlink_metadata(ancestor)?;
        require(!m.file_type().is_symlink(), "worker_path_link")?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            require(m.file_attributes() & 0x400 == 0, "worker_path_link")?;
        }
    }
    Ok(())
}
pub fn open_input(path: &Path) -> Result<File> {
    no_links(path)?;
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.share_mode(1).custom_flags(0x00200000);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(0x20000);
    }
    let f = opts.open(path)?;
    require(f.metadata()?.is_file(), "worker_input_not_file")?;
    Ok(f)
}

pub fn encode_response(value: &Value) -> Result<Vec<u8>> {
    // No artificial response/report ceiling: the report grows with a legitimate
    // installer's file table. The worker's own sandbox memory limit bounds it,
    // and consumers bound memory by the actual document (design D08).
    let mut out = Vec::new();
    serde_json::to_writer(&mut out, value).map_err(|_| Error("invalid_report".into()))?;
    out.push(b'\n');
    Ok(out)
}
fn manifest(report: &Value) -> Result<Vec<Payload>> {
    report["files"]
        .as_array()
        .ok_or_else(|| Error("invalid_report".into()))?
        .iter()
        .map(|f| {
            Ok(Payload {
                path: string(f, "destination")?.into(),
                size: number(f, "unpacked_size")?,
                sha256: string(&f["payload"], "sha256")?.into(),
            })
        })
        .collect()
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| Error("invalid_report".into()))
}
fn number(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .ok_or_else(|| Error("invalid_report".into()))
}

/// Read-only analysis or content-bound conversion; the output directory is exclusively owned.
pub fn process(file: &File, descriptor: &Descriptor, output: &Path) -> Result<Value> {
    descriptor.validate()?;
    let before = identity(file)?;
    let (exe_sha, exe_size) = descriptor.binding();
    require(before.len == exe_size, "exe_size_mismatch")?;
    // SAFETY: the fixed input is a regular read-only sandbox file. Windows denies
    // write/delete sharing; Linux production binds it read-only. The host must keep
    // the input immutable for this invocation. Identity and digest are rechecked.
    let data = unsafe { MmapOptions::new().map(file)? };
    require(
        format!("{:x}", Sha256::digest(&data[..])) == exe_sha,
        "exe_payload_changed",
    )?;
    let behavior = !matches!(descriptor, Descriptor::Analyze { .. });
    let report = audit::audit(&data, behavior)?;
    require(report["exe"]["sha256"] == exe_sha, "exe_payload_changed")?;
    require(identity(file)? == before, "exe_changed_during_extraction")?;
    match descriptor {
        Descriptor::Analyze { .. } | Descriptor::AnalyzeBehavior { .. } => {
            let action = if behavior {
                "analyze_behavior"
            } else {
                "analyze"
            };
            let response = json!({"schema_version":1,"action":action,"exe_sha256":exe_sha,"report":report,"installable":false});
            encode_response(&response)?;
            Ok(response)
        }
        Descriptor::ConvertBehavior {
            metadata_sha256,
            payload,
            behavior_sha256,
            ..
        } => {
            require(
                report["metadata"]["sha256"] == *metadata_sha256,
                "exe_metadata_changed",
            )?;
            require(
                report["behavior_sha256"] == *behavior_sha256,
                "exe_behavior_changed",
            )?;
            let files = manifest(&report)?;
            let actual: std::collections::BTreeMap<_, _> =
                files.iter().map(|p| (p.path.as_str(), p)).collect();
            let mut wanted = payload.clone();
            wanted.sort_by(|a, b| a.path.cmp(&b.path));
            require(
                wanted.windows(2).all(|v| v[0].path != v[1].path)
                    && wanted.iter().all(|item| {
                        actual
                            .get(item.path.as_str())
                            .is_some_and(|value| *value == item)
                    }),
                "exe_manifest_changed",
            )?;
            let files = payload.clone();
            let parent = output
                .parent()
                .ok_or_else(|| Error("worker_output_invalid".into()))?;
            no_links(parent)?;
            require(parent.is_dir(), "worker_output_invalid")?;
            let partial = output.with_file_name("normalized.zip.partial");
            require(
                !output.try_exists()? && !partial.try_exists()?,
                "worker_output_exists",
            )?;
            let mut target = OpenOptions::new()
                .write(true)
                .read(true)
                .create_new(true)
                .open(&partial)?;
            let outcome = (|| {
                write_zip(&mut target, &data, &report, &files)?;
                target.sync_all()?;
                require(identity(file)? == before, "exe_changed_during_extraction")?;
                require(
                    format!("{:x}", Sha256::digest(&data[..])) == exe_sha,
                    "exe_payload_changed",
                )?;
                let size = target.metadata()?.len();
                target.rewind()?;
                let mut hash = Sha256::new();
                let mut buf = [0u8; 64 * 1024];
                loop {
                    let count = target.read(&mut buf)?;
                    if count == 0 {
                        break;
                    }
                    hash.update(&buf[..count]);
                }
                let response = json!({"extractor":audit::VERSION,"exe_sha256":exe_sha,"metadata_sha256":metadata_sha256,
                    "artifact_sha256":format!("{:x}",hash.finalize()),"artifact_size":size,"files":files});
                encode_response(&response)?;
                // Atomic no-clobber publication on the same volume. A late competing
                // output cannot be overwritten, unlike Unix rename().
                fs::hard_link(&partial, output)?;
                Ok(response)
            })();
            drop(target);
            let cleanup = fs::remove_file(&partial);
            match (outcome, cleanup) {
                (Ok(v), Ok(())) => Ok(v),
                (Err(e), _) => Err(e),
                (Ok(_), Err(e)) => {
                    let _ = fs::remove_file(output);
                    Err(e.into())
                }
            }
        }
    }
}

fn u16le(w: &mut impl Write, n: u16) -> Result<()> {
    w.write_all(&n.to_le_bytes())?;
    Ok(())
}
fn u32le(w: &mut impl Write, n: u32) -> Result<()> {
    w.write_all(&n.to_le_bytes())?;
    Ok(())
}
fn u64le(w: &mut impl Write, n: u64) -> Result<()> {
    w.write_all(&n.to_le_bytes())?;
    Ok(())
}
// CPython starts ZIP64 central-directory/EOCD framing above 2^31-1,
// even though the classic ZIP fields can represent up to 2^32-1.
const PY_ZIP64_LIMIT: u64 = (1 << 31) - 1;

fn write_central_record(
    w: &mut impl Write,
    name: &[u8],
    flags: u16,
    size: u64,
    crc: u32,
    start: u64,
) -> Result<()> {
    let mut extra = Vec::new();
    let stored_size = if size > PY_ZIP64_LIMIT {
        u64le(&mut extra, size)?;
        u64le(&mut extra, size)?;
        u32::MAX
    } else {
        size as u32
    };
    let stored_start = if start > PY_ZIP64_LIMIT {
        u64le(&mut extra, start)?;
        u32::MAX
    } else {
        start as u32
    };
    u32le(w, 0x02014b50)?;
    u16le(w, 0x32d)?;
    u16le(w, 45)?;
    u16le(w, flags)?;
    u16le(w, 0)?;
    u16le(w, 0)?;
    u16le(w, 33)?;
    u32le(w, crc)?;
    u32le(w, stored_size)?;
    u32le(w, stored_size)?;
    u16le(w, name.len() as u16)?;
    u16le(
        w,
        if extra.is_empty() {
            0
        } else {
            4 + extra.len() as u16
        },
    )?;
    for _ in 0..3 {
        u16le(w, 0)?;
    }
    u32le(w, 0o100644 << 16)?;
    u32le(w, stored_start)?;
    w.write_all(name)?;
    if !extra.is_empty() {
        u16le(w, 1)?;
        u16le(w, extra.len() as u16)?;
        w.write_all(&extra)?;
    }
    Ok(())
}

fn write_end_records(w: &mut impl Write, count: u64, size: u64, start: u64) -> Result<()> {
    if start > PY_ZIP64_LIMIT || size > PY_ZIP64_LIMIT || count >= u16::MAX as u64 {
        u32le(w, 0x06064b50)?;
        u64le(w, 44)?;
        u16le(w, 45)?;
        u16le(w, 45)?;
        u32le(w, 0)?;
        u32le(w, 0)?;
        u64le(w, count as u64)?;
        u64le(w, count as u64)?;
        u64le(w, size)?;
        u64le(w, start)?;
        u32le(w, 0x07064b50)?;
        u32le(w, 0)?;
        u64le(
            w,
            start
                .checked_add(size)
                .ok_or_else(|| Error("normalized_zip_limit".into()))?,
        )?;
        u32le(w, 1)?;
    }
    u32le(w, 0x06054b50)?;
    u16le(w, 0)?;
    u16le(w, 0)?;
    u16le(w, count.min(u16::MAX as u64) as u16)?;
    u16le(w, count.min(u16::MAX as u64) as u16)?;
    u32le(w, size.min(u32::MAX as u64) as u32)?;
    u32le(w, start.min(u32::MAX as u64) as u32)?;
    u16le(w, 0)?;
    Ok(())
}

// Match the frozen CPython 3.12 ZIP_STORED / force_zip64 bytes exactly.
fn write_zip(w: &mut File, data: &[u8], report: &Value, selected: &[Payload]) -> Result<()> {
    let selected_paths: std::collections::BTreeSet<_> =
        selected.iter().map(|p| p.path.as_str()).collect();
    let mut rows: Vec<&Value> = report["files"]
        .as_array()
        .ok_or_else(|| Error("invalid_report".into()))?
        .iter()
        .filter(|row| {
            row["destination"]
                .as_str()
                .is_some_and(|p| selected_paths.contains(p))
        })
        .collect();
    rows.sort_by(|a, b| a["destination"].as_str().cmp(&b["destination"].as_str()));
    let mut central = Vec::new();
    for row in &rows {
        let name = string(row, "destination")?.as_bytes();
        require(name.len() <= u16::MAX as usize, "normalized_zip_path_limit")?;
        let flags = if name.is_ascii() { 0 } else { 0x800 };
        let size = number(row, "unpacked_size")?;
        let crc = u32::try_from(number(row, "crc32")?)
            .map_err(|_| Error("invalid_report".into()))?;
        let start = w.stream_position()?;

        u32le(w, 0x04034b50)?;
        u16le(w, 45)?;
        u16le(w, flags)?;
        u16le(w, 0)?;
        u16le(w, 0)?;
        u16le(w, 33)?;
        u32le(w, crc)?;
        u32le(w, u32::MAX)?;
        u32le(w, u32::MAX)?;
        u16le(w, name.len() as u16)?;
        u16le(w, 20)?;
        w.write_all(name)?;
        u16le(w, 1)?;
        u16le(w, 16)?;
        u64le(w, size)?;
        u64le(w, size)?;
        let info = audit::decode_payload(
            data,
            usize::try_from(number(row, "payload_offset")?)
                .map_err(|_| Error("invalid_report".into()))?,
            number(row, "packed_size")?,
            size,
            crc,
            row["compressed"]
                .as_bool()
                .ok_or_else(|| Error("invalid_report".into()))?,
            Some(w),
        )?;
        require(info == row["payload"], "exe_payload_changed")?;
        write_central_record(&mut central, name, flags, size, crc, start)?;
    }
    let start = w.stream_position()?;
    w.write_all(&central)?;
    write_end_records(w, rows.len() as u64, central.len() as u64, start)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_is_strict_and_cannot_grant_authority() {
        let sha = "a".repeat(64);
        let valid = json!({"action":"analyze_behavior","exe_sha256":sha,"exe_size":10});
        assert!(parse_descriptor(&serde_json::to_vec(&valid).unwrap()).is_ok());
        for key in [
            "semantics_approved",
            "input_path",
            "output_path",
            "command",
            "installable",
        ] {
            let mut changed = valid.clone();
            changed[key] = json!(true);
            assert_eq!(
                parse_descriptor(&serde_json::to_vec(&changed).unwrap())
                    .unwrap_err()
                    .0,
                "invalid_descriptor"
            );
        }
        for action in ["convert", "execute", "apply", ""] {
            let mut changed = valid.clone();
            changed["action"] = json!(action);
            assert!(parse_descriptor(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
        for size in [json!(-1), json!(1.5), json!(true), json!(0)] {
            let mut changed = valid.clone();
            changed["exe_size"] = size;
            assert!(parse_descriptor(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
        let duplicate =
            format!(r#"{{"action":"analyze","exe_sha256":"{sha}","exe_size":1,"exe_size":2}}"#);
        assert!(parse_descriptor(duplicate.as_bytes()).is_err());
        assert_eq!(
            parse_descriptor(&vec![b' '; MAX_MESSAGE + 1])
                .unwrap_err()
                .0,
            "descriptor_size_limit"
        );
    }
    #[test]
    fn zip64_boundaries_match_frozen_cpython_headers() {
        let golden: Value = serde_json::from_str(include_str!(
            "../../installer-format/tests/fixtures/sf/zip64-boundary-golden.json"
        ))
        .unwrap();
        for case in golden["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap().as_bytes();
            let flags = if name.is_ascii() { 0 } else { 0x800 };
            let mut central = Vec::new();
            write_central_record(
                &mut central,
                name,
                flags,
                case["size"].as_u64().unwrap(),
                0x12345678,
                case["header_offset"].as_u64().unwrap(),
            )
            .unwrap();
            let mut actual = central.clone();
            write_end_records(
                &mut actual,
                1,
                central.len() as u64,
                case["central_offset"].as_u64().unwrap(),
            )
            .unwrap();
            let expected: Vec<u8> = case["expected_hex"]
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect();
            assert_eq!(actual, expected, "{}", case["id"]);
        }
    }
    #[test]
    fn response_has_no_artificial_size_ceiling() {
        let small = encode_response(&json!({"installable":false})).unwrap();
        assert_eq!(small.last(), Some(&b'\n'));
        // A report larger than the descriptor/protocol message bound is allowed;
        // the report grows with a legitimate installer's file table.
        let big = encode_response(&json!("x".repeat(MAX_MESSAGE))).unwrap();
        assert!(big.len() > MAX_MESSAGE);
        assert_eq!(big.last(), Some(&b'\n'));
    }
    #[test]
    fn zip64_records_keep_full_offsets_sizes_and_member_counts() {
        let mut central = Vec::new();
        let size = 5u64 * 1024 * 1024 * 1024;
        let offset = size + 123;
        write_central_record(&mut central, b"file.dat", 0, size, 0, offset).unwrap();
        assert!(central.windows(8).any(|v| v == offset.to_le_bytes()));
        // Both the classic compressed/uncompressed fields saturate to
        // u32::MAX and the ZIP64 extra carries the nonzero size twice before
        // the relative offset (uncompressed, compressed, offset order).
        assert_eq!(&central[20..28], &[0xff; 8]);
        let mut extra = Vec::new();
        u64le(&mut extra, size).unwrap();
        u64le(&mut extra, size).unwrap();
        u64le(&mut extra, offset).unwrap();
        assert!(central.ends_with(&extra), "size twice then offset");
        let mut footer = Vec::new();
        write_end_records(&mut footer, 70_000, 1024, offset).unwrap();
        assert_eq!(&footer[..4], &0x06064b50u32.to_le_bytes());
        assert_eq!(
            u64::from_le_bytes(footer[24..32].try_into().unwrap()),
            70_000
        );
        assert_eq!(
            u64::from_le_bytes(footer[48..56].try_into().unwrap()),
            offset
        );
        assert_eq!(footer.len(), 98);
    }

    #[test]
    fn zip64_end_record_triggers_at_member_count_boundary() {
        // The classic ZIP32 EOCD holds at most u16::MAX - 1 members, so the
        // writer must emit ZIP64 EOCD + locator as soon as count == u16::MAX.
        let mut below = Vec::new();
        write_end_records(&mut below, u16::MAX as u64 - 1, 1024, 0).unwrap();
        assert_eq!(&below[..4], &0x06054b50u32.to_le_bytes(), "plain EOCD");
        assert_eq!(below.len(), 22);

        let mut at = Vec::new();
        write_end_records(&mut at, u16::MAX as u64, 1024, 0).unwrap();
        assert_eq!(&at[..4], &0x06064b50u32.to_le_bytes(), "ZIP64 EOCD");
        assert_eq!(
            u64::from_le_bytes(at[24..32].try_into().unwrap()),
            u16::MAX as u64
        );
        // The classic EOCD count is saturated (not truncated) and sits last.
        let eocd = &at[at.len() - 22..];
        assert_eq!(&eocd[..4], &0x06054b50u32.to_le_bytes());
        assert_eq!(u16::from_le_bytes(eocd[8..10].try_into().unwrap()), u16::MAX);
        assert_eq!(at.len(), 98);
    }

    #[test]
    fn descriptor_accepts_large_representable_exe_without_allocating_its_contents() {
        let bytes = serde_json::to_vec(&json!({"action":"analyze_behavior",
            "exe_sha256":"a".repeat(64),"exe_size":5u64*1024*1024*1024}))
        .unwrap();
        assert!(parse_descriptor(&bytes).is_ok());
    }
}

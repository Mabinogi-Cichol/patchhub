//! Fixed-action SF worker protocol. The caller must launch this code in the platform sandbox.
//! This module never authorizes installation or opens a game/state directory.
use memmap2::MmapOptions;
use patchhub_installer_format::{audit, require, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub path: String,
    pub size: u64,
    pub crc32: u32,
    pub payload_offset: u64,
    pub packed_size: u64,
    pub compressed: bool,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "action", deny_unknown_fields)]
pub enum Descriptor {
    #[serde(rename = "analyze")]
    Analyze { exe_sha256: String, exe_size: u64 },
    #[serde(rename = "analyze_behavior")]
    AnalyzeBehavior { exe_sha256: String, exe_size: u64 },
    #[serde(rename = "extract_behavior")]
    ExtractBehavior {
        exe_sha256: String,
        exe_size: u64,
        payload: Vec<Member>,
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
            | Self::ExtractBehavior {
                exe_sha256,
                exe_size,
                ..
            } => (exe_sha256, *exe_size),
        }
    }
    fn validate(&self) -> Result<()> {
        let (hash, size) = self.binding();
        require(is_sha(hash) && size > 0, "invalid_descriptor")?;
        if let Self::ExtractBehavior { payload, .. } = self {
            require(!payload.is_empty(), "invalid_descriptor")?;
            // Validate every declared member and keep the running total
            // overflow-checked. Each payload range must lie inside the input.
            let mut total = 0u64;
            for p in payload {
                require(
                    !p.path.is_empty()
                        && p.payload_offset
                            .checked_add(p.packed_size)
                            .is_some_and(|end| end <= size),
                    "invalid_descriptor",
                )?;
                total = total
                    .checked_add(p.size)
                    .ok_or_else(|| Error("invalid_descriptor".into()))?;
            }
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
/// Read-only analysis. Payload bytes are decoded only by [`extract`] during the
/// install phase, never here to precompute member digests.
pub fn process(file: &File, descriptor: &Descriptor) -> Result<Value> {
    descriptor.validate()?;
    require(
        matches!(
            descriptor,
            Descriptor::Analyze { .. } | Descriptor::AnalyzeBehavior { .. }
        ),
        "invalid_descriptor",
    )?;
    let before = identity(file)?;
    let (exe_sha, exe_size) = descriptor.binding();
    require(before.len == exe_size, "exe_size_mismatch")?;
    // SAFETY: the fixed input is a regular read-only sandbox file. Windows denies
    // write/delete sharing; Linux production binds it read-only. The host must keep
    // the input immutable for this invocation. Identity and digest are rechecked.
    let data = unsafe { MmapOptions::new().map(file)? };
    let behavior = matches!(descriptor, Descriptor::AnalyzeBehavior { .. });
    let report = audit::audit_metadata(&data, behavior)?;
    // The metadata audit hashes the complete input once; reuse that digest
    // instead of reading the file again just to bind the descriptor.
    require(report["exe"]["sha256"] == exe_sha, "exe_payload_changed")?;
    require(identity(file)? == before, "exe_changed_during_analysis")?;
    let action = if behavior {
        "analyze_behavior"
    } else {
        "analyze"
    };
    Ok(
        json!({"schema_version":1,"action":action,"exe_sha256":exe_sha,"report":report,"installable":false}),
    )
}

fn write_line(out: &mut dyn Write, value: Value) -> Result<()> {
    serde_json::to_writer(&mut *out, &value).map_err(|_| Error("invalid_report".into()))?;
    out.write_all(b"\n")?;
    Ok(())
}

/// Stream selected members to the caller as one JSON header, exactly the
/// advertised raw bytes, and one JSON trailer per member. The host consumes the
/// bytes while the worker decodes; no artifact or member image is created.
pub fn extract(file: &File, descriptor: &Descriptor, out: &mut dyn Write) -> Result<()> {
    descriptor.validate()?;
    let Descriptor::ExtractBehavior { payload, .. } = descriptor else {
        return Err(Error("invalid_descriptor".into()));
    };
    let before = identity(file)?;
    let (exe_sha, exe_size) = descriptor.binding();
    require(before.len == exe_size, "exe_size_mismatch")?;
    // SAFETY: see process(): the sandboxed input is immutable for this session.
    let data = unsafe { MmapOptions::new().map(file)? };
    require(
        format!("{:x}", Sha256::digest(&data[..])) == exe_sha,
        "exe_payload_changed",
    )?;
    write_line(out, json!({"extract":1,"members":payload.len()}))?;
    for (index, member) in payload.iter().enumerate() {
        write_line(
            out,
            json!({"index":index,"path":member.path,"size":member.size}),
        )?;
        let info = audit::decode_payload(
            &data,
            usize::try_from(member.payload_offset)
                .map_err(|_| Error("invalid_descriptor".into()))?,
            member.packed_size,
            member.size,
            member.crc32,
            member.compressed,
            Some(&mut *out),
        )?;
        write_line(
            out,
            json!({"index":index,"size":info["size"],"crc32":info["crc32"],"sha256":info["sha256"]}),
        )?;
    }
    write_line(out, json!({"done":true}))?;
    out.flush()?;
    require(identity(file)? == before, "exe_changed_during_extraction")?;
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
    fn descriptor_accepts_large_representable_exe_without_allocating_its_contents() {
        let bytes = serde_json::to_vec(&json!({"action":"analyze_behavior",
            "exe_sha256":"a".repeat(64),"exe_size":5u64*1024*1024*1024}))
        .unwrap();
        assert!(parse_descriptor(&bytes).is_ok());
    }
}

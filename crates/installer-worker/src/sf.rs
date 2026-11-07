//! Static member listing and streaming extraction for the bundled worker.
pub use archive::Member;
use memmap2::MmapOptions;
use patchhub_installer_format::{archive, require, Error, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
#[derive(Debug, Deserialize)]
#[serde(tag = "action", deny_unknown_fields)]
pub enum Descriptor {
    #[serde(rename = "list_members")]
    ListMembers { exe_sha256: String, exe_size: u64 },
    #[serde(rename = "extract_members")]
    ExtractMembers { exe_sha256: String, exe_size: u64 },
}
impl Descriptor {
    pub fn binding(&self) -> (&str, u64) {
        match self {
            Self::ListMembers {
                exe_sha256,
                exe_size,
            }
            | Self::ExtractMembers {
                exe_sha256,
                exe_size,
            } => (exe_sha256, *exe_size),
        }
    }
    fn validate(&self) -> Result<()> {
        let (hash, size) = self.binding();
        require(
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && size > 0,
            "invalid_descriptor",
        )
    }
}
pub fn parse_descriptor(raw: &[u8]) -> Result<Descriptor> {
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
    let mut out = Vec::new();
    serde_json::to_writer(&mut out, value).map_err(|_| Error("invalid_response".into()))?;
    out.push(b'\n');
    Ok(out)
}
fn input(file: &File, descriptor: &Descriptor) -> Result<(memmap2::Mmap, Identity)> {
    descriptor.validate()?;
    let before = identity(file)?;
    let (hash, size) = descriptor.binding();
    require(before.len == size, "exe_size_mismatch")?;
    // SAFETY: a regular read-only input, held immutable by the host. Windows
    // denies write/delete sharing; identity is checked again after processing.
    let data = unsafe { MmapOptions::new().map(file)? };
    require(
        format!("{:x}", Sha256::digest(&data[..])) == hash,
        "exe_payload_changed",
    )?;
    Ok((data, before))
}
pub fn process(file: &File, descriptor: &Descriptor) -> Result<Value> {
    require(
        matches!(descriptor, Descriptor::ListMembers { .. }),
        "invalid_descriptor",
    )?;
    let (data, before) = input(file, descriptor)?;
    let archive = archive::read(&data)?;
    require(identity(file)? == before, "exe_changed_during_listing")?;
    Ok(
        json!({"schema_version":1,"action":"list_members","exe_sha256":descriptor.binding().0,"members":archive.members()}),
    )
}
fn write_line(out: &mut dyn Write, value: Value) -> Result<()> {
    serde_json::to_writer(&mut *out, &value).map_err(|_| Error("invalid_response".into()))?;
    out.write_all(b"\n")?;
    Ok(())
}
/// Every file payload in container order, including duplicate target paths.
/// Offsets and compression details never cross the worker boundary.
pub fn extract(file: &File, descriptor: &Descriptor, out: &mut dyn Write) -> Result<()> {
    require(
        matches!(descriptor, Descriptor::ExtractMembers { .. }),
        "invalid_descriptor",
    )?;
    let (data, before) = input(file, descriptor)?;
    let archive = archive::read(&data)?;
    write_line(out, json!({"extract":1,"members":archive.members().len()}))?;
    for (index, member) in archive.members().iter().enumerate() {
        write_line(
            out,
            json!({"index":index,"path":member.path,"size":member.size}),
        )?;
        let info = archive.extract(&data, index, out)?;
        write_line(
            out,
            json!({"index":index,"size":info.size,"crc32":info.crc32,"sha256":info.sha256}),
        )?;
    }
    require(identity(file)? == before, "exe_changed_during_extraction")?;
    write_line(out, json!({"done":true}))?;
    out.flush()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_only_identifies_input_and_action() {
        let valid = json!({"action":"list_members","exe_sha256":"a".repeat(64),"exe_size":10});
        assert!(parse_descriptor(&serde_json::to_vec(&valid).unwrap()).is_ok());
        for key in [
            "payload",
            "input_path",
            "output_path",
            "command",
            "installable",
        ] {
            let mut changed = valid.clone();
            changed[key] = json!(true);
            assert!(parse_descriptor(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
        for action in ["analyze", "analyze_behavior", "extract_behavior", "execute"] {
            let mut changed = valid.clone();
            changed["action"] = json!(action);
            assert!(parse_descriptor(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
    }
}

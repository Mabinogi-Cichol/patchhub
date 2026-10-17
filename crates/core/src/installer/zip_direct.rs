//! Single-pass ZIP extraction into the resolved game targets.
//!
//! Members are decoded exactly once, streamed straight to their final path and
//! digested in the same pass. There is no staging copy, no backup and no
//! post-write read-back.
use super::{invalid, zip_recipe::Member, Result};
use crate::{
    install::{self, DirectTarget},
    operation,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{BufWriter, Read, Write},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
use zip::ZipArchive;

/// Extracts every planned member and returns `(destination, sha256)` pairs for
/// the direct-install commit. Written files are never reverted on failure.
pub(crate) fn extract(
    zip: &mut ZipArchive<std::fs::File>,
    members: &[Member],
    targets: &[DirectTarget],
    cancel: &AtomicBool,
    wrote: &mut bool,
) -> Result<Vec<(String, String)>> {
    let mut installed = Vec::with_capacity(members.len());
    let mut parents: BTreeSet<PathBuf> = BTreeSet::new();
    let mut buffer = vec![0u8; 256 * 1024];
    for (at, member) in members.iter().enumerate() {
        check(cancel)?;
        let target = targets
            .get(at)
            .ok_or_else(|| invalid("zip_member_plan_changed"))?;
        if target.path != member.destination {
            return Err(invalid("zip_member_plan_changed"));
        }
        operation::file(at, &member.source);
        let mut file = zip
            .by_index(member.index)
            .map_err(|e| invalid(format!("invalid_zip:{e}")))?;
        let mut out = BufWriter::new(install::open_direct_target(&target.file)?);
        *wrote = true;
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        loop {
            check(cancel)?;
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > member.size {
                return Err(invalid("zip_member_size_mismatch"));
            }
            out.write_all(&buffer[..count])?;
            hasher.update(&buffer[..count]);
            operation::advance(count as u64);
        }
        if total != member.size {
            return Err(invalid("zip_member_size_mismatch"));
        }
        let file = out
            .into_inner()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        file.sync_all()?;
        let sha = format!("{:x}", hasher.finalize());
        if member
            .expected_sha256
            .as_deref()
            .is_some_and(|expected| expected != sha)
        {
            return Err(invalid("zip_member_hash_mismatch"));
        }
        if let Some(parent) = target.file.parent() {
            parents.insert(parent.to_path_buf());
        }
        installed.push((member.destination.clone(), sha));
        operation::finish_file(at + 1);
    }
    for parent in parents {
        install::sync_directory(&parent)?;
    }
    Ok(installed)
}

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(invalid("cancelled"));
    }
    match operation::check() {
        Ok(()) => Ok(()),
        Err(error) if error.to_string() == "task_cancelled" => Err(invalid("cancelled")),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::zip_recipe::{self, Member};
    use sha2::Digest;
    use std::{
        fs::{self, File},
        io::Write,
    };
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    fn archive(temp: &tempfile::TempDir, members: &[(&str, &[u8])]) -> std::path::PathBuf {
        let path = temp.path().join("patch.zip");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, data) in members {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn targets(root: &std::path::Path, members: &[Member]) -> Vec<DirectTarget> {
        members
            .iter()
            .map(|member| DirectTarget {
                path: member.destination.clone(),
                file: root.join(&member.destination),
            })
            .collect()
    }

    #[test]
    fn extracts_and_reports_the_installed_digest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir_all(&root).unwrap();
        let path = archive(&temp, &[("a.bin", b"hello")]);
        let mut zip = zip_recipe::open(&path).unwrap();
        let members = zip_recipe::plan(&mut zip, &root, &AtomicBool::new(false)).unwrap();
        let targets = targets(&root, &members);
        let mut wrote = false;
        let installed = extract(
            &mut zip,
            &members,
            &targets,
            &AtomicBool::new(false),
            &mut wrote,
        )
        .unwrap();
        assert!(wrote);
        assert_eq!(installed[0].0, "a.bin");
        assert_eq!(installed[0].1, format!("{:x}", Sha256::digest(b"hello")));
        assert_eq!(fs::read(root.join("a.bin")).unwrap(), b"hello");
    }

    #[test]
    fn corrupt_member_checksum_is_rejected_and_written_bytes_stay() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir_all(&root).unwrap();
        let data = vec![3u8; 4096];
        let path = archive(&temp, &[("a.bin", &data)]);
        // Flip a stored byte; the central directory still carries the original
        // CRC, so the zip reader reports the checksum mismatch at member EOF.
        let mut bytes = fs::read(&path).unwrap();
        let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[28], bytes[29]]) as usize;
        bytes[30 + name_len + extra_len] ^= 0xff;
        fs::write(&path, &bytes).unwrap();

        let mut zip = zip_recipe::open(&path).unwrap();
        let members = zip_recipe::plan(&mut zip, &root, &AtomicBool::new(false)).unwrap();
        let targets = targets(&root, &members);
        let mut wrote = false;
        assert!(extract(
            &mut zip,
            &members,
            &targets,
            &AtomicBool::new(false),
            &mut wrote,
        )
        .is_err());
        // The direct route has no rollback image: decoded bytes remain.
        assert!(wrote);
        assert_eq!(fs::read(root.join("a.bin")).unwrap().len(), data.len());
    }

    #[test]
    fn signed_digest_mismatch_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir_all(&root).unwrap();
        let path = archive(&temp, &[("a.bin", b"hello")]);
        let mut zip = zip_recipe::open(&path).unwrap();
        let mut members = zip_recipe::plan(&mut zip, &root, &AtomicBool::new(false)).unwrap();
        members[0].expected_sha256 = Some("0".repeat(64));
        let targets = targets(&root, &members);
        let mut wrote = false;
        let error = extract(
            &mut zip,
            &members,
            &targets,
            &AtomicBool::new(false),
            &mut wrote,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "zip_member_hash_mismatch");
    }

    #[test]
    fn cancellation_before_the_first_member_writes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir_all(&root).unwrap();
        let path = archive(&temp, &[("a.bin", b"hello")]);
        let mut zip = zip_recipe::open(&path).unwrap();
        let members = zip_recipe::plan(&mut zip, &root, &AtomicBool::new(false)).unwrap();
        let targets = targets(&root, &members);
        let mut wrote = false;
        let error = extract(
            &mut zip,
            &members,
            &targets,
            &AtomicBool::new(true),
            &mut wrote,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "cancelled");
        assert!(!wrote);
        assert!(!root.join("a.bin").exists());
    }
}

#[cfg(test)]
mod no_preread_tests {
    use super::*;
    use crate::installer::zip_recipe;
    use std::{
        fs::{self, File},
        io::Write,
    };
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    /// Mapping reads the central directory and wrapper headers only. A member
    /// whose compressed stream is corrupt must still map; the single extraction
    /// pass is the first place the bytes are decoded.
    #[test]
    fn corrupt_member_content_does_not_block_mapping() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir_all(&root).unwrap();
        let archive = temp.path().join("corrupt.zip");
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("data.bin", options).unwrap();
        zip.write_all(&vec![7u8; 256 * 1024]).unwrap();
        zip.finish().unwrap();

        let mut bytes = fs::read(&archive).unwrap();
        let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[28], bytes[29]]) as usize;
        let data = 30 + name_len + extra_len;
        bytes[data + 8] ^= 0xff;
        fs::write(&archive, &bytes).unwrap();

        let mut zip = zip_recipe::open(&archive).unwrap();
        let members = zip_recipe::plan(&mut zip, &root, &AtomicBool::new(false)).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].size, 256 * 1024);

        let targets: Vec<DirectTarget> = members
            .iter()
            .map(|member| DirectTarget {
                path: member.destination.clone(),
                file: root.join(&member.destination),
            })
            .collect();
        let mut wrote = false;
        assert!(extract(
            &mut zip,
            &members,
            &targets,
            &AtomicBool::new(false),
            &mut wrote
        )
        .is_err());
    }
}

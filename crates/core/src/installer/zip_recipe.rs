//! Publisher ZIP metadata and path mapping (p0-contracts §2.4/§4.1; port of
//! `publisher.py::_relative`, `_extra_safe`, `_wrapped_xp3_paths` and
//! `_wrapped_ypf_paths`).
//!
//! The module only reads the archive index and the few member headers needed to
//! recognise the XP3/YPF wrapper layouts. It never decodes member content and
//! never writes to the game tree; `zip_direct` performs the single streaming
//! extraction after [`plan`]/[`plan_mapped`] has fixed every target path.

use super::{invalid, Result};
use crate::catalog::FileMapping;
use crate::operation;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use zip::ZipArchive;

/// XP3 archive signature (`58 50 33 0d 0a 20 0a 1a 8b 67 01`, 11 bytes).
const XP3_SIGNATURE: [u8; 11] = [
    0x58, 0x50, 0x33, 0x0d, 0x0a, 0x20, 0x0a, 0x1a, 0x8b, 0x67, 0x01,
];

/// One selected archive member and the game-relative path it installs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Member index in the ZIP central directory.
    pub index: usize,
    /// Exact archive member name (`FileMapping.source` for signed recipes).
    pub source: String,
    /// Validated destination relative to the game root.
    pub destination: String,
    /// Declared uncompressed size.
    pub size: u64,
    /// Digest declared by a signed recipe; checked while extracting.
    pub expected_sha256: Option<String>,
}

/// One archive member with the fields the mapping needs, detached from `zip`.
struct Entry {
    index: usize,
    name: String,
    is_dir: bool,
    size: u64,
    mode: u32,
    extra: Vec<u8>,
}

/// Opens and preflights the archive once; the same handle is used for mapping
/// and for the subsequent extraction.
pub fn open(archive: &Path) -> Result<ZipArchive<File>> {
    from_file(File::open(archive)?)
}

/// Wraps an already-open file (the signed local-ZIP route holds one handle for
/// the identity check and extraction).
pub fn from_file(mut file: File) -> Result<ZipArchive<File>> {
    crate::zip_metadata::preflight(&mut file)?;
    ZipArchive::new(file).map_err(|e| invalid(format!("invalid_zip:{e}")))
}

fn validate_root(root: &Path) -> Result<()> {
    if !root.is_dir() || !no_links(root) {
        return Err(invalid("unsafe_game_root"));
    }
    Ok(())
}

/// Validates the archive and maps every selected member to a game-relative
/// path, applying the XP3/YPF wrapper rules of the publisher sources.
pub fn plan(zip: &mut ZipArchive<File>, root: &Path, cancel: &AtomicBool) -> Result<Vec<Member>> {
    validate_root(root)?;
    let entries = collect_entries(zip, cancel)?;
    validate_entries(&entries, cancel)?;
    let mapped = mapping_paths(zip, root, &entries, cancel)?;
    finish_plan(&entries, &mapped, root, cancel)
}

/// Validates the archive against a signed recipe's `source → destination`
/// mapping. Members not named by the recipe are validated but not installed.
pub fn plan_mapped(
    zip: &mut ZipArchive<File>,
    root: &Path,
    mappings: &[FileMapping],
    cancel: &AtomicBool,
) -> Result<Vec<Member>> {
    validate_root(root)?;
    let entries = collect_entries(zip, cancel)?;
    validate_entries(&entries, cancel)?;
    let by_name: HashMap<String, &Entry> = entries
        .iter()
        .map(|entry| (entry.name.to_lowercase(), entry))
        .collect();
    let mut mapped: HashMap<String, Option<String>> = HashMap::new();
    let mut expected: HashMap<String, String> = HashMap::new();
    let mut order: HashMap<String, usize> = HashMap::new();
    for (position, mapping) in mappings.iter().enumerate() {
        relative(&mapping.destination)?;
        let entry = by_name
            .get(&mapping.source.to_lowercase())
            .ok_or_else(|| invalid("recipe_source_missing"))?;
        if entry.is_dir || entry.name != mapping.source {
            return Err(invalid("recipe_source_mismatch"));
        }
        if mapped
            .insert(entry.name.clone(), Some(mapping.destination.clone()))
            .is_some()
        {
            return Err(invalid("recipe_duplicate_source"));
        }
        expected.insert(entry.name.clone(), mapping.installed_sha256.clone());
        order.insert(entry.name.clone(), position);
    }
    for entry in &entries {
        mapped.entry(entry.name.clone()).or_insert(None);
    }
    let mut members = finish_plan(&entries, &mapped, root, cancel)?;
    // The journal keeps the signed recipe order so the projection can compare
    // `plan.changes` with `recipe.files` positionally.
    members.sort_by_key(|member| order.get(&member.source).copied().unwrap_or(usize::MAX));
    for member in &mut members {
        member.expected_sha256 = expected.get(&member.source).cloned();
    }
    Ok(members)
}

/// Reads member metadata once so later passes can borrow `zip` mutably.
fn collect_entries(zip: &mut ZipArchive<File>, cancel: &AtomicBool) -> Result<Vec<Entry>> {
    let mut entries = Vec::with_capacity(zip.len());
    for index in 0..zip.len() {
        operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let file = zip
            .by_index(index)
            .map_err(|e| invalid(format!("invalid_zip:{e}")))?;
        let decoded = file.name().to_string();
        if !name_encoding_ok(file.name_raw(), &decoded) {
            return Err(invalid("zip_name_encoding"));
        }
        let name = decoded
            .strip_suffix('/')
            .unwrap_or(decoded.as_str())
            .to_string();
        entries.push(Entry {
            index,
            name,
            is_dir: file.is_dir(),
            size: file.size(),
            mode: file.unix_mode().unwrap_or(0) & 0o170000,
            extra: file.extra_data().map(<[u8]>::to_vec).unwrap_or_default(),
        });
    }
    Ok(entries)
}

/// Port of `publisher.py::_relative` plus the reserved-prefix and encoding
/// checks from `_recipe`.
fn validate_entries(entries: &[Entry], cancel: &AtomicBool) -> Result<()> {
    let mut names: HashSet<String> = HashSet::new();
    let mut prefixes: HashMap<String, String> = HashMap::new();
    for entry in entries {
        operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        relative(&entry.name)?;
        if entry
            .name
            .split('/')
            .any(|part| part.to_ascii_lowercase().starts_with(".steam-patch-"))
        {
            return Err(invalid("reserved_installer_path"));
        }
        let key = entry.name.to_lowercase();
        if !names.insert(key) {
            return Err(invalid("zip_duplicate_or_case_collision"));
        }
        let mut prefix = String::new();
        for part in entry.name.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            let lower = prefix.to_lowercase();
            match prefixes.get(&lower) {
                Some(previous) if previous != &prefix => {
                    return Err(invalid("zip_duplicate_or_case_collision"));
                }
                None => {
                    prefixes.insert(lower, prefix.clone());
                }
                _ => {}
            }
        }

        if entry.is_dir {
            if entry.mode != 0 && entry.mode != 0o040000 {
                return Err(invalid("zip_link_or_special_file"));
            }
        } else if entry.mode != 0 && entry.mode != 0o100000 {
            return Err(invalid("zip_link_or_special_file"));
        }
        extra_safe(&entry.extra)?;
    }
    Ok(())
}

/// Port of `publisher.py::_relative`.
fn relative(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 1024
        || name.contains(['\\', ':'])
        || name.chars().any(|c| (c as u32) < 32)
    {
        return Err(invalid("unsafe_relative_path"));
    }
    for part in name.split('/') {
        let device = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let numbered_device = device.len() == 4
            && (device.starts_with("COM") || device.starts_with("LPT"))
            && device.as_bytes()[3].is_ascii_digit();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part.chars().any(|c| "<>\"|?*".contains(c))
            || ["CON", "PRN", "AUX", "NUL"].contains(&device.as_str())
            || numbered_device
        {
            return Err(invalid("unsafe_relative_path"));
        }
    }
    Ok(())
}

/// Port of `publisher.py::_extra_safe`.
pub(crate) fn extra_safe(extra: &[u8]) -> Result<()> {
    let mut i = 0usize;
    while i + 4 <= extra.len() {
        let tag = u16::from_le_bytes([extra[i], extra[i + 1]]);
        let size = u16::from_le_bytes([extra[i + 2], extra[i + 3]]) as usize;
        i += 4;
        if i + size > extra.len() {
            return Err(invalid("zip_invalid_extra_metadata"));
        }
        let record = &extra[i..i + size];
        if tag == 0x000d || tag == 0x756e {
            return Err(invalid("zip_unsafe_extra_metadata"));
        }
        if tag == 0x000a {
            // NTFS timestamps are descriptive and never applied. Validate its
            // reserved prefix and subrecord framing, without default-value gates.
            if record.len() < 4 {
                return Err(invalid("zip_invalid_extra_metadata"));
            }
            let mut at = 4;
            while at + 4 <= record.len() {
                let length = u16::from_le_bytes([record[at + 2], record[at + 3]]) as usize;
                at += 4;
                if length > record.len() - at {
                    return Err(invalid("zip_invalid_extra_metadata"));
                }
                at += length;
            }
            if at != record.len() {
                return Err(invalid("zip_invalid_extra_metadata"));
            }
        }
        i += size;
    }
    if i != extra.len() {
        return Err(invalid("zip_invalid_extra_metadata"));
    }
    Ok(())
}

/// The zip crate decodes flagged names lossily from UTF-8 and unflagged names
/// as CP437. Python rejects non-ASCII names that are not UTF-8 flagged, which is
/// equivalent to `decoded == from_utf8_lossy(raw)` for non-ASCII raw bytes.
pub(crate) fn name_encoding_ok(raw: &[u8], decoded: &str) -> bool {
    // zip decodes unflagged legacy names using CP437. Accept that lossless
    // standard representation, but never a replacement-decoded UTF-8 name.
    raw == decoded.as_bytes()
        || (std::str::from_utf8(raw).is_err() && !decoded.contains('\u{fffd}'))
}

const YPF_SIGNATURE: [u8; 4] = *b"YPF\0";
const ASF_SIGNATURE: [u8; 16] = [
    0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62, 0xce, 0x6c,
];

fn mapping_paths(
    zip: &mut ZipArchive<File>,
    root: &Path,
    entries: &[Entry],
    cancel: &AtomicBool,
) -> Result<HashMap<String, Option<String>>> {
    let xp3 = wrapped_xp3_paths(zip, root, entries, cancel)?;
    if !xp3.is_empty() {
        return Ok(xp3);
    }
    wrapped_ypf_paths(zip, root, entries, cancel)
}

fn wrapped_ypf_paths(
    zip: &mut ZipArchive<File>,
    root: &Path,
    entries: &[Entry],
    cancel: &AtomicBool,
) -> Result<HashMap<String, Option<String>>> {
    let pac = root.join("pac");
    if !no_links(&pac) || !pac.is_dir() {
        return Ok(HashMap::new());
    }
    let mut has_game_ypf = false;
    for item in fs::read_dir(&pac)? {
        operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let path = item?.path();
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ypf"))
            && no_links(&path)
            && path.is_file()
            && file_starts_with(&path, &YPF_SIGNATURE)?
        {
            has_game_ypf = true;
            break;
        }
    }
    if !has_game_ypf {
        return Ok(HashMap::new());
    }
    let files: Vec<&Entry> = entries.iter().filter(|entry| !entry.is_dir).collect();
    let candidate = files.iter().any(|entry| {
        let parts: Vec<_> = entry.name.split('/').collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("pac")
    });
    if !candidate {
        return Ok(HashMap::new());
    }
    let mut wrapper: Option<&str> = None;
    let mut has_patch_ypf = false;
    let mut mapped = HashMap::new();
    for entry in files {
        operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let parts: Vec<_> = entry.name.split('/').collect();
        if parts.len() != 3
            || !parts[1].eq_ignore_ascii_case("pac")
            || !parts[2].to_ascii_lowercase().ends_with(".ypf")
        {
            return Err(invalid("zip_wrapped_layout_unsupported"));
        }
        if let Some(previous) = wrapper {
            if previous != parts[0] {
                return Err(invalid("zip_wrapped_layout_unsupported"));
            }
        } else {
            wrapper = Some(parts[0]);
        }
        let ypf = entry_starts_with(zip, entry.index, &YPF_SIGNATURE)?;
        if !ypf && !entry_starts_with(zip, entry.index, &ASF_SIGNATURE)? {
            return Err(invalid("zip_wrapped_layout_unsupported"));
        }
        has_patch_ypf |= ypf;
        mapped.insert(entry.name.clone(), Some(format!("pac/{}", parts[2])));
    }
    if !has_patch_ypf {
        return Err(invalid("zip_wrapped_layout_unsupported"));
    }
    Ok(mapped)
}

/// Revalidate final paths after optional wrapper mapping and detect existing
/// paths that differ only in case on case-sensitive filesystems.
fn validate_destinations(
    entries: &[Entry],
    mapped: &HashMap<String, Option<String>>,
    root: &Path,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut files = HashSet::new();
    let mut prefixes: HashMap<String, String> = HashMap::new();
    let mut disk_names: HashMap<PathBuf, HashMap<String, Option<String>>> = HashMap::new();
    for entry in entries.iter().filter(|entry| !entry.is_dir) {
        operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let destination = match mapped.get(&entry.name) {
            Some(None) => continue,
            Some(Some(path)) => path.as_str(),
            None => entry.name.as_str(),
        };
        relative(destination)?;
        let key = destination.to_lowercase();
        if !files.insert(key) {
            return Err(invalid("zip_duplicate_or_case_collision"));
        }
        let mut prefix = String::new();
        let mut parent = root.to_path_buf();
        for part in destination.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            let lower = prefix.to_lowercase();
            if let Some(previous) = prefixes.get(&lower) {
                if previous != &prefix {
                    return Err(invalid("zip_duplicate_or_case_collision"));
                }
            } else {
                prefixes.insert(lower, prefix.clone());
            }
            if !disk_names.contains_key(&parent) {
                let mut names = HashMap::new();
                match fs::read_dir(&parent) {
                    Ok(items) => {
                        for item in items {
                            let name = item?.file_name().to_string_lossy().into_owned();
                            let folded = name.to_lowercase();
                            names
                                .entry(folded)
                                .and_modify(|old: &mut Option<String>| {
                                    if old.as_deref() != Some(name.as_str()) {
                                        *old = None;
                                    }
                                })
                                .or_insert(Some(name));
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
                disk_names.insert(parent.clone(), names);
            }
            if disk_names[&parent]
                .get(&part.to_lowercase())
                .is_some_and(|name| name.as_deref() != Some(part))
            {
                return Err(invalid("zip_target_case_collision"));
            }
            parent.push(part);
        }
    }
    for entry in entries.iter().filter(|entry| !entry.is_dir) {
        operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let destination = match mapped.get(&entry.name) {
            Some(None) => continue,
            Some(Some(path)) => path.as_str(),
            None => entry.name.as_str(),
        };
        if has_file_directory_collision(destination, &files, &HashSet::new()) {
            return Err(invalid("zip_file_directory_collision"));
        }
    }
    Ok(())
}

fn finish_plan(
    entries: &[Entry],
    mapped: &HashMap<String, Option<String>>,
    root: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<Member>> {
    validate_destinations(entries, mapped, root, cancel)?;
    let mut members = Vec::new();
    for entry in entries.iter().filter(|entry| !entry.is_dir) {
        let destination = match mapped.get(&entry.name) {
            Some(None) => continue,
            Some(Some(destination)) => destination.clone(),
            None => entry.name.clone(),
        };
        // Target type, links and on-disk case collisions are checked once by
        // `zip_target_sizes` and again under the game lock by
        // `install::begin_direct_at`; mapping only fixes the path.
        members.push(Member {
            index: entry.index,
            source: entry.name.clone(),
            destination,
            size: entry.size,
            expected_sha256: None,
        });
    }
    if members.is_empty() {
        return Err(invalid("empty_recipe"));
    }
    Ok(members)
}

/// Port of `publisher.py::_wrapped_xp3_paths`.
fn wrapped_xp3_paths(
    zip: &mut ZipArchive<File>,
    root: &Path,
    entries: &[Entry],
    cancel: &AtomicBool,
) -> Result<HashMap<String, Option<String>>> {
    let base = root.join("data.xp3");
    if !no_links(&base) || !base.is_file() {
        return Ok(HashMap::new());
    }
    if !file_starts_with(&base, &XP3_SIGNATURE)? {
        return Ok(HashMap::new());
    }

    let non_dirs: Vec<&Entry> = entries.iter().filter(|entry| !entry.is_dir).collect();
    if non_dirs.is_empty() {
        return Ok(HashMap::new());
    }
    let mut top: Option<&str> = None;
    for entry in &non_dirs {
        operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let mut parts = entry.name.split('/');
        let (Some(first), Some(_), None) = (parts.next(), parts.next(), parts.next()) else {
            return Ok(HashMap::new());
        };
        match top {
            None => top = Some(first),
            Some(existing) if existing == first => {}
            Some(_) => return Ok(HashMap::new()),
        }
    }

    let is_patch = |entry: &Entry| entry.name.split('/').nth(1).is_some_and(patch_name);
    let patches: Vec<&Entry> = non_dirs
        .iter()
        .copied()
        .filter(|entry| is_patch(entry))
        .collect();
    if patches.is_empty() {
        return Ok(HashMap::new());
    }
    for entry in &non_dirs {
        let basename = entry.name.split('/').nth(1).unwrap_or_default();
        if !patch_name(basename) && !readme_name(basename) {
            return Err(invalid("zip_wrapped_layout_unsupported"));
        }
    }
    for entry in &patches {
        if !entry_starts_with(zip, entry.index, &XP3_SIGNATURE)? {
            return Err(invalid("zip_wrapped_layout_unsupported"));
        }
    }
    let mut mapped = HashMap::new();
    for entry in &non_dirs {
        let basename = entry.name.split('/').nth(1).unwrap_or_default();
        mapped.insert(
            entry.name.clone(),
            patch_name(basename).then(|| basename.to_string()),
        );
    }
    Ok(mapped)
}

fn patch_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("patch") else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(".xp3") else {
        return false;
    };
    digits.bytes().all(|b| b.is_ascii_digit())
}

fn readme_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("readme.txt") || name.eq_ignore_ascii_case("readme.md")
}

fn has_file_directory_collision(
    name: &str,
    names: &HashSet<String>,
    directory_names: &HashSet<String>,
) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    (1..parts.len()).any(|count| {
        let parent = parts[..count].join("/").to_lowercase();
        names.contains(&parent) && !directory_names.contains(&parent)
    })
}

fn file_starts_with(path: &Path, signature: &[u8]) -> Result<bool> {
    let mut file = File::open(path)?;
    let mut head = vec![0u8; signature.len()];
    let mut filled = 0usize;
    while filled < head.len() {
        let count = file.read(&mut head[filled..])?;
        if count == 0 {
            break;
        }
        filled += count;
    }
    Ok(filled == signature.len() && head == signature)
}

fn entry_starts_with(zip: &mut ZipArchive<File>, index: usize, signature: &[u8]) -> Result<bool> {
    let mut file = zip
        .by_index(index)
        .map_err(|e| invalid(format!("invalid_zip:{e}")))?;
    let mut head = vec![0u8; signature.len()];
    let mut filled = 0usize;
    while filled < head.len() {
        let count = file.read(&mut head[filled..])?;
        if count == 0 {
            break;
        }
        filled += count;
    }
    Ok(filled == signature.len() && head == signature)
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
    use crate::catalog::FileMapping;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    const SIGNATURE: [u8; 11] = XP3_SIGNATURE;

    fn zip_with(path: &Path, members: &[(&str, &[u8])]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, data) in members {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    fn game_dir(temp: &TempDir) -> PathBuf {
        let root = temp.path().join("game");
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn plan_zip(archive: &Path, root: &Path) -> Result<Vec<Member>> {
        let mut zip = open(archive)?;
        plan(&mut zip, root, &AtomicBool::new(false))
    }

    fn recipe_err(name: &str, members: &[(&str, &[u8])]) -> String {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        let archive = temp.path().join(name);
        zip_with(&archive, members);
        plan_zip(&archive, &root).unwrap_err().to_string()
    }

    #[test]
    fn preserves_root_paths_without_reading_content() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        fs::write(root.join("data.txt"), b"original").unwrap();
        let archive = temp.path().join("patch.zip");
        zip_with(
            &archive,
            &[("data.txt", b"patched"), ("sub/new.txt", b"new")],
        );
        let files = plan_zip(&archive, &root).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].source, "data.txt");
        assert_eq!(files[0].destination, "data.txt");
        assert_eq!(files[0].size, 7);
        assert_eq!(files[0].expected_sha256, None);
        assert_eq!(files[1].destination, "sub/new.txt");
        // Mapping never mutates the game tree.
        assert_eq!(fs::read(root.join("data.txt")).unwrap(), b"original");
        assert!(!root.join("sub").exists());
    }

    #[test]
    fn wrapped_xp3_maps_patches_to_root_basename() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        let base = [SIGNATURE.as_slice(), b"base"].concat();
        fs::write(root.join("data.xp3"), &base).unwrap();
        fs::write(
            root.join("patch.xp3"),
            [SIGNATURE.as_slice(), b"old"].concat(),
        )
        .unwrap();
        let archive = temp.path().join("patch.zip");
        zip_with(
            &archive,
            &[
                (
                    "Arbitrary wrapper/patch.xp3",
                    [SIGNATURE.as_slice(), b"new"].concat().as_slice(),
                ),
                ("Arbitrary wrapper/Readme.txt", b"instructions"),
            ],
        );
        let files = plan_zip(&archive, &root).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].source, "Arbitrary wrapper/patch.xp3");
        assert_eq!(files[0].destination, "patch.xp3");
    }

    #[test]
    fn wrapped_xp3_is_not_guessed_without_matching_data_xp3() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        fs::write(root.join("data.xp3"), b"different engine").unwrap();
        let archive = temp.path().join("patch.zip");
        zip_with(
            &archive,
            &[(
                "Wrapper/patch.xp3",
                [SIGNATURE.as_slice(), b"new"].concat().as_slice(),
            )],
        );
        let files = plan_zip(&archive, &root).unwrap();
        assert_eq!(files[0].destination, "Wrapper/patch.xp3");
    }

    #[test]
    fn wrapped_xp3_unknown_members_and_bad_signatures_block() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        fs::write(
            root.join("data.xp3"),
            [SIGNATURE.as_slice(), b"base"].concat(),
        )
        .unwrap();
        let patch = [SIGNATURE.as_slice(), b"a"].concat();

        let first = temp.path().join("bad.zip");
        zip_with(
            &first,
            &[
                ("wrapper/patch.xp3", patch.as_slice()),
                ("wrapper/run.exe", b"unknown"),
            ],
        );
        assert_eq!(
            plan_zip(&first, &root).unwrap_err().to_string(),
            "zip_wrapped_layout_unsupported"
        );

        let second = temp.path().join("bad2.zip");
        zip_with(&second, &[("wrapper/patch.xp3", b"not XP3")]);
        assert_eq!(
            plan_zip(&second, &root).unwrap_err().to_string(),
            "zip_wrapped_layout_unsupported"
        );
    }

    #[test]
    fn traversal_case_collision_and_directory_collision_rejected() {
        assert_eq!(
            recipe_err("t.zip", &[("../escape", b"bad")]),
            "unsafe_relative_path"
        );
        assert_eq!(
            recipe_err("c.zip", &[("A.txt", b"one"), ("a.txt", b"two")]),
            "zip_duplicate_or_case_collision"
        );
        assert_eq!(
            recipe_err("d.zip", &[("file", b"one"), ("file/child", b"two")]),
            "zip_file_directory_collision"
        );
    }

    #[test]
    fn device_names_and_reserved_paths_rejected() {
        assert_eq!(
            recipe_err("dev.zip", &[("CON", b"x")]),
            "unsafe_relative_path"
        );
        assert_eq!(
            recipe_err("dev2.zip", &[("LPT1.txt", b"x")]),
            "unsafe_relative_path"
        );
        assert_eq!(
            recipe_err("res.zip", &[(".steam-patch-state", b"x")]),
            "reserved_installer_path"
        );
    }

    #[test]
    fn unsafe_extra_metadata_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        let archive = temp.path().join("extra.zip");
        let mut options =
            zip::write::FullFileOptions::default().compression_method(CompressionMethod::Stored);
        options
            .add_extra_data(0xbeef, b"unsafe-tag", false)
            .unwrap();
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file("data.txt", options).unwrap();
        zip.write_all(b"bytes").unwrap();
        zip.finish().unwrap();

        // The crate refuses to write reserved tags, so rewrite the first
        // occurrence (local header) to the unsafe PKWARE Unix tag 0x000d.
        let mut bytes = fs::read(&archive).unwrap();
        let mut patched = 0usize;
        for index in 0..bytes.len().saturating_sub(3) {
            if bytes[index] == 0xef
                && bytes[index + 1] == 0xbe
                && bytes[index + 2] == b"unsafe-tag".len() as u8
            {
                bytes[index] = 0x0d;
                bytes[index + 1] = 0x00;
                patched += 1;
            }
        }
        assert!(patched > 0, "test setup failed to find the extra field");
        fs::write(&archive, &bytes).unwrap();
        assert_eq!(
            plan_zip(&archive, &root).unwrap_err().to_string(),
            "zip_unsafe_extra_metadata"
        );
    }

    #[test]
    fn symlink_member_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        let archive = temp.path().join("link.zip");
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        zip.add_symlink("link", "target", SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
        assert_eq!(
            plan_zip(&archive, &root).unwrap_err().to_string(),
            "zip_link_or_special_file"
        );
    }

    #[test]
    fn high_ratio_and_more_than_8192_members_are_valid() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        let archive = temp.path().join("many.zip");
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for index in 0..8200 {
            zip.add_directory(format!("dir{index}/"), options).unwrap();
        }
        zip.start_file("zeros.bin", options).unwrap();
        zip.write_all(&vec![0u8; 2 * 1024 * 1024]).unwrap();
        zip.finish().unwrap();
        let files = plan_zip(&archive, &root).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].destination, "zeros.bin");
    }

    #[test]
    fn empty_recipe() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        let empty = temp.path().join("empty.zip");
        zip_with(&empty, &[]);
        assert_eq!(
            plan_zip(&empty, &root).unwrap_err().to_string(),
            "empty_recipe"
        );
    }

    #[test]
    fn ypf_wrapper_maps_to_pac_and_accepts_asf_video() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        fs::create_dir(root.join("pac")).unwrap();
        fs::write(
            root.join("pac/bn.ypf"),
            [YPF_SIGNATURE.as_slice(), b"old"].concat(),
        )
        .unwrap();
        let archive = temp.path().join("patch.zip");
        let ypf = [YPF_SIGNATURE.as_slice(), b"new"].concat();
        let asf = [ASF_SIGNATURE.as_slice(), b"video"].concat();
        zip_with(
            &archive,
            &[
                ("wrapper/pac/bn.ypf", &ypf),
                ("wrapper/pac/update1.ypf", &ypf),
                ("wrapper/pac/video.ypf", &asf),
            ],
        );
        let files = plan_zip(&archive, &root).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].destination, "pac/bn.ypf");
        assert_eq!(files[1].destination, "pac/update1.ypf");
        assert_eq!(files[2].destination, "pac/video.ypf");
    }

    #[test]
    fn ypf_wrapper_rejects_mixed_and_unsafe_layouts() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        fs::create_dir(root.join("pac")).unwrap();
        fs::write(root.join("pac/bn.ypf"), YPF_SIGNATURE).unwrap();
        let archive = temp.path().join("mixed.zip");
        zip_with(
            &archive,
            &[
                ("wrapper/pac/update.ypf", &YPF_SIGNATURE),
                ("wrapper/run.exe", b"bad"),
            ],
        );
        assert_eq!(
            plan_zip(&archive, &root).unwrap_err().to_string(),
            "zip_wrapped_layout_unsupported"
        );
        let archive2 = temp.path().join("unsafe.zip");
        zip_with(&archive2, &[("wrapper/pac/update.ypf", b"unknown")]);
        assert_eq!(
            plan_zip(&archive2, &root).unwrap_err().to_string(),
            "zip_wrapped_layout_unsupported"
        );
    }

    #[test]
    fn ypf_wrapper_rejects_existing_case_collision() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        fs::create_dir(root.join("pac")).unwrap();
        fs::write(root.join("pac/bn.ypf"), YPF_SIGNATURE).unwrap();
        fs::write(root.join("pac/Update.ypf"), YPF_SIGNATURE).unwrap();
        let archive = temp.path().join("case.zip");
        zip_with(&archive, &[("wrapper/pac/update.ypf", &YPF_SIGNATURE)]);
        assert_eq!(
            plan_zip(&archive, &root).unwrap_err().to_string(),
            "zip_target_case_collision"
        );
    }

    #[test]
    fn six_gib_declared_member_has_no_recipe_size_gate() {
        let entries = vec![Entry {
            index: 0,
            name: "update1.ypf".into(),
            is_dir: false,
            size: 6 * 1024 * 1024 * 1024,
            mode: 0,
            extra: Vec::new(),
        }];
        validate_entries(&entries, &AtomicBool::new(false)).unwrap();
    }

    #[test]
    fn signed_mapping_uses_recipe_destinations_and_keeps_declared_digest() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        let archive = temp.path().join("signed.zip");
        zip_with(
            &archive,
            &[("payload.dat", b"patched"), ("readme.txt", b"notes")],
        );
        let mut zip = open(&archive).unwrap();
        let members = plan_mapped(
            &mut zip,
            &root,
            &[FileMapping {
                source: "payload.dat".into(),
                destination: "data/original.dat".into(),
                original_sha256: Some("0".repeat(64)),
                installed_sha256: "a".repeat(64),
            }],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].destination, "data/original.dat");
        assert_eq!(
            members[0].expected_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );
    }

    #[test]
    fn signed_mapping_requires_the_declared_member() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        let archive = temp.path().join("signed.zip");
        zip_with(&archive, &[("payload.dat", b"patched")]);
        let mut zip = open(&archive).unwrap();
        let error = plan_mapped(
            &mut zip,
            &root,
            &[FileMapping {
                source: "missing.dat".into(),
                destination: "original.dat".into(),
                original_sha256: None,
                installed_sha256: "a".repeat(64),
            }],
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "recipe_source_missing");
    }

    #[test]
    fn signed_mapping_keeps_recipe_order() {
        let temp = tempfile::tempdir().unwrap();
        let root = game_dir(&temp);
        let archive = temp.path().join("signed.zip");
        zip_with(&archive, &[("first.dat", b"one"), ("second.dat", b"two")]);
        let mut zip = open(&archive).unwrap();
        let members = plan_mapped(
            &mut zip,
            &root,
            &[
                FileMapping {
                    source: "second.dat".into(),
                    destination: "second.dat".into(),
                    original_sha256: None,
                    installed_sha256: "b".repeat(64),
                },
                FileMapping {
                    source: "first.dat".into(),
                    destination: "first.dat".into(),
                    original_sha256: None,
                    installed_sha256: "a".repeat(64),
                },
            ],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(members[0].source, "second.dat");
        assert_eq!(members[1].source, "first.dat");
    }

    #[test]
    #[ignore = "manual ZIP metadata and mapping scaling benchmark"]
    fn benchmark_metadata_and_mapping_10k_50k() {
        use std::time::Instant;
        for count in [10_000, 50_000] {
            let temp = tempfile::tempdir().unwrap();
            let root = game_dir(&temp);
            let archive = temp.path().join("many.zip");
            let mut zip = ZipWriter::new(File::create(&archive).unwrap());
            let options = SimpleFileOptions::default();
            for index in 0..count {
                zip.start_file(format!("file{index}.txt"), options).unwrap();
                zip.write_all(b"x").unwrap();
            }
            zip.finish().unwrap();
            let start = Instant::now();
            let members = plan_zip(&archive, &root).unwrap();
            let mapping_time = start.elapsed();
            assert_eq!(members.len(), count);
            eprintln!(
                "zip files={} mapping_ms={} archive_bytes={}",
                count,
                mapping_time.as_millis(),
                fs::metadata(&archive).unwrap().len()
            );
        }
    }
}

#[cfg(test)]
mod zip64_tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    #[test]
    fn zip64_member_keeps_its_declared_size() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir_all(&root).unwrap();
        let archive = temp.path().join("zip64.zip");
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(true);
        zip.start_file("data.bin", options).unwrap();
        zip.write_all(b"zip64").unwrap();
        zip.finish().unwrap();

        let mut zip = open(&archive).unwrap();
        let members = plan(&mut zip, &root, &AtomicBool::new(false)).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].destination, "data.bin");
        assert_eq!(members[0].size, 5);
    }
}

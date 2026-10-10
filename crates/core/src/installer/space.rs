//! Checked per-volume budget for acquisition, normalization and transaction recovery.
use super::{
    exe_report::{relative_path, PayloadEntry},
    invalid, Result,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub const SPACE_MARGIN_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_ORIGINAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceBudget {
    pub state_bytes: u64,
    pub target_bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeSpace {
    pub volume_id: String,
    pub available_bytes: u64,
    pub path: PathBuf,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceEvidence {
    pub budget: SpaceBudget,
    pub state: VolumeSpace,
    pub target: VolumeSpace,
    pub same_volume: bool,
}
impl SpaceBudget {
    pub fn new(download_bytes: u64, payload_bytes: u64, original_bytes: u64) -> Result<Self> {
        let add = |a: u64, b: u64| {
            a.checked_add(b)
                .ok_or_else(|| invalid("exe_space_budget_overflow"))
        };
        // State: download plus sandbox EXE copy, normalized ZIP, stage, originals.
        let state_bytes = add(
            add(
                add(
                    add(add(download_bytes, download_bytes)?, payload_bytes)?,
                    payload_bytes,
                )?,
                original_bytes,
            )?,
            SPACE_MARGIN_BYTES
                .checked_mul(2)
                .ok_or_else(|| invalid("exe_space_budget_overflow"))?,
        )?;
        // Target: temporary copy plus restoration scratch; include all bytes even on reinstall.
        let target_bytes = add(add(payload_bytes, original_bytes)?, SPACE_MARGIN_BYTES)?;
        Ok(Self {
            state_bytes,
            target_bytes,
        })
    }
    pub fn check_available(
        &self,
        state_free: u64,
        target_free: u64,
        same_volume: bool,
    ) -> Result<()> {
        let state_need = if same_volume {
            self.state_bytes
                .checked_add(self.target_bytes)
                .ok_or_else(|| invalid("exe_space_budget_overflow"))?
        } else {
            self.state_bytes
        };
        if state_free < state_need {
            return Err(invalid("exe_state_insufficient_space"));
        }
        if target_free
            < if same_volume {
                state_need
            } else {
                self.target_bytes
            }
        {
            return Err(invalid("exe_target_insufficient_space"));
        }
        Ok(())
    }
    pub fn inspect(&self, state: &Path, target: &Path) -> Result<SpaceEvidence> {
        let state = inspect_volume(state)?;
        let target = inspect_volume(target)?;
        let same_volume = state.volume_id == target.volume_id;
        self.check_available(state.available_bytes, target.available_bytes, same_volume)?;
        Ok(SpaceEvidence {
            budget: self.clone(),
            state,
            target,
            same_volume,
        })
    }
}
/// Existing destination sizes are evidence, never a permission to read links.
pub fn original_bytes(root: &Path, payload: &[PayloadEntry]) -> Result<u64> {
    if !root.is_absolute() || !root.is_dir() || !super::zip_recipe::no_links(root) {
        return Err(invalid("exe_game_identity_changed"));
    }
    let mut total = 0u64;
    for entry in payload {
        relative_path(&entry.path)?;
        let path = root.join(&entry.path);
        if !super::zip_recipe::no_links(&path) {
            return Err(invalid("linked_target"));
        }
        let m = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if !m.is_file() {
            return Err(invalid("target_not_file"));
        }
        total = total
            .checked_add(m.len())
            .ok_or_else(|| invalid("exe_space_budget_overflow"))?;
        if total > MAX_ORIGINAL_BYTES {
            return Err(invalid("exe_original_size_limit"));
        }
    }
    Ok(total)
}
pub fn inspect_volume(path: &Path) -> Result<VolumeSpace> {
    if !path.is_absolute() || !super::zip_recipe::no_links(path) || !path.is_dir() {
        return Err(invalid("exe_space_path_invalid"));
    }
    let volume_id = volume_identity(path)?;
    let available_bytes = fs2::available_space(path)?;
    if !super::zip_recipe::no_links(path) || volume_identity(path)? != volume_id {
        return Err(invalid("exe_space_volume_changed"));
    }
    Ok(VolumeSpace {
        volume_id,
        available_bytes,
        path: path.to_owned(),
    })
}
#[cfg(unix)]
fn volume_identity(path: &Path) -> Result<String> {
    use std::os::unix::fs::MetadataExt;
    Ok(format!("dev:{}", fs::metadata(path)?.dev()))
}
#[cfg(windows)]
fn volume_identity(path: &Path) -> Result<String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumePathNameW(path: *const u16, volume: *mut u16, len: u32) -> i32;
        fn GetVolumeNameForVolumeMountPointW(path: *const u16, name: *mut u16, len: u32) -> i32;
    }
    let input: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut mount = vec![0u16; 32768];
    let mut name = vec![0u16; 1024];
    if unsafe { GetVolumePathNameW(input.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe {
        GetVolumeNameForVolumeMountPointW(mount.as_ptr(), name.as_mut_ptr(), name.len() as u32)
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let length = name
        .iter()
        .position(|c| *c == 0)
        .ok_or_else(|| invalid("exe_space_volume_invalid"))?;
    String::from_utf16(&name[..length]).map_err(|_| invalid("exe_space_volume_invalid"))
}
#[cfg(not(any(unix, windows)))]
fn volume_identity(_path: &Path) -> Result<String> {
    Err(invalid("exe_space_platform_unsupported"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combined_volume_budget() {
        let b = SpaceBudget::new(100, 200, 300).unwrap();
        let combined = b.state_bytes + b.target_bytes;
        b.check_available(b.state_bytes, b.target_bytes, false)
            .unwrap();
        assert!(b
            .check_available(b.state_bytes, b.target_bytes, true)
            .is_err());
        b.check_available(combined, combined, true).unwrap();
        assert!(b.check_available(combined - 1, combined, true).is_err());
        assert!(b.check_available(combined, combined - 1, true).is_err());
    }
    #[test]
    fn checked_arithmetic() {
        assert!(SpaceBudget::new(u64::MAX, 1, 0).is_err());
        let b = SpaceBudget {
            state_bytes: u64::MAX,
            target_bytes: 1,
        };
        assert!(b.check_available(u64::MAX, u64::MAX, true).is_err());
    }
    #[test]
    fn directory_volume_and_insufficient_space() {
        let t = tempfile::tempdir().unwrap();
        let s = inspect_volume(t.path()).unwrap();
        assert!(!s.volume_id.is_empty());
        let b = SpaceBudget {
            state_bytes: u64::MAX,
            target_bytes: 0,
        };
        assert!(b.inspect(t.path(), t.path()).is_err());
    }
}

/// Peak extra bytes for ZIP stage, backup, target temporary files and rollback.
/// The downloaded ZIP already exists when this budget is checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZipSpaceBudget {
    pub payload_bytes: u64,
    pub original_bytes: u64,
    pub staged: bool,
}
impl ZipSpaceBudget {
    pub fn for_stage(payload_bytes: u64, original_bytes: u64) -> Result<Self> {
        let budget = Self {
            payload_bytes,
            original_bytes,
            staged: false,
        };
        budget.required(false)?;
        Ok(budget)
    }
    pub fn after_staging(&self) -> Self {
        Self {
            staged: true,
            ..self.clone()
        }
    }
    pub fn required(&self, same_volume: bool) -> Result<(u64, u64)> {
        let add = |a: u64, b: u64| a.checked_add(b).ok_or_else(|| invalid("space_overflow"));
        let state = add(
            add(
                if self.staged { 0 } else { self.payload_bytes },
                self.original_bytes,
            )?,
            SPACE_MARGIN_BYTES,
        )?;
        // Payload scratch during writes and original scratch during restoration.
        let target = add(
            add(self.payload_bytes, self.original_bytes)?,
            SPACE_MARGIN_BYTES,
        )?;
        if same_volume {
            let total = add(state, target)?;
            Ok((total, total))
        } else {
            Ok((state, target))
        }
    }
    pub fn check_available(
        &self,
        state_free: u64,
        target_free: u64,
        same_volume: bool,
    ) -> Result<()> {
        let (state_need, target_need) = self.required(same_volume)?;
        for (name, need, available) in [
            ("state", state_need, state_free),
            ("target", target_need, target_free),
        ] {
            if need > available {
                return Err(invalid(format!(
                    "insufficient_space:{name}:required={need}:available={available}"
                )));
            }
        }
        Ok(())
    }
    pub fn inspect(&self, state_path: &Path, target_path: &Path) -> Result<()> {
        let state = inspect_volume(state_path)?;
        let target = inspect_volume(target_path)?;
        self.check_available(
            state.available_bytes,
            target.available_bytes,
            state.volume_id == target.volume_id,
        )
    }
}

#[cfg(test)]
mod zip_budget_tests {
    use super::*;
    #[test]
    fn zip_budget_tracks_same_and_different_volumes_and_consumed_stage() {
        let payload = 6_073_754_660;
        let original = 123;
        let budget = ZipSpaceBudget::for_stage(payload, original).unwrap();
        let (state, target) = budget.required(false).unwrap();
        assert_eq!(state, payload + original + SPACE_MARGIN_BYTES);
        assert_eq!(
            budget.required(true).unwrap(),
            (state + target, state + target)
        );
        assert!(budget.check_available(state, target, false).is_ok());
        assert!(budget
            .check_available(state, target, true)
            .unwrap_err()
            .to_string()
            .starts_with("insufficient_space:state:"));
        assert_eq!(
            budget.after_staging().required(false).unwrap().0,
            original + SPACE_MARGIN_BYTES
        );
        assert!(ZipSpaceBudget::for_stage(u64::MAX, 1).is_err());
    }
}

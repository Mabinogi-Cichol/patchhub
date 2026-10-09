//! Cutover write blockers, never receipts or installation authorization.
//! The deployment tool records current instances with preserved, unverified
//! modifications. No legacy database or profile is read by this module.
use super::{invalid, store::safe_path, InstallerPaths, Result};
use serde::Deserialize;
use std::fs;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Protection {
    schema_version: u32,
    instances: Vec<String>,
    appids: Vec<u32>,
}

impl Protection {
    pub fn is_blocked(&self, instance: &str, appid: u32) -> bool {
        self.instances.iter().any(|value| value == instance) || self.appids.contains(&appid)
    }
}

pub fn load(paths: &InstallerPaths) -> Result<Protection> {
    let path = paths.root().join("preexisting-installations.json");
    safe_path(&path)?;
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Protection::default())
        }
        Err(error) => return Err(error),
    };
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return Err(invalid("preexisting_protection_invalid"));
    }
    let value: Protection = serde_json::from_slice(&fs::read(path)?)
        .map_err(|_| invalid("preexisting_protection_invalid"))?;
    if value.schema_version != 1
        || value.instances.len() > 8192
        || value
            .instances
            .iter()
            .any(|id| id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control))
    {
        return Err(invalid("preexisting_protection_invalid"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protection_is_read_only_and_invalid_marker_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        assert!(!load(&paths).unwrap().is_blocked("known-instance", 42));
        assert!(!paths.root().exists());
        fs::create_dir_all(paths.root()).unwrap();
        let file = paths.root().join("preexisting-installations.json");
        fs::write(
            &file,
            br#"{"schema_version":1,"instances":["known-instance"],"appids":[42]}"#,
        )
        .unwrap();
        assert!(load(&paths).unwrap().is_blocked("known-instance", 42));
        assert!(load(&paths)
            .unwrap()
            .is_blocked("moved-to-another-library", 42));
        fs::write(&file, b"bad marker").unwrap();
        assert_eq!(
            load(&paths).unwrap_err().to_string(),
            "preexisting_protection_invalid"
        );
    }
}

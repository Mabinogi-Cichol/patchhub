//! AppContainer profile creation (including LPAC) and ACL grants to the package SID.

use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows::Win32::Security::{
    ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE,
    PSECURITY_DESCRIPTOR, PSID,
};

use super::util::{hresult_to_win32, hstring, local_free, wide};
use crate::model::IsolationError;

/// Delete access right (not re-exported by the enabled feature set).
const DELETE: u32 = 0x0001_0000;

/// An AppContainer profile plus its package SID.
#[derive(Debug, Clone, Copy)]
pub struct Profile {
    pub sid: PSID,
}

/// Access bundle granted to the package SID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    /// Read + execute (worker binaries and read-only inputs).
    ReadExecute,
    /// Read + write + execute + delete (exclusive output).
    ReadWriteDelete,
}

impl AccessKind {
    fn mask(self) -> u32 {
        match self {
            AccessKind::ReadExecute => GENERIC_READ.0 | GENERIC_EXECUTE.0,
            AccessKind::ReadWriteDelete => {
                GENERIC_READ.0 | GENERIC_WRITE.0 | GENERIC_EXECUTE.0 | DELETE
            }
        }
    }
}

impl Profile {
    pub fn sid_string(&self) -> Result<String, IsolationError> {
        sid_to_string(self.sid)
    }
}

/// Create the AppContainer profile, or derive its SID when it already exists.
pub fn ensure_profile(name: &str) -> Result<Profile, IsolationError> {
    let hname = hstring(name);
    let result = unsafe { CreateAppContainerProfile(&hname, &hname, &hname, None) };
    match result {
        Ok(sid) => Ok(Profile { sid }),
        Err(err) => {
            let code = hresult_to_win32(&err);
            if code == 183 {
                // ERROR_ALREADY_EXISTS: the profile exists (e.g. a previous probe run).
                match unsafe { DeriveAppContainerSidFromAppContainerName(&hname) } {
                    Ok(sid) => Ok(Profile { sid }),
                    Err(err) => Err(IsolationError::new(
                        "profile",
                        Some(hresult_to_win32(&err)),
                        format!("DeriveAppContainerSidFromAppContainerName failed: {err}"),
                    )),
                }
            } else {
                Err(IsolationError::new(
                    "profile",
                    Some(code),
                    format!("CreateAppContainerProfile('{name}') failed: {err}"),
                ))
            }
        }
    }
}

/// Delete a profile created by [`ensure_profile`]. Best effort.
pub fn delete_profile(name: &str) -> Result<(), IsolationError> {
    let hname = hstring(name);
    unsafe { DeleteAppContainerProfile(&hname) }.map_err(|err| {
        IsolationError::new(
            "profile",
            Some(hresult_to_win32(&err)),
            format!("DeleteAppContainerProfile failed: {err}"),
        )
    })
}

/// Grant `kind` to the package SID on `path`, preserving the existing DACL and adding
/// inheritable ACEs for containers and objects.
pub fn grant_path_access(path: &Path, sid: PSID, kind: AccessKind) -> Result<(), IsolationError> {
    let path_str = path.to_string_lossy().to_string();
    let wpath = wide(&path_str);

    let mut old_dacl: *mut ACL = std::ptr::null_mut();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    let res = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wpath.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut old_dacl),
            None,
            &mut sd,
        )
    };
    if res.0 != 0 {
        return Err(IsolationError::new(
            "acl",
            Some(res.0),
            format!("GetNamedSecurityInfoW('{path_str}') failed"),
        ));
    }

    let trustee = TRUSTEE_W {
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_UNKNOWN,
        ptstrName: PWSTR(sid.0 as *mut u16),
        ..Default::default()
    };

    let ea = EXPLICIT_ACCESS_W {
        grfAccessPermissions: kind.mask(),
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE,
        Trustee: trustee,
    };

    let old_opt = if old_dacl.is_null() {
        None
    } else {
        Some(old_dacl as *const ACL)
    };
    let mut new_dacl: *mut ACL = std::ptr::null_mut();
    let res = unsafe { SetEntriesInAclW(Some(&[ea]), old_opt, &mut new_dacl) };
    if res.0 != 0 {
        unsafe { local_free(sd.0) };
        return Err(IsolationError::new(
            "acl",
            Some(res.0),
            format!("SetEntriesInAclW('{path_str}') failed"),
        ));
    }

    let res = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(wpath.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(new_dacl as *const ACL),
            None,
        )
    };

    unsafe {
        local_free(new_dacl as *mut core::ffi::c_void);
        local_free(sd.0);
    }

    if res.0 != 0 {
        return Err(IsolationError::new(
            "acl",
            Some(res.0),
            format!("SetNamedSecurityInfoW('{path_str}') failed"),
        ));
    }
    Ok(())
}

/// Convert a SID into its `S-1-...` string form.
pub fn sid_to_string(sid: PSID) -> Result<String, IsolationError> {
    let mut raw = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut raw) }.map_err(|err| {
        IsolationError::new(
            "sid",
            Some(hresult_to_win32(&err)),
            format!("ConvertSidToStringSidW failed: {err}"),
        )
    })?;
    let out = unsafe { raw.to_string() }.map_err(|err| {
        IsolationError::new("sid", None, format!("sid string conversion failed: {err}"))
    });
    unsafe { local_free(raw.0 as *mut core::ffi::c_void) };
    out
}

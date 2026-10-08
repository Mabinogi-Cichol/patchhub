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
#[derive(Debug)]
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

impl Drop for Profile {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::Security::FreeSid(self.sid);
        }
    }
}

/// Protect a fresh, still-empty invocation directory before copying any data.
/// Only the current host user gets inheritable full access; LPAC is granted later.
pub fn make_private(path: &Path) -> Result<(), IsolationError> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::{
        GetTokenInformation, SetFileSecurityW, TokenUser, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| IsolationError::new("acl", Some(hresult_to_win32(&e)), e.to_string()))?;
    let result = (|| {
        let mut size = 0u32;
        let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut size) };
        if size == 0 {
            return Err(IsolationError::new("acl", None, "cannot query host SID"));
        }
        let mut buf = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr().cast()),
                size,
                &mut size,
            )
        }
        .map_err(|e| IsolationError::new("acl", Some(hresult_to_win32(&e)), e.to_string()))?;
        let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
        let sid = sid_to_string(user.User.Sid)?;
        let sddl = hstring(&format!("O:{sid}D:P(A;OICI;FA;;;{sid})"));
        let mut sd = PSECURITY_DESCRIPTOR::default();
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, 1, &mut sd, None) }
            .map_err(|e| IsolationError::new("acl", Some(hresult_to_win32(&e)), e.to_string()))?;
        let ok = unsafe {
            SetFileSecurityW(
                &hstring(&path.to_string_lossy()),
                DACL_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION
                    | OWNER_SECURITY_INFORMATION,
                sd,
            )
        };
        let code = super::util::last_error();
        unsafe { local_free(sd.0) };
        if !ok.as_bool() {
            return Err(IsolationError::new(
                "acl",
                Some(code),
                "protect private staging DACL failed",
            ));
        }
        Ok(())
    })();
    unsafe { super::util::close_handle(token) };
    result
}
/// Current host token user, used only for private staging ownership markers.
pub fn host_sid() -> Result<String, IsolationError> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| IsolationError::new("identity", Some(hresult_to_win32(&e)), e.to_string()))?;
    let result = (|| {
        let mut size = 0u32;
        let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut size) };
        if size == 0 {
            return Err(IsolationError::new(
                "identity",
                None,
                "missing host token user",
            ));
        }
        let mut buf = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr().cast()),
                size,
                &mut size,
            )
        }
        .map_err(|e| IsolationError::new("identity", Some(hresult_to_win32(&e)), e.to_string()))?;
        sid_to_string(unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid })
    })();
    unsafe { super::util::close_handle(token) };
    result
}
/// File ownership check for startup collection. The marker alone is not proof
/// that a directory was created by this host user.
pub fn path_owner_sid(path: &Path) -> Result<String, IsolationError> {
    use windows::Win32::Security::OWNER_SECURITY_INFORMATION;
    let mut owner = PSID::default();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    let code = unsafe {
        GetNamedSecurityInfoW(
            &hstring(&path.to_string_lossy()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            &mut sd,
        )
    };
    if code.0 != 0 {
        return Err(IsolationError::new(
            "identity",
            Some(code.0),
            "cannot query staging owner",
        ));
    }
    let result = sid_to_string(owner);
    unsafe { local_free(sd.0) };
    result
}

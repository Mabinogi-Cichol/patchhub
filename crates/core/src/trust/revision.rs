//! Persistent trust rollback protection, separate from read-only verification.
use super::{TrustKeys, VerifiedCatalog};
use crate::{install, installer, projection::DomainError, steam::GameInstall};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    time::Duration,
};

/// The provider supplies raw material; only core can authenticate it.
pub struct TrustedCatalogMaterial {
    pub envelope: Vec<u8>,
    pub keys: TrustKeys,
}

pub trait TrustedCatalogProvider {
    fn load(&self) -> Result<TrustedCatalogMaterial, DomainError>;
}

pub trait TrustRevisionStore {
    /// Persistent production stores expose their host binding; fixtures may omit it.
    fn state_dir(&self) -> Option<&Path> {
        None
    }

    /// Missing state or the singleton row means zero. Never creates state.
    fn highest_revision(&self) -> Result<u64, DomainError>;
    /// Only authenticated catalogs can advance the persistent high-water mark.
    fn remember_verified_revision(&self, catalog: &VerifiedCatalog) -> Result<(), DomainError>;
}

/// Authenticates, checks validity and rollback, but never remembers a revision.
pub fn verify_catalog(
    provider: &dyn TrustedCatalogProvider,
    store: &dyn TrustRevisionStore,
    now: u64,
) -> Result<VerifiedCatalog, DomainError> {
    let material = provider.load()?;
    let highest = store.highest_revision()?;
    VerifiedCatalog::from_bytes(&material.envelope, &material.keys, now, highest).map_err(io_error)
}

/// Production store. Its only database is `state_dir/trust.sqlite`.
///
/// Construction is read-only. Hosts with Steam instances should use
/// `new_for_games` so every subsequent access rechecks their state boundary.
#[derive(Debug, Clone)]
pub struct SqliteTrustRevisionStore {
    state_dir: PathBuf,
    games: Vec<GameInstall>,
}

impl SqliteTrustRevisionStore {
    pub fn new(state_dir: &Path) -> Result<Self, DomainError> {
        Self::new_for_games(state_dir, &[])
    }

    pub fn new_for_games(state_dir: &Path, games: &[GameInstall]) -> Result<Self, DomainError> {
        // Reject traversal before absolute() has a chance to normalize it.
        if state_dir
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(DomainError::code("unsafe_path_component"));
        }
        let store = Self {
            state_dir: std::path::absolute(state_dir).map_err(io_error)?,
            games: games.to_vec(),
        };
        store.validate_paths()?;
        Ok(store)
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    fn database(&self) -> PathBuf {
        self.state_dir.join("trust.sqlite")
    }

    fn validate_paths(&self) -> Result<(), DomainError> {
        install::validate_projection_state(&self.state_dir, &self.games).map_err(io_error)?;
        match fs::symlink_metadata(&self.state_dir) {
            Ok(metadata) if !metadata.is_dir() => {
                return Err(DomainError::code("invalid_state_path"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
        for name in [
            "trust.sqlite",
            "trust.sqlite-journal",
            "trust.sqlite-wal",
            "trust.sqlite-shm",
        ] {
            let path = self.state_dir.join(name);
            install::validate_projection_state(&path, &[]).map_err(io_error)?;
            match fs::symlink_metadata(&path) {
                Ok(metadata) if !metadata.is_file() => {
                    return Err(DomainError::code("invalid_trust_state_path"));
                }
                Ok(_) => {
                    drop(open_regular_nofollow(&path)?);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(error)),
            }
        }
        Ok(())
    }
}

impl TrustRevisionStore for SqliteTrustRevisionStore {
    fn state_dir(&self) -> Option<&Path> {
        Some(&self.state_dir)
    }

    fn highest_revision(&self) -> Result<u64, DomainError> {
        self.validate_paths()?;
        let path = self.database();
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(io_error(error)),
            Ok(_) => {}
        }
        // Read-only SQLite can still create or alter WAL shared-memory sidecars.
        // Production trust state has always used rollback journaling. Reject a
        // foreign WAL database rather than ignore its committed high-water mark.
        let mut header = [0_u8; 20];
        let mut file = open_regular_nofollow(&path)?;
        file.read_exact(&mut header)
            .map_err(|error| DomainError::new("trust_store_error", error.to_string()))?;
        drop(file);
        if header[18] == 2 || header[19] == 2 {
            return Err(DomainError::new(
                "trust_store_error",
                "read-only trust state requires rollback-journal mode",
            ));
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(sql_error)?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(sql_error)?;
        // A read-only connection cannot initialize, migrate or recover a DB.
        self.validate_paths()?;
        let revision: Option<i64> = conn
            .query_row(
                "SELECT revision FROM catalog_revision WHERE id=1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        u64::try_from(revision.unwrap_or(0))
            .map_err(|_| DomainError::code("invalid_catalog_revision"))
    }

    fn remember_verified_revision(&self, catalog: &VerifiedCatalog) -> Result<(), DomainError> {
        if let Some(code) = installer::host_identity_blocker() {
            return Err(DomainError::code(code));
        }
        let revision = i64::try_from(catalog.catalog().revision)
            .map_err(|_| DomainError::code("catalog_revision_out_of_range"))?;
        self.validate_paths()?;
        fs::create_dir_all(&self.state_dir).map_err(io_error)?;
        self.validate_paths()?;
        let mut conn = Connection::open_with_flags(
            self.database(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(sql_error)?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(sql_error)?;
        self.validate_paths()?;
        conn.execute_batch(
            "PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS catalog_revision(id INTEGER PRIMARY KEY, revision INTEGER NOT NULL);",
        )
        .map_err(sql_error)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        // Do not trust an earlier read: other processes may have advanced it.
        let current: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(revision),0) FROM catalog_revision",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if current < 0 {
            return Err(DomainError::code("invalid_catalog_revision"));
        }
        if revision < current {
            return Err(DomainError::code("catalog_rollback"));
        }
        self.validate_paths()?;
        tx.execute(
            "INSERT INTO catalog_revision VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision",
            [revision],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        Ok(())
    }
}

/// Open the actual file object without following a final link and check its
/// link count. SQLite NOFOLLOW rejects symlinks, but cannot reject hardlinks.
fn open_regular_nofollow(path: &Path) -> Result<fs::File, DomainError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Query an opened reparse object instead of following it. Sharing must
        // allow already-open SQLite connections; this handle is strictly read-only.
        options.custom_flags(0x0020_0000).share_mode(1 | 2 | 4);
    }
    let file = options.open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(DomainError::code("invalid_trust_state_path"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(DomainError::code("linked_state_path"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(DomainError::code("linked_path"));
        }
        // Same ABI/layout as install::regular_file's existing handle check.
        #[repr(C)]
        struct FileInformation {
            attributes: u32,
            times: [u32; 6],
            volume: u32,
            size_high: u32,
            size_low: u32,
            links: u32,
            index_high: u32,
            index_low: u32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandle(
                handle: *mut std::ffi::c_void,
                info: *mut FileInformation,
            ) -> i32;
        }
        let mut information: FileInformation = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        if information.links != 1 {
            return Err(DomainError::code("linked_state_path"));
        }
    }
    Ok(file)
}
fn io_error(error: io::Error) -> DomainError {
    let text = error.to_string();
    if !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        DomainError::code(text)
    } else {
        DomainError::new("io_error", text)
    }
}

fn sql_error(error: rusqlite::Error) -> DomainError {
    DomainError::new("trust_store_error", error.to_string())
}

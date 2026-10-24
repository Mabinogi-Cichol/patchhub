//! Request-idempotency ledger `installer/rust-v1/requests.sqlite`
//! (p0-contracts §3.1, p1-host-contract §5.1).
//!
//! Frozen binding tuple: `(instance_id, selection_id|release_id, operation,
//! resolved content_id)`. This is a persistence skeleton only; the protocol
//! layer decides idempotent replay vs `request_id_binding_conflict`.

use super::{invalid, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Stable identity for new unified-route plans, without adding journal fields.
pub(crate) fn request_plan_id(request: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"patchhub-request-plan-v1\0");
    hash.update(request.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

const MAX_ID_BYTES: usize = 256;
const MAX_VALUE_BYTES: usize = 512;

/// Frozen binding tuple (p1-host-contract §5.1).
///
/// `selection_id` and `release_id` are mutually exclusive; `content_id` is
/// optional and may be back-filled once a plan resolves the content identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestBinding {
    pub instance_id: String,
    pub selection_id: Option<String>,
    pub release_id: Option<String>,
    pub operation: String,
    pub content_id: Option<String>,
}

/// One ledger row: the binding plus the terminal summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestRecord {
    pub request_id: String,
    pub binding: RequestBinding,
    pub transaction_id: Option<String>,
    pub plan_id: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Opens/creates `requests.sqlite` and exposes `record`/`get`.
#[derive(Debug, Clone)]
pub struct RequestLedger {
    path: PathBuf,
}

/// Independent request ownership lock; never a journal or game lock.
/// OS release on host death makes interrupted requests recoverable.
pub struct RequestClaim(std::fs::File);
impl RequestClaim {
    pub fn acquire(paths: &super::InstallerPaths, id: &str) -> Result<Self> {
        use fs2::FileExt;
        use sha2::{Digest, Sha256};
        if id.is_empty() || id.len() > MAX_ID_BYTES {
            return Err(invalid("invalid_request_binding"));
        }
        let path = paths
            .jobs_dir()
            .join(format!("request-{:x}.lock", Sha256::digest(id.as_bytes())));
        if !super::zip_recipe::no_links(&path) {
            return Err(invalid("linked_installer_state"));
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).read(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let file = options.open(path)?;
        file.try_lock_exclusive()
            .map_err(|_| invalid("install_request_in_progress_or_interrupted"))?;
        Ok(Self(file))
    }
}
impl Drop for RequestClaim {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

impl RequestLedger {
    pub fn new(paths: &super::InstallerPaths) -> Self {
        Self {
            path: paths.requests(),
        }
    }

    /// Opens the ledger, ensuring the schema exists (idempotent).
    pub fn open(paths: &super::InstallerPaths) -> Result<Self> {
        let ledger = Self::new(paths);
        ensure_requests_db(&ledger.path)?;
        Ok(ledger)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Inserts a binding/terminal row, or updates the existing row for the same
    /// `request_id` (back-filling `content_id` / terminal summary). The original
    /// `created_at` is preserved on update.
    pub fn record(&self, record: &RequestRecord) -> Result<()> {
        validate_record(record)?;
        let binding = &record.binding;
        let connection = self.connect()?;
        let changed=connection
            .execute(
                "INSERT INTO requests(
                    request_id, instance_id, selection_id, release_id, operation,
                    content_id, transaction_id, plan_id, state, error,
                    created_at, updated_at
                ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
                ON CONFLICT(request_id) DO UPDATE SET
                    instance_id=excluded.instance_id,
                    selection_id=excluded.selection_id,
                    release_id=excluded.release_id,
                    operation=excluded.operation,
                    content_id=excluded.content_id,
                    transaction_id=excluded.transaction_id,
                    plan_id=excluded.plan_id,
                    state=excluded.state,
                    error=excluded.error,
                    updated_at=excluded.updated_at
                WHERE requests.instance_id=excluded.instance_id
                    AND requests.selection_id IS excluded.selection_id
                    AND requests.release_id IS excluded.release_id
                    AND requests.operation=excluded.operation
                    AND (requests.content_id IS NULL OR requests.content_id IS excluded.content_id)",
                params![
                    record.request_id,
                    binding.instance_id,
                    binding.selection_id,
                    binding.release_id,
                    binding.operation,
                    binding.content_id,
                    record.transaction_id,
                    record.plan_id,
                    record.state,
                    record.error,
                    record.created_at as i64,
                    record.updated_at as i64,
                ],
            )
            .map_err(|error| invalid(format!("requests_error:{error}")))?;
        if changed != 1 {
            return Err(invalid("request_id_binding_conflict"));
        }
        Ok(())
    }

    /// Returns the stored row for `request_id`, if any.
    pub fn get(&self, request_id: &str) -> Result<Option<RequestRecord>> {
        if request_id.is_empty() {
            return Err(invalid("invalid_request_binding"));
        }
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT request_id, instance_id, selection_id, release_id, operation,
                        content_id, transaction_id, plan_id, state, error,
                        created_at, updated_at
                 FROM requests WHERE request_id=?1",
                [request_id],
                |row| {
                    Ok(RequestRecord {
                        request_id: row.get(0)?,
                        binding: RequestBinding {
                            instance_id: row.get(1)?,
                            selection_id: row.get(2)?,
                            release_id: row.get(3)?,
                            operation: row.get(4)?,
                            content_id: row.get(5)?,
                        },
                        transaction_id: row.get(6)?,
                        plan_id: row.get(7)?,
                        state: row.get(8)?,
                        error: row.get(9)?,
                        created_at: row.get::<_, i64>(10)? as u64,
                        updated_at: row.get::<_, i64>(11)? as u64,
                    })
                },
            )
            .optional()
            .map_err(|error| invalid(format!("requests_error:{error}")))
    }

    fn connect(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&self.path)
            .map_err(|error| invalid(format!("requests_error:{error}")))?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
            )
            .map_err(|error| invalid(format!("requests_error:{error}")))?;
        Ok(connection)
    }
}

/// Creates/ensures the `requests` table at `path` (used by `init_atomic`).
pub(crate) fn create_requests_db(path: &Path) -> Result<()> {
    ensure_requests_db(path)
}

fn ensure_requests_db(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection =
        Connection::open(path).map_err(|error| invalid(format!("requests_error:{error}")))?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS requests(
                 request_id     TEXT PRIMARY KEY NOT NULL,
                 instance_id    TEXT NOT NULL,
                 selection_id   TEXT,
                 release_id     TEXT,
                 operation      TEXT NOT NULL,
                 content_id     TEXT,
                 transaction_id TEXT,
                 plan_id        TEXT,
                 state          TEXT,
                 error          TEXT,
                 created_at     INTEGER NOT NULL,
                 updated_at     INTEGER NOT NULL
             );",
        )
        .map_err(|error| invalid(format!("requests_error:{error}")))?;
    Ok(())
}

fn validate_record(record: &RequestRecord) -> Result<()> {
    if record.request_id.is_empty() || record.request_id.len() > MAX_ID_BYTES {
        return Err(invalid("invalid_request_binding"));
    }
    let binding = &record.binding;
    if binding.instance_id.is_empty()
        || binding.instance_id.len() > super::service::MAX_INSTANCE_ID_BYTES
    {
        return Err(invalid("invalid_request_binding"));
    }
    if binding.operation.is_empty() || binding.operation.len() > MAX_VALUE_BYTES {
        return Err(invalid("invalid_request_binding"));
    }
    let selection = binding.selection_id.as_deref().unwrap_or("");
    let release = binding.release_id.as_deref().unwrap_or("");
    if selection.is_empty() == release.is_empty() {
        // Exactly one of selection_id | release_id must be set.
        return Err(invalid("invalid_request_binding"));
    }
    for value in [
        selection,
        release,
        binding.content_id.as_deref().unwrap_or(""),
    ] {
        if value.len() > MAX_VALUE_BYTES {
            return Err(invalid("invalid_request_binding"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::{init_atomic, InstallerPaths};

    fn record(selection: Option<&str>, release: Option<&str>) -> RequestRecord {
        RequestRecord {
            request_id: "req-1".to_string(),
            binding: RequestBinding {
                instance_id: "steam-76561198000000000".to_string(),
                selection_id: selection.map(str::to_string),
                release_id: release.map(str::to_string),
                operation: "install".to_string(),
                content_id: None,
            },
            transaction_id: None,
            plan_id: None,
            state: None,
            error: None,
            created_at: 100,
            updated_at: 100,
        }
    }

    #[test]
    fn request_claim_is_exclusive_and_released() {
        let temp = tempfile::tempdir().unwrap();
        let paths = super::super::InstallerPaths::new(temp.path());
        super::super::init_atomic(&paths).unwrap();
        let first = RequestClaim::acquire(&paths, "same-request").unwrap();
        assert_eq!(
            RequestClaim::acquire(&paths, "same-request")
                .err()
                .unwrap()
                .to_string(),
            "install_request_in_progress_or_interrupted"
        );
        let unrelated = RequestClaim::acquire(&paths, "other-request").unwrap();
        drop(unrelated);
        drop(first);
        assert!(RequestClaim::acquire(&paths, "same-request").is_ok());
    }
    #[test]
    fn ledger_round_trips_and_backfills_content_id() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        init_atomic(&paths).unwrap();
        let ledger = RequestLedger::open(&paths).unwrap();

        let first = record(Some("exe-selection-abc-100"), None);
        ledger.record(&first).unwrap();
        assert_eq!(ledger.get("req-1").unwrap(), Some(first.clone()));

        // Back-fill the resolved content id and terminal summary.
        let mut terminal = first.clone();
        terminal.binding.content_id = Some("exe-content-def-100".to_string());
        terminal.transaction_id = Some("tx-1".to_string());
        terminal.state = Some("committed".to_string());
        terminal.updated_at = 200;
        ledger.record(&terminal).unwrap();

        let stored = ledger.get("req-1").unwrap().unwrap();
        assert_eq!(
            stored.binding.content_id.as_deref(),
            Some("exe-content-def-100")
        );
        assert_eq!(stored.state.as_deref(), Some("committed"));
        assert_eq!(stored.created_at, 100, "created_at survives updates");
        assert_eq!(stored.updated_at, 200);

        assert_eq!(ledger.get("missing").unwrap(), None);
    }

    #[test]
    fn ledger_rejects_ambiguous_or_empty_bindings() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        init_atomic(&paths).unwrap();
        let ledger = RequestLedger::open(&paths).unwrap();

        // Both set.
        assert!(ledger.record(&record(Some("sel"), Some("rel"))).is_err());
        // Neither set.
        assert!(ledger.record(&record(None, None)).is_err());

        // Release-only is accepted.
        let mut release_only = record(None, Some("patchmoe-100"));
        release_only.request_id = "req-2".to_string();
        ledger.record(&release_only).unwrap();
        assert_eq!(
            ledger
                .get("req-2")
                .unwrap()
                .unwrap()
                .binding
                .release_id
                .as_deref(),
            Some("patchmoe-100")
        );
    }
}

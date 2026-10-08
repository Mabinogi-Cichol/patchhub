//! Foreground task ownership before acquisition; durable revocation precedes effective cancellation.
use super::{invalid, store, InstallerPaths, Result};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[derive(Clone)]
pub struct TaskControl {
    pub id: String,
    job: PathBuf,
    pub cancel: Arc<AtomicBool>,
    deadline: Arc<AtomicU64>,
    plan: Arc<Mutex<Option<PathBuf>>>,
    failure: Arc<Mutex<Option<String>>>,
    /// Owned `tasks/<id>/owner.lock` handle. Held for the task lifetime and
    /// released before the terminal directory is removed (Windows refuses to
    /// delete a file that is still open).
    owner_lock: Arc<Mutex<Option<File>>>,
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn marker(job: &std::path::Path) -> Result<()> {
    fs::create_dir_all(job)?;
    let path = job.join("revoked");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    file.write_all(b"revoked\n")?;
    file.sync_all()?;
    #[cfg(unix)]
    fs::File::open(job)?.sync_all()?;
    Ok(())
}

/// Reclaims abandoned `tasks/<uuid>/` directories left by a crash.
///
/// A task directory with an `owner.lock` is deleted only when the lock can be
/// acquired (no live owner); a directory without `owner.lock` is deleted only
/// when it is older than one hour, closing the window between `create_dir` and
/// the lock file in [`TaskControl::new`]. Per-entry failures are skipped so one
/// bad directory cannot block the sweep. A missing `tasks/` is not an error.
pub fn cleanup_stale_tasks(paths: &InstallerPaths) -> Result<()> {
    let entries = match fs::read_dir(paths.tasks_dir()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let lock_path = path.join("owner.lock");
        match OpenOptions::new().read(true).write(true).open(&lock_path) {
            Ok(file) => {
                let unlocked = file.try_lock_exclusive().is_ok();
                drop(file);
                if unlocked {
                    let _ = fs::remove_dir_all(&path);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if directory_is_stale(&path) {
                    let _ = fs::remove_dir_all(&path);
                }
            }
            Err(_) => {}
        }
    }
    Ok(())
}

/// Whether a directory's modification time is at least an hour in the past.
fn directory_is_stale(path: &std::path::Path) -> bool {
    let Ok(modified) = fs::metadata(path).and_then(|metadata| metadata.modified()) else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age.as_secs() > 3600)
        .unwrap_or(false)
}

impl TaskControl {
    pub fn new(
        paths: &InstallerPaths,
        request: &str,
        instance: &str,
        selection: &str,
        deadline: u64,
    ) -> Result<Self> {
        // Best-effort sweep of crash leftovers before allocating a new task.
        let _ = cleanup_stale_tasks(paths);
        let id = uuid::Uuid::new_v4().to_string();
        let job = paths.task(&id)?;
        fs::create_dir_all(paths.tasks_dir())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(paths.tasks_dir(), fs::Permissions::from_mode(0o700))?;
        }
        fs::create_dir(&job)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&job, fs::Permissions::from_mode(0o700))?;
        }
        // The exclusive lock proves sole ownership of this id and doubles as the
        // liveness marker for `cleanup_stale_tasks`.
        let owner_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(job.join("owner.lock"))?;
        owner_lock
            .try_lock_exclusive()
            .map_err(|error| invalid(format!("task_id_conflict: {error}")))?;
        store::write_atomic(
            &job.join("task.json"),
            &serde_json::to_vec(&serde_json::json!({
            "schema_version":1,"task_id":id,"request_id":request,"instance_id":instance,"selection_id":selection,
            "created_at_ms":now_ms()}))?,
            65536,
        )?;
        let control = Self {
            id,
            job,
            cancel: Arc::new(AtomicBool::new(false)),
            deadline: Arc::new(AtomicU64::new(deadline)),
            plan: Arc::new(Mutex::new(None)),
            failure: Arc::new(Mutex::new(None)),
            owner_lock: Arc::new(Mutex::new(Some(owner_lock))),
        };
        control.renew(deadline)?;
        Ok(control)
    }
    pub fn job(&self) -> &std::path::Path {
        &self.job
    }
    pub fn attach_plan(&self, paths: &InstallerPaths, id: &str) -> Result<()> {
        let mut plan = self.plan.lock().unwrap();
        *plan = Some(paths.job(id)?);
        if self.cancel.load(Ordering::SeqCst) {
            marker(plan.as_ref().unwrap())?;
        }
        let lease = serde_json::to_vec(
            &serde_json::json!({"deadline_ms":self.deadline.load(Ordering::Acquire),"renewed_at":now_ms()}),
        )?;
        store::write_atomic(&plan.as_ref().unwrap().join("lease.json"), &lease, 4096)?;
        Ok(())
    }
    pub fn renew(&self, deadline: u64) -> Result<()> {
        self.deadline.store(deadline, Ordering::Release);
        let bytes =
            serde_json::to_vec(&serde_json::json!({"deadline_ms":deadline,"renewed_at":now_ms()}))?;
        store::write_atomic(&self.job.join("lease.json"), &bytes, 4096)?;
        if let Some(plan) = self.plan.lock().unwrap().as_ref() {
            store::write_atomic(&plan.join("lease.json"), &bytes, 4096)?;
        }
        Ok(())
    }
    pub fn revoke(&self) -> Result<()> {
        let plan = self.plan.lock().unwrap();
        let result = marker(&self.job).and_then(|_| match plan.as_ref() {
            Some(p) => marker(p),
            None => Ok(()),
        });
        if let Err(e) = result {
            *self.failure.lock().unwrap() = Some(format!("durable_revoke_failed: {e}"));
            // Even a storage failure is fail-closed; it never permits subsequent authorization/apply.
            self.cancel.store(true, Ordering::SeqCst);
            return Err(e);
        }
        self.cancel.store(true, Ordering::SeqCst);
        Ok(())
    }
    pub fn check(&self) -> Result<()> {
        if let Some(e) = self.failure.lock().unwrap().as_ref() {
            return Err(invalid(e));
        }
        if self.cancel.load(Ordering::SeqCst) {
            return Err(invalid("cancelled"));
        }
        Ok(())
    }
    pub fn check_requested(
        &self,
        requested: &AtomicBool,
        deadline: &dyn Fn() -> u64,
    ) -> Result<()> {
        if requested.load(Ordering::Acquire) || now_ms() >= deadline() {
            self.revoke()?;
        }
        self.check()
    }
    pub fn run<T>(
        &self,
        requested: &AtomicBool,
        deadline: &(dyn Fn() -> u64 + Sync),
        work: impl FnOnce() -> T,
    ) -> T {
        let done = AtomicBool::new(false);
        struct Finish<'a>(&'a AtomicBool);
        impl Drop for Finish<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let mut last = 0;
                while !done.load(Ordering::Acquire) {
                    let d = deadline();
                    if requested.load(Ordering::Acquire) || now_ms() >= d {
                        let _ = self.revoke();
                        break;
                    }
                    if d != last {
                        if let Err(e) = self.renew(d) {
                            *self.failure.lock().unwrap() =
                                Some(format!("lease_persist_failed: {e}"));
                            let _ = self.revoke();
                            break;
                        }
                        last = d;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            });
            // Synchronous first gate closes the monitor scheduling race.
            if requested.load(Ordering::Acquire) || now_ms() >= deadline() {
                let _ = self.revoke();
            }
            let finish = Finish(&done);
            let original = crate::operation::cancel_flag();
            let durable = self.clone();
            let result = crate::operation::with_cancel(self.cancel.clone(), || {
                crate::operation::with_gate(
                    move || {
                        if original.as_ref().is_some_and(|c| c.load(Ordering::Acquire)) {
                            durable.revoke()?;
                        }
                        durable.check()
                    },
                    work,
                )
            });
            drop(finish);
            done.store(true, Ordering::Release);
            worker.join().expect("task supervisor panicked");
            // Terminal state: release the id lock, then reclaim this task's
            // transient directory. Failures are non-fatal (a later sweep retries).
            self.owner_lock.lock().unwrap().take();
            let _ = fs::remove_dir_all(&self.job);
            result
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operation_gate_persists_before_abort_and_rollback_ignores_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(dir.path());
        super::super::init_atomic(&paths).unwrap();
        let control = TaskControl::new(&paths, "gate", "instance", "selection", u64::MAX).unwrap();
        let requested = Arc::new(AtomicBool::new(false));
        crate::operation::run(
            requested.clone(),
            |_| {},
            || {
                control.run(&requested, &|| u64::MAX, || {
                    requested.store(true, Ordering::Release);
                    assert!(crate::operation::check().is_err());
                    assert!(control.job.join("revoked").is_file());
                    assert!(control.cancel.load(Ordering::Acquire));
                    crate::operation::restoring();
                    assert!(crate::operation::check().is_ok());
                })
            },
        );
    }
    #[test]
    fn revoked_storage_failure_never_authorizes_work() {
        let dir = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(dir.path());
        super::super::init_atomic(&paths).unwrap();
        let control =
            TaskControl::new(&paths, "failure", "instance", "selection", u64::MAX).unwrap();
        fs::create_dir(control.job.join("revoked")).unwrap();
        assert!(control.revoke().is_err());
        assert!(control.cancel.load(Ordering::Acquire));
        assert!(control
            .check()
            .unwrap_err()
            .to_string()
            .starts_with("durable_revoke_failed"));
    }
    #[test]
    fn durable_marker_precedes_effective_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(dir.path());
        super::super::init_atomic(&paths).unwrap();
        let control = TaskControl::new(
            &paths,
            "req",
            "instance",
            "exe-selection-x",
            now_ms() + 15000,
        )
        .unwrap();
        let plan = uuid::Uuid::new_v4().to_string();
        fs::create_dir(paths.job(&plan).unwrap()).unwrap();
        control.attach_plan(&paths, &plan).unwrap();
        control.revoke().unwrap();
        assert!(control.cancel.load(Ordering::SeqCst));
        assert!(control.job.join("revoked").exists());
        assert!(paths.job(&plan).unwrap().join("revoked").exists());
        assert_eq!(control.check().unwrap_err().to_string(), "cancelled");
    }
    #[test]
    fn eof_or_expired_lease_cancels_before_work() {
        let dir = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(dir.path());
        super::super::init_atomic(&paths).unwrap();
        let control = TaskControl::new(&paths, "req", "instance", "selection", 1).unwrap();
        control.run(&AtomicBool::new(false), &|| 1, || {
            assert!(control.job.join("revoked").exists());
            assert!(control.check().is_err());
        });
    }

    #[test]
    fn terminal_run_reclaims_its_task_dir() {
        let dir = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(dir.path());
        super::super::init_atomic(&paths).unwrap();
        let control = TaskControl::new(&paths, "req", "instance", "selection", u64::MAX).unwrap();
        let id = control.id.clone();
        assert!(paths.task(&id).unwrap().join("owner.lock").is_file());
        let outcome = control.run(&AtomicBool::new(false), &|| u64::MAX, || 42);
        assert_eq!(outcome, 42);
        assert!(
            !paths.task(&id).unwrap().exists(),
            "terminal task directory must be reclaimed"
        );
    }

    #[test]
    fn cleanup_stale_tasks_reclaims_unlocked_and_spares_locked() {
        let dir = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(dir.path());
        super::super::init_atomic(&paths).unwrap();
        let tasks = paths.tasks_dir();
        fs::create_dir_all(&tasks).unwrap();

        let free = tasks.join("free-task");
        fs::create_dir(&free).unwrap();
        fs::write(free.join("owner.lock"), b"").unwrap();

        let held = tasks.join("held-task");
        fs::create_dir(&held).unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(held.join("owner.lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();

        cleanup_stale_tasks(&paths).unwrap();
        assert!(!free.exists(), "unlocked task directory must be reclaimed");
        assert!(held.exists(), "lock-held task directory must be spared");
        drop(lock);
    }

    #[test]
    fn cleanup_stale_tasks_spares_recent_unowned_and_tolerates_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(dir.path());
        super::super::init_atomic(&paths).unwrap();

        // Missing tasks/ is not an error and creates nothing.
        cleanup_stale_tasks(&paths).unwrap();
        assert!(!paths.tasks_dir().exists());

        // A freshly created directory without owner.lock is inside the
        // create_dir → lock window and must not be removed.
        let recent = paths.tasks_dir().join("recent-task");
        fs::create_dir_all(&recent).unwrap();
        cleanup_stale_tasks(&paths).unwrap();
        assert!(recent.exists(), "recent unowned directory must be spared");
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_stale_tasks_reclaims_old_unowned_dir() {
        let dir = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(dir.path());
        super::super::init_atomic(&paths).unwrap();

        let old = paths.tasks_dir().join("old-task");
        fs::create_dir_all(&old).unwrap();
        let handle = File::open(&old).unwrap();
        handle
            .set_modified(SystemTime::now() - Duration::from_secs(7200))
            .unwrap();
        drop(handle);

        cleanup_stale_tasks(&paths).unwrap();
        assert!(
            !old.exists(),
            "unowned directory older than one hour must be reclaimed"
        );
    }
}

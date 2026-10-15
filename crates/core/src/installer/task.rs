//! Foreground task ownership before acquisition; durable revocation precedes effective cancellation.
use super::outcome::CancellationCause;
use super::{invalid, store, InstallerPaths, Result};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct TaskControl {
    pub id: String,
    job: PathBuf,
    pub cancel: Arc<AtomicBool>,
    plan: Arc<Mutex<Option<PathBuf>>>,
    failure: Arc<Mutex<Option<String>>>,
    cause: Arc<Mutex<Option<CancellationCause>>>,
    /// Owned `tasks/<id>/owner.lock` handle. Held for the task lifetime and
    /// released before the terminal directory is removed (Windows refuses to
    /// delete a file that is still open).
    owner_lock: Arc<Mutex<Option<File>>>,
    owned_dir: Arc<Mutex<Option<super::cache::OwnedTaskDirectory>>>,
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

impl TaskControl {
    pub fn new(
        paths: &InstallerPaths,
        request: &str,
        instance: &str,
        selection: &str,
    ) -> Result<Self> {
        // Abandoned tasks are reclaimed only by CacheService under cache exclusive.
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
        // liveness marker for the cache service reclaimer.
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
            job: job.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            plan: Arc::new(Mutex::new(None)),
            failure: Arc::new(Mutex::new(None)),
            cause: Arc::new(Mutex::new(None)),
            owned_dir: Arc::new(Mutex::new(Some(super::cache::OwnedTaskDirectory::capture(
                &job,
            )?))),
            owner_lock: Arc::new(Mutex::new(Some(owner_lock))),
        };
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
        Ok(())
    }
    pub fn cancellation_cause(&self) -> Option<CancellationCause> {
        *self.cause.lock().unwrap()
    }
    pub fn revoke(&self) -> Result<()> {
        self.revoke_with_cause(CancellationCause::UserCancelled)
    }
    pub fn revoke_with_cause(&self, cause: CancellationCause) -> Result<()> {
        self.cause.lock().unwrap().get_or_insert(cause);
        self.revoke_for_failure()
    }
    /// Fail closed after an ordinary failure without inventing user cancellation.
    pub fn revoke_for_failure(&self) -> Result<()> {
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
            return Err(invalid(
                self.cancellation_cause()
                    .map(CancellationCause::code)
                    .unwrap_or("cancelled"),
            ));
        }
        Ok(())
    }
    pub fn check_requested(&self, requested: &AtomicBool) -> Result<()> {
        self.check_requested_with_cause(requested, &|| None)
    }
    pub fn check_requested_with_cause(
        &self,
        requested: &AtomicBool,
        cause: &dyn Fn() -> Option<CancellationCause>,
    ) -> Result<()> {
        if let Some(cause) = cause() {
            self.revoke_with_cause(cause)?;
        } else if requested.load(Ordering::Acquire) {
            self.revoke()?;
        }
        self.check()
    }
    pub fn run<T>(&self, requested: &AtomicBool, work: impl FnOnce() -> T) -> T {
        self.run_with_cause(requested, &|| None, work)
    }
    pub fn run_with_cause<T>(
        &self,
        requested: &AtomicBool,
        cause: &(dyn Fn() -> Option<CancellationCause> + Sync),
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
                while !done.load(Ordering::Acquire) {
                    if cause().is_some() || requested.load(Ordering::Acquire) {
                        let reason = cause().unwrap_or(CancellationCause::UserCancelled);
                        let _ = self.revoke_with_cause(reason);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            });
            // Synchronous first gate closes the monitor scheduling race.
            if cause().is_some() || requested.load(Ordering::Acquire) {
                let reason = cause().unwrap_or(CancellationCause::UserCancelled);
                let _ = self.revoke_with_cause(reason);
            }
            let finish = Finish(&done);
            let original = crate::operation::cancel_flag();
            let durable = self.clone();
            let result = crate::operation::with_cancel(self.cancel.clone(), || {
                crate::operation::with_gate(
                    move || {
                        if original.as_ref().is_some_and(|c| c.load(Ordering::Acquire)) {
                            // The borrowed host cause is read by the scoped monitor and
                            // terminal route; this static gate must not invent a cause.
                            durable.revoke_for_failure()?;
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
            if let Some(directory) = self.owned_dir.lock().unwrap().take() {
                directory.cleanup();
            }
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
        let control = TaskControl::new(&paths, "gate", "instance", "selection").unwrap();
        let requested = Arc::new(AtomicBool::new(false));
        crate::operation::run(
            requested.clone(),
            |_| {},
            || {
                control.run(&requested, || {
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
            TaskControl::new(&paths, "failure", "instance", "selection").unwrap();
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
        let control = TaskControl::new(&paths, "req", "instance", "exe-selection-x").unwrap();
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
    fn requested_cancel_before_work_marks_revocation() {
        let dir = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(dir.path());
        super::super::init_atomic(&paths).unwrap();
        let control = TaskControl::new(&paths, "req", "instance", "selection").unwrap();
        control.run(&AtomicBool::new(true), || {
            assert!(control.job.join("revoked").exists());
            assert!(control.check().is_err());
        });
    }

    #[test]
    fn typed_user_cause_survives_task_teardown() {
        for (requested, external, expected) in [
            (true, None, CancellationCause::UserCancelled),
            (
                false,
                Some(CancellationCause::UserCancelled),
                CancellationCause::UserCancelled,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let paths = InstallerPaths::new(dir.path());
            super::super::init_atomic(&paths).unwrap();
            let control = TaskControl::new(&paths, "typed", "instance", "selection").unwrap();
            control.run_with_cause(
                &AtomicBool::new(requested),
                &|| external,
                || {
                    assert_eq!(control.check().unwrap_err().to_string(), expected.code());
                    assert!(control.job.join("revoked").exists());
                },
            );
            assert_eq!(control.cancellation_cause(), Some(expected));
            assert!(!control.job.exists());
        }
    }

    #[test]
    fn terminal_run_reclaims_its_task_dir() {
        let dir = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(dir.path());
        super::super::init_atomic(&paths).unwrap();
        let control = TaskControl::new(&paths, "req", "instance", "selection").unwrap();
        let id = control.id.clone();
        assert!(paths.task(&id).unwrap().join("owner.lock").is_file());
        let outcome = control.run(&AtomicBool::new(false), || 42);
        assert_eq!(outcome, 42);
        assert!(
            !paths.task(&id).unwrap().exists(),
            "terminal task directory must be reclaimed"
        );
    }
}

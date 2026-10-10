//! All mutation tests are confined to synthetic Steam libraries inside TempDir.
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
};
use steam_patch_core::{
    catalog::{FileMapping, Fingerprint, PatchRelease, Platform, Recipe, SupportLevel},
    install::*,
    steam,
};
use zip::{write::SimpleFileOptions, ZipWriter};

#[test]
fn cancellation_after_first_live_write_restores_all_originals() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use steam_patch_core::operation;
    let fixture = Fixture::new();
    let plan = fixture.plan().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let result = operation::run(
        cancel,
        move |p| {
            if p.stage == "writing" && p.files_done == 1 {
                signal.store(true, Ordering::Relaxed);
            }
        },
        || apply(&plan.plan_id, &fixture.state),
    )
    .unwrap();
    assert_eq!(result.state, "rolled_back");
    assert_eq!(result.error.as_deref(), Some("task_cancelled"));
    assert_eq!(
        fs::read(fixture.game.join("original.dat")).unwrap(),
        b"original bytes"
    );
    assert!(!fixture.game.join("content/new.dat").exists());
    assert_eq!(
        list_transactions(&fixture.state).unwrap()[0].state,
        "rolled_back"
    );
}

#[test]
fn cancellation_inside_large_backup_or_write_cleans_partial_and_rolls_back() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use steam_patch_core::operation;
    for stage in ["backing_up", "writing"] {
        let mut fixture = Fixture::new();
        let original = vec![1u8; 1024 * 1024];
        let patched = vec![2u8; 2 * 1024 * 1024];
        fs::write(fixture.game.join("original.dat"), &original).unwrap();
        let mapping = &mut fixture.release.recipe.as_mut().unwrap().files[0];
        mapping.original_sha256 = Some(hash(&original));
        mapping.installed_sha256 = hash(&patched);
        fixture.archive_with(&[("replace.dat", &patched), ("add.dat", b"new bytes")]);
        let plan = fixture.plan().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let result = operation::run(
            cancel,
            move |p| {
                if p.stage == stage && p.bytes_done > 0 && p.bytes_done < p.total_bytes.unwrap() {
                    signal.store(true, Ordering::Relaxed);
                }
            },
            || apply(&plan.plan_id, &fixture.state),
        )
        .unwrap();
        assert_eq!(result.state, "rolled_back", "{stage}: {:?}", result.error);
        assert_eq!(result.error.as_deref(), Some("task_cancelled"));
        assert_eq!(
            fs::read(fixture.game.join("original.dat")).unwrap(),
            original
        );
        assert!(!fixture.game.join("content/new.dat").exists());
        assert!(fs::read_dir(&fixture.game).unwrap().all(|item| !item
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }
}

#[test]
fn cancelled_preparation_removes_staging_and_discarded_plan_cannot_apply() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use steam_patch_core::operation;
    let fixture = Fixture::new();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let error = operation::run(
        cancel,
        move |p| {
            if p.stage == "validating" && p.total_files == Some(2) && p.bytes_done > 0 {
                signal.store(true, Ordering::Relaxed);
            }
        },
        || fixture.plan(),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "task_cancelled");
    assert_eq!(fs::read_dir(fixture.state.join("jobs")).unwrap().count(), 0);
    let plan = fixture.plan().unwrap();
    discard_plan(&plan.plan_id, &fixture.state).unwrap();
    assert_eq!(
        apply(&plan.plan_id, &fixture.state)
            .unwrap_err()
            .to_string(),
        "plan_already_consumed"
    );
    assert_eq!(
        fs::read(fixture.game.join("original.dat")).unwrap(),
        b"original bytes"
    );
}

fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

struct Fixture {
    _temp: tempfile::TempDir,
    library: PathBuf,
    game: PathBuf,
    state: PathBuf,
    archive: PathBuf,
    release: PatchRelease,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("Steam 测试库");
        let game = library.join("steamapps/common/Synthetic Game");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("original.dat"), b"original bytes").unwrap();
        let state = temp.path().join("state");
        let archive = temp.path().join("patch.zip");
        let mut fixture = Self {
            _temp: temp,
            library,
            game,
            state,
            archive,
            release: PatchRelease {
                patch_id: "fixture".into(),
                release_id: "fixture-v1".into(),
                appid: 42,
                name: "Synthetic patch".into(),
                version: "1".into(),
                language: "zh-CN".into(),
                category: "content_restore".into(),
                source_page: "https://example.invalid/fixture".into(),
                support_level: SupportLevel::ManagedZip,
                artifact: None,
                platforms: vec![Platform::Windows, Platform::Steamos],
                verified_buildids: vec!["100".into()],
                detection: vec![Fingerprint {
                    path: "content/new.dat".into(),
                    sha256: hash(b"new bytes"),
                }],
                recipe: Some(Recipe {
                    artifact_sha256: String::new(),
                    files: vec![
                        FileMapping {
                            source: "replace.dat".into(),
                            destination: "original.dat".into(),
                            original_sha256: Some(hash(b"original bytes")),
                            installed_sha256: hash(b"patched bytes"),
                        },
                        FileMapping {
                            source: "add.dat".into(),
                            destination: "content/new.dat".into(),
                            original_sha256: None,
                            installed_sha256: hash(b"new bytes"),
                        },
                    ],
                }),
            },
        };
        fixture.manifest("100", "4");
        fixture.archive_with(&[("replace.dat", b"patched bytes"), ("add.dat", b"new bytes")]);
        fixture
    }
    fn manifest(&self, build: &str, flags: &str) {
        fs::write(self.library.join("steamapps/appmanifest_42.acf"), format!(r#""AppState" {{ "appid" "42" "name" "Synthetic Game" "installdir" "Synthetic Game" "buildid" "{build}" "StateFlags" "{flags}" }}"#)).unwrap();
    }
    fn archive_with(&mut self, members: &[(&str, &[u8])]) {
        let mut zip = ZipWriter::new(File::create(&self.archive).unwrap());
        for (name, contents) in members {
            zip.start_file(
                *name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap();
        self.rehash_archive();
    }
    fn rehash_archive(&mut self) {
        self.release.recipe.as_mut().unwrap().artifact_sha256 =
            hash(&fs::read(&self.archive).unwrap());
    }
    fn plan(&self) -> std::io::Result<InstallPlan> {
        let install = steam::scan(std::slice::from_ref(&self.library))
            .installs
            .remove(0);
        prepare(&install, &self.release, &self.archive, &self.state, 1)
    }
    // Upgrade samples retain legacy recovery semantics; new plans always clean.
    fn legacy_plan(&self) -> std::io::Result<InstallPlan> {
        let plan = self.plan()?;
        self.edit_journal(&plan.plan_id, |job| {
            job.as_object_mut().unwrap().remove("transient_recovery");
        });
        Ok(plan)
    }
    fn complete_legacy_uninstall(&self, id: &str) -> std::io::Result<Transaction> {
        if let Some(transaction) = list_transactions(&self.state)?
            .into_iter()
            .find(|t| t.plan_id == id && t.state == "uninstalled")
        {
            return Ok(transaction);
        }
        // Model an existing older journal that already persisted uninstall intent.
        self.edit_journal(id, |job| {
            job["transaction"]["state"] = "uninstalling".into();
            job["uninstall_requested"] = true.into();
        });
        Ok(recover(&self.state)?
            .into_iter()
            .find(|t| t.plan_id == id)
            .unwrap())
    }
    fn reinstall(&self) -> std::io::Result<InstallPlan> {
        let game = steam::scan(std::slice::from_ref(&self.library))
            .installs
            .remove(0);
        prepare_with_operation(
            &game,
            &self.release,
            &self.archive,
            &self.state,
            1,
            InstallOperation::Reinstall,
        )
    }
    fn overwrite(&self) -> std::io::Result<InstallPlan> {
        let game = steam::scan(std::slice::from_ref(&self.library))
            .installs
            .remove(0);
        prepare_with_operation(
            &game,
            &self.release,
            &self.archive,
            &self.state,
            1,
            InstallOperation::Overwrite,
        )
    }
    fn job_file(&self, id: &str, name: &str) -> PathBuf {
        self.state.join("jobs").join(id).join(name)
    }
    fn temp_file(&self, id: &str, index: usize, destination: &str) -> PathBuf {
        self.game
            .join(destination)
            .parent()
            .unwrap()
            .join(format!(".steam-patch-{id}-{index}.tmp"))
    }
    fn edit_journal(&self, id: &str, edit: impl FnOnce(&mut serde_json::Value)) {
        let db = rusqlite::Connection::open(self.state.join("journal.sqlite")).unwrap();
        let text: String = db
            .query_row("SELECT document FROM jobs WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        edit(&mut value);
        db.execute(
            "UPDATE jobs SET document=?2 WHERE id=?1",
            rusqlite::params![id, value.to_string()],
        )
        .unwrap();
    }
    fn assert_original(&self) {
        assert_eq!(
            fs::read(self.game.join("original.dat")).unwrap(),
            b"original bytes"
        );
        assert!(!self.game.join("content/new.dat").exists());
    }
}

#[test]
fn legacy_uninstall_recovery_is_persistent_idempotent_and_preserves_unrelated_files() {
    let f = Fixture::new();
    fs::write(f.game.join("save.sav"), b"user save").unwrap();
    let plan = f.legacy_plan().unwrap();
    assert!(plan.blockers.is_empty());
    f.assert_original();
    assert!(plan.disk_requirement > 0);
    assert_eq!(apply(&plan.plan_id, &f.state).unwrap().state, "committed");
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"patched bytes"
    );
    assert_eq!(
        fs::read(f.game.join("content/new.dat")).unwrap(),
        b"new bytes"
    );
    assert_eq!(apply(&plan.plan_id, &f.state).unwrap().state, "committed");
    assert_eq!(list_transactions(&f.state).unwrap().len(), 1);
    assert_eq!(
        f.complete_legacy_uninstall(&plan.plan_id).unwrap().state,
        "uninstalled"
    );
    assert_eq!(
        f.complete_legacy_uninstall(&plan.plan_id).unwrap().state,
        "uninstalled"
    );
    f.assert_original();
    assert!(!f.game.join("content").exists());
    assert_eq!(fs::read(f.game.join("save.sav")).unwrap(), b"user save");
}

#[test]
fn unmanaged_baseline_conflicts_and_stale_plans_never_write() {
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    fs::write(f.game.join("original.dat"), b"new user bytes").unwrap();
    assert!(apply(&plan.plan_id, &f.state)
        .unwrap_err()
        .to_string()
        .contains("file_changed"));
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"new user bytes"
    );
    let blocked = f.plan().unwrap();
    assert!(!blocked.blockers.is_empty());
    assert!(apply(&blocked.plan_id, &f.state).is_err());
    f.edit_journal(&plan.plan_id, |j| {
        j["plan"]["created_at"] = 1.into();
        j["plan"]["expires_at"] = 2.into();
    });
    assert!(apply(&plan.plan_id, &f.state)
        .unwrap_err()
        .to_string()
        .contains("plan_expired"));
}

#[test]
fn steam_update_or_downloading_prevents_apply_and_old_backup_restore() {
    let f = Fixture::new();
    let plan = f.legacy_plan().unwrap();
    f.manifest("100", "6");
    assert!(apply(&plan.plan_id, &f.state).is_err());
    f.manifest("100", "4");
    assert_eq!(apply(&plan.plan_id, &f.state).unwrap().state, "committed");
    f.manifest("101", "4");
    assert_eq!(
        f.complete_legacy_uninstall(&plan.plan_id).unwrap().state,
        "needs_recovery"
    );
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"patched bytes"
    );
}

#[test]
fn user_modification_and_corrupt_backup_block_the_entire_restore() {
    for corrupt_backup in [false, true] {
        let f = Fixture::new();
        let plan = f.legacy_plan().unwrap();
        apply(&plan.plan_id, &f.state).unwrap();
        if corrupt_backup {
            fs::write(f.job_file(&plan.plan_id, "0.backup"), b"bad backup").unwrap();
        } else {
            fs::write(f.game.join("original.dat"), b"user mod").unwrap();
        }
        let result = f.complete_legacy_uninstall(&plan.plan_id).unwrap();
        assert_eq!(result.state, "needs_recovery");
        assert_eq!(
            fs::read(f.game.join("content/new.dat")).unwrap(),
            b"new bytes"
        );
        assert_eq!(recover(&f.state).unwrap()[0].state, "needs_recovery");
        if !corrupt_backup {
            assert_eq!(fs::read(f.game.join("original.dat")).unwrap(), b"user mod");
        }
    }
}

#[test]
fn stage_tampering_rolls_back_without_game_changes() {
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    fs::write(f.job_file(&plan.plan_id, "1.stage"), b"tampered stage").unwrap();
    let result = apply(&plan.plan_id, &f.state).unwrap();
    assert_eq!(result.state, "rolled_back");
    f.assert_original();
}

#[test]
fn managed_updates_are_explicitly_blocked_until_clean_uninstall() {
    let f = Fixture::new();
    let first = f.legacy_plan().unwrap();
    let second = f.legacy_plan().unwrap();
    apply(&first.plan_id, &f.state).unwrap();
    assert!(apply(&second.plan_id, &f.state).is_err());
    assert!(f
        .plan()
        .unwrap_err()
        .to_string()
        .contains("managed_install_exists"));
    f.complete_legacy_uninstall(&first.plan_id).unwrap();
    let new_plan = f.legacy_plan().unwrap();
    assert_eq!(
        apply(&new_plan.plan_id, &f.state).unwrap().state,
        "committed"
    );
}

#[test]
fn crash_after_backup_before_and_after_replacement_recovers() {
    for phase in ["backed_up", "writing", "installed", "restoring"] {
        let f = Fixture::new();
        let plan = f.plan().unwrap();
        fs::write(f.job_file(&plan.plan_id, "0.backup"), b"original bytes").unwrap();
        if ["installed", "restoring"].contains(&phase) {
            fs::write(f.game.join("original.dat"), b"patched bytes").unwrap();
        }
        f.edit_journal(&plan.plan_id, |j| {
            j["transaction"]["state"] = "applying".into();
            j["progress"] = serde_json::json!([phase, "backed_up"]);
        });
        let result = recover(&f.state).unwrap();
        assert_eq!(
            result[0].state, "rolled_back",
            "phase {phase}: {:?}",
            result[0]
        );
        f.assert_original();
        assert!(recover(&f.state).unwrap().is_empty());
    }
}

#[test]
fn interrupted_uninstall_finishes_and_removes_created_directories() {
    let f = Fixture::new();
    let plan = f.legacy_plan().unwrap();
    apply(&plan.plan_id, &f.state).unwrap();
    fs::remove_file(f.game.join("content/new.dat")).unwrap();
    f.edit_journal(&plan.plan_id, |j| {
        j["transaction"]["state"] = "uninstalling".into();
        j["uninstall_requested"] = true.into();
        j["progress"][1] = "restoring".into();
    });
    assert_eq!(recover(&f.state).unwrap()[0].state, "uninstalled");
    f.assert_original();
    assert!(!f.game.join("content").exists());
}

#[test]
fn recovery_preserves_files_changed_during_interruption() {
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    fs::write(f.job_file(&plan.plan_id, "0.backup"), b"original bytes").unwrap();
    fs::write(f.game.join("original.dat"), b"user mod after crash").unwrap();
    f.edit_journal(&plan.plan_id, |j| {
        j["transaction"]["state"] = "applying".into();
        j["progress"][0] = "writing".into();
    });
    assert_eq!(recover(&f.state).unwrap()[0].state, "needs_recovery");
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"user mod after crash"
    );
}

#[test]
fn interrupted_install_copy_accepts_only_verified_stage_prefixes() {
    for prefix in [b"pat".as_slice(), b"".as_slice()] {
        let f = Fixture::new();
        let plan = f.plan().unwrap();
        fs::write(f.job_file(&plan.plan_id, "0.backup"), b"original bytes").unwrap();
        let temp = f.temp_file(&plan.plan_id, 0, "original.dat");
        fs::write(&temp, prefix).unwrap();
        f.edit_journal(&plan.plan_id, |j| {
            j["transaction"]["state"] = "applying".into();
            j["progress"] = serde_json::json!(["writing", "backed_up"]);
        });
        assert_eq!(recover(&f.state).unwrap()[0].state, "rolled_back");
        assert!(!temp.exists());
        f.assert_original();
    }
}

#[test]
fn interrupted_uninstall_copy_accepts_verified_backup_prefix() {
    let f = Fixture::new();
    let plan = f.legacy_plan().unwrap();
    assert_eq!(apply(&plan.plan_id, &f.state).unwrap().state, "committed");
    let temp = f.temp_file(&plan.plan_id, 0, "original.dat");
    fs::write(&temp, b"orig").unwrap();
    f.edit_journal(&plan.plan_id, |j| {
        j["transaction"]["state"] = "uninstalling".into();
        j["uninstall_requested"] = true.into();
        j["progress"][0] = "restoring".into();
    });
    assert_eq!(recover(&f.state).unwrap()[0].state, "uninstalled");
    assert!(!temp.exists());
    f.assert_original();
}

#[test]
fn unrecognized_partial_temps_and_changed_sources_are_preserved() {
    for case in [
        "wrong_prefix",
        "wrong_progress",
        "changed_source",
        "missing_source",
    ] {
        let f = Fixture::new();
        let plan = f.plan().unwrap();
        fs::write(f.job_file(&plan.plan_id, "0.backup"), b"original bytes").unwrap();
        let temp = f.temp_file(&plan.plan_id, 0, "original.dat");
        let bytes = if case == "wrong_prefix" {
            b"wrong".as_slice()
        } else {
            b"pat".as_slice()
        };
        fs::write(&temp, bytes).unwrap();
        if case == "changed_source" {
            fs::write(f.job_file(&plan.plan_id, "0.stage"), b"changed stage").unwrap();
        } else if case == "missing_source" {
            fs::remove_file(f.job_file(&plan.plan_id, "0.stage")).unwrap();
        }
        f.edit_journal(&plan.plan_id, |j| {
            j["transaction"]["state"] = "applying".into();
            j["progress"] = serde_json::json!([
                if case == "wrong_progress" {
                    "backed_up"
                } else {
                    "writing"
                },
                "backed_up"
            ]);
        });
        assert_eq!(
            recover(&f.state).unwrap()[0].state,
            "needs_recovery",
            "{case}"
        );
        assert_eq!(fs::read(&temp).unwrap(), bytes, "{case}");
        f.assert_original();
    }
}

#[test]
fn preflight_checks_every_temp_before_restoring_any_live_file() {
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    fs::write(f.job_file(&plan.plan_id, "0.backup"), b"original bytes").unwrap();
    fs::create_dir(f.game.join("content")).unwrap();
    fs::write(f.game.join("content/new.dat"), b"new bytes").unwrap();
    let temp = f.temp_file(&plan.plan_id, 0, "original.dat");
    fs::write(&temp, b"unknown partial").unwrap();
    f.edit_journal(&plan.plan_id, |j| {
        j["transaction"]["state"] = "applying".into();
        j["progress"] = serde_json::json!(["writing", "installed"]);
    });
    assert_eq!(recover(&f.state).unwrap()[0].state, "needs_recovery");
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"original bytes"
    );
    assert_eq!(
        fs::read(f.game.join("content/new.dat")).unwrap(),
        b"new bytes"
    );
    assert_eq!(fs::read(temp).unwrap(), b"unknown partial");
}

#[test]
fn linked_temp_is_rejected_and_preserved() {
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    fs::write(f.job_file(&plan.plan_id, "0.backup"), b"original bytes").unwrap();
    let temp = f.temp_file(&plan.plan_id, 0, "original.dat");
    fs::hard_link(f.job_file(&plan.plan_id, "0.stage"), &temp).unwrap();
    f.edit_journal(&plan.plan_id, |j| {
        j["transaction"]["state"] = "applying".into();
        j["progress"][0] = "writing".into();
    });
    assert_eq!(recover(&f.state).unwrap()[0].state, "needs_recovery");
    assert_eq!(fs::read(temp).unwrap(), b"patched bytes");
    f.assert_original();
}

#[test]
fn corrupt_backup_blocks_recovery_before_other_file_is_removed() {
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    fs::write(f.job_file(&plan.plan_id, "0.backup"), b"partial backup").unwrap();
    fs::create_dir(f.game.join("content")).unwrap();
    fs::write(f.game.join("content/new.dat"), b"new bytes").unwrap();
    f.edit_journal(&plan.plan_id, |j| {
        j["transaction"]["state"] = "applying".into();
        j["progress"] = serde_json::json!(["backed_up", "installed"]);
    });
    assert_eq!(recover(&f.state).unwrap()[0].state, "needs_recovery");
    assert_eq!(
        fs::read(f.game.join("content/new.dat")).unwrap(),
        b"new bytes"
    );
    assert_eq!(
        fs::read(f.job_file(&plan.plan_id, "0.backup")).unwrap(),
        b"partial backup"
    );
}

#[test]
fn zip_traversal_ntfs_names_and_unmapped_malicious_entries_are_rejected() {
    for bad in [
        "../escape",
        "/absolute",
        "C:/escape",
        "file:ads",
        "CON.txt",
        "dir/aux",
        "dir./file",
        "x\\y",
        "NUL",
        ".steam-patch-forged",
    ] {
        let mut f = Fixture::new();
        f.archive_with(&[
            ("replace.dat", b"patched bytes"),
            ("add.dat", b"new bytes"),
            (bad, b"attack"),
        ]);
        assert!(f.plan().is_err(), "accepted {bad}");
        f.assert_original();
    }
}

#[test]
fn zip_case_collisions_and_file_directory_collisions_are_rejected() {
    for extra in [
        vec![("Replace.dat", &b"x"[..])],
        vec![("Folder/a", &b"x"[..]), ("folder/b", &b"y"[..])],
        vec![("some", &b"x"[..]), ("some/child", &b"y"[..])],
    ] {
        let mut f = Fixture::new();
        let mut entries = vec![
            ("replace.dat", &b"patched bytes"[..]),
            ("add.dat", &b"new bytes"[..]),
        ];
        entries.extend(extra);
        f.archive_with(&entries);
        assert!(f.plan().is_err());
    }
}

#[test]
fn zip_symlink_is_rejected_and_high_ratio_can_stage_with_real_space() {
    let mut f = Fixture::new();
    let mut zip = ZipWriter::new(File::create(&f.archive).unwrap());
    zip.add_symlink("link", "../outside", SimpleFileOptions::default())
        .unwrap();
    zip.finish().unwrap();
    f.rehash_archive();
    assert!(f.plan().unwrap_err().to_string().contains("zip_link"));
    f.assert_original();

    let payload = vec![0; 2 * 1024 * 1024];
    let mut zip = ZipWriter::new(File::create(&f.archive).unwrap());
    for (name, bytes) in [
        ("replace.dat", payload.as_slice()),
        ("add.dat", &b"new bytes"[..]),
    ] {
        zip.start_file(
            name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    f.rehash_archive();
    f.release.recipe.as_mut().unwrap().files[0].installed_sha256 = hash(&payload);
    let plan = f.plan().unwrap();
    assert!(plan.blockers.is_empty());
    assert_eq!(apply(&plan.plan_id, &f.state).unwrap().state, "committed");
    assert_eq!(fs::read(f.game.join("original.dat")).unwrap(), payload);
}

#[test]
fn artifact_and_mapped_file_hashes_are_required() {
    let mut f = Fixture::new();
    f.release.recipe.as_mut().unwrap().artifact_sha256 = hash(b"wrong");
    assert!(f
        .plan()
        .unwrap_err()
        .to_string()
        .contains("artifact_hash_mismatch"));
    f.rehash_archive();
    f.release.recipe.as_mut().unwrap().files[0].installed_sha256 = hash(b"wrong");
    assert!(f.plan().unwrap_err().to_string().contains("installed_hash"));
    f.assert_original();
}

#[test]
fn tampered_journal_paths_and_ids_cannot_escape_game_or_state() {
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    assert!(get_plan("../../outside", &f.state).is_err());
    f.edit_journal(&plan.plan_id, |j| {
        j["plan"]["changes"][0]["path"] = "../../outside".into();
    });
    assert!(apply(&plan.plan_id, &f.state).is_err());
    f.assert_original();
}

#[test]
fn forged_install_outside_steam_common_and_overlapping_state_are_rejected() {
    let f = Fixture::new();
    let mut install = steam::scan(std::slice::from_ref(&f.library))
        .installs
        .remove(0);
    assert!(prepare(&install, &f.release, &f.archive, &f.game.join("state"), 1).is_err());
    assert!(!f.game.join("state").exists());
    install.install_path = f.library.clone();
    assert!(prepare(&install, &f.release, &f.archive, &f.state, 1).is_err());
}

#[test]
fn hard_link_destination_is_rejected() {
    let f = Fixture::new();
    fs::hard_link(f.game.join("original.dat"), f.game.join("alias.dat")).unwrap();
    assert!(f
        .plan()
        .unwrap_err()
        .to_string()
        .contains("hard_link_rejected"));
}

#[test]
fn concurrent_state_writer_is_rejected() {
    use fs2::FileExt;
    let f = Fixture::new();
    fs::create_dir(&f.state).unwrap();
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(f.state.join("journal.lock"))
        .unwrap();
    file.lock_exclusive().unwrap();
    assert!(f.plan().unwrap_err().to_string().contains("installer_busy"));
}

#[cfg(unix)]
#[test]
fn replacing_and_restoring_preserves_executable_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::set_permissions(
        f.game.join("original.dat"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let plan = f.legacy_plan().unwrap();
    assert_eq!(apply(&plan.plan_id, &f.state).unwrap().state, "committed");
    assert_eq!(
        fs::metadata(f.game.join("original.dat"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        f.complete_legacy_uninstall(&plan.plan_id).unwrap().state,
        "uninstalled"
    );
    assert_eq!(
        fs::metadata(f.game.join("original.dat"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}

#[cfg(unix)]
#[test]
fn symlinked_game_ancestor_or_target_is_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let outside = f._temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, f.game.join("content")).unwrap();
    assert!(f.plan().is_err());
    let alias = f._temp.path().join("alias");
    symlink(&f.library, &alias).unwrap();
    let mut install = steam::scan(std::slice::from_ref(&f.library))
        .installs
        .remove(0);
    install.library_path = alias;
    assert!(prepare(&install, &f.release, &f.archive, &f.state, 1).is_err());
}

#[test]
fn legacy_committed_uninstall_intent_is_recovered() {
    let f = Fixture::new();
    let plan = f.legacy_plan().unwrap();
    apply(&plan.plan_id, &f.state).unwrap();
    f.edit_journal(&plan.plan_id, |j| {
        j["uninstall_requested"] = true.into();
    });
    assert_eq!(recover(&f.state).unwrap()[0].state, "uninstalled");
    f.assert_original();
}

#[test]
fn fixed_ntfs_timestamps_are_ignored_but_other_ntfs_metadata_is_rejected() {
    for case in 0..5 {
        let safe = case == 0;
        let mut f = Fixture::new();
        let mut timestamps = vec![0u8; 32];
        timestamps[4] = 1;
        timestamps[6] = 24;
        match case {
            1 => timestamps[4] = 2,
            2 => timestamps[0] = 1,
            3 => {
                timestamps.pop();
            }
            4 => {
                timestamps.push(0);
            }
            _ => (),
        }
        let mut options = zip::write::FullFileOptions::default();
        options.add_extra_data(0xbeef, &timestamps, false).unwrap();
        let mut archive = ZipWriter::new(File::create(&f.archive).unwrap());
        for (name, data) in [
            ("replace.dat", b"patched bytes".as_slice()),
            ("add.dat", b"new bytes".as_slice()),
        ] {
            archive.start_file(name, options.clone()).unwrap();
            archive.write_all(data).unwrap();
        }
        archive.finish().unwrap();
        let mut bytes = fs::read(&f.archive).unwrap();
        for index in 0..bytes.len().saturating_sub(3) {
            if bytes[index..index + 4] == [0xef, 0xbe, timestamps.len() as u8, 0] {
                bytes[index..index + 2].copy_from_slice(&[10, 0]);
            }
        }
        fs::write(&f.archive, bytes).unwrap();
        f.rehash_archive();
        if safe {
            assert!(f.plan().is_ok());
        } else {
            assert!(f.plan().unwrap_err().to_string().contains("zip"));
        }
        f.assert_original();
    }
}

#[test]
fn successful_install_and_reinstall_clean_temporaries_and_keep_receipts() {
    let f = Fixture::new();
    let first = f.plan().unwrap();
    assert_eq!(apply(&first.plan_id, &f.state).unwrap().state, "committed");
    assert!(!f.job_file(&first.plan_id, "0.backup").exists());
    assert!(!f.job_file(&first.plan_id, "1.stage").exists());
    assert!(recover(&f.state).unwrap().is_empty());
    let second = f.reinstall().unwrap();
    assert!(second.blockers.is_empty());
    assert_eq!(apply(&second.plan_id, &f.state).unwrap().state, "committed");
    let records = list_transactions(&f.state).unwrap();
    assert_eq!(records.iter().filter(|t| t.state == "committed").count(), 1);
    assert!(records
        .iter()
        .any(|t| t.plan_id == first.plan_id && t.state == "superseded"));
    assert!(!f.job_file(&second.plan_id, "0.backup").exists());
}

#[test]
fn same_content_reinstall_failure_before_backups_rolls_back() {
    let f = Fixture::new();
    let first = f.plan().unwrap();
    assert_eq!(apply(&first.plan_id, &f.state).unwrap().state, "committed");
    let second = f.reinstall().unwrap();
    fs::write(f.job_file(&second.plan_id, "0.stage"), b"invalid stage").unwrap();
    let result = apply(&second.plan_id, &f.state).unwrap();
    assert_eq!(result.state, "rolled_back");
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"patched bytes"
    );
    assert_eq!(
        fs::read(f.game.join("content/new.dat")).unwrap(),
        b"new bytes"
    );
    assert!(recover(&f.state).unwrap().is_empty());
    assert!(list_transactions(&f.state)
        .unwrap()
        .iter()
        .any(|t| t.plan_id == first.plan_id && t.state == "committed"));
}

#[test]
fn owned_missing_and_damaged_files_reinstall_and_failed_retry_preserves_preop_and_receipt() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use steam_patch_core::operation;
    let f = Fixture::new();
    let first = f.plan().unwrap();
    apply(&first.plan_id, &f.state).unwrap();
    fs::write(f.game.join("original.dat"), b"damaged current bytes").unwrap();
    fs::remove_file(f.game.join("content/new.dat")).unwrap();
    let second = f.reinstall().unwrap();
    assert_eq!(
        second.changes[0].original_sha256,
        Some(hash(b"damaged current bytes"))
    );
    assert_eq!(second.changes[1].original_sha256, None);
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let transaction = operation::run(
        cancel,
        move |p| {
            if p.stage == "writing" && p.files_done == 1 {
                signal.store(true, Ordering::Relaxed);
            }
        },
        || apply(&second.plan_id, &f.state),
    )
    .unwrap();
    assert_eq!(transaction.state, "rolled_back");
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"damaged current bytes"
    );
    assert!(!f.game.join("content/new.dat").exists());
    assert!(list_transactions(&f.state)
        .unwrap()
        .iter()
        .any(|t| t.plan_id == first.plan_id && t.state == "committed"));
    assert!(!f.job_file(&second.plan_id, "0.backup").exists());
    let third = f.reinstall().unwrap();
    assert_eq!(apply(&third.plan_id, &f.state).unwrap().state, "committed");
}

#[test]
fn reinstall_rejects_changed_content_and_unowned_damage() {
    let mut f = Fixture::new();
    fs::write(f.game.join("original.dat"), b"patched bytes").unwrap();
    assert_eq!(
        f.reinstall().unwrap_err().to_string(),
        "reinstall_unowned_or_incomplete"
    );
    fs::write(f.game.join("original.dat"), b"original bytes").unwrap();
    let plan = f.plan().unwrap();
    apply(&plan.plan_id, &f.state).unwrap();
    f.release.recipe.as_mut().unwrap().files[0].destination = "save.sav".into();
    assert_eq!(
        f.reinstall().unwrap_err().to_string(),
        "reinstall_content_identity_changed"
    );
    assert!(!f.game.join("save.sav").exists());
}

#[test]
fn revoked_marker_alone_blocks_apply_and_during_write_rolls_back() {
    use std::sync::{atomic::AtomicBool, Arc};
    use steam_patch_core::operation;
    let f = Fixture::new();
    let first = f.plan().unwrap();
    fs::write(f.job_file(&first.plan_id, "revoked"), b"").unwrap();
    assert_eq!(
        apply(&first.plan_id, &f.state).unwrap_err().to_string(),
        "plan_revoked"
    );
    let second = f.plan().unwrap();
    let marker = f.job_file(&second.plan_id, "revoked");
    let result = operation::run(
        Arc::new(AtomicBool::new(false)),
        move |p| {
            if p.stage == "writing" && p.files_done == 1 {
                fs::write(&marker, b"").unwrap();
            }
        },
        || apply(&second.plan_id, &f.state),
    )
    .unwrap();
    assert_eq!(result.state, "rolled_back");
    f.assert_original();
}

#[test]
fn migration_revokes_old_plans_once_and_retains_unfinished_legacy_recovery() {
    let f = Fixture::new();
    let old = f.legacy_plan().unwrap();
    migrate_state(&f.state).unwrap();
    assert!(f.job_file(&old.plan_id, "revoked").exists());
    let fresh = f.plan().unwrap();
    migrate_state(&f.state).unwrap();
    assert!(!f.job_file(&fresh.plan_id, "revoked").exists());
    assert_eq!(apply(&fresh.plan_id, &f.state).unwrap().state, "committed");

    let legacy = Fixture::new();
    let old = legacy.legacy_plan().unwrap();
    apply(&old.plan_id, &legacy.state).unwrap();
    legacy.edit_journal(&old.plan_id, |job| {
        job["transaction"]["state"] = "uninstalling".into();
        job["uninstall_requested"] = true.into();
    });
    migrate_state(&legacy.state).unwrap();
    assert!(legacy.job_file(&old.plan_id, "0.backup").exists());
    assert_eq!(recover(&legacy.state).unwrap()[0].state, "uninstalled");
    legacy.assert_original();
    assert!(!legacy.job_file(&old.plan_id, "0.backup").exists());
}

#[test]
fn migration_cleans_legacy_success_but_damaged_terminal_state_never_deletes_material() {
    let f = Fixture::new();
    let old = f.legacy_plan().unwrap();
    apply(&old.plan_id, &f.state).unwrap();
    migrate_state(&f.state).unwrap();
    assert!(!f.job_file(&old.plan_id, "0.backup").exists());
    assert_eq!(list_transactions(&f.state).unwrap()[0].state, "committed");

    let damaged = Fixture::new();
    let old = damaged.legacy_plan().unwrap();
    damaged.edit_journal(&old.plan_id, |job| {
        job["transaction"]["state"] = "committed".into();
    });
    assert_eq!(
        migrate_state(&damaged.state).unwrap_err().to_string(),
        "journal_invalid_terminal"
    );
    assert!(damaged.job_file(&old.plan_id, "0.stage").exists());
}

#[test]
fn committed_success_wins_over_late_cancel_and_cleanup_failure_can_retry() {
    use std::sync::{atomic::AtomicBool, Arc};
    use steam_patch_core::operation;
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    // An unexpected directory in a derived cleanup slot is retained and recorded.
    fs::create_dir(f.job_file(&plan.plan_id, "1.backup")).unwrap();
    let committed = apply(&plan.plan_id, &f.state).unwrap();
    assert_eq!(committed.state, "committed");
    assert!(committed
        .error
        .as_deref()
        .unwrap()
        .contains("cleanup_pending"));
    fs::remove_dir(f.job_file(&plan.plan_id, "1.backup")).unwrap();
    migrate_state(&f.state).unwrap();
    assert!(!f.job_file(&plan.plan_id, "1.stage").exists());
    assert_eq!(list_transactions(&f.state).unwrap()[0].error, None);
    fs::write(f.job_file(&plan.plan_id, "revoked"), b"").unwrap();
    let repeated = operation::run(
        Arc::new(AtomicBool::new(true)),
        |_| (),
        || apply(&plan.plan_id, &f.state),
    )
    .unwrap();
    assert_eq!(repeated.state, "committed");
}

#[test]
fn unfinished_legacy_transaction_blocks_reinstall_without_journal_discard() {
    let f = Fixture::new();
    let first = f.legacy_plan().unwrap();
    apply(&first.plan_id, &f.state).unwrap();
    f.edit_journal(&first.plan_id, |job| {
        job["transaction"]["state"] = "needs_recovery".into();
    });
    let original_journal = fs::read(f.state.join("journal.sqlite")).unwrap();
    assert!(f
        .reinstall()
        .unwrap_err()
        .to_string()
        .contains("managed_install_exists_or_needs_recovery"));
    assert_eq!(
        fs::read(f.state.join("journal.sqlite")).unwrap(),
        original_journal
    );
    assert!(f.job_file(&first.plan_id, "0.backup").exists());
}

#[test]
fn cancellation_at_planning_boundary_never_leaves_executable_plan() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use steam_patch_core::operation;
    let f = Fixture::new();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let result = operation::run(
        cancel,
        move |p| {
            if p.stage == "planning" {
                signal.store(true, Ordering::Relaxed);
            }
        },
        || f.plan(),
    );
    assert_eq!(result.unwrap_err().to_string(), "task_cancelled");
    assert!(list_transactions(&f.state).unwrap().is_empty());
    assert_eq!(fs::read_dir(f.state.join("jobs")).unwrap().count(), 0);
    f.assert_original();
}

#[test]
fn committed_install_survives_cleanup_diagnostic_journal_write_failure() {
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    fs::create_dir(f.job_file(&plan.plan_id, "1.backup")).unwrap();
    // The final install commit succeeds; only writes against the committed
    // receipt fail, simulating a full disk while persisting cleanup diagnostics.
    let db = rusqlite::Connection::open(f.state.join("journal.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_cleanup BEFORE UPDATE ON jobs WHEN json_extract(OLD.document, '$.transaction.state') = 'committed' BEGIN SELECT RAISE(FAIL, 'simulated_disk_full'); END;").unwrap();
    drop(db);
    let transaction = apply(&plan.plan_id, &f.state).unwrap();
    assert_eq!(transaction.state, "committed");
    assert!(transaction
        .error
        .as_deref()
        .unwrap()
        .contains("cleanup_pending"));
    assert_eq!(list_transactions(&f.state).unwrap()[0].state, "committed");
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"patched bytes"
    );
    assert_eq!(
        fs::read(f.game.join("content/new.dat")).unwrap(),
        b"new bytes"
    );
}

#[test]
fn planned_change_source_is_optional_for_older_journals() {
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    assert!(
        plan.changes.iter().all(|change| change.source.is_some()),
        "new plans record the archive member"
    );
    // Model a journal written before `PlannedChange.source` existed.
    f.edit_journal(&plan.plan_id, |job| {
        for change in job["plan"]["changes"].as_array_mut().unwrap() {
            change.as_object_mut().unwrap().remove("source");
        }
    });
    let reloaded = get_plan(&plan.plan_id, &f.state).unwrap();
    assert!(reloaded
        .changes
        .iter()
        .all(|change| change.source.is_none()));
    assert_eq!(reloaded.changes.len(), plan.changes.len());
}

#[test]
fn overwrite_accepts_unowned_changed_targets_and_supersedes_old_receipt() {
    let mut f = Fixture::new();
    fs::write(f.game.join("original.dat"), b"external patch").unwrap();
    fs::write(f.game.join("keep-save.dat"), b"save data").unwrap();
    let first = f.overwrite().unwrap();
    assert!(first.blockers.is_empty());
    assert_eq!(
        first.changes[0].original_sha256.as_deref(),
        Some(hash(b"external patch").as_str())
    );
    assert_eq!(apply(&first.plan_id, &f.state).unwrap().state, "committed");
    fs::write(f.game.join("original.dat"), b"user modified").unwrap();
    f.release.release_id = "fixture-v2".into();
    f.release.recipe.as_mut().unwrap().files[0].installed_sha256 = hash(b"updated patch");
    f.archive_with(&[("replace.dat", b"updated patch"), ("add.dat", b"new bytes")]);
    let second = f.overwrite().unwrap();
    assert!(second.blockers.is_empty());
    assert_eq!(apply(&second.plan_id, &f.state).unwrap().state, "committed");
    assert_eq!(apply(&second.plan_id, &f.state).unwrap().state, "committed");
    let receipts = list_transactions(&f.state).unwrap();
    assert_eq!(
        receipts
            .iter()
            .find(|t| t.plan_id == first.plan_id)
            .unwrap()
            .state,
        "superseded"
    );
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"updated patch"
    );
    assert_eq!(
        fs::read(f.game.join("keep-save.dat")).unwrap(),
        b"save data"
    );
}

#[test]
fn cancelled_overwrite_restores_current_bytes_and_keeps_old_receipt() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let f = Fixture::new();
    let first = f.plan().unwrap();
    apply(&first.plan_id, &f.state).unwrap();
    fs::write(f.game.join("original.dat"), b"changed since old install").unwrap();
    fs::remove_file(f.game.join("content/new.dat")).unwrap();
    let second = f.overwrite().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let result = steam_patch_core::operation::run(
        cancel,
        move |p| {
            if p.stage == "writing" && p.files_done == 1 {
                signal.store(true, Ordering::Relaxed);
            }
        },
        || apply(&second.plan_id, &f.state),
    )
    .unwrap();
    assert_eq!(result.state, "rolled_back");
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"changed since old install"
    );
    assert!(!f.game.join("content/new.dat").exists());
    assert_eq!(
        list_transactions(&f.state)
            .unwrap()
            .iter()
            .find(|t| t.plan_id == first.plan_id)
            .unwrap()
            .state,
        "committed"
    );
}

#[test]
fn overwrite_still_blocks_unfinished_transaction_and_pre_apply_changes() {
    let f = Fixture::new();
    let first = f.overwrite().unwrap();
    f.edit_journal(&first.plan_id, |j| {
        j["transaction"]["state"] = "backing_up".into();
    });
    assert!(f
        .overwrite()
        .unwrap_err()
        .to_string()
        .contains("managed_install_exists_or_needs_recovery"));
    f.edit_journal(&first.plan_id, |j| {
        j["transaction"]["state"] = "planned".into();
    });
    fs::write(f.game.join("original.dat"), b"changed after prepare").unwrap();
    assert!(apply(&first.plan_id, &f.state)
        .unwrap_err()
        .to_string()
        .contains("file_changed"));
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"changed after prepare"
    );
}

#[test]
fn delta_writing_and_restoring_intents_recover_crash_and_partial_copy() {
    for state in ["writing", "restoring"] {
        for renamed in [false, true] {
            let f = Fixture::new();
            let plan = f.overwrite().unwrap();
            fs::write(f.job_file(&plan.plan_id, "0.backup"), b"original bytes").unwrap();
            f.edit_journal(&plan.plan_id, |j| {
                j["transaction"]["state"] = if state == "writing" {
                    "applying"
                } else {
                    "rolling_back"
                }
                .into();
                j["progress"] = serde_json::json!(["backed_up", "backed_up"]);
            });
            let db = rusqlite::Connection::open(f.state.join("journal.sqlite")).unwrap();
            db.execute(
                "INSERT INTO job_progress VALUES(?1,0,?2,1)",
                rusqlite::params![plan.plan_id, state],
            )
            .unwrap();
            db.execute(
                "INSERT INTO job_directories VALUES(?1,0,'content')",
                [&plan.plan_id],
            )
            .unwrap();
            drop(db);
            fs::create_dir(f.game.join("content")).unwrap();
            if state == "restoring" || renamed {
                fs::write(f.game.join("original.dat"), b"patched bytes").unwrap();
            }
            if !renamed {
                fs::write(
                    f.temp_file(&plan.plan_id, 0, "original.dat"),
                    if state == "writing" {
                        &b"patch"[..]
                    } else {
                        &b"origin"[..]
                    },
                )
                .unwrap();
            } else if state == "restoring" {
                fs::write(f.game.join("original.dat"), b"original bytes").unwrap();
            }
            assert_eq!(
                recover(&f.state).unwrap()[0].state,
                "rolled_back",
                "{state} renamed={renamed}"
            );
            f.assert_original();
            assert!(!f.game.join("content").exists());
            let db = rusqlite::Connection::open(f.state.join("journal.sqlite")).unwrap();
            let count:i64=db.query_row("SELECT (SELECT count(*) FROM job_progress)+(SELECT count(*) FROM job_directories)",[],|r|r.get(0)).unwrap();
            assert_eq!(count, 0);
        }
    }
}

#[test]
fn corrupt_delta_is_rejected_and_old_documents_need_no_overlay_schema() {
    let f = Fixture::new();
    let plan = f.plan().unwrap();
    let db = rusqlite::Connection::open(f.state.join("journal.sqlite")).unwrap();
    db.execute(
        "INSERT INTO job_progress VALUES(?1,900,'writing',1)",
        [&plan.plan_id],
    )
    .unwrap();
    assert!(get_plan_ro(&f.state, &plan.plan_id)
        .unwrap_err()
        .to_string()
        .contains("journal_invalid"));
    f.assert_original();
    db.execute("DELETE FROM job_progress", []).unwrap();
    db.execute(
        "INSERT INTO job_directories VALUES(?1,9,'content')",
        [&plan.plan_id],
    )
    .unwrap();
    assert!(get_plan_ro(&f.state, &plan.plan_id)
        .unwrap_err()
        .to_string()
        .contains("journal_invalid_directory"));
    db.execute("DELETE FROM job_directories", []).unwrap();
    drop(db);
    f.edit_journal(&plan.plan_id, |j| {
        j.as_object_mut().unwrap().remove("progress_format");
        j["plan"].as_object_mut().unwrap().remove("space_budget");
    });
    let db = rusqlite::Connection::open(f.state.join("journal.sqlite")).unwrap();
    db.execute_batch("DROP TABLE job_progress; DROP TABLE job_directories;")
        .unwrap();
    drop(db);
    assert_eq!(
        get_plan_ro(&f.state, &plan.plan_id).unwrap().changes.len(),
        2
    );
    assert_eq!(apply(&plan.plan_id, &f.state).unwrap().state, "committed");
}

#[test]
fn failed_commit_compaction_preserves_old_receipt_and_rollback_evidence() {
    let f = Fixture::new();
    let first = f.plan().unwrap();
    apply(&first.plan_id, &f.state).unwrap();
    fs::write(f.game.join("original.dat"), b"current external bytes").unwrap();
    let second = f.overwrite().unwrap();
    let db = rusqlite::Connection::open(f.state.join("journal.sqlite")).unwrap();
    db.execute_batch(&format!("CREATE TRIGGER fail_commit BEFORE UPDATE ON jobs WHEN NEW.id='{}' AND json_extract(NEW.document,'$.transaction.state')='committed' BEGIN SELECT RAISE(FAIL,'simulated_disk_full'); END;",second.plan_id)).unwrap();
    drop(db);
    let result = apply(&second.plan_id, &f.state).unwrap();
    assert_eq!(result.state, "rolled_back");
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"current external bytes"
    );
    assert_eq!(
        list_transactions(&f.state)
            .unwrap()
            .iter()
            .find(|t| t.plan_id == first.plan_id)
            .unwrap()
            .state,
        "committed"
    );
}

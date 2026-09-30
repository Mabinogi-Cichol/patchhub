use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
use steam_patch_core::{
    catalog::{self, Catalog, Platform, SupportLevel},
    download, install, steam,
    trust::{self, VerifiedCatalog},
};

#[derive(Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Params {
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    pub catalog: Option<PathBuf>,
    pub platform: Option<Platform>,
    pub instance_id: Option<String>,
    pub release_id: Option<String>,
    pub signed_catalog: Option<PathBuf>,
    pub trust_keys: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub archive: Option<PathBuf>,
    pub plan_id: Option<String>,
    pub transaction_id: Option<String>,
    #[serde(default)]
    pub confirmed_game_closed: bool,
}

pub const METHODS: &[&str] = &[
    "scan",
    "preview",
    "plan_install",
    "start_install",
    "fetch_artifact",
    "list_transactions",
    "uninstall",
    "recover",
];
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn value(data: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(data).map_err(err)
}
fn host_platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Steamos
    }
}
fn state(params: &Params) -> Result<PathBuf, String> {
    params
        .state_dir
        .as_ref()
        .map(|p| std::path::absolute(p).map_err(err))
        .unwrap_or_else(|| Err("state_dir_required".into()))
}

fn state_safe(path: &Path) -> Result<(), String> {
    for component in path.ancestors() {
        if component.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(component) {
            Ok(meta) => {
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    meta.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let linked = meta.file_type().is_symlink();
                if linked {
                    return Err("linked_state_path".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(err(e)),
        }
    }
    Ok(())
}

fn highest_revision(state: Option<&Path>) -> Result<u64, String> {
    let Some(state) = state else {
        return Ok(0);
    };
    let path = state.join("trust.sqlite");
    state_safe(&path)?;
    if !path.exists() {
        return Ok(0);
    }
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(err)?;
    conn.query_row(
        "SELECT revision FROM catalog_revision WHERE id=1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map_err(err)
    .and_then(|r| u64::try_from(r).map_err(err))
}

fn remember_revision(state: Option<&Path>, revision: u64) -> Result<(), String> {
    let Some(state) = state else {
        return Ok(());
    };
    state_safe(state)?;
    fs::create_dir_all(state).map_err(err)?;
    for name in ["trust.sqlite", "trust.sqlite-journal"] {
        state_safe(&state.join(name))?;
    }
    let mut conn = rusqlite::Connection::open(state.join("trust.sqlite")).map_err(err)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(err)?;
    conn.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS catalog_revision(id INTEGER PRIMARY KEY, revision INTEGER NOT NULL);").map_err(err)?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let current: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(revision),0) FROM catalog_revision",
            [],
            |row| row.get(0),
        )
        .map_err(err)?;
    let revision = i64::try_from(revision).map_err(err)?;
    if revision < current {
        return Err("catalog_rollback".into());
    }
    tx.execute("INSERT INTO catalog_revision VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision", [revision]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(())
}

fn signed(params: &Params) -> Result<VerifiedCatalog, String> {
    let path = params
        .signed_catalog
        .as_ref()
        .ok_or("signed_catalog_required")?;
    let keys = match &params.trust_keys {
        Some(path) => trust::load_keys(path).map_err(err)?,
        None => {
            serde_json::from_str(include_str!("../../../catalog/trust-keys.json")).map_err(err)?
        }
    };
    let catalog = VerifiedCatalog::load(
        path,
        &keys,
        trust::now().map_err(err)?,
        highest_revision(params.state_dir.as_deref())?,
    )
    .map_err(err)?;
    remember_revision(params.state_dir.as_deref(), catalog.catalog().revision)?;
    Ok(catalog)
}

fn ready_release<'a>(
    catalog: &'a VerifiedCatalog,
    release_id: &str,
    game: &steam::GameInstall,
    platform: Platform,
) -> Result<&'a catalog::PatchRelease, String> {
    catalog
        .ensure_active(release_id, trust::now().map_err(err)?)
        .map_err(err)?;
    let release = catalog
        .catalog()
        .releases
        .iter()
        .find(|r| r.release_id == release_id && r.appid == game.appid)
        .ok_or("release_not_matched")?;
    if !release.platforms.contains(&platform) {
        return Err("platform_not_verified".into());
    }
    if game.availability != "installed" {
        return Err("game_not_ready".into());
    }
    if !game
        .buildid
        .as_ref()
        .is_some_and(|b| release.verified_buildids.contains(b))
    {
        return Err("build_not_verified".into());
    }
    if release.support_level != SupportLevel::ManagedZip {
        return Err("source_requires_guidance".into());
    }
    Ok(release)
}

fn canonical_future(path: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(path).map_err(err)?;
    let mut base = absolute.as_path();
    let mut suffix = Vec::new();
    while !base.exists() {
        suffix.push(base.file_name().ok_or("invalid_state_path")?.to_owned());
        base = base.parent().ok_or("invalid_state_path")?;
    }
    let mut result = fs::canonicalize(base).map_err(err)?;
    for part in suffix.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}

fn validate_state_boundary(state: &Path, installs: &[steam::GameInstall]) -> Result<(), String> {
    state_safe(state)?;
    let state = canonical_future(state)?;
    for game in installs {
        let game = canonical_future(&game.install_path)?;
        if state.starts_with(&game) || game.starts_with(&state) {
            return Err("state_overlaps_game".into());
        }
    }
    Ok(())
}

pub fn execute(method: &str, params: Params) -> Result<Value, String> {
    if !METHODS.contains(&method) {
        return Err("unsupported_method".into());
    }
    if params.roots.len() > 32 {
        return Err("too_many_roots".into());
    }
    if method == "scan" && (params.instance_id.is_some() || params.release_id.is_some()) {
        return Err("unexpected_preview_arguments".into());
    }
    let roots = if params.roots.is_empty() {
        steam::discover_roots()
    } else {
        params.roots.clone()
    };
    let report = steam::scan(&roots);
    if let Some(state) = &params.state_dir {
        validate_state_boundary(state, &report.installs)?;
    }
    if ["start_install", "uninstall", "recover"].contains(&method) && !params.confirmed_game_closed
    {
        return Err("game_closed_confirmation_required".into());
    }
    if ["list_transactions", "uninstall", "recover"].contains(&method) {
        let state = state(&params)?;
        state_safe(&state)?;
        return match method {
            "list_transactions" if !state.join("journal.sqlite").exists() => Ok(json!([])),
            "list_transactions" => value(install::list_transactions(&state).map_err(err)?),
            "uninstall" => value(
                install::uninstall(
                    params
                        .transaction_id
                        .as_deref()
                        .ok_or("transaction_required")?,
                    &state,
                )
                .map_err(err)?,
            ),
            _ => value(install::recover(&state).map_err(err)?),
        };
    }
    let platform = params.platform.unwrap_or_else(host_platform);
    if method == "start_install" {
        let state = state(&params)?;
        let plan = install::get_plan(params.plan_id.as_deref().ok_or("plan_required")?, &state)
            .map_err(err)?;
        let current = report
            .installs
            .iter()
            .find(|i| i.instance_id == plan.instance_id)
            .ok_or("instance_not_found")?;
        let catalog = signed(&params)?;
        let release = ready_release(&catalog, &plan.release_id, current, platform)?;
        let recipe = release.recipe.as_ref().ok_or("recipe_missing")?;
        if plan.catalog_revision != catalog.catalog().revision
            || plan.install.buildid != current.buildid
            || plan.changes.len() != recipe.files.len()
            || plan
                .changes
                .iter()
                .zip(&recipe.files)
                .any(|(change, mapping)| {
                    change.path != mapping.destination
                        || change.original_sha256 != mapping.original_sha256
                        || change.installed_sha256 != mapping.installed_sha256
                })
        {
            return Err("plan_authorization_changed".into());
        }
        return value(install::apply(&plan.plan_id, &state).map_err(err)?);
    }
    if ["plan_install", "fetch_artifact"].contains(&method) {
        let state = state(&params)?;
        let game = report
            .installs
            .iter()
            .find(|i| Some(&i.instance_id) == params.instance_id.as_ref())
            .ok_or("instance_not_found")?;
        let catalog = signed(&params)?;
        let release = ready_release(
            &catalog,
            params.release_id.as_deref().ok_or("release_required")?,
            game,
            platform,
        )?;
        if method == "fetch_artifact" {
            let artifact = release
                .artifact
                .as_ref()
                .ok_or("artifact_requires_manual_download")?;
            let archive = download::fetch(
                artifact,
                &state.join("downloads"),
                &AtomicBool::new(false),
                |_, _| (),
            )
            .map_err(err)?;
            return Ok(json!({"archive":archive}));
        }
        let archive = params.archive.as_ref().ok_or("archive_required")?;
        return value(
            install::prepare(game, release, archive, &state, catalog.catalog().revision)
                .map_err(err)?,
        );
    }
    let mut catalog_error = None;
    let verified = if params.signed_catalog.is_some() {
        match signed(&params) {
            Ok(c) => Some(c),
            Err(e) => {
                catalog_error = Some(e);
                None
            }
        }
    } else {
        None
    };
    let catalog = match verified.as_ref() {
        Some(c) => c.catalog().clone(),
        None => match params.catalog.as_ref() {
            Some(path) => match catalog::load(path) {
                Ok(c) => c,
                Err(e) => {
                    catalog_error = Some(err(e));
                    Catalog {
                        schema_version: 1,
                        revision: 0,
                        releases: vec![],
                    }
                }
            },
            None => Catalog {
                schema_version: 1,
                revision: 0,
                releases: vec![],
            },
        },
    };
    if method == "preview" {
        let game = report
            .installs
            .iter()
            .find(|i| Some(&i.instance_id) == params.instance_id.as_ref())
            .ok_or("instance_not_found")?;
        return value(
            catalog::preview(
                game,
                &catalog,
                params.release_id.as_deref().ok_or("release_required")?,
                platform,
            )
            .map_err(err)?,
        );
    }
    let transactions = params
        .state_dir
        .as_ref()
        .filter(|s| s.join("journal.sqlite").exists())
        .map(|s| install::list_transactions(s).map_err(err))
        .transpose()?
        .unwrap_or_default();
    let rows: Vec<_> = report.installs.iter().map(|game| {
        let mut patches = catalog::matches(game, &catalog, platform);
        for patch in &mut patches {
            if verified.as_ref().is_some_and(|c| c.is_revoked(&patch.release_id)) { patch.blockers.push("release_revoked".into()); }
            if patch.detection.status == "installed_external" && transactions.iter().any(|t|
                t.instance_id == game.instance_id && t.release_id == patch.release_id && t.state == "committed") {
                patch.detection.status = "installed_managed".into();
            }
        }
        json!({"game":game,"match_status":if patches.is_empty(){"no_catalog_match"}else{"matched"},"patches":patches})
    }).collect();
    Ok(
        json!({"catalog_revision":catalog.revision,"catalog_trust":if verified.is_some(){"signed"}else{"unsigned_development"},
        "catalog_error":catalog_error,"platform":platform,"roots":roots,"installs":rows,"diagnostics":report.diagnostics}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rollback_is_rechecked_inside_write_transaction() {
        let state = tempfile::tempdir().unwrap();
        remember_revision(Some(state.path()), 1).unwrap();
        assert_eq!(highest_revision(Some(state.path())).unwrap(), 1);
        remember_revision(Some(state.path()), 3).unwrap();
        assert_eq!(
            remember_revision(Some(state.path()), 2).unwrap_err(),
            "catalog_rollback"
        );
        assert_eq!(highest_revision(Some(state.path())).unwrap(), 3);
    }
    #[test]
    fn recover_and_plan_reject_game_state_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("steamapps/common/Test");
        fs::create_dir_all(&game).unwrap();
        fs::write(temp.path().join("steamapps/appmanifest_42.acf"),
            "\"AppState\" { \"appid\" \"42\" \"name\" \"Test\" \"installdir\" \"Test\" \"StateFlags\" \"4\" }").unwrap();
        for method in [
            "recover",
            "plan_install",
            "fetch_artifact",
            "list_transactions",
        ] {
            let result = execute(
                method,
                Params {
                    roots: vec![temp.path().into()],
                    state_dir: Some(game.clone()),
                    confirmed_game_closed: true,
                    ..Default::default()
                },
            );
            assert_eq!(result.unwrap_err(), "state_overlaps_game");
            assert_eq!(fs::read_dir(&game).unwrap().count(), 0);
        }
    }
}

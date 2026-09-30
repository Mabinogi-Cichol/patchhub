use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signer, SigningKey};
use std::{fs, io::Write, path::Path};
use steam_patch_core::{
    catalog,
    trust::{self, CatalogPayload, SignedEnvelope, TrustKey, TrustKeys},
};

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

pub fn run(command: &str, args: &[String]) -> Result<(), String> {
    match (command, args) {
        ("keygen", [private, public, id]) => {
            let mut secret = [0u8; 32];
            getrandom::fill(&mut secret).map_err(|e| e.to_string())?;
            let key = SigningKey::from_bytes(&secret);
            let keys = TrustKeys { schema_version: 1, keys: vec![TrustKey {
                key_id: id.clone(), public_key: STANDARD.encode(key.verifying_key().to_bytes()), revoked: false,
            }] };
            keys.validate().map_err(|e| e.to_string())?;
            write_new(Path::new(private), STANDARD.encode(secret).as_bytes())?;
            write_new(Path::new(public), &serde_json::to_vec_pretty(&keys).map_err(|e| e.to_string())?)
        }
        ("sign-catalog", [catalog_path, private, id, output, seconds, revocations @ ..]) if revocations.len() <= 1 => {
            let catalog = catalog::load(Path::new(catalog_path)).map_err(|e| e.to_string())?;
            let secret = trust::read_bounded(Path::new(private), 128).map_err(|e| e.to_string())?;
            let secret: [u8; 32] = STANDARD.decode(secret).map_err(|e| e.to_string())?.try_into().map_err(|_| "invalid_private_key")?;
            let key = SigningKey::from_bytes(&secret);
            let generated_at = trust::now().map_err(|e| e.to_string())?;
            let lifetime: u64 = seconds.parse().map_err(|_| "invalid_lifetime")?;
            if lifetime == 0 || lifetime > 90 * 86400 { return Err("invalid_lifetime".into()); }
            let revoked_release_ids: Vec<String> = match revocations.first() {
                Some(path) => serde_json::from_slice(&trust::read_bounded(Path::new(path), 1024 * 1024).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?,
                None => vec![],
            };
            let mut unique = std::collections::HashSet::new();
            if revoked_release_ids.iter().any(|release| release.is_empty() || release.len() > 128 || !unique.insert(release)) {
                return Err("invalid_revocations".into());
            }
            let payload = serde_json::to_vec(&CatalogPayload { catalog, generated_at, expires_at: generated_at + lifetime, revoked_release_ids }).map_err(|e| e.to_string())?;
            let envelope = SignedEnvelope { schema_version: 1, key_id: id.clone(), signature: STANDARD.encode(key.sign(&payload).to_bytes()), payload: STANDARD.encode(payload) };
            write_new(Path::new(output), &serde_json::to_vec_pretty(&envelope).map_err(|e| e.to_string())?)
        }
        ("schema", [output]) => {
            let schema = schemars::schema_for!(CatalogPayload);
            write_new(Path::new(output), &serde_json::to_vec_pretty(&schema).map_err(|e| e.to_string())?)
        }
        _ => Err("usage: keygen PRIVATE PUBLIC KEY_ID | sign-catalog CATALOG PRIVATE KEY_ID OUTPUT LIFETIME_SECONDS [REVOCATIONS_JSON] | schema OUTPUT".into()),
    }
}

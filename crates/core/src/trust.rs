//! Signed exact-byte payloads, expiration, key/release revocation and rollback protection.
pub mod revision;
use crate::catalog::{Catalog, MAX_CATALOG_BYTES};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Read},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrustKeys {
    pub schema_version: u32,
    pub keys: Vec<TrustKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrustKey {
    pub key_id: String,
    pub public_key: String,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignedEnvelope {
    pub schema_version: u32,
    pub key_id: String,
    pub payload: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogPayload {
    pub catalog: Catalog,
    pub generated_at: u64,
    pub expires_at: u64,
    pub revoked_release_ids: Vec<String>,
}

#[derive(Debug)]
pub struct VerifiedCatalog {
    payload: CatalogPayload,
    key_id: String,
}

pub fn now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(|_| invalid("invalid_system_clock"))
}

fn invalid(code: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, code)
}

pub fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("file_too_large"));
    }
    Ok(bytes)
}

pub fn load_keys(path: &Path) -> io::Result<TrustKeys> {
    let keys: TrustKeys = serde_json::from_slice(&read_bounded(path, 65536)?)
        .map_err(|_| invalid("invalid_trust_keys"))?;
    keys.validate()?;
    Ok(keys)
}

impl TrustKeys {
    pub fn validate(&self) -> io::Result<()> {
        if self.schema_version != 1 || self.keys.len() > 64 {
            return Err(invalid("invalid_trust_keys"));
        }
        let mut ids = HashSet::new();
        for key in &self.keys {
            if key.key_id.is_empty() || key.key_id.len() > 128 || !ids.insert(&key.key_id) {
                return Err(invalid("invalid_key_id"));
            }
            let bytes = STANDARD
                .decode(&key.public_key)
                .map_err(|_| invalid("invalid_public_key"))?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| invalid("invalid_public_key"))?;
            VerifyingKey::from_bytes(&bytes).map_err(|_| invalid("invalid_public_key"))?;
        }
        Ok(())
    }
}

impl VerifiedCatalog {
    pub fn from_bytes(
        envelope_bytes: &[u8],
        keys: &TrustKeys,
        timestamp: u64,
        highest_revision: u64,
    ) -> io::Result<Self> {
        keys.validate()?;
        if envelope_bytes.len() as u64 > MAX_CATALOG_BYTES * 2 {
            return Err(invalid("catalog_too_large"));
        }
        let envelope: SignedEnvelope = serde_json::from_slice(envelope_bytes)
            .map_err(|_| invalid("invalid_signed_envelope"))?;
        if envelope.schema_version != 1 {
            return Err(invalid("unsupported_envelope_schema"));
        }
        let key = keys
            .keys
            .iter()
            .find(|k| k.key_id == envelope.key_id)
            .ok_or_else(|| invalid("unknown_signing_key"))?;
        if key.revoked {
            return Err(invalid("signing_key_revoked"));
        }
        let public: [u8; 32] = STANDARD
            .decode(&key.public_key)
            .map_err(|_| invalid("invalid_public_key"))?
            .try_into()
            .map_err(|_| invalid("invalid_public_key"))?;
        let verifier =
            VerifyingKey::from_bytes(&public).map_err(|_| invalid("invalid_public_key"))?;
        let signature_bytes = STANDARD
            .decode(&envelope.signature)
            .map_err(|_| invalid("invalid_signature"))?;
        let signature =
            Signature::from_slice(&signature_bytes).map_err(|_| invalid("invalid_signature"))?;
        let payload_bytes = STANDARD
            .decode(&envelope.payload)
            .map_err(|_| invalid("invalid_payload"))?;
        if payload_bytes.len() as u64 > MAX_CATALOG_BYTES {
            return Err(invalid("catalog_too_large"));
        }
        verifier
            .verify_strict(&payload_bytes, &signature)
            .map_err(|_| invalid("signature_mismatch"))?;
        let payload: CatalogPayload = serde_json::from_slice(&payload_bytes)
            .map_err(|_| invalid("invalid_catalog_payload"))?;
        payload.catalog.validate()?;
        if payload.generated_at > timestamp.saturating_add(300)
            || payload.expires_at <= payload.generated_at
        {
            return Err(invalid("catalog_time_invalid"));
        }
        if timestamp >= payload.expires_at {
            return Err(invalid("catalog_expired"));
        }
        if payload.catalog.revision < highest_revision {
            return Err(invalid("catalog_revision_rollback"));
        }
        let mut revoked = HashSet::new();
        if payload
            .revoked_release_ids
            .iter()
            .any(|id| !revoked.insert(id))
        {
            return Err(invalid("duplicate_revocation"));
        }
        Ok(Self {
            payload,
            key_id: envelope.key_id,
        })
    }

    pub fn load(
        path: &Path,
        keys: &TrustKeys,
        timestamp: u64,
        highest_revision: u64,
    ) -> io::Result<Self> {
        Self::from_bytes(
            &read_bounded(path, MAX_CATALOG_BYTES * 2)?,
            keys,
            timestamp,
            highest_revision,
        )
    }

    pub fn catalog(&self) -> &Catalog {
        &self.payload.catalog
    }
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
    pub fn expires_at(&self) -> u64 {
        self.payload.expires_at
    }
    pub fn is_revoked(&self, release_id: &str) -> bool {
        self.payload
            .revoked_release_ids
            .iter()
            .any(|id| id == release_id)
    }
    pub fn ensure_active(&self, release_id: &str, timestamp: u64) -> io::Result<()> {
        if timestamp >= self.payload.expires_at {
            return Err(invalid("catalog_expired"));
        }
        if self.is_revoked(release_id) {
            return Err(invalid("release_revoked"));
        }
        if !self
            .payload
            .catalog
            .releases
            .iter()
            .any(|r| r.release_id == release_id)
        {
            return Err(invalid("release_not_found"));
        }
        Ok(())
    }
}

use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signer, SigningKey};
use steam_patch_core::{catalog::Catalog, trust::*};

fn fixture(revision: u64) -> (Vec<u8>, TrustKeys) {
    let key = SigningKey::from_bytes(&[7; 32]);
    let payload = serde_json::to_vec(&CatalogPayload {
        catalog: Catalog {
            schema_version: 1,
            revision,
            releases: vec![],
        },
        generated_at: 1000,
        expires_at: 2000,
        revoked_release_ids: vec![],
    })
    .unwrap();
    let envelope = SignedEnvelope {
        schema_version: 1,
        key_id: "test".into(),
        signature: STANDARD.encode(key.sign(&payload).to_bytes()),
        payload: STANDARD.encode(payload),
    };
    let keys = TrustKeys {
        schema_version: 1,
        keys: vec![TrustKey {
            key_id: "test".into(),
            public_key: STANDARD.encode(key.verifying_key().to_bytes()),
            revoked: false,
        }],
    };
    (serde_json::to_vec(&envelope).unwrap(), keys)
}

#[test]
fn signature_time_revision_and_revoked_key_are_enforced() {
    let (bytes, mut keys) = fixture(3);
    assert!(VerifiedCatalog::from_bytes(&bytes, &keys, 1500, 3).is_ok());
    assert!(VerifiedCatalog::from_bytes(&bytes, &keys, 2001, 3).is_err());
    assert!(VerifiedCatalog::from_bytes(&bytes, &keys, 500, 3).is_err());
    assert!(VerifiedCatalog::from_bytes(&bytes, &keys, 1500, 4).is_err());
    keys.keys[0].revoked = true;
    assert!(VerifiedCatalog::from_bytes(&bytes, &keys, 1500, 0).is_err());
}

#[test]
fn signed_payload_tampering_and_unknown_keys_fail_closed() {
    let (bytes, keys) = fixture(3);
    let mut envelope: SignedEnvelope = serde_json::from_slice(&bytes).unwrap();
    envelope.payload = STANDARD.encode(b"{}");
    assert!(
        VerifiedCatalog::from_bytes(&serde_json::to_vec(&envelope).unwrap(), &keys, 1500, 0)
            .is_err()
    );
    let (_, mut keys) = fixture(3);
    keys.keys[0].key_id = "other".into();
    assert!(VerifiedCatalog::from_bytes(&bytes, &keys, 1500, 0).is_err());
}

#[test]
fn signed_release_revocation_blocks_execution() {
    let (bytes, keys) = fixture(3);
    let mut envelope: SignedEnvelope = serde_json::from_slice(&bytes).unwrap();
    let mut payload: CatalogPayload =
        serde_json::from_slice(&STANDARD.decode(&envelope.payload).unwrap()).unwrap();
    payload.revoked_release_ids = vec!["withdrawn-release".into()];
    let payload = serde_json::to_vec(&payload).unwrap();
    envelope.signature =
        STANDARD.encode(SigningKey::from_bytes(&[7; 32]).sign(&payload).to_bytes());
    envelope.payload = STANDARD.encode(payload);
    let catalog =
        VerifiedCatalog::from_bytes(&serde_json::to_vec(&envelope).unwrap(), &keys, 1500, 0)
            .unwrap();
    assert!(catalog.is_revoked("withdrawn-release"));
    assert_eq!(
        catalog
            .ensure_active("withdrawn-release", 1500)
            .unwrap_err()
            .to_string(),
        "release_revoked"
    );
}

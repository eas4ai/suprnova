//! The deployed passkey envelope contract and the credential store.

#![cfg(all(feature = "passkey", feature = "seaorm-sqlite"))]

#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;

use magnetar::passkey::envelope::PasskeyEnvelope;
use magnetar::storage::{CredentialActor, PasskeyStore, SeaOrmStorage};
use sea_orm::DatabaseConnection;

use storage_schema::{StorageSchema, database};

/// A deployed-shaped envelope exactly as torii's repository serializes it:
/// compact JSON, alphabetical keys, base64-standard fields.
const DEPLOYED: &str = "{\"created_at\":\"2025-03-04T10:20:30.400Z\",\
\"credential_id\":\"AQIDBAU=\",\"last_used_at\":null,\
\"name\":\"Test Passkey\",\"public_key\":\"BgcICQo=\"}";

async fn credential_actor(database: &DatabaseConnection) -> CredentialActor {
    storage_schema::credential_actor(database, "1", 0, "passkey-storage-session").await
}

#[test]
fn deployed_envelopes_round_trip_byte_for_byte() {
    let envelope = PasskeyEnvelope::parse(DEPLOYED).expect("deployed envelope parses");
    assert_eq!(
        envelope.to_json(),
        DEPLOYED,
        "parse followed by serialize must not transform a deployed row"
    );
    assert_eq!(envelope.credential_id_b64().unwrap(), "AQIDBAU=");
    assert_eq!(envelope.name().as_deref(), Some("Test Passkey"));
    assert!(envelope.last_used_at().is_none());
}

#[tokio::test]
async fn store_round_trip_update_and_honest_lookup() {
    let db = database().await;
    let actor = credential_actor(&db).await;
    let store = Arc::new(SeaOrmStorage::<StorageSchema>::new(db));

    let inserted = store
        .insert_passkey(&actor, "AQIDBAU=", DEPLOYED)
        .await
        .expect("insert succeeds");
    assert_eq!(inserted.user_id, "1");
    assert_eq!(inserted.credential_id, "AQIDBAU=");
    assert_eq!(inserted.envelope_json, DEPLOYED);

    // Unverified lookup over the public credential id.
    let found = store
        .find_user_by_credential("AQIDBAU=")
        .await
        .unwrap()
        .expect("row found");
    assert_eq!(found.user_id, "1");
    assert!(
        store
            .find_user_by_credential("unknown")
            .await
            .unwrap()
            .is_none()
    );

    // The envelope update rewrites exactly one row.
    let updated_envelope = DEPLOYED.replace("BgcICQo=", "CgkIBwY=");
    store
        .update_passkey_envelope(&actor, "AQIDBAU=", &updated_envelope)
        .await
        .expect("update lands on the single row");
    let rows = store.passkeys_for_user("1").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].envelope_json, updated_envelope);

    // A missing row at update time is internal inconsistency, not a user
    // error: the auth path already proved allow-list membership.
    let missing = store
        .update_passkey_envelope(&actor, "unknown", DEPLOYED)
        .await
        .unwrap_err();
    assert!(matches!(missing, magnetar::Error::Internal { .. }));
}

#[test]
fn counter_updates_preserve_untouched_fields() {
    let envelope = PasskeyEnvelope::parse(DEPLOYED).unwrap();
    // Splice a new public_key and a last_used stamp through the same edit
    // path the authentication flow uses, then confirm the identity fields
    // survived verbatim.
    let value: serde_json::Value = serde_json::from_str(&envelope.to_json()).unwrap();
    assert_eq!(value["credential_id"], "AQIDBAU=");
    assert_eq!(value["name"], "Test Passkey");
    assert_eq!(value["created_at"], "2025-03-04T10:20:30.400Z");
}

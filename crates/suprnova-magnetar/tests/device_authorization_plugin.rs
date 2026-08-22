//! Mounted first-party device-authorization route contract.

#![cfg(all(
    feature = "oauth",
    feature = "device-authorization",
    feature = "seaorm-sqlite"
))]

#[path = "fixtures/grants_harness.rs"]
mod grants_harness;
#[path = "fixtures/oauth_harness.rs"]
mod oauth_harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;

use async_trait::async_trait;
use magnetar::Result;
use magnetar::oauth::device::{DeviceAuthorizationConfig, DeviceAuthorizationService};
use magnetar::plugin::{
    Encryptor, Method, PluginContext, PluginError, PluginRegistry, WireBody, WireRequest,
};
use magnetar::plugins::device_authorization::{
    DeviceAuthorizationPlugin, DeviceAuthorizationPluginConfig,
};
use serde_json::{Value, json};

struct IdentityEncryptor;

#[async_trait]
impl Encryptor for IdentityEncryptor {
    async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        Ok(plaintext.to_vec())
    }

    async fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        Ok(ciphertext.to_vec())
    }
}

fn post(path: &str, body: Value) -> WireRequest {
    let mut request = WireRequest::new(Method::Post, path);
    request.body = WireBody::Json(body);
    request
}

#[tokio::test]
async fn device_authorization_plugin_mounts_the_complete_first_party_route_group() {
    let h = grants_harness::harness().await;
    let service = Arc::new(DeviceAuthorizationService::new(
        h.storage(),
        h.storage(),
        h.storage(),
        h.gate.clone(),
        h.sessions.clone(),
        h.oauth.limiter.clone(),
        h.oauth.encryptor.clone(),
        DeviceAuthorizationConfig::default(),
    ));
    let context: PluginContext<storage_schema::StorageSchema> = PluginContext::new(
        h.storage(),
        h.sessions.clone(),
        h.gate.clone(),
        Arc::new(IdentityEncryptor),
        h.oauth.limiter.clone(),
        h.oauth.mail.clone(),
        h.http.clone(),
        h.oauth.links.clone(),
    );
    let registry = PluginRegistry::new(context)
        .register(DeviceAuthorizationPlugin::new(
            service,
            DeviceAuthorizationPluginConfig::default(),
        ))
        .build()
        .await
        .expect("first-party device plugin mounts");

    for (path, body) in [
        ("/oauth/device/code", json!({})),
        ("/oauth/device/verify", json!({"user_code": "ABCD-EFGH"})),
        ("/oauth/device/approve", json!({"user_code": "ABCD-EFGH"})),
        (
            "/oauth/device/approve/challenge",
            json!({"challenge_selector": "challenge", "code": "123456"}),
        ),
        ("/oauth/device/deny", json!({"user_code": "ABCD-EFGH"})),
        ("/oauth/device/poll", json!({"device_code": "device-code"})),
    ] {
        let result = registry.handle(post(path, body)).await;
        assert!(
            !matches!(result, Err(PluginError::RouteNotFound { .. })),
            "{path} must be mounted by the first-party device plugin"
        );
    }
    assert!(
        h.oauth
            .limiter
            .acquired
            .lock()
            .iter()
            .any(|(key, _)| key.starts_with("device-code.issue:")),
        "device code issuance must acquire its named row-creation budget",
    );
}

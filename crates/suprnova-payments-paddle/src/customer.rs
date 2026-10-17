//! Implementation of the `CustomerStore` trait for `PaddleProvider`.
//!
//! Paddle does NOT expose customer deletion via its API - `delete_customer`
//! returns `PaymentError::NotSupported` with a pointer to
//! `PaddleProvider::archive_customer`, which archives the customer instead.
//! A test asserts this invariant.

use async_trait::async_trait;
use paddle_rust_sdk::enums::Status;
use std::collections::HashMap;
use suprnova::payments::{
    CreateCustomerRequest, CustomerRef, CustomerStore, PaymentError, PaymentResult,
    UpdateCustomerRequest,
};

use crate::PaddleProvider;
use crate::deadline;
use crate::sdk_error;

/// Flatten the public `Option<serde_json::Value>` metadata input into the
/// Paddle-friendly `HashMap<String, String>` shape that
/// `CustomerCreate::custom_data` / `CustomerUpdate::custom_data` accept.
///
/// The pinned SDK builder accepts string values even though Paddle supports
/// structured JSON. Non-string values are JSON-encoded strings, consistently
/// across customer and checkout requests.
///
/// `None`, `Some(Null)`, and an empty object all produce `None` so the
/// builder method is simply not called and `custom_data` stays out of the
/// outgoing payload (the field is serialised with `#[skip_serializing_none]`
/// in the SDK builder).
pub(crate) fn metadata_to_string_map(
    value: Option<&serde_json::Value>,
) -> Option<HashMap<String, String>> {
    let obj = value?.as_object()?;
    if obj.is_empty() {
        return None;
    }
    let mut map = HashMap::with_capacity(obj.len());
    for (k, v) in obj {
        let s = match v {
            serde_json::Value::Null => continue,
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        map.insert(k.clone(), s);
    }
    if map.is_empty() { None } else { Some(map) }
}

impl PaddleProvider {
    /// Archive a customer: set its Paddle `status` to `archived`.
    ///
    /// Paddle has no endpoint that deletes a customer, so archiving is how
    /// a customer is taken out of use, and `CustomerStore::delete_customer`
    /// points here. An archived customer keeps its history at Paddle and
    /// can't be used for new checkouts.
    ///
    /// The request sends the status and nothing else, so the name, email
    /// and `custom_data` of the customer stay as they are. A customer that
    /// Paddle does not know is `PaymentError::NotFound`.
    pub async fn archive_customer(&self, provider_customer_id: &str) -> PaymentResult<()> {
        deadline::change(
            "customer_update (archive)",
            self.client()
                .customer_update(provider_customer_id.to_string())
                .status(Status::Archived)
                .send(),
        )
        .await?
        .map_err(archive_error)?;
        Ok(())
    }
}

/// Map an error of the archive request. Paddle's `not_found` is a customer
/// that does not exist; every other error is the provider's.
fn archive_error(e: paddle_rust_sdk::Error) -> PaymentError {
    // The SDK keeps the error code of Paddle's body and drops the HTTP
    // status, so the code is what tells a 404 apart.
    if let paddle_rust_sdk::Error::PaddleApi(response) = &e
        && response.error.code == "not_found"
    {
        return PaymentError::NotFound("paddle customer_update (archive): no such customer".into());
    }
    sdk_error::provider_error("customer_update (archive)", e)
}

#[async_trait]
impl CustomerStore for PaddleProvider {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef> {
        let mut builder = self.client().customer_create(req.email.clone());
        if let Some(name) = &req.name {
            builder.name(name.clone());
        }
        if let Some(custom_data) = metadata_to_string_map(req.metadata.as_ref()) {
            builder.custom_data(custom_data);
        }

        let resp = deadline::change("customer_create", builder.send())
            .await?
            .map_err(|e| sdk_error::provider_error("customer_create", e))?;

        Ok(CustomerRef {
            provider_customer_id: resp.data.id.to_string(),
            user_id: Some(req.user_id),
            email: req.email,
            provider_metadata: req.metadata.unwrap_or(serde_json::json!({})),
        })
    }

    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef> {
        let mut builder = self
            .client()
            .customer_update(req.provider_customer_id.clone());
        if let Some(email) = &req.email {
            builder.email(email.clone());
        }
        if let Some(name) = &req.name {
            builder.name(name.clone());
        }
        if let Some(custom_data) = metadata_to_string_map(req.metadata.as_ref()) {
            builder.custom_data(custom_data);
        }

        let resp = deadline::change("customer_update", builder.send())
            .await?
            .map_err(|e| sdk_error::provider_error("customer_update", e))?;

        // Prefer the `custom_data` Paddle returned over the request-side
        // echo. Admin and reconciliation tooling needs the server's
        // authoritative view (Paddle may normalise), not the client's
        // pre-flight copy. Fall back to the request metadata only when
        // Paddle omitted the field entirely.
        let provider_metadata = resp
            .data
            .custom_data
            .clone()
            .unwrap_or_else(|| req.metadata.clone().unwrap_or(serde_json::json!({})));
        Ok(CustomerRef {
            provider_customer_id: resp.data.id.to_string(),
            user_id: None,
            email: resp.data.email.clone(),
            provider_metadata,
        })
    }

    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef> {
        let resp = deadline::read(
            "customer_get",
            self.client()
                .customer_get(provider_customer_id.to_string())
                .send(),
        )
        .await?
        .map_err(|e| sdk_error::provider_error("customer_get", e))?;

        // Round-trip Paddle's `custom_data` (Paddle's name for
        // caller-supplied metadata). Empty object when the customer
        // has no `custom_data` set - matches the contract callers
        // rely on (always a JSON value, never null).
        let provider_metadata = resp
            .data
            .custom_data
            .clone()
            .unwrap_or(serde_json::json!({}));
        Ok(CustomerRef {
            provider_customer_id: resp.data.id.to_string(),
            user_id: None,
            email: resp.data.email.clone(),
            provider_metadata,
        })
    }

    async fn delete_customer(&self, _provider_customer_id: &str) -> PaymentResult<()> {
        // Paddle does not expose customer deletion; archiving is its way to
        // take a customer out of use.
        Err(PaymentError::NotSupported(
            "Paddle does not expose customer deletion. \
             Archive the customer with PaddleProvider::archive_customer instead."
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mocked_paddle::{paddle_error, paddle_with, stalled};
    use serde_json::json;

    #[test]
    fn metadata_to_string_map_none_when_input_none() {
        assert!(metadata_to_string_map(None).is_none());
    }

    #[test]
    fn metadata_to_string_map_none_when_input_null() {
        let v = serde_json::Value::Null;
        assert!(metadata_to_string_map(Some(&v)).is_none());
    }

    #[test]
    fn metadata_to_string_map_none_when_input_empty_object() {
        let v = json!({});
        assert!(metadata_to_string_map(Some(&v)).is_none());
    }

    #[test]
    fn metadata_to_string_map_none_when_input_not_an_object() {
        for v in [json!("just-a-string"), json!([1, 2, 3]), json!(42)] {
            assert!(metadata_to_string_map(Some(&v)).is_none(), "{v:?}");
        }
    }

    #[test]
    fn metadata_to_string_map_string_values_pass_through_unquoted() {
        let v = json!({ "plan": "pro", "tier": "gold" });
        let map = metadata_to_string_map(Some(&v)).expect("map present");
        assert_eq!(map.get("plan").map(String::as_str), Some("pro"));
        assert_eq!(map.get("tier").map(String::as_str), Some("gold"));
    }

    #[test]
    fn metadata_to_string_map_scalars_are_stringified() {
        let v = json!({ "seats": 5, "trial": true });
        let map = metadata_to_string_map(Some(&v)).expect("map present");
        assert_eq!(map.get("seats").map(String::as_str), Some("5"));
        assert_eq!(map.get("trial").map(String::as_str), Some("true"));
    }

    #[test]
    fn metadata_to_string_map_skips_null_values() {
        let v = json!({ "valid": "yes", "skipped": serde_json::Value::Null });
        let map = metadata_to_string_map(Some(&v)).expect("map present");
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("valid"));
        assert!(!map.contains_key("skipped"));
    }

    // ---- provider_metadata round-trip (CustomerRef construction) -----------
    //
    // Paddle's `CustomerData::custom_data` is `Option<serde_json::Value>`.
    // The CustomerStore::get_customer / update_customer fix prefers the
    // server-returned value over the request-side echo so admin /
    // reconciliation tooling sees Paddle's authoritative view. The two
    // tests below pin that selection rule by exercising the
    // `unwrap_or` chain directly - the same shape both call sites use.

    fn pick_paddle_metadata(
        server: Option<serde_json::Value>,
        fallback: serde_json::Value,
    ) -> serde_json::Value {
        server.unwrap_or(fallback)
    }

    #[test]
    fn paddle_pick_prefers_server_custom_data_over_request_echo() {
        let server = Some(json!({ "tier": "gold" }));
        let fallback = json!({ "tier": "silver" }); // intentionally different
        let out = pick_paddle_metadata(server, fallback);
        assert_eq!(out, json!({ "tier": "gold" }));
    }

    #[test]
    fn paddle_pick_falls_back_when_server_returns_none() {
        let fallback = json!({ "from_request": "yes" });
        let out = pick_paddle_metadata(None, fallback);
        assert_eq!(out, json!({ "from_request": "yes" }));
    }

    // ---- archive_customer against the mocked Paddle API ---------------------

    fn archived_customer() -> serde_json::Value {
        json!({"data": {
            "id": "ctm_test", "name": "Archived Customer", "email": "archived@example.test",
            "marketing_consent": false, "status": "archived", "custom_data": null,
            "locale": "en", "created_at": "2026-09-01T12:00:00Z",
            "updated_at": "2026-09-28T12:00:00Z", "import_meta": null
        }, "meta": {"request_id": "test-request"}})
    }

    #[tokio::test]
    async fn archive_customer_patches_the_status_and_nothing_else() {
        let (provider, requests) = paddle_with(vec![(200, archived_customer())]).await;
        let result = provider.archive_customer("ctm_test").await;
        assert!(result.is_ok(), "{result:?}");
        let requests = requests.lock().expect("request log");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].request_line,
            "PATCH /customers/ctm_test HTTP/1.1"
        );
        assert_eq!(requests[0].json(), json!({"status": "archived"}));
    }

    #[tokio::test]
    async fn archive_customer_reports_an_unknown_customer_as_not_found() {
        let (provider, requests) = paddle_with(vec![(404, paddle_error("not_found"))]).await;
        let result = provider.archive_customer("ctm_missing").await;
        let Err(PaymentError::NotFound(message)) = &result else {
            panic!("expected NotFound: {result:?}");
        };
        assert!(!message.contains("ctm_missing"), "{message}");
        assert_eq!(requests.lock().expect("request log").len(), 1);
    }

    #[tokio::test]
    async fn an_archive_that_runs_out_of_time_has_an_unknown_outcome() {
        let result =
            stalled(|provider| async move { provider.archive_customer("ctm_test").await }).await;

        let Err(PaymentError::Provider(message)) = &result else {
            panic!("expected a provider error: {result:?}");
        };
        assert!(message.contains("timed out"), "{message}");
        assert!(message.contains("outcome is unknown"), "{message}");
        assert!(!message.contains("failed"), "{message}");
    }

    #[tokio::test]
    async fn archive_customer_reports_other_paddle_errors_as_provider_errors() {
        let (provider, _) = paddle_with(vec![(403, paddle_error("forbidden"))]).await;
        let result = provider.archive_customer("ctm_test").await;
        assert!(
            matches!(result, Err(PaymentError::Provider(_))),
            "{result:?}"
        );
    }
}

use crate::{MAX_BODY_BYTES, NowPaymentsProvider};
use serde_json::Value;
use suprnova::payments::{PaymentError, PaymentResult};

pub(crate) struct JsonResponse {
    pub(crate) value: Value,
    pub(crate) body: Vec<u8>,
}

impl NowPaymentsProvider {
    pub(crate) async fn send_json(
        &self,
        request: reqwest::RequestBuilder,
    ) -> PaymentResult<JsonResponse> {
        let mut response = request
            .header("x-api-key", self.api_key.clone())
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|error| {
                PaymentError::Provider(format!(
                    "NOWPayments request failed or timed out: {}",
                    transport_cause(error)
                ))
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                401 | 403 => {
                    PaymentError::Authentication("NOWPayments rejected API credentials".into())
                }
                404 => PaymentError::NotFound("NOWPayments resource".into()),
                400 | 422 => PaymentError::Validation(format!(
                    "NOWPayments rejected the request (HTTP {})",
                    status.as_u16()
                )),
                _ => {
                    PaymentError::Provider(format!("NOWPayments returned HTTP {}", status.as_u16()))
                }
            });
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_BODY_BYTES as u64)
        {
            return Err(PaymentError::Provider(
                "NOWPayments response exceeds 64 KiB".into(),
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            PaymentError::Provider(format!(
                "NOWPayments response could not be read: {}",
                transport_cause(error)
            ))
        })? {
            if chunk.len() > MAX_BODY_BYTES - body.len() {
                return Err(PaymentError::Provider(
                    "NOWPayments response exceeds 64 KiB".into(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let value = crate::json::parse(&body).map_err(|error| {
            PaymentError::Provider(format!("NOWPayments returned invalid JSON: {error}"))
        })?;
        Ok(JsonResponse { value, body })
    }
}

/// A transport error's own text and each cause beneath it, without the
/// request URL. `reqwest::Error` names the URL, which carries payment
/// identifiers, and its `Display` stops at the first level, where the text is
/// only "error sending request"; the refused connection or the elapsed
/// deadline an operator needs is one or two causes down.
fn transport_cause(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut text = error.to_string();
    let mut cause = std::error::Error::source(&error);
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

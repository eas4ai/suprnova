use std::sync::Arc;
use std::time::Duration;

use paddle_rust_sdk::Paddle;
use suprnova::payments::{
    Checkout, PaymentError, SessionMode, StartSessionRequest, Subscription,
    UpdateSubscriptionRequest,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{PaddleEnvironment, PaddleProvider};

async fn provider_with_capture() -> (PaddleProvider, tokio::task::JoinHandle<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind request-capture listener");
    let address = listener.local_addr().expect("capture listener address");
    let capture = tokio::spawn(async move {
        let accepted = tokio::time::timeout(Duration::from_millis(250), listener.accept()).await;
        let Ok(Ok((mut stream, _))) = accepted else {
            return false;
        };

        let mut bytes = [0_u8; 4096];
        let _ = stream.read(&mut bytes).await.expect("read Paddle request");
        let response_body = r#"{"error":{"detail":"forced response"}}"#;
        let response = format!(
            "HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response_body}",
            response_body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write Paddle response");
        true
    });

    let client = Paddle::new("pdl_sdbx_apikey_test", format!("http://{address}"))
        .expect("valid capture-server URL");
    let provider = PaddleProvider {
        client: Arc::new(client),
        webhook_key: "pdl_ntfset_test".into(),
        client_token: "test_client_token".into(),
        environment: PaddleEnvironment::Sandbox,
    };
    (provider, capture)
}

#[tokio::test]
async fn unsupported_idempotency_keys_fail_before_paddle_mutations() {
    let (provider, capture) = provider_with_capture().await;
    let checkout = provider
        .start_session(StartSessionRequest {
            mode: SessionMode::Subscription,
            customer_ref: "ctm_test".into(),
            price_refs: vec!["pri_test".into()],
            success_return_url: "https://example.test/success".into(),
            cancel_return_url: "https://example.test/cancel".into(),
            amount_hint: None,
            idempotency_key: Some("checkout-key".into()),
            metadata: None,
        })
        .await;
    let checkout_reached_endpoint = capture.await.expect("checkout capture task");
    assert!(
        matches!(checkout, Err(PaymentError::NotSupported(_))),
        "Paddle checkout must reject a key it cannot forward: {checkout:?}"
    );
    assert!(
        !checkout_reached_endpoint,
        "keyed Paddle checkout reached the provider endpoint"
    );

    let (provider, capture) = provider_with_capture().await;
    let update = provider
        .update(UpdateSubscriptionRequest {
            provider_subscription_id: "sub_test".into(),
            new_price_refs: None,
            cancel_at_period_end: Some(true),
            idempotency_key: Some("update-key".into()),
        })
        .await;
    let update_reached_endpoint = capture.await.expect("update capture task");
    assert!(
        matches!(update, Err(PaymentError::NotSupported(_))),
        "Paddle update must reject a key it cannot forward: {update:?}"
    );
    assert!(
        !update_reached_endpoint,
        "keyed Paddle update reached the provider endpoint"
    );
}

use std::sync::Arc;
use std::time::Duration;

use paddle_rust_sdk::Paddle;
use suprnova::payments::{
    Checkout, CustomerStore, PaymentError, SessionMode, StartSessionRequest, Subscription,
    UpdateCustomerRequest, UpdateSubscriptionRequest,
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
            proration: None,
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

#[tokio::test]
async fn provider_ids_are_single_path_segments_before_network() {
    use crate::mocked_paddle::{paddle_error, paddle_with};

    let (provider, requests) = paddle_with(vec![(400, paddle_error("bad_request"))]).await;
    for id in [
        "sub_target/cancel?x=",
        "id/other",
        "id?query=value",
        "id#fragment",
        ".",
        "..",
        "id/../other",
        "id%2fother",
        "%2e%2e",
        "id%252fother",
        "id\\other",
        "id value",
        " id",
        "id\tvalue",
        "id\nvalue",
        "id\0value",
        "id\u{7f}value",
        "id\u{a0}value",
        "",
    ] {
        for operation in [
            "cancel_later",
            "cancel_now",
            "get",
            "archive_customer",
            "get_customer",
            "update_customer",
            "update",
            "schedule_cancellation",
            "change_prices",
        ] {
            let result = match operation {
                "cancel_later" | "cancel_now" => provider
                    .cancel(id, operation == "cancel_later")
                    .await
                    .map(|_| ()),
                "get" => provider.get(id).await.map(|_| ()),
                "archive_customer" => provider.archive_customer(id).await,
                "get_customer" => provider.get_customer(id).await.map(|_| ()),
                "update_customer" => provider
                    .update_customer(UpdateCustomerRequest {
                        provider_customer_id: id.into(),
                        email: None,
                        name: None,
                        metadata: None,
                    })
                    .await
                    .map(|_| ()),
                "update" | "schedule_cancellation" | "change_prices" => provider
                    .update(UpdateSubscriptionRequest {
                        provider_subscription_id: id.into(),
                        new_price_refs: (operation == "change_prices")
                            .then(|| vec!["pri_test".into()]),
                        proration: None,
                        cancel_at_period_end: (operation == "schedule_cancellation")
                            .then_some(true),
                        idempotency_key: None,
                    })
                    .await
                    .map(|_| ()),
                _ => unreachable!(),
            };
            assert!(
                matches!(result, Err(PaymentError::Validation(_))),
                "operation {operation} must refuse {id:?} before sending: {result:?}"
            );
        }
    }
    assert!(requests.lock().expect("request log").is_empty());
}

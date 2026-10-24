use crate::NowPaymentsProvider;
use async_trait::async_trait;
use suprnova::payments::{
    CreateCustomerRequest, CustomerRef, CustomerStore, PaymentError, PaymentResult,
    SubscribeRequest, Subscription, SubscriptionResult, UpdateCustomerRequest,
    UpdateSubscriptionRequest,
};

fn unsupported(operation: &str) -> PaymentError {
    PaymentError::NotSupported(format!(
        "NOWPayments invoice adapter does not implement {operation}"
    ))
}

#[async_trait]
impl CustomerStore for NowPaymentsProvider {
    async fn create_customer(&self, _: CreateCustomerRequest) -> PaymentResult<CustomerRef> {
        Err(unsupported("customer creation"))
    }
    async fn update_customer(&self, _: UpdateCustomerRequest) -> PaymentResult<CustomerRef> {
        Err(unsupported("customer updates"))
    }
    async fn get_customer(&self, _: &str) -> PaymentResult<CustomerRef> {
        Err(unsupported("customer retrieval"))
    }
    async fn delete_customer(&self, _: &str) -> PaymentResult<()> {
        Err(unsupported("customer deletion"))
    }
}

#[async_trait]
impl Subscription for NowPaymentsProvider {
    async fn subscribe(&self, _: SubscribeRequest) -> PaymentResult<SubscriptionResult> {
        Err(unsupported("subscription creation"))
    }
    async fn update(&self, _: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult> {
        Err(unsupported("subscription updates"))
    }
    async fn cancel(&self, _: &str, _: bool) -> PaymentResult<SubscriptionResult> {
        Err(unsupported("subscription cancellation"))
    }
    async fn get(&self, _: &str) -> PaymentResult<SubscriptionResult> {
        Err(unsupported("subscription retrieval"))
    }
}

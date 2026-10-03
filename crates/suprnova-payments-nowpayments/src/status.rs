use crate::NowPaymentsProvider;
use serde::Deserialize;
use serde_json::Value;
use suprnova::payments::{Currency, Money, NeutralEventKind, PaymentError, PaymentResult};

/// Provider payment state. Only `Finished` represents completed settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NowPaymentsStatus {
    /// No payment has arrived.
    Waiting,
    /// Blockchain confirmations are in progress.
    Confirming,
    /// The deposit is confirmed but settlement has not finished.
    Confirmed,
    /// Settlement is being sent to the merchant.
    Sending,
    /// The provider reports completed settlement.
    Finished,
    /// The received amount is partial; no automatic success is emitted.
    PartiallyPaid,
    /// The payment failed.
    Failed,
    /// The payment expired.
    Expired,
    /// The payment was refunded.
    Refunded,
    /// The payment was cancelled.
    Cancelled,
    /// A future or unrecognized provider status; never treated as success.
    Unknown,
}

impl NowPaymentsStatus {
    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "waiting" => Self::Waiting,
            "confirming" => Self::Confirming,
            "confirmed" => Self::Confirmed,
            "sending" => Self::Sending,
            "finished" => Self::Finished,
            "partially_paid" => Self::PartiallyPaid,
            "failed" => Self::Failed,
            "expired" => Self::Expired,
            "refunded" => Self::Refunded,
            "cancelled" | "canceled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }

    /// Neutral framework classification. Pending, partial and unknown states
    /// remain provider-specific events instead of triggering fulfillment.
    pub fn neutral_event(self) -> Option<NeutralEventKind> {
        match self {
            Self::Finished => Some(NeutralEventKind::PaymentSucceeded),
            Self::Failed | Self::Expired | Self::Cancelled => Some(NeutralEventKind::PaymentFailed),
            Self::Refunded => Some(NeutralEventKind::PaymentRefunded),
            _ => None,
        }
    }
}

/// Authenticated payment state, with references for application reconciliation.
///
/// `price` is the provider's invoice price in fiat, not the received crypto
/// quantity. Check it, `invoice_id`, and `order_id` against the persisted order.
/// Merchant fulfillment must be idempotent by order and payment ID.
#[derive(Debug, Clone, PartialEq)]
pub struct NowPaymentsPayment {
    /// Payment ID used by `/v1/payment/{payment_id}`.
    pub payment_id: String,
    /// Hosted invoice that produced this payment, if present.
    pub invoice_id: Option<String>,
    /// Merchant order reference, if present.
    pub order_id: Option<String>,
    /// Conservative interpretation of `payment_status`.
    pub status: NowPaymentsStatus,
    /// Provider invoice price, converted exactly into fiat minor units.
    pub price: Money,
    /// Original parsed response, including crypto amounts and unknown fields.
    pub raw: Value,
}

impl NowPaymentsProvider {
    /// Read a payment using the API key, validating the returned payment ID.
    ///
    /// Pass the payment ID from a verified notification, not a browser return
    /// parameter or the invoice ID returned by `start_session`. This lookup
    /// performs no state changes and does not automatically grant entitlements.
    pub async fn payment_status(&self, payment_id: &str) -> PaymentResult<NowPaymentsPayment> {
        let id = identifier(Some(&Value::String(payment_id.into())), "payment_id")?;
        let mut url = self.api_url.clone();
        url.path_segments_mut()
            .map_err(|_| PaymentError::Internal("invalid NOWPayments payment endpoint".into()))?
            .pop_if_empty()
            .push("payment")
            .push(&id);
        let response = self.send_json(self.client.get(url)).await?;
        let payment = parse_payment(response.value, &response.body)?;
        if payment.payment_id != id {
            return Err(PaymentError::Provider(
                "NOWPayments returned a different payment_id".into(),
            ));
        }
        Ok(payment)
    }
}

pub(crate) fn parse_payment(raw: Value, body: &[u8]) -> PaymentResult<NowPaymentsPayment> {
    let payment_id = identifier(raw.get("payment_id"), "payment_id")?;
    let invoice_id = match raw.get("invoice_id") {
        None | Some(Value::Null) => None,
        value => Some(identifier(value, "invoice_id")?),
    };
    let order_id = match raw.get("order_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(value))
            if !value.trim().is_empty()
                && value.len() <= 128
                && !value.chars().any(char::is_control) =>
        {
            Some(value.clone())
        }
        _ => {
            return Err(PaymentError::Provider(
                "NOWPayments returned an invalid order_id".into(),
            ));
        }
    };
    let status_name = status_name(&raw)?;
    let status = NowPaymentsStatus::parse(status_name);
    let price = price(&raw, body)?;
    Ok(NowPaymentsPayment {
        payment_id,
        invoice_id,
        order_id,
        status,
        price,
        raw,
    })
}

pub(crate) fn status_name(raw: &Value) -> PaymentResult<&str> {
    raw.get("payment_status")
        .and_then(Value::as_str)
        .filter(|s| {
            !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        })
        .ok_or_else(|| {
            PaymentError::Provider("NOWPayments payment_status is missing or invalid".into())
        })
}

pub(crate) fn identifier(value: Option<&Value>, field: &str) -> PaymentResult<String> {
    let invalid =
        || PaymentError::Validation(format!("{field} must be a positive decimal identifier"));
    let id = match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(number)) => {
            number.as_u64().map(|n| n.to_string()).ok_or_else(invalid)?
        }
        _ => return Err(invalid()),
    };
    if id.is_empty()
        || id.len() > 64
        || id.starts_with('0')
        || !id.bytes().all(|c| c.is_ascii_digit())
    {
        return Err(invalid());
    }
    Ok(id)
}

fn price(raw: &Value, body: &[u8]) -> PaymentResult<Money> {
    let invalid = || {
        PaymentError::Provider("NOWPayments price must be positive, exact fiat minor units".into())
    };
    let code = raw
        .get("price_currency")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if code.len() != 3 || !code.bytes().all(|c| c.is_ascii_alphabetic()) {
        return Err(invalid());
    }
    let currency = Currency::from_code(&code.to_ascii_uppercase()).ok_or_else(invalid)?;
    let exponent: u32 = currency.exponent().ok_or_else(invalid)?.into();
    #[derive(Deserialize)]
    struct RawPrice<'a> {
        #[serde(borrow)]
        price_amount: &'a serde_json::value::RawValue,
    }
    let lexical = serde_json::from_slice::<RawPrice<'_>>(body).map_err(|_| invalid())?;
    let lexical = lexical.price_amount.get();
    let amount = if lexical.starts_with('"') {
        serde_json::from_str::<String>(lexical).map_err(|_| invalid())?
    } else {
        lexical.to_owned()
    };
    let minor = exact_minor_units(&amount, exponent).ok_or_else(invalid)?;
    if !lexical.starts_with('"') {
        // The IPN MAC authenticates JavaScript's numeric representation.
        // Reject a numeric lexeme whose monetary value would change under
        // that representation instead of trusting unverified extra digits.
        let number = raw
            .get("price_amount")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .ok_or_else(invalid)?;
        let mut buffer = ryu_js::Buffer::new();
        let canonical = buffer.format_finite(number);
        if exact_minor_units(canonical, exponent) != Some(minor) {
            return Err(invalid());
        }
    }
    Ok(Money::from_minor_units(minor, currency))
}

/// Convert the source decimal lexeme with integer arithmetic only. Scientific
/// notation is allowed, but neither mantissas nor sub-minor units are rounded.
fn exact_minor_units(amount: &str, currency_exponent: u32) -> Option<i64> {
    if amount.is_empty() || amount.len() > 64 {
        return None;
    }
    let (mantissa, exponent) = match amount.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
        None => (amount, 0),
    };
    if exponent.unsigned_abs() > 64 {
        return None;
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let coefficient: i128 = digits.parse().ok()?;
    if coefficient <= 0 {
        return None;
    }
    let scale = exponent
        .checked_add(currency_exponent.try_into().ok()?)?
        .checked_sub(fraction.len().try_into().ok()?)?;
    let power = 10i128.checked_pow(scale.unsigned_abs())?;
    let minor = if scale >= 0 {
        coefficient.checked_mul(power)?
    } else {
        if coefficient % power != 0 {
            return None;
        }
        coefficient / power
    };
    i64::try_from(minor).ok().filter(|value| *value > 0)
}

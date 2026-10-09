# Escribir un adaptador de proveedor de pagos

Esta guía recorre la construcción de un crate adaptador de terceros -
`suprnova-payments-mollie` - que se conecta a la superficie de pagos
neutral respecto al proveedor de Suprnova. Al final tendrás un crate
que se registra a sí mismo, pasa el flujo discriminador, y se puede
soltar dentro de cualquier app de Suprnova con un solo `cargo add`.

La misma estructura aplica a cualquier proveedor: Square, Braintree,
Adyen, o cualquier otro con una API HTTP.

### Por qué Suprnova diverge

Laravel distribuye Cashier como una integración de Stripe de primera
parte. Es excelente para el camino de Stripe, pero codifica el
vocabulario de un proveedor dentro del framework - añadir un segundo
proveedor implica bifurcar Cashier o construir una superficie paralela
al lado.

Suprnova mantiene a todo proveedor bajo el mismo contrato de cinco
traits: `Checkout`, `Subscription`, `CustomerStore`, `WebhookHandler`,
y el `Payment` opcional para los proveedores de captura del lado del
servidor. El código de dominio nunca guarda más que un
`Arc<dyn PaymentProvider>` desde el registro. Cambiar Stripe por Paddle
(o por el adaptador de Mollie que estás por escribir) es un cambio de
bootstrap, no un cambio de código. Los adaptadores de referencia en
`crates/suprnova-payments-stripe/` y `crates/suprnova-payments-paddle/`
demuestran que el contrato de traits se sostiene para dos modelos
comerciales muy distintos - pasarela de captura directa y Merchant of
Record - y tu adaptador encaja en la misma forma.

## 1. Crea el crate miembro del workspace

Desde la raíz del repo:

```bash
cargo new --lib crates/suprnova-payments-mollie
```

Añádelo a tu `Cargo.toml` raíz:

```toml
[workspace]
members = [
    "framework",
    "app",
    "suprnova-cli",
    "suprnova-macros",
    "crates/suprnova-payments-mollie",  # añade esta línea
]
```

(Los adaptadores de referencia - `crates/suprnova-payments-stripe` y
`crates/suprnova-payments-paddle` - viven en este mismo directorio
`crates/` y son buenas plantillas para leer junto a esta guía.)

**`crates/suprnova-payments-mollie/Cargo.toml`:**

```toml
[package]
name = "suprnova-payments-mollie"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Mollie payment adapter for Suprnova"

[dependencies]
suprnova = { path = "../../framework" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
inventory = "0.3"
tracing = "0.1"
tokio = { version = "1", features = ["macros", "rt"] }
# Tu SDK de Mollie:
mollie-rs = "0.1"
hmac = "0.12"   # para la verificación HMAC del webhook
sha2 = "0.10"
hex = "0.4"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
```

## 2. Organiza los archivos fuente

Refleja la estructura que usan los adaptadores distribuidos:

```
crates/suprnova-payments-mollie/src/
├── lib.rs          # struct MollieProvider, impl de PaymentProvider, from_env
├── checkout.rs     # impl de Checkout
├── customer.rs     # impl de CustomerStore
├── subscription.rs # impl de Subscription
├── webhook.rs      # impl de WebhookHandler
├── event_map.rs    # cadena de evento del proveedor → NeutralEventKind
└── payment.rs      # impl de Payment (si Mollie soporta captura del lado del servidor)
```

## 3. `lib.rs` - el struct del proveedor

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{Payment, PaymentProvider};

mod checkout;
mod customer;
mod event_map;
mod payment;
mod subscription;
mod webhook;

pub use event_map::mollie_event_to_neutral;

/// Adaptador de Mollie para la superficie de pagos neutral respecto
/// al proveedor de Suprnova.
#[derive(Clone, Debug)]
pub struct MollieProvider {
    /// Clave de API de Mollie (`test_…` / `live_…`).
    api_key: String,
    /// Secreto de firma del webhook - usado en la verificación HMAC.
    webhook_secret: String,
    /// Cliente HTTP - se comparte entre solicitudes.
    client: reqwest::Client,
}

impl MollieProvider {
    pub fn new(api_key: impl Into<String>, webhook_secret: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            webhook_secret: webhook_secret.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Construye la instancia a partir de variables de entorno.
    ///
    /// Lee:
    /// - `MOLLIE_API_KEY`
    /// - `MOLLIE_WEBHOOK_SECRET`
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("MOLLIE_API_KEY")
            .map_err(|_| "MOLLIE_API_KEY not set".to_string())?;
        let webhook_secret = std::env::var("MOLLIE_WEBHOOK_SECRET")
            .map_err(|_| "MOLLIE_WEBHOOK_SECRET not set".to_string())?;
        Ok(Self::new(api_key, webhook_secret))
    }
}

impl PaymentProvider for MollieProvider {
    fn name(&self) -> &'static str {
        "mollie"
    }

    // Anula `as_payment()` solo si también implementas `Payment`
    // (captura del lado del servidor). El impl por defecto de
    // `PaymentProvider` devuelve `None` - omite esta anulación por
    // completo si Mollie es solo-checkout / al estilo MoR.
    fn as_payment(&self) -> Option<&dyn Payment> {
        Some(self)
    }
}
```

`PaymentProvider` es el trait paraguas - la cláusula de supertrait es
`Checkout + Subscription + CustomerStore + WebhookHandler`, así que el
compilador se negará a vincular tu proveedor hasta que los cuatro estén
implementados. El quinto trait, `Payment`, es **opcional** - solo los
proveedores que exponen captura del lado del servidor lo implementan,
y `as_payment()` le reporta el resultado al framework. El
`as_payment()` por defecto devuelve `None`, así que omite la anulación
por completo si tu proveedor no hace captura del lado del servidor.

## 4. Implementa los cuatro traits requeridos

### `checkout.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    Checkout, PaymentError, PaymentResult, SessionMode, SessionPayload, StartSessionRequest,
};

use crate::MollieProvider;

#[async_trait]
impl Checkout for MollieProvider {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload> {
        // Llama a la API de Mollie para crear un pago o un pedido.
        // Mapea la respuesta a una de las variantes de SessionPayload.
        // Mollie usa páginas de checkout alojadas, así que Redirect es
        // el ajuste natural.
        let checkout_url = self.create_mollie_payment(&req).await
            .map_err(|e| PaymentError::Internal(format!("Mollie checkout error: {e}")))?;

        Ok(SessionPayload::Redirect {
            url: checkout_url,
            provider_session_id: "mollie_session_id_here".into(),
        })
    }
}

impl MollieProvider {
    async fn create_mollie_payment(&self, req: &StartSessionRequest) -> Result<String, mollie_rs::Error> {
        // Conecta aquí la llamada al SDK de Mollie.
        // Devuelve la URL de checkout alojada.
        todo!("Mollie payment creation")
    }
}
```

### `customer.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    CreateCustomerRequest, CustomerRef, CustomerStore, PaymentError, PaymentResult,
    UpdateCustomerRequest,
};

use crate::MollieProvider;

#[async_trait]
impl CustomerStore for MollieProvider {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef> {
        // POST a /v2/customers en Mollie
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef> {
        // PATCH /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef> {
        // GET /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()> {
        // DELETE /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

### `subscription.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    PaymentError, PaymentResult, SubscribeRequest, Subscription, SubscriptionResult,
    UpdateSubscriptionRequest,
};

use crate::MollieProvider;

#[async_trait]
impl Subscription for MollieProvider {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult> {
        // POST /v2/customers/{id}/subscriptions
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult> {
        // PATCH /v2/customers/{id}/subscriptions/{sub_id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn cancel(
        &self,
        provider_subscription_id: &str,
        at_period_end: bool,
    ) -> PaymentResult<SubscriptionResult> {
        if at_period_end {
            // Fija la fecha de cancelación al final del periodo
        } else {
            // DELETE /v2/customers/{id}/subscriptions/{sub_id}
        }
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult> {
        // GET /v2/customers/{id}/subscriptions/{sub_id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

Si tu proveedor no soporta un método, devuelve
`PaymentError::NotSupported`:

```rust,ignore
Err(PaymentError::NotSupported(
    "Mollie creates subscriptions via checkout - use start_session instead".into()
))
```

### `payment.rs` - captura del lado del servidor (opcional)

Implementa esto solo si tu proveedor soporta cargos directos del lado
del servidor contra un método de pago guardado. Elimina la anulación
de `as_payment()` en `lib.rs` si te saltas esto.

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    ChargeRequest, ChargeResult, Payment, PaymentError, PaymentResult, PaymentStatus,
    RefundRequest, RefundResult,
};

use crate::MollieProvider;

#[async_trait]
impl Payment for MollieProvider {
    async fn charge(&self, req: ChargeRequest) -> PaymentResult<ChargeResult> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn capture(&self, provider_transaction_id: &str) -> PaymentResult<ChargeResult> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn refund(&self, req: RefundRequest) -> PaymentResult<RefundResult> {
        // POST /v2/payments/{id}/refunds
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn void(&self, provider_transaction_id: &str) -> PaymentResult<()> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn status(&self, provider_transaction_id: &str) -> PaymentResult<PaymentStatus> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

## 5. Mapea los eventos del proveedor a `NeutralEventKind`

**`event_map.rs`:**

```rust,ignore
use suprnova::payments::NeutralEventKind;

/// Mapea una cadena de tipo de evento de webhook de Mollie a la
/// taxonomía neutral del framework.
/// Devuelve `None` para eventos específicos del proveedor que no
/// tienen equivalente neutral.
pub fn mollie_event_to_neutral(event_type: &str) -> Option<NeutralEventKind> {
    match event_type {
        // Pagos de Mollie
        "payment.paid"          => Some(NeutralEventKind::PaymentSucceeded),
        "payment.failed"        => Some(NeutralEventKind::PaymentFailed),
        "payment.expired"       => Some(NeutralEventKind::PaymentFailed),
        "refund.created"        => Some(NeutralEventKind::PaymentRefunded),
        "chargeback.created"    => Some(NeutralEventKind::PaymentDisputed),
        // Suscripciones de Mollie
        "subscription.created"  => Some(NeutralEventKind::SubscriptionCreated),
        "subscription.updated"  => Some(NeutralEventKind::SubscriptionUpdated),
        "subscription.canceled" => Some(NeutralEventKind::SubscriptionCanceled),
        // Pedidos/facturas de Mollie
        "order.paid"            => Some(NeutralEventKind::InvoicePaid),
        // Eventos de cliente
        "customer.created"      => Some(NeutralEventKind::CustomerCreated),
        "customer.updated"      => Some(NeutralEventKind::CustomerUpdated),
        // Específico del proveedor - cae hacia raw_payload
        _                       => None,
    }
}
```

Cubre como mínimo los eventos listados arriba. Para cualquier evento
que no esté en la taxonomía neutral, devuelve `None` - de todos modos
se persiste en `payments_webhook_events` bajo `provider_event_type` +
`raw_payload` para que el código de dominio pueda leerlo.

## 6. Implementa la verificación de firma de webhook

**`webhook.rs`:**

Mollie firma los payloads de webhook usando HMAC-SHA256. Compara
siempre las firmas en tiempo constante para prevenir ataques de
temporización.

```rust,ignore
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use suprnova::payments::{
    NeutralEventKind, PaymentError, PaymentResult, WebhookContext, WebhookEvent, WebhookHandler,
};

use crate::{MollieProvider, event_map::mollie_event_to_neutral};

type HmacSha256 = Hmac<Sha256>;

#[async_trait]
impl WebhookHandler for MollieProvider {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()> {
        // Lee el encabezado de firma que envía Mollie.
        // El nombre exacto del encabezado y el esquema de firma -
        // revisa la documentación de Mollie para tu versión.
        let signature = ctx
            .headers
            .get("X-Mollie-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| PaymentError::WebhookSignature(
                "missing X-Mollie-Signature header".into()
            ))?;

        // Calcula el HMAC-SHA256 esperado sobre el cuerpo en bruto.
        let mut mac = HmacSha256::new_from_slice(self.webhook_secret.as_bytes())
            .map_err(|e| PaymentError::Internal(format!("HMAC init: {e}")))?;
        mac.update(ctx.body);

        // Decodifica la firma recibida, codificada en hex.
        let received = hex::decode(signature)
            .map_err(|_| PaymentError::WebhookSignature("non-hex signature".into()))?;

        // Comparación en tiempo constante.
        mac.verify_slice(&received)
            .map_err(|_| PaymentError::WebhookSignature("signature mismatch".into()))
    }

    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        // Mollie envía JSON - analízalo.
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| PaymentError::Validation(format!("invalid mollie webhook body: {e}")))?;

        let event_id = raw["id"].as_str()
            .ok_or_else(|| PaymentError::Validation("missing event id".into()))?
            .to_string();

        // Mollie usa tipos de recurso en lugar de cadenas de tipo de
        // evento en algunas formas de webhook. Adapta esto a lo que
        // envíe tu versión del SDK.
        let event_type = raw["resource"].as_str()
            .unwrap_or("unknown")
            .to_string();

        let neutral = mollie_event_to_neutral(&event_type);

        Ok(WebhookEvent {
            provider: "mollie".into(),
            provider_event_id: event_id,
            provider_event_type: event_type,
            neutral,
            raw_payload: raw,
        })
    }
}
```

Puntos clave:

- `PaymentError::WebhookSignature(String)` es la única variante para
  cualquier fallo de firma - encabezado ausente, codificación
  malformada, discordancia. La ruta de webhook del framework trata
  todo `WebhookSignature(_)` como un 401.
- Usa `PaymentError::Validation(String)` para cuerpos que no se puedan
  analizar. La ruta de webhook devuelve 400 ante cualquier fallo de
  análisis.
- El handler `webhook_routes` del framework llama a `verify` antes
  de `parse_event`, y luego hidrata dentro de una transacción de BD.
  Los fallos de hidratación devuelven 503 para que el proveedor
  reintente.
- Nunca registres en el log el secreto en bruto ni la firma recibida.

### Hidratación de la tabla de copia local: `extract_payload_ids` + `extract_payment_snapshot` + `extract_customer_snapshot`

Después de que `parse_event` devuelve un `WebhookEvent`, la ruta de
webhook del framework hidrata las tablas de copia local. Tres métodos
opcionales del trait impulsan eso - todos tienen implementaciones por
defecto seguras que no hacen nada, así que un adaptador puede
distribuirse sin ellos y de todos modos pasar por la capa de
auditoría:

```rust,ignore
fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;
fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;
fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
```

`PayloadIds` es el puente entre el evento analizado y la lógica de
copia local del framework. Impleméntalo para que el framework pueda
encontrar la entidad correcta:

```rust,ignore
pub struct PayloadIds {
    pub subscription_id: Option<String>,
    pub customer_id: Option<String>,
    pub transaction_id: Option<String>,
}
```

Para cada valor de `neutral`, completa los IDs que expone el payload
del proveedor. Los eventos de suscripción deben fijar
`subscription_id` para que el framework pueda llamar a
`Subscription::get(id)` y refrescar la copia local desde el estado
canónico. Los eventos de cliente fijan `customer_id`. Los eventos de
pago / factura fijan `transaction_id`, más `subscription_id` cuando es
un cargo recurrente.

`PaymentSnapshot` se construye directamente a partir del payload del
webhook - no hay ningún callback `Payment::get`. Impleméntalo para los
neutrales de pago / factura:

```rust,ignore
pub struct PaymentSnapshot {
    pub provider_transaction_id: String,
    pub provider_customer_id: String,
    pub provider_subscription_id: Option<String>,
    pub amount_total_minor: i64,
    pub amount_tax_minor: i64,
    pub currency: String,
    pub status: String,             // "succeeded" | "failed" | "refunded" | "disputed"
    pub paid_at: Option<DateTime<Utc>>,
    pub provider_metadata: Value,   // normalmente el objeto de entidad del payload
}
```

La implementación de referencia de Stripe lee
`data.object.{id,amount,currency,customer}` para eventos de
`PaymentIntent`/`Charge` y
`data.object.{id,amount_paid,tax,currency,customer,subscription,status_transitions.paid_at}`
para eventos de `Invoice`. La de Paddle lee
`data.{id,customer_id,currency_code,details.totals.{total,tax},billed_at,subscription_id}`.
Replica las convenciones que coincidan con la forma del payload de tu
proveedor - al framework no le importa cómo extraigas, solo que la
instantánea sea correcta.

Si devuelves `None` desde `extract_payment_snapshot`, la fila de
auditoría de todos modos se escribe, pero `payments_transactions` no
se toca. Ese es el retorno correcto para eventos de suscripción /
cliente, o para cualquier evento de pago cuyo payload no lleve
suficiente información para completar una fila.

`CustomerSnapshot` mantiene la sincronización de la copia local de
clientes impulsada por el proveedor (sin rutas JSON codificadas a mano
en el framework):

```rust,ignore
pub struct CustomerSnapshot {
    pub provider_customer_id: String,
    pub email: Option<String>,
    pub provider_metadata: Value,
}
```

El framework hará `email = Set(snapshot.email)` solo cuando la
instantánea provea uno; `provider_metadata` siempre se reemplaza con
la vista del proveedor sobre el cliente (`updated_at` también avanza
sin importar el caso). Las filas de la copia local de clientes solo se
**actualizan** - nunca se insertan - porque `user_id` es `NOT NULL` y
la app es dueña del enlace usuario ↔ cliente vía
`CustomerStore::create_customer`.

### Semántica de fallos

Si `extract_payload_ids` devuelve `None` para `subscription_id` en un
evento de suscripción (o para `customer_id` en un evento de cliente),
el framework lo trata como un error `Validation`: la transacción de
hidratación se revierte, se fija el `process_error` de la fila de
auditoría, y la respuesta HTTP es **503 hydration-failed** para que el
proveedor reintente. Un éxito silencioso ante un payload malformado
dejaría la copia local desactualizada sin que quien opera el sistema
se dé cuenta - los reintentos del proveedor son el mecanismo de
recuperación.

Este contrato significa que el extractor de un adaptador debe
completar los IDs relevantes con honestidad. Devolver `None` está
reservado para eventos que tu proveedor no puede traducir en absoluto
(p. ej. un evento de pago sin ID de cargo en el payload), no para "no
me molesté en analizar este".

## 7. Regístralo al arrancar la app

Hay dos mecanismos disponibles - elige uno:

### Registro en tiempo de ejecución (recomendado para apps con configuración por variables de entorno)

```rust,ignore
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_mollie::MollieProvider;

let mollie = MollieProvider::from_env().expect("Mollie env vars not set");
PaymentProviderRegistry::bind("mollie", Arc::new(mollie));
```

### Registro en tiempo de compilación vía `inventory`

Para crates adaptadores que quieren registro sin configuración - útil
cuando distribuyes una biblioteca que quienes la consumen solo
`cargo add` sin ninguna conexión en tiempo de arranque:

```rust,ignore
use suprnova::payments::{PaymentProviderEntry, PaymentProviderRegistry};
use inventory;

// En lib.rs, dentro de un inicializador estático:
inventory::submit!(PaymentProviderEntry {
    name: "mollie",
    factory: || Arc::new(MollieProvider::from_env().expect("Mollie env not set")),
});
```

`inventory::submit!` se ejecuta antes de `main`. El closure de la
fábrica se llama una vez, cuando se accede al registro por primera
vez.

## 8. Pasa la prueba del discriminador

Todo crate adaptador debería incluir una prueba de integración que
demuestre que el contrato de traits es correcto de punta a punta. Esta
es la prueba de solidez - si esta prueba pasa, el proveedor se conecta
a cualquier app de Suprnova sin sorpresas.

```rust,ignore
// tests/discriminator.rs (dentro de crates/suprnova-payments-mollie/)

use suprnova::payments::*;
use suprnova_payments_mollie::MollieProvider;

/// Requiere que MOLLIE_API_KEY y MOLLIE_WEBHOOK_SECRET estén
/// fijadas.
/// Ejecuta con: cargo test --test discriminator -- --ignored
#[tokio::test]
#[ignore = "requires live Mollie sandbox credentials"]
async fn discriminator_flow() {
    let provider = MollieProvider::from_env().expect("Mollie env vars not set");

    // 1. Crea el cliente
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "test_user_1".into(),
        email: "test@example.com".into(),
        name: Some("Test User".into()),
        metadata: None,
    }).await.expect("create_customer failed");
    assert!(!cus.provider_customer_id.is_empty());

    // 2. Inicia la sesión de checkout
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some("discriminator_test_checkout".into()),
        metadata: None,
    }).await.expect("start_session failed");
    assert!(matches!(session, SessionPayload::Redirect { .. }));

    // 3. Suscribe directamente (si tu proveedor lo soporta; Mollie
    //    puede requerir checkout)
    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        trial_days: None,
        idempotency_key: Some("discriminator_test_sub".into()),
        metadata: None,
    }).await.expect("subscribe failed");
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // 4. Vuelve a leer
    let fetched = provider.get(&sub.provider_subscription_id).await.expect("get failed");
    assert_eq!(fetched.provider_subscription_id, sub.provider_subscription_id);

    // 5. Cancela al final del periodo
    let s = provider.cancel(&sub.provider_subscription_id, true).await.expect("cancel failed");
    assert!(s.cancel_at_period_end);

    // 6. Cancela de inmediato
    let s = provider.cancel(&sub.provider_subscription_id, false).await.expect("cancel failed");
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // 7. Verifica el invariante de as_payment()
    let p: &dyn PaymentProvider = &provider;
    // Si implementaste Payment: assert!(p.as_payment().is_some())
    // Si NO implementaste Payment: assert!(p.as_payment().is_none())
    let _ = p.as_payment();
}
```

Controla las pruebas de integración en vivo con `#[ignore]` para que
`cargo test` pase en CI sin credenciales. Ejecútalas explícitamente
con `-- --ignored` contra una cuenta sandbox.

## 9. Referencia de variantes de `PaymentError`

El enum completo vive en `framework/src/payments/error.rs`. Elige la
variante que coincida con lo que realmente salió mal:

| Variante | Cuándo usarla |
|---|---|
| `Provider(String)` | La API del proveedor devolvió un error que no necesitas traducir más |
| `Validation(String)` | Los campos de la solicitud son inválidos, o el cuerpo de un webhook no se puede analizar |
| `NotSupported(String)` | El método no aplica para este proveedor (p. ej. el `subscribe` de Paddle) |
| `Declined { reason, decline_code }` | Tarjeta rechazada - reenvía `decline_code` cuando el proveedor provea uno |
| `Authentication(String)` | El proveedor rechazó tu clave de API o tus credenciales |
| `NotFound(String)` | El ID de cliente, suscripción, o transacción no existe |
| `WebhookSignature(String)` | Cualquier fallo de firma - encabezado ausente, codificación malformada, o discordancia |
| `InvalidPhoneNumber(String)` | La validación E.164 falló en flujos de Mobile Money |
| `InvalidCountryCode(String)` | La validación ISO-3166-1 alpha-2 falló |
| `Internal(String)` | Error inesperado del SDK, fallo de red, fallo al iniciar el HMAC, o cualquier otro problema del lado del framework |

La ruta de webhook mapea esto a códigos de estado:
`WebhookSignature(_)` → 401, `Validation(_)` desde `parse_event` → 400,
cualquier otra cosa desde la hidratación → 503 (para que el proveedor
reintente).

Una vez que tu adaptador compile y la prueba del discriminador pase:

- Añade tu crate al `Cargo.toml` de tu app con
  `cargo add suprnova-payments-mollie --path ./crates/suprnova-payments-mollie`.
- Regístralo al arrancar, como se mostró en el paso 7.
- Monta `webhook_routes(db.clone())` una sola vez al arrancar la app -
  el mismo handler despacha a cada proveedor registrado por nombre,
  así que un solo montaje sirve a Stripe, Paddle, y tu nuevo
  adaptador.

## Siguiente

- [Pagos](payments.md) - la superficie neutral respecto al proveedor y
  el inicio rápido
- [Pagos - Adaptador de Stripe](payments-stripe.md) - plantilla
  completa para un adaptador de pasarela
- [Pagos - Adaptador de Paddle](payments-paddle.md) - plantilla
  completa para un adaptador de Merchant of Record
- [Pagos - Frontend](payments-frontend.md) - cómo renderizar el
  `SessionPayload` que devuelve tu adaptador
- [Modelo de errores](error-model.md) - cómo `PaymentError` desemboca
  en un `HttpResponse`

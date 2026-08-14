# Pagos

La superficie de pagos de Suprnova es neutral respecto al proveedor.
Elige un crate adaptador - Stripe, Paddle, o uno que escribas tú mismo -,
lo registras al arrancar, y tu código de dominio llama a los mismos
cuatro traits fundamentales (más un quinto opcional para la captura del
lado del servidor) sin importar qué proveedor haya detrás. Las tablas de
copia local de tu base de datos se mantienen sincronizadas mediante
webhooks, así que tu código de dominio lee de tu propia BD en lugar de
llamar a la API del proveedor en cada consulta.

Ninguna función queda supeditada a un solo proveedor. Tanto el modelo de
captura directa de Stripe como el modelo de Merchant of Record de Paddle
encajan en el mismo contrato de traits. La única superficie que difiere
es `Payment` (captura del lado del servidor), que es opcional - Paddle
no la necesita, así que Paddle no la implementa. Los proveedores
anuncian su capacidad anulando `PaymentProvider::as_payment()` para
devolver `Some(&dyn Payment)`; quien llama consulta en tiempo de
ejecución.

## Por qué Suprnova diverge

Laravel distribuye Cashier como una integración de Stripe de primera
parte en su documentación principal. Es cómodo, pero solo para Stripe -
añadir un segundo proveedor implica bifurcar Cashier o construir una
superficie paralela. Suprnova trata a los proveedores de pago igual que
trata los drivers de cache y almacenamiento: un conjunto de traits
genérico, con adaptadores intercambiables. Tu código de dominio nunca
nombra `StripeProvider` ni `PaddleProvider`; llama a
`provider.subscribe(...)` contra un `Arc<dyn PaymentProvider>` resuelto
desde un registro, y el proveedor que hay detrás puede convertirse en
otro con un solo cambio en el bootstrap.

## Inicio rápido

Añade el crate adaptador. Hasta que Suprnova publique su versión v0.1,
el framework y sus crates adaptadores se consumen vía git en lugar de
crates.io:

```toml
# Cargo.toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.2" }
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.2" }
```

Registra el proveedor y el router de webhooks al arrancar. El router de
webhooks es un `Router` normal que compones dentro de tu
`routes::register()`:

```rust,ignore
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_stripe::StripeProvider;

pub async fn register() {
    let stripe = StripeProvider::from_env().expect("Stripe env vars not set");
    PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
}
```

```rust,ignore
// src/routes.rs
use std::sync::Arc;
use suprnova::payments::webhook_routes;
use suprnova::container::App;
use suprnova::Router;
use sea_orm::DatabaseConnection;

/// `Application::routes(routes::register)` llama a esto una vez al
/// arrancar. Partimos del router de webhooks de pagos, y luego
/// apilamos el resto de las rutas de la app encima con las llamadas
/// normales `.get(...)` / `.post(...)`.
pub fn register() -> Router {
    let db: Arc<DatabaseConnection> = App::get().expect("db not bound");

    webhook_routes(db)
        .get("/", crate::controllers::home::index)
        .post("/login", crate::controllers::auth::login)
        // ... el resto de tus rutas ...
        .into()
}
```

`webhook_routes(db)` devuelve un `Router` que contiene solo
`POST /webhooks/payments/{provider}`. Como `Router::get` y `Router::post`
devuelven cada uno un `RouteBuilder` que se convierte de vuelta a
`Router` vía `.into()`, encadenar sobre el router de pagos es la forma
más directa de componer. Si ya usas la macro `routes!{}` para tus rutas
normales, deja caer el POST del webhook dentro del mismo bloque -
`webhook_routes` es un envoltorio de conveniencia alrededor de una sola
llamada a `Router::new().post(...)`.

En tu controlador, busca el proveedor, crea un cliente, y abre una
sesión de checkout:

```rust,ignore
// src/controllers/billing.rs
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("stripe")
        .ok_or_else(|| PaymentError::Internal("stripe not registered".into()))?;

    let customer = provider.create_customer(CreateCustomerRequest {
        user_id,
        email,
        name: None,
        metadata: None,
    }).await?;

    provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: customer.provider_customer_id,
        price_refs: vec!["price_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: None,
        metadata: None,
    }).await
}
```

Ese `SessionPayload` va a las props de tu página Inertia. El frontend
despacha según `payload.flow` para renderizar el widget correcto - ver
[Pagos - Integración de Frontend](payments-frontend.md).

## Elegir un adaptador

### Stripe

```toml
# Cargo.toml
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.2" }
```

Variables de entorno requeridas:

| Variable | Descripción |
|---|---|
| `STRIPE_SECRET_KEY` | Clave secreta (`sk_live_…` / `sk_test_…`) |
| `STRIPE_PUBLISHABLE_KEY` | Clave publicable (`pk_live_…` / `pk_test_…`) |
| `STRIPE_WEBHOOK_SIGNING_SECRET` | Secreto de firma del endpoint de webhook (`whsec_…`) |

```rust,ignore
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// Desde el entorno (recomendado en producción):
let stripe = StripeProvider::from_env().expect("Stripe env vars not set");

// O constrúyelo directamente:
let stripe = StripeProvider::new("sk_test_...", "pk_test_...", "whsec_...");

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

Stripe implementa todos los traits, incluidos los opcionales `Payment`
(captura del lado del servidor vía PaymentIntents) y `Promotions`
(acuñado de códigos de promoción vía `/v1/promotion_codes`). Tanto
`provider.as_payment()` como `provider.as_promotions()` devuelven
`Some`.

### Paddle

```toml
# Cargo.toml
suprnova-payments-paddle = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.2" }
```

Variables de entorno requeridas:

| Variable | Descripción |
|---|---|
| `PADDLE_API_KEY` | Clave de API (`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`) |
| `PADDLE_WEBHOOK_KEY` | Secreto del destino de notificación (`pdl_ntfset_…`) |
| `PADDLE_CLIENT_TOKEN` | Token del lado del cliente (`live_…` / `test_…`) |
| `PADDLE_ENVIRONMENT` | Opcional, por defecto `"sandbox"` |

```rust,ignore
use suprnova_payments_paddle::{PaddleProvider, PaddleEnvironment};
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// Desde el entorno:
let paddle = PaddleProvider::from_env().expect("Paddle env vars not set");

// O constrúyelo directamente:
let paddle = PaddleProvider::new(
    "pdl_sdbx_apikey_...",
    "pdl_ntfset_...",
    "test_...",
    PaddleEnvironment::Sandbox,
).expect("Paddle client init failed");

PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
```

Paddle es un Merchant of Record - gestiona los impuestos, los reintentos
de cobro, y todo el ciclo de vida de la suscripción. No expone captura
del lado del servidor, así que `Payment` no está implementado. Llamar a
`provider.as_payment()` devuelve `None`. Las suscripciones se crean de
forma indirecta: llama a `Checkout::start_session`, completa el widget
de Paddle, y llega el webhook `SubscriptionCreated` para confirmar el id
de la suscripción.

## La división de traits

`PaymentProvider` es un paraguas que agrupa cuatro traits universales -
`Checkout`, `Subscription`, `CustomerStore`, `WebhookHandler` - que todo
adaptador implementa. Hay dos traits más que son opcionales: `Payment`
(la captura del lado del servidor solo tiene sentido para pasarelas como
Stripe) y `Promotions` (acuñado de códigos de promoción). Los
adaptadores se suman anulando `PaymentProvider::as_payment()` /
`PaymentProvider::as_promotions()`.

```rust,ignore
pub trait PaymentProvider: Checkout + Subscription + CustomerStore + WebhookHandler {
    fn name(&self) -> &'static str;

    /// Devuelve `Some` si este proveedor también implementa `Payment`
    /// (captura del lado del servidor). Por defecto devuelve `None`.
    fn as_payment(&self) -> Option<&dyn Payment> {
        None
    }

    /// Devuelve `Some` si este proveedor también implementa
    /// `Promotions` (acuñado de códigos de promoción). Por defecto
    /// devuelve `None`.
    fn as_promotions(&self) -> Option<&dyn Promotions> {
        None
    }
}
```

### `Checkout` - universal, abre el widget del cliente

Todo proveedor implementa `Checkout`. Llama a `start_session` para
obtener un `SessionPayload` etiquetado por flujo que tu frontend
renderiza. `session_status` (por defecto: `NotSupported`; anulado por
los proveedores cuyas sesiones se pueden interrogar, p. ej. Stripe)
informa el estado autoritativo, del lado del proveedor, de una sesión
que iniciaste antes.

```rust,ignore
#[async_trait]
pub trait Checkout: Send + Sync {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload>;

    async fn session_status(&self, provider_session_id: &str)
        -> PaymentResult<CheckoutSessionState>;
}
```

Campos de `StartSessionRequest`:

| Campo | Tipo | Descripción |
|---|---|---|
| `mode` | `SessionMode` | `OneOff` o `Subscription` |
| `customer_ref` | `String` | ID de cliente del proveedor, de `CustomerStore::create_customer` |
| `price_refs` | `Vec<String>` | IDs de precio/producto del proveedor |
| `success_return_url` | `String` | Adónde enviar al usuario después del pago |
| `cancel_return_url` | `String` | Adónde enviar al usuario si abandona |
| `amount_hint` | `Option<Money>` | Anulación o pista para importes puntuales |
| `idempotency_key` | `Option<String>` | Para reintentos seguros |

`session_status` es la primitiva de verificación del lado del servidor
para los flujos de redirección. Cuando el cliente vuelve a caer en tu
página de retorno, NO confíes en los parámetros de consulta que trajo
su navegador - pasa el `provider_session_id` que registraste al llamar
a `start_session` y bifurca según el resultado:

```rust,ignore
match provider.session_status(&order.provider_session_id).await? {
    CheckoutSessionState::Complete { paid: true, payment_ref, amount_total } => {
        // Cumple con el pedido. `payment_ref` (p. ej. el `pi_…` de
        // Stripe) se correlaciona con las operaciones de `Payment` y
        // con la copia local de payments_transactions.
    }
    CheckoutSessionState::Complete { paid: false, .. } => { /* liquidación pendiente */ }
    CheckoutSessionState::Open => { /* el cliente no ha terminado de pagar */ }
    CheckoutSessionState::Expired => { /* la sesión caducó - cierra el pedido */ }
}
```

La misma llamada impulsa los barridos de reconciliación: vuelve a
sondear los pedidos que sigan abiertos en tu base de datos y cumple con
aquellos cuyas sesiones se completaron después de que el cliente cerrara
la pestaña.

### `Payment` - opcional, captura del lado del servidor

Solo los proveedores que exponen captura del lado del servidor
implementan `Payment`. Stripe sí; Paddle no. Para comprobarlo en tiempo
de ejecución:

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(payment) = provider.as_payment() {
    let result = payment.charge(ChargeRequest {
        customer_ref: "cus_...".into(),
        payment_method_ref: "pm_...".into(),
        amount: Money::from_minor_units(2999, Currency::USD),
        description: Some("Pro plan one-off".into()),
        idempotency_key: Some("charge_user42_order99".into()),
        metadata: None,
    }).await?;
}
```

Interfaz completa de `Payment`:

```rust,ignore
#[async_trait]
pub trait Payment: Send + Sync {
    async fn charge(&self, req: ChargeRequest) -> PaymentResult<ChargeResult>;
    async fn capture(&self, provider_transaction_id: &str) -> PaymentResult<ChargeResult>;
    async fn refund(&self, req: RefundRequest) -> PaymentResult<RefundResult>;
    async fn void(&self, provider_transaction_id: &str) -> PaymentResult<()>;
    async fn status(&self, provider_transaction_id: &str) -> PaymentResult<PaymentStatus>;
}
```

`ChargeResult` es un enum etiquetado con `kind` - ver la sección
[El dinero y ChargeResult](#chargeresult).

### `Promotions` - opcional, acuña códigos de promoción

Los proveedores con una superficie de códigos de promoción implementan
`Promotions`. El objeto de descuento en sí (un cupón de porcentaje o de
importe fijo) se crea de antemano - normalmente una vez, en el panel del
proveedor - y este trait acuña *códigos* a partir de él, cada uno
restringido a un cliente y a una ventana de canje. Esa es la forma que
necesitan las campañas de recuperación y de upsell: cada destinatario
recibe un código personal, inutilizable por cualquier otra persona y
muerto en cuanto se cierra la ventana.

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(promotions) = provider.as_promotions() {
    let minted = promotions.create_promotion_code(CreatePromotionCodeRequest {
        coupon_ref: "coupon_15off".into(),          // cupón ya creado
        customer_ref: "cus_...".into(),             // solo este cliente puede canjearlo
        expires_at: Some(chrono::Utc::now() + chrono::Duration::days(7)),
        max_redemptions: Some(1),                   // un solo uso
    }).await?;
    // Envía por correo `minted.code` al cliente; lo introduce en el
    // checkout y el proveedor hace cumplir cada restricción.
}
```

`MockPaymentProvider` implementa `Promotions` (los códigos se acuñan
como `PROMO_MOCK_n`) y registra cada solicitud - haz aserciones sobre
`recorded_promotion_requests()` en las pruebas.

### `Subscription` - suscribir, actualizar, cancelar, obtener

```rust,ignore
#[async_trait]
pub trait Subscription: Send + Sync {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult>;
    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult>;
    async fn cancel(&self, provider_subscription_id: &str, at_period_end: bool) -> PaymentResult<SubscriptionResult>;
    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult>;
}
```

Cancelar al final del periodo (mantiene el acceso hasta que termine el
ciclo de facturación):

```rust,ignore
let sub = provider.cancel(&sub_id, true).await?;
// sub.cancel_at_period_end == true, sub.status == Active

// Cancelar de inmediato:
let sub = provider.cancel(&sub_id, false).await?;
// sub.status == Canceled
```

Nota: `Paddle::subscribe` devuelve `PaymentError::NotSupported` - Paddle
crea las suscripciones a través de la finalización del checkout, no con
llamadas directas a la API. Usa `Checkout::start_session` y espera el
webhook `SubscriptionCreated`.

### `CustomerStore` - crear, actualizar, obtener, eliminar

```rust,ignore
#[async_trait]
pub trait CustomerStore: Send + Sync {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef>;
    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()>;
}
```

`CreateCustomerRequest` toma `user_id`, `email`, `name: Option<String>`,
y `metadata: Option<Value>`. `CustomerRef` vuelve con
`provider_customer_id` - guarda eso junto a tu registro de usuario para
usarlo en llamadas posteriores.

### `WebhookHandler` - verificar, analizar, y extraer

```rust,ignore
#[async_trait]
pub trait WebhookHandler: Send + Sync {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()>;
    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent>;

    /// Extrae los IDs de entidad del payload en bruto para que el
    /// framework sepa qué filas de copia local hidratar. Por defecto
    /// devuelve un `PayloadIds` vacío.
    fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;

    /// Construye un `PaymentSnapshot` a partir de un evento de pago o
    /// de factura. Por defecto devuelve `None`, lo cual omite el
    /// upsert de `payments_transactions`.
    fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;

    /// Construye un `CustomerSnapshot` a partir de un evento de
    /// cliente. Por defecto devuelve `None`, lo cual omite la
    /// actualización de email / metadatos en la fila existente.
    fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
}
```

En la práctica nunca llamas a nada de esto directamente - `webhook_routes`
los invoca en cada webhook entrante. Viven en el trait para que los
crates adaptadores puedan implementar la verificación de firma, el
análisis de eventos, y la extracción de payload específicos del
proveedor de forma comprobable. Los métodos `extract_*` tienen todos
valores por defecto sensatos; los adaptadores de Stripe y Paddle que se
distribuyen los anulan con implementaciones conscientes de la forma de
cada proveedor (Stripe entra en `data.object.*`, Paddle en `data.*`).

## El payload de Inertia etiquetado por flujo

`start_session` devuelve un enum `SessionPayload` que se serializa a
JSON con un campo discriminador `flow`. Tu frontend conmuta según `flow`
para renderizar el widget correcto:

```rust,ignore
#[serde(tag = "flow", rename_all = "snake_case")]
pub enum SessionPayload {
    StripeElements {
        client_secret: String,
        publishable_key: String,
        provider_session_id: String,
    },
    StripeCheckoutRedirect {
        url: String,
        provider_session_id: String,
    },
    PaddleInline {
        transaction_id: String,
        customer_token: Option<String>,
        client_token: String,
    },
    /// Flujo de Mobile Money - sin redirección ni incrustación. El
    /// frontend muestra un mensaje al cliente pidiéndole que confirme
    /// desde su teléfono (aviso USSD o app del operador), y luego
    /// sondea al proveedor vía `provider_transaction_id` para las
    /// actualizaciones de estado.
    MobileMoneyPrompt {
        provider_transaction_id: String,
        message: String,
        operator: MobileMoneyOperator,
    },
    Redirect {
        url: String,
        provider_session_id: String,
    },
}
```

Forma serializada de un payload `StripeElements`:

```json
{
  "flow": "stripe_elements",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_...",
  "provider_session_id": "pi_..."
}
```

Un payload `MobileMoneyPrompt` luce así - no hay URL porque el cliente
nunca abandona tu página; el frontend renderiza `message` y empieza a
sondear:

```json
{
  "flow": "mobile_money_prompt",
  "provider_transaction_id": "ch_mm_...",
  "message": "Check your phone for the MTN MoMo prompt.",
  "operator": { "kind": "mtn_momo" }
}
```

Devuelve la variante que produzca el proveedor desde tu controlador
como props de Inertia. La integración de frontend se describe en
[Pagos - Integración de Frontend](payments-frontend.md).

## Tablas de copia local

El framework crea seis tablas mediante la migración. Trae el alias
público e incluyelo en el migrador de tu app:

```rust,ignore
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... tus otras migraciones ...
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

El mismo módulo también exporta un ayudante
`pub fn migrations() -> Vec<Box<dyn MigrationTrait>>` si prefieres
llamar a eso y repartir el resultado dentro de tu propia lista.

### Resumen de las tablas

| Tabla | Propósito |
|---|---|
| `payments_customers` | Una fila por cada par `(provider, user_id)` |
| `payments_payment_methods` | Métodos de pago guardados por cliente |
| `payments_subscriptions` | Estado del ciclo de vida de la suscripción |
| `payments_subscription_items` | Partidas dentro de una suscripción |
| `payments_transactions` | Cargos puntuales y facturas de suscripción |
| `payments_webhook_events` | Registro de auditoría y salvaguarda de idempotencia |

Toda tabla tiene una columna JSON `provider_metadata`. Cuando la
representación neutral del framework no cubre un campo específico del
proveedor, léelo de ahí.

### Tabla de transacciones

`payments_transactions` divide los importes en `amount_total_minor` y
`amount_tax_minor`. Stripe reporta importes sin impuestos - el impuesto
es cero en la fila de la transacción, y cualquier dato de impuestos vive
en `provider_metadata`. Paddle reporta importes con impuestos incluidos
y fija `amount_tax_minor` al componente de impuesto. Ambas
representaciones funcionan; suma
`amount_total_minor - amount_tax_minor` para obtener el importe neto.

### Tabla de eventos de webhook

`payments_webhook_events` tiene un índice `UNIQUE(provider, provider_event_id)`.
Todo webhook entrante se comprueba contra esto antes de procesarlo - los
duplicados devuelven 200 OK sin volver a procesarse. Esto es
estructural: Stripe, Paddle, y la mayoría de los proveedores reintentan
agresivamente los webhooks fallidos.

### Advertencias

El código de dominio lee de las tablas de copia local, no directamente
de la API del proveedor. Las mutaciones (crear suscripción, cancelar,
etc.) van al proveedor; el webhook resultante vuelve a sincronizar las
tablas de copia local. Esto significa que hay una breve ventana entre
una mutación y la llegada del webhook en la que tus tablas de copia
local se quedan atrás. Diseña tu UX teniendo esto en cuenta (muestra
estados "procesando", confía en las URLs de redirección del proveedor
para la confirmación inmediata).

## Manejo de webhooks

Monta la ruta de ingesta de webhooks una sola vez al arrancar - ver el
ejemplo de rutas de [Inicio rápido](#inicio-rápido) para el patrón de
composición. `webhook_routes(db)` devuelve un `Router` que lleva el
único handler `POST /webhooks/payments/{provider}` incorporado al
framework. Encadenas tus propias rutas sobre él (o llamas a las
primitivas subyacentes de la ruta directamente dentro de tu propio
bloque `routes!{}`).

El handler del framework hace esto en cada solicitud:

1. Busca el proveedor nombrado en `PaymentProviderRegistry`.
2. Llama a `WebhookHandler::verify` para comprobar la firma. Devuelve
   401 si falla.
3. Llama a `WebhookHandler::parse_event` para construir un
   `WebhookEvent`. Devuelve 400 si el análisis falla.
4. Comprueba `payments_webhook_events` en busca de una fila existente
   con el mismo `(provider, provider_event_id)`. Si la encuentra,
   devuelve 200 de inmediato - esta es la salvaguarda de idempotencia.
5. Inserta la fila de auditoría.

### Estructura de WebhookEvent

```rust,ignore
pub struct WebhookEvent {
    pub provider: String,
    pub provider_event_id: String,
    pub provider_event_type: String,        // cadena en bruto del proveedor, p. ej. "customer.subscription.created"
    pub neutral: Option<NeutralEventKind>,  // mapeado a la taxonomía del framework, o None para eventos específicos del proveedor
    pub raw_payload: Value,                 // cuerpo JSON completo para los casos sin asignar
}
```

`NeutralEventKind` cubre el camino común:

```rust,ignore
pub enum NeutralEventKind {
    PaymentSucceeded,
    PaymentFailed,
    PaymentRefunded,
    PaymentDisputed,
    SubscriptionCreated,
    SubscriptionUpdated,
    SubscriptionCanceled,
    InvoicePaid,
    InvoiceFailed,
    CustomerCreated,
    CustomerUpdated,
}
```

Cuando `neutral` es `None`, el evento es específico del proveedor. Lee
`provider_event_type` y `raw_payload` para los datos completos.

### Hidratación de la tabla de copia local

Después de persistir la fila de auditoría, el framework despacha el
evento hacia la tabla de copia local correspondiente según `neutral`.
**Todas las escrituras de copia local de un evento ocurren dentro de
una sola transacción de BD junto con `mark_processed`** - el estado
parcial de la copia local nunca es observable. O todo se confirma junto,
o todo se revierte.

| `NeutralEventKind` | Efecto en la copia local |
|----------------------------------|-----------------------------------------------------------------------------------------------------|
| `SubscriptionCreated/Updated` | Llama a `Subscription::get(id)` en el proveedor, hace upsert de `payments_subscriptions`, sincroniza las partidas. |
| `SubscriptionCanceled` | Igual que arriba; además fija `canceled_at` y cambia `status` a `canceled` en la fila existente. |
| `PaymentSucceeded / Failed / Refunded / Disputed` | Hace upsert de `payments_transactions` a partir de la instantánea que el proveedor produce desde `raw_payload`. |
| `InvoicePaid / InvoiceFailed` | Hace upsert de `payments_transactions` con `provider_subscription_id` enlazado. |
| `CustomerCreated / CustomerUpdated` | Actualiza el `email` / `provider_metadata` de la fila existente en `payments_customers` a partir del `CustomerSnapshot` del proveedor. **Nunca inserta.** |
| `None` (sin asignar) | Solo fila de auditoría - sin cambio en la copia local. |

La copia local de clientes es deliberadamente de solo actualización en
el camino del webhook. `user_id` es `NOT NULL` y solo la app sabe a qué
usuario pertenece un cliente del proveedor (el enlace lo crea tu código
justo después de `CustomerStore::create_customer`). Los clientes fuera
de banda - creados en el panel de Stripe, por ejemplo - se registran en
el log pero nunca se sintetizan dentro de la copia local.

### Contrato de recuperación ante fallos

El handler trata los reintentos del proveedor como el mecanismo de
recuperación:

- **La hidratación tiene éxito:** la transacción se confirma, se fija
  `processed_at`, se limpia `process_error`. Respuesta: `200 ok`.
- **La hidratación falla:** la transacción se revierte (sin estado
  parcial de copia local), la fila de auditoría mantiene
  `processed_at = NULL` y `process_error` registra el fallo. Respuesta:
  `503 hydration-failed` - el proveedor reintentará con backoff.
- **El proveedor reintenta el evento fallido:** la comprobación de
  idempotencia ve la fila de auditoría existente pero con
  `processed_at IS NULL`, así que la hidratación se ejecuta de nuevo.
  El reintento sustituye el `process_error` obsoleto por el resultado
  del intento actual.
- **El proveedor reintenta un evento exitoso:** la comprobación de
  idempotencia ve `processed_at IS NOT NULL`, y devuelve
  `200 duplicate` de inmediato. Sin nueva hidratación.

Un evento de suscripción/cliente al que le falte `subscription_id` /
`customer_id` en el payload se trata como un error `Validation`
(también 503 + `process_error` registrado). Un éxito silencioso ante un
payload malformado dejaría la copia local desactualizada sin que quien
opera el sistema se dé cuenta.

Las partidas eliminadas de una suscripción en el lado del proveedor
(p. ej. el usuario soltó un complemento de puesto) se eliminan de
`payments_subscription_items` cuando llega el siguiente webhook
`subscription.updated`. La respuesta de `Subscription::get(id)` del
proveedor es la fuente de verdad en cada sincronización.

## Métodos de pago más allá de las tarjetas

`PaymentMethod` es el enum que el framework usa para los métodos
guardados en `payments_payment_methods` y para cualquier proveedor que
exponga metadatos de método. Cubre los casos obvios - tarjetas,
transferencias bancarias, monederos electrónicos - más los métodos
regionales que son de primera clase en muchos mercados:

```rust,ignore
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentMethod {
    Card { brand: String, last4: String, exp_month: u8, exp_year: u16 },
    BankTransfer { bank_name: String, last4: String },
    EWallet { provider: String, identifier: String },
    /// Pagador identificado por teléfono + operador + país.
    MobileMoney {
        operator: MobileMoneyOperator,
        phone: PhoneNumber,
        country: CountryCode,
    },
    /// Cripto anclada - equivalente a efectivo para la mayoría de los
    /// proveedores.
    Stablecoin { asset: StablecoinAsset, network: Option<String> },
    /// Criptomoneda sin anclar.
    Crypto { network: String, address: String },
    /// Vía de escape para métodos regionales o específicos del
    /// proveedor que aún no están modelados.
    Custom { kind: String, descriptor: String },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MobileMoneyOperator {
    MtnMomo,
    Mpesa,
    AirtelMoney,
    OrangeMoney,
    Lipila,
    Custom { identifier: String },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StablecoinAsset {
    Usdc,
    Usdt,
    Dai,
    Custom { ticker: String },
}
```

Los operadores y activos nombrados son los que hemos enumerado. Las
variantes `Custom { ... }` de cada uno cubren operadores regionales y
stablecoins que aún no hemos fijado, así que añadir soporte para uno no
obliga a una nueva versión del framework.

`PhoneNumber` y `CountryCode` son DTOs validados en
`suprnova::payments` - rechazan la entrada malformada en el momento de
la construcción, que es donde quieres el fallo, en lugar de en la
llamada al proveedor.

## El dinero

Los importes se representan como `Money` - un contador `i64` en
unidades menores más una `Currency`. Sin `f64` de por medio.

```rust,ignore
use suprnova::payments::{Money, Currency};
use rust_decimal::Decimal;
use std::str::FromStr;

// A partir de unidades menores (céntimos, peniques, yenes, etc.)
let price = Money::from_minor_units(1999, Currency::USD);  // $19.99

// A partir de una cadena decimal
let price = Money::from_decimal(Decimal::from_str("19.99").unwrap(), Currency::USD);

// Monedas sin decimales - 1234 menor = 1234 JPY (sin conversión)
let yen = Money::from_minor_units(1234, Currency::JPY);

// Aritmética - entra en pánico si las monedas no coinciden
let total = price + Money::from_minor_units(100, Currency::USD);  // $20.99

// Los valores negativos representan reembolsos o créditos
let refund = Money::from_minor_units(-500, Currency::USD);  // -$5.00

// Léelo de vuelta
println!("{} minor units in {:?}", price.minor_units(), price.currency());
```

`Add` y `Sub` entran en pánico si las monedas no coinciden y en caso de
desbordamiento de `i64`. Usa la aritmética que entra en pánico por
corrección - una suma silenciosa entre monedas distintas es un bug, no
una característica.

## ChargeResult

`Payment::charge` devuelve un enum `ChargeResult`. No todo cargo se
completa de inmediato - el step-up de 3DS y las tarjetas fuera de sesión
pueden requerir una redirección o una acción del lado del cliente:

```rust,ignore
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChargeResult {
    Completed {
        provider_transaction_id: String,
        amount: Money,
        status: PaymentStatus,
        provider_metadata: Value,
    },
    RedirectRequired {
        provider_transaction_id: String,
        url: String,
        return_to: Option<String>,
    },
    RequiresClientAction {
        provider_transaction_id: String,
        action_kind: String,
        client_secret: Option<String>,
        publishable_key: Option<String>,
    },
}
```

Maneja `RequiresClientAction` devolviendo el payload a tu frontend. El
frontend renderiza el desafío 3DS usando `client_secret` +
`publishable_key`. Consulta
[Pagos - Integración de Frontend](payments-frontend.md) para el código
de despacho del frontend.

## Claves de idempotencia

Todo DTO que muta tiene un `idempotency_key: Option<String>` opcional.
Fija uno en las llamadas de red que se puedan reintentar:

```rust,ignore
provider.start_session(StartSessionRequest {
    // ...
    idempotency_key: Some(format!("checkout_{}_{}", user_id, order_id)),
    // ...
}).await?;

provider.subscribe(SubscribeRequest {
    // ...
    idempotency_key: Some(format!("sub_{}_{}", user_id, plan_id)),
    // ...
}).await?;
```

Stripe honra las claves de idempotencia mediante el encabezado HTTP
`Idempotency-Key`. Paddle tiene un mecanismo equivalente. Si una
solicitud falla a mitad de camino y reintentas con la misma clave, el
proveedor devuelve la respuesta original en lugar de crear un cargo o
una suscripción duplicados.

## El patrón discriminador

Todo adaptador que afirme implementar `PaymentProvider` debe pasar el
mismo flujo E2E:

```
create_customer → start_session → subscribe → get → cancel(at_period_end) → cancel(immediate) → assert as_payment invariant
```

El `MockPaymentProvider` incluido con el framework pasa esto:

```rust,ignore
use suprnova::payments::*;

#[tokio::test]
async fn discriminator_flow() {
    let provider = MockPaymentProvider::new();

    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "user_42".into(),
        email: "alice@example.com".into(),
        name: Some("Alice".into()),
        metadata: None,
    }).await.unwrap();

    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["price_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some("idem_1".into()),
        metadata: None,
    }).await.unwrap();
    assert!(matches!(session, SessionPayload::Redirect { .. }));

    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["price_pro_monthly".into()],
        trial_days: None,
        idempotency_key: Some("idem_2".into()),
        metadata: None,
    }).await.unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // Cancela al final del periodo
    let s = provider.cancel(&sub.provider_subscription_id, true).await.unwrap();
    assert!(s.cancel_at_period_end);

    // Cancela de inmediato
    let s = provider.cancel(&sub.provider_subscription_id, false).await.unwrap();
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // MockPaymentProvider omite Payment a propósito (opcional al estilo Paddle)
    let p: &dyn PaymentProvider = &provider;
    assert!(p.as_payment().is_none());
}
```

`MockPaymentProvider` no implementa `Payment` - esto ejercita el mismo
invariante que Paddle. Tanto `StripeProvider` como `PaddleProvider`
pasan el mismo flujo contra la API en vivo, en pruebas de integración.

## Apps multiproveedor

Registra ambos adaptadores al arrancar y despacha según dónde se creó
el registro de cada cliente:

```rust,ignore
PaymentProviderRegistry::bind("stripe", Arc::new(stripe_provider));
PaymentProviderRegistry::bind("paddle", Arc::new(paddle_provider));

// Más adelante, por solicitud:
let provider_name = user.payment_provider.as_str(); // "stripe" o "paddle"
let provider = PaymentProviderRegistry::get(provider_name).expect("unknown provider");
let sub = provider.cancel(&sub_id, true).await?;
```

Usos comunes: enrutar a los clientes de la UE a través de Paddle (para
el manejo de impuestos como MoR) y a los de EE. UU. a través de Stripe;
hacer pruebas A/B de conversión de checkout entre proveedores; usar un
proveedor para suscripciones y otro para cargos puntuales.

## Migración desde Laravel Cashier

Cashier es exclusivo de Stripe por diseño. Suprnova ofrece
multiproveedor de fábrica. Mapeo rápido:

| Laravel Cashier | Suprnova |
|---|---|
| `$user->newSubscription('default', 'price_pro')->create()` | `provider.subscribe(SubscribeRequest { ... }).await` |
| `$user->subscription('default')->cancel()` | `provider.cancel(&sub_id, true).await` |
| `Cashier::webhookHandler` | `webhook_routes(db.clone())` |
| `$user->createAsStripeCustomer()` | `provider.create_customer(CreateCustomerRequest { ... }).await` |
| `$user->charge(1999, 'pm_...')` | `payment.charge(ChargeRequest { ... }).await` (si el proveedor lo soporta) |
| `$invoice->download()` | No viene incluido; lee `provider_metadata["invoice_pdf_url"]` desde la tabla de copia local de transacciones |

## Siguiente

- [Pagos - Adaptador de Stripe](payments-stripe.md) - el flujo de la
  pasarela en detalle: PaymentIntents, el formato de firma de webhook,
  el mapeo de tipos de evento
- [Pagos - Adaptador de Paddle](payments-paddle.md) - el flujo de MoR
  en detalle: creación de suscripciones guiada por el checkout, manejo
  de impuestos, verificación de notificaciones
- [Pagos - Integración de Frontend](payments-frontend.md) - ejemplos de
  despacho por flujo en Svelte 5, React 19 y Vue 3.5
- [Escribir un adaptador de proveedor de pagos](payments-provider-guide.md) -
  construye tu propio crate adaptador de principio a fin
- [Base de datos](database.md) - la capa de SeaORM sobre la que se
  apoyan las tablas de copia local

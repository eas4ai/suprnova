# Pagos - Adaptador de Paddle

El adaptador de Paddle (`suprnova-payments-paddle`) conecta Paddle a
la superficie de pagos genérica de Suprnova. Recurre a él cuando
quieras un proveedor de pago que también maneje, en tu nombre, el
impuesto sobre ventas, el IVA, el GST, los reintentos de cobro, la
facturación, y los reembolsos - Paddle es un Merchant of Record (MoR),
lo cual significa que es el vendedor de registro ante tus clientes y
absorbe la superficie de cumplimiento que una pasarela de captura
directa como Stripe te deja a ti.

Esa elección cambia el modelo mental. Tu código de dominio no *es
dueño* de la suscripción - Paddle sí lo es. Abres un checkout, el
cliente lo completa, y el webhook `SubscriptionCreated` te dice que la
suscripción ya existe. No puedes crear una suscripción vía API, y no
puedes cambiar su conjunto de precios después del hecho. Puedes
cancelar, puedes leer el estado, puedes actualizar los metadatos de
facturación. El resto es de Paddle.

Este capítulo asume que ya leíste [Pagos](payments.md) para la
superficie genérica de cinco traits. Aquí cubrimos lo que es cierto
*solo* para Paddle.

## Cuándo elegir Paddle

Elige Paddle cuando se cumpla una o más de estas condiciones:

- Vendes productos digitales a nivel global y el cumplimiento
  tributario (IVA, GST, impuesto sobre ventas de EE. UU.) es un costo
  real en tu hoja de ruta.
- No quieres gestionar tú mismo los reintentos de pagos fallidos, los
  correos de cobro, o la emisión de recibos.
- Quieres una sola factura de un único vendedor de registro para tu
  contabilidad.
- Tu modelo de negocio es primero-suscripción, y aceptas que el
  proveedor impulse el ciclo de vida de la suscripción.

Elige [Stripe](payments.md#stripe) en su lugar cuando quieras control
directo sobre la captura de cargos, manejes tú mismo el impuesto, o
necesites llamadas `charge`/`capture`/`refund` del lado del servidor
desde tus propias rutas de código.

## Configuración

Añade el crate:

```bash
cargo add suprnova-payments-paddle
```

Fija las cuatro variables de entorno:

```env
PADDLE_API_KEY=pdl_sdbx_apikey_...
PADDLE_WEBHOOK_KEY=pdl_ntfset_...
PADDLE_CLIENT_TOKEN=test_...
PADDLE_ENVIRONMENT=sandbox
```

| Variable | Qué es | De dónde viene |
|---|---|---|
| `PADDLE_API_KEY` | Clave de API del lado del servidor (`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`) | Panel de Paddle → Developer Tools → Authentication |
| `PADDLE_WEBHOOK_KEY` | Secreto del destino de notificación (`pdl_ntfset_…`) | Panel de Paddle → Developer Tools → Notifications → tu endpoint |
| `PADDLE_CLIENT_TOKEN` | Token de cliente seguro para el navegador (`live_…` / `test_…`) | Panel de Paddle → Developer Tools → Authentication → Client-side tokens |
| `PADDLE_ENVIRONMENT` | `sandbox` (por defecto) o `production` | Tu decisión |

Registra el proveedor al arrancar. Ambas formas son válidas:

```rust
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

pub async fn bootstrap() {
    // Desde el entorno (recomendado):
    let paddle = PaddleProvider::from_env()
        .expect("Paddle env vars not set");

    // O constrúyelo directamente:
    let paddle = PaddleProvider::new(
        "pdl_sdbx_apikey_...",
        "pdl_ntfset_...",
        "test_...",
        PaddleEnvironment::Sandbox,
    ).expect("Paddle client init failed");

    PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
}
```

La ruta de ingesta de webhooks la registra el ayudante
`webhook_routes(db.clone())` del framework - ver
[Pagos](payments.md#webhook-handling). Tanto `from_env()` como `new()`
devuelven `Result` porque el `paddle_rust_sdk::Paddle::new` subyacente
valida la forma de la clave de API y la URL del endpoint en el momento
de la construcción.

## El modelo mental de MoR

La forma que sorprende a los usuarios de Stripe:

```
Stripe (pasarela):
    tu app    ─────────►  Stripe  ──►  red de tarjetas
       │                    ▲
       └────── webhook ─────┘
    eres dueño del estado de la suscripción en tu BD; Stripe es quien ejecuta

Paddle (Merchant of Record):
    tu app  ─►  enlace de checkout  ─►  cliente  ──►  Paddle  ──►  red de tarjetas
                                                          │
       ◄───────────────────  webhook  ───────────────────┘
    Paddle es dueño del estado de la suscripción; tu BD es la copia local
```

En el código, la diferencia aparece en tres puntos:

1. **No puedes crear una suscripción vía API.** Llama a
   `Checkout::start_session` con un precio recurrente; el cliente
   completa el widget de Paddle; el webhook `SubscriptionCreated`
   hidrata tu copia local.
2. **No puedes cambiar el conjunto de precios de una suscripción vía
   API.** Paddle reserva los cambios de plan para su propio panel o
   para flujos de migración que le pertenecen.
3. **No puedes eliminar un cliente.** Archivar vía update es la
   solución alternativa soportada.

Suprnova hace emerger estas restricciones como
`PaymentError::NotSupported` en lugar de disimularlas - ver la
[matriz de capacidades](#matriz-de-capacidades) más abajo.

## Flujo de checkout

`Checkout::start_session` es la única forma de iniciar un pago con
Paddle. El frontend abre el `transaction_id` resultante con paddle.js,
usando el `client_token` que fijaste al arrancar:

```rust
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("paddle")
        .expect("paddle provider not registered");

    // 1. Crea el cliente en Paddle (o reutiliza uno existente).
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: user_id.clone(),
        email,
        name: None,
        metadata: None,
    }).await?;

    // 2. Abre una sesión de checkout. Paddle decide entre pago
    //    puntual y suscripción según el *tipo de precio*, no según
    //    el campo SessionMode de abajo.
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,           // ignorado por Paddle (ver nota)
        customer_ref: cus.provider_customer_id,
        price_refs: vec!["pri_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some(format!("checkout_{user_id}")),
        metadata: None,
    }).await?;

    Ok(session)
}
```

El `SessionPayload::PaddleInline` devuelto lleva todo lo que necesita
el frontend:

```json
{
  "flow": "paddle_inline",
  "transaction_id": "txn_01h...",
  "customer_token": "ctm_01h...",
  "client_token": "test_..."
}
```

Ver [Pagos - Integración de Frontend](payments-frontend.md) para el
código de montaje de paddle.js en Svelte / React / Vue.

### Paddle decide según el tipo de precio, no según `SessionMode`

Una trampa genuinamente específica de Paddle: el campo
`SessionMode::OneOff` / `SessionMode::Subscription` de
`StartSessionRequest` **es ignorado por el adaptador de Paddle**. La
API de Paddle tiene un único endpoint `transaction_create`, y el
proveedor inspecciona los IDs de precio suministrados para inferir el
flujo - un precio recurrente inicia una suscripción, un precio puntual
inicia un cargo único. Con Stripe el campo impulsa el flujo; con
Paddle es el *precio* el que lo hace. Configura tu catálogo de Paddle
con los tipos de precio correctos antes de apuntar el adaptador hacia
ellos.

## Las suscripciones llegan vía webhook

Como Paddle es dueño del ciclo de vida de la suscripción, tu código de
dominio solo *se entera* de una suscripción cuando Paddle te lo dice.
El flujo:

```
tu app                           Paddle                    cliente
   │                              │                          │
   │  start_session(price=pri_…)  │                          │
   ├─────────────────────────────►│                          │
   │  PaddleInline { txn_id, … }  │                          │
   │◄─────────────────────────────┤                          │
   │                              │       paddle.js          │
   │                              │◄─────────────────────────┤
   │                              │   completa el checkout   │
   │                              ├─────────────────────────►│
   │                              │                          │
   │   webhook subscription.created                          │
   │◄─────────────────────────────┤                          │
   │                              │                          │
   ▼                              │                          │
 tablas de copia local hidratadas;│                          │
 la fila de payments_subscriptions│                          │
 tiene provider_subscription_id   │                          │
```

El handler `webhook_routes(db)` del framework hace la hidratación
por ti: llama a `WebhookHandler::extract_payload_ids` para encontrar
el `subscription_id`, llama a `Subscription::get(id)` para leer el
estado canónico, y hace upsert de `payments_subscriptions` +
`payments_subscription_items` dentro de una sola transacción. Para
cuando el webhook devuelve 200, tu copia local es consistente con
Paddle.

Hay una breve ventana entre que el cliente completa el widget y que
llega el webhook, en la que `payments_subscriptions` no tiene ninguna
fila para la nueva suscripción. Dos patrones la cubren:

- **Usa la URL de redirección para una UX inmediata.**
  `success_return_url` se dispara del lado del cliente en cuanto
  Paddle confirma la transacción, así que puedes mostrar "Suscripción
  activa" sin esperar al webhook del lado del servidor.
- **Sondear y renderizar.** Después de la redirección, refresca la
  página tras una breve demora para que el controlador de Inertia
  pueda leer la copia local ya hidratada.

## Matriz de capacidades

No todo método de cada trait hace lo que hace su equivalente de
Stripe. La tabla de abajo es la verdad. `subscribe()` y `update()` con
`new_price_refs.is_some()` son los únicos métodos que *siempre*
fallan; el resto funciona, con las advertencias señaladas.

| Método del trait | Comportamiento |
|---|---|
| `Checkout::start_session` | Funciona. Decide entre pago puntual y suscripción según el tipo de precio, no según `SessionMode`. |
| `Subscription::subscribe` | Siempre `NotSupported`. Las suscripciones nacen de la finalización del checkout + webhook. |
| `Subscription::update(cancel_at_period_end: Some(true), new_price_refs: None)` | Funciona. Se conecta a `subscription_cancel` con el valor por defecto `EffectiveFrom::NextBillingPeriod`. |
| `Subscription::update(new_price_refs: Some(...))` | `NotSupported` en v1. Paddle reserva el reemplazo del conjunto de precios para sus propios flujos de migración. |
| `Subscription::update` (no-op) | Funciona. Vuelve a obtener el estado actual vía `subscription_get`. |
| `Subscription::cancel` | Funciona, pero `at_period_end` se **ignora** - siempre programa para el siguiente periodo de facturación. Ver [más abajo](#la-cancelación-siempre-queda-programada). |
| `Subscription::get` | Funciona. |
| `CustomerStore::create_customer` | Funciona. |
| `CustomerStore::update_customer` | Funciona. |
| `CustomerStore::get_customer` | Funciona. |
| `CustomerStore::delete_customer` | `NotSupported`. Usa `update_customer` con el estado `archived` si lo necesitas. |
| `Payment::*` | El trait no está implementado. `provider.as_payment()` devuelve `None`. |
| `WebhookHandler::*` | Funciona. |

Los invariantes de que `Payment` no esté implementado, de que
`subscribe`/`delete_customer` devuelvan `NotSupported`, y del rechazo
de firma de webhook, están fijados por pruebas siempre activas en
`crates/suprnova-payments-paddle/tests/integration.rs`, así que la
matriz de arriba no se desactualizará en silencio.

### La cancelación siempre queda programada

`Subscription::cancel(id, at_period_end)` acepta el bool por
compatibilidad de trait, pero **siempre se comporta como una
cancelación programada** - el enum `EffectiveFrom` de Paddle es
privado en `paddle_rust_sdk` 0.18, así que la cancelación inmediata no
es viable en v1. El usuario mantiene el acceso hasta que termina el
periodo de facturación actual, momento en el cual Paddle dispara
`subscription.canceled` y la copia local cambia `status` a `Canceled`.

Si quieres un "cancelar ahora" a nivel de UX que revoque el acceso a
la app de inmediato mientras dejas que Paddle liquide la facturación
en segundo plano, controla el acceso con tu propio flag
`subscription.status != Canceled && subscription.cancel_at_period_end == false`
y actualiza la UI justo después de que `cancel()` devuelva - el
siguiente webhook lo confirmará.

### Eliminar un cliente es "archivar vía update"

`delete_customer` devuelve `PaymentError::NotSupported` porque la API
pública de Paddle no expone ningún endpoint de borrado en absoluto. Si
necesitas suprimir un registro de cliente en Paddle, llama a
`update_customer` con el estado `archived`. El adaptador del framework
no envuelve esto directamente - el campo de metadatos es la vía de
escape:

```rust
provider.update_customer(UpdateCustomerRequest {
    provider_customer_id: customer_id,
    email: None,
    name: None,
    metadata: Some(serde_json::json!({ "status": "archived" })),
}).await?;
```

Confirma la ruta exacta del campo contra tu versión de la API de
Paddle antes de distribuir esto - el SDK todavía no modela el enum
`status` directamente.

## Verificación de firma de webhook

Paddle firma cada webhook con HMAC. El encabezado `Paddle-Signature`
luce como `ts=1716000000,h1=abcdef…`. El adaptador delega la
verificación a `Paddle::unmarshal` del SDK, que:

- Analiza el encabezado
- Vuelve a calcular el HMAC usando tu `PADDLE_WEBHOOK_KEY`
- Rechaza las firmas cuya marca de tiempo esté fuera de
  `MaximumVariance::default()` (5 segundos al momento de escribir
  esto - las repeticiones más antiguas que eso se descartan)

El handler `webhook_routes` del framework llama a `verify` antes de
hacer cualquier otra cosa; un fallo devuelve `401 invalid-signature`
sin fuga de cuerpo. Tú no escribes nada de este código, pero vale la
pena saber que la verificación es HMAC + tolerancia de marca de
tiempo, no una comparación de secreto estático.

## Forma del payload de webhook

Los métodos `extract_payload_ids`, `extract_payment_snapshot`, y
`extract_customer_snapshot` del adaptador conocen la forma del payload
de Paddle para que el framework pueda hidratar las tablas de copia
local. Mapeo rápido:

| `event_type` del webhook | `NeutralEventKind` | Efecto en la copia local |
|---|---|---|
| `transaction.completed`, `transaction.paid` | `PaymentSucceeded` | Upsert de `payments_transactions` |
| `transaction.payment_failed` | `PaymentFailed` | Upsert de `payments_transactions` (fallido) |
| `transaction.billed` | `InvoicePaid` | Upsert de `payments_transactions` con `provider_subscription_id` enlazado |
| `adjustment.created`, `adjustment.updated` | `PaymentRefunded` | Upsert de `payments_transactions` (reembolsado) |
| `subscription.created` | `SubscriptionCreated` | `Subscription::get` → upsert de `payments_subscriptions` + partidas |
| `subscription.updated`, `.activated`, `.paused`, `.resumed`, `.trialing` | `SubscriptionUpdated` | Igual que arriba |
| `subscription.canceled` | `SubscriptionCanceled` | Igual; fija `canceled_at`, cambia el estado |
| `customer.created` | `CustomerCreated` | Solo actualización: refresca `email`/`metadata` si la fila de copia local existe |
| `customer.updated` | `CustomerUpdated` | Igual |
| cualquier otra cosa | `None` (sin asignar) | Solo fila de auditoría - sin cambio en la copia local |

Paddle pone el objeto de entidad directamente bajo `data` (no bajo
`data.object` como Stripe). Los importes llegan como **cadenas de
unidades menores** (`"1234"` = 12.34 en la unidad mayor), no como
decimales - el adaptador analiza tanto la forma de cadena como la
numérica, por compatibilidad futura. La moneda llega como
`currency_code`, en minúsculas, y la instantánea la pone en
mayúsculas.

### Importes con impuesto incluido

Paddle reporta los importes de transacción **con impuesto incluido**.
La copia local `payments_transactions` del framework divide esto:

- `amount_total_minor` - el importe total que pagó el cliente
  (impuesto incluido)
- `amount_tax_minor` - el componente de impuesto

El neto de impuesto es `amount_total_minor - amount_tax_minor`. Esto
difiere de Stripe (que reporta sin impuesto, con
`amount_tax_minor = 0`). El código que suma ingresos entre ambos
proveedores necesita ser consciente del impuesto:

```rust
let net_revenue_minor = txn.amount_total_minor - txn.amount_tax_minor;
```

## Creación de clientes

`CreateCustomerRequest` mapea directamente al `customer_create` de
Paddle:

```rust
let cus = provider.create_customer(CreateCustomerRequest {
    user_id: "user_42".into(),       // el id de usuario de tu app
    email: "alice@example.com".into(),
    name: Some("Alice".into()),
    metadata: None,                  // no se reenvía a Paddle en v1
}).await?;
// cus.provider_customer_id == "ctm_01h..."
```

Guarda `cus.provider_customer_id` junto a tu registro de usuario. Toda
llamada posterior (iniciar un checkout, buscar una suscripción, etc.)
toma el ID de cliente de Paddle, no el ID de usuario de la app. La
tabla de copia local `payments_customers` lleva ambas columnas, así
que una sola búsqueda por índice te da cualquiera de las dos
direcciones.

`update_customer` y `get_customer` pasan directamente a los métodos
equivalentes del SDK. `update_customer` acepta actualizaciones de
`email` / `name` y devuelve el `CustomerRef` refrescado.
`get_customer` obtiene una instantánea desde Paddle (no desde la copia
local) - usa esto cuando necesites una lectura fresca después de un
cambio fuera de banda en el panel de Paddle.

## La forma intencional de `NotSupported`

Un lector que no conozca el código base podría asumir que
`PaymentError::NotSupported` en `subscribe()` y `delete_customer()` es
un TODO aplazado. No lo es. Las restricciones son parte de la
superficie de producto de Paddle, y Suprnova las codifica en lugar de
emular mutaciones locales que el proveedor nunca honrará.

Cada mensaje de error `NotSupported` señala hacia el flujo de trabajo
soportado:

- `subscribe`: "use `Checkout::start_session` with `SessionMode::Subscription`
  and await the `SubscriptionCreated` webhook"
- `update` con `new_price_refs`: "Paddle price-set replacement on existing
  subscription not in v1"
- `delete_customer`: "use `UpdateCustomer` with `archived` status"

Bifurca explícitamente sobre este error cuando escribas código de
dominio agnóstico al proveedor:

```rust
match provider.delete_customer(&cus_id).await {
    Ok(()) => { /* camino de Stripe */ }
    Err(PaymentError::NotSupported(_)) => {
        // Camino de Paddle - archiva vía update en su lugar
        provider.update_customer(UpdateCustomerRequest {
            provider_customer_id: cus_id,
            email: None,
            name: None,
            metadata: Some(serde_json::json!({ "status": "archived" })),
        }).await?;
    }
    Err(e) => return Err(e),
}
```

### Por qué Suprnova diverge

Laravel Cashier es exclusivo de Stripe y modela las suscripciones como
propiedad de la app: `$user->newSubscription('default', 'pri_pro')->create()`
tiene la forma de si la aplicación estuviera iniciando la suscripción.
Con una pasarela de captura directa eso es preciso. Con un MoR, es una
mentira - el proveedor es el actor, no tu app.

La superficie de pagos de Suprnova es neutral respecto al proveedor,
así que no toma partido. La superficie de traits (`subscribe`,
`update`, `cancel`, `get`) es la forma genérica; cada adaptador
implementa lo que su proveedor exponga y devuelve `NotSupported`
donde el modelo de producto del proveedor difiera. El adaptador de
Stripe implementa `subscribe`. El adaptador de Paddle no, porque
Paddle no lo permite. Ocultar la diferencia detrás de un "create"
local falso haría que el adaptador te mintiera - Suprnova prefiere el
`NotSupported` tipado, con un mensaje de migración en la cadena del
error.

La misma divergencia aplica a `Payment` (captura del lado del
servidor). Stripe la implementa; Paddle no, y `provider.as_payment()`
devuelve `None`. El código que necesite charge/capture/refund debe
comprobar `as_payment().is_some()` en lugar de llamar a ciegas - ver
[Pagos](payments.md#payment--optional-server-side-capture).

## Prueba tu integración

El crate incluye pruebas de invariantes siempre activas (sin necesidad
de acceso a la red) más una prueba de integración controlada por
variables de entorno contra la API de sandbox de Paddle:

```bash
# Invariantes siempre activas (rechazo de firma, formas de NotSupported):
cargo test -p suprnova-payments-paddle

# Además, integración con el sandbox (requiere PADDLE_API_KEY, etc.):
PADDLE_API_KEY=pdl_sdbx_apikey_... \
PADDLE_WEBHOOK_KEY=pdl_ntfset_... \
PADDLE_CLIENT_TOKEN=test_... \
PADDLE_ENVIRONMENT=sandbox \
  cargo test -p suprnova-payments-paddle
```

Las pruebas de invariantes son las que debes replicar en tu propio
código si construyes abstracciones específicas del adaptador. Tres
formas de prueba que vale la pena copiar:

```rust
use suprnova::payments::*;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

#[test]
fn paddle_does_not_implement_payment_trait() {
    let p = PaddleProvider::new(
        "pdl_sdbx_apikey_test",
        "pdl_ntfset_test",
        "test_client",
        PaddleEnvironment::Sandbox,
    ).expect("provider construction");
    assert!(p.as_payment().is_none());
}

#[tokio::test]
async fn paddle_subscribe_returns_not_supported() {
    let p = /* ...como arriba... */;
    let err = p.subscribe(SubscribeRequest {
        customer_ref: "ctm_test".into(),
        price_refs: vec!["pri_test".into()],
        trial_days: None,
        idempotency_key: None,
        metadata: None,
    }).await.unwrap_err();
    assert!(matches!(err, PaymentError::NotSupported(_)));
}

#[test]
fn webhook_verify_rejects_bad_signature() {
    let p = /* ...como arriba... */;
    let mut headers = http::HeaderMap::new();
    headers.insert("paddle-signature", "ts=1234,h1=deadbeef".parse().unwrap());
    let ctx = WebhookContext {
        body: b"{}",
        headers: &headers,
        remote_addr: None,
    };
    assert!(matches!(p.verify(&ctx).unwrap_err(), PaymentError::WebhookSignature(_)));
}
```

Para pruebas locales de punta a punta sin llamar a Paddle en
absoluto, el framework distribuye `MockPaymentProvider`. Igual que
Paddle, el `as_payment()` del mock devuelve `None` (sin captura del
lado del servidor), así que el código que bifurca según
`as_payment().is_some()` sigue el mismo camino bajo el mock que bajo
Paddle. El `subscribe()` del mock devuelve `Ok` (a diferencia de
Paddle), así que las pruebas que necesiten afirmar la rama
`NotSupported` deben usar el `PaddleProvider` real. Vincula el mock en
las pruebas en lugar del proveedor real:

```rust
use std::sync::Arc;
use suprnova::payments::{MockPaymentProvider, PaymentProviderRegistry};

#[suprnova_test]
async fn checkout_flow() {
    PaymentProviderRegistry::bind("paddle", Arc::new(MockPaymentProvider::new()));
    // ...ejercita tu controlador contra el mock...
}
```

## Lista de verificación para producción

Antes de cambiar a `PADDLE_ENVIRONMENT=production`:

- [ ] Las cuatro variables de entorno están fijadas en los secretos de
  producción, no en el commit
- [ ] La URL del endpoint de webhook está registrada en la
  configuración *Notifications* del panel de Paddle, y el secreto de
  destino que generaste ahí coincide con `PADDLE_WEBHOOK_KEY`
- [ ] El catálogo tiene IDs de precio en vivo (no de sandbox), y los
  IDs que referencias en `price_refs` existen en el catálogo en vivo
- [ ] Tu `success_return_url` y `cancel_return_url` apuntan a
  endpoints HTTPS (Paddle rechaza HTTP en producción)
- [ ] Decidiste cómo responde tu app cuando `subscribe()`,
  `delete_customer()`, o `update(price_refs)` devuelven
  `NotSupported` - ya sea bifurcando en el código o documentando que
  esos flujos son solo-MoR
- [ ] Probaste a fondo la UX de cancelación: la cancelación siempre
  queda programada, así que "cancelaste pero mantienes el acceso
  hasta FECHA" es el mensaje que tu UI debería mostrar
- [ ] Probaste a fondo el webhook de llegada de la suscripción: hay
  una ventana en la que el cliente ya pagó pero la copia local
  todavía no tiene fila
- [ ] Estás agregando los ingresos correctamente: los importes de
  Paddle incluyen impuesto, los de Stripe no

## Siguiente

- [Pagos](payments.md) - la superficie genérica de cinco traits y el
  contrato de hidratación de copia local del handler de webhook
- [Pagos - Integración de Frontend](payments-frontend.md) - checkout
  incrustado de paddle.js en Svelte / React / Vue
- [Pagos - Guía del proveedor](payments-provider-guide.md) - escribe
  tu propio crate adaptador de principio a fin
- [Configuración](configuration.md) - el registro de configuración
  tipada al que se conectan las variables de entorno de Paddle
- [Arranque de la aplicación](bootstrap.md) - donde vive realmente
  `PaymentProviderRegistry::bind` en tu app

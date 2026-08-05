# Pagos - Adaptador de Stripe

`suprnova-payments-stripe` es el adaptador de referencia para la
superficie de pagos neutral respecto al proveedor de Suprnova.
Implementa los cinco traits de pago (`Checkout`, `Payment`,
`Subscription`, `CustomerStore`, `WebhookHandler`) contra la API de
Stripe vía `async-stripe` 1.0.0-rc.5. Recurre a este capítulo cuando
necesites saber exactamente qué endpoint de Stripe llama un método,
cómo se verifica el formato de firma del webhook, cómo fluyen los
PaymentIntents a través de `ChargeResult`, o qué tipos de evento se
mapean al enum de evento neutral.

Para las formas de los traits en sí, la configuración de variables de
entorno, y el patrón de bootstrap, lee primero [Pagos](payments.md).
Este capítulo es la inmersión profunda específica de Stripe.

## Pasarela, no Merchant of Record

Stripe es, por defecto, una **pasarela de pago**: recibes los fondos
directamente en tu propia cuenta bancaria, y eres responsable de
recaudar y liquidar el impuesto, de facturar, de los reintentos de
cobro, y del manejo de contracargos. Contrasta con Paddle
([Pagos - Paddle](payments-paddle.md)), donde Paddle es el Merchant of
Record - ellos recaudan los fondos, declaran el impuesto, y te pagan
neto de comisiones.

La consecuencia práctica para este capítulo: `StripeProvider`
implementa `Payment` (puedes autorizar, capturar, reembolsar, y anular
una tarjeta en el servidor). `PaddleProvider` no. La división de traits
existe porque los dos flujos son genuinamente distintos - no porque se
nos acabara el tiempo.

### Stripe Managed Payments (opt-in Merchant of Record)

El programa **Managed Payments** de Stripe mueve a Stripe hacia el
papel de Merchant of Record para las transacciones elegibles - Stripe
se convierte en el vendedor legal, calcula, recauda, declara y liquida
el impuesto sobre ventas/IVA/GST, y asume las disputas. El programa
tiene restricciones de integración estrictas:

- **Solo Checkout alojado.** Las sesiones deben correr en la página
  alojada de Stripe. Los flujos con Elements o personalizados quedan
  excluidos - por eso el camino alojado de pago puntual del adaptador
  (más abajo) es la única forma de `OneOff` que compone con esto.
- **Precios predefinidos con códigos de impuesto elegibles.** Las
  partidas deben referenciar objetos `price_…` cuyos productos lleven
  un código de impuesto marcado como elegible para Managed Payments en
  el panel de Stripe. Los importes ad-hoc se rechazan.
- **Inscripción de la cuenta.** La cuenta de Stripe debe estar
  incorporada al programa; las sesiones que lleven el flag en una
  cuenta no inscrita fallan.

Actívalo por proveedor con `.with_managed_payments(true)` o
`STRIPE_MANAGED_PAYMENTS=true` - el adaptador entonces envía
`managed_payments[enabled]=true` al crear sesiones alojadas de pago
puntual. Cuando está apagado (por defecto) el campo se omite por
completo.

### Por qué Suprnova diverge

Laravel distribuye Cashier como una integración de Stripe de primera
parte en su documentación principal. Es cómodo, pero solo para Stripe -
y añadir un segundo proveedor implica bifurcar Cashier o construir una
superficie paralela.

Suprnova mantiene a Stripe a distancia. El adaptador de Stripe es un
solo crate que se registra contra los mismos cinco traits que
implementa cualquier otro proveedor. Tu código de dominio nunca nombra
`StripeProvider`; llama a `provider.charge(...)` contra un
`Arc<dyn PaymentProvider>` resuelto desde el registro, y el
comportamiento de Stripe está a un solo intercambio del comportamiento
de Paddle. Cuando más adelante añadas Mollie, o conectes una pasarela
regional que todavía no existe, implementas los mismos cinco traits y
el resto de tu app no se mueve.

## Construcción

```rust
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// Producción: lee desde el entorno.
let stripe = StripeProvider::from_env()
    .expect("STRIPE_SECRET_KEY / PUBLISHABLE_KEY / WEBHOOK_SIGNING_SECRET");

// Pruebas / configuración explícita:
let stripe = StripeProvider::new(
    "sk_test_...",
    "pk_test_...",
    "whsec_...",
);

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

`StripeProvider` es `Clone` (barato - el `stripe::Client` subyacente
está respaldado por `Arc`) y guarda estos valores:

| Campo | Origen | Uso |
|---|---|---|
| `secret_key` | `sk_live_…` / `sk_test_…` | `Authorization: Bearer …` HTTP en cada llamada a la API |
| `publishable_key` | `pk_live_…` / `pk_test_…` | Se hace emerger dentro de `SessionPayload::StripeElements` para que el frontend pueda montar Stripe.js sin una búsqueda de configuración separada |
| `webhook_signing_secret` | `whsec_…` | Verificación HMAC-SHA256 del encabezado `Stripe-Signature` |
| `managed_payments` | `STRIPE_MANAGED_PAYMENTS` (`true`/`1`) o `.with_managed_payments(bool)` | Envía `managed_payments[enabled]=true` al crear una sesión alojada de pago puntual (ver [Managed Payments](#stripe-managed-payments-opt-in-merchant-of-record)) |

`from_env()` devuelve `Result<Self, String>` - el mensaje de error
nombra la variable requerida que falta (`STRIPE_MANAGED_PAYMENTS` es
opcional; su ausencia significa apagado). No hay ninguna vía de pánico
al arrancar.

## Sesiones de checkout

`Checkout::start_session` elige su superficie de Stripe según la
solicitud:

| Forma de la solicitud | Objeto de Stripe | Variante de `SessionPayload` |
|---|---|---|
| `OneOff` + `price_refs` no vacío | Sesión de Checkout alojada, `mode=payment` | `StripeCheckoutRedirect { url, provider_session_id: "cs_…" }` |
| `OneOff` + `price_refs` vacío + `amount_hint` | PaymentIntent | `StripeElements { client_secret, publishable_key, provider_session_id: "pi_…" }` |
| `Subscription` + `price_refs` | Sesión de Checkout alojada, `mode=subscription` | `StripeCheckoutRedirect` |

El camino alojado de pago puntual envía `allow_promotion_codes=true`
(los clientes pueden introducir códigos de promoción en la página de
Stripe - combínalo con el trait `Promotions` más abajo) y, cuando el
proveedor está configurado para ello, el flag de Managed Payments. Pon
el literal de plantilla `{CHECKOUT_SESSION_ID}` de Stripe en tu
`success_return_url` - Stripe sustituye el id real `cs_…` en la
redirección, y tu página de retorno lo alimenta a `session_status`.

`Checkout::session_status` mapea `GET /v1/checkout/sessions/{id}` al
`CheckoutSessionState` neutral:

| `status` / `payment_status` de Stripe | `CheckoutSessionState` |
|---|---|
| `open` | `Open` |
| `expired` | `Expired` |
| `complete` + `paid` o `no_payment_required` | `Complete { paid: true, payment_ref, amount_total }` |
| `complete` + `unpaid` (liquidación retrasada) | `Complete { paid: false, … }` |

`payment_ref` lleva el id de PaymentIntent de la sesión (`pi_…`) para
que las páginas de retorno y los barridos puedan correlacionar la
sesión con las operaciones de `Payment` y con la copia local de
`payments_transactions`. `amount_total` es el total ya liquidado, con
los descuentos del lado del proveedor y el impuesto de Managed Payments
ya incorporados.

## Códigos de promoción

`StripeProvider` implementa el trait opcional `Promotions`
(`provider.as_promotions()` devuelve `Some`). `create_promotion_code`
mapea a `POST /v1/promotion_codes`: acuña un código a partir de un
cupón ya creado (`coupon_ref`), restringido a un cliente
(`customer_ref`), con una caducidad y un tope de canjes opcionales. Las
restricciones las hace cumplir Stripe en el momento del canje - un
código acuñado para el cliente A se rechaza cuando el cliente B lo
escribe, los códigos caducados se rechazan, y `max_redemptions:
Some(1)` hace que el código sea de un solo uso. Ver la sección
`Promotions` de [Pagos](payments.md) para el patrón de campaña.

## El ciclo de vida de PaymentIntent

Stripe representa un solo intento de cargo como un **PaymentIntent**.
El intent avanza a través de estados; el trait `Payment` de Suprnova
impulsa las transiciones. Todo método `Payment` de `StripeProvider`
mapea a un endpoint `/v1/payment_intents/...`:

| Método de `Payment` | Endpoint de Stripe | Qué hace |
|---|---|---|
| `charge` | `POST /v1/payment_intents` | Crea y confirma en una sola llamada contra un método de pago guardado. `capture_method: "manual"` para que el intent pase a `requires_capture`, **no** a `succeeded`. |
| `capture` | `POST /v1/payment_intents/{id}/capture` | Liquida un intent previamente autorizado. Estado `requires_capture` → `succeeded`. |
| `refund` | `POST /v1/refunds` | Revierte, total o parcialmente, un intent ya capturado. |
| `void` | `POST /v1/payment_intents/{id}/cancel` | Libera una autorización antes de la captura. Estado `requires_capture` → `canceled`. |
| `status` | `GET /v1/payment_intents/{id}` | Recupera el estado actual (devuelve `PaymentStatus`). |

### Autorizar primero, capturar después

`StripeProvider::charge` **no** liquida los fondos de inmediato. Envía
`capture_method=manual` + `confirm=true`, lo cual autoriza la tarjeta y
reserva los fondos, y luego espera una llamada explícita a `capture`.
Este es el flujo canónico de dos pasos:

```rust
use suprnova::payments::{
    PaymentProviderRegistry, ChargeRequest, ChargeResult,
    Money, Currency, PaymentStatus,
};

let provider = PaymentProviderRegistry::get("stripe").unwrap();
let payment = provider.as_payment()
    .expect("Stripe implements Payment");

let result = payment.charge(ChargeRequest {
    customer_ref: "cus_NffrFeUfNV2Hib".into(),
    payment_method_ref: "pm_card_visa".into(),
    amount: Money::from_minor_units(2999, Currency::USD),
    description: Some("Pro plan, manual capture".into()),
    idempotency_key: Some("order-12345".into()),  // ver "Idempotencia" más abajo
    metadata: None,
}).await?;

match result {
    ChargeResult::Completed { provider_transaction_id, status, .. }
        if status == PaymentStatus::Pending => {
        // Autorizado - liquida cuando el pedido se despache.
        let settled = payment.capture(&provider_transaction_id).await?;
        assert!(matches!(
            settled,
            ChargeResult::Completed { status: PaymentStatus::Succeeded, .. }
        ));
    }
    ChargeResult::RequiresClientAction { client_secret, .. } => {
        // Se necesita el step-up 3DS - ver "3DS y SCA" más abajo.
    }
    other => panic!("unexpected charge result: {other:?}"),
}
```

Si quieres una captura **inmediata** - la compra puntual habitual del
e-commerce - usa `Checkout::start_session` con `SessionMode::OneOff` en
su lugar. Ese camino crea un PaymentIntent con
`automatic_payment_methods` activado y le entrega el client secret al
frontend para que el navegador del cliente confirme el intent en el
sitio. `Payment::charge` es para flujos impulsados por el servidor
donde ya tienes el método de pago guardado del cliente y quieres
control explícito de autorizar-y-luego-capturar (típico para
marketplaces, SaaS de cumplimiento retrasado, o comercio de envío
dividido).

### Mapeo de estados

Los estados de Stripe se pliegan dentro del enum `PaymentStatus` de
Suprnova:

| `PaymentIntentStatus` | `PaymentStatus` |
|---|---|
| `Succeeded` | `Succeeded` |
| `Processing` | `Pending` |
| `RequiresCapture` | `Pending` (autorizado, en espera de captura) |
| `RequiresAction` | `Pending` (devuelto como `RequiresClientAction` desde `charge`) |
| `RequiresConfirmation` | `Pending` |
| `RequiresPaymentMethod` | `Pending` |
| `Canceled` | `Canceled` |
| _nuevo estado de Stripe (el enum es `#[non_exhaustive]`)_ | `Failed` |

El resultado de reserva de `non_exhaustive` es intencional. Stripe
añade estados de vez en cuando (p. ej. al introducir nuevos tipos de
método de pago). Hacerlos emerger como `Failed` es la opción por
defecto conservadora - tu app trata el cargo como aún-no-confirmado
hasta que actualices el adaptador.

### 3DS y SCA

La Autenticación Reforzada del Cliente europea, las reglas del RBI de
India, y varios otros reguladores exigen que el titular de la tarjeta
autentique el cargo en un contexto de navegador separado. Stripe lo
hace emerger como `requires_action` con un bloque `next_action`.

`StripeProvider::charge` traduce esto en una de dos variantes de
`ChargeResult`:

```rust
ChargeResult::RequiresClientAction {
    provider_transaction_id,   // pi_xxx - conserva esto
    action_kind: "stripe_3ds", // etiqueta específica de Stripe
    client_secret,             // entrégalo a Stripe.js
    publishable_key,           // entrégalo a Stripe.js
}
```

Cuando el `next_action` del intent contiene una URL de redirección
(algunos flujos de autenticación son de redirección por URL en lugar
de un modal en el sitio), el resultado se reescribe como:

```rust
ChargeResult::RedirectRequired {
    provider_transaction_id,
    url,                       // redirige el navegador aquí
    return_to: None,
}
```

Tu controlador entrega el payload de `RequiresClientAction` a la
página de Inertia; el frontend llama a
`stripe.confirmCardPayment(client_secret, ...)` y el cliente completa
el 3DS. Cuando la confirmación tiene éxito, Stripe dispara
`payment_intent.succeeded` y la ruta de webhook escribe la fila de
copia local. Consulta
[Pagos - Integración de Frontend](payments-frontend.md) para los
fragmentos de Svelte / React / Vue.

### Anular vs reembolsar

`void` libera una autorización **antes** de la captura; `refund`
revierte un pago ya capturado. Llamar a `void` sobre un intent ya
capturado fallará - Stripe rechaza con un mensaje que contiene
`"already succeeded"` o `"You cannot cancel"`, y el adaptador lo hace
emerger como `PaymentError::Validation` para que tu handler pueda
distinguir un error de usuario recuperable (usa `refund` en su lugar)
de una caída real del proveedor. Cualquier otro fallo es
`PaymentError::Provider`.

```rust
let voided = payment.void("pi_3PNzj...").await;
match voided {
    Ok(()) => { /* autorización liberada */ }
    Err(suprnova::payments::PaymentError::Validation(msg)) => {
        // Ya capturado - llama a refund en su lugar.
        let refund = payment.refund(RefundRequest {
            provider_transaction_id: "pi_3PNzj...".into(),
            amount: None,           // reembolso total
            reason: Some("requested_by_customer".into()),
            idempotency_key: None,  // refund() no reenvía esto - ver "Idempotencia"
        }).await?;
    }
    Err(e) => return Err(e.into()),
}
```

## Clientes

`StripeProvider` implementa `CustomerStore` contra `/v1/customers`. El
adaptador mapea un `Customer` devuelto al `CustomerRef` neutral,
preservando el email y el `user_id` de tu aplicación:

```rust
use suprnova::payments::CreateCustomerRequest;

let customer = provider.create_customer(CreateCustomerRequest {
    user_id: "user-42".into(),       // el id de usuario de tu app
    email: "alice@example.com".into(),
    name: Some("Alice Example".into()),
    metadata: None,
}).await?;

// customer.provider_customer_id == "cus_NffrFeUfNV2Hib"
// Guarda esto junto a tu fila de User para que los cargos,
// las suscripciones, y los webhooks posteriores resuelvan de vuelta.
```

`update_customer`, `get_customer`, y `delete_customer` llaman a
`POST /v1/customers/{id}`, `GET /v1/customers/{id}`, y
`DELETE /v1/customers/{id}` respectivamente. El delete de Stripe
devuelve una envoltura `DeletedCustomer` que el adaptador descarta -
solo se propaga el éxito/fallo de la llamada.

## Suscripciones

`StripeProvider::subscribe` hace POST a `/v1/subscriptions` con la
referencia del cliente, un array `items[]`, y un `trial_period_days`
opcional:

```rust
use suprnova::payments::{SubscribeRequest, SubscriptionStatus};

let sub = provider.subscribe(SubscribeRequest {
    customer_ref: "cus_NffrFeUfNV2Hib".into(),
    price_refs: vec!["price_pro_monthly".into()],
    trial_days: Some(14),
    idempotency_key: None,
    metadata: None,
}).await?;

assert!(matches!(
    sub.status,
    SubscriptionStatus::Trialing | SubscriptionStatus::Active
));

println!("Period ends at {}", sub.current_period_end);
for item in &sub.items {
    println!(
        "  {} × {} @ {:?}",
        item.quantity, item.provider_price_id, item.unit_amount,
    );
}
```

### Límites del periodo

Stripe movió las marcas de tiempo `current_period_start` /
`current_period_end` desde la Subscription padre hacia cada
`SubscriptionItem`, en la versión de API `2023-08-16`. Las
suscripciones con varias partidas pueden, en teoría, tener periodos de
partida divergentes, pero en la práctica toda partida de una misma
suscripción comparte el ciclo de facturación del padre. El adaptador
toma el periodo de la **primera partida** como el periodo padre en el
`SubscriptionResult` devuelto. Si genuinamente necesitas periodos por
partida, léelos desde `sub.items[n]` - se conservan en la instantánea.

### Cancelar al final del periodo vs de inmediato

```rust
// Cancelación blanda - mantiene el acceso hasta current_period_end:
let sub = provider.cancel("sub_1234", /* at_period_end */ true).await?;
// sub.cancel_at_period_end == true
// sub.status == Active

// Cancelación inmediata - Stripe DELETE /v1/subscriptions/{id}:
let sub = provider.cancel("sub_1234", /* at_period_end */ false).await?;
// sub.status == Canceled
```

Los dos caminos llaman a distintos endpoints de Stripe. La cancelación
blanda es `POST /v1/subscriptions/{id}` con `cancel_at_period_end=true` -
la suscripción sigue activa hasta el final del periodo de facturación,
y entonces Stripe la finaliza. La cancelación inmediata es
`DELETE /v1/subscriptions/{id}` con `prorate=false` e
`invoice_now=false`.

### `update()` está limitado a propósito

`UpdateSubscriptionRequest` tiene dos campos sobre los que actúa el
adaptador: `cancel_at_period_end` y `new_price_refs`. El primero está
soportado; el segundo devuelve `PaymentError::NotSupported`:

```rust
provider.update(UpdateSubscriptionRequest {
    provider_subscription_id: "sub_1234".into(),
    new_price_refs: Some(vec!["price_team_yearly".into()]),
    cancel_at_period_end: None,
    idempotency_key: None,
}).await
// → Err(PaymentError::NotSupported(
//      "Stripe price-set replacement on existing subscription not in v1. \
//       Cancel the subscription and create a new one with the new price set."
//   ))
```

Este es uno de los pocos lugares donde `NotSupported` es la respuesta
honesta en lugar de un aplazamiento. El reemplazo del conjunto de
precios en Stripe exige eliminar y volver a crear las partidas de la
suscripción - la forma varía según el proveedor (prorrateo, anclaje del
ciclo de facturación, comportamiento de periodo de prueba retenido) y
comprimir eso en una única API neutral ocultaría más de lo que
ayudaría. El camino recomendado es cancelar la suscripción existente y
volver a `subscribe` con el nuevo conjunto de precios, aplicando tu
propia política de prorrateo si necesitas una.

## Webhooks

Stripe envía los webhooks firmados con HMAC-SHA256 en el formato:

```
Stripe-Signature: t=1717000000,v1=5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd
```

`StripeProvider::verify` analiza el encabezado, vuelve a calcular
HMAC-SHA256 sobre `"{timestamp}.{raw_body}"` usando el secreto de firma
del webhook, y hace una comparación en **tiempo constante** contra cada
valor `v1=` del encabezado. Existen varios valores `v1=` durante la
rotación del secreto de firma - Stripe superpone el secreto antiguo y
el nuevo durante una ventana para que puedas volver a firmar y
desplegar sin una transición abrupta coordinada.

```
Stripe-Signature: t=1717000000,v1=<old_sig>,v1=<new_sig>
```

El adaptador acepta la solicitud si **cualquier** valor `v1=` coincide.
Un encabezado al que le falte `t=` o que no tenga ningún valor `v1=` se
rechaza como `PaymentError::WebhookSignature`. Los bytes no-ASCII en
cualquier parte del encabezado también se rechazan - Stripe nunca los
envía, y tratarlos como inválidos es más seguro que sustituirlos por un
carácter de reemplazo.

Nunca llamas a `verify` directamente. El `webhook_routes(db.clone())`
del framework registra `POST /webhooks/payments/{provider}` e invoca
`verify` + `parse_event` + los extractores de payload del adaptador en
cada solicitud que llega ahí. Consulta [Idempotencia](idempotency.md)
para el comportamiento de auditoría consciente de reintentos - incluida
la regla de que los eventos previamente fallidos vuelven a intentar la
hidratación cuando el proveedor reintenta.

### Mapeo de eventos → neutral

Los tipos de evento de Stripe se mapean al `NeutralEventKind` de
Suprnova mediante la función `stripe_event_to_neutral`. La tabla de
mapeo:

| Tipo de evento de Stripe | `NeutralEventKind` |
|---|---|
| `payment_intent.succeeded` | `PaymentSucceeded` |
| `payment_intent.payment_failed` | `PaymentFailed` |
| `charge.refunded` | `PaymentRefunded` |
| `charge.dispute.created` | `PaymentDisputed` |
| `customer.subscription.created` | `SubscriptionCreated` |
| `customer.subscription.updated` | `SubscriptionUpdated` |
| `customer.subscription.deleted` | `SubscriptionCanceled` |
| `customer.subscription.paused` | `SubscriptionUpdated` |
| `customer.subscription.resumed` | `SubscriptionUpdated` |
| `customer.subscription.trial_will_end` | `SubscriptionUpdated` |
| `invoice.payment_succeeded` / `invoice.paid` | `InvoicePaid` |
| `invoice.payment_failed` | `InvoiceFailed` |
| `customer.created` | `CustomerCreated` |
| `customer.updated` | `CustomerUpdated` |
| _cualquier otra cosa_ | `None` |

Los eventos que se mapean a `None` (señales de fraude de Radar,
transferencias, movimientos de saldo, eventos del ciclo de vida de una
disputa posteriores a `created`) de todos modos se persisten en la
tabla de auditoría `payments_webhook_events` - simplemente no impulsan
las tablas de copia local. Si los necesitas, léelos directamente desde
`event.raw_payload` en un handler propio.

El mapeo también se reexporta en la raíz del crate para que puedas
usarlo fuera de la ruta de webhook:

```rust
use suprnova_payments_stripe::stripe_event_to_neutral;
use suprnova::payments::NeutralEventKind;

assert_eq!(
    stripe_event_to_neutral("payment_intent.succeeded"),
    Some(NeutralEventKind::PaymentSucceeded),
);
assert_eq!(
    stripe_event_to_neutral("radar.early_fraud_warning.created"),
    None,
);
```

### Extracción de payload

Después de que `verify` y `parse_event` tengan éxito, el framework
llama a `extract_payload_ids`, `extract_payment_snapshot`, y
`extract_customer_snapshot` para extraer los campos que impulsan las
tablas de copia local (ver [Eloquent](eloquent.md) para el patrón
subyacente de leer-desde-tu-propia-BD). Stripe es estructuralmente
consistente: todo webhook coloca la entidad relevante en
`data.object`, con `id` como su clave primaria.

Los extractores manejan cuatro familias de eventos:

- **Eventos de suscripción** - se extraen `data.object.id` (el id de
  la suscripción) y `data.object.customer`.
- **Eventos de cliente** - se extrae `data.object.id` (el id del
  cliente).
- **Eventos de PaymentIntent / Charge** - se extraen `data.object.id`,
  `data.object.amount`, `data.object.currency`,
  `data.object.customer`, y (solo para `payment_intent.succeeded`)
  `data.object.created` como `paid_at`.
- **Eventos de Invoice** - se extraen `data.object.id`, el puntero al
  cliente, `data.object.subscription` (solo cargos recurrentes),
  `amount_paid` (recurriendo a `amount_due` si falta), `tax`,
  `currency`, y `data.object.status_transitions.paid_at`.

Cualquier otra cosa devuelve `None` desde los extractores de
instantánea; la fila de auditoría de todos modos se registra.

## Tablas de copia local

Seis tablas respaldan la superficie de pagos en la base de datos de tu
aplicación. Aplica la migración del framework junto con las tuyas:

```rust
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... tus migraciones ...
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

Las tablas creadas son `payments_customers`,
`payments_payment_methods`, `payments_subscriptions`,
`payments_subscription_items`, `payments_transactions`, y
`payments_webhook_events`. La ruta de webhook las hidrata dentro de una
sola transacción de BD por evento - el estado parcial nunca es
observable, y la fila de auditoría lleva `process_error` a través de
los reintentos para que los fallos sigan siendo visibles para quienes
operan el sistema.

## Idempotencia

La idempotencia de salida en las llamadas a la API de Stripe y la
idempotencia de entrada en las entregas de webhook son dos historias
separadas. Léelas como tales.

### Salida: cobertura por método

Stripe soporta la idempotencia de solicitudes mediante el encabezado
HTTP de solicitud `Idempotency-Key` - la misma clave con el mismo
cuerpo devuelve el mismo objeto de respuesta durante una ventana de
deduplicación de 24 horas; un cuerpo que no coincida devuelve un error.
El adaptador de Stripe para Suprnova **no** traslada de forma uniforme
el campo `idempotency_key` del DTO a ese encabezado, hoy por hoy. El
comportamiento real al momento de escribir esto:

| Método | Campo del DTO | Qué hace el adaptador |
|---|---|---|
| `Payment::charge` | `ChargeRequest::idempotency_key` | Se reenvía dentro del cuerpo POST como `idempotency_key=...` (no el encabezado HTTP). La API de Stripe **no** lee claves de idempotencia en el cuerpo, así que lo mejor es tratar esto como no efectivo hasta que el adaptador migre a la vía del encabezado de solicitud. |
| `Payment::refund` | `RefundRequest::idempotency_key` | Se descarta en silencio - el campo no se reenvía. |
| `Checkout::start_session` | `StartSessionRequest::idempotency_key` | Se descarta en silencio. |
| `Subscription::subscribe` / `update` | `*Request::idempotency_key` | Se descarta en silencio. |

Si hoy dependes de semántica de a lo sumo una vez para los reintentos
de cargo/reembolso contra Stripe, controla el reintento en tu propio
punto de llamada (una clave de dominio determinística persistida en tu
BD, con un índice único que impida la segunda inserción) hasta que el
adaptador conecte el encabezado. Los campos del DTO se aceptan en la
API pero hoy no se respetan hasta llegar a la petición HTTP real -
fíjalos en `None` en pruebas y en código de producción para que la
brecha sea explícita, y no asumas que Stripe está deduplicando tus
reintentos.

Esta es una brecha conocida en el adaptador v1 y una candidata a
arreglo para la próxima versión; la forma de la superficie se mantiene
igual una vez que la conexión llegue.

### Entrada: deduplicación de webhooks

La idempotencia de webhooks la maneja el framework en el lado de
ingesta y está completamente conectada. Todo evento llega a
`payments_webhook_events` con un índice UNIQUE en
`(provider, provider_event_id)`. Las entregas duplicadas de un evento
ya procesado devuelven 200 a Stripe de inmediato sin volver a ejecutar
la hidratación; los duplicados de un evento previamente **fallido**
vuelven a intentar la hidratación, de modo que el reintento del
proveedor es tu mecanismo de recuperación. Consulta
[Idempotencia](idempotency.md) para el contrato completo de auditoría +
reintento.

## Pruebas

El adaptador está respaldado por hyper y expuesto mediante rustls. Las
pruebas que construyen un `StripeProvider` necesitan un proveedor de
criptografía registrado; instalamos `ring` exactamente una vez en
`#[cfg(test)]`:

```rust
#[cfg(test)]
mod tests {
    use suprnova_payments_stripe::StripeProvider;
    use std::sync::OnceLock;

    fn install_crypto_provider() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    fn provider() -> StripeProvider {
        install_crypto_provider();
        StripeProvider::new("sk_test_dummy", "pk_test_dummy", "whsec_dummy")
    }

    #[test]
    fn parses_subscription_webhook_ids() {
        let p = provider();
        let event = /* construye un WebhookEvent con raw_payload */;
        let ids = p.extract_payload_ids(&event);
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_abc"));
    }
}
```

Para pruebas de integración que llaman al sandbox real de Stripe, fija
`STRIPE_SECRET_KEY` y compañía en tu entorno de pruebas. Para pruebas
unitarias de tus propios controladores, prefiere `MockPaymentProvider`
del framework - implementa los cinco traits con retornos predecibles y
cero red.

## Siguiente

- [Pagos](payments.md) - la superficie de traits, el registro, el
  patrón de bootstrap, y el `SessionPayload` etiquetado por flujo.
- [Pagos - Paddle](payments-paddle.md) - la contraparte de Merchant of
  Record; los mismos cinco traits, un reparto de responsabilidad
  distinto.
- [Pagos - Guía del proveedor](payments-provider-guide.md) - cómo
  escribir un adaptador para una pasarela que Suprnova no distribuye.
- [Pagos - Integración de Frontend](payments-frontend.md) - despacho
  en Svelte / React / Vue según `SessionPayload.flow`, incluido el
  bucle de confirm-card-payment de Stripe.js.
- [Idempotencia](idempotency.md) - el contrato de auditoría + reintento
  que hace seguro el manejo de webhooks bajo entrega al menos una vez.
- [Eloquent](eloquent.md) - consulta las tablas de copia local junto a
  tus propios modelos; todo es solo una entidad de SeaORM.

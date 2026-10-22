# Pagos - NOWPayments

El adaptador `suprnova-payments-nowpayments` crea facturas alojadas, verifica
las notificaciones de pago y lee el estado del pago usando la clave de API del
comercio. Se registra como `nowpayments` en el registro normal de proveedores
de pago.

## Instalación y configuración

Usa una revisión de Suprnova que contenga este adaptador para ambas
dependencias. Para una aplicación local junto a un checkout del código fuente
de Suprnova:

```toml
[dependencies]
suprnova = { path = "../suprnova/framework" }
suprnova-payments-nowpayments = { path = "../suprnova/crates/suprnova-payments-nowpayments" }
serde_json = "1"
```

Configura estos valores en el entorno protegido de la aplicación:

```dotenv
NOWPAYMENTS_ENVIRONMENT=sandbox
NOWPAYMENTS_API_KEY=your-sandbox-api-key
NOWPAYMENTS_IPN_SECRET=your-sandbox-ipn-secret
NOWPAYMENTS_IPN_CALLBACK_URL=https://app.example/webhooks/payments/nowpayments
```

El entorno acepta `sandbox` o `production` y su valor por defecto es
`sandbox`. Un entorno desconocido o vacío hace fallar la configuración. Las
credenciales vacías fallan antes de las peticiones HTTP. Usa credenciales
distintas para cada entorno.

Registra el proveedor durante el arranque, propagando cualquier error de
configuración:

```rust,no_run
use std::sync::Arc;
use suprnova::payments::{PaymentProviderRegistry, PaymentResult};
use suprnova_payments_nowpayments::NowPaymentsProvider;

fn register_nowpayments() -> PaymentResult<Arc<NowPaymentsProvider>> {
    let provider = Arc::new(NowPaymentsProvider::from_env()?);
    PaymentProviderRegistry::bind("nowpayments", provider.clone());
    Ok(provider)
}
```

Añade las migraciones de pagos y compón `webhook_routes(db)` en el router de
la aplicación como se muestra en la
[descripción general de pagos](payments.md). El endpoint es
`POST /webhooks/payments/nowpayments`. Mantén esta ruta autenticada por el
proveedor fuera del middleware CSRF de sesiones de navegador. Configura el
callback con la URL HTTPS pública exacta; los cuerpos de las peticiones deben
llegar al adaptador sin modificar.

## Iniciar una factura alojada

Persiste un intento de checkout con una referencia de pedido del comercio
única antes de hacer la petición. El importe procede del pedido de confianza
de la aplicación, nunca de un total suministrado por el navegador.

```rust,no_run
use suprnova::payments::{Money, SessionMode, StartSessionRequest};
use suprnova_payments_nowpayments::{InvoiceCreationError, NowPaymentsInvoice, NowPaymentsProvider};
use serde_json::json;

async fn create_order_invoice(
    provider: &NowPaymentsProvider,
    order_id: String,
    amount: Money,
) -> Result<NowPaymentsInvoice, InvoiceCreationError> {
    provider.create_invoice(StartSessionRequest {
        mode: SessionMode::OneOff,
        customer_ref: String::new(),
        price_refs: Vec::new(),
        amount_hint: Some(amount),
        success_return_url: "https://app.example/billing/return".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        idempotency_key: None,
        metadata: Some(json!({"order_id": order_id})),
    }).await
}
```

`Checkout::start_session` acepta la misma petición y devuelve el genérico
`SessionPayload::Redirect`. Guarda su `provider_session_id` como **ID de
factura** y después redirige a su URL validada. El método concreto
`create_invoice` también devuelve la referencia de pedido del comercio y
distingue los errores de creación inciertos.

El adaptador de facturas exige el modo de pago único, referencias de cliente y
de precio vacías, y un importe `Money` positivo con un exponente de moneda
fiat definido. Envía el importe decimal exacto en la unidad principal y el
código de moneda en minúsculas. Los clientes eligen una criptomoneda en la
página alojada, salvo que se suministre `pay_currency`.

Los metadatos son un objeto estricto con estos campos:

| Campo | Significado |
| --- | --- |
| `order_id` | Referencia del comercio obligatoria y no vacía, de 128 bytes como máximo |
| `order_description` | Descripción opcional, de 500 bytes como máximo |
| `pay_currency` | Ticker del proveedor opcional en minúsculas, como `btc` |
| `is_fixed_rate` | Ajuste opcional del tipo de cambio del proveedor |
| `is_fee_paid_by_user` | Ajuste opcional de comisiones del proveedor |

Las claves de metadatos desconocidas se rechazan. No pongas datos arbitrarios
de la aplicación o del cliente en este objeto. Las URL de retorno deben usar
el mismo origen HTTPS que el callback, sin credenciales ni fragmentos. Las
redirecciones de factura deben apuntar al entorno de NOWPayments seleccionado
y contener el ID de factura devuelto.

## Fallos de creación y reintentos

El endpoint de facturas de NOWPayments no documenta ninguna garantía de clave
de idempotencia. El adaptador rechaza un `idempotency_key` distinto de `None`;
`order_id` son datos de correlación, no una deduplicación del lado del
proveedor. Ni las redirecciones HTTP ni los reintentos automáticos están
habilitados. Las peticiones tienen un plazo de 30 segundos y un límite de
respuesta de 64 KiB.

`InvoiceCreationError::Rejected` representa una validación de entrada o un
rechazo definitivo de la API. `Unknown` significa que la factura puede existir
ya: por ejemplo, la petición agotó su plazo, el proveedor devolvió un error de
servidor, o su respuesta de éxito no era válida. Marca ese intento como
incierto y concilia el pedido en el panel del proveedor antes de crear otra
factura. No metas la creación de facturas dentro de un bucle de reintentos
genérico. A través de `Checkout::start_session`, un resultado desconocido es
un `PaymentError::Provider` con esta instrucción de recuperación.

## Verificar y conciliar pagos

Una factura puede producir un ID de pago aparte.
`payment_status(payment_id)` llama al endpoint de pago autenticado y rechaza
una respuesta con un ID distinto. `Checkout::session_status(invoice_id)`
devuelve `NotSupported`; un ID de factura no se puede sustituir en el endpoint
de consulta de pagos. Este adaptador no usa las credenciales de correo y
contraseña del panel para listar los pagos por factura.

```rust,no_run
use suprnova::payments::{Money, PaymentResult};
use suprnova_payments_nowpayments::{NowPaymentsProvider, NowPaymentsStatus};

async fn payment_matches_order(
    provider: &NowPaymentsProvider,
    verified_payment_id: &str,
    expected_invoice_id: &str,
    expected_order_id: &str,
    expected_price: Money,
) -> PaymentResult<bool> {
    let payment = provider.payment_status(verified_payment_id).await?;
    Ok(payment.status == NowPaymentsStatus::Finished
        && payment.invoice_id.as_deref() == Some(expected_invoice_id)
        && payment.order_id.as_deref() == Some(expected_order_id)
        && payment.price == expected_price)
}
```

El ID de pago debe venir de un IPN verificado o de otro registro del lado del
servidor en el que confíes. Un retorno del navegador no es prueba de pago. Una
consulta correcta tampoco es una comprobación de autorización: contrástala con
el pedido, la factura, el importe y la moneda almacenados antes de cambiar el
acceso. `price` es el precio fiat solicitado, no la cantidad de cripto
recibida. Revisa los ajustes de aceptación de pagos parciales del comercio;
este adaptador nunca convierte `partially_paid` en un éxito.

| Estado del proveedor | Evento neutral |
| --- | --- |
| `finished` | `PaymentSucceeded` |
| `failed`, `expired`, `cancelled`, `canceled` | `PaymentFailed` |
| `refunded` | `PaymentRefunded` |
| `waiting`, `confirming`, `confirmed`, `sending`, `partially_paid`, desconocido | Sin clasificación neutral |

Las firmas de IPN usan JSON ordenado de forma recursiva y HMAC SHA-512. La
verificación usa una comparación de MAC en tiempo constante. Se rechazan las
firmas ausentes, duplicadas o no válidas, las claves JSON duplicadas, los
payloads de tamaño excesivo y los campos obligatorios mal formados. El
proveedor no firma ninguna marca de tiempo de entrega independiente, así que
no hay una ventana de repetición por marca de tiempo inventada. La protección
contra repeticiones usa recibos persistidos.

Los IPN de NOWPayments no tienen un ID de evento independiente. El adaptador
identifica un recibo por ID de pago y estado, así que una entrega repetida con
un `updated_at` distinto no repite el mismo evento terminal. El framework
guarda los eventos verificados en `payments_webhook_events` y reintenta el
procesamiento fallido. Un evento pendiente que llega tarde no modifica una
copia local de transacción ya liquidada.

### Facturas sin cliente y estado de la aplicación

La API de facturas no aporta la identidad de cliente que exigen las copias
locales de transacciones de Suprnova. Este adaptador devuelve `false` desde
`WebhookHandler::mirrors_payment_transactions`: todos los eventos verificados,
incluidas las devoluciones, quedan en el registro de auditoría, pero no crea
ninguna copia local de cliente ni de transacción. Stripe y Paddle conservan el
comportamiento de copia local por defecto.

La ruta genérica de webhooks registra y procesa los eventos del proveedor; no
cumple los pedidos de la aplicación. Un trabajo de conciliación de la
aplicación debería consumir los recibos verificados, leer el estado de pago
actual y actualizar su pedido en una transacción idempotente. Guarda una clave
de cumplimiento única por proveedor y pago o por pedido, para que los
reintentos y las entregas reordenadas no puedan conceder el acceso dos veces.
Lleva el estado de finalización duradero del propio trabajo aparte del
`processed_at` del framework. Usa el estado autenticado actual para los
eventos que llegan tarde y trata las devoluciones según la política de la
aplicación.

## Límites de capacidad

Las operaciones de `CustomerStore` y `Subscription` devuelven `NotSupported`.
`as_payment()` y `as_promotions()` devuelven `None`. Este adaptador no
implementa saldos en custodia, facturación recurrente, checkout directo por
dirección de depósito, pagos salientes, inicio de devoluciones ni gestión de
clientes del proveedor. NOWPayments ofrece productos aparte para algunas de
estas operaciones; este adaptador de facturas no pretende implementarlas.

### Por qué Suprnova diverge

Los traits comunes de proveedor siguen disponibles, pero las operaciones no
soportadas fallan de forma explícita. Las facturas sin clientes del proveedor
usan notificaciones auditadas y pedidos propiedad de la aplicación en lugar de
registros de cliente o suscripciones inventados.

## Verificación

Ejecuta las pruebas del adaptador y las regresiones de pagos compartidas desde
el checkout del código fuente:

```sh
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova-payments-nowpayments
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova --test payments
```

La suite local usa respuestas HTTP falsas, un fixture de firma en JavaScript
generado de forma independiente, e ingreso real del framework con SQLite.
Cubre el checkout correcto, los fallos de autenticación, las respuestas mal
formadas o grandes, los tiempos de espera agotados, las redirecciones no
confiables, las firmas no válidas, el mapeo de estados, las entregas
duplicadas y la recuperación tras un fallo de base de datos. Los fixtures
locales no establecen interoperabilidad con el proveedor.

Antes de habilitar producción, usa una cuenta de sandbox de NOWPayments y un
callback HTTPS alcanzable. Crea una factura, ejercita los casos de éxito,
parciales y de fallo que ofrezca el proveedor, confirma que su firma se
acepta, y concilia el ID de pago contra la factura y el pedido almacenados.
Repite una entrega y confirma que la clave de cumplimiento de la aplicación
impide una segunda concesión. Las pruebas locales no implican ninguna
ejecución en sandbox o producción con una cuenta real.

Referencias del proveedor: [endpoints de la API](https://nowpayments.zendesk.com/hc/en-us/articles/21345824322717-API-and-endpoint-description),
[autenticación de IPN](https://nowpayments.zendesk.com/hc/en-us/articles/21395546303389-IPN-and-how-to-setup)
y el [SDK oficial](https://github.com/NowPaymentsIO/nowpayments-sdk-nodejs).

## Siguiente

Consulta [Integración con el frontend](payments-frontend.md) para renderizar
payloads de redirección, o la
[Guía del proveedor](payments-provider-guide.md) para los contratos
compartidos.

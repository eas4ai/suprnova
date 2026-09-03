# Web Push

Web Push entrega un mensaje corto a un navegador incluso cuando tu
sitio está cerrado - el Service Worker se despierta, descifra el
payload, y muestra una notificación a nivel de sistema operativo.
Suprnova entrega el protocolo de punta a punta: generación de claves
VAPID, cifrado de payload con AES128GCM, el transporte HTTP, y un
`WebPushChannel` que se conecta al subsistema de notificaciones para
que la misma `Notification` que envías a mail o a la base de datos
también llegue como un push.

Recurre a esto cuando quieras alertar a los usuarios en tiempo real
sin un WebSocket abierto - un pedido enviado, una solicitud de
amistad, una mención, un saldo acreditado. Si el usuario está en un
navegador de escritorio con el sitio cerrado, web push es el único
mecanismo que los alcanza; si están en el sitio,
[Difusión](broadcasting.md) suele ser una mejor opción.

La API está detrás de la feature de Cargo `web-push`, que está
activada por defecto. Las aplicaciones que usan
`default-features = false` deben activar `web-push` explícitamente.

## Las cuatro piezas

Web Push tiene más piezas móviles que mail o database, porque la
especificación ([RFC 8030](https://datatracker.ietf.org/doc/html/rfc8030) +
[RFC 8291](https://datatracker.ietf.org/doc/html/rfc8291) +
[RFC 8292](https://datatracker.ietf.org/doc/html/rfc8292)) divide la
identidad, el cifrado, y el transporte en tres contratos:

| Pieza | Qué es |
|---|---|
| `VapidKey` / `VapidSigner` | Un par de claves ECDSA P-256 usado para firmar JWTs que demuestran que tu servidor es quien dice ser |
| `WebPushClient` | El cliente HTTP que cifra un payload, firma un JWT VAPID, y hace POST al endpoint de la suscripción |
| `WebPushChannel` | El adaptador del subsistema de notificaciones que convierte una `Notification` en una llamada a `WebPushClient::send` |
| `SubscriptionInfo` | La tripleta opaca (`endpoint`, `p256dh`, `auth`) que el navegador te entrega cuando un usuario se suscribe - tú la almacenas; no la generas |

Las tres capas inferiores - `VapidKey`, `WebPushClient`, el POST
cifrado - se re-exportan desde `suprnova::web_push`, así que las
aplicaciones nunca necesitan depender directamente del crate
`suprnova-web-push` subyacente.

## Generar un par de claves VAPID

Web Push usa VAPID (Voluntary Application Server Identification)
para que los servicios de push puedan limitar la velocidad y
contactar a los remitentes que se comportan mal. Necesitas un par de
claves P-256 por aplicación; la clave pública va en tu frontend para
que el navegador pueda anclar las suscripciones a tu servidor, y la
clave privada permanece en el servidor firmando JWTs.

Genera una sola vez, persístela, y reúsala para siempre:

```rust
use suprnova::VapidKey;

let key = VapidKey::generate();

// Guarda el PEM en algún lugar durable - un secrets manager, un archivo
// que el pipeline de deploy monta, un volumen de env-vars-as-files. NO
// PUEDES regenerar esto sin invalidar cada suscripción existente.
let pem = key.to_pem()?;
std::fs::write("vapid_private.pem", &pem)?;

// El frontend necesita la clave pública sin comprimir en
// base64url-no-padding. Entrégasela a tu JS para que
// `pushManager.subscribe()` pueda usarla como `applicationServerKey`.
println!("PUBLIC_VAPID_KEY={}", key.public_key_uncompressed_b64url());
```

En el arranque, carga el PEM guardado:

```rust
use suprnova::{VapidKey, VapidSigner};

let pem = std::fs::read_to_string("vapid_private.pem")?;
let key = VapidKey::from_pem(&pem)?;
let signer = VapidSigner::new(key);
```

Un `VapidSigner` produce JWTs pero no envía nada - es puramente una
primitiva de firma. La siguiente capa lo envuelve.

## Construir un WebPushClient

`WebPushClient` es la primitiva del lado HTTP: dale un signer y una
URI de contacto ("cómo puede contactarte el servicio de push si te
comportas mal"), y obtienes de vuelta un objeto cuyo método `send`
cifra un payload, firma un JWT, y hace POST al endpoint de la
suscripción.

```rust
use std::sync::Arc;
use suprnova::{VapidKey, VapidSigner, WebPushClient};

let signer = VapidSigner::new(VapidKey::from_pem(&pem)?);

// El subject DEBE ser una URI mailto: o una URL https: según RFC
// 8292 §2.1. Cualquier otra cosa se rechaza en la construcción, así
// que un deploy mal configurado falla rápido en el arranque - no en
// silencio después del primer despacho fallido.
let client = WebPushClient::new(signer, "mailto:ops@example.org")?;

let client = Arc::new(client);
```

¿Por qué `Arc<WebPushClient>`? `WebPushClient` envuelve un
`VapidSigner`, que a su vez envuelve un `ES256KeyPair` privado.
Ninguno de ellos es `Clone` - las claves privadas no deberían
duplicarse a la ligera - y construir un signer nuevo por cada
registro de canal significaría N identidades VAPID independientes
para la misma aplicación. Envolver en `Arc` permite que una sola
identidad firmada respalde cada registro y cada entrega concurrente.

### Política de endpoint

Los endpoints de suscripción son datos derivados del usuario: el
navegador recibe la URL de un servicio de push remoto cuando un
usuario se suscribe, y tu servidor almacena lo que sea que el
navegador haya devuelto. Una suscripción almacenada de forma
maliciosa puede apuntar el POST HTTP a cualquier lugar alcanzable,
convirtiendo al remitente de push en un gadget de SSRF.

`WebPushClient` usa `EndpointPolicy::Strict` por defecto:

- El scheme debe ser `https`
- El host debe ser un dominio con nombre, no un literal de IP
- Se rechazan los hostnames de metadatos de nube y los TLD
  reservados por RFC 2606 (`.localhost`, `.local`, `.internal`,
  `.test`, `.example`, `.invalid`)

Esto bloquea las sondas obvias de SSRF sin romper los servicios de
push reales (FCM, Mozilla Autopush, el `web.push.apple.com` de
Apple).

Para tests de integración locales contra un servidor mock de
`wiremock` tienes que optar por lo contrario:

```rust
use suprnova::{EndpointPolicy, WebPushClient};

let client = WebPushClient::new(signer, "mailto:test@example.org")?
    .with_endpoint_policy(EndpointPolicy::AllowAny);
```

No uses `AllowAny` en producción. Las comprobaciones estrictas
existen para evitar que una tabla de suscripciones manipulada se
convierta en un arma.

### Transporte personalizado

`WebPushClient::new` aplica un timeout de 30 segundos por solicitud.
Si necesitas una política de transporte distinta - un proxy
corporativo, TLS anclado, un timeout más corto - pasa un
`reqwest::ClientBuilder` a `WebPushClient::with_client_builder`.
Todas las opciones del builder se respetan, pero la política de
redirección se desactiva forzosamente: un endpoint validado que
responde 3xx no debe rebotar el POST a una URL no validada, así
que la librería no acepta la configuración de redirección del
llamador.

```rust
use reqwest::Client;
use std::time::Duration;
use suprnova::WebPushClient;

let client = WebPushClient::with_client_builder(
    Client::builder().timeout(Duration::from_secs(10)),
    signer,
    "mailto:ops@example.org",
)?;
```

`WebPushClient::with_client` acepta un cliente ya construido cuya
política de redirección la librería no puede inspeccionar. Los
envíos bajo la política `Strict` por defecto se rechazan para ese
transporte antes de cualquier I/O - cambia a
`with_client_builder`, o acepta el riesgo explícitamente con
`.allow_unconfined_redirects()` cuando se sabe que el cliente no
sigue redirecciones.

## Conectar WebPushChannel con las notificaciones

El `WebPushClient::send` en crudo funciona - pero la forma en que
realmente envías notificaciones push en Suprnova es a través del
subsistema de [Notificaciones](notifications.md). Una `Notification`
declara `vec!["webpush"]` en su `channels()`, un destinatario
`Notifiable` devuelve un `SubscriptionInfo` codificado en JSON desde
`route_for("webpush")`, y el `NotificationDispatcher` vinculado hace
la dispersión.

```rust
use std::sync::Arc;
use suprnova::{
    NotificationDispatcher, WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};

let client: Arc<WebPushClient> = Arc::new(
    WebPushClient::new(signer, "mailto:ops@example.org")?
);

// ttl_secs: cuánto tiempo retiene el servicio de push un mensaje sin
// entregar. 86_400 (24h) es un valor por defecto razonable para
// notificaciones no urgentes; bájalo a 60 para alertas de "actúa
// ahora mismo" donde un mensaje obsoleto es peor que ningún mensaje.
let webpush = Arc::new(WebPushChannel::new(client, 86_400));

let dispatcher = NotificationDispatcher::new()
    .register_channel(webpush);

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` es last-write-wins sobre el `name()` del canal,
así que los tests pueden sustituir por un stub sin afectar la
vinculación de producción.

## Definir una notificación

Una notificación destinada a push tiene la misma forma que cualquier
otra notificación de Suprnova - declara `"webpush"` en `channels()`
y pon en `data()` cualquier JSON que quieras entregar:

```rust
use serde::{Deserialize, Serialize};
use suprnova::Notification;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OrderShipped {
    pub order_id: i64,
    pub tracking_url: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str {
        "OrderShipped"
    }

    fn channels(&self) -> Vec<&'static str> {
        vec!["webpush"]
    }

    fn data(&self) -> serde_json::Value {
        serde_json::json!({
            "title":   "Your order has shipped",
            "body":    format!("Track order #{}", self.order_id),
            "url":     self.tracking_url,
        })
    }
}
```

El `data()` JSON es lo que tu Service Worker recibe. Elige una forma
estable y documéntala para el frontend - Suprnova no impone ninguna,
porque la UI de notificaciones es un asunto del frontend.

## Enrutar al destinatario

Un `Notifiable` devuelve la ruta para cada canal que admite. Para
Web Push, esa ruta es el `SubscriptionInfo` codificado en JSON -
exactamente lo que el navegador produjo mediante
`PushSubscription.toJSON()`, almacenado literalmente:

```rust
use suprnova::Notifiable;

pub struct User {
    pub id: i64,
    pub push_subscription_json: Option<String>,
}

impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "webpush" => self.push_subscription_json.clone(),
            _ => None,
        }
    }
}
```

Devolver `None` hace que el despachador omita el canal en silencio -
útil para usuarios que no se han suscrito a push pero de todos modos
reciben correo.

## Enviarlo

Síncrono:

```rust
use suprnova::Notify;

let user = User::find(42).await?.unwrap();
Notify::send(&user, &OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

Encolado - resuelve por adelantado la ruta de la suscripción en el
momento de encolar, así que el worker no necesita volver a cargar al
usuario:

```rust
Notify::queue(&user, OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

Para que `Notify::queue` funcione, registra la factory de la
notificación en el arranque, así el worker puede reconstruir el
payload JSON en la notificación tipada:

```rust
suprnova::notifications::register_notification_factory::<OrderShipped>()?;
suprnova::queue::worker::register_job::<suprnova::SendNotificationJob>();
```

Detrás de escena, el despacho encolado construye un
`SendNotificationJob` que lleva
`(notification_name, payload, per_channel_routes, channels)`. El
worker rehidrata la notificación, busca `WebPushChannel` por nombre
en el despachador vinculado, y llama a
`deliver(route, &notification)` - la misma ruta de código que el
`Notify::send` síncrono.

## El lado del navegador

Suprnova no entrega un SDK de JavaScript - el lado del navegador es
la API de Web Push sin más. El flujo que tu frontend necesita
implementar:

1. Registrar un Service Worker.
2. Pedirle permiso al usuario.
3. Suscribirse mediante `pushManager.subscribe({ userVisibleOnly: true,
   applicationServerKey: <tu clave pública VAPID> })`.
4. Hacer POST de `subscription.toJSON()` a un endpoint de Suprnova
   que lo almacene en la fila del usuario.

```js
// Registro del Service Worker (en algún lugar del entrypoint de tu app)
const registration = await navigator.serviceWorker.register('/sw.js');

if (Notification.permission === 'default') {
    await Notification.requestPermission();
}

if (Notification.permission === 'granted') {
    const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: window.PUBLIC_VAPID_KEY,
    });

    await fetch('/api/push/subscribe', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(subscription.toJSON()),
    });
}
```

Tu endpoint de Suprnova recibe el JSON, valida la forma, y lo
almacena en el usuario - el string es opaco para tu servidor, pero
tiene que ser exactamente el JSON que produjo el navegador (el tipo
`SubscriptionInfo` usa `Deserialize` para analizarlo más adelante):

```rust
use suprnova::{Auth, Request, Response, SubscriptionInfo, attrs, json_response};

pub async fn subscribe(req: Request) -> Response {
    let user_id = Auth::id().expect("auth middleware");

    let (_parts, bytes) = match req.body_bytes().await {
        Ok(b) => b,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };
    let raw = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return json_response!({ "error": "body not utf-8" }).map(|r| r.status(400)),
    };

    // Analiza para validar la forma - endpoint, keys.p256dh, keys.auth.
    // Si el análisis falla, el navegador nos entregó algo malformado.
    let sub: SubscriptionInfo = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };

    // Persiste `raw` literalmente - ese es el string exacto que
    // WebPushChannel entregará a serde_json::from_str en el despacho.
    User::query()
        .db_where_op("id", "=", user_id)
        .update_all(attrs! { push_subscription_json: raw })
        .await
        .unwrap();

    json_response!({ "ok": true, "endpoint": sub.endpoint })
}
```

El Service Worker descifra el payload del push y renderiza la
notificación:

```js
// /sw.js
self.addEventListener('push', (event) => {
    const data = event.data.json();
    event.waitUntil(
        self.registration.showNotification(data.title, {
            body: data.body,
            data: { url: data.url },
        }),
    );
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    event.waitUntil(clients.openWindow(event.notification.data.url));
});
```

## Límites de payload

La especificación de Web Push topa cada payload cifrado en 4096
bytes en total. Suprnova rechaza los textos planos mayores de 3992
bytes (el tope menos el overhead de cifrado de AES128GCM, de unos 85
bytes) en el momento de cifrar, así que el fallo aparece en tu
código, no en un 413 del servicio de push. Una `Notification` cuyo
`data()` serializado excede ese límite devuelve
`WebPushError::Encryption` desde el `deliver` del canal.

Para cualquier cosa más grande - un cuerpo de mensaje largo, una
miniatura - envía una notificación corta que lleve una URL que el
Service Worker consulta al hacer clic. Eso es a la vez más rápido
(sin cifrado sobre un payload de varios KB) y más flexible (el fetch
puede devolver la forma que quieras).

## Suscripciones muertas

Cuando el servicio de push devuelve 404 o 410, la suscripción está
muerta - el usuario desinstaló el navegador, revocó el permiso, o
limpió el almacenamiento. `WebPushChannel` trata esto como un warn no
fatal:

```text
WARN webpush subscription gone (404/410); caller should remove
     channel=webpush endpoint=https://fcm.googleapis.com/fcm/send/abc
```

El despacho devuelve `Ok(())` porque la notificación llegó a un
estado terminal - no hay ningún destinatario contra el cual
reintentar. Se espera que tu aplicación actúe sobre el warn: analiza
`endpoint` desde el log (o conecta un oyente de `NotificationFailed`
que clasifique mediante `WebPushError`) y elimina la fila de la
suscripción. Suprnova entrega el warn; no depura automáticamente la
tabla de suscripciones por ti.

## Reintentos y Retry-After

Cuando el servicio de push devuelve un 5xx, 408, o 429 transitorio,
el `WebPushError::PushServiceRejected` subyacente lleva la pista
`Retry-After` analizada (solo la forma en delta-seconds - la forma
HTTP-date devuelve `None`):

```rust
use suprnova::WebPushError;

match client.send(&sub, payload, ContentEncoding::Aes128Gcm, 60).await {
    Ok(_) => (),
    Err(e) if e.is_retryable() => {
        let wait = e.retry_after().unwrap_or(Duration::from_secs(30));
        tokio::time::sleep(wait).await;
        // ...reintenta, o vuelve a encolar con un retardo
    }
    Err(WebPushError::SubscriptionGone) => {
        // elimina la suscripción
    }
    Err(e) => return Err(e.into()),
}
```

La pista `Retry-After` está topada a 24 horas para que un servidor
hostil no pueda dejar a un worker durmiendo durante años.

Cuando usas `Notify::queue`, se aplica el retry/backoff propio de la
cola - un `WebPushError` que se propaga fuera de
`WebPushChannel::deliver` emerge como un error de job y el sobre
gestiona el reencolado según la política de backoff del job. La
pista `Retry-After` se registra en el log pero (todavía) no
retroalimenta el cálculo del retardo de la cola; si necesitas eso,
conecta un oyente de `NotificationFailed` que reencole con el
retardo sugerido.

## Telemetría

El despachador de notificaciones envuelve la dispersión en un span
info `notification.dispatch` etiquetado con el nombre de la
notificación y la cantidad de canales. Cada entrega exitosa emite un
evento `NotificationSent`; los fallos emiten `NotificationFailed`
llevando el nombre del canal, la ruta, y el string del error. Conecta
cualquiera de estos a tu pipeline de métricas/logs de la misma forma
en que conectas otros eventos del framework - ver [Eventos](events.md).

Una suscripción muerta emite un WARN estructurado con
`channel="webpush"`, el endpoint, y el nombre de la notificación. Esa
es la señal que hay que rastrear para un job automatizado de limpieza
de suscripciones.

### Por qué Suprnova diverge

El driver `WebPush` de Laravel es un paquete comunitario
(`laravel-notification-channels/webpush`) - no está en el núcleo, se
versiona por separado, y tiene opiniones sobre el ORM. Suprnova
integra Web Push directamente en el framework porque el protocolo
está bien definido y el POST HTTP cifrado es un contrato demasiado
pequeño como para envolverlo en una abstracción de terceros. El
subsistema de notificaciones mantiene la superficie uniforme: la
misma `Notification` que envías a mail o a la base de datos también
llega como un push, sin matriz de drivers, sin árbol de
configuración separado.

También exponemos la política de endpoint estricto por defecto. El
paquete comunitario de Laravel deja la protección contra SSRF en
manos de la aplicación; nosotros sostenemos que "el endpoint proviene
de datos de usuario" es la forma de cada suscripción de Web Push, y
que el valor por defecto seguro pertenece al framework, no a tu
código.

La clasificación de reintento (`is_retryable`, `retry_after`) se
expone como métodos tipados sobre `WebPushError` en lugar de como
una tabla de constantes mágicas en la capa de cola. La cola sigue
siendo la propietaria de la política de reintento - el error te dice
si un reintento podría tener éxito y cuánto esperar; la cola decide
si y cuándo volver a extraer. Separar las dos cosas significa que
tus estrategias de reintento personalizadas (backoff exponencial, con
jitter, topado) no tienen que hacer un caso especial para Web Push.

## Pruebas

Levanta un servidor `wiremock`, apunta un `WebPushClient` hacia él
con `EndpointPolicy::AllowAny`, y haz aserciones sobre las
solicitudes que recibe:

```rust
use std::sync::Arc;
use suprnova::{
    EndpointPolicy, NotificationDispatcher, Notify, VapidKey, VapidSigner,
    WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn order_shipped_pushes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/push"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let signer = VapidSigner::new(VapidKey::generate());
    let client = Arc::new(
        WebPushClient::new(signer, "mailto:test@example.org")
            .unwrap()
            .with_endpoint_policy(EndpointPolicy::AllowAny),
    );
    let channel = Arc::new(WebPushChannel::new(client, 60));

    let dispatcher = NotificationDispatcher::new().register_channel(channel);
    set_dispatcher(Arc::new(dispatcher)).unwrap();

    let user = test_user_with_subscription(&server.uri()).await;
    Notify::send(&user, &OrderShipped {
        order_id: 1,
        tracking_url: "https://ship.example.org/o/1".into(),
    }).await.unwrap();
    // server.received_requests() ahora contiene el POST cifrado.
}
```

Para tests end-to-end a los que no les importan los bytes cifrados,
`Notify::fake()` (cubierto en [Notificaciones](notifications.md))
captura el despacho sin ejecutar el canal - más rápido, sin servidor
mock, sin ida y vuelta de cifrado.

## Referencia

- Primitivas: `suprnova::VapidKey`, `suprnova::VapidSigner`,
  `suprnova::VapidClaims`
- Cliente: `suprnova::WebPushClient`, `suprnova::EndpointPolicy`,
  `suprnova::PushResponse`, `suprnova::SubscriptionInfo`
- Error: `suprnova::WebPushError` - `.is_retryable()`,
  `.retry_after()`, `WebPushError::SubscriptionGone`
- Codificación: `suprnova::ContentEncoding` (Aes128Gcm; tope de 3992
  bytes de texto plano)
- Canal: `suprnova::WebPushChannel`
- Fachada: `suprnova::Notify`
- Job de cola: `suprnova::SendNotificationJob`
- Registro de factory:
  `suprnova::notifications::register_notification_factory`

## Siguiente

- [Notificaciones](notifications.md) - el despachador multicanal al
  que se conecta `WebPushChannel`
- [Correo](mail.md) - la contraparte del canal de correo para
  usuarios sin push
- [Difusión](broadcasting.md) - entrega en tiempo real para usuarios
  que están en el sitio
- [Cola](queues.md) - cómo `Notify::queue` respalda a
  `SendNotificationJob`
- [Eventos](events.md) - escuchar `NotificationSent` /
  `NotificationFailed` para impulsar la limpieza de suscripciones
  muertas

# Notificaciones

Una notificación es un mensaje pequeño que quieres que un usuario (o
"cualquiera con una dirección de correo") reciba a través de uno o más
canales - correo, bandeja de entrada dentro de la app, push del
navegador, WebSocket en tiempo real - desde un único sitio de llamada.
Escribes `Notify::send(&user, &OrderShipped { … })`; el despachador
dispersa esa única notificación a través de cada canal que la
notificación declaró, dirigiéndose a cada uno a través del
destinatario.

Usa notificaciones cuando el *qué* (un pedido se envió, una factura se
pagó) le importa más a tu código que el *cómo* (qué transporte terminó
entregándolo). Para acceso directo al transporte - componer un cuerpo
de correo personalizado, publicar en un canal de difusión específico,
enviar un web push puntual - pasa directamente por [mail](mail.md),
[difusión](broadcasting.md), o [web push](web-push.md).

## Inicio rápido

```rust
use serde::{Deserialize, Serialize};
use suprnova::FrameworkError;
use suprnova::NotificationMailable;          // macro derive
use suprnova::notifications::channels::mail::MailRendering;
use suprnova::{Notifiable, Notification, Notify};

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Order shipped - tracking {{ tracking }}",
    html    = "<p>Your order is on its way.</p><p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@example.com",
    from_name = "Acme Orders",
)]
pub struct OrderShipped {
    pub tracking: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str { "OrderShipped" }
    fn channels(&self) -> Vec<&'static str> { vec!["mail", "database"] }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "tracking": self.tracking })
    }
}

struct User { id: i64, email: String }
impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "mail"     => Some(self.email.clone()),
            "database" => Some(self.id.to_string()),
            _          => None,
        }
    }
}

async fn ship(user: &User, tracking: String) -> Result<(), FrameworkError> {
    Notify::send(user, &OrderShipped { tracking }).await
}
```

`Notify::send` despacha al canal de mail y al canal de database en
una sola llamada. El destinatario declina un canal devolviendo `None`
desde `route_for` - útil para usuarios "solo email" o "solo push".

## Los tres traits

| Trait | Qué representa | Implementado por |
|---|---|---|
| `Notification` | Un mensaje tipado + los canales a los que despacha | Tus structs de notificación |
| `Notifiable` | Un destinatario - expone un `route_for` por canal | Tu `User`, `Order`, cualquier cosa direccionable |
| `Channel` | Un transporte - sabe entregar a una ruta | Incluidos: `MailChannel`, `DatabaseChannel`, `BroadcastChannel`, `WebPushChannel` |

### `Notifiable`

```rust
pub trait Notifiable: Send + Sync {
    fn route_for(&self, channel: &str) -> Option<String>;
}
```

El destinatario es propietario del direccionamiento por canal.
`route_for("mail")` devuelve la dirección de correo; `route_for("database")`
devuelve el id de la entidad como string; `route_for("webpush")`
devuelve un `SubscriptionInfo` serializado en JSON; `route_for("broadcast")`
devuelve el nombre del canal de difusión. Devuelve `None` para omitir
un canal para este destinatario.

### `Notification`

```rust
pub trait Notification: Serialize + DeserializeOwned + Send + Sync + 'static {
    fn notification_name() -> &'static str where Self: Sized;
    fn channels(&self) -> Vec<&'static str>;
    fn data(&self) -> serde_json::Value;

    fn should_send(&self, _channel: &str) -> bool { true }
    fn after_sending(&self, _channel: &str) -> Result<(), FrameworkError> { Ok(()) }
    fn queue(&self) -> Option<&'static str> { None }
    fn timeout(&self) -> Option<std::time::Duration> { None }
    fn fail_on_timeout(&self) -> bool { false }
    fn max_tries(&self) -> u32 { 3 }
    fn backoff(&self) -> BackoffSchedule { BackoffSchedule::default() }
}
```

| Método | Propósito |
|---|---|
| `notification_name()` | Identificador estable persistido por el canal de database, usado como clave del sobre en la cola, y como clave de búsqueda en el registro de renderers de mail. |
| `channels(&self)` | Nombres de canal a los que despacha esta notificación. El orden es el orden de iteración. |
| `data(&self)` | Payload serializable a JSON que los canales entregan / persisten. Típicamente `serde_json::to_value(self)` del subconjunto de campos que los canales necesitan. |
| `should_send(&self, channel)` | Veto por canal consultado en ambas rutas, la síncrona y la encolada. Devolver `false` omite ese canal para este despacho. Por defecto: siempre envía. |
| `after_sending(&self, channel)` | Hook posterior al éxito invocado una vez por cada canal que se completó, en ambas rutas, la síncrona y la encolada. Devolver `Err` se propaga de la misma forma que lo haría un error de canal. Por defecto: no-op. |
| `queue(&self)` | Cola a la que resuelve el despacho `Notify::queue` de esta notificación. Por defecto: `None` (el predeterminado del driver, o un `Queue::route` si se registró uno). Consulta [Ajuste de cola](#queue-tuning). |
| `timeout(&self)` | Tiempo de espera por intento para los jobs encolados de esta notificación. Por defecto: `None` (sin timeout). |
| `fail_on_timeout(&self)` | Si es `true`, un timeout es un fallo permanente (dead-letter, sin reintento). Por defecto: `false`. |
| `max_tries(&self)` | Máximo de intentos para los jobs encolados de esta notificación. Por defecto: `3`. |
| `backoff(&self)` | Programa de backoff para los jobs encolados de esta notificación. Por defecto: el del framework. |

`should_send` y `after_sending` se respetan en **ambas** rutas.
`Notify::send` los consulta en el despachador; `Notify::queue`
comprueba `should_send` antes de encolar cada job por canal, y el
worker vuelve a comprobar `should_send` antes de la entrega (el
estado puede cambiar entre el encolado y la ejecución) y ejecuta
`after_sending` después de un envío exitoso. Los tres *eventos* de
ciclo de vida (`NotificationSending` / `NotificationSent` /
`NotificationFailed`) siguen disparándose solo en la ruta síncrona.

## Canales

### Correo

El canal de mail entrega a través del transporte de mail vinculado
(ver [Correo](mail.md)). Una notificación participa implementando
`NotificationMailable`:

```rust
pub trait NotificationMailable: Notification {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError>;
}
```

`MailRendering` es el sobre de renderizado - `subject` (obligatorio),
`html` y/o `text` (al menos uno obligatorio), y opcionalmente `from`,
`cc`, `bcc`, `reply_to`, y `attachments`. El canal de mail ensambla un
mensaje saliente a partir de este renderizado más el
`route_for("mail")` del destinatario, aplica los valores por defecto
de remitente configurados (`Mail::always_from(...)`, `always_to(...)`,
etc.), y despacha a través de `Mail::current_transport`.

Si el renderer devuelve un renderizado sin `html` ni `text`, la
entrega falla rápido - nunca se envía en silencio un correo de
notificación en blanco.

#### `#[derive(NotificationMailable)]`

El derive colapsa el `impl` de `to_mail` por cada `Notification` en un
único atributo `#[mail(...)]`. Los templates usan
[Tera](https://keats.github.io/tera/); los campos serializados de
`self` son el contexto.

```rust
#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Welcome {{ name }}",
    html_template = "templates/welcome.html",
    text_template = "templates/welcome.txt",
    from = "hello@example.com",
    from_name = "Acme",
    cc = "ops@example.com, support@example.com",
)]
pub struct Welcome { pub name: String }
```

Claves admitidas:

| Clave | ¿Obligatoria? | Propósito |
|---|---|---|
| `subject` | sí | Template de Tera - renderizado con `self` como contexto. |
| `html` | daga | Template de Tera en línea para el cuerpo HTML. |
| `html_template` | daga | Ruta a un template de Tera para el cuerpo HTML (incrustado vía `include_str!`). |
| `text` | daga | Template de Tera en línea para el cuerpo de texto plano. |
| `text_template` | daga | Ruta a un template de Tera para el cuerpo de texto plano (incrustado vía `include_str!`). |
| `from` | no | Correo del remitente - anula el `noreply@localhost` por defecto. |
| `from_name` | no | Nombre para mostrar. Requiere `from`. |
| `cc` | no | Lista de CC separada por comas. Se ignoran los espacios y las comas finales. |
| `bcc` | no | Lista de BCC separada por comas. |
| `reply_to` | no | Lista de Reply-To separada por comas. |

(daga) Debe estar presente al menos una variante de cuerpo. `html` y
`html_template` son mutuamente excluyentes; lo mismo para `text` y
`text_template`.

Cada invariante se aplica en tiempo de compilación - un `subject`
faltante, un cuerpo vacío, variantes en conflicto, `from_name` sin
`from`, o claves desconocidas hacen fallar el build en lugar de
fallar en el despacho.

Para adjuntos (payloads binarios) o destinatarios dinámicos por
instancia, implementa `NotificationMailable` a mano y construye el
`MailRendering` directamente.

### Base de datos

El canal de database persiste cada notificación como una fila en la
tabla `notifications`:

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, NotificationDispatcher};

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")));
```

El segundo argumento es la etiqueta de tipo polimórfico del
destinatario (lo que almacenas en `notifiable_type` para poder
consultar las filas de la bandeja de entrada más adelante). El
`route_for("database")` del destinatario se convierte en el
`notifiable_id`. La migración se entrega con el framework
(`framework/migrations/20260516_create_notifications_table.sql`);
ejecuta `suprnova migrate` y la tabla aparece.

#### Leer la bandeja de entrada

Los helpers de lectura viven en `suprnova::notifications` como
funciones libres sobre `(notifiable_type, notifiable_id)`:

```rust
use suprnova::notifications::{
    all_for, unread_for, read_for,
    mark_as_read, mark_as_unread, mark_all_as_read,
    delete_for, StoredNotification,
};

let unread: Vec<StoredNotification> = unread_for(&db, "users", "42").await?;
let count = mark_all_as_read(&db, "users", "42").await?;
let removed = delete_for(&db, "users", "42").await?;
```

`StoredNotification` lleva `id`, `type_name` (el
`Notification::notification_name`), `notifiable_type`,
`notifiable_id`, el `data` en JSON decodificado, `read_at`,
`created_at`, `updated_at`. `mark_as_read` / `mark_as_unread` son
idempotentes (igualan el contrato de Laravel).

### Web Push

El canal de web push cifra el payload y hace POST a un endpoint de
suscripción push del navegador almacenado, a través del cliente de
firma VAPID del framework:

```rust
use std::sync::Arc;
use suprnova::WebPushChannel;
use suprnova::web_push::{VapidKey, WebPushClient};

let client = WebPushClient::new(
    VapidKey::from_pem(b"-----BEGIN PRIVATE KEY-----\n…")?,
    "mailto:ops@example.com",
)?;
let push_channel = WebPushChannel::new(Arc::new(client), 86_400 /* TTL en segundos */);
```

El `route_for("webpush")` del destinatario devuelve un
`SubscriptionInfo` serializado en JSON (la misma forma que el
navegador devuelve desde `PushSubscription.toJSON()` - almacénalo
literalmente, devuélvelo sin tocarlo). El TTL se reenvía al servicio
de push.

Cuando el servicio de push le dice al canal que una suscripción ya no
existe (HTTP 404/410), el canal registra un WARN estructurado y
devuelve éxito - la notificación llegó a un estado terminal sin
ningún destinatario contra el cual reintentar. Los operadores ven el
log y eliminan la suscripción muerta; la entrega no falla.

Ver [Web Push](web-push.md) para el cliente completo.

### Difusión

El canal de broadcast publica cada notificación en el `BroadcastHub`
de la aplicación para que los suscriptores de WebSocket la reciban en
tiempo real. El `route_for("broadcast")` del destinatario es el
nombre del canal, el tipo de la notificación es el evento, y `data()`
es el payload:

```rust
use std::sync::Arc;
use suprnova::BroadcastChannel;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;

// En el arranque - vincula el hub antes de cualquier despacho de difusión.
App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

let dispatcher = suprnova::NotificationDispatcher::new()
    .register_channel(Arc::new(BroadcastChannel::new()));
```

El canal resuelve el hub desde el contenedor en el momento de la
entrega. Si no hay ningún `BroadcastHub` vinculado cuando una
notificación declara `"broadcast"`, el canal devuelve un error - una
aplicación mal configurada expone el problema en lugar de descartar
el mensaje en silencio. Publicar en un canal sin suscriptores activos
no es un error.

Ver [Difusión](broadcasting.md) para la configuración del hub y el
cableado de WebSocket.

## Notificaciones a demanda

A veces quieres notificar a *alguien que no está en tu base de
datos* - una alerta puntual de operaciones a una dirección de correo,
un receptor de webhook, un canal de difusión que no pertenece a
ningún usuario. `AnonymousNotifiable` es el "usuario sin fila":

```rust
use suprnova::Notify;

let recipient = Notify::route("mail", "ops@example.com")?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;

// Varios canales en un solo builder:
let recipient = Notify::routes([
    ("mail", "ops@example.com"),
    ("broadcast", "ops-channel"),
])?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;
```

`Notify::route("database", …)` y `Notify::routes([..., ("database",
…)])` devuelven `Err` - el canal de database persiste un par
`(notifiable_type, notifiable_id)` que un destinatario anónimo no
puede suministrar.

## El despachador

`NotificationDispatcher` mantiene el registro de canales.
Constrúyelo una vez en el arranque y vincúlalo globalmente:

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, MailChannel, NotificationDispatcher, WebPushChannel};
use suprnova::notifications::set_dispatcher;

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(MailChannel::new()))
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")))
    .register_channel(Arc::new(WebPushChannel::new(push_client, 86_400)));

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` es last-write-wins sobre el nombre del canal -
registrar dos canales llamados `"mail"` reemplaza en silencio al
primero. Esto hace que los setups de test sean ergonómicos.

Una notificación que declara un canal que el despachador no tiene
registrado registra un WARN (`no channel registered; skipping`) y
continúa con el siguiente canal - el despacho no falla ante un nombre
de canal desconocido.

`set_dispatcher` devuelve `Result<(), FrameworkError>` porque el
registro del despachador vive detrás de un `RwLock`; la ruta de error
solo se dispara si el bloqueo está envenenado (un escritor anterior
entró en pánico). En la práctica, el sitio de llamada en el arranque
usa `?`.

### Eventos de ciclo de vida

Tres eventos rodean cada entrega síncrona por canal:

| Evento | Cuándo | Comportamiento ante un error de oyente |
|---|---|---|
| `NotificationSending` | Inmediatamente antes de que el canal se ejecute | Un `Err` del oyente **veta** el canal para este despacho |
| `NotificationSent` | Después de una entrega exitosa | Despacho best-effort - los errores del oyente no se propagan |
| `NotificationFailed` | Cuando un canal devolvió un error | Despacho best-effort; el error de canal subyacente de todos modos se propaga según el contrato de detención en el primer fallo |

Los tres llevan `(notification, channel, route, data)`. `Failed`
agrega el `error` convertido a string. Escúchalos con
`EventFacade::listen::<E, L>` - ver [Eventos](events.md).

Estos eventos se disparan solo en la ruta síncrona de `Notify::send`.
El worker encolado entrega los canales directamente sin despachar los
eventos.

### Telemetría

`NotificationDispatcher::notify` envuelve la dispersión en un span de
tracing `notification.dispatch`:

- `notification` - `Notification::notification_name()`
- `channel_count` - cantidad de canales declarados
- `duration_ms` - latencia de la dispersión al completarse
- log terminal: `notification dispatched` (info) o
  `notification dispatch failed` (warn)

El canal de mail anida su propio span `mail.send` por dentro.

### El contrato de detención en el primer fallo

`Notify::send` retorna ante el primer error de canal. Los canales que
ya tuvieron éxito no se revierten; los canales que aún no se
ejecutaron no se intentan. El mismo contrato se aplica al worker
encolado.

Para al-menos-una-vez a través de varios canales, despacha cada canal
mediante su propia llamada a `Notify::queue` - las claves de
idempotencia del sobre de la cola protegen contra envíos duplicados
en un reintento.

## Entrega encolada

`Notify::send` se ejecuta en proceso. `Notify::queue` empuja un
`SendNotificationJob` a la [Cola](queues.md), resolviendo por
adelantado las rutas por canal a partir del destinatario para que el
worker no necesite un handle `Notifiable` en el momento de la
ejecución:

```rust
use suprnova::notifications::register_notification_factory;
use suprnova::Notify;

// En el arranque - una vez por cada notificación concreta alcanzable vía Notify::queue.
register_notification_factory::<OrderShipped>()?;

// En cualquier lugar:
Notify::queue(&user, OrderShipped { tracking }).await?;
```

En el momento del despacho, el worker:

1. Busca la factory de la notificación por `notification_name`
2. Reconstruye la notificación tipada a partir del payload JSON
3. Recorre los canales registrados en el momento de encolar
4. Para cada uno, vuelve a comprobar `should_send(channel)`
   (omitiendo los canales vetados), busca el canal en el
   despachador vinculado, llama a `deliver(route, &notification)`,
   y luego ejecuta `after_sending(channel)`

Los canales que se declararon en el momento de encolar pero que no
están registrados cuando el worker se ejecuta registran un WARN y se
omiten - el mismo contrato que la ruta síncrona. Los canales sin
ruta pre-resuelta se omiten en silencio (el destinatario devolvió
`None` en el momento de encolar).

`Notify::queue` también evalúa `should_send` en el momento de
encolar, así que un canal vetado nunca llega a encolarse; la
re-comprobación del worker cubre el estado que cambia entre el
encolado y la ejecución. La ruta encolada **no** dispara los tres
eventos de ciclo de vida (`NotificationSending` / `NotificationSent`
/ `NotificationFailed`) - esos siguen siendo solo-síncronos. Si
dependes de los eventos, envía a través de `Notify::send`.

### Política de cola por notificación

Cinco métodos más de `Notification` llevan la política de cola por
notificación al despacho de `Notify::queue`, reflejando los propios
métodos de ajuste de `Job`:

| Método | Predeterminado | Equivalente |
|---|---|---|
| `queue(&self)` | `None` - predeterminado del driver, o un `Queue::route` si se registró uno | `Job::queue()` |
| `timeout(&self)` | `None` - sin timeout por intento | `Job::timeout()` |
| `fail_on_timeout(&self)` | `false` - un timeout se reintenta como cualquier otro fallo | `Job::fail_on_timeout()` |
| `max_tries(&self)` | `3` | `Job::max_tries()` |
| `backoff(&self)` | exponencial, base de 2 s, tope de 5 min, jitter ±25 % | `Job::backoff()` |

`Notify::queue` los lee de la instancia de notificación una vez y los
lleva a cada push de `SendNotificationJob` por canal. Una notificación
que no sobrescribe ninguno de los cinco obtiene el envelope exacto que
siempre producía una llamada simple a `Notify::queue`.

```rust
struct WelcomeDigest;

impl Notification for WelcomeDigest {
    fn notification_name() -> &'static str { "WelcomeDigest" }
    fn channels(&self) -> Vec<&'static str> { vec!["mail"] }
    fn data(&self) -> serde_json::Value { serde_json::Value::Null }

    fn queue(&self) -> Option<&'static str> { Some("digests") }
    fn timeout(&self) -> Option<std::time::Duration> { Some(std::time::Duration::from_secs(10)) }
    fn fail_on_timeout(&self) -> bool { true }
}
```

Establece `fail_on_timeout(&self)` en `true` cuando un timeout significa
que la entrega es irrecuperable en lugar de transitoria: el worker envía a
dead-letter en el primer timeout en vez de reintentarlo hasta
`max_tries`.

Estos cinco métodos solo se aplican a `Notify::queue`: `Notify::send`
se ejecuta en proceso y no tiene un envelope de cola que ajustar.

### Por qué Suprnova diverge

Laravel condiciona las notificaciones encoladas a la interfaz
marcadora `ShouldQueue` - la misma llamada
`Notification::send($user, $notification)` encola si la notificación
implementa `ShouldQueue` y envía en línea si no lo hace. El
comportamiento depende de un flag a nivel de tipo en el sitio de la
notificación, que es invisible desde el sitio de la llamada.

Suprnova hace esa elección explícita en cada llamada: `Notify::send`
siempre es síncrono; `Notify::queue` siempre está encolado. No hay
ningún interruptor de modo oculto. (Esa es también la razón por la
que no hay `send_now` - `send` ya es el síncrono.)

El lado del destinatario también diverge. El trait `Notifiable` de
Laravel es un mixin que trae consigo la relación de bandeja de
entrada, los métodos `routeNotificationFor*`, y la clave primaria
polimórfica. El `Notifiable` de Suprnova es deliberadamente mínimo -
solo `route_for(channel) -> Option<String>` - porque los traits de
Rust no se componen por mixin. El equivalente Laravel del lado de
lectura se entrega como funciones libres sobre `(notifiable_type,
notifiable_id)` (`unread_for`, `mark_as_read`, …) para que structs
simples puedan ser notifiable sin heredar una relación de ORM.

## Pruebas

Dos superficies de fake, que responden preguntas distintas.

### `Notify::fake()` - "¿se despachó una notificación?"

```rust
use suprnova::Notify;
use suprnova::notifications::{
    assert_count, assert_nothing_sent, assert_sent_named,
    assert_sent_times, assert_sent_to, assert_sent_to_on,
    recorded_notifications,
};

#[tokio::test]
async fn ship_dispatches_order_shipped() {
    let _fake = Notify::fake();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    assert_sent_named("OrderShipped");
    assert_sent_to("alice@example.org", "OrderShipped");
    assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    assert_sent_times("OrderShipped", 1);
    assert_count(2); // mail + database
}
```

Mientras la guarda del fake está viva, tanto `Notify::send` como
`Notify::queue` registran el despacho en lugar de ejecutar canales o
encolar un job - no se ejecuta ningún canal, no se escribe ninguna
fila de cola. El fake mantiene un mutex de serialización de todo el
proceso, así que los tests en paralelo no pueden entrelazar sus
capturas; deja que la guarda `_fake` se descarte al final del test
para limpiar el recorder.

Usa `recorded_notifications()` para tener custodia completa de los
datos capturados:

```rust
let records = recorded_notifications();
assert_eq!(records[0].notification, "OrderShipped");
assert_eq!(records[0].channel, "mail");
assert_eq!(records[0].data["tracking"], "1Z…");
```

### `Mail::fake()` + `MailChannel` real - "¿la notificación se *renderizó* correctamente?"

`Notify::fake()` hace cortocircuito antes del canal. Para verificar
que el cuerpo del correo realmente se renderizó como esperas, ejecuta
el canal real bajo `Mail::fake()`:

```rust
use serial_test::serial;
use std::sync::Arc;
use suprnova::mail::Mail;
use suprnova::notifications::{set_dispatcher, NotificationDispatcher};
use suprnova::{MailChannel, Notify, register_mail_renderer};

#[tokio::test]
#[serial]
async fn ordershipped_renders_tracking_in_subject() {
    let fake = Mail::fake();
    register_mail_renderer::<OrderShipped>().unwrap();
    set_dispatcher(Arc::new(
        NotificationDispatcher::new()
            .register_channel(Arc::new(MailChannel::new())),
    )).unwrap();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.subject.contains("1Z…"));
}
```

Los tests que tocan el despachador, el renderer, o los globales del
transporte deben ser `#[serial_test::serial]` - son estáticos
globales de proceso.

## Buenas prácticas

### Registra cada factory y renderer en el arranque

`Notify::queue` reconstruye la notificación a través del registro de
factories en el worker, y `MailChannel` renderiza a través de
`register_mail_renderer`. Registra por adelantado cada notificación
encolable / mailable:

```rust
// bootstrap.rs
use suprnova::notifications::register_notification_factory;
use suprnova::register_mail_renderer;

pub fn register() -> Result<(), FrameworkError> {
    // Factories de notificación (una por cada Notification alcanzable vía Notify::queue).
    register_notification_factory::<OrderShipped>()?;
    register_notification_factory::<InvoicePaid>()?;

    // Renderers de mail (uno por cada NotificationMailable).
    register_mail_renderer::<OrderShipped>()?;
    register_mail_renderer::<InvoicePaid>()?;
    Ok(())
}
```

Una notificación sin registrar en la cola emerge como `unknown
notification: {name}` en el momento en que el worker la ejecuta, y
reintenta a través de la ruta de envío a fallidos. Un despacho de
`MailChannel` para un renderer sin registrar expone un error
`register via suprnova::register_mail_renderer::<N>()` de la misma
forma.

### Encola para dispersiones multicanal

El despachador síncrono visita los canales en orden y retorna ante el
primer error. Un fallo en el canal #2 deja al canal #1 confirmado y a
los canales #3 en adelante sin intentar. Para cualquier notificación
que atraviese más de un canal, prefiere `Notify::queue` para que el
worker gestione los reintentos con backoff y el despacho sobreviva a
una caída del proceso.

### Haz idempotentes las entregas por canal

Los reintentos del worker significan que el mismo
`SendNotificationJob` puede ejecutarse más de una vez. Los canales
incluidos son amigables con la idempotencia: `MailChannel` reenvía a
proveedores que típicamente deduplican por message-id;
`DatabaseChannel` inserta un UUID nuevo por ejecución (que es el
comportamiento correcto para una fila de auditoría); `WebPushChannel`
hace POST a un proveedor que absorbe los duplicados. Los canales
personalizados deberían apuntar a operaciones idempotentes - POSTs
HTTP con claves de deduplicación estables del lado del cliente,
upserts en lugar de inserciones ciegas, sin efectos secundarios de
"incrementar un contador" en la ruta de entrega.

### Vincula el despachador en un solo lugar

`register_channel` es last-write-wins, así que los tests pueden
sustituir un canal real por un stub en su setup. Mantén la
vinculación de producción en `bootstrap.rs` y deja que los tests
construyan su propio despachador con los stubs que necesiten. No
llames a `register_channel` de forma perezosa dentro de handlers de
solicitud - las escrituras del bloqueo global más la semántica
last-write-wins se vuelven impredecibles bajo carga concurrente.

## Referencia

| Símbolo | Ruta |
|---|---|
| `Notifiable`, `Notification`, `Channel`, `DynNotification` | `suprnova::` |
| `Notify` (fachada), `NotifyFakeGuard` | `suprnova::` |
| `NotificationDispatcher`, `NotificationFactory` | `suprnova::` |
| `AnonymousNotifiable` | `suprnova::` |
| `MailChannel`, `MailRendering`, `NotificationMailable` | `suprnova::` |
| `register_mail_renderer::<N>()` | `suprnova::` |
| `DatabaseChannel`, `StoredNotification` | `suprnova::` |
| `WebPushChannel` | `suprnova::` |
| `BroadcastChannel` | `suprnova::` |
| `SendNotificationJob` | `suprnova::` |
| `NotificationSending`, `NotificationSent`, `NotificationFailed` | `suprnova::` |
| `set_dispatcher`, `register_notification_factory` | `suprnova::notifications::` |
| `all_for`, `unread_for`, `read_for`, `mark_as_read`, `mark_as_unread`, `mark_all_as_read`, `delete_for` | `suprnova::notifications::` |
| `assert_sent`, `assert_sent_named`, `assert_sent_times`, `assert_sent_to`, `assert_sent_to_on`, `assert_nothing_sent`, `assert_nothing_sent_to`, `assert_count`, `recorded_notifications` | `suprnova::notifications::` |
| `#[derive(NotificationMailable)]` | `suprnova::` |

## Siguiente

- [Correo](mail.md) - el transporte y la superficie `Mailable` sobre
  la que se apoya el canal de mail
- [Difusión](broadcasting.md) - el `BroadcastHub` a través del cual
  publica el canal de broadcast
- [Web Push](web-push.md) - VAPID, cifrado, almacenamiento de
  suscripciones
- [Eventos](events.md) - escuchar `NotificationSending` / `Sent` /
  `Failed`
- [Cola](queues.md) - el worker que impulsa `Notify::queue`
- [Pruebas](testing.md) - superficies de fake y patrones de
  serial-test

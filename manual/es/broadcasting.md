# Difusión

La difusión es la capa de notificación de servidor a cliente
construida sobre la [primitiva de WebSocket](websockets.md) de
Suprnova. Despachas un evento `Broadcastable` a través de
`EventFacade`; el framework dispersa el sobre JSON del evento hacia
cada suscriptor de WebSocket en los canales que el evento nombra.
Nunca gestionas conexiones individuales - gestionas suscripciones a
canales, y el hub hace el resto.

El `BroadcastHub` es el bus. El `InMemoryBroadcastHub` por defecto se
ejecuta enteramente en proceso - perfecto para despliegues de una sola
réplica y para la suite de tests. Detrás de la feature de Cargo
`broadcasting-fanout`, `SeaStreamerBroadcastHub` enruta los mismos
eventos a través de un broker de streams (Redis Streams, Kafka,
archivo, stdio) para que una publicación en un proceso alcance a los
suscriptores en cada otro proceso.

Todo lo del capítulo de [WebSocket](websockets.md) sigue aplicando -
pings de heartbeat, `max_missed_pings`, `WsConfig`, middleware de
ruta, parámetros de ruta. La difusión solo añade un protocolo en la
red y un registro de canales por encima.

## Inicio rápido

Cuatro archivos y el navegador ve un evento.

`src/channels/order_updates.rs`:

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
}
```

`src/events/order_placed.rs`:

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        vec!["order.updates".into()]
    }
}
```

`src/bootstrap.rs`:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::container::App;
use suprnova::events::EventFacade;

pub async fn register() {
    // 1. Vincula el hub detrás del trait - los handlers lo resuelven de forma uniforme.
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    // 2. Registra cada canal por adelantado; el handler de WS resuelve por nombre.
    let mut registry = ChannelRegistry::new();
    registry.register(OrderUpdates);
    App::singleton(Arc::new(registry));

    // 3. Conecta el puente evento → hub una vez por cada tipo Broadcastable.
    EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
}
```

`src/routes.rs` - construye un `BroadcastingWsHandler` por ruta
resolviendo el hub y el registro ya arrancados desde el contenedor:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{
    BroadcastHub, BroadcastingWsHandler, ChannelRegistry, InMemoryBroadcastHub,
};
use suprnova::container::App;
use suprnova::{routes, ws, AuthMiddleware};

fn broadcasting_handler() -> BroadcastingWsHandler {
    // Primero el contenedor; recurre a un hub en proceso nuevo + registro
    // vacío para que los tests unitarios que ensamblan el router sin
    // bootstrap sigan funcionando.
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .unwrap_or_else(|| Arc::new(InMemoryBroadcastHub::new()));
    let registry: Arc<ChannelRegistry> = App::get::<Arc<ChannelRegistry>>()
        .unwrap_or_else(|| Arc::new(ChannelRegistry::new()));
    BroadcastingWsHandler::new(hub, registry)
}

routes! {
    ws!("/ws/broadcast", broadcasting_handler())
        .middleware(AuthMiddleware::new()),
}
```

Conéctate y observa:

```bash
wscat -c ws://localhost:3000/ws/broadcast
> {"action":"connected","socket_id":"6f1a3c2e-…"}
> {"action":"subscribe","channel":"order.updates","data":{}}
< {"action":"subscribed","channel":"order.updates"}
```

Despacha desde cualquier controlador, worker, o tarea programada:

```rust
EventFacade::dispatch(OrderPlaced { order_id: 99, user_id: 42 }).await?;
```

```
< {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}
```

## Canales

Un canal es un destino de suscripción con nombre. Los clientes se
suscriben por nombre; el hub entrega eventos a cada suscriptor activo
en ese nombre. El trait `Channel` tiene valores por defecto
asimétricos que fallan cerrados en las escrituras y abiertos en las
lecturas - ver [Por qué Suprnova diverge](#por-qué-suprnova-diverge)
más abajo.

### Canales públicos

El valor por defecto. Cualquier cliente puede suscribirse.

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
    // authorize() es true por defecto - abierto a todos los suscriptores.
}
```

### Canales privados

Anula `authorize` para controlar el acceso a las suscripciones. Un
subscribe rechazado produce un frame `error` con
`reason: "unauthorized"`; no se envía ningún frame `subscribed`.

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct PrivateChat;

#[async_trait]
impl Channel for PrivateChat {
    fn name(&self) -> &'static str { "chat.private" }

    async fn authorize(
        &self,
        _req: &Request,
        _params: &ChannelParams,
        data: &Value,
    ) -> bool {
        data["token"].as_str().map(|t| t == "valid").unwrap_or(false)
    }
}

impl PrivateChannel for PrivateChat {}
```

`data` es lo que sea que el cliente envió en el campo `data` del
frame de subscribe - un bearer token, un channel-bind firmado,
cualquier cosa definida por la aplicación. `Request` es la solicitud
de actualización HTTP original (los encabezados y las cookies se
pueden leer directamente). `params` lleva los valores capturados de
un nombre parametrizado y está vacío para nombres fijos.

`PrivateChannel` es un trait marcador. El framework no lo comprueba
en tiempo de ejecución - es una señal a nivel de tipo de que el canal
anula `authorize`, y está pensado para herramientas futuras (un lint
de clippy, un pase de auditoría).

### Canales parametrizados

Incrusta segmentos `{param}` en `name()` y un único registro sirve a
cada suscripción concreta que coincida con el patrón - el mismo
modelo que el `Broadcast::channel('orders.{id}', …)` de Laravel. Los
valores capturados llegan a cada hook como un mapa `ChannelParams`.

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct OrderChannel;

#[async_trait]
impl Channel for OrderChannel {
    fn name(&self) -> &'static str { "orders.{id}" }

    async fn authorize(
        &self,
        _req: &Request,
        params: &ChannelParams,
        _data: &Value,
    ) -> bool {
        let order_id = params.get("id").unwrap_or_default();
        // Controla el acceso según el id capturado - ¿el usuario de
        // la sesión es propietario de este pedido?
        !order_id.is_empty()
    }
}

impl PrivateChannel for OrderChannel {}

// Un único registro sirve a orders.42, orders.99, orders.featured, …
registry.register(OrderChannel);
```

Cada `{param}` se vincula a exactamente un segmento separado por
puntos: `orders.{id}` coincide con `orders.42` pero no con `orders`
ni con `orders.42.line`. La resolución prefiere un registro de nombre
fijo exacto sobre cualquier patrón (`orders.featured` gana a
`orders.{id}` para ese nombre concreto), luego el patrón más
específico (más segmentos literales), con el patrón
lexicográficamente menor como desempate determinista.

### Canales de presencia

Los canales de presencia rastrean la membresía. Cuando un cliente se
suscribe, el hub le entrega una instantánea `presence.here` a ese
cliente y difunde `presence.joined` a cada otro suscriptor. Cuando un
cliente se va, el hub difunde `presence.left`.

El contrato de dos partes es fácil de implementar solo a medias:
tienes que tanto anular `Channel::presence_info` para que devuelva
`Some(self)` COMO implementar `PresenceChannel::member_info`. Olvidar
`presence_info` conecta el canal como no-presencia - los subscribes
funcionan, pero `presence.joined` / `presence.here` / `presence.left`
nunca se disparan.

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use suprnova::FrameworkError;
use suprnova::broadcasting::{Channel, ChannelParams, PresenceChannel};
use suprnova::http::Request;

pub struct PresenceLobby;

#[async_trait]
impl Channel for PresenceLobby {
    fn name(&self) -> &'static str { "presence.lobby" }

    // Obligatorio - sin esta anulación, PresenceChannel queda conectado pero inerte.
    fn presence_info(&self) -> Option<&dyn PresenceChannel> {
        Some(self)
    }
}

#[async_trait]
impl PresenceChannel for PresenceLobby {
    async fn member_info(
        &self,
        _req: &Request,
        _params: &ChannelParams,
    ) -> Result<Value, FrameworkError> {
        // Devuelve lo que otros suscriptores necesitan para identificar a
        // este miembro - típicamente un user id. Nunca incluyas secretos ni PII privada.
        Ok(json!({ "user_id": 42, "display_name": "Alice" }))
    }
}
```

Ver [Presencia](#presencia) para el flujo de eventos completo y el
eco del self-join.

### Nombres reservados

Los nombres que empiezan con `__` están reservados para los
meta-canales del framework (`__presence__` lleva la replicación de
presencia entre procesos). Llamar a `registry.register(channel)`
sobre un nombre con prefijo `__` entra en pánico en el registro, así
que el error se detecta en el arranque, no en tiempo de ejecución.

### Por qué Suprnova diverge

Laravel vincula la autorización de canales a un parámetro de
callback `$user` porque PHP inyecta implícitamente al usuario
autenticado actual. El `authorize` de Suprnova, en cambio, toma el
`Request` en crudo, el `ChannelParams` capturado, y un
`data: Value` arbitrario - tres entradas ortogonales, todas
disponibles, sin ningún contexto implícito. Lees la cookie de sesión
o el bearer token desde `Request` y los params de estilo enrutamiento
desde `ChannelParams`; el payload `data` es un espacio libre para
tokens que el cliente proporciona en el momento de suscribirse.

Los valores por defecto del trait `Channel` son **asimétricos a
propósito**: `authorize` es `true` por defecto (el subscribe es
público por defecto), `authorize_publish` es `false` por defecto (el
publish iniciado por el cliente se niega por defecto). La acción
peligrosa falla cerrada; la segura falla abierta. Ante la duda, deja
ambas como están.

## El trait Broadcastable

`Broadcastable: Event + Serialize` - cada `Broadcastable` es también
un `Event`. El despacho vía `EventFacade::dispatch(event)` ejecuta
cada oyente en proceso Y envía el payload serializado en JSON a cada
suscriptor de WebSocket en los canales que el evento nombra.

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        // Un evento, varios canales. Cada suscriptor de cada canal
        // recibe el mismo sobre.
        vec![
            format!("user.{}.orders", self.user_id),
            "orders.global".into(),
        ]
    }
}
```

Conecta el puente una vez por cada tipo Broadcastable, en el arranque:

```rust
EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
```

Después de eso, `EventFacade::dispatch(event).await?` es todo el
lado del envío - sin ninguna llamada separada a `publish`.

Por defecto el evento se serializa mediante
`serde_json::to_value(&event)` y se envía a cada suscriptor. Los
canales sin suscriptores se omiten en silencio en el hub en proceso;
el hub entre procesos de todos modos los publica para que otros
procesos tengan la oportunidad de entregarlos.

Cuatro métodos opcionales refinan el comportamiento por defecto:

**`broadcast_event_name(&self) -> &'static str`** - anula el nombre
del evento en la red. Por defecto es `Self::event_name()`. Úsalo
para desacoplar la identidad del evento en proceso del nombre en la
red.

**`broadcast_with(&self) -> Option<Value>`** - devuelve `Some(value)`
para enviar un payload curado en lugar de la serialización completa
del evento (el `broadcastWith()` de Laravel). Omite secretos o
reestructura para el cliente sin cambiar el tipo del evento:

```rust
impl Broadcastable for AccountFunded {
    fn broadcast_on(&self) -> Vec<String> {
        vec![format!("account.{}", self.account_id)]
    }
    fn broadcast_with(&self) -> Option<serde_json::Value> {
        // Nunca pongas el balance en la red - solo el id público.
        Some(serde_json::json!({ "account_id": self.account_id }))
    }
}
```

**`broadcast_when(&self) -> bool`** - devuelve `false` para despachar
el evento a los oyentes en proceso pero omitir el envío por
WebSocket (el `broadcastWhen()` de Laravel). Solo la difusión se
controla; el resto del pipeline del evento se ejecuta sin cambios:

```rust
impl Broadcastable for DraftSaved {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("doc.{}", self.doc_id)] }
    fn broadcast_when(&self) -> bool { self.publish } // solo difunde al publicar
}
```

**`broadcast_to_others(&self) -> bool`** - devuelve `true` para
excluir la conexión que disparó la difusión (el `toOthers()` de
Laravel). El framework asigna a cada conexión de difusión un
`socket_id` al conectar (enviado en el frame `connected`); el
navegador lo repite como el encabezado `X-Socket-ID` en las
solicitudes HTTP; un evento `broadcast_to_others` despachado mientras
se maneja esa solicitud omite la conexión de origen. Fuera de una
solicitud (un worker o un job) o cuando no hay ningún `X-Socket-ID`
presente, degrada a difundir a todos:

```rust
impl Broadcastable for MessagePosted {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("chat.{}", self.room)] }
    fn broadcast_to_others(&self) -> bool { true } // el remitente ya lo tiene
}
```

Esta es una elección por tipo de evento. Para exclusión por
despacho, publica directamente:

```rust
use suprnova::broadcasting::BroadcastEnvelope;

hub.publish(
    BroadcastEnvelope::new(channel, event, data).with_except(socket_id),
).await?;
```

### Orden de despacho con oyentes hermanos

`EventFacade::dispatch` es **fail-fast**: si un publish del hub
devuelve `Err` (por ejemplo, una desconexión del broker en un hub
entre procesos), el `BroadcastListener` devuelve `Err` y cualquier
oyente hermano registrado **después** de él no se ejecuta. Dos formas
de manejar esto:

- Registra el puente de difusión DESPUÉS de los oyentes en proceso
  cuyos efectos secundarios (escrituras en la BD, emisión de logs)
  deben ejecutarse sin importar el resultado de la difusión.
- Cambia a `EventFacade::dispatch_best_effort(event)` cuando cada
  oyente debe ejecutarse sin importar que uno devuelva `Err`.

Los hubs en memoria nunca devuelven `Err` - solo la variante entre
procesos expone los fallos del broker.

## El protocolo en la red

Cada mensaje sobre la ruta de difusión es un frame JSON en UTF-8.
Dos formas: `ClientFrame` (cliente → servidor) y `ServerFrame`
(servidor → cliente).

### Frames del cliente

| `action` | Campos obligatorios | Campos opcionales | Significado |
|----------|-----------------|-----------------|---------|
| `subscribe` | `channel` | `data` | Se suscribe a `channel`. `data` se reenvía a `Channel::authorize`. |
| `unsubscribe` | `channel` | | Se desconecta de `channel`. |
| `publish` | `channel`, `event`, `data` | | Envía un evento a cada suscriptor en `channel`. Controlado por `Channel::authorize_publish` Y requiere una suscripción activa. |

El `publish` iniciado por el cliente está controlado por **dos**
comprobaciones: la conexión DEBE tener una suscripción autorizada al
canal objetivo, Y `Channel::authorize_publish` debe devolver `true`
(por defecto es `false`). Esto refleja el contrato de client-event de
Pusher - los canales que quieren publishes de cliente participan
explícitamente anulando el hook. La mayoría de los canales de
difusión del lado del servidor nunca quieren eventos iniciados por el
cliente, y la forma de denegar por defecto refleja esa intención.

```json
{"action":"subscribe","channel":"chat.42","data":{"token":"abc"}}
{"action":"unsubscribe","channel":"chat.42"}
{"action":"publish","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
```

### Frames del servidor

| `action` | Campos | Significado |
|----------|--------|---------|
| `connected` | `socket_id` | Se envía una vez, primero. Repite `socket_id` como el encabezado HTTP `X-Socket-ID` para que el `broadcast_to_others` del lado del servidor pueda excluir esta conexión. |
| `subscribed` | `channel` | Suscripción aceptada. |
| `unsubscribed` | `channel` | Cancelación de suscripción confirmada. |
| `event` | `channel`, `event`, `data` | Se difundió un evento en `channel`. |
| `lagged` | `channel`, `skipped` | El suscriptor se quedó atrás respecto al ring buffer por canal del servidor y se descartaron `skipped` sobres en esta conexión. El estado local del cliente sobre `channel` está obsoleto; vuelve a consultar antes de procesar más eventos. |
| `error` | `channel` (nullable), `reason` | La última acción falló. `channel` es `null` para errores a nivel de sobre que no están ligados a un canal. |

```json
{"action":"connected","socket_id":"6f1a3c2e-…"}
{"action":"subscribed","channel":"chat.42"}
{"action":"unsubscribed","channel":"chat.42"}
{"action":"event","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
{"action":"lagged","channel":"chat.42","skipped":42}
{"action":"error","channel":"chat.42","reason":"unauthorized"}
{"action":"error","channel":null,"reason":"malformed envelope: …"}
```

#### Sobre `lagged`

Cada canal tiene un ring buffer por proceso (256 sobres). Un
suscriptor que no drena lo bastante rápido - un cliente lento, un
forwarder atascado - se queda atrás, y el buffer sobrescribe los
eventos más antiguos. Cuando eso ocurre, el servidor envía un frame
`lagged` nombrando el canal y la cantidad de eventos descartados, y
luego continúa entregando los frames siguientes normalmente. El
hueco **no** es recuperable desde el lado del servidor; el cliente
debe volver a consultar o resincronizar antes de procesar más
eventos en ese canal. Descartar eventos en silencio dejaría que los
bugs se escondieran como "perdimos un tick" en lugar de "el estado
del cliente divergió del estado del servidor".

#### Fallos de publicación

Cuando un `publish` iniciado por el cliente es aceptado por
`authorize_publish` pero el publish del hub en sí falla
(desconexión del broker en el hub entre procesos), el cliente de
origen recibe un frame `error` con `reason: "publish failed: …"` para
que sepa que el evento no alcanzó a otros procesos. Los demás
suscriptores no son notificados.

### Sesión de ejemplo

```
S → C  {"action":"connected","socket_id":"6f1a3c2e-…"}
C → S  {"action":"subscribe","channel":"order.updates","data":{}}
S → C  {"action":"subscribed","channel":"order.updates"}

# El servidor despacha OrderPlaced:
S → C  {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}

C → S  {"action":"subscribe","channel":"chat.private","data":{"token":"bad"}}
S → C  {"action":"error","channel":"chat.private","reason":"unauthorized"}

C → S  {"action":"unsubscribe","channel":"order.updates"}
S → C  {"action":"unsubscribed","channel":"order.updates"}
```

## Middleware por ruta

Las rutas de difusión admiten el mismo encadenamiento
`.middleware(M)` que las rutas WebSocket normales:

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new()),
```

Una respuesta que no es 2xx desde cualquier middleware hace
cortocircuito en la actualización - el cliente recibe la respuesta de
error HTTP y no ocurre ningún handshake de WebSocket. Este es el
lugar correcto para aplicar la autenticación a nivel de transporte
(validez de sesión, comprobaciones de origen, límites de velocidad en
el momento de la conexión) sin duplicar la comprobación dentro del
`authorize` de cada canal.

Varios middleware se componen de izquierda a derecha:

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

La división es intencional: lo **de nivel de transporte** (quién
puede abrir la conexión en absoluto) vive en el middleware; lo **de
nivel de canal** (quién puede suscribirse a qué canal) vive en
`Channel::authorize`.

### `WsConfig` por ruta

Anula los valores por defecto de WebSocket de todo el proceso, por
ruta. Encadena `.config(WsConfig { ... })` después del handler -
antes o después de `.middleware(M)` (el orden no importa):

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

ws!("/ws/chat", broadcasting_handler())
    .config(WsConfig {
        ping_interval: Duration::from_secs(5),
        max_missed_pings: 1,
        ..Default::default()
    })
    .middleware(AuthMiddleware::new())
```

Los cinco campos configurables y dónde importa cada uno:

| Campo | Por defecto | Caso de uso |
|-------|---------|----------|
| `ping_interval` | 30s | Chat / presencia: acórtalo a 5-10s para detectar rápido conexiones móviles muertas. Streaming de datos masivos: alárgalo para reducir el overhead. |
| `max_missed_pings` | 2 | Ponlo en `1` para chat, donde un Pong perdido debería cerrar de inmediato. Ponlo en `3` o más para redes móviles inestables. Ponlo en `usize::MAX` para desactivar el cierre sin pong. |
| `max_message_size` | 1 MiB | Valor por defecto seguro para endpoints públicos. Parte de `WsConfig::generous()` (64 MiB) para feeds internos de confianza. |
| `max_frame_size` | 64 KiB | Dimensionado para frames de chat / notificación con margen. Parte de `WsConfig::generous()` (16 MiB) para frames grandes sin fragmentar. |
| `origin_policy` | `SameOrigin` | Por defecto rechaza las actualizaciones cross-origin - la única protección CSRF que tiene un handshake de WS de navegador. Usa `AllowList(vec![...])` para frontends cross-origin explícitos, o `AllowAny` solo para endpoints que no son de navegador. |

Cuando no se suministra ningún `.config(...)`, la ruta hereda
`WsConfig::default()`. La configuración explícita por ruta siempre
gana sobre el valor por defecto.

Para rutas que sirven feeds internos de confianza (dispersión
servidor-a-servidor, transferencias binarias grandes), parte de la
factory de feed de confianza y ajusta según lo necesites:

```rust
use suprnova::ws::WsConfig;
use std::time::Duration;

ws!("/ws/internal/firehose", FirehoseHandler::new())
    .config(WsConfig {
        ping_interval: Duration::from_secs(10),
        ..WsConfig::generous() // mensaje de 64 MiB / frame de 16 MiB
    })
```

## Presencia

Cuando un cliente se suscribe exitosamente a un canal de presencia,
el hub:

1. Llama a `PresenceChannel::member_info` con el `Request` de la
   actualización y el `ChannelParams` capturado para recolectar los
   datos del miembro que se une.
2. Envía un frame de evento `presence.here` al nuevo suscriptor con
   `data: { "members": [...] }` - una instantánea de todos los
   miembros rastreados actualmente (excluyendo al que recién se
   une).
3. Publica un evento `presence.joined` con `data: <member_info>` en
   el canal. Cada suscriptor - incluyendo al nuevo, a través de su
   propio forwarder - lo recibe; los clientes filtran el self-join
   comparando la identidad del miembro que se une con la suya
   propia.

Cuando un suscriptor se desconecta o envía un frame de unsubscribe:

4. El hub publica un evento `presence.left` con los datos del
   miembro que se va. Cada suscriptor restante lo recibe.

Los tres frames llegan como frames de acción `event` con nombres de
`event` reservados:

```json
{"action":"event","channel":"presence.lobby","event":"presence.here","data":{"members":[{"user_id":1},{"user_id":2}]}}
{"action":"event","channel":"presence.lobby","event":"presence.joined","data":{"user_id":3}}
{"action":"event","channel":"presence.lobby","event":"presence.left","data":{"user_id":3}}
```

Entre procesos, el estado de presencia se replica a través del
meta-canal reservado `__presence__` (ver [Dispersión entre
procesos](#dispersión-entre-procesos)). Las operaciones de track y
untrack en cualquier proceso se propagan a todos los suscriptores;
`list_members` devuelve la vista combinada (local + remota). A los
procesos muertos cuyo `untrack_member` nunca se disparó se les
depuran sus miembros mediante TTL - 60 s por defecto.

## Dispersión entre procesos

El `InMemoryBroadcastHub` por defecto solo dispersa hacia los
suscriptores del proceso actual. Para despliegues con varias réplicas,
activa la feature de Cargo `broadcasting-fanout` y cambia a
`SeaStreamerBroadcastHub`:

`Cargo.toml`:

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.1", features = ["broadcasting-fanout"] }
```

`src/bootstrap.rs`:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry};
use suprnova::broadcasting::fanout::SeaStreamerBroadcastHub;
use suprnova::container::App;

pub async fn register() {
    let hub: Arc<dyn BroadcastHub> = Arc::new(
        SeaStreamerBroadcastHub::new(
            "redis://broker:6379",   // URI del streamer (el backend se elige por el esquema)
            "suprnova-broadcast",    // clave de stream (compartida por todo proceso del clúster)
        )
        .await
        .expect("connect"),
    );
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));
    // ... el resto del bootstrap sin cambios
}
```

El constructor toma dos argumentos: la URI del streamer (selecciona el
backend en tiempo de ejecución por el esquema) y la clave de stream (el
nombre del topic compartido por todos los procesos del clúster). Usa la
misma clave de stream en cada réplica o no verán los eventos de las
demás.

`new_with_presence_ttl(uri, key, ttl)` sobrescribe el TTL de presencia
de 60 s por defecto - útil para tests que necesitan ejercitar
rápidamente la ruta de recuperación tras una caída.
`new_loopback(uri, key)` habilita el loopback por stdio para tests de
integración de un solo proceso; la salvaguarda de duplicados garantiza
que cada evento de la app se sigue entregando exactamente una vez en
local.

### Backends

El backend se selecciona en tiempo de ejecución a partir del esquema de
la URI:

| Esquema de URI | Backend | Listo para producción | Notas |
|------------|---------|------------------|-------|
| `redis://`, `rediss://` | Redis Streams | **Sí** | La recomendación por defecto. `rediss://` usa TLS. Activado en el build por defecto. |
| `kafka://`, `kafka+ssl://` | Kafka | **Sí** | Requiere `kafka` en el conjunto de features de `sea-streamer` (`framework/Cargo.toml`). |
| `stdio://` | tuberías de stdin/stdout | No - solo para tests | Loopback de un solo proceso. |
| `file://` | Archivo local | No - un solo host | Requiere `file` en el conjunto de features de `sea-streamer`. |

El build por defecto de Suprnova activa `stdio` + `redis` + `socket`.
Para activar Kafka o file, edita `framework/Cargo.toml` y añade la
feature de `sea-streamer` correspondiente.

### Arquitectura

Cada `publish(envelope)` hace dos cosas en paralelo:

1. **Dispersión local** - el `InMemoryBroadcastHub` interno entrega de
   inmediato a los suscriptores de este proceso. Los suscriptores
   locales nunca esperan a la red.
2. **Escritura en el stream** - el mismo sobre se serializa y se empuja
   al stream de sea-streamer, de modo que el bucle de consumo de cada
   uno de los demás procesos lo recoge y lo entrega en local.

Una salvaguarda de entrega duplicada evita ver dos veces cada evento de
datos de la app: la instancia del hub tiene un UUID aleatorio, cada sobre
que produce lleva ese UUID, y el bucle de consumo se salta los sobres
entrantes cuyo id de instancia coincide con el del propio hub local. Los
mensajes del meta-canal de presencia son una excepción - cada hub
necesita sus propios eventos en la vista entre procesos para que la ruta
de lectura quede unificada.

El despacho de backend se basa en enum, no en objeto de trait: el hub
guarda un `SeaProducer` / `SeaConsumer` concreto del adaptador de socket
de sea-streamer, que es un enum sobre todos los backends compilados. Sin
sobrecarga de `dyn` en el punto de llamada a publish.

### Presencia entre procesos

`SeaStreamerBroadcastHub` replica el estado de presencia entre procesos
de forma automática. Cada instancia tiene un `instance_id` UUID desde su
construcción; `track_member` / `untrack_member` publican `PresenceEvent`
en el meta-canal reservado `__presence__`. Cada proceso mantiene una
`cross_process_view` que actualiza su tarea de consumo; `list_members`
devuelve la vista fusionada (local y remota de forma uniforme).

Vivacidad: cada proceso vuelve a publicar sus miembros cada `ttl / 6`
(10 s con el TTL de 60 s por defecto) como heartbeat. Las entradas
obsoletas - los miembros cuyo `last_seen` supera el TTL - se podan cada
`ttl / 2`. Esto cubre las caídas de proceso que no llegaron a publicar
`MemberRemoved`.

## Cierre sin pong

Las rutas de difusión participan en el mismo heartbeat de WebSocket
que las rutas `ws!` normales. El framework envía un Ping cada
`WsConfig::ping_interval` (30 s por defecto). Si una conexión no
responde con un Pong dentro de `max_missed_pings` intervalos
consecutivos (2 por defecto), el framework cierra con código 1011.

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let config = WsConfig {
    ping_interval: Duration::from_secs(15),
    max_missed_pings: 3,
    ..WsConfig::default()
};
```

Bajar `ping_interval` detecta conexiones muertas más rápido a costa
de un tráfico base más alto. `max_missed_pings: 1` cierra después
del primer Pong perdido - úsalo solo cuando los fallos de red son
raros y quieres la limpieza de conexiones muertas más rápida
posible. `max_missed_pings: usize::MAX` desactiva por completo el
cierre sin pong.

## Despliegue en producción

Las rutas de difusión son conexiones HTTP actualizadas sobre el
mismo listener de hyper que tus rutas HTTP. La terminación de TLS
ocurre en un punto anterior, exactamente como se describe en [el
capítulo de WebSocket](websockets.md#production-deployment). Las
configuraciones de nginx y Caddy de ese capítulo aplican sin cambios -
extiéndelas para cubrir la ruta `/ws/broadcast`.

Las tareas de handler de WebSocket activas (incluidas las conexiones
de difusión) se rastrean en el conjunto `WS_TASKS` del framework y
se drenan en el apagado ordenado, así que las entregas de eventos en
vuelo se completan antes de que el proceso termine.

## Pruebas de difusión

`RecordingBroadcastHub` es el análogo en Suprnova del
`Broadcast::fake()` de Laravel - un `BroadcastHub` que registra cada
sobre publicado mientras sigue entregando a los suscriptores
activos. Vincúlalo en lugar de `InMemoryBroadcastHub` en un test y
haz aserciones sobre lo que se difundió sin suscribirte primero:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, RecordingBroadcastHub};
use suprnova::container::App;

#[tokio::test]
async fn shipping_an_order_broadcasts_to_the_user_channel() {
    let hub = Arc::new(RecordingBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub) as Arc<dyn BroadcastHub>);

    // ... ejecuta código que publica (directamente, o vía un Broadcastable despachado) ...

    hub.assert_broadcast("orders.42", "OrderShipped");
    assert_eq!(hub.count(), 1);
}
```

| Helper                         | Verifica                                                  |
|--------------------------------|----------------------------------------------------------|
| `assert_broadcast(ch, ev)`     | que hay al menos un sobre en `ch` con el nombre de evento `ev` |
| `assert_nothing_broadcast()`   | que no se publicó nada                                    |
| `broadcasts()`                 | `Vec<BroadcastEnvelope>` - cada sobre registrado          |
| `count()`                      | la cantidad total de sobres registrados                   |

Para verificar que un *evento* `Broadcastable` se despachó en
absoluto (en lugar de lo que llegó a la red), `EventFacade::fake()`
registra el evento en sí - ver
[Eventos](events.md#testing--eventfacadefake).

## Matriz de paridad con Laravel

| Laravel | Suprnova |
|---------|----------|
| `Broadcast::channel('name', fn(...))` | `Channel` trait impl + `registry.register(...)` |
| `Broadcast::channel('orders.{id}', ...)` | `fn name() -> "orders.{id}"`, params en `ChannelParams` |
| `PrivateChannel` (interface) | trait marcador `PrivateChannel` + anular `authorize` |
| `PresenceChannel` (interface) | `PresenceChannel` + anular `Channel::presence_info` |
| `ShouldBroadcast` (interface) | trait `Broadcastable` |
| `broadcastOn()` | `broadcast_on(&self) -> Vec<String>` |
| `broadcastAs()` | `broadcast_event_name(&self) -> &'static str` |
| `broadcastWith()` | `broadcast_with(&self) -> Option<Value>` |
| `broadcastWhen()` | `broadcast_when(&self) -> bool` |
| `toOthers()` | `broadcast_to_others(&self) -> bool` |
| `Broadcast::fake()` | `RecordingBroadcastHub` vinculado como `dyn BroadcastHub` |
| `assertBroadcasted` | `RecordingBroadcastHub::assert_broadcast(channel, event)` |
| Driver Pusher / Reverb / Ably | `InMemoryBroadcastHub` (un solo proceso) o `SeaStreamerBroadcastHub` (entre procesos: Redis / Kafka / file / stdio) |
| Biblioteca cliente Echo | no se incluye - por ahora, conecta el protocolo de sobre JSON desde el navegador a mano |

## Referencia

| Símbolo | Propósito |
|--------|---------|
| `suprnova::broadcasting::Channel` | Trait de canal. Anula `name()` (obligatorio), `authorize`, `authorize_publish`, `presence_info`. |
| `suprnova::broadcasting::ChannelParams` | Valores capturados de un `name()` parametrizado. `get(key) -> Option<&str>`. Vacío para nombres fijos. |
| `suprnova::broadcasting::PrivateChannel` | Trait marcador sobre un `Channel` que anula `authorize`. Sin métodos obligatorios. |
| `suprnova::broadcasting::PresenceChannel` | `async fn member_info(req, params) -> Result<Value, FrameworkError>`. Requiere anular `Channel::presence_info`. |
| `suprnova::broadcasting::ChannelRegistry` | Contiene cada canal registrado. Se vincula como `Arc<ChannelRegistry>` en el contenedor; lo resuelve `BroadcastingWsHandler`. |
| `suprnova::broadcasting::Broadcastable` | Trait sobre `Event + Serialize`. Obligatorio: `broadcast_on()`. Opcional: `broadcast_event_name`, `broadcast_with`, `broadcast_when`, `broadcast_to_others`. |
| `suprnova::broadcasting::BroadcastHub` | Trait de hub. `subscribe`, `publish`, `subscriber_count`, presence track/untrack/list. |
| `suprnova::broadcasting::InMemoryBroadcastHub` | Hub en proceso por defecto. Sin dependencias externas. Publish devuelve `Ok` incondicionalmente. |
| `suprnova::broadcasting::RecordingBroadcastHub` | Doble de test. Registra cada publish; de todos modos entrega a los suscriptores activos. |
| `suprnova::broadcasting::BroadcastEnvelope` | Un evento publicado: `channel`, `event`, `data`, `except`. Builder `new(ch, ev, data)`; `.with_except(socket_id)` para exclusión por despacho. |
| `suprnova::broadcasting::ClientFrame` / `ServerFrame` | Los tipos en la red del sobre JSON. `ServerFrame::Lagged { channel, skipped }` expone los desbordamientos del ring buffer por canal. |
| `suprnova::broadcasting::BroadcastingWsHandler` | El `WebSocketHandler` reutilizable del framework. Constructor: `BroadcastingWsHandler::new(hub, registry)`. Pásalo a `ws!()`. |
| `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub` | Hub entre procesos detrás de `broadcasting-fanout`. `new(uri, stream_key)`, `new_with_presence_ttl(uri, key, ttl)`, `new_loopback(uri, key)`. |
| `EventFacade::broadcast::<E>(hub)` | Registra el puente evento → hub para `E`. Llámalo una vez por cada `Broadcastable`, en el arranque. |
| `EventFacade::dispatch(event)` | Dispara los oyentes en proceso Y publica en el hub en cada canal que devuelve `E::broadcast_on()`. |
| `WsRouteDef::config(WsConfig)` | Anulación de configuración de WS por ruta. Se compone con `.middleware(M)` en cualquier orden. |
| `WsRouteDef::middleware(M)` | Cadena de middleware por ruta. Una respuesta que no es 2xx hace cortocircuito en la actualización. |
| `WsConfig::generous()` | Factory de feed de confianza: mensaje de 64 MiB / frame de 16 MiB, el resto de los campos sin cambios. NO la uses en rutas públicas. |

## Siguiente

- [WebSockets](websockets.md) - la primitiva subyacente, `WsSocket`,
  `OriginPolicy`
- [Eventos](events.md) - `EventFacade`, despacho fail-fast frente a
  best-effort
- [Eventos enviados por el servidor](sse.md) - push unidireccional
  sin handshake de Upgrade
- [Notificaciones](notifications.md) - el driver de notificación
  `BroadcastChannel`
- [Web Push](web-push.md) - notificaciones enviadas por el servidor
  a usuarios sin conexión

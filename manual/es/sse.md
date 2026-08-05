# Eventos enviados por el servidor

Server-Sent Events (SSE) es el canal push mínimo y unidireccional de
servidor a navegador: el navegador abre `EventSource(url)`, el servidor
mantiene abierta una respuesta `text/event-stream`, y empuja eventos
enmarcados a medida que ocurren. Sin handshake de WebSocket, sin
permessage-deflate, sin bibliotecas de framing - solo líneas `data:`,
`event:`, `id:`, `retry:` terminadas por una línea en blanco, según la
especificación
[WHATWG `EventSource`](https://html.spec.whatwg.org/multipage/server-sent-events.html).

La primitiva SSE de Suprnova se conecta a la ruta del cuerpo en
streaming: construye un `Stream<Item = SseEvent>`, entrégalo a
`HttpResponse::sse(...)`, y el framework se encarga de la gestión de la
conexión, el framing, los encabezados y el aislamiento de pánicos. La
conexión permanece abierta hasta que el stream productor termina o el
cliente se desconecta.

## Cuándo recurrir a SSE frente a WebSockets

| Propiedad | SSE | WebSockets |
|----------|-----|------------|
| Dirección | Servidor → navegador | Bidireccional |
| Transporte | HTTP/1.1 o HTTP/2 sin más | Solo mediante upgrade |
| Reconexión | Automática, con `retry:` y `Last-Event-ID` | Manual |
| Proxies / CDN | Funciona en cualquier cosa que permita respuestas HTTP largas | A menudo necesita soporte explícito de Upgrade |
| API del navegador | `EventSource` (incorporada) | `WebSocket` (incorporada) |
| Frames binarios | Solo texto (UTF-8) | Texto o binario |
| Límite de conexiones por pestaña | 6 (HTTP/1.1) / sin límite (HTTP/2) | Sin límite |

Recurre a SSE cuando solo necesites push de servidor a cliente (feeds
de actividad, notificaciones, colas de logs, streaming de IA). Recurre
a [WebSockets](websockets.md) cuando necesites tráfico bidireccional o
frames binarios.

## Inicio rápido

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub async fn stream_ticks(_req: Request) -> Response {
    let (tx, rx) = mpsc::channel::<SseEvent>(16);
    tokio::spawn(async move {
        for i in 0..10 {
            let evt = SseEvent::data(format!("tick {i}"))
                .with_event("tick")
                .with_id(i.to_string());
            if tx.send(evt).await.is_err() {
                break; // el cliente se desconectó
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Ok(HttpResponse::sse(ReceiverStream::new(rx)))
}
```

Salida en la red para un tick:

```text
event: tick
id: 0
data: tick 0

```

El navegador analiza esto y dispara un evento `tick` con
`evt.data === "tick 0"` y `evt.lastEventId === "0"`.

## La API de `SseEvent`

`SseEvent` es el tipo que empujas hacia el stream. Tiene dos tipos:

* **Frame** - un evento normal con `event` / `id` / `retry` opcionales
  y un payload `data` de varias líneas. Se construye mediante
  [`SseEvent::data`](#constructores), `SseEvent::json`, o
  `SseEvent::error`.
* **Comment** - un keep-alive que solo existe en la red (`:\n\n` o
  `: <texto>\n\n`). Se construye mediante `SseEvent::comment(text)` o
  `SseEvent::keep_alive()`. El navegador ignora los comments según la
  especificación; son los bytes que atraviesan la conexión los que
  evitan que proxies y balanceadores de carga inactivos la cierren.

### Constructores

| Constructor | Produce | Uso |
|-------------|----------|-----|
| `SseEvent::data(text)` | Frame con solo líneas `data:` | El evento mínimo |
| `SseEvent::json(event, &payload)` | Frame con `event:` + `data:` en JSON | El caso del 95% - `JSON.parse(evt.data)` en el cliente |
| `SseEvent::error(message)` | Frame con `event: error` | Evento de error de dominio, distinto del `error` de nivel de conexión que el navegador dispara ante un fallo de transporte |
| `SseEvent::comment(text)` | Comment | Keep-alive con una marca que el operador puede detectar en los logs |
| `SseEvent::keep_alive()` | Comment vacío (`:\n\n`) | El heartbeat canónico de bytes mínimos |

### Builders

| Builder | Efecto | En `Comment` |
|---------|--------|--------------|
| `.with_event(name)` | Establece el campo `event:` | No-op silencioso |
| `.with_id(id)` | Establece el campo `id:` - necesario para la semántica de reanudación | No-op silencioso |
| `.with_retry(Duration)` | Establece el campo `retry:` (ms); la especificación dice que `Duration::ZERO` significa "reconectar de inmediato" | No-op silencioso |
| `.try_with_event(name)` | Variante falible - ver [Contrato de seguridad](#contrato-de-seguridad) | `Ok(self)` sin cambios |
| `.try_with_id(id)` | Variante falible de `with_id` | `Ok(self)` sin cambios |

Los builders sobre `Comment` son no-ops a propósito - el formato en la
red no tiene forma de expresar "comment con un nombre de evento". Un
mal uso permanece silencioso en lugar de convertir el evento en un
frame y sorprender al productor.

### Accesores

| Método | Devuelve |
|--------|---------|
| `.event()` | `Option<&str>` - el nombre del evento, si está definido |
| `.id()` | `Option<&str>` - el last-event-id, si está definido |
| `.retry()` | `Option<Duration>` - el retardo de reconexión, si está definido |
| `.payload()` | `&str` - el payload de `data:` (o `""` para `Comment`) |
| `.is_comment()` | `bool` |
| `.comment_text()` | `Option<&str>` - el texto del comment, si esto es un `Comment` |

### Codificación en la red

`SseEvent::to_wire()` serializa el evento a `Bytes` listos para el
stream del cuerpo:

**Frame:**

```text
event: <event>\n   (solo si es Some)
id: <id>\n         (solo si es Some)
retry: <ms>\n      (solo si es Some)
data: <line>\n     (una por línea del payload, tras normalizar \r/\r\n)
\n                 (terminador - exigido por la especificación)
```

**Comment:**

```text
: <line>\n         (una por línea del texto del comment; `:\n` para líneas vacías)
\n                 (límite de flush)
```

## Contrato de seguridad

El formato en la red de SSE usa CR / LF / NUL como terminadores de
campo sin ningún mecanismo de escape. Un productor que deja que la
entrada del usuario llegue a `event:` o `id:` sin sanitizar expondría
una vulnerabilidad de inyección de campo - un valor como
`"legit\ndata: injected"` produciría dos campos `data:` en la red, y
`"legit\n\nevent: spoofed"` terminaría el evento actual y comenzaría
uno nuevo.

El `to_wire()` de Suprnova se defiende en dos capas:

* **Los valores de los campos `event:` y `id:`** - cada CR / LF / NUL
  se elimina en el momento de serializar. Se dispara un `WARN`
  estructurado por cada eliminación: `target: "suprnova::sse"`,
  `field = "event"|"id"`. El warn nunca registra el valor - esos bytes
  están controlados por el atacante por construcción.
* **El texto de `data:` y de los comments** - `\r\n` y `\r` sueltos se
  normalizan a `\n` antes de dividir, así que un productor que
  incrusta `\r` en un payload no puede hacer que el parser del
  receptor sintetice un campo `data:` / `event:` / `id:` en el momento
  de analizar. El NUL se elimina del texto del comment con un `WARN`
  equivalente.

Si quieres **fallar rápido** ante una entrada incorrecta en lugar de
eliminarla en silencio, recurre a las contrapartes `try_with_*`:

```rust
use suprnova::{Response, sse::SseEvent};

let evt = SseEvent::data("hello")
    .try_with_event(&user_supplied_event)?     // devuelve Err ante CR/LF/NUL
    .try_with_id(&user_supplied_id)?;
```

El `FrameworkError::validation(field, ...)` devuelto nombra el campo;
NO repite el valor de vuelta, así que un 400 mostrado al cliente es
seguro de registrar.

## Keep-alive y tiempos de espera por inactividad de los proxies

Las conexiones SSE de larga duración son silenciosas por defecto. La
mayoría de los despliegues de producción se sitúan detrás de un proxy
/ balanceador de carga / CDN que cierra las conexiones inactivas para
liberar recursos:

* nginx por defecto: 60 segundos
* AWS ALB por defecto: 60 segundos
* Cloudflare por defecto: 100 segundos

Un comment de `keep_alive()` cada 15-30 segundos mantiene la conexión
viva a través de todos ellos sin despachar un evento `message` al
navegador. La forma de bytes mínimos (`:\n\n`) basta para vaciar los
búferes de escritura del proxy sin enviar ningún payload.

```rust
use std::time::Duration;
use futures::StreamExt;
use suprnova::sse::SseEvent;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

let (tx, rx) = mpsc::channel::<SseEvent>(16);

// Tarea de heartbeat - independiente del productor de eventos.
let hb_tx = tx.clone();
tokio::spawn(async move {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        if hb_tx.send(SseEvent::keep_alive()).await.is_err() {
            break; // el cliente se fue
        }
    }
});

// El productor de eventos ... envía frames a `tx` a medida que ocurren.
```

## Reanudación tras una caída (`Last-Event-ID`)

Cuando el `EventSource` del navegador pierde la conexión, se reconecta
automáticamente y envía el `id:` más reciente que vio como el
encabezado `Last-Event-ID` en la nueva solicitud. Etiqueta cada evento
con `.with_id(...)` y lee el encabezado en la solicitud de reanudación:

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::{self, SseEvent}};

pub async fn stream_from_resume(req: Request) -> Response {
    let resume_from: u64 = sse::last_event_id(&req)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Construye el stream productor a partir de `resume_from + 1`. El
    // closure posee su propio contador interno, así que la mutación
    // permanece dentro del stream.
    let stream = futures::stream::iter(events_since(resume_from))
        .scan(resume_from + 1, |next_id, payload| {
            let id = *next_id;
            *next_id += 1;
            futures::future::ready(Some((id, payload)))
        })
        .map(|(id, payload)| {
            SseEvent::json("activity", &payload)
                .expect("payload is a Serialize value")
                .with_id(id.to_string())
        });

    Ok(HttpResponse::sse(stream))
}
```

`sse::last_event_id(&Request) -> Option<String>` devuelve `None`
cuando el encabezado está ausente O cuando el valor contiene un byte
NUL (según la especificación WHATWG, un NUL invalida un last-event-id
y el parser del navegador lo descartaría). El `String` devuelto es,
por lo demás, una entrada de usuario opaca - analízalo como tu propio
cursor / secuencia / offset antes de usarlo.

## Errores de dominio

`SseEvent::error("...")` produce la forma convencional
`event: error\ndata: <msg>\n\n`. Los suscriptores pueden escucharlo
por separado del `error` de nivel de conexión que el navegador dispara
ante un fallo de transporte:

```js
const es = new EventSource("/stream");

// Errores de conexión / transporte (sin `data`).
es.onerror = (evt) => console.warn("transport error", evt);

// Errores de dominio emitidos por SseEvent::error(...).
es.addEventListener("error", (evt) => console.error("server-side:", evt.data));
```

Al mapear un `Stream<Item = Result<T, E>>` a un `Stream<Item = SseEvent>`,
el patrón idiomático es `map(|r| match r { Ok(x) => SseEvent::json(...), Err(e) => SseEvent::error(...) })` -
el mapeo del error del lado del consumidor permanece en manos del
productor, y el framework nunca tiene que inventar una forma por
defecto.

## Difundir un solo stream a muchos suscriptores

La dispersión hacia muchos suscriptores de SSE ya está cubierta por el
[subsistema de difusión](broadcasting.md): suscríbete a un canal de
`BroadcastHub` y adapta el `broadcast::Receiver` al stream de
`SseEvent` con `tokio_stream::wrappers::BroadcastStream` + `.map(...)`.
Cada conexión obtiene su propio receiver; el hub gestiona la política
para consumidores lentos (errores `Lagged(n)` cuando un suscriptor se
queda atrás) y tú decides cómo mostrárselo al cliente.

El ejemplo dogfood funcional en `app/src/controllers/sse_example.rs`
implementa esto en ~25 líneas:

```rust
use futures::StreamExt;
use std::sync::Arc;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio_stream::wrappers::BroadcastStream;

pub async fn stream(_req: Request) -> Response {
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .expect("BroadcastHub not bootstrapped");
    let rx = hub.subscribe("user_registered");

    let stream = BroadcastStream::new(rx).map(|result| match result {
        Ok(envelope) => SseEvent::json("user.registered", &envelope.data)
            .unwrap_or_else(|_| {
                SseEvent::data(envelope.data.to_string())
                    .with_event("user.registered")
            }),
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
            SseEvent::data(n.to_string()).with_event("lagged")
        }
    });

    Ok(HttpResponse::sse(stream))
}
```

El evento `lagged` permite que el cliente dispare una recarga completa
y una reanudación - la conexión permanece abierta durante el retraso.

## Configuración de producción

### Encabezados de la respuesta

`HttpResponse::sse(...)` establece los encabezados necesarios por ti:

| Encabezado | Valor | Por qué |
|--------|-------|-----|
| `Content-Type` | `text/event-stream` | Definido por la especificación; el `EventSource` del navegador lo exige |
| `Cache-Control` | `no-cache` | Evita que los intermediarios almacenen en caché el stream |
| `Connection` | `keep-alive` | Respuesta HTTP/1.1 de larga duración |
| `X-Accel-Buffering` | `no` | Desactiva el buffering del proxy nginx - los eventos se vacían de inmediato. No-op fuera de nginx |

### Ajustar la reconexión

El retardo de reconexión por defecto del navegador es de 3 segundos.
Envía un campo `retry:` una vez al inicio del stream para anularlo:

```rust
let preamble = SseEvent::data("ready").with_retry(Duration::from_secs(5));
```

`Duration::ZERO` es válido según la especificación ("reconectar de
inmediato") y se emite literalmente - sin coerción. Para streams de
producción, un retry de 5-15 segundos logra un equilibrio entre una
recuperación rápida y no bombardear al servidor durante una caída
regional.

### Por qué Suprnova diverge

Laravel entrega SSE como un helper puntual sobre `Response`:
`Response::eventStream(fn () => ...)` toma un closure que produce un
generador y enmarca cada valor producido como una línea `data:`. No
modela `event:` / `id:` / `retry:` como campos de primera clase, no
tiene una primitiva de keep-alive incorporada, y no sanitiza valores
que inyectarían campos adicionales en la red.

Suprnova trata SSE como un subsistema real en lugar de un helper
puntual:

- `SseEvent` es un valor tipado con builders falibles (`try_with_*`) e
  infalibles (`with_*`), tipos `Frame` y `Comment` distintos, y un
  contrato de sanitización documentado sobre cada campo de una sola
  línea.
- `HttpResponse::sse(stream)` se conecta al mismo pipeline de cuerpo
  `stream_bytes` que usa cualquier otra respuesta de larga duración,
  así que SSE comparte una única ruta de cancelación, encabezados y
  aislamiento de pánicos con el resto del framework.
- Los productores componen cualquier `Stream<Item = SseEvent>` -
  `tokio::sync::mpsc`, `tokio::sync::broadcast`,
  `futures::stream::iter`, o el adaptador de dispersión de
  [BroadcastHub](broadcasting.md). Ninguno de ellos necesita una vía
  de escape del framework.
- Un lector de `Last-Event-ID` (`sse::last_event_id`) y la regla
  WHATWG de descarte por NUL vienen incluidos, así que la reanudación
  tras una caída está a una sola llamada de parseo, en lugar de una
  utilidad de encabezados hecha a medida por cada app.

## Referencia

| Símbolo | Propósito |
|--------|---------|
| `suprnova::sse::SseEvent` | Una pieza emitible de un stream SSE. Dos tipos - `Frame` (evento con `event` / `id` / `retry` opcionales + `data`) y `Comment` (keep-alive). |
| `SseEvent::data(text)` | Construye un frame con solo líneas `data:`. |
| `SseEvent::json(event, &payload)` | Construye un frame cuyo payload es `payload` serializado con `serde_json`; establece `event:` a `event`. Devuelve `Result<Self, serde_json::Error>`. |
| `SseEvent::error(message)` | Construye un frame con `event: error` y el mensaje suministrado como `data`. |
| `SseEvent::comment(text)` | Construye un evento solo-comment (`: <text>\n\n`). Invisible para el navegador; mantiene despiertos a los proxies. |
| `SseEvent::keep_alive()` | Forma abreviada del comment vacío `:\n\n`. Heartbeat de bytes mínimos. |
| `.with_event(name)` / `.with_id(id)` / `.with_retry(Duration)` | Builders infalibles sobre un `Frame`; no-op silencioso sobre un `Comment`. Eliminan CR / LF / NUL en el momento de `to_wire()` con un WARN estructurado. |
| `.try_with_event(name)` / `.try_with_id(id)` | Contrapartes falibles - devuelven `Err(FrameworkError::validation(...))` ante CR / LF / NUL. Úsalas cuando el valor proviene de una entrada de usuario y quieres un 4xx en lugar de una eliminación silenciosa. |
| `.event()` / `.id()` / `.retry()` / `.payload()` / `.is_comment()` / `.comment_text()` | Accesores. `payload()` se llama así para evitar colisionar con el constructor `data`. |
| `SseEvent::to_wire()` | Serializa a `Bytes` en el formato en la red de SSE. Público para que los tests y los adaptadores puedan codificar sin pasar por el builder de la respuesta. |
| `suprnova::sse::last_event_id(&Request) -> Option<String>` | Lee el encabezado `Last-Event-ID`. Devuelve `None` cuando está ausente O cuando el valor contiene un byte NUL (WHATWG descarta los ids inválidos). |
| `suprnova::sse::last_event_id_from_value(Option<&str>)` | Helper puro que expone el mismo contrato de validación - testeable de forma unitaria sin construir un `Request`. |
| `HttpResponse::sse(stream)` | Construye una respuesta en streaming a partir de cualquier `Stream<Item = SseEvent> + Send + Sync + 'static`. Establece `Content-Type`, `Cache-Control`, `Connection`, `X-Accel-Buffering`. |

## Siguiente

- [WebSockets](websockets.md) - la otra conexión de larga duración,
  para cuando necesites frames bidireccionales o binarios.
- [Difusión](broadcasting.md) - la dispersión de `BroadcastHub`
  compartida con los suscriptores de WebSocket.
- [Notificaciones](notifications.md) - drivers de canal para entrega
  push sin streaming (correo, base de datos, difusión).
- [Web Push](web-push.md) - notificaciones enviadas por el servidor
  que llegan al cliente aunque no haya ningún `EventSource` abierto.
- [Respuestas](responses.md) - el resto de la superficie del builder
  de `HttpResponse`.

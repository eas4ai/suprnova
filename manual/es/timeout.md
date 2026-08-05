# Tiempos de espera de solicitudes

`TimeoutMiddleware` impone un plazo estricto sobre cada solicitud HTTP. Un
handler lento - una consulta a la base de datos colgada, una API upstream
que no responde, un bucle infinito accidental en alguna ruta de ejecución
frecuente - de otro modo mantendría abierta una conexión de hyper hasta
que el cliente se rindiera o el sistema operativo matara el proceso. El
middleware de timeout acota esa espera, descarta el handler en vuelo, y
devuelve `503 Service Unavailable` para que el operador vea el fallo en
lugar de que la aplicación filtre conexiones en silencio.

Recurre a él cuando estés construyendo algo que hable con la internet
pública, algo que se dispersa hacia APIs de terceros, o algo donde "hoy la
base de datos podría estar lenta" sea un martes realista.

```rust
use suprnova::{global_middleware, TimeoutMiddleware};

pub async fn register() {
    // Cada ruta HTTP obtiene un techo de 30 segundos.
    global_middleware!(TimeoutMiddleware::default());
}
```

Esa única línea le da a toda la aplicación el mismo techo por defecto que
Suprnova usa para el timeout de conexión a la base de datos - se elige
una vez, se aplica en todas partes. Las anulaciones por ruta son una
línea cada una. El resto de este capítulo explica exactamente qué acota
el plazo, qué intencionalmente no acota, y cómo interactúa con el límite
de pánico, las respuestas en streaming, y los WebSockets.

## El middleware

`TimeoutMiddleware` vive en `suprnova::TimeoutMiddleware`. Expone tres
constructores y un accesor:

```rust
use std::time::Duration;
use suprnova::TimeoutMiddleware;

let default_30s = TimeoutMiddleware::default();
let custom      = TimeoutMiddleware::new(Duration::from_millis(2_500));
let whole_secs  = TimeoutMiddleware::seconds(5);

assert_eq!(default_30s.duration(), Duration::from_secs(30));
assert_eq!(custom.duration(),      Duration::from_millis(2_500));
assert_eq!(whole_secs.duration(),  Duration::from_secs(5));
```

`TimeoutMiddleware::default()` usa un plazo de 30 segundos. Ese número no
es arbitrario - coincide con `DB_CONNECT_TIMEOUT` (también 30s) para que
una solicitud bloqueada esperando una conexión nueva a la base de datos y
una solicitud bloqueada dentro del handler compartan un mismo techo. Si
subes uno, sube el otro.

`TimeoutMiddleware::seconds(n)` es la forma abreviada para el caso común
de segundos enteros. `TimeoutMiddleware::new(Duration::…)` es la vía de
escape cuando necesitas precisión de milisegundos (un health check
interno que nunca debería tardar más de 200ms; una sonda sintética con un
presupuesto de 50ms).

## Instalarlo globalmente

Un timeout global es el punto de partida correcto: le da a cada ruta un
techo sin que nadie tenga que acordarse de añadirlo. Instálalo en
`bootstrap.rs` junto al resto de tu middleware global:

```rust
// src/bootstrap.rs
use suprnova::{
    global_middleware, CorsConfig, CorsMiddleware, DB, RequestIdMiddleware, TimeoutMiddleware,
};
use crate::middleware::LoggingMiddleware;

pub async fn register() {
    DB::init().await.expect("database connect");

    // El orden de ejecución importa: primero request-id (para que los
    // logs del timeout lo lleven), luego logging (para que las
    // solicitudes lentas igual se observen), y por último el timeout mismo.
    global_middleware!(RequestIdMiddleware);
    global_middleware!(LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());

    global_middleware!(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"]),
    ));
}
```

El orden importa porque el middleware global envuelve el resto de la
cadena en el orden de registro: `RequestIdMiddleware` se ejecuta primero
en la entrada y último en la salida, así que el id de solicitud está en
el alcance mientras el timeout dispara su `503`. Poner el timeout antes
que el logging ocultaría del log de acceso las solicitudes lentas que sí
llegaron a completarse al final.

## Restringir por ruta

Un techo global de 30 segundos es generoso a propósito - está ahí para
atrapar handlers desbocados, no para hacer cumplir SLAs. Cuando un
endpoint específico debería fallar más rápido, adjúntale un timeout por
ruta:

```rust
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    // Endpoint público de reportes: debe responder en 5s o preferimos
    // devolver 503 y dejar que el cliente reintente, antes que bloquear.
    .get("/report", controllers::report::show)
    .middleware(TimeoutMiddleware::seconds(5));
```

También puedes adjuntar un timeout más estricto a un grupo de rutas. Esta
es la forma típica para una API pública donde cada solicitud debería ser
rápida, mientras el resto de la app conserva el valor por defecto de 30
segundos:

```rust
use suprnova::Router;
use suprnova::TimeoutMiddleware;

Router::new()
    .group("/api", |r| {
        r.get("/users",       controllers::api::users::index)
         .post("/users",      controllers::api::users::create)
         .get("/users/{id}",  controllers::api::users::show)
    })
    .middleware(TimeoutMiddleware::seconds(3));
```

### El global es un techo; lo de por ruta solo puede restringir

El middleware global se ejecuta **por fuera** del middleware de ruta. La
cadena se envuelve de adentro hacia afuera:

```
Timeout global (30s) → Timeout de ruta (3s) → handler
```

Ambos futures de `tokio::time::timeout` están armados; el interior se
dispara primero porque tiene el plazo más corto. Así que un timeout por
ruta solo puede hacer una ruta *más estricta* que la global, nunca más
laxa.

Si un único endpoint legítimamente necesita ejecutarse *más tiempo* que
el valor global por defecto - un reporte lento, una carga grande, un
fallback de long-poll - tienes dos opciones:

1. Sube el valor global. Es lo más simple, pero relaja el techo para
   todas las demás rutas también.
2. Acota el middleware global a un grupo de rutas que *excluya* el
   endpoint largo, y adjúntale un timeout separado (o ninguno) a la ruta
   lenta. Esto conserva el valor estricto por defecto en todas las demás
   partes.

La segunda opción es la forma correcta para un caso atípico aislado; la
primera es correcta cuando toda una clase de trabajo necesita más margen.

## Qué acota realmente el plazo

El plazo compite con el future que devuelve `next(request)`. Ese future
se resuelve en el momento en que tu handler devuelve su `HttpResponse` -
no cuando el cuerpo termina de transmitirse en streaming. Esa distinción
es estructural:

- **Los handlers normales** construyen todo su cuerpo antes de retornar,
  así que el plazo efectivamente acota el tiempo total del handler. Un
  handler que serializa una lista JSON, renderiza una página de Inertia,
  o ensambla una respuesta HTML mantiene el future hasta que el trabajo
  está hecho.
- **Las respuestas en streaming** (`HttpResponse::sse(...)`,
  `HttpResponse::stream_bytes(...)`) retornan *de inmediato* con un
  cuerpo perezoso. La cadena de middleware ya ha terminado en el momento
  en que hyper empieza a extraer bytes del stream, así que el plazo
  nunca observa el tiempo de vida del cuerpo. Un stream de eventos SSE
  puede permanecer abierto durante horas bajo un timeout de 30 segundos,
  por diseño - consulta [Eventos enviados por el servidor](sse.md) para
  el modelo de streaming.
- **Las actualizaciones de WebSocket** se saltan explícitamente. Consulta
  la siguiente sección.

Este es casi con certeza el comportamiento que quieres. Si envolvieras un
stream SSE de larga duración en un timeout de 30 segundos, el framework
derribaría la conexión a mitad del stream cada 30 segundos y la
funcionalidad sería inutilizable.

## La excepción de WebSocket

El middleware inspecciona la solicitud antes de armar el plazo:

```rust
if is_websocket_upgrade(request.headers()) {
    return next(request).await;
}
```

Cualquier solicitud que lleve `Upgrade: websocket` se salta el timeout
por completo. La comprobación no distingue mayúsculas de minúsculas en el
valor del token (`WebSocket`, `websocket`, `WEBSOCKET` coinciden todos),
y un `Connection: upgrade` desnudo sin `Upgrade: websocket` *no* se trata
como una actualización de WS - eso fluye a través del timeout
normalmente.

Hoy, las actualizaciones de WebSocket toman una ruta de servidor separada
que no ejecuta middleware global en absoluto, así que esta guarda es
defensa en profundidad - evita que el timeout llegue a acotar alguna vez
un canal bidireccional de larga duración el día en que eso cambie.
Consulta [WebSockets](websockets.md) para ver cómo se despachan las
actualizaciones y cuál es el tiempo de vida de un socket conectado.

## Qué sucede cuando llega el plazo

Cuando `tokio::time::timeout` transcurre antes de que el handler termine,
el middleware hace tres cosas, en este orden:

1. **Descarta el future del handler en vuelo.** El future estaba siendo
   sondeado dentro del combinador `timeout`; el combinador devuelve
   `Err(Elapsed)` y el future se descarta en el punto donde estaba
   suspendido por última vez.
2. **Registra una advertencia** con la ruta y la duración del timeout en
   milisegundos:

   ```
   WARN suprnova::timeout request exceeded its timeout; returning 503 Service Unavailable
       route=/report timeout_ms=5000
   ```

   El log está en nivel `WARN` para que aparezca por defecto en los
   dashboards de los operadores, separado de los logs de acceso en
   `INFO` de las solicitudes normales.
3. **Devuelve `503 Service Unavailable`** con un cuerpo de texto plano:

   ```
   HTTP/1.1 503 Service Unavailable
   Content-Type: text/plain
   Content-Length: 42

   Service Unavailable: request timed out
   ```

El 503 se envuelve en `Err(HttpResponse::…)` así que hace cortocircuito
en el resto de la cadena, igual que cualquier otra solicitud rechazada
por middleware. El middleware exterior (logging, request-id, CORS) igual
ejecuta su lado posterior al handler, así que la respuesta sale con los
encabezados correctos.

### Por qué 503 y no 504

`504 Gateway Timeout` es el código correcto cuando *tú* eres el gateway y
un *upstream* superó su timeout. `503 Service Unavailable` es el código
correcto cuando *este* servicio no pudo producir la respuesta a tiempo.
El middleware de timeout está acotando *nuestro propio* handler, así que
devuelve 503. Si quieres una forma distinta - un cuerpo JSON, un estado
distinto, un código legible por máquina - envuelve tu propio middleware
exterior alrededor del timeout y traduce su respuesta 503.

## Seguridad ante la cancelación

Cuando el plazo transcurre, el future del handler se **descarta** en su
punto `.await` actual. Esto es cancelación normal de Tokio; lo mismo
sucede cuando un cliente cierra la conexión a mitad de la solicitud.
Cualquier cosa retenida a través del límite de un await se libera
mediante su impl de `Drop`:

- **Las transacciones de base de datos** se revierten. Un
  `DatabaseTransaction` de SeaORM tiene un impl de `Drop` que emite
  `ROLLBACK` sobre la conexión subyacente.
- **Las guardas de `Mutex` y `RwLock`** se liberan. Una guarda de la
  biblioteca estándar o de `parking_lot` se libera al descartarse; otro
  que esté esperando puede tomarla de inmediato.
- **Los descriptores de archivo** se cierran. El descriptor a nivel de
  sistema operativo se libera cuando se descarta el `tokio::fs::File`.
- **Las conexiones de red** regresan al pool o se cierran, dependiendo
  del comportamiento de drop del pool.

El resultado es que un handler cuyo timeout expiró no deja nada
colgando - el operador ve el 503, la base de datos ve el rollback, la
siguiente solicitud ve un pool limpio.

### Qué *no* se cancela

Cualquier cosa que hayas movido fuera de la solicitud con `tokio::spawn`
queda **independiente**. Las tareas lanzadas viven en el runtime, no en
el future de la solicitud, así que descartar la solicitud no las
detiene. Esto importa cuando escribiste algo como esto:

```rust
pub async fn webhook(req: Request) -> Response {
    let payload: WebhookPayload = req.json().await?;

    // Trabajo en segundo plano de tipo dispara y olvida. Sobrevive a que
    // la solicitud agote su timeout.
    tokio::spawn(async move {
        if let Err(e) = process_webhook(payload).await {
            tracing::error!("webhook processing failed: {e}");
        }
    });

    Ok(HttpResponse::new().status(204))
}
```

Si la solicitud agota su timeout *antes* de que se ejecute la línea del
`spawn`, el spawn nunca ocurre. Si la solicitud agota su timeout
*después* del spawn, la tarea en segundo plano sigue ejecutándose - no se
cancela junto con la solicitud. Eso es casi siempre lo que quieres para
trabajo de estilo webhook, pero sí significa que la limpieza después de
un `.await` largo dentro del handler **no** está garantizada:

```rust
pub async fn upload(req: Request) -> Response {
    let temp_path = save_to_temp(&req).await?;

    // Si esto es lo que agota el timeout, la limpieza de abajo NO SE EJECUTA.
    let processed = long_running_processing(&temp_path).await?;

    // No está garantizado bajo un timeout.
    tokio::fs::remove_file(&temp_path).await?;

    Ok(HttpResponse::json(serde_json::to_value(&processed)?))
}
```

La solución es usar RAII. Envuelve el archivo temporal en un struct cuyo
impl de `Drop` lo elimine; así la limpieza se ejecuta ya sea que el
handler retorne, retorne un error, o se lo descarte a mitad de un
`.await` por el timeout. Es la misma disciplina que aplicarías ante
cualquier fuente de cancelación - desconexión del cliente, apagado del
runtime, recuperación de pánico.

## Interacción con el límite de pánico

El servidor de Suprnova envuelve toda la cadena de middleware en
[`execute_chain_safely`](lifecycle.md), que usa
`AssertUnwindSafe(...).catch_unwind()` para traducir los pánicos en un
`500 Internal Server Error` saneado. Una solicitud cuyo timeout expiró
**no** es un pánico - el future se descarta limpiamente - así que el
`503` del timeout sale sin involucrar en absoluto al límite de pánico.

Los dos límites manejan modos de fallo distintos:

| Fallo | Límite | Estado | Cuerpo |
|---|---|---|---|
| El `.await` del handler supera el plazo | `TimeoutMiddleware` | `503` | `Service Unavailable: request timed out` |
| El handler entra en pánico (`.unwrap()` sobre `None`, etc.) | `execute_chain_safely` | `500` | `{"message": "Internal Server Error"}` |
| El handler devuelve `Err(HttpResponse)` | flujo normal de `Response` | lo que el handler haya establecido | lo que el handler haya establecido |

No tienes que elegir - ambos límites siempre están instalados. Un handler
que entra en pánico *después* de superar su timeout igual produce un 503
(el future se descartó antes de que el pánico pudiera ocurrir). Un
handler que entra en pánico *antes* de superar su timeout produce un
500.

## Ajuste operativo

Tres consideraciones al elegir valores de timeout:

1. **Haz coincidir tu timeout de conexión a la base de datos.** Si
   `DB_CONNECT_TIMEOUT=30` (el valor por defecto), un timeout de
   solicitud más corto que 30s se disparará antes de que una conexión
   lenta llegue a completarse - el usuario ve `503` en lugar de tener la
   oportunidad de recuperarse. O bien sube el timeout de conexión, o
   acepta que "30s" es el piso.
2. **Ten en cuenta el handler legítimo más lento.** Mira un histograma
   de las duraciones de solicitud en nivel `INFO`. El p99 de la cola
   lenta debería situarse cómodamente por debajo del timeout, con margen
   para el desajuste de reloj y el jitter del event loop. Un timeout que
   se dispara rutinariamente sobre tráfico saludable es una mala
   configuración, no una funcionalidad.
3. **Los timeouts por ruta son observabilidad.** Restringir
   `TimeoutMiddleware::seconds(3)` sobre `/api/*` convierte una API
   degradada en una alerta visible (logs llenos de WARN, 503 en el
   balanceador de carga) en lugar de un problema de latencia que se
   arrastra poco a poco. Úsalos donde tengas un SLA y quieras un fallo
   contundente cuando no lo cumplas.

Los propios tests de integración del framework usan duraciones en el
rango de los milisegundos (`TimeoutMiddleware::new(Duration::from_millis(50))`)
para ejercitar el plazo de forma determinista. Los plazos de producción
casi siempre son en segundos enteros.

### Por qué Suprnova diverge

En un despliegue de Laravel + PHP-FPM, los timeouts de solicitud viven
fuera de la aplicación: el `proxy_read_timeout` de nginx, el
`request_terminate_timeout` de PHP-FPM, el timeout de inactividad del
balanceador de carga. El proceso PHP se mata cuando se agota el
presupuesto, y cualquier estado abierto - conexiones de base de datos,
descriptores de archivo - se filtra hasta que la siguiente solicitud
reutiliza el worker.

Suprnova acota la solicitud dentro de la propia aplicación porque puede
hacerlo. El handler es un future de Tokio, no un proceso PHP, así que
descartarlo ejecuta los impls de `Drop` de forma limpia: las
transacciones se revierten, los bloqueos se liberan, los descriptores se
cierran, el pool de conexiones se mantiene saludable. El 503 también sale
*como una respuesta HTTP real* - los clientes ven un código de estado
apropiado en lugar de un reset del upstream.

Esta es también la razón por la que el middleware no intenta ser una
capa `Timeout` de Tower. La capa de Tower es genérica sobre cualquier
servicio de Tokio y devuelve `tower::timeout::error::Elapsed`, que quien
la llama luego tiene que mapear a un estado HTTP. El middleware de
Suprnova sabe que está envolviendo un pipeline de solicitudes HTTP;
devuelve `503` directamente, registra la ruta responsable, y respeta las
excepciones de WebSocket y de streaming del framework sin que quien lo
llama tenga que razonar sobre ellas. La capa de Tower es la primitiva
correcta para un servicio de Tokio genérico; para una solicitud HTTP,
esta es la forma correcta.

## Siguiente

- [Middleware](middleware.md) - el trait, la cadena, el registro global
  frente al de ruta, los ganchos terminables
- [Ciclo de vida de la solicitud](lifecycle.md) - dónde se ubica el
  timeout dentro de la cadena, y cómo `execute_chain_safely` maneja los
  pánicos
- [Eventos enviados por el servidor](sse.md) - el modelo de respuesta en
  streaming que el timeout intencionalmente no acota
- [WebSockets](websockets.md) - la ruta de actualización que evita el
  timeout por completo
- [Errores](errors.md) - cómo las respuestas 5xx se despachan como
  eventos `ErrorOccurred` para la observabilidad

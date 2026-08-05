# Observabilidad

El framework incluye tres capas de señales visibles para el operador:
registros estructurados (siempre activos), correlación por id de
solicitud (siempre activa, se propaga a las tareas lanzadas) y un
puente de OpenTelemetry opcional que convierte cada span de `tracing`
en un span de OTel exportado. El mismo `#[tracing::instrument]` que
escribirías para los registros locales se convierte en un span de
traza distribuida cuando la feature de OTel está activa - sin una
segunda API de instrumentación.

```rust
use suprnova::telemetry::{init_telemetry, OtelConfig};
use suprnova::logging::LogConfig;

#[suprnova::main]
async fn main() {
    let guard = init_telemetry(LogConfig::from_env(), OtelConfig::from_env());

    // ... ejecuta la app ...

    // Vacía la telemetría en búfer antes de salir. Los procesadores por
    // lotes de OTel mantienen spans/métricas/registros en memoria; soltar
    // el guard sin `shutdown` pierde lo que aún no se haya exportado.
    guard.shutdown().await;
}
```

El `Server` de una app con andamiaje ya llama a `init_telemetry` por ti
y vacía el guard ante la señal de apagado - solo lo conectas a mano
cuando embebes Suprnova en tu propio runtime.

## Las tres capas

| Capa | Siempre activa | Qué te da |
|---|---|---|
| Registro estructurado (`tracing`) | Sí | Registros por `stdout` en formato `pretty` (dev) o `json` (producción), según el entorno |
| Correlación por id de solicitud | Sí | Id por solicitud acotado con un `tokio::task_local!`, repetido en `X-Request-Id`, se propaga a las tareas de `spawn_with_request_id` |
| Exportación de OpenTelemetry | feature `otel` + endpoint de recolector | Exportación OTLP HTTP/proto de trazas, métricas y registros; propagación `traceparent` de W3C en ambos sentidos |

La capa de OTel es **opcional en tiempo de compilación**, de modo que
las compilaciones por defecto no cargan dependencias de OpenTelemetry
y la fachada [`Metrics`](#métricas) compila a no-ops inertes. Con la
feature apagada, "la traza" y "la exportación de métricas" se
vuelven no-ops en silencio - tus registros siguen funcionando.

### Por qué Suprnova diverge

La historia de observabilidad de Laravel se reparte entre eventos
propios del framework (`QueryExecuted`, `MessageSent`, `JobProcessed`)
y asuntos de runtime delegados a extensiones de PHP (OpenTelemetry,
Sentry, New Relic) conectadas en la capa de FPM. La superficie de
eventos es rica; la superficie de runtime es "instala la extensión
que tu proveedor de APM necesite".

Suprnova es un único proceso asíncrono, así que posee ambas mitades.
La superficie de eventos tiene paridad (la misma forma de
`QueryExecuted`/`NotificationSent`/`ErrorOccurred`), y la superficie
de runtime es un puente `tracing` → OpenTelemetry dentro del
framework. No instalas una extensión; activas una feature flag y los
mismos spans que ya emites pasan a exportarse a OTel.

## Registro estructurado

`LogConfig::from_env()` lee dos variables de entorno:

| Var | Por defecto | Notas |
|---|---|---|
| `LOG_LEVEL` | `"info"` | Sintaxis de env-filter de `tracing-subscriber` (p. ej. `"debug,sqlx=warn,hyper=warn"`) |
| `LOG_FORMAT` | según el entorno | `"json"` en producción, `"pretty"` en todo lo demás; un valor explícito siempre gana |

El valor por defecto del formato se detecta a partir de `APP_ENV`
mediante `Environment::detect()`: un despliegue de producción obtiene
por defecto una salida de un objeto JSON por línea para los
agregadores de registros, y las ejecuciones locales/de desarrollo
obtienen una salida multilínea legible para humanos. Un
`LOG_FORMAT=pretty` explícito anula el valor por defecto de
producción si quieres `stdout` en crudo en producción.

```bash
# Desarrollo local - los overrides explícitos ganan
LOG_LEVEL=debug,sqlx=warn,hyper=warn LOG_FORMAT=pretty cargo run

# Producción - APP_ENV=production cambia el formato por defecto a json
APP_ENV=production LOG_LEVEL=info cargo run --release
```

Una directiva `LOG_LEVEL` mal formada no tumba el arranque - recae en
`"info"` e imprime una advertencia de una línea en `stderr` para que
la mala configuración sea visible para el operador.

### Contexto de span en cada línea

Toda solicitud HTTP enrutada se ejecuta dentro de un span `request`
creado por el middleware más externo del framework. El span lleva
tres campos - `request_id`, `method`, `path` - y el formateador JSON
los anida bajo `span` en cada evento emitido dentro de la solicitud.
Tu código de aplicación no necesita leer ni registrar el id en cada
línea; el span lo lleva de forma implícita:

```rust
use tracing::info;

pub async fn show(req: suprnova::Request) -> suprnova::Response {
    info!(user_id = 42, "loaded dashboard");
    // La línea JSON lleva span.request_id / span.method / span.path
    // sin que el sitio de la llamada tenga que enhebrar nada.
    Ok(suprnova::json_response!({ "ok": true }))
}
```

## Correlación por id de solicitud

Toda solicitud recibe un id UUID v4 en minúsculas de 36 caracteres,
acotado con un `tokio::task_local!`. El middleware reutiliza un
`X-Request-Id` entrante cuando el valor del encabezado pasa una
comprobación de seguridad estricta (alfanuméricos ASCII más
`-_.:`, hasta 128 bytes); cualquier cosa fuera de ese conjunto de
caracteres se rechaza y se sustituye por un UUID nuevo, de modo que
un atacante no pueda inyectar caracteres de control en la salida de
registro ni inflar las canalizaciones posteriores.

El mismo id se repite en **toda** respuesta - éxito, error y
recuperación de pánico - como el encabezado `X-Request-Id`, de modo
que un frontend o un servicio corriente arriba puede incluirlo en
reportes de bugs y los operadores pueden buscarlo con grep en el
registro estructurado.

### Leer el id

```rust
use suprnova::{current_request_id, spawn_with_request_id};

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    // Dentro de una solicitud, el id siempre está presente.
    let id = current_request_id().expect("inside a request");
    tracing::info!(request_id = %id, "checkout starting");

    // Trabajo en segundo plano lanzado desde un handler. `tokio::spawn`
    // arranca una tarea con los task-locals vacíos - el future lanzado
    // perdería el id de solicitud sin ayuda. `spawn_with_request_id`
    // captura el id de quien llama y lo vuelve a acotar para el future
    // lanzado, y adjunta el span actual de `tracing` para que los
    // eventos de la tarea hereden `request_id` igual que los eventos
    // dentro de la solicitud.
    spawn_with_request_id(async move {
        // Esta línea de registro lleva el id de la solicitud de origen.
        tracing::info!("post-checkout fanout running");
    });

    Ok(suprnova::ok!())
}
```

`current_request_id()` devuelve `None` fuera de una solicitud - los
jobs en segundo plano, las tareas programadas y los tests sin el
middleware no ven ningún id, y el ayudante no inventa uno.
`spawn_with_request_id` fuera de un alcance de solicitud es
exactamente `tokio::spawn`; no pasa nada especial.

### Dónde más está disponible el id

| Superficie | Cómo |
|---|---|
| Eventos de `tracing` | `span.request_id` en cada línea dentro de la solicitud |
| Encabezado de respuesta | `X-Request-Id` en respuestas de éxito, error y recuperadas de un pánico |
| Bolsa `Context` | `Context::get("_request_id")` - legible desde observadores, oyentes y jobs que consultan `Context` |
| Tareas lanzadas | `current_request_id()` después de `spawn_with_request_id` |

## Eventos incorporados para observabilidad

El framework despacha eventos tipados en los puntos donde un operador
suele querer instrumentar. Cada uno es un `suprnova::Event` al que
puedes `listen` mediante `EventFacade::listen::<E, _>(...)` y enviar a
Sentry, Datadog, Slack o tu propia canalización de métricas. Todos
pasan por `dispatch_best_effort`, así que un oyente que falla no rompe
la solicitud que lo disparó.

| Evento | Cuándo se dispara | Lleva |
|---|---|---|
| `ErrorOccurred` | Cualquier conversión de `FrameworkError` → 5xx (incluida la recuperación de pánico) | contexto de error + id de solicitud |
| `QueryExecuted` | Toda consulta enrutada a través de los ayudantes de ejecución instrumentados | sql, bindings, duración, conexión, clasificación lectura/escritura, resultado |
| `ConnectionEstablished` | `DbConnection::connect` tuvo éxito | nombre de la conexión |
| `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` | `DB::transaction` en forma de closure + handles manuales | nombre de la conexión |
| `NotificationSending` / `NotificationSent` / `NotificationFailed` | Antes/después/error por canal de `Notification::send` | notificación + canal + destinatario |

`ErrorOccurred` es el gancho para enviar excepciones 5xx;
`QueryExecuted` es el gancho para alertas de consultas lentas; el
trío de notificaciones es el gancho para paneles de entrega. Consulta
[Eventos](events.md) para la API de oyentes y [Ciclo de
vida](lifecycle.md) para en qué punto de la ruta de la solicitud se
dispara cada evento.

### Observación directa de consultas a la BD

`DB::listen` es un segundo gancho, síncrono, hecho a medida
específicamente para `QueryExecuted`. Se dispara en línea dentro del
ejecutor, así que un oyente lento ralentiza la consulta - mantenlo
ligero. La ruta del despachador (`EventFacade::listen::<QueryExecuted,
_>`) ejecuta a todos de mejor esfuerzo y tolera errores; prefiérela
para cualquier cosa que pueda fallar.

```rust
use suprnova::DB;

// En bootstrap.rs:
DB::listen(|q| {
    if q.time > std::time::Duration::from_millis(100) {
        tracing::warn!(
            sql = %q.sql,
            ms = q.time.as_millis(),
            "slow query"
        );
    }
})?;
```

Un oyente que a su vez emite una consulta a la base de datos **no**
volverá a disparar `QueryExecuted` para la llamada anidada - un guard
de reentrancia task-local evita el bucle "oyente-que-registra-en-BD →
emite evento → registro-en-BD → ...".

### Capturar un registro de consultas para tests / depuración

Para aserciones de test o para depurar de forma puntual "¿qué se
ejecutó durante este bloque?":

```rust
use suprnova::DB;

DB::enable_query_log()?;
// ... ejecuta el código que quieres inspeccionar ...
let queries = DB::get_query_log()?;
for q in &queries {
    println!("{:>4}ms  {}", q.time.as_millis(), q.to_raw_sql());
}
DB::disable_query_log()?;
DB::flush_query_log()?;
```

El búfer es **ilimitado** - cada consulta capturada lo hace crecer.
Úsalo para tests y para una investigación de una sola vez, y
vacíalo periódicamente si lo dejas activo en producción.

## Trazado distribuido (OTel)

Añade la feature `otel` para activarlo:

```toml
[dependencies]
suprnova = { git = "...", features = ["otel"] }
```

Configúralo con las variables de entorno estándar de OTel:

```bash
# Mínimo: dónde vive el recolector.
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
OTEL_SERVICE_NAME=my-app          # por defecto "suprnova"
OTEL_SERVICE_VERSION=1.4.2        # por defecto la versión de tu crate
```

La telemetría está **habilitada** solo cuando `OTEL_EXPORTER_OTLP_ENDPOINT`
está establecida **y** el interruptor de corte `OTEL_SDK_DISABLED` no
está activo. Sin ningún endpoint la capa de registro se ejecuta sola,
y el guard devuelto no contiene ningún proveedor, así que soltarlo sin
`shutdown()` es silencioso (sin la advertencia espuria de "buffered
telemetry may be lost" en cada proceso de test).

### El contexto de traza se une automáticamente

**Entrante.** Cuando llega una solicitud que lleva un encabezado
[`traceparent`](https://www.w3.org/TR/trace-context/) de W3C - es
decir, la hizo otro servicio trazado - el middleware extrae ese
contexto y reasigna como padre el span de la solicitud al span de
quien llamó. Tu span de servidor aparece como hijo dentro de la
*misma* traza distribuida, no como una raíz nueva. Una solicitud sin
`traceparent` (un golpe directo del navegador) se mantiene como un
span raíz limpio.

**Saliente.** El cliente HTTP del framework ([`Http`](http-client.md))
inyecta el contexto de traza activo como `traceparent` en cada
llamada saliente, de modo que el servicio corriente abajo continúa
la misma traza.

En conjunto: `servicio de origen → tu handler → servicio de destino`
es una única traza conectada, sin plomería de spans manual en tus
handlers.

**Estado de error.** Cuando un handler devuelve un 5xx, el span de la
solicitud se marca como erróneo para que el backend de OTel muestre
`Status::Error`. (Un *pánico* del handler se atrapa y se convierte en
un 500 con un registro de nivel error y un evento `ErrorOccurred`,
pero el estado del span de OTel no se establece en esa ruta - el
pánico desenrolla el future del span antes de que el marcador se
ejecute.)

### Añadir tus propios spans

Como el puente convierte cada span de `tracing` en un span de OTel,
instrumentas con `tracing` puro - sin ninguna API específica de OTel
en tu código:

```rust
use suprnova::DatabaseConnection;

#[tracing::instrument(skip(db))]
async fn load_dashboard(db: &DatabaseConnection, user_id: i64) -> anyhow::Result<()> {
    // Este span se anida automáticamente bajo el span de la solicitud,
    // y se exporta a tu recolector cuando la feature `otel` está activa.
    Ok(())
}
```

### Variables de entorno que Suprnova lee

| Var | Efecto |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | URL base del recolector. Sin establecer → telemetría deshabilitada. |
| `OTEL_SERVICE_NAME` | Atributo de recurso `service.name` (por defecto `"suprnova"`). |
| `OTEL_SERVICE_VERSION` | Atributo de recurso `service.version` (por defecto: la versión del crate). |
| `OTEL_SDK_DISABLED` | Interruptor de corte. `true` o `1`, sin distinguir mayúsculas, deshabilita la exportación aunque haya un endpoint establecido. |

El resto de los controles estándar de OTLP los lee el propio SDK, así
que configúralos de la forma habitual:

| Var | La lee |
|---|---|
| `OTEL_EXPORTER_OTLP_HEADERS` | el exportador (autenticación del recolector, p. ej. `Authorization=Bearer ...`) |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | el exportador (`http/protobuf`, etc.) |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | el exportador |
| `OTEL_EXPORTER_OTLP_COMPRESSION` | el exportador |

Los overrides de endpoint por señal (`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`,
`_METRICS_ENDPOINT`, `_LOGS_ENDPOINT`) quedan por ahora eclipsados por
el endpoint base - las tres señales van a
`OTEL_EXPORTER_OTLP_ENDPOINT`. Si necesitas repartir las señales entre
distintos recolectores, ejecuta un recolector local que las
encamine.

## Métricas

`Metrics` es la fachada para contadores, histogramas y gauges. Los
handles son baratos de clonar y resuelven el meter global en cada
construcción:

```rust
use suprnova::telemetry::Metrics;

// Contador - monótono.
let signups = Metrics::counter("user.signups");
signups.inc();                                  // +1
signups.inc_by(3);                              // +3
signups.inc_with(&[("plan", "pro")]);           // +1 con una etiqueta

// Histograma - distribuciones (latencia, tamaños).
let latency = Metrics::histogram("request.latency_ms");
latency.record(42.0);
latency.record_with(42.0, &[("route", "/checkout")]);

// Gauge - valor puntual en el tiempo.
let queue_depth = Metrics::gauge("jobs.pending");
queue_depth.set(17.0);
queue_depth.set_with(17.0, &[("queue", "emails")]);
```

Sin la feature `otel` cada llamada de arriba es un no-op sin ninguna
asignación de memoria - deja la instrumentación en las rutas de
ejecución frecuente y no pagas nada en las compilaciones por defecto.

Los handles de métrica se enlazan a cualquier proveedor de meter que
esté activo cuando el instrumento subyacente se resuelve por primera
vez. Crea los handles **después** de que `init_telemetry` se haya
ejecutado (o de forma perezosa en el primer uso) - un handle
construido antes de la inicialización se resuelve contra el proveedor
no-op y se queda inerte. El patrón idiomático es un handle
`once_cell` / `LazyLock` resuelto en la primera emisión, bien después
del arranque.

Los valores de los atributos son de tipo cadena
(`&[(&'static str, &str)]`). Los atributos numéricos y booleanos son
una mejora planeada; por ahora, formátealos como cadenas en el sitio
de la llamada.

Nomenclatura: estable, ASCII, delimitada por puntos (p. ej.
`"http.requests.total"`, `"http.request.duration"`). Las convenciones
semánticas estándar de OTel viven en
`opentelemetry-semantic-conventions::metric::*`.

## El contrato de apagado

`init_telemetry` devuelve un `TelemetryGuard` que posee los handles
del proveedor del SDK. Los procesadores por lotes de OTel almacenan
en búfer spans / métricas / registros en memoria y los vacían de
forma asíncrona, así que debes llamar a `guard.shutdown().await`
antes de que el proceso salga o pierdes lo que aún esté en el búfer.

- Llamar a `shutdown()` vacía el búfer y es seguro llamarlo una vez
  (toma `self`).
- Soltar el guard **sin** `shutdown()` registra una advertencia -
  pero solo cuando el guard realmente contiene proveedores. Una
  ejecución con la telemetría deshabilitada (sin endpoint, o
  `OTEL_SDK_DISABLED`, o una compilación sin `otel`) devuelve un
  guard sin proveedores cuyo drop es silencioso, así que las
  ejecuciones de desarrollo y test sin recolector no se ven
  inundadas de avisos.

## Resumen

| Tarea | API |
|---|---|
| Habilitar OTel | `features = ["otel"]` + `OTEL_EXPORTER_OTLP_ENDPOINT` |
| Inicializar | `init_telemetry(LogConfig::from_env(), OtelConfig::from_env())` |
| Vaciar al salir | `guard.shutdown().await` |
| Deshabilitar en tiempo de ejecución | `OTEL_SDK_DISABLED=true` |
| Span propio | `#[tracing::instrument]` (con puente automático a OTel) |
| Contador / histograma / gauge | `Metrics::counter/histogram/gauge(name)` |
| Unión de traza distribuida | Automática - entrante `traceparent` extraído, saliente inyectado |
| Leer el id de solicitud actual | `current_request_id()` |
| Propagar el id al lanzar una tarea | `spawn_with_request_id(future)` |
| Observador síncrono de consultas | `DB::listen(|q| { ... })` |
| Observador de consultas de mejor esfuerzo | `EventFacade::listen::<QueryExecuted, _>(...)` |
| Capturar consultas para tests | `DB::enable_query_log()` → `DB::get_query_log()` |

## Siguiente

- [Eventos](events.md) - API de oyentes, modos de despacho,
  `EventFacade::fake()` para tests
- [Ciclo de vida](lifecycle.md) - en qué punto de la ruta de la
  solicitud se dispara cada evento y dónde se construye el span de
  la solicitud
- [Manejo de errores](errors.md) - `ErrorOccurred`, `HttpError`,
  cuerpos 5xx sanitizados
- [Base de datos](database.md) - `QueryExecuted`, `DB::transaction`,
  los ayudantes de ejecución que disparan los eventos
- [Cliente HTTP](http-client.md) - la inyección saliente de
  `traceparent` que cierra el bucle de la traza distribuida

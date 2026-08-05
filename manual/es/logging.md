# Registro de eventos

Suprnova registra a través de [`tracing`](https://docs.rs/tracing) - cada
línea de registro es un evento estructurado con campos, no una cadena
formateada. En el arranque se instala un subscriber que lee `LOG_LEVEL` y
`LOG_FORMAT` del entorno, emite salida pretty multilínea en desarrollo y
un objeto JSON por línea en producción, y propaga un id por solicitud a
cada evento que emite un handler.

Este capítulo cubre la superficie de registro en sí: el subscriber, los
formatos, los niveles y la correlación por id de solicitud que hace que
un registro de producción sea buscable. Para el puente con OpenTelemetry
y el registro de consultas consulta
[Observabilidad](observability.md); para la bolsa `Context` de la
solicitud que los emisores pueden leer junto al id consulta
[Contexto](context.md).

## Qué se registra y dónde

Dos salidas por defecto:

| Dónde | Formato | Cuándo |
|---|---|---|
| `stdout` | `LogFormat::Pretty` - multilínea, con color, legible para humanos | desarrollo (`APP_ENV` es `local`, `dev`, `testing`, …) |
| `stdout` | `LogFormat::Json` - un objeto JSON por línea | producción (`APP_ENV=production` / `prod`) |

El valor por defecto de desarrollo/producción se calcula a partir de
`APP_ENV` mediante `Environment::detect()`. Anúlalo con
`LOG_FORMAT=pretty` o `LOG_FORMAT=json` para forzar uno de forma
explícita.

```env
# .env (desarrollo)
LOG_LEVEL=info,sqlx=warn
LOG_FORMAT=pretty   # opcional; este es el valor por defecto en desarrollo

# .env.production
LOG_LEVEL=info,sqlx=warn,suprnova::queue=debug
LOG_FORMAT=json     # opcional; este es el valor por defecto en producción
```

El framework solo escribe en `stdout`. En producción apunta ahí el
runtime de tu contenedor, el journal de systemd o tu agregador de
registros (`docker logs`, `kubectl logs`, `journalctl -u my-app`, un
agente de Loki/Vector, etc.). No hay un appender de archivo con
rotación - deja que la plataforma sea la dueña de la persistencia de los
registros.

## Emitir eventos

Usa las macros de `tracing` en handlers, trabajos, middleware, donde sea:

```rust
use suprnova::{json_response, session, Request, Response};
use tracing::{debug, info, warn, error, instrument};

pub async fn checkout(_req: Request) -> Response {
    let user_id: i64 = session()
        .and_then(|s| s.get::<i64>("user_id"))
        .unwrap_or(0);

    info!(user_id, "checkout starting");

    let order = place_order(user_id).await.map_err(|e| {
        error!(user_id, error = %e, "checkout failed");
        e
    })?;

    info!(user_id, order_id = order.id, total = order.total_cents, "checkout succeeded");

    json_response!(order)
}
```

Cada campo se convierte en una clave de primer nivel en la salida JSON y
en un par `field=value` con color en la salida pretty. Prefiere los
campos a la interpolación - son buscables en los registros JSON y el
formateador se encarga de renderizarlos según su tipo.

Para envolver una función en un span y estampar campos compartidos en
cada evento de su interior, usa `#[instrument]`:

```rust
#[instrument(skip(db), fields(user_id = %user_id))]
pub async fn load_dashboard(
    db: &suprnova::DatabaseConnection,
    user_id: i64,
) -> Result<Dashboard, FrameworkError> {
    info!("loading"); // lleva user_id del span automáticamente
    // … consultas …
}
```

El mismo `#[instrument]` se convierte en un span de OpenTelemetry cuando
la feature `otel` está habilitada - consulta
[Observabilidad](observability.md#opentelemetry).

## Niveles de registro

`LOG_LEVEL` es una [directiva de env-filter de
`tracing-subscriber`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html),
no un único nivel. La gramática son pares `target=level` separados por
comas, donde los valores sueltos fijan el nivel por defecto:

```env
LOG_LEVEL=info                                  # todo a partir de info
LOG_LEVEL=debug                                 # todo a partir de debug
LOG_LEVEL=info,sqlx=warn                        # info por defecto, sqlx más callado
LOG_LEVEL=warn,suprnova::queue=debug,my_app=info  # warn por defecto, dos targets verbosos
```

Los targets suelen ser el crate o la ruta de módulo que emite
(`suprnova::queue`, `hyper::server`, `my_app::services::checkout`).
Encuentra un target leyendo la línea de registro JSON - el campo `target`
de cada evento es su clave de filtrado.

Niveles en orden de verbosidad creciente: `error` < `warn` < `info` (por
defecto) < `debug` < `trace`. La respuesta de error que llega al cliente
siempre se sanitiza a `{"message": "Internal Server Error"}` sea cual sea
el nivel - el detalle va solo al registro estructurado.

### Las directivas inválidas no tumban el arranque

Un `LOG_LEVEL` mal formado (por ejemplo, `LOG_LEVEL=app=notalevel`)
recurre a `"info"` y escribe una advertencia de una línea en `stderr`:

```text
suprnova: invalid LOG_LEVEL directive "app=notalevel" (...); falling back to "info". Fix LOG_LEVEL to silence this.
```

Es `stderr` en vez de `tracing::warn!` porque el subscriber todavía no se
ha instalado - un `warn!` se descartaría en silencio. Corrige la
directiva y la advertencia desaparece.

## Salida pretty frente a JSON

El mismo `info!(user_id = 42, "saved")` se renderiza de forma distinta
según el formato.

**Pretty (desarrollo):**

```text
  2026-05-30T22:14:08.221341Z  INFO request{request_id=78a9...} my_app::handlers::checkout: saved
    at src/handlers/checkout.rs:48
    in checkout
    in request with request_id: 78a9..., method: POST, path: /checkout
```

**JSON (producción):**

```json
{
  "timestamp": "2026-05-30T22:14:08.221341Z",
  "level": "INFO",
  "fields": { "message": "saved", "user_id": 42 },
  "target": "my_app::handlers::checkout",
  "span": { "name": "checkout" },
  "spans": [
    { "name": "request", "request_id": "78a9...", "method": "POST", "path": "/checkout" }
  ]
}
```

La forma JSON es lo que los agregadores de producción (Datadog, Loki,
Honeycomb, CloudWatch, …) analizan sin configuración. `span.request_id`
es la clave de correlación - ver abajo.

## Correlación por id de solicitud

Toda solicitud HTTP recibe un `RequestId` de `RequestIdMiddleware`, el
middleware más externo de todas las cadenas. El id:

- **Se reutiliza** a partir de un encabezado `X-Request-Id` entrante que
  sea seguro (alfanuméricos más `- _ . :`, hasta 128 bytes), o **se acuña
  uno nuevo** como UUID v4 si falta o no es seguro.
- **Se devuelve** en la respuesta como `X-Request-Id` (tanto en las
  variantes 2xx como en las 5xx).
- **Se acota** dentro de un span `request` de `tracing`, de modo que cada
  evento de cualquier middleware, handler o biblioteca posterior lleva
  `request_id` en su array `spans` automáticamente.
- **Se siembra** en la bolsa `Context` de la solicitud como
  `_request_id`, de modo que los emisores que quieran la cadena a secas
  (trabajos, payloads de difusión, informes de error) puedan leerlo por
  su nombre.

Léelo desde el código con `current_request_id()`:

```rust
use suprnova::current_request_id;
use tracing::info;

if let Some(id) = current_request_id() {
    info!(request_id = %id, "checkpoint reached");
}
```

`current_request_id()` devuelve `Option<RequestId>` porque las tareas
en segundo plano (jobs en cola, tareas programadas, tests que no
instalaron el middleware) se ejecutan fuera de cualquier alcance de
solicitud.

### Tareas en segundo plano: lanzarlas con el id

`tokio::spawn` arranca una tarea nueva con los task-locals vacíos - un
handler que lanza trabajo con efectos secundarios pierde
`current_request_id()` y sus eventos de registro quedan huérfanos. Usa
`spawn_with_request_id` en su lugar:

```rust
use suprnova::spawn_with_request_id;
use tracing::info;

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    let order = place_order().await?;

    spawn_with_request_id(async move {
        // Esta tarea sigue observando current_request_id().
        // Sus eventos de registro llevan el mismo request_id que los del handler.
        info!(order_id = order.id, "post-checkout fanout running");
        send_receipt(order.id).await;
        update_analytics(order.id).await;
    });

    suprnova::Response::ok().json(&order)
}
```

El ayudante propaga tanto el task-local `RequestId` como el
`tracing::Span` actual, de modo que los eventos del future lanzado se
anidan bajo el mismo span `request` en el registro. Fuera de un alcance
de solicitud activo recae en un `tokio::spawn` a secas - se puede usar
sin condiciones.

Solo el id de solicitud y el span de tracing siguen a la tarea - la bolsa
`Context` de la solicitud deliberadamente no, porque el trabajo en
segundo plano no está atendiendo la solicitud HTTP que lo originó.

## El subscriber

El framework instala un subscriber global de `tracing` en el arranque
desde `Server::run()`. Casi nunca lo llamas tú; está documentado porque
los tests, quienes lo integran en otro programa y los puntos de entrada
poco habituales a veces sí lo necesitan.

```rust
use suprnova::{LogConfig, init_subscriber};

// Lee LOG_LEVEL / LOG_FORMAT del entorno:
init_subscriber(LogConfig::from_env());

// O de forma programática:
init_subscriber(LogConfig {
    level: "info,sqlx=warn".to_string(),
    format: suprnova::LogFormat::Json,
});
```

`init_subscriber` es **idempotente**. Una segunda llamada deja en su
sitio el subscriber existente y emite un `tracing::warn!` para que un
operador pueda ver que la nueva `LogConfig` no se aplicó. Esto es lo que
permite que los tests que llaman cada uno a `init_subscriber` no compitan
entre sí - gana el primero, el resto no hacen nada.

Para la variante consciente de OTel (la misma `LogConfig`, más la
exportación de trazado distribuido), usa
[`init_telemetry`](observability.md#opentelemetry).

### Los demonios

`queue:work`, `schedule:work`, `schedule:run` y `workflow:work` son
subcomandos del binario de tu aplicación y no arrancan a través de
`Server::run()`, así que instalan su propio subscriber en el arranque.
Leen el mismo `LOG_LEVEL` y el mismo `LOG_FORMAT` que el servidor, y tú
no llamas a nada:

```bash
LOG_LEVEL=info,suprnova::queue=debug cargo run --bin my-app -- queue:work

# …o, en un contenedor, contra el binario ya construido:
LOG_LEVEL=info my-app queue:work
```

Antes de la 0.9.1 esa ruta no instalaba absolutamente nada. Cada línea
`tracing::` que emiten los demonios no iba a ninguna parte y `LOG_LEVEL`
era inerte para ellos, lo que en un contenedor dejaba el banner de
arranque como única salida - un worker mandando trabajos a la cola de
fallidos, un planificador saltándose un tick cuya elección había
perdido, y un bloqueo que no pudo soltar, todo se veía idéntico a un
proceso ocioso. Si estás ejecutando una compilación fijada anterior a la
0.9.1 y te preguntas por qué un worker no dice nada, ese es el motivo, y
el arreglo es la actualización, no un cambio de configuración.

La mayor parte de lo que un worker tiene que decir lo dice en `warn!` y
`error!` - un trabajo que agota sus intentos, un trabajo fallido que no
pudo persistir, un bloqueo que no pudo soltar - así que el nivel `info`
por defecto basta para ver los problemas. Baja a `debug` cuando necesites
también las decisiones más silenciosas.

## Pruebas

Los tests no necesitan instalar un subscriber - el atributo
`#[suprnova_test]` y `TestContainer::fake` montan maquinaria suficiente
para que fluyan los eventos de los handlers. Si quieres hacer aserciones
sobre la salida de registro, captúrala con
[`tracing_subscriber::fmt::TestWriter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/struct.TestWriter.html)
de `tracing-subscriber` o con una capa propia; el framework
deliberadamente no incluye un fake del tipo "captura todos los registros
de este test" porque los patrones de prueba estándar de
`tracing-subscriber` funcionan sin fricción.

## Por qué Suprnova diverge

Laravel usa [Monolog](https://github.com/Seldaek/monolog) - cadenas de
mensaje con arrays de contexto opcionales, canales de registro y handlers
por canal (archivo, syslog, Slack, …). El modelo de una solicitud por
proceso de PHP hace que un único logger estático global sea seguro: cada
solicitud recibe su propio proceso y su propio contexto.

El modelo de procesos de Rust es lo opuesto - un solo proceso atiende
muchas solicitudes concurrentes en muchos hilos. Un formateador de
cadenas global tendría una condición de carrera sobre el
contexto y exigiría cablear
`request_id` de forma explícita en cada sitio de llamada. `tracing`
resuelve ambas cosas con campos estructurados y spans task-local: sin
cableado, los campos siguen tipados, y la correlación es automática
porque el span de la solicitud sigue vigente para cada evento que emite
la cadena.

La salida exclusiva por `stdout` también es intencionada. En despliegues
en contenedores (la única forma en que se distribuye Suprnova) el dueño
de la persistencia de los registros es el runtime, no la aplicación - la
rotación de archivos, la retención y el envío pertenecen todos a la
plataforma.

## Siguiente

- [Observabilidad](observability.md) - OpenTelemetry, registro de
  consultas, la superficie completa para operadores
- [Contexto](context.md) - la bolsa por solicitud donde viven
  `_request_id` y otros campos contextuales
- [Manejo de errores](errors.md) - cómo el límite de pánico del framework
  y la ruta de los 5xx emiten sus propios eventos estructurados
- [Variables de entorno](env-vars.md) - referencia de `LOG_LEVEL` y
  `LOG_FORMAT`

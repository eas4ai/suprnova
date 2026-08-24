# Cola

La fachada `Queue` despacha trabajo en segundo plano hacia un driver y
deja que un proceso worker separado la drene: los handlers HTTP retornan
rápido, el trabajo pesado se ejecuta detrás de escena. Recurre a ella
cada vez que una solicitud de otro modo bloquearía en algo que puede
hacerse después - enviar correo, invocar un webhook, generar un reporte.
Emparéjala con [`Bus`](bus.md) cuando quieras que el trabajo se ejecute
*ahora* en la tarea actual y devuelva un resultado tipado; emparéjala con
[`Events`](events.md) cuando quieras que una sola señal se disperse hacia
muchos oyentes.

## Inicio rápido

Define un job, regístralo una vez en el arranque, encólalo:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use suprnova::{error::FrameworkError, queue::{Job, Queue}};

#[derive(Serialize, Deserialize)]
struct SendWelcomeEmail { user_id: i64 }

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        // … de verdad enviar el correo
        Ok(())
    }
}

// Arráncalo una vez (lo necesitan tanto el proceso worker como el proceso que despacha).
Queue::set_driver(std::sync::Arc::new(suprnova::queue::MemoryQueueDriver::new()));
suprnova::queue::worker::register_job::<SendWelcomeEmail>();

// Encola desde un handler:
Queue::push(SendWelcomeEmail { user_id: 42 }).await?;
```

Un proceso worker drena el driver configurado hasta que se lo cancela:

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use suprnova::queue::{Queue, worker::{WorkerConfig, run_worker}};

let driver = Queue::driver()?;
let cfg = WorkerConfig {
    visibility_timeout: Duration::from_secs(60),
    poll_interval: Duration::from_millis(100),
    max_jobs: None,
    queues: Vec::new(),
};
let shutdown = CancellationToken::new();
run_worker(driver, cfg, shutdown).await;
```

En una aplicación con andamiaje, el worker se arranca mediante el
subcomando `queue:work` del binario - `cargo run -- queue:work` - que
ejecuta el mismo bootstrap que tu servidor HTTP, así que los observadores
y los oyentes registrados en `bootstrap()` se disparan de forma idéntica
para las inserciones provenientes de un handler de cola.

## Drivers

Cinco drivers vienen incluidos en el árbol del framework. Configúralos
mediante la variable de entorno `QUEUE_DRIVER` o llamando a
`Queue::set_driver(...)` programáticamente.

| Driver | Úsalo para | Puntos fuertes |
| --- | --- | --- |
| `MemoryQueueDriver` | tests, apps de un solo proceso | `tokio::time::DelayQueue` para `available_at`, compatible con reloj virtual |
| `RedisQueueDriver` | dispersión en producción | grupos de consumidores + `XAUTOCLAIM` + jobs retrasados respaldados por ZSET |
| `DatabaseQueueDriver` | apps de una sola base de datos | `FOR UPDATE SKIP LOCKED` en Postgres/MySQL, serializado con `BEGIN` en SQLite |
| `SyncQueueDriver` | desarrollo, CI | ejecuta el handler en línea en `push`, sin worker |
| `NullQueueDriver` | envoltorios de test | descarta cada push sin ejecutarlo |

`Queue::bootstrap_from_env()` lee `QUEUE_DRIVER` y conecta el driver
correspondiente; `Queue::bootstrap_default()` siempre conecta el driver
de memoria. La ruta de arranque del servidor llama a uno de estos por
ti - la mayoría de las apps solo se configuran vía entorno.

### Configuración del entorno

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# Driver de base de datos - DB::init() debe ejecutarse primero
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

El driver de base de datos valida `QUEUE_DB_TABLE` como un identificador
SQL en el momento de construirse, así que un valor de entorno malformado
hace fallar el arranque en lugar de llegar a la composición SQL. Redis
usa sea-streamer-redis por debajo con `AutoCommit::Disabled`; el timeout
de visibilidad queda fijado en el momento de construir el grupo de
consumidores, así que el argumento `visibility_timeout` de cada pop se
ignora en Redis (una divergencia documentada respecto al contrato del
trait, impuesta por Redis Streams).

### Por qué Suprnova diverge

Laravel enruta todo lo encolable a través del Bus, distinguiendo los
jobs `ShouldQueue` en el momento del despacho. Suprnova separa los dos:
`Bus` para trabajo síncrono que devuelve un resultado tipado, `Queue`
para trabajo asíncrono que sobrevive a una caída del proceso. PHP
necesita el enrutamiento implícito porque su modelo de un proceso por
solicitud hace difícil modelar de otro modo "hacer esto después, en otro
proceso". Tokio no - `Bus::dispatch` frente a `Queue::push` de forma
explícita es más claro, más rápido, y hace visible la elección de
durabilidad en el sitio de la llamada. Consulta [`bus.md`](bus.md) para
la comparación en paralelo.

## Variantes de push

Toda variante de push toma un valor tipado `J: Job` y retorna cuando el
sobre queda confirmado en el driver - no cuando se ejecuta el handler.

| Método | Comportamiento |
| --- | --- |
| `Queue::push(job)` | encola de inmediato |
| `Queue::push_later(job, at)` | disponible en un `DateTime<Utc>` concreto |
| `Queue::later(delay, job)` | disponible después de `delay` desde ahora |
| `Queue::push_with(job, overrides)` | encola de inmediato con `EnvelopeOverrides` por push |
| `Queue::later_with(delay, job, overrides)` | disponible después de `delay` desde ahora, con `EnvelopeOverrides` por push |
| `Queue::push_unique(job)` | deduplica por `J::unique_id` dentro de `J::unique_for`, devuelve `Ok(true)` cuando se empujó el sobre, `Ok(false)` cuando una clave de deduplicación activa lo suprimió |
| `Queue::push_unique_later(job, at)` | único + programado |
| `Queue::later_unique(delay, job)` | único + retrasado |
| `Queue::bulk(vec![job1, job2, ...])` | empuja todos los jobs (el driver puede usar una ruta masiva nativa) |

`push_unique` requiere que la capa de caché esté arrancada - el bloqueo
de deduplicación vive en [`Cache`](cache.md) vía
[`Idempotency::commit_on_success`](idempotency.md). Un push fallido
libera la clave de deduplicación para que quien llama pueda reintentar;
un push con éxito la retiene durante `J::unique_for` segundos. El job
debe sobrescribir `Job::unique_id(&self)` para devolver `Some(id)` -
`None` devuelve un error interno.

El booleano responde a una sola pregunta - "¿está este job en la cola?" -
y detrás de ella hay un tercer caso. Si el lease del bloqueo de
deduplicación se pierde mientras el push está en vuelo, el push se
completa igualmente (la capa de idempotencia nunca cancela un cuerpo que
puede haber tenido ya un efecto) y sigues obteniendo `Ok(true)`, con un
log de nivel `warn` que nombra el job y su clave única. El job está
encolado; lo que no queda demostrado es que nadie más encolara el mismo
de forma concurrente. Tu handler ya tiene que tolerar la reentrega, así
que esto no necesita ningún manejo extra - pero el log está ahí porque
una ráfaga de ellos significa que la caché que respalda tu bloqueo de
deduplicación está sufriendo.

### Anulaciones por push con `EnvelopeOverrides`

`Queue::push_with` y `Queue::later_with` reciben un `EnvelopeOverrides`
junto con el job, para el único despacho que necesita un comportamiento de
cola, conexión, timeout o reintento diferente de los propios valores
predeterminados del job:

```rust
use std::time::Duration;
use suprnova::queue::{EnvelopeOverrides, Queue};

let overrides = EnvelopeOverrides {
    queue: Some("priority".into()),
    timeout: Some(Duration::from_secs(10)),
    max_tries: Some(1),
    ..Default::default()
};

Queue::push_with(SendWelcomeEmail { user_id: 42 }, overrides.clone()).await?;

// The delayed counterpart, mirroring `Queue::later`'s relationship to `Queue::push`.
Queue::later_with(Duration::from_secs(60), SendWelcomeEmail { user_id: 42 }, overrides).await?;
```

Cada campo tiene como predeterminado `None` y difiere a la resolución normal
que ya ejecuta `Queue::push`; un campo `Some` gana sobre todo ello para este
único push, superando tanto una ruta registrada con
[`Queue::route`](#queue-routing) como la declaración `Job::*` del propio job
para ese campo:

| Campo | Supera a |
| --- | --- |
| `queue` | `Queue::route`, `Job::queue()` |
| `connection` | `Queue::route`, `Job::connection()` |
| `timeout` | `Job::timeout()` |
| `fail_on_timeout` | `Job::fail_on_timeout()` |
| `max_tries` | `Job::max_tries()` |
| `backoff` | `Job::backoff()` |

`EnvelopeOverrides` es la primitiva sobre la que se construyen
`Mail::on_queue`/`.on_connection()` y el ajuste de cola por notificación de
`Notify::queue`: consulta [Correo](mail.md#queueing) y
[Notificaciones](notifications.md).

### Retraso declarado por el job

Un job puede llevar su propio retraso predeterminado en lugar de que cada sitio
de llamada repita `Queue::later(Duration::from_secs(60), job)`:

```rust
impl Job for SendDigest {
    // ...
    fn delay() -> Option<Duration> { Some(Duration::from_secs(60)) }
}
```

`Queue::push(job)`, `Queue::push_with(job, overrides)`, `Queue::push_unique(job)`
y `Queue::bulk(vec![job1, job2])` lo respetan: `available_at` pasa a ser
`now + J::delay()` en lugar de `now`. `Queue::bulk` resuelve el retraso una
vez por llamada, ya que cada job del vector comparte el mismo `J` concreto y,
por lo tanto, el mismo `Job::delay()`.

Un retraso explícito en el sitio de llamada siempre gana:
`Queue::push_later(job, at)`, `Queue::later(delay, job)`,
`Queue::later_with(delay, job, overrides)`,
`Queue::push_unique_later(job, at)` y `Queue::later_unique(delay, job)`
usan todos literalmente el timestamp o retraso que pasó quien llama:
`Job::delay()` no se consulta para ninguno. Usa el método del trait cuando
cualquier despacho de un tipo de job deba comenzar retrasado por defecto; usa
una de las variantes `later`/`push_later` para un retraso que necesita un
despacho específico, pero que el tipo no declara de otro modo.

Los lotes y las cadenas tampoco lo consultan:
`Queue::batch()...add(job)` y `Queue::chain()...add(job)?` construyen sus
envelopes con `available_at` establecido en el momento en que llamaste a
`add`, de modo que un job con `Job::delay()` declarado se despacha de inmediato
como parte de un lote o una cadena aunque un `Queue::push(job)` simple del
mismo job esperaría. Da al job un retraso explícito de otra forma  - un campo en
el propio job, aplicado en `handle()` -  si un paso en lote o encadenado necesita
uno.

### Por qué Suprnova diverge

El `$job->delay` de Laravel es una propiedad de instancia, establecida por
despacho (`SendDigest::dispatch($user)->delay(60)`), por lo que dos despachos
de la misma clase pueden llevar retrasos distintos. Aquí `Job::delay()` es en
cambio un valor predeterminado a nivel de clase, como `Job::queue()` o
`Job::max_tries()`: un despacho que necesita un retraso calculado a partir de
sus propios datos usa `Queue::later`/`push_later`, que ya supera el
predeterminado declarado.

## Configuración del job

Anula las funciones asociadas de `Job` para ajustar el comportamiento por
cada impl:

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

    fn delay() -> Option<Duration> { None }                // default: no delay

    fn max_tries() -> u32 { 5 }                            // por defecto: 3
    fn timeout() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn fail_on_timeout() -> bool { false }                 // por defecto: false (el timeout reintenta)
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Sequence { secs: vec![5, 15, 60, 300] }
    }
    fn unique_id(&self) -> Option<String> {
        Some(format!("welcome:{}", self.user_id))
    }
    fn unique_for() -> Duration { Duration::from_secs(600) }  // por defecto: 5 minutos
    fn middleware() -> Vec<std::sync::Arc<dyn JobMiddleware>> {
        vec![/* ver "Middleware de job" más abajo */]
    }
}
```

## Enrutamiento de colas

Por defecto cada job va a una sola cola y cada worker la drena por
completo. En cuanto algunos jobs son más lentos o más importantes que
otros, quieres pools de workers dedicados: una exportación de larga
duración no debería quedar detrás de mil correos de bienvenida.

Un job puede declarar a dónde pertenece:

```rust
#[async_trait]
impl Job for GenerateExport {
    fn job_name() -> &'static str { "GenerateExport" }
    async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }

    fn queue() -> Option<&'static str> { Some("exports") }
    fn connection() -> Option<&'static str> { None }   // conexión por defecto
}
```

…y un operador puede anular eso de forma centralizada, sin tocar el job:

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::route::<GenerateExport>(None, Some("heavy"));
Queue::route::<SendInvoice>(Some("redis"), Some("billing"));
```

La resolución se ejecuta en orden de mayor prioridad primero:

1. una anulación por push pasada a `Queue::push_with` / `Queue::later_with`
   (consulta [Anulaciones por push con `EnvelopeOverrides`](#per-push-overrides-with-envelopeoverrides))
2. una ruta registrada con `Queue::route`
3. el propio `Job::queue` / `Job::connection` del job
4. el driver / el valor por defecto global

Pasar `None` para un campo deja esa dimensión intacta, así que enrutar la
conexión de un job no perturba la cola que ya había declarado.

Las dos dimensiones se ejecutan hoy a profundidades distintas. La
**cola** se respeta de punta a punta - se estampa en el sobre, se
almacena en el driver, se filtra con `--queue`. La **conexión** resuelve
el *nombre* de conexión que llevan los eventos de ciclo de vida
`JobQueueing` / `JobQueued`, que es lo que ven los oyentes y los
dashboards; un único driver global de proceso sigue recibiendo cada push,
así que enrutar la conexión de un job todavía no selecciona un driver
distinto. Declarar conexiones ahora es compatible hacia adelante para
cuando lleguen los drivers por conexión, no algo con efecto de
comportamiento todavía.

Luego dedícale un worker:

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # drena todas las colas, como antes
```

Un job sin ruta pertenece a `default`, así que `--queue=default` drena el
trabajo sin enrutar en lugar de dejarlo varado.

### Por qué Suprnova diverge

El `Queue::route(...)` de Laravel toma un string de clase; Suprnova toma
el job como un parámetro de tipo, así que un job renombrado o eliminado
es un error de compilación en lugar de una ruta que silenciosamente deja
de coincidir.

La divergencia más grande es qué sucede cuando un driver no puede
filtrar. `QueueDriver::pop_from` **rechaza** un filtro de cola que no
puede respetar en lugar de recurrir a drenar todo. Un worker al que se le
dijo que drenara solo `billing` y que silenciosamente drena todas las
colas se ve idéntico a un despliegue que funciona, hasta que el pool
equivocado consume los jobs equivocados - así que la mala configuración
se vuelve evidente desde el primer sondeo. Los drivers de memoria y de
base de datos filtran de forma nativa; un driver que no lo hace - el
driver de Redis es uno de ellos, ya que un único grupo de consumidores de
stream no tiene almacenamiento por cola - dará un error en lugar de
engañar.

### La tabla `jobs`

`DatabaseQueueDriver` espera este esquema. La columna `queue` es lo que
hace posible el filtrado con `--queue`:

```sql
CREATE TABLE jobs (
    id              TEXT PRIMARY KEY,
    job_name        TEXT NOT NULL,
    queue           TEXT NULL,
    envelope_json   TEXT NOT NULL,
    available_at    BIGINT NOT NULL,
    reserved_until  BIGINT NULL,
    reserved_token  TEXT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    created_at      BIGINT NOT NULL
);
CREATE INDEX idx_jobs_available_at ON jobs(available_at);
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

`queue` admite nulos, y un job sin enrutar almacena `NULL` en lugar de
`'default'`. Eso es deliberado: una fila escrita por un binario más
antiguo es indistinguible de una fila sin enrutar escrita por uno nuevo,
así que una flota de versiones mixtas drena el mismo trabajo durante una
actualización progresiva.

Añadir la columna a una tabla existente es **obligatorio**, no solo para
el filtrado: `push` nombra la columna `queue` en su `INSERT` sin importar
si el job está enrutado o no, así que un binario 0.7.0+ falla en cada
push contra una tabla que no la tenga. Ejecuta primero la migración, y
luego despliega los binarios - los binarios más antiguos listan sus
columnas explícitamente e ignoran la nueva, así que ese orden es seguro:

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

### Esquemas de backoff

| Variante | Comportamiento |
| --- | --- |
| `Fixed { secs }` | retardo constante por intento |
| `Exponential { base_secs, cap_secs, jitter_ratio }` | `min(base * 2^(attempts-1), cap)` × aleatorio en `[1±jitter]` |
| `Sequence { secs }` | una entrada por intento; la última entrada se repite una vez agotadas |

El valor por defecto es `Exponential { base_secs: 2, cap_secs: 300,
jitter_ratio: 0.25 }` - de 2 segundos a 5 minutos con un jitter de ±25%.

## Middleware de job

Seis middleware vienen incluidos en el árbol, todos reflejando
`Illuminate\Queue\Middleware\*`:

| Middleware | Comportamiento |
| --- | --- |
| `WithoutOverlapping` | sostiene un `Cache::lock` durante toda la duración; libera con retardo ante contención |
| `RateLimited` | se condiciona al presupuesto del `RateLimiter`; libera hasta que la ventana se reinicia |
| `ThrottlesExceptions` | limita la velocidad según *fallos* consecutivos, no según solicitudes |
| `Skip::when(cond)` / `Skip::unless(cond)` | descarta el job cuando se cumple la condición |
| `FailOnException` | promueve los errores que coinciden a fallos permanentes (sin reintento) |
| `SkipIfBatchCancelled` | descarta el job si su lote propietario fue cancelado |

Conéctalos en el impl de `Job`:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{JobMiddleware, RateLimited, WithoutOverlapping};

fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
    vec![
        Arc::new(
            WithoutOverlapping::new("user-42")
                .expire_after(Duration::from_secs(120))
        ),
        Arc::new(
            RateLimited::new(10, Duration::from_secs(60))
                .by("send-mail")
        ),
    ]
}
```

`WithoutOverlapping` y `RateLimited` necesitan que el subsistema de caché
esté arrancado (`Cache::init` o `App::bind::<dyn CacheStore>(...)` en el
arranque).

### Un bloqueo que no se libera no hace fallar el job

Si `WithoutOverlapping` no puede liberar su bloqueo después de que el
handler se ejecutó - el backend de caché tuvo un tropiezo, la conexión se
cayó - registra en `warn` y de todos modos devuelve el resultado propio
del handler. El bloqueo entonces caduca en `expire_after`.

Eso es deliberado. Para cuando se ejecuta la liberación, el handler ya
confirmó sus efectos secundarios: filas escritas, correo enviado, cobros
hechos. Reportar el fallo de liberación como un fallo del job haría que
el worker reintentara y repitiera todo por segunda vez, lo cual es un
resultado peor que una clave de bloqueo sostenida durante su TTL. Un
handler que de verdad falló sigue reportando su fallo - suprimir el error
de liberación no suprime el del handler.

### El contrato de liberar sin gastar un intento

El middleware devuelve un `JobOutcome` en lugar de un `Result<()>`.
Cuatro variantes:

- `JobOutcome::Completed` - el handler se ejecutó, ack.
- `JobOutcome::Released { delay }` - se reencola después de `delay`
  **sin** incrementar `attempts`. Lo usan `WithoutOverlapping` y
  `RateLimited`. El worker le entrega toda la operación a
  `QueueDriver::release`, y cada driver incluido en el árbol reencola su
  propia copia almacenada en el sitio, así que el mensaje nunca está
  simultáneamente reservado y visible, y nunca ninguno de los dos. El
  contador de intentos se preserva sin ninguna aritmética en el worker
  con la que un driver pueda estar en desacuerdo - la copia almacenada
  nunca se incrementó para esta ejecución.
- `JobOutcome::Failed { reason }` - se envía a fallidos de inmediato, se
  persiste en el store de jobs fallidos, no se reintenta.
- `JobOutcome::Deleted` - descarta la reserva sin enviarlo a fallidos. Lo
  usa `Skip`. Si el job pertenecía a un lote, el `pending_jobs` del lote
  decrementa de todos modos para que los callbacks puedan dispararse.

Este contrato es lo que hace que "limitado porque el cubo estaba
lleno" se sienta distinto de "fallido porque el handler tuvo un error" en
la contabilidad de reintentos, las métricas, y los eventos de ciclo de
vida.

### Qué cuenta como un intento

Hay dos formas en que un job abandona un worker sin terminar, y ambas
consumen un intento:

- **El handler falló** - devolvió `Err`, o entró en pánico dentro del
  límite del framework. El worker envía un nack; el driver reencola con
  `attempts + 1`.
- **El worker murió** - un kill por OOM, `abort()`, un segfault, `docker
  kill`, o el SIGKILL que envía un supervisor cuando una detención agota
  su timeout. Nada resuelve nada; la reserva simplemente caduca. El
  worker que sea que reclame el job carga el intento en ese momento.

El segundo caso solía ser gratis, y eso era un agujero más que una
gentileza: un job que mata de forma confiable a su worker nunca podría
agotar `max_tries` y por lo tanto nunca podría enviarse a fallidos.
Mataría a cada worker que lo reclamara, volvería byte por byte idéntico,
y mataría al siguiente, mientras algo siguiera reiniciando workers.

Los tres drivers incluidos en el árbol lo cargan, porque cambiar
`QUEUE_DRIVER` no debe alterar si un trabajo envenenado puede detenerse.
`database` detecta un `reserved_until` caducado; `memory` lo carga cuando
el reaper devuelve la reserva a visible; `redis` lee el contador de
entregas de la entrada desde `XPENDING`, ya que una entrada de Redis
stream es inmutable y su propio contador es el único registro.

`JobOutcome::Released` es la excepción deliberada - consulta el contrato
de arriba. Un job limitado por `RateLimited` nunca se ejecutó, así que no
debe nada.

**En Redis, el reclamo tiene dos relojes.** `--visibility-timeout` fija
cuánto tiempo debe permanecer una entrada sin hacer ack antes de
calificar para el reclamo; un segundo intervalo gobierna con qué
frecuencia mira un consumidor. El driver ata el segundo al primero, así
que un job perdido vuelve dentro de aproximadamente el doble del timeout
configurado, en lugar del timeout más 30 segundos fijos.

**El presupuesto se comprueba antes de que el handler se ejecute, no
solo al resolverse.** Toda otra decisión de enviar a fallidos ocurre
después de que un handler retorna, lo cual asume que el handler retorna.
Un job que mata a su worker no puede llegar a esa comprobación, así que
el worker también se niega a despachar un job cuyos intentos ya están
agotados - en su lugar lo envía a fallidos, antes de que derribe a otro
worker. Sin esto, contar el intento solo haría subir un número mientras
el job seguía circulando.

**Qué significa esto para ti.** `attempts` cuenta *entregas a un
worker*, no *fallos del handler*. Un worker perdido por razones ajenas al
job - un reinicio del host, un OOM causado por un vecino ruidoso -
también gasta un intento del presupuesto de ese job. Laravel se comporta
igual. Dimensiona `max_tries` teniendo esto en cuenta, y prefiere
handlers idempotentes: la entrega al menos una vez siempre fue el
contrato, y esto hace que la ruta de reentrega cuente con honestidad en
lugar de en silencio.

## Eventos de ciclo de vida

Los workers emiten eventos de ciclo de vida con forma de Laravel a través
de la fachada [`Event`](events.md). Los oyentes reciben la identidad del
sobre (`id`, `job_name`, `attempts`, `max_tries`, `connection`), no la
instancia tipada del job - el worker pierde el tipo concreto y solo
opera sobre payloads JSON. Los errores viajan como un `String` ya que
`FrameworkError` no deriva `Clone`.

| Evento | Se dispara cuando… |
| --- | --- |
| `JobQueueing` | antes de que el sobre llegue al driver |
| `JobQueued` | después de que el driver acepta |
| `UniqueJobSkipped` | `push_unique` suprimió un duplicado dentro de la ventana `unique_for` |
| `JobProcessing` | el worker extrajo el job, a punto de despacharlo |
| `JobProcessed` | el handler devolvió `Ok` |
| `JobAttempted` | cada resolución terminal (éxito, fallo, timeout) |
| `JobExceptionOccurred` | el handler devolvió `Err`, se reintentará |
| `JobReleasedAfterException` | ocurrió el reencolado tras un error para reintentar |
| `JobReleased` | liberación impulsada por middleware (sin fallo) |
| `JobFailed` | enviado a dead-letter |
| `JobTimedOut` | se superó el timeout por intento |
| `Looping` | cada iteración del bucle (antes de extraer) |
| `WorkerStarting` / `WorkerStopping` | una vez por cada vida del worker |
| `WorkerInterrupted` | se observó la señal de `Queue::restart()` |
| `QueuePaused` | `Queue::pause` estableció el interruptor propio de una cola |
| `QueueResumed` | `Queue::resume` limpió el interruptor propio de una cola |
| `QueuesPaused` | `Queue::pause_all` estableció el interruptor global |
| `QueuesResumed` | `Queue::resume_all` limpió el interruptor global |

Suscríbete con la API normal de `Event::listen`. Los eventos son
best-effort - `Event::dispatch` sin oyentes es un no-op `Ok(())`, así que
los workers en despliegues sin `Event::init()` no pagan ningún costo.

`UniqueJobSkipped` es el único evento que se dispara en el lado de
*push* en vez del lado del worker, y el único que informa de una no-falla.
Lleva `job_name`, `unique_id` y `connection`: la decisión de deduplicación
ocurre antes de que exista un sobre, así que no hay id de sobre que
informar. El push aún devuelve `Ok(false)`; el evento hace observable una
supresión que de otro modo sería invisible.

`QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` se disparan
del mismo modo: desde `Queue::pause` / `resume` / `pause_all` / `resume_all`
en sí, no desde el bucle del worker. Tampoco llevan identidad de sobre;
consulta «Pausar colas» abajo para el contrato completo.

## Almacenamiento de jobs fallidos

Los jobs enviados a fallidos terminan en el `FailedJobStore` configurado:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryFailedJobStore};

Queue::set_failed_store(Arc::new(MemoryFailedJobStore::new()));

// En herramientas de administración:
let store = Queue::failed_store().unwrap();
for record in store.all().await? {
    println!("{} failed: {}", record.job_name, record.exception);
}
store.forget(some_id).await?;
store.flush(None).await?;
```

Tres backends:

- `MemoryFailedJobStore` - un `Vec` dentro del proceso, se pierde al
  reiniciar.
- `DatabaseFailedJobStore` - persiste en una tabla `failed_jobs` vía
  SeaORM.
- `NullFailedJobStore` - descarta cada registro. Refleja el
  `NullFailedJobProvider` de Laravel.

### Cuando el store rechaza un registro

Si el store configurado devuelve un error, el worker registra en `error`
y **deja la reserva intacta** en lugar de hacer ack. El job vuelve cuando
expira la visibilidad y se reintenta - no se descarta en silencio.

Eso es deliberado. La alternativa, hacer ack de todos modos, descarta un
job que ya agotó sus intentos *y* no logró registrarse en ningún lado, lo
cual es irrecuperable. Un job que sigue volviendo es recuperable: arregla
el store y la siguiente entrega llega.

El caso práctico es un `DatabaseFailedJobStore` apuntando a una tabla
`failed_jobs` sin migrar. Hasta que migres, los jobs que se envían a
fallidos siguen circulando a razón de una reentrega por cada timeout de
visibilidad, cada una registrando el error del store. Si de verdad
quieres que los fallos se descarten, configura `NullFailedJobStore` -
eso tiene éxito, así que el job hace ack y desaparece.

### Reintentar

```rust
use uuid::Uuid;

// Un solo registro - false si el id no estaba en el store.
Queue::retry_failed(some_id).await?;

// En bloque - corte opcional (solo reintenta registros más antiguos que `before`).
let count = Queue::retry_all_failed(None).await?;
```

`retry_failed` carga el sobre, reinicia `attempts`, `available_at`, y el
`idempotency_key`, lo encola a través del driver configurado, y luego
elimina el registro de job fallido. Refleja `php artisan queue:retry
<id>` más la semántica de `queue:flush` (cada sobre reintentado se
encola Y se elimina del store).

### Esquema de `failed_jobs`

El `DatabaseFailedJobStore` espera esta tabla (gestionada por tus
migraciones):

```sql
CREATE TABLE failed_jobs (
    id              TEXT PRIMARY KEY,
    connection      TEXT NOT NULL,
    queue           TEXT NOT NULL,
    job_name        TEXT NOT NULL,
    envelope_json   TEXT NOT NULL,
    exception       TEXT NOT NULL,
    failed_at       BIGINT NOT NULL
);
CREATE INDEX idx_failed_jobs_failed_at ON failed_jobs(failed_at);
```

El argumento `table` de `DatabaseFailedJobStore::new` se valida como un
identificador SQL en el momento de construirse.

## Lotes encolados

Despacha un grupo de jobs con seguimiento de progreso y callbacks de
finalización:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryBatchRepository, batch::register_callback};

Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));

// Registra callbacks con nombre en el arranque.
register_callback(Arc::new(SendSummary));
register_callback(Arc::new(PageOnFail));

let id = Queue::batch()
    .name("import-users")
    .add(ImportUser { id: 1 })
    .add(ImportUser { id: 2 })
    .add(ImportUser { id: 3 })
    .then("send-summary-email")
    .catch("page-on-fail")
    .finally("cleanup-temp-tables")
    .dispatch()
    .await?;

// Inspecciona el progreso más adelante:
let repo = Queue::batch_repository().unwrap();
let snap = repo.find(&id).await?.unwrap();
println!("{}/{} jobs done ({}%)", snap.processed_jobs(), snap.total_jobs, snap.progress());
```

Cada worker resuelve su job contra el lote, y cuando `pending_jobs` llega
a cero el worker dispara los callbacks `then`/`catch`/`finally`
registrados. Por defecto el primer fallo cancela el lote;
`.allow_failures()` mantiene en marcha los jobs restantes.

### Lotes durables

`MemoryBatchRepository` se pierde al reiniciar, lo cual deja varado cada
lote en vuelo: sus contadores desaparecen, `pending_jobs` nunca puede
volver a llegar a cero, y los callbacks nunca se disparan. Usa
`DatabaseBatchRepository` en producción:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, DatabaseBatchRepository};

Queue::set_batch_repository(Arc::new(DatabaseBatchRepository::new(db.clone())));
```

Dos tablas, que el framework no crea - añádelas a tus migraciones, de la
misma forma que funcionan `jobs` y `failed_jobs`:

```sql
CREATE TABLE job_batches (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    total_jobs    INTEGER NOT NULL,
    options_json  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    cancelled_at  INTEGER NULL,
    finished_at   INTEGER NULL
);

CREATE TABLE job_batch_settlements (
    batch_id   TEXT NOT NULL,
    job_id     TEXT NOT NULL,
    failed     INTEGER NOT NULL,
    settled_at INTEGER NOT NULL,
    PRIMARY KEY (batch_id, job_id)
);
```

`DatabaseBatchRepository::with_tables(db, batches, settlements)` te
permite nombrarlas tú mismo; ambos nombres se validan como identificadores
SQL en el momento de construirse.

Fíjate en lo que `pending_jobs` y `failed_jobs` **no** son: columnas. Se
derivan de las filas de resolución en cada lectura -

```text
pending_jobs = max(0, total_jobs - COUNT(settlements))
failed_jobs  = COUNT(settlements WHERE failed)
```
 -
porque las colas son al menos una vez, así que el mismo job se resuelve
más de una vez cada vez que ocurre una reentrega, se duplica un ack, o un
worker muere entre hacer el trabajo y registrarlo. Un contador
decrementado por cada resolución se desincroniza en cada uno de esos
casos, y esa desincronización no es cosmética: `pending_jobs` condiciona
los callbacks, así que un cero prematuro dispara `then` mientras otros
jobs del lote todavía se están ejecutando. Con los conteos derivados y la
clave primaria en `(batch_id, job_id)`, una resolución repetida no
inserta nada y no hay ningún contador que se pueda desincronizar - entre
procesos, no solo dentro de uno.

### Cuando un despacho falla a mitad de camino

Si un `driver.push` falla a mitad de `dispatch()`, los jobs que ya
llegaron a la cola son reales y ya están estampados con el id del lote.
Así que el lote se resuelve en lugar de eliminarse: cada sobre que *no*
se encoló se registra como un job fallido, y el lote se cancela.

`total_jobs` sigue contando lo que pediste, `failed_job_ids` nombra
exactamente los jobs que nunca llegaron, los que ya estaban encolados se
resuelven normalmente, y `SkipIfBatchCancelled` descarta el resto - así
que `pending_jobs` de todos modos llega a cero y tus callbacks
`catch`/`finally` de todos modos se ejecutan. Si no se encoló nada en
absoluto, `dispatch` los dispara él mismo, porque no queda ningún worker
que lo haga. De cualquier forma, recibes de vuelta el error de push
original.

### Opciones de lote

| Opción | Método del builder | Efecto |
| --- | --- | --- |
| Permitir fallos | `.allow_failures()` | continúa programando después de que un job falla |
| Callback then | `.then(name)` | se ejecuta cuando todos los jobs tienen éxito |
| Callback catch | `.catch(name)` | se ejecuta ante el primer fallo |
| Callback finally | `.finally(name)` | se ejecuta después de que el lote se resuelve, de cualquier forma |
| Omitir si se cancela | middleware `SkipIfBatchCancelled` en el job | descarta los jobs restantes cuando se cancela el lote |

### Impl de `BatchCallback`

```rust
use async_trait::async_trait;
use suprnova::queue::{Batch, BatchCallback};
use suprnova::error::FrameworkError;

pub struct SendSummary;

#[async_trait]
impl BatchCallback for SendSummary {
    fn name(&self) -> &'static str { "send-summary-email" }

    async fn handle(&self, batch: Batch, error: Option<String>) -> Result<(), FrameworkError> {
        let subject = match error {
            Some(_) => format!("Batch {} failed", batch.name),
            None    => format!("Batch {} done - {} jobs", batch.name, batch.total_jobs),
        };
        // … enviar correo
        Ok(())
    }
}
```

Regístralo en el arranque con
`batch::register_callback(Arc::new(SendSummary))`. Los callbacks se
indexan por `name()` - las opciones del lote almacenan nombres de
callback, así que un reinicio del proceso recupera los callbacks
registrados por búsqueda en lugar de intentar deserializar un closure
(los closures de Rust no serializan).

## Cadenas encoladas

Flujos secuenciales donde cada eslabón se ejecuta solo después de que el
handler del anterior hace ack:

```rust
Queue::chain()
    .add(GenerateReport { id: 99 })?
    .add(UploadToBucket { id: 99 })?
    .add(NotifyOwner { id: 99 })?
    .dispatch()
    .await?;
```

El primer sobre se encola de inmediato; el resto viaja en su campo de
payload `chain_remaining`. En cada resolución exitosa el worker extrae la
siguiente entrada y la despacha. Un fallo rompe la cadena - los eslabones
subsecuentes nunca se encolan.

### Resolución terminal

Terminar un job encadenado significa dos cosas: encolar al sucesor, y
liberar el job que acaba de terminar. Como dos operaciones separadas, no
hay un orden seguro. Haz ack primero, y una caída en el intervalo pierde
el resto de la cadena permanentemente - no queda nada en la cola desde
donde reintentar. Encola primero, y la misma caída reentrega el job
terminado, así que su handler se ejecuta de nuevo y el sucesor se encola
dos veces.

Así que el worker le entrega ambas cosas al driver a la vez, vía
`QueueDriver::settle(token, follow_ups)`:

| Resultado | Significado |
| --- | --- |
| `Settled::Atomically` | el sucesor se encoló y la reserva se descartó en una sola transacción |
| `Settled::Stale` | la reserva fue reclamada por otro consumidor; **no** se encoló ni se descartó nada |
| `Settled::Unsupported` | este driver no puede resolver de forma transaccional |

`DatabaseQueueDriver` lo implementa: ambos efectos son una sola
transacción, y el `DELETE` indexado por la reserva funciona también como
barrera. Si tu timeout de visibilidad expiró mientras el handler se
estaba ejecutando y otro worker recogió el job, el delete no coincide con
nada, la transacción se revierte, y obtienes `Stale` - sin haber
encolado nada. La resolución en dos pasos no puede expresar eso en
absoluto: tu push tiene éxito, el push del nuevo propietario tiene éxito,
y la cadena se bifurca.

Redis y el driver en memoria responden `Unsupported` y mantienen el
orden de push-antes-que-ack, lo cual cambia una pérdida permanente por un
duplicado de al menos una vez. Ese es el contrato documentado del
framework, y es la razón por la que los ids de sobre encadenados se
derivan de su predecesor en lugar de ser aleatorios - un paso reentregado
vuelve a encolar el mismo id que encoló antes, así que el duplicado es
reconocible como el mismo paso lógico.

Si escribes un driver cuya escritura de seguimiento y su acuse de recibo
comparten un dominio transaccional, implementa `settle`. Su valor por
defecto devuelve `Unsupported`, así que los drivers escritos antes de que
esto existiera siguen funcionando sin cambios.

## Introspección

```rust
Queue::size().await?;            // total
Queue::pending_size().await?;    // available_at <= ahora, sin reservar
Queue::delayed_size().await?;    // available_at > ahora
Queue::reserved_size().await?;   // extraído actualmente, sin ack todavía
Queue::clear().await?;           // descarta cada sobre, devuelve el conteo
Queue::driver_name()?;           // nombre del driver configurado, para logs / administración
```

El trait `QueueDriver` declara valores por defecto para `size` /
`pending_size` / `reserved_size` / `delayed_size` / `clear`;
`MemoryQueueDriver` y `DatabaseQueueDriver` los implementan de forma
nativa. `RedisQueueDriver` devuelve un error de "no soportado" para
`size` / `clear` - usa el redis-cli de administración para esos casos.

## Señal de reinicio del worker

`php artisan queue:restart` se traduce a:

```rust
Queue::restart().await?;
```

La señal vive en `Cache` como un timestamp en milisegundos. Los workers
sondean una vez por bucle y salen limpiamente cuando el timestamp es más
nuevo que su propia hora de arranque. Empareja esto con un supervisor
(systemd, Kubernetes, el módulo `supervisor`) para que un worker nuevo
continúe donde el anterior se detuvo.

## Pausar colas

`php artisan queue:pause` / `queue:resume` se traducen a:

```rust
Queue::pause(&connection, "billing").await?;
Queue::resume(&connection, "billing").await?;
Queue::pause_all().await?;
Queue::resume_all().await?;
```

o desde la CLI:

```bash
./app queue:pause billing
./app queue:pause --all
./app queue:resume billing
./app queue:resume --all      # alias: queue:continue
```

Un worker pausado termina lo que ya había extraído  - pausar nunca
interrumpe un job en vuelo -  y después deja de reclamar trabajo nuevo hasta
que se reanuda. `pause_all` / `resume_all` son el interruptor global;
pausar (o reanudar) una cola con nombre solo afecta a esa cola. **`resume_all`
no elimina una pausa por cola**: una cola pausada individualmente sigue
pausada después de una reanudación global, igual que en Laravel. Límpiala
explícitamente con `Queue::resume(&connection, "billing")`.

Ambas señales viven en `Cache`, junto a la señal de reinicio de arriba:

| Clave | Significado |
| --- | --- |
| `suprnova:queues:paused` | interruptor global, establecido por `pause_all` |
| `suprnova:queue:paused:{connection}:{queue}` | interruptor de una cola, establecido por `pause` |

Consulta el estado con `Queue::is_paused(&connection, "billing").await?`
(es `true` si cualquiera de las claves está establecida) o con
`Queue::paused_queues(&connection, &queues).await?` (cuáles de `queues`
están pausadas actualmente).

### Pausar por cola requiere un `--queue` con nombre

Un worker iniciado con `--queue=billing,exports` solo reclama de esas dos
colas, así que pausar `billing` reduce esa lista a `exports` mientras dure
la pausa. Un worker iniciado sin ningún `--queue` drena todas las colas que
contiene el driver, y no hay forma de pedir «pausa solo `billing`» contra eso:
`QueueDriver::pop_from` nunca informa qué nombres de cola existen, así que no
hay nada con lo que comprobar una clave de pausa por cola. `pause_all` todavía
detiene completamente un worker sin filtro; una pausa por cola con nombre solo
surte efecto una vez que también nombras las colas de ese worker.

### Desactivar el sondeo de pausa

Establece `QUEUE_PAUSABLE=false` y cada worker de ese proceso ignora por
completo las señales de pausa, sin coste adicional de lectura de caché por
bucle. `queue:pause` (no `queue:resume`) también se niega a ejecutarse y sale
con código distinto de cero, por lo que un operador que desactivó la pausa se
entera de inmediato en vez de emitir una pausa que silenciosamente no hace
nada. Refleja `Worker::$pausable` de Laravel.

### Por qué Suprnova diverge

Una caché inalcanzable falla **abierta**: un worker que no puede leer las
claves de pausa se comporta como «no pausado» y sigue drenando, el mismo
contrato de fallo abierto que ya usa la señal de reinicio del worker de arriba.
Una interrupción temporal de la caché debe degradar una flota de workers a
«ignorar la pausa», nunca a «todos los workers se congelan silenciosamente»:
el estado de pausa es una señal explícita de participación voluntaria, y su
propia indisponibilidad no debe convertirse en un interruptor de apagado
oculto.

## Apagado ordenado

El `CancellationToken` del worker se dispara en el siguiente límite de
extracción, nunca a mitad de un despacho. Un handler que ya fue extraído
se ejecuta hasta completarse (acotado por su propio `Job::timeout()` si
está establecido) antes de que el worker salga. Eso significa que los
efectos secundarios en vuelo no se cortan a mitad de camino, pero un
SIGTERM puede tardar hasta el timeout por job en drenar. Establece
`WorkerConfig::max_jobs` para una estrategia de reinicio periódico en
workers de larga duración; el worker sale limpiamente después de esa
cantidad de resoluciones, sin importar el resultado.

## Métricas de resolución

El worker emite un contador `queue.settlement.failures` vía
[`Metrics`](observability.md) en cada fallo de ack/nack. Atributos:
`operation` (`"ack"` | `"nack"`), `driver` (el nombre del driver
configurado), `job` (el job_name), `outcome` (`"success"`,
`"dead_letter"`, `"retry"`, `"deleted"`, `"timeout_dead_letter"`,
`"timeout_retry"`, `"released"`).

Una tasa distinta de cero aquí significa que la entrega al menos una vez
puede reentregar un efecto secundario que ya tuvo éxito, o perder la
contabilidad de intentos - alerta sobre esto de forma explícita.

## Errores tipados

`MaxAttemptsExceeded`, `TimeoutExceeded`, y `ManuallyFailed` reflejan
`MaxAttemptsExceededException` / `TimeoutExceededException` /
`ManuallyFailedException` de Laravel. El worker adjunta la causa
relevante al evento `JobFailed` de envío a fallidos, para que los
oyentes puedan usar pattern-matching en lugar de buscar substrings en el
mensaje de error.

## Nombrado de conexiones

Los workers etiquetan cada evento de ciclo de vida con un nombre de
conexión. Por defecto es el `name()` del driver (por ejemplo, `"memory"`,
`"redis"`, `"database"`). Las apps que ejecutan varias conexiones a la
vez pueden anularlo:

```rust
Queue::set_connection_name("orders-redis");
```

## Pruebas

La semántica de `Queue::fake()` vive en `queue::testing`:

```rust
let _guard = suprnova::queue::testing::install_fake();
my_code_that_dispatches_jobs().await;

suprnova::queue::testing::assert_pushed::<SendWelcomeEmail>(|j| j.user_id == 42);

// Para despachos retrasados, fija el timestamp programado:
suprnova::queue::testing::assert_pushed_later::<SendWelcomeEmail>(|j, at| {
    j.user_id == 42 && at > chrono::Utc::now()
});
```

La guarda del fake serializa los tests en paralelo mediante un mutex para
todo el proceso; captura `(payload, available_at, overrides)` por cada
push y se limpia al `Drop`. `overrides` es
`EnvelopeOverrides::default()` salvo en `push_with`/`later_with`.
Consulta [Simulación](mocking.md#queue---queuetestinginstall_fake) para
`assert_pushed_on_queue` / `assert_pushed_on_connection` y
`pushed_with_overrides`. En modo fake, `push_unique` siempre registra el
push como nuevo - la deduplicación es irrelevante cuando no hay ningún
driver conectado.

## La idempotencia es el contrato del worker contigo

Los drivers de cola respaldados por Redis no pueden hacer que `nack` sea
atómico - `XADD` y `XACK` son comandos separados. Una caída entre ambos
reentrega el mensaje vía `XAUTOCLAIM`. Los drivers en memoria y de base
de datos son exactamente-una-vez-por-intento, pero el bucle del worker no
distingue entre drivers, así que **todo handler de job en un despliegue
de producción debe ser idempotente**.

Para jobs típicos de estilo comando, envuelve el cuerpo del handler en
[`Idempotency::once`](idempotency.md) o en
[`Idempotency::commit_on_success`](idempotency.md), indexado por una
clave estable por operación (id de entidad, id de solicitud suministrado
por quien llama, etc.). Cuando un reintento debe devolver el resultado
*original* en lugar de saltarse la reejecución, usa
`Idempotency::remember`, que registra el valor de éxito y lo reproduce en
entregas posteriores.

## Siguiente

- [Bus](bus.md) - despachador síncrono con resultados tipados
- [Eventos](events.md) - dispersión pub/sub
- [Idempotencia](idempotency.md) - el contrato que los handlers respetan
  para la entrega al menos una vez
- [Caché](cache.md) - respalda a `push_unique`, `WithoutOverlapping`,
  `RateLimited`
- [Simulación y falsificaciones](mocking.md) - cada guarda de fake,
  incluyendo `Queue::fake`

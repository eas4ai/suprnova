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

Se incluyen cinco drivers en el árbol. Se configuran mediante la variable
de entorno `QUEUE_DRIVER` o llamando a `Queue::set_driver(...)` de forma
programática.

| Driver | Úsalo para | Puntos fuertes |
| --- | --- | --- |
| `MemoryQueueDriver` | pruebas, aplicaciones de un solo proceso | `tokio::time::DelayQueue` para `available_at`, compatible con el reloj virtual |
| `RedisQueueDriver` | dispersión en producción | grupos de consumidores + `XAUTOCLAIM` + jobs retardados respaldados por ZSET |
| `DatabaseQueueDriver` | aplicaciones con una sola base de datos | `FOR UPDATE SKIP LOCKED` en Postgres/MySQL, serializado con `BEGIN` en SQLite |
| `SyncQueueDriver` | desarrollo, CI | ejecuta el handler en línea en el `push`, sin worker |
| `NullQueueDriver` | envoltorios de pruebas | descarta todos los push sin ejecutarlos |

`Queue::bootstrap_from_env()` lee `QUEUE_DRIVER` y conecta el driver
correspondiente; `Queue::bootstrap_default()` conecta siempre el driver de
memoria. La ruta de arranque del servidor llama a una de las dos por ti;
la mayoría de las aplicaciones solo configuran mediante variables de
entorno.

`FailoverQueueDriver` no es un sexto backend. Envuelve una lista ordenada
de los drivers de arriba para que un push que una conexión rechaza recaiga
en la siguiente. Consulta [Conexiones con failover](#conexiones-con-failover).

### Configuración del entorno

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# Driver de base de datos - DB::init() tiene que ejecutarse antes
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

El driver de base de datos valida `QUEUE_DB_TABLE` como identificador SQL
en el momento de construirse, así que un valor de entorno malformado hace
fallar el arranque en lugar de llegar a la composición del SQL. Redis usa
sea-streamer-redis por debajo con `AutoCommit::Disabled`; el tiempo de
espera de visibilidad se fija al construir el grupo de consumidores, así
que el argumento `visibility_timeout` de cada `pop` se ignora en Redis
(una divergencia documentada del contrato del trait, impuesta por Redis
Streams).

### Por qué Suprnova diverge

Laravel enruta todo lo encolable a través del Bus, distinguiendo los jobs
`ShouldQueue` en el momento del despacho. Suprnova separa los dos: `Bus`
para el trabajo síncrono que devuelve un resultado tipado, `Queue` para el
trabajo asíncrono que sobrevive a la caída de un proceso. PHP necesita el
enrutamiento implícito porque su modelo de un proceso por solicitud hace
difícil modelar de otro modo "haz esto luego, en otro proceso". Tokio no:
la elección explícita entre `Bus::dispatch` y `Queue::push` es más clara,
más rápida y expone la decisión de durabilidad en el sitio de la llamada.
Consulta [`bus.md`](bus.md) para verlos uno al lado del otro.

## Conexiones con failover

`FailoverQueueDriver` envuelve una lista ordenada de conexiones. Un push
que la primera conexión rechaza se reintenta en la siguiente, y así
sucesivamente hacia abajo de la lista, de modo que una caída de Redis no
convierte cada despacho en un job perdido.

Configúralo desde el entorno:

```bash
QUEUE_DRIVER=failover
QUEUE_FAILOVER_CONNECTIONS=redis,database

# Cada conexión lee sus propias variables, exactamente igual que si
# fuera QUEUE_DRIVER por sí sola.
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_DB_TABLE=jobs
```

O conéctalo tú mismo, cuando las conexiones necesiten una configuración
en tiempo de ejecución que el entorno no puede expresar:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{
    DatabaseQueueDriver, FailoverQueueDriver, Queue, QueueDriver, RedisQueueDriver,
};
use suprnova::{DB, FrameworkError};

pub async fn register() -> Result<(), FrameworkError> {
    let redis = RedisQueueDriver::connect(
        "redis://127.0.0.1:6379",
        "suprnova-queue",
        "default",
        "consumer-1",
        Duration::from_secs(60),
    )
    .await?;
    let database =
        DatabaseQueueDriver::new(DB::connection()?.inner().clone(), "jobs".to_string())?;

    let failover = FailoverQueueDriver::new(vec![
        ("redis".to_string(), Arc::new(redis) as Arc<dyn QueueDriver>),
        ("database".to_string(), Arc::new(database) as Arc<dyn QueueDriver>),
    ])?;
    Queue::set_driver(Arc::new(failover));
    Ok(())
}
```

El `String` de cada entrada es la etiqueta de conexión que se reporta en
el evento `QueueFailedOver`. No se deriva del tipo de driver, porque dos
conexiones pueden usar el mismo driver.

`QUEUE_FAILOVER_CONNECTIONS` es obligatoria cuando
`QUEUE_DRIVER=failover`, y la lista no puede contener `failover` en sí. Una
entrada que nombre un driver que no existe es un error de arranque, y no
el respaldo de avisar y usar memoria que `QUEUE_DRIVER` se aplica a sí
mismo: dentro de una cadena de failover, una errata que se convirtiera en
silencio en una conexión en memoria pondría un backend efímero en una
lista durable.

### Las escrituras conmutan, las lecturas no

Solo `push` y `bulk_push` recorren la lista de conexiones. Todas las demás
operaciones - `pop`, `ack`, `nack`, `release`, `settle`, `clear`, los
cuatro contadores y los tres listados de inspección - van a la **primera**
conexión y a ninguna otra.

Esa asimetría es el contrato, no un olvido. Un token de reserva solo tiene
sentido para el driver que lo emitió, así que hacer ack contra otra
conexión no resolvería nada y corrompería ambas. Los contadores y los
listados siguen la misma regla para que lo que inspeccionas sea lo que
drena el worker de esta conexión, y no una suma entre backends que no
coincide con la vista de ningún worker.

**Un worker sobre la conexión de failover drena únicamente la primaria.**
Los jobs que conmutaron a un respaldo necesitan un worker ejecutándose
contra esa conexión de respaldo directamente:

```bash
# Drena la primaria de la cadena de failover.
QUEUE_DRIVER=failover QUEUE_FAILOVER_CONNECTIONS=redis,database ./app queue:work

# Drena lo que conmutó a la base de datos. Ejecuta esto también.
QUEUE_DRIVER=database ./app queue:work
```

La documentación de Laravel lleva la misma advertencia por la misma razón.

Esto alcanza a las cadenas, pero solo por una puerta. Un worker resuelve
un job y encola el siguiente eslabón de una [cadena
encolada](#cadenas-encoladas) en una sola llamada, `settle`, y el
decorador delega esa llamada únicamente en la primaria. Así que, con una
primaria transaccional como el driver de base de datos, una primaria caída
hace fallar la resolución y nada conmuta: el worker deja la reserva
intacta y la expiración de la visibilidad reentrega el job. La caída hacia
la siguiente conexión ocurre cuando la primaria responde
`Settled::Unsupported`, como hacen los drivers de memoria y de Redis,
porque entonces el worker empuja el siguiente eslabón a través del driver
vinculado como cualquier otro push - y ese push sí conmuta. El resto de
esa cadena espera entonces a un worker en la conexión de respaldo. Sin él,
la cadena se atasca: el eslabón es durable y no se pierde nada, pero
tampoco hay nada que lo ejecute.

### El evento `QueueFailedOver`

Cada conexión que rechaza un push despacha
`queue::events::QueueFailedOver { connection, job_name, exception }`, pero
solo en el push que lleva a esa conexión *al* estado de fallo. Una
conexión que ya se sabe que está fallando se mantiene callada hasta que un
push posterior tiene éxito en ella, lo que la rearma. Una caída de cuatro
horas produce un evento, no uno por despacho, que es lo que lo hace
utilizable como alerta.

`connection` es la etiqueta de la conexión que falló, no la de la que
aceptó el job.

Cuando todas las conexiones rechazan un push, el push devuelve el error de
la última conexión. `bulk_push` empuja cada sobre por separado, así que
cada uno recorre la lista por su cuenta: un lote que la primaria aceptó a
medias nunca se vuelve a empujar entero al respaldo, y cada sobre conserva
el `available_at` con el que se construyó. Un lote no es atómico. Si todas
las conexiones rechazan un sobre, `bulk_push` devuelve el error de ese
sobre con los sobres anteriores ya encolados.

Conmutar no es deduplicar. El decorador nunca reintenta un sobre que una
conexión aceptó, pero una conexión que escribe el sobre y *luego* informa
de un fallo produce un duplicado en la siguiente conexión, porque "lo
escribió y perdió el acuse de recibo" es indistinguible de "nunca lo
tomó". Ambas copias llevan el mismo id de job. Ese es el contrato de
entrega al menos una vez del framework, el mismo que hace de la
idempotencia del handler un requisito en todas partes - consulta [La
idempotencia es el contrato del worker
contigo](#la-idempotencia-es-el-contrato-del-worker-contigo).

### Por qué Suprnova diverge

La conexión de failover de Laravel es un array `connections` en
`config/queue.php`, resuelto a través del registro de conexiones. Suprnova
no tiene un registro de drivers por conexión - hay un solo driver
vinculado para todo el proceso -, así que las etiquetas vienen de
`QUEUE_FAILOVER_CONNECTIONS` (o del `String` que pases a
`FailoverQueueDriver::new`) y las lecturas delegan en el primer *driver*
en lugar de en una conexión con nombre.

El `FailoverQueue::bulk` de Laravel recorre los jobs uno a uno para que el
retardo de cada uno sobreviva. Suprnova resuelve el retardo sobre el sobre
antes de que ningún driver lo vea, así que el bucle por sobre lo conserva
gratis - pero el bucle sigue siendo lo que evita que un lote aterrizado a
medias se empuje dos veces, así que se queda.

## Variantes de push

Cada variante de push toma un valor tipado `J: Job` y retorna cuando el
sobre queda confirmado en el driver, no cuando se ejecuta el handler.

| Método | Comportamiento |
| --- | --- |
| `Queue::push(job)` | encola de inmediato |
| `Queue::push_later(job, at)` | disponible en un `DateTime<Utc>` concreto |
| `Queue::later(delay, job)` | disponible tras `delay` a partir de ahora |
| `Queue::push_with(job, overrides)` | encola de inmediato con `EnvelopeOverrides` por push |
| `Queue::push_after_commit(job)` | encola cuando confirma el `DB::transaction` circundante |
| `Queue::later_with(delay, job, overrides)` | disponible tras `delay` a partir de ahora, con `EnvelopeOverrides` por push |
| `Queue::push_unique(job)` | deduplica por `J::unique_id` dentro de `J::unique_for`; devuelve `Ok(true)` cuando el sobre se empujó y `Ok(false)` cuando una clave de deduplicación viva lo suprimió |
| `Queue::push_unique_later(job, at)` | único + programado |
| `Queue::later_unique(delay, job)` | único + retardado |
| `Queue::bulk(vec![job1, job2, ...])` | empuja todos los jobs (el driver puede usar una ruta bulk nativa) |

`push_unique` exige que la capa de caché esté arrancada: el bloqueo de
deduplicación vive en [`Cache`](cache.md) mediante
[`Idempotency::commit_on_success`](idempotency.md). Un push fallido libera
la clave de deduplicación para que quien llama pueda reintentar; un push
correcto la retiene durante `J::unique_for` segundos. El job tiene que
sobrescribir `Job::unique_id(&self)` para devolver `Some(id)`; con `None`
se devuelve un error interno.

El booleano responde a una sola pregunta - "¿está este job en la cola?" -
y detrás de ella hay un tercer caso. Si el lease del bloqueo de
deduplicación se pierde mientras el push está en vuelo, el push se
completa igualmente (la capa de idempotencia nunca cancela un cuerpo que
quizá ya haya tenido efecto) y sigues obteniendo `Ok(true)`, con un log de
nivel `warn` que nombra el job y su clave única. El job está encolado; lo
que queda sin demostrar es que nadie más encoló el mismo en paralelo. Tu
handler ya tiene que tolerar la reentrega, así que esto no necesita ningún
tratamiento extra, pero el log está ahí porque una ráfaga de estos avisos
significa que la caché que respalda tu bloqueo de deduplicación va justa.

### Único hasta el procesamiento

Un bloqueo de unicidad dura normalmente toda la ventana `unique_for`,
incluso después de que el job se haya ejecutado. Cuando el bloqueo existe
para fusionar duplicados *encolados* y no para serializar la ejecución,
súmate a liberarlo en el momento en que empieza el procesamiento:

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct RebuildSearchIndex {
    index: String,
}

#[async_trait]
impl Job for RebuildSearchIndex {
    fn job_name() -> &'static str { "rebuild-search-index" }
    fn unique_id(&self) -> Option<String> { Some(self.index.clone()) }
    fn unique_until_processing() -> bool { true }
    fn unique_for() -> Duration { Duration::from_secs(3600) }

    async fn handle(self) -> Result<(), FrameworkError> {
        // Una reconstrucción que dura 20 minutos ya no se traga el
        // redespacho que llega en el minuto 2.
        Ok(())
    }
}
```

El worker libera el bloqueo tras la pasada de middleware del job y justo
antes de que se ejecute el handler. De ahí se siguen cuatro consecuencias:

- Un job que un middleware libera de vuelta a la cola conserva su bloqueo.
  No ha empezado a procesarse, así que para un duplicado no ha cambiado
  nada.
- Un job que un middleware cortocircuita de cualquier otra forma cede su
  bloqueo, porque no va a procesarse en absoluto. Eso cubre eliminar el
  job, enviarlo a fallidos y darlo por completado sin llegar a llamar al
  handler.
- Un job que falla libera su bloqueo y aun así se reintenta. El bloqueo se
  fue en el momento en que empezó el procesamiento, así que un duplicado
  puede encolarse mientras el intento fallido espera su backoff, y acabas
  con dos sobres para el mismo id único. Ese es el intercambio que hace
  esta opción. Si un reintento tiene que seguir ocupando la plaza, deja
  `unique_until_processing` desactivado y que el TTL de `unique_for` cubra
  toda la cadena de intentos.
- La liberación está acotada al propietario. `push_unique` registra el
  token de propietario del bloqueo en el sobre, y el worker libera con ese
  token, así que un intento reentregado nunca puede liberar un bloqueo que
  un despacho más nuevo haya adquirido entretanto.

`unique_until_processing` necesita las dos mismas cosas que necesita
`push_unique`: un `unique_id` que devuelva `Some(id)` y una capa de caché
arrancada.

Bajo el driver `sync` el handler se ejecuta en línea dentro de la misma
llamada a `push_unique` que tomó el bloqueo, así que el job libera un
bloqueo que quien lo llamó sigue reteniendo nominalmente. Si ese handler
tarda más de un tercio de `unique_for`, el renovador del lease de
deduplicación nota que el bloqueo ya no está y registra un aviso de lease
perdido, y `push_unique` registra encima su propio aviso de "no se pudo
demostrar la exclusividad". Aquí ambos son esperables y no un fallo: el
job se ejecutó, el push devuelve `Ok(true)` y el bloqueo ya no está porque
el propio job lo liberó.

### Por qué Suprnova diverge

Laravel libera el bloqueo de un job único *ordinario* en cuanto retorna el
handler. Suprnova deja en cambio que ese bloqueo caduque con el TTL de
`unique_for`, lo que mantiene honesta la ventana de deduplicación cuando
un worker muere a mitad del job: la ventana que configuraste es la ventana
que obtienes, haya retornado el handler o no.
`unique_until_processing` se comporta igual en los dos frameworks.

Suprnova tampoco fuerza nunca la liberación de un bloqueo de unicidad.
Laravel recurre a una liberación forzada para un primer intento que no
lleva token de propietario. Los únicos sobres que llegan a un worker de
Suprnova sin token son los sobres encolados antes de que el token
existiera, y esos se quedan con la caducidad por TTL en lugar de arriesgar
una liberación que borre el bloqueo de un despacho más nuevo.

### Anulaciones por push con `EnvelopeOverrides`

`Queue::push_with` y `Queue::later_with` toman un `EnvelopeOverrides`
junto al job, para ese despacho concreto que necesita un comportamiento de
cola, conexión, tiempo de espera o reintentos distinto de los valores por
defecto del propio job:

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

// La contraparte retardada, que refleja la relación de `Queue::later` con `Queue::push`.
Queue::later_with(Duration::from_secs(60), SendWelcomeEmail { user_id: 42 }, overrides).await?;
```

Todos los campos valen `None` por defecto y ceden a la resolución normal
que `Queue::push` ya ejecuta; un campo `Some` gana sobre todo eso para
este único push, mandando tanto sobre una ruta registrada con
[`Queue::route`](#enrutamiento-de-colas) como sobre la propia declaración
`Job::*` del job para ese campo:

| Campo | Manda sobre |
| --- | --- |
| `queue` | `Queue::route`, `Job::queue()` |
| `connection` | `Queue::route`, `Job::connection()` |
| `timeout` | `Job::timeout()` |
| `fail_on_timeout` | `Job::fail_on_timeout()` |
| `max_tries` | `Job::max_tries()` |
| `backoff` | `Job::backoff()` |
| `after_commit` | `Job::after_commit()` |

`EnvelopeOverrides` es la primitiva sobre la que están construidos tanto
`Mail::on_queue`/`.on_connection()` como el ajuste de cola por
notificación de `Notify::queue` - consulta [Correo](mail.md#queueing) y
[Notificaciones](notifications.md).

### Retardo declarado por el job

Un job puede llevar su propio retardo por defecto en lugar de que cada
sitio de llamada repita `Queue::later(Duration::from_secs(60), job)`:

```rust
impl Job for SendDigest {
    // ...
    fn delay() -> Option<Duration> { Some(Duration::from_secs(60)) }
}
```

`Queue::push(job)`, `Queue::push_with(job, overrides)`,
`Queue::push_unique(job)` y `Queue::bulk(vec![job1, job2])` lo respetan
todos: `available_at` pasa a ser `now + J::delay()` en lugar de `now`.
`Queue::bulk` resuelve el retardo una sola vez por llamada, ya que todos
los jobs del vector comparten el mismo `J` concreto y, por tanto, el mismo
`Job::delay()`.

Un retardo explícito en el sitio de la llamada siempre gana:
`Queue::push_later(job, at)`, `Queue::later(delay, job)`,
`Queue::later_with(delay, job, overrides)`,
`Queue::push_unique_later(job, at)` y `Queue::later_unique(delay, job)`
usan todos, literalmente, la marca de tiempo o el retardo que pasó quien
llamó; `Job::delay()` no se consulta en ninguno de ellos. Echa mano del
método del trait cuando todo despacho de un tipo de job deba arrancar
retardado por defecto; echa mano de una de las variantes
`later`/`push_later` para un retardo que necesita un despacho concreto
pero que el tipo no declara por su cuenta.

Los lotes y las cadenas tampoco lo consultan: `Queue::batch()...add(job)` y
`Queue::chain()...add(job)?` construyen sus sobres con `available_at`
fijado al momento en que llamaste a `add`, así que un job con un
`Job::delay()` declarado se despacha de inmediato como parte de un lote o
de una cadena aunque un `Queue::push(job)` a secas del mismo job
esperaría. Dale al job un retardo explícito por otra vía - un campo en el
propio job, aplicado en `handle()` - si un paso por lote o en cadena
necesita uno.

### Por qué Suprnova diverge

El `$job->delay` de Laravel es una propiedad de instancia, fijada por
despacho (`SendDigest::dispatch($user)->delay(60)`), así que dos despachos
de la misma clase pueden llevar retardos distintos. Aquí `Job::delay()` es
en cambio un valor por defecto a nivel de clase, como `Job::queue()` o
`Job::max_tries()`: un despacho que necesite un retardo calculado a partir
de sus propios datos usa `Queue::later`/`push_later`, que ya manda sobre
el valor por defecto declarado.

### Despacho posterior al commit

Un job empujado dentro de un [`DB::transaction`](database.md#transactions)
compite con esa transacción. Un worker en otro proceso puede sacar el
sobre, buscar la fila que la transacción todavía mantiene abierta y
fallar - o, peor aún, la transacción revierte y el job se ejecuta contra
datos que ya no existen.

Haz que el job se sume a esperar a la confirmación:

```rust
use suprnova::{DB, FrameworkError, Job, Queue, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct SendReceipt {
    order_id: i64,
}

#[async_trait]
impl Job for SendReceipt {
    fn job_name() -> &'static str { "send-receipt" }
    fn after_commit() -> bool { true }

    async fn handle(self) -> Result<(), FrameworkError> {
        // Está garantizado que la fila del pedido es durable cuando esto se ejecuta.
        Ok(())
    }
}

DB::transaction(|_tx| {
    Box::pin(async move {
        let order = Order::create(suprnova::attrs! { total: 4999i64 }).await?;
        // Aquí no llega nada al driver.
        Queue::push(SendReceipt { order_id: order.id }).await?;
        Ok::<(), FrameworkError>(())
    })
})
.await?;
// El sobre está en la cola ahora, y solo ahora.
```

Tres reglas cubren todos los casos:

- **Dentro de una transacción, el push entero espera a la confirmación.**
  No solo la escritura en el driver: la construcción del sobre, el evento
  `JobQueueing` y el evento `JobQueued` ocurren también en el momento de
  la confirmación, así que a un oyente nunca se le habla de un job que un
  rollback descarta después.
- **Un rollback lo descarta.** El push sencillamente no ocurre nunca. Si
  tomó un bloqueo de unicidad, el rollback devuelve ese bloqueo.
- **Fuera de una transacción el push ocurre de inmediato.** Eso es lo que
  hace seguro declarar esta opción en el tipo del job: un sitio de
  despacho no tiene que saber si la ruta de código en la que está es
  transaccional.

El rollback a un [savepoint](database.md#savepoints) cuenta como un
rollback para todo lo registrado dentro de él. `tx.rollback_to("name")`
descarta los push diferidos desde `tx.savepoint("name")` y libera los
bloqueos que tomaron, justo en ese momento, así que un redespacho dentro
de la misma transacción vuelve a ganar la clave. Los push hechos antes del
savepoint quedan intactos, y un savepoint que nunca reviertes conserva
todo lo registrado dentro de él.

Por despacho, en lugar de por tipo de job, usa
`EnvelopeOverrides::after_commit`. `Some(true)` es el `afterCommit()` de
Laravel y tiene el atajo `Queue::push_after_commit(job)`; `Some(false)` es
el `beforeCommit()` de Laravel, para ese despacho concreto que tiene que
ser visible para un worker antes de que aterrice la confirmación:

```rust
use suprnova::queue::{EnvelopeOverrides, Queue};

// Difiere un job cuyo tipo no se suma.
Queue::push_after_commit(SendWelcomeEmail { user_id: 42 }).await?;

// Empuja de inmediato aunque el tipo del job sí se sume.
Queue::push_with(
    SendReceipt { order_id: 7 },
    EnvelopeOverrides { after_commit: Some(false), ..Default::default() },
)
.await?;
```

Un `Queue::push` diferido vuelve a resolver
[`Job::delay()`](#retardo-declarado-por-el-job) contra la confirmación y no
contra el push, porque el retardo significa "espera este tiempo tras el
despacho" y, para un job diferido, el despacho *es* la confirmación. Una
marca de tiempo explícita es la intención de quien llama sobre un momento
concreto, así que `Queue::push_later`, `Queue::later` y
`Queue::later_with` llevan la suya a través del aplazamiento sin cambios.

`Queue::push_unique` se difiere con una asimetría deliberada: el bloqueo
de deduplicación se toma de inmediato, así que un segundo `push_unique`
para el mismo id único dentro de la misma transacción sigue suprimiéndose
y sigue informando `Ok(false)`. Solo espera el sobre. El ganador informa
`Ok(true)` aunque su push esté pendiente, porque el push va a ocurrir. Un
rollback libera el bloqueo que tomó, acotado al propietario, así que la
ventana `unique_for` nunca queda bloqueada por un despacho que nunca
ocurrió - y lo mismo hace cualquier otro final en el que la confirmación
no aterrice, incluido un `COMMIT` rechazado. El único límite de esa
garantía es el propio TTL: una transacción que permanece abierta más
tiempo que `unique_for` puede ver caducar su bloqueo y que otro despacho
lo tome en pleno vuelo, así que dale a `unique_for` margen por encima de
tu transacción más larga si la deduplicación importa. La familia
`push_unique*` no toma `EnvelopeOverrides`, así que `Job::after_commit()`
es lo único que decide si un push único se difiere: no hay una anulación
por push para ello.

Los lotes y las cadenas no se difieren, igual que no consultan
`Job::delay()`: `Queue::batch()` y `Queue::chain()` construyen y empujan
sus sobres directamente. Envuelve la llamada a `.dispatch()` para que se
ejecute después de que retorne la transacción si un lote tiene que esperar
a una confirmación.

El [correo](mail.md#queueing) y las
[notificaciones](notifications.md) encolados tampoco se difieren. Cada uno
viaja sobre un único tipo de job compartido (`SendMailJob` /
`SendNotificationJob`), y todavía no hay un equivalente de
`ShouldQueueAfterCommit` en `Mailable` ni en `Notification`, así que una
llamada a `Mail::queue` o `Notify::queue` dentro de una transacción llega
al driver de inmediato. Envía esos después de que retorne la transacción.

Bajo `Queue::fake()` un push se registra de inmediato, aplazamiento
incluido, así que una prueba puede afirmar sobre él sin confirmar nada.
Esto coincide con el `Bus::fake` de Laravel, y es lo que permite que una
prueba ejecute un handler transaccional y afirme sus despachos de una sola
vez.

### Por qué Suprnova diverge

`Queue::bulk` es monomórfico - todos los elementos comparten un mismo `J`
concreto -, así que su partición posterior al commit es todo o nada para
la llamada. Laravel parte un array heterogéneo en mitades diferida e
inmediata; aquí no hay nada que partir.

El aplazamiento está ligado a la forma con closure. Un push dentro de un
[`DB::begin_transaction`](database.md#manual-form) manual ocurre **de
inmediato**, porque el modo manual no instala ninguna transacción
ambiental y, por tanto, no tiene ninguna confirmación de la que colgar un
callback. Diferir ahí encolaría un callback que nada ejecutaría nunca, y
un despacho que desaparece en silencio es peor que uno que ocurre
demasiado pronto. Echa mano de `DB::transaction` cuando un despacho tenga
que esperar a la confirmación.

Laravel lee además una clave de configuración `after_commit` a nivel de
conexión como último respaldo de su cadena de precedencia. Suprnova se
detiene en la anulación por push y luego en el propio `Job::after_commit()`
del job: aquí las conexiones de cola no llevan su propia política de
despacho.

## Configuración de job

Sobrescribe las funciones asociadas de `Job` para ajustar el
comportamiento por impl:

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

    fn delay() -> Option<Duration> { None }                // por defecto: sin retardo
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
    fn unique_until_processing() -> bool { true }          // por defecto: false (el TTL es la ventana)
    fn middleware() -> Vec<std::sync::Arc<dyn JobMiddleware>> {
        vec![/* consulta "Middleware de job" más abajo */]
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
   (consulta [Anulaciones por push con `EnvelopeOverrides`](#anulaciones-por-push-con-envelopeoverrides))
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
Queue::pending_size().await?;    // available_at <= now, sin reservar
Queue::delayed_size().await?;    // available_at > now
Queue::reserved_size().await?;   // sacados ahora mismo, todavía sin ack
Queue::clear().await?;           // descarta todos los sobres, devuelve el recuento
Queue::driver_name()?;           // nombre del driver configurado, para logs / administración
```

El trait `QueueDriver` declara valores por defecto para `size` /
`pending_size` / `reserved_size` / `delayed_size` / `clear`;
`MemoryQueueDriver`, `DatabaseQueueDriver` y `RedisQueueDriver` los
implementan todos de forma nativa.

### Inspeccionar colas

Los recuentos te dicen cuánto hay encolado; a veces necesitas ver los
sobres reales: un panel de administración, una sesión de depuración, la
pregunta de "qué es exactamente lo que está atascado".
`Queue::pending_jobs` / `delayed_jobs` / `reserved_jobs` devuelven la
misma información que cuentan los contadores de tamaño, como un listado de
DTOs `InspectedJob`:

```rust
use suprnova::queue::{InspectedJob, Queue};

let pending: Vec<InspectedJob> = Queue::pending_jobs(None).await?;
let billing_only: Vec<InspectedJob> = Queue::pending_jobs(Some("billing")).await?;
let delayed = Queue::delayed_jobs(None).await?;
let reserved = Queue::reserved_jobs(None).await?;

for job in &pending {
    println!(
        "{} attempts={} queue={:?} payload={}",
        job.name, job.attempts, job.queue, job.payload
    );
}
```

`InspectedJob` lleva `id`, `queue`, `name`, `attempts`, `payload` y
`created_at`. `id` y `created_at` son `Option`: los listados del driver de
base de datos siguen reportando una fila cuyo `envelope_json` no se pudo
decodificar - como `id: None` y `payload: {"unparseable": true}` - en
lugar de descartarla y ocultar un trabajo envenenado a quien esté mirando;
la proyección de `Queue::fake()` nunca registra una marca de tiempo de
despacho distinta de `available_at`, así que ahí `created_at` es siempre
`None`.

En el driver de memoria, `delayed_size()` lee directamente la longitud del
almacén de retardados, mientras que `delayed_jobs()` y `pending_jobs()`
promueven primero cualquier entrada cuyo `available_at` ya haya pasado. En
la estrecha ventana entre que un job vence y el siguiente tick de 50 ms
del recolector en segundo plano, `delayed_size()` todavía puede contar un
job que `delayed_jobs()` ya ha promovido a `pending_jobs()`: los listados
son la vista más actual, y una discrepancia ahí es esperable, no un bug.

Una reserva cuyo tiempo de espera de visibilidad ha vencido sigue
apareciendo en `reserved_jobs()` hasta que un `pop` o el recolector en
segundo plano la reclama. Solo esos dos reclaman, y reclamar es lo que
gasta un intento, así que una llamada de listado nunca cambia el contador
de intentos de un job por muchas veces que la llames.

#### Por qué Suprnova diverge

- **Un método con `Option<&str>`, no un par por listado.** Laravel
  distribuye `pendingJobs($queue)` junto a un `allPendingJobs()` aparte;
  aquí `queue: None` colapsa los dos en una sola llamada. La misma forma
  para `delayedJobs`/`allDelayedJobs` y `reservedJobs`/`allReservedJobs`.
- **El valor por defecto del trait es un `Err` honesto, no una colección
  vacía.** Los drivers de Beanstalkd y SQS de Laravel devuelven `[]` desde
  estos métodos incluso para una cola que a todas luces tiene jobs: una
  mentira por omisión que quien escriba un driver de terceros podría
  copiar sin darse cuenta. Un driver de Suprnova que no ha implementado la
  inspección lo dice; `sync` y `null` la sobrescriben con `Ok(vec![])`
  porque para ellos "nunca hay nada que listar" es la verdad literal, no
  un método sin implementar.
- **El `reserved_jobs` de Redis es por consumidor.** El driver solo conoce
  las reservas que ha entregado personalmente dentro del proceso; las
  entradas en vuelo de otro consumidor solo son visibles a través del
  propio `XPENDING` de Redis, no a través de esta llamada.
- **El `pending_jobs` de Redis significa "nunca entregado a ningún
  consumidor de este grupo".** Escanea `XRANGE (<last-delivered-id> +` -
  todo lo que hay más allá del cursor de entrega del grupo (`XINFO
  GROUPS`) - en lugar del stream entero, porque `ack` solo hace `XACK`
  sobre una entrada (este driver nunca hace `XDEL` ni `XTRIM` sobre el
  stream), así que un escaneo que se limitara a excluir las reservas en
  memoria de un consumidor reportaría todos los jobs con ack como
  pendientes para siempre. Un job liberado o con nack se vuelve a publicar
  bajo un id nuevo por encima del cursor, así que reaparece en cuanto su
  reintento está vivo. El mismo registro de "cota superior" que
  `pending_size`: el cursor se lee una sola vez, así que un `pop`
  concurrente puede reclamar una entrada entre esa lectura y el escaneo.
  En la práctica, la tarea de lectura anticipada en segundo plano de un
  consumidor en marcha tiende a reclamar una entrada recién empujada a los
  pocos milisegundos del push, mucho antes de que una aplicación llame a
  `pop`, así que `pending_jobs` refleja sobre todo el trabajo empujado
  mientras ningún consumidor de ese stream está sondeando activamente, y
  no "cualquier sobre que nadie ha sacado explícitamente todavía".

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

# Supervisores

Un supervisor es una tarea de Tokio de larga duración que el framework
arranca al iniciar y reinicia automáticamente cuando termina. Los
supervisores son para trabajo "siempre activo": heartbeats en segundo
plano, recolectores de métricas, calentadores de conexiones,
barredores periódicos, o cualquier bucle asíncrono que nunca debería
dejar de ejecutarse. Se distinguen de los [workers de
cola](queues.md), que consumen items discretos de tipo `Job` desde una
cola. Un supervisor no tiene cola de jobs - posee su propio bucle y
decide cuándo dormir, esperar o actuar.

El `SupervisorRegistry` arranca cada supervisor registrado como una
tarea de Tokio independiente, vigila el `JoinHandle` de cada tarea, y
la reinicia según su `RestartPolicy` cuando termina - ya sea
devolviendo `Err`, devolviendo `Ok`, o entrando en pánico. Los
reinicios se separan mediante un backoff exponencial que empieza en
100 ms y se acota en 60 segundos, de modo que un supervisor que falla
no entra en un bucle desbocado que inunda los registros.

## Inicio rápido

Define un supervisor, regístralo mediante `inventory::submit!`, y
llama a `SupervisorRegistry::start_all()` en el arranque.

**`src/supervisors/heartbeat.rs`:**

```rust
use async_trait::async_trait;
use std::time::Duration;
use suprnova::supervisor::{RestartPolicy, Supervisor};
use suprnova::{FrameworkError, SupervisorEntry};
use tokio_util::sync::CancellationToken;

pub struct LogHeartbeat;

#[async_trait]
impl Supervisor for LogHeartbeat {
    fn name(&self) -> &'static str { "heartbeat" }

    async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    tracing::info!("supervisor heartbeat tick");
                }
            }
        }
    }

    fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }
}

// Usa el `suprnova::inventory` reexportado para que una app generada por el
// andamiaje no necesite añadir `inventory` como dependencia directa.
suprnova::inventory::submit!(SupervisorEntry {
    factory: || Box::new(LogHeartbeat),
});
```

**`src/bootstrap.rs`:**

```rust
use suprnova::supervisor::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

Esa es toda la configuración. El supervisor `LogHeartbeat` arranca al
iniciar, registra un log cada 60 segundos, y - porque
`RestartPolicy::Always` reinicia tanto en salidas `Ok` como `Err` - se
reinicia de inmediato si el bucle termina por cualquier motivo.

## Políticas de reinicio

Cada supervisor declara su `RestartPolicy` mediante el método del
trait. El valor por defecto es `OnError`.

| Política | Se reinicia cuando... | Caso de uso |
|--------|-----------------|----------|
| `RestartPolicy::OnError` | `run()` devuelve `Err` o entra en pánico | Tareas que deberían ejecutarse hasta completarse con éxito (por ejemplo, un job de inicialización de una sola vez envuelto como supervisor). |
| `RestartPolicy::Always` | `run()` devuelve `Ok` o `Err`, o entra en pánico | Demonios verdaderos - bucles que nunca deberían retornar. Si el bucle termina por cualquier motivo, eso es un bug y un reinicio está justificado. |
| `RestartPolicy::Never` | (nunca) | Tareas de una sola vez que deberían ejecutarse una vez y no reiniciarse sin importar el resultado. |

```rust
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::OnError }   // por defecto
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }    // bucle de demonio
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Never }     // de una sola vez
```

**Cuándo elegir `Always` frente a `OnError`.** Un supervisor de bucle
infinito (`loop { ... }`) debería usar `Always` - si el bucle alguna
vez devuelve `Ok(())`, ocurrió algo inesperado y un reinicio es la
respuesta correcta. Un supervisor que hace trabajo finito y devuelve
`Ok` al tener éxito (por ejemplo, refrescar una caché una vez) debería
usar `OnError`, de modo que una finalización limpia no dispare un
reinicio.

**`Never` para trabajo de una sola vez.** Prefiere los [workers de
cola](queues.md) o las [tareas programadas](scheduling.md) para
trabajo que se ejecuta según un calendario. Usa `RestartPolicy::Never`
cuando el patrón de supervisor resulte conveniente para algo que debe
ejecutarse una vez al iniciar y nunca más.

## Manejo de pánicos

Los pánicos dentro de `run()` son atrapados por el registro y tratados
como errores - un supervisor que entra en pánico se reinicia con
backoff en lugar de derribar el proceso. El registro vigila el
`JoinHandle` de cada supervisor y detecta los pánicos mediante el
mecanismo estándar de join de Tokio.

Desde la perspectiva de la política de reinicio, un pánico siempre se
trata como una salida `Err`, sin importar la política:

- `OnError` - se reinicia tras un pánico (el pánico cuenta como
  error).
- `Always` - se reinicia tras un pánico (igual que cualquier otra
  salida).
- `Never` - no se reinicia tras un pánico (igual que cualquier otra
  salida).

El pánico se registra en el nivel `error!` con el nombre del
supervisor antes de que empiece el backoff de reinicio.

## Backoff

Cuando un supervisor termina y su política indica reiniciar, el
registro espera antes de lanzar el reemplazo:

| Reinicio consecutivo | Retardo |
|---------|-------|
| 1.º | 100ms |
| 2.º | 200ms |
| 3.º | 400ms |
| 4.º | 800ms |
| ... | se duplica cada vez |
| Tope | 60s |

El backoff se reinicia después de una ejecución saludable. El retardo
se duplica en cada reinicio *consecutivo* hasta el tope de 60 s, pero
una ejecución que permanece activa al menos 60 s (la duración del
tope) se considera saludable: el siguiente reinicio vuelve al piso de
100 ms en lugar de heredar el backoff que había escalado durante una
ráfaga de fallos anterior. Así, un demonio que se ejecutó sin
problemas durante horas y luego tiene un tropiezo se reinicia con
prontitud, no después de una espera de 60 s acumulada hace mucho
tiempo.

El reinicio del contador se basa en la actividad (liveness), y es
deliberadamente conservador: solo una ejecución que *sobrevive más
tiempo que el backoff máximo posible* cuenta como saludable. Una
ejecución que termina antes de ese umbral traslada el backoff actual
hacia adelante, de modo que un supervisor que verdaderamente oscila
entre caerse y arrancar - uno cuyas ejecuciones nunca alcanzan el
umbral - sigue escalando hasta el tope de 60 s y se queda ahí. El
reinicio del contador nunca enmascara a un supervisor que está en un
bucle de caídas.

El tope de 60 segundos evita que un supervisor permanentemente
averiado duerma indefinidamente o golpee dependencias externas en cada
reintento. Combínalo con el registro en nivel `error!` para alertar
cuando un supervisor entra en la banda de backoff alto.

## Apagado ordenado

Los supervisores reciben un `CancellationToken` como parámetro de
`run()`. El framework cancela este token ante Ctrl-C / SIGTERM como
parte de la secuencia de apagado de `Server::run`. Los supervisores
que quieran vaciar su estado, terminar el trabajo en vuelo, o salir de
forma limpia por cualquier otro medio deberían hacer `tokio::select!`
sobre `cancel.cancelled()`:

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                tracing::info!("supervisor heartbeat tick");
            }
        }
    }
}
```

El framework drena el `JoinSet` de supervisores con una ventana de
gracia de 5 segundos tras la cancelación. Los supervisores que no
respeten el token dentro de esa ventana se abortan mediante
`JoinSet::abort_all`. Este drenaje se ejecuta después del drenaje de
los handlers de WebSocket (para que las conexiones WS se limpien
primero) y antes de que se vacíen los búferes de telemetría.

Los supervisores que ignoren el token por completo seguirán
ejecutándose hasta que expire la ventana de 5 segundos, y entonces se
abortarán forzosamente. Si tu supervisor retiene recursos que
necesitan vaciarse (descriptores de archivo abiertos, solicitudes HTTP
en vuelo, registros parcialmente escritos), siempre haz `select` sobre
`cancel.cancelled()` y limpia antes de retornar.

### Integradores y tests de integración

`Server::run` llama a `SupervisorRegistry::shutdown(...)` por ti. El
código que llama a `SupervisorRegistry::start_all()` fuera de
`Server::run` (integradores que conducen el framework desde un binario
propio, o tests de integración que levantan supervisores directamente)
también debe llamar a `SupervisorRegistry::shutdown(timeout)` en el
desmontaje, o las tareas de supervisor sobrevivirán más allá de la
vida del test:

```rust
use std::time::Duration;
use suprnova::SupervisorRegistry;

// Configuración del test
SupervisorRegistry::start_all().await;

// ... ejercita el supervisor ...

// Desmontaje del test - cancela el token compartido, drena el JoinSet
// hasta `timeout`, y luego hace `abort_all` con los rezagados.
SupervisorRegistry::shutdown(Duration::from_secs(1)).await;
```

`shutdown` es un no-op si `start_all` nunca se llamó, así que es
seguro llamarlo desde el desmontaje sin condiciones.

## Observabilidad

Cada reinicio por la ruta de error emite una entrada de registro en
nivel `error!` con campos estructurados:

- `supervisor` - proviene de `Supervisor::name()`.
- `error` - el mensaje de error del valor de retorno `Err` de `run()`,
  o `"panic: <payload>"` para un pánico atrapado, o
  `"join error: <detail>"` para un fallo de join inusual.
- `backoff_ms` - el retardo de backoff en milisegundos antes del
  siguiente lanzamiento.

Los pánicos se reportan a través del mismo registro de error - no hay
un mensaje distinto de "entró en pánico":

```
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error=connection refused backoff_ms=400
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error="panic: \"deliberate test panic\"" backoff_ms=800
```

`RestartPolicy::Always` que devuelve `Ok(())` emite un `warn!` (no un
`error!`) con los mismos campos `supervisor` / `backoff_ms` y el
mensaje "supervisor returned Ok under Always policy; restarting" -
útil para detectar bucles de demonio que terminaron limpiamente cuando
no debían.

Los supervisores no obtienen un span de tracing automático alrededor
de `run()` - el `SupervisorRegistry` abre un span sobre el ciclo de
vida (arranque, reinicio) pero no sobre el interior de la tarea. Emite
tu propio `info_span!` o instrumenta (`instrument`) el cuerpo de tu
bucle si quieres contexto de span sobre el trabajo hecho dentro del
supervisor:

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = async {
                let span = tracing::info_span!("heartbeat.tick");
                let _guard = span.enter();
                do_work().await.ok();
                tokio::time::sleep(Duration::from_secs(60)).await;
            } => {}
        }
    }
}
```

### Por qué Suprnova diverge

Laravel no tiene un equivalente directo. El modelo de un proceso por
solicitud de PHP hace imposibles los demonios siempre activos dentro
del proceso - el trabajo de larga duración tiene que vivir fuera del
ciclo de vida de la solicitud, típicamente como un proceso worker
gestionado por `supervisord` que consume una cola, o como un comando
programado por cron. El worker de cola de Laravel
(`php artisan queue:work`) es el análogo más cercano, pero sigue
siendo un proceso de CLI de una sola vez que un supervisor
externo reinicia.

Suprnova se ejecuta sobre Tokio dentro de un único proceso de larga
duración. Las tareas en segundo plano siempre activas encajan de forma
natural como tareas de Tokio supervisadas junto al servidor HTTP - sin
límite de proceso adicional, sin supervisor externo, sin un canal IPC
separado para el estado. El trait `Supervisor` es el equivalente
dentro del proceso a `supervisord`, acotado al árbol de tareas propio
del framework, con las mismas garantías de reinicio-al-salir +
backoff.

Los workers de `Queue` (que Laravel también tiene) igual se incluyen -
consulta [Cola](queues.md) - para trabajo de jobs discretos. Los
supervisores cubren el caso de "siempre en tick" que Laravel empuja
por completo fuera del límite del framework.

## Fuera del alcance de la v1

Los siguientes puntos se dejan deliberadamente para más adelante:

- **Árboles de supervisión (padre/hijo).** No hay jerarquía - todos
  los supervisores son pares bajo el único `SupervisorRegistry`. La
  supervisión estructurada (donde un supervisor posee y reinicia
  supervisores hijos) es territorio del orquestador.

- **Límites de recursos (cgroup, memoria, CPU).** Aplica las
  restricciones de recursos mediante archivos de unidad de systemd
  (`MemoryMax=`, `CPUQuota=`) o mediante requests/limits de recursos
  de Kubernetes a nivel de pod. El framework no impone límites de
  recursos internos al proceso sobre tareas de supervisor
  individuales.

- **Supervisión multi-máquina.** Los supervisores se ejecutan dentro
  de un único proceso en una única máquina. Distribuir las decisiones
  de supervisión entre máquinas es territorio del orquestador
  (Kubernetes, Nomad, systemd en varios hosts).

## Referencia

Los cuatro tipos principales - `Supervisor`, `RestartPolicy`,
`SupervisorEntry`, `SupervisorRegistry` - se reexportan en la raíz del
crate (`suprnova::Supervisor`, etc.) además de la ruta más larga
`suprnova::supervisor::*`. Los dos accesores libres permanecen bajo
`suprnova::supervisor::*`.

| Símbolo | Propósito |
|--------|---------|
| `Supervisor` | Trait a implementar sobre tu struct de supervisor. Métodos requeridos: `name() -> &'static str`, `async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError>`. Opcional: `restart_policy() -> RestartPolicy` (por defecto `OnError`). El token `cancel` se señaliza al apagar el proceso; haz `select` sobre `cancel.cancelled()` para salir de forma limpia antes de que expire la ventana de aborto de 5 segundos. |
| `RestartPolicy` | Enum con las variantes `OnError`, `Always`, `Never`. Controla cuándo el registro lanza una tarea de reemplazo. |
| `SupervisorEntry` | Elemento de inventario. Declara `factory: fn() -> Box<dyn Supervisor>`. Envía una entrada por supervisor mediante `suprnova::inventory::submit!(SupervisorEntry { factory: || Box::new(MySupervisor) })`. |
| `SupervisorRegistry::start_all()` | Fn async. Recorre todos los valores `SupervisorEntry` enviados, lanza cada supervisor como una tarea de Tokio independiente dentro del `JoinSet` del proceso, y empieza a vigilar los reinicios. Idempotente - los estáticos por proceso son `OnceLock`s. Llámala una vez desde tu `register()` de bootstrap. |
| `SupervisorRegistry::shutdown(timeout)` | Fn async. Cancela el token de cancelación compartido para que cada supervisor que vigila `cancel.cancelled()` termine, drena el `JoinSet` hasta `timeout`, y luego hace `abort_all` con los rezagados. `Server::run` invoca esto como parte de su secuencia de apagado; los integradores y los tests de integración que llaman a `start_all` fuera de `Server::run` deben llamarlo ellos mismos para evitar que se filtren tareas. No-op si `start_all` nunca se llamó. |
| `suprnova::supervisor::supervisor_tasks()` / `supervisor_cancel_token()` | Accesores que devuelven `Option<&'static …>` hacia el `JoinSet` y el token de cancelación subyacentes. Los usa la secuencia de apagado de `Server::run`; se exponen como `pub` para que los integradores que conducen el framework desde un binario propio puedan integrarse. El código de aplicación no debería necesitarlos. |

## Siguiente

- [Cola](queues.md) - la decisión entre supervisor y worker de cola, y
  la alternativa de jobs discretos
- [Programación de tareas](scheduling.md) - para trabajo periódico que
  no necesita un bucle de larga duración
- [Flujos de trabajo](workflows.md) - para trabajo con estado y de
  larga duración que necesita reanudación durable
- [Difusión](broadcasting.md) - usa la misma secuencia de apagado
  (orden de drenaje)
- [Ciclo de vida de la solicitud](lifecycle.md) - dónde encajan
  `Server::run` y el drenaje de apagado

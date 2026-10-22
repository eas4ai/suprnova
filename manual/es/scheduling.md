# Programación de tareas

Las tareas programadas son funciones async que el framework ejecuta según
una expresión cron - cada minuto, cada hora, diariamente, semanalmente, o
cualquier cron personalizado de 5 campos. Las tareas viven dentro del
binario de tu aplicación; `schedule:run` evalúa las tareas vencidas una
vez (llámalo desde el cron del sistema) y `schedule:work` ejecuta el
mismo evaluador como un demonio de larga duración.

## Generar tareas

La forma más rápida de crear una tarea programada nueva es usando la CLI
de suprnova:

```bash
suprnova make:task CleanupLogs
```

Este comando:
1. Crea `src/tasks/cleanup_logs_task.rs` con un stub de tarea funcional
2. Crea `src/tasks/mod.rs` si no existe, reexportando la tarea
3. Crea `src/schedule.rs` para registrar tareas, si no existe
4. Declara `pub mod schedule;` y `pub mod tasks;` en `src/lib.rs`
5. Conecta `.schedule(<crate>::schedule::register)` en el builder de tu
   aplicación en `cmd/main.rs` (o `src/main.rs` para el starter de API)

Los pasos 2-5 son idempotentes, así que volver a ejecutar `make:task`
repara las conexiones que se hayan eliminado a mano. El planificador se
ejecuta dentro del binario de tu aplicación - no hay ningún ejecutable de
planificador separado que compilar ni desplegar.

```bash Examples
# Crea CleanupLogsTask en src/tasks/cleanup_logs_task.rs
suprnova make:task CleanupLogs

# Crea SendRemindersTask en src/tasks/send_reminders_task.rs
suprnova make:task SendReminders

# También puedes incluir el sufijo "Task" (mismo resultado)
suprnova make:task BackupDatabaseTask
```

```rust Generated File
//! CleanupLogsTask scheduled task
//!
//! Created with `suprnova make:task cleanup_logs_task`.

use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

/// CleanupLogsTask - A scheduled task.
///
/// Register the task in `src/schedule.rs` with the fluent API; the skeleton
/// below times its own run and prints a structured log line on each
/// invocation so it works end-to-end the first time you wire it up.
pub struct CleanupLogsTask;

impl CleanupLogsTask {
    /// Create a new instance of this task.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CleanupLogsTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        let started_at = Instant::now();
        println!("[CleanupLogsTask] task started");

        // Replace this with the real job. The skeleton ships as a
        // no-op success so the task can be scheduled and observed
        // before the implementation is filled in.

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

## Definir programaciones

suprnova admite dos enfoques para definir tareas programadas:

### 1. Tareas basadas en trait (recomendado)

Para tareas complejas que necesitan dependencias o lógica reutilizable,
implementa el trait `Task` y configura la programación durante el
registro:

```rust
// src/tasks/cleanup_logs_task.rs
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::{Task, TaskResult};
use crate::models::Log;

pub struct CleanupLogsTask;

impl CleanupLogsTask {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        // Eloquent funciona exactamente igual que dentro de un controlador;
        // las tareas ven las mismas vinculaciones del contenedor
        // (`DB::connection()`, `App::get::<T>()`) que ve un handler de
        // solicitud - consulta Arranque de la aplicación más abajo.
        let cutoff = Utc::now() - Duration::days(30);
        Log::query()
            .filter_op("created_at", "<", cutoff)
            .delete_all()
            .await?;

        println!("Old logs cleaned up successfully");
        Ok(())
    }
}
```

Luego regístrala con la API fluida de programación en `src/schedule.rs`:

```rust
// src/schedule.rs
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );
}
```

### 2. Tareas basadas en closure

Para tareas rápidas e inline sin archivos separados:

```rust
// src/schedule.rs
use suprnova::Schedule;

pub fn register(schedule: &mut Schedule) {
    // Tarea de closure simple
    schedule.add(
        schedule.call(|| async {
            println!("Ping! Running every minute");
            Ok(())
        })
        .every_minute()
        .name("heartbeat")
    );

    // Tarea de closure configurada
    schedule.add(
        schedule.call(|| async {
            // La lógica de tu tarea
            Ok(())
        })
        .daily()
        .at("09:00")
        .name("morning-report")
        .description("Sends daily morning report")
    );
}
```

## Registrar tareas

Registra tus tareas en `src/schedule.rs`:

```rust
// src/schedule.rs
use suprnova::Schedule;
use crate::tasks;

pub fn register(schedule: &mut Schedule) {
    // Tareas basadas en trait con configuración de programación fluida
    schedule.add(
        schedule.task(tasks::CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );

    schedule.add(
        schedule.task(tasks::SendRemindersTask::new())
            .daily()
            .at("09:00")
            .name("send:reminders")
            .description("Sends daily reminder emails")
    );

    schedule.add(
        schedule.task(tasks::BackupDatabaseTask::new())
            .weekly()
            .at("00:00")
            .name("backup:database")
            .description("Weekly database backup")
            .without_overlapping()
    );

    // Tareas basadas en closure
    schedule.add(
        schedule.call(|| async {
            println!("Quick task!");
            Ok(())
        })
        .hourly()
        .name("quick-task")
    );
}
```

## Opciones de frecuencia de programación

suprnova ofrece una API fluida para definir cuándo deben ejecutarse las
tareas:

### Intervalos comunes

| Método | Descripción |
|--------|-------------|
| `.every_minute()` | Se ejecuta cada minuto |
| `.every_two_minutes()` | Se ejecuta cada 2 minutos |
| `.every_five_minutes()` | Se ejecuta cada 5 minutos |
| `.every_ten_minutes()` | Se ejecuta cada 10 minutos |
| `.every_fifteen_minutes()` | Se ejecuta cada 15 minutos |
| `.every_thirty_minutes()` | Se ejecuta cada 30 minutos |
| `.hourly()` | Se ejecuta cada hora en el minuto 0 |
| `.hourly_at(30)` | Se ejecuta cada hora en el minuto 30 |
| `.every_two_hours()` / `.every_three_hours()` / `.every_four_hours()` / `.every_six_hours()` | Se ejecuta en punto cada N horas |
| `.daily()` | Se ejecuta a diario a medianoche |
| `.daily_at("03:00")` | Se ejecuta a diario a las 3:00 |
| `.twice_daily(1, 13)` | Se ejecuta dos veces al día (por ejemplo, a la 1:00 y a las 13:00) |
| `.weekly()` | Se ejecuta cada semana, el domingo a medianoche |
| `.monthly()` | Se ejecuta cada mes, el día 1 a medianoche |
| `.monthly_on(15)` | Se ejecuta cada mes en un día concreto |
| `.quarterly()` | Se ejecuta el día 1 de enero/abril/julio/octubre a medianoche |
| `.yearly()` | Se ejecuta el 1 de enero a medianoche |

### Programaciones para días concretos

```rust
use suprnova::DayOfWeek;

// Ejecución en días concretos
.weekly_on(DayOfWeek::Monday)
.weekly_on(DayOfWeek::Friday)

// Métodos abreviados por día
.sundays()
.mondays()
.tuesdays()
.wednesdays()
.thursdays()
.fridays()
.saturdays()

// Varios días
.days(&[DayOfWeek::Monday, DayOfWeek::Wednesday, DayOfWeek::Friday])

// Días laborables / fines de semana
.weekdays()  // De lunes a viernes
.weekends()  // Sábado y domingo
```

### Modificadores de hora

Encadena `.at()` con cualquier programación para fijar una hora concreta:

```rust
.daily().at("14:30")           // A diario a las 14:30
.weekly().at("09:00")          // Cada semana a las 9:00
.mondays().at("08:00")         // Todos los lunes a las 8:00
.monthly().at("00:00")         // El día 1 del mes a medianoche
```

### Zonas horarias

Por defecto, el planificador lee cada expresión cron contra la zona local
del proceso, sea cual sea el `TZ` con el que se arrancó el contenedor.
Fija una tarea a una zona IANA con nombre cuando su programación
pertenezca a un lugar y no a un servidor:

```rust
use suprnova::chrono_tz;

schedule.add(
    schedule.task(GenerateReportTask::new())
        .daily()
        .at("02:00")
        .timezone(chrono_tz::America::New_York)
        .name("report:generate")
);
```

`timezone` toma un `chrono_tz::Tz` tipado, así que una zona mal escrita es
un error de compilación y no una tarea que se ejecuta en silencio a la
hora equivocada. Las constantes de zona viven bajo `suprnova::chrono_tz`
(`chrono_tz::Asia::Tokyo`, `chrono_tz::Europe::Berlin`, etc.),
reexportadas para que no necesites `chrono-tz` en tu propio `Cargo.toml`.

Cuando el nombre de la zona solo existe en tiempo de ejecución - un valor
de configuración, una columna de tenant - usa la contraparte falible:

```rust
schedule.add(
    schedule.task(GenerateReportTask::new())
        .daily()
        .at("02:00")
        .try_timezone(&tenant.timezone)?   // Err(String) ante una zona desconocida
        .name("report:generate")
);
```

Una zona fijada cambia exactamente una cosa: contra qué reloj de pared se
leen los cinco campos del cron. El planificador sigue haciendo tick una
vez por minuto de proceso, y la compuerta de deduplicación del mismo
minuto no se ve afectada.

#### Un valor por defecto para toda la programación

Si la mayoría de tus tareas pertenecen a una misma zona de negocio,
fíjala una vez en la programación en lugar de repetirla en cada tarea:

```rust
pub fn register(schedule: &mut Schedule) {
    schedule.timezone(chrono_tz::America::Chicago);

    // Se lee como las 02:00 de America/Chicago
    let nightly = schedule
        .call(|| async { Ok(()) })
        .daily()
        .at("02:00")
        .name("nightly");
    schedule.add(nightly);

    // Una zona explícita por tarea siempre gana
    let tokyo = schedule
        .call(|| async { Ok(()) })
        .daily()
        .at("09:00")
        .timezone(chrono_tz::Asia::Tokyo)
        .name("tokyo-open");
    schedule.add(tokyo);
}
```

El valor por defecto se aplica cuando se añade una tarea, así que cubre
las tareas registradas después de la llamada y deja intactas las
anteriores.

#### Horario de verano

Algunas zonas observan el horario de verano. Cuando los relojes cambian,
una tarea fijada a una zona así puede ejecutarse dos veces o no
ejecutarse en absoluto:

- Al atrasar la hora, una misma hora de reloj de pared ocurre dos veces.
  Una tarea a las `01:30` casa en las dos pasadas. Son dos minutos
  distintos de tiempo real, así que la compuerta de deduplicación del
  mismo minuto no los fusiona y la tarea se ejecuta dos veces.
- Al adelantar la hora, una hora de reloj de pared no ocurre nunca. Una
  tarea a las `02:30` se omite por completo ese día.

Evita la programación por zona horaria siempre que puedas, y prefiere una
zona sin horario de verano (`chrono_tz::UTC`) para todo lo que deba
ejecutarse exactamente una vez.

#### Leer el listado en otra zona

`schedule:list` acepta `--timezone` y muestra tanto la expresión cron
como la hora de la próxima ejecución tal como se leen en esa zona.
Consulta [Listar tareas](#listar-tareas) para ver una salida trabajada.

### Por qué Suprnova diverge: zonas horarias

El `timezone()` de Laravel toma una cadena, y su valor por defecto para
toda la programación viene de una clave de configuración
`app.schedule_timezone`. Suprnova toma un `chrono_tz::Tz` tipado y no
tiene clave de configuración: `Schedule::timezone` dentro de tu función
`schedule::register` es el único sitio donde se fija un valor por
defecto, así que la programación se lee de arriba abajo sin un segundo
archivo que consultar.

El valor por defecto de Suprnova cuando no hay nada fijado es la zona
local del proceso, no una zona horaria de aplicación configurada. Ese es
el comportamiento que el planificador ha tenido siempre, y sigue siendo
el valor por defecto para que añadir esta funcionalidad no cambie nada en
las programaciones que no la usan.

### Expresiones cron personalizadas

Para un control total, usa la sintaxis cron:

```rust
// Formato cron estándar: minuto hora día-del-mes mes día-de-la-semana
.cron("0 */2 * * *")    // Cada 2 horas
.cron("30 4 * * 1-5")   // A las 4:30 en días laborables
.cron("0 0 1,15 * *")   // El 1 y el 15 de cada mes
```

`.cron(...)` **entra en pánico** si la expresión está malformada (número
de campos incorrecto, paso/rango/lista imposible de parsear). Usa
`.try_cron(expr)` cuando la expresión llegue en tiempo de ejecución
(configuración, entrada del usuario) y prefieras propagar el error de
parseo:

```rust
schedule.add(
    schedule.task(MyTask::new())
        .try_cron(env_expr)?   // devuelve Err(String) ante una expresión incorrecta
        .name("from-config")
);
```

El mismo par `panic` / `try_*` existe en cada método del builder con
rangos numéricos: `try_hourly_at`, `try_daily_at`, `try_twice_daily`,
`try_monthly_on`. Las variantes infalibles entran en pánico ante valores
numéricos fuera de rango (por ejemplo, `daily_at("25:00")` o
`monthly_on(40)`); las contrapartes falibles devuelven `Err(String)`.

## Configuración de tareas

### Prevenir el solapamiento

Omite un tick cuando una ejecución anterior de la misma tarea todavía
está en vuelo:

```rust
schedule.add(
    schedule.task(LongRunningTask::new())
        .daily()
        .name("long-task")
        .without_overlapping()
);
```

**Cómo funciona el bloqueo.** Cuando el flag está activado, suprnova
intenta adquirir un mutex distribuido a través del backend de
[`Cache`](cache.md) configurado (`schedule:lock:<task-name>`). Una
adquisición exitosa ejecuta la tarea y libera el bloqueo; una adquisición
que choca con contención se reporta como una omisión exitosa - `Ok(())`,
con el contador de omisiones de la tarea incrementado para que las
superficies de observabilidad puedan verlo sin envenenar el código de
salida de `schedule:run`.

**Se requiere Cache para la protección entre procesos.** Si ejecutas
varios procesos que programan la misma tarea (por ejemplo, varias
máquinas invocando `suprnova schedule:run` desde el cron del sistema, o
demonios `schedule:work` detrás de un balanceador de carga), el backend
de Cache es lo que los coordina. **Sin un Cache configurado,
`without_overlapping()` se degrada silenciosamente a un `AtomicBool` por
proceso** - dos procesos separados no verán los bloqueos del otro. El
framework emite un `WARN` de una sola vez (`suprnova::schedule`) la
primera vez que se activa este mecanismo de respaldo, para que los
operadores noten la garantía más débil:

> `without_overlapping() falling back to in-process AtomicBool protection - Cache is not bootstrapped. Multi-process deployments will NOT see each other's locks. Configure Cache (CACHE_DRIVER=memory|redis) before relying on cross-process overlap protection.`

**TTL de bloqueo personalizado.** El TTL del bloqueo tiene un valor por
defecto de 30 minutos - suficientemente largo para que la mayoría de las
tareas terminen, suficientemente corto para que una tarea que se cayó
mientras sostenía el bloqueo desbloquee el siguiente tick sin
intervención del operador. Anúlalo por tarea con
`.without_overlapping_for(Duration)`. `Duration::ZERO` no está definido
de forma consistente entre los backends de caché (Redis da error, en
memoria expira al instante, Memcached lo trata como "nunca expira"), así
que el builder lo coerciona al valor por defecto de 30 minutos con un
`WARN` de una sola vez para que el operador pueda corregir el sitio de la
llamada.

```rust
use std::time::Duration;

schedule.add(
    schedule.task(SlowBackupTask::new())
        .daily()
        .name("backup:full")
        // Este job legítimamente se ejecuta más tiempo que el valor por
        // defecto de 30 minutos; dale al bloqueo un TTL de 2 horas para
        // que una ejecución lenta no se vea desalojada por el siguiente
        // tick.
        .without_overlapping_for(Duration::from_secs(2 * 3600))
);
```

### Ejecutar en un solo servidor

Ejecuta una tarea exactamente una vez por cada tick vencido, sin importar
cuántas réplicas estén ejecutando el planificador:

```rust
schedule.add(
    schedule.task(NightlyBillingTask::new())
        .daily()
        .at("02:00")
        .name("billing:nightly")
        .on_one_server()
);
```

**Qué sale mal sin esto.** Cada réplica que ejecuta `schedule:work`
evalúa la programación de forma independiente, y nada impide que todas
decidan que el mismo tick les pertenece. Se midieron tres réplicas
produciendo tres ejecuciones de la misma tarea, cada minuto, sin
variación. Para un job de facturación nocturna, eso significa que a cada
cliente se le cobra tres veces.

**Por qué `without_overlapping()` no cubre esto.** Los dos se parecen,
pero resuelven problemas distintos:

| | Clave del bloqueo | Se sostiene durante | Evita |
|---|---|---|---|
| `without_overlapping()` | la tarea | la duración de la tarea | que una ejecución lenta se solape con su propio siguiente tick |
| `on_one_server()` | la tarea **+ el tick** | la ventana del tick | que una segunda réplica ejecute el mismo tick |

La distinción que importa es cuándo se libera el bloqueo.
`without_overlapping()` se libera en cuanto el handler retorna - para una
tarea rápida, antes incluso de que una segunda réplica haya mirado, así
que las N igual se ejecutan. `on_one_server()` deliberadamente sostiene
su bloqueo más allá del handler y lo deja expirar por TTL, porque una
réplica que llega más tarde en el mismo tick tiene que encontrarlo
tomado.

Se combinan. Una tarea de larga duración que también deba ser de un solo
servidor usa ambos.

**Requiere una caché compartida.** La elección es un bloqueo de
[`Cache`](cache.md), así que "un servidor" significa "un proceso entre
los que comparten un backend de caché". Bajo `CACHE_DRIVER=memory` el
bloqueo vive en el heap de un único proceso, cada réplica gana su propia
elección, y la garantía está silenciosamente ausente.

En producción eso es un **fallo de arranque**, no una advertencia:

> `refusing to boot in production: 1 task(s) request single-server execution (billing:nightly) but CACHE_DRIVER is memory or unset, so the election lock lives in this process's heap. Every replica would win its own election and run the task, which is what on_one_server() exists to prevent. Set CACHE_DRIVER=redis with REDIS_URL, or set SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true to acknowledge per-process locking - which is only accurate if you run exactly one scheduler.`

Establece `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` si tu
despliegue de verdad ejecuta un único planificador. Fuera de producción
el driver de memoria sigue siendo usable y el framework en su lugar
advierte una sola vez.

**TTL de bloqueo personalizado.** Por defecto son 60 segundos - un tick
alineado al minuto. Ambos extremos importan: demasiado corto y una
réplica cuyo tick llega unos segundos tarde encuentra el bloqueo ya
liberado y ejecuta la tarea otra vez; demasiado largo y el bloqueo
sobrevive a su tick, así que la *siguiente* ejecución vencida lo
encuentra tomado y se omite por completo. Usa `.on_one_server_for(Duration)`
para programaciones de grano más grueso.

```rust
use std::time::Duration;

schedule.add(
    schedule.task(HourlyRollupTask::new())
        .hourly()
        .name("rollup:hourly")
        // Una tarea horaria solo necesita que el bloqueo sobreviva a la
        // ventana en la que las réplicas todavía podrían considerar
        // vencido este tick.
        .on_one_server_for(Duration::from_secs(300))
);
```

**Si la caché es inalcanzable**, el tick se omite en lugar de
ejecutarse. Perder la coordinación es el peor momento posible para dejar
pasar a todas las réplicas: un tick omitido es recuperable en el
siguiente tick, los efectos secundarios duplicados por lo general no lo
son.

### Por qué Suprnova diverge

El `onOneServer()` de Laravel es el mismo mecanismo opt-in, y Suprnova lo
conserva: las tareas por servidor - rotación de logs, calentar una caché
local - son legítimas y siguen siendo expresables.

Donde diverge es en el modo de fallo. Laravel ejecuta `onOneServer()` sin
problema alguno contra un driver de caché que no puede coordinar.
Suprnova en cambio se niega a arrancar en producción, con el mismo
razonamiento que el limitador de velocidad en memoria: un control que
silenciosamente hace mucho menos de lo que promete es peor que uno que
está visiblemente ausente.

### Ejecutar en segundo plano

Desacopla las tareas de la ruta crítica por tick para que no bloqueen el
inicio de otras tareas vencidas:

```rust
schedule.add(
    schedule.task(BackgroundTask::new())
        .hourly()
        .name("background-task")
        .run_in_background()
);
```

**Aislamiento de pánicos.** Las tareas en segundo plano se ejecutan
dentro de un `tokio::task::JoinSet` con `catch_unwind`, así que una tarea
que entra en pánico emerge como un `FrameworkError` registrado contra el
nombre de la tarea, en lugar de derribar el planificador. El demonio
`schedule:work` drena el JoinSet al apagarse (Ctrl-C / SIGTERM) para que
las tareas en segundo plano en vuelo se completen antes de salir.

**Combínalo con `without_overlapping`.** Los dos flags se combinan - una
tarea en segundo plano con `without_overlapping()` se lanzará dentro del
JoinSet y adquirirá el bloqueo de solapamiento desde dentro del future
lanzado, así que la semántica de bloqueo descrita arriba sigue aplicando.

### Deduplicación en el mismo minuto

La resolución de cron es a nivel de minuto, y suprnova hace cumplir eso:
si a la misma tarea se le pide ejecutarse dos veces dentro del mismo
minuto de reloj dentro de un único proceso, la segunda llamada es una
omisión sin efecto - `Ok(())`, con el contador de omisiones de la tarea
incrementado. Esto cierra una clase de bug en la que un bucle de demonio
o una invocación ajustada de `schedule:run` podría ejecutar una tarea
`.every_minute()` varias veces en el mismo minuto.

Esta compuerta dentro del proceso está **siempre activa**,
independientemente de `without_overlapping`. NO abarca varios procesos
(cada proceso tiene su propio estado por tarea). Si necesitas
coordinación entre procesos en el mismo minuto, añade `without_overlapping` + un backend de Cache configurado - juntos cubren ambas direcciones.

## Ejecutar el planificador

suprnova ofrece comandos de CLI para ejecutar las tareas programadas:

### Ejecutar una vez

Ejecuta todas las tareas vencidas una sola vez (normalmente lo invoca
cron cada minuto):

```bash
suprnova schedule:run
```

### Modo demonio

Se ejecuta de forma continua y comprueba cada minuto si hay tareas
vencidas:

```bash
suprnova schedule:work
```

Es lo ideal para desarrollo o cuando usas un gestor de procesos como
systemd.

### Listar tareas

Muestra todas las tareas programadas registradas:

```bash
suprnova schedule:list
```

Salida:
```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] next: 2026-05-29 03:00 UTC
  send:reminders [0 9 * * *] next: 2026-05-28 09:00 UTC
  report:generate [0 6 * * *] (UTC) next: 2026-05-29 06:00 UTC
```

Cada línea es el nombre de la tarea, la expresión cron, una etiqueta de
zona opcional, la próxima vez que la tarea se dispara y la descripción de
la tarea, si la tiene.

`next:` es el primer minuto posterior a ahora en el que la expresión
casa, calculado en la zona en la que se evalúa la tarea y mostrado luego
en la zona del listado. Una expresión que nunca puede casar (`0 0 30 2 *`
nombra una fecha que no existe) imprime `next: never`.

La zona del listado es UTC salvo que pases `--timezone`. `cleanup:logs` y
`send:reminders` de arriba no fijaron zona, así que sus expresiones se
imprimen tal como están escritas - el planificador las lee contra la zona
local del proceso, que no tiene un nombre IANA desde el que convertir - y
no llevan etiqueta de zona. `report:generate` fijó `America/New_York` y
pidió las `02:00`, así que su expresión se reescribe a la zona del
listado y se etiqueta con ella.

```bash
suprnova schedule:list --timezone=Asia/Tokyo
```

```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] next: 2026-05-29 12:00 JST
  send:reminders [0 9 * * *] next: 2026-05-28 18:00 JST
  report:generate [0 15 * * *] (Asia/Tokyo) next: 2026-05-29 15:00 JST
```

Una sola tarea puede ocupar varias líneas. Una expresión que cruza la
medianoche en la zona del listado necesita una línea de cron por cada
lado, porque ninguna expresión única de cinco campos describe ambos:

```
  monday-digest [0 23 * * 1] (Asia/Tokyo) next: 2026-06-01 23:00 JST
  monday-digest [0 5 * * 2] (Asia/Tokyo) next: 2026-06-01 23:00 JST
```

`next:` pertenece a la tarea, no a la línea, así que se repite: ambas
líneas describen la misma tarea y la misma ejecución próxima.

Algunas conversiones se rechazan en lugar de aproximarse, y la expresión
rechazada se imprime exactamente como está escrita, etiquetada con la
zona propia de la tarea. Una conversión se rechaza cuando una transición
de horario de verano cae entre las dos próximas ejecuciones (ninguna
expresión es correcta en ambos lados), cuando un cambio de día tendría
que mover a la vez un día del mes restringido y un día de la semana
restringido (cron aplica un OR entre esos dos campos, así que desplazar
ambos cambiaría qué días casan), o cuando un cambio de día tendría que
decidir cuántos días tiene febrero.

## Configuración de producción

### Usar cron

Añade una única entrada de cron para ejecutar el planificador cada
minuto:

```bash
* * * * * cd /path/to/your/project && suprnova schedule:run >> /dev/null 2>&1
```

**Coordinación entre procesos.** Si ejecutas `schedule:run` desde el
cron del sistema en más de un host (o junto a un demonio `schedule:work`),
las tareas con `.without_overlapping()` necesitan un backend de **Cache**
configurado (`CACHE_DRIVER=redis` recomendado para producción) para
coordinarse entre procesos. Sin él, el flag de solapamiento se degrada a
protección por proceso y la misma tarea puede ejecutarse en varios hosts
en el mismo minuto. Consulta [Prevenir el solapamiento](#prevenir-el-solapamiento)
más arriba para la semántica de bloqueo completa.

### Usar systemd

Crea un servicio de systemd para el demonio del planificador:

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/path/to/suprnova schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

## Acceder al contexto de la app

Las tareas programadas tienen acceso completo al contexto de la
aplicación, igual que los controladores:

```rust
use async_trait::async_trait;
use suprnova::{App, Task, TaskResult};
use crate::actions::SendEmailAction;
use crate::models::User;

pub struct SendRemindersTask;

#[async_trait]
impl Task for SendRemindersTask {
    async fn handle(&self) -> TaskResult {
        // Eloquent: `.get()` devuelve una `Collection<User>` que puedes iterar.
        let users = User::query()
            .filter("reminder_enabled", true)
            .get()
            .await?;

        // Cualquier cosa vinculada en `bootstrap.rs` también es alcanzable aquí.
        let send_email = App::get::<SendEmailAction>()
            .expect("SendEmailAction bound in bootstrap()");

        for user in users.iter() {
            send_email.execute(&user.email, "Daily Reminder").await?;
        }

        Ok(())
    }
}
```

## Organización de archivos

La estructura de archivos recomendada para tareas programadas:

```
src/
├── tasks/
│   ├── mod.rs              # Reexporta todas las tareas (actualizado automáticamente por make:task)
│   ├── cleanup_logs_task.rs
│   ├── send_reminders_task.rs
│   └── backup_database_task.rs
├── schedule.rs             # Registra tareas (ejecutado por los comandos schedule:*)
├── bootstrap.rs
├── routes.rs
└── lib.rs                  # Declara `pub mod schedule;` + `pub mod tasks;`
cmd/
└── main.rs                 # Llama a `.schedule(<crate>::schedule::register)`
```

**src/tasks/mod.rs:**
```rust
pub mod cleanup_logs_task;
pub mod send_reminders_task;
pub mod backup_database_task;

pub use cleanup_logs_task::CleanupLogsTask;
pub use send_reminders_task::SendRemindersTask;
pub use backup_database_task::BackupDatabaseTask;
```

## Conectar el planificador a tu aplicación

`make:task` conecta `.schedule(<crate>::schedule::register)` en tu
builder de `Application` automáticamente. Si construyes la cadena a
mano, la llamada relevante está en `Application`:

```rust
// cmd/main.rs (o src/main.rs para el starter de api)
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)        // <- esta línea
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await;
```

Sin `.schedule(...)` todos los subcomandos `schedule:*` reportan que no
hay ninguna tarea registrada. `schedule:work` y `schedule:run` también
ejecutan los mismos drivers de runtime y la misma `bootstrap_fn` que el
servidor HTTP, así que los observadores, los oyentes, y las vinculaciones
del contenedor registradas en el arranque son visibles para los handlers
de tus tareas exactamente igual que para los controladores (consulta
[Arranque de la aplicación](bootstrap.md)).

### Por qué Suprnova diverge

El planificador de Laravel es en sí mismo un único comando de Artisan
(`schedule:run`) que PHP-cron dispara cada minuto. El runtime de PHP
arranca, evalúa las tareas vencidas, las ejecuta dentro del proceso o
invoca un proceso externo, y luego derriba el runtime. PHP no tiene
procesos de larga duración, así que la forma de demonio (`schedule:work`)
la incorporó Lumen y viene incluida en el propio Laravel como una
solución alternativa para sitios sin acceso a crontab.

En Suprnova el demonio es de primera clase. `schedule:work` se ejecuta
dentro de un runtime de Tokio que ya es de larga duración, así que:

- **Las tareas en segundo plano (`run_in_background`) se combinan con el
  bucle de ticks.** Laravel lanza un proceso hijo por cada tarea en
  segundo plano; nosotros lanzamos dentro de un `JoinSet` y hacemos
  emerger las finalizaciones en el siguiente tick o al apagarse.
- **El apagado ordenado es un brazo de `tokio::select!`.** Ctrl-C /
  SIGTERM drena las tareas en segundo plano en vuelo antes de salir; las
  tareas dentro del proceso terminan su llamada actual.
- **La deduplicación en el mismo minuto es estado dentro del proceso.**
  Un atómico `last_run_minute` por tarea garantiza que un único proceso
  no pueda disparar dos veces una tarea alineada al minuto, incluso si
  el bucle marca el tick rápido. PHP no puede hacer esto - cada tick de
  cron es un proceso nuevo - que es la razón por la que Laravel usa
  bloqueos de sistema de archivos como única línea de defensa.

El `without_overlapping` respaldado por `Cache::lock` sigue existiendo
para el caso multi-proceso (cron del sistema en varios hosts, varios
demonios `schedule:work` detrás de un balanceador de carga). Es el mismo
mecanismo, solo que en una capa que el planificador no siempre necesita.

## Resumen

| Funcionalidad | Uso |
|---------|-------|
| Crear tarea | `suprnova make:task TaskName` |
| Basada en trait | Implementa el trait `Task`, configura la programación durante el registro |
| Basada en closure | `schedule.call(\|\| async { ... })` |
| Registrar tareas | `schedule.add(schedule.task(...).daily().name("..."))` |
| Conectarla a la app | `Application::new().schedule(schedule::register)` |
| Ejecutar una vez | `suprnova schedule:run` |
| Ejecutar como demonio | `suprnova schedule:work` |
| Listar tareas | `suprnova schedule:list` |
| Prevenir solapamiento | `.without_overlapping()` (TTL de bloqueo de 30 min por defecto vía backend de Cache) |
| TTL de solapamiento personalizado | `.without_overlapping_for(Duration)` |
| Segundo plano | `.run_in_background()` (aislado de pánicos vía JoinSet) |
| Deduplicación en el mismo minuto | Siempre activa por proceso; las ejecuciones omitidas devuelven `Ok(())` |
| Cron validado en tiempo de ejecución | `.try_cron(expr)` / `.try_daily_at(s)` / `.try_hourly_at(n)` |

## Siguiente

- [Comandos de programación](cli-scheduling.md) - referencia de CLI para
  `schedule:run` / `schedule:work` / `schedule:list`
- [Cola](queues.md) - para trabajo que debería recoger un worker en lugar
  de marcar el tick de un reloj
- [Consola](console.md) - `#[command]` para tareas de operador de una
  sola vez (no programadas)
- [Caché](cache.md) - el backend que impulsa el `without_overlapping`
  entre procesos
- [Arranque de la aplicación](bootstrap.md) - cómo se conecta
  `.schedule(...)` al builder, y qué pueden resolver las tareas desde el
  contenedor

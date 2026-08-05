# Flujos de trabajo

Los flujos de trabajo son funciones asíncronas durables y de larga
duración cuyo estado intermedio sobrevive a caídas, reinicios y pánicos.
Recurre a ellos cuando una unidad de trabajo abarca varios pasos - cada
uno potencialmente lento, falible o con efectos secundarios - y no puedes
permitirte perder el progreso a mitad de camino. El cuerpo de un flujo de
trabajo se ejecuta una vez; la salida de cada paso se persiste; un
reintento reanuda desde el primer paso que todavía no se completó.
Emparéjalo con [`Queue`](queues.md) cuando el trabajo sea un job de una
sola vez; emparéjalo con [`Bus`](bus.md) cuando el trabajo se ejecute de
forma síncrona dentro de la tarea de la solicitud.

## Inicio rápido

Un flujo de trabajo es una función async que devuelve `Result<T,
FrameworkError>`; su cuerpo invoca una o más funciones `#[workflow_step]`;
lo encolas mediante la macro `start_workflow!` y un proceso worker lo
drena.

```rust
use suprnova::{workflow, workflow_step, start_workflow, FrameworkError};

#[workflow_step]
async fn fetch_user(user_id: i64) -> Result<String, FrameworkError> {
    Ok(format!("user:{}", user_id))
}

#[workflow_step]
async fn send_welcome_email(user: String) -> Result<(), FrameworkError> {
    // … de verdad enviar el correo
    Ok(())
}

#[workflow]
async fn welcome_flow(user_id: i64) -> Result<(), FrameworkError> {
    let user = fetch_user(user_id).await?;
    send_welcome_email(user).await?;
    Ok(())
}

// Desde un handler o cualquier contexto async:
let handle = start_workflow!(welcome_flow, 123).await?;
```

La macro serializa los argumentos a JSON, inserta una fila en la tabla
`workflows`, y devuelve un [`WorkflowHandle`](#esperar-los-resultados) que
identifica la instancia encolada. Un proceso worker separado recoge la
fila, ejecuta el cuerpo, y persiste la salida de cada paso sobre la
marcha.

`#[workflow]` recopila la función dentro del inventario de flujos de
trabajo bajo su ruta totalmente calificada (`module_path::fn_name`). Los
registros duplicados bajo el mismo nombre abortan el arranque del worker
mediante `registry::assert_no_duplicates` - un sombreado silencioso sería
imposible de depurar, así que el framework falla de forma estrepitosa.

## Esquema

Los flujos de trabajo persisten en dos tablas: `workflows` (una fila por
instancia) y `workflow_steps` (una fila por invocación de paso, indexada
por `(workflow_id, step_index)`). El framework es dueño del esquema; tú
eliges cuándo aplicarlo.

Hay dos formas de conectar las migraciones.

### Archivos de migración generados

La CLI genera mediante andamiaje copias de las migraciones del framework
dentro de tu app:

```bash
suprnova workflow:install
suprnova migrate
```

`workflow:install` escribe `m_create_workflows_table.rs` y
`m_create_workflow_steps_table.rs` bajo `src/migrations/`, y luego los
registra en tu `Migrator`. Usa esto cuando quieras que el esquema quede
versionado junto al resto de las migraciones de tu app.

### Registro programático

Alternativamente, registra directamente los structs de migración que
posee el framework:

```rust
use sea_orm_migration::MigratorTrait;
use suprnova::workflow::migrations::{
    CreateWorkflowsTable, CreateWorkflowStepsTable,
};

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![
            Box::new(CreateWorkflowsTable),
            Box::new(CreateWorkflowStepsTable),
        ]
    }
}
```

Ambas rutas producen SQL idéntico. La misma convención la usan
[`features::migrations`](feature-flags.md) y
[`payments::migrations`](payments.md).

## Ejecutar el worker

En una aplicación con andamiaje, el worker se arranca mediante el
subcomando `workflow:work` del binario:

```bash
suprnova workflow:work
```

El worker ejecuta el mismo bootstrap que tu servidor HTTP, así que los
observadores, los oyentes, y las vinculaciones del contenedor registradas
en `bootstrap()` son visibles para los pasos del flujo de trabajo. Ante
`SIGINT` / `SIGTERM` el worker deja de extraer nuevos reclamos y espera
(`await`) cada flujo de trabajo en vuelo antes de salir - ningún flujo de
trabajo queda huérfano a mitad de un paso en un apagado limpio.

La ruta de reclamo (`claim_next_workflow`) usa `FOR UPDATE SKIP LOCKED`
contra la tabla `workflows`, así que el proceso worker **requiere
Postgres**. SQLite y MySQL funcionan para los tests y para la ruta de
encolado/persistencia, pero el demonio worker saldrá con un error en el
primer reclamo si la conexión no es Postgres.

## Configuración

Cinco variables de entorno ajustan el worker. Los valores fuera de rango
se acotan a mínimos seguros con un `tracing::warn!`, de modo que una
errata en `.env` no pueda inutilizar el demonio.

| Variable | Por defecto | Notas |
|---|---|---|
| `WORKFLOW_POLL_INTERVAL_MS` | `1000` | Espera entre rondas de reclamo vacías |
| `WORKFLOW_CONCURRENCY` | `4` | Máximo de flujos de trabajo en ejecución por worker (mín. 1) |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30` | Duración del lease antes de que otro worker pueda reclamarlo |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | Presupuesto de intentos por flujo de trabajo (mín. 1) |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | Backoff lineal: `attempts * value` (mín. 0) |

Para configuraciones programáticas (construidas en código en lugar de
analizadas desde el entorno), llama a `WorkflowConfig::validate()` para
fallar rápido sobre los mismos invariantes antes de construir un
`WorkflowWorker`.

## Recuperación ante caídas

Tres capas de protección evitan que los flujos de trabajo se queden
atascados por fallos del worker.

**Límite de pánico.** El cuerpo del flujo de trabajo se ejecuta dentro de
`AssertUnwindSafe(...).catch_unwind()`. Un pánico en cualquier paso se
atrapa, el payload se captura en la columna de error, y la fila pasa por
la misma contabilidad de reintento/fallo que un `Err` devuelto. Sin el
límite, un pánico se saltaría la ruta de resolución y dejaría la fila en
`status='running'` para siempre.

**Heartbeat de lease.** Un paso de larga duración que sobrevive a
`WORKFLOW_LOCK_TIMEOUT_SECS` podría, si no fuera por esto, ver cómo su
propio lease expira mientras todavía está en marcha. El worker lanza una
tarea de heartbeat que refresca `locked_until` a la mitad del intervalo
de lock-timeout hasta que el cuerpo se resuelve. El heartbeat se aborta al
descartarse (`drop`), así que un `?` devuelto no puede provocar una fuga
de la tarea de renovación y congelar el lease de un flujo de trabajo que
nadie está ejecutando.

**Reclamo de lease expirado.** Cuando un worker muere sin haber liberado
nunca su bloqueo (kill forzado, caída del host, OOM del kernel), la fila
permanece en `status='running'` hasta que pasa `locked_until`. La
consulta de reclamo recoge explícitamente esas filas: cualquier flujo de
trabajo en `running` cuyo lease haya expirado se vuelve reclamable por
otro worker en la siguiente ronda, incrementando `attempts`. La
recuperación ante caídas es automática - no hay ningún script que
escribir ni ningún comando de administración que recordar.

## Semántica de entrega - al menos una vez

Los cuerpos de paso se ejecutan con semántica de **al menos una vez**. Un
paso puede ejecutarse más de una vez en dos situaciones:

1. **Se devuelve `Err`** - el flujo de trabajo se vuelve a encolar; al
   reintentar, el paso fallido se ejecuta de nuevo, y cualquier paso
   anterior se reproduce desde la caché.
2. **Caída después del efecto secundario, antes de que
   `mark_step_succeeded` confirme** - el lease expira, otro worker lo
   reclama, no ve ninguna salida en caché en ese índice de paso, y
   ejecuta el cuerpo de nuevo.

El framework persiste las **salidas** de los pasos de forma durable, pero
no puede observar el efecto secundario en sí. Hacer que los cuerpos de
paso sean idempotentes es responsabilidad tuya. Dos patrones funcionan
para casi todos los casos.

**Escrituras condicionales.** Usa `INSERT ... ON CONFLICT DO NOTHING`,
columnas de clave de idempotencia, o marcadores `seen_event_id`. Deriva
una clave estable por paso a partir de datos que ya están en el alcance:
los argumentos de entrada del flujo de trabajo más una etiqueta de paso
literal (`("wf-charge", customer_id)`) bastan porque los mismos
argumentos se mapean a la misma fila de flujo de trabajo a través de los
reintentos.

**Claves de idempotencia externas.** La mayoría de las APIs de terceros
(Stripe, SES, SQS) aceptan un encabezado `Idempotency-Key`. Pasa una
clave derivada de la entrada del flujo de trabajo más una etiqueta local
al paso (`format!("wf-charge-{}", customer_id)`) para que las solicitudes
reintentadas se dedupliquen en el proveedor.

**No** asumas que un paso que devolvió `Ok` no puede ejecutarse una
segunda vez - una caída puede hacer que esa segunda ejecución caiga en
cualquier worker posterior, incluso después de un reinicio en un host
distinto. Consulta el capítulo [Idempotencia](idempotency.md) para
`Idempotency::once`, `Idempotency::commit_on_success`, y
`Idempotency::remember` - todos ellos envoltorios válidos alrededor de un
cuerpo de paso.

## Contrato de determinismo

Los flujos de trabajo deben ser deterministas a través de las
reproducciones. Cada paso está indexado por `(step_name, step_index)`, y
el framework almacena en caché su entrada serializada junto con la
salida. Cuando un paso en el mismo índice se reproduce con una entrada
serializada distinta, el framework devuelve un error en lugar de
enmascarar la corrupción devolviendo la salida en caché de la entrada
anterior.

En la práctica, esto significa:

- No bifurques según `Utc::now()`, `rand::random()`, u otras fuentes no
  deterministas fuera de un `#[workflow_step]`. Los cuerpos de paso
  pueden llamarlas libremente - su resultado se captura en la caché de
  salida del paso.
- No insertes pasos de forma condicional. Si un reintento encuentra un
  número distinto de pasos antes de un índice dado, obtienes un error de
  discrepancia de nombre de paso. Pon la lógica de bifurcación dentro de
  un paso.
- No cambies la forma de los argumentos de un paso entre despliegues sin
  renombrar el paso. Renombrarlo cambia `step_name`, lo que reinicia la
  caché desde cero para ese paso.

## Esperar los resultados

`WorkflowHandle` permite que quien llama consulte (poll) la fila, espere
a que termine, u obtenga la salida serializada.

```rust
use std::time::Duration;
use suprnova::{FrameworkError, WorkflowStatus};

let handle = start_workflow!(welcome_flow, 123).await?;

match handle.wait_with_timeout(Duration::from_secs(30)).await {
    Ok(WorkflowStatus::Succeeded) => { /* listo */ }
    Ok(WorkflowStatus::Failed) => { /* columna de error persistida */ }
    Ok(_) => unreachable!("wait_* only returns terminal status"),
    Err(FrameworkError::Internal { message }) if message.contains("Timed out") => {
        // El flujo de trabajo sigue en ejecución; continúa hacia la UX asíncrona.
    }
    Err(other) => return Err(other),
}
```

`wait()` consulta indefinidamente - úsalo solo en tests o en scripts de
vida corta donde bloquearse para siempre sea aceptable. Para las rutas de
solicitud HTTP, `wait_with_timeout(Duration)` siempre gana frente al
bucle de consulta interno, incluso si la consulta de estado subyacente se
atasca. Un error de timeout **no** cancela el flujo de trabajo - el
worker continúa, y `handle.status().await` devuelve más adelante el
estado en vivo.

`wait_with_options(Some(poll), Some(deadline))` expone ambos ajustes
cuando los valores por defecto no encajan.

Para salidas tipadas, define un retorno `T: Serialize + DeserializeOwned`
en el flujo de trabajo y llama a `handle.output::<T>().await?`. El JSON
crudo está disponible vía `output_raw()`.

## Caché de pasos, en detalle

La caché de pasos está indexada por **nombre de paso + índice de paso**.
La primera invocación de un paso persiste su JSON de entrada, ejecuta el
cuerpo, y en caso de éxito persiste el JSON de salida. Una reproducción
en el mismo índice:

- Devuelve la salida en caché si el paso está en `succeeded` y la entrada
  reproducida coincide con la entrada en caché.
- Devuelve un error si la entrada difiere (la salvaguarda de determinismo).
- Vuelve a ejecutar el cuerpo si el paso está en `running` o `failed` (no
  hay salida en caché que devolver).

Los índices de paso los asigna un `AtomicI32` por contexto de flujo de
trabajo, así que el orden queda determinado por las llamadas que hace el
cuerpo de tu flujo de trabajo. Una bifurcación que produce un paso
distinto en el mismo índice durante un reintento emerge como un error de
discrepancia de nombre de paso, en lugar de corromper silenciosamente los
pasos posteriores.

Las salidas y las entradas se almacenan como JSON TEXT, así que todos los
tipos de retorno y los argumentos de los pasos deben ser `Serialize +
DeserializeOwned`.

## Detectar el contexto de flujo de trabajo desde un ayudante

`WorkflowContext::is_active()` devuelve si la tarea actual se está
ejecutando bajo un flujo de trabajo. Úsalo desde ayudantes que necesiten
comportarse de forma distinta dentro frente a fuera del worker - por
ejemplo, un logger que adjunta la etiqueta de flujo de trabajo solo
cuando existe una:

```rust
use suprnova::workflow::WorkflowContext;

fn maybe_workflow_tagged(message: &str) -> String {
    if WorkflowContext::is_active() {
        format!("[workflow] {message}")
    } else {
        message.to_string()
    }
}
```

Fuera de un flujo de trabajo (llamada directamente desde un test o un
handler), una función `#[workflow_step]` igual se ejecuta -
`WorkflowContext::current()` simplemente devuelve `None`, el cuerpo se
ejecuta sin persistencia, y el paso evita la caché por completo. Eso es
intencional: hace que las funciones de paso sean comprobables
individualmente sin tener que levantar un worker.

### Por qué Suprnova diverge

Laravel no tiene una primitiva de flujo de trabajo de primera clase - los
jobs son el vecino más cercano, pero reintentan volviendo a ejecutar todo
el cuerpo del job, no reanudando desde el último paso exitoso. Suprnova
incluye los flujos de trabajo como un constructo separado porque Tokio
abarata el patrón de "quedarse conectado dentro de una función async
lenta durante una hora", y porque la persistencia a nivel de paso es la
abstracción correcta para cualquier interacción externa de varios pasos
(aprovisionar un cliente, ejecutar una saga a través de dos proveedores
de pago, generar un reporte que involucra varias APIs upstream).

El diseño está más cerca de [DBOS](https://www.dbos.dev/) y de
Cadence/Temporal que de una cola: estado durable, reproducción
determinista, límites de paso explícitos. La diferencia con Temporal es
el peso operativo - no hay un servicio de flujo de trabajo separado que
ejecutar; el worker es simplemente `suprnova workflow:work` contra la
base de datos de tu aplicación.

## Notas

- Los cuerpos de paso pueden devolver cualquier tipo `Serialize +
  DeserializeOwned`. El tipo unidad `()` funciona para pasos que existen
  solo por su efecto secundario.
- Una función `#[workflow_step]` llamada fuera de un contexto de flujo de
  trabajo se ejecuta en línea - sin caché, sin reproducción. Así es como
  los tests ejercitan los cuerpos de paso directamente.
- La caché de pasos está indexada por `(step_name, step_index)`; renombra
  un paso (o reordena las llamadas) y la caché se reinicia para ese paso
  en la siguiente reproducción.
- `start_workflow!` acepta cualquier tupla de argumentos serializables.
  Las tuplas preservan el orden de los argumentos, así que renombrar
  parámetros posicionales es seguro; cambiar los tipos de los argumentos
  es una ruptura de esquema para cualquier flujo de trabajo en vuelo.
- La capa de [observabilidad](observability.md) del framework captura
  logs estructurados del worker (`worker_id`, `workflow_id`, `attempts`,
  `max_attempts`) en cada ruta de resolución, de modo que puedas auditar
  los presupuestos de reintento en producción sin instrumentar tus pasos.

## Siguiente

- [Cola](queues.md) - jobs en segundo plano de una sola vez con drivers
  sync/redis/database
- [Idempotencia](idempotency.md) - envoltorios para entrega al menos una
  vez
- [Bus](bus.md) - despacho síncrono de comandos con resultados tipados
- [Supervisores](supervisors.md) - supervisión de tareas de larga
  duración con reinicio automático al atrapar pánicos
- [Modelo de errores](error-model.md) - `FrameworkError`, el límite de
  pánico, y por qué la resolución pasa a través de `?`

# Idempotencia

Cuando un cliente reintenta un POST, quieres que la segunda llamada
sea segura. La red no es confiable y los clientes reintentan - pero
`POST /charges` nunca debería cobrar la tarjeta dos veces, y
`POST /orders` nunca debería producir dos pedidos por un solo clic.
Las claves de idempotencia son el contrato que dice "si vuelves a
ver esta misma clave, dame la respuesta original; no repitas el
trabajo."

`Idempotency` de Suprnova es una fachada fina sobre `Cache::lock` que
te da tres garantías escalonadas: solo deduplicación, deduplicación
con reintento en fallo, y reproducción de resultado al estilo
Stripe. Las tres mantienen viva la concesión (lease) del bloqueo
durante todo el tiempo que el cuerpo se ejecute, así que un cuerpo
lento nunca puede dejar que el bloqueo expire y un duplicado se
cuele.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome: Idempotent<OrderId> = Idempotency::once(
    "create-order:user-42:client-key-abc",
    Duration::from_secs(86_400),
    || async {
        // Corre exactamente una vez por clave dentro de la ventana de 24 horas.
        place_order(&user, &cart).await
    },
)
.await?;

match outcome {
    Idempotent::Fresh(id) => /* primera llamada - id es el pedido nuevo */ {},
    Idempotent::FreshUnfenced(id) => {
        // El pedido se colocó, pero el lease del bloqueo se perdió a
        // mitad de camino, así que otro llamador puede haber colocado
        // uno también. Reconcilia o alerta - ver "Cuando se pierde la
        // exclusividad" más abajo.
    },
    Idempotent::Duplicate => /* la misma clave ya se usó */ {},
}
```

## Las tres primitivas

| Método | El cuerpo corre | El duplicado ve | ¿El fallo libera el bloqueo? | Úsalo cuando |
|---|---|---|---|---|
| `Idempotency::once` | exactamente una vez por ventana | marcador `Duplicate` | no | los efectos secundarios NUNCA deben repetirse (correo enviado, cobro intentado) |
| `Idempotency::commit_on_success` | una vez por éxito por ventana | marcador `Duplicate` | sí | los fallos transitorios deben poder reintentarse, pero un éxito se mantiene |
| `Idempotency::remember` | una vez por éxito por ventana | el valor de retorno original | sí | los duplicados deben recibir el payload original, no un marcador |

Las tres viven bajo `suprnova::idempotency` y se reexportan desde la
raíz del crate como `Idempotency`, `Idempotent`, y `Replay`. Comparten
el mismo hasheo de clave, la misma renovación de lease, y la misma
semántica de bloqueo - solo difiere la política de éxito/fallo.

### `Idempotency::once` - como mucho una vez

El contrato más estricto. El primer llamador dentro de la ventana de
TTL corre el cuerpo y obtiene `Fresh(value)`. Cada llamador
subsiguiente dentro de la ventana obtiene `Duplicate`, y el cuerpo NO
vuelve a correr - incluso si el cuerpo del primer llamador devolvió
`Err`. El TTL ES la ventana de deduplicación.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

// Envía un correo de bienvenida exactamente una vez por registro,
// sin importar cuántas veces reintente el callback de registro.
let result = Idempotency::once(
    &format!("welcome-mail:{}", user.id),
    Duration::from_secs(7 * 24 * 3600),
    || async {
        Mail::to(&user.email).send(WelcomeMail { user: user.clone() }).await
    },
)
.await?;
```

Recurre a `once` cuando el efecto secundario es del tipo "lo intenté;
incluso si fallé después del efecto secundario, no lo intentes de
nuevo" - enviar un correo, publicar en una API externa que no honra
sus propias claves de idempotencia, escribir una entrada de log de
auditoría cuya doble escritura corrompería analíticas río abajo.

### `Idempotency::commit_on_success` - al menos una vez en éxito, reintento en fallo

Como `once`, pero si el cuerpo devuelve `Err`, el bloqueo de
deduplicación se libera para que el siguiente llamador dentro de la
ventana de TTL pueda reintentar. Un cuerpo exitoso mantiene el
bloqueo por el resto de la ventana.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome = Idempotency::commit_on_success(
    &format!("publish-post:{}", post.id),
    Duration::from_secs(300),
    || async {
        // Publica un mensaje en un servicio upstream. Los errores de
        // red son transitorios - el siguiente reintento debería
        // volver a entrar, no que se le diga "ya hecho" cuando en
        // realidad no pasó nada.
        social_media_client.post(&post).await
    },
)
.await?;
```

Usa `commit_on_success` cuando el cuerpo tiene modos de fallo
reintentables (errores de red transitorios, límites de velocidad
upstream, credenciales expiradas que un refresco arreglaría) y
quieres al menos una vez en éxito, pero que el bloqueo se entregue
ante un fallo para que un reintento pueda volver a entrar.

### `Idempotency::remember` - reproducción de resultado al estilo Stripe

El contrato para el que se inventó el encabezado HTTP
`Idempotency-Key`. El primer llamador corre el cuerpo, guarda el
valor de éxito, y obtiene `Replay::Fresh`. Un llamador posterior
dentro de la ventana obtiene `Replay::Replayed(<valor original>)` -
el valor de retorno registrado, no un marcador. Un llamador
concurrente que llega *mientras* el primero todavía está corriendo
obtiene `Replay::InProgress`.

```rust
use std::time::Duration;
use suprnova::{
    handler, Auth, FrameworkError, HttpResponse, Idempotency, Replay, Request, Response,
};

#[handler]
pub async fn create_charge(req: Request) -> Response {
    // Extrae el encabezado a un String propio antes de consumir `req` para el cuerpo.
    let key = req
        .header("Idempotency-Key")
        .ok_or_else(|| FrameworkError::bad_request("Idempotency-Key header required"))?
        .to_string();

    let user = Auth::user_as::<User>()
        .await?
        .ok_or_else(|| FrameworkError::unauthorized("login required"))?;

    let form: ChargeForm = req.json().await?;

    let outcome = Idempotency::remember(
        &format!("charge:{}:{}", user.id, key),
        Duration::from_secs(24 * 3600),
        || async {
            let charge = StripeClient::charge(&form).await?;
            Ok(ChargeResponse {
                id: charge.id,
                amount: charge.amount,
                status: charge.status,
            })
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(body) | Replay::Replayed(body) => {
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::FreshUnfenced(body) => {
            // La misma respuesta para el cliente, pero merece una
            // métrica: la exclusividad no se sostuvo durante todo el cuerpo.
            tracing::warn!("idempotent body completed unfenced");
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

Nota que `Fresh` y `Replayed` se manejan de forma idéntica en la
respuesta de cara al cliente - el objetivo entero de `remember` es
que el segundo llamador no pueda distinguir si fue quien corrió el
cuerpo o si recibió el resultado registrado.

`InProgress` es el caso que vale la pena pensar: un duplicado llegó
mientras el cuerpo del primer llamador todavía se estaba ejecutando,
así que aún no hay ningún resultado registrado que devolver. Un `409
Conflict` con un encabezado `Retry-After: 1` es la respuesta
canónica - el cliente espera brevemente, luego reintenta, y el
segundo intento o compite con el original por el cortocircuito de
`Cache::get`, o golpea `Replayed`.

## Material de la clave

Los tres métodos aceptan un `&str` arbitrario como clave. Antes de
que toque el backend de la caché, la clave se hashea con SHA-256 en
un resumen hexadecimal de 64 caracteres. Esto te compra tres cosas:

1. **Longitud acotada de la clave en el backend.** Un cliente que
   hace POST con un encabezado `Idempotency-Key` de 10 KB de todos
   modos produce una clave de caché de 64 bytes.
2. **Los identificadores en crudo no se filtran a las herramientas
   de la caché.** Si la clave contiene una dirección de correo, un
   id de sesión, o un id de usuario interno, esos no aparecen en
   `redis-cli KEYS idem:*`.
3. **Sin colisiones de clase de carácter.** Lo que sea que el backend
   de la caché interprete de forma especial (dos puntos, caracteres
   glob, bytes de control) ya desapareció - el hash es solo
   hexadecimal.

El hash se aplica sobre la clave suministrada por el usuario, no
sobre el prefijo de la clave de caché - `Idempotency::once("k", …)` y
`Idempotency::once("k", …)` desde dos sitios de llamada distintos en
el mismo proceso colisionan a propósito. Pon tus propias claves en su
propio namespace si no quieres eso:

```rust
Idempotency::once(
    &format!("billing:charge:{}:{}", tenant_id, client_key),
    Duration::from_secs(86_400),
    || async { /* … */ },
)
.await?;
```

## Renovación del lease - el problema del cuerpo lento

Una combinación naíf de bloqueo + TTL tiene un bug de ventana: si el
cuerpo corre más tiempo que el TTL, el bloqueo expira mientras el
cuerpo todavía está corriendo, y un segundo llamador puede adquirir
un bloqueo nuevo y correr el cuerpo otra vez de forma concurrente. El
contrato de deduplicación se rompe exactamente para las operaciones
lo bastante lentas como para necesitarlo.

Suprnova resuelve esto lanzando una tarea en segundo plano que
refresca el bloqueo a un tercio del TTL (con un piso de 50 ms)
durante toda la duración del cuerpo. Un `tokio::select!` con orden
`biased` garantiza que la rama del cuerpo es la única que jamás
resuelve el future.

Un *error* de refresco no se trata como un lease perdido. Significa
que no se pudo consultar al backend, no que otro se llevó el
bloqueo, así que la renovación reintenta en el siguiente intervalo y
solo se rinde después de varios fallos consecutivos seguidos.
Abandonar ante el primer contratiempo garantizaba que el lease
caducara incluso cuando el backend se recuperaba milisegundos
después.

### Cuando se pierde la exclusividad

La renovación todavía puede fallar de verdad: el token deja de
coincidir, porque el bloqueo expiró y otro lo reclamó. En ese
momento, dos llamadores pueden estar corriendo el mismo cuerpo.

El cuerpo **no** se cancela. Para el momento en que se pierde un
lease, puede que ya haya cobrado una tarjeta o enviado un mensaje, y
cancelarlo dejaría eso varado a medias sin nada que lo registre. El
cuerpo corre hasta completarse y la pérdida se reporta:

| Resultado | Significa |
|---|---|
| `Fresh(v)` / `Replay::Fresh(v)` | el cuerpo corrió, la exclusividad se sostuvo todo el tiempo |
| `FreshUnfenced(v)` | el cuerpo corrió y produjo `v`, pero otro llamador puede haber corrido de forma concurrente |

`FreshUnfenced` es una variante separada en lugar de un flag sobre
`Fresh`, específicamente para que un `match` exhaustivo no pueda
ignorarlo por accidente. Qué hacer con esto es decisión tuya -
reconciliar, alertar, compensar - pero tratarlo como `Fresh`
desperdicia la única señal que tienes de que la garantía no se
sostuvo.

Perder un lease requiere que el backend sea inalcanzable durante
varios intervalos de refresco, o una pausa stop-the-world más larga
que el TTL. Es raro. No es imposible, y antes era invisible.

La conclusión práctica: elige un TTL según tu ventana de
deduplicación (`¿cuánto tiempo debería deduplicarse una solicitud
duplicada?`), no según la duración de tu cuerpo en el peor caso. Un
cuerpo de 30 minutos con un TTL de 1 minuto está bien - el bloqueo se
refrescará unas noventa veces durante la ejecución del cuerpo.

Un test que ejercita esto: un TTL de 200 ms con un cuerpo que bloquea
durante 500 ms, y un segundo llamador que llega a los 400 ms. Sin
renovación, el segundo llamador reejecutaría el cuerpo. Con
renovación, ve `Duplicate`. El bloqueo se sostiene.

## Backend compartido

La deduplicación entre procesos requiere una caché entre procesos. El
backend en memoria guarda los bloqueos en un `HashMap` por proceso,
así que dos instancias de `cargo run` en la misma máquina no verán
las claves de idempotencia de la otra. Los despliegues de producción
donde algo de esto importa - varios procesos de app, escalado
horizontal, despliegues blue/green con ventanas de tráfico
superpuestas - deben establecer `CACHE_DRIVER=redis` y proveer una
`REDIS_URL` alcanzable.

El arranque es fail-closed: si `CACHE_DRIVER=redis` y Redis es
inalcanzable, la app se niega a arrancar en lugar de degradar en
silencio a memoria por proceso. Ver [cache.md](cache.md) para el
contrato completo del backend de caché.

## Manejo de errores

El `FrameworkError` del cuerpo se propaga hacia arriba a través de
`Idempotency` sin cambios. Un fallo de adquisición del bloqueo (Redis
está caído a mitad de solicitud, el backend devuelve un error) se
propaga como un `FrameworkError` desde la capa de caché - no hay
ningún fallback silencioso. El tipo de error es el `FrameworkError`
estándar del framework, así que los handlers pueden propagarlo con
`?` hasta el conversor de errores de su controlador:

```rust
use std::time::Duration;
use suprnova::{handler, FrameworkError, HttpResponse, Idempotency, Replay, Response};

#[handler]
pub async fn handler(order_id: i64) -> Response {
    let outcome: Replay<MyDto> = Idempotency::remember(
        &format!("order:{order_id}"),
        Duration::from_secs(60),
        || async move {
            let row = MyRow::find(order_id)
                .await?
                .ok_or_else(|| FrameworkError::not_found("missing"))?;
            Ok(MyDto::from(row))
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(dto) | Replay::Replayed(dto) | Replay::FreshUnfenced(dto) => {
            let json = serde_json::to_value(&dto)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

Un fallo de liberación en la ruta `Err` de `commit_on_success` o
`remember` se **registra, nunca se devuelve** - el error del cuerpo
es el único error que ve el llamador en esa ruta. Un fallo de
liberación significa que el bloqueo se sostendrá hasta que el TTL
caduque; un reintento dentro de la ventana verá `Duplicate` o
`InProgress` hasta entonces. Los logs incluyen la clave hasheada
(nunca el material de la clave en crudo) para que los operadores
puedan correlacionar sin filtrar PII.

## Cancelación

Si quien llama abandona el future de `Idempotency::remember` antes de
que el cuerpo complete, el cuerpo se cancela como cualquier otra rama
de `tokio::select!` - el bloqueo **no** se libera, y un duplicado que
llegue antes de que el TTL caduque ve `InProgress` (luego, tras el
TTL, `Fresh` otra vez). Este es el valor por defecto seguro: un
cuerpo a medio terminar cuyos efectos no conoces no debería
presumirse seguro para reintentar. Envuelve los cuerpos que sostienen
efectos secundarios no gestionados en `tokio::spawn` y haz join sobre
el handle si necesitas que el cuerpo sea no cancelable.

## Integración con la cola

La capa de cola usa `Idempotency::commit_on_success` internamente
para implementar `Queue::push_unique`. Si quieres que un job se
encole como mucho una vez por ventana de `Job::unique_for()` por
`Job::unique_id(&self)`, no necesitas llamar a `Idempotency::*` tú
mismo:

```rust
use suprnova::{Job, Queue};

let was_pushed = Queue::push_unique(SendReceipt { order_id: 42 }).await?;
if was_pushed {
    // Ganamos la carrera; el job está en la cola.
} else {
    // Otro llamador ya encoló esto; trátalo como éxito.
}
```

Ver [queues.md](queues.md) para el contrato completo de unicidad de
jobs.

## Ingesta de webhooks de pago

El handler de webhooks de pagos NO usa `Idempotency::*`. La ingesta
de webhooks tiene un requisito más estricto - cada evento debe ser
auditable, incluso en la primera entrega, así que la fila de
auditoría es la fuente de verdad y la clave de deduplicación es la
restricción `UNIQUE(provider, provider_event_id)` de la base de
datos. `Idempotency::remember` guardaría el payload de la respuesta
en la caché; el handler de webhooks guarda el *sobre completo del
evento más el resultado del procesamiento* en
`payments_webhook_events`, lo que significa que un operador puede
reproducir o volver a procesar eventos sin conexión leyendo la
tabla.

Los dos patrones son complementarios. Usa `Idempotency::*` para
claves impulsadas por el cliente con deduplicación acotada por TTL;
usa una tabla de auditoría indexada por `UNIQUE` para la ingesta de
webhooks impulsada por el proveedor que necesita auditabilidad más
allá del TTL de la caché. Ver [payments.md](payments.md) para el
contrato de webhooks.

### Por qué Suprnova diverge

`Cache::lock` de Laravel es una primitiva; el contrato de idempotencia
al estilo Stripe (registra el resultado, reprodúcelo, distingue en
curso de duplicado) se deja como una receta de espacio de usuario.
Cada proyecto de Laravel que lo necesita termina escribiendo el mismo
baile de bloqueo-y-caché, casi siempre con uno de estos tres bugs:

1. **Sin renovación de lease.** Un cuerpo que sobrevive al TTL se
   reejecuta de forma concurrente en un llamador duplicado. El
   bloqueo estaba ahí; simplemente expiró en el momento equivocado.
2. **Liberación en la ruta de éxito.** Liberar el bloqueo cuando el
   cuerpo tiene éxito abre una ventana entre `body() -> Ok` y el
   siguiente llamador adquiriendo un bloqueo nuevo - justo la
   ventana que la deduplicación debía cerrar.
3. **Claves en crudo en el backend de caché.** Los encabezados
   `Idempotency-Key` suministrados por el cliente van directo a las
   claves de Redis, filtrando PII a las herramientas del operador y
   produciendo tamaños de clave sin acotar.

Suprnova ofrece la receta como una primitiva de primera clase, así
que cada llamador obtiene la misma renovación de lease, la misma
semántica de liberación fail-closed, la misma seguridad de clave
hasheada. Los tres métodos (`once`, `commit_on_success`, `remember`)
nombran las tres políticas entre las que de verdad tienes que elegir -
elige la que coincida con el modelo de fallo de tu cuerpo y sigue
adelante.

## Pruebas

`Idempotency` resuelve su `CacheStore` a través del contenedor, así
que los tests que vinculan un `InMemoryCache` obtienen una caché
nueva y aislada por test:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::cache::InMemoryCache;
use suprnova::cache::store::CacheStore;
use suprnova::container::testing::TestContainer;
use suprnova::idempotency::{Idempotency, Replay};

#[tokio::test]
async fn duplicate_remember_replays_the_first_result() {
    let _guard = TestContainer::fake();
    let store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_prefix("idem:"));
    TestContainer::bind::<dyn CacheStore>(store);

    let r1: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(7) },
    )
    .await
    .unwrap();
    assert_eq!(r1, Replay::Fresh(7));

    let r2: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(999) },
    )
    .await
    .unwrap();
    assert_eq!(r2, Replay::Replayed(7));
}
```

`framework/tests/idempotency.rs`, del propio framework, cubre la
superficie del contrato: supresión de duplicados, expiración del
TTL, política de liberación éxito-versus-fallo, renovación de lease
a través de duraciones de cuerpo que sobreviven al TTL, la carrera de
`InProgress`, y el caso en que el propio `release_lock` de la caché
falla. Lee esos tests si quieres ver el comportamiento exacto con el
que puedes contar.

## Trampas

- **`Idempotency::once` consume la ventana en un error.** Un primer
  llamador que falla de todos modos sostiene el bloqueo hasta que el
  TTL caduque. Usa `commit_on_success` si quieres reintentos dentro
  de la ventana.
- **`Idempotency::remember` guarda `T` en el backend de la caché.**
  La clave se hashea, pero el *payload* se serializa con serde y se
  escribe en el backend. No pongas secretos en un valor reproducido
  que no deba aparecer en tu store de caché.
- **Dos procesos necesitan una caché compartida.** La deduplicación
  en memoria es por proceso. La corrección entre procesos exige
  `CACHE_DRIVER=redis` (u otro store entre procesos).
- **Los TTLs por debajo de 150 ms no están probados con lease.** El
  piso de renovación es 50 ms, así que un TTL de 100 ms se refresca
  cada 50 ms aproximadamente - bien para el contrato, pero los tests
  de lease del framework corren con `ttl >= 1s`. Usa ventanas de
  deduplicación realistas; una ventana de idempotencia medida en
  milisegundos normalmente significa que el contrato no es
  exactamente la herramienta correcta.
- **La cancelación del cuerpo no libera el bloqueo.** Un cuerpo
  cancelado deja el bloqueo sosteniéndose hasta que el TTL caduque.
  Esta es la elección fail-closed; organiza tus timeouts para que la
  cancelación coincida con lo que un llamador duplicado debería ver.

## Siguiente

- [cache.md](cache.md) - la primitiva de bloqueo subyacente y la
  selección de `CACHE_DRIVER`.
- [queues.md](queues.md) - cómo `Queue::push_unique` se construye
  sobre `Idempotency::commit_on_success` para deduplicación a nivel
  de job.
- [payments.md](payments.md) - ingesta de webhooks que usa
  idempotencia por fila de base de datos en lugar de deduplicación
  indexada por caché, y cuándo recurrir a cuál.
- [rate-limiting.md](rate-limiting.md) - middleware adyacente que usa
  el mismo backend `Cache` para la aplicación de ventana deslizante.
- [middleware.md](middleware.md) - cómo factorizar la extracción de
  la clave de idempotencia en un middleware reutilizable sobre tus
  rutas POST/PUT.

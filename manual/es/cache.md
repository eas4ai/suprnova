# Caché

Suprnova incluye una fachada `Cache` con forma de Laravel respaldada
por uno de dos drivers - en memoria o Redis - elegido explícitamente en
el arranque vía `CACHE_DRIVER`. La fachada es una capa delgada sobre un
trait `CacheStore`, así que los backends personalizados se conectan de
la misma forma que lo hacen los integrados.

## La fachada

```rust
use suprnova::Cache;
use std::time::Duration;

Cache::put("user:1", &user, Some(Duration::from_secs(3600))).await?;

let cached: Option<User> = Cache::get("user:1").await?;

if Cache::has("user:1").await? {
    // acierto
}

Cache::forget("user:1").await?;
```

Cada método serializa a través de `serde_json` en el límite de la
fachada, así que cualquier `T: Serialize + DeserializeOwned` hace el
viaje de ida y vuelta. El trait bajo la fachada (`CacheStore`) solo ve
strings JSON opacos.

## Arranque de la aplicación

La caché se vincula durante el paso de arranque de drivers de
`Server::run()` (consulta [Ciclo de vida de la solicitud](lifecycle.md)).
`Cache::bootstrap` lee el `CacheConfig` configurado (o construye uno
desde el entorno) y despacha según `CacheConfig::driver`:

- `Memory` - vincula un `InMemoryCache` con el prefijo configurado y el
  TTL por defecto. Siempre tiene éxito.
- `Redis` - se conecta a `REDIS_URL` y vincula el `RedisCache`
  resultante. **Falla en cerrado** si la URL es inalcanzable. No hay
  ninguna degradación silenciosa a memoria.

Los workers (`queue:work`, `schedule:run`, `workflow:work`) pasan por
el mismo arranque, así que un job que usa `Cache::get` ve el mismo
backend que ve el handler HTTP.

### Por qué Suprnova diverge

El `cache.php` de Laravel elige un store por defecto y Laravel cambia
silenciosamente a `array` (en proceso) cuando un backend mal
configurado falla en algunos caminos de código. Ese es un valor por
defecto productivo para `php artisan tinker` y una trampa en
producción - un solo fallo de Redis cambia silenciosamente las
garantías de cada vaciado de etiquetas y cada adquisición de bloqueo en
la app.

Suprnova elige el valor por defecto opuesto. `CACHE_DRIVER=memory` es
explícito (y el valor por defecto para `cargo run`), y
`CACHE_DRIVER=redis` contra un Redis inalcanzable devuelve un error
desde `Server::from_config`. El binario termina con un código de salida
distinto de cero junto con un mensaje de remediación; supervisord/systemd
ve un fallo de arranque en lugar de una app medio funcional.

## Configuración

| Env | Significado | Por defecto |
|---|---|---|
| `CACHE_DRIVER` | `memory` o `redis` | `memory` |
| `REDIS_URL` | URL de Redis (consultada solo cuando `driver=redis`) | `redis://127.0.0.1:6379` |
| `REDIS_PREFIX` | Prefijo de clave aplicado a cada operación del store | `suprnova_cache:` |
| `CACHE_DEFAULT_TTL` | TTL por defecto en segundos para `Cache::put(None)`; `0` significa sin valor por defecto | `3600` |

`CACHE_DRIVER` sin establecer se analiza como `Memory`; cualquier otro
valor (sin distinguir mayúsculas de minúsculas, recortado) que no sea
`memory`/`in-memory`/`inmemory`/`redis` devuelve un error en el
arranque.

También puedes construir la config programáticamente cuando no quieras
análisis de entorno:

```rust
use suprnova::{Config, CacheConfig, cache::CacheDriver};

Config::register(
    CacheConfig::builder()
        .driver(CacheDriver::Redis)
        .url("redis://cache.internal:6379")
        .prefix("myapp:")
        .default_ttl(7200)
        .build(),
);
```

`CacheConfigBuilder::build` es determinista - los campos sin establecer
recurren a `CacheConfig::default()` en lugar de volver a leer el
entorno.

### El contrato de `forever` se mantiene entre backends

`Cache::forever` y `Cache::remember_forever` pasan por alto
`CACHE_DEFAULT_TTL` por completo; el valor nunca expira sin importar el
valor por defecto configurado. `Cache::put(key, value, None)` sí aplica
el valor por defecto - ese es el sentido de tener uno.

La resolución del TTL por defecto ocurre en la capa de la fachada.
Ambos backends de `CacheStore` honran `None` literalmente en el límite
del store (sin expiración), que es la razón por la que `forever`
realmente significa para siempre tanto en memoria como en Redis.

## Lecturas, escrituras, eliminaciones

```rust
use suprnova::Cache;
use std::time::Duration;

// Escritura con un TTL explícito
Cache::put("session:42", &session, Some(Duration::from_secs(1800))).await?;

// Escritura para siempre - pasa por alto CACHE_DEFAULT_TTL
Cache::forever("config:features", &features).await?;

// Lectura (None en caso de fallo o expiración)
let session: Option<Session> = Cache::get("session:42").await?;

// Existencia - true significa presente y no expirado
if Cache::has("session:42").await? { /* … */ }

// Negación con forma de Laravel
if Cache::missing("session:42").await? { /* precalentar */ }

// Leer y eliminar en una sola llamada
let one_shot: Option<String> = Cache::pull("notice:welcome:42").await?;

// Devuelve true si la clave existía y fue eliminada
Cache::forget("session:42").await?;

// Vaciar todo (con alcance de prefijo en ambos backends)
Cache::flush().await?;
```

`Cache::pull` **no** es atómico - es un `get` seguido de un `forget`,
la misma forma que el `Repository::pull` de Laravel. Para desencolar de
forma atómica usa `Cache::lock` (ver más abajo).

### Refrescar un TTL sin reescribir

```rust
let refreshed = Cache::touch("session:42", Duration::from_secs(1800)).await?;
```

`touch` devuelve `true` si la clave existía y el TTL se extendió,
`false` en caso contrario. El valor almacenado queda intacto.

## Add - escritura solo si está ausente (atómica)

```rust
let won = Cache::add(
    "daily:winner",
    &user_id,
    Some(Duration::from_secs(86_400)),
).await?;
if won {
    send_winner_email(user_id).await?;
}
```

`Cache::add` escribe solo si la clave está vacía (o ha expirado).
Devuelve `true` si escribió, `false` en caso de contención.
**Atómico** en ambos backends integrados:

- `InMemoryCache` mantiene un bloqueo de escritura a lo largo de la
  comprobación de existencia + la inserción
- `RedisCache` usa `SET key value NX EX ttl` (o `NX` sin `EX`)

Las implementaciones personalizadas de `CacheStore` que no sobrescriben
`add_raw` recurren a una comprobación-y-escritura no atómica, igual que
el mecanismo de respaldo de `Repository::add` de Laravel para stores
sin un `add` nativo.

## Remember - obtener o calcular

```rust
let user = Cache::remember(
    "user:1",
    Some(Duration::from_secs(3600)),
    || async { User::find(1).await },
).await?;

let cfg = Cache::remember_forever("config:app", || async {
    load_config_from_db().await
}).await?;
```

`remember` llama a tu closure solo en caso de fallo, y luego almacena
el resultado. El closure devuelve `Result<T, FrameworkError>`, así que
los fallos de dominio burbujean a través de `?` en lugar de envenenar
la caché.

`Cache::sear(key, default)` es el alias con forma de Laravel para
`remember_forever`. Mismo cuerpo, misma semántica - se envía bajo
ambos nombres para que el código migrado se lea de la misma manera.

### Remember NO es seguro ante una estampida

`remember` es un par `get`-luego-`put` no atómico. N fallos
concurrentes para la misma clave fría ejecutan el closure N veces y
escriben N resultados. Eso coincide exactamente con el
`Repository::remember` de Laravel, y está bien para el caso común (el
closure es idempotente, las escrituras son idénticas).

No está bien cuando:

- El closure es costoso (1s o más para calcularse, o golpea un
  upstream lento)
- La clave es lo bastante popular como para que un evento de caché fría
  envíe N solicitudes a la vez al store de respaldo
- El closure tiene efectos secundarios más allá de calcular el valor

Para esos casos, envuelve con `Cache::lock`:

```rust
use suprnova::Cache;
use std::time::Duration;

let key = "rebuild:user:1";

if let Some(guard) = Cache::lock(key, Duration::from_secs(10)).await? {
    let user = Cache::remember(
        "user:1",
        Some(Duration::from_secs(3600)),
        || async { User::find(1).await },
    ).await?;
    guard.release().await?;
    return Ok(user);
}

// Perdimos la carrera - el ganador está calculando. Lee lo que sea que
// haya escrito, o recurre a un valor desactualizado.
let user = Cache::get::<User>("user:1").await?
    .ok_or_else(|| FrameworkError::internal("cache miss after losing rebuild lock"))?;
```

## Bloqueos

`Cache::lock` devuelve un `LockGuard` que contiene el token de
propiedad. Los bloqueos son orientativos y cruzan procesos cuando están
respaldados por Redis.

```rust
use suprnova::Cache;
use std::time::Duration;

if let Some(guard) = Cache::lock("job:42", Duration::from_secs(30)).await? {
    do_exclusive_work().await?;
    guard.release().await?;
}
// `Some(guard)` significa que lo poseemos. `None` significa que otro titular nos ganó.
```

La guarda expone:

| Método | Úsalo para |
|---|---|
| `guard.token()` | Leer el token de propiedad (nombre del lado de Rust) |
| `guard.owner()` | El mismo valor, alias con forma de Laravel |
| `guard.refresh(ttl)` | Extender el TTL - devuelve `false` si ya no poseemos el bloqueo |
| `guard.release()` | Liberar si todavía poseemos el bloqueo - devuelve `false` si el token ya no coincide |

Intencionalmente **no hay auto-liberación por `Drop`**. Un bloqueo de
Redis debe reconocerse a través de límites de proceso; la
auto-liberación al descartarse o bien robaría en silencio un bloqueo
robado de vuelta (incorrecto) u ocultaría fallos de liberación dentro
de pánicos del destructor (peor). La liberación es explícita para que
los errores se propaguen.

`refresh` permite que un job de larga duración extienda su propio
bloqueo para evitar un timeout autoinfligido - consulta
[Idempotencia](idempotency.md) para el consumidor que ya está en el
árbol.

## Contadores atómicos

```rust
// Se inicializa a 0 si está ausente, luego incrementa. Devuelve el valor nuevo.
let visits = Cache::increment("page:visits", 1).await?;

// Misma forma para pasos negativos
let remaining = Cache::decrement("quota:remaining", 1).await?;

// Cantidad personalizada
let total = Cache::increment("stats:downloads", 10).await?;
```

Atómico en ambos backends integrados: `InMemoryCache` usa un
`HashMap::entry` con bloqueo de escritura; `RedisCache` usa
`INCRBY`/`DECRBY`. El valor almacenado es un entero codificado en JSON,
así que `Cache::get::<i64>("page:visits")` hace el viaje de ida y
vuelta con la misma clave.

## Caché etiquetada

Las etiquetas te permiten invalidar toda una familia de entradas
relacionadas con una sola llamada. El caso de uso clásico son las
cachés por recurso que tienen que vaciarse juntas cuando el recurso
cambia.

```rust
use suprnova::Cache;
use std::time::Duration;

// Almacenar bajo una o más etiquetas
Cache::tags_put(
    &["users", "user:1"],
    "user:1:profile",
    &profile,
    Some(Duration::from_secs(3600)),
).await?;

Cache::tags_put(
    &["users", "user:1"],
    "user:1:posts",
    &posts,
    Some(Duration::from_secs(600)),
).await?;

// Camino de actualización: elimina cada clave etiquetada `user:1`
Cache::flush_tags(&["user:1"]).await?;
```

La pertenencia a una etiqueta es **por entrada**: cada escritura
etiquetada instala el conjunto de etiquetas de esa escritura como la
fuente de verdad de la entrada, reemplazando cualquier etiqueta previa.
Dos consecuencias que vale la pena conocer:

- Un `Cache::put` sin etiquetar sobre una clave previamente etiquetada
  **limpia** las etiquetas de la entrada. Un `flush_tags` posterior de
  la etiqueta antigua no eliminará el valor sin etiquetar que sigue
  vivo.
- Sobrescribir `tags_put(&["a"], …)` con `tags_put(&["b"], …)` hace que
  la entrada solo responda a `flush_tags(&["b"])`.

Las referencias de índice inverso desactualizadas se purgan durante el
recorrido de vaciado y en `flush()`, así que no se acumulan
indefinidamente para las etiquetas que se escriben pero nunca se
vacían.

## Dos backends

| Característica | `InMemoryCache` | `RedisCache` |
|---|---|---|
| Compartido entre procesos | No | Sí |
| Persistencia | No | Sí, si Redis está configurado para ello |
| `add` atómico | Sí (bloqueo de escritura) | Sí (`SET NX`) |
| `increment`/`decrement` atómico | Sí (bloqueo de escritura) | Sí (`INCRBY`/`DECRBY`) |
| Caché etiquetada | Sí | Sí |
| Bloqueos | Sí | Sí (entre procesos) |
| TTL de menos de un segundo | Sí (`tokio::time::Instant`) | Sí (`PX`/`PEXPIRE`) |
| Se selecciona vía | `CACHE_DRIVER=memory` (por defecto) | `CACHE_DRIVER=redis` |

No hay ningún driver de caché de base de datos - los dos backends de
arriba son los que el framework incluye. Los backends personalizados
pueden implementar `CacheStore` y vincularse en el contenedor
directamente; consulta el patrón de inyección para tests más abajo.

### Expiración en memoria

`InMemoryCache` desaloja las entradas expiradas **de forma perezosa en
la lectura**: `get_raw`, `has`, y `add_raw` purgan una entrada la
primera vez que la observan expirada. Las claves reaccedidas nunca
acumulan entradas muertas.

Una carga de trabajo que escribe un conjunto de claves de cardinalidad
alta y de vida corta y nunca las vuelve a leer no tiene ese disparador.
Llama a `InMemoryCache::purge_expired()` desde una tarea periódica en
ese caso - devuelve el número de entradas eliminadas. Redis maneja su
propia expiración del lado del servidor; el equivalente no hace falta
ahí.

### Precisión del TTL de Redis

Cada TTL de Redis pasa por `PX` / `PEXPIRE`, no por `EX` / `EXPIRE`.
Eso evita dos trampas:

- Los `Duration` de menos de un segundo truncarían a `0 segundos` bajo
  `EX`, algo que Redis rechaza (`SET … EX 0`) o, peor aún, interpreta
  como "elimina la clave" (`EXPIRE key 0`).
- `Duration::ZERO` se acota a 1 ms antes de la llamada, así que ninguno
  de los dos caminos de rechazo es alcanzable desde el código del
  usuario.

### Reintentos de comandos ante fallos transitorios

Un socket caído hacía fallar cualquier `Cache::get` que estuviera en curso.
El gestor de conexiones de Redis se reconecta por su cuenta, pero el
comando que dio con el socket muerto te sigue devolviendo su error.

Los comandos con forma de lectura ahora reintentan una vez: `GET`,
`EXISTS` y las páginas de `SCAN` / `SSCAN` que hay detrás de `Cache::flush`
y `Cache::flush_tags`. Las lecturas `XLEN`, `ZCARD` y `XPENDING` del driver
de colas y el cálculo del `Retry-After` del limitador de velocidad
reintentan igual. Fija `REDIS_COMMAND_RETRIES` para añadir más reintentos
por encima del que ya viene incorporado.

Presupuesta el reintento en segundos, no en la pausa de 50 ms que lo
precede. Una vez que una conexión se ha caído, el siguiente intento espera
a la conexión de reemplazo antes de poder enviar nada, así que paga el
presupuesto de conexión entero del driver y después su tiempo de espera de
respuesta:

- El driver de caché permite hasta 3 reintentos de conexión, separados como
  mucho por 500 ms, cada uno acotado por un tiempo de espera de conexión de
  2 s, con un tiempo de espera de respuesta de 5 s.
- Los drivers de colas y de limitación de velocidad toman los valores por
  defecto de redis-rs: hasta 6 reintentos de conexión con un retardo
  exponencial sin tope que arranca en 100 ms, cada uno acotado por un
  tiempo de espera de conexión de 1 s, con un tiempo de espera de respuesta
  de 500 ms.

`REDIS_COMMAND_RETRIES` se acota en 10, y ese tope limita los intentos, no
los segundos: en el máximo, una sola lectura hace 12 intentos, lo que
contra un Redis caído son decenas de segundos o minutos en una única
llamada. Un comando cuyo tiempo de espera se agota cuenta como transitorio
igual que uno caído, así que un Redis simplemente lento hace que cada
lectura envuelta emita hasta esa cantidad de comandos en lugar de uno. Sube
el ajuste solo donde quien llama se pueda permitir la espera.

Las escrituras nunca reintentan, con ningún ajuste. Un error transitorio
significa que la conexión falló, no que el servidor rechazara el comando -
puede que el servidor ya lo haya ejecutado -, así que reintentar un `SET`,
un `INCR`, la adquisición de un bloqueo, un impacto contra el limitador de
velocidad o sacar un job de la cola corre el riesgo de ejecutarlo dos
veces. Esos comandos te devuelven el fallo, y así la decisión de reintentar
la toma quien tiene la información.

### Por qué Suprnova diverge

La configuración `command_retries` de Laravel eleva el presupuesto de
reintentos de todos los comandos de Redis, porque su método `command()` es
un único punto de paso obligado que sabe qué comando está ejecutando y
consulta una lista de permitidos de solo lectura con 60 entradas. Los
drivers de Suprnova llaman directamente a comandos tipados, así que la
lista de permitidos pasa a ser una decisión de cada punto de llamada, y
`REDIS_COMMAND_RETRIES` solo puede profundizar los reintentos de los
comandos que ya son seguros de repetir. No hay ningún ajuste que haga que
se reintente sacar un job de la cola.

## Pruebas

Vincula un `InMemoryCache` en el `TestContainer` y la fachada lo
resuelve como cualquier otro store:

```rust
use std::sync::Arc;
use suprnova::{Cache, CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;

#[tokio::test]
async fn cache_round_trips() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    Cache::put("k", &"v", None).await.unwrap();

    let v: Option<String> = Cache::get("k").await.unwrap();
    assert_eq!(v.as_deref(), Some("v"));
}
```

`TestContainer::bind` escribe en el alcance thread-local, así que los
tests en paralelo no filtran estado de caché entre sí. Consulta el
capítulo [Contenedor de servicios](container.md) para el modelo de
búsqueda en tres capas.

### Suites contra un Redis real

Los propios tests de Redis del framework llevan `#[ignore]`, así que
`cargo test` nunca necesita un servidor. Ejecútalos con `-- --ignored` y
apúntalos a una instancia:

- `cache_redis_integration` lee `CACHE_REDIS_TEST_URL`, y recurre a
  `REDIS_URL` y luego a `redis://127.0.0.1:6379`. Cada test se acota a sí
  mismo a un prefijo de clave único, así que es seguro contra un Redis de
  desarrollo compartido.
- `cache_redis_retry` cubre el reintento de comandos ante fallos
  transitorios y exige `CACHE_REDIS_TEST_URL` de forma explícita, sin
  recurso alternativo. Emite `CLIENT KILL TYPE normal`, que desconecta a
  todos los demás clientes de la instancia, así que hay que darle un
  servidor desechable. Con la variable sin definir imprime una línea de
  omisión y pasa sin conectarse.

## Patrones

Algunas formas recurrentes que vale la pena nombrar:

```rust
// Claves jerárquicas separadas por dos puntos - la misma convención que usa Laravel
Cache::put("users:1:profile", &profile, None).await?;
Cache::put("posts:123:comments:count", &count, None).await?;

// TTL según la volatilidad del dato
Cache::put("stats:active", &count, Some(Duration::from_secs(60))).await?;
Cache::put("config:features", &features, Some(Duration::from_secs(3600))).await?;
Cache::forever("translations:en", &translations).await?;

// Invalidación por etiqueta alrededor de una escritura
async fn update_user(id: i64, data: UserUpdate) -> Result<User, FrameworkError> {
    let user = User::update(id, data).await?;
    Cache::flush_tags(&[&format!("user:{}", id)]).await?;
    Ok(user)
}
```

## Siguiente

- [Configuración](configuration.md) - cómo se combinan `Config::register` y las variables de entorno
- [Limitación de velocidad](rate-limiting.md) - la fachada `RateLimiter` con forma de Laravel está construida sobre `Cache`
- [Idempotencia](idempotency.md) - el middleware de deduplicación de solicitudes usa `Cache::lock` de punta a punta
- [Contenedor de servicios](container.md) - cómo se vincula y se resuelve `CacheStore`
- [Modelo de errores](error-model.md) - qué devuelve `Cache::*` cuando Redis está inalcanzable a mitad de la solicitud

# Limitación de velocidad

Suprnova ofrece dos superficies de límite de velocidad
complementarias:

| Superficie | Úsala cuando... | Backend |
|---------|-------------|---------|
| `RateLimiterDriver` + `RateLimitMiddleware` | Quieres una aplicación estricta de ventana deslizante contra almacenamiento arbitrario (Redis ZSET, deque en memoria) | `dyn RateLimiterDriver` |
| `RateLimiter` + `ThrottleRequestsMiddleware` | Quieres limitadores con nombre con forma de Laravel, callbacks del flujo `attempt()`, o encabezados de respuesta `X-RateLimit-*` | store `Cache` (memory o Redis) |

El driver de ventana deslizante es la forma nativa de Suprnova - un
slot por solicitud, sin clave de temporizador separada, evaluación
Lua atómica en Redis. La fachada de Laravel es a lo que recurren las
apps migradas y lo que exige el patrón de limitador con
nombre/callback de respuesta. Ambas coexisten por diseño, y una ruta
puede apilar las dos.

## SPI del driver de ventana deslizante

`RateLimiterDriver` es la SPI de almacenamiento para el algoritmo de
ventana deslizante. Cada clave rastrea un deque de marcas de tiempo
de golpes. En cada `try_acquire`, las entradas más antiguas que
`now - window` se desalojan; si el conteo restante está por debajo de
`max_requests`, se añade `now` y la llamada acepta. En caso contrario
rechaza.

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
let cfg = SlidingWindowConfig {
    max_requests: 60,
    window: Duration::from_secs(60),
};
let ok = limiter.try_acquire("user:42", &cfg).await?;
if !ok {
    let wait = limiter.retry_after("user:42", &cfg).await?;
    // wait es el Option<Duration> hasta que el slot más antiguo del
    // cubo caduque.
}
```

### Drivers integrados

| Driver | Almacenamiento | Se selecciona vía |
|--------|---------|--------------|
| `InMemoryRateLimiter` | `HashMap<String, Bucket>` por proceso con `tokio::time::Instant`, para que los tests con `start_paused` puedan manejar el reloj | `RATE_LIMIT_DRIVER=memory` (por defecto) |
| `RedisRateLimiter` | Redis ZSET + verificación-y-registro atómico con Lua | `RATE_LIMIT_DRIVER=redis` + `RATE_LIMIT_REDIS_URL` |

`bootstrap_from_env()` conecta el driver correspondiente en el
contenedor. Fuera de producción, un valor de driver desconocido recae
en memory con un registro `warn!`.

### Producción falla en cerrado sobre el driver en memoria

En producción, resolver al limitador en memoria es un fallo de
arranque:

```
refusing to boot in production: RATE_LIMIT_DRIVER is unset, which defaults
to the in-memory limiter. Per-process buckets mean every configured quota
is multiplied by your replica count and reset by every deploy...
```

El driver en memoria guarda sus cubos en el heap de un solo proceso.
Detrás de N réplicas, cada una mantiene su propio conteo, así que un
límite de "5 intentos por 15 minutos" para restablecer contraseña es
en realidad 5N, y cada despliegue los reinicia todos a cero. El
límite que configuraste no es el límite que obtienes - y nada lo
indica, porque las solicitudes tienen éxito, que es justo el aspecto
de un limitador que funciona visto desde fuera. Esto emerge como un
incidente de relleno de credenciales o de enumeración de cuentas, no
como un error.

Un valor de driver **no reconocido** falla por la misma razón: recae
en memory. `RATE_LIMIT_DRIVER=Redis` - con mayúscula - advertiría una
sola vez en el arranque y dejaría en silencio un despliegue
multirréplica limitando por proceso. Ese es el caso con más
probabilidades de llegar a producción, porque parece configurado.

O bien apúntalo a Redis:

```env
RATE_LIMIT_DRIVER=redis
RATE_LIMIT_REDIS_URL=redis://cache.internal:6379
```

o, si de verdad ejecutas un único proceso, dilo:

```env
RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true
```

Desarrollo, testing y **staging** quedan intactos. Staging
deliberadamente no está protegido por esta compuerta, con el mismo
razonamiento que la protección de correo: hacerla fallar de forma
estricta empuja a los equipos a establecer la anulación de forma
global, lo que desarma la comprobación justo donde importa.

### `RateLimitMiddleware`

El envoltorio HTTP alrededor del driver. Constrúyelo con un closure
`key_fn` para gobernar la selección de cubo por solicitud:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::container::App;
use suprnova::rate_limit::{
    BackendErrorPolicy, RateLimitMiddleware, RateLimiterDriver, SlidingWindowConfig,
};

let limiter: Arc<dyn RateLimiterDriver> =
    App::resolve_make::<dyn RateLimiterDriver>().unwrap();

let mw = RateLimitMiddleware::new(
    limiter,
    SlidingWindowConfig {
        max_requests: 100,
        window: Duration::from_secs(60),
    },
    |req| format!("route:{}", req.path()),
)
.on_backend_error(BackendErrorPolicy::FailClosed);
```

Al rechazar (por exceder la cuota) devuelve un HTTP 429 con un
encabezado `Retry-After`.

### Limitar por destinatario, no solo por quien llama

Un límite indexado por dirección responde *¿está un cliente haciendo
demasiadas solicitudes?*. No puede responder *¿está un buzón siendo
inundado?*. Un atacante repartido en una botnet, un pool de proxies,
o un único `/64` de IPv6 se mantiene por debajo de cualquier
presupuesto por IP mientras envía miles de correos de restablecimiento
de contraseña a una sola víctima - la bandeja de entrada es el
recurso que se agota, y la dirección de la víctima es lo único que
esas solicitudes comparten. Lo contrario también hace daño: detrás de
un NAT de grado operador o de la puerta de enlace de una oficina, los
límites por IP castigan a toda una multitud por el comportamiento de
un solo miembro.

`identity_key` indexa un cubo sobre la cuenta que está siendo *objeto
de la acción*:

```rust
use suprnova::rate_limit::{identity_key, names_identity};

let per_recipient = RateLimitMiddleware::new(
    limiter.clone(),
    SlidingWindowConfig { max_requests: 3, window: Duration::from_secs(900) },
    |req| identity_key(req, "email", "auth-issuance"),
)
.key_reads_body(4096)
.only_when(|req| names_identity(req, "email"))
.on_backend_error(BackendErrorPolicy::FailClosed);
```

Apílalo *junto a* un limitador por IP, en lugar de sustituir uno por
el otro. Cada uno atrapa lo que el otro no puede: por IP detiene a un
host que enumera muchas direcciones; por destinatario detiene a
muchos hosts que apuntan a una sola dirección.

Tres detalles cargan con la seguridad:

- **`key_reads_body`** almacena el cuerpo en un búfer (hasta el tope
  dado) antes de calcular la clave, para que el campo se pueda leer
  tanto de un POST form-encoded como de un query string. Es opt-in
  porque almacenar en búfer es trabajo que un llamador no autenticado
  te hace hacer; el tope lo acota. Un cuerpo por encima del tope se
  rechaza con 413 en lugar de dejarlo pasar sin clave - de lo
  contrario, rellenar el cuerpo sería una forma de escapar del
  límite.
- **`only_when`** se salta el limitador para solicitudes que no
  nombran a nadie. Sin esto, esas solicitudes caen en el valor de
  reserva de dirección de `identity_key` y se contabilizan contra la
  cuota de *este* limitador - y dado que un presupuesto por
  destinatario suele ser el más estricto del par, se convertiría en
  silencio en el límite vinculante para cada ruta que no nombra a
  nadie.
- **El valor se normaliza y se aplica hash.** `Alice@Example.com` y
  `alice@example.com` llegan al mismo buzón y deben compartir un
  cubo, o el límite se elude cambiando la capitalización. El
  resultado se hashea porque un backend de límite de velocidad suele
  ser un Redis compartido con un control de acceso más débil que la
  base de datos primaria, y un volcado de claves no debería leerse
  como una lista de quién está restableciendo su contraseña.

### Política de error de backend

`BackendErrorPolicy` gobierna qué pasa cuando el *backend* mismo del
limitador falla - por ejemplo, Redis es inalcanzable - a diferencia
de una solicitud que legítimamente excede su cuota. El backend no
puede tomar una decisión, así que el middleware debe elegir entre
disponibilidad y la garantía del límite.

| Política | Comportamiento | Cuándo usarla |
|--------|-----------|-------------|
| `FailOpen` (por defecto) | Deja pasar la solicitud; registra a nivel `warn` | La mayoría de las APIs públicas - una caída del limitador no debería derribar el tráfico |
| `FailClosed` | Rechaza con HTTP 503 + `Retry-After: 1`; registra a nivel `error` | Rutas sensibles (login, restablecimiento de contraseña, pagos) donde el tráfico sin límite durante una caída de backend es peor que rechazar brevemente |

Elígela con `.on_backend_error(BackendErrorPolicy::FailClosed)` en el
middleware. Las solicitudes que agotan la cuota siempre son 429 sin
importar la política - la política solo afecta el recaer tras un
error de backend.

## Fachada con forma de Laravel respaldada por caché

`RateLimiter` (el struct) refleja `Illuminate\Cache\RateLimiter`. Es
un contador de ventana fija construido sobre la fachada
[`Cache`](cache.md) de Suprnova. Úsalo para limitadores con nombre,
flujos `attempt()`, o cualquier vez que quieras los encabezados
`X-RateLimit-*` que las apps de Laravel esperan.

### Disposición de almacenamiento

Para una clave de contador de intentos `K` con un decaimiento de `D`
segundos:

- `K` - contador i64 incrementado por cada `hit`. La siembra inicial
  es 0 (vía `Cache::add`).
- `K:timer` - marca de tiempo i64 unix-en-segundos de cuándo termina
  la ventana, fijada vía `Cache::add` para que solo quien llama
  primero en una ventana fije el plazo.

Ambas claves llevan el mismo TTL, así que la caché las limpia
automáticamente cuando la ventana termina. Cuando el contador llegó a
`max_attempts` pero el `:timer` ya no está, `too_many_attempts`
reinicia el contador - esto es lo que hace que la ventana se deslice
hacia adelante tras un período de cuota agotada.

### API del contador

```rust
use suprnova::RateLimiter;

// Gasta un intento; siembra la ventana si falta.
let n = RateLimiter::hit("login:1.2.3.4", 60).await?;

// Gasta un intento Y comprueba el límite en un único viaje de ida y
// vuelta atómico. Devuelve `true` cuando este golpe empujó el cubo
// por encima de `max` (rechaza la solicitud), `false` cuando fue
// admitido. Usa esto en lugar de un par separado de
// `too_many_attempts` + `hit`: comprobar y luego golpear como dos
// llamadas deja que solicitudes concurrentes se cuelen más allá del
// límite (una carrera de comprobar-y-actuar).
// `i64::MAX` como max significa "sin límite" - siempre admite, pero
// sigue contando.
let over_limit = RateLimiter::hit_and_check("login:1.2.3.4", 5, 60).await?;
if over_limit { /* devuelve 429 */ }

// Incrementa por N; útil para límites "ponderados por costo" (cada
// solicitud gasta más de un intento).
let n = RateLimiter::increment("api:user:1", 60, 5).await?;

// Lee el conteo actual (0 cuando nunca se golpeó o expiró).
let attempts = RateLimiter::attempts("login:1.2.3.4").await?;

// Segundos hasta que la ventana reabra (0 cuando no hay ventana abierta).
let secs = RateLimiter::available_in("login:1.2.3.4").await?;

// Reintentos restantes antes de dispararse.
let remaining = RateLimiter::remaining("login:1.2.3.4", 5).await?;
// retries_left es el alias con la grafía de Laravel para remaining.
let remaining = RateLimiter::retries_left("login:1.2.3.4", 5).await?;

// ¿Está el cubo sobre su límite AHORA MISMO (con la ventana aún abierta)?
let over = RateLimiter::too_many_attempts("login:1.2.3.4", 5).await?;

// Descarta solo el contador (el timer se queda - la ventana sigue fijada).
RateLimiter::reset_attempts("login:1.2.3.4").await?;

// Descarta tanto el contador como el timer.
RateLimiter::clear("login:1.2.3.4").await?;
```

### El flujo de trabajo `attempt()`

Ejecuta un callback solo cuando el cubo está bajo la cuota; el golpe
solo se gasta cuando el callback se ejecuta:

```rust
let result = RateLimiter::attempt(
    "login:1.2.3.4",
    5,
    || async { do_login_work().await },
    60,
).await?;
match result {
    Some(value) => { /* el callback corrió, el intento se contó */ }
    None => { /* sobre el límite, el callback NO se ejecutó */ }
}
```

Esta es la forma correcta para formularios de login - no gastas un
intento salvo que el trabajo de verdad llegue al callback.

### Limitadores con nombre

Regístralos en el arranque, resuélvelos en el momento de la
solicitud. El nombre del lado de Laravel `for` es una palabra clave
reservada en Rust, así que el nombre primario del lado de Rust es
`define`; el alias literal de Laravel se expone vía `r#for`.

```rust
use suprnova::{Limit, RateLimiter};

// En el arranque - `define` es el nombre primario del lado de Rust.
RateLimiter::define("api", |req| {
    // `req.ip()`, no el encabezado en crudo `X-Forwarded-For` - ver abajo.
    let key = req.ip().unwrap_or_else(|| "anon".into());
    Limit::per_minute(60).by(format!("ip:{key}")).into()
});

// Alias del lado de Laravel - lo mismo bajo la grafía de escape de palabra clave.
RateLimiter::r#for("uploads", |_req| Limit::per_hour(100).into());

// Resuelve.
let cb = RateLimiter::limiter("api").unwrap();
let limit_result = cb(&request);
```

Un callback de limitador con nombre devuelve un [`LimitResult`],
construible a partir de:

- Un único `Limit` - aplica este límite.
- Un `Vec<Limit>` - aplica cada límite; el primero en dispararse
  gana.
- Un `HttpResponse` - corta en corto de inmediato con esta respuesta
  (se usa para "el admin tiene acceso ilimitado" vía `Limit::none()`,
  o para rechazar la solicitud sin más).

### Sanitizar claves

`RateLimiter::clean_rate_limiter_key(key)` elimina de una clave las
marcas de entidad HTML `&abc;` - Laravel usa esto para cadenas
suministradas por el usuario que hacen ida y vuelta por
`htmlentities`. Suprnova reproduce exactamente la etapa de
eliminación, pero NO antepone la codificación `htmlentities` (que
solo importa para entradas no UTF-8, irrelevante para un `String` de
Rust). La función es determinista e idempotente dentro de Suprnova;
quienes necesiten un hash byte-idéntico con un servicio PHP deberían
correr su propio paso previo de `htmlentities` sobre la entrada.

```rust
assert_eq!(RateLimiter::clean_rate_limiter_key("a&amp;b"), "aab");
```

## Builder `Limit`

El tipo de dato que devuelven los callbacks de limitador con nombre.
Los constructores abreviados reflejan los `Limit::per*` de Laravel:

```rust
use suprnova::Limit;
use std::time::Duration;

Limit::per_second(10, 1);           // 10 por 1 segundo (max_attempts, decay_seconds)
Limit::per_minute(60);              // 60 por minuto
Limit::per_minutes(5, 100);         // 100 por 5 minutos (decay primero, firma de Laravel)
Limit::per_hour(1_000);             // 1000/h
Limit::per_hours(6, 5_000);         // 5000 por 6 horas
Limit::per_day(10_000);             // 10000/día
Limit::per_days(7, 50_000);         // 50000 por 7 días
Limit::new(123, Duration::from_secs(45));  // ctor a secas

// Cadena de builder.
let l = Limit::per_minute(5)
    .by("user:42")
    .response(|req| {
        suprnova::HttpResponse::text("blocked").status(429)
    })
    .after(|response| response.status_code() >= 400);
```

- `.by(key)` - fija la clave del cubo. Una clave vacía es "global"
  (todos los que llaman comparten un cubo).
- `.response(callback)` - genera una respuesta personalizada cuando
  el límite se dispara; el valor por defecto es un 429 plano "Too
  Many Attempts.".
- `.after(callback)` - solo gasta el intento cuando
  `callback(response)` devuelve true. Uso canónico: contar solo
  logins fallidos (`after(|r| r.status_code() >= 400)`).

`Limit::none()` devuelve un `Unlimited` (un `GlobalLimit` con
`max_attempts = i64::MAX`). Devolverlo desde un limitador con nombre
es el patrón de Laravel para el bypass. `GlobalLimit` en sí es un
envoltorio fino alrededor de `Limit` con una clave vacía, mantenido
por paridad con `Illuminate\Cache\RateLimiting\GlobalLimit`.

## `ThrottleRequestsMiddleware`

Envoltorio HTTP alrededor de la fachada respaldada por caché. Refleja
`Illuminate\Routing\Middleware\ThrottleRequests`. Tres constructores:

```rust
use suprnova::{Limit, ThrottleRequestsMiddleware};

// Limitador con nombre - resuelve en el momento de la solicitud vía RateLimiter::limiter(name).
ThrottleRequestsMiddleware::by_name("api");

// max/decay/prefix inline - la forma literal `throttle:60,1` de Laravel.
ThrottleRequestsMiddleware::with(60, 1, "myroute");

// Lista explícita de Limits - el primero en dispararse gana; lo más idiomático en Rust.
ThrottleRequestsMiddleware::with_limits(vec![
    Limit::per_hour(5_000).by("user:1"),
    Limit::per_minute(60).by("user:1"),
]);
```

Conéctalo a un grupo de rutas:

```rust
use suprnova::{Limit, RateLimiter, Router, ThrottleRequestsMiddleware};

RateLimiter::define("api", |req| {
    Limit::per_minute(60)
        .by(req.ip().unwrap_or_else(|| "anon".into()))
        .into()
});

let router = Router::new()
    .get("/api/items", list_items)
    .post("/api/items", create_item)
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

### Indexa por `req.ip()`, nunca por el encabezado

`X-Forwarded-For` lo suministra quien llama. Un limitador indexado
por el encabezado en crudo se elude enviando un valor distinto en
cada solicitud - el atacante elige su propio cubo, así que la cuota
queda por solicitud en lugar de por cliente.

`Request::ip()` es la lectura segura. Devuelve `X-Forwarded-For` /
`X-Real-IP` **solo cuando el par TCP está en la lista de
`APP_TRUSTED_PROXIES`**, y en caso contrario la dirección del par, así
que un encabezado que no venga de tu propio proxy se ignora.

El corolario importa tanto como la regla: con esa variable sin
establecer - el valor por defecto - `req.ip()` detrás de un proxy
terminador devuelve la dirección *del proxy* en cada solicitud, y
cada límite por IP de la app colapsa en un único cubo compartido.
`ThrottleRequestsMiddleware::with(20, 1, "login")` entonces significa
20 intentos por minuto entre todos los usuarios combinados, que
cualquier llamador puede gastar para bloquear a todos los demás.
Desplegar detrás de nginx, Traefik, un ALB o Cloudflare significa
establecer
[`APP_TRUSTED_PROXIES`](env-vars.md#behind-a-reverse-proxy-set-app_trusted_proxies).

### Encabezados de la respuesta

Cada respuesta envuelta lleva:

- `X-RateLimit-Limit` - el `max_attempts` configurado.
- `X-RateLimit-Remaining` - reintentos restantes para este cubo.

Las respuestas 429 además llevan:

- `Retry-After` - segundos hasta que la ventana reabra.
- `X-RateLimit-Reset` - marca de tiempo unix-en-segundos de cuándo
  el cubo reabre.

Esto coincide exactamente con la forma de
`ThrottleRequests::getHeaders` de Laravel.

### Limitador con nombre ausente

Cuando una ruta está conectada a `by_name("X")` pero no se ha
registrado ningún limitador bajo `X`, el middleware devuelve HTTP 503
con un cuerpo que nombra el limitador ausente. Laravel lanza
`MissingRateLimiterException`; nosotros lo exponemos como una
respuesta HTTP para que un arranque mal configurado no haga pánico al
hilo del worker.

### Composición driver-versus-fachada

Los dos middlewares pueden coexistir en un mismo router. Apila el
driver de ventana deslizante para equidad de bajo nivel, y luego el
throttle respaldado por caché para límites con nombre por endpoint:

```rust
let router = Router::new()
    .get("/api/items", list_items)
    .middleware(RateLimitMiddleware::new(limiter_driver, cfg, key_fn))
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

## Configuración

La SPI del driver se configura vía variables de entorno; la fachada
respaldada por caché se configura donde sea que esté configurado tu
store [`Cache`](cache.md) (memory o Redis).

| Variable | Usada por | Por defecto |
|----------|---------|---------|
| `RATE_LIMIT_DRIVER` | Arranque de la SPI del driver | `memory` (rechazado en producción - ver arriba) |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | Anulación de fallo-en-cerrado en producción | sin establecer |
| `RATE_LIMIT_REDIS_URL` | Driver de Redis | `redis://127.0.0.1:6379` |
| `RATE_LIMIT_PREFIX` | Prefijo de clave de Redis | `suprnova:` |
| `CACHE_DRIVER` / `REDIS_URL` / `CACHE_DEFAULT_TTL` / `REDIS_PREFIX` | Fachada `RateLimiter` respaldada por caché (ver [`Cache`](cache.md)) | varios |

## Migración desde Laravel

| Laravel | Suprnova |
|---------|----------|
| `RateLimiter::for('api', fn ($req) => Limit::perMinute(60))` | `RateLimiter::define("api", \|req\| Limit::per_minute(60).into())` o `RateLimiter::r#for(...)` |
| `RateLimiter::hit($key, $decay)` | `RateLimiter::hit(key, decay).await?` |
| `RateLimiter::tooManyAttempts($key, $max)` | `RateLimiter::too_many_attempts(key, max).await?` |
| `RateLimiter::availableIn($key)` | `RateLimiter::available_in(key).await?` |
| `RateLimiter::attempt($key, $max, $cb, $decay)` | `RateLimiter::attempt(key, max, \|\| async { ... }, decay).await?` |
| `RateLimiter::retriesLeft($key, $max)` | `RateLimiter::retries_left(key, max).await?` |
| `RateLimiter::cleanRateLimiterKey($key)` | `RateLimiter::clean_rate_limiter_key(key)` |
| `Limit::perMinute(60)->by($ip)->response(fn () => abort(429))` | `Limit::per_minute(60).by(ip).response(\|_\| HttpResponse::text("...").status(429))` |
| `Limit::perMinutes(3, 100)` | `Limit::per_minutes(3, 100)` |
| `Limit::none()` | `Limit::none()` |
| `throttle:api` middleware | `ThrottleRequestsMiddleware::by_name("api")` |
| `throttle:60,1` middleware | `ThrottleRequestsMiddleware::with(60, 1, "")` |
| Encabezados `X-RateLimit-Limit/Remaining/Reset` + `Retry-After` | Los mismos encabezados, la misma forma |

### Por qué Suprnova diverge

Laravel ofrece una sola forma: `Illuminate\Cache\RateLimiter`
(contador de ventana fija respaldado por caché) con
`Illuminate\Routing\Middleware\ThrottleRequests` como su envoltorio
HTTP. Suprnova ofrece esa forma *y además* una SPI de driver de
ventana deslizante nativa, porque dos preguntas reales necesitan dos
respuestas reales.

Un contador respaldado por caché es la respuesta correcta a "tengo
limitadores con nombre, callbacks de respuesta, callbacks posteriores
solo para contar logins fallidos, y quiero ser compatible en el
código fuente con migraciones de Laravel". Es la respuesta
equivocada a "necesito una aplicación exacta de ventana deslizante de
un slot por solicitud, contra un Redis ZSET con evaluación Lua
atómica y sin clave de temporizador separada". Esa segunda pregunta
es lo que de verdad tiene la mayoría de los servicios en Rust que
topan con los límites de concurrencia de Tokio, así que
`RateLimiterDriver` + `RateLimitMiddleware` existen en paralelo, no
detrás de un feature flag.

La política de error de backend también es una adición de Suprnova.
El middleware de Laravel nunca expone una decisión de "el limitador
está roto", porque el ciclo de vida por solicitud de PHP la esconde -
la siguiente solicitud obtiene un proceso nuevo. Un worker de Tokio
de larga duración que pierde Redis durante diez segundos debe decidir
qué hacer con las solicitudes que llegan durante esa ventana;
`BackendErrorPolicy::FailOpen` (por defecto) frente a `FailClosed` es
esa decisión expuesta explícitamente.

## Siguiente

- [Middleware](middleware.md) - cómo se compone, se ejecuta, y hace
  cortocircuito el middleware en la cadena de la solicitud
- [Caché](cache.md) - el store sobre el que se construye la fachada
  `RateLimiter` con forma de Laravel
- [Configuración](configuration.md) - configuración tipada para los
  backends de caché y Redis
- [Flujos de autenticación](auth-flows.md) - `LoginThrottleMiddleware`
  y el patrón de bloqueo por fuerza bruta se construyen sobre esta
  superficie
- [Modelo de errores](error-model.md) - por qué
  `Result<HttpResponse, HttpResponse>` deja que el middleware haga
  cortocircuito de forma limpia

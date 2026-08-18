# Middleware

El middleware envuelve un handler de solicitud. Se ejecuta antes de que el
handler vea la solicitud, y otra vez después de que el handler devuelve
una respuesta, así que es el lugar donde poner el trabajo transversal -
auth, logging, CORS, limitación de velocidad, medición de tiempos,
transformar la solicitud o la respuesta. La superficie de Suprnova es la
misma que ya conocen quienes usan Laravel: un método `handle(request,
next)` que decide si reenvía la solicitud, hace cortocircuito, o muta la
respuesta a la salida.

## El trait

Un middleware es un struct que implementa `Middleware`:

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct LoggingMiddleware;

#[async_trait]
impl Middleware for LoggingMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Preprocesamiento: se ejecuta antes del handler.
        println!("--> {} {}", request.method(), request.path());

        // Reenvía al siguiente middleware (o al handler si esta es
        // la última capa).
        let response = next(request).await;

        // Postprocesamiento: se ejecuta después de que el handler retorna.
        println!("<-- complete");

        response
    }
}
```

`handle` tiene tres cosas que puede hacer, y en una solicitud dada solo
hace falta hacer una de ellas:

- **Reenviar.** Llama a `next(request).await` para pasar el control a la
  siguiente capa. La `Response` devuelta es lo que verá cada capa por
  encima.
- **Cortocircuito.** Devuelve `Err(HttpResponse::...)` sin llamar a
  `next`. El framework colapsa ambas ramas de `Response`
  (`Result<HttpResponse, HttpResponse>`) en una única respuesta - un
  `Err` es una respuesta, no un crash. Consulta [Modelo de
  errores](error-model.md).
- **Mutar.** Modifica la solicitud antes de reenviarla, o modifica la
  respuesta después.

`Next` es `Arc<dyn Fn(Request) -> MiddlewareFuture + Send + Sync>` -
trátalo como una función async de `Request` a `Response`.

## Generación de un stub

La CLI genera el andamiaje de un archivo de middleware funcional:

```bash
suprnova make:middleware Auth         # → src/middleware/auth.rs (AuthMiddleware)
suprnova make:middleware RateLimit    # → src/middleware/rate_limit.rs
suprnova make:middleware CorsMiddleware  # el sufijo "Middleware" está bien, mismo resultado
```

El archivo generado no es un stub con TODOs - es un middleware real que
mide el tiempo de la solicitud envuelta y registra los eventos de
entrada/salida con el id por solicitud instalado por
`RequestIdMiddleware`. Reemplaza el cuerpo por lo que realmente necesites.

## Registro de middleware

Tres lugares donde instalarlo, según el alcance:

### Global

Se ejecuta en cada solicitud, en el orden de registro. Usa la macro
`global_middleware!` dentro de `bootstrap()`:

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, FrameworkError};
use crate::middleware;

pub async fn bootstrap() -> Result<(), FrameworkError> {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
    Ok(())
}
```

`global_middleware!(M)` se expande a `register_global_middleware(M)`. El
registro es **idempotente por tipo concreto** - registrar el mismo struct
dos veces conserva el primer registro y emite un log de depuración. Eso
hace que volver a ejecutar el arranque (tests, hot-reload, varias
instancias de `Server` en un mismo proceso) sea seguro. Para instalar
varias copias del mismo comportamiento con configuración distinta, envuelve
cada una en un newtype propio.

### Por ruta

Encadena `.middleware(M)` en una definición de ruta de la macro `routes!`:

```rust
// src/routes.rs
use suprnova::{routes, get};
use crate::{controllers, middleware::AuthMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/public", controllers::home::public),

    get!("/protected", controllers::dashboard::index)
        .middleware(AuthMiddleware),
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware),
}
```

### Por grupo

Aplica middleware a cada ruta dentro de un bloque `group(...)`:

```rust
use suprnova::Router;
use crate::middleware::{ApiMiddleware, AuthMiddleware};
use crate::controllers::{user, admin};

Router::new()
    // Rutas públicas - sin middleware.
    .get("/", home_handler)
    .get("/login", login_handler)

    // Cada ruta bajo /api lleva ApiMiddleware.
    .group("/api", |r| {
        r.get("/users", user::index)
         .post("/users", user::store)
         .get("/users/{id}", user::show)
    })
    .middleware(ApiMiddleware)

    // Las rutas de admin comparten auth.
    .group("/admin", |r| {
        r.get("/dashboard", admin::dashboard)
         .get("/settings", admin::settings)
    })
    .middleware(AuthMiddleware);
```

## Orden de ejecución

En tiempo de ejecución, la cadena corre de afuera hacia adentro:

```
Solicitud  →  RequestId  →  globales  →  MW de grupo  →  MW de ruta  →  handler
                                                                          │
Respuesta  ←  RequestId  ←  globales  ←  MW de grupo  ←  MW de ruta  ←  handler
```

El primer middleware agregado es el primero en ejecutarse. A la salida, el
orden se invierte - `MiddlewareChain::execute` anida el post-procesamiento
de cada capa dentro del de la anterior.

Si un middleware hace cortocircuito con `Err(response)`, la cadena se
desenrolla de inmediato: toda capa POR ENCIMA del cortocircuito sigue
viendo la respuesta a la salida, pero las capas POR DEBAJO (más cerca del
handler) no se ejecutan.

### El middleware de grupo se aplana, no se apila

Este punto importa y merece mencionarse aparte. **El middleware de grupo
de rutas no es una capa de runtime separada.** Cuando se ejecuta
`GroupBuilder::try_finalize`, copia el middleware del grupo en la lista de
middleware `(method, pattern)` de cada ruta agrupada. En tiempo de
ejecución, el middleware de grupo es indistinguible del middleware
adjuntado directamente a la ruta.

Dos consecuencias:

- El orden en tiempo de ejecución sigue siendo correcto (el middleware de
  grupo se ejecuta antes que el middleware de ruta porque se registra
  primero), pero **la introspección no puede distinguir el middleware de
  grupo del middleware de ruta**.
- El middleware se indexa por el patrón coincidente (`"/posts/{id}"`), no
  por la ruta cruda (`/posts/42`), de modo que el middleware de grupo en
  rutas parametrizadas se dispara de forma confiable.

Consulta `framework/src/routing/group.rs` para el paso de aplanado y
`framework/src/middleware/chain.rs` para el bucle de ejecución.

## Cortocircuito

Retorna anticipadamente para bloquear una solicitud antes de que llegue al
handler:

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct RequireApiKey;

#[async_trait]
impl Middleware for RequireApiKey {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if request.header("X-Api-Key").is_none() {
            return Err(HttpResponse::text("Unauthorized").status(401));
        }
        next(request).await
    }
}
```

La cadena colapsa `Result<HttpResponse, HttpResponse>` en una única
respuesta, así que `Err(...)` es solo una respuesta con un rol distinto.
Las capas por encima de este middleware igual la observan a la salida y
pueden post-procesarla.

## Seguridad ante pánicos

`MiddlewareChain::execute` NO atrapa pánicos - un pánico en cualquier
middleware o en el handler se desenrolla directamente hacia afuera, como
cualquier otra función async. La red de seguridad de la ruta de solicitud
vive un nivel más arriba, en el límite del servidor, dentro de
`execute_chain_safely`, que envuelve la cadena en `catch_unwind` y
convierte un pánico en un 500 sanitizado con el id de solicitud,
despachando `ErrorOccurred` para cualquier oyente de observabilidad.
Consulta [Ciclo de vida de la solicitud](lifecycle.md) para el flujo
completo de recuperación de pánicos.

Esta separación es deliberada: el manejo estandarizado de pánicos ocurre
exactamente una vez, donde lo posee el ciclo de vida de la solicitud, en
lugar de duplicarse dentro de la primitiva agnóstica de capa. Quien
conduzca una cadena fuera de ese límite es responsable de su propio
`catch_unwind`.

## Middleware integrado

Un mapa no exhaustivo. Cada uno viene listo para instalar - la mayoría
necesita un struct de configuración, ninguno necesita andamiaje.

| Middleware | Propósito |
|---|---|
| `RequestIdMiddleware` | Capa siempre más externa; asigna un UUID por solicitud y lo etiqueta a lo largo de los logs y de `X-Request-Id` |
| `TimeoutMiddleware` | Acota el tiempo hasta la respuesta; devuelve 503 cuando se supera (ver más abajo) |
| `CorsMiddleware` | Gestiona el preflight de CORS y decora las respuestas cross-origin (ver más abajo) |
| `CsrfMiddleware` | Protección CSRF de doble envío por cookie, con `OriginPolicy` configurable |
| `RateLimitMiddleware` / `ThrottleRequestsMiddleware` | Limitación por cubo de tokens y por ventana deslizante; consulta [Limitación de velocidad](rate-limiting.md) |
| `SessionMiddleware` | Carga y persiste la sesión sobre cookies; alimenta `req.session()` |
| `AuthMiddleware` / `GuestMiddleware` / `BearerTokenMiddleware` | Comprobaciones de pertenencia al guard; consulta [Autenticación](authentication.md) |
| `LoginThrottleMiddleware` / `EnsureEmailVerifiedMiddleware` / `TwoFactorChallengeMiddleware` | Compuertas de los flujos de auth; consulta [Flujos de autenticación](auth-flows.md) |
| `MaintenanceMiddleware` | Devuelve 503 cuando está puesto el flag de mantenimiento en la caché o en el sistema de archivos |
| `InertiaHeadersMiddleware` / `InertiaVersionMiddleware` / `Inertia303Middleware` / `EncryptHistoryMiddleware` | Protocolo de Inertia: `Vary: X-Inertia` en todas las respuestas y redirección de vuelta ante un 200 vacío; rebote 409 de versión de assets; 302→303 en redirecciones que no son GET; cifrado del historial. Los tres primeros los registra `Inertia::install`; consulta [Respuestas de Inertia](frontend-inertia-responses.md#bootstrap-inertia-install) |
| `IncludeMiddleware` | Conjuntos de include por campo para las recargas parciales de `#[derive(Data)]` |

### Tiempos de espera de solicitudes

`TimeoutMiddleware` acota cuánto puede tardar un handler en *producir*
una respuesta. Si no, un handler lento o una consulta de base de datos
colgada pueden mantener una conexión abierta indefinidamente; el tiempo
de espera devuelve `503 Service Unavailable` en cuanto se supera el
plazo.

```rust
// src/bootstrap.rs - techo de 30 segundos en cada ruta HTTP.
use suprnova::{global_middleware, TimeoutMiddleware};

global_middleware!(TimeoutMiddleware::default()); // DEFAULT_TIMEOUT = 30s
```

```rust
// Aprieta un solo endpoint a 5 segundos.
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    .get("/report", heavy_report_handler)
    .middleware(TimeoutMiddleware::seconds(5));
```

`TimeoutMiddleware::new(Duration)` acepta cualquier duración;
`TimeoutMiddleware::seconds(n)` es la forma corta para segundos
enteros.

El middleware global se ejecuta **fuera** del middleware de ruta, así
que un tiempo de espera global es un techo exterior y uno por ruta solo
puede hacer *más estricta* una ruta concreta - el plazo más corto se
dispara primero. Para dejar que una ruta corra más tiempo que el valor
global por defecto, sube el valor global o acota el middleware global a
un grupo de rutas que excluya ese endpoint.

Las respuestas en streaming (`HttpResponse::sse(...)`,
`HttpResponse::stream_bytes(...)`) están exentas de forma natural: el
handler retorna de inmediato con un cuerpo perezoso que hyper drena
después de que la cadena de middleware termine. Las mejoras a WebSocket
también se omiten explícitamente. Consulta
[Tiempos de espera](timeout.md) para la semántica de seguridad ante la
cancelación.

### CORS

`CorsMiddleware` añade los encabezados `Access-Control-*` que un
navegador necesita para dejar que una página de otro origen lea tus
respuestas, y responde a la solicitud `OPTIONS` de preflight que los
navegadores envían antes de las llamadas cross-origin no simples. Las
aplicaciones de mismo origen (la configuración de Inertia por defecto)
no lo necesitan - solo importa cuando un navegador en un origen
*distinto* llama a tu API.

CORS debe instalarse de forma **global** para que los preflights lleguen
hasta él (un preflight nunca coincide con una ruta, así que un
middleware de CORS por ruta nunca vería ninguno). Deliberadamente no hay
un valor por defecto permisivo - elige una política de origen de forma
explícita:

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

global_middleware!(CorsMiddleware::new(
    CorsConfig::allow_origins(["https://app.example", "https://admin.example"])
        .allow_credentials(true)
        .max_age(std::time::Duration::from_secs(600)),
));
```

`CorsConfig::any_origin()` opta explícitamente por
`Access-Control-Allow-Origin: *`. Métodos del builder: `.methods([...])`,
`.allow_headers([...])` / `.allow_any_headers()`,
`.expose_headers([...])`, `.paths([...])` (acota CORS a patrones de
URL), `.allow_origin_patterns([regex...])`, `.skip_when(|req| bool)`,
`.allow_credentials(bool)`, `.max_age(Duration)`. Junto a ellos vienen
alias con nombres de Laravel (p. ej. `.supports_credentials`,
`.allowed_methods`) para que una configuración de Laravel se mapee
directamente.

`Access-Control-Allow-Origin: *` es inválido junto con credenciales - el
navegador lo rechaza. Cuando se establece `.allow_credentials(true)`, el
middleware siempre devuelve el `Origin` concreto de la solicitud en vez
de `*`, así que la combinación inválida nunca se puede emitir. Las
respuestas sin comodín reciben además `Vary: Origin` para que las cachés
compartidas sigan siendo correctas. Consulta [CORS](cors.md).

## Pipeline - el `Illuminate\Pipeline\Pipeline` de Laravel

`Pipeline` es el análogo en Suprnova de la clase pipeline de Laravel - un
builder fluido sobre `MiddlewareChain` que refleja la forma `send /
through / pipe / then / then_return / finally_with` que ya conocen quienes
usan Laravel. Útil cuando se quiere ensamblar una cadena de middleware
fuera del ciclo de vida de la solicitud (un job, un comando de la CLI, un
test de integración puntual):

```rust
use suprnova::{Pipeline, Request};

let response = Pipeline::new()
    .send(request)
    .through([AuthMiddleware, LoggingMiddleware])
    .pipe(CorsMiddleware::new(cors_config))
    .finally_with(|| tracing::info!("pipeline complete"))
    .then(|req| async move { handler(req).await })
    .await;
```

Los alias del lado de Rust vienen junto a los nombres de Laravel:
`with_request` para `send`, `with_middleware` para `through`, `push` para
`pipe`, `on_finally` para `finally_with`, `execute` para `then`. Usa el
que se lea mejor en tu código.

| Método de Pipeline | Laravel | Alias en Rust | Propósito |
|---|---|---|---|
| `send(request)` | `send($passable)` | `with_request(request)` | Fija la solicitud que se pasa a través de la cadena |
| `through(iter)` | `through($pipes)` | `with_middleware(iter)` | Reemplaza la lista de pipes |
| `through_boxed(iter)` | - | - | Reemplaza la lista de pipes con middleware ya envuelto en `Box` |
| `pipe(M)` | `pipe($pipes)` | `push(M)` | Agrega un único middleware |
| `pipe_boxed(M)` | - | - | Agrega un middleware ya envuelto en `Box` |
| `then(destination)` | `then($destination)` | `execute(destination)` | Ejecuta la cadena con el handler de destino |
| `then_with(req, dst)` | - | - | Sobrescribe el passable en línea |
| `then_return()` | `thenReturn()` | - | Ejecuta la cadena y devuelve un 204 No Content |
| `finally_with(F)` | `finally($callback)` | `on_finally(F)` | Se ejecuta después de que el destino se resuelve |

## Middleware terminable - ganchos posteriores a la respuesta

El middleware terminable se ejecuta *después* de que la respuesta se
envió al cliente. Úsalo para IO lento que no necesita bloquear la
respuesta: persistencia de sesión, logging de auditoría, envío de
métricas.

Suprnova ofrece esto como un trait `Terminable` dedicado, separado de
`Middleware`, de modo que la ruta de solicitud y la ruta de terminación
quedan claramente tipadas por separado. Un tipo puede implementar uno, el
otro, o ambos:

```rust
use suprnova::{Terminable, TerminationSnapshot, register_terminable, async_trait};

pub struct AuditLogTerminator;

#[async_trait]
impl Terminable for AuditLogTerminator {
    async fn terminate(&self, snapshot: &TerminationSnapshot) {
        tracing::info!(
            method = %snapshot.method,
            path = %snapshot.path,
            status = snapshot.status,
            "request handled",
        );
    }
}

// En bootstrap.rs
register_terminable(AuditLogTerminator);
```

El servidor itera los terminables registrados en orden de registro
después de cada respuesta (4xx y 5xx incluidos) y espera (`await`) cada
uno. Los errores se registran vía `tracing::error!` y se descartan - la
respuesta ya salió por la puerta, así que ya no queda nadie a quien
mostrárselos.

El registro es idempotente por tipo concreto. `registered_terminables()`,
`terminable_count()`, y `has_terminable::<T>()` ofrecen introspección para
tests y diagnósticos en tiempo de arranque.

## Alias y grupos con nombre

Para quienes prefieren middleware indexado por string (los
`middlewareAliases` / `middlewareGroups` de Laravel), Suprnova ofrece un
registro de alias + grupos global para todo el proceso:

```rust
use suprnova::middleware::{
    register_middleware_alias, register_middleware_group,
    resolve_middleware_group,
};

// Los alias son closures de fábrica - se invocan de cero en cada
// resolución, así que cada registro de ruta produce una instancia de
// middleware independiente.
register_middleware_alias("auth", || AuthMiddleware::new());
register_middleware_alias("throttle", || ThrottleRequestsMiddleware::default());

// Los grupos agrupan alias. Se admiten grupos anidados.
register_middleware_group("api", ["auth".into(), "throttle".into()]);
register_middleware_group("web", ["session".into(), "auth".into()]);

// Resuelve en un Vec<BoxedMiddleware> en el arranque o por ruta.
let api_mws = resolve_middleware_group("api")?;
```

`resolve_middleware_group` devuelve `Err(MiddlewareResolveError)` cuando:

- `UnknownGroup(name)` - el grupo con ese nombre nunca se registró;
- `UnknownAlias { group, missing }` - una entrada del grupo no es un alias
  conocido;
- `UnknownNestedGroup { group, missing }` - una referencia a un grupo
  anidado no se puede resolver;
- `CycleDetected { group }` - la definición del grupo es recursiva.

El registro de un alias o grupo sigue la regla de que **gana el último**
para el mismo nombre, reflejando el array reasignable del kernel de
Laravel.

## Prioridad de middleware

`prepend_middleware_priority::<M>()` / `append_middleware_priority::<M>()`
registran un `TypeId` en la lista de prioridad global del proceso - el
análogo en Suprnova de `Kernel::$middlewarePriority` de Laravel. El
middleware cuyo tipo aparece antes en la lista se ordena hacia el frente
de la cadena sin importar el orden de registro:

```rust
use suprnova::{append_middleware_priority};

// SessionMiddleware siempre se ejecuta antes que AuthMiddleware sin
// importar el orden en que se registraron.
append_middleware_priority::<SessionMiddleware>();
append_middleware_priority::<AuthMiddleware>();
```

`middleware_priority()` devuelve una instantánea del `Vec<TypeId>` actual,
para diagnóstico o para quien incorpore el framework y quiera manejar su
propio ordenamiento.

## Introspección del registro

Más allá de `register_global_middleware`, el registro expone:

| Superficie | Laravel | Propósito |
|---|---|---|
| `prepend_global_middleware(M)` | `prependMiddleware` | Inserta al frente de la cadena |
| `has_global_middleware::<M>()` | `hasMiddleware` | Si el tipo `M` está registrado |
| `global_middleware_count()` | - | Cantidad de globales registrados actualmente |
| `MiddlewareRegistry::from_global()` | - | Toma una instantánea del registro global hacia un registro por servidor |
| `MiddlewareRegistry::prepend(M)` | - | Inserta al frente, al estilo builder, sobre una instancia de registro |
| `MiddlewareRegistry::append_boxed(M)` | - | Agrega un middleware ya envuelto en `Box` |
| `MiddlewareRegistry::prepend_boxed(M)` | - | Inserta al frente un middleware ya envuelto en `Box` |
| `MiddlewareRegistry::len()` / `is_empty()` | - | Introspección del builder |

`MiddlewareRegistry::from_global()` toma una instantánea del registro
global en el momento de la llamada. Registra todo el middleware global
ANTES de construir el servidor - una llamada a `global_middleware!` hecha
DESPUÉS de que el servidor ya se construyó no se aplica retroactivamente,
de modo que la pila de middleware de un servidor en ejecución no puede
cambiar por debajo de él.

## Layout de archivos

Un layout típico una vez que se tienen unos cuantos middlewares:

```
src/
├── middleware/
│   ├── mod.rs          # mod + pub use
│   ├── auth.rs         # AuthMiddleware
│   ├── logging.rs      # LoggingMiddleware
│   └── audit.rs        # AuditLogTerminator
├── bootstrap.rs        # global_middleware! + register_terminable
├── routes.rs           # .middleware(M) por ruta
└── main.rs
```

`make:middleware` mantiene `src/middleware/mod.rs` sincronizado - agrega
la nueva declaración `mod foo;` y el re-export `pub use
foo::FooMiddleware;` correspondiente cuando el archivo se genera.

## Por qué Suprnova diverge

Laravel registra las clases de middleware en `app/Http/Kernel.php` y las
resuelve a través del contenedor, que hace reflection sobre los
type-hints del constructor para inyectar dependencias. El modelo de
solicitud-por-proceso de PHP implica que el kernel se reconstruye en cada
solicitud, así que el costo de la resolución por reflection se paga una
vez por solicitud y desaparece entre solicitudes.

El modelo de procesos de Suprnova es un único binario que sirve muchas
solicitudes concurrentes en muchos hilos. Construir una cadena nueva por
solicitud forzaría un punto de sincronización sobre la lista global de
middleware y volvería a asignar `Arc<dyn Middleware>` para cada capa en
cada solicitud. En cambio:

- El middleware global se registra en un `OnceLock<RwLock<Vec<...>>>` en
  el arranque, indexado por `TypeId` para un registro idempotente.
- `MiddlewareRegistry::from_global()` toma una instantánea de la lista
  global una sola vez en la construcción del servidor; la cadena por
  solicitud reutiliza esa instantánea.
- La cadena en sí se compone anidando closures `Arc<dyn Fn>`, así que el
  trabajo por solicitud es un `Arc::clone` por capa en lugar de una
  asignación nueva.

La superficie de cara al usuario - `handle(request, next)`, la macro
`global_middleware!`, los alias con nombre, las listas de prioridad, los
ganchos terminables - es la misma a la que recurre un desarrollador de
Laravel. La maquinaria de abajo cambia la reconstrucción por solicitud de
PHP por un modelo de instantánea en el arranque con forma de Rust, para
que el framework pueda servir solicitudes concurrentes sin contención
sobre el registro.

## Siguiente

- [Ciclo de vida de la solicitud](lifecycle.md) - dónde corre la cadena y
  cómo se atrapan los pánicos en el límite del servidor
- [Modelo de errores](error-model.md) - qué significa realmente
  `Result<HttpResponse, HttpResponse>` y cómo colapsan los cortocircuitos
- [Tiempos de espera de solicitudes](timeout.md) - la seguridad ante
  cancelación de `TimeoutMiddleware` en detalle
- [CORS](cors.md) - manejo de preflight, patrones de origen, alcance por
  ruta
- [Limitación de velocidad](rate-limiting.md) - `RateLimitMiddleware` /
  `ThrottleRequestsMiddleware` y `BackendErrorPolicy`
- [Enrutamiento](routing.md) - en qué se expanden `routes!`, `Router`, y
  `group(...)`

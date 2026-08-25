# Mapa de paridad con Laravel

El mapeo honesto, característica por característica, entre Laravel 13.x y
Suprnova. Úsalo cuando te preguntes "¿Suprnova tiene X?" y quieras una
respuesta de sí/no/dónde en una sola fila.

Las secciones reflejan el índice de la documentación de Laravel para que un
desarrollador de Laravel pueda escanear de arriba abajo. Dentro de cada
sección, las columnas son siempre las mismas:

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|

La columna **Estado** usa cuatro valores:

| Símbolo | Significado |
|---|---|
| **disponible** | Misma superficie, mismo comportamiento (a menudo los mismos nombres de método) |
| **diverge** | Mismo trabajo, forma distinta porque Rust permite tomar una mejor decisión |
| **aún no** | Planificado de verdad, todavía no existe en el código |
| **no, por diseño** | No se va a ofrecer - explicación en la columna Notas |

El capítulo correspondiente (cuando existe) está enlazado desde la columna
**Notas**.

Este es un mapa vivo. Suprnova ofrece toda la superficie de Laravel 13.x en
los 30 dominios documentados; los vacíos listados abajo son los vacíos
reales y actuales del framework tal como está publicado hoy.

## Conceptos de arquitectura

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Ciclo de vida de la solicitud | cadena `Application` → `Server` → `handle_request` | disponible | [Ciclo de vida de la solicitud](lifecycle.md) |
| Contenedor de servicios | `Container` + la fachada `App`, de tres capas (tarea / hilo / global) | diverge | Task-local para lo de cada solicitud, thread-local para los tests - [Contenedor de servicios](container.md) |
| Vinculación contextual (`when()->needs()->give()`) | Sin vinculaciones contextuales - una vinculación por trait y por capa del contenedor | no, por diseño | El contenedor se indexa por `TypeId` y no tiene reflexión en tiempo de ejecución con la que indexar una vinculación según "quién está preguntando". Compón de forma explícita: pasa la dependencia, o vincula un newtype distinto por consumidor. [Contenedor de servicios](container.md) |
| Proveedores de servicios | función `bootstrap()` + `#[service]`, `#[policy]`, `#[command]`, macros de observer | diverge | Sin clase de registro - el arranque es una sola función; las macros usan `inventory` para el registro en tiempo de compilación. [Arranque de la aplicación](bootstrap.md) |
| Fachadas | `App::get`, `Cache::*`, `Mail::*`, `Auth::*`, `Storage::*`, `Queue::*`, `Bus::*`, `Event::*`, `Notification::*`, `Gate::*`, `Schedule::*`, `DB::*`, `Vector::*` estáticas | disponible | La misma forma de llamada; las fachadas son tipos reales, no alias |
| Contratos | Traits - `Mailer`, `KeyValueStore`, `Hasher`, `Channel`, `VectorDriver`, `Evaluator`, `PaymentProvider`, etc. | disponible | Todas las costuras públicas viven en traits; vincula por trait y cambia de implementación libremente |

## Primeros pasos

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Instalación | `cargo install --git …suprnova-cli` y luego `suprnova new <name>` | disponible | [Instalación](installation.md) |
| Configuración | Configuración tipada vía `#[derive(Config)]` + `Config::register` | diverge | Tipada en tiempo de compilación en lugar de bolsas de arrays. [Configuración](configuration.md) |
| Desarrollo agéntico (IA) | Sin SDK de IA de primera clase en el framework | no, por diseño | Usa los crates que usarías de todos modos (`async-openai`, `anthropic-rs`, `tokenizers`, etc.) bajo `App::bind(Arc<dyn YourLlm>)` |
| Estructura de directorios | `src/{actions,bootstrap,controllers,middleware,models,routes}` | disponible | Misma intención, con una disposición idiomática de Rust. [Estructura de directorios](structure.md) |
| Frontend | Inertia v3 sobre Svelte 5 / React 19 / Vue 3.5 | disponible | [Frontend](frontend.md), [Componentes de página](frontend-pages.md), [Tipos de TypeScript](frontend-typescript-types.md) |
| Kits de inicio | **Nebula** (auth) y **Pulsar** (sitio de producto completo), además del andamiaje simple de `suprnova new` | disponible | Hoy se ofrecen dos kits - Nebula es el equivalente de Breeze; Pulsar añade docs, blog, comunidad y RBAC. [Kits de inicio](starter-kits.md) |
| Despliegue | Binario único; recetas de Docker / Railway / DO / Hetzner | diverge | Un solo artefacto, no un runtime de PHP + opcache + FPM. [Despliegue](deployment.md) |

## Lo esencial

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Definiciones de rutas | Macro `routes!` + `get!` / `post!` / `put!` / `patch!` / `delete!` / `any!` / `head!` / `options!` / `fallback!` / `ws!` | disponible | [Enrutamiento](routing.md) |
| Parámetros de ruta | Parámetros de ruta `{id}` + `req.param("id")` | disponible | Parámetros opcionales mediante `{id?}`; restricciones mediante `where!()` |
| Nombres de ruta | `.name("posts.show")` sobre la ruta + `url("posts.show", &[("id", "42")])` | disponible | [Generación de URLs](urls.md) |
| Grupos de rutas | Macro `group!` con `.prefix()` / `.middleware()` / `.name()` / `.controller()` | disponible | El middleware de grupo se aplana sobre cada ruta en el momento del registro |
| Rutas de recursos | `resource!("posts", PostController)` registra las 7 rutas estándar | disponible | `apiResource!`, `only(...)` y `except(...)` están todos soportados |
| URLs firmadas | `sign_url(...)`, `sign_route(...)`, `verify_signature(...)` | disponible | HMAC-SHA256 con `APP_KEY` |
| Vinculación de modelo de ruta | `#[handler]` extrae `Post` de `{post}` mediante la impl de `RouteBinding` | disponible | El derive `AutoRouteBinding` se implementa solo para los tipos `#[suprnova::model]` |
| Limitación de velocidad | Middleware `throttle:60,1` + `RateLimiter::for_signature` | disponible | [Limitación de velocidad](rate-limiting.md) |
| Middleware | Trait `impl Middleware`; se registra globalmente o por ruta | disponible | [Middleware](middleware.md) |
| Grupos y alias de middleware | `register_middleware_group`, `register_middleware_alias` | disponible | Se buscan por nombre de cadena en las rutas |
| Protección CSRF | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | disponible | La validación del token por sesión es el comportamiento por defecto. Las políticas opcionales `SameOriginOnly`, `AllowSameSite` y `OriginOnly` consultan `Sec-Fetch-Site`; la comprobación del origen no está activada por defecto. [CSRF](csrf.md) |
| Controladores | `#[handler] pub async fn show(req: Request) -> Response` | disponible | Los controladores son módulos de funciones libres, no clases. [Controladores](controllers.md) |
| Controladores de una sola acción | Un handler ya es una sola función; agrúpalos en módulos | disponible | La convención de Rust - sin la ceremonia de `__invoke` |
| Solicitudes | Struct `Request` con `.input()`, `.param()`, `.query()`, `.header()`, `.cookie()`, `.json()`, `.file()`, etc. | disponible | [Solicitudes](requests.md) |
| Solicitudes de formulario | `#[derive(Data, Validate, FormRequest)]` | disponible | La validación se ejecuta a medida que extraes |
| Subida de archivos | `req.file("avatar")?` devuelve `UploadedFile`; multipart en streaming con topes de tamaño y de partes | disponible | Vuelco automático a un archivo temporal por encima del umbral |
| Respuestas | Builders de `HttpResponse` + `json_response!()` / `text_response!()` / `Redirect::to` / respuestas de Inertia | disponible | [Respuestas](responses.md) |
| Respuestas en streaming (`eventStream`, `stream`, `streamJson`) | `HttpResponse::sse(...)` / `event_stream(...)` / `stream_bytes(...)` / `stream_json(...)` | disponible | Las mismas formas de respuesta que esperan los hooks de `@laravel/stream-{react,vue,svelte}`. [SSE](sse.md) |
| `withoutCookie` / `withoutCookies` | `.without_cookie(name)` / `.without_cookies([...])` sobre `HttpResponse`, `Response`, `Redirect` y `RedirectRouteBuilder` | disponible | `Cookie::forget_with(name, path, domain)` para una cookie que no se estableció en `/` |
| Vistas (Blade) | Páginas de Inertia renderizadas en el servidor (Svelte/React/Vue) - no hay equivalente de Blade | diverge | Inertia es la capa de vista. Usa [Componentes de página](frontend-pages.md) en lugar de Blade |
| Empaquetado de assets (Vite) | Vite 8 se incluye en todo proyecto con andamiaje; `suprnova serve` ejecuta Vite y el backend a la vez | disponible | Lectura del manifest + HMR conectados automáticamente |
| Assets estáticos (`public/`, servidos por el servidor web en Laravel) | Handler de respaldo en proceso `StaticFiles::public()` que sirve `public/` en la raíz web | disponible | `StaticFiles::from_dir(...)` + `cache_control(...)`; no hace falta un servidor web aparte |
| Generación de URLs | `url("posts.show", &[…])`, `route("posts.show", …)`, `redirect(...)`, `redirect_to(...)` | disponible | [Generación de URLs](urls.md) |
| Sesión | `session()`, `session_mut()`, flash bag mediante `req.flash()` | disponible | Respaldada por base de datos por defecto mediante `DatabaseSessionDriver`; la cookie cifrada del navegador lleva el identificador de sesión y los metadatos de actividad, no la bolsa de datos de la sesión. [Sesiones](session.md) |
| Cola de cookies (`Cookie::queue`) | `Cookie::queue`/`queued`/`unqueue`/`expire` - un almacén de cookies task-local que `SessionMiddleware` drena sobre la respuesta | disponible | Requiere `SessionMiddleware` en la cadena; se encolan por nombre, no por nombre+ruta como el `CookieJar` de Laravel |
| Validación | `#[derive(Validate)]` + 28 reglas integradas + los traits `Rule`/`ValueRule`/`AsyncRule` | disponible | `Url` usa la lista de permitidos de esquemas de Laravel y `Url::protocols([...])` refleja `url:http,https`. Las reglas asíncronas (por ejemplo, `Unique`) van a la base de datos. `ArrayKeys` y `Distinct` son `ValueRule`s sobre `serde_json::Value`, y se corresponden con `array:keys` y `distinct` de Laravel. [Validación](validation.md) |
| Regla `Password` (`Password::defaults()`, `uncompromised()`) | `Password::min(n)` + los builders de fortaleza (`.letters()`, `.mixed_case()`, `.numbers()`, `.symbols()`) + `.uncompromised()` | disponible | Comprobación con k-anonimato de Have I Been Pwned; falla abierto ante un error de red, igual que el `NotPwnedVerifier` de Laravel. [Validación](validation.md#password-strength) |
| Manejo de errores | `FrameworkError`, `AppError`, el trait `HttpError`, el límite de pánico en `execute_chain_safely` | disponible | [Manejo de errores](errors.md), [Modelo de errores](error-model.md) |
| Registro de eventos | Subscriber de `tracing` con campos estructurados, `LogFormat` (json / pretty / compact) | diverge | Una línea de log es un documento JSON; `request_id` está siempre presente. [Registro de eventos](logging.md) |
| Canales de log / drivers de archivo (`single`, `daily`, `monthly`, `stack`) | `tracing` escribe líneas estructuradas en stdout; la plataforma las rota y las transporta | no, por diseño | Los contenedores, systemd y cualquier transportador de logs ya hacen rotación y retención. Reimplementarlo dentro del proceso duplica la plataforma y le oculta los logs. [Registro de eventos](logging.md) |
| Ayudantes de aborto | `abort_if(cond, status, msg)`, `abort_unless(...)`, `abort_with(status, msg)` | disponible | La misma forma que la familia `abort_if` de Laravel |

## Profundizando

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Consola Artisan | Binario `console` por aplicación, construido a partir de `#[command]` + `#[derive(Command)]` | disponible | [Consola](console.md). `cargo run --bin console <subcomando>` |
| Tinker (REPL) | Sin REPL | no, por diseño | Escribe un script puntual `cargo run --bin xxx` o un `#[suprnova_test]` |
| Difusión | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | disponible | Dispersión con sea-streamer para varios nodos. [Difusión](broadcasting.md) |
| Caché | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`, `RedisCache` | disponible | Operaciones atómicas + caché con etiquetas + bloqueos de caché (`LockGuard`). [Caché](cache.md) |
| Colecciones | `eloquent::Collection<M>` con métodos con forma de Laravel | disponible | `Deref<Target = Vec<M>>`, así que los idiomas de Vec existentes siguen funcionando. [Colecciones de Eloquent](eloquent-collections.md) |
| Concurrencia | Tokio en todas partes - `tokio::spawn`, `tokio::join!`, `tokio::select!` | disponible | Todo el framework es async. La facade `Concurrency::run([...])` de Laravel no se incluye; Tokio es la respuesta |
| Contexto | `Context::put` / `Context::get` / `ContextStore` + inyección automática en la cola, el correo y los eventos | disponible | [Contexto](context.md) |
| Contratos | Todas las costuras públicas son traits | disponible | Consulta la fila "Arquitectura / Contratos" de más arriba |
| Eventos | `EventFacade::dispatch(e).await?`, `#[derive(Event)]`, `EventDispatcher`, oyentes encolados, subscribers | disponible | [Eventos](events.md) |
| Almacenamiento de archivos | `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` sobre OpenDAL | disponible | La misma superficie `put/get/delete/copy/move/exists/url`. Protección contra path traversal incorporada. [Sistema de archivos](filesystem.md) |
| Ayudantes | Los equivalentes viven en sus módulos de origen (no hay un `helpers.md` cajón de sastre) | diverge | Por ejemplo, los ayudantes de URL viven en [urls.md](urls.md), los de cadenas en `std`/`heck` y los de arrays en `std::collections` - Rust hace esto con crates, no con un espacio de nombres global |
| Cliente HTTP | Builder `Http::get/post/...` + `Http::fake(...)` para pruebas | disponible | Registra las solicitudes automáticamente; `assert_sent` / `assert_not_sent`; `.retry_when(predicate)` estrecha la política de reintentos integrada con un `RetryContext`. [Cliente HTTP](http-client.md) |
| Image (`Illuminate\Image`) | `Image::from_bytes/from_path/from_disk/from_upload/from_stream` + la misma superficie de operaciones y terminales | disponible | Vive en `suprnova::media`. Dos drivers, como el `gd`/`imagick` de Laravel: `IMAGE_DRIVER=oxideav` (por defecto, Rust puro) o `magick`. Lee y escribe PNG, JPEG, WebP, GIF y BMP; la salida AVIF queda aplazada a la publicación del codificador AV1 propio. Límites de decodificación comprobados en la cabecera. [Imágenes](images.md) |
| Decodificación de HEIC en el driver por defecto | `IMAGE_DRIVER=magick` en un anfitrión con el delegado libheif | no, por diseño | HEVC está sujeto a patentes y el único decodificador creíble en Rust puro tiene licencia dual AGPL/comercial, así que no se incluye ningún decodificador integrado. La misma forma que en Laravel, donde GD no puede leer HEIC en absoluto e Imagick necesita el delegado compilado tanto en el binario como en la extensión de PHP. [Imágenes](images.md#why-suprnova-diverges) |
| Localización | `Lang::get` / `get_with` / `try_get` / `has` + la macro `__!("key", name: value)` sobre catálogos Fluent `.ftl` en `lang/<locale>/`, detección con `LocaleMiddleware`, mensajes de validación traducidos y formateo con ICU4X | disponible | El mismo catálogo se sirve al navegador en `/_suprnova/lang/<locale>.ftl` y lo tipa `generate-types`. [Localización](localization.md) |
| Correo | `Mail::to(...).send(MyMail { ... }).await?` + los drivers `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory/file` | disponible | Trait `Mailable` + cuerpos HTML/texto renderizados con Tera; los envíos por SES llevan `TenantName` / `ConfigurationSetName` / `ListManagementOptions`; el despacho encolado se enruta con `.on_queue(...)` / `.on_connection(...)`, que mandan sobre `Queue::route`. [Correo](mail.md) |
| Notificaciones | `Notify::send(&user, notif).await?` + los canales `mail/database/broadcast/webpush` | disponible | Trait `Notifiable` + `Notification` por canal; el despacho encolado (`Notify::queue`) lleva `queue`/`timeout`/`fail_on_timeout`/`max_tries`/`backoff` por notificación hasta el job de cada canal, mediante la misma primitiva `EnvelopeOverrides` que usa Mail. [Notificaciones](notifications.md), [Web Push](web-push.md) |
| Desarrollo de paquetes | Crates adaptadores del workspace (por ejemplo, `suprnova-payments-stripe`) | disponible | La misma forma que los paquetes de Laravel: dependen del framework, se vinculan al contenedor y exponen macros si hace falta |
| Procesos (ejecutar comandos de shell) | `tokio::process::Command` de la biblioteca estándar | no, por diseño | Sin facade - la API de Tokio ya tiene la forma correcta |
| Cola | `Queue::push(job).await?` + los drivers `sync/memory/database/redis/null`, lotes, cadenas, `JobMiddleware`, `FailedJobStore` | disponible | [Cola](queues.md) |
| Retardo declarado por el job | `fn delay() -> Option<Duration>` en `Job`, respetado por `Queue::push` y `Queue::bulk` | disponible | Una llamada explícita a `Queue::push_later` / `Queue::later(delay, job)` siempre gana sobre el valor por defecto del propio job. [Cola](queues.md) |
| Evento de job único omitido | `queue::events::UniqueJobSkipped { job_name, unique_id, connection }` | disponible | Se dispara en el lado del push cuando `push_unique` deduplica; la llamada sigue devolviendo `Ok(false)` |
| Pausar colas (`queue:pause` / `queue:resume`) | `Queue::pause`/`resume`/`pause_all`/`resume_all`/`is_paused`/`paused_queues`, respaldados por caché, con los eventos `QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` | disponible | Una pausa por cola solo surte efecto en un worker arrancado con una lista `--queue=...` explícita; `resume_all` no limpia una pausa por cola. [Cola](queues.md) |
| Despacho posterior al commit (`afterCommit()`) | `fn after_commit() -> bool` en `Job`, `EnvelopeOverrides::after_commit` por push, `Queue::push_after_commit` | disponible | El push entero espera a la confirmación, eventos incluidos, y un rollback lo descarta; un `push_unique` diferido sigue tomando su bloqueo de inmediato para que la deduplicación funcione dentro de la transacción. Un `DB::begin_transaction` manual nunca difiere. [Cola](queues.md) |
| Conexión de cola con failover | `FailoverQueueDriver` sobre una lista ordenada de conexiones, mediante `QUEUE_DRIVER=failover` + `QUEUE_FAILOVER_CONNECTIONS` | disponible | Las escrituras recorren la lista; `pop`, los contadores y los listados se quedan en la primera conexión, así que cada respaldo necesita su propio worker. `QueueFailedOver` se dispara por flanco, y `bulk_push` recorre la lista sobre por sobre, de modo que cada uno conserva su propio retardo. [Cola](queues.md) |
| `ShouldBeUniqueUntilProcessing` | `fn unique_until_processing() -> bool` en `Job`, liberado tras la pasada de middleware y antes del handler | disponible | La liberación está acotada al propietario, así que un intento reentregado nunca libera el bloqueo de un despacho más nuevo. Un job que un middleware libera de vuelta a la cola conserva su bloqueo. [Cola](queues.md) |
| Inspección de la cola (`pendingJobs` / `delayedJobs` / `reservedJobs`) | `Queue::pending_jobs(queue)` / `delayed_jobs` / `reserved_jobs`, con un `Option<&str>` que colapsa en una sola llamada el gemelo `all*Jobs()` de Laravel | disponible | DTO `InspectedJob` (`id`/`queue`/`name`/`attempts`/`payload`/`created_at`); el valor por defecto del trait es un `Err` honesto en lugar de una colección vacía; `sync` y `null` lo sobrescriben con `Ok(vec![])`; el `reserved_jobs` de Redis es por consumidor. [Cola](queues.md) |
| Zona horaria por tarea en la programación | `.timezone(chrono_tz::Tz)` / `.try_timezone("name")` por tarea, valor por defecto `Schedule::timezone`, `schedule:list --timezone` | disponible | `chrono_tz::Tz` tipado en lugar de la cadena de Laravel; el valor por defecto para toda la programación es `Schedule::timezone` dentro de `schedule::register`, y no una clave de configuración `app.schedule_timezone`, y una tarea sin zona fijada conserva la zona local del proceso. [Programación de tareas](scheduling.md) |
| Limitación de velocidad | `RateLimiter::for_signature(...)`, `ThrottleRequestsMiddleware`, `RateLimitMiddleware` | disponible | Ventana deslizante mediante `SlidingWindowConfig`. [Limitación de velocidad](rate-limiting.md) |
| Búsqueda (Scout) | Sin adaptador propio de búsqueda de texto completo | aún no | Hoy se incluye la búsqueda vectorial mediante [Vector](vector.md); el equivalente a Scout para búsqueda por palabras clave está planificado |
| Cadenas (ayudantes) | Crate `heck` (conversiones de mayúsculas y minúsculas), `std::str`, `regex` | diverge | Los mismos crates que usa el resto del ecosistema de Rust; sin un `Str::camel($x)` global |
| Programación de tareas | `Schedule::call/command/task` + `#[derive(Task)]` + sintaxis cron + worker `schedule:run` | disponible | [Programación de tareas](scheduling.md) |
| Claves de idempotencia | `Idempotency::remember(key, ttl, body)` - protección contra repeticiones al estilo de Stripe | disponible | Quien llama pone la clave en un espacio de nombres con la ruta y la identidad de usuario o de negocio. [Idempotencia](idempotency.md) |
| Tiempo de espera de la solicitud | `TimeoutMiddleware` configurable por ruta | disponible | Nativo de Rust - aborta el future en vuelo y libera el worker. [Tiempos de espera](timeout.md) |
| Indicadores de características (Pennant) | `Feature` + `Evaluator` + `FeatureMiddleware` + CRUD de administración | disponible | Propagación en menos de un segundo mediante el trait `FeatureSync`. [Indicadores de características](feature-flags.md) |
| Observabilidad (Pulse) | OpenTelemetry mediante `init_telemetry`, `Metrics` y `tracing` en todas partes | diverge | OTel es la lingua franca de la observabilidad en Rust: apunta tu colector al binario. [Observabilidad](observability.md) |
| Telescope (panel de depuración) | Todavía sin equivalente | aún no | Aplazado a v2+; la salida de tracing y OTel del framework cubre la mayoría de las necesidades de diagnóstico |
| Pulse (panel de rendimiento) | Todavía sin equivalente | aún no | Igual que Telescope: expón las métricas con tu stack de observabilidad actual hasta que llegue un panel |
| Búsqueda vectorial | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | disponible | Sin la barrera de "solo pgvector de Postgres". [Búsqueda vectorial](vector.md) |

### Exclusivo de Suprnova (sin equivalente en Laravel)

| Suprnova | Qué es | Notas / enlace |
|---|---|---|
| Macro `ws!()` + handlers de WebSocket | Rutas WS tipadas que comparten el router y la pila de middleware | [WebSockets](websockets.md) |
| Flujos de trabajo | Trabajo con estado y de larga duración, con reintentos, esperas y fronteras de paso | [Flujos de trabajo](workflows.md) |
| Supervisores | Trait `Supervisor` con reinicio automático y captura de pánicos para tareas de Tokio de larga vida | [Supervisores](supervisors.md) |
| Web Push (VAPID) | Notificaciones push del navegador como canal de primera clase | [Web Push](web-push.md) |
| División lectura/escritura multiconexión | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [Base de datos](database.md) |
| HTTP/2 y WebSocket sobre el mismo socket | `hyper.with_upgrades()` en `Server::run` | [Ciclo de vida de la solicitud](lifecycle.md) |
| Contenido Markdown + pipeline de documentación | `MarkdownRenderer` (comrak saneado → syntect → ammonia) + `build_docs(DocsBuildConfig)` → un `DocsCatalog` de `DocsChapter`s con búsqueda | Extracción de encabezados + `slugify_heading`; da soporte a la documentación y el blog en Markdown sin un generador de sitios estáticos aparte |

## Seguridad

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Autenticación | `Auth::user/check/login/logout/attempt`, trait `Authenticatable`, `Guard` por nombre | disponible | [Autenticación](authentication.md) |
| Varios guards | `Guard` registrado por nombre (`web`, `api`, …) mediante `AuthManager` | disponible | `SessionGuard`, `TokenGuard` e impls propias |
| Proveedores de usuarios | `EloquentUserProvider<U>`, `DatabaseUserProvider`, y propios mediante el trait `UserProvider` | disponible | [Flujos de autenticación](auth-flows.md) |
| Verificación de correo | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail`; contrato `MustVerifyEmail` | disponible | Respaldada por el proveedor y ligada al actor - [Flujos de autenticación](auth-flows.md) |
| Restablecimiento de contraseña | `PasswordReset` + la transacción de primera prueba por correo de Magnetar o el respaldo de un `UserProvider` verificado + correos de restablecimiento y cambio | disponible | Magnetar se encarga de la primera prueba atómica; las aplicaciones respaldadas por un proveedor pueden restablecer a usuarios ya verificados - [Flujos de autenticación](auth-flows.md) |
| Limitación de fuerza bruta | Motor de bloqueo de Magnetar + `BruteForce` + `LoginThrottleMiddleware` | disponible | Bloqueo de cuenta más la limitación por IP y por ruta del framework |
| Dos factores (TOTP) | Facade de compatibilidad `TwoFactor` del framework más el motor de factores de Magnetar | disponible | Códigos de recuperación, protección contra repeticiones e inicio de sesión integrado con puerta de factores |
| Recordarme | Credencial rotatoria de Magnetar ligada a un propósito, detrás de la cookie del framework | disponible | Comprobaciones de época de autenticación, rotación, tratamiento de anomalías y respaldo heredado |
| OAuth (Socialite) | Registro de proveedores de Magnetar y facade `Auth::oauth(provider)` | disponible | OAuth, `form_post` de Apple, vinculación de PKCE/state y política de identidad verificada - [OAuth](oauth.md) |
| Sanctum (tokens de API) | `BearerTokenMiddleware` sobre las sesiones bearer de Magnetar | diverge | Autentica sesiones bearer; no hay una API de gestión de tokens al estilo de Sanctum |
| Passport (servidor OAuth) | Motores de protocolo y de plugins de Magnetar | diverge | Se incluyen las primitivas del motor; no hay una facade de aplicación compatible con Laravel Passport |
| Fortify (backend de autenticación) | Las facades `Auth`/`auth_flows` del framework sobre los motores de Magnetar | disponible | El framework es dueño del HTTP, el correo, los eventos, las cookies y la vinculación con la aplicación |
| Autorización (políticas / compuertas) | `Gate::allows/denies` + `#[policy] impl PostPolicy` + trait `Authorizable` + registro por macro + `Gate::default_denial_response` | disponible | [Autorización](authorization.md) |
| Roles y permisos (spatie/laravel-permission) | Trait `HasRoles` + tablas `roles` / `permissions` / `role_has_permissions` (`CreateRbacTables`) + `RoleMiddleware` / `PermissionMiddleware` (fallan cerrado) | disponible | De primera parte, no un paquete de la comunidad. Ayudantes `create_role` / `give_permission_to_role` / `assign_role_to_model`; se apoya sobre Gate y Policy. [Autorización](authorization.md) |
| Cifrado | `Crypt::encrypt/decrypt` + vinculación AAD con `CryptPurpose` | disponible | AES-256-GCM, rotación de claves mediante `APP_KEY_PREVIOUS`. [Cifrado](encryption.md) |
| Hashing | `hash::*` + `BcryptHasher`, `Argon2idHasher`, `Argon2iHasher`, `needs_rehash`, `is_hashed`, `verify` | disponible | Bcrypt por defecto; argon2id disponible. [Hashing](hashing.md) |

## Base de datos

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| DB::table('users')->where(...)->get() | `DB::table("users").db_where("id", "=", 1).get().await?` | disponible | [Base de datos](database.md), [Constructor de consultas](queries.md) |
| Múltiples conexiones | `DB::on("read")` + `ConnectionRegistry` | disponible | División lectura/escritura de primera clase |
| Transacciones | `DB::transaction(\|tx\| async move { ... }).await?` | disponible | Savepoints + reintento ante deadlock |
| Eventos de consulta | `QueryListener` + evento `QueryExecuted` | disponible | `DB::listen(\|q\| { ... })` |
| Expresiones raw | `DB::raw("...")`, `DB::select("...", &[...])` | disponible | Requiere vinculación de parámetros (sin interpolación de strings) |
| Postgres / MySQL / SQLite | Los tres de primera clase vía SeaORM | disponible | Detección por URL en `database::config::database_type()` |
| MariaDB | De primera clase como opción propia (vector + JSON + temporal) | diverge | Se trata por separado por las características multiparadigma que Laravel solo ofrece para Postgres |
| Redis | Usado por los drivers (cache/queue/rate-limit) - sin una fachada `Redis::*` aparte | diverge | Recurre directamente al crate `redis` cuando necesites comandos ad hoc; cache/queue/rate-limit cubren el 95% del uso habitual |
| MongoDB | Todavía sin adaptador de primera parte | aún no | Usa el crate `mongodb` directamente vía `App::bind` |
| Constructor de consultas | `Builder<M>` con `db_where` / `or_where` / `where_in` / `where_between` / `where_null` / `where_has` / `with` / `with_count` / `order_by` / `group_by` / `having` / `paginate` / etc. | disponible | [Constructor de consultas](queries.md) |
| Paginación | `LengthAwarePaginator`, `Paginator` (simple), `CursorPaginator` | disponible | Las tres serializan a JSON con forma de Laravel. [Paginación](pagination.md) |
| Migraciones | `#[derive(DeriveMigrationName)] struct M;` + `up`/`down` + `Migrator` | disponible | Se ejecutan vía `suprnova migrate`/`migrate:rollback`/`migrate:status`/`migrate:fresh`. [Migraciones](migrations.md), [Migraciones de CLI](cli-migrations.md) |
| Sembradores | trait `Seeder` + subcomando `db:seed` | disponible | Factories por modelo. [Siembra de datos](seeding.md) |

## Eloquent ORM

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| `class User extends Model` | `#[suprnova::model(table = "users")] struct User { ... }` | disponible | El struct ES el `Model` de SeaORM. [Eloquent](eloquent.md) |
| Find / first / get | `User::find(id)`, `User::query().first()`, `User::all()`, `Builder::get` | disponible | Todo async |
| Create / update / delete | `User::create(attrs)`, `user.update(attrs)`, `user.delete()` | disponible | La macro `attrs! { name: "...", email: "..." }` para attrs parciales |
| Salvaguardas de asignación masiva | `#[model(fillable = [...])]` / `#[model(guarded = [...])]` + scope `unguarded \|\| { ... }` | disponible | `prevent_silently_discarding_attributes()` para el modo estricto |
| Eliminaciones suaves | `#[model(soft_deletes)]` inyecta automáticamente `deleted_at` + el trait `SoftDeletes` | disponible | `with_trashed()`, `only_trashed()`, `restore()`, `force_delete()` |
| Prunable / MassPrunable | `#[prunable] impl Prunable for User { ... }` + worker `model:prune` | disponible | Anclado en cascada a las relaciones |
| Timestamps | `created_at`/`updated_at` automáticos si las columnas están presentes | disponible | Se desactivan vía `#[model(timestamps = false)]` |
| Tipos de clave primaria | i64 por defecto; UUID / ULID vía `#[model(unique_id = "uuid")]` o `unique_id = "ulid"` | disponible | Genera el id automáticamente al insertar |
| Scopes locales | `#[scopes(User)] impl User { fn active(b: &mut Builder<User>) { ... } }` | disponible | Despacho de métodos sobre `Builder<M>` |
| Scopes globales | `impl GlobalScope for ActiveOnly { ... }` + registro | disponible | Se eliminan vía `Builder::without_global_scope` |
| Relaciones (11 tipos) | `HasOne`, `HasMany`, `BelongsTo`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`, `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany` | disponible | Enum morph por familia. [Relaciones de Eloquent](eloquent-relationships.md) |
| Carga anticipada | `User::query().with(&["posts", "posts.comments"]).get()` | disponible | `EagerLoadDispatch` está sellado; solo las relaciones generadas por macro pueden implementarlo |
| Prevención de carga perezosa | `prevent_silently_discarding_attributes(true)` | disponible | Misma forma que `preventLazyLoading` de Laravel |
| Agregados sobre relaciones | `with_count("posts")`, `with_sum("orders", "total")`, `with_avg`, `with_min`, `with_max` | disponible | Una sola subconsulta por agregado |
| `whereHas` / `whereDoesntHave` | `where_has("posts", \|q\| q.db_where("published", "=", true))` | disponible | Motor EXISTS correlacionado |
| `loadMissing` | `user.load_missing(&["posts"]).await?` | disponible | Opera sobre toda la colección |
| Clonar un registro | `user.replicate()` / `user.replicate_into::<OtherType>()` | disponible | Despacha el evento `Replicating` |
| Touch del padre (timestamps) | `#[model(touches = ["post"])]` | disponible | Un `UPDATE` por propietario `BelongsTo`, un solo nivel y sin eventos (sin recursión al abuelo ni evento `saved` del padre). `without_touching` / `without_touching_on::<M, _, _>()` para omitirlo. [Actualizar propietarios](eloquent.md#parent-touching) |
| Observers | `impl Observer<User>` + `#[suprnova::observer(User)]` | disponible | 16 eventos de ciclo de vida |
| 16 eventos de ciclo de vida | `Created`, `Creating`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Retrieved`, `Replicating`, `ForceDeleting`, `ForceDeleted`, `Pruning` | disponible | Submódulo `events::*` por modelo. `EventResult::cancel(_)` hace cortocircuito con un 400 |
| Mutadores / Accesores | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | disponible | [Conversiones, accesores y mutadores de Eloquent](eloquent-mutators.md) |
| Casts (22 integrados) | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | disponible | Implementa `Cast` para uno personalizado |
| Colecciones | `Collection<M>` con `pluck`, `filter`, `map`, `each`, `chunk`, `groupBy`, `keyBy`, `sort_by`, `where_`, `first`, `last`, `count`, `is_empty`, `to_array` y los equivalentes de Laravel; `Deref<Target = Vec<M>>` para que todos los modismos de `Vec` sigan funcionando | disponible | [Colecciones de Eloquent](eloquent-collections.md) |
| `modelKeys()` | `Builder::model_keys().await?` (sin hidratar, clave calificada) y `Collection::model_keys()` | disponible | Ambos devuelven `Vec<M::Key>`; el terminal del builder proyecta `users.id` para sobrevivir a los JOIN |
| API Resources | `#[derive(Resource)]` + `IntoJsonResource` + `JsonApiResponse` + fieldsets + includes | disponible | Están disponibles tanto la forma JSON:API como la forma de resource al estilo Laravel. [Recursos JSON:API](eloquent-resources.md) |
| Serialización | `#[model(hidden = [...], visible = [...], appends = [...])]` | disponible | Mismo control sobre qué atributos se serializan. [Serialización de Eloquent](eloquent-serialization.md) |
| Factories | `#[derive(Factory)] struct UserFactory` + `UserFactory::new().count(5).create().await?` (o `UserFactory::times(5).create_many().await?`) | disponible | `Sequence` para ciclar valores. [Fábricas de Eloquent](eloquent-factories.md) |
| Ciclo de vida: chunking / lazy / cursor | `Builder::chunk(n, \|page\| async { ... })`, `lazy()`, `cursor()` | disponible | Iteración con memoria acotada sobre tablas grandes |
| Bloqueo pesimista | `Builder::lock_for_update()`, `shared_lock()` | disponible | Dentro de una transacción |
| Familia `whereJsonContains` | Disponible vía las expresiones de columna de SeaORM (dependientes del driver) | disponible | La sintaxis exacta difiere según el backend; hay ayudantes para los casos comunes |

## Paginación

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| `LengthAwarePaginator` | `LengthAwarePaginator` (page + total + per_page + last_page) | disponible | `Builder::paginate(n).await?` |
| `Paginator` (simple) | `Paginator` (page + per_page + has_more, sin count) | disponible | `Builder::simple_paginate(n).await?` |
| `CursorPaginator` | `CursorPaginator` (token de cursor opaco + dirección) | disponible | `Builder::cursor_paginate(n).await?`; determinista para el scroll infinito |
| Integración con Inertia | trait `IntoInertiaScroll` + `ScrollMetadata` | disponible | Se conecta directamente con `WhenVisible` / `merge` de Inertia |

## IA (Laravel la ofrece de forma nativa hoy; nosotros no ponemos barreras)

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| SDK de IA | Sin SDK de IA de primera parte | no, por diseño | Trae el crate que ya uses (`async-openai`, `anthropic-sdk`, `ollama-rs`, `tokenizers`, etc.) y vincúlalo bajo `App` |
| MCP (Model Context Protocol) | Sin adaptador de servidor MCP de primera parte | no, por diseño | Los crates de MCP en Rust (`mcp-rs`, `mcp-sdk-rust`) encajan bien bajo la superficie existente de routing / supervisor |
| Boost (agente de codificación de Laravel) | n/a | no, por diseño | Fuera del alcance del framework |

## Pruebas

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| `php artisan test` | `cargo test` | disponible | [Pruebas](testing.md) |
| Estilo Pest / PHPUnit | `#[suprnova_test]` (consciente de async) + aserciones `expect!()` al estilo Jest + macros BDD `describe!()` / `test!()` | disponible | Las tres funcionan de forma intercambiable |
| Tests de feature (HTTP) | Conducen `handle_request(router, registry, req)` en el mismo proceso, normalmente mediante una conexión loopback de hyper para que el servidor reciba un cuerpo `Incoming` real | disponible | [Pruebas HTTP](http-tests.md) |
| Envoltorio `TestResponse` | `suprnova::testing::TestResponse`: `assert_status` / `assert_json_path` / `assert_cookie` / `assert_session_has` fluidos y encadenables con `&Self` | disponible | [Pruebas HTTP](http-tests.md#fluent-response-assertions-with-testresponse) |
| Ayudantes de testing de Inertia | `suprnova::testing::AssertableInertia`: `component` / `url` / `version` / `prop` / `has` / `missing` / `where_` / `count` / `has_flash`, además de `reload_only` / `reload_except` / `load_deferred_props` con un cierre `with_reload` proporcionado por quien llama | disponible | [Pruebas HTTP](http-tests.md#testing-inertia-responses) |
| Tests de consola | Ejecuta `dispatch_argv(["console", "..."])` y afirma | disponible | La misma forma que los tests HTTP, para el binario de consola |
| Tests de navegador (Dusk) | n/a en el framework - usa Playwright / WebdriverIO / el agente de navegador `gstack` | no, por diseño | Ya existe herramienta multilenguaje; no la reinventamos |
| Tests de base de datos | `TestDatabase::fresh::<Migrator>()` | disponible | Crea una base de datos SQLite en memoria nueva por test, aplica las migraciones, la registra en el contenedor de tests y descarta esa base de datos y el estado aislado del contenedor al destruirse; no envuelve cada test en una transacción de rollback. [Pruebas de base de datos](database-testing.md) |
| Simulación y fakes | Fakes por fachada: `MailFake`, `NotifyFakeGuard`, `EventFakeGuard`, `Queue::fake`, `Bus::fake`, `Http::fake`, `Storage::fake` | disponible | Llamadas grabadas + ayudantes de aserción. [Simulación y falsificaciones](mocking.md) |
| UUID de jobs de `QueueFake` | `queue::testing::pushed_with_id::<J>()` | disponible | El fake asigna un id de envelope en cada push y emite el mismo `JobQueued` que un push real |
| Viaje en el tiempo | `tokio::time::{pause, advance, resume}` del runtime de la biblioteca estándar | disponible | No ofrecemos el nuestro - la API de Tokio ya lo hace |
| Aislamiento del contenedor | `TestContainer::fake(\|tc\| tc.bind(...))` - thread-local | diverge | Seguro en paralelo por construcción. [Contenedor de servicios](container.md) |

## Pagos (el Cashier de Laravel; el nuestro es genérico por proveedor)

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Cashier (Stripe) | crate adaptador `suprnova-payments-stripe` detrás de los traits genéricos `Payment` / `Subscription` / `CustomerStore` / `WebhookHandler` | diverge | Superficie genérica, adaptador concreto. [Pagos](payments.md), [Adaptador de Stripe](payments-stripe.md) |
| Cashier (Paddle) | adaptador `suprnova-payments-paddle` | diverge | Flujo Merchant-of-Record + sin impl directo de `Payment` (Paddle controla el gateway). [Adaptador de Paddle](payments-paddle.md) |
| Proveedor personalizado | Implementa `PaymentProvider` + `SessionPayload` + `WebhookHandler` | disponible | [Escribir un adaptador de proveedor de pagos](payments-provider-guide.md) |
| Componentes de checkout de Inertia | Bucles de despacho documentados para Svelte / React / Vue sobre `SessionPayload.flow` | disponible | [Pagos - Integración de Frontend](payments-frontend.md). Páginas de facturación ya hechas están planificadas como una futura incorporación a los kits de inicio ([Kits de inicio](starter-kits.md)) |
| Ciclos de vida de suscripción | `Subscription::subscribe / update / cancel / get` (donde el proveedor los soporte) | disponible | Se devuelve `NotSupported` donde el proveedor no los soporta (p. ej. `subscribe` de Paddle y el reemplazo de conjuntos de precios) |
| Idempotencia de webhooks | tabla de copia local `payments_webhook_events` con `UNIQUE(provider, provider_event_id)` | disponible | Protección contra repetición al estilo Stripe |
| Tablas de copia local | `payments_customers`, `payments_payment_methods`, `payments_subscriptions`, `payments_subscription_items`, `payments_transactions`, `payments_webhook_events` | disponible | Columna JSONB `provider_metadata` en cada una para campos específicos del adaptador |

## Frontend (Laravel tiene Blade + kits de inicio; nosotros tenemos Inertia)

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Blade | n/a - Inertia es la capa de vista | diverge | [Frontend](frontend.md) |
| Inertia.js | De primera clase: v3 sobre Svelte 5 / React 19 / Vue 3.5 | disponible | [Respuestas de Inertia](frontend-inertia-responses.md), [Componentes de página](frontend-pages.md) |
| `Route::inertia($uri, $component, $props)` | `Router::inertia(path, component, props)` | disponible | Devuelve un `RouteBuilder`, por lo que se pueden encadenar `.name(...)` / `.middleware(...)`; `Router::view` es el alias anterior |
| Resolución de la URL de página (`Inertia::resolveUrlUsing`) | `page.url` es ruta + query; sobrescríbelo con `InertiaConfig::url_resolver` | disponible | La derivación por defecto coincide byte a byte con el `X-Inertia-Location` del middleware de versión; un `url_resolver` solo cambia `page.url` |
| Middleware del protocolo de Inertia (`Vary`, respuesta vacía, rebote de versión) | `InertiaHeadersMiddleware` + `InertiaVersionMiddleware` + `Inertia303Middleware` - tres de los cuatro middlewares que conecta `Inertia::install` (el cuarto, la redirección por error de validación, es la fila siguiente) | disponible | `Vary: X-Inertia` en todas las respuestas; un `200` vacío en una visita de Inertia se convierte en un `303` de vuelta; el rebote 409 vuelve a poner la sesión en flash |
| Redirección externa + limpieza del historial | `InertiaResponse::location_for(&req, url)`, `App::clear_history()` | disponible | `location_for` es `409` para XHR y `302` para una navegación completa; `App::clear_history()` sobrevive a la redirección de cierre de sesión |
| Redirección por error de validación (`Middleware::resolveValidationErrors`, `$withAllErrors`) | `InertiaValidationRedirectMiddleware`, conectado por `Inertia::install`; `InertiaConfig::with_all_errors(bool)` | disponible | Un `422` en una visita de Inertia se convierte en un `303` de vuelta con los errores en flash; el valor de un campo se reduce a su primer mensaje salvo con `with_all_errors(true)`. [Respuestas de Inertia](frontend-inertia-responses.md#validation-failures) |
| `Inertia::share` / `getShared` / `flushShared` | `App::inertia_share` / `_lazy` / `_once`, `App::inertia_shared(key)`, `App::flush_inertia_shared()` | disponible | El anidamiento de claves con puntos sigue la semántica de `Arr::set`; `InertiaSharedData::share(&req, component)` por solicitud puede variar según la página. Una compartición con puntos permanece plana hasta el paso de desempaquetado de la respuesta, por lo que `only`/`except` coinciden con una entrada antecesora (`only: ['auth']` llega a `auth.user`) donde Laravel obtiene el mismo resultado de `Arr::set` al compartir |
| Recargas parciales | `#[derive(Data)]` + `req.includes("subset")` + el protocolo de recarga parcial de Inertia | disponible | Conjuntos de include con seguridad de tipos. `?include=` controla cada variante lazy, incluida `lazy(deferred)`, y se ejecuta antes que `X-Inertia-Partial-Data`, por lo que un include no permitido aún devuelve 400. `errors` está exento de `only`/`except`, igual que el share `Inertia::always` de Laravel |
| Props diferidos | `.defer(…)` / `.defer_with(…, DeferOptions)`, o `Prop::…defer()` | disponible | Protocolo de props diferidos de Inertia v3; `DeferOptions` lleva el grupo y la señal de rescate. `deferredProps` se envía solo en la visita inicial - `resolveDeferredProps` devuelve `[]` en cualquier partial coincidente |
| Props de fusión | `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with(MergeStrategy)` / `.merge_lazy` / `.merge_lazy_with`, o `Prop::…merge().merge_with_path(...)` | disponible | Protocolo de fusión de Inertia v3; `match_on` acepta un campo o varios; `merge_with_path` fusiona un campo anidado en lugar de la raíz del prop |
| Composición de props (`defer()->merge()`, `merge()->once()`, `optional()->once()`) | Builder de flags de `Prop` + `InertiaResponse::prop(key, prop)` | disponible | `Prop` es un struct de flags ortogonales que refleja las interfaces `Deferrable` / `Mergeable` / `Onceable` del adaptador PHP |
| Cifrar el historial | `EncryptHistoryMiddleware` | disponible | Historial cifrado en reposo en el cliente |
| Posición de scroll | `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.paginate` + `ScrollMetadata` / `ProvidesScrollMetadata` | disponible | Restauración automática al navegar; `reset` lee `X-Inertia-Reset`, igual que `resolveScrollProps` |
| Tipos de TypeScript | `suprnova generate-types` lee `#[derive(InertiaProps)]` y emite `.d.ts` | disponible | [Tipos de TypeScript](frontend-typescript-types.md) |
| Lectura del manifiesto de Vite | Cableada automáticamente vía `InertiaConfig::manifest_path` | disponible | HMR en dev, assets con hash en producción. `Inertia::install` falla cerrado en producción cuando falta el manifiesto |
| Versión de assets desde el manifiesto de compilación | Configuración predeterminada de `InertiaConfig`: `VersionResolver::from_manifest(manifest_path)` | disponible | Hash de los bytes del manifiesto; fallback estático `"1.0"` cuando no hay una compilación que hashear |
| SSR de Inertia (`inertia:start-ssr`) | `InertiaConfig::ssr(...)` en la configuración que se pasa a `Inertia::install`, worker lanzado por `suprnova ssr:start` | disponible | Worker fuera de proceso sobre loopback HTTP; recae en CSR ante un error o timeout salvo con `ssr_throw_on_error(true)`. `InertiaConfig::ssr_bundle_path(...)` condiciona el despacho a que el bundle compilado exista en disco (refleja `ensure_bundle_exists`), activado con `.ssr_ensure_bundle_exists(bool)` (activado por defecto una vez configurada una ruta de bundle); `suprnova new` genera `frontend/src/ssr.{ts,tsx}` y un script `build:ssr` para cada starter; `suprnova ssr:check` verifica la ruta `GET /health` del worker. [Respuestas de Inertia](frontend-inertia-responses.md) |

## CLI

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| `php artisan` | Binario `console` por app, construido con macros `#[command]` | disponible | [Consola](console.md), [Descripción general de CLI](cli.md) |
| `make:controller` / `make:model` / etc. | `suprnova make:controller / make:middleware / make:action / make:error / make:inertia / make:migration / make:task` | disponible | [Generadores de código](cli-generators.md) |
| `serve` | `suprnova serve` (backend + servidor de dev de Vite juntos) | disponible | [suprnova serve](cli-serve.md) |
| familia `migrate` | `suprnova migrate / migrate:rollback / migrate:status / migrate:fresh` | disponible | [Migraciones de CLI](cli-migrations.md) |
| `db:seed` | `cargo run --bin console db:seed` (vía el console por app) | disponible | Los sembradores se registran vía el trait `Seeder` |
| `schedule:run` / `schedule:work` / `schedule:list` | Mismos nombres vía el binario console por app | disponible | [Comandos de programación](cli-scheduling.md) |
| `queue:work` | Mismo nombre vía el binario console por app | disponible | Apagado ordenado ante SIGTERM/SIGINT |
| `tinker` | Sin REPL | no, por diseño | Ver la fila en "Profundizando" |

## Despliegue

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| `php artisan optimize` | `cargo build --release` | diverge | Un binario, sin paso de opcache |
| `php artisan config:cache` | La config tipada ya está comprobada en tiempo de compilación | diverge | No hay caché en tiempo de ejecución que invalidar |
| `php artisan route:cache` | Las rutas se expanden por macro en tiempo de compilación | diverge | El router se construye en el arranque a partir de rutas ya tipadas |
| Envoy (despliegues por SSH) | Usa cualquier orquestador - Docker, systemd, Kubernetes, fly.io, Railway | no, por diseño | El binario es el artefacto de despliegue |
| Forge / Vapor | No es cosa nuestra ofrecerlo - pero las recetas de Railway, DO y Hetzner cubren el mismo trabajo | diverge | [Despliegue](deployment.md), [Railway](deployment-railway.md), [Digital Ocean](deployment-digital-ocean.md), [Hetzner](deployment-hetzner.md) |
| Modo de mantenimiento (`php artisan down` / `up`) | `./app down` / `./app up` - secreto de bypass, rutas de retry/message/except personalizadas, driver `file` o `cache` | disponible | [Despliegue](deployment.md) |
| Horizon (panel de colas) | Todavía sin panel | aún no | Hasta entonces, inspección de jobs fallidos vía `cargo run --bin console queue:failed` |

## Paquetes (los paquetes oficiales de Laravel - los nuestros se ofrecen en el core, como adaptadores, o son vacíos deliberados)

| Paquete de Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Cashier (Stripe) | `suprnova-payments-stripe` | disponible | Genérico + adaptador. [Pagos](payments.md) |
| Cashier (Paddle) | `suprnova-payments-paddle` | disponible | Flujo MoR. [Pagos](payments.md) |
| Dusk | n/a | no, por diseño | Ya existen herramientas de navegador multilenguaje (Playwright, etc.) |
| Envoy | n/a | no, por diseño | Los contenedores / systemd / orquestadores hacen el trabajo |
| Fortify | Reemplazado por `auth_flows` | disponible | Mismo trabajo, integrado. [Flujos de autenticación](auth-flows.md) |
| Folio | n/a - el enrutamiento basado en páginas no es idiomático en Rust | no, por diseño | Usa `routes!` para enrutamiento explícito |
| Homestead | n/a - usa Docker / DevContainers | no, por diseño | [Receta de Docker](cli-docker.md) |
| Horizon | n/a por ahora | aún no | Los jobs fallidos se exponen vía el console por app |
| Mix | Reemplazado por Vite | diverge | Vite viene incluido en todo andamiaje |
| Octane | n/a - ya somos Tokio de larga duración | no, por diseño | Un solo binario, siempre caliente, sin FPM que sustituir |
| Passport | n/a por ahora | aún no | Ejecuta un IdP dedicado detrás de Suprnova hasta que se publique |
| Pennant (indicadores de características) | Reimplementado como `features::*` | disponible | [Indicadores de características](feature-flags.md) |
| Pint (estilo de código PHP) | `cargo fmt` + `cargo clippy` | diverge | Toolchain estándar de Rust |
| Precognition | Solicitudes precognitivas de Inertia vía recargas parciales + los mismos tipos `#[derive(Data, Validate, FormRequest)]` | disponible | Las dos mitades de Precog (validación temprana + recarga ligera) surgen ambas de Inertia v3 + los form requests |
| Prompts (UI de CLI) | Usa el crate `dialoguer` / `inquire` cuando lo necesites | no, por diseño | El ecosistema de Rust ya cubre esto |
| Pulse | n/a por ahora | aún no | OTel hoy, panel más adelante |
| Reverb (servidor WebSocket) | Integrado en Suprnova (`ws!()` + `BroadcastHub`) | diverge | No hace falta un servidor aparte - es el mismo proceso |
| Sail (dev con Docker) | `suprnova-cli` incluye recetas de Docker integradas | disponible | [Docker](cli-docker.md) |
| Sanctum | `BearerTokenMiddleware` sobre sesiones bearer Magnetar | diverge | No hay superficie de paquete separada ni gestión de tokens personales |
| Scout (búsqueda de texto completo) | n/a por ahora | aún no | La búsqueda vectorial ya está disponible ([Vector](vector.md)); un equivalente de Scout por palabras clave llegará después |
| Socialite | Registro de proveedores Magnetar y `Auth::oauth(provider)` | disponible | [OAuth e inicio de sesión sin contraseña](oauth.md) |
| Telescope | n/a por ahora | aún no | Tracing + OTel cubren el vacío de diagnóstico hasta que se publique un panel |
| Valet | n/a - las apps de Rust se ejecutan directamente | no, por diseño | `suprnova serve` es el runner de dev |

## Macros (superficie específica de Rust; análogos más cercanos en Laravel, para contexto)

Suprnova ofrece un amplio conjunto de proc-macros que no tienen un análogo
en Laravel, porque Laravel no tiene macros - tiene reflexión en tiempo de
ejecución. Se incluyen aquí para que no te las pierdas.

| Macro | Idea más cercana en Laravel | Qué hace |
|---|---|---|
| `#[suprnova::model]` | `extends Model` | Genera la entidad de SeaORM + implementa el trait `Model` |
| `#[suprnova::observer(M)]` | `User::observe(UserObserver::class)` | Registra un impl de `Observer<M>` vía `inventory` |
| `#[scopes(M)]` | Scopes locales en un modelo | Añade métodos a `Builder<M>` |
| `#[accessor]` / `#[mutator]` | Accesores / mutadores de Eloquent | Hooks de get/set a nivel de campo |
| `#[handler]` | `__invoke` de controlador | Autoextrae parámetros tipados desde `Request` |
| `#[command]` / `#[derive(Command)]` | Clase de comando de Artisan | Registra un subcomando de console |
| `#[policy]` | Clase de policy | Registra un impl de `Policy` vía `inventory` |
| `#[service(T)]` | `register` de un service provider | Vincula `T` en el contenedor |
| `#[injectable]` | Inyección por constructor | Genera un constructor respaldado por `App::make` |
| `#[derive(InertiaProps)]` | Props de Inertia | Codegen de TypeScript + serialización de Inertia |
| `#[derive(Data)]` | DTO de solicitud | Extraíble desde `Request`, con soporte de conjuntos de include |
| `#[derive(FormRequest)]` | Clase `FormRequest` | Validación + compuerta de auth + transformación |
| `#[derive(Factory)]` | Factory de modelo | Generación de datos de test respaldada por Faker |
| `#[derive(Resource)]` | API Resource | Serialización JSON:API + con forma de Laravel |
| `#[workflow]` / `#[workflow_step]` | n/a en Laravel | Trabajo con estado de larga duración |
| `routes!` + `get!` / `post!` / `ws!` etc. | `Route::get` / `Route::post` | Registro de rutas en tiempo de compilación |
| `casts!` | `protected $casts = [...]` | Declaración de casts por modelo |
| `attrs!` | Array de asignación masiva | Builder de atributos parciales |
| `json_response!` / `text_response!` | `response()->json(...)` | Atajo para `Ok(HttpResponse::...)` |

Consulta [Macros](macros.md) para la referencia completa.

## Funciones ayudantes (los ayudantes globales de Laravel; los nuestros son tipados)

Laravel ofrece cientos de pequeños globales (`str_replace_first`,
`array_flatten`, `now()`, `tap()`, `optional()` …). La mayoría tiene un
equivalente directo en Rust, en `std` o en un pequeño crate estándar, así
que Suprnova no los reintroduce como un único namespace. Los que *sí* son
útiles como alias vienen incluidos en su módulo propio.

| Ayudante de Laravel | Equivalente en Suprnova / Rust | Dónde |
|---|---|---|
| `auth()` | `Auth::user().await?` | [Autenticación](authentication.md) |
| `cache()` | `Cache::get/put/...` | [Caché](cache.md) |
| `config('app.name')` | `Config::get::<AppConfig>()?.name` | [Configuración](configuration.md) |
| `csrf_token()` | `csrf_token()` (mismo nombre) | [CSRF](csrf.md) |
| `dd()` | `Builder::dd()` (dump-and-die de consultas de Eloquent) / `dbg!()` de la stdlib | `Builder::dump()` / `Builder::dd()` existen para inspeccionar consultas; usa `dbg!()` para valores generales |
| `env('APP_KEY')` | `env("APP_KEY")` / `env_required("APP_KEY")` / `env_optional("APP_KEY")` | [Configuración](configuration.md), [Variables de entorno](env-vars.md) |
| `now()` | `chrono::Utc::now()` (reexportado como `suprnova::chrono`) | - |
| `optional($x)->y` | `x.as_ref().map(\|x\| x.y)` | Rust resuelve esto directamente con `Option<T>` |
| `redirect('/')` | `redirect("/")` (mismo nombre) | [Enrutamiento](routing.md) |
| `request()` | `Request` se pasa a tu handler | [Solicitudes](requests.md) |
| `response()` | `HttpResponse::json/text/redirect/...` | [Respuestas](responses.md) |
| `route('posts.show', ['post' => 1])` | `url("posts.show", &[("post", "1")])` | [Generación de URLs](urls.md) |
| `session('key')` | `session().get("key")` | [Sesiones](session.md) |
| `str()` / `Str::camel($x)` | métodos del crate `heck` (`ToUpperCamelCase`, etc.) | - |
| `tap($x, fn) → $x` | `tap` del crate `tap`, o `dbg!` para inspección rápida | Usa el crate `tap` de forma idiomática |
| `today()` | `chrono::Utc::now().date_naive()` | - |
| `value($x)` | Simplemente llama al closure: `x()` | n/a - los closures de Rust no necesitan un ayudante |
| `view('home', $data)` | Respuesta de Inertia: `Inertia::render("Home", data)` | [Respuestas de Inertia](frontend-inertia-responses.md) |

## Lo que de verdad todavía no tenemos

Una lista consolidada de cada **aún no** de más arriba, para que veas la
forma del hueco en un solo sitio:

| Área | Qué falta | Solución provisional |
|---|---|---|
| Búsqueda (Scout - palabras clave) | Adaptador de Algolia / Meilisearch / Elastic | Móntatelo con `meilisearch-sdk` / `elasticsearch` mientras tanto; [Vector](vector.md) cubre hoy la búsqueda semántica |
| Passport (servidor OAuth) | Proveedor de identidad OAuth de primera parte | Ejecuta Hydra o Keycloak detrás de Suprnova |
| Telescope (panel de depuración) | Interfaz web para solicitudes / consultas / eventos / aciertos de caché | Usa la salida de OTel y tracing ([Observabilidad](observability.md)) |
| Pulse (panel de rendimiento) | Interfaz web para consultas lentas / errores / rutas calientes | Lo mismo: hoy la superficie de OTel, el panel más adelante |
| Horizon (panel de colas) | Interfaz web para profundidad de cola / jobs fallidos / rendimiento | `cargo run --bin console queue:failed` y las métricas de OTel |

## Lo que no vamos a ofrecer (y por qué)

| Característica de Laravel | Por qué Suprnova no la tiene |
|---|---|
| Tinker (REPL) | Rust no tiene una historia de REPL productiva para binarios compilados. Un `#[suprnova_test]` corto o un script puntual con `cargo run --bin <thing>` hacen el trabajo |
| Plantillas Blade | Inertia es la capa de vistas; no ofrecemos un motor de plantillas renderizado en servidor en paralelo |
| `helpers.md` de cajón de sastre | Rust ofrece `std` + pequeños crates enfocados (`heck`, `chrono`, `regex`); no reintroducimos un único namespace global |
| Mix | Vite lo cubre y viene incluido en todo andamiaje |
| Octane | Suprnova ya es Tokio de larga duración; no hay un modo FPM del que optimizar la salida |
| Dusk (tests de navegador) | Las herramientas multilenguaje (Playwright, WebdriverIO, el navegador de agente `gstack`) ya resuelven esto |
| Sail (dev con Docker) | Las recetas de Docker vienen incluidas ([Docker](cli-docker.md)); no hace falta un paquete aparte |
| Valet | `suprnova serve` es el servidor de dev |
| Envoy (despliegues por SSH) | Los contenedores / systemd / orquestadores hacen el trabajo; no necesitamos un DSL de SSH a medida |
| Fachada Concurrency (`Concurrency::run`) | Tokio (`tokio::join!` / `tokio::spawn` / `tokio::select!`) es la respuesta; no hace falta una fachada |
| Fachada Processes | `tokio::process::Command` ya tiene la forma correcta |
| SDK de IA / MCP / Boost de primera parte | Elige los crates de Rust que ya uses; no ponemos barreras |
| Fachada dedicada de Redis | Cache/queue/rate-limit cubren el 95% del uso habitual; recurre al crate `redis` cuando necesites comandos ad hoc |
| Fachada Strings | `heck`, `regex`, `std::str` lo cubren; sin un `Str::camel($x)` global |
| Prompts (biblioteca de UI de CLI) | `dialoguer` / `inquire` ya existen; no reinventamos |
| Archivos de traducción PHP/JSON al estilo Laravel | La localización está disponible, pero el formato del catálogo es Fluent `.ftl` - un único formato que tanto el servidor como el navegador interpretan. `trans_choice` tampoco tiene equivalente: Fluent selecciona categorías de plural CLDR dentro del propio mensaje. [Localización](localization.md) |
| `php artisan dev --tabs` (modo de desarrollo TUI multipanel) | La salida en una sola terminal con prefijo `[name]` es la norma de las herramientas de desarrollo de Rust (`cargo watch`, `bacon`, `just`) - `suprnova serve` ya da a cada proceso (backend, frontend y cualquier entrada de `Suprnova.toml`) su propio prefijo coloreado y reinicio automático. Una TUI con pestañas es un segundo modelo de interacción para una señal que ya ofrece esto; el trabajo de `--stream` - un flujo de salida en tiempo real y programable - se ofrece como `suprnova serve --json` (NDJSON, un evento por línea). [Serve](cli-serve.md#extra-dev-processes) |

## Cómo se mantiene honesta esta lista

Toda fila de la columna **disponible** se puede verificar:

1. Haciendo grep de `framework/src/lib.rs` por el export nombrado
2. Ejecutando la suite de tests del framework (`cargo test --workspace`)
3. Leyendo el capítulo enlazado

Toda fila de la columna **aún no** es trabajo previsto, no una negativa.
Toda fila de la columna **no, por diseño** tiene un motivo de una frase en
la columna Notas; esos motivos son los principios de diseño de
[Introducción](introduction.md) aplicados a una característica concreta.

Revisado por última vez contra Laravel 13.25.0.

Si encuentras una característica de Laravel a la que recurres y que no
está en este mapa, abre un issue - o tiene una respuesta en Suprnova a la
que le falta una fila, o es un hueco real y queremos saberlo.

## Siguiente

- [Desde Laravel](from-laravel.md) - el mismo mapa, narrado en paralelo
- [Introducción](introduction.md) - los principios de diseño que sigue
  este trabajo de paridad
- [`documentation.md`](documentation.md) - el índice maestro de todos los
  capítulos

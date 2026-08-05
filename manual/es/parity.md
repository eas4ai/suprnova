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
| Ciclo de vida de la solicitud | cadena `Application` → `Server` → `handle_request` | disponible | [Ciclo de vida](lifecycle.md) |
| Contenedor de servicios | `Container` + fachada `App`, tres capas (tarea / hilo / global) | diverge | Task-local para cada solicitud, thread-local para los tests - [Contenedor](container.md) |
| Proveedores de servicios | función `bootstrap()` + `#[service]`, `#[policy]`, `#[command]`, macros de observer | diverge | Sin clase de registro - el arranque es una sola función; las macros usan `inventory` para el registro en tiempo de compilación. [Arranque](bootstrap.md) |
| Fachadas | `App::get`, `Cache::*`, `Mail::*`, `Auth::*`, `Storage::*`, `Queue::*`, `Bus::*`, `Event::*`, `Notification::*`, `Gate::*`, `Schedule::*`, `DB::*`, `Vector::*` estáticos | disponible | Misma forma de llamada; las fachadas son tipos reales, no alias |
| Contratos | Traits - `Mailer`, `KeyValueStore`, `Hasher`, `Channel`, `VectorDriver`, `Evaluator`, `PaymentProvider`, etc. | disponible | Todas las costuras públicas viven en traits; vincula por trait, cambia de implementación libremente |

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
| Definiciones de rutas | macro `routes!` + `get!` / `post!` / `put!` / `patch!` / `delete!` / `any!` / `head!` / `options!` / `fallback!` / `ws!` | disponible | [Enrutamiento](routing.md) |
| Parámetros de ruta | parámetros de ruta `{id}` + `req.param("id")` | disponible | Parámetros opcionales vía `{id?}`; restricciones vía `where!()` |
| Nombres de ruta | `.name("posts.show")` en la ruta + `url("posts.show", &[("id", "42")])` | disponible | [Generación de URLs](urls.md) |
| Grupos de rutas | macro `group!` con `.prefix()` / `.middleware()` / `.name()` / `.controller()` | disponible | El middleware de grupo se aplana sobre cada ruta en el momento del registro |
| Rutas de recursos | `resource!("posts", PostController)` registra las 7 rutas estándar | disponible | `apiResource!`, `only(...)`, `except(...)` - todas compatibles |
| URLs firmadas | `sign_url(...)`, `sign_route(...)`, `verify_signature(...)` | disponible | HMAC-SHA256 con `APP_KEY` |
| Vinculación de modelo de ruta | `#[handler]` extrae `Post` desde `{post}` vía un impl de `RouteBinding` | disponible | El derive `AutoRouteBinding` autoimplementa para los tipos `#[suprnova::model]` |
| Limitación de velocidad | middleware `throttle:60,1` + `RateLimiter::for_signature` | disponible | [Limitación de velocidad](rate-limiting.md) |
| Middleware | trait `impl Middleware`; se registra de forma global o por ruta | disponible | [Middleware](middleware.md) |
| Grupos y alias de middleware | `register_middleware_group`, `register_middleware_alias` | disponible | Se buscan por nombre de cadena en las rutas |
| Protección CSRF | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | disponible | La política de origen exige que el POST sea del mismo origen. [CSRF](csrf.md) |
| Controladores | `#[handler] pub async fn show(req: Request) -> Response` | disponible | Los controladores son módulos de funciones libres, no clases. [Controladores](controllers.md) |
| Controladores de una sola acción | Un handler ya es una única función; agrúpalos en módulos | disponible | La convención de Rust - sin la ceremonia de `__invoke` |
| Solicitudes | struct `Request` con `.input()`, `.param()`, `.query()`, `.header()`, `.cookie()`, `.json()`, `.file()`, etc. | disponible | [Solicitudes](requests.md) |
| Form Requests | `#[derive(Data, Validate, FormRequest)]` | disponible | La validación se ejecuta al extraer |
| Subida de archivos | `req.file("avatar")?` devuelve `UploadedFile`; multipart en streaming con límites de tamaño y de partes | disponible | Se derrama automáticamente a un tempfile por encima del umbral |
| Respuestas | builders de `HttpResponse` + `json!()` / `text!()` / `Redirect::to` / `view` | disponible | [Respuestas](responses.md) |
| Vistas (Blade) | Páginas de Inertia renderizadas en el servidor (Svelte/React/Vue) - sin equivalente de Blade | diverge | Inertia es la capa de vistas. Usa [Componentes de página](frontend-pages.md) en lugar de Blade |
| Empaquetado de assets (Vite) | Vite 8 viene incluido en todo andamiaje; `suprnova serve` ejecuta Vite y el backend juntos | disponible | Lectura de manifest + HMR conectados automáticamente |
| Assets estáticos (`public/`, servidos por el servidor web en Laravel) | `StaticFiles::public()`, un handler de fallback en el mismo proceso que sirve `public/` en la raíz web | disponible | `StaticFiles::from_dir(...)` + `cache_control(...)`; no hace falta un servidor web aparte |
| Generación de URLs | `url("posts.show", &[…])`, `route("posts.show", …)`, `redirect(...)`, `redirect_to(...)` | disponible | [Generación de URLs](urls.md) |
| Sesión | `session()`, `session_mut()`, flash bag vía `req.flash()` | disponible | Respaldada por BD vía `DatabaseSessionDriver`; respaldada por cookie por defecto. [Sesiones](session.md) |
| Validación | `#[derive(Validate)]` + 17 reglas integradas + traits `Rule`/`AsyncRule` | disponible | Las reglas async (p. ej. `Unique`) consultan la BD. [Validación](validation.md) |
| Manejo de errores | `FrameworkError`, `AppError`, trait `HttpError`, límite de pánico en `execute_chain_safely` | disponible | [Manejo de errores](errors.md), [Modelo de errores](error-model.md) |
| Registro de eventos | subscriber de `tracing` con campos estructurados, `LogFormat` (json / pretty / compact) | diverge | Cada línea de log es un documento JSON; `request_id` siempre está presente. [Registro de eventos](logging.md) |
| Ayudantes `abort_*` | `abort_if(cond, status, msg)`, `abort_unless(...)`, `abort_with(status, msg)` | disponible | Misma forma que la familia `abort_if` de Laravel |

## Profundizando

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Artisan Console | Binario `console` por app, construido con `#[command]` + `#[derive(Command)]` | disponible | [Consola](console.md). `cargo run --bin console <subcommand>` |
| Tinker (REPL) | Sin REPL | no, por diseño | Escribe un script puntual con `cargo run --bin xxx` o un `#[suprnova_test]` |
| Difusión | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | disponible | fanout con sea-streamer para multi-nodo. [Difusión](broadcasting.md) |
| Caché | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`, `RedisCache` | disponible | Operaciones atómicas + caché con tags + locks de caché (`LockGuard`). [Caché](cache.md) |
| Colecciones | `eloquent::Collection<M>` con métodos con forma de Laravel | disponible | `Deref<Target = Vec<M>>`, así que los idioms existentes de Vec siguen funcionando. [Colecciones de Eloquent](eloquent-collections.md) |
| Concurrencia | Tokio en todas partes - `tokio::spawn`, `tokio::join!`, `tokio::select!` | disponible | Todo el framework es async. La fachada `Concurrency::run([...])` de Laravel no se ofrece; Tokio es la respuesta |
| Contexto | `Context::put` / `Context::get` / `ContextStore` + auto-inyección en cola / correo / eventos | disponible | [Contexto](context.md) |
| Contratos | Todas las costuras públicas son traits | disponible | Ver la fila "Arquitectura / Contratos" más arriba |
| Eventos | `EventFacade::dispatch(e).await?`, `#[derive(Event)]`, `EventDispatcher`, oyentes encolados, suscriptores | disponible | [Eventos](events.md) |
| Almacenamiento de archivos | `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` sobre OpenDAL | disponible | Misma superficie `put/get/delete/copy/move/exists/url`. Protección contra path traversal incorporada. [Sistema de archivos y almacenamiento](filesystem.md) |
| Ayudantes | Los equivalentes viven en sus módulos propios (sin un `helpers.md` de cajón de sastre) | diverge | P. ej., los ayudantes de URL viven en [urls.md](urls.md), los ayudantes de strings en `std`/`heck`, los ayudantes de arrays en `std::collections` - Rust hace esto con crates, no con un namespace global |
| Cliente HTTP | builder `Http::get/post/...` + `Http::fake(...)` para tests | disponible | Registra las solicitudes automáticamente; `assert_sent` / `assert_not_sent`. [Cliente HTTP](http-client.md) |
| Localización | `Lang::get` / `get_with` / `try_get` / `has` + la macro `__!("key", name: value)` sobre catálogos Fluent `.ftl` en `lang/<locale>/`, detección con `LocaleMiddleware`, mensajes de validación traducidos, formateo con ICU4X | disponible | El mismo catálogo se sirve al navegador en `/_suprnova/lang/<locale>.ftl` y se tipa con `generate-types`. [Localización](localization.md) |
| Correo | `Mail::to(...).send(MyMail { ... }).await?` + drivers `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory` | disponible | Trait `Mailable` + cuerpos HTML/texto renderizados con Tera. [Correo](mail.md) |
| Notificaciones | `Notify::send(&user, notif).await?` + canales `mail/database/broadcast/webpush` | disponible | Trait `Notifiable` + `Notification` por canal. [Notificaciones](notifications.md), [Web Push](web-push.md) |
| Desarrollo de paquetes | Crates adaptadores del workspace (p. ej. `suprnova-payments-stripe`) | disponible | Misma forma que los paquetes de Laravel: dependen del framework, se vinculan en el contenedor, exponen macros si hace falta |
| Procesos (ejecutar comandos de shell) | `tokio::process::Command` de la stdlib | no, por diseño | Sin fachada - la API de Tokio ya tiene la forma correcta |
| Colas | `Queue::push(job).await?` + drivers `sync/memory/database/redis/null`, batches, cadenas, `JobMiddleware`, `FailedJobStore` | disponible | [Cola](queues.md) |
| Limitación de velocidad | `RateLimiter::for_signature(...)`, `ThrottleRequestsMiddleware`, `RateLimitMiddleware` | disponible | Ventana deslizante vía `SlidingWindowConfig`. [Limitación de velocidad](rate-limiting.md) |
| Búsqueda (Scout) | Sin adaptador de búsqueda de texto completo de primera parte | aún no | La búsqueda vectorial ya está disponible vía [Vector](vector.md); un equivalente de Scout para búsqueda por palabras clave está planificado |
| Strings (ayudantes) | crate `heck` (conversión de mayúsculas/minúsculas), `std::str`, `regex` | diverge | Los mismos crates que usa el resto del ecosistema Rust; sin un `Str::camel($x)` global |
| Programación de tareas | `Schedule::call/command/task` + `#[derive(Task)]` + sintaxis cron + worker `schedule:run` | disponible | [Programación de tareas](scheduling.md) |
| Claves de idempotencia | `Idempotency::remember(key, ttl, body)` - protección contra repetición al estilo Stripe | disponible | Quien llama asigna el namespace de la clave con la ruta + la identidad de usuario/negocio. [Idempotencia](idempotency.md) |
| Timeout de solicitud | `TimeoutMiddleware` configurable por ruta | disponible | Nativo de Rust - aborta el future en curso y libera al worker. [Tiempos de espera de solicitudes](timeout.md) |
| Indicadores de características (Pennant) | `Feature` + `Evaluator` + `FeatureMiddleware` + CRUD de admin | disponible | Propagación en menos de un segundo vía el trait `FeatureSync`. [Indicadores de características](feature-flags.md) |
| Observabilidad (Pulse) | OpenTelemetry vía `init_telemetry`, `Metrics`, `tracing` en todas partes | diverge | OTel es la lingua franca de la observabilidad en Rust - apunta tu colector al binario. [Observabilidad](observability.md) |
| Telescope (panel de depuración) | Sin equivalente todavía | aún no | Aplazado a v2+; la salida de tracing + OTel del framework cubre la mayoría de las necesidades de diagnóstico |
| Pulse (panel de rendimiento) | Sin equivalente todavía | aún no | Igual que Telescope - expón métricas con tu stack de observabilidad actual hasta que se publique un panel |
| Búsqueda vectorial | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | disponible | Sin la barrera de "solo Postgres pgvector". [Vector](vector.md) |

### Exclusivo de Suprnova (sin equivalente en Laravel)

| Suprnova | Qué es | Notas / enlace |
|---|---|---|
| macro `ws!()` + handlers de WebSocket | Rutas WS tipadas que comparten el router y el stack de middleware | [WebSockets](websockets.md) |
| Eventos enviados por el servidor | `SseEvent` + `HttpResponse::sse(...)` | [SSE](sse.md) |
| Flujos de trabajo | Trabajo con estado de larga duración, con reintentos, sleep y límites de paso | [Flujos de trabajo](workflows.md) |
| Supervisors | Trait `Supervisor` con reinicio automático al capturar pánicos, para tareas tokio de larga duración | [Supervisores](supervisors.md) |
| Web Push (VAPID) | Notificaciones push del navegador como canal de primera clase | [Web Push](web-push.md) |
| División lectura/escritura multiconexión | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [Base de datos](database.md) |
| HTTP/2 + WebSocket en el mismo socket | `hyper.with_upgrades()` en `Server::run` | [Ciclo de vida](lifecycle.md) |
| Contenido en Markdown + pipeline de docs | `MarkdownRenderer` (comrak saneado → syntect → ammonia) + `build_docs(DocsBuildConfig)` → un `DocsCatalog` buscable de `DocsChapter`s | Extracción de encabezados + `slugify_heading`; impulsa los docs/blog en Markdown sin un generador de sitio estático aparte |

## Seguridad

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Autenticación | `Auth::user/check/login/logout/attempt`, trait `Authenticatable`, `Guard` con nombre | disponible | [Autenticación](authentication.md) |
| Múltiples guards | `Guard` registrado por nombre (`web`, `api`, …) vía `AuthManager` | disponible | `SessionGuard`, `TokenGuard`, implementaciones personalizadas |
| Proveedores de usuario | `EloquentUserProvider<U>`, `DatabaseUserProvider`, personalizados vía el trait `UserProvider` | disponible | [Flujos de autenticación](auth-flows.md) |
| Verificación de email | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail`; el contrato `MustVerifyEmail` en el modelo de usuario | disponible | Respaldada por un provider (sin torii) - [Flujos de autenticación](auth-flows.md) |
| Restablecimiento de contraseña | `PasswordReset` + `PasswordResetMail` + `PasswordChangedMail`; el contrato `CanResetPassword` en el modelo de usuario | disponible | Respaldado por un provider (sin torii) - [Flujos de autenticación](auth-flows.md) |
| Limitación por fuerza bruta | `BruteForce` + `LoginThrottleMiddleware` | disponible | Contabilidad por IP y por usuario |
| Dos factores (TOTP) | `TwoFactor` + `TwoFactorChallengeMiddleware` + trait `TwoFactorUser` | disponible | Códigos de recuperación + protección contra repetición |
| Remember-me | Cookie firmada de larga duración vía `SessionGuard` | disponible | `auth::remember`, propiedad del framework: fila en BD + bcrypt + rotación de un solo uso |
| OAuth (Socialite) | Vía el fork vendorizado `torii_integration` (Google / GitHub / Apple, etc.) | disponible | [Autenticación](authentication.md) |
| Sanctum (tokens de API) | `TokenGuard` + tokens respaldados por BD vía torii | diverge | El modelo de token y el middleware bearer están disponibles; no hay una superficie de API de Sanctum aparte |
| Passport (servidor OAuth) | Todavía no | aún no | Si necesitas un proveedor OAuth, ejecuta un servicio de identidad dedicado (Keycloak, Hydra) detrás de Suprnova |
| Fortify (backend de auth) | Reemplazado por el módulo `auth_flows` + los tipos `auth_flows::*` | disponible | Mismo trabajo; no hace falta la separación headless/headed porque el frontend es Inertia |
| Autorización (Policies / Gates) | `Gate::allows/denies` + `#[policy] impl PostPolicy` + trait `Authorizable` + registro por macro | disponible | [Autorización](authorization.md) |
| Roles y permisos (spatie/laravel-permission) | trait `HasRoles` + tablas `roles` / `permissions` / `role_has_permissions` (`CreateRbacTables`) + `RoleMiddleware` / `PermissionMiddleware` (fail-closed) | disponible | De primera parte, no un paquete de la comunidad. Ayudantes `create_role` / `give_permission_to_role` / `assign_role_to_model`; se apoya sobre Gate/Policy. [Autorización](authorization.md) |
| Cifrado | `Crypt::encrypt/decrypt` + vinculación AAD de `CryptPurpose` | disponible | AES-256-GCM, rotación de claves vía `APP_KEY_PREVIOUS`. [Cifrado](encryption.md) |
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
| Touch del padre (timestamps) | `#[model(touches = ["post"])]` | disponible | `without_touching \|\| { ... }` para omitirlo |
| Observers | `impl Observer<User>` + `#[suprnova::observer(User)]` | disponible | 16 eventos de ciclo de vida |
| 16 eventos de ciclo de vida | `Created`, `Creating`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Retrieved`, `Replicating`, `ForceDeleting`, `ForceDeleted`, `Pruning` | disponible | Submódulo `events::*` por modelo. `EventResult::cancel(_)` hace cortocircuito con un 400 |
| Mutadores / Accesores | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | disponible | [Conversiones, accesores y mutadores de Eloquent](eloquent-mutators.md) |
| Casts (22 integrados) | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | disponible | Implementa `Cast` para uno personalizado |
| Colecciones | `Collection<M>` con `pluck`, `filter`, `map`, `each`, `chunk`, `groupBy`, `keyBy`, `sort_by`, `where_`, `first`, `last`, `count`, `is_empty`, `to_array` y demás amigos de Laravel; `Deref<Target = Vec<M>>`, así que todos los idioms de `Vec` siguen funcionando | disponible | [Colecciones de Eloquent](eloquent-collections.md) |
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
| Estilo Pest / PHPUnit | `#[suprnova_test]` (con soporte async) + aserciones `expect!()` al estilo Jest + macros BDD `describe!()` / `test!()` | disponible | Los tres funcionan indistintamente |
| Feature tests (HTTP) | Se conduce `handle_request(router, registry, req)` en el mismo proceso - sin abrir un socket | disponible | [Pruebas HTTP](http-tests.md) |
| Console tests | Ejecuta `dispatch_argv(["console", "..."])` y verifica con un assert | disponible | Misma forma que los tests HTTP, para el binario console |
| Browser tests (Dusk) | n/a en el framework - usa Playwright / WebdriverIO / el navegador de agente `gstack` | no, por diseño | Ya existen herramientas multilenguaje; no las reinventamos |
| Database tests | `TestDatabase::fresh::<Migrator>()` + rollback por test | disponible | [Pruebas de base de datos](database-testing.md) |
| Mocking y fakes | Fakes por fachada: `MailFake`, `NotifyFakeGuard`, `EventFakeGuard`, `Queue::fake`, `Bus::fake`, `Http::fake`, `Storage::fake` | disponible | Llamadas grabadas + ayudantes de aserción. [Simulación y falsificaciones](mocking.md) |
| Viaje en el tiempo | `tokio::time::{pause, advance, resume}` del runtime de la stdlib | disponible | No ofrecemos uno propio - la API de Tokio ya lo hace |
| Aislamiento del contenedor | `TestContainer::fake(\|tc\| tc.bind(...))` - thread-local | diverge | Seguro en paralelo por construcción. [Contenedor](container.md) |

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

## Frontend (Laravel tiene Blade + starter kits; nosotros tenemos Inertia)

| Laravel | Suprnova | Estado | Notas / enlace |
|---|---|---|---|
| Blade | n/a - Inertia es la capa de vistas | diverge | [Frontend](frontend.md) |
| Inertia.js | De primera clase: v3 sobre Svelte 5 / React 19 / Vue 3.5 | disponible | [Respuestas de Inertia](frontend-inertia-responses.md), [Componentes de página](frontend-pages.md) |
| Recargas parciales | `#[derive(Data)]` + `req.includes("subset")` + el protocolo de recarga parcial de Inertia | disponible | Conjuntos de include con seguridad de tipos |
| Deferred props | `Prop::deferred(...)` + `DeferConfig` | disponible | Protocolo de deferred props de Inertia v3 |
| Merge props | `MergeConfig` + `MergeStrategy::{Append, Prepend, Replace}` | disponible | Protocolo de merge de Inertia v3 |
| Encrypt history | `EncryptHistoryMiddleware` | disponible | El historial se cifra en reposo en el cliente |
| Posición de scroll | `ScrollConfig` + `ScrollMetadata` | disponible | Se restaura automáticamente al navegar |
| Tipos de TypeScript | `suprnova generate-types` lee `#[derive(InertiaProps)]` y emite `.d.ts` | disponible | [Tipos de TypeScript](frontend-typescript-types.md) |
| Lectura del manifest de Vite | Conectado automáticamente vía `Inertia::root_view` | disponible | HMR en dev, assets con hash en prod |

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
| `php artisan optimize` | `cargo build --release` | diverge | Un solo binario, sin paso de opcache |
| `php artisan config:cache` | La configuración tipada ya se comprueba en tiempo de compilación | diverge | No hay caché en tiempo de ejecución que invalidar |
| `php artisan route:cache` | Las rutas se expanden por macro en tiempo de compilación | diverge | El router se construye al arrancar a partir de rutas ya tipadas |
| Envoy (despliegues por SSH) | Usa cualquier orquestador - Docker, systemd, Kubernetes, fly.io, Railway | no, por diseño | El binario es el artefacto de despliegue |
| Forge / Vapor | No nos corresponde ofrecerlo - pero las recetas de Railway, DO y Hetzner cubren el mismo trabajo | diverge | [Despliegue](deployment.md), [Railway](deployment-railway.md), [Digital Ocean](deployment-digital-ocean.md), [Hetzner](deployment-hetzner.md) |
| Horizon (panel de colas) | Todavía sin panel | aún no | Inspección de jobs fallidos vía `cargo run --bin console queue:failed` mientras tanto |

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
| Sanctum | `TokenGuard` + middleware bearer | diverge | El modelo de token está disponible; no hay una superficie de paquete aparte |
| Scout (búsqueda de texto completo) | n/a por ahora | aún no | La búsqueda vectorial ya está disponible ([Vector](vector.md)); un equivalente de Scout por palabras clave llegará después |
| Socialite | Vía el fork vendorizado de torii | disponible | [Autenticación](authentication.md) |
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

Una lista consolidada de cada **aún no** de arriba, para que veas de un
vistazo la forma de lo que falta:

| Área | Qué falta | Solución mientras tanto |
|---|---|---|
| Búsqueda (Scout - por palabras clave) | Adaptador de Algolia / Meilisearch / Elastic | Monta el tuyo con `meilisearch-sdk` / `elasticsearch` mientras tanto; [Vector](vector.md) ya cubre la búsqueda semántica hoy |
| Passport (servidor OAuth) | Proveedor de identidad OAuth de primera parte | Ejecuta Hydra / Keycloak detrás de Suprnova |
| Telescope (panel de depuración) | UI web para requests / consultas / eventos / hits de caché | Usa la salida de OTel + tracing ([Observabilidad](observability.md)) |
| Pulse (panel de rendimiento) | UI web para consultas lentas / errores / rutas calientes | Igual: superficie de OTel hoy, panel más adelante |
| Horizon (panel de colas) | UI web para profundidad de cola / jobs fallidos / throughput | `cargo run --bin console queue:failed` y métricas de OTel |

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

## Cómo esta lista se mantiene honesta

Cada fila de la columna **disponible** se puede verificar:

1. Haciendo grep de `framework/src/lib.rs` en busca del export nombrado
2. Ejecutando la suite de tests del framework (`cargo test --workspace`)
3. Leyendo el capítulo enlazado

Cada fila de la columna **aún no** es trabajo previsto, no una negativa.
Cada fila de la columna **no, por diseño** tiene una razón de una frase en
la columna Notas; esas razones son los principios de diseño de
[Introducción](introduction.md) aplicados a una característica concreta.

Si encuentras una característica de Laravel que buscas y no está en este
mapa, abre un issue - o bien tiene una respuesta en Suprnova a la que le
falta una fila, o es un vacío real y queremos saberlo.

## Siguiente

- [Desde Laravel](from-laravel.md) - el mismo mapa, narrado en paralelo
- [Introducción](introduction.md) - los principios de diseño que sigue
  este trabajo de paridad
- [`documentation.md`](documentation.md) - el índice maestro de todos los
  capítulos

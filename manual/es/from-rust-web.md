# Desde Rust web

Has desplegado servicios de Rust en Axum, Actix, Rocket, o hyper escrito a mano.
Conoces el lenguaje y el runtime. ¿Qué te ofrece realmente Suprnova?

**La capa de productividad.** Enrutamiento, controladores, un ORM, migraciones,
colas, programación de tareas, autenticación, correo, notificaciones, difusión, caché,
almacenamiento, validación y un puente typado hacia el frontend - todo interconectado,
todo usando las mismas convenciones, todo listo para producción. Escribes
controladores y modelos; no eliges el diseño.

Si ya has construido una o dos aplicaciones reales en Axum, sabes cuánto
de ese esfuerzo fue interconectar en lugar de implementar características. Suprnova es la interconexión,
hecha una sola vez, opinionada donde la opinión importa, conectable donde no.

## El resumen de 30 segundos

```bash
suprnova new myapp --frontend svelte    # genera el andamiaje de backend + SPA + Vite
cd myapp
suprnova db:sync                        # ejecuta migraciones, regenera entidades
suprnova serve                          # backend + servidor de desarrollo de Vite
```

Ahora tienes:

- Un servidor hyper con HTTP/1.1 y HTTP/2, actualización de WebSocket, apagado elegante
- Una capa Eloquent respaldada por SeaORM con relaciones, carga anticipada, eliminaciones suaves
- Inertia.js puente entre Rust → Svelte 5 con `#[derive(InertiaProps)]` tipado
- Auth con guards y middleware del framework, más motores de contraseña,
  passkey, enlace mágico, OAuth, bearer-session, bloqueo y remember
  respaldados por Magnetar
- Una cola con drivers memory/sync/redis/database/null
- Un planificador cron impulsado por el trait `Task`
- Un binario de consola por proyecto para `cargo run --bin console <cmd>`
- Caché, almacenamiento (fs/s3/azblob/gcs), correo (SMTP + 5 proveedores: SES, Mailgun, Postmark, SendGrid, Resend), web push
- Difusión sobre un hub conectable (sea-streamer de forma predeterminada)
- Validación, CSRF, CORS, limitación de velocidad, idempotencia, tiempos de espera de solicitud, errores estructurados

Y un binario estáticamente enlazado al final de `cargo build --release`.

## Lo que hay debajo

| Aspecto | Crate |
|---|---|
| Servidor HTTP | `hyper` + middleware tipo tower (implementación propia) |
| Runtime asincrónico | `tokio` |
| Router | `matchit` |
| ORM | `sea-orm` (re-exportado como `suprnova::sea_orm`) |
| Migraciones | `sea-orm-migration` |
| Drivers de base de datos | `sqlx` (postgres / mysql / mariadb / sqlite) |
| Serialización | `serde` / `serde_json` |
| Validación | `validator` |
| Sesiones de navegador | `SessionMiddleware` del framework y stores de sesión conectables |
| Motores de autenticación | `suprnova-magnetar` detrás de fachadas propiedad del framework |
| Plantillas | `tera` (para cuerpos de correo; el frontend es Inertia) |
| Criptografía | `aes-gcm`, `argon2`, `bcrypt` |
| WebSockets | `hyper-tungstenite` |
| Streaming | `sea-streamer` (backend de difusión fanout) |
| OAuth | Registro de proveedores y motor de ceremonias de Magnetar |
| Rastreo | `tracing` + `tracing-subscriber` |

Típicamente no accederás a ninguno de estos directamente - Suprnova
re-exporta lo que necesitas. SeaORM es el passthrough más profundo: `Entity`,
`Column`, `ActiveModel`, `ConnectionTrait`, el constructor de consultas, el
prelude de migraciones. La vía de escape es `use suprnova::sea_orm;` si
necesitas algo que la superficie curada no cubre.

## Lo que Suprnova añade sobre Axum puro

Axum es excelente. Actix también. Rocket también. La razón por la que Suprnova existe
no es que esos frameworks sean malos - es que cada equipo construyendo un
producto real sobre ellos termina re-implementando la misma capa de
productividad. Suprnova proporciona esa capa:

| Capacidad | Implementar a mano en Axum | En Suprnova |
|---|---|---|
| Macros de enrutamiento que escalen a cientos de rutas | API de constructor, puede ser ruidosa | macro `routes!` con agrupación, prefijos, middleware, nombres |
| Vinculación de modelo de ruta (id de ruta → modelo cargado) | Extractor personalizado por tipo | `#[handler]` resuelve `post::Model` de `{id}` automáticamente |
| Constructor de consultas encadenable estilo Eloquent | Usar SeaORM directamente | `Post::query().db_where(...).order_by(...).get().await?` |
| Eliminaciones suaves, observadores, eventos del ciclo de vida | Construir por modelo | `#[model(soft_deletes)] + impl Observer<Post>` |
| Migraciones + generación de entidades | Conectar sea-orm-cli + scripts | `suprnova db:sync` ejecuta migraciones y regenera entidades |
| Autenticación (sesiones, proveedores, guards) | Unir tower-sessions + lógica propia | `Auth::attempt`, `Auth::user`, `.middleware(AuthMiddleware)` por ruta |
| Verificación de correo, restablecimiento de contraseña, 2FA, fuerza bruta | Construir los cuatro | Todos integrados, configurables, idempotentes |
| Cola de fondo | Elegir un driver, escribir workers | `Queue::push` + `cargo run -- queue:work` |
| Programación cron | Escribir una tarea tokio con `tokio_cron_scheduler` | `impl Task` + `Schedule::task(...).daily().at("03:00")` |
| Puente Inertia | Construir extractores + adaptador JS | `inertia_response!(&req, "Page", props)` |
| Props del frontend tipadas (Rust → TS) | Escribir un generador | `#[derive(InertiaProps)]` + `suprnova generate-types` |
| Difusión (canales públicos / privados / presencia) | Conectar backend de streaming + autenticación | traits `BroadcastHub` + `Channel`/`PrivateChannel`/`PresenceChannel` |
| Correo con múltiples proveedores | Elegir uno, escribir tu propia abstracción | `Mail::driver("ses")` etc., API uniforme `Mailable` |
| WebPush | Leer la especificación, construir un notificador | `WebPushChannel` incluida, VAPID incorporada |
| Validación + solicitudes de formulario | Usar `validator` + extractor personalizado | solicitudes de formulario `#[derive(Data, Validate)]`, validación asincrónica |
| Recursos JSON:API | Formatear respuestas a mano | `#[derive(Resource)]` |
| Limitación de velocidad con política fail-open/closed | Construirla | `RateLimiter` + `BackendErrorPolicy` |
| Claves de idempotencia | Construirlas | `Idempotency::remember(key, ttl, body)` con replay estilo Stripe |
| CSRF (con exclusiones de glob estilo Laravel) | Construirla | `CsrfMiddleware` con `except` + `except_method` |
| Errores estructurados con 5xx desinfectados | Construirla | trait `FrameworkError` / `HttpError`, recuperación de pánico |
| Contenedor con ámbitos task-local → thread-local → global | Escribir el tuyo | `App::bind` / `singleton` / `factory` con aislamiento adecuado |
| Endpoint de salud, id de solicitud, registro estructurado | Pegar junto | Todo activado de forma predeterminada |

El compromiso es las opiniones: Suprnova elige un diseño, elige un driver
predeterminado, elige una convención de nombres. Puedes desviarte (los drivers son enchufables,
la config es sobrescribible, el contenedor te permite cambiar servicios), pero los
valores predeterminados están diseñados para ser la opción correcta para "construir un producto
rápidamente".

## Patrones de Rust familiares

Reconocerás las formas:

```rust
// Un handler devuelve `Result<HttpResponse, HttpResponse>` (con alias Response).
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    Ok(HttpResponse::json(serde_json::json!({ "post": post })))
}

// Middleware es un trait, no un cierre:
#[async_trait]
impl Middleware for RequireAdmin {
    async fn handle(&self, req: Request, next: Next) -> Response {
        let user = Auth::user_as::<User>().await?
            .ok_or_else(|| HttpResponse::text("Unauthorized").status(401))?;
        if !user.is_admin {
            return Err(HttpResponse::text("Forbidden").status(403));
        }
        next(req).await
    }
}

// El trabajo de fondo es el trait `Job` - `handle(self)` ejecuta el trabajo:
#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}
```

Si estás acostumbrado a middleware de Tower: el middleware de Suprnova es conceptualmente
lo mismo (un envoltorio alrededor de `next`), pero usa un trait propio (no
`Service` de Tower) porque los tipos combinadores de tower se ponen feos cuando empiezas
a anidar extractores específicos de la aplicación. La forma es más simple; el
modelo mental es el mismo.

Si has usado el patrón de extractores de Axum: la macro `#[handler]` de Suprnova
juega el mismo rol, pero se resuelve a través del contenedor de servicios en lugar de
a través de traits, lo que te permite inyectar servicios de la aplicación así como datos de
solicitud. La vinculación de modelo de ruta (`Post` de `{id}`) está integrada.

Si has usado `sqlx` directamente: el ORM de Suprnova se asienta sobre SeaORM, que
se asienta sobre sqlx. Puedes pasar a SQL puro a través de `DB::select(...)` /
`DB::select_one(...)` o usar `DB::table("name")` para consultas dinámicas encadenables;
puedes pasar directamente a SeaORM para cosas que la superficie Eloquent
no cubre (por ejemplo, consultas `Statement` crudas con mapeo de resultados personalizado).
El [capítulo de Eloquent](eloquent.md) cubre las vías de escape.

## ¿Cuál es el delta de productividad?

Elige una característica que hayas construido antes en Axum puro. Suprnova la proporciona como un
capítulo:

- **"Construí un sistema de autenticación una vez y tomó dos semanas."** →
  [Autenticación](authentication.md) + [Flujos de autenticación](auth-flows.md). Establece
  la migración, configura la guarda, listo.
- **"Escribí mi propio worker de cola con reintentos/backoff."** →
  [Colas](queues.md). `Queue::push` + `cargo run -- queue:work`.
- **"Conecté WebSockets con hyper-tungstenite una vez."** →
  [WebSockets](websockets.md). La macro `ws!()` tipifica el handler;
  la actualización, latido ping/pong, apretón de mano close-frame, y
  la contrapresión se encargan.
- **"Construí un adaptador de Inertia desde cero."** →
  [Inertia](frontend.md). `inertia_response!(&req, "Page", props)`, con
  `InertiaProps` generando los tipos de TS.
- **"Construí un limitador de velocidad por tenant."** →
  [Limitación de velocidad](rate-limiting.md). Clave configurable, política configurable
  fail-open vs fail-closed, fail-closed devuelve 503.
- **"Implementé verificación de firma de webhook de Stripe + protección contra repetición."** →
  [Pagos: Stripe](payments-stripe.md). Integrado en el adaptador,
  los webhooks van a una tabla espejo con idempotencia ÚNICA.

Lo que construirías a mano en dos semanas, lo importas en una línea.

## Lo que todavía reconocerás como "tuyo"

Algunas pocas cosas permanecen cerca de Rust puro porque el lenguaje te da
algo mejor que una abstracción de framework:

- **Primitivas de concurrencia.** `tokio::spawn`, `Arc`, `Mutex`, canales -
  úsalos. El framework no los envuelve.
- **Tipos de error.** Tú defines tus errores de dominio. Implementa el
  trait `HttpError` en ellos para obtener un código de estado adecuado + mensaje en
  la respuesta. `FrameworkError` y `AppError` del framework
  son vías de escape para errores transversales + ad-hoc respectivamente.
- **Drivers personalizados.** Caché, cola, correo, difusión, vector, pagos - cada subsistema de "registro de drivers" acepta drivers personalizados. Implementa
  el trait, regístralo en `bootstrap.rs`, listo.
- **SQL puro cuando lo quieras.** `DB::select(...)`, `DB::table(...).get()`
  para filas dinámicas, o cae completamente a SeaORM. El ORM se quita del camino.
- **¿Tu propio middleware de tower?** Suprnova no proporciona un adaptador
  de Tower - el middleware aquí es `impl Middleware`, no `tower::Service`.
  Si necesitas llevar un crate solo para Tower, lo adaptarías a mano.
  En la práctica, el sistema de middleware integrado cubre casi todo
  lo que intentarías usar. Ver [Middleware](middleware.md).

## Lo que cedes

La honestidad importa más que el marketing:

- **Convenciones.** Los modelos viven aquí, los controladores allá, las migraciones
  allá, los observadores allá. El generador de andamiaje elige. Puedes pelear; probablemente
  no deberías. Las convenciones son de Laravel, auditadas y
  probadas en batalla.
- **Algo de flexibilidad en cómo fluye la solicitud.** La cadena de middleware
  tiene un orden fijo más externo (request-id → globals → middleware de ruta
  → handler). Puedes insertar middleware en cualquier lugar en eso, pero no
  puedes mover los niveles request-id o panic-recovery - son
  invariantes.
- **Las esquinas con forma de PHP.** Donde Laravel hace algo porque es PHP,
  Suprnova hace la cosa con forma de Rust en su lugar - pero te lo decimos cuando.
  Busca los cuadros **"Por qué Suprnova diverge"** en los capítulos.

## Por qué "inspirado en Laravel" debería importarte incluso si nunca has escrito PHP

El ecosistema web de Rust es aproximadamente donde estaba el de PHP alrededor de 2009. Los
crates existen; los patrones no. Suprnova portea un conjunto extremadamente refinado
de patrones de un framework que ha tenido 10+ años de presión de producción
moldeándolo. Obtienes patrones que ya han sobrevivido al contacto con
la realidad.

El costo es que Suprnova *es opinionada*. Si quieres un framework mínimo
"elige-todo-tú-mismo", Axum está ahí y es excelente. Si quieres un
"framework que decide cosas para que puedas enfocarte en el producto", eso es Suprnova.

## Próximos pasos

- [Instalación](installation.md) - `suprnova new`, qué genera el andamiaje
- [Inicio rápido](quickstart.md) - construye una aplicación pequeña en 5 minutos
- [Ciclo de vida de la solicitud](lifecycle.md) - cómo fluye una solicitud, qué se ejecuta dónde
- [Contenedor de servicios](container.md) - cómo se vinculan y resuelven los servicios
- [Eloquent](eloquent.md) - el capítulo más largo; la superficie es amplia

O salta a cualquier lugar a través de [`documentation.md`](documentation.md).

# Desde Laravel

Si has desplegado aplicaciones de Laravel, ya conoces el 80% de Suprnova. Este
capítulo mapea tus hábitos al equivalente de Rust para que puedas ser productivo
rápidamente. Te mostraremos los patrones que utilizas a diario, los patrones que
cambian de forma, y las pocas cosas que Rust te ofrece gratuitamente que PHP no puede.

## Resumen lado a lado

| Escribiste en Laravel | Escribes en Suprnova |
|---|---|
| `composer create laravel/laravel my-app` | `suprnova new my-app --frontend svelte` |
| `php artisan serve` | `suprnova serve` |
| `php artisan migrate` | `suprnova migrate` |
| `php artisan make:controller PostController` | `suprnova make:controller post` |
| `Route::get('/posts/{id}', [PostController::class, 'show'])` | `get!("/posts/{id}", controllers::post::show)` (in `routes!`) |
| `class Post extends Model` | `#[suprnova::model] struct Post { … }` |
| `Post::find($id)` | `Post::find(id).await?` |
| `Post::where('status', 'published')->get()` | `Post::query().db_where("status", "published").get().await?` |
| `Auth::user()` | `Auth::user().await?` |
| `Cache::remember('key', 60, fn() => …)` | `Cache::remember("key", Some(Duration::from_secs(60)), \|\| async { … }).await?` |
| `Queue::push(new SendEmail($user))` | `Queue::push(SendEmail { user_id }).await?` |
| `Mail::to($u)->send(new Welcome($u))` | `Mail::to(&u.email).send(WelcomeMail { user: u }).await?` |
| `Storage::disk('s3')->put($path, $bytes)` | `Storage::disk("s3")?.put(&path, bytes).await?` |
| `Notification::send($u, new Invoice($i))` | `Notify::send(&u, &InvoiceNotification { invoice }).await?` |
| `Gate::allows('update', $post)` | `Gate::allows::<PostPolicy, _>("update", &user, &post).await?` |
| `request()->validate([...])` | `#[handler]` extracts an `#[derive(Data, Validate)]` arg directly |
| `event(new OrderShipped($order))` | `EventFacade::dispatch(OrderShipped { order }).await?` |
| `Bus::dispatch(new ProcessFoo($x))` | `Bus::dispatch(ProcessFoo { x }).await?` |
| `php artisan schedule:list` | `suprnova schedule:list` |
| `php artisan tinker` | (sin REPL - escriba un script o una prueba `cargo run` puntual) |
| `composer require league/csv` | `cargo add csv` |

## El cambio en el modelo mental

### Asincronía, en todas partes

El cambio más grande: cada llamada a base de datos, llamada HTTP, E/S de archivo, llamada de caché,
push de cola - cualquier cosa que cruce un límite - es `async` y la llamas
con `.await?`. Una vez que lo has hecho durante un par de horas, desaparece
en el ritmo. Hasta entonces, el compilador te señalará cada lugar que olvides.

```rust
// Laravel
$user = User::find($id);
$user->subscribe($plan);
Mail::to($user)->send(new Welcome($user));

// Suprnova
let user = User::find(id).await?;
user.subscribe(&plan).await?;
Mail::to(&user.email).send(WelcomeMail { user }).await?;
```

`?` es el "retorno temprano en error" de Rust. Un handler devuelve
`Result<HttpResponse, HttpResponse>` (con alias `Response`), por lo que un `?`
en un error de BD se cortocircuita en tu convertidor de errores y el cliente
obtiene un 500 adecuado (o 4xx, dependiendo del tipo de error). Casi
nunca tienes que escribir un `try/catch` - `?` lo hace.

### Modelos en tiempo de compilación

Donde Eloquent lee tu esquema de BD en tiempo de ejecución, Suprnova lo lee en
tiempo de compilación:

```rust
#[suprnova::model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Eso es - esa estructura ES el modelo Eloquent. Obtienes
`Post::find`, `Post::query()`, `Post::create`, `post.update(...)`,
`post.delete()`, eliminación suave (con `#[model(soft_deletes)]`),
marcas de tiempo, observadores, todo. La macro genera un `Entity`,
`Model`, `ActiveModel`, y enumeración `Column` de SeaORM, e implementa el
trait `Model` de Suprnova - pero depende de `Post`, no de ninguno de esos.

Si renombras una columna en una migración, la estructura ya no coincide con el
esquema de BD - y dependiendo de tu configuración, el compilador
lo detecta en tiempo de compilación o el cast coercionado falla en la primera
consulta. De cualquier forma, te das cuenta antes de staging, no después.

### Binario único

No hay PHP-FPM, no hay configuración nginx leyendo `index.php`, no hay `composer
install` en el despliegue. `cargo build --release` te da un único binario
estáticamente enlazado. `scp` a un servidor, `systemd`, listo. O construye un
contenedor - `FROM scratch` funciona.

Tenemos [recetas de despliegue](deployment.md) para Railway, Digital
Ocean y Hetzner. La forma común: construir el binario, enviar el
binario, establecer variables de entorno, ejecutar.

## Mapear el framework

### Rutas

`routes!` juega el rol de `routes/web.php` y `routes/api.php`
combinados.

```rust
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),

    // Grupo de rutas con prefijo y middleware compartidos
    group("/admin")
        .middleware(crate::middleware::admin())
        .routes(routes! {
            get!("/users", controllers::admin::users::index).name("admin.users"),
            post!("/users", controllers::admin::users::store),
            put!("/users/{id}", controllers::admin::users::update),
            delete!("/users/{id}", controllers::admin::users::destroy),
        }),

    // Enrutamiento de recursos (el Route::resource de Laravel)
    resource!("posts", controllers::post),
}
```

Referencia completa: [Enrutamiento](routing.md). Diferencias que vale la pena conocer:

- El middleware de grupo se **aplana** en la lista de middleware de cada ruta
  en el momento del registro (no se ejecuta como una capa de cadena separada) - esto significa
  que no hay costo de tiempo de ejecución adicional para la agrupación.
- Tanto la sintaxis `{id}` de Laravel como la sintaxis `:id` al estilo Rails funcionan; se
  normalizan internamente.
- Las rutas nombradas se resuelven vía `route("posts.show", &[("id", "42")])` y
  hay una variante de URL firmada para enlaces con tiempo limitado.

### Controladores

Un controlador es simplemente una función libre que devuelve `Response`:

```rust
use suprnova::{Request, Response, json_response, HttpResponse};
use crate::models::Post;

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0").parse::<i64>()?;
    let post = Post::find_or_fail(id).await?;
    json_response!({ "post": post })
}
```

También puedes usar la macro `#[handler]` para extraer argumentos tipados (parámetros
de ruta, consulta, cuerpo, la solicitud en sí, servicios de contenedor) en la
firma:

```rust
use suprnova::handler;

#[handler]
pub async fn show(post: post::Model) -> Response {
    // La vinculación de modelo de ruta ya se ejecutó; `post` es la fila cargada.
    json_response!({ "post": post })
}
```

El tipo `post::Model` proviene del módulo generado del modelo - esa es la
señal que usa `#[handler]` para elegir la vinculación de modelo de ruta
sobre la extracción predeterminada de solicitud de formulario. Si la fila no
existe, la vinculación devuelve un 404 antes de que se ejecute tu código -
el mismo comportamiento que la vinculación implícita de Laravel.

Las estructuras de acción (controladores "invocables" de método único, al estilo Laravel)
también son compatibles: ver [Acciones](actions.md).

### Eloquent

El constructor de consultas de API dual toma nombres de Laravel o nombres
idiomáticos de Rust - ambos funcionan, elige el que se lea más limpiamente en el sitio de llamada.

```rust
// Superficie Laravel
let active = User::query()
    .db_where("status", "active")
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;

// Superficie Rust (resultado idéntico)
let active = User::query()
    .filter("status", "active")
    .order_by_desc("created_at")
    .take(20)
    .get()
    .await?;
```

`db_where` es el nombre del lado de Laravel (el `where` desnudo colisiona con la
palabra clave de Rust). `filter` es el alias idiomático de Rust. Ambos existen; ambos
hacen lo mismo. Para operadores de no-igualdad, usa `db_where_op`
(o su alias `filter_op`): `.db_where_op("status", "!=", "archived")`.
Ver la [referencia de Eloquent](eloquent.md) - es el capítulo más largo
por una razón, la superficie es amplia.

### Autenticación

```rust
use suprnova::{Auth, Credentials};

// En un handler:
let user = Auth::user().await?;   // Option<Arc<dyn Authenticatable>>
let id = user.as_ref().map(|u| u.get_auth_identifier());

// Iniciar sesión (p. ej. dentro de tu controlador de login):
let creds = Credentials::password("alice@x.com", "secret");
Auth::attempt(&creds, false).await?;

// Cerrar sesión:
Auth::logout().await?;
```

`Auth::attempt` valida las credenciales mediante el guard con estado
predeterminado y su `UserProvider` configurado; este es el camino que usa el
andamiaje generado de pila completa. `Auth::password()`, el restablecimiento
de contraseña, `BruteForce`, las passkeys, los enlaces mágicos, OAuth, las
sesiones bearer y la gestión de sesiones de Magnetar requieren el motor
Magnetar instalado. La verificación de correo y la fachada de compatibilidad
`TwoFactor` siguen siendo propiedad del framework. Consulta
[Autenticación](authentication.md), [Flujos de autenticación](auth-flows.md)
y [OAuth e inicio de sesión sin contraseña](oauth.md).

### Migraciones

Escribes migradores de SeaORM. La forma se verá familiar incluso si la
sintaxis es nueva:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(
            Table::create()
                .table(Alias::new("posts"))
                .if_not_exists()
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key().auto_increment())
                .col(ColumnDef::new(Alias::new("title")).string().not_null())
                .col(ColumnDef::new(Alias::new("body")).text().not_null())
                .to_owned()
        ).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("posts")).to_owned()).await
    }
}
```

`suprnova make:migration create_posts_table` estructura el archivo.
`suprnova migrate`, `migrate:rollback`, `migrate:status`, `migrate:fresh`
todos hacen lo que esperarías. `suprnova db:sync` ejecuta migraciones y
regenera las entidades de SeaORM contra las que compila la capa de macro.
Ver [Migraciones](migrations.md).

### Colas y programación

```rust
use suprnova::{FrameworkError, Job, Queue, async_trait};
use serde::{Deserialize, Serialize};

// Define un trabajo - los datos viven en la estructura, el contrato vive en
// `impl Job`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SendWelcomeEmail {
    pub user_id: i64,
}

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str {
        "SendWelcomeEmail"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}

// Empújalo a la cola:
Queue::push(SendWelcomeEmail { user_id: user.id }).await?;

// O con un retraso:
Queue::later(
    std::time::Duration::from_secs(60),
    SendWelcomeEmail { user_id },
).await?;
```

Los workers se ejecutan con `cargo run -- queue:work`. Los drivers incluyen
memoria y sincronía (en proceso, para pruebas), base de datos, redis y nulo.
Lotes, cadenas, trabajos únicos, reintentos, retroceso, middleware, almacén de trabajos
fallidos - todo está ahí. Ver [Colas](queues.md).

La programación utiliza el trait `Task` y el binario del planificador por proyecto:

```rust
use suprnova::{Task, TaskResult, async_trait};

pub struct DailyDigest;

#[async_trait]
impl Task for DailyDigest {
    async fn handle(&self) -> TaskResult {
        // …
        Ok(())
    }
}

// Registra dentro de bootstrap (p. ej. vía Schedule::call / .task / .add):
//   schedule.add(schedule.task(DailyDigest).daily().at("03:00").name("daily-digest"));
```

Ver [Programación de tareas](scheduling.md).

### Correo, notificaciones, difusión

Estos siguen a Laravel uno a uno. `Mailable` es una macro derive;
`Notifiable` es un trait en tu modelo de usuario; los canales son
`mail`/`database`/`broadcast`/`webpush`; la difusión admite
canales públicos, privados y de presencia. Ver [Correo](mail.md),
[Notificaciones](notifications.md), [Difusión](broadcasting.md).

### Frontend

No hay Blade. En su lugar, el frontend es un SPA real vía Inertia.js,
y pasas props tipados desde Rust:

```rust
use suprnova::{inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps, serde::Serialize)]
pub struct ShowProps {
    pub post: Post,
    pub comments: Vec<Comment>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    let comments = post.comments().get().await?;
    inertia_response!(&req, "Posts/Show", ShowProps { post, comments })
}
```

`Posts/Show` es un componente de Svelte (o React, o Vue - tu iniciador
elige). Los tipos de TypeScript para los props se generan automáticamente desde
el derive `InertiaProps` - ejecuta `suprnova generate-types` después de agregar una
nueva estructura de prop y el frontend obtiene enlaces tipados.

Si has usado Inertia en Laravel vía `inertia()`, esto es lo mismo - solo tipado de extremo a extremo. Ver la [descripción general del Frontend](frontend.md).

## Cosas que cambian de forma

Algunas cosas funcionan de manera diferente en Suprnova. Ninguna de ellas son bloqueadores,
pero vale la pena saberlas de antemano.

### Sin proveedores de servicios

Laravel tiene docenas de proveedores de servicios registrando enlaces, observadores,
compositores de vistas, etc. Suprnova tiene **una** función de bootstrap en el
`bootstrap.rs` de tu app. Registras todo allí, en orden. No es
elegante pero es transparente - puedes ver en 30 líneas exactamente qué
arranca tu app.

```rust
// bootstrap.rs
use std::sync::Arc;

pub async fn register() {
    suprnova::App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
    crate::observers::register();
}
```

Los capítulos [Contenedor](container.md) y [Arranque](bootstrap.md)
tienen los detalles.

### La configuración está tipada

Donde Laravel usa `config('app.timezone')` devolviendo lo-que-el-array-diga,
Suprnova tiene estructuras de configuración tipadas:

```rust
let cfg = suprnova::Config::get::<AppConfig>()?;
let tz = &cfg.timezone;   // &str, no mixed
```

Puedes registrar tus propias secciones de configuración tipadas. Ver [Configuración](configuration.md).

### Sin fachadas como alias

Las fachadas de Laravel como `DB::` son alias de clase configurados en `config/app.php`.
Las fachadas de Suprnova son módulos reales en la raíz del crate:

```rust
use suprnova::{Auth, Cache, DB, Event, Gate, Mail, Notify, Queue, Schedule, Storage};
```

La misma superficie, sin necesidad de alias global.

### Los tiempos de compilación son reales

Los tiempos de compilación de Rust no son PHP. Una compilación limpia de una aplicación Suprnova fresca
toma 1-2 minutos; las compilaciones incrementales durante el desarrollo son unos pocos
segundos. El flujo de trabajo de dev es el mismo - `suprnova serve` observa los
cambios y reconstruye - pero lo sentirás la primera vez que cambies una
macro y recompiles un crate descendente. El almacenamiento en caché se amortiza rápidamente.

### El verificador de préstamo existe

La mayoría de los controladores y handlers nunca tocan una anotación de tiempo de vida - las
firmas del framework las ocultan. Cuando el verificador de préstamo te grita,
es generalmente porque intentaste mantener una referencia a través de un `.await`
que cruzó un mutex o mantuviste una transacción de BD a través de una llamada awaited
que necesitaba acceso exclusivo. Los errores son claros y las correcciones son
generalmente `.clone()` o reestructurar-en-ámbitos-más-pequeños.

### Sin REPL `tinker`

No hay REPL. El equivalente más cercano es un script `cargo run`
único en `examples/`, o una prueba `#[suprnova_test]` que ejercita lo
que estás depurando. La mayoría de lo que harías en tinker (tocar un
modelo, disparar una notificación, enviar un trabajo) es una prueba de 5 líneas.

## Dónde aterrizan los capítulos de Laravel

Búsqueda rápida si sabes qué buscas pero no dónde vive:

| Tema de Laravel | Capítulo de Suprnova |
|---|---|
| Ciclo de vida | [Ciclo de vida de la solicitud](lifecycle.md) |
| Contenedor de servicios | [Contenedor de servicios](container.md) |
| Proveedores de servicios | [Arranque de la aplicación](bootstrap.md) |
| Fachadas | [Contenedor de servicios](container.md) |
| Enrutamiento | [Enrutamiento](routing.md) |
| Middleware | [Middleware](middleware.md) |
| Protección CSRF | [Protección CSRF](csrf.md) |
| Controladores | [Controladores](controllers.md) |
| Solicitudes | [Solicitudes](requests.md) |
| Respuestas | [Respuestas](responses.md) |
| Generación de URLs | [Generación de URLs](urls.md) |
| Sesión | [Sesión](session.md) |
| Validación | [Validación](validation.md) |
| Manejo de errores | [Manejo de errores](errors.md) |
| Registro de eventos | [Registro de eventos](logging.md) |
| Consola Artisan | [Consola](console.md) + [Referencia de CLI](cli.md) |
| Difusión | [Difusión](broadcasting.md) |
| Caché | [Caché](cache.md) |
| Eventos | [Eventos](events.md) |
| Almacenamiento de archivos | [Almacenamiento de archivos](filesystem.md) |
| Cliente HTTP | [Cliente HTTP](http-client.md) |
| Localización | [Localización](localization.md) - catálogos Fluent `.ftl`, no arrays PHP |
| Correo | [Correo](mail.md) |
| Notificaciones | [Notificaciones](notifications.md) |
| Colas | [Colas](queues.md) |
| Limitación de velocidad | [Limitación de velocidad](rate-limiting.md) |
| Programación de tareas | [Programación de tareas](scheduling.md) |
| Autenticación | [Autenticación](authentication.md) |
| Autorización | [Autorización](authorization.md) |
| Verificación de correo | [Flujos de autenticación](auth-flows.md) |
| Restablecimiento de contraseña | [Flujos de autenticación](auth-flows.md) |
| Cifrado | [Cifrado](encryption.md) |
| Hashing | [Hashing](hashing.md) |
| Base de datos | [Base de datos](database.md) |
| Constructor de consultas | [Constructor de consultas](queries.md) |
| Paginación | [Paginación](pagination.md) |
| Migraciones | [Migraciones](migrations.md) |
| Siembra de datos | [Siembra de datos](seeding.md) |
| Eloquent | [Eloquent](eloquent.md) |
| Eloquent: Relaciones | [Relaciones](eloquent-relationships.md) |
| Eloquent: Colecciones | [Colecciones](eloquent-collections.md) |
| Eloquent: Mutadores / Casts | [Conversiones, accesores y mutadores](eloquent-mutators.md) |
| Eloquent: Recursos de API | [Recursos JSON:API](eloquent-resources.md) |
| Eloquent: Serialización | [Serialización](eloquent-serialization.md) |
| Eloquent: Fábricas | [Fábricas](eloquent-factories.md) |
| Pruebas | [Pruebas](testing.md) |
| Pruebas HTTP | [Pruebas HTTP](http-tests.md) |
| Pruebas de base de datos | [Pruebas de base de datos](database-testing.md) |
| Simulación | [Simulación y falsificaciones](mocking.md) |
| Cashier (Stripe) | [Pagos: Stripe](payments-stripe.md) |
| Cashier (Paddle) | [Pagos: Paddle](payments-paddle.md) |
| Sanctum / Passport | Sesiones bearer de Magnetar mediante `BearerTokenMiddleware`; no hay una API separada de Sanctum o Passport |
| Horizon | La introspección de cola está integrada en el framework; no hay panel de Horizon |
| Telescope / Pulse | (diferido a v2+) |

Cosas que Laravel tiene que Suprnova no tiene (todavía):

- Paneles de Telescope / Pulse. Se incluye [observabilidad](observability.md)
  básica.
- APIs de paquete Sanctum / Passport. Las sesiones bearer de Magnetar y
  `BearerTokenMiddleware` proporcionan autenticación por token, pero no
  la superficie de gestión de tokens de Laravel.
- Panel de Horizon. La introspección de cola está integrada en el framework.
- Blade - por diseño; Inertia es la historia del frontend
- `trans_choice` - [Localización](localization.md) se envía, pero los plurales se
  seleccionan dentro del mensaje por categoría CLDR en lugar de por
  rangos de números enteros al estilo `[1,19]` que toma `trans_choice`

## Siguiente

- [Instalación](installation.md) - poner un proyecto en funcionamiento
- [Inicio rápido](quickstart.md) - construir una pequeña app en 5 minutos
- [Enrutamiento](routing.md) - el siguiente capítulo natural desde aquí

O salta a cualquier lugar vía [`documentación.md`](documentation.md).

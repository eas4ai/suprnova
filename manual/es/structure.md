# Estructura de directorios

Cuando ejecutas `suprnova new my-app --frontend svelte`, el generador te da esto:

```
my-app/
├── Cargo.toml                      # manifiesto del crate + dependencias, dos [[bin]] targets
├── .env                            # config local - URL de BD, app key, puertos
├── .env.example                    # plantilla para ops/CI
├── .gitignore                      # excluye target/, .env, node_modules/, public/assets/
├── cmd/
│   └── main.rs                     # la entrada binaria; llama a Application::new().run()
├── src/
│   ├── lib.rs                      # cableado de módulos (`pub mod controllers;` etc.)
│   ├── bootstrap.rs                # registra servicios, observadores, oyentes - el
│   │                               # análogo de Suprnova de los service providers de Laravel
│   ├── routes.rs                   # el árbol de macros `routes!` - cada URL que la app sirve
│   ├── bin/
│   │   └── console.rs              # entrada `cargo run --bin console <subcommand>` -
│   │                               # el análogo de Suprnova de `php artisan`
│   ├── actions/
│   │   ├── mod.rs
│   │   └── example_action.rs       # controladores invocables de un solo método
│   ├── commands/
│   │   └── mod.rs                  # handlers anotados con `#[command]` se registran aquí
│   ├── config/
│   │   ├── mod.rs
│   │   ├── database.rs             # config tipada de BD (driver, URL, pool)
│   │   └── mail.rs                 # config tipada de mail
│   ├── controllers/
│   │   ├── mod.rs
│   │   ├── home.rs                 # handler GET /
│   │   ├── auth.rs                 # login / register / logout
│   │   └── dashboard.rs            # requiere auth; ruta protegida de ejemplo
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── logging.rs              # registro de request/response
│   │   └── authenticate.rs         # guard de auth basado en sesión
│   ├── migrations/
│   │   ├── mod.rs
│   │   ├── m_*_create_users_table.rs
│   │   ├── m_*_create_sessions_table.rs
│   │   ├── m_*_create_remember_tokens_table.rs
│   │   ├── m_*_create_workflows_table.rs
│   │   └── m_*_create_workflow_steps_table.rs
│   └── models/
│       ├── mod.rs
│       └── user.rs                 # modelo de User con `#[suprnova::model]`
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html                  # entrada de Vite; monta el SPA
│   └── src/
│       ├── main.{tsx,ts}           # setup de cliente Inertia (por-framework)
│       ├── app.css                 # estilos globales + Tailwind
│       ├── pages/
│       │   ├── Home.{tsx,svelte,vue}
│       │   ├── Dashboard.{tsx,svelte,vue}
│       │   └── auth/
│       │       ├── Login.{tsx,svelte,vue}
│       │       └── Register.{tsx,svelte,vue}
│       └── types/
│           └── inertia-props.ts    # auto-generado desde #[derive(InertiaProps)]
└── public/
    └── assets/                     # aquí va la salida de build de Vite en producción
```

Svelte agrega `frontend/svelte.config.js` y `frontend/src/app.d.ts`.
Vue agrega `frontend/src/shims-vue.d.ts`.

El starter de API (`suprnova new my-api --api`) es más delgado: sin
`frontend/`, sin controladores de auth, y `cmd/main.rs` se reemplaza por
`src/main.rs`.

## Qué es cada directorio

### `cmd/main.rs`

El punto de entrada binaria. Un archivo corto - típicamente 10–20 líneas - que
llama al pipeline de boot estándar:

```rust
use suprnova::Application;
use my_app::{bootstrap, config, migrations, routes};

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

`Application::run()` parsea la CLI del binario (`serve` / `web:run` /
`migrate*` / `schedule:*` / `workflow:work` / `queue:work`), carga
`.env`, ejecuta tu función de config, y luego despacha el subcomando. El
path de serve también ejecuta tu función de bootstrap e inicia el
servidor HTTP.

Casi nunca editas `cmd/main.rs` después del andamiaje inicial.

### `src/lib.rs`

Un archivo de declaración de módulos plano:

```rust
pub mod actions;
pub mod bootstrap;
pub mod commands;
pub mod config;
pub mod controllers;
pub mod middleware;
pub mod migrations;
pub mod models;
pub mod routes;
```

Esto es lo que hace que `crate::controllers::home::index` sea accesible desde
`routes.rs`.

### `src/bootstrap.rs`

La función única que cablea tu app. Registras bindings del contenedor de servicios,
observadores, oyentes de eventos, middleware personalizado, y cualquier otra
configuración de boot aquí. Es el análogo de `AppServiceProvider`,
`EventServiceProvider`, `BroadcastServiceProvider`, etc. de Laravel, todo en un
archivo:

```rust
use std::sync::Arc;
use suprnova::App;

pub async fn register() {
    // Vincula un servicio en el contenedor
    App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));

    // Registra un observador de Eloquent
    crate::models::user::register_observer();

    // Escucha eventos
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
}
```

`register()` se ejecuta una vez por proceso, después del config loader pero antes
de que `serve` acepte la primera request. Los workers (`queue:work`,
`schedule:run`, `workflow:work`) reutilizan el mismo bootstrap para ver los
mismos servicios. Ver [Arranque de la aplicación](bootstrap.md).

### `src/routes.rs`

Tu superficie de URL. La macro `routes!` a nivel de módulo se expande a
una `pub fn register() -> Router` que `cmd/main.rs` pasa a
`Application::routes(...)`:

```rust
use suprnova::{get, post, put, delete, routes};
use crate::{controllers, middleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Auth (registradas + protegidas)
    get!("/login", controllers::auth::show_login).name("login.show"),
    post!("/login", controllers::auth::login).name("login.attempt"),
    post!("/logout", controllers::auth::logout).name("logout"),
    get!("/register", controllers::auth::show_register).name("register.show"),
    post!("/register", controllers::auth::register).name("register"),

    // El dashboard requiere el middleware authenticate
    get!("/dashboard", controllers::dashboard::index)
        .middleware(middleware::authenticate::auth())
        .name("dashboard"),
}
```

Ver [Enrutamiento](routing.md).

### `src/bin/console.rs`

Tu binario de consola por-proyecto. Se ejecuta como `cargo run --bin console
<subcommand>` y despacha el `db:seed` integrado del framework más
cada handler anotado con `#[command]` (o struct tipado `#[derive(Command)]`)
en `src/commands/` - ambas formas se registran a través de inventario en
tiempo de compilación:

```bash
cargo run --bin console db:seed           # integrado del framework
cargo run --bin console report:daily      # tu comando personalizado
```

Los workers de larga duración (`queue:work`, `schedule:run`,
`schedule:work`, `workflow:work`) viven en el binario principal de la app
porque `Application::run()` los despacha - llámales como
`cargo run -- queue:work` (o vía `suprnova schedule:run` /
`suprnova workflow:work` si prefieres la CLI del paraguas).

Ver [Consola](console.md).

### `src/commands/`

Donde viven tus handlers de consola. Dos tipos: un struct tipado con
args derivados de clap e `impl TypedCommand`, o un `#[command]` crudo en un
`async fn(Vec<String>) -> Result<(), FrameworkError>`. El generador de andamiaje
genera la forma tipada:

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "report:daily", description = "Generate the daily report")]
pub struct DailyReport {
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for DailyReport {
    async fn run(self) -> Result<(), FrameworkError> {
        // …
        Ok(())
    }
}
```

`suprnova make:command report-daily` genera el archivo y lo agrega a
`src/commands/mod.rs`. Ver [Consola](console.md).

### `src/config/`

Structs de configuración tipados. El andamiaje incluye `database.rs` y
`mail.rs`; agrega los tuyos para cualquier subsistema que tu app necesite. Cada
struct de config lee sus valores del environment, y
`config::register_all()` los registra con el framework:

```rust
use suprnova::{env, env_required};

#[derive(Clone, Debug)]
pub struct AnalyticsConfig {
    pub api_key: String,
    pub max_batch: u32,
}

impl AnalyticsConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: env_required::<String>("ANALYTICS_API_KEY"),
            max_batch: env("ANALYTICS_MAX_BATCH", 100u32),
        }
    }
}
```

Cablealo en `config/mod.rs`:

```rust
use suprnova::Config;

pub fn register_all() {
    Config::register(AnalyticsConfig::from_env());
}
```

Ver [Configuración](configuration.md).

### `src/controllers/`

Funciones de manejo HTTP. Un módulo por recurso. Cada `pub async fn`
que toma una `Request` y devuelve una `Response` es invocable desde una
ruta.

### `src/middleware/`

Implementaciones de middleware. El andamiaje incluye `logging` y
`authenticate`; agrega los tuyos aquí como `pub struct Foo` con
`impl Middleware for Foo`. Registralos globalmente en `bootstrap.rs`
o aplicalos por-ruta vía `.middleware(…)` en el árbol `routes!`. Ver
[Middleware](middleware.md).

### `src/migrations/`

Migradores de SeaORM. El andamiaje incluye algunos para las tablas
auth + workflow. `suprnova make:migration <name>` agrega uno nuevo. `suprnova
migrate`, `migrate:rollback`, `migrate:status`, `migrate:fresh`,
`db:sync` todos operan en este directorio. Ver [Migraciones](migrations.md).

### `src/models/`

Tus modelos de Eloquent. Un archivo por modelo, cada uno un struct
`#[suprnova::model]`. El andamiaje incluye `user.rs`; agrega nuevos modelos escribiendo un nuevo
archivo a mano o ejecutando `suprnova db:sync --regenerate-models` después de una
migración de esquema. Ver [API de Eloquent](eloquent.md).

### `src/actions/`

Controladores invocables de un solo método. Patrón opcional - úsalos cuando
un controlador tendría exactamente un método y prefieras llamarlo
"Action" en lugar de envolverlo. El andamiaje incluye un ejemplo que puedes eliminar o
adaptar. Ver [Acciones](actions.md).

### `frontend/`

El SPA de Vite + Inertia. Este es un proyecto frontend normal - `package.json`,
`vite.config.ts`, `tsconfig.json`, una entrada `index.html` de Vite, fuente
bajo `src/`. El setup del cliente Inertia vive en `src/main.{tsx,ts}` y
los componentes de página en `src/pages/`. Los tipos TypeScript para tus props de Rust
`#[derive(InertiaProps)]` se regeneran en
`src/types/inertia-props.ts` por `suprnova generate-types`.

Ver [Frontend](frontend.md).

### `public/assets/`

Donde Vite lanza el build de producción (`npm run build`). El
servidor de Suprnova sirve este directorio como activos estáticos en `/assets/*` en
producción.

## Directorios que agregarás a medida que crezca la app

El andamiaje te da lo mínimo - lo suficiente para desplegar el flujo de bienvenida
y un dashboard protegido. Las apps reales crecen con más subsistemas. Adiciones
comunes:

| Directorio | Cuándo lo agregas |
|---|---|
| `src/jobs/` | Primera vez que `Queue::push(SomeJob)`. Ver [Cola](queues.md). |
| `src/listeners/` | Primera vez que `Event::listen`. Ver [Eventos](events.md). |
| `src/observers/` | Primera vez que implementas `Observer<MyModel>`. Ver [API de Eloquent](eloquent.md#observers). |
| `src/notifications/` | Primera vez que implementas una `Notification`. Ver [Notificaciones](notifications.md). |
| `src/mail/` | Primera vez que implementas un `Mailable`. Ver [Correo](mail.md). |
| `src/policies/` | Primera vez que escribes una `#[policy]`. Ver [Autorización](authorization.md). |
| `src/factories/` | Primera vez que escribes un `Factory<Model>` para pruebas. Ver [Fábricas de Eloquent](eloquent-factories.md). |
| `src/seeders/` | Primera vez que escribes un `Seeder` para `db:seed`. Ver [Siembra de datos](seeding.md). |
| `src/events/` | Primera vez que `impl Event` para tu propio tipo de evento. Ver [Eventos](events.md). |
| `src/broadcasting/` | Primera vez que defines un `Channel` privado/presencia. Ver [Difusión](broadcasting.md). |
| `src/ws/` | Primera vez que escribes un handler `ws!()`. Ver [WebSockets](websockets.md). |
| `src/supervisors/` | Primera vez que implementas un `Supervisor` de larga duración. Ver [Supervisores](supervisors.md). |
| `src/payments/` | Primera vez que cablea Stripe/Paddle en tu app. Ver [Pagos](payments.md). |
| `src/props/` | Cuando quieres mantener structs `#[derive(InertiaProps)]` separados de los controladores. |
| `resources/views/` | Primera vez que agregas una plantilla de Tera para cuerpos de mail. |
| `storage/` | Primera vez que escribes archivos al disco del sistema de archivos local (ver [Sistema de archivos y almacenamiento](filesystem.md)). |
| `tests/` | Primera vez que escribes una prueba de integración. |

No tienes que pedir permiso - `mkdir src/jobs` y agrega
`pub mod jobs;` a `src/lib.rs`, y listo. El framework
no obliga los nombres de directorios; las convenciones existen para que otros
desarrolladores de Suprnova encuentren las cosas rápidamente.

## El `app/` dogfood en este repo

Si estás leyendo esto desde dentro del repo de Suprnova mismo, verás
un directorio `app/` en la raíz que usa cada característica del framework
en conjunto. Ese es nuestro banco de pruebas interno - ejercita pagos,
difusión, web push, flujos de trabajo, supervisores, etc. todo a la vez. No es
una referencia limpia para una app nueva; el output del andamiaje de arriba es
deliberadamente más pequeño y más fácil de aprender. Lee `app/` una vez que
quieras ver un ejemplo maximal de cómo las piezas se componen.

## Siguiente

- [Configuración](configuration.md) - cómo `.env` se convierte en config tipado
- [Arranque de la aplicación](bootstrap.md) - qué hace `bootstrap.rs` en realidad
- [Enrutamiento](routing.md) - tu primera ruta
- [Contenedor de servicios](container.md) - cómo funcionan `App::bind` y `App::get`

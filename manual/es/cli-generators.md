# Generadores de código

La familia `suprnova make:*` genera el andamiaje del archivo
convencional para cada pieza de un proyecto - un controlador, una
acción, un middleware, un comando de consola, un error de dominio,
una tarea programada, una página de Inertia o una estructura de
props, una migración de base de datos - y conecta el módulo nuevo a
su `mod.rs` padre (y, donde sea necesario, a `src/lib.rs` y
`cmd/main.rs`). Recurre a ellos cuando de otro modo estarías
retecleando el mismo boilerplate + `pub mod x;` línea de importación, que es la mayoría de las veces.

## make:controller

Genera el andamiaje de un controlador - un archivo en
`src/controllers/` con una única fn async `#[handler]` llamada
`invoke`.

```bash
suprnova make:controller User
suprnova make:controller order_item
```

El nombre se normaliza a `snake_case` para el nombre de archivo, y se
usa tal cual para el eco `controller:` en la respuesta. Solo se
aceptan letras ASCII, dígitos, y `_` - las rutas como `api/User` se
rechazan.

### Archivo generado

```rust
// src/controllers/user.rs
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn invoke(_req: Request) -> Response {
    json_response!({
        "controller": "User"
    })
}
```

### Qué conecta

1. Se escribe `src/controllers/<name>.rs` con la fn `#[handler]`.
2. Se añade `pub mod <name>;` a `src/controllers/mod.rs` (se crea el
   archivo si no existía).
3. Se imprime una pista para añadir una ruta en `src/routes.rs`:
   `.get("/<name>", controllers::<name>::invoke)`.

Consulta [Controladores](controllers.md) para el contrato de handler,
los extractores, y la macro `routes!`.

---

## make:action

Genera el andamiaje de una acción de responsabilidad única - una
estructura resoluble desde el contenedor con un método async
`execute` que devuelve un `Result<String, FrameworkError>`, para que
el esqueleto compile antes de que rellenes el cuerpo.

```bash
suprnova make:action CreateUser
suprnova make:action SendNotification
```

El nombre se convierte a PascalCase; se añade un sufijo `Action` si
falta, y el archivo usa el nombre de la estructura en snake_case.

### Archivo generado

```rust
// src/actions/create_user_action.rs
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct CreateUserAction {
    // Add injected dependencies as fields here, e.g.
    // db: suprnova::DbConnection,
}

impl CreateUserAction {
    pub async fn execute(&self) -> Result<String, FrameworkError> {
        Ok("CreateUserAction executed".to_string())
    }
}
```

### Qué conecta

1. Se escribe `src/actions/<snake>.rs`.
2. Se añade `pub mod <snake>;` a `src/actions/mod.rs`.
3. `#[injectable]` registra la acción en el contenedor en tiempo de
   enlazado, así que cualquier controlador puede resolverla vía
   `App::get::<CreateUserAction>()` y llamar a
   `action.execute().await?`.

Consulta [Acciones](actions.md) para el patrón de resolver e invocar,
y cómo las acciones componen con el contenedor.

---

## make:middleware

Genera el andamiaje de un middleware - una estructura unitaria que
implementa `suprnova::Middleware`. El cuerpo por defecto cronometra
el handler interno y registra los eventos de entrada + salida con el
id por solicitud, así que funciona de punta a punta desde la primera
vez.

```bash
suprnova make:middleware Auth
suprnova make:middleware RateLimit
```

El nombre se convierte a PascalCase; se añade un sufijo `Middleware`
si falta. El archivo usa el nombre base en snake_case (sin el
sufijo), por ejemplo `Auth` → `src/middleware/auth.rs`, estructura
`AuthMiddleware`.

### Archivo generado

```rust
// src/middleware/auth.rs
use std::time::Instant;

use suprnova::{async_trait, current_request_id, Middleware, Next, Request, Response};

pub struct AuthMiddleware;

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let method = request.method().to_string();
        let path = request.path().to_string();
        let request_id = current_request_id()
            .map(|id| id.as_str().to_string())
            .unwrap_or_default();
        let started_at = Instant::now();

        println!(
            "[AuthMiddleware] --> {} {} (request_id={})",
            method, path, request_id,
        );

        let response = next(request).await;

        println!(
            "[AuthMiddleware] <-- {} {} ({} ms, request_id={})",
            method, path, started_at.elapsed().as_millis(), request_id,
        );

        response
    }
}
```

### Qué conecta

1. Se escribe `src/middleware/<snake>.rs`.
2. Se añaden `mod <snake>;` + `pub use <snake>::<StructName>;` a
   `src/middleware/mod.rs` (se crea si es necesario).
3. Se imprimen tanto la forma por ruta
   (`.get("/path", handler).middleware(AuthMiddleware)`) como la
   forma global (`global_middleware!(middleware::AuthMiddleware)` en
   `bootstrap.rs`).

Consulta [Middleware](middleware.md) para la semántica completa de la
cadena, el orden, y la distinción entre global y por ruta.

---

## make:command

Genera el andamiaje de un comando de consola - una estructura
`#[derive(clap::Parser, Command)]` que el binario `console` por
proyecto recoge vía `inventory` en tiempo de enlazado. El cuerpo por
defecto es un `println!("…: not yet implemented")`, así que el
comando se ejecuta de inmediato.

```bash
suprnova make:command CleanCache
suprnova make:command mail:send
suprnova make:command clean-cache
```

La nomenclatura sigue tres reglas:

- Las entradas que contienen `:` se usan textualmente como el nombre
  de comando registrado (al estilo de namespace de Laravel:
  `db:seed`, `mail:send`).
- En caso contrario, el nombre de la fn en snake_case se convierte a
  kebab-case para el nombre registrado (`CleanCache` → comando
  `clean-cache`).
- El archivo y la estructura de Rust siempre son las formas en
  snake_case / PascalCase del mismo identificador.

### Archivo generado

```rust
// src/commands/clean_cache.rs
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "clean-cache", description = "TODO: describe what clean-cache does")]
pub struct CleanCache {
    // Add clap-derive args here.
}

#[async_trait]
impl TypedCommand for CleanCache {
    async fn run(self) -> Result<(), FrameworkError> {
        println!("clean-cache: not yet implemented");
        Ok(())
    }
}
```

### Qué conecta

1. Se escribe `src/commands/<snake>.rs`.
2. Se añade `pub mod <snake>;` a `src/commands/mod.rs` (se crea si es
   necesario).
3. Advierte de forma evidente si a `src/lib.rs` le falta `pub mod
   commands;` - el comando no se enlazará en el binario de consola
   sin eso.
4. Se imprime el comando de ejecución: `cargo run --bin console --
   clean-cache`.

Consulta [Consola](console.md) para la superficie completa de
comandos tipados, el atajo `#[command]` para handlers solo de argv, y
el rol del binario console por proyecto.

---

## make:error

Genera el andamiaje de un error de dominio - una estructura unitaria
anotada con `#[domain_error]`, de modo que trae de fábrica un status
HTTP, un mensaje `Display`, y un impl `From<…> for FrameworkError`.

```bash
suprnova make:error UserNotFound
suprnova make:error PaymentFailed
```

El nombre se convierte a PascalCase para la estructura y a
snake_case para el archivo. El status por defecto es 500 y el
mensaje es el nombre de la estructura en sentence case - cambia
ambos atributos en el archivo generado para que se ajusten a la
situación.

### Archivo generado

```rust
// src/errors/user_not_found.rs
use suprnova::domain_error;

#[domain_error(status = 500, message = "User not found")]
pub struct UserNotFound;
```

Cambia `status = 500` por lo que corresponda - `404` para
no-encontrado, `402` para pago-requerido, `403` para prohibido - y
edita el string del mensaje. Para payloads más ricos, añade campos
con nombre a la estructura y referéncialos en el mensaje mediante
interpolación en un impl `Display` escrito a mano (en ese punto,
elimina la macro `#[domain_error]`).

### Qué conecta

1. Se escribe `src/errors/<snake>.rs`.
2. Se añade `pub mod <snake>;` a `src/errors/mod.rs` (se crea si es
   necesario).
3. Advierte sobre declarar `mod errors;` en `src/lib.rs` si el
   directorio `errors/` se creó de nuevo.

### Usarlo

Dentro de un handler que devuelve `Response`, eleva el tipo de
dominio a un `FrameworkError` para que `?` haga cortocircuito
limpiamente:

```rust
use crate::errors::user_not_found::UserNotFound;
use suprnova::FrameworkError;

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;
    let user = find_user(id).await
        .ok_or_else(|| FrameworkError::from(UserNotFound))?;
    json_response!({ "user": user })
}
```

El capítulo [Errores](errors.md) cubre la historia completa de
errores personalizados, incluyendo cuándo usar `#[domain_error]`
frente a `AppError::bad_request(…)` frente a un impl `HttpError`
escrito a mano.

---

## make:task

Genera el andamiaje de una tarea programada - una estructura unitaria
que implementa `suprnova::Task` e imprime líneas estructuradas de
inicio/fin, para que el andamiaje registre progreso antes de que
rellenes el cuerpo real.

```bash
suprnova make:task CleanupLogs
suprnova make:task SendReminders
```

El nombre se convierte a PascalCase; se añade un sufijo `Task` si
falta. El archivo usa el nombre de la estructura en snake_case, por
ejemplo `CleanupLogs` → `src/tasks/cleanup_logs_task.rs`.

### Archivo generado

```rust
// src/tasks/cleanup_logs_task.rs
use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

pub struct CleanupLogsTask;

impl CleanupLogsTask {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CleanupLogsTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        let started_at = Instant::now();
        println!("[CleanupLogsTask] task started");

        // Replace this with the real job.

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

### Qué conecta

La primera invocación de `make:task` hace un cableado más pesado que
los otros generadores - crea desde cero la superficie del
planificador en el proyecto:

1. Se crean `src/tasks/` y `src/tasks/mod.rs` si faltan.
2. Se crea `src/schedule.rs` (el punto de entrada `register(schedule:
   &mut Schedule)`) si falta.
3. Se declaran `pub mod schedule;` y `pub mod tasks;` en `src/lib.rs`.
4. Se inserta `.schedule(<crate>::schedule::register)` en la cadena
   `Application::new()` en `cmd/main.rs` o `src/main.rs`,
   inmediatamente antes de `.run()`.
5. Se escribe `src/tasks/<snake>.rs` y se añade a
   `src/tasks/mod.rs`.

Las invocaciones posteriores omiten los pasos que ya se ejecutaron.

### Registrar la tarea

Abre `src/schedule.rs` y añade una llamada de registro con la API
fluida de schedule:

```rust
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes old log files daily"),
    );
}
```

Luego ejecuta el planificador:

```bash
suprnova schedule:work   # demonio - revisa cada minuto
suprnova schedule:run    # de una sola vez - normalmente lo llama cron
suprnova schedule:list   # muestra cada tarea registrada
```

Consulta [Programación de tareas](scheduling.md) para la superficie
completa de tareas (`hourly`, `weekly`, `cron(...)`, `between`,
`when`, `without_overlapping`, manejo de zonas horarias) y [Comandos
de programación](cli-scheduling.md) para la disyuntiva entre
ejecutar como cron o como demonio.

---

## make:inertia

Genera el andamiaje de un componente de página de Inertia (por
defecto) o de una estructura Data tipada (`--data`), según el flag.
El generador de páginas detecta el framework de frontend (Svelte 5,
React 19, Vue 3.5) desde `.env` y emite la extensión de archivo
correspondiente.

### Modo de página (por defecto)

```bash
suprnova make:inertia About
suprnova make:inertia UserProfile
```

El nombre se convierte a PascalCase y se añade el sufijo `Page` si
falta, así que `About` → `AboutPage`. El archivo aterriza en
`frontend/src/pages/` con la extensión según el frontend:
`AboutPage.svelte` para Svelte, `AboutPage.tsx` para React,
`AboutPage.vue` para Vue.

Ejemplo (Svelte):

```svelte
<!-- frontend/src/pages/AboutPage.svelte -->
<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">AboutPage</h1>
  <p class="mt-2">
    Edit <code class="bg-gray-100 px-1 rounded">frontend/src/pages/AboutPage.svelte</code> to get started.
  </p>
</div>
```

Renderízalo desde un controlador:

```rust
inertia_response!(&req, "AboutPage", props)
```

Consulta [Componentes de página](frontend-pages.md) y [Respuestas de
Inertia](frontend-inertia-responses.md) para el puente entre
controladores y páginas, las recargas parciales, y las props
compartidas.

### Modo de estructura Data (`--data`)

```bash
suprnova make:inertia UserProps --data
```

Emite una estructura `#[derive(Data, Validate)]` en `app/src/props/`
(no en `src/props/` - el prefijo `app/` está hardcodeado, así que el
archivo aterriza en la app de ejemplo/anfitriona del workspace):

```rust
// app/src/props/user_props.rs
use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UserProps {
    pub id: i64,
    // Add fields here.
    //
    // Available field attributes:
    //   #[data(input_only)] - accepted on Deserialize, omitted from Serialize
    //   #[data(output_only)] - rejected on Deserialize, included in Serialize
    //   #[data(allow_include)] - registers as ?include=-eligible (default-deny)
    //
    // For PATCH endpoints, use suprnova::data::Field<T> to distinguish
    // absent from null. For lazy outbound fields, use suprnova::inertia::Prop<T>.
}
```

Úsala en un controlador para validar los cuerpos de las solicitudes:

```rust
let dto: UserProps = req.validate_json().await?;
```

---

## make:migration

Genera el andamiaje de un archivo de migración de SeaORM con marca de
tiempo. Cubierto en detalle en [Migraciones de CLI](cli-migrations.md),
que también recorre los comandos `migrate` / `migrate:rollback` /
`migrate:status` / `migrate:fresh` / `db:sync`. La forma breve:

```bash
suprnova make:migration create_users_table
```

El nombre de la migración se preserva textualmente y se le antepone
una marca `YYYYMMDDHHMMSS_` para que los archivos se ordenen
cronológicamente. El archivo generado aterriza en `migrations/`.

Consulta [Migraciones](migrations.md) para la superficie del
constructor de esquemas y [Pruebas de base de datos](database-testing.md)
para el patrón `TestDatabase::fresh` que ejecuta migraciones contra
una base de datos aislada por test.

---

## generate-types

Emite interfaces de TypeScript a partir de cada estructura de Rust
anotada con `#[derive(InertiaProps)]`. El servidor de dev ejecuta
esto automáticamente; el comando independiente es para
verificaciones de CI y regeneraciones de una sola vez.

```bash
suprnova generate-types [--output <PATH>] [--watch]
```

| Opción | Por defecto | Descripción |
|---|---|---|
| `-o, --output <PATH>` | `frontend/src/types/inertia-props.ts` | Ruta del archivo de salida |
| `-w, --watch` | off | Monitorea los archivos fuente y regenera ante cambios |

```bash
# De una sola vez
suprnova generate-types

# Modo de monitoreo (útil cuando no quieres ejecutar el servidor de dev completo)
suprnova generate-types --watch

# Ruta de salida personalizada
suprnova generate-types --output frontend/src/types/props.ts
```

Una forma de Rust a la izquierda produce una interfaz de TypeScript a
la derecha:

```rust
#[derive(InertiaProps)]
pub struct UserPageProps {
    pub user: User,
    pub posts: Vec<Post>,
}
```

```typescript
export interface UserPageProps {
    user: User;
    posts: Post[];
}
```

Consulta [Tipos de TypeScript](frontend-typescript-types.md) para la
tabla de mapeo completa (enums, opciones, fechas, estructuras
anidadas) y los hooks de sobrescritura.

---

### Por qué Suprnova diverge

El `php artisan make:*` de Laravel deja caer un archivo en el
directorio correcto y ya está - el autoloading PSR-4 recoge la clase
nueva la siguiente vez que el framework arranca. Rust no tiene un
equivalente. Un archivo en `src/foo/bar.rs` no se compila dentro del
crate hasta que `src/foo/mod.rs` declara `pub mod bar;`, y el
directorio padre tiene que conectarse de la misma forma en
`src/lib.rs`.

Así que cada generador `suprnova make:*` hace dos cosas en lugar de
una: escribe el archivo nuevo *y* edita el `mod.rs` más cercano (y,
para `make:task` y `make:command`, también `src/lib.rs` y
`cmd/main.rs`). Por eso cada generador imprime una línea `Created
src/.../mod.rs` o `Updated src/.../mod.rs` - el cableado es parte del
trabajo, no un paso posterior que tengas que recordar por tu cuenta.

---

## Resumen

| Comando | Crea | Conecta en |
|---|---|---|
| `make:controller <name>` | `src/controllers/<snake>.rs` | `controllers/mod.rs` |
| `make:action <Name>` | `src/actions/<snake>_action.rs` | `actions/mod.rs` |
| `make:middleware <Name>` | `src/middleware/<snake>.rs` | `middleware/mod.rs` |
| `make:command <name>` | `src/commands/<snake>.rs` | `commands/mod.rs` (+ advierte sobre `lib.rs`) |
| `make:error <Name>` | `src/errors/<snake>.rs` | `errors/mod.rs` |
| `make:task <Name>` | `src/tasks/<snake>_task.rs` | `tasks/mod.rs`, `schedule.rs`, `lib.rs`, `main.rs` |
| `make:inertia <Name>` | `frontend/src/pages/<Name>Page.<ext>` | (sin cableado de módulo) |
| `make:inertia <Name> --data` | `app/src/props/<snake>.rs` | (sin cableado de módulo) |
| `make:migration <name>` | `migrations/YYYYMMDDHHMMSS_<name>.rs` | (sin cableado de módulo) |
| `generate-types` | `frontend/src/types/inertia-props.ts` | n/a |

## Siguiente

- [Descripción general de CLI](cli.md) - la tabla completa de
  subcomandos
- [Consola](console.md) - el binario console por proyecto en el que
  se alimenta `make:command`
- [Controladores](controllers.md) - el contrato de handler que
  `make:controller` genera con andamiaje
- [Programación de tareas](scheduling.md) - la API fluida de
  schedule usada para registrar las tareas generadas por `make:task`
- [Migraciones de CLI](cli-migrations.md) - los comandos migrate /
  db:sync que se combinan con `make:migration`

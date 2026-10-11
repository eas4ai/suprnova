# Code Generators

The `suprnova make:*` family scaffolds the conventional file for each
piece of a project - a controller, an action, a middleware, a console
command, a domain error, a scheduled task, a checked view, an Inertia
page or props struct, a database migration, and the twenty-one kinds of
[one-file generators](#the-one-file-generators) Laravel's artisan has, from
a factory to an interface - and wires the new module into its parent
`mod.rs` (and where needed, `src/lib.rs` and `cmd/main.rs`).
Reach for them when you'd otherwise be retyping the same boilerplate + `pub mod x;` import line, which is most of the time.

## make:controller

Scaffold a controller - a file in `src/controllers/` with a single
`#[handler]` async fn named `invoke`.

```bash
suprnova make:controller User
suprnova make:controller order_item
```

The name is normalised to `snake_case` for the file name and used
as-is for the `controller:` echo in the response. Only ASCII letters,
digits, and `_` are accepted - paths like `api/User` are rejected.

### Generated file

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

### What it wires

1. Writes `src/controllers/<name>.rs` with the `#[handler]` fn.
2. Adds `pub mod <name>;` to `src/controllers/mod.rs` (creates the
   file if it didn't exist).
3. Prints a hint to add a route in `src/routes.rs`:
   `.get("/<name>", controllers::<name>::invoke)`.

See [Controllers](controllers.md) for the handler contract,
extractors, and the `routes!` macro.

---

## make:action

Scaffold a single-responsibility action - a container-resolvable
struct with an async `execute` method that returns a
`Result<String, FrameworkError>` so the skeleton compiles before you
fill in the body.

```bash
suprnova make:action CreateUser
suprnova make:action SendNotification
```

The name is PascalCased; an `Action` suffix is appended if missing,
and the file is the snake-cased struct name.

### Generated file

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

### What it wires

1. Writes `src/actions/<snake>.rs`.
2. Adds `pub mod <snake>;` to `src/actions/mod.rs`.
3. `#[injectable]` registers the action with the container at link
   time, so any controller can resolve it via `App::get::<CreateUserAction>()`
   and call `action.execute().await?`.

See [Actions](actions.md) for the resolve-and-invoke pattern and how
actions compose with the container.

---

## make:middleware

Scaffold a middleware - a unit struct that implements
`suprnova::Middleware`. Its `handle` passes the request to `next` and
returns the response, as Laravel's middleware stub does, so the new
middleware changes nothing until you give it logic.

```bash
suprnova make:middleware Auth
suprnova make:middleware RateLimit
suprnova make:middleware Admin/EnsureRole --test
```

The name is PascalCased; a `Middleware` suffix is appended if missing.
The file uses the snake-cased base name (without the suffix), e.g.
`Auth` → `src/middleware/auth.rs`, struct `AuthMiddleware`.

A nested name writes under a subdirectory, as Laravel's
`make:middleware Admin/EnsureRole` writes under
`app/Http/Middleware/Admin`: `Admin/EnsureRole` (or `Admin\EnsureRole`)
writes `src/middleware/admin/ensure_role.rs`, struct
`EnsureRoleMiddleware`, reached as
`crate::middleware::admin::EnsureRoleMiddleware`.

### Generated file

```rust
// src/middleware/auth.rs
use suprnova::{async_trait, Middleware, Next, Request, Response};

/// Auth middleware.
pub struct AuthMiddleware;

#[async_trait]
impl Middleware for AuthMiddleware {
    /// Handle an incoming request.
    ///
    /// Code before `next(request).await` runs before the route's handler:
    /// return a response there to stop the request. Code after it can
    /// change the response the handler returned.
    async fn handle(&self, request: Request, next: Next) -> Response {
        next(request).await
    }
}
```

### With `--test`

`--test` also writes `tests/<path>_middleware.rs`
(`tests/admin_ensure_role_middleware.rs` for `Admin/EnsureRole`). The test
puts the middleware in front of a route, sends a request through
`TestClient`, and asserts the route's response comes back. It imports
the middleware from your library crate, so run the command from the
project root, where `Cargo.toml` and `src/lib.rs` are:

```rust
// tests/admin_ensure_role_middleware.rs
use my_app::middleware::admin::EnsureRoleMiddleware;
use suprnova::testing::TestClient;
use suprnova::{MiddlewareRegistry, Request, Response, Router};

async fn handled(_req: Request) -> Response {
    suprnova::http::text("handled")
}

#[tokio::test]
async fn admin_ensure_role_middleware_passes_the_request_to_the_route() {
    let router = Router::new().get("/", handled);
    let client = TestClient::new(router, MiddlewareRegistry::new().append(EnsureRoleMiddleware));

    client.get("/").send().await.assert_ok().assert_see("handled");
}
```

When you give the middleware logic, change the test to match it. See
[HTTP Tests](http-tests.md) for the client.

### What it wires

1. Writes `src/middleware/<snake>.rs`, or for a nested name the same file
   under `src/middleware/<dir>/`. An existing file is never overwritten.
2. Adds `mod <snake>;` + `pub use <snake>::<StructName>;` to the module
   file of that directory (`src/middleware/mod.rs`, or
   `src/middleware/admin/mod.rs`), creating it if needed.
3. For a nested name, declares each directory in the one above it:
   `pub mod admin;` in `src/middleware/mod.rs`.
4. With `--test`, writes the test under `tests/`.
5. Prints both the per-route shape
   (`.get("/path", handler).middleware(AuthMiddleware)`) and the
   global shape (`global_middleware!(crate::middleware::AuthMiddleware)`
   in `bootstrap.rs`).

Every file is checked before anything is written, and the command writes
all of them or none.

See [Middleware](middleware.md) for the full chain semantics,
ordering, and the global vs per-route distinction.

---

## make:command

Scaffold a console command - a `#[derive(clap::Parser, Command)]`
struct that the per-project `console` binary picks up via `inventory`
at link time. The default body prints a line with
`suprnova::console::line` that says the command is not implemented, so the
command runs immediately and a test can read what it prints.

```bash
suprnova make:command CleanCache
suprnova make:command mail:send
suprnova make:command clean-cache
```

Naming follows three rules:

- Inputs containing `:` are used verbatim as the registered command
  name (Laravel namespace style: `db:seed`, `mail:send`).
- Otherwise the snake-cased fn name is kebabbed for the registered
  name (`CleanCache` → command `clean-cache`).
- The Rust file and struct are always snake-cased / PascalCased
  forms of the same identifier.

### Generated file

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
        suprnova::console::line("clean-cache: not yet implemented");
        Ok(())
    }
}
```

### What it wires

1. Writes `src/commands/<snake>.rs`.
2. Adds `pub mod <snake>;` to `src/commands/mod.rs` (creates it if
   needed).
3. Warns loudly if `src/lib.rs` is missing `pub mod commands;` - the
   command won't link into the console binary without it.
4. Prints the run command: `cargo run --bin console -- clean-cache`.

See [Console](console.md) for the full typed-command surface, the
`#[command]` shorthand for argv-only handlers, and the per-project
console binary's role.

---

## live:make

Scaffold a Live component: a server-owned island whose typed actions
arrive over the Live protocol and whose re-rendered view is morphed in
place by the shipped browser runtime.

```bash
suprnova live:make Counter
suprnova live:make todo-list
suprnova live:make Counter --dry-run
```

Names must be plain ASCII identifiers in any of `Counter`, `TodoList`,
`todo-list`, or `todo_list` form; the file and module are snake-cased,
the struct is PascalCased, and the registered component name is
`<package>.<kebab>` (for a package named `demo-app`: `demo-app.counter`).
Rust keywords, separators, dots, and non-ASCII input are rejected before
anything is written.

### Generated file

```rust
// src/live/counter.rs
use suprnova::live::{LiveComponent, live};

/// A counter island rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "demo-app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

```html
<!-- templates/live/counter.html -->
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

### What it wires

1. Validates every target path first and refuses traversal and
   symlinks; if the component file or the view already exists, it
   warns and writes nothing at all.
2. Writes `src/live/<snake>.rs` and `templates/live/<snake>.html`
   atomically; if any write fails, every file the run created or changed
   is rolled back, and any file that could not be restored is named in
   the error instead of being reported as untouched.
3. Inserts `pub mod <snake>;` and `.register::<snake::Pascal>()?` into
   the `registry()` builder in `src/live/mod.rs`. Every project created
   by `suprnova new` ships that module with an empty registry, a
   `routes()` function that installs the guarded reserved Live routes,
   and a bootstrap that binds the registry; an older project gets the
   same module created on first use.
4. Adds `pub mod live;` to `src/lib.rs` when it is missing.
5. Prints the bootstrap line that binds the registry, then the check
   command: `suprnova live:check`.

In a project that predates the Live module, bind the registry during
bootstrap and install the routes from `cmd/main.rs` by hand:

```rust
suprnova::App::singleton(crate::live::registry().expect("Live registry"));
```

```rust
.try_routes(|| live::routes(routes::register()))
```

---

## make:error

Scaffold a domain error - a unit struct annotated with
`#[domain_error]` so it carries an HTTP status, a `Display` message,
and a `From<…> for FrameworkError` impl out of the box.

```bash
suprnova make:error UserNotFound
suprnova make:error PaymentFailed
```

The name is PascalCased for the struct and snake-cased for the file.
The default status is 500 and the message is the sentence-cased
struct name - change both attributes in the generated file to match
the situation.

### Generated file

```rust
// src/errors/user_not_found.rs
use suprnova::domain_error;

#[domain_error(status = 500, message = "User not found")]
pub struct UserNotFound;
```

Change `status = 500` to whatever fits - `404` for not-found,
`402` for payment-required, `403` for forbidden - and edit the
message string. For richer payloads, add named fields to the struct
and reference them in the message via interpolation in a hand-rolled
`Display` impl (drop the `#[domain_error]` macro at that point).

### What it wires

1. Writes `src/errors/<snake>.rs`.
2. Adds `pub mod <snake>;` to `src/errors/mod.rs` (creates it if
   needed).
3. Warns about declaring `mod errors;` in `src/lib.rs` if the
   `errors/` directory was created fresh.

### Using it

Inside a handler returning `Response`, lift the domain type to a
`FrameworkError` so `?` short-circuits cleanly:

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

The [Errors](errors.md) chapter covers the full custom-error story,
including when to use `#[domain_error]` vs `AppError::bad_request(…)`
vs a hand-rolled `HttpError` impl.

---

## make:task

Scaffold a scheduled task - a unit struct that implements
`suprnova::Task` and prints structured start/finish lines so the
scaffold logs progress before you fill in the real body.

```bash
suprnova make:task CleanupLogs
suprnova make:task SendReminders
```

The name is PascalCased; a `Task` suffix is appended if missing.
The file is the snake-cased struct name, e.g. `CleanupLogs` →
`src/tasks/cleanup_logs_task.rs`.

### Generated file

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

### What it wires

The first `make:task` invocation does heavier wiring than the other
generators - it creates the scheduler's surface in the project from
scratch:

1. Creates `src/tasks/` and `src/tasks/mod.rs` if missing.
2. Creates `src/schedule.rs` (the `register(schedule: &mut Schedule)`
   entrypoint) if missing.
3. Declares `pub mod schedule;` and `pub mod tasks;` in `src/lib.rs`.
4. Inserts `.schedule(<crate>::schedule::register)` into the
   `Application::new()` chain in `cmd/main.rs` or `src/main.rs`,
   immediately before `.run()`.
5. Writes `src/tasks/<snake>.rs` and adds it to `src/tasks/mod.rs`.

Subsequent invocations skip the steps that already ran.

### Registering the task

Open `src/schedule.rs` and add a registration call with the fluent
schedule API:

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

Then run the scheduler:

```bash
suprnova schedule:work   # daemon - checks every minute
suprnova schedule:run    # one-shot - typically called by cron
suprnova schedule:list   # show every registered task
```

A task is a type that implements `Task`, whose only method is `handle`. The
name of a task and the times it runs at are set where you register it, not on
the trait. The scaffolded `src/tasks/mod.rs` carries a doc comment that shows
the same pair: a `Task` in `src/tasks/` and its registration in
`src/schedule.rs`.

See [Scheduling](scheduling.md) for the full task surface (`hourly`,
`weekly`, `cron(...)`, `without_overlapping`, `on_one_server`,
timezone handling) and [CLI Scheduling](cli-scheduling.md) for the
run-as-cron vs run-as-daemon trade.

---

## make:view

Scaffold a checked view, Laravel's `make:view`: an Askama template under
`templates/` and a `#[suprnova::view]` struct that names it. The macro
compiles the template against the struct, so a name the template reads
that the struct lacks fails the build.

```bash
suprnova make:view welcome
suprnova make:view admin.dashboard
suprnova make:view admin/dashboard --test
```

`admin.dashboard` and `admin/dashboard` name the same view, as in
Laravel: the template is `templates/admin/dashboard.html`, and the struct
is `DashboardView` in `src/views/admin/dashboard.rs`, reached as
`crate::views::admin::dashboard::DashboardView`. Each segment is letters,
digits, `_` and `-`, starting with a letter; the template path keeps the
segment as you write it, and the module name is its snake case
(`reports.user-summary` → `templates/reports/user-summary.html`,
`src/views/reports/user_summary.rs`).

### Generated files

```html
<!-- templates/admin/dashboard.html -->
<div>
    <h1>{{ title }}</h1>
</div>
```

```rust
// src/views/admin/dashboard.rs
/// The data `templates/admin/dashboard.html` renders.
#[suprnova::view(path = "admin/dashboard.html")]
pub struct DashboardView {
    /// The page heading.
    pub title: String,
}
```

Render it from a handler with `ViewTemplate::render_view`, and return the
HTML:

```rust
use suprnova::view::ViewTemplate;
use suprnova::{FrameworkError, HttpResponse, Request, Response};

use crate::views::admin::dashboard::DashboardView;

pub async fn dashboard(_req: Request) -> Response {
    let mut html = String::new();
    DashboardView { title: "Dashboard".into() }
        .render_view(&mut html)
        .map_err(|failure| FrameworkError::internal(format!("dashboard view: {failure:?}")))?;
    Ok(HttpResponse::html(html))
}
```

### With `--test`

`--test` also writes `tests/<path>_view.rs`
(`tests/admin_dashboard_view.rs`), which renders the view with a title
and asserts the title is in the HTML. It imports the view from your
library crate, so run the command from the project root.

### What it wires

1. Refuses to overwrite: if the template, the view file or (with
   `--test`) the test exists, the command says which and writes nothing.
   Pass `--force` to overwrite them.
2. Writes `templates/<path>.html` and `src/views/<path>.rs`.
3. Declares the modules on the way: `pub mod views;` in `src/lib.rs`,
   `pub mod admin;` in `src/views/mod.rs` and `pub mod dashboard;` in
   `src/views/admin/mod.rs`, creating each module file it needs. A
   declaration that is already there is left alone.
4. With `--test`, writes the test under `tests/`.

Every file is checked before anything is written, and the command writes
all of them or none.

---

## make:inertia

Scaffold either an Inertia page component (default) or a typed Data
struct (`--data`), depending on the flag. The page generator detects
the frontend framework (Svelte 5, React 19, Vue 3.5) from `.env` and
emits the matching file extension.

### Page mode (default)

```bash
suprnova make:inertia About
suprnova make:inertia UserProfile
suprnova make:inertia Admin/Users --test
```

The name is PascalCased and the suffix `Page` is appended if missing,
so `About` → `AboutPage`. The file lands in `frontend/src/pages/`
with the per-frontend extension: `AboutPage.svelte` for Svelte,
`AboutPage.tsx` for React, `AboutPage.vue` for Vue.

A nested name writes under the same directory of `frontend/src/pages/`,
and the component is named with it: `Admin/Users` writes
`frontend/src/pages/Admin/UsersPage.svelte`, rendered as
`"Admin/UsersPage"`. Directory segments keep the spelling you give them.

An existing page is kept: the command says so and writes nothing. Pass
`--force` to overwrite it.

Example (Svelte):

```svelte
<!-- frontend/src/pages/AboutPage.svelte -->
<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">AboutPage</h1>
  <p class="mt-2">
    Edit <code class="bg-gray-100 px-1 rounded">frontend/src/pages/AboutPage.svelte</code> to get started.
  </p>
</div>
```

Render it from a controller:

```rust
inertia_response!(&req, "AboutPage", props)
```

`--test` also writes `tests/<path>.rs` (`tests/admin_users_page.rs` for
`Admin/Users`). The test answers a request with the page through
`TestClient` and asserts that the response names the component and that
its file exists under `frontend/src/pages`:

```rust
// tests/admin_users_page.rs
use suprnova::testing::TestClient;
use suprnova::{inertia_response, MiddlewareRegistry, Request, Response, Router};

async fn page(req: Request) -> Response {
    inertia_response!(&req, "Admin/UsersPage", {})
}

#[tokio::test]
async fn admin_users_page_renders() {
    let client = TestClient::new(Router::new().get("/", page), MiddlewareRegistry::new());

    client
        .get("/")
        .inertia()
        .send()
        .await
        .assert_ok()
        .assert_inertia()
        .component_exists("Admin/UsersPage", true);
}
```

`--force` overwrites the test too. `--test` and `--data` cannot be
combined.

See [Frontend Pages](frontend-pages.md) and
[Inertia Responses](frontend-inertia-responses.md) for the bridge
between controllers and pages, partial reloads, and shared props.

### Data struct mode (`--data`)

```bash
suprnova make:inertia UserProps --data
```

Emits a `#[derive(Data, Validate)]` struct in `src/props/`, and warns
when it creates the directory, since `src/lib.rs` has to declare
`pub mod props;` for the struct to be reachable:

```rust
// src/props/user_props.rs
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

Use it in a controller to validate request bodies:

```rust
let dto: UserProps = req.validate_json().await?;
```

---

## make:migration

Scaffold a timestamped SeaORM migration file. Covered in detail in
[CLI Migrations](cli-migrations.md), which also walks the
`migrate` / `migrate:rollback` / `migrate:status` / `migrate:fresh` /
`db:sync` commands. The short form:

```bash
suprnova make:migration create_users_table
```

The migration name is preserved verbatim and prefixed with a
`YYYYMMDDHHMMSS_` stamp so files sort chronologically. The generated
file lands in `migrations/`.

The generated file uses SeaORM's builder. The
[schema builder](migrations.md#the-schema-builder) is the alternative you write
by hand. See [Migrations](migrations.md) for both forms and
[Database Testing](database-testing.md) for the `TestDatabase::fresh`
pattern that runs migrations against an isolated database per test.

---

## The one-file generators

Twenty-one generators write one file from a stub, as Laravel's artisan
commands of the same names do: `make:factory`, `make:seeder`, `make:cast`,
`make:channel`, `make:class`, `make:enum`, `make:event`, `make:listener`,
`make:job`, `make:job-middleware`, `make:mail`, `make:notification`,
`make:observer`, `make:policy`, `make:provider`, `make:resource`,
`make:rule`, `make:scope`, `make:test`, `make:trait` and `make:interface`.
They share these rules:

- **The name is a Rust identifier**: ASCII letters, digits and `_`,
  starting with a letter. A name whose module would be a Rust keyword
  (`Type` would be the module `type`) is refused too. A refused name exits
  with status 1 and a message that says why, and nothing is written. A
  name in snake case is read as words (`order_shipped` is `OrderShipped`);
  any other keeps its spelling, with the first letter upper-cased.
- **One file**, `src/<directory>/<snake>.rs`, in the directory each section
  below names. `make:class` writes `src/<snake>.rs`, and `make:test` writes
  `tests/<snake>.rs`.
- **The module is declared**: `pub mod <directory>;` in `src/lib.rs`, and
  `pub mod <snake>;` in the directory's `mod.rs`, which the generator
  creates when the project has none. A declaration that is already there
  is left alone. `make:test` declares nothing, because every file under
  `tests/` is a crate of its own.
- **An existing file is kept.** The command says the file exists, writes
  nothing and exits 0. Pass `--force` to write the stub over it.
- **The path is printed**: `Created <path>` or `Updated <path>` for each
  file the run writes, then the line that wires the new item in.
- **No clash with the stub's imports.** `make:rule Rule` is refused,
  because the stub imports the `Rule` trait under that name.

Every file is checked before anything is written, and the command writes
all of them or none. Each stub compiles in a project `suprnova new`
creates.

```bash
suprnova make:event OrderShipped
suprnova make:listener SendShipmentNotification --event OrderShipped
suprnova make:job ProcessPodcast --force
```

```text
  ✓ Created src/events/order_shipped.rs
  ✓ Updated src/lib.rs
  ✓ Created src/events/mod.rs
```

---

## make:factory

Writes `src/factories/<snake>.rs`, a [model factory](eloquent-factories.md):
a marker struct and its `Factory` impl. A `Factory` suffix is appended when
the name lacks one, and the model is the name without it: `make:factory
Post` writes `PostFactory` for `crate::models::post::Post`. `--model <Model>`
names another model. When `src/models/<snake>.rs` is missing, the command
warns, since the project does not compile until the model exists.

```bash
suprnova make:factory Post
```

```rust
// src/factories/post_factory.rs
use suprnova::Factory;

use crate::models::post::Post;

pub struct PostFactory;

impl Factory for PostFactory {
    type Model = Post;

    fn definition() -> Post {
        Post {
            ..Default::default()
        }
    }
}
```

Set the fields a new model needs in `definition`, with a `suprnova::Faker`
value where one should vary. Every `#[suprnova::model]` struct implements
`Default`, so a field you leave out keeps its default.

---

## make:seeder

Writes `src/seeders/<snake>.rs`, a [seeder](seeding.md) that `db:seed` runs.
A `Seeder` suffix is appended when the name lacks one.

```bash
suprnova make:seeder Users
```

```rust
// src/seeders/users_seeder.rs
use suprnova::{FrameworkError, Seeder, async_trait};

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str {
        "UsersSeeder"
    }

    async fn run() -> Result<(), FrameworkError> {
        Ok(())
    }
}
```

Register it in `src/bootstrap.rs` with
`suprnova::seed::register::<crate::seeders::users_seeder::UsersSeeder>();`,
then run it with `suprnova db:seed UsersSeeder`.

---

## make:cast

Writes `src/casts/<snake>.rs`, a custom attribute
[cast](eloquent-mutators.md): a unit struct that implements `Cast`. The stub
stores the value as it is, as Laravel's cast stub returns `$value`; change
`Runtime`, `Storage` and the two conversions to the types you need.

```bash
suprnova make:cast AsMoney
```

```rust
// src/casts/as_money.rs
use suprnova::{Cast, FrameworkError};

pub struct AsMoney;

impl Cast for AsMoney {
    type Runtime = String;
    type Storage = String;

    fn to_storage(value: &String) -> Result<String, FrameworkError> {
        Ok(value.clone())
    }

    fn from_storage(stored: &String) -> Result<String, FrameworkError> {
        Ok(stored.clone())
    }
}
```

Name it on a model field:
`#[model(casts = { amount = crate::casts::as_money::AsMoney })]`.

---

## make:channel

Writes `src/broadcasting/<snake>.rs`, a private
[broadcast channel](broadcasting.md#channels) and who may join it. The
channel name is the type's words without `Channel`, joined by dots:
`OrderUpdates` is `order.updates`. Its `authorize` admits a subscriber the
session authenticated on the WebSocket upgrade.

```bash
suprnova make:channel OrderUpdates
```

```rust
// src/broadcasting/order_updates.rs
use suprnova::async_trait;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;
use suprnova::serde_json::Value;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str {
        "order.updates"
    }

    async fn authorize(&self, req: &Request, _params: &ChannelParams, _data: &Value) -> bool {
        req.auth_user_id().is_some()
    }
}

impl PrivateChannel for OrderUpdates {}
```

Register it in the `ChannelRegistry` your bootstrap binds:
`registry.register(OrderUpdates);`.

---

## make:class

Writes `src/<snake>.rs` and declares it in `src/lib.rs`, as Laravel's
`make:class` writes under `app/`: a plain struct with a doc comment and an
`impl` block. The names `Lib` and `Main` are refused, because they would
write the crate's own root.

```bash
suprnova make:class Invoice
```

```rust
// src/invoice.rs
/// A plain Rust type: give `Invoice` the fields and methods it needs.
#[derive(Debug, Default)]
pub struct Invoice;

impl Invoice {
    /// Create a new `Invoice`.
    pub fn new() -> Self {
        Self
    }
}
```

---

## make:enum

Writes `src/enums/<snake>.rs`, an enum with no variants yet, as Laravel's
enum stub has no cases. Write one variant per case.

```bash
suprnova make:enum Status
```

```rust
// src/enums/status.rs
/// The values a `Status` can take: write one variant per case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {}
```

---

## make:event

Writes `src/events/<snake>.rs`, an [event](events.md): a struct whose fields
every listener receives, and its `Event` impl.

```bash
suprnova make:event OrderShipped
```

```rust
// src/events/order_shipped.rs
use suprnova::Event;

#[derive(Debug, Clone)]
pub struct OrderShipped {}

impl Event for OrderShipped {
    fn event_name() -> &'static str {
        "OrderShipped"
    }
}
```

Dispatch it with `EventFacade::dispatch(OrderShipped {}).await?`.

---

## make:listener

Writes `src/listeners/<snake>.rs`, an [event listener](events.md). With
`--event <Event>`, it handles that event, imported from
`crate::events::<snake>`; the command warns when that file is missing.
Without `--event`, it handles any event it is registered for, as Laravel's
listener stub takes `object $event`.

```bash
suprnova make:listener SendShipmentNotification --event OrderShipped
```

```rust
// src/listeners/send_shipment_notification.rs
use suprnova::{FrameworkError, Listener, async_trait};

use crate::events::order_shipped::OrderShipped;

pub struct SendShipmentNotification;

#[async_trait]
impl Listener<OrderShipped> for SendShipmentNotification {
    async fn handle(&self, _event: &OrderShipped) -> Result<(), FrameworkError> {
        Ok(())
    }
}
```

The listener of any event implements `Listener<E>` for every `E: Event`.
Register a listener in `src/bootstrap.rs`:

```rust
EventFacade::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
```

---

## make:job

Writes `src/jobs/<snake>.rs`, a [queued job](queues.md): a struct whose
fields go into the queue payload, and its `Job` impl.

```bash
suprnova make:job ProcessPodcast
```

```rust
// src/jobs/process_podcast.rs
use serde::{Deserialize, Serialize};
use suprnova::{FrameworkError, Job, async_trait};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessPodcast {}

#[async_trait]
impl Job for ProcessPodcast {
    fn job_name() -> &'static str {
        "ProcessPodcast"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}
```

Register it at boot with
`suprnova::queue::worker::register_job::<ProcessPodcast>();` and push it
with `Queue::push(ProcessPodcast {}).await?`.

---

## make:job-middleware

Writes `src/jobs/middleware/<snake>.rs`, as Laravel writes under
`app/Jobs/Middleware`: [job middleware](queues.md#job-middleware) that passes
the job on. It declares `pub mod jobs;` in `src/lib.rs` and
`pub mod middleware;` in the `jobs` module when they are missing.

```bash
suprnova make:job-middleware RateLimitedByUser
```

```rust
// src/jobs/middleware/rate_limited_by_user.rs
use suprnova::{
    Envelope, FrameworkError, JobMiddleware, JobMiddlewareNext, JobOutcome, async_trait,
};

pub struct RateLimitedByUser;

#[async_trait]
impl JobMiddleware for RateLimitedByUser {
    async fn handle(
        &self,
        envelope: Envelope,
        next: JobMiddlewareNext,
    ) -> Result<JobOutcome, FrameworkError> {
        next(envelope).await
    }
}
```

Return it from a job's `middleware()`: `vec![Arc::new(RateLimitedByUser)]`.

---

## make:mail

Writes `src/mail/<snake>.rs`, a [mailable](mail.md). The subject is the
type's words, as Laravel's mail stub uses the class name's headline, and the
bodies are Tera templates whose context is the struct's fields.

```bash
suprnova make:mail OrderShipped
```

```rust
// src/mail/order_shipped.rs
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::mail::Mailable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderShipped {}

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str {
        "OrderShipped"
    }

    fn subject(&self) -> String {
        "Order Shipped".to_string()
    }

    fn html_template_source(&self) -> Option<String> {
        Some("<p>Order Shipped</p>".to_string())
    }

    fn text_template_source(&self) -> Option<String> {
        Some("Order Shipped".to_string())
    }
}
```

Send it with `Mail::to("user@example.com").send(OrderShipped {}).await?`.

---

## make:notification

Writes `src/notifications/<snake>.rs`, a [notification](notifications.md)
sent on the `mail` channel, with the mail rendering
`#[derive(NotificationMailable)]` builds from its `#[mail(...)]` attribute.

```bash
suprnova make:notification InvoicePaid
```

```rust
// src/notifications/invoice_paid.rs
use serde::{Deserialize, Serialize};
use suprnova::serde_json;
use suprnova::{Notification, NotificationMailable};

#[derive(Debug, Clone, Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Invoice Paid",
    html = "<p>The introduction to the notification.</p><p>Thank you for using our application!</p>",
    text = "The introduction to the notification.\nThank you for using our application!"
)]
pub struct InvoicePaid {}

impl Notification for InvoicePaid {
    fn notification_name() -> &'static str {
        "InvoicePaid"
    }

    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }

    fn data(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}
```

Register its mail renderer at boot with
`suprnova::register_mail_renderer::<InvoicePaid>()?;`, then send it with
`Notify::send(&user, &InvoicePaid {}).await?`.

---

## make:observer

Writes `src/observers/<snake>.rs`, a model [observer](eloquent.md#observers-and-lifecycle-events)
with the `created`, `updated`, `deleted`, `restored` and `force_deleted`
hooks Laravel's observer stub has. An `Observer` suffix is appended when the
name lacks one, and the model is the name without it; `--model <Model>`
names another. `#[suprnova::observer]` installs it when the application
boots. The attribute names the model by its full path, because the macro
reaches the model's events through the module the model is in.

```bash
suprnova make:observer User
```

```rust
// src/observers/user_observer.rs
use suprnova::{FrameworkError, Observer, async_trait};

use crate::models::user::User;

pub struct UserObserver;

#[suprnova::observer(crate::models::user::User)]
#[async_trait]
impl Observer<User> for UserObserver {
    async fn created(&self, _user: &User) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn updated(&self, _previous: &User, _current: &User) -> Result<(), FrameworkError> {
        Ok(())
    }

    // `deleted`, `restored` and `force_deleted` follow the same shape.
}
```

---

## make:policy

Writes `src/policies/<snake>.rs`, a `#[policy]` for a model with the seven
abilities of Laravel's model policy stub. A `Policy` suffix is appended when
the name lacks one, and the model is the name without it; `--model <Model>`
names another. The user is `crate::models::user::User`. Every method
denies until you write its rule, as Laravel's stub returns `false`.

```bash
suprnova make:policy Note
```

```rust
// src/policies/note_policy.rs
use suprnova::policy;

use crate::models::user::User;
use crate::models::note::Note;

pub struct NotePolicy;

#[policy(User, Note)]
impl NotePolicy {
    fn view_any(_user: &User, _note: &Note) -> bool {
        false
    }

    fn view(_user: &User, _note: &Note) -> bool {
        false
    }

    // `create`, `update`, `delete`, `restore` and `force_delete` follow.
}
```

`update` registers the gate `update-note`; see
[Authorization](authorization.md#policies-with-policy). A policy of `User`
itself names its second parameter `_model`, as Laravel's stub does.

---

## make:provider

Writes `src/providers/<snake>.rs`: a `pub async fn register()` for a group
of service registrations, which your bootstrap calls.

```bash
suprnova make:provider PaymentServiceProvider
```

```rust
// src/providers/payment_service_provider.rs
pub async fn register() {
    // `App::singleton`, `App::bind` and `EventFacade::listen` calls go here.
}
```

Call it from `register` in `src/bootstrap.rs`:

```rust
crate::providers::payment_service_provider::register().await;
```

### Why Suprnova diverges

Laravel's `make:provider` writes a `ServiceProvider` class with `register`
and `boot` methods and adds it to `bootstrap/providers.php`. Suprnova has no
provider class: [Application Bootstrap](bootstrap.md#why-suprnova-diverges)
explains why `register` and `register_http_stack` replace the provider
list. What plays a provider's part is a public `register` function that
`src/bootstrap.rs` calls, so that is the stub. The command does not edit
your bootstrap, because where the call goes in the boot order is your
decision; it prints the line to add.

---

## make:resource

Writes `src/resources/<snake>.rs`, a [JSON:API resource](eloquent-resources.md):
a `#[derive(Data)]` struct with `#[json_resource]`. A `Resource` suffix is
appended when the name lacks one. The JSON:API type is the plural of the
name's words in kebab case: `User` is `users`, `UserProfile` is
`user-profiles`.

```bash
suprnova make:resource User
```

```rust
// src/resources/user_resource.rs
use suprnova::Data;
use validator::Validate;

#[derive(Debug, Clone, Data, Validate)]
#[json_resource("users")]
pub struct UserResource {
    pub id: u64,
}
```

Add the attributes as fields, and render it with
`Resource::single(resource).render().await` or `Resource::collection`.

---

## make:rule

Writes `src/rules/<snake>.rs`, a custom [validation rule](validation.md#writing-your-own-rule)
that passes every value until you write its check, as Laravel's rule stub
does.

```bash
suprnova make:rule Uppercase
```

```rust
// src/rules/uppercase.rs
use suprnova::{Rule, ValidationMessage};

pub struct Uppercase;

impl Rule for Uppercase {
    fn passes(&self, _value: &str) -> Result<(), ValidationMessage> {
        Ok(())
    }
}
```

Use it in a `validate!` row: `name => Required, Uppercase;`.

---

## make:scope

Writes `src/scopes/<snake>.rs`, a [global scope](eloquent.md#global-scopes)
that adds nothing to the query yet. A `Scope` suffix is appended when the
name lacks one. A scope's name does not say which model it scopes, so the
model is `User` unless `--model <Model>` names another.

```bash
suprnova make:scope Published --model Post
```

```rust
// src/scopes/published_scope.rs
use suprnova::{Builder, GlobalScope};

use crate::models::post::Post;

pub struct PublishedScope;

impl GlobalScope<Post> for PublishedScope {
    fn apply(&self, query: Builder<Post>) -> Builder<Post> {
        query
    }
}
```

Register it at boot: `ScopeRegistry::register::<Post, _>(PublishedScope);`.

---

## make:test

Writes `tests/<snake>.rs`, an integration test that sends a request through
`TestClient` and asserts the answer, as Laravel's feature test stub does.
It declares no module. See [HTTP Tests](http-tests.md) for the client.

```bash
suprnova make:test UserTest
cargo test --test user_test
```

```rust
// tests/user_test.rs
use suprnova::testing::TestClient;
use suprnova::{MiddlewareRegistry, Request, Response, Router};

async fn home(_req: Request) -> Response {
    suprnova::http::text("Hello")
}

#[tokio::test]
async fn example() {
    let client = TestClient::new(Router::new().get("/", home), MiddlewareRegistry::new());

    client.get("/").send().await.assert_ok().assert_see("Hello");
}
```

---

## make:trait

Writes `src/concerns/<snake>.rs`, as Laravel writes traits under
`app/Concerns`: a trait with a default method, which every implementor gets
without writing it.

```bash
suprnova make:trait HasUuid
```

```rust
// src/concerns/has_uuid.rs
pub trait HasUuid {
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}
```

---

## make:interface

Writes `src/contracts/<snake>.rs`, as Laravel writes interfaces under
`app/Contracts`: a trait whose methods are all required, with `Send + Sync`
so an implementor can be bound in the container as `Arc<dyn PaymentGateway>`.

```bash
suprnova make:interface PaymentGateway
```

```rust
// src/contracts/payment_gateway.rs
pub trait PaymentGateway: Send + Sync {
    fn name(&self) -> &str;
}
```

### Why Suprnova diverges

PHP has two constructs where Rust has one. A PHP trait carries method
bodies that are copied into the class that uses it; a PHP interface names
methods without bodies. Rust's `trait` does both: a method with a body is a
default that an implementor may override, and a method without one is
required. So `make:trait` and `make:interface` both write a `trait`.
`make:trait` gives it a default method and `make:interface` gives it
required methods only, and each goes in the directory Laravel uses for its
construct.

---

## make:model

There is no `make:model`. Write the model by hand with `#[suprnova::model]`,
or let `db:sync --regenerate-models` write it from the schema after a
migration runs. See [Eloquent](eloquent.md) and
[CLI Migrations](cli-migrations.md).

### Why Suprnova diverges

Laravel's `make:model` writes an empty class, because Eloquent reads the
columns from the database at run time. A Suprnova model is a struct whose
fields are its columns, checked when the crate compiles, so an empty model
is not a model. The struct has to name the columns, which either you or
`db:sync` know and a generator does not. `make:migration` writes the schema
change, and `db:sync` turns the migrated schema into model files.

---

## generate-types

Emit TypeScript interfaces from every Rust struct annotated with
`#[derive(InertiaProps)]`. The dev server runs this automatically; the
standalone command is for CI checks and one-shot regenerations.

```bash
suprnova generate-types [--output <PATH>] [--watch]
```

| Option | Default | Description |
|---|---|---|
| `-o, --output <PATH>` | `frontend/src/types/inertia-props.ts` | Output file path |
| `-w, --watch` | off | Watch source files and regenerate on change |

```bash
# One-shot
suprnova generate-types

# Watch mode (useful when you don't want to run the full dev server)
suprnova generate-types --watch

# Custom output path
suprnova generate-types --output frontend/src/types/props.ts
```

A Rust shape on the left produces a TypeScript interface on the right:

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

See [Frontend TypeScript Types](frontend-typescript-types.md) for the
full mapping table (enums, options, dates, nested structs) and the
override hooks.

---

### Why Suprnova diverges

Laravel's `php artisan make:*` drops a file in the right directory
and that's it - PSR-4 autoloading picks the new class up the next
time the framework boots. Rust has no equivalent. A file at
`src/foo/bar.rs` isn't compiled into the crate until `src/foo/mod.rs`
declares `pub mod bar;`, and the parent directory has to be wired up
the same way in `src/lib.rs`.

So every `suprnova make:*` generator does two things instead of one:
it writes the new file *and* edits the closest `mod.rs` (and, for
`make:task`, `src/lib.rs` and `cmd/main.rs` as well, and for `make:view`,
`src/lib.rs`). A nested name declares every module on the way to its
file, since each directory is a module of its own.

Laravel's `--test` writes a PHPUnit or Pest test under `tests/Feature`.
Suprnova's writes a Rust integration test under `tests/`, one file per
generated item, which imports the item from your library crate. That's why every generator prints a `Created src/.../mod.rs`
or `Updated src/.../mod.rs` line - the wiring is part of the work,
not a follow-up step you remember on your own.

---

## Summary

| Command | Creates | Wires into |
|---|---|---|
| `make:controller <name>` | `src/controllers/<snake>.rs` | `controllers/mod.rs` |
| `make:action <Name>` | `src/actions/<snake>_action.rs` | `actions/mod.rs` |
| `make:middleware <Name>` | `src/middleware/<snake>.rs` | `middleware/mod.rs` |
| `make:middleware <Dir>/<Name> --test` | `src/middleware/<dir>/<snake>.rs`, `tests/<dir>_<snake>_middleware.rs` | `middleware/<dir>/mod.rs`, `middleware/mod.rs` |
| `make:view <dir>.<name>` | `templates/<dir>/<name>.html`, `src/views/<dir>/<snake>.rs` | `views/<dir>/mod.rs`, `views/mod.rs`, `lib.rs` |
| `make:command <name>` | `src/commands/<snake>.rs` | `commands/mod.rs` (+ warns about `lib.rs`) |
| `make:error <Name>` | `src/errors/<snake>.rs` | `errors/mod.rs` |
| `make:task <Name>` | `src/tasks/<snake>_task.rs` | `tasks/mod.rs`, `schedule.rs`, `lib.rs`, `main.rs` |
| `make:inertia <Name>` | `frontend/src/pages/<Name>Page.<ext>` | (no module wiring) |
| `make:inertia <Dir>/<Name> --test` | `frontend/src/pages/<Dir>/<Name>Page.<ext>`, `tests/<dir>_<name>_page.rs` | (no module wiring) |
| `make:inertia <Name> --data` | `src/props/<snake>.rs` | (no module wiring; warns when `src/props/` is new) |
| `make:migration <name>` | `migrations/YYYYMMDDHHMMSS_<name>.rs` | (no module wiring) |
| `make:factory <Name> [--model <Model>]` | `src/factories/<snake>_factory.rs` | `factories/mod.rs`, `lib.rs` |
| `make:seeder <Name>` | `src/seeders/<snake>_seeder.rs` | `seeders/mod.rs`, `lib.rs` |
| `make:cast <Name>` | `src/casts/<snake>.rs` | `casts/mod.rs`, `lib.rs` |
| `make:channel <Name>` | `src/broadcasting/<snake>.rs` | `broadcasting/mod.rs`, `lib.rs` |
| `make:class <Name>` | `src/<snake>.rs` | `lib.rs` |
| `make:enum <Name>` | `src/enums/<snake>.rs` | `enums/mod.rs`, `lib.rs` |
| `make:event <Name>` | `src/events/<snake>.rs` | `events/mod.rs`, `lib.rs` |
| `make:listener <Name> [--event <Event>]` | `src/listeners/<snake>.rs` | `listeners/mod.rs`, `lib.rs` |
| `make:job <Name>` | `src/jobs/<snake>.rs` | `jobs/mod.rs`, `lib.rs` |
| `make:job-middleware <Name>` | `src/jobs/middleware/<snake>.rs` | `jobs/middleware/mod.rs`, `jobs/mod.rs`, `lib.rs` |
| `make:mail <Name>` | `src/mail/<snake>.rs` | `mail/mod.rs`, `lib.rs` |
| `make:notification <Name>` | `src/notifications/<snake>.rs` | `notifications/mod.rs`, `lib.rs` |
| `make:observer <Name> [--model <Model>]` | `src/observers/<snake>_observer.rs` | `observers/mod.rs`, `lib.rs` |
| `make:policy <Name> [--model <Model>]` | `src/policies/<snake>_policy.rs` | `policies/mod.rs`, `lib.rs` |
| `make:provider <Name>` | `src/providers/<snake>.rs` | `providers/mod.rs`, `lib.rs` |
| `make:resource <Name>` | `src/resources/<snake>_resource.rs` | `resources/mod.rs`, `lib.rs` |
| `make:rule <Name>` | `src/rules/<snake>.rs` | `rules/mod.rs`, `lib.rs` |
| `make:scope <Name> [--model <Model>]` | `src/scopes/<snake>_scope.rs` | `scopes/mod.rs`, `lib.rs` |
| `make:test <Name>` | `tests/<snake>.rs` | (no module wiring) |
| `make:trait <Name>` | `src/concerns/<snake>.rs` | `concerns/mod.rs`, `lib.rs` |
| `make:interface <Name>` | `src/contracts/<snake>.rs` | `contracts/mod.rs`, `lib.rs` |
| `generate-types` | `frontend/src/types/inertia-props.ts` | n/a |

## Next

- [CLI Overview](cli.md) - the full subcommand table
- [Console](console.md) - the per-project console binary that
  `make:command` feeds into
- [Controllers](controllers.md) - the handler contract `make:controller`
  scaffolds
- [Scheduling](scheduling.md) - the fluent schedule API used to
  register tasks generated by `make:task`
- [CLI Migrations](cli-migrations.md) - the migrate / db:sync
  commands that pair with `make:migration`
- [Eloquent Factories](eloquent-factories.md) and [Seeding](seeding.md) -
  what `make:factory` and `make:seeder` write fills in
- [Events](events.md) and [Queues](queues.md) - registering the events,
  listeners and jobs the generators write

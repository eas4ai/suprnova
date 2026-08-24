# Code-Generatoren

Die `suprnova make:*`-Familie scaffoldet die konventionelle Datei
für jedes Stück eines Projekts - einen Controller, eine Aktion, eine
Middleware, einen Console-Befehl, einen Domänenfehler, einen
geplanten Task, eine Inertia-Seite oder Props-Struktur, eine
Datenbank-Migration - und verdrahtet das neue Modul in sein
Eltern-`mod.rs` (und, wo nötig, in `src/lib.rs` und `cmd/main.rs`).
Greifen Sie darauf zurück, wenn Sie sonst dasselbe Boilerplate
+ `pub mod x;`-Import-Zeile erneut eintippen würden, was die meiste
+Zeit der Fall ist.

## make:controller

Scaffoldet einen Controller - eine Datei in `src/controllers/` mit
einer einzigen `#[handler]`-async-fn namens `invoke`.

```bash
suprnova make:controller User
suprnova make:controller order_item
```

Der Name wird für den Dateinamen zu `snake_case` normalisiert und
unverändert für das `controller:`-Echo in der Response verwendet.
Nur ASCII-Buchstaben, Ziffern und `_` werden akzeptiert - Pfade wie
`api/User` werden abgelehnt.

### Generierte Datei

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

### Was es verdrahtet

1. Schreibt `src/controllers/<name>.rs` mit der
   `#[handler]`-Funktion.
2. Fügt `pub mod <name>;` zu `src/controllers/mod.rs` hinzu (erstellt
   die Datei, falls sie nicht existierte).
3. Gibt einen Hinweis aus, eine Route in `src/routes.rs`
   hinzuzufügen: `.get("/<name>", controllers::<name>::invoke)`.

Siehe [Controller](controllers.md) für den Handler-Vertrag,
Extraktoren und das `routes!`-Makro.

---

## make:action

Scaffoldet eine Single-Responsibility-Aktion - eine aus dem
Container auflösbare Struktur mit einer asynchronen
`execute`-Methode, die ein `Result<String, FrameworkError>`
zurückgibt, sodass das Skelett kompiliert, bevor Sie den Rumpf
ausfüllen.

```bash
suprnova make:action CreateUser
suprnova make:action SendNotification
```

Der Name wird PascalCased; ein `Action`-Suffix wird angehängt,
falls er fehlt, und die Datei ist der snake-cased Struktur-Name.

### Generierte Datei

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

### Was es verdrahtet

1. Schreibt `src/actions/<snake>.rs`.
2. Fügt `pub mod <snake>;` zu `src/actions/mod.rs` hinzu.
3. `#[injectable]` registriert die Aktion beim Container zur
   Linkzeit, sodass jeder Controller sie über
   `App::get::<CreateUserAction>()` auflösen und
   `action.execute().await?` aufrufen kann.

Siehe [Aktionen](actions.md) für das Resolve-and-Invoke-Muster und
wie Aktionen mit dem Container komponieren.

---

## make:middleware

Scaffoldet eine Middleware - eine Unit-Struktur, die
`suprnova::Middleware` implementiert. Der Standard-Rumpf misst die
Zeit des inneren Handlers und protokolliert die eingehenden +
ausgehenden Events mit der Pro-Request-ID, sodass sie beim ersten
Mal end-to-end läuft.

```bash
suprnova make:middleware Auth
suprnova make:middleware RateLimit
```

Der Name wird PascalCased; ein `Middleware`-Suffix wird angehängt,
falls er fehlt. Die Datei verwendet den snake-cased Basisnamen (ohne
das Suffix), z. B. `Auth` → `src/middleware/auth.rs`, Struktur
`AuthMiddleware`.

### Generierte Datei

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

### Was es verdrahtet

1. Schreibt `src/middleware/<snake>.rs`.
2. Fügt `mod <snake>;` + `pub use <snake>::<StructName>;` zu
   `src/middleware/mod.rs` hinzu (erstellt sie, falls nötig).
3. Gibt sowohl die Pro-Route-Form
   (`.get("/path", handler).middleware(AuthMiddleware)`) als auch
   die globale Form (`global_middleware!(middleware::AuthMiddleware)`
   in `bootstrap.rs`) aus.

Siehe [Middleware](middleware.md) für die vollständige
Chain-Semantik, Reihenfolge und die Unterscheidung
global-vs-pro-Route.

---

## make:command

Scaffoldet einen Console-Befehl - eine
`#[derive(clap::Parser, Command)]`-Struktur, die die
projektspezifische `console`-Binary über `inventory` zur Linkzeit
aufnimmt. Der Standard-Rumpf ist ein
`println!("…: not yet implemented")`, sodass der Befehl sofort läuft.

```bash
suprnova make:command CleanCache
suprnova make:command mail:send
suprnova make:command clean-cache
```

Die Namensgebung folgt drei Regeln:

- Eingaben, die `:` enthalten, werden wörtlich als registrierter
  Befehlsname verwendet (Laravel-Namespace-Stil: `db:seed`,
  `mail:send`).
- Andernfalls wird der snake-cased Funktionsname für den
  registrierten Namen kebabbed (`CleanCache` → Befehl `clean-cache`).
- Die Rust-Datei und die Struktur sind immer snake-cased /
  PascalCased Formen desselben Identifiers.

### Generierte Datei

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

### Was es verdrahtet

1. Schreibt `src/commands/<snake>.rs`.
2. Fügt `pub mod <snake>;` zu `src/commands/mod.rs` hinzu (erstellt
   sie, falls nötig).
3. Warnt sichtbar, falls in `src/lib.rs` `pub mod commands;` fehlt -
   ohne das linkt der Befehl nicht in die console-Binary.
4. Gibt den Ausführungsbefehl aus:
   `cargo run --bin console -- clean-cache`.

Siehe [Konsole](console.md) für die vollständige
Typed-Command-Oberfläche, die `#[command]`-Kurzform für
argv-only-Handler und die Rolle der projektspezifischen
console-Binary.

---

## make:error

Scaffoldet einen Domänenfehler - eine mit `#[domain_error]`
annotierte Unit-Struktur, sodass sie von Haus aus einen HTTP-Status,
eine `Display`-Nachricht und eine
`From<…> for FrameworkError`-Impl mitbringt.

```bash
suprnova make:error UserNotFound
suprnova make:error PaymentFailed
```

Der Name wird PascalCased für die Struktur und snake-cased für die
Datei. Der Standard-Status ist 500 und die Nachricht ist der
sentence-cased Struktur-Name - ändern Sie beide Attribute in der
generierten Datei passend zur Situation.

### Generierte Datei

```rust
// src/errors/user_not_found.rs
use suprnova::domain_error;

#[domain_error(status = 500, message = "User not found")]
pub struct UserNotFound;
```

Ändern Sie `status = 500` zu dem, was passt - `404` für Not-Found,
`402` für Payment-Required, `403` für Forbidden - und bearbeiten Sie
die Nachrichtenzeichenkette. Für reichhaltigere Payloads fügen Sie
der Struktur benannte Felder hinzu und referenzieren Sie sie in der
Nachricht über Interpolation in einer handgerollten `Display`-Impl
(lassen Sie das `#[domain_error]`-Makro an diesem Punkt weg).

### Was es verdrahtet

1. Schreibt `src/errors/<snake>.rs`.
2. Fügt `pub mod <snake>;` zu `src/errors/mod.rs` hinzu (erstellt
   sie, falls nötig).
3. Weist darauf hin, `mod errors;` in `src/lib.rs` zu deklarieren,
   falls das Verzeichnis `errors/` frisch erstellt wurde.

### Verwendung

Innerhalb eines Handlers, der `Response` zurückgibt, heben Sie den
Domänentyp zu einem `FrameworkError` an, sodass `?` sauber
kurzschließt:

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

Das Kapitel [Fehlerbehandlung](errors.md) behandelt die
vollständige Custom-Error-Geschichte, einschließlich wann
`#[domain_error]` vs. `AppError::bad_request(…)` vs. eine
handgerollte `HttpError`-Impl zu verwenden ist.

---

## make:task

Scaffoldet einen geplanten Task - eine Unit-Struktur, die
`suprnova::Task` implementiert und strukturierte Start-/Ende-Zeilen
ausgibt, sodass das Scaffold Fortschritt protokolliert, bevor Sie
den echten Rumpf ausfüllen.

```bash
suprnova make:task CleanupLogs
suprnova make:task SendReminders
```

Der Name wird PascalCased; ein `Task`-Suffix wird angehängt, falls
er fehlt. Die Datei ist der snake-cased Struktur-Name, z. B.
`CleanupLogs` → `src/tasks/cleanup_logs_task.rs`.

### Generierte Datei

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

### Was es verdrahtet

Der erste `make:task`-Aufruf verdrahtet umfangreicher als die
anderen Generatoren - er erstellt die Oberfläche des Schedulers im
Projekt von Grund auf:

1. Erstellt `src/tasks/` und `src/tasks/mod.rs`, falls sie fehlen.
2. Erstellt `src/schedule.rs` (den
   `register(schedule: &mut Schedule)`-Einstiegspunkt), falls sie
   fehlt.
3. Deklariert `pub mod schedule;` und `pub mod tasks;` in
   `src/lib.rs`.
4. Fügt `.schedule(<crate>::schedule::register)` in die
   `Application::new()`-Chain in `cmd/main.rs` oder `src/main.rs`
   ein, direkt vor `.run()`.
5. Schreibt `src/tasks/<snake>.rs` und fügt sie zu
   `src/tasks/mod.rs` hinzu.

Nachfolgende Aufrufe überspringen die Schritte, die schon gelaufen
sind.

### Den Task registrieren

Öffnen Sie `src/schedule.rs` und fügen Sie einen
Registrierungsaufruf mit der fluent Schedule-API hinzu:

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

Führen Sie dann den Scheduler aus:

```bash
suprnova schedule:work   # Daemon - prüft jede Minute
suprnova schedule:run    # einmalig - wird typischerweise von cron aufgerufen
suprnova schedule:list   # zeigt jeden registrierten Task
```

Siehe [Task-Planung](scheduling.md) für die vollständige
Task-Oberfläche (`hourly`, `weekly`, `cron(...)`, `between`, `when`,
`without_overlapping`, Zeitzonen-Handling) und
[Befehlsplanung](cli-scheduling.md) für den Trade-off zwischen
Run-as-Cron und Run-as-Daemon.

---

## make:inertia

Scaffoldet je nach Flag entweder eine Inertia-Seiten-Komponente
(Standard) oder eine typisierte Data-Struktur (`--data`). Der
Seiten-Generator erkennt das Frontend-Framework (Svelte 5, React 19,
Vue 3.5) aus `.env` und gibt die passende Dateiendung aus.

### Seiten-Modus (Standard)

```bash
suprnova make:inertia About
suprnova make:inertia UserProfile
```

Der Name wird PascalCased und das Suffix `Page` wird angehängt,
falls es fehlt, sodass `About` → `AboutPage` wird. Die Datei landet
in `frontend/src/pages/` mit der Pro-Frontend-Endung:
`AboutPage.svelte` für Svelte, `AboutPage.tsx` für React,
`AboutPage.vue` für Vue.

Beispiel (Svelte):

```svelte
<!-- frontend/src/pages/AboutPage.svelte -->
<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">AboutPage</h1>
  <p class="mt-2">
    Edit <code class="bg-gray-100 px-1 rounded">frontend/src/pages/AboutPage.svelte</code> to get started.
  </p>
</div>
```

Rendern Sie sie aus einem Controller:

```rust
inertia_response!(&req, "AboutPage", props)
```

Siehe [Seiten-Komponenten](frontend-pages.md) und
[Inertia Responses](frontend-inertia-responses.md) für die Brücke
zwischen Controllern und Seiten, Partial Reloads und geteilte Props.

### Data-Struktur-Modus (`--data`)

```bash
suprnova make:inertia UserProps --data
```

Gibt eine `#[derive(Data, Validate)]`-Struktur in `app/src/props/`
aus (nicht `src/props/` - das `app/`-Präfix ist hartcodiert, sodass
die Datei in der Beispiel-/Host-App des Workspace landet):

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

Verwenden Sie sie in einem Controller, um Request-Bodys zu
validieren:

```rust
let dto: UserProps = req.validate_json().await?;
```

---

## make:migration

Scaffoldet eine zeitstempelte SeaORM-Migrationsdatei. Im Detail
behandelt in [CLI Migrationen](cli-migrations.md), das auch die
Befehle `migrate` / `migrate:rollback` / `migrate:status` /
`migrate:fresh` / `db:sync` durchgeht. Die Kurzform:

```bash
suprnova make:migration create_users_table
```

Der Migrationsname wird wörtlich beibehalten und mit einem
`YYYYMMDDHHMMSS_`-Stempel versehen, sodass Dateien chronologisch
sortieren. Die generierte Datei landet in `migrations/`.

Siehe [Migrationen](migrations.md) für die Schema-Builder-Oberfläche
und [Datenbank-Tests](database-testing.md) für das
`TestDatabase::fresh`-Muster, das Migrationen gegen eine pro Test
isolierte Datenbank ausführt.

---

## generate-types

Gibt TypeScript-Interfaces aus jeder mit `#[derive(InertiaProps)]`
annotierten Rust-Struktur aus. Der Dev-Server führt das automatisch
aus; der eigenständige Befehl ist für CI-Checks und einmalige
Regenerationen.

```bash
suprnova generate-types [--output <PATH>] [--watch]
```

| Option | Standard | Beschreibung |
|---|---|---|
| `-o, --output <PATH>` | `frontend/src/types/inertia-props.ts` | Pfad der Ausgabedatei |
| `-w, --watch` | off | Beobachtet Quelldateien und regeneriert bei Änderung |

```bash
# Einmalig
suprnova generate-types

# Watch-Modus (nützlich, wenn Sie nicht den vollen Dev-Server ausführen wollen)
suprnova generate-types --watch

# Eigener Ausgabepfad
suprnova generate-types --output frontend/src/types/props.ts
```

Eine Rust-Form links erzeugt ein TypeScript-Interface rechts:

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

Siehe [TypeScript Types](frontend-typescript-types.md) für die
vollständige Mapping-Tabelle (Enums, Options, Datumstypen,
verschachtelte Strukturen) und die Override-Hooks.

---

### Warum Suprnova abweicht

Laravels `php artisan make:*` legt eine Datei ins richtige
Verzeichnis, und das war's - PSR-4-Autoloading nimmt die neue Klasse
beim nächsten Boot des Frameworks auf. Rust hat kein Äquivalent.
Eine Datei unter `src/foo/bar.rs` wird erst in die Crate kompiliert,
wenn `src/foo/mod.rs` `pub mod bar;` deklariert, und das
Elternverzeichnis muss auf dieselbe Art in `src/lib.rs` verdrahtet
werden.

Also macht jeder `suprnova make:*`-Generator zwei Dinge statt eines:
Er schreibt die neue Datei *und* bearbeitet das nächstliegende
`mod.rs` (und, für `make:task` und `make:command`, auch `src/lib.rs`
und `cmd/main.rs`). Deshalb gibt jeder Generator eine Zeile
`Created src/.../mod.rs` oder `Updated src/.../mod.rs` aus - die
Verdrahtung ist Teil der Arbeit, kein Folgeschritt, an den Sie sich
selbst erinnern müssen.

---

## Zusammenfassung

| Befehl | Erstellt | Verdrahtet in |
|---|---|---|
| `make:controller <name>` | `src/controllers/<snake>.rs` | `controllers/mod.rs` |
| `make:action <Name>` | `src/actions/<snake>_action.rs` | `actions/mod.rs` |
| `make:middleware <Name>` | `src/middleware/<snake>.rs` | `middleware/mod.rs` |
| `make:command <name>` | `src/commands/<snake>.rs` | `commands/mod.rs` (+ warnt wegen `lib.rs`) |
| `make:error <Name>` | `src/errors/<snake>.rs` | `errors/mod.rs` |
| `make:task <Name>` | `src/tasks/<snake>_task.rs` | `tasks/mod.rs`, `schedule.rs`, `lib.rs`, `main.rs` |
| `make:inertia <Name>` | `frontend/src/pages/<Name>Page.<ext>` | (keine Modul-Verdrahtung) |
| `make:inertia <Name> --data` | `app/src/props/<snake>.rs` | (keine Modul-Verdrahtung) |
| `make:migration <name>` | `migrations/YYYYMMDDHHMMSS_<name>.rs` | (keine Modul-Verdrahtung) |
| `generate-types` | `frontend/src/types/inertia-props.ts` | n/a |

## Nächste Schritte

- [CLI - Übersicht](cli.md) - die vollständige Subkommando-Tabelle
- [Konsole](console.md) - die projektspezifische console-Binary, in
  die `make:command` einspeist
- [Controller](controllers.md) - der Handler-Vertrag, den
  `make:controller` scaffoldet
- [Task-Planung](scheduling.md) - die fluent Schedule-API, mit der
  von `make:task` generierte Tasks registriert werden
- [CLI Migrationen](cli-migrations.md) - die migrate-/db:sync-Befehle,
  die zu `make:migration` passen

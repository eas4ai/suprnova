# Laravel Parity Map

Die ehrliche Feature-für-Feature-Zuordnung zwischen Laravel 13.x und
Suprnova. Nutzen Sie diese Seite, wenn Sie fragen „Hat Suprnova X?“ und
eine Ja/Nein/Wo-Antwort in einer Zeile wollen.

Die Abschnitte spiegeln den Laravel-Doku-Index, damit ein
Laravel-Entwickler von oben nach unten scannen kann. Innerhalb jedes
Abschnitts sind die Spalten immer dieselben:

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|

Die Spalte **Status** verwendet vier Werte:

| Symbol | Bedeutung |
|---|---|
| **ausgeliefert** | Gleiche Oberfläche, gleiches Verhalten (oft dieselben Methodennamen) |
| **abweichend** | Gleiche Aufgabe, andere Form, weil Rust eine bessere Wahl möglich macht |
| **noch nicht** | Wirklich geplant, aber noch nicht umgesetzt |
| **absichtlich nicht** | Wird nicht ausgeliefert - Erklärung in der Spalte Hinweise |

Das jeweilige Kapitel (wo vorhanden) ist von der Spalte **Hinweise** aus
verlinkt.

Dies ist eine lebende Karte. Suprnova liefert jede Laravel-13.x-Oberfläche
über die 30 dokumentierten Domänen hinweg aus; die unten aufgeführten
Lücken sind die echten, aktuellen Lücken zum Stand des ausgelieferten
Frameworks.

## Architekturkonzepte

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Request Lifecycle | `Application` → `Server` → `handle_request`-Kette | ausgeliefert | [Request-Lifecycle](lifecycle.md) |
| Service Container | `Container` + `App`-Facade, dreischichtig (Task / Thread / Global) | abweichend | Task-lokal pro Anfrage, Thread-lokal für Tests - [Service Container](container.md) |
| Service Providers | `bootstrap()`-Funktion + `#[service]`, `#[policy]`, `#[command]`, Observer-Makros | abweichend | Keine Registrierungsklasse - Bootstrap ist eine einzige Funktion; Makros nutzen `inventory` für die Compile-Zeit-Registrierung. [Bootstrap](bootstrap.md) |
| Facades | Statisches `App::get`, `Cache::*`, `Mail::*`, `Auth::*`, `Storage::*`, `Queue::*`, `Bus::*`, `Event::*`, `Notification::*`, `Gate::*`, `Schedule::*`, `DB::*`, `Vector::*` | ausgeliefert | Gleiche Aufrufform; die Facades sind echte Typen, keine Aliase |
| Contracts | Traits - `Mailer`, `KeyValueStore`, `Hasher`, `Channel`, `VectorDriver`, `Evaluator`, `PaymentProvider` usw. | ausgeliefert | Alle öffentlichen Schnittstellen leben auf Traits; binden Sie per Trait, tauschen Sie Implementierungen frei aus |

## Erste Schritte

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Installation | `cargo install --git …suprnova-cli` dann `suprnova new <name>` | ausgeliefert | [Installation](installation.md) |
| Konfiguration | Typisierte Config via `#[derive(Config)]` + `Config::register` | abweichend | Zur Compile-Zeit typisiert statt Array-Bags. [Konfiguration](configuration.md) |
| Agentische Entwicklung (KI) | Kein First-Class-KI-SDK im Framework | absichtlich nicht | Nutzen Sie die Crates, die Sie ohnehin nutzen würden (`async-openai`, `anthropic-rs`, `tokenizers` usw.) unter `App::bind(Arc<dyn YourLlm>)` |
| Verzeichnisstruktur | `src/{actions,bootstrap,controllers,middleware,models,routes}` | ausgeliefert | Gleiche Absicht, Rust-idiomatisches Layout. [Verzeichnisstruktur](structure.md) |
| Frontend | Inertia v3 über Svelte 5 / React 19 / Vue 3.5 | ausgeliefert | [Frontend](frontend.md), [Seiten](frontend-pages.md), [TS Types](frontend-typescript-types.md) |
| Starter Kits | **Nebula** (Auth) und **Pulsar** (vollständige Produkt-Site), plus das reine `suprnova new`-Scaffold | ausgeliefert | Zwei Kits werden heute ausgeliefert - Nebula ist das Breeze-Äquivalent; Pulsar bringt Docs, Blog, Community und RBAC mit. [Starter-Kits](starter-kits.md) |
| Deployment | Eine einzige Binärdatei; Docker-/Railway-/DO-/Hetzner-Rezepte | abweichend | Ein Artefakt, keine PHP-Runtime + Opcache + FPM. [Bereitstellung](deployment.md) |

## Die Grundlagen

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Route-Definitionen | `routes!`-Makro + `get!` / `post!` / `put!` / `patch!` / `delete!` / `any!` / `head!` / `options!` / `fallback!` / `ws!` | ausgeliefert | [Routing](routing.md) |
| Route-Parameter | `{id}`-Pfadparameter + `req.param("id")` | ausgeliefert | Optionale Parameter via `{id?}`; Constraints via `where!()` |
| Route-Namen | `.name("posts.show")` auf der Route + `url("posts.show", &[("id", "42")])` | ausgeliefert | [URL-Generierung](urls.md) |
| Route-Gruppen | `group!`-Makro mit `.prefix()` / `.middleware()` / `.name()` / `.controller()` | ausgeliefert | Gruppen-Middleware wird zur Registrierungszeit auf jede Route abgeflacht |
| Resource-Routes | `resource!("posts", PostController)` registriert die 7 Standard-Routen | ausgeliefert | `apiResource!`, `only(...)`, `except(...)` werden alle unterstützt |
| Signed URLs | `sign_url(...)`, `sign_route(...)`, `verify_signature(...)` | ausgeliefert | HMAC-SHA256 mit `APP_KEY` |
| Route Model Binding | `#[handler]` extrahiert `Post` aus `{post}` via `RouteBinding`-Impl | ausgeliefert | `AutoRouteBinding`-Derive implementiert automatisch für `#[suprnova::model]`-Typen |
| Ratenbegrenzung | `throttle:60,1`-Middleware + `RateLimiter::for_signature` | ausgeliefert | [Ratenbegrenzung](rate-limiting.md) |
| Middleware | `impl Middleware`-Trait; global oder pro Route registrieren | ausgeliefert | [Middleware](middleware.md) |
| Middleware-Gruppen + Aliase | `register_middleware_group`, `register_middleware_alias` | ausgeliefert | Nachschlagen per String-Name in den Routen |
| CSRF-Schutz | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | ausgeliefert | Origin-Policy erzwingt Same-Origin-POST. [CSRF](csrf.md) |
| Controller | `#[handler] pub async fn show(req: Request) -> Response` | ausgeliefert | Controller sind Module aus freien Funktionen, keine Klassen. [Controller](controllers.md) |
| Single-Action-Controller | Ein Handler ist bereits eine einzelne Funktion; in Module gruppieren | ausgeliefert | Die Rust-Konvention - keine `__invoke`-Zeremonie |
| Anfragen | `Request`-Struktur mit `.input()`, `.param()`, `.query()`, `.header()`, `.cookie()`, `.json()`, `.file()` usw. | ausgeliefert | [Anfragen](requests.md) |
| Form Requests | `#[derive(Data, Validate, FormRequest)]` | ausgeliefert | Validierung läuft beim Extrahieren |
| Datei-Uploads | `req.file("avatar")?` gibt `UploadedFile` zurück; streamendes Multipart mit Größen- und Teil-Caps | ausgeliefert | Auto-Spill in eine Tempdatei oberhalb des Schwellwerts |
| Antworten | `HttpResponse`-Builder + `json!()` / `text!()` / `Redirect::to` / `view` | ausgeliefert | [Antworten](responses.md) |
| Views (Blade) | Server-gerenderte Inertia-Seiten (Svelte/React/Vue) - kein Blade-Äquivalent | abweichend | Inertia ist die View-Schicht. Nutzen Sie [Seiten](frontend-pages.md) statt Blade |
| Asset Bundling (Vite) | Vite 8 ist in jedem Scaffold enthalten; `suprnova serve` betreibt Vite + Backend zusammen | ausgeliefert | Manifest-Lesen + HMR automatisch verdrahtet |
| Statische Assets (`public/`, in Laravel vom Webserver ausgeliefert) | `StaticFiles::public()` In-Process-Fallback-Handler, liefert `public/` an der Web-Root aus | ausgeliefert | `StaticFiles::from_dir(...)` + `cache_control(...)`; kein separater Webserver nötig |
| URL-Generierung | `url("posts.show", &[…])`, `route("posts.show", …)`, `redirect(...)`, `redirect_to(...)` | ausgeliefert | [URL-Generierung](urls.md) |
| Session | `session()`, `session_mut()`, Flash-Bag via `req.flash()` | ausgeliefert | DB-gestützt via `DatabaseSessionDriver`; standardmäßig Cookie-gestützt. [Sitzungen](session.md) |
| Validierung | `#[derive(Validate)]` + 17 eingebaute Regeln + `Rule`/`AsyncRule`-Traits | ausgeliefert | Asynchrone Regeln (z. B. `Unique`) greifen auf die DB zu. [Validierung](validation.md) |
| Fehlerbehandlung | `FrameworkError`, `AppError`, `HttpError`-Trait, Panic-Grenze in `execute_chain_safely` | ausgeliefert | [Fehlerbehandlung](errors.md), [Fehlermodell](error-model.md) |
| Protokollierung | `tracing`-Subscriber mit strukturierten Feldern, `LogFormat` (json / pretty / compact) | abweichend | Eine Log-Zeile ist ein JSON-Dokument; `request_id` ist immer vorhanden. [Protokollierung](logging.md) |
| Abort-Helfer | `abort_if(cond, status, msg)`, `abort_unless(...)`, `abort_with(status, msg)` | ausgeliefert | Gleiche Form wie Laravels `abort_if`-Familie |

## Tiefere Einblicke

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Artisan Console | Pro-App-`console`-Binary, gebaut aus `#[command]` + `#[derive(Command)]` | ausgeliefert | [Konsole](console.md). `cargo run --bin console <subcommand>` |
| Tinker (REPL) | Kein REPL | absichtlich nicht | Schreiben Sie ein einmaliges `cargo run --bin xxx`-Skript oder einen `#[suprnova_test]` |
| Broadcasting | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | ausgeliefert | sea-streamer-Fanout für Multi-Node. [Broadcasting](broadcasting.md) |
| Cache | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`, `RedisCache` | ausgeliefert | Atomare Operationen + getaggter Cache + Cache-Sperren (`LockGuard`). [Cache](cache.md) |
| Collections | `eloquent::Collection<M>` mit Laravel-förmigen Methoden | ausgeliefert | `Deref<Target = Vec<M>>`, sodass bestehende Vec-Idiome weiter funktionieren. [Collections](eloquent-collections.md) |
| Concurrency | Tokio überall - `tokio::spawn`, `tokio::join!`, `tokio::select!` | ausgeliefert | Das gesamte Framework ist async. Die Laravel-Facade `Concurrency::run([...])` wird nicht ausgeliefert; Tokio ist die Antwort |
| Context | `Context::put` / `Context::get` / `ContextStore` + Auto-Injektion in Queue / Mail / Events | ausgeliefert | [Kontext](context.md) |
| Contracts | Alle öffentlichen Schnittstellen sind Traits | ausgeliefert | Siehe die Zeile „Architecture / Contracts“ oben |
| Ereignisse | `EventFacade::dispatch(e).await?`, `#[derive(Event)]`, `EventDispatcher`, gequeuete Listener, Subscriber | ausgeliefert | [Ereignisse](events.md) |
| Dateispeicher | `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` über OpenDAL | ausgeliefert | Gleiche Oberfläche `put/get/delete/copy/move/exists/url`. Path-Traversal-Schutz eingebaut. [Dateisystem](filesystem.md) |
| Helpers | Äquivalente liegen in ihren Heimat-Modulen (kein Kitchen-Sink-`helpers.md`) | abweichend | Z. B. leben URL-Helfer in [urls.md](urls.md), String-Helfer in `std`/`heck`, Array-Helfer in `std::collections` - Rust löst das mit Crates, nicht mit einem globalen Namensraum |
| HTTP Client | `Http::get/post/...`-Builder + `Http::fake(...)` für Tests | ausgeliefert | Zeichnet Requests automatisch auf; `assert_sent` / `assert_not_sent`. [HTTP-Client](http-client.md) |
| Lokalisierung | `Lang::get` / `get_with` / `try_get` / `has` + das Makro `__!("key", name: value)` über Fluent-`.ftl`-Kataloge in `lang/<locale>/`, `LocaleMiddleware`-Erkennung, übersetzte Validierungsmeldungen, ICU4X-Formatierung | ausgeliefert | Derselbe Katalog wird dem Browser unter `/_suprnova/lang/<locale>.ftl` ausgeliefert und von `generate-types` typisiert. [Lokalisierung](localization.md) |
| Mail | `Mail::to(...).send(MyMail { ... }).await?` + Treiber `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory` | ausgeliefert | `Mailable`-Trait + Tera-gerenderte HTML/Text-Bodies. [Mail](mail.md) |
| Benachrichtigungen | `Notify::send(&user, notif).await?` + Kanäle `mail/database/broadcast/webpush` | ausgeliefert | `Notifiable`-Trait + `Notification` pro Kanal. [Benachrichtigungen](notifications.md), [Web Push](web-push.md) |
| Package Development | Workspace-Adapter-Crates (z. B. `suprnova-payments-stripe`) | ausgeliefert | Gleiche Form wie Laravel-Packages: hängt vom Framework ab, bindet in den Container, exponiert bei Bedarf Makros |
| Processes (Shell-Befehle ausführen) | `tokio::process::Command` aus der Stdlib | absichtlich nicht | Keine Facade - Tokios API hat bereits die richtige Form |
| Warteschlangen | `Queue::push(job).await?` + Treiber `sync/memory/database/redis/null`, Batches, Ketten, `JobMiddleware`, `FailedJobStore` | ausgeliefert | [Warteschlange](queues.md) |
| Ratenbegrenzung | `RateLimiter::for_signature(...)`, `ThrottleRequestsMiddleware`, `RateLimitMiddleware` | ausgeliefert | Sliding Window via `SlidingWindowConfig`. [Ratenbegrenzung](rate-limiting.md) |
| Suche (Scout) | Kein First-Party-Volltextsuche-Adapter | noch nicht | Vektorsuche wird heute ausgeliefert über [Vector](vector.md); ein Scout-Äquivalent für Keyword-Suche ist geplant |
| Strings (Helfer) | `heck`-Crate (Case-Konvertierungen), `std::str`, `regex` | abweichend | Dieselben Crates, die der Rest des Rust-Ökosystems nutzt; kein globales `Str::camel($x)` |
| Task-Planung | `Schedule::call/command/task` + `#[derive(Task)]` + Cron-Syntax + `schedule:run`-Worker | ausgeliefert | [Task-Planung](scheduling.md) |
| Idempotenzschlüssel | `Idempotency::remember(key, ttl, body)` - Stripe-artiger Replay-Schutz | ausgeliefert | Der Aufrufer versieht den Key mit der Route + Nutzer-/Geschäftsidentität als Namensraum. [Idempotenz](idempotency.md) |
| Request-Timeouts | `TimeoutMiddleware` pro Route konfigurierbar | ausgeliefert | Rust-nativ - bricht das laufende Future ab, gibt den Worker frei. [Timeout](timeout.md) |
| Feature Flags (Pennant) | `Feature` + `Evaluator` + `FeatureMiddleware` + Admin-CRUD | ausgeliefert | Propagierung unter einer Sekunde via `FeatureSync`-Trait. [Feature Flags](feature-flags.md) |
| Beobachtbarkeit (Pulse) | OpenTelemetry via `init_telemetry`, `Metrics`, `tracing` überall | abweichend | OTel ist die Lingua franca für Rust-Observability - richten Sie Ihren Collector auf die Binary. [Beobachtbarkeit](observability.md) |
| Telescope (Debug-Dashboard) | Noch kein Äquivalent | noch nicht | Auf v2+ verschoben; die Tracing- + OTel-Ausgabe des Frameworks deckt die meisten Diagnosebedürfnisse ab |
| Pulse (Perf-Dashboard) | Noch kein Äquivalent | noch nicht | Wie Telescope - Metriken mit Ihrem bestehenden Observability-Stack sichtbar machen, bis ein Dashboard ausgeliefert wird |
| Vector-Suche | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | ausgeliefert | Kein „nur Postgres pgvector“-Gatekeeping. [Vector-Suche](vector.md) |

### Suprnova-exklusiv (kein Laravel-Äquivalent)

| Suprnova | Was es ist | Hinweise / Link |
|---|---|---|
| `ws!()`-Makro + WebSocket-Handler | Typisierte WS-Routen, die sich Router + Middleware-Stack teilen | [WebSockets](websockets.md) |
| Server-Sent Events | `SseEvent` + `HttpResponse::sse(...)` | [SSE](sse.md) |
| Workflows | Lang laufende, zustandsbehaftete Arbeit mit Wiederholungen, Sleep, Schritt-Grenzen | [Workflows](workflows.md) |
| Supervisoren | `Supervisor`-Trait mit Panic-Catch-Auto-Restart für langlebige Tokio-Tasks | [Supervisoren](supervisors.md) |
| Web Push (VAPID) | Browser-Push-Benachrichtigungen als First-Class-Kanal | [Web Push](web-push.md) |
| Multi-Connection Read/Write Split | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [Datenbank](database.md) |
| HTTP/2 + WebSocket auf demselben Socket | `hyper.with_upgrades()` in `Server::run` | [Request-Lifecycle](lifecycle.md) |
| Markdown-Content + Docs-Pipeline | `MarkdownRenderer` (sanitisiert comrak → syntect → ammonia) + `build_docs(DocsBuildConfig)` → durchsuchbarer `DocsCatalog` aus `DocsChapter`s | Heading-Extraktion + `slugify_heading`; treibt Markdown-Docs/Blog ohne separaten Static-Site-Generator |

## Sicherheit

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Authentifizierung | `Auth::user/check/login/logout/attempt`, `Authenticatable`-Trait, `Guard` pro Name | ausgeliefert | [Authentifizierung](authentication.md) |
| Mehrere Guards | `Guard`, per Name registriert (`web`, `api`, …) via `AuthManager` | ausgeliefert | `SessionGuard`, `TokenGuard`, eigene Impls |
| User Providers | `EloquentUserProvider<U>`, `DatabaseUserProvider`, eigene via `UserProvider`-Trait | ausgeliefert | [Auth-Flows](auth-flows.md) |
| Email-Verifizierung | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail`; `MustVerifyEmail`-Contract auf dem User-Modell | ausgeliefert | Provider-gestützt (kein torii) - [Auth-Flows](auth-flows.md) |
| Passwort zurücksetzen | `PasswordReset` + `PasswordResetMail` + `PasswordChangedMail`; `CanResetPassword`-Contract auf dem User-Modell | ausgeliefert | Provider-gestützt (kein torii) - [Auth-Flows](auth-flows.md) |
| Brute-Force-Drosselung | `BruteForce` + `LoginThrottleMiddleware` | ausgeliefert | Buchführung pro IP + pro Nutzer |
| Zwei-Faktor (TOTP) | `TwoFactor` + `TwoFactorChallengeMiddleware` + `TwoFactorUser`-Trait | ausgeliefert | Recovery-Codes + Replay-Schutz |
| Remember-me | Langlebiges signiertes Cookie via `SessionGuard` | ausgeliefert | Framework-eigenes `auth::remember`: DB-Zeile + bcrypt + Single-Use-Rotation |
| OAuth (Socialite) | Über den vendorten `torii_integration`-Fork (Google / GitHub / Apple usw.) | ausgeliefert | [Authentifizierung](authentication.md) |
| Sanctum (API-Tokens) | `TokenGuard` + DB-gestützte Tokens via torii | abweichend | Token-Modell + Bearer-Middleware werden ausgeliefert; keine separate Sanctum-API-Oberfläche |
| Passport (OAuth-Server) | Noch nicht | noch nicht | Wenn Sie einen OAuth-Provider brauchen, betreiben Sie einen dedizierten Identity-Service (Keycloak, Hydra) hinter Suprnova |
| Fortify (Auth-Backend) | Ersetzt durch das Modul `auth_flows` + `auth_flows::*`-Typen | ausgeliefert | Gleiche Aufgabe; kein Headless-vs-Headed-Split nötig, weil das Frontend Inertia ist |
| Autorisierung (Policies / Gates) | `Gate::allows/denies` + `#[policy] impl PostPolicy` + `Authorizable`-Trait + Makro-Registrierung | ausgeliefert | [Autorisierung](authorization.md) |
| Rollen & Berechtigungen (spatie/laravel-permission) | `HasRoles`-Trait + Tabellen `roles` / `permissions` / `role_has_permissions` (`CreateRbacTables`) + `RoleMiddleware` / `PermissionMiddleware` (Fail-Closed) | ausgeliefert | First-Party, kein Community-Package. Helfer `create_role` / `give_permission_to_role` / `assign_role_to_model`; setzt auf Gate/Policy auf. [Autorisierung](authorization.md) |
| Verschlüsselung | `Crypt::encrypt/decrypt` + `CryptPurpose`-AAD-Bindung | ausgeliefert | AES-256-GCM, Key-Rotation via `APP_KEY_PREVIOUS`. [Verschlüsselung](encryption.md) |
| Hashing | `hash::*` + `BcryptHasher`, `Argon2idHasher`, `Argon2iHasher`, `needs_rehash`, `is_hashed`, `verify` | ausgeliefert | Bcrypt als Standard; argon2id verfügbar. [Hashing](hashing.md) |

## Datenbank

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| DB::table('users')->where(...)->get() | `DB::table("users").db_where("id", "=", 1).get().await?` | ausgeliefert | [Datenbank](database.md), [Query Builder](queries.md) |
| Mehrere Connections | `DB::on("read")` + `ConnectionRegistry` | ausgeliefert | Read/Write-Split first-class |
| Transaktionen | `DB::transaction(\|tx\| async move { ... }).await?` | ausgeliefert | Savepoints + Wiederholung bei Deadlock |
| Query Events | `QueryListener` + `QueryExecuted`-Event | ausgeliefert | `DB::listen(\|q\| { ... })` |
| Rohe Ausdrücke | `DB::raw("...")`, `DB::select("...", &[...])` | ausgeliefert | Parameter-Bindung erforderlich (keine String-Interpolation) |
| Postgres / MySQL / SQLite | Alle drei first-class via SeaORM | ausgeliefert | URL-Erkennung in `database::config::database_type()` |
| MariaDB | Eigenständig first-class (Vector + JSON + Temporal) | abweichend | Separat behandelt wegen der Multi-Paradigmen-Features, die Laravel nur für Postgres ausliefert |
| Redis | Von Treibern genutzt (Cache/Queue/Rate-Limit) - keine separate `Redis::*`-Facade | abweichend | Greifen Sie direkt zur `redis`-Crate, wenn Sie Ad-hoc-Befehle brauchen; Cache/Queue/Rate-Limit decken 95 % der typischen Nutzung ab |
| MongoDB | Noch kein First-Party-Adapter | noch nicht | Nutzen Sie die `mongodb`-Crate direkt via `App::bind` |
| Query Builder | `Builder<M>` mit `db_where` / `or_where` / `where_in` / `where_between` / `where_null` / `where_has` / `with` / `with_count` / `order_by` / `group_by` / `having` / `paginate` usw. | ausgeliefert | [Query Builder](queries.md) |
| Paginierung | `LengthAwarePaginator`, `Paginator` (einfach), `CursorPaginator` | ausgeliefert | Alle drei serialisieren in Laravel-Form-JSON. [Paginierung](pagination.md) |
| Migrationen | `#[derive(DeriveMigrationName)] struct M;` + `up`/`down` + `Migrator` | ausgeliefert | Ausführung via `suprnova migrate`/`migrate:rollback`/`migrate:status`/`migrate:fresh`. [Migrationen](migrations.md), [CLI Migrationen](cli-migrations.md) |
| Seeders | `Seeder`-Trait + `db:seed`-Subcommand | ausgeliefert | Pro-Modell-Factories. [Seeding](seeding.md) |

## Eloquent ORM

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| `class User extends Model` | `#[suprnova::model(table = "users")] struct User { ... }` | ausgeliefert | Die Struktur IST das SeaORM-`Model`. [Eloquent](eloquent.md) |
| Find / first / get | `User::find(id)`, `User::query().first()`, `User::all()`, `Builder::get` | ausgeliefert | Alles async |
| Create / update / delete | `User::create(attrs)`, `user.update(attrs)`, `user.delete()` | ausgeliefert | `attrs! { name: "...", email: "..." }`-Makro für partielle Attribute |
| Mass-Assignment-Guards | `#[model(fillable = [...])]` / `#[model(guarded = [...])]` + Scope `unguarded \|\| { ... }` | ausgeliefert | `prevent_silently_discarding_attributes()` für strikten Modus |
| Soft Deletes | `#[model(soft_deletes)]` injiziert `deleted_at` automatisch + `SoftDeletes`-Trait | ausgeliefert | `with_trashed()`, `only_trashed()`, `restore()`, `force_delete()` |
| Prunable / MassPrunable | `#[prunable] impl Prunable for User { ... }` + `model:prune`-Worker | ausgeliefert | Cascade-gepinnt an Relationen |
| Timestamps | Automatisches `created_at`/`updated_at`, wenn die Spalten vorhanden sind | ausgeliefert | Deaktivieren via `#[model(timestamps = false)]` |
| Primärschlüssel-Typen | i64 als Standard; UUID / ULID via `#[model(unique_id = "uuid")]` oder `unique_id = "ulid"` | ausgeliefert | Erzeugt die ID beim Insert automatisch |
| Lokale Scopes | `#[scopes(User)] impl User { fn active(b: &mut Builder<User>) { ... } }` | ausgeliefert | Methoden-Dispatch auf `Builder<M>` |
| Globale Scopes | `impl GlobalScope for ActiveOnly { ... }` + registrieren | ausgeliefert | Entfernbar via `Builder::without_global_scope` |
| Relationships (11 Arten) | `HasOne`, `HasMany`, `BelongsTo`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`, `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany` | ausgeliefert | Pro-Familie-Morph-Enum. [Relationships](eloquent-relationships.md) |
| Eager Loading | `User::query().with(&["posts", "posts.comments"]).get()` | ausgeliefert | `EagerLoadDispatch` ist sealed; nur makro-generierte Relationen können es implementieren |
| Lazy-Loading-Prävention | `prevent_silently_discarding_attributes(true)` | ausgeliefert | Gleiche Form wie Laravels `preventLazyLoading` |
| Aggregate auf Relationen | `with_count("posts")`, `with_sum("orders", "total")`, `with_avg`, `with_min`, `with_max` | ausgeliefert | Eine einzige Subquery pro Aggregat |
| `whereHas` / `whereDoesntHave` | `where_has("posts", \|q\| q.db_where("published", "=", true))` | ausgeliefert | Korrelierte EXISTS-Engine |
| `loadMissing` | `user.load_missing(&["posts"]).await?` | ausgeliefert | Wirkt collection-weit |
| Einen Datensatz klonen | `user.replicate()` / `user.replicate_into::<OtherType>()` | ausgeliefert | Löst das `Replicating`-Event aus |
| Parent-Timestamps berühren | `#[model(touches = ["post"])]` | ausgeliefert | `without_touching \|\| { ... }`, um es zu überspringen |
| Observers | `impl Observer<User>` + `#[suprnova::observer(User)]` | ausgeliefert | 16 Lifecycle-Events |
| 16 Lifecycle-Events | `Created`, `Creating`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Retrieved`, `Replicating`, `ForceDeleting`, `ForceDeleted`, `Pruning` | ausgeliefert | Pro Modell ein `events::*`-Submodul. `EventResult::cancel(_)` unterbricht per Short-Circuit mit einem 400 |
| Mutators / Accessors | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | ausgeliefert | [Mutators](eloquent-mutators.md) |
| Casts (22 eingebaute) | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | ausgeliefert | `Cast` implementieren für eigene |
| Collections | `Collection<M>` mit `pluck`, `filter`, `map`, `each`, `chunk`, `groupBy`, `keyBy`, `sort_by`, `where_`, `first`, `last`, `count`, `is_empty`, `to_array` und Laravel-Verwandten; `Deref<Target = Vec<M>>`, sodass alle `Vec`-Idiome weiter funktionieren | ausgeliefert | [Collections](eloquent-collections.md) |
| API Resources | `#[derive(Resource)]` + `IntoJsonResource` + `JsonApiResponse` + Fieldsets + Includes | ausgeliefert | Sowohl JSON:API-Form als auch Laravel-Resource-Form verfügbar. [JSON:API Resources](eloquent-resources.md) |
| Serialization | `#[model(hidden = [...], visible = [...], appends = [...])]` | ausgeliefert | Gleiche Kontrolle darüber, welche Attribute serialisiert werden. [Serialization](eloquent-serialization.md) |
| Factories | `#[derive(Factory)] struct UserFactory` + `UserFactory::new().count(5).create().await?` (oder `UserFactory::times(5).create_many().await?`) | ausgeliefert | `Sequence` für zyklische Werte. [Factories](eloquent-factories.md) |
| Lifecycle: Chunking / Lazy / Cursor | `Builder::chunk(n, \|page\| async { ... })`, `lazy()`, `cursor()` | ausgeliefert | Speicherbegrenzte Iteration über große Tabellen |
| Pessimistisches Sperren | `Builder::lock_for_update()`, `shared_lock()` | ausgeliefert | Innerhalb einer Transaction |
| `whereJsonContains`-Familie | Verfügbar über die Column-Expressions von SeaORM (treiberabhängig) | ausgeliefert | Die genaue Schreibweise unterscheidet sich je Backend; Helfer werden für die gängigen Fälle ausgeliefert |

## Paginierung

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| `LengthAwarePaginator` | `LengthAwarePaginator` (page + total + per_page + last_page) | ausgeliefert | `Builder::paginate(n).await?` |
| `Paginator` (einfach) | `Paginator` (page + per_page + has_more, keine Zählung) | ausgeliefert | `Builder::simple_paginate(n).await?` |
| `CursorPaginator` | `CursorPaginator` (opakes Cursor-Token + Richtung) | ausgeliefert | `Builder::cursor_paginate(n).await?`; deterministisch für Infinite Scroll |
| Inertia-Integration | `IntoInertiaScroll`-Trait + `ScrollMetadata` | ausgeliefert | Wird direkt in Inertias `WhenVisible` / `merge` verdrahtet |

## KI (Laravel liefert heute nativ; wir gatekeepen nicht)

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| AI SDK | Kein First-Party-KI-SDK | absichtlich nicht | Bringen Sie die Crate mit, die Sie bereits nutzen (`async-openai`, `anthropic-sdk`, `ollama-rs`, `tokenizers` usw.) und binden Sie sie unter `App` |
| MCP (Model Context Protocol) | Kein First-Party-MCP-Server-Adapter | absichtlich nicht | Die Rust-MCP-Crates (`mcp-rs`, `mcp-sdk-rust`) sitzen sauber unterhalb der bestehenden Routing-/Supervisor-Oberfläche |
| Boost (Laravel Coding Agent) | n/a | absichtlich nicht | Außerhalb des Framework-Scopes |

## Testen

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| `php artisan test` | `cargo test` | ausgeliefert | [Testen](testing.md) |
| Pest-/PHPUnit-Stil | `#[suprnova_test]` (async-bewusst) + Jest-artige `expect!()`-Assertions + BDD-Makros `describe!()` / `test!()` | ausgeliefert | Alle drei funktionieren austauschbar |
| Feature-Tests (HTTP) | `handle_request(router, registry, req)` in-Process treiben - kein offener Socket | ausgeliefert | [HTTP-Tests](http-tests.md) |
| Konsolen-Tests | `dispatch_argv(["console", "..."])` ausführen und assertieren | ausgeliefert | Gleiche Form wie HTTP-Tests für die Konsolen-Binary |
| Browser-Tests (Dusk) | n/a im Framework - Playwright / WebdriverIO / den `gstack`-Agent-Browser nutzen | absichtlich nicht | Sprachübergreifendes Tooling existiert bereits; wir erfinden es nicht neu |
| Datenbank-Tests | `TestDatabase::fresh::<Migrator>()` + Rollback pro Test | ausgeliefert | [Datenbank-Tests](database-testing.md) |
| Mocking & Fakes | Fakes pro Facade: `MailFake`, `NotifyFakeGuard`, `EventFakeGuard`, `Queue::fake`, `Bus::fake`, `Http::fake`, `Storage::fake` | ausgeliefert | Aufgezeichnete Aufrufe + Assertion-Helfer. [Mocking](mocking.md) |
| Zeitreise | `tokio::time::{pause, advance, resume}` aus der Stdlib-Runtime | ausgeliefert | Wir liefern keine eigene - Tokios API kann das bereits |
| Container-Isolation | `TestContainer::fake(\|tc\| tc.bind(...))` - thread-lokal | abweichend | Parallel-sicher per Konstruktion. [Service Container](container.md) |

## Payments (Laravels Cashier; unseres ist providergenerisch)

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Cashier (Stripe) | Adapter-Crate `suprnova-payments-stripe` hinter generischen Traits `Payment` / `Subscription` / `CustomerStore` / `WebhookHandler` | abweichend | Generische Oberfläche, konkreter Adapter. [Zahlungen](payments.md), [Zahlungen - Stripe Adapter](payments-stripe.md) |
| Cashier (Paddle) | Adapter `suprnova-payments-paddle` | abweichend | Merchant-of-Record-Flow + keine direkte `Payment`-Impl (Paddle besitzt das Gateway). [Zahlungen - Paddle Adapter](payments-paddle.md) |
| Eigener Provider | `PaymentProvider` + `SessionPayload` + `WebhookHandler` implementieren | ausgeliefert | [Zahlungen - Provider-Leitfaden](payments-provider-guide.md) |
| Inertia-Checkout-Komponenten | Dokumentierte Dispatch-Loops für Svelte / React / Vue gegen `SessionPayload.flow` | ausgeliefert | [Zahlungen - Frontend Integration](payments-frontend.md). Fertige Billing-Seiten sind eine geplante Ergänzung der Starter-Kits ([Starter-Kits](starter-kits.md)) |
| Subscription-Lifecycles | `Subscription::subscribe / update / cancel / get` (wo der Provider sie unterstützt) | ausgeliefert | `NotSupported` wird zurückgegeben, wo der Provider es nicht tut (z. B. Paddle `subscribe` und Ersetzen von Preis-Sets) |
| Webhook-Idempotenz | Mirror-Tabelle `payments_webhook_events` mit `UNIQUE(provider, provider_event_id)` | ausgeliefert | Stripe-artiger Replay-Schutz |
| Mirror-Tabellen | `payments_customers`, `payments_payment_methods`, `payments_subscriptions`, `payments_subscription_items`, `payments_transactions`, `payments_webhook_events` | ausgeliefert | `provider_metadata`-JSONB-Spalte auf jeder für adapterspezifische Felder |

## Frontend (Laravel hat Blade + Starter-Kits; wir haben Inertia)

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Blade | n/a - Inertia ist die View-Schicht | abweichend | [Frontend](frontend.md) |
| Inertia.js | First-Class: v3 über Svelte 5 / React 19 / Vue 3.5 | ausgeliefert | [Inertia Responses](frontend-inertia-responses.md), [Seiten](frontend-pages.md) |
| Partial Reloads | `#[derive(Data)]` + `req.includes("subset")` + Inertias Partial-Reload-Protokoll | ausgeliefert | Typsichere Include-Sets |
| Deferred Props | `Prop::deferred(...)` + `DeferConfig` | ausgeliefert | Inertia-v3-Deferred-Props-Protokoll |
| Merge Props | `MergeConfig` + `MergeStrategy::{Append, Prepend, Replace}` | ausgeliefert | Inertia-v3-Merge-Protokoll |
| Encrypt History | `EncryptHistoryMiddleware` | ausgeliefert | History im Client at rest verschlüsselt |
| Scroll-Position | `ScrollConfig` + `ScrollMetadata` | ausgeliefert | Automatisch wiederhergestellt bei Navigation |
| TypeScript Types | `suprnova generate-types` liest `#[derive(InertiaProps)]` und erzeugt `.d.ts` | ausgeliefert | [TypeScript Types](frontend-typescript-types.md) |
| Vite-Manifest-Lesen | Automatisch verdrahtet via `Inertia::root_view` | ausgeliefert | HMR in Dev, gehashte Assets in Prod |

## CLI

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| `php artisan` | Pro-App-`console`-Binary, gebaut aus `#[command]`-Makros | ausgeliefert | [Konsole](console.md), [CLI-Übersicht](cli.md) |
| `make:controller` / `make:model` usw. | `suprnova make:controller / make:middleware / make:action / make:error / make:inertia / make:migration / make:task` | ausgeliefert | [Generatoren](cli-generators.md) |
| `serve` | `suprnova serve` (Backend + Vite-Dev-Server zusammen) | ausgeliefert | [Serve](cli-serve.md) |
| `migrate`-Familie | `suprnova migrate / migrate:rollback / migrate:status / migrate:fresh` | ausgeliefert | [Migrationen-CLI](cli-migrations.md) |
| `db:seed` | `cargo run --bin console db:seed` (via Pro-App-Konsole) | ausgeliefert | Seeder werden via `Seeder`-Trait registriert |
| `schedule:run` / `schedule:work` / `schedule:list` | Gleiche Namen via Pro-App-Konsolen-Binary | ausgeliefert | [Scheduling-CLI](cli-scheduling.md) |
| `queue:work` | Gleicher Name via Pro-App-Konsolen-Binary | ausgeliefert | Graceful Shutdown bei SIGTERM/SIGINT |
| `tinker` | Kein REPL | absichtlich nicht | Siehe die Zeile in „Tiefere Einblicke“ |

## Deployment

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| `php artisan optimize` | `cargo build --release` | abweichend | Eine Binary, kein Opcache-Schritt |
| `php artisan config:cache` | Typisierte Config ist bereits zur Compile-Zeit geprüft | abweichend | Kein Runtime-Cache zu invalidieren |
| `php artisan route:cache` | Routen werden zur Compile-Zeit makro-expandiert | abweichend | Der Router wird beim Boot aus bereits typisierten Routen gebaut |
| Envoy (SSH-Deploys) | Nutzen Sie einen beliebigen Orchestrator - Docker, systemd, Kubernetes, fly.io, Railway | absichtlich nicht | Die Binary ist das Deploy-Artefakt |
| Forge / Vapor | Nicht unseres zum Ausliefern - aber die Rezepte für Railway, DO und Hetzner decken dieselbe Aufgabe ab | abweichend | [Bereitstellung](deployment.md), [Railway](deployment-railway.md), [Digital Ocean](deployment-digital-ocean.md), [Hetzner](deployment-hetzner.md) |
| Horizon (Queue-Dashboard) | Noch kein Dashboard | noch nicht | Inspektion fehlgeschlagener Jobs via `cargo run --bin console queue:failed` bis dahin |

## Packages (Laravels offizielle Packages - unsere liefern entweder im Core, als Adapter, oder sind bewusste Lücken)

| Laravel-Package | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Cashier (Stripe) | `suprnova-payments-stripe` | ausgeliefert | Generisch + Adapter. [Zahlungen](payments.md) |
| Cashier (Paddle) | `suprnova-payments-paddle` | ausgeliefert | MoR-Flow. [Zahlungen](payments.md) |
| Dusk | n/a | absichtlich nicht | Sprachübergreifendes Browser-Tooling existiert bereits (Playwright usw.) |
| Envoy | n/a | absichtlich nicht | Container/systemd/Orchestratoren erledigen die Aufgabe |
| Fortify | Ersetzt durch `auth_flows` | ausgeliefert | Gleiche Aufgabe, integriert. [Auth-Flows](auth-flows.md) |
| Folio | n/a - seitenbasiertes Routing ist kein idiomatisches Rust | absichtlich nicht | Nutzen Sie `routes!` für explizites Routing |
| Homestead | n/a - Docker / DevContainers nutzen | absichtlich nicht | [Docker-Rezept](cli-docker.md) |
| Horizon | n/a bisher | noch nicht | Fehlgeschlagene Jobs erscheinen via die Pro-App-Konsole |
| Mix | Ersetzt durch Vite | abweichend | Vite ist in jedem Scaffold enthalten |
| Octane | n/a - wir sind bereits langlebiges Tokio | absichtlich nicht | Eine Binary, immer warm, kein FPM zum Austauschen |
| Passport | n/a bisher | noch nicht | Betreiben Sie einen dedizierten IdP hinter Suprnova, bis es ausgeliefert wird |
| Pennant (Feature Flags) | Neu implementiert als `features::*` | ausgeliefert | [Feature Flags](feature-flags.md) |
| Pint (PHP-Codestil) | `cargo fmt` + `cargo clippy` | abweichend | Standard-Rust-Toolchain |
| Precognition | Inertia-präkognitive Requests via Partial Reloads + dieselben Typen `#[derive(Data, Validate, FormRequest)]` | ausgeliefert | Die zwei Hälften von Precog (frühe Validierung + leichtgewichtiger Reload) fallen beide aus Inertia v3 + Form Requests heraus |
| Prompts (CLI-UI) | Nutzen Sie bei Bedarf die Crate `dialoguer` / `inquire` | absichtlich nicht | Das Rust-Ökosystem deckt das bereits ab |
| Pulse | n/a bisher | noch nicht | OTel heute, Dashboard später |
| Reverb (WebSocket-Server) | In Suprnova eingebaut (`ws!()` + `BroadcastHub`) | abweichend | Kein separater Server nötig - es ist derselbe Prozess |
| Sail (Docker Dev) | `suprnova-cli` liefert Docker-Rezepte inline aus | ausgeliefert | [CLI Docker](cli-docker.md) |
| Sanctum | `TokenGuard` + Bearer-Middleware | abweichend | Token-Modell wird ausgeliefert; keine separate Package-Oberfläche |
| Scout (Volltextsuche) | n/a bisher | noch nicht | Vektorsuche wird ausgeliefert ([Vector](vector.md)); Keyword-Scout-Äquivalent später |
| Socialite | Über den vendorten torii-Fork | ausgeliefert | [Authentifizierung](authentication.md) |
| Telescope | n/a bisher | noch nicht | Tracing + OTel decken die Diagnoselücke ab, bis ein Dashboard ausgeliefert wird |
| Valet | n/a - Rust-Apps laufen direkt | absichtlich nicht | `suprnova serve` ist der Dev-Runner |

## Makros (Rust-spezifische Oberfläche; nächstgelegene Laravel-Analogien zum Kontext)

Suprnova liefert einen breiten Satz an Proc-Makros aus, die kein
Laravel-Analogon haben, weil Laravel keine Makros hat - es hat
Runtime-Reflection. Sie sind hier aufgeführt, damit Sie sie nicht
übersehen.

| Makro | Nächstgelegene Laravel-Idee | Was es tut |
|---|---|---|
| `#[suprnova::model]` | `extends Model` | Generiert eine SeaORM-Entity + implementiert das `Model`-Trait |
| `#[suprnova::observer(M)]` | `User::observe(UserObserver::class)` | Registriert eine `Observer<M>`-Impl via `inventory` |
| `#[scopes(M)]` | Lokale Scopes auf einem Modell | Fügt Methoden zu `Builder<M>` hinzu |
| `#[accessor]` / `#[mutator]` | Eloquent-Accessoren / -Mutatoren | Get-/Set-Hooks auf Feldebene |
| `#[handler]` | Controller-`__invoke` | Extrahiert automatisch typisierte Parameter aus `Request` |
| `#[command]` / `#[derive(Command)]` | Artisan-Command-Klasse | Registriert ein Konsolen-Subcommand |
| `#[policy]` | Policy-Klasse | Registriert eine `Policy`-Impl via `inventory` |
| `#[service(T)]` | Service-Provider-`register` | Bindet `T` in den Container |
| `#[injectable]` | Konstruktor-Injection | Generiert einen von `App::make` gestützten Konstruktor |
| `#[derive(InertiaProps)]` | Inertia-Props | TypeScript-Codegen + Inertia-Serialization |
| `#[derive(Data)]` | Request-DTO | Aus `Request` extrahierbar, mit Include-Set-Unterstützung |
| `#[derive(FormRequest)]` | `FormRequest`-Klasse | Validierung + Auth-Gate + Transformation |
| `#[derive(Factory)]` | Model-Factory | Faker-gestützte Testdaten-Generierung |
| `#[derive(Resource)]` | API Resource | JSON:API + Laravel-Form-Serialization |
| `#[workflow]` / `#[workflow_step]` | n/a in Laravel | Lang laufende, zustandsbehaftete Arbeit |
| `routes!` + `get!` / `post!` / `ws!` usw. | `Route::get` / `Route::post` | Compile-Zeit-Routenregistrierung |
| `casts!` | `protected $casts = [...]` | Pro-Modell-Cast-Deklaration |
| `attrs!` | Mass-Assignment-Array | Partial-Attribute-Builder |
| `json_response!` / `text_response!` | `response()->json(...)` | Schnelles `Ok(HttpResponse::...)` |

Siehe [Makros](macros.md) für die vollständige Referenz.

## Helper-Funktionen (Laravels globale Helfer; unsere sind typisiert)

Laravel liefert Hunderte kleiner Globals aus (`str_replace_first`,
`array_flatten`, `now()`, `tap()`, `optional()` …). Die meisten davon
haben ein direktes Rust-Äquivalent in `std` oder einer kleinen
Standard-Crate, daher führt Suprnova sie nicht als einzelnen
Namensraum wieder ein. Diejenigen, die tatsächlich nützlich sind, um
sie als Alias auszuliefern, liegen unter ihrem Heimat-Modul.

| Laravel-Helfer | Suprnova-/Rust-Äquivalent | Wo |
|---|---|---|
| `auth()` | `Auth::user().await?` | [Authentifizierung](authentication.md) |
| `cache()` | `Cache::get/put/...` | [Cache](cache.md) |
| `config('app.name')` | `Config::get::<AppConfig>()?.name` | [Konfiguration](configuration.md) |
| `csrf_token()` | `csrf_token()` (gleicher Name) | [CSRF](csrf.md) |
| `dd()` | `Builder::dd()` (Eloquent-Query Dump-and-Die) / `dbg!()` aus der Stdlib | `Builder::dump()` / `Builder::dd()` existieren für Query-Inspektion; nutzen Sie `dbg!()` für allgemeine Werte |
| `env('APP_KEY')` | `env("APP_KEY")` / `env_required("APP_KEY")` / `env_optional("APP_KEY")` | [Konfiguration](configuration.md), [Umgebungsvariablen](env-vars.md) |
| `now()` | `chrono::Utc::now()` (re-exportiert als `suprnova::chrono`) | - |
| `optional($x)->y` | `x.as_ref().map(\|x\| x.y)` | Rust löst das direkt mit `Option<T>` |
| `redirect('/')` | `redirect("/")` (gleicher Name) | [Routing](routing.md) |
| `request()` | `Request` wird in Ihren Handler übergeben | [Anfragen](requests.md) |
| `response()` | `HttpResponse::json/text/redirect/...` | [Antworten](responses.md) |
| `route('posts.show', ['post' => 1])` | `url("posts.show", &[("post", "1")])` | [URL-Generierung](urls.md) |
| `session('key')` | `session().get("key")` | [Sitzungen](session.md) |
| `str()` / `Str::camel($x)` | `heck`-Crate-Methoden (`ToUpperCamelCase` usw.) | - |
| `tap($x, fn) → $x` | `tap` aus der `tap`-Crate, oder `dbg!` für schnelle Inspektion | Nutzen Sie die `tap`-Crate idiomatisch |
| `today()` | `chrono::Utc::now().date_naive()` | - |
| `value($x)` | Rufen Sie einfach die Closure auf: `x()` | n/a - Rust-Closures brauchen keinen Helfer |
| `view('home', $data)` | Inertia-Response: `Inertia::render("Home", data)` | [Inertia Responses](frontend-inertia-responses.md) |

## Was uns wirklich noch fehlt

Eine konsolidierte Liste jedes obigen **noch nicht**, damit Sie die
Form der Lücke an einem Ort sehen können:

| Bereich | Was fehlt | Workaround bis zur Auslieferung |
|---|---|---|
| Search (Scout - Keyword) | Algolia-/Meilisearch-/Elastic-Adapter | Bauen Sie sich mit `meilisearch-sdk` / `elasticsearch` Ihren eigenen, bis er ausgeliefert wird; [Vector](vector.md) übernimmt semantische Suche schon heute |
| Passport (OAuth-Server) | First-Party-OAuth-Identity-Provider | Betreiben Sie Hydra / Keycloak hinter Suprnova |
| Telescope (Debug-Dashboard) | Web-UI für Requests / Queries / Events / Cache-Treffer | OTel- + Tracing-Ausgabe nutzen ([Beobachtbarkeit](observability.md)) |
| Pulse (Perf-Dashboard) | Web-UI für langsame Queries / Fehler / Hot Routes | Ebenso: OTel-Oberfläche heute, Dashboard später |
| Horizon (Queue-Dashboard) | Web-UI für Queue-Tiefe / fehlgeschlagene Jobs / Durchsatz | `cargo run --bin console queue:failed` und OTel-Metriken |

## Was wir nicht ausliefern werden (und warum)

| Laravel-Feature | Warum Suprnova es nicht hat |
|---|---|
| Tinker (REPL) | Rust hat keine produktive REPL-Geschichte für kompilierte Binaries. Ein kurzer `#[suprnova_test]` oder ein einmaliges `cargo run --bin <thing>`-Skript erledigt die Aufgabe |
| Blade-Templates | Inertia ist die View-Schicht; wir liefern keine parallele serverseitig gerenderte Template-Engine |
| `helpers.md`-Kitchen-Sink | Rust liefert `std` + kleine fokussierte Crates (`heck`, `chrono`, `regex`); wir führen keinen einzelnen globalen Namensraum wieder ein |
| Mix | Vite deckt es ab und ist in jedem Scaffold enthalten |
| Octane | Suprnova ist bereits langlebiges Tokio; es gibt keinen FPM-Modus, aus dem man herausoptimieren müsste |
| Dusk (Browser-Tests) | Sprachübergreifendes Tooling (Playwright, WebdriverIO, der `gstack`-Agent-Browser) löst das bereits |
| Sail (Docker Dev) | Docker-Rezepte sind inline enthalten ([CLI Docker](cli-docker.md)); kein separates Package nötig |
| Valet | `suprnova serve` ist der Dev-Server |
| Envoy (SSH-Deploys) | Container/systemd/Orchestratoren erledigen die Aufgabe; wir brauchen keine bespoke SSH-DSL |
| Concurrency-Facade (`Concurrency::run`) | Tokio (`tokio::join!` / `tokio::spawn` / `tokio::select!`) ist die Antwort; keine Facade nötig |
| Processes-Facade | `tokio::process::Command` hat bereits die richtige Form |
| First-Party-KI-SDK / MCP / Boost | Wählen Sie die Rust-Crates, die Sie bereits nutzen; wir gatekeepen nicht |
| Dedizierte Redis-Facade | Cache/Queue/Rate-Limit decken 95 % der typischen Nutzung ab; greifen Sie zur `redis`-Crate, wenn Sie Ad-hoc-Befehle brauchen |
| Strings-Facade | `heck`, `regex`, `std::str` decken es ab; kein globales `Str::camel($x)` |
| Prompts (CLI-UI-Library) | `dialoguer` / `inquire` existieren bereits; wir erfinden nicht neu |
| Laravel-artige PHP-/JSON-Übersetzungsdateien | Lokalisierung wird ausgeliefert, aber das Katalogformat ist Fluent `.ftl` - ein Format, das Server und Browser beide parsen. `trans_choice` hat ebenfalls kein Äquivalent: Fluent wählt CLDR-Pluralkategorien innerhalb der Nachricht aus. [Lokalisierung](localization.md) |

## Wie diese Liste ehrlich bleibt

Jede Zeile in der Spalte **ausgeliefert** ist verifizierbar durch:

1. Grep von `framework/src/lib.rs` nach dem benannten Export
2. Ausführen der Framework-Testsuite (`cargo test --workspace`)
3. Lesen des verlinkten Kapitels

Jede Zeile in der Spalte **noch nicht** ist beabsichtigte Arbeit, keine
Verweigerung. Jede Zeile in der Spalte **absichtlich nicht** hat einen
Ein-Satz-Grund in der Spalte Hinweise; diese Gründe sind die
Design-Prinzipien aus [Einführung](introduction.md), angewandt auf ein
konkretes Feature.

Wenn Sie ein Laravel-Feature finden, zu dem Sie greifen und das nicht
auf dieser Karte steht, eröffnen Sie ein Issue - entweder hat es eine
Suprnova-Antwort, der eine Zeile fehlt, oder es ist eine echte Lücke,
und wir wollen es wissen.

## Nächste Schritte

- [Von Laravel kommend](from-laravel.md) - dieselbe Karte, als
  Seite-an-Seite-Erzählung
- [Einführung](introduction.md) - die Design-Prinzipien, denen diese
  Parity-Arbeit folgt
- [`documentation.md`](documentation.md) - das Master-Inhaltsverzeichnis
  über jedes Kapitel hinweg

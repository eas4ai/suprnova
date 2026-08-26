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
| Request Lifecycle | Chain aus `Application` → `Server` → `handle_request` | ausgeliefert | [Request-Lifecycle](lifecycle.md) |
| Service Container | `Container` + `App`-Facade, dreischichtig (Task / Thread / Global) | abweichend | Task-lokal für den Pro-Anfrage-Fall, thread-lokal für Tests - [Service Container](container.md) |
| Kontextuelle Bindung (`when()->needs()->give()`) | Keine kontextuellen Bindings - eine Bindung pro Trait pro Container-Schicht | absichtlich nicht | Der Container ist `TypeId`-geschlüsselt und hat keine Runtime-Reflection, um eine Bindung darauf zu schlüsseln, „wer fragt“. Komponieren Sie explizit: Übergeben Sie die Abhängigkeit, oder binden Sie pro Konsument einen eigenen Newtype. [Service Container](container.md) |
| Service Provider | `bootstrap()`-Funktion + `#[service]`, `#[policy]`, `#[command]`, Observer-Makros | abweichend | Keine Registrierungsklasse - Bootstrap ist eine einzige Funktion; Makros nutzen `inventory` für die Registrierung zur Compile-Zeit. [Application Bootstrap](bootstrap.md) |
| Facades | Statische `App::get`, `Cache::*`, `Mail::*`, `Auth::*`, `Storage::*`, `Queue::*`, `Bus::*`, `Event::*`, `Notification::*`, `Gate::*`, `Schedule::*`, `DB::*`, `Vector::*` | ausgeliefert | Gleiche Aufrufform; die Facades sind echte Typen, keine Aliase |
| Contracts | Traits - `Mailer`, `KeyValueStore`, `Hasher`, `Channel`, `VectorDriver`, `Evaluator`, `PaymentProvider` usw. | ausgeliefert | Alle öffentlichen Nahtstellen liegen auf Traits; binden Sie per Trait und tauschen Sie Implementierungen frei aus |

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
| Routen-Definitionen | `routes!`-Makro + `get!` / `post!` / `put!` / `patch!` / `delete!` / `any!` / `head!` / `options!` / `fallback!` / `ws!` | ausgeliefert | [Routing](routing.md) |
| Routen-Parameter | `{id}`-Pfad-Parameter + `req.param("id")` | ausgeliefert | Optionale Parameter über `{id?}`; Einschränkungen über `where!()` |
| Routen-Namen | `.name("posts.show")` an der Route + `url("posts.show", &[("id", "42")])` | ausgeliefert | [URL-Generierung](urls.md) |
| Routen-Gruppen | `group!`-Makro mit `.prefix()` / `.middleware()` / `.name()` / `.controller()` | ausgeliefert | Gruppen-Middleware wird bei der Registrierung auf jede Route abgeflacht |
| Ressourcen-Routen | `resource!("posts", PostController)` registriert die 7 Standard-Routen | ausgeliefert | `apiResource!`, `only(...)` und `except(...)` werden alle unterstützt |
| Signierte URLs | `sign_url(...)`, `sign_route(...)`, `verify_signature(...)` | ausgeliefert | HMAC-SHA256 mit `APP_KEY` |
| Route-Model-Binding | `#[handler]` extrahiert `Post` aus `{post}` über eine `RouteBinding`-Implementierung | ausgeliefert | Das `AutoRouteBinding`-Derive implementiert das automatisch für `#[suprnova::model]`-Typen |
| Ratenbegrenzung | `throttle:60,1`-Middleware + `RateLimiter::for_signature` | ausgeliefert | [Ratenbegrenzung](rate-limiting.md) |
| Middleware | `impl Middleware`-Trait; global oder pro Route registrieren | ausgeliefert | [Middleware](middleware.md) |
| Middleware-Gruppen + Aliase | `register_middleware_group`, `register_middleware_alias` | ausgeliefert | Nachschlagen über den String-Namen in den Routen |
| CSRF-Schutz | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | ausgeliefert | Die Token-Validierung pro Sitzung ist der Standard. Die optionalen Policies `SameOriginOnly`, `AllowSameSite` und `OriginOnly` konsultieren `Sec-Fetch-Site`; die Origin-Durchsetzung ist standardmäßig nicht aktiviert. [CSRF](csrf.md) |
| Controller | `#[handler] pub async fn show(req: Request) -> Response` | ausgeliefert | Controller sind Module aus freien Funktionen, keine Klassen. [Controller](controllers.md) |
| Controller mit einer einzigen Aktion | Ein Handler ist bereits eine einzelne Funktion; gruppieren Sie sie in Modulen | ausgeliefert | Die Rust-Konvention - keine `__invoke`-Zeremonie |
| Anfragen | `Request`-Struktur mit `.input()`, `.param()`, `.query()`, `.header()`, `.cookie()`, `.json()`, `.file()` usw. | ausgeliefert | [Anfragen](requests.md) |
| Form-Requests | `#[derive(Data, Validate, FormRequest)]` | ausgeliefert | Die Validierung läuft beim Extrahieren mit |
| Datei-Uploads | `req.file("avatar")?` liefert ein `UploadedFile`; Streaming-Multipart mit Obergrenzen für Größe und Teile | ausgeliefert | Automatisches Auslagern in eine temporäre Datei oberhalb der Schwelle |
| Antworten | `HttpResponse`-Builder + `json_response!()` / `text_response!()` / `Redirect::to` / Inertia-Antworten | ausgeliefert | [Antworten](responses.md) |
| Gestreamte Antworten (`eventStream`, `stream`, `streamJson`) | `HttpResponse::sse(...)` / `event_stream(...)` / `stream_bytes(...)` / `stream_json(...)` | ausgeliefert | Dieselben Wire-Formen, die die Hooks von `@laravel/stream-{react,vue,svelte}` erwarten. [SSE](sse.md) |
| `withoutCookie` / `withoutCookies` | `.without_cookie(name)` / `.without_cookies([...])` an `HttpResponse`, `Response`, `Redirect`, `RedirectRouteBuilder` | ausgeliefert | `Cookie::forget_with(name, path, domain)` für ein Cookie, das nicht unter `/` gesetzt wurde |
| Views (Blade) | Serverseitig gerenderte Inertia-Seiten (Svelte/React/Vue) - kein Blade-Äquivalent | abweichend | Inertia ist die View-Schicht. Nehmen Sie [Seiten-Komponenten](frontend-pages.md) statt Blade |
| Asset-Bundling (Vite) | Vite 8 wird in jedem Scaffold ausgeliefert; `suprnova serve` startet Vite und Backend gemeinsam | ausgeliefert | Manifest-Lesen + HMR automatisch verdrahtet |
| Statische Assets (`public/`, in Laravel vom Webserver ausgeliefert) | `StaticFiles::public()` als prozessinterner Fallback-Handler, der `public/` an der Web-Wurzel ausliefert | ausgeliefert | `StaticFiles::from_dir(...)` + `cache_control(...)`; kein separater Webserver nötig |
| URL-Generierung | `url("posts.show", &[…])`, `route("posts.show", …)`, `redirect(...)`, `redirect_to(...)` | ausgeliefert | [URL-Generierung](urls.md) |
| Sitzung | `session()`, `session_mut()`, Flash-Bag über `req.flash()` | ausgeliefert | Standardmäßig datenbankgestützt über `DatabaseSessionDriver`; das verschlüsselte Browser-Cookie trägt die Sitzungskennung und Metadaten zur Aktivitätsberührung, nicht den Daten-Bag der Sitzung. [Sitzungen](session.md) |
| Cookie-Queue (`Cookie::queue`) | `Cookie::queue`/`queued`/`unqueue`/`expire` - ein Task-lokales Jar, das die `SessionMiddleware` auf die Antwort leert | ausgeliefert | Erfordert die `SessionMiddleware` in der Chain; eingereiht wird nach Name, nicht nach Name+Pfad wie in Laravels `CookieJar` |
| Validierung | `#[derive(Validate)]` + 28 eingebaute Regeln + die Traits `Rule`/`ValueRule`/`AsyncRule` | ausgeliefert | `Url` nutzt Laravels Schema-Allowlist, und `Url::protocols([...])` spiegelt `url:http,https`. Async-Regeln (z. B. `Unique`) greifen auf die DB zu. `ArrayKeys`/`Distinct` sind `ValueRule`s über `serde_json::Value`, passend zu Laravels `array:keys` und `distinct`. [Validierung](validation.md) |
| `Password`-Regel (`Password::defaults()`, `uncompromised()`) | `Password::min(n)` + Stärke-Builder (`.letters()`, `.mixed_case()`, `.numbers()`, `.symbols()`) + `.uncompromised()` | ausgeliefert | k-Anonymitäts-Prüfung bei Have I Been Pwned; lässt das Passwort bei einem Netzwerkfehler durchgehen, passend zu Laravels `NotPwnedVerifier`. [Validierung](validation.md#password-strength) |
| Fehlerbehandlung | `FrameworkError`, `AppError`, `HttpError`-Trait, Panic-Grenze in `execute_chain_safely` | ausgeliefert | [Fehlerbehandlung](errors.md), [Fehlermodell](error-model.md) |
| Protokollierung | `tracing`-Subscriber mit strukturierten Feldern, `LogFormat` (json / pretty / compact) | abweichend | Eine Logzeile ist ein JSON-Dokument; `request_id` ist immer vorhanden. [Protokollierung](logging.md) |
| Log-Kanäle / Datei-Treiber (`single`, `daily`, `monthly`, `stack`) | `tracing` schreibt strukturierte Zeilen nach stdout; die Plattform rotiert und versendet sie | absichtlich nicht | Container, systemd und jeder Log-Shipper erledigen Rotation und Aufbewahrung bereits. Das im Prozess nachzubauen dupliziert die Plattform und versteckt die Logs vor ihr. [Protokollierung](logging.md) |
| Abort-Helfer | `abort_if(cond, status, msg)`, `abort_unless(...)`, `abort_with(status, msg)` | ausgeliefert | Dieselbe Form wie Laravels `abort_if`-Familie |

## Tiefere Einblicke

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Artisan Console | Eine `console`-Binary pro App, gebaut aus `#[command]` + `#[derive(Command)]` | ausgeliefert | [Konsole](console.md). `cargo run --bin console <subcommand>` |
| Tinker (REPL) | Kein REPL | absichtlich nicht | Schreiben Sie ein einmaliges `cargo run --bin xxx`-Skript oder einen `#[suprnova_test]` |
| Broadcasting | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | ausgeliefert | sea-streamer-Fanout für mehrere Knoten. [Broadcasting](broadcasting.md) |
| Cache | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`, `RedisCache` | ausgeliefert | Atomare Operationen + getaggter Cache + Cache-Sperren (`LockGuard`). [Cache](cache.md) |
| Collections | `eloquent::Collection<M>` mit Methoden in Laravel-Form | ausgeliefert | `Deref<Target = Vec<M>>`, sodass bestehende Vec-Idiome weiterhin funktionieren. [Collections](eloquent-collections.md) |
| Nebenläufigkeit | Überall Tokio - `tokio::spawn`, `tokio::join!`, `tokio::select!` | ausgeliefert | Das ganze Framework ist async. Laravels Facade `Concurrency::run([...])` wird nicht ausgeliefert; Tokio ist die Antwort |
| Kontext | `Context::put` / `Context::get` / `ContextStore` + automatische Injektion in Queue / Mail / Ereignisse | ausgeliefert | [Kontext](context.md) |
| Contracts | Alle öffentlichen Nahtstellen sind Traits | ausgeliefert | Siehe die Zeile „Architektur / Contracts“ weiter oben |
| Ereignisse | `EventFacade::dispatch(e).await?`, `#[derive(Event)]`, `EventDispatcher`, eingereihte Listener, Subscriber | ausgeliefert | [Ereignisse](events.md) |
| Dateispeicher | `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` über OpenDAL | ausgeliefert | Dieselbe `put/get/delete/copy/move/exists/url`-Oberfläche. Schutz vor Path-Traversal eingebaut. [Dateisystem](filesystem.md) |
| Helfer | Die Entsprechungen liegen in ihren Heimatmodulen (kein Sammelsurium `helpers.md`) | abweichend | Zum Beispiel leben URL-Helfer in [urls.md](urls.md), String-Helfer in `std`/`heck` und Array-Helfer in `std::collections` - Rust macht das mit Crates statt mit einem globalen Namensraum |
| HTTP-Client | `Http::get/post/...`-Builder + `Http::fake(...)` für Tests | ausgeliefert | Zeichnet Anfragen automatisch auf; `assert_sent` / `assert_not_sent`; `.retry_when(predicate)` verengt die eingebaute Wiederholungs-Policy mit einem `RetryContext`. [HTTP-Client](http-client.md) |
| Image (`Illuminate\Image`) | `Image::from_bytes/from_path/from_disk/from_upload/from_stream` + dieselbe Oberfläche aus Operationen und Abschlussmethoden | ausgeliefert | Lebt in `suprnova::media`. Zwei Treiber wie Laravels `gd`/`imagick`: `IMAGE_DRIVER=oxideav` (Standard, pures Rust) oder `magick`. Liest und schreibt PNG, JPEG, WebP, GIF, BMP; die AVIF-Ausgabe ist zurückgestellt, bis der hauseigene AV1-Encoder veröffentlicht ist. Header-geprüfte Dekodier-Limits. [Bilder](images.md) |
| HEIC-Dekodierung im Standardtreiber | `IMAGE_DRIVER=magick` auf einem Host mit dem libheif-Delegate | absichtlich nicht | HEVC ist patentbelastet, und der einzige glaubwürdige Pure-Rust-Decoder ist dual AGPL/kommerziell lizenziert, deshalb wird kein eingebauter Decoder ausgeliefert. Dieselbe Form wie bei Laravel, wo GD HEIC überhaupt nicht lesen kann und Imagick das Delegate sowohl in die Binary als auch in die PHP-Erweiterung einkompiliert braucht. [Bilder](images.md#why-suprnova-diverges) |
| Lokalisierung | `Lang::get` / `get_with` / `try_get` / `has` + das Makro `__!("key", name: value)` über Fluent-`.ftl`-Kataloge in `lang/<locale>/`, Erkennung durch die `LocaleMiddleware`, übersetzte Validierungsmeldungen, ICU4X-Formatierung | ausgeliefert | Derselbe Katalog wird dem Browser unter `/_suprnova/lang/<locale>.ftl` ausgeliefert und von `generate-types` typisiert. [Lokalisierung](localization.md) |
| Mail | `Mail::to(...).send(MyMail { ... }).await?` + die Treiber `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory/file` | ausgeliefert | `Mailable`-Trait + Tera-gerenderte HTML-/Text-Rümpfe; SES-Sendungen tragen `TenantName` / `ConfigurationSetName` / `ListManagementOptions`; der eingereihte Dispatch wird über `.on_queue(...)` / `.on_connection(...)` geroutet und sticht `Queue::route` aus. [Mail](mail.md) |
| Benachrichtigungen | `Notify::send(&user, notif).await?` + die Kanäle `mail/database/broadcast/webpush` | ausgeliefert | `Notifiable`-Trait + `Notification` pro Kanal; der eingereihte Dispatch (`Notify::queue`) trägt `queue`/`timeout`/`fail_on_timeout`/`max_tries`/`backoff` pro Benachrichtigung über dasselbe `EnvelopeOverrides`-Primitiv, das auch Mail nutzt, auf den Job jedes Kanals. [Benachrichtigungen](notifications.md), [Web Push](web-push.md) |
| Paketentwicklung | Adapter-Crates im Workspace (z. B. `suprnova-payments-stripe`) | ausgeliefert | Dieselbe Form wie Laravel-Pakete: vom Framework abhängen, in den Container binden, bei Bedarf Makros bereitstellen |
| Prozesse (Shell-Befehle ausführen) | `tokio::process::Command` aus der Standardbibliothek | absichtlich nicht | Keine Facade - Tokios API hat bereits die richtige Form |
| Warteschlangen | `Queue::push(job).await?` + die Treiber `sync/memory/database/redis/null`, Batches, Chains, `JobMiddleware`, `FailedJobStore` | ausgeliefert | [Warteschlange](queues.md) |
| Vom Job deklarierte Verzögerung | `fn delay() -> Option<Duration>` auf `Job`, beachtet von `Queue::push` und `Queue::bulk` | ausgeliefert | Ein expliziter Aufruf von `Queue::push_later` / `Queue::later(delay, job)` gewinnt immer gegen den Standard des Jobs. [Warteschlange](queues.md) |
| Ereignis für übersprungene eindeutige Jobs | `queue::events::UniqueJobSkipped { job_name, unique_id, connection }` | ausgeliefert | Wird auf der Push-Seite gefeuert, wenn `push_unique` dedupliziert; der Aufruf gibt weiterhin `Ok(false)` zurück |
| Queue pausieren (`queue:pause` / `queue:resume`) | `Queue::pause`/`resume`/`pause_all`/`resume_all`/`is_paused`/`paused_queues`, Cache-gestützt, mit den Ereignissen `QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` | ausgeliefert | Eine Pause pro Queue greift nur bei einem Worker, der mit einer expliziten `--queue=...`-Liste gestartet wurde; `resume_all` hebt eine Pause pro Queue nicht auf. [Warteschlange](queues.md) |
| Dispatch nach dem Commit (`afterCommit()`) | `fn after_commit() -> bool` auf `Job`, `EnvelopeOverrides::after_commit` pro Push, `Queue::push_after_commit` | ausgeliefert | Der ganze Push wartet auf den Commit, Ereignisse eingeschlossen, und ein Rollback verwirft ihn; ein verzögertes `push_unique` nimmt seine Sperre trotzdem sofort, damit die Deduplizierung innerhalb der Transaktion funktioniert. Ein manuelles `DB::begin_transaction` verzögert nie. [Warteschlange](queues.md) |
| Failover-Queue-Connection | `FailoverQueueDriver` über eine geordnete Connection-Liste, über `QUEUE_DRIVER=failover` + `QUEUE_FAILOVER_CONNECTIONS` | ausgeliefert | Schreibzugriffe fallen durch die Liste; `pop`, die Zähler und die Auflistungen bleiben auf der ersten Connection, jeder Fallback braucht also seinen eigenen Worker. `QueueFailedOver` ist flankengesteuert, und `bulk_push` fällt pro Envelope durch, sodass jedes seine eigene Verzögerung behält. [Warteschlange](queues.md) |
| `ShouldBeUniqueUntilProcessing` | `fn unique_until_processing() -> bool` auf `Job`, freigegeben nach dem Middleware-Durchlauf und vor dem Handler | ausgeliefert | Eigentümerbezogene Freigabe, sodass ein erneut zugestellter Versuch nie die Sperre eines neueren Dispatch freigibt. Ein Job, den eine Middleware zurück auf die Queue legt, behält seine Sperre. [Warteschlange](queues.md) |
| Queue-Inspektion (`pendingJobs` / `delayedJobs` / `reservedJobs`) | `Queue::pending_jobs(queue)` / `delayed_jobs` / `reserved_jobs`; ein `Option<&str>` fasst Laravels `all*Jobs()`-Zwilling zu einem Aufruf zusammen | ausgeliefert | Das `InspectedJob`-DTO (`id`/`queue`/`name`/`attempts`/`payload`/`created_at`); der Trait-Standard ist ein ehrliches `Err` statt einer leeren Collection; `sync`/`null` überschreiben mit `Ok(vec![])`; Redis' `reserved_jobs` gilt pro Consumer. [Warteschlange](queues.md) |
| Zeitzone pro geplanter Task | `.timezone(chrono_tz::Tz)` / `.try_timezone("name")` pro Task, Standard über `Schedule::timezone`, `schedule:list --timezone` | ausgeliefert | Typisiertes `chrono_tz::Tz` statt Laravels String; der planweite Standard ist `Schedule::timezone` in `schedule::register` statt eines Config-Schlüssels `app.schedule_timezone`, und eine Task ohne festgelegte Zone behält die prozesslokale Zone. [Task-Planung](scheduling.md) |
| Ratenbegrenzung | `RateLimiter::for_signature(...)`, `ThrottleRequestsMiddleware`, `RateLimitMiddleware` | ausgeliefert | Gleitendes Fenster über `SlidingWindowConfig`. [Ratenbegrenzung](rate-limiting.md) |
| Suche (Scout) | Kein First-Party-Adapter für die Volltextsuche | noch nicht | Die Vektorsuche wird heute schon über [Vector](vector.md) ausgeliefert; ein Scout-Äquivalent für die Stichwortsuche ist geplant |
| Strings (Helfer) | Das Crate `heck` (Umwandlung der Schreibweise), `std::str`, `regex` | abweichend | Dieselben Crates, die der Rest des Rust-Ökosystems nutzt; kein globales `Str::camel($x)` |
| Task-Planung | `Schedule::call/command/task` + `#[derive(Task)]` + Cron-Syntax + der `schedule:run`-Worker | ausgeliefert | [Task-Planung](scheduling.md) |
| Idempotenz-Schlüssel | `Idempotency::remember(key, ttl, body)` - Replay-Schutz im Stripe-Stil | ausgeliefert | Der Aufrufer versieht den Schlüssel mit einem Namensraum aus Route + Benutzer- oder Geschäftsidentität. [Idempotenz](idempotency.md) |
| Anfrage-Timeout | `TimeoutMiddleware`, pro Route konfigurierbar | ausgeliefert | Rust-nativ - das laufende Future abbrechen, den Worker freigeben. [Timeout](timeout.md) |
| Feature Flags (Pennant) | `Feature` + `Evaluator` + `FeatureMiddleware` + Admin-CRUD | ausgeliefert | Ausbreitung im Sekundenbruchteil über den `FeatureSync`-Trait. [Feature Flags](feature-flags.md) |
| Beobachtbarkeit (Pulse) | OpenTelemetry über `init_telemetry`, `Metrics`, überall `tracing` | abweichend | OTel ist die Lingua franca für Beobachtbarkeit in Rust - richten Sie Ihren Collector auf die Binary. [Beobachtbarkeit](observability.md) |
| Telescope (Debug-Dashboard) | Noch kein Äquivalent | noch nicht | Auf v2+ verschoben; die tracing- und OTel-Ausgabe des Frameworks deckt die meisten Diagnosebedürfnisse ab |
| Pulse (Performance-Dashboard) | Noch kein Äquivalent | noch nicht | Wie bei Telescope - stellen Sie Metriken mit Ihrem bestehenden Stack für Beobachtbarkeit dar, bis ein Dashboard ausgeliefert wird |
| Vektorsuche | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | ausgeliefert | Kein Gatekeeping auf „nur Postgres pgvector“. [Vector-Suche](vector.md) |

### Suprnova-exklusiv (kein Laravel-Äquivalent)

| Suprnova | Was es ist | Hinweise / Link |
|---|---|---|
| `ws!()`-Makro + WebSocket-Handler | Typisierte WS-Routen, die sich Router und Middleware-Stack teilen | [WebSockets](websockets.md) |
| Workflows | Langlaufende zustandsbehaftete Arbeit mit Wiederholungen, Schlafphasen und Schrittgrenzen | [Workflows](workflows.md) |
| Supervisoren | `Supervisor`-Trait mit automatischem Neustart nach abgefangener Panic für langlebige Tokio-Tasks | [Supervisoren](supervisors.md) |
| Web Push (VAPID) | Browser-Push-Benachrichtigungen als erstklassiger Kanal | [Web Push](web-push.md) |
| Read/Write-Split über mehrere Connections | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [Datenbank](database.md) |
| HTTP/2 + WebSocket auf demselben Socket | `hyper.with_upgrades()` in `Server::run` | [Lifecycle](lifecycle.md) |
| Markdown-Inhalte + Docs-Pipeline | `MarkdownRenderer` (bereinigtes comrak → syntect → ammonia) + `build_docs(DocsBuildConfig)` → durchsuchbarer `DocsCatalog` aus `DocsChapter`n | Heading-Extraktion + `slugify_heading`; treibt Markdown-Doku und Blog ohne separaten Static-Site-Generator |

## Sicherheit

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Authentifizierung | `Auth::user/check/login/logout/attempt`, `Authenticatable`-Trait, `Guard` pro Name | ausgeliefert | [Authentifizierung](authentication.md) |
| Mehrere Guards | `Guard` über `AuthManager` nach Name registriert (`web`, `api`, …) | ausgeliefert | `SessionGuard`, `TokenGuard`, eigene Implementierungen |
| User-Provider | `EloquentUserProvider<U>`, `DatabaseUserProvider`, eigene über den `UserProvider`-Trait | ausgeliefert | [Auth-Flows](auth-flows.md) |
| E-Mail-Verifizierung | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail`; der `MustVerifyEmail`-Contract | ausgeliefert | Provider-gestützt und akteursgebunden - [Auth-Flows](auth-flows.md) |
| Passwort-Reset | `PasswordReset` + Magnetars Transaktion für den ersten E-Mail-Nachweis oder ein verifizierter `UserProvider`-Fallback + Reset-/Änderungs-Mail | ausgeliefert | Magnetar erledigt den atomaren ersten Nachweis; Provider-gestützte Apps können bereits verifizierte Benutzer zurücksetzen - [Auth-Flows](auth-flows.md) |
| Brute-Force-Drosselung | Magnetars Lockout-Engine + `BruteForce` + `LoginThrottleMiddleware` | ausgeliefert | Kontosperre plus IP-/Routen-Begrenzung des Frameworks |
| Zwei-Faktor (TOTP) | Die Kompatibilitäts-Facade `TwoFactor` des Frameworks plus Magnetars Faktor-Engine | ausgeliefert | Wiederherstellungscodes, Replay-Schutz und faktorgesteuerte integrierte Anmeldung |
| Remember-me | Magnetars zweckgebundenes rotierendes Credential hinter dem Framework-Cookie | ausgeliefert | Auth-Epochen-Prüfungen, Rotation, Anomalie-Behandlung und Legacy-Fallback |
| OAuth (Socialite) | Magnetars Provider-Registry und die Facade `Auth::oauth(provider)` | ausgeliefert | OAuth, Apples `form_post`, PKCE-/State-Bindung, Policy für verifizierte Identitäten - [OAuth](oauth.md) |
| Sanctum (API-Token) | `BearerTokenMiddleware` über Magnetars Bearer-Sitzungen | abweichend | Authentifiziert Bearer-Sitzungen; keine separate API zur Verwaltung von Sanctum-Token |
| Passport (OAuth-Server) | Magnetars Protokoll- und Plugin-Engines | abweichend | Die Engine-Primitive werden ausgeliefert; keine zu Laravel Passport kompatible Anwendungs-Facade |
| Fortify (Auth-Backend) | Die Facades `Auth`/`auth_flows` des Frameworks über Magnetars Engines | ausgeliefert | Das Framework besitzt HTTP, Mail, Ereignisse, Cookies und die Anwendungsbindung |
| Autorisierung (Policies / Gates) | `Gate::allows/denies` + `#[policy] impl PostPolicy` + `Authorizable`-Trait + Makro-Registrierung + `Gate::default_denial_response` | ausgeliefert | [Autorisierung](authorization.md) |
| Rollen & Berechtigungen (spatie/laravel-permission) | `HasRoles`-Trait + die Tabellen `roles` / `permissions` / `role_has_permissions` (`CreateRbacTables`) + `RoleMiddleware` / `PermissionMiddleware` (fail-closed) | ausgeliefert | First-Party, kein Community-Paket. Die Helfer `create_role` / `give_permission_to_role` / `assign_role_to_model`; setzt auf Gate/Policy auf. [Autorisierung](authorization.md) |
| Verschlüsselung | `Crypt::encrypt/decrypt` + `CryptPurpose`-AAD-Bindung | ausgeliefert | AES-256-GCM, Schlüsselrotation über `APP_KEY_PREVIOUS`. [Verschlüsselung](encryption.md) |
| Hashing | `hash::*` + `BcryptHasher`, `Argon2idHasher`, `Argon2iHasher`, `needs_rehash`, `is_hashed`, `verify` | ausgeliefert | Bcrypt ist der Standard; argon2id steht bereit. [Hashing](hashing.md) |

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
| Parent-Timestamps berühren | `#[model(touches = ["post"])]` | ausgeliefert | Ein `UPDATE` pro `BelongsTo`-Besitzer, eine Ebene tief und event-frei (keine Grandparent-Rekursion, kein `saved`-Event des Parents). `without_touching` / `without_touching_on::<M, _, _>()` zum Überspringen. [Parent-Touching](eloquent.md#parent-touching) |
| Observers | `impl Observer<User>` + `#[suprnova::observer(User)]` | ausgeliefert | 16 Lifecycle-Events |
| 16 Lifecycle-Events | `Created`, `Creating`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Retrieved`, `Replicating`, `ForceDeleting`, `ForceDeleted`, `Pruning` | ausgeliefert | Pro Modell ein `events::*`-Submodul. `EventResult::cancel(_)` unterbricht per Short-Circuit mit einem 400 |
| Mutators / Accessors | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | ausgeliefert | [Mutators](eloquent-mutators.md) |
| Casts (22 eingebaute) | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | ausgeliefert | `Cast` implementieren für eigene |
| Collections | `Collection<M>` mit `pluck`, `filter`, `map`, `each`, `chunk`, `groupBy`, `keyBy`, `sort_by`, `where_`, `first`, `last`, `count`, `is_empty`, `to_array` und Laravel-Verwandten; `Deref<Target = Vec<M>>`, sodass alle `Vec`-Idiome weiter funktionieren | ausgeliefert | [Collections](eloquent-collections.md) |
| `modelKeys()` | `Builder::model_keys().await?` (keine Hydration, qualifizierter Schlüssel) und `Collection::model_keys()` | ausgeliefert | Beide liefern `Vec<M::Key>`; das Builder-Terminal projiziert `users.id`, damit Joins überlebt werden |
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
| Pest-/PHPUnit-Stil | `#[suprnova_test]` (async-bewusst) + Jest-artige Assertions mit `expect!()` + BDD-Makros `describe!()` / `test!()` | ausgeliefert | Alle drei sind untereinander austauschbar |
| Feature-Tests (HTTP) | `handle_request(router, registry, req)` im selben Prozess treiben, normalerweise über eine Loopback-hyper-Verbindung, sodass der Server einen echten `Incoming`-Body erhält | ausgeliefert | [HTTP-Tests](http-tests.md) |
| `TestResponse`-Wrapper | `suprnova::testing::TestResponse` - Fluent-`assert_status` / `assert_json_path` / `assert_cookie` / `assert_session_has` und weitere, alle mit `&Self` verkettbar | ausgeliefert | [HTTP-Tests](http-tests.md#fluent-response-assertions-with-testresponse) |
| Inertia-Testhelfer | `suprnova::testing::AssertableInertia` - `component`/`url`/`version`/`prop`/`has`/`missing`/`where_`/`count`/`has_flash`, plus `reload_only`/`reload_except`/`load_deferred_props` über eine vom Aufrufer gelieferte `with_reload`-Closure | ausgeliefert | [HTTP-Tests](http-tests.md#testing-inertia-responses) |
| Konsolen-Tests | `dispatch_argv(["console", "..."])` ausführen und assertieren | ausgeliefert | Gleiche Form wie HTTP-Tests, für das Konsolen-Binary |
| Browser-Tests (Dusk) | Nicht im Framework - verwenden Sie Playwright / WebdriverIO / den `gstack`-Agent-Browser | absichtlich nicht | Sprachübergreifendes Tooling existiert bereits; wir erfinden es nicht neu |
| Datenbank-Tests | `TestDatabase::fresh::<Migrator>()` | ausgeliefert | Erstellt eine frische In-Memory-SQLite-Datenbank pro Test, wendet Migrationen an, registriert sie im Test-Container und verwirft diesen isolierten Datenbank-/Container-Zustand beim Drop; es umschließt nicht jeden Test mit einer Rollback-Transaktion. [Datenbank-Tests](database-testing.md) |
| Mocking & Fakes | Fakes pro Facade: `MailFake`, `NotifyFakeGuard`, `EventFakeGuard`, `Queue::fake`, `Bus::fake`, `Http::fake`, `Storage::fake` | ausgeliefert | Aufgezeichnete Aufrufe + Assertion-Helfer. [Mocking](mocking.md) |
| `QueueFake`-Job-UUIDs | `queue::testing::pushed_with_id::<J>()` | ausgeliefert | Das Fake versieht pro Push ein Envelope mit einer ID und emittiert dasselbe `JobQueued`, das ein echter Push ausgibt |
| Zeitreise | `tokio::time::{pause, advance, resume}` aus der Standard-Runtime | ausgeliefert | Wir liefern keine eigene aus - Tokios API kann das bereits |
| Container-Isolation | `TestContainer::fake(\|tc\| tc.bind(...))` - thread-lokal | abweichend | Konstruktionsbedingt parallelsicher. [Service Container](container.md) |

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

## Frontend (Laravel hat Blade + Starter Kits; wir haben Inertia)

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Blade | n/a - Inertia ist die View-Schicht | abweichend | [Frontend](frontend.md) |
| Inertia.js | First-Class: v3 über Svelte 5 / React 19 / Vue 3.5 | ausgeliefert | [Inertia Responses](frontend-inertia-responses.md), [Seiten-Komponenten](frontend-pages.md) |
| `Route::inertia($uri, $component, $props)` | `Router::inertia(path, component, props)` | ausgeliefert | Gibt einen `RouteBuilder` zurück, sodass `.name(...)` / `.middleware(...)` verkettet werden; `Router::view` ist der ältere Alias |
| Auflösung der Seiten-URL (`Inertia::resolveUrlUsing`) | `page.url` ist Pfad + Query; überschreibbar mit `InertiaConfig::url_resolver` | ausgeliefert | Die Standard-Ableitung stimmt Byte für Byte mit dem `X-Inertia-Location` der Versions-Middleware überein; ein `url_resolver` ändert nur `page.url` |
| Inertia-Protokoll-Middleware (`Vary`, leere Response, Versions-Bounce) | `InertiaHeadersMiddleware` + `InertiaVersionMiddleware` + `Inertia303Middleware` - drei der vier Middlewares, die `Inertia::install` verdrahtet (die vierte, der Validierungsfehler-Redirect, ist die nächste Zeile) | ausgeliefert | `Vary: X-Inertia` auf jeder Response; eine leere `200` bei einem Inertia-Besuch wird zu einem `303` zurück; der 409-Bounce flasht die Session erneut |
| Validierungsfehler-Redirect (`Middleware::resolveValidationErrors`, `$withAllErrors`) | `InertiaValidationRedirectMiddleware`, verdrahtet durch `Inertia::install`; `InertiaConfig::with_all_errors(bool)` | ausgeliefert | Eine `422` bei einem Inertia-Besuch wird zu einem `303` zurück, wobei die Fehler geflasht werden; der Wert eines Felds reduziert sich auf seine erste Nachricht, außer bei `with_all_errors(true)`. [Inertia Responses](frontend-inertia-responses.md#validation-failures) |
| Externer Redirect + History-Leerung | `InertiaResponse::location_for(&req, url)`, `App::clear_history()` | ausgeliefert | `location_for` ist `409` für XHR und `302` für eine harte Navigation; `App::clear_history()` überlebt den Logout-Redirect |
| `Inertia::share` / `getShared` / `flushShared` | `App::inertia_share` / `_lazy` / `_once`, `App::inertia_shared(key)`, `App::flush_inertia_shared()` | ausgeliefert | Verschachtelung per Punkt-Schlüssel mit `Arr::set`-Semantik; das pro Anfrage bereitgestellte `InertiaSharedData::share(&req, component)` kann nach Seite variieren. Ein geteilter Schlüssel mit Punkten bleibt bis zum Entpack-Durchlauf der Response flach, sodass `only`/`except` mit einem übergeordneten Eintrag übereinstimmen (`only: ['auth']` erreicht `auth.user`), während Laravel dasselbe Ergebnis durch `Arr::set` bereits beim Teilen erhält |
| Partial Reloads | `#[derive(Data)]` + `req.includes("subset")` + Inertias Partial-Reload-Protokoll | ausgeliefert | Typsichere Include-Sets. `?include=` begrenzt jede Lazy-Variante einschließlich `lazy(deferred)` und läuft vor `X-Inertia-Partial-Data`, sodass ein nicht erlaubtes Include weiterhin 400 zurückgibt. `errors` ist von `only`/`except` ausgenommen, entsprechend Laravels `Inertia::always`-Share |
| Deferred Props | `.defer(…)` / `.defer_with(…, DeferOptions)`, oder `Prop::…defer()` | ausgeliefert | Inertia-v3-Protokoll für Deferred Props; `DeferOptions` trägt die Gruppe und das Rescue-Flag. `deferredProps` wird nur beim ersten Besuch ausgeliefert - `resolveDeferredProps` gibt `[]` bei jedem passenden Partial zurück |
| Merge Props | `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with(MergeStrategy)` / `.merge_lazy` / `.merge_lazy_with`, oder `Prop::…merge().merge_with_path(...)` | ausgeliefert | Inertia-v3-Merge-Protokoll; `match_on` nimmt ein oder mehrere Felder an; `merge_with_path` merged ein verschachteltes Feld statt der Wurzel der Prop |
| Prop-Komposition (`defer()->merge()`, `merge()->once()`, `optional()->once()`) | `Prop`-Flag-Builder + `InertiaResponse::prop(key, prop)` | ausgeliefert | `Prop` ist eine Struktur mit orthogonalen Flags und spiegelt die Interfaces `Deferrable` / `Mergeable` / `Onceable` des PHP-Adapters |
| History verschlüsseln | `EncryptHistoryMiddleware` | ausgeliefert | Die History wird im Client verschlüsselt abgelegt |
| Scroll-Position | `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.paginate` + `ScrollMetadata` / `ProvidesScrollMetadata` | ausgeliefert | Automatische Wiederherstellung bei Navigation; `reset` liest `X-Inertia-Reset`, entsprechend `resolveScrollProps` |
| TypeScript-Typen | `suprnova generate-types` liest `#[derive(InertiaProps)]` und gibt `.d.ts` aus | ausgeliefert | [TypeScript Types](frontend-typescript-types.md) |
| Vite-Manifest lesen | Automatisch verdrahtet über `InertiaConfig::manifest_path` | ausgeliefert | HMR in Dev, gehashte Assets in Prod. `Inertia::install` schlägt in Produktion geschlossen fehl, wenn das Manifest fehlt |
| Asset-Version aus dem Build-Manifest | `InertiaConfig`-Standard: `VersionResolver::from_manifest(manifest_path)` | ausgeliefert | Hash der Manifest-Bytes; statischer Fallback `"1.0"`, wenn kein Build zum Hashen vorhanden ist |
| Inertia SSR (`inertia:start-ssr`) | `InertiaConfig::ssr(...)` auf der Config, die an `Inertia::install` übergeben wird; der Worker wird von `suprnova ssr:start` gestartet | ausgeliefert | Worker außerhalb des Prozesses über HTTP-Loopback; fällt bei Fehler oder Timeout auf CSR zurück, sofern nicht `ssr_throw_on_error(true)` gesetzt ist. `InertiaConfig::ssr_bundle_path(...)` begrenzt Dispatch darauf, dass das gebaute Bundle auf dem Datenträger vorhanden ist (spiegelt `ensure_bundle_exists`), umschaltbar mit `.ssr_ensure_bundle_exists(bool)` (standardmäßig an, sobald ein Bundle-Pfad gesetzt ist); `suprnova new` erzeugt für jeden Starter `frontend/src/ssr.{ts,tsx}` und ein `build:ssr`-Skript; `suprnova ssr:check` prüft die Route `GET /health` des Workers. [Inertia Responses](frontend-inertia-responses.md) |

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

## Bereitstellung

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| `php artisan optimize` | `cargo build --release` | abweichend | Ein Binary, kein Opcache-Schritt |
| `php artisan config:cache` | Typisierte Config wird bereits zur Compile-Zeit geprüft | abweichend | Kein Laufzeit-Cache, der invalidiert werden müsste |
| `php artisan route:cache` | Routen werden zur Compile-Zeit per Makro expandiert | abweichend | Der Router wird beim Boot aus bereits typisierten Routen gebaut |
| Envoy (SSH-Deploys) | Verwenden Sie einen beliebigen Orchestrator - Docker, systemd, Kubernetes, fly.io, Railway | absichtlich nicht | Das Binary ist das Deploy-Artefakt |
| Forge / Vapor | Nicht unsere Sache - aber die Rezepte für Railway, DO und Hetzner decken dieselbe Aufgabe ab | abweichend | [Bereitstellung](deployment.md), [Railway](deployment-railway.md), [Digital Ocean](deployment-digital-ocean.md), [Hetzner](deployment-hetzner.md) |
| Wartungsmodus (`php artisan down` / `up`) | `./app down` / `./app up` - Bypass-Secret, eigene retry/message/except-Pfade, Treiber `file` oder `cache` | ausgeliefert | [Bereitstellung](deployment.md) |
| Horizon (Queue-Dashboard) | Noch kein Dashboard | noch nicht | Bis dahin Inspektion fehlgeschlagener Jobs über `cargo run --bin console queue:failed` |

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
| Sanctum | `BearerTokenMiddleware` über Magnetar-Bearer-Sessions | abweichend | Keine separate Package- oder Personal-Access-Token-Verwaltungsoberfläche |
| Scout (Volltextsuche) | n/a bisher | noch nicht | Vektorsuche wird ausgeliefert ([Vector](vector.md)); Keyword-Scout-Äquivalent später |
| Socialite | Magnetar-Provider-Registry und `Auth::oauth(provider)` | ausgeliefert | [OAuth](oauth.md) |
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

## Was wir wirklich noch nicht haben

Eine gebündelte Liste jedes **noch nicht** von oben, damit Sie die Form
der Lücke an einer Stelle sehen:

| Bereich | Was fehlt | Behelf bis zur Auslieferung |
|---|---|---|
| Suche (Scout - Stichwort) | Adapter für Algolia / Meilisearch / Elastic | Bauen Sie sich bis dahin selbst etwas mit `meilisearch-sdk` / `elasticsearch`; [Vector](vector.md) deckt die semantische Suche schon heute ab |
| Passport (OAuth-Server) | Ein First-Party-OAuth-Identity-Provider | Betreiben Sie Hydra / Keycloak hinter Suprnova |
| Telescope (Debug-Dashboard) | Web-UI für Anfragen / Queries / Ereignisse / Cache-Treffer | Nutzen Sie OTel + die tracing-Ausgabe ([Beobachtbarkeit](observability.md)) |
| Pulse (Performance-Dashboard) | Web-UI für langsame Queries / Fehler / heiße Routen | Dasselbe: heute die OTel-Oberfläche, das Dashboard später |
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
| `php artisan dev --tabs` (TUI-Modus für mehrteilige Dev-Prozesse) | Ein einzelnes Terminal mit `[name]`-präfixierter Ausgabe ist die Rust-Norm für Dev-Tools (`cargo watch`, `bacon`, `just`) - `suprnova serve` gibt bereits jedem Prozess (Backend, Frontend und jedem `Suprnova.toml`-Eintrag) ein eigenes farbiges Präfix und automatischen Neustart. Ein Tab-TUI wäre ein zweites Interaktionsmodell für ein Signal, das bereits vorhanden ist; die Aufgabe von `--stream` - ein skriptbarer Echtzeit-Ausgabestrom - wird als `suprnova serve --json` ausgeliefert (NDJSON, ein Event pro Zeile). [Serve](cli-serve.md#extra-dev-processes) |

## Wie diese Liste ehrlich bleibt

Jede Zeile in der Spalte **ausgeliefert** lässt sich verifizieren durch:

1. Greppen von `framework/src/lib.rs` nach dem genannten Export
2. Ausführen der Framework-Testsuite (`cargo test --workspace`)
3. Lesen des verlinkten Kapitels

Jede Zeile in der Spalte **noch nicht** ist beabsichtigte Arbeit, keine
Absage. Jede Zeile in der Spalte **absichtlich nicht** trägt eine
einsätzige Begründung in der Spalte Hinweise; diese Gründe sind die
Designprinzipien aus der [Einführung](introduction.md), angewandt auf ein
konkretes Feature.

Zuletzt gegen Laravel 13.25.0 geprüft.

Wenn Sie ein Laravel-Feature vermissen, zu dem Sie greifen und das nicht
auf dieser Karte steht, öffnen Sie ein Issue - entweder gibt es dafür eine
Suprnova-Antwort, der die Zeile fehlt, oder es ist eine echte Lücke, und
wir wollen davon wissen.

## Nächste Schritte

- [Von Laravel kommend](from-laravel.md) - dieselbe Karte, als
  Seite-an-Seite-Erzählung
- [Einführung](introduction.md) - die Design-Prinzipien, denen diese
  Parity-Arbeit folgt
- [`documentation.md`](documentation.md) - das Master-Inhaltsverzeichnis
  über jedes Kapitel hinweg

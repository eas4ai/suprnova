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
| Routendefinitionen | Makro `routes!` + `get!` / `post!` / `put!` / `patch!` / `delete!` / `any!` / `head!` / `options!` / `fallback!` / `ws!` | ausgeliefert | [Routing](routing.md) |
| Routenparameter | Pfadparameter `{id}` + `req.param("id")` | ausgeliefert | Optionale Parameter über `{id?}`; Constraints über `where!()` |
| Routennamen | `.name("posts.show")` an der Route + `url("posts.show", &[("id", "42")])` | ausgeliefert | [URL-Generierung](urls.md) |
| Routengruppen | Makro `group!` mit `.prefix()` / `.middleware()` / `.name()` / `.controller()` | ausgeliefert | Gruppen-Middleware wird bei der Registrierung auf jede Route flachgezogen |
| Resource-Routen | `resource!("posts", PostController)` registriert die 7 Standardrouten | ausgeliefert | `apiResource!`, `only(...)`, `except(...)` werden alle unterstützt |
| Signierte URLs | `sign_url(...)`, `sign_route(...)`, `verify_signature(...)` | ausgeliefert | HMAC-SHA256 mit `APP_KEY` |
| Route-Model-Binding | `#[handler]` extrahiert `Post` aus `{post}` über eine `RouteBinding`-Implementierung | ausgeliefert | Das Derive `AutoRouteBinding` implementiert es für `#[suprnova::model]`-Typen automatisch |
| Ratenbegrenzung | Middleware `throttle:60,1` + `RateLimiter::for_signature` | ausgeliefert | [Ratenbegrenzung](rate-limiting.md) |
| Middleware | Trait `impl Middleware`; global oder pro Route registrieren | ausgeliefert | [Middleware](middleware.md) |
| Middleware-Gruppen + Aliase | `register_middleware_group`, `register_middleware_alias` | ausgeliefert | Werden in Routen über ihren Namen als String nachgeschlagen |
| CSRF-Schutz | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | ausgeliefert | Standard ist die Token-Prüfung pro Session. Die optionalen Richtlinien `SameOriginOnly`, `AllowSameSite` und `OriginOnly` ziehen `Sec-Fetch-Site` heran; die Origin-Durchsetzung ist standardmäßig nicht aktiv. [CSRF](csrf.md) |
| Controller | `#[handler] pub async fn show(req: Request) -> Response` | ausgeliefert | Controller sind Module aus freien Funktionen, keine Klassen. [Controller](controllers.md) |
| Single-Action-Controller | Ein Handler ist bereits eine einzelne Funktion; fassen Sie sie in Modulen zusammen | ausgeliefert | Die Rust-Konvention - kein `__invoke`-Zeremoniell |
| Anfragen | Struktur `Request` mit `.input()`, `.param()`, `.query()`, `.header()`, `.cookie()`, `.json()`, `.file()` usw. | ausgeliefert | [Anfragen](requests.md) |
| Form-Requests | `#[derive(Data, Validate, FormRequest)]` | ausgeliefert | Die Validierung läuft beim Extrahieren |
| Datei-Uploads | `req.file("avatar")?` liefert `UploadedFile`; Streaming-Multipart mit Größen- und Teilobergrenzen | ausgeliefert | Ab einem Schwellenwert automatisches Auslagern in eine temporäre Datei |
| Antworten | `HttpResponse`-Builder + `json_response!()` / `text_response!()` / `Redirect::to` / Inertia-Responses | ausgeliefert | [Antworten](responses.md) |
| Gestreamte Antworten (`eventStream`, `stream`, `streamJson`) | `HttpResponse::sse(...)` / `event_stream(...)` / `stream_bytes(...)` / `stream_json(...)` | ausgeliefert | Dieselben Wire-Formen, die die Hooks von `@laravel/stream-{react,vue,svelte}` erwarten. [Server-Sent Events](sse.md) |
| `withoutCookie` / `withoutCookies` | `.without_cookie(name)` / `.without_cookies([...])` auf `HttpResponse`, `Response`, `Redirect`, `RedirectRouteBuilder` | ausgeliefert | `Cookie::forget_with(name, path, domain)` für ein Cookie, das nicht auf `/` gesetzt wurde |
| Views (Blade) | Serverseitig gerenderte Inertia-Seiten (Svelte/React/Vue) - kein Blade-Äquivalent | abweichend | Inertia ist die View-Schicht. Nutzen Sie [Seiten-Komponenten](frontend-pages.md) statt Blade |
| Asset Bundling (Vite) | Vite 8 liegt jedem Scaffold bei; `suprnova serve` startet Vite und Backend gemeinsam | ausgeliefert | Manifest-Auswertung + HMR automatisch verdrahtet |
| Statische Assets (`public/`, in Laravel vom Webserver ausgeliefert) | Prozessinterner Fallback-Handler `StaticFiles::public()`, der `public/` unter dem Web-Root ausliefert | ausgeliefert | `StaticFiles::from_dir(...)` + `cache_control(...)`; kein separater Webserver nötig |
| URL-Generierung | `url("posts.show", &[…])`, `route("posts.show", …)`, `redirect(...)`, `redirect_to(...)` | ausgeliefert | [URL-Generierung](urls.md) |
| Session | `session()`, `session_mut()`, Flash-Bag über `req.flash()` | ausgeliefert | Standardmäßig datenbankgestützt über `DatabaseSessionDriver`; das verschlüsselte Browser-Cookie trägt die Session-Kennung und die Aktivitäts-Metadaten, nicht den Datenbestand der Session. [Sitzungen](session.md) |
| Cookie-Queue (`Cookie::queue`) | `Cookie::queue`/`queued`/`unqueue`/`expire` - ein task-lokales Jar, das `SessionMiddleware` auf die Antwort leert | ausgeliefert | Erfordert `SessionMiddleware` in der Chain; die Einreihung erfolgt nach Name, nicht nach Name + Pfad wie bei Laravels `CookieJar` |
| Validierung | `#[derive(Validate)]` + 35 eingebaute Regeln + die Traits `Rule`/`ValueRule`/`AsyncRule` | ausgeliefert | `Url` nutzt Laravels Schema-Allowlist, und `Url::protocols([...])` spiegelt `url:http,https`. Asynchrone Regeln (z. B. `Unique`) gehen an die Datenbank. `ArrayKeys`/`Distinct` sind `ValueRule`s über `serde_json::Value` und entsprechen Laravels `array:keys` und `distinct`. `InArray` nimmt die Liste des anderen Feldes direkt entgegen statt Laravels Regel-String `in_array:other.*`, und `Contains`/`DoesntContain` treffen JSON-String-Elemente exakt. `Gt`/`Gte`/`Lt`/`Lte` nehmen einen expliziten `CompareWith`-Operanden (Zahlenliteral, numerisches Geschwisterfeld oder ein nach Zeichenzahl verglichenes Geschwisterfeld), statt Laravels vier Größenmaße abzuleiten; für Array- und Datei-Vergleiche gibt es keine Entsprechung. [Validierung](validation.md) |
| Regel `Password` (`Password::defaults()`, `uncompromised()`) | `Password::min(n)` + Stärke-Builder (`.letters()`, `.mixed_case()`, `.numbers()`, `.symbols()`) + `.uncompromised()` | ausgeliefert | K-Anonymitätsprüfung gegen Have I Been Pwned; lässt das Passwort bei einem Netzwerkfehler durchgehen, wie Laravels `NotPwnedVerifier`. [Validierung](validation.md#password-strength) |
| Fehlerbehandlung | `FrameworkError`, `AppError`, Trait `HttpError`, Panic-Grenze in `execute_chain_safely` | ausgeliefert | [Fehlerbehandlung](errors.md), [Fehlermodell](error-model.md) |
| Protokollierung | `tracing`-Subscriber mit strukturierten Feldern, `LogFormat` (json / pretty / compact) | abweichend | Eine Logzeile ist ein JSON-Dokument; `request_id` ist immer vorhanden. [Protokollierung](logging.md) |
| Log-Kanäle / Datei-Treiber (`single`, `daily`, `monthly`, `stack`) | `tracing` schreibt strukturierte Zeilen nach stdout; die Plattform rotiert und transportiert sie | absichtlich nicht | Container, systemd und jeder Log-Shipper erledigen Rotation und Aufbewahrung bereits. Das prozessintern nachzubauen dupliziert die Plattform und versteckt die Logs vor ihr. [Protokollierung](logging.md) |
| Abort-Helfer | `abort_if(cond, status, msg)`, `abort_unless(...)`, `abort_with(status, msg)` | ausgeliefert | Gleiche Form wie Laravels `abort_if`-Familie |

## Tiefere Einblicke

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| Artisan Console | App-eigenes `console`-Binary, gebaut aus `#[command]` + `#[derive(Command)]` | ausgeliefert | [Konsole](console.md). `cargo run --bin console <subcommand>` |
| Tinker (REPL) | Keine REPL | absichtlich nicht | Schreiben Sie ein einmaliges `cargo run --bin xxx`-Skript oder einen `#[suprnova_test]` |
| Broadcasting | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | ausgeliefert | sea-streamer-Fanout für mehrere Knoten. [Broadcasting](broadcasting.md) |
| Cache | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`, `RedisCache` | ausgeliefert | Atomare Operationen + getaggter Cache + Cache-Sperren (`LockGuard`). [Cache](cache.md) |
| Collections | `eloquent::Collection<M>` mit Methoden in Laravel-Form | ausgeliefert | `Deref<Target = Vec<M>>`, sodass bestehende Vec-Idiome weiter funktionieren. [Collections](eloquent-collections.md) |
| Nebenläufigkeit | Überall Tokio - `tokio::spawn`, `tokio::join!`, `tokio::select!` | ausgeliefert | Das gesamte Framework ist asynchron. Laravels Facade `Concurrency::run([...])` wird nicht ausgeliefert; Tokio ist die Antwort |
| Kontext | `Context::put` / `Context::get` / `ContextStore` + automatisches Einschleusen in Queue / Mail / Events | ausgeliefert | [Kontext](context.md) |
| Contracts | Alle öffentlichen Nahtstellen sind Traits | ausgeliefert | Siehe die Zeile „Architektur / Contracts“ weiter oben |
| Events | `EventFacade::dispatch(e).await?`, `#[derive(Event)]`, `EventDispatcher`, Queued Listener, Subscriber | ausgeliefert | [Ereignisse](events.md) |
| Dateispeicher | `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` über OpenDAL | ausgeliefert | Dieselbe Oberfläche `put/get/delete/copy/move/exists/url`. Schutz vor Path Traversal ist eingebaut. `Storage::register_read_through` komponiert zwei Disks zu einer Read-Through-Disk, die Treffer auf dem Fallback auf die primäre Disk hochzieht, mit `copy: false` zum Überspringen der Übernahme und mit `copy` / `rename` über die Fallback-Grenze hinweg. [Dateisystem](filesystem.md) |
| Helfer | Die Entsprechungen liegen in ihren Heimatmodulen (kein Sammelbecken `helpers.md`) | abweichend | Zum Beispiel liegen URL-Helfer in [urls.md](urls.md), String-Helfer in `std`/`heck`, Array-Helfer in `std::collections` - Rust löst das über Crates, nicht über einen globalen Namensraum |
| HTTP-Client | Builder `Http::get/post/...` + `Http::fake(...)` für Tests | ausgeliefert | Zeichnet Anfragen automatisch auf; `assert_sent` / `assert_not_sent`; `.retry_when(predicate)` verengt die eingebaute Retry-Richtlinie über einen `RetryContext`. [HTTP-Client](http-client.md) |
| Image (`Illuminate\Image`) | `Image::from_bytes/from_path/from_disk/from_upload/from_stream` + dieselbe Operations- und Terminal-Oberfläche | ausgeliefert | Liegt in `suprnova::media`. Zwei Treiber wie Laravels `gd`/`imagick`: `IMAGE_DRIVER=oxideav` (Standard, reines Rust) oder `magick`. Liest und schreibt PNG, JPEG, WebP, GIF, BMP; die AVIF-Ausgabe wartet auf die Veröffentlichung des hauseigenen AV1-Encoders. Dekodier-Limits werden anhand des Headers geprüft. [Bilder](images.md) |
| HEIC-Dekodierung im Standardtreiber | `IMAGE_DRIVER=magick` auf einem Host mit dem libheif-Delegate | absichtlich nicht | HEVC ist patentbelastet, und der einzige glaubwürdige reine Rust-Decoder steht unter dualer AGPL-/Kommerzlizenz, deshalb wird kein eingebauter Decoder ausgeliefert. Gleiche Form wie bei Laravel, wo GD HEIC überhaupt nicht lesen kann und Imagick den Delegate sowohl im Binary als auch in der PHP-Erweiterung einkompiliert braucht. [Bilder](images.md#why-suprnova-diverges) |
| Lokalisierung | `Lang::get` / `get_with` / `try_get` / `has` + das Makro `__!("key", name: value)` über Fluent-`.ftl`-Kataloge in `lang/<locale>/`, Erkennung per `LocaleMiddleware`, übersetzte Validierungsmeldungen, ICU4X-Formatierung | ausgeliefert | Derselbe Katalog wird dem Browser unter `/_suprnova/lang/<locale>.ftl` ausgeliefert und von `generate-types` typisiert. [Lokalisierung](localization.md) |
| Mail | `Mail::to(...).send(MyMail { ... }).await?` + die Treiber `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory/file` | ausgeliefert | `Mailable`-Trait + per Tera gerenderte HTML-/Text-Rümpfe; SES-Sendungen tragen `TenantName` / `ConfigurationSetName` / `ListManagementOptions`; ein Dispatch über die Queue wird per `.on_queue(...)` / `.on_connection(...)` geroutet und sticht `Queue::route` aus. [Mail](mail.md) |
| Benachrichtigungen | `Notify::send(&user, notif).await?` + die Kanäle `mail/database/broadcast/webpush` | ausgeliefert | `Notifiable`-Trait + `Notification` pro Kanal; ein Dispatch über die Queue (`Notify::queue`) trägt `queue`/`timeout`/`fail_on_timeout`/`max_tries`/`backoff` pro Benachrichtigung über dasselbe `EnvelopeOverrides`-Primitiv, das auch Mail nutzt, an den Job jedes Kanals. [Benachrichtigungen](notifications.md), [Web Push](web-push.md) |
| Package-Entwicklung | Adapter-Crates im Workspace (z. B. `suprnova-payments-stripe`) | ausgeliefert | Gleiche Form wie Laravel-Packages: vom Framework abhängen, in den Container binden, bei Bedarf Makros bereitstellen |
| Prozesse (Shell-Befehle ausführen) | `tokio::process::Command` aus der Standardbibliothek | absichtlich nicht | Keine Facade - Tokios API hat bereits die richtige Form |
| Warteschlangen | `Queue::push(job).await?` + die Treiber `sync/memory/database/redis/null`, Batches, Chains, `JobMiddleware`, `FailedJobStore` | ausgeliefert | [Warteschlange](queues.md) |
| Vom Job deklarierte Verzögerung | `fn delay() -> Option<Duration>` auf `Job`, beachtet von `Queue::push` und `Queue::bulk` | ausgeliefert | Ein expliziter Aufruf von `Queue::push_later` / `Queue::later(delay, job)` sticht den Standardwert des Jobs immer aus. [Warteschlange](queues.md) |
| `Queue::forward` | `Queue::forward(from, to)` / `Queue::forward_on(from, to, connection)`, angewandt auf das Envelope und auf die `--queue`-Liste des Workers | ausgeliefert | Nur von Queue zu Queue: `connection` steuert die Umleitung, statt einen Treiber auszuwählen, und wird gegen den Connection-Namen des Prozesses verglichen, sodass der Push und der Anspruch des Workers am selben Wert hängen; `to` ist Pflicht, wo Laravels Variante optional ist. [Warteschlange](queues.md) |
| Event für übersprungene Unique-Jobs | `queue::events::UniqueJobSkipped { job_name, unique_id, connection }` | ausgeliefert | Wird auf der Push-Seite ausgelöst, wenn `push_unique` dedupliziert; der Aufruf gibt weiterhin `Ok(false)` zurück |
| Queues pausieren (`queue:pause` / `queue:resume`) | `Queue::pause`/`resume`/`pause_all`/`resume_all`/`is_paused`/`paused_queues`, Cache-gestützt, mit den Events `QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` | ausgeliefert | Eine Pause pro Queue greift nur bei einem Worker, der mit einer expliziten `--queue=...`-Liste gestartet wurde; `resume_all` hebt eine Pause pro Queue nicht auf. Ein laufender Worker gibt außerdem einmal pro Übergang `WorkerQueuePaused` / `WorkerQueueResumed` aus, und `queue:work` schreibt zu jedem eine Zeile; ihr Feld `queue` ist ein `Option<String>`, denn ein ohne `--queue` gestarteter Worker hat unter einer globalen Pause keine Queue-Namen zu melden. [Warteschlange](queues.md) |
| Wiederholung transienter Redis-Befehle | Lesende Redis-Befehle wiederholen einmal, wenn die Verbindung ausfällt, wobei jeder Versuch das Reconnect-Budget des Treibers abwartet; `REDIS_COMMAND_RETRIES` fügt weitere hinzu | ausgeliefert | Laravels `command_retries` deckt über einen einzigen Dispatch-Punkt und eine Allowlist mit 60 Einträgen jeden Befehl ab; Suprnova wiederholt pro Aufrufstelle, und keine Einstellung lässt einen Schreibzugriff oder ein Queue-Pop erneut laufen. [Cache](cache.md) |
| Dispatch nach dem Commit (`afterCommit()`) | `fn after_commit() -> bool` auf `Job`, `EnvelopeOverrides::after_commit` pro Push, `Queue::push_after_commit` | ausgeliefert | Der gesamte Push wartet auf den Commit, Events eingeschlossen, und ein Rollback verwirft ihn; ein aufgeschobenes `push_unique` nimmt seine Sperre trotzdem sofort, damit die Deduplizierung innerhalb der Transaktion funktioniert. Ein manuelles `DB::begin_transaction` schiebt nie auf. [Warteschlange](queues.md) |
| Failover-Queue-Connection | `FailoverQueueDriver` über eine geordnete Connection-Liste, per `QUEUE_DRIVER=failover` + `QUEUE_FAILOVER_CONNECTIONS` | ausgeliefert | Schreibzugriffe fallen durch die Liste; `pop`, die Zähler und die Auflistungen bleiben auf der ersten Connection, also braucht jeder Fallback einen eigenen Worker. `QueueFailedOver` ist flankengesteuert, und `bulk_push` fällt pro Envelope durch, sodass jedes seine eigene Verzögerung behält. [Warteschlange](queues.md) |
| `ShouldBeUniqueUntilProcessing` | `fn unique_until_processing() -> bool` auf `Job`, freigegeben nach dem Middleware-Durchlauf und vor dem Handler | ausgeliefert | Die Freigabe ist an den Besitzer gebunden, sodass ein erneut zugestellter Versuch nie die Sperre eines neueren Dispatches freigibt. Ein Job, den eine Middleware zurück auf die Queue legt, behält seine Sperre. [Warteschlange](queues.md) |
| Entprellte Jobs (`#[DebounceFor]`) | `Job::debounce_for` / `max_debounce_wait` / `debounce_id`, dazu `Queue::push_debounced(job, DebounceOptions)` | ausgeliefert | Trait-Methoden statt eines Klassenattributs, sodass ein Tippfehler ein Compile-Fehler ist. Jeder Dispatch wird eingereiht, und das Zusammenfassen wird im Worker entschieden; ein Job, der sowohl `debounce_for` als auch `unique_id` deklariert, wird mit einem `FrameworkError` abgelehnt, wo Laravel wirft. Chains und Batches lehnen einen entprellten Job rundheraus ab. [Warteschlange](queues.md) |
| Entprellte Queued Listener | `Job::debounce_for` auf dem Job des Listeners oder `DebouncedListener::new(window, build).keyed_by(...)` | ausgeliefert | Laravel setzt das Attribut auf die Listener-Klasse; Suprnovas Brücke vom Listener zum Job läuft ohnehin über `Queue::push`, also deckt die Deklaration am Job den Regelfall ab, und `DebouncedListener` deckt ein Fenster pro Registrierung ab. [Ereignisse](events.md) |
| Queue-Inspektion (`pendingJobs` / `delayedJobs` / `reservedJobs`) | `Queue::pending_jobs(queue)` / `delayed_jobs` / `reserved_jobs`, ein `Option<&str>` fasst Laravels `all*Jobs()`-Zwilling zu einem Aufruf zusammen | ausgeliefert | DTO `InspectedJob` (`id`/`queue`/`name`/`attempts`/`payload`/`created_at`); der Trait-Standard ist ein ehrliches `Err` statt einer leeren Collection; `sync`/`null` überschreiben mit `Ok(vec![])`; `reserved_jobs` ist bei Redis pro Consumer. Anders als bei Laravel folgen diese Aufrufe keinem `Queue::forward`, sie melden also die wörtlich benannte Queue - und genau so bleibt ein Rückstau, der auf einer weitergeleiteten Queue liegen geblieben ist, sichtbar. [Warteschlange](queues.md) |
| Zeitzone pro geplanter Aufgabe | `.timezone(chrono_tz::Tz)` / `.try_timezone("name")` pro Aufgabe, `Schedule::timezone` als Standard, `schedule:list --timezone` | ausgeliefert | Typisiertes `chrono_tz::Tz` statt Laravels String; der planweite Standard ist `Schedule::timezone` in `schedule::register` statt eines Config-Schlüssels `app.schedule_timezone`, und eine nicht festgelegte Aufgabe behält die prozesslokale Zone. [Task-Planung](scheduling.md) |
| Ratenbegrenzung | `RateLimiter::for_signature(...)`, `ThrottleRequestsMiddleware`, `RateLimitMiddleware` | ausgeliefert | Gleitendes Fenster über `SlidingWindowConfig`. [Ratenbegrenzung](rate-limiting.md) |
| Suche (Scout) | Kein First-Party-Adapter für Volltextsuche | noch nicht | Vektorsuche wird heute über [Vector-Suche](vector.md) ausgeliefert; ein Scout-Äquivalent für die Stichwortsuche ist geplant |
| Strings (Helfer) | Crate `heck` (Schreibweisen-Umwandlung), `std::str`, `regex` | abweichend | Dieselben Crates, die das übrige Rust-Ökosystem nutzt; kein globales `Str::camel($x)` |
| Task-Planung | `Schedule::call/command/task` + `#[derive(Task)]` + Cron-Syntax + Worker `schedule:run` | ausgeliefert | [Task-Planung](scheduling.md) |
| Idempotenzschlüssel | `Idempotency::remember(key, ttl, body)` - Replay-Schutz im Stripe-Stil | ausgeliefert | Der Aufrufer versieht den Schlüssel mit einem Namensraum aus Route + Nutzer-/Geschäftsidentität. [Idempotenz](idempotency.md) |
| Request-Timeout | `TimeoutMiddleware`, pro Route konfigurierbar | ausgeliefert | Rust-nativ - das laufende Future abbrechen, den Worker freigeben. [Request-Timeouts](timeout.md) |
| Feature Flags (Pennant) | `Feature` + `Evaluator` + `FeatureMiddleware` + Admin-CRUD | ausgeliefert | Ausbreitung im Sekundenbruchteil über das `FeatureSync`-Trait. [Feature Flags](feature-flags.md) |
| Beobachtbarkeit (Pulse) | OpenTelemetry über `init_telemetry`, `Metrics`, überall `tracing` | abweichend | OTel ist die Lingua franca der Rust-Beobachtbarkeit - richten Sie Ihren Collector auf das Binary. [Beobachtbarkeit](observability.md) |
| Telescope (Debug-Dashboard) | Noch keine Entsprechung | noch nicht | Auf v2+ verschoben; die `tracing`- und OTel-Ausgabe des Frameworks deckt die meisten Diagnosebedürfnisse ab |
| Pulse (Performance-Dashboard) | Noch keine Entsprechung | noch nicht | Wie bei Telescope - fördern Sie Metriken mit Ihrem vorhandenen Observability-Stack zutage, bis ein Dashboard erscheint |
| Vektorsuche | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | ausgeliefert | Kein Gatekeeping nach dem Motto „nur Postgres pgvector“. [Vector-Suche](vector.md) |

### Suprnova-exklusiv (keine Laravel-Entsprechung)

| Suprnova | Was es ist | Hinweise / Link |
|---|---|---|
| Makro `ws!()` + WebSocket-Handler | Typisierte WS-Routen, die sich Router und Middleware-Stack teilen | [WebSockets](websockets.md) |
| Workflows | Lang laufende zustandsbehaftete Arbeit mit Wiederholungen, Schlafphasen und Schrittgrenzen | [Workflows](workflows.md) |
| Supervisoren | `Supervisor`-Trait mit automatischem Neustart nach abgefangenem Panic für langlebige Tokio-Tasks | [Supervisoren](supervisors.md) |
| Web Push (VAPID) | Browser-Push-Benachrichtigungen als First-Class-Kanal | [Web Push](web-push.md) |
| Read/Write-Trennung über mehrere Connections | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [Datenbank](database.md) |
| HTTP/2 + WebSocket auf demselben Socket | `hyper.with_upgrades()` in `Server::run` | [Request-Lifecycle](lifecycle.md) |
| Markdown-Inhalte + Docs-Pipeline | `MarkdownRenderer` (bereinigtes comrak → syntect → ammonia) + `build_docs(DocsBuildConfig)` → durchsuchbarer `DocsCatalog` aus `DocsChapter`s | Überschriften-Extraktion + `slugify_heading`; treibt Markdown-Docs und -Blog ohne separaten Static-Site-Generator |

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
| Mehrere Connections | `DB::on("read")` + `ConnectionRegistry` | ausgeliefert | Read/Write-Trennung als First-Class-Bürger |
| Transaktionen | `DB::transaction(\|tx\| async move { ... }).await?` | ausgeliefert | Savepoints + Wiederholung bei Deadlock |
| Query-Events | `QueryListener` + `QueryExecuted`-Event | ausgeliefert | `DB::listen(\|q\| { ... })` |
| Rohe Ausdrücke | `DB::raw("...")`, `DB::select("...", &[...])` | ausgeliefert | Parameter-Binding ist Pflicht (keine String-Interpolation) |
| Postgres / MySQL / SQLite | Alle drei First-Class über SeaORM | ausgeliefert | URL-Erkennung in `database::config::database_type()` |
| Postgres-DSN-Optionen `keepalives_*` | Pool-Lebendigkeit über `DB_IDLE_TIMEOUT` / `DB_MAX_LIFETIME` / `DB_ACQUIRE_TIMEOUT` / `DB_TEST_BEFORE_ACQUIRE` / `DB_PING_AFTER_IDLE` | abweichend | sqlx bietet keinen Setter für TCP-Keepalive, deshalb recycelt und pingt Suprnova stattdessen die Pool-Verbindungen. [Datenbank](database.md#pool-liveness) |
| MariaDB | First-Class als eigene Option (Vector + JSON + Temporal) | abweichend | Wird wegen der multiparadigmatischen Features gesondert behandelt, die Laravel nur für Postgres ausliefert |
| Redis | Wird von den Treibern genutzt (Cache/Queue/Rate-Limit) - keine eigene `Redis::*`-Facade | abweichend | Greifen Sie direkt zur `redis`-Crate, wenn Sie Ad-hoc-Befehle brauchen; Cache/Queue/Rate-Limit decken 95 % der typischen Nutzung ab |
| MongoDB | Noch kein First-Party-Adapter | noch nicht | Nutzen Sie die `mongodb`-Crate direkt über `App::bind` |
| Query Builder | `Builder<M>` mit `db_where` / `or_where` / `where_in` / `where_between` / `where_null` / `where_has` / `with` / `with_count` / `order_by` / `group_by` / `having` / `paginate` usw. | ausgeliefert | [Query Builder](queries.md) |
| `whereBinary()`-Familie | `Builder::where_binary` / `or_where_binary` / `where_not_binary` / `or_where_not_binary`, sowie `DB::table(...).where_binary(...)` | ausgeliefert | MySQL und MariaDB geben `= binary` aus; Postgres und SQLite liefern einen Fehler statt eines kollationsabhängigen Treffers. [Query Builder](queries.md) |
| Paginierung | `LengthAwarePaginator`, `Paginator` (einfach), `CursorPaginator` | ausgeliefert | Alle drei serialisieren zu JSON in Laravel-Form. [Paginierung](pagination.md) |
| Migrationen | `#[derive(DeriveMigrationName)] struct M;` + `up`/`down` + `Migrator` | ausgeliefert | Ausführung über `suprnova migrate`/`migrate:rollback`/`migrate:status`/`migrate:fresh`. [Migrationen](migrations.md), [CLI Migrationen](cli-migrations.md) |
| Seeder | `Seeder`-Trait + Unterbefehl `db:seed` | ausgeliefert | Factories pro Modell. [Seeding](seeding.md) |

## Eloquent ORM

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| `class User extends Model` | `#[suprnova::model(table = "users")] struct User { ... }` | ausgeliefert | Die Struktur IST das SeaORM-`Model`. [Eloquent](eloquent.md) |
| Find / first / get | `User::find(id)`, `User::query().first()`, `User::all()`, `Builder::get` | ausgeliefert | Alles asynchron |
| Create / update / delete | `User::create(attrs)`, `user.update(attrs)`, `user.delete()` | ausgeliefert | Makro `attrs! { name: "...", email: "..." }` für partielle Attribute |
| Schutz vor Mass Assignment | `#[model(fillable = [...])]` / `#[model(guarded = [...])]` + Scope `unguarded \|\| { ... }` | ausgeliefert | `prevent_silently_discarding_attributes()` für den strikten Modus |
| Soft Deletes | `#[model(soft_deletes)]` fügt automatisch `deleted_at` + das `SoftDeletes`-Trait ein | ausgeliefert | `with_trashed()`, `only_trashed()`, `restore()`, `force_delete()` |
| Prunable / MassPrunable | `#[prunable] impl Prunable for User { ... }` + Worker `model:prune` | ausgeliefert | Per Cascade an Relationen gebunden |
| Timestamps | Automatisches `created_at`/`updated_at`, wenn die Spalten vorhanden sind | ausgeliefert | Abschaltbar über `#[model(timestamps = false)]` |
| Primärschlüsseltypen | `i64` als Standard; UUID / ULID über `#[model(unique_id = "uuid")]` oder `unique_id = "ulid"` | ausgeliefert | Erzeugt die ID beim Insert automatisch |
| Lokale Scopes | `#[scopes(User)] impl User { fn active(b: &mut Builder<User>) { ... } }` | ausgeliefert | Methoden-Dispatch auf `Builder<M>` |
| Globale Scopes | `impl GlobalScope for ActiveOnly { ... }` + Registrierung | ausgeliefert | Entfernbar über `Builder::without_global_scope` |
| Relationen (11 Arten) | `HasOne`, `HasMany`, `BelongsTo`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`, `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany` | ausgeliefert | Morph-Enum pro Familie. [Relationships](eloquent-relationships.md) |
| `wherePivot`-Familie (inkl. der Closure-Form) | `where_pivot` / `where_pivot_op` / `where_pivot_in` / `where_pivot_not_in` / `where_pivot_null` / `where_pivot_not_null` / `where_pivot_between` / `where_pivot_not_between` / `where_pivot_group` plus die `or_`-Zwillinge | abweichend | Nur lesend - ein Pivot-Filter schränkt `attach` / `detach` / `sync` nie ein, und Eager Loads tragen ihn nicht mit. [Relationships](eloquent-relationships.md) |
| Eager Loading | `User::query().with(&["posts", "posts.comments"]).get()` | ausgeliefert | `EagerLoadDispatch` ist versiegelt; nur makrogenerierte Relationen können es implementieren |
| Verhinderung von Lazy Loading | `prevent_silently_discarding_attributes(true)` | ausgeliefert | Gleiche Form wie Laravels `preventLazyLoading` |
| Aggregate über Relationen | `with_count("posts")`, `with_sum("orders", "total")`, `with_avg`, `with_min`, `with_max` | ausgeliefert | Eine Subquery pro Aggregat |
| `whereHas` / `whereDoesntHave` | `where_has("posts", \|q\| q.db_where("published", "=", true))` | ausgeliefert | Engine mit korreliertem EXISTS |
| `loadMissing` | `user.load_missing(&["posts"]).await?` | ausgeliefert | Arbeitet über die gesamte Collection |
| Einen Datensatz klonen | `user.replicate()` / `user.replicate_into::<OtherType>()` | ausgeliefert | Löst das `Replicating`-Event aus |
| Timestamps des Elternteils berühren | `#[model(touches = ["post"])]` | ausgeliefert | Ein `UPDATE` pro `BelongsTo`-Besitzer, eine Ebene tief und ohne Events (keine Rekursion zum Großelternteil, kein `saved`-Event auf dem Elternteil). `without_touching` / `without_touching_on::<M, _, _>()` zum Überspringen. [Übergeordnete Modelle berühren](eloquent.md#parent-touching) |
| Observer | `impl Observer<User>` + `#[suprnova::observer(User)]` | ausgeliefert | 16 Lifecycle-Events |
| 16 Lifecycle-Events | `Created`, `Creating`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Retrieved`, `Replicating`, `ForceDeleting`, `ForceDeleted`, `Pruning` | ausgeliefert | Submodul `events::*` pro Modell. `EventResult::cancel(_)` bricht mit einem 400 ab |
| Mutators / Accessors | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | ausgeliefert | [Mutators](eloquent-mutators.md) |
| Casts (22 eingebaute) | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | ausgeliefert | Für eigene Casts `Cast` implementieren |
| Collections | `Collection<M>` mit `pluck`, `filter`, `map`, `each`, `chunk`, `groupBy`, `keyBy`, `sort_by`, `where_`, `first`, `last`, `count`, `is_empty`, `to_array` und den Laravel-Geschwistern; `Deref<Target = Vec<M>>`, sodass alle `Vec`-Idiome weiter funktionieren | ausgeliefert | [Collections](eloquent-collections.md) |
| `modelKeys()` | `Builder::model_keys().await?` (ohne Hydrierung, qualifizierter Schlüssel) und `Collection::model_keys()` | ausgeliefert | Beide liefern `Vec<M::Key>`; das Builder-Terminal projiziert `users.id`, sodass es Joins übersteht |
| API Resources | `#[derive(Resource)]` + `IntoJsonResource` + `JsonApiResponse` + Fieldsets + Includes | ausgeliefert | JSON:API-Form und Resource-Form im Laravel-Stil stehen beide zur Verfügung. `?include=`-Pfade sind auf `max_relationship_depth` begrenzt (Standard 5), passend zu `JsonApiResource::$maxRelationshipDepth`. [API Resources](eloquent-resources.md) |
| Serialisierung | `#[model(hidden = [...], visible = [...], appends = [...])]` | ausgeliefert | Dieselbe Kontrolle darüber, welche Attribute serialisiert werden. [Serialization](eloquent-serialization.md) |
| Factories | `#[derive(Factory)] struct UserFactory` + `UserFactory::new().count(5).create().await?` (oder `UserFactory::times(5).create_many().await?`) | ausgeliefert | `Sequence` für rotierende Werte. [Factories](eloquent-factories.md) |
| Lifecycle: Chunking / Lazy / Cursor | `Builder::chunk(n, \|page\| async { ... })`, `lazy()`, `cursor()` | ausgeliefert | Speicherbegrenzte Iteration über große Tabellen |
| Pessimistisches Locking | `Builder::lock_for_update()`, `shared_lock()` | ausgeliefert | Innerhalb einer Transaktion |
| `refreshForUpdate()` | `model.refresh_for_update().await?` | ausgeliefert | Neuladen per `SELECT ... FOR UPDATE`; unter SQLite ist die Sperrklausel wirkungslos. [Zeilensperren](eloquent.md#row-locking) |
| `inOrderOf(col, values)` | `Builder::in_order_of(col, values)` | ausgeliefert | Sortierung per gebundenem `CASE WHEN`; nicht aufgeführte Werte landen am Ende. Nur im typisierten Builder. [Sortierung](eloquent.md#ordering) |
| `orWhereKey` / `orWhereKeyNot` | `Builder::or_where_key(id)` / `Builder::or_where_key_not(id)` | ausgeliefert | Falten sich als Disjunktion in die vorangehende Klausel; Aliase `or_filter_key` / `or_filter_key_not` |
| `whereJsonContains`-Familie | Verfügbar über die Spaltenausdrücke von SeaORM (treiberbewusst) | ausgeliefert | Die genaue Schreibweise unterscheidet sich je Backend; für die häufigen Fälle werden Helfer ausgeliefert |

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
| `php artisan` | App-eigenes `console`-Binary, gebaut aus `#[command]`-Makros | ausgeliefert | [Konsole](console.md), [CLI - Übersicht](cli.md) |
| `make:controller` / `make:model` usw. | `suprnova make:controller / make:middleware / make:action / make:error / make:inertia / make:migration / make:task` | ausgeliefert | [Code-Generatoren](cli-generators.md) |
| `serve` | `suprnova serve` (Backend + Vite-Dev-Server zusammen) | ausgeliefert | [Serve](cli-serve.md). Überspringt bei einem `--api`-Projekt den Vite-Bereich, statt den Start zu verweigern. |
| `migrate`-Familie | `suprnova migrate / migrate:rollback / migrate:status / migrate:fresh` | ausgeliefert | [CLI Migrationen](cli-migrations.md) |
| `db:seed` | `cargo run --bin console db:seed` (über die App-eigene Konsole) | ausgeliefert | Seeder werden über das `Seeder`-Trait registriert; ein gezielter Lauf gibt RUNNING / DONE samt vergangener Millisekunden aus |
| `schedule:run` / `schedule:work` / `schedule:list` | Dieselben Namen über das App-eigene Konsolen-Binary | ausgeliefert | [Befehlsplanung](cli-scheduling.md) |
| `queue:work` | Derselbe Name über das App-eigene Konsolen-Binary | ausgeliefert | Graceful Shutdown bei SIGTERM/SIGINT |
| `tinker` | Keine REPL | absichtlich nicht | Siehe die Zeile in „Tiefere Einblicke“ |

## Bereitstellung

| Laravel | Suprnova | Status | Hinweise / Link |
|---|---|---|---|
| `php artisan optimize` | `cargo build --release` | abweichend | Ein Binary, kein Opcache-Schritt |
| `php artisan config:cache` | Typisierte Config wird bereits zur Compile-Zeit geprüft | abweichend | Kein Laufzeit-Cache, der invalidiert werden müsste |
| `php artisan route:cache` | Routen werden zur Compile-Zeit per Makro expandiert | abweichend | Der Router wird beim Boot aus bereits typisierten Routen aufgebaut |
| Envoy (SSH-Deploys) | Nutzen Sie einen beliebigen Orchestrator - Docker, systemd, Kubernetes, fly.io, Railway | absichtlich nicht | Das Binary ist das Deploy-Artefakt |
| Forge / Vapor | Nicht unsere Aufgabe - aber die Anleitungen für Railway, DO und Hetzner decken dieselbe Aufgabe ab | abweichend | [Bereitstellung](deployment.md), [Railway](deployment-railway.md), [Digital Ocean](deployment-digital-ocean.md), [Hetzner](deployment-hetzner.md) |
| Wartungsmodus (`php artisan down` / `up`) | `./app down` / `./app up` - Bypass-Secret mit serverseitig geprüftem 12-Stunden-Ablauf, eigene Retry-/Message-/Except-Pfade, Treiber `file` oder `cache` | ausgeliefert | [Bereitstellung](deployment.md) |
| Horizon (Queue-Dashboard) | Noch kein Dashboard | noch nicht | Fehlgeschlagene Jobs lassen sich bis dahin über `cargo run --bin console queue:failed` inspizieren |

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

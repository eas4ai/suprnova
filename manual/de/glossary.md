# Glossar

Suprnova-spezifische Begriffe, einmal definiert. Wenn ein Kapitel ein
Wort verwendet, ohne es zu erklären, lebt die Definition hier. Die
Einträge sind alphabetisch; folgen Sie dem Cross-Link zum Kapitel,
das den Begriff im Kontext verwendet.

Ein paar Konventionen, die Sie beim Lesen des restlichen
Verzeichnisses im Hinterkopf behalten sollten:

- **Trait** bezeichnet einen Rust-Trait - einen Verhaltensvertrag,
  den Sie auf einem Typ implementieren. **Facade** bezeichnet eine
  zero-sized Struktur, deren statische Methoden der Einstiegspunkt
  zu einem Subsystem sind (`Cache`, `Mail`, `Auth`, `Storage`,
  `Bus`, `Notify`, `Vector`, `DB`, `Schedule`, `App`).
- **Treiber** bezeichnet ein austauschbares Backend hinter einer
  Facade oder Registry - `CacheStore`, `QueueDriver`,
  `VectorDriver`, `RateLimiterDriver`, `MailDriver`. Treiber werden
  beim Boot über Umgebungsvariablen ausgewählt und über den
  Container gebunden.
- **Registry** bezeichnet ein prozessglobales Lookup, das zur
  Compile-Zeit über `inventory` oder beim Boot per expliziter
  Registrierung befüllt wird - `ConnectionRegistry`,
  `MiddlewareRegistry`, `InertiaRegistry`, `ChannelRegistry`,
  `VectorRegistry`, `SupervisorRegistry`,
  `PaymentProviderRegistry`, `ScopeRegistry`.

## A

### Accessor

Eine leseseitige Transformation, die auf einem Eloquent-Modell mit
dem `#[accessor]`-Makro deklariert wird. Läuft jedes Mal, wenn die
Eigenschaft gelesen wird, und liefert einen berechneten Wert aus
einer oder mehreren zugrunde liegenden Spalten (zum Beispiel
`full_name` aus `first_name + last_name`). Das Gegenstück zu einem
[Mutator](#mutator). Siehe
[Eloquent - Accessors and mutators](eloquent.md#accessors-and-mutators).

### Aktion

Eine injizierbare Service-Klasse, die ein Stück Geschäftslogik
kapselt - eine einzelne öffentliche Methode, Abhängigkeiten über
das `#[injectable]`-Makro injiziert. Das Suprnova-Analogon zu
Laravels Single-Action-Invokables. Aktionen werden automatisch als
Singletons im Container gebunden und von Handlern, Jobs und
anderen Aktionen aufgelöst. Siehe [Aktionen](actions.md).

### Anwendung

Der fluente Builder in `Application::new()`, der Ihre Config-,
Bootstrap-, Routen- und Migrationsfunktionen registriert und dann
`.run()` aufruft, um das CLI-Subcommand der Binary zu dispatchen
(`serve`, `migrate`, `queue:work` usw.). Eine pro Binary, lebt in
`src/app.rs`. Siehe [Request-Lifecycle](lifecycle.md).

### Atomarer Zähler

Eine Cache-Operation (`Cache::increment`, `Cache::decrement`), die
einen numerischen Wert als einzelnen Roundtrip ändert, ohne
Read-Modify-Write-Races. Beim Redis-Store durch Redis'
`INCR`/`DECR` gestützt, beim In-Memory-Store durch einen gehaltenen
Guard. Siehe [Cache - Atomic Counters](cache.md#atomic-counters).

### Authenticatable

Der Trait, den ein authentifizierter Nutzertyp implementiert
(`get_auth_identifier() -> String`, `get_auth_password()` usw.),
damit Guards und Middleware mit ihm sprechen können, ohne die
konkrete Nutzer-Struktur zu kennen. Siehe
[Authentifizierung](authentication.md).

### Authorizable

Der Trait, der einem Nutzertyp die Policy-Einstiegspunkte (`can`,
`can_any`, `cannot`) gibt, die vom [Gate](#gate) verwendet werden.
Siehe [Autorisierung](authorization.md).

## B

### Backoff-Zeitplan

Die Folge von Verzögerungen, die ein Queue-Worker zwischen
Wiederholungen eines fehlschlagenden Jobs abwartet.
`BackoffSchedule::linear`, `BackoffSchedule::exponential`, oder ein
eigener `Vec<Duration>`. Siehe
[Queues - Backoff schedules](queues.md#backoff-schedules).

### Batch (Queue)

Eine Gruppe von Jobs, die zusammen dispatcht und als Einheit
verfolgt werden - `PendingBatch::new().add(job).add(other).dispatch()`
gibt die persistierte Batch-ID zurück. Nützlich, wenn Sie Arbeit
auffächern und einen Callback laufen lassen wollen, sobald der
gesamte Batch abgeschlossen ist. Siehe
[Queues - Queued batches](queues.md#queued-batches).

### `BelongsTo`

Die Inverse-von-`HasOne`/`HasMany`-Relationsart - das Kind hält den
Fremdschlüssel, der Elternteil steht auf der anderen Seite. Eine
der elf Eloquent-Relationsarten. Siehe
[Eloquent - Relationships](eloquent.md#relationships).

### `BelongsToMany`

Eine Many-to-many-Relationsart, die über ein drittes,
erstklassiges [Pivot](#pivot)-Modell läuft. `BelongsToMany<Local,
Related, Pivot>` - das Pivot ist im Typ benannt, nicht per
String-Konvention synthetisiert. Siehe
[Eloquent - Relationships](eloquent.md#relationships).

### Bootstrap

Die `bootstrap_fn`, die Sie auf dem `Application`-Builder
registrieren und die einmal beim Boot läuft (nach der Config, vor
dem Serving). Wo Sie Services in den [Container](#container)
binden, Observer und Event-Listener registrieren, Standard-Header
konfigurieren und so weiter. Das Suprnova-Analogon zu Laravels
Service-Providern, zusammengefasst in einer einzigen Funktion.
Siehe [Application Bootstrap](bootstrap.md).

### Broadcastable

Der Trait, den ein [Event](#event) implementiert, wenn es an
WebSocket-Abonnenten gepusht werden soll (statt oder zusätzlich zu
lokalen In-Process-Listenern). Die Brücke zwischen dem
Event-Dispatcher und dem [Broadcast Hub](#broadcasthub). Siehe
[Broadcasting](broadcasting.md).

### `BroadcastHub`

Der Trait, der „das Ding, das eine Nachricht an alle
WebSocket-Abonnenten eines Kanals auffächert“ benennt - die
In-Memory-Implementierung (`InMemoryBroadcastHub`) ist der
Standard; die sea-streamer-Implementierung
(`SeaStreamerBroadcastHub`) ist die
Multi-Process-Produktionsbereitstellung. Siehe
[Broadcasting - Multi-Process Fanout](broadcasting.md#multi-process-fanout).

### Builder (Eloquent)

Das fluente Query-Objekt, das `Model::query()` zurückgibt - die
verkettbare Oberfläche, auf der Sie `where`, `order_by`, `with`,
`limit` usw. aufbauen, bevor Sie `.get()`, `.first()` oder
`.paginate(...)` aufrufen. Doppelt benannt: Jede Filtermethode
existiert sowohl unter ihrem Laravel-Namen (`db_where`,
`db_or_where`) als auch unter ihrem Rust-nativen Synonym (`filter`,
`or_filter`). Siehe
[Eloquent - Query builder](eloquent.md#query-builder--dual-api).

### Bus-Command

Eine serialisierbare Struktur, die über `Bus::dispatch(cmd)`
dispatcht wird und zu einem einzigen registrierten `Handler<C>`
routet. Bus-Commands sind für In-Process-Arbeit gedacht, deren
Ergebnis zum Aufrufer zurückblubbern soll - Queue-[Job](#job)s sind
für Arbeit, die persistiert und im Hintergrund wiederholt werden
soll. Siehe [Command Bus](bus.md).

## C

### Cache-Treiber

Das gewählte Backend (`memory` oder `redis`) hinter der
`Cache`-Facade. Wird beim Boot über `CACHE_DRIVER` gewählt und über
den [CacheStore](#cachestore)-Trait freigelegt. Siehe
[Cache](cache.md).

### `CacheStore`

Der Trait, der die Cache-Treiber-SPI definiert - `get`, `put`,
`forget`, `increment` usw. `InMemoryCache` und `RedisCache` sind
die ausgelieferten Implementierungen. Siehe
[Cache - Configuration](cache.md#configuration).

### Cast (Eloquent)

Eine bidirektionale Transformation, die mit `casts!` auf einem
Eloquent-Modell deklariert wird - DB-Spaltentyp ↔ Rust-Typ. 22
eingebaute (`AsBool`, `AsDateTime`, `AsJson`, `AsEncrypted`,
`AsArray` usw.) werden ausgeliefert; ein selbst implementierter
`Cast`-Trait deckt alles andere ab. Siehe
[Eloquent - Casts](eloquent.md#casts).

### Chain (Queue)

Eine Sequenz von [Job](#job)s, die so verkettet sind, dass jeder
nur läuft, wenn der vorherige erfolgreich war. Gebaut mit
`PendingChain::dispatch` / `Queue::chain`. Siehe
[Queues - Queued chains](queues.md#queued-chains).

### Kanal (Broadcasting)

Der Trait, an den ein Event broadcastet - `PublicChannel`,
`PrivateChannel`, oder `PresenceChannel`. Die Kanal-Struktur benennt
sich selbst (`fn name() -> String`) und autorisiert die Verbindung
(`fn authorize(...)`); private Kanäle und Presence-Kanäle fügen
stärkere Trait-Bounds hinzu. Siehe
[Broadcasting - Channels](broadcasting.md#channels).

### Kanal (Benachrichtigung)

Der Trait, der eine [Notification](#notification) an einen
Zustellmechanismus routet - Mail, Datenbank, Broadcast, Web Push.
Eine Benachrichtigung benennt ihre Kanäle in `fn via(...)`; jeder
Kanal löst das Ziel auf und sendet. Zu unterscheiden vom
Broadcasting-Trait desselben Namens. Siehe
[Notifications - Channels](notifications.md#channels).

### Container

Die dreischichtige (Task-lokal → Thread-lokal → Global) Registry,
in der Services über die `App`-Facade gebunden und aufgelöst
werden. Das Suprnova-Analogon zu Laravels Service Container, mit
zusätzlichen Schichten für Pro-Anfrage- und Pro-Test-Isolation.
Siehe [Service Container](container.md).

### Kontext (pro Anfrage)

Die Pro-Anfrage-Bag typisierter Werte, erreichbar von jedem Code im
selben async Task - `Context::set::<T>(value)`,
`Context::get::<T>()`. Überlebt Task-Spawns, wenn Sie ihn explizit
propagieren. Zu unterscheiden vom Feature-Flag-Kontext, der
denselben Namen teilt. Siehe [Kontext](context.md).

### CORS

Cross-Origin Resource Sharing. Die Browser-Sicherheitsregel, die
einen JavaScript-Fetch von Origin A zu Origin B gattert; Suprnova
liefert `CorsMiddleware` aus, um die Response-Header zu setzen, die
signalisieren, welche Cross-Origin-Anfragen erlaubt sind. Siehe
[CORS](cors.md).

### CSRF

Cross-Site Request Forgery. Der Angriff, gegen den sich eine
zustandsbehaftete Session verteidigen muss; Suprnova liefert
`CsrfMiddleware` aus, um bei jeder zustandsändernden Anfrage ein
passendes Token zu verlangen. Siehe [CSRF-Schutz](csrf.md).

## D

### `DB`-Facade

Der modell-lose Einstiegspunkt zur Datenbank - `DB::table(...)`,
`DB::transaction(...)`, `DB::raw(...)`. Für Queries, die nicht in
die Eloquent-Form passen (dynamische Spalten, gejointe Aggregate,
rohes SQL). Siehe
[Eloquent - DB facade](eloquent.md#db-facade--model-less-queries).

### Disk

Ein benanntes Storage-Backend, registriert über die
`Storage`-Facade - `Storage::disk("s3")`, `Storage::disk("local")`.
Jede Disk implementiert [DiskExt](#diskext) und ist über ihren
Registrierungsnamen adressiert. Siehe [Dateisystem](filesystem.md).

### `DiskExt`

Der Trait, den jedes Storage-Backend implementiert - `put`, `get`,
`delete`, `list`, `signed_url` usw. Von `opendal` unter der Haube
gestützt; liefert Adapter für lokales Dateisystem, In-Memory, S3,
Azure Blob und GCS aus. Siehe [Dateisystem](filesystem.md).

## E

### Eloquent

Die gesamte ORM-Schicht - `Model`-Trait, `Builder<M>`, Relationen,
Casts, Scopes, Observers, Events, Soft Deletes, Prunable,
Factories. Der Laravel-Name für das, was andere Ökosysteme ein ORM
nennen; in Suprnova sitzt es oben auf SeaORM (das der Nutzer nicht
sehen sollte). Siehe [Eloquent](eloquent.md).

### Envelope (Queue)

Die Wrapper-Struktur (`Envelope { payload, attempts, max_attempts,
delay, ... }`), das ein Queue-Treiber tatsächlich serialisiert und
speichert. Isoliert die [Job](#job)-Payload von der
Queue-Verrohrung. Siehe [Warteschlange](queues.md).

### Event

Eine klonbare Struktur, die über `EventDispatcher::dispatch(evt)`
dispatcht und an jeden registrierten `Listener<E>` zugestellt wird.
Suprnova liefert den Trait, die Facade (`EventFacade`), den
`Subscriber`-Aggregator und Hooks für
[Queued Listener](#queued-listener) aus. Siehe [Ereignisse](events.md).

### Event-Listener

Siehe [Listener](#listener).

## F

### Facade

Die Namenskonvention für eine zero-sized Struktur, deren
`impl`-Block die öffentliche API eines Subsystems hält - `Cache`,
`Mail`, `Auth`, `Storage`, `Bus`, `Notify`, `Vector`, `DB`,
`Schedule`, `App`. Von Laravel geerbt; in Suprnova wird die
zugrunde liegende Implementierung über den [Container](#container)
statt über PHPs Magic-Call aufgelöst. Siehe
[Service Container](container.md).

### Factory (Eloquent)

Das `#[derive(Factory)]`-Makro und der `Factory`-Trait, die
realistische Testzeilen mit `fake`-getriebenen Defaults erzeugen -
`UserFactory::times(5).create_many().await?`. Das Rust-Gegenstück
zu Laravels Model-Factories. Siehe
[Macros - Factories](macros.md#factories).

### Fail-closed

Eine Treiber-Ausfallpolicy, bei der ein Backend-Ausfall die Anfrage
mit einem 5xx ablehnen lässt - verwendet von Rate Limit, Session
und Idempotenz, wenn „lieber ablehnen als leaken“ gilt. Das
Gegenteil von [Fail-open](#fail-open). Konfiguriert über
`BackendErrorPolicy::FailClosed`. Siehe
[Ratenbegrenzung](rate-limiting.md).

### Fail-open

Eine Treiber-Ausfallpolicy, bei der ein Backend-Ausfall die Anfrage
durchlässt (mit einer protokollierten Warnung), statt sie
abzulehnen - verwendet, wenn Verfügbarkeit wichtiger ist als das
Limit. Konfiguriert über `BackendErrorPolicy::FailOpen`. Siehe
[Ratenbegrenzung](rate-limiting.md).

### Feature Flag

Ein Bool (oder ein typisierter Wert), benannt und gegen den
aktuellen Nutzer/Kontext ausgewertet - `feature!(MyFeature)`.
Gestützt vom `Evaluator`-Trait; liefert einen Datenbank-Evaluator
und einen TTL-gecachten Evaluator obendrauf aus. Siehe
[Feature Flags](feature-flags.md).

### Fillable

Die Compile-Zeit-Allowlist, die festlegt, welche Modellspalten aus
einer Hash-Map nicht vertrauenswürdiger Attribute mass-zugewiesen
werden dürfen - deklariert auf der Modell-Struktur über das
`#[fillable]`-Attribut oder den `Fillable`-Trait. Das Gegenstück zu
`#[guarded]`. Siehe
[Eloquent - Mass assignment](eloquent.md#mass-assignment).

### Dateisystem

Das gesamte Storage-Subsystem - die `Storage`-Facade, registrierte
[Disk](#disk)s, der [DiskExt](#diskext)-Trait,
Cross-Disk-Streaming-Kopie. Siehe [Dateisystem](filesystem.md).

### Form-Request

Eine Struktur, die `FormRequest` implementiert (oder über
`#[request]` abgeleitet wird) und einen Anfrage-Body extrahiert und
validiert, bevor der Handler läuft. Das komponierbare, typsichere
Analogon zu Laravels Form-Request-Klassen. Siehe
[Validierung](validation.md).

### `FrameworkError`

Das einzelne Enum, in das jeder Framework-interne Fehlschlag
konvertiert. Trägt seine eigene `HttpResponse`-Projektion
(`From<FrameworkError> for HttpResponse`), die 5xx-Bodies
sanitisiert und eine Request-ID stempelt. Siehe
[Fehlermodell](error-model.md).

## G

### Gate

Der Autorisierungs-Einstiegspunkt - `Gate::allows("update-post",
user, post)`. Löst gegen registrierte Policies auf (deklariert über
das `#[policy]`-Makro) und bricht per Short-Circuit bei
Erlauben/Verweigern ab. Gibt eine `GateResponse` zurück
(re-exportiert als der Autorisierungs-`Response`). Siehe
[Autorisierung](authorization.md).

### Globaler Scope

Eine Query-Einschränkung, die auf jeden `Model::query()`-Aufruf
angewendet wird, bis sie explizit entfernt wird
(`Builder::without_global_scope`). Implementiert über den
`GlobalScope`-Trait und im Bootstrap registriert. Siehe
[Eloquent - Scopes](eloquent.md#scopes).

### Guard (Auth)

Die benannte Authentifizierungsstrategie, die an eine Anfrage
gebunden ist - `session` (zustandsbehaftet, Cookie-gestützt),
`token` (zustandslos, Bearer-Token). Mehrere Guards koexistieren;
`Auth::guard("api")` wählt einen aus. Siehe
[Authentifizierung](authentication.md).

### Guarded

Die Compile-Zeit-Blocklist, die festlegt, welche Modellspalten
*nicht* mass-zugewiesen werden dürfen. Das Gegenstück zu
[Fillable](#fillable). Siehe
[Eloquent - Mass assignment](eloquent.md#mass-assignment).

## H

### `HasMany`

Eine One-to-many-Relationsart - der Elternteil hält den lokalen
Schlüssel, die Kinder halten den Fremdschlüssel. Eine der elf
Eloquent-Relationsarten. Siehe
[Eloquent - Relationships](eloquent.md#relationships).

### `HasManyThrough`

Eine Relation, die das verwandte Modell erreicht, indem sie durch
ein drittes zwischengeschaltetes Modell hüpft - `Country -> User ->
Post`. Siehe [Eloquent - Relationships](eloquent.md#relationships).

### `HasOne`

Das Einzelzeilen-Geschwister von [HasMany](#hasmany) - der
Elternteil hält den lokalen Schlüssel, das Kind hat den
Fremdschlüssel, gibt höchstens eine Zeile zurück. Siehe
[Eloquent - Relationships](eloquent.md#relationships).

### Hash-Facade

Der Passwort-Hashing-Einstiegspunkt - `hash(password)`,
`verify(password, hash)`. Wählt bcrypt oder argon2 über
`HASH_DRIVER`; `needs_rehash` lässt Sie Nutzer beim Login zwischen
Algorithmen migrieren. Siehe [Hashing](hashing.md).

### Handler

Die async Funktion, die für eine passende Route eine `Response`
zurückgibt - vom `#[handler]`-Makro in die typisierte
Handler-Form des Frameworks verwandelt. Am inneren Rand der
Middleware-Kette komponiert. Siehe [Routing](routing.md),
[Controller](controllers.md).

### `HttpError`

Der Trait, den ein selbst definierter Fehlertyp implementiert, um
festzulegen, wie er als HTTP-Antwort gerendert werden soll -
Status, Body, Header. Spiegelt Laravels `Renderable`-Exceptions.
Siehe [Fehlerbehandlung](errors.md).

### `HttpResponse`

Der konkrete HTTP-Response-Typ, den Handler und Middleware
erzeugen. Umschließt einen Statuscode, Header und einen Body - das,
was tatsächlich an den Client geschrieben wird. Siehe
[Antworten](responses.md).

## I

### Idempotenzschlüssel

Ein clientseitig mitgelieferter Header (`Idempotency-Key`), der
sagt: „Wenn Sie bereits eine Anfrage mit diesem Schlüssel
verarbeitet haben, spielen Sie dieselbe Antwort erneut ab, statt
den Handler erneut laufen zu lassen.“ Erforderlich für
retry-sichere POST/PUT/PATCH/DELETE; Suprnova liefert
`Idempotency`, `Idempotent` und `Replay` aus, um Handler zu
umschließen. Siehe [Idempotenz](idempotency.md).

### Inertia Response

Eine Antwort, die einen typisierten Komponentennamen plus
serialisierte Props zurückgibt statt HTML - die Brücke zwischen
einem Rust-Handler und einer Svelte-/React-/Vue-Seite. Gebaut mit
`Inertia::render(...)` oder dem `#[derive(InertiaProps)]`-Makro
plus `inertia_response!`. Siehe [Frontend](frontend.md),
[Inertia Responses](frontend-inertia-responses.md).

### `InertiaProps`

Das Derive-Makro, das die `Serialize`-Impl plus
TypeScript-Typ-Metadaten für eine als Inertia-Seiten-Props
verwendete Struktur erzeugt. Treibt den Befehl `suprnova
generate-types` an. Siehe
[TypeScript Types](frontend-typescript-types.md).

## J

### Job

Eine serialisierbare Struktur, die den `Job`-Trait implementiert -
hat eine `handle(self)`-Methode, über `Queue::push(job)` (oder
`Queue::push_later(job, when)` für einen verzögerten Dispatch)
eingereiht. Im Storage des Queue-Treibers persistiert und von
einem Worker ausgeführt. Siehe [Warteschlange](queues.md).

### Job-Middleware

Die komponierbaren Wrapper (`WithoutOverlapping`, `RateLimited`,
`ThrottlesExceptions`, `Skip`, `FailOnException`,
`SkipIfBatchCancelled`), die um den `handle`-Aufruf eines Jobs
laufen. Das Queue-Äquivalent zu HTTP-Middleware. Siehe
[Queues - Job middleware](queues.md#job-middleware).

### `JobOutcome`

Das diskriminierte Enum, das der Abschluss eines Jobs erzeugt -
`Completed`, `Failed`, `Released`, `Deleted`, `Skipped` - berichtet
über Job-Lifecycle-Events und den Queue-Metrik-Zähler. Siehe
[Warteschlange](queues.md).

## L

### Lazy Collection

Das Streaming-Gegenstück zu [Collection](#collection-eloquent) -
`Model::query().lazy().await` gibt eine `LazyCollection<M>`
zurück, die Zeilen häppchenweise aus der Datenbank zieht, statt
jede Zeile in den Speicher zu laden. Siehe
[Eloquent - Chunking and lazy iteration](eloquent.md#chunking-and-lazy-iteration).

### Längenbewusster Paginator

Der klassische nummerierte-Seiten-Paginator
(`Builder::paginate(per_page)`), der die Query plus ein `COUNT(*)`
ausführt - kennt die Gesamtzeilenzahl. Siehe
[Eloquent - Pagination](eloquent.md#pagination).

### Listener

Der Trait, den ein Event-Handler implementiert -
`Listener<E>::handle(evt)`. Registriert mit
`EventDispatcher::listen::<E, _>(arc_listener)` oder über den
`Subscriber`-Aggregator. Siehe [Ereignisse](events.md).

### Lock-Guard (Cache)

Das von `Cache::lock(key, ttl).acquire()` zurückgegebene Handle,
das prozessübergreifenden gegenseitigen Ausschluss darstellt -
`LockGuard`. Das Freigeben des Guards gibt die Sperre frei; ihn
fallen zu lassen, verlässt sich auf die TTL. Siehe [Cache](cache.md).

### Lock-Richtlinie

Die projektweite Richtlinie für den Umgang mit
`std::sync::Mutex`-/`std::sync::RwLock`-Poisoning in einem
langlebigen Prozess - zwei sanktionierte Muster (map-to-error oder
recover-in-place); niemals bloßes `.lock().unwrap()`. Siehe
[Lock-Richtlinie](lock-policy.md).

## M

### `Mailable`

Der Trait, den eine Mail-Nachricht implementiert - `subject`,
`to`, `cc`, `bcc`, `view`, Anhänge. Entweder handgeschrieben oder
über das `#[derive(NotificationMailable)]`-Makro abgeleitet;
versendet über `Mail::to(...).send(MyMail).await`. Siehe
[Mail](mail.md).

### Wartungsmodus

Ein Request-Zeit-Umschalter, der die Anwendung für alle außer
einer Allowlist offline nimmt - `maintenance_mode().set(payload)`.
Gestützt von `FileMaintenanceMode` (Standard, eine
Sentinel-Datei) oder `CacheMaintenanceMode` (Cache-gestützt für
Multi-Instanz-Deployments); ausgeliefert von
`MaintenanceMiddleware`. An der Crate-Wurzel re-exportiert.

### Middleware

Ein komponierbarer Wrapper um einen Handler - sieht die Anfrage
vorher, die Antwort nachher, und kann per Short-Circuit abbrechen,
indem er `Err(resp)` zurückgibt. Global, pro Route oder pro Gruppe
registriert; läuft in einer festen Outside-in-Reihenfolge. Siehe
[Middleware](middleware.md).

### Modell

Eine mit `#[suprnova::model]` annotierte Struktur, die eine
Datenbanktabelle benennt. Die Struktur *ist* das SeaORM-`Model`,
nachdem das Makro expandiert - Suprnova umschließt es nicht. Trägt
CRUD über den `Model`-Trait, Query-Konstruktion über
`Model::query()`, Factories, Casts, Scopes, Relationen, Observers.
Siehe [Eloquent](eloquent.md).

### Morph

Kurz für „polymorph“. Eine Morph-Relation lässt eine einzelne
Relation auf einen von mehreren Modelltypen zeigen - `MorphTo`
(einzelner Besitzer mehrerer möglicher Typen),
`MorphMany`/`MorphOne` (die Umkehrung, sammelt gemorphte Kinder),
`MorphToMany`/`MorphedByMany` (Many-to-many über gemorphte Typen).
Das Framework hält eine Laufzeit-[Registry](#registry) von
`MorphTypeEntry`-Mappings zwischen Diskriminator-Strings und
Rust-Typen. Siehe
[Eloquent - Relationships](eloquent.md#relationships).

### Mutator

Eine schreibseitige Transformation, deklariert mit dem
`#[mutator]`-Makro - läuft jedes Mal, wenn die Eigenschaft gesetzt
wird, bevor der Wert auf dem Modell gespeichert wird. Das
Gegenstück zu einem [Accessor](#accessor). Siehe
[Eloquent - Accessors and mutators](eloquent.md#accessors-and-mutators).

## N

### Notifiable

Der Trait, den ein Nutzer (oder jedes Objekt, das
Benachrichtigungen empfangen kann) implementiert -
`route_for(channel)` gibt die Adresse für den benannten Kanal
zurück (Mail-Adresse, Push-Abonnement, Broadcast-Nutzer-ID usw.)
oder `None`, um zu überspringen. Siehe
[Notifications - The Notifiable Trait](notifications.md#the-notifiable-trait).

### Notification

Der Trait, den eine Benachrichtigungsnachricht implementiert -
`channels()` gibt die Liste der Kanalnamen zurück, an die sie
ausgefächert werden soll; jeder Kanal ruft über kanalspezifische
Traits (wie `MailRendering`-/`DatabaseChannel`-Payload-Methoden)
in die Notification zurück, für die kanalspezifische Payload.
Dispatcht über `Notify::send(&user, &notif).await`. Siehe
[Benachrichtigungen](notifications.md).

## O

### Observer

Eine Struktur, die `Observer<M>` implementiert und auf die
Lifecycle-Events eines Eloquent-Modells lauscht - `creating`,
`created`, `updating`, `updated`, `deleting`, `deleted`, `saving`,
`saved`, `retrieved`, `replicating` usw. Registriert über das
`#[suprnova::observer(M)]`-Makro; beim Boot aus dem Inventory
entleert. Siehe
[Eloquent - Observers and lifecycle events](eloquent.md#observers-and-lifecycle-events).

### `OriginPolicy`

Die Durchsetzungswahl der CSRF-Middleware für den `Origin`-Header
bei zustandsändernden Anfragen - `Strict` (muss dem Host
entsprechen), `AllowList`, oder `None`. Siehe [CSRF-Schutz](csrf.md).

## P

### Paginator

Das Ergebnis eines `.paginate(...)`-Aufrufs - eine von drei
Varianten. `LengthAwarePaginator` (nummerierte Seiten mit einem
`COUNT(*)`), `Paginator` (next/prev, keine Gesamtzahl),
`CursorPaginator` (opaker Cursor für stabile Iteration über eine
sich bewegende Ergebnismenge). Alle drei serialisieren in eine
Laravel-förmige JSON-Payload. Siehe
[Eloquent - Pagination](eloquent.md#pagination).

### Panic-Grenze

Der `AssertUnwindSafe(...).catch_unwind()`-Wrapper um die
Middleware-Kette (und um jeden Background-Worker-Handler), der
einen unbehandelten Panic in ein sanitisiertes 500 plus ein
protokolliertes `ErrorOccurred`-Event verwandelt. Ein
Sicherheitsnetz, kein Vertrag - öffentliche APIs sollten weiterhin
`Result` zurückgeben. Siehe
[Request Lifecycle - Panic boundary](lifecycle.md#5-panic-boundary--execute_chain_safely).

### Zahlungs-Provider

Ein Typ, der den `PaymentProvider`-Supertrait implementiert
(= `Checkout`
+ `Subscription` + `CustomerStore` + `WebhookHandler`). Referenzadapter:
`suprnova-payments-stripe` (Gateway, vollständige `Payment`-Impl) und
`suprnova-payments-paddle` (Merchant-of-Record, kein `Payment`).
Siehe [Zahlungen](payments.md),
[Zahlungen - Provider-Leitfaden](payments-provider-guide.md).

### Pivot

Das Zwischenmodell in einer
[BelongsToMany](#belongstomany)-Relation - ein erstklassiges
`#[suprnova::model]` mit eigener Struktur, Casts und Timestamps,
explizit als dritter Typparameter benannt (`BelongsToMany<L, R,
P>`). Suprnova synthetisiert kein implizites Pivot aus einem
Tabellennamen. Siehe
[Eloquent - Relationships](eloquent.md#relationships).

### Presence-Kanal

Eine [Kanal](#kanal-broadcasting)-Variante, bei der der Server
verfolgt, wer gerade abonniert ist, und Join-/Leave-Events mit den
Metadaten jedes Mitglieds ausgibt. Nützlich für
„Wer-ist-online“-Anzeigen. Siehe
[Broadcasting - Presence Channels](broadcasting.md#presence-channels).

### Privater Kanal

Eine [Kanal](#kanal-broadcasting)-Variante, die beim Abonnieren
eine Autorisierung verlangt - `authorize(...)` muss für den
abonnierenden Nutzer `true` zurückgeben. Nützlich für
Pro-Nutzer-Benachrichtigungsströme. Siehe
[Broadcasting - Channels](broadcasting.md#channels).

### Prunable

Der Trait, der ein Soft-deleted (oder abfragbares) Modell für die
Bereinigung durch `model:prune` markiert -
`Prunable::prunable_query()` gibt den Builder für die Zeilen
zurück, die verschwinden sollen. `MassPrunable` löscht in einem
einzigen `DELETE WHERE`; der Standard gibt Pro-Zeile-Deletes aus,
damit Observers feuern. Für die Registry über das
`#[prunable]`-Makro getaggt. Siehe
[Eloquent - Prunable](eloquent.md#prunable).

## Q

### Warteschlange

Das gesamte Hintergrundarbeit-Subsystem - `Queue`-Facade,
[Job](#job)-Trait, [Envelope](#envelope-queue), Treiber (memory,
sync, redis, database, null), Worker, Batches, Chains. Siehe
[Warteschlange](queues.md).

### Queue-Treiber

Ein Typ, der `QueueDriver` implementiert (push, pop, release
usw.) - liefert `MemoryQueueDriver`, `SyncQueueDriver` (inline
ausgeführt), `RedisQueueDriver`, `DatabaseQueueDriver`,
`NullQueueDriver` aus. Beim Boot über `QUEUE_DRIVER` gewählt.
Siehe [Queues - Drivers](queues.md#drivers).

### Queue-Worker

Die langlebige Schleife, die Envelopes vom Queue-Treiber zieht,
Job-Middleware um den Handler laufen lässt und das Ergebnis
berichtet. Bootet über denselben Lifecycle wie der HTTP-Server,
sodass Observers und Listener identisch feuern. Gestartet durch
`cargo run -- queue:work`. Siehe [Warteschlange](queues.md).

### Queued Listener

Ein `Listener<E>`, der beim Aufruf die Event-Payload in die Queue
persistiert und `handle` in einem Background-Worker laufen lässt,
statt in-process. Nützlich, wenn ein Event-Listener I/O macht, das
den Dispatch-Pfad nicht blockieren sollte. Über den
`QueuedListener`-Adapter umschlossen. Siehe [Ereignisse](events.md).

## R

### Rate Limiter

Das gesamte Rate-Limiting-Subsystem - `RateLimiter` (die
Cache-gestützte Facade), `Limit`-Builder, `SlidingWindowConfig`
(Sliding-Window-Treiber), `RateLimitMiddleware` (routen-gebunden),
`ThrottleRequestsMiddleware` (Laravel-benannter Alias),
`BackendErrorPolicy` (Fail-open vs. Fail-closed). Siehe
[Ratenbegrenzung](rate-limiting.md).

### Redirect

Eine spezialisierte [HttpResponse](#httpresponse), die einen
`Location`-Header umschließt - gebaut über `Redirect::to(...)`,
`Redirect::route(...)`, `Redirect::back()`, mit
`.with(...)`-/`.with_input(...)`-Ketten für Flash-Daten. Siehe
[URL-Generierung](urls.md), [Antworten](responses.md).

### Registry

Ein prozessglobales Lookup, entweder zur Compile-Zeit von
`inventory` (`ModelEntry`, `RelationEntry`, `MorphTypeEntry`,
`ObserverEntry`, `PrunerEntry`, `TaskEntry`,
`PaymentProviderEntry`, `CommandEntry`) oder beim Boot per
expliziter Registrierung (`ConnectionRegistry`,
`MiddlewareRegistry`, `InertiaRegistry`, `ChannelRegistry`,
`VectorRegistry`, `SupervisorRegistry`) befüllt. Alle werden
während der Boot-Sequenz entleert oder abgefragt.

### Relation

Der Trait, den jede Relationsart implementiert - `BelongsTo`,
`HasOne`, `HasMany`, `BelongsToMany`, `HasOneThrough`,
`HasManyThrough`, `MorphTo`, `MorphOne`, `MorphMany`,
`MorphToMany`, `MorphedByMany`. Ein Modell deklariert seine
Relationen als Methoden, die eine Relations-Struktur zurückgeben; das
Framework treibt Eager Loading, `with(...)`,
Relations-Existenz-Queries und kaskadierende Touches aus dem
Trait. Siehe [Eloquent - Relationships](eloquent.md#relationships).

### Anfrage

Die typisierte Request-Struktur des Frameworks - umschließt die
zugrunde liegende hyper-Anfrage und legt `req.param("id")`,
`req.json::<T>()`, `req.form_data()`, `req.flash()` usw. frei.
Re-exportiert als `suprnova::Request`. Siehe [Anfragen](requests.md).

### `Response`

Suprnova bindet `http::Response` an `Result<HttpResponse,
HttpResponse>` - beide Arme tragen eine `HttpResponse`.
Handler-Bodies geben `Response` zurück, propagieren
fehlschlagbare Arbeit mit `?`, und die Runtime kollabiert beide
Arme mit `result.unwrap_or_else(|e| e)`. Der
Autorisierungs-Entscheidungstyp wird als `GateResponse`
re-exportiert, um die Kollision zu vermeiden. Siehe
[Antworten](responses.md),
[Request Lifecycle](lifecycle.md#the-response-contract).

### Resource

Zwei nicht verwandte Dinge teilen sich den Namen; beide werden
ausgeliefert.

1. **JSON:API-Resource** - eine `#[derive(Resource)]`-Struktur, die
   ein Modell in die JSON:API-Form mit Sparse Fieldsets und
   Includes serialisiert. Siehe [API Resources](eloquent-resources.md).
2. **Resource-Routing** - ein Routen-Helfer, der ein CRUD-Set
   `index`/`show`/`store`/`update`/`destroy` gegen eine
   `ResourceController`-Impl mountet. Siehe [Routing](routing.md).

### `routes!`-Makro

Das Compile-Zeit-Makro, das eine Routing-DSL
(`get!("/users", users::index)`, `group!`, `middleware!(Auth)`) zu
einer `Router`-Factory-Funktion expandiert. Die einzige
Wahrheitsquelle für Routen einer Anwendung. Siehe
[Routing](routing.md), [Makros](macros.md).

## S

### Lokaler Scope

Ein wiederverwendbares Query-Fragment, deklariert auf einem
Eloquent-Modell mit dem `#[scopes(Model)]`-Makro -
`Post::query().published().recent().get()`. Lokale Scopes sind
standardmäßig aus; sie laufen nur, wenn sie aufgerufen werden. Das
Gegenstück zum [Globalen Scope](#globaler-scope). Siehe
[Eloquent - Scopes](eloquent.md#scopes).

### Seeder

Ein Typ, der den `Seeder`-Trait implementiert und die Datenbank
mit Startdaten befüllt - registriert über `suprnova db:seed`. Oft
von einer [Factory](#factory-eloquent) gestützt. Siehe
[Eloquent](eloquent.md).

### Signierte URL

Eine URL, deren Query-String eine HMAC-Signatur trägt
(`?signature=...&expires=...`), die beweist, dass sie von der
Anwendung erzeugt wurde und nicht manipuliert wurde. Gebaut über
`sign_url(...)` / `sign_route(...)`; verifiziert von Middleware
oder über `verify_signature(...)`. Siehe
[URL Generation - Signed URLs](urls.md#signed-urls).

### Soft Deletes

Das Muster, bei dem das Löschen einer Modellzeile einen
`deleted_at`-Timestamp setzt, statt ein `DELETE` auszuführen. Pro
Modell über `soft_deletes = true` auf dem
`#[suprnova::model]`-Attribut opt-in; `Model::query()` filtert
verworfene Zeilen automatisch heraus; `with_trashed()` und
`only_trashed()` holen sie wieder zurück. Siehe
[Eloquent - Deleting and soft deletes](eloquent.md#deleting-and-soft-deletes).

### `Storage`-Facade

Der Einstiegspunkt zum Dateisystem-Subsystem -
`Storage::disk("s3")`, `Storage::disk("local")` - gibt eine
[DiskExt](#diskext)-Implementierung zurück. Siehe
[Dateisystem](filesystem.md).

### Subscriber

Ein Aggregator, der viele Listener in einem Aufruf registriert -
implementiert `Subscriber::subscribe(dispatcher)` und wird über
`EventDispatcher::subscribe(subscriber)` registriert. Siehe
[Ereignisse](events.md).

### Supervisor

Der Trait, den ein langlebiger Background-Akteur implementiert
(`Supervisor::run`), um unter der `SupervisorRegistry` zu leben.
Die Registry fängt Panics in der Run-Schleife ab, wendet eine
`RestartPolicy` an und spawnt neu. Das Rust-Äquivalent zu Erlangs
`gen_server`-Supervisor-Muster. Siehe [Supervisoren](supervisors.md).

## T

### Task

Eine Struktur, die den `Task`-Trait implementiert - deklariert einen
Cron-Ausdruck oder eine übergeordnete Frequenz (`daily()`,
`every_minute()`) und läuft auf dem Scheduler. Zur Compile-Zeit
über das `TaskEntry`-Inventory entdeckt. Siehe
[Task-Planung](scheduling.md).

### Terminable Middleware

Middleware, die einen Hook registriert, der läuft, *nachdem* die
Antwort an den Client geschrieben wurde - implementiert über den
`Terminable`-Trait, in einen `TerminationSnapshot` erfasst und von
`dispatch_termination` dispatcht. Nützlich für Logging,
Metrik-Flushes, Post-Flight-Auditing. Siehe
[Middleware - Terminable middleware](middleware.md#terminable-middleware-post-response-hooks).

### Through (Relation)

Eine Relation, die durch ein drittes zwischengeschaltetes Modell
hüpft - [HasManyThrough](#hasmanythrough) und `HasOneThrough`.
Siehe [Eloquent - Relationships](eloquent.md#relationships).

### Timeout

Die Middleware, die die Wall-Clock-Zeit einer einzelnen Anfrage
begrenzt und ein 504 zurückgibt, wenn die Grenze überschritten
wird - `TimeoutMiddleware`. Zu unterscheiden von
Queue-Worker-Timeouts (`TimeoutExceeded` auf der Queue-Seite) und
von HTTP-Client-Timeouts. Siehe [Timeout](timeout.md).

### `TypedCommand`

Der konsolenseitige Trait - implementiert von
`#[derive(Command)]`-Strukturen -, der einem Konsolenbefehl
typisierte Argumente (über `clap`) und eine async
`handle(self)`-Methode gibt. Zur Compile-Zeit in das
`CommandEntry`-Inventory registriert. Siehe [Konsole](console.md).

## U

### `UserId`

Der opake String-Identifier, den `Auth::id()` zurückgibt. Die Guard-/Provider-Pfade des Frameworks tragen den stabilen Schlüssel, den der konfigurierte `UserProvider` verwendet; bei `EloquentUserProvider<User>` ist dies normalerweise der als String dargestellte Primärschlüssel. Magnetar-Fassaden stellen einen Newtype `UserId` bereit, binden dessen Wert aber wieder an die kanonische Benutzer-ID der Anwendung, bevor sie Framework-Session-Zustand schreiben. Eine stringförmige Request-Grenze ermöglicht numerischen IDs, UUIDs und providerunabhängigen opaken IDs, dieselben Middleware- und Event-Verträge zu verwenden. Siehe [Authentifizierung](authentication.md).

## V

### VAPID

Voluntary Application Server Identification - die
IETF-Spezifikation zur Identifizierung eines
Web-Push-Absenders. Suprnova liefert `VapidKey`, `VapidSigner`,
`VapidClaims` und den `WebPushClient` aus, der jede Push-Anfrage
signiert. Siehe [Web Push](web-push.md).

### `Vector`-Facade

Der Einstiegspunkt zum Vector-Search-Subsystem -
`Vector::driver("qdrant").await?.upsert(...)`. Gestützt von
`VectorDriver`-Implementierungen: In-Memory, Qdrant, Pinecone
(Feature-gated), MariaDB nativ. Siehe [Vector-Suche](vector.md).

### `VectorDriver`

Der Trait, den jedes Vector-Backend implementiert - `upsert`,
`search`, `delete`, `count`. Erlaubt dem Framework, mehrere
Vektor-DBs zu unterstützen, ohne eine zu erzwingen. Siehe
[Vector-Suche](vector.md).

## W

### Web Push

Das Web-Plattform-Push-Benachrichtigungsprotokoll - verschlüsselte
Payloads, zugestellt über den Push-Service des User-Agents.
Suprnova liefert `WebPushClient` (VAPID-Signer,
Retry-After-Parsing, 8-KiB-Ablehnungs-Cap) und `WebPushChannel`
für die [Notification](#notification)-Zustellung aus. Siehe
[Web Push](web-push.md).

### Webhook

Eine HTTP-Anfrage, die ein Drittanbieter (Zahlungsanbieter,
Identity-Provider, …) an Ihre Anwendung sendet, um ein Ereignis zu
melden. Suprnova behandelt jeden Webhook standardmäßig als
idempotent - Provider-Adapter implementieren
`WebhookHandler::verify(...)` und speichern die Event-ID des
Providers in einem `UNIQUE`-Constraint, das Replays ablehnt.
Siehe [Payments - Webhook Handling](payments.md#webhook-handling),
[Idempotenz](idempotency.md).

### Workflow

Ein lang laufendes, zustandsbehaftetes Stück Hintergrundarbeit,
zusammengesetzt aus typisierten Schritten - `#[workflow]`- und
`#[workflow_step]`-Makros. Der Rückgabewert jedes Schritts wird
persistiert, sodass ein Worker-Neustart mitten im Workflow beim
letzten abgeschlossenen Schritt fortsetzt. Suprnovas Antwort auf
mehrschrittige Hintergrundprozesse, die nicht in einen einzelnen
[Job](#job) passen. Siehe [Workflows](workflows.md).

### `WsConfig`

Die Pro-Route-WebSocket-Konfiguration - Payload-Größen-Caps
(Standard 1 MiB Text / 64 KiB Binär), maximale Frame-Größe,
Ping-Intervall, Idle-Timeout, Origin-Policy. Verwendet von
`ws!()`-Routen. Siehe [WebSockets](websockets.md).

### `WsSocket`

Das typisierte WebSocket-Handle des Frameworks, das an einen
`ws!()`-Handler übergeben wird. Über `WsSocket::split()` in eine
`Sink`- (Senden) und eine `Stream`-Hälfte (Empfangen) geteilt;
Pings/Pongs werden von einem Heartbeat-Task mit einem
`AbortHandle` verwaltet, sodass ein fallengelassener Handler immer
sauber abgebaut wird. Siehe [WebSockets](websockets.md).

## Nächste Schritte

- [Laravel Parity Map](parity.md) - Feature-für-Feature-Vergleich
  mit Laravel 13
- [Umgebungsvariablen](env-vars.md) - jede `env!`, die das
  Framework liest
- [Dokumentations-Index](documentation.md) - die Kapitelkarte

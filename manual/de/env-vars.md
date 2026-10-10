# Umgebungsvariablen

Dies ist die geprüfte Liste jeder Umgebungsvariable, die das
Suprnova-Framework zur Laufzeit liest, gruppiert nach dem Subsystem,
das sie konsultiert. Jeder Eintrag wurde gegen den Framework-Quellcode
validiert - Standardwerte, Typen und Verhalten spiegeln das wider, was
der Code tatsächlich tut, nicht das, was die Starter-`.env` zufällig
mitliefert.

Die Liste deckt außerdem die Variablen ab, die die `suprnova`-CLI-
Binary liest (Dev-Server, SSR-Worker), da diese in der Starter-`.env`
auftauchen und Leser sie hier erwarten.

Siehe [Konfiguration](configuration.md) für die Laderegeln
(`.env` → `.env.<environment>` → Prozessumgebung), die
`env*`-Helfer (`env`, `env_required`, `env_optional`) und das
typisierte `Config::*`-Registrierungsmuster.

## Konventionen

- **Standard** - der Wert, den das Framework verwendet, wenn die
  Variable nicht gesetzt ist. `none` bedeutet, es gibt keinen
  Standard; das Framework schlägt beim Boot fehl, fällt auf einen
  Feature-Standard zurück (z. B. den `Memory`-Treiber) oder behandelt
  den Wert als `None`.
- **Typ** - der Rust-Typ, in den die Variable geparst wird. `bool`-
  Werte akzeptieren `true`/`false`/`1`/`0`/`yes`/`no`/`on`/`off`
  (ohne Unterscheidung von Groß-/Kleinschreibung). Werte außerhalb
  des gültigen Bereichs oder nicht parsbare Werte werden für
  typisierte Framework-Regler geklemmt (Workflow), protokolliert
  (`warn!`) und dann auf den Standardwert zurückgesetzt (nachsichtiges
  `env()` / `env_optional()`), oder sie lassen den Boot fehlschlagen
  (strenges `try_from_env`).
- **Erforderlich** - `boot` bedeutet, das Framework verweigert den
  Start ohne sie in den genannten Umgebungen. `driver` bedeutet, sie
  ist nur erforderlich, wenn der übergeordnete Treiber ausgewählt ist
  (z. B. ist `MAIL_SES_REGION` irrelevant, solange nicht
  `MAIL_DRIVER=ses` gilt). Alles andere ist optional.

Führt eine Starter-`.env` einen Schlüssel, den das Framework nie
liest (`MAIL_FROM_ADDRESS`, `FILESYSTEM_DISK`), wird das am Ende
dieses Kapitels benannt.

## Anwendung

Die `APP_*`-Familie ist die Identität und die Krypto-Wurzel des
Frameworks. Das sind die Variablen, die jede Suprnova-App setzt; der
Rest der Datei wird relevant, sobald Sie Subsysteme aktivieren.

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `APP_NAME` | `"Suprnova Application"` | `String` | Anwendungsname. Verwendet als TOTP-Issuer (2FA), als Realm für `WWW-Authenticate` bei HTTP Basic, als Branding im Mail-Betreff und in Feldern des strukturierten Logs. |
| `APP_ENV` | `local` | `String` | Steuert `Environment::detect()` und den `.env.<suffix>`-Lookup. Erkannte Aliase (ohne Unterscheidung von Groß-/Kleinschreibung): `local`, `development`/`dev`, `staging`/`stage`/`stg`, `production`/`prod`, `testing`/`test`. Jeder andere Wert wird als `Environment::Custom(...)` mit der ursprünglichen Schreibweise erhalten. |
| `APP_DEBUG` | umgebungsbewusst (siehe Erforderlich) | `bool` | Ausführliche Fehlerseiten + zusätzliche Logs. Der Standard ist `true` in `local`/`development`/`testing` und `false` überall sonst (einschließlich `staging`, `production` und jeder nicht erkannten benutzerdefinierten Umgebung). Ein expliziter Wert gewinnt immer; ein nicht parsbarer Wert fällt mit einer `warn!`-Meldung auf den umgebungsbewussten Standard zurück. Die strenge `try_from_env`-Variante bricht den Boot bei einem Parse-Fehler ab. |
| `APP_URL` | `"http://localhost:8765"` (AppConfig) / `"http://localhost"` (URL-Fallback) | `String` | Basis-URL für die Generierung absoluter URLs, signierte URLs und Inertia-Redirects. Abschließende Schrägstriche werden beim Lesen entfernt. |
| `APP_KEY` | keine - in Nicht-Dev erforderlich | `String` (Base64-URL ohne Padding, 32 Bytes) | AES-256-GCM-Schlüssel für `Crypt`, verschlüsselte Sessions, Paginierungs-Cursor, signierte URLs und jeden anderen Encrypt-at-Rest-Pfad. Der Boot **schlägt geschlossen fehl**, wenn der Schlüssel außerhalb von `local`/`development`/`testing` fehlt oder fehlerhaft ist. Generieren Sie ihn mit `suprnova key:generate`. |
| `APP_KEY_PREVIOUS` | keine | `String` (kommagetrennte Base64-Schlüssel, max. 8) | Kommagetrennte frühere Schlüssel, verwendet während der Rotation. `Crypt::decrypt` versucht zuerst den aktuellen `APP_KEY`, dann jeden Eintrag der Reihe nach. Hartes Limit von 8 Einträgen - `crypto::MAX_PREVIOUS_KEYS`. Ein halb rotierter Eintrag, der sich nicht dekodieren lässt, bricht den Boot ab. Siehe [Verschlüsselung](encryption.md#key-rotation). |
| `APP_PREVIOUS_KEYS` | keine | `String` (Alias von `APP_KEY_PREVIOUS`) | Laravel-kompatibler Alias, akzeptiert, damit eine Laravel-`.env`, die in ein Suprnova-Deployment fällt, Legacy-Daten weiterhin graceful entschlüsselt. Sind beide mit unterschiedlichen Werten gesetzt, gewinnt `APP_KEY_PREVIOUS` mit einer `warn!`-Meldung, die das Duplikat sichtbar macht; identische Werte werden stillschweigend akzeptiert. |
| `APP_BASE_PATH` | aktuelles Arbeitsverzeichnis | `Path` | Wurzelverzeichnis, das der Pfad-Resolver für `config/`, `database/`, `public/`, `storage/`, `resources/`, `lang/` verwendet. Nützlich, wenn die Binary aus einem anderen Arbeitsverzeichnis als dem Projekt-Root gestartet wird (z. B. eine systemd-Unit, deren `WorkingDirectory=` nicht auf das Projekt zeigt). Fällt auf das aktuelle Arbeitsverzeichnis zurück, dann auf `.`, wenn dieses nicht verfügbar ist. |
| `APP_TRUSTED_PROXIES` | keine - leere Allowlist | `String` (kommagetrennte IPs) | TCP-Peer-Adressen, deren `X-Forwarded-*`-/`X-Real-IP`-Header `Request::ip()` und die Host-/Scheme-/Port-Accessoren glauben dürfen. **Standardmäßig leer, sodass Proxy-Header ignoriert werden und der TCP-Peer immer gewinnt** - siehe den Hinweis unten, bevor Sie hinter einem Proxy deployen. Ein nicht parsbarer Eintrag lässt den Boot fehlschlagen (`try_from_env`). |
| `AUTH_GUARD` | `"web"` | `String` | Name des Standard-Guards, den `Auth::*` liest. Spiegelt Laravel - nur der Standard ist über die Umgebung wählbar; benannte Guards leben im Code über `AuthConfig::guard(name, …)`. |

Zwei weitere `APP_*`-Variablen - `APP_LOCALE` und
`APP_FALLBACK_LOCALE` - werden vom Lokalisierungs-Subsystem gelesen,
nicht von `AppConfig`, und stehen deshalb unten unter
**Lokalisierung**.

### Hinter einem Reverse-Proxy `APP_TRUSTED_PROXIES` setzen

Proxy-Header zu ignorieren ist der sichere Standard -
`X-Forwarded-For` wird vom Aufrufer mitgeliefert, und ihm
bedingungslos zu vertrauen lässt jeden eine beliebige Adresse
behaupten. Doch sobald ein terminierender Proxy vor Ihnen steht
(nginx, Traefik, ein ALB, Cloudflare), ist der TCP-Peer bei jeder
Anfrage *der Proxy*, und dies ungesetzt zu lassen kostet Sie nicht
nur die Adresse des Clients:

- **Pro-IP-Rate-Limits fallen in einen einzigen Bucket zusammen.**
  Der Standard-Schlüssel von `ThrottleRequestsMiddleware` ist
  `request.ip()`, sodass `ThrottleRequestsMiddleware::with(20, 1,
  "login")` nicht mehr "20 Login-Versuche pro Client und Minute"
  bedeutet, sondern 20 *insgesamt, über alle hinweg*. Das ist
  zugleich schwächer (kein Pro-Angreifer-Budget) und aktiv
  gefährlich: Ein einzelner Aufrufer kann das Kontingent aufbrauchen
  und jeden legitimen Nutzer aus dem Login-Formular aussperren. Siehe
  [Ratenbegrenzung](rate-limiting.md).
- `Request::host()`, `scheme()` und `port()` fallen auf die
  Verbindung zurück statt auf `X-Forwarded-Host` / `-Proto` /
  `-Port`, sodass generierte absolute URLs die interne Adresse und
  das interne Schema statt der öffentlichen nennen können.

Listen Sie die Adressen auf, von denen aus die Proxy-Hops Sie
erreichen - nicht die des Clients:

```bash
APP_TRUSTED_PROXIES=10.0.0.5,10.0.0.6
```

Nichts erkennt das für Sie: Eine App hinter einem Proxy mit
ungesetzter Variable sieht gesund aus, bedient korrekt und
rate-limitet stillschweigend alle wie einen einzigen Nutzer.

### Pflicht-Matrix für `APP_KEY`

| Umgebung | `APP_KEY` beim Boot erforderlich |
|---|---|
| `local` | nein (erzeugt bei Fehlen einen flüchtigen Schlüssel) |
| `development` | nein |
| `testing` | nein |
| `staging` | ja - der Boot beendet sich mit Non-Zero und einer Behebungsmeldung |
| `production` | ja |
| `Custom(...)` | ja - alles, was nicht auf der Safe-List steht, wird für diese Prüfung wie Produktion behandelt |

## Server

Der HTTP-Listener und die Grenzen für Request-Bodys.

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `SERVER_HOST` | `"127.0.0.1"` | `String` | Bind-Adresse. Auf `0.0.0.0` setzen, um außerhalb des Loopback-Interfaces zu exponieren (z. B. in Containern). |
| `SERVER_PORT` | `8765` | `u16` | Bind-Port. Der nachsichtige Parse protokolliert eine Warnung und fällt auf den Standard zurück; das strenge `try_from_env` bricht den Boot bei einem Tippfehler ab. |
| `SERVER_MAX_BODY_SIZE` | `8388608` (8 MiB) | `usize` (Bytes) | Prozessweite Obergrenze für die Größe des Request-Bodys. Pro-`FormRequest::max_body_bytes`-Overrides gelten weiterhin auf einzelnen Endpunkten. Der konfigurierte Wert wird während `Server::from_config` in die globale Obergrenze verdrahtet. |
| `SERVER_MAX_CONNECTIONS` | nicht gesetzt (unbegrenzt) | `usize` | Obergrenze für gleichzeitig aktive TCP-Verbindungen. Nicht gesetzt bedeutet keine Obergrenze. Ein Wert, der null oder nicht parsbar ist, fällt mit einer Warnung auf ein endliches `10000` zurück, statt stillschweigend auf unbegrenzt zurückzufallen - ein verpfuschtes Limit ist immer noch die Anfrage nach einem Limit. |
| `SERVER_HEADER_READ_TIMEOUT` | `30` | `u64` (Sekunden) | Deadline, um den vollständigen Head einer Anfrage zu lesen. Die Slowloris-Abwehr. Null wird als ungültig behandelt, nicht als "deaktivieren", und fällt auf den Standard zurück. Gilt nicht für bereits etablierte WebSocket-/SSE-Verbindungen. |
| `SERVER_HEALTH_READINESS_TOKEN` | nicht gesetzt (Readiness ist öffentlich) | `String` | Gemeinsames Secret, das nötig ist, um `/_suprnova/health/ready` und `/_suprnova/health?db=true` zu erreichen, gesendet als `X-Suprnova-Health-Token`. Ohne es antworten diese Pfade mit 404, nicht unterscheidbar von jedem ungerouteten Pfad; Liveness bleibt öffentlich. Siehe [Bereitstellung](deployment.md#health-check). |

## Datenbank

Connection-URL und Feinabstimmung des sqlx-Pools. `DATABASE_URL` ist für
jeden Subcommand erforderlich, der die Datenbank berührt (`migrate*`,
`db:sync`, `db:seed`, `queue:work` mit `QUEUE_DRIVER=database`,
`workflow:work`, der Session-DB-Speicher) und für `serve`, wenn die App
Migrationen registriert hat. Die fünf Stellschrauben für die Lebendigkeit
sind Ihr Mittel gegen ein Netzwerk, das untätige Verbindungen verwirft -
siehe [Pool-Lebendigkeit](database.md#pool-liveness).

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `DATABASE_URL` | keiner - erforderlich, wenn Migrationen existieren | `String` | Connection-URL. Das Schema wählt den Treiber: `sqlite://path`, `postgres://...` / `postgresql://...`, `mysql://...`, `mariadb://...`. Für SQLite-Pfade legt das Framework das übergeordnete Verzeichnis automatisch an. `serve` überspringt die Datenbankverbindung vollständig, wenn der konfigurierte `Migrator` keine Migrationen hat. |
| `DB_MAX_CONNECTIONS` | `10` | `u32` | Obergrenze des sqlx-Pools. |
| `DB_MIN_CONNECTIONS` | `1` | `u32` | Untergrenze des sqlx-Pools (wird warm gehalten). |
| `DB_CONNECT_TIMEOUT` | `30` (Sekunden) | `u32` | Wie lange sqlx auf eine erste Verbindung wartet, bevor es einen Fehler meldet. |
| `DB_LOGGING` | `false` | `bool` | Wenn true, protokolliert sqlx jedes Statement (in Produktion sparsam einsetzen - geschwätzig). |
| `DB_IDLE_TIMEOUT` | nicht gesetzt (sqlx nutzt 600 Sekunden) | `u64` (Sekunden) | Wie lange eine Pool-Verbindung untätig liegen darf, bevor der Pool sie schließt. `0` schaltet das Ernten untätiger Verbindungen ab. |
| `DB_MAX_LIFETIME` | nicht gesetzt (sqlx nutzt 1800 Sekunden) | `u64` (Sekunden) | Wie lange eine Pool-Verbindung leben darf, bevor der Pool sie recycelt. `0` schaltet das Recyceln nach Lebensdauer ab. |
| `DB_ACQUIRE_TIMEOUT` | nicht gesetzt (fällt auf `DB_CONNECT_TIMEOUT` zurück) | `u64` (Sekunden) | Wie lange ein Aufrufer auf eine freie Pool-Verbindung wartet. Überschreibt `DB_CONNECT_TIMEOUT` für die Wartezeit beim Checkout; setzen Sie das eine oder das andere, nicht beides. Null wird beim Boot abgelehnt. |
| `DB_TEST_BEFORE_ACQUIRE` | `true` | `bool` | Eine Pool-Verbindung vor der Herausgabe anpingen. Lassen Sie es an, sofern Sie nicht den Round-Trip pro Checkout gemessen haben und `DB_PING_AFTER_IDLE` nicht ausreicht. |
| `DB_PING_AFTER_IDLE` | nicht gesetzt | `u64` (Sekunden) | Eine Pool-Verbindung erst anpingen, wenn sie so lange untätig war. Es zu setzen schaltet `DB_TEST_BEFORE_ACQUIRE` ab, sodass heiße Verbindungen unangetastet herausgegeben werden. |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT` | `false` | `bool` | Wenn true, wird eine fehlschlagende Auto-Migration beim `serve`-Boot protokolliert, bricht den Start aber nicht ab. Der Standard schließt bei Fehlern: Der Boot endet mit einem Wert ungleich null, statt gegen ein teilweise migriertes Schema zu starten. Übergeben Sie `--no-migrate`, um die Auto-Migration ganz zu überspringen. |

## Sitzungen

Cookie-Attribute und Lebensdauer für das Session-Subsystem. Beachten
Sie, dass `SESSION_SECURE` standardmäßig **`true`** ist -
produktionssicher von Haus aus; schalten Sie es nur für lokale
HTTP-Entwicklung aus.

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `SESSION_LIFETIME` | `120` (Minuten) | `u64` | Session-Lebensdauer in Minuten. Geparst über `env_optional`; fällt stillschweigend zurück, wenn nicht parsbar. |
| `SESSION_TOUCH_INTERVAL` | `300` (Sekunden) | `u64` | Minimale Persistenz-Kadenz für den gleitenden Ablauf. Die Laufzeit erzwingt eine Obergrenze bei der Hälfte der Session-Lebensdauer. |
| `SESSION_GC_INTERVAL` | `3600` (Sekunden) | `u64` | Kadenz für den überwachten Collector abgelaufener Sessions, installiert von `SessionMiddleware::install`. |
| `SESSION_COOKIE` | `"suprnova_session"` | `String` | Name des Session-Cookies. |
| `SESSION_PATH` | `"/"` | `String` | Cookie-Attribut `Path=`. |
| `SESSION_DOMAIN` | nicht gesetzt | `String` | Cookie-Attribut `Domain=`. Ungesetzt lassen für Host-only-Cookies (der sicherere Standard für die meisten Apps). |
| `SESSION_SECURE` | `true` | `bool` | Cookie-Attribut `Secure`. Standardmäßig `true`; nur in lokaler HTTP-Entwicklung auf `false` setzen. `cookie_http_only` ist immer `true` und nicht über die Umgebung konfigurierbar. |
| `SESSION_SAME_SITE` | `"Lax"` | `String` | Attribut `SameSite`. Akzeptiert `Strict`, `Lax`, `None` (ohne Unterscheidung von Groß-/Kleinschreibung). |
| `SESSION_COOKIE_PREFIX` | nicht gesetzt | `String` (`__Host-` / `__Secure-`) | Präfix, das auf die Wire-Namen von Session und Remember-me angewendet wird. `Config::init` validiert den Wert und seine Einschränkungen durch `SESSION_DOMAIN` / `SESSION_PATH` beim Booten; ungültige Kombinationen schlagen fehl, bevor die Anwendung Requests bedient. |
| `SESSION_PARTITIONED` | `false` | `bool` | Gibt das Cookie-Attribut `Partitioned` / CHIPS für third-party-isolierte Cookies aus. |
| `SESSION_EXPIRE_ON_CLOSE` | `false` | `bool` | Wenn wahr, wird `Max-Age` weggelassen, sodass der Browser das Cookie beim Schließen löscht (Session-Cookie-Semantik). |
| `SESSION_CONNECTION` | nicht gesetzt | `String` | Benannte DB-Connection für den Session-Store. Ungesetzt bedeutet die Standard-Connection. |
| `REMEMBER_LIFETIME` | `43200` (30 Tage, in Minuten) | `u64` | Lebensdauer des "Remember me"-Cookies/Tokens in Minuten. |

## Lokalisierung

Die drei `APP_*`-Variablen, die das Lokalisierungs-Subsystem liest.
Alles andere daran - die Erkennungskette, der Session-Schlüssel und
der Cookie-Name, die es konsultiert, Unicode-Isolationsmarken - ist
Code-Ebenen-Konfiguration auf `LocalizationConfig`, nicht Env. Siehe
[Lokalisierung](localization.md).

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `APP_LOCALE` | `"en"` | `String` (BCP-47) | Locale, das verwendet wird, wenn die Erkennungskette (Session → Cookie → `Accept-Language`) nichts findet. Auch das Locale, aus dem `suprnova generate-types` Message-Keys für `lang-keys.ts` extrahiert. Ein Wert, der kein gültiger BCP-47-Identifier ist, lässt den Boot fehlschlagen, statt stillschweigend zu defaulten. |
| `APP_FALLBACK_LOCALE` | `"en"` | `String` (BCP-47) | Locale, das konsultiert wird, wenn ein Schlüssel im Katalog des aktuellen Locale fehlt. Fehlt ein Schlüssel in beiden, wird der Schlüssel selbst plus eine einmalige `warn!`-Meldung ausgegeben; `Lang::try_get` liefert statt dessen `Err`. Derselbe strenge Parse wie bei `APP_LOCALE`. |
| `APP_LOCALE_PARENTS` | keine - leere Map | `String` (kommagetrennte `child=parent`-Paare, beidseitig BCP-47) | Pro-Locale-Fallback-Eltern, die vor `APP_FALLBACK_LOCALE` konsultiert werden, z. B. `APP_LOCALE_PARENTS=pt-PT=pt-BR,en-AU=en-GB`. Die Fallback-Kette von `Lang` läuft diese transitiv ab, und `FluentTranslator` flacht die konfigurierte Elternkette jedes Locale in seinen ausgelieferten Katalog. Ein fehlerhaftes Paar, ein ungültiges Locale, ein Kind, das mehr als einmal genannt wird, oder ein Zyklus (einschließlich eines Locale, das sich selbst als eigenes Elternteil nennt) lässt den Boot fehlschlagen, statt zur Request-Zeit zu degradieren. Siehe [Fallback-Ketten](localization.md#fallback-chains). |

Die Kataloge selbst sind Dateien, nicht Env: `lang/<locale>/*.ftl`
unter `APP_BASE_PATH`. Ein fehlendes `lang/`-Verzeichnis ist kein
Fehler - die App bootet mit dem eingebetteten englischen
Validierungskatalog des Frameworks.

## Cache

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `CACHE_DRIVER` | `memory` | `String` (`memory`/`in-memory`/`inmemory`, `redis`) | Wählt das Bootstrap-Ziel. Memory hält alles prozessintern; Redis braucht `REDIS_URL` und lässt den Boot scheitern, wenn es nicht erreichbar ist. Unbekannte Werte lassen den Boot mit einer klaren Fehlermeldung scheitern. |
| `REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | Redis-Connection-URL (wird nur bei `CACHE_DRIVER=redis` herangezogen). |
| `REDIS_PREFIX` | `"suprnova_cache:"` | `String` | Schlüssel-Präfix für Cache-Einträge (Kollisionsvermeidung bei geteiltem Redis). |
| `CACHE_DEFAULT_TTL` | `3600` (Sekunden) | `u64` | Standard-TTL in Sekunden. `0` bedeutet „kein Ablauf“. Gilt für `Cache::put(None)` / `Cache::tags_put(None)`; `Cache::forever` und `Cache::remember_forever` umgehen sie immer. |
| `REDIS_COMMAND_RETRIES` | `0` | `u32` | Zusätzliche Wiederholungen für lesende Redis-Befehle, über die eine hinaus, die jedes Lesen ohnehin bekommt. Gilt für die Cache-, Queue- und Rate-Limit-Treiber. Schreibzugriffe wiederholen bei keinem Wert. Rechnen Sie in Sekunden: Eine Wiederholung auf einer abgerissenen Verbindung wartet auf den Reconnect und kostet damit das gesamte Verbindungs- und Antwortbudget des Treibers - bis zu 3 Verbindungsversuche im Abstand von höchstens 500 ms, jeder auf 2 s gedeckelt, plus 5 s Antwort-Timeout beim Cache-Treiber; bis zu 6 Verbindungsversuche mit ungedeckelter exponentieller Verzögerung, jeder auf 1 s gedeckelt, plus 500 ms Antwort-Timeout bei den Queue- und Rate-Limit-Treibern. Die Deckelung auf `10` begrenzt Versuche, nicht Sekunden: Bei dieser Einstellung macht ein Lesen 12 Versuche. Ein Timeout gilt ebenfalls als transient, während einer Blockade setzt jedes umschlossene Lesen also bis zu so viele Befehle ab. Ein nicht parsbarer Wert fällt auf `0` zurück. |

## Warteschlange

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `QUEUE_DRIVER` | `memory` | `String` (`memory`, `redis`, `database`, `failover`) | Aktives Queue-Backend. Unbekannte Werte protokollieren ein `warn!` und fallen auf Memory zurück. `failover` umschließt eine geordnete Liste der übrigen - siehe `QUEUE_FAILOVER_CONNECTIONS`. |
| `QUEUE_FAILOVER_CONNECTIONS` | - | `String` (kommagetrennt, z. B. `redis,database`) | Nach Priorität geordnete Connection-Liste für `QUEUE_DRIVER=failover`. Erforderlich, wenn dieser Treiber gewählt ist; ein fehlender oder leerer Wert ist ein Boot-Fehler, ebenso ein Eintrag, der `failover` benennt (keine Verschachtelung), oder einer, der einen nicht existierenden Treiber benennt. Jeder Eintrag liest die Variablen seines eigenen Treibers. Nur Pushes fallen durch die Liste; jedes Lesen und jede Bestätigung geht an die erste Connection, jeder Fallback braucht also seinen eigenen Worker. |
| `QUEUE_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | Redis-URL (treiberabhängig erforderlich, wenn `QUEUE_DRIVER=redis`). |
| `QUEUE_REDIS_STREAM` | `"suprnova-queue"` | `String` | Redis-Stream-Schlüssel für das Fan-out. |
| `QUEUE_REDIS_GROUP` | `"default"` | `String` | Name der Consumer-Gruppe. |
| `QUEUE_REDIS_CONSUMER` | `"consumer-1"` | `String` | Consumer-Name innerhalb der Gruppe. Für parallele Worker pro Worker setzen. |
| `QUEUE_VISIBILITY_TIMEOUT_SECS` | `60` | `u64` | Wie lange ein beanspruchter Job unsichtbar bleibt, bevor ein anderer Consumer ihn erneut beanspruchen kann. Richten Sie das an Ihrem langsamsten Job aus. |
| `QUEUE_DB_TABLE` | `"jobs"` | `String` | Tabellenname für den Datenbank-Treiber. Wird als SQL-Bezeichner validiert - ein fehlerhafter Wert scheitert beim Boot, nicht erst beim Zusammensetzen des SQL. Treiberabhängig erforderlich, wenn `QUEUE_DRIVER=database`; der Treiber verlangt außerdem, dass `DB::init()` zuvor gelaufen ist. |
| `QUEUE_FAILED_DB_TABLE` | `"failed_jobs"` | `String` | Tabelle, in die der Dead-Letter-Speicher schreibt. Wird bei `QUEUE_DRIVER=database` automatisch gebunden - `queue:retry` liest sie und `Queue::retry_failed` braucht sie, die Tabelle gehört also zum Vertrag dieses Treibers. Von `memory` (konstruktionsbedingt flüchtig) und `redis` (keine Tabelle zum Schreiben) nicht verwendet. Anders als bei `QUEUE_DB_TABLE` lässt ein fehlerhafter Bezeichner hier den Boot **nicht** scheitern: Er wird auf `error!` protokolliert und es bleibt kein Speicher gebunden, sodass ins Dead-Letter verschobene Jobs vollständig protokolliert statt persistiert werden. Von Hand wiederherstellbar, aber nicht durch `queue:retry`. |

## Zeitplan

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION` | nicht gesetzt | `bool`-artig | Bestätigt, dass ein mit `on_one_server()` markierter Task einen Leader über einen **pro-Prozess**-Cache wählt. Diese Wahl ist nur so geteilt wie der Cache dahinter, daher ist in Produktion `CACHE_DRIVER=memory` zusammen mit einem Single-Server-Task ein harter Boot-Fehler, der die betroffenen Tasks benennt, statt einer stillen Herabstufung zu "jede Replik führt ihn aus". Setzen Sie dies nur, wenn das Deployment tatsächlich genau einen Scheduler betreibt; andernfalls setzen Sie `CACHE_DRIVER=redis`. Siehe [Task-Planung](scheduling.md). |

## Workflow

Der langlebige, zustandsbehaftete `#[workflow]`-Worker. Alle Werte
werden auf sichere Mindestwerte geklemmt statt blind befolgt - ein
`WORKFLOW_CONCURRENCY=0` würde das Worker-Semaphore für immer parken,
daher warnt das Framework und klemmt, statt eine offensichtlich
kaputte Konfiguration zu akzeptieren.

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `WORKFLOW_CONCURRENCY` | `4` | `usize` | Maximale gleichzeitige Workflow-Ausführungen pro Worker-Prozess. Geklemmt auf `>= 1`. |
| `WORKFLOW_POLL_INTERVAL_MS` | `1000` (ms) | `u64` | Wie oft der Worker nach neu fälligen Workflows pollt. |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30` (Sekunden) | `u64` | Reclaim-Timeout für eine beanspruchte Workflow-Zeile, deren Worker gestorben ist. |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | `i32` | Maximale Versuche pro Workflow-Lauf, bevor er als fehlgeschlagen markiert wird. Geklemmt auf `>= 1`. |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | `i64` | Linearer Backoff pro Versuch. Geklemmt auf `>= 0` - negativer Backoff würde Wiederholungen in der Vergangenheit einplanen und einen Tight-Loop-Reclaim erzeugen. |

## Mail

`MAIL_DRIVER` ist standardmäßig **`log`** - ausgehende Mail wird an den konfigurierten Tracing-Subscriber ausgegeben, statt das Netzwerk zu erreichen. Verwenden Sie `memory` in Tests, `file` für `.eml`-Vorschauen, die Sie in einem Mail-Client öffnen können, und `smtp`/`ses`/usw. in Produktion. Die providerspezifischen Schlüssel und Token sind nur erforderlich, wenn dieser Treiber gewählt ist; ein unbekannter Treiberwert protokolliert `warn!` und fällt auf `log` zurück.

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `MAIL_DRIVER` | `"log"` | `String` (`log`, `memory`, `file`, `smtp`, `ses`, `sendgrid`, `mailgun`, `postmark`, `resend`) | Wählt das Bootstrap-Ziel. |
| `MAIL_FROM` | keine - für Auth-Flow-Fassaden erforderlich | `String` | Standard-Absenderadresse für Auth-Flow-Fassaden (`EmailVerification`, `PasswordReset`, `TwoFactor`). Für diese Pfade erforderlich; fehlt sie, schlägt der Aufruf fehl, statt stillschweigend auf einen Platzhalter zurückzufallen, der DMARC/SPF verletzen würde. |
| `MAIL_FROM_NAME` | nicht gesetzt | `String` | Optionaler Anzeigename für das Auth-Flow-`From` (seit **0.5.9**). Ist er gesetzt, wird der Header als `Name <MAIL_FROM>` gerendert; `MAIL_FROM` bleibt eine reine Adresse. Wird beim Senden gelesen und gilt daher auch für eingereihte Auth-Flow-Mail. |

### Datei (`MAIL_DRIVER=file`)

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `MAIL_FILE_PATH` | `storage_path("mail")` | `String` | Verzeichnis, in das pro Sendung eine RFC-5322-Datei `.eml` geschrieben wird. Wird nie bereinigt. Absolute Pfade werden unverändert verwendet; relative Pfade sind im Anwendungsbasisverzeichnis verankert (siehe `APP_BASE_PATH`). |

### SMTP (`MAIL_DRIVER=smtp`)

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `MAIL_SMTP_HOST` | `"127.0.0.1"` | `String` | SMTP-Host. |
| `MAIL_SMTP_PORT` | `587` | `u16` | SMTP-Port. |
| `MAIL_SMTP_USER` | nicht gesetzt | `String` | SMTP-Benutzername. Für einen verschlüsselten Transport müssen sowohl `MAIL_SMTP_USER` **als auch** `MAIL_SMTP_PASS` gesetzt sein; ist keines gesetzt, verwendet die Verbindung standardmäßig den unverschlüsselten Local-Catcher-Modus. Ist genau eines gesetzt, warnt der Boot. |
| `MAIL_SMTP_PASS` | nicht gesetzt | `String` | SMTP-Passwort. Siehe `MAIL_SMTP_USER` für das Verhalten bei unvollständigen Credentials. |
| `MAIL_SMTP_ENCRYPTION` | abgeleitet | `starttls` \| `tls` \| `none` | Wie die Verbindung verschlüsselt wird. Ungesetzt leitet sich aus den Credentials ab: `starttls`, wenn beide gesetzt sind, `none`, wenn keines gesetzt ist. `tls` wählt implizites TLS (Port 465). `ssl` und `null` werden als Laravel-kompatible Aliase akzeptiert. Ein nicht erkannter Wert lässt den Boot in **jeder** Umgebung fehlschlagen - ein Tippfehler darf nicht zu Klartext degradieren. |
| `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION` | nicht gesetzt | `bool`-artig | Produktion verweigert den Boot bei einer unverschlüsselten SMTP-Verbindung. Setzen Sie `1`/`true`/`yes`/`on`, um Klartext zu bestätigen - vertretbar nur, wenn das Relay ausschließlich über ein privates Netzwerk erreichbar ist. |

### Postmark (`MAIL_DRIVER=postmark`)

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `MAIL_POSTMARK_TOKEN` | treiberabhängig erforderlich | `String` | Postmark-Server-Token. |
| `MAIL_POSTMARK_ENDPOINT` | Postmark-Standard | `String` | Überschreibt den API-Endpunkt (regional oder Mock-Server). |

### Amazon SES (`MAIL_DRIVER=ses`)

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `MAIL_SES_ACCESS_KEY` | treiberabhängig erforderlich | `String` | AWS-Access-Key. |
| `MAIL_SES_SECRET_KEY` | treiberabhängig erforderlich | `String` | AWS-Secret-Key. |
| `MAIL_SES_REGION` | `"us-east-1"` | `String` | AWS-Region. |
| `MAIL_SES_ENDPOINT` | AWS-Standard für die Region | `String` | Überschreibt den SES-Endpunkt (regional oder Mock-Server). |

### SendGrid (`MAIL_DRIVER=sendgrid`)

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `MAIL_SENDGRID_API_KEY` | treiberabhängig erforderlich | `String` | SendGrid-API-Key. |
| `MAIL_SENDGRID_ENDPOINT` | SendGrid-Standard | `String` | Überschreibt den API-Endpunkt. |

### Mailgun (`MAIL_DRIVER=mailgun`)

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `MAIL_MAILGUN_API_KEY` | treiberabhängig erforderlich | `String` | Mailgun-API-Key. |
| `MAIL_MAILGUN_DOMAIN` | treiberabhängig erforderlich | `String` | Mailgun-Sendedomain. |
| `MAIL_MAILGUN_ENDPOINT` | Mailgun-Standard | `String` | Überschreibt den API-Endpunkt (z. B. EU vs. US). |

### Resend (`MAIL_DRIVER=resend`)

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `MAIL_RESEND_API_KEY` | treiberabhängig erforderlich | `String` | Resend-API-Key. |
| `MAIL_RESEND_ENDPOINT` | Resend-Standard | `String` | Überschreibt den API-Endpunkt. |

## Ratenbegrenzung

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `RATE_LIMIT_DRIVER` | `memory` | `String` (`memory`, `redis`) | Wählt das Rate-Limiter-Backend. Außerhalb der Produktion protokolliert ein unbekannter Wert eine `warn!`-Meldung und fällt auf Memory zurück; **in Produktion lässt Memory - auch über einen unbekannten Wert - den Boot fehlschlagen**, sofern nicht `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` gesetzt ist. |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | nicht gesetzt | `bool`-artig | Bestätigt Pro-Prozess-Rate-Limit-Buckets in Produktion. Nur korrekt, wenn Sie genau einen Prozess betreiben: Hinter N Repliken ist jedes Kontingent effektiv N-fach und setzt sich bei jedem Deploy zurück. |
| `RATE_LIMIT_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | Redis-URL (treiberabhängig erforderlich, wenn `RATE_LIMIT_DRIVER=redis`). |
| `RATE_LIMIT_PREFIX` | `"suprnova:"` | `String` | Schlüssel-Präfix in Redis. |

## Bilder

Auswahl des Bildtreibers und die Dekodier-Limits, die feindliche Eingabe
begrenzen. Limits außerhalb des gültigen Bereichs werden mit einem
`warn!` begrenzt, statt den Boot scheitern zu lassen: Ein Limit von null
würde jedes Bild in der Anwendung ablehnen. Ein unbekanntes
`IMAGE_DRIVER` scheitert beim ersten Gebrauch und benennt dabei die
gültigen Werte.

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `IMAGE_DRIVER` | `oxideav` | `String` (`oxideav`, `magick`) | Wählt das Bild-Backend. `oxideav` ist pures Rust ohne Host-Abhängigkeit; `magick` ruft ein auf dem Host installiertes ImageMagick 7 auf, für breitere Eingabeunterstützung. Groß-/Kleinschreibung wird nicht beachtet. |
| `IMAGE_MAX_DIMENSION` | `16384` | `u32` | Obergrenze für Breite und Höhe eines dekodierten Bildes, geprüft gegen den Header der Eingabe selbst, bevor irgendetwas alloziert wird. Begrenzt auch die Ziele einer Größenänderung. Minimum `1`. |
| `IMAGE_MAX_ALLOC_BYTES` | `268435456` (256 MiB) | `u64` | Obergrenze für den dekodierten RGBA-Speicherbedarf (`width * height * 4`). Begrenzt auch die Größe der Quelldatei selbst, ob sie nun aus einem Pfad, von einer Disk oder aus `Image::from_stream` kommt (das schon beim Einsammeln prüft). Minimum `4`. |
| `IMAGE_MAGICK_BINARY` | `magick` | `String` | Binary, die der `magick`-Treiber aufruft. Nur ImageMagick 7; der Name `convert` aus ImageMagick 6 wird nicht akzeptiert. Eine fehlende Binary ist ein klarer Fehler beim ersten Gebrauch. |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | `u32` | Echtzeit-Obergrenze für einen einzelnen ImageMagick-Aufruf. Sie ist zugleich ImageMagicks eigenes `-limit time`-Argument und die Frist auf der Rust-Seite, die zwei Sekunden später die gesamte Prozessgruppe des Kindprozesses tötet, denn `-limit time` wird von einem Monitor durchgesetzt, den ein in einem Delegate feststeckendes Kind nie erreicht. Begrenzt ein hängendes Delegate, das sonst für die Lebensdauer des Prozesses einen blockierenden Worker belegen würde. Nur `magick`-Treiber. Minimum `1`. |

Wie die Limits auf zwei Ebenen durchgesetzt werden und wie Sie zwischen
den Treibern wählen, steht unter [Bilder](images.md).

## Hashing

Passwort-Hashing-Treiber und Parameter je Algorithmus. Ungültige
Werte liefern beim ersten Hash einen `FrameworkError::param` und
machen eine Fehlkonfiguration sofort sichtbar, statt stillschweigend
zu defaulten.

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `HASH_DRIVER` | `bcrypt` | `String` (`bcrypt`, `argon`/`argon2i`, `argon2id`) | Aktiver Hashing-Algorithmus. Ohne Unterscheidung von Groß-/Kleinschreibung. |
| `HASH_ROUNDS` | `12` | `u32` | Bcrypt-Cost (Bereich `4..=31`). Werte außerhalb des Bereichs schlagen mit einem klaren Fehler fehl. |
| `HASH_MEMORY` | `65536` (64 MiB, Einheit KiB) | `u32` | Argon2-Speicher in KiB. Minimum `8`. Nur für Argon. |
| `HASH_TIME` | `4` | `u32` | Argon2-Zeit/Iterationen. Minimum `1`. Nur für Argon. |
| `HASH_THREADS` | `1` | `u32` | Argon2-Parallelität (entspricht OWASP/libsodium). Minimum `1`. Nur für Argon. |
| `HASH_VERIFY` | `false` | `bool` | Wenn wahr, weist `verify()` Hashes eines anderen Algorithmus als `HASH_DRIVER` zurück (liefert `Ok(false)`). Standard `false`, damit Legacy-Bcrypt-Hashes nach einem Treiberwechsel weiterhin verifizieren, bis sie rotiert wurden. |

## Validierung

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `HIBP_TIMEOUT_SECS` | `30` (Sekunden) | `u64` | Anfrage-Timeout für die Have-I-Been-Pwned-Range-Prüfung von `Password::uncompromised()`, bei jeder Konstruktion eines Standard-`HibpVerifier` frisch gelesen. Ein langsames oder nicht erreichbares HIBP lässt das Passwort weiterhin durchgehen - siehe [Validierung](validation.md). |

## Auth-Flows

Zwei-Faktor-Authentifizierung verwendet `APP_NAME` (behandelt unter
Anwendung) als TOTP-Issuer-String - es gibt keine eigene
`2FA_ISSUER`-Env-Variable. Der Issuer fällt auf `"Suprnova"` zurück,
wenn `APP_NAME` ungesetzt ist.

## Inertia / Frontend

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `SUPRNOVA_FRONTEND` | `svelte` | `String` (`svelte`, `react`, `vue`) | Aktives Frontend. Ohne Unterscheidung von Groß-/Kleinschreibung. Steuert `Frontend::detect_from_env()`, den Standard-Vite-Einstiegspunkt und die Suchreihenfolge der Seiten-Komponenten-Erweiterung zur Compile-Zeit. Unbekannte oder ungesetzte Werte fallen auf `svelte` zurück. |

## Wartungsmodus

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `MAINTENANCE_DRIVER` | `file` | `String` (`file`, `cache`) | Wählt, wie der `down`/`up`-Zustand gespeichert wird. `file` schreibt in den Storage-Pfad des Frameworks; `cache` reitet auf dem konfigurierten Cache-Treiber (nützlich, wenn viele App-Instanzen den Wartungszustand koordinieren müssen). Jeder andere Wert fällt auf `file` zurück. |

## Ereignisse

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `EVENT_MAX_CONCURRENCY` | `256` | `usize` | Obergrenze für gleichzeitige eingereihte Listener-Tasks. Werte `<= 0` oder nicht parsbare fallen auf den Standard zurück. Gilt für `Event::queue` / eingereihte Listener; synchrone Listener unterliegen dieser Grenze nicht. |

## Protokollierung

`LOG_FORMAT` ist **umgebungsbewusst**: In Produktion
(`APP_ENV=production`) ist der Standard `json` für die
Log-Aggregator-Freundlichkeit; überall sonst ist der Standard
`pretty` für menschenlesbare lokale/Dev-Ausgabe. Ein expliziter Wert
gewinnt immer.

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `LOG_LEVEL` | `"info"` | `String` (`error`, `warn`, `info`, `debug`, `trace` - ohne Unterscheidung von Groß-/Kleinschreibung) | Filter-Level des Tracing-Subscribers. |
| `LOG_FORMAT` | umgebungsbewusst (`json` in Produktion, `pretty` sonst) | `String` (`json`, `pretty`) | Ausgabeformat des Tracing-Subscribers. |

## Beobachtbarkeit (OpenTelemetry)

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | nicht gesetzt (Telemetrie deaktiviert) | `String` | OTLP-Collector-Endpunkt. Wenn ungesetzt (oder nur Whitespace), werden keine Exporter installiert, und das Framework nutzt weiterhin den Standard-`tracing`-Subscriber. |
| `OTEL_SERVICE_NAME` | `"suprnova"` | `String` | Ressourcen-Attribut `service.name` auf jedem Span-/Metrik-/Log-Eintrag. |
| `OTEL_SERVICE_VERSION` | `CARGO_PKG_VERSION` zur Build-Zeit | `String` | Ressourcen-Attribut `service.version`. |
| `OTEL_SDK_DISABLED` | `false` | `bool` | Standard-OTel-Kill-Switch. Wenn wahr, werden keine Exporter installiert, unabhängig von `OTEL_EXPORTER_OTLP_ENDPOINT`. |

## CLI / Dev-Server

Diese werden von der `suprnova`-CLI-Binary gelesen (Dev-Server,
SSR-Worker), nicht vom Laufzeit-Framework - sie tauchen in der
Starter-`.env` auf oder werden von `suprnova serve` /
`suprnova ssr:*` beachtet.

| Variable | Standard | Typ | Zweck |
|---|---|---|---|
| `VITE_PORT` | `5765` | `u16` | Port, an den Vite in `suprnova serve` bindet. Die CLI-Option `--frontend-port` überschreibt ihn. |
| `SUPRNOVA_SSR_RUNTIME` | `"node"` | `String` | Runtime, unter der der SSR-Worker gestartet wird (`suprnova ssr:start`). Die CLI-Option `--runtime` überschreibt sie. |
| `SUPRNOVA_SSR_BUNDLE` | `frontend/bootstrap/ssr/ssr.js` | `Path` | Pfad zum gebauten SSR-Bundle. Die CLI-Option `--bundle` überschreibt ihn. |
| `SUPRNOVA_SSR_URL` | `"http://127.0.0.1:13714"` | `String` | SSR-Worker-URL für `suprnova ssr:check`. Die CLI-Option `--url` überschreibt sie. |

## Subsysteme ohne Env-Variablen

Ein paar Subsysteme werden vollständig im Rust-Code über den
Container oder die Service-Registrierung konfiguriert - sie haben
**null** Env-Variablen, die das Framework liest:

- **Dateisystem / Storage.** Disks werden mit
  `FilesystemRegistry::add_disk(name, driver)` in `bootstrap()`
  registriert. Es gibt keine `FILESYSTEM_DISK`-Env-Variable (der
  Name taucht in manchen Starter-`.env`-Dateien auf, wird aber vom
  Framework nicht konsultiert - siehe "Variablen, die das Framework
  nicht liest" unten).
- **Broadcasting & WebSockets.** Channels werden mit dem
  `ws!()`-Makro und der `BroadcastHub`-Konfiguration im Code
  registriert. Der Treiber selbst reitet auf dem, was der
  konfigurierte `CACHE_DRIVER` wählt.
- **CORS, CSRF, Idempotenz, Timeout.** Konfiguriert über
  Builder-Strukturen, die in `bootstrap()` an die
  Middleware-Konstruktoren übergeben werden. Die Standardwerte sind
  konservativ genug, dass eine typische App sie nie berührt.
- **Magnetar und OAuth.** `MagnetarConfig` wird im Bootstrap der Anwendung erstellt. Der API-Starter liest `PASSKEY_RP_ID` und `PASSKEY_RP_ORIGIN`, das Framework selbst jedoch nicht. OAuth-Provider-IDs, -Secrets, Callback-URLs, Scopes, Transports und Policy-Werte werden programmatisch über die Magnetar-Provider-Registry bereitgestellt. Anwendungen können diese Werte aus Umgebungsvariablen oder einem Secret Manager beziehen.
- **Vector-Suche, Benachrichtigungen, Zahlungen, Feature Flags.**
  Jedes registriert konkrete Treiber über `App::bind` in
  `bootstrap()`. Wählen Sie Ihren Treiber in Rust; übergeben Sie
  benötigte URLs/Keys als eigene Env-Variablen.

## Variablen, die das Framework nicht liest

Die gescaffoldete Starter-`.env` listet ein paar Schlüssel zur
Bequemlichkeit des menschlichen Autors, die das Framework nie
konsultiert. Sie sind hier dokumentiert, damit ein Leser, der nach
ihnen sucht, nicht im Unklaren bleibt:

- `MAIL_FROM_ADDRESS` - ein Laravel-artiger Platzhalter, den das
  Framework nie konsultiert. Die tatsächliche Von-Adresse, die die
  Auth-Flow-Facades verwenden, ist `MAIL_FROM` (behandelt unter
  Mail). Ihre eigenen `Mailable`-Typen können ihn über
  `env_optional` lesen, wenn Sie den Laravel-Namen beibehalten
  wollen, aber nichts in `suprnova::*` tut das. (`MAIL_FROM_NAME`
  **wird** seit 0.5.9 gelesen - siehe das Mail-Kapitel - und steht
  deshalb hier nicht mehr.)
- `FILESYSTEM_DISK` - Platzhalter für den Namen der Standard-Disk.
  Setzen Sie den Standard statt dessen im Code über
  `FilesystemRegistry::set_default(name)`.

## Wie Werte geparst werden

Eine kurze Referenz für die drei Env-Helfer-Varianten - siehe
[Konfiguration](configuration.md#direct-env-access) für die
vollständige Behandlung:

| Helfer | Verhalten bei Fehlen | Verhalten bei Nicht-Parsbarkeit |
|---|---|---|
| `env(key, default)` | liefert `default` | `warn!` + liefert `default` |
| `env_required(key)` | **gerät in Panic** | **gerät in Panic** |
| `env_optional(key)` | liefert `None` | `warn!` + liefert `None` |
| `env_strict(key)` (intern, verwendet von `try_from_env`) | liefert `Ok(None)` | liefert `Err(FrameworkError)` - der Boot bricht ab |

Die strengen Varianten (`AppConfig::try_from_env`,
`ServerConfig::try_from_env`) sind das, was `Config::init` aufruft,
sodass ein Tippfehler in `APP_DEBUG=tru` oder `SERVER_PORT=80a0` den
Boot mit einem strukturierten Fehler abbricht, statt stillschweigend
auf den Standard zurückzufallen. Die nachsichtigen Varianten
existieren für die breitere Population an Aufrufstellen
(einschließlich `impl Default`), wo ein Parse-Fehler nicht in Panic
geraten darf.

## Umgebungsspezifische Overrides

Der Loader liest Dateien in dieser Reihenfolge, jede überschreibt
die vorherige:

1. `.env`
2. `.env.<environment>` (z. B. `.env.production`, `.env.staging`,
   `.env.testing`, `.env.<custom>` für `APP_ENV=<custom>`)
3. Prozessumgebung

Das bedeutet, ein containerisiertes Produktions-Deployment kann eine
minimale `.env.production` mitliefern, die nur die Schlüssel
überschreibt, die von `.env` abweichen (Treiber-Namen, URLs,
Schlüsselmaterial), und die echte Container-Umgebung überschreibt
beide für Secrets, die nie in einer committeten Datei landen
sollten.

Siehe [Konfiguration](configuration.md#how-env-loading-works) für
das exakte Loader-Verhalten und das `LOADED_KEYS`-Tracking, das
verhindert, dass veraltete `.env`-Werte über Reloads hinweg in die
Stufe der "echten System-Umgebung" aufsteigen.

## Nächste Schritte

- [Konfiguration](configuration.md) - typisierte
  `Config::*`-Registrierung, die `env*`-Helfer, Umgebungserkennung
- [Bereitstellung](deployment.md) - was in Produktion zu setzen ist
- [Verschlüsselung](encryption.md) - `APP_KEY`-Rotation über
  `APP_KEY_PREVIOUS`
- [Application Bootstrap](bootstrap.md) - wo die env-gesteuerte
  Boot-Reihenfolge festgelegt wird

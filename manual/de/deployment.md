# Bereitstellung - Übersicht

Eine Suprnova-Anwendung wird in eine selbstständige Binärdatei kompiliert, die
den Web-Server, den Migrations-Runner, den Scheduler und den Queue-Worker
besitzt. Die Bereitstellung ist "Binärdatei kopieren, vier Umgebungsvariablen setzen,
ausführen." Dieses Kapitel behandelt, was diese vier Variablen sind, was die
Subkommandos der Binärdatei in der Produktion tun, und wie der integrierte
Health-Endpunkt mit einem Liveness-Probe der Plattform integriert. Plattformspezifische
Anleitungen folgen in [Railway](deployment-railway.md),
[Digital Ocean](deployment-digital-ocean.md) und
[Hetzner](deployment-hetzner.md).

## Die einzelne Binärdatei

Ihre Anwendung wird in eine Binärdatei mit einer clap-Subkommando-Oberfläche kompiliert:

```bash
./app                       # serve (Standard) - automatisch migrieren, dann HTTP
./app serve                 # explizites serve, mit Auto-Migration
./app serve --no-migrate    # serve, ohne Migrationen auszuführen
./app web:run               # Alias für serve

./app migrate               # ausstehende Migrationen anwenden und beenden
./app migrate:status        # Migrationsstatus anzeigen
./app migrate:rollback [N]  # die letzten N Migrationen zurückrollen (Standard 1)
./app migrate:fresh         # alle Tabellen löschen, dann neu migrieren - in der Produktion
                            # braucht das --force UND eine eingetippte Bestätigung auf
                            # einem interaktiven Terminal; siehe cli-migrations.md

./app schedule:work         # Scheduler-Daemon - wacht jede Minute auf
./app schedule:run          # fällige Tasks einmal ausführen und beenden
./app schedule:list         # jeden registrierten Task ausgeben
./app queue:work            # Queue-Worker-Daemon
./app workflow:work         # Workflow-Worker-Daemon

./app down [--secret …] [--retry …] [--except …] [--message …]
./app up                    # Wartungsmodus verlassen
```

Eine Binärdatei bedeutet ein Docker-Image, ein CI-Artefakt, eine Bereitstellung zu
verifizieren. Das gleiche Image führt den Web-Service, den Scheduler, den Queue-Worker
und den Workflow-Worker aus - Sie starten für jeden einen anderen Subkommando.

## Vier Produktions-Umgebungsvariablen

Suprnova bricht beim Start ab, wenn die Produktionsumgebung nicht
richtig konfiguriert ist. Der Mindestsatz zum Bereitstellen:

| Variable | Was es tut | Fehlerfall |
|---|---|---|
| `APP_ENV` | Wählt die Umgebung (`production`, `staging`, usw.). | Verwendet `local` wenn nicht gesetzt - Ihre Anwendung läuft im Dev-Modus in der Produktion. |
| `APP_KEY` | 32-Byte AES-256 base64-Schlüssel für `Crypt`, Sessions, Cookies und Paginierungscursor. | Start gibt einen typisierten Fehler aus und beendet mit Non-Zero, wenn `APP_ENV` nicht local/dev/test ist und `APP_KEY` fehlt oder fehlerhaft ist. |
| `APP_URL` | Kanonische absolute URL Ihrer Anwendung (`https://app.example.com`). | Standardwert ist `http://localhost:8765`; signierte URLs, Umleitung, Mail-Links und absolute Inertia-URLs verwenden dies. |
| `DATABASE_URL` | Verbindungs-URL für Ihre relationale Datenbank. | Start verweigert den Start, wenn `APP_ENV` `production` oder `staging` ist und `DATABASE_URL` nicht gesetzt ist - der Dev SQLite-Fallback wird explizit abgelehnt. |

Generieren Sie `APP_KEY` einmal mit der CLI:

```bash
suprnova key:generate           # schreibt APP_KEY=… in ./.env
suprnova key:generate --show    # gibt den Schlüssel für $(…) aus
```

Für Schlüsselrotation siehe [Verschlüsselung](encryption.md) -
`APP_KEY_PREVIOUS` (oder das Laravel-kompatible `APP_PREVIOUS_KEYS`)
nimmt eine komma-separierte Liste älterer Schlüssel für Decrypt-Only-Fallback.

Neben den vier erforderlichen Variablen, gemeinsame Produktionsoptionen:

| Variable | Standard | Hinweise |
|---|---|---|
| `SERVER_HOST` | `127.0.0.1` | Verwenden Sie `0.0.0.0` in Containern. |
| `SERVER_PORT` | `8765` | Entsprechen Sie dem erwarteten Port Ihrer Plattform. |
| `APP_DEBUG` | Umgebungsabhängig | `false` in production/staging/custom envs. Setzen Sie explizit, wenn Sie sichtbare Fehler im Staging möchten. |
| `SERVER_MAX_BODY_SIZE` | Pro-Handler-Standard | Prozessweite Request-Body-Grenze. |
| `SERVER_MAX_CONNECTIONS` | Nicht gesetzt (unbegrenzt) | Limit für gleichzeitig aktive TCP-Verbindungen. Siehe unten. |
| `SERVER_HEALTH_READINESS_TOKEN` | Nicht gesetzt (Readiness ist öffentlich) | Gemeinsames Secret erforderlich um die Readiness-Probe zu erreichen. Siehe [Health Check](#health-check). |
| `DB_MAX_CONNECTIONS` | `10` | Pool-Größe. |
| `REDIS_URL` | Nicht gesetzt | Erforderlich wenn Sie die Redis cache/queue/session Treiber konfiguriert haben. |

Die vollständige Tabelle finden Sie in [Umgebungsvariablen](env-vars.md).

## Empfohlene Datenbank: MariaDB

Suprnova unterstützt SQLite, PostgreSQL, MySQL und MariaDB als erstklassige
relationale Backends. Die Empfehlung ist umgebungsspezifisch:

- **Entwicklung.** SQLite. Der Scaffolder schreibt
  `DATABASE_URL=sqlite://./database.db` sodass `suprnova serve` mit
  null Datenbanksetup funktioniert.
- **Produktion.** MariaDB. Es vereinigt das, was sonst drei separate
  Services wären (relational + vector + KV cache) auf einer Engine,
  mit systemversionierten Tabellen für Audit, falls Sie diese brauchen.

```bash
# .env.production
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production
```

Verwenden Sie das `mysql://`-Schema - SeaORMs MySQL-Treiber behandelt MariaDB
nativ, und Suprnovas `MariaDbVectorDriver` (`VECTOR(N)` + HNSW)
kann direkt für Vector-Workloads angehakt werden.

Die anderen relationalen Backends sind auch erstklassig:

```bash
# PostgreSQL
DATABASE_URL=postgres://app_user:secret@db.internal:5432/app_production

# MySQL
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production

# SQLite (für winzige Bereitstellungen mit einer Instanz)
DATABASE_URL=sqlite:///var/lib/myapp/data.db
```

### Warum Suprnova abweicht

Laravels Standardeinstellungen drängen neue Projekte zu PostgreSQL, da PHP +
PostgreSQL der gut ausgetretene Pfad ist. Suprnova wählt die Datenbank, die
das saubere Single-Engine-Produktions-Posture für eine Rust-Anwendung bietet.
MariaDBs `VECTOR(N)` (11.7+), Dynamic Columns und systemversionierte
Tabellen bedeuten, dass ein kleines bis mittleres Produkt Suche, KV und Audit
ausliefern kann, ohne Redis, OpenSearch oder pgvector anzuschrauben. PostgreSQL
bleibt vollständig unterstützt - die Test-Matrix des Frameworks läuft gegen alle
drei relationalen Backends - aber unsere Bereitstellungsdokumentation führt mit der
Engine, die bewegliche Teile minimiert. Siehe
[Vector Storage](vector.md) und [Datenbank](database.md) für die
Backend-spezifischen Oberflächen.

## Ein Produktions-Image erstellen

Der Scaffolder liefert einen Generator für ein mehrstufiges Dockerfile:

```bash
suprnova docker:init
```

Dies schreibt ein `Dockerfile` mit drei Stufen:

1. **Frontend build** - `node:20-alpine`, führt `npm ci && npm run build`
   gegen Ihre `frontend/` Inertia-Anwendung (Svelte 5, React 19 oder Vue 3.5
   je nach Ihrer Scaffold-Wahl) aus.
2. **Backend build** - `rust:1.91.1-slim-bookworm`, kompiliert Ihre Crate im
   Release-Modus mit Dependency-Caching.
3. **Runtime** - `debian:bookworm-slim`, kopiert die kompilierte Binärdatei
   und Vite-Ausgabe, läuft als Non-Root `appuser`, stellt Port 8765 bereit,
   und führt `CMD ["./app"]` aus (den Auto-Migrating-Server).

Erstellen und führen Sie lokal aus um vor dem Push zu überprüfen:

```bash
docker build -t myapp .

# Mit einer env-Datei
docker run --rm -p 8765:8765 --env-file .env.production myapp

# Oder mit expliziten Variablen (den vier erforderlichen)
docker run --rm -p 8765:8765 \
  -e APP_ENV=production \
  -e APP_KEY=$APP_KEY \
  -e APP_URL=https://app.example.com \
  -e DATABASE_URL=mysql://user:pass@host:3306/app \
  myapp
```

Commiten Sie niemals `.env.production` (oder irgendeine Datei mit `APP_KEY` oder
`DATABASE_URL`) zu Ihrem Repository. Verwenden Sie den Secrets-Store Ihrer Plattform
und lesen Sie die Werte zur Bereitstellungszeit.

## Migrationen beim Start

Der Standard `./app` (und explizite `./app serve`) Befehl führt alle
ausstehenden Migrationen durch, bevor der Socket gebunden wird. Die zwei praktischen
Implikationen:

- **Sicher mit mehreren Instanzen.** SeaORMs Migration-Runner nimmt ein
  Datenbank-Ebenen Advisory Lock; der langsamste Pod wartet, die anderen
  fahren fort, wenn er fertig ist. Sie benötigen keinen separaten "migrate-then-deploy"
  Schritt für regelmäßige Release-Rollouts.
- **Fehlerhafte Migration = fehlgeschlagene Bereitstellung.** Falls eine Migration
  fehlschlägt, beendet sich der Prozess mit Non-Zero, bevor der Server bindet. Die
  Health-Probe der Plattform (siehe unten) meldet den Pod als unhealthy und der
  Rollout stoppt. Beheben Sie vorwärts durch Versand einer korrigierenden Migration
  in der nächsten Ausgabe.

Für CI-Pipelines, die die Bereitstellung beim erfolgreichen Migrations-Gate
vor dem Annehmen von Traffic durch einen Pod durchführen möchten, führen Sie
Migrationen in einem One-Shot durch:

```bash
docker run --rm myapp ./app migrate
# … dann die eigentliche Bereitstellung ausrollen
docker run myapp ./app serve --no-migrate
```

`--no-migrate` springt die Auto-Migrations-Phase, startet aber noch den Server
normal.

## Worker als separate Services

Der Scheduler, Queue und Workflow-Systeme haben jeweils ihre eigenen Daemon-Subkommandos.
In der Produktion führen Sie sie als separate Prozesse gegen das gleiche Image aus und teilen
die gleiche Umgebung:

```bash
docker run myapp ./app schedule:work    # eine Instanz - siehe unten
docker run myapp ./app queue:work       # auf N Instanzen skalieren
docker run myapp ./app workflow:work    # auf N Instanzen skalieren
```

Zwei Regeln zum Verinnerlichen:

- **Führen Sie entweder genau einen `schedule:work` Prozess aus, oder markieren Sie Ihre Tasks
  `.on_one_server()`.** Scheduler-Replikas koordinieren nicht standardmäßig:
  jeder bewertet den Zeitplan unabhängig, also führen drei Replikas jede fällige Task dreimal aus.
  `replicas: 1` ist die einfache Antwort; `.on_one_server()` wählt eine Replik pro Tick
  gegen einen gemeinsamen Cache und ist was Sie wollen wenn der Scheduler
  hochverfügbar sein muss. Siehe [Task-Planung](scheduling.md#running-on-one-server).
- **Queue- und Workflow-Worker skalieren horizontal.** Beide nehmen Arbeit
  aus einem gemeinsamen Store und verwenden Visibility Timeouts oder Row-Level-Locks
  zur Koordination; das Hinzufügen von Pods erhöht den Durchsatz. `./app queue:work
  --max-jobs N` macht den Worker beenden nach N Jobs, damit ein Supervisor den Prozess
  rotieren kann - nützlich für Release-on-Restart Bereitstellungen.

Siehe [Warteschlangen](queues.md), [Task-Planung](scheduling.md) und
[Workflows](workflows.md) für die per-Subsystem Details.

## Sauber stoppen

Jeder lange laufende Suprnova-Prozess - der Server und alle drei Daemons -
leert auf **SIGTERM** sowie SIGINT. SIGTERM ist was `docker stop`,
Coolify, systemd und Kubernetes senden; SIGINT ist was Ctrl-C sendet. Beide
nehmen den gleichen Weg: Stopp um neue Arbeit zu akzeptieren, beenden Sie was in der Luft ist
innerhalb einer begrenzten Grace-Zeit, beenden Sie `0`.

Die Grace-Fenster sind pro-Subsystem und absichtlich begrenzt - ein langsamer
Client oder eine lange Task muss einen Prozess nicht unbegrenzt am Leben halten können:

| Prozess | Wartet auf | Grace |
|---|---|---|
| `serve` | in-flight HTTP-Verbindungen | 5s |
| `queue:work` | das In-Flight-Job zu begleichen | bis das Job zurückkehrt |
| `schedule:work` | `.run_in_background()` tasks | 30s |
| `workflow:work` | in-flight Workflow-Schritte | bis diese zurückkehren |

**Dimensionieren Sie die Beendigungs-Grace-Zeit Ihrer Plattform über diese hinaus.** Docker hat als Standard
10 Sekunden, Kubernetes 30. Falls das Fenster der Plattform kürzer ist als
die Arbeit dauert, sendet es SIGKILL und Sie sind zurück zum Verlieren von In-Flight-Jobs:

```yaml
# docker compose
services:
  worker:
    command: ["app", "queue:work"]
    stop_grace_period: 60s
```

```yaml
# kubernetes
spec:
  terminationGracePeriodSeconds: 60
```

**Ein Job der In-Flight beendet wird, geht nicht verloren, aber kostet einen Versuch.** Seine
Reservierung verfällt und ein anderer Worker fordert es zurück und belastet einen Versuch,
sodass ein Job, der seinen Worker zuverlässig beendet, stattdessen Dead-Letter werden kann
als ewig zu zykeln. Siehe [Warteschlangen](queues.md#what-counts-as-an-attempt).

**PID 1 ist eine echte Einschränkung.** Ein Container-Eintrittspunkt läuft als PID 1 und
der Kernel wendet keine Standard-Signal-Dispositionen auf PID 1 an - ein
Prozess ohne SIGTERM-Handler stirbt nicht auf SIGTERM, ignoriert es
bis die Plattform aufgibt und SIGKILL sendet. Suprnova installiert den
Handler, also ist `CMD ["app", "queue:work"]` wie geschrieben in Ordnung und keine `tini`
Shim ist erforderlich.

## Health Check

Suprnova stellt drei eingebaute Health-Pfade bereit. Das `_suprnova/`-Präfix ist
reserviert, damit Ihre eigenen Routen niemals mit ihnen kollidieren.

| Pfad | Berührt | Verwendung für |
|---|---|---|
| `/_suprnova/health/live` | nichts | Liveness. Antwortet 200 solange der Prozess eine Anfrage bedienen kann. |
| `/_suprnova/health/ready` | die Datenbank | Readiness. 503 wenn eine Abhängigkeit nicht erreichbar ist. |
| `/_suprnova/health` | nichts oder die Datenbank mit `?db=true` | Der ursprüngliche Endpunkt. Verhält sich wie einer der oben genannten. |

```bash
curl http://localhost:8765/_suprnova/health/live
# 200 {"status":"ok","timestamp":"2026-05-30T12:34:56+00:00"}

curl http://localhost:8765/_suprnova/health/ready
# Gesund:        200 {"status":"ok","timestamp":"…","database":"connected"}
# Eingeschränkt: 503 {"status":"degraded","timestamp":"…","database":"error"}
```

`/_suprnova/health` und `/_suprnova/health?db=true` funktionieren genau wie zuvor,
und nichts, das Sie bereits bereitgestellt haben, braucht zu ändern - der
[Hetzner-Anleitung](deployment-hetzner.md) benennt sie immer noch für One-Off-Checks
und könnte Ihre eigene Spezifikation tun. Die benannten Pfade sind klarer,
bevorzugen Sie sie also in neuen Konfigurationen; die [Railway](deployment-railway.md),
[DigitalOcean](deployment-digital-ocean.md) und [Docker](cli-docker.md)
Anleitung verwenden sie.

### Verwenden Sie die richtige Probe für die richtige Frage

Wählen Sie Liveness zu `/live` und Readiness zu `/ready`. Die Unterscheidung ist wichtiger als es aussieht:
eine fehlgeschlagene **Liveness**-Probe startet den Pod neu,
während eine fehlgeschlagene **Readiness**-Probe ihn nur aus dem Load-Balancer nimmt.
Verdrahten Sie einen Datenbankcheck in Liveness und ein Datenbankfehler startet
jede Replik neu, die Sie haben - genau in dem Moment, wenn die Datenbank am wenigsten
eine dröhnende Herde von Reconnects ertragen kann.

```yaml
livenessProbe:
  httpGet:
    path: /_suprnova/health/live
    port: 8765
readinessProbe:
  httpGet:
    path: /_suprnova/health/ready
    port: 8765
```

Der Endpunkt macht einen Short-Circuit vor der Middleware-Chain, sodass er
responsiv bleibt, auch wenn eine Middleware Deadlock hat oder die
Request-ID-Middleware Traffic ablehnt.

### Degradierte Antworten tragen keine Treiber-Details

Der 503-Text meldet `"database":"error"` und nichts weiter. Die Treiber-eigene
Nachricht - die Hosts, Ports, Datenbank- und Schemanamen und Serverversionen benennt,
und für einige Konfigurationsfehler die Verbindungs-URL - geht zum Log auf `error!`-Ebene,
wo ein Operator es lesen kann und ein Fremder nicht. In Debug-Builds wird es auch
in den Text als `database_error` aufgenommen, sodass lokales Debugging nicht beeinträchtigt wird.

### Readiness abschalten

Readiness führt einen Datenbank-Roundtrip für jeden aus, der fragt. Falls der Endpunkt
internetverfügbar ist, setzen Sie ein gemeinsames Secret:

```bash
SERVER_HEALTH_READINESS_TOKEN=<a long random string>
```

Probes müssen es dann als Header senden:

```bash
curl -H "X-Suprnova-Health-Token: $SERVER_HEALTH_READINESS_TOKEN" \
  http://localhost:8765/_suprnova/health/ready
```

```yaml
readinessProbe:
  httpGet:
    path: /_suprnova/health/ready
    port: 8765
    httpHeaders:
      - name: X-Suprnova-Health-Token
        value: <the same value>
```

Ohne den Header antwortet Readiness **404** - die gleiche Antwort wie jeder
Pfad, der nicht existiert, also ist der Endpunkt unsichtbar statt nur geschlossen.
Liveness bleibt auf jeden Fall öffentlich, daher brauchen Sie das Secret nicht
in jedem Manifest zu haben um Ihr Restart-on-Hang-Signal zu behalten.

Nicht gesetzt ist der Standard und Readiness ist öffentlich. Das ist absichtlich: die
Konfigurationen, die dieses Handbuch und der Scaffolder generieren, rufen alle
`?db=true` ohne Header auf und Standardmäßig geschlossen würde sie brechen.

## Wartungsmodus

Um eine destruktive Migration auszurollen oder den Verkehr bei einem
Vorfall stillzulegen:

```bash
./app down --secret abc123 \
           --retry 60 \
           --message "Deploying - back in a few minutes" \
           --except /webhooks/stripe

./app up
```

`down` schreibt einen Wartungs-Marker, den die Middleware bei jeder
Anfrage liest. Anfragen bekommen ein 503 (über `--status`
konfigurierbar) mit der angegebenen Nachricht, außer bei Pfaden in
`--except` und bei jeder Anfrage, die das Secret mitführt. `up` entfernt
den Marker.

Das Secret ist ein Bearer-Credential: Wer `/<secret>` besucht, bekommt
ein Bypass-Cookie mit zwölf Stunden Gültigkeit ausgestellt. Sowohl der
URL-Match als auch der Cookie-Match sind Vergleiche in konstanter Zeit,
sodass das Response-Timing einem Prober nicht verrät, wie lang das von
ihm korrekt geratene Präfix war. Bevorzugen Sie `--with-secret`, das
eines für Sie prägt (16 zufällige Bytes, 32 Hex-Zeichen) und die
Bypass-URL ausgibt, gegenüber dem Wählen einer merkbaren Zeichenkette
für `--secret` - und behandeln Sie es wie jedes andere Credential in
Ihren Incident-Notizen.

## Skalierung

### Web

Horizontale Skalierung ist die Standard-Story: jeder Pod führt `./app` aus,
teilt `DATABASE_URL` und verbindet sich mit demselben Redis (falls Sie
Redis-gestützte cache/queue/session konfiguriert haben). Auto-Migration ist sicher
wegen des Advisory Locks oben. Sticky Sessions sind nicht erforderlich -
Session-Status lebt in Ihrem Session-Treiber (Datenbank oder Redis),
nicht im Process-Memory.

### Worker

- **Scheduler.** Genau eine Instanz, immer.
- **Queue.** Skalieren Sie horizontal. Wenn Sie Arbeit über mehrere
  benannte Queues verteilt haben, führen Sie einen Worker pro Queue aus (oder übergeben Sie treiberspezifische Queue-
  Filter - siehe [Warteschlangen](queues.md)).
- **Workflow.** Skalieren Sie horizontal; Row-Level-Claim/Heartbeat
  koordiniert die Worker.

## Verbindungs-Grenze (`SERVER_MAX_CONNECTIONS`)

Standardmäßig akzeptiert der Server eine unbegrenzte Anzahl von gleichzeitigen TCP-Verbindungen.
In den meisten Bereitstellungen bietet ein Reverse-Proxy (nginx, Caddy, Traefik)
oder der Load-Balancer der Plattform die erste Verteidigungslinie. Wenn
Sie einen harten Backstop innerhalb des Prozesses selbst möchten - um zu verhindern, dass ein einzelner
fehlerhafter Client-Pool Datei-Deskriptoren erschöpft - setzen Sie
`SERVER_MAX_CONNECTIONS`:

```bash
# .env.production - gleichzeitige Verbindungen auf 1024 begrenzen
SERVER_MAX_CONNECTIONS=1024
```

Wenn die Grenze erreicht wird, **blockiert die Accept-Schleife** (Back-Pressure auf der
TCP-Ebene) bis eine bestehende Verbindung geschlossen wird; der ausstehende Handshake
bleibt in dem Kernel-Accept-Backlog. Die Berechtigung wird für die vollständige
Lebensdauer jeder Verbindung gehalten und zum Moment der Verbindungsverschluss freigegeben,
damit Slots pünktlich wechseln.

Faustregeln:

- **Nicht gesetzt (Standard = unbegrenzt).** Korrekt wenn Sie einen Reverse-Proxy haben
  der sein eigenes Connection-Limit anwendet oder wenn Sie hinter einem PaaS laufen
  der Parallelität für Sie verwaltet.
- **Setzen Sie auf einen konkreten Wert** wenn der Prozess direkt auf dem
  Internet läuft oder Sie Defense-in-Depth wollen, unabhängig von der Proxy-Konfiguration.
  Ein typischer Startpunkt ist 2 × Ihre erwartete Peak-gleichzeitig Benutzer,
  nach oben für langlebige Verbindungen (WebSocket, SSE) angepasst.
- **Koppeln Sie mit `LimitNOFILE`** (systemd) oder `ulimit -n` sodass das OS
  Datei-Deskriptor-Limit nicht die Überraschungs-Grenze wird. Jede HTTP-Verbindung
  kostet einen Datei-Deskriptor; addieren Sie Ihre Datenbank-Pool-Größe und
  einige Dutzend für OS-Haushalt.
- **Das ist ein Backstop, kein Ersatz für Upstream-Rate-Limiting.**
  `SERVER_MAX_CONNECTIONS` stoppt unkontrolliertes Ansammeln; Ihr Reverse-Proxy
  oder `rate_limit` Middleware sollte Pro-Client- oder Pro-IP-Drosselung handhaben.

Leere, nicht parsbare oder Null-Werte werden stillschweigend als nicht gesetzt behandelt, daher ein
Tippfehler verhindert nicht, dass der Server startet.

## Plattformspezifische Anleitungen

Das obige Rezept portiert auf jede moderne PaaS oder VPS. Die nächsten drei
Kapitel gehen Sie durch die Specifika:

| Plattform | Stil | Anleitung |
|---|---|---|
| Railway | PaaS mit Auto-Deploy aus Git | [Zu Railway bereitstellen](deployment-railway.md) |
| Digital Ocean | App-Plattform (PaaS) oder Droplets (VPS) | [Zu Digital Ocean bereitstellen](deployment-digital-ocean.md) |
| Hetzner | VPS mit systemd + Caddy | [Zu Hetzner VPS bereitstellen](deployment-hetzner.md) |

## Nächste Schritte

- [Umgebungsvariablen](env-vars.md) - jede Umgebungsvariable, die das Framework liest
- [Verschlüsselung](encryption.md) - `APP_KEY`, Schlüsselrotation, was verschlüsselt ist
- [Konfiguration](configuration.md) - typisierte Konfigurationsabschnitte auf Basis von Umgebungsvariablen
- [Datenbank](database.md) - Treiberauswahl, Pool-Tuning, Multi-Connection-Split
- [Warteschlange](queues.md) - Worker-Skalierung und Queue-Treiber

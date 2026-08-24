# Docker

Suprnova liefert zwei CLI-Befehle aus, die Docker-Artefakte
generieren, die Sie unverändert übernehmen oder anpassen können.
`docker:init` schreibt ein mehrstufiges `Dockerfile`
+ `.dockerignore` für die Produktion. `docker:compose` schreibt eine
`docker-compose.yml` für lokale Entwicklungs-Services (Datenbank,
Cache und optional Mailpit + MinIO). Beide Befehle schreiben in das
aktuelle Projekt-Root; keiner von beiden versucht, Ihre
Container-Runtime zu steuern.

## docker:init

Generiert ein Produktions-Dockerfile zusammen mit einem passenden
`.dockerignore`.

```bash
suprnova docker:init
```

Der Befehl verweigert das Überschreiben eines bestehenden
`Dockerfile`; entfernen Sie die bestehende Datei zuerst, wenn Sie
neu generieren möchten.

### Was geschrieben wird

| Datei | Zweck |
|------|---------|
| `Dockerfile` | Dreistufiger Build: Frontend-Assets, kompilierte Rust-Binärdatei, Runtime-Image |
| `.dockerignore` | Schließt `target/`, `node_modules/`, `.env*`, die bestehenden Build-Artefakte und die Docker-Dateien selbst aus |

### Die Form des Dockerfiles

Das generierte Dockerfile verwendet drei Stufen, sodass das
Runtime-Image nur die kompilierte Binärdatei plus ihre benötigten
Shared Libraries trägt:

1. **`frontend-builder`** - `node:20-alpine`. Installiert
   npm-Abhängigkeiten und führt `npm run build` aus, was
   `frontend/dist` erzeugt.
2. **`backend-builder`** - `rust:1.94.0-slim-bookworm`. Zwischenspeichert `Cargo.toml`
   + `Cargo.lock` als Dependency-Layer, kopiert dann Ihr `cmd/`, `src/` und das gebaute
   `frontend/dist` (als `public/assets`) und führt `cargo build --release` aus.
3. **`runtime`** - `debian:bookworm-slim` mit `ca-certificates` und
   `libssl3`. Läuft als Non-Root-`appuser`. Kopiert die Binärdatei
   als `./app` hinein und das `public/`-Verzeichnis daneben. Stellt
   Port 8765 bereit.

Der Standard-`CMD` des finalen Images ist `["./app"]`, der das
`serve`-Subkommando der einheitlichen Binärdatei ausführt
(Web-Server mit Auto-Migrationen beim Start). Um ein anderes
Subkommando auszuführen, überschreiben Sie den Befehl zur
`docker run`-Zeit:

```bash
# Web-Server (Standard)
docker run -p 8765:8765 --env-file .env.production my-app

# Nur Migrationen ausführen und beenden
docker run --env-file .env.production my-app ./app migrate

# Den Scheduler-Daemon ausführen
docker run --env-file .env.production my-app ./app schedule:work

# Den Queue-Worker ausführen
docker run --env-file .env.production my-app ./app queue:work
```

Übergeben Sie die Produktions-Konfiguration über
`--env-file .env.production` oder einzelne `-e`-Flags.
`.env.production` sollte niemals committet werden - es ist bereits
durch die `.dockerignore` abgedeckt.

### Die Rust-Toolchain anheben

Das Dockerfile legt `rust:1.94.0-slim-bookworm` für die Build-Phase fest, damit ein neu erzeugtes Image reproduzierbar ist und dem aktuellen `main`-Branch entspricht. Benutzerdefinierte Dockerfiles sollten dieselbe oder eine neuere Toolchain verwenden.

```dockerfile
FROM rust:1.94.0-slim-bookworm AS backend-builder
```

Pinnen Sie auf die Toolchain-Version, die dem entspricht, was
`rust-toolchain.toml` (falls Sie eine haben) oder Ihr lokales
`rustc --version` meldet.


Der aktuelle `main`-Branch verwendet SeaORM 2.0, SeaQuery 1.0 und SQLx 0.9. Anwendungen, die SeaORM direkt aufrufen, müssen `ExprTrait` für SeaQuery-Ausdrucksmethoden importieren und explizite `*_raw`-Verbindungsmethoden für vorab erstellte `Statement`-Werte verwenden. Das Abhängigkeits-Upgrade erfordert keine Migration der Anwendungsdaten.

### Warum Suprnova abweicht

Laravel-Deployments führen typischerweise **mehrere Prozesse pro
Container oder Host** aus: php-fpm für Web, ein Queue-Worker, ein
Scheduler, manchmal ein Horizon-Dashboard, manchmal ein
Octane-Runner. Jeder ist seine eigene Service-Definition.

Suprnova kompiliert zu **einer statisch gelinkten Binärdatei**, die
jedes Subkommando kennt, das das Framework ausliefert - `serve`,
`migrate`, `queue:work`, `schedule:work`, `workflow:work`,
`ssr:start`. Dasselbe Docker-Image führt jede Rolle aus; das
Einzige, was sich ändert, ist der Befehl. Das macht „Web + Worker +
Scheduler“ zu drei Services in Ihrem Orchestrator, die alle auf
denselben Image-Tag zeigen - ein Build, um die gesamte App
vorwärtszubringen.

## docker:compose

Generiert eine `docker-compose.yml`, die lokale
Entwicklungs-Services hochfährt.

```bash
suprnova docker:compose [OPTIONS]
```

Wie `docker:init` verweigert dies das Überschreiben einer
bestehenden `docker-compose.yml`. Es hängt außerdem
`docker-compose.override.yml` an Ihre `.gitignore` an (falls eine
`.gitignore` vorhanden ist), sodass Sie Pro-Entwickler-Overrides
lokal behalten können, ohne sie zu committen.

### Optionen

| Option | Beschreibung |
|--------|-------------|
| `--with-mailpit` | Bindet den Mailpit-E-Mail-Test-Service ein |
| `--with-minio` | Bindet MinIO ein (S3-kompatibler Objektspeicher) |

Übergeben Sie keines der beiden Flags, fragt der Befehl interaktiv
nach beiden. Das Übergeben eines der beiden Flags überspringt die
Abfrage und verwendet die Flag-Werte, die Sie angegeben haben.

### Was Sie immer bekommen

PostgreSQL und Redis werden in jede generierte Compose-Datei
geschrieben:

| Service | Standard-Port | Image |
|---------|-------------:|-------|
| PostgreSQL | 5432 | `postgres:16-alpine` |
| Redis | 6379 | `redis:7-alpine` |

Beide Services haben Health Checks, persistente benannte Volumes,
und leben auf einem projektbezogenen Netzwerk (`<project>_network`).
Der Postgres-Benutzer, das Passwort und die Datenbank stehen
standardmäßig auf `suprnova` / `suprnova_secret` / `suprnova_db`.

### Optionale Services

Wenn Sie sich dafür entscheiden:

| Service | Standard-Ports | Image |
|---------|--------------:|-------|
| Mailpit | 1025 (SMTP), 8025 (UI) | `axllent/mailpit:latest` |
| MinIO | 9000 (S3 API), 9001 (Console) | `minio/minio:latest` |

Mailpit akzeptiert standardmäßig jede SMTP-Authentifizierung, sodass
Sie während der Entwicklung keine Credentials konfigurieren müssen;
die Web-UI unter `http://localhost:8025` zeigt jede E-Mail, die Ihre
App sendet. MinIOs Standard-Credentials sind `minioadmin` /
`minioadmin`.

### Den Stack ausführen

```bash
# Alles im Hintergrund hochfahren
docker compose up -d

# Logs verfolgen
docker compose logs -f

# Container stoppen und entfernen (Volumes bleiben erhalten)
docker compose down

# Auch Volumes entfernen (löscht die lokale Datenbank)
docker compose down -v
```

### `.env` mit Compose verdrahten

Die Compose-Datei verwendet überall die `${VAR:-default}`-Syntax,
sodass Sie alles überschreiben können, indem Sie es in `.env` oder
Ihrer Shell setzen. Eine typische `.env` für den Standard-Stack:

```env
DATABASE_URL=postgres://suprnova:suprnova_secret@localhost:5432/suprnova_db
REDIS_URL=redis://localhost:6379

# Mailpit (falls aktiviert)
MAIL_DRIVER=smtp
MAIL_HOST=localhost
MAIL_PORT=1025

# MinIO (falls aktiviert)
FILESYSTEM_DISK=s3
S3_ENDPOINT=http://localhost:9000
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_BUCKET=local
S3_REGION=us-east-1
```

Um einen Port zu überschreiben (z. B. weil 5432 schon belegt ist),
setzen Sie die passende Env-Var, bevor Sie den Stack hochfahren:

```bash
DB_PORT=5433 docker compose up -d
```

Die vollständige Menge überschreibbarer Ports:

| Variable | Service | Standard |
|----------|---------|--------:|
| `DB_PORT` | PostgreSQL | 5432 |
| `REDIS_PORT` | Redis | 6379 |
| `MAILPIT_SMTP_PORT` | Mailpit SMTP | 1025 |
| `MAILPIT_UI_PORT` | Mailpit UI | 8025 |
| `MINIO_API_PORT` | MinIO S3 | 9000 |
| `MINIO_CONSOLE_PORT` | MinIO Console | 9001 |

### Die Compose-Datei anpassen

`docker-compose.yml` gehört nach der Generierung Ihnen zum
Bearbeiten - Suprnova regeneriert oder liest sie später nicht.
Häufige Anpassungen:

- `postgres:16-alpine` gegen `mysql:8` oder `mariadb:11` tauschen,
  falls Sie einen dieser Treiber bevorzugen; beide sind in Suprnova
  erstklassig
- Einen `volumes:`-Eintrag hinzufügen, der Ihr
  `migrations/`-Verzeichnis mountet, falls Sie Migrationen innerhalb
  eines einmaligen Containers ausführen möchten
- Weitere Services (Qdrant, Elasticsearch, Nats) auf die gleiche Art
  hinzufügen

## Production-Deployment

Für eine echte Bereitstellung führen Sie `docker:init` aus und
behandeln das generierte `Dockerfile` als Ihren Build-Input. Die
meisten Orchestratoren (Railway, Fly, Digital Ocean App Platform,
Kubernetes) brauchen nur drei Dinge:

1. Den Image-Tag, der aus diesem `Dockerfile` gebaut wird
2. Eine Env-Datei mit `DATABASE_URL`, `APP_KEY` und allen
   treiberspezifischen Schlüsseln
3. Einen Health Check, der auf `GET /_suprnova/health/live` zeigt
   (und, falls die Plattform die beiden unterscheidet, einen
   Readiness Check auf `/_suprnova/health/ready`)

Die Single-Binary-Form bedeutet, dass jede Rolle dasselbe Image
verwendet; Sie deklarieren einen „web“-Service, der `./app`
ausführt, und einen „scheduler“- oder „worker“-Service, der
`./app schedule:work` (oder `./app queue:work`) ausführt. Beide
lesen dieselbe Env, sodass sie bei jedem Deploy im Gleichschritt
bleiben.

Siehe [Bereitstellung](deployment.md) für die
plattformunabhängige Checkliste, und die Plattform-Anleitungen für
vollständig durchgearbeitete Beispiele:
[Railway](deployment-railway.md),
[Digital Ocean](deployment-digital-ocean.md),
[Hetzner VPS](deployment-hetzner.md).

## Zusammenfassung

| Befehl | Schreibt | Wann verwenden |
|---------|--------|-------------|
| `suprnova docker:init` | `Dockerfile`, `.dockerignore` | Produktions-Images bauen |
| `suprnova docker:compose` | `docker-compose.yml` | Lokales Postgres/Redis/Mailpit/MinIO hochfahren |

## Nächste Schritte

- [Bereitstellung](deployment.md) - die plattformunabhängige
  Bereitstellungs-Checkliste
- [Railway](deployment-railway.md) - verwaltetes PaaS mit
  Build-from-Git
- [Digital Ocean](deployment-digital-ocean.md) - App-Platform-Deploys
- [Hetzner VPS](deployment-hetzner.md) - Bare-Metal mit systemd +
  Caddy
- [Umgebungsvariablen](env-vars.md) - jeder Schlüssel, den das
  Framework liest

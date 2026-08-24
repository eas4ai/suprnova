# Zu Hetzner VPS bereitstellen

Diese Anleitung behandelt die Bereitstellung einer Suprnova-Anwendung auf einem VPS mit Hetzner Cloud. Dieselben Prinzipien gelten für jeden Single-Box-Host - Linode, Vultr, AWS EC2 oder einen dedizierten Server, den Sie bereits besitzen. Wählen Sie diesen Weg, wenn Sie volle Kontrolle über die Box, planbare monatliche Kosten und die Möglichkeit wollen, Postgres / Redis gemeinsam auf derselben Maschine unterzubringen.

In dieser Anleitung verwenden wir `myapp` als Projektnamen und `myapp.com` als Domain - ersetzen Sie diese durch Ihre eigenen.

## Voraussetzungen

- Ein VPS mit Ubuntu 22.04 oder Debian 12
- SSH-Zugriff auf Ihren Server
- Ein Domainname, der auf die IP-Adresse Ihres Servers zeigt
- Ein Suprnova-Projekt - entweder ein funktionierender Quellbaum, oder ein mit `suprnova docker:init` generiertes Dockerfile (siehe [Docker](cli-docker.md))

## Server-Einrichtung

### 1. Einen VPS erstellen

1. Gehen Sie zur [Hetzner Cloud Console](https://console.hetzner.cloud)
2. Erstellen Sie ein neues Projekt und fügen Sie einen Server hinzu
3. Wählen Sie **Ubuntu 22.04** als Image
4. Wählen Sie Ihre Servergröße (CX11 reicht für kleine Apps)
5. Fügen Sie Ihren SSH-Schlüssel für sicheren Zugriff hinzu

### 2. Anfängliche Serverkonfiguration

Verbinden Sie sich per SSH mit Ihrem Server und führen Sie die erste Einrichtung aus:

```bash
# Pakete aktualisieren
apt update && apt upgrade -y

# Einen Non-Root-User für Ihre App erstellen
useradd -m -s /bin/bash app
mkdir -p /opt/myapp
chown app:app /opt/myapp

# Erforderliche Pakete installieren
apt install -y curl postgresql redis-server
```

### 3. PostgreSQL konfigurieren

```bash
# Datenbank und User erstellen
sudo -u postgres psql << EOF
CREATE USER myapp WITH PASSWORD 'your_secure_password';
CREATE DATABASE myapp_production OWNER myapp;
GRANT ALL PRIVILEGES ON DATABASE myapp_production TO myapp;
EOF
```

> **Tipp:**
>
> Erwägen Sie für die Produktion einen verwalteten Datenbank-Service wie Hetzners kommendes verwaltetes PostgreSQL, oder Services wie Neon, Supabase oder AWS RDS für bessere Zuverlässigkeit und Backups.


## Bereitstellungsoptionen

Wählen Sie eine der folgenden Bereitstellungsmethoden. Jede endet mit einer Binärdatei (oder einem Container) namens `app`, die unter `/opt/myapp/app` liegt und die die systemd-Unit unten auszuführen weiß.

### Option A: Lokal bauen

Bauen Sie auf Ihrer Maschine und laden Sie die Binärdatei hoch. Ersetzen Sie `myapp` durch Ihren tatsächlichen Projektnamen - `cargo build` benennt die Binärdatei nach dem `[package].name` in `Cargo.toml`:

```bash
# Auf Ihrer lokalen Maschine - Cross-Compile für Linux (falls auf macOS)
cargo build --release --target x86_64-unknown-linux-gnu

# Oder mit Docker für Linux bauen (das Dockerfile benennt die Binärdatei in `app` um)
docker build -t myapp .
docker create --name temp myapp
docker cp temp:/app/app ./app-linux
docker rm temp

# Auf den Server hochladen, dabei beim Landen in `app` umbenennen
scp target/x86_64-unknown-linux-gnu/release/myapp root@your-server:/opt/myapp/app
# oder, falls Sie den Docker-Weg gegangen sind:
scp ./app-linux root@your-server:/opt/myapp/app
```

### Option B: Auf dem Server bauen

Installieren Sie Rust 1.94.0+ für den aktuellen `main`-Branch (Suprnova verwendet die Edition 2024) und führen Sie den Build direkt auf dem Server aus:

```bash
# Rust installieren
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Klonen, bauen und die Binärdatei am Standardpfad ablegen
cd /opt/myapp
git clone https://github.com/your-username/your-repo.git .
cargo build --release
cp target/release/myapp ./app   # umbenennen, damit systemds ExecStart=/opt/myapp/app sie findet
```

Der aktuelle `main`-Branch verwendet SeaORM 2.0, SeaQuery 1.0 und SQLx 0.9. Anwendungen, die SeaORM direkt aufrufen, müssen `ExprTrait` für SeaQuery-Ausdrucksmethoden importieren und explizite `*_raw`-Verbindungsmethoden für vorab erstellte `Statement`-Werte verwenden. Das Abhängigkeits-Upgrade erfordert keine Migration der Anwendungsdaten.

### Option C: Docker verwenden

Führen Sie Ihre App in einem Docker-Container aus - das gescaffoldete Dockerfile benennt die Runtime-Binärdatei bereits `app` (siehe [Docker](cli-docker.md)):

```bash
# Docker installieren
curl -fsSL https://get.docker.com | sh

# Ihr Image pullen und ausführen
docker run -d \
  --name myapp \
  --restart unless-stopped \
  -p 8765:8765 \
  --env-file /opt/myapp/.env.production \
  your-registry/myapp:latest
```

Falls Sie sich für Docker entschieden haben, überspringen Sie den systemd-Abschnitt und gehen Sie zu [Caddy-Reverse-Proxy](#caddy-reverse-proxy) - Docker übernimmt die Prozessüberwachung.

## Umgebungskonfiguration

Generieren Sie zuerst einen Produktions-`APP_KEY` auf dem Server (oder lokal - der Wert ist, was zählt). `APP_KEY` ist ein 32-Byte-AES-256-Schlüssel, den `suprnova::Crypt` für Session-Cookies und signierte URLs verwendet. Suprnova **bricht beim Start ab**, wenn `APP_ENV` nicht `local`/`dev`/`test` ist und `APP_KEY` nicht gesetzt ist - das ist also in der Produktion nicht optional:

```bash
suprnova key:generate --show
# -> APP_KEY=base64-url-safe-32-bytes
```

Schreiben Sie dann die Env-Datei:

```bash
cat > /opt/myapp/.env.production << 'EOF'
APP_NAME="My App"
APP_ENV=production
APP_DEBUG=false
APP_URL=https://myapp.com
APP_KEY=paste-the-generated-key-here

SERVER_HOST=127.0.0.1
SERVER_PORT=8765

# Datenbank - an localhost binden, wenn die DB auf derselben Box läuft
DATABASE_URL=postgres://myapp:your_secure_password@localhost:5432/myapp_production
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1

# Session
SESSION_SECURE=true
SESSION_SAME_SITE=Lax

# Redis (optional - verwendet von Cache-, Queue-, Broadcasting-Treibern)
REDIS_URL=redis://127.0.0.1:6379

# Mail
MAIL_DRIVER=smtp
MAIL_HOST=your-smtp-host
MAIL_PORT=587
MAIL_USERNAME=
MAIL_PASSWORD=
MAIL_FROM_ADDRESS=hello@myapp.com
MAIL_FROM_NAME="My App"
EOF

# Die Datei sichern - nur der App-User sollte sie lesen können
chmod 600 /opt/myapp/.env.production
chown app:app /opt/myapp/.env.production
```

Siehe [Konfiguration](configuration.md) für die vollständige Env-Oberfläche und wie sie zu typisierter Konfiguration wird.

## systemd-Services

Eine Suprnova-Binärdatei unterstützt mehrere Befehle - `./app` (serve, mit Auto-Migration), `./app schedule:work` (Scheduler-Daemon), `./app queue:work` (Queue-Worker), `./app workflow:work` (Workflow-Runner). Jeder langlaufende Prozess bekommt seine eigene systemd-Unit, die dieselbe Binärdatei und Env-Datei verwendet.

### Web-Server-Service

Erstellen Sie `/etc/systemd/system/myapp.service`:

```ini
[Unit]
Description=Suprnova Application
After=network.target postgresql.service redis.service
Requires=postgresql.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app
Restart=always
RestartSec=5

# Umgebung
EnvironmentFile=/opt/myapp/.env.production

# Sicherheitshärtung
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

Der Standard `ExecStart=/opt/myapp/app` führt `serve` mit Auto-Migration aus. Wenn Sie Migrationen lieber als separaten Bereitstellungsschritt möchten, verwenden Sie `ExecStart=/opt/myapp/app serve --no-migrate` und führen Sie `./app migrate` aus Ihrem Bereitstellungsskript aus, bevor Sie die Binärdatei umschalten.

### Scheduler-Service

Wenn Ihre App über `Schedule::call(...)` registrierte Tasks hat (siehe das Kapitel [Planung](cli-scheduling.md)), führen Sie **genau einen** Scheduler-Prozess aus, um doppelte Task-Ausführung zu vermeiden. Erstellen Sie `/etc/systemd/system/myapp-scheduler.service`:

```ini
[Unit]
Description=Suprnova Scheduler
After=network.target myapp.service
Requires=myapp.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app schedule:work
Restart=always
RestartSec=5

# Umgebung
EnvironmentFile=/opt/myapp/.env.production

# Sicherheitshärtung
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

### Queue-Worker (optional)

Wenn Sie Jobs an eine Queue dispatchen, fügen Sie `/etc/systemd/system/myapp-queue.service` hinzu:

```ini
[Unit]
Description=Suprnova Queue Worker
After=network.target myapp.service
Requires=myapp.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app queue:work
Restart=always
RestartSec=5

EnvironmentFile=/opt/myapp/.env.production

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

Sie können Queue-Worker horizontal skalieren - mehrere `myapp-queue.service`-Instanzen auf derselben oder auf verschiedenen Boxen sind sicher.

### Services aktivieren und starten

```bash
# systemd nach dem Schreiben der Unit-Dateien neu laden
systemctl daemon-reload

# Services aktivieren, damit sie beim Boot starten
systemctl enable myapp
systemctl enable myapp-scheduler
systemctl enable myapp-queue        # falls Sie den Queue-Worker hinzugefügt haben

# Jetzt starten
systemctl start myapp
systemctl start myapp-scheduler
systemctl start myapp-queue

# Überprüfen
systemctl status myapp
systemctl status myapp-scheduler
systemctl status myapp-queue
```

## Caddy-Reverse-Proxy

Caddy handhabt HTTPS-Zertifikate automatisch mit Let's Encrypt.

### Caddy installieren

```bash
apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list
apt update
apt install caddy
```

### Caddy konfigurieren

Bearbeiten Sie `/etc/caddy/Caddyfile`:

```
myapp.com {
    reverse_proxy localhost:8765

    # Kompression aktivieren
    encode gzip

    # Logging
    log {
        output file /var/log/caddy/myapp.log
    }
}
```

Ersetzen Sie `myapp.com` durch Ihre tatsächliche Domain.

### Caddy starten

```bash
systemctl enable caddy
systemctl start caddy
```

Caddy holt und erneuert SSL-Zertifikate automatisch.

## Health Checks

Suprnova liefert einen eingebauten `/_suprnova/health`-Endpunkt, der vor der Middleware-Chain einen Short-Circuit macht und nie mit Ihren Routen kollidiert:

```bash
curl https://myapp.com/_suprnova/health
```

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z"
}
```

### Datenbankkonnektivität prüfen

Fügen Sie `?db=true` hinzu, um zusätzlich die Datenbank zu überprüfen:

```bash
curl https://myapp.com/_suprnova/health?db=true
```

Gesunde Antwort (HTTP 200):

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z",
  "database": "connected"
}
```

Falls die Datenbankprüfung fehlschlägt, wechselt der Endpunkt zu HTTP **503** mit `"status": "degraded"` und einem `"database_error"`-Feld - verdrahten Sie das in einen Health Check im Stil von `livenessProbe` / `readinessProbe`, damit der Load Balancer eine ungesunde Instanz aus der Rotation nehmen kann.

### Externes Monitoring

Verwenden Sie den Health-Endpunkt mit Monitoring-Services:

- **UptimeRobot**: HTTP-Monitor für `https://myapp.com/_suprnova/health` hinzufügen
- **Better Stack** (früher Better Uptime): Health-Check-Endpunkt mit dem 503-Trigger konfigurieren
- **Prometheus / Grafana**: Den JSON-Body nach den Feldern `status` + `database` scrapen

## Bereitstellungsskript

Erstellen Sie ein Bereitstellungsskript für atomare Updates. Ersetzen Sie `myapp` durch Ihren Projektnamen (das `[package].name` in `Cargo.toml`) - so benennt `cargo build` die Ausgabe-Binärdatei:

```bash
#!/bin/bash
# deploy.sh - auf Ihrer lokalen Maschine ausführen

set -e

PROJECT="myapp"               # der Cargo-Package-Name
SERVER="root@your-server"
APP_PATH="/opt/myapp"
BIN="target/x86_64-unknown-linux-gnu/release/$PROJECT"

echo "Anwendung wird gebaut..."
cargo build --release --target x86_64-unknown-linux-gnu

echo "Binärdatei wird hochgeladen..."
scp "$BIN" "$SERVER:$APP_PATH/app.new"

echo "Wird bereitgestellt..."
ssh "$SERVER" << 'EOF'
    set -e
    cd /opt/myapp

    # Langlaufende Services stoppen (Fehler beim ersten Deploy ignorieren)
    systemctl stop myapp-queue || true
    systemctl stop myapp-scheduler || true
    systemctl stop myapp

    # Atomarer Swap - Rename ist ein einzelner Syscall auf demselben Dateisystem
    mv app.new app
    chmod +x app

    # Migrationen explizit ausführen (die Unit auto-migriert auch, aber
    # es hier zu tun macht Fehler sichtbar, bevor wir Traffic neu starten)
    sudo -u app ./app migrate

    # Services starten
    systemctl start myapp
    systemctl start myapp-scheduler || true
    systemctl start myapp-queue || true

    # Health prüfen (dem Server einen Moment zum Binden geben)
    sleep 2
    curl -fsS http://localhost:8765/_suprnova/health?db=true > /dev/null || exit 1

    echo "Bereitstellung abgeschlossen!"
EOF
```

Machen Sie es ausführbar:

```bash
chmod +x deploy.sh
./deploy.sh
```

## Logs und Monitoring

### Logs ansehen

```bash
# Web-Server-Logs
journalctl -u myapp -f

# Scheduler-Logs
journalctl -u myapp-scheduler -f

# Caddy-Zugriffslogs
tail -f /var/log/caddy/myapp.log
```

### Log-Rotation

systemds journald handhabt Log-Rotation automatisch. Für Langzeitspeicherung erwägen Sie:

- **Loki + Grafana**: Selbst gehostete Log-Aggregation
- **Papertrail**: Cloud-basierter Logging-Service
- **Logtail**: Einfaches Log-Management

## Firewall-Konfiguration

Sichern Sie Ihren Server mit UFW:

```bash
# SSH erlauben
ufw allow 22/tcp

# HTTP/HTTPS erlauben (Caddy)
ufw allow 80/tcp
ufw allow 443/tcp

# Firewall aktivieren
ufw enable
```

> **Warnung:**
>
> Legen Sie Port 8765 niemals direkt offen. Verwenden Sie immer Caddy als Reverse-Proxy, um SSL und Security-Header zu handhaben.


## Skalierung

Eine einzelne Suprnova-Binärdatei ist sehr effizient - ein kleiner VPS bewältigt eine überraschende Menge Traffic, bevor Sie skalieren müssen. Wenn Sie es tun:

### Vertikale Skalierung

Upgraden Sie den VPS auf eine größere Instanz für mehr CPU/Speicher. Die Binärdatei, die Env-Datei und die systemd-Units kommen unverändert mit.

### Horizontale Skalierung

Für mehrere Anwendungsinstanzen:

1. Richten Sie einen Load Balancer ein (Hetzner Load Balancer, HAProxy oder Caddy auf einem dedizierten Node)
2. Verschieben Sie Postgres auf einen verwalteten Service oder einen dedizierten Node, damit die App-Boxen zustandslos sind
3. Verschieben Sie Sessions, Cache und Broadcasting zu Redis, damit jede App-Instanz jede Anfrage bedienen kann
4. Stellen Sie mehrere App-Instanzen bereit; jede führt sicher ihre eigene Auto-Migration beim Start aus (der Migrations-Runner nimmt eine Sperre, damit gleichzeitige Starts nicht kollidieren)
5. Halten Sie **einen** Scheduler (`schedule:work`) über die gesamte Flotte hinweg am Laufen - Queue-Worker können gefahrlos parallel laufen, der Scheduler nicht

### Warum Suprnova abweicht

Laravel führt typischerweise PHP-FPM hinter nginx aus, wobei Cron einmal pro Minute `schedule:run` auslöst und Horizon (oder supervisord) die Queue-Worker verwaltet. Suprnova fasst das in einer Binärdatei mit Subkommandos zusammen. `./app` ist ein langlebiger Tokio-Prozess - er braucht keinen Process-Pool davor, braucht keinen separaten Cron und bleibt über Anfragen hinweg warm. systemd ist der Supervisor sowohl für den Web-Prozess als auch für die Worker, und Caddy tut nur das, was nginx nicht vermeiden konnte: TLS terminieren und proxyen.

## Dimensionierung

Wählen Sie einen VPS basierend auf der Workload, nicht auf einem Marketing-Tier-Namen. Hetzners Lineup ändert sich regelmäßig; die Dimensionierungslogik nicht:

| Workload | Grobe Passung |
|---|---|
| Kleine Site, wenig Traffic, SQLite oder gemeinsame DB | Kleinste Shared-vCPU-Instanz (1 vCPU / 2 GB) |
| Moderater Traffic mit Postgres + Redis auf derselben Box | 2 vCPU / 4 GB |
| Schwerere API + Scheduler + Queue-Worker + Postgres | 2–4 vCPU / 8 GB |
| Produktion im großen Maßstab | Dedizierte CPU-Instanz, oder DB auf einen eigenen Node aufteilen |

Prüfen Sie Hetzners [aktuelle Preise](https://www.hetzner.com/cloud) für den aktuellen Katalog. Suprnovas Idle-Speicherbedarf ist klein (einstellige MB), sodass RAM größtenteils aus dem Working Set der Datenbank plus Ihrem Domain-Code besteht.

## Fehlerbehebung

### Service startet nicht

Prüfen Sie die Logs auf Fehler:

```bash
journalctl -u myapp -n 50
```

Häufige Probleme:
- Fehlende Umgebungsvariablen
- Datenbankverbindung fehlgeschlagen
- Port bereits belegt

### Caddy-Zertifikatsfehler

Stellen Sie sicher:
- Domain-DNS zeigt auf Ihren Server
- Ports 80 und 443 sind offen
- Kein anderer Service verwendet Port 80

```bash
caddy validate --config /etc/caddy/Caddyfile
```

### Datenbankverbindungsprobleme

Testen Sie die Verbindung manuell:

```bash
sudo -u app psql $DATABASE_URL -c "SELECT 1"
```

### Health Check schlägt fehl

```bash
# Prüfen, ob die App läuft
systemctl status myapp

# Health-Endpunkt direkt testen
curl http://localhost:8765/_suprnova/health

# Mit Datenbank prüfen
curl http://localhost:8765/_suprnova/health?db=true
```

Eine `503`-Antwort mit `"status": "degraded"` bedeutet, dass die App läuft, aber der Datenbank-Health-Check fehlgeschlagen ist - untersuchen Sie `database_error` im Body und prüfen Sie `DATABASE_URL`, die Postgres-Logs und die Verbindungslimits.

## Nächste Schritte

- [Bereitstellung - Übersicht](deployment.md) - die plattformunabhängige Geschichte für Single-Binary-Bereitstellungen
- [Docker](cli-docker.md) - Details zu `docker:init` und `docker:compose`
- [Konfiguration](configuration.md) - vollständige Env-Oberfläche und typisierte Konfiguration
- [Zu Railway bereitstellen](deployment-railway.md) - PaaS-Alternative mit automatischen Builds
- [Zu Digital Ocean bereitstellen](deployment-digital-ocean.md) - App Platform mit verwalteter Infrastruktur

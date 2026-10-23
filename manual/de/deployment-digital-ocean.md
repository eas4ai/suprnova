# Zu Digital Ocean bereitstellen

Digital Ocean hat zwei Produktionsziele, die zu einer Suprnova-App
passen: **App Platform** (ein verwaltetes Docker-PaaS - pushen und
vergessen) und ein **Droplet** (Ihr eigener VPS, Sie verwalten alles
selbst). Dieses Kapitel geht beide durch. Verwenden Sie App Platform,
wenn Sie verwaltete Datenbanken, automatische Bereitstellungen und für
Sie erledigtes SSL möchten. Verwenden Sie ein Droplet, wenn Sie volle
Kontrolle wollen, bereits andere Services auf der Box laufen haben,
oder die Rechnung unabhängig vom Traffic konstant halten wollen.

## Voraussetzungen

- Ein [Digital-Ocean-Konto](https://www.digitalocean.com)
- Ein Suprnova-Projekt mit einem Dockerfile - generieren Sie eines
  mit:
  ```bash
  suprnova docker:init
  ```
- Ein `APP_KEY` für die Produktion. Generieren Sie einen und bewahren
  Sie ihn sicher auf:
  ```bash
  suprnova key:generate --show
  ```
  Suprnova bricht beim Start ab, wenn `APP_ENV` etwas anderes als
  `local` / `development` / `testing` ist und `APP_KEY` nicht gesetzt
  ist.
- Ein Git-Repository (GitHub oder GitLab) - erforderlich für App
  Platform; für Droplets können Sie auch ein vorgebautes Image zu
  einer Registry pushen.

## App Platform

App Platform baut Ihr Dockerfile, führt die einzelne
Suprnova-Binärdatei aus und gibt Ihnen ein verwaltetes Postgres, wenn
Sie eines möchten.

### 1. Die App erstellen

1. Gehen Sie zu [Digital Ocean Apps](https://cloud.digitalocean.com/apps).
2. Klicken Sie auf **Create App**, verbinden Sie GitHub/GitLab und
   wählen Sie das Repo und den Branch.
3. App Platform erkennt das `Dockerfile` im Repo-Root automatisch.

### 2. Den Web-Service konfigurieren

| Setting | Value |
|---|---|
| Resource-Typ | Web Service |
| HTTP-Port | `8765` |
| Run-Befehl | leer lassen - der `CMD` des Dockerfiles führt `./app` aus |
| Health Check (HTTP-Pfad) | `/_suprnova/health/live` |

Die Standard-Suprnova-Binärdatei führt `serve` mit Auto-Migrationen
aus, sodass der Container beim Start Migrationen ausführt und dann
den Listener bindet.

### 3. Ein verwaltetes Postgres hinzufügen

1. **Add Resource** -> **Database** -> **PostgreSQL**.
2. Wählen Sie einen Plan (Dev Database zum Testen; einen
   Production-Plan für echten Traffic).

App Platform injiziert `DATABASE_URL` automatisch in jede Komponente
über die `${db.DATABASE_URL}`-Bindung.

### 4. Umgebungsvariablen

Setzen Sie im Abschnitt **Environment Variables** Ihrer Web-Komponente:

| Variable | Value | Hinweise |
|---|---|---|
| `APP_ENV` | `production` | löst die Fail-closed-Prüfung von `APP_KEY` aus |
| `APP_KEY` | Ausgabe von `suprnova key:generate --show` | als **encrypted** markieren |
| `SERVER_HOST` | `0.0.0.0` | an alle Schnittstellen binden |
| `SERVER_PORT` | `8765` | entspricht dem `EXPOSE` des Dockerfiles |
| `APP_URL` | `https://your-app.ondigitalocean.app` | wird von Inertia + signierten URLs verwendet |

`DATABASE_URL` wird automatisch von der verwalteten Datenbank-Bindung
bereitgestellt; setzen Sie es nicht manuell.

Wenn Sie Redis für Cache/Sessions verwenden, fügen Sie einen
verwalteten Redis-Cluster hinzu und setzen Sie `REDIS_URL` auf dessen
Bindungswert (`${redis.REDIS_URL}`).

### 5. Bereitstellen

Klicken Sie auf **Create Resources**. Der erste Build dauert ein paar
Minuten (Rust-Release-Build + Frontend-Build); nachfolgende Builds
nutzen den Dockerfile-Layer-Cache und laufen viel schneller.

### Einen Scheduler-Worker hinzufügen

Geplante Tasks (`#[derive(Task)]`-Handler, registriert über
`Schedule::call`) brauchen einen langlebigen Prozess. Fügen Sie eine
Worker-Komponente hinzu, die dasselbe Image mit einem anderen Befehl
ausführt:

1. **Create** -> **Add Resource** -> **Detect from source code**,
   wählen Sie dasselbe Repository.
2. Setzen Sie den Resource-Typ auf **Worker**.
3. **Run command**:
   ```bash
   ./app schedule:work
   ```
4. Der Worker erbt Umgebungsvariablen von der App, einschließlich
   `DATABASE_URL` und `APP_KEY`.

Worker empfangen keinen HTTP-Traffic. Führen Sie genau **eine**
Worker-Instanz aus - mehrere Scheduler würden jeden Task mehrfach
ausführen.

Für Queue-Worker (`./app queue:work`) ist das Muster identisch; Sie
können normalerweise gefahrlos mehr als einen Queue-Worker ausführen,
weil der Queue-Treiber koordiniert, welcher Worker welchen Job
übernimmt. Siehe [Warteschlange](queues.md).

### App-Spezifikation (Infrastructure as Code)

Für wiederholbare Bereitstellungen committen Sie ein `.do/app.yaml`:

```yaml
name: my-suprnova-app

services:
  - name: web
    dockerfile_path: Dockerfile
    github:
      repo: your-username/your-repo
      branch: main
      deploy_on_push: true
    http_port: 8765
    instance_count: 1
    instance_size_slug: basic-xxs
    health_check:
      # Nur Liveness - App Platform startet den Container neu, wenn
      # das fehlschlägt, also darf es nicht von Postgres abhängen.
      # Siehe den Hinweis zum Health Check unter Fehlerbehebung.
      http_path: /_suprnova/health/live
    envs:
      - key: APP_ENV
        value: production
      - key: APP_KEY
        scope: RUN_TIME
        type: SECRET
        value: ${APP_KEY}
      - key: SERVER_HOST
        value: 0.0.0.0
      - key: SERVER_PORT
        value: "8765"
      - key: APP_URL
        value: https://your-app.ondigitalocean.app
      - key: DATABASE_URL
        scope: RUN_TIME
        value: ${db.DATABASE_URL}

workers:
  - name: scheduler
    dockerfile_path: Dockerfile
    github:
      repo: your-username/your-repo
      branch: main
      deploy_on_push: true
    instance_count: 1
    instance_size_slug: basic-xxs
    run_command: ./app schedule:work
    envs:
      - key: APP_ENV
        value: production
      - key: APP_KEY
        scope: RUN_TIME
        type: SECRET
        value: ${APP_KEY}
      - key: DATABASE_URL
        scope: RUN_TIME
        value: ${db.DATABASE_URL}

databases:
  - name: db
    engine: PG
    version: "16"
    size: db-s-dev-database
```

Stellen Sie mit der `doctl`-CLI bereit:

```bash
doctl apps create --spec .do/app.yaml
```

Setzen Sie das Secret `APP_KEY` separat über die Apps-UI oder:

```bash
doctl apps update <app-id> --spec .do/app.yaml \
  --set-env "APP_KEY=$(suprnova key:generate --show)"
```

### Eigene Domain

Geben Sie unter **Settings** -> **Domains** -> **Add Domain** Ihre
Domain ein und folgen Sie den DNS-Anweisungen. App Platform stellt
automatisch ein Let's-Encrypt-Zertifikat aus und erneuert es.

Nachdem die Domain live ist, aktualisieren Sie `APP_URL` entsprechend -
Inertia verwendet es für den X-Inertia-Location-Header, und signierte
URLs verwenden es für die Hash-Eingabe.

### Skalierung

- **Horizontal**: Erhöhen Sie **Instance Count** beim Web-Service.
  Jede Instanz teilt sich das verwaltete Postgres; dass mehrere
  Instanzen beim Start Auto-Migrationen ausführen, ist sicher -
  Suprnova verwendet SeaORMs mit einem Advisory Lock abgesicherten
  Migrator.
- **Vertikal**: Ändern Sie **Instance Size**. Die Rust-Binärdatei ist
  auf dem kleinsten Slug für Apps mit wenig Traffic zufrieden; erhöhen
  Sie sie, wenn Sie im großen Stil WebSockets oder langlebige
  Verbindungen bedienen.

Halten Sie den Scheduler-Worker bei Instance Count **1**.

## Droplet (VPS)

Ein Droplet ist der Weg, wenn Sie Suprnova auf Ihrem eigenen VPS
betreiben möchten. Die Mechanik ist identisch zu jedem anderen
Linux-VPS - systemd-Service, Caddy-Reverse-Proxy, verwaltetes oder
selbst gehostetes Postgres. Das Kapitel
[Hetzner VPS](deployment-hetzner.md) ist der kanonische Walkthrough
für dieses Muster; alles dort gilt wortwörtlich auch für ein Droplet.
Die einzigen erwähnenswerten Unterschiede:

- **Image**: Wählen Sie **Ubuntu 24.04** oder **Debian 12** in der
  Droplet-Konsole.
- **Datenbank**: Sie können Digital Oceans **Managed Databases** für
  Postgres / MySQL / Redis verwenden, statt sie auf dem Droplet
  laufen zu lassen - dieselbe `DATABASE_URL`- / `REDIS_URL`-Geschichte,
  richten Sie sie auf den verwalteten Endpunkt, und Suprnova merkt den
  Unterschied nicht.
- **Backups**: Aktivieren Sie Droplet-Snapshots und tägliche Backups
  der verwalteten DB in der DO-Konsole.
- **Networking**: Verwenden Sie ein DO-**VPC**, um das Droplet und
  alle verwalteten Datenbanken in einem privaten Netzwerk zu halten;
  binden Sie den Listener an `127.0.0.1` und stellen Sie Caddy davor
  für TLS.

Wenn Sie Docker auf einem Droplet möchten (statt einer
System-Binärdatei), passt das Docker-Compose-Muster aus
[Docker](cli-docker.md) sauber hinein - tauschen Sie das selbst
gehostete Postgres gegen die verwaltete Datenbank aus, und Sie sind
fertig.

### Warum Suprnova abweicht

Laravels typische PHP-Bereitstellung braucht PHP-FPM + einen Opcache +
einen Queue-Runner + einen Scheduler-Cron-Eintrag - mindestens drei
bewegliche Teile, jedes mit eigener Neustart-Semantik. Eine
Suprnova-Bereitstellung ist eine einzelne Binärdatei plus ein
optionaler Worker-Prozess. Die Binärdatei führt Migrationen aus,
bedient HTTP, handhabt WebSockets und lebt hinter einem Reverse-Proxy.
Dieselbe Binärdatei, aufgerufen mit `./app schedule:work` oder
`./app queue:work`, ist Ihr Scheduler oder Queue-Worker. Das Modell
„ein Image, mehrere Komponenten“ von App Platform passt dazu ganz
natürlich - dasselbe Dockerfile für jede Komponente, unterschiedlicher
`run_command` pro Rolle.

## Fehlerbehebung

### Build schlägt fehl

Das Erste, was Sie prüfen sollten, ist, ob das Dockerfile lokal baut:

```bash
docker build -t myapp .
```

Häufige Ursachen, wenn der lokale Build funktioniert, aber der von App
Platform nicht:

- **Fehlende Build-Kontext-Dateien**: Prüfen Sie, dass `.dockerignore`
  nicht `Cargo.lock` oder das Verzeichnis `migrations/` ausschließt.
- **Out-of-Memory während des Cargo-Builds**: Erhöhen Sie die
  Build-Instance-Größe unter App Settings -> Resources -> Build.
  Rust-Release-Builds sind speicherhungrig.

### App startet und stürzt dann beim Hochfahren ab

Prüfen Sie die Runtime-Logs im Tab **Runtime Logs**. Die zwei
häufigsten Suprnova-Boot-Fehler sind:

- **`APP_KEY is required when APP_ENV=production`** - generieren Sie
  einen mit `suprnova key:generate --show` und fügen Sie ihn als
  verschlüsselte Umgebungsvariable hinzu.
- **`SERVER_HOST=…`-Wert ungültig** - muss für App Platform `0.0.0.0`
  sein, nicht `127.0.0.1` (das Loopback ist vom Load Balancer aus
  nicht erreichbar).

### Health Check schlägt fehl

Die Plattform pingt `/_suprnova/health/live` an und erwartet einen 200
innerhalb des konfigurierten Timeouts. Falls das fehlschlägt:

- Bestätigen Sie, dass der Pfad exakt `/_suprnova/health/live` ist
  (nicht `/health`). Das ältere `/_suprnova/health` funktioniert
  weiterhin, falls das ist, was Ihre Spezifikation bereits nennt.
- Bestätigen Sie, dass der Port `8765` ist und `SERVER_PORT`
  entspricht.
- Um „kann nicht binden“ von „kann Postgres nicht erreichen“ zu
  unterscheiden, prüfen Sie die Datenbank **von Hand** über die
  Konsole statt über den Health Check:

  ```bash
  curl http://localhost:8765/_suprnova/health/ready
  # Gesund:        200 {"status":"ok","database":"connected"}
  # Eingeschränkt: 503 {"status":"degraded","database":"error"}
  ```

  Eine eingeschränkte Antwort bedeutet, dass die App gebunden hat,
  aber Postgres nicht erreichen kann - prüfen Sie die
  `DATABASE_URL`-Bindung. Übergeben Sie nicht `-f`: Das lässt curl bei
  der 503 stillschweigend beenden, was genau der Fall ist, den Sie zu
  lesen versuchen.

Setzen Sie die Datenbank-Probe nicht in den `health_check` der
App-Spezifikation. App Platform startet den Container neu, wenn dieser
Check fehlschlägt, sodass ein Datenbank-Aussetzer die App mit sich
reißen würde - der Fehlermodus ist eine Neustart-Schleife genau
während des Vorfalls, den die App überleben soll. Siehe [Verwenden Sie
die richtige Probe für die richtige
Frage](deployment.md#use-the-right-probe-for-the-right-question).

### Datenbank-Migrationen laufen nicht

Migrationen laufen automatisch als Teil des Standard-`./app`-Boots.
Falls nicht, prüfen Sie die Runtime-Logs auf SeaORM-Fehler. Um sie
manuell über die App-Platform-Konsole auszuführen:

1. Öffnen Sie den Tab **Console** bei der Web-Komponente.
2. Führen Sie `./app migrate` aus.

Wenn Sie Migrationen lieber aus dem Boot-Pfad heraushalten möchten,
setzen Sie den Run-Befehl auf `./app serve --no-migrate` und fügen Sie
der App-Spezifikation einen einmaligen **Job** hinzu, der
`./app migrate` vor der Bereitstellung ausführt.

## Nächste Schritte

- [Bereitstellung - Übersicht](deployment.md) - die
  plattformübergreifende Bereitstellungs-Einführung (Binärdatei,
  Migrationen, Scheduler, Health)
- [Docker](cli-docker.md) - was `suprnova docker:init` und
  `docker:compose` generieren
- [Konfiguration](configuration.md) - jede Umgebungsvariable, die
  Suprnova liest
- [Umgebungsvariablen](env-vars.md) - vollständige Referenz,
  einschließlich der in der Produktion erforderlichen
- [Zu Hetzner VPS bereitstellen](deployment-hetzner.md) - der
  Droplet-Walkthrough gilt hier wortwörtlich

# Zu Railway bereitstellen

[Railway](https://railway.app) ist ein Git-gesteuertes PaaS, das Ihr
Dockerfile baut und auf verwalteter Infrastruktur ausführt. Kombinieren
Sie es mit Railways verwaltetem Postgres und Redis, und Sie haben
einen kompletten Suprnova-Produktionsstack ohne Server, um die Sie
sich kümmern müssen. Dieses Rezept bringt eine frisch gescaffoldete
App von `suprnova new` bis zu einer Live-URL.

## Voraussetzungen

- Ein [Railway-Konto](https://railway.app)
- Ein Suprnova-Projekt, gepusht zu GitHub, GitLab oder Bitbucket
- Ein `Dockerfile` und `.dockerignore` im Repo-Root, generiert mit:
  ```bash
  suprnova docker:init
  ```
- Ein generierter `APP_KEY`, den Sie in Railways Variablen einfügen
  können:
  ```bash
  suprnova key:generate --show
  ```

`suprnova` wird nur lokal gebraucht - Railway baut das Dockerfile
selbst. Die Framework-Crate wird während des Builds als normale
Cargo-Abhängigkeit aus Git gezogen.

## Das Projekt einrichten

1. Öffnen Sie das [Railway-Dashboard](https://railway.app/dashboard),
   klicken Sie auf **New Project** und wählen Sie
   **Deploy from GitHub repo**.
2. Wählen Sie das Repository aus. Railway erkennt das `Dockerfile` und
   startet automatisch den ersten Build.
3. Fügen Sie während des Builds eine Datenbank hinzu: **New** →
   **Database** → **Add PostgreSQL**. Railway macht `DATABASE_URL` als
   Referenzvariable im Projekt verfügbar.
4. Fügen Sie optional auf die gleiche Weise Redis hinzu (**New** →
   **Database** → **Redis**), falls Ihre App den Redis-Treiber für
   Cache, Session, Queue oder Rate-Limit verwendet. Railway macht die
   Verbindungs-URL als `REDIS_URL` verfügbar.

## Die Variablen verdrahten

Öffnen Sie den Web-Service, gehen Sie zu **Variables** und fügen Sie
die Produktionskonfiguration hinzu. Verwenden Sie Railways
`${{ }}`-Referenzsyntax, um URLs aus den Datenbank-Services zu ziehen,
damit Rotationen kein erneutes Einfügen erfordern.

```env
APP_ENV=production
APP_KEY=<paste the output of `suprnova key:generate --show`>
SERVER_HOST=0.0.0.0
SERVER_PORT=8765
DATABASE_URL=${{ Postgres.DATABASE_URL }}
REDIS_URL=${{ Redis.REDIS_URL }}
```

Ein paar Dinge, die gut zu wissen sind:

- **`APP_KEY` ist in Nicht-Entwicklungsumgebungen zwingend
  erforderlich.** Suprnova bricht beim Start ab, wenn
  `APP_ENV != local|dev|test` ist und `APP_KEY` fehlt oder fehlerhaft
  ist. Der Server protokolliert eine Behebungsmeldung und beendet sich
  mit Non-Zero - Railway markiert die Bereitstellung als
  fehlgeschlagen. Generieren Sie den Schlüssel mit
  `suprnova key:generate --show`.
- **`SERVER_HOST=0.0.0.0` ist erforderlich.** Railway leitet Traffic
  durch die Netzwerkschnittstelle des Containers; eine Bindung an
  `127.0.0.1` (den lokalen Standard) sieht wie eine verweigerte
  Verbindung aus.
- **`SERVER_PORT` entspricht `EXPOSE` im Dockerfile.** Das generierte
  Dockerfile exponiert 8765. Railway ordnet es automatisch einer
  öffentlichen URL zu.

## Bauen und bereitstellen

Railway baut bei jedem Push auf den verbundenen Branch. Das von
`docker:init` generierte Dockerfile macht Folgendes:

1. **Stufe 1 - Frontend.** Führt `npm ci` und `npm run build` in
   `frontend/` aus. Die Vite-Ausgabe landet in `frontend/dist/`.
2. **Stufe 2 - Backend.** Führt `cargo build --release` gegen Ihren
   Workspace aus; gecachte Dependency-Layer halten iterative Builds
   schnell.
3. **Stufe 3 - Runtime.** Ein `debian:bookworm-slim`-Image mit
   `ca-certificates` + `libssl3`, einem Non-Root-`appuser` und der
   kompilierten `./app`-Binärdatei. Der Standard-`CMD` ist `./app`,
   der `serve` mit Auto-Migration ausführt.

Der erste Build dauert typischerweise mehrere Minuten (kalter
Rust-Cache); nachfolgende Builds sind dank Docker-Layer-Caching viel
schneller.

## Einen Scheduler-Service hinzufügen

Wenn Ihre App `#[derive(Task)]`-Zeitpläne verwendet, braucht der
Scheduler seinen eigenen langlaufenden Prozess. Fügen Sie einen
zweiten Service aus demselben Repo hinzu:

1. **New** → **GitHub Repo** → dasselbe Repository auswählen.
2. Benennen Sie ihn `scheduler`, damit er im Dashboard leicht zu
   finden ist.
3. Setzen Sie unter **Settings** → **Deploy** den **Custom Start
   Command** auf:
   ```bash
   ./app schedule:work
   ```
4. Kopieren Sie dieselben Variablen (insbesondere `APP_KEY` und die
   Datenbank-Referenzen), damit der Worker dieselbe Konfiguration wie
   der Web-Service liest.

`schedule:work` ist eine Daemon-Schleife - sie wacht einmal pro Minute
auf, fragt den Zeitplan nach fälligen Tasks ab und führt sie über
denselben Bootstrap aus wie der HTTP-Server. Siehe [Konsole](console.md)
und das Scheduler-Kapitel für den Vertrag.

Führen Sie genau eine Scheduler-Instanz aus. Mehrere
`schedule:work`-Prozesse koordinieren sich über Cache-gestützte
Sperren, aber die Standarderwartung ist ein einzelner Worker.

### Warum Suprnova abweicht

Eine Laravel-Bereitstellung auf Forge oder Vapor verdrahtet
typischerweise einen Webserver (php-fpm + nginx), einen Queue-Worker
(`php artisan queue:work`) und einen Cron-Eintrag, der jede Minute
`schedule:run` aufruft. Drei Komponenten, drei
Bereitstellungsoberflächen.

Suprnova kompiliert jede Rolle in dieselbe Binärdatei. Die
Railway-Service-Spezifikation ist `./app` für die Web-Rolle und
`./app schedule:work` für den Scheduler - dasselbe Image, derselbe
Bootstrap, unterschiedliches argv. Es gibt keinen separaten
php-fpm-Container, kein separates Worker-Image, keinen Host-Cron.
Fügen Sie `./app queue:work` als dritten Service hinzu, falls Sie
Queue-Jobs haben, und Sie haben die vollständige Laravel-Topologie in
drei Railway-Services aus einem Dockerfile.

## Health Checks und `railway.json`

Für mehr Kontrolle über die Bereitstellung committen Sie ein
`railway.json` in den Repo-Root. Railway übernimmt es automatisch.

```json
{
  "$schema": "https://railway.app/railway.schema.json",
  "build": {
    "builder": "DOCKERFILE",
    "dockerfilePath": "Dockerfile"
  },
  "deploy": {
    "startCommand": "./app",
    "healthcheckPath": "/_suprnova/health/live",
    "healthcheckTimeout": 300,
    "restartPolicyType": "ON_FAILURE",
    "restartPolicyMaxRetries": 10
  }
}
```

Suprnova liefert eingebaute Health-Endpunkte, die vor der
Middleware-Chain einen Short-Circuit machen - sie liefern einen
200-JSON-Status zurück, ohne durch Auth, CSRF oder Rate-Limiting zu
gehen. Das Präfix `/_suprnova/` ist reserviert, damit sie nie mit
Ihren Routen kollidieren.

`healthcheckPath` zeigt oben auf `/_suprnova/health/live`, was nichts
berührt. Diese Paarung ist beabsichtigt: Dieser Service ist mit
`"restartPolicyType": "ON_FAILURE"` konfiguriert, also ist alles, was
der Health Check prüft, ein Neustart-Auslöser. Ihn auf die Datenbank
zu richten - über `/_suprnova/health/ready` oder das ältere
`/_suprnova/health?db=true` - bedeutet, dass ein Datenbank-Aussetzer
jede Replik in genau dem Moment neu startet, in dem die Datenbank sich
einen Reconnect-Sturm am wenigsten leisten kann. Prüfen Sie die
Datenbank über einen separaten Readiness-Check oder Ihr Monitoring,
nicht über den Pfad, der den Prozess neu startet. Siehe [Verwenden Sie
die richtige Probe für die richtige
Frage](deployment.md#use-the-right-probe-for-the-right-question).

Beide älteren Pfade funktionieren weiterhin, sodass ein bestehender
Railway-Service keine Änderung braucht; die benannten Pfade sind
einfach klarer.

## Eigene Domains und TLS

1. Öffnen Sie im Web-Service **Settings** → **Networking**.
2. Klicken Sie auf **Generate Domain** für eine
   `*.up.railway.app`-Subdomain, oder auf **Custom Domain**, um Ihren
   eigenen Hostnamen auf den Service zu richten.
3. Aktualisieren Sie DNS wie von Railway angewiesen (ein `CNAME` für
   Subdomains, ein ANAME/ALIAS für Apex-Domains).

Railway stellt Let's-Encrypt-Zertifikate für sowohl generierte als
auch eigene Domains bereit und erneuert sie.

## Migrationen in CI/CD

Der Standard-`CMD ["./app"]` führt Migrationen beim Start aus, was für
Bereitstellungen mit einer Instanz in Ordnung ist. Für
Multi-Replika-Setups entkoppeln Sie den Migrationsschritt:

1. Fügen Sie einen einmaligen **Pre-Deploy-Hook** hinzu, der
   `./app migrate` gegen die Produktionsdatenbank ausführt, bevor die
   neuen Repliken starten.
2. Ändern Sie den Runtime-Start-Befehl auf `./app serve --no-migrate`,
   damit es zwischen den Repliken kein Race gibt.

Der Migrations-Runner ist idempotent - selbst wenn Sie die Schritte
nicht aufteilen, ist es über Repliken hinweg sicher, Migrationen bei
jedem Start auszuführen. Die Aufteilung existiert, damit Sie die
Bereitstellung bei einer fehlerhaften Migration früh scheitern lassen
können, ohne den Rollout offen zu halten.

## Logs, Metriken, Rollbacks

Der Web-Service-Tab zeigt:

- **Deployments** - jeder Build in chronologischer Reihenfolge; das
  Drei-Punkte-Menü an einer früheren erfolgreichen Bereitstellung ist
  der Ein-Klick-Rollback-Pfad
- **Logs** - `tracing`-Ausgabe aus dem Container, mit
  Structured-Log-Feldern (`request_id`, `route`, `status`), bereit für
  die Filter des Log-Viewers
- **Metrics** - CPU, Speicher, Netzwerk-IO; nützlich, um die Instanz
  hoch- oder herunterzuskalieren

## Fehlerbehebung

**Build schlägt bei `cargo build --release` fehl.** Reproduzieren Sie
lokal mit `docker build -t myapp .`. Die häufigste Ursache ist ein
Workspace-Mitglied, das auf Ihrer Maschine kompiliert, aber im Repo
fehlt - das Dockerfile kopiert zuerst `Cargo.toml` und `Cargo.lock`,
sodass fehlende Crates sichtbar scheitern.

**App liefert „connection refused“.** Prüfen Sie, dass
`SERVER_HOST=0.0.0.0` beim Service gesetzt ist. Der Standard ist
`127.0.0.1`, wohin Railway nicht routen kann.

**App startet und beendet sich dann mit einem Key-Fehler.** `APP_KEY`
ist nicht gesetzt oder fehlerhaft. Das Framework verweigert den Start
in der Produktion ohne einen; fügen Sie die Ausgabe von
`suprnova key:generate --show` erneut in die Variablen des Service
ein.

**Migrationen schlagen beim Start fehl.** Prüfen Sie die Logs auf den
zugrunde liegenden SQL-Fehler. Häufige Ursachen sind ein nicht
gesetztes `DATABASE_URL` (überprüfen Sie, ob die Referenz
`${{ Postgres.DATABASE_URL }}` aufgelöst wurde) oder eine Migration,
die gegen eine veraltete Baseline gelaufen ist (`./app migrate:status`
meldet, was wo angewendet ist).

**Scheduler feuert nie.** Überprüfen Sie, dass der Start-Befehl exakt
`./app schedule:work` ist (nicht `schedule:run`, das fällige Tasks
einmal ausführt und sich beendet). `schedule:list` aus einer
einmaligen Bereitstellung bestätigt, dass Ihre Tasks registriert sind.

## Nächste Schritte

- [Bereitstellung - Übersicht](deployment.md) - das Modell der
  einzelnen Binärdatei, auf dem Ihre Railway-Services laufen
- [Docker-CLI](cli-docker.md) - was `docker:init` und
  `docker:compose` tatsächlich generieren
- [Konfiguration](configuration.md) - `.env`-Laden, typisierte
  Konfiguration, erforderliche Schlüssel
- [Konsole](console.md) - `schedule:work`, `queue:work`,
  `workflow:work` und der Rest der einheitlichen CLI
- [Zu Digital Ocean bereitstellen](deployment-digital-ocean.md) -
  dasselbe Rezept auf einem anderen PaaS

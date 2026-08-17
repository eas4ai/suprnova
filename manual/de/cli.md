# CLI - Übersicht

Suprnova liefert zwei Binaries mit unterschiedlichen Aufgaben aus.
Die globale `suprnova` - einmalig installiert nach `~/.cargo/bin` -
scaffoldet neue Projekte, generiert Code, startet Dev-Server und
führt Migrationen aus. Die projektspezifische `console`, gebaut aus
dem `src/bin/console.rs` jeder App, führt Laufzeitbefehle aus, die
die kompilierten Typen der App brauchen (Seeder, Pruner, Ihre eigenen
`#[command]`-Handler). Dieses Kapitel ist die Karte; jedes
Subkommando hat seinen eigenen Deep-Dive in den Nachbarkapiteln, die
unter [Nächste Schritte](#nächste-schritte) aufgeführt sind.

## Installation

Die CLI wird über `cargo install --git` ausgeliefert. Suprnova ist
noch nicht auf crates.io - siehe die [Notiz vor dem Start in
Installation](installation.md#pre-launch-note) für den Grund.

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.3 suprnova-cli
suprnova --version
```

Um später zu aktualisieren, übergeben Sie `--force`:

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.2.3 suprnova-cli
```

## Die zwei Binaries

| Binary | Gebaut aus | Verwendet für |
|---|---|---|
| `suprnova` | `suprnova-cli/` (diese Crate) | Scaffolding (`new`), Generatoren (`make:*`), Dev Runner (`serve`), Migrationen (`migrate*`, `db:sync`), Docker-Konfiguration (`docker:*`), SSR-Worker (`ssr:*`), Schlüssel-Prägung (`key:generate`), Typgenerierung (`generate-types`) |
| `console` | `src/bin/console.rs` in Ihrem Projekt | Laufzeitbefehle, die die kompilierten Typen Ihrer App einbinden - eingebautes `db:seed` und `model:prune` plus jedes von Ihnen definierte `#[command]` / `#[derive(Command)]` |

Worker-Daemons (`schedule:run`, `schedule:work`, `schedule:list`,
`workflow:work`, `queue:work`) sitzen auf einer dritten Oberfläche:
dem eigenen clap-Parser Ihrer *App*-Binary, derselben Binary, die
HTTP bedient. Die globale `suprnova` shellt dafür in
`cargo run --quiet -- <name>` aus, sodass Sie sie aus der CLI heraus
starten können, die Sie bereits offen haben. Siehe
[Konsole](console.md) für die vollständige Dreiteilung.

### Warum Suprnova abweicht

Laravel löst das mit einem einzigen Pro-Projekt-Skript -
`php artisan` -, weil PHP Framework- und Nutzercode zur Laufzeit
gemeinsam lädt. Rust linkt Binaries zur Compile-Zeit, sodass eine
globale `suprnova`-Binary Ihre Seeder, Factories oder
`#[command]`-Handler nicht statisch sehen kann. Die pragmatische
Aufteilung:

- Reine Dateiarbeit (Scaffolding, Generatoren, Ops) lebt auf der
  globalen `suprnova`-Binary
- Laufzeitarbeit, die Ihre kompilierten Typen braucht, lebt auf der
  projektspezifischen `console`-Binary
- Daemons leben auf Ihrer App-/Server-Binary, sodass sie sich
  denselben Boot-Pfad wie `serve` teilen

Sie bekommen die Ergonomie von `php artisan`
(`cargo run --bin console -- db:seed` oder direkt `console <name>`)
ohne die Lüge des statischen Linkens.

## Befehle im Überblick

Dieselbe Liste, die `suprnova --help` ausgibt, in derselben
Gruppierung.

### Erstellen

| Befehl | Beschreibung |
|---|---|
| `suprnova new [name]` | Scaffoldet ein neues Projekt. Siehe [`suprnova new`](cli-new.md). |
| `suprnova serve` | Startet Backend + Vite zusammen mit Hot Reload. Siehe [`suprnova serve`](cli-serve.md). |
| `suprnova dev:tls` | Vertraut der CA von portless und registriert eine `https://<name>.localhost`-Dev-URL. Siehe [HTTPS-Dev-URLs](dev-tls.md). |
| `suprnova web:run` | Führt die App-Binary direkt aus (kein Vite, keine Rebuild-Schleife). Ein produktionsartiger lokaler Lauf. |

### Generieren

| Befehl | Beschreibung |
|---|---|
| `suprnova make:controller <name>` | Scaffoldet einen Controller in `src/controllers/`. |
| `suprnova make:action <name>` | Scaffoldet eine aufrufbare Aktion in `src/actions/`. |
| `suprnova make:middleware <name>` | Scaffoldet eine Middleware in `src/middleware/`. |
| `suprnova make:migration <name>` | Scaffoldet eine SeaORM-Migration in `src/migrations/`. |
| `suprnova make:inertia <name>` | Scaffoldet eine Inertia-Seite in `frontend/src/pages/`. Übergeben Sie `--data`, um stattdessen eine `#[derive(Data, Validate)]`-Props-Struktur in `src/props/` zu erzeugen. |
| `suprnova make:error <name>` | Scaffoldet einen Domänenfehler in `src/errors/`. |
| `suprnova make:task <name>` | Scaffoldet einen geplanten Task in `src/tasks/`. |
| `suprnova make:command <name>` | Scaffoldet einen `#[derive(Command)]`-Console-Befehl in `src/commands/`. |
| `suprnova generate-types` | Gibt TypeScript-Typen aus jeder `#[derive(InertiaProps)]`-Struktur aus. `-o <path>`, um die Ausgabe zu überschreiben, `-w`, um zu überwachen und neu zu generieren. |

Siehe [Code-Generatoren](cli-generators.md) für die vollständigen
Scaffold-Details und wie jede generierte Datei aussieht.

### Datenbank

| Befehl | Beschreibung |
|---|---|
| `suprnova migrate` | Führt alle ausstehenden Migrationen aus. |
| `suprnova migrate:status` | Zeigt, welche Migrationen angewendet sind und welche ausstehen. |
| `suprnova migrate:rollback [--step N]` | Rollt die letzten N Migrationen zurück (Standard 1). |
| `suprnova migrate:fresh [--force]` | Löscht jede Tabelle und führt alle Migrationen erneut aus. **Zerstörerisch.** In Produktion braucht es `--force` plus eine eingetippte Bestätigung auf einem interaktiven Terminal. |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | Führt Migrationen aus und regeneriert SeaORM-Entities aus dem lebenden Schema. `--regenerate-models` überschreibt eigene Model-Dateien in `src/models/`. |

`db:seed` steht **nicht** hier - es lebt auf der projektspezifischen
`console`-Binary, weil die Seeder-Registry in Ihre Crate kompiliert
ist. Führen Sie es über `cargo run --bin console -- db:seed` oder
`./target/debug/console db:seed` aus. Siehe [Konsole](console.md)
für das Registrierungsmuster.

Siehe das [Migrations-Kapitel](cli-migrations.md) für den
vollständigen Migrations-Workflow.

### Zeitplan

| Befehl | Beschreibung |
|---|---|
| `suprnova schedule:run` | Führt jeden fälligen Task einmal aus. Die cron-freundliche Form. |
| `suprnova schedule:work` | Vordergrund-Daemon, der jede Minute prüft und fällige Tasks ausführt. |
| `suprnova schedule:list` | Gibt jeden registrierten Task mit seinem Cron-Ausdruck aus. |

Jeder dieser Befehle shellt in `cargo run --quiet -- <name>` gegen
Ihre App-/Server-Binary aus - dieselbe Binary, die HTTP bedient -,
sodass registrierte Tasks und gebootstrappte Services sichtbar sind.
Siehe [Scheduling-CLI](cli-scheduling.md) und das Kapitel
[Task-Planung](scheduling.md).

### Workflow

| Befehl | Beschreibung |
|---|---|
| `suprnova workflow:work` | Startet den Workflow-Worker-Daemon. Zieht Workflow-Schritte aus der Registry und führt sie mit derselben Panic-Grenze wie HTTP-Handler aus. |
| `suprnova workflow:install` | Legt die Migrationen `workflow` + `workflow_steps` in `src/migrations/` ab. In frischen Scaffolds bereits vorhanden. |

Siehe [Workflows](workflows.md).

### SSR

| Befehl | Beschreibung |
|---|---|
| `suprnova ssr:start [--runtime node\|bun\|deno] [--bundle <path>]` | Startet den Inertia-SSR-Worker im Vordergrund. Fällt zurück auf die Env `SUPRNOVA_SSR_RUNTIME`, dann auf `node`; das Bundle fällt zurück auf `SUPRNOVA_SSR_BUNDLE`, dann auf `frontend/bootstrap/ssr/ssr.js`. |
| `suprnova ssr:check [--url <url>] [--timeout-ms N]` | Sondiert den SSR-Worker. Fällt zurück auf `SUPRNOVA_SSR_URL`, dann auf `http://127.0.0.1:13714`. Standard-Timeout 2000 ms. |

Siehe [Inertia SSR](frontend.md) für das Produktions-Setup.

### Bereitstellen

| Befehl | Beschreibung |
|---|---|
| `suprnova docker:init` | Gibt ein mehrstufiges Produktions-`Dockerfile` + `.dockerignore` aus. |
| `suprnova docker:compose [--with-mailpit] [--with-minio]` | Gibt eine `docker-compose.yml` für die lokale Entwicklung aus. Postgres + Redis immer enthalten; Mailpit und MinIO als Opt-in. |

Siehe [Docker](cli-docker.md) und das Kapitel
[Bereitstellung](deployment.md).

### Sicherheit

| Befehl | Beschreibung |
|---|---|
| `suprnova key:generate [--show]` | Prägt einen 32-Byte-AES-256-Schlüssel, base64 URL-safe ohne Padding (dasselbe Wire-Format, das `EncryptionKey::to_base64` erzeugt). `--show` gibt nur den Schlüssel aus, für `APP_KEY=$(suprnova key:generate --show)`. |

Siehe [Verschlüsselung](encryption.md) für das, was `APP_KEY`
schützt, und wie die Rotation über `APP_KEY_PREVIOUS` funktioniert.

## Schnellstart

Der häufigste Weg von „nichts installiert“ zu „laufende App“:

```bash
# 1. Die CLI installieren
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.3 suprnova-cli

# 2. Ein Projekt scaffolden (interaktiv - wählt standardmäßig Svelte)
suprnova new my-app

# 3. Starten
cd my-app
suprnova migrate
npm install
suprnova serve
```

Nicht-interaktives Scaffold (CI, skriptgesteuertes Setup):

```bash
suprnova new my-app \
  --frontend svelte \
  --no-interaction \
  --no-git
```

Reines API-Scaffold (kein Inertia, keine SPA):

```bash
suprnova new my-api --api
```

Code in einem bestehenden Projekt generieren:

```bash
suprnova make:controller Posts
suprnova make:migration create_posts_table
suprnova make:command reports:daily   # registriert sich unter der projektspezifischen console-Binary
suprnova migrate
```

## Hilfe erhalten

`--help` (oder `-h`) funktioniert bei jedem Subkommando. Die oberste
Hilfe ist handformatiert (`ui::print_help`) und gruppiert Befehle
nach Abschnitt; die Hilfe pro Subkommando kommt von clap und zeigt
jedes Flag mit seinem Standardwert:

```bash
suprnova --help
suprnova new --help
suprnova serve --help
suprnova make:inertia --help
```

Für die projektspezifische `console`-Binary:

```bash
cargo run --bin console -- --help
cargo run --bin console -- db:seed --help
cargo run --bin console -- <your-command> --help
```

`--version` gibt die Version auf ihrer eigenen Zeile aus, was Sie
wollen, wenn Sie einen Bug melden oder prüfen, ob eine Installation
erfolgreich war:

```bash
suprnova --version
# suprnova 1.2.3
```

Sowohl `-v` als auch `-V` werden akzeptiert. Claps generiertes Flag
bietet nur `-V`; dieses hier ist von Hand deklariert, damit auch die
Kleinschreibung - die die meisten zuerst versuchen - funktioniert.
Die Version erscheint außerdem im `--help`-Banner, wo sie schon
lebte, bevor es das Flag gab.

## Nächste Schritte

- [`suprnova new`](cli-new.md) - jedes Flag, das der Scaffolder
  akzeptiert, und das Verzeichnislayout, das er erzeugt
- [`suprnova serve`](cli-serve.md) - der Dev Runner: Backend + Vite +
  Typgenerierung
- [Code-Generatoren](cli-generators.md) - die vollständige
  `make:*`-Familie mit Ausgabe-Vorlagen
- [Migrations-CLI](cli-migrations.md) - `migrate`, `migrate:fresh`,
  `db:sync` und der SeaORM-Workflow
- [Konsole](console.md) - die projektspezifische `console`-Binary,
  `#[command]`, `#[derive(Command)]` und die
  Drei-Binary-Asymmetrie

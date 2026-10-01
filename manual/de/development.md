# Entwicklung

Die tägliche Suprnova-Schleife ist ein Befehl: `suprnova serve`. Es
führt das Rust-Backend, das Vite-Frontend und einen TypeScript-Typen-
Regenerator in einem einzigen Prozess aus, die jeweils die richtigen
Dateien überwachen. Dieses Kapitel behandelt den Dev-Server, wie die
Hot-Reload-Komponenten zusammenpassen, und die Befehle, die Sie täglich
brauchen. Für die erste Einrichtung siehe [Installation](installation.md);
für den Rundgang durch das Verzeichnis siehe [Verzeichnisstruktur](structure.md).

## Der Dev-Server

Aus dem Stammverzeichnis eines generierten Projekts:

```bash
suprnova serve
```

Die CLI gibt zwei URLs aus, gefolgt von einem kontinuierlichen Datenstrom
mit Präfixen aus jedem Kindprozess:

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765

[backend]  Compiling links v0.1.0
[backend]  Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.21s
[backend]  Running `target/debug/links`
[frontend] VITE v6.0.1  ready in 312 ms
[frontend]   ➜  Local:   http://localhost:5765/
[types]    Watching for Rust file changes to regenerate types
```

Sie besuchen die Backend-URL (`127.0.0.1:8765`). Vite bedient Ihr JS/CSS
über Inertias Dev-Integration - Sie besuchen `:5765` nicht direkt.
Drücken Sie `Ctrl+C` einmal und die CLI fährt beide Kindprozesse sauber herunter.

### Flags

| Flag | Standard | Was es tut |
|---|---|---|
| `-p`, `--port <N>` | `8765` | Backend-Port |
| `--frontend-port <N>` | `5765` | Vite-Port |
| `--backend-only` | off | Vite-Kind überspringen (nur API-Arbeit) |
| `--frontend-only` | off | Backend-Kind überspringen (Komponentenarbeit gegen ein anderswo laufendes Backend) |
| `--skip-types` | off | TypeScript-Typ-Generator und seinen Watcher überspringen |

Die gleichen Ports können in `.env` über `SERVER_PORT` und `VITE_PORT` gesetzt
werden. Ein Flag in der Befehlszeile hat Vorrang vor `.env`.

### Was es vorab prüft

Bevor etwas erzeugt wird, führt `suprnova serve` folgende Prüfungen durch:

1. **Prüft, ob Sie sich in einem Projekt befinden.** Bricht mit einer
   klaren Fehlermeldung ab, wenn es keine `Cargo.toml` gibt (oder kein
   `frontend/` bei der Ausführung des Frontend).
2. **Generiert TypeScript-Typen einmal.** Durchsucht `src/` nach
   `#[derive(InertiaProps)]` und schreibt
   `frontend/src/types/inertia-props.ts`. Wird von `--skip-types` oder
   `--frontend-only` übersprungen.
3. **Installiert `cargo-watch` falls fehlend.** Beim ersten Durchlauf auf
   einer neuen Maschine wird `cargo install cargo-watch` für Sie
   ausgeführt und dann fortgesetzt.
4. **Führt `npm install` aus wenn `frontend/node_modules` fehlt.** Kein
   manueller Installationsschritt bei einem frischen Klon erforderlich.

## Hot Reload

Drei Watcher laufen gleichzeitig in `suprnova serve`:

- **`cargo watch -x 'run --bin <pkg>'`** steuert das Backend. Jede `.rs`-
  Änderung im Projekt löst eine Neukompilierung und einen
  Neustart im Prozess aus. Kompilierungsfehler werden in den
  `[backend]`-Datenstrom gedruckt und die vorherige Binärdatei bleibt
  aktiv, bis der nächste erfolgreiche Build abgeschlossen ist.
- **Vite** steuert das Frontend. Komponenten-, Stil- und Asset-Änderungen
  werden ohne vollständiges Neuladen in den offenen Browser-Reiter
  hot-module-replaced.
- **`notify`-basierter Typ-Watcher** führt den InertiaProps-Scanner neu aus,
  wenn sich eine `.rs`-Datei ändert. Er puffert bei 500ms ab, sodass ein
  Speicherburst `inertia-props.ts` einmal regeneriert. Die Ausgabe
  erscheint unter dem `[types]`-Präfix.

Das Dritte ist das Bit, über das Sie nicht nachdenken müssen: Benennen Sie
ein Feld auf einer `#[derive(InertiaProps)]`-Struktur um und das
entsprechende TypeScript-Interface folgt beim nächsten Speichern nach. Die
Svelte/React/Vue-Seite nimmt den neuen Typ sofort auf. Kein
`suprnova generate-types`-Aufruf während der normalen Entwicklung erforderlich.

### Warum Suprnova abweicht

Die meisten Rust-Web-Stacks machen Hot Reload zu Ihrem Problem - wählen
Sie Ihren eigenen Datei-Watcher, schreiben Sie Ihren eigenen
Neustart-Wrapper, führen Sie Vite in einem separaten Terminal aus. Die
meisten Laravel-Stacks machen TypeScript-Typen zu Ihrem Problem - deklarieren
Sie sie an zwei Stellen (PHP und TS) und halten Sie sie synchron.
`suprnova serve` führt beide Watcher aus, plus den Typ-Generator, der Ihre
Frontend-Typen ehrlich hält, als einen überwachten Prozess. Die Tokio-Runtime
macht "viele Dinge gleichzeitig" billig genug, dass eine Dev-Schleife es
frei ausgeben kann.

## Alltägliche Befehle

Eine Handvoll, die Sie stündlich ausführen werden:

```bash
suprnova serve                    # Dev starten (Backend + Vite + Typ-Watcher)
suprnova make:controller orders   # Einen Controller generieren
suprnova make:migration add_idx   # Eine Migration generieren
suprnova db:sync                  # Migrationen ausführen, SeaORM-Entitäten regenerieren
suprnova migrate:status           # Sehen, was angewendet wurde
suprnova migrate:fresh            # Tabellen löschen + von vorne ausführen
suprnova key:generate --show      # APP_KEY rotieren
cargo run --bin console <cmd>     # Jeden `#[command]`-annotierten Konsolen-Handler
cargo test                        # Test-Suite ausführen
```

`db:sync` ist die Dev-Abkürzung für "Migration + Entity-Regeneration in
einem Schritt." In Production verwenden Sie einfach `suprnova migrate`, da Sie
nicht möchten, dass eine Regeneration auf einer Release-Maschine erfolgt. Die
vollständige Generator-Oberfläche finden Sie in
[Code-Generatoren](cli-generators.md) und die Migration-Befehle in
[Migrationen](migrations.md).

## Debugging

### Protokollierung

Suprnova nutzt `tracing` end-to-end. Filtern Sie, was gedruckt wird, mit
`LOG_LEVEL` (die gleiche Syntax wie `tracing-subscriber`'s `EnvFilter`):

```bash
# Ausführliche Framework-Ausgabe
LOG_LEVEL=debug suprnova serve

# hyper leise, aber Ihr Crate ausführlich
LOG_LEVEL=info,my_app=debug,hyper=warn suprnova serve
```

Das Ausgabeformat wird durch `LOG_FORMAT` kontrolliert (`pretty` für
Menschen lesbar, `json` für maschinell auswertbar). Der Dev-Standard ist
`pretty`. Siehe [Beobachtbarkeit](observability.md) für die vollständige
Protokollierungs-Oberfläche.

### SQL-Abfragen

Schalten Sie Pro-Query-Protokollierung mit einer Umgebungsvariable ein:

```env
DB_LOGGING=true
```

Dies leitet jede SeaORM-Abfrage durch `tracing` auf `info` Level, sodass Sie
genau sehen können, was ausgeführt wird. Lassen Sie es in Production aus, es
sei denn, Sie verfolgen eine bestimmte langsame Abfrage - die Logmenge wird
schnell unübersichtlich.

### Backtraces

Standard Rust:

```bash
RUST_BACKTRACE=1 suprnova serve
```

Ein Panic in einem Handler wird abgefangen und in eine strukturierte
500-Antwort umgewandelt; das Backtrace landet in Ihren Logs, ohne den
Server herunterzufahren. Siehe [Fehlermodell](error-model.md) für
Details zum Funktionieren dieses Vertrags.

## Tests in der Schleife

```bash
cargo test                        # ganzer Workspace
cargo test -p my_app              # nur Ihr App-Crate
cargo test some_test_name         # nach Name filtern
cargo test -- --nocapture         # println!/tracing-Ausgabe anzeigen
```

Die Testausführung erfolgt mit reinem Cargo. Die Framework-seitigen Helfer
(`#[suprnova_test]`, `TestDatabase`, `expect!`, Fakes für Mail/Queue/
Storage/etc.) sind in [Testen](testing.md) und
[Datenbank-Tests](database-testing.md) dokumentiert. Sie laufen unter dem
gleichen `cargo test`, das Sie bereits kennen.

## Arbeiten mit dem SSR-Worker

Wenn Ihre App Inertia-Server-seitige Rendering nutzt, möchten Sie den
SSR-Worker neben `suprnova serve` während der Entwicklung haben:

```bash
# Terminal 1
suprnova serve

# Terminal 2
suprnova ssr:start
```

`ssr:start` führt den gebündelten SSR-Worker unter Node, Bun oder Deno aus
(`--runtime`). `ssr:check` prüft, ob ein laufender Worker erreichbar ist.
Beide sind im Frontend-Kapitel dokumentiert - siehe
[Frontend](frontend.md).

## Wenn etwas falsch aussieht

Eine kurze Triage-Liste für die häufigsten Dev-Loop-Probleme:

- **Port bereits in Benutzung.** Ein anderes `suprnova serve` ist noch aktiv,
  oder ein vorheriges Backend ist blockiert. Verwenden Sie `lsof -i :8765`
  um es zu finden, oder übergeben Sie einfach `--port 8001`.
- **`cargo-watch` kompiliert ständig neu.** Ein Editor schreibt Dateien beim
  Speichern um (Formatter, Linter mit Autofix). Deaktivieren Sie das
  Format-beim-Speichern für das Projekt, oder grenzen Sie Ihren Watcher mit
  `CARGO_WATCH_IGNORE`-Mustern ein.
- **TypeScript-Typen aktualisieren sich nicht.** Entweder `--skip-types` wurde
  übergeben, oder der Watcher stolperte über einen `.rs`-Parse-Fehler. Schauen
  Sie sich die `[types]`-Zeilen an - es druckt eine Warnung und setzt fort,
  anstatt das ganze Serve zu beenden.
- **Vite-Fehler aber das Backend ist in Ordnung.** Führen Sie `npm install`
  in `frontend/` aus (die CLI tut dies beim ersten Serve, aber wenn Sie
  `node_modules` weglöschen, wird sie es erst erneut tun, wenn dieses
  Verzeichnis beim nächsten Start erneut fehlt).

Für alles andere deckt das Kapitel [Fehler](errors.md) tiefere
Triage-Muster ab.

## Nächste Schritte

- [Installation](installation.md) - Erste Einrichtung der CLI und eines
  Projekts
- [Schnellstart](quickstart.md) - Erstellen einer winzigen App von Ende zu Ende
- [Verzeichnisstruktur](structure.md) - Was jedes Verzeichnis enthält
- [Code-Generatoren](cli-generators.md) - Jeden `make:*`-Befehl
- [Testen](testing.md) - `#[suprnova_test]`, Fakes und die Test-Datenbank

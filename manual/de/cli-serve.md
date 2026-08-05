# suprnova serve

`suprnova serve` führt Ihr Backend und den Vite-Dev-Server zusammen
aus, mit Hot Reload auf beiden Seiten, plus automatischer
TypeScript-Typ-Regenerierung, sobald Sie eine
`#[derive(InertiaProps)]`-Struktur anfassen. Es ist der eine Befehl,
den Sie beim Entwickeln in einem Terminal offen halten.

```bash
suprnova serve
```

Beide Prozesse streamen ihr stdout in dasselbe Terminal, mit
farbigen `[backend]`- und `[frontend]`-Präfixen, damit Sie erkennen,
wer was gesagt hat. `Ctrl+C` fährt beide Kindprozesse sauber
herunter.

## Verwendung

```bash
suprnova serve [OPTIONS]
```

| Option | Standard | Beschreibung |
|---|---|---|
| `-p, --port <PORT>` | `8765` (CLI) / `$SERVER_PORT` (env) | Backend-HTTP-Port |
| `--frontend-port <PORT>` | `5765` (CLI) / `$VITE_PORT` (env) | Vite-Dev-Server-Port |
| `--backend-only` | `false` | Überspringt den Vite-Dev-Server |
| `--frontend-only` | `false` | Überspringt das Backend, führt nur Vite aus |
| `--skip-types` | `false` | Regeneriert TypeScript-Typen bei Rust-Änderungen nicht |

CLI-Flags haben Vorrang vor Umgebungsvariablen, die wiederum Vorrang
vor den eingebauten Standardwerten haben. Eine gescaffoldete `.env`
liefert `SERVER_PORT=8765` und `VITE_PORT=5765` mit; diese Werte
werden verwendet, sofern Sie sie nicht mit `--port` überschreiben.

## Beispiele

### Standard - beide Server

```bash
suprnova serve
```

Ausgabe:

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765
[backend] Compiling my-app v0.1.0 ...
[frontend] VITE v6.3.0  ready in 312 ms
```

Rufen Sie `http://127.0.0.1:8765` in Ihrem Browser auf. Das Backend
liefert die Inertia-HTML-Shell aus und leitet Asset-Anfragen an Vite
weiter, sodass Sie die Vite-URL nicht direkt aufrufen müssen.

### Eigene Ports

```bash
suprnova serve --port 3000 --frontend-port 3001
```

Oder setzen Sie sie in `.env` und starten Sie ohne Flags:

```env
SERVER_PORT=3000
VITE_PORT=3001
```

### Nur Backend

```bash
suprnova serve --backend-only
```

Gut geeignet, wenn Sie an einem reinen API-Projekt arbeiten oder Ihr
Frontend bereits in einem anderen Terminal läuft (oder auf einer
anderen Maschine, oder als bereitgestellte Preview).

### Nur Frontend

```bash
suprnova serve --frontend-only
```

Gut geeignet, um an der UI zu arbeiten, ohne bei jedem Speichern die
Kosten eines Rust-Rebuilds zu zahlen, oder wenn das Backend in einer
anderen Shell (oder in Docker) läuft.

### Typgenerierung überspringen

```bash
suprnova serve --skip-types
```

Deaktiviert den TypeScript-Regenerierungs-Watcher. Verwenden Sie
dies, wenn Sie `frontend/src/types/inertia-props.ts` von Hand
verwalten, oder wenn Sie weit entfernt von jeglichem Inertia-Code
arbeiten und eine ruhigere Ausgabe möchten.

## Was es tatsächlich tut

Wenn Sie `suprnova serve` ausführen, macht die CLI Folgendes:

1. Lädt `.env` aus dem aktuellen Verzeichnis.
2. Löst Backend- und Frontend-Ports auf (CLI-Flag → Env-Var →
   Standard).
3. Prüft, dass Sie sich in einem Suprnova-Projekt befinden -
   `Cargo.toml` muss existieren (außer bei `--frontend-only`), und
   ein `frontend/`-Verzeichnis muss existieren (außer bei
   `--backend-only`).
4. Regeneriert TypeScript-Typen aus jeder
   `#[derive(InertiaProps)]`-Struktur, die es in `src/` findet, und
   schreibt sie nach `frontend/src/types/inertia-props.ts`.
5. Installiert `cargo-watch` über `cargo install --locked --version
   "^8.5" cargo-watch`, falls es noch nicht im PATH liegt (einmalig,
   mit einem „Installing...“-Hinweis). Wird unter `--frontend-only`
   übersprungen.
   Die Version ist begrenzt, weil `serve` `cargo watch -x` antreibt,
   dessen Bedeutung über einen Major-Sprung hinweg nicht garantiert
   ist; `--locked` baut den von cargo-watch veröffentlichten
   Dependency-Baum, statt ihn zur Installationszeit neu aufzulösen.
   Ein Befehl, der als Nebeneffekt des Startens eines Dev-Servers
   Software installiert, sollte nicht auch noch Versionen für Sie
   auswählen.
6. Führt `npm install` in `frontend/` aus, falls `node_modules` noch
   nicht existiert. Wird unter `--backend-only` übersprungen.
7. Startet `cargo watch -x 'run --bin <package-name>'` für das
   Backend. `cargo-watch` führt die Binary bei jeder Änderung einer
   `.rs`-Datei erneut aus.
8. Startet `npm run dev` in `frontend/` für Vite, was Ihnen HMR für
   Svelte-/React-/Vue-Komponenten und Tailwind-Klassen gibt.
9. Startet einen Datei-Watcher auf `src/`, der den Typ-Generator bei
   jeder Änderung einer `.rs`-Datei erneut ausführt, sobald der
   Speicher-Burst 500 ms lang ruhig war. Das Debounce ist
   trailing-edge, sodass ein Burst - `cargo fmt`, Format-beim-
   Speichern über mehrere Dateien, ein Branch-Wechsel - zu genau
   einer Regenerierung zusammenfließt, die *nach* dem letzten
   Schreibvorgang läuft, statt einer, die beim ersten Datei-Ereignis
   feuert und den Rest verpasst.
10. Leitet stdout/stderr beider Kindprozesse mit den Präfixen
    `[backend]` und `[frontend]` an Ihr Terminal weiter.

`Ctrl+C` signalisiert dem Manager, sein Shutdown-Flag zu setzen,
beide Kindprozesse zu beenden und sich selbst zu beenden. Beendet
sich einer der beiden Prozesse von selbst - meist wegen eines
Rust-Compile-Fehlers, von dem sich `cargo watch` nicht mehr erholen
kann, oder wegen eines Port-Konflikts -, behandelt der Manager das
als Shutdown-Signal und fährt den anderen ebenfalls herunter.

### Warum Suprnova abweicht

Laravel-Nutzer führen für das Backend typischerweise `php artisan
serve` aus und `npm run dev` in einem weiteren Terminal, und die
meisten Teams überbrücken die Zwei-Terminal-Aufteilung mit einem
`Procfile` und `foreman`/`overmind`. Suprnova liefert diesen
Multiplexer als erstklassigen CLI-Befehl aus. Sie bekommen ein
Terminal, ein `Ctrl+C`, automatisches Toolchain-Bootstrapping
(`cargo-watch`, `npm install`) und eine typisierte Inertia-Brücke,
die `frontend/src/types/inertia-props.ts` laufend regeneriert, sodass
Ihre Svelte-/React-/Vue-Komponenten immer die aktuelle Prop-Form ohne
manuellen Typ-Sync sehen.

## Hot Reload

**Backend.** `cargo watch -x 'run --bin <package>'` ist die
Schleife: Jede `.rs`-Änderung im Projekt löst eine Neukompilierung
und einen Neustart des Servers aus. Kalte Neukompilierungen nach dem
Anfassen einer schweren Crate können mehrere Sekunden dauern;
inkrementelle Änderungen in einer einzelnen Datei liegen meist unter
einer Sekunde.

**Frontend.** Vites HMR hot-module-replaced Komponentenänderungen
direkt an Ort und Stelle, ohne vollständiges Neuladen, und bewahrt
den Komponentenzustand. Tailwind-Klassen aktualisieren sich live
über den Tailwind-v4-Watcher.

**TypeScript-Typen.** Bei jeder Änderung einer `.rs`-Datei führt der
Typ-Watcher den Generator erneut aus. Erscheinen neue
`#[derive(InertiaProps)]`-Strukturen (oder ändert eine bestehende
ihre Form), löst das regenerierte
`frontend/src/types/inertia-props.ts` Vites HMR für die Komponente
aus, die es importiert.

## Fehlerbehebung

### Port bereits in Benutzung

```text
[backend] Error: Address already in use (os error 98)
```

Suchen Sie den Prozess und beenden Sie ihn, oder wählen Sie einen
anderen Port:

```bash
lsof -i :8765
kill -9 <pid>

# oder
suprnova serve --port 8081
```

### `cargo-watch`-Installation schlägt fehl

Die CLI führt `cargo install cargo-watch` aus, falls es noch nicht im
PATH liegt. Schlägt diese Installation fehl (kein Netzwerk,
eingeschränkte Umgebung), installieren Sie es einmalig von Hand:

```bash
cargo install cargo-watch
```

Danach findet `suprnova serve` es und versucht nicht erneut, es zu
installieren.

### Frontend-Abhängigkeiten hängen

Schlägt `npm install` mitten im Bootstrap fehl, beheben Sie die
Ursache (npm-Registry erreichbar, Speicherplatz, Lockfile in Ordnung)
und führen Sie es manuell aus:

```bash
cd frontend && npm install
```

Führen Sie danach `suprnova serve` erneut aus. Die CLI führt
`npm install` nur automatisch aus, wenn `node_modules` fehlt; eine
erfolgreiche manuelle Installation lässt sie diesen Schritt also
überspringen.

### Typ-Regenerierung erkennt Änderungen nicht

Der Watcher pollt alle 2 Sekunden (mit `notify` und einem
Poll-Intervall - gewählt für plattformübergreifende Zuverlässigkeit
gegenüber inotify-Eigenheiten) und debounced die Regenerierung auf
einmal alle 500 ms. Zeigt sich eine Änderung nicht:

- Stellen Sie sicher, dass die Datei unter `src/` liegt (der Watcher
  rekursiert nicht in `crates/`, `cmd/` oder `migrations/`).
- Stellen Sie sicher, dass die Struktur tatsächlich
  `#[derive(InertiaProps)]` trägt.
- Starten Sie `suprnova serve` neu und achten Sie auf die
  Startmeldung `Generated N type(s)` - sehen Sie
  `No InertiaProps structs found`, hat der Scanner nichts zum
  Ausgeben gefunden.

### Backend beendet sich unauffällig direkt nach dem Start

Beendet sich einer der beiden Kindprozesse, fährt der Manager auch
den anderen herunter. Ist das Backend mit einem Compile-Fehler
gestorben, zeigen die `[backend]`-Zeilen direkt über der Meldung
„Servers stopped.“ das `error[E…]` von rustc. Beheben Sie den
Compile-Fehler und starten Sie erneut.

## Nächste Schritte

- [Installation](installation.md) - die CLI auf Ihrer Maschine
  installieren
- [Schnellstart](quickstart.md) - eine vollständige Walkthrough der
  ersten App
- [Verzeichnisstruktur](structure.md) - was `suprnova new`
  gescaffoldet hat
- [Code-Generatoren](cli-generators.md) - `make:controller`,
  `make:action` usw.
- [Konsole](console.md) - die projektspezifische
  `cargo run --bin console`-Binary

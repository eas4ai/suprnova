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
| `--no-restart` | `false` | Einen abgestürzten Dev-Prozess nicht neu starten, sondern stattdessen die gesamte Sitzung beenden (das frühere Verhalten) |
| `--restart-tries <N>` | `5` | Nach so vielen aufeinanderfolgenden Abstürzen eines Prozesses keine weiteren Neustartversuche unternehmen. Bei `--no-restart` ignoriert, da dies die Sitzung bereits beim ersten Absturz beendet. |
| `--timestamps` | `false` | Jeder Ausgabezeile eine Uhrzeit im Format `HH:MM:SS` voranstellen |
| `--json` | `false` | Auf stdout ein JSON-Objekt pro Zeile (NDJSON) statt Text mit Präfix ausgeben - siehe [JSON-Ausgabe](#json-ausgabe). Die Kombination mit `--timestamps` ist kein Fehler; `--timestamps` hat keine zusätzliche Wirkung, weil jedes Ereignis bereits seinen eigenen Zeitstempel trägt. |

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
liefert die Inertia-HTML-Hülle aus und leitet Asset-Anfragen an Vite
weiter, Sie müssen die Vite-URL also nicht direkt besuchen.

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

Gut geeignet für die Arbeit an einem reinen API-Projekt oder wenn Ihr
Frontend bereits in einem anderen Terminal läuft (oder auf einer anderen
Maschine oder in einer bereitgestellten Vorschau).

### Nur Frontend

```bash
suprnova serve --frontend-only
```

Gut geeignet für die Arbeit an der Oberfläche, ohne bei jedem Speichern
einen Rust-Rebuild zu bezahlen, oder wenn das Backend in einer anderen
Shell (oder in Docker) läuft.

### Reines API-Projekt

Ein mit `suprnova new --api` gescaffoldetes Projekt hat kein
`frontend/`-Verzeichnis. Führen Sie `serve` genau so aus wie überall
sonst:

```bash
suprnova serve
```

`serve` findet keine `frontend/package.json`, überspringt den
Vite-Bereich und die TypeScript-Generierung, die ihn speist, und startet
das Backend. `--frontend-only` bleibt bei einem solchen Projekt ein
Fehler: Es verlangt genau den Bereich, den es nicht gibt.

### Typgenerierung überspringen

```bash
suprnova serve --skip-types
```

Schaltet den Watcher für die TypeScript-Regenerierung ab. Nutzen Sie das,
wenn Sie `frontend/src/types/inertia-props.ts` von Hand pflegen oder wenn
Sie weit entfernt von jedem Inertia-Code arbeiten und eine ruhigere
Ausgabe wollen.

## Was es tatsächlich tut

Wenn Sie `suprnova serve` ausführen, macht die CLI Folgendes:

1. Lädt `.env` aus dem aktuellen Verzeichnis.
2. Löst Backend- und Frontend-Ports auf (CLI-Flag → Env-Var →
   Standard).
3. Prüft, dass Sie sich in einem Suprnova-Projekt befinden -
   `Cargo.toml` muss existieren (außer bei `--frontend-only`), und
   `--frontend-only` braucht ein `frontend/`-Verzeichnis mit einer
   `package.json`. Ein Projekt ohne ein solches Verzeichnis wird
   backend-only bedient statt abgelehnt.
4. Regeneriert TypeScript-Typen aus jeder
   `#[derive(InertiaProps)]`-Struktur, die es in `src/` findet, und
   schreibt sie nach `frontend/src/types/inertia-props.ts`. Wird
   übersprungen, wenn das Projekt kein Frontend hat.
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
   nicht existiert. Wird unter `--backend-only` übersprungen und
   ebenso, wenn das Projekt kein Frontend hat.
7. Startet `cargo watch` für das Backend, per `-w` auf genau die
   Pfade eingegrenzt, aus denen der Server tatsächlich gebaut wird:
   `src/`, `cmd/`, `Cargo.toml`, `Cargo.lock`, `.env` und `lang/`. In
   `cmd/` legt das Full-Stack-Scaffold die `main.rs` der Server-Binary
   ab; das `--api`-Scaffold legt sie in `src/` und hat gar kein
   `cmd/`. Jeder Pfad wird nur dann übergeben, wenn er existiert, denn
   cargo-watch verweigert den Start bei einem `-w`-Pfad, den es nicht
   gibt - ein noch nicht gebautes Projekt hat keine `Cargo.lock`, und
   die wird beim nächsten `serve` mit aufgenommen.

   `--no-vcs-ignores` gehört dazu. cargo-watch wendet Ihre
   `.gitignore` auch auf ausdrücklich benannte `-w`-Wurzeln an, nicht
   nur auf seinen eigenen Projektdurchlauf, und das Scaffold trägt
   `.env` in die `.gitignore` ein - ohne dieses Flag beobachtet
   `-w .env` also überhaupt nichts. Das Flag kann nicht erweitern, was
   das Backend neu startet, denn `-w` hat das bereits auf die sechs
   Pfade oben eingegrenzt, und per `.gitignore` ausgeschlossen sind
   darin nur `.env` und (bei `--api`) `Cargo.lock` - beide werden
   absichtlich beobachtet. `target/`, `node_modules` und
   der Rest liegen ohnehin außerhalb jeder beobachteten Wurzel.

   In einem gescaffoldeten Full-Stack-Projekt lautet der vollständige
   Aufruf `cargo watch --no-vcs-ignores -w src -w cmd -w Cargo.toml
   -w Cargo.lock -w .env -w lang -x 'run --bin <package-name>'`.
   Frontend-Änderungen und die generierten
   `frontend/src/types/*.ts`-Dateien liegen außerhalb dieses Bereichs
   und starten das Backend deshalb nie neu.
8. Startet `npm run dev` in `frontend/` für Vite, was Ihnen HMR für
   Svelte-/React-/Vue-Komponenten und Tailwind-Klassen gibt. Wird
   unter `--backend-only` übersprungen und ebenso, wenn das Projekt
   kein Frontend hat.
9. Startet jeden zusätzlichen Prozess, der in der `Suprnova.toml` des
   Projekts deklariert ist (siehe [Zusätzliche
   Entwicklungsprozesse](#zusätzliche-entwicklungsprozesse) unten),
   jeweils mit eigenem `[name]`-Präfix - Queue-Worker, Log-Tailer oder
   alles andere, das Sie sonst in einem anderen Terminal verwalten
   würden.
10. Startet einen Dateiwächter auf `src/`, der den Typgenerator nach
    jeder Änderung einer `.rs`-Datei erneut ausführt, sobald die Folge
    von Speicherungen 500 ms lang ruhig war. Nur echte Änderungen
    zählen - ein Anlegen, ein Schreiben oder ein Löschen. Lesezugriffe
    nicht, und das ist wichtig, weil der Generator bei jedem Durchlauf
    jede `.rs`-Datei unter dem Baum liest, den er beobachtet.
    Wird übersprungen, wenn
    das Projekt kein Frontend hat, genau wie die Typgenerierung beim
    Start in Schritt 4. Der Debounce erfolgt am Ende der Ruhephase;
    eine Folge - `cargo fmt`, Format-on-save über mehrere Dateien
    hinweg, ein Branch-Wechsel - wird so zu genau einer Regenerierung
    zusammengefasst, die *nach* dem letzten Schreibvorgang läuft, statt
    bei der ersten Datei auszulösen und den Rest zu verpassen.
    Eine Regenerierung schreibt die Datei nur dann, wenn sich das
    ausgegebene TypeScript von dem unterscheidet, was schon da ist, und
    der Wächter meldet nur, was er geschrieben hat: Eine Änderung, die
    keine Prop-Form verändert, gibt nichts aus und löst kein
    `types_regenerated`-Ereignis aus. Schweigen nach einer Speicherung
    heißt, dass Ihre Änderung die generierten Typen nicht verändert hat.
11. Leitet stdout/stderr jedes Kindprozesses mit einem `[name]`-Präfix
    (`[backend]`, `[frontend]` oder dem konfigurierten Prozessnamen) an
    Ihr Terminal weiter, optional mit Zeitstempel über `--timestamps` -
    oder mit `--json` stattdessen als NDJSON-Ereignisse (siehe
    [JSON-Ausgabe](#json-ausgabe) unten).

`Ctrl+C` signalisiert dem Manager, sein Shutdown-Flag zu setzen, alle
Kindprozesse zu beenden und selbst zu beenden. Endet ein Kindprozess
von selbst - ein Rust-Kompilierungsfehler, von dem sich `cargo watch`
nicht erholen kann, ein abgestürzter Vite-Prozess, ein fehlgeschlagener
`Suprnova.toml`-Prozess -, wird er nach einem kurzen Backoff neu
gestartet (200 ms, bei jedem aufeinanderfolgenden Absturz verdoppelt,
auf 5 s begrenzt; ein Prozess, der 30 s lief, setzt die Steigerung
zurück), statt die Sitzung abzubauen. Übergeben Sie `--no-restart`, um
das frühere Verhalten wiederherzustellen: Endet ein Kindprozess, wird
die gesamte Sitzung sofort beendet.

Ein Prozess, der weiterhin abstürzt, wird nicht endlos erneut versucht:
`--restart-tries` (standardmäßig `5`) begrenzt, wie viele
aufeinanderfolgende Abstürze `serve` erneut versucht, bevor es für
diesen einen Prozess aufgibt - weitere 30 s Laufzeit setzen die Anzahl
zurück, ebenso wie die Backoff-Verzögerung. Beim Aufgeben wird eine
konkrete Meldung ausgegeben und *nur* dieser Prozess nicht weiter neu
gestartet; die übrigen Prozesse (und die Sitzung selbst) laufen weiter.
Das entspricht Laravels eigenem Standardwert
`concurrently --restart-tries=5`. Siehe
[Fehlerbehebung](#ein-prozess-gerät-in-eine-absturzschleife).

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

Laravels Befehl `dev` bietet außerdem die Modi `--tabs` und
`--stream`, die beide die Ausgabe durch ein kleines Node-TUI
(`@laravel/multiplex`) rendern. Suprnova liefert dieses TUI nicht aus:
Ausgabe mit Präfixen in einem einzelnen Terminal ist im Rust-Ökosystem
für Entwicklungstools (`cargo watch`, `bacon`, `just`) die Norm, und
ein Prozessregister mit farbigen Präfixen gibt bereits das Signal
„welcher Prozess hat das gesagt?“, das ein TUI liefert. Die zugrunde
liegende Aufgabe von `--stream` - ein skriptbarer
Echtzeit-Ereignisstrom - wird als `--json` ausgeliefert (siehe
[JSON-Ausgabe](#json-ausgabe)); das Mehrbereichs-TUI von `--tabs` ist
die bewusste Absage, keine Lücke - ein zweites Interaktionsmodell und
eine zweite Bibliothek, die über Terminals hinweg funktionieren muss,
für ein Problem, das diese Seite bereits löst. Siehe die entsprechende
Zeile in [Parität](parity.md#what-we-won-t-ship-and-why).

## Hot Reload

**Backend.** `cargo watch` ist die Schleife, eingegrenzt auf die
Pfade, aus denen der Server gebaut wird. Es kompiliert neu und startet
neu bei einer Änderung unter `src/` oder `cmd/`, an `Cargo.toml`,
`Cargo.lock` oder `.env` oder unter `lang/` - `.env` wird beim Boot
einmal von `Config::init` gelesen und die Fluent-Kataloge einmal beim
Bootstrap, eine Änderung an einem von beiden greift also erst nach
einem Neustart. `.env` wird über `--no-vcs-ignores` beobachtet; ohne
das Flag würde Ihre `.gitignore` es vor dem Watcher verbergen. Eine
Komponente zu speichern oder
`frontend/src/types/inertia-props.ts` zu regenerieren liegt außerhalb
dieses Bereichs und lässt das Backend laufen. Kalte
Neukompilierungen nach dem Anfassen einer schweren Crate können
mehrere Sekunden dauern; inkrementelle Änderungen in einer einzelnen
Datei liegen meist unter einer Sekunde.

**Frontend.** Vites HMR hot-module-replaced Komponentenänderungen
direkt an Ort und Stelle, ohne vollständiges Neuladen, und bewahrt
den Komponentenzustand. Tailwind-Klassen aktualisieren sich live
über den Tailwind-v4-Watcher.

**TypeScript-Typen.** Bei jeder Änderung einer `.rs`-Datei führt der
Typ-Watcher den Generator erneut aus. Erscheinen neue
`#[derive(InertiaProps)]`-Strukturen (oder ändert eine bestehende
ihre Form), löst das regenerierte
`frontend/src/types/inertia-props.ts` Vites HMR für die Komponente
aus, die es importiert. Ist das ausgegebene TypeScript Byte für Byte
identisch mit dem, was schon auf der Platte liegt, bleibt die Datei
unangetastet und der Watcher sagt nichts - eine Regenerierung, die
nichts geändert hat, ist so auch keine Änderung, auf die irgendetwas
weiter unten reagieren müsste: weder Vite noch der Backend-Watcher
noch das, was `--json` mitliest.

## Zusätzliche Entwicklungsprozesse

`suprnova serve` führt immer Backend und Vite aus, aber die meisten Projekte haben mehr als zwei Dinge, die weiterlaufen müssen - einen Queue-Worker, einen Log-Tailer, einen Mail-Catcher. Deklarieren Sie sie in einer `Suprnova.toml` im Projektwurzelverzeichnis; `serve` startet sie, versieht ihre Ausgabe mit Präfixen und startet sie automatisch neu, direkt neben Backend und Frontend:

```toml
[[serve.process]]
name = "queue"
command = "cargo"
args = ["run", "--bin", "console", "--", "queue:work"]
color = "yellow"

[[serve.process]]
name = "logs"
command = "tail"
args = ["-f", "storage/logs/app.log"]
```

Jeder Eintrag benötigt `name` und `command`; `args` hat standardmäßig keine Einträge, `color` wird in Deklarationsreihenfolge einer der Farben Grün/Gelb/Blau/Weiß zugewiesen (oder wählen Sie eine der acht benannten `console`-Farben: Schwarz, Rot, Grün, Gelb, Blau, Magenta, Cyan, Weiß). Namen müssen eindeutig sein. `Suprnova.toml` ist vollständig optional; ein Projekt ohne diese Datei verhält sich genau wie zuvor.

### Warum Suprnova abweicht

Laravel registriert zusätzliche `dev`-Prozesse aus PHP heraus - `DevCommands::register($command, $name)`, typischerweise in `boot()` eines Service-Providers -, weil `php artisan dev` einen Multiplexer aus demselben Prozess heraus startet, der die Anwendung bereits gebootet hat. `suprnova serve` ist ein von Ihrer Anwendung getrenntes Binary; es linkt oder startet niemals Ihren Rust-Code und ruft nur `cargo watch` und `npm` als Kindprozesse auf. Es gibt keinen Anwendungs-Bootstrapping-Hook, an den man sich hängen könnte; die Registrierung muss daher Daten sein, die die CLI liest, statt eines Aufrufs aus Ihrem Code - daher `Suprnova.toml` statt einer API `DevProcesses::register()`.

## JSON-Ausgabe

Übergeben Sie `--json`; dann schreibt `suprnova serve` ein JSON-Objekt pro Zeile (NDJSON) auf stdout statt farbigem Text mit `[name]`-Präfix - während es aktiv ist, geht nichts anderes auf stdout, sodass Sie die Ausgabe direkt an `jq` oder einen anderen zeilenorientierten JSON-Consumer weiterleiten können. Jede Zeile besitzt ein Feld `type`:

| `type` | Felder | Bedeutung |
|---|---|---|
| `started` | `ts`, `name`, `pid` | Ein Prozess (Backend, Frontend oder ein `Suprnova.toml`-Eintrag) wurde erstmals gestartet. |
| `output` | `ts`, `name`, `stream` (`"stdout"` oder `"stderr"`), `line` | Eine Ausgabezeile eines Kindprozesses, die als Feld übertragen wird, statt roh durchgeleitet zu werden. |
| `exited` | `ts`, `name`, `code` (nullable) | Ein Prozess wurde beendet. `code` ist `null`, wenn er durch ein Signal beendet wurde, statt mit einem Status zurückzukehren. |
| `restart_scheduled` | `ts`, `name`, `delay_ms` | Ein abgestürzter Prozess wird nach `delay_ms` neu gestartet (siehe den Backoff-Zeitplan oben). |
| `restart_succeeded` | `ts`, `name`, `pid` | Ein geplanter Neustart war erfolgreich; der Prozess läuft wieder unter einer neuen PID. |
| `gave_up` | `ts`, `name`, `tries` | Der Prozess ist `tries` Mal hintereinander abgestürzt (`--restart-tries`) und `serve` versucht keinen Neustart mehr. Die Sitzung und alle anderen Prozesse laufen weiter. |
| `types_regenerated` | `ts`, `artifact` (`"inertia_props"` oder `"lang_keys"`), `count` | Der Dateiwächter hat nach einer Änderung von `.rs`/`.ftl` ein TypeScript-Artefakt neu geschrieben. Wird nur ausgelöst, wenn sich die generierte Datei tatsächlich geändert hat: Eine `.rs`-Änderung, die das ausgegebene TypeScript Byte für Byte gleich lässt, schreibt nichts und gibt nichts aus, ein Ereignis heißt also immer, dass die Datei auf der Platte jetzt anders ist. `count` ist die Anzahl der Strukturen (oder Message-Ids) in der neu geschriebenen Datei, nicht die Anzahl derer, die sich geändert haben. |
| `shutdown` | `ts` | Die Sitzung wird heruntergefahren. Immer die letzte Zeile. |

Beispielsweise sehen ein Vite-Absturz und sein Neustart so aus:

```json
{"type":"exited","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","code":1}
{"type":"restart_scheduled","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","delay_ms":200}
{"type":"restart_succeeded","ts":"2026-08-18T10:15:23.657-07:00","name":"frontend","pid":48391}
```

`--json` lässt sich mit `--timestamps` kombinieren, statt damit zu kollidieren: Die Kombination ist kein Fehler, aber `--timestamps` hat keine zusätzliche Wirkung, weil jedes Ereignis bereits ein eigenes Feld `ts` enthält.

Dies ist maschinenlesbare Ausgabe, die andere Tools parsen - Feldnamen und Werte von `type` werden nicht ohne einen Hinweis im Changelog umbenannt oder entfernt. Behandeln Sie einen unbekannten `type` oder ein unerwartetes zusätzliches Feld als etwas, das zu ignorieren ist, nicht als Fehler, damit eine künftige Veröffentlichung das Schema erweitern kann, ohne Ihren Consumer zu beschädigen.

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

### Ein Prozess gerät in eine Absturzschleife

Kann ein Kindprozess - Backend, Frontend oder ein `Suprnova.toml`-Eintrag - nicht starten (fehlerhafter Code, fehlendes Binary, Portkonflikt), wird er nach dem oben beschriebenen Backoff-Zeitplan neu gestartet, statt die Sitzung zu beenden. Prüfen Sie die `[name]`-Zeilen unmittelbar vor jedem Hinweis „respawning in …ms“ auf den tatsächlichen Fehler (ein rustc-`error[E…]`, ein ENOENT oder was immer der Kindprozess ausgegeben hat). Beheben Sie die Ursache; der nächste Neustartversuch übernimmt sie automatisch. Um die Wiederholungsversuche anzuhalten und den Fehler nur einmal zu sehen, führen Sie den Befehl erneut mit `--no-restart` aus - dann wird die Sitzung beim ersten Absturz beendet, wie sich `suprnova serve` verhielt, bevor es diese Funktion gab.

Nach `--restart-tries` (standardmäßig `5`) aufeinanderfolgenden Abstürzen beendet `serve` die Neustartversuche für diesen Prozess selbstständig und gibt eine Meldung mit seinem Namen aus:

```text
gave up restarting `backend` after 5 attempts; fix the error and run `suprnova serve` again
```

Die übrigen Prozesse und die Sitzung selbst laufen weiter - beheben Sie die Ursache und führen Sie `suprnova serve` erneut aus, um den aufgegebenen Prozess wieder zu starten; Sie müssen nicht die gesamte Sitzung neu starten.

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

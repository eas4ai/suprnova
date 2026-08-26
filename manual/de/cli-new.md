# suprnova new

`suprnova new` scaffoldet ein Suprnova-Projekt - eine frische
Cargo-Crate mit Controllern, Routen, Migrationen, einer Inertia-SPA
und einem bereits verdrahteten, funktionierenden Auth-Flow. Führen
Sie es einmal pro App aus und leben Sie danach in `suprnova serve`.

## Verwendung

```bash
suprnova new [name] [options]
```

Wird `name` weggelassen, fragt der interaktive Wizard danach. Der
Name wird zum Projektverzeichnis, zum Cargo-Paketnamen (nach der
Umwandlung in snake_case) und zum Standard-`APP_NAME` in `.env`.
Namen müssen aus ASCII-Buchstaben/-Ziffern/`-`/`_` bestehen, mit
einem Buchstaben beginnen, dürfen keine Pfadtrenner oder `..`
enthalten und höchstens 64 Zeichen lang sein.

## Optionen

| Option | Beschreibung |
|---|---|
| `--frontend <svelte\|react\|vue>` | Wählt das SPA-Framework nicht-interaktiv. Schließt sich mit `--api` aus. |
| `--api` | Scaffoldet ein reines JSON:API-Projekt (kein Inertia, keine SPA, Token-Auth statt Sessions). |
| `--no-interaction` | Überspringt alle Eingabeaufforderungen und verwendet die Standardwerte (Name `my-suprnova-app`, Frontend `svelte`, leerer Autor/leere Beschreibung). |
| `--no-git` | Überspringt `git init` im neuen Projekt. |
| `--with-portless` | Gibt eine `portless.json` aus, sodass [`suprnova dev:tls`](dev-tls.md) die App unter `https://<name>.localhost` bedienen kann. Opt-in; ändert sonst nichts. |

## Interaktiver Modus

```bash
suprnova new my-app
```

Der Wizard stellt der Reihe nach vier Fragen:

1. **Projektname** - Standardwert ist das Verzeichnis-Argument
   (`my-app`)
2. **Beschreibung** - wird als Cargo-Paketbeschreibung verwendet
3. **Autor** - wird als Cargo-Paketautor verwendet; Standardwert ist
   Ihr `git config user.name <name@email>`, falls gesetzt
4. **Frontend-Framework** - `Svelte (recommended)`, `React` oder
   `Vue`

Nach der Bestätigung schreibt der Scaffolder das Projekt, führt
`git init` aus (sofern nicht `--no-git`) und gibt die nächsten
Schritte aus:

```
Backend  http://localhost:8765
Frontend http://localhost:5765
```

## Nicht-interaktiver Modus

Für CI, Dotfiles oder skriptgesteuertes Setup übergeben Sie
`--no-interaction` plus die Flags, die Sie überschreiben möchten:

```bash
suprnova new my-app --frontend svelte --no-interaction
```

Standardwerte unter `--no-interaction`:

- Frontend: `svelte`
- Beschreibung: `"A web application built with Suprnova"`
- Autor: leer
- Git: initialisiert

Es gibt keine Flags `--description` oder `--author`; diese Werte
werden nur über die interaktiven Eingabeaufforderungen gesetzt oder
übernehmen ihre Standardwerte.

## Reines API-Projekt

Für Service-Backends ohne SPA verwenden Sie `--api`:

```bash
suprnova new my-api --api
```

Der API-Starter ist erheblich schlanker: kein `frontend/`-Verzeichnis, kein Inertia, keine Auth-Ansichten und ein Single-Crate-Layout mit `src/main.rs`. Er initialisiert Magnetar mit der gemeinsam genutzten SeaORM-Verbindung, erstellt das kanonische Modell `app_users`, installiert `BearerTokenMiddleware` und verwendet `Auth::password()` für Registrierung und Anmeldung. `PASSKEY_RP_ID` und `PASSKEY_RP_ORIGIN` werden vom generierten Bootstrap mit lokalen Standardwerten gelesen. Der Starter enthält außerdem einen beispielhaften Benutzer-Controller und den JSON-Serialisierer `UserResource` und bindet in `.env` an Port 8765.

`--api` schließt sich mit `--frontend` gegenseitig aus; werden beide
übergeben, gibt es einen Fehler. Unter `--api` wird nur nach dem
Projektnamen gefragt - die Eingabeaufforderungen für
Beschreibung/Autor/Frontend werden übersprungen.

## Was per Scaffold erzeugt wird

Eine vollständige Verzeichnistour steht in
[Verzeichnisstruktur](structure.md); die Kurzfassung:

- `cmd/main.rs` - Binary-Einstieg; ruft `Application::new()…run()` auf
- `src/` - Controller, Aktionen, Befehle, Konfiguration, Middleware, Modelle, Migrationen sowie `bootstrap.rs` und `routes.rs`. Die generierte `bootstrap.rs` verdrahtet die globale Middleware-Kette - Logging, Session, Locale, CSRF und Include-Parsing - und ruft [`Inertia::install`](frontend-inertia-responses.md) auf, das die Inertia-Protokoll-Middleware hinzufügt (Asset-Version `409`, `302 → 303` bei Weiterleitungen außerhalb von GET). Die beworbene Asset-Version ist standardmäßig ein Hash des Vite-Build-Manifests, sodass die Auslieferung eines Frontend-Builds sie automatisch ändert - siehe [Versionserkennung](frontend-inertia-responses.md). Derselbe Aufruf fixiert das von Ihnen gescaffoldete Frontend, sodass die HTML-Shell den Vite-Einstiegspunkt dieses Frameworks lädt; `.env` enthält das passende `SUPRNOVA_FRONTEND` für die eigenen Generatoren der CLI
- `src/bin/console.rs` - das projektspezifische Analogon zu
  `php artisan`
- `frontend/` - Vite 8 + Tailwind v4 + Ihr gewähltes Framework, mit
  bereits über Inertia verdrahteten Seiten Home / Dashboard / Login /
  Register
- `src/migrations/` - die Tabellen `users`, `sessions` und
  `remember_tokens`, startbereit
- `.env` - standardmäßig SQLite-Datenbank, mit frisch generiertem
  `APP_KEY`, sodass die App ohne Eingriff des Betreibers bootet
- `.gitignore`, `Cargo.toml`

### Warum Suprnova abweicht

Laravel liefert Blade mit und zieht ein Frontend nachträglich über
Breeze/Jetstream herein. Suprnova geht den anderen Weg: `suprnova new`
scaffoldet immer entweder eine echte SPA (Svelte/React/Vue auf
Inertia) oder ein echtes JSON:API-Projekt. Es gibt keinen Starter, bei
dem die Template-Engine an erster Stelle steht - wenn Sie
servergerendertes HTML wollen, steht Tera bereit, aber es ist nicht
die Standardform, und es gibt keinen Scaffolder-Pfad, der Views an die
Front Ihrer App stellt.

Das Standard-Frontend ist **Svelte 5** (runes-on), nicht React. Wir
haben es gewählt, weil es zur Laufzeit das leichteste der drei ist und
der Philosophie „Compile-Zeit-Gewinne vor Cleverness zur Laufzeit“ des
Frameworks am nächsten kommt. React und Vue sind gleichermaßen
erstklassig - wählen Sie, was Ihr Team kennt.

## Verteilung

Die CLI selbst wird über Git ausgeliefert, nicht über crates.io
(Pre-Launch):

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.3 suprnova-cli
```

`--force` auf demselben Befehl aktualisiert eine bestehende
Installation. Per Scaffold erzeugte Projekte hängen genauso vom
Framework-Crate ab - über eine Git-Abhängigkeit in ihrer `Cargo.toml`,
gepinnt auf den aktuellen Release-Tag. Siehe
[Installation](installation.md) für die vollständigen
Toolchain-Voraussetzungen.

## Nächste Schritte

- [Installation](installation.md) - Rust-/Node-/DB-Voraussetzungen
  und Toolchain-Einrichtung
- [Verzeichnisstruktur](structure.md) - was jede gescaffoldete Datei
  tut
- [Schnellstart](quickstart.md) - die ersten 5 Minuten nach
  `suprnova new`
- [suprnova serve](cli-serve.md) - der Dev Runner, den Sie als
  Nächstes nutzen
- [Konsole](console.md) - `cargo run --bin console` und das
  `#[command]`-System

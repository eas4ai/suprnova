# Installation

Dieses Kapitel führt Sie von "kein Suprnova auf dieser Maschine" zu einem laufenden,
generierten Projekt. Wenn Sie bereits soweit sind, springen Sie zum
[Schnellstart](quickstart.md).

## Anforderungen

- **Rust 1.91.1+** (der Workspace verwendet die 2024-Edition). Installieren Sie über
  [rustup](https://rustup.rs/):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Node.js 20+** und **npm** (oder pnpm/yarn/bun) für die Frontend-Toolchain. Suprnova
  nutzt Vite 8 und Ihr Starter enthält TypeScript + Tailwind v4. Installieren Sie über
  [nodejs.org](https://nodejs.org/) oder Ihren Package Manager.
- **Eine Datenbank-Clientbibliothek**, die zum gewünschten Treiber passt:
  - SQLite - keine zusätzlichen Anforderungen; sqlite ist gebündelt
  - PostgreSQL - `libpq` auf den meisten Systemen (oft vorinstalliert)
  - MySQL oder MariaDB - `libmariadb` / `libmysqlclient` auf den meisten Systemen

Sie müssen sich jetzt noch nicht für eine Datenbank entscheiden. Der Standard-Scaffolder wählt
SQLite, sodass eine neue App ohne Konfiguration läuft.

## CLI installieren

Suprnova wird als Cargo-Projekt verteilt, und der CLI-Installer ruft das
Framework aus Git ab (nicht von crates.io - siehe die [Notiz vor dem Start](#pre-launch-note) unten):

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.2 suprnova-cli
```

Dies kompiliert die `suprnova`-Binärdatei und platziert sie in `~/.cargo/bin`.
Überprüfen Sie, ob es funktioniert hat:

```bash
suprnova --version
```

Sie sollten `suprnova 0.x.x` sehen.

Wenn `suprnova` nicht gefunden wird, ist `~/.cargo/bin` nicht in `PATH`. Fügen Sie dies
Ihrer Shell-Konfiguration hinzu:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Ein Projekt erstellen

`suprnova new` generiert ein vollständiges Projekt - Backend + gewähltes Frontend +
Vite-Konfiguration + Auth-Migrationen + Beispielrouten. Es ist standardmäßig interaktiv:

```bash
suprnova new my-app
```

Der Wizard fragt der Reihe nach:

1. **Projektname** - wird übersprungen, wenn Sie ihn als Argument übergeben (`my-app`)
2. **Beschreibung** - wird in `Cargo.toml` verwendet
3. **Autor** - wird in `Cargo.toml` verwendet; Standardwert ist Ihr Git `user.name`
4. **Frontend-Framework** - eines von `svelte` (Standard), `react`, `vue`

Wenn Sie die Eingabeaufforderungen überspringen möchten (CI, automatisierte Einrichtung), übergeben Sie
`--no-interaction` und wählen Sie ein Frontend explizit:

```bash
suprnova new my-app --frontend svelte --no-interaction
```

`--no-interaction` akzeptiert die Standardwerte für Beschreibung ("A web
application built with Suprnova") und Autor (leer). Um diese zu setzen,
bearbeiten Sie die generierte `Cargo.toml` nach der Generierung.

Die drei Frontend-Optionen bringen jeweils ihre eigenen runes-on/Svelte-5,
React-19 oder Vue-3.5 Starter mit. Alle drei nutzen Inertia v3 + Vite 8 +
Tailwind v4 und haben einen vorgefertigten Login/Register/Dashboard-Flow mit
sitzungsbasierter Authentifizierung.

Suprnova bietet auch einen schlankereren **API-Starter** für Service-Backends
ohne SPA:

```bash
suprnova new my-api --api
```

Der API-Starter hat den gleichen Backend-Stack, aber kein Frontend, kein Inertia
und nutzt Token-basierte Authentifizierung statt Session-Cookies.

## Erste Ausführung

```bash
cd my-app

# Migrationen ausführen (users, sessions, etc.)
suprnova migrate

# Frontend-Abhängigkeiten installieren
npm install              # im Projektstamm

# Backend und Vite zusammen starten
suprnova serve
```

`suprnova serve` startet das Backend unter `http://127.0.0.1:8765` und Vite
unter `http://127.0.0.1:5765`. Öffnen Sie die Backend-URL - Vite wird als Proxy
verwendet, sodass Sie es nicht direkt besuchen müssen.

Sie sollten die Willkommensseite sehen. Besuchen Sie dann `/register`, um ein
Konto zu erstellen, und `/login`, um sich anzumelden.

## Was wurde generiert

```
my-app/
├── Cargo.toml          # Crate-Manifest, zwei [[bin]]-Ziele
├── .env                # Lokale Konfiguration (DB-URL, App-Schlüssel, Ports)
├── .env.example        # Vorlage für Ops/CI
├── .gitignore
├── cmd/
│   └── main.rs         # Binary-Einstiegspunkt; ruft Application::new().run() auf
├── src/
│   ├── lib.rs          # Modul-Verdrahtung
│   ├── bootstrap.rs    # Service-Registrierung (das Suprnova-Äquivalent von Providern)
│   ├── routes.rs       # Der routes!-Makrobaum
│   ├── bin/
│   │   └── console.rs  # `cargo run --bin console <subcommand>`
│   ├── actions/        # Ein-Methoden-aufrufbare Controller
│   ├── commands/       # Mit `#[command]` kommentierte Handler
│   ├── config/         # Typisierte Konfigurationsabschnitte (Datenbank, Mail)
│   ├── controllers/    # home, auth, dashboard
│   ├── middleware/     # logging, authenticate
│   ├── migrations/     # SeaORM-Migratoren (users, sessions usw.)
│   └── models/         # `#[suprnova::model]`-Strukturen (user)
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html
│   └── src/
│       ├── main.{tsx,ts}
│       ├── app.css
│       ├── pages/
│       │   ├── Home, Dashboard
│       │   └── auth/{Login,Register}
│       └── types/
│           └── inertia-props.ts
└── public/
    └── assets/         # Vite-Produktions-Build-Ausgabe
```

Die vollständige Verzeichnisübersicht finden Sie unter [Verzeichnisstruktur](structure.md).

## CLI aktualisieren

Die CLI befindet sich in Ihrem `~/.cargo/bin`. Um auf die neueste Version zu aktualisieren:

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.2.2 suprnova-cli
```

`--force` veranlasst Cargo, die bestehende Binärdatei zu überschreiben.

## Framework-Version der App aktualisieren

Eine generierte App hängt vom `suprnova`-Framework-Crate über eine Git-Abhängigkeit in
`Cargo.toml` ab:

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.2" }
```

Um die neuesten Framework-Änderungen zu abrufen:

```bash
cargo update -p suprnova
```

Die Git-Abhängigkeit folgt dem benannten Release-Tag. Aktualisieren Sie den Tag in
`Cargo.toml`, dann führen Sie `cargo update -p suprnova` aus; Ihre `Cargo.lock` speichert
den exakten Commit, auf den es aufgelöst wurde, sodass Builds zwischen Updates reproduzierbar
bleiben - kein manuelles Festlegen eines `rev` in `Cargo.toml` erforderlich.

## Verteilungsmodell

Suprnova wird über Git verteilt, nicht über crates.io - sowohl das Framework
als auch die CLI werden von GitHub installiert. Jede Version wird als markiertes
GitHub Release (z.B. `v0.7.2`) für das Änderungsprotokoll veröffentlicht, aber Sie hängen nicht
von dem Tag ab: die Git-Abhängigkeit folgt dem Standardbranch, und `Cargo.lock`
pin den exakten Commit, auf den Ihre App aufgelöst wurde, sodass Builds zwischen
`cargo update`-Läufen reproduzierbar sind - kein manuelles Festlegen eines
`tag` oder `rev` erforderlich.

## Editor-Setup

Ein paar VS Code-Erweiterungen machen die Erfahrung reibungsloser:

- **rust-analyzer** - der Rust-Sprachserver
- **Svelte for VS Code** (oder React/Vue, wenn Sie diese gewählt haben)
- **Tailwind CSS IntelliSense**
- **Even Better TOML**

`rust-analyzer` indexiert das Projekt beim ersten Öffnen; erwarten Sie 1-2
Minuten beim ersten Mal, dann inkrementell.

## Nächste Schritte

- [Schnellstart](quickstart.md) - bauen Sie in 5 Minuten eine kleine App
- [Verzeichnisstruktur](structure.md) - was in jeder Datei des generierten
  Scaffolders enthalten ist
- [Konfiguration](configuration.md) - die `.env` und typed config Story
- [Routing](routing.md) - fügen Sie Ihre erste Route hinzu

# Installation

Dieses Kapitel führt Sie von "kein Suprnova auf dieser Maschine" zu einem laufenden,
generierten Projekt. Wenn Sie bereits soweit sind, springen Sie zum
[Schnellstart](quickstart.md).

## Anforderungen

- **Rust 1.94.0+** für den aktuellen `main`-Branch (der Workspace verwendet die Edition 2024). Das getaggte Release v1.3.4 hat dieselbe Mindestversion Rust 1.94.0. Installation über [rustup](https://rustup.rs/):
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


Der aktuelle `main`-Branch verwendet SeaORM 2.0, SeaQuery 1.0 und SQLx 0.9. Anwendungen, die SeaORM direkt aufrufen, müssen `ExprTrait` für SeaQuery-Ausdrucksmethoden importieren und explizite `*_raw`-Verbindungsmethoden für vorab erstellte `Statement`-Werte verwenden. Das Abhängigkeits-Upgrade erfordert keine Migration der Anwendungsdaten.

## Die CLI installieren

Suprnova wird als Cargo-Projekt verteilt, und der CLI-Installer holt das
Framework aus Git (nicht von crates.io - siehe den
[Pre-Launch-Hinweis](#pre-launch-note) weiter unten):

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.4 suprnova-cli
```

Das kompiliert das `suprnova`-Binary und legt es in `~/.cargo/bin` ab.
Prüfen Sie, ob es funktioniert hat:

```bash
suprnova --version
```

Sie sollten `suprnova 0.x.x` sehen.

Wenn `suprnova` nicht gefunden wird, liegt Ihr `~/.cargo/bin` nicht auf dem
`PATH`. Fügen Sie dies Ihrer Shell-Konfiguration hinzu:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Erstellen Sie ein Projekt

`suprnova new` setzt ein komplettes Projekt auf - Backend + gewählte Frontend + Schnelle Konfiguration + Auth-Migrationen + Stichprobenrouten. Es ist standardmäßig interaktiv:

```bash
suprnova new my-app
```

Der Zauberer fragt nach:

1. **Projektname** - übersprungen, wenn Sie es als Argument übergeben (`my-app`)
2. **Beschreibung** - verwendet in `Cargo.toml`
3. **Author** - verwendet in `Cargo.toml`; Standard für Ihre Git `user.name`
4. **Frontend-Framework** - einer der `svelte` (Standard) `react` `vue`

Wenn Sie die Anfragen (CI, scripted setup) überspringen möchten, passieren Sie `--no-interaction` und wählen Sie explizit ein Frontend aus:

```bash
suprnova new my-app --frontend svelte --no-interaction
```

`--no-interaction` akzeptiert die Standardbeschreibungen ("Eine Webanwendung, die mit Suprnova erstellt wurde") und den Autor (leer). Um diese einzusetzen, bearbeiten Sie die generierte `Cargo.toml` nach dem Scaffold.

Die drei Frontend-Optionen versenden jeweils ihre eigenen Runen-on/Svelte-5, React-19 oder Vue-3.5 Starter. Alle drei verwenden Inertia v3 + Vite 8 + Tailwind v4 und vorwirbeln einen Login/Register/Dashboard-Fluss mit sessbasierter Auth.

Suprnova liefert auch einen schlankeren **API-Starter** für Service-Backends ohne SPA:

```bash
suprnova new my-api --api
```

Der API-Starter hat keine Frontend- oder Inertia-Schicht. Er initialisiert
Magnetar auf der Anwendungsdatenbank, installiert `BearerTokenMiddleware` und
scaffoldet Passwortregistrierung sowie Login für `app_users`.

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

## Die CLI aktualisieren

Die CLI liegt in Ihrem `~/.cargo/bin`. Um auf die neueste Version zu
aktualisieren:

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.3.4 suprnova-cli
```

`--force` lässt Cargo das bestehende Binary überschreiben.

## Die Framework-Version Ihrer App aktualisieren

Eine per Scaffold erzeugte App hängt über eine Git-Abhängigkeit in
`Cargo.toml` vom `suprnova`-Framework-Crate ab:

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.4" }
```

Um die neuesten Framework-Änderungen zu holen:

```bash
cargo update -p suprnova
```

Die Git-Abhängigkeit folgt dem benannten Release-Tag. Aktualisieren Sie den
Tag in `Cargo.toml` und führen Sie dann `cargo update -p suprnova` aus; Ihre
`Cargo.lock` verzeichnet den exakten Commit, auf den sie aufgelöst hat, sodass
Builds zwischen Updates reproduzierbar bleiben - ein `rev` muss in
`Cargo.toml` nicht von Hand gepinnt werden.

## Verteilungsmodell

Suprnova wird über Git verteilt, nicht über crates.io - sowohl das
Framework als auch die CLI installieren von GitHub. Jede Version wird als
getaggtes GitHub Release veröffentlicht (z. B. `v1.2.4`), und der Tag ist
das, wovon Ihre App abhängt: Eine per Scaffold erzeugte `Cargo.toml` pinnt
`tag = "v1.3.4"`, und `Cargo.lock` verzeichnet den exakten Commit, auf den
dieser Tag aufgelöst hat, sodass Builds reproduzierbar sind, bis Sie sich
entscheiden, weiterzugehen. Das Aktualisieren ist eine bewusste
Entscheidung, nie ein Nebeneffekt - erhöhen Sie den Tag und führen Sie
`cargo update -p suprnova` aus; der Abschnitt zum Aktualisieren der
Framework-Version Ihrer App führt Sie Schritt für Schritt hindurch.

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

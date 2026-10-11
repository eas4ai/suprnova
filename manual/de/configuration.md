# Konfiguration

Suprnova liest die Konfiguration aus Umgebungsvariablen (geladen aus
`.env` während der Entwicklung, aus der Prozessumgebung in der Produktion) und
stellt sie Ihrem Code in zwei Formen zur Verfügung:

1. **Direkter Zugriff auf Umgebungsvariablen** - `env::env`, `env_required`, `env_optional`
   für einmalige Abfragen
2. **Typisierte Konfigurationsstrukturen** - `Config::register` / `Config::get` für
   alles, das Sie mehr als einmal lesen, mit starker Typisierung

Das Framework liest selbst eine Handvoll Umgebungsvariablen (`APP_KEY`,
`APP_ENV`, `DATABASE_URL` usw.); der Rest gehört Ihnen.

## Die `.env`-Datei

`suprnova new` schreibt eine erste `.env`-Datei mit den Werten, die Ihre App
zum Starten benötigt:

```env
APP_NAME="my-app"
APP_ENV=local                # local, development, staging, production, testing, …
APP_DEBUG=true               # detaillierte Fehlerseiten + ausführliche Protokolle
APP_URL=http://localhost:8765

# 32-byte AES-256 key (URL-safe base64, no padding). Encrypts session
# cookies, pagination cursors, and anything via `suprnova::Crypt`.
# Generated at scaffold time. Rotate with `suprnova key:generate`.
APP_KEY=<32-byte base64>

SERVER_HOST=127.0.0.1
SERVER_PORT=8765
VITE_PORT=5765

# Datenbank - standardmäßig SQLite; wechseln zu postgres://user:pass@host/db
DATABASE_URL=sqlite://./database.db
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1
DB_CONNECT_TIMEOUT=30
DB_LOGGING=false

# Session
SESSION_LIFETIME=120         # Minuten
SESSION_COOKIE=suprnova_session
SESSION_SECURE=false         # in Produktion auf true setzen (nur HTTPS)
SESSION_PATH=/
SESSION_SAME_SITE=Lax

# Mail - verwendet standardmäßig den `log`-Treiber (schreibt ausgehende E-Mails in das
# Tracing-Protokoll, gut für Entwicklung). Setzen Sie MAIL_DRIVER auf einen der
# smtp / ses / mailgun / postmark / sendgrid / resend / log / memory
# für Produktion.
MAIL_DRIVER=log
# SMTP-Anmeldedaten (nur gelesen, wenn MAIL_DRIVER=smtp):
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=
MAIL_SMTP_PASS=
# starttls | tls | none. Wenn leer, leitet es sich von den Anmeldedaten ab
# oben - starttls damit, nichts ohne. Produktion weigert sich zu starten
# unverschlüsselt; siehe das Mail-Kapitel.
MAIL_SMTP_ENCRYPTION=
```

Eine entsprechende `.env.example`-Datei enthält die gleichen Schlüssel mit Platzhalter-Werten -
committen Sie diese; committen Sie nicht `.env`. Die Standard-`.gitignore` schließt
`.env` bereits aus.

## Wie `.env` geladen wird

Beim Starten tut das Framework Folgendes:

1. Erkennt die Umgebung aus `APP_ENV` (ohne Unterscheidung von Groß- und Kleinschreibung,
   `prod`/`dev`/`stage`/`stg`/`test` werden auch erkannt).
2. Lädt `.env` aus dem Projektstammverzeichnis.
3. Wenn eine umgebungsspezifische Datei vorhanden ist (`.env.staging`, `.env.production`),
   lädt sie diese darüber - ihre Werte überschreiben `.env`.
4. Echte Prozessumgebungsvariablen überschreiben beide (dies ist, worauf
   Container-Orchestrierung angewiesen ist).

Die Reihenfolge in einer Zeile: **Prozess-Umgebung > `.env.<environment>` > `.env`**.

```rust
use suprnova::Config;

let env = Config::environment();           // Environment::Local
let is_prod = Config::is_production();     // false
```

Bei einer CI-Ausführung mit `APP_ENV=testing` lädt das Framework `.env.testing`
über `.env`, sodass Sie DB-URLs überschreiben und Mail-Treiber deaktivieren können
ohne die Entwicklungs-`.env` zu ändern.

## Direkter Umgebungszugriff

Für einmalige Lesevorgänge von Zeichenketten, Zahlen, Booleans - alles, das
`std::str::FromStr` implementiert - verwenden Sie die `env::*`-Familie:

```rust
use suprnova::config::{env, env_required, env_optional};

let port: u16 = env("SERVER_PORT", 8765);                    // mit Standardwert
let url: String = env_required("APP_URL");                   // löst einen Panic aus, wenn er fehlt - nur beim Boot
let smtp_host: Option<String> = env_optional("MAIL_HOST");   // None, wenn er fehlt
```

- `env(key, default)` - typisierter Lesevorgang mit Fallback
- `env_required(key)` - panikiert, wenn der Schlüssel fehlt oder nicht
  geparst werden kann. Verwenden Sie dies nur beim Starten (in `bootstrap()` oder `config::register()`)
  wo ein fehlender erforderlicher Wert den Prozess sofort beenden sollte
- `env_optional(key)` - gibt `Option<T>` zurück; `None` für fehlende oder
  nicht analysierbare Werte

Jeder eindeutige Schlüssel wird auch beim ersten Lesevorgang einmal protokolliert, sodass Sie
genau überprüfen können, welche Umgebungsvariablen Ihre App berührt.

## Typisierte Konfigurationsstrukturen

Für alles, das Ihre App mehr als einmal liest, definieren Sie eine typisierte Struktur
und registrieren Sie diese. Das Muster ist:

```rust
// src/config/database.rs
use suprnova::Config;
use suprnova::config::{env, env_required, env_optional};

#[derive(Clone, Debug)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u32,
    pub logging: bool,
}

pub fn register() {
    Config::register(DatabaseConfig {
        url: env_required("DATABASE_URL"),
        max_connections: env("DB_MAX_CONNECTIONS", 10),
        min_connections: env("DB_MIN_CONNECTIONS", 1),
        connect_timeout_secs: env("DB_CONNECT_TIMEOUT", 30),
        logging: env("DB_LOGGING", false),
    });
}
```

Dann lesen Sie es überall mit einer Zeile:

```rust
let db = Config::get::<DatabaseConfig>().expect("DB config registered at boot");
println!("Pool size: {}", db.max_connections);
```

Das Register wird durch `TypeId` indiziert, sodass jede Struktur einmal gespeichert wird.
Das erneute Aufrufen von `Config::register` mit demselben Typ ersetzt den
vorherigen Eintrag - praktisch für Tests.

### Registrierung in Ihre App verdrahten

Das Scaffold der `cmd/main.rs` enthält einen `.config(…)`-Schritt in der
fließenden Boot-Pipeline:

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)   // ← das ruft Ihre Registrierung auf
        .bootstrap(my_app::bootstrap::register)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

`my_app::config::register` delegiert normalerweise an jedes Abschnittsmodul:

```rust
// src/config/mod.rs
pub mod database;
pub mod mail;

pub fn register() {
    database::register();
    mail::register();
}
```

### Deserialisierung von ganzen Strukturen aus Umgebungsvariablen

Bei größeren Konfigurationen können Sie direkt aus Umgebungsvariablen über
`serde` deserialisieren. Suprnova stellt zwei Helfer bereit:

```rust
use suprnova::Config;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

// Liest SERVER_HOST / SERVER_PORT aus der Umgebung
let cfg = Config::resolve_prefixed::<ServerConfig>("SERVER_")?;
```

- `Config::resolve::<T>()` - deserialisiere aus allen Prozessumgebungsvariablen
- `Config::resolve_prefixed::<T>("PREFIX_")` - deserialisiere nur
  Variablen mit dem angegebenen Präfix (das Präfix wird entfernt, bevor
  die Deserialisierung stattfindet)

Beide geben `Result<T, FrameworkError>` zurück, sodass ein fehlendes erforderliches Feld
als `FrameworkError::Internal` mit der envy-Diagnose anstelle eines Panics auftritt.

## Umgebungsspezifische Konfiguration

Das `Environment`-Enum deckt den Standard-Satz ab:

| Variante | Erkannte `APP_ENV`-Werte |
|---|---|
| `Local` | `local` |
| `Development` | `development`, `dev` |
| `Staging` | `staging`, `stage`, `stg` |
| `Production` | `production`, `prod` |
| `Testing` | `testing`, `test` |
| `Custom(String)` | alles andere (behält Ihre Groß-/Kleinschreibung bei, wird für `.env.<custom>`-Lookup verwendet) |

Häufige Verzweigungen:

```rust
use suprnova::{Config, Environment};

if Config::is_production() {
    // strikte Cookies, echter Mail-Treiber usw.
}

if Config::is_debug() {
    // ausführliche Fehlerseiten, Query-Protokollierung
}

match Config::environment() {
    Environment::Production => { /* … */ },
    Environment::Staging    => { /* … */ },
    _ => { /* dev/test path */ },
}
```

`is_debug()` gibt `true` zurück, wenn `APP_DEBUG=true` ausdrücklich gesetzt ist,
oder - wenn `APP_DEBUG` nicht gesetzt ist - wenn die erkannte Umgebung
`Local`, `Development` oder `Testing` ist. Produktion, Staging und jede
nicht erkannte benutzerdefinierte Umgebung sind standardmäßig `false`. Halten Sie es in der
Produktion aus; es kontrolliert Fehlerseiten-Details und einige interne Standardwerte.

### `APP_KEY` ist in der Nicht-Entwicklung erforderlich

In der Produktion (jedes `APP_ENV` außer `local`/`development`/
`testing`) erfordert Suprnova, dass `APP_KEY` auf eine gültige 32-Byte-
URL-sichere base64-Zeichenkette gesetzt ist. Das Starten ohne diese Komponente schlägt fehl, indem
eine aussagekräftige Fehlermeldung angezeigt wird - es gibt keine stille Fallback.

Wenn Sie noch keinen `APP_KEY` haben:

```bash
suprnova key:generate          # gibt den Schlüssel mit einem Hinweis aus, ihn in .env einzutragen
suprnova key:generate --show   # gibt nur den Schlüssel aus, geeignet für `APP_KEY=$(suprnova key:generate --show)`
```

Keine Form bearbeitet `.env` für Sie - kopieren Sie den ausgedruckten Schlüssel selbst in Ihre
`.env`-Datei (oder Ihren Secrets Manager).

Zur Schlüsselrotation (wobei alte verschlüsselte Daten während des
Migrationsfensters noch entschlüsselt werden können), siehe [Verschlüsselung](encryption.md#key-rotation).

## Konfiguration in Tests

Registrieren Sie die Konfiguration in Tests im Test-Setup, anstatt sich auf
`.env` zu verlassen:

```rust
use suprnova::suprnova_test;

#[suprnova_test]
async fn test_with_custom_db() {
    suprnova::Config::register(DatabaseConfig {
        url: "sqlite::memory:".to_string(),
        max_connections: 1,
        min_connections: 1,
        connect_timeout_secs: 5,
        logging: false,
    });

    // … Ihr Test
}
```

Das `#[suprnova_test]`-Attribut richtet auch einen isolierten Container-Zustand ein,
sodass gleichzeitige Tests die Bindungen der anderen nicht sehen - siehe
[Testen](testing.md).

## Häufig von Suprnova gelesene Umgebungsvariablen

Eine nicht vollständige Liste - dies sind Variablen, auf die das Framework selbst schaut.
Ihre App liest darüber hinaus mehr.

| Variable | Standard | Was es tut |
|---|---|---|
| `APP_NAME` | `"app"` | Beim Starten protokolliert, in einigen Standard-Fehlermeldungen verwendet |
| `APP_ENV` | `local` | Steuert `Environment::detect` und `.env.<suffix>`-Lookup |
| `APP_DEBUG` | umgebungsbewusst (`false` in Produktion) | Ausführliche Fehlerseiten + zusätzliches Protokollieren |
| `APP_URL` | `http://localhost:8765` | Basis-URL für die Generierung absoluter URLs, signierte URLs |
| `APP_KEY` | keine (erforderlich in Produktion) | AES-256-Schlüssel für `Crypt`, Sessions, Cursor |
| `APP_KEY_PREVIOUS` | keine | Durch Kommas getrennte vorherige Schlüssel zur Rotation (max. 8) |
| `SERVER_HOST` | `127.0.0.1` | Bindungsadresse |
| `SERVER_PORT` | `8765` | Bindungsport |
| `DATABASE_URL` | keine | Erforderlich, wenn Ihre App die Datenbank nutzt |
| `DB_MAX_CONNECTIONS` | `10` | sqlx-Pool-Maximum |
| `DB_MIN_CONNECTIONS` | `1` | sqlx-Pool-Minimum |
| `DB_CONNECT_TIMEOUT` | `30` (Sekunden) | sqlx-Pool-Verbindungs-Timeout |
| `SESSION_LIFETIME` | `120` (Minuten) | Session-Ablauf |
| `SESSION_TOUCH_INTERVAL` | `300` (Sekunden) | Minimale Schreib-Kadenz mit gleitender Ablauf |
| `SESSION_GC_INTERVAL` | `3600` (Sekunden) | Überwachte Bereinigungs-Kadenz abgelaufener Sessions |
| `SESSION_COOKIE` | `suprnova_session` | Cookie-Name |
| `SESSION_SECURE` | `true` | Setzen Sie das `Secure`-Cookie-Flag. Für die lokale HTTP-Entwicklung auf `false` überschreiben. |
| `SESSION_SAME_SITE` | `Lax` | `Strict`, `Lax` oder `None` |
| `MAIL_DRIVER` | `log` | Einer von `smtp`, `ses`, `mailgun`, `postmark`, `sendgrid`, `resend`, `log`, `memory` |
| `CACHE_DRIVER` | `memory` | Einer von `memory`, `redis`, `database` |
| `QUEUE_DRIVER` | `memory` | Einer von `memory`, `redis`, `database` (unbekannte Werte warnen und fallen auf `memory` zurück) |
| `RATE_LIMIT_DRIVER` | `memory` | Einer von `memory`, `redis` |
| `LOG_FORMAT` | umgebungsbewusst (`pretty` in Entwicklung/lokal, `json` in Produktion) | `pretty` oder `json` |
| `LOG_LEVEL` | `info` | Einer von `error`, `warn`, `info`, `debug`, `trace` |

Die vollständige überprüfte Liste befindet sich in [Umgebungsvariablen](env-vars.md).

## Nächste Schritte

- [Application Bootstrap](bootstrap.md) - wo die typisierte Konfigurationsregistrierung
  aufgerufen wird
- [Service Container](container.md) - wie registrierte Konfiguration gelesen wird
  neben gebundenen Diensten
- [Umgebungsvariablen](env-vars.md) - die vollständige Referenzliste
- [Bereitstellung](deployment.md) - Umgebungseinrichtung für die Produktion

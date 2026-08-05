# Konsole

Jedes Suprnova-Projekt bringt eine `console`-Binary mit - den
Laufzeit-Kommando-Dispatcher für alles, was die kompilierten Typen der
App braucht: Datenbank-Seeder, Pruner, einmalige Wartungsaufgaben,
alles, was Sie mit Laravels `php artisan` bauen würden. Befehle sind
entweder typisierte Strukturen, die `#[derive(Command)]` tragen
(aufgebaut auf `clap::Parser`), oder asynchrone Funktionen, die mit
`#[command]` annotiert sind; das Framework sammelt sie zur Linkzeit
über `inventory`, sodass ein neuer Befehl nur eine einzelne Datei ist,
ohne eine zentrale Registry zu bearbeiten. Das ist das
Suprnova-Äquivalent von `php artisan` - dasselbe Skript, derselbe
Prozess, derselbe Adressraum, endet, wenn der Handler zurückkehrt.

## Schnellstart

Die empfohlene Form verwendet `#[derive(clap::Parser, Command)]` für
typisierte Args:

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "greet", description = "Print a friendly greeting")]
pub struct Greet {
    #[arg(short, long, default_value = "world")]
    pub name: String,

    #[arg(long, default_value_t = false)]
    pub loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let prefix = if self.loud { "HELLO" } else { "Hello" };
        println!("{prefix}, {}!", self.name);
        Ok(())
    }
}
```

Legen Sie das in `src/commands/greet.rs` ab, fügen Sie `pub mod greet;`
zu `src/commands/mod.rs` hinzu, und führen Sie es aus:

```bash
cargo run --bin console -- greet
# Hello, world!
cargo run --bin console -- greet --name Alice --loud
# HELLO, Alice!
cargo run --bin console -- greet --help
# (von clap generierte Hilfe pro Befehl, einschließlich der typisierten Flags)
```

Keine zentrale Registry zum Bearbeiten. `#[derive(Command)]` reicht
einen `CommandEntry { name, description, clap_builder, handler }` über
inventory ein; die console-Binary ruft
`suprnova::console::dispatch_argv_with_init(argv, init)` auf, was aus
jedem registrierten Eintrag einen einzigen clap-Parser-Baum aufbaut,
die Bootstrap-`init`-Closure nur ausführt, wenn ein echter Subcommand
matcht, und die geparsten `ArgMatches` an den richtigen Handler
weiterleitet.

### Der einfachere Weg: rohes `Vec<String>`

Für triviale Befehle, die keine typisierten Args brauchen, funktioniert
auch das `#[command]`-Attribut auf einer asynchronen Funktion:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "ping", description = "Smoke test")]
pub async fn ping(_args: Vec<String>) -> Result<(), FrameworkError> {
    println!("pong");
    Ok(())
}
```

Unter der Haube landen beide Wege in derselben `CommandEntry`-Registry;
die rohe Form verwendet einfach einen clap-Subcommand mit einem
`trailing_var_arg`, um argv in das `Vec<String>` einzufangen. Bevorzugen
Sie die typisierte Form für jeden Befehl mit Argumenten - Sie bekommen
`--help` pro Befehl, Value-Parsing, Standardwerte und Kurz-/Lang-Flag-
Paare, ohne von Hand einen Parser zu schreiben.

## Die Console-Binary

`suprnova new` scaffoldet zwei Binaries in jedes neue Projekt:

- **`<project>`** (`cmd/main.rs` oder `src/main.rs`) - der HTTP-Server,
  gestartet über `cargo run` oder `suprnova serve`. Langlebig; dient,
  bis er beendet wird.
- **`console`** (`src/bin/console.rs`) - der Laufzeit-Kommando-
  Dispatcher. Einmalig; endet, wenn der Handler zurückkehrt.

Das `main` der Console-Binary ist klein und vorhersehbar:

```rust
use std::process::ExitCode;

#[suprnova::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // Zeigt die Version dieses Projekts über `--version` / `--help`.
    // env! löst zur App-Version des Nutzers auf, nicht zu der des Frameworks.
    suprnova::console::set_version(env!("CARGO_PKG_VERSION"));

    let argv: Vec<String> = std::env::args().collect();
    let result = suprnova::console::dispatch_argv_with_init(argv, || async {
        my_app::config::register_all();
        my_app::bootstrap::register().await;
    })
    .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
```

Tokio läuft in der `current_thread`-Variante - es gibt in einem
einmaligen Befehl keine Arbeit, die sich über Kerne verteilen ließe, und
der Worker-Pool der Multi-Thread-Runtime wäre nur Overhead.

Zwei Dinge sind bemerkenswert:

- **Bootstrap ist lazy.** Die an `dispatch_argv_with_init` übergebene
  Closure läuft nur, wenn clap einen echten registrierten Subcommand
  matcht. `console --help`, `console --version`, ein fehlender
  Subcommand und Parse-Fehler-Pfade überspringen sie alle - sodass
  `console --help` auf einem frischen Checkout funktioniert, der noch
  kein `DATABASE_URL` gesetzt hat.
- **`main` gibt keine Fehler aus.** `dispatch_argv_with_init` besitzt
  das gesamte nutzerseitige stderr - es eprintlnt die Fehlermeldung des
  Handlers (außer der Fehler ist still, wie ein clap-Parse-Fehler, den
  clap bereits ausgegeben hat) und druckt claps eigene Hilfe-,
  Versions- und Parse-Fehler-Ausgabe. `main` ist reine
  `Result → ExitCode`-Übersetzung; ein redundantes `eprintln!`
  hinzuzufügen würde doppelt drucken.

Wenn Sie möchten, dass ein bestimmter Befehl einen teuren
Bootstrap-Schritt vollständig überspringt, gaten Sie den Schritt selbst
über eine Env-Variable, statt ein „Lazy-Bootstrap“-Flag durch das
Framework zu fädeln.

## Eingebaute Befehle

Das Framework registriert selbst eine kleine Menge an Befehlen. Das
Framework in ein Projekt einzubinden zieht sie automatisch mit.

| Befehl        | Was er tut                              |
|---------------|-------------------------------------------|
| `db:seed`     | Führt jeden registrierten `Seeder` der Reihe nach aus. Akzeptiert `--class=<Name>` (oder ein bloßes Positional), um einen einzelnen benannten Seeder auszuführen, passend zu `php artisan db:seed --class=UserSeeder`. |
| `model:prune` | Durchläuft die `PrunerEntry`-Registry und löscht zwangsweise jede Zeile, die jeder registrierte `Prunable`- / `MassPrunable`-Scope zurückgibt. `--model=<Name>` schränkt auf einen Typ ein; `--pretend` meldet die Zeilenzahl, ohne Zeilen zu ändern. |
| `--help` / `-h` | Listet verfügbare Befehle auf; `--help` pro Subcommand wird von clap aus den typisierten Args gebaut. |
| `--version`   | Gibt die über `set_version` registrierte Version aus (typischerweise die `CARGO_PKG_VERSION` Ihrer App). Fehlt vollständig, wenn `set_version` nie aufgerufen wurde. |

`db:seed` führt aus, was auch immer Sie in `bootstrap::register()` mit
`suprnova::seed::register::<MySeeder>()` registriert haben. Bei einer
leeren Registry gibt es eine Warnung aus und liefert `Ok(())` zurück -
`db:seed` aufzurufen, bevor Seeder registriert wurden, ist ein
harmloser Nutzerfehler, kein Programmierfehler.

> Die Worker-Daemons (`queue:work`, `schedule:run`, `schedule:work`,
> `schedule:list`, `workflow:work`) sind **nicht** auf der
> Console-Binary. Sie leben auf dem clap-Parser der App-/Server-Binary
> (derselben Binary, die HTTP bedient). Die globale `suprnova`-CLI
> shellt für diese in `cargo run --quiet -- <name>` aus. Siehe den
> [Asymmetrie-Abschnitt](#asymmetrie-mit-suprnova-migrate) unten.

## Befehle definieren

Zwei Makros, eine Registry. Wählen Sie, was zur Form des Befehls passt.

### `#[derive(Command)]` - typisierte Args (empfohlen)

Sitzt oben auf `#[derive(clap::Parser)]`. Die Felder der Struktur sind
die Args des Befehls; clap parst argv in die Struktur; das Framework
ruft Ihr `TypedCommand::run(self)` auf.

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "users:purge", description = "Purge users older than N days")]
pub struct UsersPurge {
    #[arg(long)]
    pub older_than_days: u32,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for UsersPurge {
    async fn run(self) -> Result<(), FrameworkError> {
        // self.older_than_days, self.dry_run - typisiert, von clap validiert
        Ok(())
    }
}
```

Attribute:

| Attribut    | Erforderlich | Zweck                                       |
|--------------|----------|-----------------------------------------------|
| `#[console(name = "...")]` | ja | Der Aufrufname auf der CLI (`"users:purge"`, `"mail:send"`, `"greet"`). |
| `#[console(description = "...")]` | nein | Einzeilige Beschreibung, gezeigt in der obersten Hilfe. |
| `#[arg(...)]` (clap) | n/a | Claps eigene Feld-Attribute für Kurz-/Lang-Flags, Standardwerte, Value-Parser usw. |

Sie bekommen außerdem claps automatisch generierte Hilfe pro Befehl
(`console users:purge --help`) gratis dazu.

### `#[command]` - rohes `Vec<String>` (einfache Fälle)

Für Befehle, die keine Argumente nehmen oder Positionals nur als Liste
konsumieren, reicht das Attribut auf einer asynchronen Funktion:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "cache:clear", description = "Drop every entry from the cache")]
pub async fn cache_clear(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::Cache::flush().await
}
```

Die annotierte Funktion muss `async fn(Vec<String>) -> Result<(),
FrameworkError>` sein. Das Makro erhält die ursprüngliche Funktion, Sie
können sie also auch direkt aus Rust aufrufen - nützlich für Unit-Tests,
die keine argv-Strings durch den Dispatcher fädeln wollen.

Namen in beiden Formen unterstützen Laravel-artiges Namespacing:
`mail:send`, `queue:work`, `db:fresh`. Der Doppelpunkt ist rein
kosmetisch - er ist eine Zeichenkette, gegen die der Dispatcher
`argv[1]` matcht.

## `suprnova make:command`

Der CLI-Generator legt einen lauffähigen Stub ab. Die generierte Datei
verwendet die **typisierte Form** (`#[derive(Parser, Command)]` +
`impl TypedCommand`) - das ist der empfohlene Standard, und er gibt
Ihnen `--help` pro Befehl gratis dazu:

```bash
suprnova make:command cache:clear
# → src/commands/cache_clear.rs (pub struct CacheClear mit #[console(name = "cache:clear")])
# → src/commands/mod.rs bekommt `pub mod cache_clear;` angehängt (wird erstellt, falls es fehlt)
```

Der Stub ist so, wie er ist, lauffähig - `cargo run --bin console --
cache:clear` gibt `cache:clear: not yet implemented` aus und liefert
`Ok(())` zurück, sodass Sie ihn verdrahten und iterieren können. Füllen
Sie Felder auf der Struktur für typisierte Args aus und ersetzen Sie
den Rumpf von `TypedCommand::run`.

Namensnormalisierung:

| Eingabe          | Datei              | Befehlsname   |
|----------------|-------------------|----------------|
| `greet`        | `greet.rs`        | `greet`        |
| `CleanCache`   | `clean_cache.rs`  | `clean-cache`  |
| `clean-cache`  | `clean_cache.rs`  | `clean-cache`  |
| `mail:send`    | `mail_send.rs`    | `mail:send`    |

Enthält die Eingabe `:`, bleibt der Doppelpunkt-Namespace wörtlich
erhalten. Andernfalls ist der Rust-Funktionsname snake_case und der
Befehlsname kebab-case.

Stellen Sie sicher, dass `pub mod commands;` in `src/lib.rs`
deklariert ist, damit die Inventory-Einreichung von der Console-Binary
aus linkbar erreichbar ist. Der Generator scaffoldet das für neue
Projekte und warnt sichtbar, falls es fehlt; haben Sie es entfernt,
kompiliert der `inventory::submit!`-Block der neuen Datei zwar,
landet aber nie in der Registry.

### Warum Suprnova abweicht

Das Framework macht bewusst **keinen** globalen `suprnova`-CLI-Befehl
für Laufzeitaufgaben wie `db:seed`. Eine globale Binary kann die
Seeder, Factories oder `#[command]`-Async-Funktionen Ihrer App nicht
statisch laden, ohne entweder:

- in `cargo run --bin app -- ...` auszushellen (langsam - vollständiger
  Compile pro Aufruf, verfehlt den Zweck), oder
- dynamisches Laden zu betreiben (zu viel Komplexität für v1)

Also erzeugt das Projekt des Nutzers eine `console`-Binary. Führen Sie
sie direkt aus:

```bash
./target/debug/console db:seed
./target/release/console greet Alice
cargo run --bin console -- mail:send
```

Laravel löst dasselbe Problem mit `php artisan` - einem
Pro-Projekt-Skript, das das Framework bootet und an
nutzerdefinierte Befehle dispatcht. PHP kann das dynamisch tun, weil
der Framework-Code zur Laufzeit neben dem des Nutzers lebt. Rusts
Compile-und-Link-Modell schließt das aus, also liefern wir den
Dispatcher als Library (`suprnova::console::*`) und lassen jedes
Projekt seine eigene einzeilige `console`-Binary linken.

### Asymmetrie mit `suprnova migrate`

Es gibt drei unterschiedliche Befehlsaufrufpfade in einem
Suprnova-Projekt, und die Asymmetrie ist **strukturell** - versuchen
Sie nicht, sie zu vereinheitlichen:

| Befehlsoberfläche                                   | Aufruf                                              | Warum                                                 |
|---------------------------------------------------|---------------------------------------------------------|-----------------------------------------------------|
| `suprnova new`, `suprnova make:*`, `suprnova serve`, `suprnova key:generate`, … | Globale CLI-Binary (installiert über `cargo install --git`) | Reine Datei-Generatoren und Scaffolder; brauchen keinen Nutzercode. |
| `suprnova migrate`, `suprnova migrate:status`, `suprnova schedule:run`, `suprnova schedule:work`, `suprnova schedule:list`, `suprnova workflow:work` | Globale CLI shellt in `cargo run --quiet -- <name>` gegen die App-/Server-Binary aus | Langlebige Daemons und Schema-Arbeit, die demselben `Application::run`-clap-Parser gehören. Das `queue:work` der Server-Binary lebt auch hier - `cargo run --bin <app> -- queue:work`. |
| `console db:seed`, `console model:prune`, `console <your-command>` | Projektspezifische `console`-Binary (`src/bin/console.rs`) | Einmalige Befehle, die Nutzertypen (Seeder, Befehle, prunable Modelle) brauchen, kompiliert in die Crate des Nutzers. |

Die Trennung ist beabsichtigt. Die Server-Binary braucht ohnehin schon
einen clap-Parser, um zwischen `serve`, `migrate`, `queue:work` usw. zu
wählen; Daemons, die ihren Lebenszyklus teilen, leben dort. Die
Console-Binary existiert für alles andere - kurzlebig, nutzerdefiniert,
typreich. Neue Laufzeitbefehle gehören in `#[command]` /
`#[derive(Command)]`, dispatcht von der `console`-Binary des Projekts.

## Best Practices

### Handler klein halten; für geteilte Services zum Container greifen

Ein `#[command]` ist der CLI-förmige Wrapper; die Geschäftslogik sollte
in einer `Action`, einem Service oder einer Methode auf einem Modell
leben. Der Handler parst Args, löst den Service aus dem Container auf
und leitet weiter. Das hält dieselbe Logik testbar aus einem Unit-Test,
einer HTTP-Route und der Console heraus.

```rust
#[command(name = "users:purge")]
pub async fn users_purge(args: Vec<String>) -> Result<(), FrameworkError> {
    let action = App::resolve::<PurgeStaleUsers>()?;
    action.execute(parse(args)?).await
}
```

`App::resolve` gibt `Result<T, FrameworkError::ServiceUnresolved(_)>`
zurück - die `?`-Variante von `App::get` (das `Option` zurückgibt).
Siehe [Service Container](container.md) für die vollständige
Oberfläche.

### Namespaces für verwandte Befehle verwenden

Gruppieren Sie mit `:`: `mail:send`, `mail:retry`, `mail:queue:work`.
Der Dispatcher behandelt es als opak, aber Menschen überfliegen
`mail:*` besser als `send-mail`, `retry-mail`, `mail-queue-work`.

### Keine strukturierten Daten ausgeben - zurückgeben

Console-Handler drucken für menschenlesbare Ausgabe auf stdout. Braucht
ein nachgelagertes Werkzeug die Ausgabe, schreiben Sie eine
`console <name> --json`-Variante, die maschinenlesbares JSON auf stdout
und eine Statuszeile auf stderr ausgibt. Machen Sie den
menschenlesbaren Pfad nicht für beide Zielgruppen verantwortlich.

### Exit-Codes als den Vertrag behandeln

`FrameworkError` → `ExitCode::FAILURE` ist der einzige Fehlerpfad.
Rufen Sie nicht `std::process::exit(custom_code)` aus einem Handler
heraus auf - geben Sie `Err(...)` zurück und lassen Sie das `main` der
Binary übersetzen. Künftiges Tooling (CI-Gates, überwachte Worker)
muss nur den Exit-Code lesen.

## Referenz

| Symbol                                    | Zweck                                       |
|-------------------------------------------|-----------------------------------------------|
| `suprnova::Command` (derive)              | Registriert eine `clap::Parser`-ableitende Struktur als typisierten Console-Befehl. Gehört zu `TypedCommand`. |
| `suprnova::TypedCommand` (trait)          | Trait mit `async fn run(self) -> Result<(), FrameworkError>` - der Rumpf eines typisierten Befehls. |
| `suprnova::command` (attribute)           | Registriert eine asynchrone Funktion, die `Vec<String>` nimmt, als Console-Befehl mit rohen Args. |
| `suprnova::console::dispatch_argv(argv)`  | Baut den clap-Parser-Baum aus jedem registrierten Eintrag, parst argv, leitet an den Handler weiter. Kein Lazy-Init - praktisch für Tests und programmatische Aufrufer. |
| `suprnova::console::dispatch_argv_with_init(argv, init)` | Wie `dispatch_argv`, führt aber die `init`-Closure zwischen claps argv-Parse und dem gematchten Handler aus. Das Init feuert nur, wenn ein echter Subcommand matcht - `--help` / `--version` / Parse-Fehler-Pfade überspringen es. Das ist, was die gescaffoldete `console`-Binary verwendet. |
| `suprnova::console::set_version(&'static str)` | Registriert die über `--version` und in `--help` gezeigte Versions-Zeichenkette. Einmal am Anfang von `main` aufrufen. Die erste Registrierung gewinnt. |
| `suprnova::console::find(name)`           | Sucht einen registrierten Befehl anhand des exakten Namens.   |
| `suprnova::console::list()`               | Alle registrierten Befehle, nach Namen sortiert.      |
| `suprnova::CommandEntry`                  | Inventory-Eintrag: `{ name, description, clap_builder, handler }`. Von beiden Makros eingereicht. |
| `suprnova::CommandHandler`                | Der Handler-Funktionszeiger-Typ: `fn(&clap::ArgMatches) -> Pin<Box<dyn Future<...>>>`. |
| `FrameworkError::silent()` / `.is_silent()` | Konstruiert / erkennt einen Fehler, den der Dispatcher NICHT auf stderr ausgibt. Intern verwendet, um Doppel-Ausgaben zu unterdrücken, wenn clap bereits einen Parse-Fehler ins Terminal geschrieben hat. |

## Nächste Schritte

- [Application Bootstrap](bootstrap.md) - was innerhalb der `dispatch_argv_with_init`-Closure läuft
- [Service Container](container.md) - `App::resolve` vs. `App::get`, und wie ein Handler geteilte Services erreicht
- [Seeding](seeding.md) - was `db:seed` tatsächlich aufruft
- [Eloquent](eloquent.md) - `Prunable`, `MassPrunable`, und wie `model:prune` die Registry durchläuft
- [Task-Planung](scheduling.md) - die Asymmetrie: Scheduler-Daemons leben auf der App-Binary, nicht der Console

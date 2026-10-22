# CLI Migrationen

Die `suprnova`-Entwickler-CLI shellt in Ihre Anwendungs-Binary, um
SeaORMs Migrations-Runner anzutreiben, sodass derselbe
Migrations-Satz ausgeführt wird, egal ob Sie ihn aus einem
Entwickler-Terminal, aus CI oder implizit beim Server-Start
ausführen. Verwenden Sie diese Befehle, um Migrationsdateien zu
verfassen, sie anzuwenden, zurückzurollen und Ihre generierten
SeaORM-Entities mit dem Schema synchron zu halten.

Für die API zum Definieren des Schemas (Spaltentypen, Indizes,
Fremdschlüssel, das vollständige `MigrationTrait`) siehe
[Migrationen](migrations.md). Für das Einfügen von Testdaten,
nachdem das Schema gelandet ist, siehe [Seeding](seeding.md).

## make:migration

Generiert eine neue Migrationsdatei unter `src/migrations/` und
verdrahtet sie in den `Migrator` in `src/migrations/mod.rs`.

```bash
suprnova make:migration <name>
```

`<name>` wird zu snake_case normalisiert. Der Generator erkennt die
Standard-Namensmuster und verwendet sie, um das `DeriveIden`-Enum zu
wählen:

- `create_<table>_table` - scaffoldet einen `create_table`-Rumpf
- `add_<column>_to_<table>` - scaffoldet einen Stub für
  `alter_table`
- `drop_<table>_table` - scaffoldet einen `drop_table`-Rumpf
- alles andere - verwendet den Namen als Tabellen-Identifier

### Beispiele

```bash
suprnova make:migration create_users_table
suprnova make:migration add_email_to_users
suprnova make:migration drop_legacy_sessions_table
```

### Generierte Datei

Die Datei wird nach
`src/migrations/m{YYYYMMDD}_{HHMMSS}_<name>.rs` geschrieben (zum
Beispiel `m20260530_142301_create_users_table.rs`) und zum
`Migrator::migrations()`-Vec hinzugefügt.

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Users::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Users::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Users::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Users::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    CreatedAt,
    UpdatedAt,
}
```

Bearbeiten Sie die generierte Datei, um Ihre Spalten, Indizes und
Constraints zu deklarieren. Siehe [Migrationen](migrations.md) für
die vollständige Schema-Builder-Oberfläche.

## migrate

Führt jede ausstehende Migration in `src/migrations/` aus.

```bash
suprnova migrate
```

Die CLI shellt zu `cargo run -- migrate` aus, sodass die Arbeit vom
`Application`-Runner Ihrer App erledigt wird - dieselbe Binary,
derselbe `Migrator`, dieselbe Datenbankverbindung, die auch `serve`
verwenden würde.

```
Running migrations...
Migrations completed successfully!
```

Der Pfad serve / web:run führt `migrate` automatisch aus, bevor der
Socket gebunden wird, sofern Sie nicht mit `--no-migrate` abwählen
oder `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` setzen, um über einen
Fehlschlag hinweg weiterzumachen. Ein Migrationsfehler während des
Auto-Migrate beendet sich vor dem Booten des Servers mit Non-Zero;
siehe `framework/src/app/mod.rs` für den Fail-Closed-Vertrag.

## migrate:status

Gibt den angewendet/ausstehend-Status jeder Migration aus.

```bash
suprnova migrate:status
```

```
Migration status:
...SeaORM-formatierte Tabelle aus angewendeten/ausstehenden Migrationen...
```

Der Rumpf des Reports kommt von SeaORMs `MigratorTrait::status`,
sodass die genaue Formatierung der SeaORM-Version folgt, von der Ihre
App abhängt.

## migrate:rollback

Rollt die letzte angewendete Migration zurück (oder die letzten `N`).

```bash
suprnova migrate:rollback [--step <N>]
```

| Option | Standard | Beschreibung |
|---|---|---|
| `--step <N>` | `1` | Anzahl der zurückzurollenden Migrationen |

```bash
# Eine Migration zurückrollen
suprnova migrate:rollback

# Die letzten drei zurückrollen
suprnova migrate:rollback --step 3
```

```
Rolling back 3 migration(s)...
Rollback completed successfully!
```

Das `down()` jeder Migration läuft in umgekehrter
Anwendungsreihenfolge. Ein fehlschlagendes `down()` beendet sich mit
Non-Zero und lässt den Rest der Kette unangetastet - es wird nichts
weiter versucht.

## migrate:fresh

Löscht jede Tabelle in der Datenbank und führt jede Migration von
vorne erneut aus.

```bash
suprnova migrate:fresh
```

```
WARNING: Dropping all tables and re-running migrations...
Database refreshed successfully!
```

Das zerstört alle Daten in der verbundenen Datenbank. Es ist für
lokale Entwicklung und Test-Setup gedacht, nicht für eine Umgebung,
in der die Daten wichtig sind.

### Die Produktions-Absicherung

Außerhalb der Produktion läuft es sofort, ohne
Eingabeaufforderung - eine lokale Datenbank zu löschen ist Routine,
und eine Bestätigung, die Sie immer gleich beantworten, trainiert
Sie darauf, sie nicht mehr zu lesen.

Löst sich `APP_ENV` zu production auf, verlangt es zwei
unterschiedliche Arten von Nachweis:

```bash
suprnova migrate:fresh --force   # …dann den Umgebungsnamen eintippen, wenn gefragt
```

1. **`--force`** belegt die Absicht in dem Moment, in dem Sie den
   Befehl eingetippt haben.
2. **Eine eingetippte Bestätigung auf einem interaktiven Terminal**
   belegt, dass ein Mensch anwesend ist.

Die Terminal-Anforderung ist der Sinn der zweiten Bedingung. Ohne
sie würde `echo production | suprnova migrate:fresh --force` in
einem Deploy-Skript die Eingabeaufforderung automatisch beantworten,
und die Bestätigung wäre nur ein weiteres Flag. Deshalb wird ein
nicht-interaktives stdin auch mit `--force` abgelehnt.

Alles außer dem exakten Umgebungsnamen bricht ab, bevor eine einzige
Tabelle gelöscht wird.

Dasselbe Gate gilt für das eigene Subkommando Ihrer
Anwendungs-Binary (`./app migrate:fresh --force`), das ist
dasjenige, das ein Produktions-Deploy tatsächlich ausführt.

## db:sync

Regeneriert die SeaORM-Entity-Dateien in `src/models/entities/` aus
dem aktuellen Datenbankschema und führt (falls eine
`src/bin/migrate.rs` existiert) zuerst ausstehende Migrationen aus.

```bash
suprnova db:sync [--skip-migrations] [--regenerate-models]
```

| Option | Beschreibung |
|---|---|
| `--skip-migrations` | Überspringt den Migrationsdurchlauf und regeneriert nur Entities |
| `--regenerate-models` | Überschreibt auch `src/models/<table>.rs`-Dateien, nicht nur `src/models/entities/<table>.rs` |

### Was es tut

1. (Optional) Führt ausstehende Migrationen aus. Das
   Standard-Scaffold liefert kein `src/bin/migrate.rs` mit, also ist
   dieser Schritt ein No-op und gibt
   `Migration binary not found, skipping migrations` aus. In einem
   Standardprojekt führen Sie zuerst `suprnova migrate` aus, dann
   `suprnova db:sync --skip-migrations`.
2. Verbindet sich mit `DATABASE_URL`, introspiziert jede
   Nutzer-Tabelle (überspringt `seaql_migrations` und jeden Namen,
   der mit `_` beginnt), und schreibt eine Entity-Datei pro Tabelle
   nach `src/models/entities/<table>.rs`.
3. Schreibt eine dünne, nutzerseitige Model-Datei nach
   `src/models/<table>.rs` - aber nur, falls diese Datei noch nicht
   existiert, sodass Ihre handgeschriebenen Accessoren, Scopes und
   Observer-Hooks überleben.
4. `--regenerate-models` setzt den Schutz aus Schritt 3 außer Kraft
   und überschreibt diese Nutzerdateien. Verwenden Sie es, wenn Sie
   sie noch nicht angepasst haben, oder wenn Sie ein Backup haben.

### Typischer Workflow

```bash
# 1. Eine Migration verfassen
suprnova make:migration create_posts_table
# (src/migrations/m..._create_posts_table.rs bearbeiten)

# 2. Anwenden
suprnova migrate

# 3. Die Entities regenerieren, damit die neue Tabelle aus Code erreichbar ist
suprnova db:sync --skip-migrations
```

### Warum Suprnova abweicht

Laravel hat ein einziges globales `artisan`, dem jeder
Framework-Befehl gehört, einschließlich `db:seed`. Suprnova teilt
das in zwei:

- Die `suprnova`-Entwickler-CLI (dieses Kapitel) besitzt
  Projekt-Scaffolding, Generatoren und die Migrations-Befehle. Sie
  wird einmal pro Entwickler-Maschine über `cargo install`
  installiert und shellt in Ihre App-Binary, um Arbeit zu erledigen,
  die den `Migrator` der App braucht.
- Eine projektspezifische `console`-Binary, gebaut aus dem
  `src/bin/console.rs` Ihres Projekts, besitzt `db:seed`, Ihre mit
  `#[command]` annotierten Handler, `queue:work`, `schedule:run`,
  `workflow:work` und weitere einmalige Tasks, die den Bootstrap
  Ihrer App, Container-Bindings und registrierte Observer brauchen.

Migrations-Befehle leben auf der Entwickler-CLI, weil sie eine
deterministische Form haben, die nicht von Ihrem Bootstrap abhängt.
Alles, was Ihren Service Container oder Ihre registrierten Seeder
braucht, lebt auf der projektspezifischen console-Binary. Siehe
[Konsole](console.md) für die vollständige Console-Oberfläche.

## db:seed

Kein `suprnova`-CLI-Befehl. Führen Sie Seeder über die
projektspezifische console-Binary aus:

```bash
cargo run --bin console -- db:seed
cargo run --bin console -- db:seed --class=UsersSeeder
```

Die Seeder-Registry, die Reihenfolge-Regeln und das
`--class`-Matching sind in [Seeding](seeding.md) behandelt. Das
Framework liefert `db:seed` als eingebauten Console-Befehl aus - Ihr
Scaffold bekommt ihn ohne jede Verdrahtung auf Ihrer Seite, aber Sie
rufen ihn über `console` auf, nicht über `suprnova`.

## Zusammenfassung

| Befehl | Was er tut |
|---|---|
| `suprnova make:migration <name>` | Scaffoldet eine neue Migrationsdatei und registriert sie im `Migrator` |
| `suprnova migrate` | Führt ausstehende Migrationen aus |
| `suprnova migrate:status` | Zeigt den Status angewendet/ausstehend |
| `suprnova migrate:rollback [--step N]` | Rollt die letzten `N` Migrationen zurück (Standard 1) |
| `suprnova migrate:fresh` | Löscht alle Tabellen und führt jede Migration erneut aus |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | Regeneriert SeaORM-Entities aus dem lebenden Schema |
| `cargo run --bin console -- db:seed` | Führt registrierte Seeder aus (projektspezifische console, nicht die `suprnova`-CLI) |

## Nächste Schritte

- [Migrationen](migrations.md) - Schema-Builder-API: Tabellen,
  Spalten, Indizes, Fremdschlüssel
- [Seeding](seeding.md) - Seeder verfassen und der
  `db:seed`-Console-Befehl
- [Konsole](console.md) - die projektspezifische `console`-Binary
  und `#[command]`-Handler
- [Datenbank](database.md) - Connections, Treiber, Transactions,
  der Query Builder
- [CLI - Übersicht](cli.md) - jedes `suprnova`-Subkommando im
  Überblick

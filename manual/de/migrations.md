# Migrationen

Migrationen beschreiben, wie sich Ihr Schema entwickelt - jede Datei
ist eine kleine Rust-Struktur mit den Methoden `up()` und `down()`,
die das Framework in Zeitstempel-Reihenfolge ausführt. Verwenden Sie
sie, wann immer Sie Tabellen, Spalten, Indizes oder Fremdschlüssel
ändern; diese Änderung wandert vom eigenen Laptop über Staging bis in
die Produktion, indem an jedem Ort derselbe migrate-Befehl ausgeführt
wird.

Suprnovas Migrationen sind darunter SeaORM-Migrationen. Die CLI
generiert sie, der `Migrator` sammelt sie, und
`Application::migrations::<Migrator>()` klinkt sie in den Boot Ihrer
App ein. Für die vollständige Referenz pro Befehl (Flags,
Ausgabebeispiele, Exit-Codes) siehe
[CLI-Migrationen-Referenz](cli-migrations.md); dieses Kapitel deckt
ab, was *in* die Dateien gehört.

## Migrationen erstellen

Generieren Sie eine neue Migrationsdatei:

```bash
suprnova make:migration create_users_table
```

Der Generator schreibt eine zeitstempelte Datei unter
`src/migrations/` (und legt das Verzeichnis beim ersten Mal an) und
registriert sie im `Migrator`:

```
src/migrations/
├── mod.rs                              ← der Migrator (CLI-verwaltet)
└── m20240115_120000_create_users_table.rs
```

Der Dateiname ist `m{YYYYMMDD}_{HHMMSS}_<name>.rs`; die Reihenfolge
richtet sich nach dem Dateinamen, sodass das Zeitstempel-Präfix die
deterministische Anwendungsreihenfolge erzwingt.

### Was der Generator ausgibt

`make:migration create_users_table` erzeugt dieses Skelett:

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

Der Generator leitet den Tabellennamen aus dem Migrationsnamen ab
(`create_X_table` → `X`, `add_Y_to_X` → `X`, `drop_X_table` → `X`).
Alles andere wird zum wörtlichen Namen.

### Der Migrator

`src/migrations/mod.rs` sammelt jede Migration in einem einzigen
`Migrator`, den `MigratorTrait` durchläuft. Die CLI pflegt diese
Datei, wenn Sie `make:migration` ausführen, sodass Sie sie selten von
Hand berühren:

```rust
pub use sea_orm_migration::prelude::*;

mod m20240115_120000_create_users_table;
mod m20240115_130000_create_posts_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240115_120000_create_users_table::Migration),
            Box::new(m20240115_130000_create_posts_table::Migration),
        ]
    }
}
```

Verdrahten Sie den Migrator in die `main.rs` Ihrer App, sodass
`serve`, `migrate`, `migrate:status`, `migrate:rollback` und
`migrate:fresh` alle dieselbe Liste sehen:

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)
        .bootstrap(my_app::bootstrap::bootstrap)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

Der Scaffolder schreibt das bei `suprnova new` für Sie.

### Warum Suprnova abweicht

Der größte Teil des Frameworks verbirgt SeaORM absichtlich - Sie
schreiben `#[suprnova::model]` und `User::query().db_where(...)`,
nicht `Entity::find().filter(...)`. Migrationen sind der eine Ort, an
dem wir `sea_orm_migration::prelude::*` sichtbar lassen. Zwei Gründe.

Erstens ist die Schema-Builder-DSL wirklich gut, und jeden Namen
darin (`Table`, `ColumnDef`, `Index`, `ForeignKey`, `Expr`,
`ForeignKeyAction`, `DeriveIden`, …) neu zu aliasen, würde nur eine
längere Import-Zeile einbringen und sonst nichts. Zweitens sind
Migrationsdateien reines Rust - Ihr CI-Compiler verifiziert sie -,
und das fängt mehr Tippfehler ab, als jedes Neu-Aliasing einer DSL
könnte. Wir behandeln Migrationen wie Schema-as-Code, und die
kanonischen SeaORM-Namen *sind* das Schema-Vokabular.

Falls Sie doch einmal einen SeaORM-Typ brauchen, den das Framework
nicht re-exportiert hat: Der Ausweichhaken ist `use
suprnova::sea_orm;`. Sie brauchen ihn fast nie.

## Migrationsstruktur

Jede Migration hat zwei Methoden:

```rust
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // Wendet die Änderung an
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }

    // Macht die Änderung rückgängig
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }
}
```

Beide Zweige liefern `Result<(), DbErr>` - lassen Sie Fehler mit `?`
durchreichen, und das Framework verwandelt eine fehlgeschlagene
Migration in einen Non-Zero-Exit, sodass Deploy-Pipelines abbrechen.

## Schema-Operationen

### Tabellen erstellen

```rust
use sea_orm_migration::prelude::*;

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
                .col(ColumnDef::new(Users::Email).string().not_null().unique_key())
                .col(ColumnDef::new(Users::Name).string().not_null())
                .col(ColumnDef::new(Users::PasswordHash).string().not_null())
                .col(ColumnDef::new(Users::CreatedAt).timestamp().not_null())
                .col(ColumnDef::new(Users::UpdatedAt).timestamp().not_null())
                .to_owned(),
        )
        .await
}

// Definiert die Tabellen- und Spalten-Identifier
#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    Name,
    PasswordHash,
    CreatedAt,
    UpdatedAt,
}
```

### Tabellen löschen

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_table(Table::drop().table(Users::Table).to_owned())
        .await
}
```

### Spaltentypen

| Methode | Datenbanktyp | Anmerkungen |
|--------|---------------|-------|
| `integer()` | INTEGER | 32-Bit-Ganzzahl |
| `big_integer()` | BIGINT | 64-Bit-Ganzzahl |
| `small_integer()` | SMALLINT | 16-Bit-Ganzzahl |
| `float()` | FLOAT | Gleitkommazahl |
| `double()` | DOUBLE | Doppelte Genauigkeit |
| `decimal()` | DECIMAL | Festkommazahl |
| `string()` | VARCHAR(255) | String variabler Länge |
| `string_len(n)` | VARCHAR(n) | String mit eigener Länge |
| `text()` | TEXT | Langer Text |
| `boolean()` | BOOLEAN | Wahr/falsch |
| `timestamp()` | TIMESTAMP | Datum und Uhrzeit |
| `date()` | DATE | Nur Datum |
| `time()` | TIME | Nur Uhrzeit |
| `blob()` | BLOB | Binärdaten |
| `json()` | JSON | JSON-Daten |
| `uuid()` | UUID | UUID-Typ |

### Spalten-Modifikatoren

```rust
ColumnDef::new(Column::Name)
    .string()
    .not_null()                                // NOT-NULL-Constraint
    .null()                                    // Erlaubt NULL (Standard)
    .default("value")                          // Standardwert
    .default(Expr::current_timestamp())        // Funktions-Standard (z. B. NOW())
    .unique_key()                              // UNIQUE-Constraint
    .primary_key()                             // PRIMARY KEY
    .auto_increment()                          // AUTO_INCREMENT
```

Für Surrogat-Primärschlüssel bevorzugen Sie auf echten Tabellen
`big_integer().auto_increment().primary_key()` - `INTEGER` (32-Bit)
ist für winzige Lookup-Tabellen in Ordnung, aber die gescaffoldeten
Tabellen `users`, `sessions` und ähnliche verwenden alle `BIGINT`,
weil ein 4-Byte-Zähler die Art von Einschränkung ist, die man drei
Jahre später bereut.

## Spalten hinzufügen

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .add_column(
                    ColumnDef::new(Users::PhoneNumber)
                        .string()
                        .null()
                )
                .to_owned(),
        )
        .await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .drop_column(Users::PhoneNumber)
                .to_owned(),
        )
        .await
}
```

## Spalten ändern

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .modify_column(
                    ColumnDef::new(Users::Name)
                        .string_len(500)  // Ändert VARCHAR(255) zu VARCHAR(500)
                        .not_null()
                )
                .to_owned(),
        )
        .await
}
```

## Spalten umbenennen

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .rename_column(Users::Name, Users::FullName)
                .to_owned(),
        )
        .await
}
```

## Indizes

### Indizes erstellen

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_index(
            Index::create()
                .name("idx_users_email")
                .table(Users::Table)
                .col(Users::Email)
                .unique()  // Optional: macht ihn eindeutig
                .to_owned(),
        )
        .await
}
```

### Zusammengesetzte Indizes

```rust
manager
    .create_index(
        Index::create()
            .name("idx_posts_user_created")
            .table(Posts::Table)
            .col(Posts::UserId)
            .col(Posts::CreatedAt)
            .to_owned(),
    )
    .await
```

### Indizes löschen

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_index(Index::drop().name("idx_users_email").to_owned())
        .await
}
```

## Fremdschlüssel

### Fremdschlüssel hinzufügen

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Posts::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Posts::Id)
                        .integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Posts::UserId).integer().not_null())
                .col(ColumnDef::new(Posts::Title).string().not_null())
                .col(ColumnDef::new(Posts::Content).text().not_null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_posts_user")
                        .from(Posts::Table, Posts::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade)
                        .on_update(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
}
```

### Fremdschlüssel-Aktionen

| Aktion | Beschreibung |
|--------|-------------|
| `Cascade` | Löscht/aktualisiert Kind-Zeilen automatisch |
| `SetNull` | Setzt den Fremdschlüssel auf NULL |
| `SetDefault` | Setzt den Fremdschlüssel auf den Standardwert |
| `Restrict` | Verhindert Löschen/Aktualisieren, solange referenziert |
| `NoAction` | Ähnlich wie Restrict |

## Migrations-Workflow

Eine typische Änderung durchläuft vier Schritte:

```bash
# 1. Die Datei generieren (erzeugt
#    src/migrations/m{ts}_create_posts_table.rs und aktualisiert
#    src/migrations/mod.rs).
suprnova make:migration create_posts_table

# 2. src/migrations/m{ts}_create_posts_table.rs bearbeiten, um Ihr
#    Schema zu definieren.

# 3. Die Migration anwenden.
suprnova migrate

# 4. Die SeaORM-Entity-Dateien aus dem lebenden Schema neu
#    generieren, damit die Models gegen die neue Form kompilieren.
#    `db:sync` führt dabei zuerst auch jede ausstehende Migration aus
#    (mit --skip-migrations überspringen Sie diesen Schritt).
suprnova db:sync
```

`db:sync` schreibt automatisch generierten Entity-Klebstoff nach
`src/models/entities/<table>.rs` und einen von Nutzern editierbaren
Stub nach `src/models/<table>.rs`. Erneutes Ausführen aktualisiert
die Entity-Dateien; Ihre Nutzer-Stubs bleiben unangetastet, sofern
Sie nicht `--regenerate-models` übergeben (das überschreibt sie -
bewahren Sie eigene Methoden anderswo auf oder versionieren Sie sie,
bevor Sie es ausführen).

### Auto-Migrate bei serve

`suprnova serve` und `suprnova web:run` wenden jede ausstehende
Migration an, bevor sie den HTTP-Socket öffnen. Die Standard-Richtlinie
ist **fail-closed**: Schlägt `up()` fehl, bricht der Prozess mit
Non-Zero ab, bevor er bindet, sodass eine defekte Migration nie
Traffic erreichen kann.

Zwei Ausweichhaken:

| Flag / Env | Wirkung |
|---|---|
| `--no-migrate` (bei `serve` / `web:run`) | Überspringt den Auto-Migrate-Schritt vollständig. Nützlich, wenn Migrationen aus einem separaten Deploy-Schritt laufen. |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` | Steigt zurück auf das alte Log-und-weitermachen-Verhalten ein. Der Prozess bootet bei einem Migrationsfehler trotzdem weiter. In Produktion nicht empfohlen. |

Hintergrund-Worker (`queue:work`, `workflow:work`, `schedule:run`)
auto-migrieren *nicht* - sie nehmen an, dass das Schema beim Booten
bereits vorhanden ist, da das gleichzeitige Ausführen von Migrationen
aus N Workern eine Race Condition wäre.

### Migrationen in Tests ausführen

`TestDatabase::fresh::<Migrator>()` fährt eine isolierte
In-Memory-SQLite-Datenbank hoch, führt jede Migration aus und bindet
die Connection in den Test-Container, sodass `DB::connection()` und
`#[inject]` sie auflösen:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn users_table_is_created() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // `db` wird am Ende des Tests gedroppt, was den Container leert.
}
```

Siehe [Datenbank-Tests](database-testing.md) für das vollständige
Muster (Factories, Parallelsicherheit, einen echten Treiber statt
In-Memory-SQLite wählen).

## Best Practices

### Migrationen immer mit `down()` ausstatten

Implementieren Sie `down()` immer, um Rollbacks zu ermöglichen:

```rust
// Gut: Umkehrbare Migration
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.create_table(/* ... */).await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.drop_table(/* ... */).await
}
```

### Aussagekräftige Namen verwenden

```bash
# Gut: Beschreibt die Änderung
suprnova make:migration add_email_verified_to_users
suprnova make:migration create_order_items_table
suprnova make:migration add_index_to_posts_slug

# Schlecht: Vage Namen
suprnova make:migration update_users
suprnova make:migration change_table
```

### Eine Änderung pro Migration

Halten Sie Migrationen auf eine einzelne Änderung fokussiert:

```bash
# Gut: Getrennte Migrationen
suprnova make:migration create_categories_table
suprnova make:migration add_category_id_to_posts

# Vermeiden: Mehrere unzusammenhängende Änderungen in einer Migration
```

### Migrationen in beide Richtungen testen

Verifizieren Sie vor dem Committen, dass beide Richtungen
funktionieren:

```bash
suprnova migrate           # Anwenden
suprnova migrate:rollback  # Zurückrollen
suprnova migrate           # Erneut anwenden
```

## CLI-Befehle im Überblick

| Befehl | Beschreibung |
|---------|-------------|
| `suprnova make:migration <name>` | Erstellt eine neue Migration |
| `suprnova migrate` | Führt jede ausstehende Migration aus |
| `suprnova migrate:status` | Zeigt den Migrationsstatus |
| `suprnova migrate:rollback` | Rollt die letzte Migration zurück |
| `suprnova migrate:rollback --step 3` | Rollt die letzten 3 Migrationen zurück |
| `suprnova migrate:fresh` | Löscht alle Tabellen und führt jede Migration neu aus |
| `suprnova db:sync` | Führt Migrationen aus und regeneriert Entity-Dateien |
| `suprnova db:sync --skip-migrations` | Regeneriert Entity-Dateien, ohne Migrationen anzuwenden |
| `suprnova db:sync --regenerate-models` | Überschreibt auch nutzer-editierbare Model-Stubs |

Siehe [CLI-Migrationen-Referenz](cli-migrations.md) für die
vollständige Referenz pro Befehl (Flags, Ausgabebeispiele,
Exit-Codes).

## Nächste Schritte

- [CLI-Migrationen-Referenz](cli-migrations.md) - Flag-für-Flag-Referenz
  für `migrate*` und `db:sync`
- [Datenbank](database.md) - Connection-Konfiguration, Transaktionen,
  Read/Write-Split
- [Eloquent](eloquent.md) - die Model-Schicht, die Ihre Migrationen
  füttern
- [Seeding](seeding.md) - Tabellen befüllen, sobald ihr Schema
  existiert
- [Datenbank-Tests](database-testing.md) -
  `TestDatabase::fresh::<Migrator>()` und parallelsichere Muster

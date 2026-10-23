# Migrations

Les migrations décrivent comment votre schéma évolue - chaque fichier
est une petite struct Rust avec des méthodes `up()` et `down()` que le
framework exécute dans l'ordre des timestamps. Utilisez-les chaque
fois que vous changez des tables, des colonnes, des index, ou des
clés étrangères ; ce changement passe de votre portable au staging
puis à la production en exécutant la même commande migrate à chaque
endroit.

Les migrations de Suprnova sont des migrations SeaORM en dessous. Le
CLI les génère, le `Migrator` les agrège, et
`Application::migrations::<Migrator>()` les branche dans le boot de
votre app. Pour la référence complète par commande (flags,
échantillons de sortie, codes de sortie) voir
[Référence Migrations CLI](cli-migrations.md) ; ce chapitre couvre ce
qu'il faut mettre *à l'intérieur* des fichiers.

## Créer des migrations

Générez un nouveau fichier de migration :

```bash
suprnova make:migration create_users_table
```

Le générateur écrit un fichier horodaté sous `src/migrations/` (en
créant le répertoire la première fois) et l'enregistre dans le
`Migrator` :

```
src/migrations/
├── mod.rs                              ← le Migrator (géré par le CLI)
└── m20240115_120000_create_users_table.rs
```

Le nom de fichier est `m{YYYYMMDD}_{HHMMSS}_<name>.rs` ; l'ordre est
déterminé par le nom de fichier, donc c'est le préfixe timestamp qui
impose un ordre d'application déterministe.

### Ce que le générateur émet

`make:migration create_users_table` produit ce squelette :

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

Le générateur déduit le nom de la table depuis le nom de la migration
(`create_X_table` → `X`, `add_Y_to_X` → `X`, `drop_X_table` → `X`).
Tout le reste devient le nom littéral.

### Le Migrator

`src/migrations/mod.rs` regroupe chaque migration dans un seul
`Migrator` que `MigratorTrait` parcourt. Le CLI maintient ce fichier
quand vous faites `make:migration`, donc vous y touchez rarement à la
main :

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

Câblez le migrateur dans le `main.rs` de votre app pour que `serve`,
`migrate`, `migrate:status`, `migrate:rollback`, et `migrate:fresh`
voient tous la même liste :

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

Le scaffolder écrit ceci pour vous lors d'un `suprnova new`.

### Pourquoi Suprnova diverge

La plus grande partie du framework masque délibérément SeaORM - vous
écrivez `#[suprnova::model]` et `User::query().db_where(...)`, pas
`Entity::find().filter(...)`. Les migrations sont le seul endroit où
nous laissons `sea_orm_migration::prelude::*` visible. Deux raisons.

Premièrement, le DSL du schema-builder est vraiment bon, et réaliaser
chaque nom qu'il contient (`Table`, `ColumnDef`, `Index`,
`ForeignKey`, `Expr`, `ForeignKeyAction`, `DeriveIden`, ...)
achèterait une ligne d'import plus longue et rien d'autre.
Deuxièmement, les fichiers de migration sont du Rust pur - le
compilateur de votre CI les vérifie - et cela attrape plus de fautes
de frappe qu'un réaliasage de DSL ne le ferait jamais. Nous traitons
les migrations comme du schéma-en-code, et les noms canoniques de
SeaORM *sont* le vocabulaire du schéma.

Si jamais vous avez besoin d'un type SeaORM que le framework n'a pas
réexporté, l'échappatoire est `use suprnova::sea_orm;`. Vous n'en
avez presque jamais besoin.

## Structure de migration

Chaque migration a deux méthodes :

```rust
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // Applique le changement
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }

    // Inverse le changement
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }
}
```

Les deux branches retournent `Result<(), DbErr>` - faites remonter
les erreurs avec `?`, et le framework transforme une migration en
échec en un code de sortie non nul pour que les pipelines de
déploiement s'interrompent.

## Opérations de schéma

### Créer des tables

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

// Définit les identifiants de la table et des colonnes
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

### Supprimer des tables

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_table(Table::drop().table(Users::Table).to_owned())
        .await
}
```

### Types de colonne

| Méthode | Type de base de données | Remarques |
|--------|---------------|-------|
| `integer()` | INTEGER | Entier 32 bits |
| `big_integer()` | BIGINT | Entier 64 bits |
| `small_integer()` | SMALLINT | Entier 16 bits |
| `float()` | FLOAT | Virgule flottante |
| `double()` | DOUBLE | Double précision |
| `decimal()` | DECIMAL | Virgule fixe |
| `string()` | VARCHAR(255) | Chaîne de longueur variable |
| `string_len(n)` | VARCHAR(n) | Chaîne de longueur personnalisée |
| `text()` | TEXT | Texte long |
| `boolean()` | BOOLEAN | Vrai/faux |
| `timestamp()` | TIMESTAMP | Date et heure |
| `date()` | DATE | Date seule |
| `time()` | TIME | Heure seule |
| `blob()` | BLOB | Données binaires |
| `json()` | JSON | Données JSON |
| `uuid()` | UUID | Type UUID |

### Modificateurs de colonne

```rust
ColumnDef::new(Column::Name)
    .string()
    .not_null()                                // contrainte NOT NULL
    .null()                                    // autorise NULL (défaut)
    .default("value")                          // valeur par défaut
    .default(Expr::current_timestamp())        // défaut par fonction (ex. NOW())
    .unique_key()                              // contrainte UNIQUE
    .primary_key()                             // PRIMARY KEY
    .auto_increment()                          // AUTO_INCREMENT
```

Pour les clés primaires de substitution, préférez
`big_integer().auto_increment().primary_key()` sur les vraies tables -
`INTEGER` (32 bits) convient pour les petites tables de référence,
mais les tables scaffoldées `users`, `sessions`, et similaires
utilisent toutes `BIGINT`, parce qu'un compteur de 4 octets est le
genre de contrainte que vous regretterez trois ans plus tard.

## Ajouter des colonnes

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

## Modifier des colonnes

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .modify_column(
                    ColumnDef::new(Users::Name)
                        .string_len(500)  // Change VARCHAR(255) en VARCHAR(500)
                        .not_null()
                )
                .to_owned(),
        )
        .await
}
```

## Renommer des colonnes

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

## Index

### Créer des index

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_index(
            Index::create()
                .name("idx_users_email")
                .table(Users::Table)
                .col(Users::Email)
                .unique()  // Facultatif : la rendre unique
                .to_owned(),
        )
        .await
}
```

### Index composites

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

### Supprimer des index

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_index(Index::drop().name("idx_users_email").to_owned())
        .await
}
```

## Clés étrangères

### Ajouter des clés étrangères

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

### Actions de clé étrangère

| Action | Description |
|--------|-------------|
| `Cascade` | Supprime/met à jour automatiquement les lignes enfants |
| `SetNull` | Met la clé étrangère à NULL |
| `SetDefault` | Met la clé étrangère à sa valeur par défaut |
| `Restrict` | Empêche la suppression/mise à jour si référencée |
| `NoAction` | Similaire à Restrict |

## Flux de travail des migrations

Un changement typique passe par quatre étapes :

```bash
# 1. Générer le fichier (crée src/migrations/m{ts}_create_posts_table.rs
#    et met à jour src/migrations/mod.rs).
suprnova make:migration create_posts_table

# 2. Éditer src/migrations/m{ts}_create_posts_table.rs pour définir
#    votre schéma.

# 3. Appliquer la migration.
suprnova migrate

# 4. Régénérer les fichiers d'entité SeaORM depuis le schéma en
#    vigueur pour que les modèles compilent contre la nouvelle forme.
#    `db:sync` exécute aussi d'abord toute migration en attente
#    (utilisez --skip-migrations pour sauter cette étape).
suprnova db:sync
```

`db:sync` écrit la glue d'entité auto-générée dans
`src/models/entities/<table>.rs` et un stub éditable par l'utilisateur
dans `src/models/<table>.rs`. Le réexécuter met à jour les fichiers
d'entité ; vos stubs utilisateur sont laissés tranquilles à moins que
vous ne passiez `--regenerate-models` (qui les écrase - gardez vos
méthodes personnalisées ailleurs, ou faites un commit avant de
l'exécuter).

### Migration automatique au lancement de serve

`suprnova serve` et `suprnova web:run` appliquent toute migration en
attente avant d'ouvrir le socket HTTP. La policy par défaut
**échoue fermée** : si `up()` échoue, le processus s'interrompt avec
un code non nul avant le bind, si bien qu'une migration cassée ne
peut jamais atteindre le trafic.

Deux échappatoires :

| Flag / env | Effet |
|---|---|
| `--no-migrate` (sur `serve` / `web:run`) | Ignore entièrement l'étape d'auto-migration. Utile quand les migrations s'exécutent depuis une étape de déploiement séparée. |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` | Revenir au comportement historique de journaliser-et-continuer. Le processus continue de démarrer même en cas d'erreur de migration. Non recommandé en production. |

Les workers en arrière-plan (`queue:work`, `workflow:work`,
`schedule:run`) ne font *pas* d'auto-migration - ils supposent que le
schéma est déjà en place quand ils démarrent, car exécuter des
migrations depuis N workers en concurrence entrerait en course.

### Exécuter les migrations dans les tests

`TestDatabase::fresh::<Migrator>()` démarre une base de données
SQLite en mémoire isolée, exécute chaque migration, et lie la
connexion dans le conteneur de test pour que `DB::connection()` et
`#[inject]` s'y résolvent :

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn users_table_is_created() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // `db` est droppée à la fin du test, ce qui vide le conteneur.
}
```

Voir [Tests de base de données](database-testing.md) pour le motif
complet (fabriques, sûreté en parallèle, choisir un vrai driver au
lieu de SQLite en mémoire).

## Bonnes pratiques

### Toujours implémenter `down()`

Implémentez toujours `down()` pour permettre les rollbacks :

```rust
// Bien : migration réversible
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.create_table(/* ... */).await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.drop_table(/* ... */).await
}
```

### Utiliser des noms descriptifs

```bash
# Bien : décrit le changement
suprnova make:migration add_email_verified_to_users
suprnova make:migration create_order_items_table
suprnova make:migration add_index_to_posts_slug

# Mauvais : noms vagues
suprnova make:migration update_users
suprnova make:migration change_table
```

### Un changement par migration

Gardez les migrations focalisées sur un seul changement :

```bash
# Bien : migrations séparées
suprnova make:migration create_categories_table
suprnova make:migration add_category_id_to_posts

# À éviter : plusieurs changements sans rapport dans une seule migration
```

### Tester les migrations dans les deux sens

Avant de commiter, vérifiez que les deux directions fonctionnent :

```bash
suprnova migrate           # Applique
suprnova migrate:rollback  # Rollback
suprnova migrate           # Applique à nouveau
```

## Commandes CLI en un coup d'œil

| Commande | Description |
|---------|-------------|
| `suprnova make:migration <name>` | Crée une nouvelle migration |
| `suprnova migrate` | Exécute toutes les migrations en attente |
| `suprnova migrate:status` | Affiche le statut des migrations |
| `suprnova migrate:rollback` | Annule la dernière migration |
| `suprnova migrate:rollback --step 3` | Annule les 3 dernières migrations |
| `suprnova migrate:fresh` | Supprime toutes les tables et relance chaque migration |
| `suprnova db:sync` | Exécute les migrations et régénère les fichiers d'entité |
| `suprnova db:sync --skip-migrations` | Régénère les fichiers d'entité sans appliquer les migrations |
| `suprnova db:sync --regenerate-models` | Écrase aussi les stubs de modèle éditables par l'utilisateur |

Voir [Référence Migrations CLI](cli-migrations.md) pour la référence
complète par commande (flags, échantillons de sortie, codes de
sortie).

## Suivant

- [Référence Migrations CLI](cli-migrations.md) - référence flag par
  flag pour `migrate*` et `db:sync`
- [Base de données](database.md) - configuration de connexion,
  transactions, séparation lecture/écriture
- [Eloquent](eloquent.md) - la couche modèle que vos migrations
  alimentent
- [Ensemencement](seeding.md) - peupler les tables une fois leur
  schéma en place
- [Tests de base de données](database-testing.md) -
  `TestDatabase::fresh::<Migrator>()` et les motifs sûrs en parallèle

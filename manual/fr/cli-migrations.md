# Migrations CLI

Le CLI développeur `suprnova` shelle vers le binaire de votre
application pour piloter le gestionnaire de migrations de SeaORM, si
bien que le même jeu de migrations s'exécute que vous le lanciez
depuis un terminal développeur, depuis la CI, ou implicitement au
démarrage du serveur. Utilisez ces commandes pour écrire des fichiers
de migration, les appliquer, les annuler, et garder vos entités
SeaORM générées synchronisées avec le schéma.

Pour l'API d'écriture de schéma (types de colonne, index, clés
étrangères, le `MigrationTrait` complet), voir
[Migrations](migrations.md). Pour insérer des données de test une
fois le schéma en place, voir [Ensemencement](seeding.md).

## make:migration

Génère un nouveau fichier de migration sous `src/migrations/` et le
câble dans le `Migrator` de `src/migrations/mod.rs`.

```bash
suprnova make:migration <name>
```

`<name>` est normalisé en snake_case. Le générateur reconnaît les
motifs de nommage standards et les utilise pour choisir l'enum
`DeriveIden` :

- `create_<table>_table` - scaffolde un corps `create_table`
- `add_<column>_to_<table>` - scaffolde un stub pour `alter_table`
- `drop_<table>_table` - scaffolde un corps `drop_table`
- n'importe quoi d'autre - utilise le nom comme identifiant de table

### Exemples

```bash
suprnova make:migration create_users_table
suprnova make:migration add_email_to_users
suprnova make:migration drop_legacy_sessions_table
```

### Fichier généré

Le fichier est écrit dans
`src/migrations/m{YYYYMMDD}_{HHMMSS}_<name>.rs` (par exemple
`m20260530_142301_create_users_table.rs`) et ajouté au vec
`Migrator::migrations()`.

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

Éditez le fichier généré pour déclarer vos colonnes, index, et
contraintes. Voir [Migrations](migrations.md) pour la surface
complète du schema-builder.

## migrate

Exécute chaque migration en attente dans `src/migrations/`.

```bash
suprnova migrate
```

Le CLI shelle vers `cargo run -- migrate` pour que l'`Application` de
votre application fasse le travail - même binaire, même `Migrator`,
même connexion de base de données que celle que `serve` utiliserait.

```
Running migrations...
Migrations completed successfully!
```

Le chemin serve / web:run exécute automatiquement `migrate` avant de
lier le socket, sauf si vous optez explicitement pour ne pas le faire
avec `--no-migrate` ou définissez
`SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` pour continuer malgré un
échec. Une erreur de migration pendant l'auto-migration quitte avec
un code non nul avant que le serveur ne démarre ; voir
`framework/src/app/mod.rs` pour le contrat d'échec fermé.

## migrate:status

Affiche l'état appliqué/en attente de chaque migration.

```bash
suprnova migrate:status
```

```
Migration status:
...SeaORM-formatted table of applied/pending migrations...
```

Le corps du rapport vient de `MigratorTrait::status` de SeaORM, donc
le format exact suit la version de SeaORM dont dépend votre
application.

## migrate:rollback

Annule la dernière migration appliquée (ou les `N` dernières).

```bash
suprnova migrate:rollback [--step <N>]
```

| Option | Par défaut | Description |
|---|---|---|
| `--step <N>` | `1` | Nombre de migrations à annuler |

```bash
# Annuler une migration
suprnova migrate:rollback

# Annuler les trois dernières
suprnova migrate:rollback --step 3
```

```
Rolling back 3 migration(s)...
Rollback completed successfully!
```

Le `down()` de chaque migration s'exécute dans l'ordre inverse
d'application. Un `down()` en échec quitte avec un code non nul et
laisse le reste de la chaîne intact - rien de plus n'est tenté.

## migrate:fresh

Supprime chaque table de la base de données et relance chaque
migration depuis zéro.

```bash
suprnova migrate:fresh
```

```
WARNING: Dropping all tables and re-running migrations...
Database refreshed successfully!
```

Cela détruit toutes les données de la base de données connectée.
C'est prévu pour le développement local et la configuration de test,
pas pour un environnement où les données comptent.

### Le garde-fou de production

Hors production, elle s'exécute immédiatement, sans invite -
supprimer une base de données locale est routinier, et une
confirmation à laquelle vous répondez toujours de la même façon vous
entraîne à arrêter de la lire.

Quand `APP_ENV` se résout en production, elle exige deux sortes
différentes de preuve :

```bash
suprnova migrate:fresh --force   # …puis tapez le nom de l'environnement quand demandé
```

1. **`--force`** prouve l'intention au moment où vous avez tapé la
   commande.
2. **Une confirmation saisie sur un terminal interactif** prouve
   qu'un humain est présent.

L'exigence de terminal est tout l'intérêt du second point. Sans elle,
`echo production | suprnova migrate:fresh --force` dans un script de
déploiement répondrait automatiquement à l'invite, et la confirmation
ne serait qu'un flag de plus. Donc un stdin non interactif est refusé
même avec `--force`.

Tout ce qui n'est pas le nom exact de l'environnement abandonne avant
qu'une seule table ne soit supprimée.

Le même garde-fou s'applique à la propre sous-commande de votre
binaire d'application (`./app migrate:fresh --force`), qui est celle
qu'un déploiement en production exécute réellement.

## db:sync

Régénère les fichiers d'entité SeaORM dans `src/models/entities/`
depuis le schéma actuel de la base de données, et (quand un
`src/bin/migrate.rs` existe) exécute d'abord les migrations en
attente.

```bash
suprnova db:sync [--skip-migrations] [--regenerate-models]
```

| Option | Description |
|---|---|
| `--skip-migrations` | Ignore la passe de migration et régénère seulement les entités |
| `--regenerate-models` | Écrase aussi les fichiers `src/models/<table>.rs`, pas seulement `src/models/entities/<table>.rs` |

### Ce qu'elle fait

1. (Optionnel) Exécute les migrations en attente. Le scaffold par
   défaut ne livre pas de `src/bin/migrate.rs`, donc cette étape est
   sans effet et affiche `Migration binary not found, skipping
   migrations`. Dans un projet par défaut, exécutez d'abord `suprnova
   migrate`, puis `suprnova db:sync --skip-migrations`.
2. Se connecte à `DATABASE_URL`, introspecte chaque table
   utilisateur (en ignorant `seaql_migrations` et tout nom commençant
   par `_`), et écrit un fichier d'entité par table dans
   `src/models/entities/<table>.rs`.
3. Écrit un fichier de modèle fin destiné à l'utilisateur à
   `src/models/<table>.rs` - mais seulement si ce fichier n'existe
   pas déjà, pour que vos accesseurs, scopes, et hooks d'observateur
   écrits à la main survivent.
4. `--regenerate-models` outrepasse la protection de l'étape 3 et
   écrase ces fichiers utilisateur. Utilisez-le quand vous ne les
   avez pas encore personnalisés, ou quand vous avez une sauvegarde.

### Flux de travail typique

```bash
# 1. Écrire une migration
suprnova make:migration create_posts_table
# (éditer src/migrations/m..._create_posts_table.rs)

# 2. L'appliquer
suprnova migrate

# 3. Régénérer les entités pour que la nouvelle table soit accessible depuis le code
suprnova db:sync --skip-migrations
```

### Pourquoi Suprnova diverge

Laravel a un `artisan` global unique qui possède chaque commande du
framework, y compris `db:seed`. Suprnova scinde cela en deux :

- Le CLI développeur `suprnova` (ce chapitre) possède le scaffolding
  de projet, les générateurs, et les commandes de migration. Il est
  installé une fois par machine de développeur via `cargo install` et
  shelle vers le binaire de votre app pour faire le travail qui a
  besoin du `Migrator` de l'app.
- Un binaire `console` par projet, construit depuis le
  `src/bin/console.rs` de votre projet, possède `db:seed`, vos
  handlers annotés `#[command]`, `queue:work`, `schedule:run`,
  `workflow:work`, et d'autres tâches ponctuelles qui ont besoin du
  bootstrap, des liaisons de conteneur, et des observateurs
  enregistrés de votre app.

Les commandes de migration vivent sur le CLI développeur parce
qu'elles ont une forme déterministe qui ne dépend pas de votre
bootstrap. Tout ce qui a besoin de votre conteneur de service ou de
vos seeders enregistrés vit sur le binaire console par projet. Voir
[Console](console.md) pour la surface complète de la console.

## db:seed

Pas une commande CLI `suprnova`. Exécutez les seeders via le binaire
console par projet :

```bash
cargo run --bin console -- db:seed
cargo run --bin console -- db:seed --class=UsersSeeder
```

Le registre de seeders, les règles d'ordre, et la correspondance
`--class` sont couverts dans [Ensemencement](seeding.md). Le
framework livre `db:seed` comme commande console intégrée - votre
scaffold l'obtient sans aucun câblage de votre part, mais vous
l'invoquez via `console`, pas via `suprnova`.

## Résumé

| Commande | Ce qu'elle fait |
|---|---|
| `suprnova make:migration <name>` | Scaffolde un nouveau fichier de migration et l'enregistre dans `Migrator` |
| `suprnova migrate` | Exécute les migrations en attente |
| `suprnova migrate:status` | Affiche le statut appliqué/en attente |
| `suprnova migrate:rollback [--step N]` | Annule les `N` dernières migrations (1 par défaut) |
| `suprnova migrate:fresh` | Supprime toutes les tables et relance chaque migration |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | Régénère les entités SeaORM depuis le schéma en vigueur |
| `cargo run --bin console -- db:seed` | Exécute les seeders enregistrés (console par projet, pas le CLI `suprnova`) |

## Suivant

- [Migrations](migrations.md) - API du schema-builder : tables,
  colonnes, index, clés étrangères
- [Ensemencement](seeding.md) - écrire des seeders et la commande
  console `db:seed`
- [Console](console.md) - le binaire `console` par projet et les
  handlers `#[command]`
- [Base de données](database.md) - connexions, drivers, transactions,
  le query builder
- [Présentation CLI](cli.md) - chaque sous-commande `suprnova` en un
  coup d'œil

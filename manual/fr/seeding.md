# Ensemencement

Les seeders peuplent la base de données avec des données de fixture -
les lignes dont votre app a besoin avant même qu'un vrai utilisateur
n'ait fait quoi que ce soit. Pensez à un compte admin par défaut, à la
liste canonique des pays, aux articles de démo sur l'environnement de
staging, aux 50 utilisateurs + 200 articles dont dépend votre boucle
d'itération de dev locale. Ils sont le pendant à l'exécution des
[migrations](migrations.md) : les migrations construisent le schéma
vide, les seeders le remplissent.

Un seeder est un type de taille nulle qui implémente le trait
`Seeder`. Le framework garde un registre ordonné, global au
processus ; la commande `console db:seed` propre au projet exécute
chaque seeder enregistré dans l'ordre d'enregistrement, ou un seeder
spécifique via `--class=<Name>`. La plupart des seeders finissent par
n'être que quelques lignes qui appellent une
[fabrique de modèle](eloquent.md) et laissent la fabrique faire le
travail de génération des lignes.

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

Enregistrez-le une fois au boot :

```rust
// src/bootstrap.rs
suprnova::seed::register::<crate::seeders::UsersSeeder>();
```

Puis :

```bash
cargo run --bin console -- db:seed
# running seeder UsersSeeder
# (50 rows inserted)
```

C'est toute la boucle. Le reste de ce chapitre couvre les conventions
de disposition, les motifs de composition de registre plus larges, le
flag de ciblage `--class`, l'intégration des fabriques, l'échappatoire
`without_events`, et la décision entre ensemencer, migrer, ou utiliser
une fabrique.

## Écrire un seeder

Un seeder est un type unitaire plus un impl `Seeder`. `name()` est la
clé du registre (aussi ce contre quoi `db:seed --class=<Name>` fait
correspondre), et `run()` est la fn async qui effectue les
insertions.

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

`Seeder` est réexporté à la racine de la crate, donc
`use suprnova::Seeder` suffit - vous n'avez pas besoin d'aller
chercher `suprnova::seed::Seeder`. `async_trait` est aussi réexporté
(`use suprnova::async_trait`) parce que la méthode du trait retourne
une future et Rust n'autorise pas encore `async fn` dans les traits
sans lui.

Le type de retour `FrameworkError` est la même enveloppe d'erreur que
toute autre surface async du framework utilise ; faire remonter le
`?` hors d'un appel de fabrique ou d'un `Model::create` est la forme
attendue. Voir [Modèle d'erreur](error-model.md) pour la taxonomie
complète.

### Convention de disposition

Reflétez le répertoire `database/seeders/` de Laravel, mais à la
racine des sources :

```
src/
├── bootstrap.rs
├── factories/
│   ├── mod.rs
│   ├── user_factory.rs
│   └── post_factory.rs
├── seeders/
│   ├── mod.rs              // pub mod base_seeder; pub use base_seeder::BaseSeeder;
│   └── base_seeder.rs      // impl Seeder, enregistré dans bootstrap.rs
└── …
```

Générez le fichier à la main - il n'y a pas de générateur
`make:seeder` (c'est un fichier avec une dizaine de lignes de
boilerplate). Les fabriques que le seeder appelle reçoivent le même
traitement.

### Un seeder qui exécute d'autres seeders

L'idiome Laravel d'un unique `DatabaseSeeder::run` de premier niveau
qui orchestre les ensemencements par modèle fonctionne ici aussi.
Plutôt que d'enregistrer cinq petits seeders dans le bootstrap et de
faire confiance à leur ordre d'enregistrement, enregistrez un seeder
composite et appelez vous-même le reste :

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // 50 utilisateurs d'abord - la fabrique de posts génère
        // author_id dans 1..=50, donc les références se résolvent.
        UserFactory::new().count(50).create_many().await?;

        // 200 posts référençant les ids d'utilisateur ci-dessus.
        PostFactory::new().count(200).create_many().await?;

        Ok(())
    }
}
```

C'est le défaut recommandé. Cela garde l'ordre de dépendance
(`users` avant `posts`) à l'intérieur du seeder plutôt que dispersé à
travers le fichier bootstrap, et `db:seed --class=BaseSeeder` est une
invocation à cible unique qui exécute tout le lot.

Si vous voulez chaîner des seeders par leur nom plutôt que par un
appel de fabrique direct, utilisez `seed::run_one` depuis l'intérieur
du seeder composite :

```rust
async fn run() -> Result<(), FrameworkError> {
    suprnova::seed::run_one("UsersSeeder").await?;
    suprnova::seed::run_one("PostsSeeder").await?;
    suprnova::seed::run_one("CommentsSeeder").await?;
    Ok(())
}
```

Les sous-seeders doivent quand même être enregistrés dans
`bootstrap.rs` pour que `run_one` les trouve.

## Le registre de seeders

Le framework garde une map ordonnée, globale au processus
(`IndexMap<String, fn() -> _>`), de chaque seeder enregistré. Trois
leviers la contrôlent.

### `register::<S>()`

Ajoutez un seeder au registre sous son `Seeder::name()` :

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

Deux choses à savoir sur le registre :

- **L'ordre compte.** `run_all` visite les seeders dans l'ordre où
  ils ont été enregistrés. Si `B` a besoin de lignes de `A`,
  enregistrez `A` en premier.
- **Réenregistrer un nom remplace en place.** L'emplacement garde sa
  position d'origine, le pointeur de fonction change. C'est
  intentionnel - cela permet à un test de lier un seeder factice
  par-dessus le vrai sans décaler l'ordre. En code de production,
  enregistrez chaque seeder exactement une fois au boot.

### `run_all()`

Exécute chaque seeder enregistré dans l'ordre d'enregistrement. C'est
ce que l'invocation nue `console db:seed` appelle.

```rust
suprnova::seed::run_all().await?;
```

S'arrête à la première erreur. Les seeders qui ont déjà tourné ne
sont pas annulés - `run_all` n'enveloppe pas le lot dans une
transaction, car la plupart des seeders s'étendent sur plusieurs
instructions et beaucoup de backends n'imbriquent pas proprement les
transactions. Si vous avez besoin d'une sémantique de rollback, ouvrez
la transaction à l'intérieur du seeder et gardez tout son travail dans
cette portée.

### `run_one(name)`

Exécute un seeder nommé sans exécuter les autres. C'est le moteur de
`db:seed --class=<Name>`, et c'est aussi utile depuis des scripts
ponctuels :

```rust
suprnova::seed::run_one("AdminAccountSeeder").await?;
```

Les ratés retournent
`FrameworkError::not_found("no seeder registered for \`X\`")`. La
commande console propage cela vers un code de sortie non nul et une
ligne sur stderr - pas de no-op silencieux.

### `count()` et `is_registered(name)`

Deux helpers de lecture, tous deux utiles dans les tests qui
affirment que « le bootstrap a bien câblé les seeders attendus » :

```rust
assert_eq!(suprnova::seed::count(), 3);
assert!(suprnova::seed::is_registered("BaseSeeder"));
```

Les deux retournent zéro / false sur un verrou de registre empoisonné
(après avoir journalisé une erreur), ce qui garde les tests
déterministes face à une panique amont.

## La commande `db:seed`

`db:seed` est une commande console fournie par le framework - elle
est livrée avec le framework et atterrit automatiquement dans le
binaire `console` de votre projet, via le même registre `inventory`
qui récupère vos propres `#[command]`. Voir [Console](console.md)
pour la mécanique du binaire ; cette section couvre la surface
spécifique aux seeders.

### Tout exécuter

```bash
cargo run --bin console -- db:seed
```

Exécute chaque seeder enregistré dans l'ordre. Sur un registre vide,
elle imprime un avertissement sur stderr
(`db:seed: no seeders registered - nothing to run`) et quitte avec le
code zéro - c'est le comportement correct pour « quelqu'un a exécuté
la commande avant d'enregistrer quoi que ce soit », et cela évite que
les suites de tests qui n'ont rien ensemencé de spécifique n'échouent.

### Exécuter un seeder

Trois formes acceptées, dans l'ordre croissant de leur ressemblance
avec Laravel :

```bash
cargo run --bin console -- db:seed --class=UsersSeeder
cargo run --bin console -- db:seed --class UsersSeeder
cargo run --bin console -- db:seed UsersSeeder
```

Les trois recherchent le seeder dans le registre par nom exact et
l'exécutent. Un nom inconnu échoue rapidement :

```bash
cargo run --bin console -- db:seed --class=NotARealSeeder
# Error: no seeder registered for `NotARealSeeder`
# (exit 1)
```

Un flag malformé (`--class` sans valeur suivante, `--class=` avec une
valeur vide, `--class --force`) échoue aussi rapidement, avec un
diagnostic qui nomme la forme attendue.

### Depuis un binaire construit

Dans un déploiement conteneurisé ou géré par systemd, le binaire
console vit à `target/release/console` (ou partout où votre artefact
de release atterrit). Même syntaxe, sans `cargo` devant :

```bash
./console db:seed
./console db:seed --class=BaseSeeder
```

Le binaire console appelle
`suprnova::console::dispatch_argv(std::env::args())`, qui route à
travers le même registre que `cargo run --bin console --`. Il n'y a
pas de chemin de dispatch séparé pour les artefacts construits.

## Composer avec des fabriques

Les seeders finissent presque toujours par appeler des
[fabriques](eloquent.md). Le trait de fabrique sait comment
construire une instance aléatoire d'un modèle ; le seeder enchaîne
les appels de fabrique et tout câblage non aléatoire (identifiants
admin déterministes, lignes de table jointe, téléversements de
fichiers).

Le duo minimal fabrique + seeder :

```rust
// src/factories/user_factory.rs
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        User {
            id: 0,                              // persist_via_seaorm bascule la PK sur NotSet
            name: "Factory User".into(),
            email: "factory@example.suprnova.app".into(),
            password: "factory-placeholder".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ..Default::default()
        }
    }
}
```

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

Le builder fluide vit sur `FactoryBuilder<M>` ; ce que vous pouvez
chaîner avant `create_many` correspond à Laravel :

```rust
// Construit une ligne persistée avec des redéfinitions :
let admin = UserFactory::new()
    .with(|u| u.email = "admin@example.com".into())
    .with(|u| u.role = "admin".into())
    .create()
    .await?;

// Construit N lignes persistées, toutes admins :
UserFactory::times(5)
    .with(|u| u.role = "admin".into())
    .create_many()
    .await?;

// État conditionnel - applique la closure seulement quand le flag
// est actif :
UserFactory::times(10)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

`make` / `make_one` / `make_many` sont les pendants en mémoire (sans
insertion) pour les tests unitaires qui ne veulent pas d'aller-retour
en base de données. Voir le chapitre [Eloquent](eloquent.md) pour la
surface complète des fabriques (y compris `prepend`, `Sequence`, et la
macro `#[derive(Factory)]` qui génère la struct marqueur depuis un
attribut `#[factory(model = "…")]`).

### L'idempotence est la responsabilité du seeder

`run_all` ne fait ni instantané ni enveloppement en transaction ; si
un seeder insère sans condition, le réexécuter produit des doublons.
Les deux façons standard de rendre un seeder sûr à réexécuter :

- **Réinitialiser d'abord.** La boucle « vider et réensemencer » du
  dev local fait généralement
  `suprnova migrate:fresh && cargo run --bin console -- db:seed` -
  `migrate:fresh` supprime et reconstruit chaque table, donc le
  seeder repart toujours de zéro. C'est la forme que la plupart des
  projets utilisent au jour le jour.
- **Upsert / vérifier avant.** Pour un seeder qui doit coexister avec
  des données existantes (un compte admin par défaut en production,
  la liste canonique des pays), protégez l'insertion avec une
  recherche, ou utilisez une requête upsert.

```rust
async fn run() -> Result<(), FrameworkError> {
    let exists = User::query()
        .db_where("email", "admin@example.com")
        .exists()
        .await?;

    if !exists {
        let password_hash = suprnova::hashing::hash("change-me-on-first-login")?;
        User::create(attrs!{
            email: "admin@example.com",
            name: "Admin",
            password: password_hash,
        }).await?;
    }
    Ok(())
}
```

## Faire taire les événements de modèle avec `without_events`

Un seeder qui appelle `Model::create` dans une boucle déclenche
chaque événement de cycle de vie - `Creating`, `Saving`, `Created`,
`Saved` - sur chaque ligne. Cela réveille tout `Observer<M>`
enregistré, exécute tout écouteur de diffusion mis en file d'attente,
et peut accessoirement mettre en file d'attente une centaine de jobs
en arrière-plan que vous ne voulez pas vraiment. `seed::without_events`
est l'analogue du `WithoutModelEvents` de Laravel :

```rust
use suprnova::{async_trait, FrameworkError, Seeder, seed};
use crate::models::users::User;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        seed::without_events(async {
            for i in 0..50 {
                User::create(attrs!{
                    name: format!("user{i}"),
                    email: format!("user{i}@example.com"),
                }).await?;
            }
            Ok(())
        }).await
    }
}
```

Pendant que la future intérieure attend, aussi bien le chemin de veto
annulable (`dispatch_cancellable`) que le fan-out post-événement
(`dispatch_after`) court-circuitent vers `Ok(())`. Les observateurs
sont silencieux, le diffuseur ne se réveille pas, les jobs en aval ne
se mettent pas en file d'attente.

L'effet est à portée de tâche - seul le travail effectué à l'intérieur
de `fut` est mis en sourdine. Le travail concurrent sur d'autres
tâches (handlers de requête HTTP, workers de file d'attente tournant
en arrière-plan, autres seeders) continue de déclencher les
événements normalement. Les appels imbriqués se composent : un bloc
`without_events` intérieur hérite du flag extérieur.

### Les fabriques contournent déjà les événements de modèle

Cela vaut la peine d'être su parce que ça change le moment où vous
vous tournez vers `without_events` : les fabriques persistent via
`ActiveModelTrait::insert` (l'impl `Persistable` sur le modèle
SeaORM), qui ne passe pas par les méthodes `create` / `save` du trait
`Model`. Il n'y a aucun dispatch d'événement de modèle à mettre en
sourdine sur un chemin piloté par fabrique. `seed::without_events`
est pour le code qui pilote directement le trait `Model` -
typiquement parce que vous avez besoin de l'ergonomie de forme à
l'exécution que les fabriques contournent, ou parce que vous touchez
un modèle en plein ensemencement sur lequel un observateur est censé
réagir en production mais pas pendant un chargement de fixture.

En pratique : si votre seeder est une pile d'appels
`UserFactory::new().create_many()`, vous n'avez pas besoin de
`without_events`. Si c'est une boucle écrite à la main de
`User::create(attrs)`, vous en avez probablement besoin.

## Utiliser des seeders dans les tests

Le même registre que pilote le binaire console est appelable depuis
un `#[tokio::test]` - pratique quand vous voulez un jeu de fixture
connu devant un test d'intégration :

```rust
use serial_test::serial;
use suprnova::container::testing::TestContainer;
use suprnova::{DbConnection, seed};

use app::seeders::BaseSeeder;

#[tokio::test]
#[serial]
async fn dashboard_renders_seeded_posts() {
    // Réinitialise le registre pour que les enregistrements d'un
    // test précédent ne fuitent pas.
    seed::clear();

    let _guard = TestContainer::fake();
    let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    app::migrations::Migrator::up(&conn, None).await.unwrap();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    // Enregistre le seeder voulu, l'exécute, et fait l'assertion
    // contre la base de données fraîche.
    seed::register::<BaseSeeder>();
    seed::run_all().await.unwrap();

    // …test de contrôleur contre les données ensemencées…

    seed::clear();
}
```

Deux remarques sur la forme du test :

- `#[serial]` est requis quand le test mute le registre global au
  processus - des tests parallèles partageant le même registre
  entreront en course. Ajoutez `serial_test` comme dev-dependency
  dans le `Cargo.toml` de votre projet pour obtenir l'attribut.
- `seed::clear()` est un helper `#[doc(hidden)]` réservé aux tests. Ne
  l'appelez pas depuis du code de production ; le registre est
  construit une fois au boot et n'est jamais réinitialisé.

Voir [Tests](testing.md) pour les conventions plus larges du harnais
de test (`#[suprnova_test]`, `TestContainer`,
`TestDatabase::fresh::<Migrator>()`, les fakes pour chaque surface
externe).

## Ensemencer, migrer, ou utiliser une fabrique

Ces trois motifs mettent tous des lignes dans des tables. La décision
est généralement simple, mais cela vaut la peine de nommer
explicitement les lignes de partage, parce que les équipes PHP les
brouillent souvent.

| Vous voulez… | Utilisez |
|---|---|
| Qu'une colonne existe | [Migration](migrations.md) |
| Une ligne qui doit exister pour que l'app démarre (l'admin par défaut, la ligne singleton de config du site, la liste canonique des devises) | **Seeder** - idempotent, s'exécute dans chaque environnement, y compris en production |
| Un jeu de lignes aléatoires pour le dev local ou le staging (50 utilisateurs, 200 posts, 1000 événements) | Seeder qui appelle une fabrique |
| Une ligne dont un test unitaire a besoin | [Fabrique](eloquent.md) appelée directement dans le test |
| La forme d'une ligne | [Fabrique](eloquent.md) |

Les erreurs à éviter :

- **N'insérez pas de données depuis une migration.** Les migrations
  décrivent le schéma, pas l'état. Une migration qui insère une ligne
  par défaut ne s'exécutera qu'une fois sur la base de données de
  production, puis plus jamais - au moment où une colonne change,
  vous avez une source de vérité scindée entre l'historique des
  migrations et le seeder. Mettez l'insertion dans un seeder ; si la
  production a besoin de la ligne, exécutez
  `console db:seed --class=DefaultsSeeder` comme partie du
  déploiement.
- **N'écrivez pas de données de fixture dans votre test à la main.**
  Tournez-vous vers une fabrique. Cinq blocs
  `User::create(attrs!{ … })` dans un test sont cinq réécritures au
  moment où vous ajoutez une colonne NOT NULL. Un seul
  `UserFactory::new().create()` survit.
- **Ne mettez pas de données de production dans un seeder.** Un
  seeder est pour les lignes dont l'application a besoin pour
  fonctionner, pas pour « voici les 8 000 enregistrements
  historiques que nous importons ». Les imports sont des scripts
  ponctuels (écrivez un `#[command]` pour eux ; voir
  [Console](console.md)).

### Pourquoi Suprnova diverge

Laravel livre une classe `DatabaseSeeder` avec un helper
`call($seeders)` traité comme cas spécial que le chargeur de seeders
d'Eloquent reconnaît. Suprnova ne le fait pas - le registre est un
`IndexMap` plat, chaque seeder est un pair, et un seeder composite
appelle `seed::run_one(name)` (ou appelle directement les
sous-fabriques) pour chaîner.

La raison est le même compromis que vous voyez ailleurs dans
Suprnova : un registre générique unique avec une seule règle d'ordre
est plus facile à raisonner qu'une hiérarchie de classes avec une
racine magique. Le motif Laravel fonctionne parce que l'autoload de
classes de PHP et la réflexion statique de `make()` laissent
`call([A::class, B::class])` trouver et instancier ces classes par
leur nom ; en Rust, nous demanderions à l'utilisateur de faire
circuler des objets trait `dyn Seeder`, ce qui est plus lourd que le
registre à pointeurs de fonction déjà en place.

La convention du seeder composite retrouve la même ergonomie -
`BaseSeeder` joue le rôle que `DatabaseSeeder` joue dans Laravel -
sans que le framework ait besoin de consacrer un nom comme spécial.

## Enregistrement au bootstrap

Chaque seeder a besoin d'un appel à `seed::register` dans
`bootstrap.rs`, à côté des autres câblages globaux au processus
(config, observateurs, superviseurs, jobs de file d'attente). Le
motif a la même forme qu'ailleurs dans le fichier bootstrap :

```rust
// src/bootstrap.rs
pub async fn register() {
    // …config + liaisons de conteneur + câblage auth…

    // Seeders. L'ordre compte - run_all visite dans l'ordre
    // d'enregistrement.
    suprnova::seed::register::<crate::seeders::BaseSeeder>();
    suprnova::seed::register::<crate::seeders::DemoContentSeeder>();

    // …observateurs, superviseurs, jobs de file d'attente…
}
```

Si vous oubliez d'enregistrer un seeder, `console db:seed --class=X`
échoue avec « no seeder registered for `X` » - un signal clair plutôt
qu'un skip silencieux. Les helpers `seed::count()` et
`seed::is_registered("…")` existent précisément pour qu'un test
puisse affirmer que le bootstrap a enregistré chaque seeder que vous
attendiez.

Voir [Amorçage de l'application](bootstrap.md) pour la structure
complète du fichier et l'ordre dans lequel le framework attend que
chaque sous-système soit câblé.

## Suivant

- [Migrations](migrations.md) - la moitié schéma du duo
  ensemencement/migration
- [Eloquent](eloquent.md) - les modèles, les fabriques, et la
  machinerie `Persistable` que chaque seeder appelle
- [Console](console.md) - le binaire `console` propre au projet qui
  héberge `db:seed` à côté de vos propres `#[command]`
- [Tests](testing.md) - `TestContainer`, `TestDatabase::fresh`, et le
  motif `#[serial]` pour les tests qui touchent au registre de
  seeders
- [Modèle d'erreur](error-model.md) - ce qu'est `FrameworkError` et
  comment la forme `Result<(), _>` de `run` se compose avec le reste
  du framework

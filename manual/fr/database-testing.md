# Tests de base de données

Le compagnon spécifique aux BD de [Tests](testing.md). Là où ce
chapitre couvre le harnais de test - `#[suprnova_test]`, `describe!` /
`test!`, `expect!`, et les fakes internes au processus - celui-ci
couvre ce qui change quand votre test a besoin d'une base de données :
comment `TestDatabase` en construit une pour vous, comment
l'isolation fonctionne réellement, où les fabriques et les seeders se
branchent, et quand un SQLite en mémoire suffit, ou ne suffit pas.

## Les deux constructeurs

Chaque test de base de données commence par construire un
`TestDatabase`. Deux constructeurs, deux intentions.

### `TestDatabase::fresh::<Migrator>()`

Construit une base de données SQLite en mémoire, exécute votre
migrateur de bout en bout, et enregistre la connexion dans le
conteneur de test pour que tout code appelant `DB::connection()` ou
`App::resolve::<DbConnection>()` s'y résolve. C'est le défaut correct
pour tout ce qui touche à un vrai schéma.

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn user_lifecycle_end_to_end() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);
    // Interroger directement quand vous voulez contourner la surface
    // du modèle :
    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`Migrator` est l'implémentation de `MigratorTrait` de votre
application - le même type que la commande de production
`suprnova migrate` exécute. En faisant passer le vrai migrateur à
travers le schéma de test, vous rendez la dérive de schéma
impossible : une colonne que le migrateur a oublié d'ajouter ne peut
pas être silencieusement présente dans la BD de test.

La macro `test_database!()` est du sucre pour le cas courant
(`crate::migrations::Migrator`) :

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();          // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}

// Ou avec un chemin de migrateur personnalisé :
let db = test_database!(my_crate::CustomMigrator);
```

### `TestDatabase::sqlite_memory()`

Même câblage de conteneur et de registre, mais **n'exécute aucun
migrateur**. Utilisez ceci quand le test veut un contrôle précis de
la forme des colonnes - typiquement des allers-retours de cast, des
tests de surface SQL du générateur de requêtes, ou des cas limites au
niveau du driver où un migrateur complet est excessif ou source de
bruit :

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared(
    "CREATE TABLE casts_t (id INTEGER PRIMARY KEY, payload BLOB)",
)
.await
.unwrap();

// Puis écrivez directement et relisez avec les helpers typés :
let row = db.fetch_one(
    "INSERT INTO casts_t (payload) VALUES (?) RETURNING id, payload",
    vec![sea_orm::Value::Bytes(Some(Box::new(b"hello".to_vec())))],
).await.unwrap();
```

`sqlite_memory()` est la fondation sur laquelle `fresh()` est
construit - `fresh` l'appelle puis exécute votre migrateur. Tout ce
que vous pouvez faire avec `fresh`, vous pouvez le faire ici ; vous
apportez juste votre propre DDL.

### `execute_unprepared`, `fetch_one`, `fetch_all`

`TestDatabase` réexporte les trois formes d'exécution de SeaORM vers
lesquelles vous vous tournez le plus souvent dans les tests, pour que
les fichiers de test n'aient pas à importer `ConnectionTrait` :

| Méthode | À utiliser pour |
| --- | --- |
| `execute_unprepared(sql)` | DDL ou DML sans placeholder. Retourne `Result<(), FrameworkError>` |
| `fetch_one(sql, bindings)` | SELECT à une ligne. Échoue si zéro ligne |
| `fetch_all(sql, bindings)` | SELECT sur toutes les lignes |

Les bindings sont un `Vec<sea_orm::Value>` - la même forme que le
chemin de requête de production utilise. Le backend de la connexion
(SQLite pour les deux constructeurs) est fourni pour vous, donc un
placeholder `?` est correct.

## Comment l'isolation fonctionne réellement

Le modèle base-de-données-fraîche-par-test est le mécanisme
d'isolation. Chaque appel à `fresh()` ou `sqlite_memory()` ouvre une
nouvelle connexion `sqlite::memory:`, qui sous SQLite est une instance
de base de données entièrement séparée - aucun schéma partagé, aucune
ligne partagée, aucun autre test ne peut y voir. Il n'y a pas
d'enveloppement de transaction, pas de trait `RefreshDatabase` auquel
s'inscrire, et pas de rollback à retenir : le test *suivant* obtient
une BD vide et propre parce qu'il construit la sienne.

Quand la valeur `TestDatabase` est droppée, trois choses se
produisent, dans cet ordre :

1. La `TestContainerGuard` détenue vide le conteneur de test
   thread-local, si bien qu'un `App::get::<DbConnection>()` suivant ne
   trouve plus la connexion de test.
2. Si c'était la *dernière* `TestContainerGuard` vivante dans le
   processus, le [`ConnectionRegistry`](database.md#named-connections)
   nommé est effacé. (Un compte de références sur `FAKE_GUARDS`
   garantit que le drop d'un test intérieur ne peut pas effacer un nom
   de connexion dont un test extérieur concurrent dépend encore -
   le piège permanent qui a motivé ce compte de références.)
3. La connexion SQLite elle-même se droppe, ce qui détruit la base de
   données en mémoire.

Parce que l'état est reconstruit plutôt qu'annulé, l'isolation est
plus forte qu'un enveloppement `BEGIN`/`ROLLBACK` : il n'y a pas
d'état commité qui puisse survivre par erreur, pas de bizarrerie de
transaction imbriquée, pas de dérive de compteur de séquence entre les
tests. Le coût est que vous payez pour l'exécution du migrateur une
fois par test (négligeable pour SQLite avec la plupart des schémas ;
si cela devient un coût réel, voir « Partager une base de données
migrée entre les tests » plus bas).

## Pourquoi le pool est épinglé à une seule connexion

Les deux constructeurs construisent la base de données avec
`max_connections(1)` et `min_connections(1)`. C'est porteur pour
`sqlite::memory:`, pas une policy générique.

`sqlite::memory:` est une base de données par connexion - chaque
*nouvelle* connexion dans le pool serait une instance SQLite séparée
et vide. Un pool de taille 2 signifierait que la moitié de vos
requêtes voient la base migrée et l'autre moitié une base vide.
Épingler le pool à une seule connexion fait que chaque requête du test
atterrit sur la même base en mémoire contre laquelle le migrateur a
tourné.

La conséquence : un test qui exerce une vraie concurrence de
connexion (deux transactions en course, du routage de réplique, un
worker de file d'attente qui tape la BD pendant qu'un handler de
requête le fait aussi) a besoin d'une vraie base de données. Voir
« Quand SQLite en mémoire ne suffit pas » plus bas.

## Les fabriques dans les tests

Les fabriques produisent des instances de modèle aléatoires et les
persistent (facultativement). Le chemin de persistance résout
automatiquement la connexion de test liée - il n'y a aucun câblage
côté fabrique à faire pour les tests.

```rust
use crate::factories::UserFactory;

#[tokio::test]
async fn factory_round_trip() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // En mémoire seulement : le plus rapide, aucun aller-retour BD.
    let alice = UserFactory::new()
        .with(|u| u.email = "alice@example.com".into())
        .make();
    assert_eq!(alice.email, "alice@example.com");

    // Persiste un + retourne le modèle post-insertion (id assigné).
    let bob = UserFactory::new().create().await.unwrap();
    assert!(bob.id > 0);

    // En masse : persiste 50 en séquence.
    let many = UserFactory::times(50).create_many().await.unwrap();
    assert_eq!(many.len(), 50);
}
```

Deux motifs qui valent la peine d'être connus :

**Les insertions de fabrique contournent les événements de modèle.**
L'impl `Persistable` qui adosse `create()` / `create_many()` écrit
directement via `ActiveModelTrait::insert` de SeaORM - elle ne passe
*pas* par la surface `Model::create` qui dispatche `Creating` /
`Created` / `Saving` / `Saved`. Un test qui affirme « aucun
observateur ne se déclenche pendant que nous construisons la
fixture » n'a besoin de rien de spécial ; un test qui affirme
« l'observateur `Created` S'EST bien déclenché » doit passer par
`Model::create(...)` (ou `save()`) au lieu d'une fabrique.

**`create_many` ne s'exécute pas dans une transaction.** Les
insertions sont séquentielles. Si une ligne ultérieure échoue, les
lignes précédentes ne sont pas annulées. Enveloppez l'appel dans votre
propre `DB::transaction` si un test exige l'atomicité :

```rust
DB::transaction(|tx| async move {
    UserFactory::times(50).create_many().await?;
    PostFactory::times(200).create_many().await?;
    Ok::<_, FrameworkError>(())
}).await.unwrap();
```

Voir [Eloquent → Fabriques](eloquent-factories.md) pour la surface
complète des fabriques (états, séquences, relations `with`, `count`,
`times`, `make_one` / `create_one`).

## Les seeders dans les tests

Les seeders sont des fonctions que vous avez enregistrées dans le
registre de seeders du framework sous un nom stable. Deux motifs pour
les piloter depuis les tests, un pour chaque axe d'intention.

### Exécuter un seul seeder par son nom

```rust
use suprnova::seed;
use my_app::seeders::UsersSeeder;

#[tokio::test]
async fn users_seeder_populates_fixtures() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<UsersSeeder>();
    seed::run_one("UsersSeeder").await.unwrap();

    let count = User::query().count().await.unwrap();
    assert!(count > 0);
}
```

### Exécuter l'ensemble des seeders du bootstrap

```rust
use serial_test::serial;
use suprnova::seed;

#[tokio::test]
#[serial]
async fn full_seed_lands_expected_row_counts() {
    seed::clear();                              // repartir d'un registre connu comme vide
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<my_app::seeders::UsersSeeder>();
    seed::register::<my_app::seeders::PostsSeeder>();
    seed::run_all().await.unwrap();

    let users = User::query().count().await.unwrap();
    let posts = Post::query().count().await.unwrap();
    assert_eq!(users, 50);
    assert_eq!(posts, 200);

    seed::clear();
}
```

Deux détails de contrat importants :

**Le registre de seeders est global au processus.**
`seed::register::<S>()` insère dans un `RwLock<IndexMap>` indexé par
`S::name()`. Un test qui mute le registre devrait appeler
`seed::clear()` à l'entrée, enregistrer les seeders dont il a besoin,
s'exécuter, puis rappeler `clear()` à la sortie - et le test lui-même
devrait être `#[serial_test::serial]` pour que deux tests parallèles ne
se disputent pas le registre. `#[suprnova_test]` n'enregistre **pas**
automatiquement les seeders ; seul l'appel explicite à
`seed::register::<>()` dans votre propre `bootstrap.rs` ou dans le
corps du test les place dans le registre.

**Ensemencement piloté par les modèles contre ensemencement piloté
par les fabriques.** Un seeder qui boucle sur `User::create(...)`
dans un `for` déclenche `Creating` / `Saving` / `Created` / `Saved`
par ligne et invoque chaque observateur enregistré. Pour un
ensemencement en masse où ce fan-out n'est pas souhaité, enveloppez la
boucle dans `seed::without_events` :

```rust
seed::without_events(async {
    for i in 0..50 {
        User::create(attrs! { name: format!("user{i}"), email: format!("user{i}@example.com") }).await?;
    }
    Ok::<_, FrameworkError>(())
}).await?;
```

La mise en sourdine est **à portée de tâche** - seul le travail
effectué à l'intérieur de la future est réduit au silence ; les
handlers de requête concurrents et les workers de file d'attente
continuent de déclencher les événements normalement. Les fabriques
(`create_many`) contournent déjà le chemin des événements, donc
`without_events` est superflu autour d'elles.

Voir [Ensemencement](seeding.md) pour la surface d'écriture des
seeders et [Eloquent → Fabriques](eloquent-factories.md) pour la
relation entre les deux.

## Tests de base de données sûrs en parallèle

`cargo test` exécute les tests en parallèle par thread. L'expansion
par défaut de `#[suprnova_test]` (qui est `#[tokio::test]`,
c'est-à-dire un runtime `current_thread` par test) interagit sans
danger avec cela pour deux raisons :

- **Chaque test obtient sa propre connexion `sqlite::memory:`.** Les
  tests ne partagent pas l'état de la BD.
- **La connexion liée vit dans le `TestContainer` thread-local.** Les
  tests ne partagent pas les liaisons du conteneur.

Ce à quoi vous n'avez pas à penser : `DB::connection()`,
`App::resolve`, la persistance de fabrique, les écritures du trait
modèle - tout cela atterrit de façon transparente sur la bonne base de
données par test.

Ce à quoi vous devez *bien* penser :

| Surface | Pourquoi c'est global au processus | Mitigation |
| --- | --- | --- |
| `ConnectionRegistry` (`DB::register_named`, `__read_replica__`) | Un seul `RwLock<HashMap>` partagé par le processus | `#[serial_test::serial]` pour tout test qui enregistre ou lit des connexions nommées |
| Le registre de seeders | Un seul `RwLock<IndexMap>` | `#[serial_test::serial]` + `seed::clear()` à l'entrée et à la sortie |
| Les registres d'observateurs / de scopes Eloquent | Indexés par `TypeId::<M>()` | Chaque test devrait utiliser une struct de modèle unique, ou être `#[serial]` et appeler le helper `clear()` du registre |
| Le journal de requêtes nommé (`DB::enable_query_log`) | Un seul buffer circulaire global au processus | `#[serial]` si des assertions lisent le journal |

Le compte de références du registre de connexions rend ceci plus sûr
qu'il ne le paraît : un test qui détient une `TestContainerGuard`
maintient le registre en vie même quand la garde d'un test *voisin* se
droppe. Vous voulez quand même `#[serial]` pour les tests qui mutent
réellement le registre, pour que leurs lectures et écritures ne
puissent pas s'entrelacer.

### Mise en garde sur le runtime multi-thread

`#[suprnova_test]` se développe en `#[tokio::test]` avec le runtime
`current_thread` par défaut, donc le chemin du conteneur thread-local
fonctionne toujours. Si vous faites explicitement opter un test pour
le runtime multi-thread :

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_io_test() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // PROBLÈME : les tâches lancées avec `tokio::spawn` peuvent
    // s'exécuter sur un thread de travail différent de celui qui a
    // construit le TestDatabase. Elles ne verront pas la liaison
    // TestContainer thread-local, et DB::connection() retournera
    // la valeur du conteneur global (de production), ou une
    // erreur.
}
```

Deux correctifs, selon ce que fait le test :

1. **Accès direct à la connexion** - `db.conn()` retourne toujours la
   bonne `&DatabaseConnection` quel que soit le thread de travail qui la
   lit. Si le test ne parle jamais à la BD qu'à travers le handle
   `db` (et non via `DB::connection()`), le runtime multi-thread ne
   pose pas de problème.

2. **`TestContainer::scope`** - enveloppez le corps du test dans
   `TestContainer::scope(async { ... }).await` et liez vos fakes (et
   la connexion BD) à l'intérieur. La portée lie le conteneur à la
   couche task-local, qui est préservée à travers les awaits même
   quand le runtime fait sauter la future entre les threads de travail.
   Pour les sous-tâches lancées, utilisez `TestContainer::spawn` (et
   non un `tokio::spawn` nu) pour que le conteneur task-local soit
   capturé et réinstallé à l'intérieur de la future lancée.

Voir [Conteneur de service → Ordre de recherche](container.md) pour la
stratification complète task-local / thread-local / globale.

## SQLite en mémoire contre un vrai Postgres / MySQL / MariaDB

`TestDatabase` est délibérément réservé à SQLite. Le driver est codé
en dur sur `sqlite::memory:` ; il n'y a pas de
`TestDatabase::postgres()`, de `fresh_with_url()`, ni de variante
pilotée par l'environnement. Pour l'écrasante majorité de la surface
de test - CRUD de modèle, forme du générateur de requêtes,
allers-retours de cast, chargement de relation, ordre de déclenchement
des observateurs, sémantique de suppression logicielle - SQLite en
mémoire est le bon outil : zéro configuration, zéro réseau,
millisecondes par test, isolation parfaite, aucun service externe à
maintenir en vie en CI.

Il y a quatre cas où SQLite en mémoire ne suffit pas :

1. **SQL spécifique à un driver.** Une requête qui utilise `LATERAL`
   de Postgres, les opérateurs `JSONB`, `ON CONFLICT ... WHERE`, les
   fonctions de fenêtrage de MySQL, ou toute autre surface propre à
   un dialecte ne s'exécutera pas sur SQLite. Le chemin
   modèle+builder essaie de rester générique, mais un test SQL brut
   qui affirme une sortie à la forme Postgres a besoin de Postgres.
2. **Concurrence sous une vraie contention de connexion.** SQLite en
   mémoire est mono-connexion (voir « Pourquoi le pool est épinglé à
   une seule connexion »). Les tests qui font courir deux
   transactions l'une contre l'autre, exercent le routage de réplique
   de lecture sous charge, ou mesurent le réessai sur deadlock ont
   besoin d'un serveur multi-connexion.
3. **Surfaces vectorielles / NoSQL / temporelles.** Le driver
   `VECTOR` MariaDB de Suprnova, l'intégration Qdrant, l'intégration
   Pinecone, et les drivers non-SQL similaires ne peuvent absolument
   pas être modélisés dans SQLite.
4. **Tests de fumée de parité avec la production.** Une poignée de
   tests du genre « est-ce que ça marche vraiment sur la vraie BD sur
   laquelle nous déployons ? », réservés à la CI, valent la peine
   d'être gardés même quand la couche de tests unitaires est SQLite.

Dans les quatre cas, le motif est le même : sortez entièrement de
`TestDatabase`, construisez une `DbConnection` contre une variable
d'env de style `DATABASE_URL` fournie par l'opérateur, filtrez le
test par l'env pour qu'il soit ignoré quand la variable est absente,
et marquez-le `#[serial]` pour que deux d'entre eux ne se disputent
pas la vraie base de données partagée. Le motif `MARIADB_URL` dans
`framework/tests/vector_mariadb.rs` est l'exemple canonique :

```rust
use serial_test::serial;
use suprnova::database::{DatabaseConfig, DbConnection};

async fn maybe_real_db(test_name: &str) -> Option<DbConnection> {
    let url = match std::env::var("POSTGRES_TEST_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("[{test_name}] skipping: POSTGRES_TEST_URL not set");
            return None;
        }
    };
    let config = DatabaseConfig::builder().url(&url).build();
    Some(DbConnection::connect(&config).await.expect("real DB connects"))
}

#[tokio::test]
#[serial]
async fn jsonb_operator_works_against_postgres() {
    let Some(conn) = maybe_real_db("jsonb_operator_works_against_postgres").await else {
        return;
    };
    // Piloter du SQL spécifique à Postgres directement contre `conn`.
}
```

La convention en vigueur : nommez la variable d'env d'après le driver
cible (`POSTGRES_TEST_URL`, `MYSQL_TEST_URL`, `MARIADB_URL`), imprimez
une ligne de skip pour qu'un développeur exécutant la suite localement
voie que le test a été ignoré (et non silencieusement réussi), et
documentez la variable d'env dans le doc-comment en tête du module de
test pour que la CI puisse la câbler.

## Un exemple complet

Le motif complet de dogfooding de l'app, combinant tout ce chapitre :

```rust
use app::migrations::Migrator;
use app::models::posts::Post;
use app::models::users::User;
use serial_test::serial;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, seed, FrameworkError};

#[tokio::test]
#[serial]
async fn users_and_posts_full_seed_round_trip() {
    // 1. Vider le registre de seeders.
    seed::clear();

    // 2. BD en mémoire fraîche avec le migrateur de l'app.
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // 3. Enregistrer les seeders dont le test se préoccupe.
    seed::register::<app::seeders::UsersSeeder>();
    seed::register::<app::seeders::PostsSeeder>();

    // 4. Piloter l'ensemencement à l'intérieur de without_events
    //    pour que le fan-out des observateurs n'essaie pas de
    //    mettre des jobs en file d'attente (aucune file d'attente
    //    ne tourne ici).
    seed::without_events(async {
        seed::run_all().await
    }).await.unwrap();

    // 5. Relire via la surface du modèle et la connexion brute.
    let user_count = User::query().count().await.unwrap();
    assert_eq!(user_count, 50);

    let raw_post_count = db.fetch_one(
        "SELECT COUNT(*) AS n FROM posts",
        vec![],
    ).await.unwrap();
    let n: i64 = raw_post_count.try_get("", "n").unwrap();
    assert_eq!(n, 200);

    // 6. Exercer le chemin d'observateur annulable sur un modèle
    //    frais.
    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    }).await.unwrap();
    assert!(alice.id > 0);

    seed::clear();
}
```

L'étape 5 est la partie qui prouve le câblage : la requête modèle et
le `fetch_one` brut lisent tous deux la même base de données en
mémoire - la surface du modèle parce que la recherche
`DB::connection()` a trouvé la liaison `TestContainer`, le `fetch_one`
brut parce que `db.conn()` retourne directement cette même connexion.

## Références croisées

- [Tests](testing.md) - le harnais de test, `expect!`, `describe!`,
  `test!`, les fakes.
- [Base de données](database.md#testing) - la section de test au
  niveau de la surface qui introduit `TestDatabase`.
- [Eloquent → Fabriques](eloquent-factories.md) - la syntaxe de
  définition de fabrique, les états, les séquences, les relations.
- [Ensemencement](seeding.md) - l'écriture de seeders, l'ordre,
  l'idempotence.
- [Conteneur de service](container.md) - la recherche task-local
  contre thread-local contre globale, qui décide ce vers quoi
  `DB::connection()` se résout à l'intérieur d'un test.
- [Mocking et doublures](mocking.md) - `Storage::fake`, `Mail::fake`,
  `Queue::fake`, `Notification::fake`, et le motif de liaison de
  trait pour substituer des clients HTTP fictifs et d'autres
  surfaces externes.
- [Tests HTTP](http-tests.md) - piloter des handlers à travers la
  pile de routage avec un `TestDatabase` lié.

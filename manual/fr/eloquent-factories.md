# Fabriques Eloquent

Les fabriques produisent des instances de modèle randomisées pour les
tests et les seeders. La forme est celle de Laravel :
`UserFactory::new().count(10).create_many().await?`. Le contrat est un
trait plus un builder fluide, avec un raccourci `#[derive(Factory)]`
pour le cas courant où le modèle a déjà une représentation randomisée
sensée.

Ce chapitre couvre la définition de fabriques à la main et par derive,
la composition de redéfinitions en « variantes » réutilisables, les id
déterministes via `Sequence`, le point de couture `Persistable` qui
alimente `create`, et la différence entre `make` (en mémoire) et
`create` (persisté). Pour le contexte d'écriture de tests où les
fabriques sont les plus utiles, voir [Tests](testing.md).

## Le trait `Factory`

Le trait a exactement une méthode requise :

```rust
pub trait Factory {
    type Model;

    fn definition() -> Self::Model
    where
        Self: Sized;
}
```

`definition()` retourne un modèle entièrement rempli, avec chaque
champ randomisé vers ce qui a du sens comme défaut. Le trait ne porte
aucun état par instance - les implémenteurs sont typiquement des
marqueurs de taille nulle (`struct UserFactory;`) pour qu'un appelant
puisse atteindre la fabrique par son nom sans détenir de handle.

Le trait fournit aussi deux points d'entrée de builder avec des
implémentations par défaut :

```rust
fn new() -> FactoryBuilder<Self::Model>;       // count = 1, aucune redéfinition
fn times(n: usize) -> FactoryBuilder<Self::Model>;  // sucre pour new().count(n)
```

Chaque autre méthode que vous appellerez (`with`, `count`, `make`,
`create`, `create_many`, …) vit sur `FactoryBuilder<M>`.

## Définir une fabrique à la main

La forme minimale écrite à la main associe une struct marqueur à un
impl `Factory` qui sait construire une instance. Vous vous tournerez
typiquement vers cela quand le modèle ne dérive pas `fake::Dummy` -
peut-être parce que certains champs doivent être initialisés de façon
déterministe (des id de relation dans une plage connue) ou que la
représentation randomisée a besoin de connaître les règles métier :

```rust
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        let now = chrono::Utc::now();
        User {
            // `0` est un placeholder - `persist_via_seaorm` fait passer les
            // colonnes de clé primaire à `NotSet` avant l'insertion pour que
            // la base de données assigne le vrai id.
            id: 0,
            name: format!("Factory User #{}", next_seq()),
            email: format!("factory-{}@example.test", next_seq()),
            password: "factory-placeholder".into(),
            remember_token: None,
            active: true,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

Les champs `__eager` et `__pivot` sont les champs de travail internes
de chargement hâtif et de pivot que la macro `#[suprnova::model]`
injecte sur chaque struct Eloquent. Laissez-les toujours à leur
défaut - ils sont peuplés par le query builder, pas par les fabriques.

`next_seq()` peut être ce que vous voulez - un `static AtomicU64`, une
`Sequence` (couverte plus bas), ou un compteur thread-local.
L'important est que `definition()` s'exécute à nouveau à chaque appel
à l'intérieur de `make_many` / `create_many`, si bien que toute
unicité dont vous avez besoin doit venir d'un compteur que la fonction
peut atteindre.

## `#[derive(Factory)]` pour le cas courant

Quand le modèle lui-même implémente `fake::Dummy` - soit via
`#[derive(Dummy)]`, soit via un `impl Dummy<Faker> for Model` écrit à
la main - le derive réduit le marqueur + impl à une seule ligne sur le
modèle :

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub author_id: i64,
    pub is_public: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Le derive émet `pub struct PostFactory;` comme type frère, et un
`impl Factory for PostFactory` dont `definition()` appelle
`Faker.fake::<Post>()`. La visibilité de la fabrique reflète la
visibilité du modèle - un modèle `pub` obtient une fabrique `pub`, un
modèle `pub(crate)` obtient une fabrique `pub(crate)`.

### Redéfinir le nom généré

Par défaut, `#[derive(Factory)]` émet `<Model>Factory`. Redéfinissez
via l'attribut `name` :

```rust
#[derive(Dummy, Factory)]
#[factory(name = "AccountFactory")]
pub struct User { /* … */ }
```

La valeur doit s'analyser comme un identifiant Rust - `name = "User
Factory"` ou `name = "user-factory"` échoue à la compilation avec une
erreur claire pointée sur le span. La macro émet littéralement
`pub struct <Name>;`, donc tout ce qui ne peut pas être un nom de type
ne peut pas être un nom de fabrique.

### `Dummy` écrit à la main pour une randomisation plus riche

`#[derive(Dummy)]` fonctionne pour les structs à champs primitifs mais
ne vous donne aucun contrôle sur les distributions ou les invariants
inter-champs. Pour tout ce qui n'est pas trivial, écrivez l'impl
`Dummy` à la main et associez-le à `#[derive(Factory)]` :

```rust
use suprnova::__fake::rand::Rng;
use suprnova::__fake::{Dummy, Fake, Faker, faker::lorem::en::{Paragraph, Sentence}};
use suprnova::Factory;

#[derive(Factory)]
pub struct Post { /* fields … */ }

impl Dummy<Faker> for Post {
    fn dummy_with_rng<R: Rng + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        let title: String = Sentence(3..7).fake_with_rng(rng);
        let body: String = Paragraph(3..6).fake_with_rng(rng);
        let author_id: i64 = (1..=50i64).fake_with_rng(rng);
        let now = chrono::Utc::now();

        Post {
            id: 0,
            author_id,
            title,
            body,
            is_public: Faker.fake_with_rng::<bool, _>(rng),
            created_at: now,
            updated_at: now,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

La crate `fake` est réexportée sous `suprnova::__fake` pour que les
consommateurs n'aient pas besoin d'une ligne `fake = "…"` séparée dans
`Cargo.toml`. Les types courants sont aussi réexportés à la racine de
la crate : `suprnova::{Dummy, Fake, Faker}`.

### Pourquoi `#[derive(Factory)]` ne prend que des structs ordinaires

Le derive rejette les enums, les unions, et les modèles génériques
avec une erreur de compilation claire. Les enums et les unions n'ont
pas de représentation par défaut qui ait du sens. Les génériques
forceraient une décision sur la façon dont le type fabrique
paramétrise son modèle - et il n'y a pas de bon défaut, donc le derive
refuse de deviner. Écrivez l'impl `Factory` à la main pour ces cas.

## Le builder fluide

`Factory::new()` / `Factory::times(n)` retournent un
`FactoryBuilder<M>`. Chaque opération est chaînable ; rien ne se passe
avant que vous n'appeliez une méthode terminale (`make`, `make_one`,
`make_many`, `create`, `create_one`, `create_many`).

### `count(n)` - combien d'instances

```rust
let user = UserFactory::new().make();             // 1 utilisateur
let users = UserFactory::new().count(10).make_many();  // 10 utilisateurs
let same = UserFactory::times(10).make_many();   // identique
```

`count(n)` est ignoré par `make` / `create` (toujours un) et honoré
par `make_many` / `create_many`. `times(n)` n'est que du sucre pour
`Self::new().count(n)` et correspond au `Factory::times($n)` de
Laravel.

### `with(|m| { … })` - redéfinitions par appel

`with` enregistre une closure qui s'exécute contre chaque instance
produite après `definition()`. Plusieurs appels à `with` se composent
dans l'ordre d'enregistrement, si bien qu'une redéfinition plus
tardive écrase une plus ancienne sur le même champ :

```rust
let admin = UserFactory::new()
    .with(|u| u.active = true)
    .with(|u| u.role = "admin".into())
    .make();
```

Les redéfinitions sont stockées comme
`Box<dyn Fn(&mut M) + Send + Sync + 'static>` pour que le builder
reste `Send` - important pour les chemins async `create` /
`create_many`, qui détiennent le builder à travers un `.await` sur
l'insertion SeaORM.

### `prepend(|m| { … })` - défauts que les appelants peuvent toujours redéfinir

`prepend` insère une closure au **début** de la chaîne de
redéfinitions, si bien qu'elle s'exécute **avant** tout autre
`with(...)`. Utilisez-le à l'intérieur d'une méthode de variante quand
vous voulez fournir un défaut que l'appelant peut encore écraser avec
un `.with(...)` plus tardif :

```rust
impl UserFactory {
    /// Méthode de variante - défauts admin, l'appelant peut encore personnaliser.
    pub fn admin() -> suprnova::FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.active = true)
    }
}

// L'appelant gagne sur `role` parce que son .with() vient après les prepends.
let owner = UserFactory::admin()
    .with(|u| u.role = "owner".into())
    .make();
```

C'est l'équivalent Suprnova du `Factory::prependState` de Laravel.
C'est le bon primitif spécifiquement pour les méthodes de variante -
`with` perdrait face au `.with(...)` d'un appelant, ce qui est
l'opposé de ce qu'un défaut devrait faire.

### `when(cond, |b| { … })` - chaînage conditionnel

`when` fait transiter un flag à travers une chaîne sans casser le
style fluide. La closure reçoit le builder, retourne le builder. Quand
`cond` est faux, le builder passe inchangé :

```rust
UserFactory::times(10)
    .with(|u| u.active = true)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

Reflète le `Conditionable::when($cond, $cb)` de Laravel. La signature
`FnOnce(Self) -> Self` veut dire que vous pouvez faire `await` à
l'intérieur de la closure, à condition de faire `.await` avant de
retourner le builder.

### Méthodes terminales

| Méthode | Retourne | Persisté ? |
|---|---|---|
| `make()` | un `M` | non |
| `make_one()` | un `M` (force count = 1) | non |
| `make_many()` | `Vec<M>` de `count` éléments | non |
| `create()` | `Result<M, FrameworkError>` | oui |
| `create_one()` | `Result<M, FrameworkError>` (force count = 1) | oui |
| `create_many()` | `Result<Vec<M>, FrameworkError>` | oui |

`make_one` et `create_one` sont utiles quand une méthode de variante a
positionné `count` en interne à autre chose et que l'appelant veut
exactement un résultat :

```rust
pub fn admins_in_org(org_id: i64) -> suprnova::FactoryBuilder<User> {
    UserFactory::times(5)               // défaut raisonnable pour les fixtures
        .with(move |u| u.org_id = org_id)
        .with(|u| u.role = "admin".into())
}

// Le test ne veut qu'un seul - `create_one` ignore le count(5).
let admin = admins_in_org(42).create_one().await?;
```

## Variantes : combinaisons de préréglages réutilisables

Suprnova ne fournit pas de table de recherche `state("name")`. À la
place, les variantes sont de simples méthodes sur votre marqueur de
fabrique qui retournent un `FactoryBuilder<M>` préconfiguré. Le motif
se compose par héritage - chaque méthode de variante retourne le même
type `FactoryBuilder<M>`, si bien que vous pouvez chaîner davantage de
méthodes sur le résultat :

```rust
use suprnova::FactoryBuilder;
use crate::models::users::User;

pub struct UserFactory;

impl suprnova::Factory for UserFactory {
    type Model = User;
    fn definition() -> User { /* … */ }
}

impl UserFactory {
    /// Variante inactive - superpose un défaut `active: false`.
    pub fn inactive() -> FactoryBuilder<User> {
        Self::new().prepend(|u| u.active = false)
    }

    /// Variante admin - superpose le rôle + un e-mail vérifié.
    pub fn admin() -> FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.email_verified_at = Some(chrono::Utc::now()))
    }

    /// Composable : admin inactif.
    pub fn inactive_admin() -> FactoryBuilder<User> {
        Self::admin().prepend(|u| u.active = false)
    }
}
```

```rust
// Composez aussi au site d'appel - chaînez librement d'autres redéfinitions.
let user = UserFactory::admin()
    .with(|u| u.name = "Alice".into())
    .create()
    .await?;

let batch = UserFactory::inactive().count(20).create_many().await?;
```

Le choix de `prepend` est délibéré : les redéfinitions d'une variante
sont des *défauts* que l'appelant peut encore réécrire. Si vous voulez
qu'un réglage de variante soit non négociable, utilisez `with` à la
place - il va à la fin de la chaîne et gagne.

### Pourquoi pas de recherche par `state("name")`

Un registre de variantes indexé par nom forcerait une correspondance
de chaîne à l'exécution pour quelque chose que le compilateur peut
vérifier. Les méthodes de variante vous donnent une vérification à la
compilation (la coquille `UserFactor::admn()` est une erreur dure) et
l'autocomplétion IDE complète. La composabilité - chaîner
`Self::admin()` depuis l'intérieur de `inactive_admin()` - vient
gratuitement.

## Id déterministes avec `Sequence`

`Sequence` est un compteur monotone pour initialiser des champs
uniques par appel. Chaque appel à `next()` retourne 1, 2, 3, … de
façon atomique à travers les threads :

```rust
use suprnova::{Fake, Sequence};

static ORDER_IDS: Sequence = Sequence::new();

pub struct OrderFactory;
impl suprnova::Factory for OrderFactory {
    type Model = Order;
    fn definition() -> Order {
        Order {
            id: 0,
            number: format!("ORD-{:06}", ORDER_IDS.next()),
            total_cents: (100..=10_000).fake(),
            created_at: chrono::Utc::now(),
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

`Sequence::new()` est `const`, donc il fonctionne comme initialiseur
`static`. Le compteur démarre à 0 et s'incrémente à 1 au premier
appel. Utilisez `reset()` entre les tests si vous voulez un compte
propre - la macro `#[suprnova_test]` ne le fait pas pour vous parce
que le framework ne peut pas savoir quelles séquences sont les
vôtres :

```rust
#[suprnova::suprnova_test]
async fn each_order_gets_a_unique_number(db: TestDatabase) {
    ORDER_IDS.reset();   // démarre à 1 pour ce test
    let orders = OrderFactory::new().count(5).create_many().await?;
    assert_eq!(orders[0].number, "ORD-000001");
    assert_eq!(orders[4].number, "ORD-000005");
}
```

`Sequence` utilise l'ordering `SeqCst` - excessif pour « donne-moi un
id unique » mais garde le raisonnement trivial. Si une `Sequence`
apparaît un jour dans un hot path, vous pouvez écrire la vôtre avec
`Relaxed`.

## `Persistable` : le point de couture vers votre stockage

La famille de méthodes `create` est disponible dès que le modèle
implémente `Persistable` :

```rust
#[async_trait]
pub trait Persistable: Sized + Send {
    async fn persist(self) -> Result<Self, FrameworkError>;
}
```

Un impl générique dans `factory::persist` couvre chaque modèle SeaORM
qui peut `IntoActiveModel<ActiveModel>` - c'est-à-dire chaque modèle
que la macro `#[suprnova::model]` émet. Aucun boilerplate par modèle ;
si `User` est un modèle, `UserFactory::new().create()` fonctionne.

L'impl générique récupère `DB::connection()` et insère. Le `Self`
retourné est ce que SeaORM remet après l'insertion - id assigné,
colonnes par défaut résolues, etc.

### Gestion de la clé primaire

Un impl `IntoActiveModel` de SeaORM marque chaque champ - y compris la
PK - comme `Set(value)`. Pour les modèles produits par une fabrique,
la PK est un placeholder (`0` pour un `AUTO_INCREMENT i64`), si bien
qu'une insertion directe entre en collision au second appel avec un
échec de contrainte UNIQUE.

`persist_via_seaorm` (le helper qui soutient l'impl générique) fait
passer chaque colonne de clé primaire à `NotSet` avant l'insertion, ce
qui laisse la base de données assigner son propre id - la sémantique
dont les fabriques ont réellement besoin :

```rust
pub async fn persist_via_seaorm<M, E, C>(model: M, db: &C) -> Result<M, FrameworkError>
where
    M: ModelTrait<Entity = E> + IntoActiveModel<<E as EntityTrait>::ActiveModel> + Send,
    E: EntityTrait<Model = M>,
    /* … bounds … */
    C: ConnectionTrait,
{
    let mut active = model.into_active_model();
    for pk in <<E as EntityTrait>::PrimaryKey as Iterable>::iter() {
        active.not_set(pk.into_column());
    }
    active.insert(db).await.map_err(/* … */)
}
```

Si vous voulez réellement assigner un id spécifique (test de replay,
restauration d'une fixture par id), contournez le helper et appelez
directement `model.into_active_model().insert(db).await`.

### Persister contre une connexion explicite

`persist_via_seaorm` prend la connexion en argument. Utile quand vous
voulez piloter la persistance contre une connexion qui n'est pas le
`DB::connection()` lié du framework - le plus souvent un handle
`sqlite::memory:` spécifique dans un test d'intégration :

```rust
use suprnova::factory::persist_via_seaorm;

let model = UserFactory::new().make();
let row = persist_via_seaorm(model, db.inner()).await?;
```

### Backends personnalisés hors SeaORM

Parce que l'impl générique cible chaque type `ModelTrait`, vous ne
pouvez pas écrire `impl Persistable for MyOrm::Model` depuis une crate
en aval sans entrer en collision. Pour une persistance personnalisée
non-SeaORM (Redis, Surreal, magasins blob uniquement), enveloppez le
modèle dans un newtype et implémentez `Persistable` sur le wrapper :

```rust
use suprnova::{FrameworkError, Persistable};
use suprnova::async_trait;

pub struct RedisCached<T>(pub T);

#[async_trait]
impl Persistable for RedisCached<MyValue> {
    async fn persist(self) -> Result<Self, FrameworkError> {
        let client = suprnova::App::make::<RedisClient>()
            .ok_or_else(|| FrameworkError::internal("redis client not bound"))?;
        client.set(&self.0.key, &serde_json::to_vec(&self.0)?).await?;
        Ok(self)
    }
}
```

Un `Factory<Model = RedisCached<MyValue>>` obtient alors `create` /
`create_many` gratuitement.

## `make` vs `create` : quand utiliser laquelle

`make` retourne le modèle sans toucher la base de données :

```rust
// Test unitaire pour une fonction pure - pas de BD nécessaire.
let draft = PostFactory::new().with(|p| p.is_public = false).make();
let snippet = my_lib::extract_summary(&draft);
assert!(snippet.len() < 200);
```

`create` persiste et retourne la version post-insertion :

```rust
// Test d'intégration - l'action a besoin d'une ligne réelle.
let post = PostFactory::new().create().await?;
let action = App::resolve::<PublishPostAction>().unwrap();
let published = action.execute(post.id).await?;
assert!(published.is_public);
```

Tournez-vous vers `make` chaque fois que le test ne se soucie pas que
la ligne existe. Tournez-vous vers `create` quand vous allez requêter
la ligne en retour, quand une clé étrangère a besoin d'un id réel, ou
quand vous peuplez des fixtures pour un sous-système qui lit la BD.
Notez que `create_many` persiste séquentiellement - si une insertion
ultérieure échoue, les insertions précédentes ne sont PAS annulées.
`create` / `create_many` passent par l'impl générique `Persistable`,
qui parle directement au `DB::connection()` lié du framework - ils ne
rejoignent **pas** une portée `DB::transaction(...)` ambiante. Si vous
avez besoin d'atomicité à travers un lot d'insertions, redescendez
vers le `Model::create(attrs!{...})` du trait `Model` à l'intérieur de
la closure (ce chemin route à travers le même exécuteur qui honore
`CURRENT_TX`) :

```rust
use suprnova::{DB, Model, attrs};

DB::transaction(|_tx| Box::pin(async move {
    for i in 0..50 {
        User::create(attrs!{
            name: format!("user-{i}"),
            email: format!("user-{i}@example.test"),
        }).await?;
    }
    Ok::<_, suprnova::FrameworkError>(())
})).await?;
```

## Comportement « after-creating »

Suprnova ne fournit pas de callback nommé `after_creating(|m| { … })`.
Deux motifs couvrent les cas d'usage pour lesquels ce callback existe
dans Laravel :

**1. La chaîne - faites le suivi après `create`/`create_many` :**

```rust
let user = UserFactory::new().create().await?;
ProfileFactory::new()
    .with(move |p| p.user_id = user.id)
    .create()
    .await?;
```

C'est le motif canonique quand l'id d'un modèle doit s'écouler dans
une insertion de suivi. `create` retourne la ligne persistée, si bien
que l'id est immédiatement disponible.

**2. Observateurs de modèle - réagir sur le cycle de vie du modèle,
pas sur la fabrique :**

Utilisez les [Observateurs de modèle](eloquent.md#observers) pour
câbler un comportement post-insertion sur le modèle lui-même plutôt
que sur la fabrique. L'observateur se déclenche pour
`User::create(...)`, `UserFactory::new().create()`, et tout autre
chemin de persistance - exactement ce que vous voulez quand le
comportement est « chaque fois que cette ligne arrive, fais X » :

```rust
use suprnova::{FrameworkError, Observer, async_trait, observer};

#[observer(User)]
pub struct AuditUser;

#[async_trait]
impl Observer<User> for AuditUser {
    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user_id = user.id, "user created");
        Ok(())
    }
}
```

Des callbacks propres à la fabrique inviteraient à une divergence
entre les insertions de test et les insertions réelles. Les
observateurs restent cohérents à travers les deux.

## Seeders

Les fabriques produisent des instances ; les seeders les orchestrent.
Un `Seeder` est un type de taille nulle avec un `run` async qui sait
quoi peupler :

```rust
use suprnova::{Factory, FrameworkError, Seeder};
use suprnova::async_trait;

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // Utilisateurs d'abord - les posts référencent des id d'utilisateur dans 1..=50.
        UserFactory::new().count(50).create_many().await?;
        PostFactory::new().count(200).create_many().await?;
        Ok(())
    }
}
```

Enregistrez le seeder dans `bootstrap.rs` pour que la commande
`db:seed` du binaire `console` du projet le connaisse :

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

Exécutez via le binaire `console` du projet (chaque application
scaffoldée en livre un à `src/bin/console.rs`) :

```bash
cargo run --bin console -- db:seed
```

Les seeders s'exécutent dans l'ordre d'enregistrement. L'idempotence
est la responsabilité du seeder - `run` ne prend pas d'instantané et
ne fait pas de rollback, si bien qu'un seeder qui insère sans
condition produit des doublons à la réexécution. Utilisez
`migrate:fresh` suivi de `db:seed` pour repartir de zéro.

## Tout assembler : une fixture de test complète

```rust
use suprnova::{App, describe, test, expect};
use suprnova::events::{EventFacade, assert_dispatched_times};
use suprnova::testing::TestDatabase;
use crate::factories::{PostFactory, UserFactory};
use crate::actions::publish_post::PublishPostAction;

describe!("PublishPostAction", {
    test!("publishes a draft post", async fn(db: TestDatabase) {
        // Arrange - un auteur et un post brouillon qu'il possède.
        let author = UserFactory::new()
            .with(|u| u.active = true)
            .create()
            .await
            .unwrap();

        let draft = PostFactory::new()
            .with(move |p| p.author_id = author.id)
            .with(|p| p.is_public = false)
            .create()
            .await
            .unwrap();

        // Act.
        let action = App::resolve::<PublishPostAction>().unwrap();
        let published = action.execute(draft.id).await.unwrap();

        // Assert.
        expect!(published.is_public).to_equal(true);
        expect!(published.author_id).to_equal(author.id);
    });

    test!("publishing emits exactly one event", async fn(db: TestDatabase) {
        let _guard = EventFacade::fake();
        let post = PostFactory::new().create().await.unwrap();

        App::resolve::<PublishPostAction>().unwrap()
            .execute(post.id).await.unwrap();

        assert_dispatched_times::<crate::events::PostPublished>(1);
    });
});
```

Trois motifs qui valent la mention :

- L'`id` de l'auteur s'écoule dans le post via une closure `move` à
  l'intérieur de `.with(...)`. Les captures sont explicites, ce qui
  garde la relation visible au site d'appel.
- `create().await.unwrap()` est l'idiome de test - le test a le droit
  de paniquer sur un échec de setup parce qu'une fixture cassée est un
  test cassé, pas un mode d'échec propre.
- Les fabriques se composent avec le reste de la surface de test
  (`EventFacade::fake`, `Storage::fake`, `Mail::fake`, …) - aucun des
  fakes ne connaît les fabriques, mais chaque test que vous écrirez les
  utilisera ensemble.

### Pourquoi Suprnova diverge

Les fabriques de Laravel embarquent des variantes nommées
(`->state('admin')`), des séquences à l'exécution
(`->sequence(['name' => 'A'], ['name' => 'B'])`), et un callback
`afterCreating` enregistré sur la fabrique elle-même. Suprnova
abandonne les trois et les remplace par des primitifs à la façon de
Rust :

- **Les variantes sont des méthodes, pas des chaînes.** La détection
  de coquilles à la compilation et l'autocomplétion IDE sont toutes
  deux gratuites ; le seul coût est « vous écrivez `pub fn admin()` au
  lieu de `protected function admin()` », ce qui n'est aucun coût du
  tout.
- **Les séquences sont un primitif séparé.** `Sequence` fait une seule
  chose (compteur atomique) et est réutilisable en dehors de la
  surface fabrique - vous pouvez en glisser une dans un générateur
  d'id de requête, un compteur d'étape de workflow, ou un harnais de
  test sans avoir à expliquer ce qu'elle est.
- **After-creating est câblé sur le modèle, pas sur la fabrique.** Le
  framework a déjà les [Observateurs de modèle](eloquent.md#observers)
  exactement pour cet usage. Ajouter un mécanisme parallèle sur la
  fabrique ferait diverger par construction le comportement en test et
  le comportement en production.

La surface fluide - `count(10)`, `times(10)`, `with`, `prepend`,
`when`, `make`, `create`, `create_many`, `make_one`, `create_one` -
reflète directement celle de Laravel, si bien que la mémoire
musculaire se transpose sans glossaire.

## Suivant

- [Tests](testing.md) - `#[suprnova_test]`, `TestDatabase`, les
  façades fake qui s'associent aux fixtures construites par fabrique.
- [Eloquent](eloquent.md) - la dérivation de modèle, les observateurs,
  le pipeline de cast qui s'exécute quand `create` persiste la sortie
  de votre fabrique.
- [Migrations](migrations.md) - le schéma contre lequel vos fabriques
  ont besoin d'exister ; utilisez `migrate:fresh && db:seed` pour
  repartir de zéro avec des fixtures propres.
- [Base de données](database.md) - `DB::transaction`, le routage
  multi-connexion, les savepoints - ce vers quoi se tourner quand
  `create_many` a besoin d'atomicité.
- [Conteneur de service](container.md) - comment `App::resolve` et
  `App::make` trouvent les types d'action et de service que vos tests
  appellent, aux côtés des fabriques.

# Conteneur de service

Le conteneur est l'endroit où Suprnova stocke les services de votre
application - le pool de connexion DB, le driver de mail, votre
`Arc<MyService>`. Vous liez les valeurs à l'amorçage et les résolvez
dans les handlers et les workers. C'est l'équivalent Suprnova du
conteneur de services Laravel, avec une différence importante : la
recherche est d'abord task-local, donc les tests s'exécutant
simultanément ne voient pas les liaisons les uns des autres.

## Les deux composants

| Type | Rôle |
|---|---|
| `Container` | Le registre sous-jacent : contient les liaisons, les fabriques et les singletons |
| `App` | La façade globale que vous appelez réellement - `App::bind`, `App::get`, etc. |

Vous appelez presque toujours `App::*` plutôt que de construire un
`Container` directement. Le conteneur est la plomberie ; la façade
`App` est l'API.

## Ordre de recherche

Chaque appel `App::get` / `App::make` vérifie **trois couches** dans
l'ordre :

```
        task-local
            │
            ▼  (miss)
       thread-local
            │
            ▼  (miss)
          global
            │
            ▼  (miss)
          None
```

Cela importe car :

- **L'état par requête passe par task-local** - données partagées
  Inertia, flash bag, ID de requête. Chaque requête obtient sa propre
  couche, de manière transparente.
- **Les tests utilisent thread-local** -
  `let _g = TestContainer::fake();` suivi de
  `TestContainer::bind(...)` lie à l'intérieur d'un thread sans
  toucher au conteneur global, donc les tests parallèles ne mélangent
  pas leurs services. La garde vide le conteneur de test lorsqu'elle
  est détruite (`drop`).
- **Les services à l'échelle de l'app passent par global** - lié une
  fois à l'amorçage, résolu partout.

Vous pensez rarement à la couche dans laquelle réside une liaison -
`App::bind` la met où cela a du sens, et `App::get` la trouve partout
où elle réside. Le modèle n'importe que quand quelque chose se
comporte de manière inattendue sous concurrence, et le chapitre
[Tests](testing.md) en a alors le détail.

## Lier une valeur

Cinq façons de mettre quelque chose dans le conteneur, selon ce que
vous avez :

### `App::singleton(value)` - possédé, cloné à la recherche

Pour n'importe quelle valeur `T: Any + Send + Sync + 'static` qui
devrait vivre pour toujours. La contrainte `Clone` est sur le *getter*
(`App::get`), pas sur la liaison - la valeur est stockée une fois dans
un `Arc` et clonée depuis cet `Arc` à chaque `get` :

```rust
use suprnova::App;

App::singleton(MyConfig {
    timeout_secs: 30,
    retries: 3,
});

let cfg = App::get::<MyConfig>().expect("registered at boot");
println!("{}", cfg.timeout_secs);
```

La valeur est stockée une fois ; `App::get::<MyConfig>()` retourne un
clone. Utilisez ceci pour les données simples en forme de config qui
sont peu coûteuses à cloner.

### `App::bind(Arc<T>)` - pour les traits et services partagés

Pour les objets trait ou tout ce que vous voulez placer derrière un
`Arc` :

```rust
use std::sync::Arc;
use suprnova::App;

let store: Arc<dyn KeyValueStore> = Arc::new(RedisStore::connect(url)?);
App::bind(store);

let store = App::make::<dyn KeyValueStore>().expect("bound at boot");
store.put("hello", b"world").await?;
```

`App::make::<T>()` retourne le clone de l'`Arc<T>` (une augmentation
atomique peu coûteuse du compteur de références). Utilisez ceci pour
n'importe quel service partagé entre les threads, en particulier les
objets trait.

### `App::factory(|| { … })` - construit à la demande

Quand la construction de la valeur doit se produire à la première
utilisation (ou à chaque fois) :

```rust
App::factory(|| {
    HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client config is hand-rolled and known-good")
});
```

`App::factory` enregistre une fabrique *de type concret* (`Fn() -> T`)
; `App::bind_factory` enregistre une fabrique *d'objet trait*
(`Fn() -> Arc<T>`). Aucune des deux fermetures ne retourne `Result` -
gérez l'échec de construction à l'intérieur de la fermeture (panique à
l'amorçage, ou construction d'une valeur sentinelle), ou utilisez un
`App::singleton` / `App::bind` classique après avoir construit la
valeur vous-même avec `?`. Les deux invoquent la fermeture en dehors
de tout verrou du conteneur, donc une fabrique qui réentre dans le
conteneur ne provoque pas d'interblocage, et un constructeur coûteux
ne bloque pas les autres liaisons.

### `App::*_if_absent(value)` - enregistrement convivial pour l'ordre d'amorçage

Parfois, un service par défaut est enregistré par une crate de
service, et l'app veut le remplacer uniquement si présent. Les
variantes `_if_absent` vous permettent d'enregistrer une valeur par
défaut qui n'écrasera pas une liaison existante :

```rust
// Dans une crate starter ou de bibliothèque :
App::singleton_if_absent(DefaultMailDriver::new());

// Dans le bootstrap.rs de votre application :
App::singleton(MyCustomMailDriver::new());  // l'emporte car elle s'exécute plus tard
```

`bind_if_absent`, `singleton_if_absent`, et les variantes de fabrique
retournent toutes `bool` - `true` si elles ont réellement inséré,
`false` s'il y avait déjà une liaison.

## Résoudre une valeur

Deux méthodes de lecture, plus leurs homologues retournant `Result` :

```rust
// Clone la valeur liée hors du conteneur :
let cfg: MyConfig = App::get::<MyConfig>().expect("bound at boot");

// Clone l'Arc :
let store: Arc<dyn KeyValueStore> = App::make().expect("bound at boot");

// Idem mais en Result, pour l'idiome `?` dans les chemins faillibles :
let cfg = App::resolve::<MyConfig>()?;
let store = App::resolve_make::<dyn KeyValueStore>()?;
```

`resolve` et `resolve_make` retournent `Result<_, FrameworkError>`
(spécifiquement la variante `ServiceNotFound` quand la recherche
échoue) - utile dans les chemins de handlers où un service manquant
devrait se manifester comme un 500 avec un journal approprié, pas une
panique.

Contrôles d'appartenance (rarement nécessaires) :

```rust
if App::has::<MyConfig>() { … }
if App::has_binding::<dyn KeyValueStore>() { … }
```

## Où la liaison se produit

L'endroit standard est `src/bootstrap.rs` - une fonction qui s'exécute
une fois à l'amorçage :

```rust
use std::sync::Arc;
use suprnova::App;
use crate::services::{MyService, RealEmailGateway};

pub async fn register() {
    // Singletons simples
    App::singleton(MyAppConfig {
        max_uploads_per_user: 100,
    });

    // Services objets trait
    let gateway: Arc<dyn EmailGateway> = Arc::new(RealEmailGateway::new());
    App::bind(gateway);

    // Services paresseux (construits à la première utilisation)
    App::bind_factory::<dyn HttpClient, _>(|| {
        Arc::new(ReqwestClient::with_timeout(30))
    });
}
```

Le nom de fonction `register` correspond à la valeur par défaut du
scaffold (`src/bootstrap.rs::register`) ; le type de retour est `()`,
pas `Result`. Les erreurs de liaison qui se produisent lors de
l'amorçage (par exemple les échecs de connexion du driver) devraient
se propager via le constructeur du driver ou du service, pas depuis
`register` lui-même - voir [Amorçage de l'application](bootstrap.md)
pour le câblage d'amorçage complet.

Le framework appelle également le conteneur lui-même lors de
l'amorçage :

- `App::init()` s'exécute en premier, initialisant le registre
- `App::boot_services()` résout les dépendances d'amorçage (drivers,
  clés de chiffrement, etc.) - vos services voient un framework
  complètement amorcé
- Votre `bootstrap_fn` s'exécute ensuite, elle peut donc compter sur
  la disponibilité des services du framework

Voir [Amorçage de l'application](bootstrap.md) pour l'ordre d'amorçage
complet.

## Données partagées Inertia

Le conteneur est aussi l'endroit où résident les données partagées
Inertia. Trois API de commodité rendent cela explicite :

```rust
use suprnova::App;

// Valeur eager - sérialisée une fois et réutilisée pour chaque réponse Inertia.
App::inertia_share("appName", "Suprnova");

// Valeur lazy - le résolveur s'exécute à chaque réponse. À utiliser pour les
// données par requête qui nécessitent du travail async.
App::inertia_share_lazy("locale", || async {
    Ok::<_, suprnova::FrameworkError>(detect_locale().await)
});

// Pousse une seule entrée flash dans le flash bag de la requête.
App::flash("message", "Saved!");
```

Ceux-ci lisent depuis `Container::inertia()`, qui retourne
`&Arc<InertiaRegistry>` - vous pouvez interagir avec lui directement
si vous avez besoin d'un accès de niveau inférieur. Voir [Inertia /
Frontend](frontend.md) pour savoir comment les données partagées se
retrouvent dans la réponse de page.

## Pourquoi trois couches ?

La cascade task-local → thread-local → global existe pour une seule
raison : **l'isolation sous concurrence**. Trois choses en bénéficient
:

**Isolation par requête.** Le flash bag d'Inertia est lié par requête
via la couche task-local. Deux requêtes simultanées ne voient pas le
flash l'une de l'autre, car leurs conteneurs task-local ne se
chevauchent pas. La liaison s'évapore quand la tâche de la requête se
termine.

**Isolation par test.** Un test qui lie un faux driver de mail ne
devrait pas voir un faux lié par un test voisin.
`TestContainer::fake()` retourne une garde thread-local, et
`TestContainer::bind` / `TestContainer::singleton` acheminent les
écritures dans la portée active. Les tests parallèles restent
hermétiques :

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn one_test_binds_a_fake() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn Mailer>(Arc::new(FakeMailer::new()));

    // … ce test utilise FakeMailer
    // un test voisin qui s'exécute en parallèle ne le voit pas
}
```

Pour les runtimes tokio multi-thread - où la future peut migrer entre
les threads de travail - utilisez plutôt
`TestContainer::scope(async { ... })` ; cela installe une substitution
task-local qui survit à la migration.

**Remplacement à l'amorçage.** Le code d'application peut remplacer
les valeurs par défaut enregistrées par les crates de bibliothèque.
Les variantes `_if_absent` et la recherche en couches se combinent
pour donner aux crates de bibliothèque un enregistrement par défaut
propre, sans combattre les remplacements d'application.

## Motifs courants

### Lier une struct contenant le pool DB

Vous ne faites presque jamais cela directement - le framework lie le
pool DB lui-même. Mais si vous avez votre propre sous-système avec une
ressource partagée coûteuse :

```rust
let pool = MyResourcePool::connect(url).await?;
App::bind(Arc::new(pool));

// plus tard :
let pool = App::resolve_make::<MyResourcePool>()?;
let conn = pool.checkout().await?;
```

`App::make` retourne `Option<Arc<T>>` et s'apparie avec `.expect(...)`
; `App::resolve_make` retourne
`Result<Arc<T>, FrameworkError::ServiceNotFound>` et s'apparie avec
`?` dans le code faillible. Utilisez celui qui correspond à la logique
d'erreur de votre appelant.

### Remplacer une valeur par défaut par un faux dans les tests

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn order_dispatches_email() {
    let fake = Arc::new(FakeEmailGateway::new());
    let fake_for_assert = Arc::clone(&fake);

    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.expect("place_order succeeds");

    assert_eq!(fake_for_assert.sent_count(), 1);
}
```

### Construction paresseuse et coûteuse

```rust
// Construit le modèle d'embedding à la première requête, pas à l'amorçage.
App::bind_factory::<dyn EmbeddingModel, _>(|| {
    Arc::new(
        OnnxEmbedding::load_from_disk("/models/all-mini-lm.onnx")
            .expect("embedding model must load"),
    )
});
```

Pour une construction faillible qui doit faire remonter une erreur
structurée à l'opérateur, construisez la valeur vous-même dans
`bootstrap()` avec `?` et appelez `App::bind(...)` une fois qu'elle
est prête.

## Pourquoi Suprnova diverge

Le conteneur de Laravel a une portée globale unique - les liaisons
sont globales, et l'isolation entre les tests nécessite de la
discipline `setUp` / `tearDown` plus la transaction de base de données
par test du framework. Le modèle request-per-process de PHP rend cela
sûr par accident : un processus neuf par requête signifie que le
conteneur est réinitialisé à chaque fois.

Le modèle de processus de Rust est l'opposé - un processus sert de
nombreuses requêtes simultanées sur plusieurs threads. Un conteneur
uniquement global signifierait qu'un test dans un thread peut voir un
faux lié par un autre, ou qu'une requête pourrait voir les données par
requête d'une autre requête. C'est pourquoi Suprnova a la cascade à
trois couches : task-local pour la portée par requête, thread-local
pour la portée par test, et global pour la portée applicative.

L'API du conteneur est la même que celle de Laravel ; la machinerie de
recherche est différente car le runtime est différent.

## Suivant

- [Amorçage de l'application](bootstrap.md) - où va le code de liaison
- [Configuration](configuration.md) - enregistrement de configuration
  typée aux côtés des services
- [Tests](testing.md) - `TestContainer::fake` et `#[suprnova_test]`
- [Politique de verrouillage](lock-policy.md) - pourquoi la
  récupération de verrous empoisonnés importe dans une application
  soutenue par un conteneur

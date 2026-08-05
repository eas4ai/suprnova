# Flags de fonctionnalité

Le système de flags de fonctionnalité de Suprnova combine des
déclarations `Feature` au moment de la compilation avec des
surcharges à l'exécution persistées dans une table `features`. La
valeur d'un flag au moment de l'évaluation est déterminée, dans
l'ordre, par :

1. Une ligne dotée d'une portée dans la table `features` - `user:42`
   ou `team:staff`.
2. La ligne globale dans la table `features` (portée `""`).
3. Le `default` défini au moment de la compilation dans la
   déclaration `Feature`.

Les bascules effectuées via le CRUD admin se propagent aux
évaluateurs actifs avant que l'appel de mutation ne retourne. Les
flags kill switch se désactivent réellement en temps réel, pas
« dans la prochaine fenêtre de TTL ».

## Démarrage rapide

```rust
// app/src/features.rs - chaque flag que votre app référence vit ici.
use suprnova::features::Feature;

pub const NEW_CHECKOUT_FLOW: Feature<'static> = Feature::new("new-checkout-flow", false);
```

```rust
// app/src/bootstrap.rs - câble la chaîne une seule fois au démarrage.
use std::time::Duration;
use suprnova::features::{bootstrap_database_cached, FeatureMiddleware};

pub async fn register() {
    // ... DB::init, session, etc.

    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature flags wired");

    global_middleware!(FeatureMiddleware::new());
}
```

```rust
// n'importe quel handler - Feature::is_enabled() se résout par rapport au contexte de la requête en cours.
use crate::features::NEW_CHECKOUT_FLOW;

pub async fn index(req: Request) -> Response {
    let banner = if NEW_CHECKOUT_FLOW.is_enabled() {
        Some("Try the new checkout - faster, fewer steps.")
    } else {
        None
    };
    // ...
}
```

```rust
// bascule le flag depuis une route admin ou la CLI :
use suprnova::features::admin;

let actor_id = Auth::id();  // Option<String> - None pour les changements initiés par le système
admin::upsert("new-checkout-flow", "", true, None, actor_id).await?;
//                                  ^   ^                  ^
//                                  |   |                  └ audit : qui a basculé le flag
//                                  |   └ activé
//                                  └ scope_key : "" = global, "user:42" = surcharge de portée
```

Le prochain appel à `NEW_CHECKOUT_FLOW.is_enabled()` observe `true` -
y compris toute entrée d'évaluateur en cache, qui a été invalidée de
façon synchrone à l'intérieur de `admin::upsert`.

## Les pièces

### `Feature<'a>`

La déclaration au moment de la compilation. Porte le nom du flag et
une valeur par défaut en son absence.

```rust
pub const KILL_SWITCH_PAYMENTS: Feature<'static> =
    Feature::new("kill-switch.payments", true);
//                                      ^ défaut : true (paiements activés jusqu'à désactivation)
```

Centraliser chaque déclaration dans `app/src/features.rs` vous donne :

- un unique endroit où grep quand un opérateur demande « quels flags
  existent ? »
- l'unicité du nom du flag garantie à la compilation - une faute de
  frappe au point d'appel ne compile pas
- l'endroit évident où mettre un commentaire de doc expliquant ce que
  le flag contrôle

Appelez `flag.is_enabled()` pour lire par rapport au contexte ambiant
(mis en place par [`FeatureMiddleware`](#featuremiddleware)), ou
`flag.is_enabled_in(Some(&ctx))` pour passer un
[`Context`](https://docs.rs/featureflag/latest/featureflag/context/struct.Context.html)
spécifique.

Les macros `feature!` et `is_enabled!` sont aussi ré-exportées depuis
`suprnova::*` pour les points d'appel qui ne veulent pas importer la
constante :

```rust
use suprnova::is_enabled;

if is_enabled!("new-checkout-flow", false) {
    // ...
}
```

### `DatabaseEvaluator`

Lit la table `features` dans un instantané en mémoire à l'amorçage
et à chaque
[`reload()`](#contrôle-de-flux-propagation-des-flags). Le hot path
(`is_enabled`) est entièrement synchrone - aucune requête BD par
requête, aucun `block_on` à l'intérieur de l'évaluateur.

Ordre de résolution à la recherche, du plus spécifique au moins
spécifique :

1. `user:{id}` - quand le contexte de la requête porte un
   `UserIdField`.
2. `team:{name}` - quand le contexte porte un `TeamField`.
3. `""` - le flag global.
4. `None` - la ligne n'existe pas, le défaut de compilation prend le
   relais.

### `CachedEvaluator`

Met en mémoire les recherches `(feature, user, team)` derrière un
`DashMap` avec un TTL que vous choisissez. Le hot path reste
synchrone ; les entrées sont supprimées de façon synchrone quand
[`admin::upsert`](#crud-admin) écrit un flag.

Un TTL de zéro dégénère en « pas de cache » - chaque appel retombe
sur l'évaluateur interne. Utile pour les apps à faible nombre de
flags qui veulent la plomberie de propagation sans le cache.

### `FeatureMiddleware`

Ouvre un contexte featureflag par requête, peuplé par des extracteurs
définis par l'utilisateur. Par défaut :

- `user_id` - depuis `Auth::id()`.
- `team` - aucun.

Surchargez l'un ou l'autre via le builder :

```rust
let middleware = FeatureMiddleware::new()
    .with_user_id_extractor(|req| {
        // Personnalisé : récupère depuis un en-tête plutôt que la session.
        req.header("X-User-Id").map(String::from)
    })
    .with_team_from_header("X-Team");
// ou : .with_team_extractor(|req| your_custom_team_resolver(req))

global_middleware!(middleware);
```

### CRUD admin

`suprnova::features::admin` est la couche de persistance pour la
table `features`. Utilisez-le depuis des handlers admin, des outils
CLI, des scripts de déploiement - partout où un flag doit basculer :

```rust
use suprnova::features::admin;

// Crée ou met à jour un flag global.
admin::upsert("kill-switch.payments", "", false, Some("ops-2026-05-19".into()), actor_id).await?;
// args : name, scope_key, enabled, description, actor_id

// Surcharge à portée utilisateur (l'emporte sur le global).
admin::upsert("new-checkout-flow", "user:42", true, None, actor_id).await?;

// Supprime complètement une ligne - le flag retombe sur le défaut de compilation.
admin::delete("kill-switch.payments", "", actor_id).await?;

// Lecture pour une table d'UI admin.
let all_flags = admin::list().await?;
let one_row = admin::get("kill-switch.payments", "").await?;
```

Chaque mutation déclenche l'[événement](#événements) correspondant
et appelle
[`features::sync::notify`](#contrôle-de-flux-propagation-des-flags)
pour que tout évaluateur actif lié au conteneur de l'App se
rafraîchisse avant que l'appel ne retourne.

`actor_id: Option<String>` est le pointeur d'audit. Passez l'id
utilisateur de l'opérateur (le même que celui émis par votre couche
d'auth) ; laissez `None` pour les changements initiés par le système
(CLI, migration de déploiement, etc.).

## Contrôle de flux : propagation des flags

Le trait qui fait fonctionner « bascule admin visible immédiatement » :

```rust
#[async_trait]
pub trait FeatureSync: Send + Sync + 'static {
    async fn on_flag_changed(&self, feature: &str, scope_key: &str);
}
```

Les implémenteurs réagissent aux mutations :

- `DatabaseEvaluator::on_flag_changed` appelle `self.reload()` -
  récupère l'instantané complet.
- `CachedEvaluator::on_flag_changed` appelle
  `self.invalidate(feature)` - supprime chaque entrée en cache pour
  ce nom.

La chaîne canonique est un `CompositeFeatureSync`, qui **ordonne les
sources de données avant les caches** - les caches doivent s'invalider
*après* que la source de données s'est rafraîchie, sinon un lecteur
concurrent peut tomber sur le cache vide, retomber sur la source de
données périmée, et repeupler le cache avec l'ancienne valeur.

```rust
let composite = CompositeFeatureSync::new(
    vec![database.clone() as Arc<dyn FeatureSync>], // sources de données d'abord
    vec![cached.clone() as Arc<dyn FeatureSync>],   // caches ensuite
);
App::bind::<dyn FeatureSync>(composite);
```

`features::sync::notify(feature, scope_key)` résout `Arc<dyn
FeatureSync>` depuis le conteneur et attend `on_flag_changed`. Sans
effet quand aucun sync n'est lié - le bon comportement pour les
outils admin hors-process qui ne font qu'écrire en BD et n'ont aucun
évaluateur actif à rafraîchir.

## Helper d'amorçage

`bootstrap_database_cached(ttl)` câble tout en un seul appel :

```rust
let features = bootstrap_database_cached(Duration::from_secs(60))
    .await
    .expect("feature flags wired");

// Optionnel : gardez features.database pour planifier des
// rechargements périodiques ou exposer des vues de diff admin. La
// plupart des apps abandonnent le handle et laissent le
// rafraîchissement piloté par notify faire le travail.
```

Ce qu'il fait :

1. Construit un `DatabaseEvaluator` contre la connexion BD
   principale.
2. L'enveloppe dans un `CachedEvaluator` avec le TTL demandé.
3. Appelle `install_evaluator(cached)` - fixe le défaut featureflag
   global *et* bascule un traceur « installed » possédé par le
   framework, pour que le middleware n'émette pas l'avertissement
   « no evaluator ».
4. Construit un `CompositeFeatureSync` avec le bon ordre
   d'emplacements et le lie dans le conteneur de l'App.

Retourne `BootstrappedFeatures { database, cached }` pour les
appelants qui veulent des handles directs sur l'une ou l'autre
couche.

Si votre topologie n'est pas `Cached(Database)` - un cache adossé à
Redis, une source de sync distante, une chaîne multi-niveaux -
câblez la chaîne manuellement en utilisant les mêmes primitives.
`bootstrap_database_cached` est une commodité, pas un contrat.

## Migrations

Le framework possède le schéma de la table `features` :

```rust
// app/src/migrations/mod.rs
vec![
    // ... les migrations de votre app ...
    Box::new(suprnova::features::migrations::CreateFeaturesTable),
]
```

Schéma :

```sql
features (
    id          BIGINT      PRIMARY KEY AUTO_INCREMENT,
    name        VARCHAR(255) NOT NULL,
    scope_key   VARCHAR(255) NOT NULL DEFAULT '',
    enabled     BOOLEAN     NOT NULL,
    description TEXT,
    updated_by  VARCHAR(255),
    created_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE INDEX (name, scope_key)
)
```

`scope_key` porte le type de portée en ligne (`"user:42"`,
`"team:staff"`, `""` pour global), si bien que le chemin de lecture
reste une simple recherche de chaîne contre un index unique.

## ID utilisateur et équipe

`UserIdField` et `TeamField` sont des extensions typées, rangées dans
le `Context::extensions` de featureflag. Les deux sont typées en
chaîne, si bien que les id utilisateur opaques (UUID / ULID) de
torii et les colonnes numériques `users.id` coexistent derrière la
même forme.

Construire un contexte manuellement (en dehors du middleware) :

```rust
use featureflag::context;
use std::sync::Arc;

let ctx = featureflag::evaluator::with_default(cached.clone(), || {
    // id utilisateur en chaîne - UUID, ULID, tout ce qui est opaque.
    context! { user_id = "01HZK6V3J7Q5G4P8X9N2D1B0M3".to_string(), team = "staff".to_string() }
});

// les id numériques marchent aussi - le framework convertit i64 → String au moment de on_new_context.
let ctx_numeric = featureflag::evaluator::with_default(cached.clone(), || {
    context! { user_id = 42_i64 }
});
```

## Événements

Deux événements se déclenchent depuis le chemin CRUD admin :

```rust
pub struct FeatureUpdated {
    pub name: String,
    pub scope_key: String,
    pub enabled: bool,
    pub actor_id: Option<String>,
}

pub struct FeatureDeleted {
    pub name: String,
    pub scope_key: String,
    pub actor_id: Option<String>,
}
```

Écoutez-les via le dispatcher d'événements du framework pour
alimenter un journal d'audit, une alerte Slack, ou tout autre
pipeline en aval dont vous avez besoin :

```rust
EventFacade::listen::<FeatureUpdated, _>(Arc::new(FlagChangeAuditor)).await;
```

**`is_enabled` ne déclenche pas d'événement sur le chemin de
lecture.** Chaque requête qui vérifie un flag multiplierait le volume
d'événements par le nombre de flags vérifiés - correct pour une
histoire d'audit des mutations, prohibitif pour le traçage du chemin
de lecture. Si votre déploiement a besoin d'un audit échantillonné du
chemin de lecture, superposez un évaluateur personnalisé qui
enregistre dans un canal de journal borné (un stream Redis ou une
queue à fan-out, selon l'échelle).

## Détection d'évaluateur manquant

Si `FeatureMiddleware` est installé mais qu'aucun évaluateur n'a été
enregistré via `install_evaluator` / `bootstrap_database_cached`,
chaque flag renvoie silencieusement son défaut de compilation - une
mauvaise configuration sérieuse à attraper en QA. Le middleware émet
exactement un `tracing::warn!` par processus, à la première requête
qui observe cet état :

```
WARN suprnova::features: FeatureMiddleware is in the stack but no feature-flag evaluator is installed.
     is_enabled!() calls will return compile-time defaults until features::bootstrap_database_cached(...)
     or features::install_evaluator(...) is called during app boot.
```

La bascule utilise un `AtomicBool::swap`, si bien qu'une tempête de
requêtes concurrentes à l'amorçage se sérialise en une seule émission
d'avertissement, pas une par worker.

## Tests

Deux patterns, selon ce que vous vérifiez.

### Tester unitairement un Feature en isolation

Utilisez `featureflag::evaluator::with_default` pour donner une
portée à un évaluateur de substitution à l'intérieur d'une closure
synchrone :

```rust
#[test]
fn flag_enabled_returns_new_path() {
    use featureflag::evaluator::with_default;
    use suprnova::features::DatabaseEvaluator;

    let flagger = Arc::new(tokio_test::block_on(async {
        let e = DatabaseEvaluator::new_in_memory().await.unwrap();
        e.set_flag("new-checkout-flow", "", true).await.unwrap();
        e
    }));

    with_default(flagger, || {
        assert!(crate::features::NEW_CHECKOUT_FLOW.is_enabled());
    });
}
```

`DatabaseEvaluator::new_in_memory()` est un helper réservé aux tests,
qui amorce son propre SQLite + exécute `CreateFeaturesTable` pour que
le test reste hermétique. Ne l'utilisez pas dans les chemins de
production.

### Tester en intégration la propagation de bout en bout

Utilisez `TestDatabase::fresh::<TestMigrator>()` pour la BD et
`TestContainer::bind` (PAS `App::bind`) pour le FeatureSync - sans
quoi des tests parallèles dans le même process écraseraient
mutuellement leur liaison via le conteneur global :

```rust
#[tokio::test]
async fn admin_upsert_propagates_to_cached_chain() {
    use std::sync::Arc;
    use std::time::Duration;
    use suprnova::features::sync::FeatureSync;
    use suprnova::features::{admin, CachedEvaluator, CompositeFeatureSync, DatabaseEvaluator};
    use suprnova::features::migrations::CreateFeaturesTable;
    use suprnova::testing::{TestContainer, TestDatabase};

    struct TestMigrator;
    impl sea_orm_migration::MigratorTrait for TestMigrator {
        fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
            vec![Box::new(CreateFeaturesTable)]
        }
    }

    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();

    let database = Arc::new(DatabaseEvaluator::new().await.unwrap());
    let cached = Arc::new(CachedEvaluator::new(
        database.clone() as Arc<dyn featureflag::evaluator::Evaluator + Send + Sync>,
        Duration::from_secs(60),
    ));
    let composite = Arc::new(CompositeFeatureSync::new(
        vec![database.clone() as Arc<dyn FeatureSync>],
        vec![cached.clone() as Arc<dyn FeatureSync>],
    ));
    TestContainer::bind::<dyn FeatureSync>(composite);

    let ctx = featureflag::evaluator::with_default(cached.clone(), || {
        featureflag::context! { user_id = "user-42".to_string() }
    });

    assert_eq!(cached.is_enabled("new-feature", &ctx), None);
    admin::upsert("new-feature", "", true, None, None).await.unwrap();
    assert_eq!(cached.is_enabled("new-feature", &ctx), Some(true)); // se propage instantanément
}
```

Voir `framework/tests/features.rs` pour l'ensemble complet des tests
de composition.

### Pourquoi Suprnova diverge

Laravel Pennant résout chaque flag contre la base de données à la
demande (avec une mémoïsation optionnelle au niveau du driver, par
requête). Le modèle PHP une-requête-par-process rend un accès BD par
requête peu coûteux, parce que la connexion est dédiée et meurt avec
la requête.

Le modèle de process de Suprnova est l'opposé - un unique binaire
longue durée qui sert des milliers de requêtes concurrentes. Un
accès BD par requête à chaque vérification de flag multiplierait la
charge du pool de connexions par le nombre de vérifications de
flags. La chaîne à deux couches (instantané `DatabaseEvaluator` +
TTL `CachedEvaluator`) est la réponse idiomatique en Rust : le hot
path est entièrement synchrone contre des données en mémoire, et le
trait `FeatureSync` donne aux changements initiés par l'opérateur
une propagation en moins d'une seconde, sans rechargement par
polling. La forme est la même que Pennant - définir un flag, le
vérifier dans un handler, le surcharger depuis une route admin. La
plomberie est différente parce que le runtime est différent.

## Notes de conception

- **Pourquoi un évaluateur synchrone plutôt qu'asynchrone ?**
  `is_enabled` de featureflag est le hot path. Un évaluateur
  asynchrone forcerait un `block_on` (sujet aux deadlocks) ou
  pousserait chaque handler à faire un `.await` sur les lectures de
  flags (désastre ergonomique). Le framework fait le pont entre sync
  et async via un instantané en mémoire, rafraîchi de façon
  asynchrone par `FeatureSync`.

- **Pourquoi un trait `FeatureSync` séparé plutôt que d'étendre
  `Evaluator` ?** `Evaluator` de featureflag est possédé par une
  crate en amont ; nous ne pouvons pas lui ajouter de méthodes.
  `FeatureSync` est un trait frère que les apps implémentent sur les
  mêmes types concrets. L'objet trait est lié séparément dans le
  conteneur de l'App, si bien qu'un process peut superposer
  plusieurs évaluateurs tout en routant correctement les
  notifications.

- **Pourquoi `set_flag` est-il `pub` sur `DatabaseEvaluator` ?**
  Commodité pour les tests. Le chemin d'écriture en production est
  `admin::upsert` ; `set_flag` existe pour que les tests puissent
  ensemencer des flags sans avoir à mettre en place un écouteur
  `EventFacade`. Les deux chemins appellent `features::sync::notify`,
  si bien que le contrat de propagation tient dans les deux cas.

- **Pourquoi aucun événement `FeatureRetrieved` ?** Le volume. Un
  handler qui vérifie dix flags par requête déclenche dix événements
  par requête - pour un service à 1k req/s, cela fait 36M
  événements/heure, bien au-delà du rapport signal/bruit de
  n'importe quel pipeline d'audit. L'audit du chemin de mutation
  (`FeatureUpdated` / `FeatureDeleted`) est ce qui est livré ;
  l'échantillonnage du chemin de lecture, si besoin, se superpose
  via un wrapper d'évaluateur personnalisé.

## Suivant

- [Middleware](middleware.md) - `FeatureMiddleware` se place après
  `SessionMiddleware` ; ce chapitre couvre l'ordre et la pile globale
- [Événements](events.md) - écoutez `FeatureUpdated` /
  `FeatureDeleted` pour alimenter des journaux d'audit, des alertes
  Slack, ou des pipelines en aval
- [Conteneur de service](container.md) - comment la liaison `dyn
  FeatureSync` est résolue, et pourquoi `TestContainer::bind` existe
  pour les tests parallèles
- [Tests](testing.md) - les patterns `TestDatabase::fresh::<M>()` et
  `TestContainer::fake` sur lesquels ce chapitre s'appuie
- [Authentification](authentication.md) - `Auth::id()` est
  l'extracteur d'id utilisateur par défaut et alimente `actor_id`
  pour les mutations admin

Externe : la [doc de la crate
featureflag](https://docs.rs/featureflag) couvre les primitives
`Evaluator`, `Context`, et `Feature` en amont.
`suprnova::features::admin` est la façade CRUD complète - `cargo doc
--open -p suprnova` pour naviguer.

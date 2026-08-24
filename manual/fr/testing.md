# Tests

C'est le chapitre central pour la surface de test de Suprnova - les
macros, la base de données in-process, les fakes du conteneur, et les
helpers de clé de chiffrement que vos binaires de test utilisent. Les
chapitres en profondeur vivent à ses côtés : [Tests HTTP](http-tests.md)
pour les routes + le middleware, [Tests de base de données](database-testing.md)
pour tout ce qui touche à `TestDatabase`, [Mocking et doublures](mocking.md)
pour les sept surfaces externes (Mail, Notify, Queue, Bus, Events,
Storage, client HTTP). Lisez celui-ci pour apprendre ce qu'il y a dans
la boîte ; passez à un chapitre voisin quand vous avez besoin de la
forme longue.

## Les pièces

| Pièce | Rôle |
|---|---|
| `#[tokio::test]` + `TestDatabase::fresh::<Migrator>()` | Le cheval de bataille par défaut - chaque vrai test du framework utilise ceci |
| `#[suprnova_test]` | Sucre en macro d'attribut - exécute `App::init()` + `App::boot_services()` et construit une `TestDatabase` pour vous |
| `describe!` + `test!` | Macros de regroupement à la Jest, associées à `expect!` pour une sortie d'échec nommée |
| `expect!` | Macro d'assertion fluide avec des matchers typés (égalité, option, résultat, chaîne, vec, ordre) |
| `TestDatabase::fresh` / `sqlite_memory` | SQLite en mémoire + enregistrement dans le conteneur, avec ou sans votre migrator |
| `TestContainer::fake` / `scope` / `spawn` | Substitutions DI thread-local ou task-local, hermétiques entre les tests parallèles |
| `install_test_encryption_key[ring]` | `APP_KEY` déterministe pour les tests qui touchent des casts chiffrés ou des charges utiles signées |
| Helpers `fake()` par surface | Mail, Notify, Queue, Bus, Events, Storage, HTTP - voir [Mocking](mocking.md) |
| `TestResponse` | Assertions fluides sur le triplet `(status, headers, body)` d'un test HTTP - voir [Tests HTTP](http-tests.md#assertions-de-réponse-fluides-avec-testresponse) |
| `AssertableInertia` | Assertions fluides sur un objet de page Inertia - voir [Tests HTTP](http-tests.md#tester-les-réponses-inertia) |

Vous n'irez pas chercher tout ça dans un seul test. Un test d'action
typique utilise les trois premiers ; un test riche en DI ajoute
`TestContainer` ; un test HTTP échange `TestDatabase` contre le
pipeline `handle_request` ; un test de paiement installe le porte-clés
de chiffrement.

## Le cheval de bataille par défaut

Chaque vrai test du framework ressemble à ceci :

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn create_user_persists_it() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);

    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`TestDatabase::fresh::<M>()` ouvre une connexion `sqlite::memory:`
neuve, exécute votre migrator de bout en bout, et enregistre la
connexion dans le conteneur de test. Tout code qui appelle ensuite
`DB::connection()` ou `App::resolve::<DbConnection>()` s'y résout - y
compris le query builder `#[suprnova::model]` et tout service que vous
avez résolu depuis le conteneur. Quand la `TestDatabase` est
abandonnée, l'enregistrement part avec elle.

La macro `test_database!()` est du sucre en une ligne pour le cas
`crate::migrations::Migrator` :

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();         // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}
```

Pour les tests qui veulent un contrôle précis de la forme des colonnes
(aller-retours de cast, surface SQL du query builder), utilisez
`TestDatabase::sqlite_memory()` - même câblage de conteneur, sans
migrator. Le DDL est le vôtre. Voir [Tests de base de données](database-testing.md)
pour le catalogue complet plus les helpers `execute_unprepared` /
`fetch_one` / `fetch_all`.

## `#[suprnova_test]` - quand vous voulez le sucre

`#[suprnova_test]` est une macro d'attribut qui enveloppe
`#[tokio::test]`, appelle `App::init()` + `App::boot_services()` pour
que les types `#[injectable]` se résolvent, et lie une `TestDatabase`
neuve. C'est du sucre optionnel par-dessus la forme explicite
ci-dessus, utile quand un test résout des services enregistrés dans le
conteneur :

```rust
use suprnova::suprnova_test;
use suprnova::{App, testing::TestDatabase};

#[suprnova_test]
async fn create_user_via_action(db: TestDatabase) {
    let action = App::resolve::<CreateUserAction>().unwrap();
    let user = action.execute("test@example.com").await.unwrap();

    assert_eq!(user.email, "test@example.com");
    assert!(user.id > 0);
}
```

Si la fonction prend un paramètre `TestDatabase` (par son nom), la
macro lie la base de données neuve à ce nom. Si elle ne le fait pas, la
base de données est quand même construite et enregistrée (donc
`DB::connection()` fonctionne) - elle n'est simplement pas liée à une
locale.

Redéfinissez le migrator avec la clé `migrator = …` :

```rust
#[suprnova_test(migrator = my_crate::tests::IsolatedMigrator)]
async fn create_user_with_isolated_schema(db: TestDatabase) {
    // ...
}
```

Les clés inconnues sont une erreur de compilation (une faute de frappe
`migrtor = …` ne gardera pas silencieusement le migrator par défaut).

## `describe!` et `test!` - quand le regroupement aide

Pour les fichiers de test où la même action a de nombreux cas, la
paire `describe!` + `test!` à la Jest vous donne un regroupement
imbriqué et une sortie d'échec nommée :

```rust
use suprnova::{App, describe, test, expect, testing::TestDatabase};
use crate::migrations::Migrator;

describe!("ListTodosAction", {
    test!("returns empty list when no todos exist", async fn(db: TestDatabase) {
        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_be_empty();
    });

    test!("returns all todos", async fn(db: TestDatabase) {
        Todo::create(attrs! { title: "Buy bread" }).await.unwrap();
        Todo::create(attrs! { title: "Walk dog" }).await.unwrap();

        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_have_length(2);
    });

    describe!("with pagination", {
        test!("returns first page", async fn(db: TestDatabase) {
            // les groupes imbriqués se composent
        });
    });
});
```

`test!` accepte trois formes :

```rust
// Test async avec un paramètre TestDatabase
test!("creates a user", async fn(db: TestDatabase) { … });

// Test async sans base de données
test!("calculates the right sum", async fn() { … });

// Test synchrone
test!("adds numbers", fn() { … });
```

Le wrapper de test nommé fait transiter le nom du test à travers la
machinerie `expect!` pour qu'un échec fasse surface ainsi :

```text
Test: "returns all todos"
  at src/actions/todo_action.rs:25

  expect!(actual).to_equal(expected)

  Expected: 2
  Received: 0
```

Sans `describe!`/`test!` vous obtenez la sortie `panic!` standard.
Avec eux, l'emplacement et le nom de test lisible par un humain
ouvrent le message.

## `expect!` - le catalogue de matchers

`expect!(value)` retourne un wrapper `Expect<T>`. Les matchers sont
typés sur `T` - appeler `to_be_some()` sur un `String` est une erreur
de compilation, pas une panique à l'exécution.

```rust
use suprnova::expect;

// Égalité (T: Debug + PartialEq)
expect!(actual).to_equal(expected);
expect!(actual).to_not_equal(unexpected);

// Booléen
expect!(condition).to_be_true();
expect!(condition).to_be_false();

// Option<T>
expect!(option).to_be_some();
expect!(option).to_be_none();
expect!(option).to_contain_value(5);     // vérification Some(5)

// Result<T, E>
expect!(result).to_be_ok();
expect!(result).to_be_err();

// String / &str
expect!(s).to_contain("substring");
expect!(s).to_start_with("prefix");
expect!(s).to_end_with("suffix");
expect!(s).to_have_length(10);
expect!(s).to_be_empty();

// Vec<T>
expect!(v).to_have_length(3);
expect!(v).to_contain(&item);
expect!(v).to_be_empty();

// Ordre (T: Debug + PartialOrd)
expect!(10).to_be_greater_than(5);
expect!(5).to_be_less_than(10);
expect!(10).to_be_greater_than_or_equal(10);
expect!(5).to_be_less_than_or_equal(5);
```

Vous pouvez utiliser `expect!` hors de `test!` - le fichier/la ligne
dans le message d'échec vient de `concat!(file!(), ":", line!())`.
L'en-tête de test nommé est la seule chose que la macro n'ajoute pas
d'elle-même.

## `TestContainer` - des fakes DI qui ne fuient pas

Le chapitre sur le conteneur couvre la [recherche à trois
couches](container.md) en détail. Pour les tests, les deux points
d'entrée sont `TestContainer::fake()` (thread-local) et
`TestContainer::scope(…).await` (task-local).

### Thread-local, le cas courant

`TestContainer::fake()` retourne une garde. Jusqu'à ce que la garde
soit abandonnée, les écritures `TestContainer::singleton` / `bind` /
`factory` atterrissent sur la couche de substitution thread-local et
masquent le conteneur global :

```rust
use std::sync::Arc;
use suprnova::App;
use suprnova::testing::TestContainer;

#[tokio::test]
async fn order_dispatches_email() {
    let _guard = TestContainer::fake();

    let fake = Arc::new(FakeEmailGateway::new());
    let probe = Arc::clone(&fake);
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.unwrap();

    assert_eq!(probe.sent_count(), 1);
}
```

`TestDatabase::fresh` / `sqlite_memory` installent leur propre garde
`TestContainer::fake` en interne - vous ne les empilez pas à moins de
tester le registre lui-même.

### Task-local, pour les runtimes `multi_thread`

La couche thread-local est posée sur le thread OS qui a appelé
`fake()`. Un runtime tokio `multi_thread` peut faire migrer votre
future vers un autre thread de travail à travers un `.await`, et la
substitution disparaît silencieusement. `TestContainer::scope` résout
cela en liant la substitution à la future plutôt qu'au thread :

```rust
use suprnova::testing::TestContainer;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_worker_safe() {
    TestContainer::scope(async {
        TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
        do_async_work_that_may_hop_workers().await;
    })
    .await;
}
```

Les sous-tâches spawnées par `tokio::spawn` n'héritent pas des
task-locals de tokio ; utilisez plutôt `TestContainer::spawn` - il
capture le conteneur de la portée courante et le réinstalle à
l'intérieur de la future spawnée :

```rust
TestContainer::scope(async {
    TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
    let h = TestContainer::spawn(async {
        App::make::<dyn HttpClient>().unwrap()  // voit le fake
    });
    let _client = h.await.unwrap();
})
.await;
```

### Pourquoi il y a un compteur de références `FAKE_GUARDS`

Le conteneur thread-local est propre à chaque test, mais Suprnova a
aussi un `ConnectionRegistry` global au processus, indexé par nom
(`__read_replica__`, des labels de connexion personnalisés), qui
survit à une réinitialisation thread-local. Une impl `Drop` naïve
appellerait `ConnectionRegistry::clear()` chaque fois que *n'importe
quelle* `TestContainerGuard` disparaît - effaçant la connexion nommée
d'un autre test concurrent en plein milieu de son exécution.

Le correctif est un `AtomicUsize` (`FAKE_GUARDS`) à l'échelle du
processus. `fake()` l'incrémente ; `drop` le décrémente ; seule la
transition de retour à zéro efface le registre nommé. Deux tests
parallèles utilisant `__read_replica__` sont sûrs : quelle que soit la
garde qui se dépose en dernier, c'est elle qui possède l'effacement.

Vous n'appelez pas cela depuis un test - ça s'exécute depuis le `Drop`
de `TestContainerGuard`. Vous n'avez besoin de savoir que c'est là que
si vous déboguez un symptôme du type « connexion nommée disparue en
plein test », ce qui signifie habituellement qu'un test voisin a
oublié d'attendre que sa propre garde se dépose en premier.

## Helpers de test pour la clé de chiffrement

Les tests qui exercent des casts chiffrés (`casts = { secret =
AsEncrypted }` sur un `#[model(...)]`), des charges utiles signées, ou
le repli sur la clé précédente du porte-clés ont besoin d'une
`APP_KEY` installée en cours de processus. Le framework livre deux
helpers réservés aux tests sous la feature `testing` :

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn cast_roundtrip() {
    install_test_encryption_key();   // idempotent ; clé déterministe de 32 octets nuls
    let db = TestDatabase::sqlite_memory().await.unwrap();
    // … chiffrer + relire …
}
```

`install_test_encryption_key` est idempotente - la façade `Crypt`
sous-jacente est adossée à un `OnceLock`, donc le second appel est
sans effet. La plupart des binaires de test de cast l'appellent depuis
chaque test qui touche un cast chiffré ; le premier gagne, les autres
sont gratuits.

Pour les tests de rotation (écritures sous l'ancienne clé, lectures
sous la nouvelle), utilisez la variante porte-clés :

```rust
use suprnova::crypto::EncryptionKey;
use suprnova::testing::install_test_encryption_keyring;

let new = EncryptionKey::from_base64("...").unwrap();
let old = EncryptionKey::from_base64("...").unwrap();
let installed = install_test_encryption_keyring(new, vec![old]);
assert!(installed, "first install wins");
```

Le helper de porte-clés retourne `true` seulement si l'appel a
réellement installé le porte-clés (le `OnceLock` était vide). Pour
frapper du texte chiffré sous une clé arbitraire pour un test de
rotation, utilisez `suprnova::crypto::_test_encrypt_with` plutôt que
d'installer deux fois.

Les deux helpers sont `#[doc(hidden)]` au niveau de la couche crypto
et réexportés sous le module `testing` - ils sont réservés aux tests
et contournent le chemin de validation `APP_KEY` de production.

## La feature `testing` et les builds de production

`suprnova` expose ses helpers de test (`Storage::fake()`,
`TestContainer`, `TestDatabase`, les hooks de rotation
cryptographique comme `_test_install_key`) derrière une feature
Cargo nommée `testing`. La feature fait partie de l'ensemble par
défaut, si bien que les suites de tests consommatrices les
obtiennent gratuitement :

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }

[dev-dependencies]
# `testing` est activée transitivement par la dépendance ci-dessus - rien de plus.
```

Les hooks sont `#[doc(hidden)]` et préfixés par `_test_`, donc ils
ne sont pas atteignables depuis du code applicatif idiomatique
même quand la feature est activée. Le garde-fou porteur est
`Server::from_config` : il valide `APP_KEY` à **chaque**
démarrage, pas seulement quand le trousseau n'est pas initialisé.
Une clé de test préinstallée ne peut pas contourner cette
vérification - le démarrage échoue immédiatement si `APP_KEY` est
absente ou malformée, que quelque chose in-process ait préinstallé
une clé ou non.

Si vous préférez que les helpers ne soient pas du tout liés à
votre artefact de production (défense en profondeur), dépendez de
`suprnova` avec les features par défaut désactivées et n'activez
que ce que vous livrez :

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4", default-features = false, features = ["..."] }

[dev-dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4", features = ["testing", "..."] }
```

C'est un resserrage, pas un correctif - la validation au démarrage
ferme l'exploitation réelle quelle que soit la posture que vous
choisissez.

### Pourquoi Suprnova diverge

Le harnais de test PHP de Laravel obtient l'isolation des tests
parallèles presque gratuitement, parce que le runtime est
mono-thread par requête et que les tests forkent un nouveau
processus par fichier. Le binaire de test Suprnova est un seul
processus exécutant de nombreux `#[tokio::test]` en parallèle sur
un ou plusieurs threads de travail. Un unique conteneur global
signifierait que le fake d'un test fuit dans la recherche du test
suivant dès l'instant où les deux se chevauchent sur un thread de
travail.

C'est pourquoi `TestContainer` a les deux variantes - thread-local
pour le cas courant `current_thread`, task-local pour
`multi_thread`. Le nettoyage à comptage de références
`FAKE_GUARDS` sur le `ConnectionRegistry` global au processus
existe pour la même raison : un état partagé qui ne peut pas être
rendu propre à chaque test doit au moins savoir qu'il ne faut pas
s'effacer tant qu'un autre test s'appuie encore dessus.

Le catalogue de matchers (`expect!`) est typé parce que Rust le
permet. Le `expect(x).toBeSome()` de Jest ne sait qu'à l'exécution
si `x` est une `Option` ; l'`Expect<T>` de Suprnova le sait à la
compilation, donc un mauvais matcher est une erreur de
compilation, pas un test instable.

## Où réside chaque élément

| Élément | Source |
|---|---|
| Macro d'attribut `#[suprnova_test]` | `suprnova-macros/src/suprnova_test.rs` |
| Proc-macros `describe!` / `test!` | `suprnova-macros/src/describe.rs`, `test_macro.rs` |
| Macro `expect!` + matchers `Expect<T>` | `framework/src/lib.rs` (macro), `framework/src/testing/expect.rs` (impls) |
| `TestDatabase::fresh` / `sqlite_memory` / helpers | `framework/src/database/testing.rs` |
| Macro `test_database!` | `framework/src/database/testing.rs` |
| `TestContainer` + `TestContainerGuard` + `FAKE_GUARDS` | `framework/src/container/testing.rs` |
| `install_test_encryption_key[ring]` | `framework/src/testing/mod.rs` |
| Fakes par surface (Mail, Notify, Queue, Bus, Events, Storage, HTTP) | sous-modules `testing` par domaine - voir [Mocking](mocking.md) |
| `TestResponse` | `framework/src/testing/response.rs` |
| `AssertableInertia`, `ReloadRequest` | `framework/src/testing/inertia.rs` |

## Exécuter les tests

Les invocations cargo standard s'appliquent :

```bash
# Tout l'espace de travail
cargo test --workspace

# Une crate
cargo test -p suprnova

# Un test par nom (correspondance de sous-chaîne)
cargo test create_user_persists_it

# Avec la sortie de println! et dbg!
cargo test -- --nocapture
```

Suprnova ne livre pas son propre exécuteur de tests ; le framework
s'intègre à celui de cargo. Les tests de base de données s'exécutent
en parallèle par défaut - le conteneur thread-local et le SQLite en
mémoire par test sont conçus exactement pour cela.

## Suivant

- [Tests HTTP](http-tests.md) - piloter le pipeline de requête complet
  à travers `handle_request`
- [Tests de base de données](database-testing.md) - `TestDatabase`,
  les fabriques dans les tests, les seeders dans les tests, les tests
  de base de données sûrs en parallèle
- [Mocking et doublures](mocking.md) - les sept fakes de surface
  externe et les motifs qu'ils partagent
- [Conteneur de service](container.md) - la recherche à trois couches
  que `TestContainer` redéfinit
- [Modèle d'erreur](error-model.md) - les formes de `FrameworkError`
  sur lesquelles vous porterez vos assertions

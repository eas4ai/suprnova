# Cache

Suprnova livre une façade `Cache` à la Laravel, adossée à l'un de deux
drivers - en mémoire ou Redis - choisi explicitement à l'amorçage via
`CACHE_DRIVER`. La façade est une couche mince par-dessus un trait
`CacheStore`, si bien que des backends personnalisés se branchent de la
même façon que les backends intégrés.

## La façade

```rust
use suprnova::Cache;
use std::time::Duration;

Cache::put("user:1", &user, Some(Duration::from_secs(3600))).await?;

let cached: Option<User> = Cache::get("user:1").await?;

if Cache::has("user:1").await? {
    // hit
}

Cache::forget("user:1").await?;
```

Chaque méthode sérialise via `serde_json` à la frontière de la façade,
si bien que tout `T: Serialize + DeserializeOwned` fait l'aller-retour.
Le trait sous la façade (`CacheStore`) ne voit que des chaînes JSON
opaques.

## Amorçage

Le cache est lié durant l'étape d'amorçage des drivers de
`Server::run()` (voir [Cycle de vie des requêtes](lifecycle.md)).
`Cache::bootstrap` lit la `CacheConfig` configurée (ou en construit une
depuis l'env) et dispatche selon `CacheConfig::driver` :

- `Memory` - lie un `InMemoryCache` avec le préfixe configuré et le TTL
  par défaut. Réussit toujours.
- `Redis` - se connecte à `REDIS_URL` et lie le `RedisCache` résultant.
  **Échoue fermé** si l'URL est injoignable. Il n'y a pas de repli
  silencieux vers la mémoire.

Les workers (`queue:work`, `schedule:run`, `workflow:work`) passent par
le même amorçage, si bien qu'un job qui utilise `Cache::get` voit le
même backend que le handler HTTP.

### Pourquoi Suprnova diverge

La config `cache.php` de Laravel choisit un magasin par défaut, et
Laravel bascule silencieusement vers `array` (en process) quand un
backend mal configuré échoue dans certains chemins de code. C'est une
valeur par défaut productive pour `php artisan tinker` et un footgun en
production - un seul miss Redis change silencieusement les garanties de
chaque vidage de tag et de chaque acquisition de verrou dans l'app.

Suprnova choisit la valeur par défaut opposée. `CACHE_DRIVER=memory` est
explicite (et la valeur par défaut pour `cargo run`), et
`CACHE_DRIVER=redis` contre un Redis injoignable retourne une erreur
depuis `Server::from_config`. Le binaire quitte avec un code non nul et
un message de remédiation ; supervisord/systemd voit un échec d'amorçage
au lieu d'une app à moitié fonctionnelle.

## Configuration

| Env | Signification | Défaut |
|---|---|---|
| `CACHE_DRIVER` | `memory` ou `redis` | `memory` |
| `REDIS_URL` | URL Redis (consultée seulement quand `driver=redis`) | `redis://127.0.0.1:6379` |
| `REDIS_PREFIX` | Préfixe de clé appliqué à chaque opération sur le magasin | `suprnova_cache:` |
| `CACHE_DEFAULT_TTL` | TTL par défaut en secondes pour `Cache::put(None)` ; `0` signifie aucun défaut | `3600` |

`CACHE_DRIVER` non défini s'analyse en `Memory` ; toute autre valeur
(insensible à la casse, avec espaces retirés) qui n'est ni `memory`,
`in-memory`, `inmemory`, ni `redis` retourne une erreur à l'amorçage.

Vous pouvez aussi construire la config de façon programmatique quand
vous ne voulez pas de l'analyse d'env :

```rust
use suprnova::{Config, CacheConfig, cache::CacheDriver};

Config::register(
    CacheConfig::builder()
        .driver(CacheDriver::Redis)
        .url("redis://cache.internal:6379")
        .prefix("myapp:")
        .default_ttl(7200)
        .build(),
);
```

`CacheConfigBuilder::build` est déterministe - les champs non définis
retombent sur `CacheConfig::default()` plutôt que de relire l'env.

### Le contrat `forever` tient à travers les backends

`Cache::forever` et `Cache::remember_forever` contournent entièrement
`CACHE_DEFAULT_TTL` ; la valeur n'expire jamais, quel que soit le défaut
configuré. `Cache::put(key, value, None)` applique bien le défaut -
c'est tout l'intérêt d'en avoir un.

La résolution du TTL par défaut se produit au niveau de la façade. Les
deux backends `CacheStore` honorent `None` littéralement à la frontière
du magasin (pas d'expiration), ce qui explique pourquoi `forever`
signifie vraiment « pour toujours », aussi bien en mémoire que sur
Redis.

## Lectures, écritures, suppressions

```rust
use suprnova::Cache;
use std::time::Duration;

// Écrire avec un TTL explicite
Cache::put("session:42", &session, Some(Duration::from_secs(1800))).await?;

// Écrire pour toujours - contourne CACHE_DEFAULT_TTL
Cache::forever("config:features", &features).await?;

// Lire (None en cas de miss ou d'expiration)
let session: Option<Session> = Cache::get("session:42").await?;

// Existence - true signifie présent et non expiré
if Cache::has("session:42").await? { /* … */ }

// Négation à l'orthographe Laravel
if Cache::missing("session:42").await? { /* à réchauffer */ }

// Lire-et-supprimer en un seul appel
let one_shot: Option<String> = Cache::pull("notice:welcome:42").await?;

// Retourne true si la clé existait et a été retirée
Cache::forget("session:42").await?;

// Tout vider (limité au préfixe sur les deux backends)
Cache::flush().await?;
```

`Cache::pull` n'est **pas** atomique - c'est un `get` suivi d'un
`forget`, même forme que le `Repository::pull` de Laravel. Pour un
retrait atomique, utilisez `Cache::lock` (voir ci-dessous).

### Rafraîchir un TTL sans réécrire

```rust
let refreshed = Cache::touch("session:42", Duration::from_secs(1800)).await?;
```

`touch` retourne `true` si la clé existait et que le TTL a été prolongé,
`false` sinon. La valeur stockée n'est pas touchée.

## Add - écrire si absent (atomique)

```rust
let won = Cache::add(
    "daily:winner",
    &user_id,
    Some(Duration::from_secs(86_400)),
).await?;
if won {
    send_winner_email(user_id).await?;
}
```

`Cache::add` n'écrit que si la clé est vide (ou a expiré). Retourne
`true` en cas d'écriture, `false` en cas de contention. **Atomique** sur
les deux backends intégrés :

- `InMemoryCache` détient un verrou d'écriture à travers la vérification
  d'existence + l'insertion
- `RedisCache` utilise `SET key value NX EX ttl` (ou `NX` sans `EX`)

Les implémentations `CacheStore` personnalisées qui ne redéfinissent pas
`add_raw` retombent sur un check-then-put non atomique, à l'image du
repli du `Repository::add` de Laravel pour les magasins sans `add`
natif.

## Remember - obtenir ou calculer

```rust
let user = Cache::remember(
    "user:1",
    Some(Duration::from_secs(3600)),
    || async { User::find(1).await },
).await?;

let cfg = Cache::remember_forever("config:app", || async {
    load_config_from_db().await
}).await?;
```

`remember` n'appelle votre closure qu'en cas de miss, puis stocke le
résultat. La closure retourne `Result<T, FrameworkError>`, si bien que
les échecs de domaine remontent via `?` plutôt que de corrompre le
cache.

`Cache::sear(key, default)` est l'alias à l'orthographe Laravel de
`remember_forever`. Même corps, même sémantique - livré sous les deux
noms pour que le code migré se lise de la même façon.

### Remember n'est PAS à l'abri des ruées

`remember` est une paire `get`-puis-`put` non atomique. N misses
concurrents sur la même clé froide exécutent la closure N fois et
écrivent N résultats. Cela correspond exactement au
`Repository::remember` de Laravel, et c'est très bien pour le cas
courant (la closure est idempotente, les écritures sont identiques).

Ce n'est pas bien quand :

- La closure est coûteuse (1 s ou plus à calculer, ou tape un service en
  amont lent)
- La clé est assez populaire pour qu'un événement de cache froid envoie
  N requêtes d'un coup au magasin sous-jacent
- La closure a des effets de bord au-delà du calcul de la valeur

Pour ces cas-là, enveloppez avec `Cache::lock` :

```rust
use suprnova::Cache;
use std::time::Duration;

let key = "rebuild:user:1";

if let Some(guard) = Cache::lock(key, Duration::from_secs(10)).await? {
    let user = Cache::remember(
        "user:1",
        Some(Duration::from_secs(3600)),
        || async { User::find(1).await },
    ).await?;
    guard.release().await?;
    return Ok(user);
}

// Course perdue - le gagnant est en train de calculer. Lisez ce qu'il a
// écrit, ou repliez-vous sur une valeur périmée.
let user = Cache::get::<User>("user:1").await?
    .ok_or_else(|| FrameworkError::internal("cache miss after losing rebuild lock"))?;
```

## Verrous

`Cache::lock` retourne une `LockGuard` qui détient le jeton de
possession. Les verrous sont indicatifs et inter-processus quand ils
sont adossés à Redis.

```rust
use suprnova::Cache;
use std::time::Duration;

if let Some(guard) = Cache::lock("job:42", Duration::from_secs(30)).await? {
    do_exclusive_work().await?;
    guard.release().await?;
}
// Some(guard) signifie que nous détenons le verrou. None signifie qu'un
// autre détenteur nous a devancés.
```

La garde expose :

| Méthode | À utiliser pour |
|---|---|
| `guard.token()` | Lire le jeton de possession (nom côté Rust) |
| `guard.owner()` | Même valeur, alias à l'orthographe Laravel |
| `guard.refresh(ttl)` | Prolonger le TTL - retourne `false` si nous ne détenons plus le verrou |
| `guard.release()` | Libérer si nous détenons encore le verrou - retourne `false` si le jeton ne correspond plus |

Il n'y a intentionnellement **aucune libération automatique via
`Drop`**. Un verrou Redis doit être acquitté à travers les frontières de
processus ; une libération automatique au drop reviendrait soit à
reprendre silencieusement un verrou déjà repris par un autre détenteur
(faux), soit à masquer des échecs de libération dans des paniques de
destructeur (pire). La libération est explicite pour que les erreurs se
propagent.

`refresh` permet à un job de longue durée de prolonger son propre
verrou pour éviter un timeout auto-infligé - voir
[Idempotence](idempotency.md) pour le consommateur présent dans l'arbre
du code.

## Compteurs atomiques

```rust
// S'initialise à 0 si absent, puis incrémente. Retourne la nouvelle valeur.
let visits = Cache::increment("page:visits", 1).await?;

// Même forme pour les pas négatifs
let remaining = Cache::decrement("quota:remaining", 1).await?;

// Montant personnalisé
let total = Cache::increment("stats:downloads", 10).await?;
```

Atomique sur les deux backends intégrés : `InMemoryCache` utilise un
`HashMap::entry` verrouillé en écriture ; `RedisCache` utilise
`INCRBY`/`DECRBY`. La valeur stockée est un entier encodé en JSON, donc
`Cache::get::<i64>("page:visits")` fait l'aller-retour avec la même
clé.

## Cache par tags

Les tags vous permettent d'invalider toute une famille d'entrées liées
en un seul appel. Le cas d'usage classique est celui des caches par
ressource qui doivent être vidés ensemble quand la ressource change.

```rust
use suprnova::Cache;
use std::time::Duration;

// Stocker sous un ou plusieurs tags
Cache::tags_put(
    &["users", "user:1"],
    "user:1:profile",
    &profile,
    Some(Duration::from_secs(3600)),
).await?;

Cache::tags_put(
    &["users", "user:1"],
    "user:1:posts",
    &posts,
    Some(Duration::from_secs(600)),
).await?;

// Chemin de mise à jour : supprimer chaque clé taguée `user:1`
Cache::flush_tags(&["user:1"]).await?;
```

L'appartenance à un tag est **par entrée** : chaque écriture taguée
installe l'ensemble de tags de cette écriture comme source de vérité de
l'entrée, en remplaçant tout tag antérieur. Deux conséquences à
connaître :

- Un `Cache::put` non tagué par-dessus une clé précédemment taguée
  **efface** les tags de l'entrée. Un `flush_tags` ultérieur sur
  l'ancien tag ne supprimera pas la valeur non taguée toujours vivante.
- Écraser `tags_put(&["a"], …)` avec `tags_put(&["b"], …)` fait que
  l'entrée ne répond plus qu'à `flush_tags(&["b"])`.

Les références périmées de l'index direct sont élaguées pendant le
parcours de vidage et lors de `flush()`, si bien qu'elles ne
s'accumulent pas indéfiniment pour les tags qui sont écrits mais jamais
vidés.

## Deux backends

| Fonctionnalité | `InMemoryCache` | `RedisCache` |
|---|---|---|
| Partagé entre processus | Non | Oui |
| Persistance | Non | Oui, si Redis est configuré pour ça |
| `add` atomique | Oui (verrou d'écriture) | Oui (`SET NX`) |
| `increment`/`decrement` atomique | Oui (verrou d'écriture) | Oui (`INCRBY`/`DECRBY`) |
| Cache par tags | Oui | Oui |
| Verrous | Oui | Oui (inter-processus) |
| TTL sub-seconde | Oui (`tokio::time::Instant`) | Oui (`PX`/`PEXPIRE`) |
| Sélectionné via | `CACHE_DRIVER=memory` (défaut) | `CACHE_DRIVER=redis` |

Il n'y a pas de driver de cache Database - les deux backends ci-dessus
sont ceux que le framework livre. Des backends personnalisés peuvent
implémenter `CacheStore` et se lier dans le conteneur directement ; voir
le motif d'injection de test ci-dessous.

### Expiration en mémoire

`InMemoryCache` évince les entrées expirées **paresseusement à la
lecture** : `get_raw`, `has`, et `add_raw` purgent une entrée la
première fois qu'ils l'observent expirée. Les clés ré-accédées
n'accumulent jamais de cadavres.

Une charge de travail qui écrit un ensemble de clés éphémères à forte
cardinalité et ne les relit jamais n'a pas ce déclencheur. Appelez
`InMemoryCache::purge_expired()` depuis une tâche périodique dans ce
cas - elle retourne le nombre d'entrées supprimées. Redis gère sa
propre expiration côté serveur ; l'équivalent n'y est pas nécessaire.

### Précision du TTL Redis

Chaque TTL Redis passe par `PX` / `PEXPIRE`, pas `EX` / `EXPIRE`. Cela
évite deux pièges :

- Des `Duration` sub-seconde tronqueraient à `0 seconds` sous `EX`, ce
  que Redis rejette (`SET … EX 0`) ou, pire, interprète comme
  « supprimer la clé » (`EXPIRE key 0`).
- `Duration::ZERO` est plafonnée à 1 ms avant l'appel, si bien qu'aucun
  des deux chemins de rejet n'est atteignable depuis le code
  utilisateur.

### Réessais de commandes transitoires

Une socket coupée faisait auparavant échouer le `Cache::get` qui se
trouvait en vol. La connexion Redis se rétablit d'elle-même, mais la
commande qui a heurté la socket morte vous retourne quand même son
erreur.

Les commandes de forme lecture réessaient désormais une fois : `GET`,
`EXISTS`, et les pages `SCAN` / `SSCAN` derrière `Cache::flush` et
`Cache::flush_tags`. Les lectures `XLEN`, `ZCARD` et `XPENDING` du
driver de file d'attente et le calcul du `Retry-After` du limiteur de
débit réessaient de la même façon. Définissez `REDIS_COMMAND_RETRIES`
pour ajouter d'autres réessais par-dessus celui qui est intégré.

Budgétez le réessai en secondes, et non à l'échelle de la pause de
50 ms qui le précède. Une fois qu'une connexion est tombée, la tentative suivante
attend la connexion de remplacement avant de pouvoir envoyer quoi que
ce soit : elle paie donc tout le budget de connexion du driver, puis
son timeout de réponse :

- Le driver de cache autorise jusqu'à 3 réessais de connexion, espacés
  d'au plus 500 ms, chacun plafonné par un timeout de connexion de 2 s,
  avec un timeout de réponse de 5 s.
- Les drivers de file d'attente et de limitation de débit prennent les
  valeurs par défaut de redis-rs : jusqu'à 6 réessais de connexion avec
  un délai exponentiel non plafonné démarrant à 100 ms, chacun plafonné
  par un timeout de connexion de 1 s, avec un timeout de réponse de
  500 ms.

`REDIS_COMMAND_RETRIES` est plafonné à 10, et ce plafond borne les
tentatives, pas les secondes : au maximum, une seule lecture effectue
12 tentatives, ce qui, face à un Redis en panne, représente des
dizaines de secondes à des minutes sur un seul appel. Une commande
partie en timeout compte comme transitoire au même titre qu'une
commande coupée, si bien qu'un Redis simplement lent fait émettre à
chaque lecture enveloppée jusqu'à ce nombre de commandes plutôt qu'une
seule. N'augmentez ce réglage que là où l'appelant peut se permettre
d'attendre.

Les écritures ne réessaient jamais, quel que soit le réglage. Une
erreur transitoire signifie que la connexion a échoué, pas que le
serveur a décliné la commande - il peut déjà l'avoir exécutée -, si
bien que réessayer un `SET`, un `INCR`, une acquisition de verrou, un
décompte de limitation de débit ou un dépilement de file d'attente
risque une seconde exécution. Ces commandes vous font remonter l'échec,
et votre décision de réessayer est la décision informée.

### Pourquoi Suprnova diverge

La configuration `command_retries` de Laravel relève le budget de
réessais pour toutes les commandes Redis, parce que sa méthode
`command()` est un point de passage unique qui sait quelle commande
elle exécute et consulte une liste blanche de 60 entrées en lecture
seule. Les drivers de Suprnova appellent directement des commandes
typées, si bien que la liste blanche devient une décision prise site
d'appel par site d'appel, et `REDIS_COMMAND_RETRIES` ne peut
qu'approfondir les réessais des commandes qu'il est déjà sans danger de
répéter. Aucun réglage ne fait réessayer un dépilement de file
d'attente.

## Tests

Liez un `InMemoryCache` dans le `TestContainer` et la façade le résout
comme n'importe quel autre magasin :

```rust
use std::sync::Arc;
use suprnova::{Cache, CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;

#[tokio::test]
async fn cache_round_trips() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    Cache::put("k", &"v", None).await.unwrap();

    let v: Option<String> = Cache::get("k").await.unwrap();
    assert_eq!(v.as_deref(), Some("v"));
}
```

`TestContainer::bind` écrit dans la portée thread-local, si bien que les
tests parallèles ne font pas fuiter leur état de cache l'un vers
l'autre. Voir le chapitre [Conteneur de service](container.md) pour le
modèle de recherche à trois couches.

### Suites sur un Redis réel

Les tests Redis du framework lui-même sont marqués `#[ignore]`, si bien
que `cargo test` n'a jamais besoin d'un serveur. Lancez-les avec
`-- --ignored` et pointez-les vers une instance :

- `cache_redis_integration` lit `CACHE_REDIS_TEST_URL`, en se rabattant
  sur `REDIS_URL` puis sur `redis://127.0.0.1:6379`. Chaque test se
  cantonne à un préfixe de clé unique, si bien qu'il est sans danger
  face à un Redis de développement partagé.
- `cache_redis_retry` couvre le réessai des commandes transitoires et
  exige `CACHE_REDIS_TEST_URL` explicitement, sans repli. Il émet
  `CLIENT KILL TYPE normal`, qui déconnecte tous les autres clients de
  l'instance : il faut donc lui donner un serveur jetable. Avec la
  variable non définie, il affiche une ligne de saut et passe sans se
  connecter.

## Motifs

Quelques formes récurrentes qui méritent d'être nommées :

```rust
// Clés hiérarchiques séparées par deux-points - même convention que Laravel
Cache::put("users:1:profile", &profile, None).await?;
Cache::put("posts:123:comments:count", &count, None).await?;

// TTL selon la volatilité des données
Cache::put("stats:active", &count, Some(Duration::from_secs(60))).await?;
Cache::put("config:features", &features, Some(Duration::from_secs(3600))).await?;
Cache::forever("translations:en", &translations).await?;

// Invalidation par tag autour d'une écriture
async fn update_user(id: i64, data: UserUpdate) -> Result<User, FrameworkError> {
    let user = User::update(id, data).await?;
    Cache::flush_tags(&[&format!("user:{}", id)]).await?;
    Ok(user)
}
```

## Suivant

- [Configuration](configuration.md) - comment `Config::register` et les
  variables d'env se combinent
- [Limitation de débit](rate-limiting.md) - la façade `RateLimiter` à la
  Laravel est construite par-dessus `Cache`
- [Idempotence](idempotency.md) - le middleware de déduplication de
  requêtes utilise `Cache::lock` de bout en bout
- [Conteneur de service](container.md) - comment `CacheStore` est lié et
  résolu
- [Modèle d'erreur](error-model.md) - ce que `Cache::*` retourne quand
  Redis est injoignable en cours de requête

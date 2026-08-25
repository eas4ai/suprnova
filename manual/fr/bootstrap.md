# Amorçage de l'application

`bootstrap.rs` est l'unique endroit où votre application se câble elle-même au
démarrage. Il y a deux hooks, pas un seul. `register` est global au processus :
tous les sous-commandes l'exécutent, y compris `queue:work`, `schedule:work`,
`workflow:work`, et votre binaire console, pas seulement le serveur.
Enregistrez-y la connexion à la base, les liaisons du conteneur, les écouteurs
d'événements, les observateurs, les superviseurs, et l'enregistrement des jobs
des workers. `register_http_stack`, branché via `.http_bootstrap`, ne s'exécute
que sur le chemin serveur (`serve` / `web:run`) - le middleware global et
`Inertia::install` vont là. La section « Où bootstrap se situe dans l'ordre de
démarrage » ci-dessous explique pourquoi cette séparation existe.

## La forme

Le point d'entrée d'une application scaffoldée construit une
[`Application`](lifecycle.md) de façon fluide, puis l'exécute. Bootstrap est
maintenant constitué de deux méthodes du builder :

```rust
// cmd/main.rs
use app::{bootstrap, config, migrations, routes};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

### `#[suprnova::main]`, pas `#[tokio::main]`

L'attribut n'est pas cosmétique, et revenir en arrière casse
l'amorçage avec un message qui explique pourquoi.

Charger `.env` écrit dans l'environnement du processus, et `set_var`
n'est sûr que tant que le processus est mono-thread. `#[tokio::main]`
construit le runtime *autour* de l'ensemble de `main`, si bien que
chaque thread de travail existe déjà avant que votre première
instruction ne s'exécute - et n'importe lequel d'entre eux peut
appeler `getenv` indirectement via la résolution DNS, le formatage de
date/heure, ou une dépendance C. La course est silencieuse quand elle
échoue, ce qui est la pire propriété qu'une course puisse avoir.

`#[suprnova::main]` conserve le même `async fn main` que vous écririez
de toute façon, et réordonne simplement deux choses : il charge
l'environnement, puis construit le runtime, puis exécute votre corps
de fonction sur celui-ci. Il accepte les mêmes arguments `flavor` et
`worker_threads` que `#[tokio::main]`.

Si `Application::run` constate que l'environnement n'a jamais été
chargé depuis un contexte mono-thread, elle refuse de démarrer plutôt
que d'avertir - une application qui démarre « correctement » sous
`#[tokio::main]` est précisément celle qui corrompra une lecture
d'environnement sans rapport, des semaines plus tard.

Le framework appelle votre `bootstrap_fn` une fois durant la séquence
d'amorçage, après le chargement de l'environnement et après que les drivers de
runtime (Cache, Queue, RateLimit, Mail) sont opérationnels, mais avant la
construction du routeur. Le même appel s'exécute pour les workers en
arrière-plan (`queue:work`, `workflow:work`, `schedule:work`), si bien qu'un
observateur ou un écouteur enregistré ici se déclenche de façon identique pour
une insertion venant d'un job de file d'attente et pour une insertion venant
d'un handler HTTP. `http_bootstrap_fn` s'exécute immédiatement après
`bootstrap_fn`, mais uniquement sur le chemin serveur  -  les workers en
arrière-plan et le binaire console ne l'appellent jamais. [Cycle de vie des
requêtes](lifecycle.md) détaille la séquence complète.

Les signatures des deux fonctions sont fixées par `Application::bootstrap` et
`Application::http_bootstrap` :

```rust
// src/bootstrap.rs
pub async fn register() {
    // base de données, bindings, observers, écouteurs, supervisors, enregistrement des jobs worker
}

pub fn register_http_stack() {
    // global middleware, Inertia::install
}
```

`register` retourne `()` ; `register_http_stack` est synchrone, non
`async`  -  les deux sont branchées sous forme de closures async au site de
l'appel (`.http_bootstrap(|| async { bootstrap::register_http_stack() })`),
car un pointeur de fonction simple peut aussi servir de point d'entrée à un
harness de test sans introduire `async` dans le test. Une configuration faillible
utilise `.expect("…")` avec un message qui explique la remédiation  -
l'amorçage est le bon moment pour échouer explicitement. L'appel de
l'application d'exemple est
`DB::init().await.expect("Failed to connect to database");`, si bien qu'un
`DATABASE_URL` manquant interrompt le processus à l'amorçage avec l'erreur
réelle affichée, plutôt que de se manifester comme un « connection refused »
déroutant à la première requête.

## Ce qui va dans bootstrap

Une véritable fonction `bootstrap` fait un petit nombre de choses distinctes. Chaque sous-section ci-dessous en décrit une. Pour l'initialisation Magnetar par défaut actuelle, prenez le modèle `src/bootstrap.rs` du scaffold API comme référence de travail.


### Connexion à la base de données

```rust
use suprnova::DB;

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");
}
```

`DB::init` lit `DatabaseConfig` (enregistrée par votre `config_fn`) et
ouvre le pool. La connexion est stockée dans le
[conteneur](container.md) comme un singleton - `DB::connection()` /
`DB::get()` la résout n'importe où. `DB::init_with(config)` est
l'échappatoire pour les tests et l'outillage quand vous voulez pointer
vers autre chose que l'URL dérivée de l'environnement.

### Moteur d'authentification Magnetar

Les applications qui utilisent les façades intégrées de mot de passe, passkey,
lien magique, bearer, verrouillage, remember ou OAuth initialisent Magnetar
après que la base de données et `APP_KEY` sont prêtes :

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");

    let database = DB::connection().expect("DB not initialized");
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config)
        .await
        .expect("Failed to initialize Magnetar");
}
```

Le `MagnetarConfig` par défaut lie les identités applicatives à la table canonique `app_users`. Le scaffold full-stack généré utilise un modèle `users` et n'initialise pas Magnetar ; n'ajoutez donc pas l'initialiseur par défaut tel quel à ce scaffold. Utilisez le modèle `app_users` du scaffold API, ou construisez une liaison personnalisée `MagnetarHostEngine` et `AuthSchema` pour votre table `users` existante. Gardez le `UserProvider` du framework et la liaison d'hôte Magnetar sur la même identité applicative. Le scaffold API, et non `app/src/bootstrap.rs`, est la référence de travail actuelle pour l'initialisation par défaut de `MagnetarConfig`.


### Middleware global
Le middleware global est propre au HTTP ; il appartient donc à
`register_http_stack`, non à `register` :

```rust
use suprnova::{global_middleware, SessionMiddleware, SessionConfig, TimeoutMiddleware};
use crate::middleware;

pub fn register_http_stack() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));
}
```

`global_middleware!` enregistre une couche qui s'exécute sur chaque
requête, y compris celles qui ne sont pas routées (404, préflight
OPTIONS). L'ordre dans lequel vous enregistrez est l'ordre dans lequel
la chaîne s'exécute - de l'extérieur vers l'intérieur. Le framework
place son propre `RequestIdMiddleware` le plus à l'extérieur ; tout ce
que vous ajoutez se place à l'intérieur. [Middleware](middleware.md)
explique la forme complète de la chaîne, y compris la couche par
route.

### Liaisons du conteneur

Le conteneur prend tout ce que vous y mettez ; les macros sont du
sucre syntaxique par-dessus la façade [`App`](container.md).

```rust
use std::sync::Arc;
use suprnova::{App, bind, singleton, factory};
use crate::providers::DatabaseUserProvider;

pub async fn register() {
    // Trait → singleton (wraps in Arc):
    bind!(dyn UserProvider, DatabaseUserProvider);

    // Concrete singleton:
    singleton!(MyConfig { max_uploads_per_user: 100 });

    // Factory (constructed per resolve):
    factory!(|| RequestLogger::new());

    // Or call the facade directly for finer control:
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub);
}
```

Les liaisons d'objets trait sont la forme la plus courante - liez une
interface, laissez les handlers et les tests substituer
l'implémentation. Le chapitre [Conteneur de service](container.md)
présente l'API de liaison complète, y compris `bind_factory!`, les
variantes `_if_absent`, et le modèle de recherche à trois couches.

### Écouteurs d'événements et observateurs

Le dispatcher est actif dès que bootstrap s'exécute - les écouteurs
enregistrés ici voient chaque dispatch suivant.

```rust
use std::sync::Arc;
use suprnova::EventFacade;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;

pub async fn register() {
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
}
```

Les observateurs Eloquent (`#[suprnova::observer(M)]`) se collectent
eux-mêmes via `inventory::submit!` à la compilation. Un seul appel
vide l'inventaire dans le dispatcher :

```rust
suprnova::eloquent::observers::bootstrap_observers()
    .await
    .expect("observer install failed");
```

L'appel est idempotent - réexécuter bootstrap (un worker qui démarre
une seconde fois) n'enregistre pas deux fois les adaptateurs
d'écouteurs. [Événements](events.md) couvre le dispatch et l'écriture
d'écouteurs ; [Eloquent API](eloquent.md) couvre les observateurs.

### Superviseurs

Les tâches d'arrière-plan de longue durée déclarées via le trait
`Supervisor` et `inventory::submit!` démarrent par un seul appel :

```rust
use suprnova::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

Chaque superviseur s'exécute dans sa propre tâche en boucle de
redémarrage avec une limite de panique ; un superviseur qui panique
est journalisé et redémarré, sans avoir la possibilité de faire tomber
le processus. Voir [Superviseurs](supervisors.md) pour le trait et la
politique de redémarrage.

### Enregistrement des jobs de worker

Les jobs de file d'attente et les mailables que les workers doivent
pouvoir dispatcher par nom s'enregistrent eux-mêmes à l'amorçage :

```rust
use suprnova::queue::worker::register_job;

pub async fn register() {
    register_job::<crate::jobs::welcome_log::WelcomeLog>();

    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();
}
```

Sans cela, le worker n'a aucun moyen de faire correspondre une
enveloppe mise en file d'attente au type qui la traite.

## Le hook post-amorçage : `booted()`

Bootstrap *enregistre* ; `booted()` *résout*. Le builder prend un
second callback qui se déclenche après que le serveur a terminé son
propre amorçage de services, mais avant qu'il ne commence à accepter
des connexions. Utilisez-le quand vous devez lire quelque chose que le
framework lui-même a lié durant l'amorçage :

```rust
Application::new()
    .config(config::register_all)
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
    .routes(routes::register)
    .booted(|| {
        let cfg: MyConfig = suprnova::App::get().unwrap();
        tracing::info!(?cfg, "services booted");
    })
    .run()
    .await;
```

`booted` est synchrone et s'exécute après `Server::from_config` - les
drivers sont opérationnels, les clés de chiffrement sont chargées, vos
liaisons existent. La plupart des applications n'ont pas besoin de ce
hook ; utilisez-le quand un effet de bord ponctuel post-amorçage doit
voir un conteneur entièrement construit.

## Un `bootstrap.rs` complet

Cette composition est représentative et n'est pas un extrait mot pour mot de l'application d'exemple. Elle garde l'enregistrement global au processus dans `register` et la configuration HTTP-only dans `register_http_stack`. L'initialisation Magnetar est montrée séparément plus haut parce que son schéma d'utilisateur applicatif doit correspondre au fournisseur d'utilisateurs du framework.


```rust
//! Bootstrap de l’application - enregistre les services, les écouteurs, le
//! middleware global et la couche Inertia.

use std::sync::Arc;
use std::time::Duration;

use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::features::{FeatureMiddleware, bootstrap_database_cached};
use suprnova::queue::worker::register_job;
use suprnova::{
    App, DB, EloquentUserProvider, EventFacade, FrameworkError, Inertia,
    InertiaConfig, SessionConfig, SessionMiddleware, Storage, SupervisorRegistry,
    UserProvider, bind, global_middleware,
};

use crate::broadcasting::ChatChannel;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;
use crate::middleware;
use crate::models::users::User;

pub async fn register() {
    // ── Database
    DB::init().await.expect("Failed to connect to database");

    // ── Auth provider
    bind!(dyn UserProvider, EloquentUserProvider::<User>::new());


    // ── Broadcasting hub + channel registry
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    let mut registry = ChannelRegistry::new();
    registry.register(ChatChannel);
    App::singleton(Arc::new(registry));

    // ── Écouteurs d’événements + ponts
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
    EventFacade::broadcast::<UserRegistered>(Arc::clone(&hub)).await;

    // ── Storage disks (env-gated S3 in production)
    Storage::register_fs("public", "./storage/public")
        .expect("register public disk");

    // ── Worker job registration
    register_job::<crate::jobs::welcome_log::WelcomeLog>();
    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();

    // ── Observers + supervisors
    suprnova::eloquent::observers::bootstrap_observers()
        .await
        .expect("observer install failed");
    SupervisorRegistry::start_all().await;

    // ── Feature flags
    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature-flag chain wired");
}

pub fn register_http_stack() {
    // ── Global middleware (outside-in in registration order)
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(suprnova::TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));

    // ── Inertia protocol layer (no version pin: the default hashes the
    // Vite build manifest, so a frontend build bumps the asset version
    // on its own - see "Version detection" in frontend-inertia-responses.md)
    Inertia::install(&InertiaConfig::new()).expect("Inertia install failed");

    global_middleware!(FeatureMiddleware::new());
}
```

Remarquez le rythme : chaque bloc fait une seule chose, appelle une ou
deux API, et réussit ou échoue avec un message clair. Rien ici n'est
astucieux ; les fonctions sont longues parce que l'application a beaucoup
de pièces mobiles, pas parce que le motif bootstrap est compliqué.

## Quand utiliser bootstrap plutôt que `#[injectable]`

`#[injectable]` est une macro qui enregistre automatiquement un
singleton dans l'`inventory` du conteneur à la compilation. C'est le
bon choix pour les services qui n'ont besoin de rien de plus que leurs
dépendances `#[inject]` pour se construire :

```rust
use suprnova::injectable;

#[injectable]
pub struct UserService;

#[injectable]
pub struct OrderService {
    #[inject]
    user_service: UserService,
}
```

Ceux-ci se résolvent eux-mêmes ; bootstrap n'a pas besoin de les
toucher.

Bootstrap est le bon endroit quand la construction a besoin de quoi
que ce soit d'autre - une variable d'environnement, une struct de
config construite, une liaison `dyn Trait`, une décision prise à
l'exécution, un appel de configuration async, ou l'enregistrement de
quelque chose qui n'est pas lui-même un service (un écouteur, un
observateur, un mapping de job de file d'attente, une couche de
middleware global).

| Utilisez `#[injectable]` pour | Utilisez `bootstrap` pour |
|---|---|
| Les singletons concrets sans config à l'exécution | Tout ce qui est `dyn Trait` |
| Les services construits à partir d'autres injectables | Tout ce qui est async à l'amorçage |
| Le graphe d'injection de dépendances par défaut | Les valeurs pilotées par l'environnement |
| | Les écouteurs d'événements, observateurs, superviseurs |
| | Le middleware global |
| | L'enregistrement des jobs de worker et des mailables |

Vous pouvez les mélanger librement. Les services `#[injectable]` sont
visibles dans le conteneur au moment où `bootstrap` s'exécute, si bien
qu'une liaison dans bootstrap peut les lire.

## Où bootstrap se situe dans l'ordre d'amorçage

La séquence complète (extraite de [Cycle de vie des
requêtes](lifecycle.md)) :

1. `Config::init(".")` - charge `.env`, détecte l'environnement
2. `init_policies()` - vide l'inventaire `#[policy]`
3. Votre `config_fn` s'exécute (enregistrement de configuration typée)
4. Les migrations s'exécutent (auto-migration sur `serve`)
5. **Votre `bootstrap_fn` s'exécute** ← `bootstrap::register`
6. **Votre `http_bootstrap_fn` s'exécute, chemin serveur uniquement** ← `bootstrap::register_http_stack`
7. Les routes sont assemblées à partir de votre `routes_fn`
8. `Server::from_config` amorce les drivers + le conteneur
9. Vos `booted_fn` se déclenchent
10. Le serveur commence à accepter des connexions

Les workers en arrière-plan (`queue:work`, `workflow:work`, `schedule:work`)
et le binaire console partagent les étapes 1 à 5 et 8 : ils exécutent
`bootstrap_fn`, mais jamais l'étape 6, car seuls `serve` / `web:run` exécutent
`http_bootstrap_fn`. Cela permet à un écouteur ou un observateur enregistré
dans `register` d'atteindre les chemins de code worker exactement comme les
handlers HTTP, tandis que le middleware global et `Inertia::install` de
`register_http_stack` restent hors des processus qui ne servent jamais HTTP.

### Pourquoi Suprnova diverge

Laravel exécute les `register()` et `boot()` de chaque fournisseur de service
aussi pour les commandes `artisan` et les workers de file d'attente, pas
seulement pour les requêtes HTTP  -  et s'en sort parce que son intégration Vite
résout les URL d'assets paresseusement, au rendu, selon ce que la directive
Blade `@vite` doit rendre. Un worker qui ne rend jamais de vue ne touche jamais
au manifeste ; un build manquant ne se présente donc simplement jamais.

`Inertia::install` de Suprnova résout le manifeste une fois, à l'amorçage, et
échoue de manière fermée en production lorsqu'il est absent  -  par conception,
afin qu'un déploiement mal configuré ne puisse pas servir des URL d'assets
pointant vers un serveur de dev Vite que personne n'exécute. Ce choix de
conception est précisément ce qui casse un worker ou une image console qui,
correctement, ne fournit aucun `public/assets` : l'échec que Laravel reporte
au temps de requête, Suprnova le rencontrerait autrement au démarrage du
processus, pour chaque sous-commande. Scinder la surface d'amorçage entre
`bootstrap` et `http_bootstrap` maintient la vérification à échec fermé, mais
uniquement là où elle a sa place  -  le chemin serveur qui rendra réellement une
page Inertia.

Laravel scinde aussi son amorçage entre plusieurs fournisseurs de service :
chaque fournisseur implémente `register()` et `boot()`, ils sont collectés dans
`config/app.php`, et Laravel les parcourt en deux passes (tous les `register`,
puis tous les `boot`) afin qu'un service puisse dépendre des liaisons d'un
autre fournisseur sans cérémonie d'ordonnancement dans le code utilisateur.
La classe de fournisseur vous donne une unité d'organisation quand une
application accumule des dizaines de sous-systèmes distincts.

Suprnova réduit cela à deux fonctions  -  `register` et `register_http_stack`  -
plutôt qu'à une paire `register` / `boot` par fournisseur. Les raisons :

- **La séparation en deux passes `register` / `boot` résout un problème
  d'ordonnancement que Rust n'a pas.** `#[injectable]` et le
  `bootstrap_singletons` du conteneur résolvent déjà les graphes de dépendances
  sans ordonnancement visible par l'utilisateur. Les liaisons s'enregistrent en
  ligne ; la machinerie de recherche fait le reste.
- **Deux fonctions sont plus faciles à lire que dix.** Un nouveau contributeur
  ouvre `bootstrap.rs` et voit chaque liaison, chaque écouteur, chaque
  observateur, chaque couche de middleware dans l'un des deux endroits. La
  fragmentation façon fournisseurs cache ce que l'application fait réellement.
- **L'auto-enregistrement façon inventory couvre le reste.** Les observateurs,
  superviseurs, tâches planifiées, politiques et handlers de file d'attente se
  collectent tous eux-mêmes à la compilation via `inventory::submit!`.
  Bootstrap vide les inventaires avec des appels uniques
  (`bootstrap_observers`, `SupervisorRegistry::start_all`) plutôt que de les
  énumérer un par un.

L'endroit où la séparation en fournisseurs de Laravel se justifie,
c'est la distribution de bibliothèques : une crate qui livre ses
propres liaisons voudrait un point d'entrée d'enregistrement auquel
une application puisse souscrire sans modifier son propre bootstrap.
L'analogue chez Suprnova est une `pub async fn register()` publique à
la racine de la crate, et un appel d'une ligne depuis le `bootstrap`
de l'application. Le coût ergonomique est d'une ligne ; le gain en
lisibilité, c'est tout au même endroit.

## Suivant

- [Cycle de vie des requêtes](lifecycle.md) - l'ordre d'amorçage
  complet et où `bootstrap_fn` se déclenche
- [Conteneur de service](container.md) - `App::bind` /
  `App::singleton` / `App::factory` et la recherche à trois couches
- [Configuration](configuration.md) - l'enregistrement de
  configuration typée qui s'exécute avant bootstrap
- [Middleware](middleware.md) - la composition de chaîne pour les
  couches enregistrées avec `global_middleware!`
- [Événements](events.md) - le dispatcher auquel se connectent les
  écouteurs et les observateurs

# Middleware

Le middleware enveloppe un handler de requête. Il s'exécute avant que le
handler ne voie la requête, puis de nouveau après que le handler a
retourné une réponse - c'est donc l'endroit où placer le travail
transversal : authentification, journalisation, CORS, limitation de
débit, chronométrage, transformation de la requête ou de la réponse. La
surface de Suprnova est celle que les utilisateurs de Laravel connaissent
déjà : une méthode `handle(request, next)` qui décide de transmettre la
requête, de la court-circuiter, ou de modifier la réponse sur le chemin
du retour.

## Le trait

Un middleware est une struct qui implémente `Middleware` :

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct LoggingMiddleware;

#[async_trait]
impl Middleware for LoggingMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Pré-traitement : s'exécute avant le handler.
        println!("--> {} {}", request.method(), request.path());

        // Transmet au middleware suivant (ou au handler si c'est
        // la dernière couche).
        let response = next(request).await;

        // Post-traitement : s'exécute après que le handler a retourné.
        println!("<-- complete");

        response
    }
}
```

`handle` a trois choses possibles à faire, et vous n'avez besoin d'en
faire qu'une seule pour une requête donnée :

- **Transmettre.** Appelez `next(request).await` pour passer le contrôle
  à la couche suivante. La `Response` retournée est ce que chaque couche
  au-dessus verra.
- **Court-circuiter.** Retournez `Err(HttpResponse::...)` sans appeler
  `next`. Le framework réduit les deux branches de `Response`
  (`Result<HttpResponse, HttpResponse>`) en une seule réponse - un `Err`
  est une réponse, pas un crash. Voir [Modèle d'erreur](error-model.md).
- **Modifier.** Modifiez la requête avant de la transmettre, ou modifiez
  la réponse après.

`Next` est `Arc<dyn Fn(Request) -> MiddlewareFuture + Send + Sync>` -
traitez-le comme une fonction async de `Request` vers `Response`.

## Générer un stub

La CLI scaffolde un fichier de middleware fonctionnel :

```bash
suprnova make:middleware Auth         # → src/middleware/auth.rs (AuthMiddleware)
suprnova make:middleware RateLimit    # → src/middleware/rate_limit.rs
suprnova make:middleware CorsMiddleware  # le suffixe "Middleware" convient aussi, même résultat
```

Le fichier généré n'est pas un stub à compléter - c'est un vrai
middleware qui chronomètre la requête enveloppée et journalise les
événements entrant/sortant avec l'id par requête installé par
`RequestIdMiddleware`. Remplacez le corps par ce dont vous avez
réellement besoin.

## Enregistrer le middleware

Trois endroits où l'installer, selon la portée :

### Global

S'exécute sur chaque requête, dans l'ordre d'enregistrement. Utilisez la
macro `global_middleware!` à l'intérieur de `bootstrap()` :

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, FrameworkError};
use crate::middleware;

pub async fn bootstrap() -> Result<(), FrameworkError> {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
    Ok(())
}
```

`global_middleware!(M)` se développe en `register_global_middleware(M)`.
L'enregistrement est **idempotent par type concret** - enregistrer deux
fois la même struct conserve le premier enregistrement et émet un
journal de débogage. Cela rend sûr le fait de relancer l'amorçage
(tests, rechargement à chaud, plusieurs instances de `Server` dans un
même processus). Pour installer plusieurs copies du même comportement
avec une config différente, enveloppez chacune dans un newtype distinct.

### Par route

Chaînez `.middleware(M)` sur une définition de route issue de la macro
`routes!` :

```rust
// src/routes.rs
use suprnova::{routes, get};
use crate::{controllers, middleware::AuthMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/public", controllers::home::public),

    get!("/protected", controllers::dashboard::index)
        .middleware(AuthMiddleware),
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware),
}
```

### Par groupe

Appliquez un middleware à chaque route d'un bloc `group(...)` :

```rust
use suprnova::Router;
use crate::middleware::{ApiMiddleware, AuthMiddleware};
use crate::controllers::{user, admin};

Router::new()
    // Routes publiques - pas de middleware.
    .get("/", home_handler)
    .get("/login", login_handler)

    // Chaque route sous /api porte ApiMiddleware.
    .group("/api", |r| {
        r.get("/users", user::index)
         .post("/users", user::store)
         .get("/users/{id}", user::show)
    })
    .middleware(ApiMiddleware)

    // Les routes admin partagent l'authentification.
    .group("/admin", |r| {
        r.get("/dashboard", admin::dashboard)
         .get("/settings", admin::settings)
    })
    .middleware(AuthMiddleware);
```

## Ordre d'exécution

À l'exécution, la chaîne s'exécute de l'extérieur vers l'intérieur :

```
Requête  →  RequestId  →  globaux  →  MW de groupe  →  MW de route  →  handler
                                                                         │
Réponse  ←  RequestId  ←  globaux  ←  MW de groupe  ←  MW de route  ←  handler
```

Le premier middleware ajouté s'exécute en premier. Sur le chemin du
retour, l'ordre s'inverse - `MiddlewareChain::execute` imbrique le
post-traitement de chaque couche à l'intérieur de la précédente.

Si un middleware court-circuite avec `Err(response)`, la chaîne remonte
immédiatement : chaque couche AU-DESSUS du court-circuit voit quand
même la réponse sur le chemin du retour, mais les couches EN DESSOUS
(plus proches du handler) ne s'exécutent pas.

### Le middleware de groupe est aplati, pas empilé

Celui-ci compte et mérite d'être signalé. **Le middleware de groupe de
routes n'est pas une couche d'exécution séparée.** Quand
`GroupBuilder::try_finalize` s'exécute, il copie le middleware du groupe
dans la liste de middleware `(method, pattern)` de chaque route
groupée. Au moment de l'exécution, le middleware de groupe est
indiscernable du middleware attaché directement à la route.

Deux conséquences :

- L'ordre d'exécution reste correct (le middleware de groupe s'exécute
  avant le middleware de route car il est enregistré en premier), mais
  **l'introspection ne peut pas distinguer le middleware de groupe de
  celui de route**.
- Le middleware est indexé par le motif correspondant
  (`"/posts/{id}"`), pas par le chemin brut (`/posts/42`), donc le
  middleware de groupe sur les routes paramétrées se déclenche de façon
  fiable.

Voir `framework/src/routing/group.rs` pour la passe d'aplatissement et
`framework/src/middleware/chain.rs` pour la boucle d'exécution.

## Court-circuiter

Retournez tôt pour bloquer une requête avant qu'elle n'atteigne le
handler :

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct RequireApiKey;

#[async_trait]
impl Middleware for RequireApiKey {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if request.header("X-Api-Key").is_none() {
            return Err(HttpResponse::text("Unauthorized").status(401));
        }
        next(request).await
    }
}
```

La chaîne réduit `Result<HttpResponse, HttpResponse>` en une seule
réponse, donc `Err(...)` est simplement une réponse avec un rôle
différent. Les couches au-dessus de ce middleware l'observent quand même
sur le chemin du retour et peuvent la post-traiter.

## Sécurité en cas de panique

`MiddlewareChain::execute` ne capture PAS les paniques - une panique
dans n'importe quel middleware ou dans le handler se propage
directement, comme n'importe quelle autre fonction async. Le filet de
sécurité du chemin de requête se situe un niveau au-dessus, à la
frontière du serveur, dans `execute_chain_safely`, qui enveloppe la
chaîne dans `catch_unwind` et convertit une panique en un 500 assaini
portant l'id de requête, en envoyant `ErrorOccurred` pour tout écouteur
d'observabilité. Voir [Cycle de vie des requêtes](lifecycle.md) pour le
flux complet de récupération de panique.

Cette séparation est délibérée : la gestion standardisée des paniques a
lieu exactement une fois, là où le cycle de vie de la requête en a la
responsabilité, plutôt que d'être dupliquée à l'intérieur de la
primitive agnostique aux couches. Un consommateur qui pilote une chaîne
en dehors de cette frontière est responsable de son propre
`catch_unwind`.

## Middleware intégré

Une liste non exhaustive. Chacun est livré prêt à installer - la
plupart ont besoin d'une struct de config, aucun n'a besoin de
scaffolding.

| Middleware | Rôle |
|---|---|
| `RequestIdMiddleware` | Toujours la couche la plus extérieure ; assigne un UUID par requête et le propage dans les journaux + `X-Request-Id` |
| `TimeoutMiddleware` | Borne le temps de réponse ; retourne 503 en cas de dépassement (voir ci-dessous) |
| `CorsMiddleware` | Gère le préflight CORS + décore les réponses cross-origin (voir ci-dessous) |
| `CsrfMiddleware` | Protection CSRF par double soumission de cookie, avec `OriginPolicy` configurable |
| `RateLimitMiddleware` / `ThrottleRequestsMiddleware` | Limitation par seau de jetons et par fenêtre glissante ; voir [Limitation de débit](rate-limiting.md) |
| `SessionMiddleware` | Charge/persiste la session via des cookies ; alimente `req.session()` |
| `AuthMiddleware` / `GuestMiddleware` / `BearerTokenMiddleware` | Vérifications d'appartenance à un guard ; voir [Authentification](authentication.md) |
| `LoginThrottleMiddleware` / `EnsureEmailVerifiedMiddleware` / `TwoFactorChallengeMiddleware` | Filtres de flux d'authentification ; voir [Flux d'authentification](auth-flows.md) |
| `MaintenanceMiddleware` | Retourne 503 quand le flag de maintenance du cache ou du système de fichiers est activé |
| `InertiaVersionMiddleware` / `EncryptHistoryMiddleware` | Négociation de version des assets Inertia + chiffrement de l'historique |
| `IncludeMiddleware` | Ensembles d'inclusion par champ pour les rechargements partiels de `#[derive(Data)]` |

### Délais d'attente de requête

`TimeoutMiddleware` borne le temps qu'un handler peut prendre pour
*produire* une réponse. Sans cela, un handler lent ou une requête de
base de données bloquée peut maintenir une connexion ouverte
indéfiniment ; le délai d'attente retourne `503 Service Unavailable` une
fois l'échéance dépassée.

```rust
// src/bootstrap.rs - Plafond de 30 secondes sur chaque route HTTP.
use suprnova::{global_middleware, TimeoutMiddleware};

global_middleware!(TimeoutMiddleware::default()); // DEFAULT_TIMEOUT = 30s
```

```rust
// Resserre un point de terminaison unique à 5 secondes.
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    .get("/report", heavy_report_handler)
    .middleware(TimeoutMiddleware::seconds(5));
```

`TimeoutMiddleware::new(Duration)` accepte n'importe quelle durée ;
`TimeoutMiddleware::seconds(n)` est un raccourci pour des secondes
entières.

Le middleware global s'exécute **à l'extérieur** du middleware de
route, donc un délai d'attente global est un plafond extérieur et un
délai par route ne peut que rendre une route spécifique *plus stricte* -
l'échéance la plus courte se déclenche en premier. Pour permettre à une
route de s'exécuter plus longtemps que la valeur globale par défaut,
augmentez la valeur globale ou limitez la portée du middleware global à
un groupe de routes qui exclut ce point de terminaison.

Les réponses en streaming (`HttpResponse::sse(...)`,
`HttpResponse::stream_bytes(...)`) en sont naturellement exemptes : le
handler retourne immédiatement avec un corps paresseux que hyper vide
après la fin de la chaîne de middleware. Les mises à niveau WebSocket
sont elles aussi explicitement ignorées. Voir [Délais
d'attente](timeout.md) pour la sémantique de sécurité d'annulation.

### CORS

`CorsMiddleware` ajoute les en-têtes `Access-Control-*` dont un
navigateur a besoin pour permettre à une page cross-origin de lire vos
réponses, et répond à la requête préflight `OPTIONS` que les navigateurs
envoient avant les appels cross-origin non simples. Les applications
same-origin (la configuration Inertia par défaut) n'en ont pas besoin -
cela ne compte que lorsqu'un navigateur sur une origine *différente*
appelle votre API.

CORS doit être installé **globalement** pour que les préflights
l'atteignent (un préflight ne correspond jamais à une route, donc un
middleware CORS par route n'en verrait jamais). Il n'y a
intentionnellement aucune valeur par défaut permissive - choisissez
explicitement une politique d'origine :

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

global_middleware!(CorsMiddleware::new(
    CorsConfig::allow_origins(["https://app.example", "https://admin.example"])
        .allow_credentials(true)
        .max_age(std::time::Duration::from_secs(600)),
));
```

`CorsConfig::any_origin()` active explicitement
`Access-Control-Allow-Origin: *`. Méthodes du builder : `.methods([...])`,
`.allow_headers([...])` / `.allow_any_headers()`, `.expose_headers([...])`,
`.paths([...])` (limite CORS à des motifs d'URL),
`.allow_origin_patterns([regex...])`, `.skip_when(|req| bool)`,
`.allow_credentials(bool)`, `.max_age(Duration)`. Des alias au nom
Laravel sont livrés en parallèle (par ex. `.supports_credentials`,
`.allowed_methods`) pour qu'une config Laravel se transpose directement.

`Access-Control-Allow-Origin: *` est invalide combiné avec des
identifiants - le navigateur le rejette. Quand `.allow_credentials(true)`
est défini, le middleware renvoie toujours l'`Origin` spécifique de la
requête au lieu de `*`, donc la combinaison invalide ne peut jamais être
émise. Les réponses non génériques reçoivent aussi `Vary: Origin` pour
que les caches partagés restent corrects. Voir [CORS](cors.md).

## Pipeline - le `Illuminate\Pipeline\Pipeline` de Laravel

`Pipeline` est l'équivalent Suprnova de la classe pipeline de Laravel -
un builder fluide au-dessus de `MiddlewareChain` qui reproduit la forme
`send / through / pipe / then / then_return / finally_with` que les
utilisateurs de Laravel connaissent déjà. Utile quand vous voulez
assembler une chaîne de middleware en dehors du cycle de vie de la
requête (un job, une commande CLI, un test d'intégration ponctuel) :

```rust
use suprnova::{Pipeline, Request};

let response = Pipeline::new()
    .send(request)
    .through([AuthMiddleware, LoggingMiddleware])
    .pipe(CorsMiddleware::new(cors_config))
    .finally_with(|| tracing::info!("pipeline complete"))
    .then(|req| async move { handler(req).await })
    .await;
```

Des alias côté Rust sont livrés aux côtés des noms Laravel :
`with_request` pour `send`, `with_middleware` pour `through`, `push`
pour `pipe`, `on_finally` pour `finally_with`, `execute` pour `then`.
Utilisez celui qui se lit le mieux dans votre codebase.

| Méthode Pipeline | Laravel | Alias Rust | Rôle |
|---|---|---|---|
| `send(request)` | `send($passable)` | `with_request(request)` | Définit la requête qui est enfilée à travers la chaîne |
| `through(iter)` | `through($pipes)` | `with_middleware(iter)` | Remplace la liste des pipes |
| `through_boxed(iter)` | - | - | Remplace la liste des pipes par du middleware déjà boxé |
| `pipe(M)` | `pipe($pipes)` | `push(M)` | Ajoute un middleware unique |
| `pipe_boxed(M)` | - | - | Ajoute un middleware déjà boxé |
| `then(destination)` | `then($destination)` | `execute(destination)` | Exécute la chaîne avec le handler de destination |
| `then_with(req, dst)` | - | - | Redéfinit le passable en ligne |
| `then_return()` | `thenReturn()` | - | Exécute la chaîne, retourne un 204 No Content |
| `finally_with(F)` | `finally($callback)` | `on_finally(F)` | S'exécute après la résolution de la destination |

## Middleware terminable - hooks post-réponse

Le middleware terminable s'exécute *après* que la réponse a été envoyée
au client. Utilisez-le pour des IO lentes qui n'ont pas besoin de
bloquer la réponse : persistance de session, journalisation d'audit,
vidages de métriques.

Suprnova livre cela sous la forme d'un trait `Terminable` dédié, séparé
de `Middleware`, afin que le chemin de requête et le chemin de
terminaison restent clairement typés. Un type peut implémenter l'un,
l'autre, ou les deux :

```rust
use suprnova::{Terminable, TerminationSnapshot, register_terminable, async_trait};

pub struct AuditLogTerminator;

#[async_trait]
impl Terminable for AuditLogTerminator {
    async fn terminate(&self, snapshot: &TerminationSnapshot) {
        tracing::info!(
            method = %snapshot.method,
            path = %snapshot.path,
            status = snapshot.status,
            "request handled",
        );
    }
}

// Dans bootstrap.rs
register_terminable(AuditLogTerminator);
```

Le serveur itère les terminables enregistrés dans l'ordre
d'enregistrement après chaque réponse (4xx et 5xx compris) et attend
chacun d'eux. Les erreurs sont journalisées via `tracing::error!` puis
avalées - la réponse a déjà quitté les lieux, donc il ne reste plus
personne à qui les signaler.

L'enregistrement est idempotent par type concret.
`registered_terminables()`, `terminable_count()`, et
`has_terminable::<T>()` fournissent de l'introspection pour les tests et
les diagnostics au moment de l'amorçage.

## Alias nommés et groupes

Pour les consommateurs qui préfèrent un middleware indexé par chaîne
(les `middlewareAliases` / `middlewareGroups` de Laravel), Suprnova
livre un registre d'alias + de groupes global au processus :

```rust
use suprnova::middleware::{
    register_middleware_alias, register_middleware_group,
    resolve_middleware_group,
};

// Les alias sont des fermetures de fabrique - invoquées à nouveau à chaque
// résolution, donc chaque enregistrement de route produit une instance de
// middleware indépendante.
register_middleware_alias("auth", || AuthMiddleware::new());
register_middleware_alias("throttle", || ThrottleRequestsMiddleware::default());

// Les groupes regroupent des alias. Les groupes imbriqués sont pris en charge.
register_middleware_group("api", ["auth".into(), "throttle".into()]);
register_middleware_group("web", ["session".into(), "auth".into()]);

// Résout en un Vec<BoxedMiddleware> à l'amorçage ou par route.
let api_mws = resolve_middleware_group("api")?;
```

`resolve_middleware_group` retourne `Err(MiddlewareResolveError)` dans
les cas suivants :

- `UnknownGroup(name)` - le groupe nommé n'a jamais été enregistré ;
- `UnknownAlias { group, missing }` - une entrée du groupe n'est pas un
  alias connu ;
- `UnknownNestedGroup { group, missing }` - une référence à un groupe
  imbriqué ne parvient pas à se résoudre ;
- `CycleDetected { group }` - la définition du groupe est récursive.

L'enregistrement d'un alias ou d'un groupe suit la règle **dernier
gagne** pour un même nom, à l'image du tableau de kernel réassignable de
Laravel.

## Priorité du middleware

`prepend_middleware_priority::<M>()` / `append_middleware_priority::<M>()`
enregistrent un `TypeId` dans la liste de priorité globale au processus -
l'équivalent Suprnova du `Kernel::$middlewarePriority` de Laravel. Le
middleware dont le type apparaît plus tôt dans la liste se trie en tête
de la chaîne, quel que soit l'ordre d'enregistrement :

```rust
use suprnova::{append_middleware_priority};

// SessionMiddleware s'exécute toujours avant AuthMiddleware, quel que soit
// l'ordre dans lequel ils ont été enregistrés.
append_middleware_priority::<SessionMiddleware>();
append_middleware_priority::<AuthMiddleware>();
```

`middleware_priority()` retourne un instantané du `Vec<TypeId>` courant,
pour des diagnostics ou pour un intégrateur qui veut piloter son propre
trieur.

## Introspection du registre

Au-delà de `register_global_middleware`, le registre expose :

| Surface | Laravel | Rôle |
|---|---|---|
| `prepend_global_middleware(M)` | `prependMiddleware` | Insère en tête de la chaîne |
| `has_global_middleware::<M>()` | `hasMiddleware` | Indique si le type `M` est enregistré |
| `global_middleware_count()` | - | Nombre de globaux actuellement enregistrés |
| `MiddlewareRegistry::from_global()` | - | Prend un instantané du registre global dans un registre par serveur |
| `MiddlewareRegistry::prepend(M)` | - | Insertion en tête façon builder sur une instance de registre |
| `MiddlewareRegistry::append_boxed(M)` | - | Ajoute un middleware déjà boxé |
| `MiddlewareRegistry::prepend_boxed(M)` | - | Insère en tête un middleware déjà boxé |
| `MiddlewareRegistry::len()` / `is_empty()` | - | Introspection du builder |

`MiddlewareRegistry::from_global()` prend un instantané du registre
global au moment de l'appel. Enregistrez tout le middleware global AVANT
de construire le serveur - un appel à `global_middleware!` fait APRÈS
que le serveur a été construit ne s'applique pas rétroactivement, donc
la pile de middleware d'un serveur en cours d'exécution ne peut pas se
dérober sous lui.

## Disposition des fichiers

Une disposition typique une fois que vous avez quelques middlewares :

```
src/
├── middleware/
│   ├── mod.rs          # mod + pub use
│   ├── auth.rs         # AuthMiddleware
│   ├── logging.rs      # LoggingMiddleware
│   └── audit.rs        # AuditLogTerminator
├── bootstrap.rs        # global_middleware! + register_terminable
├── routes.rs           # .middleware(M) par route
└── main.rs
```

`make:middleware` garde `src/middleware/mod.rs` synchronisé - il ajoute
la nouvelle déclaration `mod foo;` et le ré-export `pub use
foo::FooMiddleware;` correspondant lors de la génération du fichier.

## Pourquoi Suprnova diverge

Laravel enregistre les classes de middleware dans `app/Http/Kernel.php`
et les résout via le conteneur, qui effectue de la réflexion sur les
indications de type du constructeur pour injecter les dépendances. Le
modèle un-processus-par-requête de PHP signifie que le kernel est
reconstruit à chaque requête, donc le coût de la résolution par
réflexion est payé une fois par requête et disparaît entre les
requêtes.

Le modèle de processus de Suprnova est un seul binaire qui sert de
nombreuses requêtes simultanées sur de nombreux threads. Construire une
chaîne neuve par requête forcerait un point de synchronisation sur la
liste de middleware globale et réallouerait `Arc<dyn Middleware>` pour
chaque couche à chaque requête. À la place :

- Le middleware global est enregistré dans un `OnceLock<RwLock<Vec<...>>>`
  à l'amorçage, indexé par `TypeId` pour un enregistrement idempotent.
- `MiddlewareRegistry::from_global()` prend un instantané de la liste
  globale une seule fois, à la construction du serveur ; la chaîne par
  requête réutilise cet instantané.
- La chaîne elle-même est composée en imbriquant des fermetures
  `Arc<dyn Fn>`, donc le travail par requête se limite à un
  `Arc::clone` par couche plutôt qu'à une allocation neuve.

La surface visible pour l'utilisateur - `handle(request, next)`, la
macro `global_middleware!`, les alias nommés, les listes de priorité,
les hooks terminables - est la même que celle vers laquelle un
développeur Laravel se tourne. La mécanique sous-jacente remplace la
reconstruction par requête de PHP par un modèle façon Rust
d'instantané-à-l'amorçage, afin que le framework puisse servir des
requêtes concurrentes sans se disputer l'accès au registre.

## Suivant

- [Cycle de vie des requêtes](lifecycle.md) - où la chaîne s'exécute et
  comment les paniques sont capturées à la frontière du serveur
- [Modèle d'erreur](error-model.md) - ce que signifie réellement
  `Result<HttpResponse, HttpResponse>` et comment les court-circuits se
  réduisent
- [Délais d'attente](timeout.md) - la sécurité d'annulation de
  `TimeoutMiddleware` en détail
- [CORS](cors.md) - gestion du préflight, motifs d'origine, portée par
  chemin
- [Limitation de débit](rate-limiting.md) - `RateLimitMiddleware` /
  `ThrottleRequestsMiddleware` et `BackendErrorPolicy`
- [Routage](routing.md) - ce en quoi `routes!`, `Router`, et
  `group(...)` se développent

# Autorisation

L'authentification répond à _« qui êtes-vous ? »_ ; l'autorisation
répond à _« êtes-vous autorisé à faire ceci ? »_ Suprnova livre une
façade `Gate` à la Laravel plus la macro `#[policy]` pour un câblage
orienté ressource, avec des variantes sync et async de chaque
vérification, si bien que la même surface fonctionne que le corps de
votre policy ait besoin d'un accès BD ou d'une simple comparaison de
champ de struct.

## Démarrage rapide

```rust
use suprnova::{Authorizable, Gate};

#[derive(Debug)]
struct User { id: i64, is_admin: bool }
#[derive(Debug)]
struct Post { id: i64, author_id: i64, is_public: bool }

// Permet aux utilisateurs d'opter pour l'ergonomie
// `user.can(action, &resource)`.
impl Authorizable for User {}

// Câblez une ability :
Gate::define::<User, Post>("update", |user, post| {
    user.is_admin || post.author_id == user.id
});

let alice = User { id: 1, is_admin: false };
let own_post = Post { id: 10, author_id: 1, is_public: false };
let foreign_post = Post { id: 11, author_id: 99, is_public: false };

assert!(alice.can("update", &own_post));
assert!(alice.cannot("update", &foreign_post));

// Retourne un 403 directement depuis un handler :
alice.authorize("update", &foreign_post)?;
```

## La surface `Gate`

### Définir des abilities

```rust
// Fermeture sync - invoquée directement, pas de future boxée.
Gate::define::<User, Post>("view", |user, post| post.is_public || user.id == post.author_id);

// Fermeture async - la future doit être owned (pas d'emprunt au-delà
// du retour de la fermeture).
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_is_admin = user.is_admin;
    let post_id = post.id;
    async move {
        // ...recherche BD, appel RPC, etc.
        user_is_admin || check_publish_permission(post_id).await
    }
});
```

Effacé de son type en interne ; le registre s'indexe sur `(action,
TypeId<U>, TypeId<R>)`. Un gate d'action sur `User` et un gate
d'action sur `Comment` portant le même nom vivent indépendamment -
`Gate::has::<User, Post>("publish")` et `Gate::has::<User,
Comment>("publish")` répondent séparément.

### Vérifier des abilities

| Méthode | Retourne | Usage |
|---|---|---|
| `Gate::allows(action, &user, &resource)` | `bool` | Branche rapide |
| `Gate::denies(action, &user, &resource)` | `bool` | Inverse |
| `Gate::authorize(action, &user, &resource)` | `Result<(), FrameworkError>` | 403 sur un refus nu ; un refus riche porte son propre statut/message (voir [Décisions riches](#décisions-riches-response-inspect-raw)) - court-circuite un handler avec `?` |
| `Gate::inspect(action, &user, &resource)` | `Response` | Décision complète : `allowed` + `message` + `code` + `status` HTTP |
| `Gate::raw(action, &user, &resource)` | `Option<Response>` | Comme `inspect`, mais `None` = aucune règle définie (par opposition à un refus explicite) |
| `Gate::any(&[...], &user, &resource)` | `bool` | Vrai si au moins un autorise |
| `Gate::none(&[...], &user, &resource)` | `bool` | Vrai si aucun n'autorise |
| `Gate::check(&[...], &user, &resource)` | `bool` | Vrai si tous autorisent |

Chaque méthode a un homologue `_async` qui fonctionne à la fois pour
les gates enregistrés en sync et en async, si bien que les handlers
n'ont pas besoin de savoir quel type de fermeture soutient l'action.

### Introspection

```rust
// Une ability est-elle définie ?
Gate::has::<User, Post>("publish");  // bool

// Quelles abilities existent ? (triées + dédupliquées par nom d'action)
let all: Vec<String> = Gate::abilities();
```

`abilities()` déduplique à travers les types de ressource :
enregistrer `"view"` à la fois pour `User`-sur-`Post` et
`User`-sur-`Comment` donne une seule entrée `"view"`. Utile pour les
sélecteurs admin et les shared-data Inertia.

### Sémantique du gate manquant

Appeler `allows` / `denies` / `authorize` sur une action qui n'a
jamais été enregistrée **refuse par défaut**. Idem pour appeler l'API
sync sur un gate enregistré en async (le chemin sync ne peut pas
`await` - refuser par défaut fait remonter le bug dans les logs via
`tracing::warn!` plutôt que de le laisser passer silencieusement).
Les gates enregistrés en async répondent correctement depuis les
chemins `_async`.

## Policies avec `#[policy]`

Quand un type de ressource a plusieurs abilities, groupez-les dans
une struct de policy et laissez `#[policy]` enregistrer chaque
méthode comme un gate :

```rust
use suprnova::policy;
use suprnova::authorization::Response;

struct User { id: i64, is_admin: bool }
struct Post { id: i64, author_id: i64, is_public: bool }
struct PostPolicy;

#[policy(User, Post)]
impl PostPolicy {
    // Une méthode `-> bool` est un gate simple allow/deny.
    fn view_any(_user: &User, _post: &Post) -> bool {
        true // tout le monde peut lister les posts
    }
    fn view(user: &User, post: &Post) -> bool {
        post.is_public || post.author_id == user.id || user.is_admin
    }

    // Une méthode `-> Response` peut porter un message + un statut
    // HTTP en cas de refus.
    fn update(user: &User, post: &Post) -> Response {
        if post.author_id == user.id || user.is_admin {
            Response::allow()
        } else {
            Response::deny_with("You may only edit your own posts.")
        }
    }
    fn delete(user: &User, post: &Post) -> Response {
        if user.is_admin {
            Response::allow()
        } else {
            Response::deny_as_not_found() // cache le post aux non-admins
        }
    }
}
```

Chaque méthode devient un `inventory::submit!`. `Server::serve` vide
l'inventaire via `init_policies()` au démarrage, si bien qu'au moment
où la première requête arrive, chaque action est enregistrée (voir
[Amorçage de l'application](bootstrap.md) pour l'endroit où cela
s'insère dans la séquence de démarrage). `init_policies()` vit à
`suprnova::authorization::init_policies` et est idempotent - appelez-la
manuellement dans les tests qui exercent l'enregistrement de policy
sans faire tourner un serveur.

Les méthodes de policy sont des fonctions associées sans état
prenant `(user, resource)` - la même forme que le `update(User $user,
Post $post)` de Laravel, où `$this` est l'objet policy sans état.
Chaque méthode prend les deux arguments pour une signature de gate
uniforme ; `view_any` / `create` ignorent simplement la ressource
(`_post`). Les méthodes que vous n'écrivez pas ne sont pas
enregistrées, et une action non enregistrée refuse par défaut.

### Correspondance nom de méthode → action

Le nom de la méthode est utilisé directement comme segment verbe de
l'action, avec la ressource en kebab-case en suffixe :

| Méthode | Action |
|---|---|
| `view` sur `Post` | `"view-post"` |
| `view_any` sur `Post` | `"view_any-post"` |
| `force_delete` sur `UserProfile` | `"force_delete-user-profile"` |

Cela diverge des noms d'action en camelCase de Laravel (`viewAny`,
`forceDelete`) pour garder la surface Rust idiomatique - chaque
chaîne d'action reflète l'identifiant de méthode que vous
autocomplèteriez dans votre éditeur.

### Type de retour : `bool` ou `Response`

Le type de retour d'une méthode de policy choisit comment elle
s'enregistre - et ce qu'un refus peut porter :

| Type de retour | S'enregistre via | Le refus remonte comme |
|---|---|---|
| `bool` | `Gate::define` | un `403` nu (`This action is unauthorized.`) |
| `Response` | `Gate::define_with` | le message, le code, et le statut HTTP que porte le `Response` |

Retournez `bool` pour un oui/non simple. Retournez un `Response`
(importé depuis `suprnova::authorization::Response`) quand un refus
doit porter une raison ou un statut autre que 403 -
`Response::deny_with("…")` pour un message, ou
`Response::deny_as_not_found()` pour répondre `404` et cacher
l'existence de la ressource. Les deux compilent vers le même gate
effacé de son type (un `bool` est enveloppé dans un allow/deny nu).
Tout autre type de retour - ou son absence - est une erreur de
compilation.

## Le trait `Authorizable`

Sucre syntaxique côté utilisateur, prêt à brancher, pour les appels
`Gate` :

```rust
use suprnova::Authorizable;

impl Authorizable for User {}

// Sucre sync
if alice.can("update", &post)    { /* ... */ }
if alice.cannot("delete", &post) { /* ... */ }
alice.authorize("update", &post)?;  // 403 en cas de refus

// Sucre async
if alice.can_async("publish", &post).await    { /* ... */ }
alice.authorize_async("publish", &post).await?;
```

Chaque méthode a un corps par défaut qui délègue à la méthode `Gate`
correspondante, si bien qu'`impl Authorizable for User {}` (sans
corps) suffit. C'est de l'opt-in plutôt qu'un blanket-impl : tous les
types passables à `Gate::allows` ne sont pas censés être le sujet de
`.can` - le plus souvent, c'est le `User` de votre application.

## Motifs de composition

### Filtrer des groupes de routes

```rust
use suprnova::{group, get, Auth, AuthMiddleware, FrameworkError, Request, Response};

// Le middleware vérifie l'utilisateur authentifié ; le handler
// autorise l'action.
group!("/posts")
    .middleware(AuthMiddleware::new())
    .routes([
        get!("/{id}/edit", edit_form),
    ]);

async fn edit_form(req: Request) -> Response {
    let user: User = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let post = Post::find(id).await?
        .ok_or_else(|| FrameworkError::not_found("Post"))?;
    user.authorize("update", &post)?;
    // ... rend le formulaire d'édition
}
```

### Vérifications multi-actions

Une page « liste tout ce que cet utilisateur peut faire sur cette
ressource » :

```rust
let actions = ["view", "update", "delete", "restore", "force_delete"];
let mut allowed = Vec::new();
for action in &actions {
    if user.can(action, &post) {
        allowed.push(*action);
    }
}
// Ou court-circuitez :
let can_do_anything = Gate::any(&actions, &user, &post);
let is_locked_out   = Gate::none(&actions, &user, &post);
```

### Autorisation multi-gate

```rust
// N'autorise que si l'utilisateur peut faire TOUTES ces actions sur
// la ressource.
Gate::authorize_async("publish", &user, &post).await?;
if Gate::check_async(&["update", "view"], &user, &post).await {
    // Combine les vérifications.
}
```

### Filtrer des routes de ressource

Quand une surface `Router::resource` existe,
`authorize_resource::<U, R>()` câble la vérification d'ability
conventionnelle sur les sept routes à la fois, si bien que vous ne
dépendez pas du bon vouloir de chaque méthode de contrôleur pour
autoriser :

```rust
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()   // index/show→view, store→create, …
    .into();
```

Une ability refusée retourne `403` avant que le handler ne s'exécute ;
une requête non authentifiée échoue de manière fermée. La table
complète action → ability se trouve dans le [chapitre sur le
routage](routing.md).

## Sémantique async

La fermeture de `Gate::define_async` doit retourner une future
**owned** - le registre effacé de son type ne peut pas laisser des
références `&user` ou `&resource` survivre au-delà du retour de la
fermeture. Copiez ou clonez les champs dont vous avez besoin à
l'intérieur du bloc `async move {}` avant de le retourner :

```rust
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_id = user.id;        // copie un type primitif
    let post_id = post.id;
    let admin   = user.is_admin;
    async move {
        // Aucune référence `user` / `post` ici - seulement les
        // copies capturées.
        admin || check_can_publish(user_id, post_id).await
    }
});
```

Les gates sync fonctionnent de manière transparente depuis le chemin
async (`Gate::allows_async` les distribue sans `.await`), si bien
qu'une base de code peut enregistrer des gates sync aujourd'hui et
migrer des abilities individuelles vers async plus tard sans changer
les sites d'appel.

## Posture en cas de verrou empoisonné

Le registre `Gate` utilise un `RwLock` en interne. Si le verrou venait
à être empoisonné (un thread a paniqué en détenant la garde
d'écriture), le registre **refuse par sécurité** - chaque appel
`authorize` ultérieur retourne `Unauthorized` plutôt que de paniquer.
Les appels d'enregistrement journalisent via `tracing::error!` et
continuent. Cela correspond à la politique plus large du framework :
un verrou empoisonné n'interrompt jamais le processus.

## Décisions riches : `Response`, `inspect`, `raw`

Un gate `bool` nu ne répond que par autoriser ou refuser. Pour un refus qui porte
un *message*, un *code* machine ou un *statut* HTTP autre que 403,
enregistrez le gate avec `define_with` (ou `define_async_with`) et
retournez un `Response` :

```rust
use suprnova::authorization::Response;  // réexporté à la racine du crate sous le nom `GateResponse`

Gate::define_with::<User, Post>("update", |user, post| {
    if post.author_id == user.id {
        Response::allow()
    } else {
        Response::deny_with("You do not own this post.")
    }
});

// Cache l'existence d'une ressource plutôt que d'admettre qu'elle existe :
Gate::define_with::<User, Secret>("view", |user, secret| {
    if user.can_see(secret) {
        Response::allow()
    } else {
        Response::deny_as_not_found()  // un 404, pas un 403
    }
});
```

Inspectez la décision complète avec `Gate::inspect` (sync) /
`Gate::inspect_async` :

```rust
let decision = Gate::inspect("update", &user, &post);
decision.allowed();   // bool
decision.message();   // Option<&str> - Some("You do not own this post.")
decision.status();    // Option<u16> - None ici ; Some(404) après deny_as_not_found
```

Les constructeurs de `Response` reflètent Laravel : `allow()`, `deny()`,
`deny_with(msg)`, `deny_with_status(status, msg)`, `deny_as_not_found()`,
plus les builders `with_message` / `with_code` / `with_status` /
`as_not_found`.

### Comment un refus devient une erreur

`Gate::authorize` réduit la décision à travers `Response::authorize()` :

| Décision | Résultat d'`authorize` |
|---|---|
| autorisée | `Ok(())` |
| `deny()` nu (sans message/code/statut) - ce vers quoi retombe une réponse de refus par défaut non configurée | `FrameworkError::Unauthorized` (403, `"This action is unauthorized."`) |
| refus riche (message et/ou statut renseigné) - y compris une réponse de refus par défaut configurée qui en porte un | `FrameworkError::Domain { message, status_code }` |

Ainsi `deny_as_not_found()` remonte en 404, `deny_with_status(422, "…")`
en 422, et `deny_with("…")` en 403 portant votre message. Le `code` se
lit sur le `Response` inspecté mais ne voyage **pas** à travers
`authorize` - `FrameworkError` n'a pas de champ code ; lisez-le depuis
`inspect()` si vous en avez besoin.

Quel que soit le statut sur lequel un refus atterrit, il parvient au
client sous la forme du corps d'erreur JSON du framework. Une application
Inertia devrait aussi nommer une
[page d'erreur](frontend-inertia-responses.md#error-pages) - sans elle,
le client Inertia traite ce corps comme une réponse non Inertia et
affiche sa modale d'erreur plein écran au lieu de rendre quoi que ce
soit : un utilisateur au mauvais rôle voit alors un plantage plutôt qu'un
« vous n'avez pas le droit de faire cela ».

### `raw` : « refusé » vs « indéfini »

`Gate::raw` (et `raw_async`) retourne `Option<Response>` : `None`
signifie *qu'aucune règle ne s'est appliquée* - aucun hook `before` ne
s'est déclenché, aucun gate n'est enregistré, aucun hook `after` n'est
venu compléter -, par opposition à un `Some(deny)` explicite. `inspect`
normalise ce `None` vers la réponse de refus par défaut configurée (un
refus nu tant que `Gate::default_denial_response` n'a rien défini
d'autre) ; `raw` préserve le `None` pour le diagnostic (« cette action
est-elle seulement encadrée ? »).

### Réponse de refus par défaut

Le `Gate::defaultDenialResponse($response)` de Laravel remodèle
l'apparence d'un refus *indécis* - pas tous les refus, seulement ceux qui
retomberaient sinon sur le `Response::deny()` nu. Définissez-le une fois,
généralement dans `bootstrap::register()` :

```rust
use suprnova::authorization::Response;
use suprnova::Gate;

Gate::default_denial_response(Response::deny_as_not_found());
```

Après cet appel, deux sortes de résultats prennent la nouvelle forme : un
`false` nu - venu d'un gate booléen (`define`/`define_async`, y compris
une méthode `#[policy]` qui retourne `bool`), ou d'un hook
`before`/`after` qui a décidé `false` - et une évaluation que rien
d'autre n'a tranchée : une ability indéfinie sur laquelle aucun hook n'a
d'avis non plus. Tout cela remontait auparavant en `Response::deny()` nu
(un 403) ; désormais, cela remonte sous la forme donnée à
`default_denial_response` - un 404 dans l'exemple ci-dessus. C'est le
geste classique « cacher l'existence de la ressource à un utilisateur qui
n'a pas le droit de la voir » (voir l'exemple `Secret` plus haut dans ce
chapitre), appliqué une fois pour toute l'application au lieu de gate par
gate.

La valeur par défaut ne s'applique qu'au **`false` nu**. Un gate
enregistré avec `define_with` (ou `define_async_with`) a déjà retourné le
`Response` qu'il voulait - `Response::deny_with("…")`,
`Response::deny_as_not_found()`, voire un `Response::deny()` nu explicite -
et chacun d'eux traverse `inspect` intact. Cela reflète la
règle de Laravel elle-même : `Gate::inspect` ne substitue la valeur par
défaut qu'à un résultat de callback réellement falsy, jamais à un objet
`Response` que le callback a construit lui-même.

## Hooks `before` / `after`

`Gate::before` enregistre une vérification qui s'exécute *avant* tout
gate ; le premier hook à retourner `Some(decision)` court-circuite tout
le reste. L'usage canonique est une surcharge globale :

```rust
// Les administrateurs peuvent tout faire.
Gate::before::<User>(|user, _action| user.is_admin.then_some(true));
```

`Gate::after` s'exécute *après* le gate. Suivant la sémantique `??=` de
Laravel, un hook after ne peut que **compléter** un résultat indécis
(aucun gate n'a correspondu et aucun hook before ne s'est déclenché) ; il
ne peut jamais écraser un allow/deny déjà produit. Chaque hook after
s'exécute quand même, ce qui en fait aussi la couture pour la
journalisation d'audit :

```rust
Gate::after::<User>(|user, action, decided| {
    audit_log(user.id, action, decided);   // observe chaque évaluation
    None                                    // enregistre seulement ; ne change pas le résultat
});
```

Les hooks sont indexés par le **type d'utilisateur** `U`, pas par
ressource : un hook se déclenche pour chaque `(action, U, R)`. Placez la
logique propre à une ressource dans le gate. Les hooks sont des prédicats
synchrones et s'appliquent aussi au chemin d'évaluation async ; pour une
logique d'autorisation asynchrone, utilisez `define_async` /
`define_async_with`.

### Pourquoi Suprnova diverge

Le `Gate::forUser($user)->allows(...)` de Laravel relie le résolveur
d'utilisateur courant *implicite* du gate à un autre utilisateur, si bien
que la vérification suivante s'évalue en tant que celui-ci. Le gate de
Suprnova prend l'utilisateur **explicitement** à chaque appel :
« vérifier en tant qu'un autre utilisateur » se réduit donc à
`Gate::allows(action, &other_user, &resource)`. Il n'y a aucun résolveur
implicite à relier ailleurs ; l'API explicite est strictement plus
générale, ce qui rend `forUser` redondant plutôt que manquant.

Le même raisonnement vaut pour la découverte automatique des policies par
nom de classe chez Laravel. Suprnova lie les méthodes de policy à la clé
`(action, U, R)` effacée de son type au moment de l'enregistrement, si
bien qu'une policy `Post` et une policy `Comment` portant le même nom de
méthode enregistrent deux gates distincts, sans convention de nommage ni
balayage de découverte.

`Gate::default_denial_response` diverge aussi de Laravel sur un point :
lui passer un `Response::allow()` de forme autorisante est journalisé et
ignoré plutôt qu'accepté. Le `defaultDenialResponse` de Laravel n'a pas
ce garde-fou, mais il s'agit ici d'une valeur par défaut de *refus* : en
accepter une de forme autorisante inverserait silencieusement tout
résultat de gate `false` nu en autorisation, la seule direction
fail-open de cette surface.

## Suivant

- [Authentification](authentication.md) - la moitié côté utilisateur :
  guards, `Auth::user()`, `Auth::user_as::<T>()`
- [Amorçage de l'application](bootstrap.md) - où `init_policies()`
  s'exécute dans la séquence de démarrage, plus comment enregistrer
  des hooks before/after
- [Middleware](middleware.md) - associer `AuthMiddleware` à
  l'autorisation au niveau des routes
- [Modèle d'erreur](error-model.md) - comment un refus de gate se
  réduit en un 403, un 404, ou un `FrameworkError::Domain` à statut
  personnalisé
- [Événements](events.md) - écouter les résultats de policy via
  `Gate::after` pour la journalisation d'audit

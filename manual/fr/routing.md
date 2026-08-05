# Routage

Le routage est la façon dont Suprnova transforme une requête HTTP
entrante en un appel de handler. Vous déclarez vos routes dans
`src/routes.rs` avec la macro `routes!` (ou construisez un `Router` à la
main), puis `Server::from_config` prend ce routeur et l'exécute pendant
toute la durée de vie du processus. Même forme que le `routes/web.php`
de Laravel, avec des types Rust à la place des façades.

```rust
// src/routes.rs
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
    put!("/users/{id}", controllers::users::update).name("users.update"),
    delete!("/users/{id}", controllers::users::destroy).name("users.destroy"),
}
```

La macro se développe en `pub fn register() -> Router { ... }`.
Appelez-la depuis votre bootstrap et transmettez le résultat au serveur.

## Verbes HTTP

Une macro par verbe. Les sept prennent une paire chemin-puis-handler et
retournent un builder sur lequel vous pouvez chaîner `.name(...)` et
`.middleware(...)`.

| Macro | Méthode | Utilisation |
|---|---|---|
| `get!`     | GET     | Points de terminaison de lecture, pages statiques |
| `post!`    | POST    | Créer des ressources |
| `put!`     | PUT     | Mises à jour par remplacement complet |
| `patch!`   | PATCH   | Mises à jour partielles (RFC 5789) |
| `delete!`  | DELETE  | Détruire |
| `head!`    | HEAD    | Sondes headers-only (HEAD retombe sur le registre GET selon la RFC 9110 § 9.3.2 quand elle n'est pas enregistrée explicitement) |
| `options!` | OPTIONS | Découverte de capacités, `Accept-Patch`. Le préflight CORS reçoit sa réponse de `CorsMiddleware` avant le routeur, donc vous n'en avez généralement pas besoin |

```rust
use suprnova::{routes, get, post, patch, delete};

routes! {
    get!("/articles", controllers::articles::index),
    post!("/articles", controllers::articles::store),
    patch!("/articles/{id}", controllers::articles::update),
    delete!("/articles/{id}", controllers::articles::destroy),
}
```

Chaque macro de verbe vérifie à la compilation que le chemin commence
par `/` - une barre oblique de début manquante fait échouer la
compilation, pas une requête.

### Multi-méthode et `any!`

`any!` enregistre un seul handler pour les sept verbes courants.
Utilisez-le pour les récepteurs de webhook et les autres points de
terminaison qui doivent accepter tout ce que HTTP envoie.

```rust
use suprnova::{routes, any};

routes! {
    any!("/webhooks/inbound", controllers::webhooks::inbound)
        .name("webhooks.inbound")
        .middleware(SignatureCheck),
}
```

Quand vous ne voulez qu'un sous-ensemble de verbes partageant un seul
handler, tournez-vous vers l'API builder et `Router::methods` :

```rust
use suprnova::Router;
use hyper::Method;

let router = Router::new()
    .methods(&[Method::PUT, Method::PATCH], "/posts/{id}", update_post)
    .name("posts.update")
    .middleware(AuthMiddleware);
```

`.name(...)` et `.middleware(...)` se propagent à travers tous les
verbes pour lesquels la route a été enregistrée, donc la recherche
inverse produit la même URL quelle que soit la méthode que l'appelant
recherche.

### Routes WebSocket

`ws!` enregistre un handler de mise à niveau longue durée. La macro fait
partie du même corps `routes!` - couverte en détail par
[WebSockets](websockets.md).

## Paramètres de route

Les segments dynamiques utilisent des accolades (`{id}`). Par souci de
familiarité, Suprnova accepte aussi les deux-points façon Express/Rails
(`:id`) et les normalise en accolades avant de transmettre le motif à
`matchit`.

```rust
routes! {
    get!("/users/{id}", controllers::users::show),       // natif matchit
    get!("/users/:id", controllers::users::show),        // Express/Rails - la même chose
    get!("/posts/{post_id}/comments/{comment_id}", controllers::comments::show),
}
```

Les deux-points ne sont traités comme ouvreur de paramètre qu'en début
de segment de chemin, donc les deux-points littéraux au milieu d'un
segment survivent intacts (`/files/note:draft` reste une route
littérale, pas `/files/{draft}`).

Lisez les paramètres depuis la requête à l'intérieur d'un handler :

```rust
use suprnova::{Request, Response, HttpResponse};

pub async fn show(req: Request) -> Response {
    let user_id = req.param("id").unwrap_or("0");
    Ok(HttpResponse::text(format!("User ID: {}", user_id)))
}
```

Pour une extraction typée sans la gymnastique `unwrap_or`, voir la
liaison de modèle de route ci-dessous ou `#[handler]` dans
[Contrôleurs](controllers.md).

## Liaison de modèle de route

Quand un paramètre de handler est un type `*::Model` de SeaORM,
`#[handler]` extrait le paramètre de chemin correspondant, l'analyse
comme le type de clé primaire, et récupère la ligne depuis la base de
données. Une ligne absente donne 404 ; un paramètre que le type de PK ne
peut pas analyser donne 400.

```rust
use suprnova::{handler, json_response, Response};
use crate::models::users;

// Route : GET /users/{user}
#[handler]
pub async fn show(user: users::Model) -> Response {
    json_response!({ "name": user.name, "email": user.email })
}
```

Le nom du paramètre (`user`) est ce que `#[handler]` recherche dans les
params de la route correspondante - le placeholder doit donc
correspondre (`/users/{user}`, pas `/users/{id}`).

Plusieurs modèles dans une même signature fonctionnent de la même
façon ; mélangez-les avec des form requests, des primitives, ou
`Request` :

```rust
// Route : PUT /posts/{post}/comments/{comment}
#[handler]
pub async fn update(
    post: posts::Model,
    comment: comments::Model,
    form: UpdateCommentRequest,
) -> Response {
    // post et comment sont déjà récupérés ; form est validé.
    json_response!({ "post_id": post.id, "comment_id": comment.id })
}
```

### Prérequis

La liaison est automatique pour tout modèle SeaORM dont l'`Entity`
implémente `suprnova::database::EntityExt` et dont le type de clé
primaire implémente `FromStr`. Les traits d'extension généreux
d'`EntityExt` vous donnent `Entity::find_by_pk(id)`, `::all()`,
`::first()`, et consorts ; la liaison de modèle de route n'est jamais
que `find_by_pk` piloté par le paramètre de chemin.

```rust
// src/models/users.rs (la disposition historique façon SeaORM)
pub use super::entities::users::*;
use sea_orm::entity::prelude::*;

impl ActiveModelBehavior for ActiveModel {}

// Active la liaison de modèle de route (et la surface de lecture façon Laravel).
impl suprnova::database::EntityExt for Entity {}
impl suprnova::database::EntityExtMut for Entity {}
```

Si votre modèle est déclaré avec la macro `#[suprnova::model]` (la
surface Eloquent dans [Eloquent](eloquent.md)), vous y accédez
directement : `User::find_by_pk(id).await?`. La liaison de modèle de
route via `#[handler]` attend toujours la forme `*::Model` - passez le
type de modèle SeaORM, pas la struct wrapper.

### La liaison est une question d'identité, pas d'autorisation

La liaison de modèle de route répond à « cette ligne existe-t-elle ? » -
elle ne répond **pas** à « l'utilisateur courant est-il autorisé à voir
cette ligne ? ». Un handler lié tel quel laisse n'importe quel
utilisateur authentifié consulter n'importe quel post en devinant
`/posts/N`. Autorisez l'accès au modèle lié avec `Gate::authorize` ou la
macro `#[policy]` - voir [Autorisation](authorization.md).

### S'en passer

N'utilisez pas le type de paramètre `*::Model`. Extrayez l'ID et faites
la requête manuellement :

```rust
use suprnova::{handler, json_response, Response, FrameworkError};
use crate::models::users;
use suprnova::database::EntityExt;

#[handler]
pub async fn show(id: i32) -> Response {
    let user = users::Entity::find_by_pk(id)
        .await?
        .ok_or(FrameworkError::not_found("User"))?;
    json_response!({ "id": user.id, "name": user.name })
}
```

## Routes nommées

Les noms vous donnent des identifiants stables pour la génération d'URL.
Attachez-en un avec `.name(...)` :

```rust
routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

Les noms suivent la convention Laravel `<resource>.<action>` -
`users.show`, `posts.destroy`, `admin.dashboard`. Recherchez-les avec le
helper de premier niveau `route(name, &[...])` :

```rust
use suprnova::route;

let home = route("home", &[]);
//   Some("/")

let profile = route("users.show", &[("id", "123")]);
//   Some("/users/123")
```

`route` retourne `Option<String>` et encode en pourcentage les valeurs
de paramètre sous une forme sûre pour un chemin (donc `("slug", "a/b")`
devient `/posts/a%2Fb` - sûr pour matchit et fait l'aller-retour via
`req.param("slug")`). Pour les cibles de redirection et les liens
d'e-mail, utilisez l'homologue strict `suprnova::routing::try_route`,
qui retourne `Result<String, RouteUrlError>` et refuse d'émettre une URL
contenant un segment `{placeholder}` non rempli. Voir [Génération
d'URL](urls.md) pour la surface complète des URL (URL signées, URL
absolues, `Redirect::route`).

Les noms de route sont globalement uniques, à l'échelle du processus.
Enregistrer le même nom pour deux chemins différents panique à
l'amorçage - le shadowing silencieux était un bug à forme de sécurité,
car les redirections routaient vers l'enregistrement qui avait gagné,
quel qu'il soit. Utilisez `RouteBuilder::try_name` (ou
`suprnova::routing::try_register_route_name`) pour la variante
faillible.

## Middleware par route

Chaînez `.middleware(M)` sur n'importe quel builder de route :

```rust
use suprnova::{routes, get, post};
use crate::middleware::{AuthMiddleware, AdminMiddleware};

routes! {
    // Public
    get!("/", controllers::home::index).name("home"),

    // Protégé
    get!("/dashboard", controllers::dashboard::index)
        .name("dashboard")
        .middleware(AuthMiddleware),

    // Plusieurs middleware se composent de gauche à droite (le plus extérieur en premier)
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware)
        .middleware(AdminMiddleware),
}
```

Le middleware local à la route s'exécute après tout middleware global
(`Server::with_middleware`) et tout middleware de groupe qui enveloppe
la route. La map de middleware est indexée par `(method, path)`, donc
attacher l'authentification à `POST /api/posts` ne déborde jamais sur un
`GET /api/posts` public au même chemin. Pour le contrat du middleware et
pour écrire le vôtre, voir [Middleware](middleware.md).

## Groupes de routes

`group!` factorise un préfixe de chemin partagé et/ou un middleware
partagé :

```rust
use suprnova::{routes, get, post, group};
use crate::middleware::{AuthMiddleware, ApiMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Préfixe /api partagé + middleware
    group!("/api", {
        get!("/users", controllers::api::users::index).name("api.users.index"),
        post!("/users", controllers::api::users::store).name("api.users.store"),
        get!("/users/{id}", controllers::api::users::show).name("api.users.show"),
    }).middleware(ApiMiddleware),

    // Zone admin
    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard).name("admin.dashboard"),
        get!("/settings", controllers::admin::settings).name("admin.settings"),
    }).middleware(AuthMiddleware),
}
```

Un préfixe de groupe est concaténé avec le chemin de chaque route. Une
route sur `/` à l'intérieur d'un groupe se résout exactement au préfixe
du groupe (`group!("/users", { get!("/", index) })` → `GET /users`).

### Groupes imbriqués

Les groupes s'imbriquent à n'importe quelle profondeur. Les préfixes se
concatènent ; le middleware s'hérite du parent vers l'enfant :

```rust
routes! {
    group!("/api", {
        get!("/health", controllers::api::health),

        group!("/v1", {
            get!("/users", controllers::api::v1::users),

            group!("/admin", {
                get!("/stats", controllers::admin::stats),
            }).middleware(AdminMiddleware),
        }),
    }).middleware(AuthMiddleware),
}
```

| Route | Chemin effectif | Chaîne de middleware |
|---|---|---|
| `/api/health` | `/api/health` | `AuthMiddleware` |
| `/api/v1/users` | `/api/v1/users` | `AuthMiddleware` |
| `/api/v1/admin/stats` | `/api/v1/admin/stats` | `AuthMiddleware` → `AdminMiddleware` |

Pour une route unique à l'intérieur d'un groupe imbriqué, l'ordre
d'exécution est **le middleware le plus extérieur en premier** : groupe
parent → groupe enfant → local à la route. Le `.middleware(...)` par
route s'exécute en dernier, au plus profond.

## Route de repli

`fallback!` enregistre un handler qui s'exécute quand aucune autre route
ne correspond. Utilisez-le pour des pages 404 personnalisées.

```rust
use suprnova::{routes, get, fallback};

routes! {
    get!("/", controllers::home::index),

    fallback!(controllers::errors::not_found),
}
```

```rust
// src/controllers/errors.rs
use suprnova::{Request, Response, HttpResponse};

pub async fn not_found(req: Request) -> Response {
    Ok(HttpResponse::text(format!("Page not found: {}", req.path()))
        .status(404))
}
```

Fallback prend en charge sa propre chaîne de middleware
(`fallback!(handler).middleware(M)`). Si aucun fallback n'est
enregistré, le framework retourne un `404 Not Found` en texte brut.

## Routage de ressource

Pour une surface REST standard à 7 actions, implémentez
`ResourceController` et enregistrez la ressource via le builder
`Router`. Parité Laravel pour `Route::resource()` et
`Route::apiResource()`.

```rust
use suprnova::{Router, ResourceController, ResourceAction, Request, Response, HttpResponse};
use std::pin::Pin;
use std::future::Future;

struct PostsCtl;

impl ResourceController for PostsCtl {
    fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("list")) })
    }
    fn show(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("one")) })
    }
    // store / update / destroy / create / edit retombent sur 404 par défaut.
}

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .into();
```

Les méthodes que vous ne redéfinissez pas retournent 404. Utilisez
`api_resource` pour supprimer `create` et `edit` - les deux routes qui
n'existent que pour rendre des formulaires.

### Routes et noms par défaut

| Verbe | Chemin | Méthode du trait | Nom |
|---|---|---|---|
| GET    | `/posts`             | `index`   | `posts.index`   |
| GET    | `/posts/create`      | `create`  | `posts.create`  |
| POST   | `/posts`             | `store`   | `posts.store`   |
| GET    | `/posts/{post}`      | `show`    | `posts.show`    |
| GET    | `/posts/{post}/edit` | `edit`    | `posts.edit`    |
| PUT    | `/posts/{post}`      | `update`  | `posts.update`  |
| DELETE | `/posts/{post}`      | `destroy` | `posts.destroy` |

Le paramètre de chemin prend par défaut le singulier du nom de la
ressource - `posts` → `{post}`, `categories` → `{category}`. Les
pluriels irréguliers récupèrent le dernier segment tel quel ;
redéfinissez avec `.parameter(...)`.

### Restreindre et renommer

```rust
use suprnova::{Router, ResourceAction};

Router::new()
    .resource("posts", PostsCtl)
    .only(&[ResourceAction::Index, ResourceAction::Show])      // se limite à deux verbes
    .names([("index", "posts.list")])                          // renomme un défaut
    .parameter("post_id")                                      // {post} → {post_id}
    .into();
```

Des alias côté Rust qui se lisent mieux dans certains sites d'appel :
`.keep(...)` pour `.only(...)`, `.drop(...)` pour `.except(...)`,
`.rename(...)` pour `.names(...)`.

### Enregistrement en masse

```rust
Router::new()
    .resources([
        ("posts",    Box::new(PostsCtl)    as Box<dyn ResourceController>),
        ("comments", Box::new(CommentsCtl) as Box<dyn ResourceController>),
    ])
    .api_resources([("authors", Box::new(AuthorsCtl) as Box<dyn ResourceController>)]);
```

### Autoriser la ressource entière

`authorize_resource::<U, R>()` attache la vérification d'ability
conventionnelle à chaque route générée en tant que middleware par route -
parité avec `authorizeResource` de Laravel. Sans cela, une surface de
ressource n'est pas protégée à moins que chaque corps de contrôleur ne
pense à appeler `Gate::authorize` ; un seul `destroy` oublié expédie une
suppression non protégée.

```rust
use suprnova::{Router, Gate};

// Les abilities sont indexées par (ability, type d'utilisateur, type marqueur de ressource).
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()
    .into();
```

Le mappage action → ability reflète celui de Laravel :

| Action(s) | Ability |
|---|---|
| `index`, `show`     | `view`   |
| `create`, `store`   | `create` |
| `edit`, `update`    | `update` |
| `destroy`           | `delete` |

`PATCH` partage l'action `update`, donc elle est protégée à l'identique
de `PUT`. Une ability refusée court-circuite avec `403` avant que le
handler ne s'exécute, et une requête non authentifiée échoue de façon
fermée. Le marqueur de ressource `R` n'a besoin que de `Default` - le
gate discrimine sur son *type*, de la même façon que Laravel discrimine
sur la classe du modèle. Voir le [chapitre sur l'autorisation](authorization.md)
pour la définition des abilities elles-mêmes.

## Redirections et vues au niveau du routeur

Trois méthodes de sucre syntaxique sur `Router` couvrent les
déclarations de route qui n'ont pas besoin de fonction handler :

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new()
    // Redirection statique : GET /old-pricing → 302 /pricing
    .redirect("/old-pricing", "/pricing", 302)
    // L'homologue en 301
    .permanent_redirect("/legacy", "/new")
    // Page statique Inertia : GET /about rend le composant About avec des props constantes
    .view("/about", "About", json!({ "team_size": 4 }));
```

`Router::view` est l'équivalent Suprnova du `Route::view($uri, $view,
$data)` de Laravel. Laravel rend un template Blade ; Suprnova rend un
composant Inertia, car le système de templates du framework est
Inertia, pas Blade.

Pour les *réponses* de redirection (pas les déclarations de route) -
`Redirect::route`, `Redirect::back`, `Redirect::intended`, les
redirections signées - voir [Génération d'URL](urls.md) et
[Réponses](responses.md).

## URL signées

Les routes signées par HMAC sont adjacentes au routage (vous générez
une URL pour une route nommée, puis vérifiez la signature sur la
requête entrante). Elles sont couvertes intégralement par [Génération
d'URL](urls.md) ; la version courte :

```rust
use suprnova::url;

let reset = url::signed_route("password.reset", &[("user", "42")])?;
// /password/reset/42?signature=...

let expires_at = chrono::Utc::now().timestamp() + 3600;
let verify = url::temporary_signed_route("verify.email", &[("user", "42")], expires_at)?;
// /verify/email/42?expires=1748803600&signature=...
```

Vérifiez à l'intérieur d'un handler avec
`url::has_valid_signature(&request)` (booléen) ou
`url::signature_verdict(&request)` (la répartition à trois voies
`Valid`/`Expired`/`Invalid`, qui vous permet de rendre une page
« demander un nouveau lien » plutôt qu'un 403 générique).

## Enregistrement faillible

L'enregistrement de route s'exécute une seule fois à l'amorçage, donc
une route dupliquée ou malformée est traitée comme une erreur de
programmeur : les helpers simples (`Router::get`, `post`, `put`,
`delete`, `ws`, `RouteBuilder::name`, la conversion `From` de
`GroupBuilder` → `Router`) **paniquent** pour échouer explicitement au
démarrage. C'est le bon défaut pour des routes déclarées dans le code
source.

Quand les motifs ou les noms proviennent d'une source faillible - config
dynamique, système de plugins, test qui enregistre délibérément des
routes en conflit - utilisez les homologues `try_*`. Ils retournent
`Result<_, FrameworkError>` (nommant la méthode, le chemin, ou le nom en
conflit) au lieu de paniquer :

| Panique | Homologue faillible | Retourne |
|---|---|---|
| `Router::get` / `post` / `put` / `patch` / `delete` / `head` / `options` | `try_get` / `try_post` / `try_put` / `try_patch` / `try_delete` / `try_head` / `try_options` | `Result<RouteBuilder, FrameworkError>` |
| `Router::ws` (et chaque variante `ws_*`) | `try_ws` (et chaque `try_ws_*`) | `Result<Router, FrameworkError>` |
| `RouteBuilder::name` | `try_name` | `Result<Router, FrameworkError>` |
| `GroupBuilder` → `Router` via `.into()` | `GroupBuilder::try_finalize` | `Result<Router, FrameworkError>` |
| `ResourceRoutes::register` | `try_register` | `Result<Router, FrameworkError>` |

```rust
use suprnova::{FrameworkError, Router};

// `path` provient d'une config dynamique ; un motif malformé ou dupliqué
// est récupérable, pas une panique au démarrage.
fn register_dynamic(router: Router, path: &str) -> Result<Router, FrameworkError> {
    Ok(router.try_get(path, health)?.into())
}
```

Une route de groupe dupliquée est récupérable de la même façon - comme
`From` ne peut pas être faillible, le pendant faillible de `.into()` est
la méthode inhérente `try_finalize` :

```rust
let router: Router = Router::new()
    .group("/api", |r| r.get("/users", list).post("/users", create))
    .try_finalize()?;
```

Les helpers qui paniquent restent des échappatoires ergonomiques ; les
homologues `try_*` sont purement additifs.

## Pourquoi Suprnova diverge

**Double syntaxe de paramètre de chemin.** Laravel utilise `{param}` ;
Express utilise `:param`. Suprnova accepte les deux et normalise
`:param` en `{param}` avant que le chemin n'atteigne `matchit`. Les deux
styles se composent avec tout le reste - groupes, liaison de modèle, URL
signées. La raison n'est pas l'indécision ; c'est que nous ne pouvons
pas prédire quel bagage vous apportez, et la syntaxe de routage est un
point de friction bien trop fréquent pour faire réapprendre les gens.

**Deux API à égalité : macro et builder.** Laravel livre un seul DSL
(`Route::get(...)`). Suprnova livre à la fois la macro déclarative
`routes! { ... }` ET le builder chaînable
`Router::new().get(...).name(...)`. Ils produisent des enregistrements
identiques. La macro se lit mieux pour les tables de routes de premier
niveau ; le builder se lit mieux quand vous composez des routeurs
dynamiquement (plugins, routes générées, tests). Choisissez ce qui
convient au site d'appel - il n'y a pas de réponse canonique car les
deux formes sont de première classe.

**Paniques à l'amorçage, pas de shadowing silencieux.** Un nom de route
dupliqué ou une collision de motif panique au démarrage. Les registres
indexés par tableau de Laravel laissent silencieusement l'enregistrement
le plus tardif l'emporter, ce qui convient tant que votre fichier de
routes est le seul registrar, mais devient dangereux dès que des
plugins ou des routes générées entrent en jeu. Les homologues `try_*`
sont l'échappatoire quand la faillibilité est réellement ce que vous
voulez.

## Suivant

- [Contrôleurs](controllers.md) - `#[handler]`, form requests, retourner du JSON/Inertia
- [Middleware](middleware.md) - le trait `Middleware`, l'ordonnancement, construire le vôtre
- [Génération d'URL](urls.md) - URL de route nommée, URL signées, redirections, `RouteUrlError`
- [Autorisation](authorization.md) - gates et policies pour les modèles liés
- [WebSockets](websockets.md) - `ws!`, le trait `WebSocketHandler`, la config par route

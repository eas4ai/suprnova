# Contrôleurs

Un contrôleur Suprnova n'est qu'une fonction async. Elle prend dans la
requête ce dont elle a besoin - paramètres de chemin typés, un modèle
chargé, un formulaire validé - et retourne une `Response`. Il n'y a pas
de classe de base de contrôleur. Il n'y a pas de fichier de câblage à la
service-locator. La fonction est l'unité, et l'attribut `#[handler]` la
colle aux macros de routage.

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// GET /users/{user}
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

La signature de ce handler fait trois choses à la fois : elle déclare le
paramètre de route (`user`), tire la ligne de la base de données, et
retourne un 404 si la ligne n'y est pas. Rien de tout cela n'est écrit à
la main. `#[handler]` lit les types des arguments et génère
l'extraction.

## Générer un contrôleur

```bash
suprnova make:controller User
```

Cela écrit `src/controllers/user.rs` avec un unique stub `invoke` et
ajoute `pub mod user;` à `src/controllers/mod.rs`. Le stub est le
handler minimal viable :

```rust
//! User controller

use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn invoke(_req: Request) -> Response {
    json_response!({
        "controller": "User"
    })
}
```

Ajoutez autant de fonctions que vous voulez dans le fichier - Suprnova
ne suit pas de « classes » de contrôleur, seulement des fonctions.
Beaucoup d'applications découpent par ressource
(`controllers::user::{index, show, store, update, destroy}`), mais rien
dans le framework ne l'impose.

Le nom est converti en `snake_case` pour le nom de fichier :
`OrderItem` devient `order_item.rs`.

## L'attribut `#[handler]`

La macro classe le type de chaque paramètre et génère l'extracteur
correspondant. Quatre catégories :

| Type de paramètre | Extrait via | Mode d'échec |
|---|---|---|
| `Request` | transmet la requête telle quelle | - |
| `i32`, `i64`, `u32`, `u64`, `usize`, `String` | `FromParam` - analyse le param de route du même nom | 400 si l'analyse échoue, 400 s'il est absent |
| `T: AutoRouteBinding` (tout `Model` Eloquent) | analyse le param comme la clé primaire du modèle, charge la ligne | 400 si l'analyse échoue, 404 si la ligne est introuvable |
| Tout le reste (`T: FromRequest`) | appelle `T::from_request(req)` - typiquement un validateur `#[derive(FormRequest)]` | ce que `from_request` retourne ; 422 pour les erreurs de validation |

La macro exécute les extractions dans l'ordre de déclaration, donc le
corps de votre fonction voit des valeurs entièrement typées. Si une
extraction échoue, l'erreur court-circuite via `?` et le corps du
handler ne s'exécute jamais.

### Paramètres de chemin

```rust
// Route : get!("/users/{id}", controllers::user::show)
#[handler]
pub async fn show(id: i64) -> Response {
    json_response!({ "user_id": id })
}

// Route : get!("/posts/{post_id}/comments/{comment_id}", show_comment)
#[handler]
pub async fn show_comment(post_id: i64, comment_id: i64) -> Response {
    json_response!({
        "post_id": post_id,
        "comment_id": comment_id,
    })
}
```

Le nom de l'argument doit correspondre au placeholder de la route :
`{id}` exige `id: …`. Le type de l'argument est analysé via `FromParam`.
Une entrée incorrecte (`/users/abc` face à `id: i64`) retourne 400 avec
un message qui nomme le paramètre et le type cible.

### Liaison de modèle de route

Les modèles `Eloquent` implémentent `AutoRouteBinding` automatiquement.
Déclarez le modèle comme argument et le framework le charge :

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// Route : get!("/users/{user}", controllers::user::show)
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

Le nom du placeholder de route (`{user}`) et le nom de l'argument
(`user`) doivent correspondre. Le framework analyse la chaîne du
paramètre comme le type de clé primaire du modèle, appelle
`Entity::find_by_pk`, et retourne 404 si la ligne est absente. Toute
struct `#[suprnova::model]` se lie automatiquement ; la macro
`route_binding!` reste disponible pour les entités SeaORM écrites à la
main qui n'utilisent pas `#[suprnova::model]` - voir
[Macros](macros.md#route_binding).

### Requêtes de formulaire

Tout ce qui implémente `FromRequest` se branche de la même façon. Le cas
courant est une struct `#[derive(FormRequest)]` qui valide le corps de
la requête et fait remonter un 422 avec des erreurs indexées par champ
en cas d'échec :

```rust
use suprnova::{attrs, handler, json_response, Response};
use crate::models::user;
use crate::requests::UpdateUserRequest;

// Route : put!("/users/{user}", controllers::user::update)
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! { name: form.name, email: form.email }).await?;
    json_response!({ "updated": id })
}
```

Voir [Requêtes de formulaire](requests.md) pour le derive du validateur
et le pipeline de validation complet.

### Quand vous voulez la `Request` brute

Si vous préférez extraire les choses à la main - ou s'il vous faut un
en-tête, un cookie, une chaîne de requête - prenez `Request`
directement :

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;             // param de route, 400 si absent
    let ua = req.header("User-Agent");      // Option<&str>
    let page: u32 = req.query_param("page") // Option<String>
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    json_response!({ "id": id, "ua": ua, "page": page })
}
```

Vous pouvez panacher : `pub async fn nested(category_id: i64, product: product::Model, req: Request)` est une signature valide. La macro extrait chaque argument selon sa propre règle.

## Le contrat `Response`

`Response` est un alias pour `Result<HttpResponse, HttpResponse>`. Les
deux branches portent le même type de charge utile, ce qui explique
pourquoi `?` fonctionne partout. La chaîne de middleware réduit le
résultat en une seule ligne, à la frontière :

```rust
result.unwrap_or_else(|e| e)
```

C'est le contrat sur lequel repose chaque point de propagation de `?`.
Les erreurs sont converties via `From<FrameworkError> for HttpResponse`
avant d'atteindre la chaîne - voir [Modèle d'erreur](error-model.md)
pour le tableau complet.

Le corps d'un handler se lit de haut en bas et utilise `?` pour
abandonner :

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

#[handler]
pub async fn show(id: i64) -> Response {
    let user = user::Model::find_or_fail(id).await?;
    let invoices = user.invoices().get().await?;
    json_response!({ "user": user, "invoices": invoices })
}
```

Si `find_or_fail` retourne `Err`, la fonction sort avec un 404. Si
`invoices().get()` échoue, vous obtenez un 500. Pas d'instructions
`match`, pas de handlers d'exception.

## Créer des réponses

Trois macros et un builder couvrent les cas courants :

```rust
use suprnova::{handler, json_response, text_response, HttpResponse, Response, ResponseExt};

#[handler]
pub async fn json_handler() -> Response {
    json_response!({
        "users": [
            {"id": 1, "name": "John"},
            {"id": 2, "name": "Jane"},
        ]
    })
}

#[handler]
pub async fn health() -> Response {
    text_response!("OK")
}

#[handler]
pub async fn store() -> Response {
    // Statut / en-têtes chaînables intégrés, via ResponseExt.
    json_response!({ "id": 1, "created": true }).status(201)
}

#[handler]
pub async fn page() -> Response {
    Ok(HttpResponse::html("<h1>Hello</h1>"))
}
```

`json_response!`, `text_response!` et `HttpResponse::*` produisent tous
le même type `Response`. Le trait `ResponseExt` ajoute `.status(...)`,
`.header(...)`, `.cookie(...)` et `.with_headers(...)` pour que vous
puissiez chaîner de la configuration sur le résultat d'une macro.

Pour tout le reste - téléchargements de fichiers, corps en streaming,
réponses Inertia, redirections - voir [Réponses](responses.md).

## Redirections

`redirect!("route.name")` vérifie à la compilation que la route existe
et retourne un builder sur lequel vous pouvez chaîner de la
configuration :

```rust
use suprnova::{handler, redirect, Response};

#[handler]
pub async fn store() -> Response {
    // Créer l'utilisateur…
    redirect!("users.index").into()
}

#[handler]
pub async fn update(id: i64) -> Response {
    redirect!("users.show")
        .with("id", id.to_string())
        .into()
}

#[handler]
pub async fn search() -> Response {
    redirect!("users.index")
        .query("page", "1")
        .query("sort", "name")
        .into()
}
```

`.with(key, value)` remplit un placeholder de route ; `.query(key,
value)` ajoute un paramètre de chaîne de requête ; `.flash(key, value)`
écrit dans le flash bag de la session pour la requête suivante.
`.into()` convertit le builder en `Response`.

Si la route nommée n'existe pas, la macro fait échouer la compilation
avec la liste des noms de route disponibles - les fautes de frappe
apparaissent avant le staging.

## Services injectés depuis le conteneur

Résolvez les services depuis le conteneur avec `App::resolve` (types
concrets) ou `App::resolve_make` (objets trait). Les deux retournent
`Result<_, FrameworkError>`, ils se composent donc avec `?` :

```rust
use suprnova::{handler, json_response, App, Response};
use crate::services::UserService;

#[handler]
pub async fn index() -> Response {
    let user_service = App::resolve::<UserService>()?;
    let users = user_service.list_all().await?;
    json_response!({ "users": users })
}
```

Si vous liez des actions avec `#[injectable]`, c'est ainsi qu'un
contrôleur les appelle. Voir [Actions](actions.md) pour la forme d'une
action, et [Conteneur de service](container.md) pour la surface
complète du conteneur - liaison, fabriques, cascade de recherche
task-local / thread-local / globale.

## Un contrôleur RESTful complet

```rust
// src/controllers/user.rs
use suprnova::{attrs, handler, json_response, redirect, Response, ResponseExt};
use crate::models::user;
use crate::requests::{StoreUserRequest, UpdateUserRequest};

// GET /users
#[handler]
pub async fn index() -> Response {
    let users = user::Model::all().await?;
    json_response!({ "users": users })
}

// GET /users/{user}
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({ "user": user })
}

// POST /users
#[handler]
pub async fn store(form: StoreUserRequest) -> Response {
    let user = user::Model::create(attrs! {
        name: form.name,
        email: form.email,
    }).await?;
    json_response!({ "user": user }).status(201)
}

// PUT /users/{user}
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! {
        name: form.name,
        email: form.email,
    }).await?;
    json_response!({ "updated": id })
}

// DELETE /users/{user}
#[handler]
pub async fn destroy(user: user::Model) -> Response {
    user.delete().await?;
    redirect!("users.index").into()
}
```

Enregistrez-les avec la macro `routes!` :

```rust
// src/routes.rs
use suprnova::{delete, get, post, put, routes};
use crate::controllers;

routes! {
    get!("/users",           controllers::user::index   ).name("users.index"),
    get!("/users/{user}",    controllers::user::show    ).name("users.show"),
    post!("/users",          controllers::user::store   ).name("users.store"),
    put!("/users/{user}",    controllers::user::update  ).name("users.update"),
    delete!("/users/{user}", controllers::user::destroy ).name("users.destroy"),
}
```

Le placeholder de route `{user}` correspond au nom de l'argument `user: user::Model`, et c'est ainsi que le framework sait quel segment de chemin charge le modèle.

## L'API `Request`

Les méthodes auxquelles vous ferez le plus souvent appel quand vous
prenez `Request` directement :

| Méthode | Retourne | Notes |
|---|---|---|
| `method()` | `&hyper::Method` | méthode HTTP |
| `path()` | `&str` | chemin de l'URL |
| `param(name)` | `Result<&str, ParamError>` | param de route ; `?` pour abandonner |
| `params()` | `&HashMap<String, String>` | tous les params de route |
| `query()` | `Option<&str>` | chaîne de requête brute |
| `query_param(key)` | `Option<String>` | une seule valeur de la chaîne de requête |
| `query_params()` | `HashMap<String, String>` | tous les params de requête |
| `query_into::<T>()` | `Result<T, FrameworkError>` | désérialisation typée |
| `header(name)` | `Option<&str>` | un seul en-tête |
| `headers()` | `&hyper::HeaderMap` | la map complète des en-têtes |
| `has_header(name)` | `bool` | vérification de présence |
| `bearer_token()` | `Option<String>` | `Authorization: Bearer …` analysé |
| `cookie(name)` | `Option<String>` | la valeur d'un seul cookie |
| `cookies()` | `HashMap<String, String>` | tous les cookies |
| `ip()` | `Option<String>` | IP du pair, tient compte de X-Forwarded-For |
| `secure()` | `bool` | détection de HTTPS (proxies compris) |
| `is_method(m)` | `bool` | insensible à la casse |
| `is_inertia()` | `bool` | en-tête XHR d'Inertia |
| `ajax()` | `bool` | `X-Requested-With: XMLHttpRequest` |
| `expects_json()` / `wants_json()` | `bool` | inspection de l'en-tête Accept |
| `route_name()` | `Option<String>` | le `.name(...)` de la route correspondante |
| `json::<T>()` | `Result<T, FrameworkError>` | analyse le corps comme du JSON (le consomme) |
| `form::<T>()` | `Result<T, FrameworkError>` | analyse comme du form-urlencoded |
| `input::<T>()` | `Result<T, FrameworkError>` | analyse dispatchée selon le content-type |

C'est une surface de forme Laravel - chaque méthode ici reflète une
méthode de la classe `Request` de Laravel.

## Disposition des fichiers

Convention :

```
src/
├── controllers/
│   ├── mod.rs          # pub mod home; pub mod user; ...
│   ├── home.rs
│   ├── user.rs
│   └── api/
│       ├── mod.rs
│       └── user.rs
├── routes.rs           # routes! { ... }
└── main.rs
```

Rien dans le framework n'impose cette disposition - les contrôleurs
peuvent résider n'importe où, du moment que `routes.rs` peut les
atteindre. La convention existe parce que c'est ce que le scaffolding
émet et parce que routes et contrôleurs vont naturellement de pair.

## Pourquoi Suprnova diverge

Les contrôleurs Laravel sont des classes qui étendent
`Illuminate\Routing\Controller`. Les méthodes sont appelées sur des
instances que le conteneur résout par requête, et c'est là que se fait
l'injection par constructeur. Le motif convient bien en PHP - un `new` à
chaque requête coûte peu quand le processus entier est démonté après la
réponse.

En Rust, ce motif signifierait soit (a) allouer une struct de contrôleur
par requête, ce qui coûte un clone d'`Arc` dont vous n'avez pas besoin,
soit (b) réimplémenter l'injection de dépendances à travers une
hiérarchie de classes de base qui ne se rentabilise pas.

Suprnova retient le modèle le plus simple : un contrôleur est une
fonction async libre, et les « dépendances » sont soit des résolutions
du conteneur (`App::resolve::<Service>()?`), soit des arguments typés
par extraction (`form: UpdateUserRequest`). L'injection par constructeur
se fait à la frontière `#[injectable]` dans [Actions](actions.md), là où
elle a sa place. Le handler reste une fonction pure de la requête vers
la réponse, ce qui rend trivial de le tester isolément : construisez une
`Request`, appelez la fonction, faites une assertion sur le résultat.

## Suivant

- [Routage](routing.md) - ce en quoi `routes!`, `get!`, `post!` et
  `.name()` se développent
- [Requêtes de formulaire](requests.md) - validation typée via
  `#[derive(FormRequest)]`
- [Réponses](responses.md) - JSON, HTML, fichiers, flux, pages Inertia,
  redirections
- [Conteneur de service](container.md) - ce que `App::resolve` fait
  réellement
- [Actions](actions.md) - où réside la logique métier en dehors du
  contrôleur
- [Modèle d'erreur](error-model.md) - comment `?` transforme une
  `FrameworkError` en réponse

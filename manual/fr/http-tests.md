# Tests HTTP

Ce chapitre montre comment tester votre surface HTTP - routes,
middleware, flux d'authentification, réponses d'erreur - en pilotant
le pipeline de requête du framework à travers
`suprnova::handle_request`. Si vous avez écrit des tests de
fonctionnalité Laravel avec `$this->get('/users')` et affirmé sur
`$response->status()`, c'est l'équivalent Suprnova : le même `Router`
que vous montez en production s'exécute dans le test, chaque
middleware se déclenche, la limite de panique attrape toujours, et la
réponse est octet pour octet ce qu'un vrai client voit.

## La surface de test

Il y a exactement trois blocs de construction :

| Élément | Rôle |
|---|---|
| `Router` | Les routes sous test - construites de la même façon qu'en production |
| `MiddlewareRegistry` | La pile de middleware globale - construite de la même façon aussi |
| `handle_request(router, registry, req) -> hyper::Response<…>` | Le pilote en cours de processus - exécute une requête de bout en bout |

`handle_request` est la même fonction que `Server::run` appelle par
requête, exposée pour les tests et les embarqueurs. Tout ce qui
fonctionne en production fonctionne ici - le wrapper de récupération
de panique, la portée d'id de requête, la portée du flash bag
Inertia, la portée de l'état d'auth de la requête, le retrait du corps
sur HEAD, la terminaison post-réponse. Il n'y a pas de « mode test »
qui substitue un pipeline plus discret.

`handle_request_with_peer` est le même appel avec un
`Option<std::net::IpAddr>` explicite pour le pair connecté - utile
quand vous voulez affirmer sur la résolution de `Request::ip()` sans
mettre en place des en-têtes de proxy.

## Le problème du corps hyper

La seule complication à connaître d'entrée de jeu : `handle_request`
prend un `hyper::Request<hyper::body::Incoming>`. `Incoming` est le
type de corps en streaming interne de hyper ; vous ne pouvez pas en
construire un avec `Full::new(bytes)` ou n'importe lequel des types de
corps en mémoire. Il ne sort que d'une connexion hyper.

Il y a deux façons propres de contourner cela :

1. **Boucle locale TCP** - liez un écouteur `127.0.0.1:0`, servez un
   accept à l'intérieur d'un `service_fn`, envoyez la requête à
   travers un client hyper, et laissez `Incoming` être produit
   naturellement du côté serveur. C'est ce que fait déjà chaque test
   d'intégration du framework.
2. **Construction de `Request` en cours de processus** - pour les
   tests qui n'ont besoin d'inspecter que des accesseurs de `Request`
   (en-têtes, params de route, IP, analyse JSON) sans passer par le
   routage, utilisez le même motif de capture par boucle locale TCP
   mais avec un service qui extrait la `Request` vers un
   `oneshot::channel` plutôt que de l'exécuter. Le fichier
   `framework/tests/http_request_accessors.rs` a ce helper
   `build_request()` mot pour mot.

Les deux motifs produisent de vrais corps `Incoming`. La boucle locale
est locale, synchrone en termes d'horloge murale de test
(microsecondes), et ne touche jamais le réseau hors de `lo`. Il n'y a
pas de façon plus lente ou plus simple qui préserve le contrat.

### Pourquoi Suprnova diverge

Le `$this->get('/users')` de Laravel fonctionne parce que le cycle de
vie de requête de PHP est « construire un objet `Request`, le
dispatcher à travers le kernel ». Le kernel prend l'objet en mémoire
directement ; il n'y a pas de type de corps qui impose un transport.
Le serveur de Suprnova est construit sur hyper, et le type de corps de
hyper est délibérément contraint pour de bonnes raisons (streaming,
contre-pression, zéro-copie). La surface de test hérite de cette
contrainte.

Ce que vous échangez contre la contrainte, c'est la fidélité. Chaque
détail du chemin de requête de production - analyse des en-têtes,
plafonds de corps, mises à niveau de connexion - s'exécute de la même
façon dans les tests. Vous n'aurez jamais un test qui passe parce que
le harnais de test a sauté une couche que le vrai serveur exécute.

## Un premier test de bout en bout

Voici un test complet et fonctionnel qui monte une seule route,
envoie un GET contre elle, et affirme sur le statut et le corps.

```rust
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::{MiddlewareRegistry, Request, Router, handle_request};

async fn spawn_server(router: Router, accepts: usize) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else { return };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move {
                        Ok::<_, Infallible>(handle_request(router, middleware, req).await)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

async fn send_get(addr: SocketAddr, path: &str) -> (u16, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_get timeout")
        .expect("hyper send_request");
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status.as_u16(), bytes)
}

#[tokio::test]
async fn get_root_returns_hello() {
    let router = Router::new().get("/", |_req: Request| async { text("hello") });
    let addr = spawn_server(router, 1).await;

    let (status, body) = send_get(addr, "/").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"hello");
}
```

C'est la forme complète. Copiez les deux helpers par crate, adaptez-les
pour la suite (plusieurs accepts, capture d'en-tête, capture de
corps). Le framework lui-même utilise des helpers quasi identiques
dans `framework/tests/cors_middleware.rs`,
`framework/tests/middleware_panic_safety.rs`, et
`framework/tests/email_verified_middleware.rs`.

L'argument `accepts` borne combien de connexions la boucle d'accept
sert avant de sortir. Un suffit pour une seule requête ; montez à deux
ou plus quand un test exerce la récupération post-panique (voir
[Tester la limite de panique](#tester-la-limite-de-panique)).

## Construire une requête

À l'intérieur de `send_get` vous avez vu :

```rust
let req = hyper::Request::builder()
    .method("GET")
    .uri("/users/42")
    .header("Host", "localhost")
    .header("Content-Length", "0")
    .body(Full::new(Bytes::new()))
    .unwrap();
```

C'est la forme canonique. Quelques choses à savoir :

- **En-tête `Host`**. Hyper rejette les requêtes HTTP/1.1 qui n'en
  ont pas. Incluez-le toujours ; la valeur n'a pas d'importance à
  moins que votre handler ne s'indexe sur elle.
- **`Content-Length: 0`**. Faites correspondre au corps. Hyper calcule
  ceci pour vous avec `Full::new(Bytes::new())`, mais être explicite
  se lit plus proprement dans les tests.
- **Types de corps**. Le côté client envoie du `Full<Bytes>`. Le côté
  serveur reçoit de l'`Incoming`. Vous ne construisez jamais que des
  requêtes `Full<Bytes>` dans les tests ; le framework les reçoit
  comme `Incoming` après la conversion par connexion de hyper.

Un POST avec un corps JSON :

```rust
let body_bytes = serde_json::to_vec(&serde_json::json!({
    "name": "Alice",
    "email": "alice@example.com"
})).unwrap();

let req = hyper::Request::builder()
    .method("POST")
    .uri("/users")
    .header("Host", "localhost")
    .header("content-type", "application/json")
    .header("content-length", body_bytes.len())
    .body(Full::new(Bytes::from(body_bytes)))
    .unwrap();
```

## Assertions sur la réponse

La réponse qui revient de `handle_request` est une
`hyper::Response<BoxBody<Bytes, Infallible>>`. Trois choses que vous
allez y lire :

```rust
let (parts, body) = resp.into_parts();

// 1. Statut.
assert_eq!(parts.status.as_u16(), 200);

// 2. En-têtes - recherche insensible à la casse.
let location = parts.headers.get("location").and_then(|v| v.to_str().ok());
assert_eq!(location, Some("/login"));

// 3. Corps - collecter en octets, puis analyser.
use http_body_util::BodyExt;
let bytes = body.collect().await.unwrap().to_bytes();

// En texte :
let text = String::from_utf8_lossy(&bytes);

// En JSON :
let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
assert_eq!(value["message"], "ok");
```

Pour les réponses d'erreur ordinaires qui atteignent le rendu commun, la forme du corps documentée dans [Modèle d'erreur](error-model.md) comprend `message`, `errors` facultatif, `request_id`, et `debug_message` facultatif. `request_id` est `null` hors d'une portée de requête. Trois variantes spéciales rendent avant l'injection de `request_id` : `PrecognitionSuccess` est une réponse 204 sans corps, `PrecognitionFailure` est le corps de validation plus les en-têtes Precognition, et une sentinelle `AlreadyReported` rendue par erreur via HTTP est un 500 générique ne contenant que `message`. Utilisez une réponse d'erreur ordinaire quand vous affirmez que le middleware d'id de requête s'est exécuté.


## Assertions de réponse fluides avec TestResponse

Construire à la main le triplet `(status, headers, body)` et affirmer dessus
pièce par pièce, comme ci-dessus, est la base qu'utilise chaque harnais de
cette crate. `suprnova::testing::TestResponse` enveloppe ce même triplet dans
une API fluide de forme Laravel, afin qu'un test se lise comme une assertion
plutôt qu'une recherche d'en-tête :

```rust
use suprnova::testing::TestResponse;

let (parts, body) = resp.into_parts();
let bytes = body.collect().await.unwrap().to_bytes();
let headers = parts.headers.iter().map(|(k, v)| {
    (k.as_str().to_string(), v.to_str().unwrap_or_default().to_string())
});

TestResponse::new(parts.status.as_u16(), headers, bytes)
    .assert_ok()
    .assert_header("content-type", "application/json")
    .assert_json(serde_json::json!({ "message": "ok" }));
```

`new()` accepte tout itérable de paires d'en-têtes `(String, String)`  -  un
`HashMap<String, String>` (dans lequel plusieurs harnais existants collectent
déjà), un `Vec<(String, String)>`, ou `HeaderMap::iter()` mappé vers des
chaînes possédées  -  afin qu'aucun harnais n'ait à changer sa façon de piloter
une requête.

Chaque assertion retourne `&Self`, elles s'enchaînent donc :
`assert_status`, `assert_ok`, `assert_redirect(target: Option<&str>)`,
`assert_json` (correspondance de sous-ensemble  -  des clés supplémentaires dans
le corps sont acceptées), `assert_json_path` (notation à points, un segment
numérique indexe un tableau), `assert_json_count`, `assert_see`,
`assert_header`, `assert_cookie`. Les échecs d'assertion paniquent avec un
extrait attendu/réel, même contrat que `expect!` ([Tests](testing.md)) : c'est
une surface de test, non du code de bibliothèque, donc la règle interne
interdisant les paniques ne s'applique pas.

### `assert_session_has` a besoin d'un magasin de session

Toute autre assertion lit uniquement la réponse réseau.
`assert_session_has` ne le peut pas : l'état de session côté serveur vit dans
le `SessionStore`, non dans la réponse, et lorsqu'une réponse revient par le
socket de boucle locale, il ne reste aucune session en processus à lire.
Attachez le même magasin que celui avec lequel le `SessionMiddleware` du test
a été construit, plus son nom de cookie ; l'assertion déchiffre alors le
cookie de session de la réponse pour trouver la ligne elle-même :

```rust
let response = TestResponse::new(status, headers, body)
    .with_session_store(middleware.store(), "suprnova_session");

response
    .assert_session_has("flash.success", serde_json::json!("Saved!"))
    .await;
```

C'est la seule assertion `async`, car c'est la seule qui effectue des E/S ;
elle retourne toujours `&Self`, donc `.await` s'insère dans la ligne et la
chaîne continue après lui.

### Pourquoi Suprnova diverge

Le `TestResponse` de Laravel vit dans le même processus PHP que l'application
testée, de sorte que `assertSessionHas` lit directement `$this->session()` :
aucune limite réseau à franchir. Les tests Suprnova pilotent une véritable
connexion hyper, la session est donc exactement aussi opaque au test qu'à un
vrai navigateur : un cookie. `assert_session_has` reconquiert cette honnêteté
avec un handle de magasin explicite, au lieu de prétendre que le raccourci en
processus existe.

## Tester les réponses Inertia

`suprnova::testing::AssertableInertia` enveloppe un objet de page Inertia  -
qu'il revienne comme corps JSON `X-Inertia` ou intégré dans une coquille HTML
de navigation dure  -  dans le même style fluide qui panique en cas d'échec que
`TestResponse`. C'est l'équivalent Laravel de
`Inertia\Testing\AssertableInertia`.

Deux façons d'en obtenir un. Depuis un `TestResponse` qui est déjà passé par
une véritable visite `X-Inertia: true` :

```rust
use suprnova::testing::TestResponse;

let response = TestResponse::new(status, headers, body);
response
    .assert_inertia()
    .component("Users/Index")
    .url("/users")
    .has("users")
    .where_("users.0.name", "Ada")
    .count("users", 1)
    .missing("admin_only_field");
```

Ou directement depuis un `HttpResponse`  -  ce que retourne
`InertiaResponse::resolve`  -  pour un test qui pilote le pipeline de réponse
sans socket. Cette forme gère les deux représentations : un corps JSON
`X-Inertia`, ou l'élément `<script data-page="app">` intégré à la coquille
HTML :

```rust
use suprnova::testing::AssertableInertia;

let response = InertiaResponse::new("Users/Index")
    .with("users", users_json)
    .resolve(&req)
    .await?;

AssertableInertia::from_response(&response)
    .component("Users/Index")
    .where_("users.0.name", "Ada");
```

`version()` vérifie la version des assets de la page. Le résolveur par défaut
hache le manifeste Vite et retombe sur `MANIFEST_VERSION_FALLBACK` lorsqu'aucun
manifeste n'existe encore : affirmez contre cette constante plutôt que contre
un `"1.0"` codé en dur dans un test qui n'a pas construit de frontend :

```rust
use suprnova::MANIFEST_VERSION_FALLBACK;

response.assert_inertia().version(MANIFEST_VERSION_FALLBACK);
```

`has_flash(key, expected)` lit les données flash de la page avec le même
chemin pointé que `has` / `where_` lit les props ; `expected` est un `Option`,
passez donc `None::<serde_json::Value>` pour ne vérifier que la présence :

```rust
response.assert_inertia().has_flash("toast.message", Some(serde_json::json!("Saved!")));
response.assert_inertia().has_flash("toast", None::<serde_json::Value>);
```

### Recharger pour les assertions de rechargement partiel et de props deferred

`reload_only`, `reload_except` et `load_deferred_props` reflètent ce que fait
le client Inertia après la visite initiale : réémettre la même page comme
rechargement partiel et vérifier ce qui revient. Les tests HTTP de Suprnova
franchissent un vrai socket et chaque fichier de test possède son propre
harnais (voir [Où réside chaque élément](#où-réside-chaque-élément) ci-dessous),
ces méthodes ne portent donc aucun transport intégré. Attachez-en un avec
`with_reload`, une closure qui prend un `ReloadRequest` (l'URL, le composant,
la version et les clés de rechargement partiel à envoyer) et produit un future
qui retourne l'`AssertableInertia` rechargé :

```rust
use suprnova::testing::TestResponse;

let assertable = TestResponse::new(status, headers, body)
    .assert_inertia()
    .with_reload(move |reload| {
        async move {
            let header_pairs = reload.headers();
            let headers: Vec<(&str, &str)> = header_pairs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let (status, headers, body) = request(addr, "GET", &reload.url, &headers).await;
            TestResponse::new(status, headers, body).assert_inertia()
        }
    });

// Demande uniquement `users`, et affirme que le rechargement a atterri sur
// les mêmes composant, url et version, et que `users` est bien revenu.
assertable.reload_only(["users"]).await;

// Demande tout sauf `stats`, et affirme que `stats` est absent.
assertable.reload_except(["stats"]).await;

// Lit `deferredProps` sur la page d'origine, demande chaque clé deferred
// en un seul rechargement partiel, et affirme qu'elles sont toutes revenues.
assertable.load_deferred_props().await;
```

Appeler l'une des trois sans avoir d'abord appelé `with_reload` panique avec
cette instruction. Le résultat d'un rechargement transporte le même reloader,
donc un second `.reload_only(...).await` depuis lui fonctionne sans devoir le
rattacher.

### Pourquoi Suprnova diverge

Le `ReloadRequest` de Laravel réémet la requête à travers le même noyau PHP en
processus que le test d'origine  -  un client de test, toujours disponible. Les
tests HTTP de Suprnova pilotent une véritable boucle locale hyper/TCP et chaque
fichier de test définit sa propre paire `spawn_server` / `request` (voir
[Où réside chaque élément](#où-réside-chaque-élément) ci-dessous) ; aucun client
unique n'est donc disponible à `AssertableInertia`. `with_reload` rend cela
explicite au lieu de coder en dur un harnais qu'un fichier de test de forme
différente ne pourrait pas utiliser. `component()` saute aussi la vérification
d'existence du fichier de composant de page de Laravel (`view-finder`) : un
composant atteint par `Router::inertia` ou un
`InertiaResponse::new(name)` construit à la main est une chaîne au runtime,
sans fichier à vérifier ; l'équivalent à la compilation chez Suprnova est la
macro `inertia_response!` (voir [Réponses
Inertia](frontend-inertia-responses.md)). Ses noms de méthode divergent aussi
de ceux de `TestResponse` : `component`, `has`, `missing`, `where_`, `count`
et `has_flash` abandonnent entièrement le préfixe `assert_`, conformément à
`Inertia\Testing\AssertableInertia` de Laravel, dont les méthodes équivalentes
sont nues de la même manière. Le contrat de panique en cas d'échec est
identique dans les deux cas, sans l'indice visuel `assert_`.

## Tester le middleware

Les tests de middleware ressemblent exactement aux tests de route ;
la seule différence est ce que vous ajoutez avec `.append()` au
registre avant le spawn.

### Tester le middleware global

Transmettez le middleware à `MiddlewareRegistry::new().append(...)`
et utilisez ce registre - plusieurs middleware s'exécutent dans
l'ordre d'ajout, `prepend` en place un nouveau en tête.

```rust
use suprnova::{CorsConfig, CorsMiddleware, MiddlewareRegistry};

fn cors_registry() -> MiddlewareRegistry {
    MiddlewareRegistry::new().append(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(std::time::Duration::from_secs(600)),
    ))
}

#[tokio::test]
async fn cors_preflight_returns_204_with_headers() {
    let router = Router::new();
    // La forme à 3 arguments de `spawn_server` vous laisse câbler un
// MiddlewareRegistry non vide - copiez le helper depuis
// framework/tests/cors_middleware.rs (il fait ~30 lignes).
let addr = spawn_server(router, cors_registry(), 1).await;

    let (status, headers, _) = options(
        addr,
        "/anything",
        &[
            ("Origin", "https://app.example"),
            ("Access-Control-Request-Method", "POST"),
        ],
    ).await;

    assert_eq!(status, 204);
    assert_eq!(
        headers.get("access-control-allow-origin").map(String::as_str),
        Some("https://app.example"),
    );
}
```

Ce test prouve plus que la logique CORS elle-même : il prouve que le
middleware global s'exécute aussi sur les requêtes **non routées**, ce
qui est le contrat que le framework garantit (sinon un preflight
OPTIONS qui ne correspond jamais à une route sauterait CORS). Voir
`framework/tests/cors_middleware.rs` pour la suite complète.

### Tester le middleware propre à une route

Attachez avec `.middleware(...)` sur le builder de route, exactement
comme en production. Puis testez la route normalement - la chaîne de
middleware est construite à partir du même enregistrement.

```rust
let router = Router::new()
    .get("/admin/dashboard", |_req| async { text("admin") })
    .middleware(RequireRole::new("admin"));

let (status, _) = send_get(addr, "/admin/dashboard").await;
assert_eq!(status, 403); // requête non authentifiée
```

### Poser un utilisateur authentifié factice

Les vrais tests de flux d'authentification ont besoin d'un
utilisateur connecté. Le motif le plus propre est un tout petit
middleware ponctuel qui appelle `Auth::set_user` avant le middleware
sous test. Le propre
`framework/tests/email_verified_middleware.rs` du framework utilise
ceci :

```rust
use std::any::Any;
use std::sync::Arc;
use suprnova::{Auth, Authenticatable, Middleware, Next, Request, Response};

struct UserById(String);

impl Authenticatable for UserById {
    fn get_auth_identifier(&self) -> String { self.0.clone() }
    fn as_any(&self) -> &dyn Any { self }
}

struct LoginAs(String);

#[async_trait::async_trait]
impl Middleware for LoginAs {
    async fn handle(&self, request: Request, next: Next) -> Response {
        Auth::set_user(Arc::new(UserById(self.0.clone())));
        next(request).await
    }
}
```

Puis dans le test :

```rust
let registry = MiddlewareRegistry::new()
    .append(LoginAs("user-id-123".to_string()))
    .append(EnsureEmailVerifiedMiddleware::new());
```

`LoginAs` s'exécute en premier, installe l'utilisateur dans l'état
d'auth par requête, et le middleware sous test voit
`Auth::id() == Some(...)` sans jamais émettre de vraie connexion. La
portée de l'état d'auth est mise en place par `handle_request`
lui-même - le même qui s'exécute en production - si bien que
l'utilisateur est visible pour chaque middleware ultérieur et le
handler.

## Tester la liaison de modèle de route

`RouteParam<User>` hydrate un `User` typé à travers la chaîne d'extracteurs du handler, donc le test doit passer cet extracteur à une fonction `#[handler]` :

```rust
use suprnova::{RouteParam, Response, handler};

#[suprnova::model(table = "users")]
pub struct User {
    pub id: i64,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[handler]
async fn show(RouteParam(user): RouteParam<User>) -> Response {
    suprnova::http::json(serde_json::json!({ "email": user.email }))
}

#[tokio::test]
async fn show_user_binds_from_route_param() {
    // Insérer un utilisateur de test via le modèle. Configuration de base de
    // données omise - voir le chapitre sur les tests pour les motifs `TestDatabase`.
    let user = User::create(suprnova::attrs! {
        email: "bound@example.com"
    }).await.unwrap();

    // Un `RouteParam` destructuré utilise actuellement `param` comme nom de
    // paramètre de route du macro handler.
    let router: Router = Router::new()
        .get("/users/{param}", show)
        .into();

    let addr = spawn_server(router, MiddlewareRegistry::new(), 1).await;
    let (status, body) = send_get(addr, &format!("/users/{}", user.id)).await;

    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["email"], "bound@example.com");
}
```

Pour un paramètre de route `{user}` à la place, acceptez `user: RouteParam<User>` sans destructuration ; `RouteParam` déréférence vers `User` pour l'accès aux champs. Appeler `req.param(...).parse()` puis `User::find_or_fail(...)` teste l'analyse du paramètre et la recherche du modèle, pas la liaison modèle-de-route.

Pour les tests de liaison en isolation, appelez directement `<RouteParam<User> as AutoRouteBinding>::from_route_param(...)`. Cela vérifie l'implémentation de liaison sans routeur, mais n'exerce pas la chaîne d'extracteurs `#[handler]`.

## Tester les flux d'authentification de bout en bout

Pour tester une session de connexion de bout en bout, passez au serveur loopback un registre contenant `SessionMiddleware` et protégez `/dashboard` avec `AuthMiddleware` ou le middleware web-auth de l'application. Prouvez d'abord que la route rejette une requête sans cookie, puis connectez-vous, rejouez le cookie de session retourné, et prouvez que la route protégée réussit :

```rust
#[tokio::test]
async fn login_flow_issues_session_cookie() {
    // 1. Amorçage : créer l'utilisateur.
    Auth::password()
        .register("alice@example.com", "longpassword123")
        .await.expect("register");

    // 2. Monter une route protégée et le middleware de session avec état.
    let router: Router = Router::new()
        .post("/login", login_handler)
        .get("/dashboard", |_req: Request| async { text("dashboard") })
        .middleware(AuthMiddleware::new())
        .into();
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::new(SessionConfig::from_env()));
    let addr = spawn_server(router, registry, 3).await;

    // 3. Prouver que la route est protégée avant l'authentification.
    let (guest_status, _) = send_get(addr, "/dashboard").await;
    assert_eq!(guest_status, 401);

    // 4. Piloter la connexion et capturer l'en-tête Set-Cookie.
    let login = post_json(addr, "/login", serde_json::json!({
        "email": "alice@example.com",
        "password": "longpassword123",
    })).await;
    assert_eq!(login.status, 200);
    let cookie = extract_session_cookie(&login.headers);

    // 5. Rejouer le cookie contre la route protégée.
    let (status, body) = get_with_cookie(addr, "/dashboard", &cookie).await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"dashboard");
}
```

Le routeur abrégé sans ces middlewares démontre qu'injecter un cookie ne suffit pas. Gardez le middleware de session monté dans le registre et la porte d'authentification comme ci-dessus.


## Tester la limite de panique

Une panique à l'intérieur d'un handler ne doit pas faire planter le
serveur. Le wrapper de récupération de panique
(`execute_chain_safely`) l'attrape et la convertit en un 500 à travers
le même chemin que suivent les erreurs retournées. Vous pouvez
vérifier cela sans aucune infrastructure de test spéciale - définissez
`accepts >= 2` pour que l'écouteur survive à la panique :

```rust
#[tokio::test]
async fn panicking_handler_yields_500_and_server_survives() {
    let router = Router::new()
        .get("/panic", |_req: Request| async {
            panic!("intentional test panic");
            #[allow(unreachable_code)] text("unreachable")
        })
        .get("/ok", |_req: Request| async { text("ok") });

    let addr = spawn_server(router, 4).await;

    // Premièrement : la panique se traduit en un 500 assaini.
    let (s1, body) = send_get(addr, "/panic").await;
    assert_eq!(s1, 500);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["message"], "Internal Server Error");
    assert!(parsed.get("request_id").is_some());

    // Deuxièmement : l'écouteur survit. La requête suivante est normale.
    let (s2, body2) = send_get(addr, "/ok").await;
    assert_eq!(s2, 200);
    assert_eq!(&body2[..], b"ok");
}
```

## Tester les accesseurs sans passer par le routage

Parfois vous voulez tester un accesseur de `Request` (`bearer_token`,
`is_method`, `ip`, `is_json`, etc.) sans faire démarrer de routeur du
tout. L'astuce est un tout petit harnais qui exécute un service hyper
dont le seul travail est de construire la `Request` et de la renvoyer
à travers un `tokio::sync::oneshot::channel` :

```rust
let (req_tx, req_rx) = tokio::sync::oneshot::channel::<suprnova::Request>();
// ... service hyper en boucle locale dont le service_fn fait :
//     let req = suprnova::Request::new(hyper_req);
//     let _  = req_tx.send(req);
//     retourne un 200 avec un corps vide
let req = req_rx.await.unwrap();
```

`framework/tests/http_request_accessors.rs` a le helper complet
`build_request(builder, body) -> Request`. Copiez-le une fois par
crate et chaque test d'accesseur se lit proprement :

```rust
#[tokio::test]
async fn bearer_token_extracts_simple_token() {
    let req = build_request(
        hyper::Request::builder()
            .method("GET")
            .uri("/api/users")
            .header("Authorization", "Bearer secret-token-123"),
        "",
    ).await;
    assert_eq!(req.bearer_token().as_deref(), Some("secret-token-123"));
}
```

La `Request` est réelle (produite par hyper à partir d'un vrai échange
réseau), mais aucun routage ni middleware ne s'est exécuté -
exactement ce que vous voulez quand l'unité sous test est l'accesseur
lui-même.

## Hooks du builder sur `Request`

Quand vous avez une `Request` en main et devez faker un morceau de la
couche de routage, trois méthodes de builder aident :

```rust
impl Request {
    pub fn with_params(mut self, params: HashMap<String, String>) -> Self;
    pub fn with_route_pattern(mut self, pattern: String) -> Self;
    pub fn with_peer_addr(mut self, addr: std::net::IpAddr) -> Self;
}
```

Ce sont les mêmes méthodes que le serveur appelle quand il dispatche
une route correspondante - `Router` appelle `with_params` après que
`matchit` retourne, `with_route_pattern` pour que
`req.route_pattern()` se résolve, et `with_peer_addr` une fois qu'il
connaît l'IP du socket TCP accepté. Dans les tests, vous les appelez
vous-même pour court-circuiter la même mise en place.

```rust
let req = Request::new(hyper_req)
    .with_params(HashMap::from([("id".into(), "42".into())]))
    .with_route_pattern("/users/{id}".into())
    .with_peer_addr("192.168.1.10".parse().unwrap());

assert_eq!(req.param("id").unwrap(), "42");
assert_eq!(req.ip(), Some("192.168.1.10".parse().unwrap()));
```

## Ce qu'il faut savoir

Une courte liste de pièges qui attrapent les auteurs débutants :

- **`Incoming` est réservé au côté serveur.** Vous ne pouvez pas en
  construire un dans votre test. La boucle locale TCP (ou la capture
  de service en cours de processus) est le seul chemin - il n'y a pas
  de constructeur « construire une `Request` depuis un corps
  `Vec<u8>` ».
- **Ne partagez pas d'état entre les tests.** Chaque `#[tokio::test]`
  obtient son propre runtime ; la pollution inter-tests signifie
  généralement que vous partagez un global (`once_cell`,
  `lazy_static`, une variable d'env). Pour l'état DB, voir
  `TestDatabase` dans [Tests](testing.md).
- **Les cookies ont besoin d'un vrai client.** Aucun pot à cookies
  automatique - faites transiter le `Set-Cookie` d'une réponse vers le
  `Cookie` de la suivante. Voir
  `framework/tests/auth_http_middleware.rs` pour le motif.
- **Le spawn de terminaison post-réponse n'est pas bloquant.** Si vous
  voulez affirmer sur des effets de bord qui s'exécutent via
  `Terminable`, sondez-les - la réponse revient au client avant que le
  hook ne s'exécute.

## Où réside chaque élément

| Élément | Fichier |
|---|---|
| `handle_request`, `handle_request_with_peer` | `framework/src/server.rs` |
| `Request::new`, `with_params`, `with_route_pattern`, `with_peer_addr` | `framework/src/http/request.rs` |
| `MiddlewareRegistry::new`, `append`, `prepend` | `framework/src/middleware/registry.rs` |
| Harnais de test en boucle locale (canonique) | `framework/tests/cors_middleware.rs` |
| `TestResponse` (assertions fluides sur le triplet ci-dessus) | `framework/src/testing/response.rs` |
| `AssertableInertia`, `ReloadRequest` (assertions fluides sur l'objet de page Inertia) | `framework/src/testing/inertia.rs` |
| Harnais de capture de `Request` en cours de processus | `framework/tests/http_request_accessors.rs` |
| Motif de test de limite de panique | `framework/tests/middleware_panic_safety.rs` |
| Motif de bout en bout auth + middleware | `framework/tests/email_verified_middleware.rs` |

## Suivant

- [Tests](testing.md) - `#[suprnova_test]`, `TestDatabase`, les
  macros `describe!`/`test!`/`expect!`, et la surface au niveau unitaire
- [Modèle d'erreur](error-model.md) - la forme JSON que chaque réponse
  d'erreur utilise, la règle d'assainissement des 5xx, et ce que
  signifie `request_id` dans un corps de test
- [Middleware](middleware.md) - écrire le middleware que vous testez
  ici, et le cycle de vie global-contre-route
- [Routage](routing.md) - le `Router` que vous montez à la fois en
  production et dans les tests, les params de route, les noms de
  route, les URL signées
- [Authentification](authentication.md) - la façade `Auth`,
  `Authenticatable`, les guards, et comment `Auth::set_user` interagit
  avec la portée de requête que `handle_request` installe

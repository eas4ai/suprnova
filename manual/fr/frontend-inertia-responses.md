# Réponses Inertia

Une réponse Inertia est la façon dont un handler Suprnova expédie de
l'état à un composant de page Svelte / React / Vue. Chaque handler qui
rend une page Inertia en retourne une, construite soit via la macro
[`inertia_response!`](#la-macro-inertia-response) (pour des props
eager typées et vérifiées à la compilation), soit via le builder
[`InertiaResponse`](#le-builder-inertiaresponse) (pour tout le
reste - props lazy, props deferred, merge, once, scroll, flash). Ce
chapitre couvre la surface de réponse de bout en bout : la macro, le
builder, les fonctionnalités du protocole v3 (rechargements partiels,
chiffrement d'historique, détection de version), les données partagées
via `App::inertia_share*`, et le flash bag transporté à travers les
redirections.

Si vous n'avez pas encore choisi de frontend,
[Présentation du frontend](frontend.md) et
[Composants de page](frontend-pages.md) viennent d'abord ; ce chapitre
suppose que le pont SPA est câblé et se concentre sur ce que votre
handler retourne.

## La macro `inertia_response!`

La macro est le chemin le plus court d'un handler vers une page eager
typée. Elle prend la requête courante, un nom de composant et une
expression de props :

```rust
use suprnova::{Request, Response, inertia_response, InertiaProps};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        message: "Hello from Suprnova!".into(),
    })
}
```

Trois choses à savoir :

- **Le `&req` en tête est obligatoire.** La macro lit les en-têtes
  `X-Inertia`, l'URL et les en-têtes de filtrage des rechargements
  partiels sur la requête, donc elle a besoin de la valeur de requête
  (ou d'une référence). Sans lui, les rechargements partiels
  casseraient silencieusement.
- **L'existence du composant est vérifiée à la compilation.** La macro
  cherche `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}` ; si
  aucun fichier ne correspond, la compilation échoue avec une
  suggestion « vouliez-vous dire… ? » tirée des noms de fichiers
  réellement présents sur le disque. Les chemins imbriqués
  fonctionnent de la même façon - `inertia_response!(&req,
  "Admin/Dashboard", …)` résout
  `frontend/src/pages/Admin/Dashboard.svelte` (ou l'extension de votre
  frontend).
- **La macro s'expanse en un `Result` attendu par `await`.** Votre
  handler doit retourner [`Response`](error-model.md) (qui est
  `Result<HttpResponse, HttpResponse>`) ou un autre type qui absorbe
  `FrameworkError` via `?` / `From`. Les échecs pendant la
  sérialisation des props ou la construction de la réponse sont
  retournés comme `Err`, pas en paniques.

### Props façon JSON

Pour le prototypage et les toutes petites pages, vous pouvez sauter la
struct typée :

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

La macro valide toujours le fichier de composant. Le compromis est que
vous perdez la chaîne de props typées - pas de
`#[derive(InertiaProps)]`, pas de génération TypeScript automatique,
pas de vérification à la compilation que la forme attendue par le
frontend correspond.

### Redéfinition facultative de la config

La macro accepte un `InertiaConfig` final facultatif pour des
redéfinitions par réponse (des réglages SSR différents, un titre par
défaut personnalisé pour une page) :

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

La plupart des applications enregistrent une seule config à l'amorçage
via [`Inertia::install`](#amorçage-inertia-install) et ne touchent
jamais à cet argument - la config installée est déjà le point de
départ de chaque réponse. N'en passez une ici que pour redéfinir la
config installée pour une seule page.

## `#[derive(InertiaProps)]`

`InertiaProps` émet un impl `Serialize` dont les noms de clé
correspondent à vos noms de champ. Il existe pour que le chemin des
props typées reste concis et pour que le générateur TypeScript
(`suprnova generate-types`) ait un marqueur à trouver :

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
pub struct UserProps {
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
}
```

Les types imbriqués se composent normalement - les champs peuvent
être des `Vec<T>`, des `Option<T>`, des structs imbriquées, tout ce
qui est `Serialize`-able. Les types imbriqués eux-mêmes n'ont pas
besoin de dériver `InertiaProps` ; il leur faut juste `Serialize`.
Utilisez `#[derive(InertiaProps)]` sur la struct de props de
*premier niveau* et vous obtenez la surface TypeScript automatique
(voir [Types TypeScript](frontend-typescript-types.md)) pour tout
l'arbre.

## Le builder `InertiaResponse`

La macro couvre les props eager typées. Tout le reste - lazy,
optional, deferred, mergeable, mis en cache côté client, flash,
redéfinitions du chiffrement d'historique - passe directement par le
builder :

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy : la closure ne s'exécute que si la prop sera réellement
        // envoyée (visite initiale, ou rechargement partiel qui demande
        // cette clé).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional : jamais envoyée lors des visites initiales ; le client
        // doit explicitement demander la clé via X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer : sautée au rendu initial ; le client émet un XHR de suivi
        // et la closure s'exécute à ce moment-là.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge : ajout à l'existant sur les rechargements partiels
        // (« charger plus »).
        .merge("rows", next_page().await?)
        // Once : mise en cache côté client à travers les navigations ; le
        // résolveur est sauté aux visites suivantes sauf si le serveur
        // force un rafraîchissement.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash : toast ponctuel ; apparaît sous `page.flash`, pas `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Méthode | Rôle | Équivalent Laravel |
|---|---|---|
| `.with(k, v)` | Prop eager, respecte le filtrage des rechargements partiels | prop typée |
| `.always(k, v)` | Prop eager, ignore les filtres de rechargement partiel | `Inertia::always(…)` |
| `.lazy(k, ‖)` | Le résolveur ne s'exécute que si la prop sera envoyée | closure `fn () => …` |
| `.optional(k, ‖)` | Jamais à la visite initiale ; doit être demandée explicitement | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Sautée à la visite initiale ; un XHR de suivi déclenche la résolution | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combine avec l'état client existant sur les rechargements partiels | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | Le client met en cache à travers les navigations | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate` (via `Inertia::paginate`) | Pagination à défilement infini | `Inertia::scroll(…)` |
| `.flash(k, v)` | Valeur ponctuelle sous `page.flash` (pas `props`) | `session()->flash(…)` |
| `.title(…)` | `<title>` par défaut pour la coquille HTML | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Chiffrement d'historique par réponse | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Force la rotation de la clé d'historique sur **cette** page | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Garde le `#fragment` après une visite Inertia | `Inertia::preserveFragment()` |

Les méthodes eager du builder ont des homologues `try_*` (`try_with`,
`try_always`, `try_merge_with`, `try_scroll`, `try_flash`) qui
retournent `Result<Self, FrameworkError>` quand l'impl `Serialize`
d'une valeur peut échouer à l'exécution - les méthodes infaillibles
convertissent la panique en 500 via [la limite de
panique](error-model.md), alors prenez `try_*` quand vous préférez
gérer l'échec explicitement.

`.clear_history()` marque la réponse que vous êtes en train de
construire. Un handler de déconnexion redirige, et le navigateur jette
la réponse de redirection - donc c'est la page de connexion, et non la
réponse de déconnexion, qui doit porter le flag.
`App::clear_history()` est le remède pour ce cas - c'est une fonction
libre, pas une méthode de builder, donc elle n'est pas dans le tableau
ci-dessus. Elle flashe un flag de session ponctuel que le prochain
objet de page Inertia transforme en `clearHistory: true`. Elle a
besoin d'une portée de session, et elle survit exactement à un saut.

Appelez-la **après** `Auth::logout()` /
`Auth::logout_and_invalidate()`, pas avant - l'invalidation vide toute
la session, et le flag vit dans cette session, donc le flasher d'abord
ne fait que le faire effacer par le vidage :

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### Stratégies de fusion et défilement infini

`.merge` (ajout à la fin), `.merge_prepend` et `.deep_merge` couvrent
les cas courants de « charger plus ». Pour une fusion par différence -
mettre à jour les lignes que le client détient déjà au lieu de les
dupliquer - prenez `.merge_with` avec une `MergeStrategy` explicite
portant une clé `match_on` :

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // la nouvelle tranche de page
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` nomme le champ sur lequel le client déduplique (émis dans
l'objet de page comme `matchPropsOn`), si bien qu'un rechargement qui
chevauche la fenêtre courante remplace les lignes correspondantes sur
place plutôt que d'ajouter des copies. `Prepend` et `Deep` prennent le
même `match_on`.

Le défilement infini repose sur la même mécanique, avec des
métadonnées de pagination attachées. `.scroll` / `.scroll_with` - ou
`.paginate`, qui adapte directement un `LengthAwarePaginator` ou un
`CursorPaginator` - émettent `scrollProps` à côté des données, et le
composant `<InfiniteScroll>` du client pilote les récupérations
suivante/précédente :

```rust
// `posts` est un CursorPaginator issu du query builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

Le framework lit la direction de fusion depuis l'en-tête de requête
`X-Inertia-Infinite-Scroll-Merge-Intent` que le client envoie
(`append` quand on défile vers le bas, `prepend` quand on défile vers
le haut). Lors d'une visite neuve - pas d'en-tête d'intention -
`scrollProps["posts"].reset` vaut `true`, si bien que le client vide
son accumulateur avant de rendre la première fenêtre.

## Rechargements partiels

Le client Inertia 3 peut demander un sous-ensemble des props d'une
page (ou un sur-ensemble, en incluant une clé Optional ou Defer). Le
protocole utilise trois en-têtes de requête :

| En-tête | Signification |
|---|---|
| `X-Inertia-Partial-Component` | Le composant faisant l'objet du rechargement partiel - il doit correspondre au composant de la réponse pour que le filtrage s'applique. |
| `X-Inertia-Partial-Data` | Liste blanche : clés de props à inclure, séparées par des virgules. |
| `X-Inertia-Partial-Except` | Liste noire : clés de props à exclure, séparées par des virgules. L'emporte sur `Partial-Data` en cas de collision de clé. |

Règles de filtrage :

- Les props `Eager`, `Lazy`, `Merge`, `Once` et `Scroll` suivent la
  sémantique liste blanche / liste noire.
- Les props `Always` sont envoyées quoi qu'il arrive.
- Les props `Optional` et `Defer` ne sont jamais présentes lors d'une
  visite standard et n'apparaissent que sur un rechargement partiel
  correspondant qui liste explicitement la clé.

Le handler n'a rien de particulier à faire - enregistrez chaque prop
via le builder, et le framework consulte les en-têtes au moment de
sérialiser l'objet de page.

Le cache côté client d'une prop `once` n'est respecté que lors d'une
visite Inertia **complète**. Sur un rechargement partiel qui nomme la
clé (`router.reload({ only: ['stats'] })`), le résolveur s'exécute et
la valeur est envoyée - le client a demandé précisément parce qu'il en
veut une fraîche, et respecter là sa prétention de cache périmé ne
retournerait rien du tout pour la clé qu'il a demandée.

## Données partagées via `App::inertia_share*`

Certaines props sont les mêmes sur chaque page Inertia - l'état
d'auth, le token CSRF, la locale courante, des flags à l'échelle de
l'application. Enregistrez-les une fois à l'amorçage et elles se
fusionnent dans chaque réponse :

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // Sync, matérialisé une fois à l'amorçage.
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // Async, résolu par réponse (sauté par les rechargements
    // partiels qui excluent la clé).
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // Mis en cache côté client à travers les navigations -
    // `share_once` s'exécute sur la première page qui en a besoin,
    // puis le client saute la re-résolution via
    // `X-Inertia-Except-Once-Props` jusqu'à ce que la clé de cache
    // change.
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    });
}
```

Pour des données partagées par requête (l'utilisateur authentifié, des
flags à portée de requête), implémentez
[`InertiaSharedData`](#données-partagées-par-requête) et enregistrez
le singleton - le framework appelle `share(&req)` sur chaque réponse
Inertia et fusionne le résultat.

### Précédence en cas de collision de clé

Quand la même clé apparaît dans plus d'une couche, les écritures les
plus récentes l'emportent :

1. Registre statique (`App::inertia_share` / `App::inertia_share_lazy`)
2. Fournisseur de trait par requête (`InertiaSharedData::share`)
3. Méthodes de builder par réponse (`.with`, `.lazy`, etc.)

Cela permet à un handler de redéfinir une valeur par défaut partagée
globalement pour une page, sans avoir à désenregistrer quoi que ce
soit.

### Données partagées par requête

Le trait s'exécute une fois par réponse Inertia, avec accès à la
requête. Les implémentations ont besoin d'`async_trait` (ré-exporté
comme `suprnova::__async_trait`) et d'`IndexMap` (ré-exporté comme
`suprnova::indexmap`) :

```rust
use suprnova::{
    App, Auth, FrameworkError, InertiaRequestExt, InertiaSharedData, Prop,
    indexmap::IndexMap,
};
use std::sync::Arc;

pub struct AuthShare;

#[suprnova::__async_trait]
impl InertiaSharedData for AuthShare {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::Eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        Ok(out)
    }
}

// Dans bootstrap :
App::register_inertia_shared(Arc::new(AuthShare));
```

## Flash et redirections

Les données flash sont un état ponctuel qui doit apparaître au rendu
suivant puis disparaître - messages toast, IDs « tout juste créés »,
résumés de validation. Suprnova les expose sous `page.flash` sur
chaque réponse Inertia. Il y a trois écrivains :

```rust
// 1. Pousser dans le flash bag de la requête courante.
App::flash("toast", "Saved");

// 2. Attacher à une réponse précise (même effet, sur cette réponse seulement).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Transporter à travers une redirection via la façade Redirect.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

La forme `Redirect::with(key, value)` est le chemin inter-handlers :
la valeur atterrit dans la session sous `_flash.new.*`, le
[`SessionMiddleware`](csrf.md) de la requête suivante la fait vieillir
vers `_flash.old.*`, et l'`InertiaResponse` de destination l'expose
sous `page.flash`.

Le flash de la même requête (le sac task-local) l'emporte sur le flash
de session hérité en cas de collision de clé, si bien qu'un handler de
destination peut redéfinir une valeur entrante simplement en
re-flashant la clé.

Les clés de session internes (tout ce qui est préfixé par `_`) sont
filtrées hors de `page.flash` - `_old_input` pour le repeuplement des
formulaires et les flags de protocole `_inertia.*` ne fuient pas vers
le client.

### Helpers de redirection

`Redirect` offre la surface Laravel complète :

```rust
Redirect::to("/dashboard")                       // 302 vers un chemin
Redirect::route("posts.show").with("id", "42")   // route nommée, paramètres de route
Redirect::back("/")                              // URL précédente enregistrée en session
Redirect::refresh()                              // même URL, GET frais
Redirect::guest(&req, "/login")                  // met de côté l'URL visée
Redirect::intended("/dashboard")                 // dépile l'URL mise de côté
Redirect::signed_route("downloads.show", &[("id","42")])?  // URL signée
Redirect::to("/posts/42").preserve_fragment()    // garde #frag à travers la visite
```

Toutes les variantes de `Redirect` acceptent `.with(k, v)`,
`.with_input(map)`, `.with_errors(map)`, `.with_errors_bag(name,
map)`, `.cookie(c)`, `.header(k, v)`, `.permanent()`, `.status(303)`,
etc. La chaîne complète reflète le `RedirectResponse` de Laravel.

Pour les visites Inertia non-GET, le framework convertit
automatiquement la réponse en `303 See Other` quand
[`Inertia303Middleware`](#amorçage-inertia-install) est installé, si
bien que le navigateur émet un GET de suivi propre au lieu de
resoumettre le PUT/PATCH/DELETE d'origine vers la cible de la
redirection.

Pour envoyer le visiteur **hors** de l'application Inertia - un
fournisseur de paiement, un point de terminaison d'autorisation OAuth,
un portail de facturation hébergé - utilisez `location_for` :

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

Un XHR Inertia reçoit `409` + `X-Inertia-Location` (le client exécute
`window.location = url`) ; une navigation dure reçoit un simple
`302` + `Location`. Le `InertiaResponse::location(url)` nu retourne
toujours la forme 409 - ne l'utilisez que là où la requête est déjà
connue comme étant une visite Inertia, parce qu'un navigateur qui suit
un `409` sans en-tête `Location` n'a nulle part où aller.

## Détection de version

Inertia versionne le manifeste d'assets afin qu'un client de longue
durée n'essaie pas de monter une page du bundle d'hier contre le
serveur d'aujourd'hui. Quand l'en-tête `X-Inertia-Version` du client
ne correspond pas à la version configurée du serveur,
[`InertiaVersionMiddleware`](#amorçage-inertia-install) répond avec un
`409 Conflict` et un en-tête `X-Inertia-Location` nommant la nouvelle
URL - le client Inertia le récupère et fait un rechargement complet de
la page, récupérant le nouveau bundle.

Le rebond re-flashe d'abord la session. Le client répond à un 409 par
un GET de page complète, et ce GET est une requête neuve - sans le
re-flash, une erreur de validation ou un message de succès flashé par
la requête précédente vieillit et disparaît avant que la page de
destination puisse le lire, et l'utilisateur perd son message d'erreur
uniquement parce qu'un déploiement est arrivé au milieu d'une
soumission. Cela demande que `SessionMiddleware` soit enregistré avant
le middleware de version.

Vous définissez la version via `InertiaConfig` :

```rust
use suprnova::InertiaConfig;

// Statique - la plupart des applications. Intégrez un identifiant de build.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamique - lisez un hash de manifeste, un ID de déploiement de conteneur, n'importe quoi.
// La closure s'exécute à chaque vérification de version ; mettez en cache à l'intérieur si ce n'est pas peu coûteux.
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

Pour une résolution de version asynchrone ou faillible (par ex. lire
un hash de manifeste depuis S3), faites la lecture une fois à
l'amorçage et passez la `String` mise en cache à `.version(...)`.

## Amorçage : `Inertia::install`

La plupart des applications installent les trois middlewares de
protocole en un seul appel :

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register() -> Result<(), suprnova::FrameworkError> {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)?;
    // …autres données partagées, routes, etc.
    Ok(())
}
```

`Inertia::install` retourne un `Result` et, dans l'ordre :

1. Fait un échec fermé si `cfg` se résout en mode production
   (`development == false` - le défaut dès que `APP_ENV=production`)
   mais qu'aucun manifeste Vite ne peut être chargé depuis
   `cfg.manifest_path`. C'est le garde-fou CFG-01 : un démarrage en
   production avec un frontend non compilé échoue explicitement au
   lieu de retomber silencieusement sur un ancien chemin d'assets codé
   en dur.
2. Enregistre `InertiaHeadersMiddleware` - pose `Vary: X-Inertia` sur
   chaque réponse et transforme un `200` vide sur une visite Inertia
   en un `303` de retour.
3. Enregistre `InertiaVersionMiddleware` - émet le `409` +
   `X-Inertia-Location` quand le client et le serveur ne s'accordent
   pas sur la version des assets.
4. Enregistre `Inertia303Middleware` - promeut `302` en `303` sur les
   redirections Inertia non-GET.

L'ordre compte : le middleware d'en-têtes est enregistré en premier,
il est donc le plus externe et voit chaque réponse - y compris le
`409` que le middleware de version retourne avant même que le handler
ne s'exécute.

`install` **retient** aussi **la config**. Chaque `InertiaResponse`
construite ensuite en part, si bien que `.frontend(...)`,
`.version(...)`, `.default_title(...)`, `.ssr(...)` et
`.encrypt_history(...)` posés ici atteignent chaque page sans qu'un
handler ait à passer quoi que ce soit. Un handler qui veut des
réglages différents pour une page les redéfinit toujours avec
`.with_config(...)` ; une application qui n'appelle jamais
`Inertia::install` obtient `InertiaConfig::default()` ; et rappeler
`install` remplace la config retenue.

`.with_config(...)` remplace la config en bloc, `version` comprise.
`InertiaVersionMiddleware` résout toujours la version qui a été donnée
à `Inertia::install`, donc une config ici qui ne porte pas le même
`.version(...)` fait annoncer par l'objet de page une version que le
middleware fera rebondir - le client subit un chargement de page
complète supplémentaire après avoir visité cette page. Posez
`.version(...)` sur la redéfinition pour qu'elle corresponde.

Enregistrez `SessionMiddleware` **avant** `Inertia::install` si vous
utilisez des données flash. Le middleware de version re-flashe la
session avant de faire rebondir le client, si bien qu'une erreur
flashée survit au GET de page complète de suivi ; il ne peut le faire
qu'à l'intérieur d'une portée de session.

Ne sautez l'appel que si vous ne voulez véritablement pas l'un de ces
middlewares (c'est rare ; tous les trois ferment de vrais modes
d'échec - empoisonnement de cache entre les deux représentations d'une
URL, bundle périmé silencieux, et rejeu de formulaire sur
redirection).

## Éléments `<head>` pilotés par le serveur

Inertia 3.5 a ajouté une option client pour laisser le serveur décider
de ce qui va dans `<head>` - utile quand les balises meta dépendent de
l'enregistrement que vous venez de charger, et que vous ne voulez pas
que le titre et les balises OG vivent à deux endroits.

Cela n'a besoin d'aucun support du framework. Le client lit les
éléments depuis une **prop ordinaire**, si bien que n'importe quel
handler peut les fournir :

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>) -> Response {
    Ok(inertia_response!("Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    }))
}
```

Activez-le côté client :

```js
createInertiaApp({
  serverHead: true,        // lit la prop `head`
  // serverHead: 'meta',   // ou lit une prop nommée différemment
  // serverHead: (page) => [...],  // ou calcule depuis la page entière
})
```

Chaque chaîne est un élément HTML. Le client estampille un attribut
`data-inertia` sur tout élément qui n'en a pas, pour pouvoir calculer
le diff des éléments de head à travers les navigations ; fournissez
votre propre `data-inertia="og-title"` quand vous voulez une identité
stable plutôt qu'une correspondance positionnelle.

Échappez tout ce qui est interpolé depuis des données utilisateur -
ces chaînes sont injectées comme du HTML, donc les règles habituelles
s'appliquent.

## SSR

Suprnova dialogue avec un worker SSR hors processus - typiquement le
bundle `createServer()` de `@inertiajs/{svelte,react,vue}/server`
exécuté sous Node / Bun / Deno - via un loopback HTTP. Activez-le sur
la config que vous remettez à
[`Inertia::install`](#amorçage-inertia-install) - cette config est le
point de départ de chaque réponse, donc il n'y a rien à faire
transiter par vos handlers :

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // URL du worker
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

Le SSR est désactivé par défaut, et c'est une propriété de la config :
activé pour chaque réponse construite à partir de la config installée,
désactivé pour toute réponse qui redéfinit avec un `.with_config(...)`
qui ne le pose pas. Quand il est activé, le framework poste l'objet de
page vers `<url>/render` et intègre `{ head, body }` dans la coquille
HTML. En cas d'erreur ou d'expiration du worker, la réponse retombe
sur le CSR (un `<div id="app">` vide que le client hydrate) et le hook
`on_ssr_error(...)` se déclenche ; basculez `ssr_throw_on_error(true)`
en CI pour transformer ces échecs en vrais 500.

Démarrez le worker séparément - `suprnova ssr:start` est le lanceur
standard une fois que votre projet livre un point d'entrée SSR.

## Configuration

Le comportement d'Inertia se configure par programmation via
`InertiaConfig`, et la config que vous remettez à
[`Inertia::install`](#amorçage-inertia-install) est celle dont part
chaque réponse. La seule variable d'environnement que le framework lit
directement est `SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`), et
elle ne fournit que le nom de fichier du point d'entrée par défaut et
les extensions des composants de page quand la config ne le dit pas -
un `.frontend(Frontend::React)` explicite sur la config installée
l'emporte, et c'est ce que `suprnova new --frontend react` scaffolde.
Tout le reste a la forme d'un builder :

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // redéfinit SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // plafonne le fan-out des props lazy
    .url_resolver(|req| req.path_and_query()) // comment `page.url` est dérivé
    .production();                            // false → charge depuis le serveur de dev Vite
```

Défauts propres à chaque frontend :

| Frontend | Point d'entrée par défaut | Extensions de page |
|---|---|---|
| Svelte (par défaut) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

### Le champ `url`

`page.url` est le chemin **et** la chaîne de requête de la requête
(`/users?page=2&sort=name`). Le client l'écrit dans `history.state`,
c'est donc ce que la navigation avant/arrière et `router.reload()`
rejouent - supprimez la chaîne de requête et chaque page paginée ou
filtrée revient silencieusement à la page une.
`InertiaVersionMiddleware` dérive lui aussi son `X-Inertia-Location`
du chemin et de la chaîne de requête de la requête, si bien que par
défaut un rebond 409 de version d'assets amène le navigateur
exactement sur l'URL que l'objet de page a nommée.

Redéfinissez la dérivation avec `url_resolver` quand l'URL que le
client doit enregistrer diffère de celle qui est arrivée - un préfixe
de locale sur lequel le SPA ne route pas, ou un chemin qu'un proxy
inverse a réécrit :

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

Le résolveur lit la requête via `InertiaRequestExt`, et s'applique à
chaque réponse construite à partir de la config que vous passez à
[`Inertia::install`](#amorçage-inertia-install) - l'endroit habituel
pour un résolveur qui doit s'appliquer à toute l'application.
Redéfinissez-le pour une seule réponse avec
`InertiaResponse::with_config(cfg)`. Un résolveur ne change que
`page.url`. Le rebond 409 continue de nommer l'URL réellement
arrivée - c'est l'URL que le navigateur doit récupérer - donc avec un
résolveur en place, les deux diffèrent délibérément.

Le manifeste Vite situé à `manifest_path` est chargé paresseusement à
la première requête et mis en cache pour la durée de vie du
processus - chaque réponse construite à partir de la config installée
partage cet unique cache, si bien que le fichier est lu et analysé une
seule fois. Quand il est absent, les balises d'assets de production
retombent sur un ancien chemin codé en dur et un `tracing::warn!` se
déclenche pour que le manque apparaisse dans les journaux.

### Pourquoi Suprnova diverge

L'adaptateur Inertia de Laravel a un unique registre global de «
données partagées » plus un appel `Inertia::share($k, $v)` par
requête. Le modèle un-processus-par-requête de PHP rend cela sûr : un
processus neuf par requête signifie aucune fuite entre visiteurs
concurrents.

Le modèle de processus de Rust est l'inverse - un seul processus sert
de nombreuses requêtes concurrentes sur de nombreux threads. Le
registre vit donc sur le [conteneur](container.md) (task-local →
thread-local → global), pas dans des statiques globales au processus.
`App::inertia_share*` écrit dans l'`InertiaRegistry` du conteneur
actif, ce qui donne aux tests utilisant `TestContainer::fake()` une
isolation propre sans avoir à désenregistrer quoi que ce soit. Même
surface que Laravel ; machinerie différente en dessous, parce que le
runtime est différent.

Cinq autres choix propres à Rust méritent d'être signalés :

- **Les résolveurs de props lazy s'exécutent en parallèle**, plafonnés
  par `max_concurrent_resolvers` (16 par défaut). Une page avec douze
  props lazy émet douze requêtes parallèles dans une seule tâche
  Tokio - c'est pour cela que nous avons bâti le framework sur Tokio.
  Ajustez le plafond si une page a beaucoup de props lazy tapant
  chacune un service externe.
- **La vérification de composant à la compilation** n'est pas du tout
  une fonctionnalité de Laravel, parce que PHP ne peut pas voir vos
  fichiers frontend à la compilation. Suprnova le peut, donc une faute
  de frappe dans `inertia_response!("Dashbaord", …)` fait échouer la
  compilation avec une suggestion « vouliez-vous dire Dashboard ? » au
  lieu de ressortir plus tard en « composant introuvable » à
  l'exécution.
- **Un `200` vide sur une visite Inertia devient un `303`, pas un
  `302`.** Le `onEmptyResponse` de Laravel retourne
  `redirect()->back()` (un 302) et s'appuie sur sa conversion
  ultérieure `302 → 303` pour PUT/PATCH/DELETE seulement. Une
  redirection substituée n'est jamais une continuation de la méthode
  d'origine - le client doit émettre un GET - donc Suprnova dit `303`
  directement au lieu de laisser les visites GET sur un 302 que le
  client suivrait avec le verbe d'origine.
- **`Inertia::location($url)` fait deux méthodes ici, pas une.**
  `location(url)` conserve le contrat toujours-`409` de Laravel - elle
  est antérieure à la forme consciente de la requête, et des
  consommateurs épinglés à une étiquette dépendent du fait que cette
  forme ne change pas. `location_for(&req, url)` est la forme plus
  récente, consciente de la requête : `409` pour un XHR Inertia,
  simple `302` pour une navigation dure. Prenez `location_for` dans le
  code neuf.
- **`Inertia::clearHistory()` fait aussi deux méthodes ici, pas une.**
  `.clear_history()` sur le builder marque une seule réponse ;
  `App::clear_history()` flashe le flag dans la session pour qu'il
  survive à une redirection. Laravel s'en tire avec une seule méthode
  parce qu'elle est déjà adossée à la session - Suprnova garde la
  forme locale à la réponse comme défaut (pas de dépendance à la
  session) et fait du cas inter-redirection un choix explicite.

## Suivant

- [Composants de page](frontend-pages.md) - comment le frontend
  résout un nom de composant vers un module Svelte / React / Vue
- [Types TypeScript](frontend-typescript-types.md) -
  `suprnova generate-types` émet des définitions TS depuis vos structs
  `#[derive(InertiaProps)]`
- [Objets de données](data.md) - `#[derive(Data)]` pour des DTO avec
  un filtrage include/allowlist par champ qui se compose avec les
  rechargements partiels
- [Modèle d'erreur](error-model.md) - comment `Response`, la limite de
  panique, et `FrameworkError` traversent les réponses Inertia
- [Conteneur de service](container.md) - le modèle de recherche
  derrière `App::inertia_share*` et `InertiaSharedData`

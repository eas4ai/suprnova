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

- **Le `&req` en tête est requis.** La macro lit les en-têtes
  `X-Inertia`, l'URL, et les en-têtes de filtrage de rechargement
  partiel depuis la requête, elle a donc besoin de la valeur de la
  requête (ou d'une référence). Sans lui, les rechargements partiels
  casseraient silencieusement.
- **L'existence du composant est vérifiée à la compilation.** La
  macro cherche `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}` ;
  si aucun fichier ne correspond, la construction échoue avec une
  suggestion « vouliez-vous dire… ? » tirée des noms de fichiers réels
  sur le disque. Les chemins imbriqués fonctionnent de la même
  façon - `inertia_response!(&req, "Admin/Dashboard", …)` se résout en
  `frontend/src/pages/Admin/Dashboard.svelte` (ou l'extension de votre
  frontend).
- **La macro se développe en un `Result` `await`é.** Votre handler
  doit retourner [`Response`](error-model.md) (qui est
  `Result<HttpResponse, HttpResponse>`) ou un autre type qui absorbe
  `FrameworkError` via `?` / `From`. Les échecs pendant la
  sérialisation des props ou la construction de la réponse sont
  retournés comme `Err`, pas comme des panics.

### Props façon JSON

Pour le prototypage et les petites pages, vous pouvez sauter la struct
typée :

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

La macro valide quand même le fichier du composant. Le compromis est
que vous perdez la chaîne de props typées - pas de
`#[derive(InertiaProps)]`, pas de génération TypeScript automatique,
pas de vérification à la compilation que la forme attendue du frontend
correspond.

### Redéfinition facultative de la config

La macro accepte un `InertiaConfig` facultatif en fin de liste pour
des redéfinitions par réponse (des réglages SSR différents, un titre
par défaut personnalisé pour une page) :

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

La plupart des applications enregistrent une seule config à
l'amorçage via [`Inertia::install`](#amorçage-inertia-install) et ne
touchent jamais à cet argument.

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
optional, deferred, fusionnable, mis en cache côté client, flash,
redéfinitions de chiffrement d'historique - utilise le builder
directement :

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy : la closure ne s'exécute que quand la prop sera
        // réellement envoyée (visite initiale, ou rechargement
        // partiel qui demande cette clé).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional : jamais envoyée sur les visites initiales ; le
        // client doit explicitement demander la clé via
        // X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer : sautée au rendu initial ; le client émet un XHR de
        // suivi et la closure s'exécute alors.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge : ajoute à l'existant sur les rechargements partiels
        // (« charger plus »).
        .merge("rows", next_page().await?)
        // Once : mis en cache côté client à travers les navigations ;
        // résolveur sauté sur les visites suivantes sauf si le
        // serveur force un rafraîchissement.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash : toast à usage unique ; apparaît sous `page.flash`,
        // pas sous `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Méthode | Objectif | Correspond à Laravel |
|---|---|---|
| `.with(k, v)` | Prop eager, respecte le filtrage de rechargement partiel | prop typée |
| `.always(k, v)` | Prop eager, ignore les filtres de rechargement partiel | `Inertia::always(…)` |
| `.lazy(k, ‖)` | Le résolveur ne s'exécute que quand la prop sera envoyée | closure `fn () => …` |
| `.optional(k, ‖)` | Jamais à la visite initiale ; doit être demandée explicitement | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Sautée à la visite initiale ; un XHR de suivi déclenche la résolution | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combine avec l'état client existant sur les rechargements partiels | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | Le client met en cache à travers les navigations | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate` (via `Inertia::paginate`) | Pagination à défilement infini | `Inertia::scroll(…)` |
| `.flash(k, v)` | Valeur à usage unique sous `page.flash` (pas `props`) | `session()->flash(…)` |
| `.title(…)` | `<title>` par défaut pour la coque HTML | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Chiffrement d'historique par réponse | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Force la rotation de la clé d'historique | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Garde `#fragment` après une visite Inertia | `Inertia::preserveFragment()` |

Les méthodes de builder eager ont des homologues `try_*` (`try_with`,
`try_always`, `try_merge_with`, `try_scroll`, `try_flash`) qui
retournent `Result<Self, FrameworkError>` quand l'impl `Serialize`
d'une valeur pourrait échouer à l'exécution - les méthodes
infaillibles convertissent la panique en un 500 via
[la limite de panique](error-model.md), donc utilisez les `try_*`
quand vous préférez traiter l'échec explicitement.

### Stratégies de fusion et défilement infini

`.merge` (ajout), `.merge_prepend` et `.deep_merge` couvrent les cas
courants de « charger plus ». Pour une fusion différentielle - mettre
à jour les lignes que le client détient déjà au lieu de les
dupliquer - utilisez `.merge_with` avec un `MergeStrategy` explicite
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
l'objet page comme `matchPropsOn`), si bien qu'une nouvelle
récupération qui recoupe la fenêtre courante remplace les lignes
correspondantes sur place plutôt que d'en ajouter des copies.
`Prepend` et `Deep` prennent le même `match_on`.

Le défilement infini est la même mécanique, avec des métadonnées de
pagination attachées. `.scroll` / `.scroll_with` - ou `.paginate`, qui
adapte directement un `LengthAwarePaginator` ou un `CursorPaginator` -
émettent `scrollProps` à côté des données, et le composant
`<InfiniteScroll>` du client pilote les récupérations
suivante/précédente :

```rust
// `posts` est un CursorPaginator venant du query builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

Le framework lit la direction de fusion depuis l'en-tête de requête
`X-Inertia-Infinite-Scroll-Merge-Intent` que le client envoie
(`append` en défilant vers le bas, `prepend` en défilant vers le
haut). Sur une visite fraîche - pas d'en-tête d'intention -
`scrollProps["posts"].reset` vaut `true`, si bien que le client vide
son accumulateur avant de rendre la première fenêtre.

## Rechargements partiels

Le client Inertia 3 peut demander un sous-ensemble des props d'une
page (ou un sur-ensemble en incluant une clé Optional ou Defer). Le
protocole utilise trois en-têtes de requête :

| En-tête | Signification |
|---|---|
| `X-Inertia-Partial-Component` | Le composant en cours de rechargement partiel - doit correspondre au composant de la réponse pour que le filtrage s'applique. |
| `X-Inertia-Partial-Data` | Liste blanche : clés de prop séparées par des virgules à inclure. |
| `X-Inertia-Partial-Except` | Liste noire : clés de prop séparées par des virgules à exclure. L'emporte sur `Partial-Data` en cas de collision de clé. |

Règles de filtrage :

- Les props `Eager`, `Lazy`, `Merge`, `Once`, `Scroll` suivent la
  sémantique liste blanche / liste noire.
- Les props `Always` sont envoyées quoi qu'il arrive.
- Les props `Optional` et `Defer` ne sont jamais présentes sur une
  visite standard et n'apparaissent que sur un rechargement partiel
  correspondant qui liste explicitement la clé.

Le handler n'a rien de spécial à faire - enregistrez chaque prop via
le builder, et le framework consulte les en-têtes en sérialisant
l'objet page.

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

Les données flash sont un état à usage unique qui doit apparaître au
prochain rendu puis disparaître - messages toast, ID « vient d'être
créé », résumés de validation. Suprnova les fait apparaître sous
`page.flash` sur chaque réponse Inertia. Il y a trois façons de
l'écrire :

```rust
// 1. Empile dans le flash bag de la requête courante.
App::flash("toast", "Saved");

// 2. Attache à une réponse spécifique (même effet sur cette réponse uniquement).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Transporte à travers une redirection via la façade Redirect.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

La forme `Redirect::with(key, value)` est le chemin inter-handlers :
la valeur atterrit dans la session sous `_flash.new.*`, le
[`SessionMiddleware`](csrf.md) de la requête suivante la fait vieillir
en `_flash.old.*`, et l'`InertiaResponse` de la destination la fait
apparaître sous `page.flash`.

Le flash de même requête (le sac task-local) l'emporte sur le flash
de session hérité en cas de collision de clé, si bien qu'un handler de
destination peut redéfinir une valeur entrante simplement en
re-flashant la clé.

Les clés de session internes (tout ce qui est préfixé par `_`) sont
filtrées hors de `page.flash` - `_old_input` pour la repopulation de
formulaire et les flags de protocole `_inertia.*` ne fuitent pas vers
le client.

### Helpers de redirection

`Redirect` est la surface Laravel complète :

```rust
Redirect::to("/dashboard")                       // 302 vers un chemin
Redirect::route("posts.show").with("id", "42")   // route nommée, params de route
Redirect::back("/")                              // URL précédente enregistrée en session
Redirect::refresh()                              // même URL, GET frais
Redirect::guest(&req, "/login")                  // met de côté l'URL prévue
Redirect::intended("/dashboard")                 // dépile l'URL mise de côté
Redirect::signed_route("downloads.show", &[("id","42")])?  // URL signée
Redirect::to("/posts/42").preserve_fragment()    // garde #frag à travers la visite
```

Toutes les variantes de `Redirect` acceptent `.with(k, v)`,
`.with_input(map)`, `.with_errors(map)`, `.with_errors_bag(name, map)`,
`.cookie(c)`, `.header(k, v)`, `.permanent()`, `.status(303)`, etc. La
chaîne complète reflète le `RedirectResponse` de Laravel.

Pour les visites Inertia non-GET, le framework convertit
automatiquement la réponse en `303 See Other` quand
[`Inertia303Middleware`](#amorçage-inertia-install) est installé, si
bien que le navigateur émet un GET de suivi propre au lieu de
resoumettre le PUT/PATCH/DELETE d'origine à la cible de redirection.

## Détection de version

Inertia versionne le manifeste d'actifs, pour qu'un client de longue
durée n'essaie pas de monter une page depuis le bundle d'hier contre
le serveur d'aujourd'hui. Quand l'en-tête `X-Inertia-Version` du
client ne correspond pas à la version configurée du serveur,
[`InertiaVersionMiddleware`](#amorçage-inertia-install) répond par
`409 Conflict` et un en-tête `X-Inertia-Location` nommant la nouvelle
URL - le client Inertia le récupère et fait un rechargement complet de
page, récupérant le nouveau bundle.

Vous réglez la version via `InertiaConfig` :

```rust
use suprnova::InertiaConfig;

// Statique - la plupart des apps. Fige un identifiant au moment de la
// construction.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamique - lit un hash de manifeste, un ID de déploiement de
// conteneur, n'importe quoi. La closure s'exécute à chaque
// vérification de version ; mettez en cache à l'intérieur si ce n'est
// pas bon marché.
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

Pour une résolution de version async ou faillible (par exemple lire un
hash de manifeste depuis S3), faites la lecture une fois à l'amorçage
et passez la `String` mise en cache à `.version(...)`.

## Amorçage : `Inertia::install`

La plupart des applications installent les deux middlewares de
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

`Inertia::install` retourne `Result` et, dans l'ordre :

1. Échoue de manière fermée si `cfg` se résout au mode production
   (`development == false` - la valeur par défaut chaque fois que
   `APP_ENV=production`) mais qu'aucun manifeste Vite ne peut être
   chargé depuis `cfg.manifest_path`. C'est le garde-fou CFG-01 : un
   démarrage en production avec un frontend non construit échoue
   explicitement au lieu de revenir silencieusement à un chemin
   d'actif hérité codé en dur.
2. Enregistre `InertiaVersionMiddleware` - émet le `409` +
   `X-Inertia-Location` quand le client et le serveur ne sont pas
   d'accord sur la version des actifs.
3. Enregistre `Inertia303Middleware` - fait passer `302` à `303` sur
   les redirections Inertia non-GET.

Ne sautez l'appel que si vous ne voulez vraiment pas l'un de ces
middlewares (rare ; les deux ferment des modes de défaillance réels -
bundle périmé silencieux et rejeu de formulaire sur redirection).

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

Suprnova parle à un worker SSR hors-process - typiquement le bundle
`createServer()` de `@inertiajs/{svelte,react,vue}/server` exécuté
sous Node / Bun / Deno - via une boucle locale HTTP. Activez-le sur la
config :

```rust
InertiaConfig::new()
    .ssr("http://127.0.0.1:13714")  // worker URL
    .ssr_timeout(std::time::Duration::from_millis(500))
    .ssr_exclude("/admin/**")
    .ssr_max_response_bytes(8 * 1024 * 1024)
```

SSR est désactivé par défaut. Une fois activé, le framework poste
l'objet page à `<url>/render` et intègre `{ head, body }` dans la
coque HTML. En cas d'erreur ou de timeout du worker, la réponse
retombe sur le CSR (un `<div id="app">` vide que le client hydrate) et
le hook `on_ssr_error(...)` se déclenche ; basculez
`ssr_throw_on_error(true)` en CI pour faire de ces échecs des 500 durs
à la place.

Démarrez le worker séparément - `suprnova ssr:start` est le runner
standard une fois que votre projet a un point d'entrée SSR.

## Configuration

Le comportement d'Inertia est configuré de façon programmatique via
`InertiaConfig`. La seule variable d'env que le framework lit
directement est `SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`), qui
choisit le nom de fichier du point d'entrée par défaut et les
extensions de composant de page. Tout le reste prend la forme d'un
builder :

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)              // redéfinit SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)            // plafonne le fan-out des props lazy
    .production();                           // false → charge depuis le serveur de dev Vite
```

Réglages par défaut selon le frontend :

| Frontend | Point d'entrée par défaut | Extensions de page |
|---|---|---|
| Svelte (défaut) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

Le manifeste Vite à `manifest_path` est chargé paresseusement à la
première requête et mis en cache pour la durée de vie du process.
Quand il est absent, les balises d'actifs de production retombent sur
un chemin hérité codé en dur, et un `tracing::warn!` se déclenche pour
que l'écart apparaisse dans les journaux.

### Pourquoi Suprnova diverge

L'adaptateur Inertia de Laravel a un unique registre global de
« données partagées », plus un appel `Inertia::share($k, $v)` par
requête. Le modèle requête-par-process de PHP rend cela sûr : un
process frais par requête signifie aucune fuite entre visiteurs
concurrents.

Le modèle de process de Rust est l'inverse - un seul process sert de
nombreuses requêtes concurrentes à travers de nombreux threads. Donc
le registre vit sur le [conteneur](container.md) (task-local →
thread-local → global), pas dans des statiques globales au process.
`App::inertia_share*` écrit dans l'`InertiaRegistry` du conteneur
actif, ce qui donne aux tests utilisant `TestContainer::fake()` une
isolation propre sans avoir à désenregistrer quoi que ce soit. Même
surface que Laravel ; mécanique différente en dessous, parce que le
runtime est différent.

Deux autres choix façonnés par Rust valent la peine d'être signalés :

- **Les résolveurs de props lazy s'exécutent en concurrence**,
  plafonnés par `max_concurrent_resolvers` (16 par défaut). Une page
  avec douze props lazy émet douze requêtes parallèles à l'intérieur
  d'une seule tâche Tokio - c'est exactement pour cela que nous avons
  construit le framework par-dessus Tokio. Ajustez le plafond si une
  page a de nombreuses props lazy qui tapent chacune un service
  externe.
- **La vérification de composant à la compilation** n'est absolument
  pas une fonctionnalité de Laravel, parce que PHP ne peut pas voir
  vos fichiers frontend à la compilation. Suprnova le peut, si bien
  qu'une faute de frappe dans `inertia_response!("Dashbaord", …)` fait
  échouer la construction avec une suggestion « vouliez-vous dire
  Dashboard ? » au lieu de surgir plus tard comme un « composant
  introuvable » à l'exécution.

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

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

Pour une page sans aucune logique  -  à propos, conditions, confidentialité  -
sautez entièrement le handler et déclarez la route :

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new().inertia("/about", "About", json!({ "team_size": 4 }));
```

Voir [Routage](routing.md#router-level-redirects-and-views). Le composant y
est une chaîne au runtime, il ne bénéficie donc pas de la vérification
d'existence à la compilation de cette macro  -  c'est le compromis pour ne pas
écrire le handler.

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
| `.always_with(k, ‖)` | Résolveur async, ignore les filtres de rechargement partiel | `Inertia::always(fn () => …)` |
| `.lazy(k, ‖)` | Le résolveur ne s'exécute que si la prop sera envoyée | closure `fn () => …` |
| `.optional(k, ‖)` | Jamais à la visite initiale ; doit être demandée explicitement | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Sautée à la visite initiale ; un XHR de suivi déclenche la résolution | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combine avec l'état client existant sur les rechargements partiels | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | Le client met en cache à travers les navigations | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.scroll_with_wrapped` / `.paginate` (via `Inertia::paginate`) | Pagination à défilement infini | `Inertia::scroll(…)` |
| `.flash(k, v)` | Valeur ponctuelle sous `page.flash` (pas `props`) | `session()->flash(…)` |
| `.title(…)` | `<title>` par défaut pour la coquille HTML | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Chiffrement d'historique par réponse | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Force la rotation de la clé d'historique sur **cette** page | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Garde le `#fragment` après une visite Inertia | `Inertia::preserveFragment()` |

Les méthodes eager du builder ont des homologues `try_*` (`try_with`,
`try_always`, `try_merge_with`, `try_scroll`, `try_scroll_wrapped`,
`try_flash`) qui retournent `Result<Self, FrameworkError>` quand l'impl
`Serialize` d'une valeur peut échouer à l'exécution  -  les méthodes infaillibles
convertissent la panique en 500 via [la limite de
panique](error-model.md), alors prenez `try_*` quand vous préférez gérer
l'échec explicitement.

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

### Composer des flags sur une même prop

Les méthodes ci-dessus définissent chacune un flag. Une prop peut en porter
plusieurs, et certaines combinaisons correspondent au fonctionnement que le
protocole Inertia attend des pages réelles : une liste deferred qui s'ajoute
à ce que le client a déjà rendu, une prop merge que le client conserve en
cache à travers les navigations, une prop optional avec sa propre clé de
cache. Construisez la prop avec `Prop`, puis attachez-la avec
`.prop(key, prop)` :

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::lazy(|| async { json!([{ "id": 1 }]) })
        .defer()
        .merge()
        .match_on("id"),
)
```

Cette prop est sautée au premier rendu et annoncée sous `deferredProps`. Le
client émet sa requête de suivi, le résolveur s'exécute, et la valeur arrive
avec une instruction `mergeProps` : elle est donc ajoutée à la liste déjà à
l'écran plutôt que de la remplacer.

Les flags se répartissent en cinq groupes :

| Groupe | Méthodes | Effet |
|---|---|---|
| Visibilité | `.always()`, `.optional()`, `.defer()` | Mutuellement exclusifs ; le dernier appel l'emporte |
| Détail de defer | `.group(name)`, `.rescue()` | Lus seulement lorsque la prop est deferred |
| Fusion | `.merge()`, `.prepend()`, `.deep_merge()`, `.match_on(fields)`, `.merge_with_path(path)` | Manière dont le client replie la valeur, et au chemin concerné |
| Cache client | `.once()`, `.as_key(key)`, `.until(ms)`, `.fresh()` | Indique si le client conserve la valeur à travers les navigations |
| Défilement | `.scroll(metadata)`, `.scroll_wrap(key)` | Entrée `scrollProps` de défilement infini avec métadonnées de fusion inconditionnelles ; `.scroll_wrap` n'est lu que lorsque `.scroll` est défini |

Les sources sont `Prop::eager(value)`, `Prop::lazy(closure)`,
`Prop::from_resolver(resolver)` pour un résolveur que vous avez construit
vous-même, et `Prop::absent()` pour une prop qui n'atteint jamais la réponse :
c'est ce que retourne `when_loaded!` pour une relation non chargée.

Deux règles méritent d'être connues avant de composer :

- **La visibilité est un réglage, pas trois flags.** `.always().optional()`
  produit une prop optional, et `.optional().always()` une prop always. Ce
  n'est une erreur dans aucun des deux cas ; l'appel antérieur est effacé.
- **Les métadonnées suivent les listes de rechargement partiel, pas la
  valeur.** Les entrées `mergeProps`, `onceProps` et `scrollProps` d'une prop
  sont émises chaque fois que la clé passe
  `X-Inertia-Partial-Data` et `X-Inertia-Partial-Except`, y compris lors
  d'une visite où la valeur elle-même est retenue. C'est ce qui transporte
  l'instruction de fusion à travers les deux requêtes d'une prop deferred.
  Deux conséquences en découlent :
  - Une prop `.always().merge()` hors de l'ensemble demandé envoie toujours sa
    valeur et n'envoie pas son instruction de fusion ; le client remplace donc
    au lieu d'ajouter.
  - `scrollProps` possède une condition supplémentaire, en plus des listes :
    une prop `.scroll().defer()` annonce son instruction de fusion lors d'une
    visite non partielle mais n'y envoie aucun curseur, car aucun élément n'est
    encore affiché auquel un curseur puisse se rapporter. Chaque rechargement
    partiel correspondant reçoit le curseur, que cette requête résolve ou non
    aussi la valeur.
  - `deferredProps` est le seul bloc que les listes ne régissent jamais. Il
    est supprimé entièrement lors de tout rechargement partiel correspondant,
    indépendamment de ce que disent les listes : `resolveDeferredProps` de
    Laravel retourne `[]` dès que la requête est partielle. Un rechargement
    partiel correspond au client qui traite les annonces qu'il possède déjà ;
    réannoncer les clés qu'il a laissées hors de cet aller-retour le renverrait
    les chercher. Un rechargement partiel ciblant un *autre* composant est une
    visite standard pour toutes les barrières, annonces incluses.

`.group(name)` et `.rescue()` sont stockés sur toute prop, mais lus seulement
lorsque la prop est deferred ; `.rescue().defer()` et `.defer().rescue()`
signifient donc la même chose. Une prop de défilement prend sa direction de
fusion depuis l'en-tête client `X-Inertia-Infinite-Scroll-Merge-Intent`, de
sorte que `.merge()` et `.prepend()` sur une prop de défilement sont
redondants et ne sont pas lus. `.deep_merge()` est l'exception : elle route
la prop vers `deepMergeProps` plutôt que vers `mergeProps`, de la même manière
que `ScrollProp` de Laravel.

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
        MergeStrategy::Append { match_on: Some(vec!["id".into()]) },
    )
```

`match_on` nomme le ou les champs sur lesquels le client déduplique (émis
dans l'objet de page comme `matchPropsOn`)  -  un champ ou plusieurs, comme
`Prop::match_on` ci-dessous  -  si bien qu'un rechargement qui chevauche la
fenêtre courante remplace les lignes correspondantes sur place plutôt que
d'ajouter des copies. `Prepend` et `Deep` prennent le même `match_on`.

`MergeStrategy` est la forme en un appel. `Prop::merge()` / `.prepend()` /
`.deep_merge()` / `.match_on(field)` sont les mêmes réglages comme flags
séparés, lorsqu'une prop doit aussi porter un flag de visibilité ou de cache ;
voir [Composer des flags sur une même
prop](#composer-des-flags-sur-une-même-prop).

`.match_on` prend un champ ou plusieurs en un appel  -
`.match_on(["id", "slug"])` et `.match_on("id").match_on("slug")` émettent le
même `matchPropsOn`.

Pour ne fusionner qu'une partie de la valeur d'une prop plutôt que sa totalité,
nommez le champ imbriqué avec `.merge_with_path` :

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::eager(json!({ "data": next_page, "meta": meta }))
        .merge()
        .merge_with_path("data")
        .match_on("data.id"),
)
```

`mergeProps` porte alors `"posts.data"` au lieu de `"posts"`, de sorte que
seul `props.posts.data` se replie dans ce que le client détient déjà  -
`props.posts.meta` est remplacé directement, comme toute prop sans fusion.
Les appels s'accumulent : une prop avec deux champs fusionnables peut les
nommer indépendamment. Nommer un chemin désactive entièrement la fusion à la
racine pour cette prop ; une prop avec fusion par chemin ne fusionne jamais
aussi sa valeur entière. `match_on` se compose avec un chemin en incluant le
chemin dans le nom de champ (`"data.id"`, non `"id"`) ; le framework ne
l'infère pas à votre place. `.deep_merge()` ignore `.merge_with_path`  -  une
fusion profonde récursive couvre déjà chaque champ imbriqué, aucun chemin ne
peut donc la restreindre.

La valeur d'une prop merge peut aussi venir d'un résolveur, par
`.merge_lazy` / `.merge_lazy_with`  -  les homologues résolveurs de `.merge` /
`.merge_with` :

```rust
InertiaResponse::new("Feed/Index").merge_lazy("posts", || async {
    Ok::<_, FrameworkError>(load_next_page().await?)
})
```

Le résolveur ne s'exécute que si la prop merge doit réellement être envoyée  -
elle est sautée par le filtrage de rechargement partiel et par `.defer()`,
comme toute prop issue d'un résolveur.

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

Une prop de défilement porte toujours des métadonnées de fusion, pas seulement
lors d'une récupération de suivi : elle ajoute à la fin par défaut, et ne
bascule vers le début que lorsque l'en-tête client
`X-Inertia-Infinite-Scroll-Merge-Intent` le demande (`append` en défilant vers
le bas, `prepend` vers le haut). `reset` est indépendant de cet en-tête : il
vaut `true` exactement lorsque le client a nommé la clé dans `X-Inertia-Reset`,
le même en-tête que lit une prop merge ordinaire. Une visite neuve non filtrée
n'envoie aucun de ces en-têtes ; elle reçoit donc `reset: false` et une
instruction append, conformément à Laravel.

`.merge_with_path` n'a aucun effet sur une prop de défilement : le bloc de
défilement qui calcule son instruction de fusion lit la clé d'enrobage unique
de `Prop::scroll_wrap`, non la liste de chemins accumulés par
`.merge_with_path`, de sorte que `.scroll(metadata).merge_with_path("data")`
stocke un chemin que rien ne lit. `.scroll_wrap`  -  accessible directement via
`.prop(...)` ou via le raccourci de réponse `.scroll_wrapped` ci-dessous  -
est l'équivalent d'imbrication pour une prop de défilement.

Une prop de défilement respecte aussi `.match_on(...)`, comme toute prop
merge ; passez par `.prop(...)`, car ni `.scroll` ni `.match_on` ne possèdent
de raccourci combiné au niveau de la réponse :

```rust
InertiaResponse::new("Users/Index").prop(
    "users",
    Prop::eager(rows)
        .scroll(ScrollMetadata::new("page").current(1).next(2))
        .match_on("id"),
)
```

La clé de champ de correspondance suit l'endroit où la prop fusionne
réellement : la clé nue sans enrobage (`matchPropsOn: ["users.id"]`), ou
`key.wrap_key` sous `.scroll_wrap(...)` (`matchPropsOn: ["posts.data.id"]`
pour une prop enrobée sous `"data"`). L'entrée s'aligne donc toujours sur le
chemin de fusion que replie le client, au lieu de ne jamais correspondre
silencieusement.

Quand la valeur de la prop est elle-même une structure enrobée  -
`{ data: [...], meta: {...} }`, forme typique d'une ressource API construite à
la main  -  fusionner l'objet entier écraserait `meta` à chaque récupération.
Dirigez plutôt la fusion vers le champ tableau avec `.scroll_wrapped` :

```rust
InertiaResponse::new("Feed/Index").scroll_wrapped(
    "posts",
    "data",
    ScrollMetadata::new("page").current(2).next(3),
    serde_json::json!({ "data": rows, "meta": { "total": total } }),
)
```

`mergeProps` nomme alors `posts.data`, ainsi le client replie les nouvelles
lignes dans le tableau imbriqué et laisse `meta` être remplacé en entier à
chaque fois. `.scroll_with_wrapped` et `try_scroll_wrapped` sont les
homologues à résolveur et faillible, à l'image de `.scroll_with` /
`try_scroll`.

Un type extérieur au module `pagination` de cette crate  -  un paginateur tiers,
un curseur fait à la main  -  peut se décrire à `.scroll` en implémentant
`ProvidesScrollMetadata` au lieu de construire `ScrollMetadata` champ par
champ :

```rust
use suprnova::{ProvidesScrollMetadata, ScrollMetadata};

impl ProvidesScrollMetadata for MyCursorPage {
    fn page_name(&self) -> String { "cursor".to_string() }
    fn previous_page(&self) -> Option<serde_json::Value> { self.prev.clone().map(Into::into) }
    fn next_page(&self) -> Option<serde_json::Value> { self.next.clone().map(Into::into) }
    fn current_page(&self) -> Option<serde_json::Value> { Some(self.current.clone().into()) }
}

InertiaResponse::new("Feed/Index").scroll("posts", page.scroll_metadata(), page.rows)
```

`LengthAwarePaginator`, `Paginator` et `CursorPaginator` l'implémentent aussi.
Voir [Pagination](pagination.md#inertia-integration-infinite-scroll-props).

### Imbrication par notation à points

Une clé contenant `.` s'imbrique dans la réponse au lieu d'être envoyée comme
clé littérale  -  notation à points de Laravel fondée sur `Arr::set`
(`Inertia::share('user.name', …)`, `resolveArrayableProperties`) :

```rust
InertiaResponse::new("Dashboard")
    .with("user.name", "Todd")
    .with("user.locale", "es")
```

est envoyée comme :

```json
{ "user": { "name": "Todd", "locale": "es" } }
```

et non comme deux clés littérales `"user.name"` / `"user.locale"`. Deux appels
qui partagent un préfixe s'accumulent dans un seul objet ; une clé sans point
n'est pas affectée. Cela s'applique à chaque méthode qui attache une prop  -
`.with`, `.always`, `.lazy`, les clés du registre partagé  -  et à rien d'autre :
cela ne parcourt jamais récursivement la *valeur* d'une prop, donc un objet de
validation `errors` conserve les noms de champs pointés qu'il porte
intérieurement. Il n'existe aucun mécanisme d'échappement pour une clé qui
doit conserver un point littéral (`.with("config.json", …)` s'imbrique tout de
même) ; cela correspond à Laravel, où `Arr::set` ne possède pas non plus de
mécanisme d'échappement.

## Rechargements partiels

Le client Inertia 3 peut demander un sous-ensemble des props d'une
page (ou un sur-ensemble, en incluant une clé Optional ou Defer). Le
protocole utilise trois en-têtes de requête :

| En-tête | Signification |
|---|---|
| `X-Inertia-Partial-Component` | Le composant faisant l'objet du rechargement partiel - il doit correspondre au composant de la réponse pour que le filtrage s'applique. |
| `X-Inertia-Partial-Data` | Liste blanche : clés de props à inclure, séparées par des virgules. |
| `X-Inertia-Partial-Except` | Liste noire : clés de props à exclure, séparées par des virgules. L'emporte sur `Partial-Data` en cas de collision de clé. |

Le filtrage ne lit qu'une chose : la visibilité de la prop, définie par
`.always()`, `.optional()` ou `.defer()`. Une prop qui n'a aucune de ces
formes possède la visibilité par défaut.

- Les props de visibilité par défaut suivent la sémantique liste blanche /
  liste noire.
- Les props `.always()` sont envoyées quoi qu'il arrive.
- Les props `.optional()` et `.defer()` ne sont jamais envoyées lors d'une
  visite standard et n'apparaissent que dans un rechargement partiel
  correspondant qui liste explicitement la clé.

Les flags de fusion et de défilement n'entrent pas en ligne de compte : ils
décident comment le client replie une valeur qu'il reçoit, non s'il la reçoit.
Une prop `.defer().merge()` est donc filtrée exactement comme une prop
`.defer()` simple. `.once()` n'entre pas non plus en ligne de compte, bien
qu'elle ne soit pas une instruction de repliage pure : lors d'une visite
complète où le client indique que la valeur est déjà en cache, le serveur
saute le résolveur et n'envoie aucune valeur, comme l'explique la remarque
ci-dessous. Les trois modifient les blocs de métadonnées qui les accompagnent.
Voir [Composer des flags sur une même
prop](#composer-des-flags-sur-une-même-prop).

Le handler n'a rien de particulier à faire - enregistrez chaque prop
via le builder, et le framework consulte les en-têtes au moment de
sérialiser l'objet de page.

Le cache côté client d'une prop `once` n'est respecté que lors d'une
visite Inertia **complète**. Sur un rechargement partiel qui nomme la
clé (`router.reload({ only: ['stats'] })`), le résolveur s'exécute et
la valeur est envoyée - le client a demandé précisément parce qu'il en
veut une fraîche, et respecter là sa prétention de cache périmé ne
retournerait rien du tout pour la clé qu'il a demandée.

### `only` / `except` imbriqués (notation à points)

Les entrées `X-Inertia-Partial-Data` et `X-Inertia-Partial-Except` peuvent
nommer un chemin à l'intérieur de la valeur d'une prop, pas seulement la clé
de la prop elle-même. Un client qui appelle
`router.reload({ only: ['user.name'] })` envoie
`X-Inertia-Partial-Data: user.name`, et la réponse restreint la prop `user` à
ce seul champ :

```json
{ "props": { "user": { "name": "Ada" } } }
```

`except` élague de la même façon au lieu de restreindre  -
`router.reload({ except: ['user.email'] })` laisse tous les autres champs de
`user` en place.

Règles :

- Une entrée nue (`user`) désigne toujours la prop entière. Si `only` nomme à
  la fois `user` et `user.name`, la valeur entière est envoyée  -  l'entrée nue
  l'emporte.
- Une entrée peut aussi nommer un *ancêtre* d'une clé de prop pointée. Une prop
  enregistrée sous `auth.user`  -  par `.with("auth.user", …)` ou
  `App::inertia_share("auth.user", …)`  -  participe à `only: ['auth']` et est
  envoyée entière, car l'appelant a demandé toute la racine `auth`. Un
  `except: ['auth']` nu la supprime pour la même raison. Le préfixe doit se
  terminer sur une limite de segment, donc une prop sans rapport
  `authAgent.user` n'est affectée par aucune des deux formes.
- `except` l'emporte pour un chemin nommé par les deux en-têtes, comme au
  niveau supérieur.
- Un chemin qui ne se résout pas dans la valeur  -  un champ inconnu, ou un
  chemin qui traverse un scalaire ou un tableau plutôt qu'un objet  -  ne
  contribue rien pour ce chemin, sans supprimer les champs frères demandés à
  ses côtés.
- Les props `Always` ignorent entièrement `only` / `except`, notation à points
  incluse : elles sont toujours envoyées entières.
- Les props `Optional` et `Defer` exigent toujours la demande explicite pour
  se résoudre. Une entrée pointée (`permissions.read`) compte comme cette
  demande pour la clé de niveau supérieur, et la valeur résolue est restreinte
  de la même manière que celle d'une prop `Eager`.
- Un `only` pointé contre une prop dont la valeur courante n'est pas un objet
  (une chaîne, un nombre ou un tableau) se restreint à `{}`, non à la valeur
  d'origine. La réconciliation du client ne fait une fusion profonde que si
  *la valeur en cache et la valeur entrante* sont des objets
  (`inertia-3.6.1/packages/core/src/response.ts` `nestedTopKeys`) ; un objet
  vide échoue ce test contre un cache non-objet comme le ferait un objet rempli,
  il remplace donc directement le scalaire en cache au lieu de se fondre dans
  lui. Évitez une demande pointée contre une prop qui n'a pas la forme d'un
  objet.
- Un `except` pointé ne supprime pas le champ chez le client : il empêche le
  champ de se rafraîchir dans cette réponse, et la fusion du client le restaure
  depuis ce qu'il possédait déjà en cache. `deepMergeObjects` construit
  l'objet fusionné en clonant d'abord la valeur en cache, puis en n'écrasant
  que les clés que le serveur a effectivement envoyées ; une clé élaguée par
  le serveur n'est jamais touchée et survit avec son ancienne valeur. Lors du
  tout premier chargement par le client de cette prop (rien encore en cache),
  le champ élagué est réellement absent puisqu'il n'existe aucun cache de
  repli  -  le comportement « restauré depuis le cache » ne s'applique qu'à une
  page que le client a déjà vue.

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

Les clés partagées s'imbriquent sur les points de la même manière que
`.with` : deux partages statiques sous `"user.name"` / `"user.age"` arrivent
en un seul objet `user` sur le réseau. Relisez une valeur partagée, ou videz
entièrement le registre statique, avec `App::inertia_shared` /
`App::flush_inertia_shared`  -  les `Inertia::getShared` / `Inertia::flushShared`
de Laravel :

```rust
use suprnova::App;

App::inertia_share("user.name", "Todd");
assert_eq!(App::inertia_shared("user.name"), Some(serde_json::json!("Todd")));

App::flush_inertia_shared();
assert_eq!(App::inertia_shared("user.name"), None);
```

`inertia_shared` lit seulement le registre statique : il retourne `None` pour
une clé enregistrée par `inertia_share_lazy` / `inertia_share_once` (il n'y a
aucune requête pour en résoudre une, à l'image de `getShared` de Laravel, qui
retourne la closure brute au lieu de l'invoquer) et pour un partage de
fournisseur de trait par requête. `flush_inertia_shared` ne vide aussi que le
registre statique ; un fournisseur enregistré par `register_inertia_shared`
n'a aucun état par requête à vider.

Pour des données partagées par requête (l'utilisateur authentifié, des flags
à portée de requête), implémentez
[`InertiaSharedData`](#données-partagées-par-requête) et enregistrez le
singleton : le framework appelle `share(&req, component)` sur chaque réponse
Inertia et fusionne le résultat. `component` est la page rendue, si bien qu'un
fournisseur peut faire varier sa sortie selon la page  -  voir ci-dessous.

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

Le trait s'exécute une fois par réponse Inertia avec accès à la requête
**et** au nom du composant de page  -  le `RenderContext` de Laravel
(`component`, `request`), passé comme paramètre simple plutôt que dans une
struct enveloppe puisque la requête couvre déjà l'autre moitié. Les
implémentations ont besoin d'`async_trait` (ré-exporté comme
`suprnova::__async_trait`) et d'`IndexMap` (ré-exporté comme
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
        component: &str,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        // Vary by page: only the admin dashboard needs the nav counts.
        if component == "Admin/Dashboard" {
            out.insert("pendingReviews".into(), Prop::eager(serde_json::json!(12)));
        }
        Ok(out)
    }
}

// Dans bootstrap :
App::register_inertia_shared(Arc::new(AuthShare));
```

Ignorez `component` (`_component`) si votre fournisseur n'a pas besoin de
varier selon la page.

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

### Échecs de validation

Lorsqu'un handler échoue à la validation lors d'une visite Inertia, le
framework répond `303 See Other` vers la page du formulaire avec les erreurs
flashées, au lieu du JSON `422` qu'obtient un client REST. Ce n'est pas
cosmétique : le client Inertia traite toute réponse sans en-tête `X-Inertia`
comme non-Inertia et la rend dans la modale d'erreur plein écran ; un `422`
n'atteint donc jamais `form.errors`. Rien ne change dans le handler : le pont
est l'un des middlewares enregistrés par `Inertia::install`.

La destination est le `Referer` de la requête lorsqu'il est de même origine,
puis l'URL précédente enregistrée dans la session, puis l'URL de la requête
en échec elle-même. Un `Referer` d'origine croisée est ignoré plutôt que suivi,
comme celui qui ne paraît même origine qu'en apparence : un préfixe `//` ou
`/\` (un navigateur lit l'un ou l'autre comme relatif au protocole après avoir
replié la barre oblique inverse en barre oblique) et tout octet de contrôle
ASCII n'importe où dans la valeur (l'analyseur d'URL retire tabulation et saut
de ligne de la chaîne entière avant de comparer les origines, si bien qu'un
octet de contrôle peut transformer ce qui semble être un chemin sûr en une
origine différente au moment où un navigateur navigue) utilisent tous deux le
même repli. La même vérification s'applique aussi au repli final vers l'URL,
afin qu'un chemin de requête inhabituel ne puisse pas devenir une redirection
hors origine.

La valeur d'un champ est son **premier** message, une chaîne simple  -  la forme
que décrit le propre type `ErrorValue` d'Inertia et à laquelle se lie
`$page.props.errors.email`. Définissez `InertiaConfig::with_all_errors(true)`
pour obtenir à la place tous les messages sous forme de tableau ; le type côté
client a alors besoin de l'augmentation correspondante :

```ts
// global.d.ts
import '@inertiajs/core'

declare module '@inertiajs/core' {
  export interface InertiaConfig {
    errorValueType: string[]
  }
}
```

Plusieurs formulaires sur une page restent isolés : envoyez
`X-Inertia-Error-Bag: <name>` avec la visite, et les erreurs sont flashées
sous ce bag puis relues sous lui, arrivant comme `errors.<name>.<field>`.

La prop `errors` est toujours visible par défaut ; un rechargement partiel ne
la filtre ni ne la restreint jamais. `only: ['users']` envoie toujours le bag,
et `except: ['errors']` aussi ; `only: ['errors.email']` envoie le bag entier
plutôt que ce seul champ. C'est la forme de Laravel : son middleware partage
le bag comme `Inertia::always(...)`, et `resolveAlways` réinjecte la valeur
brute après la reconstruction `only` / `except`. C'est important parce que le
client replie une réponse partielle avec `{...current.props, ...response.props}` :
un objet `errors` vide effacerait les messages déjà à l'écran, tandis qu'un
objet non filtré les laisse corrects. La règle couvre les deux sources  -  le bag
flashé dans la session et le propre `.with("errors", …)` d'un handler. Un flag
de visibilité explicite l'emporte toujours ; ainsi
`.prop("errors", Prop::eager(…).optional())` se comporte optionnellement.

Cela ne fait pas deux choses. Il ne re-flashe pas l'ancienne entrée : le corps
de requête est déjà consommé lorsque le pont s'exécute, et un `useForm`
d'Inertia conserve son propre état après une soumission en échec, il n'y a
donc rien à repeupler. Et il ne touche jamais une réponse Precognition : un
`422` de simulation est exactement ce que le client a demandé.

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

Par défaut, vous ne définissez rien : `InertiaConfig` hache votre manifeste de
build Vite (`manifest_path`, par défaut
`public/assets/.vite/manifest.json`) et utilise les 16 premiers octets de son
SHA-256, encodés en hexadécimal. Le manifeste est le seul fichier qui change à
chaque build et en aucune autre occasion ; la version s'incrémente donc
d'elle-même. Lorsqu'aucun manifeste ne peut être lu  -  développement local où
Vite sert depuis la mémoire  -  elle retombe sur la chaîne statique `"1.0"` et
journalise au niveau `debug`.

Redéfinissez-la lorsque vous voulez autre chose :

```rust
use suprnova::{InertiaConfig, VersionResolver};

// Default - hash the build manifest. Nothing to write.
let cfg = InertiaConfig::new();

// A different manifest location; the version follows it.
let cfg = InertiaConfig::new().manifest_path("dist/.vite/manifest.json");

// Static - bake in a build-time identifier. Survives a later
// `.manifest_path(...)` call: an explicit version is deliberate.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamic - a container deployment id, anything. The closure runs on
// every version check; cache inside if it isn't cheap.
let cfg = InertiaConfig::new().version_with(|| deployment_id());
```

Le manifeste est lu à chaque vérification de version, comme le fait aussi
`hash_file` de Laravel  -  quelques Ko depuis le cache de pages, et un build est
pris en compte immédiatement. Si vous avez mesuré ce coût et voulez le
supprimer, résolvez une fois à l'amorçage :

```rust
use suprnova::{InertiaConfig, VersionResolver};

let version = VersionResolver::from_manifest("public/assets/.vite/manifest.json").resolve();
let cfg = InertiaConfig::new().version(version);
```

Pour une résolution de version asynchrone ou faillible (par ex. lire un hash
de manifeste depuis S3), faites la lecture une fois à l'amorçage et passez la
`String` mise en cache à `.version(...)`.

## Amorçage : `Inertia::install`

La plupart des applications installent les middlewares de protocole en
un seul appel, depuis `register_http_stack`  -  le hook d'amorçage HTTP seul,
exécuté par le chemin serveur et ignoré par les binaires de file, de
planification, de flux de travail et de console (voir
[Amorçage](bootstrap.md)) :

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)
        .expect("Inertia install failed (production needs a built frontend manifest)");
    // …le reste de vos middlewares globaux, dans l'ordre où vous voulez
    // qu'ils s'exécutent
}
```

Tout ce dont la couche Inertia dépend - `SessionMiddleware` - et tout ce
qu'une page d'erreur a besoin de lire - `LocaleMiddleware` - se place
*au-dessus* de cet appel. Voir [les règles d'ordre
ci-dessous](#amorçage-inertia-install).

```rust
// cmd/main.rs
Application::new()
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
```

Gardez-le hors de `bootstrap::register`. `Inertia::install` échoue de façon
fermée en production lorsque le manifeste frontend construit est absent, ce
qui correspond exactement à l'état d'une image de worker ou de console qui ne
livre aucun `public/assets` : l'installer depuis le hook à l'échelle du
processus rendrait aussi ces binaires indisponibles.

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
4. Enregistre `Inertia303Middleware`  -  promeut `302` en `303` sur les
   redirections Inertia non-GET.
5. Enregistre `InertiaValidationRedirectMiddleware`  -  transforme un `422`
   lors d'une visite Inertia en un `303` de retour vers la page du formulaire
   avec les erreurs flashées. Voir [Échecs de validation](#échecs-de-validation).
6. Enregistre `InertiaErrorPageMiddleware`, **uniquement lorsque** `cfg`
   nomme un `.error_page(...)` - transforme les réponses d'erreur propres
   au framework en cette page. Voir [Pages d'erreur](#pages-d-erreur).

L'ordre compte : le middleware d'en-têtes est enregistré en premier, il est
donc le plus externe et voit chaque réponse  -  y compris le `409` que le
middleware de version retourne avant même que le handler ne s'exécute. Le
middleware de redirection de validation est enregistré en dernier, il est donc
le plus interne  -  le plus proche du handler  -  et voit un `422` avant que les
trois autres middlewares aient l'occasion de le toucher.

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

Enregistrez [`LocaleMiddleware`](localization.md) **avant lui aussi**, si
vous utilisez une [page d'erreur](#pages-d-erreur). Le code d'un
middleware situé après `next` s'exécute une fois que tout ce qui se
trouve à l'intérieur a déjà rendu la main : le middleware de page
d'erreur fait donc son rendu après que toute portée ouverte à l'intérieur
a été dépilée - ce qui, pour le middleware de locale, signifie que la
page recevrait la locale par défaut de l'application au lieu de celle du
visiteur. La couche Inertia ne lit rien de la localisation, mettre la
locale à l'extérieur ne coûte donc rien. Le `bootstrap.rs` scaffoldé le
fait déjà. Le même raisonnement vaut pour tout middleware à vous dont la
page d'erreur a besoin de lire la portée de requête.

Ne sautez l'appel que si vous ne voulez véritablement pas l'un de ces
middlewares (c'est rare ; chacun ferme un vrai mode d'échec  -
empoisonnement de cache entre les deux représentations d'une URL, bundle
périmé silencieux, rejeu de formulaire sur redirection, et un `422` de
validation qui aboutit dans la modale d'erreur du client au lieu d'atteindre
`form.errors`).

## Pages d'erreur

Une visite Inertia qui reçoit en retour un code non-2xx du framework
n'affiche pas une page d'erreur - elle affiche un écran de plantage :

```
All Inertia requests must receive a valid Inertia response, however a
plain JSON response was received.
```

Le client ne vérifie qu'une chose avant d'accepter de rendre quoi que ce
soit : un en-tête `X-Inertia: true` sur la réponse. Un `403` venu d'un
contrôle d'[autorisation](authorization.md) ou d'un middleware de
permission RBAC, un `404` pour un chemin sans route, un `429` du
[limiteur de débit](rate-limiting.md), un `500` d'un
[handler en échec](errors.md) - tous portent le corps d'erreur JSON du
framework et aucun en-tête de ce genre, si bien que le client les confie
à sa modale. Un utilisateur au mauvais rôle clique sur un lien de
navigation et l'application semble cassée.

Nommez un composant de page et le framework fera plutôt passer ces
réponses par lui, en conservant le code de statut :

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    Inertia::install(
        &InertiaConfig::new()
            .version(env!("CARGO_PKG_VERSION"))
            .error_page("Error"),
    )
    .expect("Inertia install failed (production needs a built frontend manifest)");
}
```

`"Error"` est résolu exactement comme n'importe quel autre nom de page :
`frontend/src/pages/Error.svelte` (ou `.tsx`, ou `.vue`) suffit donc.
**Les trois starters en livrent une et posent déjà `.error_page("Error")`** -
un nouveau projet est couvert sans rien faire.

Une règle d'ordre l'accompagne : **enregistrez `LocaleMiddleware` avant
`Inertia::install`**, sinon les pages d'erreur s'affichent dans la locale
par défaut de l'application plutôt que dans celle du visiteur. La page
d'erreur est construite au retour, après que chaque middleware enregistré
à l'intérieur de la couche Inertia a rendu la main et dépilé la portée
qu'il avait ouverte. Le `bootstrap.rs` scaffoldé s'en charge
correctement ; si vous avez écrit le vôtre, vérifiez-le. Il en va de même
pour tout middleware à vous, à portée de requête, que lisent les props
partagées de la page d'erreur.

### Ce que la page reçoit

| Prop | Type | Toujours présent | Ce que c'est |
|---|---|---|---|
| `status` | `number` | oui | Le statut HTTP d'origine - `403`, `404`, `500`. |
| `message` | `string` | oui | Le `message` du corps d'erreur, ou le libellé du statut lorsqu'il n'en portait aucun. Déjà assaini : un `5xx` affiche `"Internal Server Error"`, jamais l'erreur sous-jacente - et cela vaut aussi sous `APP_DEBUG=true`. Le champ `debug_message`, réservé au développement, que le chemin JSON ajoute dans ce cas, n'est délibérément pas lu : l'erreur brute reste donc dans le journal et dans la réponse JSON, et ne s'affiche jamais dans une page. |
| `request_id` | `string` | non | Présent uniquement lorsque le corps d'erreur en portait un. Le même id que celui enregistré par le journal structuré, si bien que la page peut afficher une référence que l'opérateur pourra rechercher. |

```svelte
<script lang="ts">
  interface ErrorProps {
    status: number
    message: string
    request_id?: string
  }

  let { status, message, request_id }: ErrorProps = $props()
</script>

<h1>{status}</h1>
<p>{message}</p>
{#if request_id}<p>Reference: {request_id}</p>{/if}
```

Déclarez les props dans le composant plutôt que de les importer depuis
`types/inertia-props.ts` :
[`suprnova generate-types`](frontend-typescript-types.md) réécrit ce
fichier à partir de vos propres structs `#[derive(InertiaProps)]`, et ces
props-ci viennent du framework.

### Ce qui survit au remplacement

Le code de statut est conservé, et chaque en-tête posé par la réponse
d'origine l'est aussi, **sauf** deux groupes.

**Ce qui décrivait le corps remplacé.** Chaque champ `Content-*` (un
`Content-Length` sur une page quatre fois plus grosse que le JSON qu'elle
remplace est un bug de cadrage) et `Transfer-Encoding`.
`Content-Security-Policy` est nommément exclu de cette règle - il partage
le préfixe par accident historique et relève de la politique de réponse,
pas des métadonnées de représentation.

**Ce qui régissait la façon dont ce corps pouvait être stocké.**
`Cache-Control`, `Expires`, `Age`, `ETag`, `Last-Modified`. La page porte
vos props partagées - `auth.user`, le flash, le partage de locale - là où
le corps d'erreur qu'elle remplace était le même pour tout le monde ;
elle ne doit donc jamais hériter de l'autorisation d'être stockée par un
cache partagé puis remise à un autre visiteur, ni de validateurs qui
appartiennent à une entité qu'elle n'est pas. Elle pose à la place
`Cache-Control: no-cache, private` pour elle-même, le même défaut que
Laravel donne à une réponse porteuse de session.

Tout le reste est conservé : `Retry-After` sur un `429` dit toujours au
client quand revenir, `WWW-Authenticate` sur un `401` porte toujours le
challenge, et `Vary`, `Set-Cookie` et votre en-tête d'id de requête
arrivent intacts. La règle est énoncée en termes de ce qui est retiré
plutôt que de ce qui est gardé, si bien qu'un en-tête dont le framework
n'a jamais entendu parler survit au lieu de disparaître en silence.

Les deux publics sont couverts. Une visite XHR Inertia reçoit l'objet de
page JSON avec `X-Inertia: true` ; une navigation dure - quelqu'un qui
colle `/admin/articles` dans la barre d'adresse - reçoit la coquille HTML
complète, la même qu'au premier chargement de n'importe quelle page. La
page d'erreur fonctionne donc que l'utilisateur soit arrivé par la SPA ou
non.

### Ce à quoi le middleware ne touche jamais

Le middleware n'intervient que là où personne d'autre n'a de réponse. Il
laisse de côté :

- **Les `422` de validation.** `InertiaValidationRedirectMiddleware` en
  est propriétaire - voir [Échecs de validation](#échecs-de-validation).
  Un `422` qui survit à ce middleware (pas d'objet `errors`, ou un essai
  à blanc de Precognition) conserve son corps lui aussi.
- **Tout ce qui porte `X-Inertia-Location`.** Le rebond de version `409`,
  et la forme `redirect_to` des middlewares RBAC. Le client agit sur
  l'en-tête, pas sur le corps.
- **Les redirections.** Seule la plage `400`-`599` est concernée.
- **Les clients d'API.** Une requête dont l'`Accept` préfère
  `application/json` à `text/html` conserve le contrat JSON qu'elle a
  toujours eu. Le `*/*` de `curl` compte comme une absence de préférence,
  il conserve donc le JSON lui aussi. Seule une visite Inertia ou une
  navigation de navigateur obtient une page.
- **Les réponses qui sont déjà des pages Inertia.** Un handler qui a
  rendu sa propre page et lui a donné un `410` conserve son propre
  composant.
- **Les corps qui n'ont pas la forme d'erreur du framework.** Votre
  propre page d'erreur HTML, du texte brut qui n'est pas le
  `404 Not Found` propre au routeur, ou une enveloppe JSON dont les clés
  diffèrent - aucun de ces cas n'est remplacé.
- **Tout, lorsque `error_page` n'est pas défini.** Le middleware n'est
  alors pas enregistré du tout : une application qui n'y a pas adhéré
  exécute donc exactement le code qu'elle exécutait avant.

### Quels corps sont réécrits

Le critère, c'est la **forme du corps**, pas son auteur. À un statut
`400`-`599`, exactement trois formes sont remplacées :

- un corps vide ;
- un objet JSON dont le `message` est une chaîne - l'enveloppe d'erreur
  propre au framework, et tout ce qui lui ressemble ;
- le corps en texte brut fixe `404 Not Found` du routeur.

Tout le reste passe sans changement. Cela veut dire qu'un `401` auquel un
middleware à vous répond par
`HttpResponse::json(json!({ "message": "Unauthenticated." }))` **devient
bel et bien** la page d'erreur - ce qui est précisément le but, puisque
c'est exactement la réponse que le client passerait sinon à sa modale -
et cela veut dire que seuls `message` et `request_id` survivent dans les
props. Une enveloppe porteuse d'`errors`, de `code` ou d'autre chose perd
ces champs en devenant une page.

Si un middleware à vous doit conserver son propre corps JSON sur un
statut d'erreur, donnez-lui une forme que le critère ne reconnaît pas -
mettez le texte lisible par un humain sous une clé autre que `message` -
ou posez vous-même `X-Inertia: true` sur la réponse, ce qui la marque
comme étant déjà une réponse Inertia et la met hors de portée. L'un comme
l'autre tient en une ligne, à l'endroit qui construit la réponse.

Une lacune à connaître : un handler qui **panique** est hors d'atteinte.
La limite de panique enveloppe toute la chaîne de middleware, si bien que
le `500` synthétisé est construit après que chaque cadre de middleware a
déjà été dépilé. Les handlers qui paniquent font toujours apparaître la
modale du client. Retournez `Err(...)` plutôt que de paniquer (voir
[Gestion des erreurs](errors.md)) et la page d'erreur les couvre.

Si la page elle-même échoue à s'afficher - le composant ne peut pas être
résolu, le SSR est indisponible, une prop partagée échoue -, le framework
consigne un `warn` avec l'id de requête et retourne la réponse d'erreur
d'origine. Une page d'erreur cassée ne masque jamais l'erreur qu'elle
était en train de rendre.

### Pourquoi Suprnova diverge

Laravel place cela dans le handler d'exceptions : vous éditez
`bootstrap/app.php`, vous faites vous-même le `match` sur le statut, et
vous appelez
`Inertia::render('Error', ['status' => $response->getStatusCode()])`
avec `$response->setStatusCode(...)` pour remettre le code en place.
C'est souple, et c'est aussi une pièce de plomberie du framework que
chaque projet réécrit à la main, en général après avoir vu la modale en
production.

Ici, c'est une ligne de configuration, parce que la décision est la même
pour toutes les applications : une visite Inertia ou une navigation de
navigateur obtient une page, un client d'API obtient du JSON, et tout ce
qu'un autre contrat possède est laissé intact. Le compromis, c'est que la
règle est fixe plutôt qu'un `match` que vous écrivez : sortir une réponse
particulière du lot revient donc à lui donner un corps que le critère ne
reconnaît pas, ou à la marquer comme déjà-Inertia - voir
[Quels corps sont réécrits](#quels-corps-sont-réécrits).

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
async fn show(RouteParam(post): RouteParam<Post>, req: Request) -> Response {
    inertia_response!(&req, "Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    })
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

Avant même de lancer une requête, la passerelle peut vérifier que le bundle SSR
construit existe sur le disque : activez cette vérification avec
`.ssr_bundle_path(...)`, pointé vers le chemin conventionnel
`frontend/bootstrap/ssr/ssr.js` (la vérification elle-même est active par
défaut, `.ssr_ensure_bundle_exists(true)`, mais n'a aucun effet tant qu'un
chemin n'est pas défini ; il n'est délibérément pas détecté automatiquement,
afin qu'activer SSR contre un double de test n'impose pas aussi de stubber un
bundle sur disque). Un bundle absent retombe immédiatement sur le CSR, sans
payer `ssr_timeout` pour une connexion qui ne pouvait jamais réussir. Cela
reflète la configuration `ensure_bundle_exists` de Laravel.

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")
        .ssr_bundle_path("frontend/bootstrap/ssr/ssr.js")
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

`suprnova new` scaffolde `frontend/src/ssr.{ts,tsx}` et un script npm
`build:ssr` pour chaque starter. Construisez-le, puis démarrez le worker :

```bash
cd frontend && npm run build:ssr
suprnova ssr:start
```

`suprnova ssr:check` vérifie que le worker répond réellement : il appelle sa
propre route `GET /health`, que tout bundle `createServer()` expose sans code
supplémentaire.

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
    .with_all_errors(false)                   // one message per field, or all
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

Neuf autres choix propres à Rust méritent d'être signalés :

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
- **`.lazy()` n'est pas le `Inertia::lazy()` de Laravel.** La méthode de
  Laravel est dépréciée et se comporte comme `optional()`  -  `LazyProp` est un
  alias direct de `OptionalProp`, entièrement sauté lors de la visite initiale
  (`ResponseFactory.php:174-181`). Le `.lazy()` de Suprnova est la convention
  de closure simple que Laravel utilise elle-même pour une prop callable sans
  aucun wrapper : elle est incluse chaque fois que le filtrage de rechargement
  partiel laisse passer la clé, visites standard comprises. Utilisez
  `.optional()` pour le comportement « sauté à la visite initiale » que le nom
  « lazy » suggère si vous venez de Laravel.
- **Les `only` / `except` imbriqués restreignent après la résolution, non
  avant.** Le `Response::resolvePartialProperties` de Laravel parcourt le
  chemin pointé dans le tableau de props brut, non encore résolu : un chemin
  dans un `LazyProp` ou un `DeferProp` se dégrade donc en `null`  -  la
  traversée atteint une closure non résolue et s'arrête
  (`inertia-laravel-2.0.25/src/Response.php:273-297`). Suprnova résout d'abord
  la valeur de chaque prop  -  les résolveurs sont async, il n'existe donc pas
  de point synchrone où ils seraient tous des tableaux simples comme Laravel
  en possède parfois  -  puis restreint la valeur JSON obtenue. Un chemin
  imbriqué inconnu ou de type incompatible est supprimé au lieu d'être renvoyé
  comme `null`, ce qui correspond à ce qu'attend la réconciliation du client :
  elle fusionne profondément un objet restreint avec ce qu'elle détient déjà
  (`inertia-3.6.1/packages/core/src/response.ts:414-425`), et un `null`
  parasite écraserait un champ que le client possède au lieu de le laisser
  intact.
- **`.scroll_wrapped` est opt-in, non automatique.** Le
  `Inertia::scroll($value, $wrapper = 'data', …)` de Laravel imbrique par
  défaut l'instruction de fusion de chaque prop de défilement sous `"data"`,
  parce qu'une ressource de paginateur Laravel retourne typiquement
  `{ data: [...], links: {...}, meta: {...} }` et que seul le tableau devrait
  fusionner. Les paginateurs intégrés de Suprnova renvoient un tableau de
  lignes nu (`Vec<T>`, sans enveloppe) ; `.scroll` / `.paginate` fusionnent
  donc à la racine de la prop, et `.scroll_wrapped` existe pour les cas qui
  nécessitent plutôt le chemin imbriqué.
- **Une prop de défilement enrobée préfixe pour vous ses champs `match_on`.**
  Sur une prop `.scroll_wrapped("posts", "data")`, `match_on("id")` émet
  `"posts.data.id"`. Laravel émet le `"posts.id"` non préfixé, que son propre
  client ne parvient alors pas à aligner sur la cible de fusion : la
  correspondance ne se déclenche jamais silencieusement. Le point
  d'imbrication est ici non ambigu  -  une prop de défilement possède au plus un
  enrobage  -  donc Suprnova dérive le préfixe au lieu de vous le faire saisir.
  Écrivez le nom de champ nu, non le chemin.

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

# Ressources JSON:API

Suprnova fournit une couche de ressources JSON:API pour des API REST
typées. Annotez une struct `#[derive(Data)]` avec
`#[json_resource("type")]` et le framework émet un impl
`IntoJsonResource` qui gère les enveloppes simples, les collections,
les collections paginées, les sparse fieldsets
(`?fields[type]=...`), les documents composés `included`, et les
chaînes `?include=a.b.c` multi-niveaux - tout cela par le même chemin
de code. Les deux façades - `Resource` et `JsonApi` - sont le même
type sous deux noms ; utilisez celle qui correspond au style de votre
maison.

## Définir une ressource

```rust
use suprnova::Data;

#[derive(Debug, Clone, Data)]
#[json_resource("users")]
pub struct UserResource {
    pub id: i64,
    pub email: String,

    // `input_only` garde `password` disponible côté form-request mais
    // le supprime de la sortie API.
    #[data(input_only)]
    pub password: String,

    // Marque un champ comme une *relation* : il n'atterrit jamais dans
    // `attributes`, il produit à la place un objet de relation
    // JSON:API, et il est éligible à `?include=`. Le type du champ
    // doit implémenter `IntoJsonResource` (directement, ou via
    // `Vec<T>` / `Option<T>`).
    #[data(allow_include)]
    pub posts: Vec<PostResource>,
}
```

Le mot-clé `id_field` renomme le champ qui fournit l'`id` JSON:API :

```rust
#[derive(Data)]
#[json_resource("orders", id_field = "uuid")]
pub struct OrderResource {
    pub uuid: String,
    pub total_cents: i64,
}
```

## Rendu des réponses

Construisez une réponse en attente depuis un handler et appelez
`.render().await` :

```rust
use suprnova::{LengthAwarePaginator, Resource};

#[handler]
async fn show_user(id: i64) -> Result<HttpResponse, FrameworkError> {
    let user: UserResource = User::find_or_fail(id).await?.into();
    Resource::single(user).render().await
}

#[handler]
async fn list_users() -> Result<HttpResponse, FrameworkError> {
    let users: Vec<UserResource> = User::all().await?.into_iter().map(Into::into).collect();
    Resource::collection(users).render().await
}

#[handler]
async fn paginate_users() -> Result<HttpResponse, FrameworkError> {
    // `paginate(per_page)` lit automatiquement `?page=` depuis la requête courante.
    let page = User::query().paginate(10).await?;
    // Convertit le paginateur de modèle en paginateur de ressource
    // champ par champ - `data` est `pub`, le reste des compteurs/liens suit.
    let page = LengthAwarePaginator::new(
        page.data.into_iter().map(UserResource::from).collect(),
        page.total,
        page.per_page,
        page.current_page,
    )
    .with_base_url("/api/users");
    Resource::paginated(page).render().await
}
```

`JsonApi::single` / `JsonApi::collection` / `JsonApi::paginated` sont
des points d'entrée alias identiques, si vous préférez l'orthographe
Laravel.

## Mutateurs chaînables

`JsonApiResponse` est un objet en attente. Personnalisez l'enveloppe
avant d'appeler `.render().await`. Chaque mutateur est `self` →
`Self`, ce qui les rend composables :

```rust
use suprnova::{Resource, JsonApiInfo};
use serde_json::json;

let info = JsonApiInfo::new()
    .with_version("1.1")
    .with_ext("https://jsonapi.org/ext/atomic")
    .with_meta("copyright", json!("2026 Acme Inc."));

Resource::single(user)
    .status(201)                                  // redéfinition du statut HTTP
    .with_meta("trace_id", json!("req-7"))        // paire meta de premier niveau
    .with_link("self", "/api/users/1")            // lien de premier niveau
    .with_jsonapi(info)                           // `jsonapi` de premier niveau
    .additional(json!({ "api_version": "2.0" }).as_object().unwrap().clone())
    .render()
    .await
```

| Mutateur | Analogue Laravel | Effet |
|---|---|---|
| `.status(code)` | `ResourceResponse::calculateStatus` | Redéfinit le statut HTTP. |
| `.created()` | `wasRecentlyCreated → 201` | Raccourci pour `.status(201)`. |
| `.with_meta(k, v)` / `.meta(k, v)` | `with($request)` | Paire clé/valeur `meta` de premier niveau. |
| `.with_meta_map(m)` | `with($request)` en masse | Fusionne une map dans le `meta` de premier niveau. |
| `.with_link(rel, href)` / `.link(rel, href)` | `with($request)['links']` | Paire clé/valeur `links` de premier niveau. |
| `.with_link_value(rel, v)` | forme objet-lien | Lien de premier niveau sous forme `{href, meta}`. |
| `.with_additional(k, v)` | `additional($data)` | Clé de premier niveau à côté de `data`. |
| `.additional(map)` | `additional($data)` | Clés additionnelles en masse. |
| `.with_jsonapi(info)` | `JsonApiResource::configure(...)` | Membre `jsonapi` de premier niveau. |

Les membres canoniques (`data`, `included`, `links`, `meta`,
`jsonapi`, `errors`) ne sont jamais écrasés par `.additional(...)`.

## `links` et `meta` par ressource

Redéfinissez les défauts `IntoJsonResource::resource_links` et
`IntoJsonResource::resource_meta` pour attacher des liens ou des
métadonnées à l'*objet ressource*, pas à la racine du document :

```rust
use suprnova::resources::IntoJsonResource;
use serde_json::{Map, Value};

impl IntoJsonResource for MyHandRolledPost {
    // ...

    fn resource_links(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("self".into(), Value::String(format!("/api/posts/{}", self.id)));
        m
    }

    fn resource_meta(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("kind".into(), Value::String("blog".into()));
        m
    }
}
```

Les deux ont pour défaut une `Map` vide pour les ressources dérivées
par macro, si bien que le rendu JSON:API omet les clés quand elles ne
sont pas utilisées. Redéfinissez `resource_top_level_meta` pour faire
remonter des métadonnées par ressource dans le membre `meta` de
premier niveau de l'enveloppe.

## Attributs conditionnels - `Maybe<T>` / `MissingValue<T>`

Utilisez `Maybe` pour omettre un champ de l'objet `attributes` rendu
selon une condition d'exécution. C'est l'analogue Suprnova du
`MissingValue` de Laravel et de la famille `when()` / `whenLoaded()` /
`unless()`.

```rust
use suprnova::{Maybe, MissingValue};

// Les deux noms désignent le même type.
let m1: Maybe<&str> = Maybe::present("email@example.com");
let m2: MissingValue<&str> = MissingValue::missing();
let m3 = Maybe::when(user.is_verified, &user.verified_at);
let m4 = Maybe::unless(user.is_admin, &user.public_handle);
let m5 = Maybe::when_with(expensive_check(), || compute_value()); // paresseux
```

Pour les structs dérivées par macro, déclarez un champ comme `Maybe<T>`
et le rendu le supprime automatiquement quand il vaut `Missing`. Pour
un `resource_attributes` écrit à la main, utilisez le helper
`insert_maybe(map, key, maybe)` :

```rust
use suprnova::resources::{insert_maybe, Maybe};

fn resource_attributes(&self, _fs: Option<&[&str]>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    insert_maybe(&mut map, "email", Maybe::present(&self.email));
    insert_maybe(
        &mut map,
        "phone",
        if self.show_phone { Maybe::present(&self.phone) } else { Maybe::missing() },
    );
    serde_json::Value::Object(map)
}
```

Le rendu appelle aussi `strip_missing_values(&mut value)` sur
l'ensemble de l'objet attributes, si bien que les valeurs
`Maybe::Missing` imbriquées dans des structures serde arbitraires sont
supprimées récursivement - utile quand un transformateur profondément
imbriqué veut omettre des sous-champs.

## Sparse fieldsets

L'`IncludeMiddleware` du framework analyse les paramètres de requête
de forme `?fields[type]=email,name` et les lie à une task-local. Le
`resource_attributes` émis par la macro consulte le jeu de champs et
n'émet que les attributs demandés. Aucun travail côté handler n'est
nécessaire - installez le middleware et la couche de ressources
l'honore automatiquement.

```rust
// Requête : GET /api/users/7?fields[users]=email
// Réponse : { "data": { "type": "users", "id": "7", "attributes": { "email": "alice@example.com" } } }
```

## Documents composés - chaînes `?include=`

Déclarez les champs de relation avec `#[data(allow_include)]`. Le
framework construit un `IncludeTree` à partir de
`?include=author.posts.tags,comments`, parcourt chaque nœud, et pousse
des objets ressource entièrement résolus dans `included`. La
déduplication s'exécute au moment du push via `IncludedSink`, clé par
`(type, id)` selon le §8 du spec JSON:API - si bien qu'une collection
de 1 000 éléments où chaque élément partage le même auteur résout cet
auteur exactement une fois. La mémoire et le CPU au pic restent
proportionnels aux ressources incluses distinctes, pas au fan-in de
relation.

```rust
#[derive(Data)]
#[json_resource("posts")]
pub struct PostResource {
    pub id: i64,
    pub title: String,

    #[data(allow_include)]
    pub author: Option<AuthorResource>,

    #[data(allow_include)]
    pub tags: Vec<TagResource>,
}
```

Une requête qui nomme un chemin d'include absent de l'allowlist de
cette ressource reçoit une enveloppe d'erreurs JSON:API 400.

### Plafond de profondeur

Un chemin d'include peut porter au plus cinq segments.
`?include=a.b.c.d.e.f` est tronqué en `a.b.c.d.e` avant que quoi que ce
soit ne le parcoure, à l'image du `JsonApiResource::$maxRelationshipDepth`
de Laravel. Changez le plafond une fois à l'amorçage :

```rust
// Dans bootstrap::register()
suprnova::max_relationship_depth(3);
```

Le plafond compte parce qu'un graphe de relations peut être cyclique :
`?include=author.posts.author.posts...` coûte davantage de travail à
chaque segment qu'un client tape, et rien d'autre ne le borne que la
longueur de la chaîne de requête. La troncature ne fait que retirer des
segments, jamais en ajouter, et chaque niveau vérifie encore sa propre
allowlist avant de descendre - un chemin tronqué ne peut donc jamais
atteindre des données que le chemin complet ne pouvait pas atteindre.

Une conséquence mérite d'être connue : un segment au-delà du plafond est
abandonné avant que l'allowlist ne le voie. Avec un plafond de 2,
`?include=author.posts.secrets` retourne 200 avec `author` et `posts`
inclus, plutôt que le 400 que vaudrait le chemin complet, parce que
`secrets` n'existe plus au moment où quoi que ce soit le valide.

`max_relationship_depth(0)` désactive entièrement les includes. Le 0 de
Laravel émet quand même le premier saut, parce que son plafonnement ne
s'applique qu'à la queue, une fois le segment de tête détaché ; le 0 de
Suprnova veut dire aucune relation du tout.

### Pourquoi Suprnova diverge

Trois divergences visibles par rapport au `JsonApiResource` de Laravel :

1. **Refus par défaut strict pour `?include=`.** La couche de
   ressources de Laravel ignore silencieusement les chemins d'include
   qui ne se résolvent pas. Suprnova les rejette avec un
   `400 Bad Request` portant une enveloppe d'erreurs JSON:API. La
   posture de refus par défaut du §5.2.2 du spec est le contrat contre
   lequel les clients peuvent programmer ; l'ignorance silencieuse
   cache les bugs client et casse l'intégrité des documents composés.

2. **`.status(code)` / `.created()` explicites plutôt qu'un 201
   automatique.** Laravel positionne automatiquement `201` à partir de
   `wasRecentlyCreated` sur le modèle Eloquent sous-jacent. Suprnova
   découple le DTO de ressource de tout cycle de vie de persistance
   spécifique, si bien que le statut se positionne sur l'objet réponse
   lui-même - `.created()` quand c'est votre intention, `.status(204)`
   quand la réponse est vide, et ainsi de suite. Un seul mutateur reste
   honnête sous n'importe quel flux.

3. **Un plafond de profondeur de `0` désactive entièrement les
   includes.** Laravel ne plafonne que la queue d'un chemin, une fois
   le segment de tête déjà détaché : son `0` émet donc encore le
   premier saut. Suprnova tronque le chemin entier, si bien que
   `max_relationship_depth(0)` veut dire aucune relation du tout - voir
   Plafond de profondeur ci-dessus.

## Pagination

`Resource::paginated(p)` fonctionne avec tout paginateur qui implémente
le trait `Paginated<T>` - `LengthAwarePaginator<T>` et
`CursorPaginator<T>` de `suprnova::pagination` livrent tous deux cet
impl. Le rendu attache automatiquement
`links.{self,first,prev,next,last}` et un bloc `meta.pagination`.

```rust
use suprnova::{LengthAwarePaginator, Resource};

let page = LengthAwarePaginator::new(items, total, per_page, current_page)
    .with_base_url("/api/users");
Resource::paginated(page).render().await
```

## Enveloppes d'erreur

Chaque `FrameworkError` sait se rendre en enveloppe JSON:API
`{"errors": [...]}` via `into_json_api_response()`. Le helper est
exposé parce que `FrameworkError` porte un code de statut, un pointeur
source de nom de champ (pour `ValidationError`), et un jeton de
corrélation request-id sous `meta.request_id`. Les réponses 5xx sont
assainies : le message brut n'atteint jamais le client sauf si
`APP_DEBUG=true` est positionné dans l'environnement actif, auquel cas
il apparaît sous `meta.debug_message`.

```rust
let response = FrameworkError::validation("email", "email is invalid")
    .into_json_api_response();
// {
//   "errors": [{
//     "status": "422",
//     "title": "Validation failed",
//     "detail": "email is invalid",
//     "source": { "pointer": "/data/attributes/email" },
//     "meta": { "request_id": "..." }
//   }]
// }
```

## Résumé des surfaces

| Surface Suprnova | Équivalent Laravel 13 |
|---|---|
| Façades `Resource` / `JsonApi` | `JsonResource::make`, `JsonApiResource` |
| `JsonApiResponse` | `ResourceResponse`, `JsonApiResource::toResponse` |
| `JsonApiBuilder` | (builder interne pour `ResourceResponse`) |
| Trait `IntoJsonResource` | `JsonResource::toArray`, `toAttributes`, `toRelationships`, `toLinks`, `toMeta`, `with` |
| `RelationshipValue` / `ResourceIdentifier` | forme de tableau à l'intérieur de `toRelationships` |
| `IncludeTree` | `?include=` analysé depuis `JsonApiRequest` |
| `RequestFieldsetSet` | `?fields[type]=` analysé depuis `JsonApiRequest` |
| `Maybe<T>` / `MissingValue<T>` | `MissingValue` + `whenLoaded` / `when` / `unless` |
| `JsonApiInfo` | `JsonApiResource::$jsonApiInformation` |
| `JsonApiResponse::status(code)` / `.created()` | `ResourceResponse::calculateStatus` |
| `JsonApiResponse::additional(map)` / `.with_additional(k, v)` | `JsonResource::additional($data)` |
| `JsonApiResponse::with_meta(k, v)` / `.meta(k, v)` | `JsonResource::with($request)['meta']` |
| `JsonApiResponse::with_link(rel, href)` / `.link(rel, href)` | `JsonResource::with($request)['links']` |
| `JsonApiResponse::with_jsonapi(info)` | `JsonApiResource::configure(...)` |
| `current_fieldset()` / `scope_fieldset(...)` | jeu de champs task-local, positionné par `IncludeMiddleware` |
| `IncludeResolutionError` → enveloppe 400 | analyseur `?include=` en mode strict |

Réexports de premier niveau sous `suprnova::` : `Resource`, `JsonApi`,
`JsonApiResponse`, `JsonApiBuilder`, `JsonApiInfo`, `IncludedSink`,
`IntoJsonResource`, `RelationshipValue`, `ResourceIdentifier`,
`IncludeTree`, `RequestFieldsetSet`, `Maybe`, `MissingValue`,
`insert_maybe`, `strip_missing_values`, `AsRelationshipValue`,
`PushIncluded`, `IncludeResolutionError`, `current_fieldset`,
`scope_fieldset`.

## Suivant

- [Sérialisation Eloquent](eloquent-serialization.md) -
  `#[derive(Data)]`, les champs hidden/visible, l'équivalent de
  `toArray` qui alimente les attributs de ressource
- [Relations Eloquent](eloquent-relationships.md) - ce que consomme
  `#[data(allow_include)]` ; les types de relation typés qui
  soutiennent les documents composés
- [Pagination](pagination.md) - `LengthAwarePaginator`,
  `CursorPaginator`, et le trait `Paginated<T>` que consomme
  `Resource::paginated`
- [Objets de données](data.md) - la macro `#[derive(Data)]` partagée
  avec Inertia, le middleware `?include=`/`?fields[type]=`, et les
  motifs `Maybe<T>`
- [Modèle d'erreur](error-model.md) - comment
  `FrameworkError::into_json_api_response` s'inscrit dans le contrat de
  conversion

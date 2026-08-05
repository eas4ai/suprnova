# Pagination

Suprnova livre trois paginateurs qui correspondent à la surface de
Laravel ligne pour ligne : à total connu (connaît le total), simple
(une requête par page), et par curseur (keyset opaque). Les trois
dérivent `Serialize` vers le JSON à la forme Laravel que les
consommateurs Inertia et JSON:API comprennent déjà - vous récupérez
une page et la retournez ; rien d'autre n'est requis.

```rust
use crate::models::User;

let page = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;
```

Cet unique appel exécute le `COUNT(*)` et la récupération de page
`LIMIT/OFFSET`, analyse `?page=N` depuis la requête active, et
retourne un `LengthAwarePaginator<User>` prêt à livrer. Les deux
frères - `simple_paginate(20)` et `cursor_paginate(20)` - retournent
la même forme de valeur avec des compromis différents. Le reste de ce
chapitre traite de lequel choisir, de ce que chacun coûte, et de la
façon dont le JSON arrive.

## Choisir un paginateur

La façon la plus rapide de choisir est le tableau de compromis :

| Méthode | Type | Requêtes / page | Connaît le total ? | À utiliser quand |
|---|---|---|---|---|
| `paginate(n)` | `LengthAwarePaginator<M>` | 2 (`COUNT(*)` + page) | oui | l'UI affiche des pages numériques ou « page 3 sur 17 » |
| `simple_paginate(n)` | `Paginator<M>` | 1 (`LIMIT n+1`) | non | grandes tables ; un bouton « Suivant » suffit |
| `cursor_paginate(n)` | `CursorPaginator<M>` | 1 (`LIMIT n+1`) | non | défilement infini ; pages profondes sur des tables très sollicitées |

La différence de coût compte une fois que votre table est grande.
`COUNT(*)` sur cent millions de lignes est la requête la plus chère de
votre budget de requête. `simple_paginate` économise le count.
`cursor_paginate` économise le count *et* évite le scan linéaire
`OFFSET N` qui pénalise chaque requête de page profonde sur une
grande table - une recherche par curseur est en `O(1)`-ish avec le
bon index, quel que soit l'endroit du jeu de résultats où se trouve
l'utilisateur.

### Pourquoi Suprnova diverge

Les paginateurs de Laravel portent des helpers de construction d'URL -
`nextPageUrl()`, `previousPageUrl()`, le tableau `links` de
descripteurs `{url, label, page, active}` que Blade rend. L'impl
`Serialize` brute de Suprnova émet la tranche de données plus les
compteurs ; la construction d'URL vit sur les constructeurs de forme
de réponse qui possèdent déjà le contexte d'URL :
[`Inertia::paginate`](frontend-inertia-responses.md) attache les
métadonnées de défilement d'Inertia (des identifiants de page, pas
des URL absolues) ; [`Resource::paginated`](eloquent-resources.md)
attache les `links.{self,first,last,prev,next}` JSON:API conformément
à la recommandation JSON:API.

Deux raisons pour cette séparation. Premièrement, l'URL que le client
devrait voir dépend de la surface de protocole qui la rend - Inertia
se cale sur des identifiants de page, JSON:API veut des hrefs
absolues. Deuxièmement, le paginateur ne connaît pas par défaut l'URL
de base de la requête ; les helpers qui la connaissent peuvent
attacher les URL une fois, là où c'est leur place. Si vous avez
vraiment besoin d'URL sur le paginateur nu (enveloppe JSON
personnalisée, payload de télémétrie, assertion de test), appelez
`with_path(...)` et utilisez `url_for_page(n)` - couvert dans la
section [Génération d'URL](#génération-d-url-et-de-chemins).

## `paginate` - avec total connu

```rust
use suprnova::LengthAwarePaginator;
use crate::models::User;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let page: LengthAwarePaginator<User> = User::query()
        .filter("active", true)
        .order_by_desc("created_at")
        .paginate(20)
        .await?;

    Ok(suprnova::json_response!(page))
}
```

Les champs publics de la struct :

```rust
pub struct LengthAwarePaginator<T> {
    pub data: Vec<T>,           // lignes de cette page
    pub current_page: u64,       // indexé à partir de 1
    pub last_page: u64,          // indexé à partir de 1 ; 0 quand total == 0
    pub per_page: u64,
    pub total: u64,              // chaque ligne à travers toutes les pages
    pub from: Option<u64>,       // index de la première ligne de cette page, à partir de 1
    pub to: Option<u64>,         // index de la dernière ligne de cette page, à partir de 1
    pub path: Option<String>,    // URL de base pour url_for_page (facultatif)
}
```

Le JSON que le `Serialize` dérivé émet :

```json
{
  "data": [...],
  "current_page": 1,
  "last_page": 3,
  "per_page": 10,
  "total": 25,
  "from": 1,
  "to": 10,
  "path": "/api/users"
}
```

`path` est omis du JSON quand non défini ; `from` et `to` sont `null`
quand la page est vide (aucune ligne sur cette page, ou la page
demandée est après la dernière page).

### Lire `?page=N` automatiquement

`paginate(n)` lit la page courante depuis `?page=N` sur la requête
active via `Context::query_param`. Les valeurs absentes, vides, non
numériques, et nulles sont plafonnées à `1`. Il n'y a rien à câbler -
si une requête est en portée, le paramètre est lu.

### Plusieurs paginateurs sur une page

Quand une page rend plus d'une liste paginée, donnez à chacune sa
propre clé de chaîne de requête avec `paginate_using` :

```rust
let posts = Post::query()
    .order_by_desc("created_at")
    .paginate_using("posts_page", 10)
    .await?;

let comments = Comment::query()
    .order_by_desc("created_at")
    .paginate_using("comments_page", 25)
    .await?;
```

`paginate_using` définit aussi `page_name` sur le paginateur retourné
pour que `url_for_page` construise des URL avec la même clé :

```rust
posts.url_for_page(2);     // "/posts?posts_page=2"  (quand path est défini)
comments.url_for_page(3);  // "/posts?comments_page=3"
```

### Prédicats de position de page

L'ensemble complet des prédicats de l'`AbstractPaginator` de Laravel
est implémenté :

```rust
page.has_more_pages();   // current_page < last_page
page.on_first_page();    // current_page <= 1
page.on_last_page();     // !has_more_pages()
page.has_pages();        // nous ne sommes pas sur la page 1 OU d'autres pages existent
page.is_empty();         // data.is_empty()
page.is_not_empty();     // !is_empty()
page.count();            // data.len() - tranche de page, pas le total
```

`count()` est la taille de la tranche, pas le total - la forme
`Countable` de Laravel ; pour le total, utilisez directement le champ
`total`.

## `simple_paginate` - une requête, pas de count

```rust
use suprnova::Paginator;
use crate::models::User;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;
```

```rust
pub struct Paginator<T> {
    pub data: Vec<T>,
    pub current_page: u64,
    pub per_page: u64,
    pub has_more: bool,          // y avait-il une ligne supplémentaire au-delà de per_page ?
    pub path: Option<String>,
}
```

JSON :

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true,
  "path": "/api/users"
}
```

L'astuce est dans le SQL. `simple_paginate(20)` émet `LIMIT 21`,
regarde si la 21e ligne est revenue, règle `has_more` en fonction de
cela, et tronque `data` à 20. Une requête par page ; pas de
`COUNT(*)`.

Vous renoncez à `total`, `last_page`, `from`, et `to`. En échange,
vous pouvez paginer des tables où `COUNT(*)` est trop coûteux à
exécuter à chaque chargement de page. La surface UI est des boutons
« Suivant » / « Précédent », pas « page 7 sur 142 ».

Le même ensemble de prédicats que le paginateur à total connu est
implémenté : `has_more_pages()`, `on_first_page()`, `on_last_page()`,
`has_pages()`, `is_empty()`, `is_not_empty()`, `count()`.

## `cursor_paginate` - keyset opaque

```rust
use suprnova::CursorPaginator;
use crate::models::User;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;
```

```rust
pub struct CursorPaginator<T> {
    pub data: Vec<T>,
    pub per_page: u64,
    pub next_cursor: Option<String>,  // None sur la dernière page
    pub prev_cursor: Option<String>,  // None sur la première page
    pub path: Option<String>,
}
```

JSON :

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` et `prev_cursor` sont toujours présents comme clés JSON
(`null` quand absents) pour que les schémas client puissent compter
sur la présence du champ ; `path` est omis quand non défini.

### Comment les curseurs fonctionnent sur le réseau

Le client transmet le curseur de la page précédente via
`?cursor=<opaque>` :

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

`cursor_paginate` décode le curseur, parcourt le filtre keyset
(`pk > boundary ASC` pour `next` ; `pk < boundary DESC` pour `prev`,
puis reconverti en ASC), récupère `LIMIT n+1` lignes, et réémet
`next_cursor` / `prev_cursor` selon que les pages voisines existent.
C'est bidirectionnel - le client peut avancer et reculer sans perdre
sa position.

La pagination par curseur **remplace** tout `ORDER BY` existant sur le
builder. Un ordre de tri total stable sur la clé primaire est requis
pour que le filtre keyset découpe la table de façon déterministe ; un
curseur sur un `ORDER BY random_score()` arbitraire sauterait et
dupliquerait des lignes. Si vous avez besoin d'un tri autre que sur
la PK, passez à `paginate` / `simple_paginate`.

### Les curseurs sont chiffrés et authentifiés

Les curseurs de Suprnova ne sont **pas** le texte en clair
base64-JSON de Laravel. Le curseur réseau est la borne du keyset (un
`sea_orm::Value` typé - `Int`, `BigInt`, `Uuid`, dates-heures,
décimaux, chaînes, octets) plus une étiquette de direction, encodée en
JSON puis scellée avec AES-256-GCM via le porte-clés `Crypt` du
framework (liée à `CryptPurpose::Cursor`, si bien qu'un texte chiffré
de curseur ne peut jamais être rejoué vers une autre surface - cookie,
secret 2FA, cast).

Cela signifie trois choses en pratique :

1. **Pas de falsification.** Un client qui inverse des bits dans
   `?cursor=` obtient un 400 `Invalid pagination cursor`, pas une
   page différente de données.
2. **Pas de fuite d'information.** La valeur de la borne (souvent une
   clé primaire, parfois un timestamp) est scellée à l'intérieur du
   curseur - les clients ne peuvent pas énumérer des plages en
   l'éditant.
3. **Les bornes typées font l'aller-retour sans perte.** L'enveloppe
   réseau étiquette la variante SeaORM (`"BigInt"`, `"Uuid"`, etc.),
   si bien qu'au décodage la valeur se relie avec le même type SQL
   que la colonne d'origine émettait. Aucun bug de coercition de
   chaîne entre Postgres / MySQL / SQLite.

Il n'y a pas de repli en texte en clair. Si `Crypt` n'est pas
initialisé - ce qui devrait être impossible après
`Server::from_config` - l'encodage échoue plutôt que d'émettre un
curseur falsifiable.

### Pourquoi Suprnova diverge

Le paginateur par curseur de Laravel est unidirectionnel (avant
seulement) par défaut, et le curseur réseau est un blob JSON encodé
en base64 - lisible, éditable, rejouable. Le curseur de Suprnova est
bidirectionnel (à l'image de la surface `cursorPaginate()` que
Laravel a ajoutée plus tard) et est authentifié de bout en bout, si
bien que le client ne peut pas en construire ou en altérer un.
L'écosystème Rust a déjà AES-GCM comme primitive ; l'utiliser coûte
au framework un impl de trait supplémentaire et donne à chaque
curseur une propriété de sécurité qu'un payload base64 en texte en
clair ne peut pas offrir.

## La façade - `Pagination::length_aware` / `Pagination::cursor`

La plupart des chapitres de ce manuel montrent la pagination à
travers le builder Eloquent, parce que c'est le chemin courant. Si
vous construisez un `Select<E>` SeaORM directement - disons, en
joignant sur une requête sans modèle pour un rapport - la façade
`Pagination` est la surface équivalente :

```rust
use suprnova::{Pagination, LengthAwarePaginator};
use sea_orm::EntityTrait;

let select = User::find()  // ou n'importe quel Select<E> SeaORM
    .filter(user::Column::Active.eq(true));

let page: LengthAwarePaginator<user::Model> =
    Pagination::length_aware(select, 20, 1).await?;
```

La façade offre aussi `length_aware_on(conn, ...)` et
`cursor_on(conn, ...)` pour router vers une connexion nommée
spécifique, ainsi qu'une forme typée
`cursor(query, cursor, per_page, order_col)` qui prend explicitement
la colonne du keyset - utilisée quand le curseur trie sur autre chose
que la clé primaire.

Les règles de routage correspondent au builder Eloquent. Un
`DB::transaction` ambiant est honoré (le COUNT et la requête de page
s'exécutent tous deux sur la connexion de la transaction), et une
connexion `__read_replica__` enregistrée est utilisée automatiquement
pour les lectures. La sentinelle `__primary__` sélectionne le pool
par défaut quand vous voulez contourner la réplique.

## Validation - `per_page == 0`

Les trois méthodes rejettent `per_page == 0` :

```rust
let result = User::query().paginate(0).await;
assert!(matches!(
    result,
    Err(FrameworkError::ParamError { ref param_name }) if param_name == "per_page",
));
```

L'erreur se rend en HTTP 400 avec le corps d'erreur standard. Il n'y a
pas de « page vide » silencieuse - une taille de page nulle est
toujours fausse et est rejetée au site d'appel, à l'image du builder
Eloquent et de la façade `Pagination`. La même validation vit sur
`cursor_paginate`, `simple_paginate`, `Pagination::length_aware`,
`Pagination::length_aware_on`, `Pagination::cursor`, et
`Pagination::cursor_on` - une seule règle, six points d'entrée.

La valeur `current_page` est **plafonnée**, pas validée : `0` devient
`1`, les nombres négatifs venant d'un frontend défensif ne peuvent pas
se produire (le parseur est `u64`), et tout `?page=N` supérieur à
`last_page` retourne un paginateur avec un `data` vide plus des
`from`/`to` à `None`. Marcher au-delà de la fin est l'erreur du
client, pas une erreur du serveur.

## Forme d'erreur

| Condition | Variante | HTTP |
|---|---|---|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| Curseur falsifié / invalide | `FrameworkError::Domain` (`"Invalid pagination cursor"`) | 400 |
| `Crypt` non initialisé au décodage du curseur | `FrameworkError::Internal` | 500 |
| Incohérence de variante de curseur sur `decode_cursor` | `FrameworkError::Internal` | 500 |
| Échec de BD sous-jacent | `FrameworkError::Database` | 500 |

Le cas du curseur falsifié est celui à retenir. Les curseurs sont lus
directement depuis le réseau - la chaîne de requête `?cursor=…` est
une entrée d'attaquant par définition, et le base64 aux bits inversés
et le texte chiffré rejoué sont des modes d'échec attendus, pas des
bugs serveur. L'étape de déchiffrement se dégrade vers un 400
`Invalid pagination cursor` pour que les échecs déclenchables par le
client ne polluent pas le canal de télémétrie des 500. Le message
statique ne donne rien au client avec quoi sonder.

Les échecs post-déchiffrement (analyse JSON, dispatch de l'étiquette
de variante, analyse de direction) restent en 500 - toute séquence
d'octets qui a survécu à l'authentification AEAD a été produite par
*nous*, donc un payload malformé passé ce point est un bug du
framework qui vaut la peine d'être signalé.

## Génération d'URL et de chemins

Le paginateur brut porte un champ `path` optionnel. Quand il est
défini, `url_for_page(n)` et l'émission de lien de curseur l'utilisent
pour construire des chaînes de requête :

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/api/users");

page.url_for_page(1);    // "/api/users?page=1"
page.url_for_page(2);    // "/api/users?page=2"
```

Quand le chemin de base porte déjà une chaîne de requête, le
séparateur passe à `&` pour que l'URL reste bien formée :

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/users?sort=name");

page.url_for_page(2);    // "/users?sort=name&page=2"
```

Si `path` n'est pas défini, `url_for_page` retombe sur une requête
relative nue : `?page=2`. Le nom du paramètre de page vient de
`with_page_name(...)` (par défaut `"page"`) ; `paginate_using(name, n)`
le règle automatiquement pour que les URL générées utilisent la même
clé que celle depuis laquelle le paginateur a été piloté. Le nom du
paramètre est encodé en form-urlencoded, donc même un nom avec des
caractères réservés ne peut pas corrompre l'URL.

Les paginateurs par curseur ont la même forme : `with_path(...)`
définit la base, `with_cursor_name(...)` redéfinit la clé de requête
(par défaut `"cursor"`), et le builder de liens JSON:API les récupère
automatiquement.

La plupart des apps n'appellent pas `url_for_page` directement -
elles remettent le paginateur à l'une des deux surfaces d'intégration
ci-dessous, qui construisent les URL de la bonne façon pour leur
protocole.

## Intégration Inertia - props de défilement infini

Pour les frontends Inertia, le helper
`Inertia::paginate(component, key, paginator)` attache le paginateur
comme prop de défilement :

```rust
use suprnova::Inertia;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .order_by_desc("created_at")
        .cursor_paginate(20)
        .await?;

    Ok(Inertia::paginate("Users/Index", "users", users).into())
}
```

Les trois paginateurs fonctionnent ici - `LengthAwarePaginator`,
`Paginator`, et `CursorPaginator`. Le nom de page des métadonnées
vient du paginateur lui-même : `"page"` pour les deux paginateurs par
offset, `"cursor"` pour `CursorPaginator`. Le client reçoit les
lignes sous la clé de prop choisie plus un descripteur
`ScrollMetadata` avec `current_page`, `next_page`, `previous_page`
(des identifiants de page pour les paginateurs par offset ; des
chaînes de curseur pour les paginateurs par curseur) - que les
helpers Inertia `useInfiniteScroll` / `WhenVisible` consomment pour
le défilement infini.

`simple_paginate` vaut la peine d'être signalé, parce qu'un listing
sur une table assez grande pour que `COUNT(*)` devienne le coût
dominant de la requête est exactement là où une page de collection
Inertia fait mal :

```rust
let users = User::query()
    .order_by_asc("id")
    .simple_paginate(20)     // pas de COUNT, une requête
    .await?;

Ok(Inertia::paginate("Users/Index", "users", users).into())
```

Son `next_page` vient de la sonde de dépassement `LIMIT n+1` plutôt
que d'une dernière page calculée, puisqu'il n'y a pas de total à
partir duquel en calculer une. Le client obtient « il y a une autre
page » au lieu de « il y a 4 812 pages » - ce qui est tout ce qu'une
UI à défilement infini lit jamais.

### Projeter les lignes avant qu'elles ne sortent

Les paginateurs n'ont pas de `map` / `through` (ceux de Laravel en
ont). Reconstruisez plutôt depuis les champs publics - les compteurs
et les curseurs décrivent la *requête*, donc ils se reportent
inchangés à travers un changement de type de ligne :

```rust
let page = User::query().cursor_paginate(20).await?;

let page = suprnova::CursorPaginator::new(
    page.data.into_iter().map(PublicUser::from).collect(),
    page.per_page,
    page.next_cursor,
    page.prev_cursor,
);
```

Cela vaut la peine de le faire plutôt que de sérialiser le modèle
directement chaque fois que la route n'est pas authentifiée et que le
modèle porte quoi que ce soit que l'appelant ne devrait pas voir. Un
curseur sur une table d'utilisateurs ne distribue qu'une page à la
fois, mais il finit par distribuer chaque page.

Le même helper existe comme méthode chaînable sur
`InertiaResponse::paginate(key, paginator)` si vous voulez mélanger un
paginateur avec d'autres props :

```rust
inertia_response!("Dashboard")
    .with("stats", &stats)
    .paginate("recent_users", users)
    .into()
```

Voir [Réponses Inertia](frontend-inertia-responses.md) pour le modèle
de prop plus large.

## Intégration JSON:API - `Resource::paginated`

Pour les consommateurs JSON:API, `Resource::paginated(paginator)`
construit l'enveloppe complète :

```rust
use suprnova::Resource;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .paginate(20)
        .await?
        .with_path("/api/users");

    Ok(Resource::paginated(users).into())
}
```

La réponse porte :

- `data` - chaque ligne rendue via l'`IntoJsonResource` du modèle.
- `meta.pagination` - `{ total, per_page, current_page, last_page }`
  pour le total connu ; `{ next_cursor, prev_cursor }` pour le
  curseur.
- `links.{self,first,last,prev,next}` - des hrefs absolues pour le
  paginateur à total connu (construites depuis `path`) ;
  `links.{prev,next}` pour le paginateur par curseur.

Les deux types de paginateur implémentent le trait `Paginated<T>` que
`Resource::paginated` consomme - il n'y a pas de chemin de code
séparé entre total connu et curseur. Si vous construisez un type
personnalisé façon paginateur qui implémente `Paginated<T>`, il se
compose de la même façon.

Voir [Ressources JSON:API](eloquent-resources.md) pour le modèle de
ressource.

## Enveloppes JSON personnalisées

Si ni Inertia ni JSON:API ne correspond à votre client, livrez le
paginateur directement via `json_response!` :

```rust
let page = User::query().paginate(20).await?;
Ok(suprnova::json_response!({
    "users": page.data,
    "pagination": {
        "current_page": page.current_page,
        "last_page": page.last_page,
        "per_page": page.per_page,
        "total": page.total,
    }
}))
```

Ou remettez simplement le paginateur entier - l'impl `Serialize`
dérivée émet la forme documentée plus haut :

```rust
Ok(suprnova::json_response!(User::query().paginate(20).await?))
```

Les champs sont publics ; remodelez selon ce que votre contrat exige.

## Routage à travers les connexions

La pagination respecte le même routage multi-connexion que le
builder Eloquent utilise. À l'intérieur d'un `DB::transaction(...)`,
le COUNT et la requête de page s'exécutent tous deux sur la connexion
de la transaction - elles ne se séparent jamais entre connexions, donc
le count n'est jamais en désaccord avec la page qu'il décrivait. Une
`__read_replica__` enregistrée est utilisée automatiquement pour les
lectures hors transaction. Pour épingler un paginateur à une
connexion nommée spécifique, utilisez les variantes
`_on(connection, ...)` sur la façade `Pagination`, ou
`Builder::on("replica_b").paginate(20)` du côté Eloquent.

Voir [Eloquent - routage multi-connexion](eloquent.md) pour le
contrat de routage.

## Quand se tourner vers quoi

Un arbre de décision approximatif :

- **Une UI de pages numériques fait partie du design** →
  `paginate`. Vous avez besoin de `last_page` pour rendre « Page 3
  sur 17 », et le coût du COUNT est acceptable pour la taille de
  votre table.
- **Boutons « Suivant » / « Précédent » seulement, grande table** →
  `simple_paginate`. Une requête par page ; vous renoncez à `total`
  et `last_page`, mais le chargement de page est réduit de moitié.
- **Défilement infini** → `cursor_paginate`. Des curseurs
  bidirectionnels signifient que le client peut continuer à défiler
  au-delà de la page 1000 sans que l'OFFSET ne scanne d'abord des
  milliers de lignes.
- **Queue d'un flux append-only très sollicité** → `cursor_paginate`.
  Le tri keyset par clé primaire est sûr sous la concurrence : les
  nouvelles lignes atterrissent au-delà du curseur, jamais à
  l'intérieur. La pagination basée sur OFFSET saute des lignes sous
  l'effet d'insertions concurrentes.
- **Construire un `Select<E>` hors d'un modèle Eloquent** →
  `Pagination::length_aware` / `Pagination::cursor`. Mêmes
  compromis ; la façade est l'équivalent sans modèle.

En cas de doute, commencez avec `paginate`. Passez à
`simple_paginate` quand le `COUNT(*)` apparaît dans votre journal des
requêtes lentes. Passez à `cursor_paginate` quand les pages profondes
commencent à dominer le temps de requête, ou quand l'UI est un
défilement infini.

## Où réside chaque élément

| Élément | Fichier |
|---|---|
| Façade `Pagination`, trait `Paginated<T>` | `framework/src/pagination/mod.rs` |
| `LengthAwarePaginator<T>` | `framework/src/pagination/length_aware.rs` |
| `Paginator<T>` (simple) | `framework/src/pagination/simple.rs` |
| `CursorPaginator<T>`, `CursorDirection`, `encode_value`, `decode_value` | `framework/src/pagination/cursor.rs` |
| Pont `IntoInertiaScroll` | `framework/src/pagination/inertia.rs` |
| `Builder::paginate` / `simple_paginate` / `cursor_paginate` | `framework/src/eloquent/builder.rs` |
| `Inertia::paginate`, `InertiaResponse::paginate` | `framework/src/inertia/facade.rs`, `framework/src/inertia/response.rs` |
| `Resource::paginated`, `JsonApi::paginated` | `framework/src/resources/response.rs` |

## Suivant

- [Eloquent API](eloquent.md) - la couche modèle qui pilote chaque
  paginateur retourné par `Builder::paginate*`
- [Générateur de requêtes](queries.md) - les requêtes sans modèle qui
  se composent avec `Pagination::length_aware` et
  `Pagination::cursor`
- [Réponses Inertia](frontend-inertia-responses.md) - comment les
  props de défilement attachent les paginateurs aux pages Inertia
- [Ressources JSON:API](eloquent-resources.md) - `Resource::paginated`,
  les links, le meta, et le trait `Paginated<T>`
- [Modèle d'erreur](error-model.md) - la règle de validation
  `FrameworkError::param` et la dégradation en cas de falsification
  du curseur

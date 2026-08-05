# Sérialisation Eloquent

Comment les modèles Eloquent deviennent du JSON. Ce chapitre couvre
`to_array()` et `to_json()`, le pipeline de filtres
`hidden` / `visible` / `appends`, les deux helpers terminaux
`to_array_except` / `to_array_only`, la façon dont `appends` relie les
accesseurs à la sortie, et les deux divergences par rapport à Laravel
qui font trébucher les gens : le piège du contournement de serde, et
le fait que les relations chargées hâtivement ne se replient pas
automatiquement dans le corps JSON.

Si vous avez lu [Eloquent API](eloquent.md), la plupart des noms ici
vous seront familiers - la référence des attributs se trouve dans ce
chapitre. Cette page est celle où vit le *contrat de sérialisation* :
quels champs apparaissent, dans quel ordre les filtres s'appliquent,
et ce qui produit une fuite si vous l'oubliez.

## Table des matières

- [Le contrat](#le-contrat)
- [`to_array` et `to_json`](#to-array-et-to-json)
- [Masquer des champs - `hidden = [...]`](#masquer-des-champs-hidden)
- [Autoriser des champs - `visible = [...]`](#autoriser-des-champs-visible)
- [Ajouter des accesseurs - `appends = [...]`](#ajouter-des-accesseurs-appends)
- [L'ordre du pipeline de filtres](#l-ordre-du-pipeline-de-filtres)
- [Filtrage par appel - `to_array_except` / `to_array_only`](#filtrage-par-appel-to-array-except-to-array-only)
- [Masquage conditionnel selon le visiteur](#masquage-conditionnel-selon-le-visiteur)
- [Le piège du contournement de serde](#le-piège-du-contournement-de-serde)
- [Sérialiser des collections](#sérialiser-des-collections)
- [Relations chargées hâtivement et sérialisation](#relations-chargées-hâtivement-et-sérialisation)
- [Qu'en est-il de JSON:API ?](#qu-en-est-il-de-json-api)
- [Où réside chaque élément](#où-réside-chaque-élément)
- [Suivant](#suivant)

## Le contrat

Chaque struct `#[suprnova::model]` reçoit deux méthodes de
sérialisation du trait `Model` :

```rust
fn to_array(&self) -> serde_json::Value;
fn to_json(&self) -> String;
```

`to_array` produit un `serde_json::Value` utilisable dans les réponses
de handler et les tests. `to_json` est un mince wrapper -
`serde_json::to_string(&self.to_array())` - si bien qu'un seul pipeline
de filtres possède les deux formes.

La sortie est un objet JSON dont les clés sont les noms de champ de la
struct (ou le renommage serde que vous avez appliqué), filtré à
travers trois réglages optionnels déclarés sur `#[model(...)]` :

- `hidden = [...]` - liste noire de colonnes
- `visible = [...]` - liste blanche de colonnes (mutuellement exclusif
  avec `hidden`)
- `appends = [...]` - méthodes accesseurs à injecter sous des clés
  nommées

Quand le modèle ne déclare aucun de ces réglages, le corps par défaut
du trait s'exécute : sérialiser `self` via
`serde_json::to_value(self)`, supprimer deux champs de travail
internes au framework (`__eager` et `__pivot` - voir
[les relations chargées hâtivement](#relations-chargées-hâtivement-et-sérialisation)),
puis retourner le résultat. Quand le modèle en déclare au moins un, la
macro émet une redéfinition qui exécute le
[pipeline](#l-ordre-du-pipeline-de-filtres).

## `to_array` et `to_json`

L'exemple minimal utile - une ligne qui sort en JSON :

```rust
use suprnova::{json_response, model, Model, Request, Response};
use chrono::{DateTime, Utc};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    json_response!(user.to_array())
}
```

`json_response!` accepte n'importe quel `serde_json::Value` ;
`user.to_array()` en produit un. L'équivalent en forme de chaîne est
`user.to_json()` - même corps, mêmes filtres, juste un `to_string` de
plus.

Vous pouvez aussi vous tourner directement vers
`serde_json::to_value(&user)`. **Ne faites pas cela pour quoi que ce
soit qui fait face à l'utilisateur.** Cela contourne entièrement le
pipeline de filtres - voir
[le piège du contournement de serde](#le-piège-du-contournement-de-serde)
plus loin dans le chapitre pour savoir pourquoi.

## Masquer des champs - `hidden = [...]`

La forme liste noire. Chaque colonne sauf celles listées se
sérialise :

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

Le JSON exposé à l'utilisateur pour ce modèle ne contient jamais
`password` ni `remember_token` :

```json
{
    "id": 42,
    "name": "Alice",
    "email": "alice@example.com",
    "created_at": "2026-05-30T11:14:22Z",
    "updated_at": "2026-05-30T11:14:22Z"
}
```

`hidden` est le bon outil quand **la plupart des champs partent vers
le client** et que vous devez soustraire un petit ensemble de secrets,
de flags internes, ou de données réservées à l'authentification.

## Autoriser des champs - `visible = [...]`

La forme liste blanche. Seules les colonnes listées se sérialisent :

```rust
#[model(
    table = "users",
    visible = ["id", "name", "avatar_url"],
)]
pub struct PublicUserView { /* ... */ }
```

Utile pour un modèle qui existe spécifiquement pour être une fine
projection publique (pensez aux types « Profile » / « PublicUser » de
Laravel). `visible` est aussi le bon outil quand la table contient des
dizaines de colonnes internes et que seules quelques-unes partent vers
le client - lister l'ensemble à garder est plus court que lister
l'ensemble à retirer.

`hidden` et `visible` sont **mutuellement exclusifs à la
compilation**. La macro émet une erreur si vous positionnez les deux :

```text
error: cannot specify both `hidden` and `visible` on the same model
 --> src/models/user.rs:7:1
  |
7 | #[model(table = "users", hidden = ["x"], visible = ["y"])]
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

Les deux sont des politiques opposées - choisissez celle dont
l'intention correspond à la forme de votre modèle, pas les deux.

## Ajouter des accesseurs - `appends = [...]`

`appends` injecte des valeurs calculées dans la sortie JSON. Chaque
entrée nomme une méthode taguée `#[accessor]` sur le modèle ; la macro
l'appelle pendant `to_array()` et stocke la valeur retournée sous la
même clé.

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    fillable = ["first_name", "last_name"],
    appends = ["full_name", "initials"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[accessor]
    pub fn initials(&self) -> String {
        let f = self.first_name.chars().next().unwrap_or(' ');
        let l = self.last_name.chars().next().unwrap_or(' ');
        format!("{f}{l}")
    }
}
```

L'utilisateur sérialisé porte désormais les deux clés calculées :

```json
{
    "id": 7,
    "first_name": "Alice",
    "last_name": "Pond",
    "created_at": "...",
    "updated_at": "...",
    "full_name": "Alice Pond",
    "initials": "AP"
}
```

La macro valide les entrées `appends` à la compilation :

- Chaque nom doit s'analyser comme un identifiant Rust
  (`"full-name"` échoue - ce n'est pas un ident valide).
- Si la méthode nommée n'existe pas sur le bloc `impl` du modèle, le
  compilateur désigne le dispatcher généré par la macro avec une
  erreur claire `no method named 'full_name' found`.

Appeler `user.full_name()` directement depuis Rust fonctionne
exactement comme n'importe quelle autre méthode - `appends` contrôle
seulement la **table de dispatch JSON**. Les accesseurs restent des
méthodes ordinaires.

## L'ordre du pipeline de filtres

Quand un modèle déclare `hidden`, `visible`, ou `appends`, la macro
émet une redéfinition de `to_array` qui exécute quatre étapes dans cet
ordre :

1. Sérialiser `self` en `serde_json::Map` via `serde_json::to_value`.
2. Supprimer sans condition les clés internes au framework `__eager`
   et `__pivot` (plus de détails dans
   [la section sur les relations](#relations-chargées-hâtivement-et-sérialisation)).
3. Appliquer `visible` comme **liste blanche** quand elle n'est pas
   vide : toute clé qui n'est PAS dans la liste est retirée.
4. Appliquer `hidden` comme **liste noire** : toute clé listée qui a
   survécu à la liste blanche est retirée.
5. Injecter `appends` : pour chaque entrée, appeler l'accesseur
   enregistré et insérer son résultat sous le nom de l'entrée.

### Pourquoi Suprnova diverge

Laravel exécute le même ordre `hidden` → `visible` → `appends`. La
divergence est à l'étape 5 : dans Suprnova, `appends` s'exécute
**après** la liste noire `hidden`, et ces clés apparaissent toujours -
même si leur nom est aussi listé dans `hidden`. Le raisonnement est le
même que celui de Laravel : si vous déclarez à la fois
`$appends = ['full_name']` et `$hidden = ['full_name']`, l'intention
est « calcule-le et livre-le » - `appends` est le signal le plus
spécifique. L'ordre compte quand la clé d'un accesseur entre en
collision avec un nom de colonne (par exemple un accesseur qui
redéfinit la valeur de la colonne stockée `display_name`) ; c'est
l'accesseur qui l'emporte dans ce qui part vers le client.

## Filtrage par appel - `to_array_except` / `to_array_only`

Pour les cas ponctuels où la déclaration au niveau colonne ne convient
pas, deux helpers terminaux exécutent le pipeline `to_array` complet
puis rognent le résultat par nom :

```rust
use suprnova::{json_response, Model};

pub async fn admin_show(user: User) -> suprnova::Response {
    // retire quelques champs en plus pour un endpoint admin qui a besoin
    // de la plupart de la ligne mais pas de ceux-ci :
    json_response!(
        user.to_array_except(&["password_hash", "remember_token", "internal_notes"])
    ))
}

pub async fn directory_show(user: User) -> suprnova::Response {
    // annuaire public - seulement les colonnes que nous voulons publier :
    json_response!(
        user.to_array_only(&["id", "name", "avatar_url"])
    ))
}
```

Les deux produisent un `serde_json::Value` - ils ne mutent pas `self`
et ne changent pas les sérialisations futures de la même ligne. Ils
exécutent d'abord le pipeline complet `hidden` / `visible` / `appends`,
puis appliquent leur propre rognage par-dessus. `to_array_only`
retourne un objet JSON *neuf* qui ne contient que les clés nommées ;
`to_array_except` retourne l'objet complet moins les clés nommées.

### Pourquoi Suprnova diverge

Les `$user->makeHidden(['x'])` et `$user->makeVisible(['x'])` de
Laravel **mutent** l'instance du modèle - chaque appel ultérieur à
`toArray()`, y compris ceux qui se produisent quand le modèle est
imbriqué dans la sérialisation d'un parent, voit l'état modifié. Les
helpers de Suprnova sont **terminaux**. Ils produisent une `Value` et
s'arrêtent. Si vous avez besoin que le changement se propage,
déclarez-le sur `#[model(hidden = [...])]` /
`#[model(visible = [...])]` pour que le *type* exprime la politique,
pas une mutation cachée sur l'instance.

La raison à la façon de Rust : une struct Eloquent dans Suprnova est
une simple struct Rust sans sac d'attributs à l'exécution. Il n'y a
pas d'endroit où un flag de visibilité côté instance pourrait vivre
sans ajouter un état caché ambiant - exactement le genre de piège que
le framework évite intentionnellement.

## Masquage conditionnel selon le visiteur

Le motif idiomatique quand la visibilité dépend du visiteur est un
`match` au site d'appel, qui bifurque vers le bon filtre par appel :

```rust
use suprnova::{Auth, json_response, Model, Request, Response};

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    let viewer = Auth::user_as::<User>().await?;
    let viewing_self = viewer.as_ref().map(|v| v.id) == Some(user.id);

    let body = if viewing_self {
        user.to_array()
    } else {
        user.to_array_except(&["email", "phone", "stripe_customer_id"])
    };

    json_response!(body)
}
```

Pour une forme par visiteur plus élaborée - des attributs différents
pour les admins, les utilisateurs en essai, les utilisateurs payants -
le bon outil est la **couche de ressources JSON:API** avec des champs
`Maybe<T>` / `MissingValue<T>`. Voir
[Ressources JSON:API](eloquent-resources.md#conditional-attributes--maybet--missingvaluet)
pour la forme déclarative.

## Le piège du contournement de serde

C'est la chose la plus importante à savoir sur la sérialisation
Eloquent dans Suprnova.

**Les filtres `hidden` / `visible` / `appends` ne s'exécutent qu'à
travers `to_array()` et `to_json()`.** Ils ne sont *pas* imposés par
l'impl `Serialize` dérivé. Retourner la struct par tout autre chemin
serde contourne entièrement les filtres.

Cela signifie que **tout ce qui suit fait fuiter `password`** :

```rust
// serde direct - contourne to_array, hidden n'a aucun effet :
let raw = serde_json::to_value(&user).unwrap();

// json_response! avec un champ de struct - même chose :
json_response!({ "user": user }))

// Imbriqué dans un autre conteneur sérialisable - même chose :
#[derive(Serialize)]
struct EnvelopeWithUser { ok: bool, user: User }
let env = EnvelopeWithUser { ok: true, user };
json_response!(env))

// Retourner un Vec<User> via serde - même chose :
json_response!(users))   // où users: Vec<User>
```

Seuls ceux-ci passent par le pipeline de filtres :

```rust
json_response!(user.to_array()))
json_response!(users_collection.to_array()))  // Collection<User>
json_response!(user.to_array_except(&["secret"])))
json_response!(user.to_array_only(&["id", "name"])))
```

### Pourquoi cela se produit

Le `Serialize for Vec<T>` générique de serde (et tout autre conteneur)
appelle directement `T::serialize`. Le pipeline de filtres de Suprnova
vit dans la méthode de trait `Model::to_array`, pas dans `Serialize`.
La méthode de trait n'est invoquée que si vous l'appelez.

Le framework se protège contre le piège *interne* (les champs de
travail `__eager` / `__pivot` sont marqués `#[serde(skip)]` pour
qu'ils ne fuient par aucun des deux chemins), mais la macro n'émet
**délibérément pas** `#[serde(skip_serializing)]` sur les champs
`hidden` - le faire casserait des usages légitimes de serde avec le
modèle SeaORM interne, quand un appelant veut la ligne complète (RPC
interne, couches de persistance, diagnostics, tests, par exemple).

### La règle

Pour toute valeur qui traverse la frontière de confiance vers un
client, passez par `to_array()` ou l'une de ses cousines filtrées. Le
contrat en quatre lignes qui vous achète cette sécurité :

| Besoin | Utilisez | Résultat |
|---|---|---|
| Sérialiser un modèle | `user.to_array()` | Objet JSON filtré |
| Sérialiser une collection | `collection.to_array()` | Tableau JSON filtré |
| Soustraire quelques champs | `user.to_array_except(&["x"])` | Filtré + soustrait |
| Garder seulement quelques champs | `user.to_array_only(&["x"])` | Seulement les clés listées |

Un linter ou une revue au moment de la PR pour
`json_response!\({.*: [a-z_]+ ?})` et `serde_json::to_value\(&\w+\)`
sur des valeurs de modèle est un moyen peu coûteux de faire respecter
la règle. Les propres tests du framework pour la sérialisation de
`Model` couvrent les deux chemins.

## Sérialiser des collections

Une `Collection<M>` - retournée par `Builder::get()`, `Model::all()`,
et les accesseurs de relation - a ses propres `to_array()` et
`to_json()` qui parcourent le `Vec<M>` sous-jacent et appellent
`to_array()` **ligne par ligne**. Le résultat est un tableau JSON
d'objets filtrés :

```rust
use suprnova::{json_response, Model};

pub async fn list() -> suprnova::Response {
    let users = User::all().await?;
    json_response!(users.to_array())
}
```

C'est le seul endroit où obtenir le filtre par ligne sur un résultat
multi-lignes. `serde_json::to_value(&users)` émettrait un Vec via
l'impl générique de serde et contournerait les filtres sur toutes les
lignes à la fois - le helper au niveau collection existe exactement
pour combler cet écart.

```rust
// La redéfinition pour Collection<M> :
pub fn to_array(&self) -> Value {
    Value::Array(self.0.iter().map(|m| m.to_array()).collect())
}
```

Pour un paginateur, les données enveloppées vivent dans
`LengthAwarePaginator::data` / `CursorPaginator::data` et forment un
`Vec<M>` - appelez `.to_array()` sur chaque élément avant d'assembler
la réponse du paginateur, ou utilisez la
[forme paginée JSON:API](eloquent-resources.md#pagination) qui gère le
filtrage par ligne dans le cadre du pipeline de ressources.

## Relations chargées hâtivement et sérialisation

C'est la seconde divergence à intérioriser.

Quand vous appelez `.with(["posts"])` sur un builder, le framework
charge les posts et les stocke dans un `EagerLoadCache` par ligne (le
champ `__eager` auto-injecté). L'accesseur pour les lire -
`user.posts_loaded()` - puise dans ce cache.

**Le cache est `#[serde(skip)]` et `to_array()` le retire sans
condition.** Les relations chargées hâtivement ne se replient pas
automatiquement dans la sortie JSON. Un `to_array()` sur un
utilisateur dont les posts ont été chargés hâtivement a l'air
identique à un `to_array()` sur un utilisateur qui ne les a pas.

### Pourquoi Suprnova diverge

Le `toArray()` de Laravel parcourt `$model->getRelations()` et replie
chaque relation chargée dans la sortie. Le sac de modèle en forme de
tableau de PHP rend cela naturel - une relation n'est qu'une entrée
clé-valeur de plus sur le modèle.

Les structs Eloquent typées de Rust n'ont pas ce sac. Une struct
`User` a des colonnes typées, pas une map hétérogène de « peu importe
quelles relations ont été chargées ». Replier `posts` dedans exigerait
soit une injection de champ à l'exécution sur une struct typée (un
mécanisme de contournement de serde), soit un chemin de sérialisation
parallèle qui consulte le cache après avoir exécuté le sérialiseur de
colonnes. Les deux options coupleraient la forme JSON de chaque modèle
aux relations qu'un appelant particulier a chargées hâtivement - un
contrat porteur en PHP parce que les clients apprennent à en dépendre,
et un contrat que Suprnova refuse explicitement de livrer parce qu'il
ferait dépendre la forme JSON de la construction de requête côté
appelant.

### Les deux façons de livrer des données de relation

**1. Accesseur explicite + appends.** Définissez une méthode qui puise
dans `<rel>_loaded()`, enregistrez-la dans `appends`. La relation
apparaît sous la clé que vous nommez. Cela fonctionne quand la
relation est *toujours* chargée hâtivement sur le chemin de lecture :

```rust
use suprnova::{accessor, model};
use serde_json::Value;

#[model(
    table = "users",
    appends = ["posts"],
)]
pub struct User { /* ... */ }

impl User {
    #[accessor]
    pub fn posts(&self) -> Value {
        // posts_loaded() PANIQUE si .with(["posts"]) n'a pas été appelé sur
        // le chemin de lecture. L'accesseur DOIT s'exécuter après le
        // chargement hâtif.
        let posts = self.posts_loaded();
        serde_json::to_value(posts).unwrap_or(Value::Null)
    }
}

// Le chemin de lecture DOIT charger hâtivement :
let users = User::query()
    .with(["posts"])
    .get()
    .await?;
let body = users.to_array();   // la clé "posts" de chaque utilisateur est renseignée
```

Le contrat est explicite : oubliez le `.with(["posts"])`, et
l'accesseur panique dès l'appel `posts_loaded()` de la première ligne
(le cache hâtif panique à la lecture quand la relation n'a pas été
chargée, par conception - un tableau vide silencieux cacherait le
bug). Pour un chargement hâtif optionnel, utilisez la forme `HasOne`
qui retourne `Option<&T>` et vous donne un `match` :

```rust
impl User {
    #[accessor]
    pub fn profile(&self) -> Value {
        match self.profile_loaded() {
            Some(profile) => serde_json::to_value(profile).unwrap_or(Value::Null),
            None => Value::Null,
        }
    }
}
```

**2. La couche de ressources JSON:API.** Quand la forme de la relation
et la politique d'inclusion appartiennent au format réseau plutôt
qu'au modèle, utilisez une struct `#[derive(Data)] #[json_resource]`
avec `#[data(allow_include)]` sur le champ de relation. Les clients
optent via `?include=posts.comments`, le framework parcourt l'arbre
d'include, et remplit `included` avec des objets ressource
dédupliqués. C'est la bonne réponse quand :

- La forme de la relation relève du format réseau (sparse fieldsets,
  inclusion conditionnelle, métadonnées de lien croisé).
- Des endpoints différents veulent des inclusions par défaut
  différentes.
- Le même modèle apparaît sous des enveloppes différentes (un endpoint
  livre `posts`, un autre livre `subscriptions`).

Voir [Ressources JSON:API](eloquent-resources.md#compound-documents--include-chains)
pour le motif complet.

## Qu'en est-il de JSON:API ?

Le pipeline `to_array()` et la façade `Resource` / `JsonApi` sont deux
couches, et elles servent des rôles différents :

| Préoccupation | `Model::to_array` | `Resource::single` / `JsonApi::single` |
|---|---|---|
| **Forme** | Objet plat - les noms de colonne correspondent directement aux clés | Enveloppe JSON:API (`data`, `included`, `meta`, `links`, `jsonapi`) |
| **Contrôle par attribut** | `hidden` / `visible` / `appends` sur `#[model]` | `#[data(input_only)]`, `Maybe<T>`, sparse fieldsets via `?fields[type]=` |
| **Relations** | Manuel (accesseur + appends, voir plus haut) | De premier ordre via `#[data(allow_include)]` + `?include=` |
| **Pagination** | Envelopper un `Vec<Value>` à la main | `Resource::paginated(p)` gère links + meta |
| **Erreurs** | Rendu via `FrameworkError` | `into_json_api_response()` produit une enveloppe `errors` JSON:API |
| **Quand s'en servir** | Endpoints simples, outils internes, formes ad hoc | API publiques, consommateurs tiers, clients avertis JSON:API |

`to_array()` est la couche basse - c'est ce qui est appelé pour la
plupart des handlers internes, des pages admin, des props Inertia (via
serde), et des tests. La couche JSON:API se compose par-dessus : elle
ne remplace pas `to_array`, elle ajoute une enveloppe autour d'une
logique d'attribut / de relation par ressource trop riche pour vivre
sur le modèle lui-même.

Pour des props Inertia typées, vous voulez presque toujours la couche
de ressources ou un DTO `#[derive(Serialize)]` dédié avec des champs
explicites, plutôt que de faire passer le modèle par serde directement.
Les retours Inertia reçoivent le même traitement de contournement de
serde que tout le reste - le chemin sûr est « construisez un DTO,
remplissez-le depuis `to_array()`, retournez le DTO ».

## Où réside chaque élément

| Préoccupation | Fichier |
|---|---|
| Défauts de trait `Model::to_array` / `to_json` | `framework/src/eloquent/model.rs` |
| `Model::to_array_except` / `to_array_only` | `framework/src/eloquent/model.rs` |
| Défaut de trait `Model::__append_accessor` | `framework/src/eloquent/model.rs` |
| Redéfinition `to_array` émise par la macro (pipeline de filtres) | `suprnova-macros/src/model/serialization.rs` |
| Dispatcher `__append_accessor` émis par la macro | `suprnova-macros/src/model/serialization.rs` |
| `Collection<M>::to_array` / `to_json` | `framework/src/eloquent/collection.rs` |
| `EagerLoadCache` (le champ `__eager`) | `framework/src/eloquent/relations/eager_cache.rs` |
| Analyse par macro de `hidden` / `visible` / `appends` | `suprnova-macros/src/model/parse.rs` |
| Macro au niveau fonction `#[accessor]` | `suprnova-macros/src/lib.rs` |

## Suivant

- [Eloquent API](eloquent.md) - la surface complète du modèle, la
  référence des attributs, et où `#[accessor]` / `#[mutator]` sont
  définis
- [Ressources JSON:API](eloquent-resources.md) - la couche de
  ressources déclarative pour des formes par visiteur plus riches, des
  sparse fieldsets, et des documents composés `?include=`
- [Validation](validation.md) - comment l'entrée de requête devient
  une struct typée avant que la couche modèle ne la voie
- [Réponses](responses.md) - les builders `HttpResponse`, les
  en-têtes, et les cookies ; la surface que produit finalement
  `json_response!`
- [Modèle d'erreur](error-model.md) - comment une erreur devient un
  corps JSON avec la même corrélation `request_id` que le chemin de
  succès

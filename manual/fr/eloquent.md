# Eloquent API

La couche Eloquent de Suprnova donne aux développeurs Laravel l'API
qu'ils connaissent déjà, implémentée comme une fine couche par-dessus
SeaORM. Copiez du code depuis la documentation Laravel, remplacez la
syntaxe PHP par du Rust, ajoutez `.await?`, et ça fonctionne.

Toute la couche se résume à un attribut de struct
(`#[suprnova::model]`), un trait (`Model`), et un query builder
chaînable (`Builder<M>`) - c'est tout. En coulisse, la macro génère un
`Entity`, un `Model`, un `ActiveModel` SeaORM, et un enum `Column`,
plus chaque impl de trait Eloquent. Les types SeaORM restent
accessibles pour le rare cas où la surface Eloquent ne couvre pas
votre besoin (voir les
[échappatoires SeaORM](#redescendre-vers-seaorm)).

## Table des matières

- [Démarrage rapide](#démarrage-rapide)
- [L'attribut `#[suprnova::model]`](#l-attribut-suprnova-model)
- [Disposition du module de modèle](#disposition-du-module-de-modèle)
- [Trouver des lignes](#trouver-des-lignes)
- [Créer et mettre à jour](#créer-et-mettre-à-jour)
- [Supprimer et suppressions logicielles](#supprimer-et-suppressions-logicielles)
- [Query builder - double API](#query-builder-double-api)
- [Verrouillage des lignes](#verrouillage-des-lignes)
- [Transactions](#transactions)
- [Scopes](#scopes)
- [Relations](#relations)
- [Chargement hâtif](#chargement-hâtif)
- [Pagination](#pagination)
- [Itération par chunk et en mode lazy](#itération-par-chunk-et-en-mode-lazy)
- [Collections](#collections)
- [Affectation en masse](#affectation-en-masse)
- [Casts](#casts)
- [Accesseurs et mutateurs](#accesseurs-et-mutateurs)
- [Timestamps](#timestamps)
- [Observateurs et événements de cycle de vie](#observateurs-et-événements-de-cycle-de-vie)
- [Prunable](#prunable)
- [Routage multi-connexion](#routage-multi-connexion)
- [Réplication](#réplication)
- [Débogage - dump et dd](#débogage-dump-et-dd)
- [Tester les modèles](#tester-les-modèles)
- [Redescendre vers SeaORM](#redescendre-vers-seaorm)
- [Migrer depuis `database::Model`](#migrer-depuis-database-model)
- [Façade DB - requêtes sans modèle](#façade-db-requêtes-sans-modèle)
- [Parité Laravel 13 - existence de relation + raccourcis peu coûteux](#existence-de-relation-raccourcis-peu-coûteux)

## Démarrage rapide

Un seul attribut sur une struct la transforme en modèle Eloquent
complet :

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

Une fois déclaré, vous pouvez écrire :

- `User::query()` - démarre un query builder fluide.
- `User::find(id).await?` - récupère par clé primaire.
- `User::find_or_fail(id).await?` - pareil, mais échoue avec
  `ModelNotFound` si aucune ligne ne correspond.
- `User::all().await?` - chaque ligne.
- `User::create(attrs!{ name: "Alice", email: "alice@example.com" }).await?` -
  insère avec le filtrage d'affectation en masse.
- `User::filter("email", "alice@example.com").first().await?` -
  une ligne qui correspond.
- `user.update(attrs!{ name: "Alice B" }).await?` - mise à jour
  partielle.
- `user.save().await?` - persiste les changements en mémoire.
- `user.delete().await?` - retire la ligne.
- `user.refresh().await?` / `user.fresh().await?` / `user.replicate().await?` -
  le reste du cycle de vie façon Laravel.

La struct exposée au reste du code (ici `User`) EST le type que vos
handlers et contrôleurs transportent. La macro émet un module interne
par modèle (`user::`) avec les types SeaORM `Entity`, `Column`,
`ActiveModel` et `Model`, pour les cas où vous voulez redescendre
directement vers SeaORM. La struct est aussi enregistrée dans un
`ModelEntry` porté par l'inventaire, si bien que le code d'admin et
l'outillage peuvent énumérer chaque modèle au démarrage.

## L'attribut `#[suprnova::model]`

Le point d'entrée unique pour déclarer un modèle. Chaque attribut est
optionnel ; les défauts sont réglés pour qu'une struct avec `id` +
`created_at` + `updated_at` fonctionne comme un modèle Suprnova sans
aucune configuration.

### Référence des attributs de macro

| Attribut | Type | Défaut | Remarques |
|-----------|------|---------|-------|
| `table` | chaîne | pluriel snake_case du nom de la struct | Redéfinit le nom de la table |
| `primary_key` | chaîne | `"id"` | Redéfinit le nom de la colonne PK |
| `key_type` | type | `i64` | Type de PK - `String` pour un UUID, `i32` pour les schémas historiques |
| `auto_increment` | bool | `true` | À désactiver pour les PK UUID |
| `connection` | chaîne | `"default"` | Les applications multi-connexion y nomment une connexion non par défaut |
| `fillable` | liste de chaînes | (défaut = `guarded = ["id"]`) | Liste blanche d'affectation en masse |
| `guarded` | liste de chaînes | `["id"]` quand ni l'un ni l'autre n'est défini | Liste noire d'affectation en masse (mutuellement exclusif avec `fillable`) |
| `casts` | map de `field = CastType` | `{}` | Casts par colonne |
| `hidden` | liste de chaînes | `[]` | Exclu de `to_json` / `to_array` |
| `visible` | liste de chaînes | (toutes) | Variante inclusive de `hidden` (mutuellement exclusif) |
| `appends` | liste de chaînes | `[]` | Accesseurs à inclure dans la sérialisation |
| `soft_deletes` | flag | `false` | Active la colonne `deleted_at` et la sémantique de suppression logicielle |
| `soft_deletes_column` | chaîne | `"deleted_at"` | Redéfinit le nom de la colonne de suppression logicielle |
| `timestamps` | flag / bool | `true` quand `created_at` et `updated_at` existent tous les deux | Désactive les timestamps auto-gérés |
| `created_at` | chaîne | `"created_at"` | Redéfinit le nom de la colonne |
| `updated_at` | chaîne | `"updated_at"` | Redéfinit le nom de la colonne |
| `touches` | liste de noms de relation | `[]` | Relations `BelongsTo` dont la ligne parente voit son `updated_at` avancé après que ce modèle a été créé, sauvegardé, mis à jour ou supprimé |
| `mutators` | liste de chaînes | `[]` | Noms de champs dont le chemin de remplissage JSON passe par une méthode mutateur `set_<field>(value)` |

### Exemple complet

```rust
use chrono::{DateTime, Utc};
use serde_json::Value as Json;
use suprnova::{model, AsBool, AsEncrypted, AsJson};

#[model(
    table = "users",
    fillable = ["name", "email", "preferences"],
    casts = {
        active = AsBool,
        preferences = AsJson<Json>,
        api_token = AsEncrypted,
    },
    hidden = ["password", "remember_token", "api_token"],
    appends = ["full_name"],
    soft_deletes,
    timestamps,
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub api_token: Option<String>,
    pub active: bool,
    pub preferences: Json,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}
```

### Macros au niveau fonction

Des macros au niveau fonction fonctionnent aux côtés de l'attribut de
struct :

- `#[accessor]` sur un `fn name(&self) -> T` en fait un accesseur
  Eloquent. Le `to_array()` du modèle l'appelle quand `name` est
  listé dans `appends = [...]` (et `to_json()` le récupère via la
  délégation `to_array` → string).
- `#[mutator]` sur un `fn set_name(&mut self, value: serde_json::Value)`
  en fait un mutateur Eloquent. Le chemin de remplissage JSON du
  modèle passe par lui quand `name` est listé dans
  `mutators = [...]`.
- `#[suprnova::scopes(Model)]` sur un bloc `impl Model { ... }` :
  chaque méthode dont la signature est
  `fn name(query: Builder<Self>[, args…]) -> Builder<Self>` devient
  à la fois un `.scope_name(args)` chaînable sur `Builder<Self>` et
  un raccourci `Model::scope_name(args)`. Il n'existe pas de forme
  `#[scope]` au niveau fonction - les scopes se déclarent par bloc
  impl.
- Les scopes globaux sont un enregistrement à l'exécution via le
  trait `GlobalScope`, appliqué par `Model::global_scope::<GS>()`.
  Il n'existe pas de macro `#[global_scope]` au niveau fonction -
  voir [Macros](macros.md#suprnova-scopes-model) pour le motif
  complet.
- `#[prunable]` sur `impl Prunable for T { ... }` enregistre
  l'élagueur via l'inventaire pour que `model:prune` le trouve.

## Disposition du module de modèle

`#[suprnova::model]` garde votre struct exposée (p. ex. `Post`) à la
portée parente et émet un `pub mod` frère nommé d'après la struct en
snake_case (`post`). Ce module interne est là où vivent les types
SeaORM.

Pour un modèle déclaré dans `app/src/models/posts.rs` :

```rust
use chrono::{DateTime, Utc};
use suprnova::model;

#[model(table = "posts", fillable = ["title", "body"], timestamps)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Convention : ré-exporter les types SeaORM que la macro émet à
// l'intérieur du module interne pour que les sites d'appel puissent
// utiliser les noms sans préfixe. Les propres modèles dogfood de
// Suprnova portent tous cette ligne (voir `app/src/models/users.rs`,
// `app/src/models/posts.rs`, etc.).
pub use post::{ActiveModel, Column, Entity};
```

Vous avez maintenant ces éléments accessibles depuis
`crate::models::posts` :

| Chemin | Ce que c'est |
|------|-----------|
| `crate::models::posts::Post` | Votre struct exposée - le modèle Eloquent |
| `crate::models::posts::post::Entity` | Impl SeaORM `EntityTrait` pour la table `posts` |
| `crate::models::posts::post::Column` | Enum SeaORM `Column` (une variante par colonne) |
| `crate::models::posts::post::ActiveModel` | `ActiveModel` SeaORM pour insert/update |
| `crate::models::posts::post::Model` | Ligne à la forme SeaORM (colonnes typées stockage) |
| `crate::models::posts::{Entity, Column, ActiveModel}` | La convention `pub use` ci-dessus ; pas auto-émis |

Deux choses à savoir sur le `Model` du module interne :

1. C'est la ligne à la **forme SeaORM**, pas votre struct `Post`. Les
   colonnes castées portent ici leur type `Storage` (p. ex. `bool`
   devient l'entier sous-jacent), et les emplacements runtime
   `__eager` / `__pivot` de votre struct sont absents.
2. `From<post::Model> for Post` et `From<Post> for post::Model` font
   le pont entre les deux formes. Voir
   [Redescendre vers SeaORM](#redescendre-vers-seaorm) pour le motif
   d'aller-retour.

`Model` ne fait délibérément **pas** partie du ré-export conventionnel
au niveau parent - le `Post` exposé occupe déjà le nom `Post` à la
portée parente, et `post::Model` est un type séparé que les
appelants atteignent via `post::Model` (ou une conversion `From`)
quand ils ont besoin de la forme interne.

### Quand aller chercher dans le module interne

La surface Eloquent (trait `Model` + `Builder<M>`) couvre l'immense
majorité des requêtes. Allez chercher dans `post::*` quand vous avez
besoin de fonctionnalités propres à SeaORM :

- **Construction de requête brute** avec la chaîne
  `EntityTrait::find()` de SeaORM quand Eloquent n'expose pas le
  helper voulu.
- **Logique de jointure personnalisée** - construire des jointures
  `JoinType::*` explicitement via `QuerySelect::join()` pour une
  relation que le `with(...)` d'Eloquent ne modélise pas.
- **Sous-requêtes natives SeaORM** via `Entity::find().select_only()`.
- **Mutation `ActiveModel` directe** pour le rare cas où vous voulez
  contourner le cycle de vie Eloquent (aucun observateur, aucun
  auto-timestamp).

```rust
// Cas courant - Column ré-exporté au niveau du module parent via la
// convention `pub use post::{...}` ci-dessus.
use crate::models::posts::Column;

let drafts = Post::query()
    .db_where(Column::Status, "draft")
    .get()
    .await?;

// Cas avancé - aller chercher dans le module interne pour l'Entity
// SeaORM directement. C'est ce que le `pub use` parent ne fait pas
// remonter.
use crate::models::posts::post;
use suprnova::sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

let db = suprnova::DB::connection()?;
let rows: Vec<post::Model> = post::Entity::find()
    .filter(post::Column::Status.eq("published"))
    .all(db.inner())
    .await?;

// Repasser à la forme Eloquent quand l'appelant la veut.
let posts: Vec<Post> = rows.into_iter().map(Post::from).collect();
```

Si vous vous retrouvez à aller chercher dans le module interne de
façon routinière pour la même opération, c'est le signe qu'il manque
un helper à Eloquent - ouvrez une issue, ou ajoutez le helper à la
surface `Model` / `Builder`.

## Trouver des lignes

```php
// Laravel
$user = User::find(1);
$user = User::findOrFail(1);          // lève une exception si absent
$users = User::findMany([1, 2, 3]);
```

```rust
// Suprnova
let user: Option<User> = User::find(1).await?;
let user: User = User::find_or_fail(1).await?;
let users: Vec<User> = User::find_many([1, 2, 3]).await?;
```

`find_or_fail` retourne `FrameworkError::ModelNotFound` (HTTP 404 une
fois remonté jusqu'à un contrôleur).

### `first_or_create` / `update_or_create` / `first_or_new` / `first_or`

```php
// Laravel
$user = User::firstOrCreate(
    ['email' => 'alice@example.com'],
    ['name' => 'Alice'],
);
$user = User::updateOrCreate(
    ['email' => 'alice@example.com'],
    ['name' => 'Alice Updated'],
);
$user = User::firstOrNew(['email' => 'alice@example.com']);  // non sauvegardé
```

```rust
// Suprnova
let user = User::first_or_create(
    attrs! { email: "alice@example.com" },          // clés de recherche
    attrs! { name: "Alice" },                       // champs en plus à la création
).await?;

let user = User::update_or_create(
    attrs! { email: "alice@example.com" },
    attrs! { name: "Alice Updated" },
).await?;

let user = User::first_or_new(
    attrs! { email: "alice@example.com" },
).await?;   // retourne un User non sauvegardé ; l'appelant sauvegarde explicitement
```

Les clés de recherche vont dans la première map ; les champs
supplémentaires appliqués sur le chemin de création vont dans la
seconde. Retourner un modèle non sauvegardé via `first_or_new` permet
à l'appelant de le muter davantage avant `save().await?`.

## Créer et mettre à jour

### Créer

```php
// Laravel
$user = User::create([
    'name' => 'Alice',
    'email' => 'alice@example.com',
]);
```

```rust
// Suprnova
let user = User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
}).await?;
```

`attrs!` est une macro qui produit une valeur `Attrs` (une map JSON
typée). Du JSON pur fonctionne aussi -
`User::create(serde_json::json!({"name": "Alice", "email": "..."}))`.
Le filtre `Fillable` s'exécute à l'intérieur de `create` ; les champs
non fillable sont silencieusement abandonnés, comme le comportement
de Laravel.

### Sauvegarder / mettre à jour

```php
// Laravel
$user->name = 'Alice B';
$user->save();

$user->update(['name' => 'Alice B']);
```

```rust
// Suprnova
user.name = "Alice B".into();
user.save().await?;

user.update(attrs! { name: "Alice B" }).await?;
```

`save()` parcourt chaque champ non-PK, les positionne sur
l'ActiveModel via `Set(...)`, appelle le `update()` de SeaORM, et
retourne la ligne canonique. `update(attrs)` est le même flux, mais
applique d'abord une map d'attributs partielle (en exécutant le
filtre Fillable et les mutateurs déclarés).

### Incrémenter / décrémenter

```php
// Laravel
$user->increment('login_count');
$user->increment('login_count', 5);
$user->decrement('credits', 10);
User::where('plan', 'free')->increment('quota_reset_count');
```

```rust
// Suprnova
user.increment("login_count", 1).await?;
user.increment("login_count", 5).await?;
user.decrement("credits", 10).await?;
User::filter("plan", "free").increment("quota_reset_count", 1).await?;
```

`increment` / `decrement` émettent du SQL `UPDATE table SET col = col +
N WHERE ...` - atomique face aux mises à jour concurrentes, sans
race lecture-modification-écriture. Disponible à la fois sur une
instance de modèle récupérée (utilise la PK de la ligne dans la
clause WHERE) et comme terminal de Builder (utilise les clauses
WHERE de la chaîne).

### Fresh / refresh / replicate

```php
// Laravel
$user->refresh();                          // recharge depuis la BD
$user->refreshForUpdate();                 // recharge sous un verrou de ligne
$copy = $user->fresh();                    // récupère + retourne une copie
$replica = $user->replicate();             // clone non sauvegardé, PK neuve
$replica = $user->replicate(['email']);    // ignore un champ
```

```rust
// Suprnova
user.refresh().await?;
user.refresh_for_update().await?;
let copy: User = user.fresh().await?;
let replica: User = user.replicate().await?;
let replica: User = user.replicate_except(["email"]).await?;
```

`refresh` mute sur place ; `fresh` retourne une copie récupérée
séparément. `refresh_for_update` est `refresh` sous un verrou de ligne
`SELECT ... FOR UPDATE` - employez-le à l'intérieur d'une transaction
quand il vous faut les valeurs courantes de la ligne et le verrou
exclusif en une seule instruction. Contrairement à `refresh`,
`refresh_for_update` contourne tous les scopes globaux enregistrés ET
le filtre `#[model(soft_deletes)]` : il recharge aussi une ligne à la
corbeille, avec `deleted_at` positionné au retour. Le rechargement est
une recherche par clé primaire sous verrou - le cantonner comme on
cantonne une lecture ordinaire donnerait aux outils d'administration
et aux appelants inter-tenants un faux « non trouvé » pour une ligne
dont ils détiennent déjà une référence. `replicate` construit un clone
en mémoire avec la PK réinitialisée (`Default::default()` pour le type
de la clé). L'appelant sauvegarde explicitement.

`refresh` et `refresh_for_update` retournent tous deux une erreur quand
la ligne n'existe plus, plutôt que de laisser le modèle avec des
valeurs périmées. SQLite n'a pas de verrouillage au niveau de la ligne :
`refresh_for_update` y recharge donc sans verrou - voir
[Verrouillage des lignes](#verrouillage-des-lignes).

### Événement `Replicating`

`replicate` et `replicate_except` déclenchent l'événement
`Replicating { source, replica }` propre au modèle, après avoir
construit le clone en mémoire et AVANT de le retourner. Le champ
`replica` est un `Arc<tokio::sync::Mutex<Self>>`, si bien que les
écouteurs peuvent muter la réplique avant que l'appelant ne la voie -
utile pour préfixer des titres avec `(copy)`, effacer des flags,
réinitialiser des colonnes dérivées, etc.

```rust
use suprnova::events::{EventFacade, Listener};
use async_trait::async_trait;

pub struct PrefixTitle;

#[async_trait]
impl Listener<post::events::Replicating> for PrefixTitle {
    async fn handle(&self, e: &post::events::Replicating)
        -> Result<(), FrameworkError>
    {
        let mut replica = e.replica.lock().await;
        replica.title = format!("(copy) {}", replica.title);
        Ok(())
    }
}

// Câblez-le une fois au démarrage :
EventFacade::listen::<post::events::Replicating, _>(
    std::sync::Arc::new(PrefixTitle)
).await;
```

### Réplication entre types

```rust
let replica: UserDraft = user.replicate_into().await?;  // clone entre types
```

Une divergence Suprnova - Laravel ne peut pas faire ça parce que PHP
n'a pas de types. Utile pour promouvoir un modèle brouillon en un
modèle final, ou l'inverse.

`replicate_into<T>` NE déclenche PAS `Replicating` (l'événement porte
un `Arc<Mutex<Self>>`, si bien qu'un écouteur sur le type source ne
pourrait de toute façon pas muter la réplique de l'autre type). Les
appelants qui veulent une configuration par `T` devraient l'exécuter
sur le `T` retourné avant d'appeler `T::save` - la chaîne normale
`Saving` / `Created` se déclenche toujours à l'intérieur de `save`.

## Supprimer et suppressions logicielles

### Le flag `soft_deletes`

Ajoutez `soft_deletes` à l'attribut de macro, ainsi qu'une colonne
`deleted_at: Option<DateTime<Utc>>` à la struct :

```rust
#[model(table = "users", soft_deletes, timestamps)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub deleted_at: Option<DateTime<Utc>>,
    // ...
}
```

### Cycle de vie

```rust
user.delete().await?;             // UPDATE : positionne deleted_at = NOW()
user.trashed();                   // -> true
let trashed = User::with_trashed().find(user.id).await?.unwrap();
trashed.restore().await?;         // UPDATE : positionne deleted_at = NULL

let only_dead = User::only_trashed().get().await?;
let all_including_dead = User::with_trashed().get().await?;

user.force_delete().await?;       // vrai DELETE
```

### Scope par défaut

Quand `soft_deletes` est activé, la macro redéfinit `Model::query()`
pour que les lectures par défaut filtrent automatiquement les lignes
à la corbeille. `with_trashed()` et `only_trashed()` permettent de
les inclure à nouveau. Concrètement : `User::find(id)` ignore les
lignes à la corbeille ; `User::with_trashed().find(id)` les trouve.

## Query builder - double API

`Builder<M>` est le type de requête chaînable retourné par
`User::query()`, `User::filter(...)`, `User::db_where(...)`, et
toute autre méthode statique qui ne termine pas la chaîne.

### Note de nommage : double API

`where` est un mot-clé Rust, donc la méthode where d'égalité simple
ne peut pas partager le nom de Laravel. Plutôt que de trancher,
chaque méthode de forme where est livrée sous **deux** noms : un nom
idiomatique Rust (`filter`, `filter_in`, `filter_null`, …) et un nom
façon Laravel (`db_where`, `where_in`, `where_null`, …). Ce sont des
alias vers une seule implémentation canonique - choisissez celui qui
correspond à votre mémoire musculaire.

```rust
// Dev Rust :
User::query().filter("active", true).filter_in("role", ["admin"]).get().await?;

// Dev Laravel :
User::db_where("active", true).where_in("role", ["admin"]).get().await?;

// Même requête. Même résultat. Mémoire musculaire différente.
```

### Raccourcis where

```php
// Laravel
$users = User::where('email', $email)->get();
$users = User::where('age', '>=', 18)->get();
$users = User::where('email', 'like', '%@example.com')->get();
```

```rust
// Suprnova - choisissez l'une ou l'autre famille ; les deux compilent, les deux sont documentées.

// Forme Rust (famille filter) :
let users = User::query().filter("email", &email).get().await?;
let users = User::query().filter_op("age", ">=", 18).get().await?;
let users = User::query().filter_like("email", "%@example.com").get().await?;

// Forme Laravel (famille db_where / where_*) :
let users = User::db_where("email", &email).get().await?;
let users = User::query().db_where_op("age", ">=", 18).get().await?;
let users = User::query().where_like("email", "%@example.com").get().await?;
```

### Variantes de where

Chaque ligne a deux formes Suprnova équivalentes - forme Rust
(`filter*`) et forme Laravel (`db_where` / `where_*`). Les deux
appellent la même implémentation canonique ; les deux sont taguées
avec `#[doc(alias = "...")]` pour que la recherche rustdoc trouve
l'une ou l'autre.

| Laravel | Suprnova (forme Rust) | Suprnova (forme Laravel) | Remarques |
|---------|----------------------|--------------------------|-------|
| `->where(col, val)` | `.filter(col, val)` | `.db_where(col, val)` | Égalité |
| `->where(col, op, val)` | `.filter_op(col, op, val)` | `.db_where_op(col, op, val)` | Opérateur arbitraire |
| `->orWhere(...)` | `.or_filter(...)` | `.or_where(...)` | |
| `->orWhereKey(id)` | `.or_filter_key(id)` | `.or_where_key(id)` | Filtre de PK comme disjonction |
| `->orWhereKeyNot(id)` | `.or_filter_key_not(id)` | `.or_where_key_not(id)` | Filtre de PK nié comme disjonction |
| `->whereNot(col, val)` | `.filter_not(col, val)` | `.where_not(col, val)` | |
| `->whereIn(col, vals)` | `.filter_in(col, vals)` | `.where_in(col, vals)` | |
| `->whereNotIn(col, vals)` | `.filter_not_in(col, vals)` | `.where_not_in(col, vals)` | |
| `->whereBetween(col, [a, b])` | `.filter_between(col, a..=b)` | `.where_between(col, a..=b)` | Plage Rust |
| `->whereNotBetween(col, [a, b])` | `.filter_not_between(col, a..=b)` | `.where_not_between(col, a..=b)` | |
| `->whereNull(col)` | `.filter_null(col)` | `.where_null(col)` | |
| `->whereNotNull(col)` | `.filter_not_null(col)` | `.where_not_null(col)` | |
| `->whereDate(col, '2026-05-19')` | `.filter_date(col, NaiveDate)` | `.where_date(col, NaiveDate)` | |
| `->whereMonth(col, 5)` | `.filter_month(col, 5)` | `.where_month(col, 5)` | |
| `->whereDay(col, 19)` | `.filter_day(col, 19)` | `.where_day(col, 19)` | |
| `->whereYear(col, 2026)` | `.filter_year(col, 2026)` | `.where_year(col, 2026)` | |
| `->whereTime(col, '12:30')` | `.filter_time(col, NaiveTime)` | `.where_time(col, NaiveTime)` | |
| `->whereLike(col, pattern)` | `.filter_like(col, pattern)` | `.where_like(col, pattern)` | |
| `->whereNotLike(col, pattern)` | `.filter_not_like(col, pattern)` | `.where_not_like(col, pattern)` | |
| `->whereBinary(col, val)` | `.filter_binary(col, val)` | `.where_binary(col, val)` | Octet à octet ; MySQL et MariaDB uniquement |
| `->orWhereBinary(col, val)` | `.or_filter_binary(col, val)` | `.or_where_binary(col, val)` | |
| `->whereNotBinary(col, val)` | `.filter_not_binary(col, val)` | `.where_not_binary(col, val)` | |
| `->orWhereNotBinary(col, val)` | `.or_filter_not_binary(col, val)` | `.or_where_not_binary(col, val)` | |
| `->whereJsonContains(col, v)` | `.filter_json_contains(col, v)` | `.where_json_contains(col, v)` | Distribué selon le backend |
| `->whereJsonLength(col, op, n)` | `.filter_json_length(col, op, n)` | `.where_json_length(col, op, n)` | |
| `->whereColumn(a, b)` | `.filter_column(a, b)` | `.where_column(a, b)` | Comparaison colonne à colonne |
| `->whereExists(closure)` | `.filter_exists(builder)` | `.where_exists(builder)` | Sous-requête |
| `->whereHas(rel, closure)` | `.filter_has(rel, fn)` | `.where_has(rel, fn)` | Prédicat de relation (10B) |
| `->whereDoesntHave(rel)` | `.filter_doesnt_have(rel)` | `.where_doesnt_have(rel)` | (10B) |
| `->whereRelation(rel, col, op, v)` | `.filter_relation(...)` | `.where_relation(...)` | (10B) |
| `->whereRaw(sql, bindings)` | `.filter_raw(sql, bindings)` | `.where_raw(sql, bindings)` | |

La famille `binary` compare des octets bruts au lieu de faire
correspondre sous la collation de la colonne. MySQL et MariaDB émettent
`col = binary ?` ; Postgres et SQLite n'ont pas d'opérateur équivalent,
si bien qu'un terminal sur ces backends retourne une erreur au moment
où l'instruction est rendue, plutôt que de retomber sur un `=` dépendant
de la collation. Voir
[Comparaison octet à octet](queries.md#byte-exact-comparison).

Les prédicats bruts liés utilisent des marqueurs `?` portables sur
SQLite, MySQL et PostgreSQL :

```rust
let rows = User::query()
    .filter("active", true)
    .filter_raw(
        "score >= ? AND role = ?",
        vec![serde_json::json!(80), serde_json::json!("admin")],
    )
    .get()
    .await?;
```

Sur PostgreSQL, Suprnova rebase ces marqueurs après les liaisons de
requête précédentes, si bien que l'exemple rend `$1` pour `active` et
`$2`/`$3` pour le prédicat brut. Utilisez `??` pour un opérateur
point d'interrogation littéral dans un fragment brut lié, comme
`"payload ?? 'enabled' AND status = ?"`. Les fragments `$N` existants
restent acceptés, mais les marqueurs portables évitent de coupler les
sites d'appel à la position dans la requête. Le mélange de styles de
marqueurs et les décalages entre nombre de marqueurs et de liaisons
sont rejetés avant l'E/S base de données. Comme pour toute expression
brute, le texte SQL doit être fiable ; les valeurs non fiables n'ont
leur place que dans le vecteur de liaisons.

### Tri

```php
$users = User::orderBy('name', 'asc')->get();
$users = User::orderByDesc('created_at')->get();
$users = User::latest()->get();        // raccourci : orderBy(created_at, desc)
$users = User::oldest()->get();        // raccourci : orderBy(created_at, asc)
$users = User::inRandomOrder()->get();
```

```rust
let users = User::query().order_by("name", Direction::Asc).get().await?;
let users = User::query().order_by_desc("created_at").get().await?;
let users = User::latest().get().await?;
let users = User::oldest().get().await?;
let users = User::query().in_random_order().get().await?;
```

`Direction::Asc` / `Direction::Desc` est l'enum Suprnova ré-exporté
depuis SeaORM.

#### Trier selon une séquence explicite

`in_order_of` trie les lignes dans l'ordre que vous listez. Tout ce dont
la valeur ne figure pas dans la liste est trié après tout ce qui y
figure.

```php
$users = User::inOrderOf('role', ['admin', 'member', 'guest'])->get();
```

```rust
let users = User::query()
    .in_order_of("role", ["admin", "member", "guest"])
    .get()
    .await?;
```

Suprnova rend cela sous forme d'expression `CASE` liée : les valeurs
sont donc des paramètres et peuvent sans danger venir des données de la
requête :

```sql
ORDER BY CASE WHEN role = ? THEN 0 WHEN role = ? THEN 1 WHEN role = ? THEN 2 ELSE 3 END
```

Le nom de colonne est un identifiant SQL, pas un paramètre. Codez-le en
dur ou choisissez-le dans une liste blanche, comme tout autre argument
de colonne. Une liste de valeurs vide n'ajoute aucun tri du tout : vous
pouvez donc construire la séquence conditionnellement sans traiter le
cas vide à part.

Pour une colonne qui utilise le cast `AsEnum<E>`, passez chaque variante
par `as_ref()`. C'est exactement la chaîne que le cast stocke :

```rust
let users = User::query()
    .in_order_of("role", [Role::Admin.as_ref(), Role::Member.as_ref()])
    .get()
    .await?;
```

`in_order_of` est livré sur la surface typée `Builder<M>`. Le builder
sans modèle `DB::table(...)` ne trie que par colonne et direction.

### Regroupement + having

```php
$rows = User::groupBy('role')->having('count(*)', '>', 5)->get();
```

```rust
let rows = User::query()
    .group_by("role")
    .having_op("count(*)", ">", 5)
    .get()
    .await?;
```

### Limit / offset

```php
$users = User::limit(10)->offset(20)->get();
$users = User::take(10)->skip(20)->get();   // alias
```

```rust
let users = User::query().limit(10).offset(20).get().await?;
let users = User::query().take(10).skip(20).get().await?;
```

### Select / add_select / select_raw

```rust
let users = User::query().select(["id", "name", "email"]).get().await?;
let users = User::query().select("name").add_select("email").get().await?;
let rows  = User::query().select_raw("count(*) as total, role")
    .group_by("role")
    .get_raw()
    .await?;
```

`get_raw()` retourne le résultat brut à la forme colonne pour les cas
`select_raw` où les colonnes ne correspondent pas au schéma du
modèle ; `get()` retourne `Vec<User>` et exige que les colonnes
sélectionnées remplissent la struct du modèle.

### Distinct

```rust
let emails: Vec<String> = User::query().distinct().pluck("email").await?;
```

### Agrégats

```rust
let count   = User::count().await?;
let count   = User::filter("active", true).count().await?;
let sum     = User::sum::<f64>("balance").await?;
let avg     = Order::avg::<f64>("total").await?;
let min     = Order::min::<DateTime<Utc>>("created_at").await?;
let max     = Order::max::<DateTime<Utc>>("created_at").await?;
let exists  = User::filter("email", &email).exists().await?;
let missing = User::filter("email", &email).doesnt_exist().await?;
```

Les agrégats sont génériques sur le type de retour parce que SeaORM a
besoin de savoir vers quoi coercer le scalaire de la BD. Défauts de
type : `count -> i64` ; `sum`/`avg` portent un paramètre de type
explicite. Suprnova aliase en interne les expressions d'agrégat
générées pour que le même résultat typé soit décodé sur PostgreSQL,
MySQL et SQLite. `sum` et `avg` retournent zéro pour un ensemble de
correspondance vide, tandis que `min` et `max` retournent `None`. Un
type Rust demandé incompatible ou une colonne de résultat manquante
est une erreur de base de données ; ce n'est jamais converti en un
zéro ou un `None` plausible.

### Terminaux

```rust
let users:  Vec<User>          = User::all().await?;
let first:  Option<User>       = User::first().await?;
let user:   User               = User::first_or_fail().await?;
let value:  Option<String>     = User::filter("...").value("email").await?;
let emails: Vec<String>        = User::pluck::<String>("email").await?;
let keyed:  HashMap<i64, String> = User::pluck_keyed::<i64, String>("id", "name").await?;
let ids:    Vec<i64>           = User::query().model_keys().await?;

let sql:    String             = User::filter("...").to_sql();
```

`to_sql` retourne le SQL paramétré que le prochain terminal
émettrait - utile pour déboguer ou construire des vues. Les liaisons
sont accessibles via `.to_sql_with_bindings() -> (String, Vec<Value>)`.

`model_keys` est le terminal réservé aux clés : il projette la clé primaire
**qualifiée** (`users.id`) et n'hydrate jamais de modèle, si bien qu'une
question « quelles lignes ont correspondu ? » coûte une colonne plutôt
qu'une ligne entière par correspondance. La qualification lui permet de
survivre à une requête qui joint une autre table ayant son propre `id`. Tout
`select(...)` déjà présent sur le builder est écarté : l'appelant a demandé
des clés.

### Unions

```rust
let first  = User::filter("active", true);
let second = User::filter("role", "admin");
let users  = first.union(second).get().await?;
let users  = first.union_all(second).get().await?;
```

## Verrouillage des lignes

Deux méthodes du builder demandent un verrou de ligne côté base de
données au moment du SELECT :

```rust
// Verrou d'écriture exclusif - bloque les autres transactions qui
// essaient de verrouiller ou d'écrire les mêmes lignes jusqu'à ce
// que cette transaction valide.
let order = Order::query()
    .filter("id", 42)
    .lock_for_update()
    .first_or_fail()
    .await?;

// Verrou de lecture partagé - autorise d'autres lecteurs partagés,
// bloque les writers.
let inventory = Inventory::query()
    .filter("sku", sku)
    .shared_lock()
    .first_or_fail()
    .await?;
```

SQL émis par backend :

| Backend  | `lock_for_update()` | `shared_lock()`        |
|----------|---------------------|------------------------|
| Postgres | `FOR UPDATE`        | `FOR SHARE`            |
| MySQL    | `FOR UPDATE`        | `LOCK IN SHARE MODE`   |
| SQLite   | (pas de SQL, voir plus bas) | (pas de SQL, voir plus bas) |

La clause de verrou est ajoutée à la toute fin de l'instruction
composée - après chaque bras `UNION`, chaque `ORDER BY`, chaque
`LIMIT` / `OFFSET`. Un `union(...)` de deux builders suivi de
`.lock_for_update()` émet exactement **un seul** `FOR UPDATE` à la
portée externe, pas un par bras.

Pour recharger un modèle que vous détenez déjà et prendre le verrou
dans la même instruction, utilisez `refresh_for_update` :

```rust
DB::transaction(|tx| async move {
    let mut order = Order::find_or_fail(42).await?;
    order.refresh_for_update().await?;   // SELECT ... WHERE id = ? FOR UPDATE
    order.status = "processed".into();
    order.save_with_tx(&tx).await?;
    Ok(())
}).await?;
```

### Utilisation à l'intérieur d'une transaction

Le verrou n'est utile qu'**à l'intérieur d'une transaction** - sans
elle, le SQL s'émet quand même mais le verrou se relâche à la fin de
l'instruction. Associez-le à `DB::transaction(...)` :

```rust
DB::transaction(|tx| async move {
    let order = Order::query()
        .filter("id", 42)
        .lock_for_update()
        .first_or_fail()
        .with_tx(&tx)
        .await?;
    // Les autres transactions qui essaient de verrouiller id=42
    // bloquent ici jusqu'au commit.
    order.status = "processed".into();
    order.save_with_tx(&tx).await?;
    Ok(())
}).await?;
```

### `lock_for_update` vs `shared_lock`

La plupart des flux « lire puis écrire » veulent `lock_for_update`.
Un verrou partagé laisse quand même un autre lecteur `shared_lock`
vous devancer jusqu'à un `UPDATE` qui suit - seul `FOR UPDATE` est
mutuellement exclusif.

`shared_lock` est le bon choix pour des lectures d'instantané
cohérent où vous lisez une ligne, en tirez une décision, et n'écrivez
pas en retour - p. ex. une vérification de stock qui ne décrémente
pas elle-même le stock.

### SQLite

SQLite n'a pas de verrouillage au niveau ligne. Il n'utilise qu'un
verrouillage de transaction au niveau fichier (`BEGIN IMMEDIATE` /
`BEGIN EXCLUSIVE`). Les méthodes de verrou sont **conservées** dans
le chemin SQLite pour que le code cross-backend compile, mais elles
n'émettent aucun SQL.

La première fois par processus que `lock_for_update` / `shared_lock`
s'exécute contre un backend SQLite, le framework journalise un
unique `warn!` sur la cible de traçage `suprnova::eloquent::lock`.
Cela fait remonter le sans-effet sans inonder les chemins de code à
haut volume.

Si vous avez besoin de garanties de contention inter-lignes sur
SQLite, enveloppez la section critique dans une transaction
`BEGIN IMMEDIATE` explicite - au niveau fichier, cela bloque tout
autre writer.

### Ce qui n'est pas dans la v1

- **`NOWAIT` / `SKIP LOCKED`** - utiles pour des flux de
  réclamation de job-queue, mais ils ajoutent de la surface d'API.
  Différé jusqu'à ce qu'un vrai consommateur en ait besoin.

## Transactions

Suprnova livre trois points d'entrée pour les transactions de base de
données, plus le rollback imbriqué via des savepoints. Deux d'entre
eux - la forme closure et le helper de réessai sur deadlock -
installent un contexte ambiant pour que les opérations de modèle à
l'intérieur de la closure soient automatiquement routées à travers la
transaction, sans que les appelants n'aient à faire circuler un
handle à travers chaque site d'appel.

### Forme closure - `DB::transaction`

La forme closure est le cas courant. La closure reçoit un
`&Transaction` qu'elle peut utiliser pour créer un point de contrôle
via `savepoint(name)` ; chaque opération `Model::*` / `Builder::*` à
l'intérieur de la closure se route automatiquement à travers la
transaction via un `tokio::task_local!` appelé `CURRENT_TX`.

```rust
use suprnova::{DB, FrameworkError, Model};

DB::transaction(|_tx| {
    Box::pin(async move {
        let mut alice = User::query().filter("name", "alice").first_or_fail().await?;
        alice.balance -= 30;
        alice.save().await?;

        let mut bob = User::query().filter("name", "bob").first_or_fail().await?;
        bob.balance += 30;
        bob.save().await?;
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

- La closure retourne `Ok` → **commit**.
- La closure retourne `Err` → **rollback** (l'erreur d'origine se
  propage).
- La closure panique → rollback (la transaction en cours est
  droppée au déroulement de la pile ; le `drop` de
  `DatabaseTransaction` de SeaORM fait le rollback).

Les lectures à l'intérieur de la closure voient les écritures de la
même transaction (via une consultation de `CURRENT_TX` à chaque
appel SQL feuille). Le premier appel `DB::transaction` après le
démarrage du processus récupère le backend de base de données
depuis `DB::connection()` ; les appels suivants réutilisent le même
registre de connexions.

La signature utilise une contrainte de trait de rang supérieur plus
un `Pin<Box<dyn Future>>` pour que les closures puissent emprunter
`tx` à travers des points `.await` :

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        // ... travail avant le savepoint ...
        tx.savepoint("inner").await?;
        // ... travail interne ...
        if some_condition {
            tx.rollback_to("inner").await?;
        }
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

La forme `Box::pin(async move { ... })` est le prix à payer pour
laisser la future utiliser `&tx` après un `.await` - sans cela, la
durée de vie de l'emprunt ne peut pas s'échapper du corps de la
closure. Reflète la signature `TransactionTrait::transaction` de
SeaORM.

### Savepoints - `tx.savepoint(name)` / `tx.rollback_to(name)`

Les savepoints jalonnent la transaction pour que vous puissiez
abandonner un bloc de travail interne sans annuler le commit
externe. Fonctionne sur les trois backends - le `SAVEPOINT` de
SQLite est pleinement fonctionnel même si SQLite n'a pas de
verrouillage au niveau ligne.

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        let mut account = Account::query().filter("id", id).first_or_fail().await?;
        account.balance = 200;
        account.save().await?;     // validé quand la tx externe valide

        tx.savepoint("audit_trail").await?;

        let entry = AuditEntry::create(attrs! { actor_id: actor, ... }).await?;
        if audit_validation_failed(&entry) {
            tx.rollback_to("audit_trail").await?;
            // ligne audit_trail disparue ; la mise à jour du compte
            // est toujours en attente de commit
        }

        Ok::<(), FrameworkError>(())
    })
}).await?;
```

Le nom du savepoint est interpolé tel quel dans le SQL - utilisez un
identifiant statique, n'y **injectez pas** d'entrée utilisateur.

### `DB::transaction` imbriqué est rejeté à l'exécution

```rust
DB::transaction(|_outer| Box::pin(async move {
    let inner = DB::transaction(|_inner| Box::pin(async move {
        Ok::<(), FrameworkError>(())
    })).await;
    // inner vaut Err(FrameworkError::Database(
    //     "nested DB::transaction is not supported; use tx.savepoint(name) for nested rollback"
    // ))
    Ok::<(), FrameworkError>(())
})).await?;
```

Le `DatabaseConnection::begin()` de SeaORM ne se compose pas -
l'appeler sur une connexion qui détient déjà une transaction démarre
une toute nouvelle transaction physique qui valide / annule
indépendamment de la portée externe. C'est un piège silencieux pour
l'intégrité des données, donc `DB::transaction` vérifie `CURRENT_TX`
en amont et retourne une erreur de base de données plutôt que de
produire la mauvaise sémantique. Utilisez `tx.savepoint(name)` pour
un comportement imbriqué.

### Réessai sur deadlock - `DB::transaction_with_attempts`

Les lectures Postgres `SERIALIZABLE` et les verrous au niveau ligne
de MySQL peuvent lever des erreurs de serialization-failure /
deadlock qui se résolvent en réessayant la transaction.
`transaction_with_attempts` relance la closure depuis zéro chaque
fois, jusqu'à `attempts` :

```rust
DB::transaction_with_attempts(3, |_tx| {
    Box::pin(async move {
        // Logique isolée en SERIALIZABLE qui peut entrer en course
        // avec une tx concurrente et faire remonter SQLSTATE
        // 40001 / 40P01 au commit.
        let inventory = Inventory::query()
            .filter("sku", sku)
            .lock_for_update()
            .first_or_fail()
            .await?;
        if inventory.units < requested {
            return Err(FrameworkError::bad_request("out of stock"));
        }
        Inventory::query()
            .filter("sku", sku)
            .update(attrs! { units: inventory.units - requested })
            .await?;
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

La détection se fait par sous-chaîne du texte Display de l'erreur
interne :

- SQLSTATE Postgres `40001` (serialization_failure)
- SQLSTATE Postgres `40P01` (deadlock_detected)
- Sous-chaîne `"deadlock"` insensible à la casse (couvre le
  `Deadlock found when trying to get lock` de MySQL et toute
  chaîne de deadlock remontée par l'utilisateur)

À la dernière tentative, l'erreur se propage inchangée. La closure
s'exécute depuis zéro à chaque tentative - capturez un état possédé
ou des `Arc` plutôt que des références `&mut`, pour que le chemin de
réessai soit bien défini.

> **Mise en garde :** comme la détection inclut une sous-chaîne
> `"deadlock"` insensible à la casse (nécessaire pour MySQL, dont le
> driver ne fait pas remonter de SQLSTATE), toute erreur interne dont
> le `Display` contient ce mot déclenche un réessai. Quand vous
> levez vos propres erreurs depuis une closure
> `transaction_with_attempts`, évitez le mot « deadlock » dans le
> message - sinon une erreur de validation sans rapport réessaie
> jusqu'à `attempts` fois avant de se propager. Les correspondances
> SQLSTATE de Postgres (`40001` / `40P01`) sont le signal fiable ;
> l'heuristique n'est là que pour MySQL.

### Forme manuelle - `DB::begin_transaction` + les shims `*_with_tx`

Quand la durée de vie de la transaction ne tient pas dans une
closure (p. ex. si elle s'étend sur plusieurs branches de flux de
contrôle), ouvrez une `Transaction` manuelle et faites adhérer
chaque opération à celle-ci explicitement :

```rust
let tx = DB::begin_transaction().await?;

let mut user = User::query()
    .filter("name", "alice")
    .with_tx(&tx)
    .first_or_fail()
    .await?;
user.balance = 500;
user.save_with_tx(&tx).await?;

if some_condition {
    let mut other = User::query()
        .filter("name", "bob")
        .with_tx(&tx)
        .first_or_fail()
        .await?;
    other.update_with_tx(&tx, attrs! { balance: 200i64 }).await?;
}

tx.commit().await?;  // ou tx.rollback().await?;
```

Le mode manuel n'installe **pas** `CURRENT_TX`. Cantonnez les
opérations individuelles à la transaction avec
`Builder::with_tx(&tx)` ou les shims `Model::*_with_tx(&tx, ...)` :

| Méthode de trait      | Variante manuelle                         |
|---------------------|-------------------------------------------|
| `Model::create`     | `Model::create_with_tx(&tx, attrs)`       |
| `Model::save`       | `Model::save_with_tx(&tx)`                |
| `Model::update`     | `Model::update_with_tx(&tx, attrs)`       |
| `Model::delete`     | `Model::delete_with_tx(&tx)`              |
| `Model::force_delete` | `Model::force_delete_with_tx(&tx)`      |
| `Builder::*`        | `Builder::with_tx(&tx).*`                 |

Détenir une `Transaction` épingle une connexion du pool pour toute
la durée de vie du handle. Sur SQLite, le pool n'a qu'une seule
connexion, donc toute lecture parallèle non transactionnelle contre
la même base de données bloque jusqu'à ce que la transaction se
termine - **chargez toute ligne nécessaire en amont AVANT
`DB::begin_transaction()`** et routez chaque écriture dépendante à
travers le `tx` retourné.

`Transaction::commit` / `Transaction::rollback` consomment le
handle et exigent un `Arc::try_unwrap` de la transaction SeaORM
interne ; si un clone quelconque de `TxHandle` (depuis
`tx.handle()` / `Builder::with_tx(&tx)`) est encore vivant au
moment du commit / du rollback, les deux échouent avec une erreur
« TxHandle clones still alive ». Le bon correctif est de dropper
votre `Builder<M>` / vos handles en suspens avant d'appeler
`commit` - le framework refuse de faire courir une écriture à
moitié non validée contre un writer parallèle détenant la même tx.

### Précédence

Précédence à trois niveaux pour le routage d'une opération à
travers une connexion :

1. **Redéfinition au niveau du builder** - `Builder::with_tx(&tx)`
   ou n'importe quel shim `Model::*_with_tx(&tx, ...)`. L'explicite
   l'emporte sur l'ambiant.
2. **`CURRENT_TX` ambiant** - installé par `DB::transaction` /
   `DB::transaction_with_attempts` pour la portée de tâche de la
   closure.
3. **Repli sur le pool** - `DB::connection()` retourne le singleton
   `DbConnection` global.

À l'intérieur de `DB::transaction(|tx| ...)`, appeler
`Builder::with_tx(&other_tx)` explicitement route cette
requête-là à travers `other_tx` - en contournant le `CURRENT_TX`
ambiant. C'est presque certainement un bug ; le chemin de
redéfinition existe pour la forme manuelle, pas pour redéfinir la
propre tx de la closure.

### `with_tx` et les scopes globaux

Un builder qui porte une `tx_override` respecte quand même les
scopes globaux, les scopes nommés, et le plan de chargement hâtif -
la redéfinition ne change que le routage de connexion, pas le SQL.

### Limitations (v1)

- **Chargements hâtifs de relation** - `Builder::with(["posts"])`
  et `Collection::load(["posts"])` routent les sous-requêtes
  hâtives `IN (...)` à travers `DB::connection()`, pas à travers la
  transaction active. Les écritures en attente à l'intérieur d'une
  closure `DB::transaction` ne sont **pas** visibles pour les
  relations chargées via `.with(...)`. Pour l'instant, cantonnez
  le travail transactionnel aux appels directs `Model::*` /
  `Builder::*` / `DB::table(...)` ; différez les chargements de
  relation jusqu'à ce que l'écriture externe soit posée (ou avant
  `DB::begin_transaction` sur le chemin manuel). C'est une couture
  connue - le helper de routage (`ExecutorChoice`) est déjà en
  place à chaque feuille SQL ; le point bloquant est que
  `EagerLoadDispatch::eager_load` prend une `&DatabaseConnection`
  (concrète), que la macro émet pour chaque type de relation. Un
  passage ultérieur adaptera le trait au helper de dispatch.
- **DDL sur Postgres** - `DB::statement(...)` à l'intérieur d'une
  transaction exécute le DDL contre la connexion de la tx, ce que
  Postgres autorise ; MySQL valide implicitement et n'est donc pas
  pris en charge à l'intérieur d'une transaction Suprnova (ceci
  correspond à la mise en garde du `DB::transaction` de Laravel).

## Scopes

Suprnova livre deux types de scope, à l'image de Laravel :

- **Scopes locaux** - des méthodes d'extension sur le builder,
  déclarées par modèle avec `#[suprnova::scopes(Model)]`. Chaque
  fonction libre dans le bloc `impl` annoté devient à la fois
  `Model::name()` (un démarreur statique) et `Builder::name()` (une
  méthode chaînable).
- **Scopes globaux** - des implémentations de `GlobalScope<M>`
  enregistrées au démarrage via
  `ScopeRegistry::register::<M, _>(scope)`. Chaque appel
  `Model::query()` les superpose automatiquement.

### Scopes locaux

Déclarez des scopes locaux en leur donnant la forme
`fn(query: Builder<Self>, args...) -> Builder<Self>` :

```rust
#[suprnova::scopes(User)]
impl User {
    pub fn active(query: Builder<Self>) -> Builder<Self> {
        query.filter("active", true)
    }

    pub fn popular(query: Builder<Self>, threshold: i64) -> Builder<Self> {
        query.filter_op("followers_count", ">", threshold)
    }
}

// À utiliser soit comme démarreur, soit comme méthode chaînable :
let active_users  = User::active().get().await?;
let popular_users = User::query().active().popular(500).get().await?;
```

Les méthodes non-scope déclarées dans le même bloc `impl` (tout ce
dont le premier paramètre n'est pas `query: Builder<Self>`) passent
inchangées.

### Scopes globaux

Les scopes globaux s'appliquent à chaque appel `Model::query()`. Le
cas d'usage classique est le multi-tenant - chaque lecture est
cantonnée au tenant courant sans que chaque appelant ait à faire
passer le filtre.

```rust
use suprnova::eloquent::scopes::{GlobalScope, ScopeRegistry};

pub struct TenantScope;

impl GlobalScope<Article> for TenantScope {
    fn apply(&self, query: Builder<Article>) -> Builder<Article> {
        // Lit le tenant courant depuis un task-local / un
        // AtomicI64 / peu importe où vit l'état par requête.
        query.filter("tenant_id", current_tenant_id())
    }
}

// Au démarrage - typiquement à l'intérieur de votre module
// provider/bootstrap :
ScopeRegistry::register::<Article, _>(TenantScope);

// Chaque lecture est automatiquement cantonnée au tenant actif :
let scoped = Article::query().get().await?;
```

Plusieurs scopes sur un même modèle se composent dans l'ordre
d'enregistrement - le premier enregistré s'exécute le premier, si
bien que ses clauses de filtre apparaissent en premier dans la
chaîne WHERE. Les filtres combinés par AND ne se soucient pas de
l'ordre, mais l'ordre gauche-à-droite compte pour toute clause dont
l'ordre d'effet de bord est visible (p. ex. le tri, having, les
fragments bruts).

### L'opt-out d'un scope global

Chaque modèle que la macro `#[suprnova::model]` touche reçoit deux
helpers statiques émis sur lui :

```rust
// Contourne exactement un scope enregistré par type. Les autres scopes s'appliquent quand même.
let all_tenants = Article::without_global_scope::<TenantScope>().get().await?;

// Contourne tous les scopes enregistrés. Motif d'outillage admin.
let everything = Article::without_global_scopes().get().await?;
```

**Important :** les helpers d'opt-out doivent être le point
d'entrée. Chaîner `.without_global_scope::<S>()` sur un builder déjà
retourné par `Model::query()` n'annule pas les scopes qui ont déjà
tourné - `Model::query()` applique les scopes de façon hâtive à la
construction, si bien que le masque est posé trop tard. Utilisez les
helpers statiques par modèle (ci-dessus) pour la sémantique correcte.

### Où s'appliquent les scopes globaux

| Chemin | Les scopes globaux s'appliquent-ils ? |
|------|----------------------|
| `Model::query()` | Oui - le point d'entrée canonique cantonné |
| `Model::without_global_scope::<S>()` | Oui, moins `S` |
| `Model::without_global_scopes()` | Non |
| `Model::find(id)` | Non - la recherche par PK passe directement par SeaORM |
| `Model::find_many([...])` | Non - même raison |
| `Model::all()` | Non - même raison |

Ceci reflète Laravel : `Eloquent\Model::find` ne déclenche pas
`addGlobalScopes`. Les appelants qui veulent des recherches par PK
cantonnées utilisent `Self::query().filter("id", pk).first().await?`.

### Suppressions logicielles et scopes globaux coexistent

`#[suprnova::model(soft_deletes)]` installe le filtre
`deleted_at IS NULL` via un mécanisme de tag-chaîne séparé, pas via
le registre de scope typé. Les deux couches se composent :

- `Model::query()` filtre les lignes à la corbeille ET exécute
  chaque scope enregistré.
- `Model::without_global_scopes()` abandonne les scopes enregistrés
  mais préserve le filtre de suppression logicielle - l'outillage
  admin qui veut lire chaque ensemble de colonnes exclut quand même
  les lignes à la corbeille par défaut.
- `Model::with_trashed()` et `Model::only_trashed()` ignorent le
  filtrage de suppression logicielle et contournent aussi le
  registre (ils construisent un builder neuf, sans scope). Associez
  avec `.without_global_scope::<S>()` si vous avez besoin de
  lectures conscientes des scopes sur les lignes à la corbeille.

## Relations

Suprnova livre chaque variante de relation Eloquent. Elles se
déclarent dans le bloc `relations = { ... }` de
`#[suprnova::model]`, et la macro émet - par relation déclarée - une
méthode sur la struct, un accesseur chargé (`<name>_loaded()`), un
accesseur de compte (`<name>_count()`), et le bras de dispatcher que
le chargeur hâtif appelle. Cette section couvre la forme par type et
le tableau d'options ; la plongée en profondeur sur la résolution des
clés de jointure, le registre polymorphe, les lignes pivot, et
l'abaissement de l'enum polymorphe vit dans
[Relations Eloquent](eloquent-relationships.md). Les types de
relation livrés aujourd'hui :

| Type                | Un/plusieurs | À travers les familles | Porté par |
|---------------------|----------|-----------------|-----------|
| `HasOne<R>`         | un      | non              | requête `IN` sur `<parent>_id` |
| `BelongsTo<R>`      | un      | non              | requête `IN` sur la FK de cette ligne |
| `HasMany<R>`        | plusieurs     | non              | identique à `HasOne`, retourne `Vec<R>` |
| `BelongsToMany<R, P>` | plusieurs   | non              | table pivot `P`, INNER JOIN + `pivot::<P>()` |
| `HasOneThrough<B, R>`  | un   | non              | JOIN à deux requêtes `parent → B → R` |
| `HasManyThrough<B, R>` | plusieurs  | non              | identique à ci-dessus, retourne `Vec<R>` |
| `MorphOne<R>`       | un      | oui             | filtre `IN` + `<name>_type = "<self>"` |
| `MorphMany<R>`      | plusieurs     | oui             | identique à `MorphOne`, retourne `Vec<R>` |
| `MorphTo`           | un      | oui (enfants → plusieurs familles) | enum par famille émis au site de déclaration |
| `MorphToMany<R, P>` | plusieurs     | oui             | pivot m2m polymorphe `P` |
| `MorphedByMany<R, P>` | plusieurs   | oui (inverse)   | même pivot, parcouru dans l'autre sens |

### Syntaxe de `relations = { ... }`

Chaque déclaration de relation porte la même forme externe : le nom
de la relation, le type, le type lié (et les types
pivot/intermédiaire si applicable), et un bloc d'options `{ ... }`.

```rust
use suprnova::model;

#[model(
    table = "users",
    relations = {
        // HasMany<R>
        posts: HasMany<crate::models::Post> {
            fk = "author_id",         // redéfinit le défaut `user_id`
        },
        // BelongsToMany<R, Pivot>
        roles: BelongsToMany<crate::models::Role, crate::models::RoleUser> {
            with_pivot = ["assigned_at"],
            with_timestamps,
        },
    },
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Options courantes :

| Option                     | Types de relation                | Objectif |
|----------------------------|-------------------------------|---------|
| `fk = "..."`               | chaque type avec une FK enfant    | Colonne sur l'ENFANT qui pointe vers le parent. Défaut = `<snake(parent_struct)>_id`. |
| `lk = "..."`               | types un/plusieurs                | Colonne sur le PARENT utilisée comme clé de jointure. Défaut = `"id"`. |
| `related_key = "..."`      | `BelongsToMany`, `MorphToMany` | Le nom de COLONNE de PK côté lié. Défaut = `"id"`. Requis quand le modèle lié utilise une PK autre que `id`. |
| `with_pivot = ["...", ...]` | `BelongsToMany`, `MorphToMany` | Colonnes supplémentaires sur le pivot à faire remonter dans la jointure. |
| `with_timestamps`          | `BelongsToMany`, `MorphToMany` | Estampille `created_at` / `updated_at` lors d'attach/sync. |
| `with_default = \|\| { ... }` | `BelongsTo`                 | Closure qui produit un défaut quand la FK est nulle OU que le parent est absent. |
| `first_key`, `second_key`, `second_local_key` | `HasOneThrough`, `HasManyThrough` | Redéfinitions de clé de JOIN - voir la section Through plus bas. |
| `name = "..."`             | chaque type morph              | Nom de famille morph (p. ex. `"commentable"`, `"taggable"`). Pilote les colonnes `<name>_id` / `<name>_type` sur l'enfant/le pivot. |
| `targets = [T1, T2, ...]`  | `MorphTo`                     | La liste des cibles morph concrètes. La macro émet un enum `<Name>Morph` au site de déclaration, avec une variante par cible plus `Unknown(String, i64)`. |
| `target_morph_type = "..."` | `MorphedByMany`              | La chaîne de morph-type qui identifie la famille cible sur le pivot. |
| `pivot_table`, `pivot_foreign_key`, `pivot_related_key` | `BelongsToMany`, `MorphToMany` | Redéfinitions de colonne / table côté pivot quand les défauts ne conviennent pas. |

### `HasOne<R>` et `BelongsTo<R>`

Un-à-un dans les deux directions. `HasOne` vit côté parent et appelle
`R::query().filter(<fk>, <self.id>).first()`. `BelongsTo` vit côté
enfant et lit la FK depuis `self`, puis appelle
`R::query().filter(<owner_key>, <fk_value>).first()`.

```rust
#[model(table = "users", relations = {
    profile: HasOne<crate::models::Profile>,
})]
pub struct User { /* ... */ }

#[model(table = "profiles", relations = {
    user: BelongsTo<crate::models::User>,
})]
pub struct Profile {
    pub id: i64,
    pub user_id: i64,
    pub bio: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let user = User::find(1).await?.unwrap();
let profile: Option<Profile> = user.profile().first().await?;

let profile = Profile::find(42).await?.unwrap();
let owner: Option<User> = profile.user().first().await?;
```

`BelongsTo` prend en charge `with_default = || R { ... }`, qui se
déclenche soit quand la FK est nulle, soit quand la ligne parente est
absente. La closure de défaut s'exécute par appel (et par ligne
chargée hâtivement) - parfaite pour un substitut vide quand un
utilisateur supprimé a encore des commentaires :

```rust
#[model(table = "comments", relations = {
    author: BelongsTo<crate::models::User> {
        with_default = || User {
            name: "[deleted]".into(),
            ..Default::default()
        },
    },
})]
pub struct Comment { /* ... */ }

let c = Comment::find(99).await?.unwrap();
// Toujours Some - le défaut se déclenche quand la ligne user est absente.
let author = c.author().first().await?.unwrap();
```

### `HasMany<R>`

Un-à-plusieurs côté parent. Retourne un builder fluide ; chaînez
filter / order / latest / take / get / count et terminez.

```rust
#[model(table = "users", relations = {
    posts: HasMany<crate::models::Post> {
        fk = "author_id",
    },
})]
pub struct User { /* ... */ }

let u = User::find(1).await?.unwrap();

// Chaque post de cet utilisateur, tri par défaut :
let posts: Vec<Post> = u.posts().get().await?;

// Filtré + trié + paginé :
let recent = u.posts()
    .filter("published", true)
    .latest()                          // ORDER BY created_at DESC
    .take(10)
    .get()
    .await?;

// COUNT seul - pas de récupération de lignes :
let total: i64 = u.posts().count().await?;
```

Méthodes terminales disponibles : `.first()`, `.get()`, `.count()`.
Filtres chaînables disponibles : `.filter` / `.db_where`,
`.filter_in` / `.where_in`, `.order_by`, `.latest`, `.oldest`,
`.limit`, `.take`.

### `BelongsToMany<R, P>` - pivot de premier ordre

Plusieurs-à-plusieurs à travers un pivot déclaré via
`#[suprnova::model]`. Le pivot est un modèle de premier ordre avec sa
propre identité de ligne - pas un tuple, pas une hash map cachée.
Deux bénéfices clés par rapport à la forme de pivot anonyme de
Laravel :

1. La ligne pivot est type-safe. Lisez les colonnes `with_pivot` via
   `r.pivot::<P>().<column>`, jamais via `r.pivot.get("...")`.
2. Le modèle pivot est accessible depuis le reste du framework
   (fabriques, scopes, casts, hooks) de la même façon que n'importe
   quel modèle.

```rust
#[model(table = "role_user", fillable = ["user_id", "role_id", "assigned_at"])]
pub struct RoleUser {
    pub id: i64,
    pub user_id: i64,
    pub role_id: i64,
    pub assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[model(table = "users", relations = {
    roles: BelongsToMany<crate::models::Role, RoleUser> {
        with_pivot = ["assigned_at"],
        with_timestamps,
    },
})]
pub struct User { /* ... */ }

let u = User::find(1).await?.unwrap();
let admin = Role::create(attrs! { name: "admin" }).await?;

// Mutateurs attach + sync
u.roles().attach(admin.id).await?;
u.roles().attach_with(admin.id, attrs! { assigned_at: chrono::Utc::now() }).await?;
u.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
u.roles().detach(admin.id).await?;

// Lire les données pivot via l'accesseur par downcast, ligne par ligne :
let roles = u.roles().get().await?;
for r in &roles {
    let p: &RoleUser = r.pivot::<RoleUser>();
    println!("user {} got role {} at {:?}", p.user_id, p.role_id, p.assigned_at);
}
```

- `.attach(id)` - INSERT une seule ligne pivot. Échoue sur doublon
  sauf si votre pivot l'autorise (le framework ne déduplique pas à
  la couche Rust ; utilisez `.sync` pour l'idempotence).
- `.attach_with(id, attrs! { ... })` - INSERT avec des colonnes
  pivot supplémentaires. Estampille les timestamps quand
  `with_timestamps` est activé.
- `.detach(id)` - DELETE la ou les lignes pivot qui relient
  parent → id.
- `.sync([ids...])` - diff-and-apply : attach ce qui est nouveau,
  detach ce qui manque, laisse l'intersection intacte. Enveloppé
  dans une transaction.

`.get()` retourne `Vec<R>` avec le pivot estampillé sur le champ
interne `__pivot` de chaque ligne. L'accesseur `.pivot::<P>()`
downcaste l'`Arc<dyn Any>` vers le type pivot que vous avez déclaré.
L'appeler avec le mauvais type panique - faites correspondre le type
au pivot déclaré.

### `HasOneThrough<B, R>` et `HasManyThrough<B, R>`

Atteignez une cible finale `R` à travers un intermédiaire `B`. Utile
quand la relation traverse deux tables mais que vous n'avez pas
besoin d'exposer l'intermédiaire (`A → B → R`).

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<crate::models::User, crate::models::Post>,
})]
pub struct Country {
    pub id: i64,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let c = Country::find(1).await?.unwrap();
let posts: Vec<Post> = c.posts().get().await?;
```

Le dispatcher infère les clés de JOIN à partir des noms de struct.
Redéfinitions :

| Option              | Défaut                          | Description |
|---------------------|----------------------------------|-------------|
| `first_key`         | `<snake(parent_struct)>_id`      | Colonne sur l'intermédiaire `B` qui pointe vers le parent `A`. |
| `second_key`        | `<snake(intermediate_struct)>_id` | Colonne sur la cible finale `R` qui pointe vers l'intermédiaire `B`. |
| `second_local_key`  | `"id"`                           | Colonne sur l'intermédiaire `B` que `second_key` doit égaler. Requise quand `B` utilise une PK autre que `id`. |

La colonne de clé primaire du parent est lue depuis la déclaration
`primary_key` du modèle (par défaut `"id"`) - il n'y a pas de
redéfinition `local_key` sur `HasManyThrough` / `HasOneThrough` ;
changez la PK du parent via l'attribut `#[suprnova::model]` si vous
avez besoin d'une clé parente autre que `id`.

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<crate::models::User, crate::models::Post> {
        first_key = "country_id",
        second_key = "author_id",
    },
})]
pub struct Country { /* ... */ }
```

### `MorphTo` avec `targets = [...]` et l'enum par famille

Les relations polymorphes font pointer une ligne enfant vers une de
plusieurs familles parentes. L'enfant porte une paire
`(<name>_id, <name>_type)` ; la colonne `*_type` détient la chaîne de
morph-type que chaque parent déclare.

`MorphTo` vit sur l'enfant. Sa déclaration liste chaque famille
parente vers laquelle elle peut pointer via `targets = [...]`. La
macro émet un enum par famille nommé `<RelationName>Morph`
(correspondant à la forme PascalCase du nom de la relation, suffixé
par `Morph`) avec une variante par type cible plus
`Unknown(String, i64)` pour les lignes historiques dont la valeur
`<name>_type` ne correspond à aucune cible enregistrée.

```rust
#[model(table = "posts", morph_type = "post")]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video")]
pub struct Video { /* ... */ }

#[model(table = "comments", relations = {
    commentable: MorphTo {
        name = "commentable",
        targets = [
            crate::models::Post,
            crate::models::Video,
        ],
    },
})]
pub struct Comment {
    pub id: i64,
    pub commentable_id: i64,
    pub commentable_type: String,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let c = Comment::find(1).await?.unwrap();
match c.commentable().get().await? {
    CommentableMorph::Post(post)   => println!("comment on post {}", post.title),
    CommentableMorph::Video(video) => println!("comment on video {}", video.url),
    // Lignes historiques / pendantes - `<name>_type` ne correspond
    // à aucune cible, OU le morph_type correspondait mais la ligne
    // à `<name>_id` a disparu.
    CommentableMorph::Unknown(ty, id) => {
        eprintln!("comment {} points at unknown {ty}#{id}", c.id);
    }
}
```

L'attribut `morph_type = "..."` sur chaque struct cible est ce que
le loader écrit dans la colonne `<name>_type` de l'enfant à
l'insertion, et ce sur quoi il filtre à la lecture. Sans
`morph_type`, le framework dérive la chaîne de type depuis
`to_snake(struct_name)`.

Le dispatch de `MorphTo` - comment l'enum par famille choisit la
bonne variante - consulte le registre morph à l'exécution
(l'inventaire peuplé par chaque déclaration
`#[suprnova::model(morph_type = "...")]`). Pour chaque cible
déclarée, le helper de récupération recherche le `TypeId` de la
cible, lit la chaîne `morph_type` enregistrée, et la compare à la
valeur `<name>_type` stockée sur la ligne enfant. La première
correspondance gagne, dans l'ordre de déclaration. Les cibles sans
attribut `morph_type` explicite retombent sur
`to_snake(target_type_name)` - le même défaut que le côté parent
(`MorphMany` / `MorphOne`) utilise pour estampiller la chaîne de
type à l'écriture, si bien que les deux côtés restent alignés. Cela
signifie que des valeurs `morph_type` personnalisées (p. ex.
`morph_type = "blog_post"` sur une struct nommée `Post`, ou toute
chaîne non conventionnelle) se dispatchent correctement sans
changement au site de déclaration.

### `MorphOne<R>` et `MorphMany<R>` - côté parent

La direction inverse de `MorphTo` : un type parent déclare le
un-ou-plusieurs polymorphe qu'il possède. `MorphOne` retourne
`Option<R>` depuis `.first()` ; `MorphMany` retourne `Vec<R>` depuis
`.get()`. Les deux filtrent la paire `(<name>_id, <name>_type)` de
l'enfant par `self.id` et le `morph_type` du parent.

```rust
#[model(table = "posts", morph_type = "post", relations = {
    comments: MorphMany<crate::models::Comment> {
        name = "commentable",
    },
    cover: MorphOne<crate::models::Image> {
        name = "imageable",
    },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    comments: MorphMany<crate::models::Comment> {
        name = "commentable",
    },
})]
pub struct Video { /* ... */ }

let post = Post::find(1).await?.unwrap();
let post_comments: Vec<Comment> = post.comments().get().await?;
let post_cover:    Option<Image> = post.cover().first().await?;

let video = Video::find(1).await?.unwrap();
let video_comments: Vec<Comment> = video.comments().get().await?;
// post.comments() ne retourne que les lignes
// `commentable_type = "post"` ; video.comments() ne retourne que
// les lignes `commentable_type = "video"`.
```

La même surface chaînable que `HasMany` / `HasOne` : `.filter` /
`.db_where`, `.order_by` / `.latest` / `.oldest`, `.limit` / `.take`,
`.first` / `.get` / `.count`.

### `MorphToMany<R, P>` et `MorphedByMany<R, P>`

Plusieurs-à-plusieurs polymorphe. Le pivot partagé `P` porte la paire
de FK PLUS une colonne discriminante `<name>_type`. Un côté déclare
`MorphToMany` (p. ex. `Post.tags()`, `Video.tags()`), l'autre côté
déclare un `MorphedByMany` par famille cible (p. ex. `Tag.posts()`,
`Tag.videos()`).

```rust
#[model(table = "taggables", fillable = ["tag_id", "taggable_id", "taggable_type"])]
pub struct Taggable {
    pub id: i64,
    pub tag_id: i64,
    pub taggable_id: i64,
    pub taggable_type: String,
}

#[model(table = "posts", morph_type = "post", relations = {
    tags: MorphToMany<crate::models::Tag, Taggable> {
        name = "taggable",
    },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    tags: MorphToMany<crate::models::Tag, Taggable> {
        name = "taggable",
    },
})]
pub struct Video { /* ... */ }

// Inverse : Tag déclare un MorphedByMany par famille cible.
#[model(table = "tags", relations = {
    posts: MorphedByMany<crate::models::Post, Taggable> {
        name = "taggable",
        target_morph_type = "post",
    },
    videos: MorphedByMany<crate::models::Video, Taggable> {
        name = "taggable",
        target_morph_type = "video",
    },
})]
pub struct Tag { /* ... */ }

let post  = Post::find(1).await?.unwrap();
let video = Video::find(1).await?.unwrap();
let tag   = Tag::create(attrs! { name: "rust" }).await?;

// `attach` / `attach_with` / `detach` / `sync` fonctionnent de la
// même façon que pour BelongsToMany. La colonne `<name>_type`
// atterrit automatiquement depuis le `morph_type` du parent
// appelant.
post.tags().attach(tag.id).await?;
video.tags().attach(tag.id).await?;          // rattachement indépendant
post.tags().sync([tag_a.id, tag_b.id]).await?;

// Direction inverse - Tag se divise par famille :
let posts_with_tag:  Vec<Post>  = tag.posts().get().await?;   // typé "post"
let videos_with_tag: Vec<Video> = tag.videos().get().await?;  // typé "video"
```

Le `target_morph_type` de `MorphedByMany` est requis parce que la
macro, au site de déclaration de `Tag`, ne peut pas introspecter
l'attribut `morph_type = "..."` de la cible (il vit dans une
invocation `#[suprnova::model]` séparée). Le positionner
explicitement garde chaque bras `MorphedByMany` honnête sur la
famille qu'il scanne.

### Échappatoire : méthodes de relation écrites à la main

Les relations déclarées dans `relations = { ... }` sont les seules
que le dispatcher de chargement hâtif (et `with`, `with_count`,
etc.) connaît. Si une relation est trop inhabituelle pour la forme
de la macro - par exemple une requête qui agrège à travers deux
pivots, ou une vue typée d'une table de cache dénormalisée - vous
pouvez l'omettre de `relations = { ... }` et écrire un simple impl
inhérent :

```rust
impl User {
    /// Posts que cet utilisateur a écrits OU dans lesquels il est
    /// tagué. Traverse deux relations et n'est donc pas exprimable
    /// comme une seule déclaration `relations = { ... }` - écrit à
    /// la main.
    pub async fn posts_touched(&self) -> Result<Vec<Post>, FrameworkError> {
        let authored: Vec<Post> = self.posts().get().await?;
        let tagged:   Vec<Post> = /* ...requête personnalisée... */;
        // ...fusion + déduplication...
        Ok(/* ... */)
    }
}
```

Ces méthodes perdent le support du chargement hâtif -
`User::with(["posts_touched"])` échouera parce que le dispatcher n'a
aucun bras pour `posts_touched`. Les déclarations dans la macro
restent le chemin que le framework sait charger hâtivement, compter,
agréger, et filtrer par prédicat.

### Restrictions v1

Une poignée de choses que la surface v1 met de côté pour plus tard.
Chacune est aussi documentée à son site de déclaration - regroupées
ici pour la visibilité :

- **Les ID morph sont réservés à `i64`.** `MorphTo::morph_id` est
  câblé en dur sur `i64`, donc tout modèle utilisé comme cible
  `MorphTo` doit déclarer une clé primaire `i64`, et la colonne
  `<name>_id` de la table enfant doit aussi être `i64`. Les FK morph
  en `String` / UUID-en-chaîne sont pour la v2.
- **Pas de chargement hâtif imbriqué à travers `MorphTo`.** L'enum
  par famille efface le type de l'enfant, donc un chemin à points
  comme `with(["commentable.user"])` ne peut pas récurser en queue -
  le dispatcher retourne une erreur typée. Résolvez par famille en
  filtrant sur l'enum et en appelant `with(["user"])` sur chaque
  variante individuellement.

## Chargement hâtif

Le chargement hâtif évite les requêtes N+1. Au lieu de
`posts.len()` requêtes pour récupérer les posts de chaque
utilisateur, Suprnova émet UNE requête par relation de premier
niveau, quel que soit le nombre de lignes parentes chargées.

La surface complète - liste plate, chemins imbriqués, compte,
agrégats, et chargements hâtifs filtrés par prédicat - est atteinte
à travers les helpers émis par `#[suprnova::model]` sur chaque
modèle :

```rust
// Relation unique :
let users = User::with(["posts"]).get().await?;
for u in &users {
    for p in u.posts_loaded() { /* ... */ }
}

// Plusieurs relations :
let users = User::with(["posts", "profile"]).get().await?;

// Chemins imbriqués - trois requêtes (users + posts + comments), pas de N+1 :
let users = User::with(["posts.comments"]).get().await?;
let p1 = users[0].posts_loaded()[0];
let comments = p1.comments_loaded();

// L'imbrication plus profonde fonctionne comme attendu :
let users = User::with(["posts.comments.author"]).get().await?;

// Compte à côté des lignes parentes :
let users = User::with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}

// Agrégats - Sum / Avg / Min / Max sur une colonne de relation. La
// lecture ergonomique est l'accesseur `<rel>_sum_of(col)` émis par
// la macro.
let users = User::with_sum(("posts", "views")).get().await?;
let sum: f64 = users[0]
    .posts_sum_of("views")
    .expect("with_sum populated the cache");

// Plusieurs agrégats sur la même relation se composent - la clé de
// cache est la forme large `<rel>_<kind>_<col>`, si bien que des
// types et des colonnes distincts n'entrent pas en collision :
let users = User::with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .with_min(("posts", "id"))
    .get()
    .await?;
let u = &users[0];
let sum = u.posts_sum_of("views").unwrap();   // Some(_) - somme des vues
let avg = u.posts_avg_of("views").unwrap();   // Some(_) - moyenne des vues
let min = u.posts_min_of("id").unwrap();      // Some(Some(_)) - groupe non vide
let max = u.posts_max_of("id");               // None - with_max n'a pas été appelé

// Filtre les enfants chargés hâtivement. La macro émet un helper
// statique typé `with_where_<rel>(closure)` par relation, si bien
// que le type du paramètre de la closure est inféré - pas besoin
// d'épeler `Builder<Post>` :
let users = User::with_where_posts(|q| q.filter("published", true))
    .get()
    .await?;
// Le `Builder<User>` retourné chaîne avec n'importe quelle autre
// méthode de builder de requête de base :
let users = User::with_where_posts(|q| q.filter("published", true))
    .filter("active", true)
    .get()
    .await?;
// La forme générique reste disponible - utile quand le nom de la
// relation est calculé à l'exécution - mais vous devrez nommer le
// type cible sur la closure :
let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// Chaque u.posts_loaded() ne contient que des posts publiés.
```

### Disposition du cache

Les cellules de cache `__eager` par ligne sont indexées par :

- `<rel>` (le NOM de la relation seul) pour `with` et
  `with_count`.
- `<rel>_<kind>_<col>` (p. ex. `posts_sum_views`) pour les quatre
  types d'agrégat - `with_sum` / `with_avg` / `with_min` /
  `with_max`. Cette clé large permet à plusieurs agrégats sur la
  même relation de coexister sur la même ligne sans s'écraser les
  uns les autres.

| Méthode                              | Clé de cache            | Type de cellule de cache   | Valeur si groupe vide |
|-------------------------------------|----------------------|-------------------|-------------------|
| `with(["posts"])`                   | `posts`              | `Vec<Post>`       | `Vec::new()`      |
| `with(["profile"])`                 | `profile`            | `Option<Profile>` | `None`            |
| `with_count(["posts"])`             | `posts`              | `u64`             | `0`               |
| `with_sum(("posts","views"))`       | `posts_sum_views`    | `f64`             | `0.0`             |
| `with_avg(("posts","views"))`       | `posts_avg_views`    | `f64`             | `0.0`             |
| `with_min(("posts","id"))`          | `posts_min_id`       | `Option<f64>`     | `None`            |
| `with_max(("posts","id"))`          | `posts_max_id`       | `Option<f64>`     | `None`            |

La macro émet des accesseurs correspondants sur chaque modèle :

- `<rel>_loaded()` - pour les relations de type collection :
  `&[Post]` (panique si la relation n'a pas été chargée
  hâtivement). Pour les relations à valeur unique :
  `Option<&Profile>`.
- `<rel>_count()` - `u64`. Panique si `with_count(["..."])` n'a pas
  été appelé.
- `<rel>_sum_of(col)` / `<rel>_avg_of(col)` - retournent
  `Option<f64>` (`None` si le `with_sum` / `with_avg` correspondant
  n'a pas été appelé).
- `<rel>_min_of(col)` / `<rel>_max_of(col)` - retournent
  `Option<Option<f64>>` : l'`Option` externe est « `with_min` /
  `with_max` a-t-il été appelé ? », l'`Option` interne est « le SQL
  a-t-il retourné NULL parce que le groupe était vide ? ».

Les accesseurs sont la surface ergonomique - lisez à travers eux
plutôt que d'aller chercher directement dans
`__eager.get_aggregate::<T>(...)`. Ils construisent la même clé de
cache en coulisse via `eloquent::relations::aggregate_cache_key`.

### Composer des agrégats sur la même relation

La clé de cache large veut dire que vous pouvez empiler autant
d'appels `with_*` que vous voulez sur la même relation, dans une
seule requête - sans collision :

```rust
let users = User::with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .with_min(("posts", "id"))
    .with_max(("posts", "id"))
    .get()
    .await?;

let u = &users[0];
let total_views: f64 = u.posts_sum_of("views").unwrap();
let avg_views:   f64 = u.posts_avg_of("views").unwrap();

// Min/Max sont en double-Option parce que le min/max SQL retourne NULL quand c'est vide :
match u.posts_min_of("id") {
    None              => panic!("with_min not called"),
    Some(None)        => println!("no posts yet"),
    Some(Some(min))   => println!("smallest post id: {min}"),
}

// L'accesseur retourne `None` quand le `with_*` correspondant a été sauté :
assert!(u.posts_avg_of("score").is_none()); // jamais appelé avec col="score"
```

### Agrégats et colonnes INTEGER

Un SUM sur une colonne INTEGER atterrit dans le cache en `f64`. Les
bras du dispatcher essaient `try_get::<Option<f64>>` d'abord, puis
retombent sur `try_get::<Option<i64>>().map(|n| n as f64)` pour que
les types COUNT/SUM de SQLite, qui préservent l'INTEGER, ne
coercent pas silencieusement vers `0.0`. Lisez via les accesseurs
émis par la macro, quel que soit le type de la colonne source.

### Routage de prédicat pour `with_where`

`User::with_where_posts(|q| q.filter("published", true))` applique
une closure au `Builder<Post>` interne AVANT que la requête IN
`filter_in(<fk>, parent_ids)` ne soit émise, si bien que seules les
lignes enfant correspondantes atteignent le cache. La macro émet un
helper statique typé `with_where_<rel>` par relation déclarée, si
bien que le type du paramètre de la closure est inféré depuis la
signature de la méthode.

La forme générique
`with_where(("posts", |q: Builder<Post>| q.filter("published", true)))`
reste disponible - utile quand le nom de la relation est calculé à
l'exécution, ou quand vous détenez déjà un `Builder<User>` et voulez
y attacher un prédicat. Elle exige de nommer le type cible sur la
closure parce que le prédicat passe par un `Box<dyn Any>` et Rust ne
peut pas inférer le type à partir du seul nom de la relation. (Les
règles d'orphelin de Rust interdisent à la macro d'ajouter une
méthode typée directement sur `Builder<User>`, donc le raccourci
typé n'est offert que sur le modèle - `User::with_where_<rel>` - pas
comme méthode de chaîne de builder.)

Pour les types polymorphes, le prédicat s'exécute contre la requête
de la table liée - pas contre le scan du pivot.

`with_where` est pris en charge sur chaque type de relation SAUF
`MorphTo`. L'enum par famille de MorphTo efface le type de l'enfant,
si bien qu'aucun `Builder<R>` unique ne couvre toutes les variantes.
Le chargement hâtif imbriqué à travers MorphTo n'est lui non plus
pas pris en charge en v1 - `with(["commentable.user"])`, où
`commentable` est un `MorphTo`, retourne une erreur depuis le
dispatcher de chargement hâtif récursif.

### `Collection::load` / `load_missing`

Quand vous avez déjà récupéré des lignes et voulez charger
hâtivement des relations après coup :

```rust
use suprnova::Collection;

let mut users: Collection<User> = User::all().await?.into();
users.load(["posts.comments"]).await?;
```

`load_missing` est par ligne : chaque ligne de la collection est
partitionnée indépendamment. Les lignes qui ont déjà la relation
nommée en cache restent intactes ; celles qui ne l'ont pas
reçoivent la relation chargée. Reflète la sémantique du
`$collection->loadMissing(...)` de Laravel.

Pour les chemins imbriqués, la partition se répète à chaque niveau.
Avec `load_missing(["posts.comments"])` :

- Les lignes sans `posts` en cache reçoivent le chemin COMPLET
  chargé - `posts` plus leurs `comments`.
- Les lignes AVEC `posts` déjà en cache récursent dans les posts en
  cache et ne chargent `comments` que sur les posts qui n'ont pas
  déjà de comments en cache.

La même partition par ligne se répète à chaque segment
supplémentaire d'un chemin à points plus long
(`"posts.comments.author"` etc.) - à chaque étape, seules les lignes
qui manquent ce segment reçoivent le chargement en masse.

## Pagination

Trois types de paginateur se composent au-dessus de `Builder<M>` :

| Méthode | Retourne | Requêtes par page | À utiliser quand |
|--------|---------|------------------|----------|
| `paginate(per_page)` | `LengthAwarePaginator<M>` | 2 (COUNT + LIMIT) | L'UI a besoin du nombre total de pages |
| `simple_paginate(per_page)` | `Paginator<M>` | 1 (LIMIT + 1) | Grandes tables ; bouton « Suivant » seul |
| `cursor_paginate(per_page)` | `CursorPaginator<M>` | 1 (LIMIT + 1) | Défilement infini ; pagination profonde |

Les trois implémentent `Serialize` avec la forme JSON standard de
Laravel, si bien qu'ils partent directement vers les consommateurs
Inertia / JSON sans remodelage.

### À total connu

```rust
use suprnova::LengthAwarePaginator;

let page: LengthAwarePaginator<User> = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;

// page.data : Vec<User>
// page.total : u64 - nombre total de lignes sur toutes les pages
// page.last_page : u64 - index de la dernière page, à partir de 1
// page.current_page : u64
// page.per_page : u64
// page.from / page.to : Option<u64> - bornes de fenêtre, à partir de 1
// page.path : Option<String> - URL de base facultative pour la génération de liens
```

L'analyse du paramètre de page lit `?page=N` depuis la requête
active via `Context::query_param`. Pour paginer plusieurs listes sur
la même page avec leurs propres clés de requête, utilisez
`paginate_using` :

```rust
let posts = Post::query().paginate_using("posts_page", 10).await?;
let comments = Comment::query().paginate_using("comments_page", 25).await?;
```

**Forme JSON :**

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

`path` est omis du JSON quand non défini.

### Pagination simple (sans count)

`paginate` exécute toujours deux requêtes - un `COUNT(*)` plus la
récupération de la page. Sur de grandes tables, le count seul peut
dominer le temps de requête. `simple_paginate` élimine complètement
le count ; à la place, il récupère `per_page + 1` lignes et rapporte
si une page suivante existe via le flag `has_more` :

```rust
use suprnova::Paginator;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;

// page.has_more : bool - y avait-il une ligne supplémentaire au-delà de per_page ?
// page.current_page, page.per_page, page.data, page.path : comme ci-dessus.
```

**Forme JSON :**

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true
}
```

### Pagination par curseur (keyset)

La pagination par curseur est le bon choix pour le défilement
infini, la pagination profonde, ou partout où un ordre de ligne
stable avec une recherche par page bon marché en O(1) vaut plus
qu'une UI de pages numériques. Bidirectionnelle - elle lit le
paramètre de requête `?cursor=<opaque>`, avance ou recule selon la
direction du curseur, et émet `next_cursor` et `prev_cursor` selon
que les pages voisines existent (à l'image du `cursorPaginate()` de
Laravel).

```rust
use suprnova::CursorPaginator;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;

// page.data : Vec<User>
// page.per_page : u64
// page.next_cursor : Option<String> - curseur opaque pour la page suivante (None sur la dernière)
// page.prev_cursor : Option<String> - curseur opaque pour la page précédente (None sur la première)
// page.path : Option<String>
```

Les curseurs sont **chiffrés et authentifiés** via
`CursorPaginator::encode_value` - ils encodent la borne du keyset
(la clé primaire du modèle) plus une étiquette de direction, scellée
en AES-256-GCM avec l'`APP_KEY` du framework. Une falsification
produit une erreur 400 ParamParse ; le curseur est opaque pour le
client et infalsifiable sans la clé.

La requête suivante transmet le curseur via `?cursor=<opaque>` :

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

La pagination par curseur **remplace** tout `ORDER BY` existant sur
le builder - un ordre PK ASC stable est requis pour que
`gt(boundary)` découpe de façon déterministe.

**Forme JSON :**

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` et `prev_cursor` sont toujours présents comme clés
JSON (émis en `null` quand absents) pour que les schémas client
puissent compter sur la présence du champ ; `path` est omis quand
non défini.

### Erreurs

| Condition | Variante | HTTP |
|-----------|---------|------|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| Curseur invalide (mauvais base64, JSON, ou échec HMAC) | `FrameworkError::Internal` depuis `Crypt::decrypt_string` | 500 |
| Échec de BD sous-jacent | `FrameworkError::Database` | 500 |

L'échec d'authentification du curseur remonte comme `Internal` (pas
`ParamParse`) pour qu'un curseur falsifié ne divulgue pas
d'information de niveau protocole au client ; le corps de la réponse
porte quand même une raison lisible par un humain.

### Lire les paramètres de requête hors d'une vraie requête

Les tests, les commandes console, et les workers en arrière-plan ne
s'exécutent pas à l'intérieur d'une requête hyper - donc
`Context::query_param("page")` retourne `None` et `paginate` retombe
sur la page 1. Les tests qui ont besoin d'exercer une page précise
peuvent installer une redéfinition par thread :

```rust
use suprnova::context::Context;

#[tokio::test]
async fn paginate_page_2() {
    Context::test_clear_query();
    Context::test_set_query("page", "2");

    let page = User::query().paginate(10).await.unwrap();
    assert_eq!(page.current_page, 2);

    Context::test_clear_query();
}
```

`test_set_query` / `test_clear_query` sont verrouillés derrière la
feature `testing` (activée par défaut dans `framework/Cargo.toml`)
si bien que les builds de release ne voient jamais cette surface.

## Itération par chunk et en mode lazy

Sept points d'entrée de streaming sur `Builder<M>` vous laissent
traiter de grands jeux de résultats en mémoire bornée. Choisissez
selon le compromis :

| Méthode | Pagination | Sûr en concurrence ? | Retourne |
|--------|-----------|------------------|---------|
| `chunk(n, async \|batch\| { ... })` | OFFSET | Non | `Result<(), _>` |
| `chunk_by_id(n, async \|batch\| { ... })` | Curseur PK | **Oui** | `Result<(), _>` |
| `chunk_map(n, async \|batch\| { ... })` | OFFSET | Non | `Collection<U>` |
| `each(async \|row\| { ... })` | OFFSET, taille 1 | Non | `Result<(), _>` |
| `lazy()` | Curseur PK, lot de 1000 | **Oui** | `LazyCollection<M>` |
| `lazy_by_id(batch_size)` | Curseur PK, lot personnalisé | **Oui** | `LazyCollection<M>` |
| `cursor()` | Alias de `lazy()` | **Oui** | `LazyCollection<M>` |

### chunk - lots paginés par OFFSET

```rust
use suprnova::{Collection, Model};

User::query().chunk(100, |batch: Collection<User>| async move {
    for user in &batch {
        send_welcome_email(user).await?;
    }
    Ok(())
}).await?;
```

La closure reçoit un `Collection<M>` par lot - l'accès à la forme
slice (`.iter()`, l'indexation) fonctionne directement via `Deref`.

`chunk` est paginé par OFFSET et **n'est pas sûr sous des insertions
concurrentes** : les lignes insérées avant l'offset du lot suivant
sont sautées ; les lignes supprimées avant l'offset sont traitées
deux fois (ce qui a glissé dans leur emplacement). Utilisez
`chunk_by_id` pour du traitement en masse de qualité production
contre des tables sous charge d'écriture.

### chunk_by_id - lots par curseur PK, sûr en concurrence

```rust
User::query().chunk_by_id(500, |batch| async move {
    for user in &batch {
        reindex_user(user).await?;
    }
    Ok(())
}).await?;
```

Chaque lot filtre sur `WHERE id > last_id ORDER BY id ASC LIMIT n`,
si bien que les lignes insérées en cours d'itération avec des PK
au-dessus du curseur atterrissent dans un lot ultérieur (ou sont
récupérées par une exécution suivante) - elles ne causent jamais le
saut ou le doublon d'une ligne d'origine.

`chunk_by_id` exige une clé primaire `i64`. Les modèles avec des PK
`String` / `Uuid` utilisent `chunk` avec la réserve OFFSET.
(Généraliser la forme du curseur à des clés autres que `i64` est sur
la liste de suivi.)

### chunk_map - chunk + map par chunk

```rust
let totals: Collection<i64> = Order::query()
    .chunk_map(1000, |batch| async move {
        let sum: i64 = batch.iter().map(|o| o.amount).sum();
        Ok(Collection::from_vec(vec![sum]))
    })
    .await?;
```

Fait passer chaque lot à travers `f`, concatène la sortie mappée, et
retourne un unique `Collection<U>`. Borné en mémoire seulement quand
`U` est strictement plus petit que `M` - choisissez ceci quand vous
produisez des résumés (totaux par lot, ids, agrégats) plutôt que des
lignes transformées.

### each - une ligne à la fois, OFFSET

```rust
User::query().each(|user| async move {
    send_welcome_email(&user).await?;
    Ok(())
}).await?;
```

Du sucre pour `chunk(1, ...)` - une requête par ligne. Pour de
grands jeux de données, passez à `lazy()` qui met en lot en interne
(1000 lignes par récupération par défaut) tout en continuant à faire
remonter une ligne à la fois au consommateur.

### lazy / lazy_by_id / cursor - flux

```rust
let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

`lazy()` retourne un `LazyCollection<M>` - un wrapper de flux `Send`
qui produit un `Result<M, FrameworkError>` par ligne. La
contre-pression fonctionne naturellement : un consommateur lent se
gare au point `await` et le lot suivant ne se récupère que quand le
buffer en mémoire se vide.

`lazy()` met en lot via un curseur PK avec une taille par défaut de
1000 lignes. Redéfinissez la taille de lot avec `lazy_by_id(500)`.
`cursor()` est le nom Laravel et un alias sans coût pour `lazy()`.

Même contrainte de PK `i64` que `chunk_by_id`.

### Chargements hâtifs à l'intérieur des chunks

Les sept points d'entrée **rejettent `.with(...)` d'emblée** avec un
`FrameworkError::internal` bien visible. Le clone inter-lot du
Builder abandonne le plan de chargement hâtif à type effacé (son
prédicat `dyn Any` boîté n'est pas clonable sans resserrer l'API
publique), donc honorer le plan serait silencieusement incohérent
d'un lot à l'autre. Ré-appliquez `.with(...)` à l'intérieur de la
closure par chunk quand nécessaire - le `Collection<M>` de chaque
lot se compose avec `load(...)` / `load_missing(...)` :

```rust
User::query().chunk(100, |batch| async move {
    let mut batch = batch;
    batch.load("posts").await?;
    for u in &batch {
        let posts = u.posts_loaded();
        // ...
    }
    Ok(())
}).await?;
```

## Collections

`Collection<T>` est la collection en forme Laravel de Suprnova - le
type de retour de `Builder::get` (où `T` est le modèle), de
`Model::all`, de `pluck` / `chunk_map`, et de tout autre terminal qui
produit plus d'une ligne. Elle déréférence vers `&[T]`, si bien que
les sites d'appel Vec existants continuent de fonctionner sans
changement ; la surface Laravel se compose par-dessus. Cette section
est la surface du quotidien ; l'index complet des méthodes, la
séparation générique-vs-modèle, le wrapper de streaming
`LazyCollection<M>`, et les règles emprunt-vs-consommation sont dans
[Collections Eloquent](eloquent-collections.md).

### Surface générique

Disponible sur tout `Collection<T>`, quel que soit `T` :

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9]);

nums.first();              // Some(&3)
nums.last();               // Some(&9)
nums.len();                // 6
nums.is_empty();           // false
nums.contains(&4);         // true
// Les closures de prédicat reçoivent `&&T` - notez le double déréférencement `**n` :
nums.first_where(|n| **n > 3);    // Some(&4)
nums.contains_where(|n| **n > 8); // true
// Pour un compte, exécutez le prédicat en ligne : `nums.iter().filter(|n| **n > 2).count()` - 4
```

Les transformations consomment `self` et retournent un nouveau
`Collection` :

```rust
let doubled: Collection<i32> = nums.clone().map(|n| n * 2);
let evens:   Collection<i32> = nums.clone().filter(|n| n % 2 == 0);
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(2); // [[3,1],[4,1],[5,9]]
let unique:  Collection<i32> = nums.clone().unique();
let sorted:  Collection<i32> = nums.clone().sort();
```

### Méthodes conscientes du modèle sur `Collection<M>`

Quand `T` est un modèle, des méthodes supplémentaires à clé de
chaîne passent par l'accesseur `field_value(name)` émis par la
macro :

```rust
let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let by_role: HashMap<String, Vec<User>> =
    users.clone().group_by::<String>("role");
let active: Collection<User> = users.clone().where_eq("active", true);

let total: f64 = users.clone().sum::<f64>("balance");
let avg:   f64 = users.clone().avg::<f64>("balance");
let max:   Option<i64> = users.clone().max::<i64>("login_count");
```

Le `pluck_by` à base de closure est l'alternative typée - utile
quand le nom de champ exigerait sinon une recherche par chaîne que
le système de types ne peut pas vérifier :

```rust
let names: Collection<String> = users.pluck_by(|u| u.name.clone());
```

`field_value(name)` par ligne retourne `Option<serde_json::Value>` -
`None` quand le nom de colonne ne correspond à aucun champ déclaré.
Les casts personnalisés qui échouent à sérialiser remontent aussi en
`None`. Les méthodes à clé de chaîne sautent silencieusement ces
lignes ; la forme par closure court-circuite dans le corps de la
closure pour que l'appelant décide.

### Flux via `LazyCollection`

Pour des jeux de données trop grands pour être matérialisés,
`Builder::lazy()` / `lazy_by_id(n)` / `cursor()` retournent un
`LazyCollection<M>` - un wrapper `Stream` qui récupère les lignes
par lots via curseur PK. Voir
[Itération par chunk et en mode lazy](#itération-par-chunk-et-en-mode-lazy).

### Chargement hâtif sur une collection

`Collection::load(["posts"])` / `load_missing(["posts"])`
exécutent le même dispatch de chargement hâtif qu'émet une chaîne
`Builder::with(...)`, mais contre une collection existante.
`load_missing` est par ligne : chaque ligne de la collection est
partitionnée en compartiments « a besoin d'être chargée » / « déjà
chargée », et seules celles qui manquent reçoivent le chargement en
masse. Voir [Chargement hâtif](#chargement-hâtif).

## Affectation en masse

### Liste blanche `fillable`

```rust
#[model(
    table = "users",
    fillable = ["name", "email"],
)]
pub struct User { /* ... */ }

User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    admin: true,    // abandonné silencieusement à l'exécution - pas dans fillable
}).await?;
```

### Liste noire `guarded`

`guarded` est l'inverse - chaque champ est fillable SAUF ceux qui
sont guarded. Mutuellement exclusif avec `fillable` ; utiliser les
deux à la fois est une erreur de compilation venant de la macro.

```rust
#[model(
    table = "posts",
    guarded = ["id", "user_id"],   // tout le reste est fillable
)]
pub struct Post { /* ... */ }
```

### Politique par défaut

Quand ni `fillable` ni `guarded` n'est défini, la politique par
défaut est `guarded = ["id"]` (ou ce que `primary_key = "..."`
résout) - chaque champ est fillable sauf la clé primaire. Ceci
correspond au défaut de Laravel « tous les champs fillable sauf la
PK ».

### L'échappatoire `unguarded(closure)`

`unguarded(closure)` désactive le filtre pour un bloc :

```rust
use suprnova::eloquent::unguarded;

// Contourne le filtre pour un script de migration de données ponctuel :
unguarded(|| async {
    User::create(attrs! {
        name: "Bootstrap",
        email: "boot@example.com",
        admin: true,    // assignable à l'intérieur de la closure
    }).await
}).await?;
```

Implémentation : un booléen `tokio::task_local!` que le filtre
`Fillable::apply` vérifie avant de s'exécuter. Task-local veut dire
que les requêtes concurrentes ne sont pas affectées par la portée
`unguarded` d'une autre tâche.

## Casts

Les casts s'exécutent à la frontière entre le stockage (valeur de
colonne) et le runtime (champ de modèle). Chaque type de cast
implémente le trait `Cast`. Les casts intégrés couvrent l'ensemble
complet de Laravel ; les utilisateurs enregistrent des casts
personnalisés via le trait. Cette section est l'index de référence
rapide ; le contrat complet par cast - primitif, temporel, structuré,
enum, chiffré, haché, plus la macro de redéfinition à l'exécution
`casts!` - vit dans
[Eloquent - Casts, accesseurs et mutateurs](eloquent-mutators.md).

### Explicite uniquement

Les casts se déclarent dans `#[model(casts = { ... })]` - il n'y a
pas de détection automatique à partir des types de champ. Un champ
`prefs: Json` ne devient pas implicitement `AsJson` ; vous écrivez
`casts = { prefs = AsJson }`. Raison d'être : vous devriez pouvoir
lire le modèle et savoir exactement ce qui s'exécute aux frontières
de stockage. Pas de magie.

### Exemple

```rust
use suprnova::{model, AsArray, AsBool, AsCollection, AsDate, AsDateTime,
    AsEncrypted, AsEnum, AsObject, AsTimestamp};

#[model(
    table = "users",
    casts = {
        active        = AsBool,
        preferences   = AsArray<String>,
        options       = AsObject<UserOptions>,
        profile       = AsCollection<ProfileField>,
        birthday      = AsDate,
        last_seen_at  = AsDateTime,
        role          = AsEnum<UserRole>,
        api_token     = AsEncrypted,
    },
)]
pub struct User { /* ... */ }
```

### Liste complète des casts Laravel et correspondance Suprnova

| Cast Laravel | Cast Suprnova | Type runtime |
|--------------|---------------|--------------|
| `bool`, `boolean` | `AsBool` | `bool` |
| `int`, `integer` | `AsInt<I>` | `I: PrimInt` |
| `float`, `double`, `real` | `AsFloat` | `f64` |
| `decimal:N` | `AsDecimal<N>` | `rust_decimal::Decimal` |
| `string` | `AsString` | `String` |
| `array` | `AsArray<T>` | `Vec<T>` (encodé en JSON) |
| `object` | `AsObject<T>` | `T: Serialize + DeserializeOwned` |
| `collection` | `AsCollection<T>` | `Collection<T>` |
| `json` | `AsJson<T>` | `T` (colonne JSON brute) |
| `date`, `date:format` | `AsDate` | `chrono::NaiveDate` |
| `datetime`, `datetime:format` | `AsDateTime` | `chrono::DateTime<Utc>` |
| `immutable_date` | `AsImmutableDate` | `chrono::NaiveDate` |
| `immutable_datetime` | `AsImmutableDateTime` | `chrono::DateTime<Utc>` |
| `timestamp` | `AsTimestamp` | `i64` (epoch unix) |
| `encrypted` | `AsEncrypted` | `String` (chiffré via `Crypt`) |
| `encrypted:array` | `AsEncryptedArray<T>` | `Vec<T>` (JSON + chiffré) |
| `encrypted:object` | `AsEncryptedObject<T>` | `T` (JSON + chiffré) |
| `encrypted:collection` | `AsEncryptedCollection<T>` | `Collection<T>` |
| `EnumClass::class` | `AsEnum<E>` | `E: EnumString + AsRefStr` |
| `AsArrayObject::class` | `AsArrayObject<T>` | `IndexMap<String, T>` |
| `hashed` | `AsHashed` | `String` (`Hash::make` à l'écriture ; ne déchiffre jamais) |

22 casts au total. La plupart correspondent un-à-un avec Laravel ;
l'`AsOptionalDateTime` (utilisé par `soft_deletes`) est auto-injecté
par la macro quand la colonne de suppression logicielle est
`Option<DateTime<Utc>>`.

### Modes d'échec des casts chiffrés

Les quatre casts `AsEncrypted*` routent chaque chiffrement /
déchiffrement à travers la façade `Crypt` (indexée par `APP_KEY`).
Quand le déchiffrement échoue - mauvaise clé, texte chiffré tronqué,
octets falsifiés, tag AEAD qui ne correspond pas - le cast fait
remonter un `FrameworkError::Internal` clair depuis
`Cast::from_storage`. Il n'y a pas de repli silencieux vers une
valeur aberrante :

- Charger une ligne via `Model::find` / `Model::query()` propage
  l'erreur de déchiffrement et (via le `From<inner::Model>` généré
  par la macro) panique avec `cast from_storage failed - corrupt
  data in database column`. Les opérateurs voient l'échec dans les
  logs immédiatement ; le modèle ne porte jamais un texte en clair
  plausible-mais-faux.
- Le cast `AsHashed` est à sens unique ; il ne déchiffre jamais,
  donc ce mode d'échec ne s'applique pas.

Ceci correspond au cast `encrypted` de Laravel : un mauvais
`APP_KEY` contre une colonne chiffrée existante est une erreur dure,
jamais un `null`/une chaîne vide silencieux.

### Rotation d'`APP_KEY`

Suprnova prend en charge une rotation de clé sans interruption de
service via un *trousseau* de clés : l'`APP_KEY` courante chiffre ;
une variable d'env optionnelle `APP_KEY_PREVIOUS` (séparée par des
virgules, de la plus ancienne à la plus récente) fournit des replis
de déchiffrement pour les données écrites sous d'anciennes clés. Le
chiffrement utilise *toujours* la clé courante - les clés précédentes
ne participent qu'au déchiffrement.

Chaque déchiffrement qui retombe sur une clé précédente émet une
ligne `tracing::warn!` contenant l'index de la clé précédente. La
charge du log exclut délibérément le texte en clair et le texte
chiffré ; seul le fait-même de la rotation, plus une piste d'action
de rechiffrement, y voyage.

**Procédure de rotation** (sans interruption de service, sûre en
production) :

1. Produisez une nouvelle clé : `suprnova key:generate` (écrit sur
   stdout).
2. Déplacez l'ancienne clé vers `APP_KEY_PREVIOUS` et positionnez
   `APP_KEY` sur la nouvelle valeur :
   ```
   APP_KEY_PREVIOUS=<old_key>
   APP_KEY=<new_key>
   ```
3. Déployez. Les nouvelles écritures utilisent la nouvelle clé ; les
   lignes existantes continuent de se déchiffrer via le repli sur la
   clé précédente. Les avertissements dans les logs identifient les
   colonnes qui dépendent encore de `APP_KEY_PREVIOUS`.
4. Lancez une passe de rechiffrement. Pour chaque modèle avec des
   casts chiffrés :
   ```rust
   for chunk in User::query().chunk(500).await? {
       for user in chunk {
           // Touch + save réécrit chaque colonne castée sous la
           // clé courante. `Cast::to_storage` va toujours chercher
           // l'entrée courante du trousseau.
           user.save().await?;
       }
   }
   ```
   Ceci est idempotent - les lignes déjà sur la nouvelle clé sont
   simplement sans effet.
5. Une fois que les logs ne montrent plus d'avertissements
   `APP_KEY_PREVIOUS` (laissez une marge généreuse pour le lot et
   pour toute donnée supprimée-logiciellement / archivée), retirez
   `APP_KEY_PREVIOUS` de l'environnement et redéployez.

**Rotation en plusieurs étapes.** Si vous tournez à nouveau avant
d'avoir terminé la passe précédente, ajoutez :
`APP_KEY_PREVIOUS=<oldest>,<previous>`. Le trousseau essaie chaque
clé précédente dans l'ordre. La liste est plafonnée à 8 entrées - une
chaîne réaliste compte 1 à 3 entrées (une rotation en cours,
peut-être une rotation précédente restée bloquée) et une liste plus
longue est presque toujours un accident de templating de config ;
dépasser le plafond fait échouer le démarrage avec un diagnostic
exploitable plutôt que de silencieusement laisser tomber une clé
dont l'opérateur pourrait encore dépendre.

**Contraintes.**

- Une entrée malformée dans `APP_KEY_PREVIOUS` fait échouer le
  démarrage explicitement (comme un `APP_KEY` malformé) - un secret
  à moitié tourné ne devrait jamais se dégrader silencieusement.
- Plus de 8 entrées dans `APP_KEY_PREVIOUS` fait échouer le
  démarrage explicitement - voir
  [`suprnova::crypto::MAX_PREVIOUS_KEYS`].
- Les entrées vides dans la liste (p. ex. des virgules finales
  venant d'une config templatée) sont tolérées comme « pas de clé
  dans cet emplacement » - pas une erreur.
- Le format réseau est inchangé par rapport à la disposition à clé
  unique d'avant la rotation : aucun identifiant de clé n'est
  intégré dans le texte chiffré. Le trousseau essaie chaque clé par
  déchiffrement d'essai, dans l'ordre, jusqu'à ce que l'une réussisse.

### Redéfinition de cast à l'exécution - `with_casts`

```rust
let users = User::query()
    .with_casts(suprnova::casts! { birthdate = AsDateTime })
    .get()
    .await?;
```

`with_casts` redéfinit les casts déclarés du modèle pour la durée
d'une seule requête - utile quand une colonne brute revient d'une
jointure / vue / `select_raw` et a besoin d'une coercition de type
différente du défaut du modèle.

### Casts personnalisés

Les casts personnalisés implémentent `Cast` :

```rust
use suprnova::eloquent::casts::Cast;
use suprnova::FrameworkError;

pub struct AsAesGcmJson<T>(std::marker::PhantomData<T>);

impl<T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync> Cast
    for AsAesGcmJson<T>
{
    type Runtime = T;
    type Storage = String;
    fn to_storage(value: &T) -> Result<String, FrameworkError> { /* ... */ }
    fn from_storage(stored: &String) -> Result<T, FrameworkError> { /* ... */ }
}

#[model(casts = { secret = AsAesGcmJson<SecretBundle> })]
pub struct Vault { /* ... */ }
```

Le trait `Cast` est livré aux côtés des casts primitifs. Les casts
personnalisés peuvent utiliser soit un stockage `String` (quand ils
encodent en JSON), soit n'importe quel type scalaire pris en charge
par SeaORM (`i64`, `f64`, `bool`, `Vec<u8>`).

## Accesseurs et mutateurs

### Accesseurs

```rust
#[model(
    table = "users",
    appends = ["full_name"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    // ...
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }
}
```

Quand `user.to_array()` s'exécute (ou `user.to_json()`, qui lui
délègue), l'accesseur `full_name` est appelé et sa valeur de retour
est insérée dans la sortie JSON. Appeler `user.full_name()` depuis
Rust est un simple appel de méthode ordinaire.

### Mutateurs

Les mutateurs s'exécutent avant le stockage :

```rust
#[model(
    table = "users",
    fillable = ["first_name", "last_name", "password"],
    mutators = ["password"],
)]
pub struct User { /* ... */ }

impl User {
    #[mutator]
    pub fn set_password(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), suprnova::FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            suprnova::FrameworkError::validation("password", format!("{e}"))
        })?;
        self.password = hash::make(&raw);
        Ok(())
    }
}
```

Appeler `user.password = "secret".into()` affecte directement la
valeur brute sans exécuter le mutateur. Pour exécuter le chemin
mutateur, appelez `user.set_password(json!("secret"))` ou utilisez le
chemin JSON (`user.fill(attrs!{password: "secret"})`), qui passe
automatiquement par le mutateur parce que `"password"` est listé dans
`mutators = [...]`.

### Comment le routage fonctionne

- **La sérialisation (`to_array` → `Value`, `to_json` →
  `String`)** exécute les accesseurs. Chaque nom de champ listé dans
  `appends = [...]` devient un appel à `self.<name>()` ; la valeur de
  retour est insérée dans la sortie JSON. `to_json()` est un wrapper
  mince : `serde_json::to_string(&self.to_array())`.
- **Les écritures façon fill (`fill`, `create`, `update`)** passent
  par les mutateurs. Chaque nom de champ listé dans
  `mutators = [...]` devient un appel à `self.set_<field>(value)` au
  lieu d'une affectation directe.

Les macros au niveau fonction `#[accessor]` et `#[mutator]` émettent
des entrées de registre que les chemins de sérialisation / fill de
la macro parcourent.

### Les valeurs malformées sont des erreurs, pas des défauts

Une valeur qui ne peut pas se décoder vers le type de son champ fait
échouer l'écriture et nomme le champ :

```rust
let err = user.fill(attrs! { age: "not a number" }).unwrap_err();
// ValidationError { field: "age", message: "could not decode the
// supplied value: invalid type: string \"not a number\", expected i32" }
```

Le modèle reste intact - un `fill` rejeté n'applique rien.

Deux cas voisins se comportent différemment, à dessein :

- Une **colonne inconnue** est quand même sautée silencieusement,
  comme le `$model->fill()` de Laravel. Ne pas connaître une colonne
  n'est pas la même chose que recevoir une valeur cassée pour une
  colonne que vous connaissez.
- Une colonne exclue par `fillable` / `guarded` est abandonnée par
  le filtre d'affectation en masse *avant* le décodage, si bien
  qu'une valeur malformée pour un champ que l'appelant n'a pas le
  droit de positionner est aussi silencieuse. Faire échouer ici
  révélerait à un appelant non autorisé quelles colonnes existent.

L'élargissement numérique n'est pas une erreur de type : un entier
JSON se décode normalement vers un champ `f64`.

> Avant la v0.8.0, une valeur malformée était silencieusement
> remplacée par le `Default` du champ et l'appel retournait `Ok` -
> `fill(attrs!{ age: "abc" })` positionnait `age = 0` et rapportait
> un succès. Si vous comptiez sur cette coercition, validez ou
> convertissez avant d'appeler `fill`.

### Hidden / visible

```rust
#[model(
    table = "users",
    hidden = ["password", "remember_token"],
)]
pub struct User { /* ... */ }
```

`hidden = [...]` est une liste noire - chaque colonne sauf celles
listées se sérialise. `visible = [...]` est la forme inclusive -
seules celles listées se sérialisent. Mutuellement exclusifs à la
compilation.

## Timestamps

Quand les colonnes `created_at` et `updated_at` existent toutes les
deux, la macro les détecte automatiquement et active le suivi des
timestamps :

- `created_at` est positionné à `Utc::now()` lors du `save()` pour
  les nouvelles lignes.
- `updated_at` est positionné à `Utc::now()` à chaque `save()`.

La détection automatique est conservatrice : si la struct n'a
qu'une seule des deux colonnes, la macro échoue pour qu'une coquille
(`craeted_at`) ne désactive pas silencieusement les timestamps.
Positionnez `timestamps = false` pour désactiver entièrement.

### Désactiver les timestamps automatiques

```rust
#[model(table = "audit_logs", timestamps = false)]
pub struct AuditLog {
    pub id: i64,
    pub event: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    // Pas de champ updated_at - mais timestamps = false fait aussi
    // taire l'erreur `only one column found` de la macro.
}
```

### `touch()` - avance updated_at sans autre changement

```rust
user.touch().await?;
```

`touch()` émet `UPDATE table SET updated_at = ? WHERE pk = ?` -
atomique, sans lecture-modification-écriture. La macro émet un impl
`Touchable` sur chaque modèle horodaté.

### Toucher le parent

```rust
#[model(
    table = "comments",
    touches = ["post"],
    relations = {
        post: BelongsTo<Post> { fk = "post_id" },
    },
)]
pub struct Comment {
    pub id: i64,
    pub post_id: i64,
    // ...
}
```

Après la création, la sauvegarde, la mise à jour ou la suppression d'un
commentaire, le `updated_at` de son post est avancé  -  un
`UPDATE posts SET updated_at = ? WHERE id = ?`, sans SELECT. C'est ce dont a
besoin une clé de cache suspendue à `post.updated_at` pour rester exacte
lorsqu'un seul enfant a changé.

Chaque nom de `touches` doit être une relation `BelongsTo` déclarée dans le
même bloc `relations = { ... }`. Un nom qui ne se résout pas, ou qui se
résout vers un autre type de relation, est une erreur de compilation plutôt
qu'une surprise lors de la première sauvegarde. Les propriétaires polymorphes
(`MorphTo`) ne peuvent pas encore être mis à jour.

Un propriétaire dont le modèle a `timestamps = false` est **ignoré** : aucune
erreur, aucune écriture, et la sauvegarde de l'enfant retourne toujours
`Ok`. Même chose pour un propriétaire atteint par une clé étrangère `NULL`,
et pour un propriétaire supprimé logiquement.

La mise à jour s'exécute sur le même exécuteur que l'écriture qui l'a
déclenchée ; elle rejoint donc cette transaction dans une closure
`DB::transaction`, et un rollback l'annule.

### Pourquoi Suprnova diverge

Le `touchOwners` de Laravel charge chaque modèle parent et se propage
récursivement ; ainsi, la sauvegarde d'un commentaire avance aussi les
propriétaires propres du post et déclenche l'événement `saved` de chaque parent.
Suprnova résout le parent via le registre de relations et écrit directement la
colonne  -  une instruction par relation touchée, sans hydratation. La cascade
n'a donc qu'un niveau de profondeur et ne déclenche aucun événement parent.
C'est le compromis pour une sauvegarde qui n'émet pas un SELECT par relation
touchée. Utilisez un observateur lorsque vous avez besoin d'avancer le
grand-parent ou de l'événement.

`restore()` sur un enfant supprimé logiquement ne touche pas ses propriétaires.
La restauration de Laravel passe par `save` ; celle de Suprnova est un
`UPDATE deleted_at = NULL` direct.

### Format

Toujours ISO 8601 en UTC. Pas de redéfinition
`Model::$timestampsFormat` (selon le tableau des divergences avec
Eloquent - l'interopérabilité frontend prime ; le formatage de
locale appartient à la couche i18n).

## Observateurs et événements de cycle de vie

Chaque modèle traverse un cycle de vie fixe de 16 événements en
passant par les chemins `create` / `save` / `update` / `delete` /
`restore` / `replicate` / requête Builder. Les écouteurs peuvent
s'accrocher à chaque événement pour journaliser, auditer, produire
un effet de bord, valider, ou annuler l'opération en cours.

### Les 16 événements de cycle de vie

Les événements se divisent en deux groupes selon leur annulabilité :

**Annulables (5)** - se déclenchent AVANT l'écriture en base de
données. Un écouteur qui retourne `EventResult::cancel("reason")`
avorte l'opération avec `FrameworkError::bad_request(reason)`.

| Événement   | Quand                                     | Payload                                                 |
|-------------|-------------------------------------------|---------------------------------------------------------|
| `Saving`    | Avant `create` et `save`                  | `Arc<Mutex<Attrs>>` + `is_creating: bool`               |
| `Creating`  | Avant `create`                            | `Arc<Mutex<Attrs>>`                                     |
| `Updating`  | Avant `save` / `update` sur une ligne existante | Instantané du modèle avant mise à jour + `Arc<Mutex<Attrs>>` |
| `Deleting`  | Avant `delete` (logicielle ou forcée)     | Modèle + `is_force: bool` (force-delete sur suppression logicielle) |
| `Restoring` | Avant `restore` sur un modèle à suppression logicielle | Modèle                                    |

**Non annulables (11)** - se déclenchent APRÈS l'opération. Les
erreurs d'écouteur se propagent mais ne peuvent pas arrêter une
écriture déjà posée.

| Événement       | Quand                                              | Payload                          |
|-----------------|-----------------------------------------------------|----------------------------------|
| `Retrieving`    | Une fois par requête Builder, avant l'appel BD      | Aucun                            |
| `Retrieved`     | Une fois par ligne retournée par une requête Builder | Modèle                          |
| `Created`       | Après un `create` réussi                            | Modèle                            |
| `Updated`       | Après un `save` / `update` réussi                   | Instantanés précédent + actuel   |
| `Saved`         | Après `create` et `save`                            | Modèle                            |
| `Deleted`       | Après un `delete` réussi                            | Modèle + `is_force: bool`         |
| `Trashed`       | Après suppression logicielle (PAS force-delete)     | Modèle                            |
| `Restored`      | Après un `restore` réussi                           | Modèle                            |
| `Replicating`   | Pendant `replicate` / `replicate_except`, avant le retour (PAS `replicate_into` - par type source) | Source + `Arc<Mutex<replica>>` (mutable) |
| `ForceDeleting` | Avant `force_delete` sur un modèle à suppression logicielle | Modèle                   |
| `ForceDeleted`  | Après un `force_delete` réussi                      | Modèle                            |

La séparation annulable / non annulable reflète la paire de hooks
`creating` / `created` de Laravel. `Saving` se déclenche à la fois
pour l'insertion et la mise à jour - redéfinissez celui-là quand le
comportement est identique sur les deux chemins, et discriminez via
`is_creating`.

`Replicating` est l'unique hook non annulable qui remet une
référence mutable (la réplique est un `Arc<Mutex<M>>`). Utilisez-le
pour effacer les timestamps, régénérer des UUID, réinitialiser des
auto-incréments, etc. avant que le clone ne soit retourné à
l'appelant.

### Observateurs vs écouteurs bruts

Deux façons de s'accrocher aux événements de cycle de vie :

1. **Écouteurs bruts** - appelez
   `EventFacade::listen::<Created, _>(Arc::new(MyListener))` pour
   chaque événement voulu, un impl par événement. C'est le
   mécanisme sous-jacent ; les observateurs roulent par-dessus.

2. **Observateurs** - regroupent les 16 hooks sous un seul trait. La
   macro voit quelles méthodes l'utilisateur a redéfinies et
   n'enregistre que celles-là. C'est le chemin recommandé pour tout
   ensemble non trivial de hooks.

```rust
use async_trait::async_trait;
use suprnova::eloquent::attrs::Attrs;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::observers::Observer;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]   // <- DOIT précéder #[async_trait]
#[async_trait]
impl Observer<User> for AuditObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if attrs.get("email").is_none() {
            return EventResult::cancel("email is required");
        }
        EventResult::ok()
    }

    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user.id = user.id, "user created");
        Ok(())
    }
}
```

Chaque méthode du trait a un défaut sans effet, si bien que le bloc
impl ne contient que les événements qui vous intéressent. La macro
identifie les redéfinitions par correspondance de nom contre
l'ensemble fermé des 16 méthodes ; les méthodes que vous ne
redéfinissez pas n'enregistrent aucun écouteur.

### Ordre requis des attributs

`#[suprnova::observer(M)]` DOIT apparaître AU-DESSUS de
`#[async_trait]` :

```rust
#[suprnova::observer(User)]   // externe - s'exécute en premier, voit les fn async brutes
#[async_trait]                // interne - réécrit les signatures des fn async
impl Observer<User> for AuditObserver { /* ... */ }
```

Les macros d'attribut se développent de l'extérieur vers
l'intérieur. `async_trait` réécrit chaque `async fn` vers une forme
de fn-poll `Pin<Box<dyn Future>>` sans sucre syntaxique ; si
`#[async_trait]` s'exécutait en premier, la correspondance de nom de
la macro observer contre les 16 noms de méthode du trait ne
trouverait rien et émettrait silencieusement zéro écouteur.

### Quatre chemins d'enregistrement

| Chemin                                       | Quand l'utiliser                                    |
|----------------------------------------------|-----------------------------------------------------|
| `#[suprnova::observer(M)]` (inventaire)       | Observateur statique connu à la compilation. S'installe automatiquement au démarrage. |
| `#[model(observers = [Foo, Bar])]`           | Documentation + validation à la compilation que les types listés se résolvent. Ne s'enregistre PAS lui-même. |
| `Model::observe(MyObs).await`                | Enregistrement à l'exécution. Piloté à la main ; utile quand l'enregistrement dépend de la config. |
| `EventFacade::listen::<events::Created, _>(...)` | Niveau le plus bas - un événement à la fois. À utiliser quand un observateur semble trop lourd. |

L'attribut `observers = [...]` sur `#[model]` est un marqueur de
documentation. Il compile vers un bloc `const _: fn() = || { let _ =
::std::any::type_name::<T>; ... };` qui prouve que chaque type listé
se résout vers un vrai item Rust ; les coquilles remontent au site
de déclaration du modèle. L'installation réelle passe par la voie de
l'inventaire - l'attribut `#[observer(M)]` sur `Foo` est ce qui
enrôle `Foo` pour l'installation automatique.

### Amorçage

Appelez `bootstrap_observers()` une fois au démarrage pour vider
l'inventaire et installer chaque observateur enregistré via
`#[observer(M)]` :

```rust
suprnova::eloquent::observers::bootstrap_observers().await?;
```

La vidange est idempotente pour la voie de l'inventaire - la
closure d'installation de chaque observateur est verrouillée par un
`AtomicBool` par type (émission de macro de T2b), si bien qu'appeler
`bootstrap_observers()` deux fois ne double pas l'enregistrement.

Le shim à l'exécution `Model::observe(MyObs)` N'est PAS verrouillé.
L'appeler deux fois enregistre deux ensembles d'écouteurs, ce qui
correspond à la sémantique manuelle du `Model::observe(MyObs::class)`
de Laravel. Si un observateur piloté à la main porte aussi
`#[observer]`, l'adaptateur d'inventaire se déclenche en plus de
ceux installés manuellement.

### Annuler depuis un observateur

Les cinq hooks annulables retournent `EventResult`. Pour avorter
l'opération, retournez `EventResult::cancel("reason")` :

```rust
#[suprnova::observer(Subscription)]
#[async_trait]
impl Observer<Subscription> for PolicyObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if let Some(plan) = attrs.get("plan") {
            if plan == "blocked" {
                return EventResult::cancel("plan is blocked");
            }
        }
        EventResult::ok()
    }
}
```

La raison d'annulation remonte comme
`FrameworkError::bad_request(reason)` depuis `Subscription::create`.
La ligne n'atterrit jamais en base de données - l'annulation est un
véritable avortement, pas une « suppression après coup ».

Plusieurs observateurs peuvent enregistrer des hooks annulables sur
le même modèle ; que l'un d'entre eux retourne `Cancel` arrête
l'opération. L'ordre est celui de l'enrôlement dans l'inventaire
(l'ordre de link en pratique).

### Plusieurs observateurs sur un même modèle

Plusieurs impls `Observer<M>` se déclenchent toutes pour le même
événement - le dispatch d'EventFacade fait du fan-out vers chaque
écouteur enregistré plutôt que d'en choisir un seul :

```rust
#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for AuditObserver { /* ... */ }

#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for NotifyObserver { /* ... */ }

// Comment::create(...) déclenche à la fois AuditObserver::created ET NotifyObserver::created.
```

Ceci correspond à la sémantique de fan-out de Laravel et c'est la
propriété porteuse derrière le motif « décomposer les hooks par
préoccupation » : un `AuditObserver` ne connaît que l'audit, un
`NotifyObserver` ne connaît que les notifications, et la déclaration
du modèle ne se soucie pas du nombre d'observateurs qui s'attachent.

### `Model::observe()` manuel

Chaque struct `#[suprnova::model]` reçoit un shim `observe<O>()` par
modèle. Appelez-le au démarrage pour un enregistrement dynamique :

```rust
#[derive(Clone)]
struct MyObs;

#[async_trait]
impl Observer<User> for MyObs { /* ... */ }

// À l'exécution :
User::observe(MyObs).await;
```

La borne `O: Clone + 'static` du shim est ce qui permet au
framework de remettre un clone neuf de l'observateur à chacun des
16 écouteurs adaptateurs internes. Les 16 adaptateurs d'écouteur
s'installent à chaque appel - les défauts du trait font des méthodes
non redéfinies des no-op bon marché.

### Contraintes

- **La version macro exige que le bloc impl utilise les noms de
  méthode bruts correspondant aux 16 hooks du trait.** Les méthodes
  renommées, les défauts supprimés par `#[allow]`, et les corps
  verrouillés par `#[cfg]` tombent hors de la correspondance de nom
  et n'enregistrent aucun écouteur.

- **Les structs observateur que la macro inspecte doivent être de
  taille nulle** (aucun champ) en v1. La macro construit
  l'observateur via `let obs = MyObserver;` à l'intérieur de chaque
  adaptateur. Les observateurs à état (portant un `Arc<Inner>`) ont
  besoin du chemin `Model::observe()` à l'exécution, qui prend
  l'observateur par valeur et le clone dans chaque adaptateur.

- **Isolation de test : utilisez des types de modèle uniques par
  scénario.** L'EventDispatcher global au processus veut dire que
  les écouteurs installés pour `User` sont visibles par chaque test
  du même binaire. Des types de modèle uniques par test
  (`T2Comment`, `T2Subscription`, …) tiennent la contamination
  inter-test à l'écart des assertions de compteur. Les tests
  d'intégration `eloquent_observers.rs` exercent ce motif.

## Prunable

Laravel livre un trait `Prunable` qui laisse un modèle déclarer un
scope de lignes à supprimer sur un calendrier. Suprnova reflète cela
avec deux traits et une commande console.

### Déclarer un élagueur

```rust
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::eloquent::Prunable;

#[suprnova::prunable]
#[async_trait]
impl Prunable for ExpiredSession {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "expires_at",
            "<",
            (Utc::now() - Duration::days(30)).to_rfc3339(),
        )
    }
}
```

### `MassPrunable` - variante à suppression en masse

Pour les tables à haut volume (logs d'audit, logs de requêtes,
entrées de cache expirées), `MassPrunable` saute les événements par
ligne et exécute une unique instruction `DELETE WHERE …` :

```rust
use suprnova::eloquent::MassPrunable;

#[suprnova::prunable]
#[async_trait]
impl MassPrunable for AuditLog {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "created_at",
            "<",
            (Utc::now() - Duration::days(365)).to_rfc3339(),
        )
    }
}
```

### Déclencher l'élagage

S'exécute via la console par projet (pour laquelle
`app/cmd/main.rs` appelle `suprnova::console::dispatch_argv`, après
`db:seed` et les autres commandes intégrées) :

```bash
suprnova model:prune                          # élague chaque type enregistré
suprnova model:prune --model=ExpiredSession   # filtre vers un seul modèle
suprnova model:prune --pretend                # exécution à blanc ; journalise ce qui serait supprimé
```

De façon programmatique, les runners se trouvent à
`suprnova::eloquent::{prune_all, prune_all_dry, prune_one}`.

### Hook d'élagage

`Prunable::pruning(&self)` se déclenche avant chaque suppression de
ligne pour que l'utilisateur puisse exécuter des effets de bord
(nettoyer des fichiers associés, faire du fan-out d'événements,
etc.). L'impl par défaut est vide. `MassPrunable` saute ce hook par
définition - les suppressions en masse n'énumèrent pas les lignes.

### Comportement de cascade

**L'élagage NE se propage PAS automatiquement en cascade vers les
lignes liées.** Un impl `Prunable` ou `MassPrunable` sur `User`
supprime les lignes user ; leurs `posts`, entrées pivot `role_user`,
`comments` polymorphes, etc. sont LAISSÉS ORPHELINS avec des colonnes
FK qui pointent vers l'utilisateur désormais supprimé.

Ceci correspond au contrat de Laravel : le nettoyage des relations
est la responsabilité de l'utilisateur. Deux façons propres de le
gérer :

1. **Cascade FK au niveau base de données** - déclarez
   `ON DELETE CASCADE` (ou `ON DELETE SET NULL`) dans la contrainte
   de clé étrangère quand vous écrivez la migration. Le moteur de BD
   gère la cascade gratuitement, sans code Rust par ligne.

2. **Hook par ligne** - implémentez `Prunable::pruning(&self)` pour
   supprimer les enfants avant que la ligne parente ne soit
   abandonnée. Le hook se déclenche à l'intérieur de la même
   opération logique que la suppression du parent, si bien qu'un
   ordre cohérent est garanti :

   ```rust
   #[async_trait]
   impl Prunable for User {
       fn prunable() -> Builder<Self> {
           Self::query().filter_op("deleted_at", "<", thirty_days_ago())
       }

       async fn pruning(&self) -> Result<(), FrameworkError> {
           // Supprime les posts.
           Post::query().filter("user_id", self.id).get().await?
               .into_iter()
               .map(|p| p.delete());
           // Détache les pivots de rôle.
           self.roles().sync(Vec::<i64>::new()).await?;
           Ok(())
       }
   }
   ```

`MassPrunable` est basé sur un ensemble - `pruning()` ne se
déclenche pas. Utilisez `Prunable` simple chaque fois que vous avez
besoin de cascade. Le framework n'émettra pas silencieusement un
DELETE par ligne quand vous optez pour `MassPrunable` ; le compromis
est documenté explicitement.

### Mécanisme de registre

L'enregistrement des élagueurs utilise le même motif d'inventaire
que les observateurs, les commandes, et les superviseurs. L'attribut
`#[suprnova::prunable]` sur le bloc `impl Prunable for T { ... }`
s'auto-enregistre via `inventory::submit!` à la compilation. Aucun
fichier de config central ; ajouter un nouveau type élagable ne
prend qu'un seul attribut.

## Routage multi-connexion

Les applications en production ont régulièrement besoin de plus
d'une connexion à la base de données - le cas canonique est une
réplique de lecture pour l'analytique plus la primaire pour les
écritures, mais la surface se généralise à toute connexion nommée
(BD de reporting, BD d'archive, shard par tenant).

### Enregistrer une connexion

Appelez `DB::register_named(name, config)` au démarrage pour chaque
connexion non-défaut à laquelle votre application parle :

```rust
DB::register_named(
    "reporting",
    DatabaseConfig {
        url: env::var("REPORTING_DATABASE_URL")?,
        max_connections: Some(20),
        ..Default::default()
    },
).await?;
```

Deux noms sont réservés : `__primary__` court-circuite le registre
vers `DB::connection()`, et `__read_replica__` fait entrer la
connexion dans le routage automatique à séparation
lecture-écriture - voir plus bas.

### Opt-in par requête : `Model::on(name)`

`Model::on("reporting")` retourne un `Builder<M>` pré-réglé pour
router à travers la connexion nommée :

```rust
let totals = Order::on("reporting")
    .order_by_desc("total")
    .limit(100)
    .get()
    .await?;
```

`on(...)` est cantonné à la requête - il n'affecte que le builder
chaîné. L'appel `Order::query()` simple suivant se résout via le
défaut.

### Défaut par modèle : `#[model(connection = "...")]`

Quand un modèle vit toujours sur une seule connexion, déclarez le
défaut sur l'attribut :

```rust
#[model(table = "events", connection = "events_db")]
pub struct Event { /* ... */ }
```

Chaque appel `Event::query()` / `Event::create()` /
`Event::find()` route à travers `events_db` sans avoir besoin de la
redéfinition `.on(...)` par requête. Un `.on(...)` explicite sur un
builder l'emporte quand même.

### Séparation lecture-écriture

Enregistrer une connexion sous le nom réservé `__read_replica__`
fait entrer chaque modèle dans le routage automatique : les méthodes
de lecture (`first` / `get` / `find` / `count` / `paginate` /
`chunk` / les parcoureurs pilotés par closure) passent par la
réplique ; les écritures (`save` / `create` / `update` / `delete` /
`force_delete` / `replicate` / `attach` / `detach` / `sync` /
`increment` / `decrement`) passent par la primaire.

`Model::on_write_connection()` fait sortir un unique builder de la
réplique - utile quand la cohérence read-your-writes compte (p. ex.
immédiatement après un `save`, avant que la réplication ne
rattrape).

### Précédence de routage

La chaîne de dispatch fait passer chaque opération par
`ExecutorChoice::resolve_read` ou `resolve_write`. L'ordre est :

1. **Une transaction active l'emporte absolument.** À l'intérieur
   de `DB::transaction`, chaque lecture ET chaque écriture utilise
   la connexion de la tx. `on(name)` est IGNORÉ à l'intérieur d'une
   transaction - la tx est liée à une connexion physique
   spécifique. SeaORM ne peut pas démarrer une transaction sur une
   connexion et exécuter des instructions contre une autre.
2. **`on(name)` par builder.** Positionné via `Model::on(name)` /
   `Builder::on(name)`. L'emporte sur le défaut du modèle et sur la
   séparation lecture/écriture.
3. **`Model::on_write_connection()`.** Force la primaire même
   quand l'opération routerait sinon vers la réplique.
4. **Défaut par modèle `#[model(connection = "...")]`.** L'emporte
   sur la séparation lecture/écriture pour les propres requêtes du
   modèle.
5. **Séparation lecture/écriture.** Quand `__read_replica__` est
   enregistrée, les méthodes de lecture y routent ; les écritures
   routent vers la primaire.
6. **Défaut.** `DB::connection()` - la primaire, celle que
   `DB::init()` a mise en place.

### Mises en garde

- Les transactions actives IGNORENT `on(name)` (voir §1
  ci-dessus). Si vous avez besoin d'une écriture sur une connexion
  différente en cours de tx, vous ne pouvez pas - la tx est liée à
  une seule connexion.
- Les noms réservés `__primary__` et `__read_replica__` ne peuvent
  pas être utilisés comme noms de connexion utilisateur.
  `DB::register_named` retourne une erreur en cas de collision.
- Le lag de réplique est VOTRE problème. Suprnova ne fait pas de
  réessai à la lecture et ne retombe pas sur la primaire quand la
  réplique est périmée ; si vous avez besoin de read-your-writes
  après un save, utilisez `Model::on_write_connection()`
  explicitement.

## Réplication

`Model::replicate()` retourne une copie non sauvegardée du modèle
avec la clé primaire réinitialisée à son défaut. Utile pour une UX
« dupliquer cet enregistrement » où l'utilisateur veut partir d'une
ligne existante.

```rust
let template: User = User::find_or_fail(42).await?;
let mut copy = template.replicate().await?;  // id réinitialisé au défaut
copy.email = "fresh@example.com".into();
copy.save().await?;  // INSERT, pas UPDATE
```

`replicate` est **async** dans Suprnova (diverge de Laravel) parce
qu'il déclenche l'événement `Replicating` - les écouteurs `Saving` /
`Created` / etc. peuvent muter la réplique avant qu'elle ne soit
retournée. Voir [Événement `Replicating`](#événement-replicating)
pour le contrat de mutation par écouteur.

### `replicate_except`

Abandonnez des champs nommés de la réplique :

```rust
let copy = order.replicate_except(["payment_token", "stripe_id"]).await?;
```

Les champs listés retombent sur l'impl `Default` du modèle - les
`String` deviennent `""`, les `Option` deviennent `None`, etc.
Utilisez ceci pour les colonnes sensibles que la ligne répliquée ne
devrait pas transporter.

### `replicate_into::<T>` entre types

La divergence Suprnova - Laravel ne peut pas parce que PHP n'a pas
de types. `replicate_into::<T>()` fait le pont vers un type frère
via `serde_json` :

```rust
let order: Order = Order::find_or_fail(42).await?;
let invoice: Invoice = order.replicate_into::<Invoice>().await?;
invoice.save().await?;
```

Les champs dont les noms correspondent et dont les types sont
compatibles avec serde sont transportés ; les champs qui ne
correspondent d'aucun côté sont silencieusement abandonnés. `T` doit
implémenter `Default` pour que les champs non remplis aient une
valeur. La réplication entre types NE déclenche PAS `Replicating`
(l'événement porte un `&mut Self` - il n'y a aucun moyen d'adresser
`T` à travers lui). Si vous avez besoin d'une mutation pilotée par
événement, répliquez d'abord vers le même type, puis matérialisez
`T` depuis le résultat.

## Débogage - dump et dd

Deux aides de débogage interactives sur chaque `Builder<M>` :

```rust
// Journalise le SQL + les liaisons via tracing::info!, retourne self.
let users = User::query()
    .filter("active", true)
    .dump()                       // → ligne de log, le builder continue
    .order_by_desc("created_at")
    .get()
    .await?;

// Journalise en tracing::error!, puis panique avec le SQL dans le message.
User::query().filter("id", 1).dd();  // - !
```

`dump` est chaînable ; `dd` retourne `!` (ne retourne jamais - la
panique est le contrat). Les deux reflètent exactement le
`Builder::dump()` / `Builder::dd()` de Laravel.

Les deux helpers retombent sur le dialecte SQLite quand aucune
connexion BD active n'est liée (comme le repli de
`to_sql_with_bindings`), si bien qu'ils restent utiles en REPL ou
dans un test sans `TestDatabase`.

Le message de panique utilise le préfixe littéral `eloquent dd:`
pour que les tests puissent faire une assertion contre lui :

```rust
#[test]
#[should_panic(expected = "eloquent dd")]
fn dd_panics_with_sql_in_message() {
    User::query().filter("id", 1).dd();
}
```

**Ne commitez jamais `dd()` dans un chemin de code de production.**
C'est une aide de débogage interactive ; la panique en sortie est
tout le principe. `dump()` est plus sûr (il ne fait que
journaliser), mais le spammer dans des hot paths remplira vos logs -
retirez-le avant de pousser.

Si vous voulez le SQL sans les effets de bord, tournez-vous vers les
helpers qui ne journalisent pas :

- `Builder::to_sql()` - retourne le SQL rendu comme un `String`.
- `Builder::to_sql_with_bindings()` - retourne
  `(String, Vec<SeaValue>)`.
- `Builder::to_sql_for(backend)` - rend pour un dialecte explicite
  (débogage cross-backend).

## Tester les modèles

Les tests instancient une vraie base de données via `TestDatabase`,
qui enregistre la connexion dans le conteneur par test, si bien que
tout ce qui appelle `DB::connection()` à l'intérieur du SUT se
résout vers la BD de test.

### Deux points d'entrée

- **`TestDatabase::fresh::<MyMigrator>().await`** - exécute chaque
  migration que le migrateur de production exécute. Utilisez ceci
  pour les tests dogfood au niveau de l'app, où vous voulez que le
  schéma de test corresponde exactement à ce que produit
  `suprnova migrate`.
- **`TestDatabase::sqlite_memory().await`** - ouvre une base de
  données SQLite en mémoire SANS appliquer aucune migration.
  Utilisez ceci pour les tests unitaires au niveau du framework, où
  vous voulez un contrôle précis de la forme des colonnes via un
  `db.execute_unprepared("CREATE TABLE …")` par test.

### Motif dogfood au niveau app

```rust
use app::migrations::Migrator;
use app::models::users::User;
use suprnova::testing::TestDatabase;
use suprnova::{attrs, Model};

#[tokio::test]
async fn user_lifecycle() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
        password: "hashed",
    }).await.unwrap();

    assert!(alice.id > 0);

    alice.delete().await.unwrap();
    assert!(User::find(alice.id).await.unwrap().is_none(),
        "default scope hides soft-deleted rows");
}
```

La liaison `_db` détient le `TestDatabase` pour tout le test - la
dropper démantèle le conteneur et libère la connexion SQLite en
mémoire. Ne l'ombragez pas vers `_`, sinon la connexion disparaît
avant que le SUT ne s'exécute.

### Motif de forme au niveau framework

```rust
use suprnova::testing::TestDatabase;
use suprnova::{attrs, model, Model};

#[model(table = "t_users", timestamps = false)]
pub struct TUser { pub id: i64, pub name: String }

#[tokio::test]
async fn shape_test() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT)"
    ).await.unwrap();

    let u = TUser::create(attrs! { name: "Alice" }).await.unwrap();
    assert_eq!(u.name, "Alice");
}
```

### Motifs clés

- `TestDatabase::fresh::<MyMigrator>()` pour les tests au niveau app
  avec le schéma de production. `TestDatabase::sqlite_memory()`
  pour les tests de forme au niveau unitaire.
- Utilisez `TestContainer::bind` (PAS `App::bind`) pour tout
  singleton que le test mute - les redéfinitions de registre global
  entrent en course dans les exécutions parallèles. Le constructeur
  `TestDatabase` gère la liaison BD pour vous.
- Gardez les déclarations de modèle à la portée du module, pas à
  l'intérieur des fonctions de test. La macro émet un `mod` interne
  dont le `use super::*;` ne voit que les imports de premier niveau
  du fichier - déclarer un modèle à l'intérieur d'une fonction de
  test casse la résolution de type de SeaORM.

## Redescendre vers SeaORM

Trois échappatoires gardent SeaORM accessible depuis l'intérieur de
la couche Eloquent :

1. **Le module interne** - `user::Entity`, `user::Column`,
   `user::ActiveModel`, `user::Model`. La macro les émet pour chaque
   modèle ; ce sont des types SeaORM que vous pouvez utiliser
   directement. Voir
   [Disposition du module de modèle](#disposition-du-module-de-modèle)
   pour la disposition complète et pour savoir quand y aller
   chercher.
2. **Les conversions `From`** - `From<user::Model> for User` et
   `From<User> for user::Model` font le pont entre les lignes à la
   forme SeaORM (colonnes typées stockage) et les lignes à la forme
   Eloquent (colonnes typées runtime). Utile quand vous voulez
   émettre une requête SeaORM et convertir le résultat vers la forme
   Eloquent, ou l'inverse.
3. **Les types SeaORM aliasés par Suprnova** - chaque type SeaORM
   qu'un consommateur toucherait est ré-exporté sous `suprnova::*`.
   Vous ne devriez pas avoir besoin de `use sea_orm::*` dans le code
   de l'app.

```rust
use suprnova::sea_orm::{ColumnTrait, EntityTrait};

// Redescendre vers SeaORM en cours de requête - Eloquent n'a pas
// de méthode pour ça, mais SeaORM en a une :
let db = suprnova::DB::connection()?;
let users = user::Entity::find()
    .filter(user::Column::Email.like("%@example.com"))
    .all(db.inner())
    .await?;

// Convertir vers la forme Eloquent :
let eloquent: Vec<User> = users.into_iter().map(User::from).collect();
```

Trois échappatoires plus le pont `From` font que la couche Eloquent
ne vous bloque jamais pour atteindre l'ORM sous-jacent.

## Migrer depuis `database::Model`

Du code plus ancien peut porter
`impl suprnova::database::Model for Entity {}` sur une entité SeaORM
écrite à la main. Le trait a été renommé en `EntityExt` pour faire
de la place au nouveau trait `Model` - qui siège sur la struct
exposée, pas sur l'entité SeaORM.

Le chemin de migration recommandé est de faire passer le type à
`#[suprnova::model]`, qui vous donne la surface Eloquent complète
plus les traits `EntityExt` renommés en bonus. Pour le cas rare où
vous voulez garder l'ancienne forme d'extension d'Entity SeaORM, les
noms de trait `EntityExt` / `EntityExtMut` restent disponibles sous
`suprnova::database::*`. Ils se comportent exactement comme
l'ancien `database::Model`.

## Façade DB - requêtes sans modèle

Certaines tables n'ont pas leur place sur une struct
`#[suprnova::model]` : logs d'audit de courte durée, jointures de
reporting ad hoc, agrégats de tableau de bord. Pour celles-ci,
tournez-vous vers la façade `DB`. Deux surfaces se trouvent dessous :

### `DB::table(name)` - query builder chaînable

`DbTableBuilder` reflète la forme where / order / limit de
`Builder<M>` mais retourne les lignes comme `DynamicRow` (un newtype
à accesseurs typés au-dessus de `serde_json::Map<String, Value>`) :

```rust
use suprnova::DB;

let rows = DB::table("audit_log")
    .filter("actor_id", 42)
    .filter_op("created_at", ">=", "2026-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

for row in rows.iter() {
    let event: String = row.get_string("event")?;
    let actor_id: i64 = row.get_int("actor_id")?;
    println!("{actor_id}: {event}");
}
```

La surface complète :

| Méthode | Retourne | Objectif |
|--------|---------|---------|
| `.select(["id", "event"])` | `DbTableBuilder` | Restreint les colonnes (défaut `*`) |
| `.filter(col, val)` | `DbTableBuilder` | `WHERE col = ?` |
| `.filter_op(col, op, val)` | `DbTableBuilder` | `WHERE col <op> ?` |
| `.order_by_asc(col) / _desc(col)` | `DbTableBuilder` | Tri |
| `.limit(n) / .offset(n)` | `DbTableBuilder` | Fenêtrage |
| `.get()` | `Collection<DynamicRow>` | Toutes les lignes correspondantes |
| `.first()` | `Option<DynamicRow>` | Première ligne ou `None` |
| `.count()` | `u64` | `SELECT COUNT(*) ...` |
| `.insert(attrs)` | `i64` | `id` de la nouvelle ligne |
| `.update(attrs)` | `u64` | Lignes affectées |
| `.delete()` | `u64` | Lignes affectées |

**Frontière de confiance des identifiants.** Les noms de table, les
noms de colonne, les opérateurs SQL, et les directions ORDER BY sont
interpolés tels quels dans la chaîne SQL - ils ne sont PAS liés
comme des paramètres. Ne passez à ces arguments que des littéraux de
confiance, fixés à la compilation. Les valeurs (le membre de droite
de `filter` / `filter_op`) SONT liées et sûres à faire passer depuis
les données de requête.

**Un WHERE vide sur `update` / `delete` opère sur chaque ligne.**
`DB::table("audit_log").delete().await?` tronque la table par
conception - ajoutez un `filter` si ce n'est pas votre intention.

**Séparation par backend pour l'insertion.** `RETURNING id` est
utilisé sur Postgres et SQLite ; MySQL exécute l'INSERT puis émet
`SELECT LAST_INSERT_ID() as id` pour récupérer l'auto-incrément.

### `DynamicRow` - accesseurs typés sur une map JSON

`DynamicRow` enveloppe un `serde_json::Map<String, Value>` et
expose des getters typés. Chacun retourne `Result<T, FrameworkError>`
avec un message d'erreur clair en cas de clé absente ou de type
incompatible :

```rust
let event: String     = row.get_string("event")?;
let actor_id: i64     = row.get_int("actor_id")?;
let active: bool      = row.get_bool("active")?;
let prefs: Prefs      = row.get_as("prefs")?;  // n'importe quel DeserializeOwned
let raw: serde_json::Value = row.get_value("meta")?;
```

Colonnes nullables : utilisez `get_optional_*`. Ceux-ci distinguent
« colonne absente » (erreur - schéma incohérent) de « colonne
présente, valeur null » (`Ok(None)`) :

```rust
let score: Option<i64>      = row.get_optional_int("score")?;
let title: Option<String>   = row.get_optional_string("title")?;
```

`DynamicRow` fait un deref vers `Map<String, Value>`, donc
l'itération et les vérifications d'existence de clé fonctionnent
naturellement :

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}
```

### Échappatoires SQL brutes

Quand le builder ne suffit pas - fonctions de fenêtrage, CTE
récursives, DDL propre au backend - redescendez vers une chaîne
brute. Les placeholders correspondent au backend actif (`$1, $2,
...` pour Postgres, `?` pour MySQL + SQLite) :

```rust
// SELECT brut, matérialisé en DynamicRow.
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) as post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING post_count > ?",
    vec![5i64.into()],
).await?;

// UPDATE / DELETE brut - retournent les lignes affectées.
let updated = DB::update(
    "UPDATE users SET verified_at = NOW() WHERE id = ANY($1)",
    vec![ids.into()],
).await?;

let deleted = DB::delete(
    "DELETE FROM stale_sessions WHERE expires_at < ?",
    vec![now.into()],
).await?;

// DDL brut ou instructions sans liaison.
DB::statement("CREATE INDEX CONCURRENTLY idx_users_email ON users(email)")
    .await?;

// Instruction affectante générique - pour INSERT ... ON CONFLICT etc.
let rows = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1) ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec!["page_views".into()],
).await?;
```

Utilisez ces échappatoires avec parcimonie - le builder typé
attrape plus d'erreurs à la compilation et se lit plus proprement
dans la logique métier. Mais quand vous en avez besoin, elles sont
là.

**Le piège des colonnes agrégat.** Les agrégats non typés comme
`SELECT COUNT(*) AS n FROM t` fonctionnent via le helper `.count()`
du builder, mais peuvent revenir silencieusement absents des lignes
d'un `DB::select` brut sur SQLite - le
`JsonValue::from_query_result` sous-jacent parcourt les informations
de type par colonne de sqlx, et un agrégat nu n'en porte aucune. Si
vous avez besoin du chemin select brut avec des agrégats, donnez à
l'expression un contexte typé : soit utilisez un wrapper
`CAST(... AS BIGINT)`, soit lisez la colonne avec un helper typé
`DB::table(...).count()` / `.max(...)` qui utilise `query_one` +
`try_get` en coulisse.

## Existence de relation + raccourcis peu coûteux

Suprnova reprend la famille de requêtes d'existence de relation de
Laravel. Chaque méthode ici associe le nom à la forme Laravel à un
alias Rust idiomatique (la convention permanente de double API de
Suprnova).

### Filtres d'existence de relation (`has` / `where_has` / `where_belongs_to`)

La famille `EXISTS (...)` corrélée contraint la requête parente par
l'existence (ou l'absence, ou le nombre) de lignes liées, sans joindre
la relation au SELECT extérieur.

```rust
use suprnova::Model;

// Utilisateurs qui ont au moins un post.
let users = User::query().has("posts").get().await?;

// Utilisateurs qui n'ont AUCUN post.
let empty = User::query().doesnt_have("posts").get().await?;

// Utilisateurs avec >= 3 posts (Laravel `has("posts", ">=", 3)`).
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// Contrainte interne via closure - restreint le corps de la sous-requête EXISTS.
let recent = User::query()
    .where_has::<Post, _>("posts", |q| q.filter_op("created_at", ">=", "2026-01-01"))
    .get()
    .await?;

// Raccourci à une colonne - équivaut à `where_has` avec une minuscule closure.
let with_pub = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// Jointure directe belongs-to (pas d'EXISTS - la FK vit sur cette table).
let posts = Post::query().where_belongs_to("author", author.id).get().await?;
```

Toutes les variantes se composent avec leurs compagnes `or_*` et
`*_doesnt_have` :

- `has` / `or_has` / `has_count` / `doesnt_have` / `or_doesnt_have`
- `where_has` / `or_where_has` / `where_doesnt_have` / `or_where_doesnt_have`
- `where_relation` / `where_relation_op` / `or_where_relation`
- `where_belongs_to`

Le moteur lit les métadonnées de relation depuis l'inventaire
`RelationEntry` généré par la macro : colonnes de jointure, tables
pivot, discriminants morph passent tous automatiquement. Trois formes
de sous-requête sont rendues :

- **Has** - `EXISTS (SELECT 1 FROM child WHERE child.fk = parent.pk)`
- **Pivot** - `EXISTS (SELECT 1 FROM pivot INNER JOIN target ON ... WHERE pivot.parent_fk = parent.pk)`
- **Morph** - forme has/pivot plus `AND target.<morph>_type = '<value>'`

Les noms de relation inconnus rendent la forme à échec sûr (`EXISTS
(SELECT 1 WHERE 1 = 0)`), qui s'évalue à `FALSE` et retourne zéro
ligne. Une faute de frappe ne laisse jamais fuiter un balayage de
table complet.

### Divergence de `MorphTo`

L'inverse `MorphTo` de Laravel (`whereMorphedTo`, `whereHasMorph`)
parcourt plusieurs tables cibles parce que l'enfant morph porte un
discriminant `*_type` qui choisit l'un des N parents possibles. Le
`MorphTo` de Suprnova s'abaisse en une enum par famille à l'expansion
de macro - le type cible est statiquement un `<Family>Morph {
Variant1(...), ... }`, pas une unique table SQL. Le moteur d'existence
ne peut pas rendre un `EXISTS (SELECT 1 FROM <table>)` fixe pour ce
cas, parce qu'il n'y a pas de table unique.

Migration recommandée : faites la vérification d'existence au niveau
de l'enfant morph à la place. Là où Laravel écrit :

```php
Comment::whereHasMorph('commentable', [Post::class], fn ($q) => $q->where('published', true))
```

Suprnova écrit :

```rust
Comment::query()
    .filter("commentable_type", "post")
    .where_has::<Post, _>("commentable_post", |q| q.filter("published", true))
    .get()
    .await?;
```

La forme au typage plus étroit donne la complétion IDE complète sur le
builder interne, ce que le `whereHasMorph` au typage lâche ne peut
pas.

### Raccourcis peu coûteux du builder

```rust
// Filtres de PK.
User::query().where_key(7).first().await?;        // sucre pour filter("id", 7)
User::query().where_key_not(7).get().await?;      // sucre pour filter_op("id", "!=", 7)
User::query().filter("name", n).or_where_key(7).get().await?;      // ... OR id = 7
User::query().filter("name", n).or_where_key_not(7).get().await?;  // ... OR id != 7
// Alias idiomatiques Rust : filter_key / filter_key_not /
// or_filter_key / or_filter_key_not.

// Tri par created_at.
Post::query().latest().get().await?;              // ORDER BY created_at DESC
Post::query().oldest().get().await?;              // ORDER BY created_at ASC
Post::query().latest_by("published_at").get().await?;  // colonne nommée

// Correspondance à exactement un.
let one = User::query().filter("email", e).sole().await?;          // erreur sur 0 ou >1
let val: i64 = User::query().filter("id", 1).sole_value("views").await?;
let v: i64 = User::query().filter("name", "x").value_or_fail("views").await?;

// Opt-outs de chargement hâtif.
User::query().with(["posts","tags"]).without(["tags"]).get().await?;
User::query().with_only(["posts"]).get().await?;   // efface le plan d'abord

// Colonnes pleinement qualifiées (pour les jointures).
Builder::<User>::qualify_column("name");           // -> "users.name"
Builder::<User>::qualify_columns(["name", "id"]);  // -> ["users.name", "users.id"]
```

### Mutation en masse - `update_all` / `delete_all` / `upsert` / `*_each`

Celles-ci frappent la base de données directement avec une seule
instruction et ne déclenchent PAS les événements de modèle par ligne.
Utilisez-les quand un rétrécissement de scope suffit et que vous
n'avez pas besoin des hooks de cycle de vie ; pour des hooks par
ligne, itérez avec `.get()` et appelez `.update()` / `.delete()` par
ligne. `delete_all` cible toujours le `M::TABLE` statique du modèle ;
les noms de table à l'exécution ne sont pas acceptés comme SQL
exécutable. Les attributs explicitement nuls sont émis comme `NULL`
SQL, si bien que les colonnes bigint, integer, boolean, timestamp et
autres colonnes non textuelles nullables conservent leur type de base
de données sur PostgreSQL. Chaque attribut non nul reste lié en tant
que paramètre. Les lignes d'upsert doivent avoir le même ensemble de
colonnes ; une clé manquante ou surnuméraire est rejetée plutôt
qu'interprétée comme nulle.

```rust
// UPDATE en masse.
let n = User::query()
    .filter("active", false)
    .update_all(attrs! { archived_at: Utc::now() })
    .await?;

// DELETE en masse.
let n = Session::query()
    .filter_op("expires_at", "<", cutoff)
    .delete_all()
    .await?;

// INSERT ... ON CONFLICT (Postgres / SQLite) / ON DUPLICATE KEY UPDATE (MySQL).
let n = Counter::query()
    .upsert(
        vec![attrs! { key: "page_views", n: 1 }, attrs! { key: "signups", n: 1 }],
        vec!["key"],                  // cible du conflit
        Some(vec!["n"]),              // colonnes à mettre à jour ; None = chaque colonne non unique
    )
    .await?;

// Incrément/décrément atomique contre un scope.
User::query()
    .filter("id", 7)
    .increment_each(vec![("views", 1), ("likes", 1)])
    .await?;

User::query()
    .filter("id", 7)
    .decrement_each(vec![("balance", 100)])
    .await?;
```

### Helpers statiques de `Model`

```rust
// Destruction en masse par ensemble de PK. Les événements par ligne se
// déclenchent (chaque ligne passe par .delete(), donc la sémantique de
// pierre tombale des suppressions logicielles + le dispatch de
// Deleting/Deleted sont respectés).
let removed: u64 = User::destroy(vec![1i64, 2, 3]).await?;
let removed: u64 = User::force_destroy(vec![1i64, 2, 3]).await?;

// Comparaison d'identité par PK.
assert!(alice.is(&also_alice));
assert!(alice.is_not(&bob));
```

### Variantes `*Quietly` - supprimer les événements de cycle de vie

Du sucre par-dessus `seed::without_events`. Les cinq événements
statiques de cycle de vie
(`Saving`/`Creating`/`Updating`/`Deleting`/`Restoring`) et les
after-events non annulables court-circuitent tous les deux à
l'intérieur de la portée.

```rust
user.save_quietly().await?;            // pas de Saving / Updated / Saved
user.update_quietly(attrs).await?;
user.delete_quietly().await?;
user.force_delete_quietly().await?;
```

### Variantes `*_or_fail`

Erreur explicite dans le cas non trouvé. Utile dans les chemins de
code qui vérifient des invariants, où une ligne manquante est un bug.

```rust
let user = user.update_or_fail(attrs).await?;   // not_found si la ligne a été supprimée en vol
user.delete_or_fail().await?;
```

### Sérialisation filtrée - `to_array_except` / `to_array_only`

Le remplacement Rust natif de Suprnova pour les `makeHidden` /
`makeVisible` par instance de Laravel. La struct Eloquent ne porte pas
de sac d'attributs à l'exécution, donc la liste de colonnes est
fournie sur le site d'appel :

```rust
return Json::ok(user.to_array_except(&["password_hash", "remember_token"]));
return Json::ok(user.to_array_only(&["id", "name", "email"]));
```

**Note de divergence.** Le `makeHidden` par instance de Laravel mute
un état qui se propage quand le modèle est imbriqué dans l'appel
`toArray()` d'un parent. Le filtre de Suprnova est terminal - il
produit une `serde_json::Value` et n'affecte pas les sérialisations
futures de `self`. Pour un contrôle de visibilité déclaratif et
permanent, utilisez les attributs `#[model(hidden = [...])]` /
`#[model(visible = [...])]`.

### Clés primaires UUID / ULID - `#[model(unique_id = "...")]`

L'analogue Suprnova de la famille de traits `HasUuids` / `HasUlids` /
`HasVersion4Uuids` de Laravel. Posez l'attribut, typez la PK en
`String`, et la macro remplit automatiquement l'ID avant l'INSERT.

```rust
#[model(
    table = "users",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "uuid",      // ou "uuid_v4", "ulid"
)]
pub struct User {
    pub id: String,
    pub email: String,
}

// Rempli automatiquement :
let u = User::create(attrs! { email: "a@b.com" }).await?;
// u.id est un UUID v7 tout neuf.

// Les ID fournis par l'appelant l'emportent toujours (correspond au comportement de HasUuids de Laravel).
let u = User::create(attrs! { id: "...", email: "..." }).await?;
```

Stratégies prises en charge :

- `"uuid"` / `"uuid_v7"` - UUID v7 (ordonné par horodatage,
  recommandé ; correspond au `Str::uuid7()` par défaut de Laravel 11+)
- `"uuid_v4"` - UUID aléatoire (correspond à `HasVersion4Uuids`)
- `"ulid"` - ULID de 26 caractères en base32 Crockford minuscule

La macro émet un bloc `impl HasUniqueId for YourStruct` exposant
`UNIQUE_ID_KIND` et un hook `new_unique_id()` que vous pouvez
redéfinir sur le type pour un générateur personnalisé (p. ex. des ID
préfixés comme `usr_<uuid>`).

### `find_or` / `find_or_new` / `create_or_first`

Complètent la surface du trait `FirstOrCreate`.

```rust
// Recherche par PK ; exécute le repli si non trouvé.
let user = User::find_or(id, || async {
    User::create(attrs! { id, name: "guest" }).await
}).await?;

// Recherche par PK ; construit une instance non sauvegardée depuis les défauts si non trouvé.
let user = User::find_or_new(id, attrs! { name: "draft" }).await?;
// user.id == 0 ici - l'instance est en mémoire seulement.

// Insertion sûre en course : tente create, retombe sur fetch en cas de conflit.
let user = User::create_or_first(
    attrs! { email: "race@x.com" },
    attrs! { name: "race winner" },
).await?;
```

### Le scope `without_touching`

L'analogue Suprnova du `Model::withoutTouching` de Laravel. À
l'intérieur du scope, chaque appel `model.touch().await`
court-circuite - utile quand vous exécutez des migrations de données
ou des jobs de lot qui mutent les timestamps par d'autres chemins.

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    // Les appels .touch() ici sont sans effet.
    for post in posts {
        post.touch().await?;
    }
}).await;
```

Le scope est adossé à `tokio::task_local`, si bien que les requêtes
concurrentes sur d'autres tâches continuent de respecter leur propre
scope (ou son absence).

`without_touching` supprime aussi la [cascade de mise à jour du
parent](#toucher-le-parent) : un enfant sauvegardé dans le scope laisse
tranquille chaque propriétaire nommé dans sa liste `touches`.

`without_touching_on::<Post, _, _>(fut)` est la forme par type  -
`Model::withoutTouchingOn([Post::class], $cb)` de Laravel. À l'intérieur,
`post.touch()` et toute cascade qui avancerait un `Post` se taisent, tandis
que les propriétaires de tout autre type continuent d'avancer :

```rust
use suprnova::eloquent::without_touching_on;

without_touching_on::<Post, _, _>(async {
    // Les sauvegardes de Comment ici laissent tranquilles leurs propriétaires
    // Post ; un propriétaire Video sur le même commentaire avance quand même.
    comment.save().await
}).await?;
```

Les scopes s'imbriquent, et tous deux sont adossés à `tokio::task_local`.

## Suivant

- [Relations Eloquent](eloquent-relationships.md) - plongée en
  profondeur sur chaque type de relation, le registre polymorphe, et
  l'abaissement de l'enum polymorphe
- [Collections Eloquent](eloquent-collections.md) - surface
  complète de `Collection<T>`, la séparation générique-vs-modèle, et
  le streaming `LazyCollection<M>`
- [Eloquent - Casts, accesseurs et mutateurs](eloquent-mutators.md) -
  les 22 casts intégrés plus la redéfinition à l'exécution `casts!`
- [Sérialisation Eloquent](eloquent-serialization.md) - `to_array`,
  `to_json`, hidden / visible / appends, terminaux filtrés
- [Fabriques Eloquent](eloquent-factories.md) - instances de modèle
  randomisées pour les tests et les seeders

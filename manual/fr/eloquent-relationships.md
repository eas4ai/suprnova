# Relations Eloquent

[Eloquent](eloquent.md) couvre la surface de relation du quotidien -
la syntaxe de déclaration, le tableau des options, le chaînage de base
par type. Ce chapitre est la plongée en profondeur spécifique aux
relations : comment un appel `user.posts()` se résout réellement en
SQL, comment le chargeur hâtif évite le N+1, comment le moteur
d'existence (`has` / `where_has` / `where_belongs_to`) rend des
sous-requêtes `EXISTS` corrélées, comment le polymorphisme survit à
l'absence de liaison statique tardive en Rust, et ce qui découle du
système de types quand les onze types de relation doivent coexister
sur un seul trait.

Si vous découvrez Eloquent sur Suprnova, lisez d'abord
[Eloquent](eloquent.md#relationships) - cette page enseigne la syntaxe
de déclaration. Cette page suppose que vous avez déjà un modèle avec
un bloc `relations = { ... }` et que vous voulez comprendre ce qu'il y
a dessous.

## Les onze types de relation

Chaque type de relation dans [`RelationKind`][relations] est l'un des
suivants :

| Type                  | Côté       | Cardinalité | À travers les familles | Pivot |
|-----------------------|------------|-------------|-----------------|-------|
| `HasOne<R>`           | parent     | un         | non              | - |
| `HasMany<R>`          | parent     | plusieurs        | non              | - |
| `BelongsTo<R>`        | enfant      | un         | non              | - |
| `BelongsToMany<R, P>` | l'un ou l'autre     | plusieurs        | non              | oui   |
| `HasOneThrough<B, R>` | parent     | un         | non              | - |
| `HasManyThrough<B, R>`| parent     | plusieurs        | non              | - |
| `MorphOne<R>`         | parent     | un         | oui              | - |
| `MorphMany<R>`        | parent     | plusieurs        | oui              | - |
| `MorphTo`             | enfant      | un         | oui (n cibles) | - |
| `MorphToMany<R, P>`   | parent     | plusieurs        | oui              | oui   |
| `MorphedByMany<R, P>` | partenaire m2m| plusieurs        | oui (inverse)   | oui   |

« À travers les familles » veut dire que le *type* de la ligne liée
varie - un `Comment` peut appartenir à un `Post` ou à une `Video`, pas
à une seule table parente fixe. C'est le polymorphisme, et Suprnova le
gère via le [registre polymorphe](#le-registre-polymorphe) plus un
enum par famille.

[relations]: https://docs.rs/suprnova

### Ce que la macro émet

Quand vous écrivez :

```rust
use suprnova::model;

#[model(table = "users", relations = {
    posts: HasMany<Post>,
})]
pub struct User {
    pub id: i64,
    pub name: String,
}
```

`#[suprnova::model]` se développe en cinq choses pour `posts` :

1. **Méthode de relation** - `fn posts(&self) -> HasMany<Self, Post>`.
   Retourne un wrapper paresseux qui porte `self.id` plus les
   métadonnées de FK ; aucun SQL ne s'exécute encore.
2. **Accesseur chargé** - `fn posts_loaded(&self) -> &[Post]`. Lit
   depuis le cache hâtif après `User::with(["posts"])`. Slice vide
   quand aucun chargement hâtif n'a eu lieu.
3. **Accesseur de compte** - `fn posts_count(&self) -> u64`. Lit
   depuis le même cache après `User::with_count(["posts"])`.
4. **Bras de dispatcher** - bras de `match` dans la méthode inhérente
   `__eager_load` du modèle. Le chargeur hâtif recherche `"posts"` et
   exécute la requête `IN`.
5. **Entrée d'inventaire** - un
   `inventory::submit!(RelationEntry { ... })` pour que la relation
   soit énumérable à l'exécution (l'outillage admin, le moteur
   d'existence, le dispatcher polymorphe parcourent tous ceci).

Vous ne voyez jamais (4) ni (5). Ils alimentent le reste de ce
chapitre.

## Résolution paresseuse : comment `user.posts()` devient du SQL

`user.posts()` retourne un wrapper `HasMany<User, Post>`, pas un
résultat de requête. Le wrapper détient la valeur de PK du parent plus
le nom de colonne FK, et un `Builder<Post>` pré-filtré avec
`WHERE posts.user_id = ?` déjà appliqué. Rien n'a encore touché la
base de données.

```rust
use suprnova::Direction;

// Pas de SQL.
let posts_q = user.posts();

// SQL : SELECT * FROM posts WHERE user_id = ? ORDER BY id DESC LIMIT 5
let recent = user.posts()
    .order_by("id", Direction::Desc)
    .limit(5)
    .get()
    .await?;

// SQL : SELECT COUNT(*) FROM posts WHERE user_id = ?
let n = user.posts().count().await?;
```

La surface à double API ([Eloquent → Note de nommage](eloquent.md#naming-note-dual-api))
est honorée sur le wrapper : `.filter("col", v)` et
`.db_where("col", v)` fonctionnent tous les deux, à l'identique. La
surface chaînable sur `HasOne` / `HasMany` / `MorphOne` / `MorphMany`
couvre `filter` / `db_where` / `order_by` / `latest` / `oldest` /
`limit` / `take`. Les relations Through et m2m polymorphes n'exposent
que leurs méthodes terminales - elles passent par des coutures SQL
écrites à la main, pas par un `Builder<R>`, si bien qu'elles ne
peuvent pas se composer avec la chaîne standard. Voir
[relations Through](#hasonethrough-et-hasmanythrough) et
[m2m polymorphe](#morphtomany-et-morphedbymany) plus bas.

### Les suppressions logicielles se propagent

Quand le type lié implémente [`SoftDeletes`](eloquent.md#soft-deletes-flag),
le wrapper de relation hérite de sa portée globale. `user.posts().get()`
cache les posts à la corbeille de la même façon que
`Post::query().get()`. Trois relais transpercent :

```rust
let alive = user.posts().get().await?;                 // défaut : vivants seulement
let all = user.posts().with_trashed().get().await?;    // vivants + à la corbeille
let dead = user.posts().only_trashed().get().await?;   // à la corbeille seulement
```

`with_trashed` / `only_trashed` existent sur `HasOne`, `HasMany`,
`MorphOne`, `MorphMany`, `BelongsToMany`, `MorphToMany`,
`MorphedByMany`, et `BelongsTo`. Ils sont délibérément absents de
`HasOneThrough` et `HasManyThrough` - voir la
[lacune de suppression logicielle Through](#suppressions-logicielles-through-v1)
plus bas.

## Un-à-un : `HasOne` et `BelongsTo`

`HasOne` est le parent qui dit « cet enfant a une colonne qui pointe
vers moi ». `BelongsTo` est l'enfant qui dit « j'ai une colonne qui
pointe vers le parent ». Les deux exécutent un unique
`WHERE fk = ? LIMIT 1` et retournent `Option<R>`.

```rust
// HasOne - parent → enfant
let profile: Option<Profile> = user.profile().first().await?;

// BelongsTo - enfant → parent
let owner: Option<User> = profile.user().first().await?;
```

`BelongsTo` ajoute une facilité en forme Laravel dont les autres n'ont
pas besoin : `with_default`. Quand la FK de l'enfant est nulle OU que
la ligne parente a été supprimée, `first()` retourne le substitut de
la closure plutôt que `None` :

```rust
#[model(table = "comments", relations = {
    author: BelongsTo<User> {
        with_default = || User { id: 0, name: "Guest".into(), .. },
    },
})]
pub struct Comment { /* ... */ }

// Retourne toujours Some(User) - soit l'auteur réel, soit le substitut Guest.
let display: Option<User> = comment.author().first().await?;
```

Le dispatcher de chargement hâtif honore le même repli - les chemins
paresseux et hâtif partagent le comportement par défaut, si bien que
le code de template qui affiche `comment.author_loaded()[0].name` n'a
pas besoin de brancher.

## Un-à-plusieurs : `HasMany`

`HasMany` est la relation à cardinalité plurielle côté parent. Le
terminal `.get()` retourne une
[`Collection<R>`](eloquent.md#collections) - le wrapper en forme
Laravel autour de `Vec<R>` - si bien que la surface consciente du
modèle se compose :

```rust
let titles = user.posts()
    .order_by("created_at", Direction::Desc)
    .limit(10)
    .get()
    .await?
    .pluck::<String>("title");
```

`latest()` et `oldest()` sont du sucre pour
`order_by("created_at", Direction::Desc)` et `Asc` respectivement -
ils ne se résolvent que contre des modèles qui déclarent une colonne
`created_at`, que la macro `#[suprnova::model]` ajoute automatiquement
dès que les timestamps sont actifs (le défaut).

## Plusieurs-à-plusieurs : `BelongsToMany<R, P>` et le pivot de premier ordre

`BelongsToMany` est du plusieurs-à-plusieurs à travers une table de
jointure. Le pivot de Suprnova est lui-même une struct
`#[suprnova::model]` avec ses propres migrations, ses propres
accesseurs, ses propres événements. C'est la divergence - voir
[plus bas](#pourquoi-suprnova-diverge-le-pivot-est-un-vrai-modèle).

```rust
#[model(table = "users", relations = {
    roles: BelongsToMany<Role, RoleUser> {
        with_pivot = ["assigned_at"],
        with_timestamps,
    },
})]
pub struct User { /* ... */ }

#[model(table = "role_user", primary_key = "id")]
pub struct RoleUser {
    pub id: i64,
    pub user_id: i64,
    pub role_id: i64,
    pub assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Les mutateurs s'exécutent contre la ligne pivot :

```rust
use suprnova::attrs;

user.roles().attach(role.id).await?;
user.roles().attach_with(role.id, attrs! { assigned_at: now }).await?;
user.roles().detach(role.id).await?;
user.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
```

`sync` lit l'ensemble pivot courant, calcule
`attach_set = ids - current` et `detach_set = current - ids`, et
exécute les deltas à l'intérieur d'une transaction. Les doublons dans
l'ensemble en entrée se réduisent par leur forme JSON-chaîne, si bien
que `sync([1, 1, 2])` fait ce que vous voulez.

La lecture passe par la stratégie à deux requêtes :

```rust
// Requête 1 : SELECT roles.*, role_user.* via INNER JOIN, cantonnée par user_id.
// Requête 2 : SELECT role_user.* pour la même jointure, pour estampiller __pivot par ligne.
let roles = user.roles().get().await?;

// Chaque rôle porte le contexte pivot que la macro a rendu accessible :
for r in &roles {
    let pivot = r.pivot::<RoleUser>().expect("loaded via BelongsToMany");
    println!("{} assigned at {:?}", r.name, pivot.assigned_at);
}
```

### Pourquoi Suprnova diverge : le pivot est un vrai modèle

Le pivot de Laravel est un sac opaque par attribut (`$role->pivot->note`).
Suprnova exige que vous déclariez la struct pivot parce que le système
de types de Rust a besoin des colonnes à la compilation - et une fois
que vous avez payé cette déclaration, le pivot reçoit le même
traitement `#[suprnova::model]` que n'importe quelle autre table :
migrations, événements, observateurs, fabriques, suppression
logicielle. `r.pivot::<RoleUser>()` retourne une référence typée ;
aucune recherche d'attribut à clé de chaîne, aucune surprise à
l'exécution quand une colonne est mal orthographiée.

Le coût est une struct supplémentaire par table pivot. Le bénéfice est
que le pivot peut porter du comportement - de la logique métier, des
règles de validation, des colonnes d'audit - sans s'échapper vers du
SQL brut.

## `HasOneThrough` et `HasManyThrough`

Des relations à deux sauts : `A → B → C` où `B` est un modèle
intermédiaire dont la FK pointe vers `A`, et `C` est la cible finale
dont la FK pointe vers `B`. Exemple classique : `Country` a plusieurs
`User` ; `User` a plusieurs `Post` ; `Country::posts()` saute les deux
sauts en un seul aller-retour SQL.

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<User, Post>,
})]
pub struct Country { /* ... */ }

// Un seul INNER JOIN : SELECT posts.* FROM posts
//   INNER JOIN users ON posts.user_id = users.id
//   WHERE users.country_id = ?
let posts: Collection<Post> = country.posts().get().await?;
```

`HasOneThrough` a la même forme mais `.get()` retourne `Option<C>`
(à l'image de la sémantique à cardinalité un) et `.first()` en est
l'alias.

Les wrappers Through n'exposent que leurs terminaux - `get` /
`first` / `count` plus les setters de clé (`first_key` / `second_key`
/ `local_key` / `second_local_key`). Ils ne s'écoulent pas à travers un
`Builder<C>`, donc ils ne peuvent pas chaîner `.filter(...)` ou
`.order_by(...)`. Si vous avez besoin de filtrer à travers la
jointure, retombez sur deux sauts de relation explicites.

### Suppressions logicielles Through (v1)

Les relations Through utilisent du SQL `INNER JOIN` brut plutôt que le
pipeline `Builder<C>`, si bien que la portée globale de suppression
logicielle que `C::query()` installerait
(`WHERE c.deleted_at IS NULL`) n'est **pas** appliquée. Les
intermédiaires à la corbeille et les cibles à la corbeille participent
tous deux à la jointure.

Cela diverge de Laravel, où `hasManyThrough` filtre à la fois `B` et
`C` par `deleted_at IS NULL` quand les modèles déclarent
`SoftDeletes`. Jusqu'à ce que le correctif arrive, les appelants qui
ont besoin de lectures Through cantonnées devraient chaîner les deux
relations explicitement :

```rust
// Au lieu de country.posts().get() :
let users = country.users().get().await?;
let user_ids: Vec<i64> = users.iter().map(|u| u.id).collect();
let posts = Post::query().filter_in("user_id", user_ids).get().await?;
// Les portées de suppression logicielle de User et de Post s'appliquent toutes deux.
```

## Relations polymorphes

Une FK polymorphe est une paire de colonnes : `<name>_id` (la clé
primaire de la ligne) plus `<name>_type` (une chaîne qui identifie
*dans quelle table* l'id vit). Une ligne `Comment` peut pointer vers
un `Post` ou une `Video` sans ajouter ni colonne `post_id` ni colonne
`video_id`.

Suprnova livre quatre types polymorphes : `MorphOne`, `MorphMany`,
`MorphTo`, et la paire m2m `MorphToMany` / `MorphedByMany`. Ils
partagent tous un même morceau d'infrastructure :
[le registre polymorphe](#le-registre-polymorphe).

### `MorphOne<R>` et `MorphMany<R>` - côté parent

`MorphOne` et `MorphMany` reflètent `HasOne` et `HasMany` mais
superposent le discriminant `<name>_type`. Le builder interne est
pré-filtré avec `WHERE <name>_id = ? AND <name>_type = ?`, si bien que
les enfants polymorphes qui pointent vers *d'autres* familles
n'apparaissent jamais dans le résultat.

```rust
#[model(table = "posts", morph_type = "post", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Video { /* ... */ }

let post_comments = post.comments().get().await?;     // seulement commentable_type = 'post'
let video_comments = video.comments().get().await?;   // seulement commentable_type = 'video'
```

`morph_type = "post"` est la chaîne que le parent enregistre dans la
colonne `commentable_type` de l'enfant. Le défaut est le nom de struct
en snake_case, mais redéfinir est le bon choix pour tout modèle que
vous livrez - les refactorings qui renomment des tables ne devraient
pas casser la clé polymorphe.

### `MorphTo` et l'enum par famille

`MorphTo` vit du côté table polymorphe. L'utilisateur déclare la
*liste des cibles* à l'avance :

```rust
#[model(table = "comments", relations = {
    commentable: MorphTo { name = "commentable", targets = [Post, Video] },
})]
pub struct Comment {
    pub id: i64,
    pub commentable_id: i64,
    pub commentable_type: String,
    pub body: String,
}
```

La macro émet un enum par famille au site de déclaration :

```rust
// Émis par la macro - vous n'écrivez pas ceci.
pub enum CommentableMorph {
    Post(Post),
    Video(Video),
    Unknown(String, i64),     // repli pour un <name>_type non enregistré
}
```

Et `comment.commentable()` retourne un helper de récupération dont le
`.get()` se résout vers l'enum :

```rust
match comment.commentable().get().await? {
    CommentableMorph::Post(post) => println!("on post: {}", post.title),
    CommentableMorph::Video(video) => println!("on video: {}", video.url),
    CommentableMorph::Unknown(t, id) => {
        eprintln!("orphaned commentable_type={t} id={id}");
    }
}
```

### Pourquoi Suprnova diverge : enum par famille

Le `morphTo` de Laravel retourne `mixed` - le dispatch dynamique de
PHP résout la méthode à l'exécution. Rust n'a pas de liaison statique
tardive, donc Suprnova rend la famille explicite. Les bénéfices
dépassent le coût de frappe :

- **`match` exhaustif** - le compilateur vous dit quand une nouvelle
  cible polymorphe arrive et que vous avez oublié de la gérer.
- **`Unknown(String, id)` est type-safe** - les lignes orphelines
  d'une classe de modèle parent supprimée surgissent comme une
  variante, sans provoquer de panique.
- **La liste des cibles documente le schéma** - lire la déclaration
  `MorphTo` vous dit chaque type qui peut se trouver à l'autre bout.
  Aucune requête de base de données n'est nécessaire pour les
  énumérer.

### Restriction v1 : `MorphTo` est réservé à `i64`

`MorphTo::morph_id` est câblé en dur sur `i64`. Les cibles polymorphes
doivent donc utiliser des clés primaires `i64`, et la colonne
`<name>_id` de la table polymorphe doit aussi être `i64`. Les modèles
dont la PK est `String` ou `Uuid`-via-chaîne ne peuvent pas être des
cibles `MorphTo` en v1. La v2 paramétrera le type d'id polymorphe pour
que tout le treillis de PK (`i64` / `String` / `Uuid`) soit accepté.

C'est une restriction propre au sens inverse du polymorphisme.
`MorphOne` / `MorphMany` / `MorphToMany` / `MorphedByMany`
fonctionnent bien avec n'importe quelle forme de PK - ils lisent
directement l'`id` déjà typé du parent.

### `MorphToMany` et `MorphedByMany`

Du plusieurs-à-plusieurs polymorphe à travers un unique pivot. Un côté
est « morphable » (`Post.tags()`, `Video.tags()` - les deux passent
par le même pivot `taggables`). L'autre est le partenaire m2m partagé
(`Tag.posts()`, `Tag.videos()` - même pivot, parcouru dans l'autre
sens).

```rust
#[model(table = "tags", relations = {
    posts: MorphedByMany<Post, Taggable> {
        name = "taggable",
        target_morph_type = "post",
    },
    videos: MorphedByMany<Video, Taggable> {
        name = "taggable",
        target_morph_type = "video",
    },
})]
pub struct Tag { /* ... */ }

#[model(table = "posts", morph_type = "post", relations = {
    tags: MorphToMany<Tag, Taggable> { name = "taggable" },
})]
pub struct Post { /* ... */ }

#[model(table = "taggables", primary_key = "id", timestamps = false)]
pub struct Taggable {
    pub id: i64,
    pub tag_id: i64,
    pub taggable_id: i64,
    pub taggable_type: String,
}
```

`MorphToMany` est le côté mutant - `attach` / `attach_with` /
`detach` / `sync` vivent tous là. `MorphedByMany` est en lecture
seule : chaque appel `tag.posts()` ne retourne que des taggables de
type `Post`, chaque `tag.videos()` ne retourne que des taggables de
type `Video`, aucun mélange dans une même collection.

Mutez depuis le côté morphable :

```rust
post.tags().attach(rust_tag.id).await?;
post.tags().sync([rust_tag.id, async_tag.id]).await?;
```

Lisez depuis l'un ou l'autre :

```rust
let tags_on_post: Collection<Tag> = post.tags().get().await?;
let posts_with_rust_tag: Collection<Post> = rust_tag.posts().get().await?;
```

## Le registre polymorphe

Chaque struct annotée `#[suprnova::model(morph_type = "...")]` émet un
[`MorphTypeEntry`][morph] via `inventory::submit!` à la compilation.
Le registre alimente trois choses :

1. **Dispatch de l'enum par famille** - `MorphTo.get()` lit la chaîne
   `<name>_type` de la ligne enfant et la recherche pour trouver la
   bonne variante d'enum.
2. **Filtrage de cible `MorphedByMany`** - `target_morph_type = "post"`
   se résout via le registre pour s'assurer que la chaîne de type est
   réelle.
3. **Vérifications de cohérence** - `find_morph_type("post")` retourne
   `None` si aucun modèle ne s'est enregistré avec cette chaîne, ce
   qui distingue « délibérément non enregistré » de « coquille ».

```rust
use suprnova::{morph_types, find_morph_type, find_morph_type_by_id};
use std::any::TypeId;

for entry in morph_types() {
    println!("{} -> {}", entry.morph_type, entry.type_name);
}

if let Some(e) = find_morph_type("post") {
    assert_eq!(e.table, "posts");
}

let by_id = find_morph_type_by_id(TypeId::of::<Post>());
```

[morph]: https://docs.rs/suprnova

Les modèles sans attribut `morph_type = "..."` ne s'enregistrent
délibérément pas - le registre est opt-in. Un modèle `User` non
polymorphe n'y contribue rien, ce qui est ce qui fait que
`find_morph_type("user")` retournant `None` est un signal utile.

## Requêter par existence de relation

`has` / `where_has` / `doesnt_have` / `where_relation` /
`where_belongs_to` forment le moteur d'existence de relation de
Suprnova. Ils rendent tous des sous-requêtes `EXISTS (...)` corrélées
contre le **propre SELECT du parent** - pas de JOIN, pas de lignes
parentes dupliquées, pas de GROUP BY.

```rust
// Utilisateurs avec au moins un post.
let with_posts = User::query().has("posts").get().await?;

// Utilisateurs avec au moins trois posts.
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// Utilisateurs avec au moins un post PUBLIÉ.
let published_authors = User::query()
    .where_has::<Post, _>("posts", |q| q.filter("published", true))
    .get()
    .await?;

// Utilisateurs SANS AUCUN post.
let empty_users = User::query().doesnt_have("posts").get().await?;

// Utilisateurs sans post BROUILLON (ils peuvent encore avoir des posts publiés).
let clean = User::query()
    .where_doesnt_have::<Post, _>("posts", |q| q.filter("published", false))
    .get()
    .await?;

// Raccourci : where_has + une seule colonne == correspondance.
let same = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// where_belongs_to - FK directe = ? sur CETTE table (pas d'EXISTS
// nécessaire, puisque la FK est sur la ligne enfant).
let mine = Post::query()
    .where_belongs_to("author", user.id)
    .get()
    .await?;
```

### Comment ça fonctionne

Le moteur parcourt l'inventaire de relations au moment de la
construction de la requête. Pour chaque relation nommée, il récupère
le `RelationEntry` et rend la forme SQL appropriée selon le type :

- `HasOne` / `HasMany` / `MorphOne` / `MorphMany` →
  `EXISTS (SELECT 1 FROM child WHERE child.<fk> = parent.<pk>)`. Les
  types polymorphes ajoutent
  `AND child.<name>_type = '<parent_morph_type>'`.
- `BelongsTo` →
  `EXISTS (SELECT 1 FROM parent WHERE parent.<pk> = child.<fk>)`.
- `BelongsToMany` / `MorphToMany` → joint à travers le pivot :
  `EXISTS (SELECT 1 FROM pivot WHERE pivot.<parent_fk> = parent.<pk> ...)`.
- Les relations Through → joignent à travers l'intermédiaire.

La forme par closure (`where_has::<R, _>(rel, |q| ...)`) construit un
`Builder<R>` interne ; tous les termes WHERE que ce builder produit
atterrissent dans le corps de la sous-requête. La numérotation des
placeholders est monotone à travers l'instruction entière, si bien que
le moteur fonctionne correctement avec les paramètres Postgres de
style `$1`.

`where_belongs_to` est la seule exception qui ne rend pas un EXISTS.
La FK de belongs-to vit sur la *propre* ligne du parent, si bien qu'un
`WHERE child.<fk> = ?` direct est exactement le bon SQL - aucune
sous-requête nécessaire. Si le nom de relation est inconnu de
l'inventaire du parent, le moteur émet `WHERE 1 = 0` pour que la
requête retourne sûrement rien.

### Pourquoi ceci l'emporte sur LEFT JOIN

L'ancien moteur `has` / `whereHas` de Laravel émettait des JOIN et
dupliquait les lignes parentes ; la réécriture en EXISTS corrélé est
arrivée dans Laravel 9. Suprnova livre EXISTS depuis le premier jour.
Les avantages : aucun doublon dans le jeu de résultats, aucun
contournement par GROUP BY pour les agrégats, aucun besoin de
`DISTINCT`, et l'optimiseur de la base de données voit une vraie
sous-requête au lieu d'un JOIN à travers lequel il ne peut pas pousser
de prédicats. Pour `has_count(rel, ">=", n)`, le moteur rend
directement `(SELECT COUNT(*) FROM child WHERE ...) >= n` - une seule
requête, un seul plan.

## Chargement hâtif - agrégats `with`, `with_count`, `with_*`

Le `user.posts().get()` paresseux fait une requête par parent. C'est
du N+1 quand vous avez beaucoup d'utilisateurs :

```rust
// Mauvais : 1 requête pour les utilisateurs + 100 requêtes pour les posts.
let users = User::query().limit(100).get().await?;
for u in &users {
    let posts = u.posts().get().await?;
    /* ... */
}
```

`with(["posts"])` réduit cela à deux requêtes au total - quel que soit
le nombre de parents :

```rust
// Bon : 1 requête pour les utilisateurs + 1 requête IN pour tous les posts.
let users = User::query()
    .with(["posts"])
    .limit(100)
    .get()
    .await?;

for u in &users {
    for post in u.posts_loaded() {       // lit depuis le cache, pas de SQL
        println!("{}: {}", u.name, post.title);
    }
}
```

Les chemins imbriqués fonctionnent aussi - les noms de relation
séparés par des points récursent :

```rust
let users = User::query()
    .with(["posts.comments.author"])
    .get()
    .await?;
// 4 requêtes : users, posts IN users.id, comments IN posts.id, authors IN comments.user_id.
```

### `with_count` et les agrégats

`with_count` ajoute un agrégat `COUNT(*) GROUP BY parent_fk` par
relation, chargé aux côtés des parents - une requête supplémentaire
par relation :

```rust
let users = User::query().with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}
```

Quatre variantes d'agrégat s'empilent : `with_sum`, `with_avg`,
`with_min`, `with_max`. La forme de clé de cache est
`<rel>_<kind>_<col>`, si bien qu'empiler plusieurs agrégats sur la
même relation n'entre pas en collision :

```rust
let users = User::query()
    .with_count(["posts"])
    .with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .get()
    .await?;

for u in &users {
    println!(
        "{}: {} posts, {} views total, {} avg",
        u.name,
        u.posts_count(),
        u.posts_sum_of("views").unwrap_or(0.0),
        u.posts_avg_of("views").unwrap_or(0.0),
    );
}
```

Voir [Eloquent → Chargement hâtif → Disposition du cache](eloquent.md#cache-layout)
pour le contrat de stockage complet.

### Chargements hâtifs contraints - `with_where`

`with_where` filtre quelles lignes enfant atterrissent dans le cache
hâtif sans perdre les parents qui n'ont aucun enfant correspondant :

```rust
use suprnova::Builder;

let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// Chaque u.posts_loaded() ne contient que des posts publiés.
// Les utilisateurs avec zéro post publié apparaissent quand même dans
// le jeu de résultats - leur posts_loaded() retourne un slice vide.
```

`with_where` diffère de `where_has` dans son intention : `where_has`
filtre l'ensemble des parents (« utilisateurs qui ont au moins un post
publié ») ; `with_where` filtre le cache hâtif (« pour tous les
utilisateurs, ne charger que leurs posts publiés »). Utilisez les deux
ensemble quand vous voulez les deux effets.

Le prédicat est un `Fn`, pas un `FnOnce`, si bien qu'un builder qui en
porte un peut être cloné et exécuté plus d'une fois. Une closure qui
veut consommer une valeur capturée devrait la cloner à l'intérieur :

```rust
let wanted = vec!["rust".to_string(), "web".to_string()];
let users = User::query()
    // `wanted.clone()` à l'intérieur, pas `move` de `wanted` lui-même - la
    // closure peut s'exécuter une fois par clone du builder.
    .with_where(("posts", move |q: Builder<Post>| q.filter_in("tag", wanted.clone())))
    .get()
    .await?;
```

### Cloner une requête garde son plan de chargement hâtif

`Builder` est `Clone`, et le clone emporte le plan de chargement hâtif
avec lui, si bien que le motif « construire une requête de base, en
dériver plusieurs » fonctionne :

```rust
let base = User::query().with(["posts"]).filter("active", true);

let first_page = base.clone().limit(20).get().await?;
let total = base.count().await?;
// les lignes de first_page ont posts_loaded() renseigné.
```

### Pourquoi Suprnova diverge

Le `$query->with(...)` de Laravel clone librement parce que les
tableaux PHP se copient à l'affectation. Rust doit dire ce qu'un clone
signifie pour une closure à type effacé, et jusqu'à la v0.7.2 Suprnova
répondait en abandonnant le plan - le clone réussissait, la requête
réussissait, et les relations étaient simplement absentes. Partager le
prédicat via un `Arc` rend le clone total, au prix de la borne `Fn`
ci-dessus.

Le chargement hâtif à l'intérieur de `chunk` / `chunk_by_id` / `lazy`
reste une erreur explicite plutôt qu'un N+1 silencieux par chunk.
Réappliquez `.with(...)` à l'intérieur de la closure par chunk quand
vous en voulez un.

### Charger sur des collections déjà récupérées

Quand vous récupérez une `Collection<M>` sans plan de chargement
hâtif, vous pouvez en attacher un après coup :

```rust
let mut users = User::query().get().await?;

users.load(["posts"]).await?;                 // sans condition
users.load_missing(["posts.comments"]).await?; // saute ce qui est déjà chargé
```

`load_missing` parcourt le cache `__eager` de chaque parent et ne
déclenche la requête IN que pour les lignes qui n'ont pas déjà chargé
la relation. Utile dans des boucles où certains parents ont été
chargés hâtivement plus tôt dans la requête et d'autres non.

### Exclusion - `without`

`without` retire des relations nommées du plan hâtif, utile quand une
portée de base ajoute des défauts que vous ne voulez pas pour cet
appel :

```rust
let users = User::query()
    .with(["profile", "posts", "team"])
    .without(["team"])     // retire team du plan
    .get()
    .await?;
```

## L'échappatoire

Quand une relation ne correspond à aucun des onze types - arbres
récursifs, polymorphisme à travers des clés non-id, pivots à trois
voies, tout ce qui est sur mesure - écrivez la méthode à la main. La
macro ne l'empêche pas ; vous n'obtenez simplement pas l'accesseur
chargé ni le bras de dispatcher de chargement hâtif pour cette
relation.

```rust
impl User {
    /// Personnalisé : le post le plus récent quelle que soit la forme de la FK.
    pub async fn latest_post(&self) -> Result<Option<Post>, FrameworkError> {
        Post::query()
            .filter("user_id", self.id)
            .latest()
            .first()
            .await
    }
}
```

Le compromis est explicite : les méthodes écrites à la main
n'apparaissent pas dans l'inventaire `relations()`, le moteur
d'existence ne les connaît pas, et le chargeur hâtif ne peut pas les
inclure dans un plan. Pour des cas ponctuels, c'est très bien. Pour
tout ce que vous voudriez faire avec `with(["..."])`, déclarez-le
comme un type de relation à part entière, même si vous devez utiliser
les options de la macro pour le forcer dans cette forme.

## Suivant

- [Eloquent](eloquent.md) - la surface de modèle du quotidien ; la
  syntaxe de déclaration de relation vit là.
- [Base de données](database.md) - connexions, transactions,
  multi-driver, la couche basse sur laquelle tout repose.
- [Migrations](migrations.md) - le côté schéma des colonnes FK dont
  ces relations ont besoin pour exister.
- [Query Builder](eloquent.md#query-builder-dual-api) - la surface à
  double API vers laquelle les wrappers de relation transmettent.
- [Ressources Eloquent](eloquent-resources.md) - transformer des
  relations chargées en payloads JSON:API pour le réseau.

# Générateur de requêtes

Quand vous voulez interroger une table sans la modéliser comme une
struct typée `#[suprnova::model]`, tournez-vous vers `DB::table(name)`.
Elle retourne un builder chaînable à la forme du `Builder<M>` Eloquent
typé, mais matérialise les lignes en `DynamicRow` - un newtype
`serde_json::Map` avec des accesseurs typés. C'est le chapitre pour les
journaux d'audit, les rapports ad hoc, les agrégats de tableau de bord,
et toute table que vous n'avez pas pris la peine de modéliser. Pour
l'équivalent typé, voir [Eloquent](eloquent.md). Pour un `DB::select`
brut à l'intérieur de transactions ou avec l'observation `DB::listen`,
voir [Base de données](database.md).

```rust
use suprnova::DB;

let rows = DB::table("audit_log")
    .select(["id", "event", "actor_id"])
    .filter("actor_id", 42i64)
    .filter_op("created_at", ">=", "2026-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

for row in rows.iter() {
    let id: i64 = row.get_int("id")?;
    let event: String = row.get_string("event")?;
    println!("{id}: {event}");
}
```

## Quand utiliser quelle surface

Trois surfaces de requête se recoupent ; choisissez la bonne pour la
table.

| La table est… | Utilisez | Retourne |
|---|---|---|
| Modélisée avec `#[suprnova::model]` | `Model::query()` → `Builder<M>` | des valeurs typées `M` |
| Non modélisée, mais vous voulez une forme chaînable WHERE/ORDER/LIMIT | `DB::table(name)` → `DbTableBuilder` | `DynamicRow` |
| Tout ce que les builders ne peuvent pas exprimer - CTE, fonctions de fenêtrage, DDL propre au backend | `DB::select` / `DB::statement` / `DB::affecting_statement` | `DynamicRow` / `bool` / `u64` |

`DbTableBuilder` existe pour le cas intermédiaire. Vous obtenez la
chaîne WHERE / ORDER / LIMIT sans vous engager sur une struct
`#[suprnova::model]` et sans redescendre jusqu'à des chaînes SQL
brutes.

## La surface chaînable

`DB::table(name)` retourne un `DbTableBuilder`. Construisez-le, puis
appelez une méthode terminale pour l'exécuter.

### Filtrage

```rust
// Égalité.
DB::table("users").filter("email", "alice@example.com").get().await?;

// Opérateur arbitraire. Allowlist : =, <>, <, <=, >, >=, LIKE, NOT LIKE,
// ILIKE, NOT ILIKE, IS, IS NOT.
DB::table("orders").filter_op("total", ">=", 100i64).get().await?;
DB::table("posts").filter_op("title", "LIKE", "%rust%").get().await?;

// Plusieurs filtres se combinent avec AND.
DB::table("audit_log")
    .filter("actor_id", 42i64)
    .filter_op("event", "<>", "noop")
    .get()
    .await?;
```

`filter` et `filter_op` acceptent tous deux n'importe quel
`Into<SeaValue>` pour le membre de droite, ce qui couvre `i64`,
`String`, `&str`, `bool`, `f64`, `Option<T>`, `chrono::*`,
`uuid::Uuid`, et `serde_json::Value` - chaque type de colonne que le
backend comprend.

### Sélectionner des colonnes

```rust
// Le défaut est SELECT *.
DB::table("users").get().await?;

// Restreindre les colonnes quand vous n'en voulez que quelques-unes.
DB::table("users").select(["id", "email"]).get().await?;
```

### Tri et fenêtrage

```rust
DB::table("posts")
    .order_by_desc("created_at")
    .order_by_asc("title")
    .limit(20)
    .offset(40)
    .get()
    .await?;
```

`order_by_desc` et `order_by_asc` s'enchaînent dans l'ordre
d'insertion ; le SQL généré le préserve.

### Terminaux

```rust
// Toutes les lignes correspondantes.
let rows: Collection<DynamicRow> = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .get()
    .await?;

// Première ligne ou None.
let first: Option<DynamicRow> = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

// Juste le compte (efface tout select/order/limit/offset avant le
// rendu - la sémantique de count ne s'en préoccupe pas).
let n: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;
```

`get()` retourne `Collection<DynamicRow>` - le même wrapper de
collection que les modèles typés utilisent, avec la même surface
`.iter()`, `.len()`, `.into_vec()`. Voir
[Collections Eloquent](eloquent-collections.md).

### Insertions, mises à jour, suppressions

```rust
use suprnova::attrs;

// INSERT, retourne l'id auto-incrémenté de la nouvelle ligne.
let id: i64 = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

// UPDATE, retourne les lignes affectées.
let updated: u64 = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

// DELETE, retourne les lignes affectées.
let deleted: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

La macro `attrs!` construit au site d'appel la map colonne-vers-valeur.
Les clés sont des identifiants SQL (validés) et les valeurs sont liées
comme des paramètres. Une valeur explicitement nulle est émise sous la
forme SQL `NULL`, car la map d'attributs JSON ne conserve plus son type
Rust d'origine ; toutes les valeurs non nulles restent liées comme
paramètres. La même règle s'applique aux écritures en masse Eloquent
typées et aux attributs supplémentaires des pivots plusieurs-à-plusieurs.

#### Alias `update_all` et `delete_all`

`update` et `delete` sont les noms fidèles à Laravel. Les alias à la
manière de `Builder<M>` - `update_all` et `delete_all` - appellent la
même implémentation. Préférez la forme `_all` quand l'intention
« toute la table » est le point du site d'appel ; elle rend visible
pour les relecteurs un `filter` manquant :

```rust
// Même comportement que DB::table("rate_limits").delete().await?
// mais le suffixe _all dit aux relecteurs « oui, je voulais bien
// tronquer la table ».
DB::table("rate_limits").delete_all().await?;

// Mise à jour de masse avec un WHERE - le suffixe _all ici correspond
// à la convention du Builder<M> typé pour la même opération.
DB::table("sessions")
    .filter_op("expires_at", "<", chrono::Utc::now())
    .update_all(attrs! { status: "expired" })
    .await?;
```

#### Un WHERE vide sur update ou delete opère sur chaque ligne

`DB::table("x").delete().await?` supprime chaque ligne de la table.
C'est pris en charge par conception - parfois vous voulez vraiment
tronquer - mais c'est rarement correct. Regardez toujours un appel
`delete()` / `delete_all()` et vérifiez s'il y a un `filter` devant.
C'est aussi vrai pour `update` / `update_all`.

#### Séparation par backend pour l'insertion

`RETURNING id` est utilisé sur Postgres et SQLite. MySQL ne prend pas
en charge `RETURNING`, donc le builder exécute l'INSERT et lit le
`last_insert_id()` par connexion du driver depuis le résultat. Le
builder sans modèle suppose une clé primaire auto-incrémentée `id`
standard. Les clés primaires UUID, composites, renommées, ou non
entières ne sont pas prises en charge sur cette surface - utilisez
plutôt l'interface `Model` typée d'[Eloquent](eloquent.md), qui
consulte la définition du modèle pour la forme de la clé primaire.

## `DynamicRow` - accesseurs typés sur une map JSON

Chaque ligne retournée par `DB::table` ou `DB::select` se matérialise
en `DynamicRow`, un newtype `serde_json::Map<String, Value>` avec des
accesseurs typés. Chaque getter retourne `Result<T, FrameworkError>`
avec un message d'erreur clair en cas de clé absente ou de type
incompatible.

```rust
for row in rows.iter() {
    let id: i64                 = row.get_int("id")?;
    let event: String           = row.get_string("event")?;
    let active: bool            = row.get_bool("active")?;
    let weight: f64             = row.get_float("weight")?;
    let payload: serde_json::Value = row.get_value("payload")?;
}
```

Pour les colonnes nullables, utilisez `get_optional_*`. Ceux-ci
distinguent « colonne absente » (erreur - schéma incohérent) de
« colonne présente, valeur SQL NULL » (`Ok(None)`) :

```rust
let title: Option<String> = row.get_optional_string("title")?;
let score: Option<i64>    = row.get_optional_int("score")?;
```

Aujourd'hui, la famille optional couvre `String` et `i64`. Pour les
autres types nullables, utilisez `get_value` et faites vous-même
correspondre `serde_json::Value::Null`, ou lisez la colonne via
`get_as::<Option<T>>` (n'importe quel `T: DeserializeOwned`).

Pour désérialiser une colonne vers n'importe quelle struct ou type
conteneur, utilisez `get_as`. Toute la surface de désérialisation de
`serde_json` est disponible :

```rust
#[derive(serde::Deserialize)]
struct UserPrefs {
    theme: String,
    notifications: bool,
}

let prefs: UserPrefs    = row.get_as("prefs")?;
let tags: Vec<String>   = row.get_as("tags")?;
let when: chrono::DateTime<chrono::Utc> = row.get_as("created_at")?;
```

`DynamicRow` fait un deref vers `Map<String, Value>`, donc l'itération
et les vérifications d'existence de clé fonctionnent directement :

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}

if row.contains_key("deleted_at") { /* … */ }
```

## Frontière de confiance des identifiants

Les noms de table, les noms de colonne, les directions ORDER BY, et
les opérateurs SQL sont interpolés tels quels dans la chaîne SQL - ils
ne sont PAS liés comme des paramètres (SQL n'autorise pas les
identifiants liés par placeholder). Traitez chaque argument
`impl Into<String>` comme un littéral de confiance, fixé à la
compilation.

```rust
// Sûr - le nom de colonne est une constante ; la valeur est liée.
DB::table("users").filter("email", request.email()).get().await?;

// DANGEREUX - n'épissez jamais une entrée utilisateur dans un nom de
// colonne.
DB::table("users")
    .filter(request.user_supplied_column(), value)
    .get()
    .await?;
```

Le framework impose une allowlist stricte à la frontière d'E/S - les
identifiants doivent correspondre à `[A-Za-z_][A-Za-z0-9_]*` avec un
préfixe `schema.` optionnel, et les opérateurs doivent venir d'une
liste fixe. Les violations échouent de manière fermée avec une
`FrameworkError::Database` avant que le moindre SQL ne soit rendu.
C'est un garde-fou, pas une licence : gardez vos identifiants
littéraux dans votre code.

Les valeurs du membre de droite de `filter` / `filter_op` sont
toujours liées comme des paramètres et sûres à épisser depuis les
données de requête.

## Requêtes brutes

Quand le builder ne peut pas exprimer ce dont vous avez besoin - CTE
récursives, fonctions de fenêtrage, DDL propre au backend,
`INSERT … ON CONFLICT DO UPDATE` - redescendez vers une chaîne brute.
Les placeholders correspondent au backend actif (`$1, $2, …` pour
Postgres, `?` pour MySQL et SQLite) ; le framework le détecte
automatiquement depuis `DatabaseConfig::url`.

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - chaque ligne en DynamicRow.
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) AS post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING COUNT(p.id) > ?",
    vec![Value::from(5i64)],
).await?;

// SELECT - première ligne seulement, à l'image du DB::selectOne de
// Laravel.
let alice = DB::select_one(
    "SELECT * FROM users WHERE email = ?",
    vec![Value::from("alice@example.com")],
).await?;

// SELECT - première colonne de la première ligne comme scalaire typé.
let total: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// INSERT - true quand au moins une ligne a été affectée.
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - retournent le compte de lignes affectées.
let updated: u64 = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1i64)],
).await?;

let deleted: u64 = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// N'importe quelle instruction préparée avec des liaisons.
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1i64), Value::from(42i64)],
).await?;

// DDL ou autres instructions sans liaison qui rejettent la liaison de
// placeholder.
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// Chemin générique « lignes affectées » - pour les upserts et les
// opérations qui n'entrent dans aucun des helpers nommés.
let n: u64 = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1)
     ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec![Value::from("page_views")],
).await?;
```

### Le piège des colonnes agrégat

Les agrégats non typés comme `SELECT COUNT(*) AS n FROM t`
fonctionnent via le helper `.count()` du builder mais peuvent revenir
silencieusement absents des lignes d'un `DB::select` brut sur SQLite.
Le matérialiseur de lignes sous-jacent parcourt les informations de
type par colonne de sqlx, et un agrégat nu n'en porte aucune. Si vous
avez besoin d'un `DB::select` brut avec des agrégats sur SQLite, soit
enveloppez l'expression dans `CAST(… AS BIGINT)` pour lui donner une
étiquette de type, soit utilisez `DB::scalar::<i64>`, qui passe par
`query_one` + `try_get` et ne dépend pas de la détection de type par
colonne.

## Pont vers Eloquent typé

Quand la table mérite une struct `#[suprnova::model]`, la forme
chaînable se reporte telle quelle. `Model::query()` retourne
`Builder<M>`, qui offre la même surface `filter` / `filter_op` /
`order_by_*` / `limit` / `offset` / `get` / `first` / `count` - plus
un vocabulaire WHERE bien plus large (`filter_in`, `filter_between`,
`filter_null`, `filter_has`, `filter_raw`, …) et des alias à la forme
Laravel (`db_where`, `where_in`, `where_between`, `where_null`,
`where_has`, `where_raw`, …).

```rust
use suprnova::Model;

let admins = User::query()
    .filter("role", "admin")
    .filter_op("created_at", ">=", since)
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;     // Collection<User> - typé, pas DynamicRow

let alice = User::query().filter("email", &email).first().await?;
let total = User::query().filter("active", true).count().await?;
// Remarque : Builder<M>::count retourne i64 (à l'image de l'Eloquent
// de Laravel), tandis que DbTableBuilder::count retourne u64. Les deux
// surfaces vous donnent un COUNT SQL non négatif - elles ne diffèrent
// que par leur type réseau.
```

La surface complète de `Builder<M>` - chaque forme WHERE, les
agrégats, les relations, le chargement hâtif, les scopes, les
paginateurs, l'itération par chunk - se trouve dans
[Eloquent](eloquent.md). La forme chaînable apprise ci-dessus est la
même forme ; les différences portent sur le typage et la portée.

## Routage vers une connexion nommée

`DB::table` et les helpers bruts ciblent par défaut la connexion
primaire. Pour viser une réplique de lecture, un shard, ou un pool
d'entrepôt, épinglez l'appel :

```rust
// Builder épinglé à une connexion nommée.
let rows = DB::table("audit_log").on("warehouse").get().await?;

// Raccourci équivalent.
let rows = DB::table_on("warehouse", "audit_log").get().await?;

// Les escapes bruts ont aussi des variantes _on.
let rows = DB::select_on("warehouse", "SELECT …", vec![]).await?;
let n    = DB::affecting_statement_on(
    "warehouse",
    "UPDATE …",
    vec![],
).await?;
```

Quand `__read_replica__` est enregistrée, chaque terminal de forme
lecture s'y route automatiquement ; les écritures (`insert` /
`update` / `delete` / `update_all` / `delete_all`) ciblent toujours la
primaire. À l'intérieur d'une closure `DB::transaction`, la connexion
de la transaction active l'emporte absolument - `on(name)` est
silencieusement ignoré pour préserver l'atomicité. Voir
[Base de données - Connexions nommées](database.md) pour la chaîne de
priorité complète.

### Pourquoi Suprnova diverge

Le `DB::table(...)` de Laravel est son générateur de requêtes sans
modèle ; en dessous, il retourne un `stdClass` par ligne (un objet PHP
dont les propriétés sont les colonnes). Suprnova retourne `DynamicRow`
à la place - un newtype `serde_json::Map` avec des accesseurs typés.
La forme de l'accesseur attrape les erreurs de colonne absente et de
mauvais type à la frontière plutôt que de paniquer au fond du code
utilisateur avec une exception d'accès à une propriété.

Les doubles noms `update`/`update_all` et `delete`/`delete_all`
existent parce que la surface `Builder<M>` typée d'Eloquent utilise le
suffixe `_all` pour rendre explicite au site d'appel l'intention
« toute la table ». Plutôt que de choisir un camp, le builder sans
modèle livre les deux - `update` et `delete` correspondent au
`DB::table($t)->update(...)` et `->delete()` de Laravel à la lettre ;
`update_all` et `delete_all` correspondent à la convention que les
utilisateurs de `M` ont déjà dans les doigts.

## Suivant

- [Base de données](database.md) - façade `DB`, transactions avec
  savepoints, observabilité `DB::listen`, connexions nommées
- [Eloquent](eloquent.md) - structs `#[suprnova::model]` typées et la
  surface complète de `Builder<M>`
- [Pagination](pagination.md) - `paginate` / `simple_paginate` /
  `cursor_paginate` sur les builders typés
- [Collections Eloquent](eloquent-collections.md) - la `Collection<T>`
  retournée par `get()` sur les deux surfaces
- [Migrations](migrations.md) - définir le schéma que les builders
  interrogent

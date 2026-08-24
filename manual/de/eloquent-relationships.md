# Eloquent Relationships

[Eloquent](eloquent.md) behandelt die alltägliche
Relations-Oberfläche - Deklarationssyntax, die Options-Tabelle,
grundlegendes Pro-Art-Verketten. Dieses Kapitel ist der
relationsspezifische Deep Dive: wie ein Aufruf `user.posts()`
tatsächlich zu SQL aufgelöst wird, wie der Eager Loader N+1
vermeidet, wie die Existenz-Engine (`has` / `where_has` /
`where_belongs_to`) korrelierte `EXISTS`-Subqueries rendert, wie
Polymorphismus Rusts fehlende späte statische Bindung überlebt, und
was aus dem Typsystem herausfällt, wenn alle elf Relations-Arten auf
einem Trait koexistieren müssen.

Wenn Sie neu bei Eloquent auf Suprnova sind, lesen Sie zuerst
[Eloquent](eloquent.md#relationships) - jene Seite lehrt die
Deklarationssyntax. Diese Seite setzt voraus, dass Sie bereits ein
Modell mit einem `relations = { ... }`-Block haben und verstehen
wollen, was darunter passiert.

## Die elf Relation-Arten

Jede Relations-Art in [`RelationKind`][relations] ist eine von:

| Art                   | Seite      | Kardinalität | Familienübergreifend | Pivot |
|-----------------------|------------|-------------|-----------------|-------|
| `HasOne<R>`           | Eltern     | eins        | nein            | - |
| `HasMany<R>`          | Eltern     | viele       | nein            | - |
| `BelongsTo<R>`        | Kind       | eins        | nein            | - |
| `BelongsToMany<R, P>` | beide      | viele       | nein            | ja    |
| `HasOneThrough<B, R>` | Eltern     | eins        | nein            | - |
| `HasManyThrough<B, R>`| Eltern     | viele       | nein            | - |
| `MorphOne<R>`         | Eltern     | eins        | ja              | - |
| `MorphMany<R>`        | Eltern     | viele       | ja              | - |
| `MorphTo`             | Kind       | eins        | ja (n Ziele)    | - |
| `MorphToMany<R, P>`   | Eltern     | viele       | ja              | ja    |
| `MorphedByMany<R, P>` | m2m-Partner| viele       | ja (invers)     | ja    |

„Familienübergreifend“ bedeutet, dass der *Typ* der verknüpften
Zeile variiert - ein `Comment` könnte zu einem `Post` oder einem
`Video` gehören, nicht nur zu einer festen Eltern-Tabelle. Das ist
Polymorphismus, und Suprnova handhabt ihn über die
[Morph-Registry](#die-morph-registry) plus ein familienspezifisches
Enum.

[relations]: https://docs.rs/suprnova

### Was das Makro generiert

Wenn Sie schreiben:

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

expandiert `#[suprnova::model]` zu fünf Dingen für `posts`:

1. **Relations-Methode** - `fn posts(&self) -> HasMany<Self, Post>`.
   Gibt einen lazy Wrapper zurück, der `self.id` plus FK-Metadaten
   trägt; noch läuft kein SQL.
2. **Loaded-Accessor** - `fn posts_loaded(&self) -> &[Post]`. Liest
   aus dem Eager-Cache nach `User::with(["posts"])`. Leerer Slice,
   wenn kein Eager Load lief.
3. **Count-Accessor** - `fn posts_count(&self) -> u64`. Liest aus
   demselben Cache nach `User::with_count(["posts"])`.
4. **Dispatcher-Arm** - Match-Arm in der inhärenten Methode
   `__eager_load` des Modells. Der Eager Loader schlägt `"posts"`
   nach und führt die `IN`-Query aus.
5. **Inventory-Eintrag** - ein `inventory::submit!(RelationEntry {
   ... })`, sodass die Relation zur Laufzeit aufzählbar ist
   (Admin-Tooling, die Existenz-Engine, der Morph-Dispatcher laufen
   das alle ab).

Sie sehen (4) oder (5) nie. Sie treiben den Rest dieses Kapitels an.

## Lazy-Auflösung: wie `user.posts()` zu SQL wird

`user.posts()` gibt einen Wrapper `HasMany<User, Post>` zurück, kein
Query-Ergebnis. Der Wrapper hält den PK-Wert des Elternteils plus den
FK-Spaltennamen, und einen vorgefilterten `Builder<Post>` mit bereits
angewendetem `WHERE posts.user_id = ?`. Noch hat nichts die Datenbank
berührt.

```rust
use suprnova::Direction;

// Kein SQL.
let posts_q = user.posts();

// SQL: SELECT * FROM posts WHERE user_id = ? ORDER BY id DESC LIMIT 5
let recent = user.posts()
    .order_by("id", Direction::Desc)
    .limit(5)
    .get()
    .await?;

// SQL: SELECT COUNT(*) FROM posts WHERE user_id = ?
let n = user.posts().count().await?;
```

Die Dual-API-Oberfläche ([Eloquent → Namenshinweis](eloquent.md#naming-note-dual-api))
wird auf dem Wrapper respektiert: sowohl `.filter("col", v)` als auch
`.db_where("col", v)` funktionieren, identisch. Die verkettbare
Oberfläche auf `HasOne` / `HasMany` / `MorphOne` / `MorphMany` deckt
`filter` / `db_where` / `order_by` / `latest` / `oldest` / `limit` /
`take` ab. Through-Relationen und polymorphe m2m-Relationen legen
nur ihre terminalen Methoden frei - sie laufen über handgeschriebene
SQL-Stiche, nicht über einen `Builder<R>`, sodass sie sich nicht mit
der Standard-Kette komponieren lassen. Siehe [Through-Relationen](#hasonethrough-und-hasmanythrough)
und [Polymorphe m2m](#morphtomany-und-morphedbymany) unten.

### Soft Deletes wirken durch

Wenn der verknüpfte Typ [`SoftDeletes`](eloquent.md#soft-deletes-flag)
implementiert, erbt der Relations-Wrapper dessen globalen Scope.
`user.posts().get()` versteckt gelöschte Posts genauso, wie
`Post::query().get()` es tut. Drei Durchreicher durchbrechen das:

```rust
let alive = user.posts().get().await?;                 // Standard: nur lebende
let all = user.posts().with_trashed().get().await?;    // lebende + gelöschte
let dead = user.posts().only_trashed().get().await?;   // nur gelöschte
```

`with_trashed` / `only_trashed` existieren auf `HasOne`, `HasMany`,
`MorphOne`, `MorphMany`, `BelongsToMany`, `MorphToMany`,
`MorphedByMany` und `BelongsTo`. Sie fehlen absichtlich auf
`HasOneThrough` und `HasManyThrough` - siehe die
[Through-Soft-Delete-Lücke](#soft-deletes-bei-through-relationen-v1)
unten.

## Eins-zu-eins: `HasOne` und `BelongsTo`

`HasOne` ist das Elternteil, das sagt „dieses Kind hat eine Spalte,
die auf mich zeigt“. `BelongsTo` ist das Kind, das sagt „ich habe
eine Spalte, die auf das Elternteil zeigt“. Beide führen ein
einzelnes `WHERE fk = ? LIMIT 1` aus und geben `Option<R>` zurück.

```rust
// HasOne - Eltern → Kind
let profile: Option<Profile> = user.profile().first().await?;

// BelongsTo - Kind → Eltern
let owner: Option<User> = profile.user().first().await?;
```

`BelongsTo` fügt eine Laravel-förmige Annehmlichkeit hinzu, die die
anderen nicht brauchen: `with_default`. Wenn der FK des Kindes null
ist ODER die Eltern-Zeile gelöscht wurde, gibt `first()` den
Platzhalter der Closure zurück statt `None`:

```rust
#[model(table = "comments", relations = {
    author: BelongsTo<User> {
        with_default = || User { id: 0, name: "Guest".into(), .. },
    },
})]
pub struct Comment { /* ... */ }

// Gibt immer Some(User) zurück - entweder den echten Autor oder den Guest-Stub.
let display: Option<User> = comment.author().first().await?;
```

Der Eager-Load-Dispatcher respektiert denselben Fallback - Lazy- und
Eager-Pfade teilen das Standardverhalten, sodass Template-Code, der
`comment.author_loaded()[0].name` ausgibt, nicht verzweigen muss.

## Eins-zu-viele: `HasMany`

`HasMany` ist die Eltern-seitige Relation mit
Viele-Kardinalität. Das Terminal `.get()` gibt eine
[`Collection<R>`](eloquent.md#collections) zurück - den
Laravel-förmigen Wrapper um `Vec<R>` - sodass sich die
modellbewusste Oberfläche komponiert:

```rust
let titles = user.posts()
    .order_by("created_at", Direction::Desc)
    .limit(10)
    .get()
    .await?
    .pluck::<String>("title");
```

`latest()` und `oldest()` sind Zucker für
`order_by("created_at", Direction::Desc)` beziehungsweise `Asc` -
sie lösen sich nur gegen Modelle auf, die eine Spalte `created_at`
deklarieren, die das Makro `#[suprnova::model]` automatisch
hinzufügt, wann immer Timestamps aktiv sind (der Standard).

## Viele-zu-viele: `BelongsToMany<R, P>` und das erstklassige Pivot

`BelongsToMany` ist Viele-zu-viele über eine Join-Tabelle. Suprnovas
Pivot ist selbst eine `#[suprnova::model]`-Struktur mit eigenen
Migrationen, eigenen Accessoren, eigenen Events. Das ist die
Abweichung - siehe [unten](#warum-suprnova-abweicht-pivot-ist-ein-echtes-modell).

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

Mutatoren laufen gegen die Pivot-Zeile:

```rust
use suprnova::attrs;

user.roles().attach(role.id).await?;
user.roles().attach_with(role.id, attrs! { assigned_at: now }).await?;
user.roles().detach(role.id).await?;
user.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
```

`sync` liest die aktuelle Pivot-Menge, berechnet
`attach_set = ids - current` und `detach_set = current - ids`, und
führt die Deltas innerhalb einer Transaction aus. Duplikate in der
Eingabemenge fallen anhand ihrer JSON-String-Form zusammen, sodass
`sync([1, 1, 2])` das tut, was Sie meinen.

Das Lesen läuft über die Zwei-Query-Strategie:

```rust
// Query 1: SELECT roles.*, role_user.* via INNER JOIN, gescoped nach user_id.
// Query 2: SELECT role_user.* für denselben Join, um __pivot pro Zeile zu stempeln.
let roles = user.roles().get().await?;

// Jede Rolle trägt den Pivot-Kontext, den das Makro zugänglich gemacht hat:
for r in &roles {
    let pivot = r.pivot::<RoleUser>().expect("loaded via BelongsToMany");
    println!("{} assigned at {:?}", r.name, pivot.assigned_at);
}
```

### Warum Suprnova abweicht: Pivot ist ein echtes Modell

Laravels Pivot ist ein undurchsichtiger Pro-Attribut-Bag
(`$role->pivot->note`). Suprnova verlangt, dass Sie die
Pivot-Struktur deklarieren, weil Rusts Typsystem die Spalten zur
Compile-Zeit braucht - und sobald Sie für diese Deklaration bezahlt
haben, bekommt das Pivot dieselbe `#[suprnova::model]`-Behandlung wie
jede andere Tabelle: Migrationen, Events, Observer, Factories,
Soft-Delete. `r.pivot::<RoleUser>()` gibt eine typisierte Referenz
zurück; keine string-geschlüsselten Attribut-Lookups, keine
Überraschungen zur Laufzeit, wenn eine Spalte falsch geschrieben ist.

Die Kosten sind eine zusätzliche Struktur pro Pivot-Tabelle. Der
Nutzen ist, dass das Pivot Verhalten tragen kann - Domänenlogik,
Validierungsregeln, Audit-Spalten - ohne in rohes SQL auszubrechen.

## `HasOneThrough` und `HasManyThrough`

Zwei-Hop-Relationen: `A → B → C`, wobei `B` ein Zwischenmodell ist,
dessen FK auf `A` zeigt, und `C` das endgültige Ziel ist, dessen FK
auf `B` zeigt. Klassisches Beispiel: `Country` hat viele `User`s;
`User` hat viele `Post`s; `Country::posts()` springt beide Hops in
einem SQL-Roundtrip.

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<User, Post>,
})]
pub struct Country { /* ... */ }

// Ein einzelner INNER JOIN: SELECT posts.* FROM posts
//   INNER JOIN users ON posts.user_id = users.id
//   WHERE users.country_id = ?
let posts: Collection<Post> = country.posts().get().await?;
```

`HasOneThrough` hat dieselbe Form, aber `.get()` gibt `Option<C>`
zurück (passend zur Eins-Kardinalitäts-Semantik), und `.first()` ist
sein Alias.

Through-Wrapper legen nur ihre Terminals frei - `get` / `first` /
`count` plus die Key-Setter (`first_key` / `second_key` / `local_key`
/ `second_local_key`). Sie fließen nicht durch einen `Builder<C>`,
können also `.filter(...)` oder `.order_by(...)` nicht verketten.
Wenn Sie über den Join hinweg filtern müssen, greifen Sie auf zwei
explizite Relations-Hops zurück.

### Soft Deletes bei Through-Relationen (v1)

Through-Relationen verwenden rohes `INNER JOIN`-SQL statt der
`Builder<C>`-Pipeline, sodass der globale Soft-Delete-Scope, den
`C::query()` installieren würde (`WHERE c.deleted_at IS NULL`),
**nicht** angewendet wird. Sowohl gelöschte Zwischenmodelle als auch
gelöschte Ziele nehmen am JOIN teil.

Das weicht von Laravel ab, wo `hasManyThrough` sowohl `B` als auch
`C` nach `deleted_at IS NULL` filtert, wenn die Modelle `SoftDeletes`
deklarieren. Bis der Fix landet, sollten Aufrufer, die gescopte
Through-Lesevorgänge brauchen, die beiden Relationen explizit
verketten:

```rust
// Statt country.posts().get():
let users = country.users().get().await?;
let user_ids: Vec<i64> = users.iter().map(|u| u.id).collect();
let posts = Post::query().filter_in("user_id", user_ids).get().await?;
// Sowohl der Soft-Delete-Scope von User als auch von Post greifen.
```

## Polymorphe Relationen

Ein polymorpher FK ist ein Spaltenpaar: `<name>_id` (der
Primärschlüssel der Zeile) plus `<name>_type` (ein String, der
identifiziert, *in welcher Tabelle* die id lebt). Eine
`Comment`-Zeile kann auf einen `Post` oder ein `Video` zeigen, ohne
eine `post_id`- oder `video_id`-Spalte hinzuzufügen.

Suprnova liefert vier polymorphe Arten: `MorphOne`, `MorphMany`,
`MorphTo`, und das m2m-Paar `MorphToMany` / `MorphedByMany`. Sie
teilen sich alle ein Stück Infrastruktur: [die
Morph-Registry](#die-morph-registry).

### `MorphOne<R>` und `MorphMany<R>` - Elternseite

`MorphOne` und `MorphMany` spiegeln `HasOne` und `HasMany`, legen
aber den Diskriminator `<name>_type` darüber. Der innere Builder ist
vorgefiltert mit `WHERE <name>_id = ? AND <name>_type = ?`, sodass
polymorphe Kinder, die auf *andere* Familien zeigen, nie im Ergebnis
erscheinen.

```rust
#[model(table = "posts", morph_type = "post", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Video { /* ... */ }

let post_comments = post.comments().get().await?;     // nur commentable_type = 'post'
let video_comments = video.comments().get().await?;   // nur commentable_type = 'video'
```

`morph_type = "post"` ist der String, den das Elternteil in der
Spalte `commentable_type` des Kindes registriert. Standard ist der
snake-case-Name der Struktur, aber ein Override ist der richtige Zug
für jedes Modell, das Sie ausliefern - Tabellen-Umbenennungs-Refactors
sollten den polymorphen Schlüssel nicht brechen.

### `MorphTo` und das familienspezifische Enum

`MorphTo` lebt auf der Morph-Tabellen-Seite. Der Nutzer deklariert
die *`targets`-Liste* im Voraus:

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

Das Makro generiert an der Deklarationsstelle ein
familienspezifisches Enum:

```rust
// Vom Makro generiert - das schreiben Sie nicht selbst.
pub enum CommentableMorph {
    Post(Post),
    Video(Video),
    Unknown(String, i64),     // Fallback für nicht registrierte <name>_type
}
```

Und `comment.commentable()` gibt einen Fetch-Helfer zurück, dessen
`.get()` zum Enum aufgelöst wird:

```rust
match comment.commentable().get().await? {
    CommentableMorph::Post(post) => println!("on post: {}", post.title),
    CommentableMorph::Video(video) => println!("on video: {}", video.url),
    CommentableMorph::Unknown(t, id) => {
        eprintln!("orphaned commentable_type={t} id={id}");
    }
}
```

### Warum Suprnova abweicht: familienspezifisches Enum

Laravels `morphTo` gibt `mixed` zurück - PHPs dynamischer Dispatch
löst die Methode zur Laufzeit auf. Rust hat keine späte statische
Bindung, also macht Suprnova die Familie explizit. Die Vorteile
schlagen die Typisierungskosten:

- **Erschöpfendes `match`** - der Compiler sagt Ihnen, wenn ein neues
  Morph-Ziel landet und Sie vergessen haben, es zu behandeln.
- **`Unknown(String, id)` ist typsicher** - verwaiste Zeilen von
  einer entfernten Eltern-Modellklasse werden als Variante
  sichtbar, nicht wegpanickt.
- **Die `targets`-Liste dokumentiert das Schema** - das Lesen der
  `MorphTo`-Deklaration sagt Ihnen jeden Typ, der am anderen Ende
  sitzen kann. Keine Datenbankabfrage nötig, um sie aufzuzählen.

### v1-Einschränkung: `MorphTo` ist nur `i64`

`MorphTo::morph_id` ist fest auf `i64` verdrahtet. Polymorphe Ziele
müssen daher `i64`-Primärschlüssel verwenden, und die Spalte
`<name>_id` der Morph-Tabelle muss ebenfalls `i64` sein. Modelle,
deren PK `String` oder `Uuid`-über-String ist, können in v1 keine
`MorphTo`-Ziele sein. v2 wird den Morph-ID-Typ parametrisieren,
sodass das vollständige PK-Spektrum (`i64` / `String` / `Uuid`)
akzeptiert wird.

Das ist eine Einschränkung nur für die polymorphe Inverse.
`MorphOne` / `MorphMany` / `MorphToMany` / `MorphedByMany`
funktionieren mit jeder PK-Form einwandfrei - sie lesen die bereits
typisierte `id` des Elternteils direkt.

### `MorphToMany` und `MorphedByMany`

Polymorphe Viele-zu-viele-Relation über ein einziges Pivot. Eine
Seite ist „morphable“ (`Post.tags()`, `Video.tags()` - beide laufen
über dasselbe `taggables`-Pivot). Die andere ist der gemeinsame
m2m-Partner (`Tag.posts()`, `Tag.videos()` - dasselbe Pivot, in die
andere Richtung gescannt).

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

`MorphToMany` ist die mutierende Seite - `attach` / `attach_with` /
`detach` / `sync` leben alle dort. `MorphedByMany` ist nur lesbar:
jeder Aufruf `tag.posts()` gibt nur `Post`-typisierte Taggables
zurück, jeder `tag.videos()` gibt nur `Video`-typisierte Taggables
zurück, keine Mischung in einer Collection.

Mutieren Sie von der morphable Seite:

```rust
post.tags().attach(rust_tag.id).await?;
post.tags().sync([rust_tag.id, async_tag.id]).await?;
```

Lesen Sie von beiden:

```rust
let tags_on_post: Collection<Tag> = post.tags().get().await?;
let posts_with_rust_tag: Collection<Post> = rust_tag.posts().get().await?;
```

## Die Morph-Registry

Jede mit `#[suprnova::model(morph_type = "...")]` annotierte Struktur
gibt zur Compile-Zeit einen [`MorphTypeEntry`][morph] über
`inventory::submit!` aus. Die Registry treibt drei Dinge an:

1. **Familienspezifischer Enum-Dispatch** - `MorphTo.get()` liest den
   String `<name>_type` der Kind-Zeile und schlägt ihn nach, um die
   richtige Enum-Variante zu finden.
2. **`MorphedByMany`-Ziel-Filterung** - `target_morph_type = "post"`
   löst über die Registry auf, um sicherzustellen, dass der
   Typ-String echt ist.
3. **Plausibilitätsprüfungen** - `find_morph_type("post")` gibt
   `None` zurück, wenn sich kein Modell mit diesem String
   registriert hat, und unterscheidet "absichtlich nicht
   registriert" von "Tippfehler".

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

Modelle ohne ein Attribut `morph_type = "..."` registrieren sich
absichtlich nicht - die Registry ist Opt-in. Ein nicht-polymorphes
`User`-Modell trägt nichts zu ihr bei, was `find_morph_type("user")`,
das `None` zurückgibt, zu einem nützlichen Signal macht.

## Abfragen nach Relations-Existenz

`has` / `where_has` / `doesnt_have` / `where_relation` /
`where_belongs_to` bilden Suprnovas Existenz-Engine für Relationen.
Sie rendern alle als korrelierte `EXISTS (...)`-Subqueries gegen das
**eigene SELECT des Elternteils** - kein JOIN, keine doppelten
Eltern-Zeilen, kein GROUP BY.

```rust
// User mit mindestens einem Post.
let with_posts = User::query().has("posts").get().await?;

// User mit mindestens drei Posts.
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// User mit mindestens einem VERÖFFENTLICHTEN Post.
let published_authors = User::query()
    .where_has::<Post, _>("posts", |q| q.filter("published", true))
    .get()
    .await?;

// User ohne JEGLICHE Posts.
let empty_users = User::query().doesnt_have("posts").get().await?;

// User ohne ENTWURFS-Posts (sie können trotzdem veröffentlichte haben).
let clean = User::query()
    .where_doesnt_have::<Post, _>("posts", |q| q.filter("published", false))
    .get()
    .await?;

// Kurzform: where_has + eine Spalte == Match.
let same = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// where_belongs_to - direktes FK = ? auf DIESER Tabelle (kein EXISTS
// nötig, weil der FK auf der Kind-Zeile liegt).
let mine = Post::query()
    .where_belongs_to("author", user.id)
    .get()
    .await?;
```

### Wie es funktioniert

Die Engine durchläuft das Relations-Inventory zur
Query-Bauzeit. Für jede benannte Relation zieht sie den
`RelationEntry` und rendert die passende SQL-Form pro Art:

- `HasOne` / `HasMany` / `MorphOne` / `MorphMany` →
  `EXISTS (SELECT 1 FROM child WHERE child.<fk> = parent.<pk>)`.
  Morph-Arten fügen `AND child.<name>_type = '<parent_morph_type>'`
  hinzu.
- `BelongsTo` →
  `EXISTS (SELECT 1 FROM parent WHERE parent.<pk> = child.<fk>)`.
- `BelongsToMany` / `MorphToMany` → verbindet über das Pivot:
  `EXISTS (SELECT 1 FROM pivot WHERE pivot.<parent_fk> = parent.<pk> ...)`.
- Through-Relationen → verbinden über das Zwischenmodell.

Die Closure-Form (`where_has::<R, _>(rel, |q| ...)`) baut einen
inneren `Builder<R>`; welche WHERE-Terme dieser Builder auch immer
erzeugt, landen im Body der Subquery. Die Platzhalter-Nummerierung
ist über das gesamte Statement monoton, sodass die Engine mit
Postgres-Parametern im `$1`-Stil korrekt funktioniert.

`where_belongs_to` ist die eine Ausnahme, die kein EXISTS rendert.
Der Belongs-to-Fremdschlüssel lebt auf der *eigenen* Zeile des
Elternteils, sodass ein direktes `WHERE child.<fk> = ?` genau das
richtige SQL ist - keine Subquery nötig. Ist der Relationsname dem
Inventory des Elternteils unbekannt, gibt die Engine `WHERE 1 = 0`
aus, sodass die Query sicher nichts zurückgibt.

### Warum das besser ist als LEFT JOIN

Laravels ältere `has` / `whereHas`-Engine gab früher JOINs und
doppelte Eltern-Zeilen aus; die Umstellung auf korreliertes EXISTS
landete in Laravel
9. Suprnova liefert EXISTS von Tag eins. Die
Vorteile: keine Duplikate in der Ergebnismenge, keine
GROUP-BY-Workarounds für Aggregate, kein Bedarf an `DISTINCT`, und
der Optimizer der Datenbank sieht eine echte Subquery statt eines
JOIN, durch den er keine Prädikate durchschieben kann. Für
`has_count(rel, ">=", n)` rendert die Engine direkt
`(SELECT COUNT(*) FROM child WHERE ...) >= n` - eine Query, ein Plan.

## Eager Loading - `with`, `with_count`, `with_*`-Aggregate

Das lazy `user.posts().get()` macht eine Query pro Elternteil. Das
ist N+1, wenn Sie viele User haben:

```rust
// Schlecht: 1 Query für User + 100 Queries für Posts.
let users = User::query().limit(100).get().await?;
for u in &users {
    let posts = u.posts().get().await?;
    /* ... */
}
```

`with(["posts"])` kollabiert das auf zwei Queries insgesamt -
unabhängig von der Elternanzahl:

```rust
// Gut: 1 Query für User + 1 IN-Query für alle Posts.
let users = User::query()
    .with(["posts"])
    .limit(100)
    .get()
    .await?;

for u in &users {
    for post in u.posts_loaded() {       // liest aus dem Cache, kein SQL
        println!("{}: {}", u.name, post.title);
    }
}
```

Verschachtelte Pfade funktionieren auch - punktgetrennte
Relationsnamen rekursieren:

```rust
let users = User::query()
    .with(["posts.comments.author"])
    .get()
    .await?;
// 4 Queries: users, posts IN users.id, comments IN posts.id, authors IN comments.user_id.
```

### `with_count` und Aggregate

`with_count` fügt ein Pro-Relations-Aggregat `COUNT(*) GROUP BY
parent_fk` hinzu, das neben den Eltern geladen wird - eine
zusätzliche Query pro Relation:

```rust
let users = User::query().with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}
```

Vier Aggregat-Varianten stapeln sich: `with_sum`, `with_avg`,
`with_min`, `with_max`. Die Cache-Key-Form ist
`<rel>_<kind>_<col>`, sodass das Stapeln mehrerer Aggregate auf
derselben Relation nicht kollidiert:

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

Siehe [Eloquent → Eager Loading → Cache-Layout](eloquent.md#cache-layout)
für den vollständigen Storage-Vertrag.

### Eingeschränktes Eager Loading - `with_where`

`with_where` filtert, welche Kind-Zeilen im Eager-Cache landen, ohne
Eltern zu verlieren, die keine passenden Kinder haben:

```rust
use suprnova::Builder;

let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// Jedes u.posts_loaded() enthält nur veröffentlichte Posts.
// User mit null veröffentlichten Posts erscheinen trotzdem in der
// Ergebnismenge - ihr posts_loaded() gibt einen leeren Slice zurück.
```

`with_where` unterscheidet sich von `where_has` in der Absicht:
`where_has` filtert die Eltern-Menge ("User, die mindestens einen
veröffentlichten Post haben"); `with_where` filtert den Eager-Cache
("für alle User, lade nur ihre veröffentlichten Posts"). Verwenden
Sie beide zusammen, wenn Sie beide Effekte wollen.

Das Prädikat ist ein `Fn`, keine `FnOnce`, sodass ein Builder, der
eines trägt, geklont und mehr als einmal ausgeführt werden kann. Eine
Closure, die einen erfassten Wert konsumieren will, sollte ihn intern
klonen:

```rust
let wanted = vec!["rust".to_string(), "web".to_string()];
let users = User::query()
    // `wanted.clone()` innen, kein `move` von `wanted` selbst - die
    // Closure kann pro Klon des Builders einmal laufen.
    .with_where(("posts", move |q: Builder<Post>| q.filter_in("tag", wanted.clone())))
    .get()
    .await?;
```

### Eine Query zu klonen behält ihren Eager-Load-Plan

`Builder` ist `Clone`, und der Klon trägt den Eager-Load-Plan mit
sich, sodass das Muster "eine Basis-Query bauen, mehrere davon
ableiten" funktioniert:

```rust
let base = User::query().with(["posts"]).filter("active", true);

let first_page = base.clone().limit(20).get().await?;
let total = base.count().await?;
// first_page-Zeilen haben posts_loaded() befüllt.
```

### Warum Suprnova abweicht

Laravels `$query->with(...)` klont frei, weil PHP-Arrays bei
Zuweisung kopieren. Rust muss sagen, was ein Klon für eine
typgelöschte Closure bedeutet, und bis v0.7.2 antwortete Suprnova,
indem es den Plan fallen ließ - der Klon gelang, die Query gelang,
und die Relationen waren schlicht abwesend. Das Prädikat über ein
`Arc` zu teilen macht den Klon vollständig, um den Preis der
`Fn`-Schranke oben.

Eager Loading innerhalb von `chunk` / `chunk_by_id` / `lazy` bleibt
ein unübersehbarer Fehler statt eines stillen Pro-Chunk-N+1. Legen
Sie `.with(...)` innerhalb der Pro-Chunk-Closure erneut an, wenn Sie
es wollen.

### Laden auf bereits geholten Collections

Wenn Sie eine `Collection<M>` ohne Eager-Load-Plan holen, können Sie
nachträglich einen anhängen:

```rust
let mut users = User::query().get().await?;

users.load(["posts"]).await?;                 // bedingungslos
users.load_missing(["posts.comments"]).await?; // überspringt, was schon geladen ist
```

`load_missing` durchläuft den `__eager`-Cache jedes Elternteils und
löst die IN-Query nur für Zeilen aus, die die Relation noch nicht
geladen haben. Nützlich in Schleifen, in denen manche Eltern früher
in der Anfrage eager geladen wurden und andere nicht.

### Ausschließen - `without`

`without` entfernt benannte Relationen aus dem Eager-Plan, nützlich,
wenn ein Basis-Scope Standardwerte hinzufügt, die Sie für diesen
Aufruf nicht wollen:

```rust
let users = User::query()
    .with(["profile", "posts", "team"])
    .without(["team"])     // entfernt team aus dem Plan
    .get()
    .await?;
```

## Übergeordnete Modelle berühren

Ein untergeordnetes Modell kann deklarieren, dass sein Schreiben das `updated_at` seines übergeordneten Modells aktualisieren soll:

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
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Nur `BelongsTo`-Relationen können berührt werden – die betroffene Zeile muss anhand einer Spalte des untergeordneten Modells identifizierbar sein; genau das liefert Ihnen die besitzende Seite. Das Framework löst das übergeordnete Modell über das Relationsregister auf, sodass das Berühren einen `UPDATE` und keinen `SELECT` kostet.

Übergeordnete Modelle, die Zeitstempel ablehnen (`#[model(timestamps = false)]`), über einen `NULL`-Fremdschlüssel erreicht werden oder soft-gelöscht sind, werden stillschweigend übersprungen. Unterdrücken Sie die Kaskade für einen Arbeitsblock mit `without_touching` (alle übergeordneten Modelle) oder `without_touching_on::<Post, _, _>` (ein Typ). Die vollständige Semantik finden Sie unter [Eloquent – Parent touching](eloquent.md#parent-touching).

## Der Notausgang

Wenn eine Relation zu keiner der elf Arten passt - rekursive Bäume,
polymorph-über-Nicht-id-Schlüssel, dreiseitige Pivots, alles
Maßgeschneiderte - schreiben Sie die Methode von Hand. Das Makro
verhindert das nicht; Sie bekommen nur den Loaded-Accessor oder den
Eager-Load-Dispatcher-Arm für diese Relation nicht.

```rust
impl User {
    /// Benutzerdefiniert: neuester Post unabhängig von der FK-Form.
    pub async fn latest_post(&self) -> Result<Option<Post>, FrameworkError> {
        Post::query()
            .filter("user_id", self.id)
            .latest()
            .first()
            .await
    }
}
```

Der Trade-off ist explizit: handgeschriebene Methoden erscheinen
nicht im Inventory `relations()`, die Existenz-Engine weiß nichts von
ihnen, und der Eager Loader kann sie nicht in einen Plan aufnehmen.
Für Einzelfälle ist das in Ordnung. Für alles, was Sie mit
`with(["..."])` verwenden wollen, deklarieren Sie es als richtige
Relations-Art, auch wenn Sie die Makro-Optionen dafür verbiegen
müssen.

## Nächste Schritte

- [Eloquent API](eloquent.md) - die alltägliche Modell-Oberfläche;
  die Relations-Deklarationssyntax lebt dort.
- [Datenbank](database.md) - Connections, Transactions,
  Multi-Driver, die untere Schicht, auf der alles sitzt.
- [Migrationen](migrations.md) - die Schema-Seite der FK-Spalten,
  die diese Relationen zum Existieren brauchen.
- [Query Builder](eloquent.md#query-builder-dual-api) - die
  Dual-API-Oberfläche, in die Relations-Wrapper weiterleiten.
- [Eloquent Resources](eloquent-resources.md) - geladene Relationen
  in JSON:API-Payloads für den Client verwandeln.

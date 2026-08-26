# Eloquent API

Suprnovas Eloquent-Schicht gibt Laravel-Entwicklern die API, die sie
bereits kennen, implementiert als dünner Shim über SeaORM. Kopieren
Sie Code aus den Laravel-Docs, tauschen Sie PHP-Syntax gegen Rust,
fügen Sie `.await?` hinzu, und er läuft.

Die gesamte Schicht besteht aus einem Struktur-Attribut
(`#[suprnova::model]`), einem Trait (`Model`) und einem verkettbaren
Query Builder (`Builder<M>`) - das ist alles. Hinter den Kulissen
generiert das Makro ein SeaORM-`Entity`, -`Model`, -`ActiveModel` und
ein `Column`-Enum sowie jede Eloquent-Trait-Implementierung. Die
SeaORM-Typen bleiben erreichbar für den seltenen Fall, dass die
Eloquent-Oberfläche etwas nicht abdeckt (siehe die
[SeaORM-Notausgänge](#der-notausgang-zu-seaorm)).

## Inhaltsverzeichnis

- [Schnellstart](#schnellstart)
- [Das Attribut `#[suprnova::model]`](#the-suprnovamodel-attribute)
- [Layout des Modellmoduls](#layout-des-modellmoduls)
- [Zeilen finden](#zeilen-finden)
- [Erstellen und Aktualisieren](#erstellen-und-aktualisieren)
- [Löschen und Soft Deletes](#löschen-und-soft-deletes)
- [Query Builder - Dual-API](#query-builder--dual-api)
- [Zeilensperren](#zeilensperren)
- [Transaktionen](#transaktionen)
- [Scopes](#scopes)
- [Relationen](#relationen)
- [Eager Loading](#eager-loading)
- [Paginierung](#paginierung)
- [Chunking und Lazy-Iteration](#chunking-und-lazy-iteration)
- [Collections](#collections)
- [Mass Assignment](#mass-assignment)
- [Casts](#casts)
- [Accessoren und Mutatoren](#accessoren-und-mutatoren)
- [Timestamps](#timestamps)
- [Observers und Lifecycle-Events](#observers-und-lifecycle-events)
- [Prunable](#prunable)
- [Multi-Connection-Routing](#multi-connection-routing)
- [Replikation](#replikation)
- [Debugging - dump und dd](#debugging--dump-and-dd)
- [Modelle testen](#modelle-testen)
- [Der Notausgang zu SeaORM](#der-notausgang-zu-seaorm)
- [Migration von `database::Model`](#migrating-from-databasemodel)
- [DB-Facade - modell-lose Queries](#db-facade--model-less-queries)
- [Laravel-13-Parität - Relations-Existenz + günstige Kurzformen](#laravel-13-parity--relation-existence--cheap-shortcuts)

## Schnellstart

Ein Attribut auf einer Struktur macht daraus ein vollwertiges
Eloquent-Modell:

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

Einmal deklariert, können Sie schreiben:

- `User::query()` - startet einen fluenten Query Builder.
- `User::find(id).await?` - holt per Primärschlüssel.
- `User::find_or_fail(id).await?` - dasselbe, gibt bei Fehltreffer aber
  einen Fehler mit `ModelNotFound` zurück.
- `User::all().await?` - alle Zeilen.
- `User::create(attrs!{ name: "Alice", email: "alice@example.com" }).await?` -
  Insert mit Mass-Assignment-Filterung.
- `User::filter("email", "alice@example.com").first().await?` -
  eine passende Zeile.
- `user.update(attrs!{ name: "Alice B" }).await?` - Teil-Update.
- `user.save().await?` - persistiert Änderungen im Speicher.
- `user.delete().await?` - entfernt die Zeile.
- `user.refresh().await?` / `user.fresh().await?` / `user.replicate().await?` -
  der Rest des Laravel-Lifecycle.

Die nutzerseitige Struktur (hier `User`) IST der Typ, den Ihre
Handler und Controller verwenden. Das Makro erzeugt ein inneres
Modul pro Modell (`user::`) mit den SeaORM-Typen `Entity`, `Column`,
`ActiveModel` und `Model`, für die Fälle, in denen Sie direkt zu
SeaORM wechseln möchten. Die Struktur wird außerdem in einem
Inventory-gestützten `ModelEntry` registriert, sodass Admin- und
Tooling-Code beim Boot jedes Modell aufzählen kann.

## Das Attribut `#[suprnova::model]`

Der einzige Einstiegspunkt zum Deklarieren eines Modells. Jedes
Attribut ist optional; die Standardwerte sind so abgestimmt, dass
eine Struktur mit `id` + `created_at` + `updated_at` ohne jede
Konfiguration als Suprnova-Modell funktioniert.

### Makro-Attributreferenz

| Attribut | Typ | Standard | Hinweise |
|-----------|------|---------|-------|
| `table` | string | snake_case-Plural des Struktur-Namens | Überschreibt den Tabellennamen |
| `primary_key` | string | `"id"` | Überschreibt den PK-Spaltennamen |
| `key_type` | type | `i64` | PK-Typ - `String` für UUID, `i32` für Legacy-Schemas |
| `auto_increment` | bool | `true` | Deaktivieren für UUID-PKs |
| `connection` | string | `"default"` | Multi-Connection-Apps benennen eine Nicht-Standard-Connection |
| `fillable` | Liste von Strings | (Standard = `guarded = ["id"]`) | Mass-Assignment-Allowlist |
| `guarded` | Liste von Strings | `["id"]`, wenn keines gesetzt ist | Mass-Assignment-Denylist (schließt sich mit `fillable` gegenseitig aus) |
| `casts` | map of `field = CastType` | `{}` | Casts pro Spalte |
| `hidden` | Liste von Strings | `[]` | Ausgeschlossen von `to_json` / `to_array` |
| `visible` | Liste von Strings | (alle) | Inklusive Variante von `hidden` (schließt sich gegenseitig aus) |
| `appends` | Liste von Strings | `[]` | Accessoren, die in die Serialisierung aufgenommen werden |
| `soft_deletes` | flag | `false` | Aktiviert die Spalte `deleted_at` + Tombstone-Semantik |
| `soft_deletes_column` | string | `"deleted_at"` | Überschreibt den Namen der Soft-Delete-Spalte |
| `timestamps` | flag / bool | `true`, wenn sowohl `created_at` als auch `updated_at` existieren | Deaktiviert automatisch verwaltete Timestamps |
| `created_at` | string | `"created_at"` | Überschreibt den Spaltennamen |
| `updated_at` | string | `"updated_at"` | Überschreibt den Spaltennamen |
| `touches` | Liste von Relationsnamen | `[]` | `BelongsTo`-Relationen, deren übergeordnete Zeile ein aktualisiertes `updated_at` erhält, nachdem dieses Modell erstellt, gespeichert, aktualisiert oder gelöscht wurde |
| `mutators` | Liste von Strings | `[]` | Feldnamen, deren JSON-Fill-Pfad über eine `set_<field>(value)`-Mutator-Methode läuft |

### Vollständiges Beispiel

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

### Makros auf Funktionsebene

Makros auf Funktionsebene arbeiten neben dem Struktur-Attribut:

- `#[accessor]` auf einer `fn name(&self) -> T` macht daraus einen
  Eloquent-Accessor. Das `to_array()` des Modells ruft sie auf, wenn
  `name` in `appends = [...]` aufgeführt ist (und `to_json()`
  übernimmt sie über die `to_array` → String-Delegation).
- `#[mutator]` auf einer `fn set_name(&mut self, value: serde_json::Value)`
  macht daraus einen Eloquent-Mutator. Der JSON-Fill-Pfad des Modells
  läuft darüber, wenn `name` in `mutators = [...]` aufgeführt ist.
- `#[suprnova::scopes(Model)]` auf einem `impl Model { ... }`-Block:
  Jede Methode, deren Signatur
  `fn name(query: Builder<Self>[, args…]) -> Builder<Self>` lautet,
  wird sowohl zu einem verkettbaren `.scope_name(args)` auf
  `Builder<Self>` als auch zu einer Abkürzung
  `Model::scope_name(args)`. Es gibt keine `#[scope]`-Form auf
  Funktionsebene - Scopes werden pro Impl-Block deklariert.
- Globale Scopes sind eine Runtime-Registrierung über den Trait
  `GlobalScope`, angewendet über `Model::global_scope::<GS>()`. Es
  gibt kein `#[global_scope]`-Makro auf Funktionsebene - das
  vollständige Muster steht unter
  [Makros](macros.md#suprnova-scopes-model).
- `#[prunable]` auf `impl Prunable for T { ... }` registriert den
  Pruner über Inventory, sodass `model:prune` ihn findet.

## Layout des Modellmoduls

`#[suprnova::model]` behält Ihre nutzerseitige Struktur (z. B. `Post`)
im übergeordneten Scope und erzeugt daneben ein `pub mod`, benannt
nach der Struktur in snake_case (`post`). In diesem inneren Modul
leben die SeaORM-Typen.

Für ein Modell, deklariert unter `app/src/models/posts.rs`:

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

// Konvention: Re-exportieren Sie die SeaORM-Typen, die das Makro im
// inneren Modul erzeugt, damit Aufrufstellen die Namen ohne Präfix
// verwenden können. Suprnovas eigene Dogfood-Modelle tragen alle
// diese Zeile (siehe `app/src/models/users.rs`,
// `app/src/models/posts.rs` usw.).
pub use post::{ActiveModel, Column, Entity};
```

Von `crate::models::posts` aus sind nun folgende Elemente erreichbar:

| Pfad | Was es ist |
|------|-----------|
| `crate::models::posts::Post` | Ihre nutzerseitige Struktur - das Eloquent-Modell |
| `crate::models::posts::post::Entity` | SeaORM-`EntityTrait`-Implementierung für die Tabelle `posts` |
| `crate::models::posts::post::Column` | SeaORM-`Column`-Enum (eine Variante pro Spalte) |
| `crate::models::posts::post::ActiveModel` | SeaORM-`ActiveModel` für Insert/Update |
| `crate::models::posts::post::Model` | Zeile in SeaORM-Form (storage-typisierte Spalten) |
| `crate::models::posts::{Entity, Column, ActiveModel}` | Die obige `pub use`-Konvention; wird nicht automatisch erzeugt |

Zwei Dinge, die Sie über das `Model` des inneren Moduls wissen
sollten:

1. Es ist die Zeile in **SeaORM-Form**, nicht Ihre `Post`-Struktur.
   Cast-Spalten tragen hier ihren `Storage`-Typ (z. B. wird `bool`
   zum zugrunde liegenden Integer), und die Runtime-Slots `__eager` /
   `__pivot` aus Ihrer Struktur fehlen.
2. `From<post::Model> for Post` und `From<Post> for post::Model`
   überbrücken die beiden Formen. Das Round-Trip-Muster steht unter
   [Der Notausgang zu SeaORM](#der-notausgang-zu-seaorm).

`Model` ist absichtlich **kein** Teil des konventionellen
Re-Exports auf übergeordneter Ebene - die nutzerseitige `Post` belegt
den Namen `Post` im übergeordneten Scope bereits, und `post::Model`
ist ein separater Typ, den Aufrufer über `post::Model` (oder eine
`From`-Konvertierung) erreichen, wenn sie die innere Form benötigen.

### Wann Sie ins innere Modul greifen

Die Eloquent-Oberfläche (Trait `Model` + `Builder<M>`) deckt die
große Mehrheit der Queries ab. Greifen Sie in `post::*`, wenn Sie
SeaORM-exklusive Features benötigen:

- **Rohe Query-Konstruktion** mit SeaORMs `EntityTrait::find()`-Kette,
  wenn Eloquent den gewünschten Helper nicht bereitstellt.
- **Benutzerdefinierte Join-Logik** - `JoinType::*`-Joins explizit
  über `QuerySelect::join()` aufbauen, für eine Relation, die
  Eloquents `with(...)` nicht modelliert.
- **SeaORM-native Subqueries** über `Entity::find().select_only()`.
- **Reine `ActiveModel`-Mutation** für den seltenen Fall, dass Sie
  den Eloquent-Lifecycle umgehen möchten (keine Observer, keine
  Auto-Timestamps).

```rust
// Häufiger Fall - Column wird oben über die Konvention
// `pub use post::{...}` auf übergeordneter Modulebene re-exportiert.
use crate::models::posts::Column;

let drafts = Post::query()
    .db_where(Column::Status, "draft")
    .get()
    .await?;

// Power-User-Fall - direkt ins innere Modul greifen, um an die
// SeaORM-Entity zu kommen. Das legt das übergeordnete `pub use`
// nicht offen.
use crate::models::posts::post;
use suprnova::sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

let db = suprnova::DB::connection()?;
let rows: Vec<post::Model> = post::Entity::find()
    .filter(post::Column::Status.eq("published"))
    .all(db.inner())
    .await?;

// Zurückbrücken zur Eloquent-Form, wenn der Aufrufer sie möchte.
let posts: Vec<Post> = rows.into_iter().map(Post::from).collect();
```

Wenn Sie sich dabei wiederfinden, für dieselbe Operation
routinemäßig ins innere Modul zu greifen, ist das ein Signal, dass
Eloquent einen Helper fehlt - öffnen Sie ein Issue, oder fügen Sie
den Helper der `Model`- / `Builder`-Oberfläche hinzu.

## Zeilen finden

```php
// Laravel
$user = User::find(1);
$user = User::findOrFail(1);          // wirft bei fehlendem Treffer
$users = User::findMany([1, 2, 3]);
```

```rust
// Suprnova
let user: Option<User> = User::find(1).await?;
let user: User = User::find_or_fail(1).await?;
let users: Vec<User> = User::find_many([1, 2, 3]).await?;
```

`find_or_fail` gibt `FrameworkError::ModelNotFound` zurück (HTTP 404,
wenn es bis zu einem Controller durchschlägt).

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
$user = User::firstOrNew(['email' => 'alice@example.com']);  // nicht gespeichert
```

```rust
// Suprnova
let user = User::first_or_create(
    attrs! { email: "alice@example.com" },          // Lookup-Schlüssel
    attrs! { name: "Alice" },                        // zusätzliche Felder beim Create
).await?;

let user = User::update_or_create(
    attrs! { email: "alice@example.com" },
    attrs! { name: "Alice Updated" },
).await?;

let user = User::first_or_new(
    attrs! { email: "alice@example.com" },
).await?;   // gibt einen nicht gespeicherten User zurück; Aufrufer speichert explizit
```

Lookup-Schlüssel kommen in die erste Map; zusätzliche Felder, die im
Create-Pfad angewendet werden, kommen in die zweite Map. Da
`first_or_new` ein nicht gespeichertes Modell zurückgibt, kann der
Aufrufer es weiter verändern, bevor `save().await?` läuft.

## Erstellen und Aktualisieren

### Erstellen

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

`attrs!` ist ein Makro, das einen `Attrs`-Wert erzeugt (eine typisierte
JSON-Map). Reines JSON funktioniert ebenfalls -
`User::create(serde_json::json!({"name": "Alice", "email": "..."}))`.
Der `Fillable`-Filter läuft innerhalb von `create`; nicht befüllbare
Felder werden stillschweigend verworfen, wie bei Laravel.

### Save / update

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

`save()` läuft jedes Feld außer dem Primärschlüssel ab, setzt sie über
`Set(...)` am ActiveModel, ruft SeaORMs `update()` auf und gibt die
kanonische Zeile zurück. `update(attrs)` ist derselbe Ablauf, wendet aber
zuerst eine partielle Attribut-Map an (dabei laufen der
`Fillable`-Filter und alle deklarierten Mutatoren).

### Increment / decrement

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

`increment` / `decrement` geben SQL der Form
`UPDATE table SET col = col + N WHERE ...` aus - atomar gegenüber
nebenläufigen Updates, ohne Read-Modify-Write-Race. Verfügbar sowohl auf
einer geholten Modellinstanz (nutzt den Primärschlüssel der Zeile in der
WHERE-Klausel) als auch als Abschlussmethode am Builder (nutzt die
WHERE-Klauseln der Kette).

### Fresh / refresh / replicate

```php
// Laravel
$user->refresh();                          // reload from DB
$user->refreshForUpdate();                 // reload under a row lock
$copy = $user->fresh();                    // fetch + return copy
$replica = $user->replicate();             // unsaved clone with fresh PK
$replica = $user->replicate(['email']);    // skip a field
```

```rust
// Suprnova
user.refresh().await?;
user.refresh_for_update().await?;
let copy: User = user.fresh().await?;
let replica: User = user.replicate().await?;
let replica: User = user.replicate_except(["email"]).await?;
```

`refresh` verändert an Ort und Stelle; `fresh` gibt eine separat geholte
Kopie zurück. `refresh_for_update` ist `refresh` unter einer Zeilensperre
per `SELECT ... FOR UPDATE` - nutzen Sie es innerhalb einer Transaktion,
wenn Sie die aktuellen Werte der Zeile und die exklusive Sperre in einem
Statement brauchen. Anders als `refresh` umgeht `refresh_for_update`
jeden registrierten globalen Scope UND den
`#[model(soft_deletes)]`-Filter: Es lädt auch eine gelöschte Zeile neu,
und `deleted_at` kommt gesetzt zurück. Das Neuladen ist ein Nachschlag
über den Primärschlüssel unter einer Sperre - es wie ein gewöhnliches
Lesen einzugrenzen würde Admin-Werkzeugen und mandantenübergreifenden
Aufrufern ein falsches „nicht gefunden“ für eine Zeile liefern, auf die
sie bereits eine Referenz halten. `replicate` baut einen Klon im
Speicher, dessen Primärschlüssel zurückgesetzt ist
(`Default::default()` für den Schlüsseltyp). Der Aufrufer speichert
ausdrücklich.

`refresh` und `refresh_for_update` geben beide einen Fehler zurück, wenn
die Zeile nicht mehr existiert, statt das Modell mit veralteten Werten
zurückzulassen. SQLite hat keine Sperren auf Zeilenebene,
`refresh_for_update` lädt dort also ohne Sperre neu - siehe
[Zeilensperren](#zeilensperren).

### Replicating-Event

`replicate` und `replicate_except` lösen das Event
`Replicating { source, replica }` pro Modell aus, nachdem der Klon im
Speicher gebaut wurde und BEVOR er zurückgegeben wird. Das Feld `replica`
ist ein `Arc<tokio::sync::Mutex<Self>>`, sodass Listener die Replica
verändern können, bevor der Aufrufer sie sieht - nützlich, um Titeln
`(copy)` voranzustellen, Flags zu löschen, abgeleitete Spalten
zurückzusetzen und so weiter.

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

// Einmal beim Boot verdrahten:
EventFacade::listen::<post::events::Replicating, _>(
    std::sync::Arc::new(PrefixTitle)
).await;
```

### Typübergreifende Replikation

```rust
let replica: UserDraft = user.replicate_into().await?;  // typübergreifender Klon
```

Eine Suprnova-Abweichung - Laravel kann das nicht, weil PHP keine Typen
hat. Nützlich, wenn Sie ein Entwurfsmodell in ein endgültiges befördern
oder umgekehrt.

`replicate_into<T>` löst `Replicating` NICHT aus (das Event trägt
`Arc<Mutex<Self>>`, ein Listener auf dem Quelltyp könnte die
typübergreifende Replica also ohnehin nicht verändern). Wer eine
Einrichtung pro `T` möchte, führt sie auf dem zurückgegebenen `T` aus,
bevor `T::save` aufgerufen wird - die normale Kette aus `Saving` /
`Created` feuert innerhalb von `save` weiterhin.

## Löschen und Soft Deletes

### Soft-Delete-Flag

Fügen Sie `soft_deletes` zum Makro-Attribut und eine Spalte
`deleted_at: Option<DateTime<Utc>>` zur Struktur hinzu:

```rust
#[model(table = "users", soft_deletes, timestamps)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub deleted_at: Option<DateTime<Utc>>,
    // ...
}
```

### Lifecycle

```rust
user.delete().await?;             // UPDATE: setzt deleted_at = NOW()
user.trashed();                   // -> true
let trashed = User::with_trashed().find(user.id).await?.unwrap();
trashed.restore().await?;         // UPDATE: setzt deleted_at = NULL

let only_dead = User::only_trashed().get().await?;
let all_including_dead = User::with_trashed().get().await?;

user.force_delete().await?;       // echtes DELETE
```

### Standard-Scope

Wenn `soft_deletes` gesetzt ist, überschreibt das Makro
`Model::query()`, sodass Standard-Reads gelöschte Zeilen automatisch
herausfiltern. `with_trashed()` und `only_trashed()` holen sie
wieder hinein. Konkret: `User::find(id)` überspringt gelöschte
Zeilen; `User::with_trashed().find(id)` findet sie.

## Query Builder - Dual-API

`Builder<M>` ist der verkettbare Query-Typ, den `User::query()`,
`User::filter(...)`, `User::db_where(...)` und jede andere statische
Methode zurückgeben, die die Kette nicht beendet.

### Anmerkung zur Benennung: Dual-API

`where` ist ein Rust-Schlüsselwort, die Where-Methode für schlichte
Gleichheit kann sich Laravels Namen also nicht teilen. Statt einen
Gewinner zu küren, wird jede Methode in Where-Form unter **beiden** Namen
ausgeliefert: einem rust-idiomatischen (`filter`, `filter_in`,
`filter_null`, …) und einem Laravel-förmigen (`db_where`, `where_in`,
`where_null`, …). Sie sind Aliase über einer kanonischen Implementierung -
nehmen Sie den, zu dem Ihr Muskelgedächtnis passt.

```rust
// Rust-Entwickler:
User::query().filter("active", true).filter_in("role", ["admin"]).get().await?;

// Laravel-Entwickler:
User::db_where("active", true).where_in("role", ["admin"]).get().await?;

// Gleiche Query. Gleiches Ergebnis. Anderes Muskelgedächtnis.
```

### Where-Kurzformen

```php
// Laravel
$users = User::where('email', $email)->get();
$users = User::where('age', '>=', 18)->get();
$users = User::where('email', 'like', '%@example.com')->get();
```

```rust
// Suprnova - nehmen Sie eine der beiden Familien; beide kompilieren,
// beide sind dokumentiert.

// Rust-Form (filter-Familie):
let users = User::query().filter("email", &email).get().await?;
let users = User::query().filter_op("age", ">=", 18).get().await?;
let users = User::query().filter_like("email", "%@example.com").get().await?;

// Laravel-Form (db_where- / where_*-Familie):
let users = User::db_where("email", &email).get().await?;
let users = User::query().db_where_op("age", ">=", 18).get().await?;
let users = User::query().where_like("email", "%@example.com").get().await?;
```

### Where-Varianten

Jede Zeile hat zwei gleichwertige Suprnova-Formen - die Rust-Form
(`filter*`) und die Laravel-Form (`db_where` / `where_*`). Beide rufen
dieselbe kanonische Implementierung auf; beide sind mit
`#[doc(alias = "...")]` ausgezeichnet, sodass die rustdoc-Suche jede von
beiden findet.

| Laravel | Suprnova (Rust-Form) | Suprnova (Laravel-Form) | Hinweise |
|---------|----------------------|--------------------------|-------|
| `->where(col, val)` | `.filter(col, val)` | `.db_where(col, val)` | Gleichheit |
| `->where(col, op, val)` | `.filter_op(col, op, val)` | `.db_where_op(col, op, val)` | Beliebiger Operator |
| `->orWhere(...)` | `.or_filter(...)` | `.or_where(...)` | |
| `->orWhereKey(id)` | `.or_filter_key(id)` | `.or_where_key(id)` | PK-Filter als Disjunkt |
| `->orWhereKeyNot(id)` | `.or_filter_key_not(id)` | `.or_where_key_not(id)` | Negierter PK-Filter als Disjunkt |
| `->whereNot(col, val)` | `.filter_not(col, val)` | `.where_not(col, val)` | |
| `->whereIn(col, vals)` | `.filter_in(col, vals)` | `.where_in(col, vals)` | |
| `->whereNotIn(col, vals)` | `.filter_not_in(col, vals)` | `.where_not_in(col, vals)` | |
| `->whereBetween(col, [a, b])` | `.filter_between(col, a..=b)` | `.where_between(col, a..=b)` | Rust-Range |
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
| `->whereBinary(col, val)` | `.filter_binary(col, val)` | `.where_binary(col, val)` | Byte-exakt; nur MySQL und MariaDB |
| `->orWhereBinary(col, val)` | `.or_filter_binary(col, val)` | `.or_where_binary(col, val)` | |
| `->whereNotBinary(col, val)` | `.filter_not_binary(col, val)` | `.where_not_binary(col, val)` | |
| `->orWhereNotBinary(col, val)` | `.or_filter_not_binary(col, val)` | `.or_where_not_binary(col, val)` | |
| `->whereJsonContains(col, v)` | `.filter_json_contains(col, v)` | `.where_json_contains(col, v)` | Nach Backend verteilt |
| `->whereJsonLength(col, op, n)` | `.filter_json_length(col, op, n)` | `.where_json_length(col, op, n)` | |
| `->whereColumn(a, b)` | `.filter_column(a, b)` | `.where_column(a, b)` | Vergleich Spalte gegen Spalte |
| `->whereExists(closure)` | `.filter_exists(builder)` | `.where_exists(builder)` | Subquery |
| `->whereHas(rel, closure)` | `.filter_has(rel, fn)` | `.where_has(rel, fn)` | Relations-Prädikat (10B) |
| `->whereDoesntHave(rel)` | `.filter_doesnt_have(rel)` | `.where_doesnt_have(rel)` | (10B) |
| `->whereRelation(rel, col, op, v)` | `.filter_relation(...)` | `.where_relation(...)` | (10B) |
| `->whereRaw(sql, bindings)` | `.filter_raw(sql, bindings)` | `.where_raw(sql, bindings)` | |

Die `binary`-Familie vergleicht rohe Bytes, statt unter der Kollation der
Spalte zu vergleichen. MySQL und MariaDB geben `col = binary ?` aus;
Postgres und SQLite haben keinen entsprechenden Operator, eine
Abschlussmethode gibt auf diesen Backends daher beim Rendern des
Statements einen Fehler zurück, statt auf ein kollationsabhängiges `=`
zurückzufallen. Siehe
[Byte-exakter Vergleich](queries.md#byte-exact-comparison).

Gebundene rohe Prädikate nutzen unter SQLite, MySQL und PostgreSQL
portable `?`-Marker:

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

Unter PostgreSQL setzt Suprnova diese Marker hinter den vorherigen
Query-Bindings neu auf, das Beispiel rendert also `$1` für `active` und
`$2`/`$3` für das rohe Prädikat. Nutzen Sie `??` für einen wörtlichen
Fragezeichen-Operator in einem gebundenen rohen Fragment, etwa
`"payload ?? 'enabled' AND status = ?"`. Bestehende `$N`-Fragmente
werden weiterhin angenommen, aber portable Marker koppeln Aufrufstellen
nicht an die Position in der Query. Gemischte Markerstile und eine nicht
passende Anzahl von Markern und Bindings werden vor jeder
Datenbank-E/A abgelehnt. Wie bei jedem rohen Ausdruck muss dem SQL-Text
vertraut werden; nicht vertrauenswürdige Werte gehören ausschließlich in
den Bindings-Vektor.

### Sortierung

```php
$users = User::orderBy('name', 'asc')->get();
$users = User::orderByDesc('created_at')->get();
$users = User::latest()->get();        // Kurzform: orderBy(created_at, desc)
$users = User::oldest()->get();        // Kurzform: orderBy(created_at, asc)
$users = User::inRandomOrder()->get();
```

```rust
let users = User::query().order_by("name", Direction::Asc).get().await?;
let users = User::query().order_by_desc("created_at").get().await?;
let users = User::latest().get().await?;
let users = User::oldest().get().await?;
let users = User::query().in_random_order().get().await?;
```

`Direction::Asc` / `Direction::Desc` ist das Suprnova-Enum, das aus
SeaORM re-exportiert wird.

#### Nach einer ausdrücklichen Reihenfolge sortieren

`in_order_of` sortiert Zeilen in die Reihenfolge, die Sie auflisten. Was
einen Wert hat, der nicht in der Liste steht, sortiert hinter alles, was
darin steht.

```php
$users = User::inOrderOf('role', ['admin', 'member', 'guest'])->get();
```

```rust
let users = User::query()
    .in_order_of("role", ["admin", "member", "guest"])
    .get()
    .await?;
```

Suprnova rendert das als gebundenen `CASE`-Ausdruck, die Werte sind also
Parameter und dürfen aus Anfragedaten stammen:

```sql
ORDER BY CASE WHEN role = ? THEN 0 WHEN role = ? THEN 1 WHEN role = ? THEN 2 ELSE 3 END
```

Der Spaltenname ist ein SQL-Bezeichner, kein Parameter. Schreiben Sie ihn
fest oder wählen Sie ihn aus einer Allowlist, genau wie jedes andere
Spaltenargument. Eine leere Werteliste fügt gar keine Sortierung hinzu,
Sie können die Reihenfolge also bedingt aufbauen, ohne den leeren Fall
gesondert zu behandeln.

Bei einer Spalte, die den Cast `AsEnum<E>` nutzt, reichen Sie jede
Variante durch `as_ref()`. Das ist genau die Zeichenkette, die der Cast
speichert:

```rust
let users = User::query()
    .in_order_of("role", [Role::Admin.as_ref(), Role::Member.as_ref()])
    .get()
    .await?;
```

`in_order_of` wird auf der typisierten `Builder<M>`-Oberfläche
ausgeliefert. Der modell-lose `DB::table(...)`-Builder sortiert nur nach
Spalte und Richtung.

### Gruppieren + having

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
$users = User::take(10)->skip(20)->get();   // Aliase
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

`get_raw()` gibt das Ergebnis in roher Spaltenform zurück, für
`select_raw`-Fälle, in denen die Spalten nicht zum Modellschema passen;
`get()` gibt `Vec<User>` zurück und verlangt, dass die ausgewählten
Spalten die Modellstruktur füllen.

### Distinct

```rust
let emails: Vec<String> = User::query().distinct().pluck("email").await?;
```

### Aggregate

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

Aggregate sind über den Rückgabetyp generisch, weil SeaORM wissen muss,
worauf es den Skalar aus der Datenbank umwandeln soll. Typ-Vorgaben:
`count -> i64`; `sum`/`avg` tragen einen ausdrücklichen Typparameter.
Suprnova versieht generierte Aggregat-Ausdrücke intern mit Aliasen, damit
unter PostgreSQL, MySQL und SQLite dasselbe typisierte Ergebnis dekodiert
wird. `sum` und `avg` geben bei einer leeren Treffermenge null zurück,
während `min` und `max` `None` liefern. Ein unpassender angeforderter
Rust-Typ oder eine fehlende Ergebnisspalte ist ein Datenbankfehler; das
wird nie in eine plausible Null oder ein `None` umgedeutet.

### Abschlussmethoden

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

`to_sql` gibt das parametrisierte SQL zurück, das die nächste
Abschlussmethode ausgeben würde - nützlich zum Debuggen oder zum Bauen von
Views. Die Bindings sind über
`.to_sql_with_bindings() -> (String, Vec<Value>)` erreichbar.

`model_keys` ist die Abschlussmethode nur für Schlüssel: Sie projiziert den
**qualifizierten** Primärschlüssel (`users.id`) und hydriert nie ein
Modell, eine Frage nach dem Motto „welche Zeilen haben getroffen?“ kostet
also eine Spalte statt einer ganzen Zeile pro Treffer. Die Qualifizierung
ist das, was sie eine Query überstehen lässt, die eine andere Tabelle mit
eigenem `id` joint. Ein `select(...)`, das bereits auf dem Builder liegt,
wird verworfen - der Aufrufer hat nach Schlüsseln gefragt.

### Unions

```rust
let first  = User::filter("active", true);
let second = User::filter("role", "admin");
let users  = first.union(second).get().await?;
let users  = first.union_all(second).get().await?;
```

## Zeilensperren

Zwei Builder-Methoden fordern zum SELECT-Zeitpunkt eine Datenbanksperre
pro Zeile an:

```rust
// Exklusive Schreibsperre - blockiert andere Transaktionen, die
// dieselben Zeilen sperren oder schreiben wollen, bis diese Transaktion
// committet.
let order = Order::query()
    .filter("id", 42)
    .lock_for_update()
    .first_or_fail()
    .await?;

// Geteilte Lesesperre - erlaubt andere geteilte Leser, blockiert
// Schreiber.
let inventory = Inventory::query()
    .filter("sku", sku)
    .shared_lock()
    .first_or_fail()
    .await?;
```

Das je Backend ausgegebene SQL:

| Backend | `lock_for_update()` | `shared_lock()` |
|----------|---------------------|------------------------|
| Postgres | `FOR UPDATE` | `FOR SHARE` |
| MySQL | `FOR UPDATE` | `LOCK IN SHARE MODE` |
| SQLite | (kein SQL, siehe unten) | (kein SQL, siehe unten) |

Die Sperrklausel wird ganz am Ende des zusammengesetzten Statements
angehängt - nach jedem `UNION`-Zweig, jedem `ORDER BY`, jedem
`LIMIT` / `OFFSET`. Ein `union(...)` zweier Builder, gefolgt von
`.lock_for_update()`, gibt genau **ein** `FOR UPDATE` im äußeren Scope
aus, nicht eines pro Zweig.

Um ein Modell, das Sie bereits halten, neu zu laden und im selben
Statement die Sperre zu nehmen, nutzen Sie `refresh_for_update`:

```rust
DB::transaction(|tx| async move {
    let mut order = Order::find_or_fail(42).await?;
    order.refresh_for_update().await?;   // SELECT ... WHERE id = ? FOR UPDATE
    order.status = "processed".into();
    order.save_with_tx(&tx).await?;
    Ok(())
}).await?;
```

### Verwendung innerhalb einer Transaktion

Die Sperre leistet nur **innerhalb einer Transaktion** nützliche Arbeit -
ohne eine solche wird das SQL zwar ausgegeben, die Sperre wird aber am
Ende des Statements freigegeben. Kombinieren Sie sie mit
`DB::transaction(...)`:

```rust
DB::transaction(|tx| async move {
    let order = Order::query()
        .filter("id", 42)
        .lock_for_update()
        .first_or_fail()
        .with_tx(&tx)
        .await?;
    // Andere Transaktionen, die id=42 sperren wollen, blockieren hier
    // bis zum Commit.
    order.status = "processed".into();
    order.save_with_tx(&tx).await?;
    Ok(())
}).await?;
```

### `lock_for_update` vs. `shared_lock`

Die meisten Abläufe nach dem Muster „lesen, dann schreiben“ wollen
`lock_for_update`. Bei einer geteilten Sperre kann ein anderer
`shared_lock`-Leser weiterhin mit Ihnen in eine Race Condition um ein
folgendes `UPDATE` geraten - nur `FOR UPDATE` schließt sich gegenseitig
aus.

`shared_lock` ist richtig für konsistente Snapshot-Lesevorgänge, bei
denen Sie eine Zeile lesen, daraus eine Entscheidung ableiten und nichts
zurückschreiben - etwa eine Bestandsprüfung, die selbst keinen Bestand
verringert.

### SQLite

SQLite hat keine Sperren auf Zeilenebene. Es nutzt ausschließlich
Transaktionssperren auf Dateiebene (`BEGIN IMMEDIATE` /
`BEGIN EXCLUSIVE`). Die Sperrmethoden **bleiben** im SQLite-Pfad
erhalten, damit backend-übergreifender Code kompiliert, aber sie geben
kein SQL aus.

Beim ersten Mal pro Prozess, dass `lock_for_update` / `shared_lock` gegen
ein SQLite-Backend läuft, protokolliert das Framework ein einzelnes
`warn!` auf dem Tracing-Ziel `suprnova::eloquent::lock`. Das macht die
Wirkungslosigkeit sichtbar, ohne Codepfade mit hohem Aufkommen
zuzuspammen.

Wenn Sie unter SQLite Garantien gegen zeilenübergreifende Konkurrenz
brauchen, umschließen Sie den kritischen Abschnitt mit einer
ausdrücklichen `BEGIN IMMEDIATE`-Transaktion - auf Dateiebene blockiert
das jeden anderen Schreiber.

### Was in v1 nicht enthalten ist

- **`NOWAIT` / `SKIP LOCKED`** - nützlich für Abläufe, in denen ein
  Job-Queue-Anspruch erhoben wird, aber sie vergrößern die
  API-Oberfläche. Zurückgestellt, bis ein echter Konsument sie braucht.

## Transaktionen

Suprnova liefert drei Einstiegspunkte für Datenbank-Transaktionen
plus verschachteltes Rollback über Savepoints. Zwei davon - die
Closure-Form und der Wiederholung-bei-Deadlock-Helper - installieren
einen umgebenden Kontext, sodass Model-Operationen innerhalb der
Closure automatisch durch die Transaktion geroutet werden, ohne dass
Aufrufer ein Handle durch jede Aufrufstelle fädeln müssen.

### Closure-Form - `DB::transaction`

Die Closure-Form ist der häufige Fall. Die Closure erhält eine
`&Transaction`, mit der sie über `savepoint(name)` einen Checkpoint
setzen kann; jede `Model::*`- / `Builder::*`-Operation innerhalb der
Closure routet automatisch über ein `tokio::task_local!` namens
`CURRENT_TX` durch die Transaktion.

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

- Closure gibt `Ok` zurück → **Commit**.
- Closure gibt `Err` zurück → **Rollback** (der ursprüngliche Fehler
  wird durchgereicht).
- Closure paniert → Rollback (die laufende Transaktion wird beim
  Unwind gedroppt; SeaORMs `DatabaseTransaction::drop` macht ein
  Rollback).

Reads innerhalb der Closure sehen Writes aus derselben Transaktion
(über einen `CURRENT_TX`-Lookup bei jedem SQL-Blattaufruf). Der
erste `DB::transaction`-Aufruf nach Prozessstart holt sich das
Datenbank-Backend von `DB::connection()`; nachfolgende Aufrufe
verwenden dieselbe Connection-Registry weiter.

Die Signatur verwendet eine Higher-Ranked-Trait-Bound +
`Pin<Box<dyn Future>>`, damit Closures `tx` über `.await`-Punkte
hinweg borrowen können:

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        // ... Arbeit vor dem Savepoint ...
        tx.savepoint("inner").await?;
        // ... innere Arbeit ...
        if some_condition {
            tx.rollback_to("inner").await?;
        }
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

Die Form `Box::pin(async move { ... })` ist der Preis dafür, dass
das Future `&tx` nach einem `.await` verwenden darf - ohne sie kann
die Lifetime des Borrows den Closure-Body nicht verlassen. Spiegelt
SeaORMs `TransactionTrait::transaction`-Signatur.

### Savepoints - `tx.savepoint(name)` / `tx.rollback_to(name)`

Savepoints setzen einen Checkpoint in der Transaktion, sodass Sie
einen Block innerer Arbeit verwerfen können, ohne den äußeren Commit
abzubrechen. Funktioniert auf allen drei Backends - SQLites
`SAVEPOINT` ist voll funktionsfähig, obwohl SQLite keine Sperren auf
Zeilenebene hat.

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        let mut account = Account::query().filter("id", id).first_or_fail().await?;
        account.balance = 200;
        account.save().await?;     // wird committet, wenn die äußere tx committet

        tx.savepoint("audit_trail").await?;

        let entry = AuditEntry::create(attrs! { actor_id: actor, ... }).await?;
        if audit_validation_failed(&entry) {
            tx.rollback_to("audit_trail").await?;
            // audit_trail-Zeile weg; Account-Update wartet noch auf Commit
        }

        Ok::<(), FrameworkError>(())
    })
}).await?;
```

Der Savepoint-Name wird wörtlich in das SQL interpoliert - verwenden
Sie einen statischen Identifier, spleißen Sie **keine**
Nutzereingaben ein.

### Verschachteltes `DB::transaction` wird zur Laufzeit abgelehnt

```rust
DB::transaction(|_outer| Box::pin(async move {
    let inner = DB::transaction(|_inner| Box::pin(async move {
        Ok::<(), FrameworkError>(())
    })).await;
    // inner is Err(FrameworkError::Database(
    //     "nested DB::transaction is not supported; use tx.savepoint(name) for nested rollback"
    // ))
    Ok::<(), FrameworkError>(())
})).await?;
```

SeaORMs `DatabaseConnection::begin()` komponiert nicht - aufgerufen
auf einer Connection, die bereits eine Transaktion hält, startet es
eine brandneue physische Transaktion, die unabhängig vom äußeren
Scope committet bzw. zurückrollt. Das ist eine stille Footgun für
die Datenintegrität, daher prüft `DB::transaction` vorab
`CURRENT_TX` und gibt einen Datenbankfehler zurück, statt die
falsche Semantik zu erzeugen. Verwenden Sie `tx.savepoint(name)`
für verschachteltes Verhalten.

### Wiederholung bei Deadlock - `DB::transaction_with_attempts`

Postgres-`SERIALIZABLE`-Reads und MySQL-Sperren auf Zeilenebene
können Serialization-Failure- / Deadlock-Fehler auslösen, die sich
durch Wiederholen der Transaktion lösen. `transaction_with_attempts`
führt die Closure jedes Mal von vorn aus, bis zu `attempts` Mal:

```rust
DB::transaction_with_attempts(3, |_tx| {
    Box::pin(async move {
        // SERIALIZABLE-isolierte Logik, die mit einer gleichzeitigen
        // tx in ein Race laufen und beim Commit SQLSTATE 40001 /
        // 40P01 auslösen kann.
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

Die Erkennung erfolgt über einen Substring-Vergleich des
Display-Strings gegen den inneren Fehler:

- Postgres-SQLSTATE `40001` (serialization_failure)
- Postgres-SQLSTATE `40P01` (deadlock_detected)
- Groß-/Kleinschreibung ignorierender `"deadlock"`-Substring (deckt
  MySQLs `Deadlock found when trying to get lock` und jeden
  nutzerseitig aufscheinenden Deadlock-String ab)

Beim letzten Versuch wird der Fehler unverändert durchgereicht. Die
Closure läuft bei jedem Versuch von vorn - fangen Sie eigenen
(owned) Zustand oder `Arc`s ein statt `&mut`-Referenzen, damit der
Retry-Pfad wohldefiniert ist.

> **Vorbehalt:** Da die Erkennung einen Groß-/Kleinschreibung
> ignorierenden `"deadlock"`-Substring einschließt (nötig für
> MySQL, dessen Treiber keinen SQLSTATE liefert), löst jeder innere
> Fehler, dessen `Display` das Wort enthält, eine Wiederholung aus.
> Wenn Sie aus einer `transaction_with_attempts`-Closure heraus
> eigene Fehler auslösen, vermeiden Sie "deadlock" in der Nachricht
> - sonst wiederholt sich ein unabhängiger Validierungsfehler bis
> zu `attempts` Mal, bevor er durchgereicht wird. Die
> Postgres-SQLSTATE-Treffer (`40001` / `40P01`) sind das
> verlässliche Signal; die Heuristik ist nur für MySQL.

### Manuelle Form - `DB::begin_transaction` + `*_with_tx`-Shims

Wenn die Lebensdauer der Transaktion nicht in eine Closure passt
(z. B. weil sie mehrere Kontrollfluss-Zweige überspannt), öffnen Sie
eine manuelle `Transaction` und binden Sie jede Operation explizit
per Opt-in ein:

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

tx.commit().await?;  // oder tx.rollback().await?;
```

Der manuelle Modus installiert `CURRENT_TX` **nicht**. Scopen Sie
einzelne Operationen mit `Builder::with_tx(&tx)` oder den
`Model::*_with_tx(&tx, ...)`-Shims in die Transaktion:

| Trait-Methode        | Manuelle Variante                            |
|---------------------|-------------------------------------------|
| `Model::create`     | `Model::create_with_tx(&tx, attrs)`       |
| `Model::save`       | `Model::save_with_tx(&tx)`                |
| `Model::update`     | `Model::update_with_tx(&tx, attrs)`       |
| `Model::delete`     | `Model::delete_with_tx(&tx)`              |
| `Model::force_delete` | `Model::force_delete_with_tx(&tx)`      |
| `Builder::*`        | `Builder::with_tx(&tx).*`                 |

Das Halten einer `Transaction` pinnt eine Pool-Connection für die
gesamte Lebensdauer des Handles. Auf SQLite hat der Pool nur eine
einzige Connection, sodass jeder parallele nicht-transaktionale Read
gegen dieselbe Datenbank blockiert, bis die Transaktion abgeschlossen
ist - **laden Sie alle Pre-Flight-Zeilen VOR
`DB::begin_transaction()`** und routen Sie jeden abhängigen Write
über die zurückgegebene `tx`.

`Transaction::commit` / `Transaction::rollback` konsumieren das
Handle und verlangen `Arc::try_unwrap` der inneren
SeaORM-Transaktion; wenn beim Commit / Rollback noch
`TxHandle`-Klone (von `tx.handle()` / `Builder::with_tx(&tx)`) am
Leben sind, schlagen beide mit einem Fehler "TxHandle clones still
alive" fehl. Die richtige Lösung ist, Ihre `Builder<M>` /
ausstehenden Handles vor dem Aufruf von `commit` zu droppen - das
Framework verweigert es, einen halb-uncommitteten Write gegen einen
parallelen Schreiber mit derselben tx in ein Race laufen zu lassen.

### Vorrang

Dreistufiger Vorrang beim Routing einer Operation durch eine
Connection:

1. **Override auf Builder-Ebene** - `Builder::with_tx(&tx)` oder
   ein `Model::*_with_tx(&tx, ...)`-Shim. Explizit schlägt umgebend.
2. **Umgebendes `CURRENT_TX`** - installiert von `DB::transaction` /
   `DB::transaction_with_attempts` für den Task-Scope der Closure.
3. **Pool-Fallback** - `DB::connection()` gibt das globale
   `DbConnection`-Singleton zurück.

Innerhalb von `DB::transaction(|tx| ...)` routet ein Aufruf von
`Builder::with_tx(&other_tx)` explizit genau diese eine Query durch
`other_tx` - am umgebenden `CURRENT_TX` vorbei. Das ist so gut wie
sicher ein Bug; der Override-Pfad existiert für die manuelle Form,
nicht um die eigene tx der Closure zu überschreiben.

### `with_tx` und globale Scopes

Ein Builder, der einen `tx_override` trägt, respektiert weiterhin
globale Scopes, benannte Scopes und den Eager-Load-Plan - der
Override ändert nur das Connection-Routing, nicht das SQL.

### Einschränkungen (v1)

- **Relations-Eager-Loads** - `Builder::with(["posts"])` und
  `Collection::load(["posts"])` routen die eager `IN (...)`-
  Subqueries durch `DB::connection()`, nicht durch die aktive
  Transaktion. Ausstehende Writes innerhalb einer
  `DB::transaction`-Closure sind für über `.with(...)` geladene
  Relationen **nicht** sichtbar. Scopen Sie tx-Arbeit vorerst auf
  direkte `Model::*`- / `Builder::*`- / `DB::table(...)`-Aufrufe;
  verschieben Sie Relations-Loads, bis der äußere Write gelandet
  ist (oder vor `DB::begin_transaction` auf dem manuellen Pfad). Das
  ist eine bekannte Nahtstelle - der Routing-Helper
  (`ExecutorChoice`) ist an jedem SQL-Blatt bereits vorhanden; der
  Blocker ist, dass `EagerLoadDispatch::eager_load` ein
  `&DatabaseConnection` (konkret) nimmt, das das Makro für jede
  Relations-Art erzeugt. Ein Folge-Sweep wird den Trait an den
  Dispatch-Helper anpassen.
- **DDL auf Postgres** - `DB::statement(...)` innerhalb einer
  Transaktion führt das DDL gegen die tx-Connection aus, was
  Postgres erlaubt; MySQL committet implizit und ist daher
  innerhalb einer Suprnova-Transaktion nicht unterstützt (das
  entspricht Laravels Vorbehalt bei `DB::transaction`).

## Scopes

Suprnova bietet zwei Scope-Varianten, analog zu Laravel:

- **Lokale Scopes** - Extension-Methoden auf dem Builder, deklariert
  pro Modell mit `#[suprnova::scopes(Model)]`. Jede freie Funktion im
  annotierten `impl`-Block wird sowohl zu `Model::name()` (ein
  statischer Starter) als auch zu `Builder::name()` (eine
  verkettbare Methode).
- **Globale Scopes** - Implementierungen von `GlobalScope<M>`,
  registriert beim Boot über `ScopeRegistry::register::<M, _>(scope)`.
  Jeder `Model::query()`-Aufruf legt sie automatisch mit an.

### Lokale Scopes

Deklarieren Sie lokale Scopes, indem Sie ihnen die Form
`fn(query: Builder<Self>, args...) -> Builder<Self>` geben:

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

// Als Starter oder als verkettbare Methode verwenden:
let active_users  = User::active().get().await?;
let popular_users = User::query().active().popular(500).get().await?;
```

Nicht-Scope-Methoden, die im selben `impl`-Block deklariert sind
(alles, dessen erster Parameter nicht `query: Builder<Self>` ist),
werden unverändert durchgereicht.

### Globale Scopes

Globale Scopes greifen bei jedem `Model::query()`-Aufruf. Der
klassische Anwendungsfall ist Multi-Tenancy - jeder Read wird auf
den aktuellen Tenant gescoped, ohne dass jeder Aufrufer den Filter
selbst durchfädeln muss.

```rust
use suprnova::eloquent::scopes::{GlobalScope, ScopeRegistry};

pub struct TenantScope;

impl GlobalScope<Article> for TenantScope {
    fn apply(&self, query: Builder<Article>) -> Builder<Article> {
        // Liest den aktuellen Tenant aus einem Task-Local /
        // AtomicI64 / wo auch immer Pro-Request-Zustand lebt.
        query.filter("tenant_id", current_tenant_id())
    }
}

// Beim Boot - typischerweise in Ihrem Provider-/Bootstrap-Modul:
ScopeRegistry::register::<Article, _>(TenantScope);

// Jeder Read wird automatisch auf den aktiven Tenant gescoped:
let scoped = Article::query().get().await?;
```

Mehrere Scopes pro Modell komponieren in Registrierungsreihenfolge -
der zuerst registrierte läuft zuerst, sodass seine Filterklauseln
zuerst in der WHERE-Kette erscheinen. UND-kombinierte Filter kümmert
die Reihenfolge nicht, aber links-nach-rechts zählt bei jeder
Klausel, deren Seiteneffekt-Reihenfolge sichtbar ist (z. B.
Ordering, Having, rohe Fragmente).

### Opt-out aus einem globalen Scope

Jedes Modell, das das Makro `#[suprnova::model]` berührt, erhält
zwei statische Helfer:

```rust
// Genau einen registrierten Scope per Typ umgehen. Andere Scopes greifen weiterhin.
let all_tenants = Article::without_global_scope::<TenantScope>().get().await?;

// Jeden registrierten Scope umgehen. Admin-Tooling-Muster.
let everything = Article::without_global_scopes().get().await?;
```

**Wichtig:** Die Opt-out-Helfer müssen der Einstiegspunkt sein.
`.without_global_scope::<S>()` an einen Builder anzuhängen, den
`Model::query()` bereits zurückgegeben hat, macht bereits gelaufene
Scopes nicht ungeschehen - `Model::query()` wendet Scopes eager zur
Konstruktionszeit an, sodass die Maske zu spät gesetzt wird.
Verwenden Sie für korrekte Semantik die statischen
Pro-Modell-Helfer (oben).

### Wo globale Scopes greifen

| Pfad | Greifen globale Scopes? |
|------|----------------------|
| `Model::query()` | Ja - der kanonische gescopte Einstiegspunkt |
| `Model::without_global_scope::<S>()` | Ja, minus `S` |
| `Model::without_global_scopes()` | Nein |
| `Model::find(id)` | Nein - PK-Lookup läuft direkt über SeaORM |
| `Model::find_many([...])` | Nein - gleicher Grund |
| `Model::all()` | Nein - gleicher Grund |

Das entspricht Laravel: `Eloquent\Model::find` löst kein
`addGlobalScopes` aus. Aufrufer, die gescopte PK-Lookups wollen,
verwenden `Self::query().filter("id", pk).first().await?`.

### Soft Deletes und globale Scopes koexistieren

`#[suprnova::model(soft_deletes)]` installiert den Filter
`deleted_at IS NULL` über einen separaten String-Tag-Mechanismus,
nicht über die typisierte Scope-Registry. Beide Schichten
komponieren:

- `Model::query()` filtert gelöschte Zeilen heraus UND führt jeden
  registrierten Scope aus.
- `Model::without_global_scopes()` verwirft registrierte Scopes,
  bewahrt aber den Soft-Delete-Filter - Admin-Tooling, das jeden
  Spaltensatz lesen will, schließt gelöschte Zeilen standardmäßig
  weiterhin aus.
- `Model::with_trashed()` und `Model::only_trashed()` überspringen
  die Soft-Delete-Filterung und umgehen auch die Registry (sie bauen
  einen frischen, ungescopten Builder). Kombinieren Sie sie mit
  `.without_global_scope::<S>()`, wenn Sie scope-bewusste Reads über
  gelöschte Zeilen brauchen.

## Relationen

Suprnova bietet jede Art von Eloquent-Relation. Sie werden im
`relations = { ... }`-Block auf `#[suprnova::model]` deklariert, und
das Makro erzeugt - pro deklarierter Relation - eine Methode auf der
Struktur, einen Loaded-Accessor (`<name>_loaded()`), einen
Count-Accessor (`<name>_count()`) und den Dispatcher-Arm, in den der
Eager Loader einsteigt. Dieser Abschnitt deckt die Form pro Art und
die Options-Tabelle ab; der Deep Dive zur Join-Key-Auflösung, zur
Morph-Registry, zu Pivot-Zeilen und zum Lowering des polymorphen
Enums lebt in [Eloquent Relationships](eloquent-relationships.md).
Die heute verfügbaren Relations-Arten:

| Art                | Kardinalität | Familienübergreifend | Basis |
|---------------------|----------|-----------------|-----------|
| `HasOne<R>`         | eins      | nein              | `IN`-Query auf `<parent>_id` |
| `BelongsTo<R>`      | eins      | nein              | `IN`-Query auf FK dieser Zeile |
| `HasMany<R>`        | viele     | nein              | wie `HasOne`, gibt `Vec<R>` zurück |
| `BelongsToMany<R, P>` | viele   | nein              | Pivot-Tabelle `P`, INNER JOIN + `pivot::<P>()` |
| `HasOneThrough<B, R>`  | eins   | nein              | Zwei-Query-JOIN `parent → B → R` |
| `HasManyThrough<B, R>` | viele  | nein              | wie oben, gibt `Vec<R>` zurück |
| `MorphOne<R>`       | eins      | ja             | `IN` + Filter `<name>_type = "<self>"` |
| `MorphMany<R>`      | viele     | ja             | wie `MorphOne`, gibt `Vec<R>` zurück |
| `MorphTo`           | eins      | ja (Kinder → viele Familien) | familienspezifisches Enum, an der Deklarationsstelle erzeugt |
| `MorphToMany<R, P>` | viele     | ja             | polymorpher m2m-Pivot `P` |
| `MorphedByMany<R, P>` | viele   | ja (invers)   | derselbe Pivot, andersrum gescannt |

### Syntax `relations = { ... }`

Jede Relations-Deklaration trägt dieselbe äußere Form: den
Relations-Namen, die Art, den verknüpften Typ (und Pivot-/
Zwischentypen, wo zutreffend) und einen `{ ... }`-Block mit Optionen.

```rust
use suprnova::model;

#[model(
    table = "users",
    relations = {
        // HasMany<R>
        posts: HasMany<crate::models::Post> {
            fk = "author_id",         // überschreibt Standard `user_id`
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

Gebräuchliche Optionen:

| Option                     | Relations-Arten                | Zweck |
|----------------------------|-------------------------------|---------|
| `fk = "..."`               | jede Art mit einer Kind-FK    | Spalte auf dem KIND, die auf das Elternteil zeigt. Standard = `<snake(parent_struct)>_id`. |
| `lk = "..."`               | Eins-/Viele-Arten                | Spalte auf dem ELTERNTEIL, die als Join-Key dient. Standard = `"id"`. |
| `related_key = "..."`      | `BelongsToMany`, `MorphToMany` | Der PK-SPALTENNAME auf der verknüpften Seite. Standard = `"id"`. Erforderlich, wenn das verknüpfte Modell einen Nicht-`id`-PK verwendet. |
| `with_pivot = ["...", ...]` | `BelongsToMany`, `MorphToMany` | Zusätzliche Spalten auf dem Pivot, die im Join auftauchen sollen. |
| `with_timestamps`          | `BelongsToMany`, `MorphToMany` | Setzt `created_at` / `updated_at` bei attach/sync. |
| `with_default = \|\| { ... }` | `BelongsTo`                 | Closure, die einen Standardwert erzeugt, wenn die FK null ist ODER das Elternteil fehlt. |
| `first_key`, `second_key`, `second_local_key` | `HasOneThrough`, `HasManyThrough` | JOIN-Key-Overrides - siehe den Through-Abschnitt unten. |
| `name = "..."`             | jede Morph-Art              | Morph-Familienname (z. B. `"commentable"`, `"taggable"`). Steuert die Spalten `<name>_id` / `<name>_type` auf dem Kind/Pivot. |
| `targets = [T1, T2, ...]`  | `MorphTo`                     | Die Liste konkreter Morph-Ziele. Das Makro erzeugt an der Deklarationsstelle ein `<Name>Morph`-Enum mit einer Variante pro Ziel plus `Unknown(String, i64)`. |
| `target_morph_type = "..."` | `MorphedByMany`              | Der Morph-Type-String, der die Zielfamilie auf dem Pivot identifiziert. |
| `pivot_table`, `pivot_foreign_key`, `pivot_related_key` | `BelongsToMany`, `MorphToMany` | Spalten-/Tabellen-Overrides auf der Pivot-Seite, wenn die Standardwerte nicht passen. |

### `HasOne<R>` und `BelongsTo<R>`

Eins-zu-eins in beide Richtungen. `HasOne` lebt auf der Elternseite
und ruft `R::query().filter(<fk>, <self.id>).first()` auf.
`BelongsTo` lebt auf der Kindseite, liest die FK von `self` ab und
ruft dann `R::query().filter(<owner_key>, <fk_value>).first()` auf.

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

`BelongsTo` unterstützt `with_default = || R { ... }`, was auslöst,
wenn entweder die FK null ist ODER die Elternzeile fehlt. Die
Standard-Closure läuft pro Aufruf (und pro eager geladener Zeile) -
perfekt für einen leeren Platzhalter, wenn ein gelöschter User noch
Kommentare hat:

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
// Immer Some - der Standardwert greift, wenn die User-Zeile fehlt.
let author = c.author().first().await?.unwrap();
```

### `HasMany<R>`

Eins-zu-viele auf der Elternseite. Gibt einen fluenten Builder
zurück; verketten Sie filter / order / latest / take / get / count
und terminieren Sie.

```rust
#[model(table = "users", relations = {
    posts: HasMany<crate::models::Post> {
        fk = "author_id",
    },
})]
pub struct User { /* ... */ }

let u = User::find(1).await?.unwrap();

// Jeder Post dieses Users, Standard-Ordering:
let posts: Vec<Post> = u.posts().get().await?;

// Gefiltert + geordnet + paginiert:
let recent = u.posts()
    .filter("published", true)
    .latest()                          // ORDER BY created_at DESC
    .take(10)
    .get()
    .await?;

// Nur COUNT - kein Zeilen-Fetching:
let total: i64 = u.posts().count().await?;
```

Verfügbare Terminal-Methoden: `.first()`, `.get()`, `.count()`.
Verfügbare verkettbare Filter: `.filter` / `.db_where`, `.filter_in`
/ `.where_in`, `.order_by`, `.latest`, `.oldest`, `.limit`, `.take`.

### `BelongsToMany<R, P>` - erstklassiges Pivot

Viele-zu-viele über einen mit `#[suprnova::model]` deklarierten
Pivot. Der Pivot ist ein erstklassiges Modell mit eigener
Zeilenidentität - kein Tupel, keine versteckte Hash-Map. Zwei
zentrale Vorteile gegenüber Laravels anonymer Pivot-Form:

1. Die Pivot-Zeile ist typsicher. Lesen Sie `with_pivot`-Spalten
   über `r.pivot::<P>().<column>`, niemals über `r.pivot.get("...")`.
2. Das Pivot-Modell ist vom Rest des Frameworks aus erreichbar
   (Factories, Scopes, Casts, Hooks) - genauso wie jedes andere
   Modell.

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

// Attach- + Sync-Mutatoren
u.roles().attach(admin.id).await?;
u.roles().attach_with(admin.id, attrs! { assigned_at: chrono::Utc::now() }).await?;
u.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
u.roles().detach(admin.id).await?;

// Pivot-Daten über den Pro-Zeile-Downcast-Accessor lesen:
let roles = u.roles().get().await?;
for r in &roles {
    let p: &RoleUser = r.pivot::<RoleUser>();
    println!("user {} got role {} at {:?}", p.user_id, p.role_id, p.assigned_at);
}
```

- `.attach(id)` - INSERT einer einzelnen Pivot-Zeile. Gibt bei
  Duplikaten einen Fehler, sofern Ihr Pivot es nicht erlaubt (das
  Framework dedupliziert nicht auf der Rust-Ebene; verwenden Sie
  `.sync` für Idempotenz).
- `.attach_with(id, attrs! { ... })` - INSERT mit zusätzlichen
  Pivot-Spalten. Setzt Timestamps, wenn `with_timestamps` aktiv ist.
- `.detach(id)` - DELETE der Pivot-Zeile(n), die Elternteil → id
  verknüpfen.
- `.sync([ids...])` - Diff-and-Apply: hängt an, was neu ist, trennt,
  was fehlt, lässt die Schnittmenge unangetastet. In eine
  Transaktion eingehüllt.

`.get()` gibt `Vec<R>` zurück, wobei der Pivot auf dem internen
`__pivot`-Feld jeder Zeile aufgestempelt ist. Der Accessor
`.pivot::<P>()` downcastet das `Arc<dyn Any>` auf den deklarierten
Pivot-Typ. Der Aufruf mit dem falschen Typ paniert - der Typ muss
zum deklarierten Pivot passen.

### `HasOneThrough<B, R>` und `HasManyThrough<B, R>`

Erreicht ein finales Ziel `R` über ein Zwischenglied `B`. Nützlich,
wenn die Relation zwei Tabellen durchläuft, Sie das Zwischenglied
aber nicht offenlegen müssen (`A → B → R`).

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

Der Dispatcher leitet JOIN-Keys aus Struktur-Namen ab. Overrides:

| Option              | Standard                          | Beschreibung |
|---------------------|----------------------------------|-------------|
| `first_key`         | `<snake(parent_struct)>_id`      | Spalte auf Zwischenglied `B`, die auf Elternteil `A` zeigt. |
| `second_key`        | `<snake(intermediate_struct)>_id` | Spalte auf dem finalen `R`, die auf Zwischenglied `B` zeigt. |
| `second_local_key`  | `"id"`                           | Spalte auf Zwischenglied `B`, auf die `second_key` matcht. Erforderlich, wenn `B` einen Nicht-`id`-PK verwendet. |

Die Primärschlüssel-Spalte des Elternteils wird aus der
`primary_key`-Deklaration des Modells gelesen (Standard `"id"`) - es
gibt keinen `local_key`-Override auf `HasManyThrough` /
`HasOneThrough`; ändern Sie den PK des Elternteils über das Attribut
`#[suprnova::model]`, wenn Sie einen Nicht-`id`-Elternschlüssel
brauchen.

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<crate::models::User, crate::models::Post> {
        first_key = "country_id",
        second_key = "author_id",
    },
})]
pub struct Country { /* ... */ }
```

### `MorphTo` mit `targets = [...]` und familienspezifischem Enum

Polymorphe Relationen zeigen mit einer Kindzeile auf eine von
mehreren Elternfamilien. Das Kind trägt ein Paar
`(<name>_id, <name>_type)`; die Spalte `*_type` enthält den
Morph-Type-String, den jedes Elternteil deklariert.

`MorphTo` lebt auf dem Kind. Seine Deklaration listet über
`targets = [...]` jede Elternfamilie auf, auf die es zeigen kann.
Das Makro erzeugt ein familienspezifisches Enum namens
`<RelationName>Morph` (passend zur PascalCase-Form des
Relations-Namens, mit dem Suffix `Morph`) mit einer Variante pro
Zieltyp plus `Unknown(String, i64)` für Legacy-Zeilen, deren
`<name>_type`-Wert zu keinem registrierten Ziel passt.

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
    // Legacy-/hängende Zeilen - `<name>_type` passt zu keinem Ziel,
    // ODER der morph_type passte, aber die Zeile bei `<name>_id` ist weg.
    CommentableMorph::Unknown(ty, id) => {
        eprintln!("comment {} points at unknown {ty}#{id}", c.id);
    }
}
```

Das Attribut `morph_type = "..."` auf jeder Ziel-Struktur ist das,
was der Loader beim Insert in die Spalte `<name>_type` des Kindes
schreibt und beim Read danach filtert. Ohne `morph_type` leitet das
Framework den Type-String aus `to_snake(struct_name)` ab.

Das `MorphTo`-Dispatch - wie das familienspezifische Enum die
richtige Variante wählt - befragt die Morph-Registry zur Laufzeit
(das Inventory, befüllt von jeder
`#[suprnova::model(morph_type = "...")]`-Deklaration). Für jedes
deklarierte Ziel schlägt der Fetch-Helper die `TypeId` des Ziels
nach, liest den registrierten `morph_type`-String und vergleicht ihn
mit dem gespeicherten `<name>_type`-Wert auf der Kindzeile. Der
erste Treffer gewinnt, in Deklarationsreihenfolge. Ziele ohne
explizites `morph_type`-Attribut fallen auf
`to_snake(target_type_name)` zurück - denselben Standardwert, den
`MorphMany` / `MorphOne` auf der Elternseite verwenden, um den
Type-String beim Schreiben aufzustempeln, sodass beide Seiten
aligned bleiben. Das bedeutet, dass benutzerdefinierte
`morph_type`-Werte (z. B. `morph_type = "blog_post"` auf einer
Struktur namens `Post`, oder jeder unkonventionelle String) korrekt
dispatchen, ohne Änderungen an der Deklarationsstelle.

### `MorphOne<R>` und `MorphMany<R>` - Elternseite

Die Umkehrrichtung von `MorphTo`: Ein Eltern-Typ deklariert das
polymorphe Eins-oder-Viele, das er besitzt. `MorphOne` gibt
`Option<R>` von `.first()` zurück; `MorphMany` gibt `Vec<R>` von
`.get()` zurück. Beide filtern das Paar `(<name>_id, <name>_type)`
des Kindes nach `self.id` und dem `morph_type` des Elternteils.

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
// post.comments() gibt nur Zeilen mit `commentable_type = "post"`
// zurück; video.comments() nur solche mit `commentable_type = "video"`.
```

Dieselbe verkettbare Oberfläche wie `HasMany` / `HasOne`: `.filter` /
`.db_where`, `.order_by` / `.latest` / `.oldest`, `.limit` / `.take`,
`.first` / `.get` / `.count`.

### `MorphToMany<R, P>` und `MorphedByMany<R, P>`

Polymorphes Viele-zu-viele. Der gemeinsame Pivot `P` trägt das
FK-Paar PLUS eine Discriminator-Spalte `<name>_type`. Ein Ende
deklariert `MorphToMany` (z. B. `Post.tags()`, `Video.tags()`), das
andere Ende deklariert ein `MorphedByMany` pro Zielfamilie (z. B.
`Tag.posts()`, `Tag.videos()`).

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

// Inverse: Tag deklariert ein MorphedByMany pro Zielfamilie.
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

// `attach` / `attach_with` / `detach` / `sync` funktionieren genauso
// wie bei BelongsToMany. Die Spalte `<name>_type` landet automatisch
// aus dem `morph_type` des aufrufenden Elternteils.
post.tags().attach(tag.id).await?;
video.tags().attach(tag.id).await?;          // unabhängiges Attachment
post.tags().sync([tag_a.id, tag_b.id]).await?;

// Umkehrrichtung - Tag splittet nach Familie:
let posts_with_tag:  Vec<Post>  = tag.posts().get().await?;   // typisiert "post"
let videos_with_tag: Vec<Video> = tag.videos().get().await?;  // typisiert "video"
```

`target_morph_type` bei `MorphedByMany` ist erforderlich, weil das
Makro an `Tag`s Deklarationsstelle das Attribut `morph_type = "..."`
des Ziels nicht introspizieren kann (es lebt in einem separaten
`#[suprnova::model]`-Aufruf). Es explizit zu setzen, hält jeden
`MorphedByMany`-Arm ehrlich darüber, welche Familie er scannt.

### Notausgang: handgeschriebene Relations-Methoden

Die in `relations = { ... }` deklarierten Relationen sind die
einzigen, die der Eager-Load-Dispatcher (und `with`, `with_count`
usw.) kennt. Wenn eine Relation zu ungewöhnlich für die Makro-Form
ist - zum Beispiel eine Query, die über zwei Pivots aggregiert, oder
eine typisierte Sicht auf eine denormalisierte Cache-Tabelle -
können Sie sie aus `relations = { ... }` weglassen und ein
gewöhnliches inhärentes `impl` schreiben:

```rust
impl User {
    /// Posts, die dieser User verfasst hat ODER in denen er getaggt
    /// ist. Kreuzt zwei Relationen und ist daher nicht als einzelne
    /// `relations = { ... }`-Deklaration ausdrückbar - von Hand
    /// geschrieben.
    pub async fn posts_touched(&self) -> Result<Vec<Post>, FrameworkError> {
        let authored: Vec<Post> = self.posts().get().await?;
        let tagged:   Vec<Post> = /* ...eigene Query... */;
        // ...zusammenführen + deduplizieren...
        Ok(/* ... */)
    }
}
```

Solche Methoden verlieren die Eager-Load-Unterstützung -
`User::with(["posts_touched"])` gibt einen Fehler, weil der
Dispatcher keinen Arm für `posts_touched` hat. Die
Im-Makro-Deklarationen bleiben der Weg, auf dem das Framework eager
laden, zählen, aggregieren und per Prädikat filtern kann.

### v1-Einschränkungen

Eine Handvoll Dinge, die die v1-Oberfläche zurückstellt. Jedes ist
auch an seiner Deklarationsstelle dokumentiert - hier zur
Sichtbarkeit gesammelt:

- **Morph-IDs sind nur `i64`.** `MorphTo::morph_id` ist fest auf
  `i64` verdrahtet, daher muss jedes Modell, das als `MorphTo`-Ziel
  verwendet wird, einen `i64`-Primärschlüssel deklarieren, und die
  Spalte `<name>_id` der Kind-Tabelle muss ebenfalls `i64` sein.
  String- / UUID-als-String-Morph-FKs sind v2.
- **Kein verschachteltes Eager Loading über `MorphTo`.** Das
  familienspezifische Enum löscht den Kindtyp, sodass ein
  gepunkteter Pfad wie `with(["commentable.user"])` nicht
  schwanzrekursiv aufgelöst werden kann - der Dispatcher gibt einen
  typisierten Fehler zurück. Lösen Sie es pro Familie auf, indem Sie
  auf dem Enum matchen und `with(["user"])` einzeln auf jeder
  Variante aufrufen.

## Eager Loading

Eager Loading vermeidet N+1-Queries. Statt `posts.len()` Queries, um
die Posts jedes Users zu holen, gibt Suprnova EINE Query pro
Top-Level-Relation aus, unabhängig davon, wie viele Elternzeilen
geladen werden.

Die vollständige Oberfläche - flache Liste, verschachtelte Pfade,
Count, Aggregate und per Prädikat gefilterte Eager Loads - wird über
die vom `#[suprnova::model]`-Makro erzeugten Helfer auf jedem Modell
erreicht:

```rust
// Einzelne Relation:
let users = User::with(["posts"]).get().await?;
for u in &users {
    for p in u.posts_loaded() { /* ... */ }
}

// Mehrere Relationen:
let users = User::with(["posts", "profile"]).get().await?;

// Verschachtelte Pfade - drei Queries (users + posts + comments), kein N+1:
let users = User::with(["posts.comments"]).get().await?;
let p1 = users[0].posts_loaded()[0];
let comments = p1.comments_loaded();

// Tiefere Verschachtelung funktioniert wie erwartet:
let users = User::with(["posts.comments.author"]).get().await?;

// Count neben den Elternzeilen:
let users = User::with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}

// Aggregate - Sum / Avg / Min / Max über eine Relations-Spalte. Der
// ergonomische Read ist der vom Makro erzeugte `<rel>_sum_of(col)`-Accessor.
let users = User::with_sum(("posts", "views")).get().await?;
let sum: f64 = users[0]
    .posts_sum_of("views")
    .expect("with_sum populated the cache");

// Mehrere Aggregate auf derselben Relation komponieren - der
// Cache-Key ist die breite Form `<rel>_<kind>_<col>`, sodass
// unterschiedliche Arten und Spalten nicht kollidieren:
let users = User::with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .with_min(("posts", "id"))
    .get()
    .await?;
let u = &users[0];
let sum = u.posts_sum_of("views").unwrap();   // Some(_) - Summe der views
let avg = u.posts_avg_of("views").unwrap();   // Some(_) - Durchschnitt der views
let min = u.posts_min_of("id").unwrap();      // Some(Some(_)) - nicht-leere Gruppe
let max = u.posts_max_of("id");               // None - with_max wurde nicht aufgerufen

// Die eager geladenen Kinder filtern. Das Makro erzeugt pro Relation
// einen typisierten statischen Helfer `with_where_<rel>(closure)`,
// sodass der Closure-Parametertyp inferiert wird - `Builder<Post>`
// muss nicht ausgeschrieben werden:
let users = User::with_where_posts(|q| q.filter("published", true))
    .get()
    .await?;
// Der zurückgegebene `Builder<User>` verkettet sich mit jeder
// anderen Base-Query-Builder-Methode:
let users = User::with_where_posts(|q| q.filter("published", true))
    .filter("active", true)
    .get()
    .await?;
// Die generische Form ist weiterhin verfügbar - nützlich, wenn der
// Relations-Name zur Laufzeit berechnet wird - aber Sie müssen den
// Zieltyp auf der Closure benennen:
let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// Jedes u.posts_loaded() enthält nur veröffentlichte Posts.
```

### Cache-Layout

Die Pro-Zeile-Cache-Zellen `__eager` werden wie folgt geschlüsselt:

- `<rel>` (allein der Relations-NAME) für `with` und `with_count`.
- `<rel>_<kind>_<col>` (z. B. `posts_sum_views`) für die vier
  Aggregat-Arten - `with_sum` / `with_avg` / `with_min` / `with_max`.
  Dieser breite Key lässt mehrere Aggregate auf derselben Relation
  auf derselben Zeile koexistieren, ohne sich gegenseitig zu
  überschreiben.

| Methode                              | Cache-Key            | Cache-Zellen-Typ   | Wert bei leerer Gruppe |
|-------------------------------------|----------------------|-------------------|-------------------|
| `with(["posts"])`                   | `posts`              | `Vec<Post>`       | `Vec::new()`      |
| `with(["profile"])`                 | `profile`            | `Option<Profile>` | `None`            |
| `with_count(["posts"])`             | `posts`              | `u64`             | `0`               |
| `with_sum(("posts","views"))`       | `posts_sum_views`    | `f64`             | `0.0`             |
| `with_avg(("posts","views"))`       | `posts_avg_views`    | `f64`             | `0.0`             |
| `with_min(("posts","id"))`          | `posts_min_id`       | `Option<f64>`     | `None`            |
| `with_max(("posts","id"))`          | `posts_max_id`       | `Option<f64>`     | `None`            |

Das Makro erzeugt passende Accessoren auf jedem Modell:

- `<rel>_loaded()` - für Collection-Relationen: `&[Post]` (paniert,
  wenn die Relation nicht eager geladen wurde). Für
  Einzelwert-Relationen: `Option<&Profile>`.
- `<rel>_count()` - `u64`. Paniert, wenn `with_count(["..."])` nicht
  aufgerufen wurde.
- `<rel>_sum_of(col)` / `<rel>_avg_of(col)` - geben `Option<f64>`
  zurück (`None`, wenn das passende `with_sum` / `with_avg` nicht
  aufgerufen wurde).
- `<rel>_min_of(col)` / `<rel>_max_of(col)` - geben
  `Option<Option<f64>>` zurück: die äußere `Option` bedeutet "wurde
  `with_min` / `with_max` aufgerufen?", die innere `Option` bedeutet
  "hat SQL NULL zurückgegeben, weil die Gruppe leer war?".

Die Accessoren sind die ergonomische Oberfläche - lesen Sie über
sie, statt direkt in `__eager.get_aggregate::<T>(...)` zu greifen.
Sie bauen unter der Haube denselben Cache-Key über
`eloquent::relations::aggregate_cache_key`.

### Aggregate auf derselben Relation komponieren

Der breite Cache-Key bedeutet, dass Sie in einer Query so viele
`with_*`-Aufrufe auf derselben Relation stapeln können, wie Sie
wollen - keine Kollisionen:

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

// Min/Max sind doppel-Option, weil SQL min/max bei leerer Menge NULL liefert:
match u.posts_min_of("id") {
    None              => panic!("with_min not called"),
    Some(None)        => println!("no posts yet"),
    Some(Some(min))   => println!("smallest post id: {min}"),
}

// Der Accessor gibt `None` zurück, wenn das passende `with_*` ausgelassen wurde:
assert!(u.posts_avg_of("score").is_none()); // nie mit col="score" aufgerufen
```

### Aggregate und INTEGER-Spalten

SUM über eine INTEGER-Spalte landet im Cache als `f64`. Die
Dispatcher-Arme versuchen zuerst `try_get::<Option<f64>>`, dann
fallen sie auf `try_get::<Option<i64>>().map(|n| n as f64)` zurück,
damit SQLites INTEGER-erhaltende COUNT/SUM-Typen nicht
stillschweigend zu `0.0` koerzieren. Lesen Sie über die
makro-erzeugten Accessoren, unabhängig vom Typ der Quellspalte.

### `with_where`-Prädikat-Routing

`User::with_where_posts(|q| q.filter("published", true))` wendet
eine Closure auf den inneren `Builder<Post>` an, BEVOR die
`filter_in(<fk>, parent_ids)`-IN-Query ausgegeben wird, sodass nur
passende Kindzeilen den Cache erreichen. Das Makro erzeugt pro
deklarierter Relation einen typisierten statischen Helfer
`with_where_<rel>`, sodass der Parametertyp der Closure aus der
Methodensignatur inferiert wird.

Die generische Form
`with_where(("posts", |q: Builder<Post>| q.filter("published", true)))`
ist weiterhin verfügbar - nützlich, wenn der Relations-Name zur
Laufzeit berechnet wird, oder wenn Sie bereits einen `Builder<User>`
halten und ein Prädikat anhängen wollen. Sie erfordert, den Zieltyp
auf der Closure zu benennen, weil das Prädikat durch ein
`Box<dyn Any>` läuft und Rust den Typ nicht allein aus dem
Relations-Namen inferieren kann. (Rusts Orphan Rules verbieten dem
Makro, eine typisierte Methode direkt auf `Builder<User>`
hinzuzufügen, daher wird die typisierte Kurzform nur auf dem Modell
angeboten - `User::with_where_<rel>` - nicht als
Builder-Chain-Methode.)

Bei den polymorphen Arten läuft das Prädikat gegen die Query der
verknüpften Tabelle - nicht gegen den Pivot-Scan.

`with_where` wird auf jeder Relations-Art unterstützt, AUSSER
`MorphTo`. Das familienspezifische Enum von MorphTo löscht den
Kindtyp, sodass kein einzelner `Builder<R>` alle Varianten abdeckt.
Verschachteltes Eager Loading über MorphTo wird in v1 ebenfalls
nicht unterstützt - `with(["commentable.user"])`, wobei
`commentable` ein `MorphTo` ist, gibt einen Fehler vom
Recurse-Eager-Load-Dispatcher zurück.

### `Collection::load` / `load_missing`

Wenn Sie Zeilen bereits geholt haben und Relationen nachträglich
eager laden möchten:

```rust
use suprnova::Collection;

let mut users: Collection<User> = User::all().await?.into();
users.load(["posts.comments"]).await?;
```

`load_missing` ist pro Zeile: Jede Zeile in der Collection wird
unabhängig partitioniert. Zeilen, die die benannte Relation bereits
gecacht haben, bleiben unangetastet; Zeilen, die sie nicht haben,
bekommen die Relation geladen. Spiegelt die Semantik von Laravels
`$collection->loadMissing(...)`.

Bei verschachtelten Pfaden wiederholt sich die Partitionierung auf
jeder Ebene. Bei `load_missing(["posts.comments"])`:

- Zeilen ohne gecachtes `posts` bekommen den VOLLEN Pfad geladen -
  `posts` plus deren `comments`.
- Zeilen MIT bereits gecachtem `posts` rekursieren in die gecachten
  Posts und laden `comments` nur auf den Posts, die noch keine
  gecachten Comments haben.

Dieselbe Pro-Zeile-Partitionierung wiederholt sich auf jedem
weiteren Segment eines längeren gepunkteten Pfads
(`"posts.comments.author"` usw.) - bei jedem Schritt bekommen nur
die Zeilen, denen dieses Segment fehlt, den Bulk-Load.

## Paginierung

Drei Paginator-Typen komponieren auf `Builder<M>`:

| Methode | Rückgabe | Queries pro Seite | Einsetzen, wenn |
|--------|---------|------------------|----------|
| `paginate(per_page)` | `LengthAwarePaginator<M>` | 2 (COUNT + LIMIT) | die UI die Gesamt-Seitenzahl braucht |
| `simple_paginate(per_page)` | `Paginator<M>` | 1 (LIMIT + 1) | große Tabellen; nur ein "Weiter"-Button |
| `cursor_paginate(per_page)` | `CursorPaginator<M>` | 1 (LIMIT + 1) | Infinite Scroll; tiefe Paginierung |

Alle drei implementieren `Serialize` mit der Laravel-Standard-
JSON-Form, sodass sie direkt an Inertia- / JSON-Konsumenten
ausgeliefert werden, ohne umgeformt zu werden.

### Längenbewusst

```rust
use suprnova::LengthAwarePaginator;

let page: LengthAwarePaginator<User> = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;

// page.data: Vec<User>
// page.total: u64 - Gesamtzeilenzahl über alle Seiten
// page.last_page: u64 - 1-basierter Index der letzten Seite
// page.current_page: u64
// page.per_page: u64
// page.from / page.to: Option<u64> - 1-basierte Fenstergrenzen
// page.path: Option<String> - optionale Basis-URL für die Link-Generierung
```

Das Parsen des Page-Parameters liest `?page=N` aus dem aktiven
Request über `Context::query_param`. Um mehrere Listen auf derselben
Seite mit eigenen Query-Keys zu paginieren, verwenden Sie
`paginate_using`:

```rust
let posts = Post::query().paginate_using("posts_page", 10).await?;
let comments = Comment::query().paginate_using("comments_page", 25).await?;
```

**JSON-Form:**

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

`path` wird aus dem JSON ausgelassen, wenn nicht gesetzt.

### Einfache Paginierung (kein Count)

`paginate` führt immer zwei Queries aus - ein `COUNT(*)` plus den
Seiten-Fetch. Bei großen Tabellen kann allein der Count die
Request-Zeit dominieren. `simple_paginate` überspringt den Count
vollständig; stattdessen holt es `per_page + 1` Zeilen und meldet
über das Flag `has_more`, ob eine nächste Seite existiert:

```rust
use suprnova::Paginator;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;

// page.has_more: bool - gab es eine zusätzliche Zeile über per_page hinaus?
// page.current_page, page.per_page, page.data, page.path: wie oben.
```

**JSON-Form:**

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true
}
```

### Cursor-Paginierung (Keyset)

Cursor-Paginierung ist die Wahl für Infinite Scroll, tiefe
Paginierung oder überall dort, wo eine stabile Zeilenreihenfolge mit
billigem O(1)-Seeking pro Seite mehr wert ist als eine numerische
Seiten-UI. Bidirektional - sie liest den Query-Parameter
`?cursor=<opaque>`, läuft je nach Richtung des Cursors vorwärts oder
rückwärts und gibt sowohl `next_cursor` als auch `prev_cursor` aus,
sofern die Nachbarn der Seite existieren (entspricht Laravels
`cursorPaginate()`).

```rust
use suprnova::CursorPaginator;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;

// page.data: Vec<User>
// page.per_page: u64
// page.next_cursor: Option<String> - opaker Cursor für die nächste Seite (None bei der letzten)
// page.prev_cursor: Option<String> - opaker Cursor für die vorherige Seite (None bei der ersten)
// page.path: Option<String>
```

Cursor sind **verschlüsselt und authentifiziert** über
`CursorPaginator::encode_value` - sie kodieren die Keyset-Grenze
(den Primärschlüssel des Modells) plus ein Richtungs-Tag,
AES-256-GCM-versiegelt mit dem `APP_KEY` des Frameworks. Manipulation
erzeugt einen 400-ParamParse-Fehler; der Cursor ist für den Client
opak und ohne den Schlüssel nicht fälschbar.

Der nächste Request übergibt den Cursor über `?cursor=<opaque>`:

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

Cursor-Paginierung **ersetzt** jedes existierende `ORDER BY` auf dem
Builder - eine stabile PK-ASC-Ordnung ist erforderlich, damit
`gt(boundary)` deterministisch schneiden kann.

**JSON-Form:**

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` und `prev_cursor` sind als JSON-Keys immer vorhanden
(ausgegeben als `null`, wenn nicht gesetzt), sodass Client-Schemas
sich auf die Anwesenheit des Felds verlassen können; `path` wird
ausgelassen, wenn nicht gesetzt.

### Fehler

| Bedingung | Variante | HTTP |
|-----------|---------|------|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| Ungültiger Cursor (fehlerhaftes Base64, JSON oder HMAC schlägt fehl) | `FrameworkError::Internal` von `Crypt::decrypt_string` | 500 |
| Zugrunde liegender DB-Fehler | `FrameworkError::Database` | 500 |

Ein Authentifizierungsfehler beim Cursor erscheint als `Internal`
(nicht `ParamParse`), damit ein manipulierter Cursor dem Client keine
Informationen auf Protokollebene verrät; der Response-Body trägt
weiterhin einen menschenlesbaren Grund.

### Query-Parameter außerhalb eines echten Requests lesen

Tests, Konsolenbefehle und Hintergrund-Worker laufen nicht innerhalb
eines hyper-Requests - daher gibt `Context::query_param("page")`
`None` zurück, und `paginate` fällt auf Seite 1 zurück. Tests, die
eine bestimmte Seite ansteuern müssen, können einen
Pro-Thread-Override installieren:

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

`test_set_query` / `test_clear_query` sind hinter dem Feature
`testing` verriegelt (standardmäßig aktiviert in
`framework/Cargo.toml`), sodass Release-Builds diese Oberfläche
niemals sehen.

## Chunking und Lazy-Iteration

Sieben Streaming-Einstiegspunkte auf `Builder<M>` lassen Sie große
Ergebnismengen mit begrenztem Speicher verarbeiten. Wählen Sie nach
Trade-off:

| Methode | Paginierung | Nebenläufigkeitssicher? | Rückgabe |
|--------|-----------|------------------|---------|
| `chunk(n, async \|batch\| { ... })` | OFFSET | Nein | `Result<(), _>` |
| `chunk_by_id(n, async \|batch\| { ... })` | PK-Cursor | **Ja** | `Result<(), _>` |
| `chunk_map(n, async \|batch\| { ... })` | OFFSET | Nein | `Collection<U>` |
| `each(async \|row\| { ... })` | OFFSET, Größe 1 | Nein | `Result<(), _>` |
| `lazy()` | PK-Cursor, Batch 1000 | **Ja** | `LazyCollection<M>` |
| `lazy_by_id(batch_size)` | PK-Cursor, eigene Batch-Größe | **Ja** | `LazyCollection<M>` |
| `cursor()` | Alias für `lazy()` | **Ja** | `LazyCollection<M>` |

### chunk - OFFSET-paginierte Batches

```rust
use suprnova::{Collection, Model};

User::query().chunk(100, |batch: Collection<User>| async move {
    for user in &batch {
        send_welcome_email(user).await?;
    }
    Ok(())
}).await?;
```

Die Closure erhält pro Batch eine `Collection<M>` - Slice-förmiger
Zugriff (`.iter()`, Indexierung) funktioniert direkt über `Deref`.

`chunk` ist OFFSET-paginiert und **nicht sicher bei gleichzeitigen
Inserts**: Zeilen, die vor dem Offset des nächsten Batches
eingefügt werden, werden übersprungen; Zeilen, die vor dem Offset
gelöscht werden, werden doppelt verarbeitet (was auch immer in ihren
Slot nachgerutscht ist). Verwenden Sie `chunk_by_id` für
produktionsreife Bulk-Verarbeitung gegen Tabellen unter
Schreiblast.

### chunk_by_id - PK-Cursor-Batches, nebenläufigkeitssicher

```rust
User::query().chunk_by_id(500, |batch| async move {
    for user in &batch {
        reindex_user(user).await?;
    }
    Ok(())
}).await?;
```

Jeder Batch filtert auf `WHERE id > last_id ORDER BY id ASC LIMIT
n`, sodass Zeilen, die mitten in der Iteration mit PKs über dem
Cursor eingefügt werden, in einem späteren Batch landen (oder von
einem nachfolgenden Lauf erfasst werden) - sie bringen niemals eine
ursprüngliche Zeile dazu, übersprungen oder verdoppelt zu werden.

`chunk_by_id` erfordert einen `i64`-Primärschlüssel. Modelle mit
`String`- / `Uuid`-PKs verwenden `chunk` mit dem OFFSET-Vorbehalt.
(Die Cursor-Form auf Nicht-`i64`-Keys zu verallgemeinern steht auf
der Follow-up-Liste.)

### chunk_map - chunk + Pro-Chunk-Map

```rust
let totals: Collection<i64> = Order::query()
    .chunk_map(1000, |batch| async move {
        let sum: i64 = batch.iter().map(|o| o.amount).sum();
        Ok(Collection::from_vec(vec![sum]))
    })
    .await?;
```

Mapt jeden Batch über `f`, konkateniert die gemappte Ausgabe und
gibt eine einzige `Collection<U>` zurück. Nur speicherbegrenzt, wenn
`U` strikt kleiner als `M` ist - wählen Sie das, wenn Sie
Zusammenfassungen erzeugen (Pro-Batch-Summen, IDs, Aggregate) statt
transformierter Zeilen.

### each - eine Zeile auf einmal, OFFSET

```rust
User::query().each(|user| async move {
    send_welcome_email(&user).await?;
    Ok(())
}).await?;
```

Zucker für `chunk(1, ...)` - eine Query pro Zeile. Für große
Datensätze wechseln Sie zu `lazy()`, das intern in Batches arbeitet
(Standard 1000 Zeilen pro Fetch), dem Konsumenten aber weiterhin
eine Zeile auf einmal zeigt.

### lazy / lazy_by_id / cursor - Streams

```rust
let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

`lazy()` gibt eine `LazyCollection<M>` zurück - einen
`Send`-Stream-Wrapper, der pro Zeile `Result<M, FrameworkError>`
liefert. Backpressure funktioniert natürlich: Ein langsamer
Konsument parkt am `await`-Punkt, und der nächste Batch wird erst
geholt, wenn der In-Memory-Puffer sich entleert.

`lazy()` bildet Batches über den PK-Cursor mit einer Standardgröße
von 1000 Zeilen. Überschreiben Sie die Batch-Größe mit
`lazy_by_id(500)`. `cursor()` ist der Laravel-Name und ein
Zero-Cost-Alias für `lazy()`.

Dieselbe `i64`-PK-Einschränkung wie bei `chunk_by_id`.

### Eager Loads innerhalb von Chunks

Alle sieben Einstiegspunkte **lehnen `.with(...)` von vornherein**
mit einem sichtbaren `FrameworkError::internal` ab. Der
Batch-übergreifende Klon des Builders verwirft den
typ-gelöschten Eager-Load-Plan (sein geboxtes `dyn Any`-Prädikat ist
nicht klonbar, ohne die öffentliche API zu verengen), sodass das
Einhalten des Plans über Batches hinweg stillschweigend
inkonsistent wäre. Wenden Sie `.with(...)` bei Bedarf innerhalb der
Pro-Chunk-Closure erneut an - die `Collection<M>` jedes Batches
komponiert mit `load(...)` / `load_missing(...)`:

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

`Collection<T>` ist Suprnovas Laravel-förmige Collection - der
Rückgabetyp von `Builder::get` (wobei `T` das Modell ist), von
`Model::all`, von `pluck` / `chunk_map` und von jedem anderen
Terminal, das mehr als eine Zeile liefert. Sie dereferenziert zu
`&[T]`, sodass bestehende Vec-Aufrufstellen ohne Änderungen
weiterlaufen; die Laravel-Oberfläche ist darüber komponiert. Dieser
Abschnitt ist die Alltagsoberfläche; der vollständige
Methoden-Index, die Trennung generisch vs. modellbewusst, der
Streaming-Wrapper `LazyCollection<M>` und die
Ausleihen-vs-Verbrauchen-Regeln stehen in
[Eloquent Collections](eloquent-collections.md).

### Generische Oberfläche

Verfügbar auf jeder `Collection<T>`, unabhängig von `T`:

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9]);

nums.first();              // Some(&3)
nums.last();               // Some(&9)
nums.len();                // 6
nums.is_empty();           // false
nums.contains(&4);         // true
// Prädikat-Closures erhalten `&&T` - beachten Sie den doppelten Deref `**n`:
nums.first_where(|n| **n > 3);    // Some(&4)
nums.contains_where(|n| **n > 8); // true
// Für eine Zählung führen Sie das Prädikat inline aus: `nums.iter().filter(|n| **n > 2).count()` - 4
```

Transformationen verbrauchen `self` und geben eine neue `Collection`
zurück:

```rust
let doubled: Collection<i32> = nums.clone().map(|n| n * 2);
let evens:   Collection<i32> = nums.clone().filter(|n| n % 2 == 0);
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(2); // [[3,1],[4,1],[5,9]]
let unique:  Collection<i32> = nums.clone().unique();
let sorted:  Collection<i32> = nums.clone().sort();
```

### Modellbewusste Methoden auf `Collection<M>`

Wenn `T` ein Modell ist, laufen zusätzliche string-geschlüsselte
Methoden über den vom Makro erzeugten Accessor `field_value(name)`:

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

Das closure-basierte `pluck_by` ist die typisierte Alternative -
nützlich, wenn der Feldname sonst einen String-Lookup erfordern
würde, den das Typsystem nicht prüfen kann:

```rust
let names: Collection<String> = users.pluck_by(|u| u.name.clone());
```

Pro Zeile gibt `field_value(name)` `Option<serde_json::Value>`
zurück - `None`, wenn der Spaltenname zu keinem deklarierten Feld
passt. Benutzerdefinierte Casts, die nicht serialisieren können,
erscheinen ebenfalls als `None`. Die string-geschlüsselten Methoden
überspringen diese Zeilen stillschweigend; die Closure-Form bricht
im Closure-Body ab, sodass der Aufrufer entscheiden kann.

### Streaming über `LazyCollection`

Für Datensätze, die zu groß zum Materialisieren sind, geben
`Builder::lazy()` / `lazy_by_id(n)` / `cursor()` eine
`LazyCollection<M>` zurück - einen `Stream`-Wrapper, der Zeilen in
PK-Cursor-Batches holt. Siehe
[Chunking und Lazy-Iteration](#chunking-und-lazy-iteration).

### Eager Loading auf einer Collection

`Collection::load(["posts"])` / `load_missing(["posts"])` führen
denselben Eager-Load-Dispatch aus, den eine `Builder::with(...)`-
Kette erzeugt, aber gegen eine bestehende Collection. `load_missing`
ist pro Zeile: Jede Zeile in der Collection wird in die Eimer
"braucht Load" / "bereits geladen" partitioniert, und nur die
fehlenden bekommen den Bulk-Load. Siehe
[Eager Loading](#eager-loading).

## Mass Assignment

### Fillable-Allowlist

```rust
#[model(
    table = "users",
    fillable = ["name", "email"],
)]
pub struct User { /* ... */ }

User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    admin: true,    // zur Laufzeit stillschweigend verworfen - nicht in fillable
}).await?;
```

### Guarded-Denylist

`guarded` ist die Umkehrung - jedes Feld ist befüllbar, AUSSER den
guarded-Feldern. Schließt sich mit `fillable` gegenseitig aus; beide
gleichzeitig zu verwenden ist ein Compile-Time-Fehler vom Makro.

```rust
#[model(
    table = "posts",
    guarded = ["id", "user_id"],   // alles andere ist befüllbar
)]
pub struct Post { /* ... */ }
```

### Standard-Policy

Wenn weder `fillable` noch `guarded` gesetzt ist, lautet die
Standard-Policy `guarded = ["id"]` (oder was auch immer
`primary_key = "..."` auflöst) - jedes Feld ist befüllbar außer dem
Primärschlüssel. Das entspricht Laravels Standard "alle Felder
befüllbar außer dem PK".

### Notausgang `unguarded(closure)`

`unguarded(closure)` schaltet den Filter für einen Block ab:

```rust
use suprnova::eloquent::unguarded;

// Den Filter für ein einmaliges Daten-Migrationsskript umgehen:
unguarded(|| async {
    User::create(attrs! {
        name: "Bootstrap",
        email: "boot@example.com",
        admin: true,    // innerhalb der Closure zuweisbar
    }).await
}).await?;
```

Implementierung: ein `tokio::task_local!`-Bool, den der Filter
`Fillable::apply` vor der Ausführung prüft. Task-lokal bedeutet, dass
gleichzeitige Requests vom `unguarded`-Scope eines anderen Tasks
nicht betroffen sind.

## Casts

Casts laufen an der Grenze zwischen Storage (Spaltenwert) und
Runtime (Modellfeld). Jeder Cast-Typ implementiert den Trait `Cast`.
Eingebaute Casts decken Laravels vollständiges Set ab; Nutzer
registrieren eigene Casts über den Trait. Dieser Abschnitt ist der
Schnellreferenz-Index; der vollständige Pro-Cast-Vertrag - primitiv,
temporal, strukturiert, Enum, verschlüsselt, gehasht, plus das
Runtime-Override-Makro `casts!` - lebt in
[Eloquent Casts, Accessors & Mutators](eloquent-mutators.md).

### Nur explizit

Casts werden in `#[model(casts = { ... })]` deklariert - es gibt
keine automatische Erkennung aus Feldtypen. Ein Feld `prefs: Json`
wird nicht implizit zu `AsJson`; Sie schreiben
`casts = { prefs = AsJson }`. Begründung: Sie sollten das Modell
lesen und genau wissen können, was an den Storage-Grenzen läuft.
Keine Magie.

### Beispiel

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

### Vollständige Laravel-Cast-Liste und Suprnova-Mapping

| Laravel-Cast | Suprnova-Cast | Runtime-Typ |
|--------------|---------------|--------------|
| `bool`, `boolean` | `AsBool` | `bool` |
| `int`, `integer` | `AsInt<I>` | `I: PrimInt` |
| `float`, `double`, `real` | `AsFloat` | `f64` |
| `decimal:N` | `AsDecimal<N>` | `rust_decimal::Decimal` |
| `string` | `AsString` | `String` |
| `array` | `AsArray<T>` | `Vec<T>` (JSON-kodiert) |
| `object` | `AsObject<T>` | `T: Serialize + DeserializeOwned` |
| `collection` | `AsCollection<T>` | `Collection<T>` |
| `json` | `AsJson<T>` | `T` (rohe JSON-Spalte) |
| `date`, `date:format` | `AsDate` | `chrono::NaiveDate` |
| `datetime`, `datetime:format` | `AsDateTime` | `chrono::DateTime<Utc>` |
| `immutable_date` | `AsImmutableDate` | `chrono::NaiveDate` |
| `immutable_datetime` | `AsImmutableDateTime` | `chrono::DateTime<Utc>` |
| `timestamp` | `AsTimestamp` | `i64` (Unix-Epoche) |
| `encrypted` | `AsEncrypted` | `String` (verschlüsselt über `Crypt`) |
| `encrypted:array` | `AsEncryptedArray<T>` | `Vec<T>` (JSON + verschlüsselt) |
| `encrypted:object` | `AsEncryptedObject<T>` | `T` (JSON + verschlüsselt) |
| `encrypted:collection` | `AsEncryptedCollection<T>` | `Collection<T>` |
| `EnumClass::class` | `AsEnum<E>` | `E: EnumString + AsRefStr` |
| `AsArrayObject::class` | `AsArrayObject<T>` | `IndexMap<String, T>` |
| `hashed` | `AsHashed` | `String` (`Hash::make` beim Write; entschlüsselt nie) |

22 Casts insgesamt. Die meisten mappen eins-zu-eins auf Laravel;
`AsOptionalDateTime` (verwendet von `soft_deletes`) wird vom Makro
automatisch injiziert, wenn die Soft-Delete-Spalte
`Option<DateTime<Utc>>` ist.

### Fehlermodi verschlüsselter Casts

Die vier `AsEncrypted*`-Casts leiten jede Ver-/Entschlüsselung über
die `Crypt`-Facade (geschlüsselt mit `APP_KEY`). Wenn die
Entschlüsselung fehlschlägt - falscher Schlüssel, abgeschnittener
Chiffretext, manipulierte Bytes, AEAD-Tag-Mismatch - liefert der
Cast einen klaren `FrameworkError::Internal` von
`Cast::from_storage`. Es gibt keinen stillen Fallback auf Müll:

- Das Laden einer Zeile über `Model::find` / `Model::query()` reicht
  den Entschlüsselungsfehler durch und (gemäß dem makro-generierten
  `From<inner::Model>`) paniert mit
  `cast from_storage failed - corrupt data in database column`.
  Operatoren sehen den Fehler sofort in den Logs; das Modell trägt
  niemals plausibel-aber-falschen Klartext.
- Der Cast `AsHashed` ist Einweg; er entschlüsselt nie, daher trifft
  dieser Fehlermodus nicht zu.

Das entspricht Laravels `encrypted`-Cast: Ein falscher `APP_KEY`
gegen eine bestehende verschlüsselte Spalte ist ein harter Fehler,
niemals ein stilles `null` / ein leerer String.

### `APP_KEY`-Rotation

Suprnova unterstützt Zero-Downtime-Schlüsselrotation über einen
Schlüssel-*Ring*: Der aktuelle `APP_KEY` verschlüsselt; eine
optionale Env-Variable `APP_KEY_PREVIOUS` (kommagetrennt, älteste
zuerst) liefert Entschlüsselungs-Fallbacks für Daten, die unter
älteren Schlüsseln geschrieben wurden. Verschlüsselung verwendet
*immer* den aktuellen Schlüssel - vorherige Schlüssel wirken nur bei
der Entschlüsselung mit.

Jede Entschlüsselung, die auf einen vorherigen Schlüssel durchfällt,
gibt eine `tracing::warn!`-Zeile mit dem Index des vorherigen
Schlüssels aus. Die Log-Payload lässt Klartext und Chiffretext
bewusst aus; nur die Tatsache der Rotation plus einen umsetzbaren
Neuverschlüsselungs-Hinweis.

**Rotationsablauf** (Zero-Downtime, sicher für Produktion):

1. Einen neuen Schlüssel prägen: `suprnova key:generate` (schreibt
   nach stdout).
2. Den alten Schlüssel nach `APP_KEY_PREVIOUS` verschieben und
   `APP_KEY` auf den neuen Wert setzen:
   ```
   APP_KEY_PREVIOUS=<old_key>
   APP_KEY=<new_key>
   ```
3. Deployen. Neue Writes verwenden den neuen Schlüssel; bestehende
   Zeilen entschlüsseln weiterhin über den Fallback auf den
   vorherigen Schlüssel. Warnungen in den Logs identifizieren
   Spalten, die noch von `APP_KEY_PREVIOUS` abhängen.
4. Einen Neuverschlüsselungs-Durchlauf ausführen. Für jedes Modell
   mit verschlüsselten Casts:
   ```rust
   for chunk in User::query().chunk(500).await? {
       for user in chunk {
           // Touch + save schreibt jede Cast-Spalte unter dem
           // aktuellen Schlüssel neu. `Cast::to_storage` greift
           // immer auf den aktuellen Ring-Eintrag zu.
           user.save().await?;
       }
   }
   ```
   Das ist idempotent - Zeilen, die bereits auf dem neuen Schlüssel
   sind, sind einfach ein No-Op.
5. Sobald die Logs keine `APP_KEY_PREVIOUS`-Warnungen mehr zeigen
   (geben Sie dem Batch + jeglichen soft-gelöschten / archivierten
   Daten ein großzügiges Zeitfenster), entfernen Sie
   `APP_KEY_PREVIOUS` aus der Umgebung und deployen Sie erneut.

**Mehrstufige Rotation.** Wenn Sie erneut rotieren, bevor der
vorherige Durchlauf abgeschlossen ist, hängen Sie an:
`APP_KEY_PREVIOUS=<oldest>,<previous>`. Der Ring probiert jeden
vorherigen Schlüssel in Reihenfolge durch. Die Liste ist auf 8
Einträge begrenzt - eine realistische Kette umfasst 1-3 (eine
laufende Rotation, vielleicht eine zuvor ins Stocken geratene) und
eine längere Liste ist so gut wie immer ein Config-Templating-
Unfall; das Überschreiten der Obergrenze lässt den Boot mit einer
umsetzbaren Diagnose scheitern, statt stillschweigend einen
Schlüssel fallen zu lassen, von dem der Operator noch abhängen
könnte.

**Einschränkungen.**

- Ein fehlerhafter Eintrag in `APP_KEY_PREVIOUS` lässt den Boot
  sichtbar scheitern (genau wie ein fehlerhafter `APP_KEY`) - ein
  halb rotiertes Secret sollte niemals stillschweigend degradieren.
- Mehr als 8 Einträge in `APP_KEY_PREVIOUS` lassen den Boot sichtbar
  scheitern - siehe [`suprnova::crypto::MAX_PREVIOUS_KEYS`].
- Leere Einträge in der Liste (z. B. nachgestellte Kommas aus
  templatisierter Konfiguration) werden als "kein Schlüssel in
  diesem Slot" toleriert - kein Fehler.
- Das Wire-Format ist gegenüber dem Single-Key-Layout vor der
  Rotation unverändert: Im Chiffretext ist kein Schlüssel-Identifier
  eingebettet. Der Ring probiert jeden Schlüssel der Reihe nach zur
  Entschlüsselung, bis einer erfolgreich ist.

### Runtime-Cast-Override - `with_casts`

```rust
let users = User::query()
    .with_casts(suprnova::casts! { birthdate = AsDateTime })
    .get()
    .await?;
```

`with_casts` überschreibt die deklarierten Casts des Modells für
die Dauer einer einzelnen Query - nützlich, wenn eine rohe Spalte
aus einem Join / einer View / `select_raw` zurückkommt und eine
andere Typkoerzion braucht als der Standard des Modells.

### Benutzerdefinierte Casts

Benutzerdefinierte Casts implementieren `Cast`:

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

Der Trait `Cast` wird zusammen mit den primitiven Casts
ausgeliefert. Benutzerdefinierte Casts können entweder
`String`-Storage verwenden (bei JSON-Kodierung) oder jeden der von
SeaORM unterstützten skalaren Typen (`i64`, `f64`, `bool`,
`Vec<u8>`).

## Accessoren und Mutatoren

### Accessoren

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

Wenn `user.to_array()` läuft (oder `user.to_json()`, das dorthin
delegiert), wird der Accessor `full_name` aufgerufen und sein
Rückgabewert in die JSON-Ausgabe eingefügt. `user.full_name()` aus
Rust aufzurufen ist einfach ein gewöhnlicher Methodenaufruf.

### Mutatoren

Mutatoren laufen vor dem Storage:

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

`user.password = "secret".into()` aufzurufen weist den rohen Wert
direkt zu, ohne den Mutator auszuführen. Um den Mutator-Pfad
auszuführen, rufen Sie `user.set_password(json!("secret"))` auf oder
verwenden Sie den JSON-Pfad
(`user.fill(attrs!{password: "secret"})`), der automatisch über den
Mutator läuft, weil `"password"` in `mutators = [...]` aufgeführt
ist.

### Wie das Routing funktioniert

- **Serialisierung (`to_array` → `Value`, `to_json` → `String`)**
  führt Accessoren aus. Jeder in `appends = [...]` aufgeführte
  Feldname wird zu einem Aufruf von `self.<name>()`; der
  Rückgabewert wird in die JSON-Ausgabe eingefügt. `to_json()` ist
  ein dünner Wrapper: `serde_json::to_string(&self.to_array())`.
- **Fill-förmige Writes (`fill`, `create`, `update`)** laufen über
  Mutatoren. Jeder in `mutators = [...]` aufgeführte Feldname wird
  zu einem Aufruf von `self.set_<field>(value)` statt einer
  direkten Zuweisung.

Die Makros `#[accessor]` und `#[mutator]` auf Funktionsebene
erzeugen Registry-Einträge, die die Serialisierungs- / Fill-Pfade
des Makros durchlaufen.

### Fehlerhafte Werte sind Fehler, keine Standardwerte

Ein Wert, der sich nicht in den Typ seines Feldes dekodieren lässt,
lässt den Write fehlschlagen und benennt das Feld:

```rust
let err = user.fill(attrs! { age: "not a number" }).unwrap_err();
// ValidationError { field: "age", message: "could not decode the
// supplied value: invalid type: string \"not a number\", expected i32" }
```

Das Modell bleibt unangetastet - ein abgelehntes `fill` wendet
nichts an.

Zwei benachbarte Fälle verhalten sich absichtlich unterschiedlich:

- Eine **unbekannte Spalte** wird weiterhin stillschweigend
  übersprungen, passend zu Laravels `$model->fill()`. Eine Spalte
  nicht zu kennen ist nicht dasselbe, wie für eine bekannte Spalte
  einen kaputten Wert zu erhalten.
- Eine durch `fillable` / `guarded` ausgeschlossene Spalte wird vom
  Mass-Assignment-Filter *vor* dem Dekodieren verworfen, sodass auch
  ein fehlerhafter Wert für ein Feld, das der Aufrufer nicht setzen
  darf, still bleibt. Dort einen Fehler zu werfen würde einem
  unautorisierten Aufrufer verraten, welche Spalten existieren.

Numerische Erweiterung ist kein Typfehler: Ein JSON-Integer
dekodiert normal in ein `f64`-Feld.

> Vor v0.8.0 wurde ein fehlerhafter Wert stillschweigend durch den
> `Default` des Feldes ersetzt, und der Aufruf gab `Ok` zurück -
> `fill(attrs!{ age: "abc" })` setzte `age = 0` und meldete Erfolg.
> Wenn Sie sich auf diese Koerzion verlassen haben, validieren oder
> konvertieren Sie, bevor Sie `fill` aufrufen.

### Hidden / visible

```rust
#[model(
    table = "users",
    hidden = ["password", "remember_token"],
)]
pub struct User { /* ... */ }
```

`hidden = [...]` ist eine Denylist - jede Spalte außer den
aufgeführten serialisiert. `visible = [...]` ist die inklusive
Form - nur die aufgeführten serialisieren. Schließt sich zur
Compile-Zeit gegenseitig aus.

## Timestamps

Wenn sowohl die Spalte `created_at` als auch `updated_at`
existiert, erkennt das Makro sie automatisch und aktiviert das
Timestamp-Tracking:

- `created_at` wird bei `save()` für neue Zeilen auf `Utc::now()`
  gesetzt.
- `updated_at` wird bei jedem `save()` auf `Utc::now()` gesetzt.

Die automatische Erkennung ist konservativ: Wenn die Struktur nur
eine der beiden Spalten hat, gibt das Makro einen Fehler, damit ein
Tippfehler (`craeted_at`) Timestamps nicht stillschweigend
deaktiviert. Setzen Sie `timestamps = false`, um vollständig
auszusteigen.

### Auto-Timestamps deaktivieren

```rust
#[model(table = "audit_logs", timestamps = false)]
pub struct AuditLog {
    pub id: i64,
    pub event: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    // Kein updated_at-Feld - aber timestamps = false unterdrückt
    // auch den Fehler `only one column found` des Makros.
}
```

### `touch()` - updated_at erhöhen, ohne andere Änderungen

```rust
user.touch().await?;
```

`touch()` gibt `UPDATE table SET updated_at = ? WHERE pk = ?` aus -
atomar, kein Read-Modify-Write. Das Makro erzeugt eine
`Touchable`-Implementierung auf jedem Modell mit Timestamps.

### Übergeordnete Modelle berühren

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

Nachdem ein Kommentar erstellt, gespeichert, aktualisiert oder gelöscht wurde, wird das `updated_at` seines Beitrags erhöht - ein `UPDATE posts SET updated_at = ? WHERE id = ?`, kein `SELECT`. Genau das braucht ein an `post.updated_at` hängender Cache-Schlüssel, um korrekt zu bleiben, wenn sich nur ein untergeordnetes Modell geändert hat.

Jeder Name in `touches` muss eine `BelongsTo`-Relation sein, die im selben Block `relations = { ... }` deklariert ist. Ein Name, der sich nicht auflösen lässt oder zu einer anderen Relationsart auflöst, ist ein Kompilierungsfehler statt einer Überraschung beim ersten Speichern. Polymorphe (`MorphTo`) übergeordnete Modelle können noch nicht berührt werden.

Ein übergeordnetes Modell mit `timestamps = false` wird **übersprungen**: kein Fehler, kein Schreibvorgang, und das Speichern des untergeordneten Modells gibt weiterhin `Ok` zurück. Dasselbe gilt für ein über einen `NULL`-Fremdschlüssel erreichtes oder für ein soft-gelöschtes übergeordnetes Modell.

Das Berühren läuft auf demselben Executor wie der auslösende Schreibvorgang; innerhalb einer `DB::transaction`-Closure gehört es daher zu dieser Transaktion und ein Rollback macht es rückgängig.

### Warum Suprnova abweicht

Laravels `touchOwners` lädt jedes übergeordnete Modell und steigt rekursiv auf, sodass das Speichern eines Kommentars auch die eigenen übergeordneten Modelle des Beitrags aktualisiert und das `saved`-Event jedes übergeordneten Modells auslöst. Suprnova löst das übergeordnete Modell über das Relationsregister auf und schreibt die Spalte direkt - eine Anweisung pro berührter Relation, keine Hydratisierung. Die Kaskade ist daher nur eine Ebene tief und löst keine Events auf übergeordneten Modellen aus. Das ist der Preis für ein Speichern, das pro berührter Relation keinen `SELECT` ausführt. Verwenden Sie einen Observer, wenn Sie die Aktualisierung des Großelternmodells oder das Event benötigen.

`restore()` eines soft-gelöschten untergeordneten Modells berührt seine übergeordneten Modelle nicht. Laravels `restore` läuft über `save`; Suprnovas ist ein direktes `UPDATE deleted_at = NULL`.

### Format

Immer ISO 8601 mit UTC. Kein Override für
`Model::$timestampsFormat` (laut der
Divergenz-von-Eloquent-Tabelle - Frontend-Interop kommt zuerst;
Locale-Formatierung gehört in die i18n-Schicht).

## Observers und Lifecycle-Events

Jedes Modell durchläuft einen festen 16-Event-Lifecycle, während es
durch `create` / `save` / `update` / `delete` / `restore` /
`replicate` / Builder-Query-Pfade läuft. Listener können sich in
jedes Event einhängen, um zu protokollieren, zu auditieren,
Seiteneffekte auszulösen, zu validieren oder die laufende Operation
abzubrechen.

### Die 16 Lifecycle-Events

Events teilen sich nach Abbrechbarkeit in zwei Gruppen:

**Abbrechbar (5)** - feuern VOR dem Datenbank-Write. Ein Listener,
der `EventResult::cancel("reason")` zurückgibt, bricht die
Operation mit `FrameworkError::bad_request(reason)` ab.

| Event       | Wann                                      | Payload                                                 |
|-------------|-------------------------------------------|---------------------------------------------------------|
| `Saving`    | Vor sowohl `create` als auch `save`           | `Arc<Mutex<Attrs>>` + `is_creating: bool`               |
| `Creating`  | Vor `create`                           | `Arc<Mutex<Attrs>>`                                     |
| `Updating`  | Vor `save` / `update` auf existierender Zeile  | Modell-Snapshot vor dem Update + `Arc<Mutex<Attrs>>`         |
| `Deleting`  | Vor `delete` (soft oder hard)            | Modell + `is_force: bool` (Force-Delete bei Soft-Delete)  |
| `Restoring` | Vor `restore` auf Soft-Delete-Modell     | Modell                                                   |

**Nicht abbrechbar (11)** - feuern NACH der Operation.
Listener-Fehler werden durchgereicht, können aber einen bereits
gelandeten Write nicht mehr stoppen.

| Event           | Wann                                              | Payload                          |
|-----------------|---------------------------------------------------|----------------------------------|
| `Retrieving`    | Einmal pro Builder-Query, vor dem DB-Aufruf        | None                             |
| `Retrieved`     | Einmal pro von einer Builder-Query zurückgegebener Zeile          | Modell                            |
| `Created`       | Nach erfolgreichem `create`                         | Modell                            |
| `Updated`       | Nach erfolgreichem `save` / `update`                | Snapshots davor + danach     |
| `Saved`         | Nach sowohl `create` als auch `save`                    | Modell                            |
| `Deleted`       | Nach erfolgreichem `delete`                         | Modell + `is_force: bool`         |
| `Trashed`       | Nach Soft-Delete (NICHT Force-Delete)              | Modell                            |
| `Restored`      | Nach erfolgreichem `restore`                        | Modell                            |
| `Replicating`   | Während `replicate` / `replicate_except`, vor der Rückgabe (NICHT `replicate_into` - pro Quelltyp) | Quelle + `Arc<Mutex<replica>>` (veränderbar) |
| `ForceDeleting` | Vor `force_delete` auf Soft-Delete-Modell        | Modell                            |
| `ForceDeleted`  | Nach erfolgreichem `force_delete`                   | Modell                            |

Die Aufteilung abbrechbar / nicht abbrechbar spiegelt Laravels
Hook-Paar `creating` vs `created`. `Saving` feuert sowohl für
Insert als auch Update - überschreiben Sie dieses, wenn das
Verhalten über beide Pfade identisch ist, und unterscheiden Sie
über `is_creating`.

`Replicating` ist der einzige nicht abbrechbare Hook, der eine
veränderbare Referenz übergibt (die Replica ist `Arc<Mutex<M>>`).
Verwenden Sie ihn, um Timestamps zu löschen, UUIDs neu zu
generieren, Auto-Increments zurückzusetzen usw., bevor der Klon an
den Aufrufer zurückgegeben wird.

### Observers vs. rohe Listener

Zwei Wege, sich in Lifecycle-Events einzuhängen:

1. **Rohe Listener** - rufen Sie
   `EventFacade::listen::<Created, _>(Arc::new(MyListener))` für
   jedes gewünschte Event auf, eine Implementierung pro Event. Das
   ist der zugrunde liegende Mechanismus; Observers setzen darauf
   auf.

2. **Observers** - bündeln alle 16 Hooks unter einem Trait. Das
   Makro sieht, welche Methoden der Nutzer überschrieben hat, und
   registriert genau diese. Das ist der empfohlene Weg für jedes
   nicht triviale Set von Hooks.

```rust
use async_trait::async_trait;
use suprnova::eloquent::attrs::Attrs;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::observers::Observer;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]   // <- MUSS vor #[async_trait] stehen
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

Jede Trait-Methode hat einen Default-No-Op, sodass der Impl-Block
nur die Events enthält, die Sie interessieren. Das Makro
identifiziert Overrides per Namensabgleich gegen das geschlossene
16-Methoden-Set; Methoden, die Sie nicht überschreiben, registrieren
keine Listener.

### Erforderliche Attribut-Reihenfolge

`#[suprnova::observer(M)]` MUSS ÜBER `#[async_trait]` stehen:

```rust
#[suprnova::observer(User)]   // äußeres - läuft zuerst, sieht rohe async fns
#[async_trait]                // inneres - schreibt async-fn-Signaturen um
impl Observer<User> for AuditObserver { /* ... */ }
```

Attribut-Makros expandieren von außen nach innen. `async_trait`
schreibt jede `async fn` in eine desugarte
`Pin<Box<dyn Future>>`-Poll-Fn-Form um; würde `#[async_trait]`
zuerst laufen, fände der Namensabgleich des Observer-Makros gegen
die 16 Trait-Methodennamen nichts und würde stillschweigend null
Listener erzeugen.

### Vier Registrierungspfade

| Pfad                                         | Wann verwenden                                         |
|----------------------------------------------|-----------------------------------------------------|
| `#[suprnova::observer(M)]` (Inventory)       | Statischer Observer, zur Compile-Zeit bekannt. Installiert sich automatisch beim Boot. |
| `#[model(observers = [Foo, Bar])]`           | Dokumentation + Compile-Zeit-Validierung, dass die aufgeführten Typen aufgelöst werden können. Registriert selbst NICHT. |
| `Model::observe(MyObs).await`                | Runtime-Registrierung. Handgesteuert; nützlich, wenn die Registrierung von der Konfiguration abhängt. |
| `EventFacade::listen::<events::Created, _>(...)` | Niedrigste Ebene - ein Event auf einmal. Verwenden, wenn ein Observer zu schwergewichtig wirkt. |

Das Attribut `observers = [...]` auf `#[model]` ist ein
Dokumentations-Marker. Es kompiliert zu einem Block
`const _: fn() = || { let _ =
::std::any::type_name::<T>; ... };`, der beweist, dass jeder
aufgeführte Typ zu einem echten Rust-Item aufgelöst wird; Tippfehler
tauchen an der Modell-Deklarationsstelle auf. Die tatsächliche
Installation läuft über den Inventory-Pfad - das Attribut
`#[observer(M)]` auf `Foo` ist das, was `Foo` für die
Auto-Installation einschreibt.

### Bootstrap

Rufen Sie `bootstrap_observers()` einmal beim Start auf, um das
Inventory zu entleeren und jeden mit `#[observer(M)]` registrierten
Observer zu installieren:

```rust
suprnova::eloquent::observers::bootstrap_observers().await?;
```

Das Entleeren ist idempotent für den Inventory-Pfad - die
Install-Closure jedes Observers ist durch einen Pro-Typ-`AtomicBool`
abgesichert (T2bs Makro-Emission), sodass ein zweifacher Aufruf von
`bootstrap_observers()` nicht doppelt registriert.

Der Runtime-Shim `Model::observe(MyObs)` ist NICHT abgesichert. Ihn
zweimal aufzurufen registriert zwei Listener-Sets, passend zu
Laravels manueller Semantik `Model::observe(MyObs::class)`. Wenn ein
handgesteuerter Observer auch `#[observer]` trägt, feuert der
Inventory-Adapter zusätzlich zu den manuell installierten.

### Abbrechen aus einem Observer

Die fünf abbrechbaren Hooks geben `EventResult` zurück. Um die
Operation abzubrechen, geben Sie `EventResult::cancel("reason")`
zurück:

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

Der Abbruchgrund erscheint als `FrameworkError::bad_request(reason)`
von `Subscription::create`. Die Zeile landet nie in der Datenbank -
Cancel ist ein echter Abbruch, kein "Löschen im Nachhinein".

Mehrere Observer können abbrechbare Hooks auf demselben Modell
registrieren; sobald einer davon `Cancel` zurückgibt, stoppt die
Operation. Die Reihenfolge ist die Inventory-Einschreibereihenfolge
(in der Praxis die Link-Reihenfolge).

### Mehrere Observers auf einem Modell

Mehrere `Observer<M>`-Implementierungen feuern alle für dasselbe
Event - der EventFacade-Dispatch fächert an jeden registrierten
Listener auf, statt einen auszuwählen:

```rust
#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for AuditObserver { /* ... */ }

#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for NotifyObserver { /* ... */ }

// Comment::create(...) feuert AuditObserver::created UND NotifyObserver::created.
```

Das entspricht Laravels Fan-out-Semantik und ist die tragende
Eigenschaft hinter dem Muster "Hooks nach Concern aufteilen": Ein
`AuditObserver` weiß nur von Audit, ein `NotifyObserver` nur von
Benachrichtigungen, und die Modell-Deklaration kümmert sich nicht
darum, wie viele Observer sich anhängen.

### Manuelles `Model::observe()`

Jede `#[suprnova::model]`-Struktur erhält einen Pro-Modell-Shim
`observe<O>()`. Rufen Sie ihn beim Boot für dynamische
Registrierung auf:

```rust
#[derive(Clone)]
struct MyObs;

#[async_trait]
impl Observer<User> for MyObs { /* ... */ }

// Zur Laufzeit:
User::observe(MyObs).await;
```

Die `O: Clone + 'static`-Bound des Shims ist das, was dem Framework
erlaubt, jedem der 16 internen Adapter-Listener einen frischen
Observer-Klon zu übergeben. Alle 16 Listener-Adapter installieren
sich bei jedem Aufruf - die Trait-Defaults machen nicht
überschriebene Methoden zu billigen No-Ops.

### Einschränkungen

- **Die Makro-Version verlangt, dass der Impl-Block einfache
  Methodennamen verwendet, die zu den 16 Hooks des Traits passen.**
  Umbenannte Methoden, durch `#[allow]` unterdrückte Defaults und
  `#[cfg]`-abgesicherte Bodies fallen außerhalb des Namensabgleichs
  und registrieren keine Listener.

- **Observer-Strukturen, die das Makro inspiziert, müssen in v1
  zero-sized sein** (keine Felder). Das Makro konstruiert den
  Observer über `let obs = MyObserver;` innerhalb jedes Adapters.
  Zustandsbehaftete Observer (die `Arc<Inner>` tragen) brauchen den
  Runtime-Pfad `Model::observe()`, der den Observer nach Wert
  übernimmt und ihn in jeden Adapter klont.

- **Test-Isolation: eindeutige Modell-Typen pro Szenario
  verwenden.** Der prozessglobale EventDispatcher bedeutet, dass für
  `User` installierte Listener für jeden Test in derselben Binary
  sichtbar sind. Pro-Test eindeutige Modell-Typen (`T2Comment`,
  `T2Subscription`, …) halten Cross-Test-Bleed aus den
  Counter-Assertions heraus. Die Integrationstests
  `eloquent_observers.rs` üben dieses Muster aus.

## Prunable

Laravel liefert einen Trait `Prunable`, der ein Modell einen Scope
von Zeilen deklarieren lässt, die nach einem Zeitplan gelöscht
werden. Suprnova spiegelt das mit zwei Traits und einem
Konsolenbefehl.

### Einen Pruner deklarieren

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

### `MassPrunable` - Bulk-Delete-Variante

Für Tabellen mit hohem Volumen (Audit-Logs, Request-Logs,
abgelaufene Cache-Einträge) überspringt `MassPrunable`
Pro-Zeile-Events und führt ein einziges `DELETE WHERE …`-Statement
aus:

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

### Pruning auslösen

Ausgeführt über die Pro-Projekt-Konsole (für die `app/cmd/main.rs`
`suprnova::console::dispatch_argv` aufruft, nach `db:seed` und den
anderen Built-ins):

```bash
suprnova model:prune                          # jeden registrierten Typ prunen
suprnova model:prune --model=ExpiredSession   # auf ein Modell filtern
suprnova model:prune --pretend                # Dry Run; protokolliert, was gelöscht würde
```

Programmatisch stehen die Runner unter
`suprnova::eloquent::{prune_all, prune_all_dry, prune_one}`.

### Pruning-Hook

`Prunable::pruning(&self)` feuert vor jedem Zeilen-Delete, sodass
der Nutzer Seiteneffekte ausführen kann (zugehörige Dateien
aufräumen, Events auffächern usw.). Die Standard-Implementierung
ist leer. `MassPrunable` überspringt diesen Hook per Definition -
Bulk-Deletes enumerieren keine Zeilen.

### Cascade-Verhalten

**Pruning kaskadiert NICHT automatisch auf verwandte Zeilen.** Eine
`Prunable`- oder `MassPrunable`-Implementierung auf `User` löscht
User-Zeilen; ihre `posts`, `role_user`-Pivot-Einträge, polymorphe
`comments` usw. bleiben VERWAIST zurück, mit FK-Spalten, die auf
den nun gelöschten User zeigen.

Das entspricht Laravels Vertrag: Relations-Aufräumen ist die
Aufgabe des Nutzers. Zwei saubere Wege, damit umzugehen:

1. **FK-Cascade auf Datenbankebene** - deklarieren Sie `ON DELETE
   CASCADE` (oder `ON DELETE SET NULL`) im Foreign-Key-Constraint,
   wenn Sie die Migration schreiben. Die DB-Engine übernimmt die
   Cascade kostenlos, ohne Pro-Zeile-Rust-Code.

2. **Pro-Zeile-Hook** - implementieren Sie
   `Prunable::pruning(&self)`, um Kinder zu löschen, bevor die
   Elternzeile fällt. Der Hook feuert innerhalb derselben logischen
   Operation wie das Eltern-Delete, sodass eine konsistente
   Reihenfolge garantiert ist:

   ```rust
   #[async_trait]
   impl Prunable for User {
       fn prunable() -> Builder<Self> {
           Self::query().filter_op("deleted_at", "<", thirty_days_ago())
       }

       async fn pruning(&self) -> Result<(), FrameworkError> {
           // Posts löschen.
           Post::query().filter("user_id", self.id).get().await?
               .into_iter()
               .map(|p| p.delete());
           // Role-Pivots trennen.
           self.roles().sync(Vec::<i64>::new()).await?;
           Ok(())
       }
   }
   ```

`MassPrunable` ist mengenbasiert - `pruning()` feuert nicht.
Verwenden Sie schlichtes `Prunable`, wann immer Sie eine Cascade
brauchen. Das Framework wird kein Pro-Zeile-DELETE stillschweigend
ausgeben, wenn Sie sich für `MassPrunable` entscheiden; der
Trade-off ist sichtbar dokumentiert.

### Registry-Mechanismus

Die Pruner-Registrierung verwendet dasselbe Inventory-Muster wie
Observers, Commands und Supervisors. Das Attribut
`#[suprnova::prunable]` auf dem Block `impl Prunable for T { ... }`
registriert sich zur Compile-Zeit automatisch über
`inventory::submit!`. Keine zentrale Config-Datei; einen neuen
prunable Typ hinzuzufügen ist ein einziges Attribut.

## Multi-Connection-Routing

Produktions-Apps brauchen regelmäßig mehr als eine
Datenbank-Connection - der klassische Fall ist eine Read-Replica für
Analytics + die primäre für Writes, aber die Oberfläche
verallgemeinert auf jede benannte Connection (Reporting-DB,
Archiv-DB, Pro-Tenant-Shard).

### Eine Connection registrieren

Rufen Sie `DB::register_named(name, config)` beim Boot für jede
Nicht-Standard-Connection auf, mit der Ihre App spricht:

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

Zwei Namen sind reserviert: `__primary__` schneidet die Registry
kurz zu `DB::connection()`, und `__read_replica__` schaltet die
Connection in automatisches Read-Write-Split-Routing ein - siehe
unten.

### Pro-Query-Opt-in: `Model::on(name)`

`Model::on("reporting")` gibt einen `Builder<M>` zurück, der vorab
so eingestellt ist, dass er über die benannte Connection routet:

```rust
let totals = Order::on("reporting")
    .order_by_desc("total")
    .limit(100)
    .get()
    .await?;
```

`on(...)` ist request-gescoped - es betrifft nur den verketteten
Builder. Der nächste einfache `Order::query()`-Aufruf löst über den
Standard auf.

### Pro-Modell-Standard: `#[model(connection = "...")]`

Wenn ein Modell immer auf einer Connection lebt, deklarieren Sie den
Standard auf dem Attribut:

```rust
#[model(table = "events", connection = "events_db")]
pub struct Event { /* ... */ }
```

Jeder Aufruf von `Event::query()` / `Event::create()` /
`Event::find()` routet über `events_db`, ohne den
Pro-Query-Override `.on(...)` zu brauchen. Ein explizites `.on(...)`
auf einem Builder gewinnt weiterhin.

### Read-Write-Split

Eine Connection unter dem reservierten Namen `__read_replica__` zu
registrieren schaltet jedes Modell in automatisches Routing ein:
Read-Methoden (`first` / `get` / `find` / `count` / `paginate` /
`chunk` / die closure-gesteuerten Walker) fließen über die Replica;
Writes (`save` / `create` / `update` / `delete` / `force_delete` /
`replicate` / `attach` / `detach` / `sync` / `increment` /
`decrement`) fließen über die primäre.

`Model::on_write_connection()` schaltet einen einzelnen Builder AUS
der Replica heraus - nützlich, wenn Read-your-writes-Konsistenz
wichtig ist (z. B. direkt nach einem `save`, bevor die Replikation
aufgeholt hat).

### Routing-Vorrang

Die Dispatch-Kette führt jede Operation durch
`ExecutorChoice::resolve_read` oder `resolve_write`. Die
Reihenfolge ist:

1. **Eine aktive Transaktion gewinnt absolut.** Innerhalb von
   `DB::transaction` verwenden jeder Read UND jeder Write die
   tx-Connection. `on(name)` wird innerhalb einer Transaktion
   IGNORIERT - die tx ist an eine bestimmte physische Connection
   gebunden. SeaORM kann keine Transaktion auf einer Connection
   beginnen und Statements gegen eine andere ausführen.
2. **Pro-Builder `on(name)`.** Gesetzt über `Model::on(name)` /
   `Builder::on(name)`. Gewinnt über den Modell-Standard und den
   Read/Write-Split.
3. **`Model::on_write_connection()`.** Erzwingt die primäre, selbst
   wenn die Operation sonst zur Replica routen würde.
4. **Pro-Modell-Standard `#[model(connection = "...")]`.** Gewinnt
   über den Read/Write-Split für die eigenen Queries des Modells.
5. **Read/Write-Split.** Wenn `__read_replica__` registriert ist,
   routen Read-Methoden dorthin; Writes routen zur primären.
6. **Standard.** `DB::connection()` - die primäre, die von
   `DB::init()` aufgesetzt wurde.

### Vorbehalte

- Aktive Transaktionen IGNORIEREN `on(name)` (siehe §1 oben). Wenn
  Sie mitten in einer tx einen Write auf einer anderen Connection
  brauchen, können Sie das nicht - die tx ist an eine Connection
  gebunden.
- Die reservierten Namen `__primary__` und `__read_replica__`
  können nicht als Nutzer-Connection-Namen verwendet werden.
  `DB::register_named` gibt bei einer Kollision einen Fehler
  zurück.
- Replica-Lag ist IHR Problem. Suprnova wiederholt Reads nicht
  automatisch und fällt nicht auf die primäre zurück, wenn die
  Replica veraltet ist; wenn Sie nach einem Save
  Read-your-writes brauchen, verwenden Sie explizit
  `Model::on_write_connection()`.

## Replikation

`Model::replicate()` gibt eine nicht gespeicherte Kopie des Modells
zurück, mit dem Primärschlüssel auf seinen Standard zurückgesetzt.
Nützlich für "diesen Datensatz duplizieren"-UX, bei der der Nutzer
von einer bestehenden Zeile ausgehen möchte.

```rust
let template: User = User::find_or_fail(42).await?;
let mut copy = template.replicate().await?;  // id auf Standard zurückgesetzt
copy.email = "fresh@example.com".into();
copy.save().await?;  // INSERT, nicht UPDATE
```

`replicate` ist in Suprnova **async** (weicht von Laravel ab), weil
es das Event `Replicating` auslöst - `Saving`- / `Created`-Listener
usw. können die Replica verändern, bevor sie zurückgegeben wird.
Siehe [Replicating-Event](#replicating-event) für den
Listener-Mutationsvertrag.

### `replicate_except`

Benannte Felder aus der Replica entfernen:

```rust
let copy = order.replicate_except(["payment_token", "stripe_id"]).await?;
```

Aufgeführte Felder fallen auf die `Default`-Implementierung des
Modells zurück - `String`s werden zu `""`, `Option`s werden zu
`None` usw. Verwenden Sie das für sensible Spalten, die die
replizierte Zeile nicht übernehmen soll.

### Typübergreifendes `replicate_into::<T>`

Die Suprnova-Abweichung - Laravel kann das nicht, weil PHP keine
Typen hat. `replicate_into::<T>()` überbrückt zu einem Schwestertyp
über `serde_json`:

```rust
let order: Order = Order::find_or_fail(42).await?;
let invoice: Invoice = order.replicate_into::<Invoice>().await?;
invoice.save().await?;
```

Felder mit übereinstimmenden Namen + serde-kompatiblen Typen werden
übernommen; Felder, die auf einer der beiden Seiten nicht passen,
fallen still weg. `T` muss `Default` implementieren, damit
unbefüllte Felder einen Wert haben. Typübergreifende Replikation
löst KEIN `Replicating` aus (das Event trägt ein `&mut Self` - es
gibt keinen Weg, `T` darüber zu adressieren). Wenn Sie
event-getriebene Mutation brauchen, replizieren Sie zuerst
gleichtypig und materialisieren Sie `T` dann aus dem Ergebnis.

## Debugging - dump und dd

Zwei interaktive Debugging-Hilfen auf jedem `Builder<M>`:

```rust
// Protokolliert SQL + Bindings über tracing::info!, gibt self zurück.
let users = User::query()
    .filter("active", true)
    .dump()                       // → Log-Zeile, Builder läuft weiter
    .order_by_desc("created_at")
    .get()
    .await?;

// Protokolliert über tracing::error!, paniert dann mit dem SQL in der Nachricht.
User::query().filter("id", 1).dd();  // - !
```

`dump` ist verkettbar; `dd` gibt `!` zurück (kehrt nie zurück - der
Panic ist der Vertrag). Beide spiegeln Laravels `Builder::dump()` /
`Builder::dd()` exakt.

Beide Helfer fallen auf den SQLite-Dialekt zurück, wenn keine
lebende DB-Connection gebunden ist (entspricht dem Fallback von
`to_sql_with_bindings`), sodass sie am REPL oder in einem Test ohne
`TestDatabase` weiterhin nützlich sind.

Die Panic-Nachricht verwendet das literale Präfix `eloquent dd:`,
sodass Tests dagegen assertieren können:

```rust
#[test]
#[should_panic(expected = "eloquent dd")]
fn dd_panics_with_sql_in_message() {
    User::query().filter("id", 1).dd();
}
```

**Committen Sie `dd()` niemals in einen Produktions-Codepfad.** Es
ist eine interaktive Debugging-Hilfe; der Panic beim Verlassen ist
der ganze Sinn. `dump()` ist sicherer (protokolliert nur), aber es
in Hot-Paths zu spammen füllt Ihre Logs - entfernen Sie es vor dem
Push.

Wenn Sie das SQL ohne die Seiteneffekte wollen, greifen Sie zu den
nicht protokollierenden Helfern:

- `Builder::to_sql()` - gibt das gerenderte SQL als `String` zurück.
- `Builder::to_sql_with_bindings()` - gibt `(String, Vec<SeaValue>)`
  zurück.
- `Builder::to_sql_for(backend)` - rendert für einen expliziten
  Dialekt (Backend-übergreifendes Debugging).

## Modelle testen

Tests instanziieren eine echte Datenbank über `TestDatabase`, die
die Connection im Pro-Test-Container registriert, sodass alles, was
innerhalb des SUT `DB::connection()` aufruft, zur Test-DB auflöst.

### Zwei Einstiegspunkte

- **`TestDatabase::fresh::<MyMigrator>().await`** - führt jede
  Migration aus, die der Produktions-Migrator ausführt. Verwenden
  Sie das für App-Level-Dogfood-Tests, bei denen das Test-Schema
  exakt dem entsprechen soll, was `suprnova migrate` erzeugt.
- **`TestDatabase::sqlite_memory().await`** - öffnet eine
  In-Memory-SQLite-Datenbank OHNE Migrationen anzuwenden. Verwenden
  Sie das für Framework-Level-Unit-Tests, bei denen Sie präzise
  Kontrolle über die Spaltenform über ein Pro-Test
  `db.execute_unprepared("CREATE TABLE …")` wollen.

### App-Level-Dogfood-Muster

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

Die Bindung `_db` hält die `TestDatabase` für den gesamten Test -
sie zu droppen reißt den Container ein und gibt die
In-Memory-SQLite-Connection frei. Schatten Sie sie nicht auf `_`,
sonst verschwindet die Connection, bevor der SUT läuft.

### Framework-Level-Form-Muster

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

### Kernmuster

- `TestDatabase::fresh::<MyMigrator>()` für App-Level-Tests mit dem
  Produktions-Schema. `TestDatabase::sqlite_memory()` für
  Unit-Level-Form-Tests.
- Verwenden Sie `TestContainer::bind` (NICHT `App::bind`) für jedes
  Singleton, das der Test verändert - globale Registry-Overrides
  laufen bei parallelen Läufen sonst in ein Race. Der
  `TestDatabase`-Konstruktor übernimmt das DB-Binding für Sie.
- Halten Sie Modell-Deklarationen auf Modul-Scope, nicht innerhalb
  von Test-Fns. Das Makro erzeugt ein inneres `mod`, dessen
  `use super::*;` nur die Top-Level-Imports der Datei sieht - ein
  Modell innerhalb einer Testfunktion zu deklarieren zerbricht die
  SeaORM-Typauflösung.

## Der Notausgang zu SeaORM

Drei Notausgänge halten SeaORM von innerhalb der Eloquent-Schicht
aus erreichbar:

1. **Das innere Modul** - `user::Entity`, `user::Column`,
   `user::ActiveModel`, `user::Model`. Das Makro erzeugt diese für
   jedes Modell; es sind SeaORM-Typen, die Sie direkt verwenden
   können. Siehe [Layout des Modellmoduls](#layout-des-modellmoduls)
   für das vollständige Layout und wann Sie hineingreifen sollten.
2. **`From`-Konvertierungen** - `From<user::Model> for User` und
   `From<User> for user::Model` überbrücken zwischen Zeilen in
   SeaORM-Form (storage-typisierte Spalten) und Zeilen in
   Eloquent-Form (runtime-typisierte Spalten). Nützlich, wenn Sie
   eine SeaORM-Query ausgeben und das Ergebnis in die Eloquent-Form
   konvertieren wollen, oder umgekehrt.
3. **Die Suprnova-aliasierten SeaORM-Typen** - jeder SeaORM-Typ, den
   ein Konsument berühren würde, wird unter `suprnova::*`
   re-exportiert. Sie sollten `use sea_orm::*` im App-Code nicht
   brauchen.

```rust
use suprnova::sea_orm::{ColumnTrait, EntityTrait};

// Mitten in der Query zu SeaORM wechseln - Eloquent hat dafür
// keine Methode, SeaORM aber schon:
let db = suprnova::DB::connection()?;
let users = user::Entity::find()
    .filter(user::Column::Email.like("%@example.com"))
    .all(db.inner())
    .await?;

// In die Eloquent-Form konvertieren:
let eloquent: Vec<User> = users.into_iter().map(User::from).collect();
```

Drei Notausgänge und die From-Brücke bedeuten, dass die
Eloquent-Schicht Sie niemals davon abhält, das darunterliegende ORM
zu erreichen.

## Migration von `database::Model`

Älterer Code kann `impl suprnova::database::Model for Entity {}` auf
einer handgerollten SeaORM-Entity tragen. Der Trait wurde in
`EntityExt` umbenannt, um Platz für den neuen Trait `Model` zu
schaffen - der auf der nutzerseitigen Struktur sitzt, nicht auf der
SeaORM-Entity.

Der empfohlene Migrationsweg ist, den Typ auf `#[suprnova::model]`
umzustellen, was Ihnen die vollständige Eloquent-Oberfläche plus die
umbenannten `EntityExt`-Traits als Bonus gibt. Für den seltenen
Fall, dass Sie die alte SeaORM-Entity-Extension-Form beibehalten
wollen, sind die Trait-Namen `EntityExt` / `EntityExtMut` weiterhin
unter `suprnova::database::*` verfügbar. Sie verhalten sich exakt
wie das alte `database::Model`.

## DB-Facade - modell-lose Queries

Manche Tabellen gehören nicht auf eine
`#[suprnova::model]`-Struktur: kurzlebige Audit-Logs, Ad-hoc-
Reporting-Joins, Dashboard-Aggregate. Dafür greifen Sie zur
`DB`-Facade. Zwei Oberflächen liegen darunter:

### `DB::table(name)` - verkettbarer Query Builder

`DbTableBuilder` spiegelt die where- / order- / limit-Form von
`Builder<M>`, liefert Zeilen aber als `DynamicRow` (ein Newtype mit
typisierten Zugriffsmethoden über `serde_json::Map<String, Value>`):

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

Die vollständige Oberfläche:

| Methode | Rückgabe | Zweck |
|--------|---------|---------|
| `.select(["id", "event"])` | `DbTableBuilder` | Spalten einschränken (Standard `*`) |
| `.filter(col, val)` | `DbTableBuilder` | `WHERE col = ?` |
| `.filter_op(col, op, val)` | `DbTableBuilder` | `WHERE col <op> ?` |
| `.order_by_asc(col) / _desc(col)` | `DbTableBuilder` | Sortierung |
| `.limit(n) / .offset(n)` | `DbTableBuilder` | Fenster |
| `.get()` | `Collection<DynamicRow>` | Alle passenden Zeilen |
| `.first()` | `Option<DynamicRow>` | Erste Zeile oder `None` |
| `.count()` | `u64` | `SELECT COUNT(*) ...` |
| `.insert(attrs)` | `i64` | `id` der neuen Zeile |
| `.update(attrs)` | `u64` | Betroffene Zeilen |
| `.delete()` | `u64` | Betroffene Zeilen |

**Vertrauensgrenze für Identifier.** Tabellennamen, Spaltennamen,
SQL-Operatoren und ORDER-BY-Richtungen werden wortwörtlich in den
SQL-String interpoliert - sie werden NICHT als Parameter gebunden.
Übergeben Sie an diese Argumente nur vertrauenswürdige
Compile-Zeit-Literale. Werte (die rechte Seite von `filter` /
`filter_op`) WERDEN gebunden und können sicher aus Request-Daten
durchgereicht werden.

**Ein leeres WHERE bei `update` / `delete` wirkt auf jede Zeile.**
`DB::table("audit_log").delete().await?` leert die Tabelle
absichtlich - fügen Sie ein `filter` hinzu, wenn Sie das nicht
meinen.

**Insert-Backend-Split.** `RETURNING id` wird auf Postgres und
SQLite verwendet; MySQL führt das INSERT aus und gibt dann
`SELECT LAST_INSERT_ID() as id` aus, um den Auto-Increment
wiederzugewinnen.

### `DynamicRow` - typisierte Zugriffsmethoden über eine JSON-Map

`DynamicRow` umschließt eine `serde_json::Map<String, Value>` und
stellt typisierte Getter bereit. Jeder liefert
`Result<T, FrameworkError>` mit einer klaren Fehlermeldung bei
fehlendem Key oder Typ-Mismatch:

```rust
let event: String     = row.get_string("event")?;
let actor_id: i64     = row.get_int("actor_id")?;
let active: bool      = row.get_bool("active")?;
let prefs: Prefs      = row.get_as("prefs")?;  // jedes DeserializeOwned
let raw: serde_json::Value = row.get_value("meta")?;
```

Nullbare Spalten: verwenden Sie `get_optional_*`. Diese
unterscheiden "Spalte fehlt" (Fehler - Schema-Mismatch) von "Spalte
vorhanden, Wert null" (`Ok(None)`):

```rust
let score: Option<i64>      = row.get_optional_int("score")?;
let title: Option<String>   = row.get_optional_string("title")?;
```

`DynamicRow` dereferenziert zu `Map<String, Value>`, sodass
Iteration und Key-Existenz-Prüfungen natürlich funktionieren:

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}
```

### Notausgänge zu rohem SQL

Wenn der Builder nicht ausreicht - Window-Funktionen, rekursive
CTEs, backend-spezifisches DDL - fallen Sie auf einen rohen String
zurück. Platzhalter passen zum aktiven Backend (`$1, $2, ...` für
Postgres, `?` für MySQL + SQLite):

```rust
// Rohes SELECT, materialisiert als DynamicRow.
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) as post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING post_count > ?",
    vec![5i64.into()],
).await?;

// Rohes UPDATE / DELETE - liefert betroffene Zeilen.
let updated = DB::update(
    "UPDATE users SET verified_at = NOW() WHERE id = ANY($1)",
    vec![ids.into()],
).await?;

let deleted = DB::delete(
    "DELETE FROM stale_sessions WHERE expires_at < ?",
    vec![now.into()],
).await?;

// Rohes DDL oder Statements ohne Bindings.
DB::statement("CREATE INDEX CONCURRENTLY idx_users_email ON users(email)")
    .await?;

// Generisches betroffene-Zeilen-Statement - für INSERT ... ON CONFLICT usw.
let rows = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1) ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec!["page_views".into()],
).await?;
```

Verwenden Sie diese Notausgänge sparsam - der typisierte Builder
fängt mehr Fehler zur Compile-Zeit und liest sich sauberer in der
Business-Logik. Aber wenn Sie sie brauchen, sind sie da.

**Falle bei Aggregat-Spalten.** Untypisierte Aggregate wie
`SELECT COUNT(*) AS n FROM t` funktionieren über den
`.count()`-Helfer des Builders, werden bei rohen
`DB::select`-Zeilen auf SQLite aber unter Umständen stillschweigend
fallen gelassen - das zugrunde liegende
`JsonValue::from_query_result` läuft sqlxs Typinformation pro
Spalte ab, und ein bloßes Aggregat trägt keine. Brauchen Sie den
rohen Select-Pfad mit Aggregaten, geben Sie dem Ausdruck einen
typisierten Kontext: verwenden Sie entweder einen
`CAST(... AS BIGINT)`-Wrapper, oder lesen Sie die Spalte mit einem
typisierten `DB::table(...).count()` / `.max(...)`-Helfer, der
unter der Haube `query_one` + `try_get` verwendet.

## Relations-Existenz + günstige Kurzformen

Suprnova spiegelt Laravels Query-Familie zur Relations-Existenz. Jede
Methode hier paart den Laravel-förmigen Namen mit einem idiomatischen
Rust-Alias (Suprnovas stehende Dual-API-Konvention).

### Filter auf Relations-Existenz (`has` / `where_has` / `where_belongs_to`)

Die Familie mit korreliertem `EXISTS (...)` schränkt die Eltern-Query
über das Vorhandensein (oder Fehlen oder die Anzahl) verwandter Zeilen
ein, ohne die Relation in das äußere SELECT zu joinen.

```rust
use suprnova::Model;

// Nutzer, die mindestens einen Beitrag haben.
let users = User::query().has("posts").get().await?;

// Nutzer, die KEINE Beiträge haben.
let empty = User::query().doesnt_have("posts").get().await?;

// Nutzer mit >= 3 Beiträgen (Laravels `has("posts", ">=", 3)`).
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// Innere Bedingung über eine Closure - schränkt den Rumpf der
// EXISTS-Subquery ein.
let recent = User::query()
    .where_has::<Post, _>("posts", |q| q.filter_op("created_at", ">=", "2026-01-01"))
    .get()
    .await?;

// Kurzform für eine Spalte - entspricht `where_has` mit einer winzigen
// Closure.
let with_pub = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// Direkter Belongs-to-Join (kein EXISTS - die FK liegt auf dieser
// Tabelle).
let posts = Post::query().where_belongs_to("author", author.id).get().await?;
```

Alle Varianten lassen sich mit den `or_*`- und
`*_doesnt_have`-Begleitern komponieren:

- `has` / `or_has` / `has_count` / `doesnt_have` / `or_doesnt_have`
- `where_has` / `or_where_has` / `where_doesnt_have` / `or_where_doesnt_have`
- `where_relation` / `where_relation_op` / `or_where_relation`
- `where_belongs_to`

Die Engine liest die Relations-Metadaten aus dem makrogenerierten
`RelationEntry`-Inventory: Join-Spalten, Pivot-Tabellen und
Morph-Diskriminatoren fließen alle automatisch mit. Drei
Subquery-Formen werden gerendert:

- **Has** - `EXISTS (SELECT 1 FROM child WHERE child.fk = parent.pk)`
- **Pivot** - `EXISTS (SELECT 1 FROM pivot INNER JOIN target ON ... WHERE pivot.parent_fk = parent.pk)`
- **Morph** - Has- oder Pivot-Form plus `AND target.<morph>_type = '<value>'`

Unbekannte Relationsnamen rendern die sicher scheiternde Form
(`EXISTS (SELECT 1 WHERE 1 = 0)`), die zu `FALSE` auswertet und null
Zeilen zurückgibt. Ein Tippfehler lässt nie einen vollständigen
Tabellenscan durch.

### `MorphTo`-Abweichung

Laravels `MorphTo`-Umkehrung (`whereMorphedTo`, `whereHasMorph`) läuft
mehrere Zieltabellen ab, weil das Morph-Kind einen `*_type`-Diskriminator
trägt, der eines von N möglichen Elternteilen auswählt. Suprnovas
`MorphTo` wird bei der Makroexpansion zu einem Enum pro Familie
heruntergebrochen - der Zieltyp ist statisch ein
`<Family>Morph { Variant1(...), ... }` und keine einzelne SQL-Tabelle.
Die Existenz-Engine kann dafür kein festes
`EXISTS (SELECT 1 FROM <table>)` rendern, weil es keine einzelne Tabelle
gibt.

Empfohlene Migration: Führen Sie die Existenzprüfung stattdessen auf
Ebene des Morph-Kindes durch. Wo Laravel schreibt:

```php
Comment::whereHasMorph('commentable', [Post::class], fn ($q) => $q->where('published', true))
```

schreibt Suprnova:

```rust
Comment::query()
    .filter("commentable_type", "post")
    .where_has::<Post, _>("commentable_post", |q| q.filter("published", true))
    .get()
    .await?;
```

Die enger typisierte Form gibt Ihnen im inneren Builder die volle
IDE-Vervollständigung, die das lose typisierte `whereHasMorph` nicht
bieten kann.

### Günstige Builder-Kurzformen

```rust
// PK-Filter.
User::query().where_key(7).first().await?;        // Zucker für filter("id", 7)
User::query().where_key_not(7).get().await?;      // Zucker für filter_op("id", "!=", 7)
User::query().filter("name", n).or_where_key(7).get().await?;      // ... OR id = 7
User::query().filter("name", n).or_where_key_not(7).get().await?;  // ... OR id != 7
// Rust-idiomatische Aliase: filter_key / filter_key_not /
// or_filter_key / or_filter_key_not.

// Nach created_at sortieren.
Post::query().latest().get().await?;              // ORDER BY created_at DESC
Post::query().oldest().get().await?;              // ORDER BY created_at ASC
Post::query().latest_by("published_at").get().await?;  // benannte Spalte

// Treffer auf genau eine Zeile.
let one = User::query().filter("email", e).sole().await?;          // Fehler bei 0 oder >1
let val: i64 = User::query().filter("id", 1).sole_value("views").await?;
let v: i64 = User::query().filter("name", "x").value_or_fail("views").await?;

// Opt-outs beim Eager Loading.
User::query().with(["posts","tags"]).without(["tags"]).get().await?;
User::query().with_only(["posts"]).get().await?;   // löscht zuerst den Plan

// Vollqualifizierte Spalten (für Joins).
Builder::<User>::qualify_column("name");           // -> "users.name"
Builder::<User>::qualify_columns(["name", "id"]);  // -> ["users.name", "users.id"]
```

### Massenmutation - `update_all` / `delete_all` / `upsert` / `*_each`

Diese treffen die Datenbank direkt mit einem einzigen Statement und lösen
KEINE Modell-Events pro Zeile aus. Nutzen Sie sie, wenn das Eingrenzen
über den Scope genügt und Sie keine Lifecycle-Hooks brauchen; für Hooks
pro Zeile iterieren Sie mit `.get()` und rufen pro Zeile `.update()` /
`.delete()` auf. `delete_all` zielt immer auf das statische `M::TABLE`
des Modells; Tabellennamen zur Laufzeit werden nicht als ausführbares SQL
angenommen. Ausdrückliche Null-Attribute werden als SQL-`NULL`
ausgegeben, sodass nullable Spalten vom Typ bigint, integer, boolean,
timestamp und anderen Nicht-Text-Typen unter PostgreSQL ihren
Datenbanktyp behalten. Jedes Attribut ungleich null bleibt
parametergebunden. Upsert-Zeilen müssen denselben Spaltensatz haben; ein
fehlender oder zusätzlicher Schlüssel wird abgelehnt, statt als Null
gedeutet zu werden.

```rust
// Massen-UPDATE.
let n = User::query()
    .filter("active", false)
    .update_all(attrs! { archived_at: Utc::now() })
    .await?;

// Massen-DELETE.
let n = Session::query()
    .filter_op("expires_at", "<", cutoff)
    .delete_all()
    .await?;

// INSERT ... ON CONFLICT (Postgres / SQLite) / ON DUPLICATE KEY UPDATE (MySQL).
let n = Counter::query()
    .upsert(
        vec![attrs! { key: "page_views", n: 1 }, attrs! { key: "signups", n: 1 }],
        vec!["key"],                  // Konfliktziel
        Some(vec!["n"]),              // Update-Spalten; None = jede nicht eindeutige Spalte
    )
    .await?;

// Atomares Erhöhen/Verringern gegen einen Scope.
User::query()
    .filter("id", 7)
    .increment_each(vec![("views", 1), ("likes", 1)])
    .await?;

User::query()
    .filter("id", 7)
    .decrement_each(vec![("balance", 100)])
    .await?;
```

### Statische `Model`-Helfer

```rust
// Massenlöschung über eine PK-Menge. Events pro Zeile feuern (jede Zeile
// läuft durch .delete(), sodass die Grabstein-Semantik der Soft Deletes
// und der Dispatch von Deleting/Deleted beachtet werden).
let removed: u64 = User::destroy(vec![1i64, 2, 3]).await?;
let removed: u64 = User::force_destroy(vec![1i64, 2, 3]).await?;

// Identitätsvergleich über den Primärschlüssel.
assert!(alice.is(&also_alice));
assert!(alice.is_not(&bob));
```

### `*Quietly`-Varianten - Lifecycle-Events unterdrücken

Zucker über `seed::without_events`. Die fünf statischen Lifecycle-Events
(`Saving`/`Creating`/`Updating`/`Deleting`/`Restoring`) und die nicht
abbrechbaren After-Events werden innerhalb des Scopes beide
kurzgeschlossen.

```rust
user.save_quietly().await?;            // kein Saving / Updated / Saved
user.update_quietly(attrs).await?;
user.delete_quietly().await?;
user.force_delete_quietly().await?;
```

### `*_or_fail`-Varianten

Ausdrücklicher Fehler im Nicht-gefunden-Fall. Nützlich in Codepfaden,
die Invarianten prüfen und in denen eine fehlende Zeile ein Bug ist.

```rust
let user = user.update_or_fail(attrs).await?;   // not_found, wenn die Zeile zwischenzeitlich gelöscht wurde
user.delete_or_fail().await?;
```

### Gefilterte Serialisierung - `to_array_except` / `to_array_only`

Suprnovas rust-nativer Ersatz für Laravels `makeHidden` / `makeVisible`
pro Instanz. Die Eloquent-Struktur trägt keine Attribut-Bag zur Laufzeit,
die Spaltenliste wird daher an der Aufrufstelle mitgegeben:

```rust
return Json::ok(user.to_array_except(&["password_hash", "remember_token"]));
return Json::ok(user.to_array_only(&["id", "name", "email"]));
```

**Hinweis zur Abweichung.** Laravels `makeHidden` pro Instanz verändert
Zustand, der sich fortpflanzt, wenn das Modell im `toArray()`-Aufruf
eines Elternteils verschachtelt ist. Suprnovas Filter ist eine
Abschlussmethode - er erzeugt einen `serde_json::Value` und beeinflusst
künftige Serialisierungen von `self` nicht. Für deklarative und dauerhafte
Sichtbarkeitssteuerung nutzen Sie die Attribute
`#[model(hidden = [...])]` / `#[model(visible = [...])]`.

### UUID- / ULID-Primärschlüssel - `#[model(unique_id = "...")]`

Suprnovas Entsprechung zu Laravels Trait-Familie `HasUuids` /
`HasUlids` / `HasVersion4Uuids`. Setzen Sie das Attribut, typisieren Sie
den Primärschlüssel als `String`, und das Makro füllt die ID vor dem
INSERT automatisch.

```rust
#[model(
    table = "users",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "uuid",      // oder "uuid_v4", "ulid"
)]
pub struct User {
    pub id: String,
    pub email: String,
}

// Automatisch gefüllt:
let u = User::create(attrs! { email: "a@b.com" }).await?;
// u.id ist eine frische UUID v7.

// Vom Aufrufer angegebene IDs gewinnen weiterhin (entspricht dem
// Verhalten von Laravels HasUuids).
let u = User::create(attrs! { id: "...", email: "..." }).await?;
```

Unterstützte Strategien:

- `"uuid"` / `"uuid_v7"` - UUID v7 (nach Zeitstempel geordnet, empfohlen;
  entspricht dem Standard `Str::uuid7()` ab Laravel 11)
- `"uuid_v4"` - zufällige UUID (entspricht `HasVersion4Uuids`)
- `"ulid"` - kleingeschriebene ULID mit 26 Zeichen in Crockford-Base32

Das Makro gibt einen Block `impl HasUniqueId for YourStruct` aus, der
`UNIQUE_ID_KIND` und einen Hook `new_unique_id()` bereitstellt, den Sie
auf dem Typ für einen eigenen Generator überschreiben können (etwa für
IDs mit Präfix wie `usr_<uuid>`).

### `find_or` / `find_or_new` / `create_or_first`

Runden die Trait-Oberfläche `FirstOrCreate` ab.

```rust
// Über den Primärschlüssel nachschlagen; bei Nichtfund den Fallback
// ausführen.
let user = User::find_or(id, || async {
    User::create(attrs! { id, name: "guest" }).await
}).await?;

// Über den Primärschlüssel nachschlagen; bei Nichtfund aus Standardwerten
// eine ungespeicherte Instanz bauen.
let user = User::find_or_new(id, attrs! { name: "draft" }).await?;
// user.id ist hier 0 - die Instanz existiert nur im Speicher.

// Race-sicheres Einfügen: create versuchen, bei Konflikt auf ein Holen
// zurückfallen.
let user = User::create_or_first(
    attrs! { email: "race@x.com" },
    attrs! { name: "race winner" },
).await?;
```

### Scope `without_touching`

Die Suprnova-Entsprechung zu Laravels `Model::withoutTouching`.
Innerhalb des Scopes wird jeder Aufruf von `model.touch().await`
kurzgeschlossen - nützlich bei Datenmigrationen oder Batch-Jobs, die
Zeitstempel über andere Wege verändern.

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    // .touch()-Aufrufe sind hier wirkungslos.
    for post in posts {
        post.touch().await?;
    }
}).await;
```

Der Scope liegt auf `tokio::task_local`, nebenläufige Anfragen auf
anderen Tasks beachten also weiterhin ihren eigenen Scope (oder dessen
Fehlen). `without_touching` unterdrückt außerdem die
[Berührungskaskade zum Elternteil](#übergeordnete-modelle-berühren) - ein
Kind, das innerhalb des Scopes gespeichert wird, lässt jeden Besitzer in
Ruhe, den seine `touches`-Liste benennt.

`without_touching_on::<Post, _, _>(fut)` ist die Form pro Typ -
Laravels `Model::withoutTouchingOn([Post::class], $cb)`. Darin werden
`post.touch()` und jede Kaskade, die einen `Post` anstoßen würde, still,
während Besitzer jedes anderen Typs weiterhin angestoßen werden:

```rust
use suprnova::eloquent::without_touching_on;

without_touching_on::<Post, _, _>(async {
    // Comment-Speicherungen hier drin lassen ihre Post-Besitzer in Ruhe;
    // ein Video-Besitzer am selben Kommentar wird trotzdem angestoßen.
    comment.save().await
}).await?;
```

Scopes lassen sich verschachteln, und beide liegen auf
`tokio::task_local`.

## Nächste Schritte

- [Eloquent Relationships](eloquent-relationships.md) - Deep Dive zu
  jeder Relations-Art, der Morph-Registry und der Lowering des
  polymorphen Enums
- [Eloquent Collections](eloquent-collections.md) - vollständige
  `Collection<T>`-Oberfläche, die Trennung generisch vs. modell, und
  `LazyCollection<M>`-Streaming
- [Eloquent Casts, Accessors & Mutators](eloquent-mutators.md) - die
  22 eingebauten Casts plus der Runtime-Override `casts!`
- [Eloquent Serialization](eloquent-serialization.md) - `to_array`,
  `to_json`, hidden / visible / appends, gefilterte Terminals
- [Eloquent Factories](eloquent-factories.md) - zufällige
  Modell-Instanzen für Tests und Seeder

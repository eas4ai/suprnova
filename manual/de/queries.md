# Query Builder

Wenn Sie eine Tabelle abfragen wollen, ohne sie als typisierte
`#[suprnova::model]`-Struktur zu modellieren, greifen Sie zu
`DB::table(name)`. Er liefert einen verkettbaren Builder in der Form
des typisierten Eloquent-`Builder<M>`, materialisiert Zeilen aber als
`DynamicRow` - einen `serde_json::Map`-Newtype mit typisierten
Zugriffsmethoden. Dies ist das Kapitel für Audit-Logs, Ad-hoc-Reports,
Dashboard-Aggregate und jede Tabelle, die Sie erst gar nicht
modelliert haben. Für das typisierte Äquivalent siehe
[Eloquent](eloquent.md). Für rohes `DB::select` innerhalb von
Transaktionen oder mit `DB::listen`-Observation siehe
[Datenbank](database.md).

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

## Welche Oberfläche wann

Drei Query-Oberflächen überlappen sich; wählen Sie die richtige für
die Tabelle.

| Tabelle ist … | Verwenden Sie | Liefert |
|---|---|---|
| Modelliert mit `#[suprnova::model]` | `Model::query()` → `Builder<M>` | typisierte `M`-Werte |
| Nicht modelliert, aber Sie wollen eine verkettbare WHERE/ORDER/LIMIT-Form | `DB::table(name)` → `DbTableBuilder` | `DynamicRow` |
| Alles, was die Builder nicht ausdrücken können - CTEs, Window-Funktionen, Backend-DDL | `DB::select` / `DB::statement` / `DB::affecting_statement` | `DynamicRow` / `bool` / `u64` |

`DbTableBuilder` existiert für den mittleren Fall. Sie bekommen die
WHERE-/ORDER-/LIMIT-Kette, ohne sich auf eine
`#[suprnova::model]`-Struktur festzulegen und ohne ganz auf rohe
SQL-Strings herunterzufallen.

## Die verkettbare Oberfläche

`DB::table(name)` liefert einen `DbTableBuilder`. Bauen Sie ihn auf und
rufen Sie dann eine Abschlussmethode auf, um ihn auszuführen.

### Filtern

```rust
// Gleichheit.
DB::table("users").filter("email", "alice@example.com").get().await?;

// Beliebiger Operator. Allowlist: =, <>, <, <=, >, >=, LIKE, NOT LIKE,
// ILIKE, NOT ILIKE, IS, IS NOT.
DB::table("orders").filter_op("total", ">=", 100i64).get().await?;
DB::table("posts").filter_op("title", "LIKE", "%rust%").get().await?;

// Mehrere Filter werden mit AND verknüpft.
DB::table("audit_log")
    .filter("actor_id", 42i64)
    .filter_op("event", "<>", "noop")
    .get()
    .await?;
```

`filter` und `filter_op` akzeptieren beide jedes `Into<SeaValue>` auf
der rechten Seite, was `i64`, `String`, `&str`, `bool`, `f64`,
`Option<T>`, `chrono::*`, `uuid::Uuid` und `serde_json::Value`
abdeckt - jeden Spaltentyp, den das Backend versteht.

### Spalten auswählen

```rust
// Der Standard ist SELECT *.
DB::table("users").get().await?;

// Spalten einschränken, wenn Sie nur einige brauchen.
DB::table("users").select(["id", "email"]).get().await?;
```

### Sortieren und Fenstern

```rust
DB::table("posts")
    .order_by_desc("created_at")
    .order_by_asc("title")
    .limit(20)
    .offset(40)
    .get()
    .await?;
```

`order_by_desc` und `order_by_asc` verketten sich in der Reihenfolge
ihres Einfügens; das erzeugte SQL bewahrt sie.

### Abschlussmethoden

```rust
// Alle passenden Zeilen.
let rows: Collection<DynamicRow> = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .get()
    .await?;

// Erste Zeile oder None.
let first: Option<DynamicRow> = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

// Nur die Anzahl (leert vor dem Rendern jedes
// select/order/limit/offset - die Count-Semantik ignoriert die).
let n: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;
```

`get()` liefert `Collection<DynamicRow>` - denselben
Collection-Wrapper, den typisierte Modelle verwenden, mit derselben
Oberfläche aus `.iter()`, `.len()` und `.into_vec()`. Siehe
[Eloquent Collections](eloquent-collections.md).

### Inserts, Updates, Deletes

```rust
use suprnova::attrs;

// INSERT, liefert die Auto-Increment-ID der neuen Zeile.
let id: i64 = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

// UPDATE, liefert die Anzahl betroffener Zeilen.
let updated: u64 = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

// DELETE, liefert die Anzahl betroffener Zeilen.
let deleted: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

Das Makro `attrs!` baut die Spalten-zu-Wert-Map an der Aufrufstelle.
Schlüssel sind SQL-Identifier (validiert), und Werte werden als
Parameter gebunden. Ein expliziter Nullwert wird als SQL `NULL`
ausgegeben, weil die JSON-Attribut-Map ihren ursprünglichen Rust-Typ
nicht mehr mitführt; alle Nicht-Null-Werte bleiben parametergebunden.
Dieselbe Regel gilt für typisierte Eloquent-Massenschreibvorgänge und
für zusätzliche Attribute in Viele-zu-viele-Pivots.

#### Die Aliase `update_all` und `delete_all`

`update` und `delete` sind die Laravel-treuen Namen. Die Aliase im Stil
von `Builder<M>` - `update_all` und `delete_all` - rufen dieselbe
Implementierung auf. Bevorzugen Sie die `_all`-Form, wenn die
tabellenweite Absicht der Punkt der Aufrufstelle ist; sie macht ein
fehlendes `filter` für Reviewer sichtbar:

```rust
// Gleiches Verhalten wie DB::table("rate_limits").delete().await?, aber
// das Suffix _all sagt Reviewern: „ja, ich wollte die Tabelle leeren“.
DB::table("rate_limits").delete_all().await?;

// Massen-Update mit einem WHERE - das Suffix _all entspricht hier der
// Konvention des typisierten Builder<M> für dieselbe Operation.
DB::table("sessions")
    .filter_op("expires_at", "<", chrono::Utc::now())
    .update_all(attrs! { status: "expired" })
    .await?;
```

#### Ein leeres WHERE bei Update oder Delete betrifft jede Zeile

`DB::table("x").delete().await?` entfernt jede Zeile der Tabelle. Das
ist absichtlich unterstützt - manchmal wollen Sie eine Tabelle wirklich
leeren -, aber es ist selten richtig. Sehen Sie sich jeden Aufruf von
`delete()` / `delete_all()` an und prüfen Sie, ob ein `filter`
davorsteht. Für `update` / `update_all` gilt dasselbe.

#### Backend-Aufteilung beim Insert

`RETURNING id` wird auf Postgres und SQLite verwendet. MySQL
unterstützt `RETURNING` nicht, also führt der Builder den INSERT aus und
liest das `last_insert_id()` des Treibers pro Verbindung aus dem
Ergebnis. Der modell-lose Builder setzt einen standardmäßigen
Auto-Increment-Primärschlüssel `id` voraus. UUID-, zusammengesetzte,
umbenannte oder nicht ganzzahlige Primärschlüssel werden auf dieser
Oberfläche nicht unterstützt - verwenden Sie stattdessen die typisierte
`Model`-Schnittstelle von [Eloquent](eloquent.md), die für die Form des
Primärschlüssels die Modelldefinition heranzieht.

## `DynamicRow` - typisierte Zugriffsmethoden über eine JSON-Map

Jede Zeile, die `DB::table` oder `DB::select` liefert, materialisiert
sich als `DynamicRow`, ein `serde_json::Map<String, Value>`-Newtype
mit typisierten Zugriffsmethoden. Jeder Getter liefert
`Result<T, FrameworkError>` mit einer klaren Fehlermeldung bei
fehlendem Key oder Typ-Mismatch.

```rust
for row in rows.iter() {
    let id: i64                 = row.get_int("id")?;
    let event: String           = row.get_string("event")?;
    let active: bool            = row.get_bool("active")?;
    let weight: f64             = row.get_float("weight")?;
    let payload: serde_json::Value = row.get_value("payload")?;
}
```

Für nullbare Spalten verwenden Sie `get_optional_*`. Diese
unterscheiden „Spalte fehlt“ (Fehler - Schema-Mismatch) von „Spalte
vorhanden, Wert SQL NULL“ (`Ok(None)`):

```rust
let title: Option<String> = row.get_optional_string("title")?;
let score: Option<i64>    = row.get_optional_int("score")?;
```

Heute deckt die Optional-Familie `String` und `i64` ab. Für andere
nullbare Typen verwenden Sie `get_value` und matchen selbst auf
`serde_json::Value::Null`, oder lesen Sie die Spalte über
`get_as::<Option<T>>` (jedes `T: DeserializeOwned`).

Um eine Spalte in eine beliebige Struktur oder einen Container-Typ zu
deserialisieren, verwenden Sie `get_as`. Die vollständige
`serde_json`-Deserialisierungs-Oberfläche steht zur Verfügung:

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

`DynamicRow` dereferenziert zu `Map<String, Value>`, sodass Iteration und
Key-Existenz-Prüfungen direkt funktionieren:

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}

if row.contains_key("deleted_at") { /* … */ }
```

## Vertrauensgrenze für Identifier

Tabellennamen, Spaltennamen, ORDER-BY-Richtungen und SQL-Operatoren
werden wortwörtlich in den SQL-String interpoliert - sie werden NICHT
als Parameter gebunden (SQL erlaubt keine platzhaltergebundenen
Identifier). Behandeln Sie jedes `impl Into<String>`-Argument als
vertrauenswürdiges Literal zur Compile-Zeit.

```rust
// Sicher - der Spaltenname ist eine Konstante; der Wert wird gebunden.
DB::table("users").filter("email", request.email()).get().await?;

// UNSICHER - spleißen Sie niemals Nutzereingaben in einen Spaltennamen.
DB::table("users")
    .filter(request.user_supplied_column(), value)
    .get()
    .await?;
```

Das Framework erzwingt eine strikte Allowlist an der I/O-Grenze -
Identifier müssen zu `[A-Za-z_][A-Za-z0-9_]*` mit einem optionalen
`schema.`-Präfix passen, und Operatoren müssen aus einer festen Liste
stammen. Verstöße scheitern fail-closed mit einem
`FrameworkError::Database`, bevor irgendein SQL gerendert wird. Das
ist ein Sicherheitsnetz, keine Lizenz: Halten Sie Identifier in Ihrem
Code literal.

Werte auf der rechten Seite von `filter` / `filter_op` werden immer
als Parameter gebunden und können sicher aus Request-Daten
durchgereicht werden.

## Rohe Queries

Wenn der Builder nicht ausdrücken kann, was Sie brauchen - rekursive
CTEs, Window-Funktionen, backend-spezifisches DDL,
`INSERT … ON CONFLICT DO UPDATE` - fallen Sie auf einen rohen String
zurück. Platzhalter passen zum aktiven Backend (`$1, $2, …` für
Postgres, `?` für MySQL und SQLite); das Framework erkennt es
automatisch aus `DatabaseConfig::url`.

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - jede Zeile als DynamicRow.
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) AS post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING COUNT(p.id) > ?",
    vec![Value::from(5i64)],
).await?;

// SELECT - nur die erste Zeile, spiegelt Laravels DB::selectOne.
let alice = DB::select_one(
    "SELECT * FROM users WHERE email = ?",
    vec![Value::from("alice@example.com")],
).await?;

// SELECT - erste Spalte der ersten Zeile als typisierter Skalar.
let total: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// INSERT - true, wenn mindestens eine Zeile betroffen war.
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - liefern die Anzahl betroffener Zeilen.
let updated: u64 = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1i64)],
).await?;

let deleted: u64 = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// Jedes Prepared Statement mit Bindings.
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1i64), Value::from(42i64)],
).await?;

// DDL oder andere Statements ohne Bindings, die Platzhalter-Bindung
// zurückweisen.
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// Genereller „betroffene Zeilen“-Pfad - für Upserts und Operationen,
// die nicht in die benannten Helfer passen.
let n: u64 = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1)
     ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec![Value::from("page_views")],
).await?;
```

### Falle bei Aggregat-Spalten

Untypisierte Aggregate wie `SELECT COUNT(*) AS n FROM t` funktionieren
über den `.count()`-Helfer des Builders, kommen bei rohen
`DB::select`-Zeilen auf SQLite aber unter Umständen stillschweigend
fallen gelassen zurück. Der zugrunde liegende Zeilen-Materialisierer
läuft sqlxs Typinformation pro Spalte ab, und ein bloßes Aggregat
trägt keine. Brauchen Sie rohes `DB::select` mit Aggregaten auf
SQLite, hüllen Sie den Ausdruck entweder in `CAST(… AS BIGINT)`, um
ihm ein Typ-Tag zu geben, oder verwenden Sie `DB::scalar::<i64>`, das
über `query_one` + `try_get` läuft und nicht von der
Typ-Erkennung pro Spalte abhängt.

## Brücke zum typisierten Eloquent

Wenn die Tabelle eine `#[suprnova::model]`-Struktur wert ist, trägt
die verkettbare Form über. `Model::query()` liefert `Builder<M>`, der
dieselbe `filter` / `filter_op` / `order_by_*` / `limit` / `offset` /
`get` / `first` / `count`-Oberfläche mitbringt - plus ein deutlich
breiteres WHERE-Vokabular (`filter_in`, `filter_between`,
`filter_null`, `filter_has`, `filter_raw`, …) und Laravel-förmige
Aliase (`db_where`, `where_in`, `where_between`, `where_null`,
`where_has`, `where_raw`, …).

```rust
use suprnova::Model;

let admins = User::query()
    .filter("role", "admin")
    .filter_op("created_at", ">=", since)
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;     // Collection<User> - typisiert, kein DynamicRow

let alice = User::query().filter("email", &email).first().await?;
let total = User::query().filter("active", true).count().await?;
// Hinweis: Builder<M>::count liefert i64 (entspricht Laravels
// Eloquent), während DbTableBuilder::count u64 liefert. Beide
// Oberflächen liefern Ihnen ein nicht-negatives SQL COUNT - sie
// unterscheiden sich nur in ihrem Wire-Typ.
```

Die vollständige `Builder<M>`-Oberfläche - jede WHERE-Form, Aggregate,
Relationen, Eager Loading, Scopes, Paginatoren, Chunk-Iteration - ist
in [Eloquent](eloquent.md) beschrieben. Die verkettbare Form, die Sie
oben gelernt haben, ist dieselbe Form; die Unterschiede liegen in
Typisierung und Reichweite.

## Zu einer benannten Connection routen

`DB::table` und die rohen Helfer verwenden standardmäßig die primäre
Connection. Um eine Read-Replica, einen Shard oder einen Warehouse-
Pool anzusteuern, pinnen Sie den Aufruf:

```rust
// Builder, an eine benannte Connection gepinnt.
let rows = DB::table("audit_log").on("warehouse").get().await?;

// Äquivalente Kurzform.
let rows = DB::table_on("warehouse", "audit_log").get().await?;

// Auch die rohen Escapes haben _on-Varianten.
let rows = DB::select_on("warehouse", "SELECT …", vec![]).await?;
let n    = DB::affecting_statement_on(
    "warehouse",
    "UPDATE …",
    vec![],
).await?;
```

Wenn `__read_replica__` registriert ist, routet jede lesende
Abschlussmethode automatisch darüber; Writes (`insert` / `update` /
`delete` / `update_all` / `delete_all`) zielen immer auf die primäre.
Innerhalb einer `DB::transaction`-Closure gewinnt die Connection der
aktiven Transaktion unbedingt - `on(name)` wird stillschweigend
ignoriert, um Atomarität zu erhalten. Siehe
[Datenbank - Benannte Connections](database.md) für die vollständige
Vorrangkette.

### Warum Suprnova abweicht

Laravels `DB::table(...)` ist sein modell-loser Query Builder;
darunter liefert es pro Zeile ein `stdClass` (ein PHP-Objekt, dessen
Eigenschaften die Spalten sind). Suprnova liefert statt dessen
`DynamicRow` - einen `serde_json::Map`-Newtype mit typisierten
Zugriffsmethoden. Die Form der Zugriffsmethoden fängt Fehler durch
fehlende Spalten oder falsche Typen an der Grenze ab, statt tief im
Nutzercode mit einer Property-Access-Exception zu paniken.

Die doppelten Namen `update`/`update_all` und `delete`/`delete_all`
gibt es, weil die typisierte Eloquent-Oberfläche `Builder<M>` das
Suffix `_all` verwendet, um die tabellenweite Absicht an der
Aufrufstelle explizit zu machen. Statt sich für eine Seite zu
entscheiden, liefert der modell-lose Builder beide - `update` und
`delete` entsprechen Laravels `DB::table($t)->update(...)` und
`->delete()` buchstabengetreu; `update_all` und `delete_all`
entsprechen der Konvention, die Nutzer von `M` schon im Muskelgedächtnis
haben.

## Nächste Schritte

- [Datenbank](database.md) - `DB`-Facade, Transaktionen mit
  Savepoints, `DB::listen`-Observability, benannte Connections
- [Eloquent](eloquent.md) - typisierte `#[suprnova::model]`-Strukturen
  und die vollständige `Builder<M>`-Oberfläche
- [Paginierung](pagination.md) - `paginate` / `simple_paginate` /
  `cursor_paginate` auf typisierten Buildern
- [Eloquent Collections](eloquent-collections.md) - die `Collection<T>`,
  die `get()` auf beiden Oberflächen liefert
- [Migrationen](migrations.md) - definiert das Schema, das die Builder
  abfragen

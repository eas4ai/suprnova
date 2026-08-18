# Constructor de consultas

Para consultar una tabla sin modelarla como un struct
`#[suprnova::model]` tipado, recurre a `DB::table(name)`. Devuelve un
builder encadenable con la misma forma que el `Builder<M>` tipado de
Eloquent, pero materializa las filas como `DynamicRow` - un newtype de
`serde_json::Map` con accesores tipados. Este es el capítulo para
registros de auditoría, informes ad hoc, agregados de panel, y
cualquier tabla que no se haya modelado. Para el equivalente tipado,
consulta [Eloquent](eloquent.md). Para `DB::select` en bruto dentro de
transacciones o con observación vía `DB::listen`, consulta
[Base de datos](database.md).

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

## Qué superficie usar

Tres superficies de consulta se superponen; elige la adecuada para la tabla.

| La tabla es… | Usa | Devuelve |
|---|---|---|
| Modelada con `#[suprnova::model]` | `Model::query()` → `Builder<M>` | valores `M` tipados |
| Sin modelar, pero se quiere una cadena WHERE/ORDER/LIMIT encadenable | `DB::table(name)` → `DbTableBuilder` | `DynamicRow` |
| Cualquier cosa que los builders no puedan expresar - CTEs, funciones de ventana, DDL de backend | `DB::select` / `DB::statement` / `DB::affecting_statement` | `DynamicRow` / `bool` / `u64` |

`DbTableBuilder` existe para el caso intermedio. Se obtiene la cadena
WHERE / ORDER / LIMIT sin comprometerse con un struct
`#[suprnova::model]` y sin caer del todo en cadenas SQL en bruto.

## La superficie encadenable

`DB::table(name)` devuelve un `DbTableBuilder`. Constrúyelo y luego llama
a un método terminal para ejecutarlo.

### Filtrado

```rust
// Igualdad.
DB::table("users").filter("email", "alice@example.com").get().await?;

// Operador arbitrario. Lista de permitidos: =, <>, <, <=, >, >=, LIKE,
// NOT LIKE, ILIKE, NOT ILIKE, IS, IS NOT.
DB::table("orders").filter_op("total", ">=", 100i64).get().await?;
DB::table("posts").filter_op("title", "LIKE", "%rust%").get().await?;

// Varios filtros se combinan con AND.
DB::table("audit_log")
    .filter("actor_id", 42i64)
    .filter_op("event", "<>", "noop")
    .get()
    .await?;
```

`filter` y `filter_op` aceptan ambos cualquier `Into<SeaValue>` en el
lado derecho, lo que cubre `i64`, `String`, `&str`, `bool`, `f64`,
`Option<T>`, `chrono::*`, `uuid::Uuid` y `serde_json::Value` - todos los
tipos de columna que el backend entiende.

### Seleccionar columnas

```rust
// Por defecto es SELECT *.
DB::table("users").get().await?;

// Restringe las columnas cuando solo necesitas algunas.
DB::table("users").select(["id", "email"]).get().await?;
```

### Ordenación y ventana

```rust
DB::table("posts")
    .order_by_desc("created_at")
    .order_by_asc("title")
    .limit(20)
    .offset(40)
    .get()
    .await?;
```

`order_by_desc` y `order_by_asc` se encadenan en el orden de inserción;
el SQL generado lo conserva.

### Terminales

```rust
// Todas las filas que coinciden.
let rows: Collection<DynamicRow> = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .get()
    .await?;

// La primera fila, o None.
let first: Option<DynamicRow> = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

// Solo el recuento (limpia cualquier select/order/limit/offset antes de
// renderizar - la semántica de count no los tiene en cuenta).
let n: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;
```

`get()` devuelve `Collection<DynamicRow>` - el mismo envoltorio de
colección que usan los modelos tipados, con la misma superficie
`.iter()`, `.len()`, `.into_vec()`. Consulta
[Colecciones de Eloquent](eloquent-collections.md).

### Inserciones, actualizaciones, eliminaciones

```rust
use suprnova::attrs;

// INSERT; devuelve el id autoincremental de la nueva fila.
let id: i64 = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

// UPDATE; devuelve las filas afectadas.
let updated: u64 = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

// DELETE; devuelve las filas afectadas.
let deleted: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

La macro `attrs!` construye el mapa de columna a valor en el sitio de la
llamada. Las claves son identificadores SQL (validados) y los valores se
vinculan como parámetros. Un null explícito se emite como `NULL` de SQL
porque el mapa de atributos JSON ya no lleva su tipo original de Rust;
todos los valores no nulos siguen vinculados como parámetros. La misma
regla se aplica a las escrituras masivas tipadas de Eloquent y a los
atributos adicionales de los pivots many-to-many.

#### Alias `update_all` y `delete_all`

`update` y `delete` son los nombres fieles a Laravel. Los alias con
estilo `Builder<M>` - `update_all` y `delete_all` - llaman a la misma
implementación. Prefiere la forma `_all` cuando la intención de afectar a
toda la tabla sea el sentido mismo de la llamada; hace visible un
`filter` ausente para quien revisa:

```rust
// El mismo comportamiento que DB::table("rate_limits").delete().await?,
// pero el sufijo _all le dice a quien revisa "sí, quería truncar la tabla".
DB::table("rate_limits").delete_all().await?;

// Actualización masiva con un WHERE - aquí el sufijo _all coincide con la
// convención del Builder<M> tipado para la misma operación.
DB::table("sessions")
    .filter_op("expires_at", "<", chrono::Utc::now())
    .update_all(attrs! { status: "expired" })
    .await?;
```

#### Un WHERE vacío en update o delete opera sobre todas las filas

`DB::table("x").delete().await?` elimina todas las filas de la tabla. Eso
está soportado por diseño - a veces de verdad quieres truncar - pero rara
vez es lo correcto. Mira siempre una llamada a `delete()` /
`delete_all()` y comprueba si hay un `filter` delante. Lo mismo vale para
`update` / `update_all`.

#### División de backend en el insert

`RETURNING id` se usa en Postgres y SQLite. MySQL no soporta `RETURNING`,
así que el builder ejecuta el INSERT y lee del resultado el
`last_insert_id()` por conexión del driver. El builder sin modelo asume
una clave primaria `id` autoincremental estándar. Las claves primarias
UUID, compuestas, renombradas o no enteras no están soportadas en esta
superficie - usa en su lugar la interfaz `Model` tipada de
[Eloquent](eloquent.md), que consulta la definición del modelo para saber
la forma de la clave primaria.

## `DynamicRow` - accesores tipados sobre un mapa JSON

Cada fila devuelta por `DB::table` o `DB::select` se materializa como
`DynamicRow`, un newtype de `serde_json::Map<String, Value>` con
accesores tipados. Cada getter devuelve `Result<T, FrameworkError>`
con un mensaje de error claro ante una clave ausente o un tipo que no
coincide.

```rust
for row in rows.iter() {
    let id: i64                 = row.get_int("id")?;
    let event: String           = row.get_string("event")?;
    let active: bool            = row.get_bool("active")?;
    let weight: f64             = row.get_float("weight")?;
    let payload: serde_json::Value = row.get_value("payload")?;
}
```

Para columnas anulables, usa `get_optional_*`. Estos distinguen
"columna ausente" (error - desajuste de esquema) de "columna presente,
valor SQL NULL" (`Ok(None)`):

```rust
let title: Option<String> = row.get_optional_string("title")?;
let score: Option<i64>    = row.get_optional_int("score")?;
```

Hoy la familia opcional cubre `String` e `i64`. Para otros tipos
anulables, usa `get_value` y compara contra `serde_json::Value::Null`
manualmente, o lee la columna a través de `get_as::<Option<T>>`
(cualquier `T: DeserializeOwned`).

Para deserializar una columna en cualquier struct o tipo contenedor,
usa `get_as`. La superficie completa de deserialización de
`serde_json` está disponible:

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

`DynamicRow` hace deref hacia `Map<String, Value>`, así que la
iteración y la comprobación de existencia de claves funcionan
directamente:

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}

if row.contains_key("deleted_at") { /* … */ }
```

## Límite de confianza de los identificadores

Los nombres de tabla, los nombres de columna, las direcciones de
ORDER BY, y los operadores SQL se interpolan textualmente en la cadena
SQL - NO se vinculan como parámetros (SQL no permite identificadores
vinculados por placeholder). Trata cada argumento `impl Into<String>`
como un literal de confianza, fijado en tiempo de compilación.

```rust
// Seguro - el nombre de la columna es una constante; el valor se vincula.
DB::table("users").filter("email", request.email()).get().await?;

// INSEGURO - nunca inyectes entrada del usuario en un nombre de columna.
DB::table("users")
    .filter(request.user_supplied_column(), value)
    .get()
    .await?;
```

El framework aplica una lista de permitidos estricta en el límite de
E/S - los identificadores deben coincidir con `[A-Za-z_][A-Za-z0-9_]*`
con un prefijo `schema.` opcional, y los operadores deben provenir de
una lista fija. Las violaciones fallan de forma cerrada con un
`FrameworkError::Database` antes de que se genere ningún SQL. Eso es
una red de seguridad, no una licencia: mantén los identificadores
literales en el código.

Los valores en el lado derecho de `filter` / `filter_op` siempre se
vinculan como parámetros y son seguros de inyectar directamente desde
datos de la solicitud.

## Consultas en bruto

Cuando el builder no puede expresar lo necesario - CTEs recursivas,
funciones de ventana, DDL específico del backend, `INSERT … ON
CONFLICT DO UPDATE` - se cae a una cadena en bruto. Los placeholders
coinciden con el backend activo (`$1, $2, …` para Postgres, `?` para
MySQL y SQLite); el framework lo autodetecta a partir de
`DatabaseConfig::url`.

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - cada fila como DynamicRow.
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) AS post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING COUNT(p.id) > ?",
    vec![Value::from(5i64)],
).await?;

// SELECT - solo la primera fila, refleja el DB::selectOne de Laravel.
let alice = DB::select_one(
    "SELECT * FROM users WHERE email = ?",
    vec![Value::from("alice@example.com")],
).await?;

// SELECT - primera columna de la primera fila como escalar tipado.
let total: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// INSERT - true cuando al menos una fila fue afectada.
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - devuelven el conteo de filas afectadas.
let updated: u64 = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1i64)],
).await?;

let deleted: u64 = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// Cualquier statement preparado con bindings.
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1i64), Value::from(42i64)],
).await?;

// DDL u otros statements sin binding que rechazan el binding de placeholders.
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// Ruta genérica de "filas afectadas" - para upserts y operaciones que
// no encajan en los helpers con nombre.
let n: u64 = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1)
     ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec![Value::from("page_views")],
).await?;
```

### La trampa de las columnas agregadas

Los agregados sin tipo como `SELECT COUNT(*) AS n FROM t` funcionan a
través del helper `.count()` del builder, pero pueden volver
silenciosamente descartados desde filas de `DB::select` en bruto sobre
SQLite. El materializador de filas subyacente recorre la información
de tipo por columna de sqlx, y un agregado desnudo no lleva ninguna. Si
se necesita `DB::select` en bruto con agregados sobre SQLite, o bien
se envuelve la expresión en `CAST(… AS BIGINT)` para darle una
etiqueta de tipo, o se usa `DB::scalar::<i64>`, que pasa por
`query_one` + `try_get` y no depende de la detección de tipo por
columna.

## Puente hacia Eloquent tipado

Cuando la tabla merece un struct `#[suprnova::model]`, la forma
encadenable se conserva. `Model::query()` devuelve `Builder<M>`, que
ofrece la misma superficie `filter` / `filter_op` / `order_by_*` /
`limit` / `offset` / `get` / `first` / `count` - más un vocabulario
WHERE mucho más amplio (`filter_in`, `filter_between`, `filter_null`,
`filter_has`, `filter_raw`, …) y alias con la forma de Laravel
(`db_where`, `where_in`, `where_between`, `where_null`, `where_has`,
`where_raw`, …).

```rust
use suprnova::Model;

let admins = User::query()
    .filter("role", "admin")
    .filter_op("created_at", ">=", since)
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;     // Collection<User> - tipado, no DynamicRow

let alice = User::query().filter("email", &email).first().await?;
let total = User::query().filter("active", true).count().await?;
// Nota: Builder<M>::count devuelve i64 (coincide con el Eloquent de
// Laravel), mientras que DbTableBuilder::count devuelve u64. Ambas
// superficies dan un SQL COUNT no negativo - solo difieren en su tipo
// sobre el wire.
```

La superficie completa de `Builder<M>` - cada forma de WHERE,
agregados, relaciones, carga anticipada, scopes, paginadores,
iteración por chunks - está en [Eloquent](eloquent.md). La forma
encadenable aprendida arriba es la misma forma; las diferencias están
en el tipado y el alcance.

## Enrutar a una conexión con nombre

`DB::table` y los helpers en bruto usan por defecto la conexión
primaria. Para apuntar a una réplica de lectura, un shard, o un pool
de almacén de datos, fija la llamada:

```rust
// Builder fijado a una conexión con nombre.
let rows = DB::table("audit_log").on("warehouse").get().await?;

// Abreviatura equivalente.
let rows = DB::table_on("warehouse", "audit_log").get().await?;

// Los escapes en bruto también tienen variantes _on.
let rows = DB::select_on("warehouse", "SELECT …", vec![]).await?;
let n    = DB::affecting_statement_on(
    "warehouse",
    "UPDATE …",
    vec![],
).await?;
```

Cuando `__read_replica__` está registrada, cada terminal de lectura se
enruta automáticamente a través de ella; las escrituras (`insert` /
`update` / `delete` / `update_all` / `delete_all`) siempre apuntan a
la primaria. Dentro de un closure `DB::transaction` la conexión de la
transacción activa gana de forma absoluta - `on(name)` se ignora en
silencio para preservar la atomicidad. Consulta
[Base de datos - Conexiones con nombre](database.md) para la cadena de
precedencia completa.

### Por qué Suprnova diverge

El `DB::table(...)` de Laravel es su constructor de consultas sin
modelo; por debajo devuelve un `stdClass` por fila (un objeto PHP cuyas
propiedades son las columnas). Suprnova devuelve `DynamicRow` en su
lugar - un newtype de `serde_json::Map` con accesores tipados. La forma
del accesor atrapa los errores de columna ausente y de tipo incorrecto
en el límite, en lugar de entrar en pánico en lo profundo del código de
usuario con una excepción de acceso a propiedad.

Los nombres duales `update`/`update_all` y `delete`/`delete_all`
existen porque la superficie tipada de `Builder<M>` de Eloquent usa el
sufijo `_all` para hacer explícita la intención sobre toda la tabla en
el propio sitio de la llamada. En lugar de elegir un bando, el builder
sin modelo ofrece ambos - `update` y `delete` coinciden letra por
letra con `DB::table($t)->update(...)` y `->delete()` de Laravel;
`update_all` y `delete_all` coinciden con la convención que quienes
usan `M` ya tendrán en la memoria muscular.

## Siguiente

- [Base de datos](database.md) - fachada `DB`, transacciones con
  puntos de guardado, observabilidad de `DB::listen`, conexiones con
  nombre
- [Eloquent](eloquent.md) - structs `#[suprnova::model]` tipados y la
  superficie completa de `Builder<M>`
- [Paginación](pagination.md) - `paginate` / `simple_paginate` /
  `cursor_paginate` sobre builders tipados
- [Colecciones de Eloquent](eloquent-collections.md) - la
  `Collection<T>` devuelta por `get()` en ambas superficies
- [Migraciones](migrations.md) - definir el esquema que consultan los
  builders

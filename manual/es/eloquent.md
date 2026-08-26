# API de Eloquent

La capa Eloquent de Suprnova ofrece a los desarrolladores de Laravel
la API que ya conocen, implementada como un shim ligero sobre SeaORM.
Copia código de la documentación de Laravel, cambia la sintaxis de PHP
por Rust, añade `.await?`, y funciona.

Toda la capa es un atributo de struct (`#[suprnova::model]`), un trait
(`Model`) y un constructor de consultas encadenable (`Builder<M>`) -
eso es todo. Detrás de escena, la macro genera un `Entity`, `Model`,
`ActiveModel` y un enum `Column` de SeaORM, más cada impl de trait de
Eloquent. Los tipos de SeaORM siguen siendo accesibles para el caso
raro en que la superficie de Eloquent no cubra algo (consulta las
[vías de escape de SeaORM](#bajar-a-seaorm)).

## Tabla de contenidos

- [Inicio rápido](#inicio-rápido)
- [El atributo `#[suprnova::model]`](#the-suprnovamodel-attribute)
- [Disposición del módulo del modelo](#disposición-del-módulo-del-modelo)
- [Encontrar filas](#encontrar-filas)
- [Crear y actualizar](#crear-y-actualizar)
- [Eliminar y eliminaciones suaves](#eliminar-y-eliminaciones-suaves)
- [Constructor de consultas - API dual](#query-builder--dual-api)
- [Bloqueo de filas](#bloqueo-de-filas)
- [Transacciones](#transacciones)
- [Scopes](#scopes)
- [Relaciones](#relaciones)
- [Carga anticipada](#carga-anticipada)
- [Paginación](#paginación)
- [Iteración por chunks y perezosa](#iteración-por-chunks-y-perezosa)
- [Colecciones](#colecciones)
- [Asignación masiva](#asignación-masiva)
- [Casts](#casts)
- [Accesores y mutadores](#accesores-y-mutadores)
- [Timestamps](#timestamps)
- [Observers y eventos de ciclo de vida](#observers-y-eventos-de-ciclo-de-vida)
- [Prunable](#prunable)
- [Enrutamiento multi-conexión](#enrutamiento-multi-conexión)
- [Replicación](#replicación)
- [Depuración - dump y dd](#debugging--dump-and-dd)
- [Pruebas de modelos](#pruebas-de-modelos)
- [Bajar a SeaORM](#bajar-a-seaorm)
- [Migrar desde `database::Model`](#migrating-from-databasemodel)
- [Fachada DB - consultas sin modelo](#db-facade--model-less-queries)
- [Paridad con Laravel 13 - existencia de relación + atajos económicos](#laravel-13-parity--relation-existence--cheap-shortcuts)

## Inicio rápido

Un atributo en un struct lo convierte en un modelo Eloquent con todas
las funciones:

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

Una vez declarado, puedes escribir:

- `User::query()` - inicia un constructor de consultas fluido.
- `User::find(id).await?` - obtiene por clave primaria.
- `User::find_or_fail(id).await?` - igual, pero falla con
  `ModelNotFound` si no encuentra nada.
- `User::all().await?` - todas las filas.
- `User::create(attrs!{ name: "Alice", email: "alice@example.com" }).await?` -
  inserta con filtrado de asignación masiva.
- `User::filter("email", "alice@example.com").first().await?` -
  una fila que coincide.
- `user.update(attrs!{ name: "Alice B" }).await?` - actualización parcial.
- `user.save().await?` - persiste los cambios en memoria.
- `user.delete().await?` - elimina la fila.
- `user.refresh().await?` / `user.fresh().await?` / `user.replicate().await?` -
  el resto del ciclo de vida de Laravel.

El struct de cara al usuario (aquí `User`) ES el tipo que llevan tus
handlers y controladores. La macro emite un módulo interno por modelo
(`user::`) con los tipos `Entity`, `Column`, `ActiveModel` y `Model` de
SeaORM, para los casos en que quieras bajar a SeaORM directamente. El
struct también se registra en un `ModelEntry` respaldado por
inventario, de modo que el código de administración y de herramientas
pueda enumerar cada modelo al arrancar.

## El atributo `#[suprnova::model]`

El único punto de entrada para declarar un modelo. Todos los
atributos son opcionales; los valores por defecto están ajustados
para que un struct con `id` + `created_at` + `updated_at` funcione
como modelo de Suprnova sin ninguna configuración.

### Referencia de atributos de la macro

| Atributo | Tipo | Valor por defecto | Notas |
|-----------|------|---------|-------|
| `table` | string | snake_case en plural del nombre del struct | Sobrescribe el nombre de la tabla |
| `primary_key` | string | `"id"` | Sobrescribe el nombre de la columna de la PK |
| `key_type` | type | `i64` | Tipo de la PK - `String` para UUID, `i32` para esquemas heredados |
| `auto_increment` | bool | `true` | Desactívalo para PKs de tipo UUID |
| `connection` | string | `"default"` | Las apps multi-conexión nombran una conexión no predeterminada |
| `fillable` | lista de strings | (por defecto = `guarded = ["id"]`) | Lista de permitidos para la asignación masiva |
| `guarded` | lista de strings | `["id"]` cuando no se fija ninguno | Lista de bloqueo para la asignación masiva (mutuamente excluyente con `fillable`) |
| `casts` | mapa de `field = CastType` | `{}` | Casts por columna |
| `hidden` | lista de strings | `[]` | Excluido de `to_json` / `to_array` |
| `visible` | lista de strings | (todos) | Variante inclusiva de `hidden` (mutuamente excluyente) |
| `appends` | lista de strings | `[]` | Accesores a incluir en la serialización |
| `soft_deletes` | flag | `false` | Activa la columna `deleted_at` y la semántica de marcado de borrado |
| `soft_deletes_column` | string | `"deleted_at"` | Sobrescribe el nombre de la columna de eliminación suave |
| `timestamps` | flag / bool | `true` cuando existen tanto `created_at` como `updated_at` | Desactiva los timestamps autogestionados |
| `created_at` | string | `"created_at"` | Sobrescribe el nombre de la columna |
| `updated_at` | string | `"updated_at"` | Sobrescribe el nombre de la columna |
| `touches` | lista de nombres de relación | `[]` | Relaciones `BelongsTo` cuyo registro padre incrementa su `updated_at` después de que este modelo se crea, guarda, actualiza o elimina |
| `mutators` | lista de strings | `[]` | Nombres de campo cuya ruta de llenado JSON se enruta a través de un método mutador `set_<field>(value)` |

### Ejemplo completo

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

### Macros a nivel de función

Las macros a nivel de función funcionan junto con el atributo de
struct:

- `#[accessor]` en un `fn name(&self) -> T` lo convierte en un
  accesor de Eloquent. El `to_array()` del modelo lo llama cuando
  `name` aparece en `appends = [...]` (y `to_json()` lo recoge a
  través de la delegación `to_array` → string).
- `#[mutator]` en un `fn set_name(&mut self, value: serde_json::Value)`
  lo convierte en un mutador de Eloquent. La ruta de llenado JSON del
  modelo se enruta a través de él cuando `name` aparece en
  `mutators = [...]`.
- `#[suprnova::scopes(Model)]` en un bloque `impl Model { ... }`:
  cada método cuya firma es
  `fn name(query: Builder<Self>[, args…]) -> Builder<Self>` se
  convierte tanto en un `.scope_name(args)` encadenable sobre
  `Builder<Self>` como en un atajo `Model::scope_name(args)`. No
  existe una forma `#[scope]` a nivel de función - los scopes se
  declaran por bloque impl.
- Los scopes globales son un registro en tiempo de ejecución a
  través del trait `GlobalScope`, aplicado mediante
  `Model::global_scope::<GS>()`. No existe una macro
  `#[global_scope]` a nivel de función - consulta
  [Macros](macros.md#suprnova-scopes-model) para el patrón completo.
- `#[prunable]` en `impl Prunable for T { ... }` registra el pruner
  a través de inventario, para que `model:prune` lo encuentre.

## Disposición del módulo del modelo

`#[suprnova::model]` mantiene tu struct de cara al usuario (por
ejemplo, `Post`) en el ámbito superior y emite un `pub mod` hermano
con el nombre del struct en snake_case (`post`). Ese módulo interno
es donde viven los tipos de SeaORM.

Para un modelo declarado en `app/src/models/posts.rs`:

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

// Convención: reexporta los tipos de SeaORM que emite la macro
// dentro del módulo interno, para que los sitios de llamada puedan
// usar los nombres sin prefijo. Los propios modelos dogfood de
// Suprnova llevan todos esta línea (ver `app/src/models/users.rs`,
// `app/src/models/posts.rs`, etc.).
pub use post::{ActiveModel, Column, Entity};
```

Ahora tienes estos elementos accesibles desde `crate::models::posts`:

| Ruta | Qué es |
|------|-----------|
| `crate::models::posts::Post` | Tu struct de cara al usuario - el modelo Eloquent |
| `crate::models::posts::post::Entity` | Impl de `EntityTrait` de SeaORM para la tabla `posts` |
| `crate::models::posts::post::Column` | Enum `Column` de SeaORM (una variante por columna) |
| `crate::models::posts::post::ActiveModel` | `ActiveModel` de SeaORM para insertar/actualizar |
| `crate::models::posts::post::Model` | Fila con forma SeaORM (columnas con el tipo de almacenamiento) |
| `crate::models::posts::{Entity, Column, ActiveModel}` | La convención `pub use` de arriba; no se emite automáticamente |

Dos cosas que hay que saber sobre el `Model` del módulo interno:

1. Es la fila con **forma SeaORM**, no tu struct `Post`. Aquí las
   columnas con cast llevan su tipo `Storage` (por ejemplo, `bool`
   se convierte en el entero subyacente), y no están presentes los
   slots en tiempo de ejecución `__eager` / `__pivot` de tu struct.
2. `From<post::Model> for Post` y `From<Post> for post::Model` unen
   las dos formas. Consulta [Bajar a SeaORM](#bajar-a-seaorm)
   para el patrón de ida y vuelta.

`Model` deliberadamente **no** forma parte de la reexportación
convencional al ámbito superior - el `Post` de cara al usuario ya
ocupa el nombre `Post` en ese ámbito, y `post::Model` es un tipo
independiente al que quien llama llega a través de `post::Model` (o
de una conversión `From`) cuando necesita la forma interna.

### Cuándo entrar en el módulo interno

La superficie de Eloquent (el trait `Model` + `Builder<M>`) cubre la
inmensa mayoría de las consultas. Entra en `post::*` cuando necesites
funciones exclusivas de SeaORM:

- **Construcción de consultas en bruto** con la cadena
  `EntityTrait::find()` de SeaORM, cuando Eloquent no expone el
  helper que necesitas.
- **Lógica de join personalizada** - construir joins `JoinType::*`
  explícitamente mediante `QuerySelect::join()` para una relación
  que el `with(...)` de Eloquent no modela.
- **Subconsultas nativas de SeaORM** mediante
  `Entity::find().select_only()`.
- **Mutación directa de `ActiveModel`** para el caso raro en que
  quieras saltarte el ciclo de vida de Eloquent (sin observers, sin
  timestamps automáticos).

```rust
// Caso común - Column reexportado a nivel del módulo superior
// mediante la convención `pub use post::{...}` de arriba.
use crate::models::posts::Column;

let drafts = Post::query()
    .db_where(Column::Status, "draft")
    .get()
    .await?;

// Caso avanzado - entra en el módulo interno para acceder
// directamente al Entity de SeaORM. Esto es lo que el `pub use`
// del módulo superior no expone.
use crate::models::posts::post;
use suprnova::sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

let db = suprnova::DB::connection()?;
let rows: Vec<post::Model> = post::Entity::find()
    .filter(post::Column::Status.eq("published"))
    .all(db.inner())
    .await?;

// Vuelve a la forma de Eloquent cuando quien llama la necesite.
let posts: Vec<Post> = rows.into_iter().map(Post::from).collect();
```

Si te encuentras entrando en el módulo interno de forma habitual para
la misma operación, es una señal de que a Eloquent le falta un
helper - abre un issue, o añade el helper a la superficie de `Model` /
`Builder`.

## Encontrar filas

```php
// Laravel
$user = User::find(1);
$user = User::findOrFail(1);          // lanza una excepción si falta
$users = User::findMany([1, 2, 3]);
```

```rust
// Suprnova
let user: Option<User> = User::find(1).await?;
let user: User = User::find_or_fail(1).await?;
let users: Vec<User> = User::find_many([1, 2, 3]).await?;
```

`find_or_fail` devuelve `FrameworkError::ModelNotFound` (HTTP 404
cuando llega hasta un controlador).

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
$user = User::firstOrNew(['email' => 'alice@example.com']);  // unsaved
```

```rust
// Suprnova
let user = User::first_or_create(
    attrs! { email: "alice@example.com" },   // claves de búsqueda
    attrs! { name: "Alice" },                // extras al crear
).await?;

let user = User::update_or_create(
    attrs! { email: "alice@example.com" },
    attrs! { name: "Alice Updated" },
).await?;

let user = User::first_or_new(
    attrs! { email: "alice@example.com" },
).await?;   // devuelve un User sin guardar; quien llama lo guarda explícitamente
```

Las claves de búsqueda van en el primer mapa; los campos extra que
se aplican en la ruta de creación van en el segundo. Devolver un
modelo sin guardar mediante `first_or_new` permite que quien llama lo
siga modificando antes de `save().await?`.

## Crear y actualizar

### Crear

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

`attrs!` es una macro que produce un valor `Attrs` (un mapa JSON
tipado). El JSON puro también funciona -
`User::create(serde_json::json!({"name": "Alice", "email": "..."}))`.
El filtro `Fillable` se ejecuta dentro de `create`; los campos no
incluidos en `fillable` se descartan silenciosamente, igual que en
Laravel.

### Guardar / actualizar

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

`save()` recorre cada campo que no es la PK, los fija en el
ActiveModel mediante `Set(...)`, llama al `update()` de SeaORM, y
devuelve la fila canónica. `update(attrs)` sigue el mismo flujo pero
antes aplica un mapa de atributos parcial (ejecutando el filtro
Fillable y los mutadores declarados).

### Incrementar / decrementar

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

`increment` / `decrement` emiten SQL `UPDATE table SET col = col + N
WHERE ...` - atómico frente a actualizaciones concurrentes, sin
condición de carrera de lectura-modificación-escritura. Disponible
tanto en una instancia de modelo ya obtenida (usa la PK de la fila en
la cláusula WHERE) como en un terminal del Builder (usa las cláusulas
WHERE de la cadena).

### Refrescar / recargar / replicar

```php
// Laravel
$user->refresh();                          // recarga desde la BD
$user->refreshForUpdate();                 // recarga bajo un bloqueo de fila
$copy = $user->fresh();                    // obtiene una copia y la devuelve
$replica = $user->replicate();             // clon sin guardar con PK nueva
$replica = $user->replicate(['email']);    // omite un campo
```

```rust
// Suprnova
user.refresh().await?;
user.refresh_for_update().await?;
let copy: User = user.fresh().await?;
let replica: User = user.replicate().await?;
let replica: User = user.replicate_except(["email"]).await?;
```

`refresh` muta en el sitio; `fresh` devuelve una copia obtenida por
separado. `refresh_for_update` es `refresh` bajo un bloqueo de fila
`SELECT ... FOR UPDATE` - úsalo dentro de una transacción cuando
necesites los valores actuales de la fila y el bloqueo exclusivo en una
sola sentencia. A diferencia de `refresh`, `refresh_for_update` se salta
todos los scopes globales registrados Y el filtro de
`#[model(soft_deletes)]`: recarga también una fila descartada, y
`deleted_at` vuelve con valor. La recarga es una búsqueda por clave
primaria bajo un bloqueo - acotarla como se acota una lectura corriente
les daría a las herramientas de administración y a quienes llaman desde
otro tenant un falso resultado de fila no encontrada para una fila de la
que ya tienen una referencia. `replicate` construye un clon en memoria
con la PK reiniciada (`Default::default()` para el tipo de la clave).
Quien llama la guarda explícitamente.

Tanto `refresh` como `refresh_for_update` devuelven un error cuando la
fila ya no existe, en lugar de dejar el modelo con valores obsoletos.
SQLite no tiene bloqueo a nivel de fila, así que allí
`refresh_for_update` recarga sin bloqueo - véase
[Bloqueo de filas](#bloqueo-de-filas).

### Evento `Replicating`

`replicate` y `replicate_except` disparan el evento
`Replicating { source, replica }` por modelo, después de construir
el clon en memoria y ANTES de devolverlo. El campo `replica` es un
`Arc<tokio::sync::Mutex<Self>>`, de modo que los oyentes pueden
mutar la réplica antes de que quien llama la vea - útil para
prefijar títulos con `(copy)`, limpiar flags, reiniciar columnas
derivadas, etc.

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

// Conéctalo una vez al arrancar:
EventFacade::listen::<post::events::Replicating, _>(
    std::sync::Arc::new(PrefixTitle)
).await;
```

### Replicación entre tipos

```rust
let replica: UserDraft = user.replicate_into().await?;  // clon entre tipos
```

Una divergencia de Suprnova - Laravel no puede hacer esto porque PHP
no tiene tipos. Útil para promover un modelo borrador a uno final, o
viceversa.

`replicate_into<T>` NO dispara `Replicating` (el evento lleva
`Arc<Mutex<Self>>`, así que un oyente sobre el tipo de origen no
podría mutar la réplica entre tipos de todos modos). Quien llame y
quiera una configuración por tipo `T` debería aplicarla sobre el `T`
devuelto antes de llamar a `T::save` - la cadena normal
`Saving` / `Created` sigue disparándose dentro de `save`.

## Eliminar y eliminaciones suaves

### Flag de eliminaciones suaves

Añade `soft_deletes` al atributo de la macro y una columna
`deleted_at: Option<DateTime<Utc>>` al struct:

```rust
#[model(table = "users", soft_deletes, timestamps)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub deleted_at: Option<DateTime<Utc>>,
    // ...
}
```

### Ciclo de vida

```rust
user.delete().await?;             // UPDATE: fija deleted_at = NOW()
user.trashed();                   // -> true
let trashed = User::with_trashed().find(user.id).await?.unwrap();
trashed.restore().await?;         // UPDATE: fija deleted_at = NULL

let only_dead = User::only_trashed().get().await?;
let all_including_dead = User::with_trashed().get().await?;

user.force_delete().await?;       // DELETE real
```

### Scope por defecto

Cuando se fija `soft_deletes`, la macro sobrescribe `Model::query()`
para que las lecturas por defecto filtren automáticamente las filas
descartadas. `with_trashed()` y `only_trashed()` permiten volver a
incluirlas. En concreto: `User::find(id)` omite las filas
descartadas; `User::with_trashed().find(id)` sí las encuentra.

## Constructor de consultas - API dual

`Builder<M>` es el tipo de consulta encadenable que devuelven
`User::query()`, `User::filter(...)`, `User::db_where(...)`, y
cualquier otro método estático que no termine la cadena.

### Nota sobre nomenclatura: API dual

`where` es una palabra clave de Rust, así que el método de igualdad
simple no puede compartir el nombre de Laravel. En lugar de elegir un
ganador, cada método con forma de where se distribuye bajo **ambos**
nombres: uno idiomático de Rust (`filter`, `filter_in`,
`filter_null`, …) y uno con la forma de Laravel (`db_where`,
`where_in`, `where_null`, …). Son alias sobre una única
implementación canónica - elige el que coincida con tu memoria
muscular.

```rust
// Para quien viene de Rust:
User::query().filter("active", true).filter_in("role", ["admin"]).get().await?;

// Para quien viene de Laravel:
User::db_where("active", true).where_in("role", ["admin"]).get().await?;

// Misma consulta. Mismo resultado. Distinta memoria muscular.
```

### Atajos de where

```php
// Laravel
$users = User::where('email', $email)->get();
$users = User::where('age', '>=', 18)->get();
$users = User::where('email', 'like', '%@example.com')->get();
```

```rust
// Suprnova - elige cualquiera de las dos familias; ambas compilan, ambas están documentadas.

// Forma Rust (familia filter):
let users = User::query().filter("email", &email).get().await?;
let users = User::query().filter_op("age", ">=", 18).get().await?;
let users = User::query().filter_like("email", "%@example.com").get().await?;

// Forma Laravel (familia db_where / where_*):
let users = User::db_where("email", &email).get().await?;
let users = User::query().db_where_op("age", ">=", 18).get().await?;
let users = User::query().where_like("email", "%@example.com").get().await?;
```

### Variantes de where

Cada fila tiene dos formas equivalentes en Suprnova - forma Rust
(`filter*`) y forma Laravel (`db_where` / `where_*`). Ambas llaman a
la misma implementación canónica; ambas están etiquetadas con
`#[doc(alias = "...")]` para que la búsqueda de rustdoc encuentre
cualquiera de las dos.

| Laravel | Suprnova (forma Rust) | Suprnova (forma Laravel) | Notas |
|---------|----------------------|--------------------------|-------|
| `->where(col, val)` | `.filter(col, val)` | `.db_where(col, val)` | Igualdad |
| `->where(col, op, val)` | `.filter_op(col, op, val)` | `.db_where_op(col, op, val)` | Operador arbitrario |
| `->orWhere(...)` | `.or_filter(...)` | `.or_where(...)` | |
| `->orWhereKey(id)` | `.or_filter_key(id)` | `.or_where_key(id)` | Filtro de PK como disyunto |
| `->orWhereKeyNot(id)` | `.or_filter_key_not(id)` | `.or_where_key_not(id)` | Filtro de PK negado como disyunto |
| `->whereNot(col, val)` | `.filter_not(col, val)` | `.where_not(col, val)` | |
| `->whereIn(col, vals)` | `.filter_in(col, vals)` | `.where_in(col, vals)` | |
| `->whereNotIn(col, vals)` | `.filter_not_in(col, vals)` | `.where_not_in(col, vals)` | |
| `->whereBetween(col, [a, b])` | `.filter_between(col, a..=b)` | `.where_between(col, a..=b)` | Rango de Rust |
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
| `->whereBinary(col, val)` | `.filter_binary(col, val)` | `.where_binary(col, val)` | Byte a byte; solo MySQL y MariaDB |
| `->orWhereBinary(col, val)` | `.or_filter_binary(col, val)` | `.or_where_binary(col, val)` | |
| `->whereNotBinary(col, val)` | `.filter_not_binary(col, val)` | `.where_not_binary(col, val)` | |
| `->orWhereNotBinary(col, val)` | `.or_filter_not_binary(col, val)` | `.or_where_not_binary(col, val)` | |
| `->whereJsonContains(col, v)` | `.filter_json_contains(col, v)` | `.where_json_contains(col, v)` | Según el backend |
| `->whereJsonLength(col, op, n)` | `.filter_json_length(col, op, n)` | `.where_json_length(col, op, n)` | |
| `->whereColumn(a, b)` | `.filter_column(a, b)` | `.where_column(a, b)` | Comparación columna a columna |
| `->whereExists(closure)` | `.filter_exists(builder)` | `.where_exists(builder)` | Subconsulta |
| `->whereHas(rel, closure)` | `.filter_has(rel, fn)` | `.where_has(rel, fn)` | Predicado de relación (10B) |
| `->whereDoesntHave(rel)` | `.filter_doesnt_have(rel)` | `.where_doesnt_have(rel)` | (10B) |
| `->whereRelation(rel, col, op, v)` | `.filter_relation(...)` | `.where_relation(...)` | (10B) |
| `->whereRaw(sql, bindings)` | `.filter_raw(sql, bindings)` | `.where_raw(sql, bindings)` | |

La familia `binary` compara los bytes en crudo en lugar de casar bajo la
intercalación de la columna. MySQL y MariaDB emiten `col = binary ?`;
Postgres y SQLite no tienen un operador equivalente, así que en esos
backends un terminal devuelve un error al renderizar la sentencia en lugar
de recurrir a un `=` dependiente de la intercalación. Véase
[Comparación byte a byte](queries.md#byte-exact-comparison).

Los predicados en bruto vinculados usan marcadores `?` portables en
SQLite, MySQL y PostgreSQL:

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

En PostgreSQL, Suprnova reindexa esos marcadores después de los
bindings previos de la consulta, así que el ejemplo genera `$1` para
`active` y `$2`/`$3` para el predicado en bruto. Usa `??` para un
operador de signo de interrogación literal dentro de un fragmento en
bruto vinculado, como en `"payload ?? 'enabled' AND status = ?"`. Los
fragmentos `$N` existentes siguen aceptándose, pero los marcadores
portables evitan acoplar el sitio de la llamada a la posición dentro
de la consulta. Las mezclas de estilos de marcador y los desajustes
entre marcadores y bindings se rechazan antes de la E/S a la base de
datos. Como con cualquier expresión en bruto, el texto SQL debe ser
de confianza; los valores no confiables solo deben ir en el vector de
bindings.

### Orden

```php
$users = User::orderBy('name', 'asc')->get();
$users = User::orderByDesc('created_at')->get();
$users = User::latest()->get();        // atajo: orderBy(created_at, desc)
$users = User::oldest()->get();        // atajo: orderBy(created_at, asc)
$users = User::inRandomOrder()->get();
```

```rust
let users = User::query().order_by("name", Direction::Asc).get().await?;
let users = User::query().order_by_desc("created_at").get().await?;
let users = User::latest().get().await?;
let users = User::oldest().get().await?;
let users = User::query().in_random_order().get().await?;
```

`Direction::Asc` / `Direction::Desc` es el enum de Suprnova
reexportado desde SeaORM.

#### Ordenar por una secuencia explícita

`in_order_of` ordena las filas en el orden que enumeras. Todo aquello
cuyo valor no esté en la lista se ordena después de todo lo que sí lo
está.

```php
$users = User::inOrderOf('role', ['admin', 'member', 'guest'])->get();
```

```rust
let users = User::query()
    .in_order_of("role", ["admin", "member", "guest"])
    .get()
    .await?;
```

Suprnova lo renderiza como una expresión `CASE` con valores vinculados,
así que los valores son parámetros y es seguro tomarlos de los datos de
la solicitud:

```sql
ORDER BY CASE WHEN role = ? THEN 0 WHEN role = ? THEN 1 WHEN role = ? THEN 2 ELSE 3 END
```

El nombre de la columna es un identificador SQL, no un parámetro.
Escríbelo fijo o elígelo de una lista de permitidos, igual que cualquier
otro argumento de columna. Una lista de valores vacía no añade ordenación
alguna, así que puedes construir la secuencia de forma condicional sin
tratar aparte el caso vacío.

Para una columna que usa el cast `AsEnum<E>`, pasa cada variante por
`as_ref()`. Esa es la cadena exacta que almacena el cast:

```rust
let users = User::query()
    .in_order_of("role", [Role::Admin.as_ref(), Role::Member.as_ref()])
    .get()
    .await?;
```

`in_order_of` se publica en la superficie tipada `Builder<M>`. El builder
sin modelo `DB::table(...)` solo ordena por columna y dirección.

### Agrupación + having

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

### Límite / offset

```php
$users = User::limit(10)->offset(20)->get();
$users = User::take(10)->skip(20)->get();   // aliases
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

`get_raw()` devuelve el resultado con la forma de columna en bruto
para los casos de `select_raw` en los que las columnas no coinciden
con el esquema del modelo; `get()` devuelve `Vec<User>` y requiere
que las columnas seleccionadas completen el struct del modelo.

### Distinct

```rust
let emails: Vec<String> = User::query().distinct().pluck("email").await?;
```

### Agregados

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

Los agregados son genéricos sobre el tipo de retorno porque SeaORM
necesita saber a qué tipo forzar el escalar de la BD. Los valores por
defecto de tipo: `count -> i64`; `sum`/`avg` llevan un parámetro de
tipo explícito. Suprnova asigna alias internamente a las expresiones
de agregado generadas, de modo que el mismo resultado tipado se
decodifique en PostgreSQL, MySQL y SQLite. `sum` y `avg` devuelven
cero para un conjunto de coincidencias vacío, mientras que `min` y
`max` devuelven `None`. Un tipo de Rust solicitado incompatible o una
columna de resultado ausente es un error de base de datos; nunca se
convierte en un cero o `None` plausible.

### Terminales

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

`to_sql` devuelve el SQL parametrizado que emitiría el siguiente
terminal - útil para depurar o construir vistas. Los bindings son
accesibles mediante `.to_sql_with_bindings() -> (String, Vec<Value>)`.

`model_keys` es el terminal que solo obtiene claves: proyecta la clave
primaria **calificada** (`users.id`) y nunca hidrata un modelo, por lo
que preguntar «¿qué filas coincidieron?» cuesta una columna en lugar de
una fila completa por coincidencia. La calificación permite que siga
funcionando cuando una consulta hace JOIN con otra tabla que tiene su
propio `id`. Cualquier `select(...)` ya presente en el constructor se
descarta: la persona que llama pidió claves.

### Uniones

```rust
let first  = User::filter("active", true);
let second = User::filter("role", "admin");
let users  = first.union(second).get().await?;
let users  = first.union_all(second).get().await?;
```

## Bloqueo de filas

Dos métodos del builder solicitan un bloqueo de base de datos por
fila en el momento del SELECT:

```rust
// Bloqueo de escritura exclusivo - bloquea a otras transacciones
// que intenten bloquear o escribir las mismas filas hasta que esta
// transacción haga commit.
let order = Order::query()
    .filter("id", 42)
    .lock_for_update()
    .first_or_fail()
    .await?;

// Bloqueo de lectura compartido - permite otros lectores
// compartidos, bloquea a los escritores.
let inventory = Inventory::query()
    .filter("sku", sku)
    .shared_lock()
    .first_or_fail()
    .await?;
```

SQL emitido por backend:

| Backend  | `lock_for_update()` | `shared_lock()`        |
|----------|---------------------|------------------------|
| Postgres | `FOR UPDATE`        | `FOR SHARE`            |
| MySQL    | `FOR UPDATE`        | `LOCK IN SHARE MODE`   |
| SQLite   | (sin SQL, ver más abajo) | (sin SQL, ver más abajo)    |

La cláusula de bloqueo se añade al final absoluto de la
declaración compuesta - después de cada rama `UNION`, cada
`ORDER BY`, cada `LIMIT` / `OFFSET`. Un `union(...)` de dos
builders seguido de `.lock_for_update()` emite exactamente **un**
`FOR UPDATE` en el ámbito exterior, no uno por rama.

Para recargar un modelo que ya tienes en la mano y tomar el bloqueo en
la misma sentencia, usa `refresh_for_update`:

```rust
DB::transaction(|tx| async move {
    let mut order = Order::find_or_fail(42).await?;
    order.refresh_for_update().await?;   // SELECT ... WHERE id = ? FOR UPDATE
    order.status = "processed".into();
    order.save_with_tx(&tx).await?;
    Ok(())
}).await?;
```

### Uso dentro de una transacción

El bloqueo solo hace un trabajo útil **dentro de una transacción** -
sin ella, el SQL se sigue emitiendo, pero el bloqueo se libera al
terminar la declaración. Combínalo con `DB::transaction(...)`:

```rust
DB::transaction(|tx| async move {
    let order = Order::query()
        .filter("id", 42)
        .lock_for_update()
        .first_or_fail()
        .with_tx(&tx)
        .await?;
    // Otras transacciones que intenten bloquear id=42 se bloquean
    // aquí hasta el commit.
    order.status = "processed".into();
    order.save_with_tx(&tx).await?;
    Ok(())
}).await?;
```

### `lock_for_update` frente a `shared_lock`

La mayoría de los flujos de "leer y luego escribir" quieren
`lock_for_update`. Un bloqueo compartido todavía permite que otro
lector con `shared_lock` te adelante en un `UPDATE` posterior - solo
`FOR UPDATE` es mutuamente excluyente.

`shared_lock` es correcto para lecturas de instantánea consistentes
en las que lees una fila, derivas una decisión de ella, y no
escribes de vuelta - por ejemplo, una comprobación de inventario que
en sí misma no decrementa el stock.

### SQLite

SQLite no tiene bloqueo a nivel de fila. Solo usa bloqueo de
transacción a nivel de archivo (`BEGIN IMMEDIATE` / `BEGIN
EXCLUSIVE`). Los métodos de bloqueo se **mantienen** en la ruta de
SQLite para que el código multi-backend compile, pero no emiten
ningún SQL.

La primera vez por proceso que `lock_for_update` / `shared_lock` se
ejecuta contra un backend SQLite, el framework registra un único
`warn!` en el target de tracing `suprnova::eloquent::lock`. Esto
hace visible la operación nula sin saturar las rutas de código de
alto volumen.

Si necesitas garantías de contención entre filas en SQLite, envuelve
la sección crítica en una transacción `BEGIN IMMEDIATE` explícita -
a nivel de archivo eso bloquea a cualquier otro escritor.

### Qué no está en v1

- **`NOWAIT` / `SKIP LOCKED`** - útiles para flujos de reclamo de
  colas de trabajos, pero añaden superficie de API. Se posponen
  hasta que un consumidor real los necesite.

## Transacciones

Suprnova incluye tres puntos de entrada para las transacciones de
base de datos, más el rollback anidado mediante savepoints. Dos de
ellos - la forma con closure y el helper de reintento ante
deadlock - instalan un contexto ambiental para que las operaciones
de modelo dentro del closure se enruten automáticamente a través de
la transacción, sin que quien llama tenga que pasar un handle por
cada sitio de llamada.

### Forma con closure - `DB::transaction`

La forma con closure es el caso común. El closure recibe una
`&Transaction` que puede usar para marcar un punto de control con
`savepoint(name)`; cada operación `Model::*` / `Builder::*` dentro
del closure se enruta automáticamente a través de la transacción
mediante un `tokio::task_local!` llamado `CURRENT_TX`.

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

- El closure devuelve `Ok` → **commit**.
- El closure devuelve `Err` → **rollback** (el error original se
  propaga).
- El closure entra en pánico → rollback (la transacción en curso se
  descarta durante el unwind; el `drop` de `DatabaseTransaction` de
  SeaORM hace rollback).

Las lecturas dentro del closure ven las escrituras de la misma
transacción (mediante una consulta a `CURRENT_TX` en cada llamada
SQL hoja). La primera llamada a `DB::transaction` tras arrancar el
proceso toma el backend de base de datos de `DB::connection()`; las
llamadas siguientes reutilizan el mismo registro de conexiones.

La firma usa un higher-ranked trait bound + `Pin<Box<dyn Future>>`
para que los closures puedan tomar `tx` en préstamo a través de
puntos `.await`:

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        // ... trabajo previo al savepoint ...
        tx.savepoint("inner").await?;
        // ... trabajo interno ...
        if some_condition {
            tx.rollback_to("inner").await?;
        }
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

La forma `Box::pin(async move { ... })` es el costo de permitir que
el future use `&tx` después de un `.await` - sin ella, el lifetime
del préstamo no puede escapar del cuerpo del closure. Refleja la
firma de `TransactionTrait::transaction` de SeaORM.

### Puntos de guardado - `tx.savepoint(name)` / `tx.rollback_to(name)`

Los puntos de guardado marcan la transacción para poder descartar un
bloque de trabajo interno sin abortar el commit exterior. Funciona
en los tres backends - el `SAVEPOINT` de SQLite es completamente
funcional aunque SQLite no tenga bloqueo a nivel de fila.

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        let mut account = Account::query().filter("id", id).first_or_fail().await?;
        account.balance = 200;
        account.save().await?;     // se confirma cuando la tx exterior hace commit

        tx.savepoint("audit_trail").await?;

        let entry = AuditEntry::create(attrs! { actor_id: actor, ... }).await?;
        if audit_validation_failed(&entry) {
            tx.rollback_to("audit_trail").await?;
            // la fila de audit_trail desaparece; la actualización de account sigue pendiente de commit
        }

        Ok::<(), FrameworkError>(())
    })
}).await?;
```

El nombre del savepoint se interpola literalmente en el SQL - usa
un identificador estático, **no** insertes entrada del usuario.

### Un `DB::transaction` anidado se rechaza en tiempo de ejecución

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

El `DatabaseConnection::begin()` de SeaORM no compone - llamarlo
sobre una conexión que ya mantiene una transacción abierta inicia
una transacción física completamente nueva, que hace commit o
rollback de forma independiente del ámbito exterior. Eso es una
trampa silenciosa para la integridad de los datos, así que
`DB::transaction` comprueba `CURRENT_TX` de antemano y devuelve un
error de base de datos en lugar de producir la semántica incorrecta.
Usa `tx.savepoint(name)` para un comportamiento anidado.

### Reintento ante deadlock - `DB::transaction_with_attempts`

Las lecturas `SERIALIZABLE` de Postgres y los bloqueos a nivel de
fila de MySQL pueden generar errores de fallo de serialización /
deadlock que se resuelven reintentando la transacción.
`transaction_with_attempts` ejecuta el closure desde cero cada vez,
hasta `attempts`:

```rust
DB::transaction_with_attempts(3, |_tx| {
    Box::pin(async move {
        // Lógica aislada en SERIALIZABLE que puede competir con una
        // tx concurrente y mostrar el SQLSTATE 40001 / 40P01 en el commit.
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

La detección se hace mediante una subcadena del string `Display`
contra el error interno:

- SQLSTATE `40001` de Postgres (serialization_failure)
- SQLSTATE `40P01` de Postgres (deadlock_detected)
- Subcadena `"deadlock"` sin distinguir mayúsculas/minúsculas
  (cubre el `Deadlock found when trying to get lock` de MySQL y
  cualquier string de deadlock mostrado al usuario)

En el último intento, el error se propaga sin cambios. El closure
se ejecuta desde cero en cada intento - captura estado propio (owned)
o `Arc`s en lugar de referencias `&mut`, para que la ruta de
reintento esté bien definida.

> **Advertencia:** como la detección incluye una subcadena
> `"deadlock"` sin distinguir mayúsculas/minúsculas (necesaria para
> MySQL, cuyo driver no expone un SQLSTATE), cualquier error interno
> cuyo `Display` contenga esa palabra dispara un reintento. Al
> lanzar tus propios errores desde dentro de un closure de
> `transaction_with_attempts`, evita la palabra "deadlock" en el
> mensaje - de lo contrario, un error de validación no relacionado
> se reintentará hasta `attempts` veces antes de propagarse. Las
> coincidencias de SQLSTATE de Postgres (`40001` / `40P01`) son la
> señal fiable; la heurística es solo para MySQL.

### Forma manual - `DB::begin_transaction` + shims `*_with_tx`

Cuando el lifetime de la transacción no encaja en un closure (por
ejemplo, abarca varias ramas de control de flujo), abre una
`Transaction` manual y haz que cada operación se sume a ella
explícitamente:

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

tx.commit().await?;  // o tx.rollback().await?;
```

El modo manual **no** instala `CURRENT_TX`. Enruta cada operación
individual a través de la transacción con `Builder::with_tx(&tx)` o
con los shims `Model::*_with_tx(&tx, ...)`:

| Método del trait    | Variante manual                           |
|---------------------|-------------------------------------------|
| `Model::create`     | `Model::create_with_tx(&tx, attrs)`       |
| `Model::save`       | `Model::save_with_tx(&tx)`                |
| `Model::update`     | `Model::update_with_tx(&tx, attrs)`       |
| `Model::delete`     | `Model::delete_with_tx(&tx)`              |
| `Model::force_delete` | `Model::force_delete_with_tx(&tx)`      |
| `Builder::*`        | `Builder::with_tx(&tx).*`                 |

Mantener una `Transaction` fija una conexión del pool durante toda
la vida del handle. En SQLite el pool tiene una sola conexión, así
que cualquier lectura paralela no transaccional contra la misma base
de datos se bloquea hasta que la transacción termina - **carga
cualquier fila previa ANTES de `DB::begin_transaction()`** y enruta
cada escritura dependiente a través de la `tx` devuelta.

`Transaction::commit` / `Transaction::rollback` consumen el handle y
requieren un `Arc::try_unwrap` de la transacción interna de SeaORM;
si algún clon de `TxHandle` (de `tx.handle()` /
`Builder::with_tx(&tx)`) sigue vivo en el momento del commit /
rollback, ambos fallan con un error "TxHandle clones still alive".
La solución correcta es soltar tu `Builder<M>` / los handles
pendientes antes de llamar a `commit` - el framework se niega a
arriesgarse a una escritura a medio confirmar frente a otro escritor
paralelo que sostenga la misma tx.

### Precedencia

Precedencia de tres niveles para enrutar una operación a través de
una conexión:

1. **Override a nivel de builder** - `Builder::with_tx(&tx)` o
   cualquier shim `Model::*_with_tx(&tx, ...)`. Lo explícito le gana
   a lo ambiental.
2. **`CURRENT_TX` ambiental** - instalado por `DB::transaction` /
   `DB::transaction_with_attempts` para el ámbito de tarea del
   closure.
3. **Fallback al pool** - `DB::connection()` devuelve el singleton
   global `DbConnection`.

Dentro de `DB::transaction(|tx| ...)`, llamar a
`Builder::with_tx(&other_tx)` enruta explícitamente esa única
consulta a través de `other_tx` - pasando por encima del
`CURRENT_TX` ambiental. Eso es casi con certeza un error; la ruta de
override existe para la forma manual, no para anular la propia tx
del closure.

### `with_tx` y los scopes globales

Un builder que lleva un `tx_override` sigue respetando los scopes
globales, los scopes con nombre, y el plan de carga anticipada - el
override solo cambia el enrutamiento de la conexión, no el SQL.

### Limitaciones (v1)

- **Carga anticipada de relaciones** - `Builder::with(["posts"])` y
  `Collection::load(["posts"])` enrutan las subconsultas
  anticipadas `IN (...)` a través de `DB::connection()`, no a
  través de la transacción activa. Las escrituras pendientes dentro
  de un closure `DB::transaction` **no** son visibles para las
  relaciones cargadas mediante `.with(...)`. Por ahora, limita el
  trabajo transaccional a llamadas directas `Model::*` /
  `Builder::*` / `DB::table(...)`; posterga las cargas de relación
  hasta después de que la escritura exterior se asiente (o antes de
  `DB::begin_transaction` en la ruta manual). Esta es una costura
  conocida - el helper de enrutamiento (`ExecutorChoice`) ya está en
  su sitio en cada hoja SQL; el bloqueo es que
  `EagerLoadDispatch::eager_load` toma `&DatabaseConnection`
  (concreto), que la macro emite para cada tipo de relación. Un
  barrido posterior adaptará el trait al helper de despacho.
- **DDL en Postgres** - `DB::statement(...)` dentro de una
  transacción ejecuta el DDL contra la conexión de la tx, lo cual
  Postgres permite; MySQL hace commit implícito y por tanto no está
  soportado dentro de una transacción de Suprnova (esto coincide con
  la advertencia de `DB::transaction` de Laravel).

## Scopes

Suprnova incluye dos tipos de scope, reflejando a Laravel:

- **Scopes locales** - métodos de extensión sobre el builder,
  declarados por modelo con `#[suprnova::scopes(Model)]`. Cada
  función libre dentro del bloque `impl` anotado se convierte tanto
  en `Model::name()` (un iniciador estático) como en
  `Builder::name()` (un método encadenable).
- **Scopes globales** - implementaciones de `GlobalScope<M>`
  registradas al arrancar mediante
  `ScopeRegistry::register::<M, _>(scope)`. Cada llamada a
  `Model::query()` los superpone automáticamente.

### Scopes locales

Declara los scopes locales dándoles la forma
`fn(query: Builder<Self>, args...) -> Builder<Self>`:

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

// Se usa como iniciador o como método encadenable:
let active_users  = User::active().get().await?;
let popular_users = User::query().active().popular(500).get().await?;
```

Los métodos que no son scopes declarados en el mismo bloque `impl`
(cualquiera cuyo primer parámetro no sea `query: Builder<Self>`)
pasan sin cambios.

### Scopes globales

Los scopes globales se aplican en cada llamada a `Model::query()`.
El caso de uso clásico es la multi-tenencia - cada lectura queda
acotada al tenant actual sin que cada llamador tenga que pasar el
filtro a mano.

```rust
use suprnova::eloquent::scopes::{GlobalScope, ScopeRegistry};

pub struct TenantScope;

impl GlobalScope<Article> for TenantScope {
    fn apply(&self, query: Builder<Article>) -> Builder<Article> {
        // Lee el tenant actual de un task-local /
        // AtomicI64 / donde sea que viva el estado por solicitud.
        query.filter("tenant_id", current_tenant_id())
    }
}

// Al arrancar - normalmente dentro de tu módulo de provider/bootstrap:
ScopeRegistry::register::<Article, _>(TenantScope);

// Cada lectura queda acotada automáticamente al tenant activo:
let scoped = Article::query().get().await?;
```

Varios scopes por modelo se componen en el orden de registro - el
primero en registrarse se ejecuta primero, así que sus cláusulas de
filtro aparecen primero en la cadena WHERE. Los filtros combinados
con AND no dependen del orden, pero de izquierda a derecha sí
importa para cualquier cláusula cuyo orden de efecto secundario sea
visible (por ejemplo, ordenación, having, fragmentos en bruto).

### Excluir un scope global

Cada modelo que toca la macro `#[suprnova::model]` recibe dos
helpers estáticos emitidos sobre él:

```rust
// Omite exactamente un scope registrado, por tipo. Los demás scopes siguen aplicando.
let all_tenants = Article::without_global_scope::<TenantScope>().get().await?;

// Omite todos los scopes registrados. Patrón de herramientas de administración.
let everything = Article::without_global_scopes().get().await?;
```

**Importante:** los helpers de exclusión deben ser el punto de
entrada. Encadenar `.without_global_scope::<S>()` sobre un builder
que ya devolvió `Model::query()` no deshace los scopes que ya se
ejecutaron - `Model::query()` aplica los scopes de forma anticipada
en el momento de la construcción, así que la máscara se fija
demasiado tarde. Usa los helpers estáticos por modelo (arriba) para
la semántica correcta.

### Dónde se aplican los scopes globales

| Ruta | ¿Se aplican los scopes globales? |
|------|----------------------|
| `Model::query()` | Sí - el punto de entrada acotado canónico |
| `Model::without_global_scope::<S>()` | Sí, menos `S` |
| `Model::without_global_scopes()` | No |
| `Model::find(id)` | No - la búsqueda por PK va directa a través de SeaORM |
| `Model::find_many([...])` | No - misma razón |
| `Model::all()` | No - misma razón |

Esto refleja a Laravel: `Eloquent\Model::find` no dispara
`addGlobalScopes`. Quien llame y quiera búsquedas por PK acotadas
usa `Self::query().filter("id", pk).first().await?`.

### Las eliminaciones suaves y los scopes globales coexisten

`#[suprnova::model(soft_deletes)]` instala el filtro
`deleted_at IS NULL` mediante un mecanismo de etiqueta de string
separado, no a través del registro de scopes tipado. Las dos capas
se componen:

- `Model::query()` filtra las filas descartadas Y ejecuta cada
  scope registrado.
- `Model::without_global_scopes()` descarta los scopes registrados
  pero conserva el filtro de eliminación suave - las herramientas de
  administración que quieren leer todo el conjunto de columnas
  siguen excluyendo las filas descartadas por defecto.
- `Model::with_trashed()` y `Model::only_trashed()` omiten el
  filtrado de eliminación suave y también evitan el registro
  (construyen un builder nuevo sin acotar). Combínalo con
  `.without_global_scope::<S>()` si necesitas lecturas conscientes
  del scope sobre filas descartadas.

## Relaciones

Suprnova incluye cada variante de relación de Eloquent. Se declaran
en el bloque `relations = { ... }` de `#[suprnova::model]`, y la
macro emite - por cada relación declarada - un método en el struct,
un accesor de carga (`<name>_loaded()`), un accesor de conteo
(`<name>_count()`), y el brazo del dispatcher al que llama el
cargador anticipado. Esta sección cubre la forma por tipo y la tabla
de opciones; la inmersión profunda en la resolución de claves de
join, el registro morph, las filas pivot, y la reducción del enum
polimórfico vive en
[Relaciones de Eloquent](eloquent-relationships.md). Los tipos de
relación disponibles hoy:

| Tipo                | Uno/muchos | Entre familias | Respaldado por |
|---------------------|----------|-----------------|-----------|
| `HasOne<R>`         | uno      | no              | consulta `IN` sobre `<parent>_id` |
| `BelongsTo<R>`      | uno      | no              | consulta `IN` sobre la FK de esta fila |
| `HasMany<R>`        | muchos   | no              | igual que `HasOne`, devuelve `Vec<R>` |
| `BelongsToMany<R, P>` | muchos | no              | tabla pivot `P`, INNER JOIN + `pivot::<P>()` |
| `HasOneThrough<B, R>`  | uno   | no              | JOIN de dos consultas `parent → B → R` |
| `HasManyThrough<B, R>` | muchos | no             | igual que arriba, devuelve `Vec<R>` |
| `MorphOne<R>`       | uno      | sí              | filtro `IN` + `<name>_type = "<self>"` |
| `MorphMany<R>`      | muchos   | sí              | igual que `MorphOne`, devuelve `Vec<R>` |
| `MorphTo`           | uno      | sí (hijos → muchas familias) | enum por familia emitido en el sitio de declaración |
| `MorphToMany<R, P>` | muchos   | sí              | pivot m2m polimórfico `P` |
| `MorphedByMany<R, P>` | muchos | sí (inversa)   | mismo pivot, explorado en el otro sentido |

### La sintaxis `relations = { ... }`

Cada declaración de relación lleva la misma forma exterior: el
nombre de la relación, el tipo, el tipo relacionado (y los tipos
pivot/intermedios cuando corresponda), y un bloque `{ ... }` de
opciones.

```rust
use suprnova::model;

#[model(
    table = "users",
    relations = {
        // HasMany<R>
        posts: HasMany<crate::models::Post> {
            fk = "author_id",         // sobrescribe el valor por defecto `user_id`
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

Opciones comunes:

| Opción                     | Tipos de relación             | Propósito |
|----------------------------|-------------------------------|---------|
| `fk = "..."`               | cada tipo con una FK en el hijo | Columna del HIJO que apunta al padre. Por defecto = `<snake(parent_struct)>_id`. |
| `lk = "..."`               | tipos uno/muchos               | Columna del PADRE usada como clave de join. Por defecto = `"id"`. |
| `related_key = "..."`      | `BelongsToMany`, `MorphToMany` | El nombre de la COLUMNA de la PK en el lado relacionado. Por defecto = `"id"`. Necesario cuando el modelo relacionado usa una PK que no es `id`. |
| `with_pivot = ["...", ...]` | `BelongsToMany`, `MorphToMany` | Columnas extra del pivot que se exponen en el join. |
| `with_timestamps`          | `BelongsToMany`, `MorphToMany` | Sella `created_at` / `updated_at` al hacer attach/sync. |
| `with_default = \|\| { ... }` | `BelongsTo`                 | Closure que produce un valor por defecto cuando la FK es null O falta el padre. |
| `first_key`, `second_key`, `second_local_key` | `HasOneThrough`, `HasManyThrough` | Sobrescrituras de la clave de JOIN - ver la sección Through más abajo. |
| `name = "..."`             | cada tipo morph              | Nombre de la familia morph (por ejemplo, `"commentable"`, `"taggable"`). Determina las columnas `<name>_id` / `<name>_type` del hijo/pivot. |
| `targets = [T1, T2, ...]`  | `MorphTo`                     | La lista de destinos morph concretos. La macro emite un enum `<Name>Morph` en el sitio de declaración, con una variante por destino más `Unknown(String, i64)`. |
| `target_morph_type = "..."` | `MorphedByMany`              | El string de tipo morph que identifica la familia destino en el pivot. |
| `pivot_table`, `pivot_foreign_key`, `pivot_related_key` | `BelongsToMany`, `MorphToMany` | Sobrescrituras de columna / tabla del lado pivot cuando los valores por defecto no encajan. |

### `HasOne<R>` y `BelongsTo<R>`

Uno a uno en ambas direcciones. `HasOne` vive en el lado del padre y
llama a `R::query().filter(<fk>, <self.id>).first()`. `BelongsTo`
vive en el lado del hijo y lee la FK de `self`, y luego llama a
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

`BelongsTo` admite `with_default = || R { ... }`, que se dispara
cuando la FK es null O cuando falta la fila del padre. El closure
por defecto se ejecuta por llamada (y por fila cargada
anticipadamente) - perfecto para un sustituto vacío cuando un
usuario eliminado todavía tiene comentarios:

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
// Siempre es Some - el valor por defecto se dispara cuando falta la fila del usuario.
let author = c.author().first().await?.unwrap();
```

### `HasMany<R>`

Uno a muchos en el lado del padre. Devuelve un builder fluido;
encadena filter / order / latest / take / get / count y termina.

```rust
#[model(table = "users", relations = {
    posts: HasMany<crate::models::Post> {
        fk = "author_id",
    },
})]
pub struct User { /* ... */ }

let u = User::find(1).await?.unwrap();

// Cada post de este usuario, con el orden por defecto:
let posts: Vec<Post> = u.posts().get().await?;

// Filtrado + ordenado + paginado:
let recent = u.posts()
    .filter("published", true)
    .latest()                          // ORDER BY created_at DESC
    .take(10)
    .get()
    .await?;

// Solo COUNT - sin obtener filas:
let total: i64 = u.posts().count().await?;
```

Métodos terminales disponibles: `.first()`, `.get()`, `.count()`.
Filtros encadenables disponibles: `.filter` / `.db_where`,
`.filter_in` / `.where_in`, `.order_by`, `.latest`, `.oldest`,
`.limit`, `.take`.

### `BelongsToMany<R, P>` - Pivot de primera clase

Muchos a muchos a través de un pivot declarado con
`#[suprnova::model]`. El pivot es un modelo de primera clase con su
propia identidad de fila - no una tupla, no un hash map oculto. Dos
beneficios clave frente a la forma de pivot anónimo de Laravel:

1. La fila pivot es type-safe. Lee las columnas de `with_pivot` a
   través de `r.pivot::<P>().<column>`, nunca mediante
   `r.pivot.get("...")`.
2. El modelo pivot es accesible desde el resto del framework
   (factories, scopes, casts, hooks) igual que cualquier otro
   modelo.

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

// Mutadores de attach + sync
u.roles().attach(admin.id).await?;
u.roles().attach_with(admin.id, attrs! { assigned_at: chrono::Utc::now() }).await?;
u.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
u.roles().detach(admin.id).await?;

// Lee los datos del pivot a través del accesor de downcast por fila:
let roles = u.roles().get().await?;
for r in &roles {
    let p: &RoleUser = r.pivot::<RoleUser>();
    println!("user {} got role {} at {:?}", p.user_id, p.role_id, p.assigned_at);
}
```

- `.attach(id)` - INSERT de una única fila pivot. Falla ante un
  duplicado a menos que tu pivot lo permita (el framework no
  deduplica en la capa de Rust; usa `.sync` para idempotencia).
- `.attach_with(id, attrs! { ... })` - INSERT con columnas de pivot
  extra. Sella los timestamps cuando `with_timestamps` está activo.
- `.detach(id)` - DELETE de la(s) fila(s) pivot que enlazan padre →
  id.
- `.sync([ids...])` - diff-and-apply: hace attach de lo nuevo,
  detach de lo que falta, y deja la intersección intacta. Envuelto
  en una transacción.

`.get()` devuelve `Vec<R>` con el pivot sellado en el campo interno
`__pivot` de cada fila. El accesor `.pivot::<P>()` hace un downcast
del `Arc<dyn Any>` al tipo pivot que declaraste. Llamarlo con el
tipo equivocado entra en pánico - haz que el tipo coincida con el
pivot declarado.

### `HasOneThrough<B, R>` y `HasManyThrough<B, R>`

Alcanza un destino final `R` a través de un intermedio `B`. Útil
cuando la relación atraviesa dos tablas pero no necesitas exponer el
intermedio (`A → B → R`).

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

El dispatcher infiere las claves de JOIN a partir de los nombres de
struct. Sobrescrituras:

| Opción              | Por defecto                       | Descripción |
|---------------------|----------------------------------|-------------|
| `first_key`         | `<snake(parent_struct)>_id`      | Columna del intermedio `B` que apunta al padre `A`. |
| `second_key`        | `<snake(intermediate_struct)>_id` | Columna del `R` final que apunta al intermedio `B`. |
| `second_local_key`  | `"id"`                           | Columna del intermedio `B` con la que coincide `second_key`. Necesaria cuando `B` usa una PK que no es `id`. |

La columna de clave primaria del padre se lee de la declaración
`primary_key` del modelo (por defecto `"id"`) - no existe una
sobrescritura `local_key` en `HasManyThrough` / `HasOneThrough`;
cambia la PK del padre mediante el atributo `#[suprnova::model]` si
necesitas una clave de padre que no sea `id`.

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<crate::models::User, crate::models::Post> {
        first_key = "country_id",
        second_key = "author_id",
    },
})]
pub struct Country { /* ... */ }
```

### `MorphTo` con `targets = [...]` y el enum por familia

Las relaciones polimórficas apuntan una fila hija a una de varias
familias de padres. El hijo lleva un par `(<name>_id, <name>_type)`;
la columna `*_type` guarda el string de tipo morph que declara cada
padre.

`MorphTo` vive en el hijo. Su declaración lista cada familia de
padre a la que puede apuntar mediante `targets = [...]`. La macro
emite un enum por familia llamado `<RelationName>Morph` (que
coincide con la forma PascalCase del nombre de la relación, con el
sufijo `Morph`) con una variante por tipo destino, más
`Unknown(String, i64)` para las filas heredadas cuyo valor
`<name>_type` no coincide con ningún destino registrado.

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
    // Filas heredadas / colgantes - `<name>_type` no coincide con
    // ningún destino, O el morph_type coincidió pero la fila en
    // `<name>_id` ya no existe.
    CommentableMorph::Unknown(ty, id) => {
        eprintln!("comment {} points at unknown {ty}#{id}", c.id);
    }
}
```

El atributo `morph_type = "..."` en cada struct destino es lo que el
loader escribe en la columna `<name>_type` del hijo al insertar, y
por lo que filtra al leer. Sin `morph_type`, el framework deriva el
string de tipo a partir de `to_snake(struct_name)`.

El despacho de `MorphTo` - cómo el enum por familia elige la
variante correcta - consulta el registro morph en tiempo de
ejecución (el inventario que rellena cada declaración
`#[suprnova::model(morph_type = "...")]`). Para cada destino
declarado, el helper de obtención busca el `TypeId` del destino, lee
el string `morph_type` registrado, y lo compara contra el valor
`<name>_type` guardado en la fila hija. Gana la primera coincidencia,
en el orden de declaración. Los destinos sin un atributo
`morph_type` explícito recurren a `to_snake(target_type_name)` - el
mismo valor por defecto que usan `MorphMany` / `MorphOne` en el lado
del padre para sellar el string de tipo al escribir, de modo que los
dos lados se mantienen alineados. Esto significa que los valores
`morph_type` personalizados (por ejemplo, `morph_type = "blog_post"`
en un struct llamado `Post`, o cualquier string no convencional)
despachan correctamente sin cambios en el sitio de declaración.

### `MorphOne<R>` y `MorphMany<R>` - lado del padre

La dirección inversa de `MorphTo`: un tipo padre declara el
uno-o-muchos polimórfico que posee. `MorphOne` devuelve `Option<R>`
desde `.first()`; `MorphMany` devuelve `Vec<R>` desde `.get()`.
Ambos filtran el par `(<name>_id, <name>_type)` del hijo por
`self.id` y el `morph_type` del padre.

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
// post.comments() solo devuelve filas con `commentable_type = "post"`;
// video.comments() solo devuelve filas con `commentable_type = "video"`.
```

La misma superficie encadenable que `HasMany` / `HasOne`: `.filter`
/ `.db_where`, `.order_by` / `.latest` / `.oldest`, `.limit` /
`.take`, `.first` / `.get` / `.count`.

### `MorphToMany<R, P>` y `MorphedByMany<R, P>`

Muchos a muchos polimórfico. El pivot compartido `P` lleva el par
de FK MÁS una columna discriminadora `<name>_type`. Un extremo
declara `MorphToMany` (por ejemplo, `Post.tags()`, `Video.tags()`),
el otro extremo declara un `MorphedByMany` por cada familia destino
(por ejemplo, `Tag.posts()`, `Tag.videos()`).

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

// Inversa: Tag declara un MorphedByMany por cada familia destino.
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

// `attach` / `attach_with` / `detach` / `sync` funcionan igual que
// en BelongsToMany. La columna `<name>_type` se rellena
// automáticamente a partir del `morph_type` del padre que llama.
post.tags().attach(tag.id).await?;
video.tags().attach(tag.id).await?;          // adjunción independiente
post.tags().sync([tag_a.id, tag_b.id]).await?;

// Dirección inversa - Tag se divide por familia:
let posts_with_tag:  Vec<Post>  = tag.posts().get().await?;   // tipado "post"
let videos_with_tag: Vec<Video> = tag.videos().get().await?;  // tipado "video"
```

El `target_morph_type` de `MorphedByMany` es necesario porque la
macro en el sitio de declaración de `Tag` no puede introspeccionar
el atributo `morph_type = "..."` del destino (vive en una
invocación `#[suprnova::model]` separada). Fijarlo explícitamente
mantiene honesto a cada brazo `MorphedByMany` sobre qué familia
explora.

### Vía de escape: métodos de relación escritos a mano

Las relaciones declaradas en `relations = { ... }` son las únicas
que conoce el dispatcher de carga anticipada (y `with`,
`with_count`, etc.). Si una relación es demasiado inusual para la
forma de la macro - por ejemplo una consulta que agrega a través de
dos pivots, o una vista tipada de una tabla de caché
desnormalizada - puedes omitirla de `relations = { ... }` y escribir
un impl inherente normal:

```rust
impl User {
    /// Posts que este usuario escribió O en los que está etiquetado.
    /// Cruza dos relaciones y por tanto no se puede expresar como una
    /// única declaración `relations = { ... }` - escrito a mano.
    pub async fn posts_touched(&self) -> Result<Vec<Post>, FrameworkError> {
        let authored: Vec<Post> = self.posts().get().await?;
        let tagged:   Vec<Post> = /* ...consulta personalizada... */;
        // ...combinar + deduplicar...
        Ok(/* ... */)
    }
}
```

Estos métodos pierden el soporte de carga anticipada -
`User::with(["posts_touched"])` fallará porque el dispatcher no
tiene un brazo para `posts_touched`. Las declaraciones dentro de la
macro siguen siendo la ruta que el framework sabe cargar
anticipadamente, contar, agregar, y filtrar por predicado.

### Restricciones de v1

Un puñado de cosas que la superficie v1 deja para después. Cada una
también está documentada en su sitio de declaración - reunidas aquí
para visibilidad:

- **Los IDs morph son solo `i64`.** `MorphTo::morph_id` está fijado
  a `i64`, así que cualquier modelo usado como destino `MorphTo`
  debe declarar una clave primaria `i64`, y la columna `<name>_id`
  de la tabla hija también debe ser `i64`. Las FK morph de tipo
  string / UUID-como-string son v2.
- **Sin carga anticipada anidada a través de `MorphTo`.** El enum
  por familia borra el tipo del hijo, así que una ruta con puntos
  como `with(["commentable.user"])` no puede recursar en cola - el
  dispatcher devuelve un error tipado. Resuelve por familia haciendo
  match sobre el enum y llamando a `with(["user"])` en cada variante
  individualmente.

## Carga anticipada

La carga anticipada evita las consultas N+1. En lugar de
`posts.len()` consultas para obtener los posts de cada usuario,
Suprnova emite UNA consulta por relación de nivel superior, sin
importar cuántas filas padre se carguen.

Se accede a la superficie completa - lista plana, rutas anidadas,
conteo, agregados, y cargas anticipadas filtradas por predicado - a
través de los helpers que `#[suprnova::model]` emite en cada modelo:

```rust
// Una sola relación:
let users = User::with(["posts"]).get().await?;
for u in &users {
    for p in u.posts_loaded() { /* ... */ }
}

// Varias relaciones:
let users = User::with(["posts", "profile"]).get().await?;

// Rutas anidadas - tres consultas (users + posts + comments), sin N+1:
let users = User::with(["posts.comments"]).get().await?;
let p1 = users[0].posts_loaded()[0];
let comments = p1.comments_loaded();

// El anidamiento más profundo funciona como se espera:
let users = User::with(["posts.comments.author"]).get().await?;

// Conteo junto a las filas padre:
let users = User::with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}

// Agregados - Sum / Avg / Min / Max sobre una columna de la
// relación. La lectura ergonómica es el accesor `<rel>_sum_of(col)`
// que emite la macro.
let users = User::with_sum(("posts", "views")).get().await?;
let sum: f64 = users[0]
    .posts_sum_of("views")
    .expect("with_sum populated the cache");

// Varios agregados sobre la misma relación se componen - la clave
// de caché es la forma ancha `<rel>_<kind>_<col>`, así que los
// distintos tipos y columnas no colisionan:
let users = User::with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .with_min(("posts", "id"))
    .get()
    .await?;
let u = &users[0];
let sum = u.posts_sum_of("views").unwrap();   // Some(_) - suma de views
let avg = u.posts_avg_of("views").unwrap();   // Some(_) - promedio de views
let min = u.posts_min_of("id").unwrap();      // Some(Some(_)) - grupo no vacío
let max = u.posts_max_of("id");               // None - no se llamó a with_max

// Filtra los hijos cargados anticipadamente. La macro emite un
// helper estático tipado `with_where_<rel>(closure)` por relación,
// para que el tipo del parámetro del closure se infiera - sin
// necesidad de escribir `Builder<Post>`:
let users = User::with_where_posts(|q| q.filter("published", true))
    .get()
    .await?;
// El `Builder<User>` devuelto se encadena con cualquier otro método
// del builder de la consulta base:
let users = User::with_where_posts(|q| q.filter("published", true))
    .filter("active", true)
    .get()
    .await?;
// La forma genérica sigue disponible - útil cuando el nombre de la
// relación se calcula en tiempo de ejecución - pero tendrás que
// nombrar el tipo destino en el closure:
let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// Cada u.posts_loaded() contiene solo posts publicados.
```

### Disposición de la caché

Las celdas de caché `__eager` por fila se indexan por:

- `<rel>` (solo el NOMBRE de la relación) para `with` y
  `with_count`.
- `<rel>_<kind>_<col>` (por ejemplo, `posts_sum_views`) para los
  cuatro tipos de agregado - `with_sum` / `with_avg` / `with_min` /
  `with_max`. Esta clave ancha permite que varios agregados sobre la
  misma relación coexistan en la misma fila sin sobrescribirse entre
  sí.

| Método                              | Clave de caché       | Tipo de celda      | Valor de grupo vacío |
|-------------------------------------|----------------------|-------------------|-------------------|
| `with(["posts"])`                   | `posts`              | `Vec<Post>`       | `Vec::new()`      |
| `with(["profile"])`                 | `profile`            | `Option<Profile>` | `None`            |
| `with_count(["posts"])`             | `posts`              | `u64`             | `0`               |
| `with_sum(("posts","views"))`       | `posts_sum_views`    | `f64`             | `0.0`             |
| `with_avg(("posts","views"))`       | `posts_avg_views`    | `f64`             | `0.0`             |
| `with_min(("posts","id"))`          | `posts_min_id`       | `Option<f64>`     | `None`            |
| `with_max(("posts","id"))`          | `posts_max_id`       | `Option<f64>`     | `None`            |

La macro emite accesores correspondientes en cada modelo:

- `<rel>_loaded()` - para relaciones de colección: `&[Post]` (entra
  en pánico si la relación no se cargó anticipadamente). Para
  relaciones de valor único: `Option<&Profile>`.
- `<rel>_count()` - `u64`. Entra en pánico si no se llamó a
  `with_count(["..."])`.
- `<rel>_sum_of(col)` / `<rel>_avg_of(col)` - devuelven
  `Option<f64>` (`None` si no se llamó al `with_sum` / `with_avg`
  correspondiente).
- `<rel>_min_of(col)` / `<rel>_max_of(col)` - devuelven
  `Option<Option<f64>>`: el `Option` exterior responde "¿se llamó a
  `with_min` / `with_max`?", el `Option` interior responde "¿el SQL
  devolvió NULL porque el grupo estaba vacío?".

Los accesores son la superficie ergonómica - lee a través de ellos
en lugar de entrar directamente en `__eager.get_aggregate::<T>(...)`.
Por debajo construyen la misma clave de caché mediante
`eloquent::relations::aggregate_cache_key`.

### Componer agregados sobre la misma relación

La clave de caché ancha significa que puedes apilar tantas llamadas
`with_*` sobre la misma relación en una consulta como quieras - sin
colisiones:

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

// Min/Max son doble Option porque SQL da NULL en min/max sobre un grupo vacío:
match u.posts_min_of("id") {
    None              => panic!("with_min not called"),
    Some(None)        => println!("no posts yet"),
    Some(Some(min))   => println!("smallest post id: {min}"),
}

// El accesor devuelve `None` cuando se omitió el `with_*` correspondiente:
assert!(u.posts_avg_of("score").is_none()); // nunca se llamó con col="score"
```

### Agregados y columnas INTEGER

Un SUM sobre una columna INTEGER llega a la caché como `f64`. Los
brazos del dispatcher primero intentan `try_get::<Option<f64>>`, y
si falla recurren a
`try_get::<Option<i64>>().map(|n| n as f64)`, para que los tipos
COUNT/SUM de SQLite, que preservan INTEGER, no se conviertan
silenciosamente en `0.0`. Se leen a través de los accesores emitidos
por la macro sin importar el tipo de columna de origen.

### Enrutamiento de predicados de `with_where`

`User::with_where_posts(|q| q.filter("published", true))` aplica un
closure al `Builder<Post>` interno ANTES de que se emita la consulta
`IN` de `filter_in(<fk>, parent_ids)`, así que solo las filas hijas
que coinciden llegan a la caché. La macro emite un helper estático
tipado `with_where_<rel>` por cada relación declarada, así que el
tipo del parámetro del closure se infiere de la firma del método.

La forma genérica
`with_where(("posts", |q: Builder<Post>| q.filter("published", true)))`
sigue disponible - útil cuando el nombre de la relación se calcula
en tiempo de ejecución, o cuando ya tienes un `Builder<User>` y
quieres adjuntar un predicado. Requiere nombrar el tipo destino en
el closure porque el predicado pasa por un `Box<dyn Any>` y Rust no
puede inferir el tipo solo a partir del nombre de la relación. (Las
reglas de orfandad de Rust prohíben que la macro añada un método
tipado directamente sobre `Builder<User>`, así que la forma
abreviada tipada solo se ofrece en el modelo -
`User::with_where_<rel>` - no como un método encadenable del
builder.)

Para los tipos polimórficos, el predicado se ejecuta contra la
consulta de la tabla relacionada - no contra el escaneo del pivot.

`with_where` está soportado en cada tipo de relación EXCEPTO
`MorphTo`. El enum por familia de MorphTo borra el tipo del hijo,
así que ningún `Builder<R>` único cubre todas las variantes. La
carga anticipada anidada a través de MorphTo tampoco está soportada
en v1 - `with(["commentable.user"])`, donde `commentable` es un
`MorphTo`, devuelve un error del dispatcher de carga anticipada
recursiva.

### `Collection::load` / `load_missing`

Cuando ya obtuviste las filas y quieres cargar relaciones
anticipadamente después del hecho:

```rust
use suprnova::Collection;

let mut users: Collection<User> = User::all().await?.into();
users.load(["posts.comments"]).await?;
```

`load_missing` es por fila: cada fila de la colección se particiona
de forma independiente. Las filas que ya tienen la relación nombrada
en caché se dejan intactas; las que no, cargan la relación. Refleja
la semántica de `$collection->loadMissing(...)` de Laravel.

Para rutas anidadas, la partición se repite en cada nivel. Dado
`load_missing(["posts.comments"])`:

- Las filas sin `posts` en caché obtienen la ruta COMPLETA
  cargada - `posts` más sus `comments`.
- Las filas CON `posts` ya en caché recursan dentro de los posts en
  caché y cargan `comments` solo en los posts que aún no tienen
  `comments` en caché.

La misma partición por fila se repite en cada segmento adicional de
una ruta con puntos más larga (`"posts.comments.author"`, etc.) - en
cada paso, solo las filas a las que falta ese segmento reciben la
carga masiva.

## Paginación

Tres tipos de paginador se componen sobre `Builder<M>`:

| Método | Devuelve | Consultas por página | Úsalo cuando |
|--------|---------|------------------|----------|
| `paginate(per_page)` | `LengthAwarePaginator<M>` | 2 (COUNT + LIMIT) | La UI necesita el total de páginas |
| `simple_paginate(per_page)` | `Paginator<M>` | 1 (LIMIT + 1) | Tablas grandes; solo un botón "Siguiente" |
| `cursor_paginate(per_page)` | `CursorPaginator<M>` | 1 (LIMIT + 1) | Scroll infinito; paginación profunda |

Los tres implementan `Serialize` con la forma JSON estándar de
Laravel, así que se envían directamente a Inertia / consumidores
JSON sin necesidad de remodelarlos.

### Con conteo total

```rust
use suprnova::LengthAwarePaginator;

let page: LengthAwarePaginator<User> = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;

// page.data: Vec<User>
// page.total: u64 - conteo total de filas en todas las páginas
// page.last_page: u64 - índice de la última página (base 1)
// page.current_page: u64
// page.per_page: u64
// page.from / page.to: Option<u64> - límites de la ventana (base 1)
// page.path: Option<String> - URL base opcional para generar enlaces
```

El análisis del parámetro de página lee `?page=N` de la solicitud
activa mediante `Context::query_param`. Para paginar varias listas
en la misma página con sus propias claves de query, usa
`paginate_using`:

```rust
let posts = Post::query().paginate_using("posts_page", 10).await?;
let comments = Comment::query().paginate_using("comments_page", 25).await?;
```

**Forma del JSON:**

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

`path` se omite del JSON cuando no está fijado.

### Paginación simple (sin conteo)

`paginate` siempre ejecuta dos consultas - un `COUNT(*)` más la
obtención de la página. En tablas grandes, el conteo por sí solo
puede dominar el tiempo de la solicitud. `simple_paginate` se salta
el conteo por completo; en su lugar obtiene `per_page + 1` filas e
informa si existe una página siguiente mediante el flag `has_more`:

```rust
use suprnova::Paginator;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;

// page.has_more: bool - ¿había una fila extra más allá de per_page?
// page.current_page, page.per_page, page.data, page.path: como arriba.
```

**Forma del JSON:**

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true
}
```

### Paginación por cursor (keyset)

La paginación por cursor es la elección para scroll infinito,
paginación profunda, o en cualquier sitio donde un orden de filas
estable con un seek de O(1) por página barato valga más que una UI
de páginas numéricas. Es bidireccional - lee el parámetro de query
`?cursor=<opaque>`, avanza o retrocede según la dirección del
cursor, y emite tanto `next_cursor` como `prev_cursor` según existan
los vecinos de la página (igual que `cursorPaginate()` de Laravel).

```rust
use suprnova::CursorPaginator;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;

// page.data: Vec<User>
// page.per_page: u64
// page.next_cursor: Option<String> - cursor opaco para la página siguiente (None en la última)
// page.prev_cursor: Option<String> - cursor opaco para la página anterior (None en la primera)
// page.path: Option<String>
```

Los cursores están **cifrados y autenticados** mediante
`CursorPaginator::encode_value` - codifican el límite del keyset (la
clave primaria del modelo) más una etiqueta de dirección, sellados
con AES-256-GCM usando el `APP_KEY` del framework. La manipulación
produce un error 400 ParamParse; el cursor es opaco para el cliente
y no se puede falsificar sin la clave.

La siguiente solicitud pasa el cursor a través de `?cursor=<opaque>`:

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

La paginación por cursor **reemplaza** cualquier `ORDER BY`
existente en el builder - se requiere un orden `ASC` estable por PK
para que `gt(boundary)` corte de forma determinista.

**Forma del JSON:**

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` y `prev_cursor` siempre están presentes como claves
JSON (emitidas como `null` cuando están ausentes), para que los
esquemas del cliente puedan confiar en la presencia del campo;
`path` se omite cuando no está fijado.

### Errores

| Condición | Variante | HTTP |
|-----------|---------|------|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| Cursor inválido (base64, JSON o HMAC incorrectos) | `FrameworkError::Internal` desde `Crypt::decrypt_string` | 500 |
| Fallo subyacente de la BD | `FrameworkError::Database` | 500 |

El fallo de autenticación del cursor emerge como `Internal` (no
`ParamParse`) para que un cursor manipulado no filtre información a
nivel de protocolo al cliente; el cuerpo de la respuesta sigue
llevando un motivo legible por humanos.

### Leer parámetros de query fuera de una solicitud real

Los tests, los comandos de consola, y los workers en segundo plano
no se ejecutan dentro de una solicitud de hyper - así que
`Context::query_param("page")` devuelve `None` y `paginate` recurre
a la página 1 por defecto. Los tests que necesiten ejercitar una
página concreta pueden instalar un override por hilo:

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

`test_set_query` / `test_clear_query` están protegidos detrás de la
feature `testing` (activada por defecto en `framework/Cargo.toml`),
así que los builds de release nunca ven esta superficie.

## Iteración por chunks y perezosa

Siete puntos de entrada de streaming en `Builder<M>` te permiten
procesar conjuntos de resultados grandes con memoria acotada. Elige
según el trade-off:

| Método | Paginación | ¿Seguro ante la concurrencia? | Devuelve |
|--------|-----------|------------------|---------|
| `chunk(n, async \|batch\| { ... })` | OFFSET | No | `Result<(), _>` |
| `chunk_by_id(n, async \|batch\| { ... })` | Cursor de PK | **Sí** | `Result<(), _>` |
| `chunk_map(n, async \|batch\| { ... })` | OFFSET | No | `Collection<U>` |
| `each(async \|row\| { ... })` | OFFSET, tamaño 1 | No | `Result<(), _>` |
| `lazy()` | Cursor de PK, batch de 1000 | **Sí** | `LazyCollection<M>` |
| `lazy_by_id(batch_size)` | Cursor de PK, batch personalizado | **Sí** | `LazyCollection<M>` |
| `cursor()` | Alias de `lazy()` | **Sí** | `LazyCollection<M>` |

### chunk - batches paginados por OFFSET

```rust
use suprnova::{Collection, Model};

User::query().chunk(100, |batch: Collection<User>| async move {
    for user in &batch {
        send_welcome_email(user).await?;
    }
    Ok(())
}).await?;
```

El closure recibe un `Collection<M>` por batch - el acceso con forma
de slice (`.iter()`, indexación) funciona directamente a través de
`Deref`.

`chunk` está paginado por OFFSET y **no es seguro ante inserciones
concurrentes**: las filas insertadas antes del offset del siguiente
batch se omiten; las filas eliminadas antes del offset se procesan
dos veces (lo que sea que se haya desplazado a su lugar). Usa
`chunk_by_id` para procesamiento masivo de nivel de producción
contra tablas bajo carga de escritura.

### chunk_by_id - batches por cursor de PK, seguro ante la concurrencia

```rust
User::query().chunk_by_id(500, |batch| async move {
    for user in &batch {
        reindex_user(user).await?;
    }
    Ok(())
}).await?;
```

Cada batch filtra con `WHERE id > last_id ORDER BY id ASC LIMIT n`,
así que las filas insertadas a mitad de la iteración con PKs por
encima del cursor caen en un batch posterior (o las recoge una
ejecución siguiente) - nunca provocan que una fila original se omita
o se duplique.

`chunk_by_id` requiere una clave primaria `i64`. Los modelos con
PKs `String` / `Uuid` usan `chunk` con la advertencia del OFFSET.
(Generalizar la forma del cursor a claves que no son `i64` está en
la lista de seguimiento.)

### chunk_map - chunk + map por chunk

```rust
let totals: Collection<i64> = Order::query()
    .chunk_map(1000, |batch| async move {
        let sum: i64 = batch.iter().map(|o| o.amount).sum();
        Ok(Collection::from_vec(vec![sum]))
    })
    .await?;
```

Mapea cada batch a través de `f`, concatena la salida mapeada, y
devuelve un único `Collection<U>`. Solo tiene memoria acotada cuando
`U` es estrictamente más pequeño que `M` - elige esto cuando estés
produciendo resúmenes (totales por batch, ids, agregados) en lugar
de filas transformadas.

### each - una fila a la vez, OFFSET

```rust
User::query().each(|user| async move {
    send_welcome_email(&user).await?;
    Ok(())
}).await?;
```

Azúcar sintáctico para `chunk(1, ...)` - una consulta por fila. Para
conjuntos de datos grandes, cambia a `lazy()`, que agrupa
internamente en batches (1000 filas por obtención por defecto) pero
sigue exponiendo una fila a la vez al consumidor.

### lazy / lazy_by_id / cursor - streams

```rust
let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

`lazy()` devuelve un `LazyCollection<M>` - un envoltorio de stream
`Send` que produce `Result<M, FrameworkError>` por fila. El
backpressure funciona de forma natural: un consumidor lento se
detiene en el punto `await`, y el siguiente batch solo se obtiene
cuando el buffer en memoria se drena.

`lazy()` agrupa en batches mediante un cursor de PK con un tamaño
por defecto de 1000 filas. Sobrescribe el tamaño del batch con
`lazy_by_id(500)`. `cursor()` es el nombre de Laravel y es un alias
de costo cero para `lazy()`.

Misma restricción de PK `i64` que `chunk_by_id`.

### Cargas anticipadas dentro de los chunks

Los siete puntos de entrada **rechazan `.with(...)` de forma
estrepitosa desde el principio** con un `FrameworkError::internal`.
El clon entre batches del Builder descarta el plan de carga
anticipada con el tipo borrado (su predicado `Box<dyn Any>` no es
clonable sin endurecer la API pública), así que respetar el plan
sería silenciosamente inconsistente entre batches. Vuelve a aplicar
`.with(...)` dentro del closure de cada chunk cuando lo necesites -
el `Collection<M>` de cada batch se compone con `load(...)` /
`load_missing(...)`:

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

## Colecciones

`Collection<T>` es la colección con forma Laravel de Suprnova - el
tipo de retorno de `Builder::get` (donde `T` es el modelo), de
`Model::all`, de `pluck` / `chunk_map`, y de cualquier otro terminal
que produzca más de una fila. Hace deref a `&[T]`, así que los
sitios de llamada con Vec existentes siguen funcionando sin cambios;
la superficie de Laravel se compone encima. Esta sección es la
superficie del día a día; el índice completo de métodos, la
separación genérico-frente-a-modelo, el envoltorio de streaming
`LazyCollection<M>`, y las reglas de préstamo-frente-a-consumo están
en [Colecciones de Eloquent](eloquent-collections.md).

### Superficie genérica

Disponible en cualquier `Collection<T>`, sin importar `T`:

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9]);

nums.first();              // Some(&3)
nums.last();               // Some(&9)
nums.len();                // 6
nums.is_empty();           // false
nums.contains(&4);         // true
// Los closures de predicado reciben `&&T` - nota el doble deref `**n`:
nums.first_where(|n| **n > 3);    // Some(&4)
nums.contains_where(|n| **n > 8); // true
// Para un conteo, ejecuta el predicado en línea: `nums.iter().filter(|n| **n > 2).count()` - 4
```

Las transformaciones consumen `self` y devuelven un `Collection`
nuevo:

```rust
let doubled: Collection<i32> = nums.clone().map(|n| n * 2);
let evens:   Collection<i32> = nums.clone().filter(|n| n % 2 == 0);
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(2); // [[3,1],[4,1],[5,9]]
let unique:  Collection<i32> = nums.clone().unique();
let sorted:  Collection<i32> = nums.clone().sort();
```

### Métodos conscientes del modelo en `Collection<M>`

Cuando `T` es un modelo, hay métodos adicionales indexados por
string que se enrutan a través del accesor `field_value(name)`
emitido por la macro:

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

El `pluck_by` basado en closure es la alternativa tipada - útil
cuando el nombre del campo, de otro modo, requeriría una búsqueda
por string que el sistema de tipos no puede comprobar:

```rust
let names: Collection<String> = users.pluck_by(|u| u.name.clone());
```

El `field_value(name)` por fila devuelve `Option<serde_json::Value>` -
`None` cuando el nombre de columna no coincide con ningún campo
declarado. Los casts personalizados que fallan al serializar también
emergen como `None`. Los métodos indexados por string omiten esas
filas en silencio; la forma con closure corta en seco dentro del
cuerpo del closure, para que quien llama decida.

### Streaming mediante `LazyCollection`

Para conjuntos de datos demasiado grandes para materializar,
`Builder::lazy()` / `lazy_by_id(n)` / `cursor()` devuelven un
`LazyCollection<M>` - un envoltorio de `Stream` que obtiene filas en
batches por cursor de PK. Consulta
[Iteración por chunks y perezosa](#iteración-por-chunks-y-perezosa).

### Carga anticipada sobre una colección

`Collection::load(["posts"])` / `load_missing(["posts"])` ejecutan
el mismo despacho de carga anticipada que emite una cadena
`Builder::with(...)`, pero contra una colección ya existente.
`load_missing` es por fila: cada fila de la colección se particiona
en los cubos "necesita carga" / "ya cargada", y solo a las que
faltan se les aplica la carga masiva. Consulta
[Carga anticipada](#carga-anticipada).

## Asignación masiva

### Lista de permitidos `fillable`

```rust
#[model(
    table = "users",
    fillable = ["name", "email"],
)]
pub struct User { /* ... */ }

User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    admin: true,    // se descarta silenciosamente en tiempo de ejecución - no está en fillable
}).await?;
```

### Lista de bloqueo `guarded`

`guarded` es la inversa - todos los campos son fillable EXCEPTO los
que están en guarded. Es mutuamente excluyente con `fillable`; usar
ambos a la vez es un error en tiempo de compilación de la macro.

```rust
#[model(
    table = "posts",
    guarded = ["id", "user_id"],   // todo lo demás es fillable
)]
pub struct Post { /* ... */ }
```

### Política por defecto

Cuando no se fija ni `fillable` ni `guarded`, la política por
defecto es `guarded = ["id"]` (o lo que resuelva
`primary_key = "..."`) - todos los campos son fillable excepto la
clave primaria. Esto coincide con el valor por defecto de Laravel de
"todos los campos fillable excepto la PK".

### Vía de escape `unguarded(closure)`

`unguarded(closure)` desactiva el filtro para un bloque:

```rust
use suprnova::eloquent::unguarded;

// Omite el filtro para un script de migración de datos de una sola vez:
unguarded(|| async {
    User::create(attrs! {
        name: "Bootstrap",
        email: "boot@example.com",
        admin: true,    // asignable dentro del closure
    }).await
}).await?;
```

Implementación: un booleano `tokio::task_local!` que el filtro
`Fillable::apply` comprueba antes de ejecutarse. Task-local significa
que las solicitudes concurrentes no se ven afectadas por el scope
`unguarded` de otra tarea.

## Casts

Los casts se ejecutan en el límite entre el almacenamiento (valor de
columna) y el runtime (campo del modelo). Cada tipo de cast
implementa el trait `Cast`. Los casts integrados cubren el conjunto
completo de Laravel; los usuarios registran casts personalizados a
través del trait. Esta sección es el índice de referencia rápida; el
contrato completo por cast - primitivo, temporal, estructurado,
enum, cifrado, hash - más la macro `casts!` de override en tiempo de
ejecución, vive en
[Conversiones, accesores y mutadores de Eloquent](eloquent-mutators.md).

### Solo explícito

Los casts se declaran en `#[model(casts = { ... })]` - no hay
autodetección a partir de los tipos de campo. Un campo `prefs: Json`
no se convierte implícitamente en `AsJson`; escribes
`casts = { prefs = AsJson }`. Motivo: deberías poder leer el modelo
y saber exactamente qué se ejecuta en los límites de
almacenamiento. Sin magia.

### Ejemplo

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

### Lista completa de casts de Laravel y su mapeo en Suprnova

| Cast de Laravel | Cast de Suprnova | Tipo en runtime |
|--------------|---------------|--------------|
| `bool`, `boolean` | `AsBool` | `bool` |
| `int`, `integer` | `AsInt<I>` | `I: PrimInt` |
| `float`, `double`, `real` | `AsFloat` | `f64` |
| `decimal:N` | `AsDecimal<N>` | `rust_decimal::Decimal` |
| `string` | `AsString` | `String` |
| `array` | `AsArray<T>` | `Vec<T>` (codificado en JSON) |
| `object` | `AsObject<T>` | `T: Serialize + DeserializeOwned` |
| `collection` | `AsCollection<T>` | `Collection<T>` |
| `json` | `AsJson<T>` | `T` (columna JSON en bruto) |
| `date`, `date:format` | `AsDate` | `chrono::NaiveDate` |
| `datetime`, `datetime:format` | `AsDateTime` | `chrono::DateTime<Utc>` |
| `immutable_date` | `AsImmutableDate` | `chrono::NaiveDate` |
| `immutable_datetime` | `AsImmutableDateTime` | `chrono::DateTime<Utc>` |
| `timestamp` | `AsTimestamp` | `i64` (época unix) |
| `encrypted` | `AsEncrypted` | `String` (cifrado mediante `Crypt`) |
| `encrypted:array` | `AsEncryptedArray<T>` | `Vec<T>` (JSON + cifrado) |
| `encrypted:object` | `AsEncryptedObject<T>` | `T` (JSON + cifrado) |
| `encrypted:collection` | `AsEncryptedCollection<T>` | `Collection<T>` |
| `EnumClass::class` | `AsEnum<E>` | `E: EnumString + AsRefStr` |
| `AsArrayObject::class` | `AsArrayObject<T>` | `IndexMap<String, T>` |
| `hashed` | `AsHashed` | `String` (`Hash::make` al escribir; nunca descifra) |

22 casts en total. La mayoría se mapea uno a uno con Laravel; el
`AsOptionalDateTime` (usado por `soft_deletes`) es auto-inyectado
por la macro cuando la columna de eliminación suave es
`Option<DateTime<Utc>>`.

### Modos de fallo de los casts cifrados

Los cuatro casts `AsEncrypted*` enrutan cada cifrado/descifrado a
través de la fachada `Crypt` (con clave `APP_KEY`). Cuando el
descifrado falla - clave equivocada, ciphertext truncado, bytes
manipulados, desajuste de la etiqueta AEAD - el cast emerge con un
`FrameworkError::Internal` claro desde `Cast::from_storage`. No hay
un fallback silencioso a basura:

- Cargar una fila mediante `Model::find` / `Model::query()` propaga
  el error de descifrado y (según el `From<inner::Model>` generado
  por la macro) entra en pánico con `cast from_storage failed -
  corrupt data in database column`. Quienes operan el sistema ven
  el fallo en los logs de inmediato; el modelo nunca lleva un texto
  plano verosímil pero incorrecto.
- El cast `AsHashed` es de una sola dirección; nunca descifra, así
  que este modo de fallo no aplica.

Esto coincide con el cast `encrypted` de Laravel: un `APP_KEY`
equivocado contra una columna cifrada existente es un error
contundente, nunca un `null` / string vacío silencioso.

### Rotar `APP_KEY`

Suprnova admite la rotación de claves sin tiempo de inactividad
mediante un *ring* de claves: el `APP_KEY` actual cifra; una
variable de entorno opcional `APP_KEY_PREVIOUS` (separada por comas,
de la más antigua a la más reciente) proporciona fallbacks de
descifrado para los datos escritos bajo claves anteriores. El
cifrado *siempre* usa la clave actual - las claves anteriores solo
participan al descifrar.

Cada descifrado que recurre a una clave anterior emite una línea
`tracing::warn!` con el índice de la clave anterior. El payload del
log excluye deliberadamente el texto plano y el ciphertext; solo el
hecho de la rotación más una pista de re-cifrado que se puede
accionar.

**Procedimiento de rotación** (sin tiempo de inactividad, seguro
para producción):

1. Acuña una clave nueva: `suprnova key:generate` (escribe en
   stdout).
2. Mueve la clave antigua a `APP_KEY_PREVIOUS` y fija `APP_KEY` al
   nuevo valor:
   ```
   APP_KEY_PREVIOUS=<old_key>
   APP_KEY=<new_key>
   ```
3. Despliega. Las escrituras nuevas usan la clave nueva; las filas
   existentes siguen descifrando mediante el fallback de la clave
   anterior. Las advertencias en los logs identifican las columnas
   que todavía dependen de `APP_KEY_PREVIOUS`.
4. Ejecuta una pasada de re-cifrado. Para cada modelo con casts
   cifrados:
   ```rust
   for chunk in User::query().chunk(500).await? {
       for user in chunk {
           // Touch + save reescribe cada columna con cast bajo la
           // clave actual. `Cast::to_storage` siempre recurre a la
           // entrada actual del ring.
           user.save().await?;
       }
   }
   ```
   Esto es idempotente - las filas que ya están en la clave nueva
   simplemente no hacen nada.
5. Una vez que los logs ya no muestren advertencias de
   `APP_KEY_PREVIOUS` (dale al batch y a cualquier dato eliminado
   suavemente / archivado un margen generoso), elimina
   `APP_KEY_PREVIOUS` del entorno y vuelve a desplegar.

**Rotación multi-paso.** Si rotas otra vez antes de completar la
pasada anterior, añade: `APP_KEY_PREVIOUS=<oldest>,<previous>`. El
ring prueba cada clave anterior en orden. La lista está limitada a
8 entradas - una cadena realista es de 1 a 3 (una rotación en curso,
quizá una ronda anterior estancada), y una lista más larga es casi
siempre un accidente de templating de configuración; superar el
límite hace fallar el arranque con un diagnóstico que se puede
accionar, en lugar de descartar en silencio una clave de la que
quien opera el sistema todavía podría depender.

**Restricciones.**

- Una entrada malformada en `APP_KEY_PREVIOUS` hace fallar el
  arranque de forma estrepitosa (igual que un `APP_KEY`
  malformado) - un secreto rotado a medias nunca debería degradarse
  en silencio.
- Más de 8 entradas en `APP_KEY_PREVIOUS` hace fallar el arranque de
  forma estrepitosa - consulta
  [`suprnova::crypto::MAX_PREVIOUS_KEYS`].
- Las entradas vacías en la lista (por ejemplo, comas finales de una
  configuración generada por plantilla) se toleran como "ninguna
  clave en esta posición" - no es un error.
- El formato en la red no cambia respecto a la disposición de clave
  única previa a la rotación: no se incrusta ningún identificador de
  clave en el ciphertext. El ring intenta descifrar con cada clave
  en orden hasta que una funciona.

### Override de cast en tiempo de ejecución - `with_casts`

```rust
let users = User::query()
    .with_casts(suprnova::casts! { birthdate = AsDateTime })
    .get()
    .await?;
```

`with_casts` sobrescribe los casts declarados del modelo durante la
duración de una única consulta - útil cuando una columna en bruto
vuelve de un join / vista / `select_raw` y necesita una coerción de
tipo distinta a la del modelo por defecto.

### Casts personalizados

Los casts personalizados implementan `Cast`:

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

El trait `Cast` se distribuye junto con los casts primitivos. Los
casts personalizados pueden usar almacenamiento `String` (cuando
codifican en JSON) o cualquiera de los tipos escalares soportados
por SeaORM (`i64`, `f64`, `bool`, `Vec<u8>`).

## Accesores y mutadores

### Accesores

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

Cuando se ejecuta `user.to_array()` (o `user.to_json()`, que delega
en él), se llama al accesor `full_name` y su valor de retorno se
inserta en la salida JSON. Llamar a `user.full_name()` desde Rust es
solo una llamada de método normal.

### Mutadores

Los mutadores se ejecutan antes del almacenamiento:

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

Llamar a `user.password = "secret".into()` asigna directamente el
valor en bruto sin ejecutar el mutador. Para ejecutar la ruta del
mutador, llama a `user.set_password(json!("secret"))` o usa la ruta
JSON (`user.fill(attrs!{password: "secret"})`), que se enruta
automáticamente a través del mutador porque `"password"` aparece en
`mutators = [...]`.

### Cómo funciona el enrutamiento

- La **serialización (`to_array` → `Value`, `to_json` → `String`)**
  ejecuta los accesores. Cada nombre de campo listado en
  `appends = [...]` se convierte en una llamada a `self.<name>()`;
  el valor de retorno se inserta en la salida JSON. `to_json()` es
  un envoltorio fino: `serde_json::to_string(&self.to_array())`.
- Las **escrituras con estilo fill (`fill`, `create`, `update`)** se
  enrutan a través de los mutadores. Cada nombre de campo listado en
  `mutators = [...]` se convierte en una llamada a
  `self.set_<field>(value)` en lugar de una asignación directa.

Las macros a nivel de función `#[accessor]` y `#[mutator]` emiten
entradas de registro que recorren las rutas de serialización / fill
de la macro.

### Los valores malformados son errores, no valores por defecto

Un valor que no puede decodificarse al tipo de su campo hace fallar
la escritura y nombra el campo:

```rust
let err = user.fill(attrs! { age: "not a number" }).unwrap_err();
// ValidationError { field: "age", message: "could not decode the
// supplied value: invalid type: string \"not a number\", expected i32" }
```

El modelo queda intacto - un `fill` rechazado no aplica nada.

Dos casos cercanos se comportan de forma distinta, a propósito:

- Una **columna desconocida** sigue omitiéndose en silencio, igual
  que el `$model->fill()` de Laravel. No conocer una columna no es
  lo mismo que recibir un valor roto para una que sí conoces.
- Una columna excluida por `fillable` / `guarded` se descarta
  mediante el filtro de asignación masiva *antes* de decodificar,
  así que un valor malformado para un campo que quien llama no
  puede fijar también es silencioso. Fallar ahí le diría a quien
  llama sin autorización qué columnas existen.

El ensanchamiento numérico no es un error de tipo: un integer JSON
se decodifica normalmente en un campo `f64`.

> Antes de v0.8.0, un valor malformado se sustituía en silencio por
> el `Default` del campo, y la llamada devolvía `Ok` -
> `fill(attrs!{ age: "abc" })` fijaba `age = 0` e informaba de
> éxito. Si dependías de esa coerción, valida o convierte antes de
> llamar a `fill`.

### Ocultos / en lista blanca

```rust
#[model(
    table = "users",
    hidden = ["password", "remember_token"],
)]
pub struct User { /* ... */ }
```

`hidden = [...]` es una lista de bloqueo - todas las columnas
excepto las listadas se serializan. `visible = [...]` es la forma
inclusiva - solo las listadas se serializan. Mutuamente excluyentes
en tiempo de compilación.

## Timestamps

Cuando existen tanto la columna `created_at` como `updated_at`, la
macro las detecta automáticamente y activa el seguimiento de
timestamps:

- `created_at` se fija a `Utc::now()` en `save()` para las filas
  nuevas.
- `updated_at` se fija a `Utc::now()` en cada `save()`.

La autodetección es conservadora: si el struct tiene solo una de las
dos columnas, la macro falla con un error, para que un error de
tipeo (`craeted_at`) no desactive los timestamps en silencio. Fija
`timestamps = false` para desactivarlo por completo.

### Desactivar los timestamps automáticos

```rust
#[model(table = "audit_logs", timestamps = false)]
pub struct AuditLog {
    pub id: i64,
    pub event: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    // No hay campo updated_at - pero timestamps = false también
    // silencia el error `only one column found` de la macro.
}
```

### `touch()` - sube updated_at sin otros cambios

```rust
user.touch().await?;
```

`touch()` emite `UPDATE table SET updated_at = ? WHERE pk = ?` -
atómico, sin lectura-modificación-escritura. La macro emite un impl
`Touchable` en cada modelo con timestamps.

### Actualizar el propietario

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

Después de crear, guardar, actualizar o eliminar un comentario, se
incrementa el `updated_at` de su post: un
`UPDATE posts SET updated_at = ? WHERE id = ?`, sin `SELECT`. Así, una
clave de caché basada en `post.updated_at` sigue siendo válida cuando
solo cambia un hijo.

Cada nombre de `touches` debe ser una relación `BelongsTo` declarada en
el mismo bloque `relations = { ... }`. Si un nombre no se resuelve o se
resuelve a otro tipo de relación, es un error de compilación y no una
sorpresa en el primer guardado. Los propietarios polimórficos
(`MorphTo`) todavía no se pueden actualizar.

Un propietario cuyo modelo tenga `timestamps = false` se **omite**: no
hay error ni escritura, y el guardado del hijo sigue devolviendo `Ok`.
Lo mismo ocurre con un propietario al que se llega mediante una clave
foránea `NULL` y con un propietario eliminado de forma lógica.

La actualización se ejecuta en el mismo executor que la escritura que
la desencadenó; dentro de un cierre de `DB::transaction` se incorpora a
esa transacción y un rollback también la revierte.

### Por qué Suprnova diverge

El `touchOwners` de Laravel carga cada modelo padre y recurre, de modo
que guardar un comentario también actualiza los propietarios del post y
dispara el evento `saved` de cada padre. Suprnova resuelve el propietario
mediante el registro de relaciones y escribe la columna directamente:
una sentencia por relación actualizada, sin hidratación. Por tanto, la
cascada tiene un solo nivel y no dispara eventos de los padres. Ese es
el costo de que un guardado no emita un `SELECT` por cada relación
actualizada. Usa un observer cuando necesites actualizar el abuelo o
disparar el evento.

`restore()` en un hijo eliminado de forma lógica no actualiza sus
propietarios. El `restore` de Laravel pasa por `save`; el de Suprnova es
un `UPDATE deleted_at = NULL` directo.

### Formato

Siempre ISO 8601 con UTC. No hay override de
`Model::$timestampsFormat` (según la tabla de divergencia respecto
a Eloquent - la interoperabilidad con el frontend va primero; el
formateo según la configuración regional pertenece a la capa de
i18n).

## Observers y eventos de ciclo de vida

Cada modelo pasa por un ciclo de vida fijo de 16 eventos a medida
que avanza por las rutas de `create` / `save` / `update` / `delete`
/ `restore` / `replicate` / consultas del Builder. Los oyentes
pueden enganchar cada evento para registrar, auditar, producir
efectos secundarios, validar, o cancelar la operación en curso.

### Los 16 eventos del ciclo de vida

Los eventos se dividen en dos grupos según si se pueden cancelar:

**Cancelables (5)** - se disparan ANTES de la escritura en la base
de datos. Un oyente que devuelve `EventResult::cancel("reason")`
aborta la operación con `FrameworkError::bad_request(reason)`.

| Evento      | Cuándo                                    | Payload                                                 |
|-------------|-------------------------------------------|---------------------------------------------------------|
| `Saving`    | Antes de `create` y `save`, ambos         | `Arc<Mutex<Attrs>>` + `is_creating: bool`               |
| `Creating`  | Antes de `create`                         | `Arc<Mutex<Attrs>>`                                     |
| `Updating`  | Antes de `save` / `update` en una fila existente | Snapshot del modelo antes de actualizar + `Arc<Mutex<Attrs>>` |
| `Deleting`  | Antes de `delete` (suave o forzado)       | Modelo + `is_force: bool` (force-delete sobre una eliminación suave) |
| `Restoring` | Antes de `restore` en un modelo con eliminación suave | Modelo                                       |

**No cancelables (11)** - se disparan DESPUÉS de la operación. Los
errores de los oyentes se propagan, pero no pueden detener una
escritura que ya se completó.

| Evento          | Cuándo                                             | Payload                          |
|-----------------|---------------------------------------------------|----------------------------------|
| `Retrieving`    | Una vez por consulta del Builder, antes de la llamada a la BD | Ninguno              |
| `Retrieved`     | Una vez por fila devuelta por una consulta del Builder | Modelo                      |
| `Created`       | Tras un `create` exitoso                          | Modelo                            |
| `Updated`       | Tras un `save` / `update` exitoso                 | Snapshots previo + actual        |
| `Saved`         | Tras `create` y `save`, ambos                     | Modelo                            |
| `Deleted`       | Tras un `delete` exitoso                          | Modelo + `is_force: bool`         |
| `Trashed`       | Tras la eliminación suave (NO force-delete)       | Modelo                            |
| `Restored`      | Tras un `restore` exitoso                         | Modelo                            |
| `Replicating`   | Durante `replicate` / `replicate_except`, antes de devolver (NO en `replicate_into` - depende del tipo de origen) | Origen + `Arc<Mutex<replica>>` (mutable) |
| `ForceDeleting` | Antes de `force_delete` en un modelo con eliminación suave | Modelo                    |
| `ForceDeleted`  | Tras un `force_delete` exitoso                    | Modelo                            |

La división entre cancelable / no cancelable refleja el par de
hooks `creating` frente a `created` de Laravel. `Saving` se dispara
tanto para insertar como para actualizar - sobrescribe ese cuando el
comportamiento es idéntico en ambas rutas, y distingue mediante
`is_creating`.

`Replicating` es el único hook no cancelable que entrega una
referencia mutable (la réplica es `Arc<Mutex<M>>`). Úsalo para
limpiar timestamps, regenerar UUIDs, reiniciar auto-incrementos,
etc. antes de que el clon se devuelva a quien llama.

### Observers frente a oyentes en bruto

Dos formas de enganchar los eventos de ciclo de vida:

1. **Oyentes en bruto** - llama a
   `EventFacade::listen::<Created, _>(Arc::new(MyListener))` para
   cada evento que quieras, un impl por evento. Este es el
   mecanismo subyacente; los observers se apoyan encima de él.

2. **Observers** - agrupan los 16 hooks bajo un único trait. La
   macro ve qué métodos sobrescribió el usuario y registra
   exactamente esos. Esta es la ruta recomendada para cualquier
   conjunto no trivial de hooks.

```rust
use async_trait::async_trait;
use suprnova::eloquent::attrs::Attrs;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::observers::Observer;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]   // <- DEBE preceder a #[async_trait]
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

Cada método del trait tiene un no-op por defecto, así que el bloque
impl solo contiene los eventos que te interesan. La macro identifica
las sobrescrituras por coincidencia de nombre contra el conjunto
cerrado de 16 métodos; los métodos que no sobrescribes no registran
ningún oyente.

### Orden obligatorio de los atributos

`#[suprnova::observer(M)]` DEBE aparecer ENCIMA de `#[async_trait]`:

```rust
#[suprnova::observer(User)]   // externo - se ejecuta primero, ve los async fn en bruto
#[async_trait]                // interno - reescribe las firmas de los async fn
impl Observer<User> for AuditObserver { /* ... */ }
```

Las macros de atributo se expanden de fuera hacia dentro.
`async_trait` reescribe cada `async fn` en una forma de fn-de-poll
`Pin<Box<dyn Future>>` sin azúcar sintáctico; si `#[async_trait]` se
ejecutara primero, la coincidencia de nombres de la macro observer
contra los 16 nombres de método del trait no encontraría nada, y
emitiría cero oyentes en silencio.

### Cuatro rutas de registro

| Ruta                                          | Cuándo usarla                                        |
|----------------------------------------------|-----------------------------------------------------|
| `#[suprnova::observer(M)]` (inventario)       | Observer estático conocido en tiempo de compilación. Se instala automáticamente al arrancar. |
| `#[model(observers = [Foo, Bar])]`           | Documentación + validación en tiempo de compilación de que los tipos listados resuelven. NO registra por sí mismo. |
| `Model::observe(MyObs).await`                | Registro en tiempo de ejecución. Manual; útil cuando el registro depende de la configuración. |
| `EventFacade::listen::<events::Created, _>(...)` | Nivel más bajo - un evento a la vez. Úsalo cuando un observer resulte pesado. |

El atributo `observers = [...]` en `#[model]` es un marcador de
documentación. Compila a un bloque
`const _: fn() = || { let _ = ::std::any::type_name::<T>; ... };`
que demuestra que cada tipo listado resuelve a un ítem real de Rust;
los errores de tipeo emergen en el sitio de declaración del modelo.
La instalación real ocurre a través de la vía de inventario - el
atributo `#[observer(M)]` en `Foo` es lo que inscribe a `Foo` para
la instalación automática.

### Arranque de la aplicación

Llama a `bootstrap_observers()` una vez al arrancar para drenar el
inventario e instalar cada observer registrado con `#[observer(M)]`:

```rust
suprnova::eloquent::observers::bootstrap_observers().await?;
```

El drenado es idempotente para la vía de inventario - el closure de
instalación de cada observer está protegido por un `AtomicBool` por
tipo (emitido por la macro de T2b), así que llamar a
`bootstrap_observers()` dos veces no registra por duplicado.

El shim en tiempo de ejecución `Model::observe(MyObs)` NO está
protegido. Llamarlo dos veces registra dos conjuntos de oyentes, lo
que coincide con la semántica manual de
`Model::observe(MyObs::class)` de Laravel. Si un observer instalado
a mano también tiene `#[observer]`, el adaptador de inventario se
dispara además de los instalados manualmente.

### Cancelar desde un observer

Los cinco hooks cancelables devuelven `EventResult`. Para abortar la
operación, devuelve `EventResult::cancel("reason")`:

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

El motivo de la cancelación emerge como
`FrameworkError::bad_request(reason)` desde `Subscription::create`.
La fila nunca llega a la base de datos - cancelar es un abort real,
no un "eliminar después del hecho".

Varios observers pueden registrar hooks cancelables sobre el mismo
modelo; que cualquiera de ellos devuelva `Cancel` detiene la
operación. El orden es el orden de inscripción en el inventario (en
la práctica, el orden de enlazado).

### Varios observers sobre un mismo modelo

Varios impls de `Observer<M>` se disparan todos para el mismo
evento - el despacho de EventFacade se dispersa hacia cada oyente
registrado, en lugar de elegir uno solo:

```rust
#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for AuditObserver { /* ... */ }

#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for NotifyObserver { /* ... */ }

// Comment::create(...) dispara tanto AuditObserver::created COMO NotifyObserver::created.
```

Esto coincide con la semántica de dispersión de Laravel y es la
propiedad que sostiene el patrón "descomponer los hooks por
responsabilidad": un `AuditObserver` solo sabe de auditoría, un
`NotifyObserver` solo sabe de notificaciones, y a la declaración del
modelo no le importa cuántos observers se adjunten.

### `Model::observe()` manual

Cada struct `#[suprnova::model]` recibe un shim `observe<O>()` por
modelo. Llámalo al arrancar para el registro dinámico:

```rust
#[derive(Clone)]
struct MyObs;

#[async_trait]
impl Observer<User> for MyObs { /* ... */ }

// En tiempo de ejecución:
User::observe(MyObs).await;
```

El bound `O: Clone + 'static` del shim es lo que permite que el
framework entregue un clon nuevo del observer a cada uno de los 16
oyentes adaptadores internos. Los 16 adaptadores de oyente se
instalan en cada llamada - los valores por defecto del trait hacen
que los métodos no sobrescritos sean no-ops baratos.

### Restricciones

- **La versión con macro requiere que el bloque impl use nombres de
  método planos que coincidan con los 16 hooks del trait.** Los
  métodos renombrados, los valores por defecto suprimidos con
  `#[allow]`, y los cuerpos protegidos con `#[cfg]` quedan fuera de
  la coincidencia de nombres y no registran oyentes.

- **Los structs de observer que inspecciona la macro deben tener
  tamaño cero** (sin campos) en v1. La macro construye el observer
  mediante `let obs = MyObserver;` dentro de cada adaptador. Los
  observers con estado (que llevan `Arc<Inner>`) necesitan la ruta
  en tiempo de ejecución `Model::observe()`, que toma el observer
  por valor y lo clona en cada adaptador.

- **Aislamiento de tests: usa tipos de modelo únicos por
  escenario.** El EventDispatcher global del proceso significa que
  los oyentes instalados para `User` son visibles para cada test en
  el mismo binario. Los tipos de modelo únicos por test
  (`T2Comment`, `T2Subscription`, …) mantienen la filtración entre
  tests fuera de las aserciones de contador. Los tests de
  integración de `eloquent_observers.rs` ejercitan este patrón.

## Prunable

Laravel incluye un trait `Prunable` que permite que un modelo
declare un scope de filas a eliminar según una programación.
Suprnova refleja eso con dos traits y un comando de consola.

### Declarar un pruner

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

### `MassPrunable` - variante de eliminación masiva

Para tablas de alto volumen (logs de auditoría, logs de solicitudes,
entradas de caché expiradas), `MassPrunable` se salta los eventos
por fila y ejecuta una única declaración `DELETE WHERE …`:

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

### Disparar el pruning

Se ejecuta a través de la consola de cada proyecto (para la que
`app/cmd/main.rs` llama a `suprnova::console::dispatch_argv`,
después de `db:seed` y los demás comandos integrados):

```bash
suprnova model:prune                          # ejecuta prune sobre cada tipo registrado
suprnova model:prune --model=ExpiredSession   # filtra a un solo modelo
suprnova model:prune --pretend                # dry run; registra en el log qué se eliminaría
```

De forma programática, los runners están en
`suprnova::eloquent::{prune_all, prune_all_dry, prune_one}`.

### Hook de pruning

`Prunable::pruning(&self)` se dispara antes de cada eliminación de
fila, para que el usuario pueda ejecutar efectos secundarios (limpiar
archivos asociados, dispersar eventos, etc.). El impl por defecto
está vacío. `MassPrunable` se salta este hook por definición - las
eliminaciones masivas no enumeran filas.

### Comportamiento de cascada

**El pruning NO hace cascada automática hacia las filas
relacionadas.** Un impl de `Prunable` o `MassPrunable` sobre `User`
elimina filas de usuario; sus `posts`, las entradas pivot de
`role_user`, los `comments` polimórficos, etc. quedan HUÉRFANOS, con
columnas FK que apuntan al usuario ya eliminado.

Esto coincide con el contrato de Laravel: la limpieza de relaciones
es responsabilidad del usuario. Dos formas limpias de manejarlo:

1. **Cascada a nivel de base de datos mediante FK** - declara
   `ON DELETE CASCADE` (o `ON DELETE SET NULL`) en la restricción de
   clave foránea al escribir la migración. El motor de la BD maneja
   la cascada gratis, sin código Rust por fila.

2. **Hook por fila** - implementa `Prunable::pruning(&self)` para
   eliminar a los hijos antes de que se elimine la fila padre. El
   hook se dispara dentro de la misma operación lógica que la
   eliminación del padre, así que el orden consistente está
   garantizado:

   ```rust
   #[async_trait]
   impl Prunable for User {
       fn prunable() -> Builder<Self> {
           Self::query().filter_op("deleted_at", "<", thirty_days_ago())
       }

       async fn pruning(&self) -> Result<(), FrameworkError> {
           // Elimina los posts.
           Post::query().filter("user_id", self.id).get().await?
               .into_iter()
               .map(|p| p.delete());
           // Separa los pivots de rol.
           self.roles().sync(Vec::<i64>::new()).await?;
           Ok(())
       }
   }
   ```

`MassPrunable` está basado en conjuntos - `pruning()` no se
dispara. Usa `Prunable` normal cuando necesites cascada. El
framework no emitirá silenciosamente un DELETE por fila cuando optes
por `MassPrunable`; el trade-off queda documentado de forma
estrepitosa.

### Mecanismo de registro

El registro de pruners usa el mismo patrón de inventario que los
observers, los comandos, y los supervisores. El atributo
`#[suprnova::prunable]` en el bloque `impl Prunable for T { ... }`
se auto-registra mediante `inventory::submit!` en tiempo de
compilación. No hay archivo de configuración central; añadir un
nuevo tipo prunable es un solo atributo.

## Enrutamiento multi-conexión

Las apps en producción suelen necesitar más de una conexión de base
de datos - el caso canónico es una réplica de lectura para
analítica más la primaria para escrituras, pero la superficie se
generaliza a cualquier conexión con nombre (BD de reportes, BD de
archivo, shard por tenant).

### Registrar una conexión

Llama a `DB::register_named(name, config)` al arrancar para cada
conexión no predeterminada con la que hable tu app:

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

Dos nombres están reservados: `__primary__` cortocircuita el
registro hacia `DB::connection()`, y `__read_replica__` inscribe a
la conexión en el enrutamiento automático dividido por
lectura/escritura - ver más abajo.

### Opt-in por consulta: `Model::on(name)`

`Model::on("reporting")` devuelve un `Builder<M>` ya configurado
para enrutar a través de la conexión con nombre:

```rust
let totals = Order::on("reporting")
    .order_by_desc("total")
    .limit(100)
    .get()
    .await?;
```

`on(...)` tiene alcance de solicitud - solo afecta al builder
encadenado. La siguiente llamada simple a `Order::query()` se
resuelve a través de la conexión por defecto.

### Valor por defecto por modelo: `#[model(connection = "...")]`

Cuando un modelo siempre vive en una conexión, declara el valor por
defecto en el atributo:

```rust
#[model(table = "events", connection = "events_db")]
pub struct Event { /* ... */ }
```

Cada llamada a `Event::query()` / `Event::create()` /
`Event::find()` se enruta a través de `events_db` sin necesitar el
override `.on(...)` por consulta. Un `.on(...)` explícito sobre un
builder sigue ganando.

### División lectura-escritura

Registrar una conexión bajo el nombre reservado `__read_replica__`
inscribe a cada modelo en el enrutamiento automático: los métodos
de lectura (`first` / `get` / `find` / `count` / `paginate` /
`chunk` / los recorridos dirigidos por closure) fluyen a través de
la réplica; las escrituras (`save` / `create` / `update` /
`delete` / `force_delete` / `replicate` / `attach` / `detach` /
`sync` / `increment` / `decrement`) fluyen a través de la primaria.

`Model::on_write_connection()` excluye a un único builder de la
réplica - útil cuando importa la consistencia de "leer tus propias
escrituras" (por ejemplo, justo después de un `save`, antes de que
la replicación se pusiera al día).

### Precedencia de enrutamiento

La cadena de despacho ejecuta cada operación a través de
`ExecutorChoice::resolve_read` o `resolve_write`. El orden es:

1. **Una transacción activa gana de forma absoluta.** Dentro de
   `DB::transaction`, cada lectura Y cada escritura usan la
   conexión de la tx. `on(name)` se IGNORA dentro de una
   transacción - la tx está ligada a una conexión física
   específica. SeaORM no puede iniciar una transacción en una
   conexión y ejecutar declaraciones contra otra.
2. **`on(name)` por builder.** Se fija mediante `Model::on(name)` /
   `Builder::on(name)`. Gana sobre el valor por defecto del modelo
   y sobre la división lectura/escritura.
3. **`Model::on_write_connection()`.** Fuerza la primaria incluso
   cuando la operación de otro modo se enrutaría a la réplica.
4. **Valor por defecto por modelo `#[model(connection = "...")]`.**
   Gana sobre la división lectura/escritura para las consultas
   propias del modelo.
5. **División lectura/escritura.** Cuando `__read_replica__` está
   registrada, los métodos de lectura se enrutan ahí; las
   escrituras se enrutan a la primaria.
6. **Por defecto.** `DB::connection()` - la primaria, la que
   configuró `DB::init()`.

### Advertencias

- Las transacciones activas IGNORAN `on(name)` (ver el §1 de
  arriba). Si necesitas una escritura en otra conexión a mitad de
  una tx, no puedes - la tx está ligada a una conexión.
- Los nombres reservados `__primary__` y `__read_replica__` no se
  pueden usar como nombres de conexión de usuario.
  `DB::register_named` devuelve un error ante la colisión.
- El lag de la réplica es TU problema. Suprnova no reintenta en
  lectura ni recurre a la primaria cuando la réplica está
  desfasada; si necesitas leer tus propias escrituras después de un
  save, usa `Model::on_write_connection()` explícitamente.

## Replicación

`Model::replicate()` devuelve una copia sin guardar del modelo, con
la clave primaria reiniciada a su valor por defecto. Útil para una
UX de "duplicar este registro", donde el usuario quiere partir de
una fila existente.

```rust
let template: User = User::find_or_fail(42).await?;
let mut copy = template.replicate().await?;  // id reiniciado a su valor por defecto
copy.email = "fresh@example.com".into();
copy.save().await?;  // INSERT, no UPDATE
```

`replicate` es **async** en Suprnova (diverge de Laravel) porque
dispara el evento `Replicating` - los oyentes de `Saving` /
`Created` / etc. pueden mutar la réplica antes de que se devuelva.
Consulta [Evento `Replicating`](#evento-replicating) para el
contrato de mutación de los oyentes.

### `replicate_except`

Descarta campos nombrados de la réplica:

```rust
let copy = order.replicate_except(["payment_token", "stripe_id"]).await?;
```

Los campos listados recurren al impl `Default` del modelo - los
`String` se convierten en `""`, los `Option` en `None`, etc. Usa
esto para columnas sensibles que la fila replicada no debería
llevar consigo.

### `replicate_into::<T>` entre tipos

La divergencia de Suprnova - Laravel no puede porque PHP no tiene
tipos. `replicate_into::<T>()` une con un tipo hermano mediante
`serde_json`:

```rust
let order: Order = Order::find_or_fail(42).await?;
let invoice: Invoice = order.replicate_into::<Invoice>().await?;
invoice.save().await?;
```

Los campos con nombres coincidentes y tipos compatibles con serde
se trasladan; los campos que no coinciden en ningún lado se
descartan en silencio. `T` debe implementar `Default` para que los
campos sin llenar tengan un valor. La replicación entre tipos NO
dispara `Replicating` (el evento lleva un `&mut Self` - no hay forma
de dirigirse a `T` a través de él). Si necesitas mutación dirigida
por eventos, replica primero dentro del mismo tipo y luego
materializa `T` a partir del resultado.

## Depuración - dump y dd

Dos ayudas de depuración interactiva en cada `Builder<M>`:

```rust
// Registra SQL + bindings mediante tracing::info!, devuelve self.
let users = User::query()
    .filter("active", true)
    .dump()                       // → línea de log, el builder continúa
    .order_by_desc("created_at")
    .get()
    .await?;

// Registra en tracing::error!, y luego entra en pánico con el SQL en el mensaje.
User::query().filter("id", 1).dd();  // - !
```

`dump` es encadenable; `dd` devuelve `!` (nunca retorna - el pánico
es el contrato). Ambos reflejan exactamente el `Builder::dump()` /
`Builder::dd()` de Laravel.

Ambos helpers recurren al dialecto de SQLite cuando no hay ninguna
conexión de BD viva vinculada (coincide con el fallback de
`to_sql_with_bindings`), así que siguen siendo útiles en un REPL o
en un test sin `TestDatabase`.

El mensaje de pánico usa el prefijo literal `eloquent dd:` para que
los tests puedan hacer aserciones contra él:

```rust
#[test]
#[should_panic(expected = "eloquent dd")]
fn dd_panics_with_sql_in_message() {
    User::query().filter("id", 1).dd();
}
```

**Nunca hagas commit de `dd()` en una ruta de código de
producción.** Es una ayuda de depuración interactiva; el pánico al
salir es el punto entero. `dump()` es más seguro (solo registra en
el log), pero saturarlo en rutas de ejecución frecuente llenará tus
logs - quítalo antes de hacer push.

Si quieres el SQL sin los efectos secundarios, recurre a los
helpers que no registran en el log:

- `Builder::to_sql()` - devuelve el SQL renderizado como `String`.
- `Builder::to_sql_with_bindings()` - devuelve
  `(String, Vec<SeaValue>)`.
- `Builder::to_sql_for(backend)` - renderiza para un dialecto
  explícito (depuración entre backends).

## Pruebas de modelos

Los tests instancian una base de datos real mediante `TestDatabase`,
que registra la conexión en el contenedor por test, de modo que
cualquier cosa que llame a `DB::connection()` dentro del SUT se
resuelva contra la BD de test.

### Dos puntos de entrada

- **`TestDatabase::fresh::<MyMigrator>().await`** - ejecuta cada
  migración que ejecuta el migrator de producción. Usa esto para
  tests dogfood a nivel de app, donde quieres que el esquema de
  test coincida exactamente con lo que produce `suprnova migrate`.
- **`TestDatabase::sqlite_memory().await`** - abre una base de datos
  SQLite en memoria SIN aplicar ninguna migración. Usa esto para
  tests unitarios a nivel de framework, donde quieres un control
  preciso de la forma de las columnas mediante
  `db.execute_unprepared("CREATE TABLE …")` por test.

### Patrón dogfood a nivel de app

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

El binding `_db` mantiene el `TestDatabase` durante todo el test -
soltarlo (drop) desmonta el contenedor y libera la conexión SQLite
en memoria. No lo sombrees con `_`, o la conexión desaparece antes
de que se ejecute el SUT.

### Patrón de forma a nivel de framework

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

### Patrones clave

- `TestDatabase::fresh::<MyMigrator>()` para tests a nivel de app
  con el esquema de producción. `TestDatabase::sqlite_memory()`
  para tests de forma a nivel unitario.
- Usa `TestContainer::bind` (NO `App::bind`) para cualquier
  singleton que el test mute - los overrides del registro global
  compiten entre sí en ejecuciones en paralelo. El constructor de
  `TestDatabase` maneja el binding de la BD por ti.
- Mantén las declaraciones de modelo a nivel de módulo, no dentro
  de funciones de test. La macro emite un `mod` interno cuyo
  `use super::*;` solo ve los imports de nivel superior del
  archivo - declarar un modelo dentro de una función de test rompe
  la resolución de tipos de SeaORM.

## Bajar a SeaORM

Tres vías de escape mantienen a SeaORM accesible desde dentro de la
capa Eloquent:

1. **El módulo interno** - `user::Entity`, `user::Column`,
   `user::ActiveModel`, `user::Model`. La macro los emite para cada
   modelo; son tipos de SeaORM que puedes usar directamente.
   Consulta [Disposición del módulo del modelo](#disposición-del-módulo-del-modelo)
   para la disposición completa y cuándo entrar en él.
2. **Conversiones `From`** - `From<user::Model> for User` y
   `From<User> for user::Model` unen las filas con forma SeaORM
   (columnas con el tipo de almacenamiento) y las filas con forma
   Eloquent (columnas con el tipo en tiempo de ejecución). Útil
   cuando quieres emitir una consulta de SeaORM y convertir el
   resultado a la forma Eloquent, o viceversa.
3. **Los tipos de SeaORM alias de Suprnova** - cada tipo de SeaORM
   que un consumidor tocaría se reexporta bajo `suprnova::*`. No
   deberías necesitar `use sea_orm::*` en el código de la app.

```rust
use suprnova::sea_orm::{ColumnTrait, EntityTrait};

// Bajamos a SeaORM a mitad de la consulta - Eloquent no tiene un
// método para esto, pero SeaORM sí:
let db = suprnova::DB::connection()?;
let users = user::Entity::find()
    .filter(user::Column::Email.like("%@example.com"))
    .all(db.inner())
    .await?;

// Convierte a la forma Eloquent:
let eloquent: Vec<User> = users.into_iter().map(User::from).collect();
```

Tres vías de escape más el puente `From` significan que la capa
Eloquent nunca te bloquea el acceso al ORM subyacente.

## Migrar desde `database::Model`

El código más antiguo puede llevar
`impl suprnova::database::Model for Entity {}` sobre una entidad de
SeaORM escrita a mano. El trait se renombró a `EntityExt` para
dejar espacio al nuevo trait `Model` - que se sitúa sobre el struct
de cara al usuario, no sobre la entidad de SeaORM.

La ruta de migración recomendada es cambiar el tipo a
`#[suprnova::model]`, que te da la superficie completa de Eloquent
más los traits `EntityExt` renombrados como bonus. Para el caso raro
en que quieras conservar la vieja forma de extensión de Entity de
SeaORM, los nombres de trait `EntityExt` / `EntityExtMut` siguen
disponibles bajo `suprnova::database::*`. Se comportan exactamente
igual que el antiguo `database::Model`.

## Fachada DB - consultas sin modelo

Algunas tablas no pertenecen en un struct `#[suprnova::model]`:
logs de auditoría de vida corta, joins de reportes ad hoc, agregados
de panel. Para esas, recurre a la fachada `DB`. Debajo de ella hay
dos superficies:

### `DB::table(name)` - constructor de consultas encadenable

`DbTableBuilder` refleja la forma where / order / limit de
`Builder<M>`, pero devuelve las filas como `DynamicRow` (un newtype
con accesores tipados sobre `serde_json::Map<String, Value>`):

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

La superficie completa:

| Método | Devuelve | Propósito |
|--------|---------|---------|
| `.select(["id", "event"])` | `DbTableBuilder` | Restringe las columnas (por defecto `*`) |
| `.filter(col, val)` | `DbTableBuilder` | `WHERE col = ?` |
| `.filter_op(col, op, val)` | `DbTableBuilder` | `WHERE col <op> ?` |
| `.order_by_asc(col) / _desc(col)` | `DbTableBuilder` | Orden |
| `.limit(n) / .offset(n)` | `DbTableBuilder` | Ventana |
| `.get()` | `Collection<DynamicRow>` | Todas las filas que coinciden |
| `.first()` | `Option<DynamicRow>` | Primera fila o `None` |
| `.count()` | `u64` | `SELECT COUNT(*) ...` |
| `.insert(attrs)` | `i64` | `id` de la nueva fila |
| `.update(attrs)` | `u64` | Filas afectadas |
| `.delete()` | `u64` | Filas afectadas |

**Límite de confianza de los identificadores.** Los nombres de
tabla, los nombres de columna, los operadores SQL, y las direcciones
de ORDER BY se interpolan literalmente en la cadena SQL - NO se
vinculan como parámetros. Pasa solo literales de confianza, fijados
en tiempo de compilación, a estos argumentos. Los valores (el lado
derecho de `filter` / `filter_op`) SÍ se vinculan y son seguros de
pasar directamente desde datos de la solicitud.

**Un WHERE vacío en `update` / `delete` opera sobre cada fila.**
`DB::table("audit_log").delete().await?` trunca la tabla por
diseño - añade un `filter` si no es lo que quieres.

**Diferencia de backend en la inserción.** `RETURNING id` se usa en
Postgres y SQLite; MySQL ejecuta el INSERT y luego emite
`SELECT LAST_INSERT_ID() as id` para recuperar el auto-incremento.

### `DynamicRow` - accesores tipados sobre un mapa JSON

`DynamicRow` envuelve un `serde_json::Map<String, Value>` y expone
getters tipados. Cada uno devuelve `Result<T, FrameworkError>` con
un mensaje de error claro ante una clave ausente o un tipo que no
coincide:

```rust
let event: String     = row.get_string("event")?;
let actor_id: i64     = row.get_int("actor_id")?;
let active: bool      = row.get_bool("active")?;
let prefs: Prefs      = row.get_as("prefs")?;  // cualquier DeserializeOwned
let raw: serde_json::Value = row.get_value("meta")?;
```

Columnas anulables: usa `get_optional_*`. Estos distinguen "columna
ausente" (error - desajuste de esquema) de "columna presente, valor
null" (`Ok(None)`):

```rust
let score: Option<i64>      = row.get_optional_int("score")?;
let title: Option<String>   = row.get_optional_string("title")?;
```

`DynamicRow` hace deref hacia `Map<String, Value>`, así que la
iteración y la comprobación de existencia de claves funcionan de
forma natural:

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}
```

### Escapes de SQL en bruto

Cuando el builder no basta - funciones de ventana, CTEs recursivas,
DDL específico del backend - baja a una cadena en bruto. Los
placeholders coinciden con el backend activo (`$1, $2, ...` para
Postgres, `?` para MySQL + SQLite):

```rust
// SELECT en bruto, materializado como DynamicRow.
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) as post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING post_count > ?",
    vec![5i64.into()],
).await?;

// UPDATE / DELETE en bruto - devuelven las filas afectadas.
let updated = DB::update(
    "UPDATE users SET verified_at = NOW() WHERE id = ANY($1)",
    vec![ids.into()],
).await?;

let deleted = DB::delete(
    "DELETE FROM stale_sessions WHERE expires_at < ?",
    vec![now.into()],
).await?;

// DDL en bruto o declaraciones sin binding.
DB::statement("CREATE INDEX CONCURRENTLY idx_users_email ON users(email)")
    .await?;

// Declaración genérica "affecting" - para INSERT ... ON CONFLICT, etc.
let rows = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1) ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec!["page_views".into()],
).await?;
```

Usa estas vías de escape con moderación - el builder tipado detecta
más errores en tiempo de compilación y se lee más limpio en la
lógica de negocio. Pero cuando las necesites, aquí están.

**La trampa de las columnas agregadas.** Los agregados sin tipo
como `SELECT COUNT(*) AS n FROM t` funcionan a través del helper
`.count()` del builder, pero pueden quedar silenciosamente
descartados de las filas de `DB::select` en bruto sobre SQLite - el
`JsonValue::from_query_result` subyacente recorre la información de
tipo por columna de sqlx, y un agregado desnudo no lleva ninguna. Si
necesitas la ruta de select en bruto con agregados, dale a la
expresión un contexto tipado: usa un envoltorio
`CAST(... AS BIGINT)`, o lee la columna con un helper tipado
`DB::table(...).count()` / `.max(...)` que use `query_one` +
`try_get` por debajo.

## Existencia de relación + atajos económicos

Suprnova refleja la familia de consultas de existencia de relación de
Laravel. Cada método de aquí empareja el nombre con forma de Laravel con
un alias idiomático de Rust (la convención permanente de API dual de
Suprnova).

### Filtros de existencia de relación (`has` / `where_has` / `where_belongs_to`)

La familia de `EXISTS (...)` correlacionados restringe la consulta padre
por la existencia (o ausencia, o recuento) de filas relacionadas, sin
unir la relación al SELECT exterior.

```rust
use suprnova::Model;

// Usuarios que tienen al menos un post.
let users = User::query().has("posts").get().await?;

// Usuarios que NO tienen posts.
let empty = User::query().doesnt_have("posts").get().await?;

// Usuarios con >= 3 posts (el `has("posts", ">=", 3)` de Laravel).
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// Restricción interna vía closure - acota el cuerpo de la subconsulta EXISTS.
let recent = User::query()
    .where_has::<Post, _>("posts", |q| q.filter_op("created_at", ">=", "2026-01-01"))
    .get()
    .await?;

// Atajo de una columna - equivale a `where_has` con una closure diminuta.
let with_pub = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// Join directo de belongs-to (sin EXISTS - la FK vive en esta tabla).
let posts = Post::query().where_belongs_to("author", author.id).get().await?;
```

Todas las variantes se componen con sus contrapartes `or_*` y
`*_doesnt_have`:

- `has` / `or_has` / `has_count` / `doesnt_have` / `or_doesnt_have`
- `where_has` / `or_where_has` / `where_doesnt_have` / `or_where_doesnt_have`
- `where_relation` / `where_relation_op` / `or_where_relation`
- `where_belongs_to`

El motor lee los metadatos de relación del inventario `RelationEntry`
que genera la macro: columnas de join, tablas pivot y discriminadores
morph fluyen todos automáticamente. Se renderizan tres formas de
subconsulta:

- **Has** - `EXISTS (SELECT 1 FROM child WHERE child.fk = parent.pk)`
- **Pivot** - `EXISTS (SELECT 1 FROM pivot INNER JOIN target ON ... WHERE pivot.parent_fk = parent.pk)`
- **Morph** - la forma has/pivot más `AND target.<morph>_type = '<value>'`

Los nombres de relación desconocidos renderizan la forma de fallo seguro
(`EXISTS (SELECT 1 WHERE 1 = 0)`), que evalúa a `FALSE` y devuelve cero
filas. Una errata nunca deja escapar un escaneo de tabla completa.

### Divergencia de `MorphTo`

El inverso `MorphTo` de Laravel (`whereMorphedTo`, `whereHasMorph`)
recorre varias tablas de destino porque el hijo morph lleva un
discriminador `*_type` que elige uno de N padres posibles. El `MorphTo`
de Suprnova baja a un enum por familia en el momento de la expansión de
la macro - el tipo de destino es estáticamente un
`<Family>Morph { Variant1(...), ... }`, no una única tabla SQL. El motor
de existencia no puede renderizar un `EXISTS (SELECT 1 FROM <table>)`
fijo para ese caso, porque no hay una única tabla.

Migración recomendada: haz la comprobación de existencia al nivel del
hijo morph. Donde Laravel escribe:

```php
Comment::whereHasMorph('commentable', [Post::class], fn ($q) => $q->where('published', true))
```

Suprnova escribe:

```rust
Comment::query()
    .filter("commentable_type", "post")
    .where_has::<Post, _>("commentable_post", |q| q.filter("published", true))
    .get()
    .await?;
```

La forma con tipado más estrecho da autocompletado completo del IDE
sobre el builder interno, cosa que el `whereHasMorph` con tipado laxo no
puede hacer.

### Atajos económicos del builder

```rust
// Filtros de PK.
User::query().where_key(7).first().await?;        // azúcar para filter("id", 7)
User::query().where_key_not(7).get().await?;      // azúcar para filter_op("id", "!=", 7)
User::query().filter("name", n).or_where_key(7).get().await?;      // ... OR id = 7
User::query().filter("name", n).or_where_key_not(7).get().await?;  // ... OR id != 7
// Alias idiomáticos de Rust: filter_key / filter_key_not /
// or_filter_key / or_filter_key_not.

// Ordenar por created_at.
Post::query().latest().get().await?;              // ORDER BY created_at DESC
Post::query().oldest().get().await?;              // ORDER BY created_at ASC
Post::query().latest_by("published_at").get().await?;  // columna con nombre

// Coincidencia de exactamente uno.
let one = User::query().filter("email", e).sole().await?;          // da error con 0 o >1
let val: i64 = User::query().filter("id", 1).sole_value("views").await?;
let v: i64 = User::query().filter("name", "x").value_or_fail("views").await?;

// Exclusiones de la carga anticipada.
User::query().with(["posts","tags"]).without(["tags"]).get().await?;
User::query().with_only(["posts"]).get().await?;   // borra el plan primero

// Columnas totalmente cualificadas (para joins).
Builder::<User>::qualify_column("name");           // -> "users.name"
Builder::<User>::qualify_columns(["name", "id"]);  // -> ["users.name", "users.id"]
```

### Mutación masiva - `update_all` / `delete_all` / `upsert` / `*_each`

Estos golpean la base de datos directamente con una sola sentencia y NO
disparan los eventos de modelo por fila. Úsalos cuando acotar por scope
sea suficiente y no necesites los ganchos de ciclo de vida; para los
ganchos por fila, itera con `.get()` y llama a `.update()` / `.delete()`
en cada fila. `delete_all` siempre apunta a la `M::TABLE` estática del
modelo; los nombres de tabla en tiempo de ejecución no se aceptan como
SQL ejecutable. Los atributos null explícitos se emiten como `NULL` de
SQL, así que las columnas anulables bigint, integer, boolean, timestamp
y otras no textuales conservan su tipo de base de datos en PostgreSQL.
Todo atributo no nulo sigue vinculado como parámetro. Las filas de un
upsert deben tener el mismo conjunto de columnas; una clave ausente o
adicional se rechaza en vez de interpretarse como null.

```rust
// UPDATE masivo.
let n = User::query()
    .filter("active", false)
    .update_all(attrs! { archived_at: Utc::now() })
    .await?;

// DELETE masivo.
let n = Session::query()
    .filter_op("expires_at", "<", cutoff)
    .delete_all()
    .await?;

// INSERT ... ON CONFLICT (Postgres / SQLite) / ON DUPLICATE KEY UPDATE (MySQL).
let n = Counter::query()
    .upsert(
        vec![attrs! { key: "page_views", n: 1 }, attrs! { key: "signups", n: 1 }],
        vec!["key"],                  // objetivo del conflicto
        Some(vec!["n"]),              // columnas a actualizar; None = todas las no únicas
    )
    .await?;

// Incremento/decremento atómico contra un scope.
User::query()
    .filter("id", 7)
    .increment_each(vec![("views", 1), ("likes", 1)])
    .await?;

User::query()
    .filter("id", 7)
    .decrement_each(vec![("balance", 100)])
    .await?;
```

### Ayudantes estáticos de `Model`

```rust
// Destrucción masiva por conjunto de PK. Se disparan los eventos por fila
// (cada fila pasa por .delete(), así que se respetan la semántica de marca
// de eliminación suave y el despacho de Deleting/Deleted).
let removed: u64 = User::destroy(vec![1i64, 2, 3]).await?;
let removed: u64 = User::force_destroy(vec![1i64, 2, 3]).await?;

// Comparación de identidad por PK.
assert!(alice.is(&also_alice));
assert!(alice.is_not(&bob));
```

### Variantes `*Quietly` - suprimen los eventos de ciclo de vida

Azúcar sobre `seed::without_events`. Los cinco eventos estáticos de
ciclo de vida (`Saving`/`Creating`/`Updating`/`Deleting`/`Restoring`) y
los after-events no cancelables hacen ambos cortocircuito dentro del
scope.

```rust
user.save_quietly().await?;            // sin Saving / Updated / Saved
user.update_quietly(attrs).await?;
user.delete_quietly().await?;
user.force_delete_quietly().await?;
```

### Variantes `*_or_fail`

Error explícito en el caso de no encontrado. Útil en rutas de código que
comprueban invariantes, donde una fila ausente es un bug.

```rust
let user = user.update_or_fail(attrs).await?;   // not_found si la fila se eliminó en vuelo
user.delete_or_fail().await?;
```

### Serialización filtrada - `to_array_except` / `to_array_only`

El reemplazo nativo de Rust en Suprnova para los `makeHidden` /
`makeVisible` por instancia de Laravel. El struct de Eloquent no lleva
una bolsa de atributos en tiempo de ejecución, así que la lista de
columnas se aporta en el sitio de la llamada:

```rust
return Json::ok(user.to_array_except(&["password_hash", "remember_token"]));
return Json::ok(user.to_array_only(&["id", "name", "email"]));
```

**Nota de divergencia.** El `makeHidden` por instancia de Laravel muta
un estado que se propaga cuando el modelo va anidado dentro de la
llamada a `toArray()` de un padre. El filtro de Suprnova es terminal -
produce un `serde_json::Value` y no afecta a futuras serializaciones de
`self`. Para un control de visibilidad declarativo y permanente, usa los
atributos `#[model(hidden = [...])]` / `#[model(visible = [...])]`.

### Claves primarias UUID / ULID - `#[model(unique_id = "...")]`

El análogo en Suprnova de la familia de traits `HasUuids` / `HasUlids` /
`HasVersion4Uuids` de Laravel. Fija el atributo, tipa la PK como
`String`, y la macro autorrellena el ID antes del INSERT.

```rust
#[model(
    table = "users",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "uuid",      // o "uuid_v4", "ulid"
)]
pub struct User {
    pub id: String,
    pub email: String,
}

// Autorrellenado:
let u = User::create(attrs! { email: "a@b.com" }).await?;
// u.id es un UUID v7 nuevo.

// Los IDs aportados por quien llama siguen ganando (igual que HasUuids de Laravel).
let u = User::create(attrs! { id: "...", email: "..." }).await?;
```

Estrategias soportadas:

- `"uuid"` / `"uuid_v7"` - UUID v7 (ordenado por timestamp,
  recomendado; coincide con el `Str::uuid7()` por defecto de Laravel
  11+)
- `"uuid_v4"` - UUID aleatorio (coincide con `HasVersion4Uuids`)
- `"ulid"` - ULID de 26 caracteres en Crockford base32 minúscula

La macro emite un bloque `impl HasUniqueId for YourStruct` que expone
`UNIQUE_ID_KIND` y un gancho `new_unique_id()` que puedes sobrescribir
en el tipo para un generador propio (p. ej. IDs con prefijo como
`usr_<uuid>`).

### `find_or` / `find_or_new` / `create_or_first`

Completan la superficie del trait `FirstOrCreate`.

```rust
// Busca por PK; ejecuta el fallback si no se encuentra.
let user = User::find_or(id, || async {
    User::create(attrs! { id, name: "guest" }).await
}).await?;

// Busca por PK; construye una instancia sin guardar a partir de los valores
// por defecto si no se encuentra.
let user = User::find_or_new(id, attrs! { name: "draft" }).await?;
// aquí user.id == 0 - la instancia solo está en memoria.

// Inserción segura ante carreras: intenta crear y recurre a leer si hay conflicto.
let user = User::create_or_first(
    attrs! { email: "race@x.com" },
    attrs! { name: "race winner" },
).await?;
```

### Scope `without_touching`

El análogo en Suprnova del `Model::withoutTouching` de Laravel. Dentro
del scope, toda llamada a `model.touch().await` hace cortocircuito -
útil al ejecutar migraciones de datos o jobs por lotes que mutan
timestamps por otras vías.

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    // aquí las llamadas a .touch() son no-ops.
    for post in posts {
        post.touch().await?;
    }
}).await;
```

El scope está respaldado por `tokio::task_local`, así que las solicitudes
concurrentes en otras tareas siguen respetando su propio scope (o su
ausencia).
`without_touching` también suprime la [cascada de actualización de
propietarios](#actualizar-el-propietario): un hijo guardado dentro del scope deja
sin tocar a cada propietario nombrado en su lista `touches`.

`without_touching_on::<Post, _, _>(fut)` es la forma por tipo, equivalente
a `Model::withoutTouchingOn([Post::class], $cb)` de Laravel. Dentro de
ella, `post.touch()` y cualquier cascada que actualizaría un `Post` no
hacen nada, mientras los propietarios de cualquier otro tipo siguen
actualizándose:

```rust
use suprnova::eloquent::without_touching_on;

without_touching_on::<Post, _, _>(async {
    // Los guardados de Comment aquí no actualizan sus propietarios Post;
    // un propietario Video del mismo comentario sí se actualiza.
    comment.save().await
}).await?;
```

Los scopes se anidan y ambos usan `tokio::task_local`.

## Siguiente

- [Relaciones de Eloquent](eloquent-relationships.md) - inmersión
  profunda en cada tipo de relación, el registro morph, y la
  reducción del enum polimórfico
- [Colecciones de Eloquent](eloquent-collections.md) - la superficie
  completa de `Collection<T>`, la separación genérico-frente-a-modelo,
  y el streaming de `LazyCollection<M>`
- [Conversiones, accesores y mutadores de Eloquent](eloquent-mutators.md) -
  los 22 casts integrados más el override en tiempo de ejecución
  `casts!`
- [Serialización de Eloquent](eloquent-serialization.md) - `to_array`,
  `to_json`, hidden / visible / appends, terminales filtrados
- [Fábricas de Eloquent](eloquent-factories.md) - instancias de
  modelo aleatorizadas para tests y sembradores

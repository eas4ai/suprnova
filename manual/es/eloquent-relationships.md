# Relaciones de Eloquent

[Eloquent](eloquent.md) cubre la superficie de relaciones del día a
día - sintaxis de declaración, la tabla de opciones, el encadenado
básico por tipo. Este capítulo es la inmersión profunda específica
de relaciones: cómo una llamada `user.posts()` se resuelve realmente
en SQL, cómo el cargador anticipado evita el N+1, cómo el motor de
existencia (`has` / `where_has` / `where_belongs_to`) renderiza
subconsultas `EXISTS` correlacionadas, cómo el polimorfismo sobrevive
a la falta de enlace estático tardío de Rust, y qué se cae del
sistema de tipos cuando los once tipos de relación tienen que
coexistir en un solo trait.

Si eres nuevo con Eloquent en Suprnova, lee primero
[Eloquent](eloquent.md#relationships) - esa página enseña la
sintaxis de declaración. Esta página asume que ya tienes un modelo
con un bloque `relations = { ... }` y quieres entender qué hay
debajo.

## Los once tipos de relación

Todo tipo de relación en [`RelationKind`][relations] es uno de estos:

| Tipo                  | Lado       | Cardinalidad | Entre familias | Pivot |
|-----------------------|------------|-------------|-----------------|-------|
| `HasOne<R>`           | padre      | uno         | no              | - |
| `HasMany<R>`          | padre      | muchos      | no              | - |
| `BelongsTo<R>`        | hijo       | uno         | no              | - |
| `BelongsToMany<R, P>` | cualquiera | muchos      | no              | sí    |
| `HasOneThrough<B, R>` | padre      | uno         | no              | - |
| `HasManyThrough<B, R>`| padre      | muchos      | no              | - |
| `MorphOne<R>`         | padre      | uno         | sí              | - |
| `MorphMany<R>`        | padre      | muchos      | sí              | - |
| `MorphTo`             | hijo       | uno         | sí (n objetivos) | - |
| `MorphToMany<R, P>`   | padre      | muchos      | sí              | sí    |
| `MorphedByMany<R, P>` | pareja m2m | muchos      | sí (inverso)    | sí    |

"Entre familias" significa que el *tipo* de la fila relacionada
varía - un `Comment` podría pertenecer a un `Post` o a un `Video`,
no solo a una tabla padre fija. Eso es polimorfismo, y Suprnova lo
maneja vía el [registro morph](#el-registro-morph) más un enum por
familia.

[relations]: https://docs.rs/suprnova

### Qué emite la macro

Cuando escribes:

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

`#[suprnova::model]` se expande en cinco cosas para `posts`:

1. **Método de relación** - `fn posts(&self) -> HasMany<Self, Post>`.
   Devuelve un envoltorio perezoso que lleva `self.id` más metadatos
   de FK; todavía no corre ningún SQL.
2. **Accesor de cargado** - `fn posts_loaded(&self) -> &[Post]`. Lee
   de la caché de precarga después de `User::with(["posts"])`.
   Slice vacío cuando no corrió ninguna carga anticipada.
3. **Accesor de conteo** - `fn posts_count(&self) -> u64`. Lee de la
   misma caché después de `User::with_count(["posts"])`.
4. **Rama del dispatcher** - rama de match dentro del método
   inherente `__eager_load` del modelo. El cargador anticipado busca
   `"posts"` y corre la consulta `IN`.
5. **Entrada de inventario** - un
   `inventory::submit!(RelationEntry { ... })` para que la relación
   sea enumerable en tiempo de ejecución (herramientas de admin, el
   motor de existencia, el dispatcher morph, todos recorren esto).

Nunca ves (4) ni (5). Impulsan el resto de este capítulo.

## Resolución perezosa: cómo `user.posts()` se convierte en SQL

`user.posts()` devuelve un envoltorio `HasMany<User, Post>`, no un
resultado de consulta. El envoltorio guarda el valor de PK del padre
más el nombre de la columna FK, y un `Builder<Post>` prefiltrado con
`WHERE posts.user_id = ?` ya aplicado. Nada ha tocado la base de
datos todavía.

```rust
use suprnova::Direction;

// Sin SQL.
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

La superficie de API dual
([Eloquent → Nota sobre nombres](eloquent.md#naming-note-dual-api))
se respeta sobre el envoltorio: tanto `.filter("col", v)` como
`.db_where("col", v)` funcionan, de forma idéntica. La superficie
encadenable en `HasOne` / `HasMany` / `MorphOne` / `MorphMany` cubre
`filter` / `db_where` / `order_by` / `latest` / `oldest` / `limit` /
`take`. Las relaciones Through y las m2m morph exponen solo sus
métodos terminales - pasan por costuras de SQL escritas a mano, no
por un `Builder<R>`, así que no pueden componerse con la cadena
estándar. Ver [relaciones Through](#hasonethrough-y-hasmanythrough)
y [m2m polimórfico](#morphtomany-y-morphedbymany) más abajo.

### Las eliminaciones suaves se propagan

Cuando el tipo relacionado implementa
[`SoftDeletes`](eloquent.md#soft-deletes-flag), el envoltorio de
relación hereda su scope global. `user.posts().get()` oculta los
posts descartados de la misma forma que lo hace
`Post::query().get()`. Tres forwarders lo atraviesan:

```rust
let alive = user.posts().get().await?;                 // por defecto: solo vivos
let all = user.posts().with_trashed().get().await?;    // vivos + descartados
let dead = user.posts().only_trashed().get().await?;   // solo descartados
```

`with_trashed` / `only_trashed` existen en `HasOne`, `HasMany`,
`MorphOne`, `MorphMany`, `BelongsToMany`, `MorphToMany`,
`MorphedByMany`, y `BelongsTo`. Están deliberadamente ausentes de
`HasOneThrough` y `HasManyThrough` - ver la
[brecha de eliminación suave en Through](#eliminaciones-suaves-en-relaciones-through-v1)
más abajo.

## Uno a uno: `HasOne` y `BelongsTo`

`HasOne` es el padre diciendo "este hijo tiene una columna que
apunta hacia mí". `BelongsTo` es el hijo diciendo "tengo una columna
que apunta hacia el padre". Ambos corren un único
`WHERE fk = ? LIMIT 1` y devuelven `Option<R>`.

```rust
// HasOne - padre → hijo
let profile: Option<Profile> = user.profile().first().await?;

// BelongsTo - hijo → padre
let owner: Option<User> = profile.user().first().await?;
```

`BelongsTo` añade una comodidad con forma de Laravel que las otras
no necesitan: `with_default`. Cuando la FK del hijo es null O la
fila padre fue borrada, `first()` devuelve el sustituto del closure
en lugar de `None`:

```rust
#[model(table = "comments", relations = {
    author: BelongsTo<User> {
        with_default = || User { id: 0, name: "Guest".into(), .. },
    },
})]
pub struct Comment { /* ... */ }

// Siempre devuelve Some(User) - o el autor real o el sustituto Guest.
let display: Option<User> = comment.author().first().await?;
```

El dispatcher de carga anticipada respeta el mismo fallback - las
rutas perezosa y anticipada comparten el comportamiento por defecto,
así que el código de plantilla que imprime
`comment.author_loaded()[0].name` no tiene que ramificar.

## Uno a muchos: `HasMany`

`HasMany` es la relación de cardinalidad múltiple del lado del
padre. El terminal `.get()` devuelve una
[`Collection<R>`](eloquent.md#collections) - el envoltorio con forma
de Laravel alrededor de `Vec<R>` - así que la superficie consciente
del modelo se compone:

```rust
let titles = user.posts()
    .order_by("created_at", Direction::Desc)
    .limit(10)
    .get()
    .await?
    .pluck::<String>("title");
```

`latest()` y `oldest()` son azúcar para
`order_by("created_at", Direction::Desc)` y `Asc` respectivamente -
solo resuelven contra modelos que declaran una columna
`created_at`, que la macro `#[suprnova::model]` añade
automáticamente siempre que los timestamps están activos (el valor
por defecto).

## Muchos a muchos: `BelongsToMany<R, P>` y el pivot de primera clase

`BelongsToMany` es muchos a muchos a través de una tabla de unión.
El pivot de Suprnova es en sí mismo un struct `#[suprnova::model]`
con sus propias migraciones, sus propios accesores, sus propios
eventos. Esa es la divergencia - ver
[más abajo](#por-qué-suprnova-diverge-el-pivot-es-un-modelo-real).

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

Los mutadores corren contra la fila pivot:

```rust
use suprnova::attrs;

user.roles().attach(role.id).await?;
user.roles().attach_with(role.id, attrs! { assigned_at: now }).await?;
user.roles().detach(role.id).await?;
user.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
```

`sync` lee el conjunto pivot actual, calcula
`attach_set = ids - current` y `detach_set = current - ids`, y
ejecuta las diferencias dentro de una transacción. Los duplicados en
el conjunto de entrada colapsan por su forma de string JSON, así que
`sync([1, 1, 2])` hace lo que esperas.

La lectura pasa por la estrategia de dos consultas:

```rust
// Consulta 1: SELECT roles.*, role_user.* vía INNER JOIN, acotada por user_id.
// Consulta 2: SELECT role_user.* para el mismo join, para estampar __pivot por fila.
let roles = user.roles().get().await?;

// Cada role lleva el contexto pivot que la macro deja accesible:
for r in &roles {
    let pivot = r.pivot::<RoleUser>().expect("loaded via BelongsToMany");
    println!("{} assigned at {:?}", r.name, pivot.assigned_at);
}
```

### Por qué Suprnova diverge: el pivot es un modelo real

El pivot de Laravel es una bolsa opaca por atributo
(`$role->pivot->note`). Suprnova te exige declarar el struct pivot
porque el sistema de tipos de Rust necesita las columnas en tiempo
de compilación - y una vez que pagaste por esa declaración, el pivot
recibe el mismo trato `#[suprnova::model]` que cualquier otra tabla:
migraciones, eventos, observadores, factories, eliminación suave.
`r.pivot::<RoleUser>()` devuelve una referencia tipada; sin búsquedas
de atributo con clave de string, sin sorpresas en tiempo de
ejecución cuando una columna está mal escrita.

El costo es un struct extra por tabla pivot. El beneficio es que el
pivot puede llevar comportamiento - lógica de dominio, reglas de
validación, columnas de auditoría - sin escapar hacia SQL en crudo.

## `HasOneThrough` y `HasManyThrough`

Relaciones de dos saltos: `A → B → C` donde `B` es un modelo
intermedio cuya FK apunta a `A`, y `C` es el objetivo final cuya FK
apunta a `B`. Ejemplo clásico: `Country` tiene muchos `User`;
`User` tiene muchos `Post`; `Country::posts()` salta ambos tramos en
un único viaje de ida y vuelta SQL.

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<User, Post>,
})]
pub struct Country { /* ... */ }

// Un único INNER JOIN: SELECT posts.* FROM posts
//   INNER JOIN users ON posts.user_id = users.id
//   WHERE users.country_id = ?
let posts: Collection<Post> = country.posts().get().await?;
```

`HasOneThrough` tiene la misma forma pero `.get()` devuelve
`Option<C>` (igualando la semántica de cardinalidad uno) y `.first()`
es su alias.

Los envoltorios Through solo exponen sus terminales - `get` /
`first` / `count` más los setters de clave (`first_key` /
`second_key` / `local_key` / `second_local_key`). No fluyen a través
de un `Builder<C>`, así que no pueden encadenar `.filter(...)` ni
`.order_by(...)`. Si necesitas filtrar a través del join, recurre a
dos saltos de relación explícitos.

### Eliminaciones suaves en relaciones Through (v1)

Las relaciones Through usan SQL `INNER JOIN` en crudo en vez del
pipeline `Builder<C>`, así que el scope global de eliminación suave
que `C::query()` instalaría (`WHERE c.deleted_at IS NULL`) **no** se
aplica. Tanto los intermedios descartados como los objetivos
descartados participan en el JOIN.

Esto diverge de Laravel, donde `hasManyThrough` filtra tanto `B`
como `C` por `deleted_at IS NULL` cuando los modelos declaran
`SoftDeletes`. Hasta que llegue la corrección, quien necesite
lecturas Through acotadas debería encadenar las dos relaciones
explícitamente:

```rust
// En vez de country.posts().get():
let users = country.users().get().await?;
let user_ids: Vec<i64> = users.iter().map(|u| u.id).collect();
let posts = Post::query().filter_in("user_id", user_ids).get().await?;
// Los scopes de eliminación suave de User y de Post aplican ambos.
```

## Relaciones polimórficas

Una FK polimórfica es un par de columnas: `<name>_id` (la clave
primaria de la fila) más `<name>_type` (un string que identifica
*en qué tabla* vive ese id). Una fila `Comment` puede apuntar a un
`Post` o a un `Video` sin añadir ni una columna `post_id` ni una
`video_id`.

Suprnova incluye cuatro tipos polimórficos: `MorphOne`, `MorphMany`,
`MorphTo`, y el par m2m `MorphToMany` / `MorphedByMany`. Todos
comparten una misma pieza de infraestructura:
[el registro morph](#el-registro-morph).

### `MorphOne<R>` y `MorphMany<R>` - lado del padre

`MorphOne` y `MorphMany` reflejan a `HasOne` y `HasMany` pero
superponen el discriminador `<name>_type` encima. El builder interno
está prefiltrado con `WHERE <name>_id = ? AND <name>_type = ?`, así
que los hijos polimórficos que apuntan hacia *otras* familias nunca
aparecen en el resultado.

```rust
#[model(table = "posts", morph_type = "post", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Video { /* ... */ }

let post_comments = post.comments().get().await?;     // solo commentable_type = 'post'
let video_comments = video.comments().get().await?;   // solo commentable_type = 'video'
```

`morph_type = "post"` es el string que el padre registra en la
columna `commentable_type` del hijo. El valor por defecto es el
nombre del struct en snake-case, pero sobrescribirlo es la decisión
correcta para cualquier modelo que vayas a lanzar a producción - las
refactorizaciones que renombran tablas no deberían romper la clave
polimórfica.

### `MorphTo` y el enum por familia

`MorphTo` vive del lado de la tabla morph. El usuario declara la
*lista de objetivos* por adelantado:

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

La macro emite un enum por familia en el sitio de la declaración:

```rust
// Emitido por la macro - tú no escribes esto.
pub enum CommentableMorph {
    Post(Post),
    Video(Video),
    Unknown(String, i64),     // fallback para <name>_type no registrado
}
```

Y `comment.commentable()` devuelve un ayudante de fetch cuyo `.get()`
resuelve hacia el enum:

```rust
match comment.commentable().get().await? {
    CommentableMorph::Post(post) => println!("on post: {}", post.title),
    CommentableMorph::Video(video) => println!("on video: {}", video.url),
    CommentableMorph::Unknown(t, id) => {
        eprintln!("orphaned commentable_type={t} id={id}");
    }
}
```

### Por qué Suprnova diverge: el enum por familia

El `morphTo` de Laravel devuelve `mixed` - el despacho dinámico de
PHP resuelve el método en tiempo de ejecución. Rust no tiene enlace
estático tardío, así que Suprnova hace explícita la familia. Los
beneficios superan el costo de tipado:

- **`match` exhaustivo** - el compilador te avisa cuando llega un
  nuevo objetivo morph y se te olvidó manejarlo.
- **`Unknown(String, id)` es type-safe** - las filas huérfanas de una
  clase de modelo padre eliminada emergen como una variante, en
  lugar de causar pánico.
- **La lista de objetivos documenta el esquema** - leer la
  declaración `MorphTo` te dice cada tipo que puede estar del otro
  lado. No se requiere ninguna consulta a la base de datos para
  enumerarlos.

### Restricción v1: `MorphTo` solo admite `i64`

`MorphTo::morph_id` está fijado a `i64`. Los objetivos polimórficos
deben por lo tanto usar claves primarias `i64`, y la columna
`<name>_id` de la tabla morph también debe ser `i64`. Los modelos
cuya PK es `String` o `Uuid`-vía-string no pueden ser objetivos de
`MorphTo` en v1. v2 parametrizará el tipo del ID morph para que se
acepte el retículo completo de PK (`i64` / `String` / `Uuid`).

Esta es una restricción exclusiva del lado inverso del polimorfismo.
`MorphOne` / `MorphMany` / `MorphToMany` / `MorphedByMany` funcionan
bien con cualquier forma de PK - leen directamente el `id` ya tipado
del padre.

### `MorphToMany` y `MorphedByMany`

Muchos a muchos polimórfico a través de un único pivot. Un lado es
"morphable" (`Post.tags()`, `Video.tags()` - ambos pasan por el
mismo pivot `taggables`). El otro es la pareja m2m compartida
(`Tag.posts()`, `Tag.videos()` - mismo pivot, recorrido en la otra
dirección).

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

`MorphToMany` es el lado que muta - `attach` / `attach_with` /
`detach` / `sync` viven todos ahí. `MorphedByMany` es de solo
lectura: cada llamada a `tag.posts()` devuelve solo taggables tipados
`Post`, cada `tag.videos()` devuelve solo taggables tipados `Video`,
sin mezclar en una sola colección.

Muta desde el lado morphable:

```rust
post.tags().attach(rust_tag.id).await?;
post.tags().sync([rust_tag.id, async_tag.id]).await?;
```

Lee desde cualquiera de los dos:

```rust
let tags_on_post: Collection<Tag> = post.tags().get().await?;
let posts_with_rust_tag: Collection<Post> = rust_tag.posts().get().await?;
```

## El registro morph

Todo struct anotado con `#[suprnova::model(morph_type = "...")]`
emite una entrada [`MorphTypeEntry`][morph] vía
`inventory::submit!` en tiempo de compilación. El registro impulsa
tres cosas:

1. **Despacho del enum por familia** - `MorphTo.get()` lee el string
   `<name>_type` de la fila hija y lo busca para encontrar la
   variante de enum correcta.
2. **Filtrado de objetivo de `MorphedByMany`** -
   `target_morph_type = "post"` resuelve a través del registro para
   asegurar que el string de tipo sea real.
3. **Comprobaciones de coherencia** - `find_morph_type("post")`
   devuelve `None` si ningún modelo se ha registrado con ese string,
   distinguiendo "deliberadamente sin registrar" de "typo".

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

Los modelos sin un atributo `morph_type = "..."` deliberadamente no
se registran - el registro es opt-in. Un modelo `User` no
polimórfico no le aporta nada, que es lo que hace que
`find_morph_type("user")` devolviendo `None` sea una señal útil.

## Consultar por existencia de relación

`has` / `where_has` / `doesnt_have` / `where_relation` /
`where_belongs_to` forman el motor de existencia de relación de
Suprnova. Todos se renderizan como subconsultas `EXISTS (...)`
correlacionadas contra el **propio SELECT del padre** - sin JOIN,
sin filas padre duplicadas, sin GROUP BY.

```rust
// Usuarios con al menos un post.
let with_posts = User::query().has("posts").get().await?;

// Usuarios con al menos tres posts.
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// Usuarios con al menos un post PUBLICADO.
let published_authors = User::query()
    .where_has::<Post, _>("posts", |q| q.filter("published", true))
    .get()
    .await?;

// Usuarios sin NINGÚN post.
let empty_users = User::query().doesnt_have("posts").get().await?;

// Usuarios sin posts en BORRADOR (pueden seguir teniendo publicados).
let clean = User::query()
    .where_doesnt_have::<Post, _>("posts", |q| q.filter("published", false))
    .get()
    .await?;

// Atajo: where_has + una sola columna == match.
let same = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// where_belongs_to - FK directa = ? sobre ESTA tabla (no se necesita
// EXISTS, porque la FK vive en la fila hija).
let mine = Post::query()
    .where_belongs_to("author", user.id)
    .get()
    .await?;
```

### Cómo funciona

El motor recorre el inventario de relaciones al construir la
consulta. Para cada relación nombrada, toma el `RelationEntry` y
renderiza la forma SQL apropiada según el tipo:

- `HasOne` / `HasMany` / `MorphOne` / `MorphMany` →
  `EXISTS (SELECT 1 FROM child WHERE child.<fk> = parent.<pk>)`.
  Los tipos morph añaden `AND child.<name>_type = '<parent_morph_type>'`.
- `BelongsTo` →
  `EXISTS (SELECT 1 FROM parent WHERE parent.<pk> = child.<fk>)`.
- `BelongsToMany` / `MorphToMany` → unen a través del pivot:
  `EXISTS (SELECT 1 FROM pivot WHERE pivot.<parent_fk> = parent.<pk> ...)`.
- Relaciones Through → unen a través del intermedio.

La forma de closure (`where_has::<R, _>(rel, |q| ...)`) construye un
`Builder<R>` interno; cualesquiera términos WHERE que ese builder
produzca aterrizan dentro del cuerpo de la subconsulta. La
numeración de placeholders es monótona a través de todo el
statement, así que el motor funciona correctamente con parámetros de
Postgres con estilo `$1`.

`where_belongs_to` es la única excepción que no renderiza un EXISTS.
La FK de belongs-to vive en la *propia* fila del padre, así que un
`WHERE child.<fk> = ?` directo es exactamente el SQL correcto - no
se necesita subconsulta. Si el nombre de la relación es desconocido
para el inventario del padre, el motor emite `WHERE 1 = 0` para que
la consulta devuelva de forma segura ningún resultado.

### Por qué esto gana a LEFT JOIN

El motor `has` / `whereHas` más antiguo de Laravel solía emitir
JOINs y filas padre duplicadas; la reescritura a EXISTS correlacionado
llegó en Laravel
9. Suprnova incluye EXISTS desde el primer día. Las
ventajas: sin duplicados en el conjunto de resultados, sin
workarounds de GROUP BY para agregados, sin necesitar `DISTINCT`, y
el optimizador de la base de datos ve una subconsulta real en vez de
un JOIN a través del cual no puede empujar predicados. Para
`has_count(rel, ">=", n)` el motor renderiza directamente
`(SELECT COUNT(*) FROM child WHERE ...) >= n` - una consulta, un
plan.

## Carga anticipada - agregados `with`, `with_count`, `with_*`

El `user.posts().get()` perezoso hace una consulta por padre. Eso es
N+1 cuando tienes muchos usuarios:

```rust
// Mal: 1 consulta para usuarios + 100 consultas para posts.
let users = User::query().limit(100).get().await?;
for u in &users {
    let posts = u.posts().get().await?;
    /* ... */
}
```

`with(["posts"])` colapsa eso a dos consultas en total - sin
importar la cantidad de padres:

```rust
// Bien: 1 consulta para usuarios + 1 consulta IN para todos los posts.
let users = User::query()
    .with(["posts"])
    .limit(100)
    .get()
    .await?;

for u in &users {
    for post in u.posts_loaded() {       // lee de la caché, sin SQL
        println!("{}: {}", u.name, post.title);
    }
}
```

Las rutas anidadas también funcionan - los nombres de relación
separados por puntos recorren en profundidad:

```rust
let users = User::query()
    .with(["posts.comments.author"])
    .get()
    .await?;
// 4 consultas: users, posts IN users.id, comments IN posts.id, authors IN comments.user_id.
```

### `with_count` y agregados

`with_count` añade un agregado `COUNT(*) GROUP BY parent_fk` por
relación, cargado junto a los padres - una consulta extra por
relación:

```rust
let users = User::query().with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}
```

Se pueden apilar cuatro variantes de agregado: `with_sum`,
`with_avg`, `with_min`, `with_max`. La forma de la clave de caché es
`<rel>_<kind>_<col>`, así que apilar varios agregados sobre la
misma relación no colisiona:

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

Consulta
[Eloquent → Carga anticipada → Diseño de la caché](eloquent.md#cache-layout)
para el contrato de almacenamiento completo.

### Cargas anticipadas restringidas - `with_where`

`with_where` filtra qué filas hijas aterrizan en la caché de
precarga sin perder a los padres que no tienen hijos coincidentes:

```rust
use suprnova::Builder;

let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// El posts_loaded() de cada u contiene solo posts publicados.
// Los usuarios con cero posts publicados igual aparecen en el
// conjunto de resultados - su posts_loaded() devuelve un slice vacío.
```

`with_where` difiere de `where_has` en intención: `where_has` filtra
el conjunto de padres ("usuarios que tienen al menos un post
publicado"); `with_where` filtra la caché de precarga ("para todos
los usuarios, carga solo sus posts publicados"). Usa ambos juntos
cuando quieras los dos efectos.

El predicado es un `Fn`, no un `FnOnce`, así que un builder que
lleve uno puede clonarse y correr más de una vez. Un closure que
quiera consumir un valor capturado debería clonarlo internamente:

```rust
let wanted = vec!["rust".to_string(), "web".to_string()];
let users = User::query()
    // `wanted.clone()` por dentro, no un `move` de `wanted` en sí - el
    // closure puede correr una vez por cada clon del builder.
    .with_where(("posts", move |q: Builder<Post>| q.filter_in("tag", wanted.clone())))
    .get()
    .await?;
```

### Clonar una consulta conserva su plan de carga anticipada

`Builder` es `Clone`, y el clon lleva el plan de carga anticipada
consigo, así que el patrón "construye una consulta base, deriva
varias a partir de ella" funciona:

```rust
let base = User::query().with(["posts"]).filter("active", true);

let first_page = base.clone().limit(20).get().await?;
let total = base.count().await?;
// Las filas de first_page tienen posts_loaded() poblado.
```

### Por qué Suprnova diverge

El `$query->with(...)` de Laravel clona libremente porque los
arrays de PHP se copian al asignar. Rust tiene que decir qué
significa un clone para un closure con el tipo borrado, y hasta la
v0.7.2 Suprnova respondía descartando el plan - el clone tenía
éxito, la consulta tenía éxito, y las relaciones simplemente estaban
ausentes. Compartir el predicado a través de un `Arc` hace que el
clone sea total, al costo del bound `Fn` de arriba.

La carga anticipada dentro de `chunk` / `chunk_by_id` / `lazy` sigue
siendo un error estrepitoso en lugar de un N+1 silencioso por chunk.
Vuelve a aplicar `.with(...)` dentro del closure por chunk cuando lo
quieras.

### Cargar sobre colecciones ya obtenidas

Cuando obtienes una `Collection<M>` sin un plan de carga anticipada,
puedes adjuntar uno después del hecho:

```rust
let mut users = User::query().get().await?;

users.load(["posts"]).await?;                 // incondicional
users.load_missing(["posts.comments"]).await?; // omite lo que ya está cargado
```

`load_missing` recorre la caché `__eager` de cada padre y solo
dispara la consulta IN para las filas que aún no cargaron la
relación. Útil en bucles donde algunos padres se cargaron
anticipadamente antes en la solicitud y otros no.

### Excluir - `without`

`without` quita relaciones nombradas del plan de carga anticipada,
útil cuando un scope base añade valores por defecto que no quieres
para esta llamada:

```rust
let users = User::query()
    .with(["profile", "posts", "team"])
    .without(["team"])     // quita team del plan
    .get()
    .await?;
```

## Actualizar propietarios

Un hijo puede declarar que escribirlo debe actualizar el
`updated_at` de su propietario:

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

Solo se pueden actualizar relaciones `BelongsTo`: la fila actualizada
debe poder identificarse a partir de una columna del hijo, exactamente
lo que proporciona el lado propietario. El framework resuelve el
propietario mediante el registro de relaciones, por lo que la
actualización cuesta un `UPDATE` y ningún `SELECT`.

Los propietarios que desactivan las marcas de tiempo
(`#[model(timestamps = false)]`), a los que se llega mediante una clave
foránea `NULL` o que están eliminados de forma lógica se omiten
silenciosamente. Suprime la cascada para un bloque de trabajo con
`without_touching` (todos los propietarios) o
`without_touching_on::<Post, _, _>` (un tipo). Consulta la semántica
completa en [Eloquent - Actualizar propietarios](eloquent.md#parent-touching).

## La vía de escape

Cuando una relación no encaja en ninguno de los once tipos - árboles
recursivos, polimorfismo a través de claves que no son id, pivots de
tres vías, cualquier cosa hecha a medida - escribe el método a mano.
La macro no lo impide; simplemente no obtienes el accesor de cargado
ni la rama del dispatcher de carga anticipada para esa relación.

```rust
impl User {
    /// Personalizado: el post más reciente sin importar la forma de la FK.
    pub async fn latest_post(&self) -> Result<Option<Post>, FrameworkError> {
        Post::query()
            .filter("user_id", self.id)
            .latest()
            .first()
            .await
    }
}
```

El trade-off es explícito: los métodos escritos a mano no aparecen
en el inventario de `relations()`, el motor de existencia no sabe
nada de ellos, y el cargador anticipado no puede incluirlos en un
plan. Para casos puntuales eso está bien. Para cualquier cosa que
quisieras poder `with(["..."])`, decláralo como un tipo de relación
propiamente dicho, aunque tengas que usar las opciones de la macro
para forzarlo a esa forma.

## Siguiente

- [Eloquent](eloquent.md) - la superficie de modelo del día a día; la
  sintaxis de declaración de relaciones vive ahí.
- [Base de datos](database.md) - conexiones, transacciones,
  multi-driver, la capa inferior sobre la que todo se apoya.
- [Migraciones](migrations.md) - el lado de esquema de las columnas
  FK que estas relaciones necesitan que existan.
- [Query Builder](eloquent.md#query-builder-dual-api) - la superficie
  de API dual hacia la que reenvían los envoltorios de relación.
- [Recursos de Eloquent](eloquent-resources.md) - convertir
  relaciones cargadas en payloads JSON:API para la respuesta.

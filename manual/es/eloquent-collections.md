# Colecciones de Eloquent

`Collection<T>` es el tipo de colección con forma de Laravel de
Suprnova - el valor de retorno de `Builder::get`, `Model::all`, cada
`pluck`, cada terminal de carga de relación que produce más de una
fila. Es un envoltorio delgado alrededor de `Vec<T>` que hace deref a
`&[T]`, así que todo método de slice existente (`.len()`, `.iter()`,
indexado, `.contains(&v)`) funciona sin cambios. Encima se apila la
superficie de Laravel: `map`, `filter`, `pluck`, `group_by`,
`sort_by`, `where_eq`, `sum`, `avg`, y demás.

Este capítulo es la referencia independiente de la superficie de
colecciones. El capítulo padre [API de Eloquent](eloquent.md) la
resume; este capítulo recorre cada método, el contrato de
préstamo-vs-consumo, la regla de serialización que sorprende si te la
saltas, y cuándo bajar a `Vec<T>` en su lugar.

## Tabla de contenidos

- [De dónde vienen las colecciones](#de-dónde-vienen-las-colecciones)
- [Los dos bloques impl](#los-dos-bloques-impl)
- [Superficie genérica - funciona sobre cualquier `Collection<T>`](#superficie-genérica-funciona-sobre-cualquier-collection-t)
- [Superficie consciente del modelo - `Collection<M>` donde `M: Model`](#superficie-consciente-del-modelo-collection-m-donde-m-model)
- [Carga anticipada sobre una colección](#carga-anticipada-sobre-una-colección)
- [Serialización - `to_array` frente a serde](#serialización-to-array-frente-a-serde)
- [Préstamo frente a consumo](#préstamo-frente-a-consumo)
- [`Collection` frente a `Vec`](#collection-frente-a-vec)
- [`LazyCollection<M>` - resultados en streaming](#lazycollection-m-resultados-en-streaming)
- [Por qué Suprnova diverge](#por-qué-suprnova-diverge)
- [Siguiente](#siguiente)

## De dónde vienen las colecciones

Cualquier terminal que devuelva más de una fila te entrega una
`Collection<M>`:

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::all().await?;
let admins: Collection<User> = User::query()
    .db_where("role", "=", "admin")
    .get()
    .await?;
let recent: Collection<User> = User::query()
    .order_by_desc("created_at")
    .limit(50)
    .get()
    .await?;
```

También puedes envolver cualquier `Vec<T>` que ya tengas:

```rust
let from_vec: Collection<User> = users_vec.into();
let from_vec2: Collection<User> = Collection::from_vec(users_vec);
let empty: Collection<User> = Collection::new();
```

`Collection<T>` implementa `Default`, `Clone`, `Serialize`,
`Deserialize`, `PartialEq`, e `IntoIterator` (tanto por valor como
por `&`). Es `Send` cuando `T: Send`.

## Los dos bloques impl

Los métodos de `Collection` se dividen en dos familias según el
parámetro de tipo.

```rust
impl<T> Collection<T> { /* métodos genéricos - funcionan para cualquier T */ }

impl<M> Collection<M> where M: Model { /* métodos de modelo con clave de string */ }
```

El bloque genérico te da `map`, `filter`, `reject`, `chunk`,
`first`, `last`, `unique`, y una versión basada en closures de cada
accesor de columna (`pluck_by`, `group_by_with`, `sort_with`,
`key_by_with`). Estos funcionan sobre `Collection<i32>`,
`Collection<String>`, `Collection<MyDto>`, lo que sea.

El bloque consciente del modelo añade azúcar con clave de string
(`pluck("name")`, `group_by("role")`, `sort_by("created_at")`,
`sum::<f64>("balance")`) que enruta por fila a través del accesor
`Model::field_value` emitido por la macro. Estos solo existen cuando
`T` implementa `Model`.

Elige la forma de closure cuando puedas - el comprobador de tipos
valida el acceso al campo. Elige la forma con clave de string cuando
estés igualando la sintaxis de Laravel, o cuando el nombre de la
columna sea un valor en tiempo de ejecución.

## Superficie genérica - funciona sobre cualquier `Collection<T>`

### Lectura

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9, 2, 6]);

nums.len();                         // 8
nums.is_empty();                    // false
nums.is_not_empty();                // true
nums.first();                       // Some(&3)
nums.last();                        // Some(&6)
nums.first_where(|n| **n > 3);      // Some(&4)
nums.last_where(|n| **n > 3);       // Some(&6)
nums.contains(&4);                  // true - de Deref<Target = [T]>
nums.contains_where(|n| *n > 5);    // true
```

`first_where` / `last_where` toman `&&T` porque el predicado corre a
través de `Iterator::find` sobre `Iter<'_, T>`. Deref dos veces
(`**n`).

### Transformar - consume `self`, devuelve una colección nueva

```rust
let doubled: Collection<i32>      = nums.clone().map(|n| n * 2);
let evens:   Collection<i32>      = nums.clone().filter(|n| n % 2 == 0);
let odds:    Collection<i32>      = nums.clone().reject(|n| n % 2 == 0);
let unique:  Collection<i32>      = nums.clone().unique();
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(3);
let taken:   Collection<i32>      = nums.clone().take(4);
let skipped: Collection<i32>      = nums.clone().skip(2);
let middle:  Collection<i32>      = nums.clone().slice(2, 4);
let flipped: Collection<i32>      = nums.clone().reverse();
let shuffled: Collection<i32>     = nums.clone().shuffle();
```

`map` cambia el tipo de elemento:

```rust
let labels: Collection<String> = nums.clone().map(|n| format!("n={n}"));
```

`each` ejecuta un efecto secundario y conserva la colección para
seguir encadenando (Suprnova diverge de Laravel aquí a propósito -
ver más abajo):

```rust
let kept = nums.clone()
    .each(|n| tracing::debug!(value = n, "processing"))
    .filter(|n| *n > 2)
    .take(3);
```

### Agrupar y ordenar con clave de closure

```rust
use std::collections::HashMap;

// Agrupa elementos por una clave derivada del closure.
let by_parity: HashMap<bool, Collection<i32>> =
    nums.clone().group_by_with(|n| n % 2 == 0);

// Indexa elementos por una clave derivada del closure (los
// duplicados posteriores sobrescriben).
let by_value: HashMap<i32, i32> =
    nums.clone().key_by_with(|n| *n);

// Ordena por un comparador derivado del closure.
let sorted_desc: Collection<i32> =
    nums.clone().sort_with(|a, b| b.cmp(a));

// Deduplica por una clave derivada del closure.
let unique_mod3: Collection<i32> =
    nums.clone().unique_by(|n| n % 3);

// Proyecta cada elemento mediante el closure hacia una colección nueva.
let strs: Collection<String> =
    nums.pluck_by(|n| n.to_string());
```

El sufijo `*_with` / `*_by` es la convención universal de "este
método toma un closure" en todo el bloque genérico. El bloque
consciente del modelo elimina el sufijo y toma un string de nombre
de columna en su lugar.

### Reducir y agregar

```rust
let sum: i32 = nums.clone().reduce(0, |acc, n| acc + n);  // 31
```

Para agregados numéricos tipados sobre colecciones de modelos, ver
`sum` / `avg` / `min` / `max` en la sección consciente del modelo -
funcionan sobre cualquier campo que deserialice a un tipo numérico.

### Operaciones de conjuntos

```rust
let a = Collection::from_vec(vec![1, 2, 3, 4]);
let b = Collection::from_vec(vec![3, 4, 5, 6]);

let joined = a.clone().concat(b.clone());    // [1,2,3,4,3,4,5,6]
let same   = a.clone().merge(b.clone());     // alias de concat
let only_a = a.clone().diff(b.clone());      // [1,2]
let common = a.clone().intersect(b.clone()); // [3,4]
```

`concat` / `merge` son alias - Laravel trae ambos nombres. `diff` /
`intersect` son O(n*m); si tienes colecciones grandes, proyecta
primero a un `HashSet`.

### Muestreo aleatorio

```rust
let one: Option<&i32>     = nums.random();        // toma prestado uno
let many: Collection<i32> = nums.clone().random_n(3); // elige 3
```

Ambos usan el RNG thread-local (`rand::rng()`). Pasa un RNG con
semilla fija a mano si necesitas determinismo en los tests.

## Superficie consciente del modelo - `Collection<M>` donde `M: Model`

Estos métodos solo existen cuando el tipo contenido es un modelo de
Suprnova. Enrutan las lecturas por fila a través del accesor
`Model::field_value(name)` emitido por la macro, que devuelve
`Option<serde_json::Value>`. Las filas cuyo campo no existe o no
deserializa hacia el tipo destino se omiten en silencio - igualando
el comportamiento de clave ausente de Laravel.

### Proyección

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let ids:    Collection<i64>    = users.pluck::<i64>("id");
```

`pluck` toma prestado (`&self`), así que la colección original
sigue disponible después. El parámetro tipado (`::<String>`) es el
tipo destino hacia el que se deserializa el valor JSON.

`pluck_keyed` produce un `HashMap<K, V>` a partir de dos columnas:

```rust
use std::collections::HashMap;

let email_by_id: HashMap<i64, String> =
    users.pluck_keyed::<i64, String>("id", "email");
```

Las filas posteriores sobrescriben a las anteriores para la misma
clave.

### Agrupar e indexar

```rust
use std::collections::HashMap;

let by_role: HashMap<String, Collection<User>> = users.group_by("role");
let by_id:   HashMap<String, User>             = users.key_by("id");
```

Ambos métodos convierten el valor de la columna a string para usarlo
como clave `String`. Una columna numérica `id` llega como `"1"` /
`"2"` - igualando el contrato de `groupBy('team_id')` de Laravel,
donde la salida siempre tiene clave de string sin importar el tipo
subyacente.

Si quieres claves tipadas, usa la forma de closure en el bloque
genérico:

```rust
let by_id: HashMap<i64, User> = users.key_by_with(|u| u.id);
```

### Filtrado

Los métodos `where_*` conscientes del modelo toman
`serde_json::Value` porque comparan contra la forma codificada en
JSON de la columna:

```rust
use serde_json::json;

let active: Collection<User>  = users.clone().where_eq("active", json!(true));
let admins: Collection<User>  = users.clone()
    .where_in("role", vec![json!("admin"), json!("owner")]);
let non_guests: Collection<User> = users.clone()
    .where_not_in("role", vec![json!("guest")]);
```

`where_eq` y `where_in` descartan las filas cuyo `field_value`
devuelve `None`. `where_not_in` *conserva* las filas donde el campo
está ausente - la negación de "está en el conjunto" es "no está en
el conjunto O está ausente".

### Ordenar

```rust
let by_name_asc:  Collection<User> = users.clone().sort_by("name");
let by_name_desc: Collection<User> = users.clone().sort_by_desc("name");
```

La comparación es best-effort entre formas de valor JSON: numérico
contra numérico y string contra string ordenan limpiamente dentro de
su tipo; las columnas heterogéneas mixtas caen a `Ordering::Equal`.
`None` ordena antes que cualquier valor presente (refleja
`NULL FIRST` de Postgres para ASC).

Ambos métodos clonan el `Vec<M>` subyacente antes de ordenar porque
el comparador toma prestado `m.field_value(field)` mientras
`sort_by` necesita `&mut [M]`. Si tienes un bucle exigente, ordena
con `sort_with` en el bloque genérico en su lugar - opera en el
sitio.

### Agregados

```rust
let total: f64           = users.sum::<f64>("balance");
let avg:   Option<f64>   = users.avg::<f64>("balance");
let lo:    Option<i64>   = users.min::<i64>("login_count");
let hi:    Option<i64>   = users.max::<i64>("login_count");
```

`sum` devuelve `T::default()` cuando ninguna fila aporta un valor
(cero para tipos numéricos). Las otras tres devuelven `None` para
que quien llama no divida por cero ni compare contra un valor por
defecto fantasma.

El parámetro tipado (`::<f64>`) es el destino de deserialización
JSON. Elige el tipo numérico más amplio que tu columna razonablemente
use - `i64` para columnas enteras, `f64` para decimal/float,
`chrono::DateTime<Utc>` para timestamps, etc.

## Carga anticipada sobre una colección

Cuando ya tienes una `Collection<M>` y quieres cargar relaciones
sobre cada fila, usa `load` / `load_missing`:

```rust
let mut users: Collection<User> = User::query().get().await?;
users.load(["posts.comments"]).await?;

for u in &users {
    for p in u.posts_loaded() {
        println!("{}: {} comments", p.title, p.comments_loaded().len());
    }
}
```

Ambos métodos toman `&mut self` (mutan la caché de precarga por
fila) y son `async`. Ambos aceptan la misma sintaxis de ruta con
puntos que acepta `Builder::with([...])` - `"posts"`,
`"posts.comments"`, `"posts.comments.author"`.

`load_missing` particiona por fila. Las filas que ya tienen la
relación en caché se dejan intactas; las que no, reciben la carga en
bloque:

```rust
let mut users: Collection<User> = User::query().with(["posts"]).get().await?;
// Algunas filas ya tienen posts en caché. load_missing solo toca el
// resto - y recorre recursivamente los posts ya en caché para
// buscar "comments".
users.load_missing(["posts.comments"]).await?;
```

La recursión corre en cada segmento de una ruta con puntos más
larga. Con `"a.b.c"`, cada fila se particiona en cada nivel: `a` se
carga solo donde falta, luego para las filas que ya tenían `a`, `b`
se carga solo donde falta sobre esas `a`, etc.

Ambos métodos respetan el enrutamiento de
`#[model(connection = "...")]` - resuelven la misma conexión desde la
que se cargó originalmente la fila.

## Serialización - `to_array` frente a serde

Esta es la única trampa en la superficie de colecciones. Léela con
cuidado.

`Collection<T>` deriva `Serialize`. Así que esto funciona:

```rust
let json: String = serde_json::to_string(&users)?;
```

Pero - la implementación general `Serialize for Vec<T>` de serde
llama a `T::serialize` directamente sobre cada elemento. Eso
**salta** el override de `Model::to_array()` que emite la macro
`#[suprnova::model]`. Lo que significa que salta tus atributos de
modelo `hidden = ["password"]`, `visible = [...]`, y
`appends = [...]`.

Si tu modelo tiene campos ocultos, **no** serialices la colección a
través de serde. Usa `to_array()` o `to_json()`:

```rust
let value: serde_json::Value = users.to_array();
let body:  String            = users.to_json();
```

Ambos métodos enrutan a través de `Model::to_array()` para cada
fila, así que el pipeline de filtros por modelo se aplica - los
campos ocultos siguen ocultos, las listas blancas de visibilidad se
respetan, y los `appends` guiados por accesor aparecen.

La misma advertencia aplica a cualquier cosa que llame a
`serde_json::to_value(&collection)` por debajo: `Inertia::render`
cuando metes una colección en props, `JsonApi`/`Resource` si les
entregas modelos en crudo en vez de structs de recurso, expedidores
de logs que codifican sus payloads con serde. El patrón seguro es
convertir a través de un tipo de recurso
([Recursos JSON:API](eloquent-resources.md)) o a través de
`to_array()` antes de que el valor llegue a cualquier ruta de código
de serde.

Para colecciones de tipos que no son modelos (`Collection<MyDto>`,
`Collection<String>`) la ruta de serde está bien - el problema solo
aplica cuando `T` es un struct `#[suprnova::model]` con
hidden/visible/appends declarados.

## Préstamo frente a consumo

Los métodos se dividen limpiamente en dos contratos:

| Toma | Métodos |
|---|---|
| `&self` (préstamo) | `len`, `is_empty`, `is_not_empty`, `first`, `last`, `first_where`, `last_where`, `contains_where`, `random`, `as_slice`, `pluck_by`, `pluck`, `pluck_keyed`, `group_by`, `key_by`, `sum`, `avg`, `min`, `max`, `to_array`, `to_json` |
| `self` (consumo) | `map`, `filter`, `reject`, `each`, `reduce`, `chunk`, `take`, `skip`, `slice`, `reverse`, `shuffle`, `random_n`, `unique`, `unique_by`, `sort_with`, `sort_by`, `sort_by_desc`, `where_eq`, `where_in`, `where_not_in`, `concat`, `merge`, `diff`, `intersect`, `group_by_with`, `key_by_with`, `map_to_map` |
| `&mut self` | `load`, `load_missing` |

Si quieres conservar la colección después de una llamada que
consume, haz `.clone()` antes de la llamada. `Collection<T>: Clone`
cuando `T: Clone`.

Un patrón práctico: primero lee, transforma al final:

```rust
let users: Collection<User> = User::all().await?;

// Lecturas por préstamo primero - la colección sigue viva después de cada una.
let total       = users.sum::<f64>("balance");
let avg         = users.avg::<f64>("balance");
let count_admin = users.iter().filter(|u| u.role == "admin").count();
let emails      = users.pluck::<String>("email");

// Ahora consume.
let admins: Collection<User> = users.where_eq("role", json!("admin"));
```

## `Collection` frente a `Vec`

El envoltorio es deliberadamente delgado. Las rutas de conversión
van en ambas direcciones y siguen siendo baratas:

```rust
let v: Vec<User>          = User::query().get().await?.into_vec();
let c: Collection<User>   = Collection::from(v);
let c2: Collection<User>  = Collection::from_vec(c.clone().into_vec());
```

`Deref<Target = [T]>` te da automáticamente cada método de slice.
Eso incluye:

```rust
let users: Collection<User> = User::all().await?;

users.len();             // método de slice
users.iter();            // método de slice
users[0].name.clone();   // indexado de slice
users.contains(&u);      // método de slice
users.binary_search(&u); // método de slice
&users[1..4];            // subscripting de slice
```

`IntoIterator` está implementado dos veces - para `Collection<T>`
(por valor) y `&Collection<T>` (por referencia), así que ambas
formas funcionan:

```rust
for user in &users {           // itera por &User
    /* ... */
}

for user in users.clone() {    // itera por User (consume)
    /* ... */
}
```

`DerefMut` solo produce `&mut [T]` - un slice, no un `Vec`. Eso
significa que la mutación en el sitio de los campos de un elemento
funciona:

```rust
let mut users: Collection<User> = User::all().await?;
for u in users.iter_mut() {
    u.last_seen_at = Some(Utc::now());
}
```

Pero la mutación de `Vec` propia (`push`, `pop`, `clear`,
`truncate`) no está disponible directamente sobre la colección -
llama primero a `into_vec()`:

```rust
let mut v = users.into_vec();
v.push(new_user);
let users: Collection<User> = Collection::from(v);
```

Eso es deliberado. La superficie de Laravel trata una colección como
una instantánea inmutable que se transforma con métodos encadenados;
la mutación propia de la secuencia interna es el contrato de `Vec`,
no el contrato de `Collection`.

### Cuándo bajar a `Vec`

Recurre a `into_vec()` cuando:

- Necesites métodos específicos de `Vec` (`push`, `pop`,
  `swap_remove`, `drain`, `with_capacity`).
- Estés entregando los datos a una API que toma `Vec<T>` por valor y
  no quieras el envoltorio en la firma.
- Estés guardando las filas a largo plazo en tu propio struct y la
  superficie de Laravel no te aporte nada.

Para todo lo demás - retornos de handler, transformaciones, props
de Inertia (siempre que respetes la
[regla de serialización](#serialización-to-array-frente-a-serde)) -
conserva la `Collection<T>`.

## `LazyCollection<M>` - resultados en streaming

`Collection<M>` materializa cada fila en memoria. Para conjuntos de
datos demasiado grandes para caber, el builder ofrece tres
terminales de streaming que devuelven `LazyCollection<M>` en su
lugar:

```rust
use suprnova::Model;

let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

| Método | Estrategia |
|---|---|
| `Builder::lazy()` | Paginación por cursor de PK con el tamaño de lote por defecto (1000) |
| `Builder::lazy_by_id(n)` | Paginación por cursor de PK con tamaño de lote `n` |
| `Builder::cursor()` | Alias de Laravel para `lazy()` |

Por debajo, `LazyCollection<M>` es un
`Pin<Box<dyn Stream<Item = Result<M, FrameworkError>> + Send>>`, pero
expone `.next().await` directamente para que no necesites importar
`futures::StreamExt`. Cada `.next()` dispara la siguiente entrega de
fila; el fetch por lotes subyacente solo corre cuando el buffer del
lote actual se agota, así que un consumidor lento no acumula filas.

El envoltorio es `Send` (así que cruza `tokio::spawn`) pero no
`Sync` - es un stream de un solo consumidor por construcción.

Consulta
[Eloquent - iteración en chunks y perezosa](eloquent.md#chunking-and-lazy-iteration)
para la guía completa sobre qué patrón de streaming elegir.

## Por qué Suprnova diverge

El `Illuminate\Support\Collection` de Laravel es mutable:
`$c->filter(...)` modifica el array interno del mismo objeto y
devuelve `$this` para encadenar. PHP no tiene ownership, así que ese
contrato es invisible.

Rust sí tiene ownership, y fingir que no lo tiene volvería
deshonesta la superficie de colecciones. Suprnova elige en su lugar
la forma de semántica de valor: cada transformación consume `self` y
devuelve una `Collection` nueva. Ves el costo en tu propio código -
si quieres conservar la original, haces `.clone()`. Si no, no lo
haces.

Esa elección se propaga por el resto de la superficie:

- **`each` devuelve `Self`** en vez de `&self` para que una llamada
  de efecto secundario (logging, métricas) no rompa una cadena. El
  `each` de PHP corre por su efecto y devuelve la colección; no
  podrías hacer `$c->each(...)->filter(...)` limpiamente sin volver
  a buscar. En Rust movemos `self` a través, manteniendo la cadena
  fluida.

- **Alternativas con clave de closure para cada método con clave de
  string.** `pluck_by`, `group_by_with`, `key_by_with`, `sort_with`,
  `unique_by`, `map_to_map`, `contains_where`. Los closures te dejan
  leer campos que el comprobador de tipos valida en vez de strings
  que el compilador no puede ver. Las formas con clave de string
  existen por paridad de sintaxis con Laravel y para nombres de
  columna decididos en tiempo de ejecución.

- **`sum` / `avg` / `min` / `max` toman parámetros tipados
  `::<T>`.** La versión en PHP de Laravel convierte al vuelo; en
  Rust, el destino de deserialización es parte de la llamada. Las
  filas cuyo valor no hace el viaje de ida y vuelta hacia `T` se
  omiten en silencio (igualando el comportamiento de clave ausente
  de Laravel), pero tú eliges el tipo con intención.

- **`Deref<Target = [T]>`, no `Deref<Target = Vec<T>>`.** Una
  `Collection` es conceptualmente una "instantánea de filas", no un
  buffer mutable. Los métodos de slice llegan a través de `Deref`;
  si quieres `push`/`pop`, `into_vec()` te da el `Vec` en crudo y
  elimina cualquier pretensión.

- **La serialización diverge al servicio de la corrección.**
  `to_array` y `to_json` enrutan a través de `Model::to_array()` para
  que hidden/visible/appends por modelo se apliquen; el bypass de la
  implementación general `Serialize for Vec` de serde está
  documentado como la [trampa](#serialización-to-array-frente-a-serde)
  que es. El `toArray()` de Laravel hace el mismo enrutamiento;
  simplemente tenemos que nombrar la brecha explícitamente porque
  los usuarios de Rust recurrirán a `serde_json::to_string` por
  reflejo.

El trade-off es exactamente el que Suprnova hace en todas partes: la
forma de superficie de Laravel, la semántica de valor de Rust.

## Siguiente

- [API de Eloquent](eloquent.md) - el capítulo padre, con el query
  builder, las relaciones, los scopes, y el ciclo de vida completo
  del modelo.
- [Recursos JSON:API](eloquent-resources.md) - los structs de
  recurso serializan colecciones a través de `IntoJsonResource` con
  campos dispersos y cadenas `?include=`; la forma correcta para
  cualquier colección que sale de tu API.
- [Frontend - Respuestas de Inertia](frontend-inertia-responses.md) -
  las reglas para entregar colecciones a props de Inertia sin caer en
  la trampa de serialización.
- [Validación](validation.md) - los payloads de solicitud
  frecuentemente producen vectores que envuelves en `Collection` para
  el procesamiento posterior.
- [Pruebas](testing.md) - patrones para verificar el contenido de
  una colección (longitud, elementos contenidos, orden) dentro de
  tests de handler y de modelo.

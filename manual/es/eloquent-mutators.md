# Conversiones, accesores y mutadores de Eloquent

Un cast media el límite entre lo que una columna guarda en disco y lo
que tu modelo lleva en memoria. Un accesor inventa un atributo
virtual a partir de las columnas que ya tienes. Un mutador enruta las
escrituras de un campo a través de tu propia transformación. Junto
con los timestamps autogestionados, son las cuatro piezas móviles que
convierten una fila plana en un valor de Rust tipado.

Este capítulo cubre toda la superficie de casts (cada tipo
incorporado, el override en tiempo de ejecución `casts!`, el cifrado
y el hashing), las macros de atributo `#[accessor]` y `#[mutator]`,
el contrato de auto-timestamps incluyendo `touch()` y
`without_touching`, y el evento de ciclo de vida `Replicating` que se
dispara cuando clonas un modelo con `replicate()`.

Para la superficie de modelo más amplia (`#[suprnova::model]`, query
builder, relaciones, observadores) consulta el capítulo
[API de Eloquent](eloquent.md). Para los eventos de ciclo de vida de
principio a fin, consulta [Eventos y oyentes](events.md). Para la
fachada de criptografía que usan los casts cifrados, consulta
[Cifrado](encryption.md).

## Cómo funcionan los casts

Todo cast es un struct que implementa el trait `Cast`:

```rust
pub trait Cast: Send + Sync {
    type Runtime;
    type Storage;

    fn to_storage(value: &Self::Runtime) -> Result<Self::Storage, FrameworkError>;
    fn from_storage(stored: &Self::Storage) -> Result<Self::Runtime, FrameworkError>;
}
```

`Runtime` es el tipo de Rust que escribes en tu struct de modelo
(`bool`, `chrono::NaiveDate`, `rust_decimal::Decimal`, tu propio
enum). `Storage` es el tipo que SeaORM ve en la columna (`i64` para
una columna booleana de SQLite, `String` para una fecha TEXT). Ambas
direcciones son falibles - el análisis temporal y decimal puede
rechazar una entrada mal formada - así que la macro propaga el
`Result` a través de `From<inner::Model>` y de la ruta de escritura
de `ActiveModel`.

Los casts son explícitos. Un campo `Vec<String>` no se convierte
implícitamente en `AsArray<String>` porque la inspección del tipo de
campo en tiempo de macro se rompería en el momento en que renombraras
un alias o importaras un `Vec` distinto. Declaras los casts en el
atributo de la macro:

```rust
use suprnova::{model, AsArray, AsBool, AsJson};

#[model(
    table = "posts",
    casts = {
        tags = AsArray<String>,
        published = AsBool,
        metadata = AsJson<serde_json::Value>,
    },
)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub tags: Vec<String>,
    pub published: bool,
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

La macro expande cada entrada `field = CastType` en llamadas a
`Cast::to_storage` y `Cast::from_storage` en cada lectura y
escritura. Nunca invocas el cast tú mismo - escribes el tipo en
tiempo de ejecución, el cast cablea la forma de la columna.

### Por qué Suprnova diverge

Laravel declara los casts como `protected $casts = ['tags' => 'array']`.
El string `'array'` resuelve hacia una clase mediante una búsqueda en
tiempo de ejecución, lo que significa que los nombres de cast viven
como strings sin tipar hasta que corren. Suprnova toma el tipo
directamente - `AsArray<String>` es un tipo de Rust real que la
macro comprueba en tiempo de compilación. Un typo en el nombre del
cast es un error de compilación, no una excepción en tiempo de
ejecución tres semanas después del deploy.

## Los casts primitivos

Cinco casts cubren los tipos escalares de SQL.

### `AsBool`

`bool` ↔ `INTEGER` (0 / 1). SQLite no tiene una columna booleana
nativa; Postgres y MySQL hacen ambos el viaje de ida y vuelta de
`i64` limpiamente a través del límite `Value::Int` de SeaORM. Una
sola forma de almacenamiento te deja usar el mismo cast contra
cualquier backend.

```rust
#[model(table = "settings", casts = { dark_mode = AsBool })]
pub struct Settings {
    pub id: i64,
    pub dark_mode: bool,
}
```

### `AsInt<I>`

Un entero más angosto (`i32`, `u32`, `i16`) ↔ `i64`. SeaORM almacena
los enteros como `i64` en la columna; el cast reduce el ancho al
leer y lo amplía al escribir. Los valores fuera de rango producen un
error de validación en tiempo de lectura en lugar de truncarse en
silencio.

```rust
#[model(table = "counters", casts = { age = AsInt<u32> })]
pub struct Counter {
    pub id: i64,
    pub age: u32,
}
```

Usa `AsInt<i64>` (u omite el cast) cuando el tipo en tiempo de
ejecución ya coincide con el de almacenamiento.

### `AsFloat`

`f64` ↔ `REAL`. Paso directo en ambas direcciones - el cast existe
por paridad de nombres con el cast `'float'` de Laravel; los
backends hacen el viaje de ida y vuelta de floats de forma nativa.

### `AsString`

`String` ↔ `TEXT`. También de paso directo; el cast existe para que
el override en tiempo de ejecución `Builder::with_casts(...)` pueda
borrarlo hacia un `DynCast` como cualquier otro cast.

### `AsDecimal<P>`

`rust_decimal::Decimal` ↔ `TEXT`. `P` es la precisión (número de
decimales); los valores se redondean a `P` posiciones en el camino
hacia el almacenamiento. El valor por defecto es `P = 4`. El
almacenamiento es un string de formato fijo, así que los viajes de
ida y vuelta son agnósticos de backend - el tipo de columna
`Decimal` nativo de SeaORM tiene una semántica de precisión distinta
en cada driver, y el viaje de ida y vuelta por string evita eso.

```rust
use rust_decimal::Decimal;
use suprnova::AsDecimal;

#[model(
    table = "ledger",
    casts = { amount = AsDecimal<2> },  // moneda, 2 decimales
)]
pub struct LedgerEntry {
    pub id: i64,
    pub amount: Decimal,
}
```

## Los casts temporales

Seis casts cubren fechas, datetimes, variantes inmutables y timestamps
de Unix. Todos los casts que no son de timestamp se almacenan como
`TEXT` (ISO-8601 / RFC-3339), así que el viaje de ida y vuelta funciona
en todos los drivers - SQLite almacena los datetimes como cadenas de
forma nativa, y Postgres / MySQL los aceptan a través del límite
`Value::String` de SeaORM.

### `AsDate`

`chrono::NaiveDate` ↔ `TEXT` (`YYYY-MM-DD`).

```rust
use chrono::NaiveDate;
use suprnova::AsDate;

#[model(table = "people", casts = { birthday = AsDate })]
pub struct Person {
    pub id: i64,
    pub birthday: NaiveDate,
}
```

### `AsDateTime`

`chrono::DateTime<Utc>` ↔ `TEXT` (RFC-3339). El cast por defecto para
timestamps arbitrarios cuando quieres una representación de reloj de
pared.

Las escrituras se normalizan a RFC-3339. Las lecturas también aceptan el
texto `CURRENT_TIMESTAMP` nativo que emite PostgreSQL y los valores sin
zona horaria de SQLite/MySQL; los valores sin zona horaria se interpretan
como UTC. `AsImmutableDateTime` y `AsOptionalDateTime` usan el mismo
parser.

### `AsImmutableDate` y `AsImmutableDateTime`

La misma forma de almacenamiento que `AsDate` / `AsDateTime`. El borrow
checker de Rust ya impone la inmutabilidad a través de las referencias
`&`, así que estos casts comparten los tipos subyacentes - existen por
paridad con `immutable_date` / `immutable_datetime` de Laravel y para
documentar la intención en el sitio de declaración del modelo.

### `AsOptionalDateTime`

`Option<DateTime<Utc>>` ↔ `Option<String>`. Se inyecta automáticamente
con el flag `#[model(soft_deletes)]` para la columna anulable que marca
la eliminación (`deleted_at` por defecto - consulta
[Eliminaciones suaves](eloquent.md#deleting-and-soft-deletes)). La opción
que lo envuelve mantiene la columna de almacenamiento anulable, de modo
que las filas eliminadas de forma suave y las vivas se distinguen con
`IS NULL` sin necesidad de un valor centinela.

Usa el cast directamente en cualquier otra columna datetime anulable que
quieras hacer viajar de ida y vuelta como texto RFC-3339:

```rust
#[model(
    table = "subscriptions",
    casts = { cancelled_at = AsOptionalDateTime },
)]
pub struct Subscription {
    pub id: i64,
    pub cancelled_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

### `AsTimestamp`

`i64` de época Unix ↔ `INTEGER`. Úsalo cuando la columna se consulte
como un rango numérico o se use en aritmética. Distinto de `AsDateTime` -
elige `AsTimestamp` cuando quieras `WHERE created_unix > 1700000000` y
`AsDateTime` cuando quieras cadenas RFC-3339 en tus logs.

## Los casts estructurados

Cinco casts cubren colecciones, structs, y JSON arbitrario. Todos
serializan el valor en tiempo de ejecución a texto JSON y lo
almacenan en una columna `TEXT`. Las columnas nativas `JSON` /
`JSONB` de Postgres y `JSON` de MySQL aceptan el mismo payload de
string - si quieres un tipo de columna JSON nativo para indexado,
decláralo a mano en una migración; la capa de casts no restringe el
tipo de columna.

### `AsArray<T>`

`Vec<T>` ↔ `TEXT` codificado en JSON. El tipo de elemento debe ser
`Serialize + DeserializeOwned`.

```rust
use suprnova::AsArray;

#[model(table = "posts", casts = { tags = AsArray<String> })]
pub struct Post {
    pub id: i64,
    pub tags: Vec<String>,
}
```

### `AsObject<T>`

Un struct `Serialize + DeserializeOwned` ↔ `TEXT` codificado en JSON.
Úsalo cuando la forma en tiempo de ejecución es un registro fijo con
claves conocidas estáticamente.

```rust
use serde::{Deserialize, Serialize};
use suprnova::AsObject;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    pub theme: String,
    pub notifications: bool,
}

#[model(table = "users", casts = { prefs = AsObject<Prefs> })]
pub struct User {
    pub id: i64,
    pub prefs: Prefs,
}
```

### `AsCollection<T>`

`Collection<T>` ↔ `TEXT` codificado en JSON. Envoltorio delgado sobre
`AsArray` que hace el viaje de ida y vuelta a través del
`Collection<T>` de Suprnova (un newtype de `Vec<T>` con la superficie
de slice de estilo Laravel - ver [Colecciones](eloquent.md#collections)).

### `AsJson<T>`

Cualquier tipo `Serialize + DeserializeOwned` ↔ `TEXT` codificado en
JSON. Úsalo cuando el campo es un `serde_json::Value` o un struct
definido por el usuario que ya es completamente describible en
términos de serde pero no encaja en el patrón de forma fija de
`AsObject` (por ejemplo, payloads de enum, mapas sin tipar).

### `AsArrayObject<T>`

`IndexMap<String, T>` ↔ `TEXT` codificado en JSON. Úsalo cuando la
forma en tiempo de ejecución es un mapa de claves dinámicas y el
orden de las claves importa (el orden de UI de las etiquetas, el
orden canónico de un bloque de configuración). Usar `IndexMap` en
vez de `HashMap` es intencional: serde preserva el orden de
inserción a través de `IndexMap`, y el `serde_json` de Suprnova ya
está configurado con `preserve_order` por la misma razón.

Para registros de forma fija usa `AsObject`; para arrays usa
`AsArray`.

## El cast de enum

### `AsEnum<E>`

`E: FromStr + AsRef<str>` ↔ `TEXT`. El nombre de la variante del enum
(o su string personalizado por `AsRefStr`) es lo que aterriza en la
columna. El framework no te ata a `strum`, pero es la forma más
ergonómica de obtener los dos bounds sin escribirlos a mano:

```rust
use suprnova::AsEnum;

#[derive(Debug, Clone, Copy, strum::EnumString, strum::AsRefStr)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

#[model(
    table = "users",
    casts = { role = AsEnum<Role> },
)]
pub struct User {
    pub id: i64,
    pub role: Role,
}
```

El almacenamiento por discriminante entero deliberadamente no es el
valor por defecto. Un `Role::Admin = 0` que más tarde se convierte
en `Role::Admin = 2` tras un reordenamiento intercambiaría en
silencio a todos los admins en la base de datos. Los nombres de
variante se autodescriben en un cliente de base de datos y son
estables a través de reordenamientos.

## Cifrado y hashing

Cinco casts median transformaciones criptográficas en el límite de
almacenamiento. Los cuatro casts `AsEncrypted*` comparten la fachada
[`Crypt`](encryption.md) - la fachada debe inicializarse antes de
que corra cualquiera de ellos. Las apps en producción obtienen esto
a través de `Server::from_config` (que lee `APP_KEY` del entorno);
los tests llaman a
`suprnova::testing::install_test_encryption_key()` una vez al
arrancar.

### `AsEncrypted`

`String` ↔ `String` cifrado con AES-256-GCM. La columna en disco
guarda base64 URL-safe de `nonce || ciphertext_with_tag`. Cada
escritura usa un nonce aleatorio fresco, así que dos escrituras del
mismo texto plano producen ciphertexts distintos - el administrador
de tu BD no puede identificar secretos duplicados en reposo.

```rust
use suprnova::AsEncrypted;

#[model(
    table = "secrets",
    casts = { api_key = AsEncrypted },
)]
pub struct Secret {
    pub id: i64,
    pub api_key: String,  // en tiempo de ejecución es UTF-8 plano
}
```

El valor en tiempo de ejecución es el string UTF-8 descifrado; lo
lees y escribes como cualquier otro `String`.

### `AsEncryptedArray<T>` / `AsEncryptedObject<T>` / `AsEncryptedCollection<T>`

`Vec<T>` / `T` / `Collection<T>` ↔ JSON cifrado con AES-256-GCM. El
pipeline es: serializar a JSON → cifrar → base64 → almacenar; a la
inversa en la lectura. El tipo de elemento / valor debe ser
`Serialize + DeserializeOwned`.

```rust
use suprnova::AsEncryptedObject;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct CardOnFile {
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[model(
    table = "billing",
    casts = { card = AsEncryptedObject<CardOnFile> },
)]
pub struct Billing {
    pub id: i64,
    pub card: CardOnFile,
}
```

### Rotación de claves

La fachada `Crypt` soporta rotación a través de `APP_KEY_PREVIOUS`:
el cifrado siempre usa `APP_KEY`, pero el descifrado intenta primero
con `APP_KEY` y recurre a `APP_KEY_PREVIOUS` si la clave primaria
falla. Una estrategia de recifrado progresivo es: fija `APP_KEY` a
la clave nueva, mueve la clave vieja a `APP_KEY_PREVIOUS`, y luego
haz `save()` sobre cada fila cifrada para reescribir los ciphertexts
bajo la clave nueva. La capa de casts no tiene que saber nada sobre
la rotación - hace el viaje de ida y vuelta a través de `Crypt` en
cada lectura y escritura, así que un `User::all().await?` seguido de
guardar cada fila migra la columna en el sitio. Consulta
[Cifrado](encryption.md) para el protocolo de rotación completo.

### `AsHashed`

`String` ↔ un string hasheado al escribir, usando el driver de hash
activo (variable de entorno `HASH_DRIVER` - bcrypt por defecto,
argon2i y argon2id también soportados). El valor en tiempo de
ejecución ES el string hasheado; no hay dirección inversa. Refleja
el cast `hashed` de Laravel.

```rust
use suprnova::AsHashed;

#[model(
    table = "users",
    casts = { password = AsHashed },
)]
pub struct User {
    pub id: i64,
    pub password: String,
}
```

`AsHashed::to_storage` es **idempotente**: un valor que ya se parece
a CUALQUIER hash reconocido (bcrypt `$2*$`, argon2i / argon2id PHC)
pasa sin cambios. Sin esta salvaguarda,
`User::find(id).await?.save().await?` volvería a hashear el hash
existente convirtiéndolo en un hash-de-hash, rompiendo
`Hash::check(plain, stored)` e invalidando toda contraseña
existente.

Combina `AsHashed` con el patrón `#[mutator]` (más abajo) cuando
necesites aplicar algo más que un hash al escribir - por ejemplo,
normalizar espacios en blanco o rechazar contraseñas en blanco antes
de hashear.

## Override de cast en tiempo de ejecución - macro `casts!`

Los casts declarados en `#[model(casts = { ... })]` son estáticos -
se disparan en cada lectura de ese modelo. Cuando necesites un cast
distinto para una sola consulta (una herramienta de debug quiere la
forma almacenada en crudo, un script de exportación quiere una
representación JSON distinta), usa `Builder::with_casts(...)`:

```rust
use suprnova::{casts, AsDate, AsJson, User};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

La macro `casts!` construye un `HashMap<&'static str, Arc<dyn DynCast>>`.
Cada entrada es `field_name = CastType`; cada cast incorporado
implementa `IntoDynCast`, así que la versión con el tipo borrado de
`DynCast` se genera automáticamente. El mapa de override en tiempo de
ejecución solo aplica durante la consulta encadenada - el pipeline
de casts estático del modelo no cambia.

Usa esta superficie con moderación. El atributo del modelo es el
sitio correcto para los casts que quieres que se apliquen en cada
lectura; el override en tiempo de ejecución es la vía de escape para
consultas puntuales.

## Accesores - atributos virtuales a partir de columnas reales

Un accesor es un método `impl` sobre el modelo anotado con la macro
`#[accessor]`. Cuando listas el nombre del método en
`#[model(appends = [...])]`, el `to_json()` del modelo llama al
método e inserta el resultado bajo esa clave.

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    appends = ["full_name"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }
}
```

Un `serde_json::to_value(&user)` (o `user.to_json()`) ahora contiene:

```json
{
  "id": 1,
  "first_name": "Alice",
  "last_name": "Xu",
  "full_name": "Alice Xu"
}
```

El método también se puede llamar directamente
(`user.full_name()`) - la macro `#[accessor]` es principalmente un
marcador para que la macro `#[suprnova::model]` a nivel de struct
pueda cablear el despacho de `to_json()`. No hay ningún costo por
llamarlo desde tu propio código.

Cada nombre en `appends` debe coincidir con un método `#[accessor]`
real por identificador. Un typo (`appends = ["fullName"]` cuando el
método es `full_name`) se detecta en tiempo de compilación con un
mensaje de error señalado.

### Devolver valores que no son `String`

Los accesores pueden devolver cualquier tipo `Serialize`. La macro
convierte el valor devuelto a través de `serde_json::to_value` antes
de insertarlo, así que:

```rust
impl Post {
    #[accessor]
    pub fn word_count(&self) -> usize {
        self.body.split_whitespace().count()
    }
}
```

se renderiza como `"word_count": 42` en la salida JSON.

### Ocultar las columnas de origen

Cuando lo que el consumidor debería ver es el valor del accesor y
las columnas subyacentes son ruido, combina `appends` con `hidden`:

```rust
#[model(
    table = "users",
    appends = ["full_name"],
    hidden = ["first_name", "last_name"],
)]
```

`hidden` quita las columnas nombradas de la salida serializada;
`appends` inserta después el valor del accesor. El orden es fijo -
los filtros corren primero, la inyección del accesor corre después.
Consulta
[Hidden, visible y appends](eloquent.md#mass-assignment) para la
superficie completa.

## Mutadores - escrituras enrutadas a través de tu transformación

Un mutador es la contraparte del lado de escritura. Cuando el nombre
del campo aparece en `#[model(mutators = [...])]`, cada ruta de
asignación masiva (`create` / `update`) enruta el valor a través de
`self.set_<field>(value)?` en lugar de asignar el campo directamente.

```rust
use serde_json::Value;
use suprnova::{model, mutator, FrameworkError, Model};

#[model(
    table = "users",
    fillable = ["password"],
    mutators = ["password"],
)]
pub struct User {
    pub id: i64,
    pub password: String,
}

impl User {
    #[mutator]
    pub fn set_password(&mut self, value: Value) -> Result<(), FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            FrameworkError::validation("password", format!("{e}"))
        })?;
        // Normaliza + hashea; AsHashed haría el hash por su cuenta,
        // pero el mutador es donde también puedes imponer política.
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        self.password = suprnova::hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

`set_password` recibe un `serde_json::Value`. El cuerpo es dueño de
la deserialización + transformación - el tipo del campo en el struct
puede seguir siendo `String`, y tu validación corre antes de que se
toque la columna. Un error devuelto se propaga a través de
`create()` / `update()` como un `bad_request`.

La asignación directa del campo salta el mutador:

```rust
user.password = "raw".to_string();  // salta set_password
user.save().await?;                 // guarda "raw"
```

Esto coincide con el comportamiento de `$user->password = ...` frente
a `$user->fill(...)` de Laravel. Cuando quieras que el mutador sea la
única ruta, enruta todas las escrituras a través de `attrs!` +
`create` / `update`.

### Combinar mutadores con casts

Un mutador y un cast pueden coexistir sobre el mismo campo; el
mutador corre en la ruta de escritura (cuando se llama a `create` /
`update`), el cast corre en la ruta de lectura (cuando la columna se
materializa desde un SELECT). Un patrón común es usar `AsHashed`
para la garantía de idempotencia del lado de la lectura y el mutador
para la validación del lado de la escritura - el mutador hashea,
`AsHashed` ve un valor ya hasheado y pasa sin cambios.

## Timestamps autogestionados

Cuando un modelo lleva tanto `created_at` como `updated_at`
(tipados `chrono::DateTime<chrono::Utc>`), la macro:

- Fija ambos a `Utc::now()` en `create()`.
- Actualiza `updated_at` en cada `save()` y `update(attrs)`.
- Emite un `impl Touchable for YourStruct` para que puedas llamar a
  `.touch().await` y actualizar `updated_at` sin cambiar ninguna
  otra columna.

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model, Touchable};

#[model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Actualiza updated_at sin otros cambios:
let post = Post::find_or_fail(1).await?;
post.touch().await?;
```

El almacenamiento usa el cast `AsDateTime` que la macro auto-inyecta
para las columnas de timestamp. El cast permite que el mismo valor
`DateTime<Utc>` haga el viaje de ida y vuelta a través de los tres
drivers de SeaORM (SQLite, MySQL, PostgreSQL) sin forzarte a elegir
un tipo de timestamp específico de base de datos.

### Desactivación y nombres de columna personalizados

`#[model(timestamps = false)]` desactiva la auto-gestión por
completo - tú controlas los timestamps.

`#[model(created_at = "creado_en", updated_at = "actualizado_en")]`
conserva la auto-gestión pero renombra las columnas. La macro
detecta los campos renombrados y cablea la misma lógica contra
ellos.

Cuando el struct tiene solo UNO de los dos campos de timestamp, la
macro emite un `compile_error!` - casi siempre un typo (`craeted_at`)
que quieres que emerja de forma evidente en lugar de quedar
silenciosamente descartado.

### `without_touching` - supresión con alcance de tarea

A veces quieres actualizar una fila sin actualizar `updated_at` -
ejecutando un backfill, corrigiendo un typo, registrando una
sincronización interna que no debería reiniciar los TTL de caché
indexados por `updated_at`. Envuelve el trabajo en
`without_touching`:

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    for post in Post::query().get().await? {
        post.touch().await?;  // no-op dentro del alcance
    }
    Ok::<_, suprnova::FrameworkError>(())
}).await?;
```

El flag es un `tokio::task_local!`, así que no se filtra a través de
los límites de `tokio::spawn` - las solicitudes concurrentes en
otras tareas siguen respetando su propio alcance (o su ausencia).
Este es el análogo en Suprnova del `Model::withoutTouching(closure)`
de Laravel.

### Por qué Suprnova diverge

Laravel usa una propiedad estática `$timestamps = false` y un método
estático global `Model::withoutTouching` respaldado por un contador
de instancia. Ambos enfoques asumen aislamiento de una solicitud por
proceso. Suprnova corre muchas solicitudes sobre un único runtime de
Tokio, así que un flag global de proceso permitiría que una
solicitud suprimiera en silencio los timestamps de otra. El alcance
`tokio::task_local!` es consciente de async: sigue a los futures a
través de los puntos `.await` dentro de la misma tarea y sale de
alcance cuando el future se descarta, sin importar cómo termine la
solicitud.

## El evento de ciclo de vida `Replicating`

De los 16 eventos de ciclo de vida del modelo (ver
[Observadores y eventos de ciclo de vida](eloquent.md#observers-and-lifecycle-events)),
`Replicating` es el que se dispara cuando clonas una fila existente
hacia una copia sin guardar en memoria vía `replicate()`:

```rust
let original = Post::find_or_fail(1).await?;
let mut copy = original.replicate().await?;  // sin guardar
copy.title = format!("{} (copy)", original.title);
copy.save().await?;  // ahora persistida con una PK nueva
```

El evento `Replicating` se dispara DESPUÉS de que el clon en memoria
se construye pero ANTES de que hayas tenido oportunidad de mutarlo.
Los oyentes reciben `(&Self, Arc<Mutex<Self>>)` - el original y la
réplica recién construida detrás de un `Mutex`, así que puedes mutar
la réplica desde el oyente antes de que el usuario la vea:

```rust
use suprnova::{Listener, FrameworkError};

pub struct ResetReplicatedFlags;

#[async_trait::async_trait]
impl Listener<post::events::Replicating> for ResetReplicatedFlags {
    async fn handle(&self, event: &post::events::Replicating) -> Result<(), FrameworkError> {
        let mut replica = event.replica.lock().await;
        replica.published = false;       // las copias empiezan sin publicar
        replica.view_count = 0;          // los contadores se reinician
        Ok(())
    }
}
```

La PK de la réplica ya está limpiada para cuando el oyente corre -
`replicate()` llama a `reset_primary_key()` antes de disparar el
evento, así que no puedes guardar por accidente bajo el ID original.
Los timestamps también se reinician; `created_at` / `updated_at` se
disparan en el `save()` posterior como cualquier fila nueva.

### `replicate_into<T>` - replicación entre tipos

Cuando la réplica es de un tipo distinto (`Post` → `Draft`, por
ejemplo), usa `replicate_into::<Draft>()`. El evento `Replicating`
NO se dispara en esta ruta porque el struct del evento es por tipo
de origen, y un oyente registrado para `post::events::Replicating`
recibiría un `Arc<Mutex<Post>>`, no un `Arc<Mutex<Draft>>`. La ruta
entre tipos es para cuando quieres un tipo de destino fresco sin
interferencia de observadores; registra un oyente `Creating` normal
sobre el tipo de destino si quieres un gancho en la construcción.

Consulta [Replicación](eloquent.md#replication) para el resto de la
superficie de replicate (`replicate_except`, el manejo de relaciones
de la réplica, las reglas para PKs nulables).

## Uniendo todo

Un modelo con cada superficie de este capítulo:

```rust
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use suprnova::{
    accessor, hashing, model, mutator, AsBool, AsDateTime,
    AsDecimal, AsEncryptedObject, AsEnum, AsHashed, AsJson,
    AsOptionalDateTime, FrameworkError, Model,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardOnFile {
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[derive(Debug, Clone, Copy, strum::EnumString, strum::AsRefStr)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

#[model(
    table = "users",
    soft_deletes,
    appends = ["display_name"],
    hidden = ["password", "card"],
    fillable = ["name", "email", "password", "role", "credit"],
    mutators = ["password"],
    casts = {
        role = AsEnum<Role>,
        verified = AsBool,
        credit = AsDecimal<2>,
        card = AsEncryptedObject<CardOnFile>,
        metadata = AsJson<serde_json::Value>,
        password = AsHashed,
        last_login_at = AsOptionalDateTime,
    },
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: Role,
    pub verified: bool,
    pub credit: Decimal,
    pub card: CardOnFile,
    pub metadata: serde_json::Value,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // deleted_at es auto-inyectado por soft_deletes (AsOptionalDateTime)
}

impl User {
    #[accessor]
    pub fn display_name(&self) -> String {
        if self.name.is_empty() { self.email.clone() } else { self.name.clone() }
    }

    #[mutator]
    pub fn set_password(&mut self, value: Value) -> Result<(), FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            FrameworkError::validation("password", format!("{e}"))
        })?;
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        // El mutador hashea; AsHashed ve un valor ya hasheado en los
        // guardados posteriores y pasa sin cambios.
        self.password = hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

Esta única declaración te da:

- Ocho casts tipados que cablean el límite storage / runtime.
- Un accesor que sintetiza `display_name` a partir de columnas ya
  existentes.
- Un mutador que valida y hashea la contraseña.
- `created_at` / `updated_at` autogestionados.
- Eliminación suave con una columna `deleted_at` auto-inyectada.
- Almacenamiento cifrado de tarjeta en archivo con soporte de
  rotación de claves.

Cada cast se comprueba en tiempo de compilación. El query builder de
API dual (ver
[Eloquent - query builder](eloquent.md#query-builder--dual-api))
corre contra las columnas tipadas; la serialización hacia Inertia /
JSON aplica las reglas de hidden / appends; y un
`User::find(id).await?` materializa la fila a través de ocho
llamadas a `Cast::from_storage` sin que escribas una sola línea de
código de conversión.

## Siguiente

- [API de Eloquent](eloquent.md) - el resto de la superficie de
  modelo: query builder, relaciones, observadores, paginación,
  transacciones.
- [Cifrado](encryption.md) - la fachada `Crypt` que comparten los
  casts cifrados, el protocolo de rotación de claves, y la superficie
  de criptografía más amplia.
- [Eventos y oyentes](events.md) - el dispatcher detrás de
  `Replicating` y los otros 15 eventos de ciclo de vida del modelo.
- [Autenticación](authentication.md) - el trait `Authenticatable` y
  dónde encaja `AsHashed` en el flujo de contraseñas.
- [Validación](validation.md) - `FrameworkError::validation` y el
  patrón que usan los mutadores para hacer emerger errores por campo.

# Objetos de datos

El `#[derive(Data)]` de Suprnova te permite describir la forma de una
solicitud entrante, la forma de una respuesta saliente, y una
exportación de TypeScript en **un solo struct**.

## Inicio rápido

```rust
use suprnova::Data;
use suprnova::data::Field;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UserDto {
    pub id: i64,

    #[validate(email)]
    pub email: String,

    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub display_handle: String,

    pub bio: Field<String>,
}
```

`#[derive(Data)]` genera:
- `Serialize` (omitiendo los campos `#[data(input_only)]`)
- `Deserialize` (rechazando los campos `#[data(output_only)]` en el payload, y dándoles por defecto `T::default()`)
- `FormRequest` con `authorize: true` por defecto - los handlers pueden tomar el tipo directamente como un extractor
- `IntoInertiaData` (la ruta de despacho `Inertia::data(component, dto)`)
- Un registro `inventory::submit!` para cualquier campo `#[data(allow_include)]`

Añade `#[derive(Validate)]` por separado para que los atributos
`#[validate(...)]` sigan siendo visibles en el sitio de declaración del
campo.

## Atributos de campo

| Atributo | Efecto |
|---|---|
| `#[data(input_only)]` | Aceptado en Deserialize, omitido de Serialize |
| `#[data(output_only)]` | Rechazado en Deserialize (422), incluido en Serialize |
| `#[data(allow_include)]` | El campo es elegible para `?include=`. **Denegación por defecto**: cualquier solicitud `?include=foo` donde `foo` no esté en la lista de permitidos devuelve 400 |
| `#[data(lazy)]` | El campo es un `Prop` que se resuelve contra el conjunto de inclusión de la solicitud; se auto-registra como `allow_include` |
| `#[data(lazy(inertia))]` | Igual que `lazy`, etiquetado para el protocolo de recarga parcial de Inertia |
| `#[data(lazy(deferred))]` | Etiquetado para el protocolo de props diferidas de Inertia |
| `#[data(lazy(closure))]` | Siempre se resuelve en la visita inicial; perezoso en las recargas parciales |
| `#[data(lazy(when_loaded))]` | Se resuelve solo si la entidad de origen tiene la relación precargada |
| `#[data(from_route_param)]` | El valor del campo proviene de una captura de ruta (p. ej. `/users/{id}`). Clave por defecto = nombre del campo; pasa `#[data(from_route_param("id"))]` para sobrescribirla |

## Atributos de struct

| Atributo | Efecto |
|---|---|
| `#[data(auto_lazy)]` | Cada campo de tipo `Prop` es implícitamente `#[data(lazy)]` |
| `#[data(authorize = "path::to::fn")]` | Enruta el `FormRequest::authorize` generado hacia una función libre con la firma `fn(req: &Request) -> bool`. El parser del cuerpo, el validador, el soporte de Precognition, y la inyección de parámetros de ruta siguen viniendo del derive |
| `#[data(allow_unknown_fields)]` | Acepta claves del payload que no coincidan con ningún campo del struct. El valor por defecto es **estricto**: una clave no reconocida hace fallar el deserialize con `serde::de::Error::unknown_field(..)` y emerge como un 422 a través de `FormRequest`. Opta por lo permisivo solo para DTOs de respuesta que leen payloads de terceros compatibles hacia adelante |

El flag anterior `#[data(custom_authorize)]` - que suprimía todo el
impl de `FormRequest` y te obligaba a reimplementar el análisis del
cuerpo, la validación, y Precognition a mano - ha desaparecido. La
macro emite un error de migración si intentas usarlo. Usa
`#[data(authorize = "fn")]` en su lugar.

## `Field<T>` - Ausente / Nulo / Valor

Para endpoints PATCH donde "ausente del payload" debe distinguirse de
"nulo explícito":

```rust
use suprnova::data::Field;

match dto.bio {
    Field::Absent  => { /* don't touch this column */ },
    Field::Null    => { /* clear the column */ },
    Field::Value(text) => { /* set to text */ },
}
```

`Field::Absent` (por defecto) hace el viaje de ida y vuelta hasta
quedar omitido del JSON cuando se combina con
`#[serde(default, skip_serializing_if = "Field::is_absent")]` en el
sitio de declaración. Sin `skip_serializing_if`, `Absent` serializa a
`null` en JSON.

Para upserts de base de datos de tres vías:
`dto.bio.into_option_or_null() -> Option<Option<T>>` mapea
`Absent → None`, `Null → Some(None)`, `Value(v) → Some(Some(v))`. Usa
esto cuando "no tocar" y "establecer a NULL" necesiten ser distintos
más adelante en el flujo.

> **Advertencia:** `Field<Option<T>>` tiene pérdida - tanto
> `Value(None)` como `Null` serializan como `null` en JSON y
> deserializan de vuelta a `Null`. Para tipos internos anulables,
> prefiere un `Field<T>` plano y deja que `Null` lleve la señal de
> "límpialo".

## Query string `?include=`

El `IncludeMiddleware` analiza el query string de la solicitud dentro
de un `RequestIncludeSet` por solicitud:

- `?include=foo,bar` - resuelve los campos perezosos `foo` y `bar`.
- `?include[]=foo&include[]=bar` - forma de array, mismo resultado.
- `?exclude=`, `?only=`, `?except=` - paridad con la API de Laravel-Data.

Composición con `X-Inertia-Partial-Data` (el encabezado de recarga
parcial de Inertia): el conjunto de inclusión + la lista de permitidos
por DTO se ejecuta **primero** para los campos perezosos etiquetados
por propietario, así que una solicitud de un campo no permitido
devuelve 400 incluso si los datos parciales lo habrían filtrado. Los
datos parciales se aplican **después** como un filtro final de tipo
"only" sobre las props resueltas.

Registra `IncludeMiddleware` globalmente - típicamente entre la sesión
y la autorización en la pila de middleware:

```text
SessionMiddleware → IncludeMiddleware → AuthMiddleware → handlers
```

### Include/exclude/only/except programáticos

`RequestIncludeSet` refleja el contrato `IncludeableData` de
Laravel-Data con builders encadenables. Los handlers, los tests, y el
middleware pueden construir o sobrescribir un conjunto sin tocar
directamente los campos públicos:

```rust
use suprnova::data::RequestIncludeSet;

let set = RequestIncludeSet::default()
    .include(["author", "comments"])
    .exclude(["password"])
    .only(["id", "name"])
    .except(["secret"]);

assert!(set.is_visible("name"));   // en `only`, no en `except`
assert!(!set.is_visible("secret"));// `except` siempre gana
assert!(set.includes("author"));   // se solicita la relación `author`
```

| Método | Efecto | Equivalente en Laravel |
|---|---|---|
| `.include(fields)` | añade a la lista de inclusión (campos perezosos a resolver) | `Data::include(...$fields)` |
| `.exclude(fields)` | añade a la lista de exclusión (campos a descartar) | `Data::exclude(...$fields)` |
| `.only(fields)` | inicializa o extiende la lista de permitidos `only` | `Data::only(...$fields)` |
| `.except(fields)` | añade a la lista except (descarte siempre) | `Data::except(...$fields)` |
| `.include_when(cond, fields)` | añade solo cuando `cond == true` | `Data::includeWhen($field, $condition)` |
| `.exclude_when(cond, fields)` | añade solo cuando `cond == true` | `Data::excludeWhen($field, $condition)` |
| `.only_when(cond, fields)` | extiende `only` solo cuando `cond == true` | `Data::onlyWhen($field, $condition)` |
| `.except_when(cond, fields)` | añade solo cuando `cond == true` | `Data::exceptWhen($field, $condition)` |
| `.merge(other)` | une dos conjuntos (overrides por capas, en el sitio) | `array_merge` manual en PHP |
| `.includes(field)` | ¿`field` (o `field.path`) está en la lista de inclusión? | análogo a `relationLoaded()` |
| `.is_excluded(field)` | ¿`field` está en la lista de exclusión? | lee la parcial de exclude |
| `.is_excepted(field)` | ¿`field` está en la lista except? | lee la parcial de except |
| `.is_only_listed(field)` | ¿`field` está permitido por `only` (u `only` sin establecer)? | lee la parcial de only |
| `.is_visible(field)` | orden de resolución completo de Laravel: except → exclude → only | decisión de `resolveResource` |

Los builders toman cualquier `IntoIterator<Item = impl Into<String>>`,
así que los arrays, los vecs, y los slices de `&str`/`String` funcionan
todos. Los strings se recortan; las entradas vacías se descartan
(igual que `from_query`).

Las rutas con puntos en cualquier lista coinciden con el segmento raíz
cuando se consultan por nombre simple - `include=["author.posts"]`
reporta `set.includes("author") == true`, igual que la resolución de
rutas de Laravel-Data. El segmento anidado `posts` es consumido por
`IncludeTree::from_include_set` para los documentos compuestos
JSON:API.

### Override del lado del handler: `with_include_overrides`

Para superponer overrides programáticos encima de lo que el query
string de la solicitud ya declaró (sin perder el conjunto de la
solicitud), usa `with_include_overrides`:

```rust
use suprnova::data::with_include_overrides;

async fn show_album(req: Request, user: User) -> Response {
    with_include_overrides(
        |set| set
            .include_when(user.is_admin(), ["audit_log"])
            .exclude_when(!user.is_admin(), ["price_cost"]),
        async move {
            // Dentro de este ámbito, el resolver de props perezosos y el
            // resolver de include de JSON:API ven el conjunto fusionado.
            Inertia::data("Album/Show", album_dto).into_response()
        },
    ).await
}
```

El closure se ejecuta contra un clon del conjunto actualmente
vinculado (o el valor por defecto vacío si ningún middleware ha
vinculado uno). Después de que el future se completa, el conjunto
original se restaura - esto es un override con alcance, no una
mutación.

Para tests, prefiere `scope_include_set(set, future)` para instalar un
conjunto nuevo sin heredar ningún estado ambiental.

## Structs genéricos

```rust
use serde::{Serialize, Deserialize};

#[derive(suprnova::Data)]
pub struct Paginated<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    pub items: Vec<T>,
    pub total: usize,

    #[data(allow_include)]
    pub meta: Option<serde_json::Value>,
}
```

El extractor de TypeScript emite `export interface Paginated<T>` para
que el código del frontend pueda reutilizar el genérico entre
instanciaciones.

La lista de permitidos de `?include=` se indexa por la ruta de tipo
totalmente calificada
(`concat!(module_path!(), "::", stringify!(Paginated))`), no por las
instanciaciones de parámetros de tipo. `Paginated<UserDto>` y
`Paginated<ArticleDto>` declarados en el mismo módulo comparten una
sola lista de permitidos - `allow_include` nombra un campo, y los
nombres de campo no dependen de los parámetros de tipo. Dos DTOs
distintos llamados `Paginated` en módulos diferentes obtienen cada uno
su propia lista de permitidos; sus claves no colisionan.

Nota: `FormRequest` se suprime para los structs genéricos porque sus
trait bounds (`DeserializeOwned + Validate + Send`) no pueden
verificarse sin conocer los parámetros de tipo concretos. Provee tu
propio impl si necesitas extraer un struct `Data` genérico desde una
solicitud.

## Inyección de campo desde parámetro de ruta

```rust
use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UpdateUser {
    #[data(from_route_param("id"))]
    pub id: i64,

    #[validate(length(min = 1))]
    pub name: String,
}
```

Para `PATCH /users/{id}` con cuerpo `{"name": "Ada"}`, el `id`
capturado por la ruta se fusiona dentro del payload validado. **La
ruta siempre gana sobre un valor proporcionado por el cuerpo** (evita
IDOR mediante manipulación del cuerpo).

El `#[data(from_route_param)]` desnudo recurre por defecto al nombre
del campo. La macro clasifica el último segmento de la ruta de tipo del
campo en tiempo de compilación y despacha hacia un parser coincidente.
Solo se reconocen los nombres exactos listados abajo; todo lo demás
(incluidos `i8`/`i16`/`isize`, `Uuid`, `DateTime`, newtypes
personalizados) cae hacia `pass_string` y deja que el propio
`Deserialize` del campo haga el trabajo.

| Tipo de campo | Parser |
|---|---|
| `i64` | `parse_i64` |
| `u64` | `parse_u64` |
| `i32` | `parse_i32` |
| `u32` | `parse_u32` |
| `i128` | `parse_i128` (valida y luego pasa el string en crudo; el `Deserialize` del campo lo analiza) |
| `u128` | `parse_u128` (mismo patrón de paso de string) |
| `f64` | `parse_f64` (rechaza valores no finitos) |
| `f32` | `parse_f32` (rechaza valores no finitos) |
| `bool` | `parse_bool` (acepta solo `"true"` / `"false"`) |
| Cualquier otra cosa | `pass_string` - string en crudo entregado al propio `Deserialize` del campo |
| `Option<T>` o `Field<T>` de cualquiera de los anteriores | El mismo parser que `T`; un parámetro de ruta ausente deja el campo ausente |

## Props perezosas

```rust
use suprnova::Data;
use suprnova::inertia::Prop;

#[derive(Data)]
#[data(auto_lazy)]
pub struct AlbumDto {
    pub id: i64,
    pub songs: Prop,    // registrado automáticamente como ?include=songs
    pub artist: Prop,   // registrado automáticamente como ?include=artist
}
```

Variante explícita por campo:

```rust
#[derive(Data)]
pub struct AlbumDto {
    pub id: i64,

    #[data(lazy(inertia))]
    pub songs: Prop,

    #[data(lazy(deferred))]
    pub lyrics: Prop,

    #[data(lazy(closure))]
    pub artist: Prop,
}
```

Todas las variantes perezosas están sujetas a la misma condición,
incluida `lazy(deferred)`. Un campo diferido requiere consentimiento dos
veces: `?include=lyrics` lo incluye en el alcance de la solicitud, y el
protocolo de props diferidas de Inertia decide qué viaje de ida y vuelta
lo transporta. Un campo que la solicitud nunca incluyó se descarta por
completo - sin valor ni anuncio `deferredProps` -, de modo que el cliente
nunca recibe algo sobre lo que esta solicitud no tiene permiso. Un campo
nombrado por `?include=` pero ausente de la lista de permitidos devuelve
400 en la primera visita, antes de que `X-Inertia-Partial-Data` pueda
absorber silenciosamente el error.

Usa `Inertia::data(component, dto)` para renderizar - el derive genera
un impl de `IntoInertiaData` que consulta el conjunto de inclusión y la
lista de permitidos:

```rust
return Inertia::data("Album/Show", album_dto);
```

Nota: los structs que llevan campos perezosos suprimen `Serialize`,
`Deserialize`, y `FormRequest` porque `Prop` no los implementa. Si un
único endpoint necesita tanto análisis entrante como salida perezosa,
usa dos DTOs: uno entrante (`#[derive(Data, Validate)]` plano) y uno
saliente (`#[derive(Data)]` con campos perezosos).

## `when_loaded!` - perezoso condicional según relación cargada

Refleja el `#[AutoWhenLoadedLazy]` de Laravel-Data. El impl
`From<Entity>` del usuario decide si la relación se precargó:

```rust
use suprnova::data::{when_loaded, IsRelationLoaded};

impl From<&AlbumEntity> for AlbumDto {
    fn from(album: &AlbumEntity) -> Self {
        Self {
            id: album.id,
            songs: when_loaded!(album, "songs", || async {
                serde_json::json!(album.songs_relation()
                    .iter()
                    .map(SongDto::from)
                    .collect::<Vec<_>>())
            }),
            artist: Prop::eager(serde_json::json!(album.artist_name())),
            lyrics: Prop::lazy(|| async { /* ... */ }),
        }
    }
}
```

Si la entidad no ha precargado la relación nombrada (según
`IsRelationLoaded::is_relation_loaded`), `when_loaded!` devuelve
`Prop::absent()` y el campo está ausente de la respuesta.

Las entidades de SeaORM necesitan un impl personalizado de
`IsRelationLoaded` que consulte su estado de relaciones cargadas - no
hay ningún blanket impl provisto por el framework porque el
`ModelTrait` de SeaORM no lleva estado de relación-cargada por
instancia (las relaciones cargadas viven en los resultados de la
consulta, no en el struct del modelo mismo).

## Exportación a TypeScript

`suprnova generate-types` emite definiciones de TypeScript para cada
struct `#[derive(Data)]` (y, de forma heredada,
`#[derive(InertiaProps)]`). Comportamiento:

- `Field<T>` → `field?: T | null`
- `Prop` → `field?: T` (la semántica de "puede estar ausente" perezosa; el `?` la lleva, el tipo en sí es plano)
- `#[data(input_only)]` → excluido del tipo de salida
- `#[data(output_only)]` → excluido del tipo de entrada
- Struct genérico → interfaz genérica de TypeScript (`export interface Paginated<T>`)
- Cuando CUALQUIER campo tiene `input_only` / `output_only` / `lazy`, se emiten dos interfaces: `<Name>` (salida) y `<Name>Input` (entrada)

Los tipos generados nunca filtran tipos exclusivos de Rust
(`Prop<...>` no aparecerá en el `.d.ts` de salida).

## Andamiaje

```bash
suprnova make:inertia UserDto --data
```

Emite un esqueleto `#[derive(Data, Validate)]` en lugar de la plantilla
heredada `#[derive(InertiaProps)]`.

## Siguiente

- [Validación](validation.md) - `#[derive(Validate)]`, los validadores async, y cómo `FormRequest` los invoca
- [Solicitudes](requests.md) - la superficie de extractor de solicitudes en la que se conecta `FormRequest`
- [Respuestas de Inertia](frontend-inertia-responses.md) - la ruta de `Inertia::data` y cómo las props perezosas se vuelven elegibles para recarga parcial
- [Recursos de Eloquent](eloquent-resources.md) - `#[derive(Resource)]` para salidas JSON:API (hermano de `Data` para payloads solo de serialización)
- [Modelo de errores](error-model.md) - cómo el rechazo de `unknown_field` se convierte en un 422 y cómo los fallos de `FormRequest` regresan como `ValidationErrors`

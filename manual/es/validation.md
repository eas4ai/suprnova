# Validación

Suprnova valida la entrada de las solicitudes por dos vías complementarias:

1. **Validación por derive** - atributos `#[validate(...)]` sobre un
   struct `FormRequest`, ejecutados automáticamente por `extract()`. Esta
   es la vía cotidiana y se cubre en [Solicitudes](requests.md). Se
   ocupa de las reglas por campo (`email`, `length`, `range`, …) de forma
   declarativa.
2. **Objetos de regla + la macro `validate!`** - valores simples que
   implementan [`Rule`](#objetos-de-regla) / `ContextualRule` / `AsyncRule`,
   compuestos de forma imperativa. Echa mano de ellos cuando necesites
   lógica entre campos, reglas que tocan la base de datos, o reglas que
   quieras almacenar y pasar de un lado a otro.

Ambas vías se acumulan en la misma bolsa
[`ValidationErrors`](error-model.md) y renderizan la misma forma
Laravel/Inertia `{ "message", "errors": { field: [...] } }` (HTTP
422).

## Objetos de regla

Una regla es un valor que implementa uno de tres traits:

| Trait | Forma | Uso |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | comprobación pura sobre un valor |
| `ContextualRule` | `passes(&self, value, ctx)` | comprobación que lee campos hermanos |
| `AsyncRule` | `async passes(&self, value)` | comprobación que hace `.await` (base de datos, HTTP) |

`Rule` integradas: `Required`, `Email`, `Min`, `Max`, `Between`, `In`,
`NotIn`, `Integer`, `Numeric`, `Boolean`, `Alpha`, `AlphaNum`, `Url`,
`HttpUrl`, `Uuid`. `ContextualRule` integradas: `RequiredIf`,
`RequiredWith`, `RequiredUnless`, `Same`, `Different`, `Confirmed`.
`AsyncRule` integrada: [`Unique`](#la-regla-unique).

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **Nota:** `Numeric` acepta un número **finito** - `NaN`, `inf` y las magnitudes
> que desbordan a infinito se rechazan, aunque el parser de Rust aceptaría
> esas cadenas. Usa `HttpUrl` (no `Url`) para las entradas de
> callback/webhook/avatar: `Url` analiza cualquier esquema que acepte
> `url::Url` (`file:`, `javascript:`, URIs personalizadas), mientras que
> `HttpUrl` exige `http`/`https`.

### Escribir tu propia regla

Una regla personalizada es un struct unitario (o que lleva datos) con un
solo impl. El trait te da `check()` gratis - añade cualquier mensaje de
fallo a una bolsa `ValidationErrors` bajo el campo nombrado - de modo que
la regla encaja sin cambios en `validate!` y en los ganchos
`after_validation`:

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(format!("must start with {}", self.0).into())
        }
    }
}

// Ahora se puede usar en cualquier sitio:
StartsWith("acct_").passes("acct_1234")?;
// o, en una fila de validate!:
//   stripe_id => Required, StartsWith("acct_");
```

Un `String` se convierte en un `ValidationMessage` que se renderiza tal
cual, que es todo lo que necesita una aplicación en un solo idioma. Para
que el mensaje se traduzca por locale, devuelve en su lugar un mensaje
*con clave* -
`ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` -
y define el id en `lang/<locale>/validation.ftl`. Consulta
[Localización](localization.md), que también cubre cómo sobrescribir los
mensajes de las reglas integradas y la convención de nombres
`field-<name>`.

Para la lógica entre campos, implementa [`ContextualRule`] en su lugar -
el método `passes` recibe un `&FormContext` (un `HashMap<String, String>`
con los valores de los campos hermanos) junto al valor bajo prueba. Para
las comprobaciones respaldadas por la base de datos, implementa
[`AsyncRule`] y úsala desde `after_validation_async`.

## La macro `validate!`

`validate!` ejecuta una cadena de reglas sobre los campos de un struct,
acumulando todos los fallos en un único `ValidationErrors`. Es el sitio
idiomático para el gancho síncrono entre campos,
[`after_validation`](#ganchos-entre-campos).

```rust
use suprnova::{validate, ValidationErrors, rules::{Required, Email, Min, Max, RequiredIf}};

fn after_validation(&self) -> Result<(), ValidationErrors> {
    // Las reglas contextuales leen los valores hermanos de un `FormContext`
    // que construyes tú - un mapa de nombre de campo a su valor de cadena.
    let mut ctx = std::collections::HashMap::new();
    ctx.insert("billing_type".to_string(), self.billing_type.clone());
    validate! { self =>
        email       => Required, Email;          // fila de forma requerida
        bio         ?: Min(10), Max(500);        // opcional: valida solo si es Some
        card_number ?=> RequiredIf {             // presencia condicional (ver abajo)
            other: "billing_type",
            value: "card",
        } => with ctx;
    }
}
```

Cada fila tiene una de tres formas:

- **`field => Rule1, Rule2;`** - forma requerida. Las reglas se ejecutan
  directamente sobre `&self.field` (para `String`, `i64`, o cualquier
  cosa que haga deref al préstamo que la regla espera).
- **`field ?: Rule1, Rule2;`** - opcional. El campo es `Option<T>`; las
  reglas se ejecutan solo cuando es `Some`, y **se omiten por completo
  con `None`**. Esta es la semántica "si está presente, valida"
  (`sometimes`) de Laravel.
- **`field ?=> Rule1, Rule2;`** - presencia condicional. También para un
  campo `Option<String>`, pero las reglas se ejecutan **incluso cuando es
  `None`** (la ausencia se trata como la cadena vacía). Esta es la fila
  para las reglas condicionadas a la presencia, como `RequiredIf`, que
  tienen que poder *hacer fallar un campo ausente* - el caso que `?:` no
  puede expresar porque se omite con `None`.

Una regla contextual va seguida de `=> with $ctx` (un
`&HashMap<String, String>` de valores hermanos). La macro es
**síncrona** - para las reglas asíncronas usa el
[gancho](#reglas-asíncronas-en-las-solicitudes) de abajo.

> **Advertencia:** Una trampa habitual: escribir `card_number ?: RequiredIf {...} => with ctx;`.
> En una fila `?:`, `None` omite todas las reglas, así que `RequiredIf`
> nunca puede hacer fallar un campo ausente. Usa `?=>` para cualquier
> regla que deba dispararse ante una ausencia.

## Ganchos entre campos

`FormRequest` ejecuta dos ganchos entre campos después de las reglas por
campo derivadas, tanto en el flujo normal como en el de Precognition.
`extract()` ejecuta las etapas en orden - el `validate()` derivado, luego
`after_validation`, luego `after_validation_async` - y **se detiene en la
primera etapa que falla**.

```rust
use suprnova::{FormRequest, ValidationErrors};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct UpdatePassword {
    #[validate(length(min = 8))]
    pub new_password: String,
    pub confirmation: String,
}

impl FormRequest for UpdatePassword {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        if self.new_password != self.confirmation {
            errs.add("confirmation", "passwords do not match");
        }
        errs.into_result()
    }
}
```

> **Nota:** Sobrescribir los ganchos exige un `impl FormRequest` escrito a mano - el
> atributo `#[request]` y `#[derive(FormRequest)]` generan el suyo propio
> (vacío), así que solo sirven para el caso común en el que no se
> sobrescribe nada.

### Reglas asíncronas en las solicitudes

La macro `validate!` no puede entretejer `.await`, así que las reglas
respaldadas por la base de datos se ejecutan en `after_validation_async` -
la última etapa de validación, a la que `extract()` llama
automáticamente. Ahí es donde [`Unique`](#la-regla-unique) y cualquier
`AsyncRule` personalizada participan en la validación automática de
solicitudes; no hace falta cableado por handler.

```rust
use suprnova::{FormRequest, ValidationErrors, Unique, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct CreateUser {
    #[validate(email)]
    pub email: String,
}

#[async_trait]
impl FormRequest for CreateUser {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Unique::new("users", "email")
            .check_async(&self.email, &mut errs, "email")
            .await;
        errs.into_result()
    }
}
```

Como la etapa asíncrona solo se ejecuta después de que pasen las etapas
síncronas, un valor mal formado (un email sintácticamente inválido) nunca
llega a la consulta `Unique` contra la base de datos.

## La regla `Unique`

`Unique` comprueba que un valor no exista ya en una tabla. Constrúyela
con `Unique::new(table, column)` y refínala con la API fluida:

```rust
use suprnova::Unique;

// el email debe ser único, ignorando la fila que se está editando ahora
Unique::new("users", "email").ignore(current_user_id)

// email único *por tenant*, comparado sin distinguir mayúsculas de minúsculas
Unique::new("users", "email")
    .where_eq("tenant_id", tenant_id)
    .case_insensitive()
```

| Método del builder | Efecto |
|----------------|--------|
| `.ignore(id)` | excluye la fila cuyo `id` sea igual a `id` (el caso de editarse a sí mismo) |
| `.ignore_with_column(col, id)` | excluye por una columna clave distinta de `id` |
| `.where_eq(col, value)` | acota la comprobación a las filas donde `col = value`; varias llamadas se combinan con AND |
| `.case_insensitive()` | compara con `LOWER(col) = LOWER(?)` |

La tabla, la columna, la clave de exclusión y todas las columnas de
`where_eq` se validan contra una lista de identificadores permitidos
antes de llegar a la cadena SQL; el valor bajo prueba y todos los valores
de acotación son parámetros vinculados.

### Unique es orientativa - la restricción de la base de datos es la garantía

`Unique` ejecuta un `SELECT COUNT(*)` **antes** de la escritura, así que
arrastra una condición de carrera inevitable entre el momento de la
comprobación y el del uso: dos solicitudes concurrentes pueden pasar
ambas la comprobación y luego insertar las dos. La regla `unique` de
Laravel tiene exactamente la misma propiedad. La **única** garantía real
es una restricción `UNIQUE` (o un índice único) sobre la columna en tu
migración.

Usa las tres juntas:

1. **La regla orientativa** - un mensaje rápido y amable de "ese email ya
   está en uso" antes del envío (y así Precognition puede validar el
   campo).
2. **La restricción `UNIQUE`** - la salvaguarda definitiva contra la
   condición de carrera.
3. **`FrameworkError::from_unique_violation`** - en el sitio de la
   escritura, mapea la violación de restricción que recibe quien pierde
   la carrera de vuelta al mismo 422 limpio, en lugar de filtrar un 500:

```rust
use suprnova::FrameworkError;

// `users.email` tiene una restricción UNIQUE en la migración.
let user = new_user
    .insert(db)
    .await
    .map_err(|e| FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    ))?;
```

`from_unique_violation` devuelve un error `Validation` 422 cuando el
error de la base de datos es una violación de restricción de unicidad, y
deja pasar cualquier otro error sin cambios (se reconocen MySQL, Postgres
y SQLite).

## Autorización asíncrona

`FormRequest::authorize(&Request) -> bool` se ejecuta **antes** de que se
analice el cuerpo, así que puede rechazar las solicitudes no autorizadas
sin leer el payload. Es síncrono por diseño: en ese punto la solicitud
todavía sostiene el cuerpo en streaming, así que el gancho no puede hacer
`.await`. La autorización que necesita ir a la base de datos o a una
política asíncrona pertenece a uno de estos sitios, no a `authorize`:

- **Middleware** - se ejecuta antes de `extract()`, es `async`, y
  cortocircuita devolviendo `Err(response)` (consulta
  [Middleware](middleware.md)). El sitio correcto para "¿se le permite a
  este usuario llegar siquiera a esta ruta?".
- **El Gate** - llama a `Gate::allows_async` / `Gate::authorize_async` en
  el handler una vez que tengas al usuario autenticado y el recurso
  (consulta [Autorización](authorization.md)).
- **`after_validation_async`** - para una comprobación de autorización
  que dependa del cuerpo ya analizado de la solicitud, ejecútala en el
  gancho asíncrono junto a tus demás reglas asíncronas.

## Notas de diseño

- **Validación parcial.** Un `FormRequest` se deserializa en un struct
  tipado antes de que se ejecute la validación, así que el struct *es* el
  esquema: un campo que pueda estar ausente tiene que ser `Option<T>`.
  Esto es también lo que permite a Precognition validar un payload
  parcial - haz opcionales los campos que un borrador pueda omitir.
- **Mensajes de las reglas.** Las reglas integradas devuelven mensajes
  con clave (`validation-min` más sus argumentos y un fallback en
  inglés), resueltos a través del catálogo en el límite de
  serialización. Traduce o reformula cualquiera de ellos definiendo el
  mismo id en `lang/<locale>/validation.ftl` - sin envolver la regla.
  Consulta [Localización](localization.md).
- **`Min` / `Max` / `Between`** son reglas de longitud de cadena
  (contada en valores escalares Unicode). Para los límites numéricos,
  valida con `#[validate(range(...))]` en el derive o con una regla
  personalizada - las reglas de longitud no son comparaciones de valores.

## Resumen

| Tarea | API |
|------|-----|
| Reglas por campo | `#[validate(...)]` en el `FormRequest` (consulta Solicitudes) |
| Reglas compuestas / entre campos | `validate! { self => ... }` |
| Opcional "si está presente" | `field ?: Rule;` |
| Opcional requerido condicionalmente | `field ?=> Rule => with ctx;` |
| Regla asíncrona / respaldada por la base de datos | `after_validation_async` + `AsyncRule::check_async` |
| Unicidad | `Unique::new(t, c)` + restricción `UNIQUE` + `from_unique_violation` |
| Autorización asíncrona | middleware / `Gate::*_async` / `after_validation_async` |

## Siguiente

- [Solicitudes](requests.md) - la superficie `#[request]` /
  `#[derive(FormRequest)]`, la vía cotidiana de la validación por derive
- [Objetos de datos](data.md) - `#[derive(Data, Validate)]` para un
  único struct que es a la vez solicitud entrante y DTO saliente
- [Modelo de errores](error-model.md) - cómo `ValidationErrors` se
  convierte en el cuerpo JSON 422, junto a todas las demás rutas de error
- [Localización](localization.md) - traducir los mensajes de las reglas,
  la convención `field-<name>`, y los `ValidationMessage` con clave
- [Autorización](authorization.md) - `Gate`, `Policy`, y dónde encaja la
  autorización respecto a la validación
- [Middleware](middleware.md) - el sitio correcto para las
  comprobaciones del tipo "¿se permite siquiera que pase esta solicitud?"
  que necesitan `.await`

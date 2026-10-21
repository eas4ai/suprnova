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

Una regla es un valor que implementa uno de cuatro traits:

| Trait | Forma | Uso |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | comprobación pura sobre un valor |
| `ValueRule` | `passes(&self, value: &serde_json::Value)` | comprobación sobre un valor con forma JSON (array/objeto) |
| `ContextualRule` | `passes(&self, value, ctx)` | comprobación que lee campos hermanos |
| `AsyncRule` | `async passes(&self, value)` | comprobación que hace `.await` (BD, HTTP) |

`Rule`s integradas: `Required`, `Email`, `Min`, `Max`, `Between`, `In`,
`NotIn`, `InArray`, `Integer`, `Numeric`, `Boolean`, `Alpha`, `AlphaNum`,
`AlphaDash`, `Url`, `UrlProtocols`, `HttpUrl`, `Uuid`,
[`Password`](#fortaleza-de-la-contraseña) (solo las comprobaciones de
fortaleza). `ValueRule`s integradas: `ArrayKeys`, `Distinct`, `Contains`,
`DoesntContain`. `ContextualRule`s integradas: `RequiredIf`,
`RequiredWith`, `RequiredUnless`, `Same`, `Different`, `Confirmed`, `Gt`,
`Gte`, `Lt`, `Lte`. `AsyncRule`s integradas: [`Unique`](#la-regla-unique)
y [`Password`](#fortaleza-de-la-contraseña) (la fortaleza más su
comprobación HIBP `uncompromised()` - la única regla integrada que
implementa a la vez `Rule` y `AsyncRule`).

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **Nota:** `Numeric` acepta un número **finito** - `NaN`, `inf` y las
> magnitudes que desbordan a infinito se rechazan, aunque el parser de
> Rust aceptaría esas cadenas.

### Esquemas de URL

`Url` acepta un valor que se parsea como URL, cuyo esquema está en la
lista de permitidos de Laravel - la misma lista que usa
`Illuminate\Support\Str::isUrl` -, va seguido de `://` **y** va seguido a
su vez de un host no vacío, con la misma forma que el patrón
`^(PROTOCOLS)://HOST` de Laravel (el grupo del host de Laravel no lleva
`?`: un host ausente o vacío nunca casa). La lista de esquemas y la
exigencia de `://` más host son literalmente las de Laravel; el host lo
parsea el crate `url` en lugar de la expresión regular de Laravel, así
que aquí se rechaza un puerto fuera de rango que Laravel aceptaría. Las
tres condiciones tienen que cumplirse: `mailto:`, `tel:` y `data:` están
en la lista de permitidos por nombre, pero no llevan componente de
autoridad alguno, así que `Url` los rechaza; y `file:///etc/passwd` falla
por la tercera razón: tiene `://`, pero entre la tercera y la cuarta `/`
no hay nada, y nada no es un host. `javascript:` y `vbscript:` se
rechazan de plano; ni siquiera están en la lista de permitidos.

`ftp://host/x` y `ssh://host` - hosts reales, solo que no son esquemas
web - siguen pasando, así que `Url` no es una comprobación de "esto es
una página web", y no dice nada sobre adónde resuelve la URL. Rechazar
`javascript:` hace que un valor validado sea seguro de poner en un
`href`, no seguro de descargar. Un destino de webhook o de callback sigue
necesitando `HttpUrl` (o tus propias comprobaciones de esquema y de
SSRF); `Url` por sí sola no cubre eso.

Para un conjunto más estrecho, nombra los esquemas que quieras:

```rust
use suprnova::{Rule, rules::Url};

// El `url:http,https` de Laravel
Url::protocols(&["https"]).passes("https://example.com")?;   // Ok
Url::protocols(&["https"]).passes("http://example.com");     // Err

// Lo mismo, con nombre propio
use suprnova::rules::HttpUrl;
HttpUrl.passes("https://example.com")?;
```

`Url::protocols(...)` **sustituye** la lista de permitidos en lugar de
estrecharla, de modo que una aplicación puede aceptar su propio esquema
de enlace profundo (`myapp://…`) sin que el framework opine al respecto:
la exigencia de `://` más host también sigue aplicándose a ese esquema
propio. Usa `HttpUrl` (o `Url::protocols(&["https"])`) para las entradas
de callback, de webhook y de avatar - un destino de webhook que resuelve
a `ftp://internal-host/` sigue parseándose como `Url`, y un destino
`ftp:` no es un destino de webhook.

### Fortaleza de la contraseña

`Password` comprueba la longitud y la fortaleza por clases de caracteres,
más una comprobación opcional `uncompromised()` contra Have I Been
Pwned - el objeto de regla `Password` de Laravel, portado. Constrúyelo
con `Password::min(n)` y encadena los constructores de fortaleza:

```rust
use suprnova::{Password, Rule};

let rule = Password::min(8).letters().mixed_case().numbers().symbols();
Rule::passes(&rule, "Str0ng! Pass")?; // Ok(())
Rule::passes(&rule, "weak");          // Err - demasiado corta, sin dígito, sin símbolo
```

| Constructor | Exige | Expresión regular de Laravel |
|---|---|---|
| `.min(n)` (vía `Password::min`) | al menos `n` caracteres (con suelo en 1) | comprobación de longitud |
| `.max(n)` | como mucho `n` caracteres | comprobación de longitud |
| `.letters()` | al menos una letra Unicode | `/\pL/u` |
| `.mixed_case()` | una letra mayúscula y una minúscula, en cualquier orden | `/(\p{Ll}+.*\p{Lu})\|(\p{Lu}+.*\p{Ll})/u` |
| `.numbers()` | al menos un dígito Unicode | `/\pN/u` |
| `.symbols()` | al menos un separador, símbolo o signo de puntuación - **un espacio simple cuenta** | `/\p{Z}\|\p{S}\|\p{P}/u` |

`Password::defaults_with(|| Password::min(12).letters().mixed_case().numbers())`,
llamado una sola vez desde `bootstrap::register()`, fija la política por
defecto de todo el proceso que `Password::defaults()` devuelve en
cualquier otro sitio - el `Password::defaults(fn () => ...)` de Laravel.
Una segunda llamada se ignora (con un `tracing::warn!`) en lugar de
reemplazar en silencio la política que ya eligió la primera aplicación.

#### `uncompromised()` - porque la fortaleza por sí sola no basta

`.uncompromised()` (o `.uncompromised_with_threshold(n)`) añade una
comprobación contra el corpus de filtraciones de Have I Been Pwned,
usando su API de rangos con k-anonimato: del hash SHA-1 en mayúsculas de
la contraseña solo salen del proceso los **5 primeros caracteres** - `GET
https://api.pwnedpasswords.com/range/{prefix}` - y la comparación con el
hash completo ocurre localmente, contra las líneas `SUFFIX:COUNT` que la
API devuelve para ese prefijo. El servicio nunca ve la contraseña, ni
siquiera su hash completo. La comparación con el umbral es estricta
(`count > threshold`), así que el `uncompromised()` por defecto (umbral
`0`) falla ante cualquier aparición, y un fallo de red, un tiempo de
espera agotado o una respuesta que no sea 2xx **falla abierto**: la
contraseña se trata como limpia en lugar de bloquear todos los registros
durante una caída de Have I Been Pwned. Esto coincide exactamente con el
`NotPwnedVerifier` de Laravel.

Como esa comprobación es una ida y vuelta HTTP, `uncompromised()`
necesita `AsyncRule`, no el `Rule` síncrono que basta para las
comprobaciones de fortaleza por sí solas. Conéctala a través de
`after_validation_async`, la misma receta que usa
[`Unique`](#la-regla-unique):

```rust
use suprnova::{AsyncRule, FormRequest, Password, ValidationErrors, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct Register {
    pub password: String,
}

#[async_trait]
impl FormRequest for Register {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Password::defaults()
            .uncompromised()
            .check_async(&self.password, &mut errs, "password")
            .await;
        errs.into_result()
    }
}
```

Llamar al `Rule::passes` síncrono sobre un `Password` que tiene
`uncompromised()` puesto es un **error estrepitoso**, no una omisión
silenciosa: una comprobación de seguridad que no hace nada en silencio es
peor que una que nunca existió. El mensaje de error nombra
`after_validation_async` como solución.

`HIBP_TIMEOUT_SECS` (por defecto `30`) controla el tiempo de espera de la
solicitud - véanse las [Variables de entorno](env-vars.md).

Un verificador propio que devuelve `Err` es un caso distinto de una
comprobación fallida: su texto de error se registra a nivel `error` y
nunca llega al cliente, y la respuesta lleva en su lugar la clave de
catálogo `validation-password-unverifiable` ("The { $field } could not be
checked against known data leaks. Please try again."). Añade esa clave si
publicas tu propio catálogo de validación.

### Por qué Suprnova diverge: Password

- El `Password` de Laravel reúne todas las comprobaciones de fortaleza
  fallidas en un único array. El contrato `Rule` de Suprnova devuelve un
  único `ValidationMessage`, así que `Rule::passes` informa de la PRIMERA
  comprobación que falla, en el orden mínimo, máximo, mayúsculas y
  minúsculas, letras, símbolos, números - se corrige de una en una en
  lugar de ver la lista entera de golpe.
- El validador síncrono de Laravel puede llamar a `uncompromised()`
  directamente; una solicitud PHP ya está dentro de un bucle de eventos
  que tolera una llamada HTTP bloqueante. El `Rule::passes` de Suprnova
  es síncrono por contrato, así que no hay un sitio seguro desde el que
  lanzar la solicitud a HIBP. En lugar de omitir la comprobación en
  silencio - el único desenlace inaceptable para una regla relevante para
  la seguridad -, el `Rule::passes` de Suprnova devuelve un error
  estrepitoso, dirigido al desarrollador, que nombra
  `after_validation_async` como solución.
- `Password::defaults_with` toma un puntero a `fn` simple, no un closure,
  de modo que el valor por defecto configurado sigue siendo `Copy` y no
  necesita reservar memoria en el heap - un estrechamiento deliberado
  respecto del `Closure` de Laravel.

### Escribir tu propia regla

Una regla propia es un struct unitario (o portador de datos) con una sola
impl. El trait te da `check()` gratis - empuja cualquier mensaje de fallo
a una bolsa `ValidationErrors` bajo el campo nombrado -, así que la regla
encaja sin cambios en `validate!` y en los ganchos `after_validation`:

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

// Ya se puede usar en cualquier parte:
StartsWith("acct_").passes("acct_1234")?;
// o, en una fila de `validate!`:
//   stripe_id => Required, StartsWith("acct_");
```

Un `String` se convierte en un `ValidationMessage` que se renderiza tal
cual, que es cuanto necesita una aplicación monolingüe. Para que el
mensaje se traduzca por locale, devuelve en su lugar un mensaje *con
clave* -
`ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` -
y define el id en `lang/<locale>/validation.ftl`. Véase
[Localización](localization.md), que también cubre cómo anular los
mensajes de las reglas integradas y la convención de nombres
`field-<name>`.

Para la lógica entre campos, implementa [`ContextualRule`] en su lugar -
el método `passes` recibe un `&FormContext` (un `HashMap<String, String>`
con los valores de los campos hermanos) junto al valor bajo prueba. Para
las comprobaciones respaldadas por la base de datos, implementa
[`AsyncRule`] y úsala desde `after_validation_async`.

### Reglas con forma de valor

`Rule` solo llega a ver `&str`. Dos reglas integradas necesitan más
estructura de la que lleva una cadena, así que implementan `ValueRule` en
su lugar, sobre `&serde_json::Value`:

```rust
use suprnova::{ValueRule, rules::{ArrayKeys, Distinct}};

// El array:keys de Laravel - rechaza las claves fuera del conjunto
// permitido. No hace falta que estén presentes todas las claves
// listadas; una lista permitida vacía es un error de programación, y se
// informa como un mensaje sin clave.
ArrayKeys(&["name", "email"]).passes(&serde_json::json!({"name": "Ada"}))?;

// El distinct / distinct:ignore_case / distinct:strict de Laravel.
Distinct { ignore_case: false, strict: false }
    .passes(&serde_json::json!(["a", "b", "c"]))?;
```

Un campo validado por una `ValueRule` tiene que contener el propio
`serde_json::Value` (u `Option<serde_json::Value>` para una fila `?:` o
`?=>`) - normalmente un campo de la solicitud sacado directamente del
cuerpo JSON. Las filas de `validate!` aceptan `Rule`s y `ValueRule`s en
la misma lista de campos; qué trait se ejecuta lo resuelve cuál de los
dos implementa el tipo de la regla, no nada que escribas en la fila.

### Reglas de pertenencia

Tres reglas responden a "¿está este valor en aquella lista?", cada una
sobre la forma que necesita:

```rust
use suprnova::{Rule, ValueRule, rules::{Contains, DoesntContain, InArray}};

// El in_array:allowed_roles.* de Laravel - el valor tiene que aparecer en
// la lista de otro campo. Pasa la lista misma: sirven tanto un campo
// Vec<String> como un literal &[&str].
InArray(&form.allowed_roles).passes(&form.role)?;

// El contains:rust,web de Laravel - el array tiene que contener todos los
// valores listados.
Contains(&["rust", "web"]).passes(&form.tags)?;

// El doesnt_contain:banned de Laravel - el array no puede contener
// ninguno de ellos.
DoesntContain(&["banned"]).passes(&form.tags)?;
```

Todas las comparaciones son exactas. `InArray` compara cadenas con `==`, y
`Contains` y `DoesntContain` solo casan un parámetro contra un elemento
JSON de tipo cadena, así que `["1"]` contiene `"1"` y `[1]` no. Un valor
que no sea un array falla de plano en `Contains` y en `DoesntContain`.

`Contains` y `DoesntContain` rechazan una lista de parámetros vacía como
un error de construcción sin clave, igual que hace `ArrayKeys` - una lista
sin nada dentro no restringe nada. Una lista de búsqueda vacía en
`InArray` es distinta: un campo hermano puede estar legítimamente vacío en
tiempo de ejecución, así que el valor simplemente falla.

El mensaje de fallo de `InArray` no nombra ningún valor, porque su lista
viene de la solicitud y un mensaje de validación se renderiza en el cuerpo
de una respuesta.

### Reglas de comparación

`Gt`, `Gte`, `Lt` y `Lte` comparan un campo con un número o con otro
campo. `CompareWith` nombra a la vez el operando y la medida:

```rust
use suprnova::{ContextualRule, FormContext, rules::{CompareWith, Gt, Lte}};

let mut ctx = FormContext::new();
ctx.insert("max_price".to_string(), form.max_price.clone());

// El gt:0 de Laravel - un operando literal, comparado numéricamente.
Gt(CompareWith::Number(0.0)).passes(&form.price, &ctx)?;

// El lte:max_price de Laravel - un campo hermano, comparado numéricamente.
Lte(CompareWith::NumericField("max_price")).passes(&form.price, &ctx)?;

// El gt:summary de Laravel sobre dos campos de cadena - comparado por
// número de caracteres.
Gt(CompareWith::LengthField("summary")).passes(&form.body, &ctx)?;
```

Las cuatro leen campos hermanos, así que son `ContextualRule`s y toda fila
de `validate!` lleva `=> with ctx` - incluida una fila cuyo único operando
es un literal, donde el contexto no llega a leerse. Ahí se pasa un
`FormContext` vacío.

Todo lo que la regla no puede medir hace fallar el campo: un valor que no
es un número finito bajo una comparación numérica, un hermano que el
formulario nunca envió, un hermano que no es un número, o un literal no
finito como `f64::NAN`. Ninguno de esos casos provoca un pánico, y ninguno
pasa.

### Por qué Suprnova diverge

El `distinct:strict` de Laravel se apoya en el `==` coercitivo de PHP.
Los valores JSON ya vienen tipados, así que el `strict` de Suprnova solo
cambia si dos *números* con representaciones internas distintas (`1`
frente a `1.0`) cuentan como iguales - nunca hace que una cadena y un
número sean "lo mismo", en ninguno de los dos modos.

Laravel escribe el otro campo dentro de una cadena de regla -
`in_array:allowed_roles.*` - y el validador lo saca de los datos de la
solicitud con un glob en tiempo de ejecución. Suprnova no tiene parser de
cadenas de regla: a `InArray` se le pasa la lista directamente, y el
compilador comprueba que el campo existe.

Laravel 13.27 endureció `in`, `in_array` y `doesnt_contain` hasta la
comparación estricta porque el `==` de PHP convertía `"1abc"`, `true` y
`"0x1"` en coincidencias. Suprnova nunca tuvo ese agujero - `In` y `NotIn`
comparan `&str` con `==` - y las reglas nuevas casan los valores JSON
variante por variante. El `contains` de Laravel se quedó laxo; el de
Suprnova no. El coste es que estas reglas no pueden comprobar un array
numérico: `Contains(&["1"])` no casa con `[1]`.

La familia `gt` de Laravel elige su medida en tiempo de ejecución: el
número mismo para los numéricos, `count()` para los arrays, kilobytes para
los ficheros y el número de caracteres para todo lo demás, con la rama
numérica condicionada a si el campo lleva además `numeric` o `integer`.
Suprnova escribe la medida dentro de la regla, porque aquí una regla no
puede ver las demás reglas de su campo y olfatear la forma del valor es
justo la costumbre coercitiva que estas reglas existen para evitar. Dos de
las cuatro medidas de Laravel no tienen contraparte alguna: una regla solo
recibe cadenas, así que no se puede leer un hermano con valor de array, y
las subidas de ficheros nunca llegan a la superficie de las reglas - el
parser multipart limita su tamaño antes de que un handler las vea.

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
  cosa que haga deref al préstamo que la regla espera) o, para una
  `ValueRule`, directamente sobre un campo `serde_json::Value`. La
  macro infiere automáticamente qué trait usa cada regla.
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

## Envíos de formularios Inertia

Un fallo de validación responde de forma distinta a dos públicos. Un
cliente REST recibe `422` con `{ message, errors }`. Una visita de Inertia
recibe un `303` de vuelta a la página del formulario con los errores en
flash en la sesión, porque el cliente Inertia muestra un modal de error
para cualquier respuesta que no reconoce como respuesta Inertia; un
`422` nunca rellenaría `form.errors`.

El handler no cambia. En la página de destino cada campo lleva su primer
mensaje como cadena:

```svelte
{#if errors?.email}
  <p class="text-red-600">{errors.email}</p>
{/if}
```

Consulta [Respuestas de Inertia](frontend-inertia-responses.md#validation-failures)
para los grupos de errores, `with_all_errors` y el destino de la
redirección.

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
| Regla con forma JSON (array/objeto) | `field => ArrayKeys(&[...]);` / `field => Distinct { .. };` |
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

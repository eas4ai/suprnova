# Localización

La localización en Suprnova es un único módulo con cuatro caras:
catálogos de mensajes en el servidor, errores de validación que llegan
ya traducidos, los *mismos* bytes de catálogo entregados al navegador,
y formateo de números, fechas y listas consciente del locale. El
formato de mensajes es
[Fluent](https://projectfluent.org) - el `.ftl` de Mozilla, el mismo
que usa Firefox - y todo el subsistema está activo por defecto detrás
de la feature `localization`.

El recorrido más corto posible. Escribe un catálogo:

```ftl
# lang/en/app.ftl
welcome = Welcome to { $app }!
```

```ftl
# lang/es/app.ftl
welcome = ¡Bienvenido a { $app }!
```

Úsalo desde un handler:

```rust
use suprnova::{__, handler, HttpResponse, Request, Response};

#[handler]
pub async fn greet(_req: Request) -> Response {
    Ok(HttpResponse::text(__!("welcome", app: "Suprnova")))
}
```

Una solicitud con `Accept-Language: es` obtiene el string en español,
porque `LocaleMiddleware` resolvió el locale antes de que se ejecutara
tu handler. Nada más cambia en el handler - no se pasa ningún
parámetro de locale, no hay ningún `&Translator` en la firma.

## Por qué la localización

Tres razones por las que esto es una preocupación del framework, y no
un crate que eliges tú:

- **Los mensajes de validación son strings del framework, no tuyos.**
  "The email field is required." se emite en las profundidades de
  `Rule::passes`, lejos de cualquier código que poseas. A menos que el
  framework lleve una costura de traducción, una app en español ofrece
  errores de validación en inglés - o envuelves cada regla a mano. Las
  reglas integradas de Suprnova devuelven mensajes *con clave*; los
  traduces soltando un archivo `.ftl`, sin tocar nunca las reglas.
- **El navegador necesita los mismos strings.** Una app de Inertia
  renderiza la mitad de su texto en Rust y la mitad en Svelte/React/Vue.
  Dos sistemas de traducción significan dos formatos de archivo, dos
  flujos de revisión, y dos ocasiones para que la misma frase se
  desalinee. Suprnova sirve el catálogo exacto que resolvió el
  servidor, desde `/_suprnova/lang/<locale>.ftl`, y los starter kits lo
  parsean con `@fluent/bundle` - un único conjunto de archivos, una
  única fuente de verdad.
- **Los plurales y los formatos son datos CLDR, no concatenación de
  strings.** El inglés tiene dos categorías de plural, el ruso y el
  polaco cuatro, el árabe seis. Un número es `1,234.56` en `en-US` y
  `1.234,56` en `de-DE`. Fluent selecciona sobre categorías de plural
  CLDR e ICU4X hace el formateo, así que ninguno de los dos es algo que
  tengas que programar a mano por locale.

Desactivar la feature (`--no-default-features`) es compatible: el
módulo de localización no se compila, y la validación renderiza sus
strings de fallback en inglés incrustados. Nada más cambia de forma.

## Layout de archivos

Los catálogos viven bajo `lang/`, un directorio por locale:

```
myapp/
├── lang/
│   ├── en/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── es/
│       ├── app.ftl
│       └── validation.ftl
├── src/
└── frontend/
```

Las reglas:

- **El nombre de un directorio es un locale BCP-47** - `en`, `en-GB`,
  `pt-BR`, `zh-Hans`. Un directorio cuyo nombre no se puede analizar se
  omite con un `warn!` en lugar de hacer fallar el arranque.
- **Cada `.ftl` dentro del directorio de un locale se fusiona en un
  único catálogo**, en orden alfabético de nombre de archivo. Divídelo
  por feature (`auth.ftl`, `billing.ftl`, `emails.ftl`) tanto como
  quieras - los ids de mensaje son globales dentro del locale, así que
  `auth.ftl` y `billing.ftl` no pueden definir el mismo id.
- **El propio catálogo de validación en inglés del framework se carga
  primero**, en el bundle de cada locale. Tus archivos se cargan
  encima, y gana la definición más reciente. Ese es todo el mecanismo
  de override: define `validation-min` en `lang/es/validation.ftl` y
  el bundle en español usa el tuyo.
- **La raíz es `lang_path()`** - `<APP_BASE_PATH>/lang`. Fija
  `APP_BASE_PATH` cuando el binario se ejecuta desde algún sitio que
  no sea la raíz del proyecto (una unidad systemd, un contenedor con
  un `WorkingDirectory` distinto), o llama a `use_lang_path("…")` para
  mover solo el directorio `lang`. Ver [Variables de entorno](env-vars.md).
- **Un directorio `lang/` ausente no es un error.** Una app recién
  creada debe arrancar, así que el translator arranca solo con el
  catálogo en inglés incrustado y nada más. Un `.ftl` *mal formado* es
  otra historia: los errores de parseo hacen fallar el arranque,
  nombrando el archivo y lo que objetó el parser, porque un catálogo
  cargado a medias en silencio es peor que un proceso detenido.
- **En `local` y `development`, los catálogos tienen recarga en
  caliente.** Cada solicitud hace `stat` sobre `lang/` y solo vuelve a
  parsear cuando algo realmente cambió, así que editar un `.ftl` se ve
  reflejado en el siguiente refresco. Producción nunca vuelve a hacer
  `stat`; los catálogos se leen una sola vez al arrancar.

## FTL en cinco minutos

Fluent es un formato pequeño. Esta sección es todo lo que necesitas
para una app típica.

**Los mensajes** son pares `id = value`. Los ids son kebab-case por
convención (los del propio framework lo son), los valores llegan hasta
el final de la línea, y las líneas de continuación con sangría se
unen:

```ftl
# Un comentario. Adjunto al mensaje de abajo.
sign-in = Iniciar sesión
password-hint =
    Usa al menos 12 caracteres. Una passphrase de unas
    pocas palabras corrientes vence a una cadena corta de símbolos.
```

**Los argumentos** son placeables `{ $name }`. Los suministras en el
momento de la llamada; los argumentos faltantes son un error, no un
string vacío (`Lang::get` entonces recorre su cadena - ver
[La fachada `Lang`](#la-fachada-lang)):

```ftl
greeting = ¡Hola, { $name }!
invoice-line = { $qty } × { $item }
```

**Los términos** empiezan con `-`, son privados del catálogo, y
existen para que un nombre de marca o una frase repetida viva en un
solo sitio:

```ftl
-product-name = Suprnova
about = Acerca de { -product-name }
footer = © 2026 { -product-name }. Todos los derechos reservados.
```

**Los selectores** son el condicional de Fluent. El valor del selector
se compara contra las claves de las variantes; exactamente una
variante se marca como predeterminada con `*`:

```ftl
cart-summary =
    { $count ->
        [0] Tu carrito está vacío.
        [one] Un artículo en tu carrito.
       *[other] { $count } artículos en tu carrito.
    }
```

`[0]` coincide con el número literal cero. `[one]` y `[other]` son
**categorías de plural CLDR**, resueltas para el locale del bundle -
que es donde Fluent se gana el puesto. El inglés tiene dos categorías;
el ruso tiene cuatro, y un traductor ruso escribe las cuatro sin que
tú cambies una sola línea de Rust:

```ftl
# lang/ru/app.ftl
unread-messages =
    { $count ->
        [one] У вас { $count } непрочитанное сообщение.
        [few] У вас { $count } непрочитанных сообщения.
        [many] У вас { $count } непрочитанных сообщений.
       *[other] У вас { $count } непрочитанного сообщения.
    }
```

CLDR asigna `1`, `21`, `31` a `one`; `2`-`4`, `22`-`24` a `few`;
`0`, `5`-`20`, `25`-`30` a `many`; y las fracciones a `other`. La misma
llamada `__!("unread-messages", count: 22)` se renderiza correctamente
en inglés, ruso, polaco y árabe, porque la selección de categoría es
dato, no código.

**Pon siempre el `*` en `other`.** Es la única categoría que CLDR
define para todos los locales, así que es la única variante que tiene
garantizado existir - y el valor por defecto es a lo que cae un valor
de selector sin match, incluyendo cualquier count que no sea entero.
Marcar `*[many]` (o cualquier otra categoría) como el valor por
defecto envía las fracciones a un texto escrito para números enteros.

> **Pasa los counts como números.** `__!("unread-messages", count: 3)`
> envía un número JSON y selecciona una categoría de plural. `count:
> "3"` envía un string, que solo puede coincidir con una clave de
> variante literal - caerá en tu valor por defecto `*[other]`. Esta es
> la trampa de FTL que más vale memorizar.

**Las funciones** se llaman dentro de los placeables. Se registran
dos: `NUMBER()` (integrada en Fluent) y `DATETIME()` (de Suprnova):

```ftl
score = Your score is { NUMBER($points) } out of { NUMBER($total) }.
published = Published { DATETIME($when, dateStyle: "medium") }
```

Ver [Formateo consciente del locale](#formateo-consciente-del-locale)
para ambas.

**Una limitación deliberada:** Suprnova solo resuelve *valores* de
mensaje planos. La sintaxis de atributos de Fluent (`login .placeholder
= …`) se parsea, pero no es direccionable a través de `Lang::get`, así
que mantén un id por string: `login-placeholder`, no
`login.placeholder`. Los ids son un namespace plano por locale -
prefíjalos (`auth-login-title`, `billing-invoice-due`) en lugar de
recurrir a una jerarquía que el resolver no tiene.

## La fachada `Lang`

`Lang` es el punto de entrada del lado del servidor. Cada método lee
el **locale actual**, que el middleware vinculó para esta solicitud.

| Método | Devuelve | Notas |
|---|---|---|
| `Lang::get(key)` | `String` | Infalible. Recorre la cadena de fallback y luego devuelve la clave misma |
| `Lang::get_with(key, args)` | `String` | Igual, con argumentos |
| `Lang::try_get(key)` | `Result<String, FrameworkError>` | Falla en lugar de degradarse |
| `Lang::try_get_with(key, args)` | `Result<String, FrameworkError>` | Igual, con argumentos |
| `Lang::has(key)` | `bool` | Si la clave resuelve para el locale actual, o en algún punto de su cadena de fallback |
| `Lang::locale()` | `Locale` | El locale actual |
| `Lang::set_locale(locale)` | `()` | Cámbialo para el resto de esta solicitud |
| `Lang::available_locales()` | `Vec<Locale>` | Cada locale con un catálogo cargado |

```rust
use suprnova::{Lang, Locale, TranslateArgs};

let subject = Lang::get("password-reset-subject");

let mut args = TranslateArgs::new();
args.insert("name".into(), serde_json::json!("Ada"));
args.insert("count".into(), serde_json::json!(3));
let body = Lang::get_with("unread-messages", args);

if Lang::has("beta-banner") {
    // Solo algunos locales ofrecen el copy del banner.
}

let locales: Vec<String> = Lang::available_locales()
    .iter()
    .map(Locale::as_str)
    .collect();
```

`TranslateArgs` es un mapa ordenado de `String` a `serde_json::Value`,
ambos reexportados desde la raíz del crate. Los argumentos de Fluent
son strings y números; otras formas de JSON se convierten a string.

### La cadena de fallback

`Lang::get` nunca falla, y nunca devuelve un string vacío. En orden:

1. El catálogo del **locale actual**.
2. Sus **padres de fallback configurados** (ver
   [Cadenas de fallback](#cadenas-de-fallback)), recorridos de forma
   transitiva, si hay alguno configurado - `pt-PT` antes que `pt-BR`
   antes que lo que `pt-BR` mismo nombre como padre, y así
   sucesivamente.
3. El catálogo del **locale de fallback** (`APP_FALLBACK_LOCALE`, por
   defecto `en`), a menos que ya haya aparecido antes en esta cadena.
4. La **clave misma**, más un `tracing::warn!` por cada par
   `(locale, key)` faltante - una vez, no una vez por solicitud, para
   que una clave faltante en una ruta de ejecución frecuente no inunde
   tus logs.

El paso 4 es la razón por la que una traducción faltante renderiza
`checkout-submit` en el botón en lugar de un botón en blanco: un
string visiblemente incorrecto es un reporte de bug esperando a
suceder, mientras que uno vacío es un misterio.

Cuando prefieras saberlo en lugar de degradarte, usa las contrapartes
`try_*`. Ejecutan los pasos 1 a 3 y devuelven `Err` en lugar de hacer
el paso 4:

```rust
use suprnova::Lang;

// Una clave faltante aquí significa un correo roto - haz fallar el job,
// no envíes un mensaje con una clave en crudo en la línea de asunto.
let subject = Lang::try_get("invoice-paid-subject")?;
```

### La macro `__!`

`__!` es el atajo de memoria muscular de Laravel. Sin argumentos llama
a `Lang::get`; con argumentos con nombre construye un `TranslateArgs` y
llama a `Lang::get_with`:

```rust
use suprnova::__;

let plain = __!("welcome-back");
let greeted = __!("greeting", name: "Ada");
let counted = __!("unread-messages", name: "Ada", count: 3);
```

Los valores de argumento son cualquier cosa que se convierta en un
`serde_json::Value` - `&str`, `String`, enteros, floats, `bool`. La
macro se exporta en la raíz del crate, así que `suprnova::__!("welcome-back")`
funciona sin el import cuando prefieras no traer `__` al scope.

## Cadenas de fallback

`APP_FALLBACK_LOCALE` es una única red global bajo cada locale. A
veces eso no basta: el portugués europeo y el portugués brasileño
comparten casi todo y divergen en un puñado de palabras
(`ficheiro`/`arquivo`, `utilizador`/`usuário`, `tu`/`você`), y mantener
dos catálogos completos significa que cada string nuevo hay que
escribirlo dos veces. Un **padre de fallback** deja que `pt-PT` herede
de `pt-BR` antes de que `pt-BR` caiga más atrás hasta el
`fallback_locale` global - así que `lang/pt-PT/` solo tiene que
contener los strings que de verdad son distintos.

### Configurar los padres

Una variable de entorno, con pares `child=parent` separados por comas:

```env
APP_LOCALE_PARENTS=pt-PT=pt-BR
```

O el builder, una llamada por par, encadenable:

```rust
use suprnova::{Config, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .parent(
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

Ambas vías alimentan el mismo mapa (`LocalizationConfig::parents`), y
ambas se validan al arrancar, no en tiempo de solicitud:

- Un par sin `=`, o con un hijo o padre vacío, es una entrada mal
  formada de `APP_LOCALE_PARENTS` - el arranque falla nombrando el
  segmento incorrecto.
- Un locale inválido como BCP-47 en cualquiera de los dos lados del
  par falla de la misma forma.
- Nombrar al mismo hijo dos veces es una config ambigua, no
  gana-el-último - el arranque falla nombrando el hijo duplicado.
- **Un ciclo hace fallar el arranque.** El error deletrea el ciclo: dos
  locales que se nombran mutuamente (`pt-PT=pt-BR,pt-BR=pt-PT`)
  producen `` `pt-PT` -> `pt-BR` -> `pt-PT` ``. Un locale que se nombra
  a sí mismo como su propio padre (`pt-PT=pt-PT`) es el mismo caso en
  miniatura - `` `pt-PT` -> `pt-PT` ``. (Dos rutas de código lanzan
  este error: el parseo de `APP_LOCALE_PARENTS` - así que cualquier app
  cuya config pase por `LocalizationConfig::from_env()` falla en la
  carga de config - y la carga de catálogo de `FluentTranslator`, que
  atrapa un mapa cíclico construido programáticamente con
  `.parent(...)`. Solo una app que construye su config enteramente a
  mano *y* vincula su propio `Translator` personalizado en
  `bootstrap_fn` se salta ambas; el recorrido de `Lang` está protegido
  de forma independiente y de todos modos termina de forma segura ahí,
  solo que no obtiene el error ruidoso en tiempo de arranque.)

El `.parent(child, parent)` del builder es gana-la-última-escritura
para un hijo repetido - una llamada posterior que sobrescribe una
anterior es solo una anulación posterior, no el caso de entrada
ambigua contra el que protege `APP_LOCALE_PARENTS`.

### Orden de resolución

Una cadena puede tener más de un salto de longitud: `pt-PT` nombra a
`pt-BR` como su padre, y `pt-BR` puede a su vez nombrar un padre
propio. `Lang::get` / `try_get` / `get_with` / `try_get_with` / `has`
recorren todo, primero el locale actual:

1. El catálogo del **locale actual**.
2. Su **padre configurado**, y luego el padre configurado de *ese*
   locale, de forma transitiva, hasta llegar a un locale sin padre
   configurado.
3. El **`fallback_locale`** global (`APP_FALLBACK_LOCALE`), a menos
   que ya haya aparecido antes en la cadena - incluyendo el caso común
   donde es simplemente el propio locale actual (el valor por defecto
   `en`/`en`).

`Lang::get` / `Lang::get_with` caen a la clave misma si nada en la
cadena la resuelve, exactamente como describe
[La cadena de fallback](#la-cadena-de-fallback); `Lang::try_get` /
`Lang::try_get_with` devuelven `Err`, y `Lang::has` devuelve `false`.
Este recorrido se ejecuta dentro de la propia fachada `Lang`, así que
funciona para **cualquier** `Translator` - el `FluentTranslator`
incluido, o un driver que escribas tú.

### Un ejemplo ejecutable

```
myapp/
├── lang/
│   ├── pt-BR/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── pt-PT/
│       └── app.ftl
├── src/
└── frontend/
```

```ftl
# lang/pt-BR/app.ftl
welcome = Bem-vindo ao { $app }!
file-label = Arquivo
```

```ftl
# lang/pt-PT/app.ftl
file-label = Ficheiro
```

```rust
use suprnova::__;

// Una solicitud que resolvió a `pt-PT`.
assert_eq!(__!("file-label"), "Ficheiro");                    // override propio de pt-PT
assert_eq!(
    __!("welcome", app: "Suprnova"),
    "Bem-vindo ao Suprnova!"                                  // heredado de pt-BR
);
```

`lang/pt-PT/` nunca define `welcome` - no lo necesita. `file-label` es
una diferencia genuina de una sola palabra entre los dos catálogos,
así que es el único id que se gana un archivo.

### Los catálogos servidos se aplanan

El endpoint `/_suprnova/lang/pt-PT.ftl` (ver
[El endpoint del catálogo](#el-endpoint-del-catálogo)) nunca le pide
al navegador que sepa que `pt-BR` existe. `FluentTranslator`
pre-fusiona toda la cadena en un único recurso por locale en el
momento de la carga - el catálogo del framework incrustado en la base
para los locales `en`/`en-*`, luego la cadena de padres configurada,
luego los propios archivos del locale - y sirve *eso*, ya aplanado.
Pide `pt-PT.ftl` y la respuesta lleva tanto `welcome` como
`file-label`, en una sola solicitud, sin lógica de cadena del lado del
cliente. `?v=<hash>` sigue nombrando un único recurso inmutable; el
hash simplemente ahora también cubre los strings traídos de `pt-BR`.

**El aplanado solo cubre los padres configurados** - nunca llega más
allá de ellos hasta `fallback_locale`. El catálogo servido de `pt-PT`
incluye los strings de `pt-BR` porque `pt-BR` es un *padre
configurado*; no incluye los strings de `en` solo porque `en` resulte
ser el fallback global. El campo `fallback` de `LocaleShare` siempre
nombra el `fallback_locale` terminal, sin verse afectado por nada de
esto - le dice al frontend dónde acabaría eventualmente el recorrido a
nivel de fachada de `Lang`, no lo que ya está en el archivo que
acaba de obtener.

### Reglas de fusión de archivos delta

Un catálogo hijo se fusiona sobre su padre **a nivel del AST de
Fluent**, no por concatenación textual ni por sombreado de mensaje
completo. La unidad de override es el *pattern*, así que:

- **El valor de un hijo reemplaza el valor del padre**, en la posición
  del padre dentro del archivo.
- **Una entrada del hijo con atributos pero sin valor conserva el
  valor del padre.** Retraducir `.placeholder` no exige repetir el
  propio texto del mensaje.
- **Los atributos se fusionan por nombre.** Un atributo del hijo con
  el mismo nombre reemplaza al del padre, en su sitio; un atributo
  exclusivo del hijo se añade después del propio atributo del padre.
  **Los atributos que el hijo no menciona sobreviven desde el padre** -
  anular el valor de un mensaje nunca elimina en silencio su
  `.placeholder` o su `.aria-label`.
- **Las expresiones select se reemplazan enteras, nunca
  variante por variante.** Las variantes de un selector están
  indexadas a las categorías de plural CLDR de un locale; como esas
  categorías dependen del locale, empalmar una variante del padre y
  otra del hijo podría producir un selector sin la gramática de ningún
  locale único detrás. Un hijo que anula un selector debe suministrar
  todas las variantes que quiera.
- **Los comentarios sobre una entrada anulada siguen siendo los del
  padre.** El comentario documenta el id, y la unidad de override es
  el pattern, no el comentario.
- **Las entradas exclusivas del hijo se añaden al final**, en el
  propio orden del hijo, comentarios incluidos - un id que `pt-BR`
  nunca definió no es una "anulación" de nada.

Los términos (`-brand`) siguen la misma regla, con un estrechamiento:
el valor de un término nunca es opcional en la sintaxis de Fluent, así
que el caso "atributos-pero-sin-valor conserva el valor del padre" de
arriba solo aplica a los mensajes - un término hijo siempre
suministra un valor, y ese valor siempre gana. La fusión de atributos
por nombre, el reemplazo de pattern completo para el valor, y los
comentarios que gana el padre aplican a los términos exactamente igual
que a los mensajes. Los términos se rastrean en su propio namespace -
anular `-brand` nunca puede eclipsar un mensaje también llamado
`brand`.

### Por qué Suprnova diverge

Laravel 13 tiene exactamente un fallback: el valor de config global
único `fallback_locale`, consultado cuando al array del locale actual
le falta una clave. No existe el concepto de que un locale herede de
un locale hermano - `pt_PT.php` y `pt_BR.php` son dos arrays no
relacionados, y una app `pt_PT` o bien duplica todo lo que `pt_BR` ya
tiene traducido, o se publica sin ello.

Las cadenas de padres de Suprnova son la extensión del lado de Rust:
un paso intermedio entre "este locale" y "el fallback global",
configurado por locale en lugar de una sola vez de forma global. La
concesión que no quisimos hacer es empujar esa complejidad hacia el
navegador - un frontend consciente de las cadenas necesitaría pedir
`pt-PT.ftl`, descubrir que está incompleto, pedir `pt-BR.ftl` también,
y fusionarlos del lado del cliente en JavaScript, con reglas que
tendrían que coincidir exactamente con las del servidor. Aplanar en el
momento de la carga significa en cambio que el catálogo servido
siempre es un único archivo completo y autocontenido - el mismo
contrato que ya tenía el frontend antes de que existieran las cadenas
de padres, así que `@fluent/bundle` y los wrappers de los kits no
necesitaron ningún cambio para soportar esta feature.

## Detección de locale

`LocaleMiddleware` resuelve un locale por solicitud y lo vincula
durante todo el handler. La cadena está dirigida por config y **gana
el primer acierto**:

1. **Sesión** - la clave `locale` en la sesión, si
   [el middleware de sesión](session.md) se ejecutó y el valor nombra
   un locale disponible. Aquí es donde vive "el usuario eligió Español
   en los ajustes".
2. **Cookie** - la cookie `locale`. Sobrevive al logout, así que una
   elección de idioma hecha antes de iniciar sesión no se pierde.
3. **`Accept-Language`** - negociada contra `available_locales()` con
   `fluent-langneg`, respetando los valores q. `fr-CH, es;q=0.8, en;q=0.5`
   contra los catálogos `en` + `es` resuelve a `es`.
4. **`APP_LOCALE`** - el valor por defecto configurado, cuando nada de
   lo anterior acertó.

Un candidato que no se puede analizar, o que nombra un locale sin
catálogo, se **omite, no se rechaza**. Un usuario con una cookie
`locale=zz` obsoleta ve el idioma por defecto, no un 500. Un
encabezado `Accept-Language` con basura hace lo mismo. La entrada
controlada por un atacante llega a esta cadena en cada solicitud;
nunca debe poder hacer más que elegir un idioma.

Conéctalo en `bootstrap.rs`, **después** del middleware de sesión, ya
que el paso 1 lee la sesión:

```rust
use std::sync::Arc;
use suprnova::{
    global_middleware, App, LocaleMiddleware, LocaleShare, SessionConfig, SessionMiddleware,
};

pub async fn register() {
    global_middleware!(SessionMiddleware::install(SessionConfig::from_env()).await);

    // Resuelve el locale y lo vincula para la solicitud.
    global_middleware!(LocaleMiddleware::from_env().expect("locale config"));

    // Le entrega al frontend su locale + la URL del catálogo en cada página de Inertia.
    App::register_inertia_shared(Arc::new(LocaleShare));
}
```

`LocaleMiddleware::from_env()` lee `LocalizationConfig::from_env()`;
`LocaleMiddleware::new(config)` toma una que construyas tú. Una app
con andamiaje ya tiene ambas líneas.

### Cambiar el locale a mitad de solicitud

`Lang::set_locale` es el `App::setLocale` de Laravel - reescribe el
locale de la solicitud actual desde ese punto en adelante:

```rust
use suprnova::session::session_mut;
use suprnova::{FrameworkError, Lang, Locale};

/// El usuario acaba de cambiar de idioma en un formulario de ajustes.
pub fn switch_language(choice: &str) -> Result<(), FrameworkError> {
    let locale = Locale::parse(choice)?;
    Lang::set_locale(locale);                       // esta solicitud
    session_mut(|s| s.put("locale", choice));       // cada solicitud después
    Ok(())
}
```

Fíjate en las dos mitades: `set_locale` afecta a *esta* solicitud (así
que el mensaje flash de la redirección ya está en español), y la
escritura en sesión es lo que lee la cadena de detección en la
*siguiente*.

### Fuera de una solicitud

Los comandos de console, los workers de cola y las tareas programadas
no tienen solicitud ni middleware. Ahí, `Lang::set_locale` escribe una
anulación global del proceso que `Lang::locale()` consulta antes de
caer de vuelta a `APP_LOCALE`:

```rust
use suprnova::{command, FrameworkError, Lang, Locale, Mail};

use crate::mail::Digest;
use crate::models::user::User;

#[command(name = "mail:digest", description = "Send the weekly digest")]
pub async fn send_digest(_args: Vec<String>) -> Result<(), FrameworkError> {
    for user in User::query().get().await? {
        // La preferencia guardada de cada usuario, durante todo su correo.
        Lang::set_locale(Locale::parse(&user.locale)?);
        Mail::to(&user.email).send(Digest::for_user(&user)).await?;
    }
    Ok(())
}
```

Como esa anulación es de alcance global de proceso en lugar de
task-local, fíjala al principio de cada unidad de trabajo como arriba -
no confíes en que se mantenga sin cambios a través de un `.await` con
el que otra tarea podría entrelazarse.

## Configuración

Tres variables de entorno. `APP_LOCALE` y `APP_FALLBACK_LOCALE`
tienen ambas `en` por defecto; `APP_LOCALE_PARENTS` tiene vacío por
defecto - sin overrides por locale, solo aplica `fallback_locale`:

```env
APP_LOCALE=en
APP_FALLBACK_LOCALE=en
# APP_LOCALE_PARENTS=pt-PT=pt-BR
```

Todo lo demás es código, en `LocalizationConfig`. Se registra como
cualquier otra config tipada - en tu `config::register_all`, que se
ejecuta antes del arranque:

```rust
// src/config/mod.rs
use suprnova::{Config, Detect, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .default_locale(Locale::parse("es").expect("valid locale"))
        .use_isolating(true)                                // ver la nota de divergencia
        .detection(vec![Detect::Session, Detect::Header])   // ignora la cookie
        .session_key("preferred_locale")
        .cookie_name("lang")
        .parent(                                            // ver Cadenas de fallback
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

- `default_locale` / `fallback_locale` - anulan `APP_LOCALE` y
  `APP_FALLBACK_LOCALE` desde código. Un valor mal formado en
  cualquiera de los dos sitios hace fallar el arranque en lugar de
  convertirse en `en` en silencio.
- `use_isolating` - marcas de aislamiento Unicode alrededor de las
  interpolaciones. Desactivado por defecto; actívalo cuando publiques
  un locale RTL.
- `detection` - la cadena, en orden. Quitar `Detect::Cookie` significa
  que una elección de idioma solo vive en la sesión; quitar
  `Detect::Header` significa que la preferencia del navegador se
  ignora por completo.
- `session_key` / `cookie_name` - renombra las dos búsquedas.
- `parents` - padres de fallback por locale (`hijo -> padre`),
  recorridos antes de `fallback_locale` cuando falta una clave en el
  catálogo del hijo; misma forma que `APP_LOCALE_PARENTS`. Añade uno
  con `.parent(child, parent)` - encadenable, gana la última escritura
  para un hijo repetido. Ver
  [Cadenas de fallback](#cadenas-de-fallback) para el contrato
  completo (validación en tiempo de arranque, orden de resolución,
  aplanado del catálogo servido).

El arranque vincula un `Arc<dyn Translator>` en el contenedor. Si tu
app ya vinculó uno, el framework lo deja en paz - así es como
sustituyes un translator propio sin forkear nada:

```rust
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::{App, FluentTranslator, LocalizationConfig, Translator};

pub async fn register() {
    let config = LocalizationConfig::from_env().expect("locale config");
    let translator =
        FluentTranslator::from_dir("./catalogs", &config).expect("load catalogs");
    App::bind::<dyn Translator>(Arc::new(translator));
}
```

`Translator` es la costura de extensión: `translate`, `has`,
`available_locales`, `catalog`, `reload`. Se ofrece un driver
(`FluentTranslator`), y un backend nuevo es un driver nuevo - no un
fork de la superficie.

## Mensajes de validación traducidos

Cada regla integrada devuelve un mensaje **con clave**: una clave de
catálogo, los argumentos que necesita el mensaje, y un fallback en
inglés. La traducción sucede una vez, en el límite de serialización -
`ValidationErrors::to_json` y la bolsa de errores de Inertia - nunca
dentro de la regla. Las reglas se mantienen puras, y todo el subsistema
se compila fuera cuando no se necesita.

Las claves siguen una convención:

| Forma | Ejemplo | Se usa para |
|---|---|---|
| `validation-<rule>` | `validation-min`, `validation-required-if` | Una por regla integrada, en kebab-case |
| `field-<name>` | `field-email` | Un nombre humano para un campo |
| `validation-invalid-data` | - | El banner de nivel superior "The given data was invalid." |

Para traducirlos, define los ids que te importen en cualquier archivo
`.ftl` bajo el locale objetivo:

```ftl
# lang/es/validation.ftl
validation-invalid-data = Los datos proporcionados no son válidos.
validation-required = El campo { $field } es obligatorio.
validation-email = El campo { $field } debe ser una dirección de correo válida.
validation-min = El campo { $field } debe tener al menos { $min } caracteres.
validation-confirmed = La confirmación del campo { $field } no coincide.
```

`$field` siempre está disponible. Los propios parámetros de cada regla
se pasan bajo los nombres que llevan en el catálogo en inglés del
framework - `$min`, `$max`, `$other`, `$value` - y
`framework/src/localization/catalogs/en/validation.ftl` es la lista
canónica de ids y argumentos. Copia de ahí los ids que necesites; nunca
tienes que anular todos.

Anular funciona por locale y por clave. Definir `validation-min` en
`lang/en/validation.ftl` reemplaza la redacción en inglés del
framework para esa única regla y deja el resto intacto.

### Nombres de campo

Interpolar un nombre de columna en crudo produce "The email_address
field is required." La convención `field-<name>` arregla eso:

```ftl
# lang/en/validation.ftl
field-email_address = email address
field-dob = date of birth
```

Antes de renderizar, el translator busca `field-<name>` para el locale
actual. Un acierto se pasa como `$field`; un fallo cae de vuelta al
nombre de campo con los guiones bajos convertidos en espacios. Así que
el archivo de arriba solo hace falta para los nombres que se
humanizan mal.

### Reglas personalizadas

`Rule::passes` devuelve `Result<(), ValidationMessage>`. Un mensaje
con clave participa en la traducción:

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(ValidationMessage::keyed("validation-starts-with")
                .arg("prefix", self.0)
                .fallback(format!("must start with {}", self.0)))
        }
    }
}
```

```ftl
# lang/en/validation.ftl
validation-starts-with = The { $field } field must start with { $prefix }.
```

Un string plano sigue funcionando, y es la respuesta correcta para un
mensaje que solo existirá en un idioma:

```rust
Err("must start with acct_".into())   // sin clave: se renderiza literal
```

Los mensajes sin clave se saltan la traducción por completo, que es lo
que mantiene a las reglas personalizadas existentes compilando y
comportándose exactamente como antes.

### El flujo de derive

Los errores de `#[derive(Validate)]` también tienen clave. El código
de error del crate `validator` se convierte en `validation-<code>` con
los guiones bajos convertidos en guiones, y cada parámetro que adjunta
el validator se convierte en un argumento de mensaje - con dos
excepciones reservadas, `value` y `other`, que siempre se descartan.
Ambas llevan el *valor* real de un campo en lugar de metadatos sobre
la regla: `value` es el input bajo prueba tal como llegó, y `other`
(fijado por `must_match`, la regla canónica de confirmación de
contraseña) es el valor del campo hermano. Ninguna de las dos se le
entrega nunca al catálogo, así que ningún override de `.ftl` - sin
importar cómo redacte `validation-must-match` - puede interpolar un
secreto enviado en un cuerpo de respuesta 422. Así que un fallo de
`#[validate(email)]` resuelve `validation-email` igual que lo hace la
regla escrita a mano, y un locale que traduce una traduce ambas.

## El frontend

El navegador recibe los mismos bytes que resolvió el servidor. Nada se
retraduce, se reexporta, ni se mantiene sincronizado a mano.

### El endpoint del catálogo

```
GET /_suprnova/lang/es.ftl              → 200 text/plain, ETag: "<hash>"
GET /_suprnova/lang/es.ftl?v=<hash>     → 200 + Cache-Control: public,
                                          max-age=31536000, immutable
GET /_suprnova/lang/es.ftl              → 304 cuando If-None-Match coincide
GET /_suprnova/lang/zz.ftl              → 404 (no existe ese catálogo)
```

El cuerpo es el catálogo fusionado para ese locale - primero los
mensajes del framework, luego su cadena de padres de fallback
configurada si tiene alguna (ver
[Cadenas de fallback](#cadenas-de-fallback)), luego tus archivos en
orden de carga. `ETag` es el hash del contenido. Pide un hash
específico con `?v=` y la respuesta es cacheable de forma inmutable
para siempre, porque esa URL solo puede significar una cosa; pídelo
sin él y obtienes revalidación en su lugar. Como `/_suprnova/health`,
la ruta está exenta de la cadena de middleware: tiene que responder
antes de que se haya resuelto un locale, y no lleva datos de usuario.

### La prop compartida

`LocaleShare` es un `InertiaSharedData` que ofrece el framework.
Registrado en `bootstrap.rs` (ver
[Detección de locale](#detección-de-locale)), añade una prop a cada
página de Inertia:

```json
{
  "lang": {
    "locale": "es",
    "fallback": "en",
    "catalog": {
      "url": "/_suprnova/lang/es.ftl?v=9f2c1ae4",
      "hash": "9f2c1ae4"
    }
  }
}
```

`catalog` es `null` cuando no hay ningún translator vinculado - la
prop compartida nunca hace fallar el renderizado de una página.

### Los wrappers del kit

Cada starter kit ofrece un wrapper de ~100 líneas que lee esa prop,
busca el catálogo una vez, construye un bundle de `@fluent/bundle`, y
expone `t()`. Llama a `initLang` una vez en tu punto de entrada de
Inertia (las apps con andamiaje ya lo hacen):

```ts
// frontend/src/main.ts
import { createInertiaApp } from '@inertiajs/svelte'
import { mount } from 'svelte'
import { initLang } from './lib/lang.svelte'

createInertiaApp({
  resolve: (name) => { /* … sin cambios … */ },
  async setup({ el, App, props }) {
    await initLang(props.initialPage)
    mount(App, { target: el!, props })
  },
})
```

Luego, en los componentes:

```svelte
<!-- Svelte 5 -->
<script lang="ts">
  import { t, currentLocale } from '../lib/lang.svelte'
</script>

<h1>{t('welcome', { app: 'Suprnova' })}</h1>
<p>{currentLocale()}</p>
```

```tsx
// React 19
import { useLang } from '../lib/lang'

export default function Home() {
  const { t, locale } = useLang()
  return <h1>{t('welcome', { app: 'Suprnova' })}</h1>
}
```

```vue
<!-- Vue 3.5 -->
<script setup lang="ts">
import { useLang } from '../lib/lang'
const { t, locale } = useLang()
</script>

<template>
  <h1>{{ t('welcome', { app: 'Suprnova' }) }}</h1>
</template>
```

El formateo de números y fechas del lado del cliente usa el `Intl`
integrado del navegador - no se envían datos de ICU al navegador.

### Claves de mensaje tipadas

`suprnova generate-types` parsea `lang/<locale por defecto>/*.ftl` y
emite una unión de cada id de mensaje junto a los tipos de props de
página:

```ts
// frontend/src/types/lang-keys.ts
// Generated by `suprnova generate-types` - do not edit.
export type MessageKey =
  | "validation-min"
  | "welcome"
```

Los wrappers tipan `t(key: MessageKey, …)`, así que esta es la misma
promesa que [`inertia-props.ts`](frontend-typescript-types.md):
renombra un mensaje en Rust, regenera, y el compilador de TypeScript
señala cada sitio de llamada que todavía usa el id viejo. `suprnova
serve` vigila `lang/` junto con `src/`, así que el archivo se
regenera mientras editas catálogos.

Un proyecto sin directorio `lang/` y sin ids de mensaje no obtiene
**ningún archivo** - una app que no está localizada no ve aparecer
ningún artefacto nuevo.

## Formateo consciente del locale

Siete funciones en `Lang`, todas respaldadas por ICU4X, todas leyendo
el locale actual, todas con contrapartes `try_*` que devuelven
`Result<String, FrameworkError>` en lugar de degradarse:

```rust
use suprnova::chrono::NaiveDate;
use suprnova::{DateStyle, Lang, ListStyle, RelativeUnit, TimeStyle};

let dt = NaiveDate::from_ymd_opt(2026, 8, 1)
    .and_then(|d| d.and_hms_opt(14, 30, 0))
    .expect("valid datetime");

Lang::number(1_234_567.89);                          // en-US → 1,234,567.89
                                                     // de-DE → 1.234.567,89
Lang::currency(19.99, "USD");                        // en-US → $19.99
Lang::date(&dt, DateStyle::Long);                    // en-US → August 1, 2026
Lang::time(&dt, TimeStyle::Short);                   // en-US → 2:30 PM
Lang::datetime(&dt, DateStyle::Medium, TimeStyle::Short);
Lang::list(&["Ada", "Grace", "Alan"], ListStyle::And); // → Ada, Grace, and Alan
Lang::relative(-3, RelativeUnit::Day);               // → 3 days ago
```

Los enums de estilo: `DateStyle { Full, Long, Medium, Short }`,
`TimeStyle { Medium, Short }`, `ListStyle { And, Or, Unit }`,
`RelativeUnit { Second, Minute, Hour, Day, Week, Month, Year }`.
`Lang::relative` toma una cantidad con signo - negativo es el pasado
("3 days ago"), positivo el futuro ("in 3 days").

> La salida exacta viene de los datos CLDR incrustados en ICU4X y
> puede cambiar entre una actualización de ICU, particularmente para
> fechas y moneda. En tus propios tests, verifica la forma y la
> distinción entre locales (`de != en`, contiene `2026`) en lugar de
> los bytes exactos.

### Formateo dentro de un mensaje

Dos funciones son invocables desde FTL:

```ftl
order-total = Your total is { NUMBER($amount, maximumFractionDigits: 2) }.
published = Published { DATETIME($when, dateStyle: "medium", timeStyle: "short") }
```

```rust
use suprnova::__;

let line = __!("published", when: "2026-08-01T14:30:00");
```

`NUMBER()` es la integrada de Fluent, registrada explícitamente, y te
da control de dígitos de fracción dentro del mensaje. `DATETIME()` es
de Suprnova: `$value` acepta un string ISO-8601 o milisegundos época,
y `dateStyle` / `timeStyle` toman los mismos nombres que los enums de
Rust, en minúscula. Un valor que no puede parsear pasa sin cambios con
un `warn!` - una función de Fluent no puede devolver un error, y una
página renderizada con una fecha de aspecto raro es mejor que un 500.

Cuando quieras el formateo completo de ICU4X en lugar de lo que expone
una función de Fluent, formatea en Rust y pasa el string ya terminado:

```rust
use suprnova::{__, Lang};

let total = __!("order-total-text", amount: Lang::currency(19.99, "USD"));
```

## Probar tus traducciones

Dos ayudantes hacen el trabajo: `use_lang_path` apunta el loader a un
directorio de fixtures, y `scope_locale` fija el locale actual durante
la vida de un future.

La forma hermética - construir un translator sobre un directorio de
fixtures y vincularlo en un contenedor con scope de test - es lo que
usan los propios tests del framework, porque no toca ningún estado
global del proceso y sobrevive a la ejecución paralela de tests:

```rust
use std::sync::Arc;
use suprnova::testing::TestContainer;
use suprnova::{scope_locale, FluentTranslator, Lang, Locale, LocalizationConfig, Translator};

#[tokio::test]
async fn spanish_greeting_comes_from_the_catalog() {
    let _guard = TestContainer::fake();

    let config = LocalizationConfig::from_env().expect("locale config");
    let translator = FluentTranslator::from_dir("tests/fixtures/lang", &config)
        .expect("load catalogs");
    TestContainer::bind::<dyn Translator>(Arc::new(translator));

    scope_locale(Locale::parse("es").expect("locale"), async {
        assert_eq!(Lang::get("welcome"), "¡Bienvenido!");
        assert_eq!(Lang::locale().as_str(), "es");
    })
    .await;
}
```

`use_lang_path` es la herramienta correcta cuando el test arranca la
aplicación real y quieres que la app *entera* apunte a fixtures:

```rust
use suprnova::use_lang_path;

#[tokio::test]
async fn app_boots_against_fixture_catalogs() {
    use_lang_path("tests/fixtures/lang");
    // … arranca la app; `lang_path("")` ahora resuelve al directorio de fixtures.
}
```

Escribe una anulación global de ruta del proceso, así que trátalo como
un ajuste por binario en lugar de algo en lo que dos tests paralelos
puedan estar en desacuerdo.

La detección en sí - la cadena de sesión/cookie/`Accept-Language` -
vale la pena probarla a través del pipeline real en lugar de llamando
al middleware directamente, porque los casos interesantes tienen que
ver con el parseo de encabezados y con qué fuente gana. Monta una ruta
cuyo handler devuelva `__!("welcome")`, registra `LocaleMiddleware` en
el `MiddlewareRegistry`, y condúcelo con el harness de loopback de
[Pruebas HTTP](http-tests.md), enviando `Accept-Language: fr, es;q=0.8`
y verificando el cuerpo en español. Los casos que vale la pena fijar:
un encabezado negocia, una cookie le gana a un encabezado, un locale
no disponible se omite en lugar de fallar, y un encabezado mal formado
igual devuelve 200.

Ver [Pruebas](testing.md) para `TestContainer::scope` cuando tu test
se ejecuta sobre un runtime multihilo - la guarda `fake()` thread-local
de arriba no sobrevive a un future que migra entre workers.

### Por qué Suprnova diverge

**Archivos FTL, no arrays de PHP.** Laravel tiene dos formatos - arrays
anidados en `lang/en/messages.php`, más JSON plano en `lang/en.json`
para traducciones indexadas por string - y ninguno de los dos es
cargable por un navegador, ni expresa la selección de plural dentro
del archivo: eso vive en la convención de pipes y rangos de
`trans_choice` dentro del string. Fluent nos da un único formato que
el servidor y el cliente parsean ambos, que es lo que hace de "el
frontend muestra el mismo string que produjo el validador" una
propiedad del diseño en lugar de una convención que mantienes. Te
cuesta una sintaxis nueva que aprender (este capítulo es la mayor
parte) y un cambio de herramientas: Poedit no puede editar `.ftl`,
mientras que Crowdin, Weblate, Lokalise y Pontoon sí pueden. También
te cuesta el namespacing con puntos - `trans('messages.welcome')` no
tiene equivalente, porque los ids son un namespace plano por locale.
Usa un prefijo en su lugar.

**Sin `trans_choice`.** Laravel selecciona una forma de plural con
strings separados por pipes y rangos explícitos:

```php
// Laravel
trans_choice('{1} plik|[2,4] pliki|[5,*] plików', $count);
```

Ahora cuenta hasta 22 en polaco. CLDR pone el 22 en la categoría
`few` - `22 pliki` - pero `[5,*]` se lo traga y produce `22 plików`.
La misma rotura pasa en 32, 42, 102, y en ruso, árabe, checo,
lituano, y galés, cada uno en sus propios puntos. Los rangos de
enteros no pueden expresar reglas de plural, porque las reglas de
plural no son sobre rangos; son sobre el último dígito, los dos
últimos dígitos, y en algunos idiomas si el valor es siquiera un
entero. Fluent selecciona directamente sobre la categoría CLDR, así
que `$count` es un argumento ordinario y el *traductor* - la persona
que conoce el idioma - escribe las cuatro categorías del polaco:

```ftl
files =
    { $count ->
        [one] { $count } plik
        [few] { $count } pliki
        [many] { $count } plików
       *[other] { $count } pliku
    }
```

`one` es 1; `few` es 2-4, 22-24, 32-34, 102-104; `many` es 0, 5-21,
25-31; `other` atrapa las fracciones (`1,5 pliku`) y lleva la marca
por defecto, según la regla de arriba.

La forma sin rangos de Laravel (`plik|pliki|plików`) lo hace mejor -
consulta un índice por idioma y elige el segmento *n*-ésimo - pero ese
índice es una tabla mantenida a mano en lugar de datos CLDR, le ofrece
al polaco tres segmentos donde CLDR define cuatro categorías, los
segmentos son posicionales sin nombres de categoría que revisar, y
solo puede seleccionar sobre el count.

Que es el segundo beneficio, y sale gratis: un selector de Fluent
puede seleccionar sobre *cualquier* argumento, no solo un count.
Género, nivel de plan, y estado de conexión seleccionan de la misma
forma, y ninguno necesitó un método de fachada nuevo.

**Las marcas de aislamiento están desactivadas por defecto.** Fluent
normalmente envuelve cada interpolación en U+2068 (FIRST STRONG
ISOLATE) y U+2069 (POP DIRECTIONAL ISOLATE), para que un valor de
derecha a izquierda incrustado en una frase de izquierda a derecha se
renderice en el orden correcto. Correcto - e invisible, lo que
significa que cada `assert_eq!("Hello Ada", …)` en una app solo en
inglés falla con dos caracteres que nadie puede ver en el diff. Las
desactivamos por defecto y hacemos que activarlas sea una sola
llamada:

```rust
let config = LocalizationConfig::from_env()?.use_isolating(true);
```

**Actívalas cuando publiques un locale RTL** - árabe, hebreo, persa,
urdu - o cualquier locale donde valores suministrados por el usuario
mezclen escrituras dentro de una frase. Luego actualiza tus
aserciones para comparar contra strings que lleven las marcas, o
elimínalas en el ayudante de aserción. El valor por defecto optimiza
para el caso común; el caso correcto está a una línea de distancia y
este párrafo es el recordatorio de tomarla.

## Siguiente

- [Validación](validation.md) - reglas, la macro `validate!`, y de
  dónde viene `ValidationMessage`
- [Tipos de TypeScript](frontend-typescript-types.md) -
  `generate-types`, `inertia-props.ts`, y `lang-keys.ts`
- [Middleware](middleware.md) - ordenar `LocaleMiddleware` respecto al
  resto de la cadena global
- [Sesiones](session.md) - el store que lee el primer paso de
  detección
- [Variables de entorno](env-vars.md) - `APP_LOCALE`,
  `APP_FALLBACK_LOCALE`, `APP_LOCALE_PARENTS`, `APP_BASE_PATH`
- [Pruebas](testing.md) - `TestContainer`, `#[suprnova_test]`, y
  overrides herméticos de DI

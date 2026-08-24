# Respuestas

Todo handler de Suprnova devuelve un `Response`, que es un alias de
`Result<HttpResponse, HttpResponse>`. La rama `Ok` lleva la respuesta de
éxito, la rama `Err` lleva una respuesta de error ya renderizada, y el
operador `?` colapsa por el camino cualquier tipo de error que tenga un
`From` hacia `HttpResponse`. Este capítulo es la referencia práctica para
construir el lado `Ok` - los constructores de `HttpResponse`, el builder
`Redirect`, la API de cookies y los cortocircuitos `abort_*`. Para el
panorama de los errores consulta [Modelo de errores](error-model.md) y
[Manejo de errores](errors.md).

## Constructores de `HttpResponse`

`HttpResponse` es el tipo de respuesta con la forma que viaja por la red.
Los constructores establecen valores por defecto sensatos; los setters
encadenables los sobrescriben.

### Constructores de cuerpo

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn examples() -> Response {
    // text/plain
    let _ = HttpResponse::text("OK");

    // application/json (cualquier serde_json::Value)
    let _ = HttpResponse::json(json!({ "ok": true }));

    // text/html; charset=utf-8
    let _ = HttpResponse::html("<h1>Hello</h1>");

    // Bytes en crudo con un content type explícito - lo usan la
    // serialización JSON:API y cualquier otro cuerpo de bytes no JSON.
    let _ = HttpResponse::bytes_body(b"PNG...".to_vec(), "image/png");

    Ok(HttpResponse::text("done"))
}
```

Existen cuatro constructores de streaming para respuestas de larga vida:

- `HttpResponse::sse(stream)` - Server-Sent Events. Envuelve un `Stream`
  de valores `SseEvent`, establece los cuatro encabezados requeridos
  (`Content-Type: text/event-stream`, `Cache-Control: no-cache`,
  `Connection: keep-alive`, `X-Accel-Buffering: no`) y mantiene la
  conexión abierta hasta que el stream productor termina. Consulta
  [Eventos enviados por el servidor](sse.md).
- `HttpResponse::stream_bytes(stream)` - respuesta chunked genérica. Toma
  un `Stream<Item = Result<Bytes, Infallible>>`. El tipo de error es
  `Infallible` a propósito: todos los productores del framework
  convierten sus propios errores en un mensaje terminal del stream antes
  de que el stream acabe, porque no hay forma de comunicarle al cliente
  un error de nivel de transporte a mitad de la respuesta.
- `HttpResponse::event_stream(stream, end)` - equivalente a
  `ResponseFactory::eventStream` de Laravel. Envuelve un `Stream` de
  valores `sse::StreamedEvent`, enmarca cada uno como `event: update` (o
  su nombre) y permite configurar un frame terminal. Consulta [SSE](sse.md).
- `HttpResponse::stream_json(stream)` - equivalente a
  `ResponseFactory::streamJson` de Laravel. Envuelve cualquier valor
  `Serialize` y lo vacía como un único array JSON construido
  incrementalmente, en lugar de almacenar primero toda la colección.
  Consulta [SSE](sse.md#event-stream-and-stream-json).

### Estado, encabezados, cookies

Cada builder devuelve `Self`, así que encadena libremente:

```rust
use suprnova::{Cookie, HttpResponse, Response};
use serde_json::json;

pub async fn created() -> Response {
    Ok(HttpResponse::json(json!({ "id": 42 }))
        .status(201)
        .header("X-Resource-Id", "42")
        .cookie(Cookie::new("last_id", "42")))
}
```

| Método | Comportamiento |
|---|---|
| `.status(code)` | Establece el estado HTTP. Los códigos fuera de `100..=599` se degradan a 500 en el límite de la red, con una advertencia en el registro. |
| `.header(name, value)` | Añade un encabezado. Se permiten duplicados (coincide con la semántica de `Set-Cookie`). |
| `.replace_header(name, value)` | Descarta cualquier aparición previa y establece una. |
| `.with_headers([(k, v), ...])` | Añade muchos de una vez. Acepta cualquier `IntoIterator<Item = (K, V)>`. |
| `.without_header(name)` | Elimina todas las apariciones (sin distinguir mayúsculas de minúsculas). |
| `.header_value(name)` | Vuelve a leer el primer valor establecido. Útil en tests. |
| `.cookie(Cookie)` | Adjunta una cookie como `Set-Cookie`. |
| `.with_cookies([Cookie, ...])` | Adjunta varias. |
| `.without_cookie(name)` | Programa un borrado (equivalente a `Cookie::forget(name)`). |

Los mismos setters encadenables están disponibles sobre un `Response`
(el `Result`) a través del trait `ResponseExt`, de modo que las macros
siguen siendo ergonómicas:

```rust
use suprnova::{json_response, Cookie, Response, ResponseExt};

pub async fn list() -> Response {
    json_response!({ "ok": true })
        .status(200)
        .header("X-Total-Count", "42")
        .cookie(Cookie::new("last_query", "list"))
}
```

`ResponseExt` expone `.status`, `.header`, `.with_headers`,
`.without_header`, `.cookie`, `.with_cookies` y `.without_cookie`.

### Validación en el límite de la red

`HttpResponse::into_hyper` ejecuta dos filtros de seguridad antes de
entregarle la respuesta a hyper:

- **Rango de estado.** Cualquier cosa fuera de `100..=599` se degrada a
  500 con un `tracing::warn!`. Esto atrapa en el límite las erratas del
  tipo `AppError::status(700)` en lugar de dejar que salgan por la red
  códigos que no cumplen la norma.
- **Inyección de CRLF en los encabezados.** Cada nombre y cada valor de
  encabezado se validan con los propios `HeaderName::try_from` /
  `HeaderValue::try_from` de hyper. Todo encabezado rechazado se descarta
  con una advertencia en el registro y la respuesta se construye sin él.
  Los valores controlados por un atacante que acaban reflejados en un
  encabezado (allow-headers de CORS, `X-Forwarded-*`, encabezados de
  depuración propios) no pueden partir la respuesta.

Ambos filtros son silenciosos en el camino de éxito - solo los ves en los
registros cuando algo intentó colarse.

## Macros de respuesta

Existen dos macros con forma de `Response` para los casos comunes:

```rust
use suprnova::{json_response, text_response, Response};

pub async fn json_handler() -> Response {
    json_response!({ "users": [{ "id": 1, "name": "Alice" }] })
}

pub async fn text_handler() -> Response {
    text_response!("OK")
}
```

Ambas se expanden a `Ok(HttpResponse::...)`. Encadena setters de
`ResponseExt` sobre cualquiera de las dos para ajustar el estado, los
encabezados o las cookies.

## Cookies

`Cookie::new(name, value)` produce una cookie con valores por defecto
seguros - `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`. Anúlalos cookie
por cookie:

```rust
use suprnova::Cookie;
use std::time::Duration;

let session = Cookie::new("session_id", "abc123")
    .http_only(true)
    .secure(true)
    .same_site(suprnova::SameSite::Strict)
    .path("/")
    .domain("example.com")
    .max_age(Duration::from_secs(3600))
    .partitioned(true);
```

Cuatro constructores de conveniencia cubren patrones comunes:

- `Cookie::forget(name)` - valor vacío, `Max-Age=0`, ruta `/`, sin
  dominio. Úsalo al cerrar sesión para indicarle al navegador que descarte
  la cookie.
- `Cookie::forget_with(name, path, domain)` - la forma con alcance. El
  navegador solo descarta una cookie cuando el `Path` y el `Domain` de la
  cookie de borrado coinciden con aquellos con los que se estableció, de
  modo que una cookie establecida con `Path=/admin` o
  `Domain=.example.com` sobrevive a un `forget` simple. Pasa `None` para
  cualquiera de los argumentos para conservar el predeterminado.
- `Cookie::forever(name, value)` - `Max-Age` de cinco años.
- `Cookie::encrypted(name, plaintext)` - texto cifrado AES-256-GCM cuyo AAD
  está ligado al nombre lógico de la cookie. Léelo con
  `Cookie::read_encrypted_for(name, wire)` usando el mismo nombre.
  `Cookie::read_encrypted(wire)` es el lector v1 obsoleto y sin contexto; no
  puede descifrar el resultado actual de `Cookie::encrypted` y está programada
  su eliminación en 1.4.0 junto con el fallback v1. Requiere que `APP_KEY`
  esté establecida en el arranque. Consulta [Cifrado](encryption.md).

Eliminar varias cookies a la vez  - la forma usual de cierre de sesión -  usa
`without_cookies`, disponible en `HttpResponse`, en `Response` mediante
`ResponseExt` y en ambos builders de redirección:

```rust
use suprnova::{HttpResponse, Redirect};

let _ = HttpResponse::text("bye").without_cookies(["session", "remember"]);
let _: suprnova::Response = Redirect::to("/login")
    .without_cookies(["session", "remember"])
    .into();
```

En una redirección los borrados viajan en el propio 302, no en el destino, por
lo que el navegador ya los descartó cuando sigue el `Location`.

La serialización del encabezado aplica percent-encoding a todo byte que
no sea un cookie-octet válido según la RFC 6265, incluidos todos los
caracteres de control. Un CRLF en el nombre o el valor de una cookie se
codifica, no se propaga - la inyección de encabezados a través de cookies
queda cerrada en el serializador.

### Encolar una cookie para después

A veces el código que no está construyendo la respuesta aún necesita establecer
una cookie: un oyente que reacciona a un evento, una pieza de middleware que
se ejecuta antes del handler, un servicio `App::bind` sin ningún
`HttpResponse` en alcance. `Cookie::queue` es el `Cookie::queue()` de Laravel:
guarda la cookie en un jar por solicitud que `SessionMiddleware` vacía en la
respuesta saliente, justo después de la cookie de sesión.

```rust
use suprnova::Cookie;

Cookie::queue(Cookie::new("theme", "dark"));

// Look up what's queued.
let queued = Cookie::queued("theme");

// Remove it before the response goes out.
Cookie::unqueue("theme");

// Queue a deletion instead of a value - composes with `forget_with`.
Cookie::expire("theme", Some("/app"), None);
```

El jar es local a la tarea y comienza vacío para cada solicitud: nada de lo
encolado en una solicitud es visible en la siguiente, y un valor encolado que
nunca se vacía (sin `SessionMiddleware` en la cadena de la ruta) se descarta
en vez de provocar un panic. Las cookies encoladas se adjuntan a lo que
devuelva el handler, incluso a una redirección: un handler que encola una
cookie y luego devuelve `Redirect::to(...)` sigue llevando el encabezado
`Set-Cookie` en la respuesta 3xx. También se adjuntan a un 500 que el propio
`SessionMiddleware` construye para un fallo interno a mitad de la solicitud:
una sesión existente que no se puede leer, una escritura de sesión que falla o
el fallo de cifrado de la cookie de sesión, porque una cookie encolada ya puede
representar un efecto secundario confirmado en otro lugar (por ejemplo, una
fila de token remember-me ya escrita), por lo que la respuesta que informa del
fallo también la lleva. **No** sobreviven a un panic: el código de vaciado de
`SessionMiddleware` se ejecuta después de que el handler retorna normalmente,
y un panic capturado se convierte en un 500 fuera de toda la cadena de
middleware, el mismo punto en el que las propias cookies encoladas de Laravel
se pierden ante una excepción no capturada.

### Por qué Suprnova diverge

El `CookieJar` de Laravel indexa la cola por nombre *y* ruta, por lo que se
pueden encolar independientemente dos cookies con el mismo nombre en rutas
distintas. Suprnova indexa el jar solo por nombre: encolar una segunda cookie
bajo un nombre ya encolado sustituye la primera en vez de añadir una segunda
línea `Set-Cookie`. Eso cubre el caso común  - un sitio de llamada es
propietario de un nombre de cookie dado -  sin la búsqueda adicional indexada por
ruta que necesita la versión de Laravel.

## Redirecciones

`Redirect` cubre la superficie completa del redirector de Laravel. Todas
las variantes implementan `From<Redirect> for Response`, así que la forma
idiomática es `Redirect::...().into()`.

### Destinos

```rust
use suprnova::{Redirect, redirect_to};

// URL o ruta explícita
let _ = Redirect::to("/dashboard");

// Lo mismo, con una función libre algo más corta
let _ = redirect_to("/dashboard");

// Ruta con nombre (devuelve RedirectRouteBuilder)
let _ = Redirect::route("users.show").with("id", "42");

// URL externa explícita - igual que `to`, pero el nombre señala
// "esto sale del sitio" para las auditorías de open redirect
let _ = Redirect::away("https://external.example.com");

// Refresca la página (lee la URL anterior de la sesión; recurre
// a "/" si no hay ningún alcance de sesión activo)
let _ = Redirect::refresh();

// Lo mismo, pero tomando un Request explícito cuando no hay alcance activo
// let _ = Redirect::refresh_for(&request);

// El previous_url de la sesión, con fallback cuando no hay sesión en alcance
let _ = Redirect::back("/login");

// La URL prevista guardada en la sesión, que se consume al leerla, con fallback
let _ = Redirect::intended("/home");

// Redirección de invitado: guarda la URL de la solicitud actual como
// "intended" y manda al usuario a una página de login
// let _ = Redirect::guest(&request, "/login");
```

`Redirect::back`, `Redirect::intended`, `Redirect::guest` y
`Redirect::refresh` se integran todos con la sesión. Sin un alcance de
sesión caen en silencio hasta sus valores por defecto - práctico para
montajes de test parciales. Consulta [Sesiones](session.md).

### Validación de rutas nombradas

La proc-macro `redirect!` valida el nombre de la ruta en tiempo de
compilación y se expande a `Redirect::route(name)`:

```rust
use suprnova::{redirect, Response};

pub async fn store() -> Response {
    // La compilación falla si "users.index" no es un nombre de ruta registrado;
    // el mensaje de error lista las rutas disponibles y sugiere las más parecidas.
    redirect!("users.index").into()
}
```

### Códigos de estado

```rust
use suprnova::Redirect;

let _ = Redirect::to("/x").permanent();      // 301
let _ = Redirect::to("/x").status(303);      // 303, 307, 308, ...
```

El valor por defecto es 302.

### Datos flash

Los builders de `Redirect` llevan su propia flash bag. Al convertirse en
un `Response`, la flash bag se vuelca en la sesión activa y sobrevive
exactamente una solicitud más:

```rust
use suprnova::Redirect;

let _ = Redirect::back("/users/new")
    .with("status", "User created")            // una sola clave/valor
    .with_input([                              // repuebla el formulario
        ("email", "shawn@example.com"),
        ("name", "Shawn"),
    ])
    .with_errors([                             // bolsa de errores por defecto
        ("email", "Must be unique"),
    ])
    .with_errors_bag("login", [                // bolsa de errores nombrada
        ("password", "Required"),
    ]);
```

La página que los recibe los vuelve a leer con `session.get(...)` (para
`with`), `session.get_old_input(...)` (para `with_input`) y el mapa de
bolsas que vuelca `session.pull_errors_flash()` (para `with_errors` /
`with_errors_bag`). La capa de Inertia consume el flash de errores
automáticamente - la prop `errors` de toda respuesta de Inertia se
siembra desde la sesión, así que `Redirect::back().with_errors(...)` hace
aparecer los mensajes en el destino sin cableado adicional. El encabezado
de solicitud `X-Inertia-Error-Bag` acota la prop bajo una bolsa nombrada
para las páginas con varios formularios.

Ten en cuenta que sobre `RedirectRouteBuilder` (lo que devuelven
`Redirect::route` y `redirect!`), `.with(key, value)` establece un
**parámetro de ruta**, no una entrada flash - ahí usa
`.flash(key, value)`:

```rust
use suprnova::redirect;

let _ = redirect!("users.show")
    .with("id", "42")                          // parámetro de ruta
    .flash("status", "Updated");               // flash de sesión
```

El destino de `Redirect::back`  - la URL anterior registrada por la sesión -  no
recibe nunca confianza literal. El middleware de sesión solo registra desde
el principio una URL relativa a la raíz y del mismo origen (una ruta que
empieza por `//` o `/\`, o que lleva un byte de control ASCII en cualquier
parte, nunca se almacena), y la misma comprobación se repite en cada lectura,
por lo que `back` tampoco puede dirigirse fuera del origen mediante una
solicitud que llegue a tu app con una ruta inusual ni mediante una cookie de
sesión escrita antes de que existiera esta protección. Consulta
[Sesiones](session.md#other-operations) para la regla completa.

### Cookies, encabezados, fragmentos

```rust
use suprnova::{Cookie, Redirect};

let _ = Redirect::route("billing.show")
    .with_cookies([Cookie::new("welcome", "yes")])
    .with_headers([("X-Trace", "abc")])
    .with_fragment("invoices")                 // añade #invoices
    .without_fragment();                       // O quita cualquier fragmento previo
```

`with_fragment` acepta el fragmento con o sin `#` inicial. Llamar a
`with_fragment` después de `without_fragment` vuelve a adjuntar uno.

### Preservar el fragmento a través de la redirección

Para las apps de Inertia donde el destino debe conservar el hash de la
URL *de origen*, usa `preserve_fragment`:

```rust
use suprnova::Redirect;

let _ = Redirect::route("dashboard.index").preserve_fragment();
```

Al convertirse, esto escribe como flash
`_inertia.preserve_fragment = true` en la sesión; la siguiente respuesta
de Inertia lee el flag y emite `preserveFragment: true` en su objeto de
página. Sin alcance de sesión - el flag se descarta en silencio.

### Redirecciones firmadas

Dos builders envuelven la superficie de firma de URLs para redirecciones
de un solo uso hacia rutas con nombre (restablecimiento de contraseña,
verificación de correo, enlaces de descarga):

```rust
use suprnova::Redirect;

let r = Redirect::signed_route("downloads.show", &[("id", "42")])?;
let r = Redirect::temporary_signed_route(
    "downloads.show",
    &[("id", "42")],
    1_700_000_000, // expires_at_epoch_seconds
)?;
```

Ambos devuelven `Result<Redirect, FrameworkError>` - propaga el error con
`?`, ya que `Redirect` se convierte limpiamente en un `Response`.
Consulta [Generación de URLs](urls.md) para la superficie de firma.

### Guardar la URL prevista

`Redirect::set_intended_url` escribe el destino previsto de la sesión sin
llevar a cabo una redirección - normalmente se llama desde el middleware
de autenticación antes de redirigir a `/login`, para que un
`Redirect::intended` posterior pueda recuperar la URL originalmente
solicitada:

```rust
suprnova::Redirect::set_intended_url("/admin/users");
```

## Abortar desde un handler

Tres funciones libres cortocircuitan un handler en un estado dado.
Devuelven `Result<(), FrameworkError>`; combínalas con `?`:

```rust
use suprnova::{abort_if, abort_unless, abort_with, json_response, Request, Response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    json_response!({ "ok": true })
}
```

El error subyacente es `FrameworkError::Domain { message, status_code }`,
así que se renderiza con el mismo envoltorio JSON y las mismas reglas de
sanitización de los 5xx que cualquier otra ruta de error. El renderizador
de respuestas coerciona a 500 los códigos de estado fuera de rango.
Consulta [Modelo de errores](error-model.md) para el contrato de
conversión completo.

## Devolver errores directamente

Como `Response` es `Result<HttpResponse, HttpResponse>`, puedes devolver
directamente una rama `Err` - útil cuando la forma de la respuesta ya es
un cuerpo JSON concreto y lo quieres tal cual por la red:

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn legacy_lookup() -> Response {
    Err(HttpResponse::json(json!({
        "error": "deprecated endpoint",
    })).status(410))
}
```

Para cualquier cosa más rica - errores de dominio tipados, validación,
observabilidad - usa la superficie de
[Modelo de errores](error-model.md) (`AppError`, `FrameworkError`,
`#[domain_error]`).

## Referencia rápida

| Necesidad | Usa |
|---|---|
| Respuesta JSON | `HttpResponse::json(v)` o `json_response!({...})` |
| Respuesta de texto | `HttpResponse::text(s)` o `text_response!(s)` |
| Respuesta HTML | `HttpResponse::html(s)` |
| Bytes en crudo + content-type | `HttpResponse::bytes_body(b, "image/png")` |
| Server-Sent Events | `HttpResponse::sse(stream)` - consulta [SSE](sse.md) |
| Stream chunked | `HttpResponse::stream_bytes(stream)` |
| Establecer el estado | `.status(code)` |
| Añadir un encabezado | `.header(k, v)` / `.with_headers([...])` |
| Eliminar un encabezado | `.without_header(name)` |
| Adjuntar una cookie | `.cookie(c)` / `.with_cookies([...])` |
| Olvidar una cookie | `.without_cookie(name)` / `.without_cookies([...])` |
| Olvidar cookie con ruta/dominio | `Cookie::forget_with(name, Some("/admin"), Some("example.com"))` |
| Encolar una cookie para la próxima respuesta | `Cookie::queue(c)` |
| Consultar cookie encolada | `Cookie::queued(name)` |
| Quitar cookie de la cola | `Cookie::unqueue(name)` |
| Encolar una cookie de borrado | `Cookie::expire(name, path, domain)` |
| Redirección simple | `Redirect::to(path).into()` o `redirect_to(path).into()` |
| Redirección a una ruta con nombre | `redirect!("name").into()` o `Redirect::route("name")` |
| Redirección hacia atrás | `Redirect::back(fallback)` |
| Redirección a la URL prevista | `Redirect::intended(default)` |
| Redirección de invitado (guarda la URL prevista) | `Redirect::guest(&req, login)` |
| Fijar el destino previsto | `Redirect::set_intended_url(url)` |
| URL externa | `Redirect::away(url)` |
| Refrescar la página actual | `Redirect::refresh()` / `Redirect::refresh_for(&req)` |
| Redirección a una ruta firmada | `Redirect::signed_route(name, &[(k, v)])?` |
| Parámetro de ruta en la redirección | `.with("key", "value")` |
| Parámetro de query en la redirección | `.query("key", "value")` |
| Datos flash | `.with(key, value)` (o `.flash` en `RedirectRouteBuilder`) |
| Entrada de formulario en flash | `.with_input([(k, v), ...])` |
| Errores en flash | `.with_errors([(k, msg), ...])` |
| Bolsa de errores nombrada | `.with_errors_bag(bag, [(k, msg)])` |
| Añadir un fragmento | `.with_fragment("section")` |
| Quitar el fragmento | `.without_fragment()` |
| Preservar el fragmento (Inertia) | `.preserve_fragment()` |
| Redirección permanente | `.permanent()` (301) |
| Estado de redirección personalizado | `.status(303)` |
| Abortar de forma temprana | `abort_with(code, msg)?`, `abort_if(cond, code, msg)?`, `abort_unless(cond, code, msg)?` |

## Siguiente

- [Modelo de errores](error-model.md) - `FrameworkError`, `AppError`,
  `HttpError` y la única conversión que renderiza todo error a un
  `HttpResponse`
- [Manejo de errores](errors.md) - patrones prácticos de handler para
  `?`, `AppError` y errores de dominio propios
- [Eventos enviados por el servidor](sse.md) - construir y consumir
  respuestas `sse(...)`
- [Generación de URLs](urls.md) - URLs firmadas, resolución de rutas con
  nombre, la superficie que hay detrás de `Redirect::signed_route`
- [Sesiones](session.md) - datos flash, URLs previstas, la bolsa en la
  que escriben `Redirect::with`/`with_input`/`with_errors`

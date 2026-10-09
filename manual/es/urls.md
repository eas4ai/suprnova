# Generación de URLs

Las URLs son la forma en que tu aplicación se refiere a sí misma - cada
redirección, cada enlace de correo, cada href de un `<Link>` de Inertia,
cada descarga firmada tiene que salir de algún sitio. Escribir las rutas
literalmente en el código vuelve dolorosos los refactors y arriesgados
los cambios de nombre de ruta. Suprnova incluye un pequeño espacio de
nombres `url::` y un ayudante hermano `route()` que toman un nombre más
unos parámetros y te devuelven una cadena, con el percent-encoding ya
resuelto, la acuñación de firmas disponible y una verificación que
coincide byte a byte con el formato en la red de Laravel.

Este capítulo es la referencia de la superficie de generación de URLs. El
capítulo [Enrutamiento](routing.md) cubre cómo declarar rutas y ponerles
nombre; este cubre qué haces después con esos nombres.

```rust
use suprnova::{route, url};

// Búsqueda por nombre → URL
let profile = route("users.show", &[("id", "42")]).unwrap();
//   "/users/42"

// URL absoluta contra APP_URL
let absolute = url::to("/dashboard");
//   "https://app.test/dashboard"

// Enlace firmado para el restablecimiento de contraseña
let link = url::signed_route("password.reset", &[("token", reset_token)])?;
//   "/password/reset/xyz?signature=ab12..."

// Verifica en la solicitud entrante
if url::has_valid_signature(&request)? {
    // actúa en consecuencia
}
```

Todo lo de este capítulo se reexporta bajo `suprnova::url::*` y
`suprnova::route`, de modo que el código consumidor nunca tiene que meter
la mano directamente en el módulo de enrutamiento.

## Rutas nombradas

Un nombre es una etiqueta de texto que se adjunta a una ruta en el
momento del registro. Una vez que el nombre existe, `route(name, params)`
lo resuelve de vuelta a un patrón de URL y sustituye los parámetros. Los
nombres viven en un único registro global del proceso - hay una tabla
`name → path` por binario en ejecución, no una por `Router`.

```rust
use suprnova::{routes, get, post};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

La llamada `.name(...)` registra `"users.show" → "/users/{id}"`. A partir
de ese momento, cualquier punto del proceso puede resolver el nombre:

```rust
use suprnova::route;

let url = route("users.show", &[("id", "42")]);
// Some("/users/42")

let missing = route("does.not.exist", &[]);
// None
```

Volver a registrar el mismo par `(name, path)` es idempotente - útil
cuando el registro de rutas se ejecuta más de una vez durante el
arranque. Registrar un nombre bajo una ruta *distinta* entra en pánico;
esa colisión es un bug con forma de vulnerabilidad de seguridad, porque
ayudantes como `Redirect::route` apuntarían en silencio al lado que
hubiera ganado la carrera.

### Los ayudantes de búsqueda

| Función | Devuelve | Cuando la ruta no existe |
|---|---|---|
| `route(name, params)` | `Option<String>` | `None` |
| `route_with_params(name, params_map)` | `Option<String>` | `None` |
| `try_route(name, params)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |
| `try_route_with_params(name, params_map)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |

El par permisivo `route` / `route_with_params` deja tal cual en la salida
cualquier segmento `{placeholder}` sin rellenar - está bien para los
registros de depuración, pero no es seguro enviarlo a un navegador. El
par estricto `try_route` / `try_route_with_params` devuelve
`RouteUrlError::MissingParams { name, missing }` con la lista de los
placeholders sin rellenar, para que quien llama pueda fallar de forma
estrepitosa en lugar de redirigir a un usuario a `/users/{id}`.

```rust
use suprnova::routing::{try_route, RouteUrlError};

match try_route("users.show", &[]) {
    Ok(url) => /* seguro para redirigir */,
    Err(RouteUrlError::MissingParams { name, missing }) => {
        // missing == vec!["id"]
        return Err(FrameworkError::internal(
            format!("cannot build URL for {name}: missing {missing:?}"),
        ));
    }
    Err(RouteUrlError::NameNotFound(name)) => {
        return Err(FrameworkError::internal(format!("unknown route: {name}")));
    }
}
```

`Redirect::route` usa `try_route_with_params` por debajo exactamente por
esta razón - una redirección con un `{id}` en crudo en el encabezado
`Location` sería peor que fallar.

### El percent-encoding es automático

Los valores de los parámetros se codifican según las reglas de segmento
de ruta de la RFC 3986 antes de sustituirse. Eso cubre los gen-delims y
sub-delims (`/ ? # [ ] @ ! $ & ' ( ) * + , ; =`), los caracteres de
control, el espacio y el propio `%`. Los caracteres no reservados
(`A-Z a-z 0-9 - _ . ~`) pasan sin cambios.

```rust
use suprnova::route;

// Un slug que contiene una barra queda contenido en un solo segmento:
route("posts.show", &[("slug", "hello/world")]);
// Some("/posts/hello%2Fworld")

// Los intentos de path traversal no pueden escapar del segmento:
route("users.show", &[("id", "../../etc/passwd")]);
// Some("/users/..%2F..%2Fetc%2Fpasswd")

// El Unicode real pasa sin tocarse:
route("users.show", &[("id", "user-é-42")]);
// Some("/users/user-%C3%A9-42")
```

El lado que hace la coincidencia conserva este round-trip - una solicitud a
`/posts/hello%2Fworld` coincide con la ruta `/posts/{slug}` y un handler
que lee `req.param("slug")` ve `"hello/world"`, ya decodificado. Codifica
en el límite, decodifica en el límite; nunca veas los bytes en crudo en
el código del handler.

### Búsqueda inversa

Cuando tienes un patrón de ruta ya emparejado y quieres el nombre
registrado - por ejemplo para el registro en el log o para las
comprobaciones de `Request::route_is("users.show")` - usa
`route_name_for_pattern`:

```rust
use suprnova::routing::route_name_for_pattern;

let name = route_name_for_pattern("/users/{id}");
// Some("users.show")
```

Es un recorrido O(n) sobre el registro de nombres. n es el número de
nombres registrados; incluso con recuentos de rutas de cuatro cifras el
coste es despreciable frente al ciclo de vida de solicitud que lo rodea.
La función se expone para herramientas y middleware - `Request::route_is`
ya la llama por ti cuando comparas contra una ruta con nombre dentro de
un handler.

## URLs absolutas

Para todo lo demás - construir correos, compartir URLs, enviar metadatos
de Open Graph - quieres una URL absoluta con el esquema y el host
correctos. `url::to` une una ruta a `APP_URL`:

```rust
use suprnova::url;

// En el entorno: APP_URL=https://app.example.com
let url = url::to("/about");
// "https://app.example.com/about"

// Las URLs ya absolutas pasan sin cambios:
let cdn = url::to("https://cdn.example/asset.js");
// "https://cdn.example/asset.js"

let proto_relative = url::to("//cdn.example/asset.js");
// "//cdn.example/asset.js"
```

El host, el esquema y el puerto salen todos de `APP_URL`. Si `APP_URL` es
`http://localhost:8765`, entonces `url::to("/foo")` produce
`"http://localhost:8765/foo"`. La barra final de `APP_URL` se normaliza y
desaparece, así que nunca acabas con `https://host//path`.

### Forzar HTTPS

`url::secure(path)` construye la misma URL absoluta pero eleva el esquema
a `https://` aunque `APP_URL` sea `http://`:

```rust
use suprnova::url;

// En el entorno: APP_URL=http://app.example.com
url::secure("/login");
// "https://app.example.com/login"
```

En producción lo normal es fijar `APP_URL` a tu host HTTPS una vez y no
llamar nunca a `secure` directamente - la elevación es para los entornos
donde el desarrollo local va sobre HTTP pero un enlace concreto tiene que
ser HTTPS (por ejemplo, una URL de callback incrustada en una sesión de
pago).

### Leer la URL actual

Dentro de un handler, la propia solicitud es la fuente de la verdad:

```rust
use suprnova::url;

async fn breadcrumbs(req: Request) -> Response {
    let here = url::current(&req);       // "/posts/42?expand=author"
    let full = url::full(&req);          // "https://app.test/posts/42?expand=author"
    let back = url::previous("/");        // URL anterior registrada por la sesión
    // ...
}
```

| Ayudante | Devuelve | Fuente |
|---|---|---|
| `url::current(&req)` | ruta + query de esta solicitud | El `Request` actual |
| `url::full(&req)` | URL absoluta de esta solicitud | `APP_URL` + `current(&req)` |
| `url::previous(fallback)` | la URL anterior registrada por el middleware de sesión | `_previous.url` en la sesión, o `fallback` |

`previous` es lo que respalda a `Redirect::back` - el middleware de
sesión registra la URL de todo GET HTML con éxito, para que un `POST` de
formulario pueda rebotar de vuelta a la página que lo envió. Las
parciales de Inertia, las solicitudes JSON-API
(`Accept: application/json` sin `text/html`) y las respuestas que no son
2xx/3xx se omiten, así que nunca rebotas a un endpoint intermedio que el
usuario nunca vio. El middleware también se niega a registrar una URL
que no sea relativa a la raíz y del mismo origen: una ruta con forma
`//host` o `/\host` (el navegador interpreta ambas como relativas al
protocolo, no como una ruta) o que contenga en cualquier posición un
byte de control ASCII (un `TAB` o un salto de línea que el analizador de
URL del navegador elimina antes de comparar los orígenes, convirtiendo
lo que parece una ruta segura en una de las dos formas anteriores) nunca
se almacena. La misma comprobación se ejecuta de nuevo en cada lectura,
por lo que un valor guardado por una versión anterior también sigue
fallando, en lugar de considerarse fiable solo porque ya está en la
sesión. En cualquier caso, `previous` y `Redirect::back` no pueden
desviarse fuera del origen por una ruta de solicitud inusual que llegue
a la aplicación, pasada o presente.

## URLs firmadas

Las URLs firmadas te permiten acuñar una URL que demuestra que salió de
tu servidor, sin guardarla en ninguna parte. La firma es un HMAC-SHA256
sobre la forma canónica de la URL usando tu `APP_KEY`; el servidor
recalcula el HMAC en la solicitud entrante y solo acepta las firmas que
coinciden.

Echa mano de las URLs firmadas cuando:

- **Enlaces entregados por correo** - restablecimiento de contraseña,
  verificación de correo, invitación por correo, login con enlace mágico.
  La URL tiene que sobrevivir a un viaje de ida y vuelta por una bandeja
  de entrada sin poder guardarse como estado opaco.
- **Descargas efímeras** - enlaces del tipo "tu exportación CSV está
  lista" que caducan en 24 horas, alternativas a las URLs firmadas de S3
  cuando quieres que la URL siga estando en tu dominio.
- **Webhooks que apuntan de vuelta a ti** - callbacks de terceros que
  deben rechazar las llamadas falsificadas sin necesitar una consulta a
  la base de datos por solicitud.

```rust
use suprnova::url;
use chrono::Utc;

// URL firmada permanente - no caduca nunca.
let link = url::signed_route(
    "password.reset",
    &[("user", user_id), ("token", token)],
)?;
// "/password/reset/42/xyz?signature=ab12cd34..."

// URL firmada temporal - caduca dentro de una hora.
let expires_at = Utc::now().timestamp() + 3600;
let link = url::temporary_signed_route(
    "verify.email",
    &[("user", user_id)],
    expires_at,
)?;
// "/verify/email/42?expires=1748803600&signature=def012..."
```

Ten en cuenta que `expires_at_epoch_seconds` es una **marca de tiempo
UNIX absoluta**, no una duración. Calcúlala en el sitio de la llamada:

```rust
let one_hour_from_now = chrono::Utc::now().timestamp() + 3600;
let one_day_from_now  = chrono::Utc::now().timestamp() + 86_400;
```

Eso mantiene pequeña la firma del ayudante y te deja reutilizar la misma
función tanto para plazos relativos a ahora como para plazos absolutos
explícitos.

### Verificar

En el lado entrante, verificas la firma contra la solicitud en curso:

```rust
use suprnova::{url, FrameworkError, Request, Response, HttpResponse};

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    if !url::has_valid_signature(&req)? {
        return Err(FrameworkError::forbidden("Invalid or expired link"));
    }
    // La firma es buena y no ha caducado - adelante.
    let user_id = req.param("user").unwrap();
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`has_valid_signature` devuelve `true` solo cuando el HMAC coincide Y la
URL no ha caducado. Para la distinción de tres vías entre *inválida*,
*caducada* y *válida*, usa `signature_verdict`:

```rust
use suprnova::{url, FrameworkError, HttpResponse, Request, Response};
use suprnova::routing::SignatureVerdict;

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    match url::signature_verdict(&req)? {
        SignatureVerdict::Valid => {
            // Adelante.
        }
        SignatureVerdict::Expired => {
            // Rebota al usuario a una página que le explique que el enlace
            // caducó y le ofrezca enviarle uno nuevo.
            return Ok(HttpResponse::new()
                .status(302)
                .header("Location", "/password/reset-expired"));
        }
        SignatureVerdict::Invalid => {
            // Renderiza un 403 genérico - no filtres si la firma estaba
            // malformada, ausente o simplemente era incorrecta.
            return Err(FrameworkError::forbidden("Invalid link"));
        }
    }
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`signature_has_not_expired(&req)` está obsoleta y ahora responde
exactamente lo mismo que `has_valid_signature`. Echa mano en su lugar del
`signature_verdict` de arriba; una URL sin parámetro de query `expires`
es "nunca caducada" por definición, en Suprnova igual que en Laravel.

### Por qué Suprnova diverge

El `URL::signatureHasNotExpired($request)` de Laravel es literalmente "no
ha caducado", así que una firma **falsificada** vuelve como `true` -
nunca tuvo una caducidad que incumplir. El de Suprnova solía coincidir
con eso. Ya no: el ayudante exige antes una firma válida.

La razón es que `expires` lo suministra el atacante hasta que el HMAC
diga lo contrario, así que ninguna respuesta derivada de él significa
nada antes de que la firma se compruebe - y una función cuyo nombre se
lee como una salvaguarda estaba dejando pasar toda URL falsificada por
cualquier sitio que la llamara sola.

Exigir validez la colapsa en `has_valid_signature`, y por eso lleva una
marca de obsolescencia en lugar de un flag de comportamiento. Ese colapso
no es una pérdida: bajo un veredicto de tres estados no hay ningún "no ha
caducado" que un solo `bool` pueda informar con honestidad salvo `Valid`.
Si quieres distinguir *caducada* de *inválida* - para decir "solicita un
enlace nuevo" en lugar de "prohibido" - para eso está
`signature_verdict`, y lo dice en el tipo.

### Firmar URLs arbitrarias

Si la URL que quieres firmar no viene de una ruta con nombre registrada -
una URL de callback que te ha dado un tercero, una ruta construida
dinámicamente en tiempo de ejecución - usa `signed_url` directamente:

```rust
use suprnova::url;

let callback = url::signed_url(
    "/webhooks/stripe/callback?order=42",
    Some(chrono::Utc::now().timestamp() + 600),  // caducidad a los 10 minutos
)?;
```

Pasa `None` como caducidad para acuñar una firma permanente. El lado de
la verificación es el mismo - a `has_valid_signature(&req)` le da igual
si la URL se acuñó a partir de una ruta con nombre o de una ruta en
crudo.

### Formato en la red

Dos URLs que solo difieren en el orden de los parámetros de query
producen firmas idénticas, porque la forma canónica ordena los pares del
query lexicográficamente antes de hashearlos. Eso importa porque los
clientes a veces reordenan los parámetros de query en tránsito (proxies,
previsualizadores de enlaces, apps de correo móviles), y una URL firmada
que se rompiera al reordenarla sería inservible.

| Componente | Valor |
|---|---|
| Algoritmo | HMAC-SHA256 |
| Clave | Los bytes en crudo de la `APP_KEY` activa |
| Payload | `path?<sorted-query>` (se omite `?` cuando no hay parámetros) |
| Criterio de orden | `(key, value)` - todos los pares, repeticiones incluidas |
| Codificación | Digest de 64 caracteres en hexadecimal |
| Comparación | En tiempo constante vía `subtle::ConstantTimeEq` |
| Claves reservadas | `signature`, `expires` |

**Las claves repetidas se firman, no se colapsan.** `?tag=a&tag=b` lleva
ambos valores al payload, así que ninguno puede añadirse, eliminarse ni
sustituirse sin romper la firma. Ordenar por `(key, value)` en lugar de
solo por la clave es lo que mantiene total ese orden, de modo que la
garantía de reordenación de arriba se sigue cumpliendo cuando una clave
aparece más de una vez.

Vale la pena decirlo porque la alternativa muerde fuerte. Una versión
anterior construía la forma canónica sobre un mapa, que solo conservaba
el último valor de una clave repetida. `Request::query_param` devolvía el
*primero*. Así que un `?user=victim` legítimamente firmado podía
repetirse como `?user=attacker&user=victim` con la firma original: la
verificación veía `victim` y pasaba, y el handler actuaba sobre
`attacker`. La URL firmada y la ejecutada eran distintas. Los tres
accesores del query - `query_param`, `query_params` y
`Context::query_param` - resuelven ahora una clave repetida a su último
valor, y la forma canónica no pierde nada.

Un `signature` o un `expires` repetidos se rechazan de plano. Son
parámetros de control; dos de cualquiera de ellos no dejan ninguna
respuesta no arbitraria a "¿cuál manda?", y el verificador no debería ser
el componente que adivine.

El payload del HMAC excluye cualquier parámetro de query `signature`
preexistente (así que firmar sobre lo ya firmado no hace nada) y vuelve a
emitir un valor `expires` nuevo a partir de los argumentos de la llamada.
Un cliente que elimine o reescriba el `expires` rompe la firma; un
cliente que elimine el `signature` falla como `Invalid`. Ambos fallan en
cerrado.

El fragmento (`#section`) se elimina de la forma canónica porque los
navegadores nunca transmiten los fragmentos de vuelta al servidor. Firmar
sobre un fragmento invalidaría todos los enlaces en cuanto un cliente
añadiera un ancla - `?signature=...#docs` no verificaría en el lado del
servidor.

### Parámetros de query reservados

`signature` y `expires` son nombres reservados de parámetro de query. Una
ruta que legítimamente espere un parámetro de query llamado `signature` o
`expires` chocaría con la maquinaria de las URLs firmadas, y el
verificador atribuiría mal el valor. O renombras el parámetro, o
envuelves los parámetros entrantes de la ruta bajo otro espacio de
nombres.

```rust
// Mal - `signature` choca con el nombre reservado.
get!("/api/check", check)  // recibe ?signature=hash

// Bien - ponle un espacio de nombres.
get!("/api/check", check)  // recibe ?body_signature=hash
```

Las constantes se exponen por simetría con el formato en la red de
Laravel:

```rust
use suprnova::routing::{SIGNATURE_KEY, EXPIRES_KEY};
// SIGNATURE_KEY == "signature"
// EXPIRES_KEY   == "expires"
```

### Rotación de claves

Las URLs firmadas usan la misma `APP_KEY` que alimenta a `Crypt::encrypt`
y a la integridad de la cookie de sesión. Rotar `APP_KEY` invalida todas
las firmas ya acuñadas que sigan circulando - un correo de
restablecimiento de contraseña que esté en circulación se convierte en un
403 la próxima vez que el usuario lo pulse.

Para la mayoría de las aplicaciones ese es el comportamiento correcto. Si
necesitas una rotación suave con solapamiento (para que los enlaces
antiguos sigan funcionando durante una ventana de despliegue), usa
`APP_KEY_PREVIOUS` para arrastrar la clave anterior; el llavero prueba
todas las claves instaladas al verificar. Consulta el capítulo
[Hashing](hashing.md) para el panorama completo del llavero.

## Errores y casos límite

Vale la pena conocer un puñado de modos de fallo:

- **`route(name, ...)` devuelve `None`** cuando el nombre no está
  registrado. Esta es la superficie permisiva - el fallo silencioso es
  intencionado, para que el código que llama pueda recurrir a un valor
  por defecto. Usa `try_route` para un fallo estrepitoso.
- **`try_route` devuelve `Err(NameNotFound)`** para un nombre desconocido
  y `Err(MissingParams { name, missing })` cuando un `{placeholder}`
  requerido no tiene ningún valor que le corresponda.
- **`url::signed_route` y compañía devuelven `FrameworkError`** cuando la
  clave de cifrado no está instalada (por ejemplo, se te olvidó
  `APP_KEY` en `.env`). Esto falla en el arranque en producción porque
  `Crypt::init` se ejecuta durante `Server::from_config`; la ruta de
  error de aquí existe para sacar a la luz la mala configuración de forma
  evidente en lugar de producir enlaces inverificables.
- **`has_valid_signature` devuelve `Ok(false)`**, no `Err`, ante una
  firma inválida o caducada. La variante `FrameworkError` se reserva para
  los fallos del tipo "el servidor ni siquiera puede comprobarlo" (falta
  la clave).
- **Una URL firmada con el `expires` manipulado** se verifica como
  `Invalid`, no como `Expired`. El payload del HMAC incluye el valor de
  `expires`, así que cambiarlo rompe antes la firma.

```rust
use suprnova::{routing::SignatureVerdict, url};

// Todas estas son Invalid, no Expired:
url::signature_verdict(&req)?;  // falta el parámetro de query signature
url::signature_verdict(&req)?;  // la firma es basura no hexadecimal
url::signature_verdict(&req)?;  // se manipuló la ruta (/orders/1 → /orders/2)
url::signature_verdict(&req)?;  // se manipuló el valor de algún parámetro de query
url::signature_verdict(&req)?;  // se manipuló el valor de expires

// Esta es Expired:
url::signature_verdict(&req)?;  // HMAC válido, pero ahora > expires
```

## Por qué Suprnova diverge

La fachada `URL` de Laravel lleva `asset()`, `secureAsset()`,
`assetFrom()` y `action()`. Suprnova no incluye ninguna - por razones
deliberadas.

**Assets**. El planteamiento de frontend de Suprnova es Vite más los
discos del sistema de archivos ([Sistema de archivos](filesystem.md)), no
un ayudante de assets independiente. La directiva
`@vite('resources/app.ts')` de Vite (o su equivalente en el adaptador de
Inertia) emite las URLs con hash correctas en producción y la URL del
servidor de desarrollo en desarrollo. Construir un canal `URL::asset()`
paralelo repartiría el asunto de los assets entre dos sistemas que
tendrían que ponerse de acuerdo sobre el hashing, el versionado y qué
manifiesto manda. El lado de Vite ya ganó esa responsabilidad.

**Enrutamiento por acción**. El `action('UserController@show', ['id' => 1])`
de Laravel se apoya en el enrutamiento por cadena de clase de PHP - los
controladores son clases con métodos, y el framework puede hacer la
búsqueda inversa de una cadena `action`. Los handlers de Rust son
funciones libres. El análogo más cercano son las rutas con nombre, y
`route("users.show", &[("id", "1")])` ya es la interfaz correcta.
Reintroducir el enrutamiento por cadena de acción sobre los tipos de
handler de Rust no añadiría nada real frente a las rutas con nombre.

**`URL::forceScheme()` / `URL::forceRootUrl()`**. Laravel los expone para
los tests y para los sitios que están detrás de proxies inversos que no
pasan `X-Forwarded-Proto`. Suprnova resuelve ambos casos por
configuración: `APP_URL` lleva el host y el esquema canónicos; para los
entornos con proxy, el middleware de proxy de confianza
([Middleware](middleware.md)) lee los encabezados `X-Forwarded-*` y
actualiza la URL de la solicitud antes de que llegue a tu handler. No hay
nada que `forceScheme` pueda anular - `APP_URL` ya dice cuál es el
esquema.

Lo que sí aterriza aquí es la forma de cara al usuario que los
consumidores buscan, con los mismos nombres con forma de Laravel allí
donde se trasladan limpiamente. El recorte es intencionado, no un
descuido.

## Siguiente

- [Enrutamiento](routing.md) - declarar rutas, ponerles nombre, los
  grupos de rutas, el enrutamiento de recursos y la superficie completa
  de coincidencia por método
- [Respuestas](responses.md) - `Redirect::route`,
  `Redirect::signed_route`, `Redirect::back` y el resto de la familia de
  ayudantes de redirección que consume la generación de URLs
- [Hashing](hashing.md) - el ciclo de vida de `APP_KEY`, la rotación de
  claves y el llavero compartido que respalda la firma de URLs junto al
  cifrado
- [Flujos de autenticación](auth-flows.md) - los usuarios en producción
  de las URLs firmadas: restablecimiento de contraseña, verificación de
  correo y cookies de "recuérdame"
- [Solicitudes](requests.md) - `Request::path`, `Request::query`,
  `Request::route_is` y el lado inverso de todos los ayudantes de este
  capítulo

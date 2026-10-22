# CORS

`CorsMiddleware` responde las solicitudes `OPTIONS` de preflight y decora
las respuestas cross-origin corrientes con encabezados
`Access-Control-Allow-*`. Se instala una sola vez en `bootstrap()` cuando
un navegador de un origen distinto llama a tu API - APIs públicas, una
SPA alojada en otro dominio, un webview móvil, o un sitio de
documentación alojado por separado. Las apps same-origin (Inertia
servido desde el mismo host que el backend, el valor por defecto de
Suprnova) no necesitan CORS en absoluto. El middleware refleja
`HandleCors` y `config/cors.php` de Laravel, pero como un builder tipado
sobre `CorsConfig`.

## Instalarlo globalmente

```rust,ignore
use std::time::Duration;
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

pub fn register() {
    global_middleware!(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(Duration::from_secs(600)),
    ));
}
```

Un preflight es una solicitud `OPTIONS` con un encabezado
`Access-Control-Request-Method`. El router no tiene rutas `OPTIONS`, así
que un preflight nunca *coincide* con una ruta - pero el servidor de
Suprnova ejecuta la cadena de middleware global sobre las solicitudes sin
coincidencia (terminando en un 404), de modo que un `CorsMiddleware`
instalado globalmente ve el preflight y lo cortocircuita con `204` antes
de que el 404 llegue a producirse. **Por eso CORS debe instalarse de
forma global, no por ruta.**

## Elegir una política de origen

A propósito no hay `Default` para `CorsConfig`. Una política permisiva
por reflejo es pegarse un tiro en el pie en materia de seguridad, así que
hay que elegir:

| Builder | Comportamiento |
| --- | --- |
| `CorsConfig::allow_origins([...])` | Lista fija de permitidos. El origen solo se refleja de vuelta cuando coincide exactamente con una entrada. |
| `CorsConfig::any_origin()` | Wildcard `*`. Con las credenciales habilitadas, el middleware refleja el origen concreto de la solicitud en lugar de `*` (la combinación de `*` y credenciales es inválida según la especificación Fetch). |
| `.allow_origin_patterns([...])` | Patrones de regex que se suman a la lista literal. Útil para subdominios dinámicos. |

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .allow_origin_patterns([r"^https://[a-z0-9-]+\.staging\.example$"])
```

Los patrones se anclan automáticamente - `^` y `$` se anteponen y se
añaden al final si faltan, de modo que una coincidencia parcial contra
una URL de redirección como `https://evil.com/?u=https://app.example` no
puede colarse.

Un regex inválido entra en pánico en tiempo de configuración (el
arranque), no en tiempo de solicitud - saca el bug de configuración a la
luz de forma evidente en vez de fallar en modo abierto y en silencio.

`allowed_origins_patterns` (alias con nombre de Laravel) también está
disponible.

## Acotar a qué rutas se aplica CORS

La configuración `cors.php` de Laravel tiene un array `paths` (`['api/*',
'sanctum/csrf-cookie']`) que limita la aplicación de CORS a patrones de
URL concretos. Suprnova lo refleja:

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .paths(["api/*", "sanctum/csrf-cookie"])
```

Sin `paths` definido, CORS se ejecuta en cada solicitud (el valor por
defecto de Suprnova - dado que el middleware es opt-in por registro). Con
al menos un patrón definido, solo las solicitudes que coinciden reciben
tratamiento de CORS (tanto los preflights **como** la decoración de la
respuesta real); todo lo demás pasa de largo intacto.

Los patrones usan la semántica de `Str::is` de Laravel: `*` es un
wildcard multisegmento y voraz que atraviesa `/`. La `/` inicial se
normaliza, así que `"api/*"` y `"/api/*"` son equivalentes.

```rust,ignore
"api/*"             // coincide con /api/users, /api/users/42
"api/*/posts"       // coincide con /api/v2/posts, /api/v1/posts
"sanctum/csrf-cookie" // literal de coincidencia exacta
"*"                 // coincide con todo
```

## Omitir mediante un predicado

Para predicados sobre la forma de la solicitud que no encajan en un
patrón de ruta (omitir según un encabezado, ejecutar CORS solo en
producción, omitir durante las verificaciones de salud), usa
`skip_when`:

```rust,ignore
CorsConfig::any_origin()
    .skip_when(|req| req.header("X-Internal-Call").is_some())
    .skip_when(|req| req.path() == "/healthz")
```

Refleja `HandleCors::skipWhen(Closure)` de Laravel, pero vive en la
política en lugar de como estado global mutable. Se pueden registrar
varios callbacks `skip_when`; que cualquiera devuelva `true` omite CORS.

## Métodos, encabezados, encabezados expuestos

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .methods(["GET", "POST", "DELETE"])           // por defecto = GET/POST/PUT/PATCH/DELETE/OPTIONS/HEAD
    .allow_headers(["Content-Type", "X-CSRF-TOKEN"])  // restringe; por defecto = refleja la solicitud
    .allow_any_headers()                          // "refleja lo que se haya pedido", explícito
    .expose_headers(["X-Total-Count", "Link"])    // encabezados que JS puede leer en la respuesta
```

Alias con nombres de Laravel (para que quienes vienen de `cors.php`
encuentren lo que esperan):

- `allowed_methods(...)` ≡ `methods(...)`
- `allowed_headers(...)` ≡ `allow_headers(...)`
- `exposed_headers(...)` ≡ `expose_headers(...)`
- `allowed_origins_patterns(...)` ≡ `allow_origin_patterns(...)`
- `supports_credentials(...)` ≡ `allow_credentials(...)`

## Credenciales y `*`

Según la especificación Fetch, `Access-Control-Allow-Origin: *` es
inválido junto con credenciales - el navegador rechaza la respuesta. Con
una lista de orígenes explícita (`allow_origins([...])`) más
`allow_credentials(true)`, el middleware refleja el `Origin` concreto de
la solicitud en lugar de `*`, y la política funciona como se espera.

**`any_origin() + allow_credentials(true)` entra en pánico al construir
la política.** La combinación es una evasión completa de la lista de
orígenes permitidos: cualquier página atacante puede hacer solicitudes
cross-origin con credenciales y leer las respuestas. En vez de emitir el
encabezado equivocado en tiempo de ejecución, el constructor de la
política falla de forma estrepitosa para que la mala configuración nunca
llegue a un despliegue en marcha. Usa en su lugar una lista explícita de
permitidos:

```rust,ignore
// CORRECTO - lista explícita de permitidos con credenciales.
CorsConfig::allow_origins(["https://app.example"]).allow_credentials(true)
// → ante una solicitud con Origin: https://app.example
// → respuesta: Access-Control-Allow-Origin: https://app.example
//              Access-Control-Allow-Credentials: true

// RECHAZADO al construir la política - entra en pánico con un mensaje de remediación.
// CorsConfig::any_origin().allow_credentials(true)
```

## Max-age

```rust,ignore
.max_age(Duration::from_secs(600))   // tipado
.max_age_secs(600)                   // segundos enteros al estilo de Laravel
```

`Access-Control-Max-Age` le dice al navegador cuánto tiempo puede cachear
el resultado del preflight. Más alto = menos idas y vueltas de preflight,
y los cambios de política tardan más en propagarse.

## Lo que el middleware emite en realidad

### Preflight (`OPTIONS` + `Access-Control-Request-Method`)

Si el origen está permitido:

```
HTTP/1.1 204 No Content
Access-Control-Allow-Origin: <origin>
Access-Control-Allow-Credentials: true        // con credenciales habilitadas
Access-Control-Allow-Methods: GET, POST, ...
Access-Control-Allow-Headers: <reflected or fixed>
Access-Control-Max-Age: 600                   // cuando está definido
Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers
```

Si el origen no está permitido: un `204` a secas + `Vary` (sin ningún
`Access-Control-*`). Es la comprobación de encabezado ausente del
navegador la que produce el error de CORS - igual que la convención de
`tower-http`.

### La respuesta cross-origin real

Cuando la solicitud lleva un encabezado `Origin` y el origen está
permitido:

```
Access-Control-Allow-Origin: <origin or *>
Access-Control-Allow-Credentials: true        // cuando está habilitado
Access-Control-Expose-Headers: X-Total, Link  // cuando está configurado
Vary: Origin                                  // solo cuando no es "*"
```

Un ACAO con `*` es idéntico para todos los orígenes, así que no hace
falta ningún `Vary`; un origen concreto varía según el origen, de modo
que las cachés compartidas tienen que indexar por él.

## Probar los handlers con CORS

CORS se aplica del lado del navegador - el servidor ejecuta el handler
igualmente aunque el origen no esté permitido; simplemente no decora la
respuesta. Ese es el comportamiento que se puede probar:

```rust,ignore
let (status, headers, body) = request_with_origin(
    "/api/data",
    "https://app.example",
).await;
assert_eq!(status, 200);
assert_eq!(
    headers.get("access-control-allow-origin"),
    Some(&"https://app.example".to_string()),
);
```

Para un origen no permitido, el handler se ejecuta y el cuerpo vuelve,
pero es la ausencia de `Access-Control-Allow-Origin` lo que impide al
navegador leerlo.

## Matriz de paridad con Laravel

| `cors.php` de Laravel | Builder de Suprnova |
| --- | --- |
| `paths` | `.paths([...])` |
| `allowed_methods` | `.methods([...])` / `.allowed_methods([...])` |
| `allowed_origins` | `CorsConfig::allow_origins([...])` |
| `allowed_origins_patterns` | `.allow_origin_patterns([...])` / `.allowed_origins_patterns([...])` |
| `allowed_headers` | `.allow_headers([...])` / `.allowed_headers([...])` |
| `exposed_headers` | `.expose_headers([...])` / `.exposed_headers([...])` |
| `max_age` | `.max_age(Duration)` / `.max_age_secs(u64)` |
| `supports_credentials` | `.allow_credentials(bool)` / `.supports_credentials(bool)` |
| `HandleCors::skipWhen(closure)` | `.skip_when(\|req\| ...)` |

El middleware se registra de forma global en lugar del "instalado
automáticamente para `paths`" al estilo de Laravel - la cadena de
middleware de Suprnova es explícita; consulta [Middleware](middleware.md)
para el diseño.

### Por qué Suprnova diverge

El `HandleCors` de Laravel se adjunta automáticamente al kernel y lee su
política de `config/cors.php`. La forma funciona en PHP porque el array
de configuración es el único sitio donde un framework de una solicitud
por proceso puede compartir configuración sin reevaluarla en cada
solicitud. Suprnova expone las mismas opciones como un builder tipado
`CorsConfig` que se registra explícitamente con `global_middleware!`, lo
que mantiene la cadena de middleware visible en `bootstrap()` y deja que
el compilador haga cumplir la elección entre lista de permitidos y
wildcard (no hay `Default` para `CorsConfig`, así que no se puede
desplegar por accidente un `Access-Control-Allow-Origin: *` por haber
olvidado rellenar un valor de configuración).

La otra divergencia es que los preflights alcanzan el middleware incluso
en rutas sin enrutar. Laravel pasa `OPTIONS` por su router, de modo que
el preflight coincide con una ruta `OPTIONS` (registrada automáticamente
para cada ruta REST). El router de Suprnova no tiene rutas `OPTIONS`; en
su lugar, el servidor ejecuta la cadena de middleware global sobre las
solicitudes sin coincidencia antes de devolver un 404, así que un
`CorsMiddleware` instalado globalmente cortocircuita el preflight con
`204` antes de que se tome el camino del no encontrado. Por eso CORS
*debe* instalarse de forma global - un registro por ruta nunca vería el
preflight.

## Siguiente

- [Middleware](middleware.md) - el trait, la cadena, el registro global
  frente al registro por ruta, los ganchos terminables
- [CSRF](csrf.md) - el otro middleware global que la mayoría de las apps
  instala junto a CORS
- [Enrutamiento](routing.md) - cómo se hacen coincidir las rutas (y por
  qué los preflights no coinciden), más el camino sin fallback sobre el
  que corre la cadena global
- [Ciclo de vida de la solicitud](lifecycle.md) - dónde se sitúa CORS en
  la cadena respecto a la sesión, CSRF y el handler
- [Configuración](configuration.md) - patrones de configuración tipada
  para middleware que necesita ajustes dirigidos por el entorno

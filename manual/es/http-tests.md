# Pruebas HTTP

Este capítulo muestra cómo probar tu superficie HTTP - rutas,
middleware, flujos de autenticación, respuestas de error - conduciendo
el pipeline de solicitudes del framework a través de
`suprnova::handle_request`. Si has escrito tests de feature de
Laravel con `$this->get('/users')` y has hecho aserciones sobre
`$response->status()`, este es el equivalente en Suprnova: el mismo
`Router` que montas en producción se ejecuta en el test, se dispara
cada middleware, el límite de pánico sigue atrapando, y la respuesta
es, byte por byte, lo que vería un cliente real.

## La superficie de test

Hay exactamente tres piezas:

| Pieza | Rol |
|---|---|
| `Router` | Las rutas bajo prueba - construidas de la misma forma que en producción |
| `MiddlewareRegistry` | La pila de middleware global - también construida de la misma forma |
| `handle_request(router, registry, req) -> hyper::Response<…>` | El driver dentro del proceso - ejecuta una solicitud de punta a punta |

`handle_request` es la misma función que llama `Server::run` por cada
solicitud, expuesta para los tests y para quienes embeben el
framework. Todo lo que funciona en producción funciona aquí - el
envoltorio de recuperación de pánico, el alcance del id de solicitud,
el alcance de la flash bag de Inertia, el alcance del estado de auth
de la solicitud, el recorte del cuerpo en HEAD, la terminación
posterior a la respuesta. No hay ningún "modo de test" que sustituya
esto por un pipeline más silencioso.

`handle_request_with_peer` es la misma llamada con un
`Option<std::net::IpAddr>` explícito para el par que se conecta -
útil cuando quieres hacer aserciones sobre la resolución de
`Request::ip()` sin montar encabezados de proxy.

## El problema del cuerpo de hyper

La única complicación que conviene conocer de antemano:
`handle_request` toma un `hyper::Request<hyper::body::Incoming>`.
`Incoming` es el tipo de cuerpo en streaming interno de hyper; no
puedes construir uno con `Full::new(bytes)` ni con ninguno de los
tipos de cuerpo en memoria. Solo sale de una conexión de hyper.

Hay dos formas limpias de evitarlo:

1. **Loopback TCP** - vincula un listener TCP en `127.0.0.1:0`, sirve
   un accept dentro de un `service_fn`, envía la solicitud a través de
   un cliente de hyper, y deja que `Incoming` se produzca de forma
   natural en el lado del servidor. Esto es lo que ya hace cada test
   de integración en el framework.
2. **Construcción de `Request` dentro del proceso** - para tests que
   solo necesitan inspeccionar accesores de `Request` (encabezados,
   parámetros de ruta, IP, análisis de JSON) sin pasar por el
   enrutamiento, usa el mismo patrón de captura por loopback TCP pero
   con un servicio que saca el `Request` hacia un
   `oneshot::channel` en lugar de ejecutarlo. El archivo
   `framework/tests/http_request_accessors.rs` tiene este ayudante
   `build_request()` textual.

Ambos patrones producen cuerpos `Incoming` reales. El loopback es
local, síncrono en términos de reloj de pared del test
(microsegundos), y nunca toca la red fuera de `lo`. No hay una forma
más lenta ni más simple que preserve el contrato.

### Por qué Suprnova diverge

El `$this->get('/users')` de Laravel funciona porque el ciclo de vida
de la solicitud de PHP es "construye un objeto `Request`, despáchalo
a través del kernel". El kernel toma el objeto en memoria
directamente; no hay ningún tipo de cuerpo que fuerce un transporte.
El servidor de Suprnova está construido sobre hyper, y el tipo de
cuerpo de hyper tiene una postura deliberada por buenas razones
(streaming, contrapresión, cero copias). La superficie de test hereda
esa restricción.

Lo que cambias a cambio de esa restricción es fidelidad. Cada detalle
de la ruta de solicitud de producción - análisis de encabezados,
límites de cuerpo, upgrades de conexión - se ejecuta igual en los
tests. Nunca vas a tener un test que pasa porque el harness de test
se saltó una capa que el servidor real sí ejecuta.

## Un primer test de punta a punta

Aquí hay un test completo y funcional que monta una única ruta,
envía un GET contra ella, y hace aserciones sobre el estado y el
cuerpo.

```rust
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::{MiddlewareRegistry, Request, Router, handle_request};

async fn spawn_server(router: Router, accepts: usize) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else { return };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move {
                        Ok::<_, Infallible>(handle_request(router, middleware, req).await)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

async fn send_get(addr: SocketAddr, path: &str) -> (u16, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_get timeout")
        .expect("hyper send_request");
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status.as_u16(), bytes)
}

#[tokio::test]
async fn get_root_returns_hello() {
    let router = Router::new().get("/", |_req: Request| async { text("hello") });
    let addr = spawn_server(router, 1).await;

    let (status, body) = send_get(addr, "/").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"hello");
}
```

Esa es la forma completa. Copia los dos ayudantes por crate, ajústalos
para tu suite (varios accepts, captura de encabezados, captura de
cuerpo). El framework mismo usa ayudantes casi idénticos en
`framework/tests/cors_middleware.rs`,
`framework/tests/middleware_panic_safety.rs`, y
`framework/tests/email_verified_middleware.rs`.

El argumento `accepts` acota cuántas conexiones sirve el bucle de
accept antes de salir. Uno basta para una sola solicitud; sube a dos
o más cuando un test ejercita la recuperación posterior a un pánico
(consulta [Probar el límite de pánico](#probar-el-límite-de-pánico)).

## Construir una solicitud

Dentro de `send_get` viste:

```rust
let req = hyper::Request::builder()
    .method("GET")
    .uri("/users/42")
    .header("Host", "localhost")
    .header("Content-Length", "0")
    .body(Full::new(Bytes::new()))
    .unwrap();
```

Esa es la forma canónica. Algunas cosas que conviene saber:

- **El encabezado `Host`**. Hyper rechaza las solicitudes HTTP/1.1
  sin uno. Inclúyelo siempre; el valor no importa salvo que tu
  handler dependa de él.
- **`Content-Length: 0`**. Coincide con el cuerpo. Hyper lo calcula
  por ti con `Full::new(Bytes::new())`, pero ser explícito se lee más
  limpio en los tests.
- **Tipos de cuerpo**. El lado cliente envía `Full<Bytes>`. El lado
  servidor recibe `Incoming`. En los tests solo construyes
  solicitudes `Full<Bytes>`; el framework las recibe como `Incoming`
  después de la conversión por conexión de hyper.

Un POST con un cuerpo JSON:

```rust
let body_bytes = serde_json::to_vec(&serde_json::json!({
    "name": "Alice",
    "email": "alice@example.com"
})).unwrap();

let req = hyper::Request::builder()
    .method("POST")
    .uri("/users")
    .header("Host", "localhost")
    .header("content-type", "application/json")
    .header("content-length", body_bytes.len())
    .body(Full::new(Bytes::from(body_bytes)))
    .unwrap();
```

## Verificar la respuesta

La respuesta que vuelve de `handle_request` es un
`hyper::Response<BoxBody<Bytes, Infallible>>`. Tres cosas que vas a
leer de ella:

```rust
let (parts, body) = resp.into_parts();

// 1. Estado.
assert_eq!(parts.status.as_u16(), 200);

// 2. Encabezados - búsqueda sin distinguir mayúsculas de minúsculas.
let location = parts.headers.get("location").and_then(|v| v.to_str().ok());
assert_eq!(location, Some("/login"));

// 3. Cuerpo - recógelo en bytes, luego analízalo.
use http_body_util::BodyExt;
let bytes = body.collect().await.unwrap().to_bytes();

// Como texto:
let text = String::from_utf8_lossy(&bytes);

// Como JSON:
let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
assert_eq!(value["message"], "ok");
```

Para las respuestas de error, la forma del cuerpo es fija y está
documentada en [Modelo de errores](error-model.md) - `message`,
`errors`, `request_id`, y un `debug_message` opcional. La clave
`request_id` siempre está presente (puede ser `null` fuera de un
alcance de solicitud), que es lo que hay que verificar cuando
compruebas que el middleware de request-id se ejecutó.

## Probar middleware

Los tests de middleware se ven idénticos a los tests de ruta; la
única diferencia es lo que le añades con `.append()` al registry
antes de lanzar el servidor.

### Probar middleware global

Pasa el middleware a `MiddlewareRegistry::new().append(...)` y usa
ese registry - varios middlewares se ejecutan en el orden en que se
añadieron, `prepend` pone uno nuevo al frente.

```rust
use suprnova::{CorsConfig, CorsMiddleware, MiddlewareRegistry};

fn cors_registry() -> MiddlewareRegistry {
    MiddlewareRegistry::new().append(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(std::time::Duration::from_secs(600)),
    ))
}

#[tokio::test]
async fn cors_preflight_returns_204_with_headers() {
    let router = Router::new();
    // La forma de 3 argumentos de `spawn_server` te permite conectar
// un MiddlewareRegistry no vacío - copia el ayudante de
// framework/tests/cors_middleware.rs (son ~30 líneas).
let addr = spawn_server(router, cors_registry(), 1).await;

    let (status, headers, _) = options(
        addr,
        "/anything",
        &[
            ("Origin", "https://app.example"),
            ("Access-Control-Request-Method", "POST"),
        ],
    ).await;

    assert_eq!(status, 204);
    assert_eq!(
        headers.get("access-control-allow-origin").map(String::as_str),
        Some("https://app.example"),
    );
}
```

Este test demuestra más que la lógica de CORS en sí misma: demuestra
que el middleware global también se ejecuta sobre solicitudes **no
enrutadas**, que es el contrato que garantiza el framework (de otro
modo, un preflight OPTIONS que nunca coincide con una ruta se
saltaría CORS). Consulta `framework/tests/cors_middleware.rs` para la
suite completa.

### Probar middleware específico de ruta

Adjúntalo con `.middleware(...)` sobre el builder de la ruta,
exactamente como en producción. Luego prueba la ruta con normalidad -
la cadena de middleware se construye a partir del mismo registro.

```rust
let router = Router::new()
    .get("/admin/dashboard", |_req| async { text("admin") })
    .middleware(RequireRole::new("admin"));

let (status, _) = send_get(addr, "/admin/dashboard").await;
assert_eq!(status, 403); // solicitud sin autenticar
```

### Preestablecer el usuario autenticado

Los tests de flujo de autenticación reales necesitan un usuario ya
conectado. El patrón más limpio es un pequeño middleware puntual que
llama a `Auth::set_user` antes del middleware bajo prueba. El propio
`framework/tests/email_verified_middleware.rs` del framework usa
esto:

```rust
use std::any::Any;
use std::sync::Arc;
use suprnova::{Auth, Authenticatable, Middleware, Next, Request, Response};

struct UserById(String);

impl Authenticatable for UserById {
    fn get_auth_identifier(&self) -> String { self.0.clone() }
    fn as_any(&self) -> &dyn Any { self }
}

struct LoginAs(String);

#[async_trait::async_trait]
impl Middleware for LoginAs {
    async fn handle(&self, request: Request, next: Next) -> Response {
        Auth::set_user(Arc::new(UserById(self.0.clone())));
        next(request).await
    }
}
```

Luego, en el test:

```rust
let registry = MiddlewareRegistry::new()
    .append(LoginAs("user-id-123".to_string()))
    .append(EnsureEmailVerifiedMiddleware::new());
```

`LoginAs` se ejecuta primero, instala el usuario en el estado de auth
por solicitud, y el middleware bajo prueba ve `Auth::id() ==
Some(...)` sin llegar a emitir nunca un login real. El alcance del
estado de auth lo monta el propio `handle_request` - el mismo que se
ejecuta en producción - así que el usuario es visible para todo
middleware posterior y para el handler.

## Probar la vinculación de modelo de ruta

La vinculación de modelo de ruta convierte `/users/{id}` en un
argumento tipado `User`. La vinculación se ejecuta como parte de la
cadena de extractores del handler, así que un test de punta a punta
normal la ejercita gratis:

```rust
#[suprnova::model(table = "users")]
pub struct User {
    pub id: i64,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[tokio::test]
async fn show_user_binds_from_route_param() {
    // Inserta un usuario de test vía el modelo. La configuración de
    // la base de datos se omite - consulta el capítulo de pruebas
    // para los patrones de `TestDatabase`.
    let user = User::create(suprnova::attrs! {
        email: "bound@example.com"
    }).await.unwrap();

    let router = Router::new().get("/users/{id}", |req: Request| async move {
        let id: i64 = req.param("id")?.parse()
            .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
        let user = User::find_or_fail(id).await?;
        suprnova::http::json(serde_json::json!({ "email": user.email }))
    });

    let addr = spawn_server(router, 1).await;
    let (status, body) = send_get(addr, &format!("/users/{}", user.id)).await;

    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["email"], "bound@example.com");
}
```

Para tests de vinculación en aislamiento, sin router y sin bucle TCP,
sintetiza tú mismo los parámetros de ruta con
`Request::with_params(...)` (consulta [Ganchos de builder sobre
`Request`](#ganchos-de-builder-sobre-request) más abajo). Ese es el
patrón que usa `framework/tests/data_route_params.rs` para probar
extractores `#[derive(Data)]` contra parámetros sintetizados.

## Probar flujos de autenticación de punta a punta

Un test de flujo de autenticación real registra un usuario, conduce
la ruta de login, extrae la cookie de sesión de la respuesta, y la
vuelve a enviar sobre una ruta protegida. Cuatro pasos, todos a nivel
de red:

```rust
#[tokio::test]
async fn login_flow_issues_session_cookie() {
    // 1. Bootstrap: crea el usuario.
    Auth::password()
        .register("alice@example.com", "longpassword123")
        .await.expect("register");

    // 2. Monta las rutas.
    let router = Router::new()
        .post("/login", login_handler)
        .get("/dashboard", |_req: Request| async { text("dashboard") });
    let addr = spawn_server(router, 2).await;

    // 3. Conduce el login; captura el encabezado Set-Cookie.
    let login = post_json(addr, "/login", serde_json::json!({
        "email": "alice@example.com",
        "password": "longpassword123",
    })).await;
    assert_eq!(login.status, 200);
    let cookie = extract_session_cookie(&login.headers);

    // 4. Repite la cookie contra la ruta protegida.
    let (status, body) = get_with_cookie(addr, "/dashboard", &cookie).await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"dashboard");
}
```

`extract_session_cookie` y `get_with_cookie` son plomería directa de
encabezados y cookies - `framework/tests/auth_http_middleware.rs`
tiene una implementación completa. La idea: todo el flujo se ejecuta
a través del `SessionMiddleware` real, el guard `Auth` real, la
resolución `Authenticatable` real. El test verifica el contrato tal
como viaja por la red, no una simulación de él.

## Probar el límite de pánico

Un pánico dentro de un handler no debe tumbar el servidor. El
envoltorio de recuperación de pánico (`execute_chain_safely`) lo
atrapa y lo convierte en un 500 a través de la misma ruta por la que
fluyen los errores devueltos. Puedes verificar esto sin ninguna
infraestructura de test especial - establece `accepts >= 2` para que
el listener sobreviva al pánico:

```rust
#[tokio::test]
async fn panicking_handler_yields_500_and_server_survives() {
    let router = Router::new()
        .get("/panic", |_req: Request| async {
            panic!("intentional test panic");
            #[allow(unreachable_code)] text("unreachable")
        })
        .get("/ok", |_req: Request| async { text("ok") });

    let addr = spawn_server(router, 4).await;

    // Primero: el pánico se traduce en un 500 sanitizado.
    let (s1, body) = send_get(addr, "/panic").await;
    assert_eq!(s1, 500);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["message"], "Internal Server Error");
    assert!(parsed.get("request_id").is_some());

    // Segundo: el listener TCP sobrevive. La siguiente solicitud es normal.
    let (s2, body2) = send_get(addr, "/ok").await;
    assert_eq!(s2, 200);
    assert_eq!(&body2[..], b"ok");
}
```

## Probar accesores sin pasar por el enrutamiento

A veces quieres probar un accesor de `Request` (`bearer_token`,
`is_method`, `ip`, `is_json`, etc.) sin levantar un router en
absoluto. El truco es un harness pequeño que ejecuta un servicio de
hyper cuyo único trabajo es construir el `Request` y devolverlo a
través de un `tokio::sync::oneshot::channel`:

```rust
let (req_tx, req_rx) = tokio::sync::oneshot::channel::<suprnova::Request>();
// ... servicio de hyper por loopback cuyo service_fn hace:
//     let req = suprnova::Request::new(hyper_req);
//     let _  = req_tx.send(req);
//     devuelve un 200 con un cuerpo vacío
let req = req_rx.await.unwrap();
```

`framework/tests/http_request_accessors.rs` tiene el ayudante
completo `build_request(builder, body) -> Request`. Cópialo una vez
por crate y cada test de accesor se lee con limpieza:

```rust
#[tokio::test]
async fn bearer_token_extracts_simple_token() {
    let req = build_request(
        hyper::Request::builder()
            .method("GET")
            .uri("/api/users")
            .header("Authorization", "Bearer secret-token-123"),
        "",
    ).await;
    assert_eq!(req.bearer_token().as_deref(), Some("secret-token-123"));
}
```

El `Request` es real (producido por hyper a partir de un intercambio
de red real), pero no se ejecutó ningún enrutamiento ni middleware -
exactamente lo que quieres cuando la unidad bajo prueba es el accesor
mismo.

## Ganchos de builder sobre `Request`

Cuando tienes un `Request` en la mano y necesitas fingir una pieza de
la capa de enrutamiento, ayudan tres métodos de builder:

```rust
impl Request {
    pub fn with_params(mut self, params: HashMap<String, String>) -> Self;
    pub fn with_route_pattern(mut self, pattern: String) -> Self;
    pub fn with_peer_addr(mut self, addr: std::net::IpAddr) -> Self;
}
```

Son los mismos métodos que llama el servidor cuando despacha una
ruta que coincidió - `Router` llama a `with_params` después de que
`matchit` devuelve un resultado, `with_route_pattern` para que
`req.route_pattern()` se resuelva, y `with_peer_addr` en cuanto
conoce la IP del socket TCP aceptado. En los tests se llaman a mano
para cortocircuitar la misma configuración.

```rust
let req = Request::new(hyper_req)
    .with_params(HashMap::from([("id".into(), "42".into())]))
    .with_route_pattern("/users/{id}".into())
    .with_peer_addr("192.168.1.10".parse().unwrap());

assert_eq!(req.param("id").unwrap(), "42");
assert_eq!(req.ip(), Some("192.168.1.10".parse().unwrap()));
```

## Cosas a tener en cuenta

Una lista breve de trampas que atrapan a quienes escriben esto por
primera vez:

- **`Incoming` es solo del lado del servidor.** No puedes construir
  uno en tu test. El loopback TCP (o la captura de servicio dentro
  del proceso) es la única ruta - no existe ningún constructor de
  "construye un `Request` a partir de un cuerpo `Vec<u8>`".
- **No compartas estado entre tests.** Cada `#[tokio::test]` obtiene
  su propio runtime; la contaminación entre tests suele significar
  que estás compartiendo un global (`once_cell`, `lazy_static`, una
  variable de entorno). Para el estado de BD, consulta
  `TestDatabase` en [Pruebas](testing.md).
- **Las cookies necesitan un cliente real.** No hay ningún cookie jar
  automático - enhebra el `Set-Cookie` de una respuesta hacia el
  `Cookie` de la siguiente. Consulta
  `framework/tests/auth_http_middleware.rs` para el patrón.
- **El spawn de terminación posterior a la respuesta no es
  bloqueante.** Si quieres hacer aserciones sobre efectos secundarios
  que se ejecutan vía `Terminable`, sondéalos - la respuesta vuelve
  al cliente antes de que el gancho se ejecute.

## Dónde vive cada pieza

| Pieza | Archivo |
|---|---|
| `handle_request`, `handle_request_with_peer` | `framework/src/server.rs` |
| `Request::new`, `with_params`, `with_route_pattern`, `with_peer_addr` | `framework/src/http/request.rs` |
| `MiddlewareRegistry::new`, `append`, `prepend` | `framework/src/middleware/registry.rs` |
| Harness de test por loopback (canónico) | `framework/tests/cors_middleware.rs` |
| Harness de captura de `Request` dentro del proceso | `framework/tests/http_request_accessors.rs` |
| Patrón de test del límite de pánico | `framework/tests/middleware_panic_safety.rs` |
| Patrón de punta a punta de auth + middleware | `framework/tests/email_verified_middleware.rs` |

## Siguiente

- [Pruebas](testing.md) - `#[suprnova_test]`, `TestDatabase`, las
  macros `describe!`/`test!`/`expect!`, y la superficie a nivel de
  unidad
- [Modelo de errores](error-model.md) - la forma JSON que usa cada
  respuesta de error, la regla de sanitización de los 5xx, y qué
  significa `request_id` en el cuerpo de un test
- [Middleware](middleware.md) - escribir el middleware que pruebas
  aquí, y el ciclo de vida global frente a por ruta
- [Enrutamiento](routing.md) - el `Router` que montas tanto en
  producción como en los tests, los parámetros de ruta, los nombres
  de ruta, las URLs firmadas
- [Autenticación](authentication.md) - la fachada `Auth`,
  `Authenticatable`, los guards, y cómo `Auth::set_user` interactúa
  con el alcance de solicitud que instala `handle_request`

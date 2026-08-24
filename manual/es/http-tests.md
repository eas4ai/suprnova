# Pruebas HTTP

Este capítulo muestra cómo probar tu superficie HTTP - rutas, middleware, flujos de autenticación, respuestas de error - conduciendo el pipeline de solicitudes del framework a través de `suprnova::handle_request`. Si has escrito tests de feature de Laravel con `$this->get('/users')` y has hecho aserciones sobre `$response->status()`, este es el equivalente en Suprnova: el mismo `Router` que montas en producción se ejecuta en el test, se dispara cada middleware, el límite de pánico sigue atrapando, y la respuesta es, byte por byte, lo que vería un cliente real.

## La superficie de test

Hay exactamente tres piezas:

| Pieza | Rol |
|---|---|
| `Router` | Las rutas bajo prueba - construidas de la misma forma que en producción |
| `MiddlewareRegistry` | La pila de middleware global - también construida de la misma forma |
| `handle_request(router, registry, req) -> hyper::Response<…>` | El driver dentro del proceso - ejecuta una solicitud de punta a punta |

`handle_request` es la misma función que llama `Server::run` por cada solicitud, expuesta para los tests y para quienes embeben el framework. Todo lo que funciona en producción funciona aquí - el envoltorio de recuperación de pánico, el alcance del id de solicitud, el alcance de la flash bag de Inertia, el alcance del estado de auth de la solicitud, el recorte del cuerpo en HEAD, la terminación posterior a la respuesta. No hay ningún "modo de test" que sustituya esto por un pipeline más silencioso.

`handle_request_with_peer` es la misma llamada con un `Option<std::net::IpAddr>` explícito para el par que se conecta - útil cuando quieres hacer aserciones sobre la resolución de `Request::ip()` sin montar encabezados de proxy.

## El problema del cuerpo de hyper

La única complicación que conviene conocer de antemano: `handle_request` toma un `hyper::Request<hyper::body::Incoming>`. `Incoming` es el tipo de cuerpo en streaming interno de hyper; no puedes construir uno con `Full::new(bytes)` ni con ninguno de los tipos de cuerpo en memoria. Solo sale de una conexión de hyper.

Hay dos formas limpias de evitarlo:

1. **Loopback TCP** - vincula un listener TCP en `127.0.0.1:0`, sirve un accept dentro de un `service_fn`, envía la solicitud a través de un cliente de hyper, y deja que `Incoming` se produzca de forma natural en el lado del servidor. Esto es lo que ya hace cada test de integración en el framework.
2. **Construcción de `Request` dentro del proceso** - para tests que solo necesitan inspeccionar accesores de `Request` (encabezados, parámetros de ruta, IP, análisis de JSON) sin pasar por el enrutamiento, usa el mismo patrón de captura por loopback TCP pero con un servicio que saca el `Request` hacia un `oneshot::channel` en lugar de ejecutarlo. El archivo `framework/tests/http_request_accessors.rs` tiene este ayudante `build_request()` textual.

Ambos patrones producen cuerpos `Incoming` reales. El loopback es local, síncrono en términos de reloj de pared del test (microsegundos), y nunca toca la red fuera de `lo`. No hay una forma más lenta ni más simple que preserve el contrato.

### Por qué Suprnova diverge

El `$this->get('/users')` de Laravel funciona porque el ciclo de vida de la solicitud de PHP es "construye un objeto `Request`, despáchalo a través del kernel". El kernel toma el objeto en memoria directamente; no hay ningún tipo de cuerpo que fuerce un transporte. El servidor de Suprnova está construido sobre hyper, y el tipo de cuerpo de hyper tiene una postura deliberada por buenas razones (streaming, contrapresión, cero copias). La superficie de test hereda esa restricción.

Lo que cambias a cambio de esa restricción es fidelidad. Cada detalle de la ruta de solicitud de producción - análisis de encabezados, límites de cuerpo, upgrades de conexión - se ejecuta igual en los tests. Nunca vas a tener un test que pasa porque el harness de test se saltó una capa que el servidor real sí ejecuta.

## Un primer test de punta a punta

Aquí hay un test completo y funcional que monta una única ruta, envía un GET contra ella, y hace aserciones sobre el estado y el cuerpo.

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

async fn spawn_server(
    router: Router,
    middleware: MiddlewareRegistry,
    accepts: usize,
) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(middleware);
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
    let addr = spawn_server(router.into(), MiddlewareRegistry::new(), 1).await;

    let (status, body) = send_get(addr, "/").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"hello");
}
```

Eso es toda la forma. Copia los dos ayudantes por crate, ajústalos para la suite (múltiples accepts, captura de encabezados, captura de cuerpo). El propio framework usa ayudantes casi idénticos en `framework/tests/cors_middleware.rs`, `framework/tests/middleware_panic_safety.rs`, y `framework/tests/email_verified_middleware.rs`.

El argumento `accepts` limita cuántas conexiones el loop de accept sirve antes de salir. Uno es suficiente para una única solicitud; aumenta a dos-o-más cuando un test ejercita recuperación posterior a pánico (ver [Probar el límite de pánico](#probar-el-límite-de-pánico)).

## Construyendo una solicitud

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

Esa es la forma canónica. Algunas cosas que conviene conocer:

- **Encabezado `Host`.** Hyper rechaza solicitudes HTTP/1.1 sin uno. Siempre inclúyelo; el valor no importa a menos que tu handler se base en él.
- **`Content-Length: 0`.** Coincide con el cuerpo. Hyper computa esto por ti con `Full::new(Bytes::new())`, pero ser explícito se lee más limpio en tests.
- **Tipos de cuerpo.** El lado cliente envía `Full<Bytes>`. El lado servidor recibe `Incoming`. Solo construyes solicitudes `Full<Bytes>` en tests; el framework las recibe como `Incoming` después de la conversión por conexión de hyper.

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

## Haciendo aserciones sobre la respuesta

La respuesta que regresa de `handle_request` es un `hyper::Response<BoxBody<Bytes, Infallible>>`. Tres cosas que vas a leer de ella:

```rust
let (parts, body) = resp.into_parts();

// 1. Status.
assert_eq!(parts.status.as_u16(), 200);

// 2. Headers - case-insensitive lookup.
let location = parts.headers.get("location").and_then(|v| v.to_str().ok());
assert_eq!(location, Some("/login"));

// 3. Body - collect into bytes, then parse.
use http_body_util::BodyExt;
let bytes = body.collect().await.unwrap().to_bytes();

// As text:
let text = String::from_utf8_lossy(&bytes);

// As JSON:
let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
assert_eq!(value["message"], "ok");
```

Para respuestas de error, la forma del cuerpo que alcanza el renderizador
común se documenta en [Modelo de errores](error-model.md) - `message`,
`errors` opcional, `request_id` y `debug_message` opcional.
`request_id` es `null` fuera de un alcance de solicitud. Tres variantes
especiales devuelven la respuesta antes de inyectar el id de solicitud:
`PrecognitionSuccess` es un 204 sin cuerpo, `PrecognitionFailure` es el cuerpo
de validación más las cabeceras de Precognition, y un centinela
`AlreadyReported` renderizado accidentalmente como HTTP es una respuesta 500
genérica que contiene solo `message`.
Usa una respuesta de error ordinaria cuando compruebes que se ejecutó el
middleware de id de solicitud.

## Afirmaciones fluidas sobre respuestas con TestResponse

Construir la triple `(status, headers, body)` a mano y hacer aserciones sobre ella pieza por pieza, como arriba, es la base que cada harness en este crate usa. `suprnova::testing::TestResponse` envuelve esa misma triple en una API fluida, al estilo Laravel, así que un test lee como una aserción en lugar de una búsqueda de encabezado:

```rust
use suprnova::testing::TestResponse;

let (parts, body) = resp.into_parts();
let bytes = body.collect().await.unwrap().to_bytes();
let headers = parts.headers.iter().map(|(k, v)| {
    (k.as_str().to_string(), v.to_str().unwrap_or_default().to_string())
});

TestResponse::new(parts.status.as_u16(), headers, bytes)
    .assert_ok()
    .assert_header("content-type", "application/json")
    .assert_json(serde_json::json!({ "message": "ok" }));
```

`new()` acepta cualquier iterable como pares de encabezados `(String, String)` - un `HashMap<String, String>` (que varios harnesses existentes ya colectan), un `Vec<(String, String)>`, o `HeaderMap::iter()` mapeado a strings propios - así que ningún harness tiene que cambiar cómo conduce una solicitud.

Cada aserción devuelve `&Self`, así que se encadenan: `assert_status`, `assert_ok`, `assert_redirect(target: Option<&str>)`, `assert_json` (coincidencia de subconjunto - las claves extra en el cuerpo están bien), `assert_json_path` (notación de punto, un segmento numérico indexa un array), `assert_json_count`, `assert_see`, `assert_header`, `assert_cookie`. Los fallos de aserción entran en pánico con un fragmento esperado/actual, el mismo contrato que `expect!` ([Testing](testing.md)) - esta es una superficie de testing, no código de biblioteca, así que la regla de no-pánico de la casa no aplica.

### `assert_session_has` necesita un almacén de sesión

Toda otra aserción lee solo la respuesta serializada. `assert_session_has` no puede: el estado de sesión del lado del servidor vive en el `SessionStore`, no en la respuesta, y cuando una respuesta regresa sobre el socket de loopback no hay sesión en proceso a leer. Adjunta el mismo almacén que el `SessionMiddleware` de tu test fue construido con, más su nombre de cookie, y la aserción desencripta la cookie de sesión de la respuesta para encontrar la fila misma:

```rust
let response = TestResponse::new(status, headers, body)
    .with_session_store(middleware.store(), "suprnova_session");

response
    .assert_session_has("flash.success", serde_json::json!("Saved!"))
    .await;
```

Es la única aserción `async`, ya que es la única que hace I/O; aún devuelve `&Self`, así que `.await` se sienta en línea y la cadena continúa después.

### Por qué Suprnova diverge

El `TestResponse` de Laravel vive en el mismo proceso de PHP que la app bajo prueba, así que `assertSessionHas` lee `$this->session()` directamente - no hay límite de respuesta que cruzar. Los tests de Suprnova conducen una conexión hyper real, así que la sesión es exactamente tan opaca a la prueba como lo es a un navegador real: una cookie. `assert_session_has` gana esa honestidad de vuelta con un handle explícito de almacén en lugar de pretender que el atajo en proceso existe.

## Probar respuestas Inertia

`suprnova::testing::AssertableInertia` envuelve un objeto de página Inertia - ya sea que vino como un cuerpo JSON `X-Inertia` o embebido en un shell HTML de navegación dura - en el mismo estilo fluido y panic-on-failure que `TestResponse`. Equivalente a `Inertia\Testing\AssertableInertia` de Laravel.

Dos formas de obtener uno. Desde un `TestResponse` que ya pasó a través de una visita real `X-Inertia: true`:

```rust
use suprnova::testing::TestResponse;

let response = TestResponse::new(status, headers, body);
response
    .assert_inertia()
    .component("Users/Index")
    .url("/users")
    .has("users")
    .where_("users.0.name", "Ada")
    .count("users", 1)
    .missing("admin_only_field");
```

O directamente desde un `HttpResponse` - lo que `InertiaResponse::resolve` devuelve - para una prueba que conduce el pipeline de respuesta sin un socket. Este formulario maneja ambas formas: un cuerpo JSON `X-Inertia`, o el elemento `<script data-page="app">` embebido del shell HTML:

```rust
use suprnova::testing::AssertableInertia;

let response = InertiaResponse::new("Users/Index")
    .with("users", users_json)
    .resolve(&req)
    .await?;

AssertableInertia::from_response(&response)
    .component("Users/Index")
    .where_("users.0.name", "Ada");
```

`version()` comprueba la versión de asset de la página. El resolver predeterminado hashea el manifiesto de Vite y cae de vuelta a `MANIFEST_VERSION_FALLBACK` cuando no existe manifiesto - aserta contra esa constante en lugar de un `"1.0"` hardcodeado en una prueba que no ha construido un frontend:

```rust
use suprnova::MANIFEST_VERSION_FALLBACK;

response.assert_inertia().version(MANIFEST_VERSION_FALLBACK);
```

`has_flash(key, expected)` lee los datos flash de la página de la misma forma de ruta de punto que `has` / `where_` lee props - `expected` es un `Option`, así que pasa `None::<serde_json::Value>` para comprobar solo presencia:

```rust
response.assert_inertia().has_flash("toast.message", Some(serde_json::json!("Saved!")));
response.assert_inertia().has_flash("toast", None::<serde_json::Value>);
```

### Recargando para aserciones de recarga parcial y props diferidas

`reload_only`, `reload_except`, y `load_deferred_props` espejo lo que el cliente Inertia hace después de la visita inicial: reemite la misma página como una recarga parcial y comprueba qué volvió. Porque los tests HTTP de Suprnova cruzan un socket real y cada archivo de test posee su propio harness (ver [Dónde vive cada pieza](#dónde-vive-cada-pieza) abajo), estos métodos no llevan transporte incorporado - adjunta uno con `with_reload`, un closure que recibe un `ReloadRequest` y debe devolver un futuro con el `AssertableInertia` recargado:

```rust
use suprnova::testing::TestResponse;

let assertable = TestResponse::new(status, headers, body)
    .assert_inertia()
    .with_reload(move |reload| {
        async move {
            let header_pairs = reload.headers();
            let headers: Vec<(&str, &str)> = header_pairs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let (status, headers, body) = request(addr, "GET", &reload.url, &headers).await;
            TestResponse::new(status, headers, body).assert_inertia()
        }
    });

// Requests only `users`, and asserts the reload landed on the same
// component/url/version and that `users` came back.
assertable.reload_only(["users"]).await;

// Requests everything except `stats`, and asserts `stats` is absent.
assertable.reload_except(["stats"]).await;

// Reads `deferredProps` off the original page, requests every deferred
// key in one partial reload, and asserts they all came back.
assertable.load_deferred_props().await;
```

Llamar a cualquiera sin `with_reload` primero entra en pánico con una instrucción. El resultado conserva el reloader para la siguiente recarga.

### Por qué Suprnova diverge

El `ReloadRequest` de Laravel reemite la solicitud a través del mismo kernel PHP en proceso que el test original utilizó - un cliente de test, siempre disponible. Los tests HTTP de Suprnova conducen un loopback hyper/TCP real y cada archivo de test define su propio par `spawn_server` / `request` (ver [Dónde vive cada pieza](#dónde-vive-cada-pieza) abajo), así que no hay un cliente único al que `AssertableInertia` podría alcanzar - `with_reload` hace que sea explícito en lugar de hardcodificar un harness que un archivo de test de forma diferente no podría usar. `component()` también omite la comprobación de existencia de archivo de componente de página de Laravel (`view-finder`) - un componente alcanzado a través de `Router::inertia` o un `InertiaResponse::new(name)` manual es un string de tiempo de ejecución sin archivo a comprobar; el equivalente de tiempo de compilación de Suprnova es la macro `inertia_response!` (ver [Inertia Responses](frontend-inertia-responses.md)). Sus nombres de método también divergen del `TestResponse`: `component`, `has`, `missing`, `where_`, `count`, y `has_flash` descartan el prefijo `assert_` completamente, emparejando el `Inertia\Testing\AssertableInertia` de Laravel, cuyos métodos equivalentes son desnudos del mismo modo - el contrato panic-on-failure es idéntico de cualquier forma, sin la pista visual `assert_`.

## Probar middleware

Los tests de middleware se ven idénticos a los tests de ruta; la única diferencia es lo que le añades con `.append()` al registry antes de lanzar el servidor.

### Probar middleware global

Pasa el middleware a `MiddlewareRegistry::new().append(...)` y usa ese registry - varios middlewares se ejecutan en el orden en que se añadieron, `prepend` pone uno nuevo al frente.

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

Este test demuestra más que la lógica de CORS en sí misma: demuestra que el middleware global también se ejecuta sobre solicitudes **no enrutadas**, que es el contrato que garantiza el framework (de otro modo, un preflight OPTIONS que nunca coincide con una ruta se saltaría CORS). Consulta `framework/tests/cors_middleware.rs` para la suite completa.

### Probar middleware específico de ruta

Adjúntalo con `.middleware(...)` sobre el builder de la ruta, exactamente como en producción. Luego prueba la ruta con normalidad - la cadena de middleware se construye a partir del mismo registro.

```rust
let router = Router::new()
    .get("/admin/dashboard", |_req| async { text("admin") })
    .middleware(RequireRole::new("admin"));

let (status, _) = send_get(addr, "/admin/dashboard").await;
assert_eq!(status, 403); // unauthenticated request
```

### Preestablecer el usuario autenticado

Los tests de flujo de autenticación reales necesitan un usuario ya conectado. El patrón más limpio es un pequeño middleware puntual que llama a `Auth::set_user` antes del middleware bajo prueba. El propio `framework/tests/email_verified_middleware.rs` del framework usa esto:

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

`LoginAs` se ejecuta primero, instala el usuario en el estado de auth por solicitud, y el middleware bajo prueba ve `Auth::id() == Some(...)` sin llegar a emitir nunca un login real. El alcance del estado de auth lo monta el propio `handle_request` - el mismo que se ejecuta en producción - así que el usuario es visible para todo middleware posterior y para el handler.

## Probar la vinculación de modelo de ruta

`RouteParam<User>` hidrata un `User` tipado mediante la cadena de extractores
del handler, así que el test debe pasar ese extractor a una función
`#[handler]`:

```rust
use suprnova::{RouteParam, Response, handler};

#[suprnova::model(table = "users")]
pub struct User {
    pub id: i64,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[handler]
async fn show(RouteParam(user): RouteParam<User>) -> Response {
    suprnova::http::json(serde_json::json!({ "email": user.email }))
}

#[tokio::test]
async fn show_user_binds_from_route_param() {
    // Insert a test user via the model. Database setup omitted -
    // see the testing chapter for `TestDatabase` patterns.
    let user = User::create(suprnova::attrs! {
        email: "bound@example.com"
    }).await.unwrap();

    // A destructured RouteParam currently uses `param` as the handler
    // macro's route-parameter name.
    let router: Router = Router::new()
        .get("/users/{param}", show)
        .into();

    let addr = spawn_server(router, MiddlewareRegistry::new(), 1).await;
    let (status, body) = send_get(addr, &format!("/users/{}", user.id)).await;

    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["email"], "bound@example.com");
}
```

Para un parámetro de ruta `{user}`, acepta en su lugar
`user: RouteParam<User>` sin destructurar; `RouteParam` hace deref a `User`
para acceder a los campos. Llamar a `req.param(...).parse()` y después a
`User::find_or_fail(...)` prueba el análisis del parámetro y la búsqueda del
modelo, no la vinculación de modelo de ruta.

Para tests de vinculación en aislamiento, llama directamente a
`<RouteParam<User> as AutoRouteBinding>::from_route_param(...)`. Eso comprueba
la implementación de vinculación sin un router, pero no ejercita la cadena de
extractores de `#[handler]`.
## Probar flujos de autenticación de punta a punta

Para probar una sesión de login de punta a punta, pasa al servidor loopback un
registry que contenga `SessionMiddleware` y protege `/dashboard` con
`AuthMiddleware` o con el middleware de autenticación web de la aplicación.
Primero demuestra que la ruta rechaza una solicitud sin cookie; después inicia
sesión, reenvía la cookie de sesión devuelta y demuestra que la ruta protegida
tiene éxito:

```rust
#[tokio::test]
async fn login_flow_issues_session_cookie() {
    // 1. Bootstrap: create the user.
    Auth::password()
        .register("alice@example.com", "longpassword123")
        .await.expect("register");

    // 2. Mount a protected route and the stateful session middleware.
    let router: Router = Router::new()
        .post("/login", login_handler)
        .get("/dashboard", |_req: Request| async { text("dashboard") })
        .middleware(AuthMiddleware::new())
        .into();
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::new(SessionConfig::from_env()));
    let addr = spawn_server(router, registry, 3).await;

    // 3. Prove the route is protected before authenticating.
    let (guest_status, _) = send_get(addr, "/dashboard").await;
    assert_eq!(guest_status, 401);

    // 4. Drive login and capture the Set-Cookie header.
    let login = post_json(addr, "/login", serde_json::json!({
        "email": "alice@example.com",
        "password": "longpassword123",
    })).await;
    assert_eq!(login.status, 200);
    let cookie = extract_session_cookie(&login.headers);

    // 5. Replay the cookie against the protected route.
    let (status, body) = get_with_cookie(addr, "/dashboard", &cookie).await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"dashboard");
}
```

El router abreviado sin esos middleware solo demuestra la plomería de
cookies; no es un test de flujo de autenticación.
`framework/tests/auth_http_middleware.rs` prueba el comportamiento del
middleware de autenticación con registries explícitos, pero no instala un
`SessionMiddleware` real. Un test de flujo de login con estado debe instalar
tanto el middleware de sesión como la compuerta de autenticación, como se
muestra arriba.


## Probar el límite de pánico

Un pánico dentro de un handler no debe choquear el servidor. El envoltorio de recuperación de pánico (`execute_chain_safely`) lo atrapa y convierte en un 500 a través del mismo camino que los errores devueltos fluyen. Puedes verificar esto sin ninguna infraestructura de test especial - establece `accepts >= 2` para que el listener sobreviva al pánico:

```rust
#[tokio::test]
async fn panicking_handler_yields_500_and_server_survives() {
    let router = Router::new()
        .get("/panic", |_req: Request| async {
            panic!("intentional test panic");
            #[allow(unreachable_code)] text("unreachable")
        })
        .get("/ok", |_req: Request| async { text("ok") });

    let addr = spawn_server(router.into(), MiddlewareRegistry::new(), 4).await;

    // First: the panic translates to a sanitised 500.
    let (s1, body) = send_get(addr, "/panic").await;
    assert_eq!(s1, 500);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["message"], "Internal Server Error");
    assert!(parsed.get("request_id").is_some());

    // Second: the server survives. The next request is normal.
    let (s2, body2) = send_get(addr, "/ok").await;
    assert_eq!(s2, 200);
    assert_eq!(&body2[..], b"ok");
}
```

## Probar accesores sin pasar por enrutamiento

A veces quieres probar un accesor de `Request` (`bearer_token`, `is_method`, `ip`, `is_json`, etc.) sin girar un router en absoluto. El truco es un harness diminuto que ejecuta un servicio hyper cuyo único trabajo es construir el `Request` y devolverlo a través de un `tokio::sync::oneshot::channel`:

```rust
let (req_tx, req_rx) = tokio::sync::oneshot::channel::<suprnova::Request>();
// ... loopback hyper service whose service_fn does:
//     let req = suprnova::Request::new(hyper_req);
//     let _  = req_tx.send(req);
//     return a 200 with an empty body
let req = req_rx.await.unwrap();
```

`framework/tests/http_request_accessors.rs` tiene el ayudante completo `build_request(builder, body) -> Request`. Cópialo una vez por crate y cada test de accesor lee limpiamente:

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

El Request es real (producido por hyper desde un intercambio HTTP real), pero no se ejecutó enrutamiento ni middleware - exactamente lo que quieres cuando la unidad bajo prueba es el accesor mismo.

## Builder hooks on `Request`

Cuando tienes un `Request` en mano y necesitas falsificar una pieza del layer de enrutamiento, tres métodos de builder ayudan:

```rust
impl Request {
    pub fn with_params(mut self, params: HashMap<String, String>) -> Self;
    pub fn with_route_pattern(mut self, pattern: String) -> Self;
    pub fn with_peer_addr(mut self, addr: std::net::IpAddr) -> Self;
}
```

Estos son los mismos métodos que el servidor llama cuando despacha una ruta coincidida - `Router` llama a `with_params` después de que `matchit` devuelve, `with_route_pattern` para que `req.route_pattern()` se resuelva, y `with_peer_addr` una vez que conoce la IP del socket TCP aceptado. En tests los llamas tú mismo para cortocircuitar la misma configuración.

```rust
let req = Request::new(hyper_req)
    .with_params(HashMap::from([("id".into(), "42".into())]))
    .with_route_pattern("/users/{id}".into())
    .with_peer_addr("192.168.1.10".parse().unwrap());

assert_eq!(req.param("id").unwrap(), "42");
assert_eq!(req.ip(), Some("192.168.1.10".parse().unwrap()));
```

## Cosas que conviene saber

Una lista corta de trampas que atrapan a los autores primerizos:

- **`Incoming` es solo del lado del servidor.** No puedes construir uno en tu test. El loopback TCP (o captura de servicio dentro del proceso) es el único camino - no hay un constructor "construye un `Request` desde un `Vec<u8>` body".
- **No compartas estado entre tests.** Cada `#[tokio::test]` obtiene su propio runtime; la contaminación cruzada de tests suele significar que estás compartiendo un global (`once_cell`, `lazy_static`, variable de entorno). Para estado de BD ver `TestDatabase` en [Testing](testing.md).
- **Las cookies necesitan un cliente real.** Sin jar de cookies automático - hila `Set-Cookie` de una respuesta dentro de `Cookie` en la siguiente. Ver `framework/tests/auth_http_middleware.rs` para el patrón.
- **El spawn de terminación posterior a la respuesta no bloquea.** Si quieres hacer aserciones sobre efectos secundarios que se ejecutan vía `Terminable`, sondea por ellos - la respuesta regresa al cliente antes de que el hook se ejecute.

## Dónde vive cada pieza

| Pieza | Archivo |
|---|---|
| `handle_request`, `handle_request_with_peer` | `framework/src/server.rs` |
| `Request::new`, `with_params`, `with_route_pattern`, `with_peer_addr` | `framework/src/http/request.rs` |
| `MiddlewareRegistry::new`, `append`, `prepend` | `framework/src/middleware/registry.rs` |
| Harness de test por loopback (canónico) | `framework/tests/cors_middleware.rs` |
| `TestResponse` (aserciones fluidas sobre la triple) | `framework/src/testing/response.rs` |
| `AssertableInertia`, `ReloadRequest` (aserciones de page-object Inertia fluidas) | `framework/src/testing/inertia.rs` |
| Harness de captura de `Request` dentro del proceso | `framework/tests/http_request_accessors.rs` |
| Patrón de test del límite de pánico | `framework/tests/middleware_panic_safety.rs` |
| Patrón de punta a punta de auth + middleware | `framework/tests/email_verified_middleware.rs` |

## Siguiente

- [Pruebas](testing.md) - `#[suprnova_test]`, `TestDatabase`, las macros `describe!`/`test!`/`expect!`, y la superficie a nivel de unidad
- [Modelo de errores](error-model.md) - la forma JSON que usa cada respuesta de error, la regla de sanitización de los 5xx, y qué significa `request_id` en el cuerpo de un test
- [Middleware](middleware.md) - escribir el middleware que pruebas aquí, y el ciclo de vida global frente a por ruta
- [Enrutamiento](routing.md) - el `Router` que montas tanto en producción como en los tests, los parámetros de ruta, los nombres de ruta, las URLs firmadas
- [Autenticación](authentication.md) - la fachada `Auth`, `Authenticatable`, los guards, y cómo `Auth::set_user` interactúa con el alcance de solicitud que instala `handle_request`

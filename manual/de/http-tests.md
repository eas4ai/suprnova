# HTTP-Tests

Dieses Kapitel zeigt, wie Sie Ihre HTTP-Oberfläche testen - Routen,
Middleware, Auth-Flows, Fehler-Responses - indem Sie die
Request-Pipeline des Frameworks über `suprnova::handle_request`
treiben. Wenn Sie Laravel-Feature-Tests mit `$this->get('/users')`
geschrieben und auf `$response->status()` assertiert haben, ist das
das Suprnova-Äquivalent: Derselbe `Router`, den Sie in Produktion
mounten, läuft im Test, jede Middleware feuert, die Panic-Grenze
fängt weiterhin ab, und die Response ist byte-für-byte, was ein
echter Client sieht.

## Die Test-Oberfläche

Es gibt genau drei Bausteine:

| Baustein | Rolle |
|---|---|
| `Router` | Die zu testenden Routen - genauso gebaut wie in Produktion |
| `MiddlewareRegistry` | Der globale Middleware-Stack - ebenfalls genauso gebaut |
| `handle_request(router, registry, req) -> hyper::Response<…>` | Der In-Process-Treiber - führt eine Anfrage End-to-End aus |

`handle_request` ist dieselbe Funktion, die `Server::run` pro
Anfrage aufruft, exponiert für Tests und Embedder. Alles, was in
Produktion funktioniert, funktioniert hier - der
Panic-Recovery-Wrapper, der Request-ID-Scope, der
Inertia-Flash-Bag-Scope, der Auth-Request-State-Scope, das
HEAD-Body-Strip, die Post-Response-Termination. Es gibt keinen
"Test-Modus", der eine leisere Pipeline einwechselt.

`handle_request_with_peer` ist derselbe Aufruf mit einem expliziten
`Option<std::net::IpAddr>` für den verbindenden Peer - nützlich,
wenn Sie auf die Auflösung von `Request::ip()` assertieren wollen,
ohne Proxy-Header aufzusetzen.

## Das hyper-Body-Problem

Die eine Komplikation, die Sie vorab kennen sollten:
`handle_request` nimmt einen
`hyper::Request<hyper::body::Incoming>` entgegen. `Incoming` ist
hypers interner Streaming-Body-Typ; Sie können keinen mit
`Full::new(bytes)` oder irgendeinem der In-Memory-Body-Typen bauen.
Er kommt nur aus einer hyper-Connection heraus.

Es gibt zwei saubere Wege darum herum:

1. **TCP-Loopback** - binden Sie einen `127.0.0.1:0`-Listener,
   bedienen Sie einen Accept innerhalb einer `service_fn`, senden
   Sie die Anfrage über einen hyper-Client, und lassen Sie
   `Incoming` auf der Serverseite natürlich entstehen. Das tut jeder
   Integrationstest im Framework bereits.
2. **In-Process-Request-Bau** - für Tests, die nur `Request`-
   Accessoren (Header, Route-Params, IP, JSON-Parsing) inspizieren
   müssen, ohne durch das Routing zu gehen, verwenden Sie dasselbe
   TCP-Loopback-Capture-Muster, aber mit einem Service, der die
   `Request` in einen `oneshot::channel` herauszieht, statt sie
   auszuführen. Die Datei
   `framework/tests/http_request_accessors.rs` hat diesen
   `build_request()`-Helfer wortwörtlich.

Beide Muster erzeugen echte `Incoming`-Bodys. Der Loopback ist
lokal, in Test-Wall-Clock-Begriffen synchron (Mikrosekunden), und
berührt das Netzwerk außerhalb von `lo` nie. Es gibt keinen
langsameren oder einfacheren Weg, der den Vertrag bewahrt.

### Warum Suprnova abweicht

Laravels `$this->get('/users')` funktioniert, weil PHPs
Request-Lifecycle lautet: "Ein `Request`-Objekt bauen, es durch den
Kernel dispatchen". Der Kernel nimmt das In-Memory-Objekt direkt
entgegen; es gibt keinen Body-Typ, der einen Transport erzwingt.
Suprnovas Server ist auf hyper gebaut, und hypers Body-Typ ist aus
guten Gründen eigenwillig (Streaming, Backpressure, Zero-Copy). Die
Test-Oberfläche erbt diese Einschränkung.

Was Sie für die Einschränkung eintauschen, ist Treue zum Original.
Jedes Detail des Produktions-Request-Pfads - Header-Parsing,
Body-Grenzen, Connection-Upgrades - läuft in Tests auf dieselbe
Weise. Nie wird ein Test bestehen, weil der Test-Harness eine
Schicht übersprungen hat, die der echte Server ausführt.

## Ein erster End-to-End-Test

Hier ist ein vollständiger, lauffähiger Test, der eine einzelne
Route mountet, ein GET dagegen sendet und auf Status und Body
assertiert.

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

Das ist die gesamte Form. Kopieren Sie die zwei Helfer pro Crate,
stimmen Sie sie auf die Suite ab (mehrere Accepts,
Header-Erfassung, Body-Erfassung). Das Framework selbst verwendet
nahezu identische Helfer in `framework/tests/cors_middleware.rs`,
`framework/tests/middleware_panic_safety.rs` und
`framework/tests/email_verified_middleware.rs`.

Das Argument `accepts` begrenzt, wie viele Verbindungen die
Accept-Schleife bedient, bevor sie beendet. Eins reicht für eine
einzelne Anfrage; erhöhen Sie es auf zwei oder mehr, wenn ein Test
die Post-Panic-Recovery ausübt (siehe
[Die Panic-Grenze testen](#die-panic-grenze-testen)).

## Eine Anfrage bauen

Innerhalb von `send_get` haben Sie gesehen:

```rust
let req = hyper::Request::builder()
    .method("GET")
    .uri("/users/42")
    .header("Host", "localhost")
    .header("Content-Length", "0")
    .body(Full::new(Bytes::new()))
    .unwrap();
```

Das ist die kanonische Form. Ein paar Dinge, die man wissen sollte:

- **`Host`-Header**. Hyper lehnt HTTP/1.1-Anfragen ohne ihn ab.
  Fügen Sie ihn immer hinzu; der Wert spielt keine Rolle, außer Ihr
  Handler schlüsselt darauf.
- **`Content-Length: 0`**. Passend zum Body. Hyper berechnet das
  für Sie mit `Full::new(Bytes::new())`, aber explizit zu sein
  liest sich in Tests sauberer.
- **Body-Typen**. Die Client-Seite sendet `Full<Bytes>`. Die
  Server-Seite empfängt `Incoming`. Sie bauen in Tests immer nur
  `Full<Bytes>`-Anfragen; das Framework empfängt sie als `Incoming`
  nach hypers Pro-Connection-Konvertierung.

Ein POST mit einem JSON-Body:

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

## Auf die Response assertieren

Die Response, die von `handle_request` zurückkommt, ist eine
`hyper::Response<BoxBody<Bytes, Infallible>>`. Drei Dinge, die Sie
davon ablesen werden:

```rust
let (parts, body) = resp.into_parts();

// 1. Status.
assert_eq!(parts.status.as_u16(), 200);

// 2. Header - case-insensitive Lookup.
let location = parts.headers.get("location").and_then(|v| v.to_str().ok());
assert_eq!(location, Some("/login"));

// 3. Body - in Bytes sammeln, dann parsen.
use http_body_util::BodyExt;
let bytes = body.collect().await.unwrap().to_bytes();

// Als Text:
let text = String::from_utf8_lossy(&bytes);

// Als JSON:
let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
assert_eq!(value["message"], "ok");
```

Für Fehler-Responses ist die Body-Form fest und in
[Fehlermodell](error-model.md) dokumentiert - `message`, `errors`,
`request_id` und ein optionales `debug_message`. Der Schlüssel
`request_id` ist immer vorhanden (kann außerhalb eines
Request-Scopes `null` sein), und genau darauf assertieren Sie, wenn
Sie prüfen, ob die Request-ID-Middleware gelaufen ist.

## Middleware testen

Middleware-Tests sehen aus wie Routen-Tests; der einzige Unterschied
ist, was Sie vor dem Spawnen mit `.append()` an die Registry
anhängen.

### Globale Middleware testen

Übergeben Sie die Middleware an
`MiddlewareRegistry::new().append(...)` und verwenden Sie diese
Registry - mehrere Middlewares laufen in Append-Reihenfolge,
`prepend` setzt eine neue an den Anfang.

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
    // Die 3-Arg-Form von `spawn_server` lässt Sie eine nicht leere
// MiddlewareRegistry verdrahten - kopieren Sie den Helfer aus
// framework/tests/cors_middleware.rs (er hat ~30 Zeilen).
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

Dieser Test beweist mehr als nur die CORS-Logik selbst: Er beweist,
dass globale Middleware auch auf **ungeroutete** Anfragen läuft, was
der Vertrag ist, den das Framework garantiert (sonst würde ein
OPTIONS-Preflight, der nie eine Route matcht, CORS überspringen).
Siehe `framework/tests/cors_middleware.rs` für die vollständige
Suite.

### Routenspezifische Middleware testen

Hängen Sie sie mit `.middleware(...)` am Route-Builder an, genau wie
in Produktion. Testen Sie die Route dann normal - die
Middleware-Chain wird von derselben Registrierung aus gebaut.

```rust
let router = Router::new()
    .get("/admin/dashboard", |_req| async { text("admin") })
    .middleware(RequireRole::new("admin"));

let (status, _) = send_get(addr, "/admin/dashboard").await;
assert_eq!(status, 403); // nicht authentifizierte Anfrage
```

### Den authentifizierten Benutzer stubben

Echte Auth-Flow-Tests brauchen einen angemeldeten Benutzer. Das
sauberste Muster ist eine winzige Einweg-Middleware, die
`Auth::set_user` vor der zu testenden Middleware aufruft. Das
Framework selbst verwendet das in
`framework/tests/email_verified_middleware.rs`:

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

Dann im Test:

```rust
let registry = MiddlewareRegistry::new()
    .append(LoginAs("user-id-123".to_string()))
    .append(EnsureEmailVerifiedMiddleware::new());
```

`LoginAs` läuft zuerst, installiert den Benutzer in den
Pro-Request-Auth-State, und die zu testende Middleware sieht
`Auth::id() == Some(...)`, ohne je einen echten Login auszulösen.
Der Auth-State-Scope wird von `handle_request` selbst aufgesetzt -
demselben, der in Produktion läuft -, sodass der Benutzer für jede
spätere Middleware und den Handler sichtbar ist.

## Route-Model-Binding testen

Route-Model-Binding verwandelt `/users/{id}` in ein typisiertes
`User`-Argument. Das Binding läuft als Teil der Extractor-Chain des
Handlers, ein normaler End-to-End-Test übt es also kostenlos aus:

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
    // Einen Test-Benutzer über das Model einfügen. DB-Setup
    // ausgelassen - siehe das Testen-Kapitel für
    // `TestDatabase`-Muster.
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

Für Binding-in-Isolation-Tests - kein Router, kein TCP-Loop -
synthetisieren Sie die Route-Params selbst mit
`Request::with_params(...)` (siehe
[Builder-Hooks auf `Request`](#builder-hooks-auf-request) unten).
Das ist das Muster, das `framework/tests/data_route_params.rs` zum
Testen von `#[derive(Data)]`-Extraktoren gegen synthetisierte Params
verwendet.

## Auth-Flows End-to-End testen

Ein echter Auth-Flow-Test registriert einen Benutzer, treibt die
Login-Route, zieht das Session-Cookie von der Response und sendet es
erneut auf einer geschützten Route. Vier Schritte, alle auf
Wire-Ebene:

```rust
#[tokio::test]
async fn login_flow_issues_session_cookie() {
    // 1. Bootstrap: den Benutzer erzeugen.
    Auth::password()
        .register("alice@example.com", "longpassword123")
        .await.expect("register");

    // 2. Die Routen mounten.
    let router = Router::new()
        .post("/login", login_handler)
        .get("/dashboard", |_req: Request| async { text("dashboard") });
    let addr = spawn_server(router, 2).await;

    // 3. Login treiben; den Set-Cookie-Header erfassen.
    let login = post_json(addr, "/login", serde_json::json!({
        "email": "alice@example.com",
        "password": "longpassword123",
    })).await;
    assert_eq!(login.status, 200);
    let cookie = extract_session_cookie(&login.headers);

    // 4. Das Cookie erneut gegen die geschützte Route senden.
    let (status, body) = get_with_cookie(addr, "/dashboard", &cookie).await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"dashboard");
}
```

`extract_session_cookie` und `get_with_cookie` sind unkomplizierte
Header-und-Cookie-Verdrahtung - `framework/tests/auth_http_middleware.rs`
hat eine vollständige Implementierung. Der Punkt: Der gesamte Flow
läuft durch die echte `SessionMiddleware`, den echten `Auth`-Guard,
die echte `Authenticatable`-Auflösung. Der Test verifiziert den
Wire-Vertrag, nicht ein Mock davon.

## Die Panic-Grenze testen

Ein Panic innerhalb eines Handlers darf den Server nicht zum Absturz
bringen. Der Panic-Recovery-Wrapper (`execute_chain_safely`) fängt
ihn ab und wandelt ihn über denselben Pfad, den zurückgegebene
Fehler durchlaufen, in ein 500 um. Sie können das ohne besondere
Test-Infrastruktur verifizieren - setzen Sie `accepts >= 2`, damit
der Listener den Panic überlebt:

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

    // Erstens: Der Panic übersetzt sich in ein bereinigtes 500.
    let (s1, body) = send_get(addr, "/panic").await;
    assert_eq!(s1, 500);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["message"], "Internal Server Error");
    assert!(parsed.get("request_id").is_some());

    // Zweitens: Der Listener überlebt. Die nächste Anfrage ist normal.
    let (s2, body2) = send_get(addr, "/ok").await;
    assert_eq!(s2, 200);
    assert_eq!(&body2[..], b"ok");
}
```

## Accessoren testen, ohne durch das Routing zu gehen

Manchmal wollen Sie einen `Request`-Accessor (`bearer_token`,
`is_method`, `ip`, `is_json` usw.) testen, ohne überhaupt einen
Router hochzuziehen. Der Trick ist ein winziger Harness, der einen
hyper-Service ausführt, dessen einzige Aufgabe es ist, die `Request`
zu konstruieren und sie über einen `tokio::sync::oneshot::channel`
zurückzuschicken:

```rust
let (req_tx, req_rx) = tokio::sync::oneshot::channel::<suprnova::Request>();
// ... Loopback-hyper-Service, dessen service_fn Folgendes tut:
//     let req = suprnova::Request::new(hyper_req);
//     let _  = req_tx.send(req);
//     einen 200 mit leerem Body zurückgeben
let req = req_rx.await.unwrap();
```

`framework/tests/http_request_accessors.rs` hat den vollständigen
`build_request(builder, body) -> Request`-Helfer. Kopieren Sie ihn
einmal pro Crate, und jeder Accessor-Test liest sich sauber:

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

Die Request ist echt (von hyper aus einem echten Wire-Austausch
produziert), aber kein Routing oder Middleware ist gelaufen - genau
das, was Sie wollen, wenn die zu testende Einheit der Accessor
selbst ist.

## Builder-Hooks auf `Request`

Wenn Sie eine `Request` in der Hand haben und ein Stück der
Routing-Ebene faken müssen, helfen drei Builder-Methoden:

```rust
impl Request {
    pub fn with_params(mut self, params: HashMap<String, String>) -> Self;
    pub fn with_route_pattern(mut self, pattern: String) -> Self;
    pub fn with_peer_addr(mut self, addr: std::net::IpAddr) -> Self;
}
```

Das sind dieselben Methoden, die der Server aufruft, wenn er eine
gematchte Route dispatcht - `Router` ruft `with_params` auf,
nachdem `matchit` zurückkehrt, `with_route_pattern`, damit
`req.route_pattern()` sich auflöst, und `with_peer_addr`, sobald er
die IP des akzeptierten TCP-Sockets kennt. In Tests rufen Sie sie
selbst auf, um dasselbe Setup per Short-Circuit abzukürzen.

```rust
let req = Request::new(hyper_req)
    .with_params(HashMap::from([("id".into(), "42".into())]))
    .with_route_pattern("/users/{id}".into())
    .with_peer_addr("192.168.1.10".parse().unwrap());

assert_eq!(req.param("id").unwrap(), "42");
assert_eq!(req.ip(), Some("192.168.1.10".parse().unwrap()));
```

## Was Sie wissen sollten

Eine kurze Liste von Fallen, die Erstautoren erwischen:

- **`Incoming` ist nur serverseitig.** Sie können in Ihrem Test
  keins bauen. Der TCP-Loopback (oder die
  In-Process-Service-Erfassung) ist der einzige Weg - es gibt
  keinen "eine `Request` aus einem `Vec<u8>`-Body bauen"-Konstruktor.
- **Teilen Sie keinen Zustand zwischen Tests.** Jedes
  `#[tokio::test]` bekommt seine eigene Runtime; Cross-Test-
  Verschmutzung bedeutet meist, dass Sie ein Global teilen
  (`once_cell`, `lazy_static`, Env-Variable). Für DB-Zustand siehe
  `TestDatabase` in [Testen](testing.md).
- **Cookies brauchen einen echten Client.** Kein automatisches
  Cookie-Jar - fädeln Sie `Set-Cookie` aus einer Response in
  `Cookie` auf der nächsten. Siehe
  `framework/tests/auth_http_middleware.rs` für das Muster.
- **Der Post-Response-Termination-Spawn ist non-blocking.** Wenn
  Sie auf Seiteneffekte assertieren wollen, die über `Terminable`
  laufen, pollen Sie danach - die Response geht an den Client
  zurück, bevor der Hook läuft.

## Wo jedes Teil lebt

| Teil | Datei |
|---|---|
| `handle_request`, `handle_request_with_peer` | `framework/src/server.rs` |
| `Request::new`, `with_params`, `with_route_pattern`, `with_peer_addr` | `framework/src/http/request.rs` |
| `MiddlewareRegistry::new`, `append`, `prepend` | `framework/src/middleware/registry.rs` |
| Loopback-Test-Harness (kanonisch) | `framework/tests/cors_middleware.rs` |
| In-Process-`Request`-Erfassungs-Harness | `framework/tests/http_request_accessors.rs` |
| Panic-Grenze-Testmuster | `framework/tests/middleware_panic_safety.rs` |
| Auth + Middleware End-to-End-Muster | `framework/tests/email_verified_middleware.rs` |

## Nächste Schritte

- [Testen](testing.md) - `#[suprnova_test]`, `TestDatabase`, die
  Makros `describe!`/`test!`/`expect!` und die Oberfläche auf
  Unit-Ebene
- [Fehlermodell](error-model.md) - die JSON-Form, die jede
  Fehler-Response verwendet, die 5xx-Bereinigungsregel und was
  `request_id` in einem Test-Body bedeutet
- [Middleware](middleware.md) - das Schreiben der Middleware, die
  Sie hier testen, und der Global-vs-Route-Lifecycle
- [Routing](routing.md) - der `Router`, den Sie sowohl in Produktion
  als auch in Tests mounten, Route-Params, Routennamen, signierte
  URLs
- [Authentifizierung](authentication.md) - die `Auth`-Facade,
  `Authenticatable`, Guards, und wie `Auth::set_user` mit dem
  Request-Scope interagiert, den `handle_request` installiert

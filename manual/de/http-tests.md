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

Für gewöhnliche Fehler-Responses, die den gemeinsamen Renderer erreichen,
enthält die in [Fehlermodell](error-model.md) dokumentierte Body-Form
`message`, optionale `errors`, `request_id` und optionales `debug_message`.
`request_id` ist außerhalb eines Request-Scopes `null`. Drei spezielle
Varianten kehren vor der Request-ID-Injektion zurück:
`PrecognitionSuccess` ist eine bodylose 204-Response,
`PrecognitionFailure` ist der Validierungs-Body plus Precognition-Header, und
ein versehentlich HTTP-gerendertes `AlreadyReported`-Sentinel ist eine
generische 500-Response, die nur `message` enthält. Verwenden Sie eine
gewöhnliche Fehler-Response, wenn Sie prüfen, ob die Request-ID-Middleware
gelaufen ist.

## Fluente Response-Assertions mit TestResponse

Das Triple `(status, headers, body)` wie oben von Hand aufzubauen und stückweise darauf zu prüfen, ist die Grundlage jedes Harness in dieser Crate. `suprnova::testing::TestResponse` kapselt dasselbe Triple in einer flüssigen, an Laravel angelehnten API, sodass ein Test wie eine Assertion statt wie ein Header-Lookup gelesen wird:

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

`new()` akzeptiert jedes Iterable aus Header-Paaren `(String, String)` – eine `HashMap<String, String>` (in die mehrere bestehende Harnesses bereits sammeln), ein `Vec<(String, String)>` oder `HeaderMap::iter()`, das auf eigene Strings abgebildet ist. Kein Harness muss daher ändern, wie er einen Request ausführt.

Jede Assertion gibt `&Self` zurück und lässt sich deshalb verketten: `assert_status`, `assert_ok`, `assert_redirect(target: Option<&str>)`, `assert_json` (Teilmenge wird abgeglichen – zusätzliche Schlüssel im Body sind in Ordnung), `assert_json_path` (Punktnotation; ein numerisches Segment indexiert ein Array), `assert_json_count`, `assert_see`, `assert_header`, `assert_cookie`. Fehler bei Assertions lösen mit einem Ausschnitt von Erwartetem und Tatsächlichem einen Panic aus – derselbe Vertrag wie bei `expect!` ([Testen](testing.md)). Das ist eine Testoberfläche, kein Bibliothekscode; die No-Panic-Hausregel gilt daher nicht.

### `assert_session_has` benötigt einen Session-Store

Jede andere Assertion liest nur die Response auf Wire-Ebene. `assert_session_has` kann das nicht: serverseitiger Session-Zustand liegt im `SessionStore`, nicht in der Response, und sobald eine Response über den Loopback-Socket zurückkommt, gibt es keine In-Process-Session mehr zu lesen. Hängen Sie denselben Store an, mit dem die `SessionMiddleware` des Tests gebaut wurde, sowie dessen Cookie-Namen; dann entschlüsselt die Assertion das Session-Cookie der Response, um die Zeile selbst zu finden:

```rust
let response = TestResponse::new(status, headers, body)
    .with_session_store(middleware.store(), "suprnova_session");

response
    .assert_session_has("flash.success", serde_json::json!("Saved!"))
    .await;
```

Dies ist die einzige `async`-Assertion, weil sie als einzige I/O ausführt; sie gibt weiterhin `&Self` zurück, sodass `.await` inline steht und die Kette danach fortgesetzt wird.

### Warum Suprnova abweicht

Laravels `TestResponse` lebt im selben PHP-Prozess wie die getestete Anwendung; `assertSessionHas` liest daher `$this->session()` direkt – keine Wire-Grenze muss überschritten werden. Suprnovas Tests steuern eine echte Hyper-Verbindung an, sodass die Session für den Test ebenso opak ist wie für einen echten Browser: ein Cookie. `assert_session_has` gewinnt diese Ehrlichkeit mit einem expliziten Store-Handle zurück, statt vorzutäuschen, dass die In-Process-Abkürzung existiert.

## Inertia-Responses testen

`suprnova::testing::AssertableInertia` kapselt ein Inertia-Seitenobjekt – unabhängig davon, ob es als JSON-Body mit `X-Inertia` zurückkam oder in einer HTML-Shell für Hard-Navigation eingebettet ist – im selben flüssigen Stil mit Panic bei Fehlschlag wie `TestResponse`. Es ist das Gegenstück zu Laravels `Inertia\Testing\AssertableInertia`.

Es gibt zwei Wege, eines zu erhalten. Der erste führt über eine `TestResponse`, die bereits einen echten Besuch mit `X-Inertia: true` durchlaufen hat:

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

Oder direkt über eine `HttpResponse` – den Rückgabewert von `InertiaResponse::resolve` – für einen Test, der die Response-Pipeline ohne Socket ausführt. Diese Form verarbeitet beide Darstellungen: einen JSON-Body mit `X-Inertia` oder das eingebettete Element `<script data-page="app">` der HTML-Shell:

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

`version()` prüft die Asset-Version der Seite. Der Standard-Resolver hasht das Vite-Manifest und fällt auf `MANIFEST_VERSION_FALLBACK` zurück, wenn noch kein Manifest existiert. Vergleichen Sie in einem Test ohne gebautes Frontend mit dieser Konstante statt mit einem fest codierten `"1.0"`:

```rust
use suprnova::MANIFEST_VERSION_FALLBACK;

response.assert_inertia().version(MANIFEST_VERSION_FALLBACK);
```

`has_flash(key, expected)` liest die Flash-Daten der Seite über denselben Punktpfad wie `has` / `where_` Props lesen. `expected` ist ein `Option`; übergeben Sie daher `None::<serde_json::Value>`, um nur auf Vorhandensein zu prüfen:

```rust
response.assert_inertia().has_flash("toast.message", Some(serde_json::json!("Saved!")));
response.assert_inertia().has_flash("toast", None::<serde_json::Value>);
```

### Für Assertions zu Partial Reloads und Deferred Props neu laden

`reload_only`, `reload_except` und `load_deferred_props` bilden ab, was der Inertia-Client nach dem ersten Besuch macht: dieselbe Seite als Partial Reload erneut anfordern und prüfen, was zurückkam. Da Suprnovas HTTP-Tests einen echten Socket passieren und jede Testdatei ihr eigenes Harness besitzt (siehe [Wo die einzelnen Teile liegen](#where-each-piece-lives) unten), enthalten diese Methoden keinen eingebauten Transport. Hängen Sie einen mit `with_reload` an: eine Closure, die aus einem `ReloadRequest` (URL, Komponente, Version und die zu sendenden Partial-Reload-Schlüssel) ein Future erzeugt, das das neu geladene `AssertableInertia` liefert:

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

Wird eine der drei Methoden ohne vorheriges `with_reload` aufgerufen, löst sie mit dieser Anweisung einen Panic aus. Das Ergebnis eines Reloads trägt denselben Reloader weiter, sodass ein zweites `.reload_only(...).await` darauf ohne erneutes Anhängen funktioniert.

### Warum Suprnova abweicht

Laravels `ReloadRequest` sendet die Anfrage über denselben In-Process-PHP-Kernel erneut, den der ursprüngliche Test verwendete – ein immer verfügbarer Test-Client. Suprnovas HTTP-Tests steuern einen echten Hyper/TCP-Loopback an und jede Testdatei definiert ihr eigenes Paar aus `spawn_server` / `request` (siehe [Wo die einzelnen Teile liegen](#where-each-piece-lives) unten). Es gibt daher keinen einzelnen Client, den `AssertableInertia` verwenden könnte; `with_reload` macht dies explizit, statt ein Harness fest zu codieren, das eine anders geformte Testdatei nicht verwenden könnte. `component()` überspringt außerdem Laravels Prüfung auf die Existenz der Seitenkomponentendatei (`view-finder`) – eine über `Router::inertia` oder ein von Hand gebautes `InertiaResponse::new(name)` erreichbare Komponente ist ein Laufzeit-String ohne zu prüfende Datei. Suprnovas Kompilierzeit-Gegenstück ist das Makro `inertia_response!` (siehe [Inertia Responses](frontend-inertia-responses.md)). Auch seine Methodennamen weichen von denen von `TestResponse` ab: `component`, `has`, `missing`, `where_`, `count` und `has_flash` lassen das Präfix `assert_` vollständig weg, wie Laravels `Inertia\Testing\AssertableInertia`, dessen entsprechende Methoden ebenso ohne Präfix heißen – der Vertrag „Panic bei Fehlschlag“ ist derselbe, nur ohne den visuellen Hinweis `assert_`.

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

`RouteParam<User>` hydriert ein typisiertes `User` über die Extractor-Chain
des Handlers, daher muss der Test diesen Extractor an eine `#[handler]`-
Funktion übergeben:

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
    // Einen Test-Benutzer über das Model einfügen. DB-Setup
    // ausgelassen - siehe das Testen-Kapitel für
    // `TestDatabase`-Muster.
    let user = User::create(suprnova::attrs! {
        email: "bound@example.com"
    }).await.unwrap();

    // Ein destrukturiertes `RouteParam` verwendet derzeit `param` als
    // Routenparameternamen des Handler-Makros.
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

Für einen Routenparameter `{user}` akzeptieren Sie stattdessen
`user: RouteParam<User>` ohne Destrukturierung; `RouteParam` dereferenziert
für Feldzugriff zu `User`. Der Aufruf von `req.param(...).parse()` und dann
`User::find_or_fail(...)` testet Parameterparsing und Model-Lookup, nicht
Route-Model-Binding.

Für Binding-in-Isolation-Tests rufen Sie
`<RouteParam<User> as AutoRouteBinding>::from_route_param(...)` direkt auf.
Das prüft die Binding-Implementierung ohne Router, trainiert aber nicht die
`#[handler]`-Extractor-Chain.

## Auth-Flows End-to-End testen

Um eine Login-Session end-to-end zu testen, übergeben Sie dem Loopback-Server
eine Registry mit `SessionMiddleware` und schützen `/dashboard` mit
`AuthMiddleware` oder der Web-Auth-Middleware der Anwendung. Prüfen Sie zuerst,
dass die Route eine cookie-lose Anfrage ablehnt, melden Sie sich dann an,
spielen Sie das zurückgegebene Session-Cookie erneut ab und prüfen Sie, dass die
geschützte Route erfolgreich ist:

```rust
#[tokio::test]
async fn login_flow_issues_session_cookie() {
    // 1. Bootstrap: den Benutzer erzeugen.
    Auth::password()
        .register("alice@example.com", "longpassword123")
        .await.expect("register");

    // 2. Eine geschützte Route und die zustandsbehaftete Session-Middleware mounten.
    let router: Router = Router::new()
        .post("/login", login_handler)
        .get("/dashboard", |_req: Request| async { text("dashboard") })
        .middleware(AuthMiddleware::new())
        .into();
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::new(SessionConfig::from_env()));
    let addr = spawn_server(router, registry, 3).await;

    // 3. Nachweisen, dass die Route vor der Authentifizierung geschützt ist.
    let (guest_status, _) = send_get(addr, "/dashboard").await;
    assert_eq!(guest_status, 401);

    // 4. Login ausführen und den Set-Cookie-Header erfassen.
    let login = post_json(addr, "/login", serde_json::json!({
        "email": "alice@example.com",
        "password": "longpassword123",
    })).await;
    assert_eq!(login.status, 200);
    let cookie = extract_session_cookie(&login.headers);

    // 5. Das Cookie gegen die geschützte Route erneut abspielen.
    let (status, body) = get_with_cookie(addr, "/dashboard", &cookie).await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"dashboard");
}
```

Der abgekürzte Router ohne diese Middlewares demonstriert nur die
Cookie-Verdrahtung; er ist kein Authentifizierungs-Flow-Test.
`framework/tests/auth_http_middleware.rs` testet das Verhalten der
Authentifizierungs-Middleware mit expliziten Registries, installiert jedoch
keine echte `SessionMiddleware`. Ein zustandsbehafteter Login-Flow-Test muss
wie oben gezeigt sowohl die Session-Middleware als auch das
Authentifizierungs-Gate installieren.

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

    let addr = spawn_server(router.into(), MiddlewareRegistry::new(), 4).await;

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
| `TestResponse` (fluente Assertions über das obige Triple) | `framework/src/testing/response.rs` |
| `AssertableInertia`, `ReloadRequest` (fluente Assertions zum Inertia-Seitenobjekt) | `framework/src/testing/inertia.rs` |
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

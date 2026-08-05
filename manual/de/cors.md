# CORS

`CorsMiddleware` beantwortet Preflight-`OPTIONS`-Anfragen und versieht
gewöhnliche Cross-Origin-Responses mit `Access-Control-Allow-*`-Headern.
Sie installieren sie einmal in `bootstrap()`, wenn ein Browser auf einem
anderen Origin Ihre API aufruft - öffentliche APIs, eine SPA, die auf
einer anderen Domain gehostet wird, ein mobiles Webview oder eine
separat gehostete Doku-Site. Same-Origin-Apps (Inertia, ausgeliefert vom
selben Host wie das Backend, der Suprnova-Standard) brauchen überhaupt
kein CORS. Die Middleware spiegelt Laravels `HandleCors` und
`config/cors.php`, aber als typisierten Builder auf `CorsConfig`.

## Global installieren

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

Ein Preflight ist eine `OPTIONS`-Anfrage mit einem
`Access-Control-Request-Method`-Header. Der Router hat keine
`OPTIONS`-Routen, ein Preflight *matcht* also nie eine Route - aber
Suprnovas Server führt die globale Middleware-Chain auch bei nicht
gematchten Anfragen aus (die dann in einem 404 endet), sodass eine
global installierte `CorsMiddleware` den Preflight sieht und ihn mit
`204` kurzschließt, bevor der 404 überhaupt entsteht. **Deshalb muss
CORS global installiert werden, nicht pro Route.**

## Eine Origin-Policy wählen

Es gibt absichtlich kein `Default` für `CorsConfig`. Eine reflexhaft
freizügige Policy ist ein Sicherheits-Eigentor, Sie müssen sich also
entscheiden:

| Builder | Verhalten |
| --- | --- |
| `CorsConfig::allow_origins([...])` | Feste Allowlist. Der Origin wird nur dann zurückgespiegelt, wenn er exakt mit einem Eintrag übereinstimmt. |
| `CorsConfig::any_origin()` | Wildcard `*`. Sind Credentials aktiviert, spiegelt die Middleware den konkreten Origin der Anfrage statt `*` (die Kombination aus `*` und Credentials ist laut Fetch-Spezifikation ungültig). |
| `.allow_origin_patterns([...])` | Regex-Patterns zusätzlich zur wörtlichen Liste. Nützlich für dynamische Subdomains. |

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .allow_origin_patterns([r"^https://[a-z0-9-]+\.staging\.example$"])
```

Patterns werden automatisch verankert - `^` und `$` werden
vorangestellt bzw. angehängt, wenn sie fehlen, sodass ein Teiltreffer
gegen eine Redirect-URL wie `https://evil.com/?u=https://app.example`
nicht durchrutschen kann.

Ein ungültiger Regex löst zur Config-Zeit (beim Boot) einen Panic
aus, nicht zur Anfragezeit - der Config-Fehler soll sichtbar werden,
statt die Prüfung still durchlässig zu machen.

`allowed_origins_patterns` (der Alias mit dem Laravel-Namen) steht
ebenfalls zur Verfügung.

## Eingrenzen, welche Pfade CORS erhalten

Laravels `cors.php`-Config hat ein `paths`-Array (`['api/*',
'sanctum/csrf-cookie']`), das die Anwendung von CORS auf bestimmte
URL-Patterns beschränkt. Suprnova spiegelt das:

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .paths(["api/*", "sanctum/csrf-cookie"])
```

Ist kein `paths` gesetzt, läuft CORS bei jeder Anfrage (Suprnovas
Standard, denn die Middleware ist ohnehin per Registrierung opt-in). Ist
mindestens ein Pattern gesetzt, werden nur passende Anfragen von CORS
behandelt (sowohl Preflights **als auch** das Versehen der eigentlichen
Response mit den Headern); alles andere läuft unberührt durch.

Patterns verwenden Laravels `Str::is`-Semantik: `*` ist eine
mehrsegmentige Wildcard, die gierig auch über `/` hinweggeht. Ein
führender `/` wird normalisiert, sodass `"api/*"` und `"/api/*"`
gleichwertig sind.

```rust,ignore
"api/*"             // matcht /api/users, /api/users/42
"api/*/posts"       // matcht /api/v2/posts, /api/v1/posts
"sanctum/csrf-cookie" // wörtlicher Exact-Match
"*"                 // matcht alles
```

## Überspringen per Prädikat

Für Prädikate über die Gestalt der Anfrage, die sich nicht als
Pfad-Pattern ausdrücken lassen (anhand eines Headers überspringen, CORS
nur in der Produktion laufen lassen, bei Health-Checks überspringen),
verwenden Sie `skip_when`:

```rust,ignore
CorsConfig::any_origin()
    .skip_when(|req| req.header("X-Internal-Call").is_some())
    .skip_when(|req| req.path() == "/healthz")
```

Spiegelt Laravels `HandleCors::skipWhen(Closure)`, sitzt aber auf der
Policy statt als globaler veränderlicher Zustand. Es lassen sich
mehrere `skip_when`-Callbacks registrieren; liefert auch nur einer
`true`, wird CORS übersprungen.

## Methoden, Header, exponierte Header

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .methods(["GET", "POST", "DELETE"])           // Standard = GET/POST/PUT/PATCH/DELETE/OPTIONS/HEAD
    .allow_headers(["Content-Type", "X-CSRF-TOKEN"])  // einschränken; Standard = Anfrage spiegeln
    .allow_any_headers()                          // explizites "spiegle, was angefragt wurde"
    .expose_headers(["X-Total-Count", "Link"])    // Header, die JS auf der Response lesen darf
```

Aliase mit den Laravel-Namen (damit `cors.php`-Nutzer finden, was sie
erwarten):

- `allowed_methods(...)` ≡ `methods(...)`
- `allowed_headers(...)` ≡ `allow_headers(...)`
- `exposed_headers(...)` ≡ `expose_headers(...)`
- `allowed_origins_patterns(...)` ≡ `allow_origin_patterns(...)`
- `supports_credentials(...)` ≡ `allow_credentials(...)`

## Credentials und `*`

Laut Fetch-Spezifikation ist `Access-Control-Allow-Origin: *` zusammen
mit Credentials ungültig - der Browser weist die Response zurück. Mit
einer expliziten Origin-Liste (`allow_origins([...])`) plus
`allow_credentials(true)` spiegelt die Middleware den konkreten
`Origin` der Anfrage statt `*`, und die Policy funktioniert wie
erwartet.

**`any_origin() + allow_credentials(true)` löst beim Bauen einen Panic
aus.** Die Kombination umgeht die Origin-Allowlist vollständig: Jede
Angreiferseite kann Cross-Origin-Anfragen mit Credentials stellen
und die Antworten lesen. Statt zur Laufzeit den falschen Header
auszugeben, scheitert der Policy-Konstruktor sichtbar, damit die
Fehlkonfiguration nie ein laufendes Deployment erreicht. Verwenden
Sie stattdessen eine explizite Allowlist:

```rust,ignore
// RICHTIG - explizite Allowlist mit Credentials.
CorsConfig::allow_origins(["https://app.example"]).allow_credentials(true)
// → bei einer Anfrage mit Origin: https://app.example
// → Response: Access-Control-Allow-Origin: https://app.example
//             Access-Control-Allow-Credentials: true

// BEIM BAUEN ABGELEHNT - Panic mit einem Hinweis zur Behebung.
// CorsConfig::any_origin().allow_credentials(true)
```

## Max-Age

```rust,ignore
.max_age(Duration::from_secs(600))   // typisiert
.max_age_secs(600)                   // ganze Sekunden im Laravel-Stil
```

`Access-Control-Max-Age` sagt dem Browser, wie lange er das
Preflight-Ergebnis zwischenspeichern darf. Höher = weniger
Preflight-Round-Trips, dafür verbreiten sich Policy-Änderungen
langsamer.

## Was die Middleware tatsächlich ausgibt

### Preflight (`OPTIONS` + `Access-Control-Request-Method`)

Wenn der Origin erlaubt ist:

```
HTTP/1.1 204 No Content
Access-Control-Allow-Origin: <origin>
Access-Control-Allow-Credentials: true        // wenn Credentials aktiviert
Access-Control-Allow-Methods: GET, POST, ...
Access-Control-Allow-Headers: <reflected or fixed>
Access-Control-Max-Age: 600                   // wenn gesetzt
Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers
```

Wenn der Origin nicht erlaubt ist: ein nacktes `204` + `Vary` (kein
`Access-Control-*`). Die Prüfung des Browsers auf fehlende Header
erzeugt dann den CORS-Fehler - passend zur Konvention von `tower-http`.

### Die eigentliche Cross-Origin-Response

Wenn die Anfrage einen `Origin`-Header trägt und der Origin erlaubt
ist:

```
Access-Control-Allow-Origin: <origin or *>
Access-Control-Allow-Credentials: true        // wenn aktiviert
Access-Control-Expose-Headers: X-Total, Link  // wenn konfiguriert
Vary: Origin                                  // nur wenn nicht "*"
```

Ein ACAO von `*` ist für jeden Origin identisch, es braucht also kein
`Vary`; ein konkreter Origin unterscheidet sich pro Origin, gemeinsam
genutzte Caches müssen ihn daher in ihren Schlüssel aufnehmen.

## CORS-Handler testen

CORS wird auf Browser-Seite durchgesetzt - der Server führt den Handler
auch dann aus, wenn der Origin nicht erlaubt ist; er versieht die
Response nur nicht mit den Headern. Genau das ist das testbare
Verhalten:

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

Bei einem nicht erlaubten Origin läuft der Handler und der Body kommt
zurück, aber das Fehlen von `Access-Control-Allow-Origin` ist es, was
den Browser am Lesen hindert.

## Laravel-Paritätsmatrix

| Laravel `cors.php` | Suprnova-Builder |
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

Die Middleware wird global registriert und nicht nach Laravel-Art
"automatisch installiert für `paths`" - Suprnovas Middleware-Chain ist
explizit, siehe [Middleware](middleware.md) für den Entwurf.

### Warum Suprnova abweicht

Laravels `HandleCors` wird automatisch an den Kernel gehängt und liest
seine Policy aus `config/cors.php`. Diese Form geht für PHP auf, weil
das Config-Array die eine Stelle ist, an der ein Framework mit einem
Prozess pro Anfrage Konfiguration teilen kann, ohne sie pro Anfrage neu
auszuwerten. Suprnova legt dieselben Optionen als typisierten
`CorsConfig`-Builder offen, den Sie explizit mit `global_middleware!`
registrieren, was die Middleware-Chain in `bootstrap()` sichtbar hält
und den Compiler die Wahl zwischen Allowlist und Wildcard erzwingen
lässt (kein `Default` für `CorsConfig`, Sie können also nicht
versehentlich `Access-Control-Allow-Origin: *` ausliefern, weil Sie
vergessen haben, einen Config-Wert zu füllen).

Die andere Abweichung ist, dass Preflights die Middleware auch auf
ungerouteten Pfaden erreichen. Laravel schickt `OPTIONS` durch seinen
Router, sodass der Preflight auf eine `OPTIONS`-Route trifft (die für
jede REST-Route automatisch registriert wird). Suprnovas Router hat
keine `OPTIONS`-Routen; stattdessen führt der Server die globale
Middleware-Chain bei nicht gematchten Anfragen aus, bevor er 404
liefert, sodass eine global installierte `CorsMiddleware` den Preflight
mit `204` kurzschließt, bevor der Not-Found-Pfad eingeschlagen wird.
Deshalb *muss* CORS global installiert werden - eine Registrierung pro
Route würde den Preflight nie sehen.

## Nächste Schritte

- [Middleware](middleware.md) - der Trait, die Chain, globale
  Registrierung gegenüber Registrierung pro Route, Terminable-Hooks
- [CSRF](csrf.md) - die andere globale Middleware, die die meisten Apps
  neben CORS installieren
- [Routing](routing.md) - wie Routen gematcht werden (und warum
  Preflights nicht matchen), plus der Pfad ohne Fallback, auf dem die
  globale Chain läuft
- [Request-Lifecycle](lifecycle.md) - wo CORS in der Chain sitzt,
  relativ zu Session, CSRF und dem Handler
- [Konfiguration](configuration.md) - typisierte Config-Patterns für
  Middleware, die umgebungsgesteuerte Einstellungen brauchen

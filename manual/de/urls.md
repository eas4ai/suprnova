# URL-Generierung

Über URLs verweist Ihre App auf sich selbst - jeder Redirect, jeder
E-Mail-Link, jedes `href` eines Inertia-`<Link>`, jeder signierte
Download muss irgendwoher kommen. Fest verdrahtete Pfade machen
Refactorings schmerzhaft und das Umbenennen von Routen unsicher.
Suprnova bringt einen kleinen `url::`-Namensraum mit und daneben den
Helfer `route()`, die einen Namen plus Parameter entgegennehmen und
Ihnen einen String zurückgeben - mit erledigter Prozent-Kodierung, mit
der Möglichkeit, Signaturen zu prägen, und mit einer Verifikation, die
Laravels Wire-Format Byte für Byte entspricht.

Dieses Kapitel ist die Referenz für die Oberfläche der
URL-Generierung. Das Kapitel [Routing](routing.md) behandelt, wie man
Routen deklariert und benennt; dieses hier behandelt, was Sie danach
mit diesen Namen tun.

```rust
use suprnova::{route, url};

// Lookup über den Namen → URL
let profile = route("users.show", &[("id", "42")]).unwrap();
//   "/users/42"

// Absolute URL gegen APP_URL
let absolute = url::to("/dashboard");
//   "https://app.test/dashboard"

// Signierter Link für den Passwort-Reset
let link = url::signed_route("password.reset", &[("token", reset_token)])?;
//   "/password/reset/xyz?signature=ab12..."

// Auf der eingehenden Anfrage verifizieren
if url::has_valid_signature(&request)? {
    // darauf reagieren
}
```

Alles in diesem Kapitel wird unter `suprnova::url::*` und
`suprnova::route` re-exportiert, sodass Consumer-Code nie direkt ins
Routing-Modul greifen muss.

## Benannte Routen

Ein Name ist ein String-Label, das einer Route bei der Registrierung
angeheftet wird. Sobald ein Name existiert, löst `route(name, params)`
ihn zurück zu einem URL-Pattern auf und setzt die Parameter ein. Namen
leben in einer einzigen prozessglobalen Registry - es gibt eine
`name → path`-Tabelle pro laufender Binary, nicht eine pro `Router`.

```rust
use suprnova::{routes, get, post};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

Der Aufruf `.name(...)` registriert `"users.show" → "/users/{id}"`. Von
da an kann jede Stelle im Prozess den Namen auflösen:

```rust
use suprnova::route;

let url = route("users.show", &[("id", "42")]);
// Some("/users/42")

let missing = route("does.not.exist", &[]);
// None
```

Dasselbe Paar `(name, path)` erneut zu registrieren ist idempotent -
nützlich, wenn die Routenregistrierung während des Boots mehr als
einmal läuft. Einen Namen unter einem *anderen* Pfad zu registrieren
löst einen Panic aus; diese Kollision ist ein sicherheitsrelevanter
Bug, weil Helfer wie `Redirect::route` stillschweigend auf die Seite
zeigen würden, die das Rennen gewonnen hat.

### Die Lookup-Helfer

| Funktion | Liefert | Wenn die Route fehlt |
|---|---|---|
| `route(name, params)` | `Option<String>` | `None` |
| `route_with_params(name, params_map)` | `Option<String>` | `None` |
| `try_route(name, params)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |
| `try_route_with_params(name, params_map)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |

Das nachsichtige Paar `route` / `route_with_params` lässt jedes
nicht gefüllte `{placeholder}`-Segment wortwörtlich in der Ausgabe
stehen - in Ordnung für Debug-Logs, unsicher, sobald es an einen
Browser geht. Das strikte Paar `try_route` / `try_route_with_params`
liefert `RouteUrlError::MissingParams { name, missing }` mit einer
Liste der nicht gefüllten Platzhalter, sodass der Aufrufer sichtbar
fehlschlagen kann, statt einen Benutzer auf `/users/{id}` umzuleiten.

```rust
use suprnova::routing::{try_route, RouteUrlError};

match try_route("users.show", &[]) {
    Ok(url) => /* sicher umzuleiten */,
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

`Redirect::route` verwendet aus genau diesem Grund intern
`try_route_with_params` - ein Redirect mit einem rohen `{id}` im
`Location`-Header wäre schlimmer als ein Fehlschlag.

### Prozent-Kodierung erfolgt automatisch

Parameterwerte werden nach den Pfadsegment-Regeln von RFC 3986
kodiert, bevor sie eingesetzt werden. Das umfasst die gen-delims und
sub-delims (`/ ? # [ ] @ ! $ & ' ( ) * + , ; =`), Steuerzeichen, das
Leerzeichen und `%` selbst. Unreservierte Zeichen
(`A-Z a-z 0-9 - _ . ~`) gehen unverändert durch.

```rust
use suprnova::route;

// Ein Slug mit einem Schrägstrich bleibt in einem Segment eingeschlossen:
route("posts.show", &[("slug", "hello/world")]);
// Some("/posts/hello%2Fworld")

// Versuche von Path Traversal können das Segment nicht verlassen:
route("users.show", &[("id", "../../etc/passwd")]);
// Some("/users/..%2F..%2Fetc%2Fpasswd")

// Echtes Unicode geht unangetastet durch:
route("users.show", &[("id", "user-é-42")]);
// Some("/users/user-%C3%A9-42")
```

Die Matching-Seite bewahrt diesen Rundlauf - eine Anfrage an
`/posts/hello%2Fworld` matcht die Route `/posts/{slug}`, und ein
Handler, der `req.param("slug")` liest, sieht dekodiert
`"hello/world"`. An der Grenze kodieren, an der Grenze dekodieren; die
rohen Bytes in Handler-Code nie zu Gesicht bekommen.

### Rückwärtssuche

Wenn Sie ein gematchtes Routen-Pattern haben und den registrierten
Namen wollen - etwa fürs Logging oder für Prüfungen der Art
`Request::route_is("users.show")` - nehmen Sie
`route_name_for_pattern`:

```rust
use suprnova::routing::route_name_for_pattern;

let name = route_name_for_pattern("/users/{id}");
// Some("users.show")
```

Das ist ein O(n)-Scan über die Namens-Registry. n ist die Zahl der
registrierten Namen; selbst bei vierstelligen Routenzahlen sind die
Kosten gegenüber dem umgebenden Lebenszyklus der Anfrage
vernachlässigbar. Die Funktion ist für Tooling und Middleware
offengelegt - `Request::route_is` ruft sie bereits für Sie auf, wenn
Sie in einem Handler gegen eine benannte Route vergleichen.

## Absolute URLs

Für alles Übrige - E-Mails bauen, URLs teilen,
Open-Graph-Metadaten verschicken - wollen Sie eine absolute URL mit dem
richtigen Schema und Host. `url::to` fügt einen Pfad an `APP_URL` an:

```rust
use suprnova::url;

// In der Umgebung: APP_URL=https://app.example.com
let url = url::to("/about");
// "https://app.example.com/about"

// Bereits absolute URLs gehen unverändert durch:
let cdn = url::to("https://cdn.example/asset.js");
// "https://cdn.example/asset.js"

let proto_relative = url::to("//cdn.example/asset.js");
// "//cdn.example/asset.js"
```

Host, Schema und Port kommen alle aus `APP_URL`. Ist `APP_URL` gleich
`http://localhost:8765`, dann liefert `url::to("/foo")` eben
`"http://localhost:8765/foo"`. Ein abschließender Schrägstrich in
`APP_URL` wird wegnormalisiert, damit Sie nie bei `https://host//path`
landen.

### HTTPS erzwingen

`url::secure(path)` baut dieselbe absolute URL, stuft das Schema aber
auf `https://` hoch, auch wenn `APP_URL` `http://` ist:

```rust
use suprnova::url;

// In der Umgebung: APP_URL=http://app.example.com
url::secure("/login");
// "https://app.example.com/login"
```

In Produktion setzen Sie `APP_URL` typischerweise einmal auf Ihren
HTTPS-Host und rufen `secure` nie direkt auf - das Hochstufen ist für
Umgebungen gedacht, in denen die lokale Entwicklung über HTTP läuft,
ein bestimmter Link aber HTTPS sein muss (etwa eine Callback-URL, die
in einer Payment-Session steckt).

### Die aktuelle URL lesen

In einem Handler ist die Anfrage selbst die Quelle der Wahrheit:

```rust
use suprnova::url;

async fn breadcrumbs(req: Request) -> Response {
    let here = url::current(&req);       // "/posts/42?expand=author"
    let full = url::full(&req);          // "https://app.test/posts/42?expand=author"
    let back = url::previous("/");        // von der Session aufgezeichnete vorherige URL
    // ...
}
```

| Helfer | Liefert | Quelle |
|---|---|---|
| `url::current(&req)` | Pfad + Query dieser Anfrage | Der aktuelle `Request` |
| `url::full(&req)` | absolute URL dieser Anfrage | `APP_URL` + `current(&req)` |
| `url::previous(fallback)` | die von der Session-Middleware aufgezeichnete vorherige URL | `_previous.url` in der Session, sonst `fallback` |

`previous` ist das, was `Redirect::back` trägt - die
Session-Middleware zeichnet die URL jedes erfolgreichen HTML-GET auf,
damit ein Formular-`POST` auf die Seite zurückspringen kann, die ihn
abgeschickt hat. Inertia-Partials, JSON-API-Anfragen
(`Accept: application/json` ohne `text/html`) und Responses außerhalb
von 2xx/3xx werden übersprungen, damit Sie nie auf einen
Zwischenendpunkt zurückspringen, den der Benutzer nie gesehen hat.

## Signierte URLs

Mit signierten URLs prägen Sie eine URL, die beweist, dass sie von
Ihrem Server stammt, ohne die URL irgendwo zu speichern. Die Signatur
ist HMAC-SHA256 über die kanonische Form der URL mit Ihrem `APP_KEY`;
der Server berechnet den HMAC auf der eingehenden Anfrage neu und
akzeptiert nur passende Signaturen.

Greifen Sie zu signierten URLs bei:

- **Per E-Mail zugestellten Links** - Passwort-Reset,
  E-Mail-Verifizierung, Einladung per E-Mail, Login per Magic Link. Die
  URL muss einen Umlauf durch ein Postfach überstehen, ohne dass sie
  serverseitig als undurchsichtiger Zustand hinterlegt werden müsste.
- **Kurzlebigen Downloads** - Links der Art "Ihr CSV-Export ist
  fertig", die nach 24 Stunden ablaufen; Ersatz für signierte
  S3-URLs, wenn die URL auf Ihrer Domain bleiben soll.
- **Webhooks, die auf Sie zurückzeigen** - Callbacks von Dritten, die
  gefälschte Aufrufe abweisen sollen, ohne pro Anfrage einen
  Datenbank-Lookup zu verlangen.

```rust
use suprnova::url;
use chrono::Utc;

// Dauerhaft signierte URL - läuft nie ab.
let link = url::signed_route(
    "password.reset",
    &[("user", user_id), ("token", token)],
)?;
// "/password/reset/42/xyz?signature=ab12cd34..."

// Temporär signierte URL - läuft in einer Stunde ab.
let expires_at = Utc::now().timestamp() + 3600;
let link = url::temporary_signed_route(
    "verify.email",
    &[("user", user_id)],
    expires_at,
)?;
// "/verify/email/42?expires=1748803600&signature=def012..."
```

Beachten Sie, dass `expires_at_epoch_seconds` ein **absoluter
UNIX-Zeitstempel** ist und keine Dauer. Berechnen Sie ihn an der
Aufrufstelle:

```rust
let one_hour_from_now = chrono::Utc::now().timestamp() + 3600;
let one_day_from_now  = chrono::Utc::now().timestamp() + 86_400;
```

Das hält die Signatur des Helfers klein und lässt Sie dieselbe Funktion
sowohl für Fristen relativ zu jetzt als auch für explizit absolute
verwenden.

### Verifizieren

Auf der eingehenden Seite verifizieren Sie die Signatur gegen die
laufende Anfrage:

```rust
use suprnova::{url, FrameworkError, Request, Response, HttpResponse};

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    if !url::has_valid_signature(&req)? {
        return Err(FrameworkError::forbidden("Invalid or expired link"));
    }
    // Signatur ist gut und nicht abgelaufen - weiter.
    let user_id = req.param("user").unwrap();
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`has_valid_signature` liefert nur dann `true`, wenn der HMAC passt UND
die URL nicht abgelaufen ist. Für die dreiwertige Unterscheidung
zwischen *ungültig*, *abgelaufen* und *gültig* nehmen Sie
`signature_verdict`:

```rust
use suprnova::{url, FrameworkError, HttpResponse, Request, Response};
use suprnova::routing::SignatureVerdict;

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    match url::signature_verdict(&req)? {
        SignatureVerdict::Valid => {
            // Weiter.
        }
        SignatureVerdict::Expired => {
            // Den Benutzer auf eine Seite schicken, die erklärt, dass
            // der Link abgelaufen ist, und einen frischen anbietet.
            return Ok(HttpResponse::new()
                .status(302)
                .header("Location", "/password/reset-expired"));
        }
        SignatureVerdict::Invalid => {
            // Ein generisches 403 rendern - nicht verraten, ob die
            // Signatur fehlerhaft, gar nicht vorhanden oder einfach
            // falsch war.
            return Err(FrameworkError::forbidden("Invalid link"));
        }
    }
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`signature_has_not_expired(&req)` ist deprecated und antwortet jetzt
genau das, was `has_valid_signature` antwortet. Greifen Sie stattdessen
zum obigen `signature_verdict`; eine URL ohne `expires`-Query-Parameter
ist per Definition "nie abgelaufen", in Suprnova wie in Laravel.

### Warum Suprnova abweicht

Laravels `URL::signatureHasNotExpired($request)` heißt wörtlich "nicht
abgelaufen", eine **gefälschte** Signatur kommt also als `true`
zurück - sie hatte nie ein Ablaufdatum, das sie verpassen konnte.
Suprnovas Variante entsprach dem früher. Das tut sie nicht mehr: Der
Helfer verlangt zuerst eine gültige Signatur.

Der Grund: `expires` stammt vom Angreifer, bis der HMAC etwas anderes
sagt, also bedeutet keine daraus abgeleitete Antwort irgendetwas,
bevor die Signatur aufgeht - und eine Funktion, deren Name nach einer
Schutzprüfung klingt, ließ jede gefälschte URL durch alles hindurch,
was sie allein aufrief.

Gültigkeit zu verlangen lässt sie mit `has_valid_signature`
zusammenfallen, und deshalb trägt sie eine Deprecation und kein
Verhaltens-Flag. Dieses Zusammenfallen ist kein Verlust: Unter einem
dreiwertigen Urteil gibt es kein "nicht abgelaufen", das ein einzelnes
`bool` ehrlich melden könnte, außer `Valid`. Wenn Sie *abgelaufen* von
*ungültig* unterscheiden wollen - um "fordern Sie einen frischen Link
an" statt "verboten" zu sagen - ist genau dafür `signature_verdict` da,
und es sagt es im Typ.

### Beliebige URLs signieren

Wenn die URL, die Sie signieren wollen, nicht aus einer registrierten
benannten Route stammt - eine Callback-URL, die Ihnen ein Dritter
gereicht hat, ein zur Laufzeit dynamisch gebauter Pfad - nehmen Sie
direkt `signed_url`:

```rust
use suprnova::url;

let callback = url::signed_url(
    "/webhooks/stripe/callback?order=42",
    Some(chrono::Utc::now().timestamp() + 600),  // 10 Minuten Ablauffrist
)?;
```

Übergeben Sie `None` als Ablauf, um eine dauerhafte Signatur zu prägen.
Die Verifikationsseite ist dieselbe - `has_valid_signature(&req)` ist
es egal, ob die URL aus einer benannten Route oder aus einem rohen Pfad
geprägt wurde.

### Wire-Format

Zwei URLs, die sich nur in der Reihenfolge der Query-Parameter
unterscheiden, ergeben identische Signaturen, weil die kanonische Form
die Query-Paare vor dem Hashen lexikografisch sortiert. Das ist
wichtig, weil Clients Query-Parameter unterwegs manchmal umsortieren
(Proxys, Link-Vorschauen, mobile E-Mail-Apps), und eine signierte URL,
die beim Umsortieren zerbricht, wäre unbrauchbar.

| Bestandteil | Wert |
|---|---|
| Algorithmus | HMAC-SHA256 |
| Schlüssel | Rohe Bytes des aktiven `APP_KEY` |
| Payload | `path?<sorted-query>` (das `?` entfällt ohne Parameter) |
| Sortierreihenfolge | `(key, value)` - jedes Paar, Wiederholungen eingeschlossen |
| Kodierung | Hex-kodierter Digest mit 64 Zeichen |
| Vergleich | In konstanter Zeit über `subtle::ConstantTimeEq` |
| Reservierte Schlüssel | `signature`, `expires` |

**Wiederholte Schlüssel werden signiert, nicht zusammengefasst.**
`?tag=a&tag=b` trägt beide Werte in den Payload, keiner lässt sich also
hinzufügen, entfernen oder ersetzen, ohne die Signatur zu brechen.
Dass nach `(key, value)` sortiert wird und nicht nach dem Schlüssel
allein, hält diese Ordnung total, sodass die obige Garantie beim
Umsortieren auch dann gilt, wenn ein Schlüssel mehrfach vorkommt.

Das ist erwähnenswert, weil die Alternative hart zubeißt. Eine frühere
Version kanonisierte in eine Map, die bei einem wiederholten Schlüssel
nur den letzten Wert behielt. `Request::query_param` lieferte den
*ersten*. Ein legitim signiertes `?user=victim` ließ sich also mit der
ursprünglichen Signatur als `?user=attacker&user=victim`
wiedereinspielen: Die Verifikation sah `victim` und ließ durch, der
Handler handelte an `attacker`. Signiert und ausgeführt waren
verschiedene URLs. Alle drei Query-Accessoren - `query_param`,
`query_params` und `Context::query_param` - lösen einen wiederholten
Schlüssel jetzt auf seinen letzten Wert auf, und die kanonische Form
verliert nichts.

Ein wiederholtes `signature` oder `expires` wird rundweg abgewiesen.
Das sind Steuerparameter; zweimal eines davon lässt keine
nicht willkürliche Antwort auf "welches gilt?" übrig, und der
Verifizierer sollte nicht die Komponente sein, die rät.

Der HMAC-Payload schließt einen bereits vorhandenen
`signature`-Query-Parameter aus (Signieren über einer Signatur ist also
ein No-op) und gibt aus den Aufrufargumenten einen frischen
`expires`-Wert neu aus. Ein Client, der das `expires` entfernt oder
umschreibt, bricht die Signatur; ein Client, der die `signature`
entfernt, scheitert als `Invalid`. Beide sind Fail-Closed.

Das Fragment (`#section`) wird aus der kanonischen Form entfernt, weil
Browser Fragmente nie an den Server zurückschicken. Über ein Fragment
mit zu signieren würde jeden Link in dem Moment ungültig machen, in dem
ein Client einen Anker anhängt - `?signature=...#docs` würde auf der
Serverseite nicht verifizieren.

### Reservierte Query-Parameter

`signature` und `expires` sind reservierte Namen für Query-Parameter.
Eine Route, die legitim einen Query-Parameter namens `signature` oder
`expires` erwartet, würde mit der Maschinerie für signierte URLs
kollidieren, und der Verifizierer würde den Wert falsch zuordnen.
Benennen Sie den Parameter entweder um oder fassen Sie die eingehenden
Parameter der Route unter einem anderen Namensraum zusammen.

```rust
// Schlecht - `signature` kollidiert mit dem reservierten Namen.
get!("/api/check", check)  // nimmt ?signature=hash

// Gut - in einen Namensraum stecken.
get!("/api/check", check)  // nimmt ?body_signature=hash
```

Die Konstanten sind offengelegt, damit sie zum Wire-Format von Laravel
symmetrisch bleiben:

```rust
use suprnova::routing::{SIGNATURE_KEY, EXPIRES_KEY};
// SIGNATURE_KEY == "signature"
// EXPIRES_KEY   == "expires"
```

### Schlüsselrotation

Signierte URLs verwenden denselben `APP_KEY`, der auch `Crypt::encrypt`
und die Integrität des Session-Cookies trägt. `APP_KEY` zu rotieren
macht jede zuvor geprägte Signatur ungültig, die noch unterwegs ist -
eine gerade laufende Passwort-Reset-Mail wird beim nächsten Klick des
Benutzers zu einem 403.

Für die meisten Anwendungen ist das das richtige Verhalten. Wenn Sie
eine sanfte Rotation mit Überlappung brauchen (damit alte Links über
ein Deployment-Fenster hinweg weiter funktionieren), tragen Sie mit
`APP_KEY_PREVIOUS` den vorherigen Schlüssel weiter; der Keyring
probiert bei der Verifikation jeden installierten Schlüssel. Siehe das
Kapitel [Hashing](hashing.md) für den vollständigen Umgang mit dem
Keyring.

## Fehler und Grenzfälle

Eine Handvoll Fehlermodi sind es wert, dass man sie kennt:

- **`route(name, ...)` liefert `None`**, wenn der Name nicht
  registriert ist. Das ist die nachsichtige Oberfläche - das
  stille Fehlschlagen ist Absicht, damit aufrufender Code auf einen
  Standardwert zurückfallen kann. Für ein sichtbares Fehlschlagen
  nehmen Sie `try_route`.
- **`try_route` liefert `Err(NameNotFound)`** bei einem unbekannten
  Namen und `Err(MissingParams { name, missing })`, wenn ein
  erforderlicher `{placeholder}` keinen passenden Wert hat.
- **`url::signed_route` und Verwandte liefern `FrameworkError`**,
  wenn der Verschlüsselungsschlüssel nicht installiert ist (etwa
  weil Sie `APP_KEY` in `.env` vergessen haben). In Produktion
  schlägt das schon beim Boot fehl, weil `Crypt::init` während
  `Server::from_config` läuft; der Fehlerweg hier existiert, um eine
  Fehlkonfiguration sichtbar zu machen, statt nicht verifizierbare
  Links zu erzeugen.
- **`has_valid_signature` liefert `Ok(false)`** und nicht `Err`, wenn
  eine Signatur ungültig oder abgelaufen ist. Die
  `FrameworkError`-Variante ist Fehlern der Art "der Server kann nicht
  einmal prüfen" vorbehalten (fehlender Schlüssel).
- **Eine signierte URL mit manipuliertem `expires`** verifiziert als
  `Invalid`, nicht als `Expired`. Der HMAC-Payload enthält den
  `expires`-Wert, ihn zu ändern bricht also zuerst die Signatur.

```rust
use suprnova::{routing::SignatureVerdict, url};

// All diese sind Invalid, nicht Expired:
url::signature_verdict(&req)?;  // signature-Query-Parameter fehlt
url::signature_verdict(&req)?;  // signature ist Müll und kein Hex
url::signature_verdict(&req)?;  // Pfad wurde manipuliert (/orders/1 → /orders/2)
url::signature_verdict(&req)?;  // irgendein Query-Parameter-Wert wurde manipuliert
url::signature_verdict(&req)?;  // expires-Wert wurde manipuliert

// Das hier ist Expired:
url::signature_verdict(&req)?;  // gültiger HMAC, aber jetzt > expires
```

## Warum Suprnova abweicht

Laravels `URL`-Facade trägt `asset()`, `secureAsset()`, `assetFrom()`
und `action()`. Suprnova liefert keines davon mit - aus bewussten
Gründen.

**Assets**. Suprnovas Frontend-Ansatz ist Vite plus die
Filesystem-Disks ([Dateisystem](filesystem.md)), kein eigenständiger
Asset-Helfer. Vites Direktive `@vite('resources/app.ts')` (oder das
Äquivalent des Inertia-Adapters) gibt in Produktion die korrekten
gehashten URLs aus und in der Entwicklung die URL des Dev-Servers.
Einen parallelen `URL::asset()`-Kanal zu bauen würde den Umgang mit
Assets auf zwei Systeme aufteilen, die sich über Hashing, Versionierung
und darüber einig sein müssten, welches Manifest maßgeblich ist. Die
Vite-Seite hat diese Zuständigkeit bereits gewonnen.

**Action-Routing**. Laravels `action('UserController@show', ['id' => 1])`
beruht auf PHPs Routing über Klassen-Strings - Controller sind Klassen
mit Methoden, und das Framework kann einen `action`-String rückwärts
auflösen. Rust-Handler sind freie Funktionen. Die nächste Entsprechung
sind benannte Routen, und `route("users.show", &[("id", "1")])` ist
bereits die richtige Schnittstelle. Routing über Action-Strings auf
Rusts Handler-Typen wieder einzuführen würde gegenüber benannten Routen
nichts Echtes hinzufügen.

**`URL::forceScheme()` / `URL::forceRootUrl()`**. Laravel legt diese
für Tests offen und für Sites hinter Reverse Proxys, die
`X-Forwarded-Proto` nicht durchreichen. Suprnova regelt beide Fälle
über Konfiguration: `APP_URL` trägt den kanonischen Host und das
kanonische Schema; für Proxy-Umgebungen liest die
Trusted-Proxy-Middleware ([Middleware](middleware.md)) die
`X-Forwarded-*`-Header und aktualisiert die Anfrage-URL, bevor sie
Ihren Handler erreicht. Für `forceScheme` gibt es nichts zu
überschreiben - `APP_URL` sagt bereits, welches Schema gilt.

Was hier ankommt, ist die nach außen sichtbare Form, zu der Aufrufer
greifen, mit denselben Laravel-förmigen Namen, wo sie sich sauber
übertragen lassen. Der Beschnitt ist Absicht, kein Versehen.

## Nächste Schritte

- [Routing](routing.md) - Routen deklarieren und benennen,
  Routengruppen, Resource-Routing und die vollständige
  Matching-Oberfläche pro Methode
- [Antworten](responses.md) - `Redirect::route`,
  `Redirect::signed_route`, `Redirect::back` und der Rest der Familie
  von Redirect-Helfern, die die URL-Generierung konsumiert
- [Hashing](hashing.md) - der Lebenszyklus von `APP_KEY`,
  Schlüsselrotation und der gemeinsame Keyring, der das Signieren von
  URLs neben der Verschlüsselung trägt
- [Auth-Flows](auth-flows.md) - die produktiven Nutzer signierter URLs:
  Passwort-Reset, E-Mail-Verifizierung und Remember-me-Cookies
- [Anfragen](requests.md) - `Request::path`, `Request::query`,
  `Request::route_is` und die Gegenseite jedes Helfers in diesem
  Kapitel

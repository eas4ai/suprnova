# Autorisierung

Authentifizierung beantwortet _"wer sind Sie?"_; Autorisierung
beantwortet _"dürfen Sie das?"_ Suprnova liefert eine
Laravel-förmige `Gate`-Facade plus das `#[policy]`-Makro für
ressourcenorientierte Verdrahtung, mit synchronen und asynchronen
Varianten jeder Prüfung, sodass dieselbe Oberfläche funktioniert,
egal ob Ihr Policy-Rumpf einen DB-Zugriff braucht oder nur einen
Struktur-Feldvergleich.

## Schnellstart

```rust
use suprnova::{Authorizable, Gate};

#[derive(Debug)]
struct User { id: i64, is_admin: bool }
#[derive(Debug)]
struct Post { id: i64, author_id: i64, is_public: bool }

// Lässt Benutzer sich für die `user.can(action, &resource)`-Ergonomie entscheiden.
impl Authorizable for User {}

// Eine Fähigkeit verdrahten:
Gate::define::<User, Post>("update", |user, post| {
    user.is_admin || post.author_id == user.id
});

let alice = User { id: 1, is_admin: false };
let own_post = Post { id: 10, author_id: 1, is_public: false };
let foreign_post = Post { id: 11, author_id: 99, is_public: false };

assert!(alice.can("update", &own_post));
assert!(alice.cannot("update", &foreign_post));

// Direkt aus einem Handler 403 zurückgeben:
alice.authorize("update", &foreign_post)?;
```

## Die `Gate`-Oberfläche

### Fähigkeiten definieren

```rust
// Synchrone Closure - wird direkt aufgerufen, kein geboxtes Future.
Gate::define::<User, Post>("view", |user, post| post.is_public || user.id == post.author_id);

// Asynchrone Closure - das Future muss owned sein (keine Borrows über die Rückgabe der Closure hinaus).
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_is_admin = user.is_admin;
    let post_id = post.id;
    async move {
        // ...DB-Lookup, RPC-Aufruf usw.
        user_is_admin || check_publish_permission(post_id).await
    }
});
```

Intern typgelöscht; die Registry schlüsselt über
`(action, TypeId<U>, TypeId<R>)`. Ein `User`-Aktions-Gate und ein
`Comment`-Aktions-Gate desselben Namens existieren unabhängig
voneinander - `Gate::has::<User, Post>("publish")` und
`Gate::has::<User, Comment>("publish")` antworten getrennt.

### Fähigkeiten prüfen

| Methode | Liefert | Verwendung |
|---|---|---|
| `Gate::allows(action, &user, &resource)` | `bool` | Schnelle Verzweigung |
| `Gate::denies(action, &user, &resource)` | `bool` | Umkehrung |
| `Gate::authorize(action, &user, &resource)` | `Result<(), FrameworkError>` | 403 bei einer bloßen Ablehnung; eine reichhaltige Ablehnung trägt ihren eigenen Status/ihre eigene Nachricht (siehe [Reichhaltige Entscheidungen](#reichhaltige-entscheidungen-response-inspect-raw)) - bricht einen Handler mit `?` per Short-Circuit ab |
| `Gate::inspect(action, &user, &resource)` | `Response` | Vollständige Entscheidung: `allowed` + `message` + `code` + HTTP-`status` |
| `Gate::raw(action, &user, &resource)` | `Option<Response>` | Wie `inspect`, aber `None` = keine Regel definiert (im Gegensatz zu einer expliziten Ablehnung) |
| `Gate::any(&[...], &user, &resource)` | `bool` | True, wenn irgendeine erlaubt |
| `Gate::none(&[...], &user, &resource)` | `bool` | True, wenn keine erlaubt |
| `Gate::check(&[...], &user, &resource)` | `bool` | True, wenn alle erlauben |

### Introspektion

```rust
// Ist eine Fähigkeit definiert?
Gate::has::<User, Post>("publish");  // bool

// Welche Fähigkeiten existieren? (sortiert + dedupliziert nach Aktionsname)
let all: Vec<String> = Gate::abilities();
```

`abilities()` dedupliziert über Ressourcentypen hinweg: Das
Registrieren von `"view"` sowohl für `User`-auf-`Post` als auch für
`User`-auf-`Comment` ergibt einen einzigen `"view"`-Eintrag.
Nützlich für Admin-Auswahllisten und Inertia-Shared-Data.

### Semantik bei fehlendem Gate

Der Aufruf von `allows` / `denies` / `authorize` für eine Aktion,
die nie registriert wurde, **verweigert standardmäßig**. Dasselbe
gilt für den Aufruf der synchronen API auf einem asynchron
registrierten Gate (der synchrone Pfad kann nicht awaiten - die
Standardverweigerung bringt den Bug über `tracing::warn!` in die
Logs, statt ihn stillschweigend durchzulassen). Asynchron
registrierte Gates antworten über die `_async`-Pfade korrekt.

## Policies mit `#[policy]`

Wenn ein Ressourcentyp mehrere Fähigkeiten hat, gruppieren Sie sie
in einer Policy-Struktur und lassen Sie `#[policy]` jede Methode
als Gate registrieren:

```rust
use suprnova::policy;
use suprnova::authorization::Response;

struct User { id: i64, is_admin: bool }
struct Post { id: i64, author_id: i64, is_public: bool }
struct PostPolicy;

#[policy(User, Post)]
impl PostPolicy {
    // Eine `-> bool`-Methode ist ein einfaches Allow/Deny-Gate.
    fn view_any(_user: &User, _post: &Post) -> bool {
        true // jeder darf Beiträge auflisten
    }
    fn view(user: &User, post: &Post) -> bool {
        post.is_public || post.author_id == user.id || user.is_admin
    }

    // Eine `-> Response`-Methode kann bei einer Ablehnung eine Nachricht + einen HTTP-Status tragen.
    fn update(user: &User, post: &Post) -> Response {
        if post.author_id == user.id || user.is_admin {
            Response::allow()
        } else {
            Response::deny_with("You may only edit your own posts.")
        }
    }
    fn delete(user: &User, post: &Post) -> Response {
        if user.is_admin {
            Response::allow()
        } else {
            Response::deny_as_not_found() // verbirgt den Beitrag vor Nicht-Admins
        }
    }
}
```

Jede Methode wird zu einem `inventory::submit!`. `Server::serve`
leert die Inventory über `init_policies()` beim Boot, sodass zum
Zeitpunkt der ersten Anfrage jede Aktion registriert ist (siehe
[Application Bootstrap](bootstrap.md) dazu, wo das in die
Boot-Sequenz einsortiert wird). `init_policies()` liegt unter
`suprnova::authorization::init_policies` und ist idempotent - rufen
Sie es in Tests, die die Policy-Registrierung prüfen, ohne einen
Server hochzufahren, manuell auf.

Policy-Methoden sind zustandslose assoziierte Funktionen, die
`(user, resource)` entgegennehmen - dieselbe Form wie Laravels
`update(User $user, Post $post)`, wobei `$this` das zustandslose
Policy-Objekt ist. Jede Methode nimmt beide Argumente für eine
einheitliche Gate-Signatur entgegen; `view_any` / `create`
ignorieren die Ressource einfach (`_post`). Methoden, die Sie nicht
schreiben, werden nicht registriert, und eine nicht registrierte
Aktion verweigert standardmäßig.

### Methodenname → Aktions-Zuordnung

Der Methodenname wird direkt als Verbsegment der Aktion verwendet,
wobei die Ressource kebab-case-formatiert angehängt wird:

| Methode | Aktion |
|---|---|
| `view` auf `Post` | `"view-post"` |
| `view_any` auf `Post` | `"view_any-post"` |
| `force_delete` auf `UserProfile` | `"force_delete-user-profile"` |

Das weicht von Laravels camelCase-Aktionsnamen (`viewAny`,
`forceDelete`) ab, um die Rust-Oberfläche idiomatisch zu halten -
jeder Aktions-String spiegelt den Methodenbezeichner, den Sie in
Ihrem Editor autovervollständigen würden.

### Rückgabetyp: `bool` oder `Response`

Der Rückgabetyp einer Policy-Methode wählt, wie sie registriert
wird - und was eine Ablehnung tragen kann:

| Rückgabetyp | Registriert über | Ablehnung erscheint als |
|---|---|---|
| `bool` | `Gate::define` | bloßes `403` (`This action is unauthorized.`) |
| `Response` | `Gate::define_with` | die Nachricht, der Code und der HTTP-Status, den die `Response` trägt |

Geben Sie `bool` für ein einfaches Ja/Nein zurück. Geben Sie eine
`Response` zurück (importiert aus
`suprnova::authorization::Response`), wenn eine Ablehnung einen
Grund oder einen von 403 abweichenden Status tragen soll -
`Response::deny_with("…")` für eine Nachricht, oder
`Response::deny_as_not_found()`, um mit `404` zu antworten und die
Existenz der Ressource zu verbergen. Beide kompilieren zum
gleichen typgelöschten Gate (ein `bool` wird in ein bloßes
Allow/Deny eingewickelt). Jeder andere Rückgabetyp - oder ein
fehlender - ist ein Kompilierfehler.

## Der Trait `Authorizable`

Drop-in-Zucker auf Benutzerseite für die `Gate`-Aufrufe:

```rust
use suprnova::Authorizable;

impl Authorizable for User {}

// Sync-Zucker
if alice.can("update", &post)    { /* ... */ }
if alice.cannot("delete", &post) { /* ... */ }
alice.authorize("update", &post)?;  // 403 bei Ablehnung

// Async-Zucker
if alice.can_async("publish", &post).await    { /* ... */ }
alice.authorize_async("publish", &post).await?;
```

Jede Methode hat einen Standard-Rumpf, der an die passende
`Gate`-Methode delegiert, sodass `impl Authorizable for User {}`
(ohne Rumpf) ausreicht. Opt-in statt Blanket-Impl: Nicht jeder Typ,
der an `Gate::allows` übergeben werden kann, ist als Subjekt von
`.can` gedacht - meist ist es der `User` Ihrer Anwendung.

## Kompositionsmuster

### Routen-Gruppen per Gate schützen

```rust
use suprnova::{group, get, Auth, AuthMiddleware, FrameworkError, Request, Response};

// Middleware prüft den Auth-Benutzer; der Handler autorisiert die Aktion.
group!("/posts")
    .middleware(AuthMiddleware::new())
    .routes([
        get!("/{id}/edit", edit_form),
    ]);

async fn edit_form(req: Request) -> Response {
    let user: User = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let post = Post::find(id).await?
        .ok_or_else(|| FrameworkError::not_found("Post"))?;
    user.authorize("update", &post)?;
    // ... Bearbeitungsformular rendern
}
```

### Prüfungen mehrerer Aktionen

Eine Seite im Stil "alles auflisten, was dieser Benutzer mit dieser
Ressource tun darf":

```rust
let actions = ["view", "update", "delete", "restore", "force_delete"];
let mut allowed = Vec::new();
for action in &actions {
    if user.can(action, &post) {
        allowed.push(*action);
    }
}
// Oder per Short-Circuit:
let can_do_anything = Gate::any(&actions, &user, &post);
let is_locked_out   = Gate::none(&actions, &user, &post);
```

### Autorisierung über mehrere Gates

```rust
// Nur erlauben, wenn der Benutzer ALLE diese Aktionen auf der Ressource ausführen darf.
Gate::authorize_async("publish", &user, &post).await?;
if Gate::check_async(&["update", "view"], &user, &post).await {
    // Prüfungen kombinieren.
}
```

### Ressourcen-Routen per Gate schützen

Wenn eine `Router::resource`-Oberfläche existiert, verdrahtet
`authorize_resource::<U, R>()` die konventionelle
Fähigkeitsprüfung auf allen sieben Routen gleichzeitig, sodass Sie
nicht darauf angewiesen sind, dass jede Controller-Methode selbst
an die Autorisierung denkt:

```rust
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()   // index/show→view, store→create, …
    .into();
```

Eine abgelehnte Fähigkeit liefert `403`, bevor der Handler läuft;
eine nicht authentifizierte Anfrage schlägt fail-closed fehl. Die
vollständige Aktion-→-Fähigkeit-Tabelle liegt im
[Routing-Kapitel](routing.md).

## Async-Semantik

Die Closure von `Gate::define_async` muss ein **owned** Future
zurückgeben - die typgelöschte Registry kann nicht zulassen, dass
`&user`- oder `&resource`-Referenzen die Rückgabe der Closure
überleben. Kopieren oder klonen Sie alle Felder, die Sie brauchen,
innerhalb des `async move {}`-Blocks, bevor Sie ihn zurückgeben:

```rust
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_id = user.id;        // Primitive kopieren
    let post_id = post.id;
    let admin   = user.is_admin;
    async move {
        // Hier keine `user`- / `post`-Referenzen - nur die erfassten Kopien.
        admin || check_can_publish(user_id, post_id).await
    }
});
```

Synchrone Gates funktionieren transparent aus dem asynchronen Pfad
(`Gate::allows_async` dispatcht sie ohne ein `.await`), sodass eine
Codebasis heute synchrone Gates registrieren und einzelne
Fähigkeiten später auf async migrieren kann, ohne die Aufrufstellen
zu ändern.

## Haltung bei einer vergifteten Sperre

Die `Gate`-Registry verwendet intern ein `RwLock`. Wird die Sperre
jemals vergiftet (ein Thread ist gepanickt, während er den
Write-Guard hielt), **verweigert die Registry sicherheitshalber** -
jeder nachfolgende `authorize`-Aufruf liefert `Unauthorized`, statt
zu paniken. Registrierungsaufrufe protokollieren über
`tracing::error!` und laufen weiter. Das entspricht der
umfassenderen Richtlinie des Frameworks: Eine vergiftete Sperre
bricht den Prozess niemals ab.

## Reichhaltige Entscheidungen: `Response`, `inspect`, `raw`

Ein bloßes `bool`-Gate beantwortet nur Allow/Deny. Für eine
Ablehnung, die eine *Nachricht*, einen maschinenlesbaren *Code*
oder einen von 403 abweichenden HTTP-*Status* trägt, registrieren
Sie das Gate mit `define_with` (oder `define_async_with`) und geben
Sie eine `Response` zurück:

```rust
use suprnova::authorization::Response;  // am Crate-Root re-exportiert als `GateResponse`

Gate::define_with::<User, Post>("update", |user, post| {
    if post.author_id == user.id {
        Response::allow()
    } else {
        Response::deny_with("You do not own this post.")
    }
});

// Die Existenz einer Ressource verbergen, statt sie einzugestehen:
Gate::define_with::<User, Secret>("view", |user, secret| {
    if user.can_see(secret) {
        Response::allow()
    } else {
        Response::deny_as_not_found()  // ein 404, kein 403
    }
});
```

Untersuchen Sie die vollständige Entscheidung mit `Gate::inspect`
(sync) / `Gate::inspect_async`:

```rust
let decision = Gate::inspect("update", &user, &post);
decision.allowed();   // bool
decision.message();   // Option<&str> - Some("You do not own this post.")
decision.status();    // Option<u16> - None here; Some(404) after deny_as_not_found
```

`Response`-Konstruktoren spiegeln Laravel: `allow()`, `deny()`,
`deny_with(msg)`, `deny_with_status(status, msg)`,
`deny_as_not_found()`, plus die Builder `with_message` /
`with_code` / `with_status` / `as_not_found`.

### Wie eine Ablehnung zu einem Fehler wird

`Gate::authorize` kollabiert die Entscheidung über
`Response::authorize()`:

| Entscheidung | Ergebnis von `authorize` |
|---|---|
| erlaubt | `Ok(())` |
| bloßes `deny()` (ohne Nachricht/Code/Status) | `FrameworkError::Unauthorized` (403, `"This action is unauthorized."`) |
| reichhaltige Ablehnung (Nachricht und/oder Status gesetzt) | `FrameworkError::Domain { message, status_code }` |

So erscheint `deny_as_not_found()` als 404,
`deny_with_status(422, "…")` als 422 und `deny_with("…")` als 403
mit Ihrer Nachricht. Der `code` ist auf der untersuchten `Response`
lesbar, reist aber **nicht** durch `authorize` hindurch -
`FrameworkError` hat kein Code-Feld; lesen Sie ihn aus `inspect()`,
falls Sie ihn brauchen.

### `raw`: "abgelehnt" vs. "nicht definiert"

`Gate::raw` (und `raw_async`) liefert `Option<Response>`: `None`
bedeutet *keine Regel angewendet* - kein `before`-Hook hat
gefeuert, kein Gate ist registriert, kein `after`-Hook hat
aufgefüllt - im Unterschied zu einem expliziten `Some(deny)`.
`inspect` normalisiert dieses `None` zu einer Standardablehnung;
`raw` bewahrt es für Diagnosen ("ist diese Aktion überhaupt
geregelt?").

## `before`- / `after`-Hooks

`Gate::before` registriert eine Prüfung, die *vor* jedem Gate
läuft; der erste Hook, der `Some(decision)` zurückgibt, bricht
alles per Short-Circuit ab. Die kanonische Verwendung ist eine
globale Übersteuerung:

```rust
// Administratoren dürfen alles.
Gate::before::<User>(|user, _action| user.is_admin.then_some(true));
```

`Gate::after` läuft *nach* dem Gate. Nach Laravels
`??=`-Semantik kann ein After-Hook ein unentschiedenes Ergebnis nur
**auffüllen** (kein Gate hat gematcht und kein Before-Hook hat
gefeuert) - er kann niemals ein bereits erzeugtes Allow/Deny
überschreiben. Jeder After-Hook läuft trotzdem, sodass er zugleich
als Nahtstelle für Audit-Logging dient:

```rust
Gate::after::<User>(|user, action, decided| {
    audit_log(user.id, action, decided);   // jede Auswertung beobachten
    None                                    // nur aufzeichnen; das Ergebnis nicht verändern
});
```

Hooks werden über den **Benutzertyp** `U` geschlüsselt, nicht über
die Ressource - ein Hook feuert für jedes `(action, U, R)`. Legen
Sie ressourcenspezifische Logik ins Gate. Hooks sind synchrone
Prädikate und gelten auch für den asynchronen Auswertungspfad; für
asynchrone Autorisierungslogik verwenden Sie `define_async` /
`define_async_with`.

### Warum Suprnova abweicht

Laravels `Gate::forUser($user)->allows(...)` bindet den *impliziten*
Resolver des Gates für den aktuellen Benutzer neu, sodass die
nächste Prüfung als dieser Benutzer ausgewertet wird. Suprnovas
Gate nimmt den Benutzer bei jedem Aufruf **explizit** entgegen,
sodass "als anderer Benutzer prüfen" einfach
`Gate::allows(action, &other_user, &resource)` ist. Es gibt keinen
impliziten Resolver, den man neu binden müsste - die explizite API
ist strikt allgemeiner, was `forUser` überflüssig statt fehlend
macht.

Dieselbe Überlegung gilt für Laravels automatische
Policy-Erkennung anhand des Klassennamens. Suprnova bindet
Policy-Methoden zur Registrierungszeit an den typgelöschten
`(action, U, R)`-Schlüssel, sodass eine `Post`-Policy und eine
`Comment`-Policy mit demselben Methodennamen zwei eigenständige
Gates registrieren, ohne eine Namenskonvention oder einen
Discovery-Scan zu benötigen.

## Nächste Schritte

- [Authentifizierung](authentication.md) - die benutzerseitige
  Hälfte: Guards, `Auth::user()`, `Auth::user_as::<T>()`
- [Application Bootstrap](bootstrap.md) - wo `init_policies()` in
  der Boot-Sequenz läuft, plus wie Sie Before-/After-Hooks
  registrieren
- [Middleware](middleware.md) - `AuthMiddleware` mit
  routenbasierter Autorisierung kombinieren
- [Fehlermodell](error-model.md) - wie eine Gate-Ablehnung in ein
  403, ein 404 oder einen benutzerdefinierten
  `FrameworkError::Domain`-Status kollabiert
- [Ereignisse](events.md) - auf Policy-Ergebnisse über
  `Gate::after` für Audit-Logging lauschen

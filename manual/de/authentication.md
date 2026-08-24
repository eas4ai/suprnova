# Authentifizierung

Suprnova liefert ein Laravel-förmiges Authentifizierungssystem:
eine statische `Auth`-Facade, benannte Guards, die über einen
`AuthManager` aufgelöst werden, austauschbare User-Provider, einen
`Authenticatable`-Trait auf Ihrem User-Modell und Middleware, die
Routen per Gate schützt. Ein gescaffoldetes Projekt bootet mit
einem Session-Guard (`web`) und einem Token-Guard (`api`), die
bereits gegen Ihren typisierten `User` verdrahtet sind, sodass
Login, Registrierung und geschützte Routen ab dem Tag
funktionieren, an dem Sie `suprnova new` ausführen.

## Die Bausteine

| Typ | Rolle |
|---|---|
| `Auth` | Framework-Facade für Guards sowie Magnetar-gestützte Passwort-, Magic-Link-, Passkey- und OAuth-Operationen |
| `MagnetarConfig` / `init_magnetar` | Stellt die Standard-Engines für Passwort, Session, Sperre, Passkey und Faktoren zusammen und installiert sie atomar |
| `Authenticatable` | Trait, den Ihr Anwendungsmodell implementiert; stellt `get_auth_identifier() -> String` und den Passwort-Hash bereit |
| `UserProvider` | Trait zum Abrufen von Anwendungsbenutzern; `EloquentUserProvider<M>` und `DatabaseUserProvider` sind integriert |
| `AuthManager` | Hält `AuthConfig` und die registrierten Provider; löst benannte Guards bei Bedarf auf |
| `SessionGuard` / `TokenGuard` | Stateful- und stateless-Guard-Verträge des Frameworks |
| `BearerTokenMiddleware` | Löst Magnetar-Bearer-Sessions in den Authentifizierungszustand der Framework-Anfrage auf |
| `AuthMiddleware` / `GuestMiddleware` / `BasicAuthMiddleware` | Routen-Guards |
| `Credentials` | JSON-artige Credential-Map, typischerweise `{ "email", "password" }` |

Code für Framework-Guards und -Provider liegt in `framework/src/auth/`. Die
Magnetar-Host-Adapter und -Facades liegen in `framework/src/magnetar_integration/`;
die Engine-Crate liegt in `crates/suprnova-magnetar/`. Höherstufige Flows für
E-Mail-Verifizierung, Passwort-Reset, Sperre und TOTP liegen in
`framework/src/auth_flows/` und werden in [Auth-Flows](auth-flows.md)
behandelt. OAuth-, Apple- und Magic-Link-Logins werden in
[OAuth und passwortloser Login](oauth.md) behandelt.

## Identifikator-Modell

Die ID des authentifizierten Benutzers fließt durch Suprnova
durchgängig als `String` - Session-Speicher,
[`UserProvider::retrieve_by_id`], die Remember-me-Tabelle, jedes
Auth-Event. Die kanonische Oberfläche ist
`Authenticatable::get_auth_identifier() -> String` (Laravels
`getAuthIdentifier`). Numerische Primärschlüssel lassen sich
trivial in einen String umwandeln; UUIDs, ULIDs und
undurchsichtige OAuth-Provider-IDs fließen unverändert durch.

```rust
use std::any::Any;
use suprnova::Authenticatable;

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
```

`get_auth_password` ist das, wogegen die eingebauten Provider ein
Klartext-Passwort über `hashing::verify_async` verifizieren.
Liefern Sie `None` für Benutzer, die sich auf andere Weise
authentifizieren (OAuth, Passkey, Magic Link). Die Methode
`auth_identifier_name() -> &'static str` (Standard `"id"`) benennt
die Spalte, in der die ID liegt. Die Komfortmethode
`auth_identifier() -> i64` parst standardmäßig die String-ID und
fällt für nicht-numerische IDs auf `0` zurück - Suprnova selbst
ruft sie nie auf; überschreiben Sie sie nur für ganzzahlig
geschlüsselte Modelle, die das Parsen überspringen wollen.

### Warum Suprnova abweicht

Laravels `getAuthIdentifier()` liefert `mixed`. PHP ist es
gleichgültig, ob die ID ein Int, ein UUID-String oder ein
stringartig typisierter Primärschlüssel aus einer Legacy-Tabelle
ist. Rust braucht einen einzigen konkreten Typ, auf den sich
Session, Provider und Events einigen. `String` ist die einzige
Wahl, die jede ID-Form aufnimmt, ohne das Framework zu zwingen zu
wissen, welche Ihre App verwendet. Die Integer-Komfortmethode
`auth_identifier()` existiert für den üblichen Fall, dass Ihre
Spalte ein `BIGINT` ist, aber das Framework hängt nie davon ab -
stellen Sie Ihren `User` morgen auf eine ULID um, und nichts im
Auth-Stack bemerkt es.

## Auth beim Boot verdrahten

Das Rust-Analogon zu `config/auth.php` ist eine `AuthConfig`, die
als `AuthManager`-Singleton im Container registriert wird, plus ein
unter einem Namen registrierter `UserProvider`. `bootstrap.rs`
erledigt typischerweise beides in zwei Zeilen:

```rust
use std::sync::Arc;
use suprnova::{App, Auth, AuthConfig, AuthManager, EloquentUserProvider};

use crate::models::user::User;

pub async fn bootstrap() -> Result<(), suprnova::FrameworkError> {
    // ... DB::init, SessionMiddleware-Installation usw.

    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register users provider");

    Ok(())
}
```

`AuthConfig::from_env()` liest den Standard-Guard aus `AUTH_GUARD`
(Standard `"web"`) und liefert von Haus aus zwei benannte Guards:
einen `web`-Session-Guard und einen `api`-Token-Guard, beide
gestützt auf den `"users"`-Provider. Apps, die mehr Guards brauchen
(einen separaten `admins`-Provider, getrennte zustandsbehaftete und
zustandslose Guards), bauen die Konfiguration explizit auf:

```rust
use suprnova::{AuthConfig, GuardConfig};

let config = AuthConfig::new("web")
    .guard("web", GuardConfig::session("users"))
    .guard("admin", GuardConfig::session("admins"))
    .guard("api", GuardConfig::token("users"));
```

## Magnetar-Engine initialisieren

Der API-Starter initialisiert Magnetar, nachdem Datenbank und `APP_KEY`
bereitstehen:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let magnetar = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(magnetar).await
}
```

Die Standard-Engine verwendet die SeaORM-Verbindung der Anwendung und erstellt
ihr Schema, sofern nicht `.apply_migrations(false)` gewählt wurde. Sie
installiert Passwort-/Session- und Passkey-Adapter atomar. Eine erneute
Initialisierung gibt einen Fehler zurück, statt einen Adapter zu ersetzen,
während eine andere Anfrage noch den alten Speicher nutzt.

`MagnetarConfig` akzeptiert außerdem Richtlinienwerte für Session, Sperre und
Zwei-Faktor-Authentifizierung:

```rust,ignore
let magnetar = MagnetarConfig::from_sea_orm(database)
    .session_config(session_policy)
    .lockout_config(lockout_policy)
    .two_factor_config(factor_policy)
    .passkey_config(passkey_policy);
```

Die Standard-Hostbindung verwendet die kanonische Tabelle `app_users` mit
`i64`-Anwendungs-IDs. Magnetars öffentliche `UserId` bleibt an der
Facade-Grenze opak; die Standardbindung parst den gespeicherten Bezeichner nur
beim Übergang in die Anwendungstabelle.

### Magnetar-gestützte Facade-Methoden

Die installierte Engine versorgt folgende Framework-eigene Methoden:

- `Auth::password().register(...)`.
- `Auth::password().authenticate(...)`.
- `Auth::magic_link().send(...)` und `.consume(...)`.
- `Auth::passkey().begin_registration(...)` und `.finish_registration(...)`.
- `Auth::passkey().begin_authentication(...)` und
  `.finish_authentication(...)`.
- `Auth::oauth(provider)`, wenn ein OAuth-Delegate installiert ist.
- Ausstellung, Rotation und Widerruf von Remember-me-Anmeldedaten.
- Bearer-Session-Lookup über `BearerTokenMiddleware`.
- `list_sessions`, `revoke_session` und `revoke_all_sessions` in
  `suprnova::magnetar_integration`.

Eine erfolgreiche Anmeldung rotiert die Framework-Session-ID und das CSRF-Token,
speichert die Anwendungsbenutzer-ID und protokolliert eine opake
Magnetar-Webbindung. Das Framework verantwortet weiterhin HTTP-Middleware,
Cookies, E-Mails, Events und seine Guard-/Provider-Verträge.

### Passwortauthentifizierung

Verwenden Sie die Magnetar-Passwort-Facade, wenn die Anwendung den integrierten
Pfad für Anmeldedaten, Sperre, Faktor-Gate und Session nutzen soll:

```rust,ignore
let user = Auth::password()
    .register("alice@example.com", password)
    .await?;

let (user, session) = Auth::password()
    .authenticate(
        "alice@example.com",
        password,
        request.header("User-Agent").map(str::to_string),
        request.peer_ip().map(str::to_string),
    )
    .await?;
```

`authenticate` gibt bei ungültigen Anmeldedaten, einer Sperre oder einem
erforderlichen zweiten Faktor HTTP-401-Fehler zurück. Speicher- und
Engine-Fehler bleiben Serverfehler. Die Methode gibt nie Passwortmaterial
zurück.

### Passkeys

Passkey-Aufrufe zum Beginnen und Abschließen benötigen `SessionMiddleware`,
weil der Selektor der Einmal-Zeremonie in der Framework-Session gespeichert
wird:

```rust,ignore
let challenge = Auth::passkey()
    .begin_authentication("alice@example.com")
    .await?;

let (user, session) = Auth::passkey()
    .finish_authentication("alice@example.com", browser_credential)
    .await?;
```

Die Registrierung folgt dem entsprechenden Paar aus `begin_registration` und
`finish_registration`. Die Registrierung für ein bestehendes Konto benötigt
einen verifizierten Anfrage-Akteur und eine kürzliche Reauthentifizierung über
den Plugin-Pfad; eine bloße Benutzer-ID in einer älteren Session wird nicht zu
einem Credential-Akteur hochgestuft.

### Erster E-Mail-Nachweis und Auth-Epochen

Magnetar behandelt den ersten erfolgreichen Nachweis eines Postfachs bei einem
nicht verifizierten Konto als atomare Credential-Grenze. Passwort-Reset, der
Verbrauch eines Magic Links und der Abschluss einer OAuth-verifizierten E-Mail
können diese Grenze gewinnen.

Die Transaktion erhöht die Auth-Epoche des Kontos, widerruft alte Sessions und
Remember-Anmeldedaten und entfernt provisorische Anmeldedaten, die ein Squatter
vor Eintreffen des Postfachinhabers registriert haben könnte. Schreibvorgänge
für Passwort, Passkey, verknüpfte Konten und Zwei-Faktor-Authentifizierung
tragen einen Akteur-Snapshot und schlagen fehl, wenn sich die Konto-Epoche
während des Vorgangs geändert hat.

Bei einem bereits verifizierten Konto bewahrt ein Passwort-Reset legitime
Passkeys, verknüpfte Konten und die bestätigte Zwei-Faktor-Registrierung,
während Passwort und Sessions weiterhin rotiert beziehungsweise ungültig
gemacht werden. OAuth verknüpft ein nicht verifiziertes bestehendes Konto nie
allein aufgrund einer E-Mail-Adresse des Providers automatisch; es erfordert
den Abschluss des E-Mail-Nachweises oder eine explizite Verknüpfung gemäß der
Hostrichtlinie.

### Direkte Oberfläche der Magnetar-Crate

Die meisten Anwendungen bleiben bei den Framework-Facades. Anwendungen, die
einen eigenen Identity-Host erstellen, können `suprnova-magnetar` direkt
verwenden für:

- Framework-neutrale Plugin-Routen und Effekt-Handler.
- Passwort- und Passwortverwaltungs-Plugins.
- Passkey- und Zwei-Faktor-Engines.
- OAuth-Autorisierung, Grants, Provider-Plugins, Geräteautorisierung und
  Token-Broker-Dienste.
- Engines für opake, JWT-, Remember- und Grant-Sessions.
- Eigene Speicher-Bindungen und das Standard-SeaORM-Schema.
- Formbewusste Migration von Auth-Daten.

Die direkte Verwendung überträgt Magnetar weder die Verantwortung für HTTP noch
für Anwendungsbenutzer. Der Host ordnet Wire-Anfragen, Mail-Effekte,
Anwendungs-IDs, Rate-Limit-Treiber und Session-Bindungen weiterhin seinem
eigenen Framework zu.

## Die `Auth`-Facade

Die statische `Auth`-Facade ist die Laravel-förmige Oberfläche, die
Sie aus Controllern und Middleware heraus aufrufen. Die
Credential- und benutzerbasierten Methoden delegieren an den
**Standard-Guard** (worauf auch immer `AuthConfig::default_guard`
zeigt, Standard `"web"`); die synchronen Lesevorgänge
`check`/`guest`/`id` sind der session-gestützte schnelle Pfad und
brauchen keinen Manager.

```rust
use suprnova::{Auth, Credentials};

// Credentials validieren und den Benutzer anmelden. Feuert Attempting →
// (Login + Authenticated), berücksichtigt Remember-me. Liefert den
// aufgelösten Benutzer, oder None bei falschen Credentials.
if let Some(user) = Auth::attempt(&Credentials::password(&email, &password), remember).await? {
    println!("Welcome, user {}", user.get_auth_identifier());
}

// Einen bekannten Benutzer direkt anmelden.
Auth::login(user, remember).await?;

// Über die ID anmelden, ohne Credentials erneut zu prüfen (z. B. nach gerade abgeschlossener Registrierung).
Auth::login_using_id(&id, remember).await?;

// Credentials validieren, ohne eine Session zu persistieren (Passwort-Bestätigungsdialoge).
let ok: bool = Auth::validate(&Credentials::password(&email, &password)).await?;

// Nur für diese Anfrage authentifizieren - kein Session-Schreibvorgang. Laravels `once`.
let ok: bool = Auth::once(&Credentials::password(&email, &password)).await?;
Auth::once_using_id(&id).await?;

// Session-gestützter schneller Pfad (kein AuthManager nötig).
if Auth::check()    { /* authentifiziert */ }
if Auth::guest()    { /* nicht authentifiziert */ }
if let Some(id) = Auth::id() { /* String-ID */ }

// Ob der aktuelle Benutzer bei dieser Anfrage über das Remember-me-Cookie
// authentifiziert wurde. Laravels `viaRemember()`.
if Auth::via_remember() { /* … */ }

// Den aktuellen Benutzer auflösen (über den registrierten Provider).
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
if let Some(user) = Auth::user_as::<User>().await? {
    println!("Welcome, {}!", user.name);
}

// Auth abbauen + Remember-me widerrufen + CSRF rotieren + Logout feuern.
Auth::logout().await?;

// Vollständige Session-Zerstörung (ID regenerieren + leeren + Remember-me widerrufen + Logout feuern).
Auth::logout_and_invalidate().await?;
```

`Auth::attempt` liefert bei Erfolg den aufgelösten Benutzer statt
eines bloßen `bool` - reichhaltiger als Laravels API, und spart den
nachfolgenden `Auth::user()`-Aufruf. `Ok(None)` bedeutet, dass die
Credentials keinen Benutzer aufgelöst haben; `Err` bedeutet einen
Datenbank-/Hashing-/Konfigurationsfehler, der nach oben
durchgereicht werden muss.

Wenn Sie die Identität eines Benutzers bereits selbst verifiziert
haben und nur die Session aufbauen möchten - etwa nach Abschluss
eines OAuth-Callbacks -, greifen Sie zur synchronen Primitive:

```rust
// Sync, kein Provider, kein AuthManager, keine Events. Liefert Err bei
// Aufruf außerhalb eines Anfrage-Scopes (keine SessionMiddleware
// installiert), sodass ein stillschweigend verworfener Login nie wie
// ein Erfolg aussehen kann.
Auth::login_id(user.id.to_string())?;
```

`login_id` regeneriert die Session-ID (verhindert
Session-Fixation) und rotiert den CSRF-Token, dann schreibt es die
ID in die Session. Es macht Fehlschläge absichtlich sichtbar:
frühere Versionen liefen außerhalb eines Session-Scopes
stillschweigend ins Leere, und das Audit hat das behoben - ein
"erfolgreicher Login", der nie ankam, ist genau die Art von Bug,
die sonst nichts abfängt.

## `Auth::user()` und `user_as<T>`

`Auth::user()` liefert den Benutzer hinter dem Trait:

```rust
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
```

Dieses Trait-Objekt deckt jeden ab, der `Authenticatable`
implementiert. Um Ihren konkreten `User` zurückzubekommen,
downcasten Sie ihn über `user_as::<T>()`:

```rust
use suprnova::Auth;
use crate::models::user::User;

if let Some(user) = Auth::user_as::<User>().await? {
    // Direkter Feldzugriff auf das Modell.
    println!("Welcome, {}!", user.name);
}
```

`user_as` liefert `Ok(None)` sowohl, wenn kein Benutzer
authentifiziert ist, *als auch*, wenn der aufgelöste Benutzer kein
`T` ist (z. B. ein `Auth::set_user(...)` eines anderen Typs an
anderer Stelle im Stack). Innerhalb einer Anfrage wird der Benutzer
pro Anfrage zwischengespeichert, sodass wiederholte Aufrufe von
`Auth::user()` den Provider nur einmal treffen.

## Benannte Guards

Die bloßen `Auth::*`-Methoden sprechen mit dem Standard-Guard. Um
gegen einen bestimmten Guard zu agieren, lösen Sie ihn anhand
seines Namens auf:

```rust
use suprnova::Auth;

// Nur-Lese-Operationen funktionieren mit jedem Treiber.
if Auth::guard("api")?.check().await? { /* … */ }

// Login/Logout/Attempt brauchen einen zustandsbehafteten Guard. Token-Guards scheitern hier sichtbar.
let user = Auth::stateful_guard("web")?
    .attempt(&credentials, false)
    .await?;
```

`Auth::guard("name")` liefert `Arc<dyn Guard>` (den Lese-Vertrag)
und `Auth::stateful_guard("name")` liefert
`Arc<dyn StatefulGuard>` (fügt `attempt`/`login`/`logout` hinzu).
Das Anfordern des zustandsbehafteten Vertrags auf einem Token-Guard
liefert einen Fehler mit einer Abhilfe-Nachricht, statt die API
stillschweigend einzuschränken.

## User-Provider

Ein `UserProvider` sagt dem Auth-Stack, wie er Benutzer holt und
validiert. Zwei Provider sind eingebaut, sodass der übliche Fall
keine eigene Implementierung braucht:

- **`EloquentUserProvider<M>`** - löst über ein typisiertes
  `#[suprnova::model]`-`User` auf, das auch `Authenticatable` ist.
  Schlägt bei IDs über den Primärschlüssel nach, bei Credentials
  über `email` (Standard).
- **`DatabaseUserProvider`** - löst eine rohe Tabelle anhand ihres
  Namens in einen `GenericUser` auf (ID + Attribut-Map). Verwenden
  Sie ihn, wenn Sie kein typisiertes Modell haben oder wollen.

Beide filtern Credential-Lookups gegen eine Allowlist (Standard
`["email"]`) - eine feindliche Credential-Map kann keine
zusätzlichen `WHERE`-Prädikate einschleusen. Passen Sie die
Allowlist mit `.credential_columns([...])` an, die Lookup-Spalte
mit `.identifier_column("uuid")`, oder die ID-Bindungsstrategie mit
`.with_id_parser(...)`.

Um eine eigene Quelle einzubinden (LDAP, eine externe API),
implementieren Sie `UserProvider` direkt. `retrieve_by_id` nimmt
den Identifikator als `&str` entgegen:

```rust
use async_trait::async_trait;
use std::sync::Arc;
use suprnova::{Authenticatable, FrameworkError, UserProvider};

struct LdapProvider;

#[async_trait]
impl UserProvider for LdapProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        // … aus LDAP holen, als Arc<dyn Authenticatable> zurückgeben
        Ok(None)
    }

    // retrieve_by_credentials + validate_credentials haben Trait-Standards,
    // die None / false liefern. Überschreiben Sie sie, um `Auth::attempt`
    // und `Auth::validate` gegen Ihre Quelle zu unterstützen.
}
```

Registrieren Sie ihn am Manager:

```rust
Auth::register_provider("ldap", Arc::new(LdapProvider))?;
```

## Routen schützen

### `AuthMiddleware`

Schützt Routen, die nur authentifizierten Benutzern offenstehen,
per Gate. Nicht authentifizierte Anfragen werden zu einer
Login-Seite weitergeleitet oder erhalten `401`:

```rust
use suprnova::{AuthMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/dashboard", controllers::dashboard::index)
        .post("/logout", controllers::auth::logout)
        .middleware(AuthMiddleware::redirect_to("/login"))
}
```

`AuthMiddleware::new()` liefert stattdessen `401 Unauthorized` - am
besten für JSON-APIs. `AuthMiddleware::redirect_to("/login")` gibt
für gewöhnliche Anfragen ein `302` aus und für Inertia-Anfragen ein
`409 X-Inertia-Location` (das der Inertia-Client in einen
vollständigen Seitenbesuch verwandelt). Um per Gate auf einen
bestimmten Guard einzuschränken, hängen Sie `for_guard` an:

```rust
// 401, außer der api-Guard ist authentifiziert.
.middleware(AuthMiddleware::new().for_guard("api"))
```

Ein Token-Guard (`for_guard("api")`) verlässt sich darauf, dass
irgendeine Bearer-Token-Middleware weiter vorn in der Chain die
Auth-ID der Anfrage befüllt; ohne sie meldet der Guard immer
"nicht authentifiziert".

### `GuestMiddleware`

Die Umkehrung - für Login- und Registrierungsseiten, die
authentifizierte Benutzer nicht sehen sollten:

```rust
use suprnova::{GuestMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/login", controllers::auth::show_login)
        .post("/login", controllers::auth::login)
        .get("/register", controllers::auth::show_register)
        .post("/register", controllers::auth::register)
        .middleware(GuestMiddleware::redirect_to("/dashboard"))
}
```

`GuestMiddleware::for_guard("name")` funktioniert genauso wie
`AuthMiddleware::for_guard`.

### `BasicAuthMiddleware`

HTTP-Basic-Auth aus dem `Authorization: Basic`-Header gegen den
Provider eines Guards:

```rust
use suprnova::BasicAuthMiddleware;

// Zustandsbehaftet - meldet den Benutzer bei Erfolg in der Session an (Laravels `basic`).
.middleware(BasicAuthMiddleware::new())

// Zustandslos - authentifiziert nur für diese Anfrage (Laravels `onceBasic`).
.middleware(BasicAuthMiddleware::once())
```

Der dekodierte Benutzername wird gegen das `field`-Credential
(Standard `"email"`) abgeglichen; ein fehlender, fehlerhafter oder
ungültiger Header liefert `401` mit einer
`WWW-Authenticate: Basic realm="..."`-Challenge. Konfigurieren Sie
mit `.field(...)`, `.realm(...)` und `.for_guard(...)`.

## Lifecycle-Events

Die Guards dispatchen fünf Lifecycle-Events. Lauschen Sie auf sie
über die [`EventFacade`](events.md):

| Event | Wann |
|---|---|
| `Attempting` | ein Credential-Versuch beginnt (`attempt`/`once`) |
| `Authenticated` | ein Benutzer wird bei dieser Anfrage aktiv authentifiziert (`login`/`once`/`once_using_id`) |
| `Login` | ein Benutzer wird in der Session persistiert (`login`/erfolgreiches `attempt`) |
| `Logout` | ein Benutzer wird abgemeldet |
| `Failed` | ein Credential-Versuch schlägt fehl (falsches Passwort oder unbekannte ID) |

Jedes Event trägt den Guard-Namen und eine String-Benutzer-ID - nie
das Klartext-Passwort und nie die rohe Credential-Map.
`Authenticated` feuert nur, wenn ein Benutzer aktiv etabliert wird,
nicht bei einer passiven `Auth::user()`-Auflösung aus einer
bestehenden Session, sodass Listener nicht bei jeder
authentifizierten Anfrage einen Strom von Duplikaten bekommen.

## Der gescaffoldete Login-Flow

`suprnova new` erzeugt einen Authentifizierungs-Controller, der
`Auth::attempt` gegen den registrierten Provider verwendet. `FormRequest` und
`Validate` erzeugen das Validierungs-Envelope `{ message, errors }`. Für eine
Inertia-Anfrage wandelt die installierte Validierungs-Redirect-Middleware diesen
Fehler in einen HTTP-`303 See Other`-Redirect zurück und flasht die Fehler für
die ursprüngliche Seite. Ein Nicht-Inertia-Client erhält das HTTP-`422
Unprocessable Entity`-JSON-Envelope:

```rust
use serde::Deserialize;
use suprnova::{
    handler, inertia_response, redirect, serde_json, Auth, Credentials,
    FormRequest, InertiaProps, Request, Response, Validate, ValidationErrors,
};

#[derive(InertiaProps)]
pub struct LoginProps {
    pub errors: Option<serde_json::Value>,
}

#[handler]
pub async fn show_login(req: Request) -> Response {
    inertia_response!(&req, "auth/Login", LoginProps { errors: None })
}

#[derive(Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email(message = "Please enter a valid email address"))]
    pub email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}

impl FormRequest for LoginRequest {}

fn invalid_credentials() -> suprnova::FrameworkError {
    let mut errs = ValidationErrors::new();
    errs.add("email", "These credentials do not match our records.");
    suprnova::FrameworkError::Validation(errs)
}

#[handler]
pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(
        &Credentials::password(&form.email, &form.password),
        form.remember,
    )
    .await?
    {
        Some(_user) => redirect!("/dashboard").into(),
        None => Err(invalid_credentials().into()),
    }
}

#[handler]
pub async fn logout(_req: Request) -> Response {
    Auth::logout().await?;
    redirect!("/").into()
}
```

Die Registrierung folgt derselben Form: das Formular validieren,
den Benutzer anlegen, dann meldet
`Auth::login(Arc::new(user), false).await?` den frisch angelegten
Benutzer in der Session an und feuert das `Login`-Event.

## Das gescaffoldete `User`-Modell

Der generierte `User` ist ein `#[suprnova::model]`, das
`Authenticatable` implementiert. Er enthält außerdem
`email_verified_at: Option<DateTime<Utc>>` und implementiert `MustVerifyEmail`
und `CanResetPassword`. Diese Brücken ermöglichen
`EloquentUserProvider<User>`, die E-Mail-Verifizierung zu markieren und die
Identitätsdaten für Passwort-Resets bereitzustellen. Der nachfolgende Ausschnitt
zeigt nur die Felder und Helfer für den Guard-Login; verwenden Sie für die
vollständige Auth-Flow-Implementierung das generierte Model-Template. Seine
Passwort-Helfer verwenden das Modul [`hashing`](hashing.md):

```rust
use chrono::{DateTime, Utc};
use suprnova::{attrs, hashing, model, Authenticatable, FrameworkError};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
    timestamps,
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    pub async fn find_by_email(email: &str) -> Result<Option<Self>, FrameworkError> {
        <Self as suprnova::eloquent::Model>::query()
            .filter("email", email)
            .first()
            .await
    }

    pub fn verify_password(&self, password: &str) -> Result<bool, FrameworkError> {
        hashing::verify(password, &self.password)
    }

    pub async fn create(
        name: impl Into<String>,
        email: impl Into<String>,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        let hashed = hashing::hash(password)?;
        <Self as suprnova::eloquent::Model>::create(attrs! {
            name: name.into(),
            email: email.into(),
            password: hashed,
        })
        .await
    }
}
```

Das Attribut `hidden = ["password", "remember_token"]` lässt das
Modell diese Spalten beim Serialisieren nach JSON für das Wire
überspringen - sie existieren auf der Struktur, sickern aber nie
durch eine Inertia-Response.

## Remember-me

Wenn eine Magnetar-Engine installiert ist, stellen `Auth::attempt(credentials,
true)` und `Auth::issue_remember_cookie` zweckgebundene
Magnetar-Remember-Anmeldedaten aus. Der Browser erhält weiterhin das
verschlüsselte Framework-Cookie `remember_me`, während Magnetar den
Verifier-Speicher, Prüfungen der Auth-Epoche, Einmalrotation,
Anomaliebehandlung und den Widerruf verantwortet.

Bei einer Anfrage ohne aktive Framework-Anmeldung verarbeitet
`SessionMiddleware` das Cookie über die installierte Engine, rotiert die
Remember-Anmeldedaten, stellt eine frische Magnetar-Session aus und bindet
beide Session-Ebenen. Eine veraltete Auth-Epoche, eine widerrufene Konto-Session,
fehlerhafte Anmeldedaten oder ein Replay authentifiziert die Anfrage nicht.

`Auth::revoke_remember_tokens()` macht jede Remember-Anmeldedaten des aktuellen
Benutzers ungültig. Das Cookie zum Löschen wird vor dem Widerruf im Backend
eingereiht, sodass der Browser seine Anmeldedaten auch bei einem fehlgeschlagenen
Speichervorgang verwirft.

Ohne Magnetar-Engine behält das Framework den älteren Fallback
`remember_tokens` aus Kompatibilitätsgründen bei. Neue Anwendungen sollten
Magnetar initialisieren, statt sich auf diesen Fallback zu stützen.

## Sicherheitsgarantien

Eine kurze Liste von Invarianten, die der Auth-Stack etabliert:

- **`Auth::login_id` scheitert außerhalb eines Anfrage-Scopes
  sichtbar.** Frühere Versionen ließen den Session-Schreibvorgang
  stillschweigend fallen; ein "erfolgreicher Login", der nie
  ankam, ist genau die Art von Bug, die sonst nichts abfängt.
- **Session-ID und CSRF-Token regenerieren sich bei jedem Login.**
  Sowohl `login_id` als auch das guard-gestützte `login`/`attempt`
  rotieren sie, um Session-Fixation zu verhindern.
- **Logout räumt den Auth-Zustand, bevor Remember-me widerrufen
  wird.** Schlägt der DB-Widerruf fehl, befindet sich die Session
  bereits in einem abgemeldeten Zustand, sodass ein veralteter
  Auth-Slot einen teilweisen Logout nicht überleben kann. Das
  Löschen des Remember-me-Cookies wird *vor* dem DB-Delete
  eingereiht, sodass der Browser das Cookie auch dann verwirft,
  wenn das Zeilen-Delete fehlschlägt (der Aufräum-Durchlauf holt
  das später nach).
- **Credential-Allowlists blockieren Injection.** Beide eingebauten
  Provider filtern `retrieve_by_credentials` gegen
  `credential_columns`, sodass zusätzliche Schlüssel in einer von
  einem Angreifer beeinflussten Credential-Map nicht zu
  zusätzlichen `WHERE`-Prädikaten werden können.
- **Schreibvorgänge für Anmeldedaten sind durch Akteure abgegrenzt.**
  Passwort-, Passkey-, Linked-Account-, Zwei-Faktor-, Session- und
  Remember-Mutationen tragen die Benutzer-ID und Auth-Epoche, die eine
  verifizierte Authentifizierung etabliert hat. Ein Widerruf oder eine
  Änderung der Epoche beim ersten Nachweis lässt einen laufenden veralteten
  Schreibvorgang fehlschlagen.
- **Der erste Nachweis eines Postfachs ist atomar.** Bei einem nicht
  verifizierten Konto erhöhen Passwort-Reset, der Verbrauch eines Magic Links
  oder der Abschluss einer OAuth-verifizierten E-Mail die Auth-Epoche und
  entfernen provisorische Anmeldedaten in derselben Transaktion. Ein
  gleichzeitiger Schreibvorgang eines Squatters kann nach dem Commit keinen
  Zugriff wiederherstellen.
- **E-Mail-Verifizierung ist akteurgebunden.** Die Verifizierungs-Facade des
  Frameworks verlangt einen authentifizierten Benutzer, dessen ID der
  Token-Eigentümer-ID entspricht. Ein Token für ein anderes Konto wird
  abgelehnt, ohne verbraucht zu werden.
- **Eine OAuth-E-Mail beweist keine Kontoinhaberschaft.** Ein nicht
  verifiziertes bestehendes Konto wird nie allein anhand einer Provider-E-Mail
  automatisch verknüpft. Verifizierte Konten benötigen eine explizite
  Verknüpfung; nicht verifizierte Konten benötigen den Abschlusspfad für den
  ersten E-Mail-Nachweis.
- **Auth-Events tragen nie Klartext.** Nur Guard-Name und String-Benutzer-ID.
  Die Verfolgung fehlgeschlagener Versuche (E-Mail-basierte Sperren) gehört zu
  `BruteForce` in [Auth-Flows](auth-flows.md), nicht zu den Lifecycle-Events.

Das Kapitel [Session](session.md) behandelt die Cookie-Konfiguration
(`SESSION_LIFETIME`, `SESSION_COOKIE`, `SESSION_SECURE`, `SESSION_SAME_SITE`
und `SESSION_COOKIE_PREFIX`), die session-basierte Guards übernehmen.

## Nächste Schritte

- [Auth-Flows](auth-flows.md) - E-Mail-Verifizierung, Passwort-Reset,
  Magnetar-gestützte Kontosperrung, TOTP-2FA-Framework und Auth-Flow-Events
- [OAuth und passwortloser Login](oauth.md) - Magnetar OAuth, Apple, Magic
  Links, Provider-Policy und Migration von Authentifizierungsdaten
- [Autorisierung](authorization.md) - `Gate`, Policies und `Authorizable`
- [Session](session.md) - die Browser-Session und die Cookie-Ebene
- [CSRF-Schutz](csrf.md) - Schutz vor zustandsändernden Anfragen
- [Hashing](hashing.md) - bcrypt- und Argon2-Helfer

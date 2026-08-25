# Auth-Flows

`suprnova::auth_flows` ist die Lifecycle-Schicht über der
[Authentifizierung](authentication.md). Während `auth::*` die Frage
„Wer stellt diese Anfrage?“ beantwortet, deckt `auth_flows::*` den Nachweis
des Postfachs, die Passwortwiederherstellung, Kontosperren und TOTP-Challenges
des Frameworks ab.

Der Namespace umfasst fünf Oberflächen:

- `EmailVerification` erstellt und verbraucht Framework-`auth_flow_tokens`,
  versendet E-Mails über die [`Mail`](mail.md)-Facade und markiert den
  authentifizierten Besitzer des Tokens über den konfigurierten `UserProvider`
  als verifiziert.
- `PasswordReset` verwendet die installierte Magnetar-Engine, sofern sie verfügbar ist. Ohne Magnetar können verifizierte Konten das Passwort über den konfigurierten `UserProvider` und die Framework-`auth_flow_tokens` zurücksetzen. Nicht verifizierte Konten werden sicher abgewiesen, da ein generischer Provider Magnetars atomare Richtlinie für den erstmaligen E-Mail-Nachweis nicht umsetzen kann.
- `BruteForce` und `LoginThrottleMiddleware` delegieren den Status der
  Kontosperrung an die installierte Magnetar-Engine.
- `TwoFactor` ist die Framework-eigene TOTP-Facade über
  `two_factor_credentials`. Sie bietet Registrierung, Bestätigung,
  Verifizierung, Wiederherstellungscodes, Secret-Rotation,
  Challenge-Promotion und Replay-Schutz auf Zeitschritt-Ebene.
- `remember_me` re-exportiert das ältere Remember-Modul des Frameworks aus
  Kompatibilitätsgründen. Wenn Magnetar installiert ist, verwenden die normalen
  Remember-Flows von `Auth` und `SessionMiddleware` stattdessen
  Magnetar-Anmeldedaten.

Zwei Route-Gate-Middlewares befinden sich im selben Namespace:

- `EnsureEmailVerifiedMiddleware` wird nach `AuthMiddleware` eingesetzt und
  sperrt Routen anhand von `email_verified_at`.
- `TwoFactorChallengeMiddleware` wird vor `AuthMiddleware` eingesetzt und
  leitet eine Session mit ausstehender Framework-TOTP-Challenge zum
  Challenge-Formular um.

Transaktionale Nachrichten verwenden stets die Framework-[`Mail`](mail.md)-Facade.
Magnetar stellt Security-Engines und Speicherverträge bereit; es installiert
keinen zweiten Mail-Transport für die Anwendung.

### Speicherorte des Zustands

E-Mail-Verifizierungs-Token liegen in der Framework-Tabelle
`auth_flow_tokens`; der Verifizierungszeitstempel wird über den konfigurierten
`UserProvider` geschrieben. Die Verifizierung ist an den Akteur gebunden: Der
aktuell authentifizierte Benutzer muss Eigentümer des Tokens sein.

Passwort-Reset-Token, Passwort-Anmeldedaten, Sperreinträge, opake Sessions,
Remember-Anmeldedaten, Passkey-Zeremonien, OAuth-Zeremonien und Auth-Epochen
gehören zur installierten Magnetar-Host-Engine. Passwort-Reset, Magic Link und
der Abschluss einer OAuth-verifizierten E-Mail teilen Magnetars atomare
Grenze für den ersten E-Mail-Nachweis, um nicht verifizierte Konten
wiederzuerlangen.

Die öffentliche `TwoFactor`-Facade dieses Kapitels behält ihr
Framework-eigenes `two_factor_credentials`-Schema. Magnetar hat außerdem eine
Faktor-Engine für die integrierten Passwort-, Magic-Link-, Passkey-, OAuth-
und Session-Flows. Gehen Sie nicht davon aus, dass beide Speicher austauschbar
sind: Verwenden Sie pro Anwendung durchgängig eine Registrierungsoberfläche.

Suprnova verantwortet weiterhin HTTP-Middleware, Cookies, ausgehende E-Mails,
Events und die `UserProvider`-Brücke. Anwendungscode verwendet
Framework-Facades, statt Speicher-Engines direkt aufzurufen.

## Fehlersemantik über die Flows hinweg

Jede Facade folgt einer Reihenfolgeregel: Zuerst wird die dauerhafte
Zustandsänderung festgeschrieben, danach werden Benachrichtigungs-Nebeneffekte
ausgelöst. Ein Panic in einem Listener, ein vorübergehender Fehler beim
Mail-Transport oder ein Dispatcher-Fehler nach der Mutation kann die Mutation
nicht zurückrollen.

- `EmailVerification::verify` verlangt den authentifizierten Token-Besitzer,
  verbraucht den Token und markiert den Benutzer als verifiziert, bevor
  `EmailVerified` ausgelöst wird.
- `PasswordReset::complete` führt den Commit über die installierte Magnetar-Engine aus, sofern sie verfügbar ist, einschließlich Richtlinie für den erstmaligen Nachweis, Fortschreibung der Auth-Epoche und atomarem Widerruf. Der Provider-Fallback gilt nur für verifizierte Konten: Er verbraucht das Framework-Token, rotiert das Provider-Passwort und meldet anschließend die Ergebnisse des Widerrufs von Framework-Sitzungen und Remember-Anmeldungen. E-Mails und Ereignisse werden danach verarbeitet.
- `BruteForce::unlock_account` schreibt die Entsperrung fest, bevor
  `AccountUnlocked` ausgelöst wird.
- `TwoFactor::confirm` setzt `confirmed_at`, bevor `TwoFactorEnrolled`
  ausgelöst wird; `TwoFactor::disable` löscht die Zeile, bevor
  `TwoFactorDisabled` ausgelöst wird; `TwoFactor::complete_challenge`
  befördert pending → authed, bevor das Standardpaar
  `auth::Login` + `auth::Authenticated` und anschließend
  `TwoFactorChallenged` dispatcht.

Ein Listener, der Dauerhaftigkeit benötigt, sollte seine Arbeit puffern
(etwa einen Job aus dem Listener-Body in die Queue stellen); die Facade selbst
wiederholt nie.

## Bootstrapping

Initialisieren Sie Magnetar nach `DB::init` und nachdem `APP_KEY` `Crypt`
initialisiert hat:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`init_magnetar` erstellt das Standard-Auth-Schema, sofern Migrationen nicht
deaktiviert sind, und installiert anschließend Passwort-/Session- und
Passkey-Adapter atomar. Ein zweiter Aufruf gibt einen Fehler zurück. Tests,
die eine prozessweite Installation benötigen, sollten eine eigene
Integrationstest-Binary verwenden, weil eine installierte Engine nicht
ersetzbar ist.

### E-Mail-Verifizierung

Für die E-Mail-Verifizierung benötigen Sie:

1. Einen registrierten `UserProvider`, der Benutzer über ihre E-Mail-Adresse
   abrufen und den Verifizierungszeitstempel setzen kann.
2. `MustVerifyEmail` auf dem Benutzer-Typ der Anwendung.
3. Eine nullable `email_verified_at`-Spalte.
4. Die Framework-Tabelle `auth_flow_tokens`.

```rust
use chrono::{DateTime, Utc};
use suprnova::MustVerifyEmail;

impl MustVerifyEmail for User {
    fn email(&self) -> &str {
        &self.email
    }

    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }

    fn set_email_verified_at(&mut self, value: Option<DateTime<Utc>>) {
        self.email_verified_at = value;
    }
}
```

Der Verifizierungs-Handler muss im Scope einer authentifizierten Session
ausgeführt werden. Ein gültiger Token für einen anderen Benutzer wird
abgelehnt, ohne verbraucht zu werden.

### Passwort-Reset und Sperrung

`BruteForce` erfordert die installierte Magnetar-Passwort-Engine. Die Passwortzurücksetzung bevorzugt diese Engine, aber `EloquentUserProvider<M>` unterstützt das Zurücksetzen für bereits verifizierte Benutzer, wenn `M` die Schnittstellen `MustVerifyEmail + CanResetPassword` implementiert. Nicht verifizierte Benutzer erhalten keinen Provider-gestützten Link zum Zurücksetzen. Installieren Sie Magnetar, um das Zurücksetzen als atomaren erstmaligen Postfachnachweis zu verwenden.

Der Passwort-Reset normalisiert eine unbekannte Adresse nur dann zu `Ok(())`,
wenn Abuse-Limiter, Mail-Konfiguration, Engine und Speicherprüfungen erfolgreich
sind. Die Pfade für bekannte und unbekannte Konten können sich weiterhin bei
Fehlern und der Ausführungszeit unterscheiden. Der Abschluss verwendet den
atomaren Speicher für den ersten E-Mail-Nachweis und gibt für Aufrufer, die den
Status von Session- oder Remember-Widerrufen explizit benötigen, ein
`PasswordResetOutcome` zurück.

### Die 2FA-Migrationen registrieren

Das Framework liefert das Schema; Ihre App entscheidet sich dafür,
indem sie beide Migrationen in ihrem eigenen Migrator aufführt:

```rust
use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... Ihre eigenen Migrationen ...

            // Erzeugt `two_factor_credentials`.
            Box::new(suprnova::auth_flows::two_factor::migration::Migration),
            // Fügt `last_used_timestep` für TOTP-Replay-Schutz hinzu.
            Box::new(suprnova::auth_flows::two_factor::migration_replay::Migration),
        ]
    }
}
```

Beide sind idempotent gegenüber einer bereits angewendeten
Datenbank (v1 verwendet `CREATE TABLE IF NOT EXISTS`; v2 ist ein
Spalten-Add). Das erneute Ausführen von `suprnova migrate` gegen
eine Produktionsdatenbank, die das Schema schon hat, ist ein No-op.

### Umgebung

Die transaktionalen Mailables lesen beim Versand zwei
Umgebungsvariablen:

| Var | Standard | Verwendet für |
|---|---|---|
| `APP_NAME` | `"Suprnova"` | Betreff-Branding und das `otpauth://`-Issuer-Label, das Authenticator-Apps anzeigen. |
| `MAIL_FROM` | keiner - **Fehler, wenn nicht gesetzt** | Umschlag-`From` auf jeder ausgehenden Nachricht. Auf eine verifizierte Absender-Domain setzen. |

`MAIL_FROM` hat absichtlich keinen Standard. Ein Standard wie ein
Platzhalter `noreply@example.com` würde DMARC / SPF in der
Produktion stillschweigend brechen und von einer Domain aus
versenden, die der Operator nicht kontrolliert, also schlägt die
Facade statt dessen fail-closed fehl. `EmailVerification::send_link`
und `PasswordReset::send_link` lassen den Fehler als `Err` zutage
treten; `PasswordReset::complete` protokolliert über
`tracing::warn!` und läuft weiter (die Passwortänderung hat
bereits committet, sodass der Benachrichtigungspfad sie nicht
zurückrollen kann).

Apps setzen zusätzlich `APP_URL`, damit Controller die Basis-URL
ableiten können, die in `send_link`-Aufrufen verwendet wird; die
Framework-Facade selbst nimmt die Basis-URL als Parameter entgegen.

Der Mail-Treiber wird separat über `MAIL_DRIVER` konfiguriert -
siehe die [Mail](mail.md)-Dokumentation.

## E-Mail-Verifizierung

`EmailVerification` prägt, prüft und löst Verifizierungs-Tokens
gegen die Tabelle `auth_flow_tokens` ein und markiert den Benutzer
über den konfigurierten Provider als verifiziert. Vier Operationen
decken den Lifecycle ab:

| Methode | Signatur | Anmerkungen |
|---|---|---|
| `send_link` | `send_link<U: MustVerifyEmail>(user: &U, base_url: &str) -> Result<()>` | Prägt + versendet Mail, bei einem bereits vorliegenden Benutzer. |
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | Normalisiert ein unbekanntes Provider-Ergebnis zu `Ok(())`; Token-Speicher- und Mail-Fehler geben weiterhin `Err` zurück, und die Ausführungszeit wird nicht angeglichen. |
| `check` | `check(token: &str) -> Result<bool>` | Nicht einlösend - sicher auf einer Landing-Page aufzurufen. |
| `verify` | `verify(token: &str) -> Result<String>` | An den Akteur gebunden und nur einmal verwendbar: Der authentifizierte Benutzer muss im Besitz des Tokens sein; bei Erfolg wird es verbraucht, markiert den Benutzer als verifiziert und gibt diese Benutzer-ID zurück. |

```rust
use suprnova::auth_flows::EmailVerification;

// Nach einer neuen Registrierung, wenn der frisch angelegte Benutzer vorliegt:
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// Optionale Prüfung auf der Landingpage - nicht verbrauchend, sodass ein
// Neuladen der Seite den Token nicht verbraucht.
let valid: bool = EmailVerification::check(&token_str).await?;

// Der Click-through-Handler läuft hinter der Authentifizierung. `verify`
// verbraucht den Token nur, wenn `Auth::id()` seinem Besitzer entspricht.
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` feuert bei Erfolg `EmailVerified` - Listener sind der
richtige Ort, um zusätzliche Funktionalität freizuschalten
(Willkommens-Mail, Standard-Follows, "Profil vervollständigen"-CTA),
ohne sie an den Verifizierungs-Handler zu koppeln. Das Event trägt
die Benutzer-ID des Providers.

### Der resend-Endpunkt (Anti-Enumeration)

`resend` nimmt nur die E-Mail-Adresse entgegen - die Facade schlägt
den Benutzer über den aktiven Provider nach und prägt, wenn ein
Konto vorliegt, einen Token und versendet die Mail; eine unbekannte
E-Mail-Adresse ist ein stilles No-op, das trotzdem `Ok(())`
liefert. Der Handler verzweigt nie selbst über die Existenz, sodass
ein sondierender Aufrufer nicht zwischen "gesendet" und "kein
solches Konto" unterscheiden kann:

```rust
use std::collections::HashMap;
use suprnova::auth_flows::EmailVerification;
use suprnova::{FrameworkError, HttpResponse, Request, Response};

pub async fn resend(req: Request) -> Response {
    resend_inner(req).await.map_err(HttpResponse::from)
}

async fn resend_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let email = params
        .get("email")
        .ok_or_else(|| FrameworkError::bad_request("missing email"))?;

    let base = format!(
        "{}/auth/verify",
        std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:8765".into()),
    );
    // `resend` führt den Lookup + die Anti-Enumeration intern durch.
    EmailVerification::resend(email, &base).await?;

    Ok(HttpResponse::text(
        "If this email is on file, a verification link has been sent.",
    ))
}
```

`send_link` und `resend` bauen beide die URL als
`{base_url}?token={plaintext_token}`. Ein abschließender
Schrägstrich bei `base_url` wird entfernt, bevor der Query-String
angehängt wird, sodass `https://app.example.com/verify/` und
`https://app.example.com/verify` beide eine saubere URL erzeugen.

Der Click-Through-Handler muss hinter `AuthMiddleware` ausgeführt werden. Er extrahiert das Token aus dem Query-String und ruft `verify` auf:

```rust
async fn verify_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let token = params
        .get("token")
        .ok_or_else(|| FrameworkError::bad_request("missing token"))?;

    let _user_id = EmailVerification::verify(token).await?;
    Ok(HttpResponse::new().status(302).header("Location", "/"))
}
```

`verify` prüft `Auth::id()` vor der Verwendung gegen den Token-Besitzer. Ein
Token, der einem anderen Konto gehört, liefert dieselbe 'invalid-token'-Antwort zurück und
bleibt ungenutzt. Im Erfolgsfall markiert der Provider den authentifizierten Besitzer als verifiziert
und die Facade löst `EmailVerified` aus.

### Nur-verifiziert-Routen: `EnsureEmailVerifiedMiddleware`

`EnsureEmailVerifiedMiddleware` macht Routen per Gate von
`email_verified_at` des authentifizierten Benutzers abhängig.
Setzen Sie sie nach `AuthMiddleware` ein, und die Chain blockiert
jede Anfrage, deren Benutzer den Verify-Schritt noch nicht
abgeschlossen hat.

Die Wahl zwischen **403 JSON** und **302-HTML-Redirect** wird zur
Routen-Registrierungszeit über den Konstruktor getroffen - es gibt
kein Schnüffeln im Anfrage-Content, passend zum Muster, das
`AuthMiddleware::new` / `AuthMiddleware::redirect_to` setzen:

```rust
use suprnova::{AuthMiddleware, EnsureEmailVerifiedMiddleware, group, get};

// API-Oberfläche - 403 mit einem JSON-Body.
group!("/api")
    .middleware(AuthMiddleware::new())
    .middleware(EnsureEmailVerifiedMiddleware::new())
    .routes([
        get!("/me", profile::show),
    ]);

// Web-Oberfläche - 302 (oder 409 + X-Inertia-Location für Inertia-Visits).
group!("/dashboard")
    .middleware(AuthMiddleware::redirect_to("/login"))
    .middleware(EnsureEmailVerifiedMiddleware::redirect_to("/email/verify"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

Ist kein Benutzer authentifiziert, fällt die Middleware in
denselben Response-Zweig wie "authentifiziert, aber nicht
verifiziert" - passend zur Form von Laravels
`! $request->user() || ! hasVerifiedEmail()`. Setzen Sie
`AuthMiddleware` zuerst ein, wenn Sie ein separates `401` für nicht
authentifizierte Anfragen wollen.

Für die Verzweigung innerhalb eines Handlers (z. B. das bedingte
Rendern eines "bitte verifizieren"-CTA ohne Weiterleitung), laden
Sie den typisierten Benutzer über den Session-Guard und lesen Sie
die Trait-Methode:

```rust
use suprnova::{Auth, MustVerifyEmail};
use crate::models::users::User;

if let Some(user) = Auth::user_as::<User>().await? {
    let verified: bool = user.is_email_verified();
    // darauf verzweigen
}
```

## Passwort-Reset

`PasswordReset` verfügt über vier Operationen:

| Methode | Signatur | Anmerkungen |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | Gegen Enumeration geschützte Ausgabe durch Magnetar; eine unbekannte Adresse führt erst nach erfolgreichen Voraussetzungsprüfungen still zu `Ok(())`. |
| `check` | `check(token: &str) -> Result<bool>` | Nicht verbrauchende Validierung durch die installierte Magnetar-Engine. |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | Verbraucht den Token atomar, wendet die Richtlinie für den ersten Nachweis an, rotiert Anmeldedaten, widerruft Sessions und Remember-Zustand und gibt die Benutzer-ID zurück. |
| `complete_with_outcome` | `complete_with_outcome(token, new_password) -> Result<PasswordResetOutcome>` | Führt dieselbe Transaktion aus und gibt die festgeschriebenen Widerrufszähler zurück. |

```rust
use suprnova::auth_flows::PasswordReset;

// Aus dem "Passwort vergessen"-Formular. `Ok(())` für eine unbekannte
// Adresse erst nach erfolgreichen Voraussetzungsprüfungen.
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// Optionale Landing-Page-Prüfung, bevor das Neues-Passwort-Formular gerendert wird.
let valid: bool = PasswordReset::check(&token).await?;

// Der Click-through-Handler, nachdem der Benutzer ein neues Passwort
// übermittelt hat: den Token einlösen + das Passwort rotieren, wobei
// die Benutzer-ID geliefert wird.
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` übergibt das Klartextpasswort über `SecretString`; Magnetar hasht
es innerhalb der Credential-Engine. Hashen Sie es nicht vorab. Ein leeres
Passwort oder ein Passwort, das nur aus Leerraum besteht, gibt HTTP 400 zurück,
bevor die Engine aufgerufen wird.

### Begrenztes Anti-Enumeration-Verhalten

`PasswordReset::send_link` gibt für eine unbekannte Adresse nur dann `Ok(())`
zurück, wenn Abuse-Limiter, Mail-Konfiguration, Engine und Speicherprüfungen
erfolgreich sind. Konfigurations-, Limiter-, Speicher- und Mail-Fehler geben
weiterhin `Err` zurück. Der Dogfood-Controller liefert erfolgreichen Requests
für bekannte und unbekannte Konten denselben HTTP-Status und Body, aber die
Implementierung gleicht deren Ausführungszeit nicht an.

### Nebeneffekte von `complete`

Magnetar committet den Passwort-Reset in einer einzigen Transaktion:

1. Den Einmal-Reset-Token verwenden.
2. Die First-Email-Proof-Policy anwenden, wenn das Konto noch nicht verifiziert ist.
3. Das Passwort hashen und ersetzen.
4. Die Authentifizierungs-Epoch vorantreiben.
5. Alte opake Sessions und Remember-Anmeldedaten widerrufen.
6. Provisorische Anmeldedaten entfernen, wenn dieser Reset der erste Postfach-Nachweis des Kontos ist.

Nach dem Commit sendet das Framework `PasswordChangedMail` und löst `PasswordResetCompleted` aus. Ein Fehler beim Mail-Versand oder beim Listener kann den Reset nicht rückgängig machen.

Bei einem bereits verifizierten Konto behält ein Reset legitime Passkeys, verknüpfte Konten und die bestätigte Zwei-Faktor-Registrierung bei. Bei einem unverifizierten Squatting-Konto entfernt der erste Nachweis provisorische Anmeldedaten, sodass der vorherige Registrant den Zugriff nicht beibehalten kann.

## Brute-Force-Schutz

Die Brute-Force-Schicht hat zwei Teile: die `BruteForce`-Facade,
die Lockout-Zustand aufzeichnet und abfragt, und die
`LoginThrottleMiddleware`, die auf der HTTP-Schicht per
Short-Circuit abbricht, bevor der Handler aufgerufen wird.

### Die `BruteForce`-Facade

Rufen Sie `record_failed_attempt` aus dem
Fehlgeschlagen-Auth-Zweig Ihres Login-Handlers auf, und
`reset_attempts` aus dem Erfolgs-Zweig:

```rust
use suprnova::auth_flows::BruteForce;

// Im Fehlgeschlagen-Auth-Pfad:
let status = BruteForce::record_failed_attempt(&email, Some(&peer_ip)).await?;
if status.is_locked {
    // Optional eine eigene Response nach außen geben. Die Middleware
    // erledigt das für Sie bei der *nächsten* Anfrage - siehe unten.
}

// Im Erfolgspfad:
BruteForce::reset_attempts(&email).await?;
```

`record_failed_attempt` liefert den aktualisierten `LockoutStatus`
(`is_locked`, `failed_attempts` und `locked_until`, wenn gesperrt).
Übergeben Sie die optionale `ip` für Audit-Logs; übergeben Sie
`None`, wenn Ihr Transport keine Client-IP sauber nach außen gibt.

Zwei zusätzliche Operationen:

```rust
// Nur lesend - sicher bei E-Mail-Adressen ohne Historie.
let status = BruteForce::get_lockout_status(&email).await?;
let locked: bool = BruteForce::is_locked(&email).await?;

// Admin-/erzwungene Entsperrung. Feuert `AccountUnlocked` nur bei
// einem echten Zustandsübergang (eine No-op-Entsperrung auf einem
// bereits entsperrten Konto feuert nicht).
let was_locked: bool = BruteForce::unlock_account(&email).await?;
```

`unlock_account` liefert `true`, wenn das Konto zum Zeitpunkt des
Aufrufs gesperrt war, sonst `false`. Das `AccountUnlocked`-Event
feuert nur bei `true` - eine `false`-Rückgabe ist genau das No-op,
das sie ist, kein Audit-Event.

### `LoginThrottleMiddleware`

Die Middleware liest den Lockout-Zustand für welche
E-Mail-Adresse auch immer eine Anfrage anvisiert, und bricht mit
`429 Too Many Requests` per Short-Circuit ab, wenn das Konto
gesperrt ist. Der Login-Handler wird nie aufgerufen, sodass ein
gesperrtes Konto nicht einmal zu einer Credentials-Prüfung kommt:

```rust
use suprnova::auth_flows::LoginThrottleMiddleware;
use suprnova::Router;

// Der E-Mail-Extraktor ist eine synchrone Closure über `&Request`.
// Das Lesen des JSON-/Formular-Bodys ist async und verbraucht
// `Request`, sodass die Closure den Body nicht lesen kann - stattdessen
// aus einem Header, Query-String oder Routen-Parameter holen.
let throttle = LoginThrottleMiddleware::new(|req| {
    req.header("X-Login-Email").map(str::to_string)
});

let router = Router::new()
    .post("/login", login_handler)
    .middleware(throttle);
```

Praktische Extraktions-Oberflächen:

- Ein Header (`X-Login-Email`), gesetzt von einem vorgeschalteten
  Pre-Prozessor - das in der Dogfood-App verwendete Muster.
- Ein Query-String-Parameter (`?email=…`).
- Ein Routen-Parameter (`/login/{email}`).

`None` aus dem Extraktor zurückzugeben ist das explizite Signal
"ich habe nichts zu prüfen" - die Middleware lässt die Anfrage
unverändert durch. Das macht es sicher, die Middleware auf Routen
zu installieren, die gelegentlich anonymen Traffic sehen (z. B.
denselben `POST /login`-Endpunkt, der auch eine E-Mail-lose
"Passwort-Reset anfordern"-Unteraktion behandelt).

Bei Sperrung liefert die Middleware:

- Status `429 Too Many Requests`.
- `Retry-After` Header - Sekunden, berechnet aus dem
  `locked_until` des Lockouts via `LockoutStatus::retry_after_seconds`. Fällt auf
  `900` zurück (15 Minuten, die Standard-Sperrzeit von Magnetar), falls der
  Zeitstempel fehlt.
- Body: `"Account locked due to too many failed login attempts. Try
  again later."`

### Backend-Fehler (standardmäßig Fail-Closed)

Wenn `get_lockout_status` einen Fehler zurückgibt, protokolliert
`LoginThrottleMiddleware` den Fehler und gibt standardmäßig HTTP
`503 Service Unavailable` mit `Retry-After: 1` zurück, ohne den Login-Handler
aufzurufen. Um den Login bei einem Ausfall des Lockout-Backends verfügbar zu
halten, optieren Sie explizit mit
`.on_backend_error(BackendErrorPolicy::FailOpen)` ein; nur diese Richtlinie
leitet die Anfrage an den Handler weiter.

### Schichtung mit `RateLimitMiddleware`

`LoginThrottleMiddleware` ist pro Konto - sie schützt eine
einzelne E-Mail-Adresse per Gate, wenn der Schwellenwert
überschritten wird. Für Quoten pro IP schichten Sie sie mit
[`RateLimitMiddleware`](rate-limiting.md). Die beiden setzen sich
auf natürliche Weise zusammen:

```rust
let router = Router::new()
    .post("/login", login_handler)
    .middleware(LoginThrottleMiddleware::new(|req| { /* ... */ }))
    .middleware(RateLimitMiddleware::ip_based(20, std::time::Duration::from_secs(60)));
```

Zusammen decken sie die realistischen Formen von Credential
Stuffing ab: verteilt (eine E-Mail-Adresse × viele IPs) ist Aufgabe
des Rate-Limits; fokussiert (viele Versuche × eine
E-Mail-Adresse) ist Aufgabe der Throttle-Middleware.

### Konfiguration

`MagnetarConfig` akzeptiert einen/eine `LockoutConfig`. Der Standardwert sind fünf fehlgeschlagene Versuche, eine 15-minütige Zähl- und Sperrfrist, eine sieben Tage dauernde Speicherung der Versuche sowie `BackendErrorPolicy::FailClosed`:

```rust,ignore
let config = MagnetarConfig::from_sea_orm(database)
    .lockout_config(lockout_policy);
```

Verwenden Sie `LockoutConfig::disabled()` nur, wenn eine andere Fail-Closed-Identitätskontrolle die Kontosperrung ersetzt.

## Zwei-Faktor (TOTP)

`TwoFactor` deckt TOTP-basierte 2FA ab - die Art, die sich mit
jeder standardkonformen Authenticator-App paart (Google
Authenticator, 1Password, Bitwarden, Authy). Der Flow ist
Enrollment → Bestätigung → laufende Verifizierung, plus
Single-Use-Recovery-Codes für den Fall, dass der Benutzer sein
Gerät verliert, plus der Challenge-Flow, der alles in den
Login-Lifecycle einfügt.

### Der Trait `TwoFactorUser`

Das Framework kann nicht in den Benutzer-Speicher Ihrer Anwendung
hineingreifen, also implementieren Aufrufer einen kleinen Trait,
um von ihrem Benutzermodell zur 2FA-Facade zu überbrücken:

```rust
use suprnova::auth_flows::TwoFactorUser;

pub trait TwoFactorUser: Send + Sync {
    fn user_id(&self) -> &str;
    fn email(&self) -> &str;
}
```

`user_id` ist ein opaker Speicherschlüssel. Er kann eine als Text dargestellte numerische Anwendungs-ID, eine UUID oder ein Magnetar `UserId` sein. Die TOTP-Tabelle des Frameworks besitzt keinen Fremdschlüssel zur Anwendungs-Benutzertabelle.

`email` wird in das `account_name`-Segment der `otpauth://`-URL eingebettet, sodass die Authenticator-App eine erkennbare Kontobezeichnung anzeigt.

```rust
use suprnova::auth_flows::TwoFactorUser;

struct AppUser2fa<'a> {
    user: &'a User,
}

impl TwoFactorUser for AppUser2fa<'_> {
    fn user_id(&self) -> &str {
        &self.user.auth_id
    }

    fn email(&self) -> &str {
        &self.user.email
    }
}
```

### Speicherung

Der 2FA-Zustand liegt in der frameworkeigenen Tabelle
`two_factor_credentials`. Secrets und Recovery-Codes sind ruhend
verschlüsselt mit `crate::crypto::Crypt::encrypt_string`, was einen
prozessglobalen `EncryptionKey` erfordert. Apps entscheiden sich
für das Schema, indem sie beide Migrationen in ihrem
`Migrator::migrations()` aufführen - siehe
[Bootstrapping](#bootstrapping).

### Enrollment, Bestätigung, Verifizierung

```rust
use suprnova::auth_flows::{TwoFactor, EnrollmentResponse};

// 1. Enrollment: ein frisches Secret + 10 Recovery-Codes erzeugen,
//    verschlüsselt persistieren, alles zurückgeben, was zum Rendern
//    des QR-Codes nötig ist.
let response: EnrollmentResponse = TwoFactor::enroll(&user_2fa).await?;
// response.otpauth_url - `otpauth://totp/...`-Deep-Link
// response.qr_code_svg - <svg>, das ein base64-PNG umschließt, inline einbetten
// response.recovery_codes - Vec<String>, 10 Klartext-Codes - NUR EINMAL anzeigen

// 2. Bestätigen: Der Benutzer öffnet die Authenticator-App und tippt
//    den 6-stelligen Code ein. `confirm` validiert ihn und stempelt
//    `confirmed_at`.
TwoFactor::confirm(&user_2fa, &user_typed_code).await?;
// feuert `TwoFactorEnrolled`

// 3. Bei nachfolgenden Logins die Session per Gate an `verify` binden:
let ok: bool = TwoFactor::verify(&user_2fa, &code_from_login_form).await?;
if !ok {
    return Err(suprnova::FrameworkError::domain("invalid 2FA code", 401));
}
```

`enroll` liefert Klartext-Recovery-Codes **genau einmal**. Es gibt
keine API, um sie später abzurufen - die verschlüsselte Spalte ist
von diesem Punkt an einseitig. Zeigen Sie sie auf der
Enrollment-Erfolgsseite, ermutigen Sie den Benutzer, sie zu
speichern, und speichern Sie den Klartext sonst nirgendwo.

`enroll` verweigert das Überschreiben eines **bestätigten**
Enrollments - es liefert ein `409`, um den Aufrufer zu `re_enroll`
zu drängen, das einen Besitznachweis verlangt. Ein erneutes
Enrollment auf einer unbestätigten (pending) Zeile ist erlaubt: Das
vorherige Enrollment wurde nie maßgeblich.

### Replay-Schutz

`verify` schreibt bei Erfolg den aktuellen TOTP-Zeitschritt nach
`last_used_timestep`. Nachfolgende Verifys, bei denen
`current_timestep <= last_used_timestep` gilt, werden abgelehnt,
selbst wenn der Code selbst strukturell gültig ist, was einen
Replay eines gestohlenen Codes innerhalb des 30-Sekunden-Fensters
vereitelt.

Der Zeitschritt-Anspruch ist atomar. Der Stempel landet über ein
bedingtes
`UPDATE … WHERE last_used_timestep IS NULL OR last_used_timestep <
:current`, und das Verify gelingt nur, wenn das Statement genau
eine Zeile betrifft. Zwei nebenläufige Verifys im selben
Zeitschritt können nicht beide gewinnen: Das erste kippt die
Spalte, das Prädikat des zweiten passt nicht mehr, und das zweite
wird als Replay behandelt. Ein einfaches Read-Modify-Write wäre
ein TOCTOU-Race - beide Verifys lesen die Zeile vor dem Stempeln,
beide validieren denselben Code, beide stempeln, beide gelingen.
Nebenläufige Teilnehmer eines solchen Race werden ebenfalls als
fehlgeschlagene Versuche gezählt, sodass der Brute-Force-Zähler sie
erfasst.

### Recovery-Codes

```rust
let consumed: bool = TwoFactor::consume_recovery_code(&user_2fa, &code).await?;
```

Single-Use: Ein passender Code wird aus der Zeile entfernt, bevor
der Aufruf zurückkehrt, sodass ein zweiter Versuch mit demselben
Code `false` liefert. Codes sind 12 Dezimalstellen in der Form
`NNNNNN-NNNNNN` (jeweils ~40 Bit Entropie, passend zum Format von
Laravel Fortify).

`consume_recovery_code` akzeptiert Codes nur, wenn 2FA vollständig
bestätigt ist - es bricht per Short-Circuit zu `Ok(false)` ab,
solange `confirmed_at` NULL ist. Ohne dieses Gate könnte ein
Angreifer, der ein Enrollment auf einem Opferkonto ausgelöst hat
(oder jeder Flow, der die Zeile ohne Bestätigung erzeugt), sich
allein mit einem frischen Recovery-Code authentifizieren und TOTP
dabei vollständig umgehen. Der Vertrag ist symmetrisch zur
Absicherung "nur bestätigtes Enrollment" von `verify`.

### Recovery-Codes und Secrets rotieren

Wenn ein Benutzer seine Recovery-Codes aufgebraucht hat oder sie
nach einem vermuteten Kompromittieren rotieren möchte:

```rust
let fresh: Vec<String> = TwoFactor::regenerate_recovery_codes(&user_2fa, &proof).await?;
```

`proof` muss entweder als aktueller TOTP-Code oder als unbenutzter
Recovery-Code validieren. Ohne die Proof-Prüfung könnte ein
Angreifer mit einer gekaperten Session die Recovery-Codes des
rechtmäßigen Benutzers stillschweigend wegblasen
(Denial-of-Service gegen die Kontowiederherstellung). Die frischen
Codes ersetzen die persistierte Menge; das bestehende Secret und
`confirmed_at` bleiben erhalten, sodass die Authenticator-App des
Benutzers ohne erneutes Pairing weiter funktioniert. Fehler:

- `400` - es existiert kein bestätigtes Enrollment; rufen Sie
  zuerst `enroll`/`confirm` auf.
- `401` - `proof` validiert weder als TOTP-Code noch als
  unbenutzter Recovery-Code.
- `429` - das Konto ist durch Brute-Force-Drosselung gesperrt.

Um das **Secret** zu rotieren (mit einem neuen Gerät neu zu
pairen), ohne 2FA zuerst zu deaktivieren:

```rust
let response = TwoFactor::re_enroll(&user_2fa, &proof).await?;
```

Dasselbe Proof-Modell wie `regenerate_recovery_codes`. Die Zeile
wird mit einem frischen Secret + 10 frischen Recovery-Codes neu
geschrieben; `confirmed_at` setzt sich auf NULL zurück, sodass der
Benutzer mit einem Code vom neuen Authenticator `confirm` aufrufen
muss, bevor 2FA wieder aktiv ist.

### Deaktivieren

```rust
TwoFactor::disable(&user_2fa).await?;
// feuert `TwoFactorDisabled` nur, wenn eine Zeile entfernt wurde
```

Idempotent: Ein Disable auf einem Benutzer, der nie ein Enrollment
durchgeführt hat, ist kein Fehler. Das `TwoFactorDisabled`-Event
feuert nur bei einem echten Zustandsübergang, sodass
Audit-Listener einen Eintrag pro tatsächlichem Disable sehen statt
einen pro Klick auf einen No-op-Button.

### Challenge-Flow (Login per Gate an den zweiten Faktor binden)

Die Primitiven enroll / confirm / verify sind die Bausteine; der
**Challenge-Flow** fügt sie in den Login-Lifecycle ein, sodass ein
Benutzer mit aktivierter 2FA geschützte Seiten nicht allein mit dem
Passwort erreichen kann.

Der Flow:

1. Der Passwort-Login löst einen Benutzer auf.
2. Wenn `TwoFactor::is_enabled_by_id(&user_id)` `true` liefert,
   ruft der Login-Handler
   `TwoFactor::start_challenge(user_id, remember)` auf - das
   hinterlegt die Benutzer-ID als **pending** in der Session,
   leert den vollständig authentifizierten Slot, widerruft jedes
   von `Auth::attempt` ausgestellte Remember-me-Cookie und merkt
   sich, ob der Benutzer sich für Remember-me entschieden hat,
   damit das Cookie nach Abschluss der Challenge erneut ausgestellt
   werden kann. `Auth::id()` liefert von diesem Punkt an `None`,
   bis die Challenge abgeschlossen ist.
3. Der Handler leitet zu einer `/two-factor-challenge`-Route
   weiter, die das Code-Formular anzeigt.
4. Der Challenge-POST-Handler ruft
   `TwoFactor::complete_challenge(code)` auf - verifiziert den Code
   (TOTP **oder** ein unbenutzter Recovery-Code, passend zum
   Challenge-Controller von Fortify), befördert Pending → Authed,
   rotiert die Session-ID (vereitelt Session-Fixation) und den
   CSRF-Token, stellt das Remember-me-Cookie erneut aus, wenn der
   Benutzer sich dafür entschieden hat, und dispatcht das
   Standardpaar der Lifecycle-Events `auth::Login` +
   `auth::Authenticated` plus das 2FA-spezifische
   `TwoFactorChallenged`.

```rust
use suprnova::auth_flows::TwoFactor;
use suprnova::{Auth, Authenticatable, Credentials, redirect};

pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(&Credentials::password(&form.email, &form.password), form.remember).await? {
        Some(user) => {
            let user_id = user.get_auth_identifier();
            if TwoFactor::is_enabled_by_id(&user_id).await? {
                // Auf "pending" herabstufen: Auth-Slot geleert, pending gesetzt,
                // Remember-me-Cookie widerrufen. Das Remember-Flag des Formulars
                // durchreichen, damit `complete_challenge` das Cookie bei Erfolg
                // erneut ausstellen kann.
                TwoFactor::start_challenge(user_id, form.remember).await?;
                redirect!("/two-factor-challenge").into()
            } else {
                redirect!("/dashboard").into()
            }
        }
        None => Err(invalid_credentials().into()),
    }
}

pub async fn complete(form: TwoFactorChallengeRequest) -> Response {
    let _user = TwoFactor::complete_challenge(&form.code).await?;
    // Session-ID + CSRF sind rotiert; Remember-me wurde erneut ausgestellt,
    // falls das ursprüngliche Login-Formular es gesetzt hat. Listener, die
    // an `auth::Login` / `auth::Authenticated` hängen, sahen einen normalen
    // Login.
    redirect!("/dashboard").into()
}
```

`complete_challenge` rotiert die Session-ID und den CSRF-Token als
Teil der Beförderung zu Authed. Das schließt den klassischen
Session-Fixation-Angriff, bei dem ein Angreifer einem Opfer eine
bekannte Session-ID unterschiebt, bevor es sich anmeldet - nach der
Rotation ist die untergeschobene ID tot, und nur die frisch
erzeugte ID trägt den authentifizierten Zustand. Der Vertrag
entspricht `Auth::login_id` / `Auth::login_using_id`, sodass sich
2FA-Logins in Bezug auf Session-Zustand und
Listener-Beobachtbarkeit nicht von Logins ohne 2FA unterscheiden
lassen.

Schützen Sie jede geschützte Routen-Gruppe mit
`TwoFactorChallengeMiddleware` per Gate, **vor** `AuthMiddleware`,
sodass eine ausstehende Session zur Challenge-Seite statt zur
Login-Seite umgeleitet wird:

```rust
use suprnova::{AuthMiddleware, TwoFactorChallengeMiddleware, group, get};

group!("/dashboard")
    .middleware(TwoFactorChallengeMiddleware::redirect_to("/two-factor-challenge"))
    .middleware(AuthMiddleware::redirect_to("/login"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

Die Challenge-Seite selbst (das GET, das das Formular rendert, das
POST, das `complete_challenge` aufruft) darf
`TwoFactorChallengeMiddleware` NICHT installieren - sie ist das
Ziel. Der POST-Handler prüft typischerweise auch vorab
`TwoFactor::pending_user_id().is_some()`, damit ein veralteter Link
die Verify-Logik nicht mit einer leeren Session erreicht.

`TwoFactor::cancel_challenge()` leert beide Pending-Slots, ohne
jemanden zu authentifizieren - verdrahten Sie es mit einem "zurück
zum Login"-Link auf der Challenge-Seite.

**Recovery-Code-Fallback.** `complete_challenge(code)` versucht
zuerst den TOTP-Pfad und fällt darauf zurück, einen Recovery-Code
einzulösen, sodass ein Benutzer, der seinen Authenticator verloren
hat, trotzdem hineinkommt. Jeder Recovery-Code ist Single-Use.

**Brute-Force-Verknüpfung.** Fehlgeschlagene Challenge-Codes
speisen den Brute-Force-Zähler pro Konto über
`BruteForce::record_failed_attempt`, genauso wie es das bloße
`TwoFactor::verify` tut. Ein Angreifer, der das Challenge-Formular
durchackert, löst nach dem konfigurierten Schwellenwert
`AccountLocked` aus. Eine einzelne fehlerhafte Übermittlung zählt
als **ein** fehlgeschlagener Versuch, obwohl `complete_challenge`
intern sowohl den TOTP- als auch den Recovery-Code-Pfad versucht -
die stillen Validierungskerne überspringen den
Brute-Force-Zähler, sodass die äußere Schicht den kanonischen
Versuch genau einmal aufzeichnet.

**Lockout-Gate.** `complete_challenge` prüft vorab
`BruteForce::is_locked` und liefert `429 Too Many Requests`, wenn
das Konto bereits gesperrt ist - selbst wenn der übermittelte Code
korrekt ist. Ohne dieses methodeninterne Gate könnte ein Angreifer,
der die Sperrung ausgelöst hat, trotzdem hineinkommen, indem er bei
der nächsten Anfrage den richtigen Code übermittelt: Der
Brute-Force-Zähler ist über die E-Mail-Adresse des Benutzers
geschlüsselt, aber `verify` selbst konsultiert ihn nicht. Der
Passwort-Pfad erzwingt über `LoginThrottleMiddleware` denselben
Zwang auf der Routen-Schicht; sie vor die Challenge-POST-Route zu
setzen, ist unbedenklich - beide Gates sind idempotent.

**Fehlschlags-Event.** `complete_challenge` dispatcht bei einem
falschen Code (oder einem gesperrten Konto)
`TwoFactorChallengeFailed { user_id }`, getrennt vom
`auth::Failed` des Passwort-Pfads. Listener, die auf "Benutzer hat
2FA versucht und ist gescheitert" achten, abonnieren das neue
Event; Listener, die auf "Passwort hat nicht authentifiziert"
achten, bleiben bei `auth::Failed`. Die beiden Oberflächen werden
getrennt gehalten, damit ein 2FA-Vertipper für Audit-Pipelines
nicht wie ein Passwort-Fehlschlag aussieht.

### Warum Suprnova abweicht

Das Framework TOTP `user_id` ist ein `String`. Ein fester `i64`-, UUID- oder Magnetar-Identifikatortyp würde die wiederverwendbare Fassade an ein einzelnes Anwendungsschema binden. Die String-Grenze ermöglicht es einer App, jeden stabilen Identifikator auf Kosten einer Konvertierung an der Aufrufstelle zu wählen.

Das integrierte Faktor-Gate von Magnetar ist von dieser beibehaltenen Facade getrennt. Die
Trennung bewahrt die Kompatibilität für Anwendungen, die
`two_factor_credentials` verwenden, aber Anwendungen sollten nicht dasselbe Konto
über beide Stores registrieren.

## Remember-me

`suprnova::auth_flows::remember_me` re-exportiert zur Kompatibilität das
ältere Modul `suprnova::auth::remember`.

Wenn Magnetar installiert ist, verwenden `Auth::attempt(..., true)`,
`Auth::issue_remember_cookie` und die Hydrierung durch `SessionMiddleware`
zweckgebundene Remember-Anmeldedaten von Magnetar. Magnetar speichert
Verifier-Digests, prüft die Auth-Epoche, rotiert Anmeldedaten bei erfolgreicher
Verwendung, widerruft sie zusammen mit der Benutzer-Session und meldet Replay-
oder fehlerhafte Anmeldedaten, ohne das Geheimnis offenzulegen.

Das Browser-Cookie bleibt Eigentum des Frameworks. Es wird unter dem logischen
Namen `remember_me` verschlüsselt, folgt `SESSION_COOKIE_PREFIX` und wird vor
dem Widerruf im Backend gelöscht, damit ein Speicherfehler nicht dazu führt,
dass der Browser die alten Anmeldedaten weiter sendet.

Die ältere Implementierung mit Datenbankzeilen bleibt verfügbar, wenn keine
Magnetar-Engine installiert ist. Neue Anwendungen sollten Magnetar
initialisieren und den älteren Re-Export als Übergangsoberfläche behandeln.

## Ereignisse

Neun Events feuern über die Flows hinweg, eines pro
Sicherheitszustandsübergang:

| Event | Gefeuert von | Trägt |
|---|---|---|
| `EmailVerified` | `EmailVerification::verify` bei Erfolg | `user_id: String` |
| `PasswordResetLinkSent` | `PasswordReset::send_link` bei Erfolg - anti-enumerierend still für fehlende E-Mail-Adressen | `user_id: String`, `email: String` |
| `PasswordResetCompleted` | `PasswordReset::complete` bei Erfolg | `user_id: String` |
| `AccountLocked` | `BruteForce::record_failed_attempt` beim Übergang entsperrt → gesperrt | `email: String`, `failed_attempts: u32` |
| `AccountUnlocked` | `BruteForce::unlock_account`, wenn eine tatsächliche Entsperrung stattfand | `email: String` |
| `TwoFactorEnrolled` | `TwoFactor::confirm` bei Erfolg | `user_id: String` |
| `TwoFactorChallenged` | `TwoFactor::complete_challenge` befördert Pending → Authed | `user_id: String` |
| `TwoFactorChallengeFailed` | `TwoFactor::complete_challenge` lehnte einen falschen Code ab oder verweigerte ein gesperrtes Konto | `user_id: String` |
| `TwoFactorDisabled` | `TwoFactor::disable`, wenn eine Zeile tatsächlich entfernt wurde | `user_id: String` |

Jedes Event ist `Debug + Clone + 'static`, trägt keine sensiblen
Daten (keine Klartext-Tokens, keine IPs) und verwendet
stringartige Identifikatoren, sodass Listener sie über
Task-Grenzen hinweg serialisieren können, ohne Typinformationen
aus dem Benutzer-Speicher-Backend durchsickern zu lassen.

### Lauschen

Abonnieren Sie über die Standard-Event-API - dieselbe Oberfläche
wie jedes andere In-Process-Event:

```rust
use std::sync::Arc;
use suprnova::async_trait;
use suprnova::auth_flows::events::AccountLocked;
use suprnova::{EventFacade, FrameworkError, Listener};

pub struct PageOpsOnLockout;

#[async_trait]
impl Listener<AccountLocked> for PageOpsOnLockout {
    async fn handle(&self, event: &AccountLocked) -> Result<(), FrameworkError> {
        tracing::warn!(
            email = %event.email,
            failed_attempts = event.failed_attempts,
            "account locked - paging ops",
        );
        // ... Slack-Benachrichtigung, Audit-Tabellen-Append usw.
        Ok(())
    }
}

// In bootstrap.rs:
EventFacade::listen::<AccountLocked, _>(Arc::new(PageOpsOnLockout)).await;
```

Listener laufen auf Tokios Runtime und werden in
Registrierungsreihenfolge dispatcht. Siehe das Kapitel
[Ereignisse](events.md) für die vollständige Oberfläche.

## Testen

Drei Fakes decken die Auth-Flows-Oberfläche ab, und sie lassen
sich kombinieren.

### `Mail::fake()`

Installiert einen prozesslokalen Capture-Transport. Jeder Versand
während der Lebensdauer des Guards landet in einem
In-Memory-Puffer, statt hinauszugehen:

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn send_link_dispatches_email() {
    let fake = Mail::fake();
    // ... den Flow durchspielen ...
    EmailVerification::send_link(&user, "https://app.example.com/verify")
        .await
        .unwrap();
    fake.assert_sent(|m| {
        m.to.iter().any(|a| a.email == "alice@example.com")
            && m.subject.contains("Verify")
    });
    fake.assert_sent_count(1);
}
```

`MailFake` legt `assert_sent`, `assert_not_sent`,
`assert_sent_count` frei, plus die rohen Accessoren `captured()`
und `count()`. Wenn der Guard gedroppt wird, wird der zuvor
gebundene Transport wiederherstellt - Tests, die Fakes mit
expliziter Transport-Bindung verschachteln, lassen keinen Zustand
durchsickern.

### `EventFacade::fake()`

Dieselbe Form, aber für Events:

```rust
use suprnova::auth_flows::events::EmailVerified;
use suprnova::events::testing::assert_dispatched;
use suprnova::EventFacade;

#[tokio::test]
async fn verify_fires_email_verified_event() {
    let _guard = EventFacade::fake();
    // ... den Flow durchspielen ...
    EmailVerification::verify(&token).await.unwrap();
    assert_dispatched::<EmailVerified>(|e| !e.user_id.is_empty());
}
```

Der Fake zeichnet dispatchte Events auf, ohne Listener aufzurufen,
sodass ein Listener, der mit einem externen Dienst spricht,
während des Tests nicht feuert. Das Begleitstück
`assert_not_dispatched::<E>(pred)` prüft das Negativ;
`dispatched_count::<E>(pred)` liefert die rohe Anzahl für
feingranularere Assertions.

### Integrationstests für E-Mail-Verifizierung und Passwort-Reset

Tests der E-Mail-Verifizierung erstellen `auth_flow_tokens`, registrieren einen
`UserProvider`, legen den authentifizierten Besitzer des Tokens fest, setzen
`MAIL_FROM` und steuern die Facade unter `Mail::fake()`.

Passwort-Reset-Tests installieren einen Testadapter für
`MagnetarPasswordAuthEngine` und prüfen Ausstellung, nicht verbrauchende
Prüfung, atomaren Abschluss, Session-Widerruf und Einmalverhalten.

Maßgebliche Quellbeispiele sind:

- `framework/tests/email_verify.rs` für akteurgebundene Verifizierung und
  Einmal-Token.
- `framework/tests/password_reset.rs` für die Magnetar-Delegierung und
  Abschluss-Ergebnisse.
- `framework/tests/magnetar_default_engine.rs` für die Einrichtung der
  tatsächlichen Standard-Engine.
- `framework/tests/brute_force.rs` für den Lifecycle der Sperre.
- `framework/tests/two_factor_challenge_flow.rs` für den beibehaltenen
  TOTP-Challenge-Flow des Frameworks.
- `framework/tests/magnetar_remember_middleware.rs` für Remember-Rotation und
  die Bindung an zwei Sessions.

Die prozessweite Magnetar-Installation ist absichtlich nur einmal möglich.
Legen Sie Tests, die verschiedene Engines benötigen, in getrennte
Integrationstest-Binaries, oder installieren Sie einen Testadapter einmal für
das gesamte Binary.


## Referenz

| Symbol | Zweck |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`, `resend`, `check` und akteurgebundenes `verify`; `verify` gibt die Benutzer-ID zurück. |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` für 403-JSON und `redirect_to(path)` für Browser- oder Inertia-Redirects. |
| `suprnova::auth_flows::PasswordReset` | Magnetar-zentrierte Zurücksetzung mit einem `UserProvider`-Fallback für verifizierte Konten über Framework-`auth_flow_tokens`. |
| `suprnova::MustVerifyEmail` | Anwendungsnutzer-Contract für die Framework-Verifizierungs-Facade. |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | SeaORM-Tabellendefinition für Framework-Verifizierungs-Token. |
| `suprnova::auth_flows::BruteForce` | Auf Magnetar basierende Account-Lockout-Facade. |
| `suprnova::auth_flows::LoginThrottleMiddleware` | HTTP-Middleware, die einen 429 zurückgibt, bevor der Login-Handler aufgerufen wird, wenn das Konto gesperrt ist. |
| `suprnova::auth_flows::TwoFactor` | Beibehaltene TOTP-Enrollment-, Verifizierungs-, Recovery- und Challenge-Facade des Frameworks. |
| `suprnova::auth_flows::TwoFactorUser` | Application-User-Bridge für die TOTP-Facade des Frameworks. |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | Gate für Sitzungen, die auf die TOTP-Challenge des Frameworks warten. |
| `suprnova::auth_flows::remember_me` | Kompatibilitäts-Reexport des Legacy-Framework-Remember-Moduls. |
| `suprnova::MagnetarConfig` / `suprnova::init_magnetar` | Standard-Magnetar Engine-Konfiguration und One-Shot-Installation. |
| `suprnova::auth_flows::events::*` | Ereignisse des Authentifizierungs-Lebenszyklus. |

## Nächste Schritte

- [Authentifizierung](authentication.md) - Guards, Provider, die
  `Auth`-Facade, `AuthMiddleware`.
- [Mail](mail.md) - die Transportschicht, über die die
  `send_link`-Aufrufe dispatchen.
- [Ereignisse](events.md) - Listener für die neun
  Auth-Flow-Events registrieren.
- [Ratenbegrenzung](rate-limiting.md) -
  `RateLimitMiddleware::ip_based` mit `LoginThrottleMiddleware` für
  gestaffelte Verteidigung kombinieren.
- [Sitzungen](session.md) - was `start_challenge` /
  `complete_challenge` berühren, wenn sie die Session-ID rotieren.

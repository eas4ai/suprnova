# Auth-Flows

`suprnova::auth_flows` ist die Lifecycle-Schicht über der
[Session-Authentifizierung](authentication.md). Wo `auth::*` die
Frage "wer stellt diese Anfrage" beantwortet, beantwortet
`auth_flows::*` alles rund um diese Frage - zu belegen, dass die
E-Mail-Adresse echt ist, sie wiederherzustellen, wenn das Passwort
verloren ist, sie gegen Credential Stuffing zu verteidigen und sie
mit einem zweiten Faktor zu schützen. Fünf Flows liefern unter
einem Namespace:

- `EmailVerification` - prägt, prüft und löst Single-Use-
  Verifizierungs-Tokens ein; `send_link` / `resend` versenden die
  Verifizierungs-Mail über die [`Mail`](mail.md)-Facade, und
  `verify` markiert den Benutzer als verifiziert über den
  konfigurierten User-Provider.
- `PasswordReset` - anti-enumerierendes `send_link`, nicht
  einlösendes `check`, und `complete`. `complete` rotiert das
  Passwort über den konfigurierten User-Provider, widerruft jede
  Session- und Remember-me-Zeile des Benutzers und versendet eine
  `PasswordChangedMail`-Sicherheitsbenachrichtigung.
- `BruteForce` + `LoginThrottleMiddleware` - torii-gestützter
  Lockout-Zustand plus eine HTTP-Middleware, die mit
  `429 Too Many Requests` per Short-Circuit abbricht, bevor der
  Login-Handler aufgerufen wird.
- `TwoFactor` - TOTP-Enrollment, Bestätigung, Verifizierung,
  Recovery-Codes, Secret-Rotation, der vollständige Challenge-Flow,
  der einen Passwort-Login per Gate vom zweiten Faktor abhängig
  macht, und Replay-Schutz mit einer Granularität von
  30-Sekunden-Zeitschritten.
- `remember_me` - Re-Export von `crate::auth::remember`
  (DB-Zeile + bcrypt + Single-Use-rotierende persistente Cookies)
  aus Gründen der Namespace-Kohäsion.

Zwei Route-Gate-Middlewares liefern im selben Namespace:

- `EnsureEmailVerifiedMiddleware` - setzt sich nach
  `AuthMiddleware` zusammen, um Routen per Gate von
  `email_verified_at` abhängig zu machen.
- `TwoFactorChallengeMiddleware` - setzt sich vor `AuthMiddleware`,
  um eine Session mit einer ausstehenden 2FA-Challenge zum
  Challenge-Formular statt zur Login-Seite umzuleiten.

Jede transaktionale Nachricht wird über die
[`Mail`](mail.md)-Facade zugestellt. Toriis optionales
`mailer`-Feature ist in `framework/Cargo.toml` absichtlich
deaktiviert: Einen zweiten Mail-Stack innerhalb von torii laufen zu
lassen würde die Telemetrie aufspalten, die
Transport-Konfigurationsoberfläche verdoppeln und Apps zwingen,
zwei "From"-Adressen zu verdrahten.

### Wo der Zustand liegt

E-Mail-Verifizierung und Passwort-Reset sind
**provider-agnostisch**. Verifizierungs- und Reset-Tokens liegen
in der eigenen Tabelle `auth_flow_tokens` des Frameworks
(Single-Use, SHA-256-gehasht), und der Benutzer-Lookup + die
Mutation laufen über welchen [`UserProvider`](authentication.md)
auch immer die App registriert hat - denselben Provider, gegen den
`Auth::user` auflöst. Für diese beiden Flows gibt es keine globale
Auth-Instanz zu initialisieren: Eine frisch gescaffoldete App hat
bereits `EloquentUserProvider<User>` gebunden, und das ist alles,
was `EmailVerification` und `PasswordReset` brauchen.

Torii besitzt weiterhin den Sicherheitszustand für die Flows, die
tatsächlich davon abhängen - den Brute-Force-Lockout-Zähler pro
Konto, OAuth-/Passkey-/WebAuthn-Ceremonies und den Session-Pool.
Suprnova besitzt die querschnittlichen Belange über jeden Flow
hinweg - ausgehende Mail, Event-Dispatch, die 2FA-TOTP-Tabelle,
Remember-me-Cookies und die HTTP-Middleware. Anwendungscode berührt
immer nur `suprnova::auth_flows::*`. Laravel faltet die
entsprechende Oberfläche in Fortify; Suprnova hält die Modell-Traits
(`MustVerifyEmail` / `CanResetPassword`) und den Token-Speicher im
Framework, sodass die Flows gegen jedes Benutzer-Backend
funktionieren.

## Fehlersemantik über die Flows hinweg

Jede Facade folgt einer Reihenfolge-Regel: Die dauerhafte
Zustandsänderung committet zuerst, dann feuern die
Benachrichtigungs-Nebeneffekte. Ein Listener-Panic, ein
vorübergehender Mail-Transport-Fehler oder ein Dispatcher-Fehler
nach der Mutation können die Mutation nicht zurückrollen.

- `EmailVerification::verify` löst den Token ein und markiert den
  Benutzer über den Provider als verifiziert, bevor es
  `EmailVerified` feuert.
- `PasswordReset::complete` löst den Token ein und rotiert das
  Passwort zuerst über den Provider, widerruft dann jede Session-
  und Remember-me-Zeile des Benutzers (bei Fehlschlag
  protokolliert, nicht nach außen sichtbar), dispatcht dann
  `PasswordChangedMail` fire-and-forget, und feuert dann
  `PasswordResetCompleted`.
- `BruteForce::unlock_account` committet die Entsperrung, bevor es
  `AccountUnlocked` feuert.
- `TwoFactor::confirm` stempelt `confirmed_at`, bevor es
  `TwoFactorEnrolled` feuert; `TwoFactor::disable` löscht die
  Zeile, bevor es `TwoFactorDisabled` feuert;
  `TwoFactor::complete_challenge` befördert Pending → Authed,
  bevor es das Standardpaar `auth::Login` + `auth::Authenticated`
  gefolgt von `TwoFactorChallenged` dispatcht.

Ein Listener, der Dauerhaftigkeit braucht, sollte seine Arbeit
puffern (einen Job aus dem Listener-Rumpf einreihen); die Facade
selbst wiederholt nie.

## Bootstrapping

E-Mail-Verifizierung und Passwort-Reset sind provider-gestützt und
brauchen **kein torii**. Brute-Force-Schutz und 2FA brauchen
weiterhin torii. Verdrahten Sie, was die von Ihnen verwendeten
Flows brauchen - sie sind unabhängig.

### E-Mail-Verifizierung + Passwort-Reset

Drei Dinge, die eine gescaffoldete App bereits alle hat:

1. **Ein User-Provider, der die Auth-Flow-Oberfläche
   implementiert.** Registrieren Sie `EloquentUserProvider<User>`
   (denselben Provider, gegen den `Auth::user` auflöst) als
   `dyn UserProvider`-Bindung in `bootstrap.rs::register()`. Beide
   Facades lösen den aktiven Provider intern auf; an der
   Aufrufstelle wird keine Instanz übergeben.

   ```rust
   use suprnova::{bind, EloquentUserProvider};
   use suprnova::auth::UserProvider;
   use crate::models::users::User;

   bind!(dyn UserProvider, EloquentUserProvider::<User>::new());
   ```

2. **Die beiden Modell-Traits auf Ihrem `User`.**
   `EloquentUserProvider<User>` implementiert die
   Auth-Flow-Methoden (`retrieve_by_email` / `mark_email_verified` /
   `set_password` / `is_email_verified`) nur, wenn `User` sowohl
   `MustVerifyEmail` als auch `CanResetPassword` implementiert -
   Suprnovas Analoga zu Laravels `MustVerifyEmail`- /
   `CanResetPassword`-Verträgen:

   ```rust
   use chrono::{DateTime, Utc};
   use suprnova::{Authenticatable, CanResetPassword, MustVerifyEmail};

   impl MustVerifyEmail for User {
       fn email(&self) -> &str {
           &self.email
       }
       fn email_verified_at(&self) -> Option<DateTime<Utc>> {
           self.email_verified_at
       }
       fn set_email_verified_at(&mut self, v: Option<DateTime<Utc>>) {
           self.email_verified_at = v;
       }
       fn name(&self) -> Option<&str> {
           Some(&self.name)
       }
   }

   impl CanResetPassword for User {
       fn email_for_reset(&self) -> &str {
           &self.email
       }
       fn set_password_hash(&mut self, hash: &str) {
           // Der Wert kommt bereits gehasht an - unverändert speichern.
           self.password = hash.to_string();
       }
   }
   ```

   `is_email_verified()` hat einen Standard, der den Zeitstempel
   nachverfolgt (`email_verified_at().is_some()`), und `name()`
   liefert standardmäßig `None` - überschreiben Sie es, um Benutzer
   in der Mail namentlich zu begrüßen.

3. **Zwei Spalten / Tabellen in Ihrem Migrator.** Die Tabelle
   `users` braucht einen nullbaren `email_verified_at`-Zeitstempel
   (der Provider liest ihn in `is_email_verified` und stempelt ihn
   in `mark_email_verified`), und die Single-Use-Tabelle
   `auth_flow_tokens` des Frameworks hält die Verifizierungs- /
   Reset-Tokens. Das Framework liefert das `CREATE` der
   Token-Tabelle; listen Sie es in Ihrem Migrator auf:

   ```rust
   use sea_orm_migration::prelude::*;

   #[async_trait::async_trait]
   impl MigrationTrait for AuthFlowTokens {
       async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .create_table(
                   suprnova::auth_flows::token_store::create_auth_flow_tokens_table(),
               )
               .await
       }

       async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .drop_table(Table::drop().table(Alias::new("auth_flow_tokens")).to_owned())
               .await
       }
   }
   ```

   Fügen Sie `email_verified_at` in Ihrer eigenen Spalten-Migration
   zu `users` hinzu (ein nullbares `timestamp_with_time_zone`);
   `NULL` bedeutet unverifiziert, sodass bestehende Zeilen korrekt
   aufgefüllt werden.

Tokens sind Single-Use und ruhend SHA-256-gehasht - ein
Datenbank-Dump liefert nie einen brauchbaren Klartext-Token. Die
Standard-TTLs sind **24 Stunden** für E-Mail-Verifizierung und
**15 Minuten** für Passwort-Reset.

### Brute-Force + 2FA: torii verdrahten

`BruteForce` / `LoginThrottleMiddleware` und `TwoFactor` sind
torii-gestützt - sie brauchen die globale torii-Instanz,
initialisiert in `bootstrap.rs::register()`, nach `DB::init`.
(OAuth, Passkeys und WebAuthn-Ceremonies laufen über dieselbe
Instanz - siehe [Authentifizierung](authentication.md).)

```rust
use suprnova::torii_integration::{init_torii, ToriiConfig};
use suprnova::DB;

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    DB::init().await?;

    let conn = DB::connection()?.inner().clone();
    init_torii(ToriiConfig::from_sea_orm(conn)).await?;

    Ok(())
}
```

`init_torii` ist idempotent. Die `OnceLock`-Absicherung bedeutet,
dass der zweite Aufruf ein No-op ist, sodass Test-Harnesses, die
`register()` pro Fixture erneut betreten, nicht doppelt migrieren.
Für Tests tauschen Sie `ToriiConfig::sqlite_in_memory()` ein - das
fährt eine In-Memory-Datenbank mit gemeinsam genutztem Cache hoch,
die Runtimes überlebt:

```rust
let config = ToriiConfig::sqlite_in_memory()
    .await?
    .apply_migrations(true);
init_torii(config).await?;
```

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
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | Anti-Enumeration: schlägt den Benutzer über die E-Mail-Adresse nach; eine unbekannte Adresse ist ein stilles `Ok(())`. |
| `check` | `check(token: &str) -> Result<bool>` | Nicht einlösend - sicher auf einer Landing-Page aufzurufen. |
| `verify` | `verify(token: &str) -> Result<String>` | Single-Use: löst den Token ein, markiert den Benutzer als verifiziert, liefert die Benutzer-ID. |

```rust
use suprnova::auth_flows::EmailVerification;

// Nach einer frischen Registrierung, mit dem frisch angelegten Benutzer:
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// Optionale Landing-Page-Prüfung - nicht einlösend, sodass ein
// Seiten-Refresh den Token nicht verbrennt.
let valid: bool = EmailVerification::check(&token_str).await?;

// Der Click-through-Handler löst den Token ein und stempelt den
// Benutzer, wobei er die ID des verifizierten Benutzers liefert.
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

Der Click-through-Handler zieht den Token aus dem Query-String und
ruft `verify` auf:

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

Der Handler muss den Benutzer nicht nachschlagen - `verify` löst
den Token ein, markiert den Benutzer über den Provider als
verifiziert, liefert die Benutzer-ID und feuert `EmailVerified`.
Single-Use: Ein zweites `verify` auf demselben Token liefert einen
Fehler.

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

`PasswordReset` hat drei Operationen:

| Methode | Signatur | Anmerkungen |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | Anti-Enumeration: schlägt den Benutzer über die E-Mail-Adresse nach; eine unbekannte Adresse ist ein stilles `Ok(())`. |
| `check` | `check(token: &str) -> Result<bool>` | Nicht einlösend - bestätigt den Token, bevor das Neues-Passwort-Formular gerendert wird. |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | Single-Use: löst den Token ein, rotiert das Passwort, widerruft Sessions + Remember-me, versendet die Änderungsbenachrichtigung, liefert die Benutzer-ID. |

```rust
use suprnova::auth_flows::PasswordReset;

// Aus dem "Passwort vergessen"-Formular. Immer Ok(()) - die Facade
// schlägt den Benutzer nach und versendet nur, wenn ein Konto vorliegt.
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// Optionale Landing-Page-Prüfung, bevor das Neues-Passwort-Formular gerendert wird.
let valid: bool = PasswordReset::check(&token).await?;

// Der Click-through-Handler, nachdem der Benutzer ein neues Passwort
// übermittelt hat: den Token einlösen + das Passwort rotieren, wobei
// die Benutzer-ID geliefert wird.
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` hasht `new_password`, bevor es an den Provider
übergeben wird - übergeben Sie den Klartext, keinen bereits
gehashten Wert. Ein leeres Passwort / ein Passwort nur aus
Leerraum wird von vornherein mit `400` abgelehnt.

### Anti-Enumeration

`send_link` ist so strukturiert, dass die Response-Form nie
durchsickern lässt, ob eine E-Mail-Adresse ein Konto hat:

- Es liefert immer `Ok(())`. Fehlt die E-Mail-Adresse, wird kein
  Token geprägt, keine Mail dispatcht und kein
  `PasswordResetLinkSent`-Event gefeuert - aber das Fehlen tritt
  auch nicht über den Rückgabetyp zutage, sodass ein Aufrufer (und
  ein Netzwerk-Beobachter) nicht zwischen "kein solches Konto" und
  "Link gesendet" unterscheiden kann.
- Der Dogfood-Controller paart `send_link` mit einem festen
  200-Response-Body, sodass ein sondierender Aufrufer nicht über
  Statuscode, Response-Body oder Response-Timing unterscheiden
  kann.

### Nebeneffekte von `complete`

`complete` führt vier Schritte in dieser Reihenfolge aus:

1. Den Token einlösen (Single-Use) und den Passwort-Hash über den
   konfigurierten Provider rotieren (der einzige Schritt, der den
   Aufruf scheitern lassen kann).
2. Jede Session-Zeile des Benutzers über
   `crate::session::destroy_all_for_user` widerrufen (Best-Effort:
   Fehlschläge protokollieren über `tracing::warn!`).
3. Jede Remember-me-Zeile über
   `crate::auth::remember::revoke_all_for_user` widerrufen
   (Best-Effort).
4. `PasswordChangedMail` fire-and-forget dispatchen, dann
   `PasswordResetCompleted` feuern.

Eine gestohlene Session und ein abgefangenes Remember-me-Cookie
dürfen das Credential, von dem sie abhingen, nicht überleben. Die
Widerrufe passieren bei jedem erfolgreichen Reset, nicht nur bei
vom Benutzer initiierten, sodass ein vom Security-Team erzwungener
Reset auch einen aktiven Angreifer hinauswirft.

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
- Header `Retry-After` - Sekunden, berechnet aus dem
  `locked_until` der Sperrung über
  `LockoutStatus::retry_after_seconds`. Fällt auf `900` zurück
  (15 Minuten - toriis Standard-Sperrdauer), falls der Zeitstempel
  irgendwie fehlt.
- Body: `"Account locked due to too many failed login attempts. Try
  again later."`

### Fail-Open bei Backend-Fehlern

Liefert `get_lockout_status` ein `Err` (vorübergehender
Datenbank-Hänger), lässt die Middleware die Anfrage durch. Der
nachgelagerte Login-Handler macht dann den Aufruf selbst und kann
entscheiden, ob er fail-closed oder fail-open ausfällt. Die
Middleware irrt zugunsten der Verfügbarkeit: den Login-Endpunkt
lahmzulegen, wann immer die Auth-Datenbank einen Hänger hat, ist
schlimmer, als den Handler den Aufruf direkt machen zu lassen.

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

Toriis `BruteForceProtectionConfig` verwendet standardmäßig
**5 fehlgeschlagene Versuche vor der Sperrung** und eine
**15-minütige Sperrdauer**. Das ist es, was `init_torii` heute
verdrahtet; das Konfigurieren app-spezifischer Werte erfordert den
Zugriff auf toriis eigene Konfigurationsoberfläche und ist nicht
über Suprnovas `ToriiConfig`-Builder freigelegt. Die Standardwerte
sind absichtlich konservativ - akzeptieren Sie erst "fünf
Vertipper sperren mich für 15 Minuten aus", bevor Sie sich
entscheiden, sie zu lockern.

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

`user_id` ist der undurchsichtige Speicherschlüssel -
typischerweise `torii::UserId.as_str()`, aber jeder stabile
Pro-Benutzer-Identifikator funktioniert. Die 2FA-Tabelle indiziert
darüber; es gibt keinen FK zu Ihrer Benutzertabelle.

`email` wird in das `account_name`-Segment der `otpauth://`-URL
gefaltet, sodass die Authenticator-App die Zeile mit einem
menschenlesbaren Label rendert (z. B. "MyCorp
(alice@example.com)").

Ein gängiges Muster ist ein kleiner Newtype, der Ihr Benutzermodell
umschließt:

```rust
use suprnova::auth_flows::TwoFactorUser;
use suprnova::torii_integration::User as ToriiUser;

struct AppUser2FA<'a> { user: &'a ToriiUser }

impl<'a> TwoFactorUser for AppUser2FA<'a> {
    fn user_id(&self) -> &str { self.user.id.as_str() }
    fn email(&self)   -> &str { &self.user.email }
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

Die 2FA-`user_id` ist absichtlich ein `String`. Wäre sie als
`i64`, `Uuid` oder `torii::UserId` typisiert, wäre die
2FA-Tabelle dauerhaft an die Form gebunden, die das Framework
zuerst gewählt hat - Apps, die Benutzer in einer anderen Form
speichern (UUIDs statt Auto-Increment-Integers, oder Apps, die
torii gar nicht verwenden, aber das 2FA-Modul wollen), wären
ausgesperrt. Eine stringartige `user_id` lässt jede App den
stabilen Pro-Benutzer-Identifikator wählen, den sie mag; der
Kompromiss ist ein `.to_string()` an der Aufrufstelle. Laravels
Fortify bindet die entsprechende Spalte an Eloquents `User::id` -
Suprnova entkoppelt sie, sodass `TwoFactor` eine wiederverwendbare
Lifecycle-Primitive ist, kein User-förmiges Zubehör.

## Remember-me

`suprnova::auth_flows::remember_me` re-exportiert
`suprnova::auth::remember` - das Modul für persistente Cookies, das
bereits neben der Session-Auth ausgeliefert wurde. Der Re-Export
ist rein organisatorisch: Alles auth-flow-förmige liegt unter
`auth_flows::*`, selbst wenn die Implementierung diesem Namespace
zeitlich vorausgeht.

Das ausgelieferte Design:

- **DB-Zeile + bcrypt-Hash** - jeder ausgestellte Token hat eine
  Zeile in der Tabelle `remember_tokens`, die nur den bcrypt-Hash
  speichert, nie den Klartext. Ein Datenbank-Dump kann keine
  erneut authentifizierenden Credentials liefern.
- **Single-Use-Rotation** - eine erfolgreiche Verifizierung
  löscht (DELETE) die passende Zeile und stellt eine frische aus.
  Ein abgefangenes Cookie kann nicht wiederverwendet werden;
  liefern sich Angreifer und Opfer ein Race um seine Nutzung,
  sieht der Verlierer die Zeile verschwunden und scheitert bei der
  Authentifizierung.
- **Widerruf** - `revoke_all_for_user` löscht jede Zeile eines
  Benutzers in einem DELETE. `Auth::logout` kettet das an, sodass
  ein echter Logout tatsächlich persistenten Zustand räumt, und
  `PasswordReset::complete` tut dasselbe, sodass ein
  Passwort-Reset jedes bestehende persistente Cookie invalidiert.
- **Aufräumen** - `prune_expired` räumt abgelaufene Zeilen nach
  einem Zeitplan auf.

In der Praxis erledigt die Session-Middleware des Frameworks die
Schwerstarbeit; die typische App ruft das `remember_me`-Modul
nicht direkt auf. Das Dokument [Authentifizierung](authentication.md)
behandelt die benutzerseitige Oberfläche - das `remember`-Flag auf
`Auth::login`, den Cookie-Namen und die Lebensdauer-Regler.

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

### Integrationstests für E-Mail-Verifizierung + Passwort-Reset

Verify-/Reset-Tests brauchen kein torii - stellen Sie die Tabelle
`auth_flow_tokens` auf einer In-Memory-Datenbank bereit,
registrieren Sie einen Provider, setzen Sie `MAIL_FROM`, und
spielen Sie die Facade unter `Mail::fake()` durch. Die eigenen
Tests des Frameworks prägen die Tabelle direkt aus
`create_auth_flow_tokens_table()`:

```rust
use sea_orm::ConnectionTrait;
use suprnova::auth_flows::token_store::create_auth_flow_tokens_table;
use suprnova::mail::Mail;
use suprnova::testing::TestDatabase;

#[tokio::test]
#[serial_test::serial]
async fn send_link_mails_a_token_link() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let conn = db.conn();
    let stmt = create_auth_flow_tokens_table();
    conn.execute(conn.get_database_backend().build(&stmt))
        .await
        .unwrap();

    // Die Facades lesen MAIL_FROM (fail-closed); für den Test setzen.
    // SAFETY: durch `#[serial]` serialisiert - kein paralleler Beobachter.
    unsafe { std::env::set_var("MAIL_FROM", "test-mailer@example.com"); }

    let fake = Mail::fake();
    // ... EmailVerification::send_link(&user, base) durchspielen ...
    fake.assert_sent_to("ada@example.com");
}
```

Die provider-gestützten Pfade (`resend` / `verify` / `complete`)
registrieren zusätzlich eine `dyn UserProvider`-Bindung, damit der
Lookup + die Mutation aufgelöst werden - siehe
`framework/tests/email_verify.rs` und
`framework/tests/password_reset.rs`.

### `ToriiConfig::sqlite_in_memory()` für Brute-Force- + 2FA-Tests

Brute-Force- und 2FA-Tests fahren ein frisches torii auf einer
In-Memory-SQLite-Datenbank hoch. Die Beispiel-Testdateien in
`framework/tests/` verwenden ein Muster aus geteilter Runtime +
`once_cell::sync::Lazy<()>`, um die Kosten über Tests hinweg zu
amortisieren, plus `#[serial]`, um den prozessglobalen
Mail-Transport zwischen Tests, die `Mail::fake()` verschachteln,
stabil zu halten:

```rust
use once_cell::sync::Lazy;
use serial_test::serial;
use tokio::runtime::Runtime;
use suprnova::torii_integration::{init_torii, ToriiConfig};

static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

static SETUP: Lazy<()> = Lazy::new(|| {
    RT.block_on(async {
        let config = ToriiConfig::sqlite_in_memory()
            .await
            .expect("sqlite in-memory connection")
            .apply_migrations(true);
        init_torii(config).await.expect("init_torii");
    });
});

#[test]
#[serial]
fn my_test() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        // ... hier Mail::fake() / EventFacade::fake() verwenden ...
    });
}
```

Kanonische Beispiele - kopieren Sie daraus, wenn Sie Ihre eigenen
schreiben:

- `framework/tests/email_verify.rs` - Token-Round-Trip für verify,
  `send_link`-Trailing-Slash-Trimming,
  `Mail::fake()`-Assertions auf Subject/HTML.
- `framework/tests/password_reset.rs` - Reset-Round-Trip mit
  Neues-Passwort-Authentifizierung, Anti-Enumeration bei
  unbekannten E-Mail-Adressen, `complete` lehnt wiederverwendete
  Tokens ab.
- `framework/tests/brute_force.rs` - vollständiger
  Lockout-Lifecycle, `AccountLocked` feuert einmal pro Übergang,
  `unlock_account` liefert `was_locked`.
- `framework/tests/two_factor.rs` - vollständiges enroll →
  confirm → verify mit einem echten, aus der otpauth-URL
  berechneten TOTP-Code, Recovery-Code-Single-Use, erneutes
  Enrollment überschreibt das Secret, Replay-Ablehnung über zwei
  nebenläufige Verifys hinweg.
- `framework/tests/two_factor_challenge_flow.rs` - der
  End-to-End-Challenge-Flow mit Session-Rotation,
  Remember-me-Neuausstellung und Event-Dispatch.
- `framework/tests/email_verified_middleware.rs` und
  `two_factor_challenge_middleware.rs` - Middleware-Response-Formen
  (403 JSON vs. 302 vs. 409 + X-Inertia-Location).

## Referenz

| Symbol | Zweck |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`, `resend`, `check`, `verify` - provider-gestützt; `verify` liefert die Benutzer-ID. |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` für 403 JSON, `redirect_to(path)` für 302 / 409 + X-Inertia-Location. Prüft `is_email_verified` des konfigurierten Providers (fail-closed). |
| `suprnova::auth_flows::PasswordReset` | `send_link`, `check`, `complete` - provider-gestützt; `complete` liefert die Benutzer-ID. |
| `suprnova::MustVerifyEmail` / `suprnova::CanResetPassword` | Modell-Traits, die ein Benutzer hinter `EloquentUserProvider` implementiert, damit die Verify-/Reset-Facades seine E-Mail-Adresse lesen + seinen Verifizierungs-Zeitstempel / Passwort-Hash schreiben können. |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | SeaORM-`CREATE TABLE` für `auth_flow_tokens` - in Ihrem Migrator aufführen. |
| `suprnova::auth_flows::BruteForce` | `record_failed_attempt`, `reset_attempts`, `get_lockout_status`, `is_locked`, `unlock_account`. |
| `suprnova::auth_flows::LoginThrottleMiddleware` | HTTP-Middleware, die vor dem Handler 429 liefert, wenn das anvisierte Konto gesperrt ist. |
| `suprnova::auth_flows::TwoFactor` | `enroll`, `re_enroll`, `confirm`, `verify`, `consume_recovery_code`, `regenerate_recovery_codes`, `is_enabled`, `is_enabled_by_id`, `start_challenge`, `pending_user_id`, `cancel_challenge`, `complete_challenge`, `disable`. |
| `suprnova::auth_flows::TwoFactorUser` | Trait, der das Benutzermodell der App zur 2FA-Facade überbrückt. |
| `suprnova::auth_flows::EnrollmentResponse` | Rückgabewert von `TwoFactor::enroll` - `otpauth_url`, `qr_code_svg`, `recovery_codes`. |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | `new()` für 403 JSON, `redirect_to(path)` für 302 / 409 + X-Inertia-Location. Vor `AuthMiddleware` einsetzen. |
| `suprnova::auth_flows::two_factor::migration::Migration` | SeaORM-Migration für `two_factor_credentials`. In Ihrem `Migrator::migrations()` aufführen. |
| `suprnova::auth_flows::two_factor::migration_replay::Migration` | Spalten-Add für `last_used_timestep` (TOTP-Replay-Schutz). Nach der Create-Table-Migration aufführen. |
| `suprnova::auth_flows::remember_me` | Re-Export von `suprnova::auth::remember`. |
| `suprnova::auth_flows::events::*` | Neun Events - siehe [Ereignisse](#ereignisse). |
| `suprnova::auth_flows::EmailVerificationMail` | Transaktionales Mailable. Betreff `"Verify your email for {APP_NAME}"`. |
| `suprnova::auth_flows::PasswordResetMail` | Transaktionales Mailable. Betreff `"Reset your {APP_NAME} password"`. |
| `suprnova::auth_flows::PasswordChangedMail` | Sicherheitsbenachrichtigungs-Mailable. Betreff `"Your {APP_NAME} password was changed"`. |
| `suprnova::torii_integration::ToriiConfig` | Torii-Bootstrap-Konfiguration. `from_sea_orm(conn)` für Produktion, `sqlite_in_memory()` für Tests. |
| `suprnova::torii_integration::init_torii` | Idempotente globale Initialisierung. Einmal aus `bootstrap.rs::register()` aufrufen. |

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

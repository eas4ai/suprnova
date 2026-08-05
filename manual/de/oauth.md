# OAuth, Apple & Magic-Link-Anmeldung

Suprnova liefert drei torii-gestützte Anmeldemethoden hinter der
`Auth`-Facade: **generisches OAuth** (GitHub, Google oder jeder
beliebige OIDC-/OAuth2-Provider), **Sign in with Apple** und
**passwortlose Magic Links**. Sie teilen sich eine Voraussetzung
(`init_torii` plus die Ceremony-Migration) und dieselbe
Facade-Form - `Auth::oauth(provider)` / `Auth::magic_link()` -
und keine von ihnen liefert Routen mit: Sie fügen einen dünnen
Controller hinzu (Start + Callback), und das Framework erledigt
den CSRF-State, PKCE, den Token-Austausch, die
Identitätsverifizierung, den Benutzer-Upsert und das Prägen der
Session.

Die gesamte Oberfläche liegt in
`framework/src/torii_integration/`. Es gibt **keinen**
Umgebungsvariablen-Vertrag des Frameworks dafür - jedes Credential
wird programmatisch übergeben (holen Sie sich Ihre eigenen aus der
Umgebung); die Beispiele dieses Kapitels verwenden
`std::env::var(...)` rein, um zu zeigen, wohin Ihre Secrets gehören.

## Voraussetzungen

1. **torii einmal beim Boot initialisieren** - das steht hinter dem
   Benutzer-Upsert und der Session-Erzeugung:

   ```rust
   use suprnova::{init_torii, ToriiConfig};

   // in bootstrap::register(), nach DB::init()
   init_torii(ToriiConfig::from_sea_orm(db_conn)).await?;
   ```

2. **Die Ceremony-Migration ausführen.** OAuth und Apple hinterlegen
   eine kurzlebige (10 Minuten) CSRF-`state`- + PKCE-Ceremony in der
   Tabelle `auth_ceremony_tokens`. Registrieren Sie die Migration
   `m20251209_000000_create_auth_ceremony_tokens_table` in Ihrem
   `Migrator` (die Starter-Kits enthalten sie bereits). Optional
   können Sie
   `suprnova::torii_integration::ceremony::prune_expired()`
   einplanen, um veraltete Zeilen einzusammeln.

3. **`SessionMiddleware` auf der OAuth-*Start*-Route.** `begin()`
   schreibt den `state` in die Session; ein Aufruf ohne Session
   schlägt mit 500 fehl.

Magic Links benötigen nur Schritt 1.

## Generisches OAuth (GitHub, Google, benutzerdefiniert)

### Einen Provider konfigurieren

Registrieren Sie jeden Provider einmal beim Start. Die Registry ist
prozessweit und idempotent, sodass das erneute Registrieren
desselben Providers die Konfiguration einfach ersetzt:

```rust
use suprnova::Auth;
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("github").configure(OAuthProviderConfig {
    client_id: std::env::var("GITHUB_CLIENT_ID")?,
    client_secret: std::env::var("GITHUB_CLIENT_SECRET")?,
    redirect_url: "https://app.example.com/auth/oauth/github/callback".into(),
    scopes: vec!["user:email".into()],
    endpoints_override: None,   // None → die eingebaute Well-known-Tabelle
    apple_key_pair: None,       // Nur Apple; für GitHub/Google None lassen
    apple_team_id: None,        // Nur Apple
});
```

Die bekannten Authorize-/Token-/Userinfo-Endpunkte sind für
`github`, `google` und `apple` eingebaut. Für jeden anderen
Provider - oder einen selbst gehosteten / Test-Server - liefern Sie
sie selbst:

```rust
use suprnova::torii_integration::oauth::EndpointOverrides;

Auth::oauth("gitlab").configure(OAuthProviderConfig {
    client_id: /* … */,
    client_secret: /* … */,
    redirect_url: /* … */,
    scopes: vec!["read_user".into()],
    endpoints_override: Some(EndpointOverrides {
        authorize: "https://gitlab.com/oauth/authorize".into(),
        token: "https://gitlab.com/oauth/token".into(),
        userinfo: "https://gitlab.com/api/v4/user".into(),
        emails: None,   // /emails-Fallback im GitHub-Stil für eine private primäre Adresse
    }),
    apple_key_pair: None,
    apple_team_id: None,
});
```

### Den Flow starten (Authorize-URL)

```rust
// GET /auth/oauth/github/start  (Route MUSS SessionMiddleware tragen)
let kickoff = Auth::oauth("github").begin().await?;
// kickoff.authorization_url - Browser hierhin umleiten
// kickoff.state - CSRF-State, für Sie bereits in der Session gespeichert
```

`begin()` prägt den CSRF-`state` (UUID v4) und einen
RFC-7636-PKCE-Verifier/S256-Challenge, zeichnet die Ceremony auf
(10 Minuten TTL) und liefert die Authorize-URL des Providers.
Leiten Sie den Benutzer zu `authorization_url` weiter.

### Den Flow abschließen - `verify` vs. `complete`

Beim Callback haben Sie zwei Einstiegspunkte (seit 0.5.4 getrennt).
Wählen Sie danach, ob Ihre `users`-Tabelle torii's Schema **ist**:

| Methode | Liefert | Nebeneffekte | Verwenden, wenn |
|---|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity { provider, subject, email, name }` | **Keine** - verifiziert die Ceremony, tauscht den Code aus, ruft die Userinfo ab, extrahiert eine verifizierte E-Mail-Adresse + stabile `subject`. Kein Benutzer, keine Session. | Ihre App besitzt ihre eigene `users`-Tabelle, und Sie wollen den Benutzer selbst nachschlagen / anlegen. |
| `complete(code, state)` | `(User, Session)` | Upsertet den Benutzer in torii (`get_or_create_user`) und prägt eine Session. | Ihre `users`-Tabelle ist torii's Schema. |

```rust
// Benutzerdefinierte users-Tabelle:
let id = Auth::oauth("github").verify_oauth_identity(&code, &state).await?;
// id.subject ist die stabile Provider-ID; id.email ist verifiziert oder None.
let user = my_users::upsert(id.provider, id.subject, id.email, id.name).await?;

// …oder torii-gestützt:
let (user, session) = Auth::oauth("github").complete(&code, &state).await?;
```

Eine von `verify` zurückgegebene `email` ist immer eine
*verifizierte* Adresse (OIDC `email_verified`, bei GitHub als
verifiziert behandelt, oder der `/emails`-Fallback); eine
unverifizierte oder fehlende E-Mail-Adresse kommt als `None`
zurück, und wiederholte Logins werden über `subject` aufgelöst.

### Routen, die Sie hinzufügen

Das Framework liefert keine OAuth-Routen - verdrahten Sie zwei
dünne Handler (nach dem Vorbild der bestehenden `auth_verify`- /
`auth_reset`-Controller im Starter-Kit):

```rust
// start - leitet zum Provider weiter
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
// callback - GitHub/Google verwenden GET ?code&state
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
```

Stellen Sie (mindestens) die `/start`-Route hinter
`SessionMiddleware`.

## Sign in with Apple

Apple nutzt dieselbe Facade - `Auth::oauth("apple")` - mit einigen
fest eingebauten Apple-spezifischen Regeln:

- **Der Callback ist ein `POST`.** Apple verwendet
  `response_mode=form_post`, sodass der Redirect `code` + `state`
  in einem Formular-Body liefert, nicht als Query-Parameter.
  Registrieren Sie den Apple-Callback als `post!`-Route und lesen
  Sie die Felder aus dem Formular.
- **Kein PKCE.** Apple lehnt `code_challenge` ab, sodass die
  Authorize-URL es weglässt (stattdessen ist das Client-Secret ein
  signiertes JWT).
- **`client_secret` wird nicht verwendet** - lassen Sie es
  `String::new()`. Suprnova prägt bei jedem Token-Austausch das
  kurzlebige JWT-Client-Secret aus Ihrem `.p8`-Schlüssel.
- **ID-Tokens werden seit 0.5.6 gegen Apples JWKS (RS256)
  verifiziert**, nicht mehr strukturell vertraut.

### Ihren Apple-Schlüssel bereitstellen - `AppleKeyPair`

`AppleKeyPair` ist der einzige Apple-Typ, der für Apps re-exportiert
wird (Sie brauchen also keine direkte `apple`-Abhängigkeit). Bauen
Sie ihn aus Ihrem `.p8`-Signierschlüssel:

```rust
use suprnova::torii_integration::oauth::AppleKeyPair;

let key = AppleKeyPair::from_file(
    &std::env::var("APPLE_KEY_ID")?,   // Apples *Key ID* (nicht die Team-ID)
    &std::env::var("APPLE_P8_PATH")?,  // Pfad zu AuthKey_XXXXXX.p8
)?;
// oder: AppleKeyPair::from_base64(key_id, b64)  /  from_pem_bytes(key_id, bytes)
```

### Apple konfigurieren

```rust
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("apple").configure(OAuthProviderConfig {
    client_id: std::env::var("APPLE_CLIENT_ID")?,  // Ihre Services-ID
    client_secret: String::new(),                  // unbenutzt - wird aus dem Schlüssel geprägt
    redirect_url: "https://app.example.com/auth/apple/callback".into(),
    scopes: vec!["email".into(), "name".into()],
    endpoints_override: None,
    apple_key_pair: Some(key),
    apple_team_id: Some(std::env::var("APPLE_TEAM_ID")?),  // 10-stellige Team-ID
});
```

### Den Apple-Flow abschließen

Dieselbe Aufteilung wie bei generischem OAuth. `complete` upsertet +
erzeugt Sessions; der verify-Pfad liefert eine `AppleIdentity`
für eine benutzerdefinierte users-Tabelle:

```rust
// POST /auth/apple/callback - code + state aus dem FORM-Body lesen
let (user, session) = Auth::oauth("apple").complete(&code, &state).await?;

// …oder benutzerdefinierte users-Tabelle:
let id = Auth::oauth("apple").verify_apple_identity(&code, &state).await?;
// id: AppleIdentity { provider, subject, email, email_verified, is_private_email }
```

`AppleIdentity.email` ist nur dann `Some(_)`, wenn Apple sie als
verifiziert bestätigt; eine unverifizierte E-Mail-Adresse wird
abgelehnt (401), bevor die Identität aufgebaut wird.
`is_private_email` wird gesetzt, wenn der Benutzer Apples private
Relay-Adresse gewählt hat - persistieren Sie `subject` als
stabilen Schlüssel, da die Relay-Adresse die einzige
E-Mail-Adresse ist, die Sie erhalten.

## Magic-Link-Anmeldung

Passwortlose E-Mail-Anmeldung, torii-gestützt, über
`Auth::magic_link()`. Das Framework stellt den Token aus und
verifiziert ihn; **Sie** versenden den Link per E-Mail (das
Framework selbst verschickt nie eine Mail), was sich sauber mit dem
Kapitel [Mail](mail.md) verbindet.

```rust
use suprnova::Auth;

// POST /auth/magic - einen Link anfordern
let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;
// Den Link selbst bauen und per E-Mail versenden:
Mail::to("alice@example.com")
    .send(MagicLink { url: format!("https://app.example.com/auth/magic?token={token}") })
    .await?;

// GET /auth/magic?token=… - einlösen (Single-Use; ein zweiter Aufruf schlägt fehl)
let (user, session) = Auth::magic_link().consume(&token).await?;
```

Der Benutzer wird bei der ersten Verwendung automatisch angelegt.
`send` liefert den **Klartext**-Token zurück, damit Sie die Form der
URL und die Zustellung selbst kontrollieren.

> **Hinweis - `TokenPurpose::MagicLink`.** Das
> `TokenPurpose`-Enum von `auth_flows` hat eine
> `MagicLink`-Variante (hinzugefügt in 0.5.5), aber sie ist ein
> *reservierter Diskriminator* für den generischen `TokenStore` -
> kein eingebauter Flow löst sie ein. Der funktionierende,
> unterstützte Magic-Link-Pfad ist das oben gezeigte
> `Auth::magic_link()`. Greifen Sie nur dann zu
> `TokenPurpose::MagicLink`, wenn Sie Ihren eigenen, handgerollten
> Flow auf der Tabelle `auth_flow_tokens` bauen.

## Ein Hinweis zur Konfiguration

Keine dieser Methoden liest Umgebungsvariablen des Frameworks -
Provider-IDs, Secrets, Redirect-URLs und Apple-Schlüssel werden
alle programmatisch an `configure(...)` übergeben. Laden Sie sie,
wie Sie möchten (`std::env::var`, eine typisierte
Konfigurationsstruktur, ein Secrets-Manager), und registrieren Sie
Provider einmal während des `bootstrap`. Das macht
mandantenfähige / bereitstellungsspezifische Provider-Setups zum
Regelfall, statt ein festes Namensschema für Umgebungsvariablen zu
erzwingen.

## Referenz

- Facade-Einstiegspunkte: `Auth::oauth(provider)`,
  `Auth::magic_link()` (`suprnova::Auth`)
- Konfiguration: `suprnova::torii_integration::oauth::{OAuthProviderConfig, EndpointOverrides, AppleKeyPair}`
- OAuth-Ergebnisse: `OAuthKickoff { authorization_url, state }`,
  `OAuthIdentity { provider, subject, email, name }`,
  `AppleIdentity { provider, subject, email, email_verified, is_private_email }`
- Bootstrap: `suprnova::{init_torii, ToriiConfig}`
- Ceremony-Speicher: Tabelle `auth_ceremony_tokens` +
  `suprnova::torii_integration::ceremony::prune_expired()`

## Nächste Schritte

- [Authentifizierung](authentication.md) - Guards, Provider und das
  `Authenticatable`-Benutzermodell, für das diese Flows Sessions
  erzeugen
- [Auth-Flows](auth-flows.md) - E-Mail-Verifizierung, Passwort-Reset
  und 2FA
- [Mail](mail.md) - den Magic-Link per E-Mail versenden (und die
  Absenderkonfiguration `MAIL_FROM` / `MAIL_FROM_NAME`)
- [Sitzungen](session.md) - was die zurückgegebene `Session` ist
  und wie sie persistiert wird

# OAuth, Apple und Magic-Link-Login

Suprnova stellt OAuth, „Mit Apple anmelden“ und passwortlose Magic Links über
die Framework-eigene `Auth`-Facade bereit. Magnetar liefert die Engines für
Anmeldedaten, Zeremonien, Identitäten, Faktor-Gates und Sessions hinter dieser
Facade.

Die öffentlichen Einstiegspunkte sind:

- `Auth::oauth(provider)` für OAuth und Apple.
- `Auth::magic_link()` für passwortlosen E-Mail-Login.

Suprnova installiert für diese Flows keine Routen. Anwendungen stellen kleine
Start- und Callback-Handler bereit und entscheiden, wie sie Magic-Link-E-Mails
zustellen.

## Magnetar mit OAuth initialisieren

Konfigurieren Sie OAuth auf derselben `MagnetarConfig`, die Passwort-, Passkey-, Sitzungs-, Sperr- und Zwei-Faktor-Dienste initialisiert. Die Anbieterregistrierung wird atomar mit diesen Diensten veröffentlicht: Wenn ein Dienst nicht erstellt werden kann, wird keiner davon sichtbar.

```rust,no_run
use std::sync::Arc;

use suprnova::{
    AbuseLimiter, App, AutoLinkPolicy, DB, DatabaseConnection, EndpointOverrides,
    FrameworkAbuseLimiter, GoogleOAuthProvider, GoogleProviderConfig, MagnetarConfig,
    MagnetarOAuthHostConfig, MagnetarOAuthProviderConfig, OAuthAuthorizationConfig,
    OAuthHttpTransport, PasskeyConfig, RateLimiterDriver, ReqwestOAuthTransport,
    RevocationTransport, SecretString, init_magnetar,
};

fn auth_config(
    database: DatabaseConnection,
    transport: Arc<dyn OAuthHttpTransport>,
    revocation: Arc<dyn RevocationTransport>,
    limiter: Arc<dyn AbuseLimiter>,
) -> MagnetarConfig {
    let provider = Arc::new(GoogleOAuthProvider::new(
        GoogleProviderConfig {
            client_id: "google-client".to_owned(),
            client_secret: SecretString::from("google-secret".to_owned()),
            redirect_uri: Some("https://app.example.com/auth/google/callback".to_owned()),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        revocation,
    ));
    let oauth = MagnetarOAuthHostConfig::new(
        vec![MagnetarOAuthProviderConfig {
            provider,
            redirect_uri: "https://app.example.com/auth/google/callback".to_owned(),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
        }],
        transport,
        limiter,
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("valid OAuth host configuration");

    MagnetarConfig::from_sea_orm(database)
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_owned(),
            rp_origin: "https://app.example.com".to_owned(),
        })
        .oauth(oauth)
}

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let transport = Arc::new(ReqwestOAuthTransport::try_default()?);
    let limiter = Arc::new(FrameworkAbuseLimiter::new(
        App::resolve_make::<dyn RateLimiterDriver>()?,
    ));
    init_magnetar(auth_config(
        database.inner().clone(),
        transport.clone(),
        transport,
        limiter,
    ))
    .await
}
```

Das Framework re-exportiert den `OAuthProvider`-Contract, die fünf First-Party-Provider und Konfigurationstypen sowie jeden Typ, der zur Implementierung eines benutzerdefinierten Providers benötigt wird. `ReqwestOAuthTransport` stellt produktionsreife I/O für Token, Userinfo und Widerruf bereit. `FrameworkAbuseLimiter` verwendet den in der Anwendung konfigurierten `RateLimiterDriver`. Anwendungen benötigen weder eine direkte `suprnova-magnetar`-Abhängigkeit noch handgeschriebene Transport- und Limiter-Adapter.

`MagnetarConfig` erstellt sein Schema, wenn `apply_migrations` aktiviert ist, was der Standard ist. Verwenden Sie `.apply_migrations(false)` nur, wenn die Bereitstellung dasselbe Schema separat vorbereitet. Eine zweite Initialisierung gibt einen Fehler zurück, statt eine installierte Engine zu ersetzen.

### GitHub-Provider-Anforderungen

Der REST-Benutzerendpunkt von GitHub erfordert einen `User-Agent`; ein Community-Provider fügt ihn, und jeden benötigten Media-Type-`Accept`-Wert, über `OAuthProvider::userinfo_headers` hinzu. Suprnova fügt den Bearer-`Authorization`-Header separat hinzu und weist Versuche von Providern zurück, ihn zu überschreiben.

Die `/user`-Antwort von GitHub enthält nur dann eine E-Mail, wenn der Benutzer sie öffentlich gemacht hat. Die verifizierte primäre Adresse erfordert eine zweite `/user/emails`-Anfrage, während `resolve_identity` absichtlich keine I/O ausführt und eine Userinfo-Antwort erhält. Ein GitHub-Provider kann `email: None` zurückgeben und Suprnovas Zeremonie zur E-Mail-Vervollständigung verwenden oder `userinfo_endpoint` auf einen Host-Adapter verweisen lassen, der `/user` mit der verifizierten primären E-Mail kombiniert. Behandeln Sie eine unverifizierte oder lediglich öffentliche Adresse nicht als Kontoinhaberschaft.

## Session-Bindung

OAuth-Beginn benötigt `SessionMiddleware`. Magnetar bindet die Zeremonie an
einen Digest der initiierenden Framework-Session, sodass der Callback nicht in
eine andere Browser-Session verschoben werden kann.

Eine erfolgreiche Anmeldung per Passwort, Magic Link, Passkey oder OAuth
rotiert die Framework-Session-ID und das CSRF-Token, zeichnet die
Anwendungsbenutzer-ID auf und speichert eine opake Magnetar-Webbindung. Die
Hydrierung über Remember-me rotiert sowohl die Magnetar-Anmeldedaten als auch
die Framework-Session-Bindung.

## Einen OAuth-Flow starten

Verwenden Sie `begin` im Start-Handler des Providers:

```rust,ignore
use suprnova::Auth;

let kickoff = Auth::oauth("google").begin().await?;
// Eine HTTP-Weiterleitung zu kickoff.authorization_url zurückgeben.
```

Das zurückgegebene `OAuthKickoff` enthält:

- `authorization_url`, die URL, die an den Browser gesendet wird.
- `state`, den an die initiierende Session gebundenen Einmal-Selektor.

Magnetar verantwortet die Erzeugung von State, die PKCE-Richtlinie, die
Speicherung der Zeremonie, den Provider-Austausch, die
Identitätsverifizierung und die Missbrauchsbegrenzung. Der Host-Controller
verantwortet den HTTP-Redirect und die Callback-Route.

## Callback prüfen oder abschließen

Der Callback hat zwei Einstiegspunkte:

| Methode | Ergebnis | Nebeneffekte |
|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity` | Prüft den Provider-Nachweis und gibt Provider, Subject, verifizierte E-Mail und Anzeigename zurück, ohne eine Anwendungs-Session zu erstellen. |
| `complete(code, state)` | `(User, Session)` | Löst die Identität über die installierte Host-Engine auf, wendet die Richtlinie zur Kontoverknüpfung und das Faktor-Gate an, rotiert die Framework-Session und gibt den Framework-eigenen Benutzer sowie Magnetar-Session-Werte zurück. |

```rust,ignore
let identity = Auth::oauth("google")
    .verify_oauth_identity(&code, &state)
    .await?;

let (user, session) = Auth::oauth("google")
    .complete(&code, &state)
    .await?;
```

`OAuthIdentity.email` ist nur vorhanden, wenn der Provider eine verifizierte
E-Mail geliefert hat. Speichern Sie Provider und Subject als stabile externe
Identität. Die E-Mail ist kein stabiler Provider-Bezeichner.

## Richtlinie zur Kontoverknüpfung

Der OAuth-Abschluss behandelt den Besitz einer nicht verifizierten
E-Mail-Zeichenfolge nicht als Nachweis, dass der Aufrufer ein bestehendes
Anwendungskonto besitzt.

Das Ergebnis des Abschlusses kann weitere Arbeit verlangen, statt eine Session
auszustellen:

- **E-Mail-Abschluss erforderlich** gibt HTTP 409 zurück, wenn die
  Provider-Identität eine separate Zeremonie für verifizierte E-Mail benötigt.
- **Explizite Verknüpfung erforderlich** gibt HTTP 409 zurück, wenn ein
  bestehendes verifiziertes Konto die Verknüpfung autorisieren muss.
- **Faktor erforderlich** gibt HTTP 401 zurück, wenn die Kontorichtlinie vor
  der Session-Ausstellung einen zweiten Faktor verlangt.

Ein Abschluss der verifizierten E-Mail, der die Grenze für den ersten
E-Mail-Nachweis gewinnt, nimmt ein nicht verifiziertes Squatter-Konto atomar
wieder in Besitz. Die Transaktion erhöht die Auth-Epoche, entfernt
provisorische Anmeldedaten, widerruft alte Sessions und Remember-Anmeldedaten
und hängt das verifizierte Provider-Konto an. Ein verifiziertes Konto wird nie
allein über die E-Mail automatisch verknüpft.

## Mit Apple anmelden

Apple verwendet dieselbe Facade `Auth::oauth("apple")`, aber der Callback
verwendet üblicherweise `response_mode=form_post`. Registrieren Sie den Callback
als `POST`-Route und reichen Sie das optionale Apple-Formularfeld `user` an die
Apple-spezifischen Methoden weiter:

```rust,ignore
let identity = Auth::oauth("apple")
    .verify_apple_identity(&code, &state, form_post_user.clone())
    .await?;

let (user, session) = Auth::oauth("apple")
    .complete_with_apple_form_post(&code, &state, form_post_user)
    .await?;
```

`AppleIdentity` enthält das stabile Subject, eine optionale verifizierte
E-Mail, `email_verified` und `is_private_email`. Speichern Sie das Subject als
stabilen Schlüssel. Apple kann den Anzeigenamen nur bei der ersten
Autorisierung liefern; der Provider-Adapter muss deshalb den ersten
`form_post`-Wert erhalten.

Die Prüfung von Apple-Token und -Identität gehört zur installierten
Provider-Implementierung. Aktuelle Magnetar-Provider verlangen Prüfungen von
Signatur, Aussteller, Audience, Ablauf und Nonce, statt dem dekodierten JSON
eines ID-Tokens zu vertrauen.

## Magic-Link-Login

Der Magic-Link-Login verwendet die installierte Magnetar-Passwort-/Session-
Engine. Das Framework gibt den Klartext-Einmal-Token zurück, während die
Anwendung die Zusammenstellung der E-Mail und die Form der URL verantwortet:

```rust,ignore
use suprnova::{Auth, Mail};

let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;

let url = format!("https://app.example.com/auth/magic?token={token}");
Mail::to("alice@example.com")
    .send(MagicLinkMail { url })
    .await?;

let (user, session) = Auth::magic_link().consume(&token).await?;
```

`send` wendet vor der Ausstellung des Tokens das Missbrauchsbudget der
Authentifizierung an. `consume` ist nur einmal verwendbar, wendet das
Faktor-Gate an, bindet die resultierende Session in die Framework-Anfrage-
Session ein und gibt den Benutzer sowie die Magnetar-Session zurück.

Für ein nicht verifiziertes bestehendes Konto ist der erfolgreiche Verbrauch
eines Magic Links der erste E-Mail-Nachweis. Die Transaktion nimmt das Konto
wieder in Besitz und entfernt provisorischen Passwort-, Passkey-,
Linked-Account-, Zwei-Faktor-, Session- und Remember-Zustand, damit ein
früherer Squatter keinen Zugriff behalten kann.

## Hinzuzufügende Routen

Eine typische Anwendung fügt diese Routen hinzu:

```rust,ignore
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
post!("/auth/apple/callback", controllers::oauth::apple_callback),
post!("/auth/magic", controllers::magic_link::send),
get!("/auth/magic/callback", controllers::magic_link::consume),
```

Wenden Sie `SessionMiddleware` auf jede Start-/Callback-Route für OAuth und
Passkeys an. Die Session trägt den Zeremonie-Selektor und bindet den Umlauf an
den Browser, der ihn gestartet hat.

## Authentifizierung migrieren

Die Crate `suprnova-magnetar` enthält eine formbewusste Migrations-Engine für
Torii-, Suprnova-Web-, Suprnova-API- und bestehende Magnetar-Schemata. Sie ist
eine Bibliotheksoberfläche und ein Beispiel, kein `suprnova`-CLI-Unterbefehl.

Aktivieren Sie das Feature `migration` sowie den Treiber der Quelldatenbank und
führen Sie vor der Anwendung einen Probelauf des Plans aus. Für PostgreSQL:

```text
cargo run -p suprnova-magnetar \
  --features migration,seaorm-postgres \
  --example migrate -- \
  --source-shape torii \
  --database-url "$SOURCE_DATABASE_URL" \
  --app-database-url "$DATABASE_URL"
```

Verwenden Sie stattdessen `seaorm-mysql` oder `seaorm-sqlite`, wenn dies der
Treiber für Quell- und Anwendungsdatenbank ist.

Fügen Sie `--apply` hinzu, um den geprüften Plan anzuwenden. Der Runner prüft
vor dem Import erneut Quell- und Schema-Fingerprints, zeichnet den
Wiederholungszustand auf, verweigert Identitätskollisionen und verwendet
transaktionale Importe. MySQL-Migrationen innerhalb derselben Datenbank
verwenden einen durch eine Schreibbarriere geschützten Shadow-Swap mit
wiederaufnehmbaren Wiederherstellungs- und Abbruchpfaden.

Bewahren Sie den erzeugten Plan und Bericht in den Bereitstellungsunterlagen
auf. Wenden Sie keinen Plan an, dessen Quell-Fingerprint sich nach der Prüfung
geändert hat.

## Referenz

- Standard-Boot: `MagnetarConfig`, `PasskeyConfig` und `init_magnetar`.
- Facades: `Auth::oauth(provider)` und `Auth::magic_link()`.
- OAuth-Installation: `MagnetarConfig::oauth`, `ReqwestOAuthTransport` und `FrameworkAbuseLimiter`.
- Migrationsbibliothek: `magnetar::migration` aus der Crate `suprnova-magnetar`.
- Bearer-Authentifizierung: `BearerTokenMiddleware`.

## Nächste Schritte

- [Authentifizierung](authentication.md) behandelt Passwort, Passkey, Guards,
  Framework-Sessions und die Engine-Initialisierung.
- [Auth-Flows](auth-flows.md) behandelt E-Mail-Verifizierung, Passwort-Reset,
  Sperren und Zwei-Faktor-Authentifizierung.
- [Mail](mail.md) behandelt die durch die Anwendung verantwortete
  Magic-Link-Zustellung.
- [Session](session.md) behandelt die Browser-Session, die OAuth- und
  Passkey-Zeremonien bindet.

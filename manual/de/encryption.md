# Verschlüsselung

Suprnova liefert Verschlüsselung auf Anwendungsebene als
prozessweite Facade namens `Crypt`. Sie verschlüsselt Strings oder
jeden `Serialize`-Wert unter AES-256-GCM, wobei Ihr `APP_KEY` als
Schlüssel dient. Greifen Sie darauf zurück, wann immer Sie etwas
Sensibles in einem Speicher ablegen müssen, dem Sie nicht
vollständig vertrauen - eine Spalte, ein Cookie, ein
Paginierungs-Cursor - und es später intakt zurücklesen müssen.

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cast, "ssn-123-45-6789")?;
let plain = Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
assert_eq!(plain, "ssn-123-45-6789");
```

Das Framework selbst verwendet `Crypt` für verschlüsselte Cookies,
verschlüsselte Paginierungs-Cursor, 2FA-Secrets, Recovery-Codes und
die `AsEncrypted*`-Eloquent-Casts. Dieselbe Facade steht Ihrem Code
ohne zusätzliche Verdrahtung zur Verfügung, sobald `APP_KEY`
konfiguriert ist (siehe
[configuration.md](configuration.md#the-env-file)).

## Das Wire-Format

`encrypt_string` und `encrypt` liefern beide URL-sicheres base64
(ohne Padding) über `nonce || ciphertext_with_tag`:

```
base64url( [12-Byte-Zufalls-Nonce] || [Chiffrat] || [16-Byte-GCM-Tag] )
```

Jeder Aufruf zieht eine frische 12-Byte-Nonce aus dem RNG des
Betriebssystems, sodass zwei Verschlüsselungen desselben Klartexts
unter demselben Schlüssel unterschiedliche Chiffrate erzeugen. Es
gibt kein Padding-Oracle, das über die Länge des Klartexts selbst
hinaus Informationen preisgeben könnte.

Die Ausgabe kann ohne weitere Kodierung sicher in
URL-Query-Strings, JSON-Bodies, Headern und Cookies verwendet
werden. Ein minimal gültiges Wire ist 28 Byte lang (12 Nonce + 16
Tag) - alles Kürzere wird von vornherein abgelehnt.

## `APP_KEY` - das eine Secret, auf das es ankommt

Suprnova liest einen einzelnen 32-Byte-symmetrischen Schlüssel aus
der Umgebungsvariable `APP_KEY`. Das erwartete Format ist
URL-sicheres base64 ohne Padding, das auf genau 32 Byte dekodiert
(43 base64-Zeichen):

```env
APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

Erzeugen Sie einen mit der CLI:

```bash
suprnova key:generate
# Generated a new APP_KEY (AES-256, base64 URL-safe, no padding):
#
#     hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
#
# Add it to your .env (or your secrets manager):
#
#     APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

Oder direkt in die Umgebung pipen:

```bash
echo "APP_KEY=$(suprnova key:generate --show)" >> .env
```

### Boot-Zeit-Validierung - Fail-Closed

`Server::from_config` validiert `APP_KEY` **bei jedem Boot**, nicht
nur beim ersten. Die Regeln:

| Umgebung | `APP_KEY` nicht gesetzt | `APP_KEY` fehlerhaft |
|---|---|---|
| `local`, `development`, `testing` | Erzeugt einen transienten Schlüssel, warnt in den Logs | Harter Fehler - Boot schlägt fehl |
| `staging`, `production`, alles andere | Harter Fehler - Boot schlägt fehl | Harter Fehler - Boot schlägt fehl |

Ein fehlerhafter Schlüssel ist **immer** ein harter Fehler, selbst
in `local` - besser, der Boot schlägt fehl, als einen Tippfehler zu
verschleiern. Ein `Custom`-Umgebungswert, den das Framework nicht
erkennt (z. B. `APP_ENV=k8s`), wird produktionsähnlich behandelt:
kein `APP_KEY`, kein Boot.

Die Diagnosemeldung verweist auf die Lösung:

```
APP_KEY is required when APP_ENV=production. Generate one with
`suprnova key:generate` and set it in your environment (e.g. .env
or your secrets manager). Suprnova refuses to boot without an
encryption key outside of local/development/testing because session
cookies and pagination cursors would otherwise be unsigned and
forgeable.
```

## `CryptPurpose` - Domänentrennung über AAD

Jeder `Crypt::*`-Aufruf nimmt einen `CryptPurpose` entgegen. Die
Variante wird auf ein stabiles Byte-Label abgebildet, das als
Associated Data (AAD) in den AES-GCM-Authentifizierungs-Tag
eingebunden wird:

```rust
pub enum CryptPurpose {
    Cookie,            // suprnova:cookie:v1
    Cursor,            // suprnova:cursor:v1
    TwoFactorSecret,   // suprnova:2fa:secret:v1
    TwoFactorRecovery, // suprnova:2fa:recovery:v1
    Cast,              // suprnova:cast:v1
}
```

Das Label wird **nicht** im Wire gespeichert. GCM mischt die AAD in
den Authentifizierungs-Tag ein, ohne sie in das Chiffrat
aufzunehmen, sodass:

- Das On-Wire-Format bleibt unverändert - weiterhin
  `base64(nonce || ciphertext || tag)`.
- Ein unter `CryptPurpose::Cookie` erzeugtes Wire wird von jedem
  Decrypt-Aufruf, der einen anderen Zweck angibt, **abgelehnt**.
  Die GCM-Tag-Prüfung schlägt fehl, bevor irgendein Parsing nach
  der Entschlüsselung läuft.
- Eine neue Oberfläche hinzuzufügen (eine künftige
  Queue-Payload-Verschlüsselung, ein verschlüsselter Datei-Header)
  bedeutet, eine neue Variante hinzuzufügen - nicht das Wire-Format
  zu ändern.

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cookie, "session-id")?;

// Gleicher Schlüssel, gleiches Wire, anderer Zweck - schlägt fehl.
let result = Crypt::decrypt_string(CryptPurpose::Cursor, &wire);
assert!(result.is_err());

// Gleicher Zweck - erfolgreich.
let plain = Crypt::decrypt_string(CryptPurpose::Cookie, &wire)?;
```

### Warum Suprnova abweicht

Laravels `Crypt::encryptString` nimmt keinen Zweck entgegen. Der
eine `APP_KEY` wird über Cookies, signierte URLs, signierte
Ablauf-Tokens und beliebige Benutzeraufrufe von `Crypt::encrypt`
hinweg wiederverwendet, ohne Domänentrennung auf der
Krypto-Schicht. Wenn zwei Oberflächen zufällig Chiffrat derselben
Klartext-Form akzeptieren, kann ein für eine Oberfläche geprägter
Wert per Replay in die andere eingespielt werden.

Suprnova verwendet denselben `APP_KEY` aus demselben Grund wieder -
Operatoren verwalten ein Secret - bindet aber jede Oberfläche an
ihr eigenes AAD-Label. Ein oberflächenübergreifender
Chiffrat-Replay wird bei der GCM-Tag-Prüfung abgelehnt, bevor
irgendein Parsing läuft. Die Kosten für den Aufrufer sind ein
zusätzlicher Enum-Parameter; der Gewinn ist eine Eigenschaft, die
das Wire-Format allein nicht brechen kann.

Das Suffix `:v1` bei jedem Label ist für künftige Rotation pro
Oberfläche reserviert: Das Anheben von `suprnova:cookie:v1` auf
`suprnova:cookie:v2` invalidiert **nur** altes Cookie-Chiffrat -
Cursor, 2FA-Secrets und Cast-Spalten bleiben unberührt.

## An Cookie-Namen gebundene AAD (v2)

Verschlüsselte Cookies verwenden eine zweite AAD-Generation, wenn der Aufrufer
den logischen Cookie-Namen kennt. `Cookie::encrypted("suprnova_session",
value)` bindet `suprnova:cookie:v2:suprnova_session` in das GCM-Tag ein, und
`Cookie::read_encrypted_for("suprnova_session", wire)` liefert beim Lesen
denselben Kontext:

```rust
use suprnova::Cookie;

let cookie = Cookie::encrypted("suprnova_session", "session-id")?;
let wire = cookie.value().to_string();
assert_eq!(
    Cookie::read_encrypted_for("suprnova_session", &wire)?,
    "session-id"
);
assert!(Cookie::read_encrypted_for("other_cookie", &wire).is_err());
```

Der gebundene Name ist logisch, nicht der Wire-Name. Ein späteres
`__Host-`- oder `__Secure-`-Präfix des Wire-Namens ändert die AAD deshalb
nicht und meldet Benutzer nicht ab. Das Präfix ist eine Browser- und
Header-Angelegenheit; der Cookie-Name ist die kryptografische Domäne.

### Das Kompatibilitätsfenster

Das Wire-Format bleibt unverändert und versionslos: Es enthält weiterhin nur
Nonce, Ciphertext und Authentifizierungs-Tag. Es gibt kein Versionsbyte, nach
dem der Leser verzweigen könnte. `decrypt_string_for` führt eine Blindprüfung
wie bei der Schlüsselrotation aus: zuerst kontextbezogene V2-AAD über den
gesamten Schlüsselring, dann kontextlose V1-AAD über den gesamten Ring. Dadurch
bleiben Cookies, die vor der Namensbindung geschrieben wurden, lesbar, während
auch die Rotation von `APP_KEY` läuft.

Das Fenster bewahrt die alte Replay-Schwäche für seine gesamte Dauer. Ein
V1-Cookie aus einem Cookie-Slot kann weiterhin in einen anderen Slot replayt
werden, solange der kontextlose Fallback besteht; die Namensbindung greift erst
vollständig, wenn dieser Fallback in 1.4.0 entfernt wird. Nichts zieht den
Fallback automatisch zurück: `Crypt::encrypt_string(CryptPurpose::Cookie,
...)` prägt weiterhin V1, und der kontextlose Einstiegspunkt wird erst durch
die für 1.4.0 geplante Entfernung ersetzt. Stellen Sie vor diesem Termin beim
Schreiben auf `Cookie::encrypted` und beim Lesen auf `read_encrypted_for` um.

Während des Fensters entstehen messbare Kosten. Eine fehlgeschlagene
Cookie-Entschlüsselung durchläuft den Ring zweimal. Die Session-Middleware
führt zwei verschlüsselte Lesevorgänge pro Anfrage aus, wenn sowohl ein
Session- als auch ein Remember-me-Cookie vorhanden ist; eine anonyme Anfrage
mit veraltetem Remember-Cookie zahlt daher `2 × (1 + N)` Versuche, wobei `N`
die Anzahl der vorherigen Schlüssel ist.

### `DecryptOrigin` lesen

`Crypt::decrypt_string_for_inner` gibt einen `DecryptOrigin` mit zwei
unabhängigen Achsen zurück:

- `origin.key = KeyOrigin::Previous(index)` bedeutet, dass der Wert weiterhin
  von `APP_KEY_PREVIOUS[index]` abhängt. Verschlüsseln Sie den Wert erneut
  unter dem aktuellen Schlüssel und entfernen Sie diesen vorherigen Schlüssel
  erst, nachdem der Rotationstail verschwunden ist.
- `origin.aad = AadVersion::Legacy` bedeutet, dass der Wert über den
  kontextlosen V1-Fallback gelesen wurde. Stellen Sie ein Cookie erneut über
  die namensgebundene API aus; die Rückfallroute ist zur Entfernung in 1.4.0
  vorgesehen.

Beide Achsen können gleichzeitig veraltet sein. Der öffentliche Leser
protokolliert die entsprechenden Warnungen, ohne Klartext oder Ciphertext
einzuschließen. Behandeln Sie die Schlüsselwarnung als Bereinigungsaufgabe für
die Rotation und die AAD-Warnung als Migrationsaufgabe; ein Treffer auf einer
Achse darf die andere nicht verdecken.

## Die zwei Encrypt-/Decrypt-Paare

Es gibt zwei Formen für zwei Anwendungsfälle.

### Strings - `encrypt_string` / `decrypt_string`

Für UTF-8-Strings:

```rust
use suprnova::{Crypt, CryptPurpose};

let wire: String =
    Crypt::encrypt_string(CryptPurpose::Cast, "alice@example.com")?;

let plain: String =
    Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
```

Der Decrypt-Pfad liefert einen `String` - Nicht-UTF-8-Bytes (die
ein normaler Encrypt-Durchlauf nicht erzeugen kann, ein korruptes
oder von einem Angreifer geliefertes Wire aber möglicherweise doch)
treten als eindeutiger `FrameworkError::Internal` zutage.

### Alles, was `Serialize` ist - `encrypt` / `decrypt`

Für strukturierte Werte: JSON-kodieren und dann verschlüsseln, in
einem Aufruf:

```rust
use serde::{Serialize, Deserialize};
use suprnova::{Crypt, CryptPurpose};

#[derive(Serialize, Deserialize)]
struct Secret {
    api_key: String,
    last_rotated_at: chrono::DateTime<chrono::Utc>,
}

let value = Secret {
    api_key: "sk_live_…".into(),
    last_rotated_at: chrono::Utc::now(),
};

let wire = Crypt::encrypt(CryptPurpose::Cast, &value)?;
let round_trip: Secret = Crypt::decrypt(CryptPurpose::Cast, &wire)?;
```

Das Wire-Format ist dasselbe - base64 über
`nonce || ciphertext || tag` - der einzige Unterschied ist, dass
der Klartext `serde_json`-Bytes von `value` statt UTF-8 eines
Strings sind. Verwenden Sie dies für jede Datensatzform: einen
Konfigurations-Blob, eine Session-Payload, ein
Queue-Argument-Tupel.

### `appears_encrypted` - Formprüfung, keine Manipulationsprüfung

Für Middleware, die bereits verschlüsselte Werte beim
Egress-Durchlauf überspringen muss (passend zum Verhalten von
Laravels `EncryptCookies`), führt `Crypt::appears_encrypted` eine
billige heuristische Prüfung durch:

```rust
if Crypt::appears_encrypted(cookie_value) {
    // durchlassen - bereits umschlossen
} else {
    // vor dem Senden verschlüsseln
}
```

Es liefert `true`, wenn die Eingabe sich als URL-sicheres base64
dekodieren lässt und die dekodierte Länge mindestens 28 Byte
beträgt (Nonce + Tag). Es ruft nie in AES-GCM hinein, kann also
**nicht** zwischen einem gültigen Chiffrat und zufälligen Bytes der
richtigen Form unterscheiden. Aufrufer, die Authentifizierung
brauchen, müssen `decrypt_string` / `decrypt` aufrufen und den
Fehler behandeln.

## Schlüsselrotation - der Schlüsselbund

Suprnova unterstützt Rotation ohne Ausfallzeit über einen
Schlüssel*bund*: einen aktuellen Schlüssel (verwendet für jede neue
Verschlüsselung) plus eine geordnete Liste vorheriger Schlüssel
(als Fallback beim Entschlüsseln versucht). Sie rollen `APP_KEY`,
ohne jede Spalte im Gleichschritt neu verschlüsseln zu müssen.

Setzen Sie `APP_KEY_PREVIOUS` auf eine kommagetrennte Liste von
base64-Schlüsseln, vom ältesten zum neuesten:

```env
APP_KEY=<new key>
APP_KEY_PREVIOUS=<old key>
# Für mehrstufige Rotation (älter → neuer):
APP_KEY_PREVIOUS=<oldest>,<middle>,<previous>
```

`APP_KEY_PREVIOUS` ist Suprnovas kanonischer Name.
`APP_PREVIOUS_KEYS` wird als Laravel-kompatibler Alias akzeptiert. Sind beide
Variablen gesetzt, gewinnt `APP_KEY_PREVIOUS`. Wenn ihre getrimmten Werte
abweichen, protokolliert der Bootvorgang eine Warnung und ignoriert
`APP_PREVIOUS_KEYS`.

Verschlüsselung verwendet **immer** den aktuellen Schlüssel.
Entschlüsselung versucht zuerst den aktuellen Schlüssel; schlägt
das fehl, wird jeder vorherige Schlüssel der Reihe nach versucht.
Bei einem Treffer auf einen vorherigen Schlüssel gibt `Crypt` ein
`tracing::warn!` aus:

```
WARN previous_index=0 Crypt decrypted a value with APP_KEY_PREVIOUS[0];
re-encrypt (load + save) this row under the current APP_KEY and remove
the corresponding APP_KEY_PREVIOUS entry once the rotation completes.
```

Die Log-Zeile lässt absichtlich sowohl den Klartext als auch das
Chiffrat aus - es reist nur die Tatsache der Rotation plus ein
umsetzbarer Hinweis mit. Operatoren, die eine Log-Suche nach
`APP_KEY_PREVIOUS` ausführen, landen bei jeder Spalte, die noch von
einem alten Schlüssel abhängt.

### Die Obergrenze - `MAX_PREVIOUS_KEYS = 8`

`APP_KEY_PREVIOUS` ist auf 8 Einträge begrenzt. Eine realistische
Rotationskette umfasst 1-3 Einträge (eine laufende Rotation,
vielleicht eine zuvor ins Stocken geratene, die der Operator nicht
aufgeräumt hat); 8 lässt reichlich Spielraum. Jenseits der
Obergrenze **scheitert der Boot sichtbar** mit einer
Diagnosemeldung, die sowohl die Anzahl als auch die Obergrenze
benennt:

```
APP_KEY_PREVIOUS holds 12 keys; the maximum is 8. A realistic
rotation chain is 1-3 entries - a longer list is almost always a
config-templating accident. Trim the list to the keys still needed
for in-flight rotation; once a re-encrypt job has migrated every
row off an old key, drop that entry.
```

Ein stilles Abschneiden würde einen Schlüssel verwerfen, auf den
der Operator möglicherweise noch angewiesen ist, und Spalten ohne
Diagnose unentschlüsselbar zurücklassen. Die harte Obergrenze ist
beabsichtigt.

Leere Einträge werden toleriert:
`APP_KEY_PREVIOUS=,,,old1,,,old2,,,` wird zu zwei echten
Schlüsseln geparst. Ein fehlerhafter Eintrag (Tippfehler, falsche
Länge, ungültiges base64) ist ein harter Fehler - halb rotierte
Secrets lassen den Boot fehlschlagen, statt still einen Fallback zu
verwerfen.

### Rotationsverfahren

```bash
# 1. Einen neuen Schlüssel prägen.
NEW=$(suprnova key:generate --show)

# 2. Den aktuellen Schlüssel nach APP_KEY_PREVIOUS verschieben, den neuen installieren.
#    Bearbeiten Sie Ihre .env oder Ihren Secrets-Manager:
#
#      APP_KEY_PREVIOUS=<old_value_of_APP_KEY>
#      APP_KEY=<NEW>

# 3. Deployen. Neue Schreibvorgänge verwenden den neuen Schlüssel;
#    bestehende Zeilen entschlüsseln weiterhin über den
#    Fallback auf den vorherigen Schlüssel. Logs identifizieren
#    Spalten, die noch auf dem alten Schlüssel liegen.

# 4. Einen Re-Encrypt-Durchlauf ausführen. Für jedes Modell mit verschlüsselten Casts:
#
#      User::query().chunk(500, |batch| async {
#          for mut row in batch { row.save().await?; }
#          Ok(())
#      }).await?;
#
#    `Cast::to_storage` verwendet immer den aktuellen Schlüssel,
#    sodass ein No-op-Load-then-Save die Zeile migriert.

# 5. Sobald keine Warnungen mehr in den Logs erscheinen, APP_KEY_PREVIOUS
#    entfernen und erneut deployen.
```

Das gesamte Verfahren läuft online - es gibt zu keinem Zeitpunkt
ein Fenster, in dem neue Anfragen fehlschlagen.

### Den Ring beobachten

Für Operator-Dashboards oder Health-Checks:

```rust
use suprnova::Crypt;

if Crypt::has_previous_keys() {
    let n = Crypt::previous_key_count();
    tracing::info!(previous_keys = n, "APP_KEY rotation in progress");
}
```

Die Schlüssel-Bytes selbst sind nie über die öffentliche API
zugänglich. Die `Debug`-Implementierung von `EncryptionKey` druckt
`"[REDACTED]"`, und es gibt keinen Accessor, der einen rohen
Schlüssel außerhalb der Crate offenlegt.

## Eloquent-Integration - die `AsEncrypted*`-Casts

Verschlüsselung auf Anwendungsebene ist an der Spaltengrenze am
nützlichsten. Die `AsEncrypted*`-Cast-Familie umschließt
`Crypt::encrypt_string`, sodass Ihre Modellfelder zur Laufzeit
typisierten Klartext bleiben und ruhend als Chiffrat vorliegen:

```rust
use suprnova::{model, Model};
use suprnova::eloquent::casts::{
    AsEncrypted, AsEncryptedArray, AsEncryptedObject, AsEncryptedCollection,
};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct ApiKey {
    pub provider: String,
    pub secret: String,
}

#[model(table = "users", casts = {
    api_token     = AsEncrypted,
    api_keys      = AsEncryptedArray<ApiKey>,
    billing       = AsEncryptedObject<BillingDetails>,
    ssh_keys      = AsEncryptedCollection<String>,
})]
pub struct User {
    pub id: i64,
    pub api_token: String,
    pub api_keys: Vec<ApiKey>,
    pub billing: BillingDetails,
    pub ssh_keys: suprnova::eloquent::Collection<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

| Cast | Laufzeit-Typ | Speicherform |
|---|---|---|
| `AsEncrypted` | `String` | verschlüsselter String |
| `AsEncryptedArray<T>` | `Vec<T>` | JSON → verschlüsselter String |
| `AsEncryptedObject<T>` | `T` | JSON → verschlüsselter String |
| `AsEncryptedCollection<T>` | `Collection<T>` | JSON → verschlüsselter String |

Alle vier laufen über `CryptPurpose::Cast`. Ein von einem
verschlüsselten Cast geprägtes Wire wird von jedem Code abgelehnt,
der versucht, es als Cookie oder Cursor zu entschlüsseln - selbst
wenn `APP_KEY` derselbe ist, unterscheidet sich das AAD-Label.

Für die vollständige Cast-Oberfläche, die Tabelle der Fehlermodi
und Rezepte zur erneuten Verschlüsselung siehe
[eloquent.md](eloquent.md). Die Verschlüsselungsmechanik ist
dieselbe wie bei der Facade oben - der Cast ist Zucker, der an der
Speichergrenze `Crypt::encrypt_string(CryptPurpose::Cast, …)`
ausführt.

### Verschlüsselung vs. Hashing - das richtige Werkzeug wählen

`AsEncrypted` ist **umkehrbar**. Der Klartext lässt sich mit
`APP_KEY` wiederherstellen. Verwenden Sie es für Daten, die Ihre
Anwendung zurücklesen muss: API-Tokens, die Sie auf einer
Einstellungsseite anzeigen, Secrets von Drittanbietern, die Sie an
vorgelagerte Dienste weiterleiten, Adressen, an die Sie
Bestellungen versenden.

Für Daten, die Ihre Anwendung nur jemals *verifizieren* muss -
Passwörter, API-Schlüssel-Präfixe, die Sie mit eingehenden Tokens
vergleichen - verwenden Sie stattdessen einen Hash. Hashes sind
einseitig: Es gibt keinen Klartext, der durchsickern könnte, selbst
wenn `APP_KEY` kompromittiert ist. Siehe [hashing.md](hashing.md)
für die Bcrypt-/Argon2id-Facade und den `AsHashed`-Cast.

## Wo `Crypt` sonst noch innerhalb des Frameworks verwendet wird

Sie müssen nichts tun, um sich dafür zu entscheiden - sie sind
automatisch verdrahtet, sobald `APP_KEY` konfiguriert ist.

- **Verschlüsselte Cookies** - `Cookie::encrypted(...)` /
  `Cookie::read_encrypted(...)` verwenden `CryptPurpose::Cookie`.
  Das Session-Cookie, das Remember-me-Cookie und das
  Wartungsmodus-Bypass-Cookie reiten alle darauf. Siehe
  [responses.md](responses.md) und [session.md](session.md).
- **Cursor-Paginierung** - `CursorPaginator` kodiert den Cursor
  unter `CryptPurpose::Cursor`, sodass der On-Wire-Wert
  `?cursor=…` nicht gefälscht oder oberflächenübergreifend per
  Replay eingespielt werden kann. Siehe
  [eloquent.md](eloquent.md#cursor-pagination).
- **2FA-Secrets** - das verschlüsselte base32-TOTP-Secret auf
  `two_factor_authentications.secret` verwendet
  `CryptPurpose::TwoFactorSecret`; Recovery-Codes verwenden
  `CryptPurpose::TwoFactorRecovery`. Unterschiedliche Zwecke
  verhindern einen zeileninternen, spaltenübergreifenden
  Chiffrat-Replay. Siehe [auth-flows.md](auth-flows.md).
- **HMAC-abgeleitete Signierung** - signierte URLs und
  Passwort-Reset-Tokens leiten einen HMAC-Schlüssel aus `APP_KEY`
  ab, statt darunter zu verschlüsseln. Die rohen Schlüssel-Bytes
  werden nicht exportiert; die Ableitung liegt innerhalb des
  Frameworks. Siehe [routing.md](routing.md#signed-urls).

## Testen mit `Crypt`

Die `Crypt`-Facade ist `OnceLock`-gestützt, sodass der erste
Installer in einer Test-Binary gewinnt. Die Test-Helfer übernehmen
den Boilerplate:

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn encrypts_and_round_trips() {
    install_test_encryption_key(); // idempotent - kann sicher aus jedem Test aufgerufen werden

    let wire = suprnova::Crypt::encrypt_string(
        suprnova::CryptPurpose::Cast,
        "hello",
    ).unwrap();

    let plain = suprnova::Crypt::decrypt_string(
        suprnova::CryptPurpose::Cast,
        &wire,
    ).unwrap();

    assert_eq!(plain, "hello");
}
```

Der Testschlüssel ist deterministisch, sodass Tests stabile Fixtures
entschlüsseln und die Rotation gegen einen bekannten Schlüssel prüfen können.
Chiffrat-Strings dürfen nicht über Aufrufe oder Läufe hinweg auf Gleichheit
verglichen werden: Jede Verschlüsselung verwendet weiterhin eine frische
zufällige Nonce.

Für Rotationstests installieren Sie einen Schlüsselbund direkt und
prägen historisches Chiffrat mit `_test_encrypt_with`:

```rust
use suprnova::testing::install_test_encryption_keyring;
use suprnova::EncryptionKey;

let current = EncryptionKey::generate();
let old = EncryptionKey::generate();

install_test_encryption_keyring(current, vec![old.clone()]);

// Simuliert einen Wert, der geschrieben wurde, als `old` aktuell war.
let legacy_wire = suprnova::crypto::_test_encrypt_with(
    &old,
    suprnova::CryptPurpose::Cast,
    "legacy",
).unwrap();

// Der aktuelle Ring entschlüsselt es über den Fallback auf den
// vorherigen Schlüssel und gibt die Rotations-Warnzeile aus.
let plain = suprnova::Crypt::decrypt_string(
    suprnova::CryptPurpose::Cast,
    &legacy_wire,
).unwrap();

assert_eq!(plain, "legacy");
```

Beide Helfer werden aus Produktions-Binaries herauskompiliert, wenn
das `testing`-Feature deaktiviert ist (`default-features = false`).

## Fehlermodi - wie Fehler aussehen

Jeder fehlschlagen könnende `Crypt::*`-Aufruf liefert
`Result<_, FrameworkError>`. Die fünf Fehler, die Sie sehen können:

| Ursache | Wo | Erscheint als |
|---|---|---|
| `Crypt` nicht initialisiert | Jeder Aufruf vor dem Boot | `FrameworkError::Internal("Crypt is not initialized - set APP_KEY before serving")` |
| Wire ist kein gültiges base64 | `decrypt_string`, `decrypt` | `FrameworkError::Internal("Crypt base64 decode failed: …")` |
| Wire zu kurz (< 28 Byte) | `decrypt_string`, `decrypt` | `FrameworkError::Internal("AEAD wire too short …")` |
| Tag-Prüfung schlägt fehl - falscher Schlüssel, falsche AAD, manipulierte Bytes | `decrypt_string`, `decrypt` | `FrameworkError::Internal("AEAD decrypt failed: …")` |
| JSON-Encode / -Decode schlägt fehl | `encrypt`, `decrypt` | `FrameworkError::Internal("Crypt JSON {encode,decode} failed: …")` |

Es gibt keinen stillen Fallback auf Müll. Ein falscher Schlüssel
gegen ein bestehendes Chiffrat ist immer ein harter Fehler, sowohl
auf Facade-Ebene als auch auf Cast-Ebene. Das entspricht dem
Verhalten von Laravels `Encrypter` und ist die Eigenschaft, die
Rotation sicher macht: Eine übersehene Spalte würde sofort zutage
treten, statt einen plausibel-aber-falschen Klartext zurückzugeben.

Wenn ein vorheriger Schlüssel ein Wire erfolgreich entschlüsselt,
liefert der Aufruf trotzdem `Ok(...)` - aber die
`tracing::warn!`-Zeile feuert zusätzlich, sodass log-basiertes
Alerting das Rotations-Nachspiel abfängt, bevor `APP_KEY_PREVIOUS`
entfernt wird.

## Nächste Schritte

- [configuration.md](configuration.md) - `APP_KEY`, `APP_ENV` und
  der Rest der Boot-Umgebung.
- [eloquent.md](eloquent.md) - die `AsEncrypted*`-Casts, die
  vollständige Cast-Tabelle und das Rotationsverfahren für
  Modellspalten.
- [hashing.md](hashing.md) - einseitige Alternative, wenn Sie
  *verifizieren* statt *wiederherstellen* müssen; Bcrypt- und
  Argon2id-Facades plus `AsHashed`.
- [auth-flows.md](auth-flows.md) - Speicherung von 2FA-Secrets und
  Recovery-Codes, die unter ihren eigenen Zwecken auf `Crypt`
  reiten.
- [session.md](session.md) - das Session-Cookie, verschlüsselt und
  signiert von `Crypt` über `CryptPurpose::Cookie`.

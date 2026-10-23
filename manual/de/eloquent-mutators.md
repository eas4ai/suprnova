# Eloquent Casts, Accessors & Mutators

Ein Cast vermittelt die Grenze zwischen dem, was eine Spalte auf der
Platte hält, und dem, was Ihr Modell im Speicher trägt. Ein Accessor
erfindet ein virtuelles Attribut aus den Spalten, die Sie bereits
haben. Ein Mutator leitet Schreibvorgänge auf ein Feld durch Ihre
eigene Transformation. Zusammen mit automatisch verwalteten
Timestamps sind sie die vier beweglichen Teile, die eine flache
Zeile in einen typisierten Rust-Wert verwandeln.

Dieses Kapitel behandelt die vollständige Cast-Oberfläche (jeder
eingebaute Typ, das Runtime-Override `casts!`, Verschlüsselung und
Hashing), die Attribut-Makros `#[accessor]` und `#[mutator]`, den
Auto-Timestamp-Vertrag einschließlich `touch()` und
`without_touching`, und das Lifecycle-Event `Replicating`, das
feuert, wenn Sie ein Modell mit `replicate()` klonen.

Für die breitere Modell-Oberfläche (`#[suprnova::model]`, Query
Builder, Relationen, Observer) siehe das Kapitel
[Eloquent API](eloquent.md). Für Lifecycle-Events end-to-end siehe
[Events & Listener](events.md). Für die Crypto-Facade, die die
verschlüsselten Casts verwenden, siehe [Verschlüsselung](encryption.md).

## Wie Casts funktionieren

Jeder Cast ist eine Struktur, die den Trait `Cast` implementiert:

```rust
pub trait Cast: Send + Sync {
    type Runtime;
    type Storage;

    fn to_storage(value: &Self::Runtime) -> Result<Self::Storage, FrameworkError>;
    fn from_storage(stored: &Self::Storage) -> Result<Self::Runtime, FrameworkError>;
}
```

`Runtime` ist der Rust-Typ, den Sie in Ihrer Modell-Struktur
schreiben (`bool`, `chrono::NaiveDate`, `rust_decimal::Decimal`, Ihr
eigenes Enum). `Storage` ist der Typ, den SeaORM auf der Spalte
sieht (`i64` für eine SQLite-Boolean-Spalte, `String` für ein
TEXT-Datum). Beide Richtungen sind fehlschlagbar - temporales und
Dezimal-Parsen kann fehlgeformte Eingabe zurückweisen - sodass das
Makro das `Result` durch `From<inner::Model>` und den
`ActiveModel`-Schreibpfad propagiert.

Casts sind explizit. Ein Feld `Vec<String>` wird nicht implizit zu
`AsArray<String>`, weil Feldtyp-Inspektion zur Makro-Zeit in dem
Moment brechen würde, in dem Sie einen Alias umbenennen oder einen
anderen `Vec` importieren. Sie deklarieren Casts auf dem
Makro-Attribut:

```rust
use suprnova::{model, AsArray, AsBool, AsJson};

#[model(
    table = "posts",
    casts = {
        tags = AsArray<String>,
        published = AsBool,
        metadata = AsJson<serde_json::Value>,
    },
)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub tags: Vec<String>,
    pub published: bool,
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Das Makro expandiert jeden Eintrag `field = CastType` zu Aufrufen von
`Cast::to_storage` und `Cast::from_storage` bei jedem Lesen und
Schreiben. Sie rufen den Cast nie selbst auf - Sie schreiben den
Runtime-Typ, der Cast verdrahtet die Spaltenform.

### Warum Suprnova abweicht

Laravel deklariert Casts als `protected $casts = ['tags' =>
'array']`. Der String `'array'` löst sich über einen
Laufzeit-Lookup zu einer Klasse auf, was bedeutet, dass Cast-Namen
als untypisierte Strings leben, bis sie laufen. Suprnova nimmt den
Typ direkt - `AsArray<String>` ist ein echter Rust-Typ, den das
Makro zur Compile-Zeit prüft. Ein Tippfehler im Cast-Namen ist ein
Compile-Fehler, keine Laufzeit-Exception drei Wochen nach dem
Deploy.

## Die primitiven Casts

Fünf Casts decken die SQL-Skalartypen ab.

### `AsBool`

`bool` ↔ `INTEGER` (0 / 1). SQLite hat keine native
Boolean-Spalte; Postgres und MySQL round-trippen beide `i64` sauber
durch SeaORMs `Value::Int`-Grenze. Eine einzige Storage-Form lässt
Sie denselben Cast gegen jedes Backend verwenden.

```rust
#[model(table = "settings", casts = { dark_mode = AsBool })]
pub struct Settings {
    pub id: i64,
    pub dark_mode: bool,
}
```

### `AsInt<I>`

Ein engerer Integer (`i32`, `u32`, `i16`) ↔ `i64`. SeaORM speichert
Integer als `i64` auf der Spalte; der Cast verengt beim Lesen und
weitet beim Schreiben. Werte außerhalb des Bereichs erzeugen einen
Validierungsfehler zur Lesezeit, statt stillschweigend abzuschneiden.

```rust
#[model(table = "counters", casts = { age = AsInt<u32> })]
pub struct Counter {
    pub id: i64,
    pub age: u32,
}
```

Verwenden Sie `AsInt<i64>` (oder lassen Sie den Cast weg), wenn der
Runtime-Typ bereits zum Storage passt.

### `AsFloat`

`f64` ↔ `REAL`. Pass-through in beide Richtungen - der Cast
existiert für Namensparität mit Laravels `'float'`-Cast; Backends
round-trippen Floats native.

### `AsString`

`String` ↔ `TEXT`. Ebenfalls Pass-through; der Cast existiert,
damit das Runtime-Override `Builder::with_casts(...)` ihn zu einem
`DynCast` löschen kann wie jeden anderen Cast.

### `AsDecimal<P>`

`rust_decimal::Decimal` ↔ `TEXT`. `P` ist die Präzision (Anzahl der
Dezimalstellen); Werte werden auf dem Weg zum Storage auf `P`
Stellen gerundet. Standard ist `P = 4`. Storage ist ein
festformatierter String, sodass Round-Trips backend-agnostisch sind -
SeaORMs native `Decimal`-Spaltentyp hat auf jedem Treiber
unterschiedliche Präzisions-Semantik, und der String-Round-Trip
vermeidet das.

```rust
use rust_decimal::Decimal;
use suprnova::AsDecimal;

#[model(
    table = "ledger",
    casts = { amount = AsDecimal<2> },  // Währung, 2 Nachkommastellen
)]
pub struct LedgerEntry {
    pub id: i64,
    pub amount: Decimal,
}
```

## Die temporalen Casts

Sechs Casts decken Datumswerte, Datetimes, unveränderliche Varianten
und Unix-Timestamps ab. Alle Casts außer dem Timestamp-Cast speichern
als `TEXT` (ISO-8601 / RFC-3339), sodass der Round-Trip auf jedem
Treiber funktioniert - SQLite speichert Datetimes nativ als Strings,
und Postgres / MySQL akzeptieren sie über SeaORMs
`Value::String`-Grenze.

### `AsDate`

`chrono::NaiveDate` ↔ `TEXT` (`YYYY-MM-DD`).

```rust
use chrono::NaiveDate;
use suprnova::AsDate;

#[model(table = "people", casts = { birthday = AsDate })]
pub struct Person {
    pub id: i64,
    pub birthday: NaiveDate,
}
```

### `AsDateTime`

`chrono::DateTime<Utc>` ↔ `TEXT` (RFC-3339). Der Standard-Cast für
beliebige Zeitstempel, wenn Sie eine Wanduhr-Darstellung wollen.

Schreibvorgänge werden auf RFC-3339 normalisiert. Beim Lesen werden
außerdem der von PostgreSQL ausgegebene native
`CURRENT_TIMESTAMP`-Text sowie zeitzonenfreie SQLite-/MySQL-Werte
akzeptiert; zeitzonenfreie Werte werden als UTC interpretiert.
`AsImmutableDateTime` und `AsOptionalDateTime` verwenden denselben
Parser.

### `AsImmutableDate` und `AsImmutableDateTime`

Dieselbe Storage-Form wie `AsDate` / `AsDateTime`. Rusts Borrow Checker
erzwingt Unveränderlichkeit bereits über `&`-Referenzen, sodass sich
diese Casts die zugrunde liegenden Typen teilen - sie existieren für
die Namensparität mit Laravels `immutable_date` / `immutable_datetime`
und um die Absicht an der Stelle der Modelldeklaration zu
dokumentieren.

### `AsOptionalDateTime`

`Option<DateTime<Utc>>` ↔ `Option<String>`. Wird vom Flag
`#[model(soft_deletes)]` für die nullable Tombstone-Spalte automatisch
eingefügt (standardmäßig `deleted_at` - siehe
[Soft Deletes](eloquent.md#deleting-and-soft-deletes)).
Die umschließende Option hält die Storage-Spalte nullable, sodass sich
soft-gelöschte und lebende Zeilen ohne Sentinel-Wert über `IS NULL`
unterscheiden.

Verwenden Sie den Cast direkt auf jeder anderen nullable
Datetime-Spalte, die Sie als RFC-3339-Text round-trippen wollen:

```rust
#[model(
    table = "subscriptions",
    casts = { cancelled_at = AsOptionalDateTime },
)]
pub struct Subscription {
    pub id: i64,
    pub cancelled_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

### `AsTimestamp`

`i64` als Unix-Epoche ↔ `INTEGER`. Verwenden Sie ihn, wenn die Spalte
als numerischer Bereich abgefragt oder in Arithmetik verwendet wird.
Abzugrenzen von `AsDateTime` - wählen Sie `AsTimestamp`, wenn Sie
`WHERE created_unix > 1700000000` wollen, und `AsDateTime`, wenn Sie
RFC-3339-Strings in Ihren Logs wollen.

## Die strukturierten Casts

Fünf Casts decken Collections, Strukturen und beliebiges JSON ab.
Alle serialisieren den Runtime-Wert zu JSON-Text und speichern ihn
in einer `TEXT`-Spalte. Postgres' native `JSON`- / `JSONB`- und
MySQLs `JSON`-Spalten akzeptieren dieselbe String-Payload - wenn Sie
für Indizierung einen nativen JSON-Spaltentyp wollen, deklarieren
Sie ihn manuell in einer Migration; die Cast-Schicht schränkt den
Spaltentyp nicht ein.

### `AsArray<T>`

`Vec<T>` ↔ JSON-kodiertes `TEXT`. Der Elementtyp muss `Serialize +
DeserializeOwned` sein.

```rust
use suprnova::AsArray;

#[model(table = "posts", casts = { tags = AsArray<String> })]
pub struct Post {
    pub id: i64,
    pub tags: Vec<String>,
}
```

### `AsObject<T>`

Eine `Serialize + DeserializeOwned`-Struktur ↔ JSON-kodiertes
`TEXT`. Verwenden Sie es, wenn die Runtime-Form ein fester Datensatz
mit statisch bekannten Schlüsseln ist.

```rust
use serde::{Deserialize, Serialize};
use suprnova::AsObject;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    pub theme: String,
    pub notifications: bool,
}

#[model(table = "users", casts = { prefs = AsObject<Prefs> })]
pub struct User {
    pub id: i64,
    pub prefs: Prefs,
}
```

### `AsCollection<T>`

`Collection<T>` ↔ JSON-kodiertes `TEXT`. Dünner Wrapper über
`AsArray`, der durch Suprnovas `Collection<T>` round-trippt (ein
`Vec<T>`-Newtype mit der Laravel-förmigen Slice-Oberfläche - siehe
[Collections](eloquent.md#collections)).

### `AsJson<T>`

Jeder `Serialize + DeserializeOwned`-Typ ↔ JSON-kodiertes `TEXT`.
Verwenden Sie es, wenn das Feld ein `serde_json::Value` oder eine
benutzerdefinierte Struktur ist, die bereits vollständig in
serde-Begriffen beschreibbar ist, aber nicht in das
Fest-Form-Muster von `AsObject` passt (z. B. Enum-Payloads,
untypisierte Maps).

### `AsArrayObject<T>`

`IndexMap<String, T>` ↔ JSON-kodiertes `TEXT`. Verwenden Sie es,
wenn die Runtime-Form eine Map mit dynamischen Schlüsseln ist und
die Reihenfolge der Schlüssel wichtig ist (die UI-Reihenfolge von
Labels, die kanonische Reihenfolge eines Config-Blocks). `IndexMap`
statt `HashMap` ist Absicht: serde erhält die Einfügereihenfolge
über `IndexMap`, und Suprnovas `serde_json` ist aus demselben Grund
bereits mit `preserve_order` konfiguriert.

Verwenden Sie für Datensätze mit fester Form `AsObject`; für Arrays
`AsArray`.

## Der Enum-Cast

### `AsEnum<E>`

`E: FromStr + AsRef<str>` ↔ `TEXT`. Der Variantenname des Enums
(oder sein per `AsRefStr` angepasster String) ist es, was in der
Spalte landet. Es gibt kein Framework-Lock-in auf `strum`, aber es
ist der ergonomischste Weg, an die beiden Schranken zu kommen, ohne
sie von Hand zu implementieren:

```rust
use suprnova::AsEnum;

#[derive(Debug, Clone, Copy, strum::EnumString, strum::AsRefStr)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

#[model(
    table = "users",
    casts = { role = AsEnum<Role> },
)]
pub struct User {
    pub id: i64,
    pub role: Role,
}
```

Integer-Diskriminante-Storage ist absichtlich nicht der Standard.
Ein `Role::Admin = 0`, das nach einer Neuordnung später zu
`Role::Admin = 2` wird, würde stillschweigend jeden Admin in der
Datenbank vertauschen. Variantennamen sind in einem DB-Browser
selbstbeschreibend und über Neuordnungen hinweg stabil.

## Verschlüsselung und Hashing

Fünf Casts vermitteln kryptografische Transformationen an der
Storage-Grenze. Alle vier `AsEncrypted*`-Casts teilen sich die
Facade [`Crypt`](encryption.md) - die Facade muss initialisiert
sein, bevor irgendeiner von ihnen läuft. Produktions-Apps bekommen
das über `Server::from_config` (das `APP_KEY` aus der Umgebung
liest); Tests rufen einmal beim Start
`suprnova::testing::install_test_encryption_key()` auf.

### `AsEncrypted`

`String` ↔ AES-256-GCM-verschlüsseltes `String`. Die Spalte auf der
Platte hält URL-sicheres Base64 von `nonce || ciphertext_with_tag`.
Jeder Schreibvorgang verwendet eine frische zufällige Nonce, sodass
zwei Schreibvorgänge desselben Klartexts unterschiedliche
Chiffretexte erzeugen - Ihr DB-Administrator kann doppelte
Geheimnisse im Ruhezustand nicht identifizieren.

```rust
use suprnova::AsEncrypted;

#[model(
    table = "secrets",
    casts = { api_key = AsEncrypted },
)]
pub struct Secret {
    pub id: i64,
    pub api_key: String,  // Runtime ist reines UTF-8
}
```

Der Runtime-Wert ist der entschlüsselte UTF-8-String; Sie lesen und
schreiben ihn wie jeden anderen `String`.

### `AsEncryptedArray<T>` / `AsEncryptedObject<T>` / `AsEncryptedCollection<T>`

`Vec<T>` / `T` / `Collection<T>` ↔ AES-256-GCM-verschlüsseltes JSON.
Die Pipeline ist: zu JSON serialisieren → verschlüsseln → Base64 →
speichern; umgekehrt beim Lesen. Element-/Werttyp muss `Serialize +
DeserializeOwned` sein.

```rust
use suprnova::AsEncryptedObject;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct CardOnFile {
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[model(
    table = "billing",
    casts = { card = AsEncryptedObject<CardOnFile> },
)]
pub struct Billing {
    pub id: i64,
    pub card: CardOnFile,
}
```

### Schlüsselrotation

Die Facade `Crypt` unterstützt Rotation über `APP_KEY_PREVIOUS`:
Verschlüsselung verwendet immer `APP_KEY`, aber Entschlüsselung
versucht zuerst `APP_KEY` und fällt auf `APP_KEY_PREVIOUS` zurück,
wenn der primäre Schlüssel fehlschlägt. Eine rollierende
Neuverschlüsselungsstrategie ist: `APP_KEY` auf den neuen Schlüssel
setzen, den alten Schlüssel nach `APP_KEY_PREVIOUS` verschieben,
dann jede verschlüsselte Zeile `save()`n, um Chiffretexte unter dem
neuen Schlüssel neu zu schreiben. Die Cast-Schicht muss nichts über
Rotation wissen - sie round-trippt bei jedem Lesen und Schreiben
durch `Crypt`, sodass ein `User::all().await?`, gefolgt vom Speichern
jeder Zeile, die Spalte an Ort und Stelle migriert. Siehe
[Verschlüsselung](encryption.md) für das vollständige
Rotationsprotokoll.

### `AsHashed`

`String` ↔ ein beim Schreiben gehashter String, unter Verwendung
des aktiven Hash-Treibers (Env-Variable `HASH_DRIVER` - standardmäßig
bcrypt, argon2i und argon2id werden auch unterstützt). Der
Runtime-Wert IST der gehashte String; es gibt keine umgekehrte
Richtung. Spiegelt Laravels `hashed`-Cast.

```rust
use suprnova::AsHashed;

#[model(
    table = "users",
    casts = { password = AsHashed },
)]
pub struct User {
    pub id: i64,
    pub password: String,
}
```

`AsHashed::to_storage` ist **idempotent**: Ein Wert, der bereits wie
IRGENDEIN erkannter Hash aussieht (bcrypt `$2*$`, argon2i / argon2id
PHC), läuft unverändert durch. Ohne diese Absicherung würde
`User::find(id).await?.save().await?` den bestehenden Hash zu einem
Hash-eines-Hashes rehashen, was `Hash::check(plain, stored)` brechen
und jedes bestehende Passwort ungültig machen würde.

Kombinieren Sie `AsHashed` mit dem `#[mutator]`-Muster (unten), wenn
Sie beim Schreiben mehr als einen Hash anwenden müssen - z. B.
Whitespace normalisieren oder leere Passwörter zurückweisen, bevor
gehasht wird.

## Runtime-Cast-Override - das Makro `casts!`

Die in `#[model(casts = { ... })]` deklarierten Casts sind statisch -
sie feuern bei jedem Lesen dieses Modells. Wenn Sie für eine
einzelne Query einen anderen Cast brauchen (ein Debug-Tool will die
rohe Storage-Form, ein Export-Skript will eine andere
JSON-Repräsentation), verwenden Sie `Builder::with_casts(...)`:

```rust
use suprnova::{casts, AsDate, AsJson, User};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

Das Makro `casts!` baut eine `HashMap<&'static str, Arc<dyn
DynCast>>`. Jeder Eintrag ist `field_name = CastType`; jeder
eingebaute Cast implementiert `IntoDynCast`, sodass der
typgelöschte `DynCast`-Schatten automatisch entsteht. Die
Runtime-Override-Map gilt nur für die Dauer der verketteten Query -
die statische Cast-Pipeline des Modells bleibt unverändert.

Verwenden Sie diese Oberfläche sparsam. Das Modell-Attribut ist der
richtige Ort für die Casts, die Sie bei jedem Lesen anwenden wollen;
das Runtime-Override ist der Notausgang für Einzelfall-Queries.

## Accessoren - virtuelle Attribute aus echten Spalten

Ein Accessor ist eine `impl`-Methode auf dem Modell, annotiert mit
dem Makro `#[accessor]`. Wenn Sie den Namen der Methode in
`#[model(appends = [...])]` aufführen, ruft `to_json()` des Modells
die Methode auf und fügt das Ergebnis unter diesem Schlüssel ein.

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    appends = ["full_name"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }
}
```

Ein `serde_json::to_value(&user)` (oder `user.to_json()`) enthält
jetzt:

```json
{
  "id": 1,
  "first_name": "Alice",
  "last_name": "Xu",
  "full_name": "Alice Xu"
}
```

Die Methode ist auch direkt aufrufbar (`user.full_name()`) - das
Makro `#[accessor]` ist größtenteils ein Marker, damit das
struktur-weite Makro `#[suprnova::model]` den `to_json()`-Dispatch
verdrahten kann. Es gibt keine Kosten, sie aus Ihrem eigenen Code
aufzurufen.

Jeder Name in `appends` muss per Identifier zu einer echten
`#[accessor]`-Methode passen. Ein Tippfehler (`appends =
["fullName"]`, wenn die Methode `full_name` heißt) wird zur
Compile-Zeit mit einer punktgenauen Fehlermeldung erkannt.

### Nicht-`String`-Werte zurückgeben

Accessoren können jeden `Serialize`-Typ zurückgeben. Das Makro
konvertiert den zurückgegebenen Wert vor dem Einfügen über
`serde_json::to_value`, also:

```rust
impl Post {
    #[accessor]
    pub fn word_count(&self) -> usize {
        self.body.split_whitespace().count()
    }
}
```

rendert als `"word_count": 42` in der JSON-Ausgabe.

### Die Quellspalten verstecken

Wenn der Wert des Accessors das ist, was der Konsument sehen soll,
und die zugrunde liegenden Spalten Lärm sind, kombinieren Sie
`appends` mit `hidden`:

```rust
#[model(
    table = "users",
    appends = ["full_name"],
    hidden = ["first_name", "last_name"],
)]
```

`hidden` streift die benannten Spalten aus der serialisierten
Ausgabe; `appends` fügt dann den Wert des Accessors ein. Die
Reihenfolge ist fest - Filter laufen zuerst, Accessor-Injektion
läuft danach. Siehe [Hidden, visible und
appends](eloquent.md#mass-assignment) für die vollständige
Oberfläche.

## Mutatoren - über Ihre Transformation geleitete Schreibvorgänge

Ein Mutator ist das Schreibseiten-Gegenstück. Wenn der Name des
Felds in `#[model(mutators = [...])]` erscheint, leitet jeder
Mass-Assignment-Pfad (`create` / `update`) den Wert durch
`self.set_<field>(value)?`, statt das Feld direkt zuzuweisen.

```rust
use serde_json::Value;
use suprnova::{model, mutator, FrameworkError, Model};

#[model(
    table = "users",
    fillable = ["password"],
    mutators = ["password"],
)]
pub struct User {
    pub id: i64,
    pub password: String,
}

impl User {
    #[mutator]
    pub fn set_password(&mut self, value: Value) -> Result<(), FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            FrameworkError::validation("password", format!("{e}"))
        })?;
        // Normalisieren + hashen; AsHashed würde das Hashen von
        // sich aus erledigen, aber im Mutator können Sie zusätzlich
        // Policy erzwingen.
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        self.password = suprnova::hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

`set_password` erhält einen `serde_json::Value`. Der Body besitzt
das Deserialisieren + die Transformation - der Feldtyp auf der
Struktur kann `String` bleiben, und Ihre Validierung läuft, bevor
die Spalte berührt wird. Ein zurückgegebener Fehler propagiert durch
`create()` / `update()` als `bad_request`.

Direkte Feldzuweisung umgeht den Mutator:

```rust
user.password = "raw".to_string();  // überspringt set_password
user.save().await?;                 // speichert "raw"
```

Das entspricht Laravels Verhalten `$user->password = ...` gegenüber
`$user->fill(...)`. Wenn Sie wollen, dass der Mutator der einzige
Pfad ist, leiten Sie alle Schreibvorgänge durch `attrs!` + `create`
/ `update`.

### Mutatoren mit Casts kombinieren

Ein Mutator und ein Cast können auf demselben Feld koexistieren; der
Mutator läuft auf dem Schreibpfad (wenn `create` / `update`
aufgerufen wird), der Cast läuft auf dem Lesepfad (wenn die Spalte
aus einem SELECT materialisiert wird). Ein gängiges Muster ist,
`AsHashed` für die Leseseiten-Idempotenz-Garantie zu verwenden und
den Mutator für Schreibseiten-Validierung - der Mutator hasht,
`AsHashed` sieht einen bereits gehashten Wert und läuft durch.

## Automatisch verwaltete Timestamps

Wenn ein Modell sowohl `created_at` als auch `updated_at` trägt
(typisiert als `chrono::DateTime<chrono::Utc>`), macht das Makro:

- Setzt beide auf `Utc::now()` bei `create()`.
- Erhöht `updated_at` bei jedem `save()` und `update(attrs)`.
- Gibt ein `impl Touchable for YourStruct` aus, sodass Sie
  `.touch().await` aufrufen können, um `updated_at` zu erhöhen, ohne
  eine andere Spalte zu ändern.

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model, Touchable};

#[model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// updated_at erhöhen, ohne sonst etwas zu ändern:
let post = Post::find_or_fail(1).await?;
post.touch().await?;
```

Storage verwendet den Cast `AsDateTime`, den das Makro für
Timestamp-Spalten automatisch einfügt. Der Cast lässt denselben
`DateTime<Utc>`-Wert über alle drei SeaORM-Treiber (SQLite, MySQL,
PostgreSQL) round-trippen, ohne Sie zu zwingen, einen
datenbankspezifischen Timestamp-Typ zu wählen.

### Opt-out und benutzerdefinierte Spaltennamen

`#[model(timestamps = false)]` deaktiviert die automatische
Verwaltung vollständig - Sie steuern die Timestamps selbst.

`#[model(created_at = "creado_en", updated_at = "actualizado_en")]`
behält die automatische Verwaltung, benennt aber die Spalten um.
Das Makro erkennt die umbenannten Felder und verdrahtet dieselbe
Logik gegen sie.

Wenn die Struktur nur EINES der beiden Timestamp-Felder hat, gibt
das Makro einen `compile_error!` aus - fast immer ein Tippfehler
(`craeted_at`), den Sie sichtbar aufgedeckt sehen wollen statt
stillschweigend verschluckt.

### `without_touching` - task-gescopte Unterdrückung

Manchmal wollen Sie eine Zeile aktualisieren, ohne `updated_at` zu
erhöhen - beim Ausführen eines Backfills, beim Beheben eines
Tippfehlers, beim Aufzeichnen einer internen Synchronisation, die
Cache-TTLs, die auf `updated_at` geschlüsselt sind, nicht
zurücksetzen soll. Wickeln Sie die Arbeit in `without_touching` ein:

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    for post in Post::query().get().await? {
        post.touch().await?;  // No-Op innerhalb des Scopes
    }
    Ok::<_, suprnova::FrameworkError>(())
}).await?;
```

Das Flag ist ein `tokio::task_local!`, sodass es nicht über
`tokio::spawn`-Grenzen hinweg leakt - gleichzeitige Anfragen auf
anderen Tasks respektieren weiterhin ihren eigenen Scope (oder dessen
Abwesenheit). Das ist Suprnovas Analogon zu Laravels
`Model::withoutTouching(closure)`.

### Warum Suprnova abweicht

Laravel verwendet eine statische `$timestamps = false`-Eigenschaft
und eine globale statische Methode `Model::withoutTouching`, die von
einem Instanz-Zähler gestützt wird. Beide Ansätze nehmen
Isolation-pro-Prozess-und-Anfrage an. Suprnova führt viele Anfragen
auf einer Tokio-Runtime aus, sodass ein prozessglobales Flag eine
Anfrage stillschweigend Timestamps auf einer anderen unterdrücken
lassen würde. Der `tokio::task_local!`-Scope ist async-bewusst: Er
folgt Futures über `.await`-Punkte hinweg innerhalb desselben Tasks
und geht außer Scope, wenn das Future gedroppt wird, egal wie die
Anfrage endet.

## Das Lifecycle-Event `Replicating`

Von den 16 Modell-Lifecycle-Events (siehe [Observer und
Lifecycle-Events](eloquent.md#observers-and-lifecycle-events)) ist
`Replicating` dasjenige, das feuert, wenn Sie eine bestehende Zeile
über `replicate()` in eine ungespeicherte In-Memory-Kopie klonen:

```rust
let original = Post::find_or_fail(1).await?;
let mut copy = original.replicate().await?;  // ungespeichert
copy.title = format!("{} (copy)", original.title);
copy.save().await?;  // jetzt mit einem neuen PK persistiert
```

Das Event `Replicating` feuert, NACHDEM der In-Memory-Klon gebaut
wurde, aber BEVOR Sie Gelegenheit hatten, ihn zu verändern. Listener
erhalten `(&Self, Arc<Mutex<Self>>)` - das Original und das frisch
gebaute Replikat hinter einem `Mutex`, sodass Sie das Replikat vom
Listener aus verändern können, bevor der Nutzer es sieht:

```rust
use suprnova::{Listener, FrameworkError};

pub struct ResetReplicatedFlags;

#[async_trait::async_trait]
impl Listener<post::events::Replicating> for ResetReplicatedFlags {
    async fn handle(&self, event: &post::events::Replicating) -> Result<(), FrameworkError> {
        let mut replica = event.replica.lock().await;
        replica.published = false;       // Kopien starten unveröffentlicht
        replica.view_count = 0;          // Zähler zurückgesetzt
        Ok(())
    }
}
```

Der PK des Replikats ist bereits geleert, wenn der Listener läuft -
`replicate()` ruft `reset_primary_key()` auf, bevor es das Event
feuert, sodass Sie nicht versehentlich unter der ursprünglichen ID
erneut speichern können. Timestamps werden ebenfalls zurückgesetzt;
`created_at` / `updated_at` feuern beim nachfolgenden `save()` wie
bei jeder neuen Zeile.

### `replicate_into<T>` - typübergreifende Replikation

Wenn das Replikat ein anderer Typ ist (`Post` → `Draft`, zum
Beispiel), verwenden Sie `replicate_into::<Draft>()`. Das Event
`Replicating` feuert auf diesem Pfad NICHT, weil die
Event-Struktur pro Quelltyp ist und ein für
`post::events::Replicating` registrierter Listener ein
`Arc<Mutex<Post>>` erhalten würde, kein `Arc<Mutex<Draft>>`. Der
typübergreifende Pfad ist dafür da, wenn Sie einen frischen Zieltyp
ohne Observer-Einmischung wollen; registrieren Sie einen normalen
`Creating`-Listener auf dem Zieltyp, wenn Sie einen Hook bei der
Konstruktion wollen.

Siehe [Replikation](eloquent.md#replication) für den Rest der
Replicate-Oberfläche (`replicate_except`, die
Relations-Behandlung des Replikats, die Regeln für nullbare PKs).

## Alles zusammensetzen

Ein Modell mit jeder Oberfläche aus diesem Kapitel:

```rust
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use suprnova::{
    accessor, hashing, model, mutator, AsBool, AsDateTime,
    AsDecimal, AsEncryptedObject, AsEnum, AsHashed, AsJson,
    AsOptionalDateTime, FrameworkError, Model,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardOnFile {
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[derive(Debug, Clone, Copy, strum::EnumString, strum::AsRefStr)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

#[model(
    table = "users",
    soft_deletes,
    appends = ["display_name"],
    hidden = ["password", "card"],
    fillable = ["name", "email", "password", "role", "credit"],
    mutators = ["password"],
    casts = {
        role = AsEnum<Role>,
        verified = AsBool,
        credit = AsDecimal<2>,
        card = AsEncryptedObject<CardOnFile>,
        metadata = AsJson<serde_json::Value>,
        password = AsHashed,
        last_login_at = AsOptionalDateTime,
    },
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: Role,
    pub verified: bool,
    pub credit: Decimal,
    pub card: CardOnFile,
    pub metadata: serde_json::Value,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // deleted_at wird von soft_deletes automatisch eingefügt (AsOptionalDateTime)
}

impl User {
    #[accessor]
    pub fn display_name(&self) -> String {
        if self.name.is_empty() { self.email.clone() } else { self.name.clone() }
    }

    #[mutator]
    pub fn set_password(&mut self, value: Value) -> Result<(), FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            FrameworkError::validation("password", format!("{e}"))
        })?;
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        // Der Mutator hasht; AsHashed sieht bei nachfolgenden Saves
        // einen bereits gehashten Wert und läuft unverändert durch.
        self.password = hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

Diese einzelne Deklaration gibt Ihnen:

- Acht typisierte Casts, die die Storage-/Runtime-Grenze verdrahten.
- Einen Accessor, der `display_name` aus bestehenden Spalten
  synthetisiert.
- Einen Mutator, der das Passwort validiert und hasht.
- Automatisch verwaltete `created_at` / `updated_at`.
- Soft Deletes mit einer automatisch eingefügten
  `deleted_at`-Spalte.
- Verschlüsseltes Card-on-File-Storage mit
  Schlüsselrotations-Unterstützung.

Jeder Cast wird zur Compile-Zeit geprüft. Der Dual-API-Query-Builder
(siehe [Eloquent - Query Builder](eloquent.md#query-builder--dual-api))
läuft gegen die typisierten Spalten; die Serialisierung zu Inertia /
JSON wendet die hidden-/appends-Regeln an; und ein
`User::find(id).await?` materialisiert die Zeile durch acht
`Cast::from_storage`-Aufrufe, ohne dass Sie eine einzige Zeile
Konvertierungscode schreiben.

## Nächste Schritte

- [Eloquent API](eloquent.md) - der Rest der Modell-Oberfläche:
  Query Builder, Relationen, Observer, Paginierung, Transactions.
- [Verschlüsselung](encryption.md) - die Facade `Crypt`, die sich
  die verschlüsselten Casts teilen, das Schlüsselrotationsprotokoll,
  und die breitere Crypto-Oberfläche.
- [Events & Listener](events.md) - der Dispatcher hinter
  `Replicating` und den anderen 15 Modell-Lifecycle-Events.
- [Authentifizierung](authentication.md) - der Trait
  `Authenticatable` und wo `AsHashed` in den Passwort-Flow passt.
- [Validierung](validation.md) - `FrameworkError::validation` und
  das Muster, das Mutatoren verwenden, um Fehler pro Feld
  aufzuzeigen.

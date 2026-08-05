# Eloquent Serialization

Wie Eloquent-Modelle zu JSON werden. Das Kapitel behandelt
`to_array()` und `to_json()`, die Filter-Pipeline `hidden` / `visible`
/ `appends`, die beiden terminalen Helfer `to_array_except` /
`to_array_only`, wie Appends Accessoren in die Ausgabe einbrücken,
und die zwei Abweichungen von Laravel, an denen man sich stößt: die
Serde-Bypass-Footgun, und die Tatsache, dass eager geladene
Relationen sich nicht automatisch in den JSON-Body einfalten.

Wenn Sie [Eloquent API](eloquent.md) gelesen haben, sind Ihnen die
meisten Namen hier vertraut - die Attributreferenz steht in jenem
Kapitel. Diese Seite ist der Ort, an dem der *Serialisierungsvertrag*
lebt: welche Felder erscheinen, in welcher Reihenfolge die Filter
greifen, und was zu einem Leck führt, wenn man es vergisst.

## Inhaltsverzeichnis

- [Der Vertrag](#der-vertrag)
- [`to_array` und `to_json`](#to-array-und-to-json)
- [Felder verstecken - `hidden = [...]`](#felder-verstecken-hidden)
- [Felder sichtbar machen - `visible = [...]`](#felder-sichtbar-machen-visible)
- [Accessoren anhängen - `appends = [...]`](#accessoren-anhängen-appends)
- [Die Reihenfolge der Filter-Pipeline](#die-reihenfolge-der-filter-pipeline)
- [Filterung pro Aufruf - `to_array_except` / `to_array_only`](#filterung-pro-aufruf-to-array-except-to-array-only)
- [Bedingtes Verstecken je nach Betrachter](#bedingtes-verstecken-je-nach-betrachter)
- [Die Serde-Bypass-Footgun](#die-serde-bypass-footgun)
- [Collections serialisieren](#collections-serialisieren)
- [Eager geladene Relationen und Serialisierung](#eager-geladene-relationen-und-serialisierung)
- [Was ist mit JSON:API?](#was-ist-mit-json-api)
- [Wo jedes Teil lebt](#wo-jedes-teil-lebt)
- [Nächste Schritte](#nächste-schritte)

## Der Vertrag

Jede `#[suprnova::model]`-Struktur bekommt vom Trait `Model` zwei
Serialisierungsmethoden:

```rust
fn to_array(&self) -> serde_json::Value;
fn to_json(&self) -> String;
```

`to_array` erzeugt einen `serde_json::Value` zur Verwendung in
Handler-Responses und Tests. `to_json` ist ein dünner Wrapper -
`serde_json::to_string(&self.to_array())` - sodass eine einzige
Filter-Pipeline beide Formen besitzt.

Die Ausgabe ist ein JSON-Objekt, geschlüsselt nach dem Namen des
Struktur-Felds (oder was auch immer Sie per serde umbenannt haben),
gefiltert durch drei optionale Regler, deklariert auf `#[model(...)]`:

- `hidden = [...]` - Spalten-Denylist
- `visible = [...]` - Spalten-Allowlist (schließt sich mit `hidden`
  gegenseitig aus)
- `appends = [...]` - Accessor-Methoden, die unter benannten
  Schlüsseln eingefügt werden

Deklariert das Modell keinen davon, läuft der Standard-Body des
Traits: `self` via `serde_json::to_value(self)` serialisieren, zwei
frameworkinterne Hilfsfelder entfernen (`__eager` und `__pivot` -
siehe [eager geladene
Relationen](#eager-geladene-relationen-und-serialisierung)), das
Ergebnis zurückgeben. Deklariert das Modell eines davon, generiert
das Makro eine Überschreibung, die die [Pipeline](#die-reihenfolge-der-filter-pipeline)
durchläuft.

## `to_array` und `to_json`

Das minimal nützliche Beispiel - eine Zeile geht als JSON zur Tür
hinaus:

```rust
use suprnova::{json_response, model, Model, Request, Response};
use chrono::{DateTime, Utc};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    json_response!(user.to_array())
}
```

`json_response!` akzeptiert jeden `serde_json::Value`;
`user.to_array()` erzeugt einen. Das string-förmige Äquivalent ist
`user.to_json()` - identischer Body, identische Filter, nur ein
zusätzliches `to_string`.

Sie können auch direkt zu `serde_json::to_value(&user)` greifen. **Tun
Sie das nicht für irgendetwas Nutzer-Sichtbares.** Es umgeht die
Filter-Pipeline vollständig - siehe [die
Serde-Bypass-Footgun](#die-serde-bypass-footgun) später im Kapitel,
warum.

## Felder verstecken - `hidden = [...]`

Die Denylist-Form. Jede Spalte außer den aufgeführten wird
serialisiert:

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
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
```

Das nutzer-sichtbare JSON für dieses Modell enthält niemals
`password` oder `remember_token`:

```json
{
    "id": 42,
    "name": "Alice",
    "email": "alice@example.com",
    "created_at": "2026-05-30T11:14:22Z",
    "updated_at": "2026-05-30T11:14:22Z"
}
```

`hidden` ist das richtige Werkzeug, wenn **die meisten Felder an den
Client gehen** und Sie nur eine kleine Menge an Geheimnissen,
internen Flags oder Auth-only-Daten abziehen müssen.

## Felder sichtbar machen - `visible = [...]`

Die Allowlist-Form. Nur die aufgeführten Spalten werden serialisiert:

```rust
#[model(
    table = "users",
    visible = ["id", "name", "avatar_url"],
)]
pub struct PublicUserView { /* ... */ }
```

Nützlich für ein Modell, das eigens dafür existiert, eine schlanke
öffentliche Projektion zu sein (denken Sie an Laravels
"Profile"-/"PublicUser"-Typen). `visible` ist auch das richtige
Werkzeug, wenn die Tabelle Dutzende interner Spalten hält und nur
wenige an den Client gehören - die Behaltungsmenge aufzulisten ist
kürzer, als die Streichungsmenge aufzulisten.

`hidden` und `visible` schließen sich **zur Compile-Zeit gegenseitig
aus**. Das Makro gibt einen Fehler aus, wenn Sie beide setzen:

```text
error: cannot specify both `hidden` and `visible` on the same model
 --> src/models/user.rs:7:1
  |
7 | #[model(table = "users", hidden = ["x"], visible = ["y"])]
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

Die beiden sind Politik-Gegensätze - wählen Sie den, dessen Absicht
zur Form Ihres Modells passt, nicht beide.

## Accessoren anhängen - `appends = [...]`

`appends` fügt berechnete Werte in die JSON-Ausgabe ein. Jeder
Eintrag nennt eine mit `#[accessor]` markierte Methode auf dem
Modell; das Makro ruft sie während `to_array()` auf und speichert den
Rückgabewert unter demselben Schlüssel.

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    fillable = ["first_name", "last_name"],
    appends = ["full_name", "initials"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[accessor]
    pub fn initials(&self) -> String {
        let f = self.first_name.chars().next().unwrap_or(' ');
        let l = self.last_name.chars().next().unwrap_or(' ');
        format!("{f}{l}")
    }
}
```

Der serialisierte User trägt jetzt beide berechneten Schlüssel:

```json
{
    "id": 7,
    "first_name": "Alice",
    "last_name": "Pond",
    "created_at": "...",
    "updated_at": "...",
    "full_name": "Alice Pond",
    "initials": "AP"
}
```

Das Makro validiert `appends`-Einträge zur Compile-Zeit:

- Jeder Name muss als Rust-Identifier parsen (`"full-name"`
  schlägt fehl - kein gültiger Ident).
- Existiert die benannte Methode nicht im `impl`-Block des Modells,
  zeigt der Compiler mit einer klaren Fehlermeldung
  `no method named 'full_name' found` auf den makro-generierten
  Dispatcher.

Direktes Aufrufen von `user.full_name()` aus Rust funktioniert genau
wie jede andere Methode - `appends` steuert nur die
**JSON-Dispatch-Tabelle**. Accessoren bleiben gewöhnliche Methoden.

## Die Reihenfolge der Filter-Pipeline

Deklariert ein Modell irgendeines von `hidden`, `visible` oder
`appends`, generiert das Makro eine `to_array`-Überschreibung, die
vier Schritte in dieser Reihenfolge durchläuft:

1. `self` via `serde_json::to_value` zu einer `serde_json::Map`
   serialisieren.
2. Die frameworkinternen Schlüssel `__eager` und `__pivot`
   bedingungslos entfernen (mehr dazu im [Abschnitt zu den
   Relationen](#eager-geladene-relationen-und-serialisierung)).
3. `visible` als **Allowlist** anwenden, wenn nicht leer: Jeder
   Schlüssel, der NICHT in der Liste steht, wird entfernt.
4. `hidden` als **Denylist** anwenden: Jeder aufgeführte Schlüssel,
   der die Allowlist überlebt hat, wird entfernt.
5. `appends` einfügen: Für jeden Eintrag den registrierten Accessor
   aufrufen und dessen Ergebnis unter dem Namen des Eintrags
   einfügen.

### Warum Suprnova abweicht

Laravel durchläuft dieselbe Reihenfolge `hidden` → `visible` →
`appends`. Die Abweichung liegt in Schritt 5: In Suprnova laufen
Appends **nach** der Hidden-Denylist, und sie erscheinen immer -
selbst wenn ihr Name auch in `hidden` aufgeführt ist. Die Begründung
ist dieselbe wie bei Laravel: Deklarieren Sie sowohl `$appends =
['full_name']` als auch `$hidden = ['full_name']`, ist die Absicht
"berechnen und ausliefern" - `appends` ist das spezifischere Signal.
Die Reihenfolge ist wichtig, wenn der Schlüssel eines Accessors mit
einem Spaltennamen kollidiert (z. B. ein Accessor, der den Wert der
gespeicherten Spalte `display_name` überschreibt); der Accessor
gewinnt auf dem Weg zum Client.

## Filterung pro Aufruf - `to_array_except` / `to_array_only`

Für Einzelfälle, in denen die Spaltendeklaration nicht passt, laufen
zwei terminale Helfer die vollständige `to_array`-Pipeline und
beschneiden das Ergebnis dann nach Namen:

```rust
use suprnova::{json_response, Model};

pub async fn admin_show(user: User) -> suprnova::Response {
    // ein paar zusätzliche Felder für einen Admin-Endpunkt abziehen,
    // der die meiste Zeile braucht, aber nicht diese hier:
    json_response!(
        user.to_array_except(&["password_hash", "remember_token", "internal_notes"])
    ))
}

pub async fn directory_show(user: User) -> suprnova::Response {
    // öffentliches Verzeichnis - nur die Spalten, die wir veröffentlichen wollen:
    json_response!(
        user.to_array_only(&["id", "name", "avatar_url"])
    ))
}
```

Beide erzeugen einen `serde_json::Value` - sie verändern nicht
`self`, und sie ändern nicht künftige Serialisierungen derselben
Zeile. Sie durchlaufen zuerst die vollständige Pipeline `hidden` /
`visible` / `appends` und wenden dann ihren eigenen Zuschnitt darauf
an. `to_array_only` gibt ein *frisches* JSON-Objekt zurück, das nur
die benannten Schlüssel enthält; `to_array_except` gibt das
vollständige Objekt minus der benannten Schlüssel zurück.

### Warum Suprnova abweicht

Laravels `$user->makeHidden(['x'])` und `$user->makeVisible(['x'])`
**verändern** die Modellinstanz - jeder nachfolgende
`toArray()`-Aufruf, einschließlich solcher, die auftreten, wenn das
Modell in die Serialisierung eines Elternteils verschachtelt ist,
sieht den geänderten Zustand. Suprnovas Helfer sind **terminal**. Sie
erzeugen einen `Value` und hören dort auf. Wenn die Änderung
weitergegeben werden soll, deklarieren Sie sie auf
`#[model(hidden = [...])]` / `#[model(visible = [...])]`, damit der
*Typ* die Policy ausdrückt, nicht eine verborgene Mutation an der
Instanz.

Der Rust-förmige Grund: Eine Eloquent-Struktur in Suprnova ist eine
gewöhnliche Rust-Struktur ohne Laufzeit-Attribut-Bag. Es gibt keinen
Platz, an dem ein instanzseitiges Sichtbarkeitsflag leben könnte,
ohne zusätzlichen ambienten verborgenen Zustand einzuführen - genau
die Art von Footgun, die das Framework absichtlich vermeidet.

## Bedingtes Verstecken je nach Betrachter

Das idiomatische Muster, wenn Sichtbarkeit vom Betrachter abhängt,
ist ein Match an der Aufrufstelle, der in den passenden
Pro-Aufruf-Filter verzweigt:

```rust
use suprnova::{Auth, json_response, Model, Request, Response};

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    let viewer = Auth::user_as::<User>().await?;
    let viewing_self = viewer.as_ref().map(|v| v.id) == Some(user.id);

    let body = if viewing_self {
        user.to_array()
    } else {
        user.to_array_except(&["email", "phone", "stripe_customer_id"])
    };

    json_response!(body)
}
```

Für aufwendigere Pro-Betrachter-Formen - unterschiedliche Attribute
für Admins, Testnutzer, zahlende Nutzer - ist das richtige Werkzeug
die **JSON:API-Resource-Schicht** mit `Maybe<T>`- /
`MissingValue<T>`-Feldern. Siehe
[JSON:API resources](eloquent-resources.md#conditional-attributes--maybet--missingvaluet)
für die deklarative Form.

## Die Serde-Bypass-Footgun

Das ist die wichtigste einzelne Sache, die man über Eloquent
Serialization in Suprnova wissen muss.

**Die Filter `hidden` / `visible` / `appends` laufen nur über
`to_array()` und `to_json()`.** Sie werden von der abgeleiteten
`Serialize`-Implementierung *nicht* erzwungen. Die Struktur über
irgendeinen anderen serde-Pfad zurückzugeben, umgeht die Filter
vollständig.

Das bedeutet, **all diese Varianten lassen `password`
durchsickern**:

```rust
// Direktes serde - umgeht to_array, hidden hat keine Wirkung:
let raw = serde_json::to_value(&user).unwrap();

// json_response! mit einem Struktur-Feld - dasselbe:
json_response!({ "user": user }))

// Verschachtelt in einem anderen serialisierbaren Container - dasselbe:
#[derive(Serialize)]
struct EnvelopeWithUser { ok: bool, user: User }
let env = EnvelopeWithUser { ok: true, user };
json_response!(env))

// Einen Vec<User> über serde zurückgeben - dasselbe:
json_response!(users))   // wobei users: Vec<User>
```

Nur diese durchlaufen die Filter-Pipeline:

```rust
json_response!(user.to_array()))
json_response!(users_collection.to_array()))  // Collection<User>
json_response!(user.to_array_except(&["secret"])))
json_response!(user.to_array_only(&["id", "name"])))
```

### Warum das passiert

Serdes pauschales `Serialize for Vec<T>` (und jeder andere Container)
ruft `T::serialize` direkt auf. Suprnovas Filter-Pipeline lebt in der
Trait-Methode `Model::to_array`, nicht in `Serialize`. Die
Trait-Methode wird nicht aufgerufen, außer Sie rufen sie selbst auf.

Das Framework schützt vor der *internen* Footgun (die
Hilfsfelder `__eager` / `__pivot` sind mit `#[serde(skip)]`
markiert, sodass sie über keinen der beiden Pfade durchsickern),
aber das Makro gibt absichtlich **kein** `#[serde(skip_serializing)]`
auf versteckten Feldern aus - das zu tun würde legitime Verwendungen
von serde mit dem inneren SeaORM-Modell brechen, wo ein Aufrufer die
vollständige Zeile will (z. B. interne RPC, Persistenz-Schichten,
Diagnosen, Tests).

### Die Regel

Für jeden Wert, der die Vertrauensgrenze zurück zu einem Client
überschreitet, gehen Sie über `to_array()` oder einen seiner
gefilterten Verwandten. Der vierzeilige Vertrag, der Ihnen die
Sicherheit kauft:

| Ziel | Verwenden | Ergebnis |
|---|---|---|
| Ein Modell serialisieren | `user.to_array()` | Gefiltertes JSON-Objekt |
| Eine Collection serialisieren | `collection.to_array()` | Gefiltertes JSON-Array |
| Ein paar Felder abziehen | `user.to_array_except(&["x"])` | Gefiltert + abgezogen |
| Nur ein paar Felder behalten | `user.to_array_only(&["x"])` | Nur aufgeführte Schlüssel |

Ein Linter oder eine PR-Zeit-Review für
`json_response!\({.*: [a-z_]+ ?})` und `serde_json::to_value\(&\w+\)`
auf Modellwerten ist ein günstiger Weg, die Regel einzuhalten. Die
eigenen Tests des Frameworks für die `Model`-Serialisierung decken
beide Pfade ab.

## Collections serialisieren

Eine `Collection<M>` - zurückgegeben von `Builder::get()`,
`Model::all()` und Relations-Accessoren - hat ihre eigenen
`to_array()` und `to_json()`, die den zugrunde liegenden `Vec<M>`
durchlaufen und **pro Zeile** `to_array()` aufrufen. Das Ergebnis ist
ein JSON-Array gefilterter Objekte:

```rust
use suprnova::{json_response, Model};

pub async fn list() -> suprnova::Response {
    let users = User::all().await?;
    json_response!(users.to_array())
}
```

Das ist die einzige Stelle, um den Pro-Zeile-Filter bei einem
Mehrzeilen-Ergebnis zu bekommen. `serde_json::to_value(&users)` würde
über serdes pauschale Implementierung einen Vec ausgeben und die
Filter auf jeder Zeile gleichzeitig umgehen - der
Collection-Level-Helfer existiert genau, um diese Lücke zu
schließen.

```rust
// Die Collection<M>-Überschreibung:
pub fn to_array(&self) -> Value {
    Value::Array(self.0.iter().map(|m| m.to_array()).collect())
}
```

Bei einem Paginator lebt die eingepackte Datenmenge in
`LengthAwarePaginator::data` / `CursorPaginator::data` und ist ein
`Vec<M>` - rufen Sie `.to_array()` auf jedem Element auf, bevor Sie
die Paginator-Response zusammensetzen, oder verwenden Sie die
[paginierte JSON:API-Form](eloquent-resources.md#pagination), die
die Pro-Zeile-Filterung als Teil der Resource-Pipeline übernimmt.

## Eager geladene Relationen und Serialisierung

Das ist die zweite Abweichung, die man verinnerlichen muss.

Wenn Sie `.with(["posts"])` auf einem Builder aufrufen, lädt das
Framework die Posts und speichert sie in einem Pro-Zeile-`EagerLoadCache`
(dem automatisch eingefügten Feld `__eager`). Der Accessor zum Lesen
davon - `user.posts_loaded()` - zieht aus diesem Cache.

**Der Cache ist `#[serde(skip)]`, und `to_array()` entfernt ihn
bedingungslos.** Eager geladene Relationen fließen nicht automatisch
in die JSON-Ausgabe ein. Ein `to_array()` auf einem User mit eager
geladenen Posts sieht identisch aus zu einem `to_array()` auf einem
User ohne.

### Warum Suprnova abweicht

Laravels `toArray()` durchläuft `$model->getRelations()` und faltet
jede geladene Relation in die Ausgabe ein. PHPs array-förmiger
Modell-Bag macht das natürlich - eine Relation ist einfach ein
weiterer geschlüsselter Eintrag auf dem Modell.

Rusts typisierte Eloquent-Strukturen haben diesen Bag nicht. Eine
`User`-Struktur hat typisierte Spalten, keine heterogene Map von
"was auch immer an Relationen geladen wurde". `posts` einfließen zu
lassen würde entweder Laufzeit-Feldinjektion auf einer typisierten
Struktur erfordern (ein Serde-Bypass-Mechanismus), oder einen
parallelen Serialisierungspfad, der den Cache befragt, nachdem der
Spalten-Serialisierer gelaufen ist. Beide Optionen würden die
JSON-Form jedes Modells daran koppeln, welche Relationen ein
bestimmter Aufrufer eager geladen hat - ein Vertrag, der in PHP
tragend ist, weil Clients lernen, sich darauf zu verlassen, und ein
Vertrag, den Suprnova bewusst ablehnt zu liefern, weil er die
JSON-Form von der aufruferseitigen Query-Konstruktion abhängig
macht.

### Die zwei Wege, Relationsdaten auszuliefern

**1. Expliziter Accessor + Appends.** Definieren Sie eine Methode,
die aus `<rel>_loaded()` zieht, und registrieren Sie sie in
`appends`. Die Relation erscheint unter dem Schlüssel, den Sie
benennen. Das funktioniert, wenn die Relation auf dem Lesepfad
*immer* eager geladen wird:

```rust
use suprnova::{accessor, model};
use serde_json::Value;

#[model(
    table = "users",
    appends = ["posts"],
)]
pub struct User { /* ... */ }

impl User {
    #[accessor]
    pub fn posts(&self) -> Value {
        // posts_loaded() panickt, wenn .with(["posts"]) auf dem
        // Lesepfad nicht aufgerufen wurde. Der Accessor MUSS nach
        // dem Eager Loading laufen.
        let posts = self.posts_loaded();
        serde_json::to_value(posts).unwrap_or(Value::Null)
    }
}

// Der Lesepfad MUSS eager laden:
let users = User::query()
    .with(["posts"])
    .get()
    .await?;
let body = users.to_array();   // der Schlüssel "posts" jedes Users ist befüllt
```

Der Vertrag ist unübersehbar: Vergessen Sie `.with(["posts"])`, panickt der
Accessor beim `posts_loaded()`-Aufruf der ersten Zeile (der
Eager-Cache panickt beim Lesen, wenn die Relation nicht geladen
wurde, per Design - ein stillschweigendes leeres Array würde den Bug
verstecken). Für optionales Eager Loading verwenden Sie die
HasOne-Form, die `Option<&T>` zurückgibt und Ihnen ein `match` gibt:

```rust
impl User {
    #[accessor]
    pub fn profile(&self) -> Value {
        match self.profile_loaded() {
            Some(profile) => serde_json::to_value(profile).unwrap_or(Value::Null),
            None => Value::Null,
        }
    }
}
```

**2. Die JSON:API-Resource-Schicht.** Wenn die Relationsform und die
Include-Policy auf das Wire-Format gehören statt auf das Modell,
verwenden Sie eine `#[derive(Data)] #[json_resource]`-Struktur mit
`#[data(allow_include)]` auf dem Relationsfeld. Clients steigen über
`?include=posts.comments` ein, das Framework durchläuft den
Include-Tree und befüllt `included` mit deduplizierten
Resource-Objekten. Das ist die richtige Antwort, wenn:

- Die Relationsform eine Wire-Format-Angelegenheit ist (Sparse
  Fieldsets, bedingte Inclusion, Cross-Link-Metadaten).
- Verschiedene Endpunkte unterschiedliche Standard-Inclusions
  wollen.
- Dasselbe Modell unter verschiedenen Envelopes erscheint (ein
  Endpunkt liefert `posts`, ein anderer liefert `subscriptions`).

Siehe [JSON:API resources](eloquent-resources.md#compound-documents--include-chains)
für das vollständige Muster.

## Was ist mit JSON:API?

Die `to_array()`-Pipeline und die `Resource`- / `JsonApi`-Facade sind
zwei Schichten und dienen unterschiedlichen Aufgaben:

| Belang | `Model::to_array` | `Resource::single` / `JsonApi::single` |
|---|---|---|
| **Form** | Flaches Objekt - Spaltennamen bilden direkt auf Schlüssel ab | JSON:API-Envelope (`data`, `included`, `meta`, `links`, `jsonapi`) |
| **Kontrolle pro Attribut** | `hidden` / `visible` / `appends` auf `#[model]` | `#[data(input_only)]`, `Maybe<T>`, Sparse Fieldsets über `?fields[type]=` |
| **Relationen** | Manuell (Accessor + Appends, siehe oben) | Erstklassig über `#[data(allow_include)]` + `?include=` |
| **Paginierung** | Einen `Vec<Value>` von Hand einwickeln | `Resource::paginated(p)` übernimmt Links + Meta |
| **Fehler** | Über `FrameworkError` rendern | `into_json_api_response()` erzeugt eine JSON:API-`errors`-Envelope |
| **Wann darauf zurückgreifen** | Einfache Endpunkte, interne Tools, Ad-hoc-Formen | Öffentliche APIs, Drittanbieter-Konsumenten, JSON:API-bewusste Clients |

`to_array()` ist die untere Schicht - sie ist es, was für die
meisten internen Handler, Admin-Seiten, Inertia-Props (über serde)
und Tests aufgerufen wird. Die JSON:API-Schicht setzt darauf auf: Sie
ersetzt `to_array` nicht, sie fügt eine Envelope um
Pro-Resource-Attribut-/Relations-Logik hinzu, die zu reichhaltig ist,
um auf dem Modell selbst zu leben.

Für typisierte Inertia-Props wollen Sie fast immer die
Resource-Schicht oder ein dediziertes `#[derive(Serialize)]`-DTO mit
expliziten Feldern, statt das Modell direkt durch serde zu leiten.
Inertia-Returns bekommen dieselbe Serde-Bypass-Behandlung wie alles
andere - der sichere Weg ist "ein DTO bauen, es aus `to_array()`
befüllen, das DTO zurückgeben".

## Wo jedes Teil lebt

| Belang | Datei |
|---|---|
| Trait-Standards `Model::to_array` / `to_json` | `framework/src/eloquent/model.rs` |
| `Model::to_array_except` / `to_array_only` | `framework/src/eloquent/model.rs` |
| Trait-Standard `Model::__append_accessor` | `framework/src/eloquent/model.rs` |
| Makro-generierte `to_array`-Überschreibung (Filter-Pipeline) | `suprnova-macros/src/model/serialization.rs` |
| Makro-generierter `__append_accessor`-Dispatcher | `suprnova-macros/src/model/serialization.rs` |
| `Collection<M>::to_array` / `to_json` | `framework/src/eloquent/collection.rs` |
| `EagerLoadCache` (das Feld `__eager`) | `framework/src/eloquent/relations/eager_cache.rs` |
| Makro-Parsing von `hidden` / `visible` / `appends` | `suprnova-macros/src/model/parse.rs` |
| Funktionsebenen-Makro `#[accessor]` | `suprnova-macros/src/lib.rs` |

## Nächste Schritte

- [Eloquent API](eloquent.md) - die vollständige Modell-Oberfläche,
  die Attributreferenz, und wo `#[accessor]` / `#[mutator]`
  definiert sind
- [JSON:API Resources](eloquent-resources.md) - die deklarative
  Resource-Schicht für reichhaltigere Pro-Betrachter-Formen,
  Sparse Fieldsets und zusammengesetzte `?include=`-Dokumente
- [Validierung](validation.md) - wie Request-Input zu einer
  typisierten Struktur wird, bevor die Modell-Schicht sie sieht
- [Antworten](responses.md) - `HttpResponse`-Builder, Header und
  Cookies; die Oberfläche, die `json_response!` letztlich erzeugt
- [Fehlermodell](error-model.md) - wie ein Fehler zu einem JSON-Body
  wird, mit derselben `request_id`-Korrelation wie der Erfolgspfad

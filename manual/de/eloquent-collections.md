# Eloquent Collections

`Collection<T>` ist Suprnovas Collection-Typ nach Laravel-Vorbild -
der Rückgabewert von `Builder::get`, `Model::all`, jedem `pluck`,
jedem Relations-Lade-Terminal, das mehr als eine Zeile liefert. Es
ist ein dünner Wrapper um `Vec<T>`, der zu `&[T]` dereferenziert,
sodass jede vorhandene Slice-Methode (`.len()`, `.iter()`,
Indizierung, `.contains(&v)`) unverändert funktioniert. Darüber
liegt die Laravel-Oberfläche: `map`, `filter`, `pluck`, `group_by`,
`sort_by`, `where_eq`, `sum`, `avg`, und so weiter.

Dieses Kapitel ist die eigenständige Referenz für die
Collection-Oberfläche. Das übergeordnete [Eloquent API](eloquent.md)
fasst sie zusammen; dieses Kapitel geht jede Methode durch, den
Borrow-vs-Verbrauch-Vertrag, die Serialisierungsregel, die zuschlägt,
wenn man sie übergeht, und wann man stattdessen zu `Vec<T>`
wechselt.

## Inhaltsverzeichnis

- [Woher Collections kommen](#woher-collections-kommen)
- [Die zwei `impl`-Blöcke](#die-zwei-impl-blöcke)
- [Generische Oberfläche - funktioniert mit jedem `Collection<T>`](#generische-oberfläche-funktioniert-mit-jedem-collection-t)
- [Modellbewusste Oberfläche - `Collection<M>`, wobei `M: Model`](#modellbewusste-oberfläche-collection-m-wobei-m-model)
- [Eager Loading auf einer Collection](#eager-loading-auf-einer-collection)
- [Serialisierung - `to_array` vs serde](#serialisierung-to-array-vs-serde)
- [Ausleihen vs Verbrauchen](#ausleihen-vs-verbrauchen)
- [Collection vs `Vec`](#collection-vs-vec)
- [`LazyCollection<M>` - Streaming-Ergebnisse](#lazycollection-m-streaming-ergebnisse)
- [Warum Suprnova abweicht](#warum-suprnova-abweicht)
- [Nächste Schritte](#nächste-schritte)

## Woher Collections kommen

Jedes Terminal, das mehr als eine Zeile zurückgibt, händigt Ihnen
eine `Collection<M>` aus:

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::all().await?;
let admins: Collection<User> = User::query()
    .db_where("role", "=", "admin")
    .get()
    .await?;
let recent: Collection<User> = User::query()
    .order_by_desc("created_at")
    .limit(50)
    .get()
    .await?;
```

Sie können auch jeden bereits vorhandenen `Vec<T>` einpacken:

```rust
let from_vec: Collection<User> = users_vec.into();
let from_vec2: Collection<User> = Collection::from_vec(users_vec);
let empty: Collection<User> = Collection::new();
```

`Collection<T>` implementiert `Default`, `Clone`, `Serialize`,
`Deserialize`, `PartialEq` und `IntoIterator` (sowohl per Wert als
auch per `&`). Es ist `Send`, wenn `T: Send`.

## Die zwei `impl`-Blöcke

Die Methoden auf `Collection` teilen sich in zwei Familien, je nach
Typparameter.

```rust
impl<T> Collection<T> { /* generische Methoden - funktionieren für jedes T */ }

impl<M> Collection<M> where M: Model { /* string-geschlüsselte Modell-Methoden */ }
```

Der generische Block gibt Ihnen `map`, `filter`, `reject`, `chunk`,
`first`, `last`, `unique` und eine closure-basierte Version jedes
Spalten-Accessors (`pluck_by`, `group_by_with`, `sort_with`,
`key_by_with`). Diese funktionieren auf `Collection<i32>`,
`Collection<String>`, `Collection<MyDto>`, allem.

Der modellbewusste Block fügt string-geschlüsselten Zucker hinzu
(`pluck("name")`, `group_by("role")`, `sort_by("created_at")`,
`sum::<f64>("balance")`), der pro Zeile durch den makro-generierten
Accessor `Model::field_value` läuft. Diese existieren nur, wenn `T`
`Model` implementiert.

Wählen Sie die Closure-Form, wann immer Sie können - der Type
Checker validiert den Feldzugriff. Wählen Sie die
string-geschlüsselte Form, wenn Sie Laravels Syntax nachbilden, oder
wenn der Spaltenname ein Laufzeitwert ist.

## Generische Oberfläche - funktioniert mit jedem `Collection<T>`

### Lesen

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9, 2, 6]);

nums.len();                         // 8
nums.is_empty();                    // false
nums.is_not_empty();                // true
nums.first();                       // Some(&3)
nums.last();                        // Some(&6)
nums.first_where(|n| **n > 3);      // Some(&4)
nums.last_where(|n| **n > 3);       // Some(&6)
nums.contains(&4);                  // true - von Deref<Target = [T]>
nums.contains_where(|n| *n > 5);    // true
```

`first_where` / `last_where` nehmen `&&T`, weil das Prädikat über
`Iterator::find` auf `Iter<'_, T>` läuft. Zweimal dereferenzieren
(`**n`).

### Transformieren - konsumiert `self`, gibt neue Collection zurück

```rust
let doubled: Collection<i32>      = nums.clone().map(|n| n * 2);
let evens:   Collection<i32>      = nums.clone().filter(|n| n % 2 == 0);
let odds:    Collection<i32>      = nums.clone().reject(|n| n % 2 == 0);
let unique:  Collection<i32>      = nums.clone().unique();
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(3);
let taken:   Collection<i32>      = nums.clone().take(4);
let skipped: Collection<i32>      = nums.clone().skip(2);
let middle:  Collection<i32>      = nums.clone().slice(2, 4);
let flipped: Collection<i32>      = nums.clone().reverse();
let shuffled: Collection<i32>     = nums.clone().shuffle();
```

`map` ändert den Elementtyp:

```rust
let labels: Collection<String> = nums.clone().map(|n| format!("n={n}"));
```

`each` führt einen Seiteneffekt aus und behält die Collection für
weiteres Verketten (Suprnova weicht hier absichtlich von Laravel ab -
siehe unten):

```rust
let kept = nums.clone()
    .each(|n| tracing::debug!(value = n, "processing"))
    .filter(|n| *n > 2)
    .take(3);
```

### Closure-geschlüsseltes Gruppieren und Sortieren

```rust
use std::collections::HashMap;

// Elemente nach closure-abgeleitetem Schlüssel bündeln.
let by_parity: HashMap<bool, Collection<i32>> =
    nums.clone().group_by_with(|n| n % 2 == 0);

// Elemente nach closure-abgeleitetem Schlüssel indizieren (spätere
// Duplikate überschreiben).
let by_value: HashMap<i32, i32> =
    nums.clone().key_by_with(|n| *n);

// Nach closure-abgeleitetem Komparator sortieren.
let sorted_desc: Collection<i32> =
    nums.clone().sort_with(|a, b| b.cmp(a));

// Nach closure-abgeleitetem Schlüssel deduplizieren.
let unique_mod3: Collection<i32> =
    nums.clone().unique_by(|n| n % 3);

// Jedes Element per Closure in eine neue Collection projizieren.
let strs: Collection<String> =
    nums.pluck_by(|n| n.to_string());
```

Das Suffix `*_with` / `*_by` ist die universelle
"diese-Methode-nimmt-eine-Closure"-Namenskonvention im gesamten
generischen Block. Der modellbewusste Block lässt das Suffix weg und
nimmt stattdessen einen Spaltennamen-String.

### Reduzieren und Aggregieren

```rust
let sum: i32 = nums.clone().reduce(0, |acc, n| acc + n);  // 31
```

Für typisierte numerische Aggregate auf Modell-Collections siehe
`sum` / `avg` / `min` / `max` im modellbewussten Abschnitt - sie
funktionieren auf jedem Feld, das sich zu einem numerischen Typ
deserialisieren lässt.

### Mengenoperationen

```rust
let a = Collection::from_vec(vec![1, 2, 3, 4]);
let b = Collection::from_vec(vec![3, 4, 5, 6]);

let joined = a.clone().concat(b.clone());    // [1,2,3,4,3,4,5,6]
let same   = a.clone().merge(b.clone());     // Alias von concat
let only_a = a.clone().diff(b.clone());      // [1,2]
let common = a.clone().intersect(b.clone()); // [3,4]
```

`concat` / `merge` sind Aliase - Laravel liefert beide Namen. `diff`
/ `intersect` sind O(n*m); bei großen Collections zuerst auf ein
`HashSet` projizieren.

### Zufallsstichproben

```rust
let one: Option<&i32>     = nums.random();        // eines ausleihen
let many: Collection<i32> = nums.clone().random_n(3); // 3 auswählen
```

Beide verwenden den Thread-lokalen RNG (`rand::rng()`). Reichen Sie
für Determinismus in Tests einen gesäten RNG manuell durch.

## Modellbewusste Oberfläche - `Collection<M>`, wobei `M: Model`

Diese Methoden existieren nur, wenn der enthaltene Typ ein
Suprnova-Modell ist. Sie leiten Lesezugriffe pro Zeile durch den
makro-generierten Accessor `Model::field_value(name)`, der
`Option<serde_json::Value>` zurückgibt. Zeilen, deren Feld nicht
existiert oder sich nicht in den Zieltyp deserialisieren lässt,
werden stillschweigend übersprungen - passend zu Laravels Verhalten
bei fehlendem Schlüssel.

### Projektion

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let ids:    Collection<i64>    = users.pluck::<i64>("id");
```

`pluck` leiht aus (`&self`), sodass die ursprüngliche Collection
danach weiterhin verfügbar ist. Der typisierte Parameter
(`::<String>`) ist der Zieltyp, in den der JSON-Wert deserialisiert
wird.

`pluck_keyed` erzeugt eine `HashMap<K, V>` aus zwei Spalten:

```rust
use std::collections::HashMap;

let email_by_id: HashMap<i64, String> =
    users.pluck_keyed::<i64, String>("id", "email");
```

Spätere Zeilen überschreiben frühere für denselben Schlüssel.

### Gruppieren und Indizieren

```rust
use std::collections::HashMap;

let by_role: HashMap<String, Collection<User>> = users.group_by("role");
let by_id:   HashMap<String, User>             = users.key_by("id");
```

Beide Methoden wandeln den Spaltenwert in einen `String`-Schlüssel
um. Eine numerische `id`-Spalte kommt als `"1"` / `"2"` durch -
passend zu Laravels Vertrag `groupBy('team_id')`, bei dem die
Ausgabe unabhängig vom zugrunde liegenden Typ immer
string-geschlüsselt ist.

Für typisierte Schlüssel verwenden Sie die Closure-Form auf dem
generischen Block:

```rust
let by_id: HashMap<i64, User> = users.key_by_with(|u| u.id);
```

### Filtern

Die modellbewussten `where_*`-Methoden nehmen `serde_json::Value`,
weil sie gegen die JSON-kodierte Form der Spalte vergleichen:

```rust
use serde_json::json;

let active: Collection<User>  = users.clone().where_eq("active", json!(true));
let admins: Collection<User>  = users.clone()
    .where_in("role", vec![json!("admin"), json!("owner")]);
let non_guests: Collection<User> = users.clone()
    .where_not_in("role", vec![json!("guest")]);
```

`where_eq` und `where_in` verwerfen Zeilen, deren `field_value`
`None` zurückgibt. `where_not_in` *behält* Zeilen, bei denen das
Feld fehlt - die Negation von "in der Menge" ist "nicht in der
Menge ODER abwesend".

### Sortieren

```rust
let by_name_asc:  Collection<User> = users.clone().sort_by("name");
let by_name_desc: Collection<User> = users.clone().sort_by_desc("name");
```

Der Vergleich ist best-effort über JSON-Wertformen hinweg:
numerisch gegen numerisch und String gegen String sortieren
innerhalb ihrer Art sauber; gemischte heterogene Spalten fallen auf
`Ordering::Equal` zurück. `None` sortiert vor jedem vorhandenen Wert
(spiegelt Postgres' `NULL FIRST` für ASC).

Beide Methoden klonen den zugrunde liegenden `Vec<M>` vor dem
Sortieren, weil der Komparator `m.field_value(field)` ausleiht,
während `sort_by` `&mut [M]` braucht. Bei einer engen Schleife
sortieren Sie stattdessen mit `sort_with` auf dem generischen Block -
das arbeitet in place.

### Aggregate

```rust
let total: f64           = users.sum::<f64>("balance");
let avg:   Option<f64>   = users.avg::<f64>("balance");
let lo:    Option<i64>   = users.min::<i64>("login_count");
let hi:    Option<i64>   = users.max::<i64>("login_count");
```

`sum` gibt `T::default()` zurück, wenn keine Zeile einen Wert
beiträgt (null für numerische Typen). Die anderen drei geben `None`
zurück, damit der Aufrufer nicht durch null teilt oder gegen einen
Phantom-Default vergleicht.

Der typisierte Parameter (`::<f64>`) ist das
JSON-Deserialisierungsziel. Wählen Sie den breitesten numerischen
Typ, den Ihre Spalte sinnvollerweise verwendet - `i64` für
Integer-Spalten, `f64` für Dezimal-/Float-Spalten,
`chrono::DateTime<Utc>` für Zeitstempel usw.

## Eager Loading auf einer Collection

Wenn Sie bereits eine `Collection<M>` haben und Relationen auf jede
Zeile laden wollen, verwenden Sie `load` / `load_missing`:

```rust
let mut users: Collection<User> = User::query().get().await?;
users.load(["posts.comments"]).await?;

for u in &users {
    for p in u.posts_loaded() {
        println!("{}: {} comments", p.title, p.comments_loaded().len());
    }
}
```

Beide Methoden nehmen `&mut self` (sie verändern den
Pro-Zeile-Eager-Cache) und sind `async`. Beide akzeptieren dieselbe
Punkt-Pfad-Syntax, die `Builder::with([...])` akzeptiert - `"posts"`,
`"posts.comments"`, `"posts.comments.author"`.

`load_missing` partitioniert pro Zeile. Zeilen, die die Relation
bereits gecacht haben, werden in Ruhe gelassen; Zeilen ohne bekommen
den Bulk-Load:

```rust
let mut users: Collection<User> = User::query().with(["posts"]).get().await?;
// Manche Zeilen haben posts bereits gecacht. load_missing rührt nur
// den Rest an - und rekursiert in bereits gecachte posts für `comments`.
users.load_missing(["posts.comments"]).await?;
```

Die Rekursion läuft auf jedem Segment eines längeren Punkt-Pfads.
Bei `"a.b.c"` wird jede Zeile auf jeder Ebene partitioniert: `a`
wird nur geladen, wo es fehlt, dann wird für die Zeilen, die `a`
bereits hatten, `b` nur dort geladen, wo es auf diesen `a`s fehlt,
usw.

Beide Methoden respektieren das Routing über
`#[model(connection = "...")]` - sie lösen dieselbe Connection auf,
aus der die Zeile ursprünglich geladen wurde.

## Serialisierung - `to_array` vs serde

Das ist die eine Footgun in der Collection-Oberfläche. Lesen Sie sie
sorgfältig.

`Collection<T>` leitet `Serialize` ab. Das funktioniert also:

```rust
let json: String = serde_json::to_string(&users)?;
```

Aber - serdes pauschale Implementierung `Serialize for Vec<T>` ruft
`T::serialize` direkt auf jedem Element auf. Das **umgeht** die
`Model::to_array()`-Überschreibung, die das Makro
`#[suprnova::model]` generiert. Das heißt, es umgeht Ihre
Modell-Attribute `hidden = ["password"]`, `visible = [...]` und
`appends = [...]`.

Wenn Ihr Modell versteckte Felder hat, serialisieren Sie die
Collection **nicht** über serde. Verwenden Sie `to_array()` oder
`to_json()`:

```rust
let value: serde_json::Value = users.to_array();
let body:  String            = users.to_json();
```

Beide Methoden laufen für jede Zeile durch `Model::to_array()`,
sodass die Pro-Modell-Filter-Pipeline greift - versteckte Felder
bleiben versteckt, Visible-Allowlists werden erzwungen,
accessor-getriebene `appends` erscheinen.

Derselbe Vorbehalt gilt für alles, was im Hintergrund
`serde_json::to_value(&collection)` aufruft: `Inertia::render`, wenn
Sie eine Collection in Props stecken, `JsonApi`/`Resource`, wenn Sie
ihnen rohe Modelle statt Resource-Strukturen übergeben,
Log-Versender, die ihre Payloads per serde kodieren. Das sichere
Muster ist, über einen Resource-Typ zu konvertieren ([JSON:API
resources](eloquent-resources.md)) oder über `to_array()`, bevor der
Wert irgendeinen serde-Codepfad erreicht.

Für Collections von Nicht-Modell-Typen (`Collection<MyDto>`,
`Collection<String>`) ist der serde-Pfad in Ordnung - das Problem
tritt nur auf, wenn `T` eine `#[suprnova::model]`-Struktur mit
deklarierten hidden/visible/appends ist.

## Ausleihen vs Verbrauchen

Die Methoden teilen sich klar in zwei Verträge:

| Nimmt | Methoden |
|---|---|
| `&self` (Ausleihen) | `len`, `is_empty`, `is_not_empty`, `first`, `last`, `first_where`, `last_where`, `contains_where`, `random`, `as_slice`, `pluck_by`, `pluck`, `pluck_keyed`, `group_by`, `key_by`, `sum`, `avg`, `min`, `max`, `to_array`, `to_json` |
| `self` (Verbrauchen) | `map`, `filter`, `reject`, `each`, `reduce`, `chunk`, `take`, `skip`, `slice`, `reverse`, `shuffle`, `random_n`, `unique`, `unique_by`, `sort_with`, `sort_by`, `sort_by_desc`, `where_eq`, `where_in`, `where_not_in`, `concat`, `merge`, `diff`, `intersect`, `group_by_with`, `key_by_with`, `map_to_map` |
| `&mut self` | `load`, `load_missing` |

Wenn Sie die Collection nach einem verbrauchenden Aufruf behalten
wollen, `.clone()` Sie vorher. `Collection<T>: Clone`, wenn `T:
Clone`.

Ein praktisches Muster: erst lesen, dann zuletzt transformieren:

```rust
let users: Collection<User> = User::all().await?;

// Ausleihende Lesevorgänge zuerst - die Collection lebt nach jedem
// davon weiter.
let total       = users.sum::<f64>("balance");
let avg         = users.avg::<f64>("balance");
let count_admin = users.iter().filter(|u| u.role == "admin").count();
let emails      = users.pluck::<String>("email");

// Jetzt verbrauchen.
let admins: Collection<User> = users.where_eq("role", json!("admin"));
```

## Collection vs `Vec`

Der Wrapper ist absichtlich dünn. Die Konvertierungswege gehen in
beide Richtungen und bleiben günstig:

```rust
let v: Vec<User>          = User::query().get().await?.into_vec();
let c: Collection<User>   = Collection::from(v);
let c2: Collection<User>  = Collection::from_vec(c.clone().into_vec());
```

`Deref<Target = [T]>` gibt Ihnen automatisch jede Slice-Methode. Das
schließt ein:

```rust
let users: Collection<User> = User::all().await?;

users.len();             // Slice-Methode
users.iter();            // Slice-Methode
users[0].name.clone();   // Slice-Indizierung
users.contains(&u);      // Slice-Methode
users.binary_search(&u); // Slice-Methode
&users[1..4];            // Slice-Subscripting
```

`IntoIterator` ist zweimal implementiert - für `Collection<T>` (per
Wert) und `&Collection<T>` (per Referenz), sodass beides
funktioniert:

```rust
for user in &users {           // Iteration per &User
    /* ... */
}

for user in users.clone() {    // Iteration per User (verbraucht)
    /* ... */
}
```

`DerefMut` liefert nur `&mut [T]` - ein Slice, kein `Vec`. Das
bedeutet, In-Place-Mutation von Elementfeldern funktioniert:

```rust
let mut users: Collection<User> = User::all().await?;
for u in users.iter_mut() {
    u.last_seen_at = Some(Utc::now());
}
```

Aber owned-`Vec`-Mutation (`push`, `pop`, `clear`, `truncate`) ist
auf der Collection nicht direkt verfügbar - rufen Sie zuerst
`into_vec()` auf:

```rust
let mut v = users.into_vec();
v.push(new_user);
let users: Collection<User> = Collection::from(v);
```

Das ist Absicht. Die Laravel-Oberfläche behandelt eine Collection
als unveränderlichen Snapshot, den Sie mit verketteten Methoden
transformieren; owned Mutation der inneren Sequenz ist der
`Vec`-Vertrag, nicht der `Collection`-Vertrag.

### Wann Sie zu `Vec` zurückgreifen

Greifen Sie zu `into_vec()`, wenn:

- Sie `Vec`-spezifische Methoden brauchen (`push`, `pop`,
  `swap_remove`, `drain`, `with_capacity`).
- Sie die Daten an eine API übergeben, die `Vec<T>` per Wert nimmt,
  und Sie den Wrapper nicht in der Signatur wollen.
- Sie die Zeilen langfristig in Ihrer eigenen Struktur speichern und
  die Laravel-Oberfläche Ihnen nichts bringt.

Für alles andere - Handler-Returns, Transformationen, Inertia-Props
(solange Sie die [Serialisierungsregel](#serialisierung-to-array-vs-serde)
respektieren) - behalten Sie die `Collection<T>`.

## `LazyCollection<M>` - Streaming-Ergebnisse

`Collection<M>` materialisiert jede Zeile im Speicher. Für
Datenmengen, die dafür zu groß sind, bietet der Builder drei
Streaming-Terminals, die stattdessen `LazyCollection<M>`
zurückgeben:

```rust
use suprnova::Model;

let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

| Methode | Strategie |
|---|---|
| `Builder::lazy()` | PK-Cursor-Paginierung mit der Standard-Batch-Größe (1000) |
| `Builder::lazy_by_id(n)` | PK-Cursor-Paginierung mit Batch-Größe `n` |
| `Builder::cursor()` | Laravel-Alias für `lazy()` |

`LazyCollection<M>` ist darunter ein `Pin<Box<dyn Stream<Item =
Result<M, FrameworkError>> + Send>>`, legt aber `.next().await`
direkt frei, sodass Sie `futures::StreamExt` nicht importieren
müssen. Jedes `.next()` löst die Zustellung der nächsten Zeile aus;
der zugrunde liegende Batch-Fetch läuft nur, wenn der
Batch-interne Puffer leerläuft, sodass ein langsamer Konsument keine
Zeilen anhäuft.

Der Wrapper ist `Send` (sodass er `tokio::spawn` überquert), aber
nicht `Sync` - er ist konstruktionsbedingt ein
Einzelkonsument-Stream.

Siehe [Eloquent - Chunking und Lazy-Iteration](eloquent.md#chunking-and-lazy-iteration)
für die vollständige Anleitung, welches Streaming-Muster zu wählen
ist.

## Warum Suprnova abweicht

Laravels `Illuminate\Support\Collection` ist veränderlich:
`$c->filter(...)` verändert das innere Array desselben Objekts und
gibt `$this` für Verkettung zurück. PHP hat kein Ownership, also ist
dieser Vertrag unsichtbar.

Rust hat Ownership, und so zu tun, als hätte es das nicht, würde die
Collection-Oberfläche unehrlich machen. Suprnova wählt stattdessen
die wertsemantische Form: Jede Transformation verbraucht `self` und
gibt eine neue `Collection` zurück. Sie sehen die Kosten in Ihrem
eigenen Code - wenn Sie das Original behalten wollen, `.clone()`n
Sie. Wenn nicht, tun Sie es nicht.

Diese Entscheidung zieht sich durch den Rest der Oberfläche:

- **`each` gibt `Self` zurück** statt `&self`, damit ein
  Seiteneffekt-Aufruf (Logging, Metriken) eine Kette nicht bricht.
  PHPs `each` läuft um des Effekts willen und gibt die Collection
  zurück; `$c->each(...)->filter(...)` ließe sich ohne erneutes
  Holen nicht sauber machen. In Rust reichen wir `self` durch und
  halten die Kette flüssig.

- **Closure-geschlüsselte Alternativen zu jeder
  string-geschlüsselten Methode.** `pluck_by`, `group_by_with`,
  `key_by_with`, `sort_with`, `unique_by`, `map_to_map`,
  `contains_where`. Die Closures lassen Sie Felder lesen, die der
  Type Checker validiert, statt Strings, die der Compiler nicht
  sehen kann. Die string-geschlüsselten Formen existieren für
  Laravel-Syntax-Parität und für zur Laufzeit entschiedene
  Spaltennamen.

- **`sum` / `avg` / `min` / `max` nehmen typisierte
  `::<T>`-Parameter.** Laravels PHP-Version castet spontan; in
  Rust ist das Deserialisierungsziel Teil des Aufrufs. Zeilen, deren
  Wert nicht in `T` round-trippt, werden stillschweigend
  übersprungen (passend zu Laravels Verhalten bei fehlendem
  Schlüssel), aber Sie wählen den Typ absichtlich.

- **`Deref<Target = [T]>`, nicht `Deref<Target = Vec<T>>`.** Eine
  `Collection` ist konzeptuell ein "Snapshot von Zeilen", kein
  veränderlicher Puffer. Slice-Methoden kommen über `Deref`; wenn
  Sie `push`/`pop` wollen, gibt Ihnen `into_vec()` den rohen `Vec`
  und entfernt jeden Anschein des Gegenteils.

- **Serialisierung weicht im Dienst der Korrektheit ab.**
  `to_array` und `to_json` laufen durch `Model::to_array()`, sodass
  Pro-Modell-hidden/visible/appends greifen; serdes pauschaler
  Bypass `Serialize for Vec` ist als die
  [Footgun](#serialisierung-to-array-vs-serde) dokumentiert, die er
  ist. Laravels `toArray()` macht dasselbe Routing; wir müssen die
  Lücke nur explizit benennen, weil Rust-Nutzer reflexhaft zu
  `serde_json::to_string` greifen werden.

Der Kompromiss ist genau der, den Suprnova überall macht: Laravels
Oberflächenform, Rusts Wertsemantik.

## Nächste Schritte

- [Eloquent API](eloquent.md) - das übergeordnete Kapitel, mit dem
  Query Builder, Relationen, Scopes und dem vollständigen
  Modell-Lifecycle.
- [JSON:API Resources](eloquent-resources.md) - Resource-Strukturen
  serialisieren Collections über `IntoJsonResource` mit Sparse
  Fieldsets und `?include=`-Ketten; die richtige Form für jede
  Collection, die Ihre API verlässt.
- [Frontend - Inertia Responses](frontend-inertia-responses.md) -
  die Regeln, um Collections an Inertia-Props zu übergeben, ohne
  die Serialisierungs-Footgun auszulösen.
- [Validierung](validation.md) - Request-Payloads erzeugen oft
  Vektoren, die Sie für nachgelagerte Verarbeitung in `Collection`
  einpacken.
- [Testen](testing.md) - Muster, um Collection-Inhalte (Länge,
  enthaltene Elemente, Reihenfolge) innerhalb von Handler- und
  Modell-Tests zu prüfen.

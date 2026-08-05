# Vector

Suprnova bringt eine Laravel-förmige `Vector`-Facade mit, die von
einem von vier Treibern getragen wird - In-Process-Memory, Qdrant,
Pinecone oder MariaDBs natives `VECTOR(N)` -, beim Boot explizit über
`Vector::register` ausgewählt. Die Facade ist eine dünne Schicht über
einem `VectorDriver`-Trait, sodass sich benutzerdefinierte Backends
genauso einbinden wie die eingebauten.

## Schnellstart

```rust
use std::sync::Arc;
use suprnova::{MemoryVectorDriver, Vector, VectorItem};

// Bootstrap (normalerweise einmal beim App-Start)
Vector::register("documents", Arc::new(MemoryVectorDriver::new()));

// Verwenden
let store = Vector::store("documents")?;
store
    .upsert(vec![
        VectorItem::new("doc-1", embedding_for("Hello"), serde_json::json!({ "title": "Hello" })),
        VectorItem::new("doc-2", embedding_for("World"), serde_json::json!({ "title": "World" })),
    ])
    .await?;

let hits = store.similar(query_embedding, 10).await?;
for hit in hits {
    println!("{}: {} (score {:.3})", hit.id, hit.metadata["title"], hit.score);
}
```

## Der Vertrag

```rust
#[async_trait]
pub trait VectorDriver: Send + Sync + 'static {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError>;
    async fn similar(&self, store: &str, query: Vec<f32>, k: usize) -> Result<Vec<VectorMatch>, FrameworkError>;
    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError>;
    async fn count(&self, store: &str) -> Result<usize, FrameworkError>;
}
```

`VectorItem` trägt eine beliebige `String`-ID, ein `embedding:
Vec<f32>` und freie `metadata: serde_json::Value` (muss ein
JSON-Objekt oder `null` sein). `VectorMatch` liefert die
ursprüngliche ID, den Ähnlichkeits-Score des Backends und dieselbe
Metadaten-Form zurück.

Das Trait ist bewusst klein gehalten. Wenn Sie Filterausdrücke bei
der Suche, Sparse Vektoren, Scroll/List, Snapshots oder
Quantisierungs-Regler brauchen, steigen Sie über den öffentlichen
`client()`-Direktzugriff des Treibers auf dessen zugrundeliegendes
SDK ab.

### Warum Suprnova abweicht

Laravel liefert Vektoren nur über Postgres' `pgvector`. Das ist die
PHP-förmige Antwort: ein Storage-Backend wählen, es hinter einem
einzigen Treiber verstecken und die Sache für erledigt erklären.
Suprnova behandelt die Wahl als Konfigurationsfrage. Dasselbe Trait
deckt eine In-Process-`HashMap` für Tests ab, eine dedizierte
Vektor-DB (Qdrant, Pinecone), wenn die Embedding-Anzahl die
Betriebskosten rechtfertigt, sowie ein relationales Backend (MariaDB
11.7+), wenn Sie Vektoren lieber neben den Zeilen halten, die sie
erzeugt haben. Weaviate, Milvus, LanceDB, pgvector und LibSQL stehen
Schlange, sobald echte Nachfrage von Anwenderseite entsteht - keines
davon wird von der Trait-Form blockiert.

Wenn der Rest Ihrer App auf einer Engine läuft, hält MariaDB 11.7+
Vektoren neben relationalen Tabellen, JSON-Dokumenten und
systemversionierten Temporaldaten vor - weniger bewegliche Teile,
als Postgres + Redis + Qdrant getrennt zu betreiben. Siehe
[Bereitstellung](deployment.md) für die Empfehlung im Kontext.

## Treiber

### Memory - `MemoryVectorDriver`

In-Process-Treiber, getragen von `HashMap`. Cosine-Ähnlichkeit,
Punkte mit Dimensions-Mismatch werden bei der Query still
übersprungen (sodass gemischt-dimensionale Testdaten nicht
explodieren), Zero-Vector-Queries schlagen mit einer klaren
Fehlermeldung fehl.

```rust
Vector::register("docs", Arc::new(MemoryVectorDriver::new()));
```

Für Tests und Dev verwenden. Jede `MemoryVectorDriver::new()`-Instanz
ist hermetisch - kein geteilter Zustand zwischen zwei `new()`s.

### Qdrant - `QdrantVectorDriver`

Spricht über gRPC (Standardport 6334) mit Qdrant, über das offizielle
`qdrant-client`-SDK.

```rust
use suprnova::{QdrantDistance, QdrantVectorDriver};

let driver = QdrantVectorDriver::from_url("http://localhost:6334")?
    .with_distance(QdrantDistance::Cosine)  // Standard
    .with_auto_create(true);                // Standard

Vector::register("docs", Arc::new(driver));
```

Für Qdrant Cloud:

```rust
let driver = QdrantVectorDriver::from_url_with_api_key(
    "https://xxxxxxxx.eu-central.aws.cloud.qdrant.io:6334",
    std::env::var("QDRANT_API_KEY")?,
)?;
```

**ID-Mapping.** Qdrant verlangt, dass Point-IDs entweder `u64` oder
eine gültige UUID sind. Das Framework überbrückt beliebige Strings
mit drei Regeln:

1. Lässt sich der String als `u64` parsen, wird die
   `Num(u64)`-Variante verwendet.
2. Ist der String eine gültige UUID, wird die `Uuid(String)`-Variante
   wörtlich verwendet.
3. Andernfalls wird eine deterministische v5-UUID aus einem stabilen
   Namespace abgeleitet.

Der ursprüngliche String des Aufrufers wird im Payload des Points
unter dem reservierten Schlüssel `__suprnova_id` (exportiert als
`SUPRNOVA_ID_PAYLOAD_KEY`) abgelegt und bei der Auslesung aus
`VectorMatch.metadata` entfernt. Power-User, die Qdrant direkt über
`driver.client()` abfragen, können nach `__suprnova_id` filtern, um
Framework-Schreibvorgänge mit direkten Aufrufen zu verbinden.

**Auto-Create.** Beim ersten `upsert` für eine noch unbekannte
Collection legt der Treiber sie an, mit der aus dem ersten Item
abgeleiteten Dimension und der konfigurierten Distanzmetrik
(standardmäßig Cosine). Race-sicher - konkurrierende Upserter auf
derselben frischen Collection schlagen nicht fehl; wer zuerst
anlegt, gewinnt, der andere fährt fort. Deaktivieren Sie das über
`.with_auto_create(false)`, um explizites Anlegen zu erzwingen.

**Cache-Invalidierung.** Wird eine Collection extern gelöscht (oder
startet Qdrant neu, bevor die Persistenz geflusht wurde), erkennt
der Treiber den Fehler „not found“ beim Upsert, verwirft den
Cache-Eintrag, führt `ensure_collection` erneut aus und versucht es
einmal erneut.

**Direktzugriff.** `driver.client()` liefert das zugrundeliegende
`qdrant_client::Qdrant` zurück - nutzen Sie es für Filterausdrücke
bei der Suche, Scroll, Snapshots oder andere APIs, die nicht über
das Trait freigelegt sind. `QdrantVectorDriver::resolve_point_id`,
`build_point` und `decode_match` lassen Sie direkte und
Trait-geroutete Aufrufe mischen, ohne die ID-Übersetzung zu
verlieren.

**Lokales Setup.** Qdrant über Docker starten:

```bash
docker run -p 6334:6334 -p 6333:6333 qdrant/qdrant
```

Integrationstests laufen über:

```bash
QDRANT_URL=http://localhost:6334 cargo test -p suprnova --test vector_qdrant -- --ignored
```

### Pinecone - `PineconeVectorDriver`

> **Hinter einem Feature-Flag - standardmäßig aus.** Aktivieren Sie
> es mit `cargo build --features vector-pinecone` (oder fügen Sie
> `features = ["vector-pinecone"]` unter der `suprnova`-Abhängigkeit
> in Ihrer `Cargo.toml` hinzu). Das Feature kostet keine
> zusätzlichen Abhängigkeiten - es schaltet nur die Kompilierung des
> Treibers frei, nichts weiter -, daher ist es schlicht deshalb aus,
> weil die meisten Apps kein Pinecone nutzen und es nicht
> mitkompilieren sollten.

Spricht über dessen REST-API mit Pinecone, unter Verwendung des
HTTP-Clients, den das Framework bereits mitbringt.

> **Warum nicht das offizielle SDK?** Der Treiber umschloss früher
> `pinecone-sdk`, das gRPC spricht. Das neueste Release dieser Crate
> (0.1.2, veröffentlicht am 2024-09-06) pinnt `tonic 0.11 → rustls
> 0.22 → rustls-webpki 0.102`, und `rustls-webpki 0.102` trägt vier
> RustSec-Advisories, die alle Upstream in `>= 0.103.13` behoben
> sind. Eine verwaiste Crate hielt den ganzen Baum zurück, ohne dass
> ein „Warten auf Upstream“ je ein Ende gehabt hätte. Pinecone legt
> jede Operation, die dieser Treiber braucht, über HTTPS offen,
> sodass der REST-Weg vier Advisories und zwei Abhängigkeiten auf
> einen Schlag entfernt hat.

```rust
use suprnova::PineconeVectorDriver;

// API-Schlüssel direkt
let driver = PineconeVectorDriver::from_api_key(std::env::var("PINECONE_API_KEY")?)?;

// Oder über die Umgebung: PINECONE_API_KEY, plus optional PINECONE_CONTROLLER_HOST
// und PINECONE_API_VERSION
let driver = PineconeVectorDriver::from_env()?;

// An einen Non-Default-Namespace binden
let driver = driver.with_namespace("public");

Vector::register("docs", Arc::new(driver));
```

Der über `Vector::store(name)` übergebene Store-Name bildet auf
einen Pinecone-Index-Namen ab. Der Treiber löst den Host dieses
Index lazy bei erster Verwendung über das `GET /indexes/{name}` der
Control Plane auf und cacht ihn dann. Überspringen Sie den
Roundtrip, indem Sie den bereits bekannten Host pinnen:

```rust
let driver = PineconeVectorDriver::from_env()?
    .with_index_host("docs", "docs-abc123.svc.aped-1234.pinecone.io");
```

Ein von der Control Plane gelernter Host wird immer über `https`
kontaktiert, unabhängig davon, was die Antwort sagt. Ein über
`with_index_host` gepinnter Host behält das Schema, das Sie
angegeben haben, sodass auch ein lokaler Emulator auf `http://`
funktioniert.

**API-Version.** Pinecone versioniert seine REST-API nach Datum und
will diese Version in einem Header gepinnt sehen. Der Treiber pinnt
`2025-04` - die Version, gegen die seine Anfrage- und Antwortformen
geschrieben und getestet wurden - und legt `with_api_version` (oder
`PINECONE_API_VERSION`) offen, um bewusst zu wechseln. Er treibt
nicht mit: Die Namespace-Schlüssel-Konvention in
`describe_index_stats` ist eines der Dinge, die sich zwischen
Versionen geändert haben, und `count()` liest genau diese Map.

**Kein Auto-Create.** Das Anlegen eines Pinecone-Index verlangt die
Wahl von Cloud (AWS/GCP/Azure), Region, Vektor-Dimension,
Distanzmetrik und Löschschutz - zu viele Kompromisse für einen guten
Standard. Legen Sie Indizes über die Pinecone-Konsole, die
Pinecone-CLI oder einen `control_plane_post`-Aufruf an, bevor Sie
registrieren, und zeigen Sie das Framework dann auf den bestehenden
Namen.

Das ist die zentrale Asymmetrie zum Qdrant-Treiber, der Collections
beim ersten Upsert automatisch anlegt.

**IDs und Metadaten.** Pinecone akzeptiert beliebige `String`-IDs
nativ, sodass `VectorItem::id` unverändert durchgereicht wird.
Metadaten werden end-to-end als JSON getragen -
`PineconeVectorDriver::metadata_from_json` / `metadata_to_json`
erzwingen nur die eigene Regel des Frameworks, dass Metadaten ein
Objekt oder `null` sein müssen. Pinecone selbst beschränkt
Metadaten-*Werte* auf Strings, Zahlen, Booleans und Listen von
Strings und lehnt verschachtelte Objekte serverseitig ab; der
Treiber implementiert diese Prüfung nicht erneut, weil Pinecones
Regeln versioniert sind und eine lokale Kopie driften würde.

**Batch-Grenzen.** Pinecone dokumentiert ein Maximum von 1000
Vektoren pro Upsert und 1000 IDs pro Delete. Der Treiber sendet, was
Sie ihm geben, in einer einzigen Anfrage, statt still zu chunken -
ein Write mit Teilerfolg lässt sich schwerer nachvollziehen als ein
abgelehnter. Batchen Sie selbst, wenn Sie diese Grenzen
überschreiten.

**Namespaces.** Eine Treiber-Instanz bindet an einen Namespace. Um
mehrere Namespaces desselben Index zu verwenden, registrieren Sie
einen Treiber pro Namespace unter verschiedenen Store-Namen:

```rust
Vector::register("docs-public", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("public")
));
Vector::register("docs-private", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("private")
));
```

**Durchsatz.** Nichts serialisiert. Der Treiber cacht einen
Host-String pro Index, kein Connection-Handle, und Requests teilen
sich `reqwest`s Connection-Pool - sodass gleichzeitige Aufrufe
desselben Index gleichzeitig ablaufen. (Der gRPC-Treiber, den dies
ersetzt, hielt einen `Index` pro Name hinter einem `tokio::Mutex`,
weil `pinecone-sdk` `Index` nur hinter `&mut self` offenlegte.)

**Direktzugriff.** `control_plane_get`, `control_plane_post` und
`data_plane_post` erreichen jeden Endpunkt, den Pinecone ausliefert,
mit Ihren eigenen Request- und Response-Typen, über den
authentifizierten und host-aufgelösten Transport des Treibers -
Filterausdrücke, Sparse Vektoren, Fetch-by-ID, `/vectors/list`,
Index-Verwaltung:

```rust
#[derive(serde::Deserialize)]
struct FetchResponse { vectors: Vec<suprnova::vector::PineconeVector> }

let hits: FetchResponse = driver.data_plane_post(
    "docs",
    "/vectors/fetch_by_metadata",
    &serde_json::json!({ "filter": { "genre": { "$eq": "comedy" } }, "limit": 2 }),
).await?;
```

**Tests.** Wire-Vertrag-Tests laufen standardmäßig unter dem
Feature: Sie steuern den Treiber gegen einen lokalen Fake und
assertieren die exakte Methode, den Pfad, die Header und den
JSON-Body, den er sendet. Diese fixieren den Treiber auf Pinecones
*dokumentierten* Vertrag. Um zu bestätigen, dass die Dokumentation
zum Live-Service passt, braucht es die `#[ignore]`-Integrationstests,
die beide Umgebungsvariablen benötigen:

```bash
PINECONE_API_KEY=... PINECONE_TEST_INDEX=my-test-index \
    cargo test -p suprnova --features vector-pinecone \
    --test vector_pinecone -- --ignored
```

### MariaDB - `MariaDbVectorDriver`

Spricht direkt über `sqlx::MySqlPool` mit MariaDB 11.7+, unter
Verwendung von MariaDBs nativem Spaltentyp `VECTOR(N)` und
HNSW-Indizierung. Beim ersten Aufruf einer Treiber-Methode führt er
`SELECT VERSION()` aus und lehnt alles unter 11.7 ab - ältere Server
haben die Vektor-Funktionen nicht.

```rust
use std::sync::Arc;
use suprnova::{MariaDbDistance, MariaDbVectorDriver, Vector};

let driver = MariaDbVectorDriver::from_url(
    "mysql://user:pass@localhost:3306/myapp",
)?
.with_distance(MariaDbDistance::Cosine);  // Standard

Vector::register("documents", Arc::new(driver));
```

`from_url` ist lazy - es validiert die URL-Syntax, öffnet aber KEINE
Verbindung vor der ersten Verwendung, sodass der Aufruf beim
App-Bootstrap sicher ist, selbst bevor die Datenbank erreichbar ist.
Hüllen Sie einen bestehenden Pool mit
`MariaDbVectorDriver::from_pool(pool)` ein, wenn Sie
benutzerdefinierte Pool-Optionen brauchen.

**Das Schema gehört Ihnen.** Der Treiber legt keine Tabellen
automatisch an - Schema ist eine Migrations-Angelegenheit. Der
empfohlene Weg ist `driver.ensure_table_sql_for(name, dim)`, das die
konfigurierte Distanz des Treibers erbt, sodass die
`DISTANCE=`-Klausel der Migration und die Query-Funktion, die
`similar` verwendet, garantiert übereinstimmen:

```rust
let driver = MariaDbVectorDriver::from_url(url)?
    .with_distance(MariaDbDistance::Cosine);

let sql = driver.ensure_table_sql_for("documents", 1536)?;
// Ergebnis:
// CREATE TABLE IF NOT EXISTS `documents` (
//   id VARCHAR(255) NOT NULL PRIMARY KEY,
//   embedding VECTOR(1536) NOT NULL,
//   metadata JSON NULL,
//   VECTOR INDEX (embedding) DISTANCE=cosine
// ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
```

Für Migrations-Generatoren, die keinen Treiber im Scope haben
(CLI-Tools, Build-Skripte), verwenden Sie das statische
`MariaDbVectorDriver::ensure_table_sql(name, dim, distance)` und
übergeben Sie dieselbe `MariaDbDistance`, die Sie später auf dem
Treiber konfigurieren.

**Distanz muss auf beiden Seiten übereinstimmen.** MariaDB fällt
still auf einen vollständigen Table-Scan zurück, wenn die zur
Query-Zeit verwendete Funktion nicht zur `DISTANCE=`-Klausel des
Index passt. Der Treiber sichert sich dagegen auf zwei Ebenen ab:

1. **`ensure_table_sql_for(name, dim)`** liest `self.distance`
   sowohl für das ausgegebene Migrations-SQL als auch für die
   Laufzeit-Funktion in `similar` - beide können
   konstruktionsbedingt nicht auseinanderlaufen.
2. **Eine Laufzeitprüfung beim ersten `similar`-Aufruf** führt pro
   Store ein `SHOW CREATE TABLE` aus, parst die tatsächliche
   `DISTANCE=`-Klausel aus dem Live-Schema und meldet klar einen
   Fehler, wenn sie von `with_distance(...)` abweicht. Das Ergebnis
   wird gecacht, sodass nachfolgende Aufrufe kostenlos sind. Das
   fängt handgeschriebene Migrationen oder `from_pool`-Setups ab,
   die `ensure_table_sql_for` umgehen.

**Sicherheit des Store-Namens.** Store-Namen werden in das
ausgegebene SQL interpoliert (MySQL parametrisiert Identifier
nicht). Namen werden gegen `[A-Za-z_][A-Za-z0-9_]*` mit einer Länge
≤ 64 validiert; der validierte Name wird dann in jedem Statement in
Backticks gesetzt. Ungültige Namen liefern an der
`register`/`upsert`/`similar`/`delete`/`count`-Grenze einen Fehler
mit `FrameworkError::param`.

**IDs und Metadaten.** `VARCHAR(255)` akzeptiert beliebige
`String`-IDs - keine UUID-Ableitung, keine reservierten
Payload-Schlüssel. Metadaten überstehen einen Round-Trip durch
MariaDBs Spaltentyp `JSON`; `null`-Metadaten werden als SQL-`NULL`
gespeichert. Nicht-Objekt-Metadaten (Arrays, Primitives) werden mit
`FrameworkError::param` abgelehnt, zur Parität mit Qdrant und
Pinecone.

**Score-Normalisierung.** MariaDB liefert rohe *Distanz* zurück
(niedriger = näher). Der Trait-Vertrag verlangt *Score* (höher =
ähnlicher) - der Treiber rechnet pro Metrik um:

| Metrik    | MariaDB liefert       | Exponierter `score`          |
| --------- | --------------------- | ----------------------------- |
| Cosine    | `[0, 2]` (`1 - cos`)  | `1.0 - d / 2.0` → `[0, 1]`   |
| Euclidean | `[0, ∞)` L2-Norm      | `1.0 / (1.0 + d)` → `(0, 1]` |

In beiden Fällen bleibt das Ranking erhalten (bestes Ergebnis
zuerst), aber die absoluten Score-Werte sind zwischen Treibern NICHT
vergleichbar - nur die Reihenfolge ist es. Jedes Backend landet auf
der Konvention `höher = besser`, aber die Bereiche unterscheiden
sich: Memorys Cosine liefert `[-1, 1]`, MariaDBs normalisiertes
Cosine liefert `[0, 1]`, Qdrant gibt seine native Cosine-Ähnlichkeit
in `[-1, 1]` aus, und Pinecone liefert die rohe Ähnlichkeit für die
Metrik, mit der der Index angelegt wurde. Verwenden Sie `score`, um
innerhalb des Ergebnis-Sets eines einzelnen Treibers zu sortieren;
vergleichen Sie numerische Scores nicht über Treiber hinweg, ohne
selbst neu zu normalisieren.

**Direktzugriff.** `driver.pool()` liefert den zugrundeliegenden
`sqlx::MySqlPool` für rohe Queries, die das Trait nicht abdeckt.
`MariaDbVectorDriver::embedding_to_vec_text`, `score_from_distance`
und `ensure_table_sql` sind reine Funktionen, die Sie unabhängig
aufrufen können, wenn Sie direktes SQL mit Trait-gerouteten Aufrufen
mischen.

**Bulk-Upsert-Verhalten.** `upsert` gibt pro 500-Zeilen-Chunk ein
Multi-Row-`INSERT ... VALUES (...), (...), ...`-Statement aus,
alles in eine einzige Transaktion gehüllt. Netzwerk-Round-Trips
fallen beim Laden eines frischen Korpus um ~500x gegenüber
Zeile-für-Zeile-Inserts; der Aufruf bleibt über den gesamten Batch
hinweg atomar. Die Batch-Größe ist intern - rufen Sie `upsert`
einmal mit all Ihren Items auf, und der Treiber übernimmt das
Chunking.

**HNSW-Indizes bauen beim Commit neu.** MariaDB aktualisiert den
HNSW-Graphen, während Zeilen hereinkommen, aber die Index-Arbeit
konzentriert sich auf den Commit. Ein `upsert` über 1 Mio. Zeilen
hält die Transaktion für die volle Dauer des Index-Baus offen, was
Minuten dauern kann. Zerlegen Sie bei sehr großen initialen
Ladevorgängen den Korpus in Batches von 10k–100k Zeilen und rufen
Sie `upsert` wiederholt auf, sodass jeder Batch committet und das
Lock zwischen den Runden freigibt. (Kleinere `upsert`-Aufrufe sind
pro Zeile nicht langsamer - sie verteilen die Index-Arbeit nur auf
mehr Commit-Punkte.)

**Dimension wird bei der Tabellenerstellung fixiert.** `VECTOR(N)`
legt die Dimension fest; ein Wechsel des Embedding-Modells von einem
768-dimensionalen zu einem 1536-dimensionalen Modell bedeutet eine
vollständige Tabellen-Migration (neue Tabelle, Re-Embedding,
Umschalten). Planen Sie Modell-Upgrades genauso, wie Sie eine
Schema-Migration planen würden - einen Weg über „ALTER COLUMN
VECTOR(768) → VECTOR(1536)“ gibt es nicht.

**Pool-Größe.** `from_url` verwendet sqlx' Standard-`MySqlPoolOptions` -
zum Zeitpunkt des Schreibens `max_connections = 10`. Für Workloads
mit hohem QPS (hunderte `similar`-Aufrufe pro Sekunde) bauen Sie den
Pool selbst mit
`MySqlPoolOptions::new().max_connections(N).connect_lazy(url)` und
übergeben ihn an `from_pool`. Der Treiber erzwingt keine eigene
Verbindungsobergrenze.

**Lokales Setup.** MariaDB 11.7+ über Docker starten:

```bash
docker run -p 3306:3306 \
    -e MARIADB_ROOT_PASSWORD=secret \
    -e MARIADB_DATABASE=vectors \
    mariadb:11.7
```

Integrationstests laufen über:

```bash
MARIADB_URL='mysql://root:secret@localhost:3306/vectors' \
    cargo test -p suprnova --test vector_mariadb -- --ignored
```

## Treibervergleich

| Aspekt | Memory | Qdrant | Pinecone | MariaDB |
| --- | --- | --- | --- | --- |
| Backing Store | `HashMap` | Qdrant gRPC | Pinecone REST | MariaDB SQL |
| Persistenz | Keine | Ja | Ja | Ja |
| Auto-Create | n/a | Ja (konfigurierbar) | Nein (Nutzer legt Index an) | Nein (Migration ist Ihre Aufgabe) |
| String-IDs | Nativ | Auf UUID-5 gehasht | Nativ | Nativ |
| Metadaten-Schlüssel reserviert | Keiner | `__suprnova_id` | Keiner | Keiner |
| Durchsatz | Pro Prozess | Gleichzeitig | Gleichzeitig (Pool-begrenzt) | Gleichzeitig (Pool-begrenzt) |
| Distanzmetrik | Cosine | Konfigurierbar | Bei Index-Erstellung gesetzt | Cosine / Euclidean |
| Versionsanforderung | - | Beliebig | Beliebig | **11.7+** |

## Hinweise zum Betrieb

**Store-Namens-Konventionen.** Der an `Vector::register` und
`Vector::store` übergebene Store-Name ist ein Label - er kann ein
beliebiger String sein. Für Qdrant verwendet das Framework ihn als
Collection-Namen, für Pinecone als Index-Namen. Passen Sie das
Label an das bestehende Namensschema des Backends an.

**Erneutes Registrieren** eines Namens mit einer neuen
Treiber-Instanz ist per Design eine Last-Write-Wins-Operation -
nützlich, um Treiber in Test-Harnesses auszutauschen, ohne den
Prozess neu zu starten.

**Test-Isolation.** Sowohl Memory- als auch Registry-gestützte
Treiber-Tests verwenden mit Zeitstempel getaggte, eindeutige
Store-Namen, um Kollisionen bei parallelen Testläufen zu vermeiden.

**Fehler-Semantik.** `Vector::store(name)` liefert
`FrameworkError::not_found` für nicht registrierte Namen zurück.
Fehler auf Treiber-Ebene (Netzwerk, Auth, Dimensions-Mismatch)
kommen als `FrameworkError::internal` oder `FrameworkError::param`
mit dem Ursachen-String in der Display-Nachricht zurück.

## Erweiterung

Um ein fünftes Backend hinzuzufügen (Weaviate, Milvus, LanceDB,
pgvector, LibSQL, ...):

1. Eine neue `framework/src/vector/<backend>.rs` hinzufügen, die
   `VectorDriver` implementiert.
2. Den Treiber-Typ aus `framework/src/vector/mod.rs` und der
   Crate-Wurzel re-exportieren.
3. Den Pinecone-Test-Split spiegeln: Pure-Function-Tests und
   Wire-Vertrag-Tests (gegen einen lokalen `wiremock`-Fake) laufen
   immer; Integrationstests sind hinter Umgebungsvariablen für
   Credentials `#[ignore]`-gated. Die mittlere Schicht ist die, die
   sich lohnt - ein Backend, das niemand von der CI aus erreichen
   kann, hat trotzdem ein Wire-Format, das ein Tippfehler brechen
   kann.

Das Trait ist bewusst klein gehalten, damit die Hürde für einen
neuen Treiber niedrig bleibt. Wenn ein Backend Oberfläche braucht,
die nicht passt (Filterausdrücke, Sparse Vektoren, Hybrid-Suche),
legen Sie sie über einen Direktzugriff auf dem Treiber offen -
blähen Sie das Trait nicht auf.

## Nächste Schritte

- [Bereitstellung](deployment.md) - die Empfehlung
  MariaDB-als-Standard-für-Produktion im Kontext
- [Datenbank](database.md) - Multi-Treiber-SeaORM-Setup,
  einschließlich MariaDB als relationales Backend neben Vektoren
- [Umgebungsvariablen](env-vars.md) - `QDRANT_URL`,
  `PINECONE_API_KEY`, `MARIADB_URL` und weitere Treiber-Env-Verträge
- [Cache](cache.md) - das Pendant mit derselben Treiber-Trait-Form
- [Laravel Parity Map](parity.md) - wo Vektorsuche relativ zu Scout
  steht

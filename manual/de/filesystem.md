# Dateisystem & Speicher

Suprnovas Storage-Facade gibt Ihnen eine einzige, namensbasierte
Disk-API über lokale Dateisysteme, In-Memory-Backends und die
wichtigsten Objektspeicher (S3, Azure Blob, Google Cloud Storage).
Unter der Haube baut sie auf [`opendal`](https://docs.rs/opendal)
auf - aber die Oberfläche für Verbraucher ist so geformt, dass sie zu
Laravels `Storage::disk(...)`-Aufrufen passt, sodass sich
eingespielte PHP-Routine direkt übertragen lässt.

```rust,no_run
use suprnova::{DiskExt, Storage};

# async fn doc() -> Result<(), suprnova::FrameworkError> {
Storage::register_fs("local", "./storage")?;
let disk = Storage::disk("local")?;

disk.put("notes/hello.txt", b"hello world".to_vec()).await?;
let bytes = disk.get("notes/hello.txt").await?;
assert_eq!(bytes, b"hello world");
# Ok(())
# }
```

## Disks registrieren

Jede Disk wird beim Boot einmal über `Storage::register_*` registriert und
per Name über `Storage::disk(name)` nachgeschlagen. Es gibt kein
„Standard-Backend“, auf das die anderen zurückfallen - jeder Treiber ist
gleichrangig.

| Konstruktor                          | Backend                       | Feature             |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | Lokales Dateisystem           | `filesystem`        |
| `Storage::register_memory(name)`     | Speicher im Prozess (Tests)   | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3 oder S3-kompatibel  | `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage          | `filesystem-gcs`    |

`filesystem` ist standardmäßig an, die Azure- und GCS-Features nicht.
Schalten Sie eines in Ihrer `Cargo.toml` ein:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.1", features = ["filesystem-gcs"] }
```

Ohne das Feature existieren `register_azblob` / `register_gcs` und ihre
Config-Strukturen nicht - Sie bekommen einen Compile-Fehler, der das
fehlende Element benennt, keinen Laufzeitfehler.

Jeder Konstruktor hat eine `_with`-Variante, die Ihnen den
`suprnova::opendal::Operator` reicht, kurz bevor er in der Registry
landet, sodass Sie Retry-/Timeout-/Logging-Layer darum installieren
können:

```rust,ignore
use std::time::Duration;
use suprnova::opendal::layers::{LoggingLayer, RetryLayer, TimeoutLayer};
use suprnova::Storage;

Storage::register_fs_with("local", "./storage", |op| {
    op.layer(RetryLayer::new().with_max_times(3))
      .layer(TimeoutLayer::new().with_timeout(Duration::from_secs(30)))
      .layer(LoggingLayer::default())
})?;
```

Die Cloud-Konstruktoren (`register_s3`, `register_azblob`,
`register_gcs`) legen standardmäßig einen `RetryLayer` (3 Versuche) an,
da vorübergehende Drosselung und 5xx-Fehler bei Objektspeichern Routine
sind. Verwenden Sie die `_with`-Varianten, wenn Sie volle Kontrolle
brauchen.

Der vollständige Satz an opendal-Layern, die Suprnova verdrahtet, ist
`RetryLayer`, `TimeoutLayer`, `LoggingLayer`, `TracingLayer` (brückt über
`tracing-opentelemetry` zu OTel, wenn das `otel`-Feature des Frameworks
an ist) und `PrometheusClientLayer` (exportiert Histogramme und Zähler in
eine `prometheus_client::registry::Registry`, die Ihnen gehört). Die
Layer-Reihenfolge zählt - der äußerste Layer umschließt alles darin -,
und der idiomatische Stack ist
`RetryLayer → TimeoutLayer → LoggingLayer`, sodass ein abgelaufener
Versuch trotzdem protokolliert wird und eine Wiederholung
Transportfehler abdeckt.

Ein erneutes Registrieren desselben Namens ersetzt den vorherigen
Operator und gibt ein `warn!`-Log aus - Disks sollen einmal beim Boot
registriert werden, und ein versehentliches Duplikat könnte eine
Produktions-Disk gegen eine Memory-Disk tauschen. Die Ersetzung findet
trotzdem statt; die Warnung macht den Tausch nur sichtbar.

### Warum Suprnova abweicht

Laravels `config/filesystems.php` listet jeden Disk-Treiber auf, und Sie
wählen zur Laufzeit einen aus; nichts wird herauskompiliert. Suprnova
gatet Azure und GCS hinter Features, weil die Wahl in Rust
Abhängigkeitskosten hat, und diese hier hat eine Sicherheitsdimension:
Beide opendal-Service-Crates ziehen `rsa` herein, das
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)
trägt (den Marvin-Timing-Angriff), ohne dass es upstream ein
korrigiertes Release gäbe. Sie zum Opt-in zu machen bedeutet, dass eine
App, die Dateien lokal oder auf S3 speichert, diese Crate nie mitführt.

S3 ist absichtlich *nicht* gegatet - sein Signer hing nie von `rsa` ab,
ein Gate würde also das meistgenutzte Cloud-Backend brechen und nichts
entfernen.

### Schutz vor Path Traversal

Lokale Dateisystem-Disks bekommen einen `PathGuardLayer`, der vor allen
nutzerseitig angegebenen Layern angewendet wird. Eine Anfrage wie
`disk.write("../escaped.txt", ..)` wird abgelehnt, bevor sie das
Betriebssystem erreicht - keine `..`-Komponente und kein absolutes
Präfix kann der Disk-Wurzel entkommen. Objektspeicher und das
In-Memory-Backend bekommen diese Absicherung nicht (ein Key wie
`../foo` ist auf diesen Backends nur ein gewöhnliches Key-Zeichen).

Nachdem `..` und absolute Komponenten abgelehnt wurden, kanonisiert die
Absicherung die lokale Disk-Wurzel und das angeforderte Ziel auf der
Platte. Bei bestehenden Zielen wird jede Symlink-Komponente aufgelöst;
für einen Pfad, der noch nicht existiert, geht die Absicherung bis zum
nächsten existierenden Vorfahren hoch und kanonisiert diesen. Die
Operation wird abgelehnt, wenn der so aufgelöste Pfad außerhalb der
kanonischen Wurzel liegt, sodass ein während der Validierung
beobachteter Symlink innerhalb der Wurzel einen Lese-, Schreib-,
List-, Kopier- oder Umbenennungsvorgang nicht nach außerhalb der Disk
umleiten kann.

Das ist eine Absicherung nach dem Muster kanonisieren-dann-arbeiten,
keine deskriptorrelative Dateisystem-Einsperrung. Sie setzt voraus, dass
der Disk-Wurzel und ihrem Inhalt gegenüber nebenläufiger Veränderung
vertraut wird: Ein Angreifer, der Verzeichnisse oder Symlinks nach der
Validierung, aber vor dem Öffnen des Pfads durch das Backend ersetzen
kann, gewinnt womöglich ein Time-of-Check-to-Time-of-Use-Race.
Verwenden Sie Isolation auf Betriebssystemebene oder ein dediziertes
Dateisystem, wenn andere Prinzipale den Storage-Baum nebenläufig
verändern können.

Streaming-Writer, -Lister und -Copier führen diese Prüfung des
aufgelösten Pfads einmal aus, unmittelbar vor ihrem ersten Backend-I/O.
Die Validierung ist danach für diese Stream-Sitzung fixiert, sodass
nicht jeder Chunk und jedes Element auf der Kanonisierung durch das
Dateisystem blockiert. Abbrüche von Copier und Writer reichen das
Aufräumen immer an ihre Backends weiter, auch vor der Aktivierung oder
wenn die Validierung nicht mehr abgeschlossen werden kann.

## Die Laravel-förmige Disk-Oberfläche

`Storage::disk(name)` liefert direkt einen
`suprnova::opendal::Operator`, sodass Sie seine volle
Streaming-Oberfläche nutzen können (`writer`, `reader`,
`presign_read`, `list`, `stat`, ...). Zusätzlich fügt das
[`DiskExt`]-Trait - blanket-implementiert auf `Operator` und
re-exportiert als `suprnova::DiskExt` - jede Laravel-Komfortmethode
hinzu, zu der Sie über `Storage::disk('local')->...` greifen würden.

Bringen Sie es mit `use suprnova::DiskExt;` in Scope.

### Existenzprüfungen

```rust,ignore
disk.exists("a.txt").await?;        // rohes opendal
disk.missing("a.txt").await?;       // Negation
disk.file_exists("a.txt").await?;   // nur Datei (kein Verzeichnis)
disk.file_missing("a.txt").await?;
disk.directory_exists("dir/").await?;
disk.directory_missing("dir/").await?;
```

### Lesen und Schreiben

| Laravel-Name | Rust-natives Äquivalent | Hinweis |
|--------------|------------------------|------|
| `get(path)`  | `read(path)`           | `get` liefert `Vec<u8>`; `read` liefert opendals `Buffer`. |
| `put(path, contents)` | `write(path, contents)` | Beide akzeptieren jedes `Into<Bytes>`. |
| `json::<T>(path)` | - | Liest und deserialisiert über serde_json. |
| `put_json(path, &value)` | - | Formatiert über serde_json lesbar (Pretty-Print). |
| `prepend(path, data)` | - | Verbindet mit `\n`. Für ein eigenes Trennzeichen `prepend_with_separator` verwenden. |
| `append(path, data)`  | - | Verbindet mit `\n`. Für ein eigenes Trennzeichen `append_with_separator` verwenden. |

`prepend` und `append` legen die Datei an, falls sie noch nicht
existiert, sodass sie als erster Schreibvorgang in eine Log-Datei
sicher sind.

### Metadaten

```rust,ignore
let bytes  = disk.size("a.bin").await?;          // u64
let when   = disk.last_modified("a.bin").await?; // Option<DateTime<Utc>>
let mime   = disk.mime_type("a.bin").await?;     // Option<String>
let digest = disk.checksum("a.bin", ChecksumAlgorithm::Sha256).await?;
```

`mime_type` fragt zuerst das Backend - S3, Azure und GCS geben den
gespeicherten `Content-Type` durch. Hat das Backend keinen,
schnüffelt es die ersten 16 KiB über die `infer`-Crate. `Ok(None)`
ist für unerkannte Binär-Blobs reserviert.

`checksum` unterstützt `Md5`, `Sha1` und `Sha256` über
[`ChecksumAlgorithm`]. MD5 und SHA-1 sind für die Parität zu Laravel
und zu Objektspeicher-ETags enthalten; wählen Sie SHA-256 für jede
neue Integritätsprüfung.

### Auflisten

```rust,ignore
let files = disk.files("docs", false).await?;     // Dateien der obersten Ebene
let all   = disk.all_files("docs").await?;        // rekursiv
let dirs  = disk.directories("docs", false).await?;
let all   = disk.all_directories("docs").await?;
```

Alle vier liefern ein sortiertes `Vec<String>`, sodass sich Aufrufer
über alle Backends hinweg auf eine stabile Reihenfolge verlassen
können. Verzeichnisse werden aus `files` herausgefiltert, und
umgekehrt. Verzeichnispfade werden **ohne** abschließenden
Schrägstrich zurückgegeben (`"docs/sub"`), um zu Laravels
`Storage::directories()`-Ausgabe zu passen - opendals zugrunde
liegendes `list` meldet `"docs/sub/"`, aber wir entfernen den
Schrägstrich für die Parität.

### Verzeichnisse und Dateien ändern

| Laravel-Name           | opendal-nativ        |
|------------------------|-----------------------|
| `make_directory(path)` | `create_dir(path)`    |
| `delete_directory(p)`  | `delete_with(p).recursive(true)` |
| `move_to(from, to)`    | `rename(from, to)`    |

`move_to` fällt auf `copy + delete` zurück, wenn das Backend kein
Rename unterstützt, und auf `read + write + delete`, wenn es auch
kein Copy unterstützt - sodass es sowohl gegen den in Tests
verwendeten In-Memory-Treiber als auch gegen Produktions-Backends
funktioniert.

### Vorsignierte URLs

```rust,ignore
let read_url   = disk.temporary_url("uploads/a.pdf", Duration::from_secs(900)).await?;
let upload_url = disk.temporary_upload_url("uploads/new.pdf", Duration::from_secs(900)).await?;
```

`temporary_url` und `temporary_upload_url` liefern die URL für die
Laravel-Parität als `String`. Sie werden von `Operator::presign_read`
/ `presign_write` getragen und liefern daher eine
`Unsupported`-Meldung auf Backends, die kein Vorsignieren
implementieren (der In-Memory- und der lokale Dateisystem-Treiber
fallen in diese Kategorie; S3, Azure Blob und GCS unterstützen es).

## Streaming-Kopie zwischen Disks

`copy_between_disks(src, src_path, dest, dest_path)` streamt das
Quellobjekt in 64-KiB-Chunks ins Ziel, unabhängig vom Backend-Paar.
Quelle und Ziel können von *jedem* opendal-Treiber getragen werden -
lokales Dateisystem zu S3, S3 zu Azure Blob, In-Memory zu GCS und so
weiter.

```rust,ignore
use suprnova::filesystem::streaming::copy_between_disks;

Storage::register_fs("local", "./storage")?;
Storage::register_memory("scratch");
let bytes = copy_between_disks("local", "uploads/big.bin", "scratch", "big.bin").await?;
```

Schlägt ein Schritt mitten in der Kopie fehl, wird das teilweise
entstandene Zielobjekt abgebrochen und gelöscht, bevor der
ursprüngliche Fehler propagiert - eine fehlgeschlagene Kopie ist nie
als abgeschnittenes Ziel beobachtbar.

## Registry-Hygiene

```rust,ignore
let removed = Storage::forget("local");  // bool: war er vorhanden?
Storage::purge();                        // jede Disk verwerfen
let names = Storage::disks();            // Vec<String>, sortiert
```

Das spiegelt Laravels `FilesystemManager::forgetDisk` / `purge` und
ist nützlich für Konfigurations-Reloads und Admin-Dashboards. Sie
sind nicht nur für Tests: Produktionscode muss gelegentlich eine
Disk zur Laufzeit verwerfen und neu registrieren (z. B. nach einer
Secrets-Rotation).

## Testen

`Storage::fake()` liefert einen Guard zurück, der:

1. einen prozessglobalen Mutex erwirbt, sodass gleichzeitige
   `#[tokio::test]`-Fälle nicht in eine Race Condition auf der
   geteilten Registry laufen, und
2. die Registry bei der Konstruktion und beim Drop zurücksetzt,
   sodass die Suite für den jeweils nächsten Test in einem sauberen
   Zustand bleibt.

Eine `"default"`-Memory-Disk ist zur Bequemlichkeit vorregistriert.

```rust,ignore
use suprnova::filesystem::testing::DiskAssertExt;
use suprnova::{DiskExt, Storage};

#[tokio::test]
async fn stores_and_asserts() {
    let _guard = Storage::fake();
    Storage::register_memory("uploads");
    let disk = Storage::disk("uploads").unwrap();

    disk.put("a.txt", b"hello".to_vec()).await.unwrap();

    disk.assert_exists("a.txt").await;
    disk.assert_contents("a.txt", b"hello").await;
    disk.assert_missing("not-here.txt").await;
    disk.assert_count("", 1, false).await;
    disk.assert_directory_empty("docs/").await;
}
```

Die fünf Assertion-Helfer - `assert_exists`, `assert_contents`,
`assert_missing`, `assert_count`, `assert_directory_empty` - werden
über das [`DiskAssertExt`]-Trait bereitgestellt, gesperrt hinter
`#[cfg(any(test, feature = "testing"))]`, sodass Produktionscode
nicht danach greifen kann.

## Kurzreferenz zur Parität

| Laravel `Storage::disk(...)->...`     | Suprnova                                                 |
|---------------------------------------|----------------------------------------------------------|
| `exists($path)`                       | `disk.exists(path)`                                      |
| `missing($path)`                      | `disk.missing(path)`                                     |
| `fileExists($path)` / `fileMissing`   | `disk.file_exists(path)` / `file_missing(path)`          |
| `directoryExists($p)` / `directoryMissing` | `disk.directory_exists(p)` / `directory_missing(p)` |
| `get($path)`                          | `disk.get(path)` (`Vec<u8>`)                             |
| `json($path)`                         | `disk.json::<T>(path)`                                   |
| `put($path, $contents)`               | `disk.put(path, bytes)`                                  |
| `prepend($path, $data)`               | `disk.prepend(path, data)`                               |
| `append($path, $data)`                | `disk.append(path, data)`                                |
| `size($path)`                         | `disk.size(path)`                                        |
| `lastModified($path)`                 | `disk.last_modified(path)`                               |
| `mimeType($path)`                     | `disk.mime_type(path)`                                   |
| `checksum($path, ['checksum_algo' => 'sha256'])` | `disk.checksum(path, ChecksumAlgorithm::Sha256)` |
| `files($dir, $recursive)`             | `disk.files(dir, recursive)`                             |
| `allFiles($dir)`                      | `disk.all_files(dir)`                                    |
| `directories($dir, $recursive)`       | `disk.directories(dir, recursive)`                       |
| `allDirectories($dir)`                | `disk.all_directories(dir)`                              |
| `makeDirectory($path)`                | `disk.make_directory(path)`                              |
| `deleteDirectory($path)`              | `disk.delete_directory(path)`                            |
| `move($from, $to)`                    | `disk.move_to(from, to)` (or opendal-native `rename`)    |
| `copy($from, $to)`                    | `disk.copy(from, to)` (opendal-native)                   |
| `delete($path)`                       | `disk.delete(path)` (opendal-native)                     |
| `temporaryUrl($path, $expiry)`        | `disk.temporary_url(path, expire)` (or opendal-native `presign_read`) |
| `temporaryUploadUrl($path, $expiry)`  | `disk.temporary_upload_url(path, expire)` (or opendal-native `presign_write`) |
| `Storage::fake()`                     | `Storage::fake()`                                        |
| `Storage::disk()->assertExists()`     | `disk.assert_exists(path).await`                         |
| `FilesystemManager::forgetDisk($n)`   | `Storage::forget(name)`                                  |
| `FilesystemManager::purge()`          | `Storage::purge()`                                       |

## Konfiguration

Die Storage-Konfiguration lebt vollständig in Rust-Code, nicht in
`.env`. Disks werden per Name in `bootstrap()` über
`Storage::register_*` registriert und an der Aufrufstelle per Name
angesprochen (`Storage::disk("public")`). Es gibt keine
`FILESYSTEM_DISK`-Env-Var, die das Framework liest, und keine
implizite Standard-Disk - jeder Treiber ist gleichrangig. Apps
entscheiden, welchen Disk-Namen ein gegebener Upload oder Download
anvisiert, und geben alle URLs / Schlüssel / Credentials, die der
gewählte Treiber braucht, als eigene Env-Vars weiter.

Siehe [Konfiguration](configuration.md) für die umfassendere Regel,
wo das Framework aus der Umgebung liest und wo es code-seitige
Registrierung erwartet.

## Nächste Schritte

- [Konfiguration](configuration.md) - was das Framework aus `.env`
  liest (und warum Storage nicht auf dieser Liste steht)
- [Anfragen](requests.md) - Datei-Uploads landen über
  `UploadedFile::store_as` auf einer Disk
- [Antworten](responses.md) - Bytes aus einer Disk zurückstreamen
- [Cache](cache.md) - die andere namensbasierte Treiber-Registry,
  dieselbe Form
- [Testen](testing.md) - die umfassendere
  Alles-Fake-Testoberfläche

[`DiskExt`]: https://docs.rs/suprnova/latest/suprnova/trait.DiskExt.html
[`DiskAssertExt`]: https://docs.rs/suprnova/latest/suprnova/filesystem/testing/trait.DiskAssertExt.html
[`ChecksumAlgorithm`]: https://docs.rs/suprnova/latest/suprnova/enum.ChecksumAlgorithm.html

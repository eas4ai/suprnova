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

Jede Disk wird beim Boot einmal über `Storage::register_*`
registriert und über `Storage::disk(name)` per Name nachgeschlagen. Es
gibt kein „Standard-Backend“, auf das die anderen zurückfallen - jeder
Treiber ist gleichrangig.

| Konstruktor                          | Backend                       | Feature             |
|---------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | Lokales Dateisystem            | `filesystem`        |
| `Storage::register_memory(name)`     | In-Process-Memory (Tests)      | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3 oder S3-kompatibel   | `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage             | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage           | `filesystem-gcs`    |

`filesystem` ist standardmäßig aktiviert; die Azure- und GCS-Features
sind es nicht. Aktivieren Sie eines davon in Ihrer `Cargo.toml`:

```toml
[dependencies]
suprnova = { git = "https://github.com/entrepeneur4lyf/suprnova.git", tag = "v1.2.0", features = ["filesystem-gcs"] }
```

Ohne das Feature existieren `register_azblob` / `register_gcs` und
ihre Konfigurationsstrukturen nicht - Sie erhalten einen
Compile-Fehler, der das fehlende Element benennt, statt eines
Laufzeitfehlers.

Jeder Konstruktor hat eine `_with`-Variante, die Ihnen den
`suprnova::opendal::Operator` unmittelbar bevor er in der Registry
landet in die Hand gibt, sodass Sie Retry-/Timeout-/Logging-Schichten
darum herum installieren können:

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
`register_gcs`) wenden standardmäßig eine `RetryLayer` (3 Versuche)
an, da vorübergehende Drosselung / 5xx-Fehler bei Objektspeichern
alltäglich sind. Verwenden Sie die `_with`-Varianten, wenn Sie die
volle Kontrolle brauchen.

Der vollständige Satz der von Suprnova verdrahteten opendal-Schichten
ist `RetryLayer`, `TimeoutLayer`, `LoggingLayer`, `TracingLayer`
(bindet sich über `tracing-opentelemetry` an OTel an, wenn das
`otel`-Feature des Frameworks aktiviert ist) und
`PrometheusClientLayer` (exportiert Histogramme und Zähler in eine
`prometheus_client::registry::Registry`, die Ihnen gehört). Die
Schicht-Reihenfolge ist wichtig - die äußerste Schicht umschließt
alles darin - und der idiomatische Stack ist `RetryLayer →
TimeoutLayer → LoggingLayer`, sodass ein Versuch, der in ein Timeout
läuft, trotzdem protokolliert wird und eine Wiederholung
Transportfehler abdeckt.

Registrieren Sie denselben Namen erneut, ersetzt das den vorherigen
Operator und gibt ein `warn!`-Log aus - Disks sollen einmal beim Boot
registriert werden, und ein versehentliches Duplikat könnte eine
Produktions-Disk gegen eine Memory-Disk austauschen. Der Ersatz
findet trotzdem statt; die Warnung macht den Tausch nur hörbar.

### Warum Suprnova abweicht

Laravels `config/filesystems.php` listet jeden Disk-Treiber auf, und
Sie wählen zur Laufzeit einen aus; nichts wird herauskompiliert.
Suprnova sperrt Azure und GCS hinter Features, weil die Wahl in Rust
Abhängigkeitskosten hat, und diese hier hat eine
Sicherheitsdimension: Beide opendal-Service-Crates ziehen `rsa` nach,
das [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)
(den Marvin-Timing-Angriff) trägt, ohne dass es dafür upstream ein
Fix-Release gäbe. Weil sie Opt-in sind, führt eine App, die Dateien
lokal oder auf S3 speichert, diese Crate nie mit.

S3 ist bewusst *nicht* gesperrt - sein Signer hing nie von `rsa` ab,
sodass eine Sperrung das meistgenutzte Cloud-Backend brechen und
nichts entfernen würde.

### Schutz vor Path-Traversal

Lokale Dateisystem-Disks haben eine `PathGuardLayer`, die vor jeder
benutzerdefinierten Schicht angewendet wird. Eine Anfrage wie
`disk.write("../escaped.txt", ..)` wird abgewiesen, bevor sie das
Betriebssystem erreicht - keine `..`-Komponente und kein absolutes
Präfix kann die Disk-Wurzel verlassen. Objektspeicher und das
In-Memory-Backend bekommen diesen Schutz nicht (ein Schlüssel wie
`../foo` ist auf diesen Backends nur ein gewöhnliches
Schlüsselzeichen).

Nach dem Zurückweisen von `..` und absoluten Komponenten kanonisiert
der Schutz die lokale Disk-Wurzel und das angeforderte Ziel auf der
Disk. Bei existierenden Zielen wird jede Symlink-Komponente
aufgelöst; für einen Pfad, der noch nicht existiert, läuft der Schutz
bis zum nächsten existierenden Vorfahren hinauf und kanonisiert ihn.
Die Operation wird zurückgewiesen, wenn der aufgelöste Pfad außerhalb
der kanonischen Wurzel liegt, sodass ein In-Root-Symlink, der während
der Validierung beobachtet wurde, ein Lesen, Schreiben, Auflisten,
Kopieren oder Umbenennen nicht aus der Disk heraus umleiten kann.

Das ist ein Kanonisieren-dann-Operieren-Schutz, keine
deskriptorrelative Dateisystem-Eingrenzung. Er setzt voraus, dass die
Disk-Wurzel und ihr Inhalt gegen gleichzeitige Veränderung
vertrauenswürdig sind: Ein Angreifer, der Verzeichnisse oder Symlinks
nach der Validierung, aber bevor das Backend den Pfad öffnet,
ersetzen kann, kann eine Time-of-Check-to-Time-of-Use-Race gewinnen.
Verwenden Sie Isolation auf Betriebssystemebene oder ein dediziertes
Dateisystem, wenn andere Akteure den Storage-Baum gleichzeitig
verändern können.

Streamende Writer, Lister und Copier führen diese
Pfadauflösungsprüfung einmal aus, unmittelbar vor ihrer ersten
Backend-I/O. Die Validierung ist für diese Stream-Session dann fix,
sodass kein Chunk und kein Element auf Dateisystem-Kanonisierung
warten muss. Abbrüche von Copiern und Writern leiten die Bereinigung
immer an ihr Backend weiter, selbst vor der Aktivierung oder wenn die
Validierung nicht mehr abgeschlossen werden kann.

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

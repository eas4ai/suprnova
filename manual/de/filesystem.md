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
über `Storage::disk(name)` namentlich nachgeschlagen. Es gibt kein
„Standard-Backend“, auf das die anderen zurückfallen - jeder Treiber ist
ein Gleichrangiger.

| Konstruktor | Backend | Feature |
|---|---|---|
| `Storage::register_fs(name, root)` | Lokales Dateisystem | `filesystem` |
| `Storage::register_memory(name)` | Prozessinterner Speicher (Tests) | `filesystem` |
| `Storage::register_s3(name, cfg)` | Amazon S3 oder S3-kompatibel | `filesystem` |
| `Storage::register_azblob(name, cfg)` | Azure Blob Storage | `filesystem-azure` |
| `Storage::register_gcs(name, cfg)` | Google Cloud Storage | `filesystem-gcs` |
| `Storage::register_read_through(name, cfg)` | Read-Through-Komposition | `filesystem` |

`filesystem` ist standardmäßig an; die Features für Azure und GCS sind es
nicht. Schalten Sie eines in Ihrer `Cargo.toml` ein:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.4", features = ["filesystem-gcs"] }
```

Ohne das Feature existieren `register_azblob` / `register_gcs` und ihre
Config-Strukturen nicht - Sie bekommen einen Compile-Fehler, der das
fehlende Element benennt, keinen Laufzeitfehler.

Jeder Konstruktor hat eine `_with`-Variante, die Ihnen den
`suprnova::opendal::Operator` unmittelbar vor der Aufnahme in die Registry
in die Hand gibt, sodass Sie Layer für Wiederholung, Timeout oder Logging
um ihn herum installieren können:

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
`register_gcs`) legen standardmäßig einen `RetryLayer` (3 Versuche) an, da
transiente Drosselungen und 5xx-Fehler bei Objektspeichern zum Alltag
gehören. Nutzen Sie die `_with`-Varianten, wenn Sie volle Kontrolle
brauchen.

Der vollständige Satz an opendal-Layern, die Suprnova verdrahtet, lautet
`RetryLayer`, `TimeoutLayer`, `LoggingLayer`, `TracingLayer` (Brücke zu
OTel über `tracing-opentelemetry`, wenn das `otel`-Feature des Frameworks
an ist) und `PrometheusClientLayer` (exportiert Histogramme und Zähler in
eine `prometheus_client::registry::Registry`, die Ihnen gehört). Die
Reihenfolge der Layer zählt - der äußerste Layer umschließt alles, was
darin liegt -, und der idiomatische Stapel lautet
`RetryLayer → TimeoutLayer → LoggingLayer`, damit ein in ein Timeout
gelaufener Versuch trotzdem protokolliert wird und eine Wiederholung
Transportfehler abdeckt.

Denselben Namen erneut zu registrieren ersetzt den vorherigen Operator und
gibt ein `warn!`-Log aus - Disks sollen beim Boot einmal registriert
werden, und ein versehentliches Duplikat könnte eine Produktions-Disk
gegen eine Memory-Disk tauschen. Der Austausch findet trotzdem statt; die
Warnung macht ihn nur hörbar.

### Warum Suprnova abweicht

Laravels `config/filesystems.php` listet jeden Disk-Treiber auf, und Sie
wählen zur Laufzeit einen aus; nichts wird herauskompiliert. Suprnova gatet
Azure und GCS hinter Features, weil die Wahl in Rust einen
Abhängigkeitspreis hat, und dieser hier hat eine Sicherheitsdimension:
Beide opendal-Service-Crates ziehen `rsa` herein, das
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071) (den
Marvin-Timing-Angriff) trägt und stromaufwärts keine behobene
Veröffentlichung hat. Sie zur Opt-in-Sache zu machen bedeutet, dass eine
App, die Dateien lokal oder auf S3 ablegt, diese Crate nie mitträgt.

S3 ist bewusst *nicht* gegatet - sein Signierer hing nie von `rsa` ab, es
zu gaten würde also das meistgenutzte Cloud-Backend kaputtmachen und nichts
entfernen.

### Atomare lokale Schreibvorgänge

Auf einer lokalen Disk macht jede Operation, die Bytes an einem Pfad
sichtbar macht, sie in einem einzigen Schritt sichtbar. `disk.write(...)`,
`disk.writer(...)` und `disk.copy(...)` landen zuerst in
`<root>/.suprnova-atomic/`, werden dort geflusht und synchronisiert und
dann auf das Ziel umbenannt; `disk.rename(...)` ist ohnehin schon ein
einziger Schritt. Ein gleichzeitig lesender Prozess sieht deshalb entweder
das vorherige Objekt oder das fertige neue und nie eine unvollständige
Länge, und ein Prozess, der mitten im Schreiben stirbt, lässt das Ziel
unberührt, statt es an Ort und Stelle abgeschnitten zu hinterlassen.

`append` ist die einzige Operation, die an Ort und Stelle arbeitet, denn
ein zwischengelagertes `append` müsste zuerst das ganze Objekt kopieren.
Das gilt für das `append`, das das Objekt *anlegt*, genauso wie für jedes
weitere danach, sodass zwei Writer, die an dasselbe neue Objekt anhängen,
beide ankommen. An Ort und Stelle zu arbeiten ist zugleich der Preis eines
`append`: Eines, das fehlschlägt oder abgebrochen wird, lässt das Objekt
zurück, leer oder zu kurz - genau wie ein `append` auf ein bestehendes
Objekt es immer schon getan hat.

Ein bedingter Schreibvorgang wird mit `link(2)` sichtbar gemacht statt per
Umbenennen, und bleibt damit ein echtes exklusives Anlegen statt einer
Prüfung mit anschließendem Überschreiben:

```rust,ignore
// Genau einer von beliebig vielen konkurrierenden Aufrufern bekommt hier
// Ok. Jeder andere bekommt einen `ErrorKind::ConditionNotMatch`-Fehler und
// schreibt nichts.
disk.write_with("locks/import.json", body).if_not_exists(true).await?;
```

Dieses Sichtbarmachen braucht ein Dateisystem mit Hardlinks. Auf FAT,
exFAT und manchen Netzwerkdateisystemen wird `link(2)` nicht unterstützt,
und ein bedingter Schreibvorgang schlägt dort fehl, statt stillschweigend
auf eine Prüfung mit anschließendem Überschreiben zurückzufallen - was
Ihnen eine Exklusivitätsgarantie in die Hand gäbe, die nicht hält. Jede
andere Operation bleibt davon unberührt.

Weil das Objekt per Umbenennen sichtbar gemacht wird, wechselt seine Inode.
Ein erneutes Schreiben erhält deshalb weder Modus noch Eigentümer noch die
Hardlinks der vorherigen Datei, und ein Leser, der einen offenen Deskriptor
hält, liest weiter den alten Inhalt, statt die neuen Bytes zu sehen. Das
ist der übliche Preis für atomares Sichtbarmachen, aber es ist eine
Änderung, falls Sie sich auf eines von beidem verlassen haben.

Ein Pfad, der die Disk über einen Symlink erreicht, den der Schutz nicht
auflösen kann - einen toten, dessen Ziel nicht existiert -, wird abgelehnt,
statt als freier Name zum Anlegen behandelt zu werden. Durch einen solchen
Link hindurch anzulegen würde das Ziel des Links anlegen, irgendwo auf dem
Host; der Schutz kann einen harmlosen toten Link also nicht von einem
Ausbruch unterscheiden und lehnt beide ab.

Der Name `.suprnova-atomic` ist im Wurzelverzeichnis jeder lokalen Disk
reserviert. Jeder Pfad, dessen erste Komponente dieser Name ist, wird mit
einem Berechtigungsfehler abgelehnt, und ebenso jeder Pfad, der sich über
einen Symlink in das Verzeichnis hinein *auflöst*; Sie können also weder
die zwischengelagerte Datei eines anderen Writers lesen noch in das
Verzeichnis schreiben noch es löschen. Der Eintrag wird aus `files`,
`directories`, `all_files` und `all_directories` herausgefiltert und
taucht so nie als Objekt auf. Der Name wird als
`suprnova::ATOMIC_STAGING_DIR` exportiert, weil Backup- und Sync-Werkzeuge
ihn brauchen: Schließen Sie das Verzeichnis so aus, wie Sie ein
Sperrverzeichnis ausschließen würden. Es enthält die temporären Dateien
laufender Schreibvorgänge und alles, was ein mitten im Vorgang gestorbener
Prozess hinterlassen hat, und nichts räumt das weg - ein Host in einer
Absturzschleife lässt es also wachsen, bis jemand es leert, was gefahrlos
möglich ist, solange nicht geschrieben wird.

### Schutz vor Path Traversal

Disks auf dem lokalen Dateisystem bekommen einen `PathGuardLayer`, der vor
allen vom Nutzer angegebenen Layern angewandt wird. Eine Anfrage wie
`disk.write("../escaped.txt", ..)` wird abgelehnt, bevor sie das
Betriebssystem erreicht - keine `..`-Komponente und kein absolutes Präfix
kann aus dem Disk-Wurzelverzeichnis ausbrechen. Objektspeicher und das
In-Memory-Backend bekommen den Schutz nicht (ein Schlüssel wie `../foo`
besteht dort nur aus gewöhnlichen Schlüsselzeichen).

Nachdem der Schutz `..` und absolute Komponenten abgelehnt hat,
kanonisiert er das Wurzelverzeichnis der lokalen Disk und das angefragte
Ziel auf der Platte. Bei bestehenden Zielen wird jede Symlink-Komponente
aufgelöst; bei einem Pfad, der noch nicht existiert, geht der Schutz bis
zum nächstgelegenen existierenden Vorfahren hinauf und kanonisiert diesen.
Die Operation wird abgelehnt, wenn der so aufgelöste Pfad außerhalb des
kanonischen Wurzelverzeichnisses liegt, damit ein während der Prüfung
beobachteter Symlink innerhalb des Wurzelverzeichnisses kein Lesen,
Schreiben, Auflisten, Kopieren oder Umbenennen aus der Disk
hinausleiten kann.

Das ist ein Schutz nach dem Muster „kanonisieren, dann operieren“, keine
deskriptorrelative Einsperrung im Dateisystem. Er setzt voraus, dass dem
Disk-Wurzelverzeichnis und seinem Inhalt gegenüber nebenläufigen
Änderungen vertraut wird: Wer Verzeichnisse oder Symlinks nach der Prüfung
und vor dem Öffnen des Pfades durch das Backend ersetzen kann, gewinnt
möglicherweise eine Time-of-Check-to-Time-of-Use-Race. Nutzen Sie
Isolation auf Betriebssystemebene oder ein eigenes Dateisystem, wenn
andere Prinzipale den Speicherbaum nebenläufig verändern können.

Streaming-Writer, -Lister und -Copier führen diese Prüfung des aufgelösten
Pfades einmal aus, unmittelbar vor ihrer ersten Backend-E/A. Danach steht
die Prüfung für diese Stream-Sitzung fest, sodass nicht jeder Chunk und
jeder Eintrag auf die Kanonisierung im Dateisystem warten muss. Abbrüche
von Copier und Writer reichen die Aufräumarbeit immer an ihre Backends
weiter, auch vor der Aktivierung oder wenn die Prüfung nicht mehr
abgeschlossen werden kann.

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

## Read-Through-Disks

Eine Read-Through-Disk paart eine schnelle *primäre* Disk mit einem
langsameren *Fallback* und verschiebt Objekte vom zweiten auf die erste,
während sie gelesen werden. Richten Sie die primäre Disk auf den Speicher,
zu dem Sie migrieren, und den Fallback auf den, von dem Sie migrieren, und
das Arbeitsset wandert unter echtem Verkehr hinüber - kein Wartungsfenster,
kein Massenkopieren von Objekten, nach denen niemand fragt.

```rust,ignore
use suprnova::{ReadThroughConfig, S3Config, Storage};

Storage::register_s3("new-store", S3Config { bucket: "assets-2".into(), ..Default::default() })?;
Storage::register_s3("legacy-store", S3Config { bucket: "assets-1".into(), ..Default::default() })?;

Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "new-store".into(),
        fallback: "legacy-store".into(),
        ..Default::default()
    },
)?;

let assets = Storage::disk("assets")?;
// Liest `logo.png` aus `legacy-store` und schreibt es auf dem Weg nach
// draußen nach `new-store`. Jedes spätere Lesen bedient `new-store`.
let bytes = assets.read("logo.png").await?;
```

`Storage::disk("assets")` gibt einen gewöhnlichen `Operator` zurück, jede
Methode darauf und jede `DiskExt`-Bequemlichkeit funktioniert also
unverändert.

### Welche Disk welche Operation beantwortet

| Operation | Disk |
|---|---|
| `read` | Die primäre, wenn sie das Objekt hält, sonst der Fallback - und sofern `copy` nicht `false` ist, wird der Fallback-Treffer hochgezogen |
| `exists`, `size`, `last_modified`, `mime_type`, `stat` | Die primäre, wenn sie das Objekt hält, sonst der Fallback |
| `write`, `make_directory` | Nur die primäre |
| `files`, `directories`, `list` | Nur die primäre - Fallback-Einträge sind für eine Auflistung unsichtbar |
| `delete` | Beide, der Fallback zuerst |
| `copy`, `rename` / `move_to` | Die primäre, wenn sie die Quelle hält, sonst wird vom Fallback herübergestreamt; ein `rename` löscht zusätzlich die Quelle auf dem Fallback |
| `temporary_url` | Die primäre, wenn sie das Objekt hält, sonst der Fallback |
| `temporary_upload_url` | Nur die primäre - ein Upload muss dort landen, wo Schreibzugriffe landen |

Das Auflisten ist bewusst auf die primäre Disk beschränkt. Eine vereinigte
Auflistung müsste Paginierung und Reihenfolge über zwei Backends hinweg in
Einklang bringen, und sie würde Objekte melden, die eine spätere Auflistung
nicht mehr zurückgibt, sobald sie hochgezogen wurden. Nutzen Sie
`Storage::disk("legacy-store")` direkt, wenn Sie aufzählen müssen, was auf
dem Fallback übrig ist.

Ein Löschen entfernt das Objekt von beiden Disks. Entfernte es nur die
Kopie auf der primären Disk, würde das nächste Lesen die Fallback-Kopie
umgehend wieder hochziehen. Die Folge ist, dass eine Read-Through-Disk über
einem schreibgeschützten Fallback nicht löschen kann: Das Löschen auf dem
Fallback schlägt fehl, und der Fehler erreicht Sie.

### Wenn ein Hochziehen fehlschlägt

Standardmäßig wird ein Fehlschlag beim Hochziehen auf `warn` protokolliert
und verschluckt. Sie erhalten trotzdem die angefragten Bytes; die Disk fällt
lediglich darauf zurück, jedes Mal den Fallback zu lesen, bis die primäre
Disk wieder beschreibbar ist. Setzen Sie
`throw_on_promotion_failure: true`, wenn ein stiller Verlust des
Hochziehens einen Fehler verbergen würde, den Sie sehen müssen - etwa bei
einer Migration, die Sie abschließen wollen:

```rust,ignore
Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "new-store".into(),
        fallback: "legacy-store".into(),
        throw_on_promotion_failure: true,
        ..Default::default()
    },
)?;
```

Die Registrierung lehnt eine Konfiguration ab, die nicht funktionieren
kann: ein leeres `primary` oder `fallback`, ein Paar, das dieselbe Disk
zweimal benennt, eine Disk, die sich selbst benennt, oder ein Name, der
nicht registriert ist. Jeder Fall gibt einen `FrameworkError` zurück, der
das Problem benennt, und es wird keine Disk registriert.

### Lesen ohne Hochziehen

Setzen Sie `copy: false`, um Fallback-Treffer auszuliefern, ohne sie
durchzuschreiben:

```rust,ignore
Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "cache-store".into(),
        fallback: "origin-store".into(),
        copy: false,
        ..Default::default()
    },
)?;
```

Die Disk liest sich dann wie ein transparentes Overlay: Die primäre Disk
antwortet auf das, was sie hält, der Fallback auf alles andere, und
zwischen ihnen bewegt sich nichts. Nutzen Sie das, wenn die primäre Disk
ein kleiner Cache ist, den ein einmaliges Lesen nicht füllen soll, oder
wenn der Fallback maßgeblich ist und die primäre Disk nur Objekte hält, die
Sie absichtlich dort abgelegt haben.

Das Flag steuert das Hochziehen beim Lesen und sonst nichts.
Schreibzugriffe, Löschungen, Metadaten, Auflistungen und die Ziele von
`copy` und `rename` verhalten sich alle genau so wie bei eingeschaltetem
Hochziehen - eine `copy: false`-Disk legt ein kopiertes oder verschobenes
Objekt also weiterhin auf der primären Disk ab. Da nichts zurückgeschrieben
wird, holt ein Lesen mit `copy: false` nur den angefragten Bereich statt
des ganzen Objekts.

### Kopieren und Verschieben über den Fallback hinweg

`copy` und `rename` lösen die Quelle zuerst gegen die primäre Disk auf.
Hält nur der Fallback sie, wird das Objekt in 64-KiB-Blöcken herübergestreamt,
und das Ziel landet auf der primären Disk:

```rust,ignore
let assets = Storage::disk("assets")?;

// `logo.png` liegt nur auf `legacy-store`. Die Kopie streamt es herüber
// und schreibt `branding/logo.png` nach `new-store`; das Altobjekt bleibt
// liegen.
assets.copy("logo.png", "branding/logo.png").await?;

// Ein Verschieben tut dasselbe und löscht danach die alte Quelle.
assets.rename("logo.png", "branding/logo.png").await?;
```

Ein Verschieben löscht die Quelle auf dem Fallback auf beiden Pfaden -
gleich, ob die primäre Disk die Quelle hielt oder nicht. Ohne das würde das
nächste Lesen die Fallback-Kopie zurückholen und das Verschieben rückgängig
machen.

Die beiden Pfade unterscheiden sich darin, wann sie sie löschen, und dieser
Unterschied bestimmt, was ein fehlgeschlagenes Verschieben hinterlässt:

- Die primäre Disk hielt die Quelle. Die Fallback-Kopie geht zuerst, vor dem
  Umbenennen. Solange die primäre Disk den Pfad hält, ist die Kopie des
  Fallbacks über diese Disk ohnehin unerreichbar, sie zuerst zu entfernen
  ändert also nichts, was Sie beobachten könnten - und wenn das Löschen
  fehlschlägt, hat sich noch nichts bewegt. Wiederholen Sie das Verschieben.
  Gelang stattdessen das Löschen und schlug dann das Umbenennen fehl, hält
  der Fallback für diesen Pfad nichts mehr, das Ziel ist ungeschrieben, und
  die primäre Disk hält weiterhin die Quelle - eine Wiederholung nimmt also
  denselben Pfad und benennt erneut um. Der Fehlschlag kostet die kalte
  Kopie und sonst nichts.
- Nur der Fallback hielt sie. Das Löschen kann erst kommen, wenn das Ziel
  steht, ein Verschieben, das beim Löschen scheitert, hinterlässt also das
  geschriebene Ziel und die Quelle weiterhin auf dem Fallback. Wiederholen
  Sie das Verschieben; die Quelle liegt nun auf der primären Disk, die
  Wiederholung nimmt also den ersten Pfad.

In beiden Fällen lässt sich ein fehlgeschlagenes Verschieben gefahrlos
wiederholen, und das Ziel, mit dem Sie enden, ist das Objekt, von dem das
Verschieben ausging.

Bedingungen werden auch auf dem Streaming-Pfad mit der Operation
weitergereicht. `if_not_exists` wird zu einem bedingten Schreiben, sodass eine abgesicherte
Kopie oder Verschiebung ein bestehendes Ziel weiterhin verweigert, statt es
zu überschreiben, und eine Kopie, die eine Quellversion benennt, bekommt
diese Version aus dem Fallback. Das `if_match` einer Kopie ist die eine
Ausnahme: Es ist eine Bedingung, die das Backend innerhalb seiner eigenen
Kopie anwendet - genau der Aufruf, den dieser Pfad nicht machen kann -, es
wird deshalb mit einem `Unsupported`-Fehler abgelehnt, der die Bedingung
benennt, statt stillschweigend ignoriert zu werden.

Damit sind Bedingungen die eine Stelle, an der durchschlägt, welche Disk
die Quelle hält. Ein lokales Verzeichnis bietet `copy` und `rename` an,
aber keine ihrer bedingten Formen, `copy_with(a, b).if_not_exists(true)`
gelingt also, wenn nur der Fallback `a` hält (es wird zu einem bedingten
Schreiben), und wird mit `Unsupported` abgelehnt, wenn die primäre Disk es
hält. Prüfen Sie die Bedingung, die Sie brauchen, gegen den primären
Treiber, statt anzunehmen, dass sie für jedes Objekt auf der Disk gilt.

Ein Verschieben, das die primäre Disk verweigern würde, wird verweigert,
bevor irgendetwas gelöscht wird. Eine primäre Disk ganz ohne `rename`, eine
abgesicherte Verschiebung auf eine primäre Disk ohne bedingtes `rename` und
eine abgesicherte Verschiebung auf ein bereits existierendes Ziel scheitern
alle, während die Quelle auf dem Fallback weiterhin an Ort und Stelle
liegt - ein Verschieben, das nie stattfindet, darf Sie nicht die kalte
Kopie kosten.

Schlägt der Stream unterwegs fehl, wird der Writer abgebrochen und ein vom
Transfer angelegtes Ziel gelöscht, bevor der Fehler Sie erreicht, ein
fehlgeschlagener Transfer ist also nicht als abgeschnittenes Objekt
beobachtbar. Ein Ziel, das schon da war, bleibt unangetastet - eine
fehlgeschlagene Kopie darf nicht dasjenige sein, was ein Objekt zerstört,
das sie nie geschrieben hat. Eine primäre Disk im lokalen Dateisystem hält
das ebenfalls ein, denn sie lagert den Transfer unter `.suprnova-atomic/`
zwischen und benennt erst bei Erfolg um; den Writer abzubrechen entfernt die
zwischengelagerte Datei, ein fehlgeschlagener Transfer hinterlässt also
weder ein unvollständiges Ziel noch eine übrig gebliebene temporäre Datei.

### Versionierte und bedingte Lesezugriffe

Ein Lesen, das eine Version oder eine Bedingung `If-Match`, `If-None-Match`,
`If-Modified-Since` oder `If-Unmodified-Since` mitbringt, wird mit dieser
Bedingung unverändert weitergereicht, sodass die Antwort das bedeutet, was
Sie sie zu bedeuten hießen. Ein solches Lesen wird bedient, aber nie
hochgezogen: Eine alte Version oder einen von einem Validator getroffenen
Rumpf auf die primäre Disk zu schreiben würde ihn als das aktuelle Objekt
veröffentlichen, und jedes spätere schlichte Lesen bekäme ihn.

Welche Disk ein solches Lesen beantwortet, entscheidet sich auf dem
üblichen Weg. Die erste Sondierung ist eine gewöhnliche Existenzprüfung,
eine Read-Through-Disk delegiert ein versioniertes oder bedingtes Lesen
also immer dann an die primäre Disk, wenn diese den Pfad überhaupt hält;
sie erreicht den Fallback nur, wenn die primäre ihn nicht hält.

Die primäre Disk entscheidet außerdem, welche dieser Lesezugriffe eine
Read-Through-Disk überhaupt annimmt, denn der Reader der primären Disk wird
zuerst geöffnet. Ein versioniertes Lesen gegen eine Read-Through-Disk,
deren primäre Disk ein lokales Verzeichnis ist, wird abgelehnt, bevor es den
Fallback erreicht, denn ein lokales Verzeichnis hat keine Versionen.

### Warum Suprnova abweicht

Laravel baut eine Read-Through-Disk aus einem Eintrag in
`config/filesystems.php`, dessen Schlüssel `primary` und `fallback`
entweder einen Disk-Namen oder eine eingebettete Treiber-Config annehmen.
Suprnova nimmt nur Disk-Namen, weil Disks hier von typisierten
Konstruktoren registriert und nicht von Arrays beschrieben werden -
registrieren Sie zuerst die innere Disk und benennen Sie sie dann.

Laravels Hochziehen prüft die primäre Disk nach dem Lesen des Fallbacks
erneut, wodurch ein nebenläufiger Schreiber gewinnt. Suprnova behält diese
Prüfung bei und veröffentlicht das Hochziehen atomar, was Laravel nicht
tut. Auf einer primären Disk im lokalen Dateisystem werden die Bytes an
einem temporären Geschwisterpfad zwischengelagert und an ihren Platz
umbenannt; sie direkt auf das Ziel zu schreiben würde für die Dauer des
Schreibens eine wachsende, halb geschriebene Datei sichtbar lassen, und
eine Read-Through-Disk leitet Leser über genau diese Existenzprüfung. Auf
einer primären Disk ohne Umbenennen - In-Memory, S3, Azure Blob, GCS - ist
ein Schreiben ohnehin eine einzige unteilbare Veröffentlichung, das
Hochziehen schreibt das Ziel also direkt, unter der Bedingung, dass das
Objekt nicht bereits existiert, damit nicht zwei nebenläufige Leser beide
hochziehen.

Genau diese Bedingung kann ein zwischengelagertes Hochziehen nicht haben:
Der Zwischenpfad ist eindeutig, eine Nicht-Überschreiben-Bedingung darauf
wäre also gehaltlos, und das Ziel wird durch ein überschreibendes
Umbenennen veröffentlicht. Eine Read-Through-Disk auf einer primären Disk
im lokalen Dateisystem gibt sie deshalb auf - ein Schreiben, das im Moment
zwischen der letzten Existenzprüfung des Hochziehens und seinem Umbenennen
auf der primären Disk landet, wird von der hochgezogenen Kopie
überschrieben. Auf einer primären Disk ohne Umbenennen gilt die Bedingung,
und ein solches Fenster gibt es nicht.

Das zwischengelagerte Objekt ist, solange es besteht, ein echter Eintrag
auf der primären Disk, eine Auflistung mitten im Hochziehen kann also ein
Geschwister `.suprnova-promote-<id>.tmp` zeigen. Ein Lesen, das
abgeschlossen wird, fehlschlägt oder aufgibt, versucht, sein eigenes
Geschwister zu entfernen, und protokolliert eine Warnung, wenn dieses
Löschen fehlschlägt, statt das Lesen scheitern zu lassen. Nichts kehrt ein
Geschwister auf, das ein fehlgeschlagenes Löschen, ein abgestürzter Prozess
oder ein mitten im Hochziehen abgebrochenes Lese-Future hinterlassen hat:
Diese müssen von Hand entfernt werden.

Ein Lesen, das sich aus dem Fallback auflöst, hält das Objekt im Speicher,
bis das Schreiben des Hochziehens abgeschlossen ist, denn das Hochziehen
braucht das ganze Objekt. Das passt zum Tiering-Fall, für den eine
Read-Through-Disk da ist. Lesen Sie bei sehr großen kalten Objekten die
Fallback-Disk direkt oder nutzen Sie stattdessen
[`copy_between_disks`](#streaming-kopie-zwischen-disks).

Laravel reicht bei `copy` gleich `false` den eigenen Stream des Fallbacks
zurück und puffert bei `true` über `php://temp`. Suprnova verengt den
Fallback-Abruf stattdessen bei `copy` gleich `false` auf den angefragten
Bereich und puffert nur auf dem hochziehenden Pfad, wo das ganze Objekt
ohnehin gebraucht wird.

Laravels `copy` und `move` über den Fallback hinweg puffern die Quelle
ebenfalls über `php://temp`. Suprnova streamt sie stattdessen in
64-KiB-Blöcken, weil auf dem Fallback die großen, selten angefassten
Objekte liegen, und löscht ein halb geschriebenes Ziel, bevor es den Fehler
zurückgibt. Zwei weitere Unterschiede folgen aus OpenDAL. Einen Pfad zu
löschen, der nicht da ist, zählt als Erfolg, ein Verschieben räumt die
Quelle auf dem Fallback also ab, ohne vorher zu prüfen, ob sie existiert.
Und OpenDAL trägt Bedingungen auf `copy` und `rename`, für die Flysystem
keine Entsprechung hat, Suprnova muss also entscheiden, was jede von ihnen
bedeutet, wenn die Quelle nur auf dem Fallback liegt: `if_not_exists` und
die Quellversion einer Kopie werden beachtet, und das `if_match` einer
Kopie wird abgelehnt statt fallen gelassen.

Laravel löscht die Quelle auf dem Fallback auf beiden Pfaden nach dem
Verschieben. Suprnova löscht sie zuerst, wenn die primäre Disk die Quelle
hält, denn die beiden Reihenfolgen unterscheiden sich bei einer
Wiederholung: Über die Disk ist die Quelle so oder so unerreichbar, aber
zuletzt zu löschen bedeutet, dass ein Verschieben, das sein Löschen an
einen transienten Fehler verloren hat, als Verschieben zurückkommt, dessen
Quelle jetzt nur noch auf dem Fallback liegt, und die veraltete Kopie des
Fallbacks über das Ziel streamt, das der erste Versuch bereits korrekt
geschrieben hat.

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
| `move($from, $to)`                    | `disk.move_to(from, to)` (oder opendal-nativ `rename`)   |
| `copy($from, $to)`                    | `disk.copy(from, to)` (opendal-nativ)                    |
| `delete($path)`                       | `disk.delete(path)` (opendal-nativ)                      |
| `temporaryUrl($path, $expiry)`        | `disk.temporary_url(path, expire)` (oder opendal-nativ `presign_read`) |
| `temporaryUploadUrl($path, $expiry)`  | `disk.temporary_upload_url(path, expire)` (oder opendal-nativ `presign_write`) |
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

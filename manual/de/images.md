# Bilder

Suprnova liefert eine Laravel-förmige Bild-Pipeline: Sie bauen sie in
einem Handler auf, verketten die gewünschten Operationen und schließen
mit einer Abschlussmethode ab, die Ihnen Bytes, eine Response oder eine
gespeicherte Datei liefert.

```rust
use suprnova::{Image, OutputFormat, Response, handler};

#[handler]
pub async fn thumbnail() -> Response {
    Ok(Image::from_path("storage/photos/hero.jpg")
        .cover(320, 320)
        .to_format(OutputFormat::WebP)
        .quality(80)
        .to_response()
        .await?)
}
```

Dieser Handler dekodiert das JPEG, füllt eine 320x320-Box, schneidet den
Überhang aus der Mitte weg, kodiert WebP und liefert eine `200` mit
`Content-Type: image/webp` zurück.

Das Subsystem lebt in `suprnova::media`, hinter dem standardmäßig
aktiven `media`-Feature. Alles, wonach Sie üblicherweise greifen -
`Image`, `OutputFormat`, `ImageDriver`, `ImageConfig` -, ist flach an
der Crate-Wurzel re-exportiert, sodass `use suprnova::Image;` der
Import ist, den Sie wollen. Der Modulname ist mit Absicht dem Sinne nach
ein Plural: Dort werden auch die OxideAV-gestützten Audio- und
Video-Oberflächen leben.

Wenn Sie ein Upgrade durchführen, beachten Sie: Der Upload-Validator,
der früher `Image` hieß, heißt jetzt `ImageFile`, was den schlichten
Namen für diesen Pipeline-Typ frei macht. Das spiegelt Laravel, wo die
Validierungsregel `ImageFile` und der Manipulationstyp `Image` ist. Den
Validator finden Sie unter [Anfragen](requests.md).

## Die Pipeline ist lazy

Ein `Image` zu konstruieren liest nichts und dekodiert nichts.
Operationen zeichnen sich selbst auf; die Quelle wird erst geöffnet,
wenn eine Abschlussmethode läuft. Das hier ist also kostenlos:

```rust
use suprnova::Image;

let pipeline = Image::from_disk("uploads", "avatars/42.png").resize(64, 64);
```

Bis hierher hat nichts die Platte berührt. `Image` ist `Clone`, und ein
Klon führt die Pipeline erneut ab ihrer Quelle aus, statt ein Ergebnis
zu teilen.

Zwei Konstruktoren müssen eager sein und sagen das in ihrer
Dokumentation: `from_upload` (die temporäre Datei eines Uploads
überlebt die Anfrage nicht) und `from_stream` (ein Stream lässt sich
nur einmal konsumieren).

## Konstruktion

| Konstruktor | Quelle | Eager? |
|---|---|---|
| `Image::from_bytes(bytes)` | alles, was `Into<Bytes>` ist | nein |
| `Image::from_path(path)` | das Dateisystem | nein |
| `Image::from_disk(disk, path)` | eine `Storage`-Disk | nein |
| `Image::from_upload(&file).await?` | ein `UploadedFile` | ja |
| `Image::from_stream(stream).await?` | ein `Stream<Item = io::Result<Bytes>>` | ja |

`from_stream` erzwingt `IMAGE_MAX_ALLOC_BYTES` *während* des
Einsammelns, sodass ein endloser Stream abgeschnitten wird, statt erst
aufzufallen, wenn er den Speicher bereits gefüllt hat.

## Operationen

| Methode | Wirkung |
|---|---|
| `resize(w, h)` | Exakte Maße, das Seitenverhältnis wird ignoriert |
| `resize_width(w)` / `resize_height(h)` | Eine Dimension, die andere aus dem Seitenverhältnis abgeleitet |
| `scale(w, h)` | Passt in die Box, unter Wahrung des Seitenverhältnisses. **Vergrößert nie** |
| `scale_width(w)` / `scale_height(h)` | Verkleinert auf höchstens eine Dimension. Vergrößert nie |
| `crop(w, h, x, y)` | Schneidet ein Rechteck heraus. Fehler, wenn es außerhalb des Bildes liegt |
| `cover(w, h)` | Füllt die Box exakt und schneidet den Überhang aus der Mitte weg |
| `contain(w, h)` | Passt in die Box, unter Wahrung des Seitenverhältnisses. Kein Padding |
| `rotate(degrees)` | Dreht im Uhrzeigersinn um einen beliebigen Winkel und vergrößert dabei die Leinwand |
| `flip_vertically()` / `flip_horizontally()` | Laravels `flip` und `flop` |
| `blur(amount)` | Gaußsche Weichzeichnung, `0..=100`. `0` tut nichts |
| `sharpen(amount)` | Unscharfmaskierung, `0..=100`. `0` tut nichts. `50` ist die klassische Stärke |
| `grayscale()` | Entsättigt. Geschrieben wie bei Laravel |
| `to_format(format)` | Wählt den Ausgabecontainer |
| `quality(q)` | Kodierqualität, auf `1..=100` begrenzt, Standard `70` |

Werte, die Unsinn wären, werden begrenzt statt abgelehnt: `blur(500)`
merkt sich `100`, `quality(0)` merkt sich `1`. Ein Zuschnitt, der
außerhalb des Bildes liegt, ist ein echter Fehler und keine Begrenzung,
denn jemandem stillschweigend die Zuschnittbox zu verschieben ist
schlimmer, als es ihm zu sagen.

`rotate` nimmt beliebige Winkel. Ein Vielfaches von 90 Grad nimmt einen
exakten, achsenparallelen Weg ohne Resampling; alles andere ist
bilinear, und die Leinwand wächst, damit kein Pixel abgeschnitten wird.
Die freigelegten Ecken sind transparent, sofern das Ausgabeformat einen
Alphakanal hat.

## Abschlussmethoden

Jede Abschlussmethode ist `async`, verbraucht das `Image` und erledigt
das Dekodieren, Transformieren und Kodieren auf einem blockierenden
Thread, sodass sie nie die Runtime aufhält. Die I/O der Quelle findet
vor diesem Sprung statt, sodass eine langsame Platte nie einen
blockierenden Worker besetzt.

| Abschlussmethode | Liefert |
|---|---|
| `to_bytes()` | `Vec<u8>` der kodierten Datei |
| `to_response()` | Eine `HttpResponse` mit dem richtigen `Content-Type` |
| `save(path)` | Schreibt ins Dateisystem |
| `store(disk, path)` | Schreibt auf eine `Storage`-Disk |
| `dimensions()` | `(width, height)` des **verarbeiteten** Bildes |
| `mime_type()` | Der Medientyp des **verarbeiteten** Bildes |
| `dominant_color()` | Die Durchschnittsfarbe, als `#rrggbb` |

`dimensions()`, `mime_type()` und `dominant_color()` beschreiben alle
das fertige Bild, nicht die Quelle - derselbe Vertrag wie bei Laravel.
Nach dem MIME-Typ zu fragen führt trotzdem die Pipeline aus, denn einen
Typ für ein Bild zu melden, das sich gar nicht erzeugen lässt, ist eine
Lüge, die der Aufrufer erst später bemerken würde.

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

async fn describe() -> Result<(), FrameworkError> {
    let banner = Image::from_path("hero.png").resize(1200, 400);

    // Liest (1200, 400), nicht die Maße der Quelle.
    let (width, height) = banner.clone().dimensions().await?;
    println!("{width}x{height}");

    let accent = banner.to_format(OutputFormat::Jpeg).dominant_color().await?;
    println!("{accent}");

    Ok(())
}
```

## Formate

Fünf Formate werden heute gelesen und geschrieben: **PNG, JPEG, WebP,
GIF und BMP**.

| Format | Liest | Schreibt | Qualitätsregler |
|---|---|---|---|
| PNG | ja | ja | ignoriert (verlustfrei) |
| JPEG | ja | ja | wird beachtet |
| WebP | ja | ja (verlustfrei) | heute ohne Wirkung |
| GIF | ja | ja | ignoriert (Palette) |
| BMP | ja | ja | ignoriert (verlustfrei) |

AVIF wird noch weder gelesen noch geschrieben. Der hauseigene
AV1-Encoder, von dem es abhängt, ist noch nicht veröffentlicht, und ein
`OutputFormat::Avif` auszuliefern, das immer fehlschlägt, wäre ein
Versprechen, das das Framework nicht halten könnte. Es kommt mit dieser
Veröffentlichung, als neue Enum-Variante und sonst nichts.

Die GIF-Ausgabe wird vor dem Kodieren mit Floyd-Steinberg-Dithering auf
höchstens 256 Farben palettenquantisiert, sodass eine fotografische
Quelle sauber konvertiert, statt einen Fehler auszulösen.

WebP wird verlustfrei geschrieben, deshalb hat `quality()` derzeit
keine Wirkung auf die WebP-Ausgabe. Nehmen Sie JPEG, wenn Sie einen
Regler für Größe und Qualität brauchen.

## Speicherung

`from_disk` und `store` arbeiten gegen jede registrierte
`Storage`-Disk, sodass ein Round-Trip aus Verkleinern und
Zurückspeichern nie lokale Pfade berührt:

```rust
use suprnova::{FrameworkError, Image};

async fn make_web_copy() -> Result<(), FrameworkError> {
    Image::from_disk("uploads", "originals/42.png")
        .scale(1024, 1024)
        .store("uploads", "web/42.png")
        .await
}
```

Wie Sie Disks registrieren, steht unter
[Dateisystem & Speicher](filesystem.md).

## Dekodier-Limits

Beim Dekodieren richtet feindliche Eingabe ihren Schaden an: Ein paar
Kilobyte können eine Leinwand von 40000x40000 deklarieren und einen
Server auffordern, sechs Gigabyte dafür zu reservieren. Suprnova weist
das zurück, bevor überhaupt etwas reserviert wird.

| Variable | Standard | Zweck |
|---|---|---|
| `IMAGE_MAX_DIMENSION` | `16384` | Obergrenze für Breite und Höhe in Pixeln |
| `IMAGE_MAX_ALLOC_BYTES` | `268435456` (256 MiB) | Obergrenze für den dekodierten RGBA-Speicherbedarf und für die Größe der Quelldatei selbst |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | Echtzeit-Obergrenze für einen einzelnen ImageMagick-Aufruf (nur `magick`-Treiber) |

Das Framework parst den Header der Eingabe selbst - ein paar Dutzend
Bytes, keine Allokation -, liest die deklarierten Maße und weist
übergroße Eingabe zurück, bevor ein Decoder konstruiert wird. Dieselben
Obergrenzen gelten für die Ziele einer Größenänderung, denn
`resize(50_000, 50_000)` alloziert genauso viel, ob die Zahlen nun von
einem Angreifer oder aus einem Tippfehler stammen.

Ein erreichtes Limit ist ein 4xx-förmiger `FrameworkError::param`, denn
übergroße Eingabe ist ein Problem des Clients und kein Fehler des
Servers.

Konfiguration außerhalb des gültigen Bereichs wird mit einer Warnung
begrenzt, statt den Boot scheitern zu lassen: `IMAGE_MAX_DIMENSION=0`
würde jedes Bild in der Anwendung ablehnen, und das wollte niemand
konfigurieren.

### Eine Grenze ist nicht konfigurierbar

Ein WebP deklariert seine echte dekodierte Größe in seinem innersten
Bitstream-Chunk und nicht im Leinwand-Header, also durchläuft das
Framework den Container, um sie zu finden. Dieser Durchlauf hört nach
**4096 Chunks pro Ebene** auf und folgt der Verschachtelung **zwei
Ebenen tief**; eine Datei, die eines von beidem überschreitet, wird
rundweg abgelehnt statt vermessen.

Sie wird mit Absicht abgelehnt statt vermessen. Eine Zahl aus einem
Durchlauf zu melden, der das Dateiende nicht erreicht hat, wäre ein
Gate, um das ein ausreichend großer Haufen Füll-Chunks herumgehen
könnte; ein Durchlauf, der nicht zu Ende kommt, hat also keine Antwort
zu geben.

Keine der beiden Zahlen ist einstellbar, und keine
`IMAGE_MAX_*`-Variable wirkt auf sie - der Fehler sagt das auch so,
statt „konfiguriert“ zu sagen, gerade damit niemand einen Nachmittag
damit verbringt, `IMAGE_MAX_ALLOC_BYTES` anzuheben und zuzusehen, wie
sich nichts ändert. In der Praxis kommt nur eine absichtlich feindliche
Datei in ihre Nähe: Eine Animation mit 300 Frames geht bequem durch,
eine mit 4100 nicht.

## Backends

Wie bei Laravel besteht die Bild-Oberfläche aus zwei Treibern,
ausgewählt über `IMAGE_DRIVER`.

| Treiber | Wert | Braucht | Liest |
|---|---|---|---|
| OxideAV | `oxideav` (Standard) | nichts | PNG, JPEG, WebP, GIF, BMP |
| ImageMagick | `magick` | ImageMagick 7 auf dem Host | was immer die Delegates des Hosts hergeben |

### `IMAGE_DRIVER=oxideav`

Der Standard. Pures Rust, aufgebaut auf der Codec-Familie
[OxideAV](https://github.com/OxideAV): keine native Bibliothek, nichts
zu installieren, nichts zu konfigurieren. Für fast jede Anwendung ist
er die richtige Wahl, und eine gescaffoldete App bekommt genau ihn.

### `IMAGE_DRIVER=magick`

Opt-in. Führt eine auf dem Host installierte ImageMagick-7-Binary aus,
leitet das Bild über stdin hinein und liest das Ergebnis über stdout
zurück - ohne temporäre Dateien. Der Name der Binary kommt aus
`IMAGE_MAGICK_BINARY` und ist standardmäßig `magick`; eine fehlende
Binary ist ein klarer Fehler beim ersten Gebrauch und kein stiller
Rückfall.

Wählen Sie ihn, wenn Sie Eingabeformate brauchen, die der
Pure-Rust-Treiber nicht mitbringt - HEIC ist der häufige Fall. Der
Preis ist eine Host-Abhängigkeit: Der Betreiber installiert ImageMagick
und dessen Delegates und verantwortet deren Lizenzierung. Das Framework
linkt und kompiliert in beiden Fällen nichts Natives.

Argumente sind immer ein festes Array, das direkt an den Prozess
übergeben wird, nie ein Shell-String, und jedes numerische Argument
wird aus einem bereits validierten Feld formatiert. Es gibt keine
Argumentposition, die Benutzereingaben erreichen können.

Wenn das Framework die Eingabe erkennt, wird der Decoder auf der
Kommandozeile benannt - `png:-` statt eines bloßen `-`. Das ist
wichtig: Bei einem bloßen `-` sucht sich ImageMagick einen Coder aus
den übergebenen Bytes, sodass eine Datei, deren Magic MVG oder MSL
sagt, als *Skript* gelesen wird, ganz gleich, was Ihre Anwendung
anzunehmen glaubte. Den Coder festzunageln lässt eine falsch
ausgezeichnete Datei scheitern, statt sie zu etwas anderem werden zu
lassen.

**Eingabe, die das Framework nicht benennen kann, ist weiterhin auf
Ihre `policy.xml` angewiesen.** Genau diese Formate zu lesen ist der
ganze Grund, warum es diesen Treiber gibt, deshalb kann dieser Pfad
keinen Coder festnageln. Härten Sie die ImageMagick-Policy des Hosts -
mindestens durch Abschalten der Coder `MVG`, `MSL`, `URL`, `HTTPS`,
`EPHEMERAL` und `TEXT` -, wenn Sie unter `IMAGE_DRIVER=magick`
beliebige Uploads annehmen.

Unter diesem Treiber werden die Dekodier-Limits zweimal durchgesetzt.
Für die fünf Formate, die das Framework parsen kann, läuft die obige
Header-Prüfung, bevor der Prozess gestartet wird. Für alles andere ist
ein Parsen im Voraus unmöglich, deshalb trägt jeder Aufruf
ImageMagicks eigene `-limit`-Flags, abgeleitet aus derselben
Konfiguration, einschließlich eines `-limit time` in Echtzeit.

Dieses Flag ist nicht die ganze Geschichte, denn ImageMagick setzt es
mit seinem eigenen Ressourcenmonitor durch, und ein Prozess, der in
einem Delegate feststeckt, bevor dieser Monitor greift, löst es nie
aus. Deshalb hält Suprnova zusätzlich eine eigene Frist: Nach
`IMAGE_MAGICK_TIMEOUT_SECS` (plus ein paar Sekunden Kulanz, damit IMs
eigenes Limit zuerst greifen kann) tötet es die Prozessgruppe -
Delegates eingeschlossen, nicht nur den selbst gestarteten Prozess -
und hört auf, auf die Pipes zu warten. Ein hängendes Delegate kann
daher keinen Worker-Thread festsetzen. Delegates, die in der
Prozessgruppe bleiben, sterben mit ihr; eines, das die Gruppe verlässt,
oder ein Host ohne `kill`-Binary kann die Anfrage überleben - für
diesen Rest ist die Prozessüberwachung des Hosts da.

Ein Abschuss taucht als 5xx-`FrameworkError::internal` auf und nicht
als 4xx, obwohl eine Anfrage ihn ausgelöst hat. Irgendetwas hat den
Bildpfad so gründlich verkeilt, dass er abgeschossen werden musste, und
das gehört in die Überwachung der Serverfehler, wo ein Betreiber es zu
sehen bekommt - es als Client-Fehler einzustufen würde ausgerechnet die
eine Bedingung wegsortieren, für die sich ein Alarm lohnt.

## Eigene Treiber

`ImageDriver` ist der Erweiterungspunkt: `&[u8]` hinein, `Vec<u8>`
hinaus, kein Codec-Typ überquert die Grenze.

```rust
use suprnova::{FrameworkError, ImageDriver, ImagePipeline};

struct MyDriver;

impl ImageDriver for MyDriver {
    fn process(
        &self,
        contents: &[u8],
        pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, FrameworkError> {
        // `contents` dekodieren, `pipeline.transformations` erneut
        // abspielen, dann nach `pipeline.format` mit `pipeline.quality`
        // kodieren.
        todo!()
    }

    fn dimensions(&self, contents: &[u8]) -> Result<(u32, u32), FrameworkError> {
        todo!()
    }

    fn dominant_color(&self, contents: &[u8]) -> Result<String, FrameworkError> {
        todo!()
    }

    fn name(&self) -> &'static str {
        "mine"
    }
}
```

Installieren Sie ihn während `bootstrap()`, bevor das erste Bild
verarbeitet wird:

```rust
use suprnova::FrameworkError;

pub fn register() -> Result<(), FrameworkError> {
    suprnova::media::set_default_driver(Box::new(MyDriver))
}
```

Ein konformer Treiber setzt die konfigurierten `ImageConfig`-Limits
durch, bevor er für ein Dekodieren alloziert. Das Framework kann das
nicht stellvertretend für einen Treiber tun, denn es sieht den
dekodierten Puffer nie.

### Mehr Formate erreichen

Wenn die fünf eingebauten Formate nicht reichen, gibt es drei Wege,
grob danach geordnet, wie viel Sie sich damit aufladen:

1. **Der eingebaute `magick`-Treiber.** Setzen Sie
   `IMAGE_DRIVER=magick`. Die Formatbreite kommt von den
   ImageMagick-Delegates des Hosts, und es gibt keine
   Build-Abhängigkeit zu verwalten.
2. **Ein eigener Treiber um libvips**, zum Beispiel über das Crate
   [libvips-rust-bindings](https://github.com/olxgroup-oss/libvips-rust-bindings)
   (MIT). libvips ist die Maschine hinter Nodes `sharp`, mit einer sehr
   breiten Formatpalette - JPEG, JPEG XL, TIFF, PNG, WebP, HEIC, AVIF,
   PDF, SVG, GIF und mehr, dazu ImageMagick-Delegation - und starker
   Streaming-Leistung. Es bindet die C-Bibliothek libvips ein, Ihre App
   installiert libvips also zur Build- und zur Laufzeit und
   verantwortet diese Abhängigkeit; genau deshalb gehört sie hinter den
   Trait und nicht ins Framework. Ein praktischer Hinweis: Das
   `VipsImage` der Bindung ist nicht thread-sicher, worauf die
   Treiberform mit einem Bild pro `process()`-Aufruf ohnehin schon
   Rücksicht nimmt.
3. **Jedes CLI-Werkzeug**, umschlossen wie der `magick`-Treiber: ein
   festes Argument-Array an `std::process::Command`, Bildbytes über
   stdin hinein und über stdout hinaus, nie ein Shell-String.

Suprnova befürwortet die Trait-Grenze, nicht irgendeine bestimmte
Abhängigkeit dahinter. Was dort hinten sitzt, ist Ihre Entscheidung,
und ihre Lizenzierung ebenso.

## Testen

Das Subsystem braucht keine Fixtures auf der Platte - es ist seine
eigene Fixture-Fabrik, sobald Dekodieren und Kodieren einen Round-Trip
überstehen:

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

/// Ein 1x1-Fixture aus einem Byte-Literal auf die Größe wachsen lassen,
/// die ein Test gerade braucht.
async fn fixture(source: &[u8]) -> Result<Vec<u8>, FrameworkError> {
    Image::from_bytes(source.to_vec())
        .resize(4, 2)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
}
```

Tests, die die Dekodier-Limits verschärfen, müssen serialisiert werden:
Die Limits sind prozessglobal, ein parallel laufender Test würde also
unter der verschärften Obergrenze dekodieren.

### Warum Suprnova abweicht

**Kein HEIC im Standardtreiber, und der Grund sind Patente.** HEVC, der
Codec in HEIC, ist patentbelastet - unter anderem durch den
Access-Advance-Pool. Suprnova installiert keine nativen Bibliotheken,
ein eingebauter Decoder müsste also pures Rust sein und trüge dieses
Risiko unmittelbar; der eine glaubwürdige Pure-Rust-Decoder steht
zudem unter einer dualen AGPL-3.0-/kommerziellen Lizenz, und das ist
eine rechtliche Verpflichtung pro Anwendung und nichts, wozu ein
MIT-Framework irgendjemanden standardmäßig verpflichten darf.

Beide Frameworks machen HEIC zu einer Frage der Host-Bereitstellung;
Suprnovas Variante hat dabei nur ein bewegliches Teil weniger. Laravels
Standardtreiber GD kann HEIC überhaupt nicht lesen, und sein
Imagick-Pfad braucht das libheif-Delegate einkompiliert in **beides**,
die System-Binary von ImageMagick und die PHP-Erweiterung `imagick`. In
Suprnova liest der Standardtreiber kein HEIC, und `IMAGE_DRIVER=magick`
liest es, sobald das ImageMagick des Hosts das libheif-Delegate
mitbringt - ohne Erweiterungsschicht dazwischen. HEIC lässt sich also
heute schon einlesen: Installieren Sie ImageMagick mit libheif über
Ihren Paketmanager und legen Sie die Umgebungsvariable um. Die
Lizenzierung sitzt dort, wo sie hingehört, beim Host.

Wenn der `oxideav`-Treiber auf eine HEIC-Datei trifft, benennt er sie
beim Namen, verweist auf dieses Kapitel und nennt beide Wege nach vorn,
statt ein generisches „nicht unterstütztes Format“ zurückzugeben.

**AVIF steht noch aus, es wurde nicht übergangen.** Es ist
lizenzgebührenfrei und es ist die Antwort auf moderne Formate, die wir
wollen; der hauseigene AV1-Encoder ist schlicht noch nicht
veröffentlicht. WebP ist in der Zwischenzeit der Weg zum modernen
Format.

**Keine base64- oder URL-Konstruktoren.** Laravels `ImageManager` hat
`->read($base64)` und `->read($url)`. `from_bytes` lässt sich mit allem
kombinieren, was die Bytes erzeugt hat, auch mit dem
[HTTP-Client](http-client.md), und einen URL-Abruf aus dem
Bild-Subsystem herauszuhalten hält dessen Timeouts, Wiederholungen und
SSRF-Policy an einer Stelle statt an zweien.

**`from_stream` ist eager, mit einer Obergrenze.** Laravels Inhalte
sind ein Lazy-Closure. Ein Stream lässt sich nicht erneut abspielen,
deshalb wird dieser bei der Konstruktion geleert und zählt die Bytes
dabei gegen `IMAGE_MAX_ALLOC_BYTES`.

**`contain` füllt nicht auf.** Es passt das Bild in die Box ein und
hört dort auf; es setzt es nicht mit Balken auf einen Hintergrund.
Kombinieren Sie es selbst mit einem Hintergrund, wenn Sie einen
brauchen.

**Die Größenänderung nutzt bilineares Resampling.** Der Filtersatz des
Backends bringt Nearest-Neighbour und bilinear mit; bilinear ist sein
dokumentierter Standard für natürliche Bilder.

**Bilder sind nie serialisierbar.** Laravel wirft bei `__serialize`,
und Suprnova implementiert es schlicht nicht. Speichern Sie den Pfad
oder den Disk-Schlüssel und bauen Sie die Pipeline neu auf.

## Nächste Schritte

- [Dateisystem & Speicher](filesystem.md) für die Disks, aus denen `from_disk` liest und in die `store` schreibt.
- [Antworten](responses.md) dafür, was `to_response()` zurückreicht.
- [Umgebungsvariablen](env-vars.md) für die vollständige Liste der Bildeinstellungen.

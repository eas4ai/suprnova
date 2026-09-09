# RenderCache

RenderCache speichert eine nachweislich sichere Kopie der Antwort einer GET-
oder HEAD-Route und bedient die nächste passende Anfrage daraus, ohne Ihren
Handler überhaupt auszuführen. Sie nehmen Routen und Gruppen explizit auf;
alles andere funktioniert weiterhin genau wie bisher. Eine Route, die Sie nie
aufnehmen, bleibt unangetastet. Eine Route, die Sie aufnehmen, rendert und
bedient weiterhin korrekt, selbst wenn sich herausstellt, dass an dieser
konkreten Anfrage nichts sicher zu cachen ist - sie wird einfach nie
gespeichert, und Sie können herausfinden, warum.

Dieses Kapitel behandelt das Aktivieren des Cache, das Aufnehmen von Routen
und Gruppen, das Deklarieren von Varianz, das Lesen der Antwort-Header, die
es hinzufügt, die Gründe, aus denen ein Rendering abgelehnt wird, die
Betriebssteuerung und den Unterschied zu `suprnova::Cache`.

## Die Kapitel

Dies ist das erste von fünf. Lesen Sie sie beim ersten Mal der Reihe nach;
danach beantwortet jedes für sich eine Frage.

| Kapitel | Beantwortet |
|---|---|
| RenderCache (dieses hier) | Wie schalte ich es ein und nehme eine Route auf? |
| [Repräsentationen](render-cache-representations.md) | Was wird tatsächlich gespeichert, und unter welchem Schlüssel? |
| [Generationen](render-cache-generations.md) | Wann hört eine gespeicherte Kopie auf, aktuell zu sein? |
| [Bereitstellung](render-cache-deployment.md) | Wie teilen sich mehrere Knoten einen Cache? |
| [Betrieb](render-cache-operations.md) | Wie inspiziere, teste und messe ich es, und wie schalte ich es ab? |

## Den Cache aktivieren

Zwei Umgebungsvariablen sind für den Anfang wichtig:

- `RENDER_CACHE_ENABLED` - `true`, sofern nicht auf `false` oder `0` gesetzt.
  Ist sie deaktiviert, umgeht jede Anfrage RenderCache vollständig; es wird
  weder etwas nachgeschlagen noch etwas gespeichert.
- `RENDER_CACHE_L1_DIR` - standardmäßig nicht gesetzt, was bedeutet, dass es
  keine Ebene auf der Festplatte gibt. Setzen Sie sie auf ein Verzeichnis,
  das der Prozess anlegen und beschreiben kann, und gespeicherte
  Repräsentationen überstehen einen Prozessneustart in einer dateibasierten
  zweiten Ebene.

Eine Handvoll weiterer Variablen justiert die Standardwerte:
`RENDER_CACHE_L0_ENTRIES` (4.096) und `RENDER_CACHE_L0_BYTES` (128 MiB)
begrenzen die In-Process-Ebene; `RENDER_CACHE_L1_BYTES` (1 GiB) begrenzt die
Datei-Ebene; `RENDER_CACHE_FAILURE` (standardmäßig `open`, oder `closed`)
entscheidet, ob ein Store- oder Datenbankproblem die Route ungecacht bedient
oder die Anfrage verweigert; `APP_BUILD_ID` ordnet jeden gecachten Eintrag
dem Build zu, der ihn erzeugt hat. Setzen Sie sie ausdrücklich auf etwas,
das sich bei jedem Deployment ändert: Ihr Standardwert ist eine
einkompilierte Crate-Version, die das nicht tut. Siehe
[RenderCache Bereitstellung](render-cache-deployment.md).

`RENDER_CACHE_PROFILE` (standardmäßig `embedded`, oder `database` oder
`redis`) wählt, ob die zweite Ebene und der Neuaufbau-Koordinator in diesem
Prozess liegen oder mit jedem anderen Knoten geteilt werden. Ein geteiltes
Profil braucht zusätzlich eine Migration, die Ihre Anwendung auflistet.
Beides ist Thema des Kapitels
[Bereitstellung](render-cache-deployment.md), zusammen mit der vollständigen
Variablentabelle.

## Eine Route oder eine Gruppe aufnehmen

Nichts wird gecacht, bevor Sie es ausdrücklich festlegen.
`Router::try_render_cache` nimmt ein bereits registriertes Routenmuster auf;
`Router::try_render_cache_group` nimmt jede Route unter einem Pfadpräfix auf.
Beide erhalten eine mit `RenderCachePolicy::builder` erstellte Richtlinie:

```rust
use suprnova::{FrameworkError, Router};
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy,
};

fn add_render_cache(router: Router) -> Result<Router, FrameworkError> {
    router.try_render_cache_group(
        "/blog",
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
            .build()?,
    )
}
```

`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` legt
fest, wie lange eine Repräsentation frisch ist, und danach zwei Fenster, die
ab dieser Frische-Grenze gemessen werden: wie weit darüber hinaus die
gespeicherte Kopie noch ausgeliefert werden darf, während ein Neuaufbau im
Hintergrund läuft, und wie weit darüber hinaus die gespeicherte Kopie
ausgeliefert werden darf, wenn ein Neuaufbau im Vordergrund vollständig
fehlschlägt. Die beiden Fenster werden nicht gestapelt; siehe
[RenderCache Repräsentationen](render-cache-representations.md).

`RepresentationClass` reicht von der breitesten bis zur engsten gemeinsamen
Nutzung: `PublicShared` (eine Repräsentation für jeden, der der deklarierten
Varianz entspricht), `PublicShellStitched` (ein Live-Dokument, dessen
geteilte Shell einmal gespeichert wird und dessen Inseln für den jeweils
Anfragenden neu gemountet werden; siehe
[Repräsentationen](render-cache-representations.md)),
`PrivateCached` (eine Repräsentation pro angemeldetem Besucher oder Mandant)
und `Uncacheable`.

Ein Routenmuster muss bereits registriert sein, bevor Sie es aufnehmen, und
Sie müssen das Aufnehmen von Routen und Gruppen abschließen, **bevor** Sie
`RenderCache::install` (unten) aufrufen - der Installationsschritt liest,
was bis zu diesem Zeitpunkt registriert wurde.

Eine Richtlinie auf Routenebene kann statt einer vollständigen
`RenderCachePolicy` auch ein verengender Patch ihrer umschließenden Gruppe
sein, mit `PolicyPatch`: Sie erbt alles, was die Gruppe deklariert hat, und
darf es nur enger machen (ein kürzeres Frische-Fenster, eine strengere
Klasse), niemals weiter. Eine Route vollständig aus einer gecachten Gruppe
herauszunehmen, ist ein `PolicyPatch`, der die Klasse auf `Uncacheable`
setzt.

Schließen Sie die Verdrahtung von RenderCache mit einer Zeile ab, nach jeder
Middleware-Registrierung, die anfragegebundene Locale, Sitzung oder Identität
festlegt (RenderCache liest sie, um seinen Lookup-Schlüssel zu bilden, und
muss deshalb nach allem laufen, was sie einrichtet):

```rust
use suprnova::RenderCache;
use suprnova::render_cache::RenderCacheConfig;

Application::new()
    // ...
    .try_routes_async(|| async {
        let router = add_render_cache(routes::register())?;
        RenderCache::install(router, RenderCacheConfig::from_env()).await
    });
```

## Varianz deklarieren

Standardmäßig variiert eine gecachte Repräsentation nur nach Routenmuster,
Pfadparametern und dem Anwendungs-Build. Alles andere, wovon die Ausgabe
Ihres Handlers tatsächlich abhängt, muss deklariert werden, mit zwei
Mechanismen:

- **Query-Parameter.** `.query(QueryPolicy::declared(["page", "sort"]))`
  benennt die Query-Parameter, die Repräsentationen unterscheiden; jeder
  andere Query-Parameter, der bei einer Anfrage vorhanden ist, umgeht den
  Cache für diese Anfrage, statt stillschweigend ignoriert zu werden.
- **Varianzdimensionen**, einzeln hinzugefügt mit `.vary(dimension)`:
  - `VarianceDimension::Locale` partitioniert nach der ausgehandelten
    Locale.
  - `VarianceDimension::Media` partitioniert nach dem ausgehandelten
    Medientyp und fügt `Accept` zu `Vary` hinzu.
  - `VarianceDimension::Encoding` partitioniert nach der ausgehandelten
    Inhaltskodierung und fügt `Accept-Encoding` zu `Vary` hinzu.
  - `VarianceDimension::Host` partitioniert nach dem Host der Anfrage, dort
    wo Ihr Deployment mehr als einen Host sinnvoll macht.
  - `VarianceDimension::Tenant` partitioniert nach dem aktuellen Mandanten
    als opakes Schlüsselmaterial; eine Route, deren Handler jemals den
    Mandanten liest, muss dies deklarieren.
  - `VarianceDimension::Principal` partitioniert nach dem angemeldeten
    Besucher als opakes Schlüsselmaterial, gebunden an eine
    Berechtigungsversion (siehe „Epoche, Berechtigungen und Inspektion“
    unten); eine `PrivateCached`-Route muss `Principal` oder `Tenant` (oder
    beides) deklarieren, sonst lässt sie sich überhaupt nicht bauen.

`Media` und `Encoding` sind deklarierbar und gehen in den Schlüssel ein,
doch dieses Release löst beide jeweils zu einer Konstante auf: Jede Anfrage
ist `text/html` beziehungsweise `identity`. Sie zu deklarieren ist deshalb
ein Schritt zur Vorwärtskompatibilität - sie erweitern `Vary` korrekt und
reservieren den Schlüsselraum, sodass eine spätere Schicht für
Inhaltsaushandlung oder Kompression nicht mit Einträgen kollidieren kann,
die vor ihrem Bestehen veröffentlicht wurden - und nicht etwas, das den
Verkehr heute schon partitioniert.

`VarianceDimension::FeatureVersion`, `VarianceDimension::ConfigVersion` und
ein benutzerdefiniertes `VarianceDimension::Application(name)` existieren auf
dem Typ, haben in diesem Release aber keinen Resolver: Eine Route, die eines
davon deklariert, umgeht den Cache stillschweigend bei jeder Anfrage, statt
beim Bauen zu scheitern. Deklarieren Sie sie noch nicht.

## Die Antwort-Header lesen

Ein ausgelieferter Treffer trägt `ETag` (ein starker Validator, den Ihr
Client als `If-None-Match` für ein `304` zurücksenden kann),
`Cache-Control`, `Vary` und `Age` (ganze Sekunden seit der Veröffentlichung
der Repräsentation und das schnellste lokale Anzeichen dafür, dass eine
Antwort aus dem Store und nicht aus Ihrem Handler kam). Eine Antwort, die
über ihr Frische-Intervall hinaus ausgeliefert wird, trägt zusätzlich
`Warning: 110 - "Response is Stale"`. Alle fünf sind in
[RenderCache Repräsentationen](render-cache-representations.md) definiert,
zusammen mit den Werten, deren Versand für die Dogfood-Routen per Test
zugesichert ist.

## Warum ein Rendering nie gespeichert wird

Aufgenommen zu sein ist keine Garantie. Nach jedem Rendering laufen zwei
unabhängige Prüfungen, und jede von ihnen kann die Speicherung ablehnen,
ohne die Anfrage scheitern zu lassen - die Antwort, die Sie zurückbekommen,
ist in beiden Fällen identisch, sie wird nur nie zu einem Cache-Eintrag:

**Die Eignung** lehnt rundweg ab bei einer Antwort, die kein einfaches `200`
auf ein `GET` oder `HEAD` ist, die ihren Rumpf streamt, die ein Cookie
setzt, oder die einen Hop-by-Hop- oder Tracing-Header trägt. Das ist fast
immer unbeabsichtigt (eine Weiterleitung, eine Fehlerseite, eine Antwort,
die zufällig `Set-Cookie` berührt) und nichts, worum Sie herumdesignen
müssen.

**Die Klassifizierung** lehnt danach ab, was Ihr Handler während seiner
Ausführung tatsächlich getan hat, in Begriffen, die Sie wiedererkennen
werden:

- **Sie haben einen Sitzungswert gelesen.** Jedes Lesen der aktuellen
  Sitzung (über `session()`, `session_mut` oder ein Sitzungs-Cookie) zwingt
  das Rendering dauerhaft in die Klasse `Uncacheable`, unabhängig davon,
  welche Varianz die Route deklariert. Das Einzige, was dies *nicht*
  erfasst, ist die Identität des angemeldeten Besuchers selbst. `Auth::id()`
  liest sie aus der Sitzung, wenn nichts Früheres in der Anfrage sie
  aufgelöst hat, und dieses Lesen wird als Identitätslesen eingestuft, nicht
  als Sitzungslesen - eine gewöhnliche cookie-gestützte Anmeldung ist also
  genau das, wofür eine `PrivateCached`-Route mit deklarierter
  `Principal`-Varianz da ist, und der Griff nach der Id des Besuchers macht
  die Seite nicht stillschweigend cache-unfähig. Jeder andere Wert in der
  Sitzung tut das weiterhin. Zwei Konsequenzen sind wissenswert: Eine
  anonyme Anfrage an eine solche Route cacht unter dem Schlüssel
  `Anonymous`, weil das Rendering keine Identität aufgelöst und kein
  Principal-Material beobachtet hat und der Schlüssel genau das sagt - ein
  angemeldeter Besucher leitet einen `Private`-Schlüssel ab, der jenen
  Eintrag nie erreicht; und die eigene Kennung eines benannten Guards ist
  Principal-Material auf genau dieselbe Weise wie die des Standard-Guards.
- **Sie haben eine Identität gelesen, auf einer Route, die `Principal`
  nicht deklariert.** Das Lesen des angemeldeten Benutzers verengt die
  Klasse auf `PrivateCached`; enthält die deklarierte Varianz der Route
  `Principal` nicht, gibt es keine Möglichkeit, den Eintrag pro Besucher zu
  schlüsseln, daher wird er abgelehnt statt geteilt.
- **Sie haben eine Übersetzung ausgelöst (oder Ihre View-Engine hat es
  getan), ohne `Locale` zu deklarieren.** Jedes Lesen der ausgehandelten
  Locale braucht eine deklarierte `Locale`-Dimension, sonst wird das
  Rendering abgelehnt. Die Dokument-Shell jeder Inertia-Seite liest die
  Locale, um `<html lang>` zu setzen, unabhängig davon, ob die eigenen Daten
  der Seite überhaupt etwas mit Sprache zu tun haben - eine Inertia-Route
  muss also `Locale` deklarieren, um überhaupt jemals zu cachen, selbst eine
  ohne eigenen übersetzten Inhalt.
- **Sie haben eine Autorisierung geprüft.** Eine Entscheidung wird danach
  beurteilt, was ihre eigene Auswertung gelesen hat. Ein Gate, dessen
  Rumpf nur den Mandanten liest - etwa über
  `suprnova::live::current_tenant()` - klassifiziert allein unter `Tenant`
  und cacht auf einer Route, die nach `Tenant` geschlüsselt ist. Ein Gate,
  das einen benutzerbezogenen Fakt liest, oder das nichts liest, was
  RenderCache sehen kann, braucht weiterhin `Principal` deklariert: ein
  Rumpf, der über sein `user`-Argument ohne instrumentierten Zugriff
  entschieden hat, ist nicht von einem zu unterscheiden, der anhand einer
  Konstante entschieden hat, und die sichere Lesart davon ist die
  konservative.
- **Ein Modell hinter der Seite trägt einen globalen Scope, der
  anfragebezogenen Zustand liest.** Deklarieren Sie, wovon der Scope
  abhängt. Ein `GlobalScope`, der `ScopeDependency::Constant` zurückgibt,
  zeichnet nichts auf und kostet keine Cache-Treffer. Der Standardwert,
  `ScopeDependency::PerRequest`, verlangt, dass das `apply` des Scopes
  diesen Zustand über einen instrumentierten Zugriff liest -
  `suprnova::live::current_tenant()`, `Auth::id()`, `Lang::locale()`. Ein
  anfragebezogener Scope, dessen Auswertung keinen davon liest, verengt das
  Rendering auf `Uncacheable` und nennt sich selbst in der Ablehnung, sodass
  ein unsichtbarer Mandanten-Filter Sie den Cache kostet, statt Ihre
  Besucher gegenseitig deren Zeilen zu kosten.
- **Sie haben einen geheimen Konfigurationswert oder einen undeklarierten
  Anfragekontext gelesen.** Beides zwingt in `Uncacheable`. Dass eine
  Antwort von einem gewöhnlichen Anfrage-Header oder von `Config::get`
  abhängt, ist für RenderCache völlig unsichtbar - es kann nicht ablehnen,
  was es nicht sehen kann, daher liegt es an Ihnen, die passende Varianz zu
  deklarieren.
- **Sie haben rohes SQL über `DB::select`, `DB::select_one`, `DB::scalar`
  oder `DB::select_on` ausgeführt.** Das Framework kann die Tabellen, die
  eine rohe Anweisung liest, nicht benennen, daher wird das Rendering nie
  gespeichert; es wird trotzdem ausgeliefert. Lesezugriffe über
  `DB::table(..)` kennen ihre Tabelle und werden normal gecacht, ebenso
  `Auth::user()`, das über diesen Pfad aufgelöst wird.
  Die eigenen RBAC-Rollen- und Berechtigungsprüfungen des Frameworks nennen
  die fünf Tabellen, die sie lesen - `roles`, `permissions`,
  `role_permissions`, `model_roles` und `model_permissions` - daher wird
  eine gecachte Route, die eine davon auswertet, präzise beobachtet und
  normal gecacht.
- **Der Schreibzugriff erfolgte durch einen Queue-Worker, eine geplante
  Aufgabe oder einen Konsolenbefehl.** Dafür ist nichts Besonderes mehr
  nötig. Jeder Prozess, dessen Konfiguration RenderCache aktiviert und
  dessen Datenbank die RenderCache-Migration enthält, erhöht Generationen,
  sodass ein solcher Schreibzugriff genau das ungültig macht, was derselbe
  Schreibzugriff im Server ungültig macht, und
  `RenderCache::bump_permission_version()` funktioniert aus jedem von ihnen
  heraus. Ein Prozess mit `RENDER_CACHE_ENABLED=false`, oder einer, dessen
  Datenbank die Migration nicht enthält, erhöht nichts und gibt überhaupt
  kein RenderCache-SQL aus.

Auf PostgreSQL läuft das Rendering in einer `REPEATABLE READ`-Transaktion,
damit das, was es gelesen hat, und die Generationen, die es aufgezeichnet
hat, übereinstimmen; der Handler einer gecachten Route, der eine Zeile
aktualisiert, die eine andere Transaktion verändert hat, nachdem das
Rendering begonnen hat, erhält einen Serialisierungsfehler. Gestalten Sie
gecachte Routen als Lesepfade. Ein Handler, der innerhalb der
Render-Transaktion schreibt, erhöht weiterhin Generationen, konkurriert
dabei aber mit gleichzeitigen Schreibern um dieselben Zeilen und kann den
oben genannten Serialisierungsfehler erhalten.

Ein Schreibzugriff, der außerhalb jeder Transaktion erfolgt (`model.save()`
für sich allein), committet zuerst und erhöht seine Generationen in einer
unmittelbar folgenden Transaktion, sodass der Moment dazwischen „neue
Daten, alte Generation“ ist: ein zusätzlicher Neuaufbau, nie veralteter
Inhalt.

Nichts davon braucht spezielle Werkzeuge, um es in der Praxis zu beobachten:
Der verborgene Befehl `render-cache:inspect` (unten) zeigt, ob überhaupt ein
Eintrag für eine Route existiert, oder Sie probieren einfach zwei Anfragen
hintereinander aus und prüfen, ob die zweite einen `Age`-Header trägt.

## Eine Route, die cacht

Eine öffentliche Listenseite ohne besucherspezifischen Inhalt:

```rust
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    Ok(HttpResponse::html(render_post_list(&posts)))
}
```

registriert und aufgenommen:

```rust
use suprnova::{get, routes};
use suprnova::render_cache::{FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy};

routes! {
    get!("/blog", controllers::blog::index),
}

router.try_render_cache(
    "/blog",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
        .build()?,
)?;
```

`index` berührt nie die Sitzung, den angemeldeten Besucher oder die Locale,
daher rendert und veröffentlicht die erste Anfrage; jede Anfrage der
nächsten fünf Minuten wird aus dieser gespeicherten Kopie bedient, mit einem
`Age`-Header, einem `304` für einen Client, der sie bereits hat, und
`Cache-Control: public, max-age=300, s-maxage=300` für jedes CDN davor.

## Eine Route, die abgelehnt wird

Dieselbe Art von Seite, aber der Handler liest die Sitzung, um eine
Flash-Nachricht anzuzeigen:

```rust
use suprnova::session::session;
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    let flash = session().and_then(|s| s.get::<String>("status"));
    Ok(HttpResponse::html(render_post_list_with_flash(&posts, flash.as_deref())))
}
```

genau auf dieselbe Weise aufgenommen wie oben. Jede Anfrage rendert und
bedient weiterhin die korrekte Seite - Flash-Nachricht inklusive -, aber es
wird nie etwas gespeichert: Das Lesen der Sitzung verengt die Klasse auf
`Uncacheable`, bevor RenderCache überhaupt die Eignungsprüfung erreicht,
sodass eine zweite Anfrage für dieselbe URL wieder von Grund auf neu
rendert, statt mit einem `Age`-Header zurückzukommen. Die Abhilfe, falls
diese Seite cachen soll, besteht darin, im gecachten Pfad aufzuhören, die
Sitzung zu lesen (rendern Sie die Flash-Nachricht stattdessen aus einem
Query-Parameter oder einer separaten kleinen Antwort) - es gibt keine
Varianzdeklaration, die ein Sitzungslesen cachefähig macht, weil ein
Sitzungslesen bedeutet, dass die Antwort von etwas abhängt, wonach kein
Schlüssel sicher partitionieren könnte.

## Epoche, Berechtigungen und Inspektion

- **`RenderCache::bump_permission_version().await?`** - rufen Sie dies auf,
  wann immer eine Anwendungsaktion ändert, wozu ein angemeldeter Benutzer
  berechtigt ist (eine Rollenänderung, eine Berechtigungserteilung oder ein
  Berechtigungsentzug). Es erhöht eine persistierte Generation, die jedes
  principal-geschlüsselte Rendering beobachtet. Die Generation übersteht
  einen Neustart, und der Aufruf schließt sich der Transaktion an, in der
  die Rollenänderung läuft, sofern es eine gibt. Ohne diesen Aufruf passt
  ein Benutzer, dessen Berechtigungen sich gerade geändert haben, weiterhin
  zu allem, was unter seinem vorherigen Berechtigungssatz gecacht wurde.
- **`RenderCache::advance_epoch()`**, oder der verborgene Befehl
  `render-cache:epoch-advance` - eine Notfall-Invalidierung. Die Epoche ist
  direkt in den Lookup-Schlüssel selbst eingebacken, sodass ihr Vorrücken
  gespeicherte Einträge unerreichbar macht, ohne dass etwas aufzuzählen oder
  zu löschen wäre. Auf dem Prozess, der es ausführt, wirkt es sofort: Er
  gibt seine Epochen-Lease ab und leert im selben Moment seine
  In-Process-Ebene. Ein anderer Knoten zieht bei seinem nächsten
  Autoritätslesen nach, und seine dateibasierte Ebene behält ihre alten
  Dateien, bis ein Bereinigungslauf sie einsammelt - der automatische bei
  jeder 256. Veröffentlichung oder ein ausdrückliches
  `RenderCache::sweep()` -, was eine Frage der Datenträgerhygiene ist und
  keine Korrektheitsfrage. Greifen Sie darauf zurück, wenn mit gecachtem
  Inhalt etwas nicht stimmt und Sie nicht warten können, bis einzelne
  Einträge ablaufen; bei mehr als einem Knoten siehe
  [RenderCache Betrieb](render-cache-operations.md).
- **Der verborgene Befehl `render-cache:inspect <key>`** meldet die
  Metadaten eines gespeicherten Eintrags (nie seinen Rumpf) anhand des
  Schlüsseltexts, den Ihre Anwendung protokolliert oder den Ihre Telemetrie
  anzeigen kann, zusammen mit der aktuellen Epoche, sodass Sie erkennen
  können, ob das, was Sie sehen, noch gültige Autorität ist oder darunter
  bereits veraltet ist. Er schlägt den Schlüssel nur in der
  In-Process-Ebene des laufenden Prozesses nach, nie in der geteilten,
  sodass er auf einem `database`- oder `redis`-Profil für einen Schlüssel,
  den dieser Knoten nicht selbst ausgeliefert hat, keinen Eintrag meldet.

## RenderCache im Vergleich zu `suprnova::Cache`

`suprnova::Cache` ist ein Schlüssel-Wert-Speicher, den Sie explizit
aufrufen: Sie wählen den Schlüssel, Sie wählen, was gespeichert wird, Sie
wählen, wann es invalidiert wird (`Cache::put`, `Cache::get`,
`Cache::remember`, `Cache::forget`). Er funktioniert für alle Daten, von
denen Ihr Code entscheidet, dass sie das Cachen wert sind, auf jedem
Backend, das Sie konfigurieren (Memory oder Redis).

RenderCache ist kein universeller Speicher, und Sie rufen es nie aus Ihrem
Handler auf. Es cacht ganze HTTP-Antworten, der Schlüssel wird automatisch
aus der Route und ihrer deklarierten Varianz abgeleitet, und die
Invalidierung ist generationsbasiert: Ein gewöhnlicher
Datenbankschreibzugriff über den ORM oder den Query-Builder erhöht die
Generationen, von denen das Rendering abhing, und der Eintrag wird beim
nächsten Abruf neu berechnet, statt von Hand gelöscht zu werden; ein
Rendering, das rohes SQL gelesen hat, wird von vornherein nie gespeichert,
sodass es nichts neu zu berechnen gibt. Greifen Sie
zu `suprnova::Cache`, wenn Sie einen bestimmten Wert haben, den Sie einmal
berechnen und wiederverwenden möchten; greifen Sie zu RenderCache, wenn Sie
eine ganze Route haben, deren Antwort teuer zu rendern und sicher zu teilen
ist.

### Warum Suprnova abweicht

Laravel hat im Framework selbst kein Gegenstück. Response-Caching ist ein
Paket, das Sie hinzufügen; es umschließt die Route mit einer Middleware, die
die gerenderte Antwort unter einem von Ihnen zusammengesetzten Schlüssel
speichert, und alles danach liegt bei Ihnen: welche Routen sicher zu cachen
sind, was zwei Besucher unterscheidet und wann eine gespeicherte Seite
aufhört, wahr zu sein. Das Framework weiß nicht, dass eine Seite gecacht
wurde, und kann Ihnen deshalb nicht sagen, wann das Cachen einer Seite ein
Fehler war.

RenderCache ist genau aus diesem Grund Teil des Frameworks. Es sieht das
Rendering geschehen und kann deshalb aufzeichnen, was der Handler gelesen
hat, dies mit dem vergleichen, was die Route deklariert hat, und die
Speicherung einer Antwort verweigern, deren Sicherheit es nicht belegen
kann - stillschweigend, ohne zu ändern, was der Besucher ausgeliefert
bekommt. Eine Route aufzunehmen ist eine Deklaration, an der das Framework
Sie anschließend festhält, und kein Versprechen, das Sie sich selbst geben.
Der Preis ist, dass manche Routen, die Sie gern cachen würden, abgelehnt
werden und Sie herausfinden müssen, warum; der Nutzen ist, dass die
gespeicherten einmal, durch den Prozess, der sie gerendert hat, als sicher
speicherbar erwiesen wurden.

## Nächste Schritte

- [RenderCache Repräsentationen](render-cache-representations.md) - was
  tatsächlich gespeichert wird, unter welchem Schlüssel und in welchen
  Ebenen
- [RenderCache Generationen](render-cache-generations.md) - wie eine
  gespeicherte Kopie aufhört, aktuell zu sein
- [Cache](cache.md) - der explizite Schlüssel-Wert-Speicher, mit dem dieses
  Kapitel kontrastiert
- [Live](live.md) - die Dokumente, aus denen eine zusammengesetzte
  Repräsentation geschnitten wird

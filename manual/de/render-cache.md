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
oder die Anfrage verweigert; `APP_BUILD_ID` (standardmäßig die Version Ihrer
eigenen Crate) ordnet jeden gecachten Eintrag dem Build zu, der ihn erzeugt
hat, damit ein Deployment nie die Bytes eines alten Builds ausliefert.

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
fest, wie lange eine Repräsentation frisch ist, wie viel länger sie noch
ausgeliefert werden darf, während ein Neuaufbau im Hintergrund läuft, und wie
viel länger noch sie ausgeliefert werden darf, wenn dieser Neuaufbau
vollständig fehlschlägt. `RepresentationClass` reicht von der breitesten bis
zur engsten gemeinsamen Nutzung: `PublicShared` (eine Repräsentation für
jeden, der der deklarierten Varianz entspricht), `PublicShellStitched` (für
eine künftige zusammengesetzte Shell-Repräsentation reserviert, noch nicht
nutzbar), `PrivateCached` (eine Repräsentation pro angemeldetem Besucher
oder Mandant) und `Uncacheable`.

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
    Medientyp.
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

`VarianceDimension::FeatureVersion`, `VarianceDimension::ConfigVersion` und
ein benutzerdefiniertes `VarianceDimension::Application(name)` existieren auf
dem Typ, haben in diesem Release aber keinen Resolver: Eine Route, die eines
davon deklariert, umgeht den Cache stillschweigend bei jeder Anfrage, statt
beim Bauen zu scheitern. Deklarieren Sie sie noch nicht.

## Die Antwort-Header lesen

Ein ausgelieferter Treffer trägt `ETag` (ein starker Validator, den Ihr
Client als `If-None-Match` für ein `304` zurücksenden kann), `Cache-Control`
(`private`, sofern die Klasse nicht `PublicShared` ist und Sie eine
`SharedCachePolicy::SMaxAge` gesetzt haben, in welchem Fall es auch `public`
und `s-maxage` trägt), `Vary` (aus welchen deklarierten Dimensionen auch
immer eines implizieren - `Locale` impliziert `Accept-Language`, `Media`
impliziert `Accept`) und `Age` (ganze Sekunden seit der Veröffentlichung der
Repräsentation). Eine Antwort, die noch veraltet ausgeliefert werden darf,
trägt zusätzlich `Warning: 110 - "Response is Stale"`.

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
  welche Varianz die Route deklariert. Das greift auch, wenn die Identität
  eines anonymen Besuchers über den Sitzungs-Fallback aufgelöst wird - eine
  häufige Überraschung, denn der Besucher ist wirklich anonym und der
  resultierende Schlüssel ist korrekt `Anonymous`, aber das Lesen selbst ist
  trotzdem ein Sitzungslesen.
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
- **Sie haben eine Autorisierung geprüft.** `Gate` behandelt eine
  Entscheidung immer als besucherspezifisch, daher braucht es `Principal`
  deklariert, selbst auf einer Route, die nur nach `Tenant` geschlüsselt
  ist, solange die eigene Prüfung des Gate nicht nachweislich
  mandantenspezifisch ist. RenderCache kann den Unterschied von sich aus
  nicht erkennen.
- **Ein Modell hinter der Seite trägt einen mandantengebundenen globalen
  Scope.** Ein globaler Scope, der den aktuellen Mandanten aus seinem
  eigenen anfragelokalen Zustand liest, um eine Query zu filtern - das
  Muster, das die eigene `GlobalScope`-Dokumentation von Suprnova zeigt -
  ändert, was die Query zurückgibt, ohne dass RenderCache dieses Lesen je
  sieht. Deklarieren Sie `Tenant`-Varianz auf jeder Route, die auf einem
  solchen Modell aufbaut; nichts hier kann das Versäumnis für Sie abfangen.
- **Sie haben einen geheimen Konfigurationswert oder einen undeklarierten
  Anfragekontext gelesen.** Beides zwingt in `Uncacheable`. Dass eine
  Antwort von einem gewöhnlichen Anfrage-Header oder von `Config::get`
  abhängt, ist für RenderCache völlig unsichtbar - es kann nicht ablehnen,
  was es nicht sehen kann, daher liegt es an Ihnen, die passende Varianz zu
  deklarieren.

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

- **`RenderCache::bump_permission_version()`** - rufen Sie dies auf, wann
  immer eine Anwendungsaktion ändert, wozu ein angemeldeter Benutzer
  berechtigt ist (eine Rollenänderung, eine Berechtigungserteilung oder ein
  Berechtigungsentzug). Ohne dies passt ein Benutzer, dessen Berechtigungen
  sich gerade geändert haben, weiterhin zu allem, was unter seinem
  vorherigen Berechtigungssatz gecacht wurde.
- **`RenderCache::advance_epoch()`**, oder der verborgene Befehl
  `render-cache:epoch-advance` - eine Notfall-Invalidierung. Jeder aktuell
  gespeicherte Eintrag wird bei seiner allernächsten Anfrage sofort über das
  gewöhnliche Lookup unerreichbar, weil die Epoche direkt in den
  Lookup-Schlüssel selbst eingebacken ist. Die In-Process-Ebene wird im
  selben Moment ebenfalls vollständig geleert; eine dateibasierte Ebene
  behält ihre alten Dateien auf der Festplatte, bis der regelmäßige oder
  manuelle Bereinigungslauf sie einsammelt, was eine Frage der
  Datenträgerhygiene ist und keine Korrektheitsfrage. Greifen Sie darauf
  zurück, wenn mit gecachtem Inhalt etwas nicht stimmt und Sie nicht warten
  können, bis einzelne Einträge ablaufen.
- **Der verborgene Befehl `render-cache:inspect <key>`** meldet die
  Metadaten eines gespeicherten Eintrags (nie seinen Rumpf) anhand des
  Schlüsseltexts, den Ihre Anwendung protokolliert oder den Ihre Telemetrie
  anzeigen kann, zusammen mit der aktuellen Epoche, sodass Sie erkennen
  können, ob das, was Sie sehen, noch gültige Autorität ist oder darunter
  bereits veraltet ist.

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
nächsten Abruf neu berechnet, statt von Hand gelöscht zu werden. Greifen Sie
zu `suprnova::Cache`, wenn Sie einen bestimmten Wert haben, den Sie einmal
berechnen und wiederverwenden möchten; greifen Sie zu RenderCache, wenn Sie
eine ganze Route haben, deren Antwort teuer zu rendern und sicher zu teilen
ist.

# RenderCache Repräsentationen

Eine gecachte Route speichert nicht „eine Seite“. Sie speichert eine
**Repräsentation**: eine konkrete Antwort, unter einem Lookup-Schlüssel, in
einer oder mehreren Speicherebenen, mit genug Metadaten daneben, um eine
bedingte Anfrage zu beantworten und später zu belegen, dass sie noch aktuell
ist. Zwei Besucher bekommen nur dann dieselben gespeicherten Bytes, wenn der
Schlüssel, den sie ableiten, derselbe Schlüssel ist, und der Schlüssel wird
aus dem abgeleitet, was die Route deklariert hat - nie daraus, was der
Handler zufällig getan hat.

In diesem Kapitel geht es um dieses gespeicherte Etwas. Welche Formen eine
Repräsentation annehmen kann (`Complete` und `Composite`), was in ihren
Schlüssel eingeht, in welche Ebenen sie geschrieben wird, welche `ETag`-,
`Cache-Control`-, `Vary`-, `Age`- und `Warning`-Angaben ein ausgelieferter
Treffer trägt, in welchen vier Frischezuständen sie sein kann, wie sie
`If-None-Match` und `HEAD` beantwortet und was `PrivateCached` und
`PublicShellStitched` tatsächlich speichern. *Warum* eine Repräsentation das
Frische-Band verlässt - ein Schreibzugriff, ein Vorrücken der Epoche - ist
Thema des nächsten Kapitels; hier genügt es, dass es die Bänder gibt und
dass eine Repräsentation in einem von ihnen sitzt. Jedes Beispiel unten ist
eine Route in der Dogfood-Anwendung dieses Repositorys
(`app/src/live/mod.rs`) und wird durch einen benannten Test in
`app/tests/live_render_cache.rs` nachgewiesen.

## Zwei Formen von Einträgen

Ein gespeicherter Eintrag ist von einer von zwei Arten.

- **`Complete`** ist eine fertige Antwort: ein Status, ein Satz
  wiederabspielbarer Header und ein Rumpfpuffer. Ihn auszuliefern kopiert
  nichts und führt nichts aus. Jede `PublicShared`- und
  `PrivateCached`-Route speichert diese Form.
- **`Composite`** ist eine geteilte **Shell** mit typisierten Löchern darin,
  zusammen mit einem Segmentgraphen, der sagt, was in jedes Loch
  zurückgeht. Nur `RepresentationClass::PublicShellStitched` speichert diese
  Form, und nur ein Live-Dokument erzeugt eine.

Die Klasse, die Sie in der Richtlinie deklarieren, entscheidet, welche Form
überhaupt erreichbar ist. `/live/public` und `/live/todos` deklarieren beide
`PublicShared`; `the_database_profile_serves_a_hit_through_the_sql_stores`
liest den veröffentlichten Eintrag von `/live/todos` wieder aus dem Store
heraus und sichert zu, dass es ein `EntryKind::Complete` ist, und
`the_public_document_is_a_hit_whose_seed_still_promotes` liest den von
`/live/public` über `RenderCache::inspect_route_for_test` zurück und sichert
die Klasse zu, unter der er gespeichert wurde. Das ist wichtig, denn „er
wurde gespeichert“ und „er wurde stillschweigend abgelehnt“ erzeugen
dieselbe Antwort: Die Behauptung muss gegen den Eintrag erhoben werden, nicht
gegen das, was der Besucher sieht.

## Der Lookup-Schlüssel

Der Schlüssel, den eine Anfrage ableitet, wird gebildet aus dem Routenmuster,
seinen Pfadparametern, den Query-Parametern, die die Richtlinie deklariert
hat, dem aufgelösten Wert jeder deklarierten Varianzdimension, der
Build-Id der Anwendung (`APP_BUILD_ID`) und der aktuellen Autoritätsepoche.
Sonst nichts. Ein Query-Parameter, der mit der Anfrage eintrifft, aber nicht
von `QueryPolicy::declared` benannt wird, umgeht den Cache für diese Anfrage,
statt still aus dem Schlüssel zu fallen, denn ihn fallen zu lassen würde dem
Absender die falsche Seite ausliefern.

Der Schlüssel ist Text, den ein Betreiber in der Hand halten kann:
`RenderCache::key_for_route_for_test` in
`the_operator_commands_inspect_without_a_body_and_advance_the_epoch` sichert
zu, dass er mit `rk1.` beginnt, und `render-cache:inspect` nimmt genau
diesen Text entgegen.

Weil die Epoche Teil des Schlüssels ist, muss ein Vorrücken der Epoche nichts
finden und nichts löschen. Jeder zuvor gespeicherte Eintrag hört bei der
nächsten Anfrage einfach auf, über das gewöhnliche Lookup erreichbar zu
sein. Auf diesem Mechanismus beruht die Notfall-Invalidierung des Kapitels
[Betrieb](render-cache-operations.md).

## In welche Ebenen eine Richtlinie schreibt

Es gibt zwei Speicherebenen. **L0** ist Speicher im Prozess, begrenzt durch
`RENDER_CACHE_L0_ENTRIES` und `RENDER_CACHE_L0_BYTES`. **L1** ist das, was
das Bereitstellungsprofil konfiguriert - ein Verzeichnis mit Dateien, eine
Datenbanktabelle oder Redis - und wird von jedem Prozess geteilt, der darauf
zeigt.

Der Richtlinien-Builder speichert **nur in L0**, sofern Sie nichts anderes
sagen: `StorageLayers::l0_only()` ist der Standardwert. Eine Route, die es
wert ist, in die geteilte Ebene zu kommen, deklariert das:

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, StorageLayers,
};

let router = router.try_render_cache(
    "/live/todos",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .layers(StorageLayers::l0_and_l1())
        .build()?,
)?;
```

Das ist die Deklaration von `/live/todos` aus `app/src/live/mod.rs`. Es ist
das eine Dokument in jener Anwendung, dessen Bytes sich jeder Knoten teilen
kann, also ist es dasjenige, das `l0_and_l1()` deklariert. Unter dem
eingebetteten Profil, wo L1 deaktiviert ist, solange nicht
`RENDER_CACHE_L1_DIR` ein Verzeichnis benennt, ändert das Deklarieren der
Ebene nichts; unter dem Datenbankprofil landet der Eintrag in
`suprnova_render_entries`, und ein zweiter Prozess findet ihn dort.

`the_database_profile_serves_a_hit_through_the_sql_stores` ist der Nachweis.
Der Test startet die Anwendung auf den Providern des Datenbankprofils, liest
den veröffentlichten Eintrag direkt aus L1 unter genau dem Schlüssel, den die
Middleware abgeleitet hat, leert dann L0 und fragt erneut - und die zweite
Anfrage wird immer noch beantwortet, ohne dass der Handler läuft. Ein
Treffer aus dem Arbeitsspeicher sähe von der Client-Seite aus identisch aus,
und genau deshalb greift der Test in den Store.

Wählen Sie die Ebenen pro Route statt global. L1 kostet bei einem Fehltreffer
einen Roundtrip, den L0 allein nicht kostet, und ein Eintrag, nach dem immer
nur ein Knoten fragen wird, ist es nicht wert, dorthin gelegt zu werden, wo
jeder Knoten ihn sehen kann.

## Die Metadaten, die ein ausgelieferter Treffer trägt

Fünf Antwortfelder beschreiben eine ausgelieferte Repräsentation, und hier
werden sie definiert; die anderen Kapitel verwenden sie, ohne sie erneut
darzulegen.

| Feld | Was es aussagt |
|---|---|
| `ETag` | Ein starker Validator über genau die gesendeten Bytes. Ein Client darf ihn als `If-None-Match` zurücksenden. |
| `Cache-Control` | Standardmäßig `private` für jede Klasse. Eine `PublicShared`-Route, die `SharedCachePolicy::SMaxAge` setzt, erhält zusätzlich `public` und `s-maxage`, und nur so wird ein geteilter Proxy je eingeladen, die Bytes zu behalten. Ein `Composite`-Dokument mit mindestens einer Insel ist `private, no-store`, gleichgültig ob es bei einem Treffer zusammengesetzt oder von dem Render erzeugt wurde, der die Shell veröffentlicht hat. |
| `Vary` | Abgeleitet aus den deklarierten Varianzdimensionen, die einen Anfrage-Header implizieren: `Locale` impliziert `Accept-Language`, `Media` impliziert `Accept`, `Encoding` impliziert `Accept-Encoding`. Eine Dimension, die keinen impliziert, fügt nichts hinzu. Die Namen werden nach Header-Namen sortiert ausgegeben, nicht in der Reihenfolge, in der Sie die Dimensionen deklariert haben. |
| `Age` | Ganze Sekunden seit der Veröffentlichung der Repräsentation. Sein Vorhandensein ist der einfachste lokale Nachweis dafür, dass eine Antwort aus dem Store kam. |
| `Warning` | `110 - "Response is Stale"`, und nur auf einer Antwort, die über ihr Frische-Intervall hinaus ausgeliefert wird. |

Die Abbildung von Dimension auf Header ist `VarianceDimension::vary_header`
in `crates/suprnova-live/src/render_cache/variance.rs`. Zwei Engine-Tests
weisen die `Locale`- und die `Encoding`-Hälfte davon sowie den
zusammengesetzten Header-Wert nach:
`a_descriptor_orders_dimensions_and_bounds_values`
(`crates/suprnova-live/tests/render_cache_variance.rs`) sichert zu, dass ein
Deskriptor, der beide trägt, `["Accept-Encoding", "Accept-Language"]` meldet,
und `cache_control_and_vary_agree_with_class_variance_and_seed_deadline`
(`crates/suprnova-live/tests/render_cache_coherence.rs`) sichert zu, dass
dasselbe Paar `Accept-Encoding, Accept-Language` ausgibt und dass ein
Deskriptor ohne header-implizierende Dimension überhaupt kein `Vary`
ausgibt. Dass `Media` ein `Accept` impliziert, ist aus dem Code
dokumentiert; kein Test hier stellt dieses Paar her.

Drei der Antwortwerte werden gegen die laufende Anwendung zugesichert:
`the_public_document_is_a_hit_whose_seed_still_promotes` liest
`private, max-age=300` von `/live/public` und verlangt einen `Age`-Header
auf der zweiten Anfrage;
`the_private_document_is_cached_per_principal_and_never_crosses` liest
`private, max-age=60` von `/live/me`;
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` liest
`private, no-store` vom Dashboard, und zwar bei dem Render, der dessen Shell
veröffentlicht, ebenso wie bei dem zusammengesetzten Treffer danach, denn
dieser Wert richtet sich danach, was die Bytes enthalten, und nicht danach,
welcher Pfad sie erzeugt hat.

## Die vier Frischezustände

Jeder Treffer löst sich zu genau einem von vier Zuständen auf, bevor
irgendetwas ausgeliefert wird.
`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` legt
sie fest. **Beide Veraltet-Fenster werden vom Ende des Frische-Intervalls an
gemessen, nicht eines nach dem anderen gestapelt** - das ist das Detail,
über das man stolpert:

| Zustand | Alter seit Veröffentlichung | Was der Besucher bekommt |
|---|---|---|
| Frisch | unter `fresh_ms` | die gespeicherten Bytes, kein `Warning` |
| Veraltet, auslieferbar | über `fresh_ms` hinaus um weniger als `stale_servable_ms` | die gespeicherten Bytes sofort, unter `Warning`, mit einem begrenzten Neuaufbau hinter der Anfrage |
| Veraltet bei Fehler | über `fresh_ms` hinaus um mindestens `stale_servable_ms` und um weniger als `stale_on_error_ms` | ein Neuaufbau im Vordergrund; die gespeicherten Bytes unter `Warning` nur dann, wenn dieser Neuaufbau selbst fehlschlägt |
| Tot | über `fresh_ms` hinaus um das größere der beiden Fenster oder mehr | nichts; die Anfrage rendert |

`/live/todos` deklariert `FreshnessPolicy::new(300_000, 60_000, 300_000)`,
ist also fünf Minuten lang frisch, in der sechsten veraltet und
auslieferbar, bis zehn Minuten veraltet bei Fehler und danach tot.

Zwei Regeln überstimmen die Bänder. Eine
`PrivateCached`-Repräsentation wird **nie** veraltet ausgeliefert: Über ihr
Frische-Intervall hinaus ist sie tot, und deshalb deklariert `/live/me`
`FreshnessPolicy::new(60_000, 0, 0)` - ein Veraltet-Band läse sich dort als
ein Versprechen, das der Cache nicht hält. Und ein gespeichertes Dokument
mit öffentlichem Seed, dessen Beförderungsfrist verstrichen ist, ist tot,
was auch immer seine Intervalle sagen, denn ein Seed jenseits seiner Frist
kann nie wieder befördert werden.

`stale_service_is_marked_and_rebuilt_in_the_background` treibt `/live/todos`
auf einer kontrollierten Uhr über die erste Grenze und sichert den
ausgelieferten Rumpf, `Warning: 110 - "Response is Stale"` und `Age: 300`
zu. Was eine Repräsentation *veranlasst*, das Frische-Band früh zu
verlassen - ein Schreibzugriff, ein Vorrücken der Epoche -, ist Thema von
[RenderCache Generationen](render-cache-generations.md).

## Bedingte Anfragen und HEAD

Ein Client, der einen ausgelieferten `ETag` als `If-None-Match` zurücksendet,
erhält ein `304` ohne Rumpf, und ein `HEAD` erhält die Header ohne Rumpf.
Keines von beiden erreicht Ihren Handler:

```
GET  /live/todos                          -> 200, ETag: "..."
GET  /live/todos  If-None-Match: "..."    -> 304, empty body
HEAD /live/todos                          -> 200, same ETag, empty body
```

`conditional_and_head_requests_are_answered_from_the_stored_entry` sichert
alle drei gegen die laufende Anwendung zu, einschließlich dessen, dass sich
der Render-Zähler über die letzten beiden hinweg nicht bewegt.

Eine Ausnahme, und sie ist beabsichtigt: Eine `Composite`-Antwort antwortet
nie mit `304`. Jede Zusammensetzung ist eine eigene Repräsentation - frische
Inselidentitäten, eine frische Bootstrap-Nonce, wo das Dokument eine hat -,
sodass ein `304` dem Client sagen würde, er solle den Rumpf, den er bereits
hat, mit Headern paaren, die für diese Anfrage geprägt wurden. Der `ETag`
auf einer zusammengesetzten Antwort ist weiterhin stark über genau die
gesendeten Bytes; er passt nur nie zu einer späteren Anfrage. Schritt 7 von
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` sendet einen
ausgelieferten Validator direkt zurück und sichert ein `200` mit einem
anderen `ETag` zu.

## Eine Repräsentation, die einer Person gehört

`RepresentationClass::PrivateCached` speichert eine Repräsentation pro
angemeldetem Besucher. Sie wird zur Bauzeit verweigert, solange die
Richtlinie nicht zusätzlich `Principal`- oder `Tenant`-Varianz deklariert,
damit das Paar nicht versehentlich auseinanderdriften kann:

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, VarianceDimension,
};

let router = router.try_render_cache(
    "/live/me",
    RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0)?)
        .vary(VarianceDimension::Principal)
        .build()?,
)?;
```

Der Handler dahinter ist ein gewöhnlicher. Er löst den angemeldeten Besucher
auf und rendert dessen Namen:

```rust
pub async fn me(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let user = Auth::user_as::<User>()
            .await?
            .ok_or_else(|| FrameworkError::internal("Live account document without a principal"))?;
        html(&MeView { name: user.name })
    }
    .await;
    result.map_err(failed)
}
```

Es ist nichts zusätzlich verdrahtet, damit das cacht. Die Route trägt
dieselbe `AuthMiddleware::redirect_to("/login")` wie das Dashboard, sodass
ein anonymer Besucher umgeleitet wird, bevor der Handler läuft, und der
Principal selbst wird innerhalb des Renderings aufgelöst. Das Lesen der
Identität des angemeldeten Besuchers aus der Sitzung wird als
**Identitätslesen** eingestuft, nicht als Sitzungslesen, also verengt sich
das Rendering auf `PrivateCached`, der Schlüssel trägt opakes Material pro
Principal, und beide stimmen überein.

`the_private_document_is_cached_per_principal_and_never_crosses` meldet zwei
Besucher an, ruft je zweimal ohne Rendering ab und sichert zu, dass jeder
Rumpf die eigene Person nennt und nicht die andere; ein dritter Besucher
rendert, weil er mit keinem der beiden etwas teilt. Das ausgelieferte
`Cache-Control` ist `private, max-age=60`, sodass keinem geteilten Proxy je
die Bytes angeboten werden. Derselbe Test zeigt die andere Hälfte des
Handels: Das Rendering löst seinen Principal über den Provider auf, der die
Tabelle `users` liest, sodass das Seeden eines dritten Besuchers jeden
gespeicherten `/live/me`-Eintrag invalidiert und die nächste Anfrage für
jeden davon neu aufbaut. Das ist tabellengranulare Invalidierung, die genau
das tut, was [Generationen](render-cache-generations.md) beschreibt.

Zwei Konsequenzen dieser Einstufung sind wissenswert, bevor Sie die Klasse
deklarieren:

- Eine **anonyme** Anfrage an eine `PrivateCached`-Route mit
  `Principal`-Varianz cacht unter dem Schlüssel `Anonymous`. Das Rendering
  hat keine Identität aufgelöst, also wurde kein Principal-Material
  beobachtet, der Schlüssel sagt `Anonymous`, und beides stimmt überein. Ein
  angemeldeter Besucher leitet einen `Private`-Schlüssel ab, der jenen
  Eintrag nie erreichen kann. Das gilt, wenn eine solche Anfrage tatsächlich
  ein `200` rendert, was `/live/me` nie tut: Seine Anmeldeumleitung
  antwortet mit einem `302`, und ein `302` wird schon von der
  Eignungsprüfung abgelehnt, bevor irgendetwas davon herangezogen wird. Der
  Framework-Test, der den Fall wirklich erreicht, ist
  `an_anonymous_render_resolving_identity_through_the_session_caches_anonymously`
  in `framework/tests/render_cache/middleware.rs`.
- Die Kennung eines **benannten Guards** ist Principal-Material auf genau
  dieselbe Weise wie die des Standard-Guards. Sie zu lesen zeichnet ein
  Principal-Lesen auf und, wenn es eine Id gibt, den Wert.

Und eine Regel, die sich nicht geändert hat: Eine Route, die den Principal
liest, *ohne* `Principal`-Varianz zu deklarieren, wird von der Speicherung
abgelehnt. Es gibt keine Möglichkeit, einen solchen Eintrag pro Besucher zu
schlüsseln, also wird er nie gespeichert, statt geteilt zu werden. Jeder
andere Sitzungswert erzwingt weiterhin `Uncacheable`; siehe die
Klassifizierungsliste in [RenderCache](render-cache.md).

## Eine Shell mit Löchern darin

`RepresentationClass::PublicShellStitched` ist für ein Live-Dokument gedacht,
dessen Rahmen für alle gleich ist und dessen Inseln es nicht sind. Der
gespeicherte Eintrag hält allein die Shell. Weder das Markup einer
identitätsgebundenen Insel noch ein signierter Snapshot liegt je in den
gespeicherten Bytes; jeder Treffer mountet jede Insel für den jeweils
Anfragenden neu, unter Autorität, die für diese Anfrage abgeleitet wurde.

Das Dashboard dieses Repositorys ist genau diese Route:

```rust
router.try_render_cache(
    "/live",
    RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .build()?,
)
```

`the_dashboard_is_stitched_per_principal_from_one_shared_shell` sichert zu,
was das einbringt und was es kostet. Der gespeicherte Eintrag ist ein
`EntryKind::Composite` mit drei Slots, einem pro identitätsgebundener Insel.
Ein zweiter Principal wird aus dieser Shell bedient, und die beiden Dokumente
unterscheiden sich **nur** in ihren Insel-Tags: Der Test entfernt die drei
Insel-Tags aus beiden und vergleicht Byte für Byte, was übrig bleibt. Die
eigene Anmeldeumleitung der Route läuft weiterhin bei jedem Treffer: Ein
zusammengesetzter Treffer wird durch die gesamte Middleware-Kette der Route
geleitet, bevor irgendetwas ausgeliefert wird, sodass ein anonymer Besucher
die Umleitung bekommt und nie ein zusammengesetztes Dokument.

Ein zusammengesetztes Dokument mit mindestens einem Slot wird mit
`Cache-Control: private, no-store` gesendet - bei dem Render, der die Shell
veröffentlicht, ebenso wie bei jeder Zusammensetzung danach. Es hält die
Inseln eines Principals unter Autorität, die für eine Anfrage neu abgeleitet
wurde, und ein `max-age` ließe ein geteiltes Browserprofil sie demjenigen
wiedergeben, der sich als Nächstes davorsetzt; welcher Pfad die Bytes erzeugt
hat, ändert nichts daran, was in ihnen steht. Ein `Composite` ohne Slots trägt
überhaupt keine Bytes pro Principal, nur eine Nonce pro Anfrage, und behält
deshalb das private `max-age` der Klasse wie jede andere private
Repräsentation;
`a_zero_slot_composite_is_assembled_with_a_fresh_nonce_on_every_hit` in
`framework/tests/render_cache/stitch.rs` sichert das zu. So oder so
verweigert die Klasse `SharedCachePolicy::SMaxAge` beim Bauen der
Richtlinie, sodass keinem geteilten Proxy je die Bytes angeboten werden.

Zwei Grenzen sind zu kennen: Die Klasse ist nur auf einer Route sinnvoll,
deren Kette in der Live-Abschluss-Middleware endet, verwenden Sie sie also
mit `LiveDocument::render` und mit nichts sonst, und ein zusammengesetzter
Eintrag wird nie vom Veraltet-bei-Fehler-Rückfall ausgeliefert und löst nie
einen Neuaufbau im Hintergrund aus. Das Kapitel
[Generationen](render-cache-generations.md) sagt, was das Zweite in der
Praxis bedeutet.

### Warum Suprnova abweicht

Laravel hat überhaupt kein serverseitiges Repräsentationsmodell. Seine
Response-Caching-Pakete speichern die gerenderte Ausgabe einer Route unter
einem Schlüssel, den Sie selbst zusammensetzen - typischerweise die URL,
manchmal die URL plus ein handgeschriebenes Suffix für den angemeldeten
Benutzer -, und geben sie bei der nächsten Anfrage zurück. Es gibt genau eine
Form von gespeichertem Ding, es ist immer ein fertiger Rumpf, und ob zwei
Besucher es sich teilen, ist eine Eigenschaft der Zeichenkette, die Sie
gebaut haben.

Suprnova macht den Schlüssel zu einer Deklaration und die Form zu einer
Folge davon. Sie benennen die Klasse und die Varianzdimensionen; das
Framework leitet den Schlüssel ab, verweigert `PrivateCached` ohne eine
partitionierende Dimension, vergleicht, was das Rendering tatsächlich
beobachtet hat, mit dem, was der Schlüssel tatsächlich gesagt hat, und lehnt
es ab, das Rendering zu speichern, wenn beides nicht übereinstimmt. Und weil
es `PublicShellStitched` gibt, muss sich eine Seite, die zu 95 Prozent
geteilt und zu 5 Prozent privat ist, nicht entscheiden, ob sie nichts cacht
oder etwas cacht, was sie nicht sollte: Der geteilte Teil wird einmal
gespeichert und der private Teil pro Anfrage neu gerendert, wobei die
privaten Bytes nie in den Store gelangen.

## Nächste Schritte

- [RenderCache Generationen](render-cache-generations.md) - wie eine
  gespeicherte Repräsentation aufhört, aktuell zu sein, und was dann
  geschieht
- [RenderCache](render-cache.md) - Richtlinien und Varianz deklarieren und
  die Gründe, aus denen ein Rendering nie gespeichert wird
- [Live](live.md) - die Inseln, für die eine zusammengesetzte Shell Löcher
  hat

# RenderCache Generationen

Die meisten Caches laufen ab. RenderCache läuft auch ab, doch der Ablauf ist
die Rückfallebene und nicht der Mechanismus. Der Mechanismus ist eine
**Generation**: Zu jedem Datenstück, das ein Rendering gelesen hat, gibt es
einen Zähler in der Datenbank, das Rendering speichert die Zähler, die es
gesehen hat, und ein Schreibzugriff erhöht den Zähler für das, was er
geändert hat. Eine gespeicherte Repräsentation ist aktuell, wenn die Zähler,
die sie gesehen hat, noch zu den Zählern passen, die die Datenbank jetzt
hält. Sie schreiben für Ihre eigenen Daten keine Invalidierungsregel, denn
ein gewöhnliches `model.save()` ist bereits eine.

In diesem Kapitel geht es um diese Maschinerie von außen: wovon ein Rendering
als abhängig aufgezeichnet wird, wie grob diese Abhängigkeiten wirklich sind,
was das Framework nicht sehen und deshalb nicht invalidieren kann, wie die
Kohärenzprüfung bei einem Treffer bezahlt wird, welche Anfrage neu aufbaut,
wenn mehrere denselben Eintrag zugleich wollen, und was ein Besucher im
Fenster zwischen „nicht mehr aktuell“ und „neu aufgebaut“ ausgeliefert
bekommt. Jede Behauptung unten wird von einem benannten Test oder einer
eingecheckten Messung gehalten; die Dogfood-Beispiele sind Routen in
`app/src/live/mod.rs`, nachgewiesen durch `app/tests/live_render_cache.rs`.

## Wovon ein Rendering als abhängig aufgezeichnet wird

Während ein Rendering läuft, zeichnet ein anfragegebundener Sammler jede
Abhängigkeit auf, die er benennen kann: ein Tabellenlesen, ein Lesen eines
Datensatzes über den Primärschlüssel, eine Query-Klasse, eine Relation, eine
Konfigurationsidentität, ein Feature, eine Locale, eine Route und eine immer
vorhandene `Broad`-Identität, die jede Repräsentation beobachtet.
Lesezugriffe über den ORM und den Query-Builder zeichnen sich selbst auf; Sie
schreiben nichts.

`/live/todos` ist das ganze Muster in einem Handler:

```rust
pub async fn todos(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let titles: Vec<String> = Todo::all()
            .await?
            .into_vec()
            .into_iter()
            .map(|todo| todo.title)
            .collect();
        html(&TodosView { count: titles.len(), titles })
    }
    .await;
    result.map_err(failed)
}
```

`Todo::all()` zeichnet die Tabelle `todos` auf. Nichts sonst im Handler oder
im Template liest die Sitzung, den angemeldeten Besucher oder eine
Übersetzung, und genau das lässt die Route überhaupt eine geteilte
Repräsentation bleiben.

## Ein gewöhnlicher Schreibzugriff ist die Invalidierung

`an_orm_write_invalidates_the_todos_document_through_generations` geht den
ganzen Zyklus durch die laufende Anwendung ab:

1. Das erste `GET /live/todos` rendert und veröffentlicht.
2. Das zweite ist ein Treffer: Es erreicht nie den Handler, trägt kein
   `Warning` und plant nichts ein.
3. Ein `POST /todos/random` schreibt eine Zeile - über die eigene Route der
   Anwendung, mit der Sitzung und dem CSRF-Token, die ein Browser senden
   würde.
4. Das nächste `GET` wird mit `Warning: 110 - "Response is Stale"` bedient
   und plant genau einen Neuaufbau im Hintergrund ein. Seine fünf frischen
   Minuten haben kaum begonnen, also ist die erhöhte Generation der Tabelle
   `todos` das Einzige, was beides erklären kann.
5. Dieser Neuaufbau läuft wirklich: Der Test wartet auf den Render-Zähler -
   eine Zustandsbarriere, keine zeitgesteuerte Wartezeit -, bis ein
   Rendering geschehen ist, das der Test selbst nicht ausgelöst hat.
6. Die geschriebene Zeile ist wirklich in der Liste. Das ist ein
   **eigener** Schritt und bewusst keine Zusicherung über die Ausgabe des
   Hintergrund-Neuaufbaus selbst: Der Test verwirft zuerst L0 und rendert
   erneut, denn die Veröffentlichung des Neuaufbaus landet in einem Moment,
   den nichts von der Anwendung aus Erreichbares beobachtbar macht, sodass
   eine Zusicherung über die Anfrage, die ihn zufällig erwischt, ein Rennen
   wäre.
7. Und die Route pendelt sich wieder auf einen einfachen Treffer gegen den
   erneut veröffentlichten Eintrag ein.

In dieser Abfolge wurde nirgends ein Cache-Schlüssel benannt. Ein
ORM-Schreibzugriff innerhalb einer `DB::transaction` erhöht seine
Generationen innerhalb genau dieser Transaktion, sodass ein
zurückgerollter Schreibzugriff überhaupt nichts erhöht.

## Die Invalidierung ist heute tabellengranular

Das ist das mit Abstand Wichtigste, das Sie wissen sollten, bevor Sie eine
gecachte Route dimensionieren.

Ein Punktlesen über den ORM zeichnet die Identität der **Tabelle** ebenso auf
wie die Identität der Zeile. `Model::find` ruft
`observe_table_read(Self::TABLE)` auf, bevor es irgendetwas nachschlägt, und
`observe_record_read_json`, nachdem es eine Zeile hydriert hat, sodass ein
Eintrag, der eine Zeile gelesen hat, von der ganzen Tabelle abhängt. Jeder
Schreibzugriff auf diese Tabelle invalidiert deshalb **jeden** gecachten
Eintrag, der daraus gelesen hat, und nicht nur die Einträge, die die
geänderte Zeile gelesen haben.

Das ist sicher - es kann nur zu viel invalidieren, nie zu wenig - und es ist
gemessen statt angenommen. Die Invalidierungssturm-Last in
`framework/benches/render_cache_workloads.rs` veröffentlicht 64 Schlüssel
über 12 Datensatz-Identitäten, treibt 1.000 Schreibzugriffe und zeichnet den
beobachteten Fan-out in
`crates/suprnova-live/benchmarks/render-cache-workloads-v1.json` auf
(gekürzt; das aufgezeichnete Objekt trägt außerdem die Felder für Burst,
Sweep, Treffer, Neuaufbau, Statement und Latenz):

```json
"invalidation_storm": {
  "keys": 64,
  "identities": 12,
  "writes": 1000,
  "every_write_invalidates_every_key": true,
  "rebuilds_per_write": 1.28,
  "final_bodies_coherent": true
}
```

Gestalten Sie darum herum. Eine gecachte Route, die auf einer Tabelle
aufbaut, in die Ihre Anwendung ständig schreibt, wird ständig neu aufbauen,
was auch immer ihr Frische-Fenster sagt. Eine gecachte Route, die auf einer
Tabelle aufbaut, die sich ändert, wenn ein Redakteur etwas
veröffentlicht, wird stundenlang stillstehen. Wenn Sie eine feinere
Granularität als die Tabelle brauchen, lautet die ehrliche Antwort heute,
dass Sie sie nicht haben.

## Was das Framework nicht sehen kann

Eine Abhängigkeit, die sich nicht benennen lässt, lässt sich nicht
invalidieren, und das Framework ist bewusst darin, welche davon es nicht
speichert und welche es durchlässt.

**Rundweg abgelehnt.** Rohes SQL über `DB::select`, `DB::select_one`,
`DB::scalar` oder `DB::select_on` kann die Tabellen, die seine Anweisung
gelesen hat, nicht benennen, also wird das Rendering als unbeobachtbar
markiert und nie gespeichert. Die Antwort wird trotzdem jedes Mal korrekt
ausgeliefert. Die eigenen RBAC-Rollen- und Berechtigungsprüfungen des
Frameworks lesen auf diese Weise, sodass eine gecachte Route, die eine davon
auswertet, nie speichert. Lesezugriffe über `DB::table(..)` kennen ihre
Tabelle und cachen normal.

**Unsichtbar und in Ihrer Verantwortung.** Ein über `Request::header`
gelesener Anfrage-Header, ein `Config::get`-Aufruf und ein globaler
Eloquent-Scope, der eine Query aus seinem eigenen Zustand pro Anfrage
filtert, ändern alle, was ein Rendering erzeugt, ohne dass der Sammler
irgendetwas sieht. Deklarieren Sie auf einer solchen Route die passende
Varianzdimension; nichts hier kann das Versäumnis für Sie abfangen.

**Bekannte Lücken.** Nichts erhöht eine Generation für ein Feature-Flag,
wenn das Flag sich ändert, und ein Schreibzugriff durch einen Queue-Worker,
eine geplante Aufgabe oder einen Konsolenbefehl erhöht überhaupt nichts, denn
nur der Prozess, der `RenderCache::install` ausgeführt hat, trägt die
Instrumentierung der Schreibseite. Eine Seite, die von einem solchen
Schreibzugriff abhängt, bleibt nur innerhalb ihres Frische-Fensters aktuell;
führen Sie `render-cache:epoch-advance` aus, nachdem ein Job gelaufen ist,
der ändert, was gecachte Seiten anzeigen. Siehe
[RenderCache Betrieb](render-cache-operations.md).

## Was ein Treffer kostet

Die Kohärenzprüfung ist das, was aus „wir haben Bytes“ ein „diese Bytes sind
aktuell“ macht, und sie ist die einzige Arbeit, die ein Treffer leistet.

Ein Treffer führt **keinen Handler, keine ORM-Query, kein Template und
keinen Serializer** aus und kopiert keine Rumpf-Bytes: Die Bytes, die der
Server in den Socket schreibt, sind die Bytes, die der Store hält, in
`framework/tests/render_cache/bypass.rs` über die Adresse statt über den Wert
nachgewiesen. Was bleibt, ist das Datenbanklesen, das die Aktualität
nachweist, und wie oft Sie dafür zahlen, ist der `CoherenceMode` der
Richtlinie:

| Modus | SQL-Anweisungen pro heißem Treffer | Worauf er vertraut |
|---|---|---|
| `Authority` (Standard) | genau 1 | dem Ledger, bei jedem Treffer neu gelesen |
| `Lease { max_age_ms }` | 0 | einer lokal gewährten Validierungs-Lease, bis sie abläuft |

`an_authority_mode_hit_issues_exactly_one_statement` hält den
Authority-Modus auf einem Roundtrip: Die beobachteten Generationen und die
Autoritätsepoche werden zusammen in einem einzigen `UNION ALL` gelesen, nicht
als zwei Lesezugriffe. `a_lease_mode_hit_runs_nothing_and_issues_no_statement`
hält den Lease-Modus auf null, weil die Epoche, unter der der Schlüssel
abgeleitet wurde, zusammen mit den Generationen verleast statt pro Anfrage
gelesen wird.

Die Epoche selbst wird einmal pro Prozess gelesen, nicht einmal pro Anfrage.
`the_epoch_is_read_once_at_first_use` misst den ersten Fehltreffer einer
frischen Laufzeit gegen eine ansonsten identische zweite und stellt fest,
dass der erste genau eine Anweisung mehr zahlt - das eine Autoritätslesen,
das die Epochen-Lease füllt. Jede Anfrage danach zahlt nichts mehr dafür.

## Wenn sich die Epoche bewegt

`render-cache:epoch-advance` ist die Notfall-Invalidierung, und die Epoche
ist in jeden Lookup-Schlüssel eingebacken, sodass das, was als Nächstes
geschieht, davon abhängt, wo Sie stehen:

- **Auf dem Knoten, der den Befehl ausgeführt hat**, sieht die
  allernächste Anfrage die neue Epoche. L0 wird im selben Moment vollständig
  geleert, und der Cache ist sofort invalidiert.
- **Auf einem anderen Knoten** erfährt eine Route im `Authority`-Modus es
  bei ihrem allernächsten Treffer. Eine Route im `Lease`-Modus erfährt es
  bei ihrem nächsten erneuten Autoritätslesen, also höchstens `max_age_ms`
  später.

Eine Route mit einem Veraltet-auslieferbar-Fenster liefert den bewegten
Eintrag einmal unter `Warning` aus, während der Neuaufbau hinter der Anfrage
läuft; eine Route ohne ein solches Fenster baut im Vordergrund neu auf, und
die anfragende Seite wartet darauf. Dieser Unterschied ist der ganze Grund,
ein Veraltet-auslieferbar-Fenster zu deklarieren, und er gilt für jede
Bewegung, nicht nur für ein Vorrücken der Epoche.

Drei Tests in `framework/tests/render_cache/middleware.rs` halten diese Pfade
namentlich fest:
`an_epoch_advanced_by_another_node_reaches_an_authority_mode_route_on_its_next_hit`,
`an_epoch_advanced_by_another_node_reaches_a_lease_mode_route_when_its_lease_expires`
und
`an_epoch_advanced_by_another_node_serves_a_stale_servable_entry_once_then_rebuilds`.

## Ein Neuaufbau pro Schlüssel: Singleflight und Wartende

Wenn ein Eintrag fehlt oder nicht mehr aktuell ist, rendern nicht alle
Anfragen, die dafür eintreffen. Sie werden über einen
**Neuaufbau-Koordinator** zugelassen, der genau eine von ihnen auswählt:

- Der **Leader** ist die eine Anfrage, die rendert und veröffentlichen darf.
  Er hält für die Dauer seines Renderings eine Lease auf diesem Schlüssel.
- **Wartende** sind die Anfragen, die für denselben Schlüssel eintreffen,
  während der Leader ihn hält. Sie warten im Prozess, und wenn der Leader
  freigibt, bewerten sie neu, was jetzt gespeichert ist, und liefern das
  aus. Eine wartende Anfrage vertraut dem Warten nie: Wenn der Zyklus des
  Leaders nichts veröffentlicht hat oder etwas veröffentlicht hat, das die
  eigene Frischeprüfung der wartenden Anfrage für tot hält, rendert diese
  ebenfalls, statt auszuliefern, was sie vorgefunden hat.
  `a_singleflight_waiter_never_serves_a_superseded_entry_as_fresh` in
  `framework/tests/render_cache/middleware.rs` ist genau diese Regel.
- Eine Anfrage, die eintrifft, wenn bereits `RENDER_CACHE_MAX_WAITERS`
  (standardmäßig 128) warten, **umgeht** den Cache: Sie rendert und
  veröffentlicht nichts, statt eine unbegrenzte Warteschlange wachsen zu
  lassen.

`concurrent_misses_render_once_and_waiters_reuse_the_publication` weist den
gewöhnlichen Fall von Ende zu Ende nach - zwei gleichzeitige Fehltreffer, ein
Rendering, identische Rümpfe - und
`one_leader_per_key_and_fence_with_bounded_waiters` in
`crates/suprnova-live/tests/render_cache_singleflight.rs` weist die
Obergrenze direkt gegen den Koordinator nach: Jenseits seiner Wartegrenze
antwortet die Zulassung mit `Bypass`.

Zwei Veröffentlichungen für einen Schlüssel können nie beide angenommen
werden, was auch immer der Koordinator entschieden hat. Ein Leader prägt
unter seiner Lease ein Veröffentlichungs-Token, und der Store vergleicht
diesen Fence, bevor er schreibt: Eine ältere Epoche oder eine gleiche Epoche
mit einem niedrigeren Token verliert. Das macht doppeltes *Rendern* sicher
annehmbar, während doppeltes *Veröffentlichen* es nicht ist, und deshalb gibt
es überhaupt kein knotenübergreifendes Warten: Ein Schlüssel, den ein anderer
Knoten gerade neu aufbaut, ist hier ein Bypass. Siehe
[RenderCache Bereitstellung](render-cache-deployment.md).

## Etwas ausliefern, während es neu aufgebaut wird

Die vier Frischezustände, die Bänder, die `FreshnessPolicy` festlegt, und
das `Warning` und das `Age`, die eine veraltete Antwort trägt, sind in
[RenderCache Repräsentationen](render-cache-representations.md) definiert.
Worauf es hier ankommt, ist, dass eine Generationsbewegung einen Eintrag
früh in diese Bänder versetzt: Ein bewegter Eintrag wird mit einem
effektiven Alter von **mindestens** seinem Frische-Intervall bewertet, wie
alt er wirklich auch sein mag. Sein wirkliches Alter entscheidet weiterhin,
in welchem Band er damit landet:

- Wirkliches Alter unter `fresh_ms + stale_servable_ms`, auf einer Route,
  die ein Veraltet-auslieferbar-Fenster deklariert: veraltet und
  auslieferbar. Die gespeicherte Kopie wird einmal unter `Warning`
  ausgeliefert, und der Neuaufbau läuft hinter der Anfrage. Das ist Schritt 4
  des Schreibtests oben, an einem Eintrag, dessen fünf frische Minuten kaum
  begonnen hatten.
- Wirkliches Alter darüber hinaus, aber noch nicht an der Todesgrenze:
  veraltet bei Fehler. Die Anfrage wartet auf einen Neuaufbau im Vordergrund
  und sieht die gespeicherte Kopie nur, wenn dieser Neuaufbau fehlschlägt.
- Auf einer Route ganz ohne Veraltet-auslieferbar-Fenster und auf jeder
  `PrivateCached`-Route (deren Todesgrenze *ihre* Frische-Grenze ist) ist
  eine Bewegung gleich tot: Die Anfrage baut im Vordergrund neu auf und
  wartet.

`stale_service_is_marked_and_rebuilt_in_the_background` zeigt dieselbe
Übergabe, angetrieben von der Uhr statt von einem Schreibzugriff: Jenseits
der 300.000 frischen Millisekunden von `/live/todos` und innerhalb seiner
60.000 veraltet-auslieferbaren wird dem Besucher die vorhandene Kopie unter
`Warning: 110 - "Response is Stale"` und `Age: 300` gereicht, genau ein
Neuaufbau wird eingeplant, und dieser Neuaufbau läuft wirklich.

Der Veraltet-bei-Fehler-Rückfall deckt die Anfrage ab, die einen Neuaufbau
anführt, **und** eine wartende Anfrage hinter einem Leader, dessen Neuaufbau
fehlgeschlagen ist. Beide werden auf dieselbe Weise beantwortet: mit den
veralteten Bytes unter `Warning` statt mit dem Fehlschlag.
`framework/tests/render_cache/races.rs` weist jeden Arm einzeln nach,
`a_waiter_behind_a_failed_leader_is_served_the_stale_entry_it_was_waiting_on`
und `a_waiter_that_re_evaluates_onto_a_stale_on_error_entry_falls_back_to_it`,
und den zweiten durch Rückbau: Nimmt man den Rückfall aus dem wartenden Arm
heraus, werden aus seinen abschließenden Zusicherungen `200` ein `500`.

Zusammengesetzte Routen sind die Ausnahme, und sie ist beabsichtigt. Ein
`Composite`-Eintrag wird nie vom Veraltet-bei-Fehler-Rückfall ausgeliefert
und löst nie einen Neuaufbau im Hintergrund aus: Eine gespeicherte Shell nach
einem fehlgeschlagenen Neuaufbau auszuliefern würde eine Anfrage beantworten,
die die eigene Autorisierungskette der Route nie zu Gesicht bekommen hat, und
ein Neuaufbau im Hintergrund trägt nichts vom Autorisierungszustand der
Anfrage, sodass seine Shell das wäre, was die Seite für niemanden rendert.
Auf einer zusammengesetzten Route ist das eigene Ergebnis eines
fehlgeschlagenen Neuaufbaus das, was der Client sieht.

## Gecachte Routen sind Lesepfade

Das Rendering des Leaders läuft innerhalb einer Datenbanktransaktion, auf
PostgreSQL und MySQL mit `REPEATABLE READ` geöffnet, sodass die Generationen,
die es aufzeichnet, und die Daten, die es gelesen hat, sich einen Snapshot
teilen. Daraus folgen zwei Konsequenzen.

Der Handler einer gecachten Route, der **schreibt**, konkurriert mit
gleichzeitigen Schreibern um dieselben Zeilen, und auf PostgreSQL erhält ein
Handler, der eine Zeile aktualisiert, die eine andere Transaktion nach dem
Beginn des Renderings verändert hat, einen Serialisierungsfehler. Gestalten
Sie gecachte Routen als Lesepfade.

Ein Schreibzugriff außerhalb jeder Transaktion, `model.save()` für sich
allein, committet zuerst seine Zeile und erhöht seine Generationen in einer
unmittelbar folgenden Transaktion. Der Moment dazwischen ist „neue Daten,
alte Generation“: Er kostet einen zusätzlichen Neuaufbau und liefert nie
veralteten Inhalt aus.

Schließlich werden nach dem Ende des Renderings die beobachteten
Abhängigkeiten und die Epoche erneut gelesen, außerhalb der eigenen
Transaktionssicht des Renderings. Alles, was sich während des Renderings
bewegt hat, verwirft den Kandidaten, statt ihn zu veröffentlichen. Deshalb
kostet ein Schreibzugriff, der mitten im Rendering landet, einen Neuaufbau
statt einer falschen Seite.

### Warum Suprnova abweicht

Laravels Cache ist ein Schlüssel-Wert-Speicher, und seine
Response-Caching-Pakete sind darauf aufgebaut, also ist die Invalidierung
etwas, das Sie schreiben. Sie rufen `Cache::forget` auf, oder Sie taggen
Einträge und leeren ein Tag, oder Sie registrieren einen Model-Observer, der
die Schlüssel löscht, von denen Sie glauben, dass dieses Model sie speist.
Jedes davon ist eine Zuordnung, die Sie von Hand pflegen, und der Fehlerfall
ist still: Die Seite, die niemand zu vergessen erinnerte, wird weiter
ausgeliefert, bis ihre TTL abläuft.

Suprnova dreht die Richtung um. Das Rendering zeichnet auf, was es gelesen
hat, der Schreibzugriff erhöht, was er geändert hat, und beide treffen sich
in einem Datenbank-Ledger statt in Ihrem Kopf. Es gibt keinen
`forget`-Aufruf, den man vergessen könnte. Der Preis ist, dass die
aufgezeichnete Abhängigkeit eine Tabelle und keine Zeile ist, sodass eine
vielbeschriebene Tabelle ihre Abhängigen oft neu aufbaut, und dass
Lesezugriffe über rohes SQL von der Speicherung abgelehnt werden, statt mit
einer Abhängigkeit gecacht zu werden, die niemand benennen kann. Beides ist
sichtbar und gemessen - der Fan-out in der eingecheckten Sturmlast, die
Ablehnung an Ihrem eigenen fehlenden `Age`-Header - und nicht eine veraltete
Seite, von der Sie durch einen Kunden erfahren.

## Nächste Schritte

- [RenderCache Bereitstellung](render-cache-deployment.md) - Profile,
  Provider und die Migration, die die Generationswahrheit dauerhaft macht
- [RenderCache Repräsentationen](render-cache-representations.md) - was
  tatsächlich gespeichert wird und unter welchem Schlüssel
- [Datenbank](database.md) - Transaktionen und Isolation, innerhalb derer
  gecachte Renderings laufen

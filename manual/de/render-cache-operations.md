# RenderCache Betrieb

Ein Cache, den Sie nicht sehen können, ist ein Cache, dem Sie nicht trauen
können. RenderCache beantwortet zwei Betreiberfragen direkt und ohne je eine
gespeicherte Seite auszugeben: **Was hält dieser Knoten unter diesem
Schlüssel, und ist es noch aktuell?** und **Wie bringe ich alles zum
Stillstand?** Eine dritte, „Wird diese Route überhaupt aus einer
gespeicherten Kopie bedient?“, beantwortet er über Telemetrie und über den
`Age`-Header statt über einen Befehl, denn diese Frage handelt vom Verkehr
und nicht von einem Eintrag. Es gibt zwei Konsolenbefehle, acht
Telemetriezähler, einen begrenzten Bereinigungslauf auf der Festplatte und
einen Notfallhebel.

Dieses Kapitel ist die Betriebsoberfläche: die Befehle, genau das, was sie
ausgeben, und das, was sie sehen können; die Zähler und ihre geschlossenen
Ergebnismengen; wie die Datei-Ebene Plattenplatz zurückgewinnt; wie Sie eine
gecachte Route so testen, dass der Test das Cachen nachweist und nicht bloß
das Antworten; was zu tun ist, wenn etwas nicht stimmt, einschließlich des
Mehrknotenverfahrens, das eine Datenbankwiederherstellung braucht; und wie
die Leistung des Cache selbst gemessen wird und was diese Zahlen ehrlich
wert sind. Die Befehlsbeispiele sind die, die
`the_operator_commands_inspect_without_a_body_and_advance_the_epoch` über den
eigenen Konsolen-Einstiegspunkt dieses Repositorys in
`app/tests/live_render_cache.rs` durchspielt.

## Die zwei Konsolenbefehle

Beide sind verborgene Befehle, vom Framework registriert und über das
`console`-Binary Ihres Projekts erreichbar wie jeder andere. Keiner gibt je
einen gespeicherten Rumpf oder eine rohe Abhängigkeitsidentität aus.

```bash
cargo run --bin console -- render-cache:inspect rk1.<43 base64url characters>
cargo run --bin console -- render-cache:epoch-advance
```

**`render-cache:inspect <key>`** meldet die Gestalt eines gespeicherten
Eintrags: seine Repräsentationsklasse, seine `body_bytes`, seine übrigen
Metadaten und daneben die aktuelle Autoritätsepoche, sodass Sie erkennen
können, ob der Eintrag, den Sie ansehen, noch gültige Autorität ist oder
darunter bereits veraltet ist. Er gibt `no entry (current epoch: {epoch})`
aus, wenn der Schlüssel nichts benennt, was er sehen kann, und er
scheitert - er meldet keinen Erfolg - bei einem nicht parsbaren Schlüssel
oder ohne installierte Laufzeit.

**Er liest das In-Process-L0 dieses Prozesses und sonst nichts.**
`RenderCache::inspect` schlägt den Schlüssel allein in L0 nach; es zieht nie
die L1-Ebene heran. Auf dem Datenbank- oder dem Redis-Profil ist das von
Bedeutung: Ein Eintrag, der in `suprnova_render_entries` oder in Redis lebt
und von einem anderen Knoten oder von diesem hier vor einem Neustart
veröffentlicht wurde, gibt hier `no entry` aus, sofern dieser Prozess ihn
seit seinem Start nicht ausgeliefert hat. Lesen Sie den Bericht als „was
dieser Knoten im Speicher hat“ und nie als „was die Bereitstellung
gespeichert hat“. Dasselbe gilt für `RenderCache::store_inspection`, das die
L0-Belegung und die aktuelle Epoche meldet.

Der Schlüssel ist der Text, den das Lookup selbst verwendet: `rk1.` plus 43
base64url-Zeichen, und das ist es, was die Protokollierung und die
Telemetrie Ihrer Anwendung sichtbar machen können. Er ist kein zweiter Hash
von irgendetwas, sodass ein Schlüssel, den ein Betreiber in der Hand hält,
genau einen Eintrag benennt.

Die Behauptung der Rumpffreiheit wird geprüft und nicht bloß aufgestellt. Der
Test nimmt das Dokument, das tatsächlich ausgeliefert wurde, zerlegt es in
Zeilen und verlangt, dass **jede** nicht triviale Zeile davon in dem fehlt,
was der Inspect-Bericht ausgegeben hat.

**`render-cache:epoch-advance`** ist die Notfall-Invalidierung. Er rückt die
Autoritätsepoche vor und gibt `epoch advanced to {epoch}` aus. Weil die
Epoche in jeden Lookup-Schlüssel eingebacken ist, rückt das gespeicherte
Einträge außer Reichweite, ohne dass etwas aufzuzählen oder zu löschen wäre.
Der Test sichert die ausgegebene Zeile zu und dann die Konsequenz, auf die es
ankommt: Nach dem Befehl rendert die Route wieder.

**Auf dem Knoten, der ihn ausführt**, wirkt es sofort: Der Befehl gibt die
Epochen-Lease dieses Prozesses ab und leert seine In-Process-Ebene, sodass
seine allernächste Anfrage Schlüssel unter der neuen Epoche ableitet und
nichts findet. (Dieser letzte Satzteil gilt, solange sich die Epoche nur
vorwärts bewegt, was der gewöhnliche Fall ist; nach einer
Datenbankwiederherstellung kann der vorgerückte Wert einer sein, den die
Bereitstellung schon einmal verwendet hat, siehe also „Die Datenbank
wiederherstellen“ unten.) **Auf jedem anderen Knoten** hat sich das Ledger
bewegt, doch jener Prozess hält weiterhin seine alte verleaste Epoche und
sein eigenes L0, und er zieht bei seinem nächsten Autoritätslesen nach:
sofort unter `CoherenceMode::Authority` und bis zu `max_age_ms` später unter
`CoherenceMode::Lease`. Führen Sie den Befehl auf jedem Knoten aus, oder
starten Sie die anderen neu. „Die Datenbank wiederherstellen“ unten hat das
vollständige Verfahren und die Tests dahinter.

Greifen Sie danach, wenn mit gecachtem Inhalt etwas nicht stimmt und Sie
nicht warten können, bis einzelne Einträge ablaufen, und nach einem Job, der
geändert hat, was gecachte Seiten anzeigen (siehe „bekannte Lücken“ in
[RenderCache Generationen](render-cache-generations.md)).

## Berechtigungsänderungen

`RenderCache::bump_permission_version().await?` ist der eine
Invalidierungsaufruf, den eine Anwendung von Hand macht, und er ist nicht
wirklich ein Betriebsbefehl: Er gehört in den Codepfad, der ändert, wozu ein
angemeldeter Benutzer berechtigt ist. Er erhöht eine persistierte Generation,
die jedes principal-geschlüsselte Rendering beobachtet, er übersteht einen
Neustart, und er schließt sich der Transaktion an, in der die
Rollenänderung läuft, sofern es eine gibt. Ohne ihn passt ein Benutzer,
dessen Berechtigungen sich gerade geändert haben, weiterhin zu allem, was
unter seinem vorherigen Berechtigungssatz gecacht wurde.

## Telemetrie

Acht geschlossene Zählernamen, und in keinem davon wird eine Ebene, ein
Provider oder ein Backend benannt:

| Zähler | Attribut |
|---|---|
| `suprnova.render_cache.lookups` | `outcome`, und `reason`, wenn `outcome="declined"` |
| `suprnova.render_cache.hits` | `outcome` |
| `suprnova.render_cache.publications` | keines |
| `suprnova.render_cache.rebuilds` | keines |
| `suprnova.render_cache.stitch.assemblies` | `outcome` |
| `suprnova.render_cache.stitch.slots` | `outcome` |
| `suprnova.render_cache.stitch.nested` | `outcome`, `cause` |
| `suprnova.render_cache.epoch_rewinds` | keines |

`lookups` und `hits` tragen dieselbe geschlossene Menge von acht Ergebnissen:

- `l0`, `l1` - ein frischer Eintrag, ausgeliefert aus der In-Process- oder
  aus der geteilten Ebene.
- `conditional` - ein frischer Treffer, dessen `If-None-Match` gepasst hat,
  beantwortet mit `304`.
- `stale` - ein veraltet-auslieferbarer Eintrag, der sofort ausgeliefert
  wurde, oder der Veraltet-bei-Fehler-Rückfall, nachdem ein Neuaufbau im
  Vordergrund fehlgeschlagen ist.
- `miss` - nichts gefunden, ein Veraltet-bei-Fehler-Neuaufbau läuft, oder ein
  toter Eintrag.
- `bypass` - ein nicht deklarierter Query-Parameter, eine nicht auflösbare
  deklarierte Varianzdimension oder eine erschöpfte Warteliste.
- `moved` - das erneute Lesen nach dem Rendern hat festgestellt, dass sich
  eine Abhängigkeit oder die Epoche geändert hatte; der Kandidat wurde
  verworfen und nie veröffentlicht.
- `declined` - das Rendering war nicht speicherbar, aus einem von
  achtunddreißig Gründen unten, mitgeführt im Attribut `reason` neben
  `outcome`. `reason` wird nur zusammen mit `outcome="declined"`
  ausgegeben; jedes andere Ergebnis führt keinen. Der Grund wird aus einem
  typisierten Wert an genau der Verzweigung berechnet, die abgelehnt hat,
  nie im Nachhinein aus der Antwort rekonstruiert, sodass er den Vertrag
  benennt, der das Rendering tatsächlich verweigert hat:

  - Eignung (`policy.eligibility`, spiegelt die eigene `DeclineReason` der
    Engine): `policy_uncacheable`, `method`, `status`, `streaming`,
    `sets_cookie`, `unsafe_header_name`.
  - Beobachtung (der Bericht des Collectors und das Ledger-Lesen innerhalb
    der Transaktion): `observation_overflowed`, `ledger_read_failed`,
    `handler_not_begun`.
  - Klassifizierung, die zu `Uncacheable` eingeengt wurde:
    `session_value_read`, `secret_context_read`, `undeclared_context`.
  - Fakten des Live-Dokuments: `identity_bound_without_stitching`,
    `invalid_stitch_capture`, `no_store_intent`,
    `unresolvable_seed_deadline`.
  - Invarianten über den Schlüssel (ob die eigenen Beobachtungen des
    Renderings mit den Werten übereinstimmen, aus denen der
    Lookup-Schlüssel bereits gebaut war): `unreasoned_private_class`,
    `principal_undeclared`, `principal_divergent`, `tenant_undeclared`,
    `tenant_divergent`, `locale_undeclared`, `locale_divergent`.
  - Veröffentlichung: `seed_deadline_elapsed`, `unsafe_header_value`,
    `composite_capture_invalid`, `composite_slot_count_mismatch`,
    `composite_too_many_slots`, `composite_digest_mismatch`,
    `composite_empty_slot`, `composite_slot_not_found`,
    `composite_slot_ambiguous`, `composite_nested_unauthorizable`,
    `composite_nested_wider_class`, `composite_nested_longer_freshness`,
    `composite_nested_depth_exceeded`, `composite_nested_cycle`,
    `composite_nested_unresolvable`.

`hits` zählt nur für `l0`, `l1`, `conditional` und `stale` hoch.
`publications` zählt nur einen Store, der mit „veröffentlicht“ antwortet, nie
einen mit einem Fence abgewiesenen oder abgelehnten Versuch. `rebuilds` zählt
einen pro angestoßenem Neuaufbau im Hintergrund.

Die beiden Insel-Stitch-Zähler tragen ihre eigenen Mengen: `assembled` und
`fail_document` für Zusammensetzungen; `rendered`, `omitted`, `fallback` und
`failed` für Slots.

`suprnova.render_cache.stitch.nested` unterscheidet das eigene Ergebnis
eines benannten, gecachten inneren Segments von dem eines Insel-Slots, mit
einer Erhöhung pro Auflösungsversuch eines `Segment::Nested`. Sein Attribut
`outcome` nimmt genau einen von `resolved`, `omitted`, `fallback` und
`failed` an; sein Attribut `cause` nimmt genau einen von `none` (nur
verwendet, wenn `outcome="resolved"`), `fetch_failed`, `version_mismatch`,
`length_mismatch`, `depth_exceeded`, `cycle` und `unauthorized` an. Keines
der beiden Attribute führt je einen Schlüssel, einen Routennamen oder einen
Identitätsdigest mit sich. Ein fehlgeschlagenes oder degradiertes Segment
wird immer über die Richtlinie behandelt, die der einschließende Graph
dafür erklärt hat (`FailDocument`/`Omit`/`Fallback`), genau wie das
Fehlschlagen eines Insel-Slots selbst; `outcome="failed"` (aus einer
`FailDocument`-Richtlinie) bricht die Zusammensetzung für das ganze
Dokument ab und fällt zurück auf den eigenen ungecachten Handler der
Route.

`epoch_rewinds` zählt Erkennungen, nicht Einträge: eine Erhöhung jedes Mal,
wenn ein Knoten auf einen Eintrag oder eine verleaste Epoche trifft, die
über der eigenen Epoche der Autorität gestempelt ist, die Epoche des
Ledgers über diesen Stempel hinaus anhebt und sein eigenes L0 leert. Ein
von null verschiedener Wert nach einer Wiederherstellung der Datenbank ist
das Signal, dass die Wiederherstellung bemerkt wurde. Ein von null
verschiedener Wert zu jedem anderen Zeitpunkt bedeutet, dass eine Autorität
aus einem von niemandem beabsichtigten Grund rückwärtsgelaufen ist.

**Eine hohe `declined`-Rate ist das Signal, auf das zu alarmieren sich
lohnt.** Sie bedeutet, dass Routen, die Sie aufgenommen haben, korrekt
rendern und ausliefern, dabei aber nie gespeichert werden, und die Antwort
sieht so oder so identisch aus. Die schnellste lokale Prüfung sind zwei
Anfragen hintereinander: Trägt die zweite keinen `Age`-Header, wurde nichts
gespeichert.

## Datenträgerhygiene

Nur die Datei-Ebene braucht Bereinigung, und sie bereinigt sich meist selbst.

`FileRenderStore` speichert eine Datei pro Schlüssel, flach unter
`RENDER_CACHE_L1_DIR`. Ein Eintrag ist tot, wenn sein Alter seit der
Veröffentlichung die Aufbewahrung erreicht, mit der er veröffentlicht wurde,
oder wenn seine Fence-Epoche älter als die aktuelle Epoche ist. Die
Aufbewahrung stammt von derselben klassenbewussten Todesgrenze, die die
laufende Frischeprüfung verwendet, sodass die Datei eines privaten Eintrags
früher ausgemustert wird als die eines öffentlichen und ein
Bereinigungslauf einer Frischeprüfung nie darin widersprechen kann, ob ein
Eintrag wirklich tot ist.

`sweep` entfernt höchstens 64 Einträge pro Aufruf, die älteste
Veröffentlichung zuerst, und gibt zurück, ob weitere übrig sind. Es läuft
automatisch bei jeder 256. Veröffentlichung, sodass ein gesundes Verzeichnis
keine Aufmerksamkeit braucht. `RenderCache::sweep()` treibt es ausdrücklich
an, wenn Sie möchten, und ein Rückstau, der größer ist als die Grenze eines
Aufrufs, läuft über spätere Auslöser ab, statt einen langen Durchlauf zu
blockieren.

Zwei Dinge, die der Bereinigungslauf nicht ist:

- **Ein Vorrücken der Epoche rührt L1 nicht an.** Es leert L0 vollständig,
  denn das ist Speicher im Prozess, gegen den es nichts abzugleichen gibt,
  und es lässt jede Datei aus der Zeit vor der Epoche auf der Festplatte,
  bis ein Bereinigungslauf sie einsammelt. Das ist Datenträgerhygiene und
  keine Korrektheitsfrage: Die Dateien sind über das Lookup ohnehin schon
  unerreichbar.
- **Die Datenbank-Ebene hat keinen automatischen Bereinigungslauf** und wird
  nur über `RenderCache::sweep()` zurückgewonnen. Die Redis-Ebene braucht
  keinen: Jeder Eintrag, den sie speichert, trägt einen Ablauf, und Redis
  gewinnt die Bytes selbst zurück.

Die Veröffentlichung ist absturzsicher. Sie schreibt eine temporäre Datei,
fsyncet sie, benennt sie über das Ziel um und fsyncet das übergeordnete
Verzeichnis, sodass eine lesende Seite immer nur die vorherige vollständige
Datei oder die neue vollständige Datei sieht. Beim Öffnen entfernt der Store
jede übrig gebliebene temporäre Datei und jede Datei, die ihre Frame-Prüfung
nicht besteht, und behandelt einen zerrissenen Schreibvorgang damit als
selbstheilend statt als dauerhaft vergifteten Eintrag.

## Eine gecachte Route testen

Ein Test, der zusichert, dass eine gecachte Route korrekt antwortet, besteht
unabhängig davon, ob die Antwort aus dem Store oder aus einem frischen
Rendering kam. Jede Behauptung muss gegen etwas erhoben werden, das nur ein
tatsächlich ausgelieferter gespeicherter Eintrag erzeugen kann. Vier Muster
leisten das, und die eigenen Dogfood-Tests dieses Repositorys nutzen alle
vier: `app/tests/live_render_cache.rs` mit dem Harness in
`app/tests/live_support/mod.rs`.

**1. Renderings auf der Handler-Seite des Cache zählen.** Registrieren Sie
eine zählende Middleware *nach* `RenderCache::install`. Die Registrierung
hängt an, sodass sie näher am Handler landet als `RenderCacheMiddleware`, und
eine Anfrage, die der Cache beantwortet, kehrt zurück, bevor sie sie aufruft:

```rust
let router = app::live::routes_with_render_cache_with_config(routes::register(), config)
    .await
    .expect("install the routes and the RenderCache middleware");
// After the install, so it only sees requests the cache forwarded.
render_counter::register();
```

Die Differenz zwischen zwei Ablesungen von `render_counter::renders()` ist
dann die Zahl der Renderings, die der Cache nicht vermieden hat, und sonst
nichts - anders als identische Rümpfe oder ein `Age`-Header, für die es
beide ehrliche Erklärungen ohne Cache gibt. Jede Treffer-Zusicherung in
`an_orm_write_invalidates_the_todos_document_through_generations` beruht
darauf. Warten Sie auf ein Rendering, das Sie nicht ausgelöst haben (einen
Neuaufbau im Hintergrund), mit der eigenen Barriere des Zählers,
`wait_until_renders_at_least`, nie mit einem Schlaf.

**Die Ausnahme ist eine `PublicShellStitched`-Route, und sie ist keine
kleine.** Ein zusammengesetzter Treffer wird bewusst durch die ganze Kette
der Route geleitet: Ihr Autorisierungswächter muss erneut laufen, und erst
die Live-Abschluss-Middleware am Ende dieser Kette liefert den Treffer aus.
Eine zählende Middleware, die global nach der Installation registriert
wurde, sitzt außerhalb der eigenen Kette der Route und wird bei einem
zusammengesetzten Treffer genauso erreicht wie bei einem Fehltreffer. Auf
einer solchen Route kann der Zähler die Behauptung „es lief kein Handler“
überhaupt nicht tragen.

Sichern Sie stattdessen zu, was der Store hält, woraus das ausgelieferte
Dokument besteht und wie alt es ist - und genau das tut
`the_dashboard_is_stitched_per_principal_from_one_shared_shell`: Der
gespeicherte Eintrag ist ein `EntryKind::Composite` mit der erwarteten
Slot-Zahl (`inspect_route_for_test`), die Dokumente zweier Principals
unterscheiden sich in ihren Insel-Tags und nirgends sonst, und die Antwort an
den zweiten Principal meldet ein `Age` von so vielen ganzen Sekunden, wie
seit der Veröffentlichung der Shell vergangen sind.

Das Letzte ist der Beweis, auf dem dieser Test steht, und es ist der lokale
Beleg für Auslieferung aus dem Store in seiner genauen Form. Der
`Age`-Header für sich ist das schwache Signal, vor dem dieses Kapitel weiter
oben gewarnt hat, denn ein Render setzt ebenfalls einen - auf null. Die Zahl
ist nicht schwach: Ein Render veröffentlicht seine Antwort und seinen Eintrag
im selben Augenblick, sodass eine gerenderte Antwort null meldet, wie weit
die Uhr auch gelaufen ist, während eine Zusammensetzung das Alter der Shell
meldet, aus der sie zusammengesetzt wurde. Der Test stellt dafür eine
verstellbare Uhr, weit innerhalb des Frischefensters der Route, sodass das
Abgelesene exakt ist und nicht zufällig.

Die Antwort trägt außerdem `Cache-Control: private, no-store`, aber lesen Sie
das als das, was es ist: die Direktive, die eine Route dieser Klasse mit
Slots trägt, festgelegt bei dem Render, der die Shell veröffentlicht, ebenso
wie bei jeder Zusammensetzung danach, denn sie richtet sich danach, was die
Bytes enthalten, und nicht danach, welcher Pfad sie erzeugt hat. „Mit Slots“
ist dabei das entscheidende Wort: Ein `Composite` ohne Slots behält
stattdessen das private `max-age` der Klasse, sodass die Direktive etwas über
eine Route mit Inseln darin sagt und nichts über eine ohne. Jener Test
sichert bei einem Treffer `renders() == before + 1` zu und sagt in seiner
eigenen Notiz, warum das die ehrliche Lesart ist und kein Fehlschlag.

**2. Den Eintrag zurücklesen.** Zwei Facade-Aufrufe sind gewöhnliche
öffentliche API: `RenderCache::store_inspection()` meldet die L0-Belegung,
die Bytes und die aktuelle Epoche, und `RenderCache::inspect(key_text)`
meldet die rumpffreien Metadaten eines Eintrags. Daneben stellt das Framework
verborgene Testnähte bereit, `#[doc(hidden)]` und mit `_for_test` benannt,
damit sie niemand für Anwendungs-API hält:

| Naht | Was sie einem Test gibt |
|---|---|
| `RenderCache::key_for_route_for_test(pattern, params, login)` | den Schlüsseltext, den die Middleware **bei Epoche 1** ableitet, den Wert, den die Migration setzt |
| `RenderCache::key_for_route_at_epoch_for_test(pattern, params, login, epoch)` | denselben, unter einer von Ihnen benannten Epoche |
| `RenderCache::inspect_route_for_test(pattern)` | den L0-Eintrag dieses Epoche-1-Schlüssels: Klasse, Art, Status, `body_bytes`, Slots |
| `RenderCache::inspect_l1_for_test(pattern, params, login)` | denselben, aus der konfigurierten L1-Ebene |
| `RenderCache::clear_l0_for_test()` | leert L0 und lässt L1, die Epoche und den Koordinator in Ruhe |

Die Epoche ist von Bedeutung, weil sie Teil des Schlüssels ist.
`key_for_route_for_test` schreibt Epoche 1 fest, sodass ein Test, der die
Epoche vorgerückt hat - auf diesem Knoten oder, über das Ledger, auf einem
anderen -, die neue mit `key_for_route_at_epoch_for_test` benennen muss, weil
er sonst einen Schlüssel nachschlägt, unter dem nichts veröffentlicht wurde.

`the_public_document_is_a_hit_whose_seed_still_promotes` verwendet
`store_inspection` und `inspect_route_for_test`, um zuzusichern, dass der
Eintrag existiert und unter der deklarierten Klasse gespeichert ist;
`the_database_profile_serves_a_hit_through_the_sql_stores` verwendet
`inspect_l1_for_test` und danach `clear_l0_for_test`, und das ist der einzige
Weg, nachzuweisen, dass eine spätere Anfrage aus L1 und nicht aus dem
Speicher kam.

**3. Die Uhr bewegen, statt zu warten.** Die Uhr, die die Laufzeit liest, ist
auf einer `RenderCacheConfig` setzbar und nie über `from_env`, sodass ein
Test, der ein Frische-Band braucht, seine eigene installiert:

```rust
let clock = Arc::new(AdjustableTestClock::new(unix_now_ms()));
// Bound to its own name first: passing `Arc::clone(&clock)` inline leaves
// the compiler inferring the trait object as the clone's return type.
let for_runtime = Arc::clone(&clock);
let config = RenderCacheConfig::from_env()?.with_clock_for_test(for_runtime);
// ... install through the application's own configuration seam, then:
clock.advance_ms(300_001);
```

`AdjustableTestClock` kommt aus `suprnova::live::testing`, und `unix_now_ms`
ist die eigene Wanduhr-Ablesung des Harness, sodass eine verstellbare Uhr
dort startet, wo die des Systems steht, statt bei einem Zeitursprung, dem
der Rest des Prozesses widersprechen würde. (Gemeint ist ein Nullpunkt der
Uhr, nicht die Autoritätsepoche, die dieses Kapitel sonst meint, wenn von
einer Epoche die Rede ist.) Der Harness fasst das Paar als
`setup_app_with_clock` und `advance_clock_ms` zusammen, von denen das zweite
in Panic gerät, statt still nichts zu tun, wenn der Boot die Systemuhr
genommen hat. `stale_service_is_marked_and_rebuilt_in_the_background` ist
der Test.

**4. SQL-Anweisungen zählen.** Ein Cache, der den Handler übersprungen, aber
bei jedem Treffer weiterhin die Datenbank befragt hat, erfüllt jeden Zähler
auf der Handler-Seite und kostet trotzdem einen Roundtrip.
`DbConnection::observe_statements_for_test` richtet SeaORMs
Metrik-Callback auf einen eigenen Zähler, und er sieht Anweisungen auf dem
Pool und auf jeder daraus gestarteten Transaktion:

```rust
// Immediately after connecting, before the connection is cloned or bound
// into the container: installing needs sole ownership of the pool, and the
// call reports `false` rather than counting nothing silently.
let installed = conn.observe_statements_for_test(|| {
    STATEMENTS.fetch_add(1, Ordering::SeqCst);
});
assert!(installed, "the statement observer needs an unshared connection");
```

Dem Callback wird nichts über die Anweisung gesagt - kein SQL-Text, kein
gebundener Wert -, denn ein Zählen ist der ganze Sinn.
`framework/tests/render_cache/bypass.rs` ist vollständig auf diesem Muster
geschrieben: `a_lease_mode_hit_runs_nothing_and_issues_no_statement` hält
einen Treffer im Lease-Modus auf null Anweisungen,
`an_authority_mode_hit_issues_exactly_one_statement` hält einen Treffer im
Authority-Modus auf eine, und `the_epoch_is_read_once_at_first_use` misst
zwei Fehltreffer gegeneinander, um zu zeigen, dass die Epoche ein Lesen pro
Laufzeit kostet.

Zwei Gewohnheiten, die es zu behalten lohnt. Starten Sie den Harness über die
eigene Konfigurationsnaht Ihrer Anwendung statt über einen handgebauten
Router, damit der Test dieselben Routen, Richtlinien und dieselbe
Middleware-Reihenfolge installiert wie der Server. Und fügen Sie nie eine
zeitgesteuerte Wartezeit hinzu: Jede Barriere oben ist eine Zustandsbarriere
auf einem Zähler, und genau das macht diese Tests reproduzierbar statt
flatterhaft.

## Wenn etwas nicht stimmt

- **Eine Seite zeigt Inhalt, von dem Sie wissen, dass er alt ist.** Prüfen
  Sie, ob die Route überhaupt speichert (zwei Anfragen, nach `Age` sehen).
  Jeder Prozess, dessen Konfiguration RenderCache aktiviert und dessen
  Datenbank die RenderCache-Migration enthält, erhöht Generationen für die
  eigenen Schreibzugriffe, sodass ein Queue-Worker, eine geplante Aufgabe
  oder ein Konsolenbefehl dieselben Generationen ungültig macht wie der
  dienende Prozess; prüfen Sie, ob der schreibende Prozess RenderCache
  tatsächlich aktiviert und migriert hat, denn einer, bei dem das nicht der
  Fall ist, erhöht nichts. Unter `CoherenceMode::Lease` holt ein
  veraltet-aber-gespeicherter Eintrag innerhalb von `max_age_ms` von selbst
  auf, statt sofort. Führen Sie für alles andere
  `render-cache:epoch-advance` aus (pro Knoten, siehe den letzten Punkt).
- **Eine Seite, von der Sie Caching erwartet haben, trägt nie einen
  `Age`-Header.** Sie wird abgelehnt, sie scheitert nicht. Lesen Sie zuerst
  das `reason`-Label des `declined`-Lookups - es benennt den genauen
  Vertrag, der das Rendering tatsächlich verweigert hat, aus der
  geschlossenen Menge unter „Telemetrie" oben - und arbeiten Sie dann, für
  einen der klassifizierungsbedingten Gründe, die Klassifizierungsliste in
  [RenderCache](render-cache.md) durch: ein Sitzungslesen, ein
  Identitätslesen auf einer Route ohne `Principal`-Varianz, ein Locale-Lesen
  ohne `Locale`-Varianz, eine Autorisierungsprüfung oder ein Lesen mit rohem
  SQL.
- **Ein Backend ist nicht erreichbar.** `RENDER_CACHE_FAILURE` entscheidet:
  `open` (der Standard) bedient die Route ungecacht, `closed` antwortet mit
  einem nackten `503`. Ein Backend, das beim Boot fehlt, stoppt stattdessen
  den Boot, mit einem Satz, der die Migration oder die zu korrigierende
  Variable benennt.
- **Redis wurde geleert oder neu gestartet.** Einträge schlagen fehl und
  werden neu gerendert. Nichts Veraltetes kann als aktuell nachgewiesen
  werden: Die Aktualität wird gegen das Generations-Ledger der Datenbank
  nachgewiesen, nie gegen die Ebene, die die Bytes gehalten hat.
- **Ein Neuaufbau-Leader ist mitten im Neuaufbau gestorben.** Seine Lease
  wird übernommen, sobald die Store-Zeit den Ablauf überschreitet, und die
  eigene Veröffentlichung des früheren Leaders wird per Fence ausgeschlossen,
  statt gegen die neue zu rennen. Er veröffentlicht nichts; die Antwort
  seiner Anfrage wird trotzdem ausgeliefert.
- **Eine L1-Datei wurde durch einen Absturz oder eine volle Festplatte
  zerrissen.** Nichts liefert sie aus. Jede Datei trägt eine Prüfsumme über
  ihren eigenen Frame, sodass eine abgeschnittene oder veränderte Datei diese
  Prüfung nicht besteht und ein Fehltreffer ist; der Store entfernt sie, und
  jede übrig gebliebene temporäre Datei, beim nächsten Öffnen. Ein
  zerrissener Schreibvorgang heilt sich hier selbst, statt ein dauerhaft
  vergifteter Eintrag zu sein.
- **Die Datenbank wurde aus einer Sicherung wiederhergestellt.** Dieser Fall
  hat ein Verfahren statt eines Satzes; siehe „Die Datenbank
  wiederherstellen“ unten.
- **Sie brauchen alles weg, sofort.** `render-cache:epoch-advance`. Bei mehr
  als einem Knoten führen Sie ihn auf jedem aus, oder starten Sie die neu,
  auf denen Sie ihn nicht ausgeführt haben: Das Vorrücken bewegt die Epoche
  des Ledgers für die ganze Bereitstellung, doch es leert L0 und gibt die
  verleaste Epoche nur in dem Prozess ab, der es ausgeführt hat. Das
  Wiederherstellungsverfahren unten legt dar, warum.

## Die Datenbank wiederherstellen

Das Generations-Ledger ist die Autorität, gegen die jeder Treffer
nachgewiesen wird, sodass die Wiederherstellung der Datenbank ändert, was
„aktuell“ für jeden bereits gespeicherten Eintrag bedeutet. Zwei Dinge
entscheiden, was ein gespeicherter Eintrag als Nächstes tut, und keins von
beiden lautet „er wird still verworfen“.

**Das wird für Sie erledigt.** Das erste Autoritätslesen nach der
Wiederherstellung, das auf eine Epoche oder einen Eintrag trifft, die bzw.
der über dem wiederhergestellten Wert gestempelt ist, verweigert diesen
Eintrag rundweg - nicht einmal unter `Warning` ausgeliefert, in keinem
Alter, was auch immer die Frische-Richtlinie der Route sagt -, baut ihn neu
auf, hebt die Epoche des Ledgers auf eins über den höchsten gesehenen
Stempel an, ersetzt die Epochen-Lease dieses Knotens durch den angehobenen
Wert und leert das L0 dieses Knotens. Jeder andere Knoten sieht die
angehobene Epoche bei seinem eigenen nächsten Autoritätslesen: sofort unter
`CoherenceMode::Authority` und innerhalb von `max_age_ms` unter
`CoherenceMode::Lease`. `suprnova.render_cache.epoch_rewinds` zählt jede
Erkennung.

Das ist bereits alles, und es ist dieselbe Konvergenz, die der
Betreiberbefehl `render-cache:epoch-advance` erzeugt, nur ohne den
Betreiber erreicht. Die drei Tests `an_epoch_advanced_by_another_node_*` in
`framework/tests/render_cache/middleware.rs` messen die Ausbreitungsgrenze,
und `a_rewound_epoch_refuses_the_entry_rebuilds_and_lifts` misst die
Verweigerung.

**Ein optionaler Schritt bleibt.** Leeren Sie die geteilte L1-Ebene, wenn
eine Route mit einem Veraltet-auslieferbar-Fenster keine Repräsentation von
vor der Wiederherstellung auch nur einmal vor ihrem Neuaufbau ausliefern
darf. Erst das Anheben macht das erreichbar: Ein L1-Eintrag, der *unter* der
angehobenen Epoche gestempelt ist, ist wieder ein gewöhnlicher bewegter
Eintrag, und ein bewegter Eintrag auf einer solchen Route wird einmal unter
`Warning` ausgeliefert, während der Neuaufbau hinter der Anfrage läuft.
Löschen Sie den Inhalt des Verzeichnisses der Datei-Ebene, `DELETE FROM
suprnova_render_entries`, oder löschen Sie die Redis-Schlüssel, die auf
`<prefix>entry:*` passen, je nachdem, welche Ebene das Profil konfiguriert.
Lassen Sie es aus, und der schlimmste Fall ist ein `Warning`-markierter
Body von vor der Wiederherstellung pro solchem Schlüssel.

## Es messen

RenderCache bringt zwei Benchmarks mit, und sie sind **On-Demand-Werkzeuge,
nie Gate-Schritte**:

```bash
crates/suprnova-live/scripts/run-render-cache-budget.sh
```

Das führt den Engine-Bench aus (`render_cache_budget`, die Messungen für
heiße Treffer und zusammengesetzte Zusammenbauten mit einem zählenden
Allokator), dann den Workload-Bench des Frameworks (`render_cache_workloads`,
dieselbe Route durch die ganze Middleware), dann den Vertragstest über die
eingecheckten Ergebnisse. Beide sind auf `SUPRNOVA_LIVE_S1_CPUSET` gepinnt.

Ein vollständiger Lauf braucht ein wegwerfbares PostgreSQL (`PG_TEST_URL`)
und ein wegwerfbares Redis (`REDIS_TEST_URL`), denn der Vertrag über die
eingecheckten Ergebnisse verlangt alle drei aufgezeichneten Profile. **Ein
Teillauf muss beide Ergebnisdateien umleiten**, mit
`SUPRNOVA_LIVE_BENCH_RESULT` und `SUPRNOVA_LIVE_WORKLOADS_RESULT` unter
`benchmarks/local/`; ohne das überschreibt er die eingecheckten Ergebnisse
mit einer kürzeren Datei und scheitert dann an seinem eigenen Vertrag.

Die eingecheckten Zahlen, aus
`crates/suprnova-live/benchmarks/render-cache-budget-v1.json` und
`render-cache-workloads-v1.json`:

| Messung | Wert |
|---|---|
| Engine-Arbeit für einen frischen `Complete`-L0-Treffer, p95 | 0,76 Mikrosekunden |
| Heap-Allokationen, frischer Treffer | 3 |
| Heap-Allokationen, bedingter `304`-Treffer | 3 |
| Heap-Allokationen, durch eine Seed-Frist begrenzter Treffer | 4 |
| Rumpfkopien bei irgendeinem davon | keine; der Puffer wird geteilt |
| Dieselbe Route durch die Middleware, serverseitig, p95 | 14,4 Mikrosekunden |
| Dieselbe Anfrage über einen HTTP-Roundtrip auf dem Loopback, p95 | 109 Mikrosekunden |
| SQL-Anweisungen pro heißem Treffer (Lease-Modus) | 0 |

Die Middleware-Zahlen gelten für einen Rumpf von 65.536 Bytes, dessen
Rendering 12 Zeilen gelesen hat, aufgezeichnet als 14 beobachtete
Abhängigkeitsidentitäten.

**Lesen Sie diese als exploratorisch und nicht als qualifizierte Belege.**
Jedes eingecheckte Ergebnis trägt `"classification": "local_exploratory"` und
`"s1_requirements_met": false`: Sie wurden auf einer
Entwickler-Workstation mit geteilter CPU, einem `powersave`-Governor und
Loopback-Providern erzeugt. Sie sind nützlich, um eine Regression über eine
ganze Größenordnung zu erwischen, und für nichts Feineres. Eine Zahl ist nur
dann ein qualifizierter Beleg, wenn sie auf dem dedizierten Runner mit
gesetzter Attestierung erzeugt wurde, und diese wurden es nicht.

### Warum Suprnova abweicht

Laravels Response-Caching-Pakete überlassen den Betrieb dem Cache-Store
darunter. Einen Eintrag zu inspizieren heißt, seinen Schlüssel von Hand zu
finden und den Wert zu lesen - und das ist die gerenderte Seite, ihn
anzusehen heißt also, jemandes HTML auf einem Terminal auszugeben -, und
alles zu invalidieren heißt, einen Store zu leeren, der auch Ihre Sitzungen,
Ihre Ratenbegrenzungen und Ihre Queue hält. Die Beobachtbarkeit ist das, was
der Store-Treiber zufällig ausgibt.

Suprnova gibt dem Cache eine eigene Betriebsoberfläche, bewusst schmal. Die
Inspektion ist von Bauart her rumpffrei, sodass ein Betreiber bestätigen
kann, dass ein Eintrag existiert, unter welcher Klasse er gespeichert ist und
wie groß er ist, ohne je seinen Inhalt gezeigt zu bekommen. Die
Invalidierung ist ein Epochensprung, der nichts kostet und nur diesen Cache
berührt: Ihre Sitzungen und Ihre Queue liegen nicht im Wirkungsradius. Die
Telemetrie ist eine geschlossene Menge von sieben Zählern mit geschlossenen
Attributmengen, und das macht ein Dashboard darüber über Releases hinweg
stabil statt zu einem Satz driftender Zeichenketten. Der Preis ist, dass es
keinen Befehl „lösche genau diesen einen Schlüssel“ gibt: Die Hebel sind pro
Eintrag nur lesend oder epochenweit.

## Nächste Schritte

- [RenderCache](render-cache.md) - die Deklarationen, auf denen diese Befehle
  arbeiten
- [Beobachtbarkeit](observability.md) - wohin die Zähler oben exportiert
  werden
- [Testen](testing.md) - die umgebenden Testkonventionen, in denen die Muster
  oben sitzen
- [Bereitstellung](deployment.md) - die Produktions-Checkliste darum herum

# RenderCache Bereitstellung

Ein Prozess, der für sich selbst cacht, braucht nichts außer Speicher.
Mehrere Prozesse hinter einem Load Balancer müssen sich darüber einig sein,
was gespeichert ist, wer einen Eintrag neu aufbauen darf und wann etwas
aufgehört hat, wahr zu sein - und sie müssen sich einig werden, ohne dass
einer von ihnen die anderen davon überzeugen könnte, dass veralteter Inhalt
aktuell ist. RenderCache beantwortet das mit **Profilen**: Ein Profil
benennt, welche Provider ein Prozess baut, und sonst ändert sich nichts.
Routendeklarationen, Richtlinien, Schlüssel, der Sammler, der
Middleware-Fluss und das Zusammensetzen sind in jedem Profil identisch, und
kein anwendungsseitiger Typ unterscheidet sich zwischen ihnen.

Dieses Kapitel zeigt, wie Sie eines auswählen und verdrahten. Die drei
Profile und was jedes bereitstellt, die Umgebungsvariablen, die die
Framework-Konfiguration tatsächlich liest, die Migration, die Ihre Anwendung
auflisten muss, bevor ein geteiltes Profil bootet, wohin die Installation in
Ihrem Bootstrap gehört und was die Ebenen versprechen und was nicht. Die
Dogfood-Anwendung in diesem Repository läuft standardmäßig auf dem
eingebetteten Profil und wird von
`the_database_profile_serves_a_hit_through_the_sql_stores` in
`app/tests/live_render_cache.rs` auf dem Datenbankprofil gestartet.

## Drei Profile

| Profil | L1-Einträge | Neuaufbau-Führung | Live-Instanzdatensätze |
|---|---|---|---|
| `embedded` (Standard) | eine Datei pro Schlüssel, oder keine | im Prozess | im Prozess |
| `database` | `suprnova_render_entries` | `suprnova_render_leases` | `suprnova_live_instances`, `suprnova_live_promotions` |
| `redis` | ein Redis-Hash pro Schlüssel | ein Redis-Hash pro Schlüssel, plus ein Token-Zähler | ein Redis-Hash pro Datensatz |

Was sich zwischen ihnen **nicht** bewegt, ist die Generationswahrheit. Das
datenbankgestützte Generations-Ledger ist in jedem Profil die Autorität:
Welche Ebene die Bytes auch übergeben hat, die Aktualität wird gegen die
Datenbank nachgewiesen - indem sie beim Treffer unter
`CoherenceMode::Authority` erneut gelesen wird oder durch eine
Validierungs-Lease, die aus einem früheren Lesen unter
`CoherenceMode::Lease` gewährt wurde. Genau das lässt einen Beschleuniger
einen Beschleuniger bleiben: Redis kann alles verlieren, was es hält, ohne
dass irgendetwas Veraltetes als aktuell nachgewiesen würde, denn nichts, was
Redis hält, weist überhaupt Aktualität nach.

Wählen Sie danach, was Sie tatsächlich teilen müssen:

- **`embedded`** für einen einzelnen Prozess und für mehrere Prozesse, die
  gern jeder ihre eigene Kopie halten. Setzen Sie `RENDER_CACHE_L1_DIR`, und
  jeder Prozess gewinnt eine Datei-Ebene, die seinen eigenen Neustart
  übersteht.
- **`database`**, wenn mehrere Knoten sich gespeicherte Einträge teilen und
  pro Schlüssel einen Neuaufbau-Leader wählen sollen und Sie der
  Bereitstellung lieber kein weiteres bewegliches Teil hinzufügen möchten.
- **`redis`**, wenn die Latenz der geteilten Ebene wichtiger ist als ihre
  Dauerhaftigkeit, während die Datenbank darunter weiterhin die
  Generationswahrheit hält.

## Die Umgebungsvariablen

`RenderCacheConfig::from_env` liest diese, in
`framework/src/render_cache/config.rs`:

| Variable | Standard | Bedeutung |
|---|---|---|
| `RENDER_CACHE_ENABLED` | `true` | alles außer `false` oder `0`; `false` macht `RenderCache::install` zu einem No-op |
| `RENDER_CACHE_PROFILE` | `embedded` | `embedded`, `database` oder `redis`; setzt die beiden Zeilen darunter |
| `RENDER_CACHE_L1` | das des Profils | `disabled`, `file`, `database` oder `redis` |
| `RENDER_CACHE_COORDINATOR` | das des Profils | `local`, `database` oder `redis` |
| `RENDER_CACHE_L0_ENTRIES` | 4.096 | Obergrenze für Einträge im Prozess |
| `RENDER_CACHE_L0_BYTES` | 128 MiB | Obergrenze für Bytes im Prozess |
| `RENDER_CACHE_L1_DIR` | nicht gesetzt | das Verzeichnis der Datei-Ebene; unter `embedded` schaltet erst das Setzen L1 ein |
| `RENDER_CACHE_L1_BYTES` | 1 GiB | das ganze Verzeichnis für die Datei-Ebene, ein Eintrag für die Datenbank- und die Redis-Ebene |
| `RENDER_CACHE_REDIS_URL` | `REDIS_URL`, dann `redis://127.0.0.1:6379` | wohin sich beide Redis-Cache-Ebenen verbinden |
| `RENDER_CACHE_REDIS_PREFIX` | `suprnova_render:` | der Schlüsselnamensraum, unter dem beide Redis-Cache-Ebenen schreiben |
| `RENDER_CACHE_LEASE_MS` | 30.000 | Lebensdauer der Neuaufbau-Lease |
| `RENDER_CACHE_MAX_WAITERS` | 128 | Obergrenze für Wartende im Prozess |
| `RENDER_CACHE_FAILURE` | `open` | `open` bedient die Route bei einem Provider-Fehler ungecacht, `closed` antwortet mit `503` |
| `APP_BUILD_ID` | die Paketversion der Anwendung (siehe unten) | ordnet jeden Eintrag dem Build zu, der ihn erzeugt hat |

Das Profil ist eine Kurzschreibweise, keine Festlegung. `RENDER_CACHE_L1`
und `RENDER_CACHE_COORDINATOR` überschreiben jeweils ihre eigene Hälfte,
sodass eine Bereitstellung, die ihre Einträge in der Datenbank, ihre
Neuaufbau-Leases aber im Prozess haben möchte, genau das sagt, statt das
nächstgelegene ganze Profil zu wählen.

Eine Variable mit einer geschlossenen Menge akzeptierter Werte, die auf
etwas außerhalb davon gesetzt ist, lässt den Boot mit einer Meldung
scheitern, die die Variable benennt. Der abgelehnte Wert wird in dieser
Meldung nie wiederholt, denn ein Umgebungswert kann ein Geheimnis tragen.

**Setzen Sie `APP_BUILD_ID` ausdrücklich, einmal pro Deployment.** Sie geht
in jeden Lookup-Schlüssel ein, sodass ihre Änderung das ist, was einen neuen
Build daran hindert, Einträge auszuliefern, die der vorherige veröffentlicht
hat. Fehlt die Variable, greift `RenderCacheConfig::from_env` auf die eigene
Paketversion Ihrer Anwendung zurück: `#[suprnova::main]` zeichnet
`CARGO_PKG_VERSION` aus der Kompilierung der Anwendungs-Crate selbst auf, in
dem Moment, in dem es die Umgebung lädt, und dieser aufgezeichnete Wert ist
es, worauf der Standardwert hier zurückfällt. Nur ein Binary, das
`#[suprnova::main]` nie expandiert, fällt noch weiter zurück, auf die
Version dieser **Framework**-Crate selbst - so benannt, weil sie sonst
leicht mit der der Anwendung verwechselt wird. So oder so bewegt sich der
Wert nur, wenn jemand eine Versionsnummer erhöht, und eine Paketversion
ändert sich selten pro Deployment: Ein Deployment, das ein Template, eine
Übersetzung oder einen Handler ohne Versionserhöhung ändert, behält
dieselbe Build-Id und kann Einträge ausliefern, die der vorherige Build
veröffentlicht hat. Setzen Sie sie auf etwas, das sich bei jeder
Auslieferung ändert, etwa eine Commit-Id oder eine Release-Kennung:

```bash
APP_BUILD_ID=$(git rev-parse --short HEAD)
```

Eine Installation, die die Umgebung nie liest, setzt denselben Wert im Code
mit `RenderCacheConfig::with_build_id`, was überschreibt, wofür sich
`from_env` auch immer entschieden hat - ein ausdrückliches `APP_BUILD_ID`
eingeschlossen - für eine Anwendung, die ihre eigene Kennung pro Deployment
programmatisch ableitet.

Welchen Wert Sie auch setzen, das Produktions-Binary, das ihn liest, wird
in Suprnovas [Produktions-Build-Form](deployment.md#production-build-shape)
gebaut erwartet - Standard-Features aus, `testing` allein `cargo test`
vorbehalten.

Das eigene Instanz-Ledger von Live wird getrennt konfiguriert, denn es ist
die Autorität von Live und nicht der Speicher des Cache:
`LIVE_LEDGER_DRIVER` (`memory`, `database` oder `redis`), `LIVE_REDIS_URL`
und `LIVE_REDIS_PREFIX`. Eine Bereitstellung darf den Cache auf einer Ebene
und das Ledger auf einer anderen betreiben.

## Die Migration, die Ihre Anwendung auflisten muss

Das Schema von RenderCache gehört dem Framework und wird von der Anwendung
angewendet. Ihr `Migrator` listet es auf, sodass `suprnova migrate` die
Tabellen zusammen mit Ihren eigenen bereitstellt:

```rust
Box::new(suprnova::render_cache::migration::Migration),
Box::new(suprnova::render_cache::migration::TierMigration),
```

Das ist `app/src/migrations/mod.rs` wortwörtlich, und die beiden sind nicht
austauschbar:

- **`Migration`** legt die drei `suprnova_render_*`-Tabellen an, die die
  dauerhafte Generationswahrheit halten: aktuelle Generationen, ein nur
  anhängendes Änderungsprotokoll und die Autoritätsepoche. Jedes Profil
  braucht sie, auch `embedded`, denn die Generationswahrheit wandert nie in
  eine Cache-Ebene.
- **`TierMigration`** legt die vier Tabellen an, die der Datenbank-L1-Store
  und der Datenbank-Neuaufbau-Koordinator lesen. Nur ein Profil, das sie
  erreicht, braucht sie - aber `RenderCache::install` verweigert den Boot des
  Datenbankprofils ohne sie, und deshalb macht erst ihr Auflisten
  `RENDER_CACHE_PROFILE=database` zu einer Konfigurationsentscheidung, die
  Ihre Anwendung wirklich treffen kann.

Eine Anwendung, die `RENDER_CACHE_ENABLED=false` setzt, muss keine von
beiden mitführen: Die Installation gibt den Router unangetastet zurück,
sondiert nichts, baut keine Laufzeit zusammen, registriert keine Middleware
und lässt die Schreibseite uninstrumentiert, sodass nichts für einen Cache
bezahlt, der aus ist.

## Die Installation

`RenderCache::install` ist asynchron, weil es nach den Tabellen sondiert und
jeden verschiedenen Redis-Endpunkt anpingt, den die Konfiguration verwenden
würde, bevor es irgendetwas zusammenbaut. `Application::try_routes_async`
ist der Haken, der es beherbergt. Dies ist `app/src/live/mod.rs`, und die
Aufteilung in zwei Funktionen ist es wert, kopiert zu werden:

```rust
/// [`routes`] followed by the RenderCache middleware. This is the entry
/// point every server in this application uses.
pub async fn routes_with_render_cache(router: Router) -> Result<Router, FrameworkError> {
    routes_with_render_cache_with_config(router, RenderCacheConfig::from_env()?).await
}

#[doc(hidden)]
pub async fn routes_with_render_cache_with_config(
    router: Router,
    config: RenderCacheConfig,
) -> Result<Router, FrameworkError> {
    RenderCache::install(routes(router)?, config).await
}
```

`routes` ist die synchrone innere Hälfte: Sie registriert die reservierten
Live-Routen, die Dokumentrouten und jede Cache-Richtlinie und installiert
keine Middleware. `cmd/main.rs` erreicht `routes_with_render_cache` über
`Application::try_routes_async`, und der Server des Browser-Szenarios in
`app/examples/live_dogfood_host.rs` wartet direkt darauf.

Die Konfigurationsnaht darunter ist keine Zierde. Ein Test, der ein anderes
Profil oder eine bewegbare Uhr braucht, hat keinen anderen Weg hinein, und es
zählt, dass er *dieselben* Routen, Richtlinien und dieselbe
Middleware-Reihenfolge installiert wie der Server und sich nur in der
übergebenen Konfiguration unterscheidet. Beide Dogfood-Starts gehen darüber:
`the_database_profile_serves_a_hit_through_the_sql_stores` übergibt eine
Konfiguration für das Datenbankprofil, und
`stale_service_is_marked_and_rebuilt_in_the_background` übergibt eine, die
eine verstellbare Uhr trägt. Siehe „Eine gecachte Route testen“ in
[RenderCache Betrieb](render-cache-operations.md).

Zwei Reihenfolgeregeln, beide in der Verantwortung der aufrufenden Seite:

1. Jede Route und jede Gruppe muss **vor** `install` aufgenommen werden, das
   liest, was bis zu diesem Zeitpunkt registriert wurde.
2. `install` hängt an die globale Middleware-Kette an, muss also **nach** der
   Sitzungs-, Locale- und Identitäts-Middleware laufen, deren
   anfragegebundenen Zustand die Cache-Middleware beim Ableiten eines
   Lookup-Schlüssels liest.

Die Installation scheitert geschlossen, über zwei Sonden. Sie prüft, dass die
Tabellen existieren, die die Konfiguration erreichen würde, und sie pingt
jeden verschiedenen Redis-Endpunkt an, den die Konfiguration verwenden würde,
einmal pro Endpunkt. Scheitert eines von beidem, stoppt der Boot mit einem
umsetzbaren Satz, der die Migration oder die zu korrigierende Variable
benennt, sodass nie etwas gegen eine fehlende Tabelle oder einen Endpunkt
ausgeliefert wird, der nicht antwortet. (Das eigene Instanz-Ledger von Live
wird getrennt sondiert, durch `Server::run`, bevor irgendeine Anfrage
bedient wird.)

## Wählen, wo die Einträge einer Route liegen

Das Profil entscheidet, was L1 *ist*; die Richtlinie entscheidet, welche
Routen es verwenden. Der Builder speichert nur in L0, sofern eine Route
nichts anderes deklariert, sodass eine geteilte Ebene absichtlich gefüllt
wird:

```rust
RenderCachePolicy::builder(RepresentationClass::PublicShared)
    .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
    .layers(StorageLayers::l0_and_l1())
    .build()?
```

`the_database_profile_serves_a_hit_through_the_sql_stores` weist den
Rundweg von Ende zu Ende auf dem Datenbankprofil nach: Der veröffentlichte
Eintrag wird unter dem Schlüssel, den die Middleware abgeleitet hat, wieder
aus dem SQL-Store gelesen, dann wird L0 geleert, und die nächste Anfrage wird
immer noch ohne ein Rendering beantwortet. Siehe
[RenderCache Repräsentationen](render-cache-representations.md) dazu, wie
Sie pro Route entscheiden.

## Eine Konformitätssuite, jeder Provider

Jeder Store beantwortet dieselbe Suite.
`framework/tests/render_cache/store_conformance.rs` führt die
Provider-Szenarien der Engine - allein gegen den Trait `RenderStore`
geschrieben - über das dateigestützte L1, das SQL-L1 auf SQLite, PostgreSQL
und MySQL sowie das Redis-L1 aus, und der Store im Prozess beantwortet
dieselbe Suite in der Engine-Crate. PostgreSQL, MySQL und Redis laufen über
ignorierte Tests, die `scripts/check-postgres.sh`, `scripts/check-mysql.sh`
und `scripts/check-redis.sh` namentlich gegen echte Server auswählen. Ein
Provider ist hier nicht „unterstützt“, weil es ihn gibt; er ist unterstützt,
weil er dieselben Worte besteht wie jeder andere.

## Was die Ebenen versprechen und was nicht

- **Kein knotenübergreifendes Warten.** Ein Schlüssel, den ein anderer
  Knoten bereits neu aufbaut, ist ein Bypass: Dieser Knoten rendert und
  veröffentlicht nichts. Begrenzte doppelte Berechnung über Knoten hinweg
  wird akzeptiert; zwei angenommene Veröffentlichungen nicht, und der eigene
  Veröffentlichungs-Fence des Stores ist es, der die zweite verbietet.
- **Ein verlorenes Backend ist ein Fehltreffer, nie eine falsche Antwort.**
  Verdrängung, Ablauf oder ein Redis-Neustart lassen Einträge fehlschlagen
  und Instanzen fehlen. Die Kohärenzprüfung gegen das Generations-Ledger der
  Datenbank läuft bei jedem Treffer, was auch immer die Bytes geliefert hat.
- **Manipulierte Bytes sind ein Fehltreffer.** Eintrags-Bytes in einer Zeile
  oder einem Hash sind ein signierter Codec-Frame, sodass ein zerrissener,
  abgeschnittener oder veränderter Wert seine Integritätsprüfung nicht
  besteht und als Fehltreffer behandelt statt ausgeliefert wird.
- **Die Datenbank- und die Redis-Ebene verdrängen nichts, um Platz zu
  schaffen.** `RENDER_CACHE_L1_BYTES` begrenzt dort einen Eintrag, nicht die
  Tabelle oder den Schlüsselraum; das Wachstum wird stattdessen durch die
  Aufbewahrung begrenzt. Nur die Datei-Ebene begrenzt ein ganzes
  Verzeichnis, weil sie dieses Verzeichnis allein besitzt.
- **Die Redis-Adapter zielen auf eine einzelne Redis-Instanz ab Version 7.**
  Redis Cluster wird abgelehnt: Die Skripte berühren Schlüssel, die sie nicht
  deklarieren, und sie lesen die Uhr des Stores mit `TIME` innerhalb eines
  Skripts.
- **MySQL braucht 8.0.19 oder neuer** für die genaue Klassifizierung
  doppelter Schlüssel. Ältere MySQL-Versionen und MariaDB melden eine
  Kollision, die dieser Build keiner Tabelle zuordnen wird, sodass er zu
  einem Provider-nicht-verfügbar-Fehler herabstuft - die sichere Richtung,
  und so oder so wird nichts zweimal gewährt.
- **Jede knotenübergreifende Ablaufentscheidung wird auf der Uhr des
  Backends getroffen**, gelesen innerhalb der Operation, die darauf handelt.
  Ein Knoten, dessen Uhr vorgeht, kann weder eine Lease verlängern noch
  einen lebenden Eintrag vor seinen Nachbarn verbergen noch den Datensatz
  eines Nachbarn für abgelaufen erklären.

### Warum Suprnova abweicht

Laravels Response-Caching-Pakete erben den Cache-Store, den Sie bereits
konfiguriert haben, sodass „den Cache über mehrere Knoten bereitstellen“
heißt, `CACHE_STORE` auf Redis zu richten und darauf zu vertrauen, dass
richtig ist, was dort liegt. Es gibt keinen eigenen Begriff davon, wer einen
Eintrag neu aufbauen darf, keinen Fence, der zwei Worker daran hindert,
widersprüchliche Bytes für denselben Schlüssel zu veröffentlichen, und, am
folgenreichsten, keine Autorität unter dem Store. Hält Redis eine Seite, wird
die Seite ausgeliefert; wird Redis geleert, wird alles neu berechnet. Der
Store *ist* die Wahrheit.

Suprnova trennt beides bewusst. Die geteilte Ebene hält Bytes und sonst
nichts; die Datenbank hält in jedem Profil die Generationswahrheit, und
gegen sie wird ein Treffer geprüft. Deshalb kostet der Verlust von Redis hier
Latenz statt Korrektheit, deshalb wird ein Neuaufbau verleast und eine
Veröffentlichung mit einem Fence versehen, statt um sie zu rennen, und
deshalb laufen dieselben Routendeklarationen unverändert von `cargo run` auf
einem Laptop bis zu einer datenbankkoordinierten Flotte. Der Preis ist eine
Migration, die Ihre Anwendung auflisten muss, und ein Datenbanklesen auf dem
Trefferpfad, das ein einfacher Schlüssel-Wert-Cache nicht zahlt: eine
Anweisung, oder null unter einer Validierungs-Lease. Siehe
[RenderCache Generationen](render-cache-generations.md) dazu, was dieses
Lesen einbringt.

## Nächste Schritte

- [RenderCache Betrieb](render-cache-operations.md) - die Konsolenbefehle,
  die Telemetrie, die Datenträgerhygiene und was zu tun ist, wenn etwas nicht
  stimmt
- [Bereitstellung](deployment.md) - die umgebende Produktions-Checkliste
- [Migrationen](migrations.md) - wie die `Migrator`-Liste oben angewendet
  wird

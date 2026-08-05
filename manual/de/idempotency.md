# Idempotenz

Wenn ein Client ein POST wiederholt, soll der zweite Aufruf sicher
sein. Das Netzwerk ist unzuverlässig, und Clients wiederholen - aber
`POST /charges` darf die Karte niemals doppelt belasten, und `POST
/orders` darf für einen Klick niemals zwei Bestellungen erzeugen.
Idempotenzschlüssel sind der Vertrag, der sagt: „Wenn Sie diesen
Schlüssel noch einmal sehen, geben Sie mir die ursprüngliche
Antwort; erledigen Sie die Arbeit nicht noch einmal.“

Suprnovas `Idempotency` ist eine dünne Facade über `Cache::lock`,
die Ihnen drei eskalierende Garantien gibt: Dedupe-only, Dedupe mit
Retry-bei-Fehlschlag, und Stripe-artiges Replay des Ergebnisses.
Alle drei halten die Lease der Sperre am Leben, solange der Rumpf
läuft, sodass ein langsamer Rumpf die Sperre nie ablaufen lassen
kann und kein Duplikat durchrutscht.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome: Idempotent<OrderId> = Idempotency::once(
    "create-order:user-42:client-key-abc",
    Duration::from_secs(86_400),
    || async {
        // Läuft pro Schlüssel genau einmal innerhalb des 24-Stunden-Fensters.
        place_order(&user, &cart).await
    },
)
.await?;

match outcome {
    Idempotent::Fresh(id) => /* erster Aufruf - id ist die neue Bestellung */ {},
    Idempotent::FreshUnfenced(id) => {
        // Die Bestellung wurde aufgegeben, aber die Lease der Sperre
        // ging auf halbem Weg verloren, sodass ein anderer Aufrufer
        // womöglich ebenfalls eine aufgegeben hat. Abgleichen oder
        // alarmieren - siehe „Wenn die Exklusivität verloren geht“ unten.
    },
    Idempotent::Duplicate => /* derselbe Schlüssel bereits verwendet */ {},
}
```

## Die drei Primitiven

| Methode | Rumpf läuft | Duplicate sieht | Fehlschlag gibt Sperre frei? | Verwenden, wenn |
|---|---|---|---|---|
| `Idempotency::once` | genau einmal pro Fenster | `Duplicate`-Marker | nein | Seiteneffekte NIEMALS wiederholt werden dürfen (Mail versendet, Charge versucht) |
| `Idempotency::commit_on_success` | einmal pro Erfolg pro Fenster | `Duplicate`-Marker | ja | transiente Fehlschläge wiederholbar sein sollten, aber ein Erfolg bindend bleibt |
| `Idempotency::remember` | einmal pro Erfolg pro Fenster | der ursprüngliche Rückgabewert | ja | Duplikate den ursprünglichen Payload erhalten müssen, nicht einen Marker |

Alle drei leben unter `suprnova::idempotency` und werden von der
Crate-Wurzel als `Idempotency`, `Idempotent` und `Replay`
re-exportiert. Sie teilen sich dieselbe Schlüssel-Hashing-,
Lease-Erneuerungs- und Sperren-Semantik - nur die
Erfolg-/Fehlschlag-Policy unterscheidet sich.

### `Idempotency::once` - at-most-once

Der strengste Vertrag. Der erste Aufrufer im TTL-Fenster lässt den
Rumpf laufen und bekommt `Fresh(value)`. Jeder nachfolgende
Aufrufer innerhalb des Fensters bekommt `Duplicate`, und der Rumpf
läuft NICHT erneut - selbst wenn der Rumpf des ersten Aufrufers
`Err` zurückgegeben hat. Die TTL IST das Dedupe-Fenster.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

// Sendet eine Willkommens-Mail genau einmal pro Signup, egal
// wie oft der Signup-Callback wiederholt.
let result = Idempotency::once(
    &format!("welcome-mail:{}", user.id),
    Duration::from_secs(7 * 24 * 3600),
    || async {
        Mail::to(&user.email).send(WelcomeMail { user: user.clone() }).await
    },
)
.await?;
```

Greifen Sie zu `once`, wenn der Seiteneffekt von der Art ist, bei
der gilt: „Ich habe es versucht; selbst wenn ich nach dem
Seiteneffekt einen Fehler hatte, nicht noch einmal versuchen“ -
eine E-Mail versenden, an eine externe API posten, die ihre eigenen
Idempotenzschlüssel nicht honoriert, einen Audit-Log-Eintrag
schreiben, dessen doppeltes Schreiben nachgelagerte Analytics
korrumpieren würde.

### `Idempotency::commit_on_success` - at-least-once bei Erfolg, Wiederholung bei Fehlschlag

Wie `once`, aber wenn der Rumpf `Err` zurückgibt, wird die
Dedupe-Sperre freigegeben, sodass der nächste Aufrufer innerhalb
des TTL-Fensters es erneut versuchen kann. Ein erfolgreicher Rumpf
behält die Sperre für den Rest des Fensters.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome = Idempotency::commit_on_success(
    &format!("publish-post:{}", post.id),
    Duration::from_secs(300),
    || async {
        // Postet eine Nachricht an einen vorgelagerten Service.
        // Netzwerkfehler sind transient - die nächste Wiederholung
        // sollte erneut eintreten, statt „schon erledigt“ zu hören,
        // wenn tatsächlich nichts passiert ist.
        social_media_client.post(&post).await
    },
)
.await?;
```

Verwenden Sie `commit_on_success`, wenn der Rumpf wiederholbare
Fehlermodi hat (transiente Netzwerkfehler, vorgelagerte
Rate-Limits, abgelaufene Credentials, die ein Refresh beheben
würde) und Sie at-least-once bei Erfolg wollen, aber die Sperre bei
einem Fehlschlag aufgeben soll, damit eine Wiederholung erneut
eintreten kann.

### `Idempotency::remember` - Stripe-artiges Replay des Ergebnisses

Der Vertrag, für den der HTTP-Header `Idempotency-Key` erfunden
wurde. Der erste Aufrufer lässt den Rumpf laufen, speichert den
Erfolgswert und bekommt `Replay::Fresh`. Ein späterer Aufrufer
innerhalb des Fensters bekommt `Replay::Replayed(<ursprünglicher
Wert>)` - den aufgezeichneten Rückgabewert, keinen Marker. Ein
gleichzeitiger Aufrufer, der eintrifft, *während* der erste noch
läuft, bekommt `Replay::InProgress`.

```rust
use std::time::Duration;
use suprnova::{
    handler, Auth, FrameworkError, HttpResponse, Idempotency, Replay, Request, Response,
};

#[handler]
pub async fn create_charge(req: Request) -> Response {
    // Header vor dem Konsumieren von `req` für den Rumpf in einen eigenen String extrahieren.
    let key = req
        .header("Idempotency-Key")
        .ok_or_else(|| FrameworkError::bad_request("Idempotency-Key header required"))?
        .to_string();

    let user = Auth::user_as::<User>()
        .await?
        .ok_or_else(|| FrameworkError::unauthorized("login required"))?;

    let form: ChargeForm = req.json().await?;

    let outcome = Idempotency::remember(
        &format!("charge:{}:{}", user.id, key),
        Duration::from_secs(24 * 3600),
        || async {
            let charge = StripeClient::charge(&form).await?;
            Ok(ChargeResponse {
                id: charge.id,
                amount: charge.amount,
                status: charge.status,
            })
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(body) | Replay::Replayed(body) => {
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::FreshUnfenced(body) => {
            // Dieselbe Response an den Client, aber eine Metrik wert:
            // Exklusivität wurde nicht für den gesamten Rumpf gehalten.
            tracing::warn!("idempotent body completed unfenced");
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

Beachten Sie, dass `Fresh` und `Replayed` von der client-seitigen
Response identisch behandelt werden - der ganze Sinn von `remember`
ist, dass der zweite Aufrufer nicht unterscheiden kann, ob er
derjenige war, der den Rumpf ausgeführt hat, oder ob er das
aufgezeichnete Ergebnis bekommen hat.

`InProgress` ist der Fall, über den es sich nachzudenken lohnt: Ein
Duplikat traf ein, während der Rumpf des ersten Aufrufers noch
lief, sodass es noch kein aufgezeichnetes Ergebnis zum Zurückgeben
gibt. `409 Conflict` mit einem `Retry-After: 1`-Header ist die
kanonische Antwort - der Client wartet kurz, dann wiederholt er,
und der zweite Versuch konkurriert entweder mit dem Original um
den Short-Circuit bei `Cache::get`, oder er trifft auf `Replayed`.

## Schlüsselmaterial

Alle drei Methoden akzeptieren einen beliebigen `&str` als
Schlüssel. Bevor er das Cache-Backend berührt, wird der Schlüssel
SHA-256-gehasht zu einem 64-Zeichen-Hex-Digest. Das verschafft
Ihnen drei Dinge:

1. **Begrenzte Backend-Schlüssellänge.** Ein Client, der einen
   10-KB-`Idempotency-Key`-Header postet, erzeugt trotzdem nur
   einen 64-Byte-Cache-Key.
2. **Rohe Identifier lecken nicht in die Cache-Tools.** Enthält der
   Schlüssel eine E-Mail-Adresse, eine Session-ID oder eine interne
   User-ID, tauchen diese nicht in `redis-cli KEYS idem:*` auf.
3. **Keine Kollisionen zwischen Zeichenklassen.** Was auch immer
   das Cache-Backend speziell interpretiert (Doppelpunkte,
   Glob-Zeichen, Steuerbytes), ist bereits weg - der Hash ist nur
   hexadezimal.

Der Hash liegt über dem vom Nutzer angegebenen Schlüssel, nicht
über dem Cache-Key-Präfix - `Idempotency::once("k", …)` und
`Idempotency::once("k", …)` von zwei verschiedenen Call-Sites im
selben Prozess kollidieren absichtlich. Nutzen Sie einen eigenen
Namespace für Ihre Schlüssel, wenn Sie das nicht wollen:

```rust
Idempotency::once(
    &format!("billing:charge:{}:{}", tenant_id, client_key),
    Duration::from_secs(86_400),
    || async { /* … */ },
)
.await?;
```

## Lease-Erneuerung - das Problem des langsamen Rumpfs

Eine naive Kombination aus Sperre + TTL hat einen Fenster-Bug:
Läuft der Rumpf länger als die TTL, läuft die Sperre ab, während
der Rumpf noch läuft, und ein zweiter Aufrufer kann eine frische
Sperre erwerben und den Rumpf gleichzeitig erneut laufen lassen.
Der Dedupe-Vertrag bricht genau für die Operationen, die langsam
genug sind, um ihn zu brauchen.

Suprnova löst das, indem es einen Hintergrund-Task spawnt, der die
Sperre bei einem Drittel der TTL auffrischt (mit einer Untergrenze
von 50 ms), für die gesamte Dauer des Rumpfs. Ein `tokio::select!`
mit `biased`-Ordnung garantiert, dass der Rumpf-Zweig der einzige
ist, der die Future je auflöst.

Ein Refresh-*Fehler* wird nicht als verlorene Lease behandelt. Er
bedeutet, dass das Backend nicht gefragt werden konnte, nicht dass
jemand anders die Sperre übernommen hat, also versucht die
Erneuerung es beim nächsten Intervall erneut und gibt erst nach
mehreren aufeinanderfolgenden Fehlschlägen auf. Beim ersten
Aussetzer aufzugeben, garantierte, dass die Lease verfallen würde,
selbst wenn das Backend Millisekunden später wiederhergestellt
war.

### Wenn die Exklusivität verloren geht

Die Erneuerung kann trotzdem echt fehlschlagen: Das Token passt
nicht mehr, weil die Sperre abgelaufen ist und jemand anders sie
beansprucht hat. In diesem Moment können zwei Aufrufer denselben
Rumpf ausführen.

Der Rumpf wird **nicht** abgebrochen. Bis eine Lease verloren geht,
hat er möglicherweise bereits eine Karte belastet oder eine
Nachricht versendet, und ein Abbruch würde das halbfertig stranden,
ohne dass irgendetwas es aufzeichnet. Der Rumpf läuft bis zum Ende,
und der Verlust wird gemeldet:

| Ergebnis | Bedeutet |
|---|---|
| `Fresh(v)` / `Replay::Fresh(v)` | Rumpf lief, Exklusivität durchgehend gehalten |
| `FreshUnfenced(v)` | Rumpf lief und produzierte `v`, aber ein anderer Aufrufer könnte gleichzeitig gelaufen sein |

`FreshUnfenced` ist eine eigene Variante statt eines Flags auf
`Fresh`, gerade damit ein erschöpfender `match` sie nicht
versehentlich ignorieren kann. Was Sie damit tun, entscheiden Sie
selbst - abgleichen, alarmieren, kompensieren -, aber sie wie
`Fresh` zu behandeln, wirft das einzige Signal weg, das Ihnen sagt,
dass die Garantie nicht gehalten hat.

Eine Lease zu verlieren setzt voraus, dass das Backend für mehrere
Refresh-Intervalle unerreichbar ist, oder eine
Stop-the-World-Pause, die länger dauert als die TTL. Das ist
selten. Unmöglich ist es nicht, und früher war es unsichtbar.

Die praktische Konsequenz: Wählen Sie eine TTL basierend auf Ihrem
Dedupe-Fenster (`wie lange soll eine doppelte Anfrage dedupliziert
werden?`), nicht basierend auf der Worst-Case-Laufzeit des Rumpfs.
Ein 30-minütiger Rumpf mit einer 1-Minuten-TTL ist völlig in
Ordnung - die Sperre wird während des Laufs des Rumpfs etwa
neunzig Mal aufgefrischt.

Ein Test, der das ausübt: eine 200-ms-TTL mit einem Rumpf, der
500 ms lang blockiert, und ein zweiter Aufrufer, der bei 400 ms
eintrifft. Ohne Erneuerung würde der zweite Aufrufer den Rumpf
erneut ausführen. Mit Erneuerung sieht er `Duplicate`. Die Sperre
hält.

## Gemeinsam genutztes Backend

Prozessübergreifendes Dedupe braucht einen prozessübergreifenden
Cache. Das In-Memory-Backend hält Sperren in einer prozesslokalen
`HashMap`, sodass zwei `cargo run`-Instanzen auf derselben Maschine
die Idempotenzschlüssel des jeweils anderen nicht sehen.
Produktions-Deployments, bei denen etwas davon eine Rolle spielt -
mehrere App-Prozesse, horizontale Skalierung, Blue/Green-Deploys
mit überlappenden Traffic-Fenstern - müssen `CACHE_DRIVER=redis`
setzen und eine erreichbare `REDIS_URL` bereitstellen.

Das Bootstrap ist fail-closed: Ist `CACHE_DRIVER=redis` gesetzt und
Redis unerreichbar, weigert sich die App zu starten, statt still
auf prozesslokalen Memory herunterzustufen. Siehe
[cache.md](cache.md) für den vollständigen Vertrag des
Cache-Backends.

## Fehlerbehandlung

Der `FrameworkError` des Rumpfs pflanzt sich unverändert durch
`Idempotency` fort. Ein Fehlschlag beim Sperrenerwerb (Redis ist
mitten in der Anfrage down, das Backend liefert einen Fehler)
pflanzt sich als `FrameworkError` von der Cache-Schicht fort - es
gibt keinen stillen Fallback. Der Fehlertyp ist der
Standard-`FrameworkError` des Frameworks, sodass Handler ihn per
`?` zum Error-Converter ihres Controllers durchreichen können:

```rust
use std::time::Duration;
use suprnova::{handler, FrameworkError, HttpResponse, Idempotency, Replay, Response};

#[handler]
pub async fn handler(order_id: i64) -> Response {
    let outcome: Replay<MyDto> = Idempotency::remember(
        &format!("order:{order_id}"),
        Duration::from_secs(60),
        || async move {
            let row = MyRow::find(order_id)
                .await?
                .ok_or_else(|| FrameworkError::not_found("missing"))?;
            Ok(MyDto::from(row))
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(dto) | Replay::Replayed(dto) | Replay::FreshUnfenced(dto) => {
            let json = serde_json::to_value(&dto)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

Ein Freigabe-Fehlschlag auf dem `Err`-Pfad von `commit_on_success`
oder `remember` wird **protokolliert, nie zurückgegeben** - der
Fehler des Rumpfs ist der einzige Fehler, den der Aufrufer auf
diesem Pfad sieht. Eine fehlgeschlagene Freigabe bedeutet, dass die
Sperre hält, bis die TTL verstreicht; eine Wiederholung innerhalb
des Fensters sieht bis dahin `Duplicate` oder `InProgress`. Logs
enthalten den gehashten Schlüssel (nie das rohe Schlüsselmaterial),
damit Betreiber korrelieren können, ohne PII zu lecken.

## Abbruch

Droppt der Aufrufer die Future von `Idempotency::remember`, bevor
der Rumpf fertig ist, wird der Rumpf abgebrochen wie jeder andere
`tokio::select!`-Zweig - die Sperre wird **nicht** freigegeben, und
ein Duplikat, das vor Ablauf der TTL eintrifft, sieht `InProgress`
(danach, nach Ablauf der TTL, wieder `Fresh`). Das ist der sichere
Standard: Ein halbfertiger Rumpf, dessen Effekte Sie nicht kennen,
sollte nicht als sicher für eine Wiederholung angenommen werden.
Hüllen Sie Rümpfe, die unverwaltete Seiteneffekte halten, in
`tokio::spawn` und joinen Sie den Handle, wenn der Rumpf nicht
abbrechbar sein muss.

## Queue-Integration

Die Queue-Schicht verwendet intern `Idempotency::commit_on_success`,
um `Queue::push_unique` zu implementieren. Wenn Sie wollen, dass
ein Job höchstens einmal pro `Job::unique_for()`-Fenster pro
`Job::unique_id(&self)` eingereiht wird, müssen Sie
`Idempotency::*` nicht selbst aufrufen:

```rust
use suprnova::{Job, Queue};

let was_pushed = Queue::push_unique(SendReceipt { order_id: 42 }).await?;
if was_pushed {
    // Wir haben das Race gewonnen; der Job ist in der Queue.
} else {
    // Ein anderer Aufrufer hat das bereits eingereiht; als Erfolg behandeln.
}
```

Siehe [queues.md](queues.md) für den vollständigen
Job-Eindeutigkeits-Vertrag.

## Zahlungs-Webhook-Ingress

Der Payments-Webhook-Handler verwendet KEIN `Idempotency::*`.
Webhook-Ingress hat eine strengere Anforderung - jedes Event muss
auditierbar sein, selbst bei der ersten Zustellung, sodass die
Audit-Zeile die Quelle der Wahrheit ist und der Dedupe-Schlüssel
der Datenbank-Constraint `UNIQUE(provider, provider_event_id)` ist.
`Idempotency::remember` würde den Response-Payload im Cache
speichern; der Webhook-Handler speichert die *vollständige
Event-Envelope plus das Verarbeitungsergebnis* in
`payments_webhook_events`, was bedeutet, dass ein Betreiber Events
offline erneut abspielen oder erneut verarbeiten kann, indem er die
Tabelle liest.

Die beiden Muster ergänzen sich. Verwenden Sie `Idempotency::*` für
client-getriebene Schlüssel mit TTL-begrenztem Dedupe; verwenden
Sie eine `UNIQUE`-indizierte Audit-Tabelle für provider-getriebenen
Webhook-Ingress, der Auditierbarkeit über die Cache-TTL hinaus
braucht. Siehe [payments.md](payments.md) für den Webhook-Vertrag.

### Warum Suprnova abweicht

Laravels `Cache::lock` ist eine Primitive; der Stripe-artige
Idempotenz-Vertrag (das Ergebnis aufzeichnen, es abspielen,
In-Progress von Duplicate unterscheiden) bleibt ein
Userland-Rezept. Jedes Laravel-Projekt, das ihn braucht, endet
damit, denselben Sperren-und-Cache-Tanz zu schreiben, meist mit
einem dieser drei Bugs:

1. **Keine Lease-Erneuerung.** Ein Rumpf, der die TTL überlebt,
   wird bei einem doppelten Aufrufer gleichzeitig erneut
   ausgeführt. Die Sperre war da; sie ist nur im falschen Moment
   abgelaufen.
2. **Freigabe auf dem Erfolgspfad.** Die Sperre freizugeben, wenn
   der Rumpf erfolgreich ist, öffnet ein Fenster zwischen `body()
   -> Ok` und dem Erwerb einer frischen Sperre durch den nächsten
   Aufrufer - genau das Fenster, das Dedupe eigentlich schließen
   sollte.
3. **Rohe Schlüssel im Cache-Backend.** Vom Client mitgelieferte
   `Idempotency-Key`-Header gehen direkt in Redis-Schlüssel, lecken
   PII in Betreiber-Tools und erzeugen unbegrenzte Schlüsselgrößen.

Suprnova liefert das Rezept als erstklassige Primitive, sodass
jeder Aufrufer dieselbe Lease-Erneuerung, dieselbe fail-closed
Freigabe-Semantik, dieselbe Sicherheit durch gehashte Schlüssel
bekommt. Die drei Methoden (`once`, `commit_on_success`,
`remember`) benennen die drei Policies, zwischen denen Sie
tatsächlich wählen müssen - wählen Sie die, die zum Fehlermodell
Ihres Rumpfs passt, und machen Sie weiter.

## Testen

`Idempotency` löst sein `CacheStore` über den Container auf, sodass
Tests, die einen `InMemoryCache` binden, pro Test einen frischen,
isolierten Cache bekommen:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::cache::InMemoryCache;
use suprnova::cache::store::CacheStore;
use suprnova::container::testing::TestContainer;
use suprnova::idempotency::{Idempotency, Replay};

#[tokio::test]
async fn duplicate_remember_replays_the_first_result() {
    let _guard = TestContainer::fake();
    let store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_prefix("idem:"));
    TestContainer::bind::<dyn CacheStore>(store);

    let r1: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(7) },
    )
    .await
    .unwrap();
    assert_eq!(r1, Replay::Fresh(7));

    let r2: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(999) },
    )
    .await
    .unwrap();
    assert_eq!(r2, Replay::Replayed(7));
}
```

Suprnovas eigenes `framework/tests/idempotency.rs` deckt die
Vertrags-Oberfläche ab: Duplikat-Unterdrückung, TTL-Ablauf,
Freigabe-Policy je nach Fehler vs. Erfolg, Lease-Erneuerung über
Rumpf-Laufzeiten hinweg, die die TTL überleben, das
`InProgress`-Race und den Fall, in dem `release_lock` des Caches
selbst einen Fehler liefert. Lesen Sie diese Tests, wenn Sie das
exakte Verhalten sehen wollen, auf das Sie sich verlassen können.

## Fallstricke

- **`Idempotency::once` verbraucht das Fenster bei einem Fehler.**
  Ein fehlschlagender erster Aufrufer hält die Sperre trotzdem bis
  zum Ablauf der TTL. Verwenden Sie `commit_on_success`, wenn Sie
  Retries innerhalb des Fensters wollen.
- **`Idempotency::remember` speichert `T` im Cache-Backend.** Der
  Schlüssel wird gehasht, aber der *Payload* wird mit serde
  serialisiert und ins Backend geschrieben. Legen Sie keine
  Secrets in einen abgespielten Wert, der nicht in Ihrem
  Cache-Store erscheinen darf.
- **Zwei Prozesse brauchen einen gemeinsamen Cache.** In-Memory-
  Dedupe ist prozesslokal. Prozessübergreifende Korrektheit
  verlangt `CACHE_DRIVER=redis` (oder einen anderen
  prozessübergreifenden Store).
- **TTLs unter 150 ms sind nicht lease-getestet.** Die
  Erneuerungs-Untergrenze liegt bei 50 ms, sodass eine
  100-ms-TTL etwa alle 50 ms auffrischt - für den Vertrag in
  Ordnung, aber die Lease-Tests des Frameworks laufen bei `ttl >=
  1s`. Verwenden Sie realistische Dedupe-Fenster; ein in
  Millisekunden gemessenes Idempotenz-Fenster bedeutet meist, dass
  der Vertrag nicht ganz das richtige Werkzeug ist.
- **Der Abbruch des Rumpfs gibt die Sperre nicht frei.** Ein
  abgebrochener Rumpf lässt die Sperre halten, bis die TTL
  abläuft. Das ist die fail-closed Entscheidung; richten Sie Ihre
  Timeouts so ein, dass der Abbruch dem entspricht, was ein
  doppelter Aufrufer sehen soll.

## Nächste Schritte

- [cache.md](cache.md) - die zugrunde liegende Sperren-Primitive
  und die Auswahl von `CACHE_DRIVER`.
- [queues.md](queues.md) - wie `Queue::push_unique` auf
  `Idempotency::commit_on_success` für Dedupe auf Job-Ebene aufbaut.
- [payments.md](payments.md) - Webhook-Ingress, der
  Datenbank-Zeilen-Idempotenz statt Cache-Key-gestütztem Dedupe
  verwendet, und wann Sie zu welchem greifen.
- [rate-limiting.md](rate-limiting.md) - benachbarte Middleware,
  die dasselbe `Cache`-Backend für Sliding-Window-Durchsetzung
  verwendet.
- [middleware.md](middleware.md) - wie Sie die Extraktion des
  Idempotenzschlüssels in eine wiederverwendbare Middleware über
  Ihre POST/PUT-Routen faktorisieren.

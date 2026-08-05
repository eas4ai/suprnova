# Service Container

Der Container ist der Ort, an dem Suprnova die Services Ihrer Anwendung
hält - den DB-Verbindungspool, den Mail-Treiber, Ihren `Arc<MyService>`.
Sie binden Werte beim Boot hinein und lösen sie in Handlern und Workern
auf. Er ist das Suprnova-Äquivalent zu Laravels Service Container, mit
einem wichtigen Unterschied: Der Lookup ist zuerst Task-Local, sodass
gleichzeitig laufende Tests nicht die Bindings der jeweils anderen sehen.

## Die zwei Teile

| Typ | Rolle |
|---|---|
| `Container` | Die zugrunde liegende Registry: hält Bindings, Factories und Singletons |
| `App` | Die globale Facade, die Sie tatsächlich aufrufen - `App::bind`, `App::get` usw. |

Sie rufen fast immer `App::*` auf, statt selbst einen `Container` zu
konstruieren. Der Container ist die Verdrahtung; die `App`-Facade ist die
API.

## Lookup-Reihenfolge

Jeder Aufruf von `App::get` / `App::make` prüft **drei Ebenen** der Reihe
nach:

```
        Task-Local
            │
            ▼  (kein Treffer)
       Thread-Local
            │
            ▼  (kein Treffer)
          Global
            │
            ▼  (kein Treffer)
          None
```

Dies ist wichtig, weil:

- **Per-Request-Zustand läuft über Task-Local** - Inertias geteilte
  Daten, Flash-Bag, Request-ID. Jede Anfrage bekommt transparent ihre
  eigene Ebene.
- **Tests verwenden Thread-Local** - `let _g = TestContainer::fake();`
  gefolgt von `TestContainer::bind(...)` bindet innerhalb eines Threads,
  ohne den globalen Container zu berühren, sodass parallele Tests ihre
  Services nicht ineinander überlaufen lassen. Der Guard räumt den
  Test-Container auf, sobald er verworfen wird.
- **App-weite Services laufen über Global** - einmal beim Boot gebunden,
  überall aufgelöst.

Sie denken selten darüber nach, in welcher Ebene ein Binding lebt -
`App::bind` legt es dort ab, wo es sinnvoll ist, und `App::get` findet es,
wo auch immer es liegt. Das Modell wird nur relevant, wenn sich unter
Nebenläufigkeit etwas unerwartet verhält, und dann liefert das
[Testen](testing.md)-Kapitel die Details.

## Wert binden

Fünf Wege, um etwas in den Container zu legen, je nachdem, was Sie zur
Verfügung haben:

### `App::singleton(value)` - eigener Wert, beim Lookup geklont

Für jeden `T: Any + Send + Sync + 'static`-Wert, der dauerhaft leben soll.
Die `Clone`-Schranke liegt auf dem *Getter* (`App::get`), nicht auf dem
Binding - der Wert wird einmal in einem `Arc` gespeichert und bei jedem
`get` aus diesem `Arc` geklont:

```rust
use suprnova::App;

App::singleton(MyConfig {
    timeout_secs: 30,
    retries: 3,
});

let cfg = App::get::<MyConfig>().expect("registered at boot");
println!("{}", cfg.timeout_secs);
```

Der Wert wird einmal gespeichert; `App::get::<MyConfig>()` gibt jeweils
einen Klon zurück. Verwenden Sie dies für einfache, config-artige Daten,
die billig zu klonen sind.

### `App::bind(Arc<T>)` - für Traits und geteilte Services

Für Trait-Objekte oder alles, was hinter einem `Arc` liegen soll:

```rust
use std::sync::Arc;
use suprnova::App;

let store: Arc<dyn KeyValueStore> = Arc::new(RedisStore::connect(url)?);
App::bind(store);

let store = App::make::<dyn KeyValueStore>().expect("bound at boot");
store.put("hello", b"world").await?;
```

`App::make::<T>()` gibt den geklonten `Arc<T>` zurück (ein günstiges
atomares Hochzählen des Referenzzählers). Verwenden Sie dies für jeden
Service, der über Threads hinweg geteilt wird, insbesondere für
Trait-Objekte.

### `App::factory(|| { … })` - bei Bedarf gebaut

Wenn der Wert erst bei der ersten Nutzung (oder jedes Mal) konstruiert
werden soll:

```rust
App::factory(|| {
    HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client config is hand-rolled and known-good")
});
```

`App::factory` registriert eine Factory für einen *konkreten Typ*
(`Fn() -> T`); `App::bind_factory` registriert eine Factory für ein
*Trait-Objekt* (`Fn() -> Arc<T>`). Keine der beiden Closures gibt
`Result` zurück - behandeln Sie einen Konstruktionsfehler innerhalb der
Closure (Panic beim Boot, oder bauen Sie einen Sentinel-Wert) oder
verwenden Sie ein reguläres `App::singleton` / `App::bind`, nachdem Sie
den Wert selbst mit `?` konstruiert haben. Beide rufen die Closure
außerhalb jeder Container-Sperre auf, sodass eine Factory, die den
Container erneut betritt, nicht in einen Deadlock läuft und ein teurer
Konstruktor keine anderen Bindings blockiert.

### `App::*_if_absent(value)` - Registrierung im Einklang mit der Boot-Reihenfolge

Manchmal registriert eine Service-Crate einen Standard-Service, und die
App will ihn nur überschreiben, wenn er tatsächlich vorhanden ist. Die
`_if_absent`-Varianten lassen Sie einen Standard registrieren, der ein
bereits vorhandenes Binding nicht überschreibt:

```rust
// In einer Starter- oder Library-Crate:
App::singleton_if_absent(DefaultMailDriver::new());

// In der bootstrap.rs Ihrer App:
App::singleton(MyCustomMailDriver::new());  // gewinnt, weil es später lief
```

`bind_if_absent`, `singleton_if_absent` und die Factory-Varianten geben
alle ein `bool` zurück - `true`, wenn tatsächlich eingefügt wurde,
`false`, wenn bereits ein Binding existierte.

## Wert auflösen

Zwei Lesemethoden, plus ihre `Result`-zurückgebenden Geschwister:

```rust
// Klonen Sie den gebundenen Wert heraus:
let cfg: MyConfig = App::get::<MyConfig>().expect("bound at boot");

// Klonen Sie den Arc:
let store: Arc<dyn KeyValueStore> = App::make().expect("bound at boot");

// Das Gleiche, aber als Result, für das `?`-Idiom in fehlbaren Pfaden:
let cfg = App::resolve::<MyConfig>()?;
let store = App::resolve_make::<dyn KeyValueStore>()?;
```

`resolve` und `resolve_make` geben `Result<_, FrameworkError>` zurück
(genauer die Variante `ServiceNotFound`, wenn der Lookup fehlschlägt) -
nützlich in Handler-Pfaden, in denen ein fehlender Service als 500 mit
einem ordentlichen Log auftauchen soll, statt als Panic.

Existenzprüfungen (selten benötigt):

```rust
if App::has::<MyConfig>() { … }
if App::has_binding::<dyn KeyValueStore>() { … }
```

## Wo das Binding stattfindet

Der Standardort ist `src/bootstrap.rs` - eine Funktion, die einmal beim
Boot läuft:

```rust
use std::sync::Arc;
use suprnova::App;
use crate::services::{MyService, RealEmailGateway};

pub async fn register() {
    // Einfache Singletons
    App::singleton(MyAppConfig {
        max_uploads_per_user: 100,
    });

    // Trait-Objekt-Services
    let gateway: Arc<dyn EmailGateway> = Arc::new(RealEmailGateway::new());
    App::bind(gateway);

    // Lazy Services (beim ersten Gebrauch gebaut)
    App::bind_factory::<dyn HttpClient, _>(|| {
        Arc::new(ReqwestClient::with_timeout(30))
    });
}
```

Der Funktionsname `register` folgt dem Scaffold-Standard
(`src/bootstrap.rs::register`); der Rückgabetyp ist `()`, nicht
`Result`. Bind-Fehler, die beim Boot auftreten (z. B. gescheiterte
Treiber-Verbindungen), sollten über den Treiber-/Service-Konstruktor
propagieren, nicht aus `register` selbst - siehe
[Application Bootstrap](bootstrap.md) für die vollständige
Boot-Verdrahtung.

Das Framework ruft während des Boots auch selbst in den Container
hinein:

- `App::init()` läuft zuerst und initialisiert die Registry
- `App::boot_services()` löst Boot-Zeit-Abhängigkeiten auf (Treiber,
  Verschlüsselungsschlüssel usw.) - Ihre Services sehen ein vollständig
  hochgefahrenes Framework
- Ihre `bootstrap_fn` läuft danach und kann sich deshalb darauf
  verlassen, dass die Services des Frameworks verfügbar sind

Siehe [Application Bootstrap](bootstrap.md) für die vollständige
Boot-Reihenfolge.

## Geteilte Inertia-Daten

Der Container ist auch der Ort, an dem die geteilten Daten von
Inertia leben. Drei Komfort-APIs machen das explizit:

```rust
use suprnova::App;

// Eager-Wert - einmal serialisiert und für jede Inertia-Response wiederverwendet.
App::inertia_share("appName", "Suprnova");

// Lazy-Wert - der Resolver läuft pro Response. Für Per-Request-Daten,
// die asynchrone Arbeit brauchen.
App::inertia_share_lazy("locale", || async {
    Ok::<_, suprnova::FrameworkError>(detect_locale().await)
});

// Einen einzelnen Flash-Eintrag auf die Per-Request-Flash-Bag legen.
App::flash("message", "Saved!");
```

Diese lesen von `Container::inertia()`, was `&Arc<InertiaRegistry>`
zurückgibt - Sie können bei Bedarf direkt damit interagieren, wenn Sie
tiefer liegenden Zugriff brauchen. Siehe [Inertia / Frontend](frontend.md)
dafür, wie die geteilten Daten am Ende in der Seiten-Response landen.

## Warum drei Ebenen?

Die Kaskade Task-Local → Thread-Local → Global existiert aus genau einem
Grund: **Isolation unter Nebenläufigkeit**. Drei Dinge profitieren davon:

**Per-Request-Isolation.** Inertias Flash-Bag wird per-Request über die
Task-Local-Ebene gebunden. Zwei gleichzeitige Anfragen sehen nicht den
Flash der jeweils anderen, weil sich ihre Task-Local-Container nicht
überschneiden. Das Binding verschwindet, sobald die Task der Anfrage
endet.

**Per-Test-Isolation.** Ein Test, der einen Fake-Mail-Treiber bindet,
soll keinen Fake sehen, den ein Geschwister-Test gebunden hat.
`TestContainer::fake()` gibt einen Thread-Local-Guard zurück, und
`TestContainer::bind` / `TestContainer::singleton` schreiben in den
aktiven Scope. Parallele Tests bleiben hermetisch:

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn one_test_binds_a_fake() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn Mailer>(Arc::new(FakeMailer::new()));

    // … dieser Test verwendet FakeMailer
    // ein parallel laufender Geschwister-Test sieht ihn nicht
}
```

Für Multi-Thread-Tokio-Runtimes - bei denen die Future zwischen
Worker-Threads wandern kann - verwenden Sie stattdessen
`TestContainer::scope(async { ... })`; das installiert eine
Task-Local-Überschreibung, die die Migration übersteht.

**Override-at-Boot.** Anwendungscode kann Standards überschreiben, die
von Library-Crates registriert wurden. Die `_if_absent`-Varianten und
der geschichtete Lookup kombinieren sich so, dass Library-Crates eine
saubere Standardregistrierung bekommen, ohne mit Anwendungs-Overrides zu
kollidieren.

## Häufige Muster

### Eine Struktur binden, die den DB-Pool hält

Das machen Sie fast nie direkt - das Framework bindet den DB-Pool
selbst. Aber wenn Sie ein eigenes Subsystem mit einer teuren geteilten
Ressource haben:

```rust
let pool = MyResourcePool::connect(url).await?;
App::bind(Arc::new(pool));

// später:
let pool = App::resolve_make::<MyResourcePool>()?;
let conn = pool.checkout().await?;
```

`App::make` gibt `Option<Arc<T>>` zurück und passt zu `.expect(...)`;
`App::resolve_make` gibt `Result<Arc<T>, FrameworkError::ServiceNotFound>`
zurück und passt zu `?` in fehlbarem Code. Verwenden Sie die Variante,
die zur Fehlerstrategie Ihres Aufrufers passt.

### Einen Standard in Tests gegen einen Fake tauschen

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn order_dispatches_email() {
    let fake = Arc::new(FakeEmailGateway::new());
    let fake_for_assert = Arc::clone(&fake);

    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.expect("place_order succeeds");

    assert_eq!(fake_for_assert.sent_count(), 1);
}
```

### Teure Konstruktion lazy verzögern

```rust
// Baut das Embedding-Modell beim ersten Request, nicht beim Boot.
App::bind_factory::<dyn EmbeddingModel, _>(|| {
    Arc::new(
        OnnxEmbedding::load_from_disk("/models/all-mini-lm.onnx")
            .expect("embedding model must load"),
    )
});
```

Für fehlbare Konstruktionen, die dem Betreiber einen strukturierten
Fehler zeigen müssen, bauen Sie den Wert selbst in `bootstrap()` mit `?`
und rufen `App::bind(...)` auf, sobald er fertig ist.

## Warum Suprnova abweicht

Laravels Container hat einen einzigen globalen Scope - Bindings sind
global, und Isolation zwischen Tests erfordert `setUp`- /
`tearDown`-Disziplin plus die Datenbank-Transaktion des Frameworks pro
Test. PHPs Request-pro-Prozess-Modell macht das quasi zufällig sicher:
Ein frischer Prozess pro Request bedeutet, dass der Container jedes Mal
zurückgesetzt wird.

Rusts Prozessmodell ist das Gegenteil - ein einziger Prozess bedient
viele gleichzeitige Anfragen auf vielen Threads. Ein rein globaler
Container würde bedeuten, dass ein Test in einem Thread einen Fake
sehen kann, den ein anderer gebunden hat, oder dass eine Anfrage die
Per-Request-Daten einer anderen Anfrage sehen könnte. Deshalb hat
Suprnova die Drei-Ebenen-Kaskade: Task-Local für Per-Request,
Thread-Local für Per-Test, Global für App-weit.

Die Container-API ist dieselbe wie bei Laravel; die Lookup-Mechanik
unterscheidet sich, weil die Runtime eine andere ist.

## Nächste Schritte

- [Application Bootstrap](bootstrap.md) - wohin der Binding-Code gehört
- [Konfiguration](configuration.md) - typisierte Konfigurationsregistrierung
  neben Services
- [Testen](testing.md) - `TestContainer::fake` und `#[suprnova_test]`
- [Lock-Richtlinie](lock-policy.md) - warum die Wiederherstellung
  vergifteter Sperren in einer Container-gestützten Anwendung wichtig
  ist

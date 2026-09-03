# Live

Suprnova Live ist die servergesteuerte Interaktions-Engine des Frameworks. Eine
Live-Komponente ist ein Rust-Struct, dessen Zustand auf dem Server lebt, dessen
View ein Askama-Template ist und dessen Aktionen über ein signiertes Protokoll
von einer kleinen Browser-Laufzeit ausgeführt werden, die das neu gerenderte
HTML an Ort und Stelle morpht. Es gibt kein clientseitiges Zustandsmodell, das
synchron gehalten werden muss, kein Build-Werkzeug, das für die ausgelieferte
Laufzeit installiert werden müsste, und kein Inline-JavaScript in Ihren
Dokumenten.

Dieses Kapitel behandelt die anwendungsseitige Oberfläche: das Schreiben einer
Komponente, ihre Registrierung, das Ausliefern von Dokumenten und Inseln, die
Sicherheitsgrenzen, die jede Live-Anfrage überschreitet, Uploads, asynchrone
Aktualisierungen, Assets, Tests, Diagnose und Wiederherstellung. Alles hier
verwendet ausschließlich `suprnova::live` und `suprnova::view`.

## Schnellstart

Ein mit `suprnova new` erstelltes Projekt ist Live-bereit: Es liefert
`src/live/mod.rs` mit einer leeren Komponentenregistry und einer Funktion
`routes()`, sein Bootstrap bindet die Registry, und `cmd/main.rs` installiert
die Routen. Erzeugen Sie eine Komponente und prüfen Sie sie anschließend:

```bash
suprnova live:make Counter
suprnova live:check
```

`live:make` schreibt `src/live/counter.rs` und `templates/live/counter.html`,
registriert die Komponente in `src/live/mod.rs` und gibt die nächsten Schritte
aus. `live:check` baut Ihre Anwendung und beweist jede registrierte View gegen
den integrierten Checker.

## Eine Komponente schreiben

```rust
use suprnova::live::{LiveComponent, live};

/// A counter rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

- `name` ist der registrierte Komponentenname. Verwenden Sie einen Namen mit
  Punkten in Kebab-Case wie `app.counter`; die CLI leitet `<package>.<kebab>` ab.
- `view` ist die Template-Identität relativ zum Template-Wurzelverzeichnis.
- `#[public]`-Felder werden gerendert und im signierten Snapshot mitgeführt.
  `#[model]`-Felder akzeptieren zusätzlich Browser-Vorschläge über `live:model`.
- `#[action]`-Methoden sind die einzigen Einstiegspunkte, die der Browser
  aufrufen kann. Sie erhalten validierte Argumente und können typisierte
  Ergebnisse wie eine Weiterleitung oder einen Flash zurückgeben.

Jeder Feldtyp muss `Default` implementieren; eine frische Insel startet mit
diesen Standardwerten, sofern kein Mount-Hook etwas anderes vorgibt.

## Views

Views sind Askama-Templates. Das Template-Wurzelverzeichnis ist `templates/`,
sofern eine `askama.toml` keine anderen Verzeichnisse benennt, sodass
`live/counter.html` unter `templates/live/counter.html` liegt:

```html
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

Direktiven verwenden die geschlossene `live:`-Grammatik: `live:click`,
`live:submit`, `live:model`, `live:upload`, `live:key`, `live:loading` und den
Rest der dokumentierten Menge. Der Checker beweist jede Direktive gegen die
Komponente: Eine unbekannte Aktion, ein unbekanntes Modellfeld, ein roher
`safe`-Filter oder ein Barrierefreiheitsverstoß lässt `live:check` mit Datei,
Zeile und Spalte fehlschlagen.

Dokumente, die Inseln platzieren, sind gewöhnliche Views, die mit
`#[suprnova::view]` deklariert werden; der einzige nicht maskierte Wert, den
sie akzeptieren, ist `TrustedHtml` über den Filter `trusted_html`.

## Registrierung und Bootstrap

`src/live/mod.rs` besitzt die Registry und die Routen:

```rust
use suprnova::live::{LiveRegistry, RegistryError};

pub mod counter;

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<counter::Counter>()?
        .build();
    Ok(registry)
}
```

Binden Sie sie während des Bootstraps, damit der Server, die Worker und die
Befehle `suprnova live:*` dieselben Komponenten sehen:

```rust
suprnova::App::singleton(crate::live::registry().expect("Live component registry"));
```

Die Registry ist unveränderlich, sobald die Laufzeit zusammengesetzt ist. Ein
doppelter Komponentenname oder eine doppelte View oder eine Komponente, deren
Aktionen Validierung ohne Validierungsport benötigen, lässt die Registrierung
mit einem typisierten `RegistryError` fehlschlagen.

## Routen

`Router::try_live()` installiert den reservierten Namensraum genau einmal:
`/__live/v1/action`, `/__live/v1/upload`, die Steuerrouten und den
WebSocket-Handshake unter `/__live/v1/async/*` sowie die unveränderlichen
Routen unter `/__live/v1/assets/*`. Der Start schlägt fehl, wenn eine
Anwendungsroute `/__live` beanspruchen kann.

Die reservierten Anfragerouten tragen eine strikte Richtlinie: Jede Anfrage
benötigt Fakten zu Sitzung, Origin, CSRF, Principal, Mandant und Ratenlimit.
Das Framework zeichnet die Sitzung und den CSRF-Nachweis auf; Ihre Anwendung
hängt den Rest mit dem Routenwächter an:

```rust
use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveTenantMiddleware, LiveTenantResolver};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{AuthMiddleware, FrameworkError, RateLimitMiddleware, Request, Router, SlidingWindowConfig, async_trait};

pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let limiter = Arc::new(InMemoryRateLimiter::new());
    router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::new())
            .middleware(LiveTenantMiddleware::new(Arc::new(SingleTenant)))
            .middleware(RateLimitMiddleware::new(
                limiter,
                SlidingWindowConfig { max_requests: 600, window: Duration::from_secs(60) },
                |request: &Request| format!("live:{}", request.ip().unwrap_or_else(|| "anon".into())),
            ))
    })
}

struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
```

Installieren Sie die Routen vom Einstiegspunkt aus, damit die Laufzeit und der
Mount-Katalog vor der ersten Anfrage bereit sind:

```rust
Application::new()
    .bootstrap(bootstrap::register)
    .try_routes(|| live::routes(routes::register()))
    .run()
    .await;
```

## Dokumente und Inseln

Eine Dokumentroute deklariert ihre Inseln einmal, rendert sie über
`LiveDocument` und gibt die Bootstrap-Tags aus:

```rust
use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

pub fn install(router: Router) -> Result<Router, FrameworkError> {
    let mount = LiveMount::<Counter>::identity_bound("/dashboard", "counter", "dashboard-counter")?;
    let handler_mount = mount.clone();
    let router: Router = router
        .get("/dashboard", move |request: Request| {
            let mount = handler_mount.clone();
            async move { render(request, &mount).await }
        })
        .middleware(AuthMiddleware::redirect_to("/login"))
        .into();
    router.try_live_mount(&mount)
}

async fn render(request: Request, mount: &LiveMount<Counter>) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("live/page.html").map_err(|_| FrameworkError::internal("view"))?,
                &Page { bootstrap: bootstrap.html(), counter: counter.html() },
                DocumentResponseIntent::html(StatusCode::OK).map_err(|_| FrameworkError::internal("intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|_| HttpResponse::text("Live document failed").status(500))
}
```

- `LiveMount::public_seed` deklariert eine Insel, die jeder Besucher rendern
  darf; ihr Zustand ist ein wiederverwendbarer Seed, der bei der ersten Aktion
  zu einer Instanz befördert wird.
- `LiveMount::identity_bound` deklariert eine Insel, die zur aktuellen Sitzung
  und zum aktuellen Principal gehört; die Dokumentroute muss authentifizieren.
- Mounten Sie jede Insel vor `bootstrap` und rufen Sie `bootstrap` einmal auf.
  Der Bootstrap gibt das inerte Konfigurationselement und die Script-Tags für
  die ESM- oder die klassische Strategie aus, fügt die Upload- und die
  asynchrone Rolle hinzu, wenn eine gemountete Komponente sie benötigt, und die
  Stimulus-Brücke auf Anfrage.
- Das Dokument-Template platziert `{{ bootstrap|trusted_html }}` in `<head>`
  und jede Insel dort, wo sie hingehört.

## Sicherheitsgrenzen

Live umgeht niemals die Middleware des Frameworks. Was jede Anfrage benötigt:

| Fakt | Aufgezeichnet von |
|---|---|
| Sitzung | `SessionMiddleware` |
| Origin und CSRF | `CsrfMiddleware` mit aktivierter Origin-Prüfung |
| Principal | `AuthMiddleware` in ihrem authentifizierten Zweig |
| Mandant | `LiveTenantMiddleware` mit Ihrem Resolver |
| Ratenlimit | `RateLimitMiddleware` in ihrem erlaubten Zweig |

Die ausgelieferte Laufzeit sendet den Live-Medientyp und den browsereigenen
Header `Sec-Fetch-Site`; sie trägt kein Sitzungstoken. Konfigurieren Sie die
CSRF-Middleware so, dass sie Origins prüft: Eine Same-Origin-Live-Anfrage
passiert mit der zustandslosen CSRF-Disposition, während eine Cross-Site- oder
headerlose Anfrage auf die Token-Validierung zurückfällt und abgewiesen wird:

```rust
global_middleware!(CsrfMiddleware::new().with_origin_policy(OriginPolicy::SameOriginOnly));
```

Anonyme Besucher können öffentliche Seeds rendern, aber keine Aktionen
ausführen: Die `AuthMiddleware` des Wächters antwortet mit `401`, bevor
irgendeine Engine-Arbeit beginnt. Identitätsgebundene Inseln benötigen eine
Sitzung und einen Principal; der Mandant wird in den Geltungsbereich der Insel
gebunden, sobald Ihr Resolver einen benennt. Jede Ablehnung ist geschlossen:
Ein `409` für einen veralteten oder manipulierten Snapshot trägt keinen
Rumpf, und Produktionsmeldungen enthalten niemals Snapshots, Tokens, Cookies
oder gerendertes HTML.

## Uploads

Deklarieren Sie eine Upload-Richtlinie auf einem Modellfeld:

```rust
use suprnova::live::{LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadType, live};

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(512 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

#[derive(LiveComponent)]
#[live(name = "app.avatar-uploader", view = "live/avatar-uploader.html")]
pub struct AvatarUploader {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl AvatarUploader {
    #[action]
    pub fn save_avatar(&mut self) {}
}
```

Die View bindet das Feld mit `<input type="file" live:upload="avatar">`. Die
Laufzeit erstellt, überträgt und vervollständigt den Upload über
`/__live/v1/upload`; die Datei wartet in Quarantäne, bis die deklarierte
Finalisierungsaktion läuft, woraufhin das Framework sie an Ihren
`UploadFinalizer` übergibt. Binden Sie den Finalizer sowie jeden Scanner oder
Validator, bevor die Laufzeit zusammengesetzt wird:

```rust
App::singleton(LiveUploadHost::new().with_finalizer(Arc::new(AppUploadFinalizer::default())));
```

Uploads werden pro Feld und Steuerung über das Gate autorisiert. Definieren
Sie die Fähigkeiten `live:<component>.upload.<field>.<Control>` für `Create`,
`Reacquire`, `Status`, `Queue`, `BeginTransfer`, `PutChunk`, `Complete`,
`Accept`, `BeginFinalize`, `CommitFinalize`, `Cancel`, `Reject`, `Expire`
und `Fail`.

Ein Browser, der seine Übertragungsberechtigung verloren hat, erwirbt sie über
eine Route zurück, die Ihre Anwendung außerhalb des reservierten Namensraums
besitzt:

```rust
let router: Router = router
    .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")?
    .middleware(AuthMiddleware::new())
    .into();
```

Die Route verlangt dieselben Fakten wie eine Aktion, antwortet nur der Sitzung
und dem Principal, die den Upload erstellt haben, und liefert eine frische
Berechtigung mit dem aktuellen Übertragungszustand.

## Asynchrone Aktualisierungen

Eine Komponente deklariert die Streams, auf die sie hört; die Browser-Laufzeit
abonniert über SSE oder WebSocket und fällt auf Polling zurück:

```rust
use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

pub struct ActivityPosted;

impl EventPayloadMetadata for ActivityPosted {
    const NAME: &'static str = "activity.posted";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "app.activity-feed",
    view = "live/activity-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "activity", topics("activity"), events(ActivityPosted)))
)]
pub struct ActivityFeed {
    #[public]
    headline: String,
}
```

Definieren Sie die Fähigkeit `live:<component>.stream.<name>` für Abonnenten
und veröffentlichen Sie dann von überall in der Anwendung:

```rust
let streams = LiveStreams::resolve()?;
streams.event::<ActivityPosted>("activity", LiveEventTarget::Island, payload).await?;
streams.refresh("activity").await?;
```

Ein Refresh weist abonnierte Inseln an, frisch zu rendern; ein Ereignis wird
an die registrierten Handler der Insel zugestellt. Polling ist das gewöhnliche
frische Rendern, sodass nichts verloren geht, wenn ein Transport nicht
verfügbar ist.

## Assets und Nutzung ohne Build

Das Framework liefert die exakt geprüften Laufzeit-Artefakte unter
`/__live/v1/assets/<identity>/<file>` mit unveränderlichem Caching, starken
Validatoren und Integritätsattributen in den Bootstrap-Tags aus. Eine strikte
Richtlinie `script-src 'self'` hält, weil Dokumente kein Inline-Script
enthalten. Um dieselben Bytes auf ein CDN oder in ein statisches Verzeichnis zu
veröffentlichen:

```bash
suprnova live:assets --out public/__live
```

Die Veröffentlichung ist atomar und weigert sich, ein Verzeichnis zu ersetzen,
dessen Bytes abweichen, sofern Sie nicht `--replace` übergeben.

## Tests

`suprnova::live::testing` bereitet die Laufzeit und den Mount-Katalog eines
Routers für In-Process-Tests vor. Die Anwendungstests in `app/tests/live_*.rs`
zeigen das vollständige Muster: eine In-Memory-Datenbank, ein vorbereitetes
Sitzungs-Cookie, der echte globale Middleware-Stack und Anfragen über
`handle_request`:

```rust
let router = app::live::routes(app::routes::register())?;
let runtime = prepare_live_router_for_test(&router)?;
App::singleton(runtime.clone());
```

Dekodieren Sie den Snapshot einer Insel aus ihrem Attribut
`data-suprnova-live-snapshot`, senden Sie eine Aktion mit dem Sitzungs-Cookie
und `Sec-Fetch-Site: same-origin` und prüfen Sie das akzeptierte Rendering.
Ein veralteter Snapshot antwortet mit `409` und leerem Rumpf; ein fehlender
Principal antwortet mit `401`.

## Diagnose und Betrieb

- `suprnova live:check` beweist jede registrierte View; `--allow-unproved`
  akzeptiert dynamische Strukturen, über die der Checker bewusst keine Aussage
  trifft.
- `suprnova live:inspect` meldet die gebundene Registry, Konfigurationsgrenzen,
  installierte Upload-Fähigkeiten, zusammengesetzte Laufzeitdienste und die
  Asset-Identität, ohne Zustand oder Geheimnisse preiszugeben.
- `LiveConfig` begrenzt Anfrage- und Antwortbytes sowie die Lebensdauer des
  vertrauenswürdigen Kontexts; binden Sie eine eigene, bevor die Laufzeit
  zusammengesetzt wird.
- Fehler tragen geschlossene Arten wie `live_document_context_rejected` und
  `invalid_live_bootstrap`; Telemetrie-Labels sind geschlossene Aufzählungen.

## Wiederherstellung

- Ein `409` weist die Laufzeit an, die Insel frisch zu rendern; die Operation
  wird nicht wiederholt.
- Ein geschlossener asynchroner Transport wird stillgelegt, und die Laufzeit
  verbindet sich mit einer neuen Transportgeneration neu; eine veraltete
  Generation wird abgewiesen.
- Eine Sitzung, die abläuft oder rotiert, macht identitätsgebundene Arbeit
  ungültig; die Anwendung zeigt ihren Anmeldepfad, und der Besucher macht mit
  einem frischen Dokument weiter.

Live läuft vollständig ohne RenderCache; das Caching von Live-Dokumenten ist
eine eigene Funktion mit eigenem Kapitel, sobald sie erscheint.

## CLI-Referenz

| Befehl | Zweck |
|---|---|
| `suprnova live:make <name>` | Eine Komponente und ihre View erzeugen und registrieren |
| `suprnova live:check` | Jede registrierte View mit dem integrierten Checker beweisen |
| `suprnova live:inspect` | Sicheren Laufzeit-, Registry-, Provider- und Artefaktzustand melden |
| `suprnova live:assets --out <dir>` | Die geprüften Laufzeit-Artefakte atomar veröffentlichen |

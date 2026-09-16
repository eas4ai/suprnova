# Live

Suprnova Live ist die servergesteuerte Interaktions-Engine des Frameworks. Eine
Live-Komponente ist eine Rust-Struktur, deren Zustand auf dem Server lebt, dessen
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
  Ein Model-Feld deklariert sein Timing am Attribut, etwa
  `#[model(debounce = 250)]`; ein Debounce beträgt 100, 250 oder 500
  Millisekunden, die Dauern, die `live:model.debounce.<n>ms` akzeptiert, und
  jede andere lässt sich nicht kompilieren.
  Ein mit `live:submit` gesendetes Formular schlägt jedes Model-Steuerelement
  darin in einer Anfrage vor, die neben der Aktion bis zu 127 Felder trägt;
  `live:check` weist ein größeres Formular ab.
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
`live:key` benennt die stabile Identität eines Elements über Morphs hinweg und ist das eine Key-Attribut, das ein Template schreibt: Die Runtime liest es für die Morph-Identität, für Morph-Steuerungen wie `live:preserve.self` und für die Bereiche, die Browser-Zustand bewahren; `data-suprnova-live-key` ist die eigene Schreibweise der Engine auf den Wurzeln, die sie rendert.

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

## Routing

`Router::try_live()` installiert den reservierten Namensraum genau einmal:
`/__live/action`, `/__live/upload`, die Steuerrouten und den
WebSocket-Handshake unter `/__live/async/*` sowie die unveränderlichen
Routen unter `/__live/assets/*`. Der Start schlägt fehl, wenn eine
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
            .middleware(AuthMiddleware::optional())
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
Header `Sec-Fetch-Site`; sie trägt kein Sitzungstoken. Die CSRF-Middleware
prüft diesen Nachweis für jede Live-Anfrage selbst, unabhängig von der
konfigurierten Origin-Richtlinie: Eine Same-Origin-Live-Anfrage passiert mit
der zustandslosen CSRF-Disposition, während eine Cross-Site- oder headerlose
Anfrage auf die Token-Validierung zurückfällt und abgewiesen wird. Gewöhnliche
Routen behalten unter der Standardrichtlinie die Token-Validierung; Live
lockert nichts anderes:

```rust
global_middleware!(CsrfMiddleware::new());
```

Anonyme Besucher rendern öffentliche Seeds und können darauf Aktionen
ausführen, wenn der Wächter `AuthMiddleware::optional()` verwendet: Ein
angemeldeter Principal wird aufgezeichnet, ein anonymer Besucher fährt fort,
und die Mount-Art entscheidet. Ein öffentlicher Seed wird dann bei der ersten
Aktion für die eigene Sitzung des Besuchers befördert, während eine
identitätsgebundene Insel eine Anfrage ohne Principal-Nachweis weiterhin
abweist. Mit `AuthMiddleware::new()` antwortet der Wächter auf jede anonyme
Anfrage mit `401`, bevor irgendeine Engine-Arbeit beginnt. Identitätsgebundene
Inseln benötigen eine Sitzung und einen Principal; der Mandant wird in den
Geltungsbereich der Insel gebunden, sobald Ihr Resolver einen benennt, und ein
Resolver, der den Mandanten nicht bestimmen kann, muss einen Fehler statt
`None` zurückgeben. Jede Ablehnung ist geschlossen:
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
`/__live/upload`; die Datei wartet in Quarantäne, bis die deklarierte
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
frische Rendern: Der Zustand der Insel holt auf, sobald ein Transport nicht
verfügbar ist, aber zwischenzeitlich veröffentlichte Ereignisnutzlasten werden
ihren Handlern nicht erneut zugestellt, was die Laufzeit als beeinträchtigten
statt aktuellen Stream meldet. Eine Komponente, die genau einen Stream
deklariert, bekommt ihre Inselwurzel dafür abonniert; eine Komponente mit
mehreren Streams abonniert jeden über die registrierten Aufrufe der Laufzeit.

Ein Stream endet mit der Session, die ihn geöffnet hat. Wird eine Session auf
dem Knoten zerstört, der den Stream hält, ob durch einfache Abmeldung,
Invalidierung, Id-Regenerierung oder ein "überall abmelden", wird jede
Mitgliedschaft, die sie dort geöffnet hat, sofort beendet, und kein späteres
Ereignis erreicht sie mehr. Eine auf einem anderen Knoten zerstörte Session
fängt die Zustellung selbst ab: die Session jeder Mitgliedschaft wird
höchstens alle zehn Sekunden erneut gegen den Session-Store geprüft, sodass
Ereignisse innerhalb dieses Intervalls ausbleiben. Das Gate des Streams wird
ohnehin vor jeder Zustellung erneut befragt, sodass eine Richtlinienänderung
die Zustellung auf jedem Knoten sofort beendet.

## Assets und Nutzung ohne Build

Das Framework liefert die exakt geprüften Laufzeit-Artefakte unter
`/__live/assets/<identity>/<file>` mit unveränderlichem Caching, starken
Validatoren und Integritätsattributen in den Bootstrap-Tags aus. Eine strikte
Richtlinie `script-src 'self'` hält, weil Dokumente kein Inline-Script
enthalten. Um dieselben Bytes auf ein CDN oder in ein statisches Verzeichnis zu
veröffentlichen:

```bash
suprnova live:assets --out public/__live
```

Die Veröffentlichung ist atomar und weigert sich, ein Verzeichnis zu ersetzen,
dessen Bytes abweichen, sofern Sie nicht `--replace` übergeben.

## Komponentenbibliothek

Suprnova liefert die Grundlagen einer Komponentenbibliothek für Live: ein
Token-Stylesheet mit einer Basisschicht und eine Formularfamilie aus
präsentationalen Komponenten, die auf nativen Controls und dem Vokabular
`live:model`, `live:error` und `live:loading` aufbauen. Die Basis ist ein
Runtime-Artefakt. Meldet ein Dokument sich an, kommt sie als ein einzelner
Stylesheet-Link unter demselben Identitäts-, Integritäts- und Cache-Vertrag
wie die Runtime-Skripte:

```rust
let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
```

Jede Regel darin liegt in der Cascade-Layer `suprnova-ui`, sodass eigene
Styles ohne Layer ohne Spezifitätskampf gewinnen. Jeder visuelle Wert ist eine
`--sn-`-Custom-Property für Farbe, Schrift, Abstand, Radius, Schatten,
Bewegung, Dichte und Zustand, mit hellen und dunklen Werten: Überschreibe ein
Token auf `:root`, um umzugestalten, oder lasse die Layer weg und behalte
jedes Verhalten, jeden Namen und jedes Zustandsattribut, denn eine Komponente
gestaltet ihre Zustände über die Attribute, die der Checker beweist
(`aria-invalid`, `aria-busy`, `aria-expanded`, `aria-pressed`, `aria-current`,
`aria-selected`, `:disabled`), nie über eine Klasse. Ein
Tailwind-CSS-4-`@theme`-Preset bildet die Tokens auf Tailwinds Namensräume ab;
Tailwind wird nie vorausgesetzt. Komponenten werden mit `live:add`
installiert, je ein Verzeichnis unter der reservierten Wurzel
`templates/suprnova-ui/`: die Askama-Makro-View, das Stylesheet, das
JavaScript, sofern die Komponente eines hat, und das Manifest, das sie
benennt:

```bash
suprnova live:add field
suprnova live:add password-input
```

Eine Datei, die du bearbeitet hast, bleibt bei einem späteren Lauf erhalten;
`--force` ersetzt sie. Eine Komponente eines Drittanbieters wird mit
`--manifest` aus ihrem eigenen Manifest unter ihrer eigenen Wurzel
installiert. Rufe die Makros aus deinen Views auf, liefere Stylesheet und
Skript mit `try_live_ui_assets()` aus und binde sie im Dokument ein:

```html
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% call field::field("email", "Email", required=true) %}
{% call input::input("email", kind="email", required=true) %}{% endcall %}
{% endcall %}
```

Der Checker expandiert die Makros, sodass `live:check` eine Bibliotheks-View
wie jede andere beweist. Die Formularfamilie heute: Feld, Label, Eingabe,
Textbereich, Zahleneingabe, Schieberegler, Sucheingabe, Passworteingabe mit
Anzeige, Checkbox und Checkbox-Gruppe, Radiogruppe, Schalter, Auswahl, Button
und Link-Button, Button-Gruppe, Fieldset, Formularaktionen,
Validierungsübersicht und Dateieingabe. Bibliothekskomponenten heißen
`suprnova.*`, und die Registry weist dieses Präfix aus jeder anderen Crate
zurück; Custom Elements liegen im Light DOM und tragen das Präfix `sn-`.

Die Overlay-Familie erscheint auf denselben Grundlagen: Tooltip, Collapsible
und Accordion, Popover, ein einstufiges Dropdown-Menü, Dialog, Sheet und
Drawer. Jedes hält seinen Öffnungszustand über das Primitiv des Browsers, bevor
ein Skript läuft: `details` für Disclosures, das `popover`-Attribut für
Popovers und Menüs und `dialog` für die drei Modale, die die vendorierten
Elemente `sn-dialog`, `sn-sheet` und `sn-drawer` mit `showModal()` öffnen und
beim Schließen den Fokus an den Auslöser zurückgeben. Öffnen und Schließen
stellt nie eine Live-Anfrage; nur eine Aktion, die Sie in ein Overlay setzen,
tut das. Jede Overlay-Wurzel trägt einen stabilen Schlüssel und
`live:preserve.self`, sodass ein offenes Overlay einen Morph überlebt, der
seinen Bereich nicht ersetzt hat:

```html
{% import "suprnova-ui/dialog/dialog.html" as dialog %}
{% call dialog::dialog_trigger("confirm", "Delete everything", variant="danger") %}{% endcall %}
{% call dialog::dialog("confirm", "confirm", "Delete everything?") %}
<p>This removes every note.</p>
{% call button::button("Delete", action="confirm_delete", variant="danger") %}{% endcall %}
{% call dialog::dialog_close("confirm", "Cancel") %}{% endcall %}
{% endcall %}
```

Das `popover`-Attribut setzt die unterstützte Basis auf Chrome und Edge 114,
Firefox 128 und Safari 17. Wo CSS-Anker-Positionierung vorhanden ist, sitzen
Popover und Menü unter ihrem Auslöser; sonst zentriert der Browser sie. Der
Einzelöffnungsmodus des Accordions beruht auf `details name`, das ältere
unterstützte Versionen als unabhängige Disclosures behandeln.

Die Feedback-Familie und die Navigationsfamilie folgen. Feedback: Alert,
Skeleton, Spinner, Progress, Empty State und eine Toast-Region mit einer
Flash-Region daneben. Jede zeigt einen Zustand, den der Server oder die
Laufzeit bereits hält. Ein Alert wählt seine Rolle nach seiner Variante und
kennzeichnet jede Variante mit einem Zeichen und einer verborgenen
Beschriftung, nie nur mit Farbe. Ein Spinner oder Skeleton ist mit
`live:loading.show` an eine registrierte Aktion gebunden und verborgen
ausgeliefert, sodass die Laufzeit ihn nach ihrer eigenen Verzögerung zeigt und
über ihr Minimum hinaus hält; eine schnelle Aktion lässt ihn nie aufblitzen.
Progress ist das native `progress`-Element mit Beschriftung und Textablesung
und trägt einen Wert nur bei bestimmbarer Arbeit. Der Empty State nimmt seinen
Grund (leer, keine Treffer, keine Berechtigung, getrennt) aus servergerendertem
Zustand und bietet eine nächste Aktion nur dort, wo der Aufrufer eine
rendert. Ein Toast kündigt einmal aus einer höflichen Statusregion an und
nimmt nie den Fokus; das vendorierte Element `sn-toast-region` lässt Toasts
auslaufen, pausiert bei Hover oder Fokus, begrenzt, wie viele gleichzeitig
sichtbar sind, und beantwortet den Schließen-Button; jeder Toast ist
schlüsselbasiert und erhalten, sodass ein geschlossener Toast über einen Morph
hinweg geschlossen bleibt. Ein kritischer Fehler gehört auch in einen Alert; ein Toast ist nie seine einzige Fläche. Toasts werden in einer Schleife gerendert, daher laufen ihre Schlüssel durch den Filter `live_key`, und die Island, die sie mountet, stellt ihn mit `pub mod filters { pub use suprnova::view::filters::live_key; }` bereit. Die Flash-Region rendert einmal, was
die vorige Anfrage in der Session hinterlassen hat:

```html
{% import "suprnova-ui/alert/alert.html" as alert %}
{% import "suprnova-ui/spinner/spinner.html" as spinner %}
{% call alert::alert("saved", variant="success") %}<p>Your changes are saved.</p>{% endcall %}
{% call button::button("Save", action="save") %}{% endcall %}
{% call spinner::spinner(action="save", label="Saving") %}{% endcall %}
```

Navigation: Header-Leiste, Footer, Sidebar mit einklappbaren Gruppen,
Breadcrumbs, Tabs, Pagination und Load more. Jedes Ziel ist ein Anker mit
einer echten Routen-URL und jede Aktion ein Button; das aktuelle Element trägt
`aria-current` aus dem Wert, den Sie binden, nie aus dem Standort des
Browsers. Die Gruppen der Sidebar sind native `details`, schlüsselbasiert und
erhalten. Tabs verlangen einen Modus: `local` mit Tablist-Semantik,
Pfeiltasten aus dem vendorierten Element `sn-tabs` und keiner Anfrage beim
Wechsel, oder `route` mit Tabs als Ankern. Pagination verlangt ebenfalls einen
Modus: Routenseiten sind kanonische Links, Live-Seiten sind Buttons auf Ihren
Aktionen, deren Ergebnis die neue Query über `url_intent` in den aktuellen
History-Eintrag spiegelt, ohne Eintrag pro Seite. Load more ist ein Button auf
einer registrierten Aktion, der an eine schlüsselbasierte Liste anfügt, sodass
der Morph jede bereits vorhandene Zeile behält, und das Steuerelement verschwindet, sobald Sie es erschöpft rendern. Eine URL-Reflexion ist ein Ergebnis von Protokoll 2, daher deklariert eine Island, die über `url_intent` paginiert, `minimum_protocol_version = 2`; ihre Zeilen mit Schlüssel laufen wie ein Toast durch `live_key`:

```html
{% import "suprnova-ui/tabs/tabs.html" as tabs %}
{% call tabs::tabs("details", mode="local", label="Details") %}
{% call tabs::tab_list("Details") %}
{% call tabs::tab("tab-summary", "panel-summary", "Summary", selected=true) %}{% endcall %}
{% call tabs::tab("tab-history", "panel-history", "History") %}{% endcall %}
{% endcall %}
{% call tabs::tab_panel("panel-summary", "tab-summary", selected=true) %}<p>Summary</p>{% endcall %}
{% call tabs::tab_panel("panel-history", "tab-history") %}<p>History</p>{% endcall %}
{% endcall %}
```

Die Data-Display-Familie schließt den eingebauten Satz ab. Präsentational:
Separator, Scroll-Bereich, Aspect-Image, Card, Badge, Avatar und
Avatar-Gruppe, List Group, Description List und Stat Card. Jede bewahrt die
Dokumentreihenfolge und native Semantik: der Separator ist ein `hr` oder eine
beschriftete Separator-Rolle, der Scroll-Bereich eine fokussierbare,
beschriftete Region, die nativ scrollt, das Aspect-Image das `img` selbst mit
einem benannten Seitenverhältnis, die Card ein Article oder eine Section, die
von ihrer eigenen Überschrift beschriftet wird, mit Aktionen in einer
beschrifteten Gruppe, und die Description List ein `dl`. Ein Badge trägt
immer seinen Text, ein Avatar nennt seine Person im `alt` oder im Label
seiner Initialen, und der Trend einer Stat Card sagt "Up", "Down" oder "Flat"
im Text vor dem Delta, sodass kein Status allein auf Farbe ruht. Die List
Group verschlüsselt jedes Element über `live_key`, sodass eine Neuordnung
jeden Knoten behält. Das Chart wird auf dem Server gerendert: die Island ruft
`render_chart` aus `suprnova::live::charts` auf, das Balken- oder
Linienmarken über `charts-rs` aus begrenzten typisierten Reihen zeichnet und
vertrauenswürdiges Markup zurückgibt, und das Makro rendert das SVG neben
einer Textzusammenfassung und einer Datentabelle in einem Disclosure, sodass
das kanonische Dokument ohne das Bild lesbar ist und kein Chart-Skript je den
Browser erreicht:

```rust
use suprnova::live::charts::{ChartKind, ChartSeries, render_chart};

pub fn chart_svg(&self) -> TrustedHtml {
    render_chart(
        ChartKind::Bar,
        &["Apr", "May", "Jun"],
        &[ChartSeries::new("Revenue", vec![42.0, 47.0, 51.0])],
    )
    .expect("a bounded fixed series renders")
}
```

Die Datatable ist die letzte Komponente, mit einer Island pro Tabelle. Sie
ist eine native `table` mit einer Caption, die die Ergebniszahl nennt,
Spaltenköpfen mit `scope` und `aria-sort` auf der sortierten Spalte. Sortieren
und Filtern sind Live-Submits auf den Model-Feldern der Island, Seitenwechsel
sind Live-Buttons, und die Island deklariert die angewandte Sortierung, die
Richtung, den Filter und die Seite als `#[url]`-Felder und spiegelt sie nach
jeder Aktion über `url_intent`, sodass die Adresszeile immer eine teilbare
URL hält und das Dokument dieselbe Ansicht daraus mountet:

```rust
#[live(name = "app.invoices", view = "live/invoices.html", minimum_protocol_version = 2)]
pub struct Invoices {
    #[model]
    pub sort: String,
    #[url(key = "sort")]
    pub sorted_by: String,
    #[url(key = "dir")]
    pub direction: String,
    #[model]
    #[url(key = "filter")]
    pub filter: String,
    #[url(key = "page")]
    pub page: u64,
    pub rows: Vec<Invoice>,
}
```

Die Live-native-Familie ist die letzte: die Komponenten, die nur auf der laufenden Runtime Sinn ergeben. Das Upload-Widget stellt das mitgelieferte Upload-Protokoll dar: sein Dateifeld trägt `live:upload` für das Upload-Feld der Insel, sein `progress`-Element ist die Fortschrittswurzel der Runtime, und Abbrechen, Wiederholen und Entfernen wirken über `live:upload.cancel` und seine Geschwister auf die temporäre Referenz. Jeder Zustand, den die Domäne kennt, wird als Text gerendert und anhand von `data-live-upload-state` der Fortschrittswurzel angezeigt; "ready" liest sich als geprüft, aber nicht gespeichert, denn nichts ist dauerhaft, bevor die abschließende Aktion läuft:

```html
{% call upload::upload("attachment", "Attachment", accept="image/png") %}{% endcall %}
<button type="submit" live:loading.disabled="save_attachment">Save attachment</button>
```

Der Live-Feed und die Benachrichtigungsglocke sitzen auf einer stream-gestützten Insel. Die Runtime schreibt `data-live-stream-state` auf die Inselwurzel und kündigt jede Änderung in dem `[data-live-stream-status]`-Element an, das die Makros rendern (Updates disconnected, Connecting to updates, Updates current, Updates degraded, Reconnecting to updates, Updates closed), sodass ein degradierter, sich wiederverbindender oder geschlossener Stream das auch sagt und nur der aktuelle Zustand als aktuell gilt. Feed-Einträge laufen durch `live_key`. Das Kontomenü ist eine `details`-Aufklappung aus Ankern und einem Abmeldeformular, das mit dem CSRF-Token der Sitzung sendet; es ist ein Stitch-Slot unter RenderCache, also bindet eine Anwendung es als eigene identitätsgebundene Insel ein, und die geteilte Hülle enthält nie den Namen des Prinzipals.

Die Custom-Element-Stufe erweitert native Steuerelemente, die sie nie ersetzt. Jedes Element ist eine Light-DOM-Unterklasse von `HTMLElement`, die nur ihre eigene mitgelieferte Datei definiert, trägt das Präfix `sn-` und hält keinen Formularwert, denn das native Eingabefeld darin ist das Steuerelement: Blockiert man das Skript, sendet das Formular denselben Wert. Die Einmalcode-Eingabe ist ein einziges natives Eingabefeld (`inputmode="numeric"`, `autocomplete="one-time-code"`, ein Längenmuster) an einem transienten Modell, und `sn-input-otp` spiegelt die getippten Zeichen in `aria-hidden`-Zellen. Der Datumswähler ist ein `type="date"`-Eingabefeld, und seine Jahres-, Monats- und Tagesleisten sind Fieldsets aus nativen Radiobuttons in CSS-Scroll-Snap-Containern, sodass Tippen, Klicken und Pfeiltasten ohne Skript auswählen; `sn-date-picker` setzt eine vollständige Auswahl in das Eingabefeld zusammen. Die Combobox ist das barrierefreie Combobox-Muster (`role="combobox"`, `aria-expanded`, `aria-activedescendant`, eine `role="listbox"` mit Optionen) über einem nativen Eingabefeld mit einer `datalist` für den skriptfreien Fall; `sn-combobox` filtert, bewegt die aktive Option und wählt aus und verweigert eine Listbox, deren `data-sn-query` nicht der aktuelle Text des Eingabefelds ist, sodass ein veraltetes Ergebnis nie die Ergebnisse einer neueren Abfrage ersetzt:

```html
{% call otp::input_otp("code", "One-time code") %}{% for index in cells %}{% call otp::otp_cell(index) %}{% endcall %}{% endfor %}{% endcall %}
{% call date::date_picker("when", "Renewal date", years, months, days, min="2026-01-01", max="2028-12-31") %}{% endcall %}
{% call combo::combobox("country", "Country", countries, query=country, placeholder="Type a country") %}{% endcall %}
```

### Warum Suprnova abweicht

Laravel liefert Blade-Komponenten und das Markup eines Starter-Kits; Suprnova
liefert die Bibliothek über das Framework selbst, im eigenen Vokabular von
Live, ohne dass eine Client-Anwendung die Seite besitzt. Die Optik ist
standardmäßig an und lässt sich entfernen, ohne dass etwas bricht; das ist es,
was headless hier bedeutet.

## Testen

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

Live läuft vollständig ohne RenderCache. Das Caching von Live-Dokumenten ist
Aufgabe von RenderCache; siehe [RenderCache](render-cache.md).

## CLI-Referenz

| Befehl | Zweck |
|---|---|
| `suprnova live:make <name>` | Eine Komponente und ihre View erzeugen und registrieren |
| `suprnova live:check` | Jede registrierte View mit dem integrierten Checker beweisen |
| `suprnova live:inspect` | Sicheren Laufzeit-, Registry-, Provider- und Artefaktzustand melden |
| `suprnova live:assets --out <dir>` | Die geprüften Laufzeit-Artefakte atomar veröffentlichen |

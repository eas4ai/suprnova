# Inertia Responses

Inertia-Responses sind die Art, wie ein Suprnova-Handler Zustand an
eine Svelte-/React-/Vue-Seiten-Komponente schickt. Jeder Handler, der
eine Inertia-Seite rendert, liefert eine zurück, gebaut entweder über
das [`inertia_response!`](#das-inertia-response-makro)-Makro (für
typisierte, zur Compile-Zeit geprüfte Eager Props) oder den
[`InertiaResponse`](#der-inertiaresponse-builder)-Builder (für alles
andere - Lazy Props, Deferred Props, Merge, Once, Scroll, Flash).
Dieses Kapitel deckt die Response-Oberfläche end-to-end ab: das
Makro, den Builder, die v3-Protokoll-Features (Partial Reloads,
History-Verschlüsselung, Versionserkennung), gemeinsame Daten über
`App::inertia_share*` und die Flash-Bag, die über Redirects
weitergetragen wird.

Wenn Sie noch kein Frontend gewählt haben, kommen zuerst
[Frontend - Übersicht](frontend.md) und
[Seiten-Komponenten](frontend-pages.md); dieses Kapitel setzt voraus,
dass die SPA-Brücke verdrahtet ist, und konzentriert sich darauf, was
Ihr Handler zurückliefert.

## Das `inertia_response!`-Makro

Das Makro ist der kürzeste Weg von einem Handler zu einer typisierten
Eager Page. Es nimmt die aktuelle Anfrage, einen Komponentennamen und
einen Props-Ausdruck:

```rust
use suprnova::{Request, Response, inertia_response, InertiaProps};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        message: "Hello from Suprnova!".into(),
    })
}
```

Drei Dinge, die Sie wissen sollten:

- **Das führende `&req` ist erforderlich.** Das Makro liest die
  `X-Inertia`-Header, die URL und die Filter-Header für Partial Reloads
  von der Anfrage, es braucht also den Request-Wert (oder eine
  Referenz). Ohne ihn würden Partial Reloads still kaputtgehen.
- **Die Existenz der Komponente wird zur Compile-Zeit geprüft.** Das
  Makro sucht nach
  `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`; passt keine
  Datei, scheitert der Build mit einem „Meinten Sie …?“-Vorschlag, der
  aus den tatsächlichen Dateinamen auf der Platte stammt.
  Verschachtelte Pfade funktionieren genauso -
  `inertia_response!(&req, "Admin/Dashboard", …)` löst
  `frontend/src/pages/Admin/Dashboard.svelte` auf (oder die Endung Ihres
  Frontends).
- **Das Makro expandiert zu einem `await`eten `Result`.** Ihr Handler
  muss [`Response`](error-model.md) zurückgeben (was
  `Result<HttpResponse, HttpResponse>` ist) oder einen anderen Typ, der
  `FrameworkError` über `?` / `From` aufnimmt. Fehlschläge bei der
  Prop-Serialisierung oder beim Bau der Response werden als `Err`
  zurückgegeben, nicht als Panics.

### Props im JSON-Stil

Für Prototypen und winzige Seiten können Sie die typisierte Struktur
weglassen:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

Das Makro validiert weiterhin die Komponentendatei. Der Kompromiss ist,
dass Sie die typisierte Prop-Kette verlieren - kein
`#[derive(InertiaProps)]`, keine automatische TypeScript-Generierung,
keine Prüfung zur Compile-Zeit, dass die vom Frontend erwartete Form
passt.

### Optionales Config-Override

Das Makro akzeptiert eine optionale abschließende `InertiaConfig` für
Overrides pro Response (andere SSR-Einstellungen, ein eigener
Standardtitel für eine Seite):

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

Die meisten Apps registrieren beim Boot eine einzige Config über
[`Inertia::install`](#bootstrap-inertia-install) und fassen dieses
Argument nie an - die installierte Config ist bereits das, womit jede
Response startet. Übergeben Sie hier nur dann eine, wenn Sie die
installierte Config für eine einzelne Seite überschreiben wollen.

## `#[derive(InertiaProps)]`

`InertiaProps` gibt eine `Serialize`-Impl aus, deren Schlüsselnamen
zu Ihren Feldnamen passen. Es existiert, damit der typisierte
Props-Pfad knapp bleibt und der TypeScript-Generator (`suprnova
generate-types`) einen Marker zum Finden hat:

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
pub struct UserProps {
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
}
```

Verschachtelte Typen komponieren sich normal - Felder können
`Vec<T>`, `Option<T>`, verschachtelte Strukturen oder alles sein,
was `Serialize`-fähig ist. Die verschachtelten Typen selbst müssen
`InertiaProps` nicht ableiten; sie brauchen nur `Serialize`.
Verwenden Sie `#[derive(InertiaProps)]` auf der
*Top-Level*-Props-Struktur, und Sie bekommen die automatische
TypeScript-Oberfläche (siehe [TypeScript Types](frontend-typescript-types.md))
für den gesamten Baum.

## Der `InertiaResponse`-Builder

Das Makro deckt typisierte Eager Props ab. Alles andere - Lazy,
Optional, Deferred, Mergeable, clientseitig gecacht, Flash, Overrides
zur History-Verschlüsselung - nutzt den Builder direkt:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: Die Closure läuft nur, wenn die Prop tatsächlich gesendet wird
        // (Erstbesuch oder Partial Reload, der diesen Schlüssel anfragt).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: Wird bei Erstbesuchen nie gesendet; der Client muss den
        // Schlüssel explizit über X-Inertia-Partial-Data anfordern.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: Beim ersten Rendern übersprungen; der Client stellt einen
        // Folge-XHR, und dann läuft die Closure.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: Bei Partial Reloads an Bestehendes anhängen („mehr laden“).
        .merge("rows", next_page().await?)
        // Once: Clientseitig über Navigationen hinweg gecacht; der Resolver
        // wird bei Folgebesuchen übersprungen, sofern der Server kein
        // Refresh erzwingt.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: einmaliger Toast; erscheint unter `page.flash`, nicht `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Methode | Zweck | Entspricht in Laravel |
|---|---|---|
| `.with(k, v)` | Eager Prop, respektiert das Filtern bei Partial Reloads | typisierte Prop |
| `.always(k, v)` | Eager Prop, ignoriert die Partial-Reload-Filter | `Inertia::always(…)` |
| `.lazy(k, ‖)` | Resolver läuft nur, wenn die Prop gesendet wird | Closure `fn () => …` |
| `.optional(k, ‖)` | Nie beim Erstbesuch; muss explizit angefordert werden | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Beim Erstbesuch übersprungen; ein Folge-XHR löst die Auflösung aus | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Bei Partial Reloads mit bestehendem Client-Zustand kombinieren | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | Der Client cacht über Navigationen hinweg | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate` (über `Inertia::paginate`) | Infinite-Scroll-Paginierung | `Inertia::scroll(…)` |
| `.flash(k, v)` | Einmaliger Wert unter `page.flash` (nicht `props`) | `session()->flash(…)` |
| `.title(…)` | Standard-`<title>` für die HTML-Shell | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | History-Verschlüsselung pro Response | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Erzwingt die Rotation des History-Schlüssels auf **dieser** Seite | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | `#fragment` nach einem Inertia-Besuch behalten | `Inertia::preserveFragment()` |

Eager-Builder-Methoden haben `try_*`-Geschwister (`try_with`,
`try_always`, `try_merge_with`, `try_scroll`, `try_flash`), die
`Result<Self, FrameworkError>` liefern, wenn die `Serialize`-Impl eines
Werts zur Laufzeit fehlschlagen könnte - die unfehlbaren Methoden
wandeln den Panic über [die Panic-Grenze](error-model.md) in ein 500
um; greifen Sie also zu `try_*`, wenn Sie den Fehlschlag lieber
explizit behandeln.

`.clear_history()` markiert die Response, die Sie gerade bauen. Ein
Logout-Handler leitet weiter, und der Browser verwirft die Response des
Redirects - also ist die Login-Seite diejenige, die das Flag tragen
muss, nicht die Logout-Response. `App::clear_history()` ist der Fix für
diesen Fall - es ist eine freie Funktion, keine Builder-Methode, und
steht deshalb nicht in der Tabelle oben. Es flasht ein einmaliges
Session-Flag, das das nächste Inertia-Page-Objekt in
`clearHistory: true` verwandelt. Es braucht einen Session-Scope, und es
überlebt genau einen Sprung.

Rufen Sie es **nach** `Auth::logout()` /
`Auth::logout_and_invalidate()` auf, nicht davor - die Invalidierung
leert die gesamte Session, und das Flag lebt in dieser Session, es
zuerst zu flashen führt also nur dazu, dass die Leerung es wieder
löscht:

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### Merge-Strategien und Infinite Scroll

`.merge` (anhängen), `.merge_prepend` und `.deep_merge` decken die
gängigen „mehr laden“-Fälle ab. Für ein Diff-Merge - Zeilen
aktualisieren, die der Client bereits hält, statt sie zu duplizieren -
greifen Sie zu `.merge_with` mit einer expliziten `MergeStrategy`, die
einen `match_on`-Schlüssel trägt:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // der neue Seitenausschnitt
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` benennt das Feld, auf dem der Client dedupliziert (im
Page-Objekt als `matchPropsOn` ausgegeben), sodass ein erneutes
Abrufen, das sich mit dem aktuellen Fenster überschneidet, passende
Zeilen an Ort und Stelle ersetzt, statt Kopien anzuhängen. `Prepend` und
`Deep` nehmen dasselbe `match_on`.

Infinite Scroll ist dieselbe Maschinerie mit angehängten
Pagination-Metadaten. `.scroll` / `.scroll_with` - oder `.paginate`, das
einen `LengthAwarePaginator` oder `CursorPaginator` direkt adaptiert -
geben `scrollProps` neben den Daten aus, und die
`<InfiniteScroll>`-Komponente des Clients treibt die Abrufe für vorwärts
und rückwärts:

```rust
// `posts` ist ein CursorPaginator aus dem Query Builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

Das Framework liest die Merge-Richtung aus dem Request-Header
`X-Inertia-Infinite-Scroll-Merge-Intent`, den der Client sendet
(`append` beim Herunterscrollen, `prepend` beim Hochscrollen). Bei einem
frischen Besuch - ohne Intent-Header - ist `scrollProps["posts"].reset`
gleich `true`, sodass der Client seinen Akkumulator leert, bevor er das
erste Fenster rendert.

## Partial Reloads

Der Inertia-3-Client kann eine Teilmenge der Props einer Seite anfordern
(oder eine Obermenge, indem er einen Optional- oder Defer-Schlüssel
einschließt). Das Protokoll verwendet drei Request-Header:

| Header | Bedeutung |
|---|---|
| `X-Inertia-Partial-Component` | Die Komponente, die partiell neu geladen wird - sie muss mit der Komponente der Response übereinstimmen, damit gefiltert wird. |
| `X-Inertia-Partial-Data` | Whitelist: kommagetrennte Prop-Schlüssel, die eingeschlossen werden. |
| `X-Inertia-Partial-Except` | Blacklist: kommagetrennte Prop-Schlüssel, die ausgeschlossen werden. Gewinnt bei einer Schlüsselkollision gegen `Partial-Data`. |

Filterregeln:

- `Eager`-, `Lazy`-, `Merge`-, `Once`- und `Scroll`-Props folgen der
  Whitelist-/Blacklist-Semantik.
- `Always`-Props werden in jedem Fall gesendet.
- `Optional`- und `Defer`-Props sind bei einem normalen Besuch nie dabei
  und erscheinen nur bei einem passenden Partial Reload, der den
  Schlüssel explizit auflistet.

Der Handler muss nichts Besonderes tun - registrieren Sie jede Prop über
den Builder, und das Framework zieht beim Serialisieren des Page-Objekts
die Header heran.

Der clientseitige Cache einer `once`-Prop wird nur bei einem
**vollständigen** Inertia-Besuch respektiert. Bei einem Partial Reload,
der den Schlüssel nennt (`router.reload({ only: ['stats'] })`), läuft
der Resolver, und der Wert wird gesendet - der Client hat genau deshalb
gefragt, weil er einen frischen will, und seine Behauptung eines
veralteten Caches dort zu respektieren würde für den angefragten
Schlüssel überhaupt nichts zurückgeben.

## Gemeinsame Daten über `App::inertia_share*`

Manche Props sind auf jeder Inertia-Seite gleich - Auth-Zustand, das
CSRF-Token, das aktuelle Locale, app-weite Flags. Registrieren Sie
sie einmal beim Bootstrap, und sie werden in jede Response gemischt:

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // Sync, einmal beim Boot materialisiert.
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // Async, pro Response aufgelöst (ausgelassen von Partial
    // Reloads, die den Schlüssel ausschließen).
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // Client-seitig über Navigationen hinweg gecacht - `share_once`
    // läuft auf der ersten Seite, die es braucht, danach
    // überspringt der Client die erneute Auflösung über
    // `X-Inertia-Except-Once-Props`, bis sich der Cache-Schlüssel
    // ändert.
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    });
}
```

Für Pro-Request-geteilte Daten (der authentifizierte Benutzer,
Request-Scoped Flags) implementieren Sie
[`InertiaSharedData`](#pro-request-gemeinsame-daten) und registrieren
das Singleton - das Framework ruft `share(&req)` bei jeder
Inertia-Response auf und mischt das Ergebnis ein.

### Vorrang bei einer Schlüsselkollision

Erscheint derselbe Schlüssel in mehr als einer Schicht, gewinnt der
spätere Schreibvorgang:

1. Statische Registry (`App::inertia_share` / `App::inertia_share_lazy`)
2. Pro-Request-Trait-Provider (`InertiaSharedData::share`)
3. Pro-Response-Builder-Methoden (`.with`, `.lazy`, usw.)

Das erlaubt einem Handler, einen global geteilten Standard für eine
Seite zu überschreiben, ohne irgendetwas deregistrieren zu müssen.

### Pro-Request gemeinsame Daten

Das Trait läuft einmal pro Inertia-Response mit Zugriff auf die
Anfrage. Implementierungen brauchen `async_trait` (re-exportiert als
`suprnova::__async_trait`) und `IndexMap` (re-exportiert als
`suprnova::indexmap`):

```rust
use suprnova::{
    App, Auth, FrameworkError, InertiaRequestExt, InertiaSharedData, Prop,
    indexmap::IndexMap,
};
use std::sync::Arc;

pub struct AuthShare;

#[suprnova::__async_trait]
impl InertiaSharedData for AuthShare {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::Eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        Ok(out)
    }
}

// Im Bootstrap:
App::register_inertia_shared(Arc::new(AuthShare));
```

## Flash und Redirects

Flash-Daten sind einmaliger Zustand, der beim nächsten Rendern
erscheinen und danach verschwinden soll - Toast-Nachrichten, IDs von
„gerade erstellt“, Validierungs-Zusammenfassungen. Suprnova legt sie
bei jeder Inertia-Response unter `page.flash` offen. Es gibt drei
Schreiber:

```rust
// 1. In die Flash-Bag der aktuellen Anfrage legen.
App::flash("toast", "Saved");

// 2. An eine bestimmte Response hängen (gleicher Effekt, nur auf dieser Response).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Über einen Redirect hinweg mitführen, via Redirect-Facade.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

Die Form `Redirect::with(key, value)` ist der handlerübergreifende Weg:
Der Wert landet in der Session unter `_flash.new.*`, die
[`SessionMiddleware`](csrf.md) der nächsten Anfrage lässt ihn zu
`_flash.old.*` altern, und die `InertiaResponse` des Ziels legt ihn
unter `page.flash` offen.

Flash aus derselben Anfrage (die task-lokale Bag) gewinnt bei einer
Schlüsselkollision gegen geerbtes Session-Flash, sodass ein
Ziel-Handler einen eingehenden Wert einfach durch erneutes Flashen des
Schlüssels überschreiben kann.

Interne Session-Schlüssel (alles mit `_`-Präfix) werden aus
`page.flash` herausgefiltert - `_old_input` für das erneute Befüllen von
Formularen und `_inertia.*`-Protokoll-Flags dringen nicht zum Client
durch.

### Redirect-Helfer

`Redirect` ist die vollständige Laravel-Oberfläche:

```rust
Redirect::to("/dashboard")                       // 302 auf einen Pfad
Redirect::route("posts.show").with("id", "42")   // benannte Route, Routenparameter
Redirect::back("/")                              // in der Session vermerkte vorherige URL
Redirect::refresh()                              // dieselbe URL, frisches GET
Redirect::guest(&req, "/login")                  // legt die beabsichtigte URL beiseite
Redirect::intended("/dashboard")                 // holt die beiseitegelegte URL
Redirect::signed_route("downloads.show", &[("id","42")])?  // signierte URL
Redirect::to("/posts/42").preserve_fragment()    // #frag über den Besuch behalten
```

Alle `Redirect`-Varianten akzeptieren `.with(k, v)`, `.with_input(map)`,
`.with_errors(map)`, `.with_errors_bag(name, map)`, `.cookie(c)`,
`.header(k, v)`, `.permanent()`, `.status(303)` usw. Die vollständige
Kette spiegelt Laravels `RedirectResponse`.

Bei Non-GET-Inertia-Besuchen wandelt das Framework die Response
automatisch in ein `303 See Other` um, wenn
[`Inertia303Middleware`](#bootstrap-inertia-install) installiert ist,
sodass der Browser ein sauberes Folge-GET absetzt, statt das
ursprüngliche PUT/PATCH/DELETE erneut an das Redirect-Ziel zu senden.

Um den Besucher **aus** der Inertia-App hinauszuschicken - zu einem
Zahlungs-Provider, einem OAuth-Authorize-Endpunkt, einem gehosteten
Billing-Portal - verwenden Sie `location_for`:

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

Ein Inertia-XHR bekommt `409` + `X-Inertia-Location` (der Client führt
`window.location = url` aus); eine harte Navigation bekommt ein
einfaches `302` + `Location`. Das nackte
`InertiaResponse::location(url)` liefert immer die 409-Form - verwenden
Sie es nur dort, wo bereits bekannt ist, dass die Anfrage ein
Inertia-Besuch ist, denn ein Browser, der einem `409` ohne
`Location`-Header folgt, hat kein Ziel.

## Versionserkennung

Inertia versioniert das Asset-Manifest, damit ein langlebiger Client
nicht versucht, eine Seite aus dem Bundle von gestern gegen den
heutigen Server zu mounten. Wenn der `X-Inertia-Version`-Header des
Clients nicht zur konfigurierten Version des Servers passt, antwortet
[`InertiaVersionMiddleware`](#bootstrap-inertia-install) mit
`409 Conflict` und einem `X-Inertia-Location`-Header, der die neue URL
benennt - der Inertia-Client greift das auf und macht ein vollständiges
Neuladen der Seite, wodurch er das neue Bundle bekommt.

Der Bounce flasht zuerst die Session erneut. Der Client beantwortet ein
409 mit einem vollständigen Seiten-GET, und dieses GET ist eine frische
Anfrage - ohne das erneute Flashen altert ein von der vorherigen
Anfrage geflashter Validierungsfehler oder eine Erfolgsmeldung weg,
bevor die Zielseite ihn lesen kann, und der Nutzer verliert seine
Fehlermeldung allein deshalb, weil mitten im Absenden ein Deploy
gelandet ist. Dafür muss `SessionMiddleware` vor der
Versions-Middleware registriert sein.

Die Version setzen Sie über `InertiaConfig`:

```rust
use suprnova::InertiaConfig;

// Statisch - für die meisten Apps. Einen Identifier aus der Build-Zeit einbacken.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamisch - einen Manifest-Hash lesen, eine Container-Deployment-ID, was auch immer.
// Die Closure läuft bei jeder Versionsprüfung; cachen Sie darin, falls sie nicht günstig ist.
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

Für asynchrone oder fehlbare Versionsauflösung (z. B. das Lesen eines
Manifest-Hashes aus S3) führen Sie den Lesevorgang einmal beim Boot aus
und übergeben den gecachten `String` an `.version(...)`.

## Bootstrap: `Inertia::install`

Die meisten Apps installieren die drei Protokoll-Middlewares in einem
Aufruf:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register() -> Result<(), suprnova::FrameworkError> {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)?;
    // …weitere gemeinsame Daten, Routen usw.
    Ok(())
}
```

`Inertia::install` liefert `Result` und tut, in dieser Reihenfolge:

1. Schlägt geschlossen fehl, wenn `cfg` in den Produktionsmodus auflöst
   (`development == false` - der Standard, sobald `APP_ENV=production`),
   aber kein Vite-Manifest aus `cfg.manifest_path` geladen werden kann.
   Das ist die CFG-01-Absicherung: Ein Produktions-Boot mit ungebautem
   Frontend scheitert sichtbar, statt still auf einen alten, fest
   verdrahteten Asset-Pfad zurückzufallen.
2. Registriert `InertiaHeadersMiddleware` - setzt `Vary: X-Inertia` auf
   jeder Response und verwandelt eine leere `200` bei einem
   Inertia-Besuch in ein `303` zurück.
3. Registriert `InertiaVersionMiddleware` - gibt das `409` +
   `X-Inertia-Location` aus, wenn Client und Server sich über die
   Asset-Version uneinig sind.
4. Registriert `Inertia303Middleware` - hebt bei
   Non-GET-Inertia-Redirects `302` auf `303` an.

Die Reihenfolge zählt: Die Header-Middleware wird zuerst registriert,
ist also die äußerste und sieht jede Response - einschließlich des
`409`, das die Versions-Middleware zurückgibt, bevor der Handler
überhaupt läuft.

`install` **behält außerdem die Config**. Jede danach gebaute
`InertiaResponse` startet von ihr, sodass hier gesetzte
`.frontend(...)`, `.version(...)`, `.default_title(...)`, `.ssr(...)`
und `.encrypt_history(...)` jede Seite erreichen, ohne dass ein Handler
etwas übergibt. Ein Handler, der für eine Seite andere Einstellungen
will, überschreibt weiterhin mit `.with_config(...)`; eine App, die
`Inertia::install` nie aufruft, bekommt `InertiaConfig::default()`; und
ein erneuter Aufruf von `install` ersetzt die behaltene Config.

`.with_config(...)` ersetzt die Config vollständig, `version`
eingeschlossen. `InertiaVersionMiddleware` löst weiterhin die Version
auf, die `Inertia::install` bekommen hat, sodass eine Config hier, die
nicht dasselbe `.version(...)` trägt, das Page-Objekt eine Version
angeben lässt, die die Middleware mit einem Bounce beantwortet - der
Client nimmt nach dem Besuch dieser Seite einen zusätzlichen
vollständigen Seitenaufbau in Kauf. Setzen Sie `.version(...)` auf dem
Override passend dazu.

Registrieren Sie `SessionMiddleware` **vor** `Inertia::install`, wenn
Sie Flash-Daten verwenden. Die Versions-Middleware flasht die Session
erneut, bevor sie den Client zurückwirft, sodass ein geflashter Fehler
das folgende vollständige Seiten-GET überlebt; sie kann das nur
innerhalb eines Session-Scopes.

Lassen Sie den Aufruf nur aus, wenn Sie eine dieser Middlewares
wirklich nicht wollen (selten; alle drei schließen echte Fehlermodi -
Cache Poisoning über die zwei Repräsentationen einer URL, ein stilles
veraltetes Bundle und Formular-Replay beim Redirect).

## Serverseitig gesteuerte `<head>`-Elemente

Inertia 3.5 hat eine Client-Option hinzugefügt, die den Server
entscheiden lässt, was in `<head>` landet - nützlich, wenn Meta-Tags
von der Zeile abhängen, die Sie gerade geladen haben, und Sie nicht
wollen, dass Titel und OG-Tags an zwei Stellen leben.

Das braucht keine Framework-Unterstützung. Der Client liest die
Elemente aus einer **gewöhnlichen Prop**, sodass jeder Handler sie
liefern kann:

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>) -> Response {
    Ok(inertia_response!("Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    }))
}
```

Opt-in auf dem Client:

```js
createInertiaApp({
  serverHead: true,        // liest die `head`-Prop
  // serverHead: 'meta',   // oder eine anders benannte Prop lesen
  // serverHead: (page) => [...],  // oder aus der gesamten Seite berechnen
})
```

Jeder String ist ein HTML-Element. Der Client stempelt ein
`data-inertia`-Attribut auf alles, dem eines fehlt, damit er
`head`-Elemente über Navigationen hinweg diffen kann; liefern Sie
Ihr eigenes `data-inertia="og-title"`, wenn Sie stabile Identität
statt positionsbasiertem Matching wollen.

Escapen Sie alles, was aus Benutzerdaten interpoliert wird - diese
Strings werden als HTML injiziert, es gelten also die üblichen
Regeln.

## SSR

Suprnova spricht über HTTP-Loopback mit einem SSR-Worker außerhalb des
Prozesses - typischerweise dem `createServer()`-Bundle aus
`@inertiajs/{svelte,react,vue}/server`, ausgeführt unter Node / Bun /
Deno. Aktivieren Sie ihn auf der Config, die Sie
[`Inertia::install`](#bootstrap-inertia-install) übergeben - diese Config
ist das, womit jede Response startet, es gibt also nichts durch Ihre
Handler zu fädeln:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // Worker-URL
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

SSR ist standardmäßig aus, und es ist eine Eigenschaft der Config: an
für jede Response, die aus der installierten Config gebaut wird, aus für
jede Response, die mit einem `.with_config(...)` überschreibt, das es
nicht setzt. Ist es aktiviert, postet das Framework das Page-Objekt an
`<url>/render` und bettet `{ head, body }` in die HTML-Shell ein. Bei
einem Worker-Fehler oder Timeout fällt die Response auf CSR zurück (ein
leeres `<div id="app">`, das der Client hydriert), und der Hook
`on_ssr_error(...)` feuert; schalten Sie in der CI
`ssr_throw_on_error(true)` um, damit diese Fehlschläge stattdessen harte
500er werden.

Booten Sie den Worker separat - `suprnova ssr:start` ist der
Standard-Runner, sobald Ihr Projekt einen SSR-Einstieg ausliefert.

## Konfiguration

Inertias Verhalten wird programmatisch über `InertiaConfig`
konfiguriert, und die Config, die Sie
[`Inertia::install`](#bootstrap-inertia-install) übergeben, ist die, von
der jede Response startet. Die eine Umgebungsvariable, die das Framework
direkt liest, ist `SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`), und
sie liefert nur den Standard-Dateinamen des Einstiegspunkts und die
Endungen der Seiten-Komponenten, wenn die Config nichts dazu sagt - ein
explizites `.frontend(Frontend::React)` auf der installierten Config
gewinnt, und genau das scaffoldet `suprnova new --frontend react`. Alles
Übrige ist Builder-förmig:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // überschreibt SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // deckelt den Lazy-Prop-Fan-out
    .url_resolver(|req| req.path_and_query()) // wie `page.url` abgeleitet wird
    .production();                            // false → lädt vom Vite-Dev-Server
```

Frontend-spezifische Standardwerte:

| Frontend | Standard-Einstiegspunkt | Seiten-Endungen |
|---|---|---|
| Svelte (Standard) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

### Das Feld `url`

`page.url` ist der Pfad **und** der Query-String der Anfrage
(`/users?page=2&sort=name`). Der Client schreibt es in `history.state`,
es ist also das, was Vor-/Zurück-Navigation und `router.reload()` erneut
abspielen - lassen Sie die Query weg, und jede paginierte oder
gefilterte Seite setzt sich still auf Seite eins zurück.
`InertiaVersionMiddleware` leitet sein `X-Inertia-Location` ebenfalls aus
Pfad und Query der Anfrage ab, sodass ein 409-Bounce wegen der
Asset-Version den Browser standardmäßig genau auf der URL landen lässt,
die das Page-Objekt benannt hat.

Überschreiben Sie die Ableitung mit `url_resolver`, wenn die URL, die
der Client festhalten soll, von der abweicht, die ankam - ein
Locale-Präfix, auf das die SPA nicht routet, oder ein Pfad, den ein
Reverse Proxy umgeschrieben hat:

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

Der Resolver liest die Anfrage über `InertiaRequestExt` und gilt für
jede Response, die aus der Config gebaut wird, die Sie
[`Inertia::install`](#bootstrap-inertia-install) übergeben - der übliche
Ort für einen Resolver, der app-weit gelten soll. Überschreiben Sie ihn
für eine einzelne Response mit `InertiaResponse::with_config(cfg)`. Ein
Resolver ändert nur `page.url`. Der 409-Bounce benennt weiterhin die
URL, die tatsächlich ankam - das ist die URL, die der Browser holen
muss -, sodass die beiden mit einem Resolver bewusst auseinanderlaufen.

Das Vite-Manifest unter `manifest_path` wird bei der ersten Anfrage lazy
geladen und für die Lebensdauer des Prozesses gecacht - jede Response,
die aus der installierten Config gebaut wird, teilt sich diesen einen
Cache, die Datei wird also einmal gelesen und geparst. Fehlt es, fallen
Produktions-Asset-Tags auf einen fest verdrahteten alten Pfad zurück,
und ein `tracing::warn!` feuert, damit die Lücke in den Logs sichtbar
wird.

### Warum Suprnova abweicht

Laravels Inertia-Adapter hat eine einzige globale Registry für
„gemeinsame Daten“ plus einen `Inertia::share($k, $v)`-Aufruf pro
Anfrage. PHPs Modell mit einem Prozess pro Anfrage macht das sicher: Ein
frischer Prozess pro Anfrage bedeutet kein Durchsickern zwischen
gleichzeitigen Besuchern.

Rusts Prozessmodell ist das Gegenteil - ein Prozess bedient viele
gleichzeitige Anfragen über viele Threads. Deshalb lebt die Registry auf
dem [Service Container](container.md) (task-lokal → thread-lokal →
global), nicht in prozessglobalen Statics. `App::inertia_share*`
schreibt in die `InertiaRegistry` des aktiven Containers, was Tests mit
`TestContainer::fake()` saubere Isolation gibt, ohne dass irgendetwas
deregistriert werden müsste. Gleiche Oberfläche wie Laravel; andere
Maschinerie darunter, weil die Runtime eine andere ist.

Fünf weitere Rust-förmige Entscheidungen, die es wert sind, benannt zu
werden:

- **Lazy-Prop-Resolver laufen nebenläufig**, gedeckelt durch
  `max_concurrent_resolvers` (Standard 16). Eine Seite mit zwölf Lazy
  Props setzt zwölf parallele Queries innerhalb einer Tokio-Task ab -
  genau dafür haben wir das Framework auf Tokio gebaut. Justieren Sie
  die Obergrenze, wenn eine Seite viele Lazy Props hat, die jeweils
  einen externen Dienst treffen.
- **Die Komponentenprüfung zur Compile-Zeit** ist überhaupt kein
  Laravel-Feature, weil PHP Ihre Frontend-Dateien zur Compile-Zeit nicht
  sehen kann. Suprnova kann es, sodass ein Tippfehler in
  `inertia_response!("Dashbaord", …)` den Build mit einem „Meinten Sie
  Dashboard?“-Vorschlag scheitern lässt, statt später zur Laufzeit als
  „Komponente nicht gefunden“ aufzutauchen.
- **Eine leere `200` bei einem Inertia-Besuch wird zu einem `303`, nicht
  zu einem `302`.** Laravels `onEmptyResponse` liefert
  `redirect()->back()` (ein 302) und verlässt sich auf seine spätere
  `302 → 303`-Umwandlung nur für PUT/PATCH/DELETE. Ein ersetzter
  Redirect ist nie eine Fortsetzung der ursprünglichen Methode - der
  Client muss ein GET absetzen -, also sagt Suprnova direkt `303`, statt
  GET-Besuche auf einem 302 zu lassen, dem der Client mit dem
  ursprünglichen Verb folgen würde.
- **`Inertia::location($url)` sind hier zwei Methoden, nicht eine.**
  `location(url)` behält Laravels Vertrag mit immer `409` - es ist älter
  als die request-bewusste Form, und Konsumenten mit gepinnten Tags
  verlassen sich darauf, dass sich diese Form nicht ändert.
  `location_for(&req, url)` ist die neuere, request-bewusste Form: `409`
  für ein Inertia-XHR, einfaches `302` für eine harte Navigation.
  Greifen Sie in neuem Code zu `location_for`.
- **`Inertia::clearHistory()` sind hier ebenfalls zwei Methoden, nicht
  eine.** `.clear_history()` auf dem Builder markiert eine einzelne
  Response; `App::clear_history()` flasht das Flag in die Session,
  sodass es einen Redirect überlebt. Laravel kommt mit einer Methode
  durch, weil es ohnehin session-gestützt ist - Suprnova behält die
  response-lokale Form als Standard (keine Session-Abhängigkeit) und
  macht den redirect-übergreifenden Fall stattdessen zu einem expliziten
  Opt-in.

## Nächste Schritte

- [Seiten-Komponenten](frontend-pages.md) - wie das Frontend einen
  Komponentennamen zu einem Svelte-/React-/Vue-Modul auflöst
- [TypeScript Types](frontend-typescript-types.md) - `suprnova
  generate-types` gibt TS-Definitionen aus Ihren
  `#[derive(InertiaProps)]`-Strukturen aus
- [Datenobjekte](data.md) - `#[derive(Data)]` für DTOs mit
  Pro-Feld-Include-/Allowlist-Steuerung, die sich mit Partial
  Reloads kombiniert
- [Fehlermodell](error-model.md) - wie `Response`, die Panic-Grenze
  und `FrameworkError` durch Inertia-Responses fädeln
- [Service Container](container.md) - das Lookup-Modell hinter
  `App::inertia_share*` und `InertiaSharedData`

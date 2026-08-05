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

Das Makro ist der kürzeste Weg von einem Handler zu einer
typisierten Eager-Seite. Es nimmt die aktuelle Anfrage, einen
Komponentennamen und einen Props-Ausdruck entgegen:

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

- **Das führende `&req` ist erforderlich.** Das Makro liest
  `X-Inertia`-Header, die URL und die Partial-Reload-Filterheader
  von der Anfrage, braucht also den Anfragewert (oder eine
  Referenz). Ohne es würden Partial Reloads stillschweigend
  kaputtgehen.
- **Die Existenz der Komponente wird zur Compile-Zeit geprüft.** Das
  Makro sucht nach
  `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`; passt keine
  Datei, schlägt der Build mit einem „meinten Sie …?“-Vorschlag
  fehl, der aus den tatsächlichen Dateinamen auf der Platte stammt.
  Verschachtelte Pfade funktionieren genauso -
  `inertia_response!(&req, "Admin/Dashboard", …)` löst zu
  `frontend/src/pages/Admin/Dashboard.svelte` auf (oder der
  Erweiterung Ihres Frontends).
- **Das Makro expandiert zu einem `await`eten `Result`.** Ihr
  Handler muss [`Response`](error-model.md) zurückliefern (was
  `Result<HttpResponse, HttpResponse>` ist) oder einen anderen Typ,
  der `FrameworkError` über `?` / `From` aufnimmt. Fehlschläge
  während der Prop-Serialisierung oder des Response-Baus werden als
  `Err` zurückgegeben, nicht als Panics.

### Props im JSON-Stil

Für Prototyping und winzige Seiten können Sie die typisierte
Struktur überspringen:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

Das Makro validiert weiterhin die Komponentendatei. Der Kompromiss
ist, dass Sie die typisierte Prop-Kette verlieren - kein
`#[derive(InertiaProps)]`, keine automatische
TypeScript-Generierung, keine Compile-Zeit-Prüfung, dass die vom
Frontend erwartete Form passt.

### Optionale Config-Überschreibung

Das Makro akzeptiert eine optionale abschließende `InertiaConfig`
für Pro-Response-Überschreibungen (andere SSR-Einstellungen, einen
eigenen Standard-Titel für eine Seite):

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

Die meisten Apps registrieren eine einzige Config beim Boot über
[`Inertia::install`](#bootstrap-inertia-install) und fassen dieses
Argument nie an.

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

Das Makro deckt Eager Typed Props ab. Alles andere - Lazy, Optional,
Deferred, mergebar, client-gecacht, Flash,
History-Verschlüsselungs-Überschreibungen - verwendet direkt den
Builder:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: Die Closure läuft nur, wenn die Prop tatsächlich
        // gesendet wird (initialer Besuch oder Partial Reload, der
        // diesen Schlüssel anfordert).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: nie bei initialen Besuchen gesendet; der Client
        // muss den Schlüssel explizit über X-Inertia-Partial-Data
        // anfordern.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: beim initialen Rendern ausgelassen; der Client
        // stellt eine Folge-XHR, und die Closure läuft dann.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: hängt an Bestehendes an, bei Partial Reloads
        // („mehr laden“).
        .merge("rows", next_page().await?)
        // Once: client-seitig über Navigationen hinweg gecacht;
        // Resolver bei nachfolgenden Besuchen ausgelassen, außer
        // der Server zwingt eine Aktualisierung.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: einmaliger Toast; erscheint unter `page.flash`,
        // nicht unter `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Methode | Zweck | Laravel-Entsprechung |
|---|---|---|
| `.with(k, v)` | Eager Prop, respektiert Partial-Reload-Filterung | Typed Prop |
| `.always(k, v)` | Eager Prop, ignoriert Partial-Reload-Filter | `Inertia::always(…)` |
| `.lazy(k, ‖)` | Resolver läuft nur, wenn die Prop gesendet wird | `fn () => …`-Closure |
| `.optional(k, ‖)` | Nie beim initialen Besuch; muss explizit angefordert werden | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Beim initialen Besuch ausgelassen; Folge-XHR löst die Auflösung aus | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Kombiniert mit bestehendem Client-Zustand bei Partial Reloads | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | Client cacht über Navigationen hinweg | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate` (via `Inertia::paginate`) | Infinite-Scroll-Paginierung | `Inertia::scroll(…)` |
| `.flash(k, v)` | Einmaliger Wert unter `page.flash` (nicht `props`) | `session()->flash(…)` |
| `.title(…)` | Standard-`<title>` für die HTML-Shell | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Pro-Response-History-Verschlüsselung | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Erzwingt Rotation des History-Keys | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Behält `#fragment` nach einem Inertia-Besuch | `Inertia::preserveFragment()` |

Eager-Builder-Methoden haben `try_*`-Geschwister (`try_with`,
`try_always`, `try_merge_with`, `try_scroll`, `try_flash`), die
`Result<Self, FrameworkError>` zurückgeben, wenn die
`Serialize`-Impl eines Werts zur Laufzeit fehlschlagen könnte - die
unfehlbaren Methoden wandeln den Panic über [die Panic-Grenze](error-model.md)
in ein 500 um, greifen Sie also zu `try_*`, wenn Sie den Fehlschlag
lieber explizit behandeln möchten.

### Merge-Strategien und Infinite Scroll

`.merge` (anhängen), `.merge_prepend` und `.deep_merge` decken die
gängigen „mehr laden“-Fälle ab. Für ein Diff-Merge - Zeilen
aktualisieren, die der Client schon hält, statt sie zu duplizieren -
greifen Sie zu `.merge_with` mit einer expliziten `MergeStrategy`,
die einen `match_on`-Schlüssel trägt:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // der Ausschnitt der neuen Seite
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` benennt das Feld, auf dem der Client dedupliziert
(ausgegeben an das Page-Objekt als `matchPropsOn`), sodass ein
erneuter Fetch, der das aktuelle Fenster überlappt, passende Zeilen
an Ort und Stelle ersetzt, statt Kopien anzuhängen. `Prepend` und
`Deep` nehmen dasselbe `match_on`.

Infinite Scroll ist derselbe Mechanismus mit angehängten
Paginierungs-Metadaten. `.scroll` / `.scroll_with` - oder
`.paginate`, das einen `LengthAwarePaginator` oder `CursorPaginator`
direkt adaptiert - gibt `scrollProps` neben den Daten aus, und die
`<InfiniteScroll>`-Komponente des Clients steuert die
Vorwärts-/Rückwärts-Fetches:

```rust
// `posts` ist ein CursorPaginator aus dem Query Builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

Das Framework liest die Merge-Richtung aus dem
`X-Inertia-Infinite-Scroll-Merge-Intent`-Anfrage-Header, den der
Client sendet (`append` beim Runterscrollen, `prepend` beim
Hochscrollen). Bei einem frischen Besuch - ohne Intent-Header - ist
`scrollProps["posts"].reset` `true`, sodass der Client seinen
Akkumulator leert, bevor er das erste Fenster rendert.

## Partial Reloads

Der Inertia-3-Client kann eine Teilmenge der Props einer Seite
anfordern (oder eine Übermenge, indem er einen Optional- oder
Defer-Schlüssel einschließt). Das Protokoll verwendet drei
Anfrage-Header:

| Header | Bedeutung |
|---|---|
| `X-Inertia-Partial-Component` | Die Komponente, die partial-reloaded wird - muss zur Komponente der Response passen, damit die Filterung greift. |
| `X-Inertia-Partial-Data` | Allowlist: kommagetrennte Prop-Schlüssel zum Einschließen. |
| `X-Inertia-Partial-Except` | Denylist: kommagetrennte Prop-Schlüssel zum Ausschließen. Gewinnt bei einer Schlüsselkollision gegen `Partial-Data`. |

Filterregeln:

- `Eager`-, `Lazy`-, `Merge`-, `Once`-, `Scroll`-Props folgen der
  Allowlist-/Denylist-Semantik.
- `Always`-Props werden unabhängig davon gesendet.
- `Optional`- und `Defer`-Props sind nie bei einem Standard-Besuch
  vorhanden und erscheinen nur bei einem passenden Partial Reload,
  der den Schlüssel explizit auflistet.

Der Handler muss nichts Besonderes tun - registrieren Sie jede Prop
über den Builder, und das Framework konsultiert die Header, wenn es
das Page-Objekt serialisiert.

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
erscheinen und danach verschwinden soll - Toast-Nachrichten, „gerade
erstellt“-IDs, Validierungs-Zusammenfassungen. Suprnova stellt sie
unter `page.flash` bei jeder Inertia-Response bereit. Es gibt drei
Schreiber:

```rust
// 1. In die Flash-Bag der aktuellen Anfrage pushen.
App::flash("toast", "Saved");

// 2. An eine bestimmte Response anhängen (derselbe Effekt nur für diese Response).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Über einen Redirect mittragen, über die Redirect-Facade.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

Die Form `Redirect::with(key, value)` ist der Cross-Handler-Pfad: Der
Wert landet in der Session unter `_flash.new.*`, die
[`SessionMiddleware`](csrf.md) der nächsten Anfrage lässt ihn zu
`_flash.old.*` altern, und die `InertiaResponse` des Ziels stellt ihn
unter `page.flash` bereit.

Same-Request-Flash (die Task-Local-Bag) gewinnt bei einer
Schlüsselkollision gegen geerbten Session-Flash, sodass ein
Ziel-Handler einen eingehenden Wert einfach überschreiben kann,
indem er den Schlüssel erneut flasht.

Interne Session-Schlüssel (alles mit dem Präfix `_`) werden aus
`page.flash` herausgefiltert - `_old_input` für die
Formular-Rückbefüllung und `_inertia.*`-Protokoll-Flags sickern
nicht zum Client durch.

### Redirect-Helfer

`Redirect` ist die vollständige Laravel-Oberfläche:

```rust
Redirect::to("/dashboard")                       // 302 auf einen Pfad
Redirect::route("posts.show").with("id", "42")   // benannte Route, Routenparameter
Redirect::back("/")                              // in der Session vermerkte vorherige URL
Redirect::refresh()                              // dieselbe URL, frisches GET
Redirect::guest(&req, "/login")                  // merkt die vorgesehene URL vor
Redirect::intended("/dashboard")                 // holt die vorgemerkte URL ab
Redirect::signed_route("downloads.show", &[("id","42")])?  // signierte URL
Redirect::to("/posts/42").preserve_fragment()    // #frag über den Besuch hinweg behalten
```

Alle `Redirect`-Varianten akzeptieren `.with(k, v)`,
`.with_input(map)`, `.with_errors(map)`, `.with_errors_bag(name,
map)`, `.cookie(c)`, `.header(k, v)`, `.permanent()`, `.status(303)`
usw. Die vollständige Kette spiegelt Laravels `RedirectResponse`.

Bei Non-GET-Inertia-Besuchen wandelt das Framework die Response
automatisch in `303 See Other` um, wenn
[`Inertia303Middleware`](#bootstrap-inertia-install) installiert
ist, sodass der Browser ein sauberes Folge-GET ausgibt, statt das
ursprüngliche PUT/PATCH/DELETE erneut an das Redirect-Ziel zu
übermitteln.

## Versionserkennung

Inertia versioniert das Asset-Manifest, damit ein langlebiger Client
nicht versucht, eine Seite aus dem Bundle von gestern gegen den
heutigen Server zu mounten. Wenn der `X-Inertia-Version`-Header des
Clients nicht zur konfigurierten Version des Servers passt,
antwortet [`InertiaVersionMiddleware`](#bootstrap-inertia-install)
mit `409 Conflict` und einem `X-Inertia-Location`-Header, der die
neue URL benennt - der Inertia-Client greift das auf und macht ein
vollständiges Neuladen der Seite, wodurch er das neue Bundle
übernimmt.

Sie setzen die Version über `InertiaConfig`:

```rust
use suprnova::InertiaConfig;

// Statisch - die meisten Apps. Einen Build-Zeit-Identifier einbacken.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamisch - einen Manifest-Hash lesen, eine Container-Deployment-ID,
// was auch immer. Die Closure läuft bei jeder Versionsprüfung; cachen
// Sie darin, falls das nicht billig ist.
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

Für eine asynchrone oder fehlschlagbare Versionsauflösung (z. B.
einen Manifest-Hash aus S3 lesen), führen Sie das Lesen einmal beim
Boot aus und übergeben Sie den gecachten `String` an `.version(...)`.

## Bootstrap: `Inertia::install`

Die meisten Apps installieren die beiden Protokoll-Middlewares in
einem Aufruf:

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

`Inertia::install` liefert `Result` zurück und, in dieser
Reihenfolge:

1. Schlägt fail-closed fehl, wenn `cfg` zum Production-Modus
   auflöst (`development == false` - der Standard, wann immer
   `APP_ENV=production` ist), aber kein Vite-Manifest von
   `cfg.manifest_path` geladen werden kann. Das ist die
   CFG-01-Schutzmaßnahme: Ein Production-Boot mit einem nicht
   gebauten Frontend schlägt sichtbar fehl, statt stillschweigend
   auf einen veralteten, hartcodierten Asset-Pfad zurückzufallen.
2. Registriert `InertiaVersionMiddleware` - gibt das `409` +
   `X-Inertia-Location` aus, wenn Client und Server sich über die
   Asset-Version uneinig sind.
3. Registriert `Inertia303Middleware` - hebt `302` auf `303` an bei
   Non-GET-Inertia-Redirects.

Überspringen Sie den Aufruf nur, wenn Sie wirklich eine dieser
Middlewares nicht wollen (selten; beide schließen echte Fehlermodi
ab - stilles veraltetes Bundle und Formular-Replay-bei-Redirect).

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

Suprnova spricht über HTTP-Loopback mit einem
Out-of-Process-SSR-Worker - typischerweise das
`@inertiajs/{svelte,react,vue}/server`-`createServer()`-Bundle,
ausgeführt unter Node / Bun / Deno. Aktivieren Sie es an der Config:

```rust
InertiaConfig::new()
    .ssr("http://127.0.0.1:13714")  // Worker-URL
    .ssr_timeout(std::time::Duration::from_millis(500))
    .ssr_exclude("/admin/**")
    .ssr_max_response_bytes(8 * 1024 * 1024)
```

SSR ist standardmäßig aus. Wenn aktiviert, postet das Framework das
Page-Objekt an `<url>/render` und inlined `{ head, body }` in die
HTML-Shell. Bei einem Worker-Fehler oder -Timeout fällt die Response
auf CSR zurück (ein leeres `<div id="app">`, das der Client
hydratisiert), und der `on_ssr_error(...)`-Hook feuert; schalten Sie
`ssr_throw_on_error(true)` in CI um, um solche Fehlschläge
stattdessen zu harten 500ern zu machen.

Booten Sie den Worker separat - `suprnova ssr:start` ist der
Standard-Runner, sobald Ihr Projekt einen SSR-Einstiegspunkt
mitbringt.

## Konfiguration

Das Inertia-Verhalten wird programmatisch über `InertiaConfig`
konfiguriert. Die einzige Env-Var, die das Framework direkt liest,
ist `SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`), die den
Standard-Einstiegspunkt-Dateinamen und die
Seiten-Komponenten-Erweiterungen wählt. Alles andere ist
Builder-förmig:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)              // überschreibt SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)            // begrenzt den Lazy-Prop-Fan-out
    .production();                           // false → lädt vom Vite-Dev-Server
```

Frontend-spezifische Standardwerte:

| Frontend | Standard-Einstiegspunkt | Seiten-Erweiterungen |
|---|---|---|
| Svelte (Standard) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

Das Vite-Manifest unter `manifest_path` wird lazy bei der ersten
Anfrage geladen und für die Lebensdauer des Prozesses gecacht. Fehlt
es, fallen Production-Asset-Tags auf einen hartcodierten Legacy-Pfad
zurück, und ein `tracing::warn!` feuert, sodass die Lücke in den
Logs sichtbar wird.

### Warum Suprnova abweicht

Laravels Inertia-Adapter hat eine einzige globale
„Shared-Data“-Registry plus einen Pro-Request-Aufruf
`Inertia::share($k, $v)`. PHPs Request-pro-Prozess-Modell macht das
sicher: Ein frischer Prozess pro Anfrage bedeutet kein Durchsickern
zwischen gleichzeitigen Besuchern.

Rusts Prozessmodell ist das Gegenteil - ein Prozess bedient viele
gleichzeitige Anfragen über viele Threads hinweg. Die Registry lebt
daher im [Container](container.md) (Task-Local → Thread-Local →
Global), nicht in prozessglobalen Statics. `App::inertia_share*`
schreibt in die `InertiaRegistry` des aktiven Containers, was Tests,
die `TestContainer::fake()` verwenden, eine saubere Isolation gibt,
ohne dass irgendetwas deregistriert werden muss. Dieselbe Oberfläche
wie Laravel; andere Maschinerie darunter, weil die Runtime eine
andere ist.

Zwei weitere, Rust-geprägte Entscheidungen, die es wert sind,
hervorgehoben zu werden:

- **Lazy-Prop-Resolver laufen gleichzeitig**, begrenzt durch
  `max_concurrent_resolvers` (Standard 16). Eine Seite mit zwölf
  Lazy Props gibt zwölf parallele Queries innerhalb einer einzigen
  Tokio-Task aus - genau dafür haben wir das Framework auf Tokio
  aufgebaut. Passen Sie die Grenze an, wenn eine Seite viele Lazy
  Props hat, die jeweils einen externen Dienst treffen.
- **Die Compile-Zeit-Komponentenprüfung** ist überhaupt kein
  Laravel-Feature, weil PHP Ihre Frontend-Dateien zur Compile-Zeit
  nicht sehen kann. Suprnova kann das, sodass ein Tippfehler in
  `inertia_response!("Dashbaord", …)` den Build mit einem Vorschlag
  „did you mean Dashboard?“ fehlschlagen lässt, statt später als
  Laufzeit-„component not found“ aufzutauchen.

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

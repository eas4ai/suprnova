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

Für eine Seite ohne Logik - Info, Nutzungsbedingungen, Datenschutz - können Sie den Handler vollständig überspringen und die Route deklarieren:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new().inertia("/about", "About", json!({ "team_size": 4 }));
```

Siehe [Routing](routing.md#router-level-redirects-and-views). Die Komponente ist dort ein Laufzeit-String und erhält daher nicht die Prüfung der Makro-Existenz zur Compile-Zeit - das ist der Preis dafür, keinen Handler zu schreiben.

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

Die meisten Anwendungen registrieren beim Booten über [`Inertia::install`](#bootstrap-inertia-install) eine einzige Konfiguration und verwenden dieses Argument nie: Die installierte Konfiguration ist bereits der Ausgangspunkt jeder Response. Übergeben Sie hier nur eine, um die installierte Konfiguration für eine einzelne Seite zu übersteuern.

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
| `.with(k, v)` | Eager-Prop, berücksichtigt Partial-Reload-Filterung | typisierte Prop |
| `.always(k, v)` | Eager-Prop, ignoriert Partial-Reload-Filter | `Inertia::always(…)` |
| `.always_with(k, ‖)` | Asynchroner Resolver, ignoriert Partial-Reload-Filter | `Inertia::always(fn () => …)` |
| `.lazy(k, ‖)` | Resolver läuft nur, wenn die Prop gesendet wird | Closure `fn () => …` |
| `.optional(k, ‖)` | Nie beim Erstbesuch; muss ausdrücklich angefordert werden | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Beim Erstbesuch übersprungen; Folge-XHR löst die Auflösung aus | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Bei Partial Reloads mit bestehendem Client-Zustand zusammenführen | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | Der Client cached über Navigationen hinweg | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.scroll_with_wrapped` / `.paginate` (über `Inertia::paginate`) | Paginierung für Infinite Scroll | `Inertia::scroll(…)` |
| `.flash(k, v)` | Einmaliger Wert unter `page.flash` (nicht `props`) | `session()->flash(…)` |
| `.title(…)` | Standard-`<title>` für die HTML-Shell | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Verschlüsselung der History pro Response | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Erzwingt die Rotation des History-Schlüssels auf **dieser** Seite | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | `#fragment` nach einem Inertia-Besuch beibehalten | `Inertia::preserveFragment()` |

Eager-Builder-Methoden haben `try_*`-Geschwister (`try_with`, `try_always`, `try_merge_with`, `try_scroll`, `try_scroll_wrapped`, `try_flash`), die `Result<Self, FrameworkError>` zurückgeben, wenn die `Serialize`-Implementierung eines Werts zur Laufzeit fehlschlagen kann. Die unfehlbaren Methoden wandeln den Panic über [die Panic-Grenze](error-model.md) in einen 500 um; verwenden Sie `try_*`, wenn Sie den Fehler lieber ausdrücklich behandeln.

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

### Flags auf einer Prop kombinieren

Die obigen Methoden setzen jeweils ein Flag. Eine Prop kann mehrere tragen, und einige Kombinationen entsprechen der Art, wie das Inertia-Protokoll echte Seiten funktionieren lässt: eine Deferred-Liste, die an das anhängt, was der Client bereits gerendert hat, eine Merge-Prop, die der Client über Navigationen hinweg cached, eine optionale Prop mit eigenem Cache-Schlüssel. Bauen Sie die Prop mit `Prop` und hängen Sie sie dann mit `.prop(key, prop)` an:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::lazy(|| async { json!([{ "id": 1 }]) })
        .defer()
        .merge()
        .match_on("id"),
)
```

Diese Prop wird beim ersten Rendern übersprungen und unter `deferredProps` angekündigt. Der Client sendet seine Folgeanfrage, der Resolver läuft, und der Wert trifft mit einer Anweisung `mergeProps` ein; so wird er an die bereits auf dem Bildschirm befindliche Liste angehängt, statt sie zu ersetzen.

Die Flags gehören zu fünf Gruppen:

| Gruppe | Methoden | Wirkung |
|---|---|---|
| Sichtbarkeit | `.always()`, `.optional()`, `.defer()` | Gegenseitig ausschließend; der letzte Aufruf gewinnt |
| Deferred-Details | `.group(name)`, `.rescue()` | Werden nur gelesen, wenn die Prop deferred ist |
| Merge | `.merge()`, `.prepend()`, `.deep_merge()`, `.match_on(fields)`, `.merge_with_path(path)` | Wie der Client den Wert einfügt und an welchem Pfad |
| Client-Cache | `.once()`, `.as_key(key)`, `.until(ms)`, `.fresh()` | Ob der Client den Wert über Navigationen hinweg behält |
| Scroll | `.scroll(metadata)`, `.scroll_wrap(key)` | Eintrag `scrollProps` für Infinite Scroll plus bedingungslose Merge-Metadaten; `.scroll_wrap` wird nur gelesen, wenn `.scroll` gesetzt ist |

Quellen sind `Prop::eager(value)`, `Prop::lazy(closure)`, `Prop::from_resolver(resolver)` für einen selbst gebauten Resolver sowie `Prop::absent()` für eine Prop, die die Response nie erreicht - genau das gibt `when_loaded!` für eine nicht geladene Relation zurück.

Vor dem Kombinieren sind zwei Regeln wichtig:

- **Sichtbarkeit ist eine Einstellung, nicht drei Flags.** `.always().optional()` ist eine optionale Prop und `.optional().always()` eine Always-Prop. Beides ist kein Fehler; der frühere Aufruf wird gelöscht.
- **Metadaten folgen den Partial-Reload-Listen, nicht dem Wert.** Die Einträge einer Prop in `mergeProps`, `onceProps` und `scrollProps` werden ausgegeben, sobald der Schlüssel `X-Inertia-Partial-Data` und `X-Inertia-Partial-Except` passiert - auch bei einem Besuch, bei dem der Wert selbst zurückgehalten wird. Das trägt die Merge-Anweisung über die zwei Requests einer Deferred-Prop. Daraus folgen zwei Konsequenzen:
  - Eine `.always().merge()`-Prop außerhalb der angeforderten Menge sendet ihren Wert dennoch, aber nicht ihre Merge-Anweisung; der Client ersetzt daher, statt anzuhängen.
  - Für `scrollProps` gilt zusätzlich zu den Listen eine weitere Bedingung: Eine `.scroll().defer()`-Prop kündigt ihre Merge-Anweisung bei einem nicht-partiellen Besuch an, liefert dort aber keinen Cursor, weil noch nichts auf dem Bildschirm ist, das ein Cursor beschreiben könnte. Jeder passende Partial Reload erhält den Cursor, unabhängig davon, ob der Request auch den Wert auflöst.
  - `deferredProps` ist der einzige Block, den die Listen nie steuern. Bei jedem passenden Partial Reload wird er vollständig weggelassen, unabhängig von den Listen - Laravels `resolveDeferredProps` gibt `[]` zurück, sobald der Request partiell ist. Bei einem Partial Reload arbeitet der Client die Ankündigungen ab, die er bereits besitzt; die in dieser Runde ausgelassenen Schlüssel erneut anzukündigen, würde ihn erneut danach fragen lassen. Ein Partial Reload, der auf eine *andere* Komponente zielt, ist für jedes Gate ein Standardbesuch, Ankündigungen eingeschlossen.

`.group(name)` und `.rescue()` werden auf jeder Prop gespeichert, aber nur gelesen, wenn die Prop deferred ist; `.rescue().defer()` und `.defer().rescue()` bedeuten daher dasselbe. Eine Scroll-Prop bezieht ihre Merge-Richtung aus dem Header `X-Inertia-Infinite-Scroll-Merge-Intent` des Clients; `.merge()` und `.prepend()` auf einer Scroll-Prop sind folglich redundant und werden nicht gelesen. `.deep_merge()` ist die Ausnahme: Es leitet die Prop in `deepMergeProps` statt in `mergeProps`, genau wie Laravels `ScrollProp`.

### Merge-Strategien und Infinite Scroll

`.merge` (anhängen), `.merge_prepend` und `.deep_merge` decken die üblichen Fälle „mehr laden“ ab. Für Diff-Merge - Zeilen aktualisieren, die der Client bereits hält, statt Duplikate anzuhängen - verwenden Sie `.merge_with` mit einer expliziten `MergeStrategy`, die einen Schlüssel `match_on` trägt:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // the new page slice
        MergeStrategy::Append { match_on: Some(vec!["id".into()]) },
    )
```

`match_on` benennt das Feld bzw. die Felder, anhand dessen der Client dedupliziert (im Page-Objekt als `matchPropsOn` ausgegeben) - ein Feld oder mehrere, wie bei `Prop::match_on` (unten). Bei einem Refetch, der das aktuelle Fenster überlappt, werden passende Zeilen so an Ort und Stelle ersetzt, statt Kopien anzuhängen. `Prepend` und `Deep` akzeptieren dasselbe `match_on`.

`MergeStrategy` ist die Ein-Aufruf-Form. `Prop::merge()` / `.prepend()` / `.deep_merge()` / `.match_on(field)` sind dieselben Einstellungen als getrennte Flags, wenn eine Prop zusätzlich ein Sichtbarkeits- oder Cache-Flag benötigt - siehe [Flags auf einer Prop kombinieren](#flags-auf-einer-prop-kombinieren).

`.match_on` akzeptiert ein Feld oder mehrere in einem Aufruf - `.match_on(["id", "slug"])` und `.match_on("id").match_on("slug")` geben dasselbe `matchPropsOn` aus.

Um statt des gesamten Werts nur einen Teil einer Prop zusammenzuführen, benennen Sie das verschachtelte Feld mit `.merge_with_path`:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::eager(json!({ "data": next_page, "meta": meta }))
        .merge()
        .merge_with_path("data")
        .match_on("data.id"),
)
```

`mergeProps` enthält nun `"posts.data"` statt `"posts"`; nur `props.posts.data` wird also mit dem zusammengeführt, was der Client bereits hält. `props.posts.meta` wird wie jede Nicht-Merge-Prop vollständig ersetzt. Aufrufe akkumulieren, sodass eine Prop mit zwei zusammenführbaren Feldern jedes unabhängig benennen kann. Einen Pfad zu benennen deaktiviert das Merge auf Root-Ebene für diese Prop vollständig - eine Prop mit Path-Merge führt nie zugleich ihren gesamten Wert zusammen. `match_on` kombiniert sich mit einem Pfad, indem der Pfad im Feldnamen enthalten ist (`"data.id"`, nicht `"id"`); das Framework leitet ihn nicht für Sie ab. `.deep_merge()` ignoriert `.merge_with_path`: Ein Deep Merge steigt bereits in jedes verschachtelte Feld ein, sodass ein Pfad nichts weiter eingrenzt.

Der Wert einer Merge-Prop kann über `.merge_lazy` / `.merge_lazy_with`, dem Resolver-Gegenstück zu `.merge` / `.merge_with`, auch von einem Resolver stammen:

```rust
InertiaResponse::new("Feed/Index").merge_lazy("posts", || async {
    Ok::<_, FrameworkError>(load_next_page().await?)
})
```

Der Resolver läuft nur, wenn die Merge-Prop tatsächlich gesendet wird; wie jede resolvergestützte Prop wird er durch Partial-Reload-Filterung und `.defer()` übersprungen.

Infinite Scroll ist dieselbe Maschinerie mit angehängten Paginierungsmetadaten. `.scroll` / `.scroll_with` - oder `.paginate`, das einen `LengthAwarePaginator` oder `CursorPaginator` direkt adaptiert - geben `scrollProps` neben den Daten aus; die Komponente `<InfiniteScroll>` des Clients steuert die Abrufe für die nächste beziehungsweise vorherige Seite:

```rust
// `posts` is a CursorPaginator from the query builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

Eine Scroll-Prop trägt immer Merge-Metadaten, nicht nur bei einem Folgeabruf: Sie verwendet standardmäßig append und wechselt nur zu prepend, wenn der Header `X-Inertia-Infinite-Scroll-Merge-Intent` des Clients dies angibt (`append` beim Herunterscrollen, `prepend` beim Hochscrollen). `reset` ist von diesem Header unabhängig: Es ist genau dann `true`, wenn der Client den Schlüssel in `X-Inertia-Reset` nennt, demselben Header, den eine reguläre Merge-Prop liest. Ein frischer, ungefilterter Besuch sendet keinen der beiden Header; er erhält daher `reset: false` und eine Anweisung append, wie Laravel.

`.merge_with_path` hat auf eine Scroll-Prop keine Wirkung: Der Scroll-Block, der ihre Merge-Anweisung berechnet, liest den einzelnen Wrap-Schlüssel von `Prop::scroll_wrap`, nicht die angesammelte Pfadliste von `.merge_with_path`; `.scroll(metadata).merge_with_path("data")` speichert damit einen Pfad, den nichts liest. `.scroll_wrap` - direkt über `.prop(...)` oder über die Response-Abkürzung `.scroll_wrapped` unten erreichbar - ist das Verschachtelungsäquivalent für eine Scroll-Prop.

Eine Scroll-Prop berücksichtigt außerdem `.match_on(...)`, wie jede andere Merge-Prop. Verwenden Sie es über `.prop(...)`, weil weder `.scroll` noch `.match_on` eine kombinierte Response-Abkürzung besitzen:

```rust
InertiaResponse::new("Users/Index").prop(
    "users",
    Prop::eager(rows)
        .scroll(ScrollMetadata::new("page").current(1).next(2))
        .match_on("id"),
)
```

Das Match-Feld richtet sich danach, wo die Prop tatsächlich zusammengeführt wird: nach dem nackten Schlüssel ohne Wrapper (`matchPropsOn: ["users.id"]`) oder nach `key.wrap_key` unter `.scroll_wrap(...)` (`matchPropsOn: ["posts.data.id"]` für eine unter `"data"` gewrappte Prop). Der Eintrag stimmt dadurch immer mit dem Merge-Pfad überein, den der Client zusammenführt, statt stillschweigend nie zu matchen.

Ist der Wert der Prop selbst eine gewrappte Struktur - `{ data: [...], meta: {...} }`, die Form, die eine handgebaute API-Resource typischerweise zurückgibt -, würde das Zusammenführen des gesamten Objekts bei jedem Abruf `meta` überschreiben. Richten Sie das Merge stattdessen mit `.scroll_wrapped` auf das Array-Feld:

```rust
InertiaResponse::new("Feed/Index").scroll_wrapped(
    "posts",
    "data",
    ScrollMetadata::new("page").current(2).next(3),
    serde_json::json!({ "data": rows, "meta": { "total": total } }),
)
```

`mergeProps` benennt dann `posts.data`, sodass der Client neue Zeilen in das verschachtelte Array einfügt und `meta` jedes Mal vollständig ersetzt. `.scroll_with_wrapped` und `try_scroll_wrapped` sind die resolvergestützten beziehungsweise fehlbaren Gegenstücke zu `.scroll_with` / `try_scroll`.

Ein Typ außerhalb des Moduls `pagination` dieser Crate - ein Drittanbieter-Paginator, ein selbst gebauter Cursor - kann sich gegenüber `.scroll` durch Implementieren von `ProvidesScrollMetadata` beschreiben, statt `ScrollMetadata` Feld für Feld aufzubauen:

```rust
use suprnova::{ProvidesScrollMetadata, ScrollMetadata};

impl ProvidesScrollMetadata for MyCursorPage {
    fn page_name(&self) -> String { "cursor".to_string() }
    fn previous_page(&self) -> Option<serde_json::Value> { self.prev.clone().map(Into::into) }
    fn next_page(&self) -> Option<serde_json::Value> { self.next.clone().map(Into::into) }
    fn current_page(&self) -> Option<serde_json::Value> { Some(self.current.clone().into()) }
}

InertiaResponse::new("Feed/Index").scroll("posts", page.scroll_metadata(), page.rows)
```

`LengthAwarePaginator`, `Paginator` und `CursorPaginator` implementieren es ebenfalls - siehe [Pagination](pagination.md#inertia-integration-infinite-scroll-props).

### Verschachtelung per Punktnotation

Ein Schlüssel mit `.` wird in der Response verschachtelt, statt als literaler String-Schlüssel übertragen zu werden - Laravels Punktnotation auf Basis von `Arr::set` (`Inertia::share('user.name', …)`, `resolveArrayableProperties`):

```rust
InertiaResponse::new("Dashboard")
    .with("user.name", "Todd")
    .with("user.locale", "es")
```

wird wie folgt übertragen:

```json
{ "user": { "name": "Todd", "locale": "es" } }
```

und nicht als zwei literale Schlüssel `"user.name"` / `"user.locale"`. Zwei Aufrufe mit gemeinsamem Präfix sammeln sich in einem Objekt; ein Schlüssel ohne Punkt bleibt unberührt. Dies gilt für jede Methode, die Props anfügt - `.with`, `.always`, `.lazy`, Schlüssel der Shared Registry - und für nichts anderes: Es steigt niemals in den *Wert* einer Prop hinab; ein Validierungsobjekt `errors` behält daher alle darin enthaltenen Feldnamen mit Punkten. Es gibt keinen Escape Hatch für einen Schlüssel, der einen literalen Punkt behalten muss (`.with("config.json", …)` verschachtelt weiterhin). Das entspricht Laravel, wo `Arr::set` ebenfalls keinen Escape-Mechanismus hat.

## Partial Reloads

Der Inertia-3-Client kann eine Teilmenge der Props einer Seite anfordern (oder durch Einschließen eines Optional- oder Defer-Schlüssels eine Obermenge). Das Protokoll verwendet drei Request-Header:

| Header | Bedeutung |
|---|---|
| `X-Inertia-Partial-Component` | Die teilweise neu zu ladende Komponente - sie muss der Komponente der Response entsprechen, damit Filterung erfolgt. |
| `X-Inertia-Partial-Data` | Whitelist: einzuschließende, kommagetrennte Prop-Schlüssel. |
| `X-Inertia-Partial-Except` | Blacklist: auszuschließende, kommagetrennte Prop-Schlüssel. Bei einer Schlüssel-Kollision hat sie Vorrang vor `Partial-Data`. |

Die Filterung liest genau eines: die Sichtbarkeit der Prop, gesetzt durch `.always()`, `.optional()` oder `.defer()`. Eine Prop ohne eines dieser Flags hat Standard-Sichtbarkeit.

- Props mit Standard-Sichtbarkeit folgen der Whitelist-/Blacklist-Semantik.
- Props mit `.always()` werden unabhängig davon gesendet.
- Props mit `.optional()` und `.defer()` werden bei einem Standardbesuch nie übertragen und erscheinen nur bei einem passenden Partial Reload, der den Schlüssel ausdrücklich nennt.

Die Flags für Merge und Scroll spielen dabei keine Rolle: Sie entscheiden, wie der Client einen erhaltenen Wert zusammenführt, nicht ob er ihn erhält. Eine Prop `.defer().merge()` wird daher genau wie eine einfache `.defer()`-Prop gefiltert. Auch `.once()` spielt dafür keine Rolle, obwohl es nicht nur eine Anweisung zum Zusammenführen ist: Bei einem vollständigen Besuch, bei dem der Client den Wert bereits als gecached meldet, überspringt der Server den Resolver und sendet keinen Wert, wie die Anmerkung unten erläutert. Alle drei ändern jedoch, welche Metadatenblöcke mitkommen - siehe [Flags auf einer Prop kombinieren](#flags-auf-einer-prop-kombinieren).

Der Handler muss nichts Besonderes tun: Registrieren Sie jede Prop über den Builder; beim Serialisieren des Page-Objekts berücksichtigt das Framework die Header.

Der clientseitige Cache einer `once`-Prop wird nur bei einem **vollständigen** Inertia-Besuch berücksichtigt. Bei einem Partial Reload, der den Schlüssel nennt (`router.reload({ only: ['stats'] })`), läuft der Resolver und der Wert wird gesendet: Der Client fragt gerade deshalb, weil er einen frischen Wert möchte; seine Behauptung eines veralteten Caches dort zu berücksichtigen, würde für den angeforderten Schlüssel gar nichts zurückgeben.

### Verschachteltes only/except (Punktnotation)

Einträge in `X-Inertia-Partial-Data` und `X-Inertia-Partial-Except` können einen Pfad innerhalb des Werts einer Prop benennen, nicht nur den Schlüssel der Prop selbst. Ein Client, der `router.reload({ only: ['user.name'] })` aufruft, sendet `X-Inertia-Partial-Data: user.name`; die Response beschränkt die Prop `user` dann auf genau dieses Feld:

```json
{ "props": { "user": { "name": "Ada" } } }
```

`except` kürzt auf dieselbe Weise, statt einzugrenzen - `router.reload({ except: ['user.email'] })` lässt jedes andere Feld von `user` erhalten.

Regeln:

- Ein nackter Eintrag (`user`) bedeutet weiterhin die gesamte Prop. Nennt `only` sowohl `user` als auch `user.name`, wird der ganze Wert übertragen - der nackte Eintrag gewinnt.
- Ein Eintrag kann auch einen *Vorfahren* eines punktierten Prop-Schlüssels benennen. Eine unter `auth.user` registrierte Prop - per `.with("auth.user", …)` oder `App::inertia_share("auth.user", …)` - nimmt an `only: ['auth']` teil und wird vollständig übertragen, weil der Aufrufer nach dem gesamten Root `auth` fragte. Ein nacktes `except: ['auth']` lässt sie aus demselben Grund weg. Das Präfix muss an einer Segmentgrenze enden, sodass eine nicht verwandte Prop `authAgent.user` von beiden unberührt bleibt.
- `except` gewinnt auf einem Pfad, den beide Header nennen, genau wie auf oberster Ebene.
- Ein Pfad, der sich gegen den Wert nicht auflösen lässt - ein unbekanntes Feld oder einer, der durch einen Skalar oder ein Array statt durch ein Objekt führt - trägt für diesen Pfad nichts bei, ohne die zugleich angeforderten Geschwisterfelder wegzulassen.
- Always-Props ignorieren `only`/`except` vollständig, Punktnotation eingeschlossen - sie werden immer vollständig übertragen.
- Optional- und Defer-Props benötigen weiterhin die ausdrückliche Anforderung, um überhaupt aufgelöst zu werden. Ein punktierter Eintrag (`permissions.read`) zählt als diese Anforderung für den Top-Level-Schlüssel; der aufgelöste Wert wird genauso eingegrenzt wie bei einer Eager-Prop.
- Ein punktiertes `only` gegen eine Prop, deren aktueller Wert kein Objekt ist - ein String, eine Zahl, ein Array -, wird zu `{}` eingegrenzt, nicht zum ursprünglichen Wert. Die Reconciliation des Clients führt nur dann einen Deep Merge aus, wenn *sowohl* der gecachte als auch der eingehende Wert Objekte sind (`inertia-3.6.1/packages/core/src/response.ts` `nestedTopKeys`). Ein leeres Objekt scheitert gegen einen Nicht-Objekt-Cache an derselben Prüfung wie ein gefülltes; es ersetzt daher den gecachten Skalar direkt, statt darauf zusammengeführt zu werden. Vermeiden Sie eine punktierte Anforderung gegen eine Prop, die nicht als Objekt geformt ist.
- Ein punktiertes `except` löscht das Feld nicht beim Client: Es verhindert die Aktualisierung dieses Felds in dieser Response, und das Merge des Clients stellt es aus seinem bereits gecachten Wert wieder her. `deepMergeObjects` baut das zusammengeführte Objekt, indem es zuerst den gecachten Wert klont und dann nur die Schlüssel überschreibt, die der Server tatsächlich gesendet hat; einen vom Server ausgeschnittenen Schlüssel berührt es nie, sodass er mit seinem alten Wert erhalten bleibt. Beim allerersten Laden dieser Prop durch einen Client (noch nichts gecacht) fehlt das ausgeschnittene Feld wirklich, weil kein Cache als Fallback vorhanden ist - das Verhalten „aus dem Cache wiederherstellen“ gilt nur für eine Seite, die der Client bereits gesehen hat.

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

Shared Keys werden an Punkten genauso verschachtelt wie bei `.with`: Zwei statische Shares unter `"user.name"` / `"user.age"` ergeben auf dem Wire ein einzelnes Objekt `user`. Lesen Sie einen Shared-Wert zurück oder leeren Sie die statische Registry vollständig mit `App::inertia_shared` / `App::flush_inertia_shared` - Laravels `Inertia::getShared` / `Inertia::flushShared`:

```rust
use suprnova::App;

App::inertia_share("user.name", "Todd");
assert_eq!(App::inertia_shared("user.name"), Some(serde_json::json!("Todd")));

App::flush_inertia_shared();
assert_eq!(App::inertia_shared("user.name"), None);
```

`inertia_shared` liest nur die statische Registry. Für einen über `inertia_share_lazy` / `inertia_share_once` registrierten Key gibt es `None` zurück (es gibt keinen Request, gegen den dieser aufgelöst werden könnte; dies entspricht Laravels `getShared`, das die rohe Closure zurückgibt, statt sie aufzurufen), ebenso für einen Share eines Trait-Providers pro Request. `flush_inertia_shared` leert ebenfalls nur die statische Registry; ein über `register_inertia_shared` registrierter Provider besitzt keinen Zustand pro Request, der zu leeren wäre.

Für Shared Data pro Request (den authentifizierten Benutzer, requestbezogene Flags) implementieren Sie [`InertiaSharedData`](#pro-request-gemeinsame-daten) und registrieren das Singleton. Das Framework ruft bei jeder Inertia-Response `share(&req, component)` auf und führt das Ergebnis zusammen. `component` ist die gerenderte Seite; ein Provider kann seine Ausgabe daher nach Seite variieren - siehe unten.

### Vorrang bei einer Schlüsselkollision

Erscheint derselbe Schlüssel in mehr als einer Schicht, gewinnt der
spätere Schreibvorgang:

1. Statische Registry (`App::inertia_share` / `App::inertia_share_lazy`)
2. Pro-Request-Trait-Provider (`InertiaSharedData::share`)
3. Pro-Response-Builder-Methoden (`.with`, `.lazy`, usw.)

Das erlaubt einem Handler, einen global geteilten Standard für eine
Seite zu überschreiben, ohne irgendetwas deregistrieren zu müssen.

### Pro-Request gemeinsame Daten

Der Trait läuft einmal pro Inertia-Response und erhält Zugriff auf den Request **und** den Namen der Seitenkomponente - Laravels `RenderContext` (`component`, `request`), hier als einfache Parameter statt als Wrapper-Struktur, weil der Request die andere Hälfte bereits abdeckt. Implementierungen benötigen `async_trait` (reexportiert als `suprnova::__async_trait`) und `IndexMap` (reexportiert als `suprnova::indexmap`):

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
        component: &str,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        // Vary by page: only the admin dashboard needs the nav counts.
        if component == "Admin/Dashboard" {
            out.insert("pendingReviews".into(), Prop::eager(serde_json::json!(12)));
        }
        Ok(out)
    }
}

// In bootstrap:
App::register_inertia_shared(Arc::new(AuthShare));
```

Ignorieren Sie `component` (`_component`), wenn Ihr Provider nicht nach Seite variieren muss.

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

Für nicht-GET Inertia-Besuche konvertiert das Framework die Reaktion auf `303 See Other` automatisch, wenn [`Inertia303Middleware`](#bootstrap-inertia-install) installiert ist, so dass der Browser ein sauberes Follow-up GET ausstellt, anstatt das ursprüngliche PUT/PATCH/DELETE dem Ziel weiterzuleiten.

### Validierungsfehler

Scheitert ein Handler bei einem Inertia-Besuch an der Validierung, antwortet das Framework mit `303 See Other` zurück zur Formularseite und flasht die Fehler, statt mit dem JSON `422`, das ein REST-Client erhält. Das ist nicht kosmetisch: Der Inertia-Client behandelt jede Response ohne Header `X-Inertia` als Nicht-Inertia und rendert sie in einem Vollbild-Fehlermodal; ein `422` erreicht daher nie `form.errors`. Im Handler ändert sich nichts - die Brücke ist eine der Middlewares, die `Inertia::install` registriert.

Ziel ist zuerst der `Referer` des Requests, wenn er dieselbe Origin hat, dann die in der Session gespeicherte vorherige URL und zuletzt die URL des fehlgeschlagenen Requests selbst. Ein Origin-übergreifender `Referer` wird ignoriert, statt ihm zu folgen, ebenso einer, der nur nach derselben Origin aussieht: Ein führendes `//` oder `/\` (ein Browser liest beides als protokollrelativ, nachdem er einen Backslash zu einem Slash gefaltet hat) sowie jedes ASCII-Steuerbyte irgendwo im Wert (der URL-Parser entfernt Tabulator und Zeilenumbruch aus dem gesamten String, bevor er Origins vergleicht; ein Steuerbyte kann also aus einem scheinbar sicheren Pfad eine andere Origin machen, wenn der Browser ihn navigiert) führen beide auf dieselbe Fallback-Kette zurück. Dieselbe Prüfung gilt auch für den letzten URL-Fallback, sodass selbst ein ungewöhnlicher Request-Pfad keine Redirect auf eine fremde Origin werden kann.

Der Wert eines Felds ist seine **erste** Meldung, ein einfacher String - die Form, die Inertias eigener Typ `ErrorValue` beschreibt und an die `$page.props.errors.email` gebunden wird. Setzen Sie `InertiaConfig::with_all_errors(true)`, um stattdessen alle Meldungen als Array zu erhalten; dann benötigt auch der Client-Typ die passende Erweiterung:

```ts
// global.d.ts
import '@inertiajs/core'

declare module '@inertiajs/core' {
  export interface InertiaConfig {
    errorValueType: string[]
  }
}
```

Mehrere Formulare auf einer Seite bleiben isoliert: Senden Sie mit dem Besuch `X-Inertia-Error-Bag: <name>`; die Fehler werden unter dieser Bag geflasht und daraus zurückgelesen und treffen als `errors.<name>.<field>` ein.

Die Prop `errors` ist standardmäßig immer sichtbar; ein Partial Reload filtert oder begrenzt sie daher nie. `only: ['users']` liefert die Bag weiterhin, ebenso `except: ['errors']`; `only: ['errors.email']` liefert die ganze Bag statt nur dieses Feldes. Das entspricht Laravels Form: Seine Middleware teilt die Bag als `Inertia::always(...)`, und `resolveAlways` fügt den rohen Wert nach dem Neuaufbau von `only`/`except` wieder ein. Das ist wichtig, weil der Client eine partielle Response mit `{...current.props, ...response.props}` zusammenführt: Ein leeres Objekt `errors` würde die bereits auf dem Bildschirm befindlichen Meldungen löschen, während ein ungefiltertes sie korrekt lässt. Die Regel umfasst beide Quellen - die aus der Session geflashte Bag und ein eigenes `.with("errors", …)` eines Handlers. Ein explizites Sichtbarkeits-Flag hat weiterhin Vorrang; `.prop("errors", Prop::eager(…).optional())` verhält sich also optional.

Zwei Dinge tut dies nicht: Es flasht keine alten Eingaben erneut, denn der Request-Body ist bereits verbraucht, wenn die Brücke läuft, und ein Inertia-`useForm` behält seinen eigenen Zustand über ein fehlgeschlagenes Submit hinweg; es gibt daher nichts wieder aufzufüllen. Und es berührt nie eine Precognition-Response: Ein Dry-Run-`422` ist genau das, wonach der Client gefragt hat.

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

Inertia versioniert das Asset-Manifest, damit ein langlebiger Client nicht versucht, eine Seite aus dem gestrigen Bundle gegen den heutigen Server zu mounten. Stimmt der Header `X-Inertia-Version` des Clients nicht mit der konfigurierten Version des Servers überein, antwortet [`InertiaVersionMiddleware`](#bootstrap-inertia-install) mit `409 Conflict` und einem Header `X-Inertia-Location`, der die neue URL benennt. Der Inertia-Client nimmt diesen auf und führt einen vollständigen Reload der Seite aus, um das neue Bundle zu laden.

Der Bounce flasht zuerst die Session erneut. Der Client beantwortet einen 409 mit einem vollständigen Seiten-GET, und dieses GET ist ein neuer Request. Ohne erneutes Flashen würde ein von der vorherigen Anfrage geflashter Validierungsfehler oder eine Erfolgsmeldung altern, bevor die Zielseite sie lesen kann; der Benutzer verlöre die Fehlermeldung nur, weil während des Absenden ein Deploy gelandet ist. Das erfordert, dass `SessionMiddleware` vor der Version-Middleware registriert ist.

Standardmäßig müssen Sie nichts setzen: `InertiaConfig` hasht Ihr Vite-Build-Manifest (`manifest_path`, standardmäßig `public/assets/.vite/manifest.json`) und verwendet die ersten 16 Bytes seines SHA-256, hex-kodiert. Das Manifest ist die eine Datei, die sich bei jedem Build und zu keiner anderen Gelegenheit ändert; die Version erhöht sich daher selbst. Gibt es kein Manifest zu lesen - in der lokalen Entwicklung, wenn Vite aus dem Speicher liefert -, fällt sie auf den statischen String `"1.0"` zurück und protokolliert auf `debug`.

Übersteuern Sie sie, wenn Sie etwas anderes möchten:

```rust
use suprnova::{InertiaConfig, VersionResolver};

// Default - hash the build manifest. Nothing to write.
let cfg = InertiaConfig::new();

// A different manifest location; the version follows it.
let cfg = InertiaConfig::new().manifest_path("dist/.vite/manifest.json");

// Static - bake in a build-time identifier. Survives a later
// `.manifest_path(...)` call: an explicit version is deliberate.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamic - a container deployment id, anything. The closure runs on
// every version check; cache inside if it isn't cheap.
let cfg = InertiaConfig::new().version_with(|| deployment_id());
```

Das Manifest wird bei jeder Versionsprüfung gelesen, genau wie Laravels `hash_file`: einige KB aus dem Page-Cache, und ein Rebuild wird sofort übernommen. Haben Sie dies gemessen und möchten es vermeiden, lösen Sie die Version einmal beim Booten auf:

```rust
use suprnova::{InertiaConfig, VersionResolver};

let version = VersionResolver::from_manifest("public/assets/.vite/manifest.json").resolve();
let cfg = InertiaConfig::new().version(version);
```

Für asynchrone oder fehlerhafte Versionsauflösung (beispielsweise einen Manifest-Hash aus S3 lesen) führen Sie den Lesevorgang einmal beim Booten aus und übergeben den gecachten `String` an `.version(...)`.

## Bootstrap: `Inertia::install`

Die meisten Anwendungen installieren die vier Protokoll-Middlewares mit einem Aufruf aus `register_http_stack` - dem Bootstrap-Hook nur für HTTP, den der Server-Pfad ausführt und den die Binaries für Queue, Scheduler, Workflow und Konsole überspringen (siehe [Bootstrap](bootstrap.md)):

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)
        .expect("Inertia install failed (production needs a built frontend manifest)");
    // …global middleware, in the order you want it to run
}
```

```rust
// cmd/main.rs
Application::new()
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
```

Halten Sie den Aufruf aus `bootstrap::register` heraus. `Inertia::install` schlägt in Produktion fehl, wenn das gebaute Frontend-Manifest fehlt. Genau dies ist der Zustand eines Worker- oder Konsolen-Images, das kein `public/assets` ausliefert; die Installation aus dem prozessweiten Hook würde diese Binaries daher mit herunterziehen.

`Inertia::install` gibt `Result` zurück und führt in dieser Reihenfolge aus:

1. Es schlägt fehl, wenn `cfg` in den Produktionsmodus auflöst (`development == false` - die Voreinstellung, sobald `APP_ENV=production` gilt), aber kein Vite-Manifest aus `cfg.manifest_path` geladen werden kann. Dies ist die Absicherung CFG-01: Ein Produktions-Boot mit ungebautem Frontend scheitert sichtbar, statt stillschweigend auf einen alten fest codierten Asset-Pfad zurückzufallen.
2. Es registriert `InertiaHeadersMiddleware` - setzt auf jeder Response `Vary: X-Inertia` und wandelt eine leere `200` bei einem Inertia-Besuch in eine `303` zurück.
3. Es registriert `InertiaVersionMiddleware` - gibt `409` plus `X-Inertia-Location` aus, wenn Client und Server sich über die Asset-Version unterscheiden.
4. Es registriert `Inertia303Middleware` - wertet `302` bei Inertia-Redirects außerhalb von GET zu `303` auf.
5. Es registriert `InertiaValidationRedirectMiddleware` - wandelt eine `422` bei einem Inertia-Besuch in eine `303` zurück zur Formularseite mit geflashten Fehlern. Siehe [Validierungsfehler](#validierungsfehler).

Die Reihenfolge ist wichtig: Die Headers-Middleware wird zuerst registriert, ist daher die äußerste und sieht jede Response - einschließlich des `409`, den die Version-Middleware zurückgibt, bevor der Handler überhaupt läuft. Die Middleware für den Validierungs-Redirect wird zuletzt registriert, ist damit die innerste - am nächsten beim Handler - und sieht einen `422`, bevor die anderen drei Middlewares ihn berühren können.

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

Überspringen Sie den Aufruf nur, wenn Sie wirklich eine dieser Middlewares nicht möchten (selten; alle vier schließen echte Fehlermodi: Cache Poisoning zwischen den zwei Repräsentationen einer URL, stilles veraltetes Bundle, Wiederholung eines Formulars beim Redirect und ein Validierungs-`422`, das im Fehlermodal des Clients endet, statt `form.errors` zu erreichen).

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
async fn show(RouteParam(post): RouteParam<Post>, req: Request) -> Response {
    inertia_response!(&req, "Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    })
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

Suprnova kommuniziert über HTTP-Loopback mit einem SSR-Worker außerhalb des Prozesses - typischerweise dem unter Node / Bun / Deno ausgeführten Bundle `createServer()` von `@inertiajs/{svelte,react,vue}/server`. Aktivieren Sie ihn auf der Konfiguration, die Sie an [`Inertia::install`](#bootstrap-inertia-install) übergeben. Diese Konfiguration ist der Ausgangspunkt jeder Response; durch Ihre Handler muss daher nichts weitergereicht werden:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // worker URL
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

SSR ist standardmäßig ausgeschaltet und eine Eigenschaft der Konfiguration: eingeschaltet für jede aus der installierten Config gebaute Response, ausgeschaltet für jede Response, die mit einer `.with_config(...)` ohne diese Einstellung überschreibt. Ist SSR aktiviert, postet das Framework das Page-Objekt an `<url>/render` und fügt `{ head, body }` in die HTML-Shell ein. Bei Worker-Fehler oder Timeout fällt die Response auf CSR zurück (ein leeres `<div id="app">`, das der Client hydriert), und der Hook `on_ssr_error(...)` wird ausgelöst. Schalten Sie in CI `ssr_throw_on_error(true)` ein, damit diese Fehlschläge stattdessen harte 500er sind.

Noch bevor das Gateway überhaupt dispatcht, kann es prüfen, ob das gebaute SSR-Bundle auf der Festplatte existiert. Aktivieren Sie dies mit `.ssr_bundle_path(...)`, gerichtet auf das übliche `frontend/bootstrap/ssr/ssr.js`. Die Prüfung selbst ist standardmäßig eingeschaltet (`.ssr_ensure_bundle_exists(true)`), hat aber keine Wirkung, bis ein Pfad gesetzt ist. Das wird bewusst nicht automatisch erkannt, damit das Aktivieren von SSR gegen ein Test-Double nicht zusätzlich ein Bundle auf der Festplatte stubben muss. Bei einem fehlenden Bundle fällt die Response sofort auf CSR zurück, ohne `ssr_timeout` für eine Verbindung zu verbrauchen, die nie erfolgreich sein kann. Dies entspricht Laravels Konfiguration `ensure_bundle_exists`.

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")
        .ssr_bundle_path("frontend/bootstrap/ssr/ssr.js")
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

`suprnova new` scaffoldet für jeden Starter `frontend/src/ssr.{ts,tsx}` und ein npm-Skript `build:ssr`. Bauen Sie es und starten Sie anschließend den Worker:

```bash
cd frontend && npm run build:ssr
suprnova ssr:start
```

`suprnova ssr:check` prüft, ob der Worker tatsächlich antwortet: Der Befehl ruft die eigene Route `GET /health` des Workers auf, die jedes Bundle `createServer()` ohne zusätzlichen Code bereitstellt.

## Konfiguration

Das Verhalten von Inertia wird programmatisch über `InertiaConfig` konfiguriert. Die Konfiguration, die Sie an [`Inertia::install`](#bootstrap-inertia-install) übergeben, ist der Ausgangspunkt jeder Response. Die eine Umgebungsvariable, die das Framework direkt liest, ist `SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`); sie liefert nur den Standardnamen der Einstiegspunktdatei und die Endungen von Seitenkomponenten, wenn die Config nichts anderes sagt. Ein explizites `.frontend(Frontend::React)` auf der installierten Config hat Vorrang; genau das scaffoldet `suprnova new --frontend react`. Alles andere wird über Builder gesetzt:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // overrides SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // cap lazy-prop fan-out
    .with_all_errors(false)                   // one message per field, or all
    .url_resolver(|req| req.path_and_query()) // how `page.url` is derived
    .production();                            // false → loads from Vite dev server
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

Der Resolver liest den Request über `InertiaRequestExt` und gilt für jede Response, die aus der Konfiguration gebaut wird, die Sie an [`Inertia::install`](#bootstrap-inertia-install) übergeben - der übliche Ort für einen Resolver, der appweit gelten soll. Übersteuern Sie ihn für eine einzelne Response mit `InertiaResponse::with_config(cfg)`. Ein Resolver ändert nur `page.url`. Der 409-Bounce benennt weiterhin die URL, die tatsächlich angekommen ist - genau die URL, die der Browser abrufen muss. Mit einem Resolver unterscheiden sich die beiden daher bewusst.

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

Neun weitere Rust-förmige Optionen, die zu markieren sind:

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
- **`.lazy()` ist nicht Laravels `Inertia::lazy()`.** Laravels Methode ist veraltet und verhält sich wie `optional()`: `LazyProp` ist ein direkter Alias für `OptionalProp`, der beim ersten Besuch vollständig ausgelassen wird (`ResponseFactory.php:174-181`). Suprnovas `.lazy()` folgt der reinen Closure-Konvention, die Laravel selbst für eine aufrufbare Prop ohne Wrapper verwendet - sie wird immer eingeschlossen, wenn Partial-Reload-Filterung den Schlüssel durchlässt, auch bei Standardbesuchen. Verwenden Sie `.optional()` für das beim Erstbesuch ausgelassene Verhalten, das der Name „lazy“ nahelegt, wenn Sie aus Laravel kommen.
- **Verschachteltes `only`/`except` grenzt nach der Auflösung ein, nicht davor.** Laravels `Response::resolvePartialProperties` läuft den punktierten Pfad durch das rohe, noch nicht aufgelöste Prop-Array; ein Pfad in eine `LazyProp` oder `DeferProp` degradiert daher zu `null` - der Durchlauf trifft auf eine nicht aufgelöste Closure und hält an (`inertia-laravel-2.0.25/src/Response.php:273-297`). Suprnova löst zuerst den Wert jeder Prop auf - Resolver sind asynchron, daher gibt es keinen synchronen Punkt, an dem sie wie bei Laravel manchmal alle einfache Arrays sind - und grenzt anschließend den resultierenden JSON-Wert ein. Ein unbekannter oder typinkompatibler verschachtelter Pfad wird weggelassen, statt als `null` zurückgesendet. Das entspricht der Reconciliation des Clients: Sie führt ein eingegrenztes Objekt tief mit dem zusammen, was sie bereits hält (`inertia-3.6.1/packages/core/src/response.ts:414-425`); ein fremdes `null` würde ein bereits vorhandenes Feld überschreiben, statt es unangetastet zu lassen.
- **`.scroll_wrapped` ist Opt-in, nicht automatisch.** Laravels `Inertia::scroll($value, $wrapper = 'data', …)` verschachtelt die Merge-Anweisung jeder Scroll-Prop standardmäßig unter `"data"`, weil eine Laravel-Paginator-Resource typischerweise `{ data: [...], links: {...}, meta: {...} }` zurückgibt und nur das Array zusammengeführt werden soll. Suprnovas eingebaute Paginatoren geben ein nacktes Zeilen-Array zurück (`Vec<T>`, keine Envelope); `.scroll` / `.paginate` führen daher am Root der Prop zusammen, und `.scroll_wrapped` ist für die Fälle vorgesehen, die stattdessen den verschachtelten Pfad benötigen.
- **Eine gewrappte Scroll-Prop präfigiert ihre Felder `match_on` für Sie.** Bei einer Prop `.scroll_wrapped("posts", "data")` gibt `match_on("id")` `"posts.data.id"` aus. Laravel gibt das nicht präfigierte `"posts.id"` aus, das sein eigener Client dann nicht am Merge-Ziel ausrichten kann; der Match wird daher still nie ausgelöst. Der Verschachtelungspunkt ist hier eindeutig - eine Scroll-Prop hat höchstens einen Wrapper -, daher leitet Suprnova das Präfix ab, statt Sie es eingeben zu lassen. Schreiben Sie den nackten Feldnamen, nicht den Pfad.

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

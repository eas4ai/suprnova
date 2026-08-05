# Frontend - Übersicht

Suprnova verbindet Rust-Handler mit einem Single-Page-Frontend über
[Inertia.js](https://inertiajs.com/) 3.4.0. Controller werden in Rust
geschrieben und Seiten in Svelte, React oder Vue; das Framework
transportiert typisierte Props zwischen ihnen ohne eine separate HTTP-API
dazwischen.

## Drei erstklassige Starter

`suprnova new <name>` erstellt ein funktionierendes Projekt. Das `--frontend`
Flag wählt die SPA-Ebene:

```bash
suprnova new my-app                       # Svelte 5 (Standard)
suprnova new my-app --frontend svelte     # Svelte 5
suprnova new my-app --frontend react      # React 19
suprnova new my-app --frontend vue        # Vue 3.5
```

Alle drei Scaffolds nutzen denselben Stack:

| Ebene | Version |
|---|---|
| Inertia-Client-Adapter | `@inertiajs/{svelte,react,vue3}` 3.4.0 |
| Build-Tool | Vite 8 |
| Styling | Tailwind v4 (`@tailwindcss/vite`) |
| TypeScript | strict mode |

Die Wahl ist pro Projekt. Es gibt kein "primäres" Framework auf der
Serverseite - `inertia_response!` löst die Dateiendung auf, die der
gewählte Scaffold nutzt (`.svelte`, `.tsx`, `.vue`), und `App::inertia_share`,
Partial Reloads sowie TypeScript-Prop-Generierung verhalten sich über
alle drei identisch.

## Architektur

```
                       Browser
   +-------------------------------------------------+
   |               SPA (Svelte / React / Vue)        |
   |   +---------------+ +---------------+           |
   |   | Home.svelte   | | Users/Show.tsx|  ...      |
   |   +-------+-------+ +-------+-------+           |
   |           |  typisierte Props vom Rust-Struktur |
   |   +-------v-------------------------------+     |
   |   |        Inertia-Client-Adapter         |     |
   +---+------------------+------------------+--+----+
                          |
                          |   HTTP (JSON über XHR, HTML beim ersten Load)
                          v
   +-------------------------------------------------+
   |                  Suprnova-Server                |
   |   +------------------------------------------+  |
   |   |          Controller / Handler            |  |
   |   |   inertia_response!(&req, "Home",        |  |
   |   |                     HomeProps { ... })   |  |
   |   +------------------------------------------+  |
   +-------------------------------------------------+
```

Die erste Anfrage gibt eine HTML-Shell mit dem ursprünglichen Page-Objekt
zurück, das im `data-page`-Attribut des Mount-Knotens eingebettet ist.
Nachfolgende Besuche erfolgen über `<Link>` / `router.visit`, senden
`X-Inertia: true` und erhalten ein JSON-Page-Objekt zurück - der Adapter
wechselt die Komponente ohne vollständiges Neuladen.

## Ein vollständiger Seiten-Roundtrip

Der Controller definiert seine Props als Rust-Struktur, leitet
`InertiaProps` ab und übergibt den Wert dem `inertia_response!`-Makro:

```rust
use suprnova::{InertiaProps, Request, Response, inertia_response};

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

Das Makro nimmt einige Aufgaben für Sie ab. Erstens validiert es zur
Kompilezeit, dass die Seiten-Komponentendatei tatsächlich unter
`frontend/src/pages/Home.{svelte,tsx,jsx,vue}` existiert - Tippfehler
werden als Build-Fehler erkannt, nicht als 404 im Browser. Zweitens
serialisiert es die `HomeProps`-Struktur, entfaltet sie in einen Prop pro
Top-Level-Schlüssel, damit Partial Reloads filtern können, und löst
beliebige Lazy oder Deferred Props gegen `&req` auf, bevor es zurückgibt.
Das Makro ergibt einen `Result<HttpResponse, FrameworkError>`, den der
`Response`-Rückgabetyp direkt akzeptiert.

Die entsprechende Svelte-Seite (der Standard-Scaffold):

```svelte
<!-- frontend/src/pages/Home.svelte -->
<script lang="ts">
  import type { HomeProps } from '../types/inertia-props'

  let { title, message }: HomeProps = $props()
</script>

<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">{title}</h1>
  <p class="mt-2">{message}</p>
</div>
```

Für die React- und Vue-Entsprechungen siehe [Seiten-Komponenten](frontend-pages.md).

## TypeScript-Typen generieren

Jede `#[derive(InertiaProps)]`-Struktur in `src/` wird zu einer
TypeScript-Schnittstelle in `frontend/src/types/inertia-props.ts`:

```bash
suprnova generate-types
```

Mit dem Flag `--routes` gibt derselbe Befehl auch
`frontend/src/types/routes.ts` aus - typsichere URL- + Methodenpaare, die
aus dem `routes!`-Makro gescraped werden und direkt mit Inertia v2+-
APIs funktionieren. Die vollständige Typ-Zuordnungstabelle und die
Route-Helper-Form sind in [TypeScript Types](frontend-typescript-types.md) dokumentiert.

## Gemeinsame Daten

Alles, das auf jeder Seite erscheinen soll (der authentifizierte Benutzer,
das aktuelle Gebietsschema, App-Metadaten), wird einmal beim Start
registriert und in jede Inertia-Antwort zusammengeführt:

```rust
// In bootstrap.rs
App::inertia_share("appName", "Suprnova");
App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

// Async / pro-Request gemeinsame Daten gehen durch das Trait.
App::register_inertia_shared(Arc::new(AppSharedData));
```

Drei Varianten in Precedence-Reihenfolge (später gewinnt beim gleichen Schlüssel):

| API | Wenn sich der Wert materialisiert |
|---|---|
| `App::inertia_share(k, v)` | Sync, einmal beim Start gesetzt |
| `App::inertia_share_lazy(k, \|\| async { ... })` | Pro Antwort, neu berechnet |
| `App::inertia_share_once(k, \|\| async { ... })` | Pro Antwort, dann Client-gecacht |
| `App::register_inertia_shared(Arc::new(impl))` | Pro Request, sieht `&req` |

Props pro Seite, die dem Response-Builder beigefügt sind, überschreiben
immer gemeinsame Daten beim gleichen Schlüssel.

## Partial Reloads und Lazy Props

Derselbe `InertiaResponse`-Builder stellt das volle Prop-Toolkit von
Inertia v3 zur Verfügung - eager, lazy, optional, deferred, merge, once -
und Suprnova unterstützt die v3-Partial-Reload-Header (`X-Inertia-Partial-Data`,
`X-Inertia-Partial-Except`, `X-Inertia-Reset`,
`X-Inertia-Except-Once-Props`) automatisch. Das folgende Beispiel
fügt drei Props mit unterschiedlichen Evaluierungsregeln bei:

```rust
use suprnova::{InertiaResponse, FrameworkError, Request, Response};

pub async fn dashboard(req: Request) -> Response {
    let resp = InertiaResponse::new("Dashboard")
        .with("title", "Dashboard")
        .lazy("recent_orders", || async {
            Ok::<_, FrameworkError>(load_recent_orders().await?)
        })
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        .resolve(&req)
        .await?;
    Ok(resp)
}
```

`inertia_response!` deckt den Eager-Props-Fall ab; alles darüber hinaus
geht durch den Builder. Die vollständige Oberfläche - `optional`, `merge`,
`once`, `scroll`, `flash`, `paginate`, SSR, Version-Konflikt,
Verschlüsselung der Historie - ist in [Inertia Responses](frontend-inertia-responses.md) dokumentiert.

## Bootstrap

Eine erstellte App installiert die beiden protokoll-kritischen Middlewares
in einem Aufruf innerhalb von `bootstrap.rs`:

```rust
use suprnova::{Inertia, InertiaConfig};

Inertia::install(&InertiaConfig::new().version(env!("CARGO_PKG_VERSION")))
    .expect("Inertia install failed");
```

`install` gibt `Result` zurück - es schlägt fehl, wenn `InertiaConfig` zu
Production-Mode auflöst (Standard unter `APP_ENV=production`), aber kein
Vite-Manifest gefunden wird, statt stillschweigend auf einen veralteten
Asset-Pfad zurückzufallen. Siehe [Entwicklung vs. Production](#entwicklung-vs-production)
unten.

Dies registriert `InertiaVersionMiddleware` (gibt 409 + `X-Inertia-Location`
bei Versions-Konflikt aus, damit veraltete Clients neu laden) und
`Inertia303Middleware` (schreibt 302 → 303 bei Non-GET Inertia-Besuchen um,
damit die Folgeanfrage eindeutig ein GET ist). Beide waren früher optional;
`Inertia::install` macht sie zum Standard.

## Entwicklung vs. Production

In der Entwicklung läuft der Vite-Entwicklungsserver parallel zum Backend
und versorgt HMR-aktivierte Assets:

```bash
suprnova serve
```

Dies startet den Rust-Server und `vite` gemeinsam. Die HTML-Shell lädt
Module von `http://localhost:5765`.

Für Production wird das Frontend einmal gebaut und das Backend auf das
Hash-Manifest unter `public/assets/` gerichtet:

```bash
cd frontend && npm run build
APP_ENV=production suprnova serve --backend-only
```

`InertiaConfig::default()` leitet den Production- gegenüber Development-Mode
von `APP_ENV` ab (über `Environment::detect().is_production()`) -
`APP_ENV=production` bewirkt, dass die HTML-Shell erstellte Assets statt
des Vite-Entwicklungsservers lädt. `Inertia::install` schlägt dann beim
Start sichtbar fehl, wenn es kein Manifest findet, um diese Entscheidung
zu unterstützen, statt stillschweigend auf einen veralteten codierten
Pfad zurückzufallen.

Suprnova liest `public/assets/.vite/manifest.json`, um Hash-Einstiegspunkte
plus alle transitiven Importe für `modulepreload` aufzulösen. SSR ist
optional - aktivieren durch Zeigen von `InertiaConfig::ssr(...)` auf einen
laufenden `@inertiajs/{vue3,react,svelte}/server`-Worker.

### Warum Suprnova abweicht

Drei beabsichtigte Abweichungen von dem, wie ein typisches Inertia-Setup
anderswo aussieht:

- **Validierung von Komponenten zur Kompilezeit.** Das `inertia_response!`-Makro
  läuft `frontend/src/pages/` zur Build-Zeit durch und weigert sich zu expandieren,
  wenn die Komponentendatei fehlt, indem es die nächste Entsprechung vorschlägt. Sie
  können keinen Controller deployen, der auf eine gelöschte Seite zeigt.
- **Typisierte Props als Quelle der Wahrheit.** Seiten-Props sind Rust-Strukturen
  mit `#[derive(InertiaProps)]`. `suprnova generate-types` liest diese
  und schreibt TypeScript-Schnittstellen - die Frontend-Typen werden von
  Backend abgeleitet, nicht parallel gepflegt.
- **Svelte als Standard.** Die Inertia-Dokumentation greift zuerst zu Vue und
  React; der Suprnova-Scaffolder setzt standardmäßig auf Svelte 5 (mit Runes).
  React 19 und Vue 3.5 sind erstklassig, keine Nachgedanken - gleiches
  Protokoll, gleiche Prop-Pipeline, gleiche Generator-Ausgabe.

## Nächste Schritte

- [Seiten-Komponenten](frontend-pages.md)
- [Inertia Responses](frontend-inertia-responses.md)
- [TypeScript Types](frontend-typescript-types.md)
- [Routing](routing.md)
- [Controller](controllers.md)

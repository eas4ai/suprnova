# Paginierung

Suprnova liefert drei Paginatoren, die Laravels Oberfläche
deckungsgleich abbilden: längenbewusst (kennt die Gesamtzahl), simple
(eine Query pro Seite) und Cursor (opakes Keyset). Alle drei leiten
`Serialize` in das Laravel-förmige JSON ab, das Inertia- und
JSON:API-Consumer schon verstehen - Sie holen eine Seite und geben
sie zurück; nichts weiter ist nötig.

```rust
use crate::models::User;

let page = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;
```

Dieser eine Aufruf führt das `COUNT(*)` und den `LIMIT/OFFSET`-Abruf
der Seite aus, parst `?page=N` aus der aktiven Anfrage und liefert
einen versandfertigen `LengthAwarePaginator<User>`. Die zwei
Geschwister - `simple_paginate(20)` und `cursor_paginate(20)` -
liefern dieselbe Form von Wert mit anderen Trade-offs. Der Rest
dieses Kapitels behandelt, wann man zu welchem greift, was jeder
kostet, und wie das JSON ankommt.

## Einen Paginator wählen

Der schnellste Weg zur Wahl ist die Trade-off-Tabelle:

| Methode | Typ | Queries / Seite | Kennt die Gesamtzahl? | Wann verwenden |
|---|---|---|---|---|
| `paginate(n)` | `LengthAwarePaginator<M>` | 2 (`COUNT(*)` + Seite) | ja | Die UI zeigt numerische Seiten oder „Seite 3 von 17“ |
| `simple_paginate(n)` | `Paginator<M>` | 1 (`LIMIT n+1`) | nein | Große Tabellen; ein „Weiter“-Button reicht |
| `cursor_paginate(n)` | `CursorPaginator<M>` | 1 (`LIMIT n+1`) | nein | Infinite Scroll; tiefe Seiten auf stark beanspruchten Tabellen |

Der Kostenunterschied zählt, sobald Ihre Tabelle groß ist.
`COUNT(*)` über hundert Millionen Zeilen ist die teuerste Query in
Ihrem Request-Budget. `simple_paginate` spart den Count.
`cursor_paginate` spart den Count *und* vermeidet den linearen Scan
über `OFFSET N`, der jede Anfrage nach einer tiefen Seite auf einer
großen Tabelle ausbremst - ein Cursor-Seek ist mit dem richtigen
Index in etwa `O(1)`, unabhängig davon, wo in der Ergebnismenge der
Nutzer steht.

### Warum Suprnova abweicht

Laravels Paginatoren tragen URL-Bau-Helfer -
`nextPageUrl()`, `previousPageUrl()`, das `links`-Array aus
`{url, label, page, active}`-Deskriptoren, das Blade rendert.
Suprnovas rohe `Serialize`-Impl emittiert den Datenausschnitt plus
die Zähler; der URL-Bau lebt auf den Response-Form-Konstruktoren, die
schon URL-Kontext besitzen:
[`Inertia::paginate`](frontend-inertia-responses.md) hängt
Inertia-Scroll-Metadaten an (Seiten-Identifier, keine absoluten
URLs); [`Resource::paginated`](eloquent-resources.md) hängt
JSON:API-`links.{self,first,last,prev,next}` gemäß der
JSON:API-Empfehlung an.

Zwei Gründe für die Aufteilung. Erstens hängt die URL, die der Client
sehen soll, davon ab, welche Protokoll-Oberfläche sie rendert -
Inertia richtet sich nach Seiten-Identifiern, JSON:API will absolute
Hrefs. Zweitens kennt der Paginator standardmäßig nicht die Basis-URL
der Anfrage; die Helfer, die sie kennen, können die URLs einmal
anhängen, wo sie hingehören. Brauchen Sie doch URLs am blanken
Paginator (eigenes JSON-Envelope, Telemetrie-Payload, Test-Assertion),
rufen Sie `with_path(...)` auf und verwenden Sie `url_for_page(n)` -
behandelt im Abschnitt [URL-Generierung](#url-generierung-und-pfade).

## `paginate` - längenbewusst

```rust
use suprnova::LengthAwarePaginator;
use crate::models::User;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let page: LengthAwarePaginator<User> = User::query()
        .filter("active", true)
        .order_by_desc("created_at")
        .paginate(20)
        .await?;

    Ok(suprnova::json_response!(page))
}
```

Die öffentlichen Felder der Struktur:

```rust
pub struct LengthAwarePaginator<T> {
    pub data: Vec<T>,           // Zeilen auf dieser Seite
    pub current_page: u64,       // 1-basiert
    pub last_page: u64,          // 1-basiert; 0 wenn total == 0
    pub per_page: u64,
    pub total: u64,              // jede Zeile über alle Seiten
    pub from: Option<u64>,       // 1-basierter Index der ersten Zeile dieser Seite
    pub to: Option<u64>,         // 1-basierter Index der letzten Zeile dieser Seite
    pub path: Option<String>,    // Basis-URL für url_for_page (optional)
}
```

Das JSON, das das abgeleitete `Serialize` emittiert:

```json
{
  "data": [...],
  "current_page": 1,
  "last_page": 3,
  "per_page": 10,
  "total": 25,
  "from": 1,
  "to": 10,
  "path": "/api/users"
}
```

`path` wird aus dem JSON ausgelassen, wenn nicht gesetzt; `from` und
`to` sind `null`, wenn die Seite leer ist (keine Zeilen auf dieser
Seite, oder die angeforderte Seite liegt hinter der letzten Seite).

### `?page=N` automatisch lesen

`paginate(n)` liest die aktuelle Seite aus `?page=N` auf der aktiven
Anfrage über `Context::query_param`. Fehlende, leere,
nicht-numerische und Null-Werte werden auf `1` begrenzt. Es gibt
nichts zu verdrahten - ist eine Anfrage im Scope, wird der Parameter
gelesen.

### Mehrere Paginatoren auf einer Seite

Wenn eine Seite mehr als eine paginierte Liste rendert, geben Sie
jeder mit `paginate_using` ihren eigenen Query-String-Key:

```rust
let posts = Post::query()
    .order_by_desc("created_at")
    .paginate_using("posts_page", 10)
    .await?;

let comments = Comment::query()
    .order_by_desc("created_at")
    .paginate_using("comments_page", 25)
    .await?;
```

`paginate_using` setzt außerdem `page_name` auf dem gelieferten
Paginator, sodass `url_for_page` URLs mit demselben Key baut:

```rust
posts.url_for_page(2);     // "/posts?posts_page=2"  (wenn path gesetzt ist)
comments.url_for_page(3);  // "/posts?comments_page=3"
```

### Seitenpositions-Prädikate

Das vollständige Prädikaten-Set von Laravels `AbstractPaginator` ist
implementiert:

```rust
page.has_more_pages();   // current_page < last_page
page.on_first_page();    // current_page <= 1
page.on_last_page();     // !has_more_pages()
page.has_pages();        // wir sind nicht auf Seite 1 ODER es gibt weitere Seiten
page.is_empty();         // data.is_empty()
page.is_not_empty();     // !is_empty()
page.count();            // data.len() - Ausschnitt der Seite, nicht die Gesamtzahl
```

`count()` ist die Größe des Ausschnitts, nicht die Gesamtzahl -
Laravels `Countable`-Form; für die Gesamtzahl verwenden Sie direkt
das Feld `total`.

## `simple_paginate` - eine Query, kein `COUNT(*)`

```rust
use suprnova::Paginator;
use crate::models::User;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;
```

```rust
pub struct Paginator<T> {
    pub data: Vec<T>,
    pub current_page: u64,
    pub per_page: u64,
    pub has_more: bool,          // gab es eine zusätzliche Zeile über per_page hinaus?
    pub path: Option<String>,
}
```

JSON:

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true,
  "path": "/api/users"
}
```

Der Trick liegt im SQL. `simple_paginate(20)` gibt `LIMIT 21` aus,
schaut, ob die 21. Zeile zurückkam, setzt `has_more` danach, und
kürzt `data` wieder auf 20. Eine Query pro Seite; kein `COUNT(*)`.

Dafür geben Sie `total`, `last_page`, `from` und `to` auf. Im
Gegenzug können Sie Tabellen paginieren, auf denen `COUNT(*)` zu
teuer ist, um bei jedem Seitenaufruf zu laufen. Die UI-Oberfläche ist
„Weiter“- / „Zurück“-Buttons, nicht „Seite 7 von 142“.

Dasselbe Prädikaten-Set wie beim längenbewussten Paginator ist
implementiert: `has_more_pages()`, `on_first_page()`,
`on_last_page()`, `has_pages()`, `is_empty()`, `is_not_empty()`,
`count()`.

## `cursor_paginate` - opakes Keyset

```rust
use suprnova::CursorPaginator;
use crate::models::User;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;
```

```rust
pub struct CursorPaginator<T> {
    pub data: Vec<T>,
    pub per_page: u64,
    pub next_cursor: Option<String>,  // None auf der letzten Seite
    pub prev_cursor: Option<String>,  // None auf der ersten Seite
    pub path: Option<String>,
}
```

JSON:

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` und `prev_cursor` sind immer als JSON-Keys vorhanden
(`null`, wenn abwesend), sodass Client-Schemas sich auf die
Feld-Präsenz verlassen können; `path` wird ausgelassen, wenn nicht
gesetzt.

### Wie Cursor auf dem Wire funktionieren

Der Client übergibt den Cursor der vorherigen Seite über
`?cursor=<opaque>`:

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

`cursor_paginate` dekodiert den Cursor, läuft den Keyset-Filter ab
(`pk > boundary ASC` für `next`; `pk < boundary DESC` für `prev`,
zurück auf ASC gedreht), holt `LIMIT n+1` Zeilen und emittiert
`next_cursor` / `prev_cursor` neu, je nachdem, ob die Nachbarn der
Seite existieren. Es ist bidirektional - der Client kann vorwärts und
zurück laufen, ohne seine Position zu verlieren.

Cursor-Paginierung **ersetzt** jedes vorhandene `ORDER BY` auf dem
Builder. Eine stabile Gesamtordnung über den Primärschlüssel ist
erforderlich, damit der Keyset-Filter die Tabelle deterministisch
aufteilen kann; ein Cursor mit einem beliebigen `ORDER BY
random_score()` würde Zeilen überspringen und duplizieren. Brauchen
Sie eine Sortierung, die nicht auf dem PK basiert, wechseln Sie zu
`paginate` / `simple_paginate`.

### Cursor sind verschlüsselt und authentifiziert

Suprnova-Cursor sind **nicht** Laravels Base64-JSON-Klartext. Der
Wire-Cursor ist der Keyset-Grenzwert (ein typisierter
`sea_orm::Value` - `Int`, `BigInt`, `Uuid`, Datetimes, Decimals,
Strings, Bytes) plus ein Richtungs-Tag, JSON-kodiert und dann mit
AES-256-GCM über den Framework-`Crypt`-Schlüsselring versiegelt
(gebunden an `CryptPurpose::Cursor`, sodass ein Cursor-Chiffretext nie
in eine andere Oberfläche wiedereingespielt werden kann - Cookie,
2FA-Secret, Cast).

Das bedeutet in der Praxis drei Dinge:

1. **Keine Manipulation.** Ein Client, der Bits in `?cursor=`
   umklappt, bekommt ein 400 `Invalid pagination cursor`, keine
   andere Seite von Daten.
2. **Kein Informationsleck.** Der Grenzwert (oft ein Primärschlüssel,
   manchmal ein Zeitstempel) ist im Cursor versiegelt - Clients
   können keine Bereiche erkunden, indem sie ihn bearbeiten.
3. **Typisierte Grenzwerte überleben den Hin- und Rückweg
   verlustfrei.** Der Wire-Umschlag markiert die SeaORM-Variante
   (`"BigInt"`, `"Uuid"` usw.), sodass sich der Wert beim Dekodieren
   mit demselben SQL-Typ neu bindet, den die ursprüngliche Spalte
   emittiert hat. Keine String-Coercion-Bugs über Postgres / MySQL /
   SQLite hinweg.

Es gibt keinen Klartext-Fallback. Ist `Crypt` nicht initialisiert -
was nach `Server::from_config` unmöglich sein sollte -, schlägt die
Kodierung mit einem Fehler fehl, statt einen fälschbaren Cursor
auszugeben.

### Warum Suprnova abweicht

Laravels Cursor-Paginator ist standardmäßig nur vorwärts gerichtet,
und der Wire-Cursor ist ein Base64-kodierter JSON-Blob - lesbar,
editierbar, wiedereinspielbar. Suprnovas Cursor ist bidirektional
(passend zur `cursorPaginate()`-Oberfläche, die Laravel später
hinzugefügt hat) und ist Ende-zu-Ende authentifiziert, sodass der
Client keinen konstruieren oder verändern kann. Das Rust-Ökosystem
hat AES-GCM bereits als Primitive; es zu verwenden kostet das
Framework eine zusätzliche Trait-Impl und gibt jedem Cursor eine
Sicherheitseigenschaft, die ein Klartext-Base64-Payload nicht bieten
kann.

## Die Facade - `Pagination::length_aware` / `Pagination::cursor`

Die meisten Kapitel dieses Handbuchs zeigen Paginierung über den
Eloquent-Builder, weil das der übliche Pfad ist. Bauen Sie direkt
einen SeaORM-`Select<E>` - sagen wir, weil Sie für einen Report auf
eine nicht modellierte Query joinen -, ist die Facade `Pagination`
die äquivalente Oberfläche:

```rust
use suprnova::{Pagination, LengthAwarePaginator};
use sea_orm::EntityTrait;

let select = User::find()  // oder ein beliebiger SeaORM Select<E>
    .filter(user::Column::Active.eq(true));

let page: LengthAwarePaginator<user::Model> =
    Pagination::length_aware(select, 20, 1).await?;
```

Die Facade bietet außerdem `length_aware_on(conn, ...)` und
`cursor_on(conn, ...)` zum Routen an eine bestimmte benannte
Connection, sowie eine typisierte Form
`cursor(query, cursor, per_page, order_col)`, die die Keyset-Spalte
explizit nimmt - verwendet, wenn der Cursor nach etwas anderem
sortiert als dem Primärschlüssel.

Die Routing-Regeln entsprechen dem Eloquent-Builder. Eine umgebende
`DB::transaction` wird respektiert (sowohl das COUNT als auch die
Seiten-Query laufen auf der Connection der Transaktion), und eine
registrierte `__read_replica__`-Connection wird für Reads
automatisch verwendet. Das Sentinel `__primary__` wählt den
Standard-Pool, wenn Sie die Replica umgehen wollen.

## Validierung - `per_page == 0`

Alle drei Methoden weisen `per_page == 0` zurück:

```rust
let result = User::query().paginate(0).await;
assert!(matches!(
    result,
    Err(FrameworkError::ParamError { ref param_name }) if param_name == "per_page",
));
```

Der Fehler rendert als HTTP 400 mit dem Standard-Fehlerkörper. Es
gibt keine stille „leere Seite“ - eine Seitengröße von null ist immer
falsch und wird an der Aufrufstelle zurückgewiesen, passend zum
Eloquent-Builder und zur Facade `Pagination`. Dieselbe Validierung
lebt auf `cursor_paginate`, `simple_paginate`,
`Pagination::length_aware`, `Pagination::length_aware_on`,
`Pagination::cursor` und `Pagination::cursor_on` - eine Regel, sechs
Einstiegspunkte.

Der Wert `current_page` wird **begrenzt**, nicht validiert: `0` wird
zu `1`, negative Zahlen von einem defensiven Frontend können nicht
vorkommen (der Parser ist `u64`), und jedes `?page=N`, das größer als
`last_page` ist, liefert einen Paginator mit leerem `data` plus
`from`/`to` von `None`. Über das Ende hinauszulaufen ist der Fehler
des Clients, nicht ein Fehler des Servers.

## Fehlerform

| Bedingung | Variante | HTTP |
|---|---|---|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| Manipulierter / ungültiger Cursor | `FrameworkError::Domain` (`"Invalid pagination cursor"`) | 400 |
| `Crypt` beim Cursor-Decode nicht initialisiert | `FrameworkError::Internal` | 500 |
| Cursor-Varianten-Mismatch bei `decode_cursor` | `FrameworkError::Internal` | 500 |
| Zugrunde liegender DB-Fehler | `FrameworkError::Database` | 500 |

Der Fall des manipulierten Cursors ist der, den man sich merken
sollte. Cursor werden direkt vom Wire gelesen - der Query-String
`?cursor=…` ist per Definition Angreifer-Eingabe, und bit-geflipptes
Base64 und wiedereingespielte Chiffretexte sind erwartete
Fehlerfälle, keine Serverbugs. Der Entschlüsselungsschritt stuft auf
ein 400 `Invalid pagination cursor` herab, damit vom Client
auslösbare Fehler nicht den 500er-Telemetriekanal verschmutzen. Die
statische Meldung gibt dem Client nichts, womit er sondieren könnte.

Fehler nach der Entschlüsselung (JSON-Parse, Variantentag-Dispatch,
Richtungs-Parse) bleiben 500 - jede Byte-Sequenz, die die
AEAD-Authentifizierung überstanden hat, wurde von uns selbst erzeugt,
sodass ein fehlerhaftes Payload jenseits dieses Punkts ein
Framework-Bug ist, den es zu melden lohnt.

## URL-Generierung und Pfade

Der rohe Paginator trägt ein optionales Feld `path`. Ist es gesetzt,
verwenden `url_for_page(n)` und die Cursor-Link-Emission es, um
Query-Strings zu bauen:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/api/users");

page.url_for_page(1);    // "/api/users?page=1"
page.url_for_page(2);    // "/api/users?page=2"
```

Trägt der Basis-Pfad schon einen Query-String, wechselt der Trenner
zu `&`, damit die URL wohlgeformt bleibt:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/users?sort=name");

page.url_for_page(2);    // "/users?sort=name&page=2"
```

Ist `path` nicht gesetzt, fällt `url_for_page` auf eine blanke
relative Query zurück: `?page=2`. Der Name des Seiten-Parameters
kommt von `with_page_name(...)` (Standard `"page"`);
`paginate_using(name, n)` setzt ihn automatisch, sodass die erzeugten
URLs denselben Key verwenden, aus dem der Paginator getrieben wurde.
Der Parametername wird form-urlencodiert, sodass selbst ein Name mit
reservierten Zeichen die URL nicht beschädigen kann.

Cursor-Paginatoren haben dieselbe Form: `with_path(...)` setzt die
Basis, `with_cursor_name(...)` überschreibt den Query-Key (Standard
`"cursor"`), und der JSON:API-Link-Builder greift sie automatisch
auf.

Die meisten Apps rufen `url_for_page` nicht direkt auf - sie geben
den Paginator an eine der beiden Integrations-Oberflächen unten
weiter, die die URLs für ihr Protokoll richtig bauen.

## Inertia-Integration - Infinite-Scroll-Props

Für Inertia-Frontends hängt der Helfer
`Inertia::paginate(component, key, paginator)` den Paginator als
Scroll-Prop an:

```rust
use suprnova::Inertia;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .order_by_desc("created_at")
        .cursor_paginate(20)
        .await?;

    Ok(Inertia::paginate("Users/Index", "users", users).into())
}
```

Alle drei Paginatoren funktionieren hier - `LengthAwarePaginator`,
`Paginator` und `CursorPaginator`. Der Metadaten-Seitenname kommt vom
Paginator selbst: `"page"` für die beiden Offset-Paginatoren,
`"cursor"` für `CursorPaginator`. Der Client bekommt die Zeilen unter
dem gewählten Prop-Key plus einen `ScrollMetadata`-Deskriptor mit
`current_page`, `next_page`, `previous_page` (Seiten-Identifier für
die Offset-Paginatoren; Cursor-Strings für Cursor-Paginatoren) - den
die Inertia-Helfer `useInfiniteScroll` / `WhenVisible` für Infinite
Scroll konsumieren.

`simple_paginate` verdient eine eigene Erwähnung, weil eine Liste
über einer Tabelle, die groß genug ist, um `COUNT(*)` zu den
dominanten Kosten der Anfrage zu machen, genau der Fall ist, in dem
eine Inertia-Collection-Seite schmerzt:

```rust
let users = User::query()
    .order_by_asc("id")
    .simple_paginate(20)     // kein COUNT, eine Query
    .await?;

Ok(Inertia::paginate("Users/Index", "users", users).into())
```

Sein `next_page` kommt von der `LIMIT n+1`-Überlauf-Probe statt von
einer berechneten letzten Seite, da es keine Gesamtzahl gibt, aus der
man eine berechnen könnte. Der Client bekommt „es gibt noch eine
Seite“ statt „es gibt 4.812 Seiten“ - was alles ist, was eine
Infinite-Scroll-UI je liest.

### Zeilen projizieren, bevor sie ausgeliefert werden

Paginatoren haben kein `map` / `through` (Laravels haben es).
Bauen Sie statt dessen aus den öffentlichen Feldern neu auf - die
Zähler und Cursor beschreiben die *Query*, sodass sie einen Wechsel
des Zeilentyps unverändert überstehen:

```rust
let page = User::query().cursor_paginate(20).await?;

let page = suprnova::CursorPaginator::new(
    page.data.into_iter().map(PublicUser::from).collect(),
    page.per_page,
    page.next_cursor,
    page.prev_cursor,
);
```

Das lohnt sich, statt das Model direkt zu serialisieren, wann immer
die Route unauthentifiziert ist und das Model irgendetwas trägt, das
der Aufrufer nicht sehen sollte. Ein Cursor über eine Nutzertabelle
gibt jeweils eine Seite heraus, aber irgendwann jede Seite.

Derselbe Helfer existiert als verkettbare Methode auf
`InertiaResponse::paginate(key, paginator)`, wenn Sie einen Paginator
mit anderen Props mischen wollen:

```rust
inertia_response!("Dashboard")
    .with("stats", &stats)
    .paginate("recent_users", users)
    .into()
```

Siehe [Inertia Responses](frontend-inertia-responses.md) für das
breitere Prop-Modell.

## JSON:API-Integration - `Resource::paginated`

Für JSON:API-Consumer baut `Resource::paginated(paginator)` den
vollständigen Umschlag:

```rust
use suprnova::Resource;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .paginate(20)
        .await?
        .with_path("/api/users");

    Ok(Resource::paginated(users).into())
}
```

Die Response trägt:

- `data` - jede Zeile, gerendert durch `IntoJsonResource` des Models.
- `meta.pagination` - `{ total, per_page, current_page, last_page }`
  für längenbewusst; `{ next_cursor, prev_cursor }` für Cursor.
- `links.{self,first,last,prev,next}` - absolute Hrefs für den
  längenbewussten Paginator (gebaut aus `path`); `links.{prev,next}`
  für den Cursor-Paginator.

Beide Paginator-Typen implementieren den Trait `Paginated<T>`, den
`Resource::paginated` konsumiert - es gibt keinen separaten Codepfad
für längenbewusst gegen Cursor. Bauen Sie einen eigenen
paginator-artigen Typ, der `Paginated<T>` implementiert, komponiert
er auf dieselbe Weise.

Siehe [JSON:API-Resources](eloquent-resources.md) für das
Resource-Modell.

## Eigene JSON-Envelopes

Passt weder Inertia noch JSON:API zu Ihrem Client, schicken Sie den
Paginator direkt über `json_response!`:

```rust
let page = User::query().paginate(20).await?;
Ok(suprnova::json_response!({
    "users": page.data,
    "pagination": {
        "current_page": page.current_page,
        "last_page": page.last_page,
        "per_page": page.per_page,
        "total": page.total,
    }
}))
```

Oder geben Sie einfach den ganzen Paginator weiter - die abgeleitete
`Serialize`-Impl emittiert die oben dokumentierte Form:

```rust
Ok(suprnova::json_response!(User::query().paginate(20).await?))
```

Die Felder sind öffentlich; formen Sie sie um, wie Ihr Vertrag es
verlangt.

## Routing über Connections

Paginierung respektiert dasselbe Multi-Connection-Routing, das der
Eloquent-Builder verwendet. Innerhalb einer `DB::transaction(...)`
laufen das COUNT und die Seiten-Query beide auf der Connection der
Transaktion - sie spalten sich nie über Connections auf, sodass das
Count niemals mit der Seite widerspricht, die es beschreibt. Eine
registrierte `__read_replica__` wird für Reads außerhalb einer
Transaktion automatisch verwendet. Um einen Paginator an eine
bestimmte benannte Connection zu pinnen, verwenden Sie die
`_on(connection, ...)`-Varianten auf der Facade `Pagination`, oder
`Builder::on("replica_b").paginate(20)` von der Eloquent-Seite.

Siehe [Eloquent - Multi-Connection-Routing](eloquent.md) für den
Routing-Vertrag.

## Wann was verwenden

Ein grober Entscheidungsbaum:

- **Numerische Seiten-UI ist Teil des Designs** → `paginate`. Sie
  brauchen `last_page`, um „Seite 3 von 17“ zu rendern, und die
  COUNT-Kosten sind bei Ihrer Tabellengröße in Ordnung.
- **Nur „Weiter“- / „Zurück“-Buttons, große Tabelle** →
  `simple_paginate`. Eine Query pro Seite; Sie geben `total` und
  `last_page` auf, aber der Seitenaufruf halbiert sich.
- **Infinite Scroll** → `cursor_paginate`. Bidirektionale Cursor
  bedeuten, dass der Client über Seite 1000 hinaus weiterscrollen
  kann, ohne dass OFFSET vorher tausende Zeilen scannt.
- **Ende eines stark frequentierten Append-Only-Feeds** →
  `cursor_paginate`. Keyset-Ordnung nach Primärschlüssel ist
  nebenläufigkeitssicher: Neue Zeilen landen jenseits des Cursors,
  nie innerhalb davon. OFFSET-basierte Paginierung überspringt
  Zeilen bei Inserts.
- **Einen `Select<E>` außerhalb eines Eloquent-Models bauen** →
  `Pagination::length_aware` / `Pagination::cursor`. Dieselben
  Trade-offs; die Facade ist das modell-lose Äquivalent.

Im Zweifel starten Sie mit `paginate`. Wechseln Sie zu
`simple_paginate`, wenn das `COUNT(*)` in Ihrem Slow-Query-Log
auftaucht. Wechseln Sie zu `cursor_paginate`, wenn tiefe Seiten
anfangen, die Request-Zeit zu dominieren, oder wenn die UI Infinite
Scroll ist.

## Wo jedes Teil lebt

| Teil | Datei |
|---|---|
| Facade `Pagination`, Trait `Paginated<T>` | `framework/src/pagination/mod.rs` |
| `LengthAwarePaginator<T>` | `framework/src/pagination/length_aware.rs` |
| `Paginator<T>` (simple) | `framework/src/pagination/simple.rs` |
| `CursorPaginator<T>`, `CursorDirection`, `encode_value`, `decode_value` | `framework/src/pagination/cursor.rs` |
| `IntoInertiaScroll`-Brücke | `framework/src/pagination/inertia.rs` |
| `Builder::paginate` / `simple_paginate` / `cursor_paginate` | `framework/src/eloquent/builder.rs` |
| `Inertia::paginate`, `InertiaResponse::paginate` | `framework/src/inertia/facade.rs`, `framework/src/inertia/response.rs` |
| `Resource::paginated`, `JsonApi::paginated` | `framework/src/resources/response.rs` |

## Nächste Schritte

- [Eloquent API](eloquent.md) - die Model-Schicht, die jeden
  Paginator treibt, den `Builder::paginate*` liefert
- [Query Builder](queries.md) - die modell-losen Queries, die sich
  mit `Pagination::length_aware` und `Pagination::cursor` kombinieren
- [Inertia Responses](frontend-inertia-responses.md) - wie
  Scroll-Props Paginatoren an Inertia-Seiten anhängen
- [JSON:API-Resources](eloquent-resources.md) - `Resource::paginated`,
  Links, Meta und der Trait `Paginated<T>`
- [Fehlermodell](error-model.md) - die Validierungsregel
  `FrameworkError::param` und die Herabstufung bei Cursor-Manipulation

# JSON:API Resources

Suprnova bringt eine JSON:API-Resource-Schicht für typisierte
REST-APIs mit. Markieren Sie eine `#[derive(Data)]`-Struktur mit
`#[json_resource("type")]`, und das Framework generiert eine
`IntoJsonResource`-Implementierung, die einzelne Envelopes,
Collections, paginierte Collections, Sparse Fieldsets
(`?fields[type]=...`), zusammengesetzte `included`-Dokumente und
mehrstufige `?include=a.b.c`-Ketten über denselben Codepfad
behandelt. Die beiden Facades - `Resource` und `JsonApi` - sind
derselbe Typ unter zwei Namen; verwenden Sie, was zu Ihrem
Hausstil passt.

## Eine Resource definieren

```rust
use suprnova::Data;

#[derive(Debug, Clone, Data)]
#[json_resource("users")]
pub struct UserResource {
    pub id: i64,
    pub email: String,

    // `input_only` lässt `password` auf der Form-Request-Seite
    // verfügbar, unterdrückt es aber in der API-Ausgabe.
    #[data(input_only)]
    pub password: String,

    // Markiert ein Feld als *Relation*: Es landet nie in
    // `attributes`, sondern erzeugt stattdessen ein
    // JSON:API-Relations-Objekt, und es ist `?include=`-fähig.
    // Der Feldtyp muss `IntoJsonResource` implementieren (direkt,
    // oder über `Vec<T>` / `Option<T>`).
    #[data(allow_include)]
    pub posts: Vec<PostResource>,
}
```

Das Schlüsselwort `id_field` benennt das Feld um, das die
JSON:API-`id` liefert:

```rust
#[derive(Data)]
#[json_resource("orders", id_field = "uuid")]
pub struct OrderResource {
    pub uuid: String,
    pub total_cents: i64,
}
```

## Responses rendern

Konstruieren Sie aus einem Handler eine ausstehende Response und
rufen Sie `.render().await` auf:

```rust
use suprnova::{LengthAwarePaginator, Resource};

#[handler]
async fn show_user(id: i64) -> Result<HttpResponse, FrameworkError> {
    let user: UserResource = User::find_or_fail(id).await?.into();
    Resource::single(user).render().await
}

#[handler]
async fn list_users() -> Result<HttpResponse, FrameworkError> {
    let users: Vec<UserResource> = User::all().await?.into_iter().map(Into::into).collect();
    Resource::collection(users).render().await
}

#[handler]
async fn paginate_users() -> Result<HttpResponse, FrameworkError> {
    // `paginate(per_page)` liest `?page=` automatisch aus der
    // aktuellen Anfrage.
    let page = User::query().paginate(10).await?;
    // Konvertiert den Model-Paginator Feld für Feld in einen
    // Resource-Paginator - `data` ist `pub`, die übrigen
    // Counts/Links werden übernommen.
    let page = LengthAwarePaginator::new(
        page.data.into_iter().map(UserResource::from).collect(),
        page.total,
        page.per_page,
        page.current_page,
    )
    .with_base_url("/api/users");
    Resource::paginated(page).render().await
}
```

`JsonApi::single` / `JsonApi::collection` / `JsonApi::paginated` sind
identische Alias-Einstiegspunkte, falls Sie die Laravel-Schreibweise
bevorzugen.

## Verkettbare Mutatoren

`JsonApiResponse` ist ein ausstehendes Objekt. Passen Sie die
Envelope an, bevor Sie `.render().await` aufrufen. Jeder Mutator ist
`self` → `Self`, sodass sie sich verketten lassen:

```rust
use suprnova::{Resource, JsonApiInfo};
use serde_json::json;

let info = JsonApiInfo::new()
    .with_version("1.1")
    .with_ext("https://jsonapi.org/ext/atomic")
    .with_meta("copyright", json!("2026 Acme Inc."));

Resource::single(user)
    .status(201)                                  // HTTP-Status überschreiben
    .with_meta("trace_id", json!("req-7"))        // Top-Level-Meta-KV
    .with_link("self", "/api/users/1")            // Top-Level-Link
    .with_jsonapi(info)                           // Top-Level-`jsonapi`
    .additional(json!({ "api_version": "2.0" }).as_object().unwrap().clone())
    .render()
    .await
```

| Mutator | Laravel-Analogon | Effekt |
|---|---|---|
| `.status(code)` | `ResourceResponse::calculateStatus` | Überschreibt den HTTP-Status. |
| `.created()` | `wasRecentlyCreated → 201` | Kurzform für `.status(201)`. |
| `.with_meta(k, v)` / `.meta(k, v)` | `with($request)` | Top-Level-`meta`-KV. |
| `.with_meta_map(m)` | Bulk `with($request)` | Führt eine Map in das Top-Level-`meta` zusammen. |
| `.with_link(rel, href)` / `.link(rel, href)` | `with($request)['links']` | Top-Level-`links`-KV. |
| `.with_link_value(rel, v)` | Link-Objekt-Form | Top-Level-Link als `{href, meta}`. |
| `.with_additional(k, v)` | `additional($data)` | Root-Level-Schlüssel neben `data`. |
| `.additional(map)` | `additional($data)` | Mehrere zusätzliche Schlüssel auf einmal. |
| `.with_jsonapi(info)` | `JsonApiResource::configure(...)` | Top-Level-Member `jsonapi`. |

Kanonische Member (`data`, `included`, `links`, `meta`, `jsonapi`,
`errors`) werden von `.additional(...)` nie überschrieben.

## Pro-Resource-`links` und -`meta`

Überschreiben Sie die Standards `IntoJsonResource::resource_links`
und `IntoJsonResource::resource_meta`, um Links / Metadaten an das
*Resource-Objekt* zu hängen, nicht an die Dokumentwurzel:

```rust
use suprnova::resources::IntoJsonResource;
use serde_json::{Map, Value};

impl IntoJsonResource for MyHandRolledPost {
    // ...

    fn resource_links(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("self".into(), Value::String(format!("/api/posts/{}", self.id)));
        m
    }

    fn resource_meta(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("kind".into(), Value::String("blog".into()));
        m
    }
}
```

Beide sind für makro-abgeleitete Resources standardmäßig eine leere
`Map`, sodass der JSON:API-Renderer die Schlüssel auslässt, wenn sie
nicht verwendet werden. Überschreiben Sie `resource_top_level_meta`,
um Pro-Resource-Metadaten in das Top-Level-`meta`-Member der
Envelope zu heben.

## Bedingte Attribute - `Maybe<T>` / `MissingValue<T>`

Verwenden Sie `Maybe`, um ein Feld anhand einer Laufzeitbedingung aus
dem gerenderten `attributes`-Objekt auszulassen. Das ist das
Suprnova-Analogon zu Laravels `MissingValue` und der Familie
`when()` / `whenLoaded()` / `unless()`.

```rust
use suprnova::{Maybe, MissingValue};

// Beide Namen zeigen auf denselben Typ.
let m1: Maybe<&str> = Maybe::present("email@example.com");
let m2: MissingValue<&str> = MissingValue::missing();
let m3 = Maybe::when(user.is_verified, &user.verified_at);
let m4 = Maybe::unless(user.is_admin, &user.public_handle);
let m5 = Maybe::when_with(expensive_check(), || compute_value()); // lazy
```

Deklarieren Sie bei makro-abgeleiteten Strukturen ein Feld als
`Maybe<T>`, und der Renderer lässt es bei `Missing` automatisch weg.
Verwenden Sie für handgerolltes `resource_attributes` den Helfer
`insert_maybe(map, key, maybe)`:

```rust
use suprnova::resources::{insert_maybe, Maybe};

fn resource_attributes(&self, _fs: Option<&[&str]>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    insert_maybe(&mut map, "email", Maybe::present(&self.email));
    insert_maybe(
        &mut map,
        "phone",
        if self.show_phone { Maybe::present(&self.phone) } else { Maybe::missing() },
    );
    serde_json::Value::Object(map)
}
```

Der Renderer ruft außerdem `strip_missing_values(&mut value)` über
das gesamte Attributes-Objekt auf, sodass `Maybe::Missing`-Werte, die
in beliebigen serde-abgeleiteten Strukturen verschachtelt sind,
rekursiv entfernt werden - nützlich, wenn ein tief verschachtelter
Transformer Subfelder auslassen will.

## Sparse Fieldsets

Die `IncludeMiddleware` des Frameworks parst Query-Parameter im Stil
von `?fields[type]=email,name` und bindet sie an ein Task-Local. Das
makro-generierte `resource_attributes` befragt das Fieldset und
gibt nur die angeforderten Attribute aus. Auf Handler-Seite ist
keine Arbeit nötig - installieren Sie die Middleware, und die
Resource-Schicht berücksichtigt sie automatisch.

```rust
// Anfrage: GET /api/users/7?fields[users]=email
// Antwort: { "data": { "type": "users", "id": "7", "attributes": { "email": "alice@example.com" } } }
```

## Zusammengesetzte Dokumente - `?include=`-Ketten

Deklarieren Sie Relationsfelder mit `#[data(allow_include)]`. Das
Framework baut aus `?include=author.posts.tags,comments` einen
`IncludeTree`, durchläuft jeden Knoten und schiebt vollständig
aufgelöste Resource-Objekte in `included`. Die Deduplizierung läuft
beim Push über `IncludedSink`, geschlüsselt nach `(type, id)` gemäß
§8 der JSON:API-Spec - sodass eine 1.000-Element-Collection, bei der
jedes Element denselben Autor teilt, den Autor genau einmal auflöst.
Spitzenspeicher und -CPU bleiben proportional zu den
unterschiedlichen included-Resources, nicht zum Relations-Fan-in.

```rust
#[derive(Data)]
#[json_resource("posts")]
pub struct PostResource {
    pub id: i64,
    pub title: String,

    #[data(allow_include)]
    pub author: Option<AuthorResource>,

    #[data(allow_include)]
    pub tags: Vec<TagResource>,
}
```

Eine Anfrage, die einen Include-Pfad nennt, der nicht auf der
Allowlist dieser Resource steht, erhält eine JSON:API-400-Errors-
Envelope.

### Warum Suprnova abweicht

Zwei sichtbare Abweichungen von Laravels `JsonApiResource`:

1. **Striktes Default-Deny für `?include=`.** Laravels Resource-Schicht
   ignoriert Include-Pfade, die sich nicht auflösen lassen,
   stillschweigend. Suprnova weist sie mit einem `400 Bad Request`
   samt JSON:API-Errors-Envelope zurück. Die Default-Deny-Haltung aus
   §5.2.2 der Spec ist der Vertrag, gegen den Clients programmieren
   können; stillschweigendes Ignorieren verbirgt Client-Bugs und
   bricht die Integrität zusammengesetzter Dokumente.

2. **Explizites `.status(code)` / `.created()` statt Auto-201.**
   Laravel setzt `201` automatisch aus `wasRecentlyCreated` auf dem
   zugrundeliegenden Eloquent-Modell. Suprnova entkoppelt das
   Resource-DTO von jedem spezifischen Persistenz-Lifecycle, sodass
   der Status am Response-Objekt selbst gesetzt wird - `.created()`,
   wenn Sie es so meinen, `.status(204)`, wenn die Response leer ist,
   und so weiter. Ein einzelner Mutator bleibt unter jedem Ablauf
   ehrlich.

## Paginierung

`Resource::paginated(p)` funktioniert mit jedem Paginator, der den
Trait `Paginated<T>` implementiert - sowohl `LengthAwarePaginator<T>`
als auch `CursorPaginator<T>` aus `suprnova::pagination` bringen
diese Implementierung mit. Der Renderer hängt automatisch
`links.{self,first,prev,next,last}` und einen `meta.pagination`-Block
an.

```rust
use suprnova::{LengthAwarePaginator, Resource};

let page = LengthAwarePaginator::new(items, total, per_page, current_page)
    .with_base_url("/api/users");
Resource::paginated(page).render().await
```

## Error-Envelopes

Jeder `FrameworkError` weiß, wie er sich selbst als
JSON:API-`{"errors": [...]}`-Envelope über
`into_json_api_response()` rendert. Der Helfer ist öffentlich, weil
`FrameworkError` einen Statuscode, einen Feldnamen-Source-Pointer
(für `ValidationError`) und ein Request-ID-Korrelationstoken unter
`meta.request_id` trägt. 5xx-Responses werden bereinigt: Die rohe
Meldung erreicht den Client nie, außer `APP_DEBUG=true` ist in der
aktiven Umgebung gesetzt - dann erscheint sie unter
`meta.debug_message`.

```rust
let response = FrameworkError::validation("email", "email is invalid")
    .into_json_api_response();
// {
//   "errors": [{
//     "status": "422",
//     "title": "Validation failed",
//     "detail": "email is invalid",
//     "source": { "pointer": "/data/attributes/email" },
//     "meta": { "request_id": "..." }
//   }]
// }
```

## Oberflächen-Übersicht

| Suprnova-Oberfläche | Laravel-13-Äquivalent |
|---|---|
| `Resource`- / `JsonApi`-Facades | `JsonResource::make`, `JsonApiResource` |
| `JsonApiResponse` | `ResourceResponse`, `JsonApiResource::toResponse` |
| `JsonApiBuilder` | (interner Builder für `ResourceResponse`) |
| `IntoJsonResource`-Trait | `JsonResource::toArray`, `toAttributes`, `toRelationships`, `toLinks`, `toMeta`, `with` |
| `RelationshipValue` / `ResourceIdentifier` | Array-Form innerhalb von `toRelationships` |
| `IncludeTree` | geparstes `?include=` aus `JsonApiRequest` |
| `RequestFieldsetSet` | geparstes `?fields[type]=` aus `JsonApiRequest` |
| `Maybe<T>` / `MissingValue<T>` | `MissingValue` + `whenLoaded` / `when` / `unless` |
| `JsonApiInfo` | `JsonApiResource::$jsonApiInformation` |
| `JsonApiResponse::status(code)` / `.created()` | `ResourceResponse::calculateStatus` |
| `JsonApiResponse::additional(map)` / `.with_additional(k, v)` | `JsonResource::additional($data)` |
| `JsonApiResponse::with_meta(k, v)` / `.meta(k, v)` | `JsonResource::with($request)['meta']` |
| `JsonApiResponse::with_link(rel, href)` / `.link(rel, href)` | `JsonResource::with($request)['links']` |
| `JsonApiResponse::with_jsonapi(info)` | `JsonApiResource::configure(...)` |
| `current_fieldset()` / `scope_fieldset(...)` | Task-Local-Fieldset, gesetzt von `IncludeMiddleware` |
| `IncludeResolutionError` → 400-Envelope | Strict-Mode-`?include=`-Parser |

Top-Level-Re-Exports unter `suprnova::`: `Resource`, `JsonApi`,
`JsonApiResponse`, `JsonApiBuilder`, `JsonApiInfo`, `IncludedSink`,
`IntoJsonResource`, `RelationshipValue`, `ResourceIdentifier`,
`IncludeTree`, `RequestFieldsetSet`, `Maybe`, `MissingValue`,
`insert_maybe`, `strip_missing_values`, `AsRelationshipValue`,
`PushIncluded`, `IncludeResolutionError`, `current_fieldset`,
`scope_fieldset`.

## Nächste Schritte

- [Eloquent Serialization](eloquent-serialization.md) -
  `#[derive(Data)]`, versteckte/sichtbare Felder, das
  `toArray`-Äquivalent, das Resource-Attribute füttert
- [Eloquent Relationships](eloquent-relationships.md) - was
  `#[data(allow_include)]` konsumiert; die typisierten
  Relations-Arten hinter zusammengesetzten Dokumenten
- [Paginierung](pagination.md) - `LengthAwarePaginator`,
  `CursorPaginator`, und der Trait `Paginated<T>`, den
  `Resource::paginated` konsumiert
- [Datenobjekte](data.md) - das `#[derive(Data)]`-Makro, das sich
  Inertia teilt, die `?include=`/`?fields[type]=`-Middleware, und
  `Maybe<T>`-Muster
- [Fehlermodell](error-model.md) - wie
  `FrameworkError::into_json_api_response` in den
  Konvertierungsvertrag passt

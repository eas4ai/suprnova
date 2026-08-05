# Datenobjekte

Mit Suprnovas `#[derive(Data)]` beschreiben Sie eine eingehende
Request-Form, eine ausgehende Response-Form und einen TypeScript-Export
in **einer Struktur**.

## Schnellstart

```rust
use suprnova::Data;
use suprnova::data::Field;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UserDto {
    pub id: i64,

    #[validate(email)]
    pub email: String,

    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub display_handle: String,

    pub bio: Field<String>,
}
```

`#[derive(Data)]` generiert:
- `Serialize` (überspringt `#[data(input_only)]`-Felder)
- `Deserialize` (weist `#[data(output_only)]`-Felder im Payload zurück, setzt sie auf `T::default()`)
- `FormRequest` mit `authorize: true` als Standard - Handler können den Typ direkt als Extraktor nehmen
- `IntoInertiaData` (den `Inertia::data(component, dto)`-Dispatch-Pfad)
- Eine `inventory::submit!`-Registrierung für jedes `#[data(allow_include)]`-Feld

Fügen Sie `#[derive(Validate)]` separat hinzu, damit `#[validate(...)]`-Attribute am Feld-Aufrufort sichtbar bleiben.

## Feld-Attribute

| Attribut | Effekt |
|---|---|
| `#[data(input_only)]` | Bei Deserialize akzeptiert, bei Serialize ausgelassen |
| `#[data(output_only)]` | Bei Deserialize zurückgewiesen (422), bei Serialize eingeschlossen |
| `#[data(allow_include)]` | Feld ist `?include=`-fähig. **Default-Deny**: Jede `?include=foo`-Anfrage, bei der `foo` nicht auf der Allowlist steht, liefert 400 |
| `#[data(lazy)]` | Feld ist ein `Prop`, aufgelöst gegen das Include-Set der Anfrage; registriert sich automatisch als `allow_include` |
| `#[data(lazy(inertia))]` | Wie `lazy`, markiert für Inertias Partial-Reload-Protokoll |
| `#[data(lazy(deferred))]` | Markiert für Inertias Deferred-Props-Protokoll |
| `#[data(lazy(closure))]` | Wird beim ersten Besuch immer aufgelöst; lazy bei Partial Reloads |
| `#[data(lazy(when_loaded))]` | Nur aufgelöst, wenn die Quell-Entität die Relation vorgeladen hat |
| `#[data(from_route_param)]` | Feldwert kommt aus einer Pfad-Erfassung (z. B. `/users/{id}`). Standard-Schlüssel = Feldname; übergeben Sie `#[data(from_route_param("id"))]`, um das zu überschreiben |

## Struktur-Attribute

| Attribut | Effekt |
|---|---|
| `#[data(auto_lazy)]` | Jedes `Prop`-typisierte Feld ist implizit `#[data(lazy)]` |
| `#[data(authorize = "path::to::fn")]` | Leitet das generierte `FormRequest::authorize` an eine freie Funktion mit der Signatur `fn(req: &Request) -> bool`. Der Body-Parser, der Validator, die Precognition-Unterstützung und die Routenparameter-Injektion kommen weiterhin aus dem Derive |
| `#[data(allow_unknown_fields)]` | Akzeptiert Payload-Schlüssel, die zu keinem Struktur-Feld passen. Der Standard ist **strikt**: Ein nicht erkannter Schlüssel lässt das Deserialize mit `serde::de::Error::unknown_field(..)` fehlschlagen und erscheint als 422 über `FormRequest`. Aktivieren Sie permissiv nur für Response-DTOs, die vorwärtskompatible Payloads von Drittanbietern lesen |

Das frühere `#[data(custom_authorize)]`-Flag - das die gesamte
`FormRequest`-Implementierung unterdrückte und Sie zwang,
Body-Parsing, Validierung und Precognition von Hand nachzubauen - ist
verschwunden. Das Makro gibt einen Migrationsfehler aus, wenn Sie es
zu verwenden versuchen. Verwenden Sie stattdessen
`#[data(authorize = "fn")]`.

## `Field<T>` - Absent / Null / Value

Für PATCH-Endpunkte, bei denen „abwesend vom Payload“ von „explizit
null“ unterschieden werden muss:

```rust
use suprnova::data::Field;

match dto.bio {
    Field::Absent  => { /* diese Spalte nicht anfassen */ },
    Field::Null    => { /* die Spalte leeren */ },
    Field::Value(text) => { /* auf text setzen */ },
}
```

`Field::Absent` (Standard) übersteht einen Round-Trip zu
aus-JSON-ausgelassen, wenn es am Aufrufort mit `#[serde(default,
skip_serializing_if = "Field::is_absent")]` gepaart wird. Ohne
`skip_serializing_if` serialisiert `Absent` zu JSON `null`.

Für dreiwertige DB-Upserts: `dto.bio.into_option_or_null() ->
Option<Option<T>>` bildet `Absent → None`, `Null → Some(None)`,
`Value(v) → Some(Some(v))` ab. Verwenden Sie das, wenn „nicht
anfassen“ und „auf NULL setzen“ nachgelagert unterscheidbar sein
müssen.

> **Vorbehalt:** `Field<Option<T>>` ist verlustbehaftet - `Value(None)`
> und `Null` serialisieren beide als JSON `null` und deserialisieren
> zurück zu `Null`. Bevorzugen Sie für nullable innere Typen ein
> flaches `Field<T>` und lassen Sie `Null` das „Leeren“-Signal tragen.

## `?include=`-Query-String

`IncludeMiddleware` parst den Query-String der Anfrage in ein
Per-Request-`RequestIncludeSet`:

- `?include=foo,bar` - löst die Lazy-Felder `foo` und `bar` auf.
- `?include[]=foo&include[]=bar` - Array-Form, gleiches Ergebnis.
- `?exclude=`, `?only=`, `?except=` - Paritätsäquivalent zur Laravel-Data-API.

Zusammenspiel mit `X-Inertia-Partial-Data` (Inertias
Partial-Reload-Header): Das Include-Set + die Pro-DTO-Allowlist laufen
für Owner-getaggte Lazy-Felder **zuerst**, sodass eine Anfrage nach
einem nicht erlaubten Feld ein 400 zurückgibt, selbst wenn Partial-Data
es herausgefiltert hätte. Partial-Data wird **danach** als
abschließender „only“-Filter auf die aufgelösten Props angewendet.

Registrieren Sie `IncludeMiddleware` global - typischerweise zwischen
Session und Autorisierung im Middleware-Stack:

```text
SessionMiddleware → IncludeMiddleware → AuthMiddleware → Handler
```

### Programmatisches include/exclude/only/except

`RequestIncludeSet` spiegelt Laravel-Datas
`IncludeableData`-Vertrag mit verkettbaren Buildern. Handler, Tests
und Middleware können ein Set konstruieren oder überschreiben, ohne
direkt an den öffentlichen Feldern zu stochern:

```rust
use suprnova::data::RequestIncludeSet;

let set = RequestIncludeSet::default()
    .include(["author", "comments"])
    .exclude(["password"])
    .only(["id", "name"])
    .except(["secret"]);

assert!(set.is_visible("name"));   // in `only`, nicht in `except`
assert!(!set.is_visible("secret"));// `except` gewinnt immer
assert!(set.includes("author"));   // Anfrage nach der `author`-Relation
```

| Methode | Effekt | Laravel-Äquivalent |
|---|---|---|
| `.include(fields)` | an die Include-Liste anhängen (aufzulösende Lazy-Felder) | `Data::include(...$fields)` |
| `.exclude(fields)` | an die Exclude-Liste anhängen (zu verwerfende Felder) | `Data::exclude(...$fields)` |
| `.only(fields)` | die `only`-Allowlist initialisieren oder erweitern | `Data::only(...$fields)` |
| `.except(fields)` | an die Except-Liste anhängen (immer verwerfen) | `Data::except(...$fields)` |
| `.include_when(cond, fields)` | nur anhängen, wenn `cond == true` | `Data::includeWhen($field, $condition)` |
| `.exclude_when(cond, fields)` | nur anhängen, wenn `cond == true` | `Data::excludeWhen($field, $condition)` |
| `.only_when(cond, fields)` | `only` nur erweitern, wenn `cond == true` | `Data::onlyWhen($field, $condition)` |
| `.except_when(cond, fields)` | nur anhängen, wenn `cond == true` | `Data::exceptWhen($field, $condition)` |
| `.merge(other)` | zwei Sets vereinigen (In-Place, geschichtete Overrides) | manuelles `array_merge` in PHP |
| `.includes(field)` | `field` (oder `field.path`) in der Include-Liste? | `relationLoaded()`-Analogon |
| `.is_excluded(field)` | `field` in der Exclude-Liste? | liest den Exclude-Teil |
| `.is_excepted(field)` | `field` in der Except-Liste? | liest den Except-Teil |
| `.is_only_listed(field)` | `field` von `only` erlaubt (oder `only` nicht gesetzt)? | liest den Only-Teil |
| `.is_visible(field)` | vollständige Laravel-Auflösungsreihenfolge: except → exclude → only | `resolveResource`-Entscheidung |

Builder nehmen jedes `IntoIterator<Item = impl Into<String>>`, also
funktionieren Arrays, Vecs und Slices aus `&str`/`String` alle. Strings
werden getrimmt; leere Einträge werden verworfen (passend zu
`from_query`).

Dot-Pfade in jeder Liste matchen das Wurzelsegment, wenn nach bloßem
Namen geprüft wird - `include=["author.posts"]` meldet
`set.includes("author") == true`, passend zu Laravel-Datas
Pfad-Auflösung. Das verschachtelte `posts`-Segment wird von
`IncludeTree::from_include_set` für JSON:API-Compound-Dokumente
konsumiert.

### Handler-seitiges Override: `with_include_overrides`

Um programmatische Overrides über das zu schichten, was der
Query-String der Anfrage bereits deklariert hat (ohne das Set der
Anfrage zu verlieren), verwenden Sie `with_include_overrides`:

```rust
use suprnova::data::with_include_overrides;

async fn show_album(req: Request, user: User) -> Response {
    with_include_overrides(
        |set| set
            .include_when(user.is_admin(), ["audit_log"])
            .exclude_when(!user.is_admin(), ["price_cost"]),
        async move {
            // Innerhalb dieses Scopes sehen der Lazy-Prop-Resolver und der
            // JSON:API-Include-Resolver das zusammengeführte Set.
            Inertia::data("Album/Show", album_dto).into_response()
        },
    ).await
}
```

Die Closure läuft gegen einen Klon des aktuell gebundenen Sets (oder
den leeren Standard, falls keine Middleware eines gebunden hat).
Nachdem das Future abgeschlossen ist, wird das ursprüngliche Set
wiederhergestellt - das ist ein Scope-gebundenes Override, keine
Mutation.

Bevorzugen Sie für Tests `scope_include_set(set, future)`, um ein
frisches Set zu installieren, ohne bestehenden Zustand zu erben.

## Generische Strukturen

```rust
use serde::{Serialize, Deserialize};

#[derive(suprnova::Data)]
pub struct Paginated<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    pub items: Vec<T>,
    pub total: usize,

    #[data(allow_include)]
    pub meta: Option<serde_json::Value>,
}
```

Der TypeScript-Extraktor gibt `export interface Paginated<T>` aus,
sodass Frontend-Code das Generic über Instanziierungen hinweg
wiederverwenden kann.

Die `?include=`-Allowlist ist auf den vollqualifizierten Typpfad
geschlüsselt (`concat!(module_path!(), "::", stringify!(Paginated))`),
nicht auf Typparameter-Instanziierungen. `Paginated<UserDto>` und
`Paginated<ArticleDto>`, im selben Modul deklariert, teilen sich eine
Allowlist - `allow_include` benennt ein Feld, und Feldnamen hängen
nicht von Typparametern ab. Zwei verschiedene, `Paginated` genannte
DTOs in verschiedenen Modulen bekommen jeweils ihre eigene Allowlist;
ihre Schlüssel kollidieren nicht.

Hinweis: `FormRequest` wird für generische Strukturen unterdrückt, weil
seine Trait-Schranken (`DeserializeOwned + Validate + Send`) ohne
Kenntnis konkreter Typparameter nicht verifiziert werden können.
Stellen Sie Ihre eigene Implementierung bereit, wenn Sie eine
generische Data-Struktur aus einer Anfrage extrahieren müssen.

## Feldinjektion über Routenparameter

```rust
use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UpdateUser {
    #[data(from_route_param("id"))]
    pub id: i64,

    #[validate(length(min = 1))]
    pub name: String,
}
```

Für `PATCH /users/{id}` mit dem Body `{"name": "Ada"}` wird die aus der
Route erfasste `id` in den validierten Payload eingemischt. **Der Pfad
gewinnt immer über einen im Body gelieferten Wert** (verhindert IDOR
über Body-Tampering).

Bloßes `#[data(from_route_param)]` fällt auf den Feldnamen zurück. Das
Makro klassifiziert das letzte Pfadsegment des Feldtyps zur
Compile-Zeit und dispatcht zu einem passenden Parser. Nur die exakten,
unten aufgeführten Namen werden erkannt; alles andere (einschließlich
`i8`/`i16`/`isize`, `Uuid`, `DateTime`, benutzerdefinierte Newtypes)
fällt durch zu `pass_string` und lässt das eigene `Deserialize` des
Felds die Arbeit erledigen.

| Feldtyp | Parser |
|---|---|
| `i64` | `parse_i64` |
| `u64` | `parse_u64` |
| `i32` | `parse_i32` |
| `u32` | `parse_u32` |
| `i128` | `parse_i128` (validiert und reicht dann den rohen String durch; das `Deserialize` des Felds parst) |
| `u128` | `parse_u128` (dasselbe String-Durchreich-Muster) |
| `f64` | `parse_f64` (weist nicht-endliche Werte zurück) |
| `f32` | `parse_f32` (weist nicht-endliche Werte zurück) |
| `bool` | `parse_bool` (akzeptiert nur `"true"` / `"false"`) |
| Alles andere | `pass_string` - roher String, dem eigenen `Deserialize` des Felds übergeben |
| `Option<T>` oder `Field<T>` von einem der obigen | Derselbe Parser wie `T`; ein fehlender Routenparameter lässt das Feld abwesend |

## Lazy Props

```rust
use suprnova::Data;
use suprnova::inertia::Prop;

#[derive(Data)]
#[data(auto_lazy)]
pub struct AlbumDto {
    pub id: i64,
    pub songs: Prop,    // automatisch registriert als ?include=songs
    pub artist: Prop,   // automatisch registriert als ?include=artist
}
```

Explizite Variante pro Feld:

```rust
#[derive(Data)]
pub struct AlbumDto {
    pub id: i64,

    #[data(lazy(inertia))]
    pub songs: Prop,

    #[data(lazy(deferred))]
    pub lyrics: Prop,

    #[data(lazy(closure))]
    pub artist: Prop,
}
```

Verwenden Sie `Inertia::data(component, dto)` zum Rendern - das
Derive generiert eine `IntoInertiaData`-Implementierung, die das
Include-Set und die Allowlist konsultiert:

```rust
return Inertia::data("Album/Show", album_dto);
```

Hinweis: Strukturen mit Lazy-Feldern unterdrücken `Serialize`,
`Deserialize` und `FormRequest`, weil `Prop` sie nicht implementiert.
Wenn ein einzelner Endpunkt sowohl eingehendes Parsen als auch Lazy
ausgehend braucht, verwenden Sie zwei DTOs: eines eingehend
(`#[derive(Data, Validate)]` schlicht) und eines ausgehend
(`#[derive(Data)]` mit Lazy-Feldern).

## `when_loaded!` - bedingtes Lazy bei geladener Relation

Spiegelt Laravel-Datas `#[AutoWhenLoadedLazy]`. Die
`From<Entity>`-Implementierung des Nutzers entscheidet, ob die
Relation vorgeladen wurde:

```rust
use suprnova::data::{when_loaded, IsRelationLoaded};

impl From<&AlbumEntity> for AlbumDto {
    fn from(album: &AlbumEntity) -> Self {
        Self {
            id: album.id,
            songs: when_loaded!(album, "songs", || async {
                serde_json::json!(album.songs_relation()
                    .iter()
                    .map(SongDto::from)
                    .collect::<Vec<_>>())
            }),
            artist: Prop::eager(serde_json::json!(album.artist_name())),
            lyrics: Prop::lazy(|| async { /* ... */ }),
        }
    }
}
```

Hat die Entität die benannte Relation nicht vorgeladen (laut
`IsRelationLoaded::is_relation_loaded`), gibt `when_loaded!`
`Prop::EagerNone` zurück, und das Feld fehlt in der Response.

SeaORM-Entitäten brauchen eine eigene `IsRelationLoaded`-Implementierung,
die ihren geladene-Relationen-Zustand konsultiert - es gibt keine vom
Framework mitgelieferte Blanket-Implementierung, weil SeaORMs
`ModelTrait` keinen Pro-Instanz-Zustand darüber trägt, ob eine Relation
geladen ist (geladene Relationen leben auf Query-Ergebnissen, nicht
auf der Modell-Struktur selbst).

## TypeScript-Export

`suprnova generate-types` gibt TypeScript-Definitionen für jede
`#[derive(Data)]`-Struktur (und veraltete `#[derive(InertiaProps)]`-
Struktur) aus. Verhalten:

- `Field<T>` → `field?: T | null`
- `Prop` → `field?: T` (die Lazy-kann-abwesend-sein-Semantik; das `?` trägt sie, der Typ selbst ist schlicht)
- `#[data(input_only)]` → aus dem Output-Typ ausgeschlossen
- `#[data(output_only)]` → aus dem Input-Typ ausgeschlossen
- Generische Struktur → generisches TypeScript-Interface (`export interface Paginated<T>`)
- Hat IRGENDEIN Feld `input_only` / `output_only` / `lazy`, werden zwei Interfaces ausgegeben: `<Name>` (Output) und `<Name>Input` (Input)

Generierte Typen lassen niemals reine Rust-Typen durchsickern
(`Prop<...>` erscheint nicht im ausgegebenen `.d.ts`).

## Scaffolding

```bash
suprnova make:inertia UserDto --data
```

Gibt ein `#[derive(Data, Validate)]`-Skelett aus statt der veralteten
`#[derive(InertiaProps)]`-Vorlage.

## Nächste Schritte

- [Validierung](validation.md) - `#[derive(Validate)]`, asynchrone Validatoren, und wie `FormRequest` sie aufruft
- [Anfragen](requests.md) - die Request-Extraktor-Oberfläche, in die `FormRequest` sich einhängt
- [Inertia Responses](frontend-inertia-responses.md) - der `Inertia::data`-Pfad und wie Lazy Props partial-reload-fähig werden
- [Eloquent Resources](eloquent-resources.md) - `#[derive(Resource)]` für JSON:API-Ausgaben (Geschwister von `Data` für rein serialisierende Payloads)
- [Fehlermodell](error-model.md) - wie die `unknown_field`-Zurückweisung zu einem 422 wird und wie `FormRequest`-Fehlschläge als `ValidationErrors` zurückreisen

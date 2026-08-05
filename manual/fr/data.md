# Objets de données

Le `#[derive(Data)]` de Suprnova vous permet de décrire une forme de
requête entrante, une forme de réponse sortante, et un export
TypeScript dans **une seule struct**.

## Démarrage rapide

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

`#[derive(Data)]` génère :
- `Serialize` (en sautant les champs `#[data(input_only)]`)
- `Deserialize` (en rejetant les champs `#[data(output_only)]` dans le
  payload, en les ramenant par défaut à `T::default()`)
- `FormRequest` avec `authorize: true` par défaut - les handlers peuvent
  prendre le type directement comme extracteur
- `IntoInertiaData` (le chemin de dispatch
  `Inertia::data(component, dto)`)
- Un enregistrement `inventory::submit!` pour tout champ
  `#[data(allow_include)]`

Ajoutez `#[derive(Validate)]` séparément pour que les attributs
`#[validate(...)]` restent visibles au site d'appel du champ.

## Attributs de champ

| Attribut | Effet |
|---|---|
| `#[data(input_only)]` | Accepté au Deserialize, omis du Serialize |
| `#[data(output_only)]` | Rejeté au Deserialize (422), inclus dans le Serialize |
| `#[data(allow_include)]` | Le champ est éligible à `?include=`. **Refus par défaut** : toute requête `?include=foo` où `foo` n'est pas sur l'allowlist retourne 400 |
| `#[data(lazy)]` | Le champ est un `Prop` résolu contre l'include-set de la requête ; s'auto-enregistre comme `allow_include` |
| `#[data(lazy(inertia))]` | Identique à `lazy`, tagué pour le protocole de rechargement partiel d'Inertia |
| `#[data(lazy(deferred))]` | Tagué pour le protocole deferred-props d'Inertia |
| `#[data(lazy(closure))]` | Toujours résolu à la visite initiale ; lazy sur les rechargements partiels |
| `#[data(lazy(when_loaded))]` | Résolu seulement si l'entité source a préchargé la relation |
| `#[data(from_route_param)]` | La valeur du champ provient d'une capture de chemin (p. ex. `/users/{id}`). Clé par défaut = nom du champ ; passez `#[data(from_route_param("id"))]` pour redéfinir |

## Attributs de struct

| Attribut | Effet |
|---|---|
| `#[data(auto_lazy)]` | Chaque champ de type `Prop` est implicitement `#[data(lazy)]` |
| `#[data(authorize = "path::to::fn")]` | Route le `FormRequest::authorize` généré vers une fonction libre de signature `fn(req: &Request) -> bool`. Le parseur de corps, le validateur, le support Precognition, et l'injection de paramètre de route viennent toujours du derive |
| `#[data(allow_unknown_fields)]` | Accepte les clés du payload qui ne correspondent à aucun champ de la struct. Le défaut est **strict** : une clé non reconnue fait échouer le deserialize avec `serde::de::Error::unknown_field(..)` et remonte en 422 via `FormRequest`. À n'activer en mode permissif que pour les DTO de réponse qui lisent des payloads tiers compatibles avec les évolutions futures |

L'ancien flag `#[data(custom_authorize)]` - qui supprimait tout l'impl
`FormRequest` et vous forçait à réimplémenter à la main l'analyse du
corps, la validation, et Precognition - a disparu. La macro émet une
erreur de migration si vous essayez de l'utiliser. Utilisez
`#[data(authorize = "fn")]` à la place.

## `Field<T>` - Absent / Null / Value

Pour les endpoints PATCH où « absent du payload » doit être distingué
de « null explicite » :

```rust
use suprnova::data::Field;

match dto.bio {
    Field::Absent  => { /* ne pas toucher à cette colonne */ },
    Field::Null    => { /* vider la colonne */ },
    Field::Value(text) => { /* définir sur text */ },
}
```

`Field::Absent` (par défaut) fait l'aller-retour vers omis-du-JSON
quand il est associé à
`#[serde(default, skip_serializing_if = "Field::is_absent")]` au site
d'appel. Sans `skip_serializing_if`, `Absent` se sérialise en `null`
JSON.

Pour les upserts BD à trois voies :
`dto.bio.into_option_or_null() -> Option<Option<T>>` fait correspondre
`Absent → None`, `Null → Some(None)`, `Value(v) → Some(Some(v))`.
Utilisez ceci quand « ne pas toucher » et « mettre à NULL » doivent
rester distincts en aval.

> **Mise en garde :** `Field<Option<T>>` est avec perte -
> `Value(None)` et `Null` se sérialisent tous deux en `null` JSON et se
> désérialisent tous deux en `Null`. Pour les types internes nullable,
> préférez un `Field<T>` plat et laissez `Null` porter le signal
> « l'effacer ».

## Chaîne de requête `?include=`

L'`IncludeMiddleware` analyse la chaîne de requête en une
`RequestIncludeSet` propre à chaque requête :

- `?include=foo,bar` - résout les champs lazy `foo` et `bar`.
- `?include[]=foo&include[]=bar` - forme tableau, même résultat.
- `?exclude=`, `?only=`, `?except=` - parité avec l'API Laravel-Data.

Composition avec `X-Inertia-Partial-Data` (l'en-tête de rechargement
partiel d'Inertia) : l'include-set + l'allowlist par DTO s'exécute **en
premier** pour les champs lazy tagués par leur propriétaire, si bien qu'une requête pour
un champ non autorisé retourne 400 même si le partial-data l'aurait
filtré. Le partial-data est appliqué **ensuite** comme un filtre
« only » final sur les props résolues.

Enregistrez `IncludeMiddleware` globalement - typiquement entre la
session et l'autorisation dans la pile de middleware :

```text
SessionMiddleware → IncludeMiddleware → AuthMiddleware → handlers
```

### `include`/`exclude`/`only`/`except` programmatiques

`RequestIncludeSet` reflète le contrat `IncludeableData` de
Laravel-Data avec des builders chaînables. Les handlers, les tests, et
le middleware peuvent construire ou redéfinir un set sans toucher
directement aux champs publics :

```rust
use suprnova::data::RequestIncludeSet;

let set = RequestIncludeSet::default()
    .include(["author", "comments"])
    .exclude(["password"])
    .only(["id", "name"])
    .except(["secret"]);

assert!(set.is_visible("name"));   // sur `only`, pas dans `except`
assert!(!set.is_visible("secret"));// `except` gagne toujours
assert!(set.includes("author"));   // requête pour la relation `author`
```

| Méthode | Effet | Équivalent Laravel |
|---|---|---|
| `.include(fields)` | ajoute à la liste include (champs lazy à résoudre) | `Data::include(...$fields)` |
| `.exclude(fields)` | ajoute à la liste exclude (champs à retirer) | `Data::exclude(...$fields)` |
| `.only(fields)` | initialise ou étend l'allowlist `only` | `Data::only(...$fields)` |
| `.except(fields)` | ajoute à la liste except (toujours retiré) | `Data::except(...$fields)` |
| `.include_when(cond, fields)` | ajoute seulement quand `cond == true` | `Data::includeWhen($field, $condition)` |
| `.exclude_when(cond, fields)` | ajoute seulement quand `cond == true` | `Data::excludeWhen($field, $condition)` |
| `.only_when(cond, fields)` | étend `only` seulement quand `cond == true` | `Data::onlyWhen($field, $condition)` |
| `.except_when(cond, fields)` | ajoute seulement quand `cond == true` | `Data::exceptWhen($field, $condition)` |
| `.merge(other)` | union de deux sets (redéfinitions en couches, sur place) | `array_merge` manuel en PHP |
| `.includes(field)` | `field` (ou `field.path`) est-il dans la liste include ? | analogue de `relationLoaded()` |
| `.is_excluded(field)` | `field` est-il dans la liste exclude ? | lit la partielle exclude |
| `.is_excepted(field)` | `field` est-il dans la liste except ? | lit la partielle except |
| `.is_only_listed(field)` | `field` est-il autorisé par `only` (ou `only` non défini) ? | lit la partielle only |
| `.is_visible(field)` | ordre de résolution Laravel complet : except → exclude → only | décision `resolveResource` |

Les builders acceptent n'importe quel
`IntoIterator<Item = impl Into<String>>`, donc les arrays, vecs, et
slices de `&str`/`String` fonctionnent tous. Les chaînes sont élaguées ;
les entrées vides sont abandonnées (à l'image de `from_query`).

Les chemins à points dans n'importe quelle liste correspondent au
segment racine quand ils sont sondés par nom nu -
`include=["author.posts"]` fait que `set.includes("author") == true`,
à l'image de la résolution de chemin de Laravel-Data. Le segment
imbriqué `posts` est consommé par `IncludeTree::from_include_set` pour
les documents composés JSON:API.

### Redéfinition côté handler : `with_include_overrides`

Pour superposer des redéfinitions programmatiques par-dessus ce que la
chaîne de requête a déjà déclaré (sans perdre le set de la requête),
utilisez `with_include_overrides` :

```rust
use suprnova::data::with_include_overrides;

async fn show_album(req: Request, user: User) -> Response {
    with_include_overrides(
        |set| set
            .include_when(user.is_admin(), ["audit_log"])
            .exclude_when(!user.is_admin(), ["price_cost"]),
        async move {
            // À l'intérieur de cette portée, le résolveur de props lazy et le
            // résolveur d'include JSON:API voient le set fusionné.
            Inertia::data("Album/Show", album_dto).into_response()
        },
    ).await
}
```

La closure s'exécute contre un clone du set actuellement lié (ou le
défaut vide si aucun middleware n'en a lié un). Une fois la future
terminée, le set d'origine est restauré - c'est une redéfinition à
portée, pas une mutation.

Pour les tests, préférez `scope_include_set(set, future)` pour installer
un set neuf sans hériter d'aucun état ambiant.

## Structs génériques

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

L'extracteur TypeScript émet `export interface Paginated<T>` pour que
le code frontend puisse réutiliser le générique à travers les
instanciations.

L'allowlist `?include=` est indexée sur le chemin de type totalement
qualifié (`concat!(module_path!(), "::", stringify!(Paginated))`), pas
sur les instanciations de paramètre de type. `Paginated<UserDto>` et
`Paginated<ArticleDto>` déclarées dans le même module partagent une
seule allowlist - `allow_include` nomme un champ, et les noms de champ
ne dépendent pas des paramètres de type. Deux DTO différents nommés
`Paginated` dans des modules différents obtiennent chacun leur propre
allowlist ; leurs clés n'entrent pas en collision.

Remarque : `FormRequest` est supprimé pour les structs génériques parce
que ses bornes de trait (`DeserializeOwned + Validate + Send`) ne
peuvent pas être vérifiées sans connaître les paramètres de type
concrets. Fournissez votre propre impl si vous avez besoin d'extraire
une struct Data générique depuis une requête.

## Injection de champ depuis un paramètre de route

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

Pour `PATCH /users/{id}` avec le corps `{"name": "Ada"}`, l'`id` capturé
par la route est fusionné dans le payload validé. **Le chemin l'emporte
toujours sur une valeur fournie par le corps** (empêche l'IDOR via une
altération du corps).

`#[data(from_route_param)]` nu retombe sur le nom du champ. La macro
classe le dernier segment de chemin du champ à la compilation et
dispatche vers un parseur correspondant. Seuls les noms exacts listés
ci-dessous sont reconnus ; tout le reste (y compris `i8`/`i16`/`isize`,
`Uuid`, `DateTime`, les newtypes personnalisés) retombe sur
`pass_string` et laisse le `Deserialize` propre au champ faire le
travail.

| Type de champ | Parseur |
|---|---|
| `i64` | `parse_i64` |
| `u64` | `parse_u64` |
| `i32` | `parse_i32` |
| `u32` | `parse_u32` |
| `i128` | `parse_i128` (valide puis passe la chaîne brute ; le `Deserialize` du champ l'analyse) |
| `u128` | `parse_u128` (même motif de passage de chaîne) |
| `f64` | `parse_f64` (rejette les valeurs non finies) |
| `f32` | `parse_f32` (rejette les valeurs non finies) |
| `bool` | `parse_bool` (accepte seulement `"true"` / `"false"`) |
| N'importe quoi d'autre | `pass_string` - chaîne brute remise au `Deserialize` propre au champ |
| `Option<T>` ou `Field<T>` de l'un des types ci-dessus | Même parseur que `T` ; un paramètre de route manquant laisse le champ absent |

## Props lazy

```rust
use suprnova::Data;
use suprnova::inertia::Prop;

#[derive(Data)]
#[data(auto_lazy)]
pub struct AlbumDto {
    pub id: i64,
    pub songs: Prop,    // auto-enregistré comme ?include=songs
    pub artist: Prop,   // auto-enregistré comme ?include=artist
}
```

Saveur explicite par champ :

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

Utilisez `Inertia::data(component, dto)` pour rendre - le derive génère
un impl `IntoInertiaData` qui consulte l'include-set et l'allowlist :

```rust
return Inertia::data("Album/Show", album_dto);
```

Remarque : les structs qui portent des champs lazy suppriment
`Serialize`, `Deserialize`, et `FormRequest` parce que `Prop` ne les
implémente pas. Si un seul endpoint a besoin à la fois d'une analyse
entrante et d'une sortie lazy, utilisez deux DTO : un entrant
(`#[derive(Data, Validate)]` simple) et un sortant (`#[derive(Data)]`
avec des champs lazy).

## `when_loaded!` - lazy conditionnel selon le chargement de relation

Reflète le `#[AutoWhenLoadedLazy]` de Laravel-Data. L'impl
`From<Entity>` de l'utilisateur décide si la relation a été
préchargée :

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

Si l'entité n'a pas préchargé la relation nommée (selon
`IsRelationLoaded::is_relation_loaded`), `when_loaded!` retourne
`Prop::EagerNone` et le champ est absent de la réponse.

Les entités SeaORM ont besoin d'un impl `IsRelationLoaded` personnalisé
qui consulte leur état de relations chargées - il n'y a pas d'impl
générique fournie par le framework parce que le `ModelTrait` de SeaORM
ne porte pas d'état de relation chargée par instance (les relations
chargées vivent sur les résultats de requête, pas sur la struct de
modèle elle-même).

## Export TypeScript

`suprnova generate-types` émet des définitions TypeScript pour chaque
struct `#[derive(Data)]` (et l'ancien `#[derive(InertiaProps)]`).
Comportement :

- `Field<T>` → `field?: T | null`
- `Prop` → `field?: T` (la sémantique lazy peut-être-absent ; le `?` la
  porte, le type lui-même reste simple)
- `#[data(input_only)]` → exclu du type de sortie
- `#[data(output_only)]` → exclu du type d'entrée
- Struct générique → interface TypeScript générique
  (`export interface Paginated<T>`)
- Quand N'IMPORTE QUEL champ a `input_only` / `output_only` / `lazy`,
  deux interfaces sont émises : `<Name>` (sortie) et `<Name>Input`
  (entrée)

Les types générés ne fuient jamais de types propres à Rust
(`Prop<...>` n'apparaîtra pas dans le `.d.ts` de sortie).

## Scaffolding

```bash
suprnova make:inertia UserDto --data
```

Émet un squelette `#[derive(Data, Validate)]` au lieu de l'ancien
template `#[derive(InertiaProps)]`.

## Suivant

- [Validation](validation.md) - `#[derive(Validate)]`, les validateurs
  async, et comment `FormRequest` les appelle
- [Requêtes](requests.md) - la surface d'extracteur de requête dans
  laquelle `FormRequest` se branche
- [Réponses Inertia](frontend-inertia-responses.md) - le chemin
  `Inertia::data` et comment les props lazy deviennent éligibles au
  rechargement partiel
- [Ressources JSON:API](eloquent-resources.md) - `#[derive(Resource)]`
  pour les sorties JSON:API (le pendant de `Data` pour les payloads de
  sérialisation seule)
- [Modèle d'erreur](error-model.md) - comment le rejet `unknown_field`
  devient un 422 et comment les échecs de `FormRequest` reviennent sous
  forme de `ValidationErrors`

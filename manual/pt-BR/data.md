# Objetos de dados

O `#[derive(Data)]` do Suprnova deixa você descrever um formato de
solicitação de entrada, um formato de resposta de saída, e uma
exportação TypeScript em **uma única struct**.

## Início rápido

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

`#[derive(Data)]` gera:
- `Serialize` (pulando campos `#[data(input_only)]`)
- `Deserialize` (rejeitando campos `#[data(output_only)]` no payload, usando `T::default()` como padrão para eles)
- `FormRequest` com `authorize: true` por padrão - handlers podem receber o tipo diretamente como um extractor
- `IntoInertiaData` (o caminho de dispatch de `Inertia::data(component, dto)`)
- Um registro `inventory::submit!` para quaisquer campos `#[data(allow_include)]`

Adicione `#[derive(Validate)]` separadamente para que os attributes
`#[validate(...)]` permaneçam visíveis no call site do campo.

## Attributes de campo

| Attribute | Efeito |
|---|---|
| `#[data(input_only)]` | Aceito no Deserialize, omitido do Serialize |
| `#[data(output_only)]` | Rejeitado no Deserialize (422), incluído no Serialize |
| `#[data(allow_include)]` | O campo é elegível para `?include=`. **Nega por padrão**: qualquer solicitação `?include=foo` em que `foo` não esteja na allowlist retorna 400 |
| `#[data(lazy)]` | O campo é um `Prop` resolvido contra o include-set da solicitação; se auto-registra como `allow_include` |
| `#[data(lazy(inertia))]` | O mesmo que `lazy`, marcado para o protocolo de partial-reload do Inertia |
| `#[data(lazy(deferred))]` | Marcado para o protocolo de deferred-props do Inertia |
| `#[data(lazy(closure))]` | Sempre resolvido na visita inicial; lazy em partial reloads |
| `#[data(lazy(when_loaded))]` | Resolvido apenas se a entidade de origem tiver a relação pré-carregada |
| `#[data(from_route_param)]` | O valor do campo vem de uma captura de caminho (ex.: `/users/{id}`). Chave padrão = nome do campo; passe `#[data(from_route_param("id"))]` para sobrescrever |

## Attributes de struct

| Attribute | Efeito |
|---|---|
| `#[data(auto_lazy)]` | Todo campo tipado `Prop` é implicitamente `#[data(lazy)]` |
| `#[data(authorize = "path::to::fn")]` | Roteia o `FormRequest::authorize` gerado para uma função livre com assinatura `fn(req: &Request) -> bool`. O parser de corpo, o validador, o suporte a Precognition, e a injeção de route-param ainda vêm do derive |
| `#[data(allow_unknown_fields)]` | Aceita chaves do payload que não correspondem a nenhum campo da struct. O padrão é **estrito**: uma chave não reconhecida falha o deserialize com `serde::de::Error::unknown_field(..)` e aparece como um 422 através do `FormRequest`. Opte pelo permissivo apenas para DTOs de resposta que leem payloads de terceiros compatíveis com versões futuras |

A flag anterior `#[data(custom_authorize)]` - que suprimia toda a impl
de `FormRequest` e forçava você a reimplementar o parsing de corpo, a
validação, e o Precognition manualmente - se foi. A macro emite um
erro de migração se você tentar usá-la. Use
`#[data(authorize = "fn")]` em vez disso.

## `Field<T>` - Absent / Null / Value

Para endpoints PATCH em que "ausente do payload" precisa ser
distinguido de "null explícito":

```rust
use suprnova::data::Field;

match dto.bio {
    Field::Absent  => { /* não toque nesta coluna */ },
    Field::Null    => { /* limpe a coluna */ },
    Field::Value(text) => { /* defina como text */ },
}
```

`Field::Absent` (padrão) faz round-trip para omitido-do-JSON quando
emparelhado com `#[serde(default, skip_serializing_if = "Field::is_absent")]`
no call site. Sem `skip_serializing_if`, `Absent` serializa para JSON
`null`.

Para upserts de banco de dados de três vias:
`dto.bio.into_option_or_null() -> Option<Option<T>>` mapeia
`Absent → None`, `Null → Some(None)`, `Value(v) → Some(Some(v))`. Use
isto quando "não toque" e "defina como NULL" precisam ser distintos
downstream.

> **Ressalva:** `Field<Option<T>>` é lossy - `Value(None)` e `Null`
> ambos serializam como JSON `null` e desserializam de volta para
> `Null`. Para tipos internos anuláveis, prefira um `Field<T>` plano e
> deixe `Null` carregar o sinal de "limpar".

## A query string `?include=`

O `IncludeMiddleware` parseia a query string da solicitação em um
`RequestIncludeSet` por solicitação:

- `?include=foo,bar` - resolve os campos lazy `foo` e `bar`.
- `?include[]=foo&include[]=bar` - formato de array, mesmo resultado.
- `?exclude=`, `?only=`, `?except=` - paridade com a API do Laravel-Data.

Composição com `X-Inertia-Partial-Data` (o header de partial-reload do
Inertia): o include-set + a allowlist por DTO executa **primeiro**
para campos lazy marcados pelo dono, então uma solicitação por um
campo não permitido retorna 400 mesmo que o partial-data o tivesse
filtrado. O partial-data é aplicado **depois**, como um filtro "only"
final sobre os props resolvidos.

Registre o `IncludeMiddleware` globalmente - tipicamente entre sessão
e autorização na stack de middleware:

```text
SessionMiddleware → IncludeMiddleware → AuthMiddleware → handlers
```

### Include/exclude/only/except programáticos

`RequestIncludeSet` espelha o contrato `IncludeableData` do
Laravel-Data com builders encadeáveis. Handlers, testes, e middleware
podem construir ou sobrescrever um set sem cutucar os campos públicos
diretamente:

```rust
use suprnova::data::RequestIncludeSet;

let set = RequestIncludeSet::default()
    .include(["author", "comments"])
    .exclude(["password"])
    .only(["id", "name"])
    .except(["secret"]);

assert!(set.is_visible("name"));   // em `only`, não em `except`
assert!(!set.is_visible("secret"));// `except` sempre vence
assert!(set.includes("author"));   // solicitação pela relação `author`
```

| Método | Efeito | Equivalente no Laravel |
|---|---|---|
| `.include(fields)` | acrescenta à lista de include (campos lazy a resolver) | `Data::include(...$fields)` |
| `.exclude(fields)` | acrescenta à lista de exclude (campos a descartar) | `Data::exclude(...$fields)` |
| `.only(fields)` | inicializa ou estende a allowlist `only` | `Data::only(...$fields)` |
| `.except(fields)` | acrescenta à lista de except (sempre descarta) | `Data::except(...$fields)` |
| `.include_when(cond, fields)` | acrescenta apenas quando `cond == true` | `Data::includeWhen($field, $condition)` |
| `.exclude_when(cond, fields)` | acrescenta apenas quando `cond == true` | `Data::excludeWhen($field, $condition)` |
| `.only_when(cond, fields)` | estende `only` apenas quando `cond == true` | `Data::onlyWhen($field, $condition)` |
| `.except_when(cond, fields)` | acrescenta apenas quando `cond == true` | `Data::exceptWhen($field, $condition)` |
| `.merge(other)` | une dois sets (sobreposições em camadas, no lugar) | `array_merge` manual em PHP |
| `.includes(field)` | `field` (ou `field.path`) está na lista de include? | análogo a `relationLoaded()` |
| `.is_excluded(field)` | `field` está na lista de exclude? | lê a partial de exclude |
| `.is_excepted(field)` | `field` está na lista de except? | lê a partial de except |
| `.is_only_listed(field)` | `field` é permitido por `only` (ou `only` não definido)? | lê a partial de only |
| `.is_visible(field)` | ordem de resolução completa do Laravel: except → exclude → only | decisão de `resolveResource` |

Builders recebem qualquer `IntoIterator<Item = impl Into<String>>`,
então arrays, vecs, e slices de `&str`/`String` todos funcionam.
Strings são aparadas; entradas vazias são descartadas (espelhando
`from_query`).

Dot-paths em qualquer lista correspondem ao segmento raiz quando
sondados pelo nome puro - `include=["author.posts"]` reporta
`set.includes("author") == true`, espelhando a resolução de caminho do
Laravel-Data. O segmento aninhado `posts` é consumido por
`IncludeTree::from_include_set` para documentos compostos JSON:API.

### Sobrescrita do lado do handler: `with_include_overrides`

Para sobrepor overrides programáticos ao que a query string da
solicitação já declarou (sem perder o set da solicitação), use
`with_include_overrides`:

```rust
use suprnova::data::with_include_overrides;

async fn show_album(req: Request, user: User) -> Response {
    with_include_overrides(
        |set| set
            .include_when(user.is_admin(), ["audit_log"])
            .exclude_when(!user.is_admin(), ["price_cost"]),
        async move {
            // Dentro deste escopo, o resolver de lazy-prop e o resolver de
            // include do JSON:API veem o set combinado.
            Inertia::data("Album/Show", album_dto).into_response()
        },
    ).await
}
```

A closure executa contra um clone do set atualmente vinculado (ou o
padrão vazio se nenhum middleware vinculou um). Depois que o future
termina, o set original é restaurado - isto é uma sobrescrita com
escopo, não uma mutação.

Para testes, prefira `scope_include_set(set, future)` para instalar um
set novo sem herdar nenhum estado ambiente.

## Structs genéricas

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

O extractor de TypeScript emite `export interface Paginated<T>` para
que o código frontend possa reutilizar o genérico entre instanciações.

A allowlist de `?include=` é indexada pelo caminho totalmente
qualificado do tipo (`concat!(module_path!(), "::", stringify!(Paginated))`),
não pelas instanciações de type-parameter. `Paginated<UserDto>` e
`Paginated<ArticleDto>` declarados no mesmo módulo compartilham uma
allowlist - `allow_include` nomeia um campo, e nomes de campo não
dependem de type parameters. Dois DTOs diferentes chamados `Paginated`
em módulos diferentes recebem cada um sua própria allowlist; suas
chaves não colidem.

Nota: `FormRequest` é suprimido para structs genéricas porque seus
trait bounds (`DeserializeOwned + Validate + Send`) não podem ser
verificados sem conhecer os type params concretos. Forneça sua própria
impl se precisar extrair uma struct Data genérica de uma solicitação.

## Injeção de campo por parâmetro de rota

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

Para `PATCH /users/{id}` com corpo `{"name": "Ada"}`, o `id` capturado
pela rota é mesclado no payload validado. **O caminho sempre vence
sobre um valor fornecido pelo corpo** (previne IDOR via adulteração do
corpo).

`#[data(from_route_param)]` puro usa o nome do campo como padrão. A
macro classifica o último segmento de caminho do campo em tempo de
compilação e despacha para um parser correspondente. Apenas os nomes
exatos listados abaixo são reconhecidos; todo o resto (incluindo
`i8`/`i16`/`isize`, `Uuid`, `DateTime`, newtypes customizados) cai
para `pass_string` e deixa o próprio `Deserialize` do campo fazer o
trabalho.

| Tipo do campo | Parser |
|---|---|
| `i64` | `parse_i64` |
| `u64` | `parse_u64` |
| `i32` | `parse_i32` |
| `u32` | `parse_u32` |
| `i128` | `parse_i128` (valida e então passa a string crua adiante; o `Deserialize` do campo faz o parse) |
| `u128` | `parse_u128` (mesmo padrão de passthrough de string) |
| `f64` | `parse_f64` (rejeita valores não finitos) |
| `f32` | `parse_f32` (rejeita valores não finitos) |
| `bool` | `parse_bool` (aceita apenas `"true"` / `"false"`) |
| Qualquer outro | `pass_string` - string crua entregue ao próprio `Deserialize` do campo |
| `Option<T>` ou `Field<T>` de qualquer um dos acima | Mesmo parser que `T`; route param ausente deixa o campo absent |

## Props lazy

```rust
use suprnova::Data;
use suprnova::inertia::Prop;

#[derive(Data)]
#[data(auto_lazy)]
pub struct AlbumDto {
    pub id: i64,
    pub songs: Prop,    // auto-registrado como ?include=songs
    pub artist: Prop,   // auto-registrado como ?include=artist
}
```

Variante explícita por campo:

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

Use `Inertia::data(component, dto)` para renderizar - o derive gera
uma impl de `IntoInertiaData` que consulta o include-set e a
allowlist:

```rust
return Inertia::data("Album/Show", album_dto);
```

Nota: structs que carregam lazy suprimem `Serialize`, `Deserialize`, e
`FormRequest` porque `Prop` não os implementa. Se um único endpoint
precisa tanto de parsing de entrada quanto de saída lazy, use dois
DTOs: um de entrada (`#[derive(Data, Validate)]` puro) e um de saída
(`#[derive(Data)]` com campos lazy).

## `when_loaded!` - lazy condicional por relação carregada

Espelha o `#[AutoWhenLoadedLazy]` do Laravel-Data. A impl
`From<Entity>` do usuário decide se a relação foi pré-carregada:

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

Se a entidade não pré-carregou a relação nomeada (de acordo com
`IsRelationLoaded::is_relation_loaded`), `when_loaded!` retorna
`Prop::EagerNone` e o campo fica absent na resposta.

Entidades SeaORM precisam de uma impl customizada de
`IsRelationLoaded` que consulte seu estado de relações carregadas -
não há uma blanket impl fornecida pelo framework porque o
`ModelTrait` do SeaORM não carrega estado de relação-carregada por
instância (relações carregadas vivem nos resultados da query, não na
própria struct do model).

## Exportação TypeScript

`suprnova generate-types` emite definições TypeScript para toda struct
`#[derive(Data)]` (e, de forma legada, `#[derive(InertiaProps)]`).
Comportamento:

- `Field<T>` → `field?: T | null`
- `Prop` → `field?: T` (a semântica lazy de pode-estar-absent; o `?` a carrega, o tipo em si é simples)
- `#[data(input_only)]` → excluído do tipo de saída
- `#[data(output_only)]` → excluído do tipo de entrada
- Struct genérica → interface genérica TypeScript (`export interface Paginated<T>`)
- Quando QUALQUER campo tem `input_only` / `output_only` / `lazy`, duas interfaces são emitidas: `<Name>` (saída) e `<Name>Input` (entrada)

Tipos gerados nunca deixam vazar tipos exclusivos do Rust (`Prop<...>`
não vai aparecer no `.d.ts` de saída).

## Scaffolding

```bash
suprnova make:inertia UserDto --data
```

Emite um esqueleto `#[derive(Data, Validate)]` em vez do template
legado `#[derive(InertiaProps)]`.

## Próximos passos

- [Validação](validation.md) - `#[derive(Validate)]`, validadores async, e como `FormRequest` os chama
- [Solicitações](requests.md) - a superfície de extractor de solicitação em que `FormRequest` se encaixa
- [Respostas Inertia](frontend-inertia-responses.md) - o caminho de `Inertia::data` e como props lazy se tornam elegíveis a partial-reload
- [Recursos JSON:API](eloquent-resources.md) - `#[derive(Resource)]` para saídas JSON:API (irmão de `Data` para payloads somente de serialização)
- [Modelo de erros](error-model.md) - como a rejeição de `unknown_field` se torna um 422 e como falhas de `FormRequest` retornam como `ValidationErrors`

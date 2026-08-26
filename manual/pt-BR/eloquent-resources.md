# Recursos JSON:API

O Suprnova traz uma camada de recursos JSON:API para APIs REST
tipadas. Marque uma struct `#[derive(Data)]` com
`#[json_resource("type")]` e o framework emite uma impl de
`IntoJsonResource` que trata envelopes únicos, coleções, coleções
paginadas, sparse fieldsets (`?fields[type]=...`), documentos
`included` compostos, e cadeias `?include=a.b.c` multinível pelo
mesmo caminho de código. As duas facades - `Resource` e `JsonApi` -
são o mesmo tipo sob dois nomes; use a que combinar com o estilo da
sua casa.

## Definindo um recurso

```rust
use suprnova::Data;

#[derive(Debug, Clone, Data)]
#[json_resource("users")]
pub struct UserResource {
    pub id: i64,
    pub email: String,

    // `input_only` mantém `password` disponível no lado do
    // form-request, mas o suprime da saída da API.
    #[data(input_only)]
    pub password: String,

    // Marca um campo como um *relacionamento*: ele nunca aparece em
    // `attributes`, produz um objeto de relacionamento do JSON:API em
    // vez disso, e é elegível para `?include=`. O tipo do campo
    // precisa implementar `IntoJsonResource` (diretamente, ou via
    // `Vec<T>` / `Option<T>`).
    #[data(allow_include)]
    pub posts: Vec<PostResource>,
}
```

A keyword `id_field` renomeia o campo que fornece o `id` do JSON:API:

```rust
#[derive(Data)]
#[json_resource("orders", id_field = "uuid")]
pub struct OrderResource {
    pub uuid: String,
    pub total_cents: i64,
}
```

## Renderizando respostas

Construa uma resposta pendente a partir de um handler e chame
`.render().await`:

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
    // `paginate(per_page)` lê `?page=` da solicitação atual automaticamente.
    let page = User::query().paginate(10).await?;
    // Converte o paginador do model em um paginador de recurso campo a
    // campo - `data` é `pub`, o resto das contagens/links são carregados.
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

`JsonApi::single` / `JsonApi::collection` / `JsonApi::paginated` são
pontos de entrada com alias idênticos, se você preferir a grafia do
Laravel.

## Mutadores encadeáveis

`JsonApiResponse` é um objeto pendente. Personalize o envelope antes
de chamar `.render().await`. Todo mutador é `self` → `Self`, então
eles compõem:

```rust
use suprnova::{Resource, JsonApiInfo};
use serde_json::json;

let info = JsonApiInfo::new()
    .with_version("1.1")
    .with_ext("https://jsonapi.org/ext/atomic")
    .with_meta("copyright", json!("2026 Acme Inc."));

Resource::single(user)
    .status(201)                                  // override do status HTTP
    .with_meta("trace_id", json!("req-7"))        // KV de meta no nível superior
    .with_link("self", "/api/users/1")            // link no nível superior
    .with_jsonapi(info)                           // `jsonapi` no nível superior
    .additional(json!({ "api_version": "2.0" }).as_object().unwrap().clone())
    .render()
    .await
```

| Mutador | Análogo no Laravel | Efeito |
|---|---|---|
| `.status(code)` | `ResourceResponse::calculateStatus` | Sobrescreve o status HTTP. |
| `.created()` | `wasRecentlyCreated → 201` | Abreviação de `.status(201)`. |
| `.with_meta(k, v)` / `.meta(k, v)` | `with($request)` | KV de `meta` no nível superior. |
| `.with_meta_map(m)` | `with($request)` em massa | Faz merge de um mapa no `meta` do nível superior. |
| `.with_link(rel, href)` / `.link(rel, href)` | `with($request)['links']` | KV de `links` no nível superior. |
| `.with_link_value(rel, v)` | forma de objeto de link | Link de nível superior como `{href, meta}`. |
| `.with_additional(k, v)` | `additional($data)` | Chave no nível raiz, ao lado de `data`. |
| `.additional(map)` | `additional($data)` | Chaves adicionais em massa. |
| `.with_jsonapi(info)` | `JsonApiResource::configure(...)` | Membro `jsonapi` de nível superior. |

Os membros canônicos (`data`, `included`, `links`, `meta`, `jsonapi`,
`errors`) nunca são sobrescritos por `.additional(...)`.

## `links` e `meta` por recurso

Sobrescreva os padrões de `IntoJsonResource::resource_links` e
`IntoJsonResource::resource_meta` para anexar links / metadados ao
*objeto do recurso*, não à raiz do documento:

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

Os dois usam por padrão um `Map` vazio para recursos derivados da
macro, então o renderizador do JSON:API omite as chaves quando não
usadas. Sobrescreva `resource_top_level_meta` para elevar os
metadados por recurso ao membro `meta` de nível superior do
envelope.

## Atributos condicionais - `Maybe<T>` / `MissingValue<T>`

Use `Maybe` para omitir um campo do objeto `attributes` renderizado
com base em uma condição de runtime. Este é o análogo do
`MissingValue` do Laravel e da família `when()` / `whenLoaded()` /
`unless()` no Suprnova.

```rust
use suprnova::{Maybe, MissingValue};

// Os dois nomes apontam para o mesmo tipo.
let m1: Maybe<&str> = Maybe::present("email@example.com");
let m2: MissingValue<&str> = MissingValue::missing();
let m3 = Maybe::when(user.is_verified, &user.verified_at);
let m4 = Maybe::unless(user.is_admin, &user.public_handle);
let m5 = Maybe::when_with(expensive_check(), || compute_value()); // lazy
```

Para structs derivadas da macro, declare um campo como `Maybe<T>` e o
renderizador o descarta automaticamente quando `Missing`. Para
`resource_attributes` escritos à mão, use o helper
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

O renderizador também chama `strip_missing_values(&mut value)` sobre
o objeto `attributes` inteiro, então valores `Maybe::Missing`
aninhados dentro de estruturas arbitrárias derivadas de serde são
descartados recursivamente - útil quando um transformador
profundamente aninhado quer omitir subcampos.

## Sparse fieldsets

O `IncludeMiddleware` do framework parseia parâmetros de query no
formato `?fields[type]=email,name` e os vincula a um task-local. O
`resource_attributes` emitido pela macro consulta o fieldset e só
emite os attributes solicitados. Nenhum trabalho do lado do handler é
necessário - instale o middleware e a camada de recursos o honra
automaticamente.

```rust
// Solicitação: GET /api/users/7?fields[users]=email
// Resposta: { "data": { "type": "users", "id": "7", "attributes": { "email": "alice@example.com" } } }
```

## Documentos compostos - cadeias `?include=`

Declare campos de relacionamento com `#[data(allow_include)]`. O
framework constrói uma `IncludeTree` a partir de
`?include=author.posts.tags,comments`, percorre todo nó, e empurra
objetos de recurso totalmente resolvidos para `included`. A
deduplicação roda no momento do push através de `IncludedSink`,
chaveada por `(type, id)` conforme o §8 da spec do JSON:API - então
uma coleção de 1.000 itens em que todo item compartilha o mesmo autor
resolve o autor exatamente uma vez. O pico de memória e CPU fica
proporcional aos recursos incluídos distintos, não ao fan-in do
relacionamento.

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

Uma solicitação que nomeia um caminho de include fora da allowlist
deste recurso recebe um envelope de erros 400 do JSON:API.

### Limite de profundidade

Um caminho de include pode carregar no máximo cinco segmentos.
`?include=a.b.c.d.e.f` é truncado para `a.b.c.d.e` antes de qualquer
coisa percorrê-lo, correspondendo ao
`JsonApiResource::$maxRelationshipDepth` do Laravel. Mude o teto uma
vez, no boot:

```rust
// Em bootstrap::register()
suprnova::max_relationship_depth(3);
```

O limite importa porque um grafo de relacionamentos pode ser cíclico:
`?include=author.posts.author.posts...` custa mais trabalho a cada
segmento que um cliente digita, e nada além do tamanho da query string
o delimita. O truncamento só remove segmentos, nunca acrescenta, e cada
nível ainda confere a própria allowlist antes de descer - então um
caminho truncado nunca consegue alcançar dados que o caminho completo
não alcançaria.

Vale conhecer uma consequência: um segmento além do limite é descartado
antes de a allowlist vê-lo. Com um limite de 2,
`?include=author.posts.secrets` retorna 200 com `author` e `posts`
incluídos, em vez do 400 que o caminho completo renderia, porque
`secrets` já não existe no momento em que algo o valida.

`max_relationship_depth(0)` desliga os includes por completo. O 0 do
Laravel ainda emite o primeiro salto, porque o clamp dele só se aplica
à cauda depois que o segmento inicial já foi separado; o 0 do Suprnova
significa nenhum relacionamento.

### Por que Suprnova diverge

Três divergências visíveis do `JsonApiResource` do Laravel:

1. **Negação padrão estrita para `?include=`.** A camada de recursos
   do Laravel ignora silenciosamente caminhos de include que não se
   resolvem. O Suprnova os rejeita com um `400 Bad Request` carregando
   um envelope de erros do JSON:API. A postura de negação padrão do
   §5.2.2 da spec é o contrato contra o qual os clientes podem
   programar; a ignorância silenciosa esconde bugs do cliente e quebra
   a integridade do documento composto.

2. **`.status(code)` / `.created()` explícitos em vez de 201
   automático.** O Laravel define `201` automaticamente a partir de
   `wasRecentlyCreated` no model Eloquent subjacente. O Suprnova
   desacopla o DTO de recurso de qualquer ciclo de vida de persistência
   específico, então o status é definido no próprio objeto de
   resposta - `.created()` quando é isso que você quer dizer,
   `.status(204)` quando a resposta é vazia, e assim por diante. Um
   único mutador se mantém honesto sob qualquer fluxo.

3. **Um limite de profundidade de `0` desliga os includes por
   completo.** O Laravel limita apenas a cauda de um caminho, depois
   que o segmento inicial já foi separado, então o `0` dele ainda emite
   o primeiro salto. O Suprnova trunca o caminho inteiro, então
   `max_relationship_depth(0)` significa nenhum relacionamento - veja
   Limite de profundidade acima.

## Paginação

`Resource::paginated(p)` funciona com qualquer paginador que
implemente o trait `Paginated<T>` - tanto `LengthAwarePaginator<T>`
quanto `CursorPaginator<T>` de `suprnova::pagination` trazem essa
impl. O renderizador anexa `links.{self,first,prev,next,last}` e um
bloco `meta.pagination` automaticamente.

```rust
use suprnova::{LengthAwarePaginator, Resource};

let page = LengthAwarePaginator::new(items, total, per_page, current_page)
    .with_base_url("/api/users");
Resource::paginated(page).render().await
```

## Envelopes de erro

Todo `FrameworkError` sabe como se renderizar como um envelope
`{"errors": [...]}` do JSON:API via `into_json_api_response()`. O
helper é exposto porque `FrameworkError` carrega um código de status,
um ponteiro de origem por nome de campo (para `ValidationError`), e
um token de correlação de request-id sob `meta.request_id`. Respostas
5xx são sanitizadas: a mensagem crua nunca chega ao cliente a menos
que `APP_DEBUG=true` esteja definido no ambiente ativo, caso em que
ela aparece sob `meta.debug_message`.

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

## Resumo das superfícies

| Superfície Suprnova | Equivalente no Laravel 13 |
|---|---|
| Facades `Resource` / `JsonApi` | `JsonResource::make`, `JsonApiResource` |
| `JsonApiResponse` | `ResourceResponse`, `JsonApiResource::toResponse` |
| `JsonApiBuilder` | (construtor interno de `ResourceResponse`) |
| Trait `IntoJsonResource` | `JsonResource::toArray`, `toAttributes`, `toRelationships`, `toLinks`, `toMeta`, `with` |
| `RelationshipValue` / `ResourceIdentifier` | forma de array dentro de `toRelationships` |
| `IncludeTree` | `?include=` parseado a partir de `JsonApiRequest` |
| `RequestFieldsetSet` | `?fields[type]=` parseado a partir de `JsonApiRequest` |
| `Maybe<T>` / `MissingValue<T>` | `MissingValue` + `whenLoaded` / `when` / `unless` |
| `JsonApiInfo` | `JsonApiResource::$jsonApiInformation` |
| `JsonApiResponse::status(code)` / `.created()` | `ResourceResponse::calculateStatus` |
| `JsonApiResponse::additional(map)` / `.with_additional(k, v)` | `JsonResource::additional($data)` |
| `JsonApiResponse::with_meta(k, v)` / `.meta(k, v)` | `JsonResource::with($request)['meta']` |
| `JsonApiResponse::with_link(rel, href)` / `.link(rel, href)` | `JsonResource::with($request)['links']` |
| `JsonApiResponse::with_jsonapi(info)` | `JsonApiResource::configure(...)` |
| `current_fieldset()` / `scope_fieldset(...)` | fieldset task-local, definido por `IncludeMiddleware` |
| `IncludeResolutionError` → envelope 400 | parser `?include=` em modo estrito |

Reexportações de nível superior sob `suprnova::`: `Resource`,
`JsonApi`, `JsonApiResponse`, `JsonApiBuilder`, `JsonApiInfo`,
`IncludedSink`, `IntoJsonResource`, `RelationshipValue`,
`ResourceIdentifier`, `IncludeTree`, `RequestFieldsetSet`, `Maybe`,
`MissingValue`, `insert_maybe`, `strip_missing_values`,
`AsRelationshipValue`, `PushIncluded`, `IncludeResolutionError`,
`current_fieldset`, `scope_fieldset`.

## Próximos passos

- [Serialização Eloquent](eloquent-serialization.md) -
  `#[derive(Data)]`, campos hidden/visible, o equivalente a `toArray`
  que alimenta os attributes do recurso
- [Relacionamentos Eloquent](eloquent-relationships.md) - o que
  `#[data(allow_include)]` consome; os tipos de relação tipados por
  trás dos documentos compostos
- [Paginação](pagination.md) - `LengthAwarePaginator`,
  `CursorPaginator`, e o trait `Paginated<T>` que `Resource::paginated`
  consome
- [Dados](data.md) - a macro `#[derive(Data)]` compartilhada com o
  Inertia, o middleware `?include=`/`?fields[type]=`, e os padrões de
  `Maybe<T>`
- [Modelo de erros](error-model.md) - como
  `FrameworkError::into_json_api_response` se encaixa no contrato de
  conversão

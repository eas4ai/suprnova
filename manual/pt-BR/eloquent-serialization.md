# Serialização Eloquent

Como os models Eloquent se tornam JSON. O capítulo cobre `to_array()` e
`to_json()`, o pipeline de filtros `hidden` / `visible` / `appends`,
os dois helpers terminais `to_array_except` / `to_array_only`, a forma
como appends conectam acessadores à saída, e as duas divergências do
Laravel que costumam pegar as pessoas desprevenidas: a armadilha do
bypass do serde, e o fato de que relações eager-loaded não se
incorporam automaticamente ao corpo JSON.

Se você já leu [API Eloquent](eloquent.md), a maioria dos nomes aqui
é familiar - a referência de atributos está naquele capítulo. Esta
página é onde vive o *contrato de serialização*: quais campos
aparecem, em que ordem os filtros se aplicam, e o que causa um leak
se você esquecer disso.

## Sumário

- [O contrato](#o-contrato)
- [`to_array` e `to_json`](#to-array-e-to-json)
- [Ocultando campos - `hidden = [...]`](#ocultando-campos-hidden)
- [Permitindo campos - `visible = [...]`](#permitindo-campos-visible)
- [Anexando acessadores - `appends = [...]`](#anexando-acessadores-appends)
- [A ordem do pipeline de filtros](#a-ordem-do-pipeline-de-filtros)
- [Filtragem por chamada - `to_array_except` / `to_array_only`](#filtragem-por-chamada-to-array-except-to-array-only)
- [Ocultação condicional por visualizador](#ocultação-condicional-por-visualizador)
- [A armadilha do bypass do serde](#a-armadilha-do-bypass-do-serde)
- [Serializando coleções](#serializando-coleções)
- [Relações eager-loaded e serialização](#relações-eager-loaded-e-serialização)
- [E o JSON:API?](#e-o-json-api)
- [Onde cada peça vive](#onde-cada-peça-vive)
- [Próximos passos](#próximos-passos)

## O contrato

Toda struct `#[suprnova::model]` recebe dois métodos de serialização
do trait `Model`:

```rust
fn to_array(&self) -> serde_json::Value;
fn to_json(&self) -> String;
```

`to_array` produz um `serde_json::Value` para uso em respostas de
handler e em testes. `to_json` é um wrapper fino -
`serde_json::to_string(&self.to_array())` - então um único pipeline
de filtros é dono das duas formas.

A saída é um objeto JSON chaveado pelo nome do campo da struct (ou
pelo rename de serde que você tenha aplicado), filtrado por três
controles opcionais declarados em `#[model(...)]`:

- `hidden = [...]` - denylist de colunas
- `visible = [...]` - whitelist de colunas (mutuamente exclusivo com `hidden`)
- `appends = [...]` - métodos acessadores para injetar sob chaves nomeadas

Quando o model não declara nenhum destes, o corpo padrão do trait
roda: serializa `self` via `serde_json::to_value(self)`, remove dois
campos de rascunho internos do framework (`__eager` e `__pivot` - veja
[relações eager-loaded](#relações-eager-loaded-e-serialização)),
retorna o resultado. Quando o model declara qualquer um deles, a
macro emite um override que executa o [pipeline](#a-ordem-do-pipeline-de-filtros).

## `to_array` e `to_json`

O exemplo mínimo útil - uma linha saindo como JSON:

```rust
use suprnova::{json_response, model, Model, Request, Response};
use chrono::{DateTime, Utc};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    json_response!(user.to_array())
}
```

`json_response!` aceita qualquer `serde_json::Value`; `user.to_array()`
produz um. O equivalente no formato string é `user.to_json()` - corpo
idêntico, filtros idênticos, só um `to_string` extra.

Você também pode usar `serde_json::to_value(&user)` diretamente.
**Não faça isso para nada voltado ao usuário.** Isso contorna o
pipeline de filtros completamente - veja [a armadilha do bypass do
serde](#a-armadilha-do-bypass-do-serde) mais adiante no capítulo para
entender por quê.

## Ocultando campos - `hidden = [...]`

A forma de denylist. Toda coluna exceto as listadas é serializada:

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

O JSON voltado ao usuário deste model nunca contém `password` ou
`remember_token`:

```json
{
    "id": 42,
    "name": "Alice",
    "email": "alice@example.com",
    "created_at": "2026-05-30T11:14:22Z",
    "updated_at": "2026-05-30T11:14:22Z"
}
```

`hidden` é a ferramenta certa quando **a maioria dos campos vai para
o cliente** e você precisa subtrair um pequeno conjunto de segredos,
flags internas, ou dados só de autenticação.

## Permitindo campos - `visible = [...]`

A forma de allowlist. Só as colunas listadas são serializadas:

```rust
#[model(
    table = "users",
    visible = ["id", "name", "avatar_url"],
)]
pub struct PublicUserView { /* ... */ }
```

Útil para um model que existe especificamente para ser uma projeção
pública fina (pense nos tipos "Profile" / "PublicUser" do Laravel).
`visible` também é a ferramenta certa quando a tabela tem dezenas de
colunas internas e só algumas vão para o cliente - listar o
conjunto a manter é mais curto que listar o conjunto a remover.

`hidden` e `visible` são **mutuamente exclusivos em tempo de
compilação**. A macro emite um erro se você definir os dois:

```text
error: cannot specify both `hidden` and `visible` on the same model
 --> src/models/user.rs:7:1
  |
7 | #[model(table = "users", hidden = ["x"], visible = ["y"])]
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

Os dois são opostos de política - escolha o que combina com a
intenção do seu model, não os dois.

## Anexando acessadores - `appends = [...]`

`appends` injeta valores computados na saída JSON. Cada entrada nomeia
um método marcado com `#[accessor]` no model; a macro o chama durante
`to_array()` e guarda o valor de retorno sob a mesma chave.

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    fillable = ["first_name", "last_name"],
    appends = ["full_name", "initials"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[accessor]
    pub fn initials(&self) -> String {
        let f = self.first_name.chars().next().unwrap_or(' ');
        let l = self.last_name.chars().next().unwrap_or(' ');
        format!("{f}{l}")
    }
}
```

O usuário serializado agora carrega as duas chaves computadas:

```json
{
    "id": 7,
    "first_name": "Alice",
    "last_name": "Pond",
    "created_at": "...",
    "updated_at": "...",
    "full_name": "Alice Pond",
    "initials": "AP"
}
```

A macro valida as entradas de `appends` em tempo de compilação:

- Cada nome precisa parsear como um identificador Rust (`"full-name"`
  falha - não é um ident válido).
- Se o método nomeado não existir no bloco `impl` do model, o
  compilador aponta para o dispatcher gerado pela macro com um erro
  claro de `no method named 'full_name' found`.

Chamar `user.full_name()` diretamente do Rust funciona exatamente
como qualquer outro método - `appends` só controla a **tabela de
dispatch do JSON**. Acessadores continuam sendo métodos normais.

## A ordem do pipeline de filtros

Quando um model declara qualquer um de `hidden`, `visible`, ou
`appends`, a macro emite um override de `to_array` que executa quatro
passos nesta ordem:

1. Serializa `self` para um `serde_json::Map` via `serde_json::to_value`.
2. Remove as chaves internas do framework `__eager` e `__pivot`
   incondicionalmente (mais sobre elas em [a seção de
   relações](#relações-eager-loaded-e-serialização)).
3. Aplica `visible` como **whitelist** quando não vazio: toda chave
   que NÃO está na lista é removida.
4. Aplica `hidden` como **denylist**: toda chave listada que
   sobreviveu à whitelist é removida.
5. Injeta `appends`: para cada entrada, chama o acessador registrado
   e insere seu resultado sob o nome da entrada.

### Por que Suprnova diverge

O Laravel usa a mesma ordem `hidden` → `visible` → `appends`. A
divergência está no passo 5: no Suprnova, appends rodam **depois**
da denylist hidden, e sempre aparecem - mesmo que seu nome também
esteja listado em `hidden`. O raciocínio é o mesmo do Laravel: se
você declara tanto `$appends = ['full_name']` quanto
`$hidden = ['full_name']`, a intenção é "compute e envie" - `appends`
é o sinal mais específico. A ordem importa quando a chave de um
acessador colide com o nome de uma coluna (por exemplo, um acessador
que sobrescreve o valor da coluna `display_name` armazenada); o
acessador vence na saída para o cliente.

## Filtragem por chamada - `to_array_except` / `to_array_only`

Para casos isolados em que a declaração por coluna não encaixa, dois
helpers terminais executam o pipeline `to_array` completo e depois
cortam o resultado por nome:

```rust
use suprnova::{json_response, Model};

pub async fn admin_show(user: User) -> suprnova::Response {
    // remove alguns campos extras para um endpoint de admin que
    // precisa da maior parte da linha, mas não destes:
    json_response!(
        user.to_array_except(&["password_hash", "remember_token", "internal_notes"])
    ))
}

pub async fn directory_show(user: User) -> suprnova::Response {
    // diretório público - só as colunas que queremos publicar:
    json_response!(
        user.to_array_only(&["id", "name", "avatar_url"])
    ))
}
```

Os dois produzem um `serde_json::Value` - eles não alteram `self` e
não mudam serializações futuras da mesma linha. Eles rodam o pipeline
completo `hidden` / `visible` / `appends` primeiro, depois aplicam o
próprio corte por cima. `to_array_only` retorna um objeto JSON
*novo* contendo só as chaves nomeadas; `to_array_except` retorna o
objeto completo menos as chaves nomeadas.

### Por que Suprnova diverge

O `$user->makeHidden(['x'])` e o `$user->makeVisible(['x'])` do
Laravel **alteram** a instância do model - toda chamada subsequente
de `toArray()`, incluindo as que acontecem quando o model está
aninhado dentro da serialização de um pai, vê o estado alterado. Os
helpers do Suprnova são **terminais**. Eles produzem um `Value` e
param aí. Se você precisa que a mudança se propague, declare-a em
`#[model(hidden = [...])]` / `#[model(visible = [...])]` para que o
*tipo* expresse a política, não uma mutação oculta na instância.

A razão no formato do Rust: uma struct Eloquent no Suprnova é uma
struct Rust simples, sem conjunto de atributos em runtime. Não há
lugar para uma flag de visibilidade do lado da instância viver sem
adicionar estado oculto ambiente, que é o tipo de armadilha que o
framework evita de propósito.

## Ocultação condicional por visualizador

O padrão idiomático quando a visibilidade depende do visualizador é
um match no call site, ramificando para o filtro por chamada certo:

```rust
use suprnova::{Auth, json_response, Model, Request, Response};

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    let viewer = Auth::user_as::<User>().await?;
    let viewing_self = viewer.as_ref().map(|v| v.id) == Some(user.id);

    let body = if viewing_self {
        user.to_array()
    } else {
        user.to_array_except(&["email", "phone", "stripe_customer_id"])
    };

    json_response!(body)
}
```

Para uma forma mais elaborada por visualizador - attributes
diferentes para admins, usuários em trial, usuários pagantes - a
ferramenta certa é a **camada de recursos JSON:API** com campos
`Maybe<T>` / `MissingValue<T>`. Veja [Recursos
JSON:API](eloquent-resources.md#conditional-attributes--maybet--missingvaluet)
para a forma declarativa.

## A armadilha do bypass do serde

Esta é a coisa mais importante para saber sobre serialização Eloquent
no Suprnova.

**Os filtros `hidden` / `visible` / `appends` só rodam através de
`to_array()` e `to_json()`.** Eles *não* são impostos pela impl
derivada de `Serialize`. Retornar a struct por qualquer outro caminho
do serde contorna os filtros completamente.

Isso significa que **todos estes fazem `password` fazer leak**:

```rust
// Serde direto - contorna to_array, hidden não tem efeito:
let raw = serde_json::to_value(&user).unwrap();

// json_response! com um campo de struct - o mesmo:
json_response!({ "user": user }))

// Aninhado dentro de outro container serializável - o mesmo:
#[derive(Serialize)]
struct EnvelopeWithUser { ok: bool, user: User }
let env = EnvelopeWithUser { ok: true, user };
json_response!(env))

// Retornando um Vec<User> através do serde - o mesmo:
json_response!(users))   // onde users: Vec<User>
```

Só estes passam pelo pipeline de filtros:

```rust
json_response!(user.to_array()))
json_response!(users_collection.to_array()))  // Collection<User>
json_response!(user.to_array_except(&["secret"])))
json_response!(user.to_array_only(&["id", "name"])))
```

### Por que isso acontece

A impl blanket `Serialize for Vec<T>` do serde (e qualquer outro
container) chama `T::serialize` diretamente. O pipeline de filtros do
Suprnova vive no método de trait `Model::to_array`, não em
`Serialize`. O método do trait não é chamado a menos que você o
chame.

O framework se protege contra a armadilha *interna* (os campos de
rascunho `__eager` / `__pivot` são marcados `#[serde(skip)]` para que
também não façam leak por esse caminho), mas a macro deliberadamente
**não** emite `#[serde(skip_serializing)]` em campos hidden - fazer
isso quebraria usos legítimos do serde com o model SeaORM interno,
onde quem chama quer a linha completa (por exemplo, RPC interno,
camadas de persistência, diagnóstico, testes).

### A regra

Para qualquer valor que atravesse a fronteira de confiança de volta
para um cliente, passe por `to_array()` ou por um de seus primos
filtrados. O contrato de quatro linhas que compra essa segurança:

| Quero | Uso | Resultado |
|---|---|---|
| Serializar um model | `user.to_array()` | Objeto JSON filtrado |
| Serializar uma coleção | `collection.to_array()` | Array JSON filtrado |
| Subtrair alguns campos | `user.to_array_except(&["x"])` | Filtrado + subtraído |
| Manter só alguns campos | `user.to_array_only(&["x"])` | Só as chaves listadas |

Um linter ou uma revisão em tempo de PR para
`json_response!\({.*: [a-z_]+ ?})` e `serde_json::to_value\(&\w+\)`
em valores de model é uma forma barata de manter a regra. Os
próprios testes do framework para serialização de `Model` cobrem os
dois caminhos.

## Serializando coleções

Uma `Collection<M>` - retornada por `Builder::get()`, `Model::all()`,
e acessadores de relação - tem seu próprio `to_array()` e `to_json()`
que percorrem o `Vec<M>` subjacente e chamam `to_array()` **por
linha**. O resultado é um array JSON de objetos filtrados:

```rust
use suprnova::{json_response, Model};

pub async fn list() -> suprnova::Response {
    let users = User::all().await?;
    json_response!(users.to_array())
}
```

Este é o único lugar para obter o filtro por linha em um resultado
com várias linhas. `serde_json::to_value(&users)` emitiria um Vec via
a impl blanket do serde e contornaria os filtros de todas as linhas de
uma vez - o helper no nível de coleção existe exatamente para fechar
essa brecha.

```rust
// O override de Collection<M>:
pub fn to_array(&self) -> Value {
    Value::Array(self.0.iter().map(|m| m.to_array()).collect())
}
```

Para um paginador, os dados envolvidos vivem em
`LengthAwarePaginator::data` / `CursorPaginator::data` e são um
`Vec<M>` - chame `.to_array()` em cada item antes de montar a
resposta do paginador, ou use a [forma paginada do
JSON:API](eloquent-resources.md#pagination), que cuida da filtragem
por linha como parte do pipeline de recursos.

## Relações eager-loaded e serialização

Esta é a segunda divergência para internalizar.

Quando você chama `.with(["posts"])` em um builder, o framework
carrega os posts e os guarda em um `EagerLoadCache` por linha (o
campo `__eager` auto-injetado). O acessador para lê-los -
`user.posts_loaded()` - lê a partir desse cache.

**O cache é `#[serde(skip)]` e `to_array()` o remove
incondicionalmente.** Relações eager-loaded não se incorporam
automaticamente à saída JSON. Um `to_array()` num usuário com posts
eager-loaded fica idêntico a um `to_array()` num usuário sem.

### Por que Suprnova diverge

O `toArray()` do Laravel percorre `$model->getRelations()` e
incorpora toda relação carregada na saída. O conjunto de atributos em
formato de array do PHP torna isso natural - uma relação é só mais
uma entrada chaveada no model.

As structs Eloquent tipadas do Rust não têm esse conjunto. Uma struct
`User` tem colunas tipadas, não um mapa heterogêneo de "quaisquer
relações que tenham sido carregadas". Incorporar `posts` exigiria ou
injeção de campo em runtime numa struct tipada (um mecanismo de
bypass do serde), ou um caminho de serialização paralelo que consulta
o cache depois de rodar o serializador de colunas. As duas opções
acoplariam a forma JSON de todo model a quais relações um chamador em
particular fez eager-load - um contrato que é estrutural no PHP
porque os clientes aprendem a depender dele, e um contrato que o
Suprnova se recusa explicitamente a entregar porque faz a forma do
JSON depender da construção da query do lado de quem chama.

### As duas formas de entregar dados de relação

**1. Acessador explícito + appends.** Defina um método que lê de
`<rel>_loaded()`, registre-o em `appends`. A relação aparece sob
qualquer chave que você nomear. Isso funciona quando a relação é
*sempre* eager-loaded no caminho de leitura:

```rust
use suprnova::{accessor, model};
use serde_json::Value;

#[model(
    table = "users",
    appends = ["posts"],
)]
pub struct User { /* ... */ }

impl User {
    #[accessor]
    pub fn posts(&self) -> Value {
        // posts_loaded() ENTRA EM PANIC se .with(["posts"]) não foi
        // chamado no caminho de leitura. O acessador PRECISA rodar
        // depois do eager-load.
        let posts = self.posts_loaded();
        serde_json::to_value(posts).unwrap_or(Value::Null)
    }
}

// O caminho de leitura PRECISA fazer eager-load:
let users = User::query()
    .with(["posts"])
    .get()
    .await?;
let body = users.to_array();   // a chave "posts" de cada usuário é populada
```

O contrato é explícito: esqueça o `.with(["posts"])`, e o acessador
entra em panic na primeira chamada de `posts_loaded()` de uma linha
(o cache eager entra em panic na leitura quando a relação não foi
carregada, por design - um array vazio silencioso esconderia o bug).
Para eager-load opcional, use a forma HasOne, que retorna
`Option<&T>` e te dá um `match`:

```rust
impl User {
    #[accessor]
    pub fn profile(&self) -> Value {
        match self.profile_loaded() {
            Some(profile) => serde_json::to_value(profile).unwrap_or(Value::Null),
            None => Value::Null,
        }
    }
}
```

**2. A camada de recursos JSON:API.** Quando a forma da relação e a
política de inclusão pertencem ao formato de rede em vez de ao model,
use uma struct `#[derive(Data)] #[json_resource]` com
`#[data(allow_include)]` no campo de relacionamento. Clientes optam
por isso via `?include=posts.comments`, o framework percorre a árvore
de include, e popula `included` com objetos de recurso deduplicados.
Esta é a resposta certa quando:

- A forma da relação é uma preocupação de formato de rede (sparse
  fieldsets, inclusão condicional, metadados de cross-link).
- Endpoints diferentes querem inclusões padrão diferentes.
- O mesmo model aparece sob envelopes diferentes (um endpoint entrega
  `posts`, outro entrega `subscriptions`).

Veja [Recursos
JSON:API](eloquent-resources.md#compound-documents--include-chains)
para o padrão completo.

## E o JSON:API?

O pipeline `to_array()` e a facade `Resource` / `JsonApi` são duas
camadas, e servem trabalhos diferentes:

| Preocupação | `Model::to_array` | `Resource::single` / `JsonApi::single` |
|---|---|---|
| **Formato** | Objeto plano - nomes de coluna mapeiam direto para chaves | Envelope JSON:API (`data`, `included`, `meta`, `links`, `jsonapi`) |
| **Controle por atributo** | `hidden` / `visible` / `appends` em `#[model]` | `#[data(input_only)]`, `Maybe<T>`, sparse fieldsets via `?fields[type]=` |
| **Relações** | Manual (acessador + appends, veja acima) | De primeira classe via `#[data(allow_include)]` + `?include=` |
| **Paginação** | Envolva um `Vec<Value>` manualmente | `Resource::paginated(p)` cuida de links + meta |
| **Erros** | Renderiza através de `FrameworkError` | `into_json_api_response()` produz o envelope `errors` do JSON:API |
| **Quando usar** | Endpoints simples, ferramentas internas, formatos ad-hoc | APIs públicas, consumidores terceiros, clientes que entendem JSON:API |

`to_array()` é a camada mais baixa - é o que é chamado na maioria dos
handlers internos, páginas de admin, props do Inertia (via serde), e
testes. A camada JSON:API se compõe por cima: ela não substitui
`to_array`, adiciona um envelope em torno de lógica de
atributo/relacionamento por recurso que é rica demais para viver no
próprio model.

Para props tipadas do Inertia, você quase sempre quer a camada de
recursos ou um DTO dedicado `#[derive(Serialize)]` com campos
explícitos em vez de fazer o model passar direto pelo serde. Retornos
do Inertia recebem o mesmo tratamento de bypass do serde que qualquer
outra coisa - o caminho seguro é "monte um DTO, preencha-o a partir de
`to_array()`, retorne o DTO".

## Onde cada peça vive

| Preocupação | Arquivo |
|---|---|
| `Model::to_array` / `to_json` padrões do trait | `framework/src/eloquent/model.rs` |
| `Model::to_array_except` / `to_array_only` | `framework/src/eloquent/model.rs` |
| `Model::__append_accessor` padrão do trait | `framework/src/eloquent/model.rs` |
| Override de `to_array` emitido pela macro (pipeline de filtros) | `suprnova-macros/src/model/serialization.rs` |
| Dispatcher de `__append_accessor` emitido pela macro | `suprnova-macros/src/model/serialization.rs` |
| `Collection<M>::to_array` / `to_json` | `framework/src/eloquent/collection.rs` |
| `EagerLoadCache` (o campo `__eager`) | `framework/src/eloquent/relations/eager_cache.rs` |
| Parsing de `hidden` / `visible` / `appends` pela macro | `suprnova-macros/src/model/parse.rs` |
| Macro de nível de função `#[accessor]` | `suprnova-macros/src/lib.rs` |

## Próximos passos

- [API Eloquent](eloquent.md) - a superfície completa do model, a
  referência de attributes, e onde `#[accessor]` / `#[mutator]` são
  definidos
- [Recursos JSON:API](eloquent-resources.md) - a camada declarativa
  de recursos para formas mais ricas por visualizador, sparse
  fieldsets, e documentos compostos `?include=`
- [Validação](validation.md) - como a entrada da solicitação se torna
  uma struct tipada antes que a camada de model a veja
- [Respostas](responses.md) - builders de `HttpResponse`, headers, e
  cookies; a superfície que `json_response!` produz no final
- [Modelo de erros](error-model.md) - como um erro se torna um corpo
  JSON com a mesma correlação de `request_id` do caminho de sucesso

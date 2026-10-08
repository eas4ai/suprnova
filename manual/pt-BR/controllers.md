# Controladores

Um controlador Suprnova é apenas uma função async. Ela recebe da
solicitação o que precisar - parâmetros de caminho tipados, um modelo já
carregado, um formulário validado - e retorna um `Response`. Não há
classe base de controlador. Não há arquivo de fiação de service locator.
A função é a unidade, e o attribute `#[handler]` a cola nas macros de
roteamento.

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// GET /users/{user}
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

A assinatura desse handler faz três coisas de uma vez: declara o
parâmetro de rota (`user`), puxa a linha do banco de dados, e responde
404 se a linha não estiver lá. Nada disso é escrito à mão. `#[handler]`
lê os tipos dos argumentos e gera a extração.

## Gerando um controlador

```bash
suprnova make:controller User
```

Isso escreve `src/controllers/user.rs` com um único stub `invoke` e
adiciona `pub mod user;` a `src/controllers/mod.rs`. O stub é o handler
mínimo viável:

```rust
//! User controller

use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn invoke(_req: Request) -> Response {
    json_response!({
        "controller": "User"
    })
}
```

Adicione ao arquivo quantas funções quiser - o Suprnova não rastreia
"classes" de controlador, apenas funções. Muitas apps dividem por
recurso (`controllers::user::{index, show, store, update, destroy}`),
mas nada no framework obriga a isso.

O nome é convertido para `snake_case` no nome do arquivo: `OrderItem`
vira `order_item.rs`.

## O attribute `#[handler]`

A macro classifica o tipo de cada parâmetro e gera o extractor
correspondente. Quatro categorias:

| Tipo do parâmetro | Extraído via | Modo de falha |
|---|---|---|
| `Request` | passa a solicitação adiante sem alterações | - |
| `i32`, `i64`, `u32`, `u64`, `usize`, `String` | `FromParam` - faz parse do parâmetro de rota de mesmo nome | 400 em falha de parse, 400 quando ausente |
| `T: AutoRouteBinding` (qualquer `Model` do Eloquent) | faz parse do parâmetro como a chave primária do modelo e carrega a linha | 400 em falha de parse, 404 se não for encontrada |
| Qualquer outra coisa (`T: FromRequest`) | chama `T::from_request(req)` - tipicamente um validador `#[derive(FormRequest)]` | o que `from_request` retornar; 422 para erros de validação |

A macro executa as extrações na ordem de declaração, então o corpo da
sua função enxerga valores totalmente tipados. Se qualquer extração
falhar, o erro faz short-circuit via `?` e o corpo do handler nunca
executa.

### Parâmetros de caminho

```rust
// Rota: get!("/users/{id}", controllers::user::show)
#[handler]
pub async fn show(id: i64) -> Response {
    json_response!({ "user_id": id })
}

// Rota: get!("/posts/{post_id}/comments/{comment_id}", show_comment)
#[handler]
pub async fn show_comment(post_id: i64, comment_id: i64) -> Response {
    json_response!({
        "post_id": post_id,
        "comment_id": comment_id,
    })
}
```

O nome do argumento precisa corresponder ao placeholder da rota: `{id}`
exige `id: …`. O tipo do argumento é parseado via `FromParam`. Uma
entrada ruim (`/users/abc` contra `id: i64`) retorna 400 com uma
mensagem que nomeia o parâmetro e o tipo alvo.

### Binding de modelo de rota

Modelos `Eloquent` implementam `AutoRouteBinding` automaticamente.
Declare o modelo como argumento e o framework o carrega:

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// Rota: get!("/users/{user}", controllers::user::show)
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

O nome do placeholder da rota (`{user}`) e o nome do argumento (`user`)
precisam corresponder. O framework faz parse da string do parâmetro como
o tipo da chave primária do modelo, chama `Entity::find_by_pk`, e
retorna 404 se a linha estiver faltando. Qualquer struct
`#[suprnova::model]` faz binding automaticamente; a macro
`route_binding!` continua disponível para entities SeaORM escritas à mão
que não usam `#[suprnova::model]` - veja
[Macros](macros.md#route_binding).

### Form requests

Qualquer coisa que implemente `FromRequest` se encaixa do mesmo jeito. O
caso comum é um struct `#[derive(FormRequest)]` que valida o corpo da
solicitação e, na falha, expõe um 422 com os erros chaveados por campo:

```rust
use suprnova::{attrs, handler, json_response, Response};
use crate::models::user;
use crate::requests::UpdateUserRequest;

// Rota: put!("/users/{user}", controllers::user::update)
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! { name: form.name, email: form.email }).await?;
    json_response!({ "updated": id })
}
```

Veja [Form Requests](requests.md) para o derive de validação e o
pipeline de validação completo.

### Quando você quer o `Request` cru

Se preferir extrair as coisas à mão - ou se precisar de um header, um
cookie, uma query string - receba `Request` diretamente:

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;             // param de rota, 400 se faltar
    let ua = req.header("User-Agent");      // Option<&str>
    let page: u32 = req.query_param("page") // Option<String>
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    json_response!({ "id": id, "ua": ua, "page": page })
}
```

Você pode misturar e combinar: `pub async fn nested(category_id: i64, product: product::Model, req: Request)` é uma assinatura válida. A macro extrai cada argumento pela regra que couber a ele.

## O contrato `Response`

`Response` é um alias para `Result<HttpResponse, HttpResponse>`. Os dois
ramos carregam o mesmo tipo de payload, e é por isso que `?` funciona em
todo lugar. A chain de middleware colapsa o resultado com uma linha na
fronteira:

```rust
result.unwrap_or_else(|e| e)
```

Esse é o mesmo contrato de que todo ponto de propagação de `?` depende.
Erros são convertidos por `From<FrameworkError> for HttpResponse` antes
de chegarem à chain - veja [Modelo de erros](error-model.md) para o
quadro completo.

O corpo de um handler se lê de cima para baixo e usa `?` para abortar:

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

#[handler]
pub async fn show(id: i64) -> Response {
    let user = user::Model::find_or_fail(id).await?;
    let invoices = user.invoices().get().await?;
    json_response!({ "user": user, "invoices": invoices })
}
```

Se `find_or_fail` retornar `Err`, a função sai com um 404. Se
`invoices().get()` der erro, você recebe um 500. Sem instruções `match`,
sem tratadores de exceção.

## Criando respostas

Três macros e um builder cobrem os casos comuns:

```rust
use suprnova::{handler, json_response, text_response, HttpResponse, Response, ResponseExt};

#[handler]
pub async fn json_handler() -> Response {
    json_response!({
        "users": [
            {"id": 1, "name": "John"},
            {"id": 2, "name": "Jane"},
        ]
    })
}

#[handler]
pub async fn health() -> Response {
    text_response!("OK")
}

#[handler]
pub async fn store() -> Response {
    // Status / headers encadeáveis embutidos, via ResponseExt.
    json_response!({ "id": 1, "created": true }).status(201)
}

#[handler]
pub async fn page() -> Response {
    Ok(HttpResponse::html("<h1>Hello</h1>"))
}
```

`json_response!`, `text_response!`, e `HttpResponse::*` produzem todos o
mesmo tipo `Response`. A trait `ResponseExt` acrescenta `.status(...)`,
`.header(...)`, `.cookie(...)`, e `.with_headers(...)` para que você
possa encadear configuração sobre o resultado de uma macro.

Para todo o resto - downloads de arquivos, corpos em streaming,
respostas Inertia, redirecionamentos - veja [Respostas](responses.md).

## Redirecionamentos

`redirect!("route.name")` valida em tempo de compilação que a rota
existe e retorna um builder no qual você pode encadear configuração:

```rust
use suprnova::{handler, redirect, Response};

#[handler]
pub async fn store() -> Response {
    // Cria o usuário…
    redirect!("users.index").into()
}

#[handler]
pub async fn update(id: i64) -> Response {
    redirect!("users.show")
        .with("id", id.to_string())
        .into()
}

#[handler]
pub async fn search() -> Response {
    redirect!("users.index")
        .query("page", "1")
        .query("sort", "name")
        .into()
}
```

`.with(key, value)` preenche um placeholder de rota; `.query(key,
value)` acrescenta um parâmetro de query string; `.flash(key, value)`
escreve no flash bag da sessão para a próxima solicitação. `.into()`
converte o builder em um `Response`.

Se a rota nomeada não existir, a macro falha a compilação com uma lista
dos nomes de rota disponíveis - erros de digitação aparecem antes do
staging.

## Serviços injetados pelo contêiner

Resolva serviços do contêiner com `App::resolve` (tipos concretos) ou
`App::resolve_make` (trait objects). Ambos retornam
`Result<_, FrameworkError>`, então se compõem com `?`:

```rust
use suprnova::{handler, json_response, App, Response};
use crate::services::UserService;

#[handler]
pub async fn index() -> Response {
    let user_service = App::resolve::<UserService>()?;
    let users = user_service.list_all().await?;
    json_response!({ "users": users })
}
```

Se você vincula ações com `#[injectable]`, é assim que um controlador
as chama. Veja [Ações](actions.md) para a forma de uma ação, e
[Contêiner de serviços](container.md) para a superfície completa do
contêiner - vinculação, factories, a cascata de lookup task-local /
thread-local / global.

## Um controlador RESTful na prática

```rust
// src/controllers/user.rs
use suprnova::{attrs, handler, json_response, redirect, Response, ResponseExt};
use crate::models::user;
use crate::requests::{StoreUserRequest, UpdateUserRequest};

// GET /users
#[handler]
pub async fn index() -> Response {
    let users = user::Model::all().await?;
    json_response!({ "users": users })
}

// GET /users/{user}
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({ "user": user })
}

// POST /users
#[handler]
pub async fn store(form: StoreUserRequest) -> Response {
    let user = user::Model::create(attrs! {
        name: form.name,
        email: form.email,
    }).await?;
    json_response!({ "user": user }).status(201)
}

// PUT /users/{user}
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! {
        name: form.name,
        email: form.email,
    }).await?;
    json_response!({ "updated": id })
}

// DELETE /users/{user}
#[handler]
pub async fn destroy(user: user::Model) -> Response {
    user.delete().await?;
    redirect!("users.index").into()
}
```

Registre-os com a macro `routes!`:

```rust
// src/routes.rs
use suprnova::{delete, get, post, put, routes};
use crate::controllers;

routes! {
    get!("/users",           controllers::user::index   ).name("users.index"),
    get!("/users/{user}",    controllers::user::show    ).name("users.show"),
    post!("/users",          controllers::user::store   ).name("users.store"),
    put!("/users/{user}",    controllers::user::update  ).name("users.update"),
    delete!("/users/{user}", controllers::user::destroy ).name("users.destroy"),
}
```

O placeholder de rota `{user}` corresponde ao nome do argumento `user: user::Model`, e é assim que o framework sabe qual segmento do caminho carrega o modelo.

## A API `Request`

Os métodos que você mais vai usar quando receber `Request` diretamente:

| Método | Retorna | Observações |
|---|---|---|
| `method()` | `&hyper::Method` | Método HTTP |
| `path()` | `&str` | Caminho da URL |
| `param(name)` | `Result<&str, ParamError>` | param de rota; `?` para abortar |
| `params()` | `&HashMap<String, String>` | todos os params de rota |
| `query()` | `Option<&str>` | query string crua |
| `query_param(key)` | `Option<String>` | um único valor de query string |
| `query_params()` | `HashMap<String, String>` | todos os params de query |
| `query_into::<T>()` | `Result<T, FrameworkError>` | deserialização tipada |
| `header(name)` | `Option<&str>` | um único header |
| `headers()` | `&hyper::HeaderMap` | o mapa de headers completo |
| `has_header(name)` | `bool` | verificação de presença |
| `bearer_token()` | `Option<String>` | `Authorization: Bearer …` já parseado |
| `cookie(name)` | `Option<String>` | o valor de um único cookie |
| `cookies()` | `HashMap<String, String>` | todos os cookies |
| `ip()` | `Option<String>` | IP do peer, ciente de X-Forwarded-For |
| `secure()` | `bool` | detecção de HTTPS (incl. proxies) |
| `is_method(m)` | `bool` | insensível a maiúsculas e minúsculas |
| `is_inertia()` | `bool` | header de XHR do Inertia |
| `ajax()` | `bool` | `X-Requested-With: XMLHttpRequest` |
| `expects_json()` / `wants_json()` | `bool` | inspeção do header Accept |
| `route_name()` | `Option<String>` | o `.name(...)` da rota correspondida |
| `json::<T>()` | `Result<T, FrameworkError>` | faz parse do corpo como JSON (consome) |
| `form::<T>()` | `Result<T, FrameworkError>` | faz parse como form-urlencoded |
| `input::<T>()` | `Result<T, FrameworkError>` | parse despachado pelo content-type |

Esta é uma superfície com a forma do Laravel - todo método aqui espelha
um método da classe `Request` do Laravel.

## Layout de arquivos

Convenção:

```
src/
├── controllers/
│   ├── mod.rs          # pub mod home; pub mod user; ...
│   ├── home.rs
│   ├── user.rs
│   └── api/
│       ├── mod.rs
│       └── user.rs
├── routes.rs           # routes! { ... }
└── main.rs
```

Nada no framework obriga esse layout - controladores podem viver em
qualquer lugar alcançável a partir de `routes.rs`. A convenção existe
porque é o que o scaffolding emite e porque rotas e controladores são o
par natural.

## Por que Suprnova diverge

Controladores do Laravel são classes que estendem
`Illuminate\Routing\Controller`. Os métodos são chamados em instâncias
que o contêiner resolve a cada solicitação, que é onde a injeção via
construtor acontece. O padrão funciona bem em PHP - dar `new` a cada
solicitação é barato quando o processo inteiro é derrubado depois da
resposta.

Em Rust, esse padrão significaria ou (a) alocar um struct de controlador
por solicitação, o que custa um clone de `Arc` de que você não precisa,
ou (b) reimplementar injeção de dependência através de uma hierarquia de
classes base que não se paga.

O Suprnova escolhe o modelo mais simples: um controlador é uma função
async livre, e "dependências" são ou resoluções do contêiner
(`App::resolve::<Service>()?`) ou argumentos tipados por extração
(`form: UpdateUserRequest`). A injeção via construtor acontece na
fronteira do `#[injectable]` em [Ações](actions.md), que é onde ela cabe.
O handler continua sendo uma função pura de solicitação para resposta, o
que torna trivial testá-lo isoladamente: monte um `Request`, chame a
função, faça asserções sobre o resultado.

## Próximos passos

- [Roteamento](routing.md) - no que `routes!`, `get!`, `post!`, e `.name()` se expandem
- [Form Requests](requests.md) - validação tipada via `#[derive(FormRequest)]`
- [Respostas](responses.md) - JSON, HTML, arquivos, streams, páginas Inertia, redirecionamentos
- [Contêiner de serviços](container.md) - o que `App::resolve` realmente faz
- [Ações](actions.md) - onde a lógica de negócio vive fora do controlador
- [Modelo de erros](error-model.md) - como `?` transforma um `FrameworkError` em uma resposta

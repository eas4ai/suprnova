# Roteamento

Roteamento é como o Suprnova transforma uma solicitação HTTP de entrada
em uma chamada de handler. Você declara suas rotas em `src/routes.rs`
usando a macro `routes!` (ou constrói um `Router` manualmente), e então
`Server::from_config` pega esse router e o executa durante toda a vida
do processo. A mesma forma do `routes/web.php` do Laravel, com tipos
Rust em vez de facades.

```rust
// src/routes.rs
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
    put!("/users/{id}", controllers::users::update).name("users.update"),
    delete!("/users/{id}", controllers::users::destroy).name("users.destroy"),
}
```

A macro se expande para `pub fn register() -> Router { ... }`. Chame-a
a partir da sua inicialização e passe o resultado para o servidor.

## Verbos HTTP

Uma macro por verbo. Todas as sete recebem um par caminho-então-handler
e retornam um builder ao qual você pode encadear `.name(...)` e
`.middleware(...)`.

| Macro | Método | Use para |
|---|---|---|
| `get!`     | GET     | Endpoints de leitura, páginas estáticas |
| `post!`    | POST    | Criar recursos |
| `put!`     | PUT     | Atualizações de substituição completa |
| `patch!`   | PATCH   | Atualizações parciais (RFC 5789) |
| `delete!`  | DELETE  | Destruir |
| `head!`    | HEAD    | Sondas somente de headers (HEAD recai para o registro GET conforme a RFC 9110 § 9.3.2 quando não registrado explicitamente) |
| `options!` | OPTIONS | Descoberta de capacidade, `Accept-Patch`. O preflight de CORS é respondido por `CorsMiddleware` antes do router, então você geralmente não precisa deste |

```rust
use suprnova::{routes, get, post, patch, delete};

routes! {
    get!("/articles", controllers::articles::index),
    post!("/articles", controllers::articles::store),
    patch!("/articles/{id}", controllers::articles::update),
    delete!("/articles/{id}", controllers::articles::destroy),
}
```

Toda macro de verbo verifica em tempo de compilação que o caminho
começa com `/` - uma barra inicial ausente falha o build, não uma
solicitação.

### Multi-método e `any!`

`any!` registra um handler contra todos os sete verbos comuns. Use-o
para receptores de webhook e outros endpoints que precisam aceitar o
que quer que o HTTP envie.

```rust
use suprnova::{routes, any};

routes! {
    any!("/webhooks/inbound", controllers::webhooks::inbound)
        .name("webhooks.inbound")
        .middleware(SignatureCheck),
}
```

Quando você quer apenas um subconjunto de verbos compartilhando um
handler, use a API de builder e `Router::methods`:

```rust
use suprnova::Router;
use hyper::Method;

let router = Router::new()
    .methods(&[Method::PUT, Method::PATCH], "/posts/{id}", update_post)
    .name("posts.update")
    .middleware(AuthMiddleware);
```

`.name(...)` e `.middleware(...)` se propagam por todos os verbos com
os quais a rota foi registrada, então o lookup reverso produz a mesma
URL não importa qual método o chamador consulte.

### Rotas WebSocket

`ws!` registra um handler de upgrade de vida longa. A macro faz parte
do mesmo corpo `routes!` - coberta em detalhe em
[WebSockets](websockets.md).

## Parâmetros de rota

Segmentos dinâmicos usam chaves (`{id}`). Por familiaridade o Suprnova
também aceita dois-pontos no estilo Express/Rails (`:id`) e os
normaliza para chaves antes de passar o padrão para o `matchit`.

```rust
routes! {
    get!("/users/{id}", controllers::users::show),       // nativo do matchit
    get!("/users/:id", controllers::users::show),        // Express/Rails - a mesma coisa
    get!("/posts/{post_id}/comments/{comment_id}", controllers::comments::show),
}
```

O dois-pontos só é tratado como um abridor de parâmetro no início de
um segmento de caminho, então dois-pontos literais no meio do segmento
sobrevivem intactos (`/files/note:draft` continua sendo uma rota
literal, não `/files/{draft}`).

Leia parâmetros da solicitação dentro de um handler:

```rust
use suprnova::{Request, Response, HttpResponse};

pub async fn show(req: Request) -> Response {
    let user_id = req.param("id").unwrap_or("0");
    Ok(HttpResponse::text(format!("User ID: {}", user_id)))
}
```

Para extração tipada sem a dança do `unwrap_or`, veja o binding de
modelo de rota abaixo ou `#[handler]` em [Controladores](controllers.md).

## Binding de modelo de rota

Quando um parâmetro de handler é um tipo `*::Model` do SeaORM,
`#[handler]` extrai o parâmetro de caminho correspondente, faz parse
dele como o tipo da chave primária, e busca a linha no banco de dados.
Uma linha ausente produz 404; um parâmetro que o tipo da PK não
consegue fazer parse produz 400.

```rust
use suprnova::{handler, json_response, Response};
use crate::models::users;

// Rota: GET /users/{user}
#[handler]
pub async fn show(user: users::Model) -> Response {
    json_response!({ "name": user.name, "email": user.email })
}
```

O nome do parâmetro (`user`) é o que `#[handler]` procura nos params
da rota correspondida - então o placeholder precisa corresponder
(`/users/{user}`, não `/users/{id}`).

Múltiplos modelos em uma assinatura funcionam da mesma forma;
misture-os com form requests, primitivos, ou `Request`:

```rust
// Rota: PUT /posts/{post}/comments/{comment}
#[handler]
pub async fn update(
    post: posts::Model,
    comment: comments::Model,
    form: UpdateCommentRequest,
) -> Response {
    // post e comment já foram buscados; form já foi validado.
    json_response!({ "post_id": post.id, "comment_id": comment.id })
}
```

### Requisitos

O binding é automático para qualquer modelo SeaORM cuja `Entity`
implemente `suprnova::database::EntityExt` e cujo tipo de chave
primária implemente `FromStr`. As traits adicionais amigáveis a
blanket-impl do `EntityExt` dão a você `Entity::find_by_pk(id)`,
`::all()`, `::first()`, e afins; o binding de modelo de rota é apenas
`find_by_pk` guiado pelo parâmetro de caminho.

```rust
// src/models/users.rs (o layout legado estilo SeaORM)
pub use super::entities::users::*;
use sea_orm::entity::prelude::*;

impl ActiveModelBehavior for ActiveModel {}

// Habilita o binding de modelo de rota (e a superfície de leitura no
// formato Laravel).
impl suprnova::database::EntityExt for Entity {}
impl suprnova::database::EntityExtMut for Entity {}
```

Se seu modelo é declarado com a macro `#[suprnova::model]` (a
superfície Eloquent em [Eloquent](eloquent.md)), você o usa
diretamente: `User::find_by_pk(id).await?`. O binding de modelo de
rota via `#[handler]` ainda espera a forma `*::Model` - passe o tipo
de modelo SeaORM, não o struct wrapper.

### Binding é identidade, não autorização

O binding de modelo de rota responde "esta linha existe?" - ele
**não** responde "o usuário atual tem permissão para ver esta linha?".
Um handler vinculado nu deixa qualquer usuário autenticado ver
qualquer post adivinhando `/posts/N`. Autorize contra o modelo
vinculado usando `Gate::authorize` ou a macro `#[policy]` - veja
[Autorização](authorization.md).

### Optando por não usar

Não use o tipo de parâmetro `*::Model`. Extraia o ID e consulte
manualmente:

```rust
use suprnova::{handler, json_response, Response, FrameworkError};
use crate::models::users;
use suprnova::database::EntityExt;

#[handler]
pub async fn show(id: i32) -> Response {
    let user = users::Entity::find_by_pk(id)
        .await?
        .ok_or(FrameworkError::not_found("User"))?;
    json_response!({ "id": user.id, "name": user.name })
}
```

## Rotas nomeadas

Nomes dão a você identificadores estáveis para geração de URL. Anexe
um com `.name(...)`:

```rust
routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

Nomes seguem a convenção do Laravel `<resource>.<action>` -
`users.show`, `posts.destroy`, `admin.dashboard`. Procure-os com o
helper de nível superior `route(name, &[...])`:

```rust
use suprnova::route;

let home = route("home", &[]);
//   Some("/")

let profile = route("users.show", &[("id", "123")]);
//   Some("/users/123")
```

`route` retorna `Option<String>` e faz percent-encode dos valores de
parâmetro para uma forma segura para caminho (então `("slug", "a/b")`
se torna `/posts/a%2Fb` - seguro para o matchit e faz round-trip
através de `req.param("slug")`). Para alvos de redirecionamento e
links de email use o irmão estrito `suprnova::routing::try_route`, que
retorna `Result<String, RouteUrlError>` e se recusa a emitir uma URL
contendo um segmento `{placeholder}` não preenchido. Veja
[Geração de URLs](urls.md) para a superfície de URL completa (URLs
assinadas, URLs absolutas, `Redirect::route`).

Nomes de rota são globalmente únicos e process-global. Registrar o
mesmo nome para dois caminhos diferentes causa panic na inicialização -
shadowing silencioso era um bug com formato de segurança porque
redirecionamentos rotiariam para qualquer registro que por acaso
vencesse. Use `RouteBuilder::try_name` (ou
`suprnova::routing::try_register_route_name`) para a variante falível.

## Middleware por rota

Encadeie `.middleware(M)` em qualquer builder de rota:

```rust
use suprnova::{routes, get, post};
use crate::middleware::{AuthMiddleware, AdminMiddleware};

routes! {
    // Pública
    get!("/", controllers::home::index).name("home"),

    // Protegida
    get!("/dashboard", controllers::dashboard::index)
        .name("dashboard")
        .middleware(AuthMiddleware),

    // Múltiplos middleware se compõem da esquerda para a direita (mais externo primeiro)
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware)
        .middleware(AdminMiddleware),
}
```

Middleware local à rota executa depois de qualquer middleware global
(`Server::with_middleware`) e qualquer middleware de grupo que envolve
a rota. O mapa de middleware é chaveado por `(method, path)`, então
anexar auth a `POST /api/posts` nunca vaza para um `GET /api/posts`
público no mesmo caminho. Para o contrato de middleware e como
escrever o seu, veja [Middleware](middleware.md).

## Grupos de rotas

`group!` extrai um prefixo de caminho compartilhado e/ou middleware
compartilhado:

```rust
use suprnova::{routes, get, post, group};
use crate::middleware::{AuthMiddleware, ApiMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Prefixo /api compartilhado + middleware
    group!("/api", {
        get!("/users", controllers::api::users::index).name("api.users.index"),
        post!("/users", controllers::api::users::store).name("api.users.store"),
        get!("/users/{id}", controllers::api::users::show).name("api.users.show"),
    }).middleware(ApiMiddleware),

    // Área admin
    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard).name("admin.dashboard"),
        get!("/settings", controllers::admin::settings).name("admin.settings"),
    }).middleware(AuthMiddleware),
}
```

Um prefixo de grupo é concatenado com cada caminho de rota. Uma rota
em `/` dentro de um grupo resolve exatamente para o prefixo do grupo
(`group!("/users", { get!("/", index) })` → `GET /users`).

### Grupos aninhados

Grupos se aninham a qualquer profundidade. Prefixos concatenam;
middleware é herdado do pai para o filho:

```rust
routes! {
    group!("/api", {
        get!("/health", controllers::api::health),

        group!("/v1", {
            get!("/users", controllers::api::v1::users),

            group!("/admin", {
                get!("/stats", controllers::admin::stats),
            }).middleware(AdminMiddleware),
        }),
    }).middleware(AuthMiddleware),
}
```

| Rota | Caminho efetivo | Chain de middleware |
|---|---|---|
| `/api/health` | `/api/health` | `AuthMiddleware` |
| `/api/v1/users` | `/api/v1/users` | `AuthMiddleware` |
| `/api/v1/admin/stats` | `/api/v1/admin/stats` | `AuthMiddleware` → `AdminMiddleware` |

Para uma única rota dentro de um grupo aninhado, a ordem de execução
é **middleware mais externo primeiro**: grupo pai → grupo filho →
local à rota. O `.middleware(...)` por rota executa mais internamente.

## Rota de fallback

`fallback!` registra um handler que executa quando nenhuma outra rota
corresponde. Use-o para páginas 404 customizadas.

```rust
use suprnova::{routes, get, fallback};

routes! {
    get!("/", controllers::home::index),

    fallback!(controllers::errors::not_found),
}
```

```rust
// src/controllers/errors.rs
use suprnova::{Request, Response, HttpResponse};

pub async fn not_found(req: Request) -> Response {
    Ok(HttpResponse::text(format!("Page not found: {}", req.path()))
        .status(404))
}
```

O fallback suporta sua própria chain de middleware
(`fallback!(handler).middleware(M)`). Se nenhum fallback for
registrado, o framework retorna um `404 Not Found` em texto puro.

## Roteamento de recursos

Para uma superfície REST padrão de 7 ações, implemente
`ResourceController` e registre o recurso através do builder
`Router`. Paridade Laravel para `Route::resource()` e
`Route::apiResource()`.

```rust
use suprnova::{Router, ResourceController, ResourceAction, Request, Response, HttpResponse};
use std::pin::Pin;
use std::future::Future;

struct PostsCtl;

impl ResourceController for PostsCtl {
    fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("list")) })
    }
    fn show(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("one")) })
    }
    // store / update / destroy / create / edit têm 404 como padrão.
}

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .into();
```

Métodos que você não sobrescreve retornam 404. Use `api_resource` para
descartar `create` e `edit` - as duas rotas que existem apenas para
renderizar formulários.

### Rotas e nomes padrão

| Verbo | Caminho | Método da trait | Nome |
|---|---|---|---|
| GET    | `/posts`             | `index`   | `posts.index`   |
| GET    | `/posts/create`      | `create`  | `posts.create`  |
| POST   | `/posts`             | `store`   | `posts.store`   |
| GET    | `/posts/{post}`      | `show`    | `posts.show`    |
| GET    | `/posts/{post}/edit` | `edit`    | `posts.edit`    |
| PUT    | `/posts/{post}`      | `update`  | `posts.update`  |
| DELETE | `/posts/{post}`      | `destroy` | `posts.destroy` |

O parâmetro de caminho tem como padrão o singular do nome do recurso -
`posts` → `{post}`, `categories` → `{category}`. Plurais irregulares
recebem o último segmento literal; sobrescreva com `.parameter(...)`.

### Restringindo e renomeando

```rust
use suprnova::{Router, ResourceAction};

Router::new()
    .resource("posts", PostsCtl)
    .only(&[ResourceAction::Index, ResourceAction::Show])      // fixa em dois verbos
    .names([("index", "posts.list")])                          // renomeia um padrão
    .parameter("post_id")                                      // {post} → {post_id}
    .into();
```

Aliases do lado Rust que se leem melhor em alguns call sites:
`.keep(...)` para `.only(...)`, `.drop(...)` para `.except(...)`,
`.rename(...)` para `.names(...)`.

### Registro em lote

```rust
Router::new()
    .resources([
        ("posts",    Box::new(PostsCtl)    as Box<dyn ResourceController>),
        ("comments", Box::new(CommentsCtl) as Box<dyn ResourceController>),
    ])
    .api_resources([("authors", Box::new(AuthorsCtl) as Box<dyn ResourceController>)]);
```

### Autorizando o recurso inteiro

`authorize_resource::<U, R>()` anexa a verificação de ability
convencional a cada rota gerada como middleware por rota - paridade
com o `authorizeResource` do Laravel. Sem ela, uma superfície de
recurso fica sem proteção a menos que todo corpo de controlador se
lembre de chamar `Gate::authorize`; um único `destroy` esquecido
lança um delete sem proteção.

```rust
use suprnova::{Router, Gate};

// Abilities são chaveadas em (ability, tipo de usuário, tipo marcador de recurso).
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()
    .into();
```

O mapeamento ação → ability espelha o Laravel:

| Ação(ões) | Ability |
|---|---|
| `index`, `show`     | `view`   |
| `create`, `store`   | `create` |
| `edit`, `update`    | `update` |
| `destroy`           | `delete` |

`PATCH` compartilha a ação `update`, então é protegida identicamente a
`PUT`. Uma ability negada faz short-circuit com `403` antes do
handler executar, e uma solicitação não-autenticada falha de forma
fechada. O marcador de recurso `R` só precisa de `Default` - o gate
discrimina pelo seu *tipo*, da mesma forma que o Laravel discrimina
pela classe do modelo. Veja o
[capítulo de autorização](authorization.md) para definir as próprias
abilities.

## Redirecionamentos e views em nível de router

Três métodos de açúcar no `Router` cobrem declarações de rota que não
precisam de uma função handler:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new()
    // Redirecionamento estático: GET /old-pricing → 302 /pricing
    .redirect("/old-pricing", "/pricing", 302)
    // Irmão 301
    .permanent_redirect("/legacy", "/new")
    // Página estática Inertia: GET /about renderiza o componente About
    .inertia("/about", "About", json!({ "team_size": 4 }))
    .name("about");
```

`Router::inertia` é o `Route::inertia($uri, $component, $props)` do
Suprnova. Ele registra `GET`; uma solicitação `HEAD` cai nele e tem o
corpo removido no limite do servidor, então não há nada extra para
registrar. Ele retorna um `RouteBuilder`, então `.name(...)` e
`.middleware(...)` podem ser encadeados nele como em qualquer outra rota.

Props devem ser um objeto JSON ou `null` para nenhum. Qualquer outra coisa -
um array, uma string - é erro de registro, não um saco de props
silenciosamente vazio. `try_inertia` é a forma falível.

`Router::view` é o mesmo método sob seu nome antigo; ele retorna `Router`
em vez de `RouteBuilder`, então uma rota declarada com ele não pode receber
nome. Prefira `inertia`.

### Por que Suprnova diverge

O `Route::view` do Laravel renderiza um template Blade; o Suprnova renderiza
um componente Inertia, porque o sistema de templates do framework é Inertia,
não Blade. Uma consequência: o nome do componente é uma string em tempo de
execução aqui, então não recebe a verificação de componente de página em
tempo de compilação que a macro `inertia_response!` executa. Escreva o handler
com `inertia_response!` quando quiser que um erro de digitação no nome do
componente falhe no build, em vez de na solicitação.

Para *respostas* de redirecionamento (não declarações de rota) -
`Redirect::route`, `Redirect::back`, `Redirect::intended`,
redirecionamentos assinados - veja [Geração de URLs](urls.md) e
[Respostas](responses.md).

## URLs assinadas

Rotas assinadas com HMAC são adjacentes ao roteamento (você cunha uma
URL contra uma rota nomeada, depois verifica a assinatura na
solicitação de entrada). Elas são cobertas por completo em
[Geração de URLs](urls.md); a versão curta:

```rust
use suprnova::url;

let reset = url::signed_route("password.reset", &[("user", "42")])?;
// /password/reset/42?signature=...

let expires_at = chrono::Utc::now().timestamp() + 3600;
let verify = url::temporary_signed_route("verify.email", &[("user", "42")], expires_at)?;
// /verify/email/42?expires=1748803600&signature=...
```

Verifique dentro de um handler com `url::has_valid_signature(&request)`
(booleano) ou `url::signature_verdict(&request)` (a divisão de três
vias `Valid`/`Expired`/`Invalid`, para que você possa renderizar uma
página de "solicite um link novo" em vez de um 403 genérico).

## Registro falível

O registro de rotas executa uma vez na inicialização, então uma rota
duplicada ou malformada é tratada como um erro do programador: os
helpers simples (`Router::get`, `post`, `put`, `delete`, `ws`,
`RouteBuilder::name`, a conversão `From` de `GroupBuilder` → `Router`)
fazem **panic** para falhar de forma explícita na inicialização. Esse é o
padrão certo para rotas declaradas no código-fonte.

Quando padrões ou nomes vêm de uma fonte falível - config dinâmica, um
sistema de plugins, um teste que deliberadamente registra rotas
conflitantes - use os irmãos `try_*`. Eles retornam
`Result<_, FrameworkError>` (nomeando o método, caminho, ou nome
conflitante infrator) em vez de fazer panic:

| Que faz panic | Irmão falível | Retorna |
|---|---|---|
| `Router::get` / `post` / `put` / `patch` / `delete` / `head` / `options` | `try_get` / `try_post` / `try_put` / `try_patch` / `try_delete` / `try_head` / `try_options` | `Result<RouteBuilder, FrameworkError>` |
| `Router::ws` (e toda variante `ws_*`) | `try_ws` (e toda `try_ws_*`) | `Result<Router, FrameworkError>` |
| `RouteBuilder::name` | `try_name` | `Result<Router, FrameworkError>` |
| `GroupBuilder` → `Router` via `.into()` | `GroupBuilder::try_finalize` | `Result<Router, FrameworkError>` |
| `ResourceRoutes::register` | `try_register` | `Result<Router, FrameworkError>` |

```rust
use suprnova::{FrameworkError, Router};

// `path` vem de config dinâmica; um padrão malformado ou duplicado
// é recuperável, não um panic de inicialização.
fn register_dynamic(router: Router, path: &str) -> Result<Router, FrameworkError> {
    Ok(router.try_get(path, health)?.into())
}
```

Uma rota de grupo duplicada é recuperável da mesma forma - porque
`From` não pode ser falível, o irmão falível de `.into()` é o
método inerente `try_finalize`:

```rust
let router: Router = Router::new()
    .group("/api", |r| r.get("/users", list).post("/users", create))
    .try_finalize()?;
```

Os helpers que fazem panic permanecem como válvulas de escape
ergonômicas; os irmãos `try_*` são puramente aditivos.

## Por que Suprnova diverge

**Sintaxe dupla de parâmetro de caminho.** O Laravel usa `{param}`; o
Express usa `:param`. O Suprnova aceita ambos e normaliza `:param`
para `{param}` antes de o caminho chegar ao `matchit`. Ambos os
estilos se compõem com tudo o mais - grupos, binding de modelo, URLs
assinadas. A razão não é indecisão; é que não podemos prever qual
bagagem você traz, e a sintaxe de roteamento é um ponto de atrito
frequente demais para fazer as pessoas reaprenderem.

**Duas APIs co-iguais: macro e builder.** O Laravel entrega uma DSL
(`Route::get(...)`). O Suprnova entrega a macro declarativa
`routes! { ... }` E o builder encadeável
`Router::new().get(...).name(...)`. Ambos produzem registros
idênticos. A macro se lê melhor para tabelas de rotas de nível
superior; o builder se lê melhor quando você está compondo routers
dinamicamente (plugins, rotas geradas, testes). Escolha o que melhor
se encaixa no call site - não há resposta canônica porque ambas as
formas são de primeira classe.

**Panics em boot-time, não shadowing silencioso.** Um nome de rota
duplicado ou colisão de padrão faz panic na inicialização. Os
registros chaveados por array do Laravel deixam silenciosamente o
registro mais recente vencer, o que é aceitável quando seu arquivo de
rotas é o único registrador mas inseguro assim que plugins ou rotas
geradas entram em cena. Os irmãos `try_*` são a válvula de escape
quando falibilidade é o que você realmente quer.

## Próximos passos

- [Controladores](controllers.md) - `#[handler]`, form requests, retornando JSON/Inertia
- [Middleware](middleware.md) - a trait `Middleware`, ordenação, construindo o seu próprio
- [Geração de URLs](urls.md) - URLs de rota nomeada, URLs assinadas, redirecionamentos, `RouteUrlError`
- [Autorização](authorization.md) - gates e policies para modelos vinculados
- [WebSockets](websockets.md) - `ws!`, a trait `WebSocketHandler`, config por rota

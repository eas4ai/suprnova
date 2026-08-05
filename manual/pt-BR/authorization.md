# Autorização

Autenticação responde _"quem é você?"_; autorização responde _"você tem
permissão para fazer isso?"_ O Suprnova traz uma facade `Gate` no formato do
Laravel, mais a macro `#[policy]` para conexão orientada a recursos, com
variantes síncronas e assíncronas de toda verificação, para que a mesma
superfície funcione tanto quando o corpo da sua política precisa de uma
consulta ao BD quanto quando só precisa de uma comparação de campo de
struct.

## Início rápido

```rust
use suprnova::{Authorizable, Gate};

#[derive(Debug)]
struct User { id: i64, is_admin: bool }
#[derive(Debug)]
struct Post { id: i64, author_id: i64, is_public: bool }

// Deixa os usuários optarem pela ergonomia `user.can(action, &resource)`.
impl Authorizable for User {}

// Conecte uma habilidade:
Gate::define::<User, Post>("update", |user, post| {
    user.is_admin || post.author_id == user.id
});

let alice = User { id: 1, is_admin: false };
let own_post = Post { id: 10, author_id: 1, is_public: false };
let foreign_post = Post { id: 11, author_id: 99, is_public: false };

assert!(alice.can("update", &own_post));
assert!(alice.cannot("update", &foreign_post));

// Retorne 403 diretamente de um handler:
alice.authorize("update", &foreign_post)?;
```

## A superfície do `Gate`

### Definindo habilidades

```rust
// Closure síncrona - invocada diretamente, sem future boxado.
Gate::define::<User, Post>("view", |user, post| post.is_public || user.id == post.author_id);

// Closure assíncrona - o future precisa ser owned (sem borrows além do retorno da closure).
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_is_admin = user.is_admin;
    let post_id = post.id;
    async move {
        // ...consulta a BD, chamada RPC, etc.
        user_is_admin || check_publish_permission(post_id).await
    }
});
```

Type-erased internamente; o registro indexa por `(action, TypeId<U>,
TypeId<R>)`. Um gate de ação de `User` e um gate de ação de `Comment` com o
mesmo nome vivem de forma independente - `Gate::has::<User, Post>("publish")`
e `Gate::has::<User, Comment>("publish")` respondem separadamente.

### Verificando habilidades

| Método | Retorna | Uso |
|---|---|---|
| `Gate::allows(action, &user, &resource)` | `bool` | Ramificação rápida |
| `Gate::denies(action, &user, &resource)` | `bool` | Inverso |
| `Gate::authorize(action, &user, &resource)` | `Result<(), FrameworkError>` | 403 em uma negação nua; uma negação rica carrega seu próprio status/mensagem (veja [Decisões ricas](#decisões-ricas-response-inspect-raw)) - faz short-circuit em um handler com `?` |
| `Gate::inspect(action, &user, &resource)` | `Response` | Decisão completa: `allowed` + `message` + `code` + `status` HTTP |
| `Gate::raw(action, &user, &resource)` | `Option<Response>` | Como `inspect`, mas `None` = nenhuma regra definida (versus uma negação explícita) |
| `Gate::any(&[...], &user, &resource)` | `bool` | True se qualquer uma permitir |
| `Gate::none(&[...], &user, &resource)` | `bool` | True se nenhuma permitir |
| `Gate::check(&[...], &user, &resource)` | `bool` | True se todas permitirem |

Todo método tem um irmão `_async` que funciona tanto para gates registrados
de forma síncrona quanto assíncrona, então os handlers não precisam saber
qual tipo de closure sustenta a ação.

### Introspecção

```rust
// Uma habilidade está definida?
Gate::has::<User, Post>("publish");  // bool

// Quais habilidades existem? (ordenadas + sem duplicatas pelo nome da ação)
let all: Vec<String> = Gate::abilities();
```

`abilities()` remove duplicatas entre tipos de recurso: registrar `"view"`
tanto para `User`-em-`Post` quanto para `User`-em-`Comment` produz uma
única entrada `"view"`. Útil para seletores de admin e shared-data do
Inertia.

### Semântica de gate ausente

Chamar `allows` / `denies` / `authorize` em uma ação que nunca foi
registrada **assume negar por padrão**. O mesmo vale para chamar a API
síncrona em um gate registrado de forma assíncrona (o caminho síncrono não
pode fazer await - negar por padrão expõe o bug nos logs via
`tracing::warn!` em vez de passar silenciosamente). Gates registrados de
forma assíncrona respondem corretamente a partir dos caminhos `_async`.

## Políticas com `#[policy]`

Quando um tipo de recurso tem várias habilidades, agrupe-as em uma struct
de política e deixe `#[policy]` registrar cada método como um gate:

```rust
use suprnova::policy;
use suprnova::authorization::Response;

struct User { id: i64, is_admin: bool }
struct Post { id: i64, author_id: i64, is_public: bool }
struct PostPolicy;

#[policy(User, Post)]
impl PostPolicy {
    // Um método `-> bool` é um gate simples de allow/deny.
    fn view_any(_user: &User, _post: &Post) -> bool {
        true // qualquer um pode listar posts
    }
    fn view(user: &User, post: &Post) -> bool {
        post.is_public || post.author_id == user.id || user.is_admin
    }

    // Um método `-> Response` pode carregar uma mensagem + status HTTP na negação.
    fn update(user: &User, post: &Post) -> Response {
        if post.author_id == user.id || user.is_admin {
            Response::allow()
        } else {
            Response::deny_with("You may only edit your own posts.")
        }
    }
    fn delete(user: &User, post: &Post) -> Response {
        if user.is_admin {
            Response::allow()
        } else {
            Response::deny_as_not_found() // esconde o post de não-admins
        }
    }
}
```

Cada método se torna um `inventory::submit!`. `Server::serve` drena o
inventory via `init_policies()` na inicialização, então, no momento em que
a primeira solicitação chega, toda ação já está registrada (veja
[Inicialização da aplicação](bootstrap.md) para onde isso se encaixa na
sequência de boot). `init_policies()` vive em
`suprnova::authorization::init_policies` e é idempotente - chame-a
manualmente em testes que exercitam o registro de política sem levantar um
servidor.

Métodos de política são funções associadas stateless que recebem `(user,
resource)` - o mesmo formato do `update(User $user, Post $post)` do
Laravel, onde `$this` é o objeto de política stateless. Todo método recebe
os dois argumentos para uma assinatura de gate uniforme; `view_any` /
`create` simplesmente ignoram o recurso (`_post`). Métodos que você não
escreve não são registrados, e uma ação não registrada nega por padrão.

### Mapeamento de nome de método → ação

O nome do método é usado diretamente como o segmento de verbo da ação, com
o recurso em kebab-case e sufixado:

| Método | Ação |
|---|---|
| `view` em `Post` | `"view-post"` |
| `view_any` em `Post` | `"view_any-post"` |
| `force_delete` em `UserProfile` | `"force_delete-user-profile"` |

Isso diverge dos nomes de ação em camelCase do Laravel (`viewAny`,
`forceDelete`) para manter a superfície Rust idiomática - toda string de
ação espelha o identificador de método que você autocompletaria no seu
editor.

### Tipo de retorno: `bool` ou `Response`

O tipo de retorno de um método de política seleciona como ele se registra -
e o que uma negação pode carregar:

| Tipo de retorno | Registra via | A negação surge como |
|---|---|---|
| `bool` | `Gate::define` | `403` nu (`This action is unauthorized.`) |
| `Response` | `Gate::define_with` | a mensagem, o code, e o status HTTP que o `Response` carrega |

Retorne `bool` para um simples sim/não. Retorne um `Response` (importado de
`suprnova::authorization::Response`) quando uma negação deve carregar um
motivo ou um status diferente de 403 - `Response::deny_with("…")` para uma
mensagem, ou `Response::deny_as_not_found()` para responder `404` e
esconder a existência do recurso. Ambos compilam para o mesmo gate
type-erased (um `bool` é envolvido em um allow/deny nu). Qualquer outro
tipo de retorno - ou a ausência de um - é um erro de compilação.

## A trait `Authorizable`

Açúcar plug-and-play do lado do usuário para as chamadas de `Gate`:

```rust
use suprnova::Authorizable;

impl Authorizable for User {}

// Açúcar síncrono
if alice.can("update", &post)    { /* ... */ }
if alice.cannot("delete", &post) { /* ... */ }
alice.authorize("update", &post)?;  // 403 na negação

// Açúcar assíncrono
if alice.can_async("publish", &post).await    { /* ... */ }
alice.authorize_async("publish", &post).await?;
```

Todo método tem um corpo padrão que delega para o método `Gate`
correspondente, então `impl Authorizable for User {}` (sem corpo) já é o
suficiente. É opt-in em vez de blanket-impl: nem todo tipo que pode ser
passado para `Gate::allows` deve ser o sujeito de `.can` - na maioria das
vezes é o `User` da sua aplicação.

## Padrões de composição

### Bloqueando grupos de rotas

```rust
use suprnova::{group, get, Auth, AuthMiddleware, FrameworkError, Request, Response};

// O middleware verifica o usuário autenticado; o handler autoriza a ação.
group!("/posts")
    .middleware(AuthMiddleware::new())
    .routes([
        get!("/{id}/edit", edit_form),
    ]);

async fn edit_form(req: Request) -> Response {
    let user: User = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let post = Post::find(id).await?
        .ok_or_else(|| FrameworkError::not_found("Post"))?;
    user.authorize("update", &post)?;
    // ... renderiza o formulário de edição
}
```

### Verificações de várias ações

Uma página de "liste tudo que esse usuário pode fazer nesse recurso":

```rust
let actions = ["view", "update", "delete", "restore", "force_delete"];
let mut allowed = Vec::new();
for action in &actions {
    if user.can(action, &post) {
        allowed.push(*action);
    }
}
// Ou faça short-circuit:
let can_do_anything = Gate::any(&actions, &user, &post);
let is_locked_out   = Gate::none(&actions, &user, &post);
```

### Autorização com múltiplos gates

```rust
// Só permite se o usuário puder fazer TODAS essas ações no recurso.
Gate::authorize_async("publish", &user, &post).await?;
if Gate::check_async(&["update", "view"], &user, &post).await {
    // Combine verificações.
}
```

### Bloqueando rotas de recursos

Quando existe uma superfície `Router::resource`, `authorize_resource::<U,
R>()` conecta a verificação de habilidade convencional nas sete rotas de
uma vez, então você não depende de cada método de controlador lembrar de
autorizar:

```rust
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()   // index/show→view, store→create, …
    .into();
```

Uma habilidade negada retorna `403` antes de o handler executar; uma
solicitação não autenticada falha de forma fechada. A tabela completa de
ação → habilidade vive no [capítulo de roteamento](routing.md).

## Semântica assíncrona

A closure de `Gate::define_async` precisa retornar um future **owned** - o
registro type-erased não pode deixar referências `&user` ou `&resource`
sobreviverem ao retorno da closure. Copie ou clone qualquer campo de que
você precise dentro do bloco `async move {}` antes de retorná-lo:

```rust
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_id = user.id;        // copia primitiva
    let post_id = post.id;
    let admin   = user.is_admin;
    async move {
        // Nenhuma referência a `user` / `post` aqui - só as cópias capturadas.
        admin || check_can_publish(user_id, post_id).await
    }
});
```

Gates síncronos funcionam de forma transparente a partir do caminho
assíncrono (`Gate::allows_async` os despacha sem um `.await`), então uma
base de código pode registrar gates síncronos hoje e migrar habilidades
individuais para assíncrono depois, sem mudar os call sites.

## Postura de envenenamento de lock

O registro do `Gate` usa um `RwLock` internamente. Se o lock chegar a ser
envenenado (uma thread entrou em panic enquanto segurava a guarda de
escrita), o registro **nega por segurança** - toda chamada subsequente a
`authorize` retorna `Unauthorized` em vez de entrar em panic. Chamadas de
registro fazem log em `tracing::error!` e continuam. Isso corresponde à
política mais ampla do framework: um lock envenenado nunca aborta o
processo.

## Decisões ricas: `Response`, `inspect`, `raw`

Um gate `bool` nu responde só allow/deny. Para uma negação que carregue
uma *mensagem*, um *código* de máquina, ou um *status* HTTP diferente de
403, registre o gate com `define_with` (ou `define_async_with`) e retorne
um `Response`:

```rust
use suprnova::authorization::Response;  // reexportado na raiz do crate como `GateResponse`

Gate::define_with::<User, Post>("update", |user, post| {
    if post.author_id == user.id {
        Response::allow()
    } else {
        Response::deny_with("You do not own this post.")
    }
});

// Esconda a existência de um recurso em vez de admitir que ele existe:
Gate::define_with::<User, Secret>("view", |user, secret| {
    if user.can_see(secret) {
        Response::allow()
    } else {
        Response::deny_as_not_found()  // um 404, não um 403
    }
});
```

Inspecione a decisão completa com `Gate::inspect` (síncrono) /
`Gate::inspect_async`:

```rust
let decision = Gate::inspect("update", &user, &post);
decision.allowed();   // bool
decision.message();   // Option<&str> - Some("You do not own this post.")
decision.status();    // Option<u16> - None aqui; Some(404) depois de deny_as_not_found
```

Os construtores de `Response` espelham o Laravel: `allow()`, `deny()`,
`deny_with(msg)`, `deny_with_status(status, msg)`, `deny_as_not_found()`,
além dos builders `with_message` / `with_code` / `with_status` /
`as_not_found`.

### Como uma negação se torna um erro

`Gate::authorize` colapsa a decisão através de `Response::authorize()`:

| Decisão | Resultado de `authorize` |
|---|---|
| permitida | `Ok(())` |
| `deny()` nu (sem message/code/status) | `FrameworkError::Unauthorized` (403, `"This action is unauthorized."`) |
| negação rica (message e/ou status definidos) | `FrameworkError::Domain { message, status_code }` |

Então `deny_as_not_found()` surge como um 404, `deny_with_status(422,
"…")` como um 422, e `deny_with("…")` como um 403 carregando a sua
mensagem. O `code` é legível no `Response` inspecionado, mas **não** viaja
através de `authorize` - `FrameworkError` não tem campo de code; leia-o a
partir de `inspect()` se precisar dele.

### `raw`: "negado" vs "indefinido"

`Gate::raw` (e `raw_async`) retorna `Option<Response>`: `None` significa
*nenhuma regra aplicada* - nenhum hook `before` disparou, nenhum gate está
registrado, nenhum hook `after` preencheu nada - o que é distinto de um
`Some(deny)` explícito. `inspect` normaliza esse `None` para uma negação
padrão; `raw` o preserva para diagnóstico ("essa ação é governada por
alguma regra?").

## Hooks `before` / `after`

`Gate::before` registra uma verificação que executa *antes* de qualquer
gate; o primeiro hook a retornar `Some(decision)` faz short-circuit em
tudo. O uso canônico é uma sobrescrita global:

```rust
// Administradores podem fazer qualquer coisa.
Gate::before::<User>(|user, _action| user.is_admin.then_some(true));
```

`Gate::after` executa *depois* do gate. Seguindo a semântica `??=` do
Laravel, um hook after só pode **preencher** um resultado indeciso (nenhum
gate correspondeu e nenhum hook before disparou) - ele nunca pode
sobrescrever um allow/deny já produzido. Todo hook after executa mesmo
assim, então ele também serve como o ponto de costura para audit-logging:

```rust
Gate::after::<User>(|user, action, decided| {
    audit_log(user.id, action, decided);   // observa toda avaliação
    None                                    // somente registro; não muda o resultado
});
```

Hooks são indexados pelo **tipo de usuário** `U`, não pelo recurso - um
hook dispara para todo `(action, U, R)`. Coloque a lógica específica de
recurso no gate. Hooks são predicados síncronos e também se aplicam ao
caminho de avaliação assíncrono; para lógica de autorização assíncrona, use
`define_async` / `define_async_with`.

### Por que Suprnova diverge

O `Gate::forUser($user)->allows(...)` do Laravel religa o resolvedor
*implícito* de usuário atual do gate, para que a próxima verificação avalie
como aquele usuário. O gate do Suprnova recebe o usuário
**explicitamente** em toda chamada, então "verificar como um usuário
diferente" é só `Gate::allows(action, &other_user, &resource)`. Não há
resolvedor implícito para religar - a API explícita é estritamente mais
geral, o que torna o `forUser` redundante em vez de ausente.

O mesmo raciocínio se aplica à auto-descoberta de política por nome de
classe do Laravel. O Suprnova vincula métodos de política à chave
type-erased `(action, U, R)` no momento do registro, então uma política de
`Post` e uma política de `Comment` com o mesmo nome de método registram
dois gates distintos, sem uma convenção de nomenclatura ou um scan de
descoberta.

## Próximos passos

- [Autenticação](authentication.md) - a metade do lado do usuário: guards,
  `Auth::user()`, `Auth::user_as::<T>()`
- [Inicialização da aplicação](bootstrap.md) - onde `init_policies()`
  executa na sequência de boot, além de como registrar hooks before/after
- [Middleware](middleware.md) - combinando `AuthMiddleware` com autorização
  no nível de rota
- [Modelo de erros](error-model.md) - como a negação de um gate colapsa em
  um 403, um 404, ou um `FrameworkError::Domain` de status customizado
- [Eventos](events.md) - escutando resultados de política via `Gate::after`
  para audit logging

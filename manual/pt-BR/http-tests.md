# Testes HTTP

Este capítulo mostra como testar sua superfície HTTP - rotas,
middleware, fluxos de autenticação, respostas de erro - dirigindo o
pipeline de solicitação do framework através de
`suprnova::handle_request`. Se você já escreveu testes de feature do
Laravel com `$this->get('/users')` e fez asserção sobre
`$response->status()`, este é o equivalente no Suprnova: o mesmo
`Router` que você monta em produção executa no teste, todo middleware
dispara, o limite de panic ainda captura, e a resposta é byte a byte
o que um cliente real vê.

## A superfície de teste

Existem exatamente três blocos de construção:

| Peça | Papel |
|---|---|
| `Router` | As rotas sob teste - construídas da mesma forma que em produção |
| `MiddlewareRegistry` | A stack de middleware global - também construída da mesma forma |
| `handle_request(router, registry, req) -> hyper::Response<…>` | O driver em processo - executa uma solicitação de ponta a ponta |

`handle_request` é a mesma função que `Server::run` chama por
solicitação, exposta para testes e embedders. Qualquer coisa que
funciona em produção funciona aqui - o wrapper de recuperação de
panic, o escopo de request-id, o escopo do flash bag do Inertia, o
escopo de estado de auth da solicitação, a remoção do corpo em HEAD,
a terminação pós-resposta. Não existe um "modo de teste" que troca
por um pipeline mais silencioso.

`handle_request_with_peer` é a mesma chamada com um
`Option<std::net::IpAddr>` explícito para o peer que está se
conectando - útil quando você quer fazer a asserção sobre a resolução
de `Request::ip()` sem configurar headers de proxy.

## O problema do corpo do hyper

A única complicação que vale saber de antemão: `handle_request`
recebe um `hyper::Request<hyper::body::Incoming>`. `Incoming` é o
tipo de corpo de streaming interno do hyper; você não consegue
construir um com `Full::new(bytes)` nem com nenhum dos tipos de corpo
em memória. Ele só sai de uma conexão hyper.

Existem duas formas limpas de contornar isso:

1. **TCP loopback** - faça bind de um listener `127.0.0.1:0`, sirva
   um accept dentro de um `service_fn`, envie a solicitação através
   de um cliente hyper, e deixe o `Incoming` ser produzido
   naturalmente do lado do servidor. É o que todo teste de
   integração no framework já faz.
2. **Construção de `Request` em processo** - para testes que só
   precisam inspecionar acessadores de `Request` (headers, params de
   rota, IP, parsing de JSON) sem passar pelo roteamento, use o mesmo
   padrão de captura por TCP loopback, mas com um serviço que extrai
   o `Request` para um `oneshot::channel` em vez de executá-lo. O
   arquivo `framework/tests/http_request_accessors.rs` tem esse
   helper `build_request()` ao pé da letra.

Os dois padrões produzem corpos `Incoming` reais. O loopback é local,
síncrono em termos de wall-clock de teste (microssegundos), e nunca
toca a rede fora de `lo`. Não existe uma forma mais lenta ou mais
simples que preserve o contrato.

### Por que Suprnova diverge

O `$this->get('/users')` do Laravel funciona porque o ciclo de vida
de solicitação do PHP é "construa um objeto `Request`, despache-o
através do kernel". O kernel recebe o objeto em memória diretamente;
não há tipo de corpo que force um transporte. O servidor do Suprnova
é construído sobre o hyper, e o tipo de corpo do hyper é opinativo
por boas razões (streaming, backpressure, zero-copy). A superfície de
teste herda essa restrição.

O que você ganha em troca da restrição é fidelidade. Todo detalhe do
caminho de solicitação de produção - parsing de header, limites de
corpo, upgrades de conexão - executa da mesma forma nos testes. Você
nunca vai ter um teste passando porque o harness de teste pulou uma
camada que o servidor real executa.

## Um primeiro teste de ponta a ponta

Aqui está um teste completo e funcional que monta uma única rota,
envia um GET contra ela, e faz a asserção sobre o status e o corpo.

```rust
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::{MiddlewareRegistry, Request, Router, handle_request};

async fn spawn_server(router: Router, accepts: usize) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else { return };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move {
                        Ok::<_, Infallible>(handle_request(router, middleware, req).await)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

async fn send_get(addr: SocketAddr, path: &str) -> (u16, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_get timeout")
        .expect("hyper send_request");
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status.as_u16(), bytes)
}

#[tokio::test]
async fn get_root_returns_hello() {
    let router = Router::new().get("/", |_req: Request| async { text("hello") });
    let addr = spawn_server(router, 1).await;

    let (status, body) = send_get(addr, "/").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"hello");
}
```

Essa é a forma inteira. Copie os dois helpers por crate, ajuste-os
para a suíte (múltiplos accepts, captura de header, captura de
corpo). O próprio framework usa helpers quase idênticos em
`framework/tests/cors_middleware.rs`,
`framework/tests/middleware_panic_safety.rs`, e
`framework/tests/email_verified_middleware.rs`.

O argumento `accepts` limita quantas conexões o loop de accept serve
antes de sair. Um é suficiente para uma única solicitação; suba para
dois ou mais quando um teste exercita recuperação pós-panic (veja
[Testando o limite de panic](#testando-o-limite-de-panic)).

## Construindo uma solicitação

Dentro de `send_get` você viu:

```rust
let req = hyper::Request::builder()
    .method("GET")
    .uri("/users/42")
    .header("Host", "localhost")
    .header("Content-Length", "0")
    .body(Full::new(Bytes::new()))
    .unwrap();
```

Essa é a forma canônica. Algumas coisas que vale saber:

- **Header `Host`**. O hyper rejeita solicitações HTTP/1.1 sem um.
  Sempre inclua-o; o valor não importa a menos que seu handler
  dependa dele.
- **`Content-Length: 0`**. Corresponda ao corpo. O hyper calcula
  isso para você com `Full::new(Bytes::new())`, mas ser explícito lê
  mais limpo em testes.
- **Tipos de corpo**. O lado cliente envia `Full<Bytes>`. O lado
  servidor recebe `Incoming`. Você só constrói solicitações
  `Full<Bytes>` em testes; o framework as recebe como `Incoming`
  depois da conversão por conexão do hyper.

Um POST com um corpo JSON:

```rust
let body_bytes = serde_json::to_vec(&serde_json::json!({
    "name": "Alice",
    "email": "alice@example.com"
})).unwrap();

let req = hyper::Request::builder()
    .method("POST")
    .uri("/users")
    .header("Host", "localhost")
    .header("content-type", "application/json")
    .header("content-length", body_bytes.len())
    .body(Full::new(Bytes::from(body_bytes)))
    .unwrap();
```

## Fazendo asserções na resposta

A resposta que volta de `handle_request` é um
`hyper::Response<BoxBody<Bytes, Infallible>>`. Três coisas que você
vai ler dela:

```rust
let (parts, body) = resp.into_parts();

// 1. Status.
assert_eq!(parts.status.as_u16(), 200);

// 2. Headers - lookup insensível a maiúsculas/minúsculas.
let location = parts.headers.get("location").and_then(|v| v.to_str().ok());
assert_eq!(location, Some("/login"));

// 3. Body - colete em bytes, depois faça o parse.
use http_body_util::BodyExt;
let bytes = body.collect().await.unwrap().to_bytes();

// Como texto:
let text = String::from_utf8_lossy(&bytes);

// Como JSON:
let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
assert_eq!(value["message"], "ok");
```

Para respostas de erro ordinárias que alcançam o renderer comum, a forma do
corpo documentada em [Modelo de erros](error-model.md) inclui `message`,
`errors` opcionais, `request_id` e um `debug_message` opcional. `request_id` é
`null` fora de um escopo de solicitação. Três variantes especiais retornam antes
da injeção de request id: `PrecognitionSuccess` é uma 204 sem corpo,
`PrecognitionFailure` é o corpo de validação mais headers de Precognition, e um
sentinela `AlreadyReported` renderizado acidentalmente por HTTP é uma 500 genérica
que contém apenas `message`. Use uma resposta de erro ordinária ao afirmar que o
middleware de request id foi executado.

## Asserções fluentes de resposta com TestResponse

Construir manualmente a tripla `(status, headers, body)` e fazer asserções
sobre ela peça por peça, como acima, é a base usada por todo harness neste
crate. `suprnova::testing::TestResponse` envolve essa mesma tripla em uma API
fluente no formato Laravel, de modo que um teste seja lido como uma asserção em
vez de uma busca de header:

```rust
use suprnova::testing::TestResponse;

let (parts, body) = resp.into_parts();
let bytes = body.collect().await.unwrap().to_bytes();
let headers = parts.headers.iter().map(|(k, v)| {
    (k.as_str().to_string(), v.to_str().unwrap_or_default().to_string())
});

TestResponse::new(parts.status.as_u16(), headers, bytes)
    .assert_ok()
    .assert_header("content-type", "application/json")
    .assert_json(serde_json::json!({ "message": "ok" }));
```

`new()` aceita qualquer iterável como pares de headers `(String, String)` -
um `HashMap<String, String>` (no qual vários harnesses existentes já coletam),
um `Vec<(String, String)>` ou `HeaderMap::iter()` mapeado para strings de
propriedade - portanto nenhum harness precisa mudar como conduz uma solicitação.

Toda asserção retorna `&Self`, portanto elas encadeiam: `assert_status`,
`assert_ok`, `assert_redirect(target: Option<&str>)`, `assert_json`
(correspondência por subconjunto - chaves extras no corpo são aceitas),
`assert_json_path` (notação com pontos; um segmento numérico indexa um array),
`assert_json_count`, `assert_see`, `assert_header`, `assert_cookie`. Falhas de
asserção entram em pânico com um trecho de esperado/real, o mesmo contrato de
`expect!` ([Testes](testing.md)) - esta é uma superfície de testes, não código
de biblioteca, portanto a regra geral de não entrar em pânico não se aplica.

### `assert_session_has` precisa de um armazenamento de sessão

Todas as outras asserções leem somente a resposta no nível do fio.
`assert_session_has` não pode: o estado da sessão no servidor vive no
`SessionStore`, não na resposta, e quando uma resposta volta pelo soquete de
loopback não há sessão em processo restante para ler. Anexe o mesmo
armazenamento com que o `SessionMiddleware` do teste foi construído, mais seu
nome de cookie, e a asserção descriptografa o cookie de sessão da resposta para
encontrar a própria linha:

```rust
let response = TestResponse::new(status, headers, body)
    .with_session_store(middleware.store(), "suprnova_session");

response
    .assert_session_has("flash.success", serde_json::json!("Saved!"))
    .await;
```

É a única asserção `async`, pois é a única que faz E/S; ainda retorna `&Self`,
portanto `.await` fica em linha e a cadeia continua depois dele.

### Por que o Suprnova diverge

O `TestResponse` do Laravel vive no mesmo processo PHP que a aplicação sob
teste, portanto `assertSessionHas` lê `$this->session()` diretamente - não há
fronteira de fio a cruzar. Os testes do Suprnova conduzem uma conexão hyper real,
portanto a sessão é exatamente tão opaca ao teste quanto é a um navegador real:
um cookie. `assert_session_has` recupera essa fidelidade com um handle explícito
do armazenamento em vez de fingir que o atalho em processo existe.

## Testando respostas Inertia

`suprnova::testing::AssertableInertia` envolve um objeto de página Inertia -
quer ele tenha voltado como corpo JSON `X-Inertia` ou incorporado em um shell
HTML de navegação completa - no mesmo estilo fluente de pânico em falha que
`TestResponse`. É equivalente a `Inertia\Testing\AssertableInertia` do Laravel.

Duas formas de obter um. A partir de um `TestResponse` que já passou por uma
visita real com `X-Inertia: true`:

```rust
use suprnova::testing::TestResponse;

let response = TestResponse::new(status, headers, body);
response
    .assert_inertia()
    .component("Users/Index")
    .url("/users")
    .has("users")
    .where_("users.0.name", "Ada")
    .count("users", 1)
    .missing("admin_only_field");
```

Ou diretamente de uma `HttpResponse` - o que `InertiaResponse::resolve` retorna -
para um teste que conduz o pipeline de resposta sem um soquete. Esta forma trata
ambos os formatos: um corpo JSON `X-Inertia`, ou o elemento
`<script data-page="app">` incorporado pelo shell HTML:

```rust
use suprnova::testing::AssertableInertia;

let response = InertiaResponse::new("Users/Index")
    .with("users", users_json)
    .resolve(&req)
    .await?;

AssertableInertia::from_response(&response)
    .component("Users/Index")
    .where_("users.0.name", "Ada");
```

`version()` verifica a versão de ativos da página. O resolvedor padrão calcula
hash do manifesto Vite e recorre a `MANIFEST_VERSION_FALLBACK` quando ainda não
existe manifesto - faça a asserção contra essa constante em vez de um `"1.0"`
codificado em um teste que não construiu um frontend:

```rust
use suprnova::MANIFEST_VERSION_FALLBACK;

response.assert_inertia().version(MANIFEST_VERSION_FALLBACK);
```

`has_flash(key, expected)` lê os dados flash da página pela mesma forma de
caminho com pontos que `has` / `where_` leem props - `expected` é um `Option`,
portanto passe `None::<serde_json::Value>` para verificar apenas a presença:

```rust
response.assert_inertia().has_flash("toast.message", Some(serde_json::json!("Saved!")));
response.assert_inertia().has_flash("toast", None::<serde_json::Value>);
```

### Recarregando para asserções de recarga parcial e props adiadas

`reload_only`, `reload_except` e `load_deferred_props` espelham o que o cliente
Inertia faz após a visita inicial: emitem novamente a mesma página como uma
recarga parcial e verificam o que voltou. Como os testes HTTP do Suprnova
atravessam um soquete loopback hyper real e cada arquivo de teste possui seu
próprio harness (veja [Onde cada peça fica](#where-each-piece-lives) abaixo), estes
métodos não levam transporte embutido - anexe um com `with_reload`, uma closure
de um `ReloadRequest` (a URL, componente, versão e chaves de recarga parcial a
enviar) a um future que produz o `AssertableInertia` recarregado:

```rust
use suprnova::testing::TestResponse;

let assertable = TestResponse::new(status, headers, body)
    .assert_inertia()
    .with_reload(move |reload| {
        async move {
            let header_pairs = reload.headers();
            let headers: Vec<(&str, &str)> = header_pairs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let (status, headers, body) = request(addr, "GET", &reload.url, &headers).await;
            TestResponse::new(status, headers, body).assert_inertia()
        }
    });

// Requests only `users`, and asserts the reload landed on the same
// component/url/version and that `users` came back.
assertable.reload_only(["users"]).await;

// Requests everything except `stats`, and asserts `stats` is absent.
assertable.reload_except(["stats"]).await;

// Reads `deferredProps` off the original page, requests every deferred
// key in one partial reload, and asserts they all came back.
assertable.load_deferred_props().await;
```

Chamar qualquer um dos três sem antes usar `with_reload` entra em pânico com
essa instrução. O resultado de uma recarga leva o mesmo recarregador adiante,
portanto uma segunda `.reload_only(...).await` a partir dele funciona sem
anexar novamente um.

### Por que o Suprnova diverge

O `ReloadRequest` do Laravel emite novamente a solicitação pelo mesmo kernel PHP
em processo usado pelo teste original - um cliente de teste, sempre disponível.
Os testes HTTP do Suprnova conduzem um loopback hyper/TCP real e cada arquivo de
teste define seu próprio par `spawn_server` / `request` (veja
[Onde cada peça fica](#where-each-piece-lives) abaixo), portanto não há um cliente
único que `AssertableInertia` possa usar - `with_reload` torna isso explícito em
vez de codificar um harness que um arquivo de teste com formato diferente não
poderia usar. `component()` também pula a verificação de existência de arquivo
de componente de página do Laravel (`view-finder`) - um componente alcançado por
`Router::inertia` ou um `InertiaResponse::new(name)` escrito à mão é uma string
em tempo de execução sem arquivo a verificar; o equivalente em tempo de
compilação do Suprnova é a macro `inertia_response!` (veja
[Respostas Inertia](frontend-inertia-responses.md)). Seus nomes de método também
divergem dos de `TestResponse`: `component`, `has`, `missing`, `where_`, `count`
e `has_flash` eliminam inteiramente o prefixo `assert_`, correspondendo ao
`Inertia\Testing\AssertableInertia` do Laravel, cujos métodos equivalentes são
semelhantemente simples - o contrato de pânico em falha é idêntico de qualquer
forma, sem o indicativo visual `assert_`.

## Testando middleware

Testes de middleware se parecem idênticos a testes de rota; a única
diferença é o que você `.append()` no registry antes de spawnar.

### Testando middleware global

Passe o middleware para `MiddlewareRegistry::new().append(...)` e
use esse registry - múltiplos middlewares executam na ordem de
append, `prepend` coloca um novo na frente.

```rust
use suprnova::{CorsConfig, CorsMiddleware, MiddlewareRegistry};

fn cors_registry() -> MiddlewareRegistry {
    MiddlewareRegistry::new().append(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(std::time::Duration::from_secs(600)),
    ))
}

#[tokio::test]
async fn cors_preflight_returns_204_with_headers() {
    let router = Router::new();
    // A forma de 3 args de `spawn_server` te deixa conectar um
// MiddlewareRegistry não vazio - copie o helper de
// framework/tests/cors_middleware.rs (tem ~30 linhas).
let addr = spawn_server(router, cors_registry(), 1).await;

    let (status, headers, _) = options(
        addr,
        "/anything",
        &[
            ("Origin", "https://app.example"),
            ("Access-Control-Request-Method", "POST"),
        ],
    ).await;

    assert_eq!(status, 204);
    assert_eq!(
        headers.get("access-control-allow-origin").map(String::as_str),
        Some("https://app.example"),
    );
}
```

Este teste prova mais do que a lógica de CORS em si: ele prova que
middleware global executa em solicitações **não-roteadas** também, o
que é o contrato que o framework garante (senão um preflight OPTIONS
que nunca corresponde a uma rota pularia o CORS). Veja
`framework/tests/cors_middleware.rs` para a suíte completa.

### Testando middleware específico de rota

Anexe com `.middleware(...)` no builder de rota, exatamente como em
produção. Depois teste a rota normalmente - a middleware chain é
construída a partir do mesmo registro.

```rust
let router = Router::new()
    .get("/admin/dashboard", |_req| async { text("admin") })
    .middleware(RequireRole::new("admin"));

let (status, _) = send_get(addr, "/admin/dashboard").await;
assert_eq!(status, 403); // solicitação não autenticada
```

### Fazendo stub do usuário autenticado

Testes reais de auth-flow precisam de um usuário logado. O padrão
mais limpo é um middleware pontual e minúsculo que chama
`Auth::set_user` antes do middleware sob teste. O próprio
`framework/tests/email_verified_middleware.rs` do framework usa isso:

```rust
use std::any::Any;
use std::sync::Arc;
use suprnova::{Auth, Authenticatable, Middleware, Next, Request, Response};

struct UserById(String);

impl Authenticatable for UserById {
    fn get_auth_identifier(&self) -> String { self.0.clone() }
    fn as_any(&self) -> &dyn Any { self }
}

struct LoginAs(String);

#[async_trait::async_trait]
impl Middleware for LoginAs {
    async fn handle(&self, request: Request, next: Next) -> Response {
        Auth::set_user(Arc::new(UserById(self.0.clone())));
        next(request).await
    }
}
```

Depois, no teste:

```rust
let registry = MiddlewareRegistry::new()
    .append(LoginAs("user-id-123".to_string()))
    .append(EnsureEmailVerifiedMiddleware::new());
```

`LoginAs` executa primeiro, instala o usuário no estado de auth por
solicitação, e o middleware sob teste vê `Auth::id() == Some(...)`
sem nunca emitir um login real. O escopo de estado de auth é
configurado pelo próprio `handle_request` - o mesmo que executa em
produção - então o usuário fica visível para todo middleware
posterior e para o handler.

## Testando a vinculação de modelo de rota

`RouteParam<User>` hidrata um `User` tipado pela chain de extractors do handler,
portanto o teste precisa passar esse extractor para uma função `#[handler]`:

```rust
use suprnova::{RouteParam, Response, handler};

#[suprnova::model(table = "users")]
pub struct User {
    pub id: i64,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[handler]
async fn show(RouteParam(user): RouteParam<User>) -> Response {
    suprnova::http::json(serde_json::json!({ "email": user.email }))
}

#[tokio::test]
async fn show_user_binds_from_route_param() {
    // Insira um usuário de teste pelo model. Setup de banco de dados omitido -
    // veja o capítulo de testes para os padrões de `TestDatabase`.
    let user = User::create(suprnova::attrs! {
        email: "bound@example.com"
    }).await.unwrap();

    // Um RouteParam desestruturado usa atualmente `param` como o nome do
    // parâmetro de rota da macro de handler.
    let router: Router = Router::new()
        .get("/users/{param}", show)
        .into();

    let addr = spawn_server(router, MiddlewareRegistry::new(), 1).await;
    let (status, body) = send_get(addr, &format!("/users/{}", user.id)).await;

    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["email"], "bound@example.com");
}
```

Para um parâmetro de rota `{user}`, em vez disso aceite
`user: RouteParam<User>` sem desestruturá-lo; `RouteParam` faz deref para
`User` para acesso aos campos. Chamar `req.param(...).parse()` e depois
`User::find_or_fail(...)` testa o parse do parâmetro e a busca de model, não a
vinculação de modelo de rota.

Para testes de vinculação isolada, chame
`<RouteParam<User> as AutoRouteBinding>::from_route_param(...)`
diretamente. Isso verifica a implementação de vinculação sem um router, mas não
exercita a chain de extractors de `#[handler]`.

## Testando fluxos de autenticação de ponta a ponta

Para testar uma sessão de login de ponta a ponta, passe um registry que contém
`SessionMiddleware` ao servidor loopback e proteja `/dashboard` com
`AuthMiddleware` ou o middleware web-auth da aplicação. Primeiro prove que a rota
rejeita uma solicitação sem cookie, depois faça login, repasse o cookie de sessão
retornado e prove que a rota protegida é bem-sucedida:

```rust
#[tokio::test]
async fn login_flow_issues_session_cookie() {
    // 1. Bootstrap: crie o usuário.
    Auth::password()
        .register("alice@example.com", "longpassword123")
        .await.expect("register");

    // 2. Monte uma rota protegida e o middleware de sessão stateful.
    let router: Router = Router::new()
        .post("/login", login_handler)
        .get("/dashboard", |_req: Request| async { text("dashboard") })
        .middleware(AuthMiddleware::new())
        .into();
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::new(SessionConfig::from_env()));
    let addr = spawn_server(router, registry, 3).await;

    // 3. Prove que a rota está protegida antes de autenticar.
    let (guest_status, _) = send_get(addr, "/dashboard").await;
    assert_eq!(guest_status, 401);

    // 4. Faça login e capture o header Set-Cookie.
    let login = post_json(addr, "/login", serde_json::json!({
        "email": "alice@example.com",
        "password": "longpassword123",
    })).await;
    assert_eq!(login.status, 200);
    let cookie = extract_session_cookie(&login.headers);

    // 5. Reenvie o cookie para a rota protegida.
    let (status, body) = get_with_cookie(addr, "/dashboard", &cookie).await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"dashboard");
}
```

O router abreviado sem esses middlewares demonstra apenas o encanamento de
cookie; ele não é um teste de fluxo de autenticação.
`framework/tests/auth_http_middleware.rs` testa o comportamento do middleware de
autenticação com registries explícitos, mas não instala um `SessionMiddleware`
real. Um teste de fluxo de login stateful deve instalar tanto o middleware de
sessão quanto o gate de autenticação, como mostrado acima.

## Testando o limite de panic

Um panic dentro de um handler não deve derrubar o servidor. O
wrapper de recuperação de panic (`execute_chain_safely`) o captura e
converte para um 500 através do mesmo caminho por onde erros
retornados fluem. Você pode verificar isso sem nenhuma
infraestrutura de teste especial - defina `accepts >= 2` para que o
listener sobreviva ao panic:

```rust
#[tokio::test]
async fn panicking_handler_yields_500_and_server_survives() {
    let router = Router::new()
        .get("/panic", |_req: Request| async {
            panic!("intentional test panic");
            #[allow(unreachable_code)] text("unreachable")
        })
        .get("/ok", |_req: Request| async { text("ok") });

    let addr = spawn_server(router, 4).await;

    // Primeiro: o panic se traduz num 500 sanitizado.
    let (s1, body) = send_get(addr, "/panic").await;
    assert_eq!(s1, 500);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["message"], "Internal Server Error");
    assert!(parsed.get("request_id").is_some());

    // Segundo: o listener sobrevive. A próxima solicitação é normal.
    let (s2, body2) = send_get(addr, "/ok").await;
    assert_eq!(s2, 200);
    assert_eq!(&body2[..], b"ok");
}
```

## Testando acessadores sem passar pelo roteamento

Às vezes você quer testar um acessador de `Request` (`bearer_token`,
`is_method`, `ip`, `is_json`, etc.) sem subir nenhum router. O truque
é um harness minúsculo que executa um serviço hyper cujo único
trabalho é construir o `Request` e mandá-lo de volta através de um
`tokio::sync::oneshot::channel`:

```rust
let (req_tx, req_rx) = tokio::sync::oneshot::channel::<suprnova::Request>();
// ... serviço hyper de loopback cujo service_fn faz:
//     let req = suprnova::Request::new(hyper_req);
//     let _  = req_tx.send(req);
//     retorna um 200 com um corpo vazio
let req = req_rx.await.unwrap();
```

`framework/tests/http_request_accessors.rs` tem o helper completo
`build_request(builder, body) -> Request`. Copie-o uma vez por crate
e todo teste de acessador lê de forma limpa:

```rust
#[tokio::test]
async fn bearer_token_extracts_simple_token() {
    let req = build_request(
        hyper::Request::builder()
            .method("GET")
            .uri("/api/users")
            .header("Authorization", "Bearer secret-token-123"),
        "",
    ).await;
    assert_eq!(req.bearer_token().as_deref(), Some("secret-token-123"));
}
```

O Request é real (produzido pelo hyper a partir de uma troca de rede
real), mas nenhum roteamento ou middleware executou - exatamente o
que você quer quando a unidade sob teste é o próprio acessador.

## Ganchos de builder em `Request`

Quando você tem um `Request` em mãos e precisa fazer fake de uma peça
da camada de roteamento, três métodos de builder ajudam:

```rust
impl Request {
    pub fn with_params(mut self, params: HashMap<String, String>) -> Self;
    pub fn with_route_pattern(mut self, pattern: String) -> Self;
    pub fn with_peer_addr(mut self, addr: std::net::IpAddr) -> Self;
}
```

Esses são os mesmos métodos que o servidor chama quando despacha uma
rota correspondida - o `Router` chama `with_params` depois que o
`matchit` retorna, `with_route_pattern` para que `req.route_pattern()`
resolva, e `with_peer_addr` uma vez que sabe o IP do socket TCP
aceito. Em testes você os chama você mesmo para fazer short-circuit
do mesmo setup.

```rust
let req = Request::new(hyper_req)
    .with_params(HashMap::from([("id".into(), "42".into())]))
    .with_route_pattern("/users/{id}".into())
    .with_peer_addr("192.168.1.10".parse().unwrap());

assert_eq!(req.param("id").unwrap(), "42");
assert_eq!(req.ip(), Some("192.168.1.10".parse().unwrap()));
```

## Coisas para saber

Uma lista curta de armadilhas que pegam autores de primeira viagem:

- **`Incoming` é somente do lado do servidor.** Você não consegue
  construir um no seu teste. O loopback TCP (ou a captura de serviço
  em processo) é o único caminho - não existe um construtor "construa
  um `Request` a partir de um corpo `Vec<u8>`".
- **Não compartilhe estado entre testes.** Cada `#[tokio::test]`
  ganha seu próprio runtime; poluição cross-test geralmente significa
  que você está compartilhando um global (`once_cell`, `lazy_static`,
  variável de ambiente). Para estado de BD veja `TestDatabase` em
  [Testes](testing.md).
- **Cookies precisam de um cliente real.** Sem cookie jar
  automático - costure o `Set-Cookie` de uma resposta no `Cookie` da
  próxima. Veja `framework/tests/auth_http_middleware.rs` para o
  padrão.
- **O spawn de terminação pós-resposta não é bloqueante.** Se você
  quer fazer a asserção sobre efeitos colaterais que executam via
  `Terminable`, faça polling por eles - a resposta volta para o
  cliente antes de o hook executar.

## Onde cada peça vive

| Peça | Arquivo |
|---|---|
| `handle_request`, `handle_request_with_peer` | `framework/src/server.rs` |
| `Request::new`, `with_params`, `with_route_pattern`, `with_peer_addr` | `framework/src/http/request.rs` |
| `MiddlewareRegistry::new`, `append`, `prepend` | `framework/src/middleware/registry.rs` |
| Harness de teste loopback (canônico) | `framework/tests/cors_middleware.rs` |
| Harness de captura de `Request` em processo | `framework/tests/http_request_accessors.rs` |
| Padrão de teste do limite de panic | `framework/tests/middleware_panic_safety.rs` |
| Padrão de ponta a ponta de auth + middleware | `framework/tests/email_verified_middleware.rs` |

| `TestResponse` (asserções fluentes sobre a tripla acima) | `framework/src/testing/response.rs` |
| `AssertableInertia`, `ReloadRequest` (asserções fluentes de objeto de página Inertia) | `framework/src/testing/inertia.rs` |

## Próximos passos

- [Testes](testing.md) - `#[suprnova_test]`, `TestDatabase`, as
  macros `describe!`/`test!`/`expect!`, e a superfície em nível de
  unidade
- [Modelo de erros](error-model.md) - a forma JSON que toda resposta
  de erro usa, a regra de sanitização de 5xx, e o que `request_id`
  significa no corpo de um teste
- [Middleware](middleware.md) - escrevendo o middleware que você
  testa aqui, e o ciclo de vida global-vs-rota
- [Roteamento](routing.md) - o `Router` que você monta em produção e
  em testes, params de rota, nomes de rota, URLs assinadas
- [Autenticação](authentication.md) - a facade `Auth`,
  `Authenticatable`, guards, e como `Auth::set_user` interage com o
  escopo de solicitação que `handle_request` instala

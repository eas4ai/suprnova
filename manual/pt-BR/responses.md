# Respostas

Todo handler do Suprnova retorna um `Response`, que é um alias para
`Result<HttpResponse, HttpResponse>`. O ramo `Ok` carrega a resposta de
sucesso, o ramo `Err` carrega uma resposta de erro já renderizada, e o
operador `?` colapsa pelo caminho qualquer tipo de erro que tenha um
`From` para `HttpResponse`. Este capítulo é a referência prática para
construir o lado `Ok` - os builders de `HttpResponse`, o builder
`Redirect`, a API de cookies, e os short-circuits `abort_*`. Para a
abordagem de erros, veja [Modelo de erros](error-model.md) e
[Tratamento de erros](errors.md).

## Builders de `HttpResponse`

`HttpResponse` é o tipo de resposta no formato que vai para a rede. Os
construtores definem padrões sensatos; os setters encadeáveis os
sobrescrevem.

### Construtores de corpo

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn examples() -> Response {
    // text/plain
    let _ = HttpResponse::text("OK");

    // application/json (qualquer serde_json::Value)
    let _ = HttpResponse::json(json!({ "ok": true }));

    // text/html; charset=utf-8
    let _ = HttpResponse::html("<h1>Hello</h1>");

    // Bytes crus com um content type explícito - usado pela serialização
    // JSON:API e por qualquer outro corpo de bytes que não seja JSON.
    let _ = HttpResponse::bytes_body(b"PNG...".to_vec(), "image/png");

    Ok(HttpResponse::text("done"))
}
```

Existem dois construtores de streaming para respostas de vida longa:

- `HttpResponse::sse(stream)` - eventos enviados pelo servidor. Envolve
  um `Stream` de valores `SseEvent`, define os quatro headers
  obrigatórios (`Content-Type: text/event-stream`,
  `Cache-Control: no-cache`, `Connection: keep-alive`,
  `X-Accel-Buffering: no`), e mantém a conexão aberta até o stream
  produtor terminar. Veja
  [Eventos enviados pelo servidor](sse.md).
- `HttpResponse::stream_bytes(stream)` - resposta em chunks genérica.
  Recebe um `Stream<Item = Result<Bytes, Infallible>>`. O tipo de erro é
  `Infallible` por design: todo produtor do framework transforma seus
  próprios erros em uma mensagem terminal do stream antes de o stream
  acabar, porque não há como levar um erro de nível de transporte até o
  cliente no meio da resposta.
- `HttpResponse::event_stream(stream, end)` - `ResponseFactory::eventStream` do Laravel.
  Envolve um `Stream` de valores `sse::StreamedEvent`, enquadrando cada um como `event: update` (ou
  seu próprio nome) mais um frame terminal configurável. Veja [Eventos enviados pelo servidor](sse.md).
- `HttpResponse::stream_json(stream)` - `ResponseFactory::streamJson` do Laravel. Envolve um
  `Stream` de qualquer valor `Serialize` e o descarrega como um array JSON construído incrementalmente
  em vez de armazenar toda a coleção primeiro em buffer. Veja [Eventos enviados pelo servidor](sse.md#event-stream-and-stream-json).

### Status, headers, cookies

Todo builder retorna `Self`, então encadeie à vontade:

```rust
use suprnova::{Cookie, HttpResponse, Response};
use serde_json::json;

pub async fn created() -> Response {
    Ok(HttpResponse::json(json!({ "id": 42 }))
        .status(201)
        .header("X-Resource-Id", "42")
        .cookie(Cookie::new("last_id", "42")))
}
```

| Método | Comportamento |
|---|---|
| `.status(code)` | Define o status HTTP. Códigos fora de `100..=599` são rebaixados para 500 na fronteira da rede, com um log de aviso. |
| `.header(name, value)` | Acrescenta um header. Duplicatas são permitidas (corresponde à semântica de `Set-Cookie`). |
| `.replace_header(name, value)` | Descarta quaisquer ocorrências anteriores e define uma. |
| `.with_headers([(k, v), ...])` | Acrescenta vários de uma vez. Aceita qualquer `IntoIterator<Item = (K, V)>`. |
| `.without_header(name)` | Remove todas as ocorrências (insensível a maiúsculas e minúsculas). |
| `.header_value(name)` | Lê de volta o primeiro valor definido. Útil em testes. |
| `.cookie(Cookie)` | Anexa um cookie como `Set-Cookie`. |
| `.with_cookies([Cookie, ...])` | Anexa vários. |
| `.without_cookie(name)` | Agenda uma exclusão (equivalente a `Cookie::forget(name)`). |

Os mesmos setters encadeáveis estão disponíveis em um `Response` (o
`Result`) através da trait `ResponseExt`, para que as macros continuem
ergonômicas:

```rust
use suprnova::{json_response, Cookie, Response, ResponseExt};

pub async fn list() -> Response {
    json_response!({ "ok": true })
        .status(200)
        .header("X-Total-Count", "42")
        .cookie(Cookie::new("last_query", "list"))
}
```

`ResponseExt` expõe `.status`, `.header`, `.with_headers`,
`.without_header`, `.cookie`, `.with_cookies`, e `.without_cookie`.

### Validação na fronteira da rede

`HttpResponse::into_hyper` roda dois filtros de segurança antes de
entregar a resposta ao hyper:

- **Intervalo de status.** Qualquer coisa fora de `100..=599` é
  rebaixada para 500 com um `tracing::warn!`. Isso pega erros de
  digitação como `AppError::status(700)` na fronteira, em vez de deixar
  códigos não conformes chegarem à rede.
- **Injeção de CRLF em header.** Todo nome e valor de header é validado
  pelo próprio `HeaderName::try_from` / `HeaderValue::try_from` do
  hyper. Qualquer header rejeitado é descartado com um log de aviso e a
  resposta é construída sem ele. Valores controlados por um atacante que
  acabam refletidos em um header (allow-headers de CORS,
  `X-Forwarded-*`, headers de debug customizados) não conseguem dividir
  a resposta.

Os dois filtros são silenciosos no caminho de sucesso - você só os vê
nos logs quando alguma coisa tentou passar despercebida.

## Macros de resposta

Existem duas macros com o formato de `Response` para os casos comuns:

```rust
use suprnova::{json_response, text_response, Response};

pub async fn json_handler() -> Response {
    json_response!({ "users": [{ "id": 1, "name": "Alice" }] })
}

pub async fn text_handler() -> Response {
    text_response!("OK")
}
```

As duas expandem para `Ok(HttpResponse::...)`. Encadeie setters de
`ResponseExt` em qualquer uma delas para ajustar status, headers ou
cookies.

## Cookies

`Cookie::new(name, value)` produz um cookie com padrões seguros -
`HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`. Sobrescreva por cookie:

```rust
use suprnova::Cookie;
use std::time::Duration;

let session = Cookie::new("session_id", "abc123")
    .http_only(true)
    .secure(true)
    .same_site(suprnova::SameSite::Strict)
    .path("/")
    .domain("example.com")
    .max_age(Duration::from_secs(3600))
    .partitioned(true);
```

Quatro construtores de conveniência cobrem padrões comuns:

- `Cookie::forget(name)` - valor vazio, `Max-Age=0`, path `/`, sem
  domínio. Use isto no logout para instruir o navegador a descartar o cookie.
- `Cookie::forget_with(name, path, domain)` - a forma com escopo. Um navegador
  só descarta um cookie quando o `Path` e o `Domain` do cookie de exclusão
  correspondem aos usados na sua definição, portanto um cookie definido com
  `Path=/admin` ou `Domain=.example.com` sobrevive a um `forget` simples. Passe
  `None` para qualquer argumento para manter o padrão.
- `Cookie::forever(name, value)` - `Max-Age` de cinco anos.
- `Cookie::encrypted(name, plaintext)` - escreve texto cifrado AES-256-GCM cujo
  AAD está vinculado ao nome lógico do cookie. Leia-o com
  `Cookie::read_encrypted_for(name, wire)` usando o mesmo nome.
  `Cookie::read_encrypted(wire)` é o leitor v1 sem contexto e obsoleto; ele não
  pode descriptografar a saída atual de `Cookie::encrypted` e está programado
  para remoção na 1.4.0, junto com o fallback v1. Exige que `APP_KEY` esteja
  definida no boot. Veja [Criptografia](encryption.md).

Remover vários cookies de uma vez - o formato usual de logout - é
`without_cookies`, disponível em `HttpResponse`, em `Response` por
`ResponseExt` e em ambos os builders de redirecionamento:

```rust
use suprnova::{HttpResponse, Redirect};

let _ = HttpResponse::text("bye").without_cookies(["session", "remember"]);
let _: suprnova::Response = Redirect::to("/login")
    .without_cookies(["session", "remember"])
    .into();
```

Em um redirecionamento, as exclusões viajam no próprio 302, não no destino,
portanto o navegador já as descartou quando segue o `Location`.

### Enfileirando um cookie para depois

Às vezes, código que não está construindo a resposta ainda precisa definir um
cookie - um listener reagindo a um evento, um middleware executado antes do
handler, um serviço `App::bind` sem `HttpResponse` no escopo. `Cookie::queue` é
o `Cookie::queue()` do Laravel: ele guarda o cookie em um jar por solicitação
que `SessionMiddleware` drena para a resposta de saída, logo após o cookie de
sessão.

```rust
use suprnova::Cookie;

Cookie::queue(Cookie::new("theme", "dark"));

// Consulta o que está enfileirado.
let queued = Cookie::queued("theme");

// Remove antes de a resposta sair.
Cookie::unqueue("theme");

// Enfileira uma exclusão em vez de um valor - compõe com `forget_with`.
Cookie::expire("theme", Some("/app"), None);
```

O jar é task-local e começa vazio em toda solicitação - nada enfileirado em uma
solicitação fica visível na próxima, e um valor enfileirado mas nunca drenado
(sem `SessionMiddleware` na chain da rota) é descartado em vez de causar panic.
Cookies enfileirados são anexados a tudo que o handler retorna, inclusive um
redirecionamento: um handler que enfileira um cookie e então retorna
`Redirect::to(...)` ainda carrega o header `Set-Cookie` na resposta 3xx. Eles
também são anexados a um 500 que o próprio `SessionMiddleware` constrói para uma
falha interna durante a solicitação - uma sessão existente que não pode ser
lida, uma gravação de sessão que falha ou a falha da criptografia do cookie de
sessão - pois um cookie enfileirado já pode representar um efeito colateral
confirmado em outro lugar (uma linha de token de lembrar-me já gravada, por
exemplo), portanto a resposta que informa a falha ainda o carrega. Eles **não**
sobrevivem a um panic - o código de drenagem de `SessionMiddleware` roda depois
de o handler retornar normalmente, e um panic capturado é convertido em 500
fora de toda a chain de middleware, o mesmo ponto em que os próprios cookies
enfileirados do Laravel se perdem para uma exceção não capturada.

### Por que Suprnova diverge

O `CookieJar` do Laravel indexa a fila por nome *e* path, portanto dois cookies
com o mesmo nome em paths diferentes podem ser enfileirados independentemente.
O Suprnova indexa o jar somente pelo nome: enfileirar um segundo cookie com um
nome já enfileirado substitui o primeiro em vez de adicionar uma segunda linha
`Set-Cookie`. Isso cobre o caso comum - um ponto de chamada possui um nome de
cookie dado - sem a busca extra indexada por path de que a versão do Laravel
precisa.
A serialização do header faz percent-encode de todo byte que não seja um
cookie-octet válido segundo a RFC 6265, incluindo todos os caracteres de
controle. CRLF em um nome ou valor de cookie é codificado, não
propagado - a injeção de header através de cookies está fechada no
serializador.

## Redirecionamentos

`Redirect` cobre a superfície completa do redirector do Laravel. Toda
variante implementa `From<Redirect> for Response`, então a forma
idiomática é `Redirect::...().into()`.

### Alvos

```rust
use suprnova::{Redirect, redirect_to};

// URL ou caminho explícito
let _ = Redirect::to("/dashboard");

// A mesma coisa, com uma função livre um pouco mais curta
let _ = redirect_to("/dashboard");

// Rota nomeada (retorna RedirectRouteBuilder)
let _ = Redirect::route("users.show").with("id", "42");

// URL externa explícita - igual a `to`, mas o nome sinaliza
// "isto está saindo do site" para auditorias de open redirect
let _ = Redirect::away("https://external.example.com");

// Recarrega a página (lê a URL anterior da sessão; cai de volta
// para "/" se nenhum escopo de sessão estiver ativo)
let _ = Redirect::refresh();

// O mesmo, mas recebendo um Request explícito quando não há escopo ativo
// let _ = Redirect::refresh_for(&request);

// previous_url da sessão, com fallback quando não há sessão em escopo
let _ = Redirect::back("/login");

// URL pretendida guardada na sessão, consumida na leitura, com fallback
let _ = Redirect::intended("/home");

// Redirecionamento de visitante: guarda a URL da solicitação atual como
// "intended" e manda o usuário para uma página de login
// let _ = Redirect::guest(&request, "/login");
```

`Redirect::back`, `Redirect::intended`, `Redirect::guest` e
`Redirect::refresh` se integram todos com a sessão. Sem um escopo de
sessão, eles caem silenciosamente para seus padrões - conveniente para
setups de teste parciais. Veja [Sessões](session.md).

O alvo de `Redirect::back` - a URL anterior registrada da sessão - nunca é
confiado literalmente. O middleware de sessão só registra de início uma URL
relativa à raiz e de mesma origem (um path que comece com `//` ou `/\`, ou que
leve um byte de controle ASCII em qualquer ponto, nunca é armazenado), e a
mesma checagem executa novamente em toda leitura, portanto `back` não pode ser
direcionado para fora da origem nem por uma solicitação que alcance sua aplicação
com um path incomum nem por um cookie de sessão gravado antes de essa guarda
existir. Veja [Sessão](session.md#other-operations) para a regra completa.

### Validação de rota nomeada

A proc-macro `redirect!` valida o nome da rota em tempo de compilação e
expande para `Redirect::route(name)`:

```rust
use suprnova::{redirect, Response};

pub async fn store() -> Response {
    // A compilação falha se "users.index" não for um nome de rota
    // registrado; a mensagem de erro lista as rotas disponíveis e sugere
    // as mais parecidas.
    redirect!("users.index").into()
}
```

### Status codes

```rust
use suprnova::Redirect;

let _ = Redirect::to("/x").permanent();      // 301
let _ = Redirect::to("/x").status(303);      // 303, 307, 308, ...
```

O padrão é 302.

### Dados de flash

Builders de `Redirect` carregam seu próprio flash bag. Na conversão para
um `Response`, o bag é drenado para a sessão viva, sobrevivendo a
exatamente mais uma solicitação:

```rust
use suprnova::Redirect;

let _ = Redirect::back("/users/new")
    .with("status", "User created")            // par chave/valor único
    .with_input([                              // repovoa o formulário
        ("email", "shawn@example.com"),
        ("name", "Shawn"),
    ])
    .with_errors([                             // bag de erros padrão
        ("email", "Must be unique"),
    ])
    .with_errors_bag("login", [                // bag de erros nomeado
        ("password", "Required"),
    ]);
```

A página que recebe lê esses valores de volta através de
`session.get(...)` (para `with`), `session.get_old_input(...)` (para
`with_input`), e do mapa de bags drenado por
`session.pull_errors_flash()` (para `with_errors` / `with_errors_bag`).
A camada Inertia consome o flash de erros automaticamente - a prop
`errors` de toda resposta Inertia é semeada a partir da sessão, então
`Redirect::back().with_errors(...)` faz as mensagens aparecerem no
destino sem que você precise ligar mais nada. O header de solicitação
`X-Inertia-Error-Bag` coloca a prop sob um bag nomeado, para páginas com
vários formulários.

Repare que no `RedirectRouteBuilder` (o que `Redirect::route` e
`redirect!` retornam), `.with(key, value)` define um **parâmetro de
rota**, não uma entrada de flash - ali use `.flash(key, value)`:

```rust
use suprnova::redirect;

let _ = redirect!("users.show")
    .with("id", "42")                          // parâmetro de rota
    .flash("status", "Updated");               // flash de sessão
```

### Cookies, headers, fragmentos

```rust
use suprnova::{Cookie, Redirect};

let _ = Redirect::route("billing.show")
    .with_cookies([Cookie::new("welcome", "yes")])
    .with_headers([("X-Trace", "abc")])
    .with_fragment("invoices")                 // acrescenta #invoices
    .without_fragment();                       // OU remove o fragmento anterior
```

`with_fragment` aceita o fragmento com ou sem um `#` inicial. Chamar
`with_fragment` depois de `without_fragment` anexa um de novo.

### Preservar o fragmento através do redirecionamento

Para apps Inertia em que o destino deve preservar o hash da URL de
*origem*, use `preserve_fragment`:

```rust
use suprnova::Redirect;

let _ = Redirect::route("dashboard.index").preserve_fragment();
```

Na conversão, isso faz flash de `_inertia.preserve_fragment = true` na
sessão; a próxima resposta Inertia lê a flag e emite
`preserveFragment: true` no seu page object. Sem escopo de sessão, a
flag é descartada silenciosamente.

### Redirecionamentos assinados

Dois builders envolvem a superfície de assinatura de URL para
redirecionamentos de uso único a rotas nomeadas (reset de senha,
verificação de email, links de download):

```rust
use suprnova::Redirect;

let r = Redirect::signed_route("downloads.show", &[("id", "42")])?;
let r = Redirect::temporary_signed_route(
    "downloads.show",
    &[("id", "42")],
    1_700_000_000, // expires_at_epoch_seconds
)?;
```

Os dois retornam `Result<Redirect, FrameworkError>` - propague o erro
com `?`, já que `Redirect` converte para um `Response` de forma limpa.
Veja [Geração de URLs](urls.md) para a superfície de assinatura.

### Armazenando a URL pretendida

`Redirect::set_intended_url` escreve o alvo pretendido da sessão sem
executar um redirecionamento - tipicamente chamado a partir do
middleware de autenticação antes de redirecionar para `/login`, para que
um `Redirect::intended` posterior consiga recuperar a URL originalmente
solicitada:

```rust
suprnova::Redirect::set_intended_url("/admin/users");
```

## Abortando a partir de um handler

Três funções livres fazem short-circuit em um handler em um status dado.
Elas retornam `Result<(), FrameworkError>`; combine com `?`:

```rust
use suprnova::{abort_if, abort_unless, abort_with, json_response, Request, Response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    json_response!({ "ok": true })
}
```

O erro subjacente é `FrameworkError::Domain { message, status_code }`,
então ele renderiza através do mesmo envelope JSON e das mesmas regras
de sanitização de 5xx que todo outro caminho de erro. Status codes fora
do intervalo são coagidos para 500 pelo response renderer. Veja
[Modelo de erros](error-model.md) para o contrato de conversão completo.

## Retornando erros diretamente

Como `Response` é `Result<HttpResponse, HttpResponse>`, você pode
retornar um ramo `Err` diretamente - útil quando o formato da resposta
já é um corpo JSON específico e você o quer na rede exatamente assim:

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn legacy_lookup() -> Response {
    Err(HttpResponse::json(json!({
        "error": "deprecated endpoint",
    })).status(410))
}
```

Para qualquer coisa mais rica - erros de domínio tipados, validação,
observabilidade - use a superfície do
[Modelo de erros](error-model.md) (`AppError`, `FrameworkError`,
`#[domain_error]`).

## Referência rápida

| Necessidade | Use |
|---|---|
| Resposta JSON | `HttpResponse::json(v)` ou `json_response!({...})` |
| Resposta de texto | `HttpResponse::text(s)` ou `text_response!(s)` |
| Resposta HTML | `HttpResponse::html(s)` |
| Bytes crus + content-type | `HttpResponse::bytes_body(b, "image/png")` |
| Eventos enviados pelo servidor | `HttpResponse::sse(stream)` - veja [SSE](sse.md) |
| Stream em chunks | `HttpResponse::stream_bytes(stream)` |
| Definir o status | `.status(code)` |
| Adicionar um header | `.header(k, v)` / `.with_headers([...])` |
| Remover um header | `.without_header(name)` |
| Anexar um cookie | `.cookie(c)` / `.with_cookies([...])` |
| Esquecer um cookie | `.without_cookie(name)` / `.without_cookies([...])` |
| Esquecer um cookie com escopo de path/domínio | `Cookie::forget_with(name, Some("/admin"), Some("example.com"))` |
| Enfileirar um cookie para a próxima resposta | `Cookie::queue(c)` |
| Consultar um cookie enfileirado | `Cookie::queued(name)` |
| Remover um cookie da fila | `Cookie::unqueue(name)` |
| Enfileirar um cookie de exclusão | `Cookie::expire(name, path, domain)` |
| Redirecionamento simples | `Redirect::to(path).into()` ou `redirect_to(path).into()` |
| Redirecionamento para rota nomeada | `redirect!("name").into()` ou `Redirect::route("name")` |
| Redirecionamento de volta | `Redirect::back(fallback)` |
| Redirecionamento para a URL pretendida | `Redirect::intended(default)` |
| Redirecionamento de visitante (guarda a pretendida) | `Redirect::guest(&req, login)` |
| Definir o alvo pretendido | `Redirect::set_intended_url(url)` |
| URL externa | `Redirect::away(url)` |
| Recarregar a página atual | `Redirect::refresh()` / `Redirect::refresh_for(&req)` |
| Redirecionamento para rota assinada | `Redirect::signed_route(name, &[(k, v)])?` |
| Parâmetro de rota no redirecionamento | `.with("key", "value")` |
| Parâmetro de query no redirecionamento | `.query("key", "value")` |
| Dados de flash | `.with(key, value)` (ou `.flash` no `RedirectRouteBuilder`) |
| Input em flash | `.with_input([(k, v), ...])` |
| Erros em flash | `.with_errors([(k, msg), ...])` |
| Bag de erros nomeado | `.with_errors_bag(bag, [(k, msg)])` |
| Acrescentar um fragmento | `.with_fragment("section")` |
| Remover o fragmento | `.without_fragment()` |
| Preservar o fragmento (Inertia) | `.preserve_fragment()` |
| Redirecionamento permanente | `.permanent()` (301) |
| Status de redirecionamento customizado | `.status(303)` |
| Abortar cedo | `abort_with(code, msg)?`, `abort_if(cond, code, msg)?`, `abort_unless(cond, code, msg)?` |

## Próximos passos

- [Modelo de erros](error-model.md) - `FrameworkError`, `AppError`,
  `HttpError`, e a conversão única que renderiza todo erro para um
  `HttpResponse`
- [Tratamento de erros](errors.md) - padrões práticos de handler para
  `?`, `AppError`, e erros de domínio customizados
- [Eventos enviados pelo servidor](sse.md) - construindo e consumindo
  respostas `sse(...)`
- [Geração de URLs](urls.md) - URLs assinadas, resolução de rota
  nomeada, a superfície por trás de `Redirect::signed_route`
- [Sessões](session.md) - dados de flash, URLs pretendidas, o bag em que
  `Redirect::with`/`with_input`/`with_errors` escrevem

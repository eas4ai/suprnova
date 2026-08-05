# WebSockets

As rotas WebSocket do Suprnova ficam junto das rotas HTTP no mesmo
router. Você registra um caminho e um handler; o framework detecta a
solicitação `Upgrade: websocket` naquele caminho, executa a mesma
chain de middleware que um HTTP GET para aquele caminho executaria,
completa o handshake do RFC 6455, e chama seu handler com um
`WsSocket` tipado mais a `Request` original. Não há um servidor
WebSocket separado - as conexões passam por upgrade a partir do mesmo
listener hyper que serve seu tráfego HTTP. O framework também rastreia
todo handler spawnado em um `JoinSet` por servidor, então um shutdown
gracioso drena as conexões em voo antes do listener terminar.

## Início rápido

Adicione um `EchoHandler` e registre-o em `routes!`.

`src/ws/echo.rs`:

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct EchoHandler;

#[async_trait]
impl WebSocketHandler for EchoHandler {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("echo: {text}")).await?;
        }
        Ok(())
    }
}
```

`src/routes.rs` (dentro de `routes! { ... }`):

```rust
ws!("/ws/echo", app_ws::echo::EchoHandler),
```

Inicie a app e conecte com `wscat`:

```bash
cargo run --bin app
```

```text
$ wscat -c ws://localhost:3000/ws/echo
Connected (press CTRL+C to quit)
> hello
< echo: hello
> suprnova
< echo: suprnova
```

Quando `recv_text()` retorna `Ok(None)` o peer fechou a conexão; o
loop termina, o handler retorna `Ok(())`, e o framework envia um frame
Close(1000) limpo.

## Ciclo de vida de um upgrade

Um handshake WebSocket é um HTTP GET com `Upgrade: websocket`. O
framework executa o pipeline de solicitação completo contra ele antes
que qualquer frame flua:

1. **Correspondência de rota.** O router procura o caminho na tabela
   de rotas WS; em caso de miss a solicitação recai para o fallback
   HTTP.
2. **Política de origem.** A [`OriginPolicy`](#política-de-origem)
   configurada é aplicada. Uma violação retorna HTTP 403 sem upgrade.
3. **Negociação de subprotocolo.** Se a rota tem `accepted_protocols`,
   o primeiro token oferecido pelo cliente que coincide é ecoado na
   resposta 101.
4. **Chain de middleware.** `RequestIdMiddleware` executa mais
   externamente, seguido por todo middleware registrado globalmente,
   seguido pelo middleware por rota da rota. Uma resposta não-2xx de
   qualquer middleware faz short-circuit no upgrade: o peer recebe o
   erro HTTP, e a future do WebSocket é dropada de forma limpa.
5. **Handshake.** `hyper_tungstenite::upgrade` produz a future que
   resolve em um `WebSocketStream`.
6. **Dispatch do handler.** A `Request` (possivelmente reescrita por
   middleware) e um `WsSocket` recém-construído são entregues a
   `WebSocketHandler::handle`.
7. **Heartbeat + handler.** O framework spawna uma task de heartbeat
   por conexão e aguarda a future do handler sob um span de tracing
   `ws.connection` carregando o request id.
8. **Handshake de close.** Em `Ok(())` o framework envia Close(1000);
   em `Err(_)` ele envia Close(1011 "internal error"). O forwarder é
   aguardado para que o frame de close seja enviado (flush) para a
   rede antes que a task rastreada da conexão seja reportada como
   concluída.

A semântica do valor de retorno é invertida em relação a HTTP: não há
corpo. `Ok(())` significa desconexão limpa; `Err(_)` é logado e o peer
vê Close(1011). De qualquer forma a conexão é derrubada.

## A API `WsSocket`

`WsSocket` é o handle bidirecional que o framework passa para seu
handler. Internamente, o stream tungstenite subjacente é dividido em
metades Sink + Stream: uma task forwarder é dona do sink e drena um
mpsc; os métodos de send voltados para o handler enfileiram no mpsc. O
handler lê diretamente da metade stream. Essa divisão significa que o
framework também pode fazer push de frames (pings de heartbeat,
fan-out do broadcaster) sem disputar com o caminho de send do handler.

### `send_text`

```rust
socket.send_text("hello").await?;
socket.send_text(format!("user {id} joined")).await?;
```

Enfileira um frame de texto UTF-8. Retorna `Err` somente quando a
conexão já está fechada.

### `send_binary`

```rust
socket.send_binary(bytes).await?;
```

Enfileira um frame binário. Aceita qualquer `Into<Vec<u8>>`. Mesma
semântica de erro que `send_text`.

### `recv_text`

```rust
while let Some(text) = socket.recv_text().await? {
    // text: String
}
// Ok(None) means the peer closed.
```

Retorna a próxima mensagem de texto, descartando silenciosamente
variantes de frame que um handler somente-texto não deveria precisar
tratar:

- `Message::Binary` - payload binário do peer
- `Message::Ping` - ping iniciado pelo peer (o tungstenite trata o
  pong automaticamente)
- `Message::Pong` - resposta de pong do peer a um heartbeat do
  framework (o contador de pings perdidos é resetado para zero como
  efeito colateral)
- `Message::Frame` - variantes de frame bruto de contextos do lado do
  servidor; nunca esperado nesta camada

Um frame engolido se foi; não há forma retroativa de vê-lo. Se o
handler precisa observar frames binários ou códigos de close, use
[`recv`](#recv) desde a primeira leitura.

### `recv`

```rust
use tokio_tungstenite::tungstenite::Message;

while let Some(msg) = socket.recv().await? {
    match msg {
        Message::Text(t)   => { /* ... */ }
        Message::Binary(b) => { /* ... */ }
        Message::Close(_)  => break,
        _                  => {}
    }
}
```

Retorna a próxima mensagem de qualquer tipo, incluindo Binary, Ping,
Pong, e Close. `Pong` ainda reseta o contador de pings perdidos como
efeito colateral antes de ser retornado. `Ok(None)` significa que o
stream subjacente terminou.

### `close`

```rust
socket.close(1008, "policy violation").await?;
return Ok(());
```

Enfileira um frame de close e retorna. O forwarder escreve o frame no
sink, chama `close()` no sink, e termina. Sends subsequentes no mesmo
socket retornam `Err` porque o forwarder se foi. Sempre retorne
`Ok(())` imediatamente depois de chamar `close`.

`close` valida seus argumentos antecipadamente contra o RFC 6455
§7.4 + §5.5.1:

- `code` precisa satisfazer `CloseCode::is_allowed()`. Códigos
  reservados ou inválidos (1004, 1005, 1006, 1015, qualquer coisa
  abaixo de 1000, qualquer coisa acima de 4999) são rejeitados com
  `Err` e **nenhum frame é enviado**: a conexão permanece aberta e o
  chamador pode tentar de novo com um código válido. Use 1000 para
  encerramento normal, 1001-1013 para os motivos definidos, 3000-3999
  para códigos registrados no IANA, ou 4000-4999 para códigos privados
  de aplicação.
- `reason` tem um limite de 123 bytes (o limite de 125 bytes para
  frames de controle menos os dois bytes do código). Motivos mais
  longos são rejeitados sem enfileirar nada.

### Por que Suprnova diverge

Frameworks PHP acoplam suporte a WebSocket como um processo separado
(ratchet, soketi, pusher). A rota WebSocket do Suprnova vive no mesmo
`routes! { ... }` que suas rotas HTTP, servida pelo mesmo listener
hyper, drenada pelo mesmo caminho de shutdown gracioso. Há um binário,
uma config, um deploy. Conexões de longa duração são de primeira
classe porque o Tokio as torna baratas; o framework não precisa se
desculpar por elas.

## Parâmetros de caminho

Rotas WebSocket suportam a mesma sintaxe de captura `{param}` que
rotas HTTP. Valores capturados ficam disponíveis na `Request` passada
para o handler.

```rust
// Em routes!:
ws!("/ws/rooms/{id}", RoomHandler),
```

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct RoomHandler;

#[async_trait]
impl WebSocketHandler for RoomHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let room_id = req.param("id")?;
        socket.send_text(format!("joined room {room_id}")).await?;
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[{room_id}] {text}")).await?;
        }
        Ok(())
    }
}
```

`req.param("id")` retorna `Result<&str, ParamError>`; o `?` propaga um
`FrameworkError::ParamError` se o segmento estiver ausente, o que faz
o handler retornar `Err` e o framework enviar Close(1011). Na prática
a captura está sempre presente quando a rota deu match: o caminho de
erro é uma rede de segurança contra erros de digitação no nome do
param.

Segmentos `:id` no estilo Express também são aceitos
(`ws!("/ws/rooms/:id", h)`) e são convertidos internamente para a
forma matchit.

Para a API completa de `Request` - headers, cookies, query string,
endereço do peer - veja [a documentação de requests](requests.md).

## Middleware por rota

Encadeie `.middleware(M)` na entrada `ws!`. Múltiplos middlewares se
compõem da esquerda para a direita e executam na mesma ordem fixa que
uma solicitação HTTP para o mesmo caminho executaria:
`RequestIdMiddleware` mais externamente, depois todo middleware
registrado globalmente, depois a chain por rota, depois o handler.

```rust
ws!("/ws/private", PrivateHandler)
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

Uma resposta não-2xx de qualquer middleware faz short-circuit no
upgrade. O peer recebe a rejeição (ex: 401, 403) com `X-Request-Id`
definido, a future do WebSocket nunca despertada é dropada de forma
limpa, e o handler nunca é chamado. Essa é a camada certa para
verificações de nível de transporte: quem pode abrir a conexão, de
onde a conexão está vindo, quantas conexões concorrentes por
identidade.

Um middleware pode substituir por uma `Request` modificada chamando
`next(modified_req)`. O terminator captura o que quer que a chain
finalmente deixe passar, e é isso que o handler vê como seu argumento
`Request`. Middleware que resolve identidade (uma consulta de sessão,
uma verificação de token) pode anexar o resultado via extensions de
`Request`; o handler o lê de volta da mesma forma que controllers HTTP
fazem.

As variantes diretas em `Router` (`Router::ws`,
`Router::ws_with_middleware`, `Router::ws_with_config`,
`Router::ws_with_middleware_and_config`) cobrem a mesma superfície
para código que constrói um `Router` fora da macro. Cada uma tem um
irmão falível `try_*` que retorna `Err(FrameworkError)` em padrões
duplicados ou malformados em vez de entrar em panic.

### Por que Suprnova diverge

A maioria dos ecossistemas ou pula middleware em upgrades WebSocket (a
convenção do Node) ou força um ritual de registro separado para
"middleware WebSocket" (a convenção .NET / Spring). O Suprnova trata o
upgrade como o HTTP GET que ele realmente é: a mesma chain executa, na
mesma ordem, com a mesma semântica de short-circuit. Não há um segundo
conceito para aprender - `AuthMiddleware`, `RateLimitMiddleware`,
`RequestIdMiddleware`, `CorsMiddleware` funcionam em rotas WS porque
funcionam em qualquer rota. A aplicação de origem é a única
complicação extra, e é uma propriedade de `WsConfig`, não um
middleware separado.

## Autenticação na conexão

O handler recebe a `Request` reescrita por middleware. Três padrões
funcionam bem, em ordem crescente de integração com o resto do
framework:

**Padrão 1 - bearer token inline no handler.** O mais simples. Funciona
sem nenhum middleware de auth. `wscat`, clientes de navegador, e load
balancers todos passam headers de forma limpa.

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct PrivateChatHandler;

#[async_trait]
impl WebSocketHandler for PrivateChatHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let Some(token) = req.header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            socket.close(1008, "missing bearer token").await?;
            return Ok(());
        };
        let Some(user_id) = verify_token(token).await else {
            socket.close(1008, "invalid bearer token").await?;
            return Ok(());
        };
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[user {user_id}] {text}")).await?;
        }
        Ok(())
    }
}

async fn verify_token(_token: &str) -> Option<i64> { Some(42) }
```

**Padrão 2 - bloqueie o upgrade com um middleware de rota.** Rejeite
aberturas não autorizadas antes que qualquer frame flua. Separação de
responsabilidades mais limpa; o handler só vê conexões autenticadas.

```rust
ws!("/ws/private", PrivateChatHandler)
    .middleware(AuthMiddleware::new()),
```

`AuthMiddleware` retorna 401 em solicitações não autenticadas; o
upgrade é abortado com a resposta de rejeição e o handler nunca é
chamado.

**Padrão 3 - bloqueio por middleware mais releitura no handler.** O
middleware faz short-circuit em aberturas não autorizadas; o handler
então relê a mesma credencial (token, cookie, etc.) que sabe estar
presente agora para identificar qual usuário acabou de se conectar:

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    // O middleware já validou o bearer; só chegamos aqui se ele era válido.
    let token = req.bearer_token().expect("auth middleware vetted bearer presence");
    let user_id = lookup_user_by_token(&token).await?;
    // ...
}
```

**Padrão 4 - deixe o middleware autenticar e leia o resultado.**
Preferido quando um middleware de auth já executa no upgrade. A
identidade que ele resolveu é carregada na própria solicitação:

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    let Some(user_id) = req.auth_user_id() else {
        socket.close(1008, "unauthenticated").await?;
        return Ok(());
    };
    // `user_id` veio do middleware de sessão/token, não de nada que o
    // cliente enviou em um frame.
    socket.send_text(format!("welcome, {user_id}")).await?;
    Ok(())
}
```

É isso que torna o hook `authorize` de um canal de transmissão privado
significativo: ele recebe a mesma `Request`, então pode bloquear com
base em identidade derivada do servidor em vez de um valor que o
cliente escolheu. Antes de `auth_user_id` existir, um canal não tinha
nada confiável para consultar, e o placeholder óbvio - "aceitar
qualquer assinante cujo frame de assinatura carregue um token que
pareça correto" - não é um bloqueio de forma alguma.

Os acessadores thread-local que funcionam em controllers HTTP -
`session()`, `Auth::user()`, o conjunto `Context` por solicitação -
ainda **não** são populados dentro de um handler WebSocket. Os scopes
task-local da chain de middleware se desfazem quando a chain retorna;
o handler executa em uma task recém-spawnada que só herda o request id
e o id de auth resolvido. Leia tudo mais que o handler precisar direto
da `Request` (headers, cookies via `req.cookie("...")`, params
capturados, o bearer token via `req.bearer_token()`) - esses
sobrevivem para dentro da task do handler.

### Por que Suprnova diverge

O Laravel autoriza canais de transmissão através de um endpoint HTTP
separado (`/broadcasting/auth`), então o callback do canal executa em
uma solicitação comum com a sessão completa disponível. O Suprnova
autoriza in-process durante o upgrade em vez disso: uma conexão, sem
segunda round trip, o que significa que a identidade precisa ser
carregada explicitamente através da fronteira do spawn em vez de ser
consultada de novo.

## `WsConfig`

`WsConfig` controla o comportamento por conexão. Os padrões visam
endpoints públicos voltados para navegador: cada conexão ativa reserva
um buffer tungstenite do tamanho de `max_message_size`, então o
framework usa padrões pequenos e deixa rotas que precisam de mais
elevarem os limites explicitamente.

| Campo                 | Padrão         | Tipo            | Efeito |
|-----------------------|----------------|-----------------|--------|
| `ping_interval`       | 30s            | `Duration`      | Com que frequência o framework envia um frame Ping para manter a conexão viva. |
| `max_message_size`    | 1 MiB          | `usize`         | Tamanho máximo de mensagem reagrupada em bytes. Mensagens maiores são rejeitadas pelo tungstenite. |
| `max_frame_size`      | 64 KiB         | `usize`         | Tamanho máximo de um único frame WebSocket em bytes. |
| `max_missed_pings`    | 2              | `usize`         | Pongs perdidos consecutivos antes que o heartbeat feche a conexão com o código 1011. `usize::MAX` desabilita a imposição. |
| `origin_policy`       | `SameOrigin`   | `OriginPolicy`  | Verificação do header Origin aplicada no momento do upgrade. Veja [Política de origem](#política-de-origem). |
| `accepted_protocols`  | `vec![]`       | `Vec<String>`   | Tokens `Sec-WebSocket-Protocol` aceitos pelo servidor. Vazio significa sem negociação. Veja [Subprotocolos](#subprotocolos). |

Overrides recomendados por caso de uso:

- **Chat / notificações / posições de cursor** - os padrões estão
  bons. Reduza `ping_interval` para 5–10s se seu LB tiver um timeout
  de idle agressivo.
- **Feeds internos confiáveis** (fan-out servidor-para-servidor,
  exportação em massa, transferências binárias grandes) - comece a
  partir de `WsConfig::generous()`, que eleva `max_message_size` para
  64 MiB e `max_frame_size` para 16 MiB mantendo os outros padrões.
- **Payload específico fora do tamanho normal** (uma rota que faz
  upload de arquivos de áudio de 256 MiB) - defina os campos
  diretamente; não aplique o limite maior a rotas que não precisam
  dele.

A struct de config é construível via `Default` e todo campo é
público:

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let chat = WsConfig {
    ping_interval: Duration::from_secs(5),
    max_missed_pings: 1,
    ..Default::default()
};

let trusted = WsConfig::generous();
assert_eq!(trusted.max_message_size, 64 * 1024 * 1024);
assert_eq!(trusted.max_frame_size, 16 * 1024 * 1024);
```

Aplique o override por rota tanto na entrada `ws!` quanto em
`Router::ws_with_config`:

```rust
ws!("/ws/chat", ChatHandler).config(chat),
```

`WsConfig` é validado no registro da rota. Um `ping_interval` zero ou
um `max_missed_pings` zero corromperia a task de heartbeat; ambos são
rejeitados no boot em vez de entrar em panic na primeira conexão.

### Heartbeat e close por falta de pong

Para cada conexão que passou por upgrade o framework spawna uma task
de heartbeat que envia `Ping(b"")` a cada `ping_interval`. Em cada tick
o contador de pings perdidos incrementa; em cada Pong do peer ele
reseta para zero. Se o contador alcançar `max_missed_pings`, o
heartbeat envia Close(1011 "no pong response") e a conexão é
derrubada. Defina `max_missed_pings` para `usize::MAX` para desabilitar
a imposição (pings continuam fluindo, mas a conexão nunca é fechada por
pongs ausentes).

O primeiro tick é consumido no início da task, então o peer recebe
pelo menos um intervalo completo de graça antes do primeiro ping.

## Política de origem

Navegadores sempre enviam um header `Origin` em handshakes WebSocket.
Diferente de `fetch()` / `XMLHttpRequest`, upgrades WebSocket não são
protegidos por middleware de token CSRF (o handshake não carrega
token), então uma verificação de `Origin` de mesma origem é a única
coisa entre uma página maliciosa e um endpoint WS privilegiado na
sessão de um usuário logado. O framework aplica a política configurada
antes de `hyper_tungstenite::upgrade` ser chamado; uma violação retorna
HTTP 403 sem upgrade.

```rust
use suprnova::ws::{OriginPolicy, WsConfig};

let cfg = WsConfig {
    origin_policy: OriginPolicy::AllowList(vec![
        "https://app.example.com".into(),
        "https://admin.example.com".into(),
    ]),
    ..Default::default()
};
```

| Variante     | Comportamento |
|--------------|----------|
| `SameOrigin` (padrão) | Permite somente quando o host de `Origin` (e a porta, se presente) coincide com o header `Host` da solicitação. `Origin` ausente é rejeitado. O scheme não é comparado (o TLS termina upstream, então o servidor não consegue dizer com confiança se o scheme público era https ou http). |
| `AllowAny`   | Pula a verificação. Use somente para endpoints não-navegador (servidor-para-servidor, apps nativos, mocks de teste). |
| `AllowList(Vec<String>)` | Permite somente quando `Origin` coincide exatamente (sem diferenciar maiúsculas/minúsculas) com uma das origens fornecidas. Cada entrada é a forma completa `scheme://host[:port]` que um navegador enviaria. |

Clientes não-navegador (ferramentas CLI, servidores, apps nativos)
tipicamente não enviam um header `Origin`. Rotas que servem
exclusivamente esses clientes deveriam usar `AllowAny`; rotas que
servem ambos deveriam usar `AllowList` enumerando toda origem de
frontend de produção.

## Subprotocolos

Um subprotocolo WebSocket é um token de nível de aplicação (ex:
`graphql-transport-ws`, `jsonrpc-2.0`) que o cliente e o servidor
combinam durante o handshake. Popule `accepted_protocols` para
participar:

```rust
use suprnova::ws::WsConfig;

let cfg = WsConfig {
    accepted_protocols: vec![
        "graphql-transport-ws".into(),
        "graphql-ws".into(),
    ],
    ..Default::default()
};
```

Quando o cliente oferece `Sec-WebSocket-Protocol`, o framework escolhe
o primeiro token oferecido pelo cliente (na ordem de preferência do
cliente conforme o RFC 6455 §4.2.2) que coincide com
`accepted_protocols`, comparado sem diferenciar maiúsculas/minúsculas,
e o ecoa na resposta 101. Se o cliente ofereceu protocolos mas nenhum
coincidiu, o upgrade ainda tem sucesso sem header
`Sec-WebSocket-Protocol` - o RFC 6455 então exige que o navegador falhe
a conexão do lado do cliente, o que é o comportamento correto (um
servidor que seguisse adiante estaria silenciosamente falando o
protocolo errado).

Quando `accepted_protocols` está vazio, a negociação é pulada
inteiramente - a resposta de upgrade omite `Sec-WebSocket-Protocol` e o
cliente recai para o tratamento de protocolo padrão.

## Implantação em produção

O framework trata o handshake e o I/O de frames. Você não precisa de
nenhuma configuração extra do lado do framework para produção.

**A terminação de TLS acontece upstream.** Clientes conectam em
`wss://` no nginx, Caddy, ou no load balancer da cloud; o proxy remove
o TLS e repassa `ws://` puro para o framework. O framework não precisa
de uma feature `rustls` ou de um certificado TLS.

### nginx

```nginx
location /ws/ {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "Upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
}
```

`proxy_read_timeout` e `proxy_send_timeout` precisam ser longos o
suficiente para cobrir lacunas de idle entre heartbeats. Com o
`ping_interval` padrão de 30s, 3600s é um teto confortável.

### Caddy

```caddy
reverse_proxy /ws/* localhost:3000 {
    header_up Upgrade {http.request.header.Upgrade}
    header_up Connection "Upgrade"
}
```

O Caddy trata `Upgrade` / `Connection` automaticamente ao fazer proxy;
as diretivas `header_up` explícitas acima são para clareza.

### Load balancers de cloud (AWS ALB, GCP GLB)

Habilite suporte a WebSocket na regra do listener (o AWS ALB faz isso
automaticamente quando o protocolo do target group é HTTP/1.1 com
sticky sessions desligado). Garanta que o timeout de idle do load
balancer seja pelo menos tão longo quanto `ping_interval`; o heartbeat
do framework mantém a rede ativa, mas o LB derruba conexões que
pareçam ociosas da sua perspectiva.

## Shutdown gracioso

Todo handler WebSocket spawnado é rastreado no `JoinSet` `WS_TASKS` do
servidor. No `Ctrl-C` ou em um sinal de shutdown externo, o listener
para de aceitar novas conexões e `Server::run` drena o conjunto antes
do processo terminar. A future do handler não resolve até que o
handshake de close tenha sido feito flush: depois que o `handle` do
usuário retorna, o framework aguarda o forwarder para que o frame
final Close(1000) ou Close(1011) seja escrito na rede antes que a task
da conexão seja reportada como concluída. Em um shutdown limpo os
peers veem um close normal, não um reset de TCP.

Handles concluídos são colhidos oportunisticamente durante o tempo de
vida do servidor, então o `JoinSet` não cresce sem limite sob operação
de longa duração.

## Referência

| Símbolo | Propósito |
|---|---|
| `suprnova::ws::WebSocketHandler` | Trait: `async fn handle(&self, socket: WsSocket, request: Request) -> Result<(), FrameworkError>`. `Send + Sync + 'static`. |
| `suprnova::ws::WsSocket` | Handle bidirecional. Métodos: `send_text`, `send_binary`, `recv_text`, `recv`, `close`. `close` valida code + tamanho de reason antecipadamente. |
| `suprnova::ws::WsConfig` | Config por conexão. Campos: `ping_interval`, `max_message_size`, `max_frame_size`, `max_missed_pings`, `origin_policy`, `accepted_protocols`. Construtores `Default` + `generous()`. Validado no registro. |
| `suprnova::ws::OriginPolicy` | `SameOrigin` (padrão), `AllowAny`, `AllowList(Vec<String>)`. Aplicado no momento do upgrade. |
| `ws!(path, Handler)` | Forma de macro para `routes! { ... }`. Retorna um `WsRouteDef` suportando `.config(WsConfig)` e `.middleware(M)` em qualquer ordem. |
| `Router::ws(path, handler)` | Registro direto. Retorna `Router`. |
| `Router::ws_with_config(path, handler, cfg)` | Override de `WsConfig` por rota. |
| `Router::ws_with_middleware(path, handler, mws)` | Lista de middleware por rota. |
| `Router::ws_with_middleware_and_config(...)` | Ambos. |
| `Router::try_ws*` family | Irmãos falíveis - retornam `Err(FrameworkError)` em padrões duplicados ou malformados em vez de entrar em panic. |

## Próximos passos

- [Transmissão](broadcasting.md) - canais, presença, o protocolo de rede em cima de `ws!`
- [Eventos enviados pelo servidor](sse.md) - push unidirecional para navegadores atrás de proxies estritos
- [Roteamento](routing.md) - no que `routes!` e `ws!` realmente se expandem
- [Middleware](middleware.md) - escrevendo middleware que bloqueia HTTP e WS de forma uniforme
- [Solicitações](requests.md) - headers, cookies, query, extensions na `Request` que seu handler recebe

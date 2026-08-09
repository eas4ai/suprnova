# Transmissão

Transmissão é a camada de notificação servidor-para-cliente em cima da
[primitiva WebSocket](websockets.md) do Suprnova. Você despacha um
evento `Broadcastable` através de `EventFacade`; o framework faz
fan-out do envelope JSON do evento para todo assinante de WebSocket
nos canais que o evento nomeia. Você nunca gerencia conexões
individuais - você gerencia assinaturas de canal, e o hub faz o resto.

O `BroadcastHub` é o barramento. O `InMemoryBroadcastHub` padrão
executa inteiramente em processo - perfeito para deployments de
réplica única e a suíte de testes. Por trás da feature Cargo
`broadcasting-fanout`, o `SeaStreamerBroadcastHub` roteia os mesmos
eventos através de um broker de stream (Redis Streams, Kafka, file,
stdio) para que uma publicação em um processo alcance assinantes em
todo outro processo.

Tudo do capítulo [WebSocket](websockets.md) ainda se aplica -
heartbeat pings, `max_missed_pings`, `WsConfig`, middleware por rota,
parâmetros de caminho. Transmissão só adiciona um protocolo de rede e
um registry de canais em cima.

## Início rápido

Quatro arquivos e o navegador vê um evento.

`src/channels/order_updates.rs`:

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
}
```

`src/events/order_placed.rs`:

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        vec!["order.updates".into()]
    }
}
```

`src/bootstrap.rs`:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::container::App;
use suprnova::events::EventFacade;

pub async fn register() {
    // 1. Vincula o hub por trás da trait - handlers o resolvem de forma uniforme.
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    // 2. Registra todo canal antecipadamente; o handler WS resolve por nome.
    let mut registry = ChannelRegistry::new();
    registry.register(OrderUpdates);
    App::singleton(Arc::new(registry));

    // 3. Conecta a ponte evento → hub uma vez por tipo Broadcastable.
    EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
}
```

`src/routes.rs` - construa um `BroadcastingWsHandler` por rota
resolvendo o hub e o registry inicializados no boot a partir do
container:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{
    BroadcastHub, BroadcastingWsHandler, ChannelRegistry, InMemoryBroadcastHub,
};
use suprnova::container::App;
use suprnova::{routes, ws, AuthMiddleware};

fn broadcasting_handler() -> BroadcastingWsHandler {
    // Container primeiro; recorre a um hub em processo novo + registry vazio
    // para que testes unitários que montam o router sem bootstrap ainda funcionem.
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .unwrap_or_else(|| Arc::new(InMemoryBroadcastHub::new()));
    let registry: Arc<ChannelRegistry> = App::get::<Arc<ChannelRegistry>>()
        .unwrap_or_else(|| Arc::new(ChannelRegistry::new()));
    BroadcastingWsHandler::new(hub, registry)
}

routes! {
    ws!("/ws/broadcast", broadcasting_handler())
        .middleware(AuthMiddleware::new()),
}
```

Conecte e observe:

```bash
wscat -c ws://localhost:3000/ws/broadcast
> {"action":"connected","socket_id":"6f1a3c2e-…"}
> {"action":"subscribe","channel":"order.updates","data":{}}
< {"action":"subscribed","channel":"order.updates"}
```

Despache a partir de qualquer controller, worker, ou task agendada:

```rust
EventFacade::dispatch(OrderPlaced { order_id: 99, user_id: 42 }).await?;
```

```
< {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}
```

## Canais

Um canal é um alvo de assinatura nomeado. Clientes assinam por nome;
o hub entrega eventos a todo assinante ativo naquele nome. A trait
`Channel` tem padrões assimétricos que falham fechado nas escritas e
aberto nas leituras - veja [Por que Suprnova diverge](#por-que-suprnova-diverge)
abaixo.

### Canais públicos

O padrão. Qualquer cliente pode assinar.

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
    // authorize() tem como padrão true - aberto para todos os assinantes.
}
```

### Canais privados

Sobrescreva `authorize` para condicionar assinaturas. Um subscribe
rejeitado produz um frame `error` com `reason: "unauthorized"`;
nenhum frame `subscribed` é enviado.

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct PrivateChat;

#[async_trait]
impl Channel for PrivateChat {
    fn name(&self) -> &'static str { "chat.private" }

    async fn authorize(
        &self,
        _req: &Request,
        _params: &ChannelParams,
        data: &Value,
    ) -> bool {
        data["token"].as_str().map(|t| t == "valid").unwrap_or(false)
    }
}

impl PrivateChannel for PrivateChat {}
```

`data` é o que quer que o cliente tenha enviado no campo `data` do
frame de subscribe - um bearer token, um channel-bind assinado,
qualquer coisa definida pela aplicação. `Request` é a solicitação
original de upgrade HTTP (headers e cookies são legíveis diretamente).
`params` carrega os valores capturados de um nome parametrizado e
fica vazio para nomes fixos.

`PrivateChannel` é uma trait marcadora. O framework não verifica por
ela em runtime - é um sinal em nível de tipo de que o canal sobrescreve
`authorize` e se destina a ferramentas futuras (um lint do clippy, um
passo de auditoria).

### Canais parametrizados

Embuta segmentos `{param}` em `name()` e um único registro serve toda
assinatura concreta que corresponda ao padrão - o mesmo modelo do
`Broadcast::channel('orders.{id}', …)` do Laravel. Valores capturados
alcançam todo hook como um mapa `ChannelParams`.

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct OrderChannel;

#[async_trait]
impl Channel for OrderChannel {
    fn name(&self) -> &'static str { "orders.{id}" }

    async fn authorize(
        &self,
        _req: &Request,
        params: &ChannelParams,
        _data: &Value,
    ) -> bool {
        let order_id = params.get("id").unwrap_or_default();
        // Condiciona no id capturado - o usuário da sessão é dono deste pedido?
        !order_id.is_empty()
    }
}

impl PrivateChannel for OrderChannel {}

// Um único registro serve orders.42, orders.99, orders.featured, …
registry.register(OrderChannel);
```

Cada `{param}` se liga a exatamente um segmento separado por ponto:
`orders.{id}` corresponde a `orders.42` mas não a `orders` ou
`orders.42.line`. A resolução prefere um registro de nome fixo exato
sobre qualquer padrão (`orders.featured` vence `orders.{id}` para
aquele nome específico), então o padrão mais específico (mais
segmentos literais), com o padrão lexicograficamente menor como
critério de desempate determinístico.

### Canais de presença

Canais de presença rastreiam membresia. Quando um cliente assina, o
hub entrega um snapshot `presence.here` a esse cliente e transmite
`presence.joined` para todo outro assinante. Quando um cliente sai, o
hub transmite `presence.left`.

O contrato de duas partes é fácil de meio-implementar: você precisa
tanto sobrescrever `Channel::presence_info` para retornar `Some(self)`
QUANTO implementar `PresenceChannel::member_info`. Esquecer
`presence_info` conecta o canal como não-presença - subscribes
funcionam, mas `presence.joined` / `presence.here` / `presence.left`
nunca disparam.

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use suprnova::FrameworkError;
use suprnova::broadcasting::{Channel, ChannelParams, PresenceChannel};
use suprnova::http::Request;

pub struct PresenceLobby;

#[async_trait]
impl Channel for PresenceLobby {
    fn name(&self) -> &'static str { "presence.lobby" }

    // Obrigatório - sem essa sobrescrita, PresenceChannel é conectado mas inerte.
    fn presence_info(&self) -> Option<&dyn PresenceChannel> {
        Some(self)
    }
}

#[async_trait]
impl PresenceChannel for PresenceLobby {
    async fn member_info(
        &self,
        _req: &Request,
        _params: &ChannelParams,
    ) -> Result<Value, FrameworkError> {
        // Retorna o que outros assinantes precisam para identificar este
        // membro - tipicamente um user id. Nunca inclua segredos ou PII privada.
        Ok(json!({ "user_id": 42, "display_name": "Alice" }))
    }
}
```

Veja [Presença](#presença) para o fluxo de evento completo e o eco de
self-join.

### Nomes reservados

Nomes começando com `__` são reservados para meta-canais do framework
(`__presence__` carrega replicação de presença entre processos).
Chamar `registry.register(channel)` em um nome prefixado com `__`
entra em panic no registro, então o erro é pego no boot, não em
runtime.

### Por que Suprnova diverge

O Laravel vincula autorização de canal a um parâmetro de callback
`$user` porque o PHP injeta implicitamente o usuário autenticado
atual. O `authorize` do Suprnova em vez disso recebe a `Request` bruta,
o `ChannelParams` capturado, e um `data: Value` arbitrário - três
entradas ortogonais, todas disponíveis, sem contexto implícito. Você
lê o cookie de sessão ou o bearer token de `Request` e os params no
estilo de roteamento de `ChannelParams`; o payload `data` é um slot
livre para tokens que o cliente fornece no momento do subscribe.

Os padrões da trait `Channel` são **assimétricos de propósito**:
`authorize` tem como padrão `true` (subscribe é público por padrão),
`authorize_publish` tem como padrão `false` (publish iniciado pelo
cliente é negado por padrão). A ação perigosa falha fechada; a segura
falha aberta. Na dúvida, deixe as duas como estão.

## A trait Broadcastable

`Broadcastable: Event + Serialize` - todo `Broadcastable` também é um
`Event`. Dispatch via `EventFacade::dispatch(event)` executa todo
listener em processo E envia o payload serializado em JSON para todo
assinante de WebSocket nos canais que o evento nomeia.

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        // Um evento, múltiplos canais. Cada assinante em cada canal
        // recebe o mesmo envelope.
        vec![
            format!("user.{}.orders", self.user_id),
            "orders.global".into(),
        ]
    }
}
```

Conecte a ponte uma vez por tipo Broadcastable no boot:

```rust
EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
```

Depois disso, `EventFacade::dispatch(event).await?` é o lado de envio
inteiro - sem chamada `publish` separada.

Por padrão o evento é serializado via `serde_json::to_value(&event)` e
enviado para todo assinante. Canais com zero assinantes são pulados
silenciosamente no hub em processo; o hub entre processos ainda os
publica para que outros processos tenham a chance de entregar.

Quatro métodos opcionais refinam o padrão:

**`broadcast_event_name(&self) -> &'static str`** - sobrescreve o nome
do evento na rede. O padrão é `Self::event_name()`. Use para separar
a identidade do evento em processo do nome que vai pela rede.

**`broadcast_with(&self) -> Option<Value>`** - retorne `Some(value)`
para enviar um payload selecionado a dedo em vez da serialização
completa do evento (o `broadcastWith()` do Laravel). Omita segredos ou
remolde para o cliente sem mudar o tipo do evento:

```rust
impl Broadcastable for AccountFunded {
    fn broadcast_on(&self) -> Vec<String> {
        vec![format!("account.{}", self.account_id)]
    }
    fn broadcast_with(&self) -> Option<serde_json::Value> {
        // Nunca ponha o saldo na rede - somente o id público.
        Some(serde_json::json!({ "account_id": self.account_id }))
    }
}
```

**`broadcast_when(&self) -> bool`** - retorne `false` para despachar o
evento para listeners em processo mas pular o push de WebSocket (o
`broadcastWhen()` do Laravel). Somente a transmissão é condicionada; o
resto do pipeline de evento executa normalmente:

```rust
impl Broadcastable for DraftSaved {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("doc.{}", self.doc_id)] }
    fn broadcast_when(&self) -> bool { self.publish } // só transmite ao publicar
}
```

**`broadcast_to_others(&self) -> bool`** - retorne `true` para excluir
a conexão que disparou a transmissão (o `toOthers()` do Laravel). O
framework atribui a cada conexão de transmissão um `socket_id` na
conexão (enviado no frame `connected`); o navegador o ecoa de volta
como o header `X-Socket-ID` em solicitações HTTP; um evento
`broadcast_to_others` despachado enquanto trata essa solicitação pula
a conexão de origem. Fora de solicitação (um worker ou job) ou quando
nenhum `X-Socket-ID` está presente, degrada para transmitir para
todos:

```rust
impl Broadcastable for MessagePosted {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("chat.{}", self.room)] }
    fn broadcast_to_others(&self) -> bool { true } // o remetente já tem isso
}
```

Essa é uma escolha por tipo de evento. Para exclusão por dispatch,
publique diretamente:

```rust
use suprnova::broadcasting::BroadcastEnvelope;

hub.publish(
    BroadcastEnvelope::new(channel, event, data).with_except(socket_id),
).await?;
```

### Ordem de dispatch com listeners irmãos

`EventFacade::dispatch` é **fail-fast**: se uma publicação no hub
retorna `Err` (ex: desconexão de broker em um hub entre processos), o
`BroadcastListener` retorna `Err` e nenhum listener irmão registrado
**depois** dele executa. Duas formas de tratar isso:

- Registre a ponte de transmissão DEPOIS de listeners em processo cujos
  efeitos colaterais (escritas no DB, emissão de log) precisam
  executar independente do resultado da transmissão.
- Troque para `EventFacade::dispatch_best_effort(event)` quando todo
  listener precisar executar independente de um retornar `Err`.

Hubs em memória nunca retornam `Err` - só a variante entre processos
expõe falhas de broker.

## O protocolo de rede

Toda mensagem sobre a rota de transmissão é um frame JSON UTF-8. Duas
formas: `ClientFrame` (cliente → servidor) e `ServerFrame` (servidor →
cliente).

### Frames de cliente

| `action` | Campos obrigatórios | Campos opcionais | Significado |
|----------|-----------------|-----------------|---------|
| `subscribe` | `channel` | `data` | Assina `channel`. `data` é repassado para `Channel::authorize`. |
| `unsubscribe` | `channel` | | Desanexa de `channel`. |
| `publish` | `channel`, `event`, `data` | | Envia um evento a todo assinante em `channel`. Condicionado a `Channel::authorize_publish` E exige uma assinatura ativa. |

`publish` iniciado pelo cliente é condicionado a **duas**
verificações: a conexão PRECISA ter uma assinatura autorizada no
canal alvo, E `Channel::authorize_publish` precisa retornar `true`
(o padrão é `false`). Isso espelha o contrato de client-event do
Pusher - canais que querem publishes de cliente optam por participar
explicitamente sobrescrevendo o hook. A maioria dos canais de
transmissão do lado do servidor nunca quer eventos iniciados pelo
cliente, e a forma de negação-por-padrão corresponde a essa intenção.

```json
{"action":"subscribe","channel":"chat.42","data":{"token":"abc"}}
{"action":"unsubscribe","channel":"chat.42"}
{"action":"publish","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
```

### Frames de servidor

| `action` | Campos | Significado |
|----------|--------|---------|
| `connected` | `socket_id` | Enviado uma vez, primeiro. Ecoe `socket_id` como o header HTTP `X-Socket-ID` para que `broadcast_to_others` do lado do servidor possa excluir esta conexão. |
| `subscribed` | `channel` | Assinatura aceita. |
| `unsubscribed` | `channel` | Cancelamento de assinatura confirmado. |
| `event` | `channel`, `event`, `data` | Um evento foi transmitido em `channel`. |
| `lagged` | `channel`, `skipped` | O assinante ficou atrás do ring buffer por canal do servidor e envelopes `skipped` foram descartados nesta conexão. O estado local do cliente em `channel` está obsoleto; refaça o fetch antes de processar outros eventos. |
| `error` | `channel` (nullable), `reason` | A última ação falhou. `channel` é `null` para erros de nível de envelope não ligados a um canal. |

```json
{"action":"connected","socket_id":"6f1a3c2e-…"}
{"action":"subscribed","channel":"chat.42"}
{"action":"unsubscribed","channel":"chat.42"}
{"action":"event","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
{"action":"lagged","channel":"chat.42","skipped":42}
{"action":"error","channel":"chat.42","reason":"unauthorized"}
{"action":"error","channel":null,"reason":"malformed envelope: …"}
```

#### Sobre `lagged`

Todo canal tem um ring buffer por processo (256 envelopes). Um
assinante que não drena rápido o suficiente - um cliente lento, um
forwarder travado - fica atrás, e o buffer sobrescreve os eventos mais
antigos. Quando isso acontece, o servidor envia um frame `lagged`
nomeando o canal e a contagem de eventos descartados, então continua
entregando os frames subsequentes normalmente. O gap **não** é
recuperável do lado do servidor; o cliente precisa refazer o fetch ou
resincronizar antes de processar outros eventos naquele canal.
Descartar eventos silenciosamente deixaria bugs se escondendo como
"perdemos um tick" em vez de "o estado do cliente divergiu do estado
do servidor".

#### Falhas de publish

Quando um `publish` iniciado pelo cliente é aceito por
`authorize_publish` mas a publicação no hub em si falha (desconexão de
broker no hub entre processos), o cliente de origem recebe um frame
`error` com `reason: "publish failed: …"` para que saiba que o evento
não alcançou outros processos. Outros assinantes não são notificados.

### Sessão de exemplo

```
S → C  {"action":"connected","socket_id":"6f1a3c2e-…"}
C → S  {"action":"subscribe","channel":"order.updates","data":{}}
S → C  {"action":"subscribed","channel":"order.updates"}

# O servidor despacha OrderPlaced:
S → C  {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}

C → S  {"action":"subscribe","channel":"chat.private","data":{"token":"bad"}}
S → C  {"action":"error","channel":"chat.private","reason":"unauthorized"}

C → S  {"action":"unsubscribe","channel":"order.updates"}
S → C  {"action":"unsubscribed","channel":"order.updates"}
```

## Middleware por rota

Rotas de transmissão suportam o mesmo encadeamento `.middleware(M)`
que rotas WebSocket comuns:

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new()),
```

Uma resposta não-2xx de qualquer middleware faz short-circuit no
upgrade - o cliente recebe a resposta de erro HTTP e nenhum handshake
de WebSocket acontece. Este é o lugar certo para aplicar auth de
nível de transporte (validade de sessão, verificações de origem,
limites de taxa no momento da conexão) sem duplicar a verificação
dentro do `authorize` de cada canal.

Múltiplos middlewares se compõem da esquerda para a direita:

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

A separação é intencional: **de nível de transporte** (quem pode
abrir a conexão) vive no middleware; **de nível de canal** (quem pode
assinar qual canal) vive em `Channel::authorize`.

### `WsConfig` por rota

Sobrescreva os padrões de WebSocket de todo o processo por rota.
Encadeie `.config(WsConfig { ... })` depois do handler - antes ou
depois de `.middleware(M)` (a ordem não importa):

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

ws!("/ws/chat", broadcasting_handler())
    .config(WsConfig {
        ping_interval: Duration::from_secs(5),
        max_missed_pings: 1,
        ..Default::default()
    })
    .middleware(AuthMiddleware::new())
```

Os cinco campos configuráveis e onde cada um importa:

| Campo | Padrão | Caso de uso |
|-------|---------|----------|
| `ping_interval` | 30s | Chat / presença: reduza para 5–10s para detectar rapidamente conexões móveis mortas. Streaming de dados em massa: aumente para reduzir overhead. |
| `max_missed_pings` | 2 | Defina como `1` para chat onde um Pong perdido deveria fechar imediatamente. Defina como `3+` para redes móveis instáveis. Defina como `usize::MAX` para desabilitar close-on-no-pong. |
| `max_message_size` | 1 MiB | Padrão seguro para endpoint público. Comece a partir de `WsConfig::generous()` (64 MiB) para feeds internos confiáveis. |
| `max_frame_size` | 64 KiB | Dimensionado para frames de chat / notificação com margem. Comece a partir de `WsConfig::generous()` (16 MiB) para frames grandes não fragmentados. |
| `origin_policy` | `SameOrigin` | Os padrões rejeitam upgrades cross-origin - a única proteção CSRF que um handshake WS de navegador tem. Use `AllowList(vec![...])` para frontends cross-origin explícitos, ou `AllowAny` somente para endpoints não-navegador. |

Quando nenhum `.config(...)` é fornecido, a rota herda
`WsConfig::default()`. Config explícita por rota sempre vence sobre o
padrão.

Para rotas servindo feeds internos confiáveis (fan-out
servidor-para-servidor, transferências binárias grandes), comece a
partir da factory de feed confiável e ajuste conforme necessário:

```rust
use suprnova::ws::WsConfig;
use std::time::Duration;

ws!("/ws/internal/firehose", FirehoseHandler::new())
    .config(WsConfig {
        ping_interval: Duration::from_secs(10),
        ..WsConfig::generous() // 64 MiB message / 16 MiB frame
    })
```

## Presença

Quando um cliente assina com sucesso um canal de presença o hub:

1. Chama `PresenceChannel::member_info` com a `Request` de upgrade e o
   `ChannelParams` capturado para coletar os dados do membro que está
   entrando.
2. Envia um frame de evento `presence.here` ao novo assinante com
   `data: { "members": [...] }` - um snapshot de todos os membros
   atualmente rastreados (excluindo o que está entrando agora).
3. Publica um evento `presence.joined` com `data: <member_info>` no
   canal. Todo assinante - incluindo o novo, via seu próprio
   forwarder - recebe, e clientes filtram o self-join comparando a
   identidade do membro que entrou com a própria.

Quando um assinante desconecta ou envia um frame de unsubscribe:

4. O hub publica um evento `presence.left` com os dados do membro que
   saiu. Todo assinante restante recebe.

Os três frames chegam como frames de ação `event` com nomes de
`event` reservados:

```json
{"action":"event","channel":"presence.lobby","event":"presence.here","data":{"members":[{"user_id":1},{"user_id":2}]}}
{"action":"event","channel":"presence.lobby","event":"presence.joined","data":{"user_id":3}}
{"action":"event","channel":"presence.lobby","event":"presence.left","data":{"user_id":3}}
```

Entre processos, o estado de presença é replicado via o meta-canal
reservado `__presence__` (veja [Fan-out entre processos](#fan-out-entre-processos)).
Operações track e untrack em qualquer processo se propagam para todo
assinante; `list_members` retorna a visão mesclada (local + remota).
Processos mortos cujo `untrack_member` nunca disparou têm seus membros
removidos via TTL - padrão 60 s.

## Fan-out entre processos

O `InMemoryBroadcastHub` padrão faz fan-out somente para assinantes
no processo atual. Para deployments multi-réplica, habilite a feature
Cargo `broadcasting-fanout` e troque para `SeaStreamerBroadcastHub`:

`Cargo.toml`:

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.0", features = ["broadcasting-fanout"] }
```

`src/bootstrap.rs`:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry};
use suprnova::broadcasting::fanout::SeaStreamerBroadcastHub;
use suprnova::container::App;

pub async fn register() {
    let hub: Arc<dyn BroadcastHub> = Arc::new(
        SeaStreamerBroadcastHub::new(
            "redis://broker:6379",   // URI do streamer (backend escolhido pelo scheme)
            "suprnova-broadcast",    // stream key (compartilhada por todo processo no cluster)
        )
        .await
        .expect("connect"),
    );
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));
    // ... resto do bootstrap inalterado
}
```

O construtor recebe dois argumentos: a URI do streamer (seleciona o
backend em runtime pelo scheme) e a stream key (o nome do tópico
compartilhado por todo processo no cluster). Use a mesma stream key em
toda réplica ou elas não verão os eventos umas das outras.

`new_with_presence_ttl(uri, key, ttl)` sobrescreve o TTL de presença
padrão de 60 s - útil para testes que precisam exercitar o caminho de
recuperação de crash rapidamente. `new_loopback(uri, key)` habilita
loopback stdio para testes de integração de processo único; a
proteção contra duplicidade garante que cada evento da app ainda
entregue exatamente uma vez localmente.

### Backends

O backend é selecionado em runtime a partir do scheme da URI:

| Scheme de URI | Backend | Pronto para produção | Notas |
|------------|---------|------------------|-------|
| `redis://`, `rediss://` | Redis Streams | **Sim** | Recomendação padrão. `rediss://` usa TLS. Habilitado no build padrão. |
| `kafka://`, `kafka+ssl://` | Kafka | **Sim** | Exige `kafka` no conjunto de features do `sea-streamer` (`framework/Cargo.toml`). |
| `stdio://` | pipes stdin/stdout | Não - somente testes | Loopback de processo único. |
| `file://` | Arquivo local | Não - somente single-host | Exige `file` no conjunto de features do `sea-streamer`. |

O build padrão do Suprnova habilita `stdio` + `redis` + `socket`. Para
habilitar Kafka ou file, edite `framework/Cargo.toml` e adicione a
feature `sea-streamer` relevante.

### Arquitetura

Todo `publish(envelope)` faz duas coisas em paralelo:

1. **Fan-out local** - o `InMemoryBroadcastHub` interno entrega a
   assinantes neste processo imediatamente. Assinantes locais nunca
   esperam pela rede.
2. **Escrita no stream** - o mesmo envelope é serializado e enviado
   para o stream do sea-streamer para que a consumer pump de todo
   outro processo o capture e o entregue localmente.

Uma proteção contra entrega duplicada evita ver cada evento de dados
da app duas vezes: a instância do hub tem um UUID aleatório, todo
envelope que ela produz carrega aquele UUID, e a consumer pump pula
envelopes de entrada cujo instance id corresponde ao do próprio hub
local. Mensagens do meta-canal de presença são uma exceção - cada hub
precisa dos seus próprios eventos na visão entre processos para que o
caminho de leitura seja unificado.

O dispatch de backend é baseado em enum, não em trait-object: o hub
armazena um `SeaProducer` / `SeaConsumer` concreto do adaptador de
socket do sea-streamer, que é um enum sobre todo backend compilado.
Sem overhead de `dyn` no call site de publish.

### Presença entre processos

`SeaStreamerBroadcastHub` replica o estado de presença entre
processos automaticamente. Cada instância tem um `instance_id` UUID
na construção; `track_member` / `untrack_member` publicam
`PresenceEvent`s no meta-canal reservado `__presence__`. Todo processo
mantém uma `cross_process_view` atualizada pela sua consumer task;
`list_members` retorna a visão mesclada (local e remota de forma
uniforme).

Vivacidade: cada processo republica seus membros a cada `ttl / 6`
(10 s no TTL padrão de 60 s) como um heartbeat. Entradas obsoletas -
membros cujo `last_seen` excede o TTL - são removidas a cada `ttl /
2`. Isso trata crashes de processo que não chegaram a publicar
`MemberRemoved`.

## Close por falta de pong

Rotas de transmissão participam do mesmo heartbeat de WebSocket que
rotas `ws!` comuns. O framework envia um Ping a cada
`WsConfig::ping_interval` (padrão 30 s). Se uma conexão falha em
responder com um Pong dentro de `max_missed_pings` intervalos
consecutivos (padrão 2), o framework fecha com o código 1011.

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let config = WsConfig {
    ping_interval: Duration::from_secs(15),
    max_missed_pings: 3,
    ..WsConfig::default()
};
```

Reduzir `ping_interval` detecta conexões mortas mais rápido ao custo
de tráfego basal mais alto. `max_missed_pings: 1` fecha depois do
primeiro Pong perdido - use isso somente quando falhas de rede são
raras e você quer a limpeza mais rápida possível de conexões mortas.
`max_missed_pings: usize::MAX` desabilita close-on-no-pong
inteiramente.

## Implantação em produção

Rotas de transmissão são conexões HTTP que passaram por upgrade no
mesmo listener hyper que suas rotas HTTP. A terminação de TLS
acontece upstream, exatamente como descrito no [capítulo de
WebSocket](websockets.md#production-deployment). As configurações de
nginx e Caddy daquele capítulo se aplicam sem alteração - estenda-as
para cobrir o caminho `/ws/broadcast`.

Tasks ativas de handler WebSocket (incluindo conexões de transmissão)
são rastreadas no conjunto `WS_TASKS` do framework e drenadas no
shutdown gracioso, então entregas de evento em voo completam antes do
processo terminar.

## Testando transmissões

`RecordingBroadcastHub` é o análogo do Suprnova ao `Broadcast::fake()`
do Laravel - um `BroadcastHub` que registra todo envelope publicado
enquanto ainda entrega a assinantes ativos. Vincule-o no lugar de
`InMemoryBroadcastHub` em um teste e faça assert sobre o que foi
transmitido sem assinar primeiro:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, RecordingBroadcastHub};
use suprnova::container::App;

#[tokio::test]
async fn shipping_an_order_broadcasts_to_the_user_channel() {
    let hub = Arc::new(RecordingBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub) as Arc<dyn BroadcastHub>);

    // ... execute código que publica (diretamente, ou via um Broadcastable despachado) ...

    hub.assert_broadcast("orders.42", "OrderShipped");
    assert_eq!(hub.count(), 1);
}
```

| Helper                         | Verifica                                                 |
|--------------------------------|----------------------------------------------------------|
| `assert_broadcast(ch, ev)`     | pelo menos um envelope em `ch` com nome de evento `ev`   |
| `assert_nothing_broadcast()`   | nada foi publicado                                       |
| `broadcasts()`                 | `Vec<BroadcastEnvelope>` - todo envelope registrado      |
| `count()`                      | total de envelopes registrados                           |

Para verificar que um *evento* `Broadcastable` foi despachado
(independente do que alcançou a rede), `EventFacade::fake()` registra
o próprio evento - veja [Eventos](events.md#testing--eventfacadefake).

## Matriz de paridade do Laravel

| Laravel | Suprnova |
|---------|----------|
| `Broadcast::channel('name', fn(...))` | trait `Channel` + `registry.register(...)` |
| `Broadcast::channel('orders.{id}', ...)` | `fn name() -> "orders.{id}"`, params em `ChannelParams` |
| `PrivateChannel` (interface) | trait marcadora `PrivateChannel` + sobrescreva `authorize` |
| `PresenceChannel` (interface) | `PresenceChannel` + sobrescreva `Channel::presence_info` |
| `ShouldBroadcast` (interface) | trait `Broadcastable` |
| `broadcastOn()` | `broadcast_on(&self) -> Vec<String>` |
| `broadcastAs()` | `broadcast_event_name(&self) -> &'static str` |
| `broadcastWith()` | `broadcast_with(&self) -> Option<Value>` |
| `broadcastWhen()` | `broadcast_when(&self) -> bool` |
| `toOthers()` | `broadcast_to_others(&self) -> bool` |
| `Broadcast::fake()` | `RecordingBroadcastHub` vinculado como `dyn BroadcastHub` |
| `assertBroadcasted` | `RecordingBroadcastHub::assert_broadcast(channel, event)` |
| Driver Pusher / Reverb / Ably | `InMemoryBroadcastHub` (processo único) ou `SeaStreamerBroadcastHub` (entre processos: Redis / Kafka / file / stdio) |
| Biblioteca de cliente Echo | não incluída - conecte o protocolo de envelope JSON a partir do navegador manualmente por enquanto |

## Referência

| Símbolo | Propósito |
|--------|---------|
| `suprnova::broadcasting::Channel` | Trait de canal. Sobrescreva `name()` (obrigatório), `authorize`, `authorize_publish`, `presence_info`. |
| `suprnova::broadcasting::ChannelParams` | Valores capturados de um `name()` parametrizado. `get(key) -> Option<&str>`. Vazio para nomes fixos. |
| `suprnova::broadcasting::PrivateChannel` | Trait marcadora em um `Channel` que sobrescreve `authorize`. Nenhum método obrigatório. |
| `suprnova::broadcasting::PresenceChannel` | `async fn member_info(req, params) -> Result<Value, FrameworkError>`. Exige a sobrescrita de `Channel::presence_info`. |
| `suprnova::broadcasting::ChannelRegistry` | Mantém todo canal registrado. Vinculado como `Arc<ChannelRegistry>` no container; resolvido por `BroadcastingWsHandler`. |
| `suprnova::broadcasting::Broadcastable` | Trait sobre `Event + Serialize`. Obrigatório: `broadcast_on()`. Opcional: `broadcast_event_name`, `broadcast_with`, `broadcast_when`, `broadcast_to_others`. |
| `suprnova::broadcasting::BroadcastHub` | Trait de hub. `subscribe`, `publish`, `subscriber_count`, track/untrack/list de presença. |
| `suprnova::broadcasting::InMemoryBroadcastHub` | Hub em processo padrão. Sem dependências externas. Publish retorna `Ok` incondicionalmente. |
| `suprnova::broadcasting::RecordingBroadcastHub` | Duplo de teste. Registra todo publish; ainda entrega a assinantes ativos. |
| `suprnova::broadcasting::BroadcastEnvelope` | Um evento publicado: `channel`, `event`, `data`, `except`. Builder `new(ch, ev, data)`; `.with_except(socket_id)` para exclusão por dispatch. |
| `suprnova::broadcasting::ClientFrame` / `ServerFrame` | Os tipos de rede do envelope JSON. `ServerFrame::Lagged { channel, skipped }` expõe overflows de ring buffer por canal. |
| `suprnova::broadcasting::BroadcastingWsHandler` | O `WebSocketHandler` reutilizável do framework. Construtor: `BroadcastingWsHandler::new(hub, registry)`. Passe para `ws!()`. |
| `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub` | Hub entre processos por trás de `broadcasting-fanout`. `new(uri, stream_key)`, `new_with_presence_ttl(uri, key, ttl)`, `new_loopback(uri, key)`. |
| `EventFacade::broadcast::<E>(hub)` | Registra a ponte evento → hub para `E`. Chame uma vez por `Broadcastable` no boot. |
| `EventFacade::dispatch(event)` | Dispara listeners em processo E publica no hub em todo canal que `E::broadcast_on()` retorna. |
| `WsRouteDef::config(WsConfig)` | Override de config WS por rota. Se compõe com `.middleware(M)` em qualquer ordem. |
| `WsRouteDef::middleware(M)` | Chain de middleware por rota. Uma resposta não-2xx faz short-circuit no upgrade. |
| `WsConfig::generous()` | Factory de feed confiável: 64 MiB de mensagem / 16 MiB de frame, outros campos inalterados. NÃO use em rotas públicas. |

## Próximos passos

- [WebSockets](websockets.md) - a primitiva subjacente, `WsSocket`, `OriginPolicy`
- [Eventos](events.md) - `EventFacade`, dispatch fail-fast vs best-effort
- [Eventos enviados pelo servidor](sse.md) - push unidirecional sem um handshake de Upgrade
- [Notificações](notifications.md) - o driver de notificação `BroadcastChannel`
- [Web Push](web-push.md) - notificações enviadas pelo servidor para usuários offline

# Eventos enviados pelo servidor

Server-Sent Events (SSE) é o canal de push unidirecional mínimo do
servidor para o navegador: o navegador abre `EventSource(url)`, o
servidor mantém uma resposta `text/event-stream` aberta, e envia
eventos enquadrados via push conforme eles acontecem. Sem handshake de
WebSocket, sem permessage-deflate, sem bibliotecas de framing - apenas
linhas `data:`, `event:`, `id:`, `retry:` terminadas por uma linha em
branco, conforme a especificação
[WHATWG `EventSource`](https://html.spec.whatwg.org/multipage/server-sent-events.html).

A primitiva de SSE do Suprnova se encaixa no caminho de corpo em
streaming: construa um `Stream<Item = SseEvent>`, entregue-o a
`HttpResponse::sse(...)`, e o framework assume o gerenciamento de
conexão, o framing, os headers, e o isolamento de panic. A conexão
permanece aberta até o stream produtor terminar ou o cliente
desconectar.

## Quando recorrer a SSE vs WebSockets

| Propriedade | SSE | WebSockets |
|----------|-----|------------|
| Direção | Servidor → navegador | Bidirecional |
| Transporte | HTTP/1.1 ou HTTP/2 puro | Somente upgrade |
| Reconexão | Automática, com `retry:` e `Last-Event-ID` | Manual |
| Proxies / CDNs | Funciona através de qualquer coisa que permita respostas HTTP longas | Frequentemente precisa de suporte explícito a Upgrade |
| API do navegador | `EventSource` (nativa) | `WebSocket` (nativa) |
| Frames binários | Somente texto (UTF-8) | Texto ou binário |
| Limite de conexões por aba | 6 (HTTP/1.1) / ilimitado (HTTP/2) | Ilimitado |

Recorra a SSE quando você só precisa de push do servidor para o
cliente (feeds de atividade, notificações, streaming de logs,
streaming de IA). Recorra a [WebSockets](websockets.md) quando você
precisa de tráfego bidirecional ou frames binários.

## Início rápido

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub async fn stream_ticks(_req: Request) -> Response {
    let (tx, rx) = mpsc::channel::<SseEvent>(16);
    tokio::spawn(async move {
        for i in 0..10 {
            let evt = SseEvent::data(format!("tick {i}"))
                .with_event("tick")
                .with_id(i.to_string());
            if tx.send(evt).await.is_err() {
                break; // cliente desconectou
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Ok(HttpResponse::sse(ReceiverStream::new(rx)))
}
```

Saída de rede para um tick:

```text
event: tick
id: 0
data: tick 0

```

O navegador faz parse disso e dispara um evento `tick` com
`evt.data === "tick 0"` e `evt.lastEventId === "0"`.

## A API `SseEvent`

`SseEvent` é o tipo que você envia ao stream via push. Ele tem duas
variantes:

* **Frame** - um evento normal com `event` / `id` / `retry` opcionais
  e um payload `data` multilinha. Construído via
  [`SseEvent::data`](#construtores), `SseEvent::json`, ou
  `SseEvent::error`.
* **Comment** - um keep-alive visível apenas na rede (`:\n\n` ou
  `: <text>\n\n`). Construído via `SseEvent::comment(text)` ou
  `SseEvent::keep_alive()`. O navegador ignora comments por
  especificação; são os bytes atravessando a conexão que impedem
  proxies e load balancers ociosos de fechá-la.

### Construtores

| Construtor | Produz | Uso |
|-------------|----------|-----|
| `SseEvent::data(text)` | Frame apenas com linhas `data:` | O evento mínimo |
| `SseEvent::json(event, &payload)` | Frame com `event:` + `data:` em JSON | O caso dos 95% - `JSON.parse(evt.data)` no cliente |
| `SseEvent::error(message)` | Frame com `event: error` | Evento de erro de nível de domínio, distinto do `error` de nível de conexão que o navegador dispara em falha de transporte |
| `SseEvent::comment(text)` | Comment | Keep-alive com um marcador que o operador consegue identificar nos logs |
| `SseEvent::keep_alive()` | Comment vazio (`:\n\n`) | Heartbeat canônico de bytes mínimos |

### Builders

| Builder | Efeito | Em `Comment` |
|---------|--------|--------------|
| `.with_event(name)` | Define o campo `event:` | No-op silencioso |
| `.with_id(id)` | Define o campo `id:` - obrigatório para a semântica de resume | No-op silencioso |
| `.with_retry(Duration)` | Define o campo `retry:` (ms); a especificação diz que `Duration::ZERO` significa "reconectar imediatamente" | No-op silencioso |
| `.try_with_event(name)` | Variante falível - veja [Contrato de segurança](#contrato-de-segurança) | `Ok(self)` inalterado |
| `.try_with_id(id)` | Variante falível de `with_id` | `Ok(self)` inalterado |

Builders em `Comment` são no-ops de propósito - o formato de rede não
tem como expressar "comment com um nome de evento". Um uso incorreto
permanece silencioso em vez de converter o evento em um frame e
surpreender o produtor.

### Acessadores

| Método | Retorna |
|--------|---------|
| `.event()` | `Option<&str>` - o nome do evento, se definido |
| `.id()` | `Option<&str>` - o last-event-id, se definido |
| `.retry()` | `Option<Duration>` - o delay de reconexão, se definido |
| `.payload()` | `&str` - o payload de `data:` (ou `""` para `Comment`) |
| `.is_comment()` | `bool` |
| `.comment_text()` | `Option<&str>` - o texto do comment, se isto for um `Comment` |

### Codificação de rede

`SseEvent::to_wire()` serializa o evento para `Bytes` prontos para o
stream de corpo:

**Frame:**

```text
event: <event>\n   (somente se Some)
id: <id>\n         (somente se Some)
retry: <ms>\n      (somente se Some)
data: <line>\n     (uma por linha no payload, após normalização de \r/\r\n)
\n                 (terminador - exigido pela especificação)
```

**Comment:**

```text
: <line>\n         (uma por linha no texto do comment; `:\n` para linhas vazias)
\n                 (fronteira de flush)
```

## Contrato de segurança

O formato de rede do SSE usa CR / LF / NUL como terminadores de campo,
sem mecanismo de escape. Um produtor que deixa entrada do usuário
alcançar `event:` ou `id:` sem sanitizar exporia uma vulnerabilidade de
injeção de campo - um valor `"legit\ndata: injected"` produziria dois
campos `data:` na rede, e `"legit\n\nevent: spoofed"` terminaria o
evento atual e iniciaria um novo.

O `to_wire()` do Suprnova se defende em duas camadas:

* **valores dos campos `event:` e `id:`** - todo CR / LF / NUL é
  removido no momento da serialização. Um `WARN` estruturado dispara
  para cada remoção: `target: "suprnova::sse"`,
  `field = "event"|"id"`. O warn nunca loga o valor - esses bytes são
  controlados pelo atacante por construção.
* **`data:` e texto de comment** - `\r\n` e `\r` isolado são
  normalizados para `\n` antes do split, então um produtor que embute
  `\r` em um payload não consegue fazer o parser do receptor
  sintetizar um campo `data:` / `event:` / `id:` no momento do parse.
  NUL é removido do texto de comment com um `WARN` correspondente.

Se você quer **falhar rápido** em input inválido em vez de remover
silenciosamente, recorra aos irmãos `try_with_*`:

```rust
use suprnova::{Response, sse::SseEvent};

let evt = SseEvent::data("hello")
    .try_with_event(&user_supplied_event)?     // retorna Err em CR/LF/NUL
    .try_with_id(&user_supplied_id)?;
```

O `FrameworkError::validation(field, ...)` retornado nomeia o campo;
ele NÃO reflete o valor de volta, então um 400 exposto ao cliente é
seguro para logar.

## Keep-alive e timeouts de idle de proxy

Conexões SSE de longa duração são silenciosas por padrão. A maioria
dos deployments de produção fica atrás de um proxy / load balancer /
CDN que fecha conexões ociosas para liberar recursos:

* Padrão do nginx: 60 segundos
* Padrão do AWS ALB: 60 segundos
* Padrão do Cloudflare: 100 segundos

Um comment `keep_alive()` a cada 15-30 segundos mantém a conexão viva
através de todos esses sem despachar um evento `message` para o
navegador. A forma de bytes mínimos (`:\n\n`) é suficiente para fazer
flush dos buffers de escrita do proxy sem enviar nenhum payload.

```rust
use std::time::Duration;
use futures::StreamExt;
use suprnova::sse::SseEvent;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

let (tx, rx) = mpsc::channel::<SseEvent>(16);

// Task de heartbeat - independente do produtor de eventos.
let hb_tx = tx.clone();
tokio::spawn(async move {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        if hb_tx.send(SseEvent::keep_alive()).await.is_err() {
            break; // cliente sumiu
        }
    }
});

// Produtor de eventos ... envia frames para `tx` conforme acontecem.
```

## Resume após queda (`Last-Event-ID`)

Quando o `EventSource` do navegador perde a conexão, ele reconecta
automaticamente e envia o `id:` mais recente que viu como o header
`Last-Event-ID` na nova solicitação. Marque cada evento com
`.with_id(...)` e leia o header na solicitação de resume:

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::{self, SseEvent}};

pub async fn stream_from_resume(req: Request) -> Response {
    let resume_from: u64 = sse::last_event_id(&req)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Constrói o stream produtor a partir de `resume_from + 1` em
    // diante. O closure é dono do seu próprio contador, então a
    // mutação fica contida dentro do stream.
    let stream = futures::stream::iter(events_since(resume_from))
        .scan(resume_from + 1, |next_id, payload| {
            let id = *next_id;
            *next_id += 1;
            futures::future::ready(Some((id, payload)))
        })
        .map(|(id, payload)| {
            SseEvent::json("activity", &payload)
                .expect("payload is a Serialize value")
                .with_id(id.to_string())
        });

    Ok(HttpResponse::sse(stream))
}
```

`sse::last_event_id(&Request) -> Option<String>` retorna `None`
quando o header está ausente OU quando o valor contém um byte NUL
(pela especificação WHATWG, NUL invalida um last-event-id e o parser
do navegador o descartaria). A `String` retornada é, fora isso, input
de usuário opaco - faça parse dela como seu próprio cursor / sequência
/ offset antes de usá-la.

## Erros de nível de domínio

`SseEvent::error("...")` produz a forma convencional
`event: error\ndata: <msg>\n\n`. Assinantes podem escutá-lo
separadamente do `error` de nível de conexão que o navegador dispara
em falha de transporte:

```js
const es = new EventSource("/stream");

// Erros de conexão / transporte (sem `data`).
es.onerror = (evt) => console.warn("transport error", evt);

// Erros de nível de domínio emitidos por SseEvent::error(...).
es.addEventListener("error", (evt) => console.error("server-side:", evt.data));
```

Ao mapear um `Stream<Item = Result<T, E>>` para um
`Stream<Item = SseEvent>`, o padrão idiomático é
`map(|r| match r { Ok(x) => SseEvent::json(...), Err(e) => SseEvent::error(...) })` -
o mapeamento de erro do lado do consumidor fica nas mãos do produtor e
o framework nunca precisa inventar uma forma padrão.

## Transmitindo um stream para muitos assinantes

Fan-out para muitos assinantes de SSE já é coberto pelo
[subsistema de transmissão](broadcasting.md): assine um canal
`BroadcastHub` e adapte o `broadcast::Receiver` para o stream
`SseEvent` com `tokio_stream::wrappers::BroadcastStream` +
`.map(...)`. Cada conexão recebe seu próprio receiver; o hub trata a
política para consumidor lento (erros `Lagged(n)` quando um assinante
fica atrás) e você decide como expor isso ao cliente.

O exemplo funcional de dogfood em
`app/src/controllers/sse_example.rs` implementa isso em ~25 linhas:

```rust
use futures::StreamExt;
use std::sync::Arc;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio_stream::wrappers::BroadcastStream;

pub async fn stream(_req: Request) -> Response {
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .expect("BroadcastHub not bootstrapped");
    let rx = hub.subscribe("user_registered");

    let stream = BroadcastStream::new(rx).map(|result| match result {
        Ok(envelope) => SseEvent::json("user.registered", &envelope.data)
            .unwrap_or_else(|_| {
                SseEvent::data(envelope.data.to_string())
                    .with_event("user.registered")
            }),
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
            SseEvent::data(n.to_string()).with_event("lagged")
        }
    });

    Ok(HttpResponse::sse(stream))
}
```

O evento `lagged` permite que o cliente dispare um refetch completo e
um resume - a conexão permanece aberta durante o atraso.

## `event_stream` e `stream_json`

`HttpResponse::sse` toma controle total do enquadramento - você constrói cada
`SseEvent` por conta própria. Dois irmãos de nível mais alto cobrem os formatos
comuns:

```rust
use suprnova::sse::{EndSignal, StreamedEvent};
use suprnova::{HttpResponse, Request, Response};
use tokio::sync::mpsc;

pub async fn progress(_req: Request) -> Response {
    let (tx, rx) = mpsc::channel::<StreamedEvent>(16);
    tokio::spawn(async move {
        for pct in [25, 50, 75, 100] {
            let evt = StreamedEvent::message(pct).unwrap();
            if tx.send(evt).await.is_err() {
                break; // client disconnected
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(HttpResponse::event_stream(stream, EndSignal::default()))
}
```

`StreamedEvent::message(data)` define `event` como `"update"` por padrão - o que
`useEventStream` escuta imediatamente; `StreamedEvent::named(event, data)` o
substitui para um produtor que distribui mais de um canal lógico na mesma
conexão. `data` chega ao fio sem aspas para uma string simples, codificado em
JSON de outro modo. O argumento `end: EndSignal` de `event_stream` controla o
frame terminal enviado depois do fim do stream: `EndSignal::default()` envia
`event: update\ndata: </stream>\n\n` (o padrão do próprio Laravel e o que a
opção `endSignal` de `useEventStream` verifica); `EndSignal::None` o omite;
`EndSignal::text(...)` / `EndSignal::Event(...)` o personalizam. Este é o
`ResponseFactory::eventStream($callback, $headers, $endStreamWith)` do Suprnova.

`HttpResponse::stream_json(stream)` - `ResponseFactory::streamJson` /
`StreamedJsonResponse` do Laravel - recebe qualquer `Stream<Item = impl Serialize>`
e o descarrega como um array JSON construído incrementalmente
(`Content-Type: application/json`) em vez de armazenar toda a coleção primeiro
em buffer. Os bytes no fio são exatamente `[item,item,...]`; concatene toda a
resposta e ela desserializa com qualquer analisador JSON.

## Consumindo de React / Vue / Svelte

Os pacotes [`@laravel/stream-{react,vue,svelte}`](https://github.com/laravel/stream)
são donos do lado cliente deste contrato de fio - o Suprnova os visa em vez de
enviar o seu próprio:

| Hook | Fala com | Builder Suprnova |
|---|---|---|
| `useEventStream(url, options)` | `EventSource` (GET, reconexão gerida pelo navegador) | `HttpResponse::event_stream` |
| `useStream(url, options)` | `fetch` (POST, loop de leitura `ReadableStream` manual) | `HttpResponse::stream_bytes` |
| `useJsonStream(url, options)` | Igual a `useStream`, faz `JSON.parse` do resultado totalmente em buffer | `HttpResponse::stream_json` |

```tsx
import { useEventStream, useJsonStream } from "@laravel/stream-react";

const { message } = useEventStream("/progress");          // against an event_stream endpoint
const { data, send } = useJsonStream<Order[]>("/export"); // against a stream_json endpoint
```

`useStream`/`useJsonStream` fazem POST com dois headers que o Suprnova lê como
qualquer outro header de solicitação: `X-STREAM-ID` (um ID de correlação simples,
não autenticador, que o hook gera do lado cliente) e `X-CSRF-TOKEN`, lido de
`<meta name="csrf-token">` do mesmo modo que a [proteção CSRF](csrf.md) já
espera. `useEventStream` não envia nenhum dos dois - `EventSource` não pode
definir headers de solicitação personalizados, é um GET simples de navegador.

## Configuração de produção

### Cabeçalhos de resposta

`HttpResponse::sse(...)` define os headers exigidos para você:

| Header | Valor | Por quê |
|--------|-------|-----|
| `Content-Type` | `text/event-stream` | Definido pela especificação; o `EventSource` do navegador exige |
| `Cache-Control` | `no-cache` | Impede que intermediários façam cache do stream |
| `Connection` | `keep-alive` | Resposta de longa duração em HTTP/1.1 |
| `X-Accel-Buffering` | `no` | Desabilita o buffering de proxy do nginx - eventos fazem flush imediatamente. No-op fora do nginx |

### Ajustando a reconexão

O delay de reconexão padrão do navegador é 3 segundos. Envie um campo
`retry:` uma vez no início do stream para sobrescrevê-lo:

```rust
let preamble = SseEvent::data("ready").with_retry(Duration::from_secs(5));
```

`Duration::ZERO` é válido pela especificação ("reconectar
imediatamente") e é emitido ao pé da letra - sem coerção. Para streams
de produção, um retry de 5-15 segundos equilibra recuperação rápida
com não sobrecarregar o servidor durante uma queda regional.

### Por que Suprnova diverge

O Laravel entrega SSE como um helper pontual em `Response`:
`Response::eventStream(fn () => ...)` recebe um closure que gera
(yield) valores via generator e enquadra cada valor yielded como uma
linha `data:`. Ele não modela `event:` / `id:` / `retry:` como campos
de primeira classe, não tem primitiva de keep-alive embutida, e não
sanitiza valores que injetariam campos extras na rede.

O Suprnova trata SSE como um subsistema de verdade, não como um helper
pontual:

- `SseEvent` é um valor tipado com builders falíveis (`try_with_*`) e
  infalíveis (`with_*`), variantes `Frame` e `Comment` distintas, e um
  contrato de sanitização documentado em todo campo de linha única.
- `HttpResponse::sse(stream)` se encaixa no mesmo pipeline de corpo
  `stream_bytes` usado por qualquer outra resposta de longa duração,
  então SSE compartilha um único caminho de cancelamento, headers e
  isolamento de panic com o resto do framework.
- Produtores compõem qualquer `Stream<Item = SseEvent>` -
  `tokio::sync::mpsc`, `tokio::sync::broadcast`,
  `futures::stream::iter`, ou o adaptador de fan-out
  [BroadcastHub](broadcasting.md). Nenhum deles exige uma válvula de
  escape do framework.
- Um leitor de `Last-Event-ID` (`sse::last_event_id`) e a regra de
  descarte-por-NUL da WHATWG já vêm de fábrica, então resume após
  queda é uma chamada de parse de distância, em vez de um utilitário
  de header customizado por app.

## Referência

| Símbolo | Propósito |
|--------|---------|
| `suprnova::sse::SseEvent` | Uma peça emitível de um stream de SSE. Duas variantes - `Frame` (evento com `event` / `id` / `retry` opcionais + `data`) e `Comment` (keep-alive). |
| `SseEvent::data(text)` | Constrói um frame apenas com linhas `data:`. |
| `SseEvent::json(event, &payload)` | Constrói um frame cujo payload é `payload` serializado via `serde_json`; define `event:` como `event`. Retorna `Result<Self, serde_json::Error>`. |
| `SseEvent::error(message)` | Constrói um frame com `event: error` e a mensagem fornecida como `data`. |
| `SseEvent::comment(text)` | Constrói um evento somente-comment (`: <text>\n\n`). Invisível ao navegador; mantém proxies despertos. |
| `SseEvent::keep_alive()` | Abreviação para o comment vazio `:\n\n`. Heartbeat de bytes mínimos. |
| `.with_event(name)` / `.with_id(id)` / `.with_retry(Duration)` | Builders infalíveis em um `Frame`; no-op silencioso em um `Comment`. Removem CR / LF / NUL no momento do `to_wire()` com um WARN estruturado. |
| `.try_with_event(name)` / `.try_with_id(id)` | Irmãos falíveis - retornam `Err(FrameworkError::validation(...))` em CR / LF / NUL. Use quando o valor vem de input do usuário e você quer um 4xx em vez de uma remoção silenciosa. |
| `.event()` / `.id()` / `.retry()` / `.payload()` / `.is_comment()` / `.comment_text()` | Acessadores. `payload()` tem esse nome para evitar colidir com o construtor `data`. |
| `SseEvent::to_wire()` | Serializa para `Bytes` no formato de rede do SSE. Público para que testes e adaptadores possam codificar sem passar pelo builder de resposta. |
| `suprnova::sse::last_event_id(&Request) -> Option<String>` | Lê o header `Last-Event-ID`. Retorna `None` quando ausente OU quando o valor contém um byte NUL (a WHATWG descarta ids inválidos). |
| `suprnova::sse::last_event_id_from_value(Option<&str>)` | Helper puro que expõe o mesmo contrato de validação - testável com testes unitários sem construir uma `Request`. |
| `HttpResponse::sse(stream)` | Constrói uma resposta em streaming a partir de qualquer `Stream<Item = SseEvent> + Send + Sync + 'static`. Define `Content-Type`, `Cache-Control`, `Connection`, `X-Accel-Buffering`. |
| `suprnova::sse::StreamedEvent` | Um item enviado a um `event_stream` - `{ event: String, data: serde_json::Value }`. |
| `StreamedEvent::message(data)` / `StreamedEvent::named(event, data)` | Constrói sob o nome padrão `"update"` ou um explícito. Ambos retornam `Result<Self, serde_json::Error>`. |
| `suprnova::sse::EndSignal` | O frame terminal que `event_stream` envia quando o produtor termina - `None` / `Message(String)` / `Event(StreamedEvent)`. O `Default` é `text("</stream>")`. |
| `HttpResponse::event_stream(stream, end)` | Constrói uma resposta `event_stream` de qualquer `Stream<Item = StreamedEvent> + Send + Sync + 'static`. Construído sobre `sse`. |
| `HttpResponse::stream_json(stream)` | Constrói uma resposta `stream_json` de qualquer `Stream<Item = impl Serialize> + Send + Sync + 'static`. Construído sobre `stream_bytes`. |

## Próximos passos

- [WebSockets](websockets.md) - a outra conexão de longa duração, para quando você precisa de frames bidirecionais ou binários.
- [Transmissão](broadcasting.md) - fan-out de `BroadcastHub` compartilhado com assinantes de WebSocket.
- [Notificações](notifications.md) - drivers de canal para entrega via push não-streaming (mail, database, broadcast).
- [Web Push](web-push.md) - notificações enviadas pelo servidor via push que alcançam o cliente mesmo sem nenhum `EventSource` aberto.
- [Respostas](responses.md) - o resto da superfície de builder do `HttpResponse`.

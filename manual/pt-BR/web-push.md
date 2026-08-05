# Web Push

O Web Push entrega uma mensagem curta a um navegador mesmo quando seu
site está fechado - o Service Worker desperta, descriptografa o
payload, e mostra uma notificação de nível de sistema operacional. O
Suprnova entrega o protocolo de ponta a ponta: geração de chave VAPID,
criptografia de payload AES128GCM, o transporte HTTP, e um
`WebPushChannel` que se encaixa no subsistema de notificações, então a
mesma `Notification` que você envia para mail ou database também
chega como push.

Recorra a isso quando você quer alertar usuários em tempo real sem um
WebSocket aberto - pedido enviado, solicitação de amizade, menção,
saldo lançado. Se o usuário está em um navegador desktop com o site
fechado, web push é o único mecanismo que os alcança; se estão no
site, [Transmissão](broadcasting.md) geralmente é uma escolha melhor.

A API fica atrás da feature Cargo `web-push`, que é habilitada por
padrão. Aplicações usando `default-features = false` precisam
habilitar `web-push` explicitamente.

## As quatro peças

Web Push tem mais peças móveis que mail ou database, porque a
especificação ([RFC 8030](https://datatracker.ietf.org/doc/html/rfc8030) +
[RFC 8291](https://datatracker.ietf.org/doc/html/rfc8291) +
[RFC 8292](https://datatracker.ietf.org/doc/html/rfc8292)) divide
identidade, criptografia, e transporte em três contratos:

| Peça | O que é |
|---|---|
| `VapidKey` / `VapidSigner` | Um par de chaves ECDSA P-256 usado para assinar JWTs que provam que seu servidor é quem afirma ser |
| `WebPushClient` | O cliente HTTP que criptografa um payload, assina um JWT VAPID, e faz POST para o endpoint da assinatura |
| `WebPushChannel` | O adaptador do subsistema de notificações que transforma uma `Notification` em uma chamada `WebPushClient::send` |
| `SubscriptionInfo` | A tripla opaca (`endpoint`, `p256dh`, `auth`) que o navegador entrega a você quando um usuário assina - você a armazena; você não a gera |

As três camadas de baixo - `VapidKey`, `WebPushClient`, o POST
criptografado - são reexportadas de `suprnova::web_push`, então
aplicações nunca precisam depender diretamente da crate
`suprnova-web-push` subjacente.

## Gere um par de chaves VAPID

O Web Push usa VAPID (Voluntary Application Server Identification)
para deixar serviços de push aplicarem rate-limit e contatar
remetentes com mau comportamento. Você precisa de um par de chaves
P-256 por aplicação; a chave pública vai para seu frontend para que o
navegador possa fixar assinaturas ao seu servidor, e a chave privada
fica no servidor assinando JWTs.

Gere uma vez, persista, e reutilize para sempre:

```rust
use suprnova::VapidKey;

let key = VapidKey::generate();

// Salve o PEM em algum lugar durável - um secrets manager, um arquivo
// que o pipeline de deploy monta, um volume env-vars-as-files. Você
// NÃO PODE regenerar isso sem invalidar toda assinatura existente.
let pem = key.to_pem()?;
std::fs::write("vapid_private.pem", &pem)?;

// O frontend precisa da chave pública descomprimida em
// base64url-sem-padding. Entregue isso ao seu JS para que
// `pushManager.subscribe()` possa usá-la como `applicationServerKey`.
println!("PUBLIC_VAPID_KEY={}", key.public_key_uncompressed_b64url());
```

No boot, carregue o PEM salvo:

```rust
use suprnova::{VapidKey, VapidSigner};

let pem = std::fs::read_to_string("vapid_private.pem")?;
let key = VapidKey::from_pem(&pem)?;
let signer = VapidSigner::new(key);
```

Um `VapidSigner` produz JWTs mas não envia nada - é puramente uma
primitiva de assinatura. A próxima camada o envolve.

## Construa um WebPushClient

`WebPushClient` é a primitiva do lado HTTP: alimente-a com um signer
e uma URI de contato ("como o serviço de push pode alcançar você se
você se comportar mal"), e receba de volta um objeto cujo método
`send` criptografa um payload, assina um JWT, e faz POST para o
endpoint da assinatura.

```rust
use std::sync::Arc;
use suprnova::{VapidKey, VapidSigner, WebPushClient};

let signer = VapidSigner::new(VapidKey::from_pem(&pem)?);

// O subject DEVE ser uma URI mailto: ou uma URL https: conforme o RFC
// 8292 §2.1. Qualquer outra coisa é rejeitada na construção, então um
// deploy malconfigurado falha rápido no boot - não silenciosamente
// depois do primeiro dispatch com falha.
let client = WebPushClient::new(signer, "mailto:ops@example.org")?;

let client = Arc::new(client);
```

Por que `Arc<WebPushClient>`? `WebPushClient` envolve um
`VapidSigner`, que envolve um `ES256KeyPair` privado. Nenhum desses é
`Clone` - chaves privadas não deveriam ser duplicadas casualmente - e
construir um signer novo para cada registro de canal significaria N
identidades VAPID independentes para a mesma aplicação. Envolver em
`Arc` deixa uma única identidade assinada apoiar todo registro e toda
entrega concorrente.

### Política de endpoint

Endpoints de assinatura são dados derivados do usuário: o navegador
recebe a URL de um serviço de push remoto quando um usuário assina, e
seu servidor armazena o que quer que o navegador tenha devolvido. Uma
assinatura armazenada maliciosamente pode apontar o POST HTTP para
qualquer lugar alcançável, transformando o remetente de push em um
gadget de SSRF.

`WebPushClient` usa `EndpointPolicy::Strict` por padrão:

- O scheme precisa ser `https`
- O host precisa ser um domínio nomeado, não um literal de IP
- Hostnames de cloud-metadata e TLDs reservados pela RFC 2606
  (`.localhost`, `.local`, `.internal`, `.test`, `.example`,
  `.invalid`) são rejeitados

Isso bloqueia as sondas de SSRF óbvias sem quebrar serviços de push
reais (FCM, Mozilla Autopush, o `web.push.apple.com` da Apple).

Para testes de integração locais contra um mock server `wiremock`
você precisa fazer opt-out:

```rust
use suprnova::{EndpointPolicy, WebPushClient};

let client = WebPushClient::new(signer, "mailto:test@example.org")?
    .with_endpoint_policy(EndpointPolicy::AllowAny);
```

Não use `AllowAny` em produção. As verificações estritas existem para
impedir que uma tabela de assinaturas adulterada seja usada como arma.

### Transporte customizado

`WebPushClient::new` aplica um timeout de 30 segundos por solicitação.
Se você precisa de uma política de transporte diferente - proxy
corporativo, TLS fixado, timeout mais curto - construa um
`reqwest::Client` e use `WebPushClient::with_client`:

```rust
use reqwest::Client;
use std::time::Duration;
use suprnova::WebPushClient;

let http = Client::builder()
    .timeout(Duration::from_secs(10))
    .build()?;

let client = WebPushClient::with_client(http, signer, "mailto:ops@example.org")?;
```

## Conecte o WebPushChannel às notificações

O `WebPushClient::send` bruto funciona, mas a forma como você
realmente envia notificações push no Suprnova é através do subsistema
de [Notificações](notifications.md). Uma `Notification` declara
`vec!["webpush"]` em seu `channels()`, um destinatário `Notifiable`
retorna um `SubscriptionInfo` codificado em JSON a partir de
`route_for("webpush")`, e o `NotificationDispatcher` vinculado faz o
fan-out.

```rust
use std::sync::Arc;
use suprnova::{
    NotificationDispatcher, WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};

let client: Arc<WebPushClient> = Arc::new(
    WebPushClient::new(signer, "mailto:ops@example.org")?
);

// ttl_secs: por quanto tempo o serviço de push mantém uma mensagem
// não entregue. 86_400 (24h) é um padrão razoável para notificações
// não urgentes; reduza para 60 para alertas "aja agora" em que uma
// mensagem obsoleta é peor que nenhuma mensagem.
let webpush = Arc::new(WebPushChannel::new(client, 86_400));

let dispatcher = NotificationDispatcher::new()
    .register_channel(webpush);

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` é last-write-wins no `name()` do canal, então
testes podem trocar por um stub sem afetar a vinculação de produção.

## Defina uma notificação

Uma notificação vinculada a push tem a mesma forma que qualquer outra
notificação do Suprnova - declare `"webpush"` em `channels()` e
coloque qualquer JSON que você queira entregar em `data()`:

```rust
use serde::{Deserialize, Serialize};
use suprnova::Notification;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OrderShipped {
    pub order_id: i64,
    pub tracking_url: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str {
        "OrderShipped"
    }

    fn channels(&self) -> Vec<&'static str> {
        vec!["webpush"]
    }

    fn data(&self) -> serde_json::Value {
        serde_json::json!({
            "title":   "Your order has shipped",
            "body":    format!("Track order #{}", self.order_id),
            "url":     self.tracking_url,
        })
    }
}
```

O JSON de `data()` é o que seu Service Worker recebe. Escolha uma
forma estável e documente-a para o frontend - o Suprnova não impõe
uma, porque a UI de notificação é uma responsabilidade do frontend.

## Roteie o destinatário

Um `Notifiable` retorna a rota para cada canal que suporta. Para Web
Push, essa rota é o `SubscriptionInfo` codificado em JSON - exatamente
o que o navegador produziu via `PushSubscription.toJSON()`, armazenado
ao pé da letra:

```rust
use suprnova::Notifiable;

pub struct User {
    pub id: i64,
    pub push_subscription_json: Option<String>,
}

impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "webpush" => self.push_subscription_json.clone(),
            _ => None,
        }
    }
}
```

Retornar `None` faz o dispatcher pular o canal silenciosamente - útil
para usuários que não assinaram push mas ainda recebem email.

## Envie

Síncrono:

```rust
use suprnova::Notify;

let user = User::find(42).await?.unwrap();
Notify::send(&user, &OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

Em fila - resolve antecipadamente a rota da assinatura no momento de
enfileirar, então o worker não precisa recarregar o usuário:

```rust
Notify::queue(&user, OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

Para `Notify::queue` funcionar, registre a factory da notificação no
boot para que o worker possa reconstruir o payload JSON na notificação
tipada:

```rust
suprnova::notifications::register_notification_factory::<OrderShipped>()?;
suprnova::queue::worker::register_job::<suprnova::SendNotificationJob>();
```

Por baixo dos panos, o dispatch em fila constrói um
`SendNotificationJob` carregando `(notification_name, payload,
per_channel_routes, channels)`. O worker rehidrata a notificação,
procura `WebPushChannel` por nome no dispatcher vinculado, e chama
`deliver(route, &notification)` - o mesmo caminho de código do
`Notify::send` síncrono.

## O lado do navegador

O Suprnova não entrega um SDK JavaScript - o lado do navegador é a Web
Push API pura. O fluxo que seu frontend precisa implementar:

1. Registre um Service Worker.
2. Peça permissão ao usuário.
3. Assine via `pushManager.subscribe({ userVisibleOnly: true,
   applicationServerKey: <your VAPID public key> })`.
4. Faça POST de `subscription.toJSON()` para um endpoint do Suprnova
   que o armazena na linha do usuário.

```js
// Registro do Service Worker (em algum lugar no entrypoint da sua app)
const registration = await navigator.serviceWorker.register('/sw.js');

if (Notification.permission === 'default') {
    await Notification.requestPermission();
}

if (Notification.permission === 'granted') {
    const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: window.PUBLIC_VAPID_KEY,
    });

    await fetch('/api/push/subscribe', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(subscription.toJSON()),
    });
}
```

Seu endpoint do Suprnova recebe o JSON, valida a forma, e o armazena
no usuário - a string é opaca para seu servidor, mas precisa ser o
JSON exato que o navegador produziu (o tipo `SubscriptionInfo` usa
`Deserialize` para fazer o parse dela depois):

```rust
use suprnova::{Auth, Request, Response, SubscriptionInfo, attrs, json_response};

pub async fn subscribe(req: Request) -> Response {
    let user_id = Auth::id().expect("auth middleware");

    let (_parts, bytes) = match req.body_bytes().await {
        Ok(b) => b,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };
    let raw = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return json_response!({ "error": "body not utf-8" }).map(|r| r.status(400)),
    };

    // Faz parse para validar a forma - endpoint, keys.p256dh, keys.auth.
    // Se o parse falhar, o navegador nos entregou algo malformado.
    let sub: SubscriptionInfo = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };

    // Persiste `raw` ao pé da letra - essa é a string exata que o
    // WebPushChannel vai entregar a serde_json::from_str no dispatch.
    User::query()
        .db_where_op("id", "=", user_id)
        .update_all(attrs! { push_subscription_json: raw })
        .await
        .unwrap();

    json_response!({ "ok": true, "endpoint": sub.endpoint })
}
```

O Service Worker descriptografa o payload do push e renderiza a
notificação:

```js
// /sw.js
self.addEventListener('push', (event) => {
    const data = event.data.json();
    event.waitUntil(
        self.registration.showNotification(data.title, {
            body: data.body,
            data: { url: data.url },
        }),
    );
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    event.waitUntil(clients.openWindow(event.notification.data.url));
});
```

## Limites de payload

A especificação do Web Push limita cada payload criptografado a 4096
bytes no total. O Suprnova rejeita plaintexts maiores que 3992 bytes
(o limite menos o overhead de criptografia AES128GCM de ~85 bytes) no
momento da criptografia, para que a falha apareça no seu código, não
em um 413 do serviço de push. Uma `Notification` cujo `data()`
serializado excede esse limite retorna `WebPushError::Encryption` a
partir do `deliver` do canal.

Para qualquer coisa maior - um corpo de mensagem longo, uma miniatura -
envie uma notificação curta carregando uma URL que o Service Worker
busca ao clicar. Isso é ao mesmo tempo mais rápido (sem criptografia em
um payload de múltiplos KB) e mais flexível (o fetch pode retornar
qualquer forma que você quiser).

## Assinaturas mortas

Quando o serviço de push retorna 404 ou 410, a assinatura está morta -
o usuário desinstalou o navegador, revogou a permissão, ou limpou o
storage. `WebPushChannel` trata isso como um warn não fatal:

```text
WARN webpush subscription gone (404/410); caller should remove
     channel=webpush endpoint=https://fcm.googleapis.com/fcm/send/abc
```

O dispatch retorna `Ok(())` porque a notificação alcançou um estado
terminal - não há destinatário contra o qual repetir. Sua aplicação
deve agir sobre o warn: faça parse de `endpoint` a partir do log (ou
conecte um listener de `NotificationFailed` que classifica via
`WebPushError`) e remova a linha da assinatura. O Suprnova entrega o
warn; ele não faz limpeza automática da tabela de assinaturas para
você.

## Retries e Retry-After

Quando o serviço de push retorna um 5xx transitório, 408, ou 429, o
`WebPushError::PushServiceRejected` subjacente carrega a dica
`Retry-After` já parseada (somente na forma delta-seconds - a forma
HTTP-date retorna `None`):

```rust
use suprnova::WebPushError;

match client.send(&sub, payload, ContentEncoding::Aes128Gcm, 60).await {
    Ok(_) => (),
    Err(e) if e.is_retryable() => {
        let wait = e.retry_after().unwrap_or(Duration::from_secs(30));
        tokio::time::sleep(wait).await;
        // ...tente de novo, ou devolva para a fila com um delay
    }
    Err(WebPushError::SubscriptionGone) => {
        // remove a assinatura
    }
    Err(e) => return Err(e.into()),
}
```

A dica `Retry-After` tem um teto de 24 horas para que um servidor
hostil não consiga estacionar um worker em um sleep de vários anos.

Ao usar `Notify::queue`, o retry/backoff da própria fila se aplica -
um `WebPushError` que se propaga para fora de
`WebPushChannel::deliver` aparece como um erro de job e o envelope
trata o reenfileiramento conforme a política de backoff do job. A dica
`Retry-After` é logada mas (ainda) não é realimentada no cálculo de
delay da fila; se você precisar disso, conecte um listener de
`NotificationFailed` que reenfileira com o delay sugerido.

## Telemetria

O dispatcher de notificações envolve o fan-out em um span de info
`notification.dispatch` marcado com o nome da notificação e a
contagem de canais. Toda entrega bem-sucedida emite um evento
`NotificationSent`; falhas emitem `NotificationFailed` carregando o
nome do canal, a rota, e a string de erro. Conecte qualquer um desses
ao seu pipeline de métricas/log da mesma forma que você conecta outros
eventos do framework - veja [Eventos](events.md).

Uma assinatura morta emite um WARN estruturado com
`channel="webpush"`, o endpoint, e o nome da notificação. Esse é o
sinal para raspar (scrape) em um job automatizado de limpeza de
assinaturas.

### Por que Suprnova diverge

O driver `WebPush` do Laravel é um pacote da comunidade
(`laravel-notification-channels/webpush`) - fora do core, versionado
separadamente, opinativo sobre ORM. O Suprnova incorpora o Web Push no
framework porque o protocolo é bem definido e o POST HTTP criptografado
é um contrato pequeno demais para embrulhar em uma abstração de
terceiros. O subsistema de notificações mantém a superfície uniforme:
a mesma `Notification` que você envia para mail ou database também
chega como push, sem matriz de drivers, sem árvore de config separada.

Também expomos a política de endpoint estrita por padrão. O pacote da
comunidade Laravel deixa a proteção contra SSRF para a aplicação; nós
adotamos a posição de que "o endpoint veio de dados do usuário" é a
forma de toda assinatura de Web Push, e o padrão seguro pertence ao
framework, não ao seu código.

A classificação de retry (`is_retryable`, `retry_after`) é exposta
como métodos tipados em `WebPushError`, em vez de uma tabela de
constantes mágicas na camada de fila. A fila ainda é dona da política
de retry - o erro diz a você se um retry poderia ter sucesso e por
quanto tempo esperar; a fila decide se e quando tirar da fila de novo.
Separar os dois significa que suas estratégias de retry customizadas
(exponential backoff, com jitter, com teto) não precisam tratar Web
Push como caso especial.

## Testes

Levante um servidor `wiremock`, aponte um `WebPushClient` para ele com
`EndpointPolicy::AllowAny`, e faça assert nas solicitações que ele
recebe:

```rust
use std::sync::Arc;
use suprnova::{
    EndpointPolicy, NotificationDispatcher, Notify, VapidKey, VapidSigner,
    WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn order_shipped_pushes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/push"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let signer = VapidSigner::new(VapidKey::generate());
    let client = Arc::new(
        WebPushClient::new(signer, "mailto:test@example.org")
            .unwrap()
            .with_endpoint_policy(EndpointPolicy::AllowAny),
    );
    let channel = Arc::new(WebPushChannel::new(client, 60));

    let dispatcher = NotificationDispatcher::new().register_channel(channel);
    set_dispatcher(Arc::new(dispatcher)).unwrap();

    let user = test_user_with_subscription(&server.uri()).await;
    Notify::send(&user, &OrderShipped {
        order_id: 1,
        tracking_url: "https://ship.example.org/o/1".into(),
    }).await.unwrap();
    // server.received_requests() agora contém o POST criptografado.
}
```

Para testes end-to-end que não se importam com os bytes
criptografados, `Notify::fake()` (abordado em
[Notificações](notifications.md)) captura o dispatch sem executar o
canal - mais rápido, sem mock server, sem round-trip de criptografia.

## Referência

- Primitivas: `suprnova::VapidKey`, `suprnova::VapidSigner`,
  `suprnova::VapidClaims`
- Cliente: `suprnova::WebPushClient`, `suprnova::EndpointPolicy`,
  `suprnova::PushResponse`, `suprnova::SubscriptionInfo`
- Erro: `suprnova::WebPushError` - `.is_retryable()`, `.retry_after()`,
  `WebPushError::SubscriptionGone`
- Codificação: `suprnova::ContentEncoding` (Aes128Gcm; teto de
  plaintext de 3992 bytes)
- Canal: `suprnova::WebPushChannel`
- Facade: `suprnova::Notify`
- Queue job: `suprnova::SendNotificationJob`
- Registro de factory:
  `suprnova::notifications::register_notification_factory`

## Próximos passos

- [Notificações](notifications.md) - o dispatcher multicanal no qual
  `WebPushChannel` se encaixa
- [Correio](mail.md) - a contrapartida do canal de email para usuários
  sem push
- [Transmissão](broadcasting.md) - entrega em tempo real para usuários
  que estão no site
- [Filas](queues.md) - como `Notify::queue` apoia `SendNotificationJob`
- [Eventos](events.md) - escutando `NotificationSent` /
  `NotificationFailed` para conduzir a limpeza de assinaturas mortas

# Notificações

Uma notificação é uma mensagem pequena que você quer que um usuário
(ou "qualquer pessoa com um endereço de email") receba através de um
ou mais canais - mail, inbox no app, push do navegador, WebSocket em
tempo real - a partir de um único call site. Você escreve
`Notify::send(&user, &OrderShipped { … })`; o dispatcher faz fan-out
dessa notificação única através de todo canal que a notificação
declarou, endereçando cada um através do destinatário.

Use notificações quando o *o quê* (um pedido foi enviado, uma fatura
foi pago) é mais interessante para seu código do que o *como* (qual
transporte terminou entregando). Para acesso direto ao transporte -
compor um corpo de mail customizado, publicar em um canal de
transmissão específico, enviar um web push isolado - vá direto para
[mail](mail.md), [transmissão](broadcasting.md), ou
[web push](web-push.md).

## Início rápido

```rust
use serde::{Deserialize, Serialize};
use suprnova::FrameworkError;
use suprnova::NotificationMailable;          // macro derive
use suprnova::notifications::channels::mail::MailRendering;
use suprnova::{Notifiable, Notification, Notify};

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Order shipped - tracking {{ tracking }}",
    html    = "<p>Your order is on its way.</p><p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@example.com",
    from_name = "Acme Orders",
)]
pub struct OrderShipped {
    pub tracking: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str { "OrderShipped" }
    fn channels(&self) -> Vec<&'static str> { vec!["mail", "database"] }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "tracking": self.tracking })
    }
}

struct User { id: i64, email: String }
impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "mail"     => Some(self.email.clone()),
            "database" => Some(self.id.to_string()),
            _          => None,
        }
    }
}

async fn ship(user: &User, tracking: String) -> Result<(), FrameworkError> {
    Notify::send(user, &OrderShipped { tracking }).await
}
```

`Notify::send` despacha tanto para o canal de mail quanto para o
canal de database em uma única chamada. O destinatário recusa um
canal retornando `None` de `route_for` - útil para usuários
"somente-email" ou "somente-push".

## As três traits

| Trait | O que representa | Implementado por |
|---|---|---|
| `Notification` | Uma mensagem tipada + os canais para os quais despacha | Suas structs de notificação |
| `Notifiable` | Um destinatário - expõe um `route_for` por canal | Seu `User`, `Order`, qualquer coisa endereçável |
| `Channel` | Um transporte - sabe como entregar a uma rota | Embutidos: `MailChannel`, `DatabaseChannel`, `BroadcastChannel`, `WebPushChannel` |

### `Notifiable`

```rust
pub trait Notifiable: Send + Sync {
    fn route_for(&self, channel: &str) -> Option<String>;
}
```

O destinatário é dono do endereçamento por canal. `route_for("mail")`
retorna o endereço de email; `route_for("database")` retorna o id da
entidade como string; `route_for("webpush")` retorna um
`SubscriptionInfo` serializado em JSON; `route_for("broadcast")`
retorna o nome do canal de transmissão. Retorne `None` para pular um
canal para este destinatário.

### `Notification`

```rust
pub trait Notification: Serialize + DeserializeOwned + Send + Sync + 'static {
    fn notification_name() -> &'static str where Self: Sized;
    fn channels(&self) -> Vec<&'static str>;
    fn data(&self) -> serde_json::Value;

    fn should_send(&self, _channel: &str) -> bool { true }
    fn after_sending(&self, _channel: &str) -> Result<(), FrameworkError> { Ok(()) }

    fn queue(&self) -> Option<&'static str> { None }
    fn timeout(&self) -> Option<std::time::Duration> { None }
    fn fail_on_timeout(&self) -> bool { false }
    fn max_tries(&self) -> u32 { 3 }
    fn backoff(&self) -> BackoffSchedule { BackoffSchedule::default() }
```

| Método | Propósito |
|---|---|
| `notification_name()` | Identificador estável persistido pelo canal de database, usado como a chave do envelope na fila, e a chave de busca no registry do renderizador de mail. |
| `channels(&self)` | Nomes dos canais para os quais esta notificação despacha. A ordem é a ordem de iteração. |
| `data(&self)` | Payload serializável em JSON que os canais entregam / persistem. Tipicamente `serde_json::to_value(self)` do subconjunto de campos que os canais precisam. |
| `should_send(&self, channel)` | Veto por canal consultado tanto no caminho síncrono quanto no em fila. Retornar `false` pula esse canal para este dispatch. Padrão: sempre envia. |
| `after_sending(&self, channel)` | Hook pós-sucesso invocado uma vez por canal que completou, tanto no caminho síncrono quanto no em fila. Retornar `Err` se propaga da mesma forma que um erro de canal se propagaria. Padrão: no-op. |
| `queue(&self)` | Fila para a qual o despacho `Notify::queue` desta notificação é resolvido. Padrão: `None` (padrão do driver ou um `Queue::route` se houver um registrado). Veja [Ajuste de fila](#ajuste-de-fila). |
| `timeout(&self)` | Timeout por tentativa para os jobs enfileirados desta notificação. Padrão: `None` (sem timeout). |
| `fail_on_timeout(&self)` | Se `true`, um timeout é uma falha permanente (dead-letter, sem retry). Padrão: `false`. |
| `max_tries(&self)` | Máximo de tentativas para os jobs enfileirados desta notificação. Padrão: `3`. |
| `backoff(&self)` | Agenda de backoff para os jobs enfileirados desta notificação. Padrão: o padrão do framework. |

`should_send` e `after_sending` são honrados nos **dois** caminhos.
`Notify::send` os consulta no dispatcher; `Notify::queue` verifica
`should_send` antes de enfileirar cada job por canal, e o worker
reverifica `should_send` antes da entrega (o estado pode mudar entre o
enfileiramento e a execução) e executa `after_sending` depois de um
envio bem-sucedido. Os três *eventos* de ciclo de vida
(`NotificationSending` / `NotificationSent` / `NotificationFailed`)
ainda disparam somente no caminho síncrono.

## Canais

### Correio

O canal de mail entrega através do transporte de mail vinculado (veja
[Correio](mail.md)). Uma notificação opta por participar implementando
`NotificationMailable`:

```rust
pub trait NotificationMailable: Notification {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError>;
}
```

`MailRendering` é o envelope de renderização - `subject` (obrigatório),
`html` e/ou `text` (pelo menos um obrigatório), `from`, `cc`, `bcc`,
`reply_to` opcionais, e `attachments`. O canal de mail monta uma
mensagem de saída a partir dessa renderização mais o `route_for("mail")`
do destinatário, aplica os padrões de remetente configurados
(`Mail::always_from(...)`, `always_to(...)`, etc.), e despacha através
de `Mail::current_transport`.

Se o renderizador retorna uma renderização sem `html` nem `text`, a
entrega falha rápido - mail de notificação vazio nunca é enviado
silenciosamente.

#### `#[derive(NotificationMailable)]`

O derive colapsa o `impl` de `to_mail` por Notification em um único
atributo `#[mail(...)]`. Templates usam
[Tera](https://keats.github.io/tera/); os campos serializados de
`self` são o contexto.

```rust
#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Welcome {{ name }}",
    html_template = "templates/welcome.html",
    text_template = "templates/welcome.txt",
    from = "hello@example.com",
    from_name = "Acme",
    cc = "ops@example.com, support@example.com",
)]
pub struct Welcome { pub name: String }
```

Chaves suportadas:

| Chave | Obrigatória? | Propósito |
|---|---|---|
| `subject` | sim | Template Tera - renderizado com `self` como contexto. |
| `html` | adaga | Template Tera de corpo HTML inline. |
| `html_template` | adaga | Caminho para um template Tera de corpo HTML (embutido via `include_str!`). |
| `text` | adaga | Template Tera de corpo em texto puro inline. |
| `text_template` | adaga | Caminho para um template Tera de corpo em texto puro (embutido via `include_str!`). |
| `from` | não | Email do remetente - sobrescreve o padrão `noreply@localhost`. |
| `from_name` | não | Nome de exibição. Exige `from`. |
| `cc` | não | Lista de CC separada por vírgulas. Espaços em branco e vírgulas ao final são ignorados. |
| `bcc` | não | Lista de BCC separada por vírgulas. |
| `reply_to` | não | Lista de Reply-To separada por vírgulas. |

(adaga) Pelo menos uma variante de corpo precisa estar presente.
`html` e `html_template` são mutuamente exclusivos; o mesmo para
`text` e `text_template`.

Todo invariante é imposto em tempo de compilação - `subject` ausente,
corpo vazio, variantes conflitantes, `from_name` sem `from`, ou chaves
desconhecidas falham o build em vez de falhar no dispatch.

Para anexos (payloads binários) ou destinatários dinâmicos por
instância, implemente `NotificationMailable` manualmente e construa o
`MailRendering` diretamente.

### Banco de dados

O canal de database persiste cada notificação como uma linha na
tabela `notifications`:

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, NotificationDispatcher};

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")));
```

O segundo argumento é a tag de tipo polimórfico do destinatário (o
que você armazena em `notifiable_type` para poder consultar linhas do
inbox depois). O `route_for("database")` do destinatário se torna o
`notifiable_id`. A migration vem com o framework
(`framework/migrations/20260516_create_notifications_table.sql`); rode
`suprnova migrate` e a tabela aparece.

#### Lendo a caixa de entrada

Os helpers do lado de leitura vivem em `suprnova::notifications` como
funções livres sobre `(notifiable_type, notifiable_id)`:

```rust
use suprnova::notifications::{
    all_for, unread_for, read_for,
    mark_as_read, mark_as_unread, mark_all_as_read,
    delete_for, StoredNotification,
};

let unread: Vec<StoredNotification> = unread_for(&db, "users", "42").await?;
let count = mark_all_as_read(&db, "users", "42").await?;
let removed = delete_for(&db, "users", "42").await?;
```

`StoredNotification` carrega `id`, `type_name` (o
`Notification::notification_name`), `notifiable_type`,
`notifiable_id`, o `data` JSON decodificado, `read_at`, `created_at`,
`updated_at`. `mark_as_read` / `mark_as_unread` são idempotentes
(seguindo o mesmo contrato do Laravel).

### Web push

O canal de web push criptografa o payload e faz POST dele para um
endpoint de assinatura de push do navegador armazenado, via o cliente
de assinatura VAPID do framework:

```rust
use std::sync::Arc;
use suprnova::WebPushChannel;
use suprnova::web_push::{VapidKey, WebPushClient};

let client = WebPushClient::new(
    VapidKey::from_pem(b"-----BEGIN PRIVATE KEY-----\n…")?,
    "mailto:ops@example.com",
)?;
let push_channel = WebPushChannel::new(Arc::new(client), 86_400 /* TTL seconds */);
```

O `route_for("webpush")` do destinatário retorna um `SubscriptionInfo`
serializado em JSON (a mesma forma que o navegador devolve de
`PushSubscription.toJSON()` - armazene-a ao pé da letra, retorne-a
intocada). O TTL é repassado ao serviço de push.

Quando o serviço de push informa ao canal que uma assinatura morreu
(HTTP 404/410), o canal loga um WARN estruturado e retorna sucesso - a
notificação alcançou um estado terminal sem destinatário contra o qual
repetir. Operadores veem o log e removem a assinatura morta; a entrega
não retorna erro.

Veja [Web Push](web-push.md) para o cliente completo.

### Transmissão

O canal de transmissão publica cada notificação no `BroadcastHub` da
aplicação para que assinantes de WebSocket a recebam em tempo real. O
`route_for("broadcast")` do destinatário é o nome do canal, o tipo da
notificação é o evento, e `data()` é o payload:

```rust
use std::sync::Arc;
use suprnova::BroadcastChannel;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;

// No boot - vincule o hub antes de qualquer dispatch de transmissão.
App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

let dispatcher = suprnova::NotificationDispatcher::new()
    .register_channel(Arc::new(BroadcastChannel::new()));
```

O canal resolve o hub a partir do container no momento da entrega. Se
nenhum `BroadcastHub` está vinculado quando uma notificação declara
`"broadcast"`, o canal retorna um erro - uma aplicação malconfigurada
expõe o problema em vez de descartar a mensagem silenciosamente.
Publicar em um canal com zero assinantes ativos não é um erro.

Veja [Transmissão](broadcasting.md) para a configuração do hub e a
integração com WebSocket.

## Notificações sob demanda

Às vezes você quer notificar *alguém que não está no seu database* -
um alerta de ops isolado para um endereço de email, um receptor de
webhook, um canal de transmissão que nenhum usuário possui.
`AnonymousNotifiable` é o "usuário sem linha":

```rust
use suprnova::Notify;

let recipient = Notify::route("mail", "ops@example.com")?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;

// Múltiplos canais em um único builder:
let recipient = Notify::routes([
    ("mail", "ops@example.com"),
    ("broadcast", "ops-channel"),
])?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;
```

`Notify::route("database", …)` e `Notify::routes([..., ("database",
…)])` retornam `Err` - o canal de database persiste um par
`(notifiable_type, notifiable_id)` que um destinatário anônimo não
consegue fornecer.

## O dispatcher

`NotificationDispatcher` mantém o registry de canais. Construa-o uma
vez no boot e vincule-o globalmente:

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, MailChannel, NotificationDispatcher, WebPushChannel};
use suprnova::notifications::set_dispatcher;

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(MailChannel::new()))
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")))
    .register_channel(Arc::new(WebPushChannel::new(push_client, 86_400)));

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` é last-write-wins no nome do canal - registrar
dois canais chamados `"mail"` silenciosamente substitui o primeiro.
Isso torna setups de teste ergonômicos.

Uma notificação que declara um canal que o dispatcher não registra
loga um WARN (`no channel registered; skipping`) e continua para o
próximo canal - o dispatch não retorna erro em um nome de canal
desconhecido.

`set_dispatcher` retorna `Result<(), FrameworkError>` porque o
registry do dispatcher vive atrás de um `RwLock`; o caminho de erro só
é acionado se o lock estiver envenenado (um writer anterior sofreu
panic). Na prática o call site no boot usa `?`.

### Eventos de ciclo de vida

Três eventos rodeiam toda entrega síncrona de canal:

| Evento | Quando | Comportamento em erro de listener |
|---|---|---|
| `NotificationSending` | Imediatamente antes do canal executar | `Err` do listener **veta** o canal para este dispatch |
| `NotificationSent` | Depois de uma entrega bem-sucedida | Dispatch best-effort - erros de listener não se propagam |
| `NotificationFailed` | Quando um canal retornou um erro | Dispatch best-effort; o erro de canal subjacente ainda se propaga conforme o contrato de parada na primeira falha |

Os três carregam `(notification, channel, route, data)`. `Failed`
adiciona o `error` como string. Escute com
`EventFacade::listen::<E, L>` - veja [Eventos](events.md).

Esses eventos disparam somente no caminho síncrono `Notify::send`. O
worker em fila entrega canais diretamente sem despachar os eventos.

### Telemetria

`NotificationDispatcher::notify` envolve o fan-out em um span de
tracing `notification.dispatch`:

- `notification` - `Notification::notification_name()`
- `channel_count` - contagem de canais declarados
- `duration_ms` - latência do fan-out na conclusão
- log terminal: `notification dispatched` (info) ou
  `notification dispatch failed` (warn)

O canal de mail aninha seu próprio span `mail.send` dentro.

### Contrato de parada na primeira falha

`Notify::send` retorna no primeiro erro de canal. Canais que já
tiveram sucesso não são revertidos; canais que ainda não executaram
não são tentados. O mesmo contrato se aplica ao worker em fila.

Para entrega pelo menos uma vez através de múltiplos canais, despache
cada canal através de sua própria chamada `Notify::queue` - as chaves
de idempotência do envelope da fila protegem contra envios duplicados
em um retry.

## Entrega em fila

`Notify::send` executa em processo. `Notify::queue` faz push de um
`SendNotificationJob` na [Fila](queues.md), pré-resolvendo as rotas
por canal a partir do destinatário para que o worker não precise de
um handle `Notifiable` no momento da execução:

```rust
use suprnova::notifications::register_notification_factory;
use suprnova::Notify;

// No boot - uma vez por notificação concreta alcançável via Notify::queue.
register_notification_factory::<OrderShipped>()?;

// Em qualquer lugar:
Notify::queue(&user, OrderShipped { tracking }).await?;
```

No momento do dispatch o worker:

1. Procura a factory da notificação por `notification_name`
2. Reconstrói a notificação tipada a partir do payload JSON
3. Itera os canais registrados no momento do enfileiramento
4. Para cada um, reverifica `should_send(channel)` (pulando canais
   vetados), procura o canal no dispatcher vinculado, chama
   `deliver(route, &notification)`, então executa
   `after_sending(channel)`

Canais que foram declarados no momento do enfileiramento mas não
estão registrados quando o worker executa logam um WARN e são
pulados - mesmo contrato do caminho síncrono. Canais sem rota
pré-resolvida são pulados silenciosamente (o destinatário retornou
`None` no momento do enfileiramento).

`Notify::queue` também avalia `should_send` no momento de enfileirar,
então um canal vetado nunca chega a ser enfileirado; a reverificação
do worker cobre o estado que muda entre o enfileiramento e a
execução. O caminho em fila **não** dispara os três eventos de ciclo
de vida (`NotificationSending` / `NotificationSent` /
`NotificationFailed`) - esses permanecem somente-síncronos. Se você
depende dos eventos, envie através de `Notify::send`.

### Ajuste de fila

Mais cinco métodos de `Notification` carregam a política de fila por
notificação para o despacho de `Notify::queue`, espelhando os próprios
métodos de ajuste de `Job`:

| Método | Padrão | Espelha |
|---|---|---|
| `queue(&self)` | `None` - padrão do driver, ou um `Queue::route` se houver um registrado | `Job::queue()` |
| `timeout(&self)` | `None` - sem timeout por tentativa | `Job::timeout()` |
| `fail_on_timeout(&self)` | `false` - um timeout sofre retry como qualquer outra falha | `Job::fail_on_timeout()` |
| `max_tries(&self)` | `3` | `Job::max_tries()` |
| `backoff(&self)` | exponencial, base de 2s, teto de 5min, jitter de ±25% | `Job::backoff()` |

`Notify::queue` lê esses valores da instância da notificação uma vez e os
carrega em cada push de `SendNotificationJob` por canal. Uma notificação
que não substitua nenhum dos cinco recebe exatamente o envelope que uma
chamada simples a `Notify::queue` sempre produziu.

```rust
struct WelcomeDigest;

impl Notification for WelcomeDigest {
    fn notification_name() -> &'static str { "WelcomeDigest" }
    fn channels(&self) -> Vec<&'static str> { vec!["mail"] }
    fn data(&self) -> serde_json::Value { serde_json::Value::Null }

    fn queue(&self) -> Option<&'static str> { Some("digests") }
    fn timeout(&self) -> Option<std::time::Duration> { Some(std::time::Duration::from_secs(10)) }
    fn fail_on_timeout(&self) -> bool { true }
}
```

Defina `fail_on_timeout(&self)` como `true` quando um timeout significar
que a entrega é irrecuperável, não transitória: o worker envia para
dead-letter no primeiro timeout em vez de tentar novamente até
`max_tries`.

Esses cinco métodos se aplicam somente a `Notify::queue` -
`Notify::send` executa no processo e não tem envelope de fila para ajustar.

### Por que Suprnova diverge

O Laravel condiciona notificações em fila à interface marcadora
`ShouldQueue` - a mesma chamada
`Notification::send($user, $notification)` enfileira se a notificação
implementa `ShouldQueue` e envia inline se não implementa. O
comportamento depende de uma flag em nível de tipo no site da
notificação, que é invisível a partir do call site.

O Suprnova torna essa escolha explícita em toda chamada:
`Notify::send` é sempre síncrono; `Notify::queue` é sempre em fila.
Não há troca de modo escondida. (É por isso também que não há
`send_now` - `send` já é o síncrono.)

O lado do destinatário também diverge. A trait `Notifiable` do
Laravel é um mixin que traz o relacionamento de inbox, métodos
`routeNotificationFor*`, e a chave primária polimórfica. O
`Notifiable` do Suprnova é deliberadamente mínimo - apenas
`route_for(channel) -> Option<String>` - porque traits em Rust não se
compõem por mixin. O equivalente do lado de leitura do Laravel é
entregue como funções livres sobre `(notifiable_type,
notifiable_id)` (`unread_for`, `mark_as_read`, …) para que structs
simples possam ser notifiable sem herdar um relacionamento de ORM.

## Testes

Duas superfícies de fake, respondendo perguntas diferentes.

### `Notify::fake()` - "uma notificação foi despachada?"

```rust
use suprnova::Notify;
use suprnova::notifications::{
    assert_count, assert_nothing_sent, assert_sent_named,
    assert_sent_times, assert_sent_to, assert_sent_to_on,
    recorded_notifications,
};

#[tokio::test]
async fn ship_dispatches_order_shipped() {
    let _fake = Notify::fake();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    assert_sent_named("OrderShipped");
    assert_sent_to("alice@example.org", "OrderShipped");
    assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    assert_sent_times("OrderShipped", 1);
    assert_count(2); // mail + database
}
```

Enquanto a guard fake está viva, tanto `Notify::send` quanto
`Notify::queue` registram o dispatch em vez de rodar canais ou
enfileirar um job - nenhum canal executa, nenhuma linha de fila é
escrita. O fake mantém um mutex de serialização de todo o processo,
então testes paralelos não conseguem interlear capturas; deixe a
guard `_fake` dropar ao final do teste para limpar o gravador.

Use `recorded_notifications()` para custódia completa dos dados
capturados:

```rust
let records = recorded_notifications();
assert_eq!(records[0].notification, "OrderShipped");
assert_eq!(records[0].channel, "mail");
assert_eq!(records[0].data["tracking"], "1Z…");
```

### `Mail::fake()` + `MailChannel` real - "a notificação *renderizou* corretamente?"

`Notify::fake()` faz short-circuit antes do canal. Para verificar que
o corpo do mail de fato renderizou como você espera, execute o canal
real sob `Mail::fake()`:

```rust
use serial_test::serial;
use std::sync::Arc;
use suprnova::mail::Mail;
use suprnova::notifications::{set_dispatcher, NotificationDispatcher};
use suprnova::{MailChannel, Notify, register_mail_renderer};

#[tokio::test]
#[serial]
async fn ordershipped_renders_tracking_in_subject() {
    let fake = Mail::fake();
    register_mail_renderer::<OrderShipped>().unwrap();
    set_dispatcher(Arc::new(
        NotificationDispatcher::new()
            .register_channel(Arc::new(MailChannel::new())),
    )).unwrap();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.subject.contains("1Z…"));
}
```

Testes que tocam os globais de dispatcher, renderizador, ou
transporte precisam ser `#[serial_test::serial]` - esses são statics
globais de processo.

## Boas práticas

### Registre toda factory e renderizador no boot

`Notify::queue` reconstrói a notificação através do registry de
factory no worker, e `MailChannel` renderiza através de
`register_mail_renderer`. Registre toda notificação queueable /
mailable antecipadamente:

```rust
// bootstrap.rs
use suprnova::notifications::register_notification_factory;
use suprnova::register_mail_renderer;

pub fn register() -> Result<(), FrameworkError> {
    // Factories de notificação (uma por Notification alcançável via Notify::queue).
    register_notification_factory::<OrderShipped>()?;
    register_notification_factory::<InvoicePaid>()?;

    // Renderizadores de mail (um por NotificationMailable).
    register_mail_renderer::<OrderShipped>()?;
    register_mail_renderer::<InvoicePaid>()?;
    Ok(())
}
```

Uma notificação não registrada na fila aparece como `unknown
notification: {name}` no momento da execução do worker e tenta de
novo através do caminho de dead-letter. Um dispatch de `MailChannel`
para um renderizador não registrado expõe um erro
`register via suprnova::register_mail_renderer::<N>()` da mesma
forma.

### Use fila para fan-outs multicanal

O dispatcher síncrono visita canais em ordem e retorna no primeiro
erro. Uma falha no canal #2 deixa o canal #1 committed e os canais
#3+ não tentados. Para qualquer notificação que atravesse mais de um
canal, prefira `Notify::queue` para que o worker trate retries com
backoff e o dispatch sobreviva a um crash do processo.

### Torne as entregas de canal idempotentes

Retries do worker significam que o mesmo `SendNotificationJob` pode
executar mais de uma vez. Os canais embutidos são amigáveis a
idempotência: `MailChannel` repassa para provedores que tipicamente
deduplicam por message-id; `DatabaseChannel` insere um UUID novo por
execução (que é o comportamento certo para uma linha de auditoria);
`WebPushChannel` faz POST para um provedor que engole duplicatas.
Canais customizados deveriam ter como alvo operações idempotentes -
POSTs HTTP com chaves de dedupe estáveis do lado do cliente, upserts
em vez de inserts cegos, nenhum efeito colateral de "incrementar um
contador" no caminho de entrega.

### Vincule o dispatcher em um único lugar

`register_channel` é last-write-wins, então testes podem trocar um
canal real por um stub no setup. Mantenha a vinculação de produção em
`bootstrap.rs` e deixe os testes construírem seu próprio dispatcher
com os stubs que precisarem. Não faça `register_channel`
preguiçosamente dentro de handlers de solicitação - as semânticas de
escrita no lock global mais last-write-wins ficam surpreendentes sob
carga concorrente.

## Referência

| Símbolo | Caminho |
|---|---|
| `Notifiable`, `Notification`, `Channel`, `DynNotification` | `suprnova::` |
| `Notify` (facade), `NotifyFakeGuard` | `suprnova::` |
| `NotificationDispatcher`, `NotificationFactory` | `suprnova::` |
| `AnonymousNotifiable` | `suprnova::` |
| `MailChannel`, `MailRendering`, `NotificationMailable` | `suprnova::` |
| `register_mail_renderer::<N>()` | `suprnova::` |
| `DatabaseChannel`, `StoredNotification` | `suprnova::` |
| `WebPushChannel` | `suprnova::` |
| `BroadcastChannel` | `suprnova::` |
| `SendNotificationJob` | `suprnova::` |
| `NotificationSending`, `NotificationSent`, `NotificationFailed` | `suprnova::` |
| `set_dispatcher`, `register_notification_factory` | `suprnova::notifications::` |
| `all_for`, `unread_for`, `read_for`, `mark_as_read`, `mark_as_unread`, `mark_all_as_read`, `delete_for` | `suprnova::notifications::` |
| `assert_sent`, `assert_sent_named`, `assert_sent_times`, `assert_sent_to`, `assert_sent_to_on`, `assert_nothing_sent`, `assert_nothing_sent_to`, `assert_count`, `recorded_notifications` | `suprnova::notifications::` |
| `#[derive(NotificationMailable)]` | `suprnova::` |

## Próximos passos

- [Correio](mail.md) - o transporte e a superfície `Mailable` sobre a qual o canal de mail roda
- [Transmissão](broadcasting.md) - o `BroadcastHub` através do qual o canal de transmissão publica
- [Web Push](web-push.md) - VAPID, criptografia, armazenamento de assinatura
- [Eventos](events.md) - escutando `NotificationSending` / `Sent` / `Failed`
- [Filas](queues.md) - o worker que conduz `Notify::queue`
- [Testes](testing.md) - superfícies de fake e padrões de serial-test

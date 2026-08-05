# Mocking e Fakes

Toda superfície externa no Suprnova vem com um fake em processo que
captura o que seu código teria enviado - mail, notificações, jobs
enfileirados, comandos despachados, eventos disparados, arquivos
escritos, chamadas HTTP de saída - e um conjunto correspondente de
asserções que você executa depois do fato. A forma é sempre: instale o
fake, execute o código sob teste, faça a asserção sobre o que foi
capturado. Este capítulo é a visão geral consolidada; cada capítulo de
subsistema ([Correio](mail.md), [Notificações](notifications.md),
[Filas](queues.md), [Barramento](bus.md), [Eventos](events.md),
[Sistema de arquivos e armazenamento](filesystem.md),
[Cliente HTTP](http-client.md)) cobre seu fake em profundidade.

## Os sete fakes

| Superfície      | Ponto de entrada                                   | Estilo de asserção                     | Segurança em paralelo                              | Capítulo                              |
|------------------|-----------------------------------------------------|-----------------------------------------|------------------------------------------------------|---------------------------------------|
| Mail             | `Mail::fake()` → guarda `MailFake`                 | métodos na guarda                       | precisa de `#[serial]` - transporte global, sem serializer | [mail.md](mail.md)                   |
| Notificações     | `Notify::fake()` → `NotifyFakeGuard`               | funções livres em `notifications::testing` | a guarda retém um serializer global ao processo   | [notifications.md](notifications.md) |
| Fila             | `suprnova::queue::testing::install_fake()`         | funções livres em `queue::testing`     | a guarda retém um serializer global ao processo    | [queues.md](queues.md)               |
| Barramento       | `suprnova::bus::testing::install_fake()`           | funções livres em `bus::testing`       | a guarda retém um serializer global ao processo    | [bus.md](bus.md)                     |
| Eventos          | `EventFacade::fake()` → `EventFakeGuard`           | funções livres em `events`             | a guarda retém um serializer global ao processo    | [events.md](events.md)               |
| Armazenamento    | `Storage::fake()` → `StorageFakeGuard`             | métodos de `DiskAssertExt` num disco   | a guarda retém um serializer global ao processo    | [filesystem.md](filesystem.md)       |
| Cliente HTTP     | `Http::fake(\|\| async { … }).await`               | `assert_sent` / `assert_not_sent`      | task-local - genuinamente concorrente entre testes | [http-client.md](http-client.md)     |

Alguns invariantes valem para os sete:

- **O fake registra, o backend real não executa.** Mail não é
  enviado, jobs não são enviados (pushed) ao driver, handlers não
  executam, eventos pulam seus listeners, HTTP não alcança a rede,
  escritas de arquivo vão para um disco em memória. O lado capturado
  carrega informação suficiente para fazer a asserção sobre o que
  teria acontecido.
- **A guarda é RAII.** Dropar a guarda restaura o que quer que
  estivesse em vigor antes (o transporte de mail anterior, um
  registry de storage limpo, nenhuma gravação para eventos, etc.).
  Testes não precisam de uma etapa de teardown.
- **O fake não mente sobre erros.** Se seu código chama
  `Bus::dispatch` para um comando não registrado, o fake ainda
  retorna `Err(_)` - somente dispatches bem-sucedidos são capturados.

## As formas, e por que elas diferem

Três padrões se repetem. Saber qual padrão um fake usa te diz se você
deve importar uma função livre, chamar um método na guarda, ou
envolver o corpo do teste numa closure.

### Guarda-com-métodos (Mail)

`Mail::fake()` retorna um `MailFake` cujos próprios métodos são as
asserções. Isso é conveniente quando quem faz a asserção é *o
próprio* fake - você já o tem vinculado a uma local - mas é o único
fake nesse formato:

```rust,ignore
let fake = Mail::fake();
Mail::to("alice@example.org")
    .send(WelcomeEmail { name: "Alice".into() })
    .await?;
fake.assert_sent_count(1);
fake.assert_sent(|m| m.has_to("alice@example.org"));
```

### Guarda mais funções livres (Notify, Queue, Bus, Events)

A guarda é um token que não faz nada, cujo único trabalho é manter o
fake instalado; as asserções vivem num submódulo `testing` ao lado dos
internals do fake. Importe o que você precisa:

```rust,ignore
use suprnova::queue::testing::{install_fake, assert_pushed, pushed};

let _guard = install_fake();
schedule_welcome_email(user_id).await?;
assert_pushed::<WelcomeJob>(|j| j.user_id == user_id);
```

Este é o formato mais comum porque generaliza de forma limpa entre
tipos - toda asserção é genérica sobre `J: Job` / `C: Command` /
`E: Event` em vez de estar embutida (baked in) num tipo de guarda. O
trade-off é um import extra.

### Escopo-com-closure (HTTP)

`Http::fake` é o caso fora da curva. HTTP de saída executa em qualquer
task do Tokio que estiver viva no momento, então o estado do fake vive
num `tokio::task_local!`. Você não pode instalá-lo uma vez e deixar
rolando - você precisa envolver o corpo que chama o cliente:

```rust,ignore
use suprnova::{Http, fake_response, assert_sent};

Http::fake(|| async {
    fake_response("POST", "/api/users", 201, serde_json::json!({"id": 1}));

    let resp = Http::post("https://example.com/api/users")
        .json(&serde_json::json!({"name": "Ada"}))
        .send()
        .await?;

    assert_eq!(resp.status(), 201);
    assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
})
.await;
```

O ganho: todo outro fake retém um serializer global ao processo,
então testes paralelos executam um por vez, mas `Http::fake` é
genuinamente concorrente - todo teste ganha seu próprio recorder
task-local e eles nunca colidem.

### A extension trait do Storage

`Storage::fake()` retorna uma guarda *e* um disco em memória padrão,
mas suas asserções dependem do próprio disco, através da extension
trait `DiskAssertExt`:

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

let _guard = Storage::fake();
let disk = Storage::disk("default")?;

disk.put("invoices/42.pdf", b"...").await?;
disk.assert_exists("invoices/42.pdf").await;
disk.assert_count("invoices/", 1, false).await;
```

A extension trait é condicionada a
`#[cfg(any(test, feature = "testing"))]`, então código de produção não
consegue chamar `disk.assert_exists(…)` por acidente.

## Segurança em paralelo, em um parágrafo

Seis dos sete fakes protegem uma estática global ao processo. A
guarda de cada um, na construção, toma um `std::sync::Mutex`
`FAKE_SERIAL` dedicado e o retém até dropar. O efeito é que quaisquer
dois `#[tokio::test]`s que instalem o mesmo fake executam serializados
dentro de um processo - sem precisar de `#[serial]` do crate
[serial_test](https://crates.io/crates/serial_test). **Mail é a
exceção**: a guarda `MailFake` troca o `TRANSPORT` global sem tomar
um serializer, então testes `Mail::fake()` concorrentes *iriam*
colidir (clobber) um com o outro. Marque-os `#[serial]`.
**`Http::fake` também é uma exceção**: é task-local, não global ao
processo, então os testes executam genuinamente em paralelo e nunca
precisam de `#[serial]`.

Se você intercalar dispatch real com dispatch fake para a mesma
superfície dentro de um binário de teste, o caminho real não toma o
serializer, então ele pode competir (race) com um teste faked
paralelo. Marque os testes de dispatch real com `#[serial]` nesse
caso - os docs de cada capítulo apontam isso onde se aplica (veja
[Barramento](bus.md) para o exemplo canônico).

## Mail - `Mail::fake()`

```rust,ignore
use serial_test::serial;
use suprnova::mail::{Mail, Address};

#[tokio::test]
#[serial]
async fn welcome_email_is_sent() {
    let fake = Mail::fake();

    register_user("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent_to("eve@example.org");
}
```

| Asserção                                  | Faz a asserção de que…                                     |
|--------------------------------------------|-------------------------------------------------------------|
| `fake.assert_sent(\|m\| pred)`             | pelo menos uma mensagem capturada corresponde                |
| `fake.assert_sent_to("…")`                 | pelo menos uma mensagem capturada foi roteada para este email |
| `fake.assert_not_sent(\|m\| pred)`         | nenhuma mensagem capturada corresponde                        |
| `fake.assert_not_sent_to("…")`             | nenhuma mensagem capturada foi para este email                |
| `fake.assert_sent_count(n)`                | exatamente `n` mensagens capturadas                          |
| `fake.assert_nothing_sent()`               | nada foi capturado                                            |
| `fake.assert_queued("MailableName")`       | pelo menos um mailable enfileirado com este nome              |
| `fake.assert_queued_with(name, \|q\| …)`   | um mailable enfileirado corresponde ao predicado              |
| `fake.assert_queued_to("…")`               | um mailable enfileirado foi roteado para este email           |
| `fake.assert_not_queued("MailableName")`   | nenhum mailable enfileirado com este nome                     |
| `fake.assert_queued_count(n)`              | exatamente `n` mailables enfileirados                        |
| `fake.assert_nothing_queued()`             | nada foi enfileirado                                          |
| `fake.assert_outgoing_count(n)`            | enviados + enfileirados totalizam `n`                        |
| `fake.assert_nothing_outgoing()`           | nada foi enviado e nada foi enfileirado                       |

`fake.captured()`, `fake.queued()`, `fake.sent(pred)`,
`fake.sent_to(…)`, `fake.queued_named(…)`, e `fake.queued_to(…)`
retornam os dados correspondentes, para que você possa construir
asserções customizadas. Veja [Correio](mail.md) para a superfície
completa, incluindo como `Mail::queue` é espelhado no fake mesmo
quando `Queue::fake` não está instalado.

## Notificações - `Notify::fake()`

```rust,ignore
use suprnova::notifications::{Notify, testing};

#[tokio::test]
async fn order_shipped_notifies_customer() {
    let _guard = Notify::fake();

    ship_order(order_id).await.unwrap();

    testing::assert_sent_to("alice@example.org", "OrderShipped");
    testing::assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    testing::assert_sent_times("OrderShipped", 1);
}
```

| Asserção                                            | Faz a asserção de que…                             |
|------------------------------------------------------|-----------------------------------------------------|
| `assert_sent(\|r\| pred)`                            | pelo menos uma notificação despachada corresponde   |
| `assert_sent_to(route, "Name")`                      | a notificação nomeada foi para esta rota por canal  |
| `assert_sent_to_on(route, channel, "Name")`          | despachada neste canal para esta rota               |
| `assert_sent_named("Name")`                          | a notificação nomeada foi despachada em qualquer canal |
| `assert_sent_times("Name", n)`                       | exatamente `n` da notificação nomeada               |
| `assert_nothing_sent()`                              | nenhuma notificação despachada                      |
| `assert_count(n)`                                    | exatamente `n` no total, entre todos os tipos e canais |
| `assert_nothing_sent_to(route)`                      | nada despachado para esta rota                      |

`testing::recorded()` retorna todo `FakeRecord` (nome da notificação,
canal, rota, dados JSON) para asserções mais granulares. Destinatários
de notificação são indexados pelo valor `route_for` por canal, então
`assert_sent_to` recebe a string de rota (um endereço de email para
`"mail"`, o id-como-string para `"database"`, …) - veja
[Notificações](notifications.md) para o modelo de roteamento.

## Fila - `queue::testing::install_fake()`

```rust,ignore
use suprnova::Queue;
use suprnova::queue::testing::{
    install_fake, assert_pushed, assert_pushed_later, pushed,
};

#[tokio::test]
async fn order_placed_enqueues_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_pushed::<ChargeCustomerJob>(|j| j.order_id == 42);
}
```

| Asserção                                      | Faz a asserção de que…                                        |
|------------------------------------------------|------------------------------------------------------------------|
| `assert_pushed::<J>(\|j\| pred)`               | pelo menos um push de `J` corresponde                            |
| `assert_pushed_later::<J>(\|j, at\| pred)`     | um push de `J` foi agendado para `at` (dispatch atrasado)        |

O lado de dados retorna os próprios jobs tipados:

- `pushed::<J>() -> Vec<J>` - todo push capturado de `J`
- `pushed_with_available_at::<J>() -> Vec<(J, DateTime<Utc>)>` - o
  mesmo, com o timestamp agendado de cada job

Todo `Queue::push`, `Queue::push_later`, `Queue::later`,
`Queue::push_unique*`, e os dispatchers de chain/batch, todos
convergem para o mesmo recorder. Veja [Filas](queues.md) para a
semântica de `push_unique` sob o fake (ele sempre registra e relata
"pushed").

## Barramento - `bus::testing::install_fake()`

```rust,ignore
use suprnova::Bus;
use suprnova::bus::testing::{
    install_fake, assert_dispatched, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched,
};

#[tokio::test]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

| Asserção                                            | Faz a asserção de que…                                        |
|-------------------------------------------------------|------------------------------------------------------------------|
| `assert_dispatched::<C>(\|c\| pred)`                | pelo menos um comando despachado de `C` corresponde              |
| `assert_not_dispatched::<C>(\|c\| pred)`            | nenhum comando despachado de `C` corresponde                     |
| `assert_dispatched_times::<C>(\|c\| pred, n)`       | exatamente `n` comandos despachados de `C` correspondem          |
| `assert_nothing_dispatched()`                       | zero comandos de qualquer tipo despachados sob o fake ativo      |

Sob o fake, `Bus::dispatch` retorna `Ok(Dispatched::Captured)` em vez
de executar o handler. Falhas reais - erros de encode/decode, nenhum
handler registrado antes de o fake ser instalado - ainda aparecem
como `Err(_)`. Veja [Barramento](bus.md).

## Eventos - `EventFacade::fake()`

```rust,ignore
use suprnova::EventFacade;
use suprnova::events::{
    assert_dispatched, assert_dispatched_once, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched, dispatched,
    dispatched_count, dispatched_events, has_dispatched,
};

#[tokio::test]
async fn registration_dispatches_welcome_event() {
    let _guard = EventFacade::fake();

    register_user("ada@example.com").await.unwrap();

    assert_dispatched_once::<UserRegistered>();
    assert_dispatched::<UserRegistered>(|e| e.email == "ada@example.com");
}
```

| Asserção                                | Faz a asserção de que…                             |
|------------------------------------------|-----------------------------------------------------|
| `assert_dispatched::<E>(\|e\| pred)`    | pelo menos um `E` despachado corresponde            |
| `assert_dispatched_once::<E>()`         | exatamente um `E` foi despachado                    |
| `assert_dispatched_times::<E>(n)`       | exatamente `n` de `E` foram despachados             |
| `assert_not_dispatched::<E>(\|e\| ..)`  | nenhum `E` correspondente foi despachado            |
| `assert_nothing_dispatched()`           | nenhum evento de qualquer tipo despachado           |
| `assert_listening::<E, L>()`            | o listener `L` está registrado para `E`             |
| `has_dispatched::<E>()`                 | `bool`: algum `E` registrado                        |
| `dispatched::<E>(\|e\| pred)`           | clones `Vec<E>` dos eventos correspondentes         |
| `dispatched_count::<E>(\|e\| pred)`     | contagem de eventos correspondentes                 |
| `dispatched_events()`                   | `HashMap<&'static str, usize>` de todos os dispatches |

Duas variantes restringem o que é faked:

```rust,ignore
// Só faz fake destes - todo o resto despacha normalmente.
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// Faz fake de todo evento, EXCETO estes.
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

E uma variante suprime sem registrar:

```rust,ignore
EventFacade::muted(async {
    // Nenhum listener dispara, nenhum evento é registrado.
    run_bulk_import().await;
})
.await;
```

`muted` NÃO adquire o serializer, então escopos mutados podem executar
em paralelo. Veja [Eventos](events.md) para a maquinaria completa,
incluindo `assert_listening` (que observa somente os registros de
listener que acontecem *dentro* do escopo do fake).

## Armazenamento - `Storage::fake()`

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

#[tokio::test]
async fn invoice_upload_persists() {
    let _guard = Storage::fake();
    let disk = Storage::disk("default").unwrap();

    upload_invoice(b"%PDF-1.7 …").await.unwrap();

    disk.assert_exists("invoices/2026/05/30/inv-00042.pdf").await;
    disk.assert_contents("invoices/2026/05/30/inv-00042.pdf", b"%PDF-1.7 …").await;
}
```

A guarda pré-registra um disco em memória `"default"`, então testes
triviais não precisam de nenhum setup de disco. Registre discos
adicionais sob nomes customizados com
`Storage::register_memory("audit_logs")` de dentro do teste, se o
código sob teste buscar um disco não-padrão.

| Asserção                                        | Faz a asserção de que…                            |
|----------------------------------------------------|-----------------------------------------------------|
| `disk.assert_exists(path).await`                 | o caminho existe                                   |
| `disk.assert_contents(path, &expected).await`    | o arquivo corresponde a `expected` byte a byte     |
| `disk.assert_missing(path).await`                | o caminho não existe                               |
| `disk.assert_count(dir, n, recursive).await`     | `dir` contém exatamente `n` entradas               |
| `disk.assert_directory_empty(dir).await`         | `dir` não tem entradas (recursivo)                 |

Os cinco entram em pânico numa correspondência falha, com o caminho
do disco na mensagem. Veja
[Sistema de arquivos e armazenamento](filesystem.md) para a própria
facade `Storage` e a história de drivers (memory / fs / s3 / azblob /
gcs).

## Cliente HTTP - `Http::fake`

```rust,ignore
use suprnova::{Http, fake_response, assert_sent, assert_not_sent};

#[tokio::test]
async fn payment_webhook_is_acked() {
    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 201, serde_json::json!({
            "id": "ch_42",
            "status": "succeeded",
        }));

        let result = charge_card(amount_cents).await;

        assert!(result.is_ok());
        assert_sent(|r| r.method == "POST" && r.url.contains("/v1/charges"));
        assert_not_sent(|r| r.method == "DELETE");
    })
    .await;
}
```

`fake_response(method, url_substring, status, body)` enfileira uma
resposta pré-fabricada. O método `"*"` corresponde a qualquer método.
Cada entrada pré-fabricada é consumida na primeira solicitação que
corresponder; solicitações subsequentes que correspondam caem para a
próxima entrada pré-fabricada, ou retornam um `200 {}` vazio.

| Helper                                       | Propósito                                                  |
|------------------------------------------------|-------------------------------------------------------------|
| `Http::fake(\|\| async { … }).await`         | instala o escopo de fake task-local                        |
| `fake_response(method, url_substring, …)`    | enfileira uma resposta pré-fabricada                        |
| `assert_sent(\|r\| pred)`                    | faz a asserção de que pelo menos uma solicitação registrada corresponde |
| `assert_not_sent(\|r\| pred)`                | faz a asserção de que nenhuma solicitação registrada corresponde |

### Tasks spawnadas não herdam o fake por padrão

`tokio::spawn` não leva os task-locals para dentro da future spawnada,
então trabalho que escapa da task pai escapa do fake também. Duas
ferramentas lidam com isso:

```rust,ignore
// Precaução extra: transforma toda chamada de saída sem fake em um erro definitivo.
let _guard = suprnova::FailOnRealCallsGuard::install();

Http::fake(|| async {
    fake_response("GET", "/child", 204, serde_json::json!({}));

    // Opt-in explícito: este filho vê o estado de fake do pai.
    let handle = Http::spawn_with_fake_inheritance(async {
        Http::get("https://child.test").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);
})
.await;
```

`FailOnRealCallsGuard` é RAII - instale-o no topo de um teste e
qualquer chamada de saída que não atinja um fake ativo dá erro em vez
de tocar a rede. `Http::spawn_with_fake_inheritance` é o opt-in
explícito para tasks que devem compartilhar o estado de fake do pai.
Veja [Cliente HTTP](http-client.md) para a discussão completa.

## Transmissão

A transmissão por WebSocket tem um fixture de teste paralelo, mas sua
forma difere o bastante para viver em seu próprio capítulo:
`RecordingBroadcastHub` é um `BroadcastHub` de verdade que registra
todo envelope publicado enquanto ainda entrega aos assinantes ativos.
Vincule-o no lugar de `InMemoryBroadcastHub` e chame
`hub.broadcasts()` / `hub.assert_broadcast(channel, event)`. Veja
[Transmissão](broadcasting.md) para o modelo de transmissão e o uso do
hub de gravação.

## Onde cada fake vive

| Superfície    | Fonte                                  | Re-export da facade                          |
|----------------|-----------------------------------------|-------------------------------------------------|
| Mail           | `framework/src/mail/mod.rs`            | `suprnova::{Mail, MailFake}`                   |
| Notificações   | `framework/src/notifications/testing.rs` | `suprnova::{Notify, NotifyFakeGuard}` + `suprnova::notifications::testing::*` |
| Fila           | `framework/src/queue/testing.rs`       | `suprnova::queue::testing::*`                  |
| Barramento     | `framework/src/bus/testing.rs`         | `suprnova::bus::testing::*`                    |
| Eventos        | `framework/src/events/testing.rs`      | `suprnova::{EventFacade, EventFakeGuard}` + `suprnova::events::*` |
| Armazenamento  | `framework/src/filesystem/testing.rs`  | `suprnova::{Storage, DiskExt}` + `suprnova::filesystem::testing::DiskAssertExt` |
| HTTP           | `framework/src/http_client/fake.rs`    | `suprnova::{Http, fake_response, assert_sent, assert_not_sent, FailOnRealCallsGuard, RecordedRequest}` |

Os módulos `testing` e `fake` são condicionados a uma feature do
Cargo chamada `testing`. Ela está no conjunto de features padrão,
então todo teste que depende de `suprnova` ganha os helpers de graça.
Os próprios ganchos são `#[doc(hidden)]` onde poderiam ser alcançados
por acidente a partir de código de aplicação; a salvaguarda que
sustenta isso é a validação de `APP_KEY` do `Server::from_config`, que
executa em todo boot independentemente de quais helpers de teste
estão compilados. Veja [Testes](testing.md) para a história de builds
de produção.

## Por que essas formas, e não uma forma só

Uma única forma uniforme seria mais organizada na página e pior na
prática. Cada forma existe porque o estado subjacente tem semânticas
de concorrência diferentes:

- **O transporte do Mail** é um `Arc<dyn MailTransport>` global,
  trocado pela guarda. Asserções em método sobre a guarda retornada
  amarram quem faz a asserção à instalação específica, o que torna
  impossível chamar asserções quando nenhum fake está ativo.
- **Notify / Queue / Bus / Events** fazem asserção sobre payloads
  tipados heterogêneos - toda asserção é genérica sobre o tipo de
  evento/job/comando. Funções livres num módulo `testing` compõem com
  parâmetros de tipo de forma mais limpa do que um conjunto de
  métodos escrito à mão numa guarda.
- As asserções do **Storage** são por disco, não por fake - o mesmo
  `disk.assert_exists(…)` funciona contra um disco de memória faked
  ou um disco `s3` real numa suíte de integração. Colocá-las no disco
  via uma extension trait preserva essa simetria.
- **HTTP** precisa seguir tasks, não a call stack. `Http::fake` é o
  único fake cujo escopo não pode ser expresso como uma guarda - a
  semântica de spawn força uma closure.

Se algum dia você se encontrar procurando por um helper que não
existe, leia o capítulo relevante; a superfície pública de testes é
documentada de forma exaustiva por subsistema.

## Próximos passos

- [Testes](testing.md) - a macro `#[suprnova_test]`, `TestDatabase`,
  `expect!`, e `TestContainer::fake`
- [Testes HTTP](http-tests.md) - dirigindo `handle_request`
  diretamente sem abrir um socket
- [Testes de banco de dados](database-testing.md) - a história do
  banco de dados em memória por teste
- [Contêiner de serviços](container.md) - `TestContainer::fake` para
  troca de serviços injetados

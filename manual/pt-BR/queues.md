# Fila

A facade `Queue` despacha trabalho de background para um driver e
deixa um processo worker separado drená-lo: handlers HTTP retornam
rápido, o trabalho pesado executa nos bastidores. Recorra a ela sempre
que uma solicitação de outra forma bloquearia em algo que pode ser
feito depois - enviar mail, acionar um webhook, gerar um relatório.
Combine com [`Bus`](bus.md) quando você quer que o trabalho execute
*agora* na task atual e retorne um resultado tipado; combine com
[`Events`](events.md) quando você quer que um sinal faça fan-out para
muitos listeners.

## Início rápido

Defina um job, registre-o uma vez no boot, faça push dele:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use suprnova::{error::FrameworkError, queue::{Job, Queue}};

#[derive(Serialize, Deserialize)]
struct SendWelcomeEmail { user_id: i64 }

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        // … de fato envie o mail
        Ok(())
    }
}

// Boot uma vez (o processo worker e o processo de dispatch precisam disso).
Queue::set_driver(std::sync::Arc::new(suprnova::queue::MemoryQueueDriver::new()));
suprnova::queue::worker::register_job::<SendWelcomeEmail>();

// Faça push a partir de um handler:
Queue::push(SendWelcomeEmail { user_id: 42 }).await?;
```

Um processo worker drena o driver configurado até ser cancelado:

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use suprnova::queue::{Queue, worker::{WorkerConfig, run_worker}};

let driver = Queue::driver()?;
let cfg = WorkerConfig {
    visibility_timeout: Duration::from_secs(60),
    poll_interval: Duration::from_millis(100),
    max_jobs: None,
    queues: Vec::new(),
};
let shutdown = CancellationToken::new();
run_worker(driver, cfg, shutdown).await;
```

Em um app com scaffold, o worker é iniciado pelo subcomando `queue:work`
do binário - `cargo run -- queue:work` - que executa a mesma
inicialização que seu servidor HTTP executa, então observers e
listeners registrados em `bootstrap()` disparam identicamente para
inserts vindos de um handler de fila.

## Drivers

Cinco drivers vêm no próprio repositório. Configure pelo env
`QUEUE_DRIVER` ou chamando `Queue::set_driver(...)` programaticamente.

| Driver | Usar para | Pontos fortes |
| --- | --- | --- |
| `MemoryQueueDriver` | testes, apps de processo único | `tokio::time::DelayQueue` para `available_at`, compatível com relógio virtual |
| `RedisQueueDriver` | fan-out em produção | grupos de consumidores + `XAUTOCLAIM` + jobs atrasados apoiados em ZSET |
| `DatabaseQueueDriver` | apps de banco único | `FOR UPDATE SKIP LOCKED` no Postgres/MySQL, serializado por `BEGIN` no SQLite |
| `SyncQueueDriver` | dev, CI | roda o handler inline no `push`, sem worker |
| `NullQueueDriver` | wrappers de teste | descarta todo push sem executar |

O `Queue::bootstrap_from_env()` lê `QUEUE_DRIVER` e conecta o driver
correspondente; o `Queue::bootstrap_default()` sempre conecta o driver de
memória. O caminho de boot do servidor chama um desses por você - a
maioria dos apps só configura pelo env.

O `FailoverQueueDriver` não é um sexto backend. Ele embrulha uma lista
ordenada dos drivers acima, para que um push que uma conexão recuse caia
para a seguinte. Veja [Conexões de failover](#conexões-de-failover).

### Configuração via ambiente

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# Driver de banco de dados - DB::init() precisa rodar antes
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

O driver de banco de dados valida `QUEUE_DB_TABLE` como identificador SQL
na construção, então um valor de env malformado falha o boot em vez de
chegar à composição do SQL. O Redis usa o sea-streamer-redis por baixo dos
panos com `AutoCommit::Disabled`; o timeout de visibilidade é fixado no
momento da construção do grupo de consumidores, então o argumento
`visibility_timeout` de cada pop é ignorado no Redis (uma divergência
documentada do contrato da trait, imposta pelos Redis Streams).

### Por que Suprnova diverge

O Laravel roteia tudo que é enfileirável pelo Bus, distinguindo os jobs
`ShouldQueue` no momento do dispatch. O Suprnova separa os dois: `Bus` para
trabalho síncrono que devolve um resultado tipado, `Queue` para trabalho
assíncrono que sobrevive a um crash de processo. O PHP precisa do
roteamento implícito porque o seu modelo de uma solicitação por processo
torna difícil modelar "faça isto depois, em outro processo" de outra
maneira. O Tokio não - `Bus::dispatch` contra `Queue::push`, explícitos,
são mais claros, mais rápidos e expõem a escolha de durabilidade no ponto
da chamada. Veja [`bus.md`](bus.md) para a comparação lado a lado.

## Conexões de failover

O `FailoverQueueDriver` embrulha uma lista ordenada de conexões. Um push
que a primeira conexão recusar é repetido na seguinte, e assim por diante,
lista abaixo, para que uma indisponibilidade do Redis não transforme todo
dispatch em job perdido.

Configure a partir do env:

```bash
QUEUE_DRIVER=failover
QUEUE_FAILOVER_CONNECTIONS=redis,database

# Cada conexão lê as próprias variáveis, exatamente como leria se fosse
# a QUEUE_DRIVER sozinha.
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_DB_TABLE=jobs
```

Ou conecte você mesmo, quando as conexões precisarem de configuração em
runtime que o env não consegue expressar:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{
    DatabaseQueueDriver, FailoverQueueDriver, Queue, QueueDriver, RedisQueueDriver,
};
use suprnova::{DB, FrameworkError};

pub async fn register() -> Result<(), FrameworkError> {
    let redis = RedisQueueDriver::connect(
        "redis://127.0.0.1:6379",
        "suprnova-queue",
        "default",
        "consumer-1",
        Duration::from_secs(60),
    )
    .await?;
    let database =
        DatabaseQueueDriver::new(DB::connection()?.inner().clone(), "jobs".to_string())?;

    let failover = FailoverQueueDriver::new(vec![
        ("redis".to_string(), Arc::new(redis) as Arc<dyn QueueDriver>),
        ("database".to_string(), Arc::new(database) as Arc<dyn QueueDriver>),
    ])?;
    Queue::set_driver(Arc::new(failover));
    Ok(())
}
```

A `String` de cada entrada é o rótulo da conexão informado no evento
`QueueFailedOver`. Ele não é derivado do tipo do driver, porque duas
conexões podem rodar o mesmo driver.

A `QUEUE_FAILOVER_CONNECTIONS` é obrigatória quando
`QUEUE_DRIVER=failover`, e a lista não pode conter `failover` em si. Uma
entrada que nomeie um driver inexistente é um erro de boot, e não o
fallback de avisar e usar memória que o `QUEUE_DRIVER` aplica a si mesmo:
dentro de uma cadeia de failover, um erro de digitação que virasse em
silêncio uma conexão em memória colocaria um backend efêmero em uma lista
durável.

### Escritas fazem failover, leituras não

Só `push` e `bulk_push` percorrem a lista de conexões. Toda outra
operação - `pop`, `ack`, `nack`, `release`, `settle`, `clear`, os quatro
contadores e as três listagens de inspeção - vai para a **primeira**
conexão e para nenhuma outra.

Essa assimetria é o contrato, não um esquecimento. Um token de reserva só
faz sentido para o driver que o emitiu, então dar ack contra outra conexão
não liquidaria nada e corromperia as duas. Os contadores e as listagens
seguem a mesma regra, para que o que você inspeciona seja o que o worker
desta conexão drena, e não uma soma entre backends que não bate com a
visão de worker nenhum.

**Um worker na conexão de failover drena só a primária.** Jobs que fizeram
failover para um fallback precisam de um worker rodando direto contra
aquela conexão de fallback:

```bash
# Drena a primária da cadeia de failover.
QUEUE_DRIVER=failover QUEUE_FAILOVER_CONNECTIONS=redis,database ./app queue:work

# Drena o que fez failover para o banco de dados. Rode isto também.
QUEUE_DRIVER=database ./app queue:work
```

A documentação do Laravel traz o mesmo aviso pela mesma razão.

Isso alcança as chains, mas só por uma porta. Um worker liquida um job e
enfileira o próximo elo de uma [chain enfileirada](#chains-enfileiradas)
em uma única chamada, `settle`, e o decorator delega essa chamada só à
primária. Então, com uma primária transacional como o driver de banco de
dados, uma primária fora do ar faz a liquidação falhar e nada cai para a
seguinte: o worker deixa a reserva intacta e a expiração da visibilidade
reentrega o job. A queda para a seguinte acontece quando a primária
responde `Settled::Unsupported`, que é o que os drivers de memória e de
Redis fazem, porque aí o worker faz push do próximo elo pelo driver
vinculado como qualquer outro push - e esse push cai para a seguinte. O
resto daquela chain então espera por um worker na conexão de fallback. Sem
um, a chain trava - o elo é durável e nada se perde, mas também nada o
executa.

### O evento `QueueFailedOver`

Cada conexão que recusa um push despacha
`queue::events::QueueFailedOver { connection, job_name, exception }`, mas
só no push que leva aquela conexão *para dentro* da falha. Uma conexão já
conhecida como em falha fica quieta até um push posterior ter sucesso
nela, o que a rearma. Uma indisponibilidade de quatro horas produz um
evento, não um por dispatch, que é o que o torna usável como alerta.

O `connection` é o rótulo da conexão que falhou, não o da que aceitou o
job.

Quando toda conexão recusa um push, o push retorna o erro da última
conexão. O `bulk_push` faz push de cada envelope separadamente, então cada
um cai para a seguinte por conta própria: um batch que a primária aceitou
pela metade nunca é empurrado inteiro para o fallback, e cada envelope
mantém o `available_at` com que foi montado. Um batch não é atômico. Se um
envelope for recusado por toda conexão, o `bulk_push` retorna o erro
daquele envelope com os envelopes anteriores já enfileirados.

Cair para a seguinte não é deduplicação. O decorator nunca refaz a
tentativa de um envelope que uma conexão aceitou, mas uma conexão que
escreve o envelope e *depois* reporta falha produz uma duplicata na
próxima conexão, porque "escrevi e perdi o reconhecimento" é
indistinguível de "nunca peguei". As duas cópias carregam o mesmo id de
job. Esse é o contrato de entrega ao menos uma vez do framework, o mesmo
que faz da idempotência do handler um requisito em todo o resto - veja
[Idempotência é o contrato do worker com você](#idempotência-é-o-contrato-do-worker-com-você).

### Por que Suprnova diverge

A conexão de failover do Laravel é um array `connections` em
`config/queue.php`, resolvido pelo registry de conexões. O Suprnova não
tem registry de driver por conexão - um driver é vinculado para todo o
processo -, então os rótulos vêm de `QUEUE_FAILOVER_CONNECTIONS` (ou da
`String` que você passa para `FailoverQueueDriver::new`) e as leituras
delegam ao primeiro *driver*, e não a uma conexão nomeada.

O `FailoverQueue::bulk` do Laravel percorre os jobs individualmente para
que o atraso de cada um sobreviva. O Suprnova resolve o atraso no envelope
antes de qualquer driver vê-lo, então o laço por envelope o preserva de
graça - mas o laço continua sendo o que impede que um batch que aterrissou
pela metade sofra push duplicado, então ele fica.

## Variantes de push

Toda variante de push recebe um valor tipado `J: Job` e retorna quando o
envelope é confirmado no driver - não quando o handler roda.

| Método | Comportamento |
| --- | --- |
| `Queue::push(job)` | enfileira imediatamente |
| `Queue::push_later(job, at)` | disponível em um `DateTime<Utc>` específico |
| `Queue::later(delay, job)` | disponível depois de `delay` a partir de agora |
| `Queue::push_with(job, overrides)` | enfileira imediatamente com `EnvelopeOverrides` por push |
| `Queue::push_after_commit(job)` | enfileira quando a `DB::transaction` ao redor faz commit |
| `Queue::later_with(delay, job, overrides)` | disponível depois de `delay` a partir de agora, com `EnvelopeOverrides` por push |
| `Queue::push_unique(job)` | deduplica por `J::unique_id` dentro de `J::unique_for`, retorna `Ok(true)` quando o envelope foi enviado, `Ok(false)` quando uma chave de deduplicação viva o suprimiu |
| `Queue::push_unique_later(job, at)` | único + agendado |
| `Queue::later_unique(delay, job)` | único + atrasado |
| `Queue::bulk(vec![job1, job2, ...])` | faz push de cada job (o driver pode usar um caminho bulk nativo) |

O `push_unique` exige que a camada de cache esteja inicializada - o lock de
deduplicação vive em [`Cache`](cache.md) via
[`Idempotency::commit_on_success`](idempotency.md). Um push que falha
libera a chave de deduplicação para que o chamador possa tentar de novo;
um push bem-sucedido a mantém por `J::unique_for` segundos. O job precisa
sobrescrever `Job::unique_id(&self)` para retornar `Some(id)` - `None`
retorna um erro interno.

O booleano responde a uma pergunta - "este job está na fila?" - e há um
terceiro caso por trás dele. Se o lease do lock de deduplicação for perdido
enquanto o push está em voo, o push ainda assim se completa (a camada de
idempotência nunca cancela um corpo que já pode ter tido efeito) e você
ainda recebe `Ok(true)`, com um log de nível `warn` nomeando o job e a sua
chave única. O job está enfileirado; o que fica sem prova é que ninguém
mais enfileirou o mesmo em paralelo. O seu handler já tem de tolerar
reentrega, então isso não pede tratamento extra - mas o log está ali
porque uma rajada deles significa que o cache que sustenta o seu lock de
deduplicação está sofrendo.

### Único até o processamento

Um lock de unicidade normalmente dura a janela `unique_for` inteira, mesmo
depois de o job ter rodado. Quando o lock existe para unificar duplicatas
*enfileiradas* e não para serializar a execução, adira a liberá-lo no
instante em que o processamento começa:

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct RebuildSearchIndex {
    index: String,
}

#[async_trait]
impl Job for RebuildSearchIndex {
    fn job_name() -> &'static str { "rebuild-search-index" }
    fn unique_id(&self) -> Option<String> { Some(self.index.clone()) }
    fn unique_until_processing() -> bool { true }
    fn unique_for() -> Duration { Duration::from_secs(3600) }

    async fn handle(self) -> Result<(), FrameworkError> {
        // Uma reconstrução que roda por 20 minutos não engole mais o
        // re-dispatch que chega no minuto 2.
        Ok(())
    }
}
```

O worker libera o lock depois da passagem do middleware do job e
imediatamente antes de o handler rodar. Seguem quatro consequências:

- Um job que um middleware devolve para a fila mantém o seu lock. Ele não
  começou a ser processado, então nada mudou para uma duplicata.
- Um job que um middleware curto-circuita de qualquer outra maneira abre
  mão do seu lock, porque ele nunca vai ser processado. Isso cobre apagar
  o job, mandá-lo para dead-letter e reportá-lo como completo sem nunca
  chamar o handler.
- Um job que falha libera o seu lock e mesmo assim é repetido. O lock se
  foi no instante em que o processamento começou, então uma duplicata pode
  entrar na fila enquanto a tentativa falha cumpre o seu backoff, e você
  acaba com dois envelopes para o mesmo id único. Essa é a troca que esta
  adesão faz. Se uma nova tentativa tiver de continuar segurando a vaga,
  deixe `unique_until_processing` desligado e deixe o TTL de `unique_for`
  cobrir a cadeia de tentativas inteira.
- A liberação tem escopo por dono. O `push_unique` registra o token de dono
  do lock no envelope, e o worker libera com esse token, então uma
  tentativa reentregue nunca consegue liberar um lock que um dispatch mais
  novo adquiriu desde então.

O `unique_until_processing` precisa das mesmas duas coisas de que o
`push_unique` precisa: um `unique_id` que retorne `Some(id)` e uma camada
de cache inicializada.

Sob o driver `sync` o handler roda inline dentro da própria chamada de
`push_unique` que tomou o lock, então o job libera um lock que quem o
chamou ainda detém nominalmente. Se esse handler rodar por mais de um terço
de `unique_for`, o renovador do lease de deduplicação percebe que o lock
sumiu e registra um aviso de lease perdido, e o `push_unique` registra por
cima o seu próprio aviso de "não foi possível provar a exclusividade". Os
dois são esperados aqui, e não uma falha: o job rodou, o push retorna
`Ok(true)` e o lock sumiu porque o próprio job o liberou.

### Por que Suprnova diverge

O Laravel libera o lock de um job único *comum* assim que o handler
retorna. O Suprnova em vez disso deixa esse lock expirar com o TTL de
`unique_for`, o que mantém a janela de deduplicação honesta quando um
worker morre no meio do job: a janela que você configurou é a janela que
você recebe, tendo o handler retornado ou não. O
`unique_until_processing` se comporta igual nos dois frameworks.

O Suprnova também nunca força a liberação de um lock de unicidade. O
Laravel recorre a uma liberação forçada para uma primeira tentativa que
não carrega token de dono. Os únicos envelopes que chegam a um worker do
Suprnova sem um são envelopes enfileirados antes de o token existir, e
esses mantêm a expiração por TTL em vez de arriscar uma liberação que
apague o lock de um dispatch mais novo.

### Debounce - manter o último dispatch, não o primeiro

O `push_unique` suprime uma duplicata e mantém o **primeiro** dispatch. O
debounce é o oposto: ele mantém o **último**. Uma rajada de vinte eventos
"este pedido mudou" vira uma reindexação, uma janela depois do vigésimo,
carregando o payload mais novo.

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct ReindexOrder {
    order_id: u32,
}

#[async_trait]
impl Job for ReindexOrder {
    fn job_name() -> &'static str { "reindex-order" }
    fn debounce_for() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn max_debounce_wait() -> Option<Duration> { Some(Duration::from_secs(300)) }
    fn debounce_id(&self) -> Option<String> { Some(self.order_id.to_string()) }

    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}
```

- `debounce_for` é a janela: cada dispatch a rearma, então a execução acontece
  30 segundos depois do dispatch *mais recente*.
- `max_debounce_wait` impede que uma rajada contínua adie o trabalho para
  sempre. Uma vez que a rajada já esteja adiando por cinco minutos, o próximo
  dispatch é enfileirado sem atraso. A janela então recomeça, de modo que cada
  rajada mede a espera máxima dela a partir do próprio primeiro dispatch.
- `debounce_id` delimita a janela. Vinte atualizações no pedido 7 viram uma
  execução; uma atualização no pedido 8 não é afetada por elas. Omita-o e todo
  dispatch do job compartilha uma única janela.

Todo dispatch ainda é enfileirado. O colapso é resolvido no worker: cada push
sobrescreve um token de cache, e o worker descarta qualquer envelope cujo token
um dispatch mais novo tenha substituído, reconhecendo-o e emitindo
`JobDebounced`. É isso que faz a execução sobrevivente carregar o payload mais
novo em vez do mais antigo. Se o token expirou ou foi despejado, o job roda - o
debounce falha em aberto, porque um token perdido não é prova de que outra
pessoa é dona da janela.

O [driver `sync`](#drivers) não tem worker, então ele roda todo dispatch inline
e nada é jamais colapsado. O driver sync do Laravel se comporta do mesmo jeito.
O `Queue::bulk` faz push no nível do driver e também não arma uma janela, então
um job com debounce enviado em bulk roda todas as cópias. O `Queue::bulk` do
Laravel pula a própria aquisição de debounce pela mesma razão.

Defina a janela no local de chamada quando ela pertencer a quem chama:

```rust
use suprnova::queue::DebounceOptions;

Queue::push_debounced(
    ReindexOrder { order_id: 7 },
    DebounceOptions::new(Duration::from_secs(30))
        .max_wait(Duration::from_secs(300))
        .id("7"),
)
.await?;
```

Um job não pode declarar `debounce_for` e `unique_id` ao mesmo tempo: a
unicidade mantém o primeiro dispatch de uma rajada e o debounce mantém o
último, então o push retorna um erro nomeando os dois. Chains e batches recusam
um job com debounce por uma razão relacionada - um elo substituído é
descartado, o que deixaria o resto de uma chain encalhado, e um job de batch
descartado deixa a contagem de pendentes do batch acima de zero, então os
callbacks dele nunca disparam.

### Substituições por push com `EnvelopeOverrides`

O `Queue::push_with` e o `Queue::later_with` recebem um
`EnvelopeOverrides` junto do job, para aquele dispatch específico que
precisa de comportamento de fila, conexão, timeout ou retry diferente dos
padrões do próprio job:

```rust
use std::time::Duration;
use suprnova::queue::{EnvelopeOverrides, Queue};

let overrides = EnvelopeOverrides {
    queue: Some("priority".into()),
    timeout: Some(Duration::from_secs(10)),
    max_tries: Some(1),
    ..Default::default()
};

Queue::push_with(SendWelcomeEmail { user_id: 42 }, overrides.clone()).await?;

// A contraparte com atraso, espelhando a relação de `Queue::later` com `Queue::push`.
Queue::later_with(Duration::from_secs(60), SendWelcomeEmail { user_id: 42 }, overrides).await?;
```

Todo campo tem `None` como padrão e defere à resolução normal que o
`Queue::push` já executa; um campo `Some` ganha de tudo isso para este
push em particular, superando tanto uma rota registrada com
[`Queue::route`](#roteamento-de-fila) quanto a declaração `Job::*` do
próprio job para aquele campo:

| Campo | Supera |
| --- | --- |
| `queue` | `Queue::route`, `Job::queue()` |
| `connection` | `Queue::route`, `Job::connection()` |
| `timeout` | `Job::timeout()` |
| `fail_on_timeout` | `Job::fail_on_timeout()` |
| `max_tries` | `Job::max_tries()` |
| `backoff` | `Job::backoff()` |
| `after_commit` | `Job::after_commit()` |

O `EnvelopeOverrides` é a primitiva sobre a qual tanto o
`Mail::on_queue`/`.on_connection()` quanto o ajuste de fila por notificação
do `Notify::queue` são construídos - veja [Correio](mail.md#queueing) e
[Notificações](notifications.md).

### Atraso declarado pelo job

Um job pode carregar o próprio atraso padrão em vez de todo ponto de
chamada repetir `Queue::later(Duration::from_secs(60), job)`:

```rust
impl Job for SendDigest {
    // ...
    fn delay() -> Option<Duration> { Some(Duration::from_secs(60)) }
}
```

`Queue::push(job)`, `Queue::push_with(job, overrides)`,
`Queue::push_unique(job)` e `Queue::bulk(vec![job1, job2])` respeitam todos
esse atraso - o `available_at` passa a ser `now + J::delay()` em vez de
`now`. O `Queue::bulk` resolve o atraso uma vez por chamada, já que todo
job do vetor compartilha o mesmo `J` concreto e portanto o mesmo
`Job::delay()`.

Um atraso explícito no ponto de chamada sempre ganha:
`Queue::push_later(job, at)`, `Queue::later(delay, job)`,
`Queue::later_with(delay, job, overrides)`,
`Queue::push_unique_later(job, at)` e `Queue::later_unique(delay, job)`
usam todos, literalmente, o timestamp ou o atraso que o chamador passou -
o `Job::delay()` não é consultado para nenhum deles. Recorra ao método da
trait quando todo dispatch de um tipo de job deve começar atrasado por
padrão; recorra a uma das variantes `later`/`push_later` para um atraso de
que um dispatch específico precisa mas que o tipo não declara de outra
forma.

Batches e chains também não o consultam: `Queue::batch()...add(job)` e
`Queue::chain()...add(job)?` montam os seus envelopes com `available_at`
definido no momento em que você chamou `add`, então um job com um
`Job::delay()` declarado é despachado imediatamente como parte de um batch
ou de uma chain, mesmo que um `Queue::push(job)` puro do mesmo job fosse
esperar. Dê ao job um atraso explícito de outra maneira - um campo no
próprio job, aplicado no `handle()` - se um passo em batch ou em chain
precisar de um.

### Por que Suprnova diverge

O `$job->delay` do Laravel é uma propriedade de instância, definida por
dispatch (`SendDigest::dispatch($user)->delay(60)`), então dois dispatches
da mesma classe podem carregar atrasos diferentes. O `Job::delay()` aqui é
um padrão de nível de classe, como `Job::queue()` ou `Job::max_tries()` -
um dispatch que precise de um atraso calculado a partir dos próprios dados
usa `Queue::later`/`push_later`, que já supera o padrão declarado.

### Dispatch pós-commit

Um job enviado dentro de uma [`DB::transaction`](database.md#transactions)
está correndo uma corrida contra essa transação. Um worker em outro
processo pode dar pop no envelope, procurar a linha que a transação ainda
mantém aberta e falhar - ou pior, a transação sofre rollback e o job roda
contra dados que não existem mais.

Faça o job aderir a esperar pelo commit:

```rust
use suprnova::{DB, FrameworkError, Job, Queue, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct SendReceipt {
    order_id: i64,
}

#[async_trait]
impl Job for SendReceipt {
    fn job_name() -> &'static str { "send-receipt" }
    fn after_commit() -> bool { true }

    async fn handle(self) -> Result<(), FrameworkError> {
        // A linha do pedido tem durabilidade garantida quando isto rodar.
        Ok(())
    }
}

DB::transaction(|_tx| {
    Box::pin(async move {
        let order = Order::create(suprnova::attrs! { total: 4999i64 }).await?;
        // Nada chega ao driver aqui.
        Queue::push(SendReceipt { order_id: order.id }).await?;
        Ok::<(), FrameworkError>(())
    })
})
.await?;
// O envelope está na fila agora, e só agora.
```

Três regras cobrem todos os casos:

- **Dentro de uma transação, o push inteiro espera pelo commit.** Não só a
  escrita do driver: a montagem do envelope, o evento `JobQueueing` e o
  evento `JobQueued` também acontecem todos no momento do commit, então um
  listener nunca é avisado de um job que um rollback depois descarta.
- **Um rollback o descarta.** O push simplesmente nunca acontece. Se ele
  tomou um lock de unicidade, o rollback devolve esse lock.
- **Fora de uma transação o push acontece imediatamente.** É isso que torna
  a adesão segura de declarar no tipo do job: um ponto de dispatch não
  precisa saber se o caminho de código em que ele está é transacional.

Um rollback de [savepoint](database.md#savepoints) conta como um rollback
para tudo que foi registrado dentro dele. O `tx.rollback_to("name")`
descarta os pushes adiados desde o `tx.savepoint("name")` e libera os locks
que eles tomaram, naquele instante, para que um re-dispatch dentro da mesma
transação ganhe a chave de novo. Pushes feitos antes do savepoint ficam
intocados, e um savepoint em que você nunca faz rollback mantém tudo que
foi registrado dentro dele.

Por dispatch, em vez de por tipo de job, use
`EnvelopeOverrides::after_commit`. `Some(true)` é o `afterCommit()` do
Laravel e tem o atalho `Queue::push_after_commit(job)`; `Some(false)` é o
`beforeCommit()` do Laravel, para aquele dispatch específico que tem de
ficar visível para um worker antes de o commit acontecer:

```rust
use suprnova::queue::{EnvelopeOverrides, Queue};

// Adia um job cujo tipo não adere.
Queue::push_after_commit(SendWelcomeEmail { user_id: 42 }).await?;

// Faz push imediatamente mesmo que o tipo do job adira.
Queue::push_with(
    SendReceipt { order_id: 7 },
    EnvelopeOverrides { after_commit: Some(false), ..Default::default() },
)
.await?;
```

Um `Queue::push` adiado resolve de novo o
[`Job::delay()`](#atraso-declarado-pelo-job) contra o commit, e não contra
o push, porque o atraso quer dizer "espere isto depois do dispatch" e para
um job adiado o dispatch *é* o commit. Um timestamp explícito é a intenção
do chamador sobre um momento no tempo, então `Queue::push_later`,
`Queue::later` e `Queue::later_with` levam o seu através do adiamento sem
alteração.

O `Queue::push_unique` adia com uma assimetria deliberada: o lock de
deduplicação é tomado imediatamente, então um segundo `push_unique` para o
mesmo id único dentro da mesma transação continua sendo suprimido e
continua reportando `Ok(false)`. Só o envelope espera. O vencedor reporta
`Ok(true)` mesmo com o seu push pendente, porque o push vai acontecer. Um
rollback libera o lock que ele tomou, com escopo por dono, para que a
janela `unique_for` nunca fique bloqueada por um dispatch que nunca
aconteceu - e o mesmo vale para qualquer outro desfecho em que o commit não
acontece, incluindo um `COMMIT` recusado. O único limite dessa garantia é o
próprio TTL: uma transação que fica aberta por mais tempo que `unique_for`
pode ter o seu lock expirado e retomado por outro dispatch no meio do
caminho, então dê a `unique_for` folga acima da sua transação mais longa se
a deduplicação importa. A família `push_unique*` não recebe
`EnvelopeOverrides`, então o `Job::after_commit()` é a única coisa que
decide se um push único adia - não há substituição por push para isso.

Batches e chains não adiam, da mesma forma que não consultam o
`Job::delay()`: `Queue::batch()` e `Queue::chain()` montam e enviam os seus
envelopes diretamente. Envolva a chamada de `.dispatch()` para que ela rode
depois que a transação retornar, se um batch tiver de esperar por um
commit.

[Correio](mail.md#queueing) e [notificações](notifications.md)
enfileirados também não adiam. Cada um viaja em um único tipo de job
compartilhado (`SendMailJob` / `SendNotificationJob`), e ainda não há
equivalente a `ShouldQueueAfterCommit` em `Mailable` ou `Notification`,
então uma chamada de `Mail::queue` ou `Notify::queue` dentro de uma
transação chega ao driver imediatamente. Envie esses depois que a transação
retornar.

Sob `Queue::fake()` um push é registrado imediatamente, adiamento e tudo,
para que um teste possa fazer asserção sobre ele sem dar commit em nada.
Isso casa com o `Bus::fake` do Laravel, e é o que permite a um teste
conduzir um handler transacional e fazer asserção sobre os dispatches dele
no mesmo fôlego.

### Por que Suprnova diverge

O `Queue::bulk` é monomórfico - todo elemento compartilha um `J` concreto -
então a sua partição pós-commit é tudo ou nada para a chamada. O Laravel
particiona um array heterogêneo em metades adiadas e imediatas; aqui não há
nada a particionar.

O adiamento está atrelado à forma com closure. Um push dentro de um
[`DB::begin_transaction`](database.md#manual-form) manual acontece
**imediatamente**, porque o modo manual não instala transação ambiente
alguma e portanto não tem commit em que pendurar um callback. Adiar ali
enfileiraria um callback que nada jamais executa, e um dispatch que
desaparece em silêncio é pior do que um que acontece cedo demais. Recorra a
`DB::transaction` quando um dispatch tiver de esperar pelo commit.

O Laravel também lê uma chave de configuração `after_commit` de nível de
conexão como último fallback da sua cadeia de precedência. O Suprnova para
na substituição por push e depois no `Job::after_commit()` do próprio job:
conexões de fila aqui não carregam a própria política de dispatch.

## Configuração de job

Sobrescreva as funções associadas de `Job` para ajustar o comportamento por
impl:

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

    fn delay() -> Option<Duration> { None }                // padrão: sem atraso
    fn max_tries() -> u32 { 5 }                            // padrão: 3
    fn timeout() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn fail_on_timeout() -> bool { false }                 // padrão: false (timeout gera retry)
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Sequence { secs: vec![5, 15, 60, 300] }
    }
    fn unique_id(&self) -> Option<String> {
        Some(format!("welcome:{}", self.user_id))
    }
    fn unique_for() -> Duration { Duration::from_secs(600) }  // padrão: 5 minutos
    fn unique_until_processing() -> bool { true }          // padrão: false (o TTL é a janela)
    fn middleware() -> Vec<std::sync::Arc<dyn JobMiddleware>> {
        vec![/* veja "Middleware de job" abaixo */]
    }
}
```

## Roteamento de fila

Por padrão todo job vai para uma fila e todo worker drena a fila
inteira. Quando alguns jobs são mais lentos ou mais importantes que
outros, você quer pools de workers dedicados: uma exportação de longa
duração não deveria ficar atrás de mil emails de bem-vindo.

Um job pode declarar a que fila pertence:

```rust
#[async_trait]
impl Job for GenerateExport {
    fn job_name() -> &'static str { "GenerateExport" }
    async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }

    fn queue() -> Option<&'static str> { Some("exports") }
    fn connection() -> Option<&'static str> { None }   // conexão padrão
}
```

…e um operador pode sobrescrever isso centralmente, sem tocar no job:

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::route::<GenerateExport>(None, Some("heavy"));
Queue::route::<SendInvoice>(Some("redis"), Some("billing"));
```

A resolução executa da maior prioridade para a menor:

1. uma substituição por push passada para `Queue::push_with` / `Queue::later_with` (veja
   [Substituições por push com `EnvelopeOverrides`](#substituições-por-push-com-envelopeoverrides))
2. uma rota registrada com `Queue::route`
3. o próprio `Job::queue` / `Job::connection` do job
4. o driver / padrão global

Passar `None` para um campo deixa aquela dimensão intacta, então
rotear a conexão de um job não perturba a fila que ele já declarou.

As duas dimensões executam em profundidades diferentes hoje. A
**fila** é honrada de ponta a ponta - estampada no envelope,
armazenada pelo driver, filtrada por `--queue`. A **conexão** resolve
o *nome* de conexão carregado nos eventos de ciclo de vida
`JobQueueing` / `JobQueued`, que é o que listeners e dashboards veem;
um driver global de processo ainda recebe todo push, então rotear a
conexão de um job ainda não seleciona um driver diferente. Declarar
conexões agora é compatível para o futuro, para quando drivers por
conexão chegarem - não é comportamental ainda.

Então dedique um worker a ela:

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # drena toda fila, como antes
```

Um job sem rota pertence a `default`, então `--queue=default` drena
trabalho não roteado em vez de deixá-lo encalhado.

### Encaminhando uma fila inteira

O `Queue::route` é chaveado por tipo de job. Quando você quer drenar um pool
através de outro - aposentar uma fila, absorver um backlog, tirar trabalho de
um pool que você está prestes a derrubar -, chaveie o redirecionamento por
nome de fila em vez disso:

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::forward("default", "high");
Queue::forward_on("exports", "heavy", "redis");   // somente na conexão `redis`
```

A conexão em `forward_on` é um gate, e ela é comparada com o nome de conexão
deste processo - o `Queue::set_connection_name` se você definiu um, o nome do
próprio driver caso contrário. Ela não é comparada com o `Job::connection` do
job, com a conexão de um `Queue::route` nem com uma conexão de
`EnvelopeOverrides` por push: essas nomeiam o que os eventos de ciclo de vida
reportam, e um worker só tem o nome do processo para condicionar a lista de
reivindicação dele. As duas metades do redirecionamento se condicionam a esse
único valor, então um forward nunca consegue mover o push sem mover a
reivindicação.

O redirecionamento se aplica dos dois lados, e é isso que o impede de deixar
trabalho encalhado:

- **Do lado do push**, o nome é reescrito depois que o roteamento e o próprio
  `Job::queue` do job já tiveram a palavra deles, e depois de uma fila de
  `EnvelopeOverrides` por push, se você passou uma.
- **Do lado do pop**, um worker iniciado com `--queue=default` drena `high`.
  Sem essa metade, a fila de destino juntaria jobs que nenhum worker
  reivindica.

Um worker iniciado sem nenhum `--queue` já drena tudo, então um forward não
muda nada para ele. Encaminhar `default` pega os jobs que não nomearam fila
nenhuma, porque um job não roteado pertence a `default`.

Um forward é uma única busca, nunca uma cadeia. Com `a -> b` e `b -> c`
registrados, um push que resolveu para `a` aterrissa em `b`. Registrar
`b -> a` por cima de um `a -> b` existente é, portanto, uma troca coerente de
pools, e não um laço: um push para `a` continua aterrissando em `b`, um push
para `b` agora aterrissa em `a`, e um worker iniciado em qualquer um dos dois
nomes reivindica o outro - nada encadeia, então nada fica encalhado. Uma
rotação mais longa entre mais nomes de fila resolve do mesmo jeito, um salto
independente de cada vez. O `Queue::forward` do Laravel também não tem
verificação de ciclo, pela mesma razão: o resolvedor dele é essa mesma busca
única. Encaminhar uma fila para o próprio nome dela é a identidade - nenhum
redirecionamento - que é como você neutraliza um forward que já registrou.

Só pushes futuros se movem. Envelopes que já estão na fila de origem ficam
lá, e o worker que costumava drená-los agora está reivindicando o destino,
então drene o pool de origem antes de encaminhá-lo. O mesmo vale para o
`queue:retry`: um job que falhou é reenfileirado na fila em que morreu.

A pausa é avaliada antes do redirecionamento, sobre os nomes com que o worker
foi iniciado. O `Queue::pause(&connection, "default")` continua parando um
worker iniciado com `--queue=default`, mesmo enquanto `default` está
encaminhada para `high`. A recíproca também vale: pausar o *destino* do
forward - `Queue::pause(&connection, "high")` - não para um worker iniciado
com `--queue=default`, porque esse worker é alcançado pelo nome de origem
dele, e não pelo nome reescrito. O evento `WorkerQueuePaused` que essa
transição dispara carrega `queue: default`, o nome configurado, nunca `high` -
o Laravel ordena e reporta da mesma forma.

As chamadas de inspeção deliberadamente não são encaminhadas: o
`Queue::pending_jobs(Some("default"))` lista o que está literalmente em
`default`, não o que está em `high`, que é como você enxerga o backlog
encalhado em uma fila de origem que você acabou de encaminhar. O Laravel
resolve o forward ali também; veja a nota de divergência abaixo.

Leia um forward registrado de volta com o `Queue::forward_for("default")`, que
retorna o destino em `queue` e o gate de conexão em `connection`.

### Por que Suprnova diverge

O `Queue::route(...)` do Laravel recebe uma string de classe; o
Suprnova recebe o job como um type parameter, então um job renomeado
ou deletado é um erro de compilação em vez de uma rota que
silenciosamente para de corresponder.

A divergência maior é o que acontece quando um driver não consegue
filtrar. `QueueDriver::pop_from` **rejeita** um filtro de fila que não
consegue honrar em vez de recair para drenar tudo. Um worker instruído
a drenar só `billing` que silenciosamente drena todas as filas parece
idêntico a um deployment funcionando até que o pool errado consuma os
jobs errados - então a má configuração é exposta de forma evidente já
no primeiro poll. Os drivers de memória e de banco de dados filtram
nativamente; um driver que não filtra - o driver Redis é um deles, já
que um único consumer group de stream não tem armazenamento por fila -
vai dar erro em vez de induzir ao erro.

O `Queue::forward` porta a metade fila-para-fila do `Queue::forward` do
Laravel por inteiro, e somente essa metade. O terceiro argumento do Laravel
consegue mover uma fila encaminhada para uma *conexão* diferente, porque o
gerenciador de filas dele resolve um driver por nome de conexão. O Suprnova
tem um único driver global de processo, e um nome de conexão apenas rotula
eventos de ciclo de vida, então o `Queue::forward_on(from, to, connection)`
trata a conexão como um **gate** - ela decide se o redirecionamento por nome
de fila se aplica - e nunca como destino. Pela mesma razão, `to` é obrigatório
aqui, enquanto o do Laravel é opcional: um `to` omitido no Laravel significa
"mova apenas a conexão", que é exatamente a dimensão que o Suprnova não
consegue honrar, então um `forward(from, None)` seria um no-op fantasiado de
mudança de configuração.

As chamadas de inspeção do Laravel seguem um forward, porque o
`pendingJobs($queue)` e os irmãos dele passam pelo mesmo `getQueue()` no nível
do driver por onde passam o push e o pop. O `Queue::pending_jobs` /
`delayed_jobs` / `reserved_jobs` do Suprnova reportam a fila literal que você
nomear. Com um único driver global de processo, a visão literal é a única
forma de enxergar os envelopes que ficaram para trás em uma fila que você
acabou de encaminhar para outro lugar - o backlog que esta seção manda drenar
primeiro. Peça a fila de destino pelo nome para ver onde o trabalho novo está
aterrissando.

### A tabela `jobs`

`DatabaseQueueDriver` espera esse schema. A coluna `queue` é o que
torna possível a filtragem por `--queue`:

```sql
CREATE TABLE jobs (
    id              TEXT PRIMARY KEY,
    job_name        TEXT NOT NULL,
    queue           TEXT NULL,
    envelope_json   TEXT NOT NULL,
    available_at    BIGINT NOT NULL,
    reserved_until  BIGINT NULL,
    reserved_token  TEXT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    created_at      BIGINT NOT NULL
);
CREATE INDEX idx_jobs_available_at ON jobs(available_at);
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

`queue` é nullable, e um job não roteado armazena `NULL` em vez de
`'default'`. Isso é deliberado: uma linha escrita por um binário mais
antigo é indistinguível de uma linha não roteada escrita por um mais
novo, então uma frota de versões mistas drena o mesmo trabalho durante
um rolling upgrade.

Adicionar a coluna a uma tabela existente é **obrigatório**, não só
para filtragem: `push` nomeia a coluna `queue` no seu `INSERT` esteja
o job roteado ou não, então um binário 0.7.0+ falha todo push contra
uma tabela que não a tem. Execute a migration primeiro, depois faça o
rollout dos binários - binários mais antigos listam suas colunas
explicitamente e ignoram a nova, então essa ordem é segura:

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

### Padrões de backoff

| Variante | Comportamento |
| --- | --- |
| `Fixed { secs }` | delay constante por tentativa |
| `Exponential { base_secs, cap_secs, jitter_ratio }` | `min(base * 2^(attempts-1), cap)` × aleatório em `[1±jitter]` |
| `Sequence { secs }` | uma entrada por tentativa; a última entrada se repete quando esgotada |

O padrão é
`Exponential { base_secs: 2, cap_secs: 300, jitter_ratio: 0.25 }` - 2
segundos a 5 minutos com jitter de ±25%.

## Middleware de job

Seis middleware vêm in-tree, todos espelhando
`Illuminate\Queue\Middleware\*`:

| Middleware | Comportamento |
| --- | --- |
| `WithoutOverlapping` | mantém um `Cache::lock` pela duração; libera-com-delay em caso de contenção |
| `RateLimited` | bloqueia com base no orçamento do `RateLimiter`; libera até a janela resetar |
| `ThrottlesExceptions` | rate-limit em *falhas* consecutivas, não em solicitações |
| `Skip::when(cond)` / `Skip::unless(cond)` | descarta o job quando a condição é satisfeita |
| `FailOnException` | promove erros correspondentes a falhas permanentes (sem retry) |
| `SkipIfBatchCancelled` | descarta o job se o batch ao qual pertence foi cancelado |

Conecte-os na impl de `Job`:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{JobMiddleware, RateLimited, WithoutOverlapping};

fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
    vec![
        Arc::new(
            WithoutOverlapping::new("user-42")
                .expire_after(Duration::from_secs(120))
        ),
        Arc::new(
            RateLimited::new(10, Duration::from_secs(60))
                .by("send-mail")
        ),
    ]
}
```

`WithoutOverlapping` e `RateLimited` precisam que o subsistema de
cache esteja inicializado (`Cache::init` ou
`App::bind::<dyn CacheStore>(...)` na inicialização).

### Um lock que não vai liberar não falha o job

Se `WithoutOverlapping` não consegue liberar seu lock depois do
handler ter executado - o backend de cache deu um blip, a conexão caiu -
ele loga em `warn` e retorna o próprio resultado do handler de
qualquer forma. O lock então expira em `expire_after`.

Isso é deliberado. No momento em que a liberação executa, o handler já
efetivou seus efeitos colaterais: linhas escritas, mail enviado,
cobranças feitas. Reportar a falha de liberação como uma falha de job
faria o worker tentar de novo e fazer tudo isso uma segunda vez, o que
é um resultado pior do que uma chave de lock mantida pelo seu TTL. Um
handler que genuinamente falhou ainda reporta sua falha - suprimir o
erro de liberação não suprime o do handler.

### O contrato de liberação sem gastar tentativa

Middleware retorna um `JobOutcome` em vez de `Result<()>`. Quatro
variantes:

- `JobOutcome::Completed` - o handler executou, ack.
- `JobOutcome::Released { delay }` - reenfileira depois de `delay` **sem** incrementar `attempts`. Usado por `WithoutOverlapping`, `RateLimited`. O worker entrega a operação inteira para `QueueDriver::release`, e todo driver in-tree reenfileira sua própria cópia armazenada no lugar, então a mensagem nunca está simultaneamente reservada e visível, e nunca nenhuma das duas. A contagem de tentativas é preservada sem nenhuma aritmética no worker para um driver discordar - a cópia armazenada nunca foi incrementada para essa execução.
- `JobOutcome::Failed { reason }` - vai para dead-letter agora, persiste no store de jobs falhados, não tenta de novo.
- `JobOutcome::Deleted` - descarta a reserva sem dead-letter. Usado por `Skip`. Se o job pertencia a um batch, o `pending_jobs` do batch decrementa mesmo assim para que callbacks possam disparar.

Esse contrato é o que faz "throttled porque o bucket estava cheio"
parecer diferente de "falhou porque o handler deu erro" na
contabilidade de retry, nas métricas, e nos eventos de ciclo de vida.

### O que conta como uma tentativa

Duas formas de um job deixar um worker sem terminar, e as duas
consomem uma tentativa:

- **O handler falhou** - retornou `Err`, ou sofreu panic até o limite do framework. O worker faz nack; o driver reenfileira com `attempts + 1`.
- **O worker morreu** - OOM kill, `abort()`, um segfault, `docker kill`, ou o SIGKILL que um supervisor envia quando uma parada dá timeout. Nada liquida nada; a reserva simplesmente expira. Qualquer worker que reclamar o job cobra a tentativa naquele momento.

O segundo caso costumava ser de graça, e isso era um buraco em vez de
uma gentileza: um job que confiavelmente mata seu worker nunca
conseguiria esgotar `max_tries` e por isso nunca poderia ser
dead-lettered. Ele mataria cada worker que o reivindicasse, voltaria
byte-idêntico, e mataria o próximo, por tanto tempo quanto algo
continuasse reiniciando workers.

Os três drivers in-tree cobram a tentativa, porque troca de
`QUEUE_DRIVER` não deve mudar se um job envenenado pode ser parado.
`database` detecta um `reserved_until` expirado; `memory` cobra a
tentativa quando o reaper move a reserva de volta para visível;
`redis` lê a contagem de entregas da entrada a partir de `XPENDING`,
já que uma entrada de Redis stream é imutável e seu próprio contador é
o único registro.

`JobOutcome::Released` é a exceção deliberada - veja o contrato acima.
Um job throttled por `RateLimited` nunca executou, então não deve
nada.

**No Redis, o reclaim tem dois relógios.** `--visibility-timeout`
define por quanto tempo uma entrada precisa ficar sem ack antes de se
qualificar para reclaim; um segundo intervalo governa com que
frequência um consumer olha. O driver liga o segundo ao primeiro,
então um job perdido volta dentro de aproximadamente o dobro do
timeout configurado, em vez do timeout mais 30 segundos fixos.

**O orçamento é verificado antes do handler executar, não só na
liquidação.** Toda outra decisão de dead-letter acontece depois de um
handler retornar, o que assume que o handler retorna. Um job que mata
seu worker não consegue alcançar essa verificação, então o worker
também se recusa a despachar um job cujas tentativas já foram gastas -
ele o manda para dead-letter em vez disso, antes que ele derrube outro worker. Sem
isso, contar a tentativa só faria um número subir enquanto o job
continuava ciclando.

**O que isso significa para você.** `attempts` conta *entregas a um
worker*, não *falhas de handler*. Um worker perdido por razões não
relacionadas ao job - um reboot de host, um OOM causado por um vizinho
ruidoso - também gasta uma tentativa do orçamento daquele job. O
Laravel se comporta da mesma forma. Dimensione `max_tries` com isso em
mente, e prefira handlers idempotentes: entrega pelo menos uma vez
sempre foi o contrato, e isso faz o caminho de redelivery contar
honestamente em vez de silenciosamente.

## Eventos de ciclo de vida

Workers emitem eventos de ciclo de vida no formato Laravel através da
facade [`Event`](events.md). Listeners recebem a identidade do
envelope (`id`, `job_name`, `attempts`, `max_tries`, `connection`),
não a instância tipada do job - o worker é type-erased sobre payloads
JSON. Erros viajam como uma `String` já que `FrameworkError` não
deriva `Clone`.

| Evento | Dispara quando |
| --- | --- |
| `JobQueueing` | antes do envelope chegar ao driver |
| `JobQueued` | depois do driver aceitar |
| `UniqueJobSkipped` | `push_unique` suprimiu uma duplicata dentro da janela `unique_for` |
| `JobDebounced` | o worker descartou um envelope que um dispatch com debounce mais novo substituiu |
| `JobProcessing` | worker fez pop, prestes a despachar |
| `JobProcessed` | handler retornou `Ok` |
| `JobAttempted` | toda liquidação terminal (sucesso, falha, timeout) |
| `JobExceptionOccurred` | handler retornou `Err`, vai tentar de novo |
| `JobReleasedAfterException` | reenfileiramento de retry-after-error aconteceu |
| `JobReleased` | liberação conduzida por middleware (sem falha) |
| `JobFailed` | dead-lettered |
| `JobTimedOut` | timeout por tentativa excedido |
| `Looping` | toda iteração do loop (antes do pop) |
| `WorkerStarting` / `WorkerStopping` | uma vez por tempo de vida do worker |
| `WorkerInterrupted` | sinal de `Queue::restart()` observado |
| `QueuePaused` | `Queue::pause` definiu a própria chave de uma fila |
| `QueueResumed` | `Queue::resume` limpou a própria chave de uma fila |
| `QueuesPaused` | `Queue::pause_all` definiu a chave global |
| `QueuesResumed` | `Queue::resume_all` limpou a chave global |
| `WorkerQueuePaused` | um worker em execução observou pela primeira vez uma fila como pausada |
| `WorkerQueueResumed` | um worker em execução viu uma fila pausada voltar a ser reivindicável |

Inscreva-se com a API normal `Event::listen`. Eventos são best-effort -
`Event::dispatch` sem listeners é um no-op `Ok(())`, então workers em
deployments sem `Event::init()` não pagam nada.

`UniqueJobSkipped` é o único evento que dispara no lado do *push* em vez do
lado do worker, e o único que informa uma não falha. Ele carrega `job_name`,
`unique_id` e `connection` - a decisão de dedupe acontece antes de existir um
envelope, portanto não há ID de envelope para informar. O push ainda retorna
`Ok(false)`; o evento é o que torna observável uma supressão que de outro modo
seria invisível.

`QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` disparam da
mesma forma - de `Queue::pause` / `resume` / `pause_all` / `resume_all`
propriamente ditos, não do loop do worker. Eles também não carregam identidade
de envelope; veja "Pausando filas" abaixo para o contrato completo.

`WorkerQueuePaused` / `WorkerQueueResumed` são o par do lado do worker, e são
eles que te dizem *por que um worker em particular ficou quieto*. Eles disparam
uma vez por transição, de dentro do loop do worker, carregam a conexão que o
worker está drenando e carregam o nome da fila - ou `None`, quando um worker
sem filtro está ocioso sob uma pausa global e não tem nomes de fila para
reportar.

## Armazenamento de jobs falhados

Jobs dead-lettered caem no `FailedJobStore` configurado:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryFailedJobStore};

Queue::set_failed_store(Arc::new(MemoryFailedJobStore::new()));

// Em ferramental de admin:
let store = Queue::failed_store().unwrap();
for record in store.all().await? {
    println!("{} failed: {}", record.job_name, record.exception);
}
store.forget(some_id).await?;
store.flush(None).await?;
```

Três backends:

- `MemoryFailedJobStore` - `Vec` in-process, perdido no restart.
- `DatabaseFailedJobStore` - persiste em uma tabela `failed_jobs` via SeaORM.
- `NullFailedJobStore` - descarta todo registro. Espelha o `NullFailedJobProvider` do Laravel.

### Quando o store rejeita um registro

Se o store configurado retorna um erro, o worker loga em `error` e
**deixa a reserva intacta** em vez de dar ack. O job retorna na
expiração da visibilidade e é tentado de novo - ele não é
silenciosamente descartado.

Isso é deliberado. A alternativa, dar ack de qualquer forma, descarta
um job que já esgotou suas tentativas *e* falhou em ser registrado em
qualquer lugar, o que é irrecuperável. Um job que continua voltando é
recuperável: corrija o store e a próxima entrega chega.

O caso prático é um `DatabaseFailedJobStore` apontando para uma tabela
`failed_jobs` sem migration. Até você migrar, jobs em dead-letter
ciclam a uma redelivery por timeout de visibilidade, cada uma logando
o erro do store. Se você genuinamente quer falhas descartadas,
configure `NullFailedJobStore` - isso tem sucesso, então o job dá ack
e desaparece.

### Tentando de novo

```rust
use uuid::Uuid;

// Registro único - false se o id não estava no store.
Queue::retry_failed(some_id).await?;

// Bulk - cutoff opcional (só tenta de novo registros mais antigos que `before`).
let count = Queue::retry_all_failed(None).await?;
```

`retry_failed` carrega o envelope, reseta `attempts`, `available_at`,
e a `idempotency_key`, faz push através do driver configurado, então
deleta o registro de job falhado. Espelha
`php artisan queue:retry <id>` mais a semântica de `queue:flush` (cada
envelope tentado de novo é enviado E removido do store).

### `failed_jobs` schema

`DatabaseFailedJobStore` espera essa tabela (gerenciada pelas suas
migrations):

```sql
CREATE TABLE failed_jobs (
    id              TEXT PRIMARY KEY,
    connection      TEXT NOT NULL,
    queue           TEXT NOT NULL,
    job_name        TEXT NOT NULL,
    envelope_json   TEXT NOT NULL,
    exception       TEXT NOT NULL,
    failed_at       BIGINT NOT NULL
);
CREATE INDEX idx_failed_jobs_failed_at ON failed_jobs(failed_at);
```

O argumento `table` de `DatabaseFailedJobStore::new` é validado como
um identificador SQL na construção.

## Batches enfileirados

Despache um grupo de jobs com rastreamento de progresso e callbacks de
conclusão:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryBatchRepository, batch::register_callback};

Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));

// Registre callbacks nomeados no boot.
register_callback(Arc::new(SendSummary));
register_callback(Arc::new(PageOnFail));

let id = Queue::batch()
    .name("import-users")
    .add(ImportUser { id: 1 })
    .add(ImportUser { id: 2 })
    .add(ImportUser { id: 3 })
    .then("send-summary-email")
    .catch("page-on-fail")
    .finally("cleanup-temp-tables")
    .dispatch()
    .await?;

// Inspecione o progresso depois:
let repo = Queue::batch_repository().unwrap();
let snap = repo.find(&id).await?.unwrap();
println!("{}/{} jobs done ({}%)", snap.processed_jobs(), snap.total_jobs, snap.progress());
```

Cada worker liquida seu job contra o batch, e quando `pending_jobs`
chega a zero o worker dispara os callbacks `then`/`catch`/`finally`
registrados. Por padrão a primeira falha cancela o batch;
`.allow_failures()` mantém os jobs restantes continuando.

### Batches duráveis

`MemoryBatchRepository` é perdido no restart, o que deixa todo batch
em voo encalhado: seus contadores se vão, `pending_jobs` nunca mais
consegue chegar a zero, e os callbacks nunca disparam. Use
`DatabaseBatchRepository` em produção:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, DatabaseBatchRepository};

Queue::set_batch_repository(Arc::new(DatabaseBatchRepository::new(db.clone())));
```

Duas tabelas, que o framework não cria - adicione-as às suas
migrations, do mesmo jeito que `jobs` e `failed_jobs` funcionam:

```sql
CREATE TABLE job_batches (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    total_jobs    INTEGER NOT NULL,
    options_json  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    cancelled_at  INTEGER NULL,
    finished_at   INTEGER NULL
);

CREATE TABLE job_batch_settlements (
    batch_id   TEXT NOT NULL,
    job_id     TEXT NOT NULL,
    failed     INTEGER NOT NULL,
    settled_at INTEGER NOT NULL,
    PRIMARY KEY (batch_id, job_id)
);
```

`DatabaseBatchRepository::with_tables(db, batches, settlements)` deixa
você nomeá-las; os dois nomes são validados como identificadores SQL
na construção.

Note o que `pending_jobs` e `failed_jobs` **não** são: colunas. Eles
são derivados das linhas de settlement em toda leitura -

```text
pending_jobs = max(0, total_jobs - COUNT(settlements))
failed_jobs  = COUNT(settlements WHERE failed)
```
 -
porque filas são pelo menos uma vez, então o mesmo job liquida mais de
uma vez sempre que uma redelivery acontece, um ack é duplicado, ou um
worker morre entre fazer o trabalho e registrá-lo. Um contador
decrementado por liquidação desvia em cada um desses casos, e o desvio
não é cosmético: `pending_jobs` controla o disparo dos callbacks,
então um zero prematuro dispara `then` enquanto outros jobs do batch
ainda estão executando. Com as contagens derivadas e a primary key em
`(batch_id, job_id)`, uma liquidação repetida não insere nada e não há
contador para errar - entre processos, não só dentro de um.

### Quando um dispatch falha na metade

Se um `driver.push` falha no meio de `dispatch()`, os jobs que já
chegaram à fila são reais e já estampados com o id do batch. Então o
batch é liquidado em vez de removido: todo envelope que *não* foi
enviado é registrado como um job falhado, e o batch é cancelado.

`total_jobs` continua contando o que você pediu, `failed_job_ids`
nomeia exatamente os jobs que nunca chegaram, os que já foram
enfileirados liquidam normalmente, e `SkipIfBatchCancelled` descarta o
resto - então `pending_jobs` ainda chega a zero e seus callbacks
`catch`/`finally` ainda executam. Se nada foi enviado de forma alguma,
`dispatch` os dispara ele mesmo, porque não sobrou nenhum worker para
fazer isso. Você recebe o erro de push original de volta de qualquer
jeito.

### Opções de batch

| Opção | Método do builder | Efeito |
| --- | --- | --- |
| Permitir falhas | `.allow_failures()` | continua agendando depois de um job falhar |
| Callback then | `.then(name)` | executa quando todo job tem sucesso |
| Callback catch | `.catch(name)` | executa na primeira falha |
| Callback finally | `.finally(name)` | executa depois que o batch liquida de qualquer jeito |
| Pular cancelados | middleware `SkipIfBatchCancelled` no job | descarta os jobs restantes quando o batch é cancelado |

### `BatchCallback` impl

```rust
use async_trait::async_trait;
use suprnova::queue::{Batch, BatchCallback};
use suprnova::error::FrameworkError;

pub struct SendSummary;

#[async_trait]
impl BatchCallback for SendSummary {
    fn name(&self) -> &'static str { "send-summary-email" }

    async fn handle(&self, batch: Batch, error: Option<String>) -> Result<(), FrameworkError> {
        let subject = match error {
            Some(_) => format!("Batch {} failed", batch.name),
            None    => format!("Batch {} done - {} jobs", batch.name, batch.total_jobs),
        };
        // … envie o mail
        Ok(())
    }
}
```

Registre no boot com
`batch::register_callback(Arc::new(SendSummary))`. Callbacks são
indexados por `name()` - as opções do batch armazenam nomes de
callback, então um restart de processo recupera callbacks registrados
por lookup em vez de tentar desserializar uma closure (closures do
Rust não serializam).

## Chains enfileiradas

Fluxos sequenciais em que cada elo só executa depois que o handler do
anterior dá ack:

```rust
Queue::chain()
    .add(GenerateReport { id: 99 })?
    .add(UploadToBucket { id: 99 })?
    .add(NotifyOwner { id: 99 })?
    .dispatch()
    .await?;
```

O primeiro envelope é enviado imediatamente; o resto viaja no campo de
payload `chain_remaining` dele. Em toda liquidação bem-sucedida o
worker faz pop da próxima entrada e a despacha. Uma falha quebra a
chain - elos subsequentes nunca são enfileirados.

### Liquidação terminal

Terminar um job encadeado significa duas coisas: enfileirar o
sucessor, e liberar o job recém-terminado. Como duas operações
separadas não há uma ordem segura. Ack primeiro, e um crash no
intervalo perde o resto da chain permanentemente - nada fica na fila
para retomar. Push primeiro, e o mesmo crash reentrega o job
terminado, então seu handler executa de novo e o sucessor é
enfileirado duas vezes.

Então o worker entrega os dois ao driver de uma vez, via
`QueueDriver::settle(token, follow_ups)`:

| Resultado | Significado |
| --- | --- |
| `Settled::Atomically` | sucessor enfileirado e reserva descartada em uma única transação |
| `Settled::Stale` | a reserva foi reclamada por outro consumer; **nada** foi enfileirado ou descartado |
| `Settled::Unsupported` | esse driver não consegue liquidar de forma transacional |

`DatabaseQueueDriver` implementa isso: os dois efeitos são uma
transação, e o `DELETE` indexado pela reserva funciona também como uma
fence. Se seu timeout de visibilidade expirou enquanto o handler
estava executando e outro worker pegou o job, o delete não corresponde
a nada, a transação faz rollback, e você recebe `Stale` - sem ter
enfileirado nada. Liquidação em duas etapas não consegue expressar
isso de forma alguma: seu push tem sucesso, o push do novo dono tem
sucesso, e a chain se bifurca.

Redis e o driver em memória respondem `Unsupported` e mantêm a ordem
push-antes-do-ack, o que troca perda permanente por uma duplicata
pelo-menos-uma-vez. Esse é o contrato documentado do framework, e é
por isso que ids de envelope encadeados são derivados do seu
predecessor em vez de aleatórios - uma etapa reentregue reenvia o id
que enviou antes, então a duplicata é reconhecível como a mesma etapa
lógica.

Se você escrever um driver cuja escrita de follow-up e confirmação
compartilham um domínio de transação, implemente `settle`. Seu padrão
retorna `Unsupported`, então drivers escritos antes disso existir
continuam funcionando sem alteração.

## Introspecção

```rust
Queue::size().await?;            // total
Queue::pending_size().await?;    // available_at <= agora, não reservados
Queue::delayed_size().await?;    // available_at > agora
Queue::reserved_size().await?;   // com pop feito, ainda sem ack
Queue::clear().await?;           // descarta todo envelope, retorna a contagem
Queue::driver_name()?;           // nome do driver configurado para logs / admin
```

A trait `QueueDriver` declara padrões para `size` / `pending_size` /
`reserved_size` / `delayed_size` / `clear`; `MemoryQueueDriver`,
`DatabaseQueueDriver` e `RedisQueueDriver` implementam todos eles
nativamente.

### Inspecionando filas

As contagens dizem quanto está enfileirado; às vezes você precisa ver os
envelopes de verdade - um painel administrativo, uma sessão de depuração,
uma pergunta do tipo "o que exatamente está travado". O
`Queue::pending_jobs` / `delayed_jobs` / `reserved_jobs` devolvem a mesma
informação que os contadores de tamanho contam, como uma listagem de DTOs
`InspectedJob`:

```rust
use suprnova::queue::{InspectedJob, Queue};

let pending: Vec<InspectedJob> = Queue::pending_jobs(None).await?;
let billing_only: Vec<InspectedJob> = Queue::pending_jobs(Some("billing")).await?;
let delayed = Queue::delayed_jobs(None).await?;
let reserved = Queue::reserved_jobs(None).await?;

for job in &pending {
    println!(
        "{} attempts={} queue={:?} payload={}",
        job.name, job.attempts, job.queue, job.payload
    );
}
```

O `InspectedJob` carrega `id`, `queue`, `name`, `attempts`, `payload` e
`created_at`. `id` e `created_at` são `Option`: as listagens do driver de
banco de dados ainda reportam uma linha cujo `envelope_json` não pôde ser
decodificado - como `id: None` e `payload: {"unparseable": true}` - em vez
de descartá-la e esconder um job envenenado de quem está olhando; a
projeção do `Queue::fake()` nunca registra um timestamp de dispatch
separado do `available_at`, então `created_at` é sempre `None` ali.

No driver de memória, o `delayed_size()` lê o comprimento do armazenamento
de atrasados diretamente, enquanto `delayed_jobs()` e `pending_jobs()`
primeiro promovem qualquer entrada cujo `available_at` já tenha passado. Na
janela estreita entre um job ficar devido e o próximo tick de 50ms do
coletor em background, o `delayed_size()` ainda pode contar um job que o
`delayed_jobs()` já promoveu para o `pending_jobs()` - as listagens são a
visão mais atual; uma divergência ali é esperada, não um defeito.

Uma reserva cujo timeout de visibilidade venceu continua aparecendo em
`reserved_jobs()` até um `pop` ou o coletor em background reivindicá-la de
volta. Só esses dois reivindicam, e reivindicar é o que gasta uma
tentativa, então uma chamada de listagem nunca muda a contagem de
tentativas de um job, por mais vezes que você a chame.

#### Por que Suprnova diverge

- **Um método com `Option<&str>`, não um par por listagem.** O Laravel traz
  `pendingJobs($queue)` ao lado de um `allPendingJobs()` separado; aqui
  `queue: None` colapsa os dois em uma chamada. Mesmo formato para
  `delayedJobs`/`allDelayedJobs` e `reservedJobs`/`allReservedJobs`.
- **O padrão da trait é um `Err` honesto, não uma coleção vazia.** Os
  drivers de Beanstalkd e SQS do Laravel retornam `[]` desses métodos mesmo
  para uma fila que claramente tem jobs - uma mentira por omissão que um
  autor de driver de terceiros poderia copiar sem perceber. Um driver do
  Suprnova que não implementou a inspeção diz isso; `sync` e `null`
  sobrescrevem com `Ok(vec![])` porque para eles "nunca há nada a listar" é
  a verdade literal, não um método não implementado.
- **O `reserved_jobs` do Redis é por consumidor.** O driver só conhece as
  reservas que ele mesmo entregou em processo; as entradas em voo de outro
  consumidor só ficam visíveis pelo próprio `XPENDING` do Redis, não por
  esta chamada.
- **O `pending_jobs` do Redis quer dizer "nunca entregue a nenhum consumidor
  deste grupo".** Ele varre `XRANGE (<last-delivered-id> +` - tudo além do
  cursor de entrega do grupo (`XINFO GROUPS`) - em vez do stream inteiro,
  porque o `ack` só faz `XACK` de uma entrada (este driver nunca faz
  `XDEL`/`XTRIM` no stream), então uma varredura que apenas excluísse as
  reservas em memória de um consumidor reportaria todo job com ack como
  pendente para sempre. Um job liberado ou com nack é republicado sob um id
  novo acima do cursor, então ele reaparece assim que a sua nova tentativa
  fica viva. Mesmo registro de "limite superior" do `pending_size`: o cursor
  é lido uma vez, então um `pop` concorrente pode reivindicar uma entrada
  entre essa leitura e a varredura. Na prática, a task de leitura antecipada
  em background de um consumidor rodando tende a reivindicar uma entrada
  recém-enviada em milissegundos após o push, bem antes de uma aplicação
  chegar a chamar `pop` - então o `pending_jobs` reflete sobretudo trabalho
  enviado enquanto nenhum consumidor daquele stream está ativamente fazendo
  polling, e não "qualquer envelope em que ninguém deu pop explicitamente
  ainda".

## Sinal de restart do worker

`php artisan queue:restart` se traduz para:

```rust
Queue::restart().await?;
```

O sinal vive no `Cache` como um timestamp em milissegundos. Workers
fazem poll uma vez por loop e terminam de forma limpa quando o
timestamp é mais novo que o horário em que começaram. Combine com um
supervisor (systemd, Kubernetes, o módulo `supervisor`) para que um
worker novo continue de onde o anterior parou.

## Pausando filas

`php artisan queue:pause` / `queue:resume` se traduzem para:

```rust
Queue::pause(&connection, "billing").await?;
Queue::resume(&connection, "billing").await?;
Queue::pause_all().await?;
Queue::resume_all().await?;
```

ou pela CLI:

```bash
./app queue:pause billing
./app queue:pause --all
./app queue:resume billing
./app queue:resume --all      # alias: queue:continue
```

Um worker pausado conclui o que já extraiu - pausar nunca interrompe um job em
andamento - e então para de reivindicar novo trabalho até ser retomado.
`pause_all` / `resume_all` são a chave global; pausar (ou retomar) uma fila
nomeada afeta somente essa fila. **`resume_all` não limpa uma pausa por fila** -
uma fila pausada individualmente permanece pausada após uma retomada global,
como no Laravel. Limpe-a explicitamente com
`Queue::resume(&connection, "billing")`.

Um worker pausado também avisa. O `queue:work` imprime uma linha por
transição:

```text
  2026-08-25 14:03:11 Queue billing PAUSED
  2026-08-25 14:07:44 Queue billing RESUMED
```

Um worker iniciado sem `--queue` não tem nomes de fila para reportar, então uma
pausa global imprime `All queues PAUSED` no lugar. As duas linhas vêm dos
eventos `WorkerQueuePaused` / `WorkerQueueResumed`, então você pode escutá-los
por conta própria e roteá-los para onde quer que more o seu alerting.

Ambos os sinais vivem no `Cache`, junto ao sinal de reinício acima:

| Chave | Significado |
| --- | --- |
| `suprnova:queues:paused` | chave global, definida por `pause_all` |
| `suprnova:queue:paused:{connection}:{queue}` | chave de uma fila, definida por `pause` |

Consulte o estado com `Queue::is_paused(&connection, "billing").await?` (true
se qualquer chave estiver definida) ou
`Queue::paused_queues(&connection, &queues).await?` (quais de `queues` estão
pausadas no momento).

### A pausa por fila precisa de um `--queue` nomeado

Um worker iniciado com `--queue=billing,exports` só reivindica dessas duas
filas, de modo que pausar `billing` estreita essa lista a `exports` enquanto a
pausa durar. Um worker iniciado sem nenhum `--queue` drena todas as filas que o
driver mantém, e não há como pedir \"pause apenas `billing`\" contra isso -
`QueueDriver::pop_from` nunca informa quais nomes de fila existem, portanto não
há nada contra o qual conferir uma chave de pausa por fila. `pause_all` ainda
para completamente um worker não filtrado; uma pausa nomeada por fila só tem
efeito quando você também nomeia as filas desse worker.

### Desabilitando a sondagem de pausa

Defina `QUEUE_PAUSABLE=false` e todo worker desse processo ignora sinais de
pausa integralmente, sem custo extra de leitura de cache por loop.
`queue:pause` (mas não `queue:resume`) também se recusa a rodar e sai com
valor diferente de zero, portanto um operador que desabilitou a pausa descobre
isso imediatamente em vez de emitir uma pausa que silenciosamente não faz nada.
Espelha `Worker::$pausable` do Laravel.

### Por que Suprnova diverge

Um cache inacessível falha **aberto**: um worker que não consegue ler as chaves
de pausa se comporta como \"não pausado\" e continua drenando - o mesmo contrato
de falha aberta que o sinal de reinício de worker acima já usa. Uma interrupção
transitória de cache deve degradar uma frota de workers para \"ignorar pausa\",
nunca para \"todo worker congela silenciosamente\" - o estado de pausa é um
sinal explícito de adesão, e sua própria indisponibilidade não deve se tornar
uma chave de desligamento oculta.

## Shutdown gracioso

O `CancellationToken` do worker dispara no próximo limite de pop,
nunca no meio de um dispatch. Um handler que já foi popped executa até
a conclusão (limitado pelo seu próprio `Job::timeout()`, se definido)
antes do worker terminar. Isso significa que efeitos colaterais em voo
não são interrompidos no meio do caminho, mas um SIGTERM pode levar
até o timeout por job para drenar. Defina `WorkerConfig::max_jobs`
para uma estratégia de restart periódico em workers de longa duração;
o worker termina de forma limpa depois de tantas liquidações,
independente do resultado.

## Métricas de liquidação

O worker emite um contador `queue.settlement.failures` via
[`Metrics`](observability.md) em toda falha de ack/nack. Atributos:
`operation` (`"ack"` | `"nack"`), `driver` (o nome do driver
configurado), `job` (o job_name), `outcome` (`"success"`,
`"dead_letter"`, `"retry"`, `"deleted"`, `"timeout_dead_letter"`,
`"timeout_retry"`, `"released"`).

Uma taxa diferente de zero aqui significa que a entrega pelo menos uma
vez pode reentregar um efeito colateral que já teve sucesso ou perder
a contabilidade de tentativas - alerte sobre isso explicitamente.

## Erros tipados

`MaxAttemptsExceeded`, `TimeoutExceeded`, e `ManuallyFailed` espelham
`MaxAttemptsExceededException` / `TimeoutExceededException` /
`ManuallyFailedException` do Laravel. O worker anexa a causa relevante
ao evento `JobFailed` de dead-letter para que listeners possam fazer
pattern-match em vez de buscar substring na mensagem de erro.

## Nomeação de conexão

Workers marcam todo evento de ciclo de vida com um nome de conexão.
Por padrão esse é o `name()` do driver (ex.: `"memory"`, `"redis"`,
`"database"`). Apps que executam múltiplas conexões ao mesmo tempo
podem sobrescrever:

```rust
Queue::set_connection_name("orders-redis");
```

## Testes

A semântica de `Queue::fake()` vive em `queue::testing`:

```rust
let _guard = suprnova::queue::testing::install_fake();
my_code_that_dispatches_jobs().await;

suprnova::queue::testing::assert_pushed::<SendWelcomeEmail>(|j| j.user_id == 42);

// Para dispatches atrasados, fixe o timestamp agendado:
suprnova::queue::testing::assert_pushed_later::<SendWelcomeEmail>(|j, at| {
    j.user_id == 42 && at > chrono::Utc::now()
});
```

A guarda fake serializa testes paralelos por um mutex de todo o processo; ela
captura `(payload, available_at, overrides)` por push e limpa no `Drop`. O
campo `overrides` é `EnvelopeOverrides::default()` para todo ponto de entrada
exceto `push_with`/`later_with` - veja
[Mocking](mocking.md#queue---queuetestinginstall_fake) para
`assert_pushed_on_queue`/`assert_pushed_on_connection` e
`pushed_with_overrides`, as assertions sobre ele. No modo fake,
`push_unique` sempre registra o push como novo - dedupe é irrelevante quando
nenhum driver está vinculado.

Um push com debounce se comporta do mesmo jeito: o fake não escreve nada no
cache, então nenhuma janela é armada e o `available_at` registrado não carrega
atraso de debounce. O `assert_pushed_later` o enxerga como sem atraso. O que o
fake ainda pega é um job declarando `debounce_for` e `unique_id` ao mesmo
tempo - esse par não pode valer, seja qual for o ambiente, então o push
retorna um erro sob `Queue::fake()` exatamente como retornaria em produção.

## Idempotência é o contrato do worker com você

Drivers de fila apoiados em Redis não conseguem tornar `nack` atômico -
`XADD` e `XACK` são comandos separados. Um crash entre eles reentrega
a mensagem via `XAUTOCLAIM`. Drivers em memória e de banco de dados
são exatamente-uma-vez-por-tentativa, mas o loop do worker não
distingue drivers, então **todo handler de job em um deployment de
produção precisa ser idempotente**.

Para jobs típicos no estilo comando, envolva o corpo do handler em
[`Idempotency::once`](idempotency.md) ou
[`Idempotency::commit_on_success`](idempotency.md) indexado por uma
chave estável por operação (id de entidade, request id fornecido pelo
chamador, etc.). Quando uma nova tentativa precisa retornar o
resultado *original* em vez de pular a reexecução, use
`Idempotency::remember`, que registra o valor de sucesso e o reproduz
em entregas posteriores.

## Próximos passos

- [Barramento](bus.md) - dispatcher síncrono com resultados tipados
- [Eventos](events.md) - fan-out pub/sub
- [Idempotência](idempotency.md) - o contrato que handlers honram para entrega pelo menos uma vez
- [Cache](cache.md) - apoia `push_unique`, `WithoutOverlapping`, `RateLimited`
- [Mocking](mocking.md) - toda guarda fake, incluindo `Queue::fake`

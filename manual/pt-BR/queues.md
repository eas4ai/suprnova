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

Cinco drivers vêm in-tree. Configure via env `QUEUE_DRIVER` ou
chamando `Queue::set_driver(...)` programaticamente.

| Driver | Use para | Pontos fortes |
| --- | --- | --- |
| `MemoryQueueDriver` | testes, apps de processo único | `tokio::time::DelayQueue` para `available_at`, compatível com virtual-clock |
| `RedisQueueDriver` | fan-out de produção | consumer groups + `XAUTOCLAIM` + jobs atrasados apoiados em ZSET |
| `DatabaseQueueDriver` | apps de banco de dados único | `FOR UPDATE SKIP LOCKED` em Postgres/MySQL, serializado com `BEGIN` no SQLite |
| `SyncQueueDriver` | dev, CI | executa o handler inline em `push`, sem worker |
| `NullQueueDriver` | wrappers de teste | descarta todo push sem executar |

`Queue::bootstrap_from_env()` lê `QUEUE_DRIVER` e conecta o driver
correspondente; `Queue::bootstrap_default()` sempre conecta o driver
de memória. O caminho de boot do servidor chama um desses para você -
a maioria dos apps só configura via env.

### Configuração de ambiente

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# Driver de banco de dados - DB::init() precisa executar primeiro
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

O driver de banco de dados valida `QUEUE_DB_TABLE` como um
identificador SQL na construção, então um valor de env malformado
falha o boot em vez de chegar à composição de SQL. O Redis usa
sea-streamer-redis por baixo dos panos com `AutoCommit::Disabled`; o
timeout de visibilidade é fixado no momento de construção do
consumer-group, então o argumento `visibility_timeout` por pop é
ignorado no Redis (uma divergência documentada do contrato do trait
imposta pelo Redis Streams).

### Por que Suprnova diverge

O Laravel roteia todo queueable através do Bus, distinguindo jobs
`ShouldQueue` no momento do dispatch. O Suprnova separa os dois: `Bus`
para trabalho síncrono que retorna um resultado tipado, `Queue` para
trabalho assíncrono que sobrevive a um crash de processo. O PHP
precisa do roteamento implícito porque seu modelo
processo-por-solicitação torna difícil modelar "faça isso depois, em
outro processo" de outra forma. O Tokio não precisa - `Bus::dispatch`
vs `Queue::push` explícitos são mais claros, mais rápidos, e expõem a
escolha de durabilidade no call site. Veja [`bus.md`](bus.md) para a
comparação lado a lado.

## Variantes de push

Toda variante de push recebe um valor tipado `J: Job` e retorna quando o
envelope é confirmado no driver - não quando o handler roda.

| Método | Comportamento |
| --- | --- |
| `Queue::push(job)` | enfileira imediatamente |
| `Queue::push_later(job, at)` | disponível em um `DateTime<Utc>` específico |
| `Queue::later(delay, job)` | disponível depois de `delay` a partir de agora |
| `Queue::push_with(job, overrides)` | enfileira imediatamente com `EnvelopeOverrides` por push |
| `Queue::later_with(delay, job, overrides)` | disponível após `delay` a partir de agora, com `EnvelopeOverrides` por push |
| `Queue::push_unique(job)` | deduplica por `J::unique_id` dentro de `J::unique_for`, retorna `Ok(true)` quando o envelope foi enviado, `Ok(false)` quando uma chave de dedupe viva o suprimiu |
| `Queue::push_unique_later(job, at)` | único + agendado |
| `Queue::later_unique(delay, job)` | único + com atraso |
| `Queue::bulk(vec![job1, job2, ...])` | envia todo job (o driver pode usar um caminho nativo em lote) |

`push_unique` exige que a camada de cache esteja inicializada - o lock de
dedupe vive em [`Cache`](cache.md) via
[`Idempotency::commit_on_success`](idempotency.md). Um push que falha
libera a chave de dedupe para que o chamador possa tentar de novo; um
push bem-sucedido a segura por `J::unique_for` segundos. O job precisa
sobrescrever `Job::unique_id(&self)` para retornar `Some(id)` - `None`
retorna um erro interno.

O booleano responde a uma pergunta - "este job está na fila?" - e há um
terceiro caso por trás dela. Se o lease do lock de dedupe é perdido
enquanto o push está em voo, o push ainda assim completa (a camada de
idempotência nunca cancela um corpo que pode já ter tido efeito) e você
ainda recebe `Ok(true)`, com um log em nível `warn` nomeando o job e sua
chave única. O job está enfileirado; o que não fica provado é que
ninguém mais enfileirou o mesmo concorrentemente. Seu handler já precisa
tolerar reentrega, então isso não pede tratamento extra - mas o log está
lá porque uma rajada deles significa que o cache que apoia seu lock de
dedupe está sofrendo.

### Substituições por push com `EnvelopeOverrides`

`Queue::push_with` e `Queue::later_with` recebem um `EnvelopeOverrides`
junto ao job, para o único despacho que precisa de comportamento de fila,
conexão, timeout ou repetição diferente dos padrões do próprio job:

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

// The delayed counterpart, mirroring `Queue::later`'s relationship to `Queue::push`.
Queue::later_with(Duration::from_secs(60), SendWelcomeEmail { user_id: 42 }, overrides).await?;
```

Cada campo tem como padrão `None` e delega para a resolução normal que
`Queue::push` já executa; um campo `Some` vence tudo isso para este único
push, superando tanto uma rota registrada com
[`Queue::route`](#roteamento-de-fila) quanto a própria declaração `Job::*`
do job para aquele campo:

| Campo | Supera |
| --- | --- |
| `queue` | `Queue::route`, `Job::queue()` |
| `connection` | `Queue::route`, `Job::connection()` |
| `timeout` | `Job::timeout()` |
| `fail_on_timeout` | `Job::fail_on_timeout()` |
| `max_tries` | `Job::max_tries()` |
| `backoff` | `Job::backoff()` |

`EnvelopeOverrides` é a primitiva sobre a qual `Mail::on_queue` /
`.on_connection()` e o ajuste de fila por notificação de `Notify::queue` são
construídos - veja [Correio](mail.md#enfileiramento) e
[Notificações](notifications.md).

### Atraso declarado pelo job

Um job pode carregar seu próprio atraso padrão em vez de cada ponto de chamada
repetir `Queue::later(Duration::from_secs(60), job)`:

```rust
impl Job for SendDigest {
    // ...
    fn delay() -> Option<Duration> { Some(Duration::from_secs(60)) }
}
```

`Queue::push(job)`, `Queue::push_with(job, overrides)`, `Queue::push_unique(job)`
e `Queue::bulk(vec![job1, job2])` todos o honram - `available_at` se torna
`now + J::delay()` em vez de `now`. `Queue::bulk` resolve o atraso uma vez
por chamada, pois cada job do vetor compartilha o mesmo `J` concreto e,
portanto, o mesmo `Job::delay()`.

Um atraso explícito no ponto de chamada sempre vence:
`Queue::push_later(job, at)`, `Queue::later(delay, job)`,
`Queue::later_with(delay, job, overrides)`,
`Queue::push_unique_later(job, at)` e `Queue::later_unique(delay, job)`
usam literalmente o timestamp ou atraso que quem chama passou -
`Job::delay()` não é consultado em nenhum deles. Use o método da trait quando
todo despacho de um tipo de job deve iniciar atrasado por padrão; use uma das
variantes `later`/`push_later` para um atraso de que um despacho específico
precisa, mas que o tipo não declara de outra forma.

Lotes e cadeias também não o consultam: `Queue::batch()...add(job)` e
`Queue::chain()...add(job)?` ambos constroem seus envelopes com `available_at`
definido para o momento em que você chamou `add`, de modo que um job com
`Job::delay()` declarado despacha imediatamente como parte de um lote ou uma
cadeia mesmo que um `Queue::push(job)` simples do mesmo job esperasse. Dê ao
job um atraso explícito de outra forma - um campo no próprio job, aplicado em
`handle()` - se uma etapa em lote ou encadeada precisar de um.

### Por que o Suprnova diverge

O `$job->delay` do Laravel é uma propriedade de instância, definida por
despacho (`SendDigest::dispatch($user)->delay(60)`), de modo que dois
despachos da mesma classe podem carregar atrasos diferentes. Aqui,
`Job::delay()` é um padrão de nível de classe, como `Job::queue()` ou
`Job::max_tries()` - um despacho que precisa de um atraso calculado de seus
próprios dados usa `Queue::later`/`push_later`, que já supera o padrão
declarado.

## Configuração de job

Sobrescreva as funções associadas de `Job` para ajustar o
comportamento por impl:

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

    fn delay() -> Option<Duration> { None }                // default: no delay
    fn max_tries() -> u32 { 5 }                            // padrão: 3
    fn timeout() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn fail_on_timeout() -> bool { false }                 // padrão: false (timeout tenta de novo)
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Sequence { secs: vec![5, 15, 60, 300] }
    }
    fn unique_id(&self) -> Option<String> {
        Some(format!("welcome:{}", self.user_id))
    }
    fn unique_for() -> Duration { Duration::from_secs(600) }  // padrão: 5 minutos
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
de envelope; veja \"Pausando filas\" abaixo para o contrato completo.

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
Queue::pending_size().await?;    // available_at <= now, não reservado
Queue::delayed_size().await?;    // available_at > now
Queue::reserved_size().await?;   // atualmente popped, ainda sem ack
Queue::clear().await?;           // descarta todo envelope, retorna a contagem
Queue::driver_name()?;           // nome do driver configurado para logs / admin
```

O trait `QueueDriver` declara padrões para `size` / `pending_size` /
`reserved_size` / `delayed_size` / `clear`; `MemoryQueueDriver` e
`DatabaseQueueDriver` os implementam nativamente. `RedisQueueDriver`
retorna um erro "unsupported" para `size` / `clear` - use o redis-cli
de admin para esses.

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

### Por que o Suprnova diverge

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

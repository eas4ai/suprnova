# Fluxos de trabalho

Fluxos de trabalho são funções async duráveis e de longa duração cujo
estado intermediário sobrevive a crashes, restarts, e panics. Recorra
a eles quando uma unidade de trabalho abrange múltiplas etapas - cada
uma potencialmente lenta, falível, ou com efeitos colaterais - e você
não pode se dar ao luxo de perder o progresso na metade do caminho. O
corpo de um fluxo de trabalho executa uma vez; o output de cada etapa
é persistido; uma nova tentativa retoma a partir da primeira etapa que
ainda não completou. Combine com [`Queue`](queues.md) quando o
trabalho é um job de execução única; combine com [`Bus`](bus.md)
quando o trabalho executa de forma síncrona na task da solicitação.

## Início rápido

Um fluxo de trabalho é uma função async que retorna
`Result<T, FrameworkError>`; seu corpo invoca uma ou mais funções
`#[workflow_step]`; você o enfileira através da macro
`start_workflow!` e um processo worker o drena.

```rust
use suprnova::{workflow, workflow_step, start_workflow, FrameworkError};

#[workflow_step]
async fn fetch_user(user_id: i64) -> Result<String, FrameworkError> {
    Ok(format!("user:{}", user_id))
}

#[workflow_step]
async fn send_welcome_email(user: String) -> Result<(), FrameworkError> {
    // … de fato envie o mail
    Ok(())
}

#[workflow]
async fn welcome_flow(user_id: i64) -> Result<(), FrameworkError> {
    let user = fetch_user(user_id).await?;
    send_welcome_email(user).await?;
    Ok(())
}

// A partir de um handler ou de qualquer contexto async:
let handle = start_workflow!(welcome_flow, 123).await?;
```

A macro serializa os argumentos para JSON, insere uma linha na tabela
`workflows`, e retorna um [`WorkflowHandle`](#esperando-pelos-resultados)
identificando a instância enfileirada. Um processo worker separado
pega a linha, executa o corpo, e persiste o output de cada etapa
conforme avança.

`#[workflow]` coleta a função no inventory de workflows sob seu
caminho totalmente qualificado (`module_path::fn_name`). Registros
duplicados sob o mesmo nome abortam o boot do worker via
`registry::assert_no_duplicates` - shadowing silencioso seria
impossível de depurar, então o framework falha de forma explícita.

## Schema

Fluxos de trabalho persistem em duas tabelas: `workflows` (uma linha
por instância) e `workflow_steps` (uma linha por invocação de etapa,
indexada por `(workflow_id, step_index)`). O framework é dono do
schema; você escolhe quando aplicá-lo.

Duas formas de conectar as migrations.

### Arquivos de migration gerados

A CLI faz scaffold de cópias das migrations do framework dentro do seu
app:

```bash
suprnova workflow:install
suprnova migrate
```

`workflow:install` escreve `m_create_workflows_table.rs` e
`m_create_workflow_steps_table.rs` dentro de `src/migrations/`, e
então os registra no seu `Migrator`. Use isso quando você quer o
schema versionado junto com as outras migrations do seu app.

### Registro programático

Alternativamente, registre diretamente as structs de migration que
pertencem ao framework:

```rust
use sea_orm_migration::MigratorTrait;
use suprnova::workflow::migrations::{
    CreateWorkflowsTable, CreateWorkflowStepsTable,
};

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![
            Box::new(CreateWorkflowsTable),
            Box::new(CreateWorkflowStepsTable),
        ]
    }
}
```

Os dois caminhos produzem SQL idêntico. A mesma convenção é usada por
[`features::migrations`](feature-flags.md) e
[`payments::migrations`](payments.md).

## Executando o worker

Em um app com scaffold, o worker é iniciado pelo subcomando
`workflow:work` do binário:

```bash
suprnova workflow:work
```

O worker executa a mesma inicialização que seu servidor HTTP executa,
então observers, listeners, e vinculações de contêiner registradas em
`bootstrap()` ficam visíveis para as etapas do fluxo de trabalho. Em
`SIGINT` / `SIGTERM` o worker para de puxar novas claims e espera por
todo fluxo de trabalho em voo antes de terminar - nenhum fluxo de
trabalho fica órfão no meio de uma etapa em um shutdown limpo.

O caminho de claim (`claim_next_workflow`) usa
`FOR UPDATE SKIP LOCKED` contra a tabela `workflows`, então o processo
worker **exige Postgres**. SQLite e MySQL funcionam para testes e para
o caminho de enqueue/persistência, mas o daemon worker vai terminar
com um erro no primeiro claim se a conexão não for Postgres.

## Configuração

Cinco variáveis de ambiente ajustam o worker. Valores fora da faixa
são limitados a mínimos seguros com um `tracing::warn!` para que um
typo no `.env` não consiga inutilizar o daemon.

| Variável | Padrão | Notas |
|---|---|---|
| `WORKFLOW_POLL_INTERVAL_MS` | `1000` | Sleep entre rounds de claim vazios |
| `WORKFLOW_CONCURRENCY` | `4` | Máximo de fluxos de trabalho executando por worker (mín. 1) |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30` | Duração do lease antes que outro worker possa reclamar |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | Orçamento de tentativas por fluxo de trabalho (mín. 1) |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | Backoff linear: `attempts * value` (mín. 0) |

Para configs programáticas (construídas em código em vez de parseadas
do env), chame `WorkflowConfig::validate()` para falhar rápido nos
mesmos invariantes antes de construir um `WorkflowWorker`.

## Recuperação de crash

Três camadas de proteção impedem que fluxos de trabalho fiquem
travados em falhas de worker.

**Limite de panic.** O corpo do fluxo de trabalho executa dentro de
`AssertUnwindSafe(...).catch_unwind()`. Um panic em qualquer etapa é
capturado, o payload é capturado na coluna de erro, e a linha passa
pela mesma contabilidade de retry/fail que um `Err` retornado. Sem o
limite, um panic pularia o caminho de liquidação e deixaria a linha em
`status='running'` para sempre.

**Heartbeat do lease.** Uma etapa de longa duração que sobrevive mais
que `WORKFLOW_LOCK_TIMEOUT_SECS` poderia, de outra forma, ter seu
lease expirado debaixo dos próprios pés. O worker spawna uma task de
heartbeat que atualiza `locked_until` na metade do intervalo de
lock-timeout até o corpo resolver. O heartbeat aborta no drop, então
um `?` retornado não consegue vazar uma task de renovação e congelar o
lease de um fluxo de trabalho que ninguém está executando.

**Reclaim de lease expirado.** Quando um worker morre sem nunca
liberar seu lock (hard kill, crash do host, OOM do kernel), a linha
permanece em `status='running'` até `locked_until` passar. A query de
claim recolhe explicitamente essas linhas: qualquer fluxo de trabalho
`running` cujo lease expirou se torna reivindicável por outro worker
na próxima rodada, com `attempts` incrementado. A recuperação de crash
é automática - não há nada para automatizar via script e nenhum
comando de admin para lembrar.

## Semântica de entrega - pelo menos uma vez

Corpos de etapa executam com semântica de **pelo menos uma vez**. Uma
etapa pode executar mais de uma vez em duas situações:

1. **Retornou `Err`** - o fluxo de trabalho é reenfileirado; na nova tentativa a etapa que falhou executa de novo, e qualquer etapa anterior é reproduzida a partir do cache.
2. **Crash depois do efeito colateral, antes de `mark_step_succeeded` fazer commit** - o lease expira, outro worker reclama, não vê nenhum output em cache naquele índice de etapa, e executa o corpo de novo.

O framework persiste os **outputs** de etapa de forma durável, mas não
consegue observar o efeito colateral em si. Tornar os corpos de etapa
idempotentes é sua responsabilidade. Dois padrões funcionam para quase
todo caso.

**Escritas condicionais.** Use `INSERT ... ON CONFLICT DO NOTHING`,
colunas de idempotency-key, ou marcadores `seen_event_id`. Derive uma
chave estável por etapa a partir de dados já em scope: os argumentos
de input do fluxo de trabalho mais uma tag de etapa literal
(`("wf-charge", customer_id)`) já bastam porque os mesmos argumentos
mapeiam para a mesma linha de fluxo de trabalho entre tentativas.

**Chaves de idempotência externas.** A maioria das APIs de terceiros
(Stripe, SES, SQS) aceita um header `Idempotency-Key`. Passe uma chave
derivada do input do fluxo de trabalho mais uma tag local à etapa
(`format!("wf-charge-{}", customer_id)`) para que solicitações
repetidas façam dedupe no provedor.

**Não** assuma que uma etapa que retornou `Ok` não pode executar uma
segunda vez - um crash pode fazer essa segunda execução cair em
qualquer worker subsequente, inclusive depois de um restart em um host
diferente. Veja o capítulo [Idempotência](idempotency.md) para
`Idempotency::once`, `Idempotency::commit_on_success`, e
`Idempotency::remember` - todos wrappers válidos em torno de um corpo
de etapa.

## Contrato de determinismo

Fluxos de trabalho precisam ser determinísticos entre replays. Cada
etapa é indexada por `(step_name, step_index)`, e o framework guarda
em cache seu input serializado junto com o output. Quando uma etapa no
mesmo índice é reproduzida com um input serializado diferente, o
framework retorna um erro em vez de mascarar a corrupção retornando o
output em cache do input anterior.

Na prática isso significa:

- Não ramifique com base em `Utc::now()`, `rand::random()`, ou outras fontes não-determinísticas fora de um `#[workflow_step]`. Corpos de etapa podem chamá-las livremente - o resultado é capturado no cache de output da etapa.
- Não insira etapas condicionalmente. Se uma nova tentativa encontra um número diferente de etapas antes de um dado índice, você recebe um erro de step-name mismatch. Coloque a lógica de ramificação dentro de uma etapa.
- Não mude o formato dos argumentos de uma etapa entre deploys sem renomear a etapa. Renomear muda `step_name`, o que reinicia o cache do zero para aquela etapa.

## Esperando pelos resultados

`WorkflowHandle` deixa o chamador fazer poll na linha, esperar ela
terminar, ou buscar o output serializado.

```rust
use std::time::Duration;
use suprnova::{FrameworkError, WorkflowStatus};

let handle = start_workflow!(welcome_flow, 123).await?;

match handle.wait_with_timeout(Duration::from_secs(30)).await {
    Ok(WorkflowStatus::Succeeded) => { /* concluído */ }
    Ok(WorkflowStatus::Failed) => { /* coluna de erro persistida */ }
    Ok(_) => unreachable!("wait_* only returns terminal status"),
    Err(FrameworkError::Internal { message }) if message.contains("Timed out") => {
        // O fluxo de trabalho ainda está executando; segue para a UX assíncrona.
    }
    Err(other) => return Err(other),
}
```

`wait()` faz poll indefinidamente - use apenas em testes ou scripts de
vida curta onde bloquear para sempre é aceitável. Para caminhos de
solicitação HTTP, `wait_with_timeout(Duration)` sempre vence contra o
loop de poll interno, mesmo que a query de status subjacente engasgue.
Um erro de timeout **não** cancela o fluxo de trabalho - o worker
continua, e `handle.status().await` retorna o estado ao vivo depois.

`wait_with_options(Some(poll), Some(deadline))` expõe os dois
controles quando os padrões não servem.

Para outputs tipados, defina um retorno
`T: Serialize + DeserializeOwned` no fluxo de trabalho e chame
`handle.output::<T>().await?`. O JSON bruto está disponível via
`output_raw()`.

## Cache de etapa, em detalhe

O cache de etapa é indexado por **nome da etapa + índice da etapa**. A
primeira invocação de uma etapa persiste seu JSON de input, executa o
corpo, e em caso de sucesso persiste o JSON de output. Um replay no
mesmo índice:

- Retorna o output em cache se a etapa está `succeeded` e o input reproduzido corresponde ao input em cache.
- Retorna um erro se o input difere (a salvaguarda de determinismo).
- Executa o corpo de novo se a etapa está `running` ou `failed` (sem output em cache para retornar).

Índices de etapa são atribuídos por um `AtomicI32` por contexto de
fluxo de trabalho, então a ordem é determinada pelas chamadas que o
corpo do seu fluxo de trabalho faz. Uma ramificação que produz uma
etapa diferente no mesmo índice em uma nova tentativa aparece como um
erro de step-name mismatch em vez de corromper silenciosamente etapas
posteriores.

Outputs e inputs são armazenados como JSON TEXT, então todos os tipos
de retorno e argumentos de etapa precisam ser
`Serialize + DeserializeOwned`.

## Detectando o contexto de fluxo de trabalho a partir de um helper

`WorkflowContext::is_active()` retorna se a task atual está executando
sob um fluxo de trabalho. Use isso a partir de helpers que precisam se
comportar diferente dentro vs fora do worker - por exemplo, um logger
que anexa a tag de fluxo de trabalho apenas quando ela existe:

```rust
use suprnova::workflow::WorkflowContext;

fn maybe_workflow_tagged(message: &str) -> String {
    if WorkflowContext::is_active() {
        format!("[workflow] {message}")
    } else {
        message.to_string()
    }
}
```

Fora de um fluxo de trabalho (chamada diretamente de um teste ou
handler), uma função `#[workflow_step]` ainda executa -
`WorkflowContext::current()` simplesmente retorna `None`, o corpo
executa sem persistência, e a etapa pula o cache completamente. Isso é
intencional: torna as funções de etapa testáveis individualmente sem
precisar levantar um worker.

### Por que Suprnova diverge

O Laravel não tem uma primitiva de fluxo de trabalho de primeira
classe - jobs são o vizinho mais próximo, mas eles fazem retry
re-executando o job inteiro, não retomando a partir da última etapa
bem-sucedida. O Suprnova disponibiliza fluxos de trabalho como uma
construção separada porque o Tokio torna barato o padrão "ficar
conectado a uma função async lenta por uma hora", e porque a
persistência no nível de etapa é a abstração certa para qualquer
interação externa de múltiplas etapas (provisionar um cliente,
executar uma saga entre dois provedores de pagamento, gerar um
relatório que envolve várias APIs upstream).

O design está mais próximo do [DBOS](https://www.dbos.dev/) e do
Cadence/Temporal do que de uma fila: estado durável, replay
determinístico, limites explícitos de etapa. A diferença em relação ao
Temporal é o peso operacional - não há um serviço de fluxo de trabalho
separado para executar; o worker é só `suprnova workflow:work` contra
o banco de dados da sua aplicação.

## Notas

- Corpos de etapa podem retornar qualquer tipo `Serialize + DeserializeOwned`. O tipo unitário `()` funciona para etapas que existem apenas pelo seu efeito colateral.
- Uma função `#[workflow_step]` chamada fora de um contexto de fluxo de trabalho executa inline - sem cache, sem replay. É assim que os testes exercitam corpos de etapa diretamente.
- O cache de etapa é indexado por `(step_name, step_index)`; renomeie uma etapa (ou reordene as chamadas) e o cache reseta para aquela etapa no próximo replay.
- `start_workflow!` aceita qualquer tupla de argumentos serializáveis. Tuplas preservam a ordem dos argumentos, então renomear parâmetros posicionais é seguro; mudar os tipos de argumento é uma quebra de schema para qualquer fluxo de trabalho em voo.
- A camada de [observabilidade](observability.md) do framework captura logs estruturados do worker (`worker_id`, `workflow_id`, `attempts`, `max_attempts`) em todo caminho de liquidação, para que você possa auditar orçamentos de retry em produção sem instrumentar suas etapas.

## Próximos passos

- [Filas](queues.md) - jobs de background de execução única com drivers sync/redis/database
- [Idempotência](idempotency.md) - wrappers para entrega pelo menos uma vez
- [Barramento](bus.md) - dispatch de comando síncrono com resultados tipados
- [Supervisores](supervisors.md) - supervisão de task de longa duração com auto-restart por captura de panic
- [Modelo de erros](error-model.md) - `FrameworkError`, o limite de panic, e por que a liquidação passa por `?`

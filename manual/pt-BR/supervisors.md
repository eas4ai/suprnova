# Supervisores

Um supervisor é uma task Tokio de longa duração que o framework inicia
no boot e reinicia automaticamente quando ela termina. Supervisores
servem para trabalho "always-on": heartbeats em background, coletores
de métricas, aquecedores de conexão, varredores periódicos, ou
qualquer loop assíncrono que nunca deveria parar de executar. Eles são
distintos dos [workers de fila](queues.md), que consomem itens `Job`
discretos de uma fila. Um supervisor não tem fila de jobs - ele é dono
do seu próprio loop e decide quando dormir, esperar, ou agir.

O `SupervisorRegistry` inicia cada supervisor registrado como uma task
Tokio destacada, observa o `JoinHandle` de cada task, e a reinicia de
acordo com sua `RestartPolicy` quando ela termina - seja retornando
`Err`, retornando `Ok`, ou sofrendo panic. Restarts são separados por
um backoff exponencial que começa em 100ms e tem um teto de 60
segundos, para que um supervisor em crash não entre em spin-loop e
inunde os logs.

## Início rápido

Defina um supervisor, registre-o via `inventory::submit!`, e chame
`SupervisorRegistry::start_all()` na inicialização.

**`src/supervisors/heartbeat.rs`:**

```rust
use async_trait::async_trait;
use std::time::Duration;
use suprnova::supervisor::{RestartPolicy, Supervisor};
use suprnova::{FrameworkError, SupervisorEntry};
use tokio_util::sync::CancellationToken;

pub struct LogHeartbeat;

#[async_trait]
impl Supervisor for LogHeartbeat {
    fn name(&self) -> &'static str { "heartbeat" }

    async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    tracing::info!("supervisor heartbeat tick");
                }
            }
        }
    }

    fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }
}

// Use o `suprnova::inventory` reexportado para que um app com scaffold não
// precise adicionar `inventory` como dependência direta.
suprnova::inventory::submit!(SupervisorEntry {
    factory: || Box::new(LogHeartbeat),
});
```

**`src/bootstrap.rs`:**

```rust
use suprnova::supervisor::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

Essa é a configuração completa. O supervisor `LogHeartbeat` inicia no
boot, loga a cada 60 segundos, e - como `RestartPolicy::Always`
reinicia tanto em saídas `Ok` quanto `Err` - é reiniciado
imediatamente se o loop algum dia terminar por qualquer motivo.

## Políticas de restart

Cada supervisor declara sua `RestartPolicy` através do método do
trait. O padrão é `OnError`.

| Política | Reinicia quando... | Caso de uso |
|--------|-----------------|----------|
| `RestartPolicy::OnError` | `run()` retorna `Err` ou sofre panic | Tasks que devem executar até a conclusão em caso de sucesso (ex.: um job de init único envolvido como supervisor). |
| `RestartPolicy::Always` | `run()` retorna `Ok` ou `Err`, ou sofre panic | Daemons verdadeiros - loops que nunca deveriam retornar. Se o loop terminar por qualquer motivo, isso é um bug e um restart se justifica. |
| `RestartPolicy::Never` | (nunca) | Tasks de execução única que devem executar uma vez e não devem ser reiniciadas independente do resultado. |

```rust
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::OnError }   // padrão
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }    // loop de daemon
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Never }     // execução única
```

**Quando escolher `Always` vs `OnError`.** Um supervisor de loop
infinito (`loop { ... }`) deve usar `Always` - se o loop algum dia
retornar `Ok(())`, algo inesperado aconteceu e um restart é a resposta
correta. Um supervisor que faz um trabalho finito e retorna `Ok` em
caso de sucesso (ex.: atualizar um cache uma vez) deve usar `OnError`
para que um término limpo não dispare um restart.

**`Never` para trabalho de execução única.** Prefira [workers de
fila](queues.md) ou [tarefas agendadas](scheduling.md) para trabalho
que executa em uma agenda. Use `RestartPolicy::Never` quando o padrão
de supervisor é conveniente para algo que precisa executar uma vez na
inicialização e nunca mais.

## Tratamento de panic

Panics dentro de `run()` são capturados pelo registry e tratados como
erros - um supervisor que sofre panic é reiniciado com backoff em vez
de derrubar o processo. O registry monitora o `JoinHandle` de cada
supervisor e detecta panics através do mecanismo padrão de join do
Tokio.

Da perspectiva da política de restart, um panic é sempre tratado como
uma saída `Err`, independente da política:

- `OnError` - reinicia depois de um panic (panic conta como erro).
- `Always` - reinicia depois de um panic (igual a qualquer outra saída).
- `Never` - não reinicia depois de um panic (igual a qualquer outra saída).

O panic é registrado em log no nível `error!` com o nome do supervisor
antes que o backoff de restart comece.

## Backoff

Quando um supervisor termina e sua política manda reiniciar, o
registry espera antes de spawnar o substituto:

| Restart consecutivo | Delay |
|---------|-------|
| 1º | 100ms |
| 2º | 200ms |
| 3º | 400ms |
| 4º | 800ms |
| ... | dobra a cada vez |
| Teto | 60s |

O backoff é resetado depois de uma execução saudável. O delay dobra a
cada restart *consecutivo* até o teto de 60 s, mas uma execução que
permanece de pé por pelo menos 60 s (a duração do teto) é tratada como
saudável: o próximo restart volta ao piso de 100 ms em vez de herdar o
backoff que subiu durante uma explosão anterior de falhas. Então um
daemon que executou de forma limpa por horas e então dá um blip
reinicia prontamente, não depois de uma espera de 60 s acumulada há
muito tempo.

O reset é baseado em liveness, e deliberadamente conservador: apenas
uma execução que *sobrevive ao backoff máximo possível* conta como
saudável. Uma execução que termina antes desse limiar carrega o
backoff atual para a frente, então um supervisor genuinamente flapping -
um cujas execuções nunca alcançam o limiar - ainda sobe até o teto de
60 s e permanece lá. O reset nunca esconde um supervisor que está em
crash-loop.

O teto de 60 segundos previne que um supervisor permanentemente
quebrado durma indefinidamente ou martele dependências externas em
cada nova tentativa. Combine com logging no nível `error!` para
alertar quando um supervisor entra na faixa de backoff alto.

## Shutdown gracioso

Supervisores recebem um `CancellationToken` como parâmetro de `run()`.
O framework cancela esse token em Ctrl-C / SIGTERM como parte da
sequência de shutdown de `Server::run`. Supervisores que querem dar
flush no estado, terminar trabalho em voo, ou de outra forma terminar
de forma limpa devem fazer `tokio::select!` em `cancel.cancelled()`:

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                tracing::info!("supervisor heartbeat tick");
            }
        }
    }
}
```

O framework drena o JoinSet do supervisor com uma janela de graça de 5
segundos depois do cancelamento. Supervisores que não honram o token
dentro dessa janela são abortados via `JoinSet::abort_all`. A drenagem
executa depois da drenagem do handler de WebSocket (para que conexões
WS sejam encerradas primeiro) e antes do flush dos buffers de
telemetria.

Supervisores que ignoram o token completamente vão executar até a
janela de 5 segundos expirar e então serão abortados à força. Se seu
supervisor mantém recursos que precisam de flush (descritores de arquivo
abertos, solicitações HTTP em voo, registros parcialmente escritos),
sempre faça select em `cancel.cancelled()` e limpe antes de retornar.

### Embedders e testes de integração

`Server::run` chama `SupervisorRegistry::shutdown(...)` para você.
Código que chama `SupervisorRegistry::start_all()` fora de
`Server::run` (embedders conduzindo o framework a partir de um binário
customizado, ou testes de integração que sobem supervisores
diretamente) também precisa chamar
`SupervisorRegistry::shutdown(timeout)` no teardown, ou tasks de
supervisor vão vazar além do tempo de vida do teste:

```rust
use std::time::Duration;
use suprnova::SupervisorRegistry;

// Setup do teste
SupervisorRegistry::start_all().await;

// ... exercita o supervisor ...

// Teardown do teste - cancela o token compartilhado, drena o JoinSet até
// `timeout`, então `abort_all` para os que sobraram.
SupervisorRegistry::shutdown(Duration::from_secs(1)).await;
```

`shutdown` é um no-op se `start_all` nunca foi chamado, então é seguro
chamá-lo do teardown sem condição.

## Observabilidade

Todo restart pelo caminho de erro emite uma entrada de log no nível
`error!` com campos estruturados:

- `supervisor` - de `Supervisor::name()`.
- `error` - a mensagem de erro do valor de retorno `Err` de `run()`, ou `"panic: <payload>"` para um panic capturado, ou `"join error: <detail>"` para uma falha de join fora do comum.
- `backoff_ms` - o delay de backoff em milissegundos antes do próximo spawn.

Panics são reportados através do mesmo log de erro - não há uma
mensagem separada de "sofreu panic":

```
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error=connection refused backoff_ms=400
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error="panic: \"deliberate test panic\"" backoff_ms=800
```

`RestartPolicy::Always` retornando `Ok(())` emite um `warn!` (não
`error!`) com os mesmos campos `supervisor` / `backoff_ms` e a
mensagem "supervisor returned Ok under Always policy; restarting" -
útil para detectar loops de daemon que terminaram de forma limpa
quando não deveriam ter terminado.

Supervisores não recebem um tracing span automático em torno de
`run()` - o registry cria um span em torno do ciclo de vida (start,
restart), mas não do interior da task. Emita seu próprio `info_span!`
ou instrumente o corpo do seu loop se você quiser contexto de span
sobre o trabalho feito dentro do supervisor:

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = async {
                let span = tracing::info_span!("heartbeat.tick");
                let _guard = span.enter();
                do_work().await.ok();
                tokio::time::sleep(Duration::from_secs(60)).await;
            } => {}
        }
    }
}
```

### Por que Suprnova diverge

O Laravel não tem um equivalente direto. O modelo
processo-por-solicitação do PHP torna impossíveis daemons in-process
always-on - trabalho de longa duração precisa viver fora do ciclo de
vida da solicitação, tipicamente como um processo worker gerenciado
por `supervisord` consumindo uma fila ou um comando agendado por cron.
O queue worker do Laravel (`php artisan queue:work`) é o análogo mais
próximo, mas ainda é um processo CLI de execução única que um
supervisor externo reinicia.

O Suprnova executa sobre o Tokio dentro de um único processo de longa
duração. Tarefas de background always-on se encaixam naturalmente como
tasks Tokio supervisionadas ao lado do servidor HTTP - sem fronteira
extra de processo, sem supervisor externo, sem canal IPC separado para
estado. O trait `Supervisor` é o equivalente in-process do
`supervisord`, restrito à própria árvore de tasks do framework, com as
mesmas garantias de restart-on-exit + backoff.

Workers de `Queue` (que o Laravel também tem) continuam existindo -
veja [Filas](queues.md) - para trabalho de job discreto. Supervisores
cobrem o caso "sempre ticando" que o Laravel empurra completamente
para fora da fronteira do framework.

## Fora do escopo da v1

Os seguintes itens são deliberadamente postergados:

- **Árvores de supervisão (pai/filho).** Não há hierarquia - todos os supervisores são pares sob o único `SupervisorRegistry`. Supervisão estruturada (em que um supervisor possui e reinicia supervisores filhos) é território de orquestrador.

- **Limites de recursos (cgroup, memória, CPU).** Aplique restrições de recursos através de arquivos de unit do systemd (`MemoryMax=`, `CPUQuota=`) ou requests/limits de recursos do Kubernetes no nível do pod. O framework não impõe limites de recursos internos ao processo em supervisores individuais.

- **Supervisão multi-máquina.** Supervisores executam dentro de um único processo em uma única máquina. Distribuir decisões de supervisão entre máquinas é território de orquestrador (Kubernetes, Nomad, systemd em múltiplos hosts).

## Referência

Os quatro tipos primários - `Supervisor`, `RestartPolicy`,
`SupervisorEntry`, `SupervisorRegistry` - são reexportados na raiz do
crate (`suprnova::Supervisor`, etc.) além do caminho mais longo
`suprnova::supervisor::*`. Os dois acessores livres permanecem sob
`suprnova::supervisor::*`.

| Símbolo | Propósito |
|--------|----------|
| `Supervisor` | Trait para implementar na sua struct de supervisor. Métodos obrigatórios: `name() -> &'static str`, `async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError>`. Opcional: `restart_policy() -> RestartPolicy` (padrão `OnError`). O token `cancel` é sinalizado no shutdown do processo; faça select em `cancel.cancelled()` para terminar de forma limpa antes que a janela de abort de 5 segundos expire. |
| `RestartPolicy` | Enum com as variantes `OnError`, `Always`, `Never`. Controla quando o registry spawna uma task substituta. |
| `SupervisorEntry` | Item de inventory. Declare `factory: fn() -> Box<dyn Supervisor>`. Submeta uma entrada por supervisor via `suprnova::inventory::submit!(SupervisorEntry { factory: || Box::new(MySupervisor) })`. |
| `SupervisorRegistry::start_all()` | Fn async. Itera todos os valores `SupervisorEntry` submetidos, spawna cada supervisor como uma task Tokio destacada no JoinSet por processo, e começa a monitorar restarts. Idempotente - os statics por processo são `OnceLock`s. Chame uma vez a partir do seu `register()` de inicialização. |
| `SupervisorRegistry::shutdown(timeout)` | Fn async. Cancela o token de cancelamento compartilhado para que todo supervisor observando `cancel.cancelled()` termine, drena o JoinSet até `timeout`, então faz `abort_all` para os que sobraram. `Server::run` invoca isso como parte da sua sequência de shutdown; embedders e testes de integração que chamam `start_all` fora de `Server::run` precisam chamar isso eles mesmos para evitar vazar tasks. No-op se `start_all` nunca foi chamado. |
| `suprnova::supervisor::supervisor_tasks()` / `supervisor_cancel_token()` | Acessores que retornam `Option<&'static …>` para o JoinSet e o token de cancelamento subjacentes. Usados pela sequência de shutdown de `Server::run`; expostos como `pub` para que embedders conduzindo o framework a partir de um binário customizado possam se integrar. Código de aplicação não deveria precisar deles. |

## Próximos passos

- [Filas](queues.md) - decisão entre supervisor-vs-queue-worker e a alternativa de job discreto
- [Agendamento](scheduling.md) - para trabalho periódico que não precisa de um loop de longa duração
- [Fluxos de trabalho](workflows.md) - para trabalho stateful de longa duração que precisa de resume durável
- [Transmissão](broadcasting.md) - usa a mesma sequência de shutdown (ordem de drenagem)
- [Ciclo de vida da solicitação](lifecycle.md) - onde `Server::run` e a drenagem de shutdown se encaixam

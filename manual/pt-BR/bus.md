# Barramento

O Barramento é o dispatcher de comandos **síncrono** do Suprnova. Você
define um `Command` tipado (`{ input, Output type }`), registra um
`Handler` para ele no boot, e então qualquer código no processo pode
chamar `Bus::dispatch(cmd).await` e receber de volta um
`Dispatched<T>` carregando o resultado tipado do handler.

O Barramento forma par com [`Queue`](queues.md) - a irmã assíncrona.
São duas facades intencionalmente separadas, não um único dispatcher
de roteamento:

| Se você quer…                                          | Use            |
|-------------------------------------------------------|----------------|
| Executar o trabalho *agora*, nesta task, e receber o resultado de volta | `Bus`          |
| Enviar o trabalho a um worker, com retry em caso de falha, durável  | `Queue`        |

O chamador escolhe explicitamente. O Suprnova não traz um marcador
`ShouldQueue` - no Tokio os dois caminhos são non-blocking, então a
escolha explícita é mais clara e mais rápida do que roteamento
implícito.

## Início rápido

Dez linhas do comando ao dispatch:

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::bus::command::{Command, Handler};
use suprnova::bus::Bus;
use suprnova::error::FrameworkError;

#[derive(Serialize, Deserialize)]
pub struct ChargeCustomer { pub customer_id: i64, pub cents: i64 }

#[async_trait]
impl Command for ChargeCustomer {
    type Output = String; // o charge id que recebemos de volta
    fn command_name() -> &'static str { "ChargeCustomer" }
}

pub struct ChargeCustomerHandler;

#[async_trait]
impl Handler<ChargeCustomer> for ChargeCustomerHandler {
    async fn handle(&self, cmd: ChargeCustomer) -> Result<String, FrameworkError> {
        Ok(format!("charge-{}-{}", cmd.customer_id, cmd.cents))
    }
}

// No boot (uma vez):
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);

// Em um handler de solicitação:
let charge_id = Bus::dispatch(ChargeCustomer { customer_id: 42, cents: 1999 })
    .await?
    .unwrap_executed();
```

## Definindo comandos

Um `Command` é qualquer struct serializável com um tipo `Output`
associado e um `command_name()` único:

```rust
#[async_trait]
pub trait Command: Serialize + DeserializeOwned + Send + Sync + 'static {
    type Output: Send + 'static;
    fn command_name() -> &'static str;
}
```

O `Output` é o que o handler retorna. Ele só precisa ser
`Send + 'static` - o caminho real de dispatch mantém os valores
nativos via `Box<dyn Any>`, sem round-trip de serde. Isso significa
que outputs que não são serde, como `Bytes`, handles opacos, ou um
`Arc<Mutex<…>>`, fazem o round-trip de volta ao chamador como valores
vivos. O bound `Serialize + DeserializeOwned` no próprio `Command` é
para o caminho de fake-capture: `Bus::fake()` registra cada comando
despachado como um `serde_json::Value` para que assertions baseadas em
predicado (`assert_dispatched`, `assert_dispatched_times`) possam
decodificá-los e inspecioná-los.

`command_name()` deve ser uma string estável, única por impl concreta
de `Command`. Ela aparece nas mensagens de falha de
`assert_dispatched`/`assert_dispatched_times` e nos retornos de erro
quando nenhum handler está registrado.

## Registrando handlers

Um `Handler<C>` é uma função async tipada que recebe o comando e
retorna `Result<C::Output, FrameworkError>`:

```rust
#[async_trait]
pub trait Handler<C: Command>: Send + Sync + 'static {
    async fn handle(&self, cmd: C) -> Result<C::Output, FrameworkError>;
}
```

Chame `Bus::register::<C, H>(handler)` uma vez por tipo de comando no
boot. O registry é global; registrar o mesmo `C` de novo sobrescreve o
handler anterior (testes dependem disso para trocar implementações) e
emite um `tracing::warn!` para que uma vinculação duplicada vinda de
dois registros de serviço no boot fique visível no log.

```rust
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);
Bus::register::<RefundCustomer, _>(RefundCustomerHandler);
```

## Despachando

`Bus::dispatch::<C>(cmd)` executa o handler registrado in-process e
retorna um enum `Dispatched<C::Output>`:

```rust
pub enum Dispatched<T> {
    Executed(T),  // handler executou, aqui está o resultado
    Captured,    // Bus::fake() estava ativo, o handler NÃO executou
}
```

`Dispatched<T>` tem quatro helpers:

- `.unwrap_executed()` - retorna o valor, sofre panic em `Captured`
- `.executed() -> Option<T>` - converte para `Option`
- `.is_executed()` - predicado booleano
- `.is_captured()` - predicado booleano

Para call sites em modo real, `.unwrap_executed()` é a forma
idiomática.

### `Bus::chain` - sequencial

`Bus::chain(Vec<C>)` executa comandos um de cada vez, parando no (e
incluindo o) primeiro erro. Todos os comandos precisam ser do mesmo
tipo. Retorna `Vec<Result<Dispatched<C::Output>, FrameworkError>>` -
uma entrada por comando tentado.

```rust
let results = Bus::chain(vec![
    ChargeCustomer { customer_id: 1, cents: 100 },
    ChargeCustomer { customer_id: 2, cents: 200 },
    ChargeCustomer { customer_id: 3, cents: 300 },
]).await;

// Coleta os charge ids que tiveram sucesso até a primeira falha:
let charge_ids: Vec<String> = results
    .into_iter()
    .filter_map(|r| r.ok().and_then(|d| d.executed()))
    .collect();
```

`Bus::chain` é homogêneo por design - o dispatcher retorna
`Dispatched<C::Output>`, que só é bem tipado quando toda entrada
compartilha um `Output`. Para chains heterogêneas no estilo Laravel
(tipos de job misturados, cada etapa disparando a próxima), use
[`Queue::chain`](queues.md) - a fila encapsula cada job em um envelope
tipado e por isso não tem a mesma restrição.

### `Bus::batch` - concorrente

`Bus::batch(Vec<C>)` executa comandos concorrentemente via
`futures::join_all` e coleta os resultados na ordem de entrada. Mesma
restrição de tipo homogêneo que `chain`.

```rust
let results = Bus::batch(vec![
    SendWelcomeEmail { user_id: 1 },
    SendWelcomeEmail { user_id: 2 },
    SendWelcomeEmail { user_id: 3 },
]).await;
```

`Bus::batch` é homogêneo pelo mesmo motivo que `chain`. Para batches
heterogêneos e persistidos com callbacks de progresso, eventos de
ciclo de vida, e um `BatchRepository`, use
[`Queue::batch`](queues.md).

## Testes

Instale o fake no topo do teste. `install_fake()` adquire um mutex
`FAKE_SERIAL` global do processo pelo tempo de vida da guarda, então
dois testes `Bus::fake()` paralelos não conseguem atropelar o
captured-store um do outro - o segundo bloqueia até a primeira guarda
ser dropada. Você ainda marca o teste com `#[serial]` se um teste
irmão no mesmo binário chama `Bus::dispatch` real: um chamador de
dispatch real não adquire `FAKE_SERIAL`, então sem `#[serial]` ele
pode competir com um teste fake paralelo e observar
`is_active() == true`. `FAKE_SERIAL` remove o risco fake-vs-fake,
`#[serial]` remove o risco real-vs-fake.

```rust
use serial_test::serial;
use suprnova::bus::Bus;
use suprnova::bus::testing::{
    assert_dispatched,
    assert_dispatched_times,
    assert_not_dispatched,
    assert_nothing_dispatched,
    install_fake,
};

#[tokio::test]
#[serial]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(/* … */).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

O fake captura comandos despachados sem executar seus handlers. Uma
chamada a `Bus::dispatch` retorna `Ok(Dispatched::Captured)` (sem
output de handler) em vez de `Executed`. Erros reais - falhas de
encode/decode, um handler registrado ausente antes do fake ser
instalado - ainda aparecem como `Err(_)`.

`install_fake()` retorna um `BusFakeGuard`. Drope-o (é RAII) e o fake
é limpo e o mutex `FAKE_SERIAL` é liberado. O idioma típico é
`let _guard = install_fake();` no topo do teste.

### Superfície de assertion

| Assertion                                            | Verifica…                                                   |
|------------------------------------------------------|------------------------------------------------------------|
| `assert_dispatched::<C>(pred)`                       | pelo menos um comando do tipo `C` correspondendo a `pred`           |
| `assert_not_dispatched::<C>(pred)`                   | zero comandos do tipo `C` correspondendo a `pred`                  |
| `assert_dispatched_times::<C>(pred, count)`          | exatamente `count` comandos do tipo `C` correspondendo a `pred`       |
| `assert_nothing_dispatched()`                        | zero comandos de qualquer tipo despachados sob o fake ativo |

As quatro sofrem panic com `Bus::fake() must be active` se nenhum fake
está instalado. As com escopo de tipo sofrem panic com
`expected … dispatched <command_name> …` quando a contagem não
corresponde. `assert_nothing_dispatched` sofre panic com
`expected no dispatched commands but found <n>`.

## Quando usar `Queue` em vez disso

Recorra a [`Queue`](queues.md) quando você quiser qualquer um destes:

- **Durabilidade entre restarts.** Um job enfileirado sobrevive a um crash de processo se o driver for `database` ou `redis`.
- **Tentativas com backoff.** O worker da fila aplica `Job::max_tries` + `Job::backoff` (exponential / fixed / sequence) a cada falha.
- **Timeout por job.** `Job::timeout` + `Job::fail_on_timeout` são honrados pelo loop do worker.
- **Execução atrasada.** `Queue::later(duration, job)` ou `Queue::push_later(job, at)`.
- **Dedupe / idempotência.** `Job::unique_id` + `Queue::push_unique` bloqueia reenvios por um TTL configurável.
- **Desacoplar o chamador do worker.** Execute jobs em uma frota separada de workers `cargo run --bin app -- queue:work`.

Recorra a `Bus` quando você quiser qualquer um destes:

- **In-process, executa agora.** Sem serialização entre processos.
- **Resultado tipado de volta ao chamador.** `Dispatched<C::Output>` carrega o valor de retorno tipado do handler até o call site.
- **Composição síncrona.** Um handler de solicitação que decompõe o trabalho em chamadas `Command` menores e lê cada resultado em sequência.

Um app típico usa os dois: caminhos de solicitação síncronos despacham
operações que retornam resultado através do `Bus`, e trabalho "fire
and forget" / durável é enviado através do `Queue`.

## Próximos passos

- [Filas](queues.md) - irmã assíncrona, drivers, worker, política de retry, chains e batches heterogêneos
- [Eventos](events.md) - dispatcher pub/sub (um evento → muitos listeners)
- [Fluxos de trabalho](workflows.md) - trabalho stateful de longa duração que sobrevive a restarts, para quando uma chain não é suficiente
- [Testes](testing.md) - `#[suprnova_test]`, fakes de contêiner, e o padrão de serializer global do processo usado por `Bus::fake()`

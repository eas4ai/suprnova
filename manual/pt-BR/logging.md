# Logs

O Suprnova registra logs através do [`tracing`](https://docs.rs/tracing) -
toda linha de log é um evento estruturado com campos, não uma string
formatada. Um subscriber é instalado no boot que lê `LOG_LEVEL` e
`LOG_FORMAT` do ambiente, emite saída pretty de várias linhas em dev e um
objeto JSON por linha em produção, e propaga um id por solicitação para
todo evento que um handler emite.

Este capítulo cobre a superfície de log em si: o subscriber, os formatos,
os níveis, e a correlação por request id que torna um log de produção
pesquisável. Para a ponte com OpenTelemetry e o log de consultas veja
[Observabilidade](observability.md); para o conjunto `Context` da
solicitação que emissores podem ler junto com o id veja
[Contexto](context.md).

## O que é registrado e onde

Duas saídas por padrão:

| Onde | Formato | Quando |
|---|---|---|
| `stdout` | `LogFormat::Pretty` - várias linhas, colorido, amigável para humanos | dev (`APP_ENV` é `local`, `dev`, `testing`, …) |
| `stdout` | `LogFormat::Json` - um objeto JSON por linha | produção (`APP_ENV=production` / `prod`) |

O padrão dev/prod é calculado a partir de `APP_ENV` via
`Environment::detect()`. Sobrescreva com `LOG_FORMAT=pretty` ou
`LOG_FORMAT=json` para forçar um explicitamente.

```env
# .env (dev)
LOG_LEVEL=info,sqlx=warn
LOG_FORMAT=pretty   # opcional; este é o padrão em dev

# .env.production
LOG_LEVEL=info,sqlx=warn,suprnova::queue=debug
LOG_FORMAT=json     # opcional; este é o padrão em prod
```

O framework só escreve em `stdout`. Em produção aponte para lá o runtime
de contêiner, o journal do systemd, ou o seu agregador de logs
(`docker logs`, `kubectl logs`, `journalctl -u my-app`, um agente
Loki/Vector, etc.). Não existe um appender de arquivo com rotação - deixe
a plataforma ser dona da persistência de log.

## Emitindo eventos

Use as macros do `tracing` em handlers, jobs, middleware, em qualquer
lugar:

```rust
use suprnova::{json_response, session, Request, Response};
use tracing::{debug, info, warn, error, instrument};

pub async fn checkout(_req: Request) -> Response {
    let user_id: i64 = session()
        .and_then(|s| s.get::<i64>("user_id"))
        .unwrap_or(0);

    info!(user_id, "checkout starting");

    let order = place_order(user_id).await.map_err(|e| {
        error!(user_id, error = %e, "checkout failed");
        e
    })?;

    info!(user_id, order_id = order.id, total = order.total_cents, "checkout succeeded");

    json_response!(order)
}
```

Cada campo vira uma chave de primeiro nível na saída JSON e um par
`field=value` colorido na saída pretty. Prefira campos a interpolação -
eles são pesquisáveis em logs JSON e o formatador cuida da renderização
ciente do tipo.

Para envolver uma função em um span e marcar todo evento dentro dela com
campos compartilhados, use `#[instrument]`:

```rust
#[instrument(skip(db), fields(user_id = %user_id))]
pub async fn load_dashboard(
    db: &suprnova::DatabaseConnection,
    user_id: i64,
) -> Result<Dashboard, FrameworkError> {
    info!("loading"); // carrega automaticamente o user_id vindo do span
    // … consultas …
}
```

O mesmo `#[instrument]` vira um span do OpenTelemetry quando a feature
`otel` está habilitada - veja
[Observabilidade](observability.md#opentelemetry).

## Níveis de log

`LOG_LEVEL` é uma [diretiva de env-filter do
`tracing-subscriber`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html),
não um único nível. A gramática são pares `target=level` separados por
vírgula, em que valores isolados definem o padrão:

```env
LOG_LEVEL=info                                  # tudo em info+
LOG_LEVEL=debug                                 # tudo em debug+
LOG_LEVEL=info,sqlx=warn                        # info por padrão, sqlx mais quieto
LOG_LEVEL=warn,suprnova::queue=debug,my_app=info  # warn por padrão, dois targets verbosos
```

Targets geralmente são o crate emissor ou o caminho do módulo
(`suprnova::queue`, `hyper::server`, `my_app::services::checkout`).
Encontre um target lendo a linha de log JSON - o campo `target` em todo
evento é a sua chave de filtro.

Níveis em ordem crescente de verbosidade: `error` < `warn` < `info`
(padrão) < `debug` < `trace`. A resposta de erro que vai pela rede é
sempre sanitizada para `{"message": "Internal Server Error"}`
independentemente do nível - o detalhe vai apenas para o log estruturado.

### Diretivas inválidas não derrubam o boot

Um `LOG_LEVEL` malformado (por exemplo, `LOG_LEVEL=app=notalevel`) faz
fallback para `"info"` e escreve um aviso de uma linha em `stderr`:

```text
suprnova: invalid LOG_LEVEL directive "app=notalevel" (...); falling back to "info". Fix LOG_LEVEL to silence this.
```

Isso é `stderr` em vez de `tracing::warn!` porque o subscriber ainda não
foi instalado - um `warn!` seria silenciosamente descartado. Corrija a
diretiva e o aviso desaparece.

## Saída pretty vs JSON

O mesmo `info!(user_id = 42, "saved")` renderiza de forma diferente em
cada formato.

**Pretty (dev):**

```text
  2026-05-30T22:14:08.221341Z  INFO request{request_id=78a9...} my_app::handlers::checkout: saved
    at src/handlers/checkout.rs:48
    in checkout
    in request with request_id: 78a9..., method: POST, path: /checkout
```

**JSON (prod):**

```json
{
  "timestamp": "2026-05-30T22:14:08.221341Z",
  "level": "INFO",
  "fields": { "message": "saved", "user_id": 42 },
  "target": "my_app::handlers::checkout",
  "span": { "name": "checkout" },
  "spans": [
    { "name": "request", "request_id": "78a9...", "method": "POST", "path": "/checkout" }
  ]
}
```

A forma JSON é o que os agregadores de produção (Datadog, Loki,
Honeycomb, CloudWatch, …) fazem parse de imediato. `span.request_id` é a
chave de correlação - veja abaixo.

## Correlação por request id

Toda solicitação HTTP recebe um `RequestId` do `RequestIdMiddleware`, o
middleware mais externo de toda chain. O id é:

- **Reaproveitado** de um header `X-Request-Id` de entrada que seja
  seguro (alfanuméricos mais `- _ . :`, até 128 bytes), ou **cunhado na
  hora** como um UUID v4 se estiver ausente / for inseguro.
- **Ecoado** de volta na resposta como `X-Request-Id` (tanto na variante
  2xx quanto na 5xx).
- **Colocado em escopo** em um span `request` do `tracing`, para que todo
  evento de qualquer middleware, handler ou biblioteca abaixo carregue
  `request_id` no seu array `spans` automaticamente.
- **Semeado** no conjunto `Context` da solicitação como `_request_id`,
  para que emissores que querem a string pura (jobs, payloads de
  transmissão, relatórios de erro) possam lê-lo por nome.

Leia-o em código com `current_request_id()`:

```rust
use suprnova::current_request_id;
use tracing::info;

if let Some(id) = current_request_id() {
    info!(request_id = %id, "checkpoint reached");
}
```

`current_request_id()` retorna `Option<RequestId>` porque trabalho em
background (jobs, tarefas agendadas, testes que não instalaram o
middleware) roda fora de qualquer escopo de solicitação.

### Tarefas em background: faça spawn com o id

`tokio::spawn` inicia uma task nova com task-locals vazios - um handler
que faz spawn de trabalho com efeito colateral perde o
`current_request_id()` e seus eventos de log ficam órfãos. Use
`spawn_with_request_id` em vez dele:

```rust
use suprnova::spawn_with_request_id;
use tracing::info;

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    let order = place_order().await?;

    spawn_with_request_id(async move {
        // Esta task ainda enxerga current_request_id().
        // Seus eventos de log carregam o mesmo request_id que os do handler.
        info!(order_id = order.id, "post-checkout fanout running");
        send_receipt(order.id).await;
        update_analytics(order.id).await;
    });

    suprnova::Response::ok().json(&order)
}
```

O helper propaga tanto o task-local `RequestId` quanto o
`tracing::Span` atual, então os eventos da future criada aninham sob o
mesmo span `request` no log. Fora de um escopo de solicitação ativo ele
recai para um `tokio::spawn` puro - seguro de usar incondicionalmente.

Apenas o request id e o span do tracing seguem a task - o conjunto
`Context` da solicitação deliberadamente não segue, porque trabalho em
background não está servindo a solicitação HTTP que o originou.

## O subscriber

O framework instala um subscriber global do `tracing` no boot, a partir
de `Server::run()`. Você quase nunca chama isso você mesmo; está
documentado porque testes, quem embute o framework e pontos de entrada
incomuns às vezes precisam.

```rust
use suprnova::{LogConfig, init_subscriber};

// Leia LOG_LEVEL / LOG_FORMAT do ambiente:
init_subscriber(LogConfig::from_env());

// Ou de forma programática:
init_subscriber(LogConfig {
    level: "info,sqlx=warn".to_string(),
    format: suprnova::LogFormat::Json,
});
```

`init_subscriber` é **idempotente**. Uma segunda chamada deixa o
subscriber existente no lugar e emite um `tracing::warn!` para que um
operador consiga ver que o novo `LogConfig` não foi aplicado. É isso que
permite que testes que chamam `init_subscriber` cada um não disputem
entre si - o primeiro vence, os demais são no-ops.

Para a variante ciente de OTel (o mesmo `LogConfig`, mais a exportação de
tracing distribuído), use
[`init_telemetry`](observability.md#opentelemetry).

### Os daemons

`queue:work`, `schedule:work`, `schedule:run` e `workflow:work` são
subcomandos do binário da sua app e não sobem por `Server::run()`, então
instalam o próprio subscriber na subida. Eles leem o mesmo `LOG_LEVEL` e
`LOG_FORMAT` que o servidor, e você não chama nada:

```bash
LOG_LEVEL=info,suprnova::queue=debug cargo run --bin my-app -- queue:work

# …ou, em um contêiner, contra o binário compilado:
LOG_LEVEL=info my-app queue:work
```

Antes da 0.9.1 esse caminho não instalava nada. Toda linha `tracing::`
que os daemons emitem ia para lugar nenhum e o `LOG_LEVEL` era inerte
para eles, o que em um contêiner deixava o banner de inicialização como a
única saída - um worker mandando jobs para dead-letter, um agendador
pulando um tick cuja eleição ele havia perdido, e um lock que ele não
conseguiu liberar pareciam todos idênticos a um processo ocioso. Se você
está rodando um build fixado anterior à 0.9.1 e se perguntando por que um
worker não diz nada, é por isso, e a correção é a atualização, não uma
mudança de configuração.

A maior parte do que um worker tem a dizer ele diz em `warn!` e `error!` -
um job esgotando suas tentativas, uma dead-letter que ele não conseguiu
persistir, um lock que ele não conseguiu liberar - então o nível padrão
`info` já basta para enxergar problema. Baixe para `debug` quando
precisar também das decisões mais silenciosas.

## Testes

Testes não precisam instalar um subscriber - o attribute
`#[suprnova_test]` e o `TestContainer::fake` montam maquinaria suficiente
para que os eventos de handler fluam. Se você quiser fazer asserções
sobre a saída de log, capture via
[`tracing_subscriber::fmt::TestWriter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/struct.TestWriter.html)
do `tracing-subscriber` ou com uma layer customizada; o framework
deliberadamente não fornece um fake de "capture todos os logs neste
teste" porque os padrões de teste usuais do `tracing-subscriber`
funcionam bem.

## Por que Suprnova diverge

O Laravel usa o [Monolog](https://github.com/Seldaek/monolog) - strings
de mensagem com arrays de contexto opcionais, canais de log, e handlers
por canal (arquivo, syslog, Slack, …). O modelo de uma solicitação por
processo do PHP faz com que um único logger estático global seja seguro:
cada solicitação recebe seu próprio processo e seu próprio contexto.

O modelo de processo do Rust é o oposto - um processo serve muitas
solicitações concorrentes em muitas threads. Um formatador global de
strings disputaria o contexto e exigiria levar o `request_id` à mão por
cada local de chamada. O `tracing` resolve os dois com campos
estruturados e spans task-local: nada para levar à mão, os campos
continuam tipados, e a correlação é automática porque o span da
solicitação está em escopo para todo evento que a chain emite.

A saída apenas em `stdout` também é intencional. Em deploys em contêiner
(a única forma como o Suprnova é distribuído) o runtime, não a app, é
dono da persistência de log - rotação de arquivo, retenção e envio
pertencem todos à plataforma.

## Próximos passos

- [Observabilidade](observability.md) - OpenTelemetry, log de consultas,
  a superfície completa para operadores
- [Contexto](context.md) - o conjunto por solicitação onde `_request_id`
  e outros campos contextuais vivem
- [Tratamento de erros](errors.md) - como o limite de panic do framework
  e o caminho 5xx emitem seus próprios eventos estruturados
- [Variáveis de ambiente](env-vars.md) - referência de `LOG_LEVEL`,
  `LOG_FORMAT`

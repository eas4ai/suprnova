# Comandos de agendamento

Superfície de CLI para o agendador de tarefas por minuto. Os três
subcomandos `schedule:*` todos delegam para o dispatch
`Application::run()` do binário da sua aplicação, então eles veem a
mesma config, serviços, observers, e listeners que um handler de
solicitação vê. O modelo completo do agendador - trait `Task`, API
fluente de cron, `without_overlapping`, `run_in_background` - vive em
[Agendamento](scheduling.md); este capítulo é a referência de operador
para os próprios comandos.

## Como os comandos executam

`suprnova schedule:run`, `suprnova schedule:work`, e
`suprnova schedule:list` são shells finos que invocam
`cargo run -- schedule:<subcommand>` contra o projeto no diretório
atual. Os mesmos subcomandos também são alcançáveis diretamente no
binário da aplicação em produção:

```bash
# No desenvolvimento (a partir da raiz do projeto, build de fonte):
suprnova schedule:run

# Em produção (binário no PATH):
/usr/local/bin/myapp schedule:run
```

Os drivers de runtime (Cache, Queue, RateLimit, Mail) e sua
`bootstrap_fn` são inicializados antes de qualquer tarefa executar,
então uma tarefa agendada pode resolver serviços a partir do
contêiner exatamente como um controlador - veja
[Inicialização da aplicação](bootstrap.md).

Você precisa conectar o agendador ao builder da aplicação para que os
subcomandos encontrem alguma tarefa:

```rust
// cmd/main.rs (starter de backend) ou src/main.rs (starter de API)
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)   // <-- o hook do agendador
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await
```

`suprnova make:task <Name>` conecta isso automaticamente; se você
constrói a chain à mão, adicione a chamada `.schedule(...)` você
mesmo.

## schedule:run

Avalia toda tarefa registrada uma vez e executa as que têm a
expressão cron correspondendo ao minuto atual. Projetado para ser
invocado pelo cron do sistema a cada minuto. Sai com código não-zero
se alguma tarefa falhou; sai com zero (com `No tasks were due.`) se
nada estava devido neste minuto.

```bash
suprnova schedule:run
```

### Saída de exemplo

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✓ send:reminders
```

Quando uma tarefa retorna um erro, sua linha é prefixada com `✗` e a
mensagem de erro é anexada:

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✗ backup:database: connection refused
```

Quando nenhuma tarefa está devida neste minuto:

```
Running due scheduled tasks...
No tasks were due.
```

### Entrada de crontab

Uma única entrada executa o agendador a cada minuto. O binário da
aplicação avalia todas as tarefas devidas por conta própria, então
essa é a única linha de crontab que um host de produção precisa:

```cron
* * * * * cd /path/to/your/project && /usr/local/bin/myapp schedule:run >> /var/log/myapp/schedule.log 2>&1
```

Se você está executando `schedule:run` a partir do cron do sistema em
mais de um host (ou junto com um daemon `schedule:work`), tarefas
marcadas com `.without_overlapping()` precisam de um backend de Cache
configurado (`CACHE_DRIVER=redis` é a escolha de nível de produção)
para coordenar entre processos - veja
[Prevenindo sobreposição](scheduling.md#preventing-overlapping) para a
semântica de lock.

## schedule:work

Executa o agendador como um daemon de longa duração. O primeiro tick
é alinhado ao próximo limite de minuto, depois o loop avalia as
tarefas devidas uma vez por minuto até receber `SIGINT` (Ctrl-C) ou
`SIGTERM`. No shutdown, qualquer tarefa `run_in_background` ainda em
voo é aguardada antes de sair para que não sejam derrubadas no meio
de uma escrita.

```bash
suprnova schedule:work
```

### Saída de exemplo

```
Starting scheduler daemon...
Press Ctrl+C to stop

==============================================
  suprnova Scheduler Daemon
==============================================
  3 task(s) registered. Press Ctrl+C to stop.
==============================================
```

Cada tick é silencioso - só falhas são logadas. No shutdown:

```
suprnova: scheduler shutting down.
suprnova: waiting for 1 background task(s) to finish…

Scheduler daemon stopped.
```

### Casos de uso

- **Desenvolvimento.** Nenhum crontab necessário - inicie o daemon em
  um terminal e observe ele tickar.
- **Docker.** Use como o processo principal do contêiner quando você
  quer que uma imagem desempenhe o papel de agendador.
- **Systemd.** Gerencie-o como uma unidade de longa duração (veja a
  [unidade systemd](#unidade-systemd) abaixo).

### Unidade systemd

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/usr/local/bin/myapp schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

`Restart=always` traz o daemon de volta se ele crashar;
`RestartSec=5` faz debounce de um loop de crash. Como o limite de
panic do framework captura tarefas que sofrem panic e as converte em
`FrameworkError`, uma única tarefa ruim não deveria crashar o daemon -
`Restart=always` é para a rara falha em todo o processo (OOM, parent
kill).

## schedule:list

Imprime toda tarefa registrada com sua expressão cron, próximo horário de
execução e descrição.

```bash
suprnova schedule:list
suprnova schedule:list --timezone=Asia/Tokyo
```

### Saída de exemplo

```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] next: 2026-05-29 03:00 UTC
  send:reminders [0 9 * * *] next: 2026-05-28 09:00 UTC
  heartbeat [* * * * *] next: 2026-05-28 12:01 UTC
  report:generate [0 6 * * *] (UTC) next: 2026-05-29 06:00 UTC
```

Tarefas com um `.description(...)` encadeado no builder incluem a
descrição depois do próximo horário de execução; tarefas sem descrição
mostram só o cron e a próxima execução.

O `next:` é o primeiro minuto depois de agora em que a expressão casa; uma
expressão que nunca pode casar imprime `next: never`. Os horários são
mostrados em UTC, a menos que `--timezone` nomeie outro fuso IANA, e um
nome de fuso desconhecido sai com erro antes de qualquer coisa ser
impressa.

Uma tarefa que fixou o próprio fuso com `.timezone(...)` tem a sua
expressão reescrita para o fuso da listagem e rotulada com ele - o
`report:generate` acima pediu `02:00 America/New_York`. Tarefas sem fuso
fixado são impressas como escritas e não carregam rótulo. Veja
[Agendamento](scheduling.md) para as regras de fuso horário por completo,
incluindo quando uma reescrita é recusada e quando uma tarefa pode ocupar
várias linhas.

Quando nada está registrado (a chamada `.schedule(...)` do builder
está faltando, ou `schedule::register` é um no-op):

```
No scheduled tasks registered.
Define tasks in src/schedule.rs and wire it with `Application::schedule(schedule::register)`.
```

## Gerando uma tarefa

O framework distribui um gerador que cria a tarefa, a conecta ao
projeto, e adiciona a chamada do agendador ao seu `main.rs`:

```bash
suprnova make:task CleanupLogs
```

Isso:

1. Cria `src/tasks/cleanup_logs_task.rs` (um stub de `Task` funcional
   que loga sua própria duração)
2. Cria `src/tasks/mod.rs` (reexportando `CleanupLogsTask`) se ainda
   não existir
3. Cria `src/schedule.rs` (com uma função `register(&mut Schedule)`)
   se ainda não existir
4. Declara `pub mod schedule;` e `pub mod tasks;` em `src/lib.rs`
5. Adiciona `.schedule(<crate>::schedule::register)` à chain
   `Application` em `cmd/main.rs` (ou `src/main.rs` para o starter de
   API)

As etapas 2-5 são idempotentes, então executar `make:task` de novo
repara a fiação que foi removida manualmente. Veja
[Geradores](cli-generators.md) para a família `make:*` mais ampla.

Depois de gerar, registre a tarefa em `src/schedule.rs`:

```rust
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );
}
```

A API fluente do builder (`.daily()`, `.cron(...)`,
`.without_overlapping()`, `.run_in_background()`, modificadores
específicos de dia) é totalmente coberta em
[Agendamento](scheduling.md).

## Exit codes

| Comando | Sai com zero | Sai com não-zero |
|---|---|---|
| `schedule:run` | toda tarefa devida retornou `Ok(())`, ou nenhuma tarefa estava devida | pelo menos uma tarefa retornou `Err(_)` ou sofreu panic |
| `schedule:work` | shutdown limpo via `SIGINT` / `SIGTERM` (o wrapper trata o código de saída 130 como Ctrl-C limpo) | falha de bootstrap, ou o processo do daemon abortou |
| `schedule:list` | a listagem teve sucesso (incluindo a mensagem "no tasks registered") | a aplicação falhou ao inicializar |

Falhas de tarefa em background dentro de `schedule:work` são logadas
no stderr mas não fazem o daemon sair - o limite `catch_unwind` do
`JoinSet` as expõe como `FrameworkError`, e o loop de tick continua.

### Por que Suprnova diverge

O `schedule:run` do Laravel é o único ponto de entrada de primeira
classe; a forma de daemon (`schedule:work`) é um backport para hosts
sem crontab. O PHP não tem processo de longa duração, então todo
minuto é um runtime novo que precisa reinicializar o framework, o
contêiner, e toda vinculação de serviço.

No Suprnova o daemon é de primeira classe. `schedule:work` executa
dentro do mesmo runtime Tokio que serve HTTP, então:

- **Tarefas em background se compõem com o loop de tick.** Uma tarefa
  `.run_in_background()` é spawnada em um `JoinSet`; o loop faz
  polling das completadas antes do próximo tick e drena o resto no
  shutdown. O Laravel spawna um processo filho por tarefa em
  background.
- **Shutdown gracioso drena trabalho em voo.** Ctrl-C / SIGTERM deixa
  tarefas inline terminarem sua chamada atual e aguarda todo spawn em
  background antes de sair. O Laravel depende do SO para matar o
  filho do cron.
- **O custo de boot é pago uma vez.** O contêiner, os drivers, e sua
  `bootstrap_fn` inicializam no início do daemon, não em todo tick.
  `schedule:run` ainda paga o custo de boot por invocação (é um
  subcomando de execução única), mas o caminho do daemon é onde o
  modelo de runtime compensa.

`schedule:run` ainda funciona (e é a escolha certa quando o cron do
sistema já é a fonte de verdade do operador). Escolha o que se encaixa
na forma do seu deployment - os dois compartilham as mesmas
definições de tarefa.

## Próximos passos

- [Agendamento](scheduling.md) - o trait `Task`, a API fluente de
  cron, `without_overlapping`, `run_in_background`, e o dedupe no
  mesmo minuto
- [Geradores](cli-generators.md) - a família `make:*` completa,
  incluindo `make:task`
- [Console](console.md) - tarefas de operador de execução única
  anotadas com `#[command]` (fora de uma agenda)
- [Fila](queues.md) - para trabalho que deveria ser pego por um
  worker em vez de ticar em um relógio
- [Inicialização da aplicação](bootstrap.md) - como `.schedule(...)`
  se conecta ao builder, e o que tarefas conseguem resolver a partir
  do contêiner

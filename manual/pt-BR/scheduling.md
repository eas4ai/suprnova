# Agendamento de tarefas

Tarefas agendadas são funções async que o framework executa segundo
uma expressão cron - a cada minuto, de hora em hora, diariamente,
semanalmente, ou qualquer cron customizado de 5 campos. Tarefas vivem
dentro do binário da sua aplicação; `schedule:run` avalia as tarefas
devidas uma vez (chame-o a partir do cron do sistema) e
`schedule:work` executa o mesmo avaliador como um daemon de longa
duração.

## Gerando tarefas

A forma mais rápida de criar uma nova tarefa agendada é usando a CLI
do suprnova:

```bash
suprnova make:task CleanupLogs
```

Este comando vai:
1. Criar `src/tasks/cleanup_logs_task.rs` com um stub de tarefa funcional
2. Criar `src/tasks/mod.rs` se não existir, reexportando a tarefa
3. Criar `src/schedule.rs` para registrar tarefas, se não existir
4. Declarar `pub mod schedule;` e `pub mod tasks;` em `src/lib.rs`
5. Conectar `.schedule(<crate>::schedule::register)` no builder da sua aplicação em `cmd/main.rs` (ou `src/main.rs` para o starter de API)

As etapas 2 a 5 são idempotentes, então executar `make:task` de novo
repara a fiação que foi removida manualmente. O agendador executa
dentro do binário da sua aplicação - não há um executável de agendador
separado para compilar ou fazer deploy.

```bash Examples
# Cria CleanupLogsTask em src/tasks/cleanup_logs_task.rs
suprnova make:task CleanupLogs

# Cria SendRemindersTask em src/tasks/send_reminders_task.rs
suprnova make:task SendReminders

# Você também pode incluir o sufixo "Task" (mesmo resultado)
suprnova make:task BackupDatabaseTask
```

```rust Generated File
//! CleanupLogsTask scheduled task
//!
//! Created with `suprnova make:task cleanup_logs_task`.

use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

/// CleanupLogsTask - A scheduled task.
///
/// Register the task in `src/schedule.rs` with the fluent API; the skeleton
/// below times its own run and prints a structured log line on each
/// invocation so it works end-to-end the first time you wire it up.
pub struct CleanupLogsTask;

impl CleanupLogsTask {
    /// Create a new instance of this task.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CleanupLogsTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        let started_at = Instant::now();
        println!("[CleanupLogsTask] task started");

        // Replace this with the real job. The skeleton ships as a
        // no-op success so the task can be scheduled and observed
        // before the implementation is filled in.

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

## Definindo agendamentos

O suprnova suporta duas abordagens para definir tarefas agendadas:

### 1. Tarefas baseadas em trait (recomendado)

Para tarefas complexas que precisam de dependências ou lógica
reutilizável, implemente o trait `Task` e configure o agendamento
durante o registro:

```rust
// src/tasks/cleanup_logs_task.rs
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::{Task, TaskResult};
use crate::models::Log;

pub struct CleanupLogsTask;

impl CleanupLogsTask {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        // O Eloquent funciona exatamente como dentro de um controlador; tarefas
        // veem as mesmas vinculações de contêiner (`DB::connection()`,
        // `App::get::<T>()`) que um handler de solicitação vê - veja
        // Inicialização da aplicação abaixo.
        let cutoff = Utc::now() - Duration::days(30);
        Log::query()
            .filter_op("created_at", "<", cutoff)
            .delete_all()
            .await?;

        println!("Old logs cleaned up successfully");
        Ok(())
    }
}
```

Então registre com a API fluente de agendamento em `src/schedule.rs`:

```rust
// src/schedule.rs
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

### 2. Tarefas baseadas em closure

Para tarefas rápidas e inline, sem arquivos separados:

```rust
// src/schedule.rs
use suprnova::Schedule;

pub fn register(schedule: &mut Schedule) {
    // Tarefa simples baseada em closure
    schedule.add(
        schedule.call(|| async {
            println!("Ping! Running every minute");
            Ok(())
        })
        .every_minute()
        .name("heartbeat")
    );

    // Tarefa configurada baseada em closure
    schedule.add(
        schedule.call(|| async {
            // Sua lógica de tarefa
            Ok(())
        })
        .daily()
        .at("09:00")
        .name("morning-report")
        .description("Sends daily morning report")
    );
}
```

## Registrando tarefas

Registre suas tarefas em `src/schedule.rs`:

```rust
// src/schedule.rs
use suprnova::Schedule;
use crate::tasks;

pub fn register(schedule: &mut Schedule) {
    // Tarefas baseadas em trait com configuração de agendamento fluente
    schedule.add(
        schedule.task(tasks::CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );

    schedule.add(
        schedule.task(tasks::SendRemindersTask::new())
            .daily()
            .at("09:00")
            .name("send:reminders")
            .description("Sends daily reminder emails")
    );

    schedule.add(
        schedule.task(tasks::BackupDatabaseTask::new())
            .weekly()
            .at("00:00")
            .name("backup:database")
            .description("Weekly database backup")
            .without_overlapping()
    );

    // Tarefas baseadas em closure
    schedule.add(
        schedule.call(|| async {
            println!("Quick task!");
            Ok(())
        })
        .hourly()
        .name("quick-task")
    );
}
```

## Opções de frequência de agendamento

O suprnova fornece uma API fluente para definir quando as tarefas
devem executar:

### Intervalos comuns

| Método | Descrição |
|--------|-------------|
| `.every_minute()` | Executa a cada minuto |
| `.every_two_minutes()` | Executa a cada 2 minutos |
| `.every_five_minutes()` | Executa a cada 5 minutos |
| `.every_ten_minutes()` | Executa a cada 10 minutos |
| `.every_fifteen_minutes()` | Executa a cada 15 minutos |
| `.every_thirty_minutes()` | Executa a cada 30 minutos |
| `.hourly()` | Executa a cada hora no minuto 0 |
| `.hourly_at(30)` | Executa a cada hora no minuto 30 |
| `.every_two_hours()` / `.every_three_hours()` / `.every_four_hours()` / `.every_six_hours()` | Executa na hora cheia a cada N horas |
| `.daily()` | Executa diariamente à meia-noite |
| `.daily_at("03:00")` | Executa diariamente às 3:00 |
| `.twice_daily(1, 13)` | Executa duas vezes ao dia (ex.: 1:00 e 13:00) |
| `.weekly()` | Executa semanalmente no domingo à meia-noite |
| `.monthly()` | Executa mensalmente no dia 1 à meia-noite |
| `.monthly_on(15)` | Executa mensalmente em um dia específico |
| `.quarterly()` | Executa no dia 1 de jan/abr/jul/out à meia-noite |
| `.yearly()` | Executa em 1º de janeiro à meia-noite |

### Agendamentos específicos por dia

```rust
use suprnova::DayOfWeek;

// Executa em dias específicos
.weekly_on(DayOfWeek::Monday)
.weekly_on(DayOfWeek::Friday)

// Métodos de atalho por dia
.sundays()
.mondays()
.tuesdays()
.wednesdays()
.thursdays()
.fridays()
.saturdays()

// Múltiplos dias
.days(&[DayOfWeek::Monday, DayOfWeek::Wednesday, DayOfWeek::Friday])

// Dias úteis/Fins de semana
.weekdays()  // Segunda a sexta
.weekends()  // Sábado e domingo
```

### Modificadores de horário

Encadeie `.at()` com qualquer agendamento para definir um horário
específico:

```rust
.daily().at("14:30")           // Diariamente às 14:30
.weekly().at("09:00")          // Semanalmente às 9:00
.mondays().at("08:00")         // Toda segunda às 8:00
.monthly().at("00:00")         // Primeiro dia do mês à meia-noite
```

### Fusos horários

Por padrão, o agendador lê toda expressão cron contra o fuso local do
processo, qualquer que seja o `TZ` com que o contêiner foi iniciado. Fixe
uma tarefa em um fuso IANA nomeado quando o agendamento dela pertencer a um
lugar, e não a um servidor:

```rust
use suprnova::chrono_tz;

schedule.add(
    schedule.task(GenerateReportTask::new())
        .daily()
        .at("02:00")
        .timezone(chrono_tz::America::New_York)
        .name("report:generate")
);
```

O `timezone` recebe um `chrono_tz::Tz` tipado, então um fuso escrito errado
é um erro de compilação, e não uma tarefa que roda em silêncio na hora
errada. As constantes de fuso vivem sob `suprnova::chrono_tz`
(`chrono_tz::Asia::Tokyo`, `chrono_tz::Europe::Berlin` e assim por diante),
reexportadas para que você não precise de `chrono-tz` no seu próprio
`Cargo.toml`.

Quando o nome do fuso só existe em runtime - um valor de configuração, uma
coluna de tenant -, use o irmão falível:

```rust
schedule.add(
    schedule.task(GenerateReportTask::new())
        .daily()
        .at("02:00")
        .try_timezone(&tenant.timezone)?   // Err(String) em um fuso desconhecido
        .name("report:generate")
);
```

Um fuso fixado muda exatamente uma coisa: contra qual relógio de parede os
cinco campos do cron são lidos. O agendador ainda dá um tick por minuto de
processo, e o gate de deduplicação do mesmo minuto não é afetado.

#### Um padrão para todo o agendamento

Se a maioria das suas tarefas pertence a um fuso de negócio, defina-o uma
vez no agendamento em vez de repeti-lo em toda tarefa:

```rust
pub fn register(schedule: &mut Schedule) {
    schedule.timezone(chrono_tz::America::Chicago);

    // Lido como 02:00 America/Chicago
    let nightly = schedule
        .call(|| async { Ok(()) })
        .daily()
        .at("02:00")
        .name("nightly");
    schedule.add(nightly);

    // Um fuso explícito por tarefa sempre ganha
    let tokyo = schedule
        .call(|| async { Ok(()) })
        .daily()
        .at("09:00")
        .timezone(chrono_tz::Asia::Tokyo)
        .name("tokyo-open");
    schedule.add(tokyo);
}
```

O padrão é aplicado quando uma tarefa é adicionada, então ele cobre as
tarefas registradas depois da chamada e deixa as anteriores em paz.

#### Horário de verão

Alguns fusos observam horário de verão. Quando os relógios mudam, uma
tarefa fixada em um fuso desses pode rodar duas vezes ou não rodar nenhuma:

- Ao atrasar o relógio, uma hora de relógio de parede acontece duas vezes.
  Uma tarefa às `01:30` casa nas duas passagens. São dois minutos
  diferentes de tempo real, então o gate de deduplicação do mesmo minuto
  não os funde e a tarefa roda duas vezes.
- Ao adiantar o relógio, uma hora de relógio de parede nunca acontece. Uma
  tarefa às `02:30` é pulada inteiramente naquele dia.

Evite agendamento com fuso horário onde puder, e prefira um fuso sem
horário de verão (`chrono_tz::UTC`) para qualquer coisa que precise rodar
exatamente uma vez.

#### Lendo a listagem em outro fuso

O `schedule:list` recebe `--timezone` e mostra tanto a expressão cron
quanto o próximo horário de execução como eles se leem naquele fuso. Veja
[Listar tarefas](#listar-tarefas) para uma saída trabalhada.

### Por que Suprnova diverge: fusos horários

O `timezone()` do Laravel recebe uma string e o padrão para todo o
agendamento vem de uma chave de configuração `app.schedule_timezone`. O
Suprnova recebe um `chrono_tz::Tz` tipado e não tem chave de configuração:
o `Schedule::timezone` na sua função `schedule::register` é o único lugar
em que um padrão é definido, então o agendamento se lê de cima a baixo sem
um segundo arquivo a consultar.

O padrão do Suprnova quando nada está fixado é o fuso local do processo, e
não um fuso horário de aplicação configurado. Esse é o comportamento que o
agendador sempre teve, e ele continua sendo o padrão para que acrescentar
este recurso não mude nada para agendamentos que não o usam.

### Expressões cron customizadas

Para controle total, use a sintaxe cron:

```rust
// Formato cron padrão: minuto hora dia-do-mês mês dia-da-semana
.cron("0 */2 * * *")    // A cada 2 horas
.cron("30 4 * * 1-5")   // 4:30 nos dias úteis
.cron("0 0 1,15 * *")   // Dia 1 e 15 de cada mês
```

`.cron(...)` **sofre panic** se a expressão está malformada (contagem
de campos errada, step/range/list que não parseiam). Use
`.try_cron(expr)` quando a expressão é fornecida em runtime
(configuração, input do usuário) e você prefere propagar o erro de
parse:

```rust
schedule.add(
    schedule.task(MyTask::new())
        .try_cron(env_expr)?   // retorna Err(String) em uma expressão inválida
        .name("from-config")
);
```

O mesmo par `panic` / `try_*` existe em todo método builder de range
numérico: `try_hourly_at`, `try_daily_at`, `try_twice_daily`,
`try_monthly_on`. As variantes infalíveis sofrem panic em numéricos
fora da faixa (ex.: `daily_at("25:00")` ou `monthly_on(40)`); os
irmãos falíveis retornam `Err(String)`.

## Configuração de tarefa

### Prevenindo sobreposição

Pula um tick quando uma execução anterior da mesma tarefa ainda está
em voo:

```rust
schedule.add(
    schedule.task(LongRunningTask::new())
        .daily()
        .name("long-task")
        .without_overlapping()
);
```

**Como o lock funciona.** Quando a flag está definida, o suprnova
tenta adquirir um mutex distribuído através do backend
[`Cache`](cache.md) configurado (`schedule:lock:<task-name>`). Uma
aquisição bem-sucedida executa a tarefa e libera o lock; uma aquisição
em contenção é reportada como um skip bem-sucedido - `Ok(())`, com o
contador de skip da tarefa incrementado para que superfícies de
observabilidade possam vê-lo sem envenenar o exit code de
`schedule:run`.

**Cache é obrigatório para proteção entre processos.** Se você executa
múltiplos processos que agendam a mesma tarefa (ex.: várias máquinas
invocando `suprnova schedule:run` a partir do cron do sistema, ou
daemons `schedule:work` atrás de um load balancer), o backend de Cache
é o que os coordena. **Sem um Cache configurado,
`without_overlapping()` degrada silenciosamente para um `AtomicBool`
por processo** - dois processos separados não vão ver os locks um do
outro. O framework emite um `WARN` único (`suprnova::schedule`) na
primeira vez que esse fallback dispara, para que operadores notem a
garantia mais fraca:

> `without_overlapping() falling back to in-process AtomicBool protection - Cache is not bootstrapped. Multi-process deployments will NOT see each other's locks. Configure Cache (CACHE_DRIVER=memory|redis) before relying on cross-process overlap protection.`

**TTL de lock customizado.** O TTL do lock tem padrão de 30 minutos -
longo o bastante para a maioria das tarefas terminar, curto o bastante
para que uma tarefa que deu crash segurando o lock desbloqueie o
próximo tick sem intervenção do operador. Sobrescreva por tarefa com
`.without_overlapping_for(Duration)`. `Duration::ZERO` é indefinido
entre backends de cache (Redis dá erro, em memória expira
instantaneamente, Memcached trata como "nunca expira"), então o
builder o força para o padrão de 30 minutos com um `WARN` único para
que o operador possa corrigir o call site.

```rust
use std::time::Duration;

schedule.add(
    schedule.task(SlowBackupTask::new())
        .daily()
        .name("backup:full")
        // Este job legitimamente executa por mais tempo que o padrão de 30
        // minutos; dê ao lock um TTL de 2 horas para que uma execução lenta
        // não seja preemptada pelo próximo tick.
        .without_overlapping_for(Duration::from_secs(2 * 3600))
);
```

### Executando em um único servidor

Executa uma tarefa exatamente uma vez por tick devido, não importa
quantas réplicas estejam executando o agendador:

```rust
schedule.add(
    schedule.task(NightlyBillingTask::new())
        .daily()
        .at("02:00")
        .name("billing:nightly")
        .on_one_server()
);
```

**O que dá errado sem isso.** Toda réplica executando `schedule:work`
avalia o agendamento independentemente, e nada impede todas elas de
decidirem que o mesmo tick é delas. Três réplicas foram medidas
produzindo três execuções da mesma tarefa, a cada minuto, sem
variação. Para um job de faturamento noturno isso significa que todo
cliente é cobrado três vezes.

**Por que `without_overlapping()` não cobre isso.** Os dois parecem
iguais e resolvem problemas diferentes:

| | Chave de lock | Mantido por | Previne |
|---|---|---|---|
| `without_overlapping()` | tarefa | a duração da tarefa | uma execução lenta sobrepor seu próprio próximo tick |
| `on_one_server()` | tarefa **+ o tick** | a janela do tick | uma segunda réplica executar o mesmo tick |

A distinção que importa é quando o lock é liberado.
`without_overlapping()` libera assim que o handler retorna - para uma
tarefa rápida, antes até de uma segunda réplica ter olhado, então
todas as N ainda executam. `on_one_server()` deliberadamente mantém
seu lock além do handler e deixa expirar no TTL, porque uma réplica
chegando mais tarde no mesmo tick precisa encontrá-lo ocupado.

Eles se compõem. Uma tarefa de longa duração que também precisa ser
single-server usa os dois.

**Exige um cache compartilhado.** A eleição é um lock de
[`Cache`](cache.md), então "um servidor" significa "um processo entre
os que compartilham um backend de cache". Sob `CACHE_DRIVER=memory` o
lock vive no heap de um único processo, toda réplica ganha sua própria
eleição, e a garantia está silenciosamente ausente.

Em produção isso é uma **falha de boot**, não um warning:

> `refusing to boot in production: 1 task(s) request single-server execution (billing:nightly) but CACHE_DRIVER is memory or unset, so the election lock lives in this process's heap. Every replica would win its own election and run the task, which is what on_one_server() exists to prevent. Set CACHE_DRIVER=redis with REDIS_URL, or set SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true to acknowledge per-process locking - which is only accurate if you run exactly one scheduler.`

Defina `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` se o seu
deployment realmente executa um único agendador. Fora de produção o
driver de memória permanece usável e o framework avisa uma vez em vez
disso.

**TTL de lock customizado.** O padrão é 60 segundos - um tick alinhado
ao minuto. As duas pontas importam: muito curto e uma réplica cujo
tick chega alguns segundos atrasado encontra o lock já sumido e
executa a tarefa de novo; muito longo e o lock sobrevive além do seu
tick, então a *próxima* execução devida o encontra ocupado e é pulada
completamente. Use `.on_one_server_for(Duration)` para agendamentos
mais grossos.

```rust
use std::time::Duration;

schedule.add(
    schedule.task(HourlyRollupTask::new())
        .hourly()
        .name("rollup:hourly")
        // Uma tarefa de hora em hora só precisa que o lock sobreviva à
        // janela na qual réplicas ainda poderiam considerar esse tick
        // devido.
        .on_one_server_for(Duration::from_secs(300))
);
```

**Se o cache está inalcançável**, o tick é pulado em vez de executado.
Perder a coordenação é o pior momento possível para deixar toda
réplica passar: um tick pulado é recuperável no próximo tick, efeitos
colaterais duplicados geralmente não são.

### Por que Suprnova diverge

O `onOneServer()` do Laravel é o mesmo opt-in, e o Suprnova mantém
isso: tarefas por servidor - rotação de log, aquecer um cache local -
são legítimas e continuam expressáveis.

Onde ele diverge é no modo de falha. O Laravel executa `onOneServer()`
alegremente contra um driver de cache que não consegue coordenar. O
Suprnova se recusa a inicializar em produção nesse caso, pelo mesmo
raciocínio do rate limiter em memória: um controle que silenciosamente
faz muito menos do que afirma é pior do que um que está visivelmente
ausente.

### Executando em background

Desacople tarefas do caminho crítico por tick para que elas não
bloqueiem outras tarefas devidas de começar:

```rust
schedule.add(
    schedule.task(BackgroundTask::new())
        .hourly()
        .name("background-task")
        .run_in_background()
);
```

**Isolamento de panic.** Tarefas em background executam dentro de um
`tokio::task::JoinSet` com `catch_unwind`, então uma tarefa que sofre
panic aparece como um `FrameworkError` registrado contra o nome da
tarefa em vez de derrubar o agendador. O daemon `schedule:work` drena
o JoinSet no shutdown (Ctrl-C / SIGTERM) para que tarefas de
background em voo completem antes de sair.

**Combine com `without_overlapping`.** As duas flags se compõem - uma
tarefa em background com `without_overlapping()` vai spawnar no
JoinSet e adquirir o lock de sobreposição de dentro da future
spawnada, então a semântica de lock descrita acima ainda se aplica.

### Dedupe no mesmo minuto

A resolução do cron é no nível de minuto, e o suprnova impõe isso: se
a mesma tarefa é solicitada a executar duas vezes dentro do mesmo
minuto de relógio de parede em um único processo, a segunda chamada é
um skip no-op - `Ok(())`, com o contador de skip da tarefa
incrementado. Isso fecha uma classe de bug em que um loop de daemon ou
uma invocação apertada de `schedule:run` poderia executar uma tarefa
`.every_minute()` múltiplas vezes no mesmo minuto.

Esse gate in-process está **sempre ativo**, independente de
`without_overlapping`. Ele NÃO abrange múltiplos processos (cada
processo tem seu próprio estado por tarefa). Se você precisa de
coordenação entre processos no mesmo minuto, adicione `without_overlapping` + um backend de Cache configurado - juntos eles
cobrem as duas direções.

## Executando o agendador

O suprnova fornece comandos de CLI para executar tarefas agendadas:

### Executar uma vez

Executa todas as tarefas devidas uma vez (tipicamente chamado pelo
cron a cada minuto):

```bash
suprnova schedule:run
```

### Modo daemon

Executa continuamente, verificando tarefas devidas a cada minuto:

```bash
suprnova schedule:work
```

Isso é ideal para desenvolvimento ou quando se usa um gerenciador de
processos como o systemd.

### Listar tarefas

Exibe todas as tarefas agendadas registradas:

```bash
suprnova schedule:list
```

Saída:
```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] next: 2026-05-29 03:00 UTC
  send:reminders [0 9 * * *] next: 2026-05-28 09:00 UTC
  report:generate [0 6 * * *] (UTC) next: 2026-05-29 06:00 UTC
```

Cada linha traz o nome da tarefa, a expressão cron, um rótulo de fuso
opcional, o próximo horário em que a tarefa dispara e a descrição da tarefa
se ela tiver uma.

O `next:` é o primeiro minuto depois de agora em que a expressão casa,
calculado no fuso em que a tarefa é avaliada e então mostrado no fuso da
listagem. Uma expressão que nunca pode casar (`0 0 30 2 *` nomeia uma data
que não existe) imprime `next: never`.

O fuso da listagem é UTC, a menos que você passe `--timezone`. O
`cleanup:logs` e o `send:reminders` acima não fixaram fuso, então as
expressões deles são impressas como escritas - o agendador as lê contra o
fuso local do processo, que não tem nome IANA de onde converter - e não
carregam rótulo de fuso. O `report:generate` fixou `America/New_York` e
pediu `02:00`, então a expressão dele é reescrita para o fuso da listagem e
rotulada com ele.

```bash
suprnova schedule:list --timezone=Asia/Tokyo
```

```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] next: 2026-05-29 12:00 JST
  send:reminders [0 9 * * *] next: 2026-05-28 18:00 JST
  report:generate [0 15 * * *] (Asia/Tokyo) next: 2026-05-29 15:00 JST
```

Uma tarefa pode ocupar várias linhas. Uma expressão que atravessa a
meia-noite no fuso da listagem precisa de uma linha de cron por lado,
porque nenhuma expressão de cinco campos sozinha descreve as duas:

```
  monday-digest [0 23 * * 1] (Asia/Tokyo) next: 2026-06-01 23:00 JST
  monday-digest [0 5 * * 2] (Asia/Tokyo) next: 2026-06-01 23:00 JST
```

O `next:` pertence à tarefa, não à linha, então ele se repete: as duas
linhas descrevem a mesma tarefa e a mesma execução por vir.

Algumas conversões são recusadas em vez de aproximadas, e a expressão
recusada é impressa exatamente como escrita, rotulada com o fuso da própria
tarefa. Uma conversão é recusada quando uma transição de horário de verão
cai entre as duas próximas execuções (nenhuma expressão única está certa
dos dois lados), quando uma virada de dia teria de mover juntos um dia do
mês restrito e um dia da semana restrito (o cron faz OR desses dois campos,
então deslocar os dois mudaria quais dias casam), ou quando uma virada
teria de decidir quantos dias tem fevereiro.

## Configuração de produção

### Usando cron

Adicione uma única entrada de cron para executar o agendador a cada
minuto:

```bash
* * * * * cd /path/to/your/project && suprnova schedule:run >> /dev/null 2>&1
```

**Coordenação entre processos.** Se você executa `schedule:run` a
partir do cron do sistema em mais de um host (ou junto com um daemon
`schedule:work`), tarefas com `.without_overlapping()` precisam de um
backend **Cache** configurado (`CACHE_DRIVER=redis` recomendado para
produção) para coordenar entre processos. Sem isso, a flag de
sobreposição degrada para proteção por processo e a mesma tarefa pode
executar em múltiplos hosts no mesmo minuto. Veja [Prevenindo
sobreposição](#prevenindo-sobreposição) acima para a semântica de lock
completa.

### Usando systemd

Crie um serviço systemd para o daemon do agendador:

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/path/to/suprnova schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

## Acessando o contexto do app

Tarefas agendadas têm acesso completo ao contexto da aplicação, assim
como controladores:

```rust
use async_trait::async_trait;
use suprnova::{App, Task, TaskResult};
use crate::actions::SendEmailAction;
use crate::models::User;

pub struct SendRemindersTask;

#[async_trait]
impl Task for SendRemindersTask {
    async fn handle(&self) -> TaskResult {
        // Eloquent: `.get()` retorna uma `Collection<User>` que você pode iterar.
        let users = User::query()
            .filter("reminder_enabled", true)
            .get()
            .await?;

        // Qualquer coisa vinculada em `bootstrap.rs` também é alcançável aqui.
        let send_email = App::get::<SendEmailAction>()
            .expect("SendEmailAction bound in bootstrap()");

        for user in users.iter() {
            send_email.execute(&user.email, "Daily Reminder").await?;
        }

        Ok(())
    }
}
```

## Organização de arquivos

A estrutura de arquivos recomendada para tarefas agendadas:

```
src/
├── tasks/
│   ├── mod.rs              # Reexporta todas as tarefas (atualizado automaticamente por make:task)
│   ├── cleanup_logs_task.rs
│   ├── send_reminders_task.rs
│   └── backup_database_task.rs
├── schedule.rs             # Registra tarefas (executado pelos comandos schedule:*)
├── bootstrap.rs
├── routes.rs
└── lib.rs                  # Declara `pub mod schedule;` + `pub mod tasks;`
cmd/
└── main.rs                 # Chama `.schedule(<crate>::schedule::register)`
```

**src/tasks/mod.rs:**
```rust
pub mod cleanup_logs_task;
pub mod send_reminders_task;
pub mod backup_database_task;

pub use cleanup_logs_task::CleanupLogsTask;
pub use send_reminders_task::SendRemindersTask;
pub use backup_database_task::BackupDatabaseTask;
```

## Conectando o agendador à sua aplicação

`make:task` conecta `.schedule(<crate>::schedule::register)` no seu
builder `Application` automaticamente. Se você constrói a chain
manualmente, a chamada relevante é em `Application`:

```rust
// cmd/main.rs (ou src/main.rs para o starter de api)
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)        // <- esta linha
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await;
```

Sem `.schedule(...)` todos os subcomandos `schedule:*` reportam que
nenhuma tarefa está registrada. `schedule:work` e `schedule:run`
também executam os mesmos drivers de runtime e `bootstrap_fn` que o
servidor HTTP, então observers, listeners, e vinculações de contêiner
registradas no boot ficam visíveis para seus handlers de tarefa
exatamente como ficam para controladores (veja [Inicialização da
aplicação](bootstrap.md)).

### Por que Suprnova diverge

O agendador do Laravel é ele mesmo um único comando Artisan
(`schedule:run`) que o PHP-cron dispara a cada minuto. O runtime PHP
sobe, avalia as tarefas devidas, executa-as in-process ou faz shell
out, depois derruba o runtime. O PHP não tem processos de longa
duração, então a forma de daemon (`schedule:work`) foi retroportada
pelo Lumen e vem no próprio Laravel como um workaround para sites sem
acesso ao crontab.

No Suprnova o daemon é de primeira classe. `schedule:work` executa
dentro de um runtime Tokio que já é de longa duração, então:

- **Tarefas em background (`run_in_background`) se compõem com o loop de tick.** O Laravel spawna um processo filho por tarefa em background; nós spawnamos em um `JoinSet` e expomos as conclusões no próximo tick ou no shutdown.
- **Shutdown gracioso é um braço de `tokio::select!`.** Ctrl-C / SIGTERM drena tarefas de background em voo antes de sair; tarefas in-process terminam sua chamada atual.
- **Dedupe no mesmo minuto é estado in-process.** Um atomic `last_run_minute` por tarefa garante que um único processo não consegue disparar duas vezes uma tarefa alinhada ao minuto mesmo que o loop tique rápido. O PHP não consegue fazer isso - todo tick de cron é um processo novo - e é por isso que o Laravel usa locks de filesystem como a única linha de defesa.

O `without_overlapping` apoiado em `Cache::lock` ainda existe para o
caso multi-processo (cron do sistema em múltiplos hosts, múltiplos
daemons `schedule:work` atrás de um load balancer). É o mesmo
mecanismo, só que em uma camada que o agendador nem sempre precisa.

## Resumo

| Funcionalidade | Uso |
|---------|-------|
| Criar tarefa | `suprnova make:task TaskName` |
| Baseado em trait | Implemente o trait `Task`, configure o agendamento durante o registro |
| Baseado em closure | `schedule.call(\|\| async { ... })` |
| Registrar tarefas | `schedule.add(schedule.task(...).daily().name("..."))` |
| Conectar ao app | `Application::new().schedule(schedule::register)` |
| Executar uma vez | `suprnova schedule:run` |
| Executar daemon | `suprnova schedule:work` |
| Listar tarefas | `suprnova schedule:list` |
| Prevenir sobreposição | `.without_overlapping()` (TTL de lock padrão de 30 min via backend Cache) |
| TTL de sobreposição customizado | `.without_overlapping_for(Duration)` |
| Background | `.run_in_background()` (isolado de panic via JoinSet) |
| Dedupe no mesmo minuto | Sempre ativo por processo; execuções puladas retornam `Ok(())` |
| Cron validado em runtime | `.try_cron(expr)` / `.try_daily_at(s)` / `.try_hourly_at(n)` |

## Próximos passos

- [Comandos de agendamento](cli-scheduling.md) - referência de CLI de `schedule:run` / `schedule:work` / `schedule:list`
- [Filas](queues.md) - para trabalho que deveria ser pego por um worker em vez de ticar em um relógio
- [Console](console.md) - `#[command]` para tarefas de operador de execução única (fora de uma agenda)
- [Cache](cache.md) - o backend que potencializa o `without_overlapping` entre processos
- [Inicialização da aplicação](bootstrap.md) - como `.schedule(...)` se conecta ao builder, e o que tarefas conseguem resolver a partir do contêiner

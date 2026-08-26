# Console

Cada projeto Suprnova vem com um binário `console` - o dispatcher de
comandos em runtime para tudo que precisa dos tipos compilados do app:
seeders de banco de dados, pruners, tarefas de manutenção de execução
única, qualquer coisa que você construiria com o `php artisan` do
Laravel. Comandos são ou structs tipadas que fazem `#[derive(Command)]`
(construído em cima de `clap::Parser`) ou fns async anotadas com
`#[command]`; o framework as coleta via `inventory` em tempo de link,
então adicionar um novo comando é um único arquivo, sem registro
central para editar. Este é o análogo Suprnova do `php artisan` -
mesmo script, mesmo processo, mesmo espaço de endereçamento, sai
quando o handler retorna.

## Início rápido

O formato recomendado usa `#[derive(clap::Parser, Command)]` para args
tipados:

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "greet", description = "Print a friendly greeting")]
pub struct Greet {
    #[arg(short, long, default_value = "world")]
    pub name: String,

    #[arg(long, default_value_t = false)]
    pub loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let prefix = if self.loud { "HELLO" } else { "Hello" };
        println!("{prefix}, {}!", self.name);
        Ok(())
    }
}
```

Coloque isso em `src/commands/greet.rs`, adicione `pub mod greet;` a
`src/commands/mod.rs`, e execute:

```bash
cargo run --bin console -- greet
# Hello, world!
cargo run --bin console -- greet --name Alice --loud
# HELLO, Alice!
cargo run --bin console -- greet --help
# (ajuda por comando gerada pelo clap, incluindo as flags tipadas)
```

Nenhum registro central para editar. `#[derive(Command)]` submete uma
`CommandEntry { name, description, clap_builder, handler }` via
inventory; o binário console chama
`suprnova::console::dispatch_argv_with_init(argv, init)`, que constrói
uma única árvore de parser clap a partir de cada entrada registrada,
executa a closure `init` de inicialização apenas quando um subcomando
real corresponde, e roteia o `ArgMatches` parseado para o handler
certo.

### O caminho mais simples: `Vec<String>` cru

Para comandos triviais que não precisam de args tipados, o attribute
`#[command]` em uma fn async também funciona:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "ping", description = "Smoke test")]
pub async fn ping(_args: Vec<String>) -> Result<(), FrameworkError> {
    println!("pong");
    Ok(())
}
```

Por baixo dos panos, os dois caminhos acabam no mesmo registro
`CommandEntry`; o formato cru só usa um subcomando clap com um
`trailing_var_arg` para capturar o argv dentro do `Vec<String>`.
Prefira o formato tipado para qualquer comando com argumentos - você
ganha `--help` por comando, parsing de valores, valores padrão, e
pares de flag curta/longa sem escrever um parser à mão.

## O binário do console

`suprnova new` faz scaffold de dois binários em todo novo projeto:

- **`<project>`** (`cmd/main.rs` ou `src/main.rs`) - o servidor HTTP,
  iniciado por `cargo run` ou `suprnova serve`. De longa duração;
  serve até ser morto.
- **`console`** (`src/bin/console.rs`) - o dispatcher de comandos em
  runtime. De execução única; sai quando o handler retorna.

O `main` do binário console é pequeno e previsível:

```rust
use std::process::ExitCode;

#[suprnova::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // Exponha a versão deste projeto via `--version` / `--help`.
    // env! resolve para a versão do app do usuário, não a do framework.
    suprnova::console::set_version(env!("CARGO_PKG_VERSION"));

    let argv: Vec<String> = std::env::args().collect();
    let result = suprnova::console::dispatch_argv_with_init(argv, || async {
        my_app::config::register_all();
        my_app::bootstrap::register().await;
    })
    .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
```

O Tokio executa no flavor `current_thread` - não há trabalho para
paralelizar entre núcleos em um comando de execução única, e o pool de
workers do runtime multi-thread seria apenas overhead.

Duas coisas a notar:

- **A inicialização é lazy.** A closure passada para
  `dispatch_argv_with_init` só executa quando o clap corresponde a um
  subcomando real registrado. `console --help`, `console --version`,
  subcomando ausente, e caminhos de erro de parse pulam todos essa
  etapa - então `console --help` funciona em um checkout novo que
  ainda não tem `DATABASE_URL` definida.
- **`main` não imprime erros.** `dispatch_argv_with_init` possui todo
  o stderr voltado ao usuário - ela faz `eprintln` da mensagem de erro
  do handler (a menos que o erro seja silencioso, como uma falha de
  parse do clap que o próprio clap já imprimiu) e imprime a própria
  saída de help / version / parse-error do clap. `main` é tradução
  pura de `Result → ExitCode`; adicionar um `eprintln!` redundante
  imprimiria em dobro.

Se você quer que um comando específico pule inteiramente uma etapa
cara de inicialização, condicione a própria etapa a uma env var em vez
de passar uma flag de "inicialização lazy" através do framework.

## Comandos embutidos

O próprio framework registra um pequeno conjunto de comandos. Linkar o
framework em um projeto os traz automaticamente.

| Comando       | O que faz                              |
|---------------|-------------------------------------------|
| `db:seed`     | Executa todo `Seeder` registrado, em ordem. Aceita `--class=<Name>` (ou um positional puro) para executar um único seeder nomeado, espelhando `php artisan db:seed --class=UserSeeder`. |
| `model:prune` | Percorre o registro `PrunerEntry` e force-deleta toda linha que cada escopo `Prunable` / `MassPrunable` registrado retornar. `--model=<Name>` restringe a um tipo; `--pretend` relata a contagem de linhas sem modificar nenhuma linha. |
| `--help` / `-h` | Lista os comandos disponíveis; o `--help` por subcomando é construído pelo clap a partir dos args tipados. |
| `--version`   | Imprime a versão registrada por `set_version` (tipicamente o `CARGO_PKG_VERSION` do seu app). Omitido inteiramente se `set_version` nunca foi chamado. |

`db:seed` executa o que quer que você tenha registrado em
`bootstrap::register()` com `suprnova::seed::register::<MySeeder>()`.
Em um registro vazio ele imprime um aviso e retorna `Ok(())` - invocar
`db:seed` antes de registrar seeders é um erro benigno do usuário, não
um erro de programador.

`db:seed` relata o progresso de uma execução direcionada usando
`suprnova::two_column_detail`, que renderiza um nome, uma linha
pontilhada e um status como uma única linha de 80 colunas. Seus
próprios comandos podem chamá-la para ter a mesma aparência.

> Os daemons de worker (`queue:work`, `schedule:run`, `schedule:work`,
> `schedule:list`, `workflow:work`) **não** estão no binário console.
> Eles vivem no parser clap do binário app/server (o mesmo binário que
> serve HTTP). A CLI global `suprnova` faz shell para
> `cargo run --quiet -- <name>` para esses. Veja a
> [seção de Assimetria](#assimetria-com-suprnova-migrate) abaixo.

## Definindo comandos

Duas macros, um registro. Escolha a que se encaixa no formato do
comando.

### `#[derive(Command)]` - args tipados (recomendado)

Vai em cima de `#[derive(clap::Parser)]`. Os campos da struct são os
args do comando; o clap parseia o argv dentro da struct; o framework
chama seu `TypedCommand::run(self)`.

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "users:purge", description = "Purge users older than N days")]
pub struct UsersPurge {
    #[arg(long)]
    pub older_than_days: u32,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for UsersPurge {
    async fn run(self) -> Result<(), FrameworkError> {
        // self.older_than_days, self.dry_run - tipados, validados pelo clap
        Ok(())
    }
}
```

Attributes:

| Attribute    | Obrigatório | Propósito                                       |
|--------------|----------|-----------------------------------------------|
| `#[console(name = "...")]` | sim | O nome de invocação na CLI (`"users:purge"`, `"mail:send"`, `"greet"`). |
| `#[console(description = "...")]` | não | Descrição de uma linha mostrada no help de nível superior. |
| `#[arg(...)]` (clap) | n/a | Os próprios attributes de campo do clap para flags curtas/longas, padrões, value parsers, etc. |

Você também ganha de graça o help por comando auto-gerado pelo clap
(`console users:purge --help`).

### `#[command]` - `Vec<String>` cru (casos simples)

Para comandos que não recebem argumentos ou só consomem positionals
como uma lista, o attribute em uma fn async já basta:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "cache:clear", description = "Drop every entry from the cache")]
pub async fn cache_clear(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::Cache::flush().await
}
```

A função anotada precisa ser
`async fn(Vec<String>) -> Result<(), FrameworkError>`. A macro
preserva a função original, então você também pode chamá-la
diretamente a partir do Rust - útil para testes unitários que não
querem passar strings de argv através do dispatcher.

Nomes nos dois formatos suportam namespacing no estilo Laravel:
`mail:send`, `queue:work`, `db:fresh`. Os dois-pontos são puramente
cosméticos - é uma string que o dispatcher confere contra `argv[1]`.

## `suprnova make:command`

O gerador da CLI cria um stub executável. O arquivo gerado usa o
**formato tipado** (`#[derive(Parser, Command)]` + `impl TypedCommand`) -
esse é o padrão recomendado, e ele te dá `--help` por comando de graça:

```bash
suprnova make:command cache:clear
# → src/commands/cache_clear.rs (pub struct CacheClear com #[console(name = "cache:clear")])
# → src/commands/mod.rs recebe `pub mod cache_clear;` acrescentado (criado se estiver ausente)
```

O stub é executável como está - `cargo run --bin console --
cache:clear` vai imprimir `cache:clear: not yet implemented` e
retornar `Ok(())` para que você possa conectá-lo e iterar. Preencha os
campos da struct para args tipados e substitua o corpo de
`TypedCommand::run`.

Normalização de nome:

| Entrada          | Arquivo              | Nome do comando   |
|----------------|-------------------|----------------|
| `greet`        | `greet.rs`        | `greet`        |
| `CleanCache`   | `clean_cache.rs`  | `clean-cache`  |
| `clean-cache`  | `clean_cache.rs`  | `clean-cache`  |
| `mail:send`    | `mail_send.rs`    | `mail:send`    |

Se a entrada contém `:`, o namespace com dois-pontos é preservado
literalmente. Caso contrário, o nome da fn Rust é snake_case e o nome
do comando é kebab-case.

Garanta que `pub mod commands;` esteja declarado em `src/lib.rs` para
que a submissão ao inventory seja alcançável por link a partir do
binário console. O gerador faz scaffold disso para novos projetos e
emite um aviso bem visível se estiver ausente; se você o removeu, o
bloco `inventory::submit!` do novo arquivo vai compilar mas nunca vai
parar no registro.

### Por que Suprnova diverge

O framework deliberadamente **não** faz um comando de CLI global
`suprnova` para tarefas em runtime como `db:seed`. Um binário global
não consegue carregar estaticamente os seeders, factories, ou fns
async `#[command]` do seu app sem:

- fazer shell para `cargo run --bin app -- ...` (lento - compilação
  completa por invocação, o que anula o propósito), ou
- carregamento dinâmico (complexidade demais para a v1)

Então o projeto do usuário produz um binário `console`. Execute-o
diretamente:

```bash
./target/debug/console db:seed
./target/release/console greet Alice
cargo run --bin console -- mail:send
```

O Laravel resolve o mesmo problema com `php artisan` - um script por
projeto que inicializa o framework e despacha para comandos definidos
pelo usuário. O PHP consegue fazer isso dinamicamente porque o código
do framework vive ao lado do código do usuário em runtime. O modelo de
compilar-e-linkar do Rust descarta essa opção, então distribuímos o
dispatcher como uma biblioteca (`suprnova::console::*`) e deixamos
cada projeto linkar seu próprio binário `console` de uma linha.

### Assimetria com `suprnova migrate`

Existem três caminhos distintos de invocação de comando em um projeto
Suprnova, e a assimetria é **estrutural** - não tente unificá-los:

| Superfície de comando                                   | Invocação                                              | Por quê                                                 |
|---------------------------------------------------|-----------------------------------------------------------|-------------------------------------------------------|
| `suprnova new`, `suprnova make:*`, `suprnova serve`, `suprnova key:generate`, … | Binário de CLI global (instalado via `cargo install --git`) | Geradores e scaffolders que só mexem em arquivos; não precisam do código do usuário. |
| `suprnova migrate`, `suprnova migrate:status`, `suprnova schedule:run`, `suprnova schedule:work`, `suprnova schedule:list`, `suprnova workflow:work` | A CLI global faz shell para `cargo run --quiet -- <name>` contra o binário app/server | Daemons de longa duração e trabalho de schema que pertencem ao mesmo parser clap de `Application::run`. O `queue:work` do binário server também vive aqui - `cargo run --bin <app> -- queue:work`. |
| `console db:seed`, `console model:prune`, `console <your-command>` | Binário `console` por projeto (`src/bin/console.rs`) | Comandos de execução única que precisam de tipos do usuário (seeders, commands, models prunable) compilados dentro do crate do usuário. |

A separação é intencional. O binário server já precisa de um parser
clap para escolher entre `serve`, `migrate`, `queue:work`, etc.;
daemons que compartilham seu ciclo de vida vivem ali. O binário
console existe para todo o resto - de vida curta, definido pelo
usuário, rico em tipos. Novos comandos em runtime pertencem a
`#[command]` / `#[derive(Command)]` despachados pelo binário `console`
do projeto.

## Boas práticas

### Mantenha handlers pequenos; busque serviços compartilhados através do contêiner

Um `#[command]` é o wrapper no formato de CLI; a lógica de negócio
deve viver em uma `Action`, um serviço, ou um método em um model. O
handler parseia os args, resolve o serviço a partir do contêiner, e
encaminha. Isso mantém a mesma lógica testável a partir de um teste
unitário, de uma rota HTTP, e do console.

```rust
#[command(name = "users:purge")]
pub async fn users_purge(args: Vec<String>) -> Result<(), FrameworkError> {
    let action = App::resolve::<PurgeStaleUsers>()?;
    action.execute(parse(args)?).await
}
```

`App::resolve` retorna `Result<T, FrameworkError::ServiceUnresolved(_)>` -
a variante com `?` de `App::get` (que retorna `Option`). Veja
[Contêiner de serviços](container.md) para a superfície completa.

### Use namespaces para comandos relacionados

Agrupe com `:`: `mail:send`, `mail:retry`, `mail:queue:work`. O
dispatcher trata isso como opaco, mas humanos escaneiam `mail:*`
melhor do que `send-mail`, `retry-mail`, `mail-queue-work`.

### Não imprima dados estruturados - retorne-os

Handlers de console imprimem no stdout para saída legível por humanos.
Se uma ferramenta downstream precisa consumir a saída, escreva uma
variante `console <name> --json` que emite JSON legível por máquina no
stdout e uma linha de status no stderr. Não torne o caminho legível
por humanos responsável pelas duas audiências.

### Trate os exit codes como o contrato

`FrameworkError` → `ExitCode::FAILURE` é o único caminho de falha. Não
faça `std::process::exit(custom_code)` de dentro de um handler -
retorne `Err(...)` e deixe o `main` do binário traduzir. Ferramentas
futuras (gates de CI, workers supervisionados) só precisam ler o exit
code.

## Referência

| Símbolo                                    | Propósito                                       |
|-------------------------------------------|-----------------------------------------------|
| `suprnova::Command` (derive)              | Registra uma struct que deriva `clap::Parser` como um comando de console tipado. Combina com `TypedCommand`. |
| `suprnova::TypedCommand` (trait)          | Trait com `async fn run(self) -> Result<(), FrameworkError>` - o corpo de um comando tipado. |
| `suprnova::command` (attribute)           | Registra uma fn async que recebe `Vec<String>` como um comando de console de args crus. |
| `suprnova::console::dispatch_argv(argv)`  | Constrói a árvore de parser clap a partir de cada entrada registrada, parseia o argv, roteia para o handler. Sem init lazy - conveniente para testes e chamadores programáticos. |
| `suprnova::console::dispatch_argv_with_init(argv, init)` | O mesmo que `dispatch_argv` mas executa a closure `init` entre o parse de argv do clap e o handler correspondido. O init só dispara quando um subcomando real corresponde - os caminhos de `--help` / `--version` / erro de parse o pulam. É isso que o binário `console` gerado por scaffold usa. |
| `suprnova::console::set_version(&'static str)` | Registra a string de versão exposta via `--version` e em `--help`. Chame uma vez no início do `main`. O primeiro registro vence. |
| `suprnova::console::find(name)`           | Procura um comando registrado pelo nome exato.   |
| `suprnova::two_column_detail(left, right)` | Renderiza um nome, uma linha pontilhada e uma palavra de status como uma única linha de progresso de 80 colunas. Espelha o `$this->components->twoColumnDetail(...)` do Laravel. |
| `suprnova::console::list()`               | Todos os comandos registrados, ordenados por nome.      |
| `suprnova::CommandEntry`                  | Registro de inventory: `{ name, description, clap_builder, handler }`. Submetido pelas duas macros. |
| `suprnova::CommandHandler`                | O tipo de function pointer do handler: `fn(&clap::ArgMatches) -> Pin<Box<dyn Future<...>>>`. |
| `FrameworkError::silent()` / `.is_silent()` | Constrói / detecta um erro que o dispatcher NÃO vai imprimir no stderr. Usado internamente para suprimir impressões em dobro quando o clap já escreveu um erro de parse no terminal. |

## Próximos passos

- [Inicialização da aplicação](bootstrap.md) - o que executa dentro da closure `dispatch_argv_with_init`
- [Contêiner de serviços](container.md) - `App::resolve` vs `App::get`, e como um handler alcança serviços compartilhados
- [Preenchimento de dados](seeding.md) - o que `db:seed` de fato invoca
- [API Eloquent](eloquent.md) - `Prunable`, `MassPrunable`, e como `model:prune` percorre o registro
- [Agendamento de tarefas](scheduling.md) - a assimetria: daemons do agendador vivem no binário app, não no console

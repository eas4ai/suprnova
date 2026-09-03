# Geradores de código

A família `suprnova make:*` faz scaffold do arquivo convencional para
cada peça de um projeto - um controlador, uma ação, um middleware, um
comando de console, um erro de domínio, uma tarefa agendada, uma
página Inertia ou struct de props, uma migração de banco de dados - e
conecta o novo módulo ao seu `mod.rs` pai (e, onde necessário,
`src/lib.rs` e `cmd/main.rs`). Recorra a eles quando você do contrário
estaria redigitando o mesmo boilerplate + a linha de import `pub mod x;`, o que é a maior parte do tempo.

## make:controller

Faz scaffold de um controlador - um arquivo em `src/controllers/` com
uma única fn async `#[handler]` chamada `invoke`.

```bash
suprnova make:controller User
suprnova make:controller order_item
```

O nome é normalizado para `snake_case` para o nome do arquivo e usado
como está para o eco `controller:` na resposta. Só letras ASCII,
dígitos, e `_` são aceitos - caminhos como `api/User` são rejeitados.

### Arquivo gerado

```rust
// src/controllers/user.rs
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn invoke(_req: Request) -> Response {
    json_response!({
        "controller": "User"
    })
}
```

### O que conecta

1. Escreve `src/controllers/<name>.rs` com a fn `#[handler]`.
2. Adiciona `pub mod <name>;` a `src/controllers/mod.rs` (cria o
   arquivo se ele não existia).
3. Imprime uma dica para adicionar uma rota em `src/routes.rs`:
   `.get("/<name>", controllers::<name>::invoke)`.

Veja [Controladores](controllers.md) para o contrato de handler,
extractors, e a macro `routes!`.

---

## make:action

Faz scaffold de uma ação de responsabilidade única - uma struct
resolvível pelo contêiner com um método async `execute` que retorna
um `Result<String, FrameworkError>` para que o esqueleto compile
antes de você preencher o corpo.

```bash
suprnova make:action CreateUser
suprnova make:action SendNotification
```

O nome é PascalCased; um sufixo `Action` é acrescentado se estiver
faltando, e o arquivo é o nome da struct em snake-case.

### Arquivo gerado

```rust
// src/actions/create_user_action.rs
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct CreateUserAction {
    // Add injected dependencies as fields here, e.g.
    // db: suprnova::DbConnection,
}

impl CreateUserAction {
    pub async fn execute(&self) -> Result<String, FrameworkError> {
        Ok("CreateUserAction executed".to_string())
    }
}
```

### O que conecta

1. Escreve `src/actions/<snake>.rs`.
2. Adiciona `pub mod <snake>;` a `src/actions/mod.rs`.
3. `#[injectable]` registra a ação no contêiner em tempo de link,
   então qualquer controlador pode resolvê-la via
   `App::get::<CreateUserAction>()` e chamar `action.execute().await?`.

Veja [Ações](actions.md) para o padrão de resolver e invocar e como
as ações se compõem com o contêiner.

---

## make:middleware

Faz scaffold de um middleware - uma unit struct que implementa
`suprnova::Middleware`. O corpo padrão cronometra o handler interno e
loga os eventos de entrada + saída com o id por solicitação, então
ele executa de ponta a ponta na primeira vez.

```bash
suprnova make:middleware Auth
suprnova make:middleware RateLimit
```

O nome é PascalCased; um sufixo `Middleware` é acrescentado se
estiver faltando. O arquivo usa o nome base em snake-case (sem o
sufixo), por exemplo `Auth` → `src/middleware/auth.rs`, struct
`AuthMiddleware`.

### Arquivo gerado

```rust
// src/middleware/auth.rs
use std::time::Instant;

use suprnova::{async_trait, current_request_id, Middleware, Next, Request, Response};

pub struct AuthMiddleware;

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let method = request.method().to_string();
        let path = request.path().to_string();
        let request_id = current_request_id()
            .map(|id| id.as_str().to_string())
            .unwrap_or_default();
        let started_at = Instant::now();

        println!(
            "[AuthMiddleware] --> {} {} (request_id={})",
            method, path, request_id,
        );

        let response = next(request).await;

        println!(
            "[AuthMiddleware] <-- {} {} ({} ms, request_id={})",
            method, path, started_at.elapsed().as_millis(), request_id,
        );

        response
    }
}
```

### O que conecta

1. Escreve `src/middleware/<snake>.rs`.
2. Adiciona `mod <snake>;` + `pub use <snake>::<StructName>;` a
   `src/middleware/mod.rs` (cria se necessário).
3. Imprime tanto a forma por rota
   (`.get("/path", handler).middleware(AuthMiddleware)`) quanto a
   forma global (`global_middleware!(middleware::AuthMiddleware)` em
   `bootstrap.rs`).

Veja [Middleware](middleware.md) para a semântica completa da chain,
a ordenação, e a distinção entre global e por rota.

---

## make:command

Faz scaffold de um comando de console - uma struct
`#[derive(clap::Parser, Command)]` que o binário `console` por
projeto recolhe via `inventory` em tempo de link. O corpo padrão é um
`println!("…: not yet implemented")` para que o comando execute
imediatamente.

```bash
suprnova make:command CleanCache
suprnova make:command mail:send
suprnova make:command clean-cache
```

A nomenclatura segue três regras:

- Entradas contendo `:` são usadas ao pé da letra como o nome do
  comando registrado (estilo namespace do Laravel: `db:seed`,
  `mail:send`).
- Caso contrário, o nome da fn em snake-case é convertido para
  kebab-case para o nome registrado (`CleanCache` → comando
  `clean-cache`).
- O arquivo e a struct Rust são sempre as formas em snake-case /
  PascalCased do mesmo identificador.

### Arquivo gerado

```rust
// src/commands/clean_cache.rs
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "clean-cache", description = "TODO: describe what clean-cache does")]
pub struct CleanCache {
    // Add clap-derive args here.
}

#[async_trait]
impl TypedCommand for CleanCache {
    async fn run(self) -> Result<(), FrameworkError> {
        println!("clean-cache: not yet implemented");
        Ok(())
    }
}
```

### O que conecta

1. Escreve `src/commands/<snake>.rs`.
2. Adiciona `pub mod <snake>;` a `src/commands/mod.rs` (cria se
   necessário).
3. Avisa de forma evidente se `src/lib.rs` estiver sem
   `pub mod commands;` - o comando não vai linkar no binário console
   sem isso.
4. Imprime o comando de execução:
   `cargo run --bin console -- clean-cache`.

Veja [Console](console.md) para a superfície completa de comando
tipado, a forma curta `#[command]` para handlers somente de argv, e o
papel do binário console por projeto.

---

## live:make

Gera um componente Live: uma ilha pertencente ao servidor cujas ações tipadas
chegam pelo protocolo Live e cuja view re-renderizada é transformada no lugar pelo
runtime de navegador distribuído.

```bash
suprnova live:make Counter
suprnova live:make todo-list
suprnova live:make Counter --dry-run
```

Os nomes precisam ser identificadores ASCII simples em qualquer das formas
`Counter`, `TodoList`, `todo-list` ou `todo_list`; o arquivo e o módulo ficam em
snake_case, a struct em PascalCase e o nome registrado do componente é
`<package>.<kebab>` (para um pacote chamado `demo-app`: `demo-app.counter`).
Palavras-chave do Rust, separadores, pontos e entrada não ASCII são rejeitados
antes de qualquer escrita.

### Arquivo gerado

```rust
// src/live/counter.rs
use suprnova::live::{LiveComponent, live};

/// A counter island rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "demo-app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

```html
<!-- templates/live/counter.html -->
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

### O que ele conecta

1. Valida primeiro cada caminho de destino e recusa traversal e links simbólicos;
   se o arquivo do componente ou a view já existirem, avisa e não escreve nada.
2. Escreve `src/live/<snake>.rs` e `templates/live/<snake>.html` atomicamente; se
   qualquer escrita falhar, cada arquivo que a execução criou ou alterou é revertido, e
   qualquer arquivo que não pôde ser restaurado é nomeado no erro em vez de ser
   relatado como intacto.
3. Insere `pub mod <snake>;` e `.register::<snake::Pascal>()?` no builder
   `registry()` em `src/live/mod.rs`. Todo projeto criado por `suprnova new` traz esse
   módulo com um registro vazio, uma função `routes()` que instala as rotas Live
   reservadas com guarda e um bootstrap que vincula o registro; um projeto mais
   antigo recebe o mesmo módulo no primeiro uso.
4. Adiciona `pub mod live;` a `src/lib.rs` quando estiver faltando.
5. Imprime a linha de bootstrap que vincula o registro e, em seguida, o comando de
   verificação: `suprnova live:check`.

Em um projeto anterior ao módulo Live, vincule o registro durante o bootstrap e
instale as rotas a partir de `cmd/main.rs` manualmente:

```rust
suprnova::App::singleton(crate::live::registry().expect("Live registry"));
```

```rust
.try_routes(|| live::routes(routes::register()))
```

---

## make:error

Faz scaffold de um erro de domínio - uma unit struct anotada com
`#[domain_error]` para que ela carregue um status HTTP, uma mensagem
`Display`, e um impl `From<…> for FrameworkError` de fábrica.

```bash
suprnova make:error UserNotFound
suprnova make:error PaymentFailed
```

O nome é PascalCased para a struct e snake-case para o arquivo. O
status padrão é 500 e a mensagem é o nome da struct em sentence-case -
mude os dois attributes no arquivo gerado para corresponder à
situação.

### Arquivo gerado

```rust
// src/errors/user_not_found.rs
use suprnova::domain_error;

#[domain_error(status = 500, message = "User not found")]
pub struct UserNotFound;
```

Mude `status = 500` para o que se encaixar - `404` para not-found,
`402` para payment-required, `403` para forbidden - e edite a string
da mensagem. Para payloads mais ricos, adicione campos nomeados à
struct e referencie-os na mensagem via interpolação em um impl
`Display` escrito à mão (abandone a macro `#[domain_error]` nesse
ponto).

### O que conecta

1. Escreve `src/errors/<snake>.rs`.
2. Adiciona `pub mod <snake>;` a `src/errors/mod.rs` (cria se
   necessário).
3. Avisa sobre declarar `mod errors;` em `src/lib.rs` se o diretório
   `errors/` foi criado do zero.

### Usando-o

Dentro de um handler que retorna `Response`, eleve o tipo de domínio
a um `FrameworkError` para que `?` faça short-circuit de forma
limpa:

```rust
use crate::errors::user_not_found::UserNotFound;
use suprnova::FrameworkError;

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;
    let user = find_user(id).await
        .ok_or_else(|| FrameworkError::from(UserNotFound))?;
    json_response!({ "user": user })
}
```

O capítulo [Erros](errors.md) cobre a história completa de erro
customizado, incluindo quando usar `#[domain_error]` vs
`AppError::bad_request(…)` vs um impl `HttpError` escrito à mão.

---

## make:task

Faz scaffold de uma tarefa agendada - uma unit struct que implementa
`suprnova::Task` e imprime linhas estruturadas de início/fim para que
o scaffold logue o progresso antes de você preencher o corpo de
verdade.

```bash
suprnova make:task CleanupLogs
suprnova make:task SendReminders
```

O nome é PascalCased; um sufixo `Task` é acrescentado se estiver
faltando. O arquivo é o nome da struct em snake-case, por exemplo
`CleanupLogs` → `src/tasks/cleanup_logs_task.rs`.

### Arquivo gerado

```rust
// src/tasks/cleanup_logs_task.rs
use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

pub struct CleanupLogsTask;

impl CleanupLogsTask {
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

        // Replace this with the real job.

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

### O que conecta

A primeira invocação de `make:task` faz uma fiação mais pesada do que
os outros geradores - ela cria a superfície do agendador no projeto
do zero:

1. Cria `src/tasks/` e `src/tasks/mod.rs` se estiverem faltando.
2. Cria `src/schedule.rs` (o entrypoint
   `register(schedule: &mut Schedule)`) se estiver faltando.
3. Declara `pub mod schedule;` e `pub mod tasks;` em `src/lib.rs`.
4. Insere `.schedule(<crate>::schedule::register)` na chain
   `Application::new()` em `cmd/main.rs` ou `src/main.rs`,
   imediatamente antes de `.run()`.
5. Escreve `src/tasks/<snake>.rs` e o adiciona a `src/tasks/mod.rs`.

Invocações subsequentes pulam as etapas que já executaram.

### Registrando a tarefa

Abra `src/schedule.rs` e adicione uma chamada de registro com a API
fluente de agendamento:

```rust
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes old log files daily"),
    );
}
```

Depois execute o agendador:

```bash
suprnova schedule:work   # daemon - verifica a cada minuto
suprnova schedule:run    # de execução única - tipicamente chamado pelo cron
suprnova schedule:list   # mostra toda tarefa registrada
```

Veja [Agendamento](scheduling.md) para a superfície completa de
tarefa (`hourly`, `weekly`, `cron(...)`, `between`, `when`,
`without_overlapping`, tratamento de timezone) e
[Comandos de agendamento](cli-scheduling.md) para o trade-off entre
executar como cron e executar como daemon.

---

## make:inertia

Faz scaffold de um componente de página Inertia (padrão) ou de uma
struct Data tipada (`--data`), dependendo da flag. O gerador de
página detecta o framework de frontend (Svelte 5, React 19, Vue 3.5)
a partir do `.env` e emite a extensão de arquivo correspondente.

### Modo de página (padrão)

```bash
suprnova make:inertia About
suprnova make:inertia UserProfile
```

O nome é PascalCased e o sufixo `Page` é acrescentado se estiver
faltando, então `About` → `AboutPage`. O arquivo cai em
`frontend/src/pages/` com a extensão por frontend: `AboutPage.svelte`
para Svelte, `AboutPage.tsx` para React, `AboutPage.vue` para Vue.

Exemplo (Svelte):

```svelte
<!-- frontend/src/pages/AboutPage.svelte -->
<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">AboutPage</h1>
  <p class="mt-2">
    Edit <code class="bg-gray-100 px-1 rounded">frontend/src/pages/AboutPage.svelte</code> to get started.
  </p>
</div>
```

Renderize a partir de um controlador:

```rust
inertia_response!(&req, "AboutPage", props)
```

Veja [Páginas do frontend](frontend-pages.md) e
[Respostas Inertia](frontend-inertia-responses.md) para a ponte entre
controladores e páginas, reloads parciais, e props compartilhadas.

### Modo de struct Data (`--data`)

```bash
suprnova make:inertia UserProps --data
```

Emite uma struct `#[derive(Data, Validate)]` em `app/src/props/`
(não `src/props/` - o prefixo `app/` é hardcoded para que o arquivo
caia no app de exemplo/host do workspace):

```rust
// app/src/props/user_props.rs
use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UserProps {
    pub id: i64,
    // Add fields here.
    //
    // Available field attributes:
    //   #[data(input_only)] - accepted on Deserialize, omitted from Serialize
    //   #[data(output_only)] - rejected on Deserialize, included in Serialize
    //   #[data(allow_include)] - registers as ?include=-eligible (default-deny)
    //
    // For PATCH endpoints, use suprnova::data::Field<T> to distinguish
    // absent from null. For lazy outbound fields, use suprnova::inertia::Prop<T>.
}
```

Use isso em um controlador para validar corpos de solicitação:

```rust
let dto: UserProps = req.validate_json().await?;
```

---

## make:migration

Faz scaffold de um arquivo de migração SeaORM com timestamp. Coberto
em detalhe em [Migrações da CLI](cli-migrations.md), que também
percorre os comandos `migrate` / `migrate:rollback` /
`migrate:status` / `migrate:fresh` / `db:sync`. A forma curta:

```bash
suprnova make:migration create_users_table
```

O nome da migração é preservado ao pé da letra e prefixado com uma
marca `YYYYMMDDHHMMSS_` para que os arquivos ordenem
cronologicamente. O arquivo gerado cai em `migrations/`.

Veja [Migrações](migrations.md) para a superfície do construtor de
esquema e [Testes de banco de dados](database-testing.md) para o
padrão `TestDatabase::fresh` que executa migrações contra um banco de
dados isolado por teste.

---

## generate-types

Emite interfaces TypeScript a partir de toda struct Rust anotada com
`#[derive(InertiaProps)]`. O servidor de dev executa isso
automaticamente; o comando standalone é para verificações de CI e
regenerações de execução única.

```bash
suprnova generate-types [--output <PATH>] [--watch]
```

| Opção | Padrão | Descrição |
|---|---|---|
| `-o, --output <PATH>` | `frontend/src/types/inertia-props.ts` | Caminho do arquivo de saída |
| `-w, --watch` | desligado | Monitora os arquivos de origem e regenera na mudança |

```bash
# Execução única
suprnova generate-types

# Modo watch (útil quando você não quer executar o servidor de dev completo)
suprnova generate-types --watch

# Caminho de saída customizado
suprnova generate-types --output frontend/src/types/props.ts
```

Uma forma Rust à esquerda produz uma interface TypeScript à direita:

```rust
#[derive(InertiaProps)]
pub struct UserPageProps {
    pub user: User,
    pub posts: Vec<Post>,
}
```

```typescript
export interface UserPageProps {
    user: User;
    posts: Post[];
}
```

Veja [Tipos TypeScript do frontend](frontend-typescript-types.md)
para a tabela de mapeamento completa (enums, options, datas, structs
aninhadas) e os hooks de override.

---

### Por que Suprnova diverge

O `php artisan make:*` do Laravel coloca um arquivo no diretório
certo e é isso - o autoloading PSR-4 pega a nova classe na próxima
vez que o framework inicializa. O Rust não tem equivalente. Um
arquivo em `src/foo/bar.rs` não é compilado no crate até
`src/foo/mod.rs` declarar `pub mod bar;`, e o diretório pai precisa
ser conectado da mesma forma em `src/lib.rs`.

Então todo gerador `suprnova make:*` faz duas coisas em vez de uma:
ele escreve o novo arquivo *e* edita o `mod.rs` mais próximo (e, para
`make:task` e `make:command`, também `src/lib.rs` e `cmd/main.rs`).
É por isso que todo gerador imprime uma linha `Created src/.../mod.rs`
ou `Updated src/.../mod.rs` - a fiação é parte do trabalho, não uma
etapa de acompanhamento que você lembra por conta própria.

---

## Resumo

| Comando | Cria | Conecta a |
|---|---|---|
| `make:controller <name>` | `src/controllers/<snake>.rs` | `controllers/mod.rs` |
| `make:action <Name>` | `src/actions/<snake>_action.rs` | `actions/mod.rs` |
| `make:middleware <Name>` | `src/middleware/<snake>.rs` | `middleware/mod.rs` |
| `make:command <name>` | `src/commands/<snake>.rs` | `commands/mod.rs` (+ avisa sobre `lib.rs`) |
| `make:error <Name>` | `src/errors/<snake>.rs` | `errors/mod.rs` |
| `make:task <Name>` | `src/tasks/<snake>_task.rs` | `tasks/mod.rs`, `schedule.rs`, `lib.rs`, `main.rs` |
| `make:inertia <Name>` | `frontend/src/pages/<Name>Page.<ext>` | (sem fiação de módulo) |
| `make:inertia <Name> --data` | `app/src/props/<snake>.rs` | (sem fiação de módulo) |
| `make:migration <name>` | `migrations/YYYYMMDDHHMMSS_<name>.rs` | (sem fiação de módulo) |
| `generate-types` | `frontend/src/types/inertia-props.ts` | n/a |

## Próximos passos

- [Visão geral da CLI](cli.md) - a tabela completa de subcomandos
- [Console](console.md) - o binário console por projeto que
  `make:command` alimenta
- [Controladores](controllers.md) - o contrato de handler que
  `make:controller` faz scaffold
- [Agendamento](scheduling.md) - a API fluente de agendamento usada
  para registrar tarefas geradas por `make:task`
- [Migrações da CLI](cli-migrations.md) - os comandos migrate /
  db:sync que combinam com `make:migration`

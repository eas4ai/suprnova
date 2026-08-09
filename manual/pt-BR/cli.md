# Visão geral da CLI

O Suprnova distribui dois binários com funções diferentes. O
`suprnova` global - instalado uma vez em `~/.cargo/bin` - faz scaffold
de novos projetos, gera código, inicializa servidores de dev, e
executa migrações. O `console` por projeto, construído a partir do
`src/bin/console.rs` de cada app, executa comandos em runtime que
precisam dos tipos compilados do app (seeders, pruners, seus próprios
handlers `#[command]`). Este capítulo é o mapa; cada subcomando tem
seu próprio mergulho profundo nos capítulos vizinhos listados em
[Próximos passos](#próximos-passos).

## Instale

A CLI é distribuída via `cargo install --git`. O Suprnova ainda não
está no crates.io - veja a [nota de pré-lançamento em
Instalação](installation.md#pre-launch-note) para saber por quê.

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.0 suprnova-cli
suprnova --version
```

Para atualizar depois, passe `--force`:

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.2.0 suprnova-cli
```

## Os dois binários

| Binário | Construído a partir de | Usado para |
|---|---|---|
| `suprnova` | `suprnova-cli/` (este crate) | Scaffold (`new`), geradores (`make:*`), executor de dev (`serve`), migrações (`migrate*`, `db:sync`), configuração Docker (`docker:*`), worker SSR (`ssr:*`), geração de chaves (`key:generate`), geração de tipos (`generate-types`) |
| `console` | `src/bin/console.rs` no seu projeto | Comandos em runtime que linkam os tipos do seu app - `db:seed` e `model:prune` embutidos, mais todo `#[command]` / `#[derive(Command)]` que você definir |

Daemons de worker (`schedule:run`, `schedule:work`, `schedule:list`,
`workflow:work`, `queue:work`) ficam em uma terceira superfície: o
próprio parser clap do binário do seu *app*, o mesmo binário que
serve HTTP. O `suprnova` global faz shell para
`cargo run --quiet -- <name>` para esses, para que você possa
iniciá-los a partir da CLI que você já tem aberta. Veja
[Console](console.md) para a divisão completa em três vias.

### Por que Suprnova diverge

O Laravel resolve isso com um único script por projeto -
`php artisan` - porque o PHP carrega o framework e o código do
usuário juntos em runtime. O Rust linka binários em tempo de
compilação, então um binário `suprnova` global não consegue ver
estaticamente seus seeders, factories, ou handlers `#[command]`. A
divisão pragmática:

- Trabalho somente de arquivo (scaffold, geradores, ops) vive no
  binário `suprnova` global
- Trabalho em runtime que precisa dos seus tipos compilados vive no
  binário `console` por projeto
- Daemons vivem no seu binário de app/server para que compartilhem o
  mesmo caminho de boot que o `serve`

Você ganha a ergonomia do `php artisan` (`cargo run --bin console --
db:seed` ou `console <name>` diretamente) sem a mentira do linkamento
estático.

## Comandos de uma olhada

A mesma lista que `suprnova --help` imprime, agrupada da mesma forma.

### Criar

| Comando | Descrição |
|---|---|
| `suprnova new [name]` | Faz scaffold de um novo projeto. Veja [`suprnova new`](cli-new.md). |
| `suprnova serve` | Inicializa backend + Vite juntos com hot reload. Veja [`suprnova serve`](cli-serve.md). |
| `suprnova dev:tls` | Confia na CA do portless e registra uma URL de dev `https://<name>.localhost`. Veja [URLs HTTPS de Dev](dev-tls.md). |
| `suprnova web:run` | Executa o binário do app diretamente (sem Vite, sem loop de recompilação). Execução local com a forma da produção. |

### Gerar

| Comando | Descrição |
|---|---|
| `suprnova make:controller <name>` | Faz scaffold de um controlador em `src/controllers/`. |
| `suprnova make:action <name>` | Faz scaffold de uma ação invocável em `src/actions/`. |
| `suprnova make:middleware <name>` | Faz scaffold de um middleware em `src/middleware/`. |
| `suprnova make:migration <name>` | Faz scaffold de uma migração SeaORM em `src/migrations/`. |
| `suprnova make:inertia <name>` | Faz scaffold de uma página Inertia em `frontend/src/pages/`. Passe `--data` para uma struct de props `#[derive(Data, Validate)]` em `src/props/` em vez disso. |
| `suprnova make:error <name>` | Faz scaffold de um erro de domínio em `src/errors/`. |
| `suprnova make:task <name>` | Faz scaffold de uma tarefa agendada em `src/tasks/`. |
| `suprnova make:command <name>` | Faz scaffold de um comando de console `#[derive(Command)]` em `src/commands/`. |
| `suprnova generate-types` | Emite tipos TypeScript a partir de cada struct `#[derive(InertiaProps)]`. `-o <path>` para sobrescrever a saída, `-w` para monitorar e regenerar. |

Veja [Geradores](cli-generators.md) para os detalhes completos de
scaffold e como é cada arquivo gerado.

### Banco de dados

| Comando | Descrição |
|---|---|
| `suprnova migrate` | Executa todas as migrações pendentes. |
| `suprnova migrate:status` | Mostra quais migrações estão aplicadas e quais estão pendentes. |
| `suprnova migrate:rollback [--step N]` | Reverte as últimas N migrações (padrão 1). |
| `suprnova migrate:fresh [--force]` | Derruba todas as tabelas e executa todas as migrações de novo. **Destrutivo.** Em produção precisa de `--force` mais uma confirmação digitada em um terminal interativo. |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | Executa as migrações e regenera as entidades SeaORM a partir do esquema ativo. `--regenerate-models` sobrescreve os arquivos de model customizados em `src/models/`. |

`db:seed` **não** está aqui - ele vive no binário `console` por
projeto porque o registro de seeders é compilado dentro do seu crate.
Execute-o via `cargo run --bin console -- db:seed` ou
`./target/debug/console db:seed`. Veja [Console](console.md) para o
padrão de registro.

Veja o [capítulo de Migrações](cli-migrations.md) para o fluxo de
trabalho completo de migração.

### Agendamento

| Comando | Descrição |
|---|---|
| `suprnova schedule:run` | Executa todas as tarefas devidas uma vez. A forma amigável para cron. |
| `suprnova schedule:work` | Daemon em foreground que verifica a cada minuto e executa as tarefas devidas. |
| `suprnova schedule:list` | Imprime toda tarefa registrada com sua expressão cron. |

Cada um desses faz shell para `cargo run --quiet -- <name>` contra o
binário do seu app/server - o mesmo binário que serve HTTP - para que
tarefas registradas e serviços inicializados no boot fiquem visíveis.
Veja [CLI de Agendamento](cli-scheduling.md) e o capítulo de
[Agendamento](scheduling.md).

### Fluxo de trabalho

| Comando | Descrição |
|---|---|
| `suprnova workflow:work` | Inicia o daemon worker de fluxo de trabalho. Retira passos de fluxo de trabalho do registro e os executa com o mesmo limite de panic dos handlers HTTP. |
| `suprnova workflow:install` | Coloca as migrações workflow + workflow_steps em `src/migrations/`. Já presentes em scaffolds novos. |

Veja [Fluxos de trabalho](workflows.md).

### SSR

| Comando | Descrição |
|---|---|
| `suprnova ssr:start [--runtime node\|bun\|deno] [--bundle <path>]` | Inicia o worker SSR do Inertia em foreground. Recai para a env `SUPRNOVA_SSR_RUNTIME`, depois `node`; o bundle recai para `SUPRNOVA_SSR_BUNDLE`, depois `frontend/bootstrap/ssr/ssr.js`. |
| `suprnova ssr:check [--url <url>] [--timeout-ms N]` | Sonda o worker SSR. Recai para `SUPRNOVA_SSR_URL`, depois `http://127.0.0.1:13714`. Timeout padrão de 2000 ms. |

Veja [SSR do Inertia](frontend.md) para a configuração de produção.

### Implantação

| Comando | Descrição |
|---|---|
| `suprnova docker:init` | Emite um `Dockerfile` multi-estágio de produção + `.dockerignore`. |
| `suprnova docker:compose [--with-mailpit] [--with-minio]` | Emite um `docker-compose.yml` para desenvolvimento local. Postgres + Redis sempre incluídos; Mailpit e MinIO são opcionais. |

Veja [Docker](cli-docker.md) e o capítulo de
[Implantação](deployment.md).

### Segurança

| Comando | Descrição |
|---|---|
| `suprnova key:generate [--show]` | Gera uma chave AES-256 de 32 bytes, base64 URL-safe sem padding (o mesmo formato de rede que `EncryptionKey::to_base64` produz). `--show` imprime só a chave para `APP_KEY=$(suprnova key:generate --show)`. |

Veja [Criptografia](encryption.md) para o que `APP_KEY` protege e como
a rotação via `APP_KEY_PREVIOUS` funciona.

## Início rápido

O caminho mais comum de "nada instalado" até "app em execução":

```bash
# 1. Instale a CLI
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.0 suprnova-cli

# 2. Faça scaffold de um projeto (interativo - escolhe Svelte por padrão)
suprnova new my-app

# 3. Inicialize-o
cd my-app
suprnova migrate
npm install
suprnova serve
```

Scaffold não interativo (CI, configuração via script):

```bash
suprnova new my-app \
  --frontend svelte \
  --no-interaction \
  --no-git
```

Scaffold somente API (sem Inertia, sem SPA):

```bash
suprnova new my-api --api
```

Gere código em um projeto existente:

```bash
suprnova make:controller Posts
suprnova make:migration create_posts_table
suprnova make:command reports:daily   # registra sob o binário console por projeto
suprnova migrate
```

## Obtendo ajuda

`--help` (ou `-h`) funciona em qualquer subcomando. A ajuda de nível
superior é formatada à mão (`ui::print_help`) e agrupa comandos por
seção; a ajuda por subcomando vem do clap e mostra cada flag com seu
padrão:

```bash
suprnova --help
suprnova new --help
suprnova serve --help
suprnova make:inertia --help
```

Para o binário `console` por projeto:

```bash
cargo run --bin console -- --help
cargo run --bin console -- db:seed --help
cargo run --bin console -- <your-command> --help
```

`--version` imprime a versão em sua própria linha, que é o que você
quer ao reportar um bug ou verificar se uma instalação funcionou:

```bash
suprnova --version
# suprnova 1.2.0
```

Tanto `-v` quanto `-V` são aceitos. A flag gerada pelo clap oferece só
`-V`; esta é declarada à mão para que a grafia minúscula - a que a
maioria das pessoas tenta primeiro - também funcione. A versão também
aparece no banner do `--help`, que é onde ela vivia antes da flag
existir.

## Próximos passos

- [`suprnova new`](cli-new.md) - toda flag que o scaffolder aceita e o
  layout de diretórios que ele produz
- [`suprnova serve`](cli-serve.md) - o executor de dev: backend + Vite +
  geração de tipos
- [Geradores](cli-generators.md) - a família `make:*` completa com os
  templates de saída
- [CLI de Migrações](cli-migrations.md) - `migrate`, `migrate:fresh`,
  `db:sync`, e o fluxo de trabalho do SeaORM
- [Console](console.md) - o binário `console` por projeto,
  `#[command]`, `#[derive(Command)]`, e a assimetria de três binários

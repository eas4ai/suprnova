# Instalação

Este capítulo o leva de "sem Suprnova nesta máquina" até um projeto com
scaffold rodando. Se você já chegou lá, pule para o
[Início rápido](quickstart.md).

## Requisitos

- **Rust 1.94.0+** para a `main` atual (o workspace usa a edição 2024). A versão v1.3.0 marcada tem o mesmo requisito mínimo de Rust 1.94.0. Instale por meio do [rustup](https://rustup.rs/):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Node.js 20+** e **npm** (ou pnpm/yarn/bun) para a toolchain de frontend.
  Suprnova usa Vite 8 e seu starter vem com TypeScript + Tailwind v4.
  Instale via [nodejs.org](https://nodejs.org/) ou seu gerenciador de pacotes.
- **Uma biblioteca client de banco de dados** que corresponda ao driver que
  você quer usar:
  - SQLite - nenhum extra necessário; sqlite está incluído
  - PostgreSQL - `libpq` na maioria dos sistemas (geralmente pré-instalado)
  - MySQL ou MariaDB - `libmariadb` / `libmysqlclient` na maioria dos sistemas

Você não precisa escolher um banco de dados agora. O scaffolder padrão
escolhe SQLite para que um app novo rode sem configuração.


A `main` atual usa SeaORM 2.0, SeaQuery 1.0 e SQLx 0.9. Aplicações que chamam SeaORM diretamente devem importar `ExprTrait` para os métodos de expressão do SeaQuery e usar métodos de conexão `*_raw` explícitos para valores `Statement` pré-construídos. A atualização das dependências não exige nenhuma migração de dados da aplicação.

## Instale a CLI

O Suprnova é distribuído como um projeto Cargo, e o instalador da CLI
puxa o framework do git (não do crates.io - veja a [nota de
pré-lançamento](#pre-launch-note) abaixo):

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.0 suprnova-cli
```

Isso compila o binário `suprnova` e o coloca em `~/.cargo/bin`.
Confirme que funcionou:

```bash
suprnova --version
```

Você deve ver `suprnova 0.x.x`.

Se `suprnova` não for encontrado, seu `~/.cargo/bin` não está no
`PATH`. Adicione isto à configuração do seu shell:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Crie um projeto

`suprnova new` cria com scaffold um projeto completo - backend + frontend escolhido
+ configuração Vite + migrações de auth + rotas de exemplo. É interativo por padrão:

```bash
suprnova new my-app
```

O assistente pergunta, em ordem:

1. **Nome do projeto** - omitido quando você o passa como argumento (`my-app`)
2. **Descrição** - usada no `Cargo.toml`
3. **Autor** - usado no `Cargo.toml`; padrão é seu `user.name` do git
4. **Framework de frontend** - um de `svelte` (padrão), `react`, `vue`

Se você quer pular os prompts (CI, setup com script), passe
`--no-interaction` e escolha um frontend explicitamente:

```bash
suprnova new my-app --frontend svelte --no-interaction
```

`--no-interaction` aceita os padrões para descrição ("A web application
built with Suprnova") e autor (vazio). Para configurá-los,
edite o `Cargo.toml` gerado após o scaffold.

Os três starters de frontend cada um vem com seu próprio Svelte-5,
React-19, ou starter Vue-3.5. Todos os três usam Inertia v3 + Vite 8 +
Tailwind v4 e pré-configuram um fluxo Login/Register/Dashboard com
auth baseado em sessão.

Suprnova também vem com um **starter de API** mais enxuto para backends
de serviço sem SPA:

```bash
suprnova new my-api --api
```

O starter de API não tem frontend nem camada Inertia. Ele inicializa o Magnetar
no banco de dados da aplicação, instala `BearerTokenMiddleware` e gera o
scaffold de registro e login por senha contra `app_users`.

## Primeira execução

```bash
cd my-app

# Execute as migrações (users, sessions, etc.)
suprnova migrate

# Instale as dependências do frontend
npm install              # na raiz do projeto

# Inicie o backend + Vite juntos
suprnova serve
```

`suprnova serve` roda o backend em `http://127.0.0.1:8765` e Vite
em `http://127.0.0.1:5765`. Acesse a URL do backend - Vite é proxied
para que você não precise visitá-lo diretamente.

Você deve ver a página de boas-vindas. Então visite `/register` para
criar uma conta e `/login` para fazer login.

## O que foi criado com scaffold

```
my-app/
├── Cargo.toml          # manifesto do crate, dois [[bin]] targets
├── .env                # configuração local (URL do BD, chave de app, portas)
├── .env.example        # modelo para ops/CI
├── .gitignore
├── cmd/
│   └── main.rs         # entrada do binário; chama Application::new().run()
├── src/
│   ├── lib.rs          # organização de módulos
│   ├── bootstrap.rs    # registro de serviços (o análogo Suprnova dos providers)
│   ├── routes.rs       # a árvore da macro routes!
│   ├── bin/
│   │   └── console.rs  # `cargo run --bin console <subcommand>`
│   ├── actions/        # controladores invocáveis de um único método
│   ├── commands/       # handlers anotados com `#[command]`
│   ├── config/         # seções de config tipadas (database, mail)
│   ├── controllers/    # home, auth, dashboard
│   ├── middleware/     # logging, authenticate
│   ├── migrations/     # migrators SeaORM (users, sessions, etc.)
│   └── models/         # estruturas `#[suprnova::model]` (user)
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html
│   └── src/
│       ├── main.{tsx,ts}
│       ├── app.css
│       ├── pages/
│       │   ├── Home, Dashboard
│       │   └── auth/{Login,Register}
│       └── types/
│           └── inertia-props.ts
└── public/
    └── assets/         # saída do build de produção do Vite
```

O tour completo do diretório está em [Estrutura de diretórios](structure.md).

## Atualizando a CLI

A CLI vive no seu `~/.cargo/bin`. Para atualizar para a mais recente:

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.3.0 suprnova-cli
```

`--force` faz o Cargo sobrescrever o binário existente.

## Atualizando a versão do framework do seu app

Um app com scaffold depende do crate do framework `suprnova` via uma
dependência git no `Cargo.toml`:

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.0" }
```

Para puxar as mudanças mais recentes do framework:

```bash
cargo update -p suprnova
```

A dependência git rastreia a tag de release nomeada. Atualize a tag no
`Cargo.toml`, depois execute `cargo update -p suprnova`; seu
`Cargo.lock` registra o commit exato que ele resolveu, então os builds
continuam reproduzíveis entre atualizações - não há necessidade de
fixar manualmente um `rev` no `Cargo.toml`.

## Modelo de distribuição

O Suprnova é distribuído via git, não crates.io - tanto o framework
quanto a CLI instalam a partir do GitHub. Cada versão é publicada como um
GitHub Release com tag (por exemplo, `v1.2.4`), e é da tag que o seu app
depende: um `Cargo.toml` criado com scaffold fixa `tag = "v1.3.0"`, e o
`Cargo.lock` registra o commit exato que aquela tag resolveu, então os
builds são reproduzíveis até que você escolha mudar. Atualizar é
deliberado, nunca incidental - incremente a tag e execute
`cargo update -p suprnova`; a seção sobre atualizar a versão do framework
do seu app mostra o passo a passo.

## Configuração do editor

Algumas extensões do VS Code tornam a experiência mais suave:

- **rust-analyzer** - o servidor de linguagem Rust
- **Svelte for VS Code** (ou React/Vue se você escolheu aqueles)
- **Tailwind CSS IntelliSense**
- **Even Better TOML**

`rust-analyzer` indexará o projeto na primeira abertura; espere 1-2
minutos na primeira vez, depois incremental.

## Próximos passos

- [Início rápido](quickstart.md) - construa um app tiny em 5 minutos
- [Estrutura de diretórios](structure.md) - o que há em cada arquivo
  que o scaffolder gerou
- [Configuração](configuration.md) - a história do `.env` e config tipada
- [Roteamento](routing.md) - adicione sua primeira rota

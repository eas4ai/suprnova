# suprnova new

`suprnova new` faz scaffold de um projeto Suprnova - um crate Cargo
novo com controladores, rotas, migrações, uma SPA Inertia e um fluxo
de auth funcional já conectado. Execute-o uma vez por app; a partir
daí, seu loop diário é `suprnova serve`.

## Uso

```bash
suprnova new [name] [options]
```

Se `name` for omitido, o assistente interativo pergunta por ele. O
nome se torna o diretório do projeto, o nome do pacote Cargo (após a
conversão para snake_case), e o `APP_NAME` padrão em `.env`. Nomes
devem ser letras/dígitos ASCII/`-`/`_`, começar com uma letra, não
conter separadores de caminho ou `..`, e ter 64 caracteres ou menos.

## Opções

| Opção | Descrição |
|---|---|
| `--frontend <svelte\|react\|vue>` | Escolhe o framework da SPA de forma não interativa. Conflita com `--api`. |
| `--api` | Faz scaffold de um projeto somente JSON:API (sem Inertia, sem SPA, auth por token em vez de sessões). |
| `--no-interaction` | Pula todos os prompts e usa os padrões (nome `my-suprnova-app`, frontend `svelte`, autor/descrição vazios). |
| `--no-git` | Pula o `git init` no novo projeto. |
| `--with-portless` | Emite um `portless.json` para que [`suprnova dev:tls`](dev-tls.md) possa servir o app em `https://<name>.localhost`. Opcional; não muda mais nada. |

## Modo interativo

```bash
suprnova new my-app
```

O assistente faz quatro perguntas, nesta ordem:

1. **Nome do projeto** - usa como padrão o argumento de diretório
   (`my-app`)
2. **Descrição** - usada como a descrição do pacote Cargo
3. **Autor** - usado como o autor do pacote Cargo; usa como padrão seu
   `git config user.name <name@email>` se estiver definido
4. **Framework de frontend** - `Svelte (recommended)`, `React`, ou
   `Vue`

Após a confirmação, o scaffolder escreve o projeto, executa
`git init` (a menos que `--no-git`), e imprime os próximos passos:

```
Backend  http://localhost:8765
Frontend http://localhost:5765
```

## Modo não interativo

Para CI, dotfiles, ou configuração via script, passe
`--no-interaction` mais as flags que você quer sobrescrever:

```bash
suprnova new my-app --frontend svelte --no-interaction
```

Padrões sob `--no-interaction`:

- Frontend: `svelte`
- Descrição: `"A web application built with Suprnova"`
- Autor: vazio
- Git: inicializado

Não existem flags `--description` ou `--author`; esses valores só são
definidos pelos prompts interativos ou aceitam seus padrões.

## Projeto somente API

Para backends de serviço sem SPA, use `--api`:

```bash
suprnova new my-api --api
```

O starter de API é significativamente menor: sem diretório
`frontend/`, sem Inertia, sem views de auth, layout de crate único
`src/main.rs` (em vez do workspace `cmd/main.rs` do starter de SPA),
auth baseada em token, e um controlador `users` de exemplo mais um
serializador JSON `UserResource`. O starter de API vincula à porta
8765 em seu `.env`.

`--api` é mutuamente exclusivo com `--frontend`; passar os dois gera
erro. Sob `--api`, somente o nome do projeto é solicitado - os
prompts de descrição/autor/frontend são pulados.

## O que é criado com scaffold

Um tour completo de diretórios está em
[Estrutura de diretórios](structure.md); a versão curta é:

- `cmd/main.rs` - entrada do binário; chama `Application::new()…run()`
- `src/` - controladores, ações, comandos, config, middleware,
  models, migrações, além de `bootstrap.rs` e `routes.rs`
- `src/bin/console.rs` - o análogo por projeto do `php artisan`
- `frontend/` - Vite 8 + Tailwind v4 + o framework escolhido, com
  páginas Home / Dashboard / Login / Register já conectadas via
  Inertia
- `src/migrations/` - tabelas `users`, `sessions`, e
  `remember_tokens` prontas para uso
- `.env` - banco de dados SQLite por padrão, com um `APP_KEY`
  recém-gerado para que o app inicialize sem intervenção do operador
- `.gitignore`, `Cargo.toml`

### Por que Suprnova diverge

O Laravel vem com Blade e traz um frontend via Breeze/Jetstream depois
do fato. O Suprnova vai pelo caminho contrário: `suprnova new` sempre
faz scaffold de uma SPA de verdade (Svelte/React/Vue sobre Inertia) ou
de um projeto JSON:API de verdade. Não existe um starter que comece
com um motor de templates - se você quer HTML renderizado no servidor,
o Tera está disponível, mas não é a forma padrão e não há um caminho
de scaffolder que coloque views na frente do seu app.

O frontend padrão é **Svelte 5** (runes-on), não React. Escolhemos ele
porque é o mais leve dos três em runtime e o mais próximo da filosofia
do framework de que "tempo de compilação vence a inteligência em
runtime". React e Vue são igualmente de primeira classe - escolha o
que sua equipe conhece.

## Distribuição

A própria CLI é distribuída via git, não crates.io (pré-lançamento):

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.4 suprnova-cli
```

`--force` no mesmo comando atualiza uma instalação existente. Projetos
com scaffold dependem do crate do framework da mesma forma - uma
dependência git em seu `Cargo.toml`, fixada na tag de release atual.
Veja [Instalação](installation.md) para os pré-requisitos completos de
toolchain.

## Próximos passos

- [Instalação](installation.md) - pré-requisitos de Rust/Node/BD e
  configuração de toolchain
- [Estrutura de diretórios](structure.md) - o que cada arquivo com
  scaffold faz
- [Início rápido](quickstart.md) - os primeiros 5 minutos depois do
  `suprnova new`
- [suprnova serve](cli-serve.md) - o executor de dev que você vai usar
  a seguir
- [Console](console.md) - `cargo run --bin console` e o sistema
  `#[command]`

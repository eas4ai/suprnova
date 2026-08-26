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
`frontend/`, sem Inertia, sem views de auth e com um layout de crate
único em `src/main.rs`. Ele inicializa o Magnetar usando a conexão
SeaORM compartilhada, cria o modelo canônico `app_users`, instala
`BearerTokenMiddleware` e usa `Auth::password()` para registro e login.
`PASSKEY_RP_ID` e `PASSKEY_RP_ORIGIN` são lidos pelo bootstrap gerado,
com padrões locais. O starter também inclui um controlador `users` de
exemplo e um serializador JSON `UserResource`, e vincula à porta 8765
no `.env`.

`--api` é mutuamente exclusivo com `--frontend`; passar os dois gera
erro. Sob `--api`, somente o nome do projeto é solicitado - os
prompts de descrição/autor/frontend são pulados.

## O que é criado com scaffold

O tour completo do diretório está em [Estrutura de
diretórios](structure.md); a versão curta é:

- `cmd/main.rs` - ponto de entrada do binário; chama
  `Application::new()…run()`
- `src/` - controladores, ações, comandos, config, middleware,
  models, migrações, mais `bootstrap.rs` e `routes.rs`. O
  `bootstrap.rs` gerado conecta a chain de middleware global -
  logging, sessão, locale, CSRF, parsing de include - e chama
  [`Inertia::install`](frontend-inertia-responses.md), que adiciona os
  middlewares do protocolo Inertia (`409` de versão de asset,
  `302 → 303` em redirects não-GET). A versão de asset anunciada
  assume como padrão um hash do manifesto de build do Vite, então
  publicar um build do frontend a altera automaticamente - veja
  [Detecção da versão](frontend-inertia-responses.md). A mesma chamada
  fixa o frontend com o qual você fez o scaffold, então o shell HTML
  carrega o ponto de entrada do Vite daquele framework; o `.env` carrega
  o `SUPRNOVA_FRONTEND` correspondente para os geradores da própria CLI
- `src/bin/console.rs` - o análogo do `php artisan` por projeto
- `frontend/` - Vite 8 + Tailwind v4 + o framework que você escolheu,
  com páginas Home / Dashboard / Login / Register já conectadas via
  Inertia
- `src/migrations/` - tabelas `users`, `sessions` e `remember_tokens`
  prontas para uso
- `.env` - banco de dados SQLite por padrão, com uma `APP_KEY`
  recém-gerada para que o app inicialize sem intervenção do operador
- `.gitignore`, `Cargo.toml`

### Por que Suprnova diverge

O Laravel vem com o Blade e traz um frontend depois, via
Breeze/Jetstream. O Suprnova faz o contrário: `suprnova new` sempre
faz scaffold de uma SPA de verdade (Svelte/React/Vue sobre Inertia) ou
de um projeto JSON:API de verdade. Não existe starter centrado em
template engine - se você quer HTML renderizado no servidor, o Tera
está disponível, mas não é a forma padrão e não há caminho no
scaffolder que coloque views na frente do seu app.

O frontend padrão é o **Svelte 5** (runes-on), não React. Escolhemos
ele porque é o mais leve dos três em runtime e o mais próximo da
filosofia do framework de "ganhos em tempo de compilação acima de
esperteza em runtime". React e Vue são igualmente de primeira classe -
escolha o que seu time conhece.

## Distribuição

A própria CLI é distribuída via git, não crates.io (pré-lançamento):

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.5 suprnova-cli
```

`--force` no mesmo comando atualiza uma instalação existente. Projetos
com scaffold dependem do crate do framework da mesma forma - uma
dependência git no `Cargo.toml` deles, fixada na tag de release atual.
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

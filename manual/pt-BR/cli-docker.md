# Docker

O Suprnova distribui dois comandos de CLI que geram artefatos Docker
que você pode adotar ao pé da letra ou modificar. `docker:init` escreve um
`Dockerfile` multiestágio
+ `.dockerignore` para produção. `docker:compose` escreve um `docker-compose.yml` para serviços de
desenvolvimento local (banco de dados, cache, e opcionalmente Mailpit +
MinIO). Os dois comandos escrevem na raiz do projeto atual; nenhum
deles tenta controlar seu runtime de contêiner.

## docker:init

Gera um Dockerfile de produção junto com um `.dockerignore`
correspondente.

```bash
suprnova docker:init
```

O comando se recusa a sobrescrever um `Dockerfile` existente; remova
o arquivo existente primeiro se você quiser regenerar.

### O que é escrito

| Arquivo | Propósito |
|------|---------|
| `Dockerfile` | Build de três estágios: assets do frontend, binário de release Rust, imagem de runtime |
| `.dockerignore` | Exclui `target/`, `node_modules/`, `.env*`, os artefatos de build existentes, e os próprios arquivos Docker |

### Forma do Dockerfile

O Dockerfile gerado usa três estágios para que a imagem de runtime
carregue só o binário compilado mais suas bibliotecas compartilhadas
necessárias:

1. **`frontend-builder`** - `node:20-alpine`. Instala as deps npm e
   executa `npm run build`, produzindo `frontend/dist`.
2. **`backend-builder`** - `rust:1.94.0-slim-bookworm`. Faz cache de `Cargo.toml`
   + `Cargo.lock` como uma camada de dependência, depois
   copia seu `cmd/`, `src/`, e o `frontend/dist` já construído (como
   `public/assets`) e executa `cargo build --release`.
3. **`runtime`** - `debian:bookworm-slim` com `ca-certificates` e
   `libssl3`. Executa como um `appuser` sem privilégios de root. Copia
   o binário para dentro como `./app` e o diretório `public/` ao lado
   dele. Expõe a porta 8765.

A `CMD` padrão da imagem final é `["./app"]`, que executa o subcomando
`serve` do binário unificado (servidor web com auto-migração no
startup). Para executar um subcomando diferente, sobrescreva o comando
no momento do `docker run`:

```bash
# Servidor web (padrão)
docker run -p 8765:8765 --env-file .env.production my-app

# Executa só as migrações e sai
docker run --env-file .env.production my-app ./app migrate

# Executa o daemon do agendador
docker run --env-file .env.production my-app ./app schedule:work

# Executa o worker de fila
docker run --env-file .env.production my-app ./app queue:work
```

Passe a config de produção via `--env-file .env.production` ou flags
`-e` individuais. Nunca faça commit de `.env.production` - já está
coberto pelo `.dockerignore`.

### Atualizando o toolchain do Rust

O Dockerfile fixa `rust:1.94.0-slim-bookworm` no estágio de build para que uma imagem recém-gerada seja reproduzível e corresponda à `main` atual. Dockerfiles personalizados devem usar a mesma toolchain ou uma mais recente.

```dockerfile
FROM rust:1.94.0-slim-bookworm AS backend-builder
```

Fixe a versão de toolchain que corresponda ao que
`rust-toolchain.toml` (se você tiver um) ou seu `rustc --version`
local reporta.


A `main` atual usa SeaORM 2.0, SeaQuery 1.0 e SQLx 0.9. Aplicações que chamam SeaORM diretamente devem importar `ExprTrait` para os métodos de expressão do SeaQuery e usar métodos de conexão `*_raw` explícitos para valores `Statement` pré-construídos. A atualização das dependências não exige nenhuma migração de dados da aplicação.

### Por que Suprnova diverge

Deployments Laravel tipicamente executam **múltiplos processos por
contêiner ou host**: php-fpm para web, um worker de fila, um
agendador, às vezes um dashboard Horizon, às vezes um runner Octane.
Cada um é sua própria definição de serviço.

O Suprnova compila para **um único binário linkado estaticamente** que
conhece todo subcomando que o framework distribui - `serve`,
`migrate`, `queue:work`, `schedule:work`, `workflow:work`,
`ssr:start`. A mesma imagem Docker executa todo papel; a única coisa
que muda é o comando. Isso faz de "web + worker + scheduler" três
serviços no seu orquestrador que todos apontam para a mesma tag de
imagem - um build para avançar o app inteiro.

## docker:compose

Gera um `docker-compose.yml` que sobe os serviços de desenvolvimento
local.

```bash
suprnova docker:compose [OPTIONS]
```

Assim como `docker:init`, este comando se recusa a sobrescrever um
`docker-compose.yml` existente. Ele também acrescenta
`docker-compose.override.yml` ao seu `.gitignore` (se um
`.gitignore` estiver presente) para que você possa manter overrides
por desenvolvedor localmente sem fazer commit deles.

### Opções

| Opção | Descrição |
|--------|-------------|
| `--with-mailpit` | Inclui o serviço de teste de email Mailpit |
| `--with-minio` | Inclui o MinIO (armazenamento de objetos compatível com S3) |

Se você não passar nenhuma das duas flags, o comando pergunta de
forma interativa pelas duas. Passar qualquer uma das flags pula o
prompt e usa os valores de flag que você deu.

### O que você sempre recebe

PostgreSQL e Redis são escritos em todo arquivo compose gerado:

| Serviço | Porta padrão | Imagem |
|---------|-------------:|-------|
| PostgreSQL | 5432 | `postgres:16-alpine` |
| Redis | 6379 | `redis:7-alpine` |

Os dois serviços têm health checks, volumes nomeados persistentes, e
vivem em uma rede com escopo de projeto (`<project>_network`). O
usuário, senha, e banco de dados do Postgres têm como padrão
`suprnova` / `suprnova_secret` / `suprnova_db`.

### Serviços opcionais

Quando você opta por eles:

| Serviço | Portas padrão | Imagem |
|---------|--------------:|-------|
| Mailpit | 1025 (SMTP), 8025 (UI) | `axllent/mailpit:latest` |
| MinIO | 9000 (S3 API), 9001 (Console) | `minio/minio:latest` |

Por padrão o Mailpit aceita qualquer auth SMTP para que você não
precise configurar credenciais durante o desenvolvimento; a UI web em
`http://localhost:8025` mostra todo email que seu app envia. As
credenciais padrão do MinIO são `minioadmin` / `minioadmin`.

### Executando a stack

```bash
# Sobe tudo em background
docker compose up -d

# Acompanha os logs
docker compose logs -f

# Para e remove os contêineres (os volumes persistem)
docker compose down

# Remove os volumes também (limpa o banco de dados local)
docker compose down -v
```

### Conectando `.env` ao compose

O arquivo compose usa a sintaxe `${VAR:-default}` em todo lugar,
então você pode sobrescrever qualquer coisa definindo-a em `.env` ou
no seu shell. Um `.env` típico para a stack padrão:

```env
DATABASE_URL=postgres://suprnova:suprnova_secret@localhost:5432/suprnova_db
REDIS_URL=redis://localhost:6379

# Mailpit (se ativado)
MAIL_DRIVER=smtp
MAIL_HOST=localhost
MAIL_PORT=1025

# MinIO (se ativado)
FILESYSTEM_DISK=s3
S3_ENDPOINT=http://localhost:9000
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_BUCKET=local
S3_REGION=us-east-1
```

Para sobrescrever uma porta (por exemplo, porque 5432 já está em
uso), defina a env var correspondente antes de subir a stack:

```bash
DB_PORT=5433 docker compose up -d
```

O conjunto completo de portas sobrescrevíveis:

| Variável | Serviço | Padrão |
|----------|---------|--------:|
| `DB_PORT` | PostgreSQL | 5432 |
| `REDIS_PORT` | Redis | 6379 |
| `MAILPIT_SMTP_PORT` | Mailpit SMTP | 1025 |
| `MAILPIT_UI_PORT` | Mailpit UI | 8025 |
| `MINIO_API_PORT` | MinIO S3 | 9000 |
| `MINIO_CONSOLE_PORT` | MinIO Console | 9001 |

### Customizando o arquivo compose

`docker-compose.yml` é seu para editar depois da geração - o Suprnova
não o regenera nem o lê depois. Patches comuns:

- Troque `postgres:16-alpine` por `mysql:8` ou `mariadb:11` se você
  preferir um desses drivers; os dois são de primeira classe no
  Suprnova
- Adicione uma entrada `volumes:` que monta seu diretório
  `migrations/` se você quiser executar migrações dentro de um
  contêiner de execução única
- Adicione serviços adicionais (Qdrant, Elasticsearch, Nats) da mesma
  forma

## Implantação em produção

Para um deployment de verdade, execute `docker:init` e trate o
`Dockerfile` gerado como sua entrada de build. A maioria dos
orquestradores (Railway, Fly, Digital Ocean App Platform, Kubernetes)
só precisa de três coisas:

1. A tag de imagem construída a partir deste `Dockerfile`
2. Um arquivo env com `DATABASE_URL`, `APP_KEY`, e quaisquer chaves
   específicas de driver
3. Uma verificação de saúde apontando para
   `GET /_suprnova/health/live` (e, se a plataforma distinguir os
   dois, uma verificação de prontidão em `/_suprnova/health/ready`)

A forma de binário único significa que todo papel usa a mesma imagem;
você declara um serviço "web" executando `./app` e um serviço
"scheduler" ou "worker" executando `./app schedule:work` (ou
`./app queue:work`). Os dois leem o mesmo env, então ficam em
sincronia em todo deploy.

Veja [Implantação](deployment.md) para o checklist agnóstico de
plataforma, e os guias de plataforma para exemplos completamente
trabalhados: [Railway](deployment-railway.md),
[Digital Ocean](deployment-digital-ocean.md),
[Hetzner VPS](deployment-hetzner.md).

## Resumo

| Comando | Escreve | Quando usar |
|---------|--------|-------------|
| `suprnova docker:init` | `Dockerfile`, `.dockerignore` | Construindo imagens de produção |
| `suprnova docker:compose` | `docker-compose.yml` | Subindo Postgres/Redis/Mailpit/MinIO localmente |

## Próximos passos

- [Implantação](deployment.md) - o checklist de deployment agnóstico
  de plataforma
- [Railway](deployment-railway.md) - PaaS gerenciado com build a
  partir do git
- [Digital Ocean](deployment-digital-ocean.md) - deploys em App
  Platform
- [Hetzner VPS](deployment-hetzner.md) - bare-metal com systemd + Caddy
- [Variáveis de ambiente](env-vars.md) - toda chave que o framework lê

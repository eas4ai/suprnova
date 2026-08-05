# Implantar no Railway

O [Railway](https://railway.app) é um PaaS orientado a Git que constrói
seu Dockerfile e o roda em infraestrutura gerenciada. Combine isso com
o Postgres e Redis gerenciados da Railway e você tem uma stack de
produção Suprnova completa, sem servidores para cuidar. Esta receita
leva um app recém-criado com scaffold do `suprnova new` até uma URL
ativa.

## Pré-requisitos

- Uma [conta Railway](https://railway.app)
- Um projeto Suprnova publicado no GitHub, GitLab ou Bitbucket
- Um `Dockerfile` e `.dockerignore` na raiz do repositório, gerados
  por:
  ```bash
  suprnova docker:init
  ```
- Um `APP_KEY` gerado que você pode colar nas variáveis da Railway:
  ```bash
  suprnova key:generate --show
  ```

`suprnova` só é necessário localmente - a Railway constrói o
Dockerfile por conta própria. O crate do framework é obtido do git
como uma dependência cargo normal durante o build.

## Provisionando o projeto

1. Abra o [painel da Railway](https://railway.app/dashboard), clique
   em **New Project**, e escolha **Deploy from GitHub repo**.
2. Escolha o repositório. A Railway detecta o `Dockerfile` e inicia o
   primeiro build automaticamente.
3. Enquanto ele constrói, adicione um banco de dados: **New** →
   **Database** → **Add PostgreSQL**. A Railway expõe `DATABASE_URL`
   como uma variável de referência no projeto.
4. Opcionalmente, adicione o Redis da mesma forma (**New** →
   **Database** → **Redis**) se seu app usa o driver Redis de cache,
   sessão, fila ou limitação de taxa. A Railway expõe a URL de conexão
   como `REDIS_URL`.

## Conectando as variáveis

Abra o serviço web, vá em **Variables**, e adicione a configuração de
produção. Use a sintaxe de referência `${{ }}` da Railway para puxar
URLs dos serviços de banco de dados, para que rotações não exijam
colar de novo.

```env
APP_ENV=production
APP_KEY=<paste the output of `suprnova key:generate --show`>
SERVER_HOST=0.0.0.0
SERVER_PORT=8765
DATABASE_URL=${{ Postgres.DATABASE_URL }}
REDIS_URL=${{ Redis.REDIS_URL }}
```

Algumas coisas que vale a pena saber:

- **`APP_KEY` é obrigatório em ambientes não voltados a
  desenvolvimento.** O Suprnova falha fechado ao inicializar quando
  `APP_ENV != local|dev|test` e `APP_KEY` está ausente ou malformado.
  O servidor registra uma mensagem de correção e sai com código
  não-zero - a Railway marcará o deploy como falho. Gere a chave com
  `suprnova key:generate --show`.
- **`SERVER_HOST=0.0.0.0` é obrigatório.** A Railway roteia tráfego
  através da interface de rede do contêiner; vincular-se a
  `127.0.0.1` (o padrão local) vai parecer uma conexão recusada.
- **`SERVER_PORT` corresponde ao `EXPOSE` no Dockerfile.** O
  Dockerfile gerado expõe a porta 8765. A Railway mapeia isso para uma
  URL pública automaticamente.

## Compilando e implantando

A Railway constrói a cada push para a branch conectada. O Dockerfile
gerado por `docker:init` faz:

1. **Etapa 1 - Frontend.** Roda `npm ci` e `npm run build` em
   `frontend/`. A saída do Vite vai para `frontend/dist/`.
2. **Etapa 2 - Backend.** Roda `cargo build --release` contra seu
   workspace; camadas de dependência em cache mantêm os builds
   iterativos rápidos.
3. **Etapa 3 - Runtime.** Uma imagem `debian:bookworm-slim` com
   `ca-certificates` + `libssl3`, um `appuser` sem privilégios de
   root, e o binário `./app` compilado. O `CMD` padrão é `./app`, que
   roda `serve` com auto-migrate.

O primeiro build normalmente leva vários minutos (cache do Rust
frio); builds subsequentes são muito mais rápidos graças ao cache de
camadas do Docker.

## Adicionando um serviço de agendador

Se seu app usa agendamentos `#[derive(Task)]`, o agendador precisa de
seu próprio processo de longa duração. Adicione um segundo serviço a
partir do mesmo repositório:

1. **New** → **GitHub Repo** → escolha o mesmo repositório.
2. Nomeie-o `scheduler` para que seja fácil de identificar no painel.
3. Em **Settings** → **Deploy**, defina o **Custom Start Command**
   como:
   ```bash
   ./app schedule:work
   ```
4. Copie as mesmas variáveis (especialmente `APP_KEY` e as
   referências de banco de dados) para que o worker leia a mesma
   configuração que o serviço web.

`schedule:work` é um loop daemon - ele acorda uma vez por minuto,
consulta o agendamento por tarefas vencidas, e as roda através da
mesma inicialização que o servidor HTTP. Veja [Console](console.md) e
o capítulo do agendador para o contrato.

Rode exatamente uma instância do agendador. Múltiplos processos
`schedule:work` coordenam via locks apoiados em cache, mas a
expectativa padrão é um único worker.

### Por que Suprnova diverge

Um deploy Laravel no Forge ou Vapor tipicamente conecta um servidor
web (php-fpm + nginx), um worker de fila (`php artisan queue:work`), e
uma entrada de cron que invoca `schedule:run` a cada minuto. Três
componentes, três superfícies de deploy.

O Suprnova compila todo papel no mesmo binário. A especificação de
serviço da Railway é `./app` para o papel web e `./app
schedule:work` para o agendador - mesma imagem, mesma inicialização,
argv diferente. Não há contêiner php-fpm separado, nenhuma imagem de
worker separada, nenhum cron no host. Adicione `./app queue:work`
como um terceiro serviço se você tiver jobs enfileirados, e você tem
a topologia Laravel completa em três serviços Railway a partir de um
único Dockerfile.

## Verificações de saúde e `railway.json`

Para mais controle sobre o deploy, faça commit de um `railway.json`
na raiz do repositório. A Railway o detecta automaticamente.

```json
{
  "$schema": "https://railway.app/railway.schema.json",
  "build": {
    "builder": "DOCKERFILE",
    "dockerfilePath": "Dockerfile"
  },
  "deploy": {
    "startCommand": "./app",
    "healthcheckPath": "/_suprnova/health/live",
    "healthcheckTimeout": 300,
    "restartPolicyType": "ON_FAILURE",
    "restartPolicyMaxRetries": 10
  }
}
```

O Suprnova vem com endpoints de saúde integrados que fazem
shortcircuit antes da cadeia de middleware - eles retornam um status
JSON 200 sem passar por auth, CSRF, ou limitação de taxa. O prefixo
`/_suprnova/` é reservado para que nunca colidam com suas rotas.

`healthcheckPath` acima aponta para `/_suprnova/health/live`, que não
toca em nada. Esse pareamento é deliberado: este serviço está
configurado com `"restartPolicyType": "ON_FAILURE"`, então o que quer
que a verificação de saúde sonde é um gatilho de reinício. Apontá-lo
para o banco de dados - via `/_suprnova/health/ready` ou o mais
antigo `/_suprnova/health?db=true` - significa que um soluço no banco
de dados reinicia cada réplica no momento em que o banco de dados
menos pode permitir um thundering herd de reconexões. Sonde o banco
de dados a partir de uma verificação de prontidão separada ou do seu
monitoramento, não a partir do caminho que reinicia o processo. Veja
[Use a sonda correta para a pergunta
certa](deployment.md#use-the-right-probe-for-the-right-question).

Ambos os caminhos mais antigos continuam funcionando, então um
serviço Railway existente não precisa de nenhuma mudança; os
caminhos nomeados são simplesmente mais claros.

## Domínios personalizados e TLS

1. No serviço web, abra **Settings** → **Networking**.
2. Clique em **Generate Domain** para um subdomínio
   `*.up.railway.app`, ou **Custom Domain** para apontar seu próprio
   hostname para o serviço.
3. Atualize o DNS conforme a Railway instruir (um `CNAME` para
   subdomínios, um ANAME/ALIAS para domínios apex).

A Railway provisiona e renova certificados Let's Encrypt tanto para
domínios gerados quanto personalizados.

## Migrações em CI/CD

O `CMD ["./app"]` padrão roda migrações ao inicializar, o que é
adequado para deploys de instância única. Para configurações
multi-réplica, desacople a etapa de migração:

1. Adicione um **pre-deploy hook** de execução única que roda `./app
   migrate` contra o banco de dados de produção antes que as novas
   réplicas iniciem.
2. Mude o comando de início do runtime para `./app serve
   --no-migrate` para que as réplicas não corram uma contra a outra.

O executor de migrações é idempotente - mesmo que você não separe as
etapas, rodar migrações a cada inicialização é seguro entre réplicas.
A separação existe para que você possa falhar o deploy cedo em uma
migração ruim sem manter o rollout aberto.

## Logs, métricas, rollbacks

A aba do serviço web expõe:

- **Deployments** - todo build em ordem cronológica; o menu de três
  pontos em um deploy anterior bem-sucedido é o caminho de rollback
  em um clique
- **Logs** - saída do `tracing` do contêiner, com campos de log
  estruturado (`request_id`, `route`, `status`) prontos para os
  filtros do visualizador de logs
- **Metrics** - CPU, memória, IO de rede; útil para dimensionar a
  instância para cima ou para baixo

## Solução de problemas

**O build falha em `cargo build --release`.** Reproduza localmente
com `docker build -t myapp .`. A causa mais comum é um membro do
workspace que compila na sua máquina mas está faltando no
repositório - o Dockerfile copia `Cargo.toml` e `Cargo.lock` primeiro,
então crates faltando falham de forma explícita.

**O app retorna "connection refused".** Verifique se
`SERVER_HOST=0.0.0.0` está definido no serviço. O padrão é
`127.0.0.1`, para o qual a Railway não consegue rotear.

**O app inicializa e então sai com um erro de chave.** `APP_KEY` está
indefinido ou malformado. O framework se recusa a inicializar em
produção sem uma; cole novamente a saída de `suprnova key:generate
--show` nas variáveis do serviço.

**As migrações falham ao inicializar.** Verifique os logs em busca do
erro SQL subjacente. Causas comuns são um `DATABASE_URL` indefinido
(verifique se a referência `${{ Postgres.DATABASE_URL }}` foi
resolvida) ou uma migração que rodou contra uma baseline desatualizada
(`./app migrate:status` relata o que está aplicado onde).

**O agendador nunca dispara.** Verifique que o comando de início é
exatamente `./app schedule:work` (não `schedule:run`, que roda tarefas
vencidas uma vez e sai). `schedule:list` a partir de um deploy de
execução única confirma que suas tarefas estão registradas.

## Próximos passos

- [Visão geral de implantação](deployment.md) - o modelo de binário
  unificado que seus serviços Railway rodam
- [CLI do Docker](cli-docker.md) - o que `docker:init` e
  `docker:compose` realmente geram
- [Configuração](configuration.md) - carregamento de `.env`, config
  tipada, chaves obrigatórias
- [Console](console.md) - `schedule:work`, `queue:work`,
  `workflow:work`, e o resto da CLI unificada
- [Implantar no Digital Ocean](deployment-digital-ocean.md) - a mesma
  receita em um PaaS diferente

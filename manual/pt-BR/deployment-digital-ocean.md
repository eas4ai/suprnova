# Implantar no Digital Ocean

O Digital Ocean tem dois alvos de produção que servem para um app
Suprnova: **App Platform** (um PaaS Docker gerenciado - envie e
esqueça) e um **Droplet** (sua própria VPS, você gerencia tudo). Este
capítulo percorre os dois. Use o App Platform quando você quiser
bancos de dados gerenciados, deploys automáticos, e SSL cuidado para
você. Use um Droplet quando você quiser controle total, já rodar
outros serviços na máquina, ou quiser manter a fatura estável
independentemente do tráfego.

## Pré-requisitos

- Uma [conta Digital Ocean](https://www.digitalocean.com)
- Um projeto Suprnova com um Dockerfile - gere um com:
  ```bash
  suprnova docker:init
  ```
- Um `APP_KEY` para produção. Gere um e guarde-o em um lugar seguro:
  ```bash
  suprnova key:generate --show
  ```
  O Suprnova falha fechado ao inicializar quando `APP_ENV` é qualquer
  coisa além de `local` / `development` / `testing` e `APP_KEY` está
  indefinido.
- Um repositório git (GitHub ou GitLab) - obrigatório para o App
  Platform; para Droplets você também pode enviar (push) uma imagem
  pré-construída para um registry.

## App Platform

O App Platform constrói seu Dockerfile, roda o binário único do
Suprnova, e te dá um Postgres gerenciado se você quiser um.

### 1. Criando o app

1. Vá para [Digital Ocean Apps](https://cloud.digitalocean.com/apps).
2. Clique em **Create App**, conecte o GitHub/GitLab, e escolha o
   repositório e a branch.
3. O App Platform detecta automaticamente o `Dockerfile` na raiz do
   repositório.

### 2. Configurando o serviço web

| Configuração | Valor |
|---|---|
| Tipo de recurso | Web Service |
| Porta HTTP | `8765` |
| Comando de execução | deixe vazio - o `CMD` do Dockerfile roda `./app` |
| Verificação de saúde (caminho HTTP) | `/_suprnova/health/live` |

O binário padrão do Suprnova roda `serve` com auto-migrações, então o
contêiner vai rodar as migrações na inicialização e depois vincular o
listener.

### 3. Adicionando um Postgres gerenciado

1. **Add Resource** -> **Database** -> **PostgreSQL**.
2. Escolha um plano (Dev Database para testes; um plano Production
   para tráfego real).

O App Platform injeta `DATABASE_URL` em todo componente
automaticamente via a vinculação `${db.DATABASE_URL}`.

### 4. Variáveis de ambiente

Na seção **Environment Variables** do seu componente web, defina:

| Variável | Valor | Notas |
|---|---|---|
| `APP_ENV` | `production` | aciona a verificação de `APP_KEY` com falha fechada |
| `APP_KEY` | saída de `suprnova key:generate --show` | marque como **criptografada** |
| `SERVER_HOST` | `0.0.0.0` | vincula a todas as interfaces |
| `SERVER_PORT` | `8765` | corresponde ao `EXPOSE` do Dockerfile |
| `APP_URL` | `https://your-app.ondigitalocean.app` | usado pelo Inertia + URLs assinadas |

`DATABASE_URL` é fornecido automaticamente pela vinculação do banco de
dados gerenciado; não o defina manualmente.

Se você usa Redis para cache/sessões, adicione um cluster Redis
gerenciado e defina `REDIS_URL` para seu valor de vinculação
(`${redis.REDIS_URL}`).

### 5. Implantação

Clique em **Create Resources**. O primeiro build leva alguns minutos
(build de release do Rust + build do frontend); builds subsequentes
usam o cache de camadas do Dockerfile e rodam muito mais rápido.

### Adicionando um worker de agendador

Tarefas agendadas (handlers `#[derive(Task)]` registrados via
`Schedule::call`) precisam de um processo de longa duração. Adicione
um componente Worker que roda a mesma imagem com um comando
diferente:

1. **Create** -> **Add Resource** -> **Detect from source code**,
   selecione o mesmo repositório.
2. Defina o tipo de recurso como **Worker**.
3. **Run command**:
   ```bash
   ./app schedule:work
   ```
4. O worker herda as env vars do app, incluindo `DATABASE_URL` e
   `APP_KEY`.

Workers não recebem tráfego HTTP. Rode exatamente **uma** instância
de worker - múltiplos agendadores rodariam cada tarefa múltiplas
vezes.

Para workers de fila (`./app queue:work`) o padrão é idêntico; você
geralmente pode rodar mais de um worker de fila com segurança porque
o driver de fila coordena qual worker pega qual job. Veja
[Filas](queues.md).

### Especificação do app (infraestrutura como código)

Para deploys repetíveis, faça commit de um `.do/app.yaml`:

```yaml
name: my-suprnova-app

services:
  - name: web
    dockerfile_path: Dockerfile
    github:
      repo: your-username/your-repo
      branch: main
      deploy_on_push: true
    http_port: 8765
    instance_count: 1
    instance_size_slug: basic-xxs
    health_check:
      # Apenas vivacidade - o App Platform reinicia o contêiner
      # quando isso falha, então não deve depender do Postgres. Veja
      # a nota sobre verificação de saúde em Solução de problemas.
      http_path: /_suprnova/health/live
    envs:
      - key: APP_ENV
        value: production
      - key: APP_KEY
        scope: RUN_TIME
        type: SECRET
        value: ${APP_KEY}
      - key: SERVER_HOST
        value: 0.0.0.0
      - key: SERVER_PORT
        value: "8765"
      - key: APP_URL
        value: https://your-app.ondigitalocean.app
      - key: DATABASE_URL
        scope: RUN_TIME
        value: ${db.DATABASE_URL}

workers:
  - name: scheduler
    dockerfile_path: Dockerfile
    github:
      repo: your-username/your-repo
      branch: main
      deploy_on_push: true
    instance_count: 1
    instance_size_slug: basic-xxs
    run_command: ./app schedule:work
    envs:
      - key: APP_ENV
        value: production
      - key: APP_KEY
        scope: RUN_TIME
        type: SECRET
        value: ${APP_KEY}
      - key: DATABASE_URL
        scope: RUN_TIME
        value: ${db.DATABASE_URL}

databases:
  - name: db
    engine: PG
    version: "16"
    size: db-s-dev-database
```

Faça o deploy com a CLI `doctl`:

```bash
doctl apps create --spec .do/app.yaml
```

Defina o segredo `APP_KEY` separadamente via a UI de Apps ou:

```bash
doctl apps update <app-id> --spec .do/app.yaml \
  --set-env "APP_KEY=$(suprnova key:generate --show)"
```

### Domínio personalizado

Em **Settings** -> **Domains** -> **Add Domain**, digite seu domínio
e siga as instruções de DNS. O App Platform emite e renova um
certificado Let's Encrypt automaticamente.

Depois que o domínio estiver no ar, atualize `APP_URL` para
corresponder - o Inertia o usa para o header X-Inertia-Location e
URLs assinadas o usam para a entrada do hash.

### Escalando

- **Horizontal**: aumente **Instance Count** no serviço web. Cada
  instância compartilha o Postgres gerenciado; múltiplas instâncias
  rodando auto-migrações na inicialização é seguro - o Suprnova usa
  o migrador com bloqueio consultivo do SeaORM.
- **Vertical**: mude **Instance Size**. O binário Rust fica bem no
  menor slug para apps de baixo tráfego; aumente quando você começar
  a servir WebSockets ou conexões de longa duração em escala.

Mantenha o worker do agendador com uma contagem de instâncias de
**1**.

## Droplet (VPS)

Um Droplet é o caminho quando você quer rodar o Suprnova na sua
própria VPS. A mecânica é idêntica a qualquer outra VPS Linux -
serviço systemd, proxy reverso Caddy, Postgres gerenciado ou
auto-hospedado. O capítulo [Hetzner VPS](deployment-hetzner.md) é o
passo a passo canônico para esse padrão; tudo lá se aplica
literalmente em um Droplet. As únicas diferenças que vale a pena
destacar:

- **Imagem**: escolha **Ubuntu 24.04** ou **Debian 12** no console do
  Droplet.
- **Banco de dados**: você pode usar o **Managed Databases** do
  Digital Ocean para Postgres / MySQL / Redis em vez de rodá-los no
  Droplet - a mesma história de `DATABASE_URL` / `REDIS_URL`,
  aponte-os para o endpoint gerenciado e o Suprnova não percebe a
  diferença.
- **Backups**: ative snapshots do Droplet e backups diários do banco
  de dados gerenciado no console da DO.
- **Rede**: use uma **VPC** da DO para manter o Droplet e quaisquer
  bancos de dados gerenciados em uma rede privada; vincule o listener
  a `127.0.0.1` e coloque o Caddy na frente para TLS.

Se você quiser Docker em um Droplet (em vez de um binário de
sistema), o padrão docker-compose de [Docker](cli-docker.md) se
encaixa direitinho - troque o Postgres auto-hospedado pelo banco de
dados gerenciado e pronto.

### Por que Suprnova diverge

O deploy PHP típico do Laravel precisa de PHP-FPM + um opcache + um
executor de fila + uma entrada de cron do agendador - pelo menos três
peças móveis, cada uma com sua própria semântica de reinício. Um
deploy Suprnova é um único binário mais um processo worker opcional.
O binário roda migrações, serve HTTP, lida com WebSockets, e vive
atrás de um proxy reverso. O mesmo binário, invocado com `./app
schedule:work` ou `./app queue:work`, é seu agendador ou worker de
fila. O modelo "uma imagem, múltiplos componentes" do App Platform se
encaixa nisso naturalmente - mesmo Dockerfile para todo componente,
`run_command` diferente por papel.

## Solução de problemas

### O build falha

A primeira coisa a verificar é se o Dockerfile constrói localmente:

```bash
docker build -t myapp .
```

Causas comuns quando o build local funciona mas o do App Platform
não:

- **Arquivos de contexto de build ausentes**: verifique que o
  `.dockerignore` não está excluindo `Cargo.lock` ou o diretório
  `migrations/`.
- **Falta de memória durante o cargo build**: aumente o tamanho da
  instância de build em App Settings -> Resources -> Build. Builds de
  release do Rust consomem muita memória.

### O app inicializa e depois trava

Verifique os logs de runtime na aba **Runtime Logs**. As duas falhas
de boot mais comuns do Suprnova são:

- **`APP_KEY is required when APP_ENV=production`** - gere um com
  `suprnova key:generate --show` e adicione-o como uma env var
  criptografada.
- **`SERVER_HOST=…` value invalid** - deve ser `0.0.0.0` para o App
  Platform, não `127.0.0.1` (o loopback não é alcançável a partir do
  load balancer).

### Falha na verificação de saúde

A plataforma faz ping em `/_suprnova/health/live` e espera um 200
dentro do timeout configurado. Se estiver falhando:

- Confirme que o caminho é exatamente `/_suprnova/health/live` (não
  `/health`). O mais antigo `/_suprnova/health` ainda funciona se for
  isso que sua spec já nomeia.
- Confirme que a porta é `8765` e corresponde a `SERVER_PORT`.
- Para diferenciar "não consegue vincular" de "não consegue alcançar
  o Postgres", sonde o banco de dados **manualmente** a partir do
  console em vez de a partir da verificação de saúde:

  ```bash
  curl http://localhost:8765/_suprnova/health/ready
  # Saudável:  200 {"status":"ok","database":"connected"}
  # Degradado: 503 {"status":"degraded","database":"error"}
  ```

  Uma resposta degradada significa que o app vinculou mas não
  consegue alcançar o Postgres - verifique a vinculação de
  `DATABASE_URL`. Não passe `-f`: isso faz o curl sair silenciosamente
  no 503, que é justamente o caso que você está tentando ler.

Não coloque a sondagem do banco de dados no `health_check` da app
spec. O App Platform reinicia o contêiner quando essa verificação
falha, então um soluço no banco de dados derrubaria o app junto - o
modo de falha é um loop de reinício exatamente durante o incidente
que você precisa que o app sobreviva. Veja [Use a sonda correta para
a pergunta
certa](deployment.md#use-the-right-probe-for-the-right-question).

### Migrações de banco de dados não estão rodando

Migrações rodam automaticamente como parte do boot padrão de `./app`.
Se não estiverem, verifique os logs de runtime em busca de erros do
SeaORM. Para rodá-las manualmente a partir do console do App
Platform:

1. Abra a aba **Console** no componente web.
2. Rode `./app migrate`.

Se você preferir manter as migrações fora do caminho de boot, defina
o comando de execução como `./app serve --no-migrate` e adicione um
**Job** de execução única na app spec que roda `./app migrate` antes
do deploy.

## Próximos passos

- [Visão geral de implantação](deployment.md) - o guia introdutório
  de deploy multiplataforma (binário, migrações, agendador, saúde)
- [Docker](cli-docker.md) - o que `suprnova docker:init` e
  `docker:compose` geram
- [Configuração](configuration.md) - toda env var que o Suprnova lê
- [Variáveis de ambiente](env-vars.md) - referência completa,
  incluindo as obrigatórias em produção
- [Implantar no Hetzner VPS](deployment-hetzner.md) - o passo a passo
  do Droplet se aplica aqui literalmente

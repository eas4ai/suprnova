# Visão geral de implantação

Um app Suprnova compila para um único binário autossuficiente que possui
o servidor web, o executor de migrações, o agendador e o worker de fila.
Fazer deploy é "copiar o binário, definir quatro variáveis de ambiente,
executar". Este capítulo cobre quais são essas quatro variáveis, o que os
subcomandos do binário fazem em produção e como o endpoint de saúde
integrado se conecta à sonda de vivacidade de uma plataforma. Instruções
específicas da plataforma seguem em [Railway](deployment-railway.md),
[Digital Ocean](deployment-digital-ocean.md) e
[Hetzner](deployment-hetzner.md).

## O binário único

Seu app compila para um binário com uma superfície de subcomandos clap:

```bash
./app                       # serve (padrão) - auto-migrate, depois HTTP
./app serve                 # serve explícito, com auto-migrate
./app serve --no-migrate    # serve sem executar migrações
./app web:run               # alias para serve

./app migrate               # aplica as migrações pendentes e sai
./app migrate:status        # mostra o status das migrações
./app migrate:rollback [N]  # reverte as últimas N migrações (padrão 1)
./app migrate:fresh         # derruba todas as tabelas, depois migra de novo - em produção
                            # isso exige --force E uma confirmação digitada em um
                            # terminal interativo; veja cli-migrations.md

./app schedule:work         # daemon do agendador - acorda a cada minuto
./app schedule:run          # executa as tarefas vencidas uma vez e sai
./app schedule:list         # imprime todas as tarefas registradas
./app queue:work            # daemon do worker de fila
./app workflow:work         # daemon do worker de fluxo de trabalho

./app down [--secret …] [--retry …] [--except …] [--message …]
./app up                    # sai do modo de manutenção
```

Um binário significa uma imagem Docker, um artefato de CI, um deploy para
verificar. A mesma imagem executa o serviço web, o agendador, o worker de
fila e o worker de fluxo de trabalho - você inicia um subcomando diferente
para cada um.

## Quatro variáveis de ambiente de produção

Suprnova falha fechado ao inicializar se o ambiente de produção não estiver
configurado corretamente. O conjunto mínimo para fazer deploy:

| Variável | O que faz | Modo de falha |
|---|---|---|
| `APP_ENV` | Seleciona o ambiente (`production`, `staging`, etc.). | Padrão é `local` se não definido - seu app executa em modo dev em prod. |
| `APP_KEY` | Chave AES-256 base64 de 32 bytes para `Crypt`, sessões, cookies e cursores de paginação. | O boot retorna um erro digitado e sai com código não-zero quando `APP_ENV` não é local/dev/test e `APP_KEY` está ausente ou malformado. |
| `APP_URL` | URL absoluta canônica do seu app (`https://app.example.com`). | Padrão é `http://localhost:8765`; URLs assinadas, redirecionamentos, links de email e URLs Inertia absolutas usam isso. |
| `DATABASE_URL` | URL de conexão para seu banco de dados relacional. | O boot recusa iniciar quando `APP_ENV` é `production` ou `staging` e `DATABASE_URL` não está definido - o fallback SQLite dev é rejeitado explicitamente. |

Gere `APP_KEY` uma vez com a CLI:

```bash
suprnova key:generate           # escreve APP_KEY=… em ./.env
suprnova key:generate --show    # imprime a chave para $(…)
```

Para rotação de chaves, veja [Criptografia](encryption.md) -
`APP_KEY_PREVIOUS` (ou o compatível com Laravel `APP_PREVIOUS_KEYS`)
recebe uma lista separada por vírgulas de chaves antigas para fallback
somente descriptografia.

Além das quatro variáveis obrigatórias, configurações comuns de produção:

| Variável | Padrão | Notas |
|---|---|---|
| `SERVER_HOST` | `127.0.0.1` | Use `0.0.0.0` em contêineres. |
| `SERVER_PORT` | `8765` | Corresponda à porta esperada de sua plataforma. |
| `APP_DEBUG` | derivado de env | `false` em ambientes production/staging/customizados. Defina manualmente se quiser erros explícitos em staging. |
| `SERVER_MAX_BODY_SIZE` | padrão por handler | Limite de tamanho de corpo de solicitação no processo. |
| `SERVER_MAX_CONNECTIONS` | não definido (ilimitado) | Limite de conexões TCP ativas concorrentes. Veja abaixo. |
| `SERVER_HEALTH_READINESS_TOKEN` | não definido (readiness é público) | Segredo compartilhado necessário para alcançar a sonda de prontidão. Veja [Verificação de saúde](#verificação-de-saúde). |
| `DB_MAX_CONNECTIONS` | `10` | Tamanho do pool. |
| `REDIS_URL` | não definido | Obrigatório se você tiver configurado os drivers Redis cache/queue/session. |

A tabela completa está em [Variáveis de ambiente](env-vars.md).

## Banco de dados recomendado: MariaDB

Suprnova oferece suporte a SQLite, PostgreSQL, MySQL e MariaDB como backends
relacionais de primeira classe. A recomendação é específica do ambiente:

- **Desenvolvimento.** SQLite. O scaffolder escreve
  `DATABASE_URL=sqlite://./database.db` para que `suprnova serve` funcione
  com zero configuração de banco de dados.
- **Produção.** MariaDB. Ele consolida o que seriam três serviços separados
  (relacional + vetor + cache KV) em um único engine, com tabelas com
  versionamento de sistema para auditoria se você precisar.

```bash
# .env.production
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production
```

Use o esquema `mysql://` - o driver MySQL do SeaORM trata MariaDB
nativamente, e o `MariaDbVectorDriver` do Suprnova (`VECTOR(N)` + HNSW)
se conecta diretamente para cargas de trabalho de vetor.

Os outros backends relacionais também são de primeira classe:

```bash
# PostgreSQL
DATABASE_URL=postgres://app_user:secret@db.internal:5432/app_production

# MySQL
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production

# SQLite (para deploys únicos e pequenos)
DATABASE_URL=sqlite:///var/lib/myapp/data.db
```

### Por que Suprnova diverge

Os padrões do Laravel direcionam novos projetos para PostgreSQL porque
PHP + PostgreSQL é o caminho bem trilhado. Suprnova escolhe o banco de
dados que oferece a postura de single-engine mais limpa para um app Rust.
O `VECTOR(N)` (11.7+), Dynamic Columns e tabelas com versionamento de
sistema do MariaDB significam que um produto pequeno-a-médio pode entregar
busca, KV e auditoria sem agregar Redis, OpenSearch ou pgvector. PostgreSQL
permanece totalmente suportado - a matriz de testes do framework executa
contra todos os três backends relacionais - mas nossa documentação de
implantação leva com o engine que minimiza moving parts. Veja
[Armazenamento de Vetor](vector.md) e [Banco de dados](database.md) para
as superfícies específicas de backend.

## Construindo uma imagem de produção

O scaffolder fornece um gerador para um Dockerfile multi-estágio:

```bash
suprnova docker:init
```

Isso escreve um `Dockerfile` com três estágios:

1. **Build do frontend** - `node:20-alpine`, executa `npm ci && npm run build`
   contra seu app Inertia `frontend/` (Svelte 5, React 19 ou Vue 3.5
   conforme sua escolha de scaffold).
2. **Build do backend** - `rust:1.91.1-slim-bookworm`, compila seu crate
   em modo release com cache de dependências.
3. **Runtime** - `debian:bookworm-slim`, copia o binário compilado e a
   saída do Vite, executa como `appuser` sem privilégios de root, expõe
   a porta 8765 e executa `CMD ["./app"]` (o servidor auto-migrador).

Construa e execute localmente para verificar antes de fazer push:

```bash
docker build -t myapp .

# Com um arquivo env
docker run --rm -p 8765:8765 --env-file .env.production myapp

# Ou com vars explícitas (as quatro obrigatórias)
docker run --rm -p 8765:8765 \
  -e APP_ENV=production \
  -e APP_KEY=$APP_KEY \
  -e APP_URL=https://app.example.com \
  -e DATABASE_URL=mysql://user:pass@host:3306/app \
  myapp
```

Nunca faça commit de `.env.production` (ou qualquer arquivo contendo
`APP_KEY` ou `DATABASE_URL`) em seu repositório. Use o armazenamento de
segredos de sua plataforma e leia os valores no momento do deploy.

## Migrações ao inicializar

O comando padrão `./app` (e explícito `./app serve`) aplica todas as migrações
pendentes antes de vincular o socket. As duas implicações práticas:

- **Seguro com múltiplas instâncias.** O executor de migrações do SeaORM
  usa um bloqueio consultivo no nível de banco de dados; o pod mais
  lento espera, os outros procedem assim que termina. Você não precisa
  de uma etapa separada "migrate-then-deploy" para rollouts de release
  rotineiros.
- **Falha de migração = falha de deploy.** Se uma migração errar, o processo
  sai com código não-zero antes do servidor se vincular. A sonda de saúde
  da plataforma (veja abaixo) relata o pod como não saudável e o rollout
  para. Corrija avançando enviando uma migração corretiva na próxima release.

Para pipelines de CI que desejam validar o deploy em uma migração bem-sucedida
antes de qualquer pod aceitar tráfego, execute migrações em uma única execução:

```bash
docker run --rm myapp ./app migrate
# … depois faça o deploy de verdade
docker run myapp ./app serve --no-migrate
```

`--no-migrate` pula a fase de auto-migrate, mas ainda inicializa o servidor
normalmente.

## Workers como serviços separados

Os sistemas de agendador, fila e fluxo de trabalho cada um tem seu próprio
subcomando daemon. Em produção, execute-os como processos separados contra
a mesma imagem, compartilhando o mesmo ambiente:

```bash
docker run myapp ./app schedule:work    # uma instância - veja abaixo
docker run myapp ./app queue:work       # escale para N instâncias
docker run myapp ./app workflow:work    # escale para N instâncias
```

Duas regras para internalizar:

- **Ou execute exatamente um processo `schedule:work` ou marque suas tarefas
  `.on_one_server()`.** Réplicas de agendador não se coordenam por padrão:
  cada uma avalia o agendamento de forma independente, portanto três réplicas
  executam cada tarefa devida três vezes. `replicas: 1` é a resposta simples;
  `.on_one_server()` elege uma réplica por tick contra um cache compartilhado
  e é o que você quer se o agendador precisa estar altamente disponível.
  Veja [Agendamento](scheduling.md#running-on-one-server).
- **Workers de fila e fluxo de trabalho escalam horizontalmente.** Ambos
  puxam trabalho de um armazenamento compartilhado e usam timeouts de
  visibilidade ou locks no nível de linha para coordenar; adicionar pods
  adiciona throughput. `./app queue:work --max-jobs N` faz o worker sair
  após N jobs para que um supervisor possa rotacionar o processo - útil
  para deploys release-on-restart.

Veja [Filas](queues.md), [Agendamento](scheduling.md) e
[Fluxos de trabalho](workflows.md) para detalhes por subsistema.

## Parando com elegância

Todo processo Suprnova de longa duração - o servidor e todos os três daemons -
drena em **SIGTERM** bem como SIGINT. SIGTERM é o que `docker stop`, Coolify,
systemd e Kubernetes enviam; SIGINT é o que Ctrl-C envia. Ambos tomam o mesmo
caminho: parar de aceitar novo trabalho, terminar o que está em voo dentro de
uma graça limitada, sair com `0`.

As janelas de graça são por subsistema e limitadas propositalmente - um cliente
lento ou uma tarefa longa não deve ser capaz de manter um processo vivo
indefinidamente:

| Processo | Espera por | Graça |
|---|---|---|
| `serve` | conexões HTTP em voo | 5s |
| `queue:work` | o job em voo liquidar | até o job retornar |
| `schedule:work` | tarefas `.run_in_background()` | 30s |
| `workflow:work` | passos de fluxo de trabalho em voo | até retornarem |

**Dimensione o período de graça de terminação de sua plataforma acima destes.**
Docker padrão é 10 segundos, Kubernetes é 30. Se a janela da plataforma é mais
curta que o trabalho leva, ele envia SIGKILL e você volta a perder jobs em voo:

```yaml
# docker compose
services:
  worker:
    command: ["app", "queue:work"]
    stop_grace_period: 60s
```

```yaml
# kubernetes
spec:
  terminationGracePeriodSeconds: 60
```

**Um job morto a meio caminho não é perdido, mas custa uma tentativa.** Sua
reserva expira e outro worker a reclama, cobrando uma tentativa de modo que um
job que confiável mata seu worker ainda pode ser dead-lettered em vez de ciclar
para sempre. Veja [Filas](queues.md#what-counts-as-an-attempt).

**PID 1 é uma restrição real.** Um entrypoint de contêiner executa como PID 1 e
o kernel não aplica disposições de sinal padrão a PID 1 - um processo sem
handler SIGTERM não morre em SIGTERM, ignora até que a plataforma desista
e envie SIGKILL. Suprnova instala o handler, portanto `CMD ["app",
"queue:work"]` funciona conforme escrito e nenhum shim `tini` é necessário.

## Verificação de saúde

Suprnova expõe três caminhos de saúde integrados. O prefixo `_suprnova/`
é reservado para que suas próprias rotas nunca possam colidir com elas.

| Caminho | Toca | Usar para |
|---|---|---|
| `/_suprnova/health/live` | nada | Vivacidade. Responde 200 enquanto o processo pode servir uma solicitação. |
| `/_suprnova/health/ready` | o banco de dados | Prontidão. 503 quando uma dependência é inatingível. |
| `/_suprnova/health` | nada, ou o banco de dados com `?db=true` | O endpoint original. Se comporta como qualquer um dos acima. |

```bash
curl http://localhost:8765/_suprnova/health/live
# 200 {"status":"ok","timestamp":"2026-05-30T12:34:56+00:00"}

curl http://localhost:8765/_suprnova/health/ready
# Saudável:  200 {"status":"ok","timestamp":"…","database":"connected"}
# Degradado: 503 {"status":"degraded","timestamp":"…","database":"error"}
```

`/_suprnova/health` e `/_suprnova/health?db=true` continuam funcionando
exatamente como antes e nada que você já implantou precisa mudar - o
[guia Hetzner](deployment-hetzner.md) ainda os nomeia para verificações
pontuais e assim podem suas próprias especificações. Os caminhos nomeados
são mais claros, portanto prefira-os em nova configuração; os guias
[Railway](deployment-railway.md), [DigitalOcean](deployment-digital-ocean.md)
e [Docker](cli-docker.md) os usam.

### Use a sonda correta para a pergunta certa

Aponte vivacidade para `/live` e prontidão para `/ready`. A distinção importa
mais do que parece: uma sonda **vivacidade** que falha reinicia o pod, enquanto
uma sonda **prontidão** que falha apenas o puxa do load balancer. Conecte uma
verificação de banco de dados à vivacidade e um problema de banco de dados
reinicia cada réplica que você tem - no momento exato em que o banco de dados
menos pode permitir um thundering herd de reconexões.

```yaml
livenessProbe:
  httpGet:
    path: /_suprnova/health/live
    port: 8765
readinessProbe:
  httpGet:
    path: /_suprnova/health/ready
    port: 8765
```

O endpoint faz shortcircuit antes da cadeia de middleware portanto permanece
responsivo mesmo se um middleware ficar em deadlock ou o middleware de id de
solicitação estiver rejeitando tráfego.

### Respostas degradadas não carregam detalhes do driver

O corpo 503 relata `"database":"error"` e nada mais. A mensagem própria do
driver - que nomeia hosts, portas, nomes de banco de dados e esquemas e versões
de servidor, e para alguns erros de configuração a URL de conexão - vai para o
log no nível `error!`, onde um operador pode lê-lo e um estranho não. Em builds
de debug também é incluída no corpo como `database_error`, portanto o debugging
local não é afetado.

### Fechando a prontidão

Prontidão executa uma viagem redonda de banco de dados para quem pedir. Se o
endpoint for internet-acessível, defina um segredo compartilhado:

```bash
SERVER_HEALTH_READINESS_TOKEN=<a long random string>
```

As sondas devem então enviá-lo como um cabeçalho:

```bash
curl -H "X-Suprnova-Health-Token: $SERVER_HEALTH_READINESS_TOKEN" \
  http://localhost:8765/_suprnova/health/ready
```

```yaml
readinessProbe:
  httpGet:
    path: /_suprnova/health/ready
    port: 8765
    httpHeaders:
      - name: X-Suprnova-Health-Token
        value: <the same value>
```

Sem o cabeçalho, prontidão responde **404** - a mesma resposta de qualquer
caminho que não existe, portanto o endpoint é invisível em vez de meramente
fechado. Vivacidade permanece pública de qualquer forma, portanto você não
precisa colocar o segredo em cada manifesto para manter seu sinal de
restart-on-hang.

Não definido é o padrão e prontidão é pública. Isso é deliberado: as
configurações que este manual e o scaffolder geram todas chamam `?db=true`
sem um cabeçalho, e o padrão de fechado os quebraria.

## Modo de manutenção

Para fazer rollback de uma migração destrutiva ou encerrar tráfego por um
incidente:

```bash
./app down --secret abc123 \
           --retry 60 \
           --message "Deploying - back in a few minutes" \
           --except /webhooks/stripe

./app up
```

`down` escreve um marcador de manutenção que o middleware lê em cada
solicitação. As solicitações recebem um 503 (configurável via `--status`)
com a mensagem fornecida, exceto para caminhos em `--except` e qualquer
solicitação que inclua o segredo. `up` remove o marcador.

## Escalando

### Web

O escalamento horizontal é a história padrão: cada pod executa `./app`,
compartilha `DATABASE_URL` e se conecta ao mesmo Redis (se você tiver
configurado cache/queue/session suportado por Redis). Auto-migrate é
seguro por causa do bloqueio consultivo acima. Sessões pegajosas não
são necessárias - o estado da sessão vive em seu driver de sessão
(banco de dados ou Redis), não na memória do processo.

### Workers

- **Agendador.** Exatamente uma instância, sempre.
- **Fila.** Escale horizontalmente. Se você tiver dividido trabalho entre
  múltiplas filas nomeadas, execute um worker por fila (ou passe filtros
  de fila específicos do driver - veja [Filas](queues.md)).
- **Fluxo de trabalho.** Escale horizontalmente; afirmação no nível de
  linha/heartbeat coordena os workers.

## Limite de conexão (`SERVER_MAX_CONNECTIONS`)

Por padrão o servidor aceita um número ilimitado de conexões TCP concorrentes.
Na maioria dos deployments um proxy reverso (nginx, Caddy, Traefik) ou o
load balancer da plataforma fornece a primeira linha de defesa. Se você
deseja um backstop duro dentro do próprio processo - para evitar que um único
pool de cliente malcomportado esgote descritores de arquivo - defina
`SERVER_MAX_CONNECTIONS`:

```bash
# .env.production - limita as conexões concorrentes a 1024
SERVER_MAX_CONNECTIONS=1024
```

Quando o limite é atingido o **loop de aceitação bloqueia** (contrapressão
no nível TCP) até que uma conexão existente feche; o aperto de mão pendente
permanece na fila de aceitação do kernel. A permissão é mantida durante toda
a vida útil de cada conexão e liberada no momento em que a conexão termina,
portanto os slots giram prontamente.

Regras práticas:

- **Não definido (padrão = ilimitado).** Correto se você tem um proxy reverso
  aplicando seu próprio limite de conexão ou se está executando atrás de um
  PaaS que gerencia concorrência para você.
- **Defina como um valor concreto** se o processo for executado diretamente
  na internet ou se você deseja defesa em profundidade independentemente da
  configuração de proxy. Um ponto de partida típico é 2 × seus usuários
  simultâneos de pico esperados, ajustado para cima para conexões de longa
  duração (WebSocket, SSE).
- **Combine com `LimitNOFILE`** (systemd) ou `ulimit -n` para que o limite
  de descritor de arquivo do SO não se torne o limite surpresa. Cada conexão
  HTTP custa um descritor de arquivo; adicione o tamanho do seu pool de banco
  de dados e algumas dezenas para manutenção do SO.
- **Isso é um backstop, não um substituto para limitação de taxa upstream.**
  `SERVER_MAX_CONNECTIONS` interrompe acumulação descontrolada; seu proxy
  reverso ou middleware `rate_limit` deve lidar com aceleração por cliente
  ou por IP.

Valores em branco, não analisáveis ou zero são silenciosamente tratados como
não definidos de modo que um erro de digitação não impeça o servidor de
iniciar.

## Instruções específicas de plataforma

A receita acima se adapta a todo PaaS ou VPS moderno. Os próximos três
capítulos o guiam pelas especificidades:

| Plataforma | Estilo | Instrução |
|---|---|---|
| Railway | PaaS com auto-deploy do git | [Implantar no Railway](deployment-railway.md) |
| Digital Ocean | App Platform (PaaS) ou Droplets (VPS) | [Implantar no Digital Ocean](deployment-digital-ocean.md) |
| Hetzner | VPS com systemd + Caddy | [Implantar no Hetzner](deployment-hetzner.md) |

## Próximos passos

- [Variáveis de ambiente](env-vars.md) - toda var env que o framework lê
- [Criptografia](encryption.md) - `APP_KEY`, rotação de chave, o que é criptografado
- [Configuração](configuration.md) - seções de config digitadas construídas em env
- [Banco de dados](database.md) - seleção de driver, ajuste de pool, divisão multi-conexão
- [Filas](queues.md) - escalamento de worker e drivers de fila

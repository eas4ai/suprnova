# Implantar no Hetzner VPS

Este guia cobre a implantação de uma aplicação Suprnova em uma VPS
usando o Hetzner Cloud. Os mesmos princípios se aplicam a qualquer
host de máquina única - Linode, Vultr, AWS EC2, ou um servidor
dedicado que você já possui. Escolha este caminho quando você quiser
controle total da máquina, custo mensal previsível, e a capacidade de
colocar Postgres / Redis na mesma máquina.

Ao longo do guia usamos `myapp` como o nome do projeto e `myapp.com`
como o domínio - substitua pelos seus.

## Pré-requisitos

- Uma VPS rodando Ubuntu 22.04 ou Debian 12
- Acesso SSH ao seu servidor
- Um nome de domínio apontado para o endereço IP do seu servidor
- Um projeto Suprnova - seja uma árvore de código-fonte funcional, ou
  um Dockerfile gerado com `suprnova docker:init` (veja
  [Docker](cli-docker.md))

## Configuração do servidor

### 1. Criando uma VPS

1. Vá para o [Hetzner Cloud Console](https://console.hetzner.cloud)
2. Crie um novo projeto e adicione um servidor
3. Escolha **Ubuntu 22.04** como a imagem
4. Selecione o tamanho do seu servidor (CX11 é suficiente para apps
   pequenos)
5. Adicione sua chave SSH para acesso seguro

### 2. Configuração inicial do servidor

Conecte-se via SSH ao seu servidor e rode a configuração inicial:

```bash
# Atualizar pacotes
apt update && apt upgrade -y

# Criar um usuário sem privilégios de root para seu app
useradd -m -s /bin/bash app
mkdir -p /opt/myapp
chown app:app /opt/myapp

# Instalar pacotes necessários
apt install -y curl postgresql redis-server
```

### 3. Configurando o PostgreSQL

```bash
# Criar banco de dados e usuário
sudo -u postgres psql << EOF
CREATE USER myapp WITH PASSWORD 'your_secure_password';
CREATE DATABASE myapp_production OWNER myapp;
GRANT ALL PRIVILEGES ON DATABASE myapp_production TO myapp;
EOF
```

> **Dica:**
>
> Para produção, considere usar um serviço de banco de dados
> gerenciado como o futuro PostgreSQL gerenciado da Hetzner, ou
> serviços como Neon, Supabase, ou AWS RDS para melhor confiabilidade
> e backups.


## Opções de deploy

Escolha um dos métodos de deploy a seguir. Cada um termina com um
binário (ou contêiner) chamado `app` situado em `/opt/myapp/app`, que
a unidade systemd abaixo sabe como rodar.

### Opção A: Compilando localmente

Compile na sua máquina e envie o binário. Substitua `myapp` pelo nome
real do seu projeto - o `cargo build` nomeia o binário de acordo com
o `[package].name` no `Cargo.toml`:

```bash
# Na sua máquina local - cross-compile para Linux (se estiver no macOS)
cargo build --release --target x86_64-unknown-linux-gnu

# Ou compile com Docker para Linux (o Dockerfile renomeia o binário para `app`)
docker build -t myapp .
docker create --name temp myapp
docker cp temp:/app/app ./app-linux
docker rm temp

# Enviar para o servidor, renomeando para `app` ao chegar
scp target/x86_64-unknown-linux-gnu/release/myapp root@your-server:/opt/myapp/app
# ou, se você seguiu o caminho do Docker:
scp ./app-linux root@your-server:/opt/myapp/app
```

### Opção B: Compilando no servidor

Instale Rust 1.94.0+ para a `main` atual (o Suprnova usa a edição 2024) e faça o build diretamente no servidor:

```bash
# Instalar o Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Clonar, compilar, e colocar o binário no caminho padrão
cd /opt/myapp
git clone https://github.com/your-username/your-repo.git .
cargo build --release
cp target/release/myapp ./app   # renomear para que o ExecStart=/opt/myapp/app do systemd o encontre
```

A `main` atual usa SeaORM 2.0, SeaQuery 1.0 e SQLx 0.9. Aplicações que chamam SeaORM diretamente devem importar `ExprTrait` para os métodos de expressão do SeaQuery e usar métodos de conexão `*_raw` explícitos para valores `Statement` pré-construídos. A atualização das dependências não exige nenhuma migração de dados da aplicação.

### Opção C: Usando Docker

Rode seu app em um contêiner Docker - o Dockerfile com scaffold já
nomeia o binário de runtime como `app` (veja [Docker](cli-docker.md)):

```bash
# Instalar o Docker
curl -fsSL https://get.docker.com | sh

# Baixar e rodar sua imagem
docker run -d \
  --name myapp \
  --restart unless-stopped \
  -p 8765:8765 \
  --env-file /opt/myapp/.env.production \
  your-registry/myapp:latest
```

Se você seguiu com Docker, pule a seção do systemd e vá para
[Proxy reverso Caddy](#proxy-reverso-caddy) - o Docker cuida da
supervisão de processos.

## Configuração do ambiente

Primeiro, gere um `APP_KEY` de produção no servidor (ou localmente -
o que importa é o valor). `APP_KEY` é uma chave AES-256 de 32 bytes
usada pelo `suprnova::Crypt` para cookies de sessão e URLs assinadas.
O Suprnova **falha fechado ao inicializar** quando `APP_ENV` não é
`local`/`dev`/`test` e `APP_KEY` está indefinido - então isso não é
opcional em produção:

```bash
suprnova key:generate --show
# -> APP_KEY=base64-url-safe-32-bytes
```

Em seguida, escreva o arquivo de env:

```bash
cat > /opt/myapp/.env.production << 'EOF'
APP_NAME="My App"
APP_ENV=production
APP_DEBUG=false
APP_URL=https://myapp.com
APP_KEY=paste-the-generated-key-here

SERVER_HOST=127.0.0.1
SERVER_PORT=8765

# Banco de dados - vincule a localhost quando o BD estiver na mesma máquina
DATABASE_URL=postgres://myapp:your_secure_password@localhost:5432/myapp_production
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1

# Sessão
SESSION_SECURE=true
SESSION_SAME_SITE=Lax

# Redis (opcional - usado pelos drivers de cache, fila, transmissão)
REDIS_URL=redis://127.0.0.1:6379

# Correio
MAIL_DRIVER=smtp
MAIL_HOST=your-smtp-host
MAIL_PORT=587
MAIL_USERNAME=
MAIL_PASSWORD=
MAIL_FROM_ADDRESS=hello@myapp.com
MAIL_FROM_NAME="My App"
EOF

# Proteger o arquivo - somente o usuário app deve conseguir lê-lo
chmod 600 /opt/myapp/.env.production
chown app:app /opt/myapp/.env.production
```

Veja [Configuração](configuration.md) para a superfície completa de
env e como ela se torna config tipada.

## Serviços systemd

Um binário Suprnova suporta múltiplos comandos - `./app` (serve, com
auto-migrate), `./app schedule:work` (daemon do agendador), `./app
queue:work` (worker de fila), `./app workflow:work` (executor de
fluxo de trabalho). Cada processo de longa duração recebe sua própria
unidade systemd usando o mesmo binário e arquivo de env.

### Serviço do servidor web

Crie `/etc/systemd/system/myapp.service`:

```ini
[Unit]
Description=Suprnova Application
After=network.target postgresql.service redis.service
Requires=postgresql.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app
Restart=always
RestartSec=5

# Ambiente
EnvironmentFile=/opt/myapp/.env.production

# Reforço de segurança
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

O `ExecStart=/opt/myapp/app` padrão roda `serve` com auto-migração.
Se você preferir que as migrações sejam uma etapa de deploy separada,
use `ExecStart=/opt/myapp/app serve --no-migrate` e rode `./app
migrate` a partir do seu script de deploy antes de trocar o binário.

### Serviço do agendador

Se seu app tem tarefas registradas via `Schedule::call(...)` (veja o
capítulo [Agendamento](cli-scheduling.md)), rode **exatamente um**
processo de agendador para evitar execução duplicada de tarefas. Crie
`/etc/systemd/system/myapp-scheduler.service`:

```ini
[Unit]
Description=Suprnova Scheduler
After=network.target myapp.service
Requires=myapp.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app schedule:work
Restart=always
RestartSec=5

# Ambiente
EnvironmentFile=/opt/myapp/.env.production

# Reforço de segurança
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

### Worker de fila (opcional)

Se você despacha jobs para uma fila, adicione
`/etc/systemd/system/myapp-queue.service`:

```ini
[Unit]
Description=Suprnova Queue Worker
After=network.target myapp.service
Requires=myapp.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app queue:work
Restart=always
RestartSec=5

EnvironmentFile=/opt/myapp/.env.production

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

Você pode escalar workers de fila horizontalmente - múltiplas
instâncias de `myapp-queue.service` na mesma máquina ou em máquinas
diferentes é seguro.

### Habilitando e iniciando os serviços

```bash
# Recarregar o systemd depois de escrever os arquivos de unidade
systemctl daemon-reload

# Habilitar os serviços para que iniciem no boot
systemctl enable myapp
systemctl enable myapp-scheduler
systemctl enable myapp-queue        # se você adicionou o worker de fila

# Iniciá-los agora
systemctl start myapp
systemctl start myapp-scheduler
systemctl start myapp-queue

# Verificar
systemctl status myapp
systemctl status myapp-scheduler
systemctl status myapp-queue
```

## Proxy reverso Caddy

O Caddy lida automaticamente com certificados HTTPS usando o Let's
Encrypt.

### Instalando o Caddy

```bash
apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list
apt update
apt install caddy
```

### Configurando o Caddy

Edite o `/etc/caddy/Caddyfile`:

```
myapp.com {
    reverse_proxy localhost:8765

    # Habilitar compressão
    encode gzip

    # Logs
    log {
        output file /var/log/caddy/myapp.log
    }
}
```

Substitua `myapp.com` pelo seu domínio real.

### Iniciando o Caddy

```bash
systemctl enable caddy
systemctl start caddy
```

O Caddy vai obter e renovar certificados SSL automaticamente.

## Verificações de saúde

O Suprnova vem com um endpoint `/_suprnova/health` integrado que faz
shortcircuit antes da cadeia de middleware e nunca colide com suas
rotas:

```bash
curl https://myapp.com/_suprnova/health
```

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z"
}
```

### Verificando a conectividade do banco de dados

Adicione `?db=true` para também verificar o banco de dados:

```bash
curl https://myapp.com/_suprnova/health?db=true
```

Resposta saudável (HTTP 200):

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z",
  "database": "connected"
}
```

Se a verificação do banco de dados falhar, o endpoint muda para HTTP
**503** com `"status": "degraded"` e um campo `"database_error"` -
conecte isso a uma verificação de saúde no estilo `livenessProbe` /
`readinessProbe` para que o load balancer possa remover uma instância
não saudável da rotação.

### Monitoramento externo

Use o endpoint de saúde com serviços de monitoramento:

- **UptimeRobot**: adicione um monitor HTTP para
  `https://myapp.com/_suprnova/health`
- **Better Stack** (antigo Better Uptime): configure o endpoint de
  verificação de saúde com o gatilho 503
- **Prometheus / Grafana**: faça scrape do corpo JSON pelos campos
  `status` + `database`

## Script de deploy

Crie um script de deploy para atualizações atômicas. Substitua
`myapp` pelo nome do seu projeto (o `[package].name` no
`Cargo.toml`) - é assim que o `cargo build` nomeia o binário de
saída:

```bash
#!/bin/bash
# deploy.sh - Rode na sua máquina local

set -e

PROJECT="myapp"               # o nome do pacote Cargo
SERVER="root@your-server"
APP_PATH="/opt/myapp"
BIN="target/x86_64-unknown-linux-gnu/release/$PROJECT"

echo "Compilando a aplicação..."
cargo build --release --target x86_64-unknown-linux-gnu

echo "Enviando o binário..."
scp "$BIN" "$SERVER:$APP_PATH/app.new"

echo "Implantando..."
ssh "$SERVER" << 'EOF'
    set -e
    cd /opt/myapp

    # Parar os serviços de longa duração (ignorar falhas no primeiro deploy)
    systemctl stop myapp-queue || true
    systemctl stop myapp-scheduler || true
    systemctl stop myapp

    # Troca atômica - renomear é uma única syscall no mesmo sistema de arquivos
    mv app.new app
    chmod +x app

    # Rodar as migrações explicitamente (a unidade também faz auto-migrate,
    # mas fazer isso aqui expõe falhas antes de reiniciarmos o tráfego)
    sudo -u app ./app migrate

    # Iniciar os serviços
    systemctl start myapp
    systemctl start myapp-scheduler || true
    systemctl start myapp-queue || true

    # Verificar a saúde (dar um momento para o servidor vincular)
    sleep 2
    curl -fsS http://localhost:8765/_suprnova/health?db=true > /dev/null || exit 1

    echo "Deploy concluído!"
EOF
```

Torne-o executável:

```bash
chmod +x deploy.sh
./deploy.sh
```

## Logs e monitoramento

### Visualizando logs

```bash
# Logs do servidor web
journalctl -u myapp -f

# Logs do agendador
journalctl -u myapp-scheduler -f

# Logs de acesso do Caddy
tail -f /var/log/caddy/myapp.log
```

### Rotação de logs

O journald do systemd lida com a rotação de logs automaticamente.
Para armazenamento de longo prazo, considere:

- **Loki + Grafana**: agregação de logs auto-hospedada
- **Papertrail**: serviço de logs baseado em nuvem
- **Logtail**: gerenciamento simples de logs

## Configuração de firewall

Proteja seu servidor com o UFW:

```bash
# Permitir SSH
ufw allow 22/tcp

# Permitir HTTP/HTTPS (Caddy)
ufw allow 80/tcp
ufw allow 443/tcp

# Habilitar o firewall
ufw enable
```

> **Aviso:**
>
> Nunca exponha a porta 8765 diretamente. Sempre use o Caddy como um
> proxy reverso para lidar com SSL e headers de segurança.


## Escalando

Um único binário Suprnova é muito eficiente - uma VPS pequena lida
com uma quantidade surpreendente de tráfego antes que você precise
escalar horizontalmente. Quando precisar:

### Escalonamento vertical

Atualize a VPS para uma instância maior para mais CPU/memória. O
binário, o arquivo de env, e as unidades systemd vêm com você
inalterados.

### Escalonamento horizontal

Para múltiplas instâncias de aplicação:

1. Configure um load balancer (Hetzner Load Balancer, HAProxy, ou
   Caddy em um nó dedicado)
2. Mova o Postgres para um serviço gerenciado ou um nó dedicado para
   que as máquinas do app sejam stateless
3. Mova sessões, cache, e transmissão para o Redis para que qualquer
   instância do app possa atender qualquer solicitação
4. Implante múltiplas instâncias do app; cada uma roda seu próprio
   auto-migrate com segurança na inicialização (o executor de
   migrações usa um lock para que boots concorrentes não colidam)
5. Mantenha **um** agendador (`schedule:work`) rodando em toda a
   frota - workers de fila são seguros para rodar em paralelo, o
   agendador não é

### Por que Suprnova diverge

O Laravel tipicamente roda PHP-FPM atrás do nginx, com um cron
disparando `schedule:run` uma vez por minuto e o Horizon (ou
supervisord) gerenciando os workers de fila. O Suprnova reduz tudo
isso a um único binário com subcomandos. `./app` é um processo Tokio
de longa duração - ele não precisa de um pool de processos na
frente, não precisa de um cron separado, e permanece quente entre
solicitações. O systemd é o supervisor tanto para o processo web
quanto para os workers, e o Caddy faz apenas o que o nginx não
conseguia evitar: terminar o TLS e fazer proxy.

## Dimensionando

Escolha uma VPS baseado na carga de trabalho, não em um nome de tier
de marketing. A linha da Hetzner muda periodicamente; a lógica de
dimensionamento não muda:

| Carga de trabalho | Ajuste aproximado |
|---|---|
| Site pequeno, baixo tráfego, SQLite ou BD compartilhado | Menor instância de vCPU compartilhada (1 vCPU / 2 GB) |
| Tráfego moderado com Postgres + Redis na mesma máquina | 2 vCPU / 4 GB |
| API mais pesada + agendador + workers de fila + Postgres | 2–4 vCPU / 8 GB |
| Produção em escala | Instância de CPU dedicada, ou divida o BD para seu próprio nó |

Verifique os [preços atuais da Hetzner](https://www.hetzner.com/cloud)
para o catálogo ao vivo. A pegada de memória ociosa do Suprnova é
pequena (MB de um único dígito), então a RAM é majoritariamente o
working set do banco de dados mais o seu código de domínio.

## Solução de problemas

### O serviço não inicia

Verifique os logs em busca de erros:

```bash
journalctl -u myapp -n 50
```

Problemas comuns:
- Variáveis de ambiente ausentes
- Falha na conexão com o banco de dados
- Porta já em uso

### Erros de certificado do Caddy

Certifique-se de que:
- O DNS do domínio aponta para o seu servidor
- As portas 80 e 443 estão abertas
- Nenhum outro serviço está usando a porta 80

```bash
caddy validate --config /etc/caddy/Caddyfile
```

### Problemas de conexão com o banco de dados

Teste a conexão manualmente:

```bash
sudo -u app psql $DATABASE_URL -c "SELECT 1"
```

### Falha na verificação de saúde

```bash
# Verificar se o app está rodando
systemctl status myapp

# Testar o endpoint de saúde diretamente
curl http://localhost:8765/_suprnova/health

# Verificar com o banco de dados
curl http://localhost:8765/_suprnova/health?db=true
```

Uma resposta `503` com `"status": "degraded"` significa que o app
está no ar mas a verificação de saúde do banco de dados falhou -
inspecione `database_error` no corpo e verifique o `DATABASE_URL`,
os logs do Postgres, e os limites de conexão.

## Próximos passos

- [Visão geral de implantação](deployment.md) - a história
  independente de plataforma para deploys de binário único
- [Docker](cli-docker.md) - detalhes de `docker:init` e
  `docker:compose`
- [Configuração](configuration.md) - superfície completa de env e
  config tipada
- [Implantar no Railway](deployment-railway.md) - alternativa PaaS
  com builds automáticos
- [Implantar no Digital Ocean](deployment-digital-ocean.md) - App
  Platform com infraestrutura gerenciada

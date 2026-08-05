# 部署到 Hetzner VPS

本指南介绍如何用 Hetzner Cloud，把一个 Suprnova 应用部署到一台 VPS 上。同样的原则适用于任何单机型主机 - Linode、Vultr、AWS EC2，或者您已经拥有的一台专用服务器。当您想要完全掌控这台机器、可预测的月度成本，以及把 Postgres / Redis 和应用放在同一台机器上的能力时，就选这条路径。

整份指南中，我们用 `myapp` 作为项目名，`myapp.com` 作为域名 - 请替换成您自己的。

## 前提条件

- 一台运行 Ubuntu 22.04 或 Debian 12 的 VPS
- 到您服务器的 SSH 访问权限
- 一个指向您服务器 IP 地址的域名
- 一个 Suprnova 项目 - 一份能工作的源代码树，或者一个用 `suprnova docker:init` 生成的 Dockerfile（参见 [Docker](cli-docker.md)）

## 服务器搭建

### 1. 创建一台 VPS

1. 前往 [Hetzner Cloud 控制台](https://console.hetzner.cloud)
2. 创建一个新项目，并添加一台服务器
3. 选择 **Ubuntu 22.04** 作为镜像
4. 选择您的服务器规格（对小应用来说 CX11 就够用）
5. 添加您的 SSH 密钥以实现安全访问

### 2. 初始服务器配置

SSH 进您的服务器，运行初始设置：

```bash
# 更新软件包
apt update && apt upgrade -y

# 为您的应用创建一个非 root 用户
useradd -m -s /bin/bash app
mkdir -p /opt/myapp
chown app:app /opt/myapp

# 安装所需的软件包
apt install -y curl postgresql redis-server
```

### 3. 配置 PostgreSQL

```bash
# 创建数据库和用户
sudo -u postgres psql << EOF
CREATE USER myapp WITH PASSWORD 'your_secure_password';
CREATE DATABASE myapp_production OWNER myapp;
GRANT ALL PRIVILEGES ON DATABASE myapp_production TO myapp;
EOF
```

> **提示：**
>
> 对于生产环境，可以考虑使用一个托管数据库服务，比如 Hetzner 即将推出的托管 PostgreSQL，或者 Neon、Supabase、AWS RDS 这类服务，以获得更好的可靠性和备份能力。


## 部署选项

从下面的部署方法中选择一种。每一种方法最后都会得到一个名为 `app` 的二进制文件（或者容器），位于 `/opt/myapp/app`，下面的 systemd 单元知道如何运行它。

### 选项 A：本地构建

在您自己的机器上构建，然后上传这个二进制文件。把 `myapp` 换成您实际的项目名 - `cargo build` 会以 `Cargo.toml` 里的 `[package].name` 来给这个二进制文件命名：

```bash
# 在您本地的机器上 - 为 Linux 交叉编译（如果您在 macOS 上）
cargo build --release --target x86_64-unknown-linux-gnu

# 或者用 Docker 为 Linux 构建（这个 Dockerfile 会把这个二进制文件改名为 `app`）
docker build -t myapp .
docker create --name temp myapp
docker cp temp:/app/app ./app-linux
docker rm temp

# 上传到服务器，落地时改名为 `app`
scp target/x86_64-unknown-linux-gnu/release/myapp root@your-server:/opt/myapp/app
# 或者，如果您走的是 Docker 这条路：
scp ./app-linux root@your-server:/opt/myapp/app
```

### 选项 B：在服务器上构建

安装 Rust 1.91.1+（Suprnova 使用 2024 版本），直接在服务器上构建：

```bash
# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# 克隆、构建，并把这个二进制文件放到标准路径上
cd /opt/myapp
git clone https://github.com/your-username/your-repo.git .
cargo build --release
cp target/release/myapp ./app   # 改名，这样 systemd 的 ExecStart=/opt/myapp/app 才能找到它
```

### 选项 C：使用 Docker

在一个 Docker 容器里运行您的应用 - 脚手架生成的 Dockerfile 已经把这个运行时二进制文件命名为 `app`（参见 [Docker](cli-docker.md)）：

```bash
# 安装 Docker
curl -fsSL https://get.docker.com | sh

# 拉取并运行您的镜像
docker run -d \
  --name myapp \
  --restart unless-stopped \
  -p 8765:8765 \
  --env-file /opt/myapp/.env.production \
  your-registry/myapp:latest
```

如果您走的是 Docker 这条路，跳过 systemd 那一节，直接到 [Caddy 反向代理](#caddy-反向代理) - Docker 会负责进程监督。

## 环境配置

首先，在服务器上（或者本地 - 重要的是这个值本身）生成一个生产环境的 `APP_KEY`。`APP_KEY` 是一个 32 字节的 AES-256 密钥，供 `suprnova::Crypt` 用于会话 cookie 和已签名的 URL。当 `APP_ENV` 不是 `local`/`dev`/`test`，且 `APP_KEY` 未设置时，Suprnova **会在启动时关闭失败** - 所以在生产环境中这不是可选项：

```bash
suprnova key:generate --show
# -> APP_KEY=base64-url-safe-32-bytes
```

接下来，写入这个 env 文件：

```bash
cat > /opt/myapp/.env.production << 'EOF'
APP_NAME="My App"
APP_ENV=production
APP_DEBUG=false
APP_URL=https://myapp.com
APP_KEY=paste-the-generated-key-here

SERVER_HOST=127.0.0.1
SERVER_PORT=8765

# 数据库 - 当数据库和应用在同一台机器上时，绑定到 localhost
DATABASE_URL=postgres://myapp:your_secure_password@localhost:5432/myapp_production
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1

# 会话
SESSION_SECURE=true
SESSION_SAME_SITE=Lax

# Redis（可选 - 供缓存、队列、广播驱动程序使用）
REDIS_URL=redis://127.0.0.1:6379

# 邮件
MAIL_DRIVER=smtp
MAIL_HOST=your-smtp-host
MAIL_PORT=587
MAIL_USERNAME=
MAIL_PASSWORD=
MAIL_FROM_ADDRESS=hello@myapp.com
MAIL_FROM_NAME="My App"
EOF

# 加固这个文件的权限 - 只应该让 app 用户能读取它
chmod 600 /opt/myapp/.env.production
chown app:app /opt/myapp/.env.production
```

完整的 env 表面，以及它如何变成类型化配置，请参见 [配置](configuration.md)。

## systemd 服务

一个 Suprnova 二进制文件支持多个命令 - `./app`（serve，带自动迁移）、`./app schedule:work`（调度器守护进程）、`./app queue:work`（队列工作进程）、`./app workflow:work`（工作流运行器）。每一个长期运行的进程，都用同一个二进制文件和 env 文件，拥有自己的 systemd 单元。

### Web 服务器服务

创建 `/etc/systemd/system/myapp.service`：

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

# 环境
EnvironmentFile=/opt/myapp/.env.production

# 安全加固
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

默认的 `ExecStart=/opt/myapp/app` 会带着自动迁移运行 `serve`。如果您更愿意让迁移成为一个单独的部署步骤，就用 `ExecStart=/opt/myapp/app serve --no-migrate`，并在切换这个二进制文件之前，从您的部署脚本里运行 `./app migrate`。

### 调度器服务

如果您的应用有通过 `Schedule::call(...)` 注册的任务（参见 [调度](cli-scheduling.md) 那一章），就运行**恰好一个**调度器进程，以避免任务被重复执行。创建 `/etc/systemd/system/myapp-scheduler.service`：

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

# 环境
EnvironmentFile=/opt/myapp/.env.production

# 安全加固
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

### 队列工作进程（可选）

如果您把作业分发到一个队列，就添加 `/etc/systemd/system/myapp-queue.service`：

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

您可以水平扩展队列工作进程 - 在同一台或不同的机器上运行多个 `myapp-queue.service` 实例都是安全的。

### 启用并启动这些服务

```bash
# 写完单元文件后重新加载 systemd
systemctl daemon-reload

# 启用这些服务，让它们在启动时自动运行
systemctl enable myapp
systemctl enable myapp-scheduler
systemctl enable myapp-queue        # 如果您添加了队列工作进程

# 现在启动它们
systemctl start myapp
systemctl start myapp-scheduler
systemctl start myapp-queue

# 验证
systemctl status myapp
systemctl status myapp-scheduler
systemctl status myapp-queue
```

## Caddy 反向代理

Caddy 会通过 Let's Encrypt 自动处理 HTTPS 证书。

### 安装 Caddy

```bash
apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list
apt update
apt install caddy
```

### 配置 Caddy

编辑 `/etc/caddy/Caddyfile`：

```
myapp.com {
    reverse_proxy localhost:8765

    # 启用压缩
    encode gzip

    # 日志
    log {
        output file /var/log/caddy/myapp.log
    }
}
```

把 `myapp.com` 换成您实际的域名。

### 启动 Caddy

```bash
systemctl enable caddy
systemctl start caddy
```

Caddy 会自动获取并续期 SSL 证书。

## 健康检查

Suprnova 自带一个内置的 `/_suprnova/health` 端点，会在中间件链之前就短路，并且永远不会和您的路由冲突：

```bash
curl https://myapp.com/_suprnova/health
```

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z"
}
```

### 检查数据库连接

添加 `?db=true`，可以同时验证数据库：

```bash
curl https://myapp.com/_suprnova/health?db=true
```

健康时的响应（HTTP 200）：

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z",
  "database": "connected"
}
```

如果数据库检查失败，这个端点会切换到 HTTP **503**，带着一个 `"status": "degraded"` 和一个 `"database_error"` 字段 - 把它接入一个 `livenessProbe` / `readinessProbe` 风格的健康检查，这样负载均衡器就能把一个不健康的实例从轮转中移出。

### 外部监控

把这个健康端点和监控服务搭配使用：

- **UptimeRobot**：为 `https://myapp.com/_suprnova/health` 添加一个 HTTP 监视器
- **Better Stack**（原 Better Uptime）：配置健康检查端点，带上 503 触发条件
- **Prometheus / Grafana**：抓取 JSON 响应体里的 `status` + `database` 字段

## 部署脚本

创建一个用于原子式更新的部署脚本。把 `myapp` 换成您的项目名（`Cargo.toml` 里的 `[package].name`） - `cargo build` 就是用它来命名输出的二进制文件的：

```bash
#!/bin/bash
# deploy.sh - 在您本地的机器上运行

set -e

PROJECT="myapp"               # 这个 Cargo 包的名字
SERVER="root@your-server"
APP_PATH="/opt/myapp"
BIN="target/x86_64-unknown-linux-gnu/release/$PROJECT"

echo "正在构建应用……"
cargo build --release --target x86_64-unknown-linux-gnu

echo "正在上传二进制文件……"
scp "$BIN" "$SERVER:$APP_PATH/app.new"

echo "正在部署……"
ssh "$SERVER" << 'EOF'
    set -e
    cd /opt/myapp

    # 停止长期运行的服务（首次部署时可以忽略失败）
    systemctl stop myapp-queue || true
    systemctl stop myapp-scheduler || true
    systemctl stop myapp

    # 原子式替换 - 在同一个文件系统上，rename 是单次系统调用
    mv app.new app
    chmod +x app

    # 显式运行迁移（这个单元本身也会自动迁移，但在这里运行一次，
    # 能在我们重新放行流量之前，先暴露出失败）
    sudo -u app ./app migrate

    # 启动服务
    systemctl start myapp
    systemctl start myapp-scheduler || true
    systemctl start myapp-queue || true

    # 验证健康状况（给服务器一点时间来完成绑定）
    sleep 2
    curl -fsS http://localhost:8765/_suprnova/health?db=true > /dev/null || exit 1

    echo "部署完成！"
EOF
```

让它可执行：

```bash
chmod +x deploy.sh
./deploy.sh
```

## 日志和监控

### 查看日志

```bash
# Web 服务器日志
journalctl -u myapp -f

# 调度器日志
journalctl -u myapp-scheduler -f

# Caddy 访问日志
tail -f /var/log/caddy/myapp.log
```

### 日志轮转

systemd 的 journald 会自动处理日志轮转。对于长期存储，可以考虑：

- **Loki + Grafana**：自托管的日志聚合
- **Papertrail**：基于云的日志服务
- **Logtail**：简单的日志管理

## 防火墙配置

用 UFW 加固您的服务器：

```bash
# 允许 SSH
ufw allow 22/tcp

# 允许 HTTP/HTTPS（Caddy）
ufw allow 80/tcp
ufw allow 443/tcp

# 启用防火墙
ufw enable
```

> **警告：**
>
> 永远不要直接暴露 8765 端口。请始终使用 Caddy 作为反向代理，来处理 SSL 和安全相关的请求头。


## 扩展

单个 Suprnova 二进制文件非常高效 - 在您需要扩容之前，一台小型 VPS 能处理的流量多得惊人。当您确实需要扩容时：

### 垂直扩展

把这台 VPS 升级到一个更大的实例，获得更多的 CPU/内存。这个二进制文件、env 文件和 systemd 单元都原封不动地跟着您走。

### 水平扩展

想要多个应用实例：

1. 搭建一个负载均衡器（Hetzner Load Balancer、HAProxy，或者在一个专用节点上跑 Caddy）
2. 把 Postgres 挪到一个托管服务，或者一个专用节点，让应用机器保持无状态
3. 把会话、缓存和广播挪到 Redis，让任何一个应用实例都能服务任何一个请求
4. 部署多个应用实例；每一个都能安全地在启动时运行它自己的自动迁移（迁移运行器会持有一把锁，所以并发的启动不会互相冲突）
5. 在整个集群里，只保留**一个**调度器（`schedule:work`）在运行 - 队列工作进程可以安全地并行运行，但调度器不行

### 为什么 Suprnova 有所不同

Laravel 通常在 nginx 背后运行 PHP-FPM，用 cron 每分钟触发一次 `schedule:run`，并用 Horizon（或者 supervisord）来管理队列工作进程。Suprnova 把这一切都收拢进一个带子命令的二进制文件。`./app` 是一个长期存活的 Tokio 进程 - 它不需要在前面放一个进程池，不需要一个单独的 cron，并且会在多个请求之间保持热态。systemd 同时是 web 进程和这些工作进程的监督程序，而 Caddy 做的，只是 nginx 没法绕开的那部分工作：终止 TLS 和做代理。

## 规格选择

根据工作负载来选择 VPS，而不是根据一个营销层级的名字。Hetzner 的产品线会不时变化；但选型的逻辑不会：

| 工作负载 | 大致适配 |
|---|---|
| 小型站点，低流量，SQLite 或共享数据库 | 最小的共享 vCPU 实例（1 vCPU / 2 GB） |
| 中等流量，Postgres + Redis 在同一台机器上 | 2 vCPU / 4 GB |
| 更重的 API + 调度器 + 队列工作进程 + Postgres | 2–4 vCPU / 8 GB |
| 大规模生产环境 | 专用 CPU 实例，或者把数据库拆分到它自己的节点上 |

查看 Hetzner 的[当前定价](https://www.hetzner.com/cloud)以获得最新的产品目录。Suprnova 的空闲内存占用很小（个位数 MB），所以内存主要花在数据库的工作集，加上您自己的领域代码上。

## 故障排查

### 服务无法启动

检查日志里的错误：

```bash
journalctl -u myapp -n 50
```

常见问题：
- 缺失环境变量
- 数据库连接失败
- 端口已被占用

### Caddy 证书错误

确保：
- 域名 DNS 指向您的服务器
- 80 和 443 端口是开放的
- 没有其他服务占用 80 端口

```bash
caddy validate --config /etc/caddy/Caddyfile
```

### 数据库连接问题

手动测试连接：

```bash
sudo -u app psql $DATABASE_URL -c "SELECT 1"
```

### 健康检查失败

```bash
# 检查应用是否在运行
systemctl status myapp

# 直接测试健康端点
curl http://localhost:8765/_suprnova/health

# 带数据库检查
curl http://localhost:8765/_suprnova/health?db=true
```

一个带着 `"status": "degraded"` 的 `503` 响应，意味着这个应用是在运行的，但数据库健康检查失败了 - 检查响应体里的 `database_error`，并检查 `DATABASE_URL`、Postgres 日志和连接数上限。

## 下一步

- [部署概览](deployment.md) - 与平台无关的单二进制部署方法
- [Docker](cli-docker.md) - `docker:init` 和 `docker:compose` 的细节
- [配置](configuration.md) - 完整的 env 表面和类型化配置
- [部署到 Railway](deployment-railway.md) - 带自动构建的 PaaS 替代方案
- [部署到 Digital Ocean](deployment-digital-ocean.md) - 带托管基础设施的 App Platform

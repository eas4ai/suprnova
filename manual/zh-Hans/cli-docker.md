# Docker

Suprnova 发布两个 CLI 命令，生成您可以原样采用或者修改的 Docker 工件。`docker:init` 会为生产环境写入一个多阶段的 `Dockerfile` + `.dockerignore`。`docker:compose` 会为本地开发服务（数据库、缓存，外加可选的 Mailpit + MinIO）写入一份 `docker-compose.yml`。这两条命令都写在当前项目的根目录里；两者都不会去驱动您的容器运行时。

## docker:init

生成一个生产 Dockerfile，外加一份配套的 `.dockerignore`。

```bash
suprnova docker:init
```

这条命令拒绝覆盖一个已经存在的 `Dockerfile`；如果您想重新生成，请先删除那个已有的文件。

### 写入的内容

| 文件 | 用途 |
|------|---------|
| `Dockerfile` | 三阶段构建：前端资产、Rust 发布二进制文件、运行时镜像 |
| `.dockerignore` | 排除 `target/`、`node_modules/`、`.env*`、已有的构建产物，以及这些 Docker 文件自己 |

### Dockerfile 形态

生成出来的 Dockerfile 用了三个阶段，这样运行时镜像里就只带着编译好的二进制文件，加上它所需要的共享库：

1. **`frontend-builder`** - `node:20-alpine`。安装 npm 依赖，运行 `npm run build`，产出 `frontend/dist`。
2. **`backend-builder`** - `rust:1.91.1-slim-bookworm`。把 `Cargo.toml` + `Cargo.lock` 缓存成一个依赖层，然后复制您的 `cmd/`、`src/`，以及构建好的 `frontend/dist`（作为 `public/assets`），再运行 `cargo build --release`。
3. **`runtime`** - 带 `ca-certificates` 和 `libssl3` 的 `debian:bookworm-slim`。以非 root 的 `appuser` 身份运行。把这个二进制文件复制进来，命名为 `./app`，`public/` 目录就在它旁边。公开端口 8765。

这个最终镜像的默认 `CMD` 是 `["./app"]`，运行的是这个统一二进制文件的 `serve` 子命令（带启动时自动迁移的 web 服务器）。要运行一个不同的子命令，就在 `docker run` 时覆盖这条命令：

```bash
# Web 服务器（默认）
docker run -p 8765:8765 --env-file .env.production my-app

# 只运行迁移，然后退出
docker run --env-file .env.production my-app ./app migrate

# 运行这个调度器守护进程
docker run --env-file .env.production my-app ./app schedule:work

# 运行这个队列工作进程
docker run --env-file .env.production my-app ./app queue:work
```

通过 `--env-file .env.production` 或者一个个的 `-e` 标志，把生产配置传进去。`.env.production` 永远不应该被提交 - 它已经被 `.dockerignore` 覆盖了。

### 升级 Rust 工具链

这个 Dockerfile 把构建阶段固定在 `rust:1.91.1-slim-bookworm`，这样一个刚生成的镜像才是可重现的，并且匹配 Suprnova 0.6 声明的 MSRV。自定义的 Dockerfile 应该用相同或者更新的工具链：

```dockerfile
FROM rust:1.91.1-slim-bookworm AS backend-builder
```

把它固定到和 `rust-toolchain.toml`（如果您有的话）或者您本地 `rustc --version` 所报告的相匹配的那个工具链版本上。

### 为什么 Suprnova 有所不同

Laravel 的部署通常会**每个容器或主机运行多个进程**：php-fpm 跑 web，一个队列工作进程，一个调度器，有时候还有一个 Horizon 仪表盘，有时候还有一个 Octane 运行器。每一个都是它自己的服务定义。

Suprnova 编译成**一个静态链接的二进制文件**，它知道这个框架发布的每一个子命令 - `serve`、`migrate`、`queue:work`、`schedule:work`、`workflow:work`、`ssr:start`。同一个 Docker 镜像能跑每一种角色；唯一变化的东西是那条命令。这让「web + worker + 调度器」在您的编排器里成为三个服务，却全都指向同一个镜像标签 - 一次构建，就能把整个应用向前推进一步。

## docker:compose

生成一份 `docker-compose.yml`，用来拉起本地开发服务。

```bash
suprnova docker:compose [OPTIONS]
```

和 `docker:init` 一样，这条命令拒绝覆盖一个已经存在的 `docker-compose.yml`。它还会把 `docker-compose.override.yml` 追加进您的 `.gitignore`（如果有一个 `.gitignore` 存在的话），这样您就能把逐开发者的覆盖配置留在本地，而不用提交它们。

### 选项

| 选项 | 描述 |
|--------|-------------|
| `--with-mailpit` | 包含 Mailpit 这个邮件测试服务 |
| `--with-minio` | 包含 MinIO（兼容 S3 的对象存储） |

如果两个标志都不传，这条命令会针对两者都交互式地提示。传了任意一个标志，就会跳过提示，直接用您给出的那个标志值。

### 您总会得到什么

PostgreSQL 和 Redis 会被写进每一份生成的 compose 文件：

| 服务 | 默认端口 | 镜像 |
|---------|-------------:|-------|
| PostgreSQL | 5432 | `postgres:16-alpine` |
| Redis | 6379 | `redis:7-alpine` |

两个服务都带健康检查、持久化的具名卷，并且活在一个限定到项目范围的网络上（`<project>_network`）。Postgres 的用户、密码和数据库默认是 `suprnova` / `suprnova_secret` / `suprnova_db`。

### 可选服务

当您选择启用时：

| 服务 | 默认端口 | 镜像 |
|---------|--------------:|-------|
| Mailpit | 1025（SMTP）、8025（UI） | `axllent/mailpit:latest` |
| MinIO | 9000（S3 API）、9001（控制台） | `minio/minio:latest` |

Mailpit 默认接受任何 SMTP 认证，这样您在开发期间就不用配置凭据；`http://localhost:8025` 上的这个 web UI 会显示您应用发出的每一封邮件。MinIO 的默认凭据是 `minioadmin` / `minioadmin`。

### 运行这套堆栈

```bash
# 在后台把所有东西都拉起来
docker compose up -d

# 追踪日志
docker compose logs -f

# 停止并移除这些容器（卷会保留下来）
docker compose down

# 把卷也移除（会清空本地数据库）
docker compose down -v
```

### 把 `.env` 接入 compose

这份 compose 文件到处都用的是 `${VAR:-default}` 语法，所以您可以通过在 `.env` 或者您的 shell 里设置它，来覆盖任何东西。默认这套堆栈的一份典型 `.env`：

```env
DATABASE_URL=postgres://suprnova:suprnova_secret@localhost:5432/suprnova_db
REDIS_URL=redis://localhost:6379

# Mailpit（如果启用了）
MAIL_DRIVER=smtp
MAIL_HOST=localhost
MAIL_PORT=1025

# MinIO（如果启用了）
FILESYSTEM_DISK=s3
S3_ENDPOINT=http://localhost:9000
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_BUCKET=local
S3_REGION=us-east-1
```

要覆盖一个端口（比如说因为 5432 已经被占用了），就在拉起这套堆栈之前，设置匹配的那个环境变量：

```bash
DB_PORT=5433 docker compose up -d
```

完整的一套可覆盖端口：

| 变量 | 服务 | 默认值 |
|----------|---------|--------:|
| `DB_PORT` | PostgreSQL | 5432 |
| `REDIS_PORT` | Redis | 6379 |
| `MAILPIT_SMTP_PORT` | Mailpit SMTP | 1025 |
| `MAILPIT_UI_PORT` | Mailpit UI | 8025 |
| `MINIO_API_PORT` | MinIO S3 | 9000 |
| `MINIO_CONSOLE_PORT` | MinIO 控制台 | 9001 |

### 自定义这份 compose 文件

生成之后，`docker-compose.yml` 就是您的了，可以自由编辑 - Suprnova 之后不会重新生成它，也不会读取它。常见的修改：

- 如果您更喜欢其中一个驱动程序，就把 `postgres:16-alpine` 换成 `mysql:8` 或者 `mariadb:11`；两者在 Suprnova 里都是一等的
- 如果您想在一个一次性容器里运行迁移，就加一条挂载您 `migrations/` 目录的 `volumes:` 条目
- 用同样的方式加更多服务（Qdrant、Elasticsearch、Nats）

## 生产部署

对于一次真正的部署，运行 `docker:init`，把生成出来的这个 `Dockerfile` 当作您的构建输入。大多数编排器（Railway、Fly、Digital Ocean App Platform、Kubernetes）只需要三样东西：

1. 从这个 `Dockerfile` 构建出来的镜像标签
2. 一份带着 `DATABASE_URL`、`APP_KEY`，以及任何驱动程序专属键的环境文件
3. 一个指向 `GET /_suprnova/health/live` 的健康检查（如果这个平台区分这两者，再加一个指向 `/_suprnova/health/ready` 的就绪性检查）

这种单一二进制文件的形态，意味着每一种角色都用同一个镜像；您声明一个跑 `./app` 的「web」服务，和一个跑 `./app schedule:work`（或者 `./app queue:work`）的「scheduler」或「worker」服务。两者读的是同一份环境，所以它们在每一次部署上都保持同步。

平台无关的检查清单请参见[部署概览](deployment.md)，完整实操的示例见这些平台指南：[Railway](deployment-railway.md)、[Digital Ocean](deployment-digital-ocean.md)、[Hetzner VPS](deployment-hetzner.md)。

## 总结

| 命令 | 写入 | 何时使用 |
|---------|--------|-------------|
| `suprnova docker:init` | `Dockerfile`、`.dockerignore` | 构建生产镜像 |
| `suprnova docker:compose` | `docker-compose.yml` | 拉起本地的 Postgres/Redis/Mailpit/MinIO |

## 下一步

- [部署概览](deployment.md) - 平台无关的部署检查清单
- [Railway](deployment-railway.md) - 带从 git 构建的托管 PaaS
- [Digital Ocean](deployment-digital-ocean.md) - App Platform 部署
- [Hetzner VPS](deployment-hetzner.md) - 带 systemd + Caddy 的裸机
- [环境变量](env-vars.md) - 这个框架读取的每一个键

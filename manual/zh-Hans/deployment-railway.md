# 部署到 Railway

[Railway](https://railway.app) 是一个由 Git 驱动的 PaaS，会构建您的 Dockerfile 并把它运行在托管基础设施上。把它和 Railway 托管的 Postgres、Redis 搭配起来，您就有了一整套无需照看服务器的完整 Suprnova 生产环境栈。这份秘诀，会带您从一个刚用 `suprnova new` 脚手架生成的应用，走到一个可访问的线上 URL。

## 前提条件

- 一个 [Railway 账户](https://railway.app)
- 一个已推送到 GitHub、GitLab 或 Bitbucket 的 Suprnova 项目
- 仓库根目录下的一个 `Dockerfile` 和 `.dockerignore`，由以下命令生成：
  ```bash
  suprnova docker:init
  ```
- 一个生成好的 `APP_KEY`，您可以把它粘贴进 Railway 的变量里：
  ```bash
  suprnova key:generate --show
  ```

`suprnova` 只在本地才需要 - Railway 会自己构建这个 Dockerfile。框架 crate 会在构建期间，作为一个普通的 cargo 依赖，从 git 拉取下来。

## 开通项目

1. 打开 [Railway 仪表盘](https://railway.app/dashboard)，点击 **New Project**，选择 **Deploy from GitHub repo**。
2. 选择这个仓库。Railway 会检测到这个 `Dockerfile`，并自动开始第一次构建。
3. 在它构建的同时，添加一个数据库：**New** → **Database** → **Add PostgreSQL**。Railway 会把 `DATABASE_URL` 作为项目上的一个引用变量暴露出来。
4. 如果您的应用使用 Redis 缓存、会话、队列或速率限制驱动程序，可以用同样的方式选配添加 Redis（**New** → **Database** → **Redis**）。Railway 会把这个连接 URL 暴露为 `REDIS_URL`。

## 接好变量

打开这个 web 服务，进入 **Variables**，添加生产环境配置。使用 Railway 的 `${{ }}` 引用语法，从数据库服务里拉取 URL，这样密钥轮换就不需要重新粘贴。

```env
APP_ENV=production
APP_KEY=<paste the output of `suprnova key:generate --show`>
SERVER_HOST=0.0.0.0
SERVER_PORT=8765
DATABASE_URL=${{ Postgres.DATABASE_URL }}
REDIS_URL=${{ Redis.REDIS_URL }}
```

有几件事值得了解：

- **`APP_KEY` 在非开发环境中是强制性的。** 当 `APP_ENV != local|dev|test` 且 `APP_KEY` 缺失或格式错误时，Suprnova 会在启动时关闭失败。服务器会记录一条补救消息，并以非零状态退出 - Railway 会把这次部署标记为失败。用 `suprnova key:generate --show` 生成这个密钥。
- **必须设置 `SERVER_HOST=0.0.0.0`。** Railway 通过容器的网络接口路由流量；绑定到 `127.0.0.1`（本地默认值）看起来会像是一次被拒绝的连接。
- **`SERVER_PORT` 要匹配 Dockerfile 里的 `EXPOSE`。** 生成的 Dockerfile 暴露的是 8765 端口。Railway 会自动把它映射到一个公开 URL。

## 构建与部署

Railway 会在每次推送到已连接分支时构建。由 `docker:init` 生成的 Dockerfile 会做：

1. **阶段 1 - 前端。** 在 `frontend/` 里运行 `npm ci` 和 `npm run build`。Vite 的输出落在 `frontend/dist/`。
2. **阶段 2 - 后端。** 针对您的工作空间运行 `cargo build --release`；缓存的依赖层让迭代构建保持快速。
3. **阶段 3 - 运行时。** 一个 `debian:bookworm-slim` 镜像，带着 `ca-certificates` + `libssl3`、一个非 root 的 `appuser`，以及编译好的 `./app` 二进制文件。默认的 `CMD` 是 `./app`，它会带着自动迁移运行 `serve`。

第一次构建通常需要几分钟（Rust 缓存是冷的）；得益于 Docker 的层缓存，后续的构建要快得多。

## 添加一个调度器服务

如果您的应用使用 `#[derive(Task)]` 调度，调度器就需要它自己的长期存活的进程。从同一个仓库添加第二个服务：

1. **New** → **GitHub Repo** → 选择同一个仓库。
2. 把它命名为 `scheduler`，这样在仪表盘里容易辨认。
3. 在 **Settings** → **Deploy** 下，把 **Custom Start Command** 设置为：
   ```bash
   ./app schedule:work
   ```
4. 复制相同的变量（尤其是 `APP_KEY` 和数据库引用），这样这个工作进程读取的配置就和 web 服务一致。

`schedule:work` 是一个守护进程循环 - 它每分钟醒来一次，查询到期的任务，并通过和 HTTP 服务器相同的启动流程运行它们。相关契约请参见 [控制台](console.md) 和调度器那一章。

务必只运行恰好一个调度器实例。多个 `schedule:work` 进程会通过缓存支撑的锁进行协调，但默认的预期是单个工作进程。

### 为什么 Suprnova 有所不同

一次部署在 Forge 或 Vapor 上的 Laravel，通常会接好一个 web 服务器（php-fpm + nginx）、一个队列工作进程（`php artisan queue:work`），以及一条每分钟调用一次 `schedule:run` 的 cron 记录。三个组件，三个部署表面。

Suprnova 把每一种角色都编译进同一个二进制文件。Railway 的服务规格里，web 角色是 `./app`，调度器是 `./app schedule:work` - 同一个镜像，同一套启动流程，只是 argv 不同。这里没有单独的 php-fpm 容器，没有单独的工作进程镜像，也没有宿主机上的 cron。如果您有排队的作业，就把 `./app queue:work` 加为第三个服务 - 这样您就用同一个 Dockerfile，在三个 Railway 服务里，实现了完整的 Laravel 拓扑结构。

## 健康检查和 `railway.json`

想要对部署有更多控制权，就把一个 `railway.json` 提交到仓库根目录。Railway 会自动识别它。

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

Suprnova 自带内置的健康端点，会在中间件链之前就短路 - 它们返回一个 200 的 JSON 状态，不会经过认证、CSRF 或速率限制。`/_suprnova/` 前缀是保留的，所以它们永远不会和您的路由冲突。

上面的 `healthcheckPath` 指向 `/_suprnova/health/live`，它不触及任何东西。这个搭配是刻意的：这个服务配置的是 `"restartPolicyType": "ON_FAILURE"`，所以这个健康检查探测的是什么，什么就会触发重启。如果把它指向数据库 - 通过 `/_suprnova/health/ready` 或者更老的 `/_suprnova/health?db=true` - 就意味着一次数据库波动，会在数据库最无法承受重新连接风暴的那一刻，重启每一个副本。请从一个单独的就绪性检查或者您的监控系统去探测数据库，而不是从这条会重启进程的路径。参见[为正确的问题使用正确的探针](deployment.md#use-the-right-probe-for-the-right-question)。

这两条旧路径都仍然可用，所以一个已有的 Railway 服务不需要任何改动；具名路径只是更清晰而已。

## 自定义域名和 TLS

1. 在 web 服务里，打开 **Settings** → **Networking**。
2. 点击 **Generate Domain**，获得一个 `*.up.railway.app` 子域名；或者点击 **Custom Domain**，把您自己的主机名指向这个服务。
3. 按 Railway 的指示更新 DNS（子域名用一条 `CNAME`，顶级域名用 ANAME/ALIAS）。

对于生成的域名和自定义域名，Railway 都会开通并续期 Let's Encrypt 证书。

## CI/CD 中的迁移

默认的 `CMD ["./app"]` 会在启动时运行迁移，这对单实例部署来说没问题。对于多副本的部署，请把迁移这一步骤解耦出来：

1. 添加一个一次性的**预部署钩子**，在新副本启动之前，针对生产数据库运行 `./app migrate`。
2. 把运行时的启动命令改成 `./app serve --no-migrate`，这样各个副本之间就不会产生竞态。

这个迁移运行器是幂等的 - 就算您不拆分这些步骤，在每次启动时都运行迁移，在多个副本之间也是安全的。之所以要拆分，是为了让您能在一次糟糕的迁移上尽早让部署失败，而不必让这次发布一直悬而不决。

## 日志、指标、回滚

这个 web 服务标签页会展示：

- **Deployments** - 按时间顺序排列的每一次构建；之前某次成功部署上的三点菜单，就是一键回滚的路径
- **Logs** - 来自容器的 `tracing` 输出，带着结构化日志字段（`request_id`、`route`、`status`），可以直接用在日志查看器的筛选器上
- **Metrics** - CPU、内存、网络 IO；在决定该把这个实例的规格调大还是调小时很有用

## 故障排查

**构建在 `cargo build --release` 上失败。** 用 `docker build -t myapp .` 在本地复现。最常见的原因是有一个工作空间成员，在您自己的机器上能编译，却没有被提交进仓库 - 这个 Dockerfile 会先复制 `Cargo.toml` 和 `Cargo.lock`，所以缺失的 crate 会明确地失败。

**应用返回“connection refused”。** 检查这个服务上是否设置了 `SERVER_HOST=0.0.0.0`。默认值是 `127.0.0.1`，Railway 没法把流量路由到这个地址。

**应用启动后又带着一个密钥错误退出。** `APP_KEY` 未设置或者格式错误。没有它，这个框架会拒绝在生产环境中启动；请把 `suprnova key:generate --show` 的输出重新粘贴进这个服务的变量里。

**迁移在启动时失败。** 检查日志里的底层 SQL 错误。常见的原因是 `DATABASE_URL` 未设置（确认 `${{ Postgres.DATABASE_URL }}` 这个引用被正确解析了），或者是一次针对过期基线运行的迁移（`./app migrate:status` 会报告哪里应用了什么）。

**调度器一直不触发。** 确认启动命令确切是 `./app schedule:work`（而不是 `schedule:run` - 后者只会把到期的任务运行一次然后退出）。从一次一次性部署里运行 `schedule:list`，可以确认您的任务都已注册。

## 下一步

- [部署概览](deployment.md) - 您的 Railway 服务运行的这个统一二进制模型
- [Docker CLI](cli-docker.md) - `docker:init` 和 `docker:compose` 实际生成了什么
- [配置](configuration.md) - `.env` 加载、类型化配置、必需的键
- [控制台](console.md) - `schedule:work`、`queue:work`、`workflow:work`，以及这套统一 CLI 的其余部分
- [部署到 Digital Ocean](deployment-digital-ocean.md) - 同一份秘诀，用在另一个 PaaS 上

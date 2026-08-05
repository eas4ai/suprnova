# 部署到 Digital Ocean

Digital Ocean 有两种适合 Suprnova 应用的生产环境目标：**App Platform**（一个托管的 Docker PaaS - 推送即可，不必操心）和一个 **Droplet**（您自己的 VPS，一切都由您自己管理）。本章会把两者都走一遍。当您想要托管数据库、自动部署，以及帮您处理好的 SSL 时，就用 App Platform。当您想要完全的掌控权、已经在这台机器上跑着其他服务，或者想让账单不随流量变动时，就用 Droplet。

## 前提条件

- 一个 [Digital Ocean 账户](https://www.digitalocean.com)
- 一个带 Dockerfile 的 Suprnova 项目 - 用以下命令生成一个：
  ```bash
  suprnova docker:init
  ```
- 一个用于生产环境的 `APP_KEY`。生成一个，并把它保管在安全的地方：
  ```bash
  suprnova key:generate --show
  ```
  当 `APP_ENV` 是 `local` / `development` / `testing` 之外的任何值，且 `APP_KEY` 未设置时，Suprnova 会在启动时关闭失败。
- 一个 git 仓库（GitHub 或 GitLab） - App Platform 需要它；对于 Droplet，您也可以把一个预先构建好的镜像推送到一个镜像仓库。

## App Platform

App Platform 会构建您的 Dockerfile，运行这一个 Suprnova 二进制文件，并且如果您想要的话，还会给您一个托管的 Postgres。

### 1. 创建应用

1. 前往 [Digital Ocean Apps](https://cloud.digitalocean.com/apps)。
2. 点击 **Create App**，连接 GitHub/GitLab，选择仓库和分支。
3. App Platform 会自动检测仓库根目录下的 `Dockerfile`。

### 2. 配置 web 服务

| 设置 | 值 |
|---|---|
| 资源类型 | Web Service |
| HTTP 端口 | `8765` |
| 运行命令 | 留空 - Dockerfile 的 `CMD` 会运行 `./app` |
| 健康检查（HTTP 路径） | `/_suprnova/health/live` |

默认的 Suprnova 二进制文件会带着自动迁移运行 `serve`，所以这个容器会在启动时运行迁移，然后再绑定监听器。

### 3. 添加一个托管 Postgres

1. **Add Resource** -> **Database** -> **PostgreSQL**。
2. 选一个方案（测试用 Dev Database；真实流量用 Production 方案）。

App Platform 会通过 `${db.DATABASE_URL}` 这个绑定，自动把 `DATABASE_URL` 注入每一个组件。

### 4. 环境变量

在您 web 组件的 **Environment Variables** 部分，设置：

| 变量 | 值 | 备注 |
|---|---|---|
| `APP_ENV` | `production` | 触发关闭失败的 `APP_KEY` 检查 |
| `APP_KEY` | `suprnova key:generate --show` 的输出 | 标记为**已加密** |
| `SERVER_HOST` | `0.0.0.0` | 绑定到所有接口 |
| `SERVER_PORT` | `8765` | 匹配 Dockerfile 的 `EXPOSE` |
| `APP_URL` | `https://your-app.ondigitalocean.app` | 供 Inertia 和已签名 URL 使用 |

`DATABASE_URL` 由这个托管数据库绑定自动提供；不要手动设置它。

如果您用 Redis 做缓存/会话，就添加一个托管的 Redis 集群，并把 `REDIS_URL` 设置为它的绑定值（`${redis.REDIS_URL}`）。

### 5. 部署

点击 **Create Resources**。第一次构建需要几分钟（Rust release 构建 + 前端构建）；后续的构建会用上 Dockerfile 的层缓存，快得多。

### 添加一个调度器工作进程

计划任务（通过 `Schedule::call` 注册的 `#[derive(Task)]` 处理程序）需要它自己的长期存活的进程。添加一个 Worker 组件，用不同的命令运行同一个镜像：

1. **Create** -> **Add Resource** -> **Detect from source code**，选择同一个仓库。
2. 把资源类型设置为 **Worker**。
3. **Run command**：
   ```bash
   ./app schedule:work
   ```
4. 这个 Worker 会继承来自这个应用的环境变量，包括 `DATABASE_URL` 和 `APP_KEY`。

Worker 不会收到 HTTP 流量。请务必只运行**一个** Worker 实例 - 多个调度器会导致每个任务被运行多次。

对于队列工作进程（`./app queue:work`），这个模式是一样的；您通常可以安全地运行不止一个队列工作进程，因为队列驱动程序会协调好哪个工作进程拿到哪个作业。参见 [队列](queues.md)。

### 应用规格（基础设施即代码）

想要可重复的部署，就提交一个 `.do/app.yaml`：

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
      # 仅生存性 - 这项检查失败时 App Platform 会重启容器，
      # 所以它不能依赖 Postgres。相关的健康检查说明，请参见
      # 下面的故障排查一节。
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

用 `doctl` 这个 CLI 部署：

```bash
doctl apps create --spec .do/app.yaml
```

通过 Apps 界面单独设置这个 `APP_KEY` 密钥，或者：

```bash
doctl apps update <app-id> --spec .do/app.yaml \
  --set-env "APP_KEY=$(suprnova key:generate --show)"
```

### 自定义域名

在 **Settings** -> **Domains** -> **Add Domain** 里，输入您的域名，并按照 DNS 说明操作。App Platform 会自动签发并续期一个 Let's Encrypt 证书。

域名生效之后，更新 `APP_URL` 使其保持一致 - Inertia 会用它作为 X-Inertia-Location 请求头，已签名的 URL 也会用它作为哈希输入。

### 扩展

- **水平扩展**：调高 web 服务上的 **Instance Count**。每个实例共享这个托管的 Postgres；多个实例在启动时运行自动迁移是安全的 - Suprnova 用的是 SeaORM 那个带咨询锁的迁移运行器。
- **垂直扩展**：更改 **Instance Size**。对于低流量的应用，这个 Rust 二进制文件在最小的规格上也能跑得很好；当您开始大规模服务 WebSocket 或长期存活的连接时，再往上调。

把这个调度器 Worker 的实例数量保持在 **1**。

## Droplet (VPS)

如果您想在自己的 VPS 上运行 Suprnova，Droplet 就是这条路径。它的机制和任何其他 Linux VPS 完全一样 - systemd 服务、Caddy 反向代理、托管的或自托管的 Postgres。[Hetzner VPS](deployment-hetzner.md) 那一章，就是这套模式的规范演练；那里的一切内容，都可以逐字套用在 Droplet 上。值得指出的仅有的几点差异是：

- **镜像**：在 Droplet 控制台里选择 **Ubuntu 24.04** 或 **Debian 12**。
- **数据库**：您可以用 Digital Ocean 的 **Managed Databases** 来提供 Postgres / MySQL / Redis，而不是在 Droplet 上自己运行它们 - `DATABASE_URL` / `REDIS_URL` 的用法完全一样，把它们指向那个托管端点，Suprnova 不会察觉到任何区别。
- **备份**：在 DO 控制台里开启 Droplet 快照和托管数据库的每日备份。
- **网络**：用一个 DO **VPC**，把这个 Droplet 和任何托管数据库放在同一个私有网络里；把监听器绑定到 `127.0.0.1`，并在前面放一个 Caddy 来处理 TLS。

如果您想在 Droplet 上用 Docker（而不是一个系统级二进制文件），来自 [Docker](cli-docker.md) 的那个 docker-compose 模式可以干净地套用进来 - 把自托管的 Postgres 换成这个托管数据库，就大功告成了。

### 为什么 Suprnova 有所不同

典型的 Laravel PHP 部署，需要 PHP-FPM + 一个 opcache + 一个队列运行器 + 一条调度器 cron 记录 - 至少三个活动部件，每一个都有自己的重启语义。而一次 Suprnova 部署，是一个二进制文件，外加一个可选的工作进程。这个二进制文件运行迁移、服务 HTTP、处理 WebSocket，并且位于一个反向代理背后。同一个二进制文件，用 `./app schedule:work` 或 `./app queue:work` 调用，就是您的调度器或队列工作进程。App Platform 那套“一个镜像、多个组件”的模型，天然适合这种形态 - 每个组件用同一个 Dockerfile，只是每种角色的 `run_command` 不同。

## 故障排查

### 构建失败

首先要检查的，是这个 Dockerfile 能不能在本地构建：

```bash
docker build -t myapp .
```

当本地构建成功，但 App Platform 的构建失败时，常见的原因是：

- **缺失构建上下文文件**：检查 `.dockerignore` 有没有把 `Cargo.lock` 或者 `migrations/` 目录排除在外。
- **`cargo build` 期间内存不足**：在 App Settings -> Resources -> Build 里调高构建实例的规格。Rust 的 release 构建很吃内存。

### 应用启动后随即崩溃

在 **Runtime Logs** 标签页里检查运行时日志。两个最常见的 Suprnova 启动失败原因是：

- **`APP_KEY is required when APP_ENV=production`** - 用 `suprnova key:generate --show` 生成一个，并把它作为一个已加密的环境变量添加进去。
- **`SERVER_HOST=…` 的值无效** - 对 App Platform 来说必须是 `0.0.0.0`，不能是 `127.0.0.1`（负载均衡器没法到达这个环回地址）。

### 健康检查失败

这个平台会 ping `/_suprnova/health/live`，并预期在配置的超时时间内收到一个 200。如果它失败了：

- 确认这条路径精确地是 `/_suprnova/health/live`（不是 `/health`）。如果您的规格文件里已经写的是更老的 `/_suprnova/health`，它仍然可以工作。
- 确认端口是 `8765`，并且和 `SERVER_PORT` 匹配。
- 想要分清“绑定不了”和“连不上 Postgres”这两种情况，就**手动**从控制台探测数据库，而不是通过健康检查：

  ```bash
  curl http://localhost:8765/_suprnova/health/ready
  # 健康：200 {"status":"ok","database":"connected"}
  # 降级：503 {"status":"degraded","database":"error"}
  ```

  一个降级的响应意味着这个应用绑定成功了，但连不上 Postgres - 检查这个 `DATABASE_URL` 绑定。不要传 `-f`：它会让 curl 在 503 上静默退出，而这正是您想要看到的那种情况。

不要把这个数据库探测放进这个应用规格的 `health_check` 里。当这项检查失败时，App Platform 会重启容器，所以一次数据库波动会把应用一起拖垮 - 这种失败模式，恰恰是在您最需要这个应用挺过去的那次事故期间，进入一个重启循环。参见[为正确的问题使用正确的探针](deployment.md#use-the-right-probe-for-the-right-question)。

### 数据库迁移没有运行

迁移是默认 `./app` 启动流程的一部分，会自动运行。如果没有运行，就检查运行时日志里的 SeaORM 错误。想要从 App Platform 控制台手动运行它们：

1. 在 web 组件上打开 **Console** 标签页。
2. 运行 `./app migrate`。

如果您更愿意把迁移排除在启动路径之外，就把运行命令设置为 `./app serve --no-migrate`，并在应用规格里添加一个一次性的 **Job**，让它在部署前运行 `./app migrate`。

## 下一步

- [部署概览](deployment.md) - 跨平台部署入门（二进制文件、迁移、调度器、健康检查）
- [Docker](cli-docker.md) - `suprnova docker:init` 和 `docker:compose` 会生成什么
- [配置](configuration.md) - Suprnova 读取的每一个环境变量
- [环境变量](env-vars.md) - 完整参考，包括生产环境必需的那些
- [部署到 Hetzner VPS](deployment-hetzner.md) - Droplet 的演练在这里逐字适用

# 部署概览

Suprnova 应用编译为一个自包含的二进制文件，该二进制文件拥有 Web 服务器、迁移运行器、任务调度器和队列工作进程。部署就是“复制二进制文件、设置四个环境变量、运行它”。本章涵盖这四个变量是什么、二进制文件的子命令在生产环境中的作用，以及内置健康端点如何与平台的生存性探针集成。平台特定的演练见 [Railway](deployment-railway.md)、[Digital Ocean](deployment-digital-ocean.md) 和 [Hetzner](deployment-hetzner.md)。

## 单一二进制文件

您的应用编译为一个具有 clap 子命令接口的二进制文件：

```bash
./app                       # serve（默认）- 自动迁移，然后 HTTP
./app serve                 # 显式的 serve，带自动迁移
./app serve --no-migrate    # 运行 serve，但不运行迁移
./app web:run               # serve 的别名

./app migrate               # 应用待处理的迁移，然后退出
./app migrate:status        # 显示迁移状态
./app migrate:rollback [N]  # 回滚最近 N 个迁移（默认 1）
./app migrate:fresh         # 删除所有表，然后重新迁移 - 在生产环境中
                            # 这需要 --force，还需要在交互式终端上
                            # 键入确认；参见 cli-migrations.md

./app schedule:work         # 调度器守护进程 - 每分钟醒来一次
./app schedule:run          # 把到期的任务运行一次，然后退出
./app schedule:list         # 打印每一个已注册的任务
./app queue:work            # 队列工作进程，以守护进程运行
./app workflow:work         # 工作流工作进程，以守护进程运行

./app down [--secret …] [--retry …] [--except …] [--message …]
./app up                    # 退出维护模式
```

一个二进制文件意味着一个 Docker 镜像、一个 CI 工件、一次部署可验证。同一个镜像运行 Web 服务、任务调度器、队列工作进程和工作流工作进程 - 您为每个启动不同的子命令。

## 四个生产环境变量

如果生产环境配置不正确，Suprnova 在启动时会关闭失败。部署的最小集合：

| 变量 | 功能 | 失败模式 |
|---|---|---|
| `APP_ENV` | 选择环境（`production`、`staging` 等）。| 如果未设置，默认为 `local` - 应用在生产环境中以开发模式运行。|
| `APP_KEY` | 用于 `Crypt`、会话、Cookie 和分页游标的 32 字节 AES-256 base64 密钥。| 当 `APP_ENV` 不是 local/dev/test 且 `APP_KEY` 缺失或格式错误时，启动会返回类型错误并以非零代码退出。|
| `APP_URL` | 应用的规范绝对 URL(`https://app.example.com`)。| 默认为 `http://localhost:8765`；已签名 URL、重定向、邮件链接和绝对 Inertia URL 都使用此值。|
| `DATABASE_URL` | 关系数据库的连接 URL。| 当 `APP_ENV` 为 `production` 或 `staging` 且 `DATABASE_URL` 未设置时，启动拒绝启动 - 开发 SQLite 回退显式被拒绝。|

使用 CLI 生成一次 `APP_KEY`:

```bash
suprnova key:generate           # 将 APP_KEY=… 写入 ./.env
suprnova key:generate --show    # 打印密钥供 $(…) 使用
```

关于密钥轮换，参见 [加密](encryption.md) -
`APP_KEY_PREVIOUS`（或兼容 Laravel 的 `APP_PREVIOUS_KEYS`）接受一个以逗号分隔的旧密钥列表，用于仅限解密的回退。

除了这四个必需的变量，常见的生产环境旋钮：

| 变量 | 默认值 | 注释 |
|---|---|---|
| `SERVER_HOST` | `127.0.0.1` | 在容器中使用 `0.0.0.0`。|
| `SERVER_PORT` | `8765` | 匹配您平台的预期端口。|
| `APP_DEBUG` | env 衍生 | 生产/暂存/自定义环境中为 `false`。如果您想在暂存环境中显示详细错误，请显式设置。|
| `SERVER_MAX_BODY_SIZE` | per-handler 默认值 | 进程范围的请求体上限。|
| `SERVER_MAX_CONNECTIONS` | 未设置（无限制） | 并发活跃 TCP 连接上限。参见下文。|
| `SERVER_HEALTH_READINESS_TOKEN` | 未设置（就绪性为公开） | 到达就绪性探针所需的共享密钥。参见 [Health check](#健康检查)。|
| `DB_MAX_CONNECTIONS` | `10` | 连接池大小。|
| `REDIS_URL` | 未设置 | 如果您已配置 Redis 缓存/队列/会话驱动程序，则必需。|

完整表格见 [环境变量](env-vars.md)。

## 推荐数据库: MariaDB

Suprnova 支持 SQLite、PostgreSQL、MySQL 和 MariaDB 作为一级关系型后端。推荐因环境而异：

- **开发。** SQLite。脚手架编写 `DATABASE_URL=sqlite://./database.db`，以便 `suprnova serve` 无需数据库设置即可工作。
- **生产。** MariaDB。它将三个独立服务（关系型 + 向量 + KV 缓存）合并到一个引擎上，如果需要，可以使用系统版本控制的表进行审计。

```bash
# .env.production
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production
```

使用 `mysql://` 方案 - SeaORM 的 MySQL 驱动程序原生处理 MariaDB,Suprnova 的 `MariaDbVectorDriver`(`VECTOR(N)` + HNSW)直接用于向量工作负载。

其他关系型后端也是一级的：

```bash
# PostgreSQL
DATABASE_URL=postgres://app_user:secret@db.internal:5432/app_production

# MySQL
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production

# SQLite（用于极小的单实例部署）
DATABASE_URL=sqlite:///var/lib/myapp/data.db
```

### 为什么 Suprnova 有所不同

Laravel 的默认设置将新项目推向 PostgreSQL，因为 PHP + PostgreSQL 是久经考验的路径。Suprnova 选择能为 Rust 应用提供最清晰单引擎生产配置的数据库。MariaDB 的 `VECTOR(N)`(11.7+)、Dynamic Columns 和系统版本控制的表意味着中小型产品可以在不添加 Redis、OpenSearch 或 pgvector 的情况下交付搜索、KV 和审计功能。PostgreSQL 仍然得到完全支持 - 该框架的测试矩阵对所有三个关系型后端运行 - 但我们的部署文档优先选择使用移动部分最少的引擎。参见 [向量存储](vector.md) 和 [数据库](database.md) 了解后端特定的接口。

## 构建生产镜像

脚手架附带一个多阶段 Dockerfile 的生成器：

```bash
suprnova docker:init
```

这会编写一个具有三个阶段的 `Dockerfile`:

1. **前端构建** - `node:20-alpine`，对您的 `frontend/` Inertia 应用（根据脚手架选择为 Svelte 5、React 19 或 Vue 3.5）运行 `npm ci && npm run build`。
2. **后端构建** - `rust:1.94.0-slim-bookworm`，以发布模式编译您的 crate，具有依赖项缓存。
3. **运行时** - `debian:bookworm-slim`，复制编译的二进制文件和 Vite 输出，以非 root `appuser` 身份运行，公开端口 8765，并运行 `CMD ["./app"]`（自动迁移的服务器）。

当前 `main` 使用 SeaORM 2.0、SeaQuery 1.0 和 SQLx 0.9。直接调用 SeaORM 的应用程序必须导入 `ExprTrait` 以使用 SeaQuery 表达式方法，并对预构建的 `Statement` 值使用显式 `*_raw` 连接方法。此次依赖项升级不需要迁移应用程序数据。

在推送前在本地构建和运行以验证：

```bash
docker build -t myapp .

# 使用 env 文件
docker run --rm -p 8765:8765 --env-file .env.production myapp

# 或使用显式变量（四个必需的）
docker run --rm -p 8765:8765 \
  -e APP_ENV=production \
  -e APP_KEY=$APP_KEY \
  -e APP_URL=https://app.example.com \
  -e DATABASE_URL=mysql://user:pass@host:3306/app \
  myapp
```

绝不要将 `.env.production`（或任何包含 `APP_KEY` 或 `DATABASE_URL` 的文件）提交到您的存储库。使用您平台的密钥存储，并在部署时读取这些值。

## 启动时迁移

默认 `./app`（和显式 `./app serve`）命令在绑定套接字前应用任何待处理的迁移。两个实际的影响：

- **与多个实例安全。** SeaORM 的迁移运行器采用数据库级别的咨询锁；最慢的 pod 等待，其他的在完成后继续。对于常规发布，您无需单独的“迁移-然后-部署”步骤。
- **失败的迁移 = 失败的部署。** 如果迁移出错，进程在服务器绑定前以非零代码退出。平台的健康探针（见下文）报告 pod 不健康，部署停止。通过在下一个发布中交付纠正迁移来向前修复。

对于想在任何 pod 接受流量前通过成功迁移为部署设置门槛的 CI 管道，运行一次性迁移：

```bash
docker run --rm myapp ./app migrate
# … 然后部署实际部署
docker run myapp ./app serve --no-migrate
```

`--no-migrate` 跳过自动迁移阶段，但仍然正常启动服务器。

## 将工作进程作为单独的服务

调度器、队列和工作流系统各有自己的守护子命令。在生产环境中，对同一镜像运行它们作为单独的进程，共享相同的环境：

```bash
docker run myapp ./app schedule:work    # 一个实例 - 参见下文
docker run myapp ./app queue:work       # 扩展到 N 个实例
docker run myapp ./app workflow:work    # 扩展到 N 个实例
```

两个要内化的规则：

- **运行恰好一个 `schedule:work` 进程，或将您的任务标记为 `.on_one_server()`。** 调度器副本默认不协调：每个独立评估调度，所以三个副本每次都运行三次到期任务。`replicas: 1` 是简单的答案；`.on_one_server()` 针对共享缓存每次滴答选举一个副本，如果调度器必须高度可用，这就是您想要的。参见 [调度](scheduling.md#running-on-one-server)。
- **队列和工作流工作进程水平扩展。** 两者都从共享存储中拉取工作，并使用可见性超时或行级锁进行协调；添加 pod 增加吞吐量。`./app queue:work --max-jobs N` 使工作进程在 N 个任务后退出，以便监督程序可以轮换进程 - 对于发布时重启的部署很有用。

参见 [队列](queues.md)、[调度](scheduling.md) 和 [工作流](workflows.md) 了解各子系统的详细信息。

## 优雅停止

每个长期运行的 Suprnova 进程 - 服务器和所有三个守护进程 - 都在 **SIGTERM** 和 SIGINT 上进行排空。SIGTERM 是 `docker stop`、Coolify、systemd 和 Kubernetes 发送的；SIGINT 是 Ctrl-C 发送的。两者采用相同的路径：停止接受新工作，在有限的宽限期内完成飞行中的工作，以 `0` 退出。

宽限期是按子系统的，有目的地受限 - 一个缓慢的客户端或一个长期任务不能无限期地保持进程活动：

| 进程 | 等待 | 宽限期 |
|---|---|---|
| `serve` | 飞行中的 HTTP 连接 | 5 秒 |
| `queue:work` | 飞行中的任务完成 | 直到任务返回 |
| `schedule:work` | `.run_in_background()` 任务 | 30 秒 |
| `workflow:work` | 飞行中的工作流步骤 | 直到它们返回 |

**将您平台的终止宽限期设置在这些之上。** Docker 默认为 10 秒，Kubernetes 为 30 秒。如果平台的窗口比工作耗时更短，它会发送 SIGKILL，您又回到丢失飞行中的任务：

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

**在飞行中被杀死的任务不会丢失，但会消耗一次尝试。** 其预留过期，另一个工作进程重新声明它，消耗一次尝试，所以可靠地杀死其工作进程的任务仍然可以被死信处理，而不是无限循环。参见 [队列](queues.md#what-counts-as-an-attempt)。

**PID 1 是一个真实的约束。** 容器入口点以 PID 1 身份运行，内核不对 PID 1 应用默认信号处置 - 没有 SIGTERM 处理程序的进程在 SIGTERM 上不会死，它会忽略它直到平台放弃并发送 SIGKILL。Suprnova 安装了处理程序，所以 `CMD ["app", "queue:work"]` 按写入状态很好，不需要 `tini` shim。

## 健康检查

Suprnova 公开三个内置健康路径。`_suprnova/` 前缀被保留，以便您的路由永远不会与它们冲突。

| 路径 | 触及 | 用途 |
|---|---|---|
| `/_suprnova/health/live` | 无 | 生存性。只要进程可以处理请求，就回答 200。|
| `/_suprnova/health/ready` | 数据库 | 就绪性。依赖项无法访问时为 503。|
| `/_suprnova/health` | 无或带 `?db=true` 的数据库 | 原始端点。行为如上述之一。|

```bash
curl http://localhost:8765/_suprnova/health/live
# 200 {"status":"ok","timestamp":"2026-05-30T12:34:56+00:00"}

curl http://localhost:8765/_suprnova/health/ready
# 健康：200 {"status":"ok","timestamp":"…","database":"connected"}
# 降级：503 {"status":"degraded","timestamp":"…","database":"error"}
```

`/_suprnova/health` 和 `/_suprnova/health?db=true` 保持完全按之前的工作方式，您已经部署的任何内容都不需要更改 - [Hetzner 指南](deployment-hetzner.md)仍然为单次检查命名它们，您自己的规范也可能。命名路径更清晰，因此在新配置中首选它们；[Railway](deployment-railway.md)、[DigitalOcean](deployment-digital-ocean.md) 和 [Docker](cli-docker.md) 指南使用它们。

### 为正确的问题使用正确的探针

将生存性指向 `/live`，就绪性指向 `/ready`。区别比看起来更重要：失败的**生存性**探针重启 pod，而失败的**就绪性**探针只是将其从负载均衡器中移出。将数据库检查接入生存性，数据库波动会重启您的每个副本 - 正在数据库最无法承受重新连接风暴的那一刻。

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

端点在中间件链前短路，所以即使中间件死锁或请求 ID 中间件拒绝流量，它仍保持响应。

### 降级响应不包含驱动程序详情

503 响应体报告 `"database":"error"` 等等。驱动程序自己的消息 - 它命名主机、端口、数据库和架构名称、服务器版本，以及一些配置错误的连接 URL - 转到 `error!` 级别的日志，其中操作员可以读取它，陌生人则不能。在调试构建中，它也作为 `database_error` 包含在响应体中，所以本地调试不受影响。

### 关闭就绪性

就绪性对任何请求者运行数据库往返。如果端点是互联网可达的，设置一个共享密钥：

```bash
SERVER_HEALTH_READINESS_TOKEN=<a long random string>
```

探针然后必须将其作为标头发送：

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

没有标头，就绪性回答 **404** - 与任何不存在的路径相同的响应，所以端点不可见而不是仅仅关闭。生存性无论如何保持公开，所以您不必在每个清单中放置密钥以保持重启时出现问题信号。

未设置是默认值，就绪性是公开的。这是有意的：本手册和脚手架生成的所有配置都调用 `?db=true` 而不带标头，默认关闭会破坏它们。

## 维护模式

要推进一次破坏性迁移，或者在一次事故中把流量静默下来：

```bash
./app down --secret abc123 \
           --retry 60 \
           --message "Deploying - back in a few minutes" \
           --except /webhooks/stripe

./app up
```

`down` 会写下一个维护标记，中间件在每一个请求上都会读它。请求会拿到一个 503（可以通过 `--status` 配置），带上您给的那条消息 - `--except` 里的路径，以及任何带着这个密钥的请求除外。`up` 会移除这个标记。

这个密钥是一个 bearer 凭据：任何访问 `/<secret>` 的人，都会拿到一个 12 小时有效的绕过 cookie。URL 匹配和 cookie 匹配都是常数时间比较，所以响应耗时不会告诉一个探测者，他们已经猜对了多长的前缀。请优先用 `--with-secret` - 它会替您铸造一把（16 个随机字节，32 个十六进制字符）并打印出绕过 URL - 而不是给 `--secret` 挑一个好记的字符串；并且请像对待事故记录里任何其他凭据一样对待它。

## 扩展

### Web

水平扩展是默认方案：每个 pod 运行 `./app`，共享 `DATABASE_URL`，并连接到相同的 Redis（如果您已配置 Redis 支持的缓存/队列/会话）。自动迁移是安全的，因为上述咨询锁。粘性会话不是必需的 - 会话状态生存在您的会话驱动程序（数据库或 Redis）中，而不是进程内存。

### 工作进程

- **调度器。** 总是恰好一个实例。
- **队列。** 水平扩展。如果您已将工作分散到多个命名队列中，每个队列运行一个工作进程（或传递驱动程序特定的队列过滤器 - 参见 [队列](queues.md)）。
- **工作流。** 水平扩展；行级声明/心跳协调工作进程。

## 连接上限(`SERVER_MAX_CONNECTIONS`)

默认情况下，服务器接受无限数量的并发 TCP 连接。在大多数部署中，反向代理(nginx、Caddy、Traefik)或平台的负载均衡器提供第一道防线。如果您想在进程本身内设置硬缓冲 - 防止单个行为不当的客户端池耗尽文件描述符 - 设置 `SERVER_MAX_CONNECTIONS`:

```bash
# .env.production - 将并发连接上限设置为 1024
SERVER_MAX_CONNECTIONS=1024
```

当达到上限时，**接受循环阻塞**（TCP 级别的背压）直到现有连接关闭；待处理的握手保留在内核的接受积压中。许可证在每个连接的整个生存期间保有，连接结束时立即释放，所以槽位迅速轮换。

经验法则：

- **未设置（默认 = 无限制）。** 如果您有反向代理应用自己的连接限制，或者您在管理并发的 PaaS 后面运行，则正确。
- **设置为具体值**如果进程直接在互联网上运行或您想要深度防御，无论代理配置如何。典型的起点是 2 倍您预期的峰值并发用户，对于长期连接(WebSocket、SSE)向上调整。
- **与 `LimitNOFILE`**(systemd)或 `ulimit -n` 配对，以便操作系统文件描述符限制不会成为惊喜上限。每个 HTTP 连接消耗一个文件描述符；添加您的数据库池大小和几十个操作系统管理。
- **这是一个缓冲，不是上游速率限制的替代。** `SERVER_MAX_CONNECTIONS` 停止失控累积；您的反向代理或 `rate_limit` 中间件应处理每客户端或每 IP 节流。

空白、不可解析或零值被无声地视为未设置，以便打字错误不会阻止服务器启动。

## 每个平台的演练

上面的秘诀适用于每个现代 PaaS 或 VPS。接下来的三章将引导您完成具体内容：

| 平台 | 风格 | 演练 |
|---|---|---|
| Railway | 带从 git 自动部署的 PaaS | [部署到 Railway](deployment-railway.md) |
| Digital Ocean | App Platform(PaaS)或 Droplets(VPS) | [部署到 Digital Ocean](deployment-digital-ocean.md) |
| Hetzner | 带 systemd + Caddy 的 VPS | [部署到 Hetzner](deployment-hetzner.md) |

## 下一步

- [环境变量](env-vars.md) - 框架读取的每个 env 变量
- [加密](encryption.md) - `APP_KEY`、密钥轮换、加密的内容
- [配置](configuration.md) - 在 env 之上构建的类型化配置部分
- [数据库](database.md) - 驱动程序选择、连接池调优、多连接拆分
- [队列](queues.md) - 工作进程扩展和队列驱动程序

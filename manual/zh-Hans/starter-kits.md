# 起步套件

起步套件是即用型 Suprnova 应用程序，您可以复刻并上线。每个套件都连接了控制器、路由、迁移、前端页面和测试，以完成产品表面的全部内容 - 因此您可以从一个运行中的应用开始，而不是一个空的脚手架。

如今推出三个套件，以 Laravel 的血统为基础。挑选最接近您要构建内容的那个，然后从那里进行自定义。

## Nebula - 认证（Breeze 等级）

**仓库：[github.com/eas4ai/Nebula](https://github.com/eas4ai/Nebula)**

最小的完整认证套件 - Suprnova 的 Breeze 等价物。只包含账户所需的一切，不包含多余内容：

- 邮箱验证注册
- 带记住我功能的登录
- 具有反枚举响应的密码重置
- 个人资料管理 - 更新邮箱和密码、删除账户
- 品牌化的 Inertia 3 + Svelte 5 前端（默认深色），已连接登录用户菜单

Nebula 提供两个测试套件：门面级别的认证逻辑，以及线路级别的 HTTP 套件，驱动真实路由、会话、CSRF 往返和 guest/auth/verified 门，通过环回套接字进行。

当您需要一个干净的账户管理基础来构建您自己的产品时，请选择 Nebula。

## Pulsar - 产品站点和社区

**仓库：[github.com/eas4ai/Pulsar](https://github.com/eas4ai/Pulsar)**

基于 Vue 3.5 + Vuetify 的完整开发者工具/SaaS 公司网站。包含 Nebula 认证方案的全部内容，加上真实产品网站需要的表面：

- 营销落地页和用户仪表板
- Markdown 文档管道（`docs:build`），包含搜索和生成的目录
- 博客/文章系统和 RSS 源
- 公开的成员个人资料
- 分类法 - 主题、标签和类别
- 基于角色的访问控制：角色、权限和门
- 内容和成员的管理和审核表面

Pulsar 是下游产品（如 `suprnova.app`）的源套件。当您要发布一个具有文档、博客和成员社区的产品站点时（不仅仅是认证），请选择它。

## Directory starter - 条目、审核与付费发布

**仓库：[github.com/eas4ai/suprnova-directory-starter](https://github.com/eas4ai/suprnova-directory-starter)**

一个基于 Vue 3.5、采用 MIT 许可的免费目录站：所有者提交条目，审核者审阅，访客搜索，发布可以免费，也可以通过 Stripe 和 Paddle 付费。包含 Nebula 认证方案的全部内容，加上：

- 目录发现 - 可搜索的条目、分类筛选、分页和公开的详情页
- 所有者提交与审核 - 草稿、修订历史、批准、驳回、重新提交和暂停
- 免费与付费发布 - Stripe 和 Paddle、可配置的方案、分开的测试与正式设置、经过认证的 webhook 和支付恢复
- 编辑发布 - 文章草稿、预览、分类、标签和 RSS
- 管理 - 基于权限的角色、账户暂停、审计历史和管理员概览
- SEO - 站点默认值和逐内容覆盖、服务端渲染的元数据、站点地图、重定向、404 报告，以及公开页面的 Markdown 版本
- 媒体与通知、可记住浅色或深色偏好的配色预设、演示内容，以及文档化的备份与恢复流程

当产品是一个条目目录或条目集市（不论有没有付费展示位）时，请选择 directory starter。

## 选择哪个套件？

| 您需要… | 开始选择 |
|---|---|
| 账户和构建的地方 | **Nebula** |
| 完整的产品网站 - 落地页、文档、博客、社区、RBAC | **Pulsar** |
| 一个条目目录或集市，免费或付费 | **Directory starter** |
| 仅 API 后端（令牌认证、无前端） | `suprnova new my-api --api` |

三个套件都将框架跟踪为 git 依赖项，并运行在您已知的同一堆栈上 - 请参阅每个仓库的 README 了解设置。更多套件在计划中；关注[发布版本](https://github.com/eas4ai/suprnova/releases)或如果您想要一个套件，请开启一个 Issue。

## 默认脚手架为您提供的内容

如果两个套件都不适合，`suprnova new my-app --frontend svelte`（或 `react` 或 `vue`）已经包含了一个可工作的认证流程 - 登录、注册、登出、邮箱验证、密码重置、带 `authenticate` 中间件的会话认证、CSRF 保护和受保护的 `/dashboard` 路由 - 在三个前端中的任何一个上（Svelte 5、React 19、Vue 3.5），使用 Tailwind v4 和 Inertia v3。请参阅[安装](installation.md)了解脚手架输出，[快速上手](quickstart.md)了解五分钟内的演练。

对于仅 API 服务，`suprnova new my-api --api` 会初始化 Magnetar、安装 bearer-session 中间件，并针对规范的 `app_users` 表脚手架化密码注册和登录，无需前端。

## 贡献起步套件

在 Suprnova 上构建了可重用的东西，想要将其作为规范套件上游化？请参阅[贡献指南](contributions.md)。我们很高兴接受真实实现并将其打磨成通用套件。

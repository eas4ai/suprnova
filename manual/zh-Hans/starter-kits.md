# 起步套件

起步套件是即用型 Suprnova 应用程序，您可以复刻并上线。每个套件都连接了控制器、路由、迁移、前端页面和测试，以完成产品表面的全部内容 - 因此您可以从一个运行中的应用开始，而不是一个空的脚手架。

如今推出两个套件，以 Laravel 的血统为基础。挑选最接近您要构建内容的那个，然后从那里进行自定义。

## Nebula - 认证（Breeze 等级）

**仓库：[github.com/entrepeneur4lyf/Nebula](https://github.com/entrepeneur4lyf/Nebula)**

最小的完整认证套件 - Suprnova 的 Breeze 等价物。您账户所需的一切，以及您不需要的所有东西：

- 邮箱验证注册
- 带记住我功能的登录
- 具有反枚举响应的密码重置
- 个人资料管理 - 更新邮箱和密码、删除账户
- 品牌化的 Inertia 3 + Svelte 5 前端（默认深色），已连接登录用户菜单

Nebula 提供两个测试套件：门面级别的认证逻辑，以及线路级别的 HTTP 套件，驱动真实路由、会话、CSRF 往返和 guest/auth/verified 门，通过环回套接字进行。

当您需要一个干净的账户管理基础来构建您自己的产品时，请选择 Nebula。

## Pulsar - 产品站点和社区

**仓库：[github.com/entrepeneur4lyf/Pulsar](https://github.com/entrepeneur4lyf/Pulsar)**

基于 Vue 3.5 + Vuetify 的完整开发者工具/SaaS 公司网站。包含 Nebula 认证方案的全部内容，加上真实产品网站需要的表面：

- 营销落地页和用户仪表板
- Markdown 文档管道（`docs:build`），包含搜索和生成的目录
- 博客/文章系统和 RSS 源
- 公开的成员个人资料
- 分类法 - 主题、标签和类别
- 基于角色的访问控制：角色、权限和门
- 内容和成员的管理和审核表面

Pulsar 是下游产品（如 `suprnova.app`）的源套件。当您要发布一个具有文档、博客和成员社区的产品站点时（不仅仅是认证），请选择它。

## 选择哪个套件？

| 您需要… | 开始选择 |
|---|---|
| 账户和构建的地方 | **Nebula** |
| 完整的产品网站 - 落地页、文档、博客、社区、RBAC | **Pulsar** |
| 仅 API 后端（令牌认证、无前端） | `suprnova new my-api --api` |

两个套件都将框架跟踪为 git 依赖项，并运行在您已知的同一堆栈上 - 请参阅每个仓库的 README 了解设置。更多套件在计划中；关注[发布版本](https://github.com/entrepeneur4lyf/suprnova/releases)或如果您想要一个套件，请开启一个 Issue。

## 默认脚手架为您提供的内容

如果两个套件都不适合，`suprnova new my-app --frontend svelte`（或 `react` 或 `vue`）已经包含了一个可工作的认证流程 - 登录、注册、登出、带 `authenticate` 中间件的会话认证、CSRF 保护和受保护的 `/dashboard` 路由 - 在三个前端中的任何一个上（Svelte 5、React 19、Vue 3.5），使用 Tailwind v4 和 Inertia v3。请参阅[安装](installation.md)了解脚手架输出，[快速上手](quickstart.md)了解五分钟内的演练。

对于仅 API 服务，`suprnova new my-api --api` 提供相同的后端堆栈，使用基于令牌的认证而不是会话，没有前端。

## 贡献起步套件

在 Suprnova 上构建了可重用的东西，想要将其作为规范套件上游化？请参阅[贡献指南](contributions.md)。我们很高兴接受真实实现并将其打磨成通用套件。

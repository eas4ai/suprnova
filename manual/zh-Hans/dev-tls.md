# 命名 HTTPS 开发 URL (`suprnova dev:tls`)

默认情况下，`suprnova serve` 会把您的后端跑在一个裸的 `http://127.0.0.1:8765` 上。这对大多数开发场景都够用了 - 但有些浏览器特性只有在命名主机上通过 HTTPS 才能工作：

- **Passkey / WebAuthn** - 需要一个安全上下文和一个稳定的源。
- **`Secure` cookie** 和 **`SameSite=None`** - 只能在 HTTPS 下设置。
- **Service worker** - 只能在 HTTPS（或 `localhost`）下注册。
- **OAuth/OIDC 重定向 URI** - 提供者经常会拒绝裸 IP/端口的主机。

[portless](https://portless.sh) 会在端口 443 上的单个 TLS 代理背后，为每个本地应用提供一个稳定的 `https://<name>.localhost` URL。`suprnova dev:tls` 把 Suprnova 接入 portless，并且 - 这是最容易出错的部分 - 会在**您机器上的每一个浏览器证书存储**里信任 portless 的本地 CA，在 Linux 上无需 sudo。

> **严格选择加入。** portless 从不是必需的。`suprnova serve` 在没有安装 portless 的情况下也能正常工作。您可以在脚手架生成时选择加入（`suprnova new <name> --with-portless`），或者之后再添加 `portless.json`。如果您从不运行 `dev:tls`，就永远不会用到 portless。

## 安装 portless

portless 是一个 Node 工具：

```bash
npm install -g portless
```

然后一次性安装它常驻的 443 代理（这是一个系统级、需要 sudo 的步骤，属于 portless，而不是 Suprnova）：

```bash
portless service install
```

## 逐项目

您有两种方式可以让某个项目选择加入。

**用这个标志脚手架生成** - 提前写出 `portless.json`：

```bash
suprnova new myapp --frontend svelte --with-portless
```

这会在项目根目录生成一个 `portless.json`：

```json
{
  "name": "myapp",
  "appPort": 8765
}
```

`appPort` 就是您后端固定的 `SERVER_PORT`。它会告诉 portless 这个应用绑定在一个已知端口上（而不是由 portless 通过 `$PORT` 分配一个），这样命名 URL 就能直接路由到它。

**加到一个已有项目里** - 用您的 `SERVER_PORT`，手动写出同样的 `portless.json`（或者运行 `portless alias myapp 8765`）。

然后，在**每一台**会运行这个应用的机器上，执行一次性的信任 + 路由注册：

```bash
cd myapp
suprnova dev:tls
```

这会：

1. 检查 `portless` 是否在您的 PATH 上。
2. 解析出名字（`--name`，否则用 `Cargo.toml` 的 `[package].name`）和端口（`--port`，否则用 `SERVER_PORT`，再否则用 `8765`）。
3. 注册路由 `myapp.localhost → 127.0.0.1:8765`（用 `--no-alias` 跳过）。
4. 在您浏览器的证书存储里信任 portless 的 CA。
5. 打印后续步骤。

标志：

| 标志 | 效果 |
|---|---|
| `--name <name>` | 覆盖 URL 名字。默认值：`Cargo.toml` 的包名。 |
| `--port <port>` / `-p` | 覆盖路由的端口。默认值：`SERVER_PORT`，否则 `8765`。 |
| `--no-alias` | 只信任 CA；不改动 portless 的路由。 |
| `--yes` | 跳过修改您证书存储前的确认。当 CA 的指纹自上次运行以来发生变化时会被忽略 - 那种情况总会询问。 |

### 为什么第 4 步会先询问

信任一个 CA，意味着它签发的每一张证书都会被您的浏览器静默接受，对每一个站点都是如此。这值得您刻意按一次键来确认。

这个 CA 只从 portless 自身的状态里解析，绝不会受项目目录里任何东西的影响 - 一个签出的仓库没法把 `dev:tls` 指向它自己选中的 CA。这个命令会打印出它即将信任的指纹，并等待您确认。如果这个指纹和此前信任过的那个不一样，即便在 `--yes` 下它也会询问：一个变化了的 CA，要么是 portless 重装了，要么是某件您需要留意的事 - 只有您能分辨是哪一种。

## 运行

```bash
suprnova serve
```

打开 `https://myapp.localhost`。

后端默认绑定 `8765`；Vite 开发服务器则搭在 `http://localhost` 上的 `5765`。一个从 HTTPS 源提供的页面，可以引用 `http://localhost` 资源，因为浏览器把 `localhost` 当作一个安全上下文 - 它**不会**被当作混合内容而拦截。

> **HTTPS 下的模块热重载是尽力而为的。** Vite 的 HMR websocket 会连回开发服务器；这在 HTTPS 源下能不能干净地成功，取决于您的 Vite/浏览器版本。如果实时更新在 `https://` 下停止工作，请通过 `INERTIA_VITE_DEV_SERVER` 环境变量，把 Vite 指向一个 HTTPS 的开发服务器源。页面加载和流程的其余部分不受影响。

## 多个应用

portless 独占 443 端口，并按子域名做多路复用。用各自的名字和端口注册每个应用：

```bash
suprnova dev:tls --name app-one --port 8765
suprnova dev:tls --name app-two --port 8766
```

绝不要让某个应用直接绑定 443 - 那是 portless 的活。

## 故障排查

**运行 `dev:tls` 之后出现 `ERR_CERT_AUTHORITY_INVALID`。** 您的浏览器没有完全重启。浏览器只在启动时读取一次证书存储；重新加载标签页是不够的。请输入 `chrome://restart`（或者完全退出并重新打开）。

**`502 Bad Gateway`。** 代理已经启动，但您的后端没有。请在项目目录里运行 `suprnova serve`。

**`portless trust` 提示 "A terminal is required to authenticate"。** 那是 portless 自己的命令需要一个真正的 TTY 来执行 `sudo`。`suprnova dev:tls` 在 Linux 上完全绕开了这一点：它会把 CA 直接安装进您浏览器的 NSS 存储，那不需要 sudo。

**某个 Flatpak 浏览器仍然不受信任。** Flatpak 浏览器把自己的 NSS 数据库放在 `~/.var/app/<id>/.pki/nssdb` 下。`dev:tls` 会覆盖到这些 - 请重新运行它，并完全重启那个浏览器。

**`certutil: command not found`。** 请安装 NSS 工具：

| 发行版 | 命令 |
|---|---|
| Debian/Ubuntu | `sudo apt install libnss3-tools` |
| Fedora/RHEL | `sudo dnf install nss-tools` |
| Arch | `sudo pacman -S nss` |

**`portless CA not found at ~/.portless/ca.pem`。** portless 会在代理首次运行时生成自己的 CA。请先启动它一次（`systemctl start portless`，或者 `portless proxy start`），然后重新运行 `suprnova dev:tls`。

## 平台说明

上面这条浏览器 NSS 路径是 Linux 的机制。在 **macOS** 和 **Windows** 上，浏览器读取的是操作系统的钥匙串 / 证书存储，所以 `dev:tls` 把 CA 信任委托给了 `portless trust`，由它去处理那些原生存储。

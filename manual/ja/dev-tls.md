# 名前付き HTTPS 開発 URL（`suprnova dev:tls`）

デフォルトでは、`suprnova serve` はバックエンドを生の `http://127.0.0.1:8765` で提供します。たいていの開発ではこれで十分です - ただし、一部のブラウザ機能は、名前付きホストのHTTPS経由でしか動作しません:

- **パスキー / WebAuthn** - セキュアコンテキストと安定したオリジンを必要とします。
- **`Secure` クッキー**と**`SameSite=None`** - HTTPS経由でのみ設定されます。
- **サービスワーカー** - HTTPS（または `localhost`）でのみ登録されます。
- **OAuth/OIDC リダイレクトURI** - プロバイダーは、生のIP/ポートのホストをしばしば拒否します。

[portless](https://portless.sh) は、ポート443の単一のTLSプロキシの背後で、あらゆるローカルアプリに安定した `https://<name>.localhost` のURLを与えます。`suprnova dev:tls` はSuprnovaをportlessへ配線します - そして、間違えやすいのがここです - マシン上の**あらゆるブラウザの証明書ストア**でportlessのローカルCAを信頼済みにします。Linuxではsudoは不要です。

> **完全にオプトインです。** portlessは決して必須ではありません。`suprnova serve` は、portlessがインストールされていなくても動作します。オプトインするのは、スキャフォルドするとき（`suprnova new <name> --with-portless`）か、後から `portless.json` を追加するときです。`dev:tls` を一度も実行しなければ、portlessに一度も触れることはありません。

## portlessをインストールする

portlessはNodeのツールです:

```bash
npm install -g portless
```

続いて、常時起動する443番プロキシを一度だけインストールしてください（これはシステムレベルの、sudoが必要な手順であり、Suprnovaではなくportlessに属するものです）:

```bash
portless service install
```

## プロジェクトごとの設定

プロジェクトをオプトインさせる方法は2つあります。

**フラグ付きでスキャフォルドする** - 事前に `portless.json` を書き込みます:

```bash
suprnova new myapp --frontend svelte --with-portless
```

これにより、プロジェクトルートに `portless.json` が出力されます:

```json
{
  "name": "myapp",
  "appPort": 8765
}
```

`appPort` は、バックエンドの固定された `SERVER_PORT` です。これはportlessに、アプリが既知のポートにバインドしていることを伝えます（portlessが `$PORT` 経由でポートを割り当てるのではなく）。そのため、名前付きURLは直接そこへルーティングされます。

**既存のプロジェクトに追加する** - 同じ `portless.json` を、あなたの `SERVER_PORT` を使って手で書くか（あるいは `portless alias myapp 8765` を実行するか）してください。

続いて、アプリを実行する**各マシン**で、一度だけの信頼とルート登録を行います:

```bash
cd myapp
suprnova dev:tls
```

これは:

1. `portless` がPATH上にあるかを確認します。
2. 名前（`--name`、なければ `Cargo.toml` の `[package].name`）とポート（`--port`、なければ `SERVER_PORT`、それもなければ `8765`）を解決します。
3. ルート `myapp.localhost → 127.0.0.1:8765` を登録します（`--no-alias` でスキップ）。
4. ブラウザの証明書ストアにportlessのCAを信頼させます。
5. 次の手順を表示します。

フラグ:

| フラグ | 効果 |
|---|---|
| `--name <name>` | URL名を上書きします。デフォルト: `Cargo.toml` のパッケージ名。 |
| `--port <port>` / `-p` | ルーティングされるポートを上書きします。デフォルト: `SERVER_PORT`、なければ `8765`。 |
| `--no-alias` | CAだけを信頼させ、portlessのルートには触れません。 |
| `--yes` | 証明書ストアを変更する前の確認をスキップします。前回の実行からCAのフィンガープリントが変わっている場合は無視されます - その場合は常に確認を求めます。 |

### なぜ手順4はまず確認するのか

CAを信頼するということは、それが署名するすべての証明書が、あらゆるサイトについて、あなたのブラウザに無言で受け入れられるということです。それは、1回の意図的なキー入力に値します。

CAは、portless自身の状態からのみ解決され、プロジェクトディレクトリが影響を与えられるものからは決して解決されません - チェックアウトされたリポジトリが、自分で選んだCAを `dev:tls` に指し示すことはできません。コマンドは、これから信頼しようとしているフィンガープリントを表示し、あなたの確認を待ちます。フィンガープリントが以前に信頼したものと異なる場合は、`--yes` の下でも確認を求めます: 変わったCAはportlessの再インストールであるか、あなたが確認すべき何かであり、どちらであるかはあなたにしか分かりません。

## 実行する

```bash
suprnova serve
```

`https://myapp.localhost` を開きます。

バックエンドはデフォルトで `8765` にバインドされます。Viteの開発サーバーは `http://localhost` 上の `5765` に相乗りします。HTTPSオリジンから提供されたページは、`http://localhost` のアセットを参照できます。ブラウザが `localhost` をセキュアコンテキストとして扱うためです - これは、混在コンテンツとしてブロック**されません**。

> **HTTPS越しのHMRはベストエフォートです。** ViteのHMR用WebSocketは開発サーバーへ接続し直します。それがHTTPSオリジン越しにきれいに成功するかどうかは、あなたのVite/ブラウザのバージョンに依存します。ライブ更新が `https://` の下で動かなくなった場合は、`INERTIA_VITE_DEV_SERVER` 環境変数を使って、ViteをHTTPSの開発サーバーオリジンへ向けてください。ページの読み込みや、フローの残りの部分には影響しません。

## 複数のアプリ

portlessは443番を所有し、サブドメインで多重化します。各アプリを、それぞれの名前とポートで登録してください:

```bash
suprnova dev:tls --name app-one --port 8765
suprnova dev:tls --name app-two --port 8766
```

アプリから直接443番をバインドしないでください - それはportlessの仕事です。

## トラブルシューティング

**`dev:tls` を実行した後の `ERR_CERT_AUTHORITY_INVALID`。** ブラウザが完全に再起動されていません。ブラウザは起動時に一度だけ証明書ストアを読み込みます。タブのリロードでは不十分です。`chrome://restart` と入力してください（あるいは完全に終了して再起動してください）。

**`502 Bad Gateway`。** プロキシは起動していますが、バックエンドが起動していません。プロジェクトディレクトリで `suprnova serve` を実行してください。

**`portless trust` は "A terminal is required to authenticate" と表示します。** これはportless自身のコマンドが、`sudo` のために実際のTTYを必要としているだけです。`suprnova dev:tls` は、Linux上ではこれを完全に回避します。CAをブラウザのNSSストアへ直接インストールするため、sudoは不要です。

**Flatpak版のブラウザだけが信頼されていない。** Flatpak版のブラウザは、自分のNSSデータベースを `~/.var/app/<id>/.pki/nssdb` に保持しています。`dev:tls` はそれらもカバーします - 再実行して、そのブラウザを完全に再起動してください。

**`certutil: command not found`。** NSSツールをインストールしてください:

| ディストリビューション | コマンド |
|---|---|
| Debian/Ubuntu | `sudo apt install libnss3-tools` |
| Fedora/RHEL | `sudo dnf install nss-tools` |
| Arch | `sudo pacman -S nss` |

**`~/.portless/ca.pem` に `portless CA not found`。** portlessは、プロキシが最初に実行されたときにCAを生成します。一度起動してください（`systemctl start portless`、または `portless proxy start`）。その後、`suprnova dev:tls` を再実行してください。

## プラットフォームに関する注意

上記のブラウザ-NSSの経路はLinuxの仕組みです。**macOS** と**Windows** では、ブラウザはOSのキーチェーン/証明書ストアを読み取るため、`dev:tls` はCAの信頼を `portless trust` に委譲し、それがそれらのネイティブなストアを対象にします。

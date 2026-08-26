# suprnova new

`suprnova new` は、Suprnovaプロジェクトをスキャフォルドします - コントローラー、ルート、マイグレーション、Inertia SPA、そして既に配線済みの動作する認証フローを備えた、まっさらなCargoクレートです。アプリごとに一度実行し、その後は `suprnova serve` の中で過ごしてください。

## 使い方

```bash
suprnova new [name] [options]
```

`name` を省略すると、インタラクティブなウィザードがそれを尋ねます。名前は、プロジェクトディレクトリ、（snake-case化した後の）Cargoパッケージ名、そして `.env` 内のデフォルトの `APP_NAME` になります。名前はASCIIの文字/数字/`-`/`_`でなければならず、文字で始まり、パス区切り文字や `..` を含まず、64文字以下でなければなりません。

## オプション

| オプション | 説明 |
|---|---|
| `--frontend <svelte\|react\|vue>` | SPAフレームワークを非インタラクティブに選ぶ。`--api` と衝突する。 |
| `--api` | JSON:APIのみのプロジェクトをスキャフォルドする（Inertiaなし、SPAなし、セッションの代わりにトークン認証）。 |
| `--no-interaction` | すべてのプロンプトをスキップし、デフォルト値を使う（名前は `my-suprnova-app`、フロントエンドは `svelte`、作成者/説明は空）。 |
| `--no-git` | 新しいプロジェクトで `git init` をスキップする。 |
| `--with-portless` | [`suprnova dev:tls`](dev-tls.md) が `https://<name>.localhost` でアプリを配信できるよう、`portless.json` を出力する。オプトイン方式であり、他には何も変更しない。 |

## インタラクティブモード

```bash
suprnova new my-app
```

ウィザードは、次の順序で4つの質問をします:

1. **プロジェクト名** - ディレクトリの引数（`my-app`）がデフォルトになる
2. **説明** - Cargoパッケージの説明として使われる
3. **作成者** - Cargoパッケージの作成者として使われる。設定されていれば `git config user.name <name@email>` がデフォルトになる
4. **フロントエンドフレームワーク** - `Svelte (recommended)`、`React`、または `Vue`

確認すると、スキャフォルダーはプロジェクトを書き込み、（`--no-git` でなければ）`git init` を実行し、次のステップを出力します:

```
Backend  http://localhost:8765
Frontend http://localhost:5765
```

## 非インタラクティブモード

CI、dotfiles、あるいはスクリプト化されたセットアップには、`--no-interaction` に加えて、上書きしたいフラグを渡してください:

```bash
suprnova new my-app --frontend svelte --no-interaction
```

`--no-interaction` の下でのデフォルト:

- フロントエンド: `svelte`
- 説明: `"A web application built with Suprnova"`
- 作成者: 空
- Git: 初期化される

`--description` や `--author` というフラグはありません。それらの値は、インタラクティブなプロンプト経由でのみ設定されるか、デフォルト値を受け入れます。

## APIのみのプロジェクト

SPAのないサービスバックエンドには、`--api` を使ってください:

```bash
suprnova new my-api --api
```

APIスターターは、大幅に小さくなります: `frontend/` ディレクトリなし、Inertiaなし、認証ビューなし、単一クレートの `src/main.rs` レイアウトです。共有するSeaORM接続に対してMagnetarを初期化し、正規の `app_users` モデルを作成して `BearerTokenMiddleware` を導入し、登録とログインに `Auth::password()` を使用します。生成されるbootstrapは、ローカルのデフォルト値を伴う `PASSKEY_RP_ID` と `PASSKEY_RP_ORIGIN` を読み取ります。スターターにはサンプルのusersコントローラーと `UserResource` JSONシリアライザーも含まれ、`.env` でポート8765にバインドします。

`--api` は `--frontend` と互いに排他的であり、両方を渡すとエラーになります。`--api` の下では、プロジェクト名だけが尋ねられます - 説明/作成者/フロントエンドのプロンプトはスキップされます。

## スキャフォルドされる内容

完全なディレクトリツアーは[ディレクトリ構成](structure.md)にあります。短縮版は次のとおりです:

- `cmd/main.rs` - バイナリのエントリ。`Application::new()…run()` を呼び出す
- `src/` - コントローラー、アクション、コマンド、設定、ミドルウェア、モデル、マイグレーション、それに `bootstrap.rs` と `routes.rs`。生成された `bootstrap.rs` は、グローバルなミドルウェアのチェーン - ロギング、セッション、ロケール、CSRF、includeのパース - を配線し、[`Inertia::install`](frontend-inertia-responses.md)を呼び出します。これは、Inertiaプロトコルのミドルウェア（アセットバージョンの `409`、非GETのリダイレクトでの `302 → 303`）を追加します。それが広告するアセットバージョンは、Viteビルドマニフェストのハッシュがデフォルトで使われるため、フロントエンドのビルドを出荷すると自動的に変わります - [バージョン検出](frontend-inertia-responses.md)を参照してください。同じ呼び出しが、あなたがスキャフォルドしたフロントエンドをピン留めするため、HTMLシェルはそのフレームワークのViteのエントリポイントをロードします。`.env` は、CLI自身のジェネレーターのために、対応する `SUPRNOVA_FRONTEND` を運びます
- `src/bin/console.rs` - プロジェクトごとの `php artisan` に相当するもの
- `frontend/` - Vite 8 + Tailwind v4 + あなたが選んだフレームワーク。Home / Dashboard / Login / Register の各ページが、既にInertiaを通じて配線済み
- `src/migrations/` - `users`、`sessions`、`remember_tokens` の各テーブルが、すぐに使える状態で
- `.env` - デフォルトではSQLiteデータベース。オペレーターの介入なしにアプリが起動するよう、新しく生成された `APP_KEY` を伴う
- `.gitignore`、`Cargo.toml`

### Suprnovaが異なる設計を選んだ理由

LaravelはBladeを同梱しており、後からBreeze/Jetstream経由でフロントエンドを引き込みます。Suprnovaは逆の道を行きます: `suprnova new` は常に、本物のSPA（Inertia上のSvelte/React/Vue）か、本物のJSON:APIプロジェクトのどちらかをスキャフォルドします。テンプレートエンジンを最初に置くスターターは存在しません - サーバーレンダリングのHTMLが欲しければTeraが利用できますが、それはデフォルトの形ではなく、ビューをアプリの前面に置くスキャフォルダーの経路もありません。

デフォルトのフロントエンドは、Reactではなく **Svelte 5**（runes有効）です。3つの中で実行時に最も軽量であり、フレームワークの「実行時の賢さより、コンパイル時の勝利を」という哲学に最も近いため、これを選びました。ReactとVueも同じくファーストクラスです - あなたのチームが知っているものを選んでください。

## 配布

CLI自体は、crates.ioではなくgit経由で出荷されます（プレローンチのため）:

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.5 suprnova-cli
```

同じコマンドに `--force` を付けると、既存のインストールを更新します。スキャフォルドされたプロジェクトも、同じやり方でフレームワークのクレートに依存します - それらの `Cargo.toml` の中にある、現在のリリースタグにピン留めされたgit依存性です。ツールチェーンの前提条件の全体については、[インストール](installation.md)を参照してください。

## 次のステップ

- [インストール](installation.md) - Rust/Node/DBの前提条件とツールチェーンのセットアップ
- [ディレクトリ構成](structure.md) - スキャフォルドされた各ファイルの内容
- [クイックスタート](quickstart.md) - `suprnova new` の後の最初の5分間
- [suprnova serve](cli-serve.md) - 次に使う開発ランナー
- [コンソール](console.md) - `cargo run --bin console` と `#[command]` の仕組み

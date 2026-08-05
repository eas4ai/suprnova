# suprnova serve

`suprnova serve` は、バックエンドとViteの開発サーバーを、両側のホットリロードとともに一緒に実行し、さらに `#[derive(InertiaProps)]` 構造体に触れるたびに自動でTypeScriptの型を再生成します。構築している間、ターミナルで開いたままにしておく、唯一のコマンドです。

```bash
suprnova serve
```

両方のプロセスは、誰が何を言ったか分かるように、色付きの `[backend]` と `[frontend]` のプレフィックスを付けて、同じターミナルへstdoutをストリーミングします。`Ctrl+C` は、両方をきれいにシャットダウンします。

## 使い方

```bash
suprnova serve [OPTIONS]
```

| オプション | デフォルト | 説明 |
|---|---|---|
| `-p, --port <PORT>` | `8765`（CLI）/ `$SERVER_PORT`（env） | バックエンドのHTTPポート |
| `--frontend-port <PORT>` | `5765`（CLI）/ `$VITE_PORT`（env） | Viteの開発サーバーポート |
| `--backend-only` | `false` | Viteの開発サーバーをスキップする |
| `--frontend-only` | `false` | バックエンドをスキップし、Viteだけを実行する |
| `--skip-types` | `false` | Rustの変更時にTypeScriptの型を再生成しない |

CLIのフラグは環境変数より優先され、環境変数は組み込みのデフォルト値より優先されます。スキャフォルドされた `.env` には `SERVER_PORT=8765` と `VITE_PORT=5765` が同梱されており、`--port` で上書きしない限り、それらの値が使われます。

## 例

### デフォルト - 両方のサーバー

```bash
suprnova serve
```

出力:

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765
[backend] Compiling my-app v0.1.0 ...
[frontend] VITE v6.3.0  ready in 312 ms
```

ブラウザで `http://127.0.0.1:8765` にアクセスしてください。バックエンドはInertiaのHTMLシェルを配信し、アセットのリクエストをViteへプロキシするため、ViteのURLに直接アクセスする必要はありません。

### カスタムポート

```bash
suprnova serve --port 3000 --frontend-port 3001
```

あるいは `.env` で設定し、フラグなしで実行してください:

```env
SERVER_PORT=3000
VITE_PORT=3001
```

### バックエンドのみ

```bash
suprnova serve --backend-only
```

APIのみのプロジェクトで作業しているとき、あるいはフロントエンドが既に別のターミナル（または別のマシン、あるいはデプロイ済みのプレビュー）で実行されているときに便利です。

### フロントエンドのみ

```bash
suprnova serve --frontend-only
```

保存ごとにRustの再ビルドのコストを払わずにUIで作業したいとき、あるいはバックエンドが別のシェル（またはDocker内）で実行されているときに便利です。

### 型生成をスキップする

```bash
suprnova serve --skip-types
```

TypeScriptの再生成ウォッチャーを無効化します。`frontend/src/types/inertia-props.ts` を手で管理しているとき、あるいはInertiaのコードから遠く離れた場所で作業していて、より静かな出力を望むときに使ってください。

## 実際に行われること

`suprnova serve` を実行すると、CLIは次を行います:

1. 現在のディレクトリから `.env` を読み込む。
2. バックエンドとフロントエンドのポートを解決する（CLIフラグ → 環境変数 → デフォルト）。
3. Suprnovaプロジェクトの中にいることを確認する - （`--frontend-only` でなければ）`Cargo.toml` が存在しなければならず、（`--backend-only` でなければ）`frontend/` ディレクトリが存在しなければならない。
4. `src/` の中で見つけたあらゆる `#[derive(InertiaProps)]` 構造体からTypeScriptの型を再生成し、`frontend/src/types/inertia-props.ts` に書き込む。
5. `cargo-watch` がまだPATH上になければ、`cargo install --locked --version "^8.5" cargo-watch` 経由でインストールする（一度だけで、「Installing...」という通知が出る）。`--frontend-only` の下ではスキップされる。バージョンが縛られているのは、`serve` が `cargo watch -x` を駆動しており、そのセマンティクスがメジャーバンプをまたいで保証されないからです。`--locked` は、インストール時に依存関係ツリーを再解決するのではなく、cargo-watchが公開した依存関係ツリーをそのままビルドします。開発サーバーの起動の副作用としてソフトウェアをインストールするコマンドは、あなたに代わってバージョンまで選ぶべきではありません。
6. `node_modules` がまだ存在しなければ、`frontend/` の中で `npm install` を実行する。`--backend-only` の下ではスキップされる。
7. バックエンドのために `cargo watch -x 'run --bin <package-name>'` を起動する。`cargo-watch` は `.rs` ファイルが変更されるたびにバイナリを再実行する。
8. Viteのために `frontend/` の中で `npm run dev` を起動する。これにより、Svelte/React/VueのコンポーネントとTailwindのクラスに対するHMRが手に入る。
9. `src/` に対するファイルウォッチャーを起動し、`.rs` ファイルが変更されるたびに、保存のバーストが500ms静かになった時点で型ジェネレーターを再実行する。デバウンスはトレイリングエッジ方式であるため、バースト - `cargo fmt`、複数ファイルをまたぐformat-on-save、ブランチの切り替え - は、最初のファイルで発火して残りを見逃すのではなく、最後の書き込みの*後に*走る、ちょうど1回の再生成へまとめられます。
10. `[backend]` と `[frontend]` のプレフィックスを付けて、両方の子プロセスのstdout/stderrをターミナルへ転送する。

`Ctrl+C` は、マネージャーにシャットダウンフラグを立てさせ、両方の子プロセスをkillして終了させる信号を送ります。いずれかのプロセスが自分自身で終了した場合 - 通常は `cargo watch` が回復できないほど深刻なRustのコンパイルエラー、あるいはポートの競合が原因です - マネージャーはそれをシャットダウン信号として扱い、もう一方を取り壊します。

### Suprnovaが異なる設計を選んだ理由

Laravelのユーザーは通常、バックエンドには `php artisan serve` を、別のターミナルでは `npm run dev` を実行し、ほとんどのチームは、2つのターミナルへの分割を `Procfile` と `foreman`/`overmind` で覆い隠しています。Suprnovaは、そのマルチプレクサーをファーストクラスのCLIコマンドとして出荷します。得られるのは、1つのターミナル、1回の `Ctrl+C`、自動的なツールチェーンのブートストラップ（`cargo-watch`、`npm install`）、そして手動での型同期なしに、あなたのSvelte/React/Vueコンポーネントが常に現在のprop形状を見られるよう、`frontend/src/types/inertia-props.ts` をその場で再生成する、型付きInertiaブリッジです。

## ホットリロード

**バックエンド。** `cargo watch -x 'run --bin <package>'` がそのループです。プロジェクト内のあらゆる `.rs` の変更ごとに、サーバーを再ビルドして再起動します。重いクレートに触れた後のコールドリビルドは数秒かかることがあります。単一ファイル内の差分ビルドは、通常1秒未満です。

**フロントエンド。** ViteのHMRは、コンポーネントの状態を保ったまま、フルリロードなしでコンポーネントの変更をその場に注入します。Tailwindのクラスは、Tailwind v4のウォッチャー経由でライブに更新されます。

**TypeScriptの型。** `.rs` ファイルが変更されるたびに、型ウォッチャーはジェネレーターを再実行します。新しい `#[derive(InertiaProps)]` 構造体が現れる（あるいは既存のものが形状を変える）と、再生成された `frontend/src/types/inertia-props.ts` は、それらをインポートしているコンポーネントに対してViteのHMRを引き起こします。

## トラブルシューティング

### ポートが既に使用中

```text
[backend] Error: Address already in use (os error 98)
```

プロセスを見つけてkillするか、別のポートを選んでください:

```bash
lsof -i :8765
kill -9 <pid>

# または
suprnova serve --port 8081
```

### `cargo-watch` のインストールに失敗する

CLIは、`cargo-watch` がまだPATH上になければ `cargo install cargo-watch` を実行します。そのインストールが失敗した場合（ネットワークがない、制限された環境など）、一度だけ手動でインストールしてください:

```bash
cargo install cargo-watch
```

その後は、`suprnova serve` がそれを見つけ、再びインストールを試みることはありません。

### フロントエンドの依存関係が詰まる

ブートストラップの途中で `npm install` が失敗した場合は、原因を修正し（npmレジストリへの到達可能性、ディスク容量、ロックファイルの健全性）、手動で実行してください:

```bash
cd frontend && npm install
```

その後 `suprnova serve` を再実行してください。CLIは `node_modules` が欠けているときにだけ `npm install` を自動実行するため、手動インストールが成功していれば、そのステップはスキップされます。

### 型の再生成が変更を検知しない

ウォッチャーは2秒ごとにポーリングします（`notify` をポーリング間隔付きで使っており、inotifyの癖よりクロスプラットフォームの信頼性を優先して選ばれています）。そして再生成を500msに1回までデバウンスします。変更が反映されない場合:

- ファイルが `src/` の下にあることを確認してください（ウォッチャーは `crates/`、`cmd/`、`migrations/` の中には再帰しません）。
- 構造体が実際に `#[derive(InertiaProps)]` を持っていることを確認してください。
- `suprnova serve` を再起動し、起動時の `Generated N type(s)` というメッセージを確認してください - `No InertiaProps structs found` が見えたら、スキャナーは出力すべきものを何も見つけられなかったということです。

### バックエンドが起動直後にサイレントに終了する

いずれかの子プロセスが終了すると、マネージャーはもう一方もシャットダウンします。バックエンドがコンパイルエラーで死んだ場合、「Servers stopped.」というメッセージの直前にある `[backend]` の行に、rustcからの `error[E…]` が表示されます。コンパイルエラーを修正して再実行してください。

## 次のステップ

- [インストール](installation.md) - CLIをマシンに入れる
- [クイックスタート](quickstart.md) - 最初のアプリを一通り歩く
- [ディレクトリ構成](structure.md) - `suprnova new` がスキャフォルドした内容
- [コード ジェネレーター](cli-generators.md) - `make:controller`、`make:action` など
- [コンソール](console.md) - プロジェクトごとの `cargo run --bin console` バイナリ

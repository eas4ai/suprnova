# 開発

Suprnova の日々のワークフローは 1 つのコマンドです: `suprnova serve`。これは Rust バックエンド、Vite フロントエンド、および TypeScript 型再生成ツールを 1 つのプロセスで実行し、それぞれ適切なファイルをウォッチします。このチャプターでは開発サーバー、ホットリロードの仕組み、日々使用するコマンドについて説明します。初回セットアップについては[インストール](installation.md)を参照してください。ディレクトリ構成については[ディレクトリ構成](structure.md)を参照してください。

## 開発サーバー

スキャフォルドされたプロジェクトのルートから:

```bash
suprnova serve
```

CLI は 2 つの URL を出力し、その後、各子プロセスからプリフィックス付きの出力を継続的にストリーミングします:

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765

[backend]  Compiling links v0.1.0
[backend]  Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.21s
[backend]  Running `target/debug/links`
[frontend] VITE v6.0.1  ready in 312 ms
[frontend]   ➜  Local:   http://localhost:5765/
[types]    Watching for Rust file changes to regenerate types
```

バックエンド URL (`127.0.0.1:8765`) にアクセスします。Vite は Inertia の開発統合経由で JS/CSS をサーブしており、`:5765` に直接アクセスする必要はありません。`Ctrl+C` を 1 回押すと、CLI は両方の子プロセスをクリーンにシャットダウンします。

### フラグ

| フラグ | デフォルト | 説明 |
|---|---|---|
| `-p`, `--port <N>` | `8765` | バックエンド ポート |
| `--frontend-port <N>` | `5765` | Vite ポート |
| `--backend-only` | off | Vite 子プロセスをスキップ （API のみの作業） |
| `--frontend-only` | off | バックエンド子プロセスをスキップ （他の場所で実行中のバックエンドに対するコンポーネント作業） |
| `--skip-types` | off | TypeScript 型ジェネレーター + ウォッチャーをスキップ |

同じポートは `.env` で `SERVER_PORT` と `VITE_PORT` を経由して設定できます。コマンドラインのフラグは `.env` より優先されます。

### プリフライト チェック

何かを起動する前に、`suprnova serve` は:

1. **プロジェクトにいることを確認します。** `Cargo.toml` がない場合 （またはフロントエンド実行時に `frontend/` がない場合）、明確なエラーで中止します。
2. **TypeScript 型を一度生成します。** `src/` をスキャンして `#[derive(InertiaProps)]` を探し、`frontend/src/types/inertia-props.ts` に書き込みます。`--skip-types` または `--frontend-only` でスキップされます。
3. **不足している場合は `cargo-watch` をインストールします。** 新しいマシンでの最初の実行は `cargo install cargo-watch` を実行してから続行します。
4. **`frontend/node_modules` がない場合は `npm install` を実行します。** 新規クローンで手動インストール ステップは不要です。

## ホットリロード

`suprnova serve` 内で 3 つのウォッチャーが同時に実行されます:

- **`cargo watch -x 'run --bin <pkg>'`** がバックエンドを駆動します。プロジェクト内の任意の `.rs` 変更は再コンパイルとプロセス内再起動をトリガーします。コンパイル エラーは `[backend]` ストリームに出力され、前のバイナリは次の成功したビルドまで実行が続きます。
- **Vite** がフロントエンドを駆動します。コンポーネント、スタイル、アセットの編集はホット モジュール リプレースメント経由でブラウザ タブに完全リロードなしで反映されます。
- **`notify` ベースの型ウォッチャー** は `.rs` ファイルが変更されるたびに InertiaProps スキャナーを再実行します。500ms でデバウンスされるため、保存バーストは `inertia-props.ts` を 1 回だけ再生成します。出力は `[types]` プリフィックスで表示されます。

3 番目のものは考える必要がない部分です: `#[derive(InertiaProps)]` struct のフィールドをリネームすると、一致する TypeScript インターフェースは次の保存で自動的についてきます。Svelte/React/Vue ページは新しい型を即座に取得します。通常の開発中に `suprnova generate-types` 呼び出しは不要です。

### Suprnovaが異なる設計を選んだ理由

ほとんどの Rust Web スタックはホットリロードをあなたの問題にします - 独自のファイル ウォッチャーを選び、独自の再起動ラッパーを書き、Vite を別のターミナルで実行します。ほとんどの Laravel スタックは TypeScript 型をあなたの問題にします - 2 つの場所 （PHP と TS） で宣言し、同期を保ちます。`suprnova serve` は両方のウォッチャー、およびフロントエンド型を正確に保つ型ジェネレーターを 1 つの監視されたプロセスとして実行します。Tokio ランタイムは「同時に多くのことを行う」コストを十分に低くしており、開発ループはそれを自由に費やせます。

## 日々のコマンド

頻繁に実行するいくつかのコマンド:

```bash
suprnova serve                    # 開発を開始（バックエンド + Vite + 型ウォッチャー）
suprnova make:controller orders   # コントローラーをスキャフォルド
suprnova make:migration add_idx   # マイグレーションをスキャフォルド
suprnova db:sync                  # マイグレーション実行、SeaORM エンティティを再生成
suprnova migrate:status           # 適用内容を表示
suprnova migrate:fresh            # テーブル削除 + スクラッチから再実行
suprnova key:generate --show      # APP_KEY をローテーション
cargo run --bin console <cmd>     # `#[command]` アノテーション付きのコンソール ハンドラ
cargo test                        # テスト スイート実行
```

`db:sync` は開発用のショートカットで「マイグレーション + エンティティ再生成を 1 ステップで」という意味です。本番環境では `suprnova migrate` を使用します。リリース用マシンで再生成が発生しないようにするためです。完全なジェネレーター表面は[コード ジェネレーター](cli-generators.md)にあり、マイグレーション verb は[マイグレーション](migrations.md)にあります。

## デバッグ

### ロギング

Suprnova は `tracing` をエンドツーエンドで使用します。`LOG_LEVEL` で出力内容をフィルタリングします （`tracing-subscriber` の `EnvFilter` と同じ構文）:

```bash
# 詳細フレームワーク出力
LOG_LEVEL=debug suprnova serve

# hyper は静かに、クレートは詳細に
LOG_LEVEL=info,my_app=debug,hyper=warn suprnova serve
```

出力フォーマットは `LOG_FORMAT` で制御されます （`pretty` は人間が読める形式、`json` はマシンが解析可能な形式）。開発時のデフォルトは `pretty` です。完全なロギング表面については[可観測性](observability.md)を参照してください。

### SQL クエリ

1 つの環境変数でクエリごとのロギングを有効にします:

```env
DB_LOGGING=true
```

これはすべての SeaORM クエリを `tracing` 経由で `info` レベルにルーティングするため、正確に何が実行されているかを確認できます。本番環境ではオフにしておきます （特定の遅いクエリを追跡している場合を除く - ログ量はすぐにノイズになります）。

### バックトレース

標準的な Rust:

```bash
RUST_BACKTRACE=1 suprnova serve
```

ハンドラのパニックはキャッチされ、構造化された 500 レスポンスに変換されます。バックトレースはサーバーを停止させることなくログに記録されます。そのコントラクトの仕組みについては[エラー モデル](error-model.md)を参照してください。

## ループ内のテスト

```bash
cargo test                        # ワークスペース全体
cargo test -p my_app              # アプリ クレートのみ
cargo test some_test_name         # 名前でフィルタリング
cargo test -- --nocapture         # println!/tracing 出力を表示
```

テスト実行はプレーンな Cargo です。フレームワーク側のヘルパー （`#[suprnova_test]`、`TestDatabase`、`expect!`、Mail/Queue/Storage/などのフェイク） は[テスト](testing.md)および[データベース テスト](database-testing.md)にドキュメント化されています。これらはすでに知っている `cargo test` と同じ下で実行されます。

## SSR ワーカー の使用

アプリが Inertia サーバー側レンダリングを使用する場合、開発中は `suprnova serve` と共に SSR ワーカー が必要です:

```bash
# ターミナル 1
suprnova serve

# ターミナル 2
suprnova ssr:start
```

`ssr:start` はバンドルされた SSR ワーカー を Node、Bun、または Deno 下で実行します (`--runtime`)。`ssr:check` は実行中のワーカーに到達可能かどうかを確認します。両方ともフロントエンド チャプターの下にドキュメント化されています - [フロントエンド](frontend.md)を参照してください。

## 何か問題があるとき

最も一般的な開発ループの問題を短く分類したリスト:

- **ポートが既に使用中。** 別の `suprnova serve` がまだ起動中、または以前のバックエンドが問題を起こしています。`lsof -i :8765` で探すか、`--port 8001` を渡します。
- **`cargo-watch` が再コンパイルし続ける。** いくつかのエディタが保存時にファイルを書き直します （フォーマッター、自動修正付きリンター）。プロジェクトの保存時フォーマットを無効にするか、ウォッチャーを `CARGO_WATCH_IGNORE` パターンでスコープします。
- **TypeScript 型が更新されない。** `--skip-types` が渡されたか、またはウォッチャーが `.rs` 解析エラーに突き当たりました。`[types]` 行を確認します - 警告を出力して続行し、serve 全体を失敗させません。
- **Vite エラーだがバックエンド問題なし。** `frontend/` で `npm install` を 1 回実行します （CLI は最初の serve で実行しますが、`node_modules` を削除した場合は、新規開始時にそのディレクトリが再度見つからなくなるまで再度実行しません）。

その他の場合、[エラーハンドリング](errors.md)チャプターはより深い分類パターンをカバーしています。

## 次のステップ

- [インストール](installation.md) - CLI とプロジェクトの初回セットアップ
- [クイックスタート](quickstart.md) - 小さなアプリをエンドツーエンドで構築
- [ディレクトリ構成](structure.md) - 各ディレクトリの内容
- [コード ジェネレーター](cli-generators.md) - すべての `make:*` コマンド
- [テスト](testing.md) - `#[suprnova_test]`、フェイク、テスト データベース

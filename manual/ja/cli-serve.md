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
| `--no-restart` | `false` | クラッシュした開発プロセスを再起動せず、代わりにセッション全体を終了する（従来の動作） |
| `--restart-tries <N>` | `5` | 連続クラッシュがこの回数に達したら、そのプロセスの再試行を諦める。`--no-restart` と併用した場合は無視され、最初のクラッシュでセッションが終了する。 |
| `--timestamps` | `false` | 各出力行の先頭に `HH:MM:SS` の時計時刻を付ける |
| `--json` | `false` | プレフィックス付きテキストの代わりに、1行1オブジェクト（NDJSON）をstdoutへ出力する - [JSON出力](#json出力)を参照。`--timestamps` との併用はエラーにならないが、すべてのイベントに時刻があるため `--timestamps` の追加効果はない。 |

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

### APIのみのプロジェクト

`suprnova new --api` でスキャフォルドされたプロジェクトには、`frontend/` ディレクトリがありません。`serve` は、他のどこでもそうするのとまったく同じように実行してください:

```bash
suprnova serve
```

`serve` には `frontend/package.json` が見えないため、Viteのペインと、それに材料を供給するTypeScriptの生成をスキップし、バックエンドを実行します。そのようなプロジェクトでも `--frontend-only` は依然としてエラーです: 存在しないただ1つのペインを要求しているからです。

### 型生成をスキップする

```bash
suprnova serve --skip-types
```

TypeScriptの再生成ウォッチャーを無効化します。`frontend/src/types/inertia-props.ts` を手で管理しているとき、あるいはInertiaのコードから遠く離れた場所で作業していて、より静かな出力を望むときに使ってください。

## 実際に行われること

`suprnova serve` を実行すると、CLIは次を行います:

1. 現在のディレクトリから `.env` を読み込む。
2. バックエンドとフロントエンドのポートを解決する（CLIフラグ → 環境変数 → デフォルト）。
3. Suprnovaプロジェクトの中にいることを確認する - （`--frontend-only` でなければ）`Cargo.toml` が存在しなければならず、`--frontend-only` には `package.json` を持つ `frontend/` ディレクトリが必要になる。それを持たないプロジェクトは、拒否されるのではなくバックエンドのみで提供される。
4. `src/` の中で見つけたあらゆる `#[derive(InertiaProps)]` 構造体からTypeScriptの型を再生成し、`frontend/src/types/inertia-props.ts` に書き込む。プロジェクトにフロントエンドがない場合はスキップされる。
5. `cargo-watch` がまだPATH上になければ、`cargo install --locked --version "^8.5" cargo-watch` 経由でインストールする（一度だけで、「Installing...」という通知が出る）。`--frontend-only` の下ではスキップされる。バージョンが縛られているのは、`serve` が `cargo watch -x` を駆動しており、そのセマンティクスがメジャーバンプをまたいで保証されないからです。`--locked` は、インストール時に依存関係ツリーを再解決するのではなく、cargo-watchが公開した依存関係ツリーをそのままビルドします。開発サーバーの起動の副作用としてソフトウェアをインストールするコマンドは、あなたに代わってバージョンまで選ぶべきではありません。
6. `node_modules` がまだ存在しなければ、`frontend/` の中で `npm install` を実行する。`--backend-only` の下と、プロジェクトにフロントエンドがない場合はスキップされる。
7. バックエンドのために `cargo watch -x 'run --bin <package-name>'` を起動する。`cargo-watch` は `.rs` ファイルが変更されるたびにバイナリを再実行する。
8. Viteのために `frontend/` の中で `npm run dev` を起動する。これにより、Svelte/React/VueのコンポーネントとTailwindのクラスに対するHMRが手に入る。`--backend-only` の下と、プロジェクトにフロントエンドがない場合はスキップされる。
9. プロジェクトの `Suprnova.toml` に宣言された追加プロセスをすべて起動する（下記の[追加の開発プロセス](#追加の開発プロセス)を参照）。それぞれに独自の `[name]` プレフィックスを付けます - キューワーカー、ログテイラーなど、別のターミナルで動かしていたものです。
10. `src/` に対するファイルウォッチャーを起動し、`.rs` ファイルが変更されるたびに、保存のバーストが500ms静かになった時点で型ジェネレーターを再実行する。ステップ4の起動時の型生成と同じく、プロジェクトにフロントエンドがない場合はスキップされる。デバウンスはトレイリングエッジ方式であるため、バースト - `cargo fmt`、複数ファイルをまたぐformat-on-save、ブランチの切り替え - は、最初のファイルで発火して残りを見逃すのではなく、最後の書き込みの*後に*走る、ちょうど1回の再生成へまとめられます。
11. `[name]` プレフィックス（`[backend]`、`[frontend]`、またはプロセス設定名）を付け、任意で `--timestamps` により時刻を付けて、すべての子プロセスのstdout/stderrをターミナルへ転送する - `--json` の場合は代わりにNDJSONイベントとして出力します（下記の[JSON出力](#json出力)を参照）。

`Ctrl+C` は、マネージャーにシャットダウンフラグを立てさせ、すべての子プロセスをkillして終了させる信号を送ります。いずれかの子プロセスが自分自身で終了した場合 - `cargo watch` が回復できないほど深刻なRustのコンパイルエラー、クラッシュしたVite、失敗した `Suprnova.toml` プロセスなど - セッション全体を取り壊す代わりに、短いバックオフ（200msから始まり、連続クラッシュごとに倍増して最大5秒、30秒間稼働するとリセット）の後で再起動します。従来の動作に戻すには `--no-restart` を渡してください。どの子プロセスの終了でもセッション全体が直ちに終了します。

クラッシュし続けるプロセスを無限に再試行することはありません。`--restart-tries`（既定 `5`）は、`serve` がその1プロセスについて再試行する連続クラッシュの回数を上限にします。30秒間の稼働で、バックオフと同じく回数もリセットされます。諦めると実行可能なメッセージを表示し、そのプロセスだけの再試行を止めます。他のプロセスとセッション自体は動き続け、Laravel自身の `concurrently --restart-tries=5` の既定値に合わせています。[トラブルシューティング](#プロセスがクラッシュループし続ける)を参照してください。

### Suprnovaが異なる設計を選んだ理由

Laravelのユーザーは通常、バックエンドには `php artisan serve` を、別のターミナルでは `npm run dev` を実行し、ほとんどのチームは、2つのターミナルへの分割を `Procfile` と `foreman`/`overmind` で覆い隠しています。Suprnovaは、そのマルチプレクサーをファーストクラスのCLIコマンドとして出荷します。得られるのは、1つのターミナル、1回の `Ctrl+C`、自動的なツールチェーンのブートストラップ（`cargo-watch`、`npm install`）、そして手動での型同期なしに、あなたのSvelte/React/Vueコンポーネントが常に現在のprop形状を見られるよう、`frontend/src/types/inertia-props.ts` をその場で再生成する、型付きInertiaブリッジです。

Laravelの `dev` コマンドには `--tabs` と `--stream` のモードもあり、それぞれ小さなNode TUI（`@laravel/multiplex`）を通して出力を描画します。SuprnovaはTUIを出荷しません。単一ターミナルのプレフィックス付き出力はRust開発ツールのエコシステム（`cargo watch`、`bacon`、`just`）全体で標準であり、色付きプレフィックスを持つプロセスレジストリがTUIの提供する「どのプロセスが言ったか」という信号をすでに与えるからです。`--stream` の基礎となる仕事、つまりスクリプト可能なリアルタイムイベントストリームは `--json` として提供します（[JSON出力](#json出力)を参照）。`--tabs` のマルチペインTUIは意図的に採用しません。これは、このページがすでに解決している問題のために、端末間で動作し続ける別の対話モデルと別のライブラリを保守することになるためです。[Parity](parity.md#what-we-won-t-ship-and-why)の対応する行を参照してください。

## ホットリロード

**バックエンド。** `cargo watch -x 'run --bin <package>'` がそのループです。プロジェクト内のあらゆる `.rs` の変更ごとに、サーバーを再ビルドして再起動します。重いクレートに触れた後のコールドリビルドは数秒かかることがあります。単一ファイル内の差分ビルドは、通常1秒未満です。

**フロントエンド。** ViteのHMRは、コンポーネントの状態を保ったまま、フルリロードなしでコンポーネントの変更をその場に注入します。Tailwindのクラスは、Tailwind v4のウォッチャー経由でライブに更新されます。

**TypeScriptの型。** `.rs` ファイルが変更されるたびに、型ウォッチャーはジェネレーターを再実行します。新しい `#[derive(InertiaProps)]` 構造体が現れる（あるいは既存のものが形状を変える）と、再生成された `frontend/src/types/inertia-props.ts` は、それらをインポートしているコンポーネントに対してViteのHMRを引き起こします。

## 追加の開発プロセス

`suprnova serve` は常にバックエンドとViteを実行しますが、ほとんどのプロジェクトでは、実行し続けるものは2つより多くあります - キューワーカー、ログテイラー、メールキャッチャーなどです。プロジェクトルートの `Suprnova.toml` にそれらを宣言すると、`serve` はバックエンドとフロントエンドのすぐ隣で起動し、プレフィックスを付け、自動再起動します:

```toml
[[serve.process]]
name = "queue"
command = "cargo"
args = ["run", "--bin", "console", "--", "queue:work"]
color = "yellow"

[[serve.process]]
name = "logs"
command = "tail"
args = ["-f", "storage/logs/app.log"]
```

各エントリには `name` と `command` が必要です。`args` のデフォルトはなし、`color` のデフォルトは緑/黄/青/白から宣言順に割り当てられる色です（または8つの名前付き `console` 色 - black、red、green、yellow、blue、magenta、cyan、white - のいずれかを選べます）。名前は一意でなければなりません。`Suprnova.toml` は完全に任意であり、ないプロジェクトは以前とまったく同じように動きます。

### Suprnovaが異なる設計を選んだ理由

LaravelはPHPから追加の `dev` プロセスを登録します - 通常はサービスプロバイダーの `boot()` にある `DevCommands::register($command, $name)` です。`php artisan dev` は、すでにアプリケーションをブートした同じプロセスの内部からマルチプレクサーをexecするためです。`suprnova serve` はアプリケーションとは別のバイナリです。アプリケーションのRustコードをリンクも実行もせず、`cargo watch` と `npm` に対してシェルを起動するだけです。フックできるアプリケーションのブートがないため、登録はコードが呼び出すものではなくCLIが読むデータでなければなりません - それが `DevProcesses::register()` APIではなく `Suprnova.toml` である理由です。

## JSON出力

`--json` を渡すと、`suprnova serve` は色付きの `[name]` プレフィックス付きテキストの代わりに、1行1オブジェクトのNDJSONをstdoutへ書き込みます。有効な間は他のものをstdoutへ出力しないため、`jq` や他の行指向JSONコンシューマーへそのままパイプできます。すべての行には `type` フィールドがあります:

| `type` | フィールド | 意味 |
|---|---|---|
| `started` | `ts`、`name`、`pid` | プロセス（バックエンド、フロントエンド、または `Suprnova.toml` のエントリ）が初めて起動された。 |
| `output` | `ts`、`name`、`stream`（`"stdout"` または `"stderr"`）、`line` | 子プロセスの出力の1行。生のまま渡す代わりにフィールドとして運ばれる。 |
| `exited` | `ts`、`name`、`code`（nullable） | プロセスが終了した。ステータスを返すのではなくシグナルでkillされた場合、`code` は `null`。 |
| `restart_scheduled` | `ts`、`name`、`delay_ms` | クラッシュしたプロセスが `delay_ms` 後に再起動される（上記のバックオフスケジュールを参照）。 |
| `restart_succeeded` | `ts`、`name`、`pid` | スケジュールされた再起動が成功した。プロセスは新しいPIDで再び実行中。 |
| `gave_up` | `ts`、`name`、`tries` | プロセスが `tries` 回連続でクラッシュし（`--restart-tries`）、`serve` がその再試行を停止した。セッションと他のすべてのプロセスは実行を続ける。 |
| `types_regenerated` | `ts`、`artifact`（`"inertia_props"` または `"lang_keys"`）、`count` | `.rs`/`.ftl` の変更を受けて、ファイルウォッチャーがTypeScriptアーティファクトを再生成した。 |
| `shutdown` | `ts` | セッションがシャットダウン中。常に最後の行。 |

たとえば、Viteのクラッシュと再起動は次のようになります:

```json
{"type":"exited","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","code":1}
{"type":"restart_scheduled","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","delay_ms":200}
{"type":"restart_succeeded","ts":"2026-08-18T10:15:23.657-07:00","name":"frontend","pid":48391}
```

`--json` は `--timestamps` と競合せず合成されます。併用はエラーになりませんが、各イベントにすでに `ts` フィールドがあるため、`--timestamps` に追加効果はありません。

これは他のツールが解析する機械可読出力です - フィールド名と `type` の値は、変更履歴に記載せずに名前変更や削除をしません。認識できない `type` や予期しない追加フィールドはエラーではなく無視してください。これにより、将来のリリースでコンシューマーを壊さずにスキーマを拡張できます。

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

### プロセスがクラッシュループし続ける

子プロセス（バックエンド、フロントエンド、または `Suprnova.toml` のエントリ）が起動できない場合（コード不良、バイナリ欠落、ポート競合など）、停止する代わりに上記のバックオフスケジュールで再起動されます。各「respawning in …ms」という通知の直前にある `[name]` の行を見て、実際のエラー（rustcの `error[E…]`、ENOENT、子プロセスが出力したもの）を確認してください。原因を修正すれば、次の再起動試行が自動的にそれを取り込みます。再試行を止めて一度だけ失敗を確認するには `--no-restart` で再実行してください。この場合、`suprnova serve` の従来の動作と同じく、最初のクラッシュでセッションが終了します。

`--restart-tries`（既定 `5`）回連続でクラッシュすると、`serve` はそのプロセスの再試行を自分で停止し、名前を示すメッセージを表示します:

```text
gave up restarting `backend` after 5 attempts; fix the error and run `suprnova serve` again
```

他のプロセスとセッション自体は動き続けます。原因を修正して `suprnova serve` を再実行すれば、諦めたプロセスを戻せます。セッション全体を再起動する必要はありません。

## 次のステップ

- [インストール](installation.md) - CLIをマシンに入れる
- [クイックスタート](quickstart.md) - 最初のアプリを一通り歩く
- [ディレクトリ構成](structure.md) - `suprnova new` がスキャフォルドした内容
- [コード ジェネレーター](cli-generators.md) - `make:controller`、`make:action` など
- [コンソール](console.md) - プロジェクトごとの `cargo run --bin console` バイナリ

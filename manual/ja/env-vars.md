# 環境変数

これは、Suprnovaフレームワークが実行時に読み取るあらゆる環境変数の、参照先のサブシステムごとにグループ化された監査済みリストです。あらゆるエントリはフレームワークのソースに対して検証されています - デフォルト値、型、挙動は、スターターの `.env` がたまたま出荷している内容ではなく、コードが実際に行っていることを反映しています。

このリストは、`suprnova` CLIバイナリ（開発サーバー、SSRワーカー）が読み取る変数も扱います。これらはスターターの `.env` に現れ、読者がここでそれらを探すはずだからです。

読み込みのルール（`.env` → `.env.<environment>` → プロセスenv）、`env*` ヘルパー（`env`、`env_required`、`env_optional`）、型付きの `Config::*` 登録パターンについては、[設定](configuration.md)を参照してください。

## 表記規則

- **デフォルト** - その変数が未設定のときにフレームワークが使う値です。`none` は、デフォルトが存在しないことを意味します。その場合フレームワークは、起動時にエラーになるか、フィーチャーのデフォルト（例: `Memory` ドライバー）にフォールバックするか、あるいはその値を `None` として扱います。
- **型** - その変数がパースされるRustの型です。`bool` の値は、`true`/`false`/`1`/`0`/`yes`/`no`/`on`/`off`（大文字小文字を区別しません）を受け付けます。型付きのフレームワークのノブに対する範囲外の値やパース不能な値は、クランプされる（ワークフロー）か、`warn!` でログに記録されたのちデフォルト値になる（緩やかな `env()` / `env_optional()`）か、あるいは起動が失敗します（厳格な `try_from_env`）。
- **必須** - `boot` は、記載された環境ではそれなしにフレームワークが起動を拒否することを意味します。`driver` は、親のドライバーが選択されているときだけ必須になることを意味します（例えば `MAIL_SES_REGION` は `MAIL_DRIVER=ses` でない限り無関係です）。それ以外はすべて任意です。

スターターの `.env` が、フレームワークが決して読み取らないキー（`MAIL_FROM_ADDRESS`、`FILESYSTEM_DISK`）を出荷している場合は、この章の最後で言及します。

## アプリケーション

`APP_*` ファミリーは、フレームワークのアイデンティティと暗号のルートです。これらは、あらゆるSuprnovaアプリが設定する変数です - このファイルの残りの部分は、サブシステムにオプトインしていくにつれて関係してきます。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `APP_NAME` | `"Suprnova Application"` | `String` | アプリケーション名です。TOTPの発行者名（2FA）、HTTP Basicの `WWW-Authenticate` レルム、メール件名のブランディング、構造化ログのフィールドとして使われます。 |
| `APP_ENV` | `local` | `String` | `Environment::detect()` と `.env.<suffix>` のルックアップを駆動します。認識されるエイリアス（大文字小文字を区別しません）: `local`、`development`/`dev`、`staging`/`stage`/`stg`、`production`/`prod`、`testing`/`test`。それ以外の値は、元の大文字小文字を保ったまま `Environment::Custom(...)` として保持されます。 |
| `APP_DEBUG` | env-aware（「必須」を参照） | `bool` | 詳細なエラーページ + 追加のログです。デフォルトは `local`/`development`/`testing` では `true`、それ以外のすべて（`staging`、`production`、認識されないカスタム環境を含む）では `false` です。明示的な値は常に優先されます。パース不能な値は、`warn!` を伴って環境依存のデフォルトへフォールバックします。厳格な `try_from_env` バリアントは、パースの失敗で起動を中断します。 |
| `APP_URL` | `"http://localhost:8765"`（AppConfig） / `"http://localhost"`（URLフォールバック） | `String` | 絶対URL生成、署名付きURL、InertiaのリダイレクトのためのベースURLです。読み取り時に末尾のスラッシュは取り除かれます。 |
| `APP_KEY` | なし - 非開発環境では必須 | `String`（base64-url-no-pad、32バイト） | `Crypt`、暗号化されたセッション、ページネーションのカーソル、署名付きURL、その他あらゆる保存時暗号化の経路のための、AES-256-GCMキーです。`local`/`development`/`testing` の外で、これが欠けているか不正な形式の場合、起動は**フェイルクローズ**します。`suprnova key:generate` で生成してください。 |
| `APP_KEY_PREVIOUS` | なし | `String`（カンマ区切りのbase64キー、最大8個） | ローテーション中に使われる、カンマ区切りの以前のキーです。`Crypt::decrypt` は、まず現在の `APP_KEY` を試し、その後で各エントリを順番に試します。エントリの数には8個という固定の上限があります - `crypto::MAX_PREVIOUS_KEYS`。デコードに失敗する、ローテーションが中途半端なエントリは起動を中断させます。[暗号化](encryption.md#key-rotation)を参照してください。 |
| `APP_PREVIOUS_KEYS` | なし | `String`（`APP_KEY_PREVIOUS` のエイリアス） | Laravel互換のエイリアスで、LaravelのSuprnovaのデプロイに落とし込まれた `.env` が、レガシーデータをそれでも問題なく復号できるように受け付けられます。両方が異なる値で設定されている場合、重複を明らかにする `warn!` を伴って `APP_KEY_PREVIOUS` が優先されます。同一の値は、無音で受け付けられます。 |
| `APP_BASE_PATH` | 現在の作業ディレクトリ | `Path` | パスリゾルバーが `config/`、`database/`、`public/`、`storage/`、`resources/`、`lang/` のために使うルートディレクトリです。プロジェクトルートとは異なるCWDからバイナリを実行するとき（例えばsystemdユニットで、`WorkingDirectory=` がプロジェクトを指していないとき）に役立ちます。CWDへフォールバックし、CWDが使えない場合はさらに `.` へフォールバックします。 |
| `APP_TRUSTED_PROXIES` | なし - 空の許可リスト | `String`（カンマ区切りのIP） | `Request::ip()` およびホスト / スキーム / ポートのアクセッサーが、その `X-Forwarded-*` / `X-Real-IP` ヘッダーを信用してよいTCPピアのアドレスです。**デフォルトでは空であるため、プロキシヘッダーは無視され、常にTCPピアが優先されます** - プロキシの背後にデプロイする前に、下記の注記を参照してください。パース不能なエントリは起動を失敗させます（`try_from_env`）。 |
| `AUTH_GUARD` | `"web"` | `String` | `Auth::*` が読み取るデフォルトの認証ガードの名前です。Laravelを反映しています - envで選べるのはデフォルトだけで、名前付きの認証ガードは `AuthConfig::guard(name, …)` を介してコードの中に存在します。 |

もう2つの `APP_*` 変数 - `APP_LOCALE` と `APP_FALLBACK_LOCALE` - は、`AppConfig` ではなくローカライゼーションのサブシステムによって読み取られるため、下記の**ローカライゼーション**の下に一覧されています。

### リバースプロキシの背後では `APP_TRUSTED_PROXIES` を設定する

プロキシヘッダーを無視することが安全なデフォルトです - `X-Forwarded-For` は呼び出し元が送ってくるものであり、それを無条件に信用すると、誰でも任意のアドレスを名乗れてしまいます。しかし、終端プロキシ（nginx、Traefik、ALB、Cloudflareなど）が手前に立った瞬間、あらゆるリクエストにおいてTCPピアは*そのプロキシ*になり、これを未設定のままにしておくことは、単にクライアントのアドレスを失うだけでは済みません。

- **IPごとのレートリミットが、1つのバケットへ潰れます。** `ThrottleRequestsMiddleware` のデフォルトのキーは `request.ip()` であるため、`ThrottleRequestsMiddleware::with(20, 1, "login")` は「クライアントごとに1分間に20回のログイン試行」を意味しなくなり、*全員を合わせて*20回を意味するようになります。これは弱いだけでなく（攻撃者ごとの予算がありません）、実際に危険です。単一の呼び出し元が枠を使い切り、正当なあらゆるユーザーをログインフォームから締め出せてしまいます。[レート リミット](rate-limiting.md)を参照してください。
- `Request::host()`、`scheme()`、`port()` は、`X-Forwarded-Host` / `-Proto` / `-Port` ではなく、コネクションへフォールバックします。そのため、生成される絶対URLが、公開されているアドレスやスキームではなく、内部のものを指してしまうことがあります。

プロキシのホップがあなたに届くアドレスを一覧してください - クライアントのアドレスではありません。

```bash
APP_TRUSTED_PROXIES=10.0.0.5,10.0.0.6
```

これを検出してくれるものは何もありません。変数が未設定のままプロキシの背後にあるアプリは、健全に見え、正しく応答しているように見えながら、静かに全員を1人のユーザーとしてレート制限してしまいます。

### `APP_KEY` 必須マトリクス

| 環境 | 起動時に `APP_KEY` が必須か |
|---|---|
| `local` | いいえ（欠けている場合、一時的なキーを生成します） |
| `development` | いいえ |
| `testing` | いいえ |
| `staging` | はい - 対処方法を示すメッセージと共に、起動が非ゼロで終了します |
| `production` | はい |
| `Custom(...)` | はい - このチェックにおいて、セーフリストにないものはすべて本番環境として扱われます |

## サーバー

HTTPリスナーとリクエストボディの上限です。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `SERVER_HOST` | `"127.0.0.1"` | `String` | バインドアドレスです。ループバックインターフェースの外へ公開するには（例えばコンテナの中で）、`0.0.0.0` を設定してください。 |
| `SERVER_PORT` | `8765` | `u16` | バインドポートです。緩やかなパースは警告してデフォルト値になり、厳格な `try_from_env` はタイプミスで起動を中断します。 |
| `SERVER_MAX_BODY_SIZE` | `8388608`（8 MiB） | `usize`（バイト） | プロセス全体のリクエストボディの最大サイズです。個々のエンドポイントに対する `FormRequest::max_body_bytes` ごとの上書きは、それでも適用されます。設定された値は、`Server::from_config` の間にグローバルな上限へ配線されます。 |
| `SERVER_MAX_CONNECTIONS` | 未設定（無制限） | `usize` | 同時にアクティブなTCPコネクションの上限です。未設定は上限なしを意味します。ゼロまたはパース不能な値は、無音で無制限に戻るのではなく、警告を伴って有限の `10000` へフォールバックします - 失敗した上限も、それでも上限を求めるリクエストであることに変わりはありません。 |
| `SERVER_HEADER_READ_TIMEOUT` | `30` | `u64`（秒） | リクエストの完全なヘッドを読み終えるまでの締め切りです。slowloris対策です。ゼロは「無効化」ではなく不正な値として扱われ、デフォルトへフォールバックします。確立済みのWebSocket/SSEコネクションには適用されません。 |
| `SERVER_HEALTH_READINESS_TOKEN` | 未設定（レディネスは公開されます） | `String` | `/_suprnova/health/ready` と `/_suprnova/health?db=true` に到達するために必要な共有シークレットで、`X-Suprnova-Health-Token` として送信されます。これがなければ、これらのパスはルーティングされていないパスと見分けがつかない404を返します - ライブネスは公開されたままです。[デプロイメント](deployment.md#health-check)を参照してください。 |

## データベース

コネクションURLと、sqlxプールのチューニングです。`DATABASE_URL` は、データベースに触れるあらゆるサブコマンド（`migrate*`、`db:sync`、`db:seed`、`QUEUE_DRIVER=database` の `queue:work`、`workflow:work`、セッションのDBストア）に対して、そしてアプリにマイグレーションが登録されている場合の `serve` に対して必須です。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `DATABASE_URL` | なし - マイグレーションが存在する場合は必須 | `String` | コネクションURLです。スキームがドライバーを選びます: `sqlite://path`、`postgres://...` / `postgresql://...`、`mysql://...`、`mariadb://...`。フレームワークは、SQLiteのパスに対する親ディレクトリを自動的に作成します。設定済みの `Migrator` にマイグレーションがない場合、`serve` はデータベースへの接続を完全にスキップします。 |
| `DB_MAX_CONNECTIONS` | `10` | `u32` | sqlxプールの上限です。 |
| `DB_MIN_CONNECTIONS` | `1` | `u32` | sqlxプールの下限です（ウォームに保たれます）。 |
| `DB_CONNECT_TIMEOUT` | `30`（秒） | `u32` | エラーになるまでに、sqlxが最初の接続をどれだけ待つかです。 |
| `DB_LOGGING` | `false` | `bool` | trueのとき、sqlxはあらゆるステートメントをログに記録します（本番環境では控えめに使ってください - うるさいです）。 |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT` | `false` | `bool` | trueのとき、`serve` の起動中に自動マイグレーションが失敗してもログに記録されるだけで、中断はしません。デフォルトはフェイルクローズです - 部分的にしかマイグレーションされていないスキーマに対して起動するのではなく、非ゼロで終了します。自動マイグレーションを完全にスキップするには `--no-migrate` を渡してください。 |

## セッション

セッションサブシステムのためのクッキー属性と有効期間です。`SESSION_SECURE` はデフォルトで**`true`**であることに注意してください - デフォルトで本番環境に対して安全であり、ローカルのHTTP開発のときだけこれをオフにしてください。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `SESSION_LIFETIME` | `120`（分） | `u64` | 分単位のセッション有効期間です。`env_optional` を介してパースされ、パース不能な場合は無音でフォールバックします。 |
| `SESSION_TOUCH_INTERVAL` | `300`（秒） | `u64` | スライディング有効期限の永続化を行う最小の周期です。ランタイムの強制により、セッション有効期間の半分を上限とします。 |
| `SESSION_GC_INTERVAL` | `3600`（秒） | `u64` | `SessionMiddleware::install` によってインストールされる、監視付きの期限切れセッションコレクターの周期です。 |
| `SESSION_COOKIE` | `"suprnova_session"` | `String` | セッションクッキーの名前です。 |
| `SESSION_PATH` | `"/"` | `String` | クッキーの `Path=` 属性です。 |
| `SESSION_DOMAIN` | 未設定 | `String` | クッキーの `Domain=` 属性です。ホスト限定のクッキーにするには未設定のままにしてください（ほとんどのアプリにとって、より安全なデフォルトです）。 |
| `SESSION_SECURE` | `true` | `bool` | クッキーの `Secure` 属性です。デフォルトは `true` で、ローカルのHTTP開発のときだけ `false` にしてください。`cookie_http_only` は常に `true` であり、envでは設定できません。 |
| `SESSION_SAME_SITE` | `"Lax"` | `String` | `SameSite` 属性です。`Strict`、`Lax`、`None`（大文字小文字を区別しません）を受け付けます。 |
| `SESSION_COOKIE_PREFIX` | 未設定 | `String`（`__Host-` / `__Secure-`） | セッションおよびremember-meのワイヤ名に適用するプレフィックスです。`Config::init` は起動時に、この値と `SESSION_DOMAIN` / `SESSION_PATH` の制約を検証します。不正な組み合わせでは提供を開始する前に失敗します。 |
| `SESSION_PARTITIONED` | `false` | `bool` | サードパーティから分離されたクッキーのために、`Partitioned` / CHIPSのクッキー属性を出力します。 |
| `SESSION_EXPIRE_ON_CLOSE` | `false` | `bool` | trueのとき、`Max-Age` を落とすことで、ブラウザが閉じたときにクッキーを削除するようにします（セッションクッキーの挙動です）。 |
| `SESSION_CONNECTION` | 未設定 | `String` | セッションストアのための、名前付きDBコネクションです。未設定はデフォルトのコネクションを意味します。 |
| `REMEMBER_LIFETIME` | `43200`（30日間、分単位） | `u64` | 「ログイン状態を保持する」クッキー / トークンの、分単位の有効期間です。 |

## ローカライゼーション

ローカライゼーションのサブシステムが読み取る、3つの `APP_*` 変数です。それ以外のすべて - 検出チェーン、参照するセッションキーとクッキー名、Unicodeの分離マーク - は、envではなく `LocalizationConfig` 上のコードレベルの設定です。[ローカライゼーション](localization.md)を参照してください。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `APP_LOCALE` | `"en"` | `String`（BCP-47） | 検出チェーン（セッション → クッキー → `Accept-Language`）が何も見つけられなかったときに使われるロケールです。`lang-keys.ts` のために `suprnova generate-types` がメッセージキーを抜き出す元になるロケールでもあります。有効なBCP-47の識別子ではない値は、無音でデフォルトになるのではなく起動を失敗させます。 |
| `APP_FALLBACK_LOCALE` | `"en"` | `String`（BCP-47） | 現在のロケールのカタログにキーが欠けているときに参照されるロケールです。両方から欠けているキーは、キー自体と1回限りの `warn!` としてレンダリングされます - `Lang::try_get` は代わりに `Err` を返します。`APP_LOCALE` と同じ厳格なパースです。 |
| `APP_LOCALE_PARENTS` | なし - 空のマップ | `String`（カンマ区切りの `child=parent` のペア。両側ともBCP-47） | `APP_FALLBACK_LOCALE` より前に参照される、ロケールごとのフォールバック親です（例: `APP_LOCALE_PARENTS=pt-PT=pt-BR,en-AU=en-GB`）。`Lang` のフォールバックチェーンはこれらを推移的にたどり、`FluentTranslator` は各ロケールの設定済みの親チェーンを、それが提供するカタログへ平坦化します。不正な形のペア、無効なロケール、複数回名指しされた子、あるいは循環（あるロケールが自分自身を親として名指しする場合を含みます）は、リクエスト時に劣化するのではなく起動を失敗させます。[フォールバックチェーン](localization.md#fallback-chains)を参照してください。 |

カタログ自体は、envではなくファイルです - `APP_BASE_PATH` の下の `lang/<locale>/*.ftl` です。`lang/` ディレクトリが存在しないことはエラーではありません - アプリは、フレームワークに組み込まれた英語のバリデーションカタログで起動します。

## キャッシュ

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `CACHE_DRIVER` | `memory` | `String`（`memory`/`in-memory`/`inmemory`、`redis`） | ブートストラップの対象を選びます。Memoryはすべてをプロセス内に保ち、Redisは `REDIS_URL` を要求し、到達できない場合は起動を失敗させます。未知の値は、明確なエラーを伴って起動を失敗させます。 |
| `REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | Redisのコネクション URL です（`CACHE_DRIVER=redis` のときだけ参照されます）。 |
| `REDIS_PREFIX` | `"suprnova_cache:"` | `String` | キャッシュエントリのためのキー接頭辞です（共有されたRedisにおける衝突回避のためです）。 |
| `CACHE_DEFAULT_TTL` | `3600`（秒） | `u64` | 秒単位のデフォルトTTLです。`0` は「無期限」を意味します。`Cache::put(None)` / `Cache::tags_put(None)` に適用されます - `Cache::forever` と `Cache::remember_forever` は常にこれを回避します。 |

## キュー

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `QUEUE_DRIVER` | `memory` | `String`（`memory`、`redis`、`database`、`failover`） | 有効なキューのバックエンドです。未知の値は `warn!` を記録し、memoryへフォールバックします。`failover` はほかのものの順序付きリストをラップします - `QUEUE_FAILOVER_CONNECTIONS` を参照してください。 |
| `QUEUE_FAILOVER_CONNECTIONS` | - | `String`（カンマ区切り。例: `redis,database`） | `QUEUE_DRIVER=failover` のための、優先順位付きの接続リストです。そのドライバーが選ばれているときは必須で、値が欠けているか空白であれば起動エラーになります。`failover` を指すエントリ（ネストは不可）や、存在しないドライバーを指すエントリも同様です。各エントリは、それ自身のドライバーの変数を読みます。リストを落ちていくのはプッシュだけです。すべての読み取りとすべての確認応答は最初の接続へ向かうため、それぞれのフォールバックには自身のワーカーが必要です。 |
| `QUEUE_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | RedisのURLです（`QUEUE_DRIVER=redis` のときはドライバーが要求します）。 |
| `QUEUE_REDIS_STREAM` | `"suprnova-queue"` | `String` | ファンアウトに使うRedis Streamのキーです。 |
| `QUEUE_REDIS_GROUP` | `"default"` | `String` | コンシューマーグループの名前です。 |
| `QUEUE_REDIS_CONSUMER` | `"consumer-1"` | `String` | グループ内のコンシューマー名です。並列のワーカーのためには、ワーカーごとに設定してください。 |
| `QUEUE_VISIBILITY_TIMEOUT_SECS` | `60` | `u64` | 要求されたジョブが、別のコンシューマーによって再要求され得るようになるまで、どれだけ不可視のままでいるかです。あなたのいちばん遅いジョブに合わせてください。 |
| `QUEUE_DB_TABLE` | `"jobs"` | `String` | databaseドライバーのためのテーブル名です。SQLの識別子として検証されます - 不正な値は、SQLの組み立て時ではなく起動時に失敗します。`QUEUE_DRIVER=database` のときはドライバーが要求し、そのドライバーはさらに `DB::init()` が先に走っていることも要求します。 |
| `QUEUE_FAILED_DB_TABLE` | `"failed_jobs"` | `String` | デッドレターストアが書き込む先のテーブルです。`QUEUE_DRIVER=database` のときに自動的にバインドされます - `queue:retry` がこれを読み、`Queue::retry_failed` がこれを必要とするため、このテーブルはそのドライバーの契約の一部です。`memory`（構造上、揮発性です）や `redis`（書き込む先のテーブルがありません）では使われません。`QUEUE_DB_TABLE` とは違い、ここでの不正な識別子は起動を失敗**させません**: `error!` に記録し、ストアをバインドしないまま残すため、デッドレターに送られたジョブは永続化されるのではなく、そのすべてがログに記録されます。手作業では復旧できますが、`queue:retry` では復旧できません。 |

## スケジュール

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION` | 未設定 | `bool` 相当 | `on_one_server()` でマークされたタスクが、**プロセスごとの**キャッシュを通じてリーダーを選出していることを認めるものです。その選出は、裏にあるキャッシュが共有されている範囲でしか共有されないため、本番環境で `CACHE_DRIVER=memory` と単一サーバー限定のタスクを組み合わせると、「すべてのレプリカがそれを実行する」への静かな格下げではなく、問題のタスクを名指しするハードな起動失敗になります。デプロイメントが本当に1つのスケジューラーだけを動かしている場合にのみこれを設定してください - そうでなければ `CACHE_DRIVER=redis` を設定してください。[スケジューリング](scheduling.md)を参照してください。 |

## ワークフロー

`#[workflow]` の長時間実行されるステートフルなワーカーです。すべての値は、盲目的に尊重されるのではなく、安全な最小値にクランプされます - `WORKFLOW_CONCURRENCY=0` は、ワーカーのセマフォを永久に停止させてしまうため、フレームワークは明らかに壊れた設定を受け入れるのではなく、警告してクランプします。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `WORKFLOW_CONCURRENCY` | `4` | `usize` | ワーカープロセスごとの、同時実行されるワークフローの最大数です。`>= 1` にクランプされます。 |
| `WORKFLOW_POLL_INTERVAL_MS` | `1000`（ms） | `u64` | ワーカーが、新たに期限を迎えたワークフローをどれくらいの頻度でポーリングするかです。 |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30`（秒） | `u64` | ワーカーが死んだ、クレームされたワークフロー行の再クレームタイムアウトです。 |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | `i32` | 失敗としてマークされるまでの、ワークフロー実行ごとの最大試行回数です。`>= 1` にクランプされます。 |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | `i64` | 試行ごとの線形バックオフです。`>= 0` にクランプされます - 負のバックオフは、リトライを過去にスケジュールし、タイトループの再クレームを引き起こしてしまいます。 |

## メール

`MAIL_DRIVER` はデフォルトで**`log`**です - 送信メールは、ネットワークに到達するのではなく、設定済みのtracingサブスクライバーへ出力されます。テストでは `memory` に、メールクライアントで開ける `.eml` プレビューには `file` に、本番環境では `smtp`/`ses` などに切り替えてください。プロバイダー固有のキー/トークンは、そのドライバーが選択されているときだけ必須になります - 未知のドライバー値は `warn!` をログに記録し、`log` へフォールバックします。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `MAIL_DRIVER` | `"log"` | `String`（`log`、`memory`、`file`、`smtp`、`ses`、`sendgrid`、`mailgun`、`postmark`、`resend`） | ブートストラップの対象を選びます。 |
| `MAIL_FROM` | なし - 認証フローのファサードにより必須 | `String` | 認証フローのファサード（`EmailVerification`、`PasswordReset`、`TwoFactor`）のためのデフォルトのfromアドレスです。これらの経路では必須です - なければ、DMARC/SPFを壊してしまうプレースホルダーへ無音でフォールバックするのではなく、呼び出し箇所でエラーになります。 |
| `MAIL_FROM_NAME` | 未設定 | `String` | 認証フローの `From` のための、任意の表示名です（**0.5.9**以降）。設定されている場合、ヘッダーは `Name <MAIL_FROM>` としてレンダリングされます - `MAIL_FROM` は、そのまま裸のアドレスです。送信時に読み取られるため、キューに入れられた認証フローのメールにも適用されます。 |

### File（`MAIL_DRIVER=file`）

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `MAIL_FILE_PATH` | `storage_path("mail")` | `String` | 送信ごとに1つのRFC 5322 `.eml` ファイルを書き込むディレクトリです。削除は行われません。絶対パスはそのまま使われ、相対パスはアプリケーションのベースディレクトリ（`APP_BASE_PATH` を参照）を起点にします。 |

### SMTP（`MAIL_DRIVER=smtp`）

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `MAIL_SMTP_HOST` | `"127.0.0.1"` | `String` | SMTPホストです。 |
| `MAIL_SMTP_PORT` | `587` | `u16` | SMTPポートです。 |
| `MAIL_SMTP_USER` | 未設定 | `String` | SMTPユーザー名です。暗号化されたトランスポートにするには、`MAIL_SMTP_USER` と `MAIL_SMTP_PASS` の**両方**を設定する必要があります - どちらも設定しない場合、コネクションはデフォルトで暗号化されないローカルキャッチャーのモードになります。どちらか一方だけを設定すると、起動時に警告が出ます。 |
| `MAIL_SMTP_PASS` | 未設定 | `String` | SMTPパスワードです。部分的な認証情報の挙動については `MAIL_SMTP_USER` を参照してください。 |
| `MAIL_SMTP_ENCRYPTION` | 導出される | `starttls` \| `tls` \| `none` | コネクションがどのように暗号化されるかです。未設定の場合、資格情報から導出されます - 両方が設定されていれば `starttls`、どちらも設定されていなければ `none` です。`tls` は暗黙のTLS（ポート465）を選びます。`ssl` と `null` は、Laravel互換のエイリアスとして受け付けられます。認識されない値は、**あらゆる**環境で起動を失敗させます - タイプミスが平文へ格下げされてはならないからです。 |
| `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION` | 未設定 | `bool` 相当 | 本番環境は、暗号化されていないSMTPコネクションでは起動を拒否します。平文を承知の上で使うには `1`/`true`/`yes`/`on` を設定してください - リレーがプライベートネットワーク経由でのみ到達可能な場合にだけ、正当化できます。 |

### Postmark（`MAIL_DRIVER=postmark`）

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `MAIL_POSTMARK_TOKEN` | ドライバーにより必須 | `String` | Postmarkのサーバートークンです。 |
| `MAIL_POSTMARK_ENDPOINT` | Postmarkのデフォルト | `String` | APIエンドポイントを上書きします（リージョンごとのエンドポイントやモックサーバー向けです）。 |

### Amazon SES（`MAIL_DRIVER=ses`）

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `MAIL_SES_ACCESS_KEY` | ドライバーにより必須 | `String` | AWSのアクセスキーです。 |
| `MAIL_SES_SECRET_KEY` | ドライバーにより必須 | `String` | AWSのシークレットキーです。 |
| `MAIL_SES_REGION` | `"us-east-1"` | `String` | AWSのリージョンです。 |
| `MAIL_SES_ENDPOINT` | そのリージョンのAWSデフォルト | `String` | SESのエンドポイントを上書きします（リージョンごとのエンドポイントやモックサーバー向けです）。 |

### SendGrid（`MAIL_DRIVER=sendgrid`）

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `MAIL_SENDGRID_API_KEY` | ドライバーにより必須 | `String` | SendGridのAPIキーです。 |
| `MAIL_SENDGRID_ENDPOINT` | SendGridのデフォルト | `String` | APIエンドポイントを上書きします。 |

### Mailgun（`MAIL_DRIVER=mailgun`）

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `MAIL_MAILGUN_API_KEY` | ドライバーにより必須 | `String` | MailgunのAPIキーです。 |
| `MAIL_MAILGUN_DOMAIN` | ドライバーにより必須 | `String` | Mailgunの送信ドメインです。 |
| `MAIL_MAILGUN_ENDPOINT` | Mailgunのデフォルト | `String` | APIエンドポイントを上書きします（例えばEUかUSかです）。 |

### Resend（`MAIL_DRIVER=resend`）

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `MAIL_RESEND_API_KEY` | ドライバーにより必須 | `String` | ResendのAPIキーです。 |
| `MAIL_RESEND_ENDPOINT` | Resendのデフォルト | `String` | APIエンドポイントを上書きします。 |

## レート リミット

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `RATE_LIMIT_DRIVER` | `memory` | `String`（`memory`、`redis`） | レートリミッターのバックエンドを選びます。本番環境の外では、未知の値は `warn!` をログに記録し、memoryへフォールバックします - **本番環境では、`RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` が設定されていない限り、memory（未知の値を経由するものも含みます）は起動を失敗させます**。 |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | 未設定 | `bool` 相当 | 本番環境における、プロセスごとのレートリミットバケットを認めるものです。これが正確なのは、実行しているプロセスがちょうど1つの場合だけです - Nレプリカの背後では、あらゆる枠が実質的にN倍になり、デプロイごとにリセットされます。 |
| `RATE_LIMIT_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | Redis URLです（`RATE_LIMIT_DRIVER=redis` のときはドライバーにより必須です）。 |
| `RATE_LIMIT_PREFIX` | `"suprnova:"` | `String` | Redisにおけるキー接頭辞です。 |

## 画像

画像ドライバーの選択と、敵対的な入力を境界付けるデコードの上限です。範囲外の上限は、起動を失敗させるのではなく `warn!` を伴ってクランプされます。上限が0であれば、アプリケーション内のあらゆる画像を拒否してしまうからです。未知の `IMAGE_DRIVER` は、最初の使用時に有効な値を挙げて失敗します。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `IMAGE_DRIVER` | `oxideav` | `String`（`oxideav`、`magick`） | 画像のバックエンドを選びます。`oxideav` はホスト側の依存関係を持たない純粋なRustです。`magick` は、より広い入力サポートのために、ホストにインストールされたImageMagick 7へシェルアウトします。大文字小文字を区別しません。 |
| `IMAGE_MAX_DIMENSION` | `16384` | `u32` | デコードされた画像の幅と高さの上限で、何かが割り当てられる前に、入力自身のヘッダーに対して検査されます。リサイズの目標値にも上限をかけます。最小値は `1` です。 |
| `IMAGE_MAX_ALLOC_BYTES` | `268435456`（256 MiB） | `u64` | デコード後のRGBAのフットプリント（`width * height * 4`）の上限です。ソースファイル自体のサイズにも上限をかけます - パスから来ても、ディスクから来ても、`Image::from_stream`（収集しながら検査します）から来ても同じです。最小値は `4` です。 |
| `IMAGE_MAGICK_BINARY` | `magick` | `String` | `magick` ドライバーが起動するバイナリです。ImageMagick 7のみで、ImageMagick 6の `convert` という名前は受け付けません。バイナリが見つからない場合は、最初の使用時に明確なエラーになります。 |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | `u32` | 1回のImageMagickの起動に対する実時間の上限です。これはImageMagick自身の `-limit time` 引数であると同時に、その2秒後に子プロセスのプロセスグループ全体をkillするRust側の期限でもあります。`-limit time` を強制するのはモニターですが、デリゲートの内側で動かなくなった子プロセスは、そのモニターを決して起動させないからです。放置すればプロセスの寿命のあいだブロッキングワーカーを占有してしまう、停止したデリゲートを境界付けます。`magick` ドライバーのみ。最小値は `1` です。 |

2段構えの上限の強制と、ドライバー間の選び方については、[画像](images.md)を参照してください。

## ハッシング

パスワードハッシュ化のドライバーと、アルゴリズムごとのパラメータです。不正な値は、最初のハッシュ化の時点で `FrameworkError::param` を返し、無音でデフォルトになるのではなく、設定ミスを即座に明らかにします。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `HASH_DRIVER` | `bcrypt` | `String`（`bcrypt`、`argon`/`argon2i`、`argon2id`） | アクティブなハッシュ化アルゴリズムです。大文字小文字を区別しません。 |
| `HASH_ROUNDS` | `12` | `u32` | Bcryptのコストです（範囲は `4..=31`）。範囲外の値は、明確なエラーで失敗します。 |
| `HASH_MEMORY` | `65536`（64 MiB、KiB単位） | `u32` | KiB単位のArgon2のメモリです。最小は `8` です。Argon専用です。 |
| `HASH_TIME` | `4` | `u32` | Argon2の時間 / 反復回数です。最小は `1` です。Argon専用です。 |
| `HASH_THREADS` | `1` | `u32` | Argon2の並列度です（OWASP / libsodiumに一致します）。最小は `1` です。Argon専用です。 |
| `HASH_VERIFY` | `false` | `bool` | trueのとき、`verify()` は `HASH_DRIVER` とは異なるアルゴリズムからのハッシュを拒否します（`Ok(false)` を返します）。デフォルトは `false` であり、ドライバーを切り替えた後も、ローテーションされるまでレガシーのbcryptハッシュがそれでも検証できるようにしています。 |

## バリデーション

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `HIBP_TIMEOUT_SECS` | `30`（秒） | `u64` | `Password::uncompromised()` の Have I Been Pwned のレンジチェックに対するリクエストのタイムアウトで、デフォルトの `HibpVerifier` が構築されるたびに読み直されます。HIBPが遅い、あるいは到達できない場合も、依然としてフェイルオープンします - [バリデーション](validation.md)を参照してください。 |

## 認証フロー

二要素認証は、TOTPの発行者文字列として `APP_NAME`（アプリケーションの下で扱っています）を使います - 専用の `2FA_ISSUER` 環境変数はありません。発行者は、`APP_NAME` が未設定のとき `"Suprnova"` へフォールバックします。

## Inertia / フロントエンド

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `SUPRNOVA_FRONTEND` | `svelte` | `String`（`svelte`、`react`、`vue`） | アクティブなフロントエンドです。大文字小文字を区別しません。`Frontend::detect_from_env()`、デフォルトのViteのエントリポイント、そしてコンパイル時のページコンポーネントの拡張子の探索順序を駆動します。未知または未設定の値は `svelte` へフォールバックします。 |

## メンテナンスモード

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `MAINTENANCE_DRIVER` | `file` | `String`（`file`、`cache`） | `down`/`up` の状態がどのように保存されるかを選びます。`file` はフレームワークのストレージパスへ書き込み、`cache` は設定済みのキャッシュドライバーに乗ります（多数のアプリインスタンスがメンテナンス状態を協調させる必要がある場合に便利です）。それ以外の値は `file` へフォールバックします。 |

## イベント

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `EVENT_MAX_CONCURRENCY` | `256` | `usize` | 同時に実行されるキュー入りリスナータスクの上限です。`<= 0` またはパース不能な値はデフォルトへフォールバックします。`Event::queue` / キュー入りのリスナーに適用されます - 同期的なリスナーはこの上限の対象ではありません。 |

## ロギング

`LOG_FORMAT` は**環境を意識します**。本番環境（`APP_ENV=production`）では、ログアグリゲータとの親和性のためにデフォルトは `json` です - それ以外のすべてでは、人間に読みやすいローカル/開発の出力のためにデフォルトは `pretty` です。明示的な値は常に優先されます。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `LOG_LEVEL` | `"info"` | `String`（`error`、`warn`、`info`、`debug`、`trace` - 大文字小文字を区別しません） | tracing-subscriberのフィルターレベルです。 |
| `LOG_FORMAT` | 環境を意識する（本番環境では `json`、それ以外では `pretty`） | `String`（`json`、`pretty`） | tracing-subscriberの出力形式です。 |

## 可観測性（OpenTelemetry）

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | 未設定（テレメトリは無効） | `String` | OTLPコレクターのエンドポイントです。未設定（または空白）の場合、エクスポーターはインストールされず、フレームワークは標準の `tracing` サブスクライバーを使い続けます。 |
| `OTEL_SERVICE_NAME` | `"suprnova"` | `String` | あらゆるスパン / メトリクス / ログレコードにおける `service.name` リソース属性です。 |
| `OTEL_SERVICE_VERSION` | ビルド時の `CARGO_PKG_VERSION` | `String` | `service.version` リソース属性です。 |
| `OTEL_SDK_DISABLED` | `false` | `bool` | 標準的なOTelのキルスイッチです。trueのとき、`OTEL_EXPORTER_OTLP_ENDPOINT` にかかわらずエクスポーターはインストールされません。 |

## CLI / 開発サーバー

これらは、ランタイムフレームワークではなく `suprnova` CLIバイナリ（開発サーバー、SSRワーカー）によって読み取られます - スターターの `.env` に現れるか、`suprnova serve` / `suprnova ssr:*` によって尊重されます。

| 変数 | デフォルト | 型 | 用途 |
|---|---|---|---|
| `VITE_PORT` | `5765` | `u16` | `suprnova serve` の中でViteがバインドするポートです。CLIの `--frontend-port` が上書きします。 |
| `SUPRNOVA_SSR_RUNTIME` | `"node"` | `String` | SSRワーカーを起動するランタイムです（`suprnova ssr:start`）。CLIの `--runtime` が上書きします。 |
| `SUPRNOVA_SSR_BUNDLE` | `frontend/bootstrap/ssr/ssr.js` | `Path` | ビルドされたSSRバンドルへのパスです。CLIの `--bundle` が上書きします。 |
| `SUPRNOVA_SSR_URL` | `"http://127.0.0.1:13714"` | `String` | `suprnova ssr:check` のためのSSRワーカーのURLです。CLIの `--url` が上書きします。 |

## 環境変数を持たないサブシステム

いくつかのサブシステムは、コンテナやサービス登録を介してRustのコードの中で完全に設定されます - フレームワークが読み取る環境変数は**ゼロ**です。

- **ファイルシステム / ストレージ。** ディスクは、`bootstrap()` の中の `FilesystemRegistry::add_disk(name, driver)` で登録されます。`FILESYSTEM_DISK` という環境変数はありません（この名前は一部のスターター `.env` ファイルに現れますが、フレームワークによって参照されることはありません - 下記の「フレームワークが読み取らない変数」を参照してください）。
- **ブロードキャストとWebSocket。** チャネルは、`ws!()` マクロと、コードの中の `BroadcastHub` の設定で登録されます。ドライバー自体は、設定済みの `CACHE_DRIVER` が選ぶものに乗ります。
- **CORS、CSRF、べき等性、タイムアウト。** `bootstrap()` の中のミドルウェアコンストラクタへ渡される、ビルダー構造体を介して設定されます。デフォルトは十分に保守的であるため、典型的なアプリはこれらに触れることがありません。
- **MagnetarとOAuth。** `MagnetarConfig` はアプリケーションbootstrapで構築されます。APIスターターは `PASSKEY_RP_ID` と `PASSKEY_RP_ORIGIN` を読み取りますが、フレームワーク自体は読み取りません。OAuthプロバイダーのID、シークレット、コールバックURL、スコープ、トランスポート、ポリシー値は、Magnetarのプロバイダーレジストリを介してプログラムから供給されます。アプリケーションは、これらの値を環境変数またはシークレットマネージャーから取得できます。
- **ベクトル検索、通知、決済、フィーチャーフラグ。** それぞれ、`bootstrap()` の中の `App::bind` を介して具体的なドライバーを登録します。ドライバーはRustの中で選んでください - それが必要とするURLやキーは、あなた自身の環境変数として渡してください。

## フレームワークが読み取らない変数

スキャフォルドされたスターターの `.env` は、フレームワークが決して参照しない、人間の作者の便宜のためのキーをいくつか一覧しています。それらを探している読者が困惑しないよう、ここで文書化しておきます。

- `MAIL_FROM_ADDRESS` - フレームワークが決して参照しない、Laravel風のプレースホルダーです。認証フローのファサードが実際に使うfromアドレスは `MAIL_FROM` です（メールの下で扱っています）。Laravelの名前を残したい場合、あなた自身の `Mailable` 型は `env_optional` を介してこれを読み取れますが、`suprnova::*` の中の何もこれを読み取りません。（`MAIL_FROM_NAME` は0.5.9以降**読み取られます** - メールの章を参照してください - そのため、もうここには一覧されていません。）
- `FILESYSTEM_DISK` - デフォルトのディスク名のためのプレースホルダーです。代わりに、コードの中で `FilesystemRegistry::set_default(name)` を介してデフォルトを設定してください。

## 値がどのようにパースされるか

3つのenvヘルパーのバリアントについての短いリファレンスです - 完全な解説については[設定](configuration.md#direct-env-access)を参照してください。

| ヘルパー | 欠けている場合の挙動 | パース不能な場合の挙動 |
|---|---|---|
| `env(key, default)` | `default` を返す | `warn!` に加えて `default` を返す |
| `env_required(key)` | **パニックする** | **パニックする** |
| `env_optional(key)` | `None` を返す | `warn!` に加えて `None` を返す |
| `env_strict(key)`（内部用、`try_from_env` が使用） | `Ok(None)` を返す | `Err(FrameworkError)` を返す - 起動が中断する |

厳格なバリアント（`AppConfig::try_from_env`、`ServerConfig::try_from_env`）は、`Config::init` が呼ぶものです。そのため `APP_DEBUG=tru` や `SERVER_PORT=80a0` のようなタイプミスは、無音でデフォルトに戻るのではなく、構造化されたエラーを伴って起動を中断させます。緩やかなバリアントは、パースの失敗がパニックしてはならない、より広い呼び出し箇所の集団（`impl Default` を含みます）のために存在します。

## 環境ごとの上書き

ローダーは、次の順序でファイルを読み込み、それぞれが前のものを上書きします。

1. `.env`
2. `.env.<environment>`（例: `.env.production`、`.env.staging`、`.env.testing`、`APP_ENV=<custom>` に対する `.env.<custom>`）
3. プロセス環境

つまり、コンテナ化された本番環境のデプロイは、`.env` と異なるキー（ドライバー名、URL、鍵材料）だけを上書きする最小限の `.env.production` を出荷でき、コミットされたファイルに決して残してはならないシークレットについては、実際のコンテナのenvが両方を上書きします。

正確なローダーの挙動と、リロードをまたいで古い `.env` の値が「実際のシステムenv」の階層へ昇格することを防ぐ `LOADED_KEYS` の追跡については、[設定](configuration.md#how-env-loading-works)を参照してください。

## 次のステップ

- [設定](configuration.md) - 型付きの `Config::*` 登録、`env*` ヘルパー、環境検出
- [デプロイメント](deployment.md) - 本番環境で設定すべきこと
- [暗号化](encryption.md) - `APP_KEY_PREVIOUS` を介した `APP_KEY` のローテーション
- [アプリケーション ブートストラップ](bootstrap.md) - env駆動の起動順序がどこで確立されるか

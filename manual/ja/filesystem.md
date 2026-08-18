# ファイルシステムとストレージ

Suprnovaのストレージファサードは、ローカルファイルシステム、インメモリのバックエンド、そして主要なオブジェクトストア（S3、Azure Blob、Google Cloud Storage）に対して、単一の名前付きディスクAPIを提供します。内部的には[`opendal`](https://docs.rs/opendal)の上に構築されていますが、利用者向けの表面はLaravelの`Storage::disk(...)`呼び出しに合わせて形作られているため、PHPでの体に馴染んだ書き方がそのまま通用します。

```rust,no_run
use suprnova::{DiskExt, Storage};

# async fn doc() -> Result<(), suprnova::FrameworkError> {
Storage::register_fs("local", "./storage")?;
let disk = Storage::disk("local")?;

disk.put("notes/hello.txt", b"hello world".to_vec()).await?;
let bytes = disk.get("notes/hello.txt").await?;
assert_eq!(bytes, b"hello world");
# Ok(())
# }
```

## ディスクの登録

すべてのディスクは、起動時に`Storage::register_*`を通じて一度だけ登録され、`Storage::disk(name)`によって名前で検索されます。他のものがフォールバックする先となる「デフォルトのバックエンド」は存在せず、各ドライバーは対等です。

| コンストラクタ                          | バックエンド                       | フィーチャー             |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | ローカルファイルシステム              | `filesystem`        |
| `Storage::register_memory(name)`     | プロセス内メモリ（テスト用）     | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3、またはS3互換    | `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage          | `filesystem-gcs`    |

`filesystem`はデフォルトで有効になっていますが、AzureとGCSのフィーチャーはそうではありません。`Cargo.toml`でいずれかを有効にしてください。

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4", features = ["filesystem-gcs"] }
```

そのフィーチャーがなければ、`register_azblob` / `register_gcs`とそれらの設定用構造体は存在しません - 実行時エラーではなく、欠けている項目の名前を挙げるコンパイルエラーになります。

どのコンストラクタにも`_with`というバリアントがあり、レジストリに登録される直前の`suprnova::opendal::Operator`を手渡してくれます。そのため、リトライ/タイムアウト/ロギングのレイヤーをその周りに組み込めます。

```rust,ignore
use std::time::Duration;
use suprnova::opendal::layers::{LoggingLayer, RetryLayer, TimeoutLayer};
use suprnova::Storage;

Storage::register_fs_with("local", "./storage", |op| {
    op.layer(RetryLayer::new().with_max_times(3))
      .layer(TimeoutLayer::new().with_timeout(Duration::from_secs(30)))
      .layer(LoggingLayer::default())
})?;
```

クラウド向けのコンストラクタ（`register_s3`、`register_azblob`、`register_gcs`）は、デフォルトで`RetryLayer`（3回の試行）を適用します。一時的なスロットリングや5xxエラーは、オブジェクトストアではよくあることだからです。完全な制御が必要な場合は、`_with`のバリアントを使ってください。

Suprnovaが配線するopendalのレイヤーの全体は、`RetryLayer`、`TimeoutLayer`、`LoggingLayer`、`TracingLayer`（フレームワークの`otel`フィーチャーが有効なとき、`tracing-opentelemetry`経由でOTelへ橋渡しします）、そして`PrometheusClientLayer`（ヒストグラムとカウンターを、あなたが所有する`prometheus_client::registry::Registry`へ書き出します）です。レイヤーの順序は重要です - 最も外側のレイヤーが、その内側のすべてを包み込みます - そして慣用的な積み方は`RetryLayer → TimeoutLayer → LoggingLayer`です。そうすればタイムアウトした試行もログに残り、リトライが通信の失敗をカバーします。

同じ名前を再登録すると、以前のオペレーターが置き換わり、`warn!`ログが発されます - ディスクは起動時に一度だけ登録されるものであり、誤って重複させると、本番環境のディスクをメモリのディスクへすり替えてしまう可能性があります。置き換え自体はそれでも行われます。警告は、そのすり替えを聞き取れるようにするだけです。

### Suprnovaが異なる設計を選んだ理由

Laravelの`config/filesystems.php`はあらゆるディスクドライバーを列挙し、実行時にそのうち1つを選びます。何もコンパイルから除外されません。Suprnovaは、AzureとGCSをフィーチャーの裏に隠します。Rustではその選択が依存関係のコストを持ち、この場合はセキュリティ上の側面もあるからです。opendalの両方のサービスクレートは`rsa`に依存しており、これは[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)（Marvinタイミング攻撃）を抱えていて、上流での修正リリースもありません。これらをオプトインにすることで、ローカルまたはS3にファイルを保存するアプリが、そのクレートを一切運ばないようにできます。

S3は意図的にゲートされて*いません* - そのシグナーは`rsa`に依存したことが一度もないため、これをゲートしても、最もよく使われるクラウドバックエンドを壊すだけで、何も取り除けません。

### パストラバーサル保護

ローカルファイルシステムのディスクには、ユーザーが指定するどのレイヤーよりも前に、`PathGuardLayer`が適用されます。`disk.write("../escaped.txt", ..)`のようなリクエストは、OSに到達する前に拒否されます - `..`という要素や絶対パスの接頭辞は、いずれもディスクのルートから逃げ出せません。オブジェクトストアとインメモリのバックエンドには、この保護は適用されません（`../foo`のようなキーは、これらのバックエンドでは単なる普通のキー文字にすぎません）。

`..`や絶対パスの要素を拒否した後、この保護はローカルディスクのルートと、要求されたディスク上のターゲットを正規化します。既存のターゲットについては、すべてのシンボリックリンクの要素を解決します。まだ存在しないパスについては、この保護は最も近い既存の祖先まで上へたどり、それを正規化します。その解決済みのパスが正規のルートの外側にある場合、操作は拒否されます。そのため、検証の途中で観測されたルート内のシンボリックリンクが、読み取り、書き込み、一覧取得、コピー、あるいはリネームを、ディスクの外側へ差し向けることはできません。

これは、正規化してから操作するという保護であり、ディスクリプタ相対のファイルシステム閉じ込めではありません。ディスクのルートとその内容が、並行した変更に対して信頼できるものであることを前提としています。検証の後、バックエンドがそのパスを開く前にディレクトリやシンボリックリンクを置き換えられる攻撃者は、time-of-checkからtime-of-useへの競合に勝てる可能性があります。他のプリンシパルがストレージツリーを並行して変更しうる場合は、OSレベルの分離か、専用のファイルシステムを使ってください。

ストリーミングのライター、リスター、コピアーは、この解決済みパスのチェックを、最初のバックエンドI/Oの直前に一度だけ実行します。その後、検証結果はそのストリームセッションの間固定されるため、各チャンクや項目が、ファイルシステムの正規化でブロックされることはありません。コピアーとライターの中断は、有効化される前であっても、検証がもはや完了できないときであっても、常にクリーンアップをそれぞれのバックエンドへ委ねます。

## Laravelの形をしたディスク表面

`Storage::disk(name)`は`suprnova::opendal::Operator`を直接返すため、そのストリーミング表面全体（`writer`、`reader`、`presign_read`、`list`、`stat`、…）を使えます。その上に、[`DiskExt`]トレイト - `Operator`にブランケット実装され、`suprnova::DiskExt`として再エクスポートされています - が、`Storage::disk('local')->...`を通じて手を伸ばすことになる、Laravelの便利なメソッドすべてを追加します。

`use suprnova::DiskExt;`でスコープに持ち込んでください。

### 存在チェック

```rust,ignore
disk.exists("a.txt").await?;        // 生のopendal
disk.missing("a.txt").await?;       // 否定
disk.file_exists("a.txt").await?;   // ファイルのみ（ディレクトリは除く）
disk.file_missing("a.txt").await?;
disk.directory_exists("dir/").await?;
disk.directory_missing("dir/").await?;
```

### 読み取りと書き込み

| Laravel名 | Rustネイティブの相当物 | 備考 |
|--------------|------------------------|------|
| `get(path)`  | `read(path)`           | `get`は`Vec<u8>`を返します。`read`はopendalの`Buffer`を返します。 |
| `put(path, contents)` | `write(path, contents)` | どちらも、あらゆる`Into<Bytes>`を受け付けます。 |
| `json::<T>(path)` | - | serde_json経由で読み取り、デシリアライズします。 |
| `put_json(path, &value)` | - | serde_json経由でプリティプリントします。 |
| `prepend(path, data)` | - | `\n`で結合します。独自の結合には`prepend_with_separator`を使ってください。 |
| `append(path, data)`  | - | `\n`で結合します。独自の結合には`append_with_separator`を使ってください。 |

`prepend`と`append`は、ファイルがまだ存在しない場合はそれを作成するため、ログファイルへの最初の書き込みとして使っても安全です。

### メタデータ

```rust,ignore
let bytes  = disk.size("a.bin").await?;          // u64
let when   = disk.last_modified("a.bin").await?; // Option<DateTime<Utc>>
let mime   = disk.mime_type("a.bin").await?;     // Option<String>
let digest = disk.checksum("a.bin", ChecksumAlgorithm::Sha256).await?;
```

`mime_type`は、まずバックエンドに問い合わせます - S3、Azure、GCSは、保存されている`Content-Type`をそのまま通します。バックエンドがそれを持っていない場合は、`infer`クレートを介して先頭16 KiBを解析します。`Ok(None)`は、認識できないバイナリのブロブのために予約されています。

`checksum`は、[`ChecksumAlgorithm`]を介して`Md5`、`Sha1`、`Sha256`をサポートします。MD5とSHA-1は、LaravelおよびオブジェクトストアのETagとのパリティのために含まれています。新しい整合性チェックには、SHA-256を選んでください。

### 一覧取得

```rust,ignore
let files = disk.files("docs", false).await?;     // トップレベルのファイル
let all   = disk.all_files("docs").await?;        // 再帰的
let dirs  = disk.directories("docs", false).await?;
let all   = disk.all_directories("docs").await?;
```

4つとも、ソート済みの`Vec<String>`を返すため、呼び出し元はバックエンドをまたいで安定した順序に依存できます。ディレクトリは`files`から除外され、ファイルは`directories`から除外されます。ディレクトリのパスは、Laravelの`Storage::directories()`の出力に合わせて、末尾のスラッシュ**なしで**（`"docs/sub"`）返されます - opendalの内部にある`list`は`"docs/sub/"`を報告しますが、パリティのためにこちらでスラッシュを取り除いています。

### ディレクトリとファイルの変更

| Laravel名           | opendalネイティブ        |
|------------------------|-----------------------|
| `make_directory(path)` | `create_dir(path)`    |
| `delete_directory(p)`  | `delete_with(p).recursive(true)` |
| `move_to(from, to)`    | `rename(from, to)`    |

`move_to`は、バックエンドがリネームをサポートしない場合は`copy + delete`にフォールバックし、コピーもサポートしない場合は`read + write + delete`にまでフォールバックします - そのため、テストで使うインメモリドライバーに対しても、本番環境のバックエンドに対しても機能します。

### 署名付きURL

```rust,ignore
let read_url   = disk.temporary_url("uploads/a.pdf", Duration::from_secs(900)).await?;
let upload_url = disk.temporary_upload_url("uploads/new.pdf", Duration::from_secs(900)).await?;
```

`temporary_url`と`temporary_upload_url`は、Laravelとのパリティのために、URLを`String`として返します。これらは`Operator::presign_read` / `presign_write`に支えられているため、署名付きURLの発行を実装していないバックエンドでは、`Unsupported`というメッセージでエラーになります（インメモリとローカルファイルシステムのドライバーがこれに該当します。S3、Azure Blob、GCSはサポートしています）。

## ディスク間のストリーミングコピー

`copy_between_disks(src, src_path, dest, dest_path)`は、バックエンドの組み合わせにかかわらず、ソースのオブジェクトを64 KiB単位のチャンクで送信先へストリーミングします。ソースと送信先は、*どんな*opendalドライバーにも支えられえます - ローカルファイルシステムからS3へ、S3からAzure Blobへ、インメモリからGCSへ、といった具合です。

```rust,ignore
use suprnova::filesystem::streaming::copy_between_disks;

Storage::register_fs("local", "./storage")?;
Storage::register_memory("scratch");
let bytes = copy_between_disks("local", "uploads/big.bin", "scratch", "big.bin").await?;
```

コピーの途中でいずれかのステップが失敗した場合、部分的な送信先オブジェクトは、元のエラーが伝播する前に中断され、削除されます - 失敗したコピーが、切り詰められた送信先として観測されることは決してありません。

## レジストリのメンテナンス

```rust,ignore
let removed = Storage::forget("local");  // bool: 存在していたか？
Storage::purge();                        // すべてのディスクを破棄する
let names = Storage::disks();            // Vec<String>、ソート済み
```

これらは、Laravelの`FilesystemManager::forgetDisk` / `purge`を反映しており、設定の再読み込みや管理ダッシュボードに役立ちます。テスト専用ではありません - 本番環境のコードも、実行時にディスクを破棄して再登録する必要が時折あります（シークレットのローテーション後など）。

## テスト

`Storage::fake()`は、次のことを行うガードを返します。

1. プロセスグローバルなミューテックスを獲得するため、並行して実行される`#[tokio::test]`のケースが、共有レジストリの上で競合することはありません。そして
2. 構築時とドロップ時にレジストリをリセットし、次に実行されるどのテストにとってもクリーンな状態を残します。

便宜のため、`"default"`というメモリディスクがあらかじめ登録されています。

```rust,ignore
use suprnova::filesystem::testing::DiskAssertExt;
use suprnova::{DiskExt, Storage};

#[tokio::test]
async fn stores_and_asserts() {
    let _guard = Storage::fake();
    Storage::register_memory("uploads");
    let disk = Storage::disk("uploads").unwrap();

    disk.put("a.txt", b"hello".to_vec()).await.unwrap();

    disk.assert_exists("a.txt").await;
    disk.assert_contents("a.txt", b"hello").await;
    disk.assert_missing("not-here.txt").await;
    disk.assert_count("", 1, false).await;
    disk.assert_directory_empty("docs/").await;
}
```

5つのアサーションヘルパー - `assert_exists`、`assert_contents`、`assert_missing`、`assert_count`、`assert_directory_empty` - は、[`DiskAssertExt`]トレイトを介して公開されており、`#[cfg(any(test, feature = "testing"))]`でゲートされているため、本番環境のコードがそれらに手を伸ばすことはできません。

## パリティのクイックリファレンス

| Laravel `Storage::disk(...)->...`     | Suprnova                                                 |
|---------------------------------------|----------------------------------------------------------|
| `exists($path)`                       | `disk.exists(path)`                                      |
| `missing($path)`                      | `disk.missing(path)`                                     |
| `fileExists($path)` / `fileMissing`   | `disk.file_exists(path)` / `file_missing(path)`          |
| `directoryExists($p)` / `directoryMissing` | `disk.directory_exists(p)` / `directory_missing(p)` |
| `get($path)`                          | `disk.get(path)` (`Vec<u8>`)                             |
| `json($path)`                         | `disk.json::<T>(path)`                                   |
| `put($path, $contents)`               | `disk.put(path, bytes)`                                  |
| `prepend($path, $data)`               | `disk.prepend(path, data)`                               |
| `append($path, $data)`                | `disk.append(path, data)`                                |
| `size($path)`                         | `disk.size(path)`                                        |
| `lastModified($path)`                 | `disk.last_modified(path)`                               |
| `mimeType($path)`                     | `disk.mime_type(path)`                                   |
| `checksum($path, ['checksum_algo' => 'sha256'])` | `disk.checksum(path, ChecksumAlgorithm::Sha256)` |
| `files($dir, $recursive)`             | `disk.files(dir, recursive)`                             |
| `allFiles($dir)`                      | `disk.all_files(dir)`                                    |
| `directories($dir, $recursive)`       | `disk.directories(dir, recursive)`                       |
| `allDirectories($dir)`                | `disk.all_directories(dir)`                              |
| `makeDirectory($path)`                | `disk.make_directory(path)`                              |
| `deleteDirectory($path)`              | `disk.delete_directory(path)`                            |
| `move($from, $to)`                    | `disk.move_to(from, to)`（またはopendalネイティブの`rename`）    |
| `copy($from, $to)`                    | `disk.copy(from, to)`（opendalネイティブ）                   |
| `delete($path)`                       | `disk.delete(path)`（opendalネイティブ）                     |
| `temporaryUrl($path, $expiry)`        | `disk.temporary_url(path, expire)`（またはopendalネイティブの`presign_read`） |
| `temporaryUploadUrl($path, $expiry)`  | `disk.temporary_upload_url(path, expire)`（またはopendalネイティブの`presign_write`） |
| `Storage::fake()`                     | `Storage::fake()`                                        |
| `Storage::disk()->assertExists()`     | `disk.assert_exists(path).await`                         |
| `FilesystemManager::forgetDisk($n)`   | `Storage::forget(name)`                                  |
| `FilesystemManager::purge()`          | `Storage::purge()`                                       |

## 設定

ストレージの設定は、`.env`ではなく、完全にRustのコードの中に存在します。ディスクは、`bootstrap()`の中で`Storage::register_*`を通じて名前で登録され、呼び出し箇所では名前で（`Storage::disk("public")`のように）扱われます。フレームワークが読み取る`FILESYSTEM_DISK`という環境変数はなく、暗黙のデフォルトディスクもありません - 各ドライバーは対等です。あるアップロードやダウンロードがどのディスク名を対象とするかは、アプリが決めます。そして、選んだドライバーが必要とするURL / キー / 認証情報は、アプリ自身の環境変数として渡してください。

フレームワークが環境から読み取る箇所と、コード側での登録を期待する箇所についての、より広いルールは[設定](configuration.md)を参照してください。

## 次のステップ

- [設定](configuration.md) - フレームワークが`.env`から読み取るもの（そして、なぜストレージがそのリストに載っていないか）
- [リクエスト](requests.md) - `UploadedFile::store_as`を通じて、ファイルのアップロードがディスクへ着地すること
- [レスポンス](responses.md) - ディスクからバイト列をストリーミングで返すこと
- [キャッシュ](cache.md) - もう一つの名前付きドライバーレジストリで、同じ形をしています
- [テスト](testing.md) - もっと広い、すべてを偽装するテストの表面

[`DiskExt`]: https://docs.rs/suprnova/latest/suprnova/trait.DiskExt.html
[`DiskAssertExt`]: https://docs.rs/suprnova/latest/suprnova/filesystem/testing/trait.DiskAssertExt.html
[`ChecksumAlgorithm`]: https://docs.rs/suprnova/latest/suprnova/enum.ChecksumAlgorithm.html

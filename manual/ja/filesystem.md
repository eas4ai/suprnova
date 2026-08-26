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

## ディスクを登録する

すべてのディスクは、起動時に`Storage::register_*`経由で一度だけ登録され、`Storage::disk(name)`を通じて名前で参照されます。他のドライバーが劣化して落ち着く先の「デフォルトのバックエンド」というものはありません - 各ドライバーは対等です。

| コンストラクタ                        | バックエンド                   | フィーチャー         |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | ローカルファイルシステム        | `filesystem`        |
| `Storage::register_memory(name)`     | プロセス内のメモリ（テスト）     | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3またはS3互換          | `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage          | `filesystem-gcs`    |
| `Storage::register_read_through(name, cfg)` | リードスルーの複合ディスク | `filesystem` |

`filesystem`はデフォルトで有効ですが、AzureとGCSのフィーチャーはそうではありません。どちらかを`Cargo.toml`で有効にしてください:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.5", features = ["filesystem-gcs"] }
```

フィーチャーがなければ、`register_azblob` / `register_gcs`とそれらの設定構造体は存在しません - 実行時の失敗ではなく、欠けている項目を名指しするコンパイルエラーが得られます。

すべてのコンストラクタには`_with`という変種があり、レジストリに着地する直前の`suprnova::opendal::Operator`を手渡してくれるため、その周りにリトライ/タイムアウト/ロギングのレイヤーを取り付けられます:

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

クラウドのコンストラクタ（`register_s3`、`register_azblob`、`register_gcs`）は、オブジェクトストアでは一時的なスロットリングや5xxのエラーが日常茶飯事であるため、デフォルトで`RetryLayer`（3回の試行）を適用します。完全な制御が必要なときは、`_with`の変種を使ってください。

Suprnovaが配線するopendalのレイヤーの完全な集合は、`RetryLayer`、`TimeoutLayer`、`LoggingLayer`、`TracingLayer`（フレームワークの`otel`フィーチャーが有効なとき、`tracing-opentelemetry`経由でOTelへ橋渡しします）、そして`PrometheusClientLayer`（あなたが所有する`prometheus_client::registry::Registry`へ、ヒストグラムとカウンターをエクスポートします）です。レイヤーの順序は重要です - 最も外側のレイヤーが、その内側のすべてを包みます - そして、イディオマティックなスタックは`RetryLayer → TimeoutLayer → LoggingLayer`であり、これならタイムアウトした試行もログに残り、リトライがトランスポートの失敗をカバーします。

同じ名前で再登録すると、以前のオペレーターを置き換え、`warn!`のログを出力します - ディスクは起動時に一度だけ登録されることを意図しており、うっかりした重複が、本番環境のディスクをメモリのディスクへ入れ替えてしまいかねないからです。置き換え自体はそれでも起こります。警告は、その入れ替えを聞こえるようにするだけです。

### Suprnovaが異なる設計を選んだ理由

Laravelの`config/filesystems.php`はすべてのディスクドライバーを列挙し、あなたは実行時に1つを選びます。コンパイルで取り除かれるものは何もありません。SuprnovaがAzureとGCSをフィーチャーの背後にゲートするのは、Rustではその選択に依存関係のコストがあり、しかもこれにはセキュリティの側面があるからです: どちらのopendalのサービスクレートも`rsa`を引き込みますが、これは[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)（Marvinタイミング攻撃）を抱えており、上流に修正済みのリリースがありません。これらをオプトインにすることは、ファイルをローカルまたはS3に保存するアプリが、そのクレートを決して抱え込まないことを意味します。

S3は意図的にゲートされて*いません* - その署名器は`rsa`に依存したことが一度もないため、ゲートしても最も使われているクラウドバックエンドを壊すだけで、何も取り除かないからです。

### ローカルディスクへのアトミックな書き込み

ローカルディスクでは、あるパスにバイト列を公開する操作はすべて、それを1ステップで公開します。`disk.write(...)`、`disk.writer(...)`、そして`disk.copy(...)`は、いずれもまず`<root>/.suprnova-atomic/`に着地し、そこでフラッシュされて同期され、その後で行き先へリネームされます。`disk.rename(...)`は、もとから1ステップです。そのため、並行する読み手が見るのは、以前のオブジェクトか、出来上がった新しいオブジェクトのどちらかであり、途中までの長さを見ることは決してありません。そして、書き込みの途中で死んだプロセスは、生きているパスの上に切り詰められたオブジェクトを残すのではなく、行き先を触れないまま残します。

`append`だけが、その場で行われる唯一の操作です。appendをステージングするということは、まずオブジェクト全体をコピーするということになるからです。これは、そのオブジェクトを*作成する*appendにも、その後のすべてのappendとまったく同じように当てはまります。そのため、同じ新しいオブジェクトへappendする2つの書き手は、どちらも着地します。その場で行われることは、appendがあなたに払わせる代償でもあります: 失敗した、あるいは中断されたappendは、既存のオブジェクトへのappendが常にそうであったのとまったく同じように、空のまま、あるいは短いままのオブジェクトを後に残します。

条件付きの書き込みは、リネームではなく`link(2)`で公開されます。これによって、チェックしてから上書きするのではなく、本物の排他的な作成であり続けます:

```rust,ignore
// いくつの呼び出し元が競合しても、ここでOkを得るのはちょうど1つだけです。ほかのすべては
// `ErrorKind::ConditionNotMatch`のエラーを受け取り、何も書きません。
disk.write_with("locks/import.json", body).if_not_exists(true).await?;
```

この公開には、ハードリンクを持つファイルシステムが必要です。FAT、exFAT、そして一部のネットワークファイルシステムでは`link(2)`がサポートされていないため、条件付きの書き込みは、チェックしてから上書きするという形へ黙って劣化するのではなく、そこで失敗します - 劣化すれば、成り立たない排他性の保証をあなたへ手渡すことになるからです。ほかのすべての操作は影響を受けません。

リネームによる公開は、オブジェクトのinodeを置き換えます。そのため、書き直しは以前のファイルのモード、所有者、ハードリンクを保ちませんし、開いたディスクリプタを保持している読み手は、新しいバイト列を見るのではなく、古いコンテンツを読み続けます。これはアトミックな公開と引き換えになる通常のコストですが、そのどちらかに頼っていたのであれば、これは挙動の変更です。

保護機構が解決できないシンボリックリンク - リンク先が存在しない、ぶら下がったリンク - を通ってディスクへ届くパスは、作ってよい空いた名前として扱われるのではなく、拒否されます。そのようなリンクを通して作成すれば、ホスト上のどこであれ、そのリンクの指す先を作成することになります。そのため保護機構は、無害なぶら下がったリンクと、脱出の試みとを見分けられず、どちらも拒否します。

`.suprnova-atomic`という名前は、すべてのローカルディスクのルートで予約されています。最初のコンポーネントがその名前であるパスは、権限エラーで拒否されますし、シンボリックリンクを通してそのディレクトリの中へ*解決される*パスも同様です。そのため、ほかの書き手のステージングファイルを読むことも、そのディレクトリへ書き込むことも、それを削除することもできません。このエントリは`files`、`directories`、`all_files`、`all_directories`から除外されるため、オブジェクトとして現れることは決してありません。バックアップや同期のツールがこの名前を必要とするため、`suprnova::ATOMIC_STAGING_DIR`としてエクスポートされています: ロック用のディレクトリを除外するのと同じやり方で、このディレクトリを除外してください。ここには、進行中の一時ファイルと、公開の途中で死んだプロセスが残していったものが入っており、それらを掃除するものは何もありません。そのため、クラッシュループに陥ったホストでは、誰かが空にするまで膨らみ続けます - 何も書き込んでいない間であれば、空にしても安全です。

### パストラバーサル保護

ローカルファイルシステムのディスクには、ユーザーが与えるどのレイヤーよりも前に`PathGuardLayer`が適用されます。`disk.write("../escaped.txt", ..)`のようなリクエストは、OSに到達する前に拒否されます - `..`のコンポーネントも絶対パスのプレフィックスも、ディスクのルートから抜け出すことはできません。オブジェクトストアとインメモリのバックエンドは、この保護機構を受け取りません（`../foo`のようなキーは、それらのバックエンドでは単なる普通のキーの文字だからです）。

`..`と絶対パスのコンポーネントを拒否した後、保護機構はローカルディスクのルートと、要求されたディスク上のターゲットを正規化します。存在するターゲットは、すべてのシンボリックリンクのコンポーネントを解決します。まだ存在しないパスについては、保護機構は最も近い既存の祖先まで遡り、それを正規化します。解決されたパスが正規化されたルートの外側にある場合、その操作は拒否されます。そのため、検証中に観測されたルート内のシンボリックリンクが、読み取り、書き込み、一覧、コピー、リネームをディスクの外へリダイレクトすることはできません。

これは、正規化してから操作する保護機構であり、ディスクリプタ相対のファイルシステムの閉じ込めではありません。これは、ディスクのルートとその内容が、並行する変更に対して信頼できることを前提にしています: 検証の後、バックエンドがパスを開く前にディレクトリやシンボリックリンクを置き換えられる攻撃者は、time-of-checkからtime-of-useまでの競合に勝つ可能性があります。他のプリンシパルが並行してストレージツリーを変更しうる場合は、OSレベルの隔離か、専用のファイルシステムを使ってください。

ストリーミングのライター、リスター、コピアーは、この解決済みパスのチェックを、最初のバックエンドI/Oの直前に一度だけ実行します。その後、検証はそのストリームセッションについて固定されるため、各チャンクや各項目がファイルシステムの正規化でブロックすることはありません。コピアーとライターの中断は、有効化の前であっても、あるいは検証がもう完了できないときであっても、常にクリーンアップをそれらのバックエンドへ転送します。

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

## リードスルーディスク

リードスルーディスクは、速い*プライマリ*と、より遅い*フォールバック*を組にし、オブジェクトが読まれるのに合わせて、後者から前者へとそれらを移します。プライマリを移行先のストアへ、フォールバックを移行元のストアへ向ければ、ワーキングセットは実トラフィックの下で乗り移っていきます - メンテナンスウィンドウも、誰も要求しないオブジェクトの一括コピーも要りません。

```rust,ignore
use suprnova::{ReadThroughConfig, S3Config, Storage};

Storage::register_s3("new-store", S3Config { bucket: "assets-2".into(), ..Default::default() })?;
Storage::register_s3("legacy-store", S3Config { bucket: "assets-1".into(), ..Default::default() })?;

Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "new-store".into(),
        fallback: "legacy-store".into(),
        ..Default::default()
    },
)?;

let assets = Storage::disk("assets")?;
// `logo.png`を`legacy-store`から読み、その帰り道で`new-store`へ書き込む。
// 以降の読み取りはすべて`new-store`が応じる。
let bytes = assets.read("logo.png").await?;
```

`Storage::disk("assets")`は通常の`Operator`を返すため、その上のあらゆるメソッドと、あらゆる`DiskExt`の便利機能が、そのまま変わらず動作します。

### どの操作にどのディスクが答えるか

| 操作 | ディスク |
|---|---|
| `read` | オブジェクトを保持していればプライマリ、そうでなければフォールバック - そして、`copy`が`false`でないかぎり、フォールバックでのヒットは昇格される |
| `exists`、`size`、`last_modified`、`mime_type`、`stat` | オブジェクトを保持していればプライマリ、そうでなければフォールバック |
| `write`、`make_directory` | プライマリのみ |
| `files`、`directories`、`list` | プライマリのみ - フォールバックのエントリは一覧からは見えない |
| `delete` | 両方、フォールバックが先 |
| `copy`、`rename` / `move_to` | ソースを保持していればプライマリ、そうでなければフォールバックからストリームで渡される。`rename`はフォールバック側のソースも削除する |
| `temporary_url` | オブジェクトを保持していればプライマリ、そうでなければフォールバック |
| `temporary_upload_url` | プライマリのみ - アップロードは、書き込みが着地する場所に着地しなければならない |

一覧がプライマリのみであるのは、設計上の判断です。和集合の一覧は、2つのバックエンドにまたがってページングと並び順を調停しなければならず、しかも、いったん昇格されれば後の一覧では返らなくなるオブジェクトを報告することになります。フォールバックに何が残っているかを列挙する必要があるときは、`Storage::disk("legacy-store")`を直接使ってください。

削除は、両方のディスクからオブジェクトを取り除きます。プライマリのコピーだけを取り除いたなら、次の読み取りがフォールバックのコピーをそのまま昇格し直してしまうからです。その帰結として、読み取り専用のフォールバックの上に載ったリードスルーディスクは、削除ができません: フォールバック側の削除が失敗し、そのエラーがあなたに届きます。

### 昇格が失敗したとき

デフォルトでは、昇格の失敗は`warn`でログに記録され、飲み込まれます。あなたは要求したバイトを変わらず受け取り、ディスクは単に、プライマリが再び書き込み可能になるまで、毎回フォールバックを読む形へ劣化します。昇格が黙って失われることが、あなたの見なければならない障害を隠してしまうとき - 例えば、あなたが終わらせようとしている移行のとき - は、`throw_on_promotion_failure: true`を設定してください:

```rust,ignore
Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "new-store".into(),
        fallback: "legacy-store".into(),
        throw_on_promotion_failure: true,
        ..Default::default()
    },
)?;
```

登録は、動作しえない設定を拒否します: 空の`primary`または`fallback`、同じディスクを2度名指しする組、自分自身を名指しするディスク、そして登録されていない名前です。いずれの場合も、問題を名指しする`FrameworkError`が返り、ディスクは登録されません。

### 昇格せずに読む

`copy: false`を設定すると、フォールバックでのヒットを、書き抜くことなく提供します:

```rust,ignore
Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "cache-store".into(),
        fallback: "origin-store".into(),
        copy: false,
        ..Default::default()
    },
)?;
```

そうすると、ディスクは透過的なオーバーレイのように読めます: プライマリは自分が保持しているものに答え、フォールバックはそれ以外のすべてに答え、両者の間では何も移動しません。プライマリが小さなキャッシュであって、1回きりの読み取りでそれを埋めたくないとき、あるいはフォールバックが正であって、プライマリはあなたが意図して置いたオブジェクトしか保持しないときに使ってください。

このフラグが支配するのは、読み取り時の昇格だけです。書き込み、削除、メタデータ、一覧、そして`copy`と`rename`の行き先は、すべて昇格が有効なときとまったく同じように振る舞います - そのため、`copy: false`のディスクでも、コピーまたは移動されたオブジェクトはプライマリに着地します。書き戻しが起きないため、`copy: false`での読み取りは、オブジェクト全体ではなく、あなたが要求した範囲だけを取得します。

### フォールバックをまたぐコピーと移動

`copy`と`rename`は、まずプライマリに対してソースを解決します。フォールバックだけがそれを保持しているとき、オブジェクトは64 KiBのチャンクでストリームされて渡り、行き先はプライマリに着地します:

```rust,ignore
let assets = Storage::disk("assets")?;

// `logo.png`は`legacy-store`にしか存在しない。コピーはそれをストリームで渡し、
// `branding/logo.png`を`new-store`へ書き込む。レガシー側のオブジェクトはそのまま残る。
assets.copy("logo.png", "branding/logo.png").await?;

// 移動は同じことをしたうえで、レガシー側のソースを削除する。
assets.rename("logo.png", "branding/logo.png").await?;
```

移動は、どちらの経路でも - プライマリがソースを保持していたかどうかにかかわらず - フォールバック側のソースを削除します。そうしなければ、次の読み取りがフォールバックのコピーを昇格し直し、その移動を取り消してしまうからです。

2つの経路は、それをいつ削除するかで異なり、その違いは、失敗した移動が何を後に残すかとして現れます:

- プライマリがソースを保持していた場合。フォールバックのコピーが先に、renameより前に消えます。プライマリがそのパスを保持しているあいだ、フォールバックのコピーはこのディスクを通しては到達できないため、先に取り除いても、あなたに観測できるものは何も変わりません - そして、その削除が失敗したのなら、まだ何も動いていません。移動をリトライしてください。逆に、削除が成功したうえでrenameが失敗したのなら、フォールバックはそのパスについて何も保持しておらず、行き先は書かれておらず、プライマリは依然としてソースを保持しています - そのため、リトライはこれと同じ経路を通り、もう一度renameします。この失敗が奪うのは、コールド側のコピーだけで、ほかには何もありません。
- フォールバックだけがそれを保持していた場合。削除は、行き先が所定の位置に収まった後にしか来られないため、削除で失敗した移動は、行き先が書かれ、ソースがフォールバックに残ったままの状態を残します。移動をリトライしてください。ソースはもうプライマリにあるので、リトライは1つ目の経路を通ります。

いずれにせよ、失敗した移動は安全にリトライでき、最終的に手に入る行き先は、その移動が出発点にしたオブジェクトです。

条件は、ストリーミングの経路でも操作とともに運ばれます。`if_not_exists`は条件付きの書き込みになるため、ガードされたコピーや移動は、既存の行き先を踏みつぶすのではなく、変わらず拒否します。そして、ソースのバージョンを名指しするコピーは、そのバージョンをフォールバックから取り出します。コピーの`if_match`だけが例外です: これはバックエンドが自身のcopyの内側で適用する条件であり、その呼び出しこそがこの経路にはできないものであるため、黙って無視されるのではなく、その条件を名指しする`Unsupported`のエラーで拒否されます。

これによって条件は、どちらのディスクがソースを保持しているかが表に出る、唯一の場所になります。ローカルディレクトリは`copy`と`rename`が使えると表明しますが、そのどちらの条件付きの形も表明しないため、`copy_with(a, b).if_not_exists(true)`は、フォールバックだけが`a`を保持しているときには（条件付きの書き込みになるため）成功し、プライマリがそれを保持しているときには`Unsupported`で拒否されます。必要な条件は、そのディスク上のすべてのオブジェクトについて成り立つと決めてかかるのではなく、プライマリのドライバーに対して確かめてください。

プライマリが拒否するはずの移動は、何かが削除されるより前に拒否されます。`rename`をまったく持たないプライマリ、条件付きの`rename`を持たないプライマリの上へのガードされた移動、そして既に存在する行き先の上へのガードされた移動は、いずれもフォールバック側のソースをそのままに残して失敗します - 決して起こらない移動が、あなたからコールド側のコピーを奪ってはならないからです。

ストリームが途中で失敗した場合、ライターは中断され、その転送が作った行き先は、エラーがあなたに届くより前に削除されます。そのため、失敗した転送が、切り詰められたオブジェクトとして観測されることはありません。もともとそこにあった行き先は、そのまま放っておかれます - 失敗したコピーが、自分では一度も書かなかったオブジェクトを壊すものであってはなりません。ローカルファイルシステムのプライマリも、これを守ります。転送を`.suprnova-atomic/`の下にステージングし、成功したときにだけリネームするからです。ライターを中断すればステージングされたファイルは取り除かれるため、失敗した転送は、途中までの行き先も、取り残された一時ファイルも残しません。

### バージョン付きの読み取りと条件付きの読み取り

バージョンや、`If-Match`、`If-None-Match`、`If-Modified-Since`、`If-Unmodified-Since`の条件を運ぶ読み取りは、その条件を保ったまま先へ渡されるため、その答えは、あなたが意味させようとしたとおりの意味を持ちます。そのような読み取りは提供されますが、決して昇格されません: 古いバージョンや、バリデーターに一致した本体をプライマリへ書き込むことは、それをライブのオブジェクトとして公開することであり、以降のあらゆる素の読み取りがそれを受け取ってしまうからです。

どちらのディスクがそれに答えるかは、いつもどおりに決まります。最初の探りは通常の存在チェックであるため、リードスルーディスクは、プライマリがそのパスを少しでも保持しているときは常に、バージョン付き・条件付きの読み取りをプライマリへ委ねます。フォールバックへ届くのは、プライマリが保持していないときだけです。

リードスルーディスクがこれらのうちどれを受け付けるかを決めるのも、プライマリです。プライマリのリーダーが先に開かれるからです。プライマリがローカルディレクトリであるリードスルーディスクに対するバージョン付きの読み取りは、ローカルディレクトリにはバージョンがないため、フォールバックへ届く前に拒否されます。

### Suprnovaが異なる設計を選んだ理由

Laravelは、`primary`と`fallback`のキーがディスク名かインラインのドライバー設定のどちらかを受け付ける、`config/filesystems.php`のエントリからリードスルーディスクを構築します。Suprnovaが取るのはディスク名だけです。ここでのディスクは、配列で記述されるのではなく、型付きのコンストラクタによって登録されるからです - 内側のディスクを先に登録し、それから名前で指してください。

Laravelの昇格は、フォールバックを読んだ後にプライマリを再チェックするため、並行する書き込み側が勝ちます。Suprnovaはそのチェックを保ったうえで、Laravelがしないこと - 昇格をアトミックに公開すること - もします。ローカルファイルシステムのプライマリの上では、バイトはいったん一時的な隣接ファイルへ置かれ、renameで所定の位置へ収められます。それらをターゲットへ直接書けば、書き込みの長さのあいだ、育ちつつある書きかけのファイルが見えたままになり、しかもリードスルーディスクは、まさにその存在チェックによって読み手をルーティングします。renameを持たないプライマリ - インメモリ、S3、Azure Blob、GCS - では、書き込みはすでに1回の分割できない公開であるため、昇格はターゲットへ直接書き込みます。2つの並行する読み手が両方とも昇格してしまわないよう、そのオブジェクトがまだ存在しないことを条件にしたうえでです。

いったん置く形の昇格が持てないのは、まさにその条件です: いったん置くパスは一意であるため、それに対する上書き禁止の条件は空虚になり、しかもターゲットは上書きするrenameによって公開されるからです。したがって、ローカルファイルシステムのプライマリの上のリードスルーディスクは、それを手放します - 昇格の最後の存在チェックとそのrenameの間の瞬間にプライマリへ着地した書き込みは、昇格されたコピーによって上書きされます。renameを持たないプライマリでは、条件は成り立ち、そのような窓は存在しません。

いったん置かれるオブジェクトは、存在するあいだ、プライマリ上の本物のエントリです。そのため、昇格の途中で取られた一覧には、`.suprnova-promote-<id>.tmp`という隣接ファイルが現れることがあります。完了した読み取りも、失敗した読み取りも、諦めた読み取りも、自分の隣接ファイルを取り除こうとし、その削除に失敗した場合は、読み取りを失敗させるのではなく警告をログに記録します。失敗した削除や、クラッシュしたプロセスや、昇格の途中でキャンセルされた読み取りのフューチャーが残した隣接ファイルを、何かが掃除してくれることはありません: それらは手で取り除かなければなりません。

フォールバックから解決された読み取りは、昇格にはオブジェクト全体が必要であるため、昇格の書き込みが完了するまでオブジェクトをメモリ上に保持します。これは、リードスルーディスクが対象としている階層化のケースには合っています。非常に大きなコールドのオブジェクトについては、フォールバックのディスクを直接読むか、代わりに[`copy_between_disks`](#ディスク間のストリーミングコピー)を使ってください。

Laravelは、`copy`が`false`のときはフォールバック自身のストリームを返し、`true`のときは`php://temp`を通じてバッファリングします。Suprnovaは代わりに、`copy`が`false`のときはフォールバックからの取得を要求された範囲へ絞り込み、どのみちオブジェクト全体が必要になる昇格の経路でのみバッファリングします。

Laravelのフォールバックをまたぐ`copy`と`move`も、ソースを`php://temp`を通じてバッファリングします。Suprnovaは代わりに、それを64 KiBのチャンクでストリームします。フォールバックこそが、大きく、めったに触られないオブジェクトの住処だからです。そして、エラーを返すより前に、書きかけの行き先を削除します。さらに2つの相違が、OpenDALから導かれます。存在しないパスの削除は成功と数えられるため、移動は、フォールバック側のソースが存在するかを先に確かめることなく、それを消し去ります。そして、OpenDALは`copy`と`rename`の上に、Flysystemには対応物のない条件を運ぶため、Suprnovaは、ソースがフォールバックにしかないときに各条件が何を意味するのかを決めなければなりません: `if_not_exists`とコピーのソースバージョンは尊重され、コピーの`if_match`は、落とされるのではなく拒否されます。

Laravelは、どちらの経路でも、移動の後にフォールバック側のソースを削除します。Suprnovaは、プライマリがソースを保持しているときは、それを先に削除します。2つの順序は、リトライの下で違いを生むからです: どちらにせよソースはディスクを通しては到達できませんが、後で削除するということは、一時的な障害で削除を落とした移動が、ソースがもうフォールバックにしかない移動として戻ってきて、最初の試行が既に正しく書いた行き先の上へ、フォールバックの陳腐化したコピーを流し込むということを意味します。

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

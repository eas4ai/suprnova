# ベクトル

Suprnovaは、4つのドライバー - プロセス内のMemory、Qdrant、Pinecone、あるいはMariaDBネイティブの `VECTOR(N)` - のいずれかに支えられた、Laravel形の `Vector` ファサードを出荷します。ドライバーは `Vector::register` を介して起動時に明示的に選ばれます。ファサードは `VectorDriver` トレイトの上に被さる薄い層であるため、独自のバックエンドも組み込みのものと同じやり方で差し込めます。

## クイックスタート

```rust
use std::sync::Arc;
use suprnova::{MemoryVectorDriver, Vector, VectorItem};

// ブートストラップ（通常はアプリ起動時に一度だけ）
Vector::register("documents", Arc::new(MemoryVectorDriver::new()));

// 使う
let store = Vector::store("documents")?;
store
    .upsert(vec![
        VectorItem::new("doc-1", embedding_for("Hello"), serde_json::json!({ "title": "Hello" })),
        VectorItem::new("doc-2", embedding_for("World"), serde_json::json!({ "title": "World" })),
    ])
    .await?;

let hits = store.similar(query_embedding, 10).await?;
for hit in hits {
    println!("{}: {} (score {:.3})", hit.id, hit.metadata["title"], hit.score);
}
```

## 契約

```rust
#[async_trait]
pub trait VectorDriver: Send + Sync + 'static {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError>;
    async fn similar(&self, store: &str, query: Vec<f32>, k: usize) -> Result<Vec<VectorMatch>, FrameworkError>;
    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError>;
    async fn count(&self, store: &str) -> Result<usize, FrameworkError>;
}
```

`VectorItem` は、任意の `String` id、`embedding: Vec<f32>`、そして自由形式の `metadata: serde_json::Value`（JSONオブジェクトまたは `null` でなければなりません）を運びます。`VectorMatch` は、元のid、バックエンドの類似度スコア、そして同じ形のmetadataを返します。

トレイトは意図的に小さく保たれています。検索に対するフィルタ式、スパースベクトル、scroll/list、スナップショット、あるいは量子化のノブが必要なときは、公開された `client()` という逃げ道を通じて、ドライバー裏側のSDKへ降りてください。

### Suprnovaが異なる設計を選んだ理由

Laravelは、Postgresの `pgvector` を通じてのみベクトルを出荷します。それがPHP形の答えです: 1つのストレージバックエンドを選び、それを単一のドライバーの裏に隠し、それで完了とする、というものです。Suprnovaは、その選択を設定上の関心事として扱います。同じトレイトが、テスト用のプロセス内 `HashMap`、埋め込み数が運用コストを正当化するときの専用のベクトルDB（Qdrant、Pinecone）、そして、ベクトルをそれを生成した行のそばに置いておきたいときのリレーショナルバックエンド（MariaDB 11.7+）をカバーします。Weaviate、Milvus、LanceDB、pgvector、LibSQLは、実際の利用者からの需要の後ろに並んで待っています - どれもトレイトの形によってブロックされてはいません。

アプリの残りの部分が1つのエンジンに収まるなら、MariaDB 11.7+ は、リレーショナルなテーブル、JSONドキュメント、システムバージョン管理された時制データと並んでベクトルを保持します - Postgres + Redis + Qdrantを別々に動かすより、動く部品が少なくなります。この推奨の背景については[デプロイメント](deployment.md)を参照してください。

## ドライバー

### インメモリ - `MemoryVectorDriver`

`HashMap` に支えられたプロセス内ドライバーです。コサイン類似度で、次元の不一致した点はクエリ時に無言でスキップされ（そのため混在した次元のテストデータが暴発しません）、ゼロベクトルのクエリははっきりと失敗します。

```rust
Vector::register("docs", Arc::new(MemoryVectorDriver::new()));
```

テストと開発で使ってください。それぞれの `MemoryVectorDriver::new()` インスタンスは独立しています - 2つの `new()` の間で状態は共有されません。

### Qdrant - `QdrantVectorDriver`

公式の `qdrant-client` SDKを介して、gRPC（デフォルトポート6334）でQdrantと話します。

```rust
use suprnova::{QdrantDistance, QdrantVectorDriver};

let driver = QdrantVectorDriver::from_url("http://localhost:6334")?
    .with_distance(QdrantDistance::Cosine)  // デフォルト
    .with_auto_create(true);                // デフォルト

Vector::register("docs", Arc::new(driver));
```

Qdrant Cloud向けには:

```rust
let driver = QdrantVectorDriver::from_url_with_api_key(
    "https://xxxxxxxx.eu-central.aws.cloud.qdrant.io:6334",
    std::env::var("QDRANT_API_KEY")?,
)?;
```

**IDマッピング。** Qdrantは、ポイントIDが `u64` か有効なUUIDのいずれかであることを要求します。フレームワークは、3つの規則で任意の文字列を橋渡しします:

1. 文字列が `u64` としてパースできれば、`Num(u64)` バリアントを使います。
2. 文字列が有効なUUIDであれば、`Uuid(String)` バリアントをそのまま使います。
3. それ以外の場合は、安定した名前空間から決定的なv5のUUIDを導出します。

呼び出し元の元の文字列は、ポイントのpayload内の予約されたキー `__suprnova_id`（`SUPRNOVA_ID_PAYLOAD_KEY` としてエクスポートされます）に保管され、取得時に `VectorMatch.metadata` から取り除かれます。`driver.client()` を介してQdrantへ直接クエリを投げるパワーユーザーは、フレームワークによる書き込みと直接呼び出しを橋渡しするために `__suprnova_id` でフィルタできます。

**自動作成。** 未知のコレクションに対する最初の `upsert` で、ドライバーは、最初の項目から推論された次元と、設定された距離指標（デフォルトはコサイン）でそれを作成します。競合状態に対して安全です - 同じ新しいコレクションに対する並行したupsert元は失敗しません。先に作成した側が勝ち、もう一方はそのまま進みます。明示的な作成を必須にするには `.with_auto_create(false)` で無効化してください。

**キャッシュの無効化。** コレクションが外部から削除された場合（あるいはQdrantが永続化前に再起動した場合）、ドライバーはupsert時に「not found」エラーを検出し、キャッシュエントリを落とし、`ensure_collection` を再実行して、一度だけリトライします。

**逃げ道。** `driver.client()` は、裏側の `qdrant_client::Qdrant` を返します - 検索のフィルタ式、scroll、スナップショット、あるいはトレイトを介して表面化していない他のAPIに使ってください。`QdrantVectorDriver::resolve_point_id`、`build_point`、`decode_match` は、id変換を失わずに、直接の呼び出しとトレイト経由の呼び出しを混在させられます。

**ローカルセットアップ。** Dockerを介してQdrantを実行します:

```bash
docker run -p 6334:6334 -p 6333:6333 qdrant/qdrant
```

統合テストは次で実行されます:

```bash
QDRANT_URL=http://localhost:6334 cargo test -p suprnova --test vector_qdrant -- --ignored
```

### Pinecone - `PineconeVectorDriver`

> **フィーチャーゲート付き - デフォルトでオフです。** `cargo build --features vector-pinecone` で有効化してください（あるいは、`Cargo.toml` の `suprnova` 依存の下に `features = ["vector-pinecone"]` を追加してください）。このフィーチャーは追加の依存を必要としません - ドライバーのコンパイルをゲートするだけで、それ以上のことはしません - そのため、ほとんどのアプリはPineconeを使わず、それをコンパイルするコストを払うべきではないという理由だけで、単純にオフになっています。

フレームワークが既に持っているHTTPクライアントを使い、REST APIを介してPineconeと話します。

> **なぜ公式SDKではないのか？** このドライバーはかつて、gRPCを話す `pinecone-sdk` をラップしていました。そのクレートの最新リリース（0.1.2、2024-09-06公開）は `tonic 0.11 → rustls 0.22 → rustls-webpki 0.102` を固定しており、`rustls-webpki 0.102` は、`>= 0.103.13` ではすべて修正済みの、4件のRustSec勧告を抱えています。放棄された1つのクレートが、ツリー全体の足を引っ張っていました。「アップストリームを待つ」という選択肢に、終わりが来る見込みはありませんでした。Pineconeは、このドライバーが必要とするあらゆる操作をHTTPS越しに公開しているため、RESTの経路は、4件の勧告と2つの依存を一度に取り除きました。

```rust
use suprnova::PineconeVectorDriver;

// APIキーを直接
let driver = PineconeVectorDriver::from_api_key(std::env::var("PINECONE_API_KEY")?)?;

// またはenv経由: PINECONE_API_KEY、加えて任意で PINECONE_CONTROLLER_HOST
// と PINECONE_API_VERSION
let driver = PineconeVectorDriver::from_env()?;

// デフォルト以外の名前空間にバインドする
let driver = driver.with_namespace("public");

Vector::register("docs", Arc::new(driver));
```

`Vector::store(name)` を介して渡されるストア名は、Pineconeのインデックス名にマッピングされます。ドライバーは、そのインデックスのホストを、コントロールプレーンの `GET /indexes/{name}` を介して初回使用時に遅延解決し、キャッシュします。すでに分かっているホストを固定して、ラウンドトリップをスキップしてください:

```rust
let driver = PineconeVectorDriver::from_env()?
    .with_index_host("docs", "docs-abc123.svc.aped-1234.pinecone.io");
```

コントロールプレーンから学習されたホストは、レスポンスが何と言おうと、常に `https` で接続されます。`with_index_host` を通じて固定されたホストは、あなたが与えたスキームを保つため、`http://` のローカルエミュレーターも動作します。

**APIバージョン。** PineconeはREST APIを日付でバージョニングし、そのバージョンをヘッダーに固定することを求めます。ドライバーは `2025-04` を固定します - そのリクエストとレスポンスの形が書かれ、テストされたバージョンです - そして、意図的に移行するために `with_api_version`（または `PINECONE_API_VERSION`）を公開しています。これは流動的ではありません: `describe_index_stats` における名前空間キーの規約は、バージョン間で変わってきたものの1つであり、`count()` はそのマップを読みます。

**自動作成なし。** Pineconeのインデックス作成は、クラウド（AWS/GCP/Azure）、リージョン、ベクトル次元、距離指標、削除保護の選択を要求します - デフォルトでうまく決められるにはトレードオフが多すぎます。登録の前に、Pineconeのコンソール、PineconeのCLI、あるいは `control_plane_post` 呼び出しを介してインデックスを作成し、その後、既存の名前へフレームワークを向けてください。

これが、最初のupsertでコレクションを自動作成するQdrantドライバーとの主な非対称性です。

**IDとmetadata。** Pineconeは任意の `String` idをネイティブに受け付けるため、`VectorItem::id` はそのまま素通りします。metadataは最初から最後までJSONとして運ばれます - `PineconeVectorDriver::metadata_from_json` / `metadata_to_json` は、metadataがオブジェクトかnullであるというフレームワーク自身の規則だけを強制します。Pinecone自体は、metadataの*値*を文字列、数値、真偽値、文字列のリストに限定し、ネストしたオブジェクトをサーバー側で拒否しますが、Pineconeの規則はバージョン管理されており、ローカルのコピーはずれてしまうため、ドライバーはそのチェックを再実装していません。

**バッチの上限。** Pineconeは、upsertあたり最大1000ベクトル、deleteあたり最大1000idsを文書化しています。ドライバーは、無言でチャンクに分けるのではなく、渡されたものを1回のリクエストで送信します - 部分的に成功した書き込みは、拒否された書き込みよりも判断が難しいからです。この上限を超える場合は、あなたの側でバッチしてください。

**名前空間。** 1つのドライバーインスタンスは1つの名前空間にバインドされます。同じインデックスの複数の名前空間を使うには、名前空間ごとに、異なるストア名で1つのドライバーを登録してください:

```rust
Vector::register("docs-public", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("public")
));
Vector::register("docs-private", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("private")
));
```

**スループット。** 何も直列化されません。ドライバーは、コネクションハンドルではなくインデックスごとのホスト文字列をキャッシュし、リクエストは `reqwest` のコネクションプールを共有します - そのため、同じインデックスへの並行呼び出しは並行に進みます。（これが置き換えたgRPCドライバーは、名前ごとに1つの `Index` を `tokio::Mutex` の裏に保持していました。`pinecone-sdk` が `Index` を `&mut self` の裏でしか公開していなかったためです。）

**逃げ道。** `control_plane_get`、`control_plane_post`、`data_plane_post` は、あなた自身のリクエスト型とレスポンス型で、ドライバーの認証済みでホスト解決済みのトランスポートを介して、Pineconeが出荷するあらゆるエンドポイントに届きます - フィルタ式、スパースベクトル、idによるフェッチ、`/vectors/list`、インデックス管理など:

```rust
#[derive(serde::Deserialize)]
struct FetchResponse { vectors: Vec<suprnova::vector::PineconeVector> }

let hits: FetchResponse = driver.data_plane_post(
    "docs",
    "/vectors/fetch_by_metadata",
    &serde_json::json!({ "filter": { "genre": { "$eq": "comedy" } }, "limit": 2 }),
).await?;
```

**テスト。** 通信契約のテストはこのフィーチャーの下でデフォルトで実行されます: ローカルのフェイクに対してドライバーを駆動し、実際に送信するメソッド、パス、ヘッダー、JSONボディを検証します。それらは、ドライバーをPineconeの*文書化された*契約に固定します。文書が実サービスと一致していることを確認するには、両方の環境変数を必要とする `#[ignore]` 済みの統合テストが必要です:

```bash
PINECONE_API_KEY=... PINECONE_TEST_INDEX=my-test-index \
    cargo test -p suprnova --features vector-pinecone \
    --test vector_pinecone -- --ignored
```

### MariaDB - `MariaDbVectorDriver`

直接の `sqlx::MySqlPool` を介して、MariaDBネイティブの `VECTOR(N)` カラム型とHNSWインデックスを使い、MariaDB 11.7+と話します。ドライバーのメソッドを最初に呼んだとき、`SELECT VERSION()` を実行し、11.7未満のものはすべて拒否します - それより古いサーバーには、ベクトル関数がありません。

```rust
use std::sync::Arc;
use suprnova::{MariaDbDistance, MariaDbVectorDriver, Vector};

let driver = MariaDbVectorDriver::from_url(
    "mysql://user:pass@localhost:3306/myapp",
)?
.with_distance(MariaDbDistance::Cosine);  // デフォルト

Vector::register("documents", Arc::new(driver));
```

`from_url` は遅延評価です - URLの構文を検証しますが、最初の使用まで接続は開きません。そのため、データベースに到達できるようになる前でも、アプリのブートストラップ時にこれを呼ぶのは安全です。カスタムのプールオプションが必要なときは、`MariaDbVectorDriver::from_pool(pool)` で既存のプールをラップしてください。

**スキーマはあなたのものです。** ドライバーはテーブルを自動作成しません - スキーマはマイグレーションの関心事です。推奨される経路は `driver.ensure_table_sql_for(name, dim)` です。これはドライバーの設定済みの距離指標を継承するため、マイグレーションの `DISTANCE=` 句と、クエリ関数 `similar` が使う指標は、一致することが保証されます:

```rust
let driver = MariaDbVectorDriver::from_url(url)?
    .with_distance(MariaDbDistance::Cosine);

let sql = driver.ensure_table_sql_for("documents", 1536)?;
// 結果:
// CREATE TABLE IF NOT EXISTS `documents` (
//   id VARCHAR(255) NOT NULL PRIMARY KEY,
//   embedding VECTOR(1536) NOT NULL,
//   metadata JSON NULL,
//   VECTOR INDEX (embedding) DISTANCE=cosine
// ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
```

ドライバーがスコープに入っていないマイグレーションジェネレーター（CLIツール、ビルドスクリプト）のためには、静的な `MariaDbVectorDriver::ensure_table_sql(name, dim, distance)` を使い、後でドライバーに設定するのと同じ `MariaDbDistance` を渡してください。

**距離は両端で一致しなければなりません。** クエリ時に使われる関数がインデックスの `DISTANCE=` 句と一致しないとき、MariaDBは無言でテーブル全体のスキャンにフォールバックします。ドライバーは、2つの層でこれを防ぎます:

1. **`ensure_table_sql_for(name, dim)`** は、発行されるマイグレーションSQLと `similar` のランタイム関数の両方に対して `self.distance` を読みます - 構造上、両者がずれることはあり得ません。
2. **最初の `similar` 呼び出し時のランタイムチェック** は、ストアごとに1回 `SHOW CREATE TABLE` を実行し、実際のスキーマから `DISTANCE=` 句をパースし、`with_distance(...)` と食い違っていればはっきりとエラーにします。結果はキャッシュされるため、以降の呼び出しはゼロコストです。これは、`ensure_table_sql_for` を経由しない手書きのマイグレーションや `from_pool` のセットアップを捕まえます。

**ストア名の安全性。** ストア名は発行されるSQLに埋め込まれます（MySQLは識別子をパラメータ化しません）。名前は `[A-Za-z_][A-Za-z0-9_]*` かつ長さ64以下として検証され、検証済みの名前はすべての文を通じてバックティックで囲まれます。無効な名前は、`register`/`upsert`/`similar`/`delete`/`count` の境界で `FrameworkError::param` としてエラーになります。

**IDとmetadata。** `VARCHAR(255)` は任意の `String` idを受け付けます - UUIDの導出も、予約されたpayloadキーもありません。metadataはMariaDBの `JSON` カラム型を介して往復します。`null` のmetadataはSQLの `NULL` として保存されます。オブジェクトでないmetadata（配列、プリミティブ）は、QdrantおよびPineconeとの整合性のため、`FrameworkError::param` で拒否されます。

**スコアの正規化。** MariaDBは生の*距離*を返します（低いほど近い）。トレイトの契約は*スコア*です（高いほど類似している） - ドライバーは指標ごとに変換します:

| 指標 | MariaDBが返す値 | 公開される `score` |
| --------- | --------------------- | ---------------------------- |
| Cosine | `[0, 2]`（`1 - cos`） | `1.0 - d / 2.0` → `[0, 1]` |
| Euclidean | `[0, ∞)` L2ノルム | `1.0 / (1.0 + d)` → `(0, 1]` |

いずれの場合も、順位付けは保たれます（最良の結果が先頭）が、絶対的なスコアの値はドライバーをまたいで比較可能では**ありません** - 順序だけが比較可能です。それぞれのバックエンドは「高いほど良い」という規約に落ち着きますが、範囲は異なります: Memoryのcosineは `[-1, 1]` を返し、MariaDBの正規化されたcosineは `[0, 1]` を返し、Qdrantはネイティブのcosine類似度を `[-1, 1]` で発し、Pineconeはインデックスが作成された指標そのままの類似度を返します。`score` は単一のドライバーの結果セットの中で並べ替えるために使ってください。自分で再正規化せずに、ドライバーをまたいで数値スコアを比較しないでください。

**逃げ道。** `driver.pool()` は、トレイトがカバーしていない生のクエリのために、裏側の `sqlx::MySqlPool` を返します。`MariaDbVectorDriver::embedding_to_vec_text`、`score_from_distance`、`ensure_table_sql` は、直接のSQLとトレイト経由の呼び出しを混在させるときに独立して呼べる純粋関数です。

**一括upsertの振る舞い。** `upsert` は、500行のチャンクごとに1つの複数行の `INSERT ... VALUES (...), (...), ...` 文を発行し、すべてを単一のトランザクションでラップします。新しいコーパスを読み込むとき、ネットワークのラウンドトリップは行ごとの挿入に対して約500倍減ります。呼び出しはバッチ全体をまたいでアトミックなままです。バッチサイズは内部のものです - すべての項目を渡して `upsert` を一度呼べば、ドライバーがチャンク分けを処理します。

**HNSWインデックスはコミット時に再構築されます。** MariaDBは、行が入るにつれてHNSWグラフを更新しますが、インデックスの作業はコミット時に集中します。100万行の `upsert` は、インデックス構築の全期間にわたってトランザクションを開いたままにし、それは数分に及ぶことがあります。非常に大きな初期ロードには、コーパスを1万〜10万行のバッチに分割し、`upsert` を繰り返し呼んでください。そうすれば、各バッチがコミットし、ラウンド間でロックを解放します。（`upsert` の呼び出しを小さくしても、行あたりが遅くなるわけではありません - インデックスの作業がより多くのコミットポイントに分散されるだけです。）

**次元はテーブル作成時に固定されます。** `VECTOR(N)` は次元を固定します。768次元のモデルから1536次元のモデルへ埋め込みモデルを切り替えるということは、テーブル全体のマイグレーション（新しいテーブル、再埋め込み、入れ替え）を意味します。モデルのアップグレードは、スキーマのマイグレーションを計画するのと同じやり方で計画してください - 「ALTER COLUMN VECTOR(768) → VECTOR(1536)」という経路は存在しません。

**プールのサイジング。** `from_url` は、sqlxのデフォルトの `MySqlPoolOptions` を使います - この文章を書いている時点では `max_connections = 10` です。高QPSのワークロード（1秒あたり数百回の `similar` 呼び出し）には、`MySqlPoolOptions::new().max_connections(N).connect_lazy(url)` で自分でプールを構築し、`from_pool` へ渡してください。ドライバーは、自分自身のコネクション上限を課しません。

**ローカルセットアップ。** Dockerを介してMariaDB 11.7+を実行します:

```bash
docker run -p 3306:3306 \
    -e MARIADB_ROOT_PASSWORD=secret \
    -e MARIADB_DATABASE=vectors \
    mariadb:11.7
```

統合テストは次で実行されます:

```bash
MARIADB_URL='mysql://root:secret@localhost:3306/vectors' \
    cargo test -p suprnova --test vector_mariadb -- --ignored
```

## ドライバーの比較

| 項目 | Memory | Qdrant | Pinecone | MariaDB |
| --- | --- | --- | --- | --- |
| バックエンド | `HashMap` | Qdrant gRPC | Pinecone REST | MariaDB SQL |
| 永続性 | なし | あり | あり | あり |
| 自動作成 | 対象外 | あり（設定可能） | なし（利用者がインデックスを作成） | なし（マイグレーションは自分で行う） |
| 文字列ID | ネイティブ | UUID-5へハッシュ化 | ネイティブ | ネイティブ |
| 予約されたmetadataキー | なし | `__suprnova_id` | なし | なし |
| スループット | プロセスごと | 並行 | 並行（プール制限あり） | 並行（プール制限あり） |
| 距離指標 | Cosine | 設定可能 | インデックス作成時に設定 | Cosine / Euclidean |
| バージョン要件 | - | 任意 | 任意 | **11.7+** |

## 運用上の注意

**ストア名の規約。** `Vector::register` と `Vector::store` に渡されるストア名はラベルです - 任意の文字列にできます。Qdrantでは、フレームワークはそれをコレクション名として使い、Pineconeではインデックス名として使います。ラベルは、バックエンドの既存の命名スキームに合わせてください。

**再登録。** 新しいドライバーインスタンスで名前を再登録するのは、設計上、後勝ちの操作です - プロセスを再起動せずにテストハーネス内でドライバーを入れ替えるのに便利です。

**テストの分離。** Memoryとレジストリベースのドライバーのテストはどちらも、並行したテスト実行下での衝突を避けるために、タイムスタンプ付きの一意なストア名を使います。

**エラーの意味論。** `Vector::store(name)` は、未登録の名前に対して `FrameworkError::not_found` を返します。ドライバーレベルの失敗（ネットワーク、認証、次元の不一致）は、表示メッセージに原因の文字列を伴った `FrameworkError::internal` または `FrameworkError::param` として返ってきます。

## 拡張する

5番目のバックエンド（Weaviate、Milvus、LanceDB、pgvector、LibSQLなど）を追加するには:

1. `VectorDriver` を実装する新しい `framework/src/vector/<backend>.rs` を追加します。
2. ドライバー型を `framework/src/vector/mod.rs` とクレートルートから再エクスポートします。
3. Pineconeのテスト分割を手本にします: 純粋関数のテストと（ローカルの `wiremock` フェイクに対する）通信契約のテストは常に実行されます。統合テストは、認証情報のための環境変数の裏で `#[ignore]` によってゲートされます。中間の層こそが、その存在価値を発揮する層です - CIから誰も到達できないバックエンドにも、タイプミスで壊れうる通信の形はあります。

トレイトは意図的に小さく保たれているため、新しいドライバーを出荷するための基準は低いままです。バックエンドが、収まらない表面（フィルタ式、スパースベクトル、ハイブリッド検索）を必要とする場合は、トレイトを肥大化させるのではなく、ドライバー上の逃げ道を通じてそれを公開してください。

## 次のステップ

- [デプロイメント](deployment.md) - MariaDBをデフォルトの本番環境とする推奨の背景
- [データベース](database.md) - マルチドライバーのSeaORM設定。ベクトルと並ぶリレーショナルバックエンドとしてのMariaDBを含みます
- [環境変数](env-vars.md) - `QDRANT_URL`、`PINECONE_API_KEY`、`MARIADB_URL`、その他のドライバー用環境変数の契約
- [キャッシュ](cache.md) - 同じドライバートレイトの形をした姉妹ファサード
- [Laravel パリティ マップ](parity.md) - ベクトル検索がScoutに対してどこに位置するか

# Eloquent シリアライゼーション

Eloquentモデルが、どのようにJSONへ変わるかを説明します。この章は、`to_array()` と `to_json()`、`hidden` / `visible` / `appends` のフィルタパイプライン、2つの終端ヘルパー `to_array_except` / `to_array_only`、appendsがどのようにアクセッサーを出力へ橋渡しするか、そしてLaravelからの、人を引っかける2つの相違点 - serdeバイパスのフットガンと、イーガーロードされたリレーションがJSONボディへ自動的に畳み込まれないという事実 - をカバーします。

[Eloquent API](eloquent.md)を読んだことがあれば、ここに出てくる名前の大半はおなじみのはずです - 属性のリファレンスはあの章にあります。このページが、*シリアライゼーションの契約*が置かれている場所です: どのフィールドが現れるか、どの順序でフィルタが適用されるか、そして忘れると何が漏れるか、です。

## 目次

- [契約](#契約)
- [`to_array` と `to_json`](#to-array-と-to-json)
- [フィールドを隠す - `hidden = [...]`](#フィールドを隠す-hidden)
- [フィールドを許可リストで絞り込む - `visible = [...]`](#フィールドを許可リストで絞り込む-visible)
- [アクセッサーを追加する - `appends = [...]`](#アクセッサーを追加する-appends)
- [フィルタパイプラインの順序](#フィルタパイプラインの順序)
- [呼び出しごとのフィルタリング - `to_array_except` / `to_array_only`](#呼び出しごとのフィルタリング-to-array-except-to-array-only)
- [閲覧者に応じた条件付きの非表示](#閲覧者に応じた条件付きの非表示)
- [serdeバイパスのフットガン](#serdeバイパスのフットガン)
- [コレクションをシリアライズする](#コレクションをシリアライズする)
- [イーガーロードされたリレーションとシリアライゼーション](#イーガーロードされたリレーションとシリアライゼーション)
- [JSON:APIについてはどうか](#json-apiについてはどうか)
- [各要素の実装場所](#各要素の実装場所)
- [次のステップ](#次のステップ)

## 契約

すべての `#[suprnova::model]` 構造体は、`Model` トレイトから2つのシリアライゼーションメソッドを得ます:

```rust
fn to_array(&self) -> serde_json::Value;
fn to_json(&self) -> String;
```

`to_array` は、ハンドラのレスポンスやテストで使うための `serde_json::Value` を生成します。`to_json` は薄いラッパーです - `serde_json::to_string(&self.to_array())` - そのため、1つのフィルタパイプラインが両方の形を所有します。

出力は、構造体のフィールド名（またはあなたが適用したserdeのリネーム）をキーとするJSONオブジェクトであり、`#[model(...)]` で宣言される3つの任意のノブを通じてフィルタされます:

- `hidden = [...]` - カラムの拒否リスト
- `visible = [...]` - カラムの許可リスト（`hidden` とは排他的）
- `appends = [...]` - 名前付きのキーの下に注入するアクセッサーメソッド

モデルがこれらのどれも宣言していない場合、トレイトのデフォルトの本体が実行されます: `serde_json::to_value(self)` を介して `self` をシリアライズし、フレームワーク内部の2つのスクラッチフィールド（`__eager` と `__pivot` - [イーガーロードされたリレーション](#イーガーロードされたリレーションとシリアライゼーション)を参照）を取り除き、結果を返します。モデルがそれらのどれかを宣言している場合、マクロは[パイプライン](#フィルタパイプラインの順序)を実行するオーバーライドを発行します。

## `to_array` と `to_json`

最小限の実用例 - 1行をJSONとして送り出す:

```rust
use suprnova::{json_response, model, Model, Request, Response};
use chrono::{DateTime, Utc};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    json_response!(user.to_array())
}
```

`json_response!` はあらゆる `serde_json::Value` を受け付けます。`user.to_array()` はそれを1つ生成します。文字列版の相当物は `user.to_json()` です - 同じボディ、同じフィルタで、余分な `to_string` が1つ増えるだけです。

`serde_json::to_value(&user)` を直接使うこともできます。**ユーザー向けの何かに対しては、それをしないでください。** フィルタパイプラインを完全にバイパスしてしまいます - 理由については、この章の後半にある[serdeバイパスのフットガン](#serdeバイパスのフットガン)を参照してください。

## フィールドを隠す - `hidden = [...]`

拒否リスト形式です。リストに載っていないすべてのカラムがシリアライズされます:

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

このモデルのユーザー向けJSONには、`password` や `remember_token` が決して含まれません:

```json
{
    "id": 42,
    "name": "Alice",
    "email": "alice@example.com",
    "created_at": "2026-05-30T11:14:22Z",
    "updated_at": "2026-05-30T11:14:22Z"
}
```

`hidden` は、大半のフィールドが**レスポンスとして送出され**、シークレットや内部フラグ、認証専用のデータといった小さな集合だけを差し引く必要があるときに、正しい道具です。

## フィールドを許可リストで絞り込む - `visible = [...]`

許可リスト形式です。リストに載っているカラムだけがシリアライズされます:

```rust
#[model(
    table = "users",
    visible = ["id", "name", "avatar_url"],
)]
pub struct PublicUserView { /* ... */ }
```

薄い公開用の射影として存在することに特化したモデルに便利です（Laravelの「Profile」/「PublicUser」型を思い浮かべてください）。テーブルが何十もの内部カラムを持ち、レスポンスに乗せるべきものがほんの数個しかない場合も、`visible` が正しい道具です - 残す集合を列挙する方が、取り除く集合を列挙するより短くなります。

`hidden` と `visible` は、**コンパイル時に互いを排除します**。両方を設定すると、マクロはエラーを発します:

```text
error: cannot specify both `hidden` and `visible` on the same model
 --> src/models/user.rs:7:1
  |
7 | #[model(table = "users", hidden = ["x"], visible = ["y"])]
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

この2つはポリシー上の正反対です - あなたのモデルの形に意図が合う方を選んでください。両方ではありません。

## アクセッサーを追加する - `appends = [...]`

`appends` は、計算された値をJSON出力へ注入します。各エントリは、モデル上の `#[accessor]` タグ付きメソッドを名前で指定します。マクロは `to_array()` の実行中にそれを呼び出し、戻り値を同じキーの下に格納します。

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    fillable = ["first_name", "last_name"],
    appends = ["full_name", "initials"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[accessor]
    pub fn initials(&self) -> String {
        let f = self.first_name.chars().next().unwrap_or(' ');
        let l = self.last_name.chars().next().unwrap_or(' ');
        format!("{f}{l}")
    }
}
```

シリアライズされたユーザーは、これで両方の計算済みキーを運びます:

```json
{
    "id": 7,
    "first_name": "Alice",
    "last_name": "Pond",
    "created_at": "...",
    "updated_at": "...",
    "full_name": "Alice Pond",
    "initials": "AP"
}
```

マクロは、コンパイル時に `appends` のエントリを検証します:

- 各名前は、Rustの識別子としてパースできなければなりません（`"full-name"` は失敗します - 有効な識別子ではないからです）。
- 名前が指定されたメソッドがモデルの `impl` ブロック上に存在しない場合、コンパイラはマクロが生成したディスパッチャーを指し示し、`no method named 'full_name' found` という明確なエラーを出します。

Rustから直接 `user.full_name()` を呼び出すのは、他のどんなメソッドともまったく同じように機能します - `appends` が制御するのは**JSONディスパッチテーブル**だけです。アクセッサーは、普通のメソッドのままです。

## フィルタパイプラインの順序

モデルが `hidden`、`visible`、`appends` のいずれかを宣言すると、マクロは、この順序で四つのステップを実行する `to_array` のオーバーライドを発行します:

1. `serde_json::to_value` を介して、`self` を `serde_json::Map` へシリアライズします。
2. フレームワーク内部の `__eager` と `__pivot` のキーを、無条件に取り除きます（詳しくは[リレーションのセクション](#イーガーロードされたリレーションとシリアライゼーション)を参照してください）。
3. `visible` が空でない場合、それを**許可リスト**として適用します: リストに載っていないキーはすべて削除されます。
4. `hidden` を**拒否リスト**として適用します: 許可リストを生き延びた、リストに載っているキーはすべて削除されます。
5. `appends` を注入します: 各エントリについて、登録済みのアクセッサーを呼び出し、その結果をエントリの名前の下に挿入します。

### Suprnovaが異なる設計を選んだ理由

Laravelも、同じ `hidden` → `visible` → `appends` の順序で実行します。相違点はステップ5にあります: Suprnovaでは、appendsは `hidden` の拒否リストの**後に**実行され、その名前が `hidden` にも載っていたとしても、常に現れます。この理屈はLaravelと同じです: `$appends = ['full_name']` と `$hidden = ['full_name']` の両方を宣言した場合、意図は「計算して送り出す」ことであり、`appends` の方がより具体的な信号なのです。この順序が重要になるのは、アクセッサーのキーがカラム名と衝突するとき（例えば、保存された `display_name` カラムの値を上書きするアクセッサー）です - アクセッサーがレスポンス上で勝ちます。

## 呼び出しごとのフィルタリング - `to_array_except` / `to_array_only`

カラム宣言が合わない一度限りのケースのために、2つの終端ヘルパーが、`to_array` のフルパイプラインを実行してから、名前で結果を切り詰めます:

```rust
use suprnova::{json_response, Model};

pub async fn admin_show(user: User) -> suprnova::Response {
    // その行の大半を必要とするが、これらは不要な管理者向けエンドポイントのために、
    // 追加のフィールドをいくつか取り除く:
    json_response!(
        user.to_array_except(&["password_hash", "remember_token", "internal_notes"])
    ))
}

pub async fn directory_show(user: User) -> suprnova::Response {
    // 公開ディレクトリ - 公開したいカラムだけを:
    json_response!(
        user.to_array_only(&["id", "name", "avatar_url"])
    ))
}
```

どちらも `serde_json::Value` を生成します - `self` を変更することはなく、同じ行の将来のシリアライゼーションを変えることもありません。まず `hidden` / `visible` / `appends` のフルパイプラインを実行し、その上に自分自身の切り詰めを適用します。`to_array_only` は、指定されたキーだけを含む*新しい*JSONオブジェクトを返します。`to_array_except` は、指定されたキーを除いた完全なオブジェクトを返します。

### Suprnovaが異なる設計を選んだ理由

Laravelの `$user->makeHidden(['x'])` と `$user->makeVisible(['x'])` は、モデルのインスタンスを**変更します** - それ以降のすべての `toArray()` 呼び出しは、モデルが親のシリアライゼーションの内側にネストされているときに起こる呼び出しも含めて、変更された状態を目にします。Suprnovaのヘルパーは**終端です**。`Value` を生成して、そこで止まります。変更を伝播させる必要がある場合は、`#[model(hidden = [...])]` / `#[model(visible = [...])]` の上で宣言してください。そうすれば、インスタンス上の隠れた変更ではなく、*型*がそのポリシーを表現します。

Rustらしい理由はこうです: SuprnovaにおけるEloquent構造体は、実行時の属性バッグを持たない、ただのRust構造体です。周囲に隠れた状態を追加せずに、インスタンス側の可視性フラグを住まわせる場所がありません - それこそが、フレームワークが意図的に避けているフットガンの一種です。

## 閲覧者に応じた条件付きの非表示

可視性が閲覧者に依存する場合のイディオマティックなパターンは、呼び出し箇所での `match` であり、正しい呼び出しごとのフィルタへ分岐します:

```rust
use suprnova::{Auth, json_response, Model, Request, Response};

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    let viewer = Auth::user_as::<User>().await?;
    let viewing_self = viewer.as_ref().map(|v| v.id) == Some(user.id);

    let body = if viewing_self {
        user.to_array()
    } else {
        user.to_array_except(&["email", "phone", "stripe_customer_id"])
    };

    json_response!(body)
}
```

管理者、トライアルユーザー、有料ユーザーで異なる属性を持つような、閲覧者ごとのより入念な形が必要な場合、正しい道具は `Maybe<T>` / `MissingValue<T>` フィールドを備えた**JSON:APIリソース層**です。宣言的な形については、[JSON:API リソース](eloquent-resources.md#conditional-attributes--maybet--missingvaluet)を参照してください。

## serdeバイパスのフットガン

これは、SuprnovaにおけるEloquentのシリアライゼーションについて知っておくべき、最も重要な1つのことです。

**`hidden` / `visible` / `appends` のフィルタは、`to_array()` と `to_json()` を通じてのみ実行されます。** これらは、導出された `Serialize` のimplによっては強制*されません*。構造体を他のどんなserde経路で返しても、フィルタは完全にバイパスされます。

つまり、**次のすべてが `password` を漏らします**:

```rust
// 直接のserde - to_arrayをバイパスし、hiddenは効果を持たない:
let raw = serde_json::to_value(&user).unwrap();

// 構造体フィールドを伴うjson_response! - 同様:
json_response!({ "user": user }))

// 別のシリアライズ可能なコンテナの内側にネスト - 同様:
#[derive(Serialize)]
struct EnvelopeWithUser { ok: bool, user: User }
let env = EnvelopeWithUser { ok: true, user };
json_response!(env))

// serde経由でVec<User>を返す - 同様:
json_response!(users))   // usersはVec<User>
```

フィルタパイプラインを通るのは、これらだけです:

```rust
json_response!(user.to_array()))
json_response!(users_collection.to_array()))  // Collection<User>
json_response!(user.to_array_except(&["secret"])))
json_response!(user.to_array_only(&["id", "name"])))
```

### なぜこれが起きるのか

serdeの `Vec<T>`（および他のあらゆるコンテナ）に対する全面的な `Serialize` は、`T::serialize` を直接呼び出します。Suprnovaのフィルタパイプラインは、`Serialize` の中にではなく、`Model::to_array` というトレイトメソッドの中に存在します。そのトレイトメソッドは、あなたが呼び出さない限り呼び出されません。

フレームワークは、*内部の*フットガンに対しては保護機構を持っています（`__eager` / `__pivot` のスクラッチフィールドは `#[serde(skip)]` とマークされているため、どちらの経路からも漏れません）。しかし、マクロは、隠しフィールドに対して `#[serde(skip_serializing)]` を発行することを意図的に**しません** - そうすると、呼び出し元が行全体を必要とする場合（例えば内部RPC、永続化層、診断、テスト）における、内部のSeaORMモデルに対するserdeの正当な使い方を壊してしまうからです。

### 原則

クライアントへ戻る信頼境界を越えるあらゆる値については、`to_array()` か、そのフィルタ済みの類縁のどれかを経由させてください。安全を買う、四行の契約です:

| したいこと | 使うもの | 結果 |
|---|---|---|
| 1つのモデルをシリアライズする | `user.to_array()` | フィルタ済みのJSONオブジェクト |
| コレクションをシリアライズする | `collection.to_array()` | フィルタ済みのJSON配列 |
| いくつかのフィールドを差し引く | `user.to_array_except(&["x"])` | フィルタ済み + 差し引き済み |
| いくつかのフィールドだけを残す | `user.to_array_only(&["x"])` | リストに載ったキーだけ |

モデルの値に対する `json_response!\({.*: [a-z_]+ ?})` と `serde_json::to_value\(&\w+\)` のリンタ、あるいはPR時点でのレビューは、この原則を守るための安価な方法です。フレームワーク自身の `Model` シリアライゼーションのテストは、両方の経路をカバーしています。

## コレクションをシリアライズする

`Collection<M>`（`Builder::get()`、`Model::all()`、そしてリレーションのアクセッサーが返すもの）は、自分自身の `to_array()` と `to_json()` を持ち、背後にある `Vec<M>` を走査して、**行ごとに** `to_array()` を呼び出します。結果は、フィルタ済みオブジェクトのJSON配列です:

```rust
use suprnova::{json_response, Model};

pub async fn list() -> suprnova::Response {
    let users = User::all().await?;
    json_response!(users.to_array())
}
```

複数行の結果に対して、行ごとのフィルタを得られる場所は、これだけです。`serde_json::to_value(&users)` は、serdeの全面的なimplを介してVecを出力し、すべての行のフィルタを一度にバイパスしてしまいます - コレクションレベルのヘルパーは、まさにそのギャップを塞ぐために存在します。

```rust
// Collection<M>のオーバーライド:
pub fn to_array(&self) -> Value {
    Value::Array(self.0.iter().map(|m| m.to_array()).collect())
}
```

ページネーターの場合、包まれたデータは `LengthAwarePaginator::data` / `CursorPaginator::data` に存在し、`Vec<M>` です - ページネーターのレスポンスを組み立てる前に各項目へ `.to_array()` を呼び出すか、あるいは、リソースパイプラインの一部として行ごとのフィルタリングを処理する[JSON:APIのページネーション形式](eloquent-resources.md#pagination)を使ってください。

## イーガーロードされたリレーションとシリアライゼーション

これが、身につけるべき2つ目の相違点です。

ビルダー上で `.with(["posts"])` を呼び出すと、フレームワークはpostsをロードし、行ごとの `EagerLoadCache`（自動的に注入される `__eager` フィールド）へそれらを格納します。それらを読み取るためのアクセッサー - `user.posts_loaded()` - は、そのキャッシュから取り出します。

**そのキャッシュは `#[serde(skip)]` であり、`to_array()` は無条件にそれを取り除きます。** イーガーロードされたリレーションは、JSON出力へ自動的に畳み込まれません。postsをイーガーロードしたuserに対する `to_array()` は、していないuserに対する `to_array()` と、見た目がまったく同じです。

### Suprnovaが異なる設計を選んだ理由

Laravelの `toArray()` は `$model->getRelations()` を走査し、ロード済みのすべてのリレーションを出力へ畳み込みます。PHPの配列形のモデルバッグは、これを自然なものにします - リレーションは、モデル上の、もう1つのキー付きエントリにすぎません。

Rustの型付けされたEloquent構造体は、そのバッグを持ちません。`User` 構造体が持つのは型付けされたカラムであり、「ロードされていたリレーションが何であれ」を保持する異種混合のマップではありません。`posts` を畳み込むには、型付けされた構造体への実行時のフィールド注入（serdeをバイパスする仕組み）か、カラムのシリアライザーを実行した後にキャッシュを参照する並行のシリアライゼーション経路のどちらかが必要になります。どちらの選択肢も、あらゆるモデルのJSONの形を、特定の呼び出し元がイーガーロードしたリレーションへ結合してしまいます - これはPHPでは、クライアントがそれに依存することを学ぶために構造の要をなす契約ですが、Suprnovaが明示的に出荷を拒む契約です。なぜなら、それはJSONの形を呼び出し元側のクエリ構築に依存させてしまうからです。

### リレーションのデータを届ける2つの方法

**1. 明示的なアクセッサー + appends。** `<rel>_loaded()` から取り出すメソッドを定義し、`appends` に登録してください。リレーションは、あなたが名付けたどのキーの下にも現れます。これがうまくいくのは、リレーションが読み取りパス上で*常に*イーガーロードされる場合です:

```rust
use suprnova::{accessor, model};
use serde_json::Value;

#[model(
    table = "users",
    appends = ["posts"],
)]
pub struct User { /* ... */ }

impl User {
    #[accessor]
    pub fn posts(&self) -> Value {
        // 読み取りパス上で .with(["posts"]) が呼ばれていなければ、posts_loaded() は
        // パニックする。アクセッサーは、イーガーロードの後に実行されなければならない。
        let posts = self.posts_loaded();
        serde_json::to_value(posts).unwrap_or(Value::Null)
    }
}

// 読み取りパスは、イーガーロードしなければならない:
let users = User::query()
    .with(["posts"])
    .get()
    .await?;
let body = users.to_array();   // 各userの`posts`キーが埋まる
```

この契約は黙っていません: `.with(["posts"])` を忘れると、アクセッサーは最初の行の `posts_loaded()` 呼び出しでパニックします（設計上、リレーションがロードされていないときに読み取られると、イーガーキャッシュはパニックします - サイレントな空の配列は、バグを隠してしまうからです）。任意のイーガーロードには、`Option<&T>` を返し、`match` を使わせてくれるHasOne形式を使ってください:

```rust
impl User {
    #[accessor]
    pub fn profile(&self) -> Value {
        match self.profile_loaded() {
            Some(profile) => serde_json::to_value(profile).unwrap_or(Value::Null),
            None => Value::Null,
        }
    }
}
```

**2. JSON:APIリソース層。** リレーションの形と包含のポリシーが、モデルではなく通信上の形式に属すべき場合は、リレーションシップフィールドに `#[data(allow_include)]` を付けた `#[derive(Data)] #[json_resource]` 構造体を使ってください。クライアントは `?include=posts.comments` を通じて選択し、フレームワークはincludeツリーを走査して、重複を除いたリソースオブジェクトで `included` を満たします。これが正しい答えになるのは、次の場合です:

- リレーションの形が、通信上の形式の関心事である場合（スパースフィールドセット、条件付きの包含、クロスリンクのメタデータ）。
- 異なるエンドポイントが、異なるデフォルトの包含を求める場合。
- 同じモデルが、異なるエンベロープの下に現れる場合（あるエンドポイントは `posts` を送り出し、別のエンドポイントは `subscriptions` を送り出す）。

完全なパターンについては、[JSON:API リソース](eloquent-resources.md#compound-documents--include-chains)を参照してください。

## JSON:APIについてはどうか

`to_array()` のパイプラインと、`Resource` / `JsonApi` のファサードは、2つの層であり、それぞれ異なる仕事を担っています:

| 関心事 | `Model::to_array` | `Resource::single` / `JsonApi::single` |
|---|---|---|
| **形** | フラットなオブジェクト - カラム名が直接キーへ写される | JSON:APIエンベロープ（`data`、`included`、`meta`、`links`、`jsonapi`） |
| **属性ごとの制御** | `#[model]` 上の `hidden` / `visible` / `appends` | `#[data(input_only)]`、`Maybe<T>`、`?fields[type]=` によるスパースフィールドセット |
| **リレーション** | 手動（アクセッサー + appends、上記を参照） | `#[data(allow_include)]` + `?include=` によるファーストクラスの対応 |
| **ページネーション** | `Vec<Value>` を手作業でラップする | `Resource::paginated(p)` がlinks + metaを処理する |
| **エラー** | `FrameworkError` を通じて描画する | `into_json_api_response()` がJSON:APIの `errors` エンベロープを生成する |
| **いつ使うか** | 単純なエンドポイント、内部ツール、その場限りの形 | 公開API、サードパーティの消費者、JSON:APIを意識したクライアント |

`to_array()` は下位の層です - 大半の内部ハンドラ、管理者向けページ、（serde経由の）Inertiaのprops、そしてテストで呼ばれるのは、これです。JSON:API層は、その上に重なります: `to_array` を置き換えるのではなく、モデル自体に置くには豊かすぎる、リソースごとの属性／リレーションシップのロジックの周りに、エンベロープを追加するのです。

型付けされたInertiaのpropsについては、ほぼ常に、モデルを直接serdeに通すのではなく、リソース層か、明示的なフィールドを持つ専用の `#[derive(Serialize)]` のDTOを使いたくなるはずです。Inertiaの戻り値も、他のあらゆるものと同じserdeバイパスの扱いを受けます - 安全な経路は「DTOを組み立て、`to_array()` からそれを満たし、DTOを返す」ことです。

## 各要素の実装場所

| 要素 | ファイル |
|---|---|
| `Model::to_array` / `to_json` トレイトのデフォルト | `framework/src/eloquent/model.rs` |
| `Model::to_array_except` / `to_array_only` | `framework/src/eloquent/model.rs` |
| `Model::__append_accessor` トレイトのデフォルト | `framework/src/eloquent/model.rs` |
| マクロが発行する `to_array` のオーバーライド（フィルタパイプライン） | `suprnova-macros/src/model/serialization.rs` |
| マクロが発行する `__append_accessor` ディスパッチャー | `suprnova-macros/src/model/serialization.rs` |
| `Collection<M>::to_array` / `to_json` | `framework/src/eloquent/collection.rs` |
| `EagerLoadCache`（`__eager` フィールド） | `framework/src/eloquent/relations/eager_cache.rs` |
| `hidden` / `visible` / `appends` のマクロパース | `suprnova-macros/src/model/parse.rs` |
| `#[accessor]` 関数レベルのマクロ | `suprnova-macros/src/lib.rs` |

## 次のステップ

- [Eloquent API](eloquent.md) - フルのモデルの表面、属性のリファレンス、そして `#[accessor]` / `#[mutator]` が定義されている場所
- [JSON:API リソース](eloquent-resources.md) - より豊かな閲覧者ごとの形、スパースフィールドセット、そして複合的な `?include=` ドキュメントのための、宣言的なリソース層
- [バリデーション](validation.md) - リクエストの入力が、モデル層に見られる前に、どのように型付けされた構造体になるか
- [レスポンス](responses.md) - `HttpResponse` のビルダー、ヘッダー、クッキー。`json_response!` が最終的に生成する表面
- [エラー モデル](error-model.md) - エラーが、成功パスと同じ `request_id` の相関を持つJSONボディへどのように変わるか

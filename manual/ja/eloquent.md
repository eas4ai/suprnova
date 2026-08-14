# Eloquent API

Suprnovaの Eloquent 層は、Laravel 開発者に馴染みのあるAPIを、SeaORM の上の薄いラッパーとして実装して提供します。Laravel のドキュメントからコードをコピーし、PHP の構文を Rust に置き換え、`.await?` を追加すれば、それで動きます。

この層全体は、構造体アトリビュート（`#[suprnova::model]`）、トレイト（`Model`）、そして連鎖可能なクエリビルダー（`Builder<M>`）だけです - それだけです。舞台裏では、このマクロが SeaORM の `Entity`、`Model`、`ActiveModel`、そして `Column` のenumを生成し、さらにすべての Eloquentトレイトの実装を生成します。SeaORM の型は、Eloquent の表面が対応しきれないまれなケースのために、到達可能なままです（[SeaORMへのエスケープハッチ](#seaormへ降りる)を参照してください）。

## 目次

- [クイックスタート](#クイックスタート)
- [`#[suprnova::model]` アトリビュート](#the-suprnovamodel-attribute)
- [モデルモジュールのレイアウト](#モデルモジュールのレイアウト)
- [行を検索する](#行を検索する)
- [作成と更新](#作成と更新)
- [削除とソフトデリート](#削除とソフトデリート)
- [クエリビルダー - デュアルAPI](#query-builder--dual-api)
- [行ロック](#行ロック)
- [トランザクション](#トランザクション)
- [スコープ](#スコープ)
- [リレーションシップ](#リレーションシップ)
- [イーガーロード](#イーガーロード)
- [ページネーション](#ページネーション)
- [チャンクと遅延反復](#チャンクと遅延反復)
- [コレクション](#コレクション)
- [一括代入](#一括代入)
- [キャスト](#キャスト)
- [アクセッサーとミューテータ](#アクセッサーとミューテータ)
- [タイムスタンプ](#タイムスタンプ)
- [オブザーバーとライフサイクルイベント](#オブザーバーとライフサイクルイベント)
- [Prunable](#prunable)
- [マルチ接続ルーティング](#マルチ接続ルーティング)
- [複製](#複製)
- [デバッグ - dump と dd](#debugging--dump-and-dd)
- [モデルのテスト](#モデルのテスト)
- [SeaORMへ降りる](#seaormへ降りる)
- [`database::Model` からの移行](#migrating-from-databasemodel)
- [DBファサード - モデルレスなクエリ](#db-facade--model-less-queries)
- [Laravel-13パリティ - リレーション存在 + 手軽な近道](#laravel-13-parity--relation-existence--cheap-shortcuts)

## クイックスタート

構造体に付けるアトリビュートが1つあれば、それを、フル機能を備えたEloquentモデルにできます:

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

これを宣言すれば、次のように書けます:

- `User::query()` - 流れるようなクエリビルダーを開始します。
- `User::find(id).await?` - 主キーで取得します。
- `User::find_or_fail(id).await?` - 同じですが、見つからない場合は `ModelNotFound` でエラーになります。
- `User::all().await?` - すべての行です。
- `User::create(attrs!{ name: "Alice", email: "alice@example.com" }).await?` - 一括代入のフィルタリングを伴う挿入です。
- `User::filter("email", "alice@example.com").first().await?` - マッチする1行です。
- `user.update(attrs!{ name: "Alice B" }).await?` - 部分更新です。
- `user.save().await?` - メモリ上の変更を永続化します。
- `user.delete().await?` - その行を削除します。
- `user.refresh().await?` / `user.fresh().await?` / `user.replicate().await?` - Laravelのライフサイクルの残りです。

ユーザーに面した構造体（ここでは `User`）こそが、あなたのハンドラやコントローラーが運ぶ型そのものです。マクロは、モデルごとの内部モジュール（`user::`）を発行します - SeaORMへ直接降りたい場合のための、SeaORMの `Entity`、`Column`、`ActiveModel`、そして `Model` の型を伴います。この構造体は、インベントリに支えられた `ModelEntry` にも登録されるため、管理者向けコードやツール向けコードは、起動時にすべてのモデルを列挙できます。

## `#[suprnova::model]` アトリビュート

モデルを宣言するための、唯一のエントリーポイントです。すべてのアトリビュートはオプションです。デフォルトは、`id` + `created_at` + `updated_at` を持つ構造体が、一切の設定なしでSuprnovaのモデルとして動作するように調整されています。

### マクロアトリビュートのリファレンス

| アトリビュート | 型 | デフォルト | 注記 |
|-----------|------|---------|-------|
| `table` | 文字列 | 構造体名をスネークケース化した複数形 | テーブル名を上書きする |
| `primary_key` | 文字列 | `"id"` | PKカラム名を上書きする |
| `key_type` | 型 | `i64` | PKの型 - UUIDには `String`、レガシーなスキーマには `i32` |
| `auto_increment` | bool | `true` | UUIDのPKでは無効化する |
| `connection` | 文字列 | `"default"` | マルチコネクションのアプリは、デフォルト以外のコネクションに名前を付ける |
| `fillable` | 文字列のリスト | （デフォルト = `guarded = ["id"]`） | 一括代入の許可リスト |
| `guarded` | 文字列のリスト | どちらも設定されていない場合は `["id"]` | 一括代入の拒否リスト（`fillable` とは排他的） |
| `casts` | `field = CastType` のマップ | `{}` | カラムごとのキャスト |
| `hidden` | 文字列のリスト | `[]` | `to_json` / `to_array` から除外される |
| `visible` | 文字列のリスト | （すべて） | `hidden` の包含的な変種（排他的） |
| `appends` | 文字列のリスト | `[]` | シリアライゼーションに含めるアクセッサー |
| `soft_deletes` | フラグ | `false` | `deleted_at` カラム + トゥームストーンのセマンティクスを有効化する |
| `soft_deletes_column` | 文字列 | `"deleted_at"` | ソフトデリートのカラム名を上書きする |
| `timestamps` | フラグ / bool | `created_at` と `updated_at` の両方が存在する場合は `true` | 自動管理されるタイムスタンプを無効化する |
| `created_at` | 文字列 | `"created_at"` | カラム名を上書きする |
| `updated_at` | 文字列 | `"updated_at"` | カラム名を上書きする |
| `touches` | リレーション名のリスト | `[]` | パースされ、モデルのメタデータ（`TOUCHES` 定数）として格納される。列挙された親に対して `.touch()` を呼び出す保存後フックは、まだ配線されていない - 今のところは、あなたのオブザーバーやハンドラから明示的に `parent.touch().await?` を呼び出すこと。 |
| `mutators` | 文字列のリスト | `[]` | JSONによる充填の経路が `set_<field>(value)` というミューテータメソッドを経由するフィールド名 |

### 完全な例

```rust
use chrono::{DateTime, Utc};
use serde_json::Value as Json;
use suprnova::{model, AsBool, AsEncrypted, AsJson};

#[model(
    table = "users",
    fillable = ["name", "email", "preferences"],
    casts = {
        active = AsBool,
        preferences = AsJson<Json>,
        api_token = AsEncrypted,
    },
    hidden = ["password", "remember_token", "api_token"],
    appends = ["full_name"],
    soft_deletes,
    timestamps,
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub api_token: Option<String>,
    pub active: bool,
    pub preferences: Json,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}
```

### 関数レベルのマクロ

関数レベルのマクロは、構造体アトリビュートと並んで機能します:

- `fn name(&self) -> T` の上の `#[accessor]` は、それをEloquentのアクセッサーにします。`name` が `appends = [...]` に列挙されているとき、モデルの `to_array()` はそれを呼び出します（そして `to_json()` は、`to_array` → 文字列への委譲を介して、それを取り込みます）。
- `fn set_name(&mut self, value: serde_json::Value)` の上の `#[mutator]` は、それをEloquentのミューテータにします。`name` が `mutators = [...]` に列挙されているとき、モデルのJSONによる充填の経路は、それを経由してルーティングされます。
- `impl Model { ... }` ブロックの上の `#[suprnova::scopes(Model)]`:シグネチャが `fn name(query: Builder<Self>[, args…]) -> Builder<Self>` であるすべてのメソッドは、`Builder<Self>` 上の連鎖可能な `.scope_name(args)` と、`Model::scope_name(args)` という近道の、両方になります。関数レベルの `#[scope]` という形式は存在しません - スコープはimplブロックごとに宣言されます。
- グローバルスコープは、`GlobalScope` トレイトを介した実行時の登録であり、`Model::global_scope::<GS>()` を通じて適用されます。関数レベルの `#[global_scope]` マクロは存在しません - 完全なパターンについては、[マクロ](macros.md#suprnova-scopes-model)を参照してください。
- `impl Prunable for T { ... }` の上の `#[prunable]` は、インベントリを介してプルーナーを登録するため、`model:prune` がそれを見つけられます。

## モデルモジュールのレイアウト

`#[suprnova::model]` は、ユーザーに面した構造体（例えば `Post`）を親スコープに保ちつつ、その構造体名をスネークケース化した名前（`post`）を持つ、兄弟の `pub mod` を発行します。その内部モジュールこそが、SeaORMの型が生きる場所です。

`app/src/models/posts.rs` で宣言されたモデルについて:

```rust
use chrono::{DateTime, Utc};
use suprnova::model;

#[model(table = "posts", fillable = ["title", "body"], timestamps)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// 慣例: マクロが内部モジュールの中に発行するSeaORMの型を再エクスポートし、
// 呼び出し側がプレフィックスなしの名前を使えるようにする。Suprnova自身の
// dogfoodモデルは、すべてこの行を持っている（`app/src/models/users.rs`、
// `app/src/models/posts.rs` などを参照）。
pub use post::{ActiveModel, Column, Entity};
```

これで、`crate::models::posts` から、次の項目に到達できるようになります:

| パス | それが何であるか |
|------|-----------|
| `crate::models::posts::Post` | あなたのユーザーに面した構造体 - Eloquentモデルです |
| `crate::models::posts::post::Entity` | `posts` テーブルに対するSeaORMの `EntityTrait` の実装です |
| `crate::models::posts::post::Column` | SeaORMの `Column` enum（カラムごとに1つのバリアント）です |
| `crate::models::posts::post::ActiveModel` | 挿入/更新のためのSeaORMの `ActiveModel` です |
| `crate::models::posts::post::Model` | SeaORM形の行（ストレージの型を持つカラム）です |
| `crate::models::posts::{Entity, Column, ActiveModel}` | 上記の `pub use` の慣例であり、自動発行されるものではありません |

内部モジュールの `Model` について、知っておくべきことが2つあります:

1. それは、あなたの `Post` 構造体ではなく、**SeaORM形**の行です。キャストされたカラムは、ここではその `Storage` の型を運びます（例えば、`bool` は背後の整数になります）。そして、あなたの構造体にある `__eager` / `__pivot` の実行時スロットは存在しません。
2. `From<post::Model> for Post` と `From<Post> for post::Model` が、2つの形を橋渡しします。往復のパターンについては、[SeaORMへ降りる](#seaormへ降りる)を参照してください。

`Model` は、意図的に、従来の親での再エクスポートの一部には**なっていません** - ユーザーに面した `Post` が、親スコープの `Post` という名前をすでに占めているからです。そして `post::Model` は、呼び出し側が内部の形を必要とするときに、`post::Model`（あるいは `From` による変換）を経由して到達する、別の型です。

### 内部モジュールへ手を伸ばすべきとき

Eloquentの表面（`Model` トレイト + `Builder<M>`）は、クエリの大部分をカバーします。SeaORMだけの機能が必要なときは、`post::*` へ手を伸ばしてください:

- **生のクエリ構築** - Eloquentが望むヘルパーを公開していないときの、SeaORMの `EntityTrait::find()` の連鎖によるものです。
- **カスタムのjoinロジック** - Eloquentの `with(...)` がモデル化していないリレーションのために、`QuerySelect::join()` を介して `JoinType::*` のjoinを明示的に構築することです。
- **SeaORMネイティブなサブクエリ** - `Entity::find().select_only()` を介したものです。
- **素の `ActiveModel` の変更** - Eloquentのライフサイクルをバイパスしたい（オブザーバーなし、自動タイムスタンプなし）、まれなケースのためのものです。

```rust
// よくあるケース - 上記の `pub use post::{...}` という慣例を介して、
// 親モジュールのレベルで再エクスポートされたColumn。
use crate::models::posts::Column;

let drafts = Post::query()
    .db_where(Column::Status, "draft")
    .get()
    .await?;

// パワーユーザー向けのケース - SeaORMのEntityに直接アクセスするために、
// 内部モジュールへ手を伸ばす。これは、親の `pub use` が表面化させないものだ。
use crate::models::posts::post;
use suprnova::sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

let db = suprnova::DB::connection()?;
let rows: Vec<post::Model> = post::Entity::find()
    .filter(post::Column::Status.eq("published"))
    .all(db.inner())
    .await?;

// 呼び出し側がそれを望むとき、Eloquentの形へ橋渡しして戻す。
let posts: Vec<Post> = rows.into_iter().map(Post::from).collect();
```

同じ操作のために、内部モジュールへ日常的に手を伸ばしていることに気づいたなら、それはEloquentにヘルパーが欠けている信号です - issueを開くか、`Model` /`Builder` の表面にそのヘルパーを追加してください。

## 行を検索する

```php
// Laravel
$user = User::find(1);
$user = User::findOrFail(1);          // 見つからない場合はスローする
$users = User::findMany([1, 2, 3]);
```

```rust
// Suprnova
let user: Option<User> = User::find(1).await?;
let user: User = User::find_or_fail(1).await?;
let users: Vec<User> = User::find_many([1, 2, 3]).await?;
```

`find_or_fail` は `FrameworkError::ModelNotFound` を返します（コントローラーまで伝播すると、HTTP 404になります）。

### `first_or_create` / `update_or_create` / `first_or_new` / `first_or`

```php
// Laravel
$user = User::firstOrCreate(
    ['email' => 'alice@example.com'],
    ['name' => 'Alice'],
);
$user = User::updateOrCreate(
    ['email' => 'alice@example.com'],
    ['name' => 'Alice Updated'],
);
$user = User::firstOrNew(['email' => 'alice@example.com']);  // 未保存
```

```rust
// Suprnova
let user = User::first_or_create(
    attrs! { email: "alice@example.com" },          // 検索用のキー
    attrs! { name: "Alice" },                       // 作成時の追加フィールド
).await?;

let user = User::update_or_create(
    attrs! { email: "alice@example.com" },
    attrs! { name: "Alice Updated" },
).await?;

let user = User::first_or_new(
    attrs! { email: "alice@example.com" },
).await?;   // 未保存のUserを返す。呼び出し側が明示的に保存する
```

検索用のキーは最初のマップに入ります。作成の経路で適用される追加のフィールドは、2番目のマップに入ります。`first_or_new` を介して未保存のモデルを返すことで、呼び出し側は `save().await?` の前に、それをさらに変更できます。

## 作成と更新

### 作成

```php
// Laravel
$user = User::create([
    'name' => 'Alice',
    'email' => 'alice@example.com',
]);
```

```rust
// Suprnova
let user = User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
}).await?;
```

`attrs!` は、`Attrs` の値（型付けされたJSONマップ）を生成するマクロです。素のJSONも動作します - `User::create(serde_json::json!({"name": "Alice", "email": "..."}))` です。`Fillable` フィルタは `create` の内側で実行されます。許可されていないフィールドはサイレントに落とされ、Laravelの振る舞いと一致します。

### 保存 / 更新

```php
// Laravel
$user->name = 'Alice B';
$user->save();

$user->update(['name' => 'Alice B']);
```

```rust
// Suprnova
user.name = "Alice B".into();
user.save().await?;

user.update(attrs! { name: "Alice B" }).await?;
```

`save()` は、PKでないすべてのフィールドを走査し、それらを `Set(...)` を介してActiveModelに設定し、SeaORMの `update()` を呼び出し、正規の行を返します。`update(attrs)` は同じ流れですが、まず部分的な属性マップを適用します（`Fillable` フィルタと、宣言済みのミューテータを実行します）。

### インクリメント / デクリメント

```php
// Laravel
$user->increment('login_count');
$user->increment('login_count', 5);
$user->decrement('credits', 10);
User::where('plan', 'free')->increment('quota_reset_count');
```

```rust
// Suprnova
user.increment("login_count", 1).await?;
user.increment("login_count", 5).await?;
user.decrement("credits", 10).await?;
User::filter("plan", "free").increment("quota_reset_count", 1).await?;
```

`increment` / `decrement` は `UPDATE table SET col = col + N WHERE ...` というSQLを発します - 並行する更新に対してアトミックであり、read-modify-writeの競合はありません。取得済みのモデルインスタンス上でも（行のPKをWHERE句で使う）、Builderの終端メソッドとしても（連鎖のWHERE句を使う）、どちらでも利用できます。

### `fresh` / `refresh` / `replicate`

```php
// Laravel
$user->refresh();                          // DBから再読み込みする
$copy = $user->fresh();                    // 取得してコピーを返す
$replica = $user->replicate();             // 新しいPKを持つ未保存のクローン
$replica = $user->replicate(['email']);    // 1つのフィールドを省く
```

```rust
// Suprnova
user.refresh().await?;
let copy: User = user.fresh().await?;
let replica: User = user.replicate().await?;
let replica: User = user.replicate_except(["email"]).await?;
```

`refresh` はその場で変更します。`fresh` は、別途取得したコピーを返します。`replicate` は、PKがリセットされた（キーの型の `Default::default()`）メモリ上のクローンを構築します。呼び出し側が明示的に保存します。

### Replicating イベント

`replicate` と `replicate_except` は、メモリ上のクローンを構築した後、それを返す**前**に、モデルごとの `Replicating { source, replica }` イベントを発火します。`replica` フィールドは `Arc<tokio::sync::Mutex<Self>>` であるため、呼び出し側がそれを目にする前に、リスナーが複製を変更できます - タイトルに `(copy)` を前置したり、フラグをクリアしたり、派生カラムをリセットしたりするのに便利です。

```rust
use suprnova::events::{EventFacade, Listener};
use async_trait::async_trait;

pub struct PrefixTitle;

#[async_trait]
impl Listener<post::events::Replicating> for PrefixTitle {
    async fn handle(&self, e: &post::events::Replicating)
        -> Result<(), FrameworkError>
    {
        let mut replica = e.replica.lock().await;
        replica.title = format!("(copy) {}", replica.title);
        Ok(())
    }
}

// 起動時に一度、配線する:
EventFacade::listen::<post::events::Replicating, _>(
    std::sync::Arc::new(PrefixTitle)
).await;
```

### 型をまたぐ複製

```rust
let replica: UserDraft = user.replicate_into().await?;  // 型をまたぐクローン
```

Suprnovaの相違点です - PHPには型がないため、Laravelはこれをできません。ドラフトのモデルを最終的なものへ格上げする（あるいはその逆をする）ときに便利です。

`replicate_into<T>` は `Replicating` を発火**しません**（そのイベントは `Arc<Mutex<Self>>` を運ぶため、ソースの型上のリスナーは、型をまたぐ複製をどのみち変更できません）。T単位のセットアップを望む呼び出し側は、`T::save` を呼ぶ前に、返された `T` の上でそれを実行するべきです - 通常の `Saving` / `Created` の連鎖は、`save` の内側で変わらず発火します。

## 削除とソフトデリート

### ソフトデリートのフラグ

マクロのアトリビュートに `soft_deletes` を、構造体に `deleted_at: Option<DateTime<Utc>>` カラムを追加します:

```rust
#[model(table = "users", soft_deletes, timestamps)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub deleted_at: Option<DateTime<Utc>>,
    // ...
}
```

### ライフサイクル

```rust
user.delete().await?;             // UPDATE: deleted_at = NOW() を設定する
user.trashed();                   // -> true
let trashed = User::with_trashed().find(user.id).await?.unwrap();
trashed.restore().await?;         // UPDATE: deleted_at = NULL を設定する

let only_dead = User::only_trashed().get().await?;
let all_including_dead = User::with_trashed().get().await?;

user.force_delete().await?;       // 実際のDELETE
```

### デフォルトスコープ

`soft_deletes` が設定されているとき、マクロは `Model::query()` をオーバーライドするため、デフォルトの読み取りは、ゴミ箱に入った行を自動的に除外します。`with_trashed()` と `only_trashed()` は、それに再びオプトインします。具体的には: `User::find(id)` はゴミ箱に入った行をスキップします。`User::with_trashed().find(id)` はそれらを見つけます。

## クエリビルダー - デュアルAPI

`Builder<M>` は、`User::query()`、`User::filter(...)`、`User::db_where(...)`、そして連鎖を終端させないその他すべての静的メソッドが返す、連鎖可能なクエリの型です。

### 命名の注記: デュアルAPI

`where` はRustのキーワードであるため、素の等価性のwhereメソッドは、Laravelの名前を共有できません。どちらか一方を選ぶのではなく、where形のすべてのメソッドは、Rustイディオムに沿った名前（`filter`、`filter_in`、`filter_null`、…）と、Laravel形の名前（`db_where`、`where_in`、`where_null`、…）の**両方**として出荷されます。それらは、1つの正規の実装の上のエイリアスです - あなたの体が覚えている方を選んでください。

```rust
// Rust開発者向け:
User::query().filter("active", true).filter_in("role", ["admin"]).get().await?;

// Laravel開発者向け:
User::db_where("active", true).where_in("role", ["admin"]).get().await?;

// 同じクエリ。同じ結果。異なる体の記憶。
```

### Whereの近道

```php
// Laravel
$users = User::where('email', $email)->get();
$users = User::where('age', '>=', 18)->get();
$users = User::where('email', 'like', '%@example.com')->get();
```

```rust
// Suprnova - どちらのファミリーを選んでもよい。どちらもコンパイルでき、どちらもドキュメント化されている。

// Rust形（filterファミリー）:
let users = User::query().filter("email", &email).get().await?;
let users = User::query().filter_op("age", ">=", 18).get().await?;
let users = User::query().filter_like("email", "%@example.com").get().await?;

// Laravel形（db_where / where_* ファミリー）:
let users = User::db_where("email", &email).get().await?;
let users = User::query().db_where_op("age", ">=", 18).get().await?;
let users = User::query().where_like("email", "%@example.com").get().await?;
```

### Whereの変種

すべての行は、2つの等価なSuprnovaの形を持ちます - Rust形（`filter*`）とLaravel形（`db_where` / `where_*`）です。どちらも同じ正規の実装を呼び出し、どちらも `#[doc(alias = "...")]` でタグ付けされているため、rustdocの検索はどちらでも見つけられます。

| Laravel | Suprnova（Rust形） | Suprnova（Laravel形） | 注記 |
|---------|----------------------|--------------------------|-------|
| `->where(col, val)` | `.filter(col, val)` | `.db_where(col, val)` | 等価性 |
| `->where(col, op, val)` | `.filter_op(col, op, val)` | `.db_where_op(col, op, val)` | 任意の演算子 |
| `->orWhere(...)` | `.or_filter(...)` | `.or_where(...)` | |
| `->whereNot(col, val)` | `.filter_not(col, val)` | `.where_not(col, val)` | |
| `->whereIn(col, vals)` | `.filter_in(col, vals)` | `.where_in(col, vals)` | |
| `->whereNotIn(col, vals)` | `.filter_not_in(col, vals)` | `.where_not_in(col, vals)` | |
| `->whereBetween(col, [a, b])` | `.filter_between(col, a..=b)` | `.where_between(col, a..=b)` | Rustの範囲 |
| `->whereNotBetween(col, [a, b])` | `.filter_not_between(col, a..=b)` | `.where_not_between(col, a..=b)` | |
| `->whereNull(col)` | `.filter_null(col)` | `.where_null(col)` | |
| `->whereNotNull(col)` | `.filter_not_null(col)` | `.where_not_null(col)` | |
| `->whereDate(col, '2026-05-19')` | `.filter_date(col, NaiveDate)` | `.where_date(col, NaiveDate)` | |
| `->whereMonth(col, 5)` | `.filter_month(col, 5)` | `.where_month(col, 5)` | |
| `->whereDay(col, 19)` | `.filter_day(col, 19)` | `.where_day(col, 19)` | |
| `->whereYear(col, 2026)` | `.filter_year(col, 2026)` | `.where_year(col, 2026)` | |
| `->whereTime(col, '12:30')` | `.filter_time(col, NaiveTime)` | `.where_time(col, NaiveTime)` | |
| `->whereLike(col, pattern)` | `.filter_like(col, pattern)` | `.where_like(col, pattern)` | |
| `->whereNotLike(col, pattern)` | `.filter_not_like(col, pattern)` | `.where_not_like(col, pattern)` | |
| `->whereJsonContains(col, v)` | `.filter_json_contains(col, v)` | `.where_json_contains(col, v)` | バックエンドにディスパッチされる |
| `->whereJsonLength(col, op, n)` | `.filter_json_length(col, op, n)` | `.where_json_length(col, op, n)` | |
| `->whereColumn(a, b)` | `.filter_column(a, b)` | `.where_column(a, b)` | カラム対カラムの比較 |
| `->whereExists(closure)` | `.filter_exists(builder)` | `.where_exists(builder)` | サブクエリ |
| `->whereHas(rel, closure)` | `.filter_has(rel, fn)` | `.where_has(rel, fn)` | リレーションの述語 (10B) |
| `->whereDoesntHave(rel)` | `.filter_doesnt_have(rel)` | `.where_doesnt_have(rel)` | (10B) |
| `->whereRelation(rel, col, op, v)` | `.filter_relation(...)` | `.where_relation(...)` | (10B) |
| `->whereRaw(sql, bindings)` | `.filter_raw(sql, bindings)` | `.where_raw(sql, bindings)` | |

バインドされた生の述語は、SQLite、MySQL、そしてPostgreSQLの間で移植可能な `?` マーカーを使います:

```rust
let rows = User::query()
    .filter("active", true)
    .filter_raw(
        "score >= ? AND role = ?",
        vec![serde_json::json!(80), serde_json::json!("admin")],
    )
    .get()
    .await?;
```

PostgreSQLでは、Suprnovaはそれらのマーカーを、より前のクエリのバインディングの後に再配置するため、この例は `active` に対して `$1` を、生の述語に対して `$2`/`$3` をレンダリングします。バインドされた生のフラグメントの中で、リテラルな疑問符演算子が欲しい場合は、`"payload ?? 'enabled' AND status = ?"` のように `??` を使ってください。既存の `$N` のフラグメントは引き続き受け入れられますが、移植可能なマーカーは、呼び出し箇所をクエリ上の位置に結合させずに済みます。マーカーの形式が混在している場合や、マーカーとバインディングの個数が一致しない場合は、データベースI/Oの前に拒否されます。あらゆる生の式と同様に、SQLのテキストは信頼できるものでなければなりません - 信頼できない値は、バインディングのベクタの中だけに置いてください。

### 並び順

```php
$users = User::orderBy('name', 'asc')->get();
$users = User::orderByDesc('created_at')->get();
$users = User::latest()->get();        // 近道: orderBy(created_at, desc)
$users = User::oldest()->get();        // 近道: orderBy(created_at, asc)
$users = User::inRandomOrder()->get();
```

```rust
let users = User::query().order_by("name", Direction::Asc).get().await?;
let users = User::query().order_by_desc("created_at").get().await?;
let users = User::latest().get().await?;
let users = User::oldest().get().await?;
let users = User::query().in_random_order().get().await?;
```

`Direction::Asc` / `Direction::Desc` は、SeaORMから再エクスポートされたSuprnovaのenumです。

### グループ化 + having

```php
$rows = User::groupBy('role')->having('count(*)', '>', 5)->get();
```

```rust
let rows = User::query()
    .group_by("role")
    .having_op("count(*)", ">", 5)
    .get()
    .await?;
```

### LIMIT / OFFSET

```php
$users = User::limit(10)->offset(20)->get();
$users = User::take(10)->skip(20)->get();   // エイリアス
```

```rust
let users = User::query().limit(10).offset(20).get().await?;
let users = User::query().take(10).skip(20).get().await?;
```

### Select / add_select / select_raw

```rust
let users = User::query().select(["id", "name", "email"]).get().await?;
let users = User::query().select("name").add_select("email").get().await?;
let rows  = User::query().select_raw("count(*) as total, role")
    .group_by("role")
    .get_raw()
    .await?;
```

`get_raw()` は、カラムがモデルのスキーマと一致しない `select_raw` のケースのために、生のカラム形の結果を返します。`get()` は `Vec<User>` を返し、選択されたカラムがモデルの構造体を満たすことを要求します。

### DISTINCT

```rust
let emails: Vec<String> = User::query().distinct().pluck("email").await?;
```

### 集計

```rust
let count   = User::count().await?;
let count   = User::filter("active", true).count().await?;
let sum     = User::sum::<f64>("balance").await?;
let avg     = Order::avg::<f64>("total").await?;
let min     = Order::min::<DateTime<Utc>>("created_at").await?;
let max     = Order::max::<DateTime<Utc>>("created_at").await?;
let exists  = User::filter("email", &email).exists().await?;
let missing = User::filter("email", &email).doesnt_exist().await?;
```

集計は、戻り値の型についてジェネリックです。SeaORMが、DBのスカラーを何に変換すればよいかを知る必要があるからです。型のデフォルト:`count -> i64` です。`sum`/`avg` は、明示的な型パラメータを運びます。Suprnovaは、生成される集計の式を内部でエイリアス化するため、同じ型付けされた結果が、PostgreSQL、MySQL、そしてSQLiteの上で復号されます。`sum` と `avg` は、マッチする集合が空のときはゼロを返しますが、`min` と `max` は `None` を返します。要求されたRustの型に互換性がない場合や、結果のカラムが欠けている場合は、データベースエラーになります - それが、それらしいゼロや `None` へ変換されることは決してありません。

### 終端メソッド

```rust
let users:  Vec<User>          = User::all().await?;
let first:  Option<User>       = User::first().await?;
let user:   User               = User::first_or_fail().await?;
let value:  Option<String>     = User::filter("...").value("email").await?;
let emails: Vec<String>        = User::pluck::<String>("email").await?;
let keyed:  HashMap<i64, String> = User::pluck_keyed::<i64, String>("id", "name").await?;
let sql:    String             = User::filter("...").to_sql();
```

`to_sql` は、次の終端メソッドが発するはずのパラメータ化されたSQLを返します - デバッグや、ビューの構築に便利です。バインディングには、`.to_sql_with_bindings() -> (String, Vec<Value>)` を介してアクセスできます。

### UNION

```rust
let first  = User::filter("active", true);
let second = User::filter("role", "admin");
let users  = first.union(second).get().await?;
let users  = first.union_all(second).get().await?;
```

## 行ロック

2つのビルダーメソッドが、SELECT時に行単位のデータベースロックを要求します:

```rust
// 排他的な書き込みロック - このトランザクションがコミットするまで、
// 同じ行をロックあるいは書き込みしようとする他のトランザクションをブロックする。
let order = Order::query()
    .filter("id", 42)
    .lock_for_update()
    .first_or_fail()
    .await?;

// 共有読み取りロック - 他の共有リーダーは許可するが、書き込み側はブロックする。
let inventory = Inventory::query()
    .filter("sku", sku)
    .shared_lock()
    .first_or_fail()
    .await?;
```

バックエンドごとに発行されるSQL:

| バックエンド | `lock_for_update()` | `shared_lock()`        |
|----------|---------------------|------------------------|
| Postgres | `FOR UPDATE`        | `FOR SHARE`            |
| MySQL    | `FOR UPDATE`        | `LOCK IN SHARE MODE`   |
| SQLite   | （SQLなし、下記を参照） | （SQLなし、下記を参照） |

ロック句は、複合文のまさに末尾に - すべての `UNION` の枝、すべての `ORDER BY`、すべての `LIMIT` / `OFFSET` の後に - 追加されます。2つのビルダーの `union(...)` の後に続く `.lock_for_update()` は、外側のスコープに、枝ごとに1つではなく、正確に**1つ**の `FOR UPDATE` を発します。

### トランザクションの内側で使う

そのロックが有用な仕事をするのは**トランザクションの内側**でだけです - トランザクションがなければ、SQLは発行されますが、ロックは文の終わりで解放されます。`DB::transaction(...)` と組にしてください:

```rust
DB::transaction(|tx| async move {
    let order = Order::query()
        .filter("id", 42)
        .lock_for_update()
        .first_or_fail()
        .with_tx(&tx)
        .await?;
    // id=42をロックしようとする他のトランザクションは、コミットまでここでブロックされる。
    order.status = "processed".into();
    order.save_with_tx(&tx).await?;
    Ok(())
}).await?;
```

### `lock_for_update` vs `shared_lock`

「読み取ってから書き込む」というフローの大半は、`lock_for_update` を望みます。共有ロックは、別の `shared_lock` の読み取り側が、続く `UPDATE` へ向けてあなたと競争することを、それでも許します - 相互排他であるのは `FOR UPDATE` だけです。

`shared_lock` が正しいのは、行を読み取り、そこから判断を導き出し、書き戻さない、一貫したスナップショット読み取りのためです - 例えば、それ自体は在庫を減らさない、在庫チェックです。

### SQLite

SQLiteには行レベルのロックがありません。ファイルレベルのトランザクションロックだけを使います（`BEGIN IMMEDIATE` / `BEGIN EXCLUSIVE`）。バックエンドをまたぐコードがコンパイルできるように、ロックメソッドはSQLiteの経路にも**保たれています**が、それらはSQLを発しません。

プロセスごとに最初の1回、`lock_for_update` / `shared_lock` がSQLiteのバックエンドに対して実行されるとき、フレームワークは `suprnova::eloquent::lock` というtracingターゲットに、1回だけ `warn!` を記録します。これにより、高頻度なコードパスを埋め尽くすことなく、このno-opを表面化させます。

SQLite上で、行をまたぐ競合の保証が必要な場合は、明示的な `BEGIN IMMEDIATE` トランザクションでクリティカルセクションを包んでください - ファイルレベルでは、それが他のすべての書き込み側をブロックします。

### v1に含まれないもの

- **`NOWAIT` / `SKIP LOCKED`** - ジョブキューの受け取りワークフローに便利ですが、APIの表面を増やします。本物の消費者がそれを必要とするまで、先送りにされています。

## トランザクション

Suprnovaは、データベーストランザクションのための3つのエントリーポイントと、セーブポイントを介したネストしたロールバックを出荷します。そのうちの2つ - クロージャの形式と、デッドロック時のリトライを行うヘルパー - は、アンビエントなコンテキストをインストールするため、クロージャの内側のモデル操作は、呼び出し元がすべての呼び出し箇所にハンドルを通すことなく、トランザクションを自動的に経由します。

### クロージャの形式 - `DB::transaction`

クロージャの形式は、よくあるケースです。クロージャは `&Transaction` を受け取り、`savepoint(name)` でチェックポイントを打つために使えます。クロージャの内側のすべての `Model::*` / `Builder::*` の操作は、`CURRENT_TX` という `tokio::task_local!` を介して、トランザクションを自動的に経由します。

```rust
use suprnova::{DB, FrameworkError, Model};

DB::transaction(|_tx| {
    Box::pin(async move {
        let mut alice = User::query().filter("name", "alice").first_or_fail().await?;
        alice.balance -= 30;
        alice.save().await?;

        let mut bob = User::query().filter("name", "bob").first_or_fail().await?;
        bob.balance += 30;
        bob.save().await?;
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

- クロージャが `Ok` を返す → **コミット**します。
- クロージャが `Err` を返す → **ロールバック**します（元のエラーが伝播します）。
- クロージャがパニックする → ロールバックします（実行中のトランザクションは巻き戻し時にドロップされます。SeaORMの `DatabaseTransaction::drop` がロールバックします）。

クロージャの内側の読み取りは、同じトランザクションからの書き込みを目にします（すべての末端のSQL呼び出しにおける `CURRENT_TX` のルックアップを介して）。プロセス開始後の最初の `DB::transaction` の呼び出しは、`DB::connection()` からデータベースのバックエンドを取得します。それ以降の呼び出しは、同じコネクションレジストリを再利用します。

このシグネチャは、高階トレイト境界 + `Pin<Box<dyn Future>>` を使うため、クロージャは `.await` の地点をまたいで `tx` を借用できます:

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        // ... セーブポイントより前の作業 ...
        tx.savepoint("inner").await?;
        // ... 内側の作業 ...
        if some_condition {
            tx.rollback_to("inner").await?;
        }
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

`Box::pin(async move { ... })` という形は、futureが `.await` の後で `&tx` を使えるようにするための代償です - これがなければ、借用の生存期間はクロージャの本体を脱出できません。SeaORMの `TransactionTrait::transaction` のシグネチャを反映しています。

### セーブポイント - `tx.savepoint(name)` / `tx.rollback_to(name)`

セーブポイントはトランザクションにチェックポイントを打つため、外側のコミットを中断せずに、内側の作業のブロックを捨てられます。3つのバックエンドすべてで動作します - SQLiteには行レベルのロックがないにもかかわらず、SQLiteの `SAVEPOINT` は完全に機能します。

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        let mut account = Account::query().filter("id", id).first_or_fail().await?;
        account.balance = 200;
        account.save().await?;     // 外側のtxがコミットするときにコミットされる

        tx.savepoint("audit_trail").await?;

        let entry = AuditEntry::create(attrs! { actor_id: actor, ... }).await?;
        if audit_validation_failed(&entry) {
            tx.rollback_to("audit_trail").await?;
            // audit_trailの行は消える。accountの更新はコミット待ちのまま
        }

        Ok::<(), FrameworkError>(())
    })
}).await?;
```

セーブポイントの名前は、そのままSQLへ埋め込まれます - 静的な識別子を使ってください。ユーザーの入力を継ぎ合わせては**いけません**。

### ネストした `DB::transaction` は実行時に拒否される

```rust
DB::transaction(|_outer| Box::pin(async move {
    let inner = DB::transaction(|_inner| Box::pin(async move {
        Ok::<(), FrameworkError>(())
    })).await;
    // innerは次のようになる: Err(FrameworkError::Database(
    //     "nested DB::transaction is not supported; use tx.savepoint(name) for nested rollback"
    // ))
    Ok::<(), FrameworkError>(())
})).await?;
```

SeaORMの `DatabaseConnection::begin()` は合成できません - すでにトランザクションを保持しているコネクション上でそれを呼び出すと、外側のスコープとは独立してコミット/ロールバックする、まったく新しい物理トランザクションが開始されます。それはサイレントなデータ整合性のフットガンであるため、`DB::transaction` は前もって `CURRENT_TX` をチェックし、間違ったセマンティクスを生み出す代わりに、データベースエラーを返します。ネストした振る舞いには `tx.savepoint(name)` を使ってください。

### デッドロック時のリトライ - `DB::transaction_with_attempts`

Postgresの `SERIALIZABLE` の読み取りと、MySQLの行レベルのロックは、トランザクションをリトライすることで解決する、直列化失敗 / デッドロックのエラーを引き起こすことがあります。`transaction_with_attempts` は、`attempts` まで、毎回クロージャを最初から実行します:

```rust
DB::transaction_with_attempts(3, |_tx| {
    Box::pin(async move {
        // 並行するtxと競合し、コミット時にSQLSTATE 40001 / 40P01を
        // 表面化させるかもしれない、SERIALIZABLEで分離されたロジック。
        let inventory = Inventory::query()
            .filter("sku", sku)
            .lock_for_update()
            .first_or_fail()
            .await?;
        if inventory.units < requested {
            return Err(FrameworkError::bad_request("out of stock"));
        }
        Inventory::query()
            .filter("sku", sku)
            .update(attrs! { units: inventory.units - requested })
            .await?;
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

検出は、内側のエラーに対するDisplay文字列の部分文字列によるものです:

- Postgres SQLSTATE `40001`（serialization_failure）
- Postgres SQLSTATE `40P01`（deadlock_detected）
- 大文字小文字を区別しない `"deadlock"` の部分文字列（MySQLの `Deadlock found when trying to get lock` と、ユーザーが表面化させるあらゆるdeadlock文字列をカバーします）

最後の試行では、エラーは変更されずに伝播します。クロージャは、毎回の試行で最初から実行されます - リトライの経路が明確に定義されるように、`&mut` 参照ではなく、所有された状態や `Arc` をキャプチャしてください。

> **注意点:** 検出には、大文字小文字を区別しない `"deadlock"` の部分文字列が含まれるため（SQLSTATEを表面化させないドライバーを持つMySQLのために必要です）、`Display` にその語を含むあらゆる内側のエラーが、リトライを引き起こします。`transaction_with_attempts` のクロージャの内側から自分自身のエラーを発生させるときは、メッセージの中で "deadlock" を避けてください - そうしなければ、無関係なバリデーションエラーが、伝播する前に `attempts` 回までリトライされてしまいます。PostgresのSQLSTATEのマッチ（`40001` / `40P01`）は信頼できる信号です。このヒューリスティックはMySQL専用です。

### 手動の形 - `DB::begin_transaction` + `*_with_tx` のシム

トランザクションの生存期間がクロージャに収まらない場合（例えば、複数の制御フローの分岐にまたがる場合）は、手動の `Transaction` を開き、各操作を明示的にそこへオプトインさせてください:

```rust
let tx = DB::begin_transaction().await?;

let mut user = User::query()
    .filter("name", "alice")
    .with_tx(&tx)
    .first_or_fail()
    .await?;
user.balance = 500;
user.save_with_tx(&tx).await?;

if some_condition {
    let mut other = User::query()
        .filter("name", "bob")
        .with_tx(&tx)
        .first_or_fail()
        .await?;
    other.update_with_tx(&tx, attrs! { balance: 200i64 }).await?;
}

tx.commit().await?;  // あるいは tx.rollback().await?;
```

手動モードは `CURRENT_TX` をインストール**しません**。個々の操作をトランザクションの範囲に収めるには、`Builder::with_tx(&tx)`、あるいは `Model::*_with_tx(&tx, ...)` のシムを使ってください:

| トレイトメソッド    | 手動の変種                                 |
|---------------------|-------------------------------------------|
| `Model::create`     | `Model::create_with_tx(&tx, attrs)`       |
| `Model::save`       | `Model::save_with_tx(&tx)`                |
| `Model::update`     | `Model::update_with_tx(&tx, attrs)`       |
| `Model::delete`     | `Model::delete_with_tx(&tx)`              |
| `Model::force_delete` | `Model::force_delete_with_tx(&tx)`      |
| `Builder::*`        | `Builder::with_tx(&tx).*`                 |

`Transaction` を保持することは、そのハンドルの生存期間の間、1つのプール接続を固定します。SQLiteでは、プールは単一のコネクションを持つため、同じデータベースに対する並行の非トランザクション読み取りは、トランザクションが完了するまでブロックされます - **`DB::begin_transaction()` の前に、事前に必要な行をすべて読み込んでおき**、依存するすべての書き込みを、返された `tx` 経由でルーティングしてください。

`Transaction::commit` / `Transaction::rollback` はハンドルを消費し、内側のSeaORMトランザクションの `Arc::try_unwrap` を要求します。コミット /ロールバックの時点で、（`tx.handle()` / `Builder::with_tx(&tx)` からの）`TxHandle` のクローンがまだ生きている場合、どちらも "TxHandle clones still alive" というエラーで失敗します。正しい修正は、`commit` を呼ぶ前に、あなたの `Builder<M>` や、残っているハンドルをドロップすることです - フレームワークは、同じtxを保持する並行の書き込み側に対して、半分だけコミットされていない書き込みを競争させることを拒みます。

### 優先順位

操作をコネクション経由でルーティングするための、3段階の優先順位です:

1. **ビルダーレベルのオーバーライド** - `Builder::with_tx(&tx)`、あるいは任意の `Model::*_with_tx(&tx, ...)` のシムです。明示的なものが、アンビエントなものに勝ちます。
2. **アンビエントな `CURRENT_TX`** - クロージャのタスクスコープのために、`DB::transaction` / `DB::transaction_with_attempts` によってインストールされます。
3. **プールへのフォールバック** - `DB::connection()` は、グローバルな `DbConnection` のシングルトンを返します。

`DB::transaction(|tx| ...)` の内側で `Builder::with_tx(&other_tx)` を呼ぶと、その1つのクエリを明示的に `other_tx` 経由でルーティングします - アンビエントな `CURRENT_TX` をバイパスします。それはほぼ確実にバグです。オーバーライドの経路は手動の形式のために存在し、クロージャ自身のtxをオーバーライドするためのものではありません。

### `with_tx` とグローバルスコープ

`tx_override` を運ぶビルダーは、それでもグローバルスコープ、名前付きスコープ、そしてイーガーロードの計画を尊重します - オーバーライドが変えるのはコネクションのルーティングだけであり、SQLではありません。

### 制限 (v1)

- **リレーションのイーガーロード** - `Builder::with(["posts"])` と `Collection::load(["posts"])` は、イーガーな `IN (...)` サブクエリを、アクティブなトランザクション経由ではなく、`DB::connection()` 経由でルーティングします。`DB::transaction` のクロージャの内側にある保留中の書き込みは、`.with(...)` を介してロードされるリレーションから**見えません**。今のところは、tx上の作業を、直接の `Model::*` /`Builder::*` / `DB::table(...)` の呼び出しに絞ってください。リレーションのロードは、外側の書き込みが完了した後まで（あるいは手動の経路では `DB::begin_transaction` の前まで）延期してください。これは既知の縫い目です - ルーティングのヘルパー（`ExecutorChoice`）は、すべてのSQLの末端に、すでに配置されています。詰まっているのは、マクロがすべてのリレーションの種類のために発行する `EagerLoadDispatch::eager_load` が、具体的な `&DatabaseConnection` を取ることです。後続の一掃で、このトレイトをディスパッチのヘルパーに合わせます。
- **Postgres上のDDL** - トランザクションの内側の `DB::statement(...)` は、txのコネクションに対してDDLを実行します。これはPostgresが許すことです。MySQLは暗黙的にコミットするため、Suprnovaのトランザクションの内側ではサポートされません（これは、Laravelの `DB::transaction` の注意点と一致します）。

## スコープ

Suprnovaは、Laravelを反映した、2種類のスコープを出荷します:

- **ローカルスコープ** - ビルダー上の拡張メソッドで、モデルごとに `#[suprnova::scopes(Model)]` で宣言されます。注釈が付いた `impl` ブロックの中の各フリー関数は、`Model::name()`（静的な開始点）と `Builder::name()`（連鎖可能なメソッド）の、両方になります。
- **グローバルスコープ** - `ScopeRegistry::register::<M, _>(scope)` を介して起動時に登録される `GlobalScope<M>` の実装です。すべての `Model::query()` の呼び出しが、それらを自動的に積み重ねます。

### ローカルスコープ

ローカルスコープは、`fn(query: Builder<Self>, args...) -> Builder<Self>` という形を与えることで宣言します:

```rust
#[suprnova::scopes(User)]
impl User {
    pub fn active(query: Builder<Self>) -> Builder<Self> {
        query.filter("active", true)
    }

    pub fn popular(query: Builder<Self>, threshold: i64) -> Builder<Self> {
        query.filter_op("followers_count", ">", threshold)
    }
}

// 開始点、あるいは連鎖可能なメソッドとして使う:
let active_users  = User::active().get().await?;
let popular_users = User::query().active().popular(500).get().await?;
```

同じ `impl` ブロックの中で宣言された、スコープでないメソッド（最初の引数が `query: Builder<Self>` でない、あらゆるもの）は、変更されずに通過します。

### グローバルスコープ

グローバルスコープは、すべての `Model::query()` の呼び出しに適用されます。典型的な用途はマルチテナンシーです - 各呼び出し元がフィルタを通すことなく、すべての読み取りが、現在のテナントにスコープされます。

```rust
use suprnova::eloquent::scopes::{GlobalScope, ScopeRegistry};

pub struct TenantScope;

impl GlobalScope<Article> for TenantScope {
    fn apply(&self, query: Builder<Article>) -> Builder<Article> {
        // task-local / AtomicI64 / リクエストごとの状態が生きている
        // どこであれ、そこから現在のテナントを読み取る。
        query.filter("tenant_id", current_tenant_id())
    }
}

// 起動時 - 通常はあなたのプロバイダ/ブートストラップモジュールの内側:
ScopeRegistry::register::<Article, _>(TenantScope);

// すべての読み取りは、アクティブなテナントに自動的にスコープされる:
let scoped = Article::query().get().await?;
```

モデルごとの複数のスコープは、登録順に合成されます - 最初に登録されたものが最初に実行されるため、そのフィルタの句がWHEREの連鎖の中で最初に現れます。ANDで結合されたフィルタは順序を気にしませんが、副作用の順序が見える句（例えば、並べ替え、having、生のフラグメント）については、左から右への順序が重要です。

### グローバルスコープからオプトアウトする

`#[suprnova::model]` マクロが触れる各モデルには、2つの静的なヘルパーが発行されます:

```rust
// 型によって、登録されたスコープを正確に1つバイパスする。他のスコープはそれでも適用される。
let all_tenants = Article::without_global_scope::<TenantScope>().get().await?;

// 登録されたすべてのスコープをバイパスする。管理者向けツールのパターン。
let everything = Article::without_global_scopes().get().await?;
```

**重要:** オプトアウトのヘルパーは、エントリーポイントでなければなりません。`Model::query()` がすでに返したビルダーの上に `.without_global_scope::<S>()` を連鎖させても、すでに実行されたスコープは元に戻りません - `Model::query()` は、構築の時点で即座にスコープを適用するため、マスクをかけるのが遅すぎるのです。正しいセマンティクスのためには、（上記の）モデルごとの静的なヘルパーを使ってください。

### グローバルスコープが適用される場所

| パス | グローバルスコープは適用されるか？ |
|------|----------------------|
| `Model::query()` | はい - 正規のスコープされたエントリーポイントです |
| `Model::without_global_scope::<S>()` | はい、`S` を除きます |
| `Model::without_global_scopes()` | いいえ |
| `Model::find(id)` | いいえ - PKのルックアップは、SeaORMを直接経由します |
| `Model::find_many([...])` | いいえ - 同じ理由です |
| `Model::all()` | いいえ - 同じ理由です |

これはLaravelを反映しています: `Eloquent\Model::find` は `addGlobalScopes` を発火させません。スコープされたPKのルックアップを望む呼び出し元は、`Self::query().filter("id", pk).first().await?` を使います。

### ソフトデリートとグローバルスコープは共存する

`#[suprnova::model(soft_deletes)]` は、型付けされたスコープレジストリを経由するのではなく、別の文字列タグの仕組みを介して `deleted_at IS NULL` フィルタをインストールします。両方の層は合成されます:

- `Model::query()` は、ゴミ箱に入った行を除外し、かつ、登録されているすべてのスコープを実行します。
- `Model::without_global_scopes()` は、登録されているスコープを落としますが、ソフトデリートのフィルタは保ちます - すべてのカラムセットを読み取りたい管理者向けツールでも、デフォルトではゴミ箱に入った行を除外し続けます。
- `Model::with_trashed()` と `Model::only_trashed()` は、ソフトデリートのフィルタリングをスキップし、レジストリもバイパスします（それらは、新しいスコープなしのビルダーを構築します）。ゴミ箱に入った行に対して、スコープを意識した読み取りが必要な場合は、`.without_global_scope::<S>()` と組にしてください。

## リレーションシップ

Suprnovaは、すべてのEloquentリレーションの種類を出荷します。それらは `#[suprnova::model]` の `relations = { ... }` ブロックの中で宣言され、マクロは - 宣言されたリレーションごとに - 構造体上のメソッド、ロード済みアクセッサー（`<name>_loaded()`）、カウントアクセッサー（`<name>_count()`）、そしてイーガーローダーが呼び込むディスパッチャーのアームを発行します。この節は、種類ごとの形とオプションの表をカバーします。join-keyの解決、多態レジストリ、ピボットの行、そして多態的なenumへの落とし込みについての深掘りは、[Eloquent リレーションシップ](eloquent-relationships.md)にあります。今日出荷されているリレーションの種類です:

| 種類                | 単/多 | ファミリーをまたぐ | 裏付け |
|---------------------|----------|-----------------|-----------|
| `HasOne<R>`         | 単      | いいえ              | `<parent>_id` に対する `IN` クエリ |
| `BelongsTo<R>`      | 単      | いいえ              | この行の上のFKに対する `IN` クエリ |
| `HasMany<R>`        | 多      | いいえ              | `HasOne` と同じ。`Vec<R>` を返す |
| `BelongsToMany<R, P>` | 多    | いいえ              | ピボットテーブル `P`、INNER JOIN + `pivot::<P>()` |
| `HasOneThrough<B, R>`  | 単   | いいえ              | 2クエリのJOIN `parent → B → R` |
| `HasManyThrough<B, R>` | 多   | いいえ              | 上記と同じ。`Vec<R>` を返す |
| `MorphOne<R>`       | 単      | はい              | `IN` + `<name>_type = "<self>"` フィルタ |
| `MorphMany<R>`      | 多      | はい              | `MorphOne` と同じ。`Vec<R>` を返す |
| `MorphTo`           | 単      | はい（子 → 多くのファミリー） | 宣言の場所で発行される、ファミリーごとのenum |
| `MorphToMany<R, P>` | 多      | はい              | 多態的なm2mピボット `P` |
| `MorphedByMany<R, P>` | 多    | はい（逆）        | 同じピボットを、逆方向に走査する |

### `relations = { ... }` の構文

すべてのリレーションの宣言は、同じ外側の形を運びます: リレーションの名前、種類、関連する型（該当する場合はピボット/中間の型も）、そしてオプションの `{ ... }` ブロックです。

```rust
use suprnova::model;

#[model(
    table = "users",
    relations = {
        // HasMany<R>
        posts: HasMany<crate::models::Post> {
            fk = "author_id",         // デフォルトの`user_id`を上書きする
        },
        // BelongsToMany<R, Pivot>
        roles: BelongsToMany<crate::models::Role, crate::models::RoleUser> {
            with_pivot = ["assigned_at"],
            with_timestamps,
        },
    },
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

共通のオプション:

| オプション                     | リレーションの種類                | 目的 |
|----------------------------|-------------------------------|---------|
| `fk = "..."`               | 子のFKを持つすべての種類    | 親を指す、子の上のカラムです。デフォルト = `<snake(parent_struct)>_id`。 |
| `lk = "..."`               | 単/多の種類                | joinキーとして使われる、親の上のカラムです。デフォルト = `"id"`。 |
| `related_key = "..."`      | `BelongsToMany`、`MorphToMany` | 関連する側のPKのカラム名です。デフォルト = `"id"`。関連するモデルが `id` 以外のPKを使う場合に必須です。 |
| `with_pivot = ["...", ...]` | `BelongsToMany`、`MorphToMany` | joinの中で表面化させる、ピボット上の追加のカラムです。 |
| `with_timestamps`          | `BelongsToMany`、`MorphToMany` | attach/syncの際に `created_at` / `updated_at` を刻印します。 |
| `with_default = \|\| { ... }` | `BelongsTo`                 | FKがnullである、あるいは親が見つからない場合に、デフォルトを生成するクロージャです。 |
| `first_key`、`second_key`、`second_local_key` | `HasOneThrough`、`HasManyThrough` | JOINキーの上書きです - 下記のThroughの節を参照してください。 |
| `name = "..."`             | すべての多態の種類              | 多態ファミリーの名前です（例えば `"commentable"`、`"taggable"`）。子/ピボット上の `<name>_id` / `<name>_type` カラムを駆動します。 |
| `targets = [T1, T2, ...]`  | `MorphTo`                     | 具体的な多態ターゲットのリストです。マクロは、宣言の場所に、ターゲットごとに1つのバリアントと `Unknown(String, i64)` を持つ `<Name>Morph` enumを発行します。 |
| `target_morph_type = "..."` | `MorphedByMany`              | ピボット上のターゲットファミリーを識別する、morph-type文字列です。 |
| `pivot_table`、`pivot_foreign_key`、`pivot_related_key` | `BelongsToMany`、`MorphToMany` | デフォルトが合わない場合の、ピボット側のカラム / テーブルの上書きです。 |

### `HasOne<R>` と `BelongsTo<R>`

両方向の1対1です。`HasOne` は親側に存在し、`R::query().filter(<fk>,<self.id>).first()` を呼びます。`BelongsTo` は子側に存在し、`self` からFKを読み取り、それから `R::query().filter(<owner_key>, <fk_value>).first()` を呼びます。

```rust
#[model(table = "users", relations = {
    profile: HasOne<crate::models::Profile>,
})]
pub struct User { /* ... */ }

#[model(table = "profiles", relations = {
    user: BelongsTo<crate::models::User>,
})]
pub struct Profile {
    pub id: i64,
    pub user_id: i64,
    pub bio: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let user = User::find(1).await?.unwrap();
let profile: Option<Profile> = user.profile().first().await?;

let profile = Profile::find(42).await?.unwrap();
let owner: Option<User> = profile.user().first().await?;
```

`BelongsTo` は `with_default = || R { ... }` をサポートします。これは、FKがnullである場合、あるいは親の行が見つからない場合のどちらでも発火します。デフォルトのクロージャは呼び出しごとに（そしてイーガーロードされた行ごとに）実行されます - 削除されたユーザーがそれでもコメントを持っている場合の、空の代役にぴったりです。

```rust
#[model(table = "comments", relations = {
    author: BelongsTo<crate::models::User> {
        with_default = || User {
            name: "[deleted]".into(),
            ..Default::default()
        },
    },
})]
pub struct Comment { /* ... */ }

let c = Comment::find(99).await?.unwrap();
// 常にSome - userの行が見つからない場合に、デフォルトが発火する。
let author = c.author().first().await?.unwrap();
```

### `HasMany<R>`

親側の1対多です。流れるようなビルダーを返します。filter / order /latest / take / get / count を連鎖させ、終端させてください。

```rust
#[model(table = "users", relations = {
    posts: HasMany<crate::models::Post> {
        fk = "author_id",
    },
})]
pub struct User { /* ... */ }

let u = User::find(1).await?.unwrap();

// このuserによるすべてのpost、デフォルトの並び順:
let posts: Vec<Post> = u.posts().get().await?;

// フィルタ + 並べ替え + ページングされた:
let recent = u.posts()
    .filter("published", true)
    .latest()                          // ORDER BY created_at DESC
    .take(10)
    .get()
    .await?;

// COUNTだけ - 行の取得なし:
let total: i64 = u.posts().count().await?;
```

利用できる終端メソッド: `.first()`、`.get()`、`.count()`。利用できる連鎖可能なフィルタ: `.filter` / `.db_where`、`.filter_in` / `.where_in`、`.order_by`、`.latest`、`.oldest`、`.limit`、`.take`。

### `BelongsToMany<R, P>` - ファーストクラスのピボット

`#[suprnova::model]` で宣言されたピボットを介した多対多です。ピボットは、自分自身の行の同一性を持つ、ファーストクラスのモデルです - タプルでも、隠れたハッシュマップでもありません。Laravelの匿名ピボットの形に対する、2つの重要な利点があります:

1. ピボットの行は型安全です。`with_pivot` のカラムは `r.pivot::<P>().<column>` を介して読み取り、`r.pivot.get("...")` を介することは決してありません。
2. ピボットモデルは、他のすべてのモデルと同じ方法で、フレームワークの残りの部分（ファクトリー、スコープ、キャスト、フック）から到達可能です。

```rust
#[model(table = "role_user", fillable = ["user_id", "role_id", "assigned_at"])]
pub struct RoleUser {
    pub id: i64,
    pub user_id: i64,
    pub role_id: i64,
    pub assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[model(table = "users", relations = {
    roles: BelongsToMany<crate::models::Role, RoleUser> {
        with_pivot = ["assigned_at"],
        with_timestamps,
    },
})]
pub struct User { /* ... */ }

let u = User::find(1).await?.unwrap();
let admin = Role::create(attrs! { name: "admin" }).await?;

// Attach + syncのミューテータ
u.roles().attach(admin.id).await?;
u.roles().attach_with(admin.id, attrs! { assigned_at: chrono::Utc::now() }).await?;
u.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
u.roles().detach(admin.id).await?;

// 行ごとのダウンキャストアクセッサーを介して、ピボットのデータを読み取る:
let roles = u.roles().get().await?;
for r in &roles {
    let p: &RoleUser = r.pivot::<RoleUser>();
    println!("user {} got role {} at {:?}", p.user_id, p.role_id, p.assigned_at);
}
```

- `.attach(id)` - 単一のピボット行をINSERTします。あなたのピボットがそれを許さない限り、重複でエラーになります（フレームワークはRustの層で重複排除を行いません。べき等性には `.sync` を使ってください）。
- `.attach_with(id, attrs! { ... })` - 追加のピボットカラムを伴ってINSERTします。`with_timestamps` が有効なとき、タイムスタンプを刻印します。
- `.detach(id)` - 親 → idを結びつけているピボットの行を、DELETEします。
- `.sync([ids...])` - 差分を取って適用します: 新しいものをattachし、欠けているものをdetachし、共通部分はそのままにします。トランザクションで包まれています。

`.get()` は `Vec<R>` を返します。各行の内部の `__pivot` フィールドに、ピボットが刻印されています。`.pivot::<P>()` アクセッサーは、`Arc<dyn Any>` を、あなたが宣言したピボットの型へダウンキャストします。間違った型で呼び出すとパニックします - 宣言したピボットに、型を合わせてください。

### `HasOneThrough<B, R>` と `HasManyThrough<B, R>`

中間の `B` を介して、最終的なターゲット `R` に到達します。リレーションが2つのテーブルを横断するものの、中間を表面化させる必要がない場合に便利です（`A → B → R`）。

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<crate::models::User, crate::models::Post>,
})]
pub struct Country {
    pub id: i64,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let c = Country::find(1).await?.unwrap();
let posts: Vec<Post> = c.posts().get().await?;
```

ディスパッチャーは、構造体の名前からJOINキーを推論します。上書き:

| オプション          | デフォルト                          | 説明 |
|---------------------|----------------------------------|-------------|
| `first_key`         | `<snake(parent_struct)>_id`      | 親 `A` を指す、中間 `B` の上のカラムです。 |
| `second_key`        | `<snake(intermediate_struct)>_id` | 中間 `B` を指す、最終的な `R` の上のカラムです。 |
| `second_local_key`  | `"id"`                           | `second_key` によってマッチされる、中間 `B` の上のカラムです。`B` が `id` 以外のPKを使う場合に必須です。 |

親の主キーのカラムは、モデルの `primary_key` の宣言から読み取られます（デフォルトは `"id"`）- `HasManyThrough` / `HasOneThrough` には `local_key` の上書きは存在しません。`id` 以外の親のキーが必要な場合は、`#[suprnova::model]` アトリビュートを介して親のPKを変更してください。

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<crate::models::User, crate::models::Post> {
        first_key = "country_id",
        second_key = "author_id",
    },
})]
pub struct Country { /* ... */ }
```

### `targets = [...]` とファミリーごとのenumを伴う `MorphTo`

多態的なリレーションは、子の行を、いくつかの親ファミリーのうちの1つに向けます。子は `(<name>_id, <name>_type)` の組を運びます。`*_type` カラムは、各親が宣言する、morph-type文字列を保持します。

`MorphTo` は子に存在します。その宣言は、`targets = [...]` を介して、それが指しうるすべての親ファミリーを列挙します。マクロは、`<RelationName>Morph` という名前の（リレーション名のPascalCase形に一致し、`Morph` を接尾辞として持つ）ファミリーごとのenumを、ターゲットの型ごとに1つのバリアントと、`<name>_type` の値がどの登録済みターゲットにもマッチしないレガシーな行のための `Unknown(String, i64)` を伴って発行します。

```rust
#[model(table = "posts", morph_type = "post")]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video")]
pub struct Video { /* ... */ }

#[model(table = "comments", relations = {
    commentable: MorphTo {
        name = "commentable",
        targets = [
            crate::models::Post,
            crate::models::Video,
        ],
    },
})]
pub struct Comment {
    pub id: i64,
    pub commentable_id: i64,
    pub commentable_type: String,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let c = Comment::find(1).await?.unwrap();
match c.commentable().get().await? {
    CommentableMorph::Post(post)   => println!("comment on post {}", post.title),
    CommentableMorph::Video(video) => println!("comment on video {}", video.url),
    // レガシー/ダングリングな行 - `<name>_type` がどのターゲットにも
    // マッチしない、あるいはmorph_typeはマッチしたが`<name>_id`の行が
    // 消えている。
    CommentableMorph::Unknown(ty, id) => {
        eprintln!("comment {} points at unknown {ty}#{id}", c.id);
    }
}
```

各ターゲットの構造体の上の `morph_type = "..."` アトリビュートは、ローダーが挿入時に子の `<name>_type` カラムへ書き込み、読み取り時にそれでフィルタするものです。`morph_type` がなければ、フレームワークは `to_snake(struct_name)` から型文字列を導出します。

`MorphTo` のディスパッチ - ファミリーごとのenumが正しいバリアントをどのように選ぶか - は、実行時の多態レジストリ（すべての `#[suprnova::model(morph_type = "...")]` の宣言によって満たされるインベントリ）を参照します。宣言された各ターゲットについて、取得用のヘルパーはターゲットの `TypeId` をルックアップし、登録されている `morph_type` 文字列を読み取り、それを、子の行に格納されている `<name>_type` の値と比較します。宣言の順序で、最初にマッチしたものが勝ちます。明示的な `morph_type` アトリビュートを持たないターゲットは、`to_snake(target_type_name)` にフォールバックします - これは、親側の `MorphMany` / `MorphOne` が書き込み時に型文字列を刻印するために使う、同じデフォルトです。そのため、両方の側は揃ったままです。つまり、カスタムの `morph_type` の値（例えば、`Post` という名前の構造体の上の `morph_type = "blog_post"`、あるいは任意の型にはまらない文字列）は、宣言の場所を変更することなく、正しくディスパッチされます。

### `MorphOne<R>` と `MorphMany<R>` - 親側

`MorphTo` の逆方向です: 親の型が、自分が所有する多態的な単一/複数を宣言します。`MorphOne` は `.first()` から `Option<R>` を返します。`MorphMany` は `.get()` から `Vec<R>` を返します。どちらも、子の `(<name>_id, <name>_type)` の組を、`self.id` と親の `morph_type` でフィルタします。

```rust
#[model(table = "posts", morph_type = "post", relations = {
    comments: MorphMany<crate::models::Comment> {
        name = "commentable",
    },
    cover: MorphOne<crate::models::Image> {
        name = "imageable",
    },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    comments: MorphMany<crate::models::Comment> {
        name = "commentable",
    },
})]
pub struct Video { /* ... */ }

let post = Post::find(1).await?.unwrap();
let post_comments: Vec<Comment> = post.comments().get().await?;
let post_cover:    Option<Image> = post.cover().first().await?;

let video = Video::find(1).await?.unwrap();
let video_comments: Vec<Comment> = video.comments().get().await?;
// post.comments()は`commentable_type = "post"`の行だけを返す。
// video.comments()は`commentable_type = "video"`の行だけを返す。
```

`HasMany` / `HasOne` と同じ連鎖可能な表面です: `.filter` / `.db_where`、`.order_by` / `.latest` / `.oldest`、`.limit` / `.take`、`.first` / `.get` /`.count`。

### `MorphToMany<R, P>` と `MorphedByMany<R, P>`

多態的な多対多です。共有されるピボット `P` は、FKの組に加えて、`<name>_type` の判別子カラムを運びます。一方の端は `MorphToMany` を宣言します（例えば `Post.tags()`、`Video.tags()`）。もう一方の端は、ターゲットファミリーごとに1つの `MorphedByMany` を宣言します（例えば `Tag.posts()`、`Tag.videos()`）。

```rust
#[model(table = "taggables", fillable = ["tag_id", "taggable_id", "taggable_type"])]
pub struct Taggable {
    pub id: i64,
    pub tag_id: i64,
    pub taggable_id: i64,
    pub taggable_type: String,
}

#[model(table = "posts", morph_type = "post", relations = {
    tags: MorphToMany<crate::models::Tag, Taggable> {
        name = "taggable",
    },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    tags: MorphToMany<crate::models::Tag, Taggable> {
        name = "taggable",
    },
})]
pub struct Video { /* ... */ }

// 逆方向: Tagは、ターゲットファミリーごとに1つのMorphedByManyを宣言する。
#[model(table = "tags", relations = {
    posts: MorphedByMany<crate::models::Post, Taggable> {
        name = "taggable",
        target_morph_type = "post",
    },
    videos: MorphedByMany<crate::models::Video, Taggable> {
        name = "taggable",
        target_morph_type = "video",
    },
})]
pub struct Tag { /* ... */ }

let post  = Post::find(1).await?.unwrap();
let video = Video::find(1).await?.unwrap();
let tag   = Tag::create(attrs! { name: "rust" }).await?;

// `attach` / `attach_with` / `detach` / `sync` は、BelongsToManyと
// 同じ方法で動作する。`<name>_type` カラムは、呼び出す親の
// `morph_type` から自動的に収まる。
post.tags().attach(tag.id).await?;
video.tags().attach(tag.id).await?;          // 独立したattachment
post.tags().sync([tag_a.id, tag_b.id]).await?;

// 逆方向 - Tagはファミリーごとに分岐する:
let posts_with_tag:  Vec<Post>  = tag.posts().get().await?;   // "post"型
let videos_with_tag: Vec<Video> = tag.videos().get().await?;  // "video"型
```

`MorphedByMany` の `target_morph_type` が必須なのは、`Tag` の宣言の場所にあるマクロが、ターゲットの `morph_type = "..."` アトリビュートを覗き込めないからです（それは別の `#[suprnova::model]` の呼び出しの中に存在します）。それを明示的に設定することで、各 `MorphedByMany` のアームが、自分がどのファミリーを走査するのかについて、正確であり続けます。

### エスケープハッチ: 手書きのリレーションメソッド

`relations = { ... }` の中で宣言されたリレーションだけが、イーガーロードのディスパッチャー（そして `with`、`with_count` など）が知っているものです。リレーションがマクロの形にとって異例すぎる場合 - 例えば、2つのピボットをまたいで集計するクエリ、あるいは非正規化されたキャッシュテーブルの型付きビューなど - は、`relations = { ... }` からそれを省き、素の固有implを書くことができます:

```rust
impl User {
    /// このuserが著者である、あるいはタグ付けされているpost。2つの
    /// リレーションを横断するため、単一の `relations = { ... }` の宣言としては
    /// 表現できない - 手で書かれている。
    pub async fn posts_touched(&self) -> Result<Vec<Post>, FrameworkError> {
        let authored: Vec<Post> = self.posts().get().await?;
        let tagged:   Vec<Post> = /* ...カスタムクエリ... */;
        // ...マージ + 重複排除...
        Ok(/* ... */)
    }
}
```

このようなメソッドは、イーガーロードのサポートを失います - `User::with(["posts_touched"])` は、ディスパッチャーが `posts_touched` のためのアームを持たないため、エラーになります。マクロの内側の宣言だけが、フレームワークがイーガーロード、カウント、集計、そして述語によるフィルタの方法を知っている経路のままです。

### v1の制限

v1の表面が先送りにしている、いくつかのことです。それぞれ、宣言の場所にも文書化されています - 見える場所にまとめて置かれています:

- **Morph IDは `i64` のみです。** `MorphTo::morph_id` は `i64` にハードコードされているため、`MorphTo` のターゲットとして使われるモデルは、`i64` の主キーを宣言しなければならず、子テーブルの `<name>_id` カラムも `i64` でなければなりません。文字列 / 文字列経由のUUIDのmorph FKはv2です。
- **`MorphTo` を介したネストしたイーガーロードはありません。** ファミリーごとのenumは子の型を消去するため、`with(["commentable.user"])` のようなドット区切りのパスは末尾再帰できません - ディスパッチャーは型付けされたエラーを返します。ファミリーごとに解決するには、enumに対してmatchし、各バリアントに個別に `with(["user"])` を呼んでください。
## イーガーロード

イーガーロードは、N+1クエリを避けます。すべてのuserのpostを取得するために `posts.len()` 回のクエリを実行する代わりに、Suprnovaは、ロードされる親の行がいくつであっても、トップレベルのリレーションごとに1回のクエリを発行します。

フラットなリスト、ネストしたパス、カウント、集計、そして述語でフィルタされた イーガーロードを含む表面全体は、各モデルの上で `#[suprnova::model]` が発行するヘルパーを通じて到達します:

```rust
// 単一のリレーション:
let users = User::with(["posts"]).get().await?;
for u in &users {
    for p in u.posts_loaded() { /* ... */ }
}

// 複数のリレーション:
let users = User::with(["posts", "profile"]).get().await?;

// ネストしたパス - 3クエリ（users + posts + comments）、N+1なし:
let users = User::with(["posts.comments"]).get().await?;
let p1 = users[0].posts_loaded()[0];
let comments = p1.comments_loaded();

// より深いネストも、期待どおりに動作する:
let users = User::with(["posts.comments.author"]).get().await?;

// 親の行と並んでカウントする:
let users = User::with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}

// 集計 - リレーションのカラムに対するSum / Avg / Min / Maxである。
// エルゴノミクスに優れた読み方は、マクロが発行する
// `<rel>_sum_of(col)` アクセッサーだ。
let users = User::with_sum(("posts", "views")).get().await?;
let sum: f64 = users[0]
    .posts_sum_of("views")
    .expect("with_sum populated the cache");

// 同じリレーションの上の複数の集計は合成される - キャッシュキーは
// 広い `<rel>_<kind>_<col>` の形であるため、異なる種類と異なるカラムは
// 衝突しない:
let users = User::with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .with_min(("posts", "id"))
    .get()
    .await?;
let u = &users[0];
let sum = u.posts_sum_of("views").unwrap();   // Some(_) - viewsの合計
let avg = u.posts_avg_of("views").unwrap();   // Some(_) - viewsの平均
let min = u.posts_min_of("id").unwrap();      // Some(Some(_)) - 空でないグループ
let max = u.posts_max_of("id");               // None - with_maxが呼ばれていない

// イーガーロードされた子をフィルタする。マクロは、リレーションごとに
// 型付けされた `with_where_<rel>(closure)` という静的なヘルパーを発行する
// ため、クロージャの引数の型は推論される - `Builder<Post>` を書き出す
// 必要はない:
let users = User::with_where_posts(|q| q.filter("published", true))
    .get()
    .await?;
// 返される `Builder<User>` は、他のあらゆるベースクエリの
// ビルダーメソッドと連鎖する:
let users = User::with_where_posts(|q| q.filter("published", true))
    .filter("active", true)
    .get()
    .await?;
// ジェネリックな形も、それでも利用できる - リレーション名が
// 実行時に計算される場合に便利だ - ただし、クロージャの上で
// ターゲットの型に名前を付ける必要がある:
let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// 各u.posts_loaded()には、publishedなpostだけが含まれる。
```

### キャッシュのレイアウト

行ごとの `__eager` キャッシュセルは、次によってキー付けされます:

- `<rel>`（リレーション名だけ） - `with` と `with_count` のためです。
- `<rel>_<kind>_<col>`（例えば `posts_sum_views`） - 4つの集計の種類のためです - `with_sum` / `with_avg` / `with_min` / `with_max`。この広いキーによって、同じリレーション上の複数の集計が、互いを上書きすることなく、同じ行の上で共存できます。

| メソッド                              | キャッシュキー            | キャッシュセルの型   | 空グループの値 |
|-------------------------------------|----------------------|-------------------|-------------------|
| `with(["posts"])`                   | `posts`              | `Vec<Post>`       | `Vec::new()`      |
| `with(["profile"])`                 | `profile`            | `Option<Profile>` | `None`            |
| `with_count(["posts"])`             | `posts`              | `u64`             | `0`               |
| `with_sum(("posts","views"))`       | `posts_sum_views`    | `f64`             | `0.0`             |
| `with_avg(("posts","views"))`       | `posts_avg_views`    | `f64`             | `0.0`             |
| `with_min(("posts","id"))`          | `posts_min_id`       | `Option<f64>`     | `None`            |
| `with_max(("posts","id"))`          | `posts_max_id`       | `Option<f64>`     | `None`            |

マクロは、各モデルの上に、対応するアクセッサーを発行します:

- `<rel>_loaded()` - コレクションのリレーションについては: `&[Post]`（リレーションがイーガーロードされていない場合はパニックします）。単一値のリレーションについては: `Option<&Profile>` です。
- `<rel>_count()` - `u64` です。`with_count(["..."])` が呼ばれていない場合はパニックします。
- `<rel>_sum_of(col)` / `<rel>_avg_of(col)` - `Option<f64>` を返します（マッチする `with_sum` / `with_avg` が呼ばれていない場合は `None`）。
- `<rel>_min_of(col)` / `<rel>_max_of(col)` - `Option<Option<f64>>` を返します: 外側の `Option` は「`with_min` / `with_max` が呼ばれたか」であり、内側の `Option` は「グループが空だったためSQLがNULLを返したか」です。

これらのアクセッサーが、エルゴノミクスに優れた表面です - `__eager.get_aggregate::<T>(...)` に直接手を伸ばすのではなく、それらを通して読み取ってください。それらは、`eloquent::relations::aggregate_cache_key` を介して、裏側で同じキャッシュキーを構築します。

### 同じリレーションの上に集計を組み合わせる

広いキャッシュキーは、1つのクエリの中で、同じリレーションの上に、望むだけ多くの `with_*` の呼び出しを積み重ねられることを意味します - 衝突はありません:

```rust
let users = User::with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .with_min(("posts", "id"))
    .with_max(("posts", "id"))
    .get()
    .await?;

let u = &users[0];
let total_views: f64 = u.posts_sum_of("views").unwrap();
let avg_views:   f64 = u.posts_avg_of("views").unwrap();

// SQLのmin/maxは空のときにNULLになるため、Min/Maxは二重のOptionだ:
match u.posts_min_of("id") {
    None              => panic!("with_min not called"),
    Some(None)        => println!("no posts yet"),
    Some(Some(min))   => println!("smallest post id: {min}"),
}

// アクセッサーは、マッチする`with_*`がスキップされたときに`None`を返す:
assert!(u.posts_avg_of("score").is_none()); // col="score"で呼ばれたことはない
```

### 集計とINTEGERカラム

INTEGERカラムに対するSUMは、キャッシュの中で `f64` として収まります。ディスパッチャーのアームは、まず `try_get::<Option<f64>>` を試み、それから `try_get::<Option<i64>>().map(|n| n as f64)` にフォールバックします。SQLiteのINTEGERを保つCOUNT/SUMの型が、サイレントに `0.0` へ変換されてしまわないようにするためです。ソースのカラムの型にかかわらず、マクロが発行するアクセッサーを介して読み取ってください。

### `with_where` の述語ルーティング

`User::with_where_posts(|q| q.filter("published", true))` は、`filter_in(<fk>, parent_ids)` のINクエリが発行される**前**に、内側の `Builder<Post>` にクロージャを適用します。そのため、マッチする子の行だけがキャッシュに到達します。マクロは、宣言されたリレーションごとに、型付けされた `with_where_<rel>` という静的なヘルパーを1つ発行するため、クロージャの引数の型は、メソッドのシグネチャから推論されます。

ジェネリックな `with_where(("posts", |q: Builder<Post>| q.filter("published",true)))` も、それでも利用できます - リレーション名が実行時に計算される場合、あるいは、すでに `Builder<User>` を持っていて述語を付け加えたい場合に便利です。述語は `Box<dyn Any>` を経由し、Rustはリレーション名だけから型を推論できないため、クロージャの上でターゲットの型に名前を付けることが必要です。（Rustのorphanルールは、マクロが `Builder<User>` に直接型付けされたメソッドを追加することを禁じているため、型付けされた短縮形は、モデルの上でだけ提供されます - `User::with_where_<rel>` - ビルダー連鎖のメソッドとしてではありません。）

多態的な種類については、述語は関連テーブルのクエリに対して実行されます - ピボットの走査に対してではありません。

`with_where` は、`MorphTo` を**除く**すべてのリレーションの種類でサポートされています。MorphToのファミリーごとのenumは子の型を消去するため、単一の `Builder<R>` がすべてのバリアントをカバーすることはありません。MorphToを介したネストしたイーガーロードも、v1ではサポートされていません - `commentable` が `MorphTo` である場合の `with(["commentable.user"])` は、再帰イーガーロードのディスパッチャーからエラーを返します。

### `Collection::load` / `load_missing`

行をすでに取得していて、後からリレーションをイーガーロードしたい場合:

```rust
use suprnova::Collection;

let mut users: Collection<User> = User::all().await?.into();
users.load(["posts.comments"]).await?;
```

`load_missing` は行ごとです: コレクションの中の各行は、独立して分割されます。名前を指定されたリレーションをすでにキャッシュしている行は、触れられないままです。持っていない行は、リレーションがロードされます。Laravelの `$collection->loadMissing(...)` のセマンティクスを反映しています。

ネストしたパスについては、この分割がすべてのレベルで繰り返されます。`load_missing(["posts.comments"])` が与えられた場合:

- `posts` をキャッシュしていない行は、フルパスがロードされます - `posts` とその `comments` です。
- `posts` をすでにキャッシュしている行は、キャッシュされたpostへ再帰し、すでにcommentsをキャッシュしていないpostにだけ、`comments` をロードします。

同じ行ごとの分割は、より長いドット区切りのパス（`"posts.comments.author"` など）の、さらなる各セグメントで繰り返されます - 各ステップで、そのセグメントを欠いている行だけが、まとめてロードされます。

## ページネーション

3つのページネータの型が、`Builder<M>` の上に組み合わされます:

| メソッド | 戻り値 | ページごとのクエリ | 使うべきとき |
|--------|---------|------------------|----------|
| `paginate(per_page)` | `LengthAwarePaginator<M>` | 2（COUNT + LIMIT） | UIが総ページ数を必要とする |
| `simple_paginate(per_page)` | `Paginator<M>` | 1（LIMIT + 1） | 大きなテーブル。「次へ」ボタンのみ |
| `cursor_paginate(per_page)` | `CursorPaginator<M>` | 1（LIMIT + 1） | 無限スクロール。深いページネーション |

3つとも、Laravel標準のJSON形で `Serialize` を実装しているため、形を変えることなく、そのままInertia / JSONの消費者へ出荷されます。

### 長さを把握する

```rust
use suprnova::LengthAwarePaginator;

let page: LengthAwarePaginator<User> = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;

// page.data: Vec<User>
// page.total: u64 - すべてのページをまたぐ、行の総数
// page.last_page: u64 - 1始まりの、最後のページのインデックス
// page.current_page: u64
// page.per_page: u64
// page.from / page.to: Option<u64> - 1始まりのウィンドウの境界
// page.path: Option<String> - リンク生成のための、任意のベースURL
```

ページパラメータの解析は、`Context::query_param` を介して、アクティブなリクエストから `?page=N` を読み取ります。同じページ上で、複数のリストを、それぞれ独自のクエリキーでページネートするには、`paginate_using` を使ってください:

```rust
let posts = Post::query().paginate_using("posts_page", 10).await?;
let comments = Comment::query().paginate_using("comments_page", 25).await?;
```

**JSONの形:**

```json
{
  "data": [...],
  "current_page": 1,
  "last_page": 3,
  "per_page": 10,
  "total": 25,
  "from": 1,
  "to": 10,
  "path": "/api/users"
}
```

`path` は、設定されていないときはJSONから省かれます。

### シンプルなページネーション（カウントなし）

`paginate` は常に2つのクエリを実行します - `COUNT(*)` に加えて、ページの取得です。大きなテーブルでは、カウントだけでリクエストの時間を支配してしまうことがあります。`simple_paginate` は、カウントを完全にスキップします。代わりに `per_page + 1` 行を取得し、`has_more` フラグを介して、次のページが存在するかどうかを報告します:

```rust
use suprnova::Paginator;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;

// page.has_more: bool - per_pageを超える余分な行があったか？
// page.current_page、page.per_page、page.data、page.path: 上記と同じ。
```

**JSONの形:**

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true
}
```

### カーソルページネーション（キーセット）

カーソルページネーションは、無限スクロール、深いページネーション、あるいは、数値のページUIよりも、安価な1ページあたりO(1)のシークを伴う安定した行の順序の方が価値がある場所のための選択です。双方向です - `?cursor=<opaque>` というクエリパラメータを読み取り、カーソルの方向に前後へ歩き、ページの隣が存在するのに応じて、`next_cursor` と `prev_cursor` の両方を発します（Laravelの `cursorPaginate()` に一致します）。

```rust
use suprnova::CursorPaginator;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;

// page.data: Vec<User>
// page.per_page: u64
// page.next_cursor: Option<String> - 次のページのための不透明なカーソル（最後のページではNone）
// page.prev_cursor: Option<String> - 前のページのための不透明なカーソル（最初のページではNone）
// page.path: Option<String>
```

カーソルは、`CursorPaginator::encode_value` を介して**暗号化され認証されています** - それらは、キーセットの境界（モデルの主キー）に方向タグを加えたものをエンコードし、フレームワークの `APP_KEY` でAES-256-GCM封印します。改ざんは400のParamParseエラーを生じます。カーソルはクライアントにとって不透明であり、キーなしでは偽造できません。

次のリクエストは、`?cursor=<opaque>` を介してカーソルを渡します:

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

カーソルページネーションは、ビルダー上の既存の `ORDER BY` を**置き換えます** - `gt(boundary)` が決定的に切り出すためには、安定したPK ASCの順序が必要です。

**JSONの形:**

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` と `prev_cursor` は、常にJSONのキーとして存在します（存在しない場合は `null` として発行されます）。そのため、クライアントのスキーマは、そのフィールドの存在をあてにできます。`path` は、設定されていないときは省かれます。

### エラー

| 条件 | バリアント | HTTP |
|-----------|---------|------|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| 無効なカーソル（不正なbase64、JSON、あるいはHMACの失敗） | `Crypt::decrypt_string` からの `FrameworkError::Internal` | 500 |
| 背後にあるDBの失敗 | `FrameworkError::Database` | 500 |

カーソルの認証の失敗は、`ParamParse` ではなく `Internal` として表面化します。改ざんされたカーソルが、プロトコルレベルの情報をクライアントへ漏らさないようにするためです。それでも、レスポンスの本体は、人間が読める理由を運びます。

### 本物のリクエストの外側でクエリパラメータを読み取る

テスト、コンソールコマンド、そしてバックグラウンドのワーカーは、hyperのリクエストの内側では動きません - そのため `Context::query_param("page")` は `None` を返し、`paginate` はページ1にフォールバックします。特定のページを運用する必要があるテストは、スレッドごとのオーバーライドをインストールできます:

```rust
use suprnova::context::Context;

#[tokio::test]
async fn paginate_page_2() {
    Context::test_clear_query();
    Context::test_set_query("page", "2");

    let page = User::query().paginate(10).await.unwrap();
    assert_eq!(page.current_page, 2);

    Context::test_clear_query();
}
```

`test_set_query` / `test_clear_query` は `testing` フィーチャー（`framework/Cargo.toml` でデフォルト有効）の背後にゲートされているため、リリースビルドはこの表面を決して目にしません。

## チャンクと遅延反復

`Builder<M>` の上の7つのストリーミングのエントリーポイントは、境界のあるメモリの中で、大きな結果セットを処理させてくれます。トレードオフで選んでください:

| メソッド | ページネーション | 並行安全か？ | 戻り値 |
|--------|-----------|------------------|---------|
| `chunk(n, async \|batch\| { ... })` | OFFSET | いいえ | `Result<(), _>` |
| `chunk_by_id(n, async \|batch\| { ... })` | PKカーソル | **はい** | `Result<(), _>` |
| `chunk_map(n, async \|batch\| { ... })` | OFFSET | いいえ | `Collection<U>` |
| `each(async \|row\| { ... })` | OFFSET、サイズ1 | いいえ | `Result<(), _>` |
| `lazy()` | PKカーソル、バッチ1000 | **はい** | `LazyCollection<M>` |
| `lazy_by_id(batch_size)` | PKカーソル、カスタムバッチ | **はい** | `LazyCollection<M>` |
| `cursor()` | `lazy()` のエイリアス | **はい** | `LazyCollection<M>` |

### chunk - OFFSETページネートされたバッチ

```rust
use suprnova::{Collection, Model};

User::query().chunk(100, |batch: Collection<User>| async move {
    for user in &batch {
        send_welcome_email(user).await?;
    }
    Ok(())
}).await?;
```

クロージャは、バッチごとに `Collection<M>` を受け取ります - スライス形のアクセス（`.iter()`、インデックス付け）は、`Deref` を介して直接動作します。

`chunk` はOFFSETでページネートされており、**並行する挿入の下では安全ではありません**: 次のバッチのオフセットより前に挿入された行はスキップされ、オフセットより前に削除された行は（そのスロットにシフトしてきたものが）2回処理されます。書き込み負荷のあるテーブルに対する本番グレードの一括処理には、`chunk_by_id` を使ってください。

### chunk_by_id - PKカーソルバッチ、並行安全

```rust
User::query().chunk_by_id(500, |batch| async move {
    for user in &batch {
        reindex_user(user).await?;
    }
    Ok(())
}).await?;
```

各バッチは `WHERE id > last_id ORDER BY id ASC LIMIT n` でフィルタするため、反復の途中で挿入された、カーソルより大きいPKを持つ行は、より後のバッチに収まります（あるいは、後続の実行で拾われます） - それらが、元の行のスキップや重複を引き起こすことは決してありません。

`chunk_by_id` は `i64` の主キーを要求します。`String` / `Uuid` のPKを持つモデルは、OFFSETの注意点を伴う `chunk` を使ってください。（カーソルの形を `i64` 以外のキーへ一般化することは、後続のリストに載っています。）

### chunk_map - chunk + チャンクごとのmap

```rust
let totals: Collection<i64> = Order::query()
    .chunk_map(1000, |batch| async move {
        let sum: i64 = batch.iter().map(|o| o.amount).sum();
        Ok(Collection::from_vec(vec![sum]))
    })
    .await?;
```

各バッチを `f` に通してmapし、mapされた出力を連結し、単一の `Collection<U>` を返します。メモリの境界があるのは、`U` が `M` より厳密に小さい場合だけです - 変換された行ではなく、要約（バッチごとの合計、id、集計）を生成している場合に、これを選んでください。

### each - 1行ずつ、OFFSET

```rust
User::query().each(|user| async move {
    send_welcome_email(&user).await?;
    Ok(())
}).await?;
```

`chunk(1, ...)` のシュガーです - 行ごとに1クエリです。大きなデータセットについては、`lazy()` に切り替えてください。これは、消費者に対しては1行ずつ表面化させつつ、内部では（デフォルトでは1回の取得あたり1000行を）バッチ処理します。

### lazy / lazy_by_id / cursor - ストリーム

```rust
let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

`lazy()` は `LazyCollection<M>` を返します - 行ごとに `Result<M,FrameworkError>` を生成する `Send` なストリームのラッパーです。バックプレッシャーは自然に働きます: 遅い消費者は `await` の地点で待機し、次のバッチは、メモリ上のバッファが空になったときにだけ取得されます。

`lazy()` は、デフォルトサイズ1000行のPKカーソルを介してバッチ処理します。バッチサイズは `lazy_by_id(500)` で上書きしてください。`cursor()` はLaravelの名前であり、`lazy()` のゼロコストなエイリアスです。

`chunk_by_id` と同じ `i64`-PKの制約です。

### チャンクの内側でのイーガーロード

7つのエントリーポイントすべては、はっきりとした `FrameworkError::internal` で、`.with(...)` を前もって**拒否します**。Builderのバッチをまたぐクローンは、型消去されたイーガーロードの計画を落とします（そのboxed-`dyn Any` の述語は、公開APIを狭めない限りクローンできません）。そのため、その計画を尊重すると、バッチをまたいでサイレントに不整合になってしまいます。必要な場合は、チャンクごとのクロージャの内側で `.with(...)` を再適用してください - 各バッチの `Collection<M>` は、`load(...)` / `load_missing(...)` と合成します:

```rust
User::query().chunk(100, |batch| async move {
    let mut batch = batch;
    batch.load("posts").await?;
    for u in &batch {
        let posts = u.posts_loaded();
        // ...
    }
    Ok(())
}).await?;
```

## コレクション

`Collection<T>` は、SuprnovaのLaravel形のコレクションです - `Builder::get`（`T` がモデルである場合）、`Model::all`、`pluck` / `chunk_map`、そして1行を超える結果を生成するその他すべての終端メソッドの、戻り値です。それは `&[T]` へderefするため、既存のVecの呼び出し箇所は、変更なしに動作し続けます。Laravelの表面は、その上に重ねられています。この節は日常的な表面です。完全なメソッドの索引、ジェネリック対モデルの分岐、`LazyCollection<M>` のストリーミングのラッパー、そして借用対消費の規則は、[Eloquent コレクション](eloquent-collections.md)にあります。

### ジェネリックな表面

`T` にかかわらず、すべての `Collection<T>` で利用できます:

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9]);

nums.first();              // Some(&3)
nums.last();               // Some(&9)
nums.len();                // 6
nums.is_empty();           // false
nums.contains(&4);         // true
// 述語のクロージャは`&&T`を受け取る - 二重デリファレンスの`**n`に注意:
nums.first_where(|n| **n > 3);    // Some(&4)
nums.contains_where(|n| **n > 8); // true
// カウントのためには、述語をインラインで実行する: `nums.iter().filter(|n| **n > 2).count()` - 4
```

変換は `self` を消費し、新しい `Collection` を返します:

```rust
let doubled: Collection<i32> = nums.clone().map(|n| n * 2);
let evens:   Collection<i32> = nums.clone().filter(|n| n % 2 == 0);
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(2); // [[3,1],[4,1],[5,9]]
let unique:  Collection<i32> = nums.clone().unique();
let sorted:  Collection<i32> = nums.clone().sort();
```

### `Collection<M>` の上のモデルを意識したメソッド

`T` がモデルであるとき、追加の文字列キーのメソッドが、マクロが発行する `field_value(name)` アクセッサーを経由してルーティングされます:

```rust
let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let by_role: HashMap<String, Vec<User>> =
    users.clone().group_by::<String>("role");
let active: Collection<User> = users.clone().where_eq("active", true);

let total: f64 = users.clone().sum::<f64>("balance");
let avg:   f64 = users.clone().avg::<f64>("balance");
let max:   Option<i64> = users.clone().max::<i64>("login_count");
```

クロージャベースの `pluck_by` は、型付けされた代替です - フィールド名が、そうでなければ型システムがチェックできない文字列ルックアップを必要とする場合に便利です:

```rust
let names: Collection<String> = users.pluck_by(|u| u.name.clone());
```

行ごとの `field_value(name)` は `Option<serde_json::Value>` を返します - カラム名がどの宣言済みフィールドにもマッチしない場合は `None` です。シリアライズに失敗するカスタムキャストも、`None` として表面化します。文字列キーのメソッドは、それらの行をサイレントにスキップします。クロージャの形式は、呼び出し元が判断できるように、クロージャの本体の中で短絡します。

### `LazyCollection` を介したストリーミング

実体化するには大きすぎるデータセットについては、`Builder::lazy()` /`lazy_by_id(n)` / `cursor()` が `LazyCollection<M>` を返します - PKカーソルのバッチで行を取得する `Stream` のラッパーです。[チャンクと遅延反復](#チャンクと遅延反復)を参照してください。

### コレクションに対するイーガーロード

`Collection::load(["posts"])` / `load_missing(["posts"])` は、`Builder::with(...)` の連鎖が発行するのと同じイーガーロードのディスパッチを実行しますが、既存のコレクションに対してです。`load_missing` は行ごとです: コレクションの中の各行は、「ロードが必要」/「すでにロード済み」のバケットに分割され、欠けているものだけが、まとめてロードされます。[イーガーロード](#イーガーロード)を参照してください。

## 一括代入

### Fillableの許可リスト

```rust
#[model(
    table = "users",
    fillable = ["name", "email"],
)]
pub struct User { /* ... */ }

User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    admin: true,    // 実行時にサイレントに落とされる - fillableにない
}).await?;
```

### Guardedの拒否リスト

`guarded` はその逆です - guardedにあるもの**を除く**すべてのフィールドがfillableです。`fillable` とは排他的です。両方を同時に使うと、マクロからのコンパイル時エラーになります。

```rust
#[model(
    table = "posts",
    guarded = ["id", "user_id"],   // 他のすべてはfillable
)]
pub struct Post { /* ... */ }
```

### デフォルトのポリシー

`fillable` も `guarded` も設定されていない場合、デフォルトのポリシーは `guarded = ["id"]`（あるいは `primary_key = "..."` が解決する何であれ）です - 主キーを除くすべてのフィールドがfillableです。これは、Laravelの「PKを除くすべてのフィールドがfillable」というデフォルトと一致します。

### `unguarded(closure)` エスケープハッチ

`unguarded(closure)` は、あるブロックについて、フィルタをオフにします:

```rust
use suprnova::eloquent::unguarded;

// ワンショットのデータ移行スクリプトのために、フィルタをバイパスする:
unguarded(|| async {
    User::create(attrs! {
        name: "Bootstrap",
        email: "boot@example.com",
        admin: true,    // クロージャの内側では代入可能
    }).await
}).await?;
```

実装: `Fillable::apply` フィルタが、実行前にチェックする `tokio::task_local!` の真偽値です。タスクローカルであるということは、並行するリクエストが、他のタスクの `unguarded` スコープの影響を受けないということです。

## キャスト

キャストは、ストレージ（カラムの値）とランタイム（モデルのフィールド）の境界で実行されます。各キャストの型は、`Cast` トレイトを実装します。組み込みのキャストは、Laravelの完全な集合をカバーします。ユーザーは、そのトレイトを介してカスタムキャストを登録します。この節はクイックリファレンスの索引です。キャストごとの完全な契約 - プリミティブ、時間系、構造化、enum、暗号化、ハッシュ化、そして `casts!` の実行時オーバーライドマクロ - は、[Eloquent キャスト、アクセッサーとミューテータ](eloquent-mutators.md)にあります。

### 明示的のみ

キャストは `#[model(casts = { ... })]` の中で宣言されます - フィールドの型からの自動検出はありません。`prefs: Json` フィールドは、暗黙的に `AsJson` になったりしません。`casts = { prefs = AsJson }` と書きます。根拠: モデルを読めば、ストレージの境界で何が実行されるのかが、正確に分かるべきだということです。魔法はありません。

### 例

```rust
use suprnova::{model, AsArray, AsBool, AsCollection, AsDate, AsDateTime,
    AsEncrypted, AsEnum, AsObject, AsTimestamp};

#[model(
    table = "users",
    casts = {
        active        = AsBool,
        preferences   = AsArray<String>,
        options       = AsObject<UserOptions>,
        profile       = AsCollection<ProfileField>,
        birthday      = AsDate,
        last_seen_at  = AsDateTime,
        role          = AsEnum<UserRole>,
        api_token     = AsEncrypted,
    },
)]
pub struct User { /* ... */ }
```

### Laravelのキャストの完全なリストと、Suprnovaへの対応

| Laravelのキャスト | Suprnovaのキャスト | ランタイムの型 |
|--------------|---------------|--------------|
| `bool`, `boolean` | `AsBool` | `bool` |
| `int`, `integer` | `AsInt<I>` | `I: PrimInt` |
| `float`, `double`, `real` | `AsFloat` | `f64` |
| `decimal:N` | `AsDecimal<N>` | `rust_decimal::Decimal` |
| `string` | `AsString` | `String` |
| `array` | `AsArray<T>` | `Vec<T>`（JSONエンコードされる） |
| `object` | `AsObject<T>` | `T: Serialize + DeserializeOwned` |
| `collection` | `AsCollection<T>` | `Collection<T>` |
| `json` | `AsJson<T>` | `T`（生のJSONカラム） |
| `date`, `date:format` | `AsDate` | `chrono::NaiveDate` |
| `datetime`, `datetime:format` | `AsDateTime` | `chrono::DateTime<Utc>` |
| `immutable_date` | `AsImmutableDate` | `chrono::NaiveDate` |
| `immutable_datetime` | `AsImmutableDateTime` | `chrono::DateTime<Utc>` |
| `timestamp` | `AsTimestamp` | `i64`（unixエポック） |
| `encrypted` | `AsEncrypted` | `String`（`Crypt` を介して暗号化される） |
| `encrypted:array` | `AsEncryptedArray<T>` | `Vec<T>`（JSON + 暗号化） |
| `encrypted:object` | `AsEncryptedObject<T>` | `T`（JSON + 暗号化） |
| `encrypted:collection` | `AsEncryptedCollection<T>` | `Collection<T>` |
| `EnumClass::class` | `AsEnum<E>` | `E: EnumString + AsRefStr` |
| `AsArrayObject::class` | `AsArrayObject<T>` | `IndexMap<String, T>` |
| `hashed` | `AsHashed` | `String`（書き込み時に `Hash::make`。復号は決してしない） |

合計22個のキャストです。大半はLaravelと1対1で対応します。`AsOptionalDateTime`（`soft_deletes` が使うもの）は、ソフトデリートのカラムが `Option<DateTime<Utc>>` であるとき、マクロによって自動的に注入されます。

### 暗号化されたキャストの失敗モード

4つの `AsEncrypted*` キャストは、すべての暗号化/復号を（`APP_KEY` をキーとする）`Crypt` ファサードを介してルーティングします。復号が失敗したとき - 間違ったキー、切り詰められた暗号文、改ざんされたバイト、AEADタグの不一致 - キャストは、`Cast::from_storage` から、はっきりとした `FrameworkError::Internal` を表面化させます。ゴミへのサイレントなフォールバックはありません:

- `Model::find` / `Model::query()` を介した行のロードは、復号のエラーを伝播させ、（マクロが生成する `From<inner::Model>` に従って）`cast from_storage failed - corrupt data in database column` でパニックします。運用者は、失敗を即座にログで目にします。モデルが、もっともらしいが間違った平文を運ぶことは決してありません。
- `AsHashed` キャストは一方向です。決して復号しないため、この失敗モードは当てはまりません。

これは、Laravelの `encrypted` キャストと一致します: 既存の暗号化されたカラムに対する間違った `APP_KEY` は、静かな `null`/空文字列になることは決してなく、ハードエラーです。

### `APP_KEY` のローテーション

Suprnovaは、キーの*リング*を介した、ゼロダウンタイムのキーローテーションをサポートします: 現在の `APP_KEY` が暗号化します。任意の `APP_KEY_PREVIOUS` 環境変数（カンマ区切りで、古いものから新しいものへ）は、より古いキーの下で書かれたデータのための、復号のフォールバックを提供します。暗号化は*常に*現在のキーを使います - 以前のキーは、復号のときにだけ関与します。

過去のキーへフォールスルーする各復号は、過去のキーのインデックスを含む `tracing::warn!` の行を発します。ログのペイロードは、平文と暗号文のどちらも意図的に含みません - ローテーションが起きたという事実と、実行可能な再暗号化のヒントだけです。

**ローテーションの手順**（ゼロダウンタイム、本番環境でも安全）:

1. 新しいキーを発行します: `suprnova key:generate`（標準出力に書き込みます）。
2. 古いキーを `APP_KEY_PREVIOUS` へ移し、`APP_KEY` を新しい値に設定します:
   ```
   APP_KEY_PREVIOUS=<old_key>
   APP_KEY=<new_key>
   ```
3. デプロイします。新しい書き込みは新しいキーを使い、既存の行は、過去のキーのフォールバックを介して、復号され続けます。ログの警告は、まだ `APP_KEY_PREVIOUS` に依存しているカラムを特定します。
4. 再暗号化のパスを実行します。暗号化されたキャストを持つ各モデルについて:
   ```rust
   for chunk in User::query().chunk(500).await? {
       for user in chunk {
           // Touch + saveは、現在のキーの下で、すべてのキャストされた
           // カラムを書き直す。`Cast::to_storage` は常に、現在の
           // リングのエントリに手を伸ばす。
           user.save().await?;
       }
   }
   ```
   これはべき等です - すでに新しいキーの上にある行は、何もしないだけです。
5. ログに `APP_KEY_PREVIOUS` の警告が現れなくなったら（バッチと、ソフトデリートされた/アーカイブされたデータに、十分な余裕を与えてください）、環境から `APP_KEY_PREVIOUS` を削除し、再度デプロイします。

**複数段のローテーション。** 前のパスを完了する前に再度ローテーションする場合は、追記してください: `APP_KEY_PREVIOUS=<oldest>,<previous>`。リングは、過去のキーを順番にすべて試します。このリストは8エントリまでという上限があります - 現実的な連鎖は1から3エントリです（進行中のローテーションが1つ、運用者が片付けていない停滞した以前のローテーションがおそらく1つ）。それより長いリストは、ほとんど常に設定テンプレートの事故です。上限を超えると、運用者がまだ依存しているかもしれないキーをサイレントに落とすのではなく、実行可能な診断とともに起動が失敗します。

**制約。**

- `APP_KEY_PREVIOUS` の中の不正な形式のエントリは、（不正な形式の `APP_KEY` と同じように）起動をはっきりと失敗させます - 半端にローテーションされたシークレットが、サイレントに劣化することは決してあってはなりません。
- `APP_KEY_PREVIOUS` の中に8個を超えるエントリがあると、起動をはっきりと失敗させます - [`suprnova::crypto::MAX_PREVIOUS_KEYS`] を参照してください。
- リストの中の空のエントリ（例えば、テンプレート化された設定からの末尾のカンマ）は、「このスロットにはキーがない」として許容されます - エラーではありません。
- 通信上の形式は、ローテーション前の単一キーのレイアウトから変わっていません: 暗号文にキーの識別子は埋め込まれていません。リングは、1つが成功するまで、各キーを順番に試し復号します。

### 実行時のキャストオーバーライド - `with_casts`

```rust
let users = User::query()
    .with_casts(suprnova::casts! { birthdate = AsDateTime })
    .get()
    .await?;
```

`with_casts` は、単一のクエリの間だけ、モデルの宣言済みのキャストをオーバーライドします - join / view / `select_raw` から生のカラムが返ってきて、モデルのデフォルトとは異なる型変換が必要な場合に便利です。

### カスタムキャスト

カスタムキャストは `Cast` を実装します:

```rust
use suprnova::eloquent::casts::Cast;
use suprnova::FrameworkError;

pub struct AsAesGcmJson<T>(std::marker::PhantomData<T>);

impl<T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync> Cast
    for AsAesGcmJson<T>
{
    type Runtime = T;
    type Storage = String;
    fn to_storage(value: &T) -> Result<String, FrameworkError> { /* ... */ }
    fn from_storage(stored: &String) -> Result<T, FrameworkError> { /* ... */ }
}

#[model(casts = { secret = AsAesGcmJson<SecretBundle> })]
pub struct Vault { /* ... */ }
```

`Cast` トレイトは、プリミティブなキャストと並んで出荷されています。カスタムキャストは、（JSONエンコードする場合の）`String` のストレージか、SeaORMがサポートするスカラー型（`i64`、`f64`、`bool`、`Vec<u8>`）のいずれかを使えます。

## アクセッサーとミューテータ

### アクセッサー

```rust
#[model(
    table = "users",
    appends = ["full_name"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    // ...
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }
}
```

`user.to_array()` が実行されるとき（あるいは、それに委譲する `user.to_json()` が実行されるとき）、`full_name` アクセッサーが呼び出され、その戻り値がJSON出力へ挿入されます。Rustから `user.full_name()` を呼ぶのは、単なる通常のメソッド呼び出しです。

### ミューテータ

ミューテータは、ストレージの前に実行されます:

```rust
#[model(
    table = "users",
    fillable = ["first_name", "last_name", "password"],
    mutators = ["password"],
)]
pub struct User { /* ... */ }

impl User {
    #[mutator]
    pub fn set_password(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), suprnova::FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            suprnova::FrameworkError::validation("password", format!("{e}"))
        })?;
        self.password = hash::make(&raw);
        Ok(())
    }
}
```

`user.password = "secret".into()` を呼ぶと、ミューテータを実行することなく、生の値を直接代入します。ミューテータの経路を実行するには、`user.set_password(json!("secret"))` を呼ぶか、JSONの経路（`user.fill(attrs!{password: "secret"})`）を使ってください。`"password"` が `mutators = [...]` に列挙されているため、これは自動的にミューテータを経由してルーティングされます。

### ルーティングの仕組み

- **シリアライゼーション（`to_array` → `Value`、`to_json` → `String`）**は、アクセッサーを実行します。`appends = [...]` に列挙された各フィールド名は、`self.<name>()` の呼び出しになります。戻り値はJSON出力へ挿入されます。`to_json()` は薄いラッパーです: `serde_json::to_string(&self.to_array())`。
- **Fill形の書き込み（`fill`、`create`、`update`）**は、ミューテータを経由してルーティングされます。`mutators = [...]` に列挙された各フィールド名は、直接の代入の代わりに、`self.set_<field>(value)` の呼び出しになります。

関数レベルの `#[accessor]` と `#[mutator]` のマクロは、マクロのシリアライゼーション/fillの経路が走査する、レジストリのエントリを発行します。

### 不正な形式の値はエラーになる。デフォルトにはならない

そのフィールドの型へデコードできない値は、書き込みを失敗させ、そのフィールドに名前を付けます:

```rust
let err = user.fill(attrs! { age: "not a number" }).unwrap_err();
// ValidationError { field: "age", message: "could not decode the
// supplied value: invalid type: string \"not a number\", expected i32" }
```

モデルは触れられないままです - 拒否された `fill` は、何も適用しません。

近縁の2つのケースは、意図的に異なる振る舞いをします:

- **未知のカラム**は、それでもサイレントにスキップされます。Laravelの `$model->fill()` と一致します。カラムについて知らないことは、知っているカラムに壊れた値を渡されることと、同じではありません。
- `fillable` / `guarded` によって除外されたカラムは、デコードの**前**に、一括代入のフィルタによって落とされます。そのため、呼び出し元が設定できないかもしれないフィールドについての不正な形式の値も、サイレントです。そこでエラーにしてしまうと、認可されていない呼び出し元に、どのカラムが存在するかを教えてしまいます。

数値の拡大は型エラーではありません: JSONの整数は、`f64` のフィールドへ通常どおりデコードされます。

> v0.8.0より前は、不正な形式の値は、そのフィールドの `Default` にサイレントに置き換えられ、呼び出しは `Ok` を返していました - `fill(attrs!{ age: "abc" })` は `age = 0` を設定し、成功を報告していました。その変換に依存していた場合は、`fill` を呼ぶ前に、バリデーションあるいは変換を行ってください。

### Hidden / visible

```rust
#[model(
    table = "users",
    hidden = ["password", "remember_token"],
)]
pub struct User { /* ... */ }
```

`hidden = [...]` は拒否リストです - 列挙されたものを除くすべてのカラムがシリアライズされます。`visible = [...]` は包含的な形です - 列挙されたものだけがシリアライズされます。コンパイル時に排他的です。

## タイムスタンプ

`created_at` と `updated_at` の両方のカラムが存在するとき、マクロはそれらを自動検出し、タイムスタンプの追跡を有効化します:

- `created_at` は、新しい行について、`save()` の際に `Utc::now()` に設定されます。
- `updated_at` は、すべての `save()` の際に `Utc::now()` に設定されます。

この自動検出は控えめです: 構造体が2つのカラムのうち片方だけを持つ場合、マクロはエラーを出します。タイプミス（`craeted_at`）が、タイムスタンプをサイレントに無効化してしまわないようにするためです。完全にオプトアウトするには、`timestamps = false` を設定してください。

### 自動タイムスタンプを無効化する

```rust
#[model(table = "audit_logs", timestamps = false)]
pub struct AuditLog {
    pub id: i64,
    pub event: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    // updated_atフィールドはない - しかしtimestamps = falseは、
    // マクロの「1つのカラムしか見つからない」というエラーも黙らせる。
}
```

### `touch()` - 他を変更せずにupdated_atを進める

```rust
user.touch().await?;
```

`touch()` は `UPDATE table SET updated_at = ? WHERE pk = ?` を発します - アトミックであり、read-modify-writeはありません。マクロは、タイムスタンプを持つすべてのモデルの上に、`Touchable` の実装を発行します。

### 親のtouch

```rust
#[model(
    table = "comments",
    touches = ["post"],
    timestamps,
)]
pub struct Comment {
    pub id: i64,
    pub post_id: i64,
    // ...
}
```

`touches = [...]` のリストはパースされ、`TOUCHES` 定数としてモデルの上に格納されます。commentの保存の後に自動的に `self.post().touch().await?` を呼ぶはずの、保存後フックは、まだ配線されていません - 今のところは、オブザーバーやあなたのハンドラから、明示的に親の `.touch()` を呼んでください。メタデータはすでに配置されているため、後で切り替えることは、振る舞いの変更であり、APIの変更ではありません。

### 形式

常にUTCでのISO 8601です。`Model::$timestampsFormat` の上書きはありません（Eloquentとの相違点の表のとおりです - フロントエンドとの相互運用が優先され、ロケールの書式はi18nの層に属します）。

## オブザーバーとライフサイクルイベント

すべてのモデルは、`create` / `save` / `update` / `delete` / `restore` /`replicate` / Builderのクエリの経路を通過する際、固定された16イベントのライフサイクルを経ます。リスナーは、各イベントにフックして、ログを記録し、監査し、副作用を起こし、バリデーションし、あるいは実行中の操作をキャンセルできます。

### 16個のライフサイクルイベント

イベントは、キャンセル可能性によって、2つのグループに分かれます:

**キャンセル可能（5個）** - データベースへの書き込みの**前**に発火します。`EventResult::cancel("reason")` を返すリスナーは、`FrameworkError::bad_request(reason)` で操作を中断させます。

| イベント    | いつ                                      | ペイロード                                                 |
|-------------|-------------------------------------------|---------------------------------------------------------|
| `Saving`    | `create` と `save` の両方の前            | `Arc<Mutex<Attrs>>` + `is_creating: bool`               |
| `Creating`  | `create` の前                            | `Arc<Mutex<Attrs>>`                                     |
| `Updating`  | 既存の行に対する `save` / `update` の前   | 更新前のモデルのスナップショット + `Arc<Mutex<Attrs>>`   |
| `Deleting`  | `delete` の前（ソフトあるいはハード）     | Model + `is_force: bool`（ソフトデリートの上でのforce-delete） |
| `Restoring` | ソフトデリートされたモデルに対する `restore` の前 | Model                                             |

**キャンセル不可能（11個）** - 操作の**後**に発火します。リスナーのエラーは伝播しますが、すでに完了した書き込みを止めることはできません。

| イベント        | いつ                                              | ペイロード                        |
|-----------------|---------------------------------------------------|----------------------------------|
| `Retrieving`    | Builderのクエリごとに1回、DB呼び出しの前          | なし                              |
| `Retrieved`     | Builderのクエリが返す行ごとに1回                  | Model                            |
| `Created`       | 成功した `create` の後                            | Model                            |
| `Updated`       | 成功した `save` / `update` の後                   | 更新前 + 更新後のスナップショット |
| `Saved`         | `create` と `save` の両方の後                     | Model                            |
| `Deleted`       | 成功した `delete` の後                            | Model + `is_force: bool`         |
| `Trashed`       | ソフトデリートの後（force-deleteでは**ない**）    | Model                            |
| `Restored`      | 成功した `restore` の後                           | Model                            |
| `Replicating`   | `replicate` / `replicate_except` の間、返す前（`replicate_into` ではない - ソースの型ごと） | ソース + `Arc<Mutex<replica>>`（可変） |
| `ForceDeleting` | ソフトデリートされたモデルに対する `force_delete` の前 | Model                        |
| `ForceDeleted`  | 成功した `force_delete` の後                      | Model                            |

キャンセル可能/キャンセル不可能の分割は、Laravelの `creating` 対 `created` というフックの組を反映しています。`Saving` は、挿入と更新のどちらでも発火します - 両方の経路で振る舞いが同一であるときは、それをオーバーライドし、`is_creating` を介して区別してください。

`Replicating` は、可変の参照を渡す唯一のキャンセル不可能なフックです（複製は `Arc<Mutex<M>>` です）。クローンが呼び出し元に返される前に、タイムスタンプをクリアしたり、UUIDを再生成したり、自動インクリメントをリセットしたりするために使ってください。

### オブザーバー対 素のリスナー

ライフサイクルイベントにフックする方法は、2つあります:

1. **素のリスナー** - 望む各イベントについて、`EventFacade::listen::<Created,_>(Arc::new(MyListener))` を呼びます。イベントごとに1つのimplです。これが基盤となる仕組みであり、オブザーバーはその上に乗っています。

2. **オブザーバー** - すべての16個のフックを、1つのトレイトの下に束ねます。マクロは、ユーザーがどのメソッドをオーバーライドしたかを見て、まさにそれらだけを登録します。フックの集合が自明でない場合は、これが推奨される経路です。

```rust
use async_trait::async_trait;
use suprnova::eloquent::attrs::Attrs;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::observers::Observer;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]   // <- #[async_trait]より前に置かなければならない
#[async_trait]
impl Observer<User> for AuditObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if attrs.get("email").is_none() {
            return EventResult::cancel("email is required");
        }
        EventResult::ok()
    }

    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user.id = user.id, "user created");
        Ok(())
    }
}
```

すべてのトレイトメソッドは、デフォルトで何もしない実装を持つため、implブロックには、あなたが関心を持つイベントだけが含まれます。マクロは、固定された16メソッドの集合に対する名前の一致によって、オーバーライドを識別します。オーバーライドしないメソッドは、リスナーを登録しません。

### 必須のアトリビュートの順序

`#[suprnova::observer(M)]` は、`#[async_trait]` の**上に**現れなければなりません:

```rust
#[suprnova::observer(User)]   // 外側 - 最初に実行され、生のasync関数を見る
#[async_trait]                // 内側 - async関数のシグネチャを書き換える
impl Observer<User> for AuditObserver { /* ... */ }
```

アトリビュートマクロは、外側から内側へ展開します。`async_trait` は、すべての `async fn` を、脱糖された `Pin<Box<dyn Future>>` のpoll-fn形へ書き換えます。`#[async_trait]` が先に実行されていたら、オブザーバーマクロの、16個のトレイトメソッド名に対する名前の一致は、何も見つけられず、サイレントにゼロ個のリスナーを発行してしまいます。

### 4つの登録経路

| 経路                                         | 使うべきとき                                         |
|----------------------------------------------|-----------------------------------------------------|
| `#[suprnova::observer(M)]`（インベントリ）       | コンパイル時に既知の静的なオブザーバーです。起動時に自動インストールされます。 |
| `#[model(observers = [Foo, Bar])]`           | ドキュメント + 列挙された型が解決することのコンパイル時検証です。それ自体は登録**しません**。 |
| `Model::observe(MyObs).await`                | 実行時の登録です。手動で駆動され、登録が設定に依存する場合に便利です。 |
| `EventFacade::listen::<events::Created, _>(...)` | 最も低いレベルです - 1度に1つのイベントです。オブザーバーが重く感じられるときに使ってください。 |

`#[model]` の上の `observers = [...]` アトリビュートは、ドキュメントのマーカーです。それは、列挙された各型が本物のRustの項目へ解決することを証明する `const _: fn() = || { let _ = ::std::any::type_name::<T>; ... };` ブロックへコンパイルされます。タイプミスは、モデルの宣言の場所で表面化します。実際のインストールは、インベントリの経路を介したものです - `Foo` の上の `#[observer(M)]` アトリビュートこそが、`Foo` を自動インストールのために登録するものです。

### ブートストラップ

起動時に一度、`bootstrap_observers()` を呼んで、インベントリを空にし、`#[observer(M)]` で登録されたすべてのオブザーバーをインストールしてください:

```rust
suprnova::eloquent::observers::bootstrap_observers().await?;
```

この空にする処理は、インベントリの経路についてはべき等です - 各オブザーバーのインストール用クロージャは、型ごとの `AtomicBool`（T2bのマクロ発行）によってゲートされているため、`bootstrap_observers()` を2回呼んでも、二重に登録されることはありません。

実行時の `Model::observe(MyObs)` のシムは、ゲートされて**いません**。それを2回呼ぶと、2つのリスナーの集合が登録されます。Laravelの手動の `Model::observe(MyObs::class)` のセマンティクスと一致します。手動で駆動されるオブザーバーが `#[observer]` も持っている場合、インベントリのアダプターは、手動でインストールされたものに加えて発火します。

### オブザーバーからキャンセルする

5つのキャンセル可能なフックは `EventResult` を返します。操作を中断するには、`EventResult::cancel("reason")` を返してください:

```rust
#[suprnova::observer(Subscription)]
#[async_trait]
impl Observer<Subscription> for PolicyObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if let Some(plan) = attrs.get("plan") {
            if plan == "blocked" {
                return EventResult::cancel("plan is blocked");
            }
        }
        EventResult::ok()
    }
}
```

キャンセルの理由は、`Subscription::create` から `FrameworkError::bad_request(reason)` として表面化します。その行がデータベースに収まることは決してありません - キャンセルは、本物の中断であり、「後からの削除」ではありません。

複数のオブザーバーが、同じモデルにキャンセル可能なフックを登録できます。そのうちのどれか1つが `Cancel` を返せば、操作は止まります。順序は、インベントリへの登録順です（実際にはリンクの順序です）。

### 1つのモデルの上の複数のオブザーバー

複数の `Observer<M>` の実装は、すべて同じイベントについて発火します - EventFacadeのディスパッチは、1つを選ぶのではなく、登録されているすべてのリスナーへファンアウトします:

```rust
#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for AuditObserver { /* ... */ }

#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for NotifyObserver { /* ... */ }

// Comment::create(...)は、AuditObserver::createdとNotifyObserver::createdの両方を発火させる。
```

これはLaravelのファンアウトのセマンティクスと一致し、「関心ごとにフックを分解する」パターンを支える、荷重を支える特性です:`AuditObserver` は監査についてだけを知り、`NotifyObserver` は通知についてだけを知り、モデルの宣言は、いくつのオブザーバーが取り付けられているかを気にしません。

### 手動の `Model::observe()`

すべての `#[suprnova::model]` の構造体は、モデルごとの `observe<O>()` というシムを得ます。動的な登録のために、起動時にそれを呼んでください:

```rust
#[derive(Clone)]
struct MyObs;

#[async_trait]
impl Observer<User> for MyObs { /* ... */ }

// 実行時:
User::observe(MyObs).await;
```

そのシムの `O: Clone + 'static` という境界こそが、フレームワークが、16個の内部アダプターリスナーそれぞれに、新しいオブザーバーのクローンを手渡せるようにするものです。すべての16個のリスナーアダプターは、呼び出しごとにインストールされます - トレイトのデフォルトが、オーバーライドされていないメソッドを、安価な何もしない実装にします。

### 制約

- **マクロ版は、implブロックが、トレイトの16個のフックに一致する、素のメソッド名を使うことを要求します。** 名前を変えたメソッド、`#[allow]` で抑制されたデフォルト、そして `#[cfg]` でゲートされた本体は、名前の一致の外側に落ち、リスナーを登録しません。

- **マクロが検査するオブザーバーの構造体は、v1ではゼロサイズ**（フィールドなし）**でなければなりません。** マクロは、各アダプターの内側で `let obs = MyObserver;` を介してオブザーバーを構築します。（`Arc<Inner>` を運ぶ）状態を持つオブザーバーには、実行時の `Model::observe()` の経路が必要です。これは、オブザーバーを値として受け取り、各アダプターへそれをクローンします。

- **テストの分離: シナリオごとに一意なモデルの型を使う。** プロセスグローバルな EventDispatcherは、`User` のためにインストールされたリスナーが、同じバイナリの中のすべてのテストに見えることを意味します。テストごとに一意なモデルの型（`T2Comment`、`T2Subscription`、…）は、テストをまたぐ漏れを、カウンタのアサーションの外に保ちます。`eloquent_observers.rs` の統合テストが、このパターンを運用しています。

## Prunable

Laravelは、モデルが、スケジュールに従って削除する行のスコープを宣言できる `Prunable` トレイトを出荷しています。Suprnovaは、2つのトレイトと1つのコンソールコマンドで、それを反映しています。

### プルーナーを宣言する

```rust
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::eloquent::Prunable;

#[suprnova::prunable]
#[async_trait]
impl Prunable for ExpiredSession {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "expires_at",
            "<",
            (Utc::now() - Duration::days(30)).to_rfc3339(),
        )
    }
}
```

### `MassPrunable` - 一括削除の変種

大量のテーブル（監査ログ、リクエストログ、期限切れのキャッシュエントリ）については、`MassPrunable` は行ごとのイベントをスキップし、単一の `DELETE WHERE …` 文を実行します:

```rust
use suprnova::eloquent::MassPrunable;

#[suprnova::prunable]
#[async_trait]
impl MassPrunable for AuditLog {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "created_at",
            "<",
            (Utc::now() - Duration::days(365)).to_rfc3339(),
        )
    }
}
```

### プルーニングを起動する

プロジェクトごとのコンソール（`app/cmd/main.rs` が、`db:seed` や他の組み込みコマンドの後に、そのために `suprnova::console::dispatch_argv` を呼びます）を介して実行します:

```bash
suprnova model:prune                          # 登録されているすべての型を刈り取る
suprnova model:prune --model=ExpiredSession   # 1つのモデルへ絞り込む
suprnova model:prune --pretend                # ドライラン。削除されるはずのものをログに記録する
```

プログラムからは、ランナーは `suprnova::eloquent::{prune_all, prune_all_dry,prune_one}` にあります。

### プルーニングフック

`Prunable::pruning(&self)` は、ユーザーが副作用を実行できるように（関連するファイルを片付ける、イベントをファンアウトするなど）、各行の削除の前に発火します。デフォルトの実装は空です。`MassPrunable` は、定義上このフックをスキップします - 一括削除は行を列挙しません。

### カスケードの振る舞い

**プルーニングは、関連する行へ自動的にカスケードしません。** `User` の上の `Prunable` あるいは `MassPrunable` の実装は、userの行を削除します。その `posts`、`role_user` のピボットのエントリ、多態的な `comments` などは、今は削除されたuserを指すFKカラムを持ったまま、孤立した状態に**残されます**。

これはLaravelの契約と一致します: リレーションの片付けは、ユーザーの仕事です。それを処理する、2つの綻びのない方法があります:

1. **データベースレベルのFKカスケード** - マイグレーションを書くときに、外部キー制約の中で `ON DELETE CASCADE`（あるいは `ON DELETE SET NULL`）を宣言します。DBエンジンは、行ごとのRustコードなしに、カスケードを無償で処理します。

2. **行ごとのフック** - 親の行が落とされる前に子を削除するために、`Prunable::pruning(&self)` を実装します。このフックは、親の削除と同じ論理的な操作の内側で発火するため、一貫した順序が保証されます:

   ```rust
   #[async_trait]
   impl Prunable for User {
       fn prunable() -> Builder<Self> {
           Self::query().filter_op("deleted_at", "<", thirty_days_ago())
       }

       async fn pruning(&self) -> Result<(), FrameworkError> {
           // postを削除する。
           Post::query().filter("user_id", self.id).get().await?
               .into_iter()
               .map(|p| p.delete());
           // roleのピボットをdetachする。
           self.roles().sync(Vec::<i64>::new()).await?;
           Ok(())
       }
   }
   ```

`MassPrunable` は集合ベースです - `pruning()` は発火しません。カスケードが必要な場合は常に、素の `Prunable` を使ってください。`MassPrunable` を選んだとき、フレームワークがサイレントに行ごとのDELETEを発行することはありません - そのトレードオフは、はっきりと文書化されています。

### レジストリの仕組み

プルーナーの登録は、オブザーバー、コマンド、そしてスーパーバイザーと同じインベントリのパターンを使います。`impl Prunable for T { ... }` ブロックの上の `#[suprnova::prunable]` アトリビュートは、コンパイル時に `inventory::submit!` を介して自動的に登録します。中央の設定ファイルはありません。新しいプルーナブルな型を追加するのは、1つのアトリビュートです。

## マルチ接続ルーティング

本番のアプリは、日常的に、2つ以上のデータベース接続を必要とします - 典型的なケースは、分析のための読み取りレプリカ + 書き込みのためのプライマリですが、この表面は、任意の名前付き接続（レポート用DB、アーカイブ用DB、テナントごとのシャード）へ一般化されます。

### 接続を登録する

あなたのアプリが話しかける、デフォルト以外のすべての接続について、起動時に `DB::register_named(name, config)` を呼んでください:

```rust
DB::register_named(
    "reporting",
    DatabaseConfig {
        url: env::var("REPORTING_DATABASE_URL")?,
        max_connections: Some(20),
        ..Default::default()
    },
).await?;
```

2つの名前が予約されています: `__primary__` は、レジストリを `DB::connection()` へ短絡させます。`__read_replica__` は、その接続を、自動的な読み書き分割ルーティングへオプトインさせます - 下記を参照してください。

### クエリごとのオプトイン: `Model::on(name)`

`Model::on("reporting")` は、名前付き接続を経由してルーティングするように事前設定された `Builder<M>` を返します:

```rust
let totals = Order::on("reporting")
    .order_by_desc("total")
    .limit(100)
    .get()
    .await?;
```

`on(...)` はリクエストにスコープされます - 連鎖したビルダーにだけ影響します。次の素の `Order::query()` の呼び出しは、デフォルトを介して解決されます。

### モデルごとのデフォルト: `#[model(connection = "...")]`

モデルが常に1つの接続の上に存在する場合は、そのデフォルトをアトリビュートの上で宣言してください:

```rust
#[model(table = "events", connection = "events_db")]
pub struct Event { /* ... */ }
```

すべての `Event::query()` / `Event::create()` / `Event::find()` の呼び出しは、クエリごとの `.on(...)` の上書きを必要とせずに、`events_db` を経由してルーティングされます。ビルダー上の明示的な `.on(...)` は、それでも勝ちます。

### 読み書きの分割

予約された名前 `__read_replica__` の下で接続を登録すると、すべてのモデルが自動的なルーティングへオプトインします: 読み取りメソッド（`first` / `get` /`find` / `count` / `paginate` / `chunk` / クロージャで駆動されるウォーカー）はレプリカを経由して流れ、書き込み（`save` / `create` / `update` /`delete` / `force_delete` / `replicate` / `attach` / `detach` / `sync` /`increment` / `decrement`）はプライマリを経由して流れます。

`Model::on_write_connection()` は、単一のビルダーをレプリカからオプトアウトさせます - read-your-writesの一貫性が重要な場合に便利です（例えば、`save` の直後、レプリケーションが追いつく前など）。

### ルーティングの優先順位

ディスパッチの連鎖は、すべての操作を `ExecutorChoice::resolve_read` あるいは `resolve_write` を経由させます。順序は次のとおりです:

1. **アクティブなトランザクションが、絶対的に勝ちます。** `DB::transaction` の内側では、すべての読み取り**と**すべての書き込みが、txのコネクションを使います。`on(name)` は、トランザクションの内側では**無視されます** - txは、特定の物理的なコネクションに束縛されているからです。SeaORMは、1つのコネクションでトランザクションを開始し、別のコネクションに対して文を実行することはできません。
2. **ビルダーごとの `on(name)`。** `Model::on(name)` / `Builder::on(name)` を介して設定されます。モデルのデフォルトと、読み書きの分割に勝ちます。
3. **`Model::on_write_connection()`。** 操作がそうでなければレプリカへルーティングされるはずの場合でも、プライマリを強制します。
4. **モデルごとの `#[model(connection = "...")]` のデフォルト。** そのモデル自身のクエリについて、読み書きの分割に勝ちます。
5. **読み書きの分割。** `__read_replica__` が登録されているとき、読み取りメソッドはそこへルーティングされます。書き込みはプライマリへルーティングされます。
6. **デフォルト。** `DB::connection()` - プライマリであり、`DB::init()` がセットアップしたものです。

### 注意点

- アクティブなトランザクションは `on(name)` を**無視します**（上記の§1を参照）。txの途中で、別の接続への書き込みが必要な場合、それはできません - txは1つの接続に束縛されています。
- 予約された名前 `__primary__` と `__read_replica__` は、ユーザーの接続名として使えません。`DB::register_named` は、衝突時にエラーを返します。
- レプリカの遅延は、**あなたの**問題です。Suprnovaは、レプリカが古いときに、読み取りをリトライしたり、プライマリへフォールバックしたりはしません。saveの後にread-your-writesが必要な場合は、明示的に `Model::on_write_connection()` を使ってください。

## 複製

`Model::replicate()` は、主キーがデフォルトにリセットされた、モデルの未保存のコピーを返します。ユーザーが既存の行から始めたい「このレコードを複製する」というUXに便利です。

```rust
let template: User = User::find_or_fail(42).await?;
let mut copy = template.replicate().await?;  // idはデフォルトにリセットされる
copy.email = "fresh@example.com".into();
copy.save().await?;  // UPDATEではなくINSERT
```

`replicate` はSuprnovaでは**async**です（Laravelから分岐しています）。`Replicating` イベントを発火させるからです - `Saving` / `Created` などのリスナーは、それが返される前に複製を変更できます。リスナーによる変更の契約については、[Replicating イベント](#replicating-イベント)を参照してください。

### `replicate_except`

複製から、名前を指定したフィールドを落とします:

```rust
let copy = order.replicate_except(["payment_token", "stripe_id"]).await?;
```

列挙されたフィールドは、モデルの `Default` の実装にフォールバックします - `String` は `""` になり、`Option` は `None` になります。複製された行が引き継ぐべきではない、機密のカラムのために、これを使ってください。

### 型をまたぐ `replicate_into::<T>`

Suprnovaの相違点です - PHPには型がないため、Laravelはこれをできません。`replicate_into::<T>()` は、`serde_json` を介して、兄弟の型へ橋渡しします:

```rust
let order: Order = Order::find_or_fail(42).await?;
let invoice: Invoice = order.replicate_into::<Invoice>().await?;
invoice.save().await?;
```

名前が一致し、serdeと互換性のある型を持つフィールドは引き継がれます。どちらかの側で一致しないフィールドは、サイレントに落とされます。`T` は `Default` を実装していなければなりません。そうすれば、埋まっていないフィールドが値を持ちます。型をまたぐ複製は `Replicating` を発火**しません**（そのイベントは `&mut Self` を運ぶため、それを通じて `T` に対処する方法がありません）。イベント駆動の変更が必要な場合は、まず同じ型で複製し、それから、その結果から `T` を実体化してください。

## デバッグ - dump と dd

すべての `Builder<M>` の上の、2つの対話的なデバッグ用の補助です:

```rust
// tracing::info!を介して、SQL + バインディングを記録し、selfを返す。
let users = User::query()
    .filter("active", true)
    .dump()                       // → ログ行、ビルダーは継続する
    .order_by_desc("created_at")
    .get()
    .await?;

// tracing::error!で記録し、それからメッセージの中にSQLを入れてパニックする。
User::query().filter("id", 1).dd();  // - !
```

`dump` は連鎖可能です。`dd` は `!` を返します（決して返らない - パニックすることが契約です）。どちらも、Laravelの `Builder::dump()` /`Builder::dd()` を正確に反映しています。

どちらのヘルパーも、生きているDB接続が束縛されていないときは、SQLiteの方言にフォールバックします（`to_sql_with_bindings` のフォールバックと一致します）。そのため、REPLで、あるいは `TestDatabase` なしのテストの中でも、有用であり続けます。

パニックのメッセージは、リテラルなプレフィックス `eloquent dd:` を使うため、テストはそれに対してアサートできます:

```rust
#[test]
#[should_panic(expected = "eloquent dd")]
fn dd_panics_with_sql_in_message() {
    User::query().filter("id", 1).dd();
}
```

**`dd()` を本番のコードパスにコミットしては絶対にいけません。** それは対話的なデバッグ用の補助であり、出ていく途中でパニックすることこそが、まさにその要点です。`dump()` はより安全です（ただログを記録するだけです）が、ホットパスでそれを乱発すると、ログを埋め尽くしてしまいます - プッシュする前に取り除いてください。

副作用なしにSQLが欲しい場合は、ログを記録しないヘルパーに手を伸ばしてください:

- `Builder::to_sql()` - レンダリングされたSQLを `String` として返します。
- `Builder::to_sql_with_bindings()` - `(String, Vec<SeaValue>)` を返します。
- `Builder::to_sql_for(backend)` - 明示的な方言のためにレンダリングします（バックエンドをまたぐデバッグです）。

## モデルのテスト

テストは、`TestDatabase` を介して、本物のデータベースをインスタンス化します。これは、テストごとのコンテナの中にコネクションを登録するため、SUTの内側で `DB::connection()` を呼ぶものは何であれ、テスト用のDBへ解決されます。

### 2つのエントリーポイント

- **`TestDatabase::fresh::<MyMigrator>().await`** - 本番のマイグレータが実行するすべてのマイグレーションを実行します。テストのスキーマが、`suprnova migrate` が生成するものと正確に一致することを望む、アプリレベルのdogfoodテストのために、これを使ってください。
- **`TestDatabase::sqlite_memory().await`** - マイグレーションを一切適用せずに、インメモリのSQLiteデータベースを開きます。テストごとの `db.execute_unprepared("CREATE TABLE …")` を介した、正確なカラムの形の制御を望む、フレームワークレベルのユニットテストのために、これを使ってください。

### アプリレベルのdogfoodパターン

```rust
use app::migrations::Migrator;
use app::models::users::User;
use suprnova::testing::TestDatabase;
use suprnova::{attrs, Model};

#[tokio::test]
async fn user_lifecycle() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
        password: "hashed",
    }).await.unwrap();

    assert!(alice.id > 0);

    alice.delete().await.unwrap();
    assert!(User::find(alice.id).await.unwrap().is_none(),
        "default scope hides soft-deleted rows");
}
```

`_db` の束縛は、テスト全体のために `TestDatabase` を保持します - それをドロップすると、コンテナが取り壊され、インメモリのSQLite接続が解放されます。それを `_` へシャドーイングしないでください。そうしないと、SUTが実行される前に、コネクションが消えてしまいます。

### フレームワークレベルの形のパターン

```rust
use suprnova::testing::TestDatabase;
use suprnova::{attrs, model, Model};

#[model(table = "t_users", timestamps = false)]
pub struct TUser { pub id: i64, pub name: String }

#[tokio::test]
async fn shape_test() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT)"
    ).await.unwrap();

    let u = TUser::create(attrs! { name: "Alice" }).await.unwrap();
    assert_eq!(u.name, "Alice");
}
```

### 主要なパターン

- 本番のスキーマを使うアプリレベルのテストには `TestDatabase::fresh::<MyMigrator>()` を。ユニットレベルの形のテストには `TestDatabase::sqlite_memory()` を。
- テストが変更するあらゆるシングルトンには、（`App::bind` では**なく**）`TestContainer::bind` を使ってください - グローバルなレジストリの上書きは、並行する実行の中で競合します。`TestDatabase` のコンストラクタが、DBのバインディングをあなたに代わって処理します。
- モデルの宣言は、テストの関数の内側ではなく、モジュールスコープに保ってください。マクロは、その `use super::*;` がファイルのトップレベルのimportしか見ない、内部の `mod` を発行します - テスト関数の内側でモデルを宣言すると、SeaORMの型解決が壊れます。

## SeaORMへ降りる

3つのエスケープハッチが、Eloquent層の内側からSeaORMへ到達可能な状態を保ちます:

1. **内部モジュール** - `user::Entity`、`user::Column`、`user::ActiveModel`、`user::Model` です。マクロは、これらをすべてのモデルについて発行します。それらは、直接使えるSeaORMの型です。完全なレイアウトと、いつ手を伸ばすべきかについては、[モデルモジュールのレイアウト](#モデルモジュールのレイアウト)を参照してください。
2. **`From` による変換** - `From<user::Model> for User` と `From<User> for user::Model` が、SeaORM形の行（ストレージの型を持つカラム）と、Eloquent形の行（ランタイムの型を持つカラム）の間を橋渡しします。SeaORMのクエリを発行し、その結果をEloquentの形へ変換したい場合（あるいはその逆）に便利です。
3. **Suprnovaがエイリアス化したSeaORMの型** - 消費者が触れるであろうすべてのSeaORMの型は、`suprnova::*` の下に再エクスポートされています。アプリのコードの中で `use sea_orm::*` は必要ないはずです。

```rust
use suprnova::sea_orm::{ColumnTrait, EntityTrait};

// クエリの途中でSeaORMへ降りる - Eloquentにはこのための
// メソッドがないが、SeaORMにはある:
let db = suprnova::DB::connection()?;
let users = user::Entity::find()
    .filter(user::Column::Email.like("%@example.com"))
    .all(db.inner())
    .await?;

// Eloquent形へ変換する:
let eloquent: Vec<User> = users.into_iter().map(User::from).collect();
```

3つのエスケープハッチとFromによる橋渡しは、Eloquent層が、背後にあるORMへの到達を、決してブロックしないことを意味します。

## `database::Model` からの移行

古いコードは、手作りのSeaORMのentityの上に、`impl suprnova::database::Model for Entity {}` を運んでいるかもしれません。このトレイトは、新しい `Model` トレイトのために場所を空けるために、`EntityExt` へ改名されました - 新しい `Model` は、SeaORMのentityの上ではなく、ユーザーに面した構造体の上に存在します。

推奨される移行の経路は、その型を `#[suprnova::model]` へ切り替えることです。これは、完全なEloquentの表面に加えて、改名された `EntityExt` のトレイトも、おまけとして与えてくれます。古いSeaORM-Entity拡張の形を保ちたい、まれなケースについては、`EntityExt` / `EntityExtMut` というトレイト名が、それでも `suprnova::database::*` の下で利用できます。それらは、古い `database::Model` がしていたのと、まったく同じように振る舞います。

## DBファサード - モデルレスなクエリ

いくつかのテーブルは、`#[suprnova::model]` の構造体には属しません: 短命な監査ログ、アドホックなレポート用のjoin、ダッシュボードの集計です。それらのためには、`DB` ファサードに手を伸ばしてください。その下には、2つの表面があります:

### `DB::table(name)` - 連鎖可能なクエリビルダー

`DbTableBuilder` は、`Builder<M>` のwhere / order / limitの形を反映していますが、行を `DynamicRow`（`serde_json::Map<String, Value>` の上の型付きアクセサのニュータイプ）として返します:

```rust
use suprnova::DB;

let rows = DB::table("audit_log")
    .filter("actor_id", 42)
    .filter_op("created_at", ">=", "2026-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

for row in rows.iter() {
    let event: String = row.get_string("event")?;
    let actor_id: i64 = row.get_int("actor_id")?;
    println!("{actor_id}: {event}");
}
```

完全な表面:

| メソッド | 戻り値 | 目的 |
|--------|---------|---------|
| `.select(["id", "event"])` | `DbTableBuilder` | カラムを絞り込む（デフォルトは `*`） |
| `.filter(col, val)` | `DbTableBuilder` | `WHERE col = ?` |
| `.filter_op(col, op, val)` | `DbTableBuilder` | `WHERE col <op> ?` |
| `.order_by_asc(col) / _desc(col)` | `DbTableBuilder` | 並べ替え |
| `.limit(n) / .offset(n)` | `DbTableBuilder` | ウィンドウ |
| `.get()` | `Collection<DynamicRow>` | マッチするすべての行 |
| `.first()` | `Option<DynamicRow>` | 最初の行、あるいは `None` |
| `.count()` | `u64` | `SELECT COUNT(*) ...` |
| `.insert(attrs)` | `i64` | 新しい行の `id` |
| `.update(attrs)` | `u64` | 影響を受けた行数 |
| `.delete()` | `u64` | 影響を受けた行数 |

**識別子の信頼境界。** テーブル名、カラム名、SQL演算子、そしてORDER BYの方向は、そのままSQL文字列へ補間されます - パラメータとしてバインドされるわけでは**ありません**。これらの引数には、信頼できる、コンパイル時のリテラルだけを渡してください。値（`filter` / `filter_op` の右辺）はバインド**され**、リクエストのデータからそのまま渡しても安全です。

**`update` / `delete` の空のWHEREは、すべての行に対して働きます。**`DB::table("audit_log").delete().await?` は、設計上、テーブルを空にします - そうする意図がないなら、`filter` を追加してください。

**Insertのバックエンドによる分岐。** PostgresとSQLiteでは `RETURNING id` が使われます。MySQLはINSERTを実行してから、自動インクリメントの値を取り戻すために `SELECT LAST_INSERT_ID() as id` を発行します。

### `DynamicRow` - JSONマップの上の型付きアクセサ

`DynamicRow` は `serde_json::Map<String, Value>` をラップし、型付きのゲッターを公開します。各ゲッターは `Result<T, FrameworkError>` を返し、キーの欠落や型の不一致に対しては明確なエラーメッセージを伴います:

```rust
let event: String     = row.get_string("event")?;
let actor_id: i64     = row.get_int("actor_id")?;
let active: bool      = row.get_bool("active")?;
let prefs: Prefs      = row.get_as("prefs")?;  // 任意のDeserializeOwned
let raw: serde_json::Value = row.get_value("meta")?;
```

NULL許容のカラムには、`get_optional_*` を使ってください。これらは、「カラムが欠けている」（エラー - スキーマの不一致）と「カラムは存在し、値はnull」（`Ok(None)`）を区別します:

```rust
let score: Option<i64>      = row.get_optional_int("score")?;
let title: Option<String>   = row.get_optional_string("title")?;
```

`DynamicRow` は `Map<String, Value>` へderefするため、反復やキーの存在確認は自然に動作します:

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}
```

### 生SQLのエスケープ

ビルダーで十分でないとき - ウィンドウ関数、再帰CTE、バックエンド固有のDDL - は、生の文字列まで降りてください。プレースホルダーは、有効なバックエンドに合わせます（Postgresなら `$1, $2, ...`、MySQLとSQLiteなら `?`）:

```rust
// 生のSELECT。DynamicRowとして実体化される。
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) as post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING post_count > ?",
    vec![5i64.into()],
).await?;

// 生のUPDATE / DELETE - 影響を受けた行数を返す。
let updated = DB::update(
    "UPDATE users SET verified_at = NOW() WHERE id = ANY($1)",
    vec![ids.into()],
).await?;

let deleted = DB::delete(
    "DELETE FROM stale_sessions WHERE expires_at < ?",
    vec![now.into()],
).await?;

// 生のDDL、あるいはバインドなしの文。
DB::statement("CREATE INDEX CONCURRENTLY idx_users_email ON users(email)")
    .await?;

// 汎用の影響を受ける文 - INSERT ... ON CONFLICT などのため。
let rows = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1) ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec!["page_views".into()],
).await?;
```

これらのエスケープハッチは、控えめに使ってください - 型付けされたビルダーは、コンパイル時により多くのエラーを捕まえ、ビジネスロジックの中でよりすっきりと読めます。しかし、それらが必要なときは、ここにあります。

**集計カラムの落とし穴。** `SELECT COUNT(*) AS n FROM t` のような型を持たない集計は、ビルダーの `.count()` ヘルパーを通じては機能しますが、SQLite上での生の `DB::select` の行からは、サイレントに脱落することがあります - 基盤となる `JsonValue::from_query_result` はsqlxのカラムごとの型情報をたどりますが、裸の集計はそれを何も運びません。集計を伴う生のselectの経路が必要な場合は、式に型付けされたコンテキストを与えてください: `CAST(...AS BIGINT)` というラッパーを使うか、裏側で `query_one` + `try_get` を使う、型付けされた `DB::table(...).count()` / `.max(...)` ヘルパーでカラムを読み取ってください。

## リレーション存在 + 手軽な近道

Suprnovaは、Laravelのリレーション存在クエリの一族を反映しています。ここにあるすべてのメソッドは、Laravel形の名前と、イディオムに沿ったRustのエイリアスを組にします（Suprnovaの、いつものデュアルAPIの慣例です）。

### リレーション存在のフィルタ（`has` / `where_has` / `where_belongs_to`）

相関 `EXISTS (...)` の一族は、リレーションを外側のSELECTへjoinすることなく、関連する行の存在（あるいは不在、あるいは個数）によって、親のクエリを制約します。

```rust
use suprnova::Model;

// 少なくとも1件のpostを持つuser。
let users = User::query().has("posts").get().await?;

// postを1件も持たないuser。
let empty = User::query().doesnt_have("posts").get().await?;

// 3件以上のpostを持つuser（Laravelの`has("posts", ">=", 3)`）。
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// クロージャを介した内側の制約 - EXISTSサブクエリの本体を絞り込む。
let recent = User::query()
    .where_has::<Post, _>("posts", |q| q.filter_op("created_at", ">=", "2026-01-01"))
    .get()
    .await?;

// 1カラムの近道 - 小さなクロージャを伴う`where_has`と等価。
let with_pub = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// Belongs-toの直接join（EXISTSはない - FKはこのテーブルの上に存在する）。
let posts = Post::query().where_belongs_to("author", author.id).get().await?;
```

すべての変種は、`or_*` と `*_doesnt_have` の相方と合成します:

- `has` / `or_has` / `has_count` / `doesnt_have` / `or_doesnt_have`
- `where_has` / `or_where_has` / `where_doesnt_have` / `or_where_doesnt_have`
- `where_relation` / `where_relation_op` / `or_where_relation`
- `where_belongs_to`

このエンジンは、マクロが生成する `RelationEntry` のインベントリから、リレーションのメタデータを読み取ります: joinのカラム、ピボットテーブル、morphの判別子は、すべて自動的に流れ込みます。3つのサブクエリの形が描画されます:

- **Has** - `EXISTS (SELECT 1 FROM child WHERE child.fk = parent.pk)`
- **Pivot** - `EXISTS (SELECT 1 FROM pivot INNER JOIN target ON ... WHERE pivot.parent_fk = parent.pk)`
- **Morph** - has/pivotの形に `AND target.<morph>_type = '<value>'` を加えたもの

未知のリレーション名は、安全に失敗する形（`EXISTS (SELECT 1 WHERE 1 = 0)`）を描画します。これは `FALSE` に評価され、ゼロ行を返します。タイプミスが、テーブル全体のスキャンを漏らすことは決してありません。

### `MorphTo` の相違点

Laravelの `MorphTo` の逆方向（`whereMorphedTo`、`whereHasMorph`）は、多態的な子が、N個ありうる親のうちの1つを選ぶ `*_type` の判別子を運ぶため、複数のターゲットテーブルを歩きます。Suprnovaの `MorphTo` は、マクロ展開の時点で、ファミリーごとのenumへ落とし込まれます - ターゲットの型は、単一のSQLテーブルではなく、静的に `<Family>Morph { Variant1(...), ... }` です。存在エンジンは、そのケースについて、固定された1つの `EXISTS (SELECT 1 FROM <table>)` を描画できません。単一のテーブルが存在しないからです。

推奨される移行: 代わりに、多態的な子のレベルで存在チェックを行ってください。Laravelが次のように書くところを:

```php
Comment::whereHasMorph('commentable', [Post::class], fn ($q) => $q->where('published', true))
```

Suprnovaは次のように書きます:

```rust
Comment::query()
    .filter("commentable_type", "post")
    .where_has::<Post, _>("commentable_post", |q| q.filter("published", true))
    .get()
    .await?;
```

この、より狭く型付けされた形は、内側のビルダーの上で完全なIDE補完を与えます。緩く型付けされた `whereHasMorph` にはできないことです。

### 手軽なビルダーの近道

```rust
// PKのフィルタ。
User::query().where_key(7).first().await?;        // filter("id", 7)のシュガー
User::query().where_key_not(7).get().await?;      // filter_op("id", "!=", 7)のシュガー
// Rustイディオムに沿ったエイリアス: filter_key / filter_key_not。

// created_atで並べ替える。
Post::query().latest().get().await?;              // ORDER BY created_at DESC
Post::query().oldest().get().await?;              // ORDER BY created_at ASC
Post::query().latest_by("published_at").get().await?;  // 名前を指定したカラム

// 正確に1件のマッチング。
let one = User::query().filter("email", e).sole().await?;          // 0件あるいは2件以上でエラーになる
let val: i64 = User::query().filter("id", 1).sole_value("views").await?;
let v: i64 = User::query().filter("name", "x").value_or_fail("views").await?;

// イーガーロードのオプトアウト。
User::query().with(["posts","tags"]).without(["tags"]).get().await?;
User::query().with_only(["posts"]).get().await?;   // まず計画を消し去る

// 完全修飾されたカラム（joinのため）。
Builder::<User>::qualify_column("name");           // -> "users.name"
Builder::<User>::qualify_columns(["name", "id"]);  // -> ["users.name", "users.id"]
```

### 一括変更 - `update_all` / `delete_all` / `upsert` / `*_each`

これらは、単一の文で、データベースに直接命中し、行ごとのモデルイベントを発火**しません**。スコープの絞り込みで十分で、ライフサイクルフックが必要ない場合に使ってください。行ごとのフックには、`.get()` で反復し、行ごとに `.update()` / `.delete()` を呼んでください。`delete_all` は常に、モデルの静的な `M::TABLE` を対象とします。実行時のテーブル名は、実行可能なSQLとして受け入れられません。

明示的なnull属性はSQLの`NULL`として出力されるため、nullableなbigint、integer、boolean、timestamp、およびその他の非テキストカラムはPostgreSQL上でデータベース型を保持します。nullでない属性はすべて、引き続きパラメータとしてバインドされます。upsertの各行は同じカラム集合を持たなければなりません。欠落したキーや余分なキーは、nullとして解釈されるのではなく拒否されます。

```rust
// 一括UPDATE。
let n = User::query()
    .filter("active", false)
    .update_all(attrs! { archived_at: Utc::now() })
    .await?;

// 一括DELETE。
let n = Session::query()
    .filter_op("expires_at", "<", cutoff)
    .delete_all()
    .await?;

// INSERT ... ON CONFLICT (Postgres / SQLite) / ON DUPLICATE KEY UPDATE (MySQL).
let n = Counter::query()
    .upsert(
        vec![attrs! { key: "page_views", n: 1 }, attrs! { key: "signups", n: 1 }],
        vec!["key"],                  // 競合のターゲット
        Some(vec!["n"]),              // 更新するカラム。None = 一意でないすべてのカラム
    )
    .await?;

// スコープに対するアトミックなincrement/decrement。
User::query()
    .filter("id", 7)
    .increment_each(vec![("views", 1), ("likes", 1)])
    .await?;

User::query()
    .filter("id", 7)
    .decrement_each(vec![("balance", 100)])
    .await?;
```

### 静的な `Model` ヘルパー

```rust
// PKの集合による一括destroy。行ごとのイベントが発火する
// （各行が.delete()を経由するため、ソフトデリートのトゥームストーンの
// セマンティクス + Deleting/Deletedのディスパッチが尊重される）。
let removed: u64 = User::destroy(vec![1i64, 2, 3]).await?;
let removed: u64 = User::force_destroy(vec![1i64, 2, 3]).await?;

// PKによる同一性の比較。
assert!(alice.is(&also_alice));
assert!(alice.is_not(&bob));
```

### `*Quietly` の変種 - ライフサイクルイベントを抑制する

`seed::without_events` の上のシュガーです。5つの静的なライフサイクルイベント（`Saving`/`Creating`/`Updating`/`Deleting`/`Restoring`）と、キャンセル不可能な事後イベントの両方が、このスコープの内側で短絡します。

```rust
user.save_quietly().await?;            // Saving / Updated / Savedはない
user.update_quietly(attrs).await?;
user.delete_quietly().await?;
user.force_delete_quietly().await?;
```

### `*_or_fail` の変種

見つからないケースについての、明示的なエラーです。行が欠けていることがバグである、不変条件をチェックするコードパスで便利です。

```rust
let user = user.update_or_fail(attrs).await?;   // 途中で行が削除された場合はnot_found
user.delete_or_fail().await?;
```

### フィルタされたシリアライゼーション - `to_array_except` / `to_array_only`

Laravelのインスタンスごとの `makeHidden` / `makeVisible` に対する、SuprnovaのRustネイティブな代替です。Eloquentの構造体は実行時の属性バッグを運ばないため、カラムのリストは呼び出しの場所で与えられます:

```rust
return Json::ok(user.to_array_except(&["password_hash", "remember_token"]));
return Json::ok(user.to_array_only(&["id", "name", "email"]));
```

**相違点についての注記。** Laravelのインスタンスごとの `makeHidden` は、モデルが親の `toArray()` の呼び出しの内側にネストされているときに伝播する状態を変更します。Suprnovaのフィルタは終端的です - `serde_json::Value` を生成し、`self` の将来のシリアライゼーションには影響しません。宣言的かつ永続的な可視性の制御には、`#[model(hidden = [...])]` /`#[model(visible = [...])]` のアトリビュートを使ってください。

### UUID / ULIDの主キー - `#[model(unique_id = "...")]`

Laravelの `HasUuids` / `HasUlids` / `HasVersion4Uuids` というトレイトの一族に対応する、Suprnovaのものです。アトリビュートを設定し、PKの型を `String` にすれば、マクロはINSERTの前にIDを自動的に生成します。

```rust
#[model(
    table = "users",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "uuid",      // あるいは "uuid_v4"、"ulid"
)]
pub struct User {
    pub id: String,
    pub email: String,
}

// 自動的に生成される:
let u = User::create(attrs! { email: "a@b.com" }).await?;
// u.idは新しいUUID v7だ。

// 呼び出し元が与えたIDは、それでも勝つ（LaravelのHasUuidsの振る舞いと一致する）。
let u = User::create(attrs! { id: "...", email: "..." }).await?;
```

サポートされている戦略:

- `"uuid"` / `"uuid_v7"` - UUID v7（タイムスタンプ順で、推奨。Laravel 11以降のデフォルトである `Str::uuid7()` と一致します）
- `"uuid_v4"` - ランダムなUUID（`HasVersion4Uuids` と一致します）
- `"ulid"` - 小文字の26文字のCrockford-base32 ULID

マクロは、`UNIQUE_ID_KIND` と、カスタムのジェネレータ（例えば `usr_<uuid>` のようなプレフィックス付きのID）のためにその型の上でオーバーライドできる `new_unique_id()` フックを公開する、`impl HasUniqueId for YourStruct` ブロックを発行します。

### `find_or` / `find_or_new` / `create_or_first`

`FirstOrCreate` トレイトの表面を、完全なものにします。

```rust
// PKでルックアップする。見つからなければフォールバックを実行する。
let user = User::find_or(id, || async {
    User::create(attrs! { id, name: "guest" }).await
}).await?;

// PKでルックアップする。見つからなければ、デフォルトから未保存のインスタンスを構築する。
let user = User::find_or_new(id, attrs! { name: "draft" }).await?;
// ここではuser.id == 0だ - そのインスタンスはメモリ上にだけ存在する。

// 競合に対して安全なinsert: createを試み、競合時にはフェッチへフォールバックする。
let user = User::create_or_first(
    attrs! { email: "race@x.com" },
    attrs! { name: "race winner" },
).await?;
```

### `without_touching` のスコープ

Laravelの `Model::withoutTouching` に対応する、Suprnovaのものです。このスコープの内側では、すべての `model.touch().await` の呼び出しが短絡します - 他の経路を通じてタイムスタンプを変更する、データ移行やバッチジョブを実行するときに便利です。

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    // ここでの.touch()の呼び出しは、何もしない。
    for post in posts {
        post.touch().await?;
    }
}).await;
```

そのスコープは `tokio::task_local` に支えられているため、他のタスク上の並行するリクエストは、それぞれ自分自身のスコープ（あるいはその不在）を尊重し続けます。

## 次のステップ

- [Eloquent リレーションシップ](eloquent-relationships.md) - すべてのリレーションの種類、多態レジストリ、そして多態的なenumへの落とし込みについての深掘りです
- [Eloquent コレクション](eloquent-collections.md) - 完全な `Collection<T>` の表面、ジェネリック対モデルの分岐、そして `LazyCollection<M>` のストリーミングです
- [Eloquent キャスト、アクセッサーとミューテータ](eloquent-mutators.md) - 22個の組み込みキャストと、`casts!` の実行時オーバーライドです
- [Eloquent シリアライゼーション](eloquent-serialization.md) - `to_array`、`to_json`、hidden / visible / appends、そしてフィルタされた終端メソッドです
- [Eloquent ファクトリー](eloquent-factories.md) - テストとシーダーのための、ランダム化されたモデルのインスタンスです

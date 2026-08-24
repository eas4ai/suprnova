# Eloquent リレーションシップ

[Eloquent](eloquent.md)は、日常的なリレーションシップの表面 - 宣言の構文、オプションの表、種類ごとの基本的な連鎖 - をカバーしています。この章は、リレーションシップに特化した深掘りです: `user.posts()` の呼び出しが実際にどのようにSQLへ解決されるか、イーガーローダーがどのようにN+1を避けるか、存在エンジン（`has` / `where_has` / `where_belongs_to`）がどのように相関する `EXISTS` サブクエリを描画するか、多態性がどのようにRustの遅延静的束縛の欠如を乗り越えるか、そして11種類のリレーションすべてが1つのトレイト上で共存しなければならないとき、型システムから何がこぼれ落ちるか、です。

Suprnova上のEloquentが初めてなら、まず[Eloquent](eloquent.md#relationships)を読んでください - あのページが宣言の構文を教えてくれます。このページは、あなたがすでに `relations = { ... }` ブロックを持つモデルを持っていて、その裏側で何が起きているかを理解したいということを前提にしています。

## リレーションの11種類

[`RelationKind`][relations] にあるすべてのリレーションの種類は、次のいずれかです:

| 種類                  | 側       | カーディナリティ | ファミリーをまたぐ | ピボット |
|-----------------------|------------|-------------|-----------------|-------|
| `HasOne<R>`           | 親     | 単         | いいえ              | - |
| `HasMany<R>`          | 親     | 多         | いいえ              | - |
| `BelongsTo<R>`        | 子      | 単         | いいえ              | - |
| `BelongsToMany<R, P>` | どちらでも     | 多         | いいえ              | あり   |
| `HasOneThrough<B, R>` | 親     | 単         | いいえ              | - |
| `HasManyThrough<B, R>`| 親     | 多         | いいえ              | - |
| `MorphOne<R>`         | 親     | 単         | はい              | - |
| `MorphMany<R>`        | 親     | 多         | はい              | - |
| `MorphTo`             | 子      | 単         | はい（nターゲット） | - |
| `MorphToMany<R, P>`   | 親     | 多         | はい              | あり   |
| `MorphedByMany<R, P>` | m2mの相方| 多         | はい（逆）   | あり   |

「ファミリーをまたぐ」とは、関連する行の*型*が変わることを意味します - `Comment` は、1つの固定された親テーブルだけでなく、`Post` にも `Video` にも属しうるということです。それが多態性であり、Suprnovaは、[多態レジストリ](#多態レジストリ)とファミリーごとのenumを通じてそれを扱います。

[relations]: https://docs.rs/suprnova

### マクロが発行するもの

こう書くと:

```rust
use suprnova::model;

#[model(table = "users", relations = {
    posts: HasMany<Post>,
})]
pub struct User {
    pub id: i64,
    pub name: String,
}
```

`#[suprnova::model]` は、`posts` のために5つのものへ展開されます:

1. **リレーションメソッド** - `fn posts(&self) -> HasMany<Self, Post>`。`self.id` とFKのメタデータを運ぶレイジーなラッパーを返します。まだSQLは実行されません。
2. **ロード済みアクセッサー** - `fn posts_loaded(&self) -> &[Post]`。`User::with(["posts"])` の後、イーガーキャッシュから読み取ります。イーガーロードが実行されていない場合は空のスライスです。
3. **カウントアクセッサー** - `fn posts_count(&self) -> u64`。`User::with_count(["posts"])` の後、同じキャッシュから読み取ります。
4. **ディスパッチャーのアーム** - モデルの `__eager_load` inherentメソッドの中のmatchアーム。イーガーローダーは `"posts"` をルックアップし、`IN` クエリを実行します。
5. **インベントリのエントリ** - `inventory::submit!(RelationEntry { ... })` を1つ。これにより、リレーションは実行時に列挙可能になります（管理者向けツール、存在エンジン、多態ディスパッチャーはすべてこれを走査します）。

(4) と (5) を目にすることは決してありません。それらは、この章の残りの部分を動かしています。

## レイジーな解決: `user.posts()` はどのようにSQLになるか

`user.posts()` は、クエリの結果ではなく `HasMany<User, Post>` のラッパーを返します。このラッパーは、親のPKの値とFKのカラム名、そして `WHERE posts.user_id = ?` がすでに適用された、事前にフィルタされた `Builder<Post>` を保持します。まだ何もデータベースに触れていません。

```rust
use suprnova::Direction;

// SQLなし。
let posts_q = user.posts();

// SQL: SELECT * FROM posts WHERE user_id = ? ORDER BY id DESC LIMIT 5
let recent = user.posts()
    .order_by("id", Direction::Desc)
    .limit(5)
    .get()
    .await?;

// SQL: SELECT COUNT(*) FROM posts WHERE user_id = ?
let n = user.posts().count().await?;
```

デュアルAPIの表面（[Eloquent → 命名の注記](eloquent.md#naming-note-dual-api)）は、このラッパー上でも尊重されます: `.filter("col", v)` と `.db_where("col", v)` はどちらも、同一に動作します。`HasOne` / `HasMany` / `MorphOne` / `MorphMany` 上の連鎖可能な表面は、`filter` / `db_where` / `order_by` / `latest` / `oldest` / `limit` / `take` をカバーします。Throughと多態的なm2mのリレーションは、自分の終端メソッドだけを公開します - それらは `Builder<R>` ではなく手書きのSQLの縫い合わせを経由するため、標準の連鎖とは合成できません。下記の[Throughのリレーション](#hasonethrough-と-hasmanythrough)と[多態的なm2m](#morphtomany-と-morphedbymany)を参照してください。

### ソフトデリートは受け継がれる

関連する型が[`SoftDeletes`](eloquent.md#soft-deletes-flag)を実装している場合、リレーションのラッパーはそのグローバルスコープを継承します。`user.posts().get()` は、`Post::query().get()` と同じ方法で、ゴミ箱に入ったpostsを隠します。3つのフォワーダーが、それを突き通します:

```rust
let alive = user.posts().get().await?;                 // デフォルト: 生きているものだけ
let all = user.posts().with_trashed().get().await?;    // 生きている + ゴミ箱
let dead = user.posts().only_trashed().get().await?;   // ゴミ箱だけ
```

`with_trashed` / `only_trashed` は、`HasOne`、`HasMany`、`MorphOne`、`MorphMany`、`BelongsToMany`、`MorphToMany`、`MorphedByMany`、そして `BelongsTo` に存在します。`HasOneThrough` と `HasManyThrough` には意図的に存在しません - 下記の[Throughのソフトデリートの隙間](#throughのソフトデリート-v1)を参照してください。

## 1対1: `HasOne` と `BelongsTo`

`HasOne` は、親が「この子には、私を指すカラムがある」と言っているものです。`BelongsTo` は、子が「私には、親を指すカラムがある」と言っているものです。どちらも、単一の `WHERE fk = ? LIMIT 1` を実行し、`Option<R>` を返します。

```rust
// HasOne - 親 → 子
let profile: Option<Profile> = user.profile().first().await?;

// BelongsTo - 子 → 親
let owner: Option<User> = profile.user().first().await?;
```

`BelongsTo` は、他のものが必要としない、Laravel形の便宜を1つ追加します: `with_default` です。子のFKがnullである、あるいは親の行が削除されている場合、`first()` は `None` の代わりに、クロージャの代役を返します:

```rust
#[model(table = "comments", relations = {
    author: BelongsTo<User> {
        with_default = || User { id: 0, name: "Guest".into(), .. },
    },
})]
pub struct Comment { /* ... */ }

// 常にSome(User)を返す - 本物のauthor、あるいはGuestのスタブのどちらか。
let display: Option<User> = comment.author().first().await?;
```

イーガーロードのディスパッチャーも、同じフォールバックを尊重します - レイジーな経路とイーガーな経路は、デフォルトの振る舞いを共有します。そのため、`comment.author_loaded()[0].name` を出力するテンプレートのコードは、分岐する必要がありません。

## 1対多: `HasMany`

`HasMany` は、親側の、カーディナリティが多であるリレーションです。終端の `.get()` は、[`Collection<R>`](eloquent.md#collections)を返します - `Vec<R>` を包む、Laravel形のラッパーです - そのため、モデルを意識した表面が合成できます:

```rust
let titles = user.posts()
    .order_by("created_at", Direction::Desc)
    .limit(10)
    .get()
    .await?
    .pluck::<String>("title");
```

`latest()` と `oldest()` は、それぞれ `order_by("created_at", Direction::Desc)` と `Asc` のシュガーです - これらは、`created_at` カラムを宣言しているモデルに対してのみ解決されます。このカラムは、タイムスタンプが有効なとき（デフォルトです）、`#[suprnova::model]` マクロが自動的に追加します。

## 多対多: `BelongsToMany<R, P>` とファーストクラスのピボット

`BelongsToMany` は、joinテーブルを介した多対多です。Suprnovaのピボットは、それ自身が、独自のマイグレーション、独自のアクセッサー、独自のイベントを持つ `#[suprnova::model]` 構造体です。それが相違点です - [下記](#suprnovaが異なる設計を選んだ理由-ピボットは本物のモデルである)を参照してください。

```rust
#[model(table = "users", relations = {
    roles: BelongsToMany<Role, RoleUser> {
        with_pivot = ["assigned_at"],
        with_timestamps,
    },
})]
pub struct User { /* ... */ }

#[model(table = "role_user", primary_key = "id")]
pub struct RoleUser {
    pub id: i64,
    pub user_id: i64,
    pub role_id: i64,
    pub assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

ミューテータは、ピボットの行に対して実行されます:

```rust
use suprnova::attrs;

user.roles().attach(role.id).await?;
user.roles().attach_with(role.id, attrs! { assigned_at: now }).await?;
user.roles().detach(role.id).await?;
user.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
```

`sync` は現在のピボット集合を読み取り、`attach_set = ids - current` と `detach_set = current - ids` を計算し、その差分をトランザクションの内側で実行します。入力集合の中の重複は、そのJSON文字列の形によって畳み込まれるため、`sync([1, 1, 2])` はあなたの意図どおりに動作します。

読み取りは、2クエリ戦略を経由します:

```rust
// クエリ1: INNER JOINを介した、user_idでスコープされたSELECT roles.*, role_user.*。
// クエリ2: 行ごとに__pivotを刻印するための、同じjoinに対するSELECT role_user.*。
let roles = user.roles().get().await?;

// 各roleは、マクロがアクセス可能にしたピボットのコンテキストを運ぶ:
for r in &roles {
    let pivot = r.pivot::<RoleUser>().expect("loaded via BelongsToMany");
    println!("{} assigned at {:?}", r.name, pivot.assigned_at);
}
```

### Suprnovaが異なる設計を選んだ理由: ピボットは本物のモデルである

Laravelのピボットは、不透明な属性ごとのバッグです（`$role->pivot->note`）。Suprnovaは、Rustの型システムがコンパイル時にカラムを必要とするため、ピボットの構造体を宣言することを要求します - そして、その宣言の代償を一度払ってしまえば、ピボットは、他のあらゆるテーブルと同じ `#[suprnova::model]` の扱いを受けます: マイグレーション、イベント、オブザーバー、ファクトリー、ソフトデリートです。`r.pivot::<RoleUser>()` は型付けされた参照を返します。文字列キーの属性ルックアップはなく、カラムのスペルを間違えたときの実行時の驚きもありません。

コストは、ピボットテーブルごとに1つの追加の構造体です。利点は、ピボットが、生のSQLへ逃げ出すことなく、振る舞い - ドメインロジック、バリデーションルール、監査用のカラム - を運べることです。

## `HasOneThrough` と `HasManyThrough`

2ホップのリレーションです: `A → B → C` で、`B` は、そのFKが `A` を指す中間モデルであり、`C` は、そのFKが `B` を指す最終的な対象です。典型的な例です: `Country` は多くの `User` を持ち、`User` は多くの `Post` を持ちます。`Country::posts()` は、1回のSQLラウンドトリップで両方のホップを跳び越えます。

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<User, Post>,
})]
pub struct Country { /* ... */ }

// 単一のINNER JOIN: SELECT posts.* FROM posts
//   INNER JOIN users ON posts.user_id = users.id
//   WHERE users.country_id = ?
let posts: Collection<Post> = country.posts().get().await?;
```

`HasOneThrough` は同じ形ですが、`.get()` は（カーディナリティが単であるセマンティクスに一致する）`Option<C>` を返し、`.first()` はそのエイリアスです。

Throughのラッパーは、自分の終端メソッドだけを公開します - `get` / `first` / `count`、そしてキーのセッター（`first_key` / `second_key` / `local_key` / `second_local_key`）です。それらは `Builder<C>` を経由しないため、`.filter(...)` や `.order_by(...)` を連鎖させられません。joinをまたいでフィルタする必要がある場合は、2つの明示的なリレーションのホップへフォールバックしてください。

### Throughのソフトデリート（v1）

Throughのリレーションは、`Builder<C>` のパイプラインではなく、生の `INNER JOIN` SQLを使います。そのため、`C::query()` がインストールするはずのグローバルなソフトデリートのスコープ（`WHERE c.deleted_at IS NULL`）は適用**されません**。ゴミ箱に入った中間モデルと、ゴミ箱に入った対象は、どちらもJOINに参加します。

これはLaravelとは異なります。Laravelでは、モデルが `SoftDeletes` を宣言していれば、`hasManyThrough` は `B` と `C` の両方を `deleted_at IS NULL` でフィルタします。この修正が届くまでは、スコープされたThroughの読み取りを必要とする呼び出し元は、2つのリレーションを明示的に連鎖させるべきです:

```rust
// country.posts().get()の代わりに:
let users = country.users().get().await?;
let user_ids: Vec<i64> = users.iter().map(|u| u.id).collect();
let posts = Post::query().filter_in("user_id", user_ids).get().await?;
// UserとPostの両方のソフトデリートのスコープが適用される。
```

## 多態的なリレーション

多態的なFKは、カラムの組です: `<name>_id`（その行の主キー）と `<name>_type`（そのidが*どのテーブル*に存在するかを識別する文字列）です。1つの `Comment` 行は、`post_id` や `video_id` カラムを追加することなく、`Post` にも `Video` にも向けられます。

Suprnovaは、4つの多態的な種類を出荷しています: `MorphOne`、`MorphMany`、`MorphTo`、そしてm2mの組である `MorphToMany` / `MorphedByMany` です。これらはすべて、1つの基盤 - [多態レジストリ](#多態レジストリ) - を共有します。

### `MorphOne<R>` と `MorphMany<R>` - 親側

`MorphOne` と `MorphMany` は、`HasOne` と `HasMany` を反映していますが、その上に `<name>_type` という判別子を重ねます。内部のビルダーは `WHERE <name>_id = ? AND <name>_type = ?` で事前にフィルタされているため、*他の*ファミリーを指す多態的な子は、結果に決して現れません。

```rust
#[model(table = "posts", morph_type = "post", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Video { /* ... */ }

let post_comments = post.comments().get().await?;     // commentable_type = 'post'のみ
let video_comments = video.comments().get().await?;   // commentable_type = 'video'のみ
```

`morph_type = "post"` は、親が子の `commentable_type` カラムに登録する文字列です。デフォルトはスネークケース化された構造体名ですが、出荷するあらゆるモデルにとって、オーバーライドが正しい手です - テーブルの名前変更というリファクタリングが、多態的なキーを壊すべきではありません。

### `MorphTo` とファミリーごとのenum

`MorphTo` は、多態テーブル側に存在します。ユーザーは、*ターゲットのリスト*を前もって宣言します:

```rust
#[model(table = "comments", relations = {
    commentable: MorphTo { name = "commentable", targets = [Post, Video] },
})]
pub struct Comment {
    pub id: i64,
    pub commentable_id: i64,
    pub commentable_type: String,
    pub body: String,
}
```

マクロは、宣言の場所でファミリーごとのenumを発行します:

```rust
// マクロによって発行される - これはあなたが書くものではない。
pub enum CommentableMorph {
    Post(Post),
    Video(Video),
    Unknown(String, i64),     // 未登録の<name>_typeのためのフォールバック
}
```

そして、`comment.commentable()` は、`.get()` がそのenumへ解決される、取得用のヘルパーを返します:

```rust
match comment.commentable().get().await? {
    CommentableMorph::Post(post) => println!("on post: {}", post.title),
    CommentableMorph::Video(video) => println!("on video: {}", video.url),
    CommentableMorph::Unknown(t, id) => {
        eprintln!("orphaned commentable_type={t} id={id}");
    }
}
```

### Suprnovaが異なる設計を選んだ理由: ファミリーごとのenum

Laravelの `morphTo` は `mixed` を返します - PHPの動的ディスパッチが、実行時にメソッドを解決します。Rustには遅延静的束縛がないため、Suprnovaはファミリーを明示的にします。その利点は、型付けのコストに勝ります:

- **網羅的な`match`** - 新しい多態のターゲットが追加されて、それを処理し忘れたとき、コンパイラが教えてくれます。
- **`Unknown(String, id)` は型安全です** - 削除された親モデルのクラスから来た孤立した行は、パニックするのではなく、バリアントとして表面化します。
- **ターゲットのリストがスキーマを文書化します** - `MorphTo` の宣言を読めば、反対側にありうるすべての型が分かります。それらを列挙するために、データベースへのクエリは必要ありません。

### v1の制限: `MorphTo` は `i64` のみ

`MorphTo::morph_id` は `i64` にハードコードされています。したがって、多態的なターゲットは `i64` の主キーを使わなければならず、多態テーブルの `<name>_id` カラムも `i64` でなければなりません。PKが `String` あるいは文字列経由の `Uuid` であるモデルは、v1では `MorphTo` のターゲットになれません。v2は、多態IDの型をパラメータ化するため、PKの格子全体（`i64` / `String` / `Uuid`）が受け入れられるようになります。

これは、多態の逆方向だけの制限です。`MorphOne` / `MorphMany` / `MorphToMany` / `MorphedByMany` は、どんなPKの形でも問題なく動作します - それらは、親のすでに型付けされた `id` を直接読み取ります。

### `MorphToMany` と `MorphedByMany`

1つのピボットを介した、多態的な多対多です。一方の側は「多態可能」です（`Post.tags()`、`Video.tags()` - どちらも同じ `taggables` ピボットを経由します）。もう一方は、共有されるm2mの相方です（`Tag.posts()`、`Tag.videos()` - 同じピボットを、逆方向に走査します）。

```rust
#[model(table = "tags", relations = {
    posts: MorphedByMany<Post, Taggable> {
        name = "taggable",
        target_morph_type = "post",
    },
    videos: MorphedByMany<Video, Taggable> {
        name = "taggable",
        target_morph_type = "video",
    },
})]
pub struct Tag { /* ... */ }

#[model(table = "posts", morph_type = "post", relations = {
    tags: MorphToMany<Tag, Taggable> { name = "taggable" },
})]
pub struct Post { /* ... */ }

#[model(table = "taggables", primary_key = "id", timestamps = false)]
pub struct Taggable {
    pub id: i64,
    pub tag_id: i64,
    pub taggable_id: i64,
    pub taggable_type: String,
}
```

`MorphToMany` は変更する側です - `attach` / `attach_with` / `detach` / `sync` はすべてそこに存在します。`MorphedByMany` は読み取り専用です: `tag.posts()` の呼び出しはそれぞれ `Post` 型のtaggableだけを返し、`tag.videos()` はそれぞれ `Video` 型のtaggableだけを返します。1つのコレクションの中で混ざることはありません。

多態可能な側から変更する:

```rust
post.tags().attach(rust_tag.id).await?;
post.tags().sync([rust_tag.id, async_tag.id]).await?;
```

どちらからでも読み取る:

```rust
let tags_on_post: Collection<Tag> = post.tags().get().await?;
let posts_with_rust_tag: Collection<Post> = rust_tag.posts().get().await?;
```

## 多態レジストリ

`#[suprnova::model(morph_type = "...")]` という注釈が付いたすべての構造体は、コンパイル時に `inventory::submit!` を介して1つの [`MorphTypeEntry`][morph] を発行します。このレジストリは、3つのことを動かします:

1. **ファミリーごとのenumディスパッチ** - `MorphTo.get()` は、子の行の `<name>_type` 文字列を読み取り、それをルックアップして正しいenumのバリアントを見つけます。
2. **`MorphedByMany` のターゲットフィルタリング** - `target_morph_type = "post"` は、その型文字列が本物であることを確かめるため、レジストリを通じて解決されます。
3. **健全性チェック** - `find_morph_type("post")` は、その文字列で登録されているモデルが1つもない場合に `None` を返し、「意図的に未登録」と「タイプミス」を区別します。

```rust
use suprnova::{morph_types, find_morph_type, find_morph_type_by_id};
use std::any::TypeId;

for entry in morph_types() {
    println!("{} -> {}", entry.morph_type, entry.type_name);
}

if let Some(e) = find_morph_type("post") {
    assert_eq!(e.table, "posts");
}

let by_id = find_morph_type_by_id(TypeId::of::<Post>());
```

[morph]: https://docs.rs/suprnova

`morph_type = "..."` 属性を持たないモデルは、意図的に登録しません - このレジストリはオプトインです。多態的でない `User` モデルは、それに何も貢献しません。それこそが、`find_morph_type("user")` が `None` を返すことを、有用な信号にしているのです。

## リレーションの存在によるクエリ

`has` / `where_has` / `doesnt_have` / `where_relation` / `where_belongs_to` は、Suprnovaのリレーション存在エンジンを形作ります。これらはすべて、**親自身のSELECT**に対する相関 `EXISTS (...)` サブクエリとして描画されます - JOINなし、重複した親の行なし、GROUP BYなしです。

```rust
// 少なくとも1件のpostを持つuser。
let with_posts = User::query().has("posts").get().await?;

// 少なくとも3件のpostを持つuser。
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// 少なくとも1件のPUBLISHEDなpostを持つuser。
let published_authors = User::query()
    .where_has::<Post, _>("posts", |q| q.filter("published", true))
    .get()
    .await?;

// postを1件も持たないuser。
let empty_users = User::query().doesnt_have("posts").get().await?;

// DRAFTのpostを1件も持たないuser（publishedなpostは持っているかもしれない）。
let clean = User::query()
    .where_doesnt_have::<Post, _>("posts", |q| q.filter("published", false))
    .get()
    .await?;

// 近道: where_has + 単一カラム == match。
let same = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// where_belongs_to - この行の上での直接のFK = ?（EXISTSは不要。
// FKが子の行の上にあるからだ）。
let mine = Post::query()
    .where_belongs_to("author", user.id)
    .get()
    .await?;
```

### 仕組み

このエンジンは、クエリ構築時にリレーションのインベントリを走査します。名前を指定された各リレーションについて、`RelationEntry` を取得し、種類ごとに適切なSQLの形を描画します:

- `HasOne` / `HasMany` / `MorphOne` / `MorphMany` → `EXISTS (SELECT 1 FROM child WHERE child.<fk> = parent.<pk>)`。多態の種類は `AND child.<name>_type = '<parent_morph_type>'` を追加します。
- `BelongsTo` → `EXISTS (SELECT 1 FROM parent WHERE parent.<pk> = child.<fk>)`。
- `BelongsToMany` / `MorphToMany` → ピボットを介してjoinします: `EXISTS (SELECT 1 FROM pivot WHERE pivot.<parent_fk> = parent.<pk> ...)`。
- Throughのリレーション → 中間モデルを介してjoinします。

クロージャの形式（`where_has::<R, _>(rel, |q| ...)`）は、内部の `Builder<R>` を構築します。そのビルダーが生成するどんなWHERE項も、サブクエリの本体の中に収まります。プレースホルダーの番号付けは、文全体をまたいで単調であるため、このエンジンは `$1` 形式のPostgresパラメータでも正しく動作します。

`where_belongs_to` は、EXISTSを描画しない唯一の例外です。belongs-toのFKは親*自身*の行の上に存在するため、直接の `WHERE child.<fk> = ?` こそが、まさに正しいSQLです - サブクエリは不要です。リレーション名が親のインベントリにとって未知のものである場合、このエンジンは `WHERE 1 = 0` を発行するため、クエリは安全に何も返しません。

### なぜこれがLEFT JOINに勝るのか

Laravelの古い `has` / `whereHas` エンジンは、JOINを発行し、親の行を重複させていました。相関EXISTSへの書き換えは、Laravel
9. で届きました。Suprnovaは、初日からEXISTSを出荷しています。その利点です: 結果集合の中に重複がなく、集計のためのGROUP BYの回避策も不要で、`DISTINCT` も必要なく、データベースのオプティマイザは、述語を押し込めないJOINの代わりに、本物のサブクエリを目にします。`has_count(rel, ">=", n)` については、このエンジンは `(SELECT COUNT(*) FROM child WHERE ...) >= n` を直接描画します - 1つのクエリ、1つのプランです。

## イーガーロード - `with`、`with_count`、`with_*` の集計

レイジーな `user.posts().get()` は、親ごとに1つのクエリを実行します。多くのuserを持つ場合、これはN+1です:

```rust
// 悪い例: usersに1クエリ + postsに100クエリ。
let users = User::query().limit(100).get().await?;
for u in &users {
    let posts = u.posts().get().await?;
    /* ... */
}
```

`with(["posts"])` は、親の数にかかわらず、それを合計2クエリへ折り畳みます:

```rust
// 良い例: usersに1クエリ + すべてのpostsに1つのINクエリ。
let users = User::query()
    .with(["posts"])
    .limit(100)
    .get()
    .await?;

for u in &users {
    for post in u.posts_loaded() {       // キャッシュから読む、SQLなし
        println!("{}: {}", u.name, post.title);
    }
}
```

ネストしたパスも機能します - ドット区切りのリレーション名は再帰します:

```rust
let users = User::query()
    .with(["posts.comments.author"])
    .get()
    .await?;
// 4クエリ: users、users.idに対するIN検索のposts、posts.idに対するIN検索のcomments、comments.user_idに対するIN検索のauthors。
```

### `with_count` と集計

`with_count` は、親と並んでロードされる、リレーションごとの `COUNT(*) GROUP BY parent_fk` 集計を追加します - リレーションごとに1つの追加クエリです:

```rust
let users = User::query().with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}
```

4つの集計の変種が積み重なります: `with_sum`、`with_avg`、`with_min`、`with_max` です。キャッシュキーの形は `<rel>_<kind>_<col>` であるため、同じリレーションの上に複数の集計を積み重ねても衝突しません:

```rust
let users = User::query()
    .with_count(["posts"])
    .with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .get()
    .await?;

for u in &users {
    println!(
        "{}: {} posts, {} views total, {} avg",
        u.name,
        u.posts_count(),
        u.posts_sum_of("views").unwrap_or(0.0),
        u.posts_avg_of("views").unwrap_or(0.0),
    );
}
```

完全な格納の契約については、[Eloquent → イーガーロード → キャッシュのレイアウト](eloquent.md#cache-layout)を参照してください。

### 制約付きのイーガーロード - `with_where`

`with_where` は、マッチする子を持たない親を失うことなく、どの子の行がイーガーキャッシュに収まるかをフィルタします:

```rust
use suprnova::Builder;

let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// 各u.posts_loaded()には、publishedなpostだけが含まれる。
// publishedなpostを1つも持たないuserも、結果集合には現れる -
// そのposts_loaded()は空のスライスを返す。
```

`with_where` は、意図において `where_has` とは異なります: `where_has` は親の集合をフィルタします（「少なくとも1件のpublishedなpostを持つuser」）。`with_where` はイーガーキャッシュをフィルタします（「すべてのuserについて、そのpublishedなpostだけをロードする」）。両方の効果を望む場合は、両方を一緒に使ってください。

この述語は `FnOnce` ではなく `Fn` であるため、それを運ぶビルダーはクローンして複数回実行できます。キャプチャした値を消費したいクロージャは、内側でそれをクローンするべきです:

```rust
let wanted = vec!["rust".to_string(), "web".to_string()];
let users = User::query()
    // 内側で`wanted.clone()`する。`wanted`自身を`move`するのではない -
    // このクロージャは、ビルダーのクローンごとに一度実行されるかもしれない。
    .with_where(("posts", move |q: Builder<Post>| q.filter_in("tag", wanted.clone())))
    .get()
    .await?;
```

### クエリをクローンすると、イーガーロードの計画も保たれる

`Builder` は `Clone` であり、そのクローンはイーガーロードの計画を一緒に運びます。そのため、「ベースのクエリを構築し、そこから複数を導出する」というパターンが機能します:

```rust
let base = User::query().with(["posts"]).filter("active", true);

let first_page = base.clone().limit(20).get().await?;
let total = base.count().await?;
// first_pageの行は、posts_loaded()が埋まっている。
```

### Suprnovaが異なる設計を選んだ理由

LaravelのPHPの配列は代入時にコピーされるため、`$query->with(...)` は自由にクローンできます。Rustは、型消去されたクロージャに対してクローンが何を意味するかを言わなければなりません。v0.7.2までは、Suprnovaは計画を落とすことでこれに答えていました - クローンは成功し、クエリは成功しましたが、リレーションは単に存在しなくなっていたのです。述語を `Arc` を介して共有すると、上記の `Fn` 境界というコストを払って、クローンは完全なものになります。

`chunk` / `chunk_by_id` / `lazy` の内側でのイーガーロードは、サイレントなチャンクごとのN+1ではなく、はっきりと分かるエラーのままです。それが欲しい場合は、チャンクごとのクロージャの内側で `.with(...)` を再適用してください。

### すでに取得済みのコレクションに対するロード

イーガーロードの計画なしに `Collection<M>` を取得した場合、後からそれを取り付けられます:

```rust
let mut users = User::query().get().await?;

users.load(["posts"]).await?;                 // 無条件
users.load_missing(["posts.comments"]).await?; // すでにロードされているものはスキップする
```

`load_missing` は、各親の `__eager` キャッシュを走査し、まだそのリレーションをロードしていない行に対してだけ、INクエリを発します。リクエストの中で、一部の親が早い段階でイーガーロードされ、他は違ったというループの中で便利です。

### オプトアウトする - `without`

`without` は、名前を指定したリレーションをイーガーロードの計画から取り除きます。ベースのスコープが、この呼び出しでは望まないデフォルトを追加する場合に便利です:

```rust
let users = User::query()
    .with(["profile", "posts", "team"])
    .without(["team"])     // teamを計画から落とす
    .get()
    .await?;
```

## オーナーをtouchする

子は、それを書き込むとオーナーの `updated_at` も新しくすべきであると宣言できます:

```rust
#[model(
    table = "comments",
    touches = ["post"],
    relations = {
        post: BelongsTo<Post> { fk = "post_id" },
    },
)]
pub struct Comment {
    pub id: i64,
    pub post_id: i64,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

touchできるのは `BelongsTo` リレーションだけです。touchされる行は子のカラムから識別できなければならず、それはまさに所有側が提供するものだからです。フレームワークはリレーションレジストリを通じてオーナーを解決するため、touchのコストは1回の `UPDATE` であり、`SELECT` は不要です。

タイムスタンプを無効にしたオーナー（`#[model(timestamps = false)]`）、`NULL` 外部キー経由で到達するオーナー、ソフトデリート済みのオーナーは、無言でスキップされます。一連の作業でカスケードを抑止するには、`without_touching`（すべてのオーナー）または `without_touching_on::<Post, _, _>`（1型）を使用してください。完全なセマンティクスは[Eloquent - 親のtouch](eloquent.md#parent-touching)にあります。

## エスケープハッチ

リレーションが11種類のどれにも当てはまらない場合 - 再帰的な木構造、id以外のキーを介した多態性、三方向のピボット、その他あらゆる特注のもの - は、メソッドを手で書いてください。マクロはそれを妨げません。ただ、そのリレーションについては、ロード済みアクセッサーやイーガーロードのディスパッチャーのアームが手に入らないだけです。

```rust
impl User {
    /// カスタム: FKの形にかかわらず、最新のpost。
    pub async fn latest_post(&self) -> Result<Option<Post>, FrameworkError> {
        Post::query()
            .filter("user_id", self.id)
            .latest()
            .first()
            .await
    }
}
```

そのトレードオフは明示的です: 手書きのメソッドは `relations()` のインベントリには現れず、存在エンジンはそれらについて知らず、イーガーローダーは計画にそれらを含められません。一度限りのものであれば、それで構いません。`with(["..."])` したくなるようなものについては、マクロのオプションを使って形を無理やり整えることになっても、正式なリレーションの種類として宣言してください。

## 次のステップ

- [Eloquent](eloquent.md) - 日常的なモデルの表面です。リレーションの宣言の構文は、そこにあります。
- [データベース](database.md) - 接続、トランザクション、マルチドライバーです。あらゆるものが乗っている下位の層です。
- [マイグレーション](migrations.md) - これらのリレーションが存在を必要とするFKカラムの、スキーマ側です。
- [クエリビルダー](eloquent.md#query-builder-dual-api) - リレーションのラッパーが転送する先の、デュアルAPIの表面です。
- [Eloquent リソース](eloquent-resources.md) - ロードされたリレーションを、レスポンス用のJSON:APIペイロードへ変えることです。

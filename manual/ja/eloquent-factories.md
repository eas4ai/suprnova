# Eloquent ファクトリー

ファクトリーは、テストとシーダーのために、ランダム化されたモデルインスタンスを生成します。その形はLaravelのものです: `UserFactory::new().count(10).create_many().await?`。契約は、1つのトレイトとフルーエントなビルダーであり、モデルがすでに妥当なランダム化された表現を持つ、よくあるケースのための `#[derive(Factory)]` という近道が用意されています。

この章は、ファクトリーを手で、そしてderiveで定義すること、オーバーライドを再利用可能な「ステート」へ組み合わせること、`Sequence` による決定的なID、`create` を支える `Persistable` の継ぎ目、そして `make`（メモリ上）と `create`（永続化される）の違いをカバーします。ファクトリーが最も役立つ、テストを書くという文脈については、[テスト](testing.md)を参照してください。

## `Factory` トレイト

トレイトは、必須のメソッドをちょうど1つ持ちます:

```rust
pub trait Factory {
    type Model;

    fn definition() -> Self::Model
    where
        Self: Sized;
}
```

`definition()` は、意味の通るデフォルトへすべてのフィールドがランダム化された、完全に埋められたモデルを返します。トレイトはインスタンスごとの状態を運びません - 実装者は、典型的にはサイズ0のマーカーです（`struct UserFactory;`）。そのため、呼び出し元は、ハンドルを保持することなく、名前でファクトリーに到達できます。

トレイトは、デフォルト実装を伴う2つのビルダーのエントリポイントも提供します:

```rust
fn new() -> FactoryBuilder<Self::Model>;       // count = 1、オーバーライドなし
fn times(n: usize) -> FactoryBuilder<Self::Model>;  // new().count(n)のシュガー
```

呼び出すことになる他のすべてのメソッド（`with`、`count`、`make`、`create`、`create_many`、…）は、`FactoryBuilder<M>` 上にあります。

## ファクトリーを手で定義する

最小限の手書きの形は、マーカー構造体と、1つのインスタンスの構築方法を知っている `Factory` のimplを組にします。典型的には、モデルが `fake::Dummy` を導出しない場合にこれに手を伸ばします - あるフィールドが決定的なシード（既知の範囲内のリレーションID）を必要とする、あるいはランダム化された表現がビジネスルールを意識する必要がある、といった理由からです:

```rust
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        let now = chrono::Utc::now();
        User {
            // `0`はプレースホルダーだ - `persist_via_seaorm`は、挿入の前に
            // 主キーのカラムを`NotSet`へ切り替えるため、
            // データベースが本物のidを割り当てる。
            id: 0,
            name: format!("Factory User #{}", next_seq()),
            email: format!("factory-{}@example.test", next_seq()),
            password: "factory-placeholder".into(),
            remember_token: None,
            active: true,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

`__eager` と `__pivot` のフィールドは、`#[suprnova::model]` マクロがすべてのEloquent構造体に注入する、イーガーロードとピボットのスクラッチ状態です。常にそれらをデフォルトにしてください - それらは、ファクトリーではなくクエリビルダーによって埋められます。

`next_seq()` は、あなたが望むものであれば何でも構いません - `static AtomicU64`、（下記で扱う）`Sequence`、あるいはスレッドローカルなカウンタです。要点は、`definition()` が `make_many` / `create_many` の内側の呼び出しごとに新しく実行されるため、あなたが必要とする一意性は、その関数が到達できるカウンタから来なければならないということです。

## よくあるケースのための `#[derive(Factory)]`

モデル自身が `fake::Dummy` を実装している場合 - `#[derive(Dummy)]` を介して、あるいは手書きの `impl Dummy<Faker> for Model` を介して - このderiveは、マーカー + implを、モデル上の1行へ折り畳みます:

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub author_id: i64,
    pub is_public: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

このderiveは、兄弟の型として `pub struct PostFactory;` を発行し、`definition()` が `Faker.fake::<Post>()` を呼ぶ `impl Factory for PostFactory` を発行します。ファクトリー上の可視性は、モデル上の可視性を反映します - `pub` なモデルは `pub` なファクトリーを得て、`pub(crate)` なモデルは `pub(crate)` なファクトリーを得ます。

### 生成される名前をオーバーライドする

デフォルトでは、`#[derive(Factory)]` は `<Model>Factory` を発行します。`name` 属性を介してオーバーライドしてください:

```rust
#[derive(Dummy, Factory)]
#[factory(name = "AccountFactory")]
pub struct User { /* … */ }
```

この値は、Rustの識別子としてパースできなければなりません - `name = "User Factory"` や `name = "user-factory"` は、該当箇所を指し示す明確なエラーでコンパイルに失敗します。マクロは `pub struct <Name>;` を文字どおりに発行するため、型名になれないものは、ファクトリー名にもなれません。

### より豊かなランダム化のための、手書きの `Dummy`

`#[derive(Dummy)]` は、プリミティブ型の構造体では機能しますが、分布やフィールド間の不変条件に対する制御を何も与えません。自明でないものについては、`Dummy` のimplを手で書き、`#[derive(Factory)]` と組にしてください:

```rust
use suprnova::__fake::rand::Rng;
use suprnova::__fake::{Dummy, Fake, Faker, faker::lorem::en::{Paragraph, Sentence}};
use suprnova::Factory;

#[derive(Factory)]
pub struct Post { /* fields … */ }

impl Dummy<Faker> for Post {
    fn dummy_with_rng<R: Rng + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        let title: String = Sentence(3..7).fake_with_rng(rng);
        let body: String = Paragraph(3..6).fake_with_rng(rng);
        let author_id: i64 = (1..=50i64).fake_with_rng(rng);
        let now = chrono::Utc::now();

        Post {
            id: 0,
            author_id,
            title,
            body,
            is_public: Faker.fake_with_rng::<bool, _>(rng),
            created_at: now,
            updated_at: now,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

`fake` クレートは `suprnova::__fake` として再エクスポートされているため、利用者は `Cargo.toml` に別個の `fake = "…"` という行を必要としません。よく使う型も、クレートのルートの下に再エクスポートされています: `suprnova::{Dummy, Fake, Faker}`。

### なぜ `#[derive(Factory)]` はプレーンな構造体だけを取るのか

このderiveは、enum、union、そしてジェネリックなモデルを、明確なコンパイルエラーで拒否します。enumとunionには、意味の通るデフォルトの表現がありません。ジェネリクスは、ファクトリーの型がそのモデルをどのようにパラメータ化するかについての決定を強いることになります - そして、良いデフォルトはないため、このderiveは推測を拒みます。そうしたケースについては、`impl Factory` を手で書いてください。

## フルーエントなビルダー

`Factory::new()` / `Factory::times(n)` は、`FactoryBuilder<M>` を返します。すべての操作は連鎖可能です。終端メソッド（`make`、`make_one`、`make_many`、`create`、`create_one`、`create_many`）を呼ぶまで、何も起こりません。

### `count(n)` - インスタンスの数

```rust
let user = UserFactory::new().make();             // 1人のuser
let users = UserFactory::new().count(10).make_many();  // 10人のuser
let same = UserFactory::times(10).make_many();   // 同一
```

`count(n)` は、`make` / `create` では無視され（常に1つです）、`make_many` / `create_many` では尊重されます。`times(n)` は、`Self::new().count(n)` の単なるシュガーであり、Laravelの `Factory::times($n)` と一致します。

### `with(|m| { … })` - 呼び出しごとのオーバーライド

`with` は、`definition()` の後、生成されるすべてのインスタンスに対して実行されるクロージャを登録します。複数の `with` 呼び出しは登録順に合成されるため、同じフィールドに対しては、後のオーバーライドが前のものを踏みつぶします:

```rust
let admin = UserFactory::new()
    .with(|u| u.active = true)
    .with(|u| u.role = "admin".into())
    .make();
```

オーバーライドは `Box<dyn Fn(&mut M) + Send + Sync + 'static>` として格納されるため、ビルダーは `Send` のままです - これは、SeaORMの挿入に対する `.await` をまたいでビルダーを保持する、asyncの `create` / `create_many` の経路にとって重要です。

### `prepend(|m| { … })` - 呼び出し元がなおオーバーライドできるデフォルト

`prepend` は、オーバーライドの連鎖の**先頭**にクロージャを挿入します。そのため、他のどの `with(...)` よりも**前に**実行されます。呼び出し元が後の `.with(...)` でなお踏みつぶせるデフォルトを提供したい場合は、ステートメソッドの内側でこれを使ってください:

```rust
impl UserFactory {
    /// ステートメソッド - adminのデフォルト、呼び出し元はまだカスタマイズできる。
    pub fn admin() -> suprnova::FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.active = true)
    }
}

// 呼び出し元の.with()はprependの後に来るため、roleでは呼び出し元が勝つ。
let owner = UserFactory::admin()
    .with(|u| u.role = "owner".into())
    .make();
```

これは、Laravelの `Factory::prependState` に対応するSuprnovaのものです。これは、特にステートメソッドにとって正しい基本要素です - `with` では、呼び出し元の `.with(...)` に負けてしまい、それはデフォルトがすべきことの正反対です。

### `when(cond, |b| { … })` - 条件付きの連鎖

`when` は、フルーエントなスタイルを壊すことなく、連鎖を通じてフラグを縫い通します。クロージャはビルダーを受け取り、ビルダーを返します。`cond` が false のとき、ビルダーは変更されずに通過します:

```rust
UserFactory::times(10)
    .with(|u| u.active = true)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

Laravelの `Conditionable::when($cond, $cb)` を反映しています。`FnOnce(Self) -> Self` というシグネチャは、ビルダーを返す前に `.await` する限り、クロージャの内側で `await` できることを意味します。

### 終端メソッド

| メソッド | 戻り値 | 永続化される？ |
|---|---|---|
| `make()` | 1つの `M` | いいえ |
| `make_one()` | 1つの `M`（count = 1を強制する） | いいえ |
| `make_many()` | `count` 個の項目からなる `Vec<M>` | いいえ |
| `create()` | `Result<M, FrameworkError>` | はい |
| `create_one()` | `Result<M, FrameworkError>`（count = 1を強制する） | はい |
| `create_many()` | `Result<Vec<M>, FrameworkError>` | はい |

`make_one` と `create_one` は、ステートメソッドが内部で `count` を何か別のものに設定していて、呼び出し元がちょうど1つの結果を望んでいるときに便利です:

```rust
pub fn admins_in_org(org_id: i64) -> suprnova::FactoryBuilder<User> {
    UserFactory::times(5)               // フィクスチャに妥当なデフォルト
        .with(move |u| u.org_id = org_id)
        .with(|u| u.role = "admin".into())
}

// テストは1つだけを望む - create_oneはcount(5)を捨てる。
let admin = admins_in_org(42).create_one().await?;
```

## ステート: 再利用可能なプリセットの組み合わせ

Suprnovaは、`state("name")` というルックアップテーブルを出荷しません。代わりに、ステートは、あらかじめ設定された `FactoryBuilder<M>` を返す、あなたのファクトリーマーカー上の普通のメソッドです。このパターンは継承によって合成されます - すべてのステートメソッドは同じ `FactoryBuilder<M>` 型を返すため、結果へさらにメソッドを連鎖させられます:

```rust
use suprnova::FactoryBuilder;
use crate::models::users::User;

pub struct UserFactory;

impl suprnova::Factory for UserFactory {
    type Model = User;
    fn definition() -> User { /* … */ }
}

impl UserFactory {
    /// 非アクティブな変種 - `active: false` というデフォルトを上乗せする。
    pub fn inactive() -> FactoryBuilder<User> {
        Self::new().prepend(|u| u.active = false)
    }

    /// Admin変種 - roleと確認済みのメールを上乗せする。
    pub fn admin() -> FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.email_verified_at = Some(chrono::Utc::now()))
    }

    /// 合成可能: 非アクティブなadmin。
    pub fn inactive_admin() -> FactoryBuilder<User> {
        Self::admin().prepend(|u| u.active = false)
    }
}
```

```rust
// 呼び出し箇所でも合成できる - 自由にさらなるオーバーライドを連鎖させる。
let user = UserFactory::admin()
    .with(|u| u.name = "Alice".into())
    .create()
    .await?;

let batch = UserFactory::inactive().count(20).create_many().await?;
```

`prepend` という選択は意図的なものです: ステートのオーバーライドは、呼び出し元がなお書き換えられる*デフォルト*です。ステートの設定を譲れないものにしたい場合は、代わりに `with` を使ってください - それは連鎖の末尾に行き、勝ちます。

### なぜ `state("name")` によるルックアップがないのか

名前をキーとするステートレジストリは、コンパイラがチェックできることに対して、実行時の文字列マッチングを強いてしまいます。ステートメソッドは、コンパイル時の検証（タイプミスの `UserFactor::admn()` はハードエラーです）と、フルのIDE自動補完を与えてくれます。合成可能性 - `inactive_admin()` の内側から `Self::admin()` を連鎖させること - は、無償で手に入ります。

## `Sequence` による決定的なID

`Sequence` は、呼び出しごとに一意なフィールドをシードするための、単調に増加するカウンタです。各 `next()` の呼び出しは、スレッドをまたいでアトミックに、1、2、3、… を返します:

```rust
use suprnova::{Fake, Sequence};

static ORDER_IDS: Sequence = Sequence::new();

pub struct OrderFactory;
impl suprnova::Factory for OrderFactory {
    type Model = Order;
    fn definition() -> Order {
        Order {
            id: 0,
            number: format!("ORD-{:06}", ORDER_IDS.next()),
            total_cents: (100..=10_000).fake(),
            created_at: chrono::Utc::now(),
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

`Sequence::new()` は `const` であるため、`static` の初期化子として機能します。カウンタは0から始まり、最初の呼び出しで1へ増加します。きれいなカウントが欲しい場合は、テストの間で `reset()` を使ってください - `#[suprnova_test]` マクロはこれを代わりに行いません。フレームワークは、どのSequenceがあなたのものかを知ることができないからです:

```rust
#[suprnova::suprnova_test]
async fn each_order_gets_a_unique_number(db: TestDatabase) {
    ORDER_IDS.reset();   // このテストでは1から始める
    let orders = OrderFactory::new().count(5).create_many().await?;
    assert_eq!(orders[0].number, "ORD-000001");
    assert_eq!(orders[4].number, "ORD-000005");
}
```

`Sequence` は `SeqCst` の順序付けを使います - 「一意なidをくれ」に対してはやり過ぎですが、考えることを自明にしてくれます。Sequenceがホットパスに現れるようなことがあれば、`Relaxed` で自分自身のものを書けます。

## `Persistable`: あなたのストレージへの継ぎ目

`create` 系のメソッドは、モデルが `Persistable` を実装していれば、いつでも利用できます:

```rust
#[async_trait]
pub trait Persistable: Sized + Send {
    async fn persist(self) -> Result<Self, FrameworkError>;
}
```

`factory::persist` 内のブランケット実装は、`IntoActiveModel<ActiveModel>` にできるすべてのSeaORMモデル - つまり `#[suprnova::model]` マクロが発行するすべてのモデル - をカバーします。モデルごとの決まり文句は不要です。`User` がモデルであれば、`UserFactory::new().create()` は動作します。

そのブランケットは `DB::connection()` を取得して挿入します。返される `Self` は、SeaORMが挿入から返すもの - 割り当てられたid、解決されたデフォルトのカラムなど - です。

### 主キーの扱い

SeaORMの `IntoActiveModel` のimplは、PKを含むすべてのフィールドを `Set(value)` としてマークします。ファクトリーが生成したモデルでは、PKはプレースホルダー（`AUTO_INCREMENT i64` では `0`）であるため、そのまま挿入すると、2回目の呼び出しでUNIQUE制約違反に衝突します。

`persist_via_seaorm`（そのブランケットを支えるヘルパー）は、挿入の前にすべての主キーのカラムを `NotSet` へ切り替えます。これにより、データベースが自分自身のidを割り当てられるようになります - ファクトリーが実際に必要とするセマンティクスです:

```rust
pub async fn persist_via_seaorm<M, E, C>(model: M, db: &C) -> Result<M, FrameworkError>
where
    M: ModelTrait<Entity = E> + IntoActiveModel<<E as EntityTrait>::ActiveModel> + Send,
    E: EntityTrait<Model = M>,
    /* … 境界 … */
    C: ConnectionTrait,
{
    let mut active = model.into_active_model();
    for pk in <<E as EntityTrait>::PrimaryKey as Iterable>::iter() {
        active.not_set(pk.into_column());
    }
    active.insert(db).await.map_err(/* … */)
}
```

特定のidを割り当てることを実際に*望む*場合（リプレイテスト、idによるフィクスチャの復元）は、このヘルパーをバイパスして、`model.into_active_model().insert(db).await` を直接呼んでください。

### 明示的な接続に対して永続化する

`persist_via_seaorm` は、接続を引数として取ります。フレームワークの束縛された `DB::connection()` ではない接続に対して永続化を駆動したい場合に便利です - 最も多いのは、統合テストの中の特定の `sqlite::memory:` ハンドルです:

```rust
use suprnova::factory::persist_via_seaorm;

let model = UserFactory::new().make();
let row = persist_via_seaorm(model, db.inner()).await?;
```

### 独自の、SeaORMでないバックエンド

そのブランケット実装は、あらゆる `ModelTrait` 型を対象にしているため、下流のクレートから `impl Persistable for MyOrm::Model` を、衝突せずに書くことはできません。SeaORM以外の独自の永続化（Redis、Surreal、blobのみのストア）については、モデルをニュータイプでラップし、そのラッパーに `Persistable` を実装してください:

```rust
use suprnova::{FrameworkError, Persistable};
use suprnova::async_trait;

pub struct RedisCached<T>(pub T);

#[async_trait]
impl Persistable for RedisCached<MyValue> {
    async fn persist(self) -> Result<Self, FrameworkError> {
        let client = suprnova::App::make::<RedisClient>()
            .ok_or_else(|| FrameworkError::internal("redis client not bound"))?;
        client.set(&self.0.key, &serde_json::to_vec(&self.0)?).await?;
        Ok(self)
    }
}
```

そうすれば、`Factory<Model = RedisCached<MyValue>>` は、`create` / `create_many` を無償で得ます。

## `make` 対 `create`: どちらを使うべきか

`make` は、データベースに触れることなくモデルを返します:

```rust
// 純粋関数のためのユニットテスト - DBは不要。
let draft = PostFactory::new().with(|p| p.is_public = false).make();
let snippet = my_lib::extract_summary(&draft);
assert!(snippet.len() < 200);
```

`create` は永続化し、挿入後のバージョンを返します:

```rust
// 統合テスト - このアクションは本物の行を必要とする。
let post = PostFactory::new().create().await?;
let action = App::resolve::<PublishPostAction>().unwrap();
let published = action.execute(post.id).await?;
assert!(published.is_public);
```

テストが、行が存在することを気にしないときはいつでも、`make` に手を伸ばしてください。行を後で問い合わせるとき、外部キーが本物のidを必要とするとき、あるいはDBを読むサブシステムのためのフィクスチャを投入しているときは、`create` に手を伸ばしてください。`create_many` は逐次的に永続化することに注意してください - 後の挿入が失敗しても、それより前の挿入はロールバック**されません**。`create` / `create_many` は `Persistable` のブランケットを経由し、それはフレームワークの束縛された `DB::connection()` と直接話します - それらは、周囲の `DB::transaction(...)` スコープに参加**しません**。挿入のバッチをまたいで原子性が必要な場合は、クロージャの内側で `Model` トレイトの `Model::create(attrs!{...})` に入ってください（その経路は、`CURRENT_TX` を尊重する同じエグゼキューターを経由します）:

```rust
use suprnova::{DB, Model, attrs};

DB::transaction(|_tx| Box::pin(async move {
    for i in 0..50 {
        User::create(attrs!{
            name: format!("user-{i}"),
            email: format!("user-{i}@example.test"),
        }).await?;
    }
    Ok::<_, suprnova::FrameworkError>(())
})).await?;
```

## 「作成後」の振る舞い

Suprnovaは、`after_creating(|m| { … })` という名前付きのコールバックを出荷していません。Laravelでそのコールバックが存在する理由となる使用例は、2つのパターンでカバーされます:

**1. 連鎖 - `create`/`create_many` の後に、続きの作業を行う:**

```rust
let user = UserFactory::new().create().await?;
ProfileFactory::new()
    .with(move |p| p.user_id = user.id)
    .create()
    .await?;
```

これは、1つのモデルのidが、続く挿入へ流れ込む必要があるときの正規のパターンです。`create` は永続化された行を返すため、idはすぐに利用できます。

**2. モデルオブザーバー - ファクトリーではなく、モデルのライフサイクルに反応する:**

挿入後の振る舞いを、ファクトリーではなくモデル自身へ配線するには、[モデルオブザーバー](eloquent.md#observers)を使ってください。オブザーバーは、`User::create(...)`、`UserFactory::new().create()`、そして他のあらゆる永続化経路に対して発火します - 振る舞いが「この行が届くたびに、Xを行う」というものであるときに、まさに欲しいものです:

```rust
use suprnova::{FrameworkError, Observer, async_trait, observer};

#[observer(User)]
pub struct AuditUser;

#[async_trait]
impl Observer<User> for AuditUser {
    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user_id = user.id, "user created");
        Ok(())
    }
}
```

ファクトリー専用のコールバックは、テストの挿入と本物の挿入の間に相違を招いてしまいます。オブザーバーは、両方をまたいで一貫性を保ちます。

## シーダー

ファクトリーはインスタンスを生成し、シーダーはそれらを組織します。`Seeder` は、何を投入するかを知っている、asyncな `run` を持つ、サイズ0の型です:

```rust
use suprnova::{Factory, FrameworkError, Seeder};
use suprnova::async_trait;

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // まずusers - postsは1..=50の範囲のuser idを参照する。
        UserFactory::new().count(50).create_many().await?;
        PostFactory::new().count(200).create_many().await?;
        Ok(())
    }
}
```

プロジェクトごとの `console` バイナリの `db:seed` コマンドがそれを知るように、シーダーを `bootstrap.rs` に登録してください:

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

プロジェクトの `console` バイナリを通じて実行してください（スキャフォルドされたすべてのアプリは `src/bin/console.rs` にそれを1つ出荷します）:

```bash
cargo run --bin console -- db:seed
```

シーダーは、登録順に実行されます。べき等性は、シーダーの責任です - `run` はスナップショットを取ったりロールバックしたりしないため、無条件に挿入するシーダーは、再実行時に重複を生み出します。きれいな状態にするには、`migrate:fresh` に続けて `db:seed` を使ってください。

## 組み合わせる: 完全なテストフィクスチャ

```rust
use suprnova::{App, describe, test, expect};
use suprnova::events::{EventFacade, assert_dispatched_times};
use suprnova::testing::TestDatabase;
use crate::factories::{PostFactory, UserFactory};
use crate::actions::publish_post::PublishPostAction;

describe!("PublishPostAction", {
    test!("publishes a draft post", async fn(db: TestDatabase) {
        // 準備 - 著者と、その著者が所有する1件の草稿post。
        let author = UserFactory::new()
            .with(|u| u.active = true)
            .create()
            .await
            .unwrap();

        let draft = PostFactory::new()
            .with(move |p| p.author_id = author.id)
            .with(|p| p.is_public = false)
            .create()
            .await
            .unwrap();

        // 実行。
        let action = App::resolve::<PublishPostAction>().unwrap();
        let published = action.execute(draft.id).await.unwrap();

        // 検証。
        expect!(published.is_public).to_equal(true);
        expect!(published.author_id).to_equal(author.id);
    });

    test!("publishing emits exactly one event", async fn(db: TestDatabase) {
        let _guard = EventFacade::fake();
        let post = PostFactory::new().create().await.unwrap();

        App::resolve::<PublishPostAction>().unwrap()
            .execute(post.id).await.unwrap();

        assert_dispatched_times::<crate::events::PostPublished>(1);
    });
});
```

指し示す価値のある、3つのパターンです:

- 著者の `id` は、`.with(...)` の内側の `move` クロージャを介して、postへ流れ込みます。キャプチャは明示的であるため、そのリレーションは呼び出し箇所で目に見えたままです。
- `create().await.unwrap()` はテストのイディオムです - セットアップの失敗でテストがパニックすることは許されています。壊れたフィクスチャは、グレースフルな失敗モードではなく、壊れたテストだからです。
- ファクトリーは、テストの表面の残り（`EventFacade::fake`、`Storage::fake`、`Mail::fake`、…）と合成します - どのフェイクもファクトリーについて知りませんが、あなたが書くすべてのテストは、それらを一緒に使うことになります。

### Suprnovaが異なる設計を選んだ理由

Laravelのファクトリーは、名前付きのステート（`->state('admin')`）、実行時のシーケンス（`->sequence(['name' => 'A'], ['name' => 'B'])`）、そしてファクトリー自身に登録される `afterCreating` コールバックを備えて出荷されます。Suprnovaは、この3つすべてを落とし、Rustらしい基本要素に置き換えます:

- **ステートは文字列ではなく、メソッドです。** コンパイル時のタイプミス検査とIDEの自動補完は、どちらも無償で手に入ります。唯一のコストは、「`protected function admin()` の代わりに `pub fn admin()` を書く」ということですが、これはまったくコストになりません。
- **シーケンスは、別個の基本要素です。** `Sequence` は1つのことだけを行い（アトミックなカウンタ）、ファクトリーの表面の外でも再利用できます - リクエストIDジェネレータ、ワークフローのステップカウンタ、あるいはテストハーネスへ、それが何であるかを説明せずに落とし込めます。
- **作成後の処理は、ファクトリーではなくモデルに配線されます。** フレームワークには、まさにその目的のための[モデルオブザーバー](eloquent.md#observers)がすでにあります。ファクトリー上に並行する仕組みを追加することは、構造上、テスト時の振る舞いと本番時の振る舞いを分岐させてしまいます。

フルーエントな表面 - `count(10)`、`times(10)`、`with`、`prepend`、`when`、`make`、`create`、`create_many`、`make_one`、`create_one` - は、Laravelのものを直接反映しています。そのため、体が覚えたやり方は、用語集なしに移植できます。

## 次のステップ

- [テスト](testing.md) - `#[suprnova_test]`、`TestDatabase`、ファクトリーが構築したフィクスチャと組になるフェイクのファサードです。
- [Eloquent](eloquent.md) - モデルの導出、オブザーバー、そして `create` がファクトリーの出力を永続化するときに実行されるキャストのパイプラインです。
- [マイグレーション](migrations.md) - あなたのファクトリーが対象として存在を必要とするスキーマです。きれいなフィクスチャの状態には `migrate:fresh && db:seed` を使ってください。
- [データベース](database.md) - `DB::transaction`、マルチ接続ルーティング、セーブポイントです。`create_many` が原子性を必要とするときに、手を伸ばすものです。
- [サービス コンテナ](container.md) - `App::resolve` と `App::make` が、ファクトリーと並んであなたのテストが呼び出すアクションとサービスの型をどのように見つけるかです。

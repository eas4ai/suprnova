# バリデーション

Suprnovaは、2つの補完し合う経路でリクエスト入力をバリデーションします。

1. **deriveバリデーション** - `FormRequest` 構造体に付けた `#[validate(...)]` アトリビュートで、`extract()` によって自動的に実行されます。これは日常的に使う経路であり、[リクエスト](requests.md)で扱われています。フィールドごとのルール（`email`、`length`、`range`、…）を宣言的に処理します。
2. **ルールオブジェクトと `validate!` マクロ** - [`Rule`](#ルールオブジェクト) / `ContextualRule` / `AsyncRule` を実装した素の値を、命令的に組み合わせます。フィールドを横断するロジック、データベースに触れるルール、あるいは保存して持ち回りたいルールが必要なときに使ってください。

この2つの経路は、同じ [`ValidationErrors`](error-model.md) バッグへと積み上がり、同じLaravel/Inertia形式の `{ "message", "errors": { field: [...] } }`（HTTP 422）としてレンダリングされます。

## ルールオブジェクト

ルールとは、3つのトレイトのいずれかを実装した値です。

| トレイト | 形 | 用途 |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | 1つの値に対する純粋なチェック |
| `ContextualRule` | `passes(&self, value, ctx)` | 兄弟フィールドを読み取るチェック |
| `AsyncRule` | `async passes(&self, value)` | `.await` するチェック（DB、HTTP） |

組み込みの `Rule`: `Required`、`Email`、`Min`、`Max`、`Between`、`In`、
`NotIn`、`Integer`、`Numeric`、`Boolean`、`Alpha`、`AlphaNum`、`Url`、
`HttpUrl`、`Uuid`。組み込みの `ContextualRule`: `RequiredIf`、
`RequiredWith`、`RequiredUnless`、`Same`、`Different`、`Confirmed`。組み込みの `AsyncRule`: [`Unique`](#unique-ルール)。

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **注意:** `Numeric` は**有限の**数値のみを受け付けます - `NaN`、`inf`、そしてオーバーフローして
> 無限大になる大きさの値は、Rustのパーサーがその文字列を受け入れるにもかかわらず拒否されます。
> コールバック/webhook/アバターの入力には（`Url` ではなく）`HttpUrl` を使ってください。
> `Url` は `url::Url` が受け入れるあらゆるスキーム（`file:`、`javascript:`、独自のURI）をパースしますが、
> `HttpUrl` は `http`/`https` を要求します。

### 自分のルールを書く

カスタムルールは、1つのimplを持つユニット構造体（またはデータを持つ構造体）です。トレイトは `check()` を無償で提供してくれます - これは、失敗メッセージを指定したフィールド名の下で `ValidationErrors` バッグへ積むものです - そのため、ルールは変更なしに `validate!` と `after_validation` フックへ差し込めます。

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(format!("must start with {}", self.0).into())
        }
    }
}

// これで、どこでも使えます:
StartsWith("acct_").passes("acct_1234")?;
// あるいは、validate! の行として:
//   stripe_id => Required, StartsWith("acct_");
```

`String` は、そのままレンダリングされる `ValidationMessage` に変換されます - 単一言語のアプリであれば、これで十分です。メッセージをロケールごとに翻訳したい場合は、代わりに*キー付きの*メッセージを返してください - `ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` のようにし、`lang/<locale>/validation.ftl` にそのIDを定義します。組み込みルールのメッセージを上書きする方法や `field-<name>` という命名規則も含め、詳しくは[ローカライゼーション](localization.md)を参照してください。

フィールドを横断するロジックには、代わりに [`ContextualRule`] を実装してください - `passes` メソッドは、検査対象の値と並んで `&FormContext`（兄弟フィールドの値を持つ `HashMap<String, String>`）を受け取ります。データベースを裏付けとするチェックには [`AsyncRule`] を実装し、`after_validation_async` から使ってください。

## `validate!` マクロ

`validate!` は、構造体のフィールドに対してルールの連鎖を実行し、あらゆる失敗を1つの `ValidationErrors` へ積み上げます。同期的なフィールド横断フックである [`after_validation`](#フィールド横断のフック) の、慣用的な置き場所です。

```rust
use suprnova::{validate, ValidationErrors, rules::{Required, Email, Min, Max, RequiredIf}};

fn after_validation(&self) -> Result<(), ValidationErrors> {
    // コンテキストルールは、あなたが組み立てる `FormContext`（フィールド名をその文字列値へ
    // 対応付けるマップ）から、兄弟の値を読み取ります。
    let mut ctx = std::collections::HashMap::new();
    ctx.insert("billing_type".to_string(), self.billing_type.clone());
    validate! { self =>
        email       => Required, Email;          // 必須形の行
        bio         ?: Min(10), Max(500);        // オプション: Someのときだけバリデーションする
        card_number ?=> RequiredIf {             // 条件付き存在（下記参照）
            other: "billing_type",
            value: "card",
        } => with ctx;
    }
}
```

各行は、次の3つの形のいずれかです。

- **`field => Rule1, Rule2;`** - 必須形です。ルールは `&self.field` に対して直接実行されます（`String`、`i64`、あるいはルールが期待する借用へとderefできるものすべてが対象です）。
- **`field ?: Rule1, Rule2;`** - オプションです。フィールドは `Option<T>` であり、ルールは値が `Some` のときにのみ実行され、`None` のときは**完全にスキップされます**。これはLaravelの「存在すればバリデーションする」（`sometimes`）というセマンティクスです。
- **`field ?=> Rule1, Rule2;`** - 条件付き存在です。こちらも `Option<String>` フィールド向けですが、ルールは `None` のときでも**実行されます**（不在は空文字列として扱われます）。これは、`RequiredIf` のような、*不在のフィールドを失敗させる*ことができなければならない存在条件付きルールのための行です - `?:` では `None` のときにスキップしてしまうため、これを表現できません。

コンテキストルールの後には `=> with $ctx`（兄弟の値を持つ `&HashMap<String, String>`）が続きます。このマクロは**同期的**です - 非同期のルールには、下記の[フック](#リクエストにおける非同期ルール)を使ってください。

> **警告:** よくある罠は、`card_number ?: RequiredIf {...} => with ctx;` と書いてしまうことです。`?:` の行では、`None` のときにすべてのルールがスキップされるため、`RequiredIf` は不在のフィールドを決して失敗させられません。不在のときにも発火しなければならないルールには `?=>` を使ってください。

## フィールド横断のフック

`FormRequest` は、deriveされたフィールドごとのルールの後に、2つのフィールド横断フックを実行します - 通常のフローでもPrecognitionのフローでも同じです。`extract()` は、deriveされた `validate()`、続いて `after_validation`、続いて `after_validation_async` という順序でステージを実行し、**最初に失敗したステージで打ち切ります**。

```rust
use suprnova::{FormRequest, ValidationErrors};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct UpdatePassword {
    #[validate(length(min = 8))]
    pub new_password: String,
    pub confirmation: String,
}

impl FormRequest for UpdatePassword {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        if self.new_password != self.confirmation {
            errs.add("confirmation", "passwords do not match");
        }
        errs.into_result()
    }
}
```

> **注意:** フックをオーバーライドするには、手で書いた `impl FormRequest` が必要です - `#[request]` アトリビュートと `#[derive(FormRequest)]` は、それぞれ独自の（空の）implを生成するため、これらはオーバーライドしない一般的なケース専用です。

### リクエストにおける非同期ルール

`validate!` マクロは `.await` を織り込めないため、データベースを裏付けとするルールは `after_validation_async`（`extract()` が自動的に呼び出す、最後のバリデーションステージ）で実行します。[`Unique`](#unique-ルール) やカスタムの `AsyncRule` が、自動的なリクエストバリデーションに参加するのはここです - ハンドラごとの配線は必要ありません。

```rust
use suprnova::{FormRequest, ValidationErrors, Unique, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct CreateUser {
    #[validate(email)]
    pub email: String,
}

#[async_trait]
impl FormRequest for CreateUser {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Unique::new("users", "email")
            .check_async(&self.email, &mut errs, "email")
            .await;
        errs.into_result()
    }
}
```

非同期ステージは同期ステージが通過した後にのみ実行されるため、不正な値（構文的に無効なメールアドレスなど）がデータベースの `Unique` クエリに到達することはありません。

## `Unique` ルール

`Unique` は、値がテーブル内にまだ存在していないことをチェックします。`Unique::new(table, column)` で組み立て、フルーエントAPIで絞り込んでください。

```rust
use suprnova::Unique;

// メールアドレスは一意でなければならないが、現在編集中の行は無視する
Unique::new("users", "email").ignore(current_user_id)

// メールアドレスは *テナントごとに* 一意で、大文字小文字を区別せずに比較する
Unique::new("users", "email")
    .where_eq("tenant_id", tenant_id)
    .case_insensitive()
```

| ビルダーメソッド | 効果 |
|----------------|--------|
| `.ignore(id)` | `id` が指定した `id` に等しい行を除外します（自分自身を編集するケース） |
| `.ignore_with_column(col, id)` | `id` 以外のキーカラムで除外します |
| `.where_eq(col, value)` | チェックの対象を `col = value` である行に絞ります。複数回呼び出すとAND結合されます |
| `.case_insensitive()` | `LOWER(col) = LOWER(?)` で比較します |

テーブル名、カラム名、除外キー、そしてすべての `where_eq` のカラムは、SQL文字列に達する前に識別子の許可リストと照合されます。検査対象の値とすべてのスコープの値は、バインドパラメータです。

### Unique は助言的なものであり、保証するのはデータベースの制約です

`Unique` は書き込みの**前**に `SELECT COUNT(*)` を実行するため、避けられないtime-of-check/time-of-useのレースを抱えています - 2つの並行リクエストがどちらもチェックを通過し、その後どちらも挿入してしまう可能性があります。Laravelの `unique` ルールも、まったく同じ性質を持っています。**唯一の**本当の保証は、マイグレーションでそのカラムに設定する `UNIQUE` 制約（あるいは一意インデックス）です。

この3つをあわせて使ってください。

1. **助言的なルール** - 送信前に表示する、速くて親切な「そのメールアドレスは使われています」というメッセージです（Precognitionがフィールドをバリデーションできるようにもなります）。
2. **`UNIQUE` 制約** - レースに対する権威ある防御です。
3. **`FrameworkError::from_unique_violation`** - 書き込み箇所で、レースに負けた側が受け取る制約違反を、500として漏らす代わりに、同じきれいな422へマッピングします。

```rust
use suprnova::FrameworkError;

// `users.email` は、マイグレーションで UNIQUE 制約を持っています。
let user = new_user
    .insert(db)
    .await
    .map_err(|e| FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    ))?;
```

`from_unique_violation` は、データベースのエラーが一意制約違反であるとき422の `Validation` エラーを返し、それ以外のエラーはそのまま変更せずに通過させます（MySQL、Postgres、SQLiteのすべてが認識されます）。

## 非同期の認可

`FormRequest::authorize(&Request) -> bool` はボディがパースされる**前**に実行されるため、ペイロードを読むことなく認可されていないリクエストを拒否できます。これは設計上、同期的です - その時点では、リクエストはまだストリーミング中のボディを保持しているため、このフックは `.await` できません。データベースや非同期のポリシーに触れる必要がある認可は、`authorize` ではなく、次のいずれかの場所に置いてください。

- **ミドルウェア** - `extract()` の前に実行され、`async` であり、`Err(response)` を返すことでショートサーキットします（[ミドルウェア](middleware.md)を参照）。「このユーザーはそもそもこのルートに到達できるか」という判断の、正しい置き場所です。
- **Gate** - 認証済みのユーザーとリソースが揃った時点で、ハンドラの中で `Gate::allows_async` / `Gate::authorize_async` を呼び出してください（[認可](authorization.md)を参照）。
- **`after_validation_async`** - パース済みのリクエストボディに依存する認可チェックには、他の非同期ルールと並べて、この非同期フックの中で実行してください。

## 設計上の注意点

- **部分バリデーション。** `FormRequest` は、バリデーションが実行される前に型付きの構造体へとデシリアライズされるため、その構造体自体がスキーマです。不在でありうるフィールドは `Option<T>` でなければなりません。これはまた、Precognitionが部分的なペイロードをバリデーションできる理由でもあります - 下書きが省略できるフィールドは、オプションにしてください。
- **ルールのメッセージ。** 組み込みのルールは、キー付きのメッセージ（`validation-min` に、その引数と英語のフォールバックを添えたもの）を返し、シリアライズ境界でカタログを通じて解決されます。それらを翻訳・言い換えるには、`lang/<locale>/validation.ftl` に同じIDを定義してください - ルールをラップする必要はありません。[ローカライゼーション](localization.md)を参照してください。
- **`Min` / `Max` / `Between`** は、文字列長のルールです（Unicodeスカラー値で数えます）。数値の範囲については、deriveの `#[validate(range(...))]` かカスタムルールでバリデーションしてください - これらの長さルールは、値の比較ではありません。

## まとめ

| タスク | API |
|------|-----|
| フィールドごとのルール | `FormRequest` に付けた `#[validate(...)]`（リクエストを参照） |
| 合成した / フィールド横断のルール | `validate! { self => ... }` |
| オプションの「存在すれば」 | `field ?: Rule;` |
| 条件付きで必須になるオプション | `field ?=> Rule => with ctx;` |
| 非同期 / DBを裏付けとするルール | `after_validation_async` + `AsyncRule::check_async` |
| 一意性 | `Unique::new(t, c)` + `UNIQUE` 制約 + `from_unique_violation` |
| 非同期の認可 | ミドルウェア / `Gate::*_async` / `after_validation_async` |

## 次のステップ

- [リクエスト](requests.md) - `#[request]` / `#[derive(FormRequest)]` の表面、日常的なderiveバリデーションの経路
- [データ オブジェクト](data.md) - 受信リクエストと送信DTOの両方を1つの構造体で表す `#[derive(Data, Validate)]`
- [エラー モデル](error-model.md) - `ValidationErrors` が、他のあらゆるエラー経路と並んで、422のJSONボディになる仕組み
- [ローカライゼーション](localization.md) - ルールのメッセージの翻訳、`field-<name>` という規約、そしてキー付きの `ValidationMessage`
- [認可](authorization.md) - `Gate`、`Policy`、そして認可がバリデーションに対してどこに位置するか
- [ミドルウェア](middleware.md) - `.await` を必要とする「そもそもこのリクエストは通されるべきか」というチェックの、正しい置き場所

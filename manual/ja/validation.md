# バリデーション

Suprnovaは、2つの補完し合う経路でリクエスト入力をバリデーションします。

1. **deriveバリデーション** - `FormRequest` 構造体に付けた `#[validate(...)]` アトリビュートで、`extract()` によって自動的に実行されます。これは日常的に使う経路であり、[リクエスト](requests.md)で扱われています。フィールドごとのルール（`email`、`length`、`range`、…）を宣言的に処理します。
2. **ルールオブジェクトと `validate!` マクロ** - [`Rule`](#ルールオブジェクト) / `ValueRule` / `ContextualRule` / `AsyncRule` を実装した素の値を、命令的に組み合わせます。フィールドを横断するロジック、JSON形状のフィールド、データベースに触れるルール、あるいは保存して持ち回りたいルールが必要なときに使ってください。

この2つの経路は、同じ [`ValidationErrors`](error-model.md) バッグへと積み上がり、同じLaravel/Inertia形式の `{ "message", "errors": { field: [...] } }`（HTTP 422）としてレンダリングされます。

## ルールオブジェクト

ルールとは、4つのトレイトのいずれかを実装した値です:

| トレイト | 形 | 用途 |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | 1つの値に対する純粋なチェック |
| `ValueRule` | `passes(&self, value: &serde_json::Value)` | JSON形状の値（配列/オブジェクト）に対するチェック |
| `ContextualRule` | `passes(&self, value, ctx)` | 兄弟フィールドを読むチェック |
| `AsyncRule` | `async passes(&self, value)` | `.await` するチェック（DB、HTTP） |

組み込みの `Rule`: `Required`、`Email`、`Min`、`Max`、`Between`、`In`、`NotIn`、`InArray`、`Integer`、`Numeric`、`Boolean`、`Alpha`、`AlphaNum`、`AlphaDash`、`Url`、`UrlProtocols`、`HttpUrl`、`Uuid`、[`Password`](#パスワードの強度)（強度のチェックのみ）。組み込みの `ValueRule`: `ArrayKeys`、`Distinct`、`Contains`、`DoesntContain`。組み込みの `ContextualRule`: `RequiredIf`、`RequiredWith`、`RequiredUnless`、`Same`、`Different`、`Confirmed`、`Gt`、`Gte`、`Lt`、`Lte`。組み込みの `AsyncRule`: [`Unique`](#unique-ルール) と [`Password`](#パスワードの強度)（強度に加えて、その `uncompromised()` のHIBPチェック - `Rule` と `AsyncRule` の両方を実装する唯一の組み込みルールです）。

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **注意:** `Numeric` が受け付けるのは**有限の**数です - `NaN`、`inf`、そして無限大へオーバーフローする大きさの値は、Rustのパーサーならその文字列を受け付けるとしても、拒否されます。

### URLのスキーム

`Url` が受け付けるのは、URLとしてパースでき、そのスキームがLaravelの許可リスト - `Illuminate\Support\Str::isUrl` が使うのと同じ一覧です - に載っていて、`://` が続き、**さらに**その後に空でないホストが続く値です。これはLaravelの `^(PROTOCOLS)://HOST` というパターンと形が一致します（Laravelのホストのグループには `?` がありません - ホストが欠けている、あるいは空の場合は決して一致しません）。スキームの一覧と、`://` に加えてホストを要求する点は、Laravelそのままです。ホストはLaravelの正規表現ではなく `url` クレートによってパースされるため、Laravelなら受け付ける範囲外のポートが、ここでは拒否されます。3つすべてが成り立たなければなりません: `mailto:`、`tel:`、`data:` は名前としては許可リストに載っていますが、authorityの構成要素をまったく持たないため、`Url` はそれらを拒否します。そして `file:///etc/passwd` は3つ目の理由で失敗します - `://` はありますが、3つ目と4つ目の `/` のあいだには何もなく、何もないものはホストではありません。`javascript:` と `vbscript:` は端から拒否されます。そもそも許可リストに載っていません。

`ftp://host/x` と `ssh://host` - 本物のホストで、ただウェブのスキームではないだけのもの - は今も通過します。したがって `Url` は「これはウェブページである」というチェックではなく、そのURLがどこへ解決されるかについては何も語りません。`javascript:` を拒否することは、検証された値を `href` に入れても安全にしますが、取得しても安全にするわけではありません。webhookやコールバックの宛先には、依然として `HttpUrl`（またはあなた自身のスキーム + SSRFのチェック）が必要です。`Url` だけでは、そこはカバーされません。

より狭い集合が欲しいときは、望むスキームを名指ししてください:

```rust
use suprnova::{Rule, rules::Url};

// Laravelの `url:http,https`
Url::protocols(&["https"]).passes("https://example.com")?;   // Ok
Url::protocols(&["https"]).passes("http://example.com");     // Err

// 同じことを、名前付きで
use suprnova::rules::HttpUrl;
HttpUrl.passes("https://example.com")?;
```

`Url::protocols(...)` は許可リストを狭めるのではなく**置き換える**ため、アプリケーションはフレームワークに意見を持たせることなく、自身のディープリンクのスキーム（`myapp://…`）を受け付けられます - `://` に加えてホストを要求する点は、そのカスタムのスキームにも適用されます。コールバック、webhook、アバターの入力には `HttpUrl`（あるいは `Url::protocols(&["https"])`）を使ってください - `ftp://internal-host/` へ解決されるwebhookの宛先も `Url` としてはパースできてしまいますが、`ftp:` の宛先はwebhookの宛先ではありません。

### パスワードの強度

`Password` は長さと文字クラスの強度を、加えて省略可能な Have I Been Pwned の `uncompromised()` チェックを行います - Laravelの `Password` ルールオブジェクトの移植です。`Password::min(n)` で組み立て、強度のビルダーをチェーンしてください:

```rust
use suprnova::{Password, Rule};

let rule = Password::min(8).letters().mixed_case().numbers().symbols();
Rule::passes(&rule, "Str0ng! Pass")?; // Ok(())
Rule::passes(&rule, "weak");          // Err - 短すぎ、数字なし、記号なし
```

| ビルダー | 要求するもの | Laravelの正規表現 |
|---|---|---|
| `.min(n)`（`Password::min` 経由） | 少なくとも `n` 文字（下限は1） | 長さのチェック |
| `.max(n)` | 多くとも `n` 文字 | 長さのチェック |
| `.letters()` | 少なくとも1つのUnicodeの文字 | `/\pL/u` |
| `.mixed_case()` | 大文字と小文字を1つずつ、順序は問いません | `/(\p{Ll}+.*\p{Lu})\|(\p{Lu}+.*\p{Ll})/u` |
| `.numbers()` | 少なくとも1つのUnicodeの数字 | `/\pN/u` |
| `.symbols()` | 少なくとも1つの区切り文字、記号、または句読点 - **素の空白も数えます** | `/\p{Z}\|\p{S}\|\p{P}/u` |

`bootstrap::register()` から一度だけ呼び出す `Password::defaults_with(|| Password::min(12).letters().mixed_case().numbers())` は、ほかのあらゆる場所で `Password::defaults()` が返す、プロセス全体のデフォルトを設定します - Laravelの `Password::defaults(fn () => ...)` です。2回目の呼び出しは、最初のアプリケーションが選んだポリシーを黙って置き換えるのではなく、（`tracing::warn!` を伴って）無視されます。

#### `uncompromised()` - 強度だけでは足りないから

`.uncompromised()`（または `.uncompromised_with_threshold(n)`）は、Have I Been Pwned の漏洩コーパスに対するチェックを、そのk匿名性のレンジAPIを使って追加します: プロセスの外へ出るのは、パスワードの大文字のSHA-1ハッシュの**最初の5文字**だけであり - `GET https://api.pwnedpasswords.com/range/{prefix}` です - 完全なハッシュとの照合は、そのプレフィックスに対してAPIが返す `SUFFIX:COUNT` の行に対して、ローカルで行われます。サービスがパスワードを見ることも、その完全なハッシュを見ることさえもありません。しきい値の比較は厳密で（`count > threshold`）、そのためデフォルトの `uncompromised()`（しきい値 `0`）は、少しでも出現すれば失敗します。そしてネットワークの失敗、タイムアウト、2xx以外のレスポンスは**フェイルオープン**します - Have I Been Pwned の障害のあいだ、すべてのサインアップを止めるのではなく、そのパスワードを問題なしとして扱います。これはLaravelの `NotPwnedVerifier` と正確に一致します。

このチェックはHTTPの往復であるため、`uncompromised()` は、強度のチェックだけなら使えるsyncの `Rule` ではなく `AsyncRule` を必要とします。[`Unique`](#unique-ルール) が使うのと同じ手順で、`after_validation_async` を通じて配線してください:

```rust
use suprnova::{AsyncRule, FormRequest, Password, ValidationErrors, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct Register {
    pub password: String,
}

#[async_trait]
impl FormRequest for Register {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Password::defaults()
            .uncompromised()
            .check_async(&self.password, &mut errs, "password")
            .await;
        errs.into_result()
    }
}
```

`uncompromised()` が設定された `Password` に対してsyncの `Rule::passes` を呼び出すことは、黙って飛ばされるのではなく**はっきりとしたエラー**になります - 静かに何もしないセキュリティのチェックは、はじめから存在しなかったものより悪いからです。エラーメッセージは、直し方として `after_validation_async` を名指しします。

`HIBP_TIMEOUT_SECS`（デフォルトは `30`）がリクエストのタイムアウトを制御します - [環境変数](env-vars.md)を参照してください。

`Err` を返すカスタムの検証器は、チェックが失敗した場合とは別のケースです: そのエラーのテキストは `error` レベルでログに記録され、クライアントには決して届きません。そしてレスポンスは、代わりに `validation-password-unverifiable` というカタログキーを運びます（"The { $field } could not be checked against known data leaks. Please try again."）。自分自身のバリデーションカタログを出荷しているなら、そのキーを追加してください。

### Suprnovaが異なる設計を選んだ理由: Password

- Laravelの `Password` は、失敗した強度のチェックをすべて1つの配列に集めます。Suprnovaの `Rule` の契約は単一の `ValidationMessage` を返すため、`Rule::passes` が報告するのは最初に失敗したチェックであり、その順序は min、max、mixed case、letters、symbols、numbers です - 一覧全体を先に見るのではなく、1つずつ直していくことになります。
- Laravelのsyncのバリデーターは `uncompromised()` を直接呼び出せます。PHPのリクエストは、ブロッキングのHTTP呼び出しを許容するイベントループの内側にすでにいるからです。Suprnovaの `Rule::passes` は契約上同期であるため、そこからHIBPのリクエストを走らせる安全な場所がありません。チェックを黙って飛ばすこと - セキュリティに関わるルールにとって、唯一受け入れられない結末です - をするのではなく、Suprnovaの `Rule::passes` は、直し方として `after_validation_async` を名指しする、はっきりとした開発者向けのエラーを返します。
- `Password::defaults_with` はクロージャではなく素の `fn` ポインタを受け取るため、設定されたデフォルトは `Copy` のままでヒープの割り当てを必要としません - Laravelの `Closure` からの、意図的な絞り込みです。

### 自分のルールを書く

カスタムのルールは、1つのimplを持つユニット構造体（あるいはデータを運ぶ構造体）です。トレイトが `check()` を無償で与えてくれます - これは失敗メッセージがあれば、名指しされたフィールドの下で `ValidationErrors` のバッグへ積み上げます - そのため、そのルールは `validate!` と `after_validation` のフックへ、そのまま差し込めます:

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
// あるいは、validate! の行の中で:
//   stripe_id => Required, StartsWith("acct_");
```

`String` は、そのままレンダリングされる `ValidationMessage` へ変換されます。単一言語のアプリケーションに必要なのは、これだけです。メッセージをロケールごとに翻訳させるには、代わりに*キー付きの*メッセージを返し - `ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` です - そのidを `lang/<locale>/validation.ftl` に定義してください。[ローカライゼーション](localization.md)を参照してください。そこでは、組み込みのルールのメッセージを上書きする方法と、`field-<name>` という命名規約も扱っています。

フィールドを横断するロジックには、代わりに [`ContextualRule`] を実装してください - `passes` メソッドは、検査対象の値と並んで `&FormContext`（兄弟フィールドの値の `HashMap<String, String>`）を受け取ります。データベースに支えられたチェックには [`AsyncRule`] を実装し、それを `after_validation_async` から使ってください。

### 値の形をしたルール

`Rule` が見るのは、いつでも `&str` だけです。2つの組み込みルールは、文字列が運べるより多くの構造を必要とするため、代わりに `&serde_json::Value` の上で `ValueRule` を実装しています:

```rust
use suprnova::{ValueRule, rules::{ArrayKeys, Distinct}};

// Laravelの array:keys - 許された集合の外側にあるキーを拒否します。
// 挙げられたキーがすべて存在している必要はありません。許可の一覧が空
// であることはプログラミングのエラーであり、キーなしのメッセージとして報告されます。
ArrayKeys(&["name", "email"]).passes(&serde_json::json!({"name": "Ada"}))?;

// Laravelの distinct / distinct:ignore_case / distinct:strict。
Distinct { ignore_case: false, strict: false }
    .passes(&serde_json::json!(["a", "b", "c"]))?;
```

`ValueRule` によって検証されるフィールドは、`serde_json::Value` そのもの（あるいは `?:`/`?=>` の行のためには `Option<serde_json::Value>`）を保持していなければなりません - 通常は、JSONのボディから直接引き出したリクエストのフィールドです。`validate!` の行は、同じフィールドの一覧の中で `Rule` と `ValueRule` の両方を受け付けます。どちらのトレイトが走るかは、そのルールの型がどちらを実装しているかによって解決され、行の中にあなたが書く何かによって決まるのではありません。

### メンバーシップのルール

3つのルールが「この値はそのリストの中にあるか？」に答えます。それぞれが、自分の必要とする形の上で答えます:

```rust
use suprnova::{Rule, ValueRule, rules::{Contains, DoesntContain, InArray}};

// Laravelの in_array:allowed_roles.* - 値は、別のフィールドのリストの中に
// 現れなければなりません。リストそのものを渡してください: Vec<String> の
// フィールドでも、&[&str] のリテラルでも、どちらでも動きます。
InArray(&form.allowed_roles).passes(&form.role)?;

// Laravelの contains:rust,web - 配列は、挙げられた値をすべて保持していなければなりません。
Contains(&["rust", "web"]).passes(&form.tags)?;

// Laravelの doesnt_contain:banned - 配列は、それらを1つも保持していてはなりません。
DoesntContain(&["banned"]).passes(&form.tags)?;
```

比較はどれも厳密です。`InArray` は文字列を `==` で比較し、`Contains` と `DoesntContain` はパラメータをJSONの文字列要素とだけ照合します。そのため、`["1"]` は `"1"` を含みますが、`[1]` は含みません。配列でない値は、`Contains` と `DoesntContain` を端から失敗します。

`Contains` と `DoesntContain` は、`ArrayKeys` と同じやり方で、空のパラメータの一覧をキーなしの構築エラーとして拒否します - 何も入っていない一覧は、何も制約しないからです。`InArray` の探索対象の一覧が空である場合は、事情が違います: 兄弟フィールドは実行時に正当に空でありうるため、その値は単に失敗します。

`InArray` の失敗メッセージは、値を1つも名指ししません。そのリストはリクエストから出てくるものであり、バリデーションのメッセージはレスポンスのボディへレンダリングされるからです。

### 比較のルール

`Gt`、`Gte`、`Lt`、`Lte` は、あるフィールドを、数値と、あるいは別のフィールドと比較します。`CompareWith` が、オペランドと尺度を一緒に名指しします:

```rust
use suprnova::{ContextualRule, FormContext, rules::{CompareWith, Gt, Lte}};

let mut ctx = FormContext::new();
ctx.insert("max_price".to_string(), form.max_price.clone());

// Laravelの gt:0 - リテラルのオペランドで、数値として比較します。
Gt(CompareWith::Number(0.0)).passes(&form.price, &ctx)?;

// Laravelの lte:max_price - 兄弟フィールドで、数値として比較します。
Lte(CompareWith::NumericField("max_price")).passes(&form.price, &ctx)?;

// 2つの文字列フィールドに対するLaravelの gt:summary - 文字数で比較します。
Gt(CompareWith::LengthField("summary")).passes(&form.body, &ctx)?;
```

4つとも兄弟フィールドを読むため、これらは `ContextualRule` であり、`validate!` のどの行も `=> with ctx` を運びます - オペランドがリテラルだけで、そのコンテキストが読まれないままの行も含めてです。そこには空の `FormContext` を渡してください。

ルールが測れないものは、そのフィールドを失敗させます: 数値の比較のもとでの有限でない数、フォームがそもそも送らなかった兄弟、数ではない兄弟、あるいは `f64::NAN` のような有限でないリテラルです。そのどれもパニックせず、そのどれも通過しません。

### Suprnovaが異なる設計を選んだ理由

Laravelの `distinct:strict` は、PHPの型を強制変換する `==` に寄りかかっています。JSONの値はすでに型付きであるため、Suprnovaの `strict` が変えるのは、内部表現の異なる2つの*数*（`1` と `1.0`）を等しいと数えるかどうかだけです - どちらのモードでも、文字列と数を「同じもの」にすることは決してありません。

Laravelは、相手のフィールドをルールの文字列 - `in_array:allowed_roles.*` - へ書き込み、バリデーターが実行時にリクエストのデータからそれをグロブで拾い出します。Suprnovaにはルール文字列のパーサーがありません: `InArray` にはリストを直接手渡し、そのフィールドが存在することはコンパイラがチェックします。

Laravel 13.27は、PHPの `==` が `"1abc"`、`true`、`"0x1"` をマッチへ変えてしまうため、`in`、`in_array`、`doesnt_contain` を厳密な比較へ引き締めました。Suprnovaにその穴が空いていたことは一度もありません - `In` と `NotIn` は `&str` を `==` で比較します - そして新しいルールは、JSONの値をバリアントごとに照合します。Laravelの `contains` は緩いままですが、Suprnovaのそれは違います。その代償は、これらのルールが数値の配列を検査できないことです: `Contains(&["1"])` は `[1]` にマッチしません。

Laravelの `gt` の一族は、その尺度を実行時に選びます: 数値にはその数自身、配列には `count()`、ファイルにはキロバイト、それ以外のすべてには文字数の長さであり、数値の分岐は、そのフィールドが `numeric` または `integer` も併せて運んでいるかどうかで決まります。Suprnovaは代わりに、尺度をルールの中へ書き込みます。ここでのルールは、自分のフィールドに載っているほかのルールを見ることができませんし、値の形を嗅ぎ回ることは、これらのルールがまさに避けるために存在している強制変換の習慣だからです。Laravelの4つの尺度のうち2つには、そもそも対応物がありません: ルールが受け取るのは常に文字列だけであるため、配列を値に持つ兄弟は読めませんし、アップロードがルールの表面に到達することはありません - マルチパートのパーサーが、ハンドラがそれを目にするより前にサイズの上限をかけます。
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

- **`field => Rule1, Rule2;`** - 必須形です。ルールは `&self.field` に対して直接実行されます（`String`、`i64`、あるいはルールが期待する借用へとderefできるものすべてが対象です） - あるいは `ValueRule` では、`serde_json::Value` フィールドに直接実行されます。各ルールが使うトレイトは自動的に推論されます。
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

## Inertiaフォーム送信

バリデーション失敗は、二つの対象へ異なる応答を返します。RESTクライアントには `{ message, errors }` を伴う `422` です。Inertia訪問には、エラーをセッションへflashしたフォームページへの `303` backです。Inertiaクライアントは、Inertiaレスポンスとして認識しないあらゆるレスポンスに対してエラーモーダルを表示するため、`422` では `form.errors` が決して満たされません。

ハンドラ側は何も変わりません。宛先ページで各フィールドは最初のメッセージを文字列として運びます:

```svelte
{#if errors?.email}
  <p class="text-red-600">{errors.email}</p>
{/if}
```

エラーバッグ、`with_all_errors`、リダイレクト先については[Inertiaレスポンス](frontend-inertia-responses.md#validation-failures)を参照してください。

## 設計上の注意点

- **部分バリデーション。** `FormRequest` は、バリデーションが実行される前に型付きの構造体へとデシリアライズされるため、その構造体自体がスキーマです。不在でありうるフィールドは `Option<T>` でなければなりません。これはまた、Precognitionが部分的なペイロードをバリデーションできる理由でもあります - 下書きが省略できるフィールドは、オプションにしてください。
- **ルールのメッセージ。** 組み込みのルールは、キー付きのメッセージ（`validation-min` に、その引数と英語のフォールバックを添えたもの）を返し、シリアライズ境界でカタログを通じて解決されます。それらを翻訳・言い換えるには、`lang/<locale>/validation.ftl` に同じIDを定義してください - ルールをラップする必要はありません。[ローカライゼーション](localization.md)を参照してください。
- **`Min` / `Max` / `Between`** は、文字列長のルールです（Unicodeスカラー値で数えます）。数値の範囲については、deriveの `#[validate(range(...))]` かカスタムルールでバリデーションしてください - これらの長さルールは、値の比較ではありません。

## まとめ

| タスク | API |
|------|-----|
| フィールドごとのルール | `FormRequest` に付けた `#[validate(...)]`（リクエストを参照） |
| 合成した / フィールド横断のルール | `validate! { self => ... }` |
| JSON形状のルール（配列/オブジェクト） | `field => ArrayKeys(&[...]);` / `field => Distinct { .. };` |
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

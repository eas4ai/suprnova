# ローカライゼーション

Suprnovaにおけるローカライゼーションは、4つの顔を持つ1つのモジュールです: サーバー上のメッセージカタログ、すでに翻訳された状態で届くバリデーションエラー、ブラウザへ渡される*同じ*カタログのバイト列、そしてロケールを意識した数値・日付・リストのフォーマットです。メッセージ形式は[Fluent](https://projectfluent.org) - MozillaのFTL、Firefoxが出荷しているものです - で、サブシステム全体は `localization` フィーチャーの背後にデフォルトでオンになっています。

可能な限り短いツアーです。カタログを書きます:

```ftl
# lang/en/app.ftl
welcome = Welcome to { $app }!
```

```ftl
# lang/es/app.ftl
welcome = ¡Bienvenido a { $app }!
```

ハンドラから使います:

```rust
use suprnova::{__, handler, HttpResponse, Request, Response};

#[handler]
pub async fn greet(_req: Request) -> Response {
    Ok(HttpResponse::text(__!("welcome", app: "Suprnova")))
}
```

`Accept-Language: es` を持つリクエストは、スペイン語の文字列を得ます。あなたのハンドラが走る前に `LocaleMiddleware` がロケールを解決したからです。ハンドラの中の他のものは何も変わりません - ロケールのパラメータをスレッドすることもなく、シグネチャに `&Translator` を入れることもありません。

## ローカライゼーションが必要な理由

これがクレートを選ぶ話ではなく、フレームワークの関心事である理由が3つあります:

- **バリデーションメッセージは、あなたのではなく、フレームワークの文字列です。** 「The email field is required.」は、あなたが所有するどんなコードからも遠い、`Rule::passes` の奥深くで発されます。フレームワークが翻訳の継ぎ目を運んでいない限り、スペイン語のアプリは英語のバリデーションエラーを出荷するか、あなたがすべてのルールを手作業でラップすることになります。Suprnovaの組み込みルールは*キー付き*のメッセージを返します。`.ftl` ファイルを1つ落とし込むだけで翻訳でき、ルールには決して触れません。
- **ブラウザは同じ文字列を必要とします。** Inertiaアプリは、テキストの半分をRustで、半分をSvelte/React/Vueでレンダリングします。2つの翻訳システムは、2つのファイル形式、2つのレビューワークフロー、そして同じ文が食い違う2つの機会を意味します。Suprnovaは、サーバーが `/_suprnova/lang/<locale>.ftl` から解決したのとまったく同じカタログを提供し、スターターキットはそれを `@fluent/bundle` で解析します - 1組のファイル、1つの真実の源です。
- **複数形とフォーマットはCLDRのデータであって、文字列連結ではありません。** 英語には2つの複数形カテゴリがあり、ロシア語とポーランド語には4つ、アラビア語には6つあります。数値は `en-US` では `1,234.56`、`de-DE` では `1.234,56` です。FluentはCLDRの複数形カテゴリで選択し、ICU4Xがフォーマットを行うため、どちらもロケールごとに手作業で書くようなものではありません。

このフィーチャーをオフにすること（`--no-default-features`）はサポートされています: ローカライゼーションモジュールはコンパイルされず、バリデーションは埋め込まれた英語のフォールバック文字列をレンダリングします。それ以外は何も形を変えません。

## ファイル構成

カタログは `lang/` の下に、ロケールごとに1つのディレクトリで存在します:

```
myapp/
├── lang/
│   ├── en/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── es/
│       ├── app.ftl
│       └── validation.ftl
├── src/
└── frontend/
```

ルールです:

- **ディレクトリ名はBCP-47ロケールです** - `en`、`en-GB`、`pt-BR`、`zh-Hans` です。名前がパースされないディレクトリは、起動を失敗させるのではなく、`warn!` を出してスキップされます。
- **ロケールディレクトリ内のすべての `.ftl` は、ソートされたファイル名の順に、1つのカタログへマージされます**。フィーチャーごとに分割したいだけ分割してください（`auth.ftl`、`billing.ftl`、`emails.ftl`）。メッセージidはロケール内でグローバルなため、`auth.ftl` と `billing.ftl` は同じidを定義してはいけません。
- **フレームワーク自身の英語のバリデーションカタログが最初に読み込まれ**、すべてのロケールのバンドルへ入ります。あなたのファイルはその上に読み込まれ、後の定義が勝ちます。それがオーバーライドの仕組みのすべてです: `lang/es/validation.ftl` の中で `validation-min` を定義すれば、スペイン語のバンドルはあなたのものを使います。
- **ルートは `lang_path()`** です - `<APP_BASE_PATH>/lang` です。バイナリがプロジェクトのルート以外の場所から実行されるとき（systemdユニット、異なる `WorkingDirectory` を持つコンテナ）は `APP_BASE_PATH` を設定するか、`lang` ディレクトリだけを動かすために `use_lang_path("…")` を呼んでください。[環境変数](env-vars.md)を参照してください。
- **`lang/` ディレクトリが存在しないことはエラーではありません。** 新しいアプリは起動しなければならないため、トランスレーターは埋め込みの英語カタログとともに、それ以外は何もない状態で立ち上がります。*壊れた* `.ftl` は別の話です: パースエラーは起動を失敗させ、ファイル名とパーサーが何に異議を唱えたかを名指しします。サイレントに半分だけ読み込まれたカタログは、停止したプロセスより悪いからです。
- **`local` と `development` では、カタログはホットリロードします。** 各リクエストは `lang/` をstatし、実際に何かが変わったときだけ再パースするため、`.ftl` を編集すると次のリフレッシュで反映されます。本番環境は決して再statしません。カタログは起動時に一度だけ読まれます。

## 5分でわかるFTL

Fluentは小さなフォーマットです。このセクションは、典型的なアプリに必要なもののすべてです。

**メッセージ** は `id = value` のペアです。idは慣例としてケバブケースで（フレームワーク自身のものがそうです）、valueは行末まで続き、インデントされた継続行は連結されます:

```ftl
# コメントです。この下のメッセージに紐づきます。
sign-in = サインイン
password-hint =
    12文字以上を使ってください。いくつかの平凡な単語からなる
    パスフレーズは、短い記号の羅列に勝ります。
```

**引数** は `{ $name }` のプレースアブルです。呼び出し時にそれらを供給します。欠けている引数は空文字列ではなくエラーです（`Lang::get` はその後、自身のチェーンをフォールスルーします - [`Lang` ファサード](#lang-ファサード)を参照してください）:

```ftl
greeting = こんにちは、{ $name }さん！
invoice-line = { $qty } × { $item }
```

**用語** は `-` で始まり、カタログに対してプライベートで、ブランド名や繰り返されるフレーズが1か所に存在するために存在します:

```ftl
-product-name = Suprnova
about = { -product-name }について
footer = © 2026 { -product-name }。無断転載を禁じます。
```

**セレクタ** はFluentの条件分岐です。セレクタの値はバリアントのキーに対して照合され、正確に1つのバリアントが `*` でデフォルトとしてマークされます:

```ftl
cart-summary =
    { $count ->
        [0] カートは空です。
        [one] カートに1点入っています。
       *[other] カートに{ $count }点入っています。
    }
```

`[0]` はリテラルな数値ゼロに一致します。`[one]` と `[other]` は**CLDRの複数形カテゴリ**で、バンドルのロケールに対して解決されます - ここがFluentの真価を発揮する場所です。英語には2つのカテゴリがあり、ロシア語には4つあります。ロシア語の翻訳者は、あなたがRustの行を1行も変えることなく、その4つすべてを書きます:

```ftl
# lang/ru/app.ftl
unread-messages =
    { $count ->
        [one] У вас { $count } непрочитанное сообщение.
        [few] У вас { $count } непрочитанных сообщения.
        [many] У вас { $count } непрочитанных сообщений.
       *[other] У вас { $count } непрочитанного сообщения.
    }
```

CLDRは `1`、`21`、`31` を `one` に、`2`–`4`、`22`–`24` を `few` に、`0`、`5`–`20`、`25`–`30` を `many` に、そして端数を `other` に割り当てます。同じ `__!("unread-messages", count: 22)` の呼び出しが、英語、ロシア語、ポーランド語、アラビア語で正しくレンダリングされます。カテゴリの選択がコードではなくデータだからです。

**必ず `*` を `other` に付けてください。** それは、CLDRがすべてのロケールに対して定義している唯一のカテゴリであるため、存在が保証されている唯一のバリアントです - そして、デフォルトは、整数でないカウントを含め、一致しなかったセレクタの値がフォールスルーする先です。`*[many]`（あるいは他の任意のカテゴリ）をデフォルトとしてマークすると、端数が整数のために書かれたテキストへ送られてしまいます。

> **カウントは数値として渡してください。** `__!("unread-messages", count: 3)` はJSONの数値を送り、複数形カテゴリを選択します。`count: "3"` は文字列を送り、それはリテラルなバリアントのキーにしか一致し得ません - それはあなたの `*[other]` デフォルトに着地します。これは、暗記しておく価値のある唯一のFTLの罠です。

**関数** はプレースアブルの内側で呼ばれます。2つが登録されています: `NUMBER()`（Fluentの組み込み）と `DATETIME()`（Suprnovaのもの）です:

```ftl
score = あなたのスコアは { NUMBER($total) } 点中 { NUMBER($points) } 点です。
published = { DATETIME($when, dateStyle: "medium") } に公開されました
```

両方については、[ロケール対応のフォーマット](#ロケール対応のフォーマット)を参照してください。

**1つの意図的な制限:** Suprnovaは、フラットなメッセージの*value*だけを解決します。Fluentの属性構文（`login .placeholder = …`）はパースされますが、`Lang::get` を通じてアドレス可能ではありません。そのため、1つの文字列に1つのidを保ってください: `login.placeholder` ではなく `login-placeholder` です。idはロケールごとにフラットな名前空間です - リゾルバが持たない階層に手を伸ばすのではなく、（`auth-login-title`、`billing-invoice-due` のように）プレフィックスを付けてください。

## `Lang` ファサード

`Lang` はサーバーサイドの入り口です。すべてのメソッドは、ミドルウェアがこのリクエストのためにバインドした**現在のロケール**を読みます。

| メソッド | 戻り値 | 備考 |
|---|---|---|
| `Lang::get(key)` | `String` | 失敗しません。フォールバックチェーンを走り、その後キー自体を返します |
| `Lang::get_with(key, args)` | `String` | 同じですが、引数付きです |
| `Lang::try_get(key)` | `Result<String, FrameworkError>` | 劣化する代わりにエラーになります |
| `Lang::try_get_with(key, args)` | `Result<String, FrameworkError>` | 同じですが、引数付きです |
| `Lang::has(key)` | `bool` | そのキーが、現在のロケール、あるいはそのフォールバックチェーンのどこかで解決するかどうか |
| `Lang::locale()` | `Locale` | 現在のロケール |
| `Lang::set_locale(locale)` | `()` | このリクエストの残りの間、それを変更します |
| `Lang::available_locales()` | `Vec<Locale>` | 読み込まれたカタログを持つすべてのロケール |

```rust
use suprnova::{Lang, Locale, TranslateArgs};

let subject = Lang::get("password-reset-subject");

let mut args = TranslateArgs::new();
args.insert("name".into(), serde_json::json!("Ada"));
args.insert("count".into(), serde_json::json!(3));
let body = Lang::get_with("unread-messages", args);

if Lang::has("beta-banner") {
    // 一部のロケールだけがバナーのコピーを出荷しています。
}

let locales: Vec<String> = Lang::available_locales()
    .iter()
    .map(Locale::as_str)
    .collect();
```

`TranslateArgs` は `String` から `serde_json::Value` への順序付きマップで、どちらもクレートのルートから再エクスポートされています。Fluentの引数は文字列と数値です。それ以外のJSONの形は文字列化されます。

### フォールバックの解決順序

`Lang::get` は決して失敗せず、決して空文字列を返しません。順序は次のとおりです:

1. **現在のロケール**のカタログ。
2. その**設定済みのフォールバック親**（[フォールバックチェーン](#フォールバックチェーン)を参照）を、設定されていれば推移的に辿ります - `pt-BR` 自身が親として名指しするものより前に `pt-BR`、そのより前に `pt-PT`、という具合です。
3. **フォールバックロケール**のカタログ（`APP_FALLBACK_LOCALE`、デフォルトは `en`）です。すでにこのチェーンの前の方に現れていない限り。
4. **キー自体**、そして、欠けている `(locale, key)` のペアごとに1つの `tracing::warn!` です - リクエストごとに1回ではなく、一度だけです。ホットパスにおける欠けたキーが、あなたのログを埋め尽くさないようにするためです。

ステップ4こそが、翻訳の欠落がボタンを空白にするのではなく `checkout-submit` としてレンダリングする理由です: 目に見えて間違った文字列は、起こるのを待つバグレポートですが、空のものは謎です。

劣化するより知りたいときは、`try_*` の兄弟を使ってください。それらはステップ1から3を実行し、ステップ4を行う代わりに `Err` を返します:

```rust
use suprnova::Lang;

// ここでキーが見つからないのは、壊れたメールを意味します - ジョブを失敗させ、
// 件名に生のキーが入ったメッセージを送らないでください。
let subject = Lang::try_get("invoice-paid-subject")?;
```

### `__!` マクロ

`__!` は、Laravelの筋肉記憶のための省略記法です。引数なしでは `Lang::get` を呼び、名前付き引数があれば `TranslateArgs` を組み立てて `Lang::get_with` を呼びます:

```rust
use suprnova::__;

let plain = __!("welcome-back");
let greeted = __!("greeting", name: "Ada");
let counted = __!("unread-messages", name: "Ada", count: 3);
```

引数の値は、`serde_json::Value` へ変換できるものなら何でも構いません - `&str`、`String`、整数、浮動小数点数、`bool` です。このマクロはクレートのルートでエクスポートされているため、`__` をスコープへ持ち込みたくないときは、インポートなしで `suprnova::__!("welcome-back")` が動きます。

## フォールバックチェーン

`APP_FALLBACK_LOCALE` は、すべてのロケールの下にある1つのグローバルな網です。ときには、それでは足りません: ヨーロッパポルトガル語とブラジルポルトガル語はほとんどすべてを共有し、一握りの単語（`ficheiro`/`arquivo`、`utilizador`/`usuário`、`tu`/`você`）でだけ分岐します。2つの完全なカタログを維持することは、新しい文字列すべてを2回書かなければならないことを意味します。**フォールバック親**は、`pt-PT` が、グローバルな `fallback_locale` へさらに後退する前に `pt-BR` から継承することを可能にします - そのため `lang/pt-PT/` は、実際に異なる文字列だけを保持すればよいのです。

### 親を設定する

1つの環境変数で、カンマ区切りの `child=parent` のペアです:

```env
APP_LOCALE_PARENTS=pt-PT=pt-BR
```

あるいは、ビルダーでペアごとに1回、チェーン可能です:

```rust
use suprnova::{Config, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .parent(
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

どちらの経路も同じマップ（`LocalizationConfig::parents`）に流れ込み、どちらもリクエスト時ではなく起動時に検証されます:

- `=` のないペア、あるいは空の子または親は、不正な `APP_LOCALE_PARENTS` エントリです - 起動は、不正なセグメントを名指しして失敗します。
- ペアのどちらか側でBCP-47として無効なロケールも、同じように失敗します。
- 同じ子を2回名指しすることは、後勝ちではなくあいまいな設定です - 起動は、重複した子を名指しして失敗します。
- **循環は起動を失敗させます。** エラーは循環を綴ります: 互いを名指しする2つのロケール（`pt-PT=pt-BR,pt-BR=pt-PT`）は `` `pt-PT` -> `pt-BR` -> `pt-PT` `` を生成します。自分自身を自分の親として名指しするロケール（`pt-PT=pt-PT`）は、その縮小版の同じケースです - `` `pt-PT` -> `pt-PT` `` です。（この誤りは2つのコードパスが送出します: `APP_LOCALE_PARENTS` のパース - そのため、`LocalizationConfig::from_env()` を通る設定を持つあらゆるアプリは、設定の読み込みで失敗します - そして `FluentTranslator` のカタログ読み込みで、`.parent(...)` でプログラム的に構築された循環マップを捕まえます。設定を完全に手作業で構築し、*かつ* `bootstrap_fn` の中で自分自身のカスタム `Translator` をバインドするアプリだけが、両方をスキップします。`Lang` の探索はそれとは独立して保護されており、それでも安全に終了しますが、そこでは大きな起動時エラーは得られません。）

ビルダーの `.parent(child, parent)` は、繰り返された子に対して最後の書き込みが勝ちます - 後の呼び出しが前のものを上書きするのは、単なる後からの上書きであり、`APP_LOCALE_PARENTS` が守っている、あいまいな入力のケースではありません。

### 解決順序

チェーンは1ホップより長くなり得ます: `pt-PT` は `pt-BR` を自分の親として名指しし、`pt-BR` はさらに自分自身の親を名指しできます。`Lang::get` / `try_get` / `get_with` / `try_get_with` / `has` は、すべて、現在のロケールを最初に、この全体を歩きます:

1. **現在のロケール**のカタログ。
2. その**設定済みの親**、それから*その*ロケールの設定済みの親を、設定された親を持たないロケールに到達するまで、推移的に辿ります。
3. グローバルな**`fallback_locale`**（`APP_FALLBACK_LOCALE`）です。すでにチェーンの前の方に現れていない限り - それが単に現在のロケール自身であるという、よくあるケース（`en`/`en` のデフォルト）を含みます。

`Lang::get` / `Lang::get_with` は、[フォールバックの解決順序](#フォールバックの解決順序)が説明しているのとまったく同じように、チェーンの中の何もそれを解決しなければキー自体へフォールスルーします。`Lang::try_get` / `Lang::try_get_with` は `Err` を返し、`Lang::has` は `false` を返します。この歩行は `Lang` ファサード自身の内側で走るため、**あらゆる** `Translator` に対して機能します - バンドルされている `FluentTranslator`、あるいはあなたが書くドライバーです。

### 実行できる例

```
myapp/
├── lang/
│   ├── pt-BR/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── pt-PT/
│       └── app.ftl
├── src/
└── frontend/
```

```ftl
# lang/pt-BR/app.ftl
welcome = Bem-vindo ao { $app }!
file-label = Arquivo
```

```ftl
# lang/pt-PT/app.ftl
file-label = Ficheiro
```

```rust
use suprnova::__;

// `pt-PT` に解決されたリクエストです。
assert_eq!(__!("file-label"), "Ficheiro");                    // pt-PT自身のオーバーライド
assert_eq!(
    __!("welcome", app: "Suprnova"),
    "Bem-vindo ao Suprnova!"                                  // pt-BRから継承
);
```

`lang/pt-PT/` は `welcome` を決して定義しません - その必要がないからです。`file-label` は2つのカタログの間の、本物の一語だけの違いであり、そのためファイルを得るのはそのidだけです。

### 配信されるカタログはフラット化される

`/_suprnova/lang/pt-PT.ftl` エンドポイント（[カタログエンドポイント](#カタログエンドポイント)を参照）は、`pt-BR` が存在することをブラウザに知るよう決して求めません。`FluentTranslator` は、読み込み時にチェーン全体をロケールごとに1つのリソースへ事前マージします - `en`/`en-*` ロケールについては底に埋め込みのフレームワークカタログ、それから設定済みの親チェーン、それからロケール自身のファイル - そして*それ*を、すでにフラット化された状態で提供します。`pt-PT.ftl` をフェッチすると、レスポンスは `welcome` と `file-label` の両方を、クライアント側のチェーンロジックなしに、1回のリクエストで運びます。`?v=<hash>` は、それでも1つの不変のリソースを名指しします。ハッシュは、単に、今では `pt-BR` から引き込まれた文字列もカバーするようになっただけです。

**フラット化がカバーするのは設定済みの親だけです** - 決して `fallback_locale` の先へは届きません。`pt-PT` の配信されるカタログが `pt-BR` の文字列を含むのは、`pt-BR` が*設定済みの親*だからです。`en` がグローバルなフォールバックであるというだけの理由で `en` の文字列を含むことはありません。`LocaleShare` の `fallback` フィールドは、これとは関係なく、常に終端の `fallback_locale` を名指しします - それは、`Lang` のファサードレベルの歩行が最終的にどこへ着地するかをフロントエンドへ伝えるのであって、たった今フェッチしたファイルにすでに何が入っているかを伝えるのではありません。

### 差分ファイルのマージルール

子のカタログは、テキストの連結によってでも、メッセージ全体のシャドーイングによってでもなく、**Fluentの構文木レベルで**親の上にマージされます。オーバーライドの単位は*パターン*です。そのため:

- **子の値は、ファイル内の親の位置で、親の値を置き換えます。**
- **値を持たず属性だけを持つ子のエントリは、親の値を保持します。** `.placeholder` を再翻訳することは、メッセージ自身のテキストを繰り返すことを求めません。
- **属性は名前でマージされます。** 同じ名前の子の属性は、その場で親のものを置き換えます。子だけの属性は、親自身のものの後に追加されます。**子が言及しない属性は、親から生き残ります** - メッセージの値をオーバーライドすることは、その `.placeholder` や `.aria-label` をサイレントに落とすことは決してありません。
- **セレクト式は、バリアントごとにではなく、まるごと置き換えられます。** セレクタのバリアントは、1つのロケールのCLDRの複数形カテゴリにキー付けされています。それらのカテゴリはロケール依存であるため、親から1つのバリアントを、子からもう1つのバリアントを継ぎ接ぎすると、どの1つのロケールの文法にも支えられていないセレクタが生じかねません。セレクタを少しでもオーバーライドする子は、自分が望むすべてのバリアントを供給しなければなりません。
- **オーバーライドされたエントリ上のコメントは、親のものが残ります。** コメントはidを文書化するものであり、オーバーライドの単位はコメントではなくパターンだからです。
- **子だけのエントリは、コメントを含め、子自身の順序で末尾に追加されます** - `pt-BR` が決して定義しなかったidは、何かの「オーバーライド」ではありません。

用語（`-brand`）は同一のルールに従いますが、1つ絞り込みがあります: 用語の値はFluentの構文において決してオプションではないため、上記の「属性はあるが値はない場合は親の値を保持する」というケースはメッセージにのみ適用されます - 子の用語は常に値を供給し、その値が常に勝ちます。属性の名前によるマージ、値のパターン全体の置き換え、そして親が勝つコメントは、すべて、メッセージに対してとまったく同じように用語にも適用されます。用語は自分自身の名前空間で追跡されるため、`-brand` をオーバーライドすることが、同じく `brand` と名付けられたメッセージをシャドーイングすることは決してありません。

### Suprnovaが異なる設計を選んだ理由

Laravel 13には、フォールバックが正確に1つしかありません: 現在のロケールの配列がキーを欠いているときに参照される、単一のグローバルな `fallback_locale` 設定値です。1つのロケールが兄弟ロケールから継承するという概念は存在しません - `pt_PT.php` と `pt_BR.php` は2つの無関係な配列であり、`pt_PT` のアプリは、`pt_BR` がすでに翻訳したものすべてを重複させるか、それなしで出荷するかのどちらかです。

Suprnovaの親チェーンは、Rust側の拡張です: 「このロケール」と「グローバルなフォールバック」の間の中間ステップで、一度グローバルにではなく、ロケールごとに設定されます。私たちが避けたかったトレードオフは、その複雑さをブラウザへ押し付けることです - チェーンを意識したフロントエンドは、`pt-PT.ftl` をフェッチし、それが不完全であることを発見し、`pt-BR.ftl` もフェッチし、サーバーのものと正確に一致しなければならないルールを使って、それらをクライアントサイドのJavaScriptでマージする必要があるでしょう。代わりに読み込み時にフラット化することは、配信されるカタログが常に1つの完全な自己完結型のファイルであることを意味します - 親チェーンが存在する前からフロントエンドがすでに持っていたのと同じ契約であり、そのため `@fluent/bundle` とキットのラッパーは、このフィーチャーをサポートするためにゼロの変更しか必要としませんでした。

## ロケール検出

`LocaleMiddleware` はリクエストごとに1つのロケールを解決し、ハンドラの期間それをバインドします。チェーンは設定駆動で、**最初に当たったものが勝ちます**:

1. **セッション** - [セッションミドルウェア](session.md)が走り、その値が利用可能なロケールを名指ししている場合の、セッションの中の `locale` キーです。ここが、「ユーザーが設定でEspañolを選んだ」が生きる場所です。
2. **クッキー** - `locale` クッキーです。ログアウトを生き延びるため、サインインする前に行われた言語の選択は失われません。
3. **`Accept-Language`** - q値を尊重しながら、`fluent-langneg` で `available_locales()` に対して交渉されます。`en` + `es` のカタログに対する `fr-CH, es;q=0.8, en;q=0.5` は `es` に解決されます。
4. **`APP_LOCALE`** - 上記の何も当たらなかったときの、設定済みのデフォルトです。

パースされない候補、あるいはカタログを持たないロケールを名指しする候補は、**拒否されるのではなく、スキップされます**。古びた `locale=zz` クッキーを持つユーザーは、500ではなくデフォルトの言語を見ます。デタラメな `Accept-Language` ヘッダーも同じです。攻撃者が制御する入力は、あらゆるリクエストでこのチェーンに到達します。それは、言語を選ぶ以上のことを決してできてはいけません。

`bootstrap.rs` の中で、ステップ1がセッションを読むため、セッションミドルウェアの**後に**配線してください:

```rust
use std::sync::Arc;
use suprnova::{
    global_middleware, App, LocaleMiddleware, LocaleShare, SessionConfig, SessionMiddleware,
};

pub async fn register() {
    global_middleware!(SessionMiddleware::install(SessionConfig::from_env()).await);

    // ロケールを解決し、リクエストのためにそれをバインドします。
    global_middleware!(LocaleMiddleware::from_env().expect("locale config"));

    // あらゆるInertiaページで、フロントエンドにロケール + カタログURLを渡します。
    App::register_inertia_shared(Arc::new(LocaleShare));
}
```

`LocaleMiddleware::from_env()` は `LocalizationConfig::from_env()` を読みます。`LocaleMiddleware::new(config)` は、あなたが自分で組み立てたものを受け取ります。スキャフォルドされたアプリには、すでに両方の行があります。

### リクエストの途中でロケールを変更する

`Lang::set_locale` はLaravelの `App::setLocale` です - それは、その時点から先の、現在のリクエストのロケールを書き換えます:

```rust
use suprnova::session::session_mut;
use suprnova::{FrameworkError, Lang, Locale};

/// ユーザーが、設定フォームで言語を切り替えたところです。
pub fn switch_language(choice: &str) -> Result<(), FrameworkError> {
    let locale = Locale::parse(choice)?;
    Lang::set_locale(locale);                       // このリクエスト
    session_mut(|s| s.put("locale", choice));       // それ以降のすべてのリクエスト
    Ok(())
}
```

2つの半分に注目してください: `set_locale` は*この*リクエストに影響し（そのため、リダイレクトのフラッシュメッセージはすでにスペイン語です）、セッションへの書き込みは、検出チェーンが*次の*リクエストで読むものです。

### リクエストの外側で

コンソールコマンド、キューワーカー、そしてスケジュールされたタスクには、リクエストもミドルウェアもありません。そこでは、`Lang::set_locale` は、`Lang::locale()` が `APP_LOCALE` へフォールバックする前に参照する、プロセスグローバルなオーバーライドを書き込みます:

```rust
use suprnova::{command, FrameworkError, Lang, Locale, Mail};

use crate::mail::Digest;
use crate::models::user::User;

#[command(name = "mail:digest", description = "Send the weekly digest")]
pub async fn send_digest(_args: Vec<String>) -> Result<(), FrameworkError> {
    for user in User::query().get().await? {
        // 各ユーザーが保存した設定で、そのメールの間だけ有効です。
        Lang::set_locale(Locale::parse(&user.locale)?);
        Mail::to(&user.email).send(Digest::for_user(&user)).await?;
    }
    Ok(())
}
```

そのオーバーライドはタスクローカルではなくプロセス全体に及ぶため、上記のように、作業の各単位の先頭でそれをセットしてください - 他のタスクが割り込みうる `.await` をまたいで、それが変わらないままだと当てにしないでください。

## 設定

3つの環境変数です。`APP_LOCALE` と `APP_FALLBACK_LOCALE` はどちらもデフォルトが `en` です。`APP_LOCALE_PARENTS` はデフォルトが空です - ロケールごとのオーバーライドはなく、`fallback_locale` だけが適用されます:

```env
APP_LOCALE=en
APP_FALLBACK_LOCALE=en
# APP_LOCALE_PARENTS=pt-PT=pt-BR
```

それ以外はすべて、`LocalizationConfig` の上のコードです。他のあらゆる型付き設定と同じように登録されます - 起動の前に走る、あなたの `config::register_all` の中です:

```rust
// src/config/mod.rs
use suprnova::{Config, Detect, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .default_locale(Locale::parse("es").expect("valid locale"))
        .use_isolating(true)                                // 発散の注記を参照
        .detection(vec![Detect::Session, Detect::Header])   // クッキーを無視する
        .session_key("preferred_locale")
        .cookie_name("lang")
        .parent(                                            // フォールバックチェーンを参照
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

- `default_locale` / `fallback_locale` - `APP_LOCALE` と `APP_FALLBACK_LOCALE` をコードからオーバーライドします。どちらかの場所での不正な値は、サイレントに `en` になるのではなく、起動を失敗させます。
- `use_isolating` - 補間の周りのUnicode分離マークです。デフォルトはオフです。RTLロケールを出荷するときにオンにしてください。
- `detection` - チェーンを、順序どおりに指定します。`Detect::Cookie` を落とすと、言語の選択はセッションの中にだけ生きることになります。`Detect::Header` を落とすと、ブラウザの好みは完全に無視されます。
- `session_key` / `cookie_name` - 2つのルックアップの名前を変えます。
- `parents` - ロケールごとのフォールバック親（`child -> parent`）で、子のカタログにキーが欠けているとき、`fallback_locale` より前に辿られます。`APP_LOCALE_PARENTS` と同じ形です。`.parent(child, parent)` で1つ追加してください - チェーン可能で、繰り返された子には最後の書き込みが勝ちます。完全な契約（起動時の検証、解決順序、配信カタログのフラット化）については、[フォールバックチェーン](#フォールバックチェーン)を参照してください。

起動は、コンテナの中に `Arc<dyn Translator>` をバインドします。あなたのアプリがすでに1つバインドしているなら、フレームワークはそれをそのままにします - これが、何もフォークすることなく、自分自身のトランスレーターを差し替える方法です:

```rust
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::{App, FluentTranslator, LocalizationConfig, Translator};

pub async fn register() {
    let config = LocalizationConfig::from_env().expect("locale config");
    let translator =
        FluentTranslator::from_dir("./catalogs", &config).expect("load catalogs");
    App::bind::<dyn Translator>(Arc::new(translator));
}
```

`Translator` は拡張の継ぎ目です: `translate`、`has`、`available_locales`、`catalog`、`reload` です。1つのドライバーが出荷されており（`FluentTranslator`）、新しいバックエンドは新しいドライバーです - 表面のフォークではありません。

## 翻訳されたバリデーションメッセージ

すべての組み込みルールは、**キー付き**のメッセージを返します: カタログキー、メッセージが必要とする引数、そして英語のフォールバックです。翻訳は一度だけ、シリアライゼーションの境界で起こります - `ValidationErrors::to_json` とInertiaのエラーバッグです - ルールの内側では決して起こりません。ルールは純粋なままで、サブシステム全体はコンパイルアウトできます。

キーは1つの規約に従います:

| 形 | 例 | 用途 |
|---|---|---|
| `validation-<rule>` | `validation-min`、`validation-required-if` | 組み込みルールごとに1つ、ケバブケースです |
| `field-<name>` | `field-email` | フィールドの人間向けの名前 |
| `validation-invalid-data` | - | トップレベルの「The given data was invalid.」バナー |

それらを翻訳するには、対象のロケールの下にある任意の `.ftl` ファイルで、あなたが気にかけるidを定義してください:

```ftl
# lang/es/validation.ftl
validation-invalid-data = Los datos proporcionados no son válidos.
validation-required = El campo { $field } es obligatorio.
validation-email = El campo { $field } debe ser una dirección de correo válida.
validation-min = El campo { $field } debe tener al menos { $min } caracteres.
validation-confirmed = La confirmación del campo { $field } no coincide.
```

`$field` は常に利用可能です。各ルール自身のパラメータは、フレームワークの英語カタログの中で運ばれている名前の下で渡されます - `$min`、`$max`、`$other`、`$value` です。そして `framework/src/localization/catalogs/en/validation.ftl` が、idと引数の正式なリストです。必要なidをそこからコピーしてください。それらすべてをオーバーライドする必要はありません。

オーバーライドは、ロケールごと、キーごとに機能します。`lang/en/validation.ftl` で `validation-min` を定義すると、その1つのルールに対するフレームワークの英語の文言を置き換え、残りはそのままにします。

### フィールド名

生のカラム名を補間すると「The email_address field is required.」が生成されます。`field-<name>` の規約が、それを直します:

```ftl
# lang/en/validation.ftl
field-email_address = email address
field-dob = date of birth
```

レンダリングの前に、トランスレーターは現在のロケールについて `field-<name>` を探します。ヒットは `$field` として渡されます。ミスは、アンダースコアがスペースに変わったフィールド名へフォールバックします。そのため、上のファイルは、人間にとって読みにくい名前のためだけに必要です。

### カスタムルール

`Rule::passes` は `Result<(), ValidationMessage>` を返します。キー付きのメッセージは翻訳に参加します:

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(ValidationMessage::keyed("validation-starts-with")
                .arg("prefix", self.0)
                .fallback(format!("must start with {}", self.0)))
        }
    }
}
```

```ftl
# lang/en/validation.ftl
validation-starts-with = The { $field } field must start with { $prefix }.
```

素のままの文字列も動作し、1つの言語にしか存在しないメッセージにとっては正しい答えです:

```rust
Err("must start with acct_".into())   // キーなし: そのままレンダリングされます
```

キーのないメッセージは翻訳を完全にスキップします。これが、既存のカスタムルールをコンパイルさせ続け、以前とまったく同じにふるまわせ続けるものです。

### deriveフロー

`#[derive(Validate)]` のエラーもキー付きです。`validator` クレートのエラーコードは、アンダースコアがダッシュに変わった `validation-<code>` になり、バリデーターが添付するすべてのパラメータはメッセージの引数になります - `value` と `other` という2つの予約された例外は別で、これらは常に落とされます。どちらも、ルールについてのメタデータではなく、フィールドの実際の*値*を運びます: `value` はテスト対象としてエコーされた入力で、`other`（`must_match`、正規のパスワード確認ルールがセットする）は兄弟フィールドの値です。どちらもカタログへ渡されることは決してないため、`.ftl` のオーバーライドは - `validation-must-match` をどう表現しようとも - 送信された秘密を422のレスポンスボディへ補間することができません。そのため、`#[validate(email)]` の失敗は、手書きのルールと同じように `validation-email` を解決し、一方を翻訳するロケールは両方を翻訳します。

## フロントエンド

ブラウザは、サーバーが解決したのとまったく同じバイト列を得ます。何も再翻訳されず、再エクスポートされず、手作業で同期されません。

### カタログエンドポイント

```
GET /_suprnova/lang/es.ftl              → 200 text/plain, ETag: "<hash>"
GET /_suprnova/lang/es.ftl?v=<hash>     → 200 + Cache-Control: public,
                                          max-age=31536000, immutable
GET /_suprnova/lang/es.ftl              → 304 when If-None-Match matches
GET /_suprnova/lang/zz.ftl              → 404（そのようなカタログはありません）
```

本体は、そのロケールのためのマージされたカタログです - まずフレームワークのメッセージ、それから設定済みのフォールバック親チェーンがあればそれ（[フォールバックチェーン](#フォールバックチェーン)を参照）、それから読み込み順のあなたのファイルです。`ETag` はコンテンツのハッシュです。`?v=` で特定のハッシュを求めれば、そのURLは決してただ1つのことしか意味し得ないため、レスポンスは永遠にimmutable-cacheableです。それなしで求めれば、代わりに再検証を得ます。`/_suprnova/health` と同じく、このパスはミドルウェアチェーンから免除されています: それは、ロケールが解決される前に答えなければならず、ユーザーデータを一切運びません。

### 共有データ

`LocaleShare` は、フレームワークが出荷する `InertiaSharedData` です。（[ロケール検出](#ロケール検出)を参照して）`bootstrap.rs` の中に登録されると、あらゆるInertiaページに1つのpropを追加します:

```json
{
  "lang": {
    "locale": "es",
    "fallback": "en",
    "catalog": {
      "url": "/_suprnova/lang/es.ftl?v=9f2c1ae4",
      "hash": "9f2c1ae4"
    }
  }
}
```

トランスレーターがバインドされていないとき、`catalog` は `null` です - この共有は、ページのレンダリングを決して失敗させません。

### キットラッパー

各スターターキットは、そのpropを読み、カタログを一度フェッチし、`@fluent/bundle` のバンドルを構築し、`t()` を公開する、約100行のラッパーを出荷します。あなたのInertiaのエントリポイントで、一度 `initLang` を呼んでください（スキャフォルドされたアプリはすでにそうしています）:

```ts
// frontend/src/main.ts
import { createInertiaApp } from '@inertiajs/svelte'
import { mount } from 'svelte'
import { initLang } from './lib/lang.svelte'

createInertiaApp({
  resolve: (name) => { /* …変更なし… */ },
  async setup({ el, App, props }) {
    await initLang(props.initialPage)
    mount(App, { target: el!, props })
  },
})
```

それから、コンポーネントの中では:

```svelte
<!-- Svelte 5 -->
<script lang="ts">
  import { t, currentLocale } from '../lib/lang.svelte'
</script>

<h1>{t('welcome', { app: 'Suprnova' })}</h1>
<p>{currentLocale()}</p>
```

```tsx
// React 19
import { useLang } from '../lib/lang'

export default function Home() {
  const { t, locale } = useLang()
  return <h1>{t('welcome', { app: 'Suprnova' })}</h1>
}
```

```vue
<!-- Vue 3.5 -->
<script setup lang="ts">
import { useLang } from '../lib/lang'
const { t, locale } = useLang()
</script>

<template>
  <h1>{{ t('welcome', { app: 'Suprnova' }) }}</h1>
</template>
```

クライアント側の数値と日付のフォーマットは、ブラウザ組み込みの `Intl` を使います - ICUのデータはブラウザへ一切出荷されません。

### 型付きのメッセージキー

`suprnova generate-types` は `lang/<デフォルトロケール>/*.ftl` を解析し、ページpropsの型と並んで、すべてのメッセージidのユニオンを出力します:

```ts
// frontend/src/types/lang-keys.ts
// Generated by `suprnova generate-types` - do not edit.
export type MessageKey =
  | "validation-min"
  | "welcome"
```

ラッパーは `t(key: MessageKey, …)` の型を付けるため、これは[`inertia-props.ts`](frontend-typescript-types.md)と同じ約束です: Rustでメッセージの名前を変え、再生成すれば、TypeScriptのコンパイラが、古いidをまだ使っているすべての呼び出しサイトを指し示します。`suprnova serve` は `src/` と並んで `lang/` を監視するため、カタログを編集するとファイルは再生成されます。

`lang/` ディレクトリもメッセージidも持たないプロジェクトは、**ファイルを一切得ません** - ローカライズされていないアプリには、新しいアーティファクトが一切現れません。

## ロケール対応のフォーマット

`Lang` の上の7つの関数で、すべてがICU4Xに支えられ、すべてが現在のロケールを読み、すべてに、劣化する代わりに `Result<String, FrameworkError>` を返す `try_*` の兄弟があります:

```rust
use suprnova::chrono::NaiveDate;
use suprnova::{DateStyle, Lang, ListStyle, RelativeUnit, TimeStyle};

let dt = NaiveDate::from_ymd_opt(2026, 8, 1)
    .and_then(|d| d.and_hms_opt(14, 30, 0))
    .expect("valid datetime");

Lang::number(1_234_567.89);                          // en-US → 1,234,567.89
                                                     // de-DE → 1.234.567,89
Lang::currency(19.99, "USD");                        // en-US → $19.99
Lang::date(&dt, DateStyle::Long);                    // en-US → August 1, 2026
Lang::time(&dt, TimeStyle::Short);                   // en-US → 2:30 PM
Lang::datetime(&dt, DateStyle::Medium, TimeStyle::Short);
Lang::list(&["Ada", "Grace", "Alan"], ListStyle::And); // → Ada, Grace, and Alan
Lang::relative(-3, RelativeUnit::Day);               // → 3 days ago
```

スタイルのenumです: `DateStyle { Full, Long, Medium, Short }`、`TimeStyle { Medium, Short }`、`ListStyle { And, Or, Unit }`、`RelativeUnit { Second, Minute, Hour, Day, Week, Month, Year }` です。`Lang::relative` は符号付きの量を取ります - 負は過去（「3 days ago」）、正は未来（「in 3 days」）です。

> 正確な出力は、ICU4Xに焼き込まれたCLDRデータに由来し、特に日付と通貨については、ICUのアップグレードをまたいで変わりえます。あなた自身のテストでは、正確なバイト列ではなく、形とロケールごとの違い（`de != en`、`2026` を含む）をアサートしてください。

### メッセージの中でのフォーマット

FTLから呼び出せる関数が2つあります:

```ftl
order-total = 合計金額は { NUMBER($amount, maximumFractionDigits: 2) } です。
published = { DATETIME($when, dateStyle: "medium", timeStyle: "short") } に公開されました
```

```rust
use suprnova::__;

let line = __!("published", when: "2026-08-01T14:30:00");
```

`NUMBER()` はFluentの組み込みで、明示的に登録されており、メッセージの中で小数桁数の制御を与えてくれます。`DATETIME()` はSuprnovaのものです: `$value` はISO-8601の文字列かエポックミリ秒を受け入れ、`dateStyle` / `timeStyle` はRustのenumと同じ名前を、小文字で取ります。パースできない値は `warn!` とともにそのまま通過します - Fluentの関数はエラーを返せず、見た目が少し変な日付を持つレンダリング済みのページの方が、500よりましだからです。

Fluentの関数が公開しているものより、ICU4Xの完全なフォーマットが欲しいときは、Rustでフォーマットして、出来上がった文字列を渡してください:

```rust
use suprnova::{__, Lang};

let total = __!("order-total-text", amount: Lang::currency(19.99, "USD"));
```

## 翻訳をテストする

2つのヘルパーが仕事をします: `use_lang_path` はローダーをフィクスチャのディレクトリへ向け、`scope_locale` は、あるfutureの期間、現在のロケールを固定します。

ヘルメティックな形 - フィクスチャのディレクトリの上にトランスレーターを構築し、それをテストスコープのコンテナへバインドする - は、フレームワーク自身のテストが使っているものです。プロセスグローバルな状態に一切触れず、並列のテスト実行を生き延びるからです:

```rust
use std::sync::Arc;
use suprnova::testing::TestContainer;
use suprnova::{scope_locale, FluentTranslator, Lang, Locale, LocalizationConfig, Translator};

#[tokio::test]
async fn spanish_greeting_comes_from_the_catalog() {
    let _guard = TestContainer::fake();

    let config = LocalizationConfig::from_env().expect("locale config");
    let translator = FluentTranslator::from_dir("tests/fixtures/lang", &config)
        .expect("load catalogs");
    TestContainer::bind::<dyn Translator>(Arc::new(translator));

    scope_locale(Locale::parse("es").expect("locale"), async {
        assert_eq!(Lang::get("welcome"), "¡Bienvenido!");
        assert_eq!(Lang::locale().as_str(), "es");
    })
    .await;
}
```

`use_lang_path` は、テストが本物のアプリケーションを起動し、*アプリ全体*をフィクスチャへ向けたいときの正しい道具です:

```rust
use suprnova::use_lang_path;

#[tokio::test]
async fn app_boots_against_fixture_catalogs() {
    use_lang_path("tests/fixtures/lang");
    // …アプリを起動する。すると `lang_path("")` は、フィクスチャのディレクトリに解決されます。
}
```

それはプロセスグローバルなパスのオーバーライドを書き込むため、並列な2つのテストが食い違いうるものとしてではなく、バイナリごとの設定として扱ってください。

検出そのもの - セッション/クッキー/`Accept-Language` のチェーン - は、ミドルウェアを直接呼び出すのではなく、本物のパイプラインを通じてテストする価値があります。興味深いケースは、ヘッダーのパースと、どのソースが勝つかについてだからです。ハンドラが `__!("welcome")` を返すルートをマウントし、`MiddlewareRegistry` に `LocaleMiddleware` を登録し、[HTTPテスト](http-tests.md)のループバックハーネスでそれを駆動し、`Accept-Language: fr, es;q=0.8` を送って、スペイン語の本文をアサートしてください。固定しておく価値のあるケースです: ヘッダーが交渉すること、クッキーがヘッダーに勝つこと、利用できないロケールはエラーにならずスキップされること、そして不正なヘッダーもそれでも200を返すことです。

テストがマルチスレッドのランタイム上で走るときの `TestContainer::scope` については、[テスト](testing.md)を参照してください - 上の、スレッドローカルな `fake()` ガードは、ワーカー間を移動するfutureを生き延びません。

### Suprnovaが異なる設計を選んだ理由

**PHPの配列ではなく、FTLファイルです。** Laravelには2つの形式があります - `lang/en/messages.php` の中のネストした配列と、文字列キーの翻訳のための `lang/en.json` の中のフラットなJSONです - そして、どちらもブラウザから読み込めず、ファイルの中で複数形の選択を表現することもありません: それは、文字列の内側にある `trans_choice` のパイプと範囲の規約の中に生きています。Fluentは、サーバーとクライアントの両方が解析する1つの形式を私たちに与えます。それが、「フロントエンドが、バリデーターが生成したのと同じ文字列を表示する」ことを、あなたが維持する規約ではなく、設計の性質にしているものです。それは、学ぶべき新しい構文（このチャプターの大半がそれです）と、ツールの変更というコストを伴います: Poeditは `.ftl` を編集できませんが、Crowdin、Weblate、Lokalise、Pontoonはできます。それはまた、ドット区切りの名前空間というコストも伴います - `trans('messages.welcome')` には相当するものがありません。idはロケールごとにフラットな名前空間だからです。代わりにプレフィックスを付けてください。

**`trans_choice` はありません。** Laravelは、パイプ区切りの文字列と明示的な範囲で複数形を選択します:

```php
// Laravel
trans_choice('{1} plik|[2,4] pliki|[5,*] plików', $count);
```

さて、ポーランド語で22まで数えてみてください。CLDRは22を `few` カテゴリに置きます - `22 pliki` です - しかし `[5,*]` はそれを飲み込んで `22 plików` を生成します。同じ破綻が32、42、102で起こり、ロシア語、アラビア語、チェコ語、リトアニア語、ウェールズ語でも、それぞれ自分自身の場所で起こります。整数の範囲は複数形のルールを表現できません。複数形のルールは範囲についてのものではなく、最後の桁、最後の2桁について、そしていくつかの言語では、その値がそもそも整数かどうかについてのものだからです。Fluentは直接CLDRのカテゴリで選択するため、`$count` は普通の引数であり、*翻訳者* - その言語を知っている人 - がポーランド語の4つのカテゴリすべてを書きます:

```ftl
files =
    { $count ->
        [one] { $count } plik
        [few] { $count } pliki
        [many] { $count } plików
       *[other] { $count } pliku
    }
```

`one` は1です。`few` は2–4、22–24、32–34、102–104です。`many` は0、5–21、25–31です。`other` は端数（`1,5 pliku`）を捉え、上のルールに従ってデフォルトのマーカーを運びます。

Laravelの範囲なしの形（`plik|pliki|plików`）はより良くやります - それは言語ごとのインデックスを参照し、*n*番目のセグメントを選びます - しかし、そのインデックスはCLDRのデータではなく手作業で保守されるテーブルであり、CLDRが4つのカテゴリを定義しているところにポーランド語に3つのセグメントを提供し、セグメントはレビューするためのカテゴリ名のない位置的なものであり、カウントに対してしか選択できません。

これが2つ目の利点であり、ただで手に入ります: Fluentのセレクタは、カウントだけでなく*どんな*引数に対しても切り替えられます。性別、プラン階層、接続状態も同じように選択でき、そのどれもが新しいファサードメソッドを必要としませんでした。

**分離マークはデフォルトでオフです。** Fluentは通常、すべての補間をU+2068（FIRST STRONG ISOLATE）とU+2069（POP DIRECTIONAL ISOLATE）で包みます。左から右の文の中に埋め込まれた右から左の値が、正しい順序でレンダリングされるようにするためです。正しい挙動です - そして目に見えません。つまり、英語のみのアプリにおけるあらゆる `assert_eq!("Hello Ada", …)` は、diffの中で誰にも見えない2つの文字とともに失敗します。私たちはそれらをデフォルトでオフにし、オンにすることを1回の呼び出しにしています:

```rust
let config = LocalizationConfig::from_env()?.use_isolating(true);
```

**RTLロケールを出荷するとき - アラビア語、ヘブライ語、ペルシャ語、ウルドゥ語 - あるいは、ユーザー提供の値が1つの文の中でスクリプトを混在させるロケールでは、それらをオンにしてください。** それから、マークを運ぶ文字列と比較するようにあなたのアサーションを更新するか、アサーションのヘルパーの中でそれらを取り除いてください。デフォルトは一般的なケースのために最適化されており、正しいケースは1行離れたところにあります。この段落は、それを行うための注意書きです。

## 次のステップ

- [バリデーション](validation.md) - ルール、`validate!` マクロ、そして `ValidationMessage` がどこから来るか
- [TypeScript 型](frontend-typescript-types.md) - `generate-types`、`inertia-props.ts`、そして `lang-keys.ts`
- [ミドルウェア](middleware.md) - グローバルチェーンの残りに対して `LocaleMiddleware` を順序付ける
- [セッション](session.md) - 最初の検出ステップが読むストア
- [環境変数](env-vars.md) - `APP_LOCALE`、`APP_FALLBACK_LOCALE`、`APP_LOCALE_PARENTS`、`APP_BASE_PATH`
- [テスト](testing.md) - `TestContainer`、`#[suprnova_test]`、そしてヘルメティックなDIのオーバーライド

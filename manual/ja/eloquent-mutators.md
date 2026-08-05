# Eloquent キャスト、アクセッサーとミューテータ

キャストは、カラムがディスク上に保持するものと、あなたのモデルがメモリ上に運ぶものの境界を調停します。アクセッサーは、すでにあるカラムから仮想属性を作り出します。ミューテータは、あなた自身の変換を経由して、フィールドへの書き込みをルーティングします。自動管理されるタイムスタンプと合わせて、これらは、フラットな行を型付けされたRustの値へ変える、4つの動く部品です。

この章は、キャストの表面全体（すべての組み込みの型、`casts!` による実行時のオーバーライド、暗号化とハッシュ化）、`#[accessor]` と `#[mutator]` のアトリビュートマクロ、`touch()` と `without_touching` を含む自動タイムスタンプの契約、そして `replicate()` でモデルをクローンしたときに発火する `Replicating` ライフサイクルイベントをカバーします。

より広いモデルの表面（`#[suprnova::model]`、クエリビルダー、リレーションシップ、オブザーバー）については、[Eloquent API](eloquent.md)の章を参照してください。ライフサイクルイベントの全体については、[イベント](events.md)を参照してください。暗号化されたキャストが使う暗号のファサードについては、[暗号化](encryption.md)を参照してください。

## キャストの仕組み

すべてのキャストは、`Cast` トレイトを実装する構造体です:

```rust
pub trait Cast: Send + Sync {
    type Runtime;
    type Storage;

    fn to_storage(value: &Self::Runtime) -> Result<Self::Storage, FrameworkError>;
    fn from_storage(stored: &Self::Storage) -> Result<Self::Runtime, FrameworkError>;
}
```

`Runtime` は、あなたがモデル構造体に書くRustの型です（`bool`、`chrono::NaiveDate`、`rust_decimal::Decimal`、あなた自身のenum）。`Storage` は、SeaORMがカラム上で目にする型です（SQLiteのbooleanカラムには `i64`、TEXTの日付には `String`）。どちらの方向も失敗しえます - 時間系と10進数のパースは、不正な形式の入力を拒否できます - そのため、マクロは `From<inner::Model>` と `ActiveModel` の書き込み経路を通じて `Result` を伝播させます。

キャストは明示的です。`Vec<String>` フィールドは、暗黙的に `AsArray<String>` になったりしません。マクロの時点でのフィールド型の検査は、エイリアスの名前を変えたり、別の `Vec` をインポートしたりした瞬間に壊れてしまうからです。キャストは、マクロのアトリビュート上で宣言します:

```rust
use suprnova::{model, AsArray, AsBool, AsJson};

#[model(
    table = "posts",
    casts = {
        tags = AsArray<String>,
        published = AsBool,
        metadata = AsJson<serde_json::Value>,
    },
)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub tags: Vec<String>,
    pub published: bool,
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

マクロは、各 `field = CastType` のエントリを、すべての読み取りと書き込みにおける `Cast::to_storage` と `Cast::from_storage` への呼び出しへ展開します。あなた自身がキャストを呼び出すことは決してありません - あなたはランタイムの型を書き、キャストがカラムの形を配線します。

### Suprnovaが異なる設計を選んだ理由

Laravelは、`protected $casts = ['tags' => 'array']` としてキャストを宣言します。文字列 `'array'` は、実行時のルックアップを介してクラスへ解決されます。つまり、キャストの名前は、実行されるまで型を持たない文字列のまま存在するということです。Suprnovaは型を直接取ります - `AsArray<String>` は、マクロがコンパイル時にチェックする、本物のRustの型です。キャスト名のタイプミスはコンパイルエラーであり、デプロイの3週間後の実行時例外ではありません。

## プリミティブなキャスト

5つのキャストが、SQLのスカラー型をカバーします。

### `AsBool`

`bool` ↔ `INTEGER`（0 / 1）です。SQLiteにはネイティブのbooleanカラムがありません。PostgresとMySQLはどちらも、SeaORMの `Value::Int` の境界を介して `i64` をきれいに往復させます。1つのストレージの形により、あらゆるバックエンドに対して同じキャストを使えます。

```rust
#[model(table = "settings", casts = { dark_mode = AsBool })]
pub struct Settings {
    pub id: i64,
    pub dark_mode: bool,
}
```

### `AsInt<I>`

より狭い整数（`i32`、`u32`、`i16`）↔ `i64` です。SeaORMはカラム上で整数を `i64` として格納します。このキャストは、読み取り時に縮小し、書き込み時に拡大します。範囲外の値は、サイレントに切り詰められるのではなく、読み取り時にバリデーションエラーを生じます。

```rust
#[model(table = "counters", casts = { age = AsInt<u32> })]
pub struct Counter {
    pub id: i64,
    pub age: u32,
}
```

ランタイムの型がすでにストレージと一致している場合は、`AsInt<i64>`（あるいはキャストの省略）を使ってください。

### `AsFloat`

`f64` ↔ `REAL` です。両方向でそのまま通します - このキャストは、Laravelの `'float'` キャストとの命名の対応のために存在します。バックエンドは、floatをネイティブに往復させます。

### `AsString`

`String` ↔ `TEXT` です。これもそのまま通します。このキャストが存在するのは、`Builder::with_casts(...)` の実行時オーバーライドが、他のあらゆるキャストと同じように、これを `DynCast` へ消去できるようにするためです。

### `AsDecimal<P>`

`rust_decimal::Decimal` ↔ `TEXT` です。`P` は精度（小数点以下の桁数）であり、値はストレージへ向かう途中で `P` 桁に丸められます。デフォルトは `P = 4` です。ストレージは固定形式の文字列であるため、往復はバックエンドにとらわれません - SeaORMのネイティブな `Decimal` カラム型は、ドライバーごとに異なる精度のセマンティクスを持っており、文字列による往復はそれを回避します。

```rust
use rust_decimal::Decimal;
use suprnova::AsDecimal;

#[model(
    table = "ledger",
    casts = { amount = AsDecimal<2> },  // 通貨、小数点以下2桁
)]
pub struct LedgerEntry {
    pub id: i64,
    pub amount: Decimal,
}
```

## 時間系のキャスト

6つのキャストが、日付、日時、不変な変種、そしてUnixタイムスタンプをカバーします。タイムスタンプでないすべてのキャストは `TEXT`（ISO-8601 / RFC-3339）として格納されるため、往復はすべてのドライバーで機能します - SQLiteはネイティブに日時を文字列として格納し、Postgres / MySQLは、SeaORMの `Value::String` の境界を介してそれらを受け入れます。

### `AsDate`

`chrono::NaiveDate` ↔ `TEXT`（`YYYY-MM-DD`）です。

```rust
use chrono::NaiveDate;
use suprnova::AsDate;

#[model(table = "people", casts = { birthday = AsDate })]
pub struct Person {
    pub id: i64,
    pub birthday: NaiveDate,
}
```

### `AsDateTime`

`chrono::DateTime<Utc>` ↔ `TEXT`（RFC-3339）です。壁時計的な表現が欲しいときの、任意のタイムスタンプに対するデフォルトのキャストです。

### `AsImmutableDate` と `AsImmutableDateTime`

`AsDate` / `AsDateTime` と同じストレージの形です。Rustの借用チェッカーは、`&` 参照を通じて不変性をすでに強制しているため、これらのキャストは背後の型を共有します - これらは、Laravelの `immutable_date` / `immutable_datetime` との対応のため、そしてモデル宣言の場所で意図を文書化するために存在します。

### `AsOptionalDateTime`

`Option<DateTime<Utc>>` ↔ `Option<String>` です。null許容のトゥームストーンカラム（デフォルトは `deleted_at` - [ソフトデリート](eloquent.md#deleting-and-soft-deletes)を参照）のために、`#[model(soft_deletes)]` フラグによって自動的に注入されます。包まれたOptionは、ストレージのカラムをnull許容のままにするため、ソフトデリートされた行と生きている行は、センチネル値なしに `IS NULL` で区別されます。

RFC-3339テキストとして往復させたい、他のあらゆるnull許容の日時カラムに、このキャストを直接使ってください:

```rust
#[model(
    table = "subscriptions",
    casts = { cancelled_at = AsOptionalDateTime },
)]
pub struct Subscription {
    pub id: i64,
    pub cancelled_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

### `AsTimestamp`

Unixエポックの `i64` ↔ `INTEGER` です。カラムが数値の範囲としてクエリされる、あるいは算術で使われる場合に使ってください。`AsDateTime` とは異なります - `WHERE created_unix > 1700000000` が欲しいときは `AsTimestamp` を、ログの中にRFC-3339の文字列が欲しいときは `AsDateTime` を選んでください。

## 構造化されたキャスト

5つのキャストが、コレクション、構造体、そして任意のJSONをカバーします。すべて、ランタイムの値をJSONテキストへシリアライズし、`TEXT` カラムに格納します。Postgresのネイティブな `JSON` / `JSONB` と、MySQLの `JSON` カラムは、同じ文字列のペイロードを受け入れます - インデックスのためにネイティブなJSONカラム型が欲しい場合は、マイグレーションの中で手動で宣言してください。キャスト層はカラム型を制約しません。

### `AsArray<T>`

`Vec<T>` ↔ JSONエンコードされた `TEXT` です。要素の型は `Serialize + DeserializeOwned` でなければなりません。

```rust
use suprnova::AsArray;

#[model(table = "posts", casts = { tags = AsArray<String> })]
pub struct Post {
    pub id: i64,
    pub tags: Vec<String>,
}
```

### `AsObject<T>`

`Serialize + DeserializeOwned` な構造体 ↔ JSONエンコードされた `TEXT` です。ランタイムの形が、静的に既知のキーを持つ固定レコードである場合に使ってください。

```rust
use serde::{Deserialize, Serialize};
use suprnova::AsObject;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    pub theme: String,
    pub notifications: bool,
}

#[model(table = "users", casts = { prefs = AsObject<Prefs> })]
pub struct User {
    pub id: i64,
    pub prefs: Prefs,
}
```

### `AsCollection<T>`

`Collection<T>` ↔ JSONエンコードされた `TEXT` です。Suprnovaの `Collection<T>`（Laravel形のスライスの表面を持つ `Vec<T>` のニュータイプ - [コレクション](eloquent.md#collections)を参照）を介して往復する、`AsArray` の上の薄いラッパーです。

### `AsJson<T>`

あらゆる `Serialize + DeserializeOwned` な型 ↔ JSONエンコードされた `TEXT` です。フィールドが `serde_json::Value` である場合、あるいは、すでにserdeの言葉で完全に記述可能だが、固定された形の `AsObject` パターンには合わないユーザー定義の構造体（例えばenumのペイロード、型を持たないマップ）である場合に使ってください。

### `AsArrayObject<T>`

`IndexMap<String, T>` ↔ JSONエンコードされた `TEXT` です。ランタイムの形が動的なキーのマップであり、キーの順序が重要である場合（ラベルのUI上の順序、設定ブロックの正規の順序）に使ってください。`HashMap` の代わりに `IndexMap` を使うのは意図的です: serdeは `IndexMap` を介して挿入順を保持し、Suprnovaの `serde_json` も、同じ理由からすでに `preserve_order` で設定されています。

固定された形のレコードには `AsObject` を、配列には `AsArray` を使ってください。

## enumキャスト

### `AsEnum<E>`

`E: FromStr + AsRef<str>` ↔ `TEXT` です。カラムに届くのは、enumのバリアント名（あるいは、その `AsRefStr` でカスタマイズされた文字列）です。`strum` へのフレームワークのロックインはありませんが、それら2つの境界を自分で手作りせずに得る、最もエルゴノミクスに優れた方法です:

```rust
use suprnova::AsEnum;

#[derive(Debug, Clone, Copy, strum::EnumString, strum::AsRefStr)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

#[model(
    table = "users",
    casts = { role = AsEnum<Role> },
)]
pub struct User {
    pub id: i64,
    pub role: Role,
}
```

整数判別子によるストレージは、意図的にデフォルトになっていません。`Role::Admin = 0` が、並べ替えの後に `Role::Admin = 2` になると、データベース内のすべてのadminをサイレントに入れ替えてしまいます。バリアント名は、DBブラウザの中で自己記述的であり、並べ替えをまたいで安定しています。

## 暗号化とハッシュ化

5つのキャストが、ストレージの境界における暗号変換を仲介します。4つの `AsEncrypted*` キャストはすべて、[`Crypt`](encryption.md)ファサードを共有します - そのファサードは、どれかが実行される前に初期化されていなければなりません。本番のアプリは、これを `Server::from_config`（環境から `APP_KEY` を読み取ります）を介して得ます。テストは、起動時に一度 `suprnova::testing::install_test_encryption_key()` を呼びます。

### `AsEncrypted`

`String` ↔ AES-256-GCMで暗号化された `String` です。ディスク上のカラムは、`nonce || ciphertext_with_tag` のURL安全なbase64を保持します。各書き込みは新しいランダムなnonceを使うため、同じ平文の2回の書き込みは異なる暗号文を生成します - あなたのDB管理者は、保存されているシークレットの重複を識別できません。

```rust
use suprnova::AsEncrypted;

#[model(
    table = "secrets",
    casts = { api_key = AsEncrypted },
)]
pub struct Secret {
    pub id: i64,
    pub api_key: String,  // ランタイムはプレーンなUTF-8
}
```

ランタイムの値は、復号されたUTF-8文字列です。他のどんな `String` とも同じように、それを読み書きできます。

### `AsEncryptedArray<T>` / `AsEncryptedObject<T>` / `AsEncryptedCollection<T>`

`Vec<T>` / `T` / `Collection<T>` ↔ AES-256-GCMで暗号化されたJSONです。パイプラインは: JSONへシリアライズ → 暗号化 → base64 → 格納です。読み取り時はその逆です。要素/値の型は `Serialize + DeserializeOwned` でなければなりません。

```rust
use suprnova::AsEncryptedObject;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct CardOnFile {
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[model(
    table = "billing",
    casts = { card = AsEncryptedObject<CardOnFile> },
)]
pub struct Billing {
    pub id: i64,
    pub card: CardOnFile,
}
```

### キーローテーション

`Crypt` ファサードは、`APP_KEY_PREVIOUS` を介したローテーションをサポートします: 暗号化は常に `APP_KEY` を使いますが、復号はまず `APP_KEY` を試し、プライマリキーが失敗すれば `APP_KEY_PREVIOUS` にフォールバックします。ローリングでの再暗号化の戦略はこうです: `APP_KEY` を新しいキーに設定し、古いキーを `APP_KEY_PREVIOUS` へ移し、それから、新しいキーの下で暗号文を書き直すために、暗号化されたすべての行を `save()` します。キャスト層は、ローテーションについて知る必要がありません - それは、すべての読み取りと書き込みで `Crypt` を介して往復するため、`User::all().await?` に続けて各行を保存すれば、カラムはその場で移行します。完全なローテーションのプロトコルについては、[暗号化](encryption.md)を参照してください。

### `AsHashed`

`String` ↔ 書き込み時にハッシュ化される文字列です。有効なハッシュドライバー（`HASH_DRIVER` 環境変数 - デフォルトはbcrypt、argon2iとargon2idもサポートされています）を使います。ランタイムの値はハッシュ化された文字列そのものです。逆方向はありません。Laravelの `hashed` キャストを反映しています。

```rust
use suprnova::AsHashed;

#[model(
    table = "users",
    casts = { password = AsHashed },
)]
pub struct User {
    pub id: i64,
    pub password: String,
}
```

`AsHashed::to_storage` は**べき等**です: すでに認識されるどれかのハッシュ（bcryptの `$2*$`、argon2i / argon2idのPHC）のように見える値は、変更されずに通過します。この保護機構がなければ、`User::find(id).await?.save().await?` は既存のハッシュをハッシュのハッシュへ再ハッシュ化してしまい、`Hash::check(plain, stored)` を壊し、既存のすべてのパスワードを無効にしてしまいます。

書き込み時にハッシュ以上のものを適用する必要がある場合 - 例えば、ハッシュ化の前に空白を正規化したり、空のパスワードを拒否したりする場合 - は、`AsHashed` を（下記の）`#[mutator]` のパターンと組にしてください。

## 実行時のキャストオーバーライド - `casts!` マクロ

`#[model(casts = { ... })]` で宣言されたキャストは静的です - それらは、そのモデルのすべての読み取りで発火します。単一のクエリに対して異なるキャストが必要な場合（デバッグツールが生の格納された形を欲しがる、エクスポートスクリプトが異なるJSON表現を欲しがる）は、`Builder::with_casts(...)` を使ってください:

```rust
use suprnova::{casts, AsDate, AsJson, User};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

`casts!` マクロは `HashMap<&'static str, Arc<dyn DynCast>>` を構築します。各エントリは `field_name = CastType` です。すべての組み込みキャストは `IntoDynCast` を実装しているため、型消去された `DynCast` の影は自動的です。実行時オーバーライドのマップは、連鎖したクエリの間だけ適用されます - モデルの静的なキャストパイプラインは変わりません。

この表面は控えめに使ってください。すべての読み取りに適用したいキャストにとって正しい場所は、モデルの属性です。実行時のオーバーライドは、一度限りのクエリのためのエスケープハッチです。

## アクセッサー - 実カラムからの仮想属性

アクセッサーとは、`#[accessor]` マクロで注釈された、モデル上の `impl` メソッドです。そのメソッドの名前を `#[model(appends = [...])]` に列挙すると、モデルの `to_json()` はそのメソッドを呼び出し、結果をそのキーの下に挿入します。

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    appends = ["full_name"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }
}
```

これで、`serde_json::to_value(&user)`（あるいは `user.to_json()`）は、次を含みます:

```json
{
  "id": 1,
  "first_name": "Alice",
  "last_name": "Xu",
  "full_name": "Alice Xu"
}
```

そのメソッドは直接呼び出すこともできます（`user.full_name()`）- `#[accessor]` マクロは主にマーカーであり、構造体レベルの `#[suprnova::model]` マクロが `to_json()` のディスパッチを配線できるようにするためのものです。あなた自身のコードからそれを呼び出すことに、コストはありません。

`appends` の中の各名前は、識別子として本物の `#[accessor]` メソッドと一致しなければなりません。タイプミス（メソッドが `full_name` であるときに `appends = ["fullName"]` とする）は、該当箇所を指し示すエラーメッセージとともに、コンパイル時に捕まります。

### `String` でない値を返す

アクセッサーは、あらゆる `Serialize` 型を返せます。マクロは、挿入の前に、`serde_json::to_value` を介して戻り値を変換します。そのため:

```rust
impl Post {
    #[accessor]
    pub fn word_count(&self) -> usize {
        self.body.split_whitespace().count()
    }
}
```

JSON出力の中では `"word_count": 42` として描画されます。

### 元のカラムを隠す

アクセッサーの値こそが消費者に見せるべきものであり、背後のカラムが雑音である場合は、`appends` を `hidden` と組にしてください:

```rust
#[model(
    table = "users",
    appends = ["full_name"],
    hidden = ["first_name", "last_name"],
)]
```

`hidden` は、名前を指定したカラムをシリアライズされた出力から取り除きます。`appends` は、その後でアクセッサーの値を挿入します。この順序は固定です - フィルタが先に実行され、アクセッサーの注入はその後に実行されます。完全な表面については、[Hidden、visible、そしてappends](eloquent.md#mass-assignment)を参照してください。

## ミューテータ - あなたの変換を経由してルーティングされる書き込み

ミューテータは、書き込み側の対応物です。フィールドの名前が `#[model(mutators = [...])]` に現れると、すべての一括代入の経路（`create` / `update`）は、フィールドへ直接代入する代わりに、値を `self.set_<field>(value)?` を経由してルーティングします。

```rust
use serde_json::Value;
use suprnova::{model, mutator, FrameworkError, Model};

#[model(
    table = "users",
    fillable = ["password"],
    mutators = ["password"],
)]
pub struct User {
    pub id: i64,
    pub password: String,
}

impl User {
    #[mutator]
    pub fn set_password(&mut self, value: Value) -> Result<(), FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            FrameworkError::validation("password", format!("{e}"))
        })?;
        // 正規化 + ハッシュ化。AsHashedは自分自身でハッシュ化を行うが、
        // ミューテータは、ポリシーも強制できる場所だ。
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        self.password = suprnova::hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

`set_password` は `serde_json::Value` を受け取ります。本体が、デシリアライズと変換を所有します - 構造体上のフィールドの型は `String` のままでよく、あなたのバリデーションは、カラムに触れる前に実行されます。返されたエラーは、`bad_request` として `create()` / `update()` を通じて伝播します。

直接のフィールド代入は、ミューテータをバイパスします:

```rust
user.password = "raw".to_string();  // set_passwordをスキップする
user.save().await?;                 // "raw"を保存する
```

これは、Laravelの `$user->password = ...` 対 `$user->fill(...)` の振る舞いと一致します。ミューテータを唯一の経路にしたい場合は、すべての書き込みを `attrs!` + `create` / `update` を経由させてください。

### ミューテータとキャストを組み合わせる

ミューテータとキャストは、同じフィールド上で共存できます。ミューテータは書き込みパス（`create` / `update` が呼ばれるとき）で実行され、キャストは読み取りパス（カラムがSELECTから実体化されるとき）で実行されます。よくあるパターンは、読み取り側のべき等性の保証には `AsHashed` を、書き込み側のバリデーションにはミューテータを使うことです - ミューテータがハッシュ化し、`AsHashed` はすでにハッシュ化された値を見て、そのまま通します。

## 自動管理されるタイムスタンプ

モデルが `created_at` と `updated_at` の両方のフィールド（`chrono::DateTime<chrono::Utc>` の型）を運んでいる場合、マクロは:

- `create()` の際に、両方を `Utc::now()` に設定します。
- すべての `save()` と `update(attrs)` の際に、`updated_at` を進めます。
- `impl Touchable for YourStruct` を発行します。そのため、他のどのカラムも変更せずに `updated_at` を進めるために、`.touch().await` を呼べます。

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model, Touchable};

#[model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// 他を変更せずにupdated_atを進める:
let post = Post::find_or_fail(1).await?;
post.touch().await?;
```

ストレージは、マクロがタイムスタンプのカラムに対して自動的に注入する `AsDateTime` キャストを使います。このキャストは、データベース固有のタイムスタンプ型を選ぶことを強いることなく、同じ `DateTime<Utc>` の値を、3つのSeaORMドライバー（SQLite、MySQL、PostgreSQL）すべてをまたいで往復させます。

### オプトアウトとカスタムのカラム名

`#[model(timestamps = false)]` は、自動管理を完全に無効化します - あなた自身がタイムスタンプを制御します。

`#[model(created_at = "creado_en", updated_at = "actualizado_en")]` は、自動管理を保ちつつ、カラムの名前を変えます。マクロは名前が変えられたフィールドを検出し、同じロジックをそれらに対して配線します。

構造体が2つのタイムスタンプフィールドのうち片方だけを持つ場合、マクロは `compile_error!` を発します - ほとんどの場合、サイレントに飲み込まれるのではなく、目立つ形で表面化させたいタイプミス（`craeted_at`）です。

### `without_touching` - タスクスコープの抑制

`updated_at` を進めずに行を更新したいことがあります - バックフィルの実行、タイプミスの修正、`updated_at` をキーとするキャッシュのTTLをリセットするべきではない内部同期の記録などです。`without_touching` で作業を包んでください:

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    for post in Post::query().get().await? {
        post.touch().await?;  // このスコープの内側では何もしない
    }
    Ok::<_, suprnova::FrameworkError>(())
}).await?;
```

このフラグは `tokio::task_local!` であるため、`tokio::spawn` の境界をまたいで漏れません - 他のタスク上の並行するリクエストは、それぞれ自分自身のスコープ（あるいはその不在）を尊重し続けます。これは、Laravelの `Model::withoutTouching(closure)` に対応するSuprnovaのものです。

### Suprnovaが異なる設計を選んだ理由

Laravelは、静的な `$timestamps = false` プロパティと、インスタンスのカウンタに支えられたグローバルな `Model::withoutTouching` の静的メソッドを使います。どちらのアプローチも、プロセスごとに1つのリクエストという分離を前提としています。Suprnovaは、1つのTokioランタイムの上で多くのリクエストを実行するため、プロセスグローバルなフラグは、1つのリクエストが他のリクエストのタイムスタンプをサイレントに抑制することを許してしまいます。`tokio::task_local!` のスコープはasyncを意識しています: それは、同じタスクの内側で `.await` の地点をまたいでfutureに従い、リクエストがどのように終わろうとも、futureがドロップするときにスコープの外に出ます。

## `Replicating` ライフサイクルイベント

16個のモデルのライフサイクルイベント（[オブザーバーとライフサイクルイベント](eloquent.md#observers-and-lifecycle-events)を参照）のうち、`Replicating` は、`replicate()` を介して既存の行を未保存のメモリ上のコピーへクローンするときに発火するものです:

```rust
let original = Post::find_or_fail(1).await?;
let mut copy = original.replicate().await?;  // 未保存
copy.title = format!("{} (copy)", original.title);
copy.save().await?;  // 新しいPKで、これで永続化される
```

`Replicating` イベントは、メモリ上のクローンが構築された**後**、しかしあなたがそれを変更する機会を持つ**前**に発火します。リスナーは `(&Self, Arc<Mutex<Self>>)` を受け取ります - 元のものと、`Mutex` の背後にある新しく構築された複製です。そのため、ユーザーがそれを目にする前に、リスナーから複製を変更できます:

```rust
use suprnova::{Listener, FrameworkError};

pub struct ResetReplicatedFlags;

#[async_trait::async_trait]
impl Listener<post::events::Replicating> for ResetReplicatedFlags {
    async fn handle(&self, event: &post::events::Replicating) -> Result<(), FrameworkError> {
        let mut replica = event.replica.lock().await;
        replica.published = false;       // コピーは非公開から始まる
        replica.view_count = 0;          // カウンタはリセットされる
        Ok(())
    }
}
```

リスナーが実行される時点で、複製のPKはすでにクリアされています - `replicate()` は、イベントを発火する前に `reset_primary_key()` を呼ぶため、誤って元のIDのままで再保存してしまうことはありません。タイムスタンプもリセットされます。`created_at` / `updated_at` は、他のどんな新しい行とも同じように、続く `save()` で発火します。

### `replicate_into<T>` - 型をまたぐ複製

複製が異なる型である場合（例えば `Post` → `Draft`）は、`replicate_into::<Draft>()` を使ってください。この経路では `Replicating` イベントは発火**しません**。イベントの構造体はソースの型ごとであり、`post::events::Replicating` に登録されたリスナーは、`Arc<Mutex<Draft>>` ではなく `Arc<Mutex<Post>>` を受け取ることになるからです。この型をまたぐ経路は、オブザーバーの干渉なしに新しいターゲットの型が欲しいときのためのものです。構築時にフックが欲しい場合は、ターゲットの型に普通の `Creating` リスナーを登録してください。

複製の表面の残り（`replicate_except`、複製のリレーションの扱い、null許容のPKに対する規則）については、[複製](eloquent.md#replication)を参照してください。

## 組み合わせる

この章のすべての表面を備えたモデルです:

```rust
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use suprnova::{
    accessor, hashing, model, mutator, AsBool, AsDateTime,
    AsDecimal, AsEncryptedObject, AsEnum, AsHashed, AsJson,
    AsOptionalDateTime, FrameworkError, Model,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardOnFile {
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[derive(Debug, Clone, Copy, strum::EnumString, strum::AsRefStr)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

#[model(
    table = "users",
    soft_deletes,
    appends = ["display_name"],
    hidden = ["password", "card"],
    fillable = ["name", "email", "password", "role", "credit"],
    mutators = ["password"],
    casts = {
        role = AsEnum<Role>,
        verified = AsBool,
        credit = AsDecimal<2>,
        card = AsEncryptedObject<CardOnFile>,
        metadata = AsJson<serde_json::Value>,
        password = AsHashed,
        last_login_at = AsOptionalDateTime,
    },
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: Role,
    pub verified: bool,
    pub credit: Decimal,
    pub card: CardOnFile,
    pub metadata: serde_json::Value,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // deleted_atはsoft_deletesによって自動的に注入される（AsOptionalDateTime）
}

impl User {
    #[accessor]
    pub fn display_name(&self) -> String {
        if self.name.is_empty() { self.email.clone() } else { self.name.clone() }
    }

    #[mutator]
    pub fn set_password(&mut self, value: Value) -> Result<(), FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            FrameworkError::validation("password", format!("{e}"))
        })?;
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        // ミューテータがハッシュ化する。AsHashedは、後続の保存では
        // すでにハッシュ化された値を見て、変更せずに通す。
        self.password = hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

この1つの宣言が、次を与えてくれます:

- ストレージ/ランタイムの境界を配線する、8つの型付けされたキャスト。
- 既存のカラムから `display_name` を合成するアクセッサー。
- パスワードを検証してハッシュ化するミューテータ。
- 自動管理される `created_at` / `updated_at`。
- 自動的に注入される `deleted_at` カラムを伴うソフトデリート。
- キーローテーションのサポートを伴う、暗号化されたcard-on-fileのストレージ。

すべてのキャストは、コンパイル時にチェックされます。デュアルAPIのクエリビルダー（[Eloquent - クエリビルダー](eloquent.md#query-builder--dual-api)を参照）は、型付けされたカラムに対して実行されます。Inertia / JSONへのシリアライゼーションは、hidden / appendsの規則を適用します。そして `User::find(id).await?` は、変換コードを一行も書くことなく、8つの `Cast::from_storage` の呼び出しを通じて、その行を実体化します。

## 次のステップ

- [Eloquent API](eloquent.md) - モデルの表面の残り: クエリビルダー、リレーションシップ、オブザーバー、ページネーション、トランザクションです。
- [暗号化](encryption.md) - 暗号化されたキャストが共有する `Crypt` ファサード、キーローテーションのプロトコル、そしてより広い暗号の表面です。
- [イベント](events.md) - `Replicating` と、他の15個のモデルのライフサイクルイベントの背後にあるディスパッチャーです。
- [認証](authentication.md) - `Authenticatable` トレイトと、`AsHashed` がパスワードのフローのどこに収まるかです。
- [バリデーション](validation.md) - `FrameworkError::validation` と、ミューテータがフィールドごとのエラーを表面化するために使うパターンです。

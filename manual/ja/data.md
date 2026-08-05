# データ オブジェクト

Suprnovaの `#[derive(Data)]` を使えば、受信するリクエストの形、送信するレスポンスの形、そしてTypeScriptのエクスポートを、**1つの構造体**で記述できます。

## クイックスタート

```rust
use suprnova::Data;
use suprnova::data::Field;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UserDto {
    pub id: i64,

    #[validate(email)]
    pub email: String,

    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub display_handle: String,

    pub bio: Field<String>,
}
```

`#[derive(Data)]` は次のものを生成します:
- `Serialize`（`#[data(input_only)]` のフィールドを除く）
- `Deserialize`（ペイロード中の `#[data(output_only)]` フィールドを拒否し、それらを `T::default()` にデフォルトする）
- デフォルトで `authorize: true` の `FormRequest` - ハンドラは、この型をエクストラクターとして直接受け取れる
- `IntoInertiaData`（`Inertia::data(component, dto)` のディスパッチ経路）
- `#[data(allow_include)]` の付いたフィールドのための `inventory::submit!` 登録

`#[validate(...)]` アトリビュートがフィールドの呼び出し箇所で見えたままになるよう、`#[derive(Validate)]` は別途追加してください。

## フィールドアトリビュート

| アトリビュート | 効果 |
|---|---|
| `#[data(input_only)]` | Deserializeでは受け付けられ、Serializeからは除かれる |
| `#[data(output_only)]` | Deserializeでは拒否され（422）、Serializeには含まれる |
| `#[data(allow_include)]` | フィールドが `?include=` の対象になる。**デフォルト拒否**: `foo` が許可リストに載っていない `?include=foo` リクエストは400を返す |
| `#[data(lazy)]` | フィールドが、リクエストのinclude集合に対して解決される `Prop` になる。`allow_include` として自動登録される |
| `#[data(lazy(inertia))]` | `lazy` と同じだが、Inertiaの部分リロードプロトコル用にタグ付けされる |
| `#[data(lazy(deferred))]` | Inertiaのディファードプロップ プロトコル用にタグ付けされる |
| `#[data(lazy(closure))]` | 初回訪問時には常に解決され、部分リロードではレイジーになる |
| `#[data(lazy(when_loaded))]` | 元のエンティティがそのリレーションをプリロードしている場合にだけ解決される |
| `#[data(from_route_param)]` | フィールドの値がパスのキャプチャから来る（例: `/users/{id}`）。デフォルトのキーはフィールド名。上書きするには `#[data(from_route_param("id"))]` を渡す |

## 構造体アトリビュート

| アトリビュート | 効果 |
|---|---|
| `#[data(auto_lazy)]` | `Prop` 型のすべてのフィールドが、暗黙的に `#[data(lazy)]` になる |
| `#[data(authorize = "path::to::fn")]` | 生成される `FormRequest::authorize` を、`fn(req: &Request) -> bool` というシグネチャを持つフリー関数へルーティングする。ボディパーサー、バリデータ、Precognitionのサポート、ルートパラメータの注入は、それでもderiveから来る |
| `#[data(allow_unknown_fields)]` | どの構造体フィールドにも一致しないペイロードのキーを受け入れる。デフォルトは**厳格**: 認識されないキーは、`serde::de::Error::unknown_field(..)` でデシリアライズに失敗し、`FormRequest` を通じて422として表面化する。permissive（許容的な振る舞い）を選ぶのは、前方互換なサードパーティのペイロードを読み取るレスポンスDTOに限る |

以前あった `#[data(custom_authorize)]` フラグ - `FormRequest` の実装全体を抑止し、ボディパース、バリデーション、Precognitionを手で再実装することを強いていたもの - は、なくなりました。それを使おうとすると、マクロは移行エラーを発します。代わりに `#[data(authorize = "fn")]` を使ってください。

## `Field<T>` - Absent / Null / Value

「ペイロードに存在しない」ことと「明示的なnull」を区別しなければならないPATCHエンドポイントのために:

```rust
use suprnova::data::Field;

match dto.bio {
    Field::Absent  => { /* このカラムには触れない */ },
    Field::Null    => { /* カラムをクリアする */ },
    Field::Value(text) => { /* textにセットする */ },
}
```

`Field::Absent`（デフォルト）は、呼び出し箇所で `#[serde(default, skip_serializing_if = "Field::is_absent")]` と組み合わせると、JSONから省略された状態と往復します。`skip_serializing_if` がなければ、`Absent` はJSONの `null` にシリアライズされます。

3値のDBアップサートには: `dto.bio.into_option_or_null() -> Option<Option<T>>` が `Absent → None`、`Null → Some(None)`、`Value(v) → Some(Some(v))` へと写します。「触れない」と「NULLにセットする」を、下流で区別する必要がある場合に使ってください。

> **注意点:** `Field<Option<T>>` は情報が失われます - `Value(None)` と `Null` はどちらもJSONの `null` としてシリアライズされ、デシリアライズすると `Null` に戻ります。nullableな内部型には、フラットな `Field<T>` を使い、「クリアする」という信号は `Null` に運ばせることを優先してください。

## `?include=` クエリ文字列

`IncludeMiddleware` は、リクエストのクエリ文字列を、リクエストごとの `RequestIncludeSet` へパースします:

- `?include=foo,bar` - レイジーフィールドの `foo` と `bar` を解決する。
- `?include[]=foo&include[]=bar` - 配列形式で、結果は同じ。
- `?exclude=`、`?only=`、`?except=` - Laravel-Data APIとのパリティ。

`X-Inertia-Partial-Data`（Inertiaの部分リロードヘッダー）との合成: 所有者にタグ付けされたレイジーフィールドについては、include集合とDTOごとの許可リストが**先に**実行されます。そのため、許可されていないフィールドへのリクエストは、partial-dataがそれをフィルタリングしていたはずの場合でも400を返します。partial-dataは、解決済みのプロップに対する最終的な「only」フィルタとして、**後で**適用されます。

`IncludeMiddleware` はグローバルに登録します - 通常、ミドルウェアスタックの中で、セッションと認可の間です:

```text
SessionMiddleware → IncludeMiddleware → AuthMiddleware → ハンドラ
```

### プログラムによる include/exclude/only/except の操作

`RequestIncludeSet` は、連鎖可能なビルダーを伴う、Laravel-Dataの `IncludeableData` 契約を反映しています。ハンドラ、テスト、ミドルウェアは、公開フィールドを直接触ることなく、集合を構築したり上書きしたりできます:

```rust
use suprnova::data::RequestIncludeSet;

let set = RequestIncludeSet::default()
    .include(["author", "comments"])
    .exclude(["password"])
    .only(["id", "name"])
    .except(["secret"]);

assert!(set.is_visible("name"));   // `only` にあり、`except` にはない
assert!(!set.is_visible("secret"));// `except` が常に勝つ
assert!(set.includes("author"));   // `author` リレーションへのリクエスト
```

| メソッド | 効果 | Laravelでの対応 |
|---|---|---|
| `.include(fields)` | includeリストに追加する（解決すべきレイジーフィールド） | `Data::include(...$fields)` |
| `.exclude(fields)` | excludeリストに追加する（落とすフィールド） | `Data::exclude(...$fields)` |
| `.only(fields)` | `only` の許可リストを初期化または拡張する | `Data::only(...$fields)` |
| `.except(fields)` | exceptリストに追加する（常に落とす） | `Data::except(...$fields)` |
| `.include_when(cond, fields)` | `cond == true` のときだけ追加する | `Data::includeWhen($field, $condition)` |
| `.exclude_when(cond, fields)` | `cond == true` のときだけ追加する | `Data::excludeWhen($field, $condition)` |
| `.only_when(cond, fields)` | `cond == true` のときだけ `only` を拡張する | `Data::onlyWhen($field, $condition)` |
| `.except_when(cond, fields)` | `cond == true` のときだけ追加する | `Data::exceptWhen($field, $condition)` |
| `.merge(other)` | 2つの集合を統合する（その場でのレイヤー化された上書き） | PHPでの手動の `array_merge` |
| `.includes(field)` | `field`（または `field.path`）がincludeリストにあるか？ | `relationLoaded()` に類する |
| `.is_excluded(field)` | `field` がexcludeリストにあるか？ | exclude部分集合を読む |
| `.is_excepted(field)` | `field` がexceptリストにあるか？ | except部分集合を読む |
| `.is_only_listed(field)` | `field` は `only` によって許可されているか（あるいは `only` が未設定か）？ | only部分集合を読む |
| `.is_visible(field)` | Laravelの完全な解決順序: except → exclude → only | `resolveResource` の判定 |

ビルダーはあらゆる `IntoIterator<Item = impl Into<String>>` を受け取るため、配列、vec、`&str`/`String` のスライスはすべて機能します。文字列はトリムされ、空のエントリは落とされます（`from_query` と一致する挙動です）。

どのリストにおいても、ドットパスは裸の名前で調べられたときにルートのセグメントとマッチします - `include=["author.posts"]` は `set.includes("author") == true` を報告し、Laravel-Dataのパス解決と一致します。ネストした `posts` セグメントは、JSON:APIの複合ドキュメントのために `IncludeTree::from_include_set` によって消費されます。

### ハンドラ側でのオーバーライド: `with_include_overrides`

リクエストのクエリ文字列がすでに宣言しているものの上に、プログラムによる上書きを重ねるには（リクエストの集合を失うことなく）、`with_include_overrides` を使ってください:

```rust
use suprnova::data::with_include_overrides;

async fn show_album(req: Request, user: User) -> Response {
    with_include_overrides(
        |set| set
            .include_when(user.is_admin(), ["audit_log"])
            .exclude_when(!user.is_admin(), ["price_cost"]),
        async move {
            // このスコープの内側では、レイジープロップのリゾルバとJSON:APIの
            // includeリゾルバが、統合された集合を見る。
            Inertia::data("Album/Show", album_dto).into_response()
        },
    ).await
}
```

このクロージャは、現在バインドされている集合の複製に対して実行されます（ミドルウェアが何もバインドしていない場合は、空のデフォルトに対して）。futureが完了すると、元の集合が復元されます - これはスコープに閉じたオーバーライドであり、変更ではありません。

テストでは、周囲の状態を何も継承せずに新しい集合をインストールする `scope_include_set(set, future)` を優先してください。

## ジェネリック構造体

```rust
use serde::{Serialize, Deserialize};

#[derive(suprnova::Data)]
pub struct Paginated<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    pub items: Vec<T>,
    pub total: usize,

    #[data(allow_include)]
    pub meta: Option<serde_json::Value>,
}
```

TypeScriptのエクストラクターは `export interface Paginated<T>` を出力するため、フロントエンドのコードは、そのジェネリックをインスタンス化をまたいで再利用できます。

`?include=` の許可リストは、完全修飾された型のパス（`concat!(module_path!(), "::", stringify!(Paginated))`）をキーにしており、型パラメータのインスタンス化はキーにしていません。同じモジュール内で宣言された `Paginated<UserDto>` と `Paginated<ArticleDto>` は、1つの許可リストを共有します - `allow_include` はフィールドに名前を付けるものであり、フィールド名は型パラメータに依存しないからです。異なるモジュールにある、`Paginated` という名前を持つ2つの異なるDTOは、それぞれ自分自身の許可リストを持ちます。両者のキーは衝突しません。

注: `FormRequest` はジェネリック構造体に対しては抑止されます。そのトレイト境界（`DeserializeOwned + Validate + Send`）は、具体的な型パラメータを知らなければ検証できないからです。リクエストからジェネリックなData構造体を抽出する必要がある場合は、自分自身の実装を用意してください。

## ルートパラメータのフィールド注入

```rust
use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UpdateUser {
    #[data(from_route_param("id"))]
    pub id: i64,

    #[validate(length(min = 1))]
    pub name: String,
}
```

ボディが `{"name": "Ada"}` の `PATCH /users/{id}` では、ルートがキャプチャした `id` が、バリデーション済みのペイロードへマージされます。**パスは、ボディが与える値に常に勝ちます**（ボディの改ざんによるIDORを防ぎます）。

裸の `#[data(from_route_param)]` は、フィールド名にデフォルトします。マクロはコンパイル時にフィールドの最後のパスセグメントを分類し、マッチするパーサーへディスパッチします。認識されるのは、以下に挙げる正確な名前だけです。それ以外のすべて（`i8`/`i16`/`isize`、`Uuid`、`DateTime`、独自のニュータイプを含みます）は `pass_string` へ落ち込み、フィールド自身の `Deserialize` に処理を委ねます。

| フィールド型 | パーサー |
|---|---|
| `i64` | `parse_i64` |
| `u64` | `parse_u64` |
| `i32` | `parse_i32` |
| `u32` | `parse_u32` |
| `i128` | `parse_i128`（バリデーションしてから生の文字列を通す。フィールドの `Deserialize` がパースする） |
| `u128` | `parse_u128`（同じ文字列パススルーのパターン） |
| `f64` | `parse_f64`（非有限値を拒否する） |
| `f32` | `parse_f32`（非有限値を拒否する） |
| `bool` | `parse_bool`（`"true"` / `"false"` だけを受け付ける） |
| それ以外すべて | `pass_string` - 生の文字列がフィールド自身の `Deserialize` に手渡される |
| 上記いずれかの `Option<T>` または `Field<T>` | `T` と同じパーサー。ルートパラメータが欠けていれば、フィールドは値なしのままになる |

## レイジー プロップ

```rust
use suprnova::Data;
use suprnova::inertia::Prop;

#[derive(Data)]
#[data(auto_lazy)]
pub struct AlbumDto {
    pub id: i64,
    pub songs: Prop,    // ?include=songs として自動登録される
    pub artist: Prop,   // ?include=artist として自動登録される
}
```

フィールドごとに明示する場合:

```rust
#[derive(Data)]
pub struct AlbumDto {
    pub id: i64,

    #[data(lazy(inertia))]
    pub songs: Prop,

    #[data(lazy(deferred))]
    pub lyrics: Prop,

    #[data(lazy(closure))]
    pub artist: Prop,
}
```

描画するには `Inertia::data(component, dto)` を使ってください - deriveは、include集合と許可リストを参照する `IntoInertiaData` の実装を生成します:

```rust
return Inertia::data("Album/Show", album_dto);
```

注: レイジーフィールドを持つ構造体は、`Serialize`、`Deserialize`、`FormRequest` を抑止します。`Prop` がそれらを実装していないからです。1つのエンドポイントが、受信のパースと送信のレイジー出力の両方を必要とする場合は、2つのDTOを使ってください: 1つは受信用（素の `#[derive(Data, Validate)]`）、もう1つは送信用（レイジーフィールドを持つ `#[derive(Data)]`）です。

## `when_loaded!` - リレーションのロード状態に応じた条件付きレイジー

Laravel-Dataの `#[AutoWhenLoadedLazy]` を反映したものです。そのリレーションがプリロードされていたかどうかは、ユーザーの `From<Entity>` の実装が決めます:

```rust
use suprnova::data::{when_loaded, IsRelationLoaded};

impl From<&AlbumEntity> for AlbumDto {
    fn from(album: &AlbumEntity) -> Self {
        Self {
            id: album.id,
            songs: when_loaded!(album, "songs", || async {
                serde_json::json!(album.songs_relation()
                    .iter()
                    .map(SongDto::from)
                    .collect::<Vec<_>>())
            }),
            artist: Prop::eager(serde_json::json!(album.artist_name())),
            lyrics: Prop::lazy(|| async { /* ... */ }),
        }
    }
}
```

エンティティが、名前を指定したリレーションをプリロードしていない場合（`IsRelationLoaded::is_relation_loaded` によります）、`when_loaded!` は `Prop::EagerNone` を返し、そのフィールドはレスポンスから欠落します。

SeaORMのエンティティには、自身のロード済みリレーションの状態を参照する、独自の `IsRelationLoaded` の実装が必要です - フレームワークが提供する包括的な実装は存在しません。SeaORMの `ModelTrait` は、インスタンスごとのリレーションロード状態を運ばないからです（プリロードされたリレーションはクエリの結果に存在し、モデルの構造体自体には存在しません）。

## TypeScript エクスポート

`suprnova generate-types` は、すべての `#[derive(Data)]`（そして従来の `#[derive(InertiaProps)]`）構造体について、TypeScriptの定義を出力します。振る舞いは次のとおりです:

- `Field<T>` → `field?: T | null`
- `Prop` → `field?: T`（レイジーな、値が欠けているかもしれないというセマンティクス。それを運ぶのは `?` であり、型自体は素のまま）
- `#[data(input_only)]` → 出力の型から除外される
- `#[data(output_only)]` → 入力の型から除外される
- ジェネリック構造体 → TypeScriptのジェネリックインターフェース（`export interface Paginated<T>`）
- いずれかのフィールドが `input_only` / `output_only` / `lazy` を持つ場合、2つのインターフェースが出力される: `<Name>`（出力用）と `<Name>Input`（入力用）

生成される型は、Rust専用の型を決して漏らしません（`Prop<...>` は、出力される `.d.ts` には現れません）。

## スキャフォルド

```bash
suprnova make:inertia UserDto --data
```

従来の `#[derive(InertiaProps)]` テンプレートの代わりに、`#[derive(Data, Validate)]` の骨格を出力します。

## 次のステップ

- [バリデーション](validation.md) - `#[derive(Validate)]`、非同期のバリデータ、そして `FormRequest` がそれらをどう呼び出すか
- [リクエスト](requests.md) - `FormRequest` が差し込まれる、リクエストエクストラクターの表面
- [Inertia レスポンス](frontend-inertia-responses.md) - `Inertia::data` の経路、そしてレイジープロップがどのように部分リロードの対象になるか
- [JSON:API リソース](eloquent-resources.md) - JSON:API出力のための `#[derive(Resource)]`（シリアライズ専用のペイロードのための、`Data` と対になる概念）
- [エラー モデル](error-model.md) - `unknown_field` の拒否がどのように422になり、`FormRequest` の失敗がどのように `ValidationErrors` として返っていくか

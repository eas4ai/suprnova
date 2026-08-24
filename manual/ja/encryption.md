# 暗号化

Suprnovaは、アプリケーションレベルの暗号化を、`Crypt` という名のプロセス全体のファサードとして出荷します。これは、文字列や任意の `Serialize` 値を、あなたの `APP_KEY` をキーとしたAES-256-GCMの下で暗号化します。完全には信頼していないストレージに機密なものを入れる必要があり、後でそれを損なわれていない形で読み戻す必要があるとき - カラム、クッキー、ページネーションのカーソル - に、これに手を伸ばしてください。

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cast, "ssn-123-45-6789")?;
let plain = Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
assert_eq!(plain, "ssn-123-45-6789");
```

フレームワーク自身も、暗号化されたクッキー、暗号化されたページネーションカーソル、2FAのシークレット、リカバリーコード、そして `AsEncrypted*` のEloquentキャストのために `Crypt` を使っています。`APP_KEY` が設定されていれば、同じファサードは、余分な配線なしにあなたのコードからも使えます（[configuration.md](configuration.md#the-env-file)を参照）。

## 通信上の形式

`encrypt_string` と `encrypt` はどちらも、`nonce || ciphertext_with_tag` に対するURL-safeなbase64（パディングなし）を返します:

```
base64url( [12バイトのランダムなノンス] || [暗号文] || [16バイトのGCMタグ] )
```

呼び出しごとに、OSのRNGから新しい12バイトのノンスを取得します。そのため、同じキーの下で同じ平文を2回暗号化しても、異なる暗号文が生成されます。平文そのものを超える長さの情報を漏らすパディングオラクルはありません。

この出力は、それ以上のエンコードなしに、URLのクエリ文字列、JSONボディ、ヘッダー、クッキーに入れても安全です。有効な最小の暗号化文字列は28バイト（ノンス12 + タグ16）です - それより短いものは事前に拒否されます。

## `APP_KEY` - 唯一重要なシークレット

Suprnovaは、`APP_KEY` という環境変数から、単一の32バイトの対称キーを読み取ります。期待される形式はURL-safeなbase64、パディングなしで、デコードするとちょうど32バイト（base64で43文字）になるものです:

```env
APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

CLIでキーを1つ生成してください:

```bash
suprnova key:generate
# Generated a new APP_KEY (AES-256, base64 URL-safe, no padding):
#
#     hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
#
# Add it to your .env (or your secrets manager):
#
#     APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

あるいは、そのまま環境へパイプしてください:

```bash
echo "APP_KEY=$(suprnova key:generate --show)" >> .env
```

### 起動時の検証 - フェイルクローズ

`Server::from_config` は、最初の起動だけでなく**起動するたびに** `APP_KEY` を検証します。ルールは次のとおりです:

| 環境 | `APP_KEY` が未設定 | `APP_KEY` の形式が不正 |
|---|---|---|
| `local`、`development`、`testing` | 一時的なキーを生成し、ログに警告 | ハードエラー - 起動に失敗 |
| `staging`、`production`、それ以外すべて | ハードエラー - 起動に失敗 | ハードエラー - 起動に失敗 |

形式が不正なキーは、`local` であっても**常に**ハードエラーです - タイプミスを覆い隠すよりは、起動を失敗させるほうがましです。フレームワークが認識しない `Custom` な環境の値（例えば `APP_ENV=k8s`）は、本番相当として扱われます: `APP_KEY` がなければ、起動もありません。

診断メッセージは、修正すべき箇所を指し示します:

```
APP_KEY is required when APP_ENV=production. Generate one with
`suprnova key:generate` and set it in your environment (e.g. .env
or your secrets manager). Suprnova refuses to boot without an
encryption key outside of local/development/testing because session
cookies and pagination cursors would otherwise be unsigned and
forgeable.
```

## `CryptPurpose` - AADによるドメイン分離

あらゆる `Crypt::*` の呼び出しは `CryptPurpose` を取ります。このバリアントは、AES-GCMの認証タグに関連データ（AAD）として結び付けられる、安定したバイトラベルへと対応付けられます:

```rust
pub enum CryptPurpose {
    Cookie,            // suprnova:cookie:v1
    Cursor,            // suprnova:cursor:v1
    TwoFactorSecret,   // suprnova:2fa:secret:v1
    TwoFactorRecovery, // suprnova:2fa:recovery:v1
    Cast,              // suprnova:cast:v1
}
```

このラベルは暗号化文字列の中には**保存されません**。GCMは、AADを暗号文には含めずに認証タグへ混ぜ込みます。そのため:

- 通信上の形式は変わりません - 依然として `base64(nonce || ciphertext || tag)` です。
- `CryptPurpose::Cookie` の下で作られた暗号化文字列は、異なる用途を与えるあらゆる復号呼び出しによって**拒否されます**。GCMのタグチェックは、復号後のパース処理が走るよりも前に失敗します。
- 新しい表面を追加すること（将来のキューのペイロード暗号化、暗号化されたファイルヘッダーなど）は、新しいバリアントを追加することを意味します - 通信上の形式を変えることではありません。

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cookie, "session-id")?;

// 同じキー、同じ暗号化文字列、異なる用途 - 失敗します。
let result = Crypt::decrypt_string(CryptPurpose::Cursor, &wire);
assert!(result.is_err());

// 同じ用途 - 成功します。
let plain = Crypt::decrypt_string(CryptPurpose::Cookie, &wire)?;
```

### Suprnovaが異なる設計を選んだ理由

Laravelの `Crypt::encryptString` は用途を取りません。単一の `APP_KEY` が、クッキー、署名付きURL、署名付きの期限トークン、そして `Crypt::encrypt` へのあらゆるユーザー呼び出しをまたいで再利用され、暗号レイヤーでのドメイン分離は一切ありません。2つの表面が偶然同じ平文の形の暗号文を受け入れてしまう場合、一方の表面のために発行された値が、もう一方へリプレイされてしまう可能性があります。

Suprnovaも同じ理由で同じ `APP_KEY` を再利用します - 運用者が管理するシークレットは1つのままです - しかし、各表面をそれぞれ独自のAADラベルに結び付けます。表面をまたぐ暗号文のリプレイは、パース処理が走るよりも前に、GCMのタグチェックで拒否されます。呼び出し側のコストは、enumの引数が1つ増えるだけです。得られるものは、通信上の形式だけでは決して破れない性質です。

各ラベルの `:v1` というサフィックスは、将来の表面単位のローテーションのために予約されています: `suprnova:cookie:v1` を `suprnova:cookie:v2` へ引き上げると、古いクッキーの暗号文**だけ**が無効になります - カーソル、2FAのシークレット、キャストのカラムには手を付けません。

## クッキー名に束縛されたAAD（v2）

暗号化クッキーは、呼び出し側がクッキーの論理名を知っている場合、第二世代のAADを使用します。`Cookie::encrypted("suprnova_session", value)` はGCMタグへ `suprnova:cookie:v2:suprnova_session` を束縛し、`Cookie::read_encrypted_for("suprnova_session", wire)` は復号時に同じコンテキストを渡します:

```rust
use suprnova::Cookie;

let cookie = Cookie::encrypted("suprnova_session", "session-id")?;
let wire = cookie.value().to_string();
assert_eq!(
    Cookie::read_encrypted_for("suprnova_session", &wire)?,
    "session-id"
);
assert!(Cookie::read_encrypted_for("other_cookie", &wire).is_err());
```

束縛される名前はレンダリング後の名前ではなく、論理名です。したがって、後から付く `__Host-` または `__Secure-` のワイヤ名プレフィックスはAADを変更せず、ユーザーをログアウトさせません。プレフィックスはブラウザとヘッダーの関心事であり、クッキー名は暗号学的ドメインです。

### 互換性ウィンドウ

ワイヤ形式は変更されず、バージョンも持ちません。依然としてnonce、暗号文、認証タグだけを運びます。リーダーが分岐を選べるバージョンバイトはありません。`decrypt_string_for` は鍵ローテーションと同じ形でブラインド試行復号を使用します。まずコンテキスト付きv2 AADを鍵リング全体で試し、次にコンテキストなしv1 AADを鍵リング全体で試します。これにより、`APP_KEY` ローテーションも進行中の間、名前束縛以前に書かれたクッキーを読み取れます。

このウィンドウは、その全期間にわたり古いリプレイ弱点を保持します。コンテキストなしのフォールバックが存在する間、あるクッキースロットのv1クッキーは別のスロットへリプレイできます。名前束縛の利点は、そのフォールバックが1.4.0で削除された時点から始まります。フォールバックを自動的に廃止するものはありません。`Crypt::encrypt_string(CryptPurpose::Cookie, ...)` は引き続きv1を発行し、コンテキストなしのエントリポイントは1.4.0での削除予定として置き換えられています。その期限までに、クッキー書き込みを `Cookie::encrypted` へ、読み取りを `read_encrypted_for` へ移してください。

ウィンドウ中には測定可能なコストがあります。失敗したクッキー復号は、リング全体を2回試行します。セッションミドルウェアは、セッションクッキーとremember-meクッキーが両方あると、リクエストごとに2回の暗号化読み取りを行うため、期限切れのrememberクッキーを持つ匿名リクエストは、`N` を以前の鍵の数とすると `2 × (1 + N)` を2回支払います。

### `DecryptOrigin` を読む

`Crypt::decrypt_string_for_inner` は、互いに独立した2軸を持つ `DecryptOrigin` を返します:

- `origin.key = KeyOrigin::Previous(index)` は、値が依然として `APP_KEY_PREVIOUS[index]` に依存することを意味します。値を現在の鍵の下で再暗号化し、ローテーションの末尾がなくなった後にのみその以前の鍵を取り除いてください。
- `origin.aad = AadVersion::Legacy` は、値がコンテキストなしのv1フォールバックを使用したことを意味します。クッキーでは、名前束縛APIを通じて再発行してください。フォールバックは1.4.0で削除予定です。

両方の軸は同時に古くなり得ます。公開リーダーは、平文や暗号文を含めずに対応する警告をログします。鍵の警告はローテーションのクリーンアップ作業、AADの警告はマイグレーション作業として扱ってください。一方の軸に対するマッチングが、もう一方を隠してはなりません。

## 2組のencrypt / decryptペア

2つの用途に対して、2つの形があります。

### 文字列 - `encrypt_string` / `decrypt_string`

UTF-8の文字列のためのものです:

```rust
use suprnova::{Crypt, CryptPurpose};

let wire: String =
    Crypt::encrypt_string(CryptPurpose::Cast, "alice@example.com")?;

let plain: String =
    Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
```

復号の経路は `String` を返します - 非UTF-8のバイト列（通常のencryptの実行では生成され得ませんが、破損した、あるいは攻撃者が与えた暗号化文字列であれば生成しうるものです）は、明確な `FrameworkError::Internal` として表面化します。

### `Serialize` を実装する任意の値 - `encrypt` / `decrypt`

構造化された値に対しては、1回の呼び出しでJSONエンコードしてから暗号化します:

```rust
use serde::{Serialize, Deserialize};
use suprnova::{Crypt, CryptPurpose};

#[derive(Serialize, Deserialize)]
struct Secret {
    api_key: String,
    last_rotated_at: chrono::DateTime<chrono::Utc>,
}

let value = Secret {
    api_key: "sk_live_…".into(),
    last_rotated_at: chrono::Utc::now(),
};

let wire = Crypt::encrypt(CryptPurpose::Cast, &value)?;
let round_trip: Secret = Crypt::decrypt(CryptPurpose::Cast, &wire)?;
```

通信上の形式は同じです - `nonce || ciphertext || tag` に対するbase64 - 唯一の違いは、平文が文字列のUTF-8ではなく `value` の `serde_json` バイト列であることです。あらゆるレコードの形に対してこれを使ってください: 設定のブロブ、セッションのペイロード、キューの引数タプルなど。

### `appears_encrypted` - 形のチェックであり、改ざんチェックではない

出力側の経路で、すでに暗号化済みの値をスキップする必要があるミドルウェアのために（Laravelの `EncryptCookies` の振る舞いと一致します）、`Crypt::appears_encrypted` は安価なヒューリスティックチェックを行います:

```rust
if Crypt::appears_encrypted(cookie_value) {
    // そのまま通す - すでにラップ済み
} else {
    // 送信する前に暗号化する
}
```

これは、入力がURL-safeなbase64としてデコードでき、デコード後の長さが少なくとも28バイト（ノンス + タグ）ある場合に `true` を返します。AES-GCMを一切呼び出さないため、正しい形のランダムなバイト列と、有効な暗号文とを見分けることは**できません**。認証が必要な呼び出し側は、`decrypt_string` / `decrypt` を呼び出し、エラーを処理しなければなりません。

## キーローテーション - キーリング

Suprnovaは、キーの*リング*を通じて、ダウンタイムゼロのローテーションをサポートします: 1つの現在のキー（新しい暗号化すべてに使われます）と、順序付けられた過去のキーのリスト（復号時にフォールバックとして試されます）です。すべてのカラムを一斉に再暗号化することなく、`APP_KEY` をローテーションできます。

`APP_KEY_PREVIOUS` に、古いものから新しいものの順で、コンマ区切りのbase64キーのリストを設定してください:

```env
APP_KEY=<new key>
APP_KEY_PREVIOUS=<old key>
# あるいは複数段のローテーションの場合（古い → 新しい）:
APP_KEY_PREVIOUS=<oldest>,<middle>,<previous>
```

`APP_KEY_PREVIOUS` はSuprnovaの正式な名前です。`APP_PREVIOUS_KEYS` はLaravel互換の別名として受け入れられます。両方の変数が設定されている場合、`APP_KEY_PREVIOUS` が優先されます。トリム後の値が異なる場合、起動時にログに警告を出し、`APP_PREVIOUS_KEYS` は無視されます。

暗号化は**常に**現在のキーを使います。復号はまず現在のキーを試し、それが失敗すると、過去のキーを順番に試します。過去のキーで当たった場合、`Crypt` は `tracing::warn!` を発します:

```
WARN previous_index=0 Crypt decrypted a value with APP_KEY_PREVIOUS[0];
re-encrypt (load + save) this row under the current APP_KEY and remove
the corresponding APP_KEY_PREVIOUS entry once the rotation completes.
```

このログ行は、平文と暗号文のどちらも意図的に含みません - ローテーションが起きたという事実と、実行可能なヒントだけが伝わります。`APP_KEY_PREVIOUS` でログを検索する運用者は、まだ古いキーに依存しているすべてのカラムに行き着きます。

### 上限 - `MAX_PREVIOUS_KEYS = 8`

`APP_KEY_PREVIOUS` は8エントリまでという上限があります。現実的なローテーションの連鎖は1から3エントリです（進行中のローテーションが1つ、運用者が片付けていない停滞した以前のローテーションがおそらく1つ） - 8であれば十分な余裕があります。上限を超えると、起動は件数と上限の両方を示す診断とともに**はっきりと失敗します**:

```
APP_KEY_PREVIOUS holds 12 keys; the maximum is 8. A realistic
rotation chain is 1-3 entries - a longer list is almost always a
config-templating accident. Trim the list to the keys still needed
for in-flight rotation; once a re-encrypt job has migrated every
row off an old key, drop that entry.
```

サイレントな切り詰めは、運用者がまだ依存しているかもしれないキーを落としてしまい、診断なしでカラムを復号不能にしてしまいます。このハードな上限は意図的なものです。

空のエントリは許容されます: `APP_KEY_PREVIOUS=,,,old1,,,old2,,,` は、2つの実在するキーとしてパースされます。形式が不正なエントリ（タイプミス、間違った長さ、壊れたbase64）はハードエラーです - 半端にローテーションされたシークレットは、フォールバックをサイレントに落とすのではなく、起動を失敗させます。

### ローテーションの手順

```bash
# 1. 新しいキーを発行します。
NEW=$(suprnova key:generate --show)

# 2. 現在のキーを APP_KEY_PREVIOUS へ移し、新しいキーをインストールします。
#    .env かシークレットマネージャーを編集してください:
#
#      APP_KEY_PREVIOUS=<old_value_of_APP_KEY>
#      APP_KEY=<NEW>

# 3. デプロイします。新しい書き込みは新しいキーを使い、既存の行は
#    過去のキーのフォールバックを通じて復号され続けます。ログは、
#    まだ古いキーを使っているカラムを特定します。

# 4. 再暗号化のパスを実行します。暗号化されたキャストを持つ各モデルについて:
#
#      User::query().chunk(500, |batch| async {
#          for mut row in batch { row.save().await?; }
#          Ok(())
#      }).await?;
#
#    `Cast::to_storage` は常に現在のキーを使うため、何もしない
#    load-then-saveが行を移行させます。

# 5. 警告がログに現れなくなったら、APP_KEY_PREVIOUS を削除して
#    再度デプロイします。
```

この手順全体はオンラインで行えます - 新しいリクエストが失敗する時間帯は、一切存在しません。

### リングを観測する

運用者向けのダッシュボードやヘルスチェックのためのものです:

```rust
use suprnova::Crypt;

if Crypt::has_previous_keys() {
    let n = Crypt::previous_key_count();
    tracing::info!(previous_keys = n, "APP_KEY rotation in progress");
}
```

キーのバイト列そのものは、公開APIから決してアクセスできません。`EncryptionKey` の `Debug` 実装は `"[REDACTED]"` を出力し、クレートの外部に生のキーを表面化させるアクセサは存在しません。

## Eloquent統合 - `AsEncrypted*` キャスト

アプリケーションレベルの暗号化が最も役立つのは、カラムの境界です。`AsEncrypted*` というキャストのファミリーは `Crypt::encrypt_string` をラップするため、あなたのモデルのフィールドは、実行時には型付きの平文のままで、保存時には暗号文になります:

```rust
use suprnova::{model, Model};
use suprnova::eloquent::casts::{
    AsEncrypted, AsEncryptedArray, AsEncryptedObject, AsEncryptedCollection,
};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct ApiKey {
    pub provider: String,
    pub secret: String,
}

#[model(table = "users", casts = {
    api_token     = AsEncrypted,
    api_keys      = AsEncryptedArray<ApiKey>,
    billing       = AsEncryptedObject<BillingDetails>,
    ssh_keys      = AsEncryptedCollection<String>,
})]
pub struct User {
    pub id: i64,
    pub api_token: String,
    pub api_keys: Vec<ApiKey>,
    pub billing: BillingDetails,
    pub ssh_keys: suprnova::eloquent::Collection<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

| キャスト | 実行時の型 | 保存時の形 |
|---|---|---|
| `AsEncrypted` | `String` | 暗号化文字列 |
| `AsEncryptedArray<T>` | `Vec<T>` | JSON → 暗号化文字列 |
| `AsEncryptedObject<T>` | `T` | JSON → 暗号化文字列 |
| `AsEncryptedCollection<T>` | `Collection<T>` | JSON → 暗号化文字列 |

4つすべてが `CryptPurpose::Cast` を経由します。暗号化されたキャストによって発行された暗号化文字列は、それをクッキーやカーソルとして復号しようとするあらゆるコードによって拒否されます - `APP_KEY` は同じであっても、AADラベルが異なるからです。

キャストの完全な表面、失敗モードの表、そして再暗号化のレシピについては、[eloquent.md](eloquent.md)を参照してください。暗号化の仕組みは、上記のファサードと同じです - このキャストは、ストレージの境界で `Crypt::encrypt_string(CryptPurpose::Cast, …)` を実行するシュガーです。

### 暗号化 対 ハッシング - 正しい道具を選ぶ

`AsEncrypted` は**可逆的**です。平文は `APP_KEY` があれば復元できます。あなたのアプリケーションが読み戻す必要のあるデータに使ってください: 設定ページに表示するAPIトークン、上流のサービスへ転送するサードパーティのシークレット、注文を発送する先の住所などです。

あなたのアプリケーションが*検証*することしか必要としないデータ - パスワード、受信したトークンと比較するAPIキーのプレフィックスなど - には、代わりにハッシュを使ってください。ハッシュは一方向です: `APP_KEY` が漏洩したとしても、漏れる平文はそもそも存在しません。Bcrypt / Argon2idのファサードと `AsHashed` キャストについては[hashing.md](hashing.md)を参照してください。

## フレームワークの内部で、他に `Crypt` が使われている場所

これらをオプトインするために、あなたが何かをする必要はありません - `APP_KEY` が設定されれば、自動的に配線されます。

- **暗号化されたクッキー** - `Cookie::encrypted(...)` / `Cookie::read_encrypted(...)` は `CryptPurpose::Cookie` を使います。セッションクッキー、remember-meクッキー、メンテナンスモードのバイパスクッキーは、すべてこれに乗っています。[responses.md](responses.md)と[session.md](session.md)を参照してください。
- **カーソルページネーション** - `CursorPaginator` は、`CryptPurpose::Cursor` の下でカーソルをエンコードするため、通信上の `?cursor=…` の値は、表面をまたいで偽造されたりリプレイされたりすることがありません。[eloquent.md](eloquent.md#cursor-pagination)を参照してください。
- **2FAのシークレット** - `two_factor_authentications.secret` にある暗号化されたbase32のTOTPシークレットは `CryptPurpose::TwoFactorSecret` を使い、リカバリーコードは `CryptPurpose::TwoFactorRecovery` を使います。用途を分けることで、同じ行の中でカラムをまたいだ暗号文のリプレイを防ぎます。[auth-flows.md](auth-flows.md)を参照してください。
- **HMAC由来の署名** - 署名付きURLとパスワードリセットのトークンは、`APP_KEY` の下で暗号化するのではなく、そこからHMACキーを導出します。生のキーのバイト列はエクスポートされません - 導出処理はフレームワークの内部にあります。[routing.md](routing.md#signed-urls)を参照してください。

## `Crypt` を使ったテスト

`Crypt` ファサードは `OnceLock` に支えられているため、テストバイナリの中で最初にインストールしたものが有効になります。テスト用のヘルパーが、ボイラープレートを処理してくれます:

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn encrypts_and_round_trips() {
    install_test_encryption_key(); // べき等 - どのテストからも安全に呼べます

    let wire = suprnova::Crypt::encrypt_string(
        suprnova::CryptPurpose::Cast,
        "hello",
    ).unwrap();

    let plain = suprnova::Crypt::decrypt_string(
        suprnova::CryptPurpose::Cast,
        &wire,
    ).unwrap();

    assert_eq!(plain, "hello");
}
```

テスト用キーは決定的であるため、テストは安定したフィクスチャを復号し、既知のキーに対してローテーションを試せます。暗号文文字列を呼び出し間または実行間で等しいか比較してはなりません。暗号化ごとに依然として新しいランダムなノンスが使われます。

ローテーションのテストのためには、キーリングを直接インストールし、`_test_encrypt_with` で過去の暗号文を発行してください:

```rust
use suprnova::testing::install_test_encryption_keyring;
use suprnova::EncryptionKey;

let current = EncryptionKey::generate();
let old = EncryptionKey::generate();

install_test_encryption_keyring(current, vec![old.clone()]);

// `old` が現在のキーだった時点で書き込まれた値をシミュレートします。
let legacy_wire = suprnova::crypto::_test_encrypt_with(
    &old,
    suprnova::CryptPurpose::Cast,
    "legacy",
).unwrap();

// 現在のリングは、過去のキーのフォールバックを通じてそれを復号し、
// ローテーションの警告行を発します。
let plain = suprnova::Crypt::decrypt_string(
    suprnova::CryptPurpose::Cast,
    &legacy_wire,
).unwrap();

assert_eq!(plain, "legacy");
```

どちらのヘルパーも、`testing` フィーチャーが無効な場合（`default-features = false`）、本番バイナリからは除外されます。

## 失敗モード - エラーがどう見えるか

失敗しうる `Crypt::*` の呼び出しはすべて `Result<_, FrameworkError>` を返します。見えうる5つのエラーは次のとおりです:

| 原因 | どこで | どう表面化するか |
|---|---|---|
| `Crypt` が初期化されていない | 起動前のあらゆる呼び出し | `FrameworkError::Internal("Crypt is not initialized - set APP_KEY before serving")` |
| 暗号化文字列が有効なbase64ではない | `decrypt_string`、`decrypt` | `FrameworkError::Internal("Crypt base64 decode failed: …")` |
| 暗号化文字列が短すぎる（28バイト未満） | `decrypt_string`、`decrypt` | `FrameworkError::Internal("AEAD wire too short …")` |
| タグチェックが失敗する - 間違ったキー、間違ったAAD、改ざんされたバイト列 | `decrypt_string`、`decrypt` | `FrameworkError::Internal("AEAD decrypt failed: …")` |
| JSONのencode / decodeが失敗する | `encrypt`、`decrypt` | `FrameworkError::Internal("Crypt JSON {encode,decode} failed: …")` |

ゴミへのサイレントなフォールバックはありません。既存の暗号文に対して間違ったキーを使うことは、ファサードのレベルでもキャストのレベルでも、常にハードエラーです。これはLaravelの `Encrypter` の振る舞いと一致し、ローテーションを安全にする性質でもあります: 取りこぼされたカラムは、もっともらしいが間違った平文を返すのではなく、即座に表面化します。

過去のキーが暗号化文字列の復号に成功した場合でも、呼び出しは `Ok(...)` を返します - しかし、それと並んで `tracing::warn!` の行が発火するため、ログ駆動のアラートは、`APP_KEY_PREVIOUS` が削除される前に、ローテーションの尾を捉えます。

## 次のステップ

- [configuration.md](configuration.md) - `APP_KEY`、`APP_ENV`、そして起動時の環境変数の残り。
- [eloquent.md](eloquent.md) - `AsEncrypted*` キャスト、キャストの完全な表、そしてモデルのカラムに対するローテーションの手順。
- [hashing.md](hashing.md) - *復元*ではなく*検証*が必要なときの一方向の代替手段。bcryptとArgon2idのファサード、そして `AsHashed`。
- [auth-flows.md](auth-flows.md) - 2FAのシークレットとリカバリーコードの保存。それぞれ独自の用途で `Crypt` に乗っています。
- [session.md](session.md) - セッションクッキー。`CryptPurpose::Cookie` を介して `Crypt` によって暗号化され署名されます。

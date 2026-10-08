# ハッシング

`suprnova::hashing` モジュールは、フレームワークのパスワードハッシングの表面であり、3つの第一級ドライバー - **bcrypt**（デフォルト、Laravelと一致）、**Argon2i**（メモリハード、サイドチャネル耐性）、**Argon2id**（OWASP 2024の推奨）を持ちます。ユーザーのパスワードを保存するとき、remember-meの検証用トークンをハッシュ化するとき、あるいは一方向関数が正しいプリミティブとなるあらゆる場所で使ってください。ドライバーの選択は環境変数駆動であり、ファサードは端から端までアルゴリズムを意識しています（`info`、`is_hashed`、`needs_rehash`、`verify`）。そのため、保存済みのbcryptハッシュは、`HASH_DRIVER=argon2id` に切り替えた後でも検証できます。

## 概要

```rust
use suprnova::hashing;

// Async（Tokioのリクエストハンドラの内部で推奨 - CPUバウンドな
// ハッシュ計算をspawn_blockingで実行するため、ワーカースレッドが空いたままになります）:
let hashed = hashing::hash_async("my_password").await?;
let valid = hashing::verify_async("my_password", &hashed).await?;

// Sync（テスト、CLIツール、非asyncのコンテキスト）:
let hashed = hashing::hash("my_password")?;
let valid = hashing::verify("my_password", &hashed)?;
```

フリー関数のファサードは、有効なドライバーを `HASH_DRIVER` から読み取ります（未設定であればbcryptにフォールバックします）。ドライバーを明示して呼び出すには、ドライバー型を直接構築し、`hash_with` / `verify_with` / `needs_rehash_with` に渡してください。

## 設定

| 変数 | 説明 | デフォルト | 範囲 |
|----------|-------------|---------|-------|
| `HASH_DRIVER` | 有効なアルゴリズム | `bcrypt` | `bcrypt` \| `argon` \| `argon2i` \| `argon2id` |
| `HASH_ROUNDS` | Bcryptのコスト係数 | `12` | `4..=31`（bcryptのみ） |
| `HASH_MEMORY` | ArgonのメモリコストをKiB単位で | `65536`（64 MiB） | `>= 8`（argonのみ） |
| `HASH_TIME` | Argonの時間イテレーション | `4` | `>= 1`（argonのみ） |
| `HASH_THREADS` | Argonの並列度 / レーン数 | `1` | `>= 1`（argonのみ） |
| `HASH_VERIFY` | trueのとき、`verify()` はアルゴリズムをまたぐハッシュを拒否します | `false` | `true` / `false` |

設定ミス（不正な値、範囲外のパラメータ）は、`hash` / `verify` / `needs_rehash` への最初の呼び出しで `FrameworkError::param` として表面化します - サイレントなデフォルトへのフォールバックにはなりません。

### argon2id 用の `.env` の例

```env
HASH_DRIVER=argon2id
HASH_MEMORY=65536
HASH_TIME=4
HASH_THREADS=1
```

### なぜSuprnovaのArgon2デフォルトはLaravelより強力なのか

| パラメータ | Laravelのデフォルト | Suprnovaのデフォルト | 出典 |
|-------|-----------------|------------------|--------|
| メモリ | 1 024 KiB（1 MiB） | 65 536 KiB（64 MiB） | OWASP 2024 |
| 時間 | 2イテレーション | 4イテレーション | OWASP 2024 |
| スレッド | 2 | 1 | OWASP 2024 / libsodiumに準拠 |

Laravelのデフォルトは、PHPのリクエストごとプロセスモデルを前提としています - ワーカーが1回のパスワードハッシュにかけられる時間には限りがあり、それを超えるとボックスがいっぱいになってしまいます。Tokioの `spawn_blocking` は、リクエストループを凍りつかせることなく、Suprnovaがハッシュ計算をブロッキングスレッドプールへ手渡せるようにします。そのため、OWASP 2024の数値は実際の本番ハードウェア上でも現実的です。

## ドライバー

### Bcrypt（デフォルト）

```rust
use suprnova::hashing::{BcryptHasher, BcryptOptions, hash_with, verify_with};

let driver = BcryptHasher::new(BcryptOptions { rounds: 14 });
let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);
```

Bcryptには、パスワード入力に対する**72バイトのブロックサイズ上限**があります - 背後にあるプリミティブは、それより長い入力を黙って切り捨てるため、最初の72バイトを共有する2つの異なるパスフレーズは同じ値にハッシュされてしまいます。Suprnovaは事前に拒否します（フレームワークのbcryptの経路は、サイズ超過のパスワードに対して `hash()` でエラーを返し、`verify()` では `Ok(false)` を返すため、認証フローの「認証情報が無効」という応答は一律に保たれます）。Argon2にはそのような上限はありません。

このbcryptの上限は `suprnova::hashing::MAX_BCRYPT_PASSWORD_BYTES`（71 - bcryptのヌル終端文字を除いた実用上の上限）として公開されています。

### Argon2id（OWASP 2024の推奨）

```rust
use suprnova::hashing::{Argon2idHasher, Argon2Options, hash_with, verify_with};

let driver = Argon2idHasher::new(Argon2Options {
    memory: 65_536,  // 64 MiB
    time: 4,
    threads: 1,
})?;

let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);

// Argon2は任意の長さのパスフレーズを受け付けます - bcryptの72バイトの上限は
// 適用されません。
let long = "x".repeat(500);
let h = hash_with(&driver, &long)?;
assert!(verify_with(&driver, &long, &h)?);
```

### Argon2i

Argon2idと同じ形です。`Argon2iHasher::new(opts)`。新規のプロジェクトではArgon2idを使ってください - Argon2iはパリティのためにサポートされていますが、Argon2idが現代の推奨です。

## 明示的なコストを指定したBcrypt（`hash_with_cost`）

`hash_with_cost(password, cost)` と `hash_with_cost_async(password, cost)` は、`HASH_DRIVER` にかかわらず、呼び出し側が指定したコスト係数でbcryptハッシュを発行します。ポリシーやテナント単位の設定が、プロセスの環境変数ではなく呼び出し箇所へコストを流し込む場合に、これらを使ってください - 例えば、アプリの残りがデフォルトのコスト12で動く一方で、高セキュリティのアカウントクラスがコスト14を使う、といったケースです。

```rust
use suprnova::hashing::{hash_with_cost, hash_with_cost_async};

// Sync - テスト、CLIツール。
let h = hash_with_cost("my_password", 14)?;

// Async - Tokioのリクエストハンドラの内部。
let h = hash_with_cost_async("my_password", 14).await?;
```

両方のエントリーポイントは、`MIN_BCRYPT_COST..=MAX_BCRYPT_COST`（`4..=31`）の範囲外の `cost` を `FrameworkError::param` で拒否し、環境変数側の `HASH_ROUNDS` の検証を反映します:

```rust
use suprnova::hashing::{hash_with_cost, MIN_BCRYPT_COST, MAX_BCRYPT_COST};

assert!(hash_with_cost("pw", MIN_BCRYPT_COST - 1).is_err()); // < 4
assert!(hash_with_cost("pw", MAX_BCRYPT_COST + 1).is_err()); // > 31
```

この境界チェックが重要なのは、コストが1増えるごとにCPU時間が倍になるからです。コスト31では、コモディティハードウェア上で1回のbcryptハッシュに数時間かかります - フレームワーク内部の境界チェックは、ポリシーや設定のタイプミスが、ワーカースレッドをその日一日ピン留めしてしまう事故を防ぎます。async版は `spawn_blocking` を経由するため、正当に高いコストであってもリクエストループを凍りつかせません。

## アルゴリズムを意識した needs_rehash

`needs_rehash` は、保存済みのハッシュが有効なドライバーの下で再ハッシュされるべきときに `true` を返します。これは3つのケースをカバーします:

1. **アルゴリズムの不一致** - `HASH_DRIVER=argon2id` の間にbcryptハッシュが保存されている場合（あるいはその逆）。次に検証が成功したときにローテーションを引き起こします。
2. **パラメータの弱さ** - bcryptのコストが `HASH_ROUNDS` を下回る、あるいはargonの `m`/`t`/`p` が `HASH_MEMORY`/`HASH_TIME`/`HASH_THREADS` を下回る場合。
3. **Bcryptのレガシーなバリアント** - `$2a$`、`$2x$`、`$2y$` は、設定済みのコストであっても正規の `$2b$` へローテーションされます。

```rust
if hashing::needs_rehash(&stored_hash) {
    let fresh = hashing::hash_async("plaintext_at_login").await?;
    // `fresh` を永続化します。ログイン成功時に再ハッシュするという
    // Laravelの標準的なパターンであり、アルゴリズムをまたいで機能します。
}
```

形式が不正な入力は `true` を返します - 呼び出し側は、パースできないものを自然にローテーションします。

## ハッシュの検査（`info` + `is_hashed`）

```rust
use suprnova::hashing::{info, is_hashed};

let h = hashing::hash_async("my_password").await?;
let i = info(&h);
println!("algo: {}", i.algo.as_str());
println!("bcrypt cost: {:?}", i.rounds);
println!("argon memory KiB: {:?}", i.memory);

// 認識済みのアルゴリズムによるハッシュであればtrue、平文やゴミであればfalse。
assert!(is_hashed(&h));
assert!(!is_hashed("plaintext"));
```

`info().algo` は次のいずれかです: `Bcrypt`、`Argon2i`、`Argon2id`、`Argon2d`（認識はされますが、決して発行されません）、`Unknown`。

`is_hashed` は、`AsHashed` のEloquentキャストが、すでにハッシュ化されているカラムの再ハッシュをスキップするために使うものです - 3つのドライバーすべてで機能するため、プロジェクトの途中で `HASH_DRIVER` を切り替えても、次の保存でハッシュのハッシュというループが起きることはありません。

## アルゴリズムをまたぐ検証ゲート（`HASH_VERIFY`）

デフォルトでは、`verify()` は、そのハッシュを生成したアルゴリズムが何であるかにかかわらず、パスワードをハッシュと照合します - これによって、`HASH_DRIVER=argon2id` に切り替えた後も、レガシーなbcryptハッシュが検証を通り続けます（そのため、ログイン時にそれらをローテーションできます）。すべてのユーザーがローテーションを終えたら、`HASH_VERIFY=true` を設定して、有効なアルゴリズムを厳格に強制してください:

```env
HASH_VERIFY=true
```

ゲートがオンの状態では、`verify()` は、有効なドライバーとアルゴリズムが異なるあらゆるハッシュに対して `Ok(false)` を返します - Laravelの `RuntimeException` と同じ形ですが、Suprnovaは例外を投げるのではなくfalseを返します。認証フローの呼び出し側は、どちらにしても `Result<bool>` を期待しているからです。

## Async対Sync

コスト12のbcrypt（約250ms）も、メモリ=64 MiBのArgon2id（約80ms）も、意図的にCPUバウンドです - それこそが低速ハッシュの全体の狙いです。Tokioのリクエストハンドラから同期の `hash` / `verify` を直接呼び出すと、ハッシュの処理時間ぶんワーカースレッドをブロックし、同じワーカー上の他のリクエストを飢餓状態にします。

`async fn` のハンドラの内部では、`*_async` という兄弟関数を使ってください。これらは、CPUバウンドな呼び出しを `tokio::task::spawn_blocking` でラップするため、ワーカーは他のリクエストのために空いたままになります:

```rust
// GOOD - asyncハンドラの内部
let hashed = hashing::hash_async(&form.password).await?;

// BAD - ワーカーを約250msブロックする
let hashed = hashing::hash(&form.password)?;
```

同期のバリアントは、テスト、CLIツール、その他ブロッキングが問題にならない非asyncのコンテキストのためのものです。

## Eloquent統合: `AsHashed` キャスト

`#[cast(AsHashed)]` のEloquentキャストは、有効なドライバーを使って書き込み時に平文のフィールドをハッシュ化し、**すべてのドライバーをまたいでべき等**です - `password` カラムがすでに認識済みのハッシュ（bcryptまたはargon）を含んでいるモデルを保存しても、その値はそのまま変更なく通過します。この保護機構がなければ、`User::find(id).await?.save().await?` は保存のたびに既存のハッシュをハッシュ化してしまい、認証を壊してしまいます。

```rust
use suprnova::eloquent::casts::AsHashed;

#[suprnova::model]
struct User {
    #[cast(AsHashed)]
    pub password: String,
    // ...
}
```

このべき等性のチェックは `hashing::is_hashed` を使うため、プロジェクトの途中で `HASH_DRIVER` を切り替えても安全です - レガシーなbcryptハッシュも、新しいargon2idハッシュも、どちらも認識され、再保存時にスキップされます。

## `Auth::attempt` との併用

`Auth::attempt(&credentials)` は `UserProvider::validate_credentials` を呼び出し、それがさらにユーザーの保存済みハッシュに対して `hashing::verify_async` を呼び出します。Verifyは、設定されたドライバーではなく*保存済み*のハッシュのアルゴリズムにディスパッチします - そのため、`HASH_DRIVER=argon2id` に切り替えた後も、既存のすべてのbcryptハッシュは検証を通り続け、`needs_rehash` が `true` を返すため、標準的なログイン時ローテーションのパターンが、ユーザーベースを1回のログインごとに新しいアルゴリズムへと運びます。

## テストでドライバーを上書きする

`set_default_driver(Box<dyn Hasher>)` は、`HASH_DRIVER` を経由せずにドライバーを構築するテストや組み込みのCLIツールのために、プログラムからドライバーをインストールします。これは一度限りです - 最初の呼び出しが有効になり、2回目の呼び出しは、プロセスの途中でドライバーを差し替えるのではなく `FrameworkError::internal` を返します。どのコードパスがデフォルトを解決するよりも前に、スイートの起動時に使ってください。

## 次のステップ

- [認証](authentication.md) - `Auth::attempt`、ユーザープロバイダートレイト、そしてハッシングがログインとどう統合されるか
- [認証フロー](auth-flows.md) - `PasswordReset::complete` は、保存済みのパスワードハッシュを有効なドライバーを通じてローテーションします。remember-meのトークンは、保存前に `hash_async` によってハッシュ化されます
- [Eloquent](eloquent.md) - `#[cast(AsHashed)]` のリファレンスと、より広いキャストの表面
- [暗号化](encryption.md) - 保存データのための双方向の認証付き暗号化。一方向ハッシングの対をなすもの
- [エラー モデル](error-model.md) - ハッシングの設定値が拒否されたときに `FrameworkError::param` がどのような形になるか

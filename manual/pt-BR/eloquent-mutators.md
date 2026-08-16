# Eloquent - casts, acessadores e mutadores

Um cast medeia a fronteira entre o que uma coluna guarda em disco e o
que seu model carrega em memória. Um acessador inventa um atributo
virtual a partir das colunas que você já tem. Um mutador roteia
escritas para um campo através da sua própria transformação. Junto
com timestamps auto-gerenciados, eles são as quatro moving parts que
transformam uma linha plana em um valor Rust tipado.

Este capítulo cobre a superfície completa de cast (todo tipo
embutido, o override em runtime `casts!`, criptografia e hashing), as
macros de attribute `#[accessor]` e `#[mutator]`, o contrato de
auto-timestamp incluindo `touch()` e `without_touching`, e o evento
de ciclo de vida `Replicating` que dispara quando você clona um model
com `replicate()`.

Para a superfície mais ampla do model (`#[suprnova::model]`,
construtor de consultas, relacionamentos, observers) veja o capítulo
[API Eloquent](eloquent.md). Para eventos de ciclo de vida de ponta a
ponta veja [Eventos & Listeners](events.md). Para a facade de
criptografia que os casts criptografados usam veja
[Criptografia](encryption.md).

## Como os casts funcionam

Todo cast é uma struct que implementa o trait `Cast`:

```rust
pub trait Cast: Send + Sync {
    type Runtime;
    type Storage;

    fn to_storage(value: &Self::Runtime) -> Result<Self::Storage, FrameworkError>;
    fn from_storage(stored: &Self::Storage) -> Result<Self::Runtime, FrameworkError>;
}
```

`Runtime` é o tipo Rust que você escreve na struct do seu model
(`bool`, `chrono::NaiveDate`, `rust_decimal::Decimal`, seu próprio
enum). `Storage` é o tipo que o SeaORM vê na coluna (`i64` para uma
coluna booleana SQLite, `String` para uma data TEXT). As duas
direções são falíveis - o parsing temporal e decimal pode rejeitar
entrada malformada - então a macro propaga o `Result` através de
`From<inner::Model>` e do caminho de escrita `ActiveModel`.

Casts são explícitos. Um campo `Vec<String>` não se torna
implicitamente `AsArray<String>` porque a inspeção de tipo de campo
em tempo de macro quebraria no momento em que você renomeasse um
alias ou importasse um `Vec` diferente. Você declara casts no
attribute da macro:

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

A macro expande cada entrada `field = CastType` em chamadas para
`Cast::to_storage` e `Cast::from_storage` em toda leitura e escrita.
Você nunca invoca o cast você mesmo - você escreve o tipo em runtime,
o cast conecta a forma da coluna.

### Por que Suprnova diverge

O Laravel declara casts como
`protected $casts = ['tags' => 'array']`. A string `'array'` se
resolve para uma classe via um lookup em runtime, o que significa que
nomes de cast vivem como strings sem tipo até rodarem. O Suprnova
usa o tipo diretamente - `AsArray<String>` é um tipo Rust de verdade
que a macro verifica em tempo de compilação. Um erro de digitação no
nome do cast é um erro de compilação, não uma exceção em runtime três
semanas depois do deploy.

## Os casts primitivos

Cinco casts cobrem os tipos escalares do SQL.

### `AsBool`

`bool` ↔ `INTEGER` (0 / 1). O SQLite não tem coluna booleana nativa;
Postgres e MySQL fazem round-trip de `i64` de forma limpa através da
fronteira `Value::Int` do SeaORM. Uma única forma de armazenamento
deixa você usar o mesmo cast contra todo backend.

```rust
#[model(table = "settings", casts = { dark_mode = AsBool })]
pub struct Settings {
    pub id: i64,
    pub dark_mode: bool,
}
```

### `AsInt<I>`

Um inteiro mais estreito (`i32`, `u32`, `i16`) ↔ `i64`. O SeaORM
armazena inteiros como `i64` na coluna; o cast estreita na leitura e
expande na escrita. Valores fora do intervalo produzem um erro de
validação no momento da leitura em vez de truncar silenciosamente.

```rust
#[model(table = "counters", casts = { age = AsInt<u32> })]
pub struct Counter {
    pub id: i64,
    pub age: u32,
}
```

Use `AsInt<i64>` (ou omita o cast) quando o tipo em runtime já
corresponde ao de armazenamento.

### `AsFloat`

`f64` ↔ `REAL`. Passa direto nas duas direções - o cast existe por
paridade de nomenclatura com o cast `'float'` do Laravel; os backends
fazem round-trip de floats nativamente.

### `AsString`

`String` ↔ `TEXT`. Também passa direto; o cast existe para que o
override em runtime `Builder::with_casts(...)` possa fazer dele um
`DynCast` type-erased, como qualquer outro cast.

### `AsDecimal<P>`

`rust_decimal::Decimal` ↔ `TEXT`. `P` é a precisão (número de casas
decimais); valores são arredondados para `P` casas no caminho para o
armazenamento. O padrão é `P = 4`. O armazenamento é uma string de
formato fixo, então os round-trips são agnósticos de backend - o tipo
de coluna `Decimal` nativo do SeaORM tem semânticas de precisão
diferentes em cada driver, e o round-trip em string evita isso.

```rust
use rust_decimal::Decimal;
use suprnova::AsDecimal;

#[model(
    table = "ledger",
    casts = { amount = AsDecimal<2> },  // moeda, 2 casas decimais
)]
pub struct LedgerEntry {
    pub id: i64,
    pub amount: Decimal,
}
```

## Os casts temporais

Seis casts cobrem datas, datetimes, variantes imutáveis, e timestamps
Unix. Todos os casts não-timestamp armazenam como `TEXT` (ISO-8601 /
RFC-3339) para que o round-trip funcione em todo driver - o SQLite
armazena datetimes como strings nativamente, e Postgres / MySQL as
aceitam através da fronteira `Value::String` do SeaORM.

### `AsDate`

`chrono::NaiveDate` ↔ `TEXT` (`YYYY-MM-DD`).

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

`chrono::DateTime<Utc>` ↔ `TEXT` (RFC-3339). O cast padrão para
timestamps arbitrários quando você quer uma representação de
wall-clock.

As escritas são normalizadas como RFC-3339. As leituras também aceitam o texto
`CURRENT_TIMESTAMP` nativo do PostgreSQL e valores do SQLite/MySQL sem fuso
horário; valores sem fuso são interpretados como UTC. `AsImmutableDateTime` e
`AsOptionalDateTime` usam o mesmo parser.

### `AsImmutableDate` e `AsImmutableDateTime`

Mesma forma de armazenamento que `AsDate` / `AsDateTime`. O borrow
checker do Rust já impõe imutabilidade através de referências `&`,
então esses casts compartilham os tipos subjacentes - eles existem
por paridade com `immutable_date` / `immutable_datetime` do Laravel e
para documentar a intenção no local de declaração do model.

### `AsOptionalDateTime`

`Option<DateTime<Utc>>` ↔ `Option<String>`. Auto-injetado pela flag
`#[model(soft_deletes)]` para a coluna de tombstone anulável
(`deleted_at` por padrão - veja [Soft deletes](eloquent.md#deleting-and-soft-deletes)).
A option envolvida mantém a coluna de armazenamento anulável, então
linhas soft-deleted vs vivas se distinguem por `IS NULL` sem um valor
sentinela.

Use o cast diretamente em qualquer outra coluna datetime anulável que
você queira fazer round-trip como texto RFC-3339:

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

`i64` de época Unix ↔ `INTEGER`. Use quando a coluna é consultada
como um intervalo numérico ou usada em aritmética. Distinto de
`AsDateTime` - escolha `AsTimestamp` quando você quiser
`WHERE created_unix > 1700000000` e `AsDateTime` quando você quiser
strings RFC-3339 nos seus logs.

## Os casts estruturados

Cinco casts cobrem coleções, structs, e JSON arbitrário. Todos
serializam o valor em runtime para texto JSON e o armazenam em uma
coluna `TEXT`. As colunas `JSON` / `JSONB` nativas do Postgres e
`JSON` do MySQL aceitam o mesmo payload em string - se você quiser
um tipo de coluna JSON nativo para indexação, declare-o manualmente
em uma migração; a camada de cast não restringe o tipo da coluna.

### `AsArray<T>`

`Vec<T>` ↔ `TEXT` codificado em JSON. O tipo do elemento precisa ser
`Serialize + DeserializeOwned`.

```rust
use suprnova::AsArray;

#[model(table = "posts", casts = { tags = AsArray<String> })]
pub struct Post {
    pub id: i64,
    pub tags: Vec<String>,
}
```

### `AsObject<T>`

Uma struct `Serialize + DeserializeOwned` ↔ `TEXT` codificado em
JSON. Use quando a forma em runtime é um registro fixo com chaves
conhecidas estaticamente.

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

`Collection<T>` ↔ `TEXT` codificado em JSON. Wrapper fino sobre
`AsArray` que faz round-trip através do `Collection<T>` do Suprnova
(um newtype de `Vec<T>` com a superfície de slice no estilo Laravel -
veja [Coleções](eloquent.md#collections)).

### `AsJson<T>`

Qualquer tipo `Serialize + DeserializeOwned` ↔ `TEXT` codificado em
JSON. Use quando o campo é um `serde_json::Value` ou uma struct
definida pelo usuário que já é totalmente descritível em termos de
serde, mas não se encaixa no padrão de forma fixa do `AsObject`
(por exemplo, payloads de enum, mapas sem tipo).

### `AsArrayObject<T>`

`IndexMap<String, T>` ↔ `TEXT` codificado em JSON. Use quando a forma
em runtime é um mapa de chave dinâmica e a ordem das chaves importa
(a ordem de exibição de labels na UI, a ordem canônica de um bloco de
configuração). `IndexMap` em vez de `HashMap` é intencional: o serde
preserva a ordem de inserção através de `IndexMap`, e o `serde_json`
do Suprnova já vem configurado com `preserve_order` pelo mesmo
motivo.

Para registros de forma fixa use `AsObject`; para arrays use
`AsArray`.

## O cast de enum

### `AsEnum<E>`

`E: FromStr + AsRef<str>` ↔ `TEXT`. O nome da variante do enum (ou
sua string personalizada via `AsRefStr`) é o que chega na coluna. Não
há lock-in do framework em `strum`, mas é a forma mais ergonômica de
obter os dois bounds sem escrevê-los à mão:

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

Armazenamento por discriminante inteiro deliberadamente não é o
padrão. Um `Role::Admin = 0` que depois se torna `Role::Admin = 2`
após uma reordenação trocaria silenciosamente todo admin no banco de
dados. Nomes de variante são autodescritivos em um navegador de BD e
estáveis entre reordenações.

## Criptografia e hashing

Cinco casts medeiam transformações criptográficas na fronteira de
armazenamento. Os quatro casts `AsEncrypted*` compartilham a facade
[`Crypt`](encryption.md) - a facade precisa ser inicializada antes de
qualquer um deles rodar. Apps de produção conseguem isso através de
`Server::from_config` (que lê `APP_KEY` do ambiente); testes chamam
`suprnova::testing::install_test_encryption_key()` uma vez no
startup.

### `AsEncrypted`

`String` ↔ `String` criptografada com AES-256-GCM. A coluna em disco
guarda base64 URL-safe de `nonce || ciphertext_with_tag`. Cada
escrita usa um nonce novo e aleatório, então duas escritas do mesmo
texto plano produzem ciphertexts distintos - o administrador do seu
BD não consegue identificar segredos duplicados em repouso.

```rust
use suprnova::AsEncrypted;

#[model(
    table = "secrets",
    casts = { api_key = AsEncrypted },
)]
pub struct Secret {
    pub id: i64,
    pub api_key: String,  // em runtime é UTF-8 puro
}
```

O valor em runtime é a string UTF-8 descriptografada; você a lê e
escreve como qualquer outra `String`.

### `AsEncryptedArray<T>` / `AsEncryptedObject<T>` / `AsEncryptedCollection<T>`

`Vec<T>` / `T` / `Collection<T>` ↔ JSON criptografado com
AES-256-GCM. O pipeline é: serializa para JSON → criptografa →
base64 → armazena; inverso na leitura. O tipo do elemento / valor
precisa ser `Serialize + DeserializeOwned`.

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

### Rotação de chaves

A facade `Crypt` suporta rotação através de `APP_KEY_PREVIOUS`: a
criptografia sempre usa `APP_KEY`, mas a descriptografia tenta
`APP_KEY` primeiro e recorre a `APP_KEY_PREVIOUS` se a chave primária
falhar. Uma estratégia de recriptografia contínua é: defina `APP_KEY`
para a chave nova, mova a chave antiga para `APP_KEY_PREVIOUS`,
depois chame `save()` em toda linha criptografada para reescrever os
ciphertexts sob a chave nova. A camada de cast não precisa saber
sobre rotação - ela faz round-trip através de `Crypt` em toda leitura
e escrita, então um `User::all().await?` seguido de salvar cada linha
migra a coluna no lugar. Veja [Criptografia](encryption.md) para o
protocolo de rotação completo.

### `AsHashed`

`String` ↔ uma string com hash na escrita, usando o driver de hash
ativo (variável de ambiente `HASH_DRIVER` - bcrypt por padrão, argon2i
e argon2id também suportados). O valor em runtime É a string com
hash; não há direção inversa. Espelha o cast `hashed` do Laravel.

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

`AsHashed::to_storage` é **idempotente**: um valor que já parece com
QUALQUER hash reconhecido (bcrypt `$2*$`, argon2i / argon2id PHC)
passa direto sem mudança. Sem essa salvaguarda,
`User::find(id).await?.save().await?` faria hash de novo do hash
existente, transformando-o em um hash-de-hash, quebrando
`Hash::check(plain, stored)` e invalidando toda senha existente.

Combine `AsHashed` com o padrão `#[mutator]` (abaixo) quando você
precisar aplicar mais do que um hash na escrita - por exemplo,
normalizar espaços em branco ou rejeitar senhas vazias antes de fazer
hash.

## Override de cast em runtime - macro `casts!`

Os casts declarados em `#[model(casts = { ... })]` são estáticos -
eles disparam em toda leitura daquele model. Quando você precisa de
um cast diferente em uma única query (uma ferramenta de debug quer a
forma crua armazenada, um script de export quer uma representação
JSON diferente), use `Builder::with_casts(...)`:

```rust
use suprnova::{casts, AsDate, AsJson, User};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

A macro `casts!` constrói um `HashMap<&'static str, Arc<dyn DynCast>>`.
Cada entrada é `field_name = CastType`; todo cast embutido implementa
`IntoDynCast`, então a contraparte `DynCast` type-erased é
automática. O mapa de override em runtime só se aplica durante a
consulta encadeada - o pipeline de cast estático do model fica
inalterado.

Use esta superfície com moderação. O attribute do model é o lugar
certo para os casts que você quer que toda leitura aplique; o
override em runtime é a válvula de escape para consultas pontuais.

## Acessadores - attributes virtuais a partir de colunas reais

Um acessador é um método `impl` no model anotado com a macro
`#[accessor]`. Quando você lista o nome do método em
`#[model(appends = [...])]`, o `to_json()` do model chama o método e
insere o resultado sob aquela chave.

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

Um `serde_json::to_value(&user)` (ou `user.to_json()`) agora contém:

```json
{
  "id": 1,
  "first_name": "Alice",
  "last_name": "Xu",
  "full_name": "Alice Xu"
}
```

O método também pode ser chamado diretamente (`user.full_name()`) - a
macro `#[accessor]` é basicamente um marcador para que a macro
`#[suprnova::model]` no nível da struct possa conectar o dispatch de
`to_json()`. Não há custo em chamá-lo a partir do seu próprio código.

Cada nome em `appends` precisa corresponder a um método `#[accessor]`
real pelo identificador. Um erro de digitação (`appends = ["fullName"]`
quando o método é `full_name`) é capturado em tempo de compilação com
uma mensagem de erro apontada.

### Retornando valores que não são `String`

Acessadores podem retornar qualquer tipo `Serialize`. A macro
converte o valor retornado através de `serde_json::to_value` antes da
inserção, então:

```rust
impl Post {
    #[accessor]
    pub fn word_count(&self) -> usize {
        self.body.split_whitespace().count()
    }
}
```

renderiza como `"word_count": 42` na saída JSON.

### Ocultando as colunas de origem

Quando o valor do acessador é o que o consumidor deveria ver e as
colunas subjacentes são ruído, combine `appends` com `hidden`:

```rust
#[model(
    table = "users",
    appends = ["full_name"],
    hidden = ["first_name", "last_name"],
)]
```

`hidden` remove as colunas nomeadas da saída serializada; `appends`
então insere o valor do acessador. A ordem é fixa - os filtros rodam
primeiro, a injeção do acessador roda depois. Veja [Hidden, visible e
appends](eloquent.md#mass-assignment) para a superfície completa.

## Mutadores - escritas roteadas através da sua transformação

Um mutador é a contraparte do lado de escrita. Quando o nome do campo
aparece em `#[model(mutators = [...])]`, todo caminho de atribuição
em massa (`create` / `update`) roteia o valor através de
`self.set_<field>(value)?` em vez de atribuir o campo diretamente.

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
        // Normaliza + hash; o AsHashed faria o hash por conta própria,
        // mas o mutator é onde você também pode impor política.
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

`set_password` recebe um `serde_json::Value`. O corpo é dono do
desserializar + transformar - o tipo do campo na struct pode
continuar `String`, e sua validação roda antes de a coluna ser
tocada. Um erro retornado se propaga através de `create()` /
`update()` como um `bad_request`.

Atribuição direta ao campo contorna o mutador:

```rust
user.password = "raw".to_string();  // pula set_password
user.save().await?;                 // salva "raw"
```

Isso corresponde ao comportamento de `$user->password = ...` vs
`$user->fill(...)` do Laravel. Quando você quer que o mutador seja o
único caminho, roteie todas as escritas através de `attrs!` +
`create` / `update`.

### Combinando mutadores com casts

Um mutador e um cast podem coexistir no mesmo campo; o mutador roda
no caminho de escrita (quando `create` / `update` é chamado), o cast
roda no caminho de leitura (quando a coluna é materializada a partir
de um SELECT). Um padrão comum é usar `AsHashed` para a garantia de
idempotência do lado de leitura e o mutador para validação do lado de
escrita - o mutador faz hash, o `AsHashed` vê um valor já hasheado e
passa direto.

## Timestamps auto-gerenciados

Quando um model carrega tanto `created_at` quanto `updated_at`
(tipados `chrono::DateTime<chrono::Utc>`), a macro:

- Define os dois para `Utc::now()` em `create()`.
- Faz bump de `updated_at` a cada `save()` e `update(attrs)`.
- Emite um `impl Touchable for YourStruct` para que você possa
  chamar `.touch().await` e fazer bump de `updated_at` sem mudar
  nenhuma outra coluna.

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

// Faça bump de updated_at sem outras mudanças:
let post = Post::find_or_fail(1).await?;
post.touch().await?;
```

O armazenamento usa o cast `AsDateTime` que a macro auto-injeta para
colunas de timestamp. O cast deixa o mesmo valor `DateTime<Utc>`
fazer round-trip através dos três drivers do SeaORM (SQLite, MySQL,
PostgreSQL) sem forçar você a escolher um tipo de timestamp
específico do banco de dados.

### Opt-out e nomes de coluna personalizados

`#[model(timestamps = false)]` desativa o auto-gerenciamento
inteiramente - você controla os timestamps você mesmo.

`#[model(created_at = "creado_en", updated_at = "actualizado_en")]`
mantém o auto-gerenciamento, mas renomeia as colunas. A macro detecta
os campos renomeados e conecta a mesma lógica contra eles.

Quando a struct tem só UM dos dois campos de timestamp, a macro emite
um `compile_error!` - quase sempre um erro de digitação (`craeted_at`)
que você quer ver surgir de forma evidente em vez de ser engolido
silenciosamente.

### `without_touching` - supressão com escopo de task

Às vezes você quer atualizar uma linha sem fazer bump de
`updated_at` - rodando um backfill, corrigindo um erro de digitação,
registrando uma sincronização interna que não deveria resetar TTLs de
cache chaveados em `updated_at`. Envolva o trabalho em
`without_touching`:

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    for post in Post::query().get().await? {
        post.touch().await?;  // no-op dentro do escopo
    }
    Ok::<_, suprnova::FrameworkError>(())
}).await?;
```

A flag é um `tokio::task_local!`, então ela não vaza através de
fronteiras de `tokio::spawn` - solicitações concorrentes em outras
tasks continuam respeitando seu próprio escopo (ou a ausência dele).
Este é o análogo do Suprnova ao `Model::withoutTouching(closure)` do
Laravel.

### Por que Suprnova diverge

O Laravel usa uma propriedade estática `$timestamps = false` e um
método estático global `Model::withoutTouching` apoiado por um
contador de instância. As duas abordagens assumem isolamento de
processo-por-solicitação. O Suprnova roda muitas solicitações em um
único runtime Tokio, então uma flag global de processo deixaria uma
solicitação suprimir silenciosamente os timestamps de outra. O escopo
`tokio::task_local!` é ciente de async: ele acompanha futures através
de pontos `.await` dentro da mesma task e sai de escopo quando a
future é dropada, não importa como a solicitação termine.

## O evento de ciclo de vida `Replicating`

Dos 16 eventos de ciclo de vida do model (veja [Observers e eventos
de ciclo de vida](eloquent.md#observers-and-lifecycle-events)),
`Replicating` é o que dispara quando você clona uma linha existente
em uma cópia não salva em memória via `replicate()`:

```rust
let original = Post::find_or_fail(1).await?;
let mut copy = original.replicate().await?;  // não salva
copy.title = format!("{} (copy)", original.title);
copy.save().await?;  // agora persistida com uma nova PK
```

O evento `Replicating` dispara DEPOIS que o clone em memória é
construído, mas ANTES que você tenha tido a chance de alterá-lo.
Listeners recebem `(&Self, Arc<Mutex<Self>>)` - o original e a
réplica recém-construída atrás de um `Mutex`, para que você possa
alterar a réplica a partir do listener antes que o usuário a veja:

```rust
use suprnova::{Listener, FrameworkError};

pub struct ResetReplicatedFlags;

#[async_trait::async_trait]
impl Listener<post::events::Replicating> for ResetReplicatedFlags {
    async fn handle(&self, event: &post::events::Replicating) -> Result<(), FrameworkError> {
        let mut replica = event.replica.lock().await;
        replica.published = false;       // cópias começam não publicadas
        replica.view_count = 0;          // contadores resetados
        Ok(())
    }
}
```

A PK da réplica já está limpa no momento em que o listener roda - o
`replicate()` chama `reset_primary_key()` antes de disparar o evento,
então você não pode acidentalmente salvar de novo sob o ID original.
Timestamps também são resetados; `created_at` / `updated_at` disparam
no `save()` subsequente como qualquer linha nova.

### `replicate_into<T>` - replicação entre tipos

Quando a réplica é de um tipo diferente (`Post` → `Draft`, por
exemplo), use `replicate_into::<Draft>()`. O evento `Replicating` NÃO
dispara nesse caminho porque a struct do evento é por tipo-de-origem
e um listener registrado para `post::events::Replicating` receberia
um `Arc<Mutex<Post>>`, não um `Arc<Mutex<Draft>>`. O caminho entre
tipos é para quando você quer um tipo de destino novo sem
interferência de observer; registre um listener `Creating` normal no
tipo de destino se você quiser um hook na construção.

Veja [Replicação](eloquent.md#replication) para o resto da superfície
de replicate (`replicate_except`, o tratamento de relação da réplica,
as regras para PKs anuláveis).

## Juntando tudo

Um model com toda superfície deste capítulo:

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
    // deleted_at é auto-injetado por soft_deletes (AsOptionalDateTime)
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
        // O mutator faz hash; o AsHashed vê um valor já hasheado
        // em saves subsequentes e passa direto sem mudança.
        self.password = hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

Esta única declaração te dá:

- Oito casts tipados conectando a fronteira armazenamento / runtime.
- Um acessador que sintetiza `display_name` a partir de colunas
  existentes.
- Um mutador que valida e faz hash da senha.
- `created_at` / `updated_at` auto-gerenciados.
- Soft deletes com uma coluna `deleted_at` auto-injetada.
- Armazenamento criptografado de card-on-file com suporte a rotação
  de chave.

Todo cast é verificado em tempo de compilação. O construtor de
consultas de API dual (veja [Eloquent - construtor de
consultas](eloquent.md#query-builder--dual-api)) roda contra as
colunas tipadas; a serialização para Inertia / JSON aplica as regras
de hidden / appends; e um `User::find(id).await?` materializa a linha
através de oito chamadas `Cast::from_storage` sem você escrever uma
única linha de código de conversão.

## Próximos passos

- [API Eloquent](eloquent.md) - o resto da superfície de model:
  construtor de consultas, relacionamentos, observers, paginação,
  transações.
- [Criptografia](encryption.md) - a facade `Crypt` que os casts
  criptografados compartilham, o protocolo de rotação de chave, e a
  superfície de criptografia mais ampla.
- [Eventos & Listeners](events.md) - o dispatcher por trás de
  `Replicating` e dos outros 15 eventos de ciclo de vida do model.
- [Autenticação](authentication.md) - o trait `Authenticatable` e
  onde `AsHashed` se encaixa no fluxo de senha.
- [Validação](validation.md) - `FrameworkError::validation` e o
  padrão que mutadores usam para expor erros por campo.

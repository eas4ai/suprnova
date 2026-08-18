# API Eloquent

A camada Eloquent do Suprnova dá aos desenvolvedores Laravel a API
que eles já conhecem, implementada como uma camada fina sobre o
SeaORM. Copie código da documentação do Laravel, troque a sintaxe PHP
por Rust, adicione `.await?`, e funciona.

Toda a camada é um attribute de struct (`#[suprnova::model]`), uma
trait (`Model`), e um construtor de consultas encadeável
(`Builder<M>`) - é só isso. Nos bastidores a macro gera um `Entity`,
`Model`, `ActiveModel` e enum `Column` do SeaORM, além de toda impl
de trait do Eloquent. Os tipos do SeaORM permanecem acessíveis para o
raro caso em que a superfície do Eloquent não cobre (veja as
[válvulas de escape do SeaORM](#caindo-para-o-seaorm)).

## Sumário

- [Início rápido](#início-rápido)
- [O attribute `#[suprnova::model]`](#the-suprnovamodel-attribute)
- [Layout do módulo do model](#layout-do-módulo-do-model)
- [Encontrando linhas](#encontrando-linhas)
- [Criando e atualizando](#criando-e-atualizando)
- [Excluindo e soft deletes](#excluindo-e-soft-deletes)
- [Construtor de consultas - API dupla](#query-builder--dual-api)
- [Bloqueio de linha](#bloqueio-de-linha)
- [Transações](#transações)
- [Scopes](#scopes)
- [Relacionamentos](#relacionamentos)
- [Eager loading](#eager-loading)
- [Paginação](#paginação)
- [Chunking e iteração lazy](#chunking-e-iteração-lazy)
- [Coleções](#coleções)
- [Atribuição em massa](#atribuição-em-massa)
- [Casts](#casts)
- [Acessadores e mutadores](#acessadores-e-mutadores)
- [Timestamps](#timestamps)
- [Observers e eventos de ciclo de vida](#observers-e-eventos-de-ciclo-de-vida)
- [Prunable](#prunable)
- [Roteamento multi-conexão](#roteamento-multi-conexão)
- [Replicação](#replicação)
- [Depuração - dump e dd](#debugging--dump-and-dd)
- [Testando models](#testando-models)
- [Caindo para o SeaORM](#caindo-para-o-seaorm)
- [Migrando de `database::Model`](#migrating-from-databasemodel)
- [Facade DB - consultas sem model](#db-facade--model-less-queries)
- [Paridade com Laravel 13 - existência de relação + atalhos baratos](#laravel-13-parity--relation-existence--cheap-shortcuts)

## Início rápido

Um único attribute em uma struct a transforma num model Eloquent
completo:

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

Depois de declarado, você pode escrever:

- `User::query()` - inicia um construtor de consultas fluente.
- `User::find(id).await?` - busca pela chave primária.
- `User::find_or_fail(id).await?` - o mesmo, mas retorna erro com `ModelNotFound` quando não encontra.
- `User::all().await?` - toda linha.
- `User::create(attrs!{ name: "Alice", email: "alice@example.com" }).await?` -
  insere com filtragem de atribuição em massa.
- `User::filter("email", "alice@example.com").first().await?` -
  uma linha que corresponde.
- `user.update(attrs!{ name: "Alice B" }).await?` - atualização parcial.
- `user.save().await?` - persiste alterações em memória.
- `user.delete().await?` - remove a linha.
- `user.refresh().await?` / `user.fresh().await?` / `user.replicate().await?` -
  o resto do ciclo de vida do Laravel.

A struct voltada ao usuário (aqui `User`) É o tipo que seus handlers e
controllers carregam. A macro emite um módulo interno por model
(`user::`) com os tipos `Entity`, `Column`, `ActiveModel` e `Model` do
SeaORM para os casos em que você quer cair direto para o SeaORM. A
struct também é registrada em um `ModelEntry` apoiado em inventory,
para que código de admin e de ferramentas possa enumerar todo model
no boot.

## O attribute `#[suprnova::model]`

O único ponto de entrada para declarar um model. Todo attribute é
opcional; os padrões são ajustados para que uma struct com `id` +
`created_at` + `updated_at` funcione como um model Suprnova sem
nenhuma configuração.

### Referência dos attributes da macro

| Attribute | Tipo | Padrão | Notas |
|-----------|------|---------|-------|
| `table` | string | snake_case plural do nome da struct | Sobrescreve o nome da tabela |
| `primary_key` | string | `"id"` | Sobrescreve o nome da coluna de PK |
| `key_type` | type | `i64` | Tipo da PK - `String` para UUID, `i32` para esquemas legados |
| `auto_increment` | bool | `true` | Desative para PKs UUID |
| `connection` | string | `"default"` | Apps multi-conexão nomeiam uma conexão não padrão |
| `fillable` | lista de strings | (padrão = `guarded = ["id"]`) | Allowlist de atribuição em massa |
| `guarded` | lista de strings | `["id"]` quando nenhum é definido | Denylist de atribuição em massa (mutuamente exclusivo com `fillable`) |
| `casts` | map de `field = CastType` | `{}` | Casts por coluna |
| `hidden` | lista de strings | `[]` | Excluído de `to_json` / `to_array` |
| `visible` | lista de strings | (todos) | Variante inclusiva de `hidden` (mutuamente exclusivo) |
| `appends` | lista de strings | `[]` | Acessadores a incluir na serialização |
| `soft_deletes` | flag | `false` | Ativa a coluna `deleted_at` + semântica de tombstone |
| `soft_deletes_column` | string | `"deleted_at"` | Sobrescreve o nome da coluna de soft delete |
| `timestamps` | flag / bool | `true` quando `created_at` e `updated_at` existem | Desativa timestamps auto-gerenciados |
| `created_at` | string | `"created_at"` | Sobrescreve o nome da coluna |
| `updated_at` | string | `"updated_at"` | Sobrescreve o nome da coluna |
| `touches` | lista de nomes de relação | `[]` | Analisado e armazenado como metadado do model (`TOUCHES` const). O hook pós-save que chama `.touch()` nos pais listados ainda não está conectado - por ora, chame `parent.touch().await?` explicitamente a partir do seu observer ou handler. |
| `mutators` | lista de strings | `[]` | Nomes de campo cujo caminho de fill de JSON roteia através de um método mutador `set_<field>(value)` |

### Exemplo completo

```rust
use chrono::{DateTime, Utc};
use serde_json::Value as Json;
use suprnova::{model, AsBool, AsEncrypted, AsJson};

#[model(
    table = "users",
    fillable = ["name", "email", "preferences"],
    casts = {
        active = AsBool,
        preferences = AsJson<Json>,
        api_token = AsEncrypted,
    },
    hidden = ["password", "remember_token", "api_token"],
    appends = ["full_name"],
    soft_deletes,
    timestamps,
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub api_token: Option<String>,
    pub active: bool,
    pub preferences: Json,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}
```

### Macros no nível de função

Macros no nível de função funcionam junto com o attribute de struct:

- `#[accessor]` em um `fn name(&self) -> T` o transforma em um
  acessador Eloquent. O `to_array()` do model o chama quando `name`
  está listado em `appends = [...]` (e `to_json()` capta isso via a
  delegação `to_array` → string).
- `#[mutator]` em um `fn set_name(&mut self, value:
  serde_json::Value)` o transforma em um mutador Eloquent. O caminho
  de fill de JSON do model roteia por ele quando `name` está listado
  em `mutators = [...]`.
- `#[suprnova::scopes(Model)]` em um bloco `impl Model { ... }`:
  todo método cuja assinatura é
  `fn name(query: Builder<Self>[, args…]) -> Builder<Self>` se torna
  tanto um `.scope_name(args)` encadeável em `Builder<Self>` quanto
  um atalho `Model::scope_name(args)`. Não existe forma `#[scope]`
  no nível de função - scopes são declarados por bloco impl.
- Scopes globais são um registro em runtime via a trait
  `GlobalScope`, aplicado através de `Model::global_scope::<GS>()`.
  Não existe macro `#[global_scope]` no nível de função - veja
  [Macros](macros.md#suprnova-scopes-model) para o padrão completo.
- `#[prunable]` em `impl Prunable for T { ... }` registra o pruner
  via inventory para que `model:prune` o encontre.

## Layout do módulo do model

`#[suprnova::model]` mantém sua struct voltada ao usuário (por
exemplo `Post`) no scope pai e emite um `pub mod` irmão nomeado a
partir da struct em snake_case (`post`). É nesse módulo interno que
os tipos do SeaORM vivem.

Para um model declarado em `app/src/models/posts.rs`:

```rust
use chrono::{DateTime, Utc};
use suprnova::model;

#[model(table = "posts", fillable = ["title", "body"], timestamps)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Convenção: reexporte os tipos do SeaORM que a macro emite dentro
// do módulo interno para que os call sites possam usar os nomes sem
// prefixo. Os próprios models de dogfood do Suprnova carregam esta
// linha (veja `app/src/models/users.rs`, `app/src/models/posts.rs`,
// etc.).
pub use post::{ActiveModel, Column, Entity};
```

Agora você tem estes itens acessíveis a partir de
`crate::models::posts`:

| Path | O que é |
|------|-----------|
| `crate::models::posts::Post` | Sua struct voltada ao usuário - o model Eloquent |
| `crate::models::posts::post::Entity` | impl de `EntityTrait` do SeaORM para a tabela `posts` |
| `crate::models::posts::post::Column` | enum `Column` do SeaORM (uma variante por coluna) |
| `crate::models::posts::post::ActiveModel` | `ActiveModel` do SeaORM para insert/update |
| `crate::models::posts::post::Model` | linha no formato SeaORM (colunas tipadas por storage) |
| `crate::models::posts::{Entity, Column, ActiveModel}` | A convenção `pub use` acima; não é emitido automaticamente |

Duas coisas a saber sobre o `Model` do módulo interno:

1. É a linha no **formato SeaORM**, não sua struct `Post`. Colunas
   com cast carregam aqui seu tipo `Storage` (por exemplo, `bool` se
   torna o inteiro subjacente), e os slots de runtime `__eager` /
   `__pivot` da sua struct estão ausentes.
2. `From<post::Model> for Post` e `From<Post> for post::Model` fazem
   a ponte entre as duas formas. Veja [Caindo para o
   SeaORM](#caindo-para-o-seaorm) para o padrão de ida e volta.

`Model` intencionalmente **não** faz parte da reexportação
convencional do pai - o `Post` voltado ao usuário já ocupa o nome
`Post` no scope pai, e `post::Model` é um tipo separado que quem
chama alcança através de `post::Model` (ou conversão `From`) quando
precisa da forma interna.

### Quando acessar o módulo interno

A superfície do Eloquent (trait `Model` + `Builder<M>`) cobre a
grande maioria das consultas. Acesse `post::*` quando precisar de
recursos exclusivos do SeaORM:

- **Construção de consulta bruta** com a chain `EntityTrait::find()`
  do SeaORM quando o Eloquent não expõe o helper que você quer.
- **Lógica de join personalizada** - construindo joins `JoinType::*`
  explicitamente via `QuerySelect::join()` para uma relação que o
  `with(...)` do Eloquent não modela.
- **Subconsultas nativas do SeaORM** através de
  `Entity::find().select_only()`.
- **Mutação simples de `ActiveModel`** para o raro caso em que você
  quer contornar o ciclo de vida do Eloquent (sem observers, sem
  timestamps automáticos).

```rust
// Caso comum - Column reexportado no nível do módulo pai via a
// convenção `pub use post::{...}` acima.
use crate::models::posts::Column;

let drafts = Post::query()
    .db_where(Column::Status, "draft")
    .get()
    .await?;

// Caso de usuário avançado - acesse o módulo interno para o Entity
// do SeaORM diretamente. Isto é o que o `pub use` do pai não expõe.
use crate::models::posts::post;
use suprnova::sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

let db = suprnova::DB::connection()?;
let rows: Vec<post::Model> = post::Entity::find()
    .filter(post::Column::Status.eq("published"))
    .all(db.inner())
    .await?;

// Faz a ponte de volta para a forma do Eloquent quando quem chama
// quer isso.
let posts: Vec<Post> = rows.into_iter().map(Post::from).collect();
```

Se você se pegar acessando o módulo interno rotineiramente para a
mesma operação, isso é um sinal de que o Eloquent está sem um
helper - abra uma issue, ou adicione o helper à superfície `Model`
/ `Builder`.

## Encontrando linhas

```php
// Laravel
$user = User::find(1);
$user = User::findOrFail(1);          // lança exceção se não encontrar
$users = User::findMany([1, 2, 3]);
```

```rust
// Suprnova
let user: Option<User> = User::find(1).await?;
let user: User = User::find_or_fail(1).await?;
let users: Vec<User> = User::find_many([1, 2, 3]).await?;
```

`find_or_fail` retorna `FrameworkError::ModelNotFound` (HTTP 404
quando repassado a um controller).

### `first_or_create` / `update_or_create` / `first_or_new` / `first_or`

```php
// Laravel
$user = User::firstOrCreate(
    ['email' => 'alice@example.com'],
    ['name' => 'Alice'],
);
$user = User::updateOrCreate(
    ['email' => 'alice@example.com'],
    ['name' => 'Alice Updated'],
);
$user = User::firstOrNew(['email' => 'alice@example.com']);  // não salvo
```

```rust
// Suprnova
let user = User::first_or_create(
    attrs! { email: "alice@example.com" },          // chaves de busca
    attrs! { name: "Alice" },                       // extras no create
).await?;

let user = User::update_or_create(
    attrs! { email: "alice@example.com" },
    attrs! { name: "Alice Updated" },
).await?;

let user = User::first_or_new(
    attrs! { email: "alice@example.com" },
).await?;   // retorna um User não salvo; quem chama salva explicitamente
```

As chaves de busca vão no primeiro map; campos extras aplicados no
caminho de create vão no segundo map. Retornar um model não salvo
via `first_or_new` permite que quem chama o modifique ainda mais
antes de `save().await?`.

## Criando e atualizando

### Criar

```php
// Laravel
$user = User::create([
    'name' => 'Alice',
    'email' => 'alice@example.com',
]);
```

```rust
// Suprnova
let user = User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
}).await?;
```

`attrs!` é uma macro que produz um valor `Attrs` (um map JSON
tipado). JSON puro também funciona -
`User::create(serde_json::json!({"name": "Alice", "email": "..."}))`.
O filtro `Fillable` roda dentro de `create`; campos não fillable são
descartados silenciosamente, igual ao comportamento do Laravel.

### Save / update

```php
// Laravel
$user->name = 'Alice B';
$user->save();

$user->update(['name' => 'Alice B']);
```

```rust
// Suprnova
user.name = "Alice B".into();
user.save().await?;

user.update(attrs! { name: "Alice B" }).await?;
```

`save()` percorre todo campo que não é PK, define-os no ActiveModel
via `Set(...)`, chama o `update()` do SeaORM, e retorna a linha
canônica. `update(attrs)` é o mesmo fluxo, mas aplica um map de
attributes parcial primeiro (rodando o filtro Fillable e qualquer
mutador declarado).

### Increment / decrement

```php
// Laravel
$user->increment('login_count');
$user->increment('login_count', 5);
$user->decrement('credits', 10);
User::where('plan', 'free')->increment('quota_reset_count');
```

```rust
// Suprnova
user.increment("login_count", 1).await?;
user.increment("login_count", 5).await?;
user.decrement("credits", 10).await?;
User::filter("plan", "free").increment("quota_reset_count", 1).await?;
```

`increment` / `decrement` emitem SQL `UPDATE table SET col = col + N
WHERE ...` - atômico contra atualizações concorrentes, sem race de
read-modify-write. Disponível tanto em uma instância de model já
buscada (usa a PK da linha na cláusula WHERE) quanto como terminal
do Builder (usa as cláusulas WHERE da chain).

### Fresh / refresh / replicate

```php
// Laravel
$user->refresh();                          // recarrega do BD
$copy = $user->fresh();                    // busca + retorna cópia
$replica = $user->replicate();             // clone não salvo com PK nova
$replica = $user->replicate(['email']);    // pula um campo
```

```rust
// Suprnova
user.refresh().await?;
let copy: User = user.fresh().await?;
let replica: User = user.replicate().await?;
let replica: User = user.replicate_except(["email"]).await?;
```

`refresh` muta no lugar; `fresh` retorna uma cópia buscada
separadamente. `replicate` constrói um clone em memória com a PK
resetada (`Default::default()` para o tipo da chave). Quem chama
salva explicitamente.

### Evento Replicating

`replicate` e `replicate_except` disparam o evento por model
`Replicating { source, replica }` depois de construir o clone em
memória e ANTES de retorná-lo. O campo `replica` é um
`Arc<tokio::sync::Mutex<Self>>`, então listeners podem mutar a
replica antes que quem chama a veja - útil para prefixar títulos com
`(copy)`, limpar flags, resetar colunas derivadas, etc.

```rust
use suprnova::events::{EventFacade, Listener};
use async_trait::async_trait;

pub struct PrefixTitle;

#[async_trait]
impl Listener<post::events::Replicating> for PrefixTitle {
    async fn handle(&self, e: &post::events::Replicating)
        -> Result<(), FrameworkError>
    {
        let mut replica = e.replica.lock().await;
        replica.title = format!("(copy) {}", replica.title);
        Ok(())
    }
}

// Conecte isso uma vez no boot:
EventFacade::listen::<post::events::Replicating, _>(
    std::sync::Arc::new(PrefixTitle)
).await;
```

### Replicação entre tipos

```rust
let replica: UserDraft = user.replicate_into().await?;  // clone entre tipos
```

Uma divergência do Suprnova - o Laravel não consegue fazer isso
porque o PHP não tem tipos. Útil ao promover um model de rascunho
para um final, ou vice-versa.

`replicate_into<T>` NÃO dispara `Replicating` (o evento carrega
`Arc<Mutex<Self>>`, então um listener no tipo de origem não poderia
mutar a replica de outro tipo de qualquer forma). Quem chama e quer
configuração por T deve rodá-la no `T` retornado antes de chamar
`T::save` - a chain normal `Saving` / `Created` ainda dispara dentro
de `save`.

## Excluindo e soft deletes

### Flag de soft deletes

Adicione `soft_deletes` ao attribute da macro e uma coluna
`deleted_at: Option<DateTime<Utc>>` à struct:

```rust
#[model(table = "users", soft_deletes, timestamps)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub deleted_at: Option<DateTime<Utc>>,
    // ...
}
```

### Ciclo de vida

```rust
user.delete().await?;             // UPDATE: define deleted_at = NOW()
user.trashed();                   // -> true
let trashed = User::with_trashed().find(user.id).await?.unwrap();
trashed.restore().await?;         // UPDATE: define deleted_at = NULL

let only_dead = User::only_trashed().get().await?;
let all_including_dead = User::with_trashed().get().await?;

user.force_delete().await?;       // DELETE de fato
```

### Scope padrão

Quando `soft_deletes` é definido, a macro sobrescreve
`Model::query()` para que leituras padrão filtrem linhas trashed
automaticamente. `with_trashed()` e `only_trashed()` permitem optar
por incluí-las de volta. Concretamente: `User::find(id)` pula linhas
trashed; `User::with_trashed().find(id)` as encontra.

## Construtor de consultas - API dupla

`Builder<M>` é o tipo de consulta encadeável retornado por
`User::query()`, `User::filter(...)`, `User::db_where(...)`, e todo
outro método estático que não termina a chain.

### Nota de nomenclatura: API dupla

`where` é uma palavra reservada do Rust, então o método where de
igualdade simples não pode compartilhar o nome do Laravel. Em vez de
escolher um vencedor, todo método no formato where sai tanto com um
nome idiomático em Rust (`filter`, `filter_in`, `filter_null`, …)
quanto com um nome no formato Laravel (`db_where`, `where_in`,
`where_null`, …). São aliases sobre uma única implementação
canônica - escolha o que casar com sua memória muscular.

```rust
// Dev Rust:
User::query().filter("active", true).filter_in("role", ["admin"]).get().await?;

// Dev Laravel:
User::db_where("active", true).where_in("role", ["admin"]).get().await?;

// Mesma consulta. Mesmo resultado. Memória muscular diferente.
```

### Atalhos de where

```php
// Laravel
$users = User::where('email', $email)->get();
$users = User::where('age', '>=', 18)->get();
$users = User::where('email', 'like', '%@example.com')->get();
```

```rust
// Suprnova - escolha qualquer família; ambas compilam, ambas documentadas.

// Formato Rust (família filter):
let users = User::query().filter("email", &email).get().await?;
let users = User::query().filter_op("age", ">=", 18).get().await?;
let users = User::query().filter_like("email", "%@example.com").get().await?;

// Formato Laravel (família db_where / where_*):
let users = User::db_where("email", &email).get().await?;
let users = User::query().db_where_op("age", ">=", 18).get().await?;
let users = User::query().where_like("email", "%@example.com").get().await?;
```

### Variantes de where

Toda linha tem duas formas equivalentes no Suprnova - formato Rust
(`filter*`) e formato Laravel (`db_where` / `where_*`). Ambas chamam
a mesma implementação canônica; ambas são marcadas com
`#[doc(alias = "...")]` para que a busca do rustdoc encontre
qualquer uma.

| Laravel | Suprnova (formato Rust) | Suprnova (formato Laravel) | Notas |
|---------|----------------------|--------------------------|-------|
| `->where(col, val)` | `.filter(col, val)` | `.db_where(col, val)` | Igualdade |
| `->where(col, op, val)` | `.filter_op(col, op, val)` | `.db_where_op(col, op, val)` | Operador arbitrário |
| `->orWhere(...)` | `.or_filter(...)` | `.or_where(...)` | |
| `->whereNot(col, val)` | `.filter_not(col, val)` | `.where_not(col, val)` | |
| `->whereIn(col, vals)` | `.filter_in(col, vals)` | `.where_in(col, vals)` | |
| `->whereNotIn(col, vals)` | `.filter_not_in(col, vals)` | `.where_not_in(col, vals)` | |
| `->whereBetween(col, [a, b])` | `.filter_between(col, a..=b)` | `.where_between(col, a..=b)` | Range do Rust |
| `->whereNotBetween(col, [a, b])` | `.filter_not_between(col, a..=b)` | `.where_not_between(col, a..=b)` | |
| `->whereNull(col)` | `.filter_null(col)` | `.where_null(col)` | |
| `->whereNotNull(col)` | `.filter_not_null(col)` | `.where_not_null(col)` | |
| `->whereDate(col, '2026-05-19')` | `.filter_date(col, NaiveDate)` | `.where_date(col, NaiveDate)` | |
| `->whereMonth(col, 5)` | `.filter_month(col, 5)` | `.where_month(col, 5)` | |
| `->whereDay(col, 19)` | `.filter_day(col, 19)` | `.where_day(col, 19)` | |
| `->whereYear(col, 2026)` | `.filter_year(col, 2026)` | `.where_year(col, 2026)` | |
| `->whereTime(col, '12:30')` | `.filter_time(col, NaiveTime)` | `.where_time(col, NaiveTime)` | |
| `->whereLike(col, pattern)` | `.filter_like(col, pattern)` | `.where_like(col, pattern)` | |
| `->whereNotLike(col, pattern)` | `.filter_not_like(col, pattern)` | `.where_not_like(col, pattern)` | |
| `->whereJsonContains(col, v)` | `.filter_json_contains(col, v)` | `.where_json_contains(col, v)` | Despachado por backend |
| `->whereJsonLength(col, op, n)` | `.filter_json_length(col, op, n)` | `.where_json_length(col, op, n)` | |
| `->whereColumn(a, b)` | `.filter_column(a, b)` | `.where_column(a, b)` | Comparação coluna-a-coluna |
| `->whereExists(closure)` | `.filter_exists(builder)` | `.where_exists(builder)` | Subconsulta |
| `->whereHas(rel, closure)` | `.filter_has(rel, fn)` | `.where_has(rel, fn)` | Predicado de relação (10B) |
| `->whereDoesntHave(rel)` | `.filter_doesnt_have(rel)` | `.where_doesnt_have(rel)` | (10B) |
| `->whereRelation(rel, col, op, v)` | `.filter_relation(...)` | `.where_relation(...)` | (10B) |
| `->whereRaw(sql, bindings)` | `.filter_raw(sql, bindings)` | `.where_raw(sql, bindings)` | |

Predicados brutos vinculados usam marcadores `?` portáveis em
SQLite, MySQL e PostgreSQL:

```rust
let rows = User::query()
    .filter("active", true)
    .filter_raw(
        "score >= ? AND role = ?",
        vec![serde_json::json!(80), serde_json::json!("admin")],
    )
    .get()
    .await?;
```

No PostgreSQL, o Suprnova rebaseia esses marcadores depois dos
bindings de consulta anteriores, então o exemplo renderiza `$1` para
`active` e `$2`/`$3` para o predicado bruto. Use `??` para um
operador de ponto de interrogação literal em um fragmento bruto
vinculado, como `"payload ?? 'enabled' AND status = ?"`. Fragmentos
`$N` existentes continuam aceitos, mas marcadores portáveis evitam
acoplar call sites à posição na consulta. Estilos de marcador
misturados e incompatibilidades entre contagem de marcadores e de
bindings são rejeitados antes do I/O do banco de dados. Como em toda
expressão bruta, o texto SQL precisa ser confiável; valores não
confiáveis pertencem apenas ao vetor de bindings.

### Ordenação

```php
$users = User::orderBy('name', 'asc')->get();
$users = User::orderByDesc('created_at')->get();
$users = User::latest()->get();        // atalho: orderBy(created_at, desc)
$users = User::oldest()->get();        // atalho: orderBy(created_at, asc)
$users = User::inRandomOrder()->get();
```

```rust
let users = User::query().order_by("name", Direction::Asc).get().await?;
let users = User::query().order_by_desc("created_at").get().await?;
let users = User::latest().get().await?;
let users = User::oldest().get().await?;
let users = User::query().in_random_order().get().await?;
```

`Direction::Asc` / `Direction::Desc` é o enum do Suprnova
reexportado do SeaORM.

### Agrupamento + having

```php
$rows = User::groupBy('role')->having('count(*)', '>', 5)->get();
```

```rust
let rows = User::query()
    .group_by("role")
    .having_op("count(*)", ">", 5)
    .get()
    .await?;
```

### Limit / offset

```php
$users = User::limit(10)->offset(20)->get();
$users = User::take(10)->skip(20)->get();   // aliases
```

```rust
let users = User::query().limit(10).offset(20).get().await?;
let users = User::query().take(10).skip(20).get().await?;
```

### Select / add_select / select_raw

```rust
let users = User::query().select(["id", "name", "email"]).get().await?;
let users = User::query().select("name").add_select("email").get().await?;
let rows  = User::query().select_raw("count(*) as total, role")
    .group_by("role")
    .get_raw()
    .await?;
```

`get_raw()` retorna o resultado no formato bruto de colunas para
casos de `select_raw` em que as colunas não correspondem ao schema
do model; `get()` retorna `Vec<User>` e exige que as colunas
selecionadas preencham a struct do model.

### Distinct

```rust
let emails: Vec<String> = User::query().distinct().pluck("email").await?;
```

### Agregados

```rust
let count   = User::count().await?;
let count   = User::filter("active", true).count().await?;
let sum     = User::sum::<f64>("balance").await?;
let avg     = Order::avg::<f64>("total").await?;
let min     = Order::min::<DateTime<Utc>>("created_at").await?;
let max     = Order::max::<DateTime<Utc>>("created_at").await?;
let exists  = User::filter("email", &email).exists().await?;
let missing = User::filter("email", &email).doesnt_exist().await?;
```

Agregados são genéricos sobre o tipo de retorno porque o SeaORM
precisa saber para que coagir o escalar do BD. Padrões de tipo:
`count -> i64`; `sum`/`avg` carregam um parâmetro de tipo explícito.
O Suprnova cria alias internamente para as expressões de agregado
geradas, para que o mesmo resultado tipado seja decodificado no
PostgreSQL, MySQL e SQLite. `sum` e `avg` retornam zero para um
conjunto de resultado vazio, enquanto `min` e `max` retornam `None`.
Um tipo Rust solicitado incompatível ou uma coluna de resultado
ausente é um erro de banco de dados; nunca é convertido em um zero
ou `None` plausível.

### Terminais

```rust
let users:  Vec<User>          = User::all().await?;
let first:  Option<User>       = User::first().await?;
let user:   User               = User::first_or_fail().await?;
let value:  Option<String>     = User::filter("...").value("email").await?;
let emails: Vec<String>        = User::pluck::<String>("email").await?;
let keyed:  HashMap<i64, String> = User::pluck_keyed::<i64, String>("id", "name").await?;
let sql:    String             = User::filter("...").to_sql();
```

`to_sql` retorna o SQL parametrizado que o próximo terminal
emitiria - útil para depuração ou para construir views. Os bindings
são acessíveis via `.to_sql_with_bindings() -> (String, Vec<Value>)`.

### Uniões

```rust
let first  = User::filter("active", true);
let second = User::filter("role", "admin");
let users  = first.union(second).get().await?;
let users  = first.union_all(second).get().await?;
```

## Bloqueio de linha

Dois métodos do builder solicitam um bloqueio de banco de dados por
linha no momento do SELECT:

```rust
// Bloqueio de escrita exclusivo - bloqueia outras transações que
// tentam travar ou escrever as mesmas linhas até esta transação
// fazer commit.
let order = Order::query()
    .filter("id", 42)
    .lock_for_update()
    .first_or_fail()
    .await?;

// Bloqueio de leitura compartilhado - permite outros leitores
// compartilhados, bloqueia escritores.
let inventory = Inventory::query()
    .filter("sku", sku)
    .shared_lock()
    .first_or_fail()
    .await?;
```

SQL emitido por backend:

| Backend  | `lock_for_update()` | `shared_lock()`        |
|----------|---------------------|------------------------|
| Postgres | `FOR UPDATE`        | `FOR SHARE`            |
| MySQL    | `FOR UPDATE`        | `LOCK IN SHARE MODE`   |
| SQLite   | (sem SQL, veja abaixo) | (sem SQL, veja abaixo) |

A cláusula de bloqueio é anexada bem ao final da instrução
composta - depois de todo braço de `UNION`, todo `ORDER BY`, todo
`LIMIT` / `OFFSET`. Um `union(...)` de dois builders seguido de
`.lock_for_update()` emite exatamente **um** `FOR UPDATE` no scope
externo, não um por braço.

### Uso dentro de uma transação

O bloqueio só faz trabalho útil **dentro de uma transação** - sem
uma, o SQL ainda é emitido, mas o bloqueio libera ao final da
instrução. Combine com `DB::transaction(...)`:

```rust
DB::transaction(|tx| async move {
    let order = Order::query()
        .filter("id", 42)
        .lock_for_update()
        .first_or_fail()
        .with_tx(&tx)
        .await?;
    // Outras transações que tentam travar id=42 bloqueiam aqui até o commit.
    order.status = "processed".into();
    order.save_with_tx(&tx).await?;
    Ok(())
}).await?;
```

### `lock_for_update` vs `shared_lock`

A maioria dos fluxos de "ler depois escrever" quer
`lock_for_update`. Um bloqueio compartilhado ainda deixa outro
leitor `shared_lock` competir com você por um `UPDATE`
subsequente - só `FOR UPDATE` é mutuamente exclusivo.

`shared_lock` é certo para leituras de snapshot consistentes em que
você lê uma linha, deriva uma decisão a partir dela, e não escreve
de volta - por exemplo, uma verificação de estoque que não
decrementa o estoque ela mesma.

### SQLite

O SQLite não tem bloqueio no nível de linha. Ele usa apenas
bloqueio de transação no nível de arquivo (`BEGIN IMMEDIATE` /
`BEGIN EXCLUSIVE`). Os métodos de bloqueio são **mantidos** no
caminho do SQLite para que código cross-backend compile, mas eles
não emitem SQL.

Na primeira vez por processo em que `lock_for_update` /
`shared_lock` roda contra um backend SQLite, o framework loga um
único `warn!` no target de tracing `suprnova::eloquent::lock`. Isso
expõe o no-op sem inundar caminhos de código de alto volume.

Se você precisar de garantias de contenção entre linhas no SQLite,
envolva a seção crítica em uma transação `BEGIN IMMEDIATE`
explícita - no nível de arquivo isso bloqueia todo outro escritor.

### O que não está na v1

- **`NOWAIT` / `SKIP LOCKED`** - úteis para fluxos de claim de fila
  de jobs, mas adicionam superfície de API. Adiado até que um
  consumidor real precise deles.

## Transações

O Suprnova traz três pontos de entrada para transações de banco de
dados, além de rollback aninhado via savepoints. Dois deles - a
forma de closure e o helper de retry em deadlock - instalam um
contexto ambiente para que operações de model dentro da closure
roteiem automaticamente através da transação, sem que quem chama
precise passar um handle por todo call site.

### Forma de closure - `DB::transaction`

A forma de closure é o caso comum. A closure recebe um
`&Transaction` que pode usar para fazer checkpoint com
`savepoint(name)`; toda operação `Model::*` / `Builder::*` dentro da
closure roteia automaticamente através da transação via um
`tokio::task_local!` chamado `CURRENT_TX`.

```rust
use suprnova::{DB, FrameworkError, Model};

DB::transaction(|_tx| {
    Box::pin(async move {
        let mut alice = User::query().filter("name", "alice").first_or_fail().await?;
        alice.balance -= 30;
        alice.save().await?;

        let mut bob = User::query().filter("name", "bob").first_or_fail().await?;
        bob.balance += 30;
        bob.save().await?;
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

- A closure retorna `Ok` → **commit**.
- A closure retorna `Err` → **rollback** (o erro original se propaga).
- A closure entra em panic → rollback (a transação em andamento é
  dropada no unwind; o `DatabaseTransaction::drop` do SeaORM faz
  rollback).

Leituras dentro da closure veem escritas da mesma transação (via
lookup de `CURRENT_TX` em toda chamada SQL de folha). A primeira
chamada a `DB::transaction` depois do início do processo escolhe o
backend de banco de dados a partir de `DB::connection()`; chamadas
subsequentes reutilizam o mesmo registro de conexão.

A assinatura usa um higher-ranked trait bound + `Pin<Box<dyn
Future>>` para que closures possam tomar `tx` por empréstimo através
de pontos `.await`:

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        // ... trabalho antes do savepoint ...
        tx.savepoint("inner").await?;
        // ... trabalho interno ...
        if some_condition {
            tx.rollback_to("inner").await?;
        }
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

A forma `Box::pin(async move { ... })` é o custo de deixar a future
usar `&tx` depois de um `.await` - sem ela, o lifetime do empréstimo
não consegue escapar do corpo da closure. Espelha a assinatura de
`TransactionTrait::transaction` do SeaORM.

### Savepoints - `tx.savepoint(name)` / `tx.rollback_to(name)`

Savepoints fazem checkpoint da transação para que você possa
descartar um bloco de trabalho interno sem abortar o commit externo.
Funciona nos três backends - o `SAVEPOINT` do SQLite é totalmente
funcional mesmo que o SQLite não tenha bloqueio no nível de linha.

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        let mut account = Account::query().filter("id", id).first_or_fail().await?;
        account.balance = 200;
        account.save().await?;     // fica commitado quando a tx externa faz commit

        tx.savepoint("audit_trail").await?;

        let entry = AuditEntry::create(attrs! { actor_id: actor, ... }).await?;
        if audit_validation_failed(&entry) {
            tx.rollback_to("audit_trail").await?;
            // linha de audit_trail sumiu; atualização de account ainda pendente de commit
        }

        Ok::<(), FrameworkError>(())
    })
}).await?;
```

O nome do savepoint é interpolado literalmente no SQL - use um
identificador estático, **não** injete entrada do usuário.

### `DB::transaction` aninhado é rejeitado em runtime

```rust
DB::transaction(|_outer| Box::pin(async move {
    let inner = DB::transaction(|_inner| Box::pin(async move {
        Ok::<(), FrameworkError>(())
    })).await;
    // inner is Err(FrameworkError::Database(
    //     "nested DB::transaction is not supported; use tx.savepoint(name) for nested rollback"
    // ))
    Ok::<(), FrameworkError>(())
})).await?;
```

O `DatabaseConnection::begin()` do SeaORM não compõe - chamá-lo em
uma conexão que já está em uma transação inicia uma transação física
totalmente nova, que faz commit / rollback independentemente do
scope externo. Essa é uma armadilha silenciosa de integridade de
dados, então `DB::transaction` verifica `CURRENT_TX` antecipadamente
e retorna um erro de banco de dados em vez de produzir a semântica
errada. Use `tx.savepoint(name)` para comportamento aninhado.

### Retry em deadlock - `DB::transaction_with_attempts`

Leituras `SERIALIZABLE` do Postgres e bloqueios no nível de linha do
MySQL podem levantar erros de serialization-failure / deadlock que
se resolvem repetindo a transação. `transaction_with_attempts` roda
a closure do zero a cada vez, até `attempts`:

```rust
DB::transaction_with_attempts(3, |_tx| {
    Box::pin(async move {
        // Lógica isolada em SERIALIZABLE que pode competir com uma
        // tx concorrente e expor SQLSTATE 40001 / 40P01 no commit.
        let inventory = Inventory::query()
            .filter("sku", sku)
            .lock_for_update()
            .first_or_fail()
            .await?;
        if inventory.units < requested {
            return Err(FrameworkError::bad_request("out of stock"));
        }
        Inventory::query()
            .filter("sku", sku)
            .update(attrs! { units: inventory.units - requested })
            .await?;
        Ok::<(), FrameworkError>(())
    })
}).await?;
```

A detecção é por substring da string de Display contra o erro
interno:

- SQLSTATE `40001` do Postgres (serialization_failure)
- SQLSTATE `40P01` do Postgres (deadlock_detected)
- Substring `"deadlock"` sem diferenciar maiúsculas/minúsculas
  (cobre o `Deadlock found when trying to get lock` do MySQL e
  qualquer string de deadlock exposta pelo usuário)

Na tentativa final o erro se propaga sem alteração. A closure roda
do zero em toda tentativa - capture estado próprio (owned) ou
`Arc`s em vez de referências `&mut`, para que o caminho de retry
seja bem definido.

> **Ressalva:** como a detecção inclui uma substring `"deadlock"`
> sem diferenciar maiúsculas/minúsculas (necessária para o MySQL,
> cujo driver não expõe um SQLSTATE), qualquer erro interno cujo
> `Display` contenha a palavra vai disparar um retry. Ao levantar
> seus próprios erros de dentro de uma closure de
> `transaction_with_attempts`, evite "deadlock" na mensagem - caso
> contrário, um erro de validação não relacionado repete até
> `attempts` vezes antes de se propagar. As correspondências de
> SQLSTATE do Postgres (`40001` / `40P01`) são o sinal confiável; a
> heurística é só para o MySQL.

### Forma manual - `DB::begin_transaction` + shims `*_with_tx`

Quando o lifetime da transação não cabe em uma closure (por
exemplo, atravessa múltiplos ramos de controle de fluxo), abra uma
`Transaction` manual e opte cada operação nela explicitamente:

```rust
let tx = DB::begin_transaction().await?;

let mut user = User::query()
    .filter("name", "alice")
    .with_tx(&tx)
    .first_or_fail()
    .await?;
user.balance = 500;
user.save_with_tx(&tx).await?;

if some_condition {
    let mut other = User::query()
        .filter("name", "bob")
        .with_tx(&tx)
        .first_or_fail()
        .await?;
    other.update_with_tx(&tx, attrs! { balance: 200i64 }).await?;
}

tx.commit().await?;  // ou tx.rollback().await?;
```

O modo manual **não** instala `CURRENT_TX`. Direcione operações
individuais através da transação com `Builder::with_tx(&tx)` ou os
shims `Model::*_with_tx(&tx, ...)`:

| Método da trait     | Variante manual                           |
|---------------------|-------------------------------------------|
| `Model::create`     | `Model::create_with_tx(&tx, attrs)`       |
| `Model::save`       | `Model::save_with_tx(&tx)`                |
| `Model::update`     | `Model::update_with_tx(&tx, attrs)`       |
| `Model::delete`     | `Model::delete_with_tx(&tx)`              |
| `Model::force_delete` | `Model::force_delete_with_tx(&tx)`      |
| `Builder::*`        | `Builder::with_tx(&tx).*`                 |

Manter uma `Transaction` fixa uma conexão do pool durante toda a
vida do handle. No SQLite o pool tem uma única conexão, então
qualquer leitura paralela não transacional contra o mesmo banco de
dados bloqueia até a transação terminar - **carregue toda linha
pre-flight ANTES de `DB::begin_transaction()`** e roteie toda
escrita dependente através da `tx` retornada.

`Transaction::commit` / `Transaction::rollback` consomem o handle e
exigem `Arc::try_unwrap` da transação interna do SeaORM; se algum
clone de `TxHandle` (de `tx.handle()` / `Builder::with_tx(&tx)`)
ainda estiver vivo no momento do commit / rollback, ambos falham
com um erro "TxHandle clones still alive". O jeito certo de
corrigir é dropar seu `Builder<M>` / handles pendentes antes de
chamar `commit` - o framework se recusa a competir com uma escrita
meio-sem-commit contra um escritor paralelo que sustenta a mesma tx.

### Precedência

Precedência em três níveis para rotear uma operação através de uma
conexão:

1. **Override no nível do builder** - `Builder::with_tx(&tx)` ou
   qualquer shim `Model::*_with_tx(&tx, ...)`. Explícito vence
   ambiente.
2. **`CURRENT_TX` ambiente** - instalado por `DB::transaction` /
   `DB::transaction_with_attempts` para o scope de task da closure.
3. **Fallback do pool** - `DB::connection()` retorna o singleton
   global `DbConnection`.

Dentro de `DB::transaction(|tx| ...)`, chamar
`Builder::with_tx(&other_tx)` roteia explicitamente aquela consulta
através de `other_tx` - contornando o `CURRENT_TX` ambiente. Isso é
quase certamente um bug; o caminho de override existe para a forma
manual, não para sobrescrever a própria tx da closure.

### `with_tx` e scopes globais

Um builder carregando um `tx_override` ainda respeita scopes
globais, scopes nomeados, e o plano de eager-load - o override só
muda o roteamento de conexão, não o SQL.

### Limitações (v1)

- **Eager loads de relação** - `Builder::with(["posts"])` e
  `Collection::load(["posts"])` roteiam as subconsultas eager `IN
  (...)` através de `DB::connection()`, não através da transação
  ativa. Escritas pendentes dentro de uma closure `DB::transaction`
  **não** são visíveis para relações carregadas via `.with(...)`.
  Por ora, limite o trabalho de tx a chamadas diretas de `Model::*`
  / `Builder::*` / `DB::table(...)`; adie eager loads de relação
  para depois que a escrita externa for concluída (ou para antes de
  `DB::begin_transaction` no caminho manual). Esta é uma lacuna
  conhecida - o helper de roteamento (`ExecutorChoice`) já está no
  lugar em toda folha SQL; o bloqueio é o
  `EagerLoadDispatch::eager_load` receber `&DatabaseConnection`
  (concreto), que a macro emite para todo tipo de relação. Uma
  varredura futura vai adaptar a trait ao helper de dispatch.
- **DDL no Postgres** - `DB::statement(...)` dentro de uma transação
  roda o DDL contra a conexão da tx, o que o Postgres permite; o
  MySQL faz commit implícito e por isso não é suportado dentro de
  uma transação do Suprnova (isso corresponde à ressalva do
  `DB::transaction` do Laravel).

## Scopes

O Suprnova traz dois tipos de scope, espelhando o Laravel:

- **Scopes locais** - métodos de extensão no builder, declarados por
  model com `#[suprnova::scopes(Model)]`. Cada função livre no
  bloco `impl` anotado se torna tanto `Model::name()` (um starter
  estático) quanto `Builder::name()` (um método encadeável).
- **Scopes globais** - implementações de `GlobalScope<M>`
  registradas no boot via `ScopeRegistry::register::<M, _>(scope)`.
  Toda chamada a `Model::query()` os aplica em camadas
  automaticamente.

### Scopes locais

Declare scopes locais dando a eles a forma
`fn(query: Builder<Self>, args...) -> Builder<Self>`:

```rust
#[suprnova::scopes(User)]
impl User {
    pub fn active(query: Builder<Self>) -> Builder<Self> {
        query.filter("active", true)
    }

    pub fn popular(query: Builder<Self>, threshold: i64) -> Builder<Self> {
        query.filter_op("followers_count", ">", threshold)
    }
}

// Use tanto como starter quanto como método encadeável:
let active_users  = User::active().get().await?;
let popular_users = User::query().active().popular(500).get().await?;
```

Métodos que não são scope declarados no mesmo bloco `impl`
(qualquer coisa cujo primeiro parâmetro não seja `query:
Builder<Self>`) passam sem alteração.

### Scopes globais

Scopes globais se aplicam em toda chamada a `Model::query()`. O caso
de uso clássico é multi-tenancy - toda leitura é limitada ao tenant
atual sem que quem chama precise passar o filtro.

```rust
use suprnova::eloquent::scopes::{GlobalScope, ScopeRegistry};

pub struct TenantScope;

impl GlobalScope<Article> for TenantScope {
    fn apply(&self, query: Builder<Article>) -> Builder<Article> {
        // Lê o tenant atual a partir de um task-local /
        // AtomicI64 / onde quer que o estado por solicitação viva.
        query.filter("tenant_id", current_tenant_id())
    }
}

// No boot - tipicamente dentro do seu módulo provider/bootstrap:
ScopeRegistry::register::<Article, _>(TenantScope);

// Toda leitura é auto-limitada ao tenant ativo:
let scoped = Article::query().get().await?;
```

Múltiplos scopes por model compõem na ordem de registro - o
primeiro registrado roda primeiro, então suas cláusulas de filtro
aparecem primeiro na chain WHERE. Filtros combinados com AND não se
importam com a ordem, mas a ordem da esquerda para a direita importa
para qualquer cláusula cuja ordem de efeito colateral seja visível
(por exemplo, ordering, having, fragmentos brutos).

### Saindo de um scope global

Todo model que a macro `#[suprnova::model]` toca recebe dois
helpers estáticos emitidos nele:

```rust
// Contorna exatamente um scope registrado por tipo. Outros scopes ainda se aplicam.
let all_tenants = Article::without_global_scope::<TenantScope>().get().await?;

// Contorna todo scope registrado. Padrão de ferramentas de admin.
let everything = Article::without_global_scopes().get().await?;
```

**Importante:** os helpers de saída precisam ser o ponto de
entrada. Encadear `.without_global_scope::<S>()` em um builder já
retornado por `Model::query()` não desfaz scopes que já rodaram -
`Model::query()` aplica scopes eagerly no momento da construção,
então a máscara é definida tarde demais. Use os helpers estáticos
por model (acima) para a semântica correta.

### Onde scopes globais se aplicam

| Path | Scopes globais se aplicam? |
|------|----------------------|
| `Model::query()` | Sim - o ponto de entrada com scope canônico |
| `Model::without_global_scope::<S>()` | Sim, menos `S` |
| `Model::without_global_scopes()` | Não |
| `Model::find(id)` | Não - a busca por PK vai direto pelo SeaORM |
| `Model::find_many([...])` | Não - mesmo motivo |
| `Model::all()` | Não - mesmo motivo |

Isso espelha o Laravel: `Eloquent\Model::find` não dispara
`addGlobalScopes`. Quem chama e quer buscas por PK com scope usa
`Self::query().filter("id", pk).first().await?`.

### Soft deletes e scopes globais coexistem

`#[suprnova::model(soft_deletes)]` instala o filtro `deleted_at IS
NULL` via um mecanismo separado de string-tag, não através do
registro de scopes tipado. As duas camadas compõem:

- `Model::query()` filtra linhas trashed E roda todo scope
  registrado.
- `Model::without_global_scopes()` descarta scopes registrados mas
  preserva o filtro de soft delete - ferramentas de admin que
  querem ler todo conjunto de colunas ainda excluem linhas trashed
  por padrão.
- `Model::with_trashed()` e `Model::only_trashed()` pulam a
  filtragem de soft delete e também contornam o registro (eles
  constroem um builder novo, sem scope). Combine com
  `.without_global_scope::<S>()` se precisar de leituras com scope
  sobre linhas trashed.

## Relacionamentos

O Suprnova traz todo tipo de relação do Eloquent. Elas são
declaradas no bloco `relations = { ... }` em `#[suprnova::model]`, e
a macro emite - por relação declarada - um método na struct, um
acessador de carregamento (`<name>_loaded()`), um acessador de
contagem (`<name>_count()`), e o braço de dispatcher que o eager
loader chama. Esta seção cobre a forma por tipo e a tabela de
opções; o mergulho profundo em resolução de join-key, o registro de
morph, linhas de pivot, e o lowering do enum polimórfico vive em
[Relacionamentos Eloquent](eloquent-relationships.md). Os tipos de
relação disponíveis hoje:

| Tipo                | Um/muitos | Entre famílias | Baseado em |
|---------------------|----------|-----------------|-----------|
| `HasOne<R>`         | um      | não              | consulta `IN` em `<parent>_id` |
| `BelongsTo<R>`      | um      | não              | consulta `IN` na FK desta linha |
| `HasMany<R>`        | muitos     | não              | igual a `HasOne`, retorna `Vec<R>` |
| `BelongsToMany<R, P>` | muitos   | não              | tabela pivot `P`, INNER JOIN + `pivot::<P>()` |
| `HasOneThrough<B, R>`  | um   | não              | JOIN em duas consultas `parent → B → R` |
| `HasManyThrough<B, R>` | muitos  | não              | igual ao de cima, retorna `Vec<R>` |
| `MorphOne<R>`       | um      | sim             | `IN` + filtro `<name>_type = "<self>"` |
| `MorphMany<R>`      | muitos     | sim             | igual a `MorphOne`, retorna `Vec<R>` |
| `MorphTo`           | um      | sim (filhos → muitas famílias) | enum por família emitido no local da declaração |
| `MorphToMany<R, P>` | muitos     | sim             | pivot m2m polimórfico `P` |
| `MorphedByMany<R, P>` | muitos   | sim (inverso)   | mesmo pivot, escaneado no sentido contrário |

### Sintaxe de `relations = { ... }`

Toda declaração de relação carrega a mesma forma externa: o nome da
relação, o tipo, o tipo relacionado (e tipos de pivot/intermediário
quando aplicável), e um bloco `{ ... }` de opções.

```rust
use suprnova::model;

#[model(
    table = "users",
    relations = {
        // HasMany<R>
        posts: HasMany<crate::models::Post> {
            fk = "author_id",         // sobrescreve o padrão `user_id`
        },
        // BelongsToMany<R, Pivot>
        roles: BelongsToMany<crate::models::Role, crate::models::RoleUser> {
            with_pivot = ["assigned_at"],
            with_timestamps,
        },
    },
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Opções comuns:

| Opção                     | Tipos de relação                | Propósito |
|----------------------------|-------------------------------|---------|
| `fk = "..."`               | todo tipo com FK no filho    | Coluna no FILHO apontando para o pai. Padrão = `<snake(parent_struct)>_id`. |
| `lk = "..."`               | tipos um/muitos                | Coluna no PAI usada como chave de join. Padrão = `"id"`. |
| `related_key = "..."`      | `BelongsToMany`, `MorphToMany` | O nome da COLUNA de PK do lado relacionado. Padrão = `"id"`. Obrigatório quando o model relacionado usa uma PK que não é `id`. |
| `with_pivot = ["...", ...]` | `BelongsToMany`, `MorphToMany` | Colunas extras no pivot para expor no join. |
| `with_timestamps`          | `BelongsToMany`, `MorphToMany` | Marca `created_at` / `updated_at` no attach/sync. |
| `with_default = \|\| { ... }` | `BelongsTo`                 | Closure que produz um padrão quando a FK é null OU o pai está ausente. |
| `first_key`, `second_key`, `second_local_key` | `HasOneThrough`, `HasManyThrough` | Overrides de chave de JOIN - veja a seção Through abaixo. |
| `name = "..."`             | todo tipo morph              | Nome da família morph (por exemplo, `"commentable"`, `"taggable"`). Direciona as colunas `<name>_id` / `<name>_type` no filho/pivot. |
| `targets = [T1, T2, ...]`  | `MorphTo`                     | A lista de alvos morph concretos. A macro emite um enum `<Name>Morph` no local da declaração com uma variante por alvo, mais `Unknown(String, i64)`. |
| `target_morph_type = "..."` | `MorphedByMany`              | A string de morph-type identificando a família alvo no pivot. |
| `pivot_table`, `pivot_foreign_key`, `pivot_related_key` | `BelongsToMany`, `MorphToMany` | Overrides de coluna / tabela do lado do pivot quando os padrões não servem. |

### `HasOne<R>` e `BelongsTo<R>`

Um-para-um em ambas as direções. `HasOne` vive do lado do pai e
chama `R::query().filter(<fk>, <self.id>).first()`. `BelongsTo` vive
do lado do filho e lê a FK a partir de `self`, depois chama
`R::query().filter(<owner_key>, <fk_value>).first()`.

```rust
#[model(table = "users", relations = {
    profile: HasOne<crate::models::Profile>,
})]
pub struct User { /* ... */ }

#[model(table = "profiles", relations = {
    user: BelongsTo<crate::models::User>,
})]
pub struct Profile {
    pub id: i64,
    pub user_id: i64,
    pub bio: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let user = User::find(1).await?.unwrap();
let profile: Option<Profile> = user.profile().first().await?;

let profile = Profile::find(42).await?.unwrap();
let owner: Option<User> = profile.user().first().await?;
```

`BelongsTo` suporta `with_default = || R { ... }`, que dispara
tanto quando a FK é null quanto quando a linha do pai está ausente.
A closure de padrão roda por chamada (e por linha eager-loaded) -
perfeito para um substituto vazio quando um usuário deletado ainda
tem comentários:

```rust
#[model(table = "comments", relations = {
    author: BelongsTo<crate::models::User> {
        with_default = || User {
            name: "[deleted]".into(),
            ..Default::default()
        },
    },
})]
pub struct Comment { /* ... */ }

let c = Comment::find(99).await?.unwrap();
// Sempre Some - o padrão dispara quando a linha do usuário está ausente.
let author = c.author().first().await?.unwrap();
```

### `HasMany<R>`

Um-para-muitos do lado do pai. Retorna um builder fluente; encadeie
filter / order / latest / take / get / count e termine.

```rust
#[model(table = "users", relations = {
    posts: HasMany<crate::models::Post> {
        fk = "author_id",
    },
})]
pub struct User { /* ... */ }

let u = User::find(1).await?.unwrap();

// Todo post deste usuário, ordenação padrão:
let posts: Vec<Post> = u.posts().get().await?;

// Filtrado + ordenado + paginado:
let recent = u.posts()
    .filter("published", true)
    .latest()                          // ORDER BY created_at DESC
    .take(10)
    .get()
    .await?;

// Só COUNT - sem buscar linhas:
let total: i64 = u.posts().count().await?;
```

Métodos terminais disponíveis: `.first()`, `.get()`, `.count()`.
Filtros encadeáveis disponíveis: `.filter` / `.db_where`,
`.filter_in` / `.where_in`, `.order_by`, `.latest`, `.oldest`,
`.limit`, `.take`.

### `BelongsToMany<R, P>` - pivot de primeira classe

Muitos-para-muitos através de um pivot declarado com
`#[suprnova::model]`. O pivot é um model de primeira classe com sua
própria identidade de linha - não uma tupla, não um hash map
escondido. Dois benefícios principais sobre a forma de pivot anônimo
do Laravel:

1. A linha de pivot é type-safe. Leia colunas `with_pivot` via
   `r.pivot::<P>().<column>`, nunca via `r.pivot.get("...")`.
2. O model de pivot é acessível a partir do resto do framework
   (factories, scopes, casts, hooks) do mesmo jeito que todo model é.

```rust
#[model(table = "role_user", fillable = ["user_id", "role_id", "assigned_at"])]
pub struct RoleUser {
    pub id: i64,
    pub user_id: i64,
    pub role_id: i64,
    pub assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[model(table = "users", relations = {
    roles: BelongsToMany<crate::models::Role, RoleUser> {
        with_pivot = ["assigned_at"],
        with_timestamps,
    },
})]
pub struct User { /* ... */ }

let u = User::find(1).await?.unwrap();
let admin = Role::create(attrs! { name: "admin" }).await?;

// Mutators de attach + sync
u.roles().attach(admin.id).await?;
u.roles().attach_with(admin.id, attrs! { assigned_at: chrono::Utc::now() }).await?;
u.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
u.roles().detach(admin.id).await?;

// Leia dados de pivot através do acessador de downcast por linha:
let roles = u.roles().get().await?;
for r in &roles {
    let p: &RoleUser = r.pivot::<RoleUser>();
    println!("user {} got role {} at {:?}", p.user_id, p.role_id, p.assigned_at);
}
```

- `.attach(id)` - faz INSERT de uma única linha de pivot. Retorna
  erro em duplicata a menos que seu pivot permita (o framework não
  faz dedupe na camada Rust; use `.sync` para idempotência).
- `.attach_with(id, attrs! { ... })` - faz INSERT com colunas extras
  de pivot. Marca timestamps quando `with_timestamps` está ativado.
- `.detach(id)` - faz DELETE da(s) linha(s) de pivot que ligam
  parent → id.
- `.sync([ids...])` - diff-and-apply: faz attach do que é novo,
  detach do que está faltando, e deixa a intersecção intacta.
  Envolvido em uma transação.

`.get()` retorna `Vec<R>` com o pivot marcado no campo interno
`__pivot` de cada linha. O acessador `.pivot::<P>()` faz downcast do
`Arc<dyn Any>` para o tipo de pivot que você declarou. Chamá-lo com
o tipo errado entra em panic - faça o tipo corresponder ao pivot
declarado.

### `HasOneThrough<B, R>` e `HasManyThrough<B, R>`

Alcança um alvo final `R` através de um intermediário `B`. Útil
quando a relação atravessa duas tabelas, mas você não precisa expor
o intermediário (`A → B → R`).

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<crate::models::User, crate::models::Post>,
})]
pub struct Country {
    pub id: i64,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let c = Country::find(1).await?.unwrap();
let posts: Vec<Post> = c.posts().get().await?;
```

O dispatcher infere chaves de JOIN a partir dos nomes das structs.
Overrides:

| Opção              | Padrão                          | Descrição |
|---------------------|----------------------------------|-------------|
| `first_key`         | `<snake(parent_struct)>_id`      | Coluna no intermediário `B` apontando para o pai `A`. |
| `second_key`        | `<snake(intermediate_struct)>_id` | Coluna no alvo final `R` apontando para o intermediário `B`. |
| `second_local_key`  | `"id"`                           | Coluna no intermediário `B` correspondida por `second_key`. Obrigatória quando `B` usa uma PK que não é `id`. |

A coluna de chave primária do pai é lida a partir da declaração
`primary_key` do model (com padrão `"id"`) - não existe override de
`local_key` em `HasManyThrough` / `HasOneThrough`; mude a PK do pai
via o attribute `#[suprnova::model]` se precisar de uma chave de pai
que não seja `id`.

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<crate::models::User, crate::models::Post> {
        first_key = "country_id",
        second_key = "author_id",
    },
})]
pub struct Country { /* ... */ }
```

### `MorphTo` com `targets = [...]` e enum por família

Relações polimórficas apontam uma linha filha para uma de várias
famílias de pai. O filho carrega um par `(<name>_id, <name>_type)`;
a coluna `*_type` guarda a string de morph-type que cada pai
declara.

`MorphTo` vive no filho. Sua declaração lista toda família de pai
para a qual pode apontar via `targets = [...]`. A macro emite um
enum por família chamado `<RelationName>Morph` (correspondendo à
forma PascalCase do nome da relação, com o sufixo `Morph`) com uma
variante por tipo alvo, mais `Unknown(String, i64)` para linhas
legadas cujo valor de `<name>_type` não corresponde a nenhum alvo
registrado.

```rust
#[model(table = "posts", morph_type = "post")]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video")]
pub struct Video { /* ... */ }

#[model(table = "comments", relations = {
    commentable: MorphTo {
        name = "commentable",
        targets = [
            crate::models::Post,
            crate::models::Video,
        ],
    },
})]
pub struct Comment {
    pub id: i64,
    pub commentable_id: i64,
    pub commentable_type: String,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

let c = Comment::find(1).await?.unwrap();
match c.commentable().get().await? {
    CommentableMorph::Post(post)   => println!("comment on post {}", post.title),
    CommentableMorph::Video(video) => println!("comment on video {}", video.url),
    // Linhas legadas / soltas - `<name>_type` não corresponde a
    // nenhum alvo, OU o morph_type correspondeu mas a linha em
    // `<name>_id` desapareceu.
    CommentableMorph::Unknown(ty, id) => {
        eprintln!("comment {} points at unknown {ty}#{id}", c.id);
    }
}
```

O attribute `morph_type = "..."` em cada struct alvo é o que o
loader escreve na coluna `<name>_type` do filho no insert e usa
como filtro na leitura. Sem `morph_type`, o framework deriva a
type-string a partir de `to_snake(struct_name)`.

O dispatch de `MorphTo` - como o enum por família escolhe a variante
certa - consulta o registro morph em runtime (o inventory populado
por toda declaração `#[suprnova::model(morph_type = "...")]`). Para
cada alvo declarado, o helper de fetch procura o `TypeId` do alvo,
lê a string `morph_type` registrada, e a compara contra o valor
`<name>_type` armazenado na linha filha. A primeira correspondência
vence, na ordem de declaração. Alvos sem um attribute `morph_type`
explícito recorrem a `to_snake(target_type_name)` - o mesmo padrão
que `MorphMany` / `MorphOne` do lado do pai usa para marcar a
type-string no momento da escrita, então os dois lados ficam
alinhados. Isso significa que valores customizados de `morph_type`
(por exemplo, `morph_type = "blog_post"` em uma struct chamada
`Post`, ou qualquer string não convencional) fazem dispatch
corretamente sem mudanças no local da declaração.

### `MorphOne<R>` e `MorphMany<R>` - lado pai

A direção inversa de `MorphTo`: um tipo pai declara o um-ou-muitos
polimórfico que possui. `MorphOne` retorna `Option<R>` a partir de
`.first()`; `MorphMany` retorna `Vec<R>` a partir de `.get()`. Ambos
filtram o par `(<name>_id, <name>_type)` do filho por `self.id` e
pelo `morph_type` do pai.

```rust
#[model(table = "posts", morph_type = "post", relations = {
    comments: MorphMany<crate::models::Comment> {
        name = "commentable",
    },
    cover: MorphOne<crate::models::Image> {
        name = "imageable",
    },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    comments: MorphMany<crate::models::Comment> {
        name = "commentable",
    },
})]
pub struct Video { /* ... */ }

let post = Post::find(1).await?.unwrap();
let post_comments: Vec<Comment> = post.comments().get().await?;
let post_cover:    Option<Image> = post.cover().first().await?;

let video = Video::find(1).await?.unwrap();
let video_comments: Vec<Comment> = video.comments().get().await?;
// post.comments() retorna só linhas `commentable_type = "post"`;
// video.comments() retorna só linhas `commentable_type = "video"`.
```

A mesma superfície encadeável de `HasMany` / `HasOne`: `.filter` /
`.db_where`, `.order_by` / `.latest` / `.oldest`, `.limit` / `.take`,
`.first` / `.get` / `.count`.

### `MorphToMany<R, P>` e `MorphedByMany<R, P>`

Muitos-para-muitos polimórfico. O pivot compartilhado `P` carrega o
par de FK MAIS uma coluna discriminadora `<name>_type`. Um lado
declara `MorphToMany` (por exemplo, `Post.tags()`, `Video.tags()`),
o outro lado declara um `MorphedByMany` por família alvo (por
exemplo, `Tag.posts()`, `Tag.videos()`).

```rust
#[model(table = "taggables", fillable = ["tag_id", "taggable_id", "taggable_type"])]
pub struct Taggable {
    pub id: i64,
    pub tag_id: i64,
    pub taggable_id: i64,
    pub taggable_type: String,
}

#[model(table = "posts", morph_type = "post", relations = {
    tags: MorphToMany<crate::models::Tag, Taggable> {
        name = "taggable",
    },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    tags: MorphToMany<crate::models::Tag, Taggable> {
        name = "taggable",
    },
})]
pub struct Video { /* ... */ }

// Inverso: Tag declara um MorphedByMany por família alvo.
#[model(table = "tags", relations = {
    posts: MorphedByMany<crate::models::Post, Taggable> {
        name = "taggable",
        target_morph_type = "post",
    },
    videos: MorphedByMany<crate::models::Video, Taggable> {
        name = "taggable",
        target_morph_type = "video",
    },
})]
pub struct Tag { /* ... */ }

let post  = Post::find(1).await?.unwrap();
let video = Video::find(1).await?.unwrap();
let tag   = Tag::create(attrs! { name: "rust" }).await?;

// `attach` / `attach_with` / `detach` / `sync` funcionam do mesmo
// jeito que em BelongsToMany. A coluna `<name>_type` chega
// automaticamente a partir do `morph_type` do pai que chama.
post.tags().attach(tag.id).await?;
video.tags().attach(tag.id).await?;          // attachment independente
post.tags().sync([tag_a.id, tag_b.id]).await?;

// Direção inversa - Tag se divide por família:
let posts_with_tag:  Vec<Post>  = tag.posts().get().await?;   // tipado "post"
let videos_with_tag: Vec<Video> = tag.videos().get().await?;  // tipado "video"
```

O `target_morph_type` de `MorphedByMany` é obrigatório porque a
macro no local da declaração de `Tag` não consegue introspectar o
attribute `morph_type = "..."` do alvo (ele vive em uma invocação
separada de `#[suprnova::model]`). Defini-lo explicitamente mantém
cada braço `MorphedByMany` honesto sobre qual família ele escaneia.

### Válvula de escape: métodos de relação escritos à mão

As relações declaradas em `relations = { ... }` são as únicas que o
dispatcher de eager-load (e `with`, `with_count`, etc.) conhece. Se
uma relação é muito atípica para a forma da macro - por exemplo uma
consulta que agrega através de dois pivots, ou uma visão tipada de
uma tabela de cache desnormalizada - você pode omiti-la de
`relations = { ... }` e escrever um impl inerente simples:

```rust
impl User {
    /// Posts que este usuário escreveu OU em que está marcado.
    /// Atravessa duas relações e por isso não é expressável como
    /// uma única declaração `relations = { ... }` - escrito à mão.
    pub async fn posts_touched(&self) -> Result<Vec<Post>, FrameworkError> {
        let authored: Vec<Post> = self.posts().get().await?;
        let tagged:   Vec<Post> = /* ...consulta customizada... */;
        // ...merge + dedupe...
        Ok(/* ... */)
    }
}
```

Esses métodos perdem suporte a eager-load -
`User::with(["posts_touched"])` vai dar erro porque o dispatcher não
tem um braço para `posts_touched`. As declarações dentro da macro
continuam sendo o caminho que o framework sabe eager-load, contar,
agregar, e filtrar por predicado.

### Restrições da v1

Um punhado de coisas que a superfície v1 ainda não faz. Cada uma
também é documentada no seu local de declaração - reunidas aqui só
para visibilidade:

- **IDs de morph são só `i64`.** `MorphTo::morph_id` é fixo em
  `i64`, então todo model usado como alvo de `MorphTo` precisa
  declarar uma chave primária `i64`, e a coluna `<name>_id` da
  tabela filha também precisa ser `i64`. FKs de morph em String /
  UUID-como-string são v2.
- **Sem eager loading aninhado através de `MorphTo`.** O enum por
  família erase o tipo do filho, então um caminho com ponto como
  `with(["commentable.user"])` não consegue fazer tail-recursion - o
  dispatcher retorna um erro tipado. Resolva por família fazendo
  match no enum e chamando `with(["user"])` em cada variante
  individualmente.

## Eager loading

Eager loading evita consultas N+1. Em vez de `posts.len()` consultas
para buscar os posts de cada usuário, o Suprnova emite UMA consulta
por relação de nível superior, independente de quantas linhas de pai
são carregadas.

A superfície completa - lista plana, caminhos aninhados, count,
agregados, e eager loads filtrados por predicado - é alcançada
através dos helpers emitidos por `#[suprnova::model]` em cada model:

```rust
// Relação única:
let users = User::with(["posts"]).get().await?;
for u in &users {
    for p in u.posts_loaded() { /* ... */ }
}

// Múltiplas relações:
let users = User::with(["posts", "profile"]).get().await?;

// Caminhos aninhados - três consultas (users + posts + comments), sem N+1:
let users = User::with(["posts.comments"]).get().await?;
let p1 = users[0].posts_loaded()[0];
let comments = p1.comments_loaded();

// Aninhamento mais profundo funciona como esperado:
let users = User::with(["posts.comments.author"]).get().await?;

// Count junto com as linhas de pai:
let users = User::with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}

// Agregados - Sum / Avg / Min / Max sobre uma coluna de relação. A
// leitura ergonômica é o acessador `<rel>_sum_of(col)` emitido pela
// macro.
let users = User::with_sum(("posts", "views")).get().await?;
let sum: f64 = users[0]
    .posts_sum_of("views")
    .expect("with_sum populated the cache");

// Múltiplos agregados na mesma relação compõem - a chave de cache é
// a forma ampla `<rel>_<kind>_<col>`, então tipos e colunas
// distintos não colidem:
let users = User::with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .with_min(("posts", "id"))
    .get()
    .await?;
let u = &users[0];
let sum = u.posts_sum_of("views").unwrap();   // Some(_) - soma de views
let avg = u.posts_avg_of("views").unwrap();   // Some(_) - média de views
let min = u.posts_min_of("id").unwrap();      // Some(Some(_)) - grupo não vazio
let max = u.posts_max_of("id");               // None - with_max não foi chamado

// Filtra os filhos eager-loaded. A macro emite um helper estático
// tipado `with_where_<rel>(closure)` por relação, para que o tipo
// do parâmetro da closure seja inferido - sem precisar escrever
// `Builder<Post>`:
let users = User::with_where_posts(|q| q.filter("published", true))
    .get()
    .await?;
// O `Builder<User>` retornado encadeia com qualquer outro método de
// builder da consulta base:
let users = User::with_where_posts(|q| q.filter("published", true))
    .filter("active", true)
    .get()
    .await?;
// A forma genérica ainda está disponível - útil quando o nome da
// relação é calculado em runtime - mas você vai precisar nomear o
// tipo alvo na closure:
let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// Cada u.posts_loaded() contém só posts publicados.
```

### Layout do cache

As células de cache `__eager` por linha são chaveadas por:

- `<rel>` (só o NOME da relação) para `with` e `with_count`.
- `<rel>_<kind>_<col>` (por exemplo, `posts_sum_views`) para os
  quatro tipos de agregado - `with_sum` / `with_avg` / `with_min` /
  `with_max`. Essa chave ampla permite que múltiplos agregados na
  mesma relação coexistam na mesma linha sem se sobrescreverem.

| Método                              | Chave de cache            | Tipo da célula de cache   | Valor de grupo vazio |
|-------------------------------------|----------------------|-------------------|-------------------|
| `with(["posts"])`                   | `posts`              | `Vec<Post>`       | `Vec::new()`      |
| `with(["profile"])`                 | `profile`            | `Option<Profile>` | `None`            |
| `with_count(["posts"])`             | `posts`              | `u64`             | `0`               |
| `with_sum(("posts","views"))`       | `posts_sum_views`    | `f64`             | `0.0`             |
| `with_avg(("posts","views"))`       | `posts_avg_views`    | `f64`             | `0.0`             |
| `with_min(("posts","id"))`          | `posts_min_id`       | `Option<f64>`     | `None`            |
| `with_max(("posts","id"))`          | `posts_max_id`       | `Option<f64>`     | `None`            |

A macro emite acessadores correspondentes em cada model:

- `<rel>_loaded()` - para relações de coleção: `&[Post]` (entra em
  panic se a relação não foi eager-loaded). Para relações de valor
  único: `Option<&Profile>`.
- `<rel>_count()` - `u64`. Entra em panic se `with_count(["..."])`
  não foi chamado.
- `<rel>_sum_of(col)` / `<rel>_avg_of(col)` - retornam `Option<f64>`
  (`None` se o `with_sum` / `with_avg` correspondente não foi
  chamado).
- `<rel>_min_of(col)` / `<rel>_max_of(col)` - retornam
  `Option<Option<f64>>`: o `Option` externo é "`with_min` /
  `with_max` foi chamado?", o `Option` interno é "o SQL retornou
  NULL porque o grupo estava vazio?".

Os acessadores são a superfície ergonômica - leia através deles em
vez de acessar `__eager.get_aggregate::<T>(...)` diretamente. Eles
constroem a mesma chave de cache por baixo dos panos via
`eloquent::relations::aggregate_cache_key`.

### Compondo agregados na mesma relação

A chave de cache ampla significa que você pode empilhar tantas
chamadas `with_*` na mesma relação em uma consulta quanto quiser -
sem colisões:

```rust
let users = User::with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .with_min(("posts", "id"))
    .with_max(("posts", "id"))
    .get()
    .await?;

let u = &users[0];
let total_views: f64 = u.posts_sum_of("views").unwrap();
let avg_views:   f64 = u.posts_avg_of("views").unwrap();

// Min/Max são double-Option porque min/max do SQL retornam NULL quando vazio:
match u.posts_min_of("id") {
    None              => panic!("with_min not called"),
    Some(None)        => println!("no posts yet"),
    Some(Some(min))   => println!("smallest post id: {min}"),
}

// O acessador retorna `None` quando o `with_*` correspondente foi pulado:
assert!(u.posts_avg_of("score").is_none()); // nunca chamado com col="score"
```

### Agregados e colunas INTEGER

SUM sobre uma coluna INTEGER cai no cache como `f64`. Os braços do
dispatcher tentam `try_get::<Option<f64>>` primeiro, depois recorrem
a `try_get::<Option<i64>>().map(|n| n as f64)` para que os tipos
COUNT/SUM que preservam INTEGER do SQLite não coajam silenciosamente
para `0.0`. Leia através dos acessadores emitidos pela macro
independente do tipo da coluna de origem.

### Roteamento de predicado de `with_where`

`User::with_where_posts(|q| q.filter("published", true))` aplica
uma closure ao `Builder<Post>` interno ANTES da consulta IN
`filter_in(<fk>, parent_ids)` ser emitida, então só linhas filhas
correspondentes chegam ao cache. A macro emite um helper estático
tipado `with_where_<rel>` por relação declarada, então o tipo do
parâmetro da closure é inferido a partir da assinatura do método.

A forma genérica
`with_where(("posts", |q: Builder<Post>| q.filter("published", true)))`
ainda está disponível - útil quando o nome da relação é calculado em
runtime, ou quando você já tem um `Builder<User>` e quer anexar um
predicado. Ela exige nomear o tipo alvo na closure porque o
predicado passa por um `Box<dyn Any>` e o Rust não consegue inferir
o tipo só a partir do nome da relação. (As regras de orphan do Rust
proíbem a macro de adicionar um método tipado diretamente em
`Builder<User>`, então o atalho tipado só é oferecido no model -
`User::with_where_<rel>` - não como um método de chain do builder.)

Para os tipos polimórficos, o predicado roda contra a consulta da
tabela relacionada - não contra o scan do pivot.

`with_where` é suportado em todo tipo de relação EXCETO `MorphTo`.
O enum por família de MorphTo erase o tipo do filho, então nenhum
`Builder<R>` único cobre todas as variantes. Eager loading aninhado
através de MorphTo também não é suportado na v1 -
`with(["commentable.user"])`, onde `commentable` é um `MorphTo`,
retorna um erro do dispatcher de eager-load recursivo.

### `Collection::load` / `load_missing`

Quando você já buscou linhas e quer eager-load relações depois do
fato:

```rust
use suprnova::Collection;

let mut users: Collection<User> = User::all().await?.into();
users.load(["posts.comments"]).await?;
```

`load_missing` é por linha: cada linha na coleção é particionada
independentemente. Linhas que já têm a relação nomeada em cache
ficam intocadas; linhas que não têm recebem a relação carregada.
Espelha a semântica de `$collection->loadMissing(...)` do Laravel.

Para caminhos aninhados a partição se repete em todo nível. Dado
`load_missing(["posts.comments"])`:

- Linhas sem `posts` em cache recebem o caminho COMPLETO carregado -
  `posts` mais seus `comments`.
- Linhas COM `posts` já em cache recursam nos posts em cache e
  carregam `comments` só nos posts que ainda não têm comments em
  cache.

A mesma partição por linha se repete em todo segmento adicional de
um caminho com ponto mais longo (`"posts.comments.author"` etc.) -
em cada passo só as linhas que faltam aquele segmento recebem o
bulk-load.

## Paginação

Três tipos de paginador compõem sobre `Builder<M>`:

| Método | Retorna | Consultas por página | Use quando |
|--------|---------|------------------|----------|
| `paginate(per_page)` | `LengthAwarePaginator<M>` | 2 (COUNT + LIMIT) | UI precisa da contagem total de páginas |
| `simple_paginate(per_page)` | `Paginator<M>` | 1 (LIMIT + 1) | Tabelas grandes; só botão "Próximo" |
| `cursor_paginate(per_page)` | `CursorPaginator<M>` | 1 (LIMIT + 1) | Infinite scroll; paginação profunda |

Os três implementam `Serialize` com o formato JSON padrão do
Laravel, então eles vão direto para consumidores Inertia / JSON sem
reformatação.

### Length-aware

```rust
use suprnova::LengthAwarePaginator;

let page: LengthAwarePaginator<User> = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;

// page.data: Vec<User>
// page.total: u64 - contagem total de linhas em todas as páginas
// page.last_page: u64 - índice da última página, baseado em 1
// page.current_page: u64
// page.per_page: u64
// page.from / page.to: Option<u64> - limites da janela, baseados em 1
// page.path: Option<String> - URL base opcional para geração de links
```

O parsing do parâmetro de página lê `?page=N` da solicitação ativa
via `Context::query_param`. Para paginar múltiplas listas na mesma
página com suas próprias chaves de query, use `paginate_using`:

```rust
let posts = Post::query().paginate_using("posts_page", 10).await?;
let comments = Comment::query().paginate_using("comments_page", 25).await?;
```

**Formato JSON:**

```json
{
  "data": [...],
  "current_page": 1,
  "last_page": 3,
  "per_page": 10,
  "total": 25,
  "from": 1,
  "to": 10,
  "path": "/api/users"
}
```

`path` é omitido do JSON quando não definido.

### Simple paginate (sem contagem)

`paginate` sempre roda duas consultas - um `COUNT(*)` mais a busca
da página. Em tabelas grandes, o count por si só pode dominar o
tempo da solicitação. `simple_paginate` pula o count por completo;
em vez disso busca `per_page + 1` linhas e informa se existe uma
próxima página através da flag `has_more`:

```rust
use suprnova::Paginator;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;

// page.has_more: bool - havia uma linha extra além de per_page?
// page.current_page, page.per_page, page.data, page.path: como acima.
```

**Formato JSON:**

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true
}
```

### Cursor paginate (keyset)

Cursor paginate é a escolha para infinite scroll, paginação
profunda, ou qualquer lugar onde uma ordem de linha estável com
seeking barato O(1) por página vale mais que uma UI de página
numérica. Bidirecional - lê o parâmetro de query `?cursor=<opaque>`,
caminha para frente ou para trás conforme a direção do cursor, e
emite tanto `next_cursor` quanto `prev_cursor` conforme os vizinhos
da página existem (correspondendo ao `cursorPaginate()` do Laravel).

```rust
use suprnova::CursorPaginator;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;

// page.data: Vec<User>
// page.per_page: u64
// page.next_cursor: Option<String> - cursor opaco para a próxima página (None na última)
// page.prev_cursor: Option<String> - cursor opaco para a página anterior (None na primeira)
// page.path: Option<String>
```

Os cursores são **criptografados e autenticados** via
`CursorPaginator::encode_value` - eles codificam o limite do keyset
(a chave primária do model) mais uma tag de direção, selados com
AES-256-GCM usando o `APP_KEY` do framework. Adulteração produz um
erro 400 ParamParse; o cursor é opaco para o cliente e não pode ser
forjado sem a chave.

A próxima solicitação passa o cursor através de `?cursor=<opaque>`:

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

A paginação por cursor **substitui** qualquer `ORDER BY` já
existente no builder - uma ordem `PK ASC` estável é necessária para
que `gt(boundary)` corte deterministicamente.

**Formato JSON:**

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` e `prev_cursor` estão sempre presentes como chaves
JSON (emitidas como `null` quando ausentes), para que schemas de
cliente possam confiar na presença do campo; `path` é omitido
quando não definido.

### Erros

| Condição | Variante | HTTP |
|-----------|---------|------|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| Cursor inválido (base64, JSON ou HMAC inválidos) | `FrameworkError::Internal` de `Crypt::decrypt_string` | 500 |
| Falha subjacente do BD | `FrameworkError::Database` | 500 |

Falha de autenticação do cursor aparece como `Internal` (não
`ParamParse`) para que um cursor adulterado não vaze informação de
nível de protocolo para o cliente; o corpo da resposta ainda carrega
um motivo legível por humanos.

### Lendo parâmetros de query fora de uma solicitação real

Testes, comandos de console, e workers em background não rodam
dentro de uma solicitação hyper - então `Context::query_param("page")`
retorna `None` e `paginate` recorre à página 1. Testes que precisam
exercitar uma página específica podem instalar um override por
thread:

```rust
use suprnova::context::Context;

#[tokio::test]
async fn paginate_page_2() {
    Context::test_clear_query();
    Context::test_set_query("page", "2");

    let page = User::query().paginate(10).await.unwrap();
    assert_eq!(page.current_page, 2);

    Context::test_clear_query();
}
```

`test_set_query` / `test_clear_query` ficam atrás da feature
`testing` (ativada por padrão em `framework/Cargo.toml`), para que
builds de release nunca vejam essa superfície.

## Chunking e iteração lazy

Sete pontos de entrada de streaming em `Builder<M>` deixam você
processar result sets grandes em memória limitada. Escolha pelo
trade-off:

| Método | Paginação | Seguro para concorrência? | Retorna |
|--------|-----------|------------------|---------|
| `chunk(n, async \|batch\| { ... })` | OFFSET | Não | `Result<(), _>` |
| `chunk_by_id(n, async \|batch\| { ... })` | cursor de PK | **Sim** | `Result<(), _>` |
| `chunk_map(n, async \|batch\| { ... })` | OFFSET | Não | `Collection<U>` |
| `each(async \|row\| { ... })` | OFFSET, tamanho 1 | Não | `Result<(), _>` |
| `lazy()` | cursor de PK, batch 1000 | **Sim** | `LazyCollection<M>` |
| `lazy_by_id(batch_size)` | cursor de PK, batch customizado | **Sim** | `LazyCollection<M>` |
| `cursor()` | Alias para `lazy()` | **Sim** | `LazyCollection<M>` |

### chunk - batches paginados por OFFSET

```rust
use suprnova::{Collection, Model};

User::query().chunk(100, |batch: Collection<User>| async move {
    for user in &batch {
        send_welcome_email(user).await?;
    }
    Ok(())
}).await?;
```

A closure recebe um `Collection<M>` por batch - acesso no formato de
slice (`.iter()`, indexação) funciona diretamente via `Deref`.

`chunk` é paginado por OFFSET e **não é seguro sob inserts
concorrentes**: linhas inseridas antes do offset do próximo batch
são puladas; linhas deletadas antes do offset são processadas duas
vezes (o que quer que tenha deslizado para o lugar delas). Use
`chunk_by_id` para processamento em massa de qualidade de produção
contra tabelas sob carga de escrita.

### chunk_by_id - batches por cursor de PK, concurrent-safe

```rust
User::query().chunk_by_id(500, |batch| async move {
    for user in &batch {
        reindex_user(user).await?;
    }
    Ok(())
}).await?;
```

Cada batch filtra em `WHERE id > last_id ORDER BY id ASC LIMIT n`,
então linhas inseridas em meio à iteração com PKs acima do cursor
caem em um batch posterior (ou são capturadas por uma execução
subsequente) - elas nunca fazem uma linha original ser pulada ou
duplicada.

`chunk_by_id` exige uma chave primária `i64`. Models com PKs
`String` / `Uuid` usam `chunk` com a ressalva do OFFSET.
(Generalizar a forma do cursor para chaves que não são `i64` está na
lista de follow-up.)

### chunk_map - chunk + map por chunk

```rust
let totals: Collection<i64> = Order::query()
    .chunk_map(1000, |batch| async move {
        let sum: i64 = batch.iter().map(|o| o.amount).sum();
        Ok(Collection::from_vec(vec![sum]))
    })
    .await?;
```

Mapeia cada batch através de `f`, concatena a saída mapeada, e
retorna um único `Collection<U>`. Limitado em memória só quando `U`
é estritamente menor que `M` - escolha isto quando você está
produzindo resumos (totais por batch, ids, agregados) em vez de
linhas transformadas.

### each - uma linha por vez, OFFSET

```rust
User::query().each(|user| async move {
    send_welcome_email(&user).await?;
    Ok(())
}).await?;
```

Açúcar sintático para `chunk(1, ...)` - uma consulta por linha. Para
datasets grandes, mude para `lazy()`, que faz batch internamente
(1000 linhas por busca por padrão) enquanto ainda expõe uma linha
por vez ao consumidor.

### lazy / lazy_by_id / cursor - streams

```rust
let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

`lazy()` retorna um `LazyCollection<M>` - um wrapper de stream
`Send` que produz `Result<M, FrameworkError>` por linha. Backpressure
funciona naturalmente: um consumidor lento estaciona no ponto de
`await` e o próximo batch só busca quando o buffer em memória
esvazia.

`lazy()` faz batch via cursor de PK com tamanho padrão de 1000
linhas. Sobrescreva o tamanho do batch com `lazy_by_id(500)`.
`cursor()` é o nome do Laravel e é um alias de custo zero para
`lazy()`.

Mesma restrição de PK `i64` que `chunk_by_id`.

### Eager loads dentro de chunks

Todos os sete pontos de entrada **rejeitam `.with(...)`
antecipadamente**, de forma explícita, com um
`FrameworkError::internal`. O clone entre batches do Builder
descarta o plano de eager-load com tipo erased (seu predicado
`Box<dyn Any>` não é clonável sem afrouxar a API pública), então
respeitar o plano seria inconsistente silenciosamente entre batches.
Reaplique `.with(...)` dentro da closure por chunk quando
necessário - o `Collection<M>` de cada batch compõe com
`load(...)` / `load_missing(...)`:

```rust
User::query().chunk(100, |batch| async move {
    let mut batch = batch;
    batch.load("posts").await?;
    for u in &batch {
        let posts = u.posts_loaded();
        // ...
    }
    Ok(())
}).await?;
```

## Coleções

`Collection<T>` é a coleção no formato Laravel do Suprnova - o tipo
de retorno de `Builder::get` (onde `T` é o model), de `Model::all`,
de `pluck` / `chunk_map`, e de todo outro terminal que produz mais
de uma linha. Ela faz deref para `&[T]`, então call sites existentes
com Vec continuam funcionando sem alterações; a superfície do
Laravel é composta em cima. Esta seção é a superfície do dia a dia;
o índice completo de métodos, a divisão genérico-vs-model, o wrapper
de streaming `LazyCollection<M>`, e as regras de borrow-vs-consume
estão em [Coleções Eloquent](eloquent-collections.md).

### Superfície genérica

Disponível em todo `Collection<T>`, independente de `T`:

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9]);

nums.first();              // Some(&3)
nums.last();               // Some(&9)
nums.len();                // 6
nums.is_empty();           // false
nums.contains(&4);         // true
// Closures de predicado recebem `&&T` - note o double-deref `**n`:
nums.first_where(|n| **n > 3);    // Some(&4)
nums.contains_where(|n| **n > 8); // true
// Para uma contagem, rode o predicado inline: `nums.iter().filter(|n| **n > 2).count()` - 4
```

Transformações consomem `self` e retornam um novo `Collection`:

```rust
let doubled: Collection<i32> = nums.clone().map(|n| n * 2);
let evens:   Collection<i32> = nums.clone().filter(|n| n % 2 == 0);
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(2); // [[3,1],[4,1],[5,9]]
let unique:  Collection<i32> = nums.clone().unique();
let sorted:  Collection<i32> = nums.clone().sort();
```

### Métodos model-aware em `Collection<M>`

Quando `T` é um model, métodos adicionais chaveados por string
roteiam através do acessador `field_value(name)` emitido pela macro:

```rust
let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let by_role: HashMap<String, Vec<User>> =
    users.clone().group_by::<String>("role");
let active: Collection<User> = users.clone().where_eq("active", true);

let total: f64 = users.clone().sum::<f64>("balance");
let avg:   f64 = users.clone().avg::<f64>("balance");
let max:   Option<i64> = users.clone().max::<i64>("login_count");
```

O `pluck_by` baseado em closure é a alternativa tipada - útil quando
o nome do campo de outra forma exigiria um lookup de string que o
sistema de tipos não consegue checar:

```rust
let names: Collection<String> = users.pluck_by(|u| u.name.clone());
```

`field_value(name)` por linha retorna `Option<serde_json::Value>` -
`None` quando o nome da coluna não corresponde a nenhum campo
declarado. Casts customizados que falham ao serializar também
aparecem como `None`. Os métodos chaveados por string pulam essas
linhas silenciosamente; a forma de closure faz short-circuit no
corpo da closure, para que quem chama decida.

### Streaming via `LazyCollection`

Para datasets grandes demais para materializar, `Builder::lazy()` /
`lazy_by_id(n)` / `cursor()` retornam um `LazyCollection<M>` - um
wrapper de `Stream` que busca linhas em batches por cursor de PK.
Veja [Chunking e iteração lazy](#chunking-e-iteração-lazy).

### Eager loading em uma coleção

`Collection::load(["posts"])` / `load_missing(["posts"])` executam o
mesmo dispatch de eager-load que uma chain `Builder::with(...)`
emite, mas contra uma coleção já existente. `load_missing` é por
linha: cada linha na coleção é particionada em baldes de "precisa
carregar" / "já carregado" e só as que faltam recebem o bulk-load.
Veja [Eager loading](#eager-loading).

## Atribuição em massa

### Allowlist `fillable`

```rust
#[model(
    table = "users",
    fillable = ["name", "email"],
)]
pub struct User { /* ... */ }

User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    admin: true,    // descartado silenciosamente em runtime - não está em fillable
}).await?;
```

### Denylist `guarded`

`guarded` é o inverso - todo campo é fillable EXCETO os guarded.
Mutuamente exclusivo com `fillable`; usar os dois ao mesmo tempo é
um erro de compilação da macro.

```rust
#[model(
    table = "posts",
    guarded = ["id", "user_id"],   // todo o resto é fillable
)]
pub struct Post { /* ... */ }
```

### Política padrão

Quando nem `fillable` nem `guarded` são definidos, a política padrão
é `guarded = ["id"]` (ou o que quer que `primary_key = "..."`
resolva) - todo campo é fillable exceto a chave primária. Isso
corresponde ao padrão do Laravel de "todo campo fillable exceto a
PK".

### Válvula de escape `unguarded(closure)`

`unguarded(closure)` desativa o filtro para um bloco:

```rust
use suprnova::eloquent::unguarded;

// Contorna o filtro para um script de migração de dados de execução única:
unguarded(|| async {
    User::create(attrs! {
        name: "Bootstrap",
        email: "boot@example.com",
        admin: true,    // atribuível dentro da closure
    }).await
}).await?;
```

Implementação: um booleano `tokio::task_local!` que o filtro
`Fillable::apply` verifica antes de rodar. Task-local significa que
solicitações concorrentes não são afetadas pelo scope `unguarded` de
outra task.

## Casts

Casts rodam na fronteira entre storage (valor da coluna) e runtime
(campo do model). Cada tipo de cast implementa a trait `Cast`. Casts
embutidos cobrem o conjunto completo do Laravel; usuários registram
casts customizados via a trait. Esta seção é o índice de referência
rápida; o contrato completo por cast - primitivo, temporal,
estruturado, enum, criptografado, hasheado, mais a macro de override
em runtime `casts!` - vive em [Eloquent - casts, acessadores e
mutadores](eloquent-mutators.md).

### Somente explícito

Casts são declarados em `#[model(casts = { ... })]` - não existe
auto-detecção a partir dos tipos de campo. Um campo `prefs: Json`
não se torna implicitamente `AsJson`; você escreve
`casts = { prefs = AsJson }`. Motivo: você deve conseguir ler o
model e saber exatamente o que roda nas fronteiras de storage. Sem
mágica.

### Exemplo

```rust
use suprnova::{model, AsArray, AsBool, AsCollection, AsDate, AsDateTime,
    AsEncrypted, AsEnum, AsObject, AsTimestamp};

#[model(
    table = "users",
    casts = {
        active        = AsBool,
        preferences   = AsArray<String>,
        options       = AsObject<UserOptions>,
        profile       = AsCollection<ProfileField>,
        birthday      = AsDate,
        last_seen_at  = AsDateTime,
        role          = AsEnum<UserRole>,
        api_token     = AsEncrypted,
    },
)]
pub struct User { /* ... */ }
```

### Lista completa de casts do Laravel e mapeamento do Suprnova

| Laravel cast | Cast do Suprnova | Tipo em runtime |
|--------------|---------------|--------------|
| `bool`, `boolean` | `AsBool` | `bool` |
| `int`, `integer` | `AsInt<I>` | `I: PrimInt` |
| `float`, `double`, `real` | `AsFloat` | `f64` |
| `decimal:N` | `AsDecimal<N>` | `rust_decimal::Decimal` |
| `string` | `AsString` | `String` |
| `array` | `AsArray<T>` | `Vec<T>` (codificado em JSON) |
| `object` | `AsObject<T>` | `T: Serialize + DeserializeOwned` |
| `collection` | `AsCollection<T>` | `Collection<T>` |
| `json` | `AsJson<T>` | `T` (coluna JSON bruta) |
| `date`, `date:format` | `AsDate` | `chrono::NaiveDate` |
| `datetime`, `datetime:format` | `AsDateTime` | `chrono::DateTime<Utc>` |
| `immutable_date` | `AsImmutableDate` | `chrono::NaiveDate` |
| `immutable_datetime` | `AsImmutableDateTime` | `chrono::DateTime<Utc>` |
| `timestamp` | `AsTimestamp` | `i64` (epoch Unix) |
| `encrypted` | `AsEncrypted` | `String` (criptografado via `Crypt`) |
| `encrypted:array` | `AsEncryptedArray<T>` | `Vec<T>` (JSON + criptografado) |
| `encrypted:object` | `AsEncryptedObject<T>` | `T` (JSON + criptografado) |
| `encrypted:collection` | `AsEncryptedCollection<T>` | `Collection<T>` |
| `EnumClass::class` | `AsEnum<E>` | `E: EnumString + AsRefStr` |
| `AsArrayObject::class` | `AsArrayObject<T>` | `IndexMap<String, T>` |
| `hashed` | `AsHashed` | `String` (`Hash::make` na escrita; nunca descriptografa) |

22 casts no total. A maioria mapeia um-para-um com o Laravel; o
`AsOptionalDateTime` (usado por `soft_deletes`) é auto-injetado pela
macro quando a coluna de soft delete é `Option<DateTime<Utc>>`.

### Modos de falha do cast criptografado

Os quatro casts `AsEncrypted*` roteiam toda criptografia/descriptografia
através da facade `Crypt` (chaveada por `APP_KEY`). Quando a
descriptografia falha - chave errada, ciphertext truncado, bytes
adulterados, incompatibilidade de tag AEAD - o cast expõe um
`FrameworkError::Internal` claro a partir de `Cast::from_storage`.
Não existe fallback silencioso para lixo:

- Carregar uma linha através de `Model::find` / `Model::query()`
  propaga o erro de descriptografia e (pelo `From<inner::Model>`
  gerado pela macro) entra em panic com `cast from_storage failed -
  corrupt data in database column`. Operadores veem a falha nos
  logs imediatamente; o model nunca carrega um plaintext
  plausível-mas-errado.
- O cast `AsHashed` é unidirecional; ele nunca descriptografa, então
  esse modo de falha não se aplica.

Isso corresponde ao cast `encrypted` do Laravel: um `APP_KEY` errado
contra uma coluna criptografada existente é um erro definitivo,
nunca uma string `null`/vazia silenciosa.

### Rotacionando `APP_KEY`

O Suprnova suporta rotação de chave sem downtime através de um
*ring* de chaves: o `APP_KEY` atual criptografa; uma variável de
ambiente `APP_KEY_PREVIOUS` opcional (separada por vírgulas, da mais
antiga para a mais nova) fornece fallbacks de descriptografia para
dados escritos sob chaves antigas. A criptografia *sempre* usa a
chave atual - chaves anteriores participam só na descriptografia.

Toda descriptografia que recorre a uma chave anterior emite uma
linha `tracing::warn!` contendo o índice da chave anterior. O
payload do log exclui deliberadamente o plaintext e o ciphertext; só
o fato-da-rotação mais uma dica acionável de recriptografia.

**Procedimento de rotação** (sem downtime, seguro para produção):

1. Gere uma chave nova: `suprnova key:generate` (escreve no stdout).
2. Mova a chave antiga para `APP_KEY_PREVIOUS` e defina `APP_KEY`
   para o novo valor:
   ```
   APP_KEY_PREVIOUS=<old_key>
   APP_KEY=<new_key>
   ```
3. Faça deploy. Escritas novas usam a chave nova; linhas existentes
   continuam descriptografando via o fallback da chave anterior.
   Warnings nos logs identificam colunas que ainda dependem de
   `APP_KEY_PREVIOUS`.
4. Rode uma passagem de recriptografia. Para cada model com casts
   criptografados:
   ```rust
   for chunk in User::query().chunk(500).await? {
       for user in chunk {
           // Touch + save reescreve toda coluna com cast sob a
           // chave atual. `Cast::to_storage` sempre busca a
           // entrada atual do ring.
           user.save().await?;
       }
   }
   ```
   Isso é idempotente - linhas já na chave nova simplesmente fazem
   no-op.
5. Quando os logs não mostrarem mais warnings de `APP_KEY_PREVIOUS`
   (dê ao batch + qualquer dado soft-deleted / arquivado uma janela
   generosa), remova `APP_KEY_PREVIOUS` do ambiente e faça redeploy.

**Rotação em múltiplos passos.** Se você rotacionar de novo antes de
terminar a passagem anterior, anexe:
`APP_KEY_PREVIOUS=<oldest>,<previous>`. O ring tenta toda chave
anterior em ordem. A lista tem um limite de 8 entradas - uma cadeia
realista é de 1 a 3 (uma rotação em andamento, talvez um roll
anterior parado) e uma lista mais longa é quase sempre um acidente
de templating de config; exceder o limite falha o boot com um
diagnóstico acionável em vez de descartar silenciosamente uma chave
da qual o operador ainda pode depender.

**Restrições.**

- Uma entrada malformada em `APP_KEY_PREVIOUS` falha o boot de forma
  explícita (igual a um `APP_KEY` malformado) - um segredo
  parcialmente rotacionado nunca deve degradar silenciosamente.
- Mais de 8 entradas em `APP_KEY_PREVIOUS` falha o boot de forma
  explícita - veja [`suprnova::crypto::MAX_PREVIOUS_KEYS`].
- Entradas vazias na lista (por exemplo, vírgulas sobrando de config
  templated) são toleradas como "nenhuma chave neste slot" - não é
  um erro.
- O formato de rede não muda em relação ao layout de chave única
  anterior à rotação: nenhum identificador de chave é embutido no
  ciphertext. O ring tenta descriptografar com cada chave em ordem
  até uma funcionar.

### Override de cast em runtime - `with_casts`

```rust
let users = User::query()
    .with_casts(suprnova::casts! { birthdate = AsDateTime })
    .get()
    .await?;
```

`with_casts` sobrescreve os casts declarados do model pela duração
de uma única consulta - útil quando uma coluna bruta volta de um
join / view / `select_raw` e precisa de uma coerção de tipo
diferente do padrão do model.

### Casts customizados

Casts customizados implementam `Cast`:

```rust
use suprnova::eloquent::casts::Cast;
use suprnova::FrameworkError;

pub struct AsAesGcmJson<T>(std::marker::PhantomData<T>);

impl<T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync> Cast
    for AsAesGcmJson<T>
{
    type Runtime = T;
    type Storage = String;
    fn to_storage(value: &T) -> Result<String, FrameworkError> { /* ... */ }
    fn from_storage(stored: &String) -> Result<T, FrameworkError> { /* ... */ }
}

#[model(casts = { secret = AsAesGcmJson<SecretBundle> })]
pub struct Vault { /* ... */ }
```

A trait `Cast` é distribuída junto com os casts primitivos. Casts
customizados podem usar tanto storage `String` (ao codificar JSON)
quanto qualquer um dos tipos escalares suportados pelo SeaORM
(`i64`, `f64`, `bool`, `Vec<u8>`).

## Acessadores e mutadores

### Acessadores

```rust
#[model(
    table = "users",
    appends = ["full_name"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    // ...
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }
}
```

Quando `user.to_array()` roda (ou `user.to_json()`, que delega para
ele), o acessador `full_name` é chamado e seu valor de retorno é
inserido na saída JSON. Chamar `user.full_name()` a partir do Rust é
só uma chamada de método normal.

### Mutadores

Mutadores rodam antes do storage:

```rust
#[model(
    table = "users",
    fillable = ["first_name", "last_name", "password"],
    mutators = ["password"],
)]
pub struct User { /* ... */ }

impl User {
    #[mutator]
    pub fn set_password(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), suprnova::FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            suprnova::FrameworkError::validation("password", format!("{e}"))
        })?;
        self.password = hash::make(&raw);
        Ok(())
    }
}
```

Chamar `user.password = "secret".into()` atribui o valor bruto
diretamente sem rodar o mutador. Para rodar o caminho do mutador,
chame `user.set_password(json!("secret"))` ou use o caminho JSON
(`user.fill(attrs!{password: "secret"})`), que roteia através do
mutador automaticamente porque `"password"` está listado em
`mutators = [...]`.

### Como o roteamento funciona

- **Serialização (`to_array` → `Value`, `to_json` → `String`)**
  roda acessadores. Todo nome de campo listado em `appends = [...]`
  se torna uma chamada a `self.<name>()`; o valor de retorno é
  inserido na saída JSON. `to_json()` é um wrapper fino:
  `serde_json::to_string(&self.to_array())`.
- **Escritas no estilo fill (`fill`, `create`, `update`)** roteiam
  através de mutadores. Todo nome de campo listado em
  `mutators = [...]` se torna uma chamada a
  `self.set_<field>(value)` em vez de atribuição direta.

As macros de nível de função `#[accessor]` e `#[mutator]` emitem
entradas de registro que os caminhos de serialização / fill da
macro percorrem.

### Valores malformados são erros, não padrões

Um valor que não consegue decodificar para o tipo do seu campo
falha a escrita e nomeia o campo:

```rust
let err = user.fill(attrs! { age: "not a number" }).unwrap_err();
// ValidationError { field: "age", message: "could not decode the
// supplied value: invalid type: string \"not a number\", expected i32" }
```

O model permanece intocado - um `fill` rejeitado não aplica nada.

Dois casos próximos se comportam diferente, de propósito:

- Uma **coluna desconhecida** ainda é pulada silenciosamente, igual
  ao `$model->fill()` do Laravel. Não conhecer uma coluna não é a
  mesma coisa que receber um valor quebrado para uma que você
  conhece.
- Uma coluna excluída por `fillable` / `guarded` é descartada pelo
  filtro de atribuição em massa *antes* da decodificação, então um
  valor malformado para um campo que quem chama não pode definir
  também é silencioso. Retornar erro ali revelaria a um chamador não
  autorizado quais colunas existem.

Widening numérico não é um erro de tipo: um inteiro JSON decodifica
normalmente para um campo `f64`.

> Antes da v0.8.0 um valor malformado era silenciosamente
> substituído pelo `Default` do campo e a chamada retornava `Ok` -
> `fill(attrs!{ age: "abc" })` definia `age = 0` e reportava
> sucesso. Se você dependia dessa coerção, valide ou converta antes
> de chamar `fill`.

### Hidden / visible

```rust
#[model(
    table = "users",
    hidden = ["password", "remember_token"],
)]
pub struct User { /* ... */ }
```

`hidden = [...]` é uma denylist - toda coluna exceto as listadas é
serializada. `visible = [...]` é a forma inclusiva - só as listadas
são serializadas. Mutuamente exclusivos em tempo de compilação.

## Timestamps

Quando as colunas `created_at` e `updated_at` existem, a macro as
detecta automaticamente e ativa o rastreamento de timestamps:

- `created_at` é definido como `Utc::now()` em `save()` para linhas
  novas.
- `updated_at` é definido como `Utc::now()` em todo `save()`.

A auto-detecção é conservadora: se a struct tem só uma das duas
colunas, a macro retorna erro para que um erro de digitação
(`craeted_at`) não desative timestamps silenciosamente. Defina
`timestamps = false` para desativar por completo.

### Desativando timestamps automáticos

```rust
#[model(table = "audit_logs", timestamps = false)]
pub struct AuditLog {
    pub id: i64,
    pub event: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    // Sem campo updated_at - mas timestamps = false também silencia
    // o erro `only one column found` da macro.
}
```

### `touch()` - dá bump em updated_at sem outras mudanças

```rust
user.touch().await?;
```

`touch()` emite `UPDATE table SET updated_at = ? WHERE pk = ?` -
atômico, sem read-modify-write. A macro emite um impl `Touchable`
em todo model com timestamp.

### Touch do pai

```rust
#[model(
    table = "comments",
    touches = ["post"],
    timestamps,
)]
pub struct Comment {
    pub id: i64,
    pub post_id: i64,
    // ...
}
```

A lista `touches = [...]` é analisada e armazenada no model como
uma const `TOUCHES`. O hook pós-save que chamaria automaticamente
`self.post().touch().await?` depois de salvar um comentário ainda
não está conectado - por ora, chame o `.touch()` do pai
explicitamente a partir de um observer ou do seu handler. O metadado
está no lugar para que a troca futura seja uma mudança de
comportamento, não uma mudança de API.

### Formato

Sempre ISO 8601 com UTC. Sem override de
`Model::$timestampsFormat` (conforme a tabela de divergência do
Eloquent - interoperabilidade com o frontend vem primeiro;
formatação de locale pertence à camada de i18n).

## Observers e eventos de ciclo de vida

Todo model passa por um ciclo de vida fixo de 16 eventos ao se mover
através dos caminhos `create` / `save` / `update` / `delete` /
`restore` / `replicate` / consultas do Builder. Listeners podem
enganchar em cada evento para logar, auditar, gerar efeito
colateral, validar, ou cancelar a operação em andamento.

### Os 16 eventos de ciclo de vida

Os eventos se dividem em dois grupos por cancelabilidade:

**Canceláveis (5)** - disparam ANTES da escrita no banco de dados.
Um listener que retorna `EventResult::cancel("reason")` aborta a
operação com `FrameworkError::bad_request(reason)`.

| Evento       | Quando                                      | Payload                                                 |
|-------------|-------------------------------------------|---------------------------------------------------------|
| `Saving`    | Antes de `create` e `save`           | `Arc<Mutex<Attrs>>` + `is_creating: bool`               |
| `Creating`  | Antes de `create`                           | `Arc<Mutex<Attrs>>`                                     |
| `Updating`  | Antes de `save` / `update` em linha existente  | Snapshot do model antes do update + `Arc<Mutex<Attrs>>`         |
| `Deleting`  | Antes de `delete` (soft ou hard)            | Model + `is_force: bool` (force-delete em soft-delete)  |
| `Restoring` | Antes de `restore` em model soft-delete     | Model                                                   |

**Não-canceláveis (11)** - disparam DEPOIS da operação. Erros de
listener se propagam mas não conseguem parar uma escrita que já foi
concluída.

| Evento           | Quando                                              | Payload                          |
|-----------------|---------------------------------------------------|----------------------------------|
| `Retrieving`    | Uma vez por consulta do Builder, antes da chamada ao BD        | None                             |
| `Retrieved`     | Uma vez por linha retornada por uma consulta do Builder          | Model                            |
| `Created`       | Depois de um `create` bem-sucedido                         | Model                            |
| `Updated`       | Depois de um `save` / `update` bem-sucedido                | Snapshots anterior + atual     |
| `Saved`         | Depois de `create` e `save`                    | Model                            |
| `Deleted`       | Depois de um `delete` bem-sucedido                         | Model + `is_force: bool`         |
| `Trashed`       | Depois de soft-delete (NÃO force-delete)              | Model                            |
| `Restored`      | Depois de um `restore` bem-sucedido                        | Model                            |
| `Replicating`   | Durante `replicate` / `replicate_except`, antes do retorno (NÃO `replicate_into` - por tipo de origem) | Source + `Arc<Mutex<replica>>` (mutável) |
| `ForceDeleting` | Antes de `force_delete` em model soft-delete        | Model                            |
| `ForceDeleted`  | Depois de um `force_delete` bem-sucedido                   | Model                            |

A divisão cancelável / não-cancelável espelha o par de hooks
`creating` vs `created` do Laravel. `Saving` dispara tanto para
insert quanto para update - sobrescreva esse quando o comportamento
é idêntico nos dois caminhos e discrimine via `is_creating`.

`Replicating` é o único hook não-cancelável que entrega uma
referência mutável (a replica é `Arc<Mutex<M>>`). Use-o para limpar
timestamps, regenerar UUIDs, resetar auto-incrementos, etc. antes
que o clone seja retornado a quem chama.

### Observers vs listeners brutos

Duas formas de enganchar em eventos de ciclo de vida:

1. **Listeners brutos** - chame
   `EventFacade::listen::<Created, _>(Arc::new(MyListener))` para
   cada evento que você quer, um impl por evento. Este é o
   mecanismo subjacente; observers rodam em cima dele.

2. **Observers** - empacotam todos os 16 hooks sob uma trait. A
   macro vê quais métodos o usuário sobrescreveu e registra
   exatamente esses. Este é o caminho recomendado para qualquer
   conjunto não trivial de hooks.

```rust
use async_trait::async_trait;
use suprnova::eloquent::attrs::Attrs;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::observers::Observer;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]   // <- DEVE preceder #[async_trait]
#[async_trait]
impl Observer<User> for AuditObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if attrs.get("email").is_none() {
            return EventResult::cancel("email is required");
        }
        EventResult::ok()
    }

    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user.id = user.id, "user created");
        Ok(())
    }
}
```

Todo método da trait tem um no-op padrão, então o bloco impl contém
só os eventos com os quais você se importa. A macro identifica
overrides por correspondência de nome contra o conjunto fechado de
16 métodos; métodos que você não sobrescreve não registram nenhum
listener.

### Ordenação obrigatória dos attributes

`#[suprnova::observer(M)]` DEVE aparecer ACIMA de `#[async_trait]`:

```rust
#[suprnova::observer(User)]   // externo - roda primeiro, vê as fns async brutas
#[async_trait]                // interno - reescreve assinaturas de fn async
impl Observer<User> for AuditObserver { /* ... */ }
```

Macros de attribute expandem de fora para dentro. `async_trait`
reescreve toda `async fn` em uma forma de poll-fn
`Pin<Box<dyn Future>>` sem açúcar sintático; se `#[async_trait]`
rodasse primeiro, a correspondência de nome da macro do observer
contra os 16 nomes de método da trait não encontraria nada e
emitiria zero listeners silenciosamente.

### Quatro caminhos de registro

| Caminho                                         | Quando usar                                         |
|----------------------------------------------|-----------------------------------------------------|
| `#[suprnova::observer(M)]` (inventory)       | Observer estático conhecido em tempo de compilação. Se auto-instala no boot. |
| `#[model(observers = [Foo, Bar])]`           | Documentação + validação em tempo de compilação de que os tipos listados resolvem. NÃO registra por si só. |
| `Model::observe(MyObs).await`                | Registro em runtime. Conduzido manualmente; útil quando o registro depende de config. |
| `EventFacade::listen::<events::Created, _>(...)` | Nível mais baixo - um evento por vez. Use quando um observer parece pesado demais. |

O attribute `observers = [...]` em `#[model]` é um marcador de
documentação. Ele compila para um bloco
`const _: fn() = || { let _ = ::std::any::type_name::<T>; ... };`
que prova que cada tipo listado resolve para um item Rust real;
erros de digitação aparecem no local de declaração do model. A
instalação de fato é via o caminho de inventory - o attribute
`#[observer(M)]` em `Foo` é o que inscreve `Foo` para
auto-instalação.

### Inicialização

Chame `bootstrap_observers()` uma vez na inicialização para
esvaziar o inventory e instalar todo observer registrado com
`#[observer(M)]`:

```rust
suprnova::eloquent::observers::bootstrap_observers().await?;
```

O esvaziamento é idempotente para o caminho de inventory - a
closure de instalação de cada observer é controlada por um
`AtomicBool` por tipo (emissão de macro do T2b), então chamar
`bootstrap_observers()` duas vezes não registra em duplicidade.

O shim de runtime `Model::observe(MyObs)` NÃO é controlado. Chamá-lo
duas vezes registra dois conjuntos de listener, igual à semântica
manual de `Model::observe(MyObs::class)` do Laravel. Se um observer
conduzido manualmente também tem `#[observer]`, o adaptador de
inventory dispara além dos instalados manualmente.

### Cancelando a partir de um observer

Os cinco hooks canceláveis retornam `EventResult`. Para abortar a
operação, retorne `EventResult::cancel("reason")`:

```rust
#[suprnova::observer(Subscription)]
#[async_trait]
impl Observer<Subscription> for PolicyObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if let Some(plan) = attrs.get("plan") {
            if plan == "blocked" {
                return EventResult::cancel("plan is blocked");
            }
        }
        EventResult::ok()
    }
}
```

O motivo do cancelamento aparece como
`FrameworkError::bad_request(reason)` a partir de
`Subscription::create`. A linha nunca chega ao banco de dados -
cancel é um abort verdadeiro, não um "delete depois do fato".

Múltiplos observers podem registrar hooks canceláveis no mesmo
model; qualquer um deles que retornar `Cancel` interrompe a
operação. A ordem é a ordem de inscrição no inventory (na prática, a
ordem de link).

### Múltiplos observers em um model

Múltiplos impls de `Observer<M>` disparam todos para o mesmo
evento - o dispatch do EventFacade faz fan-out para todo listener
registrado em vez de escolher um:

```rust
#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for AuditObserver { /* ... */ }

#[suprnova::observer(Comment)]
#[async_trait]
impl Observer<Comment> for NotifyObserver { /* ... */ }

// Comment::create(...) dispara AuditObserver::created E NotifyObserver::created.
```

Isso corresponde à semântica de fan-out do Laravel e é a propriedade
estrutural por trás do padrão "decompor hooks por
responsabilidade": um `AuditObserver` só sabe sobre auditoria, um
`NotifyObserver` só sabe sobre notificações, e a declaração do model
não se importa com quantos observers se conectam.

### `Model::observe()` manual

Toda struct `#[suprnova::model]` recebe um shim `observe<O>()` por
model. Chame-o no boot para registro dinâmico:

```rust
#[derive(Clone)]
struct MyObs;

#[async_trait]
impl Observer<User> for MyObs { /* ... */ }

// Em runtime:
User::observe(MyObs).await;
```

O bound `O: Clone + 'static` do shim é o que permite ao framework
entregar um clone novo do observer para cada um dos 16 listeners
adaptadores internos. Os 16 adaptadores de listener se instalam em
toda chamada - os padrões da trait tornam métodos não sobrescritos
no-ops baratos.

### Restrições

- **A versão via macro exige que o bloco impl use nomes de método
  simples correspondendo aos 16 hooks da trait.** Métodos
  renomeados, padrões suprimidos por `#[allow]`, e corpos
  condicionados por `#[cfg]` ficam fora da correspondência de nome e
  não registram listeners.

- **Structs de observer que a macro inspeciona precisam ser
  zero-sized** (sem campos) na v1. A macro constrói o observer via
  `let obs = MyObserver;` dentro de cada adaptador. Observers com
  estado (carregando `Arc<Inner>`) precisam do caminho de runtime
  `Model::observe()`, que recebe o observer por valor e o clona em
  cada adaptador.

- **Isolamento de teste: use tipos de model únicos por cenário.** O
  EventDispatcher global do processo significa que listeners
  instalados para `User` são visíveis para todo teste no mesmo
  binário. Tipos de model únicos por teste (`T2Comment`,
  `T2Subscription`, …) mantêm o vazamento entre testes fora das
  asserções de contador. Os testes de integração
  `eloquent_observers.rs` exercitam esse padrão.

## Prunable

O Laravel traz uma trait `Prunable` que deixa um model declarar um
scope de linhas para deletar em uma agenda. O Suprnova espelha isso
com duas traits e um comando de console.

### Declarando um pruner

```rust
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::eloquent::Prunable;

#[suprnova::prunable]
#[async_trait]
impl Prunable for ExpiredSession {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "expires_at",
            "<",
            (Utc::now() - Duration::days(30)).to_rfc3339(),
        )
    }
}
```

### `MassPrunable` - variante de bulk-delete

Para tabelas de alto volume (audit logs, request logs, entradas de
cache expiradas), `MassPrunable` pula eventos por linha e roda uma
única instrução `DELETE WHERE …`:

```rust
use suprnova::eloquent::MassPrunable;

#[suprnova::prunable]
#[async_trait]
impl MassPrunable for AuditLog {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "created_at",
            "<",
            (Utc::now() - Duration::days(365)).to_rfc3339(),
        )
    }
}
```

### Disparando pruning

Rode via o console por projeto (para o qual `app/cmd/main.rs` chama
`suprnova::console::dispatch_argv`, depois de `db:seed` e dos outros
built-ins):

```bash
suprnova model:prune                          # faz prune de todo tipo registrado
suprnova model:prune --model=ExpiredSession   # filtra para um model
suprnova model:prune --pretend                # dry run; loga o que seria deletado
```

Programaticamente os runners estão em
`suprnova::eloquent::{prune_all, prune_all_dry, prune_one}`.

### Hook de pruning

`Prunable::pruning(&self)` dispara antes de cada delete de linha
para que o usuário possa rodar efeitos colaterais (limpar arquivos
associados, fazer fan-out de eventos, etc.). O impl padrão é vazio.
`MassPrunable` pula esse hook por definição - deletes em massa não
enumeram linhas.

### Comportamento de cascade

**Pruning NÃO faz cascade automático para linhas relacionadas.** Um
impl `Prunable` ou `MassPrunable` em `User` deleta linhas de user;
seus `posts`, entradas de pivot `role_user`, `comments`
polimórficos, etc. ficam ÓRFÃOS, com colunas de FK apontando para o
usuário agora deletado.

Isso corresponde ao contrato do Laravel: a limpeza de relação é
trabalho do usuário. Duas formas limpas de lidar com isso:

1. **Cascade de FK no nível do banco de dados** - declare `ON
   DELETE CASCADE` (ou `ON DELETE SET NULL`) na constraint de
   foreign key quando você escrever a migration. O engine do BD
   cuida do cascade de graça, sem código Rust por linha.

2. **Hook por linha** - implemente `Prunable::pruning(&self)` para
   deletar filhos antes que a linha do pai seja descartada. O hook
   dispara dentro da mesma operação lógica do delete do pai, então a
   ordenação consistente é garantida:

   ```rust
   #[async_trait]
   impl Prunable for User {
       fn prunable() -> Builder<Self> {
           Self::query().filter_op("deleted_at", "<", thirty_days_ago())
       }

       async fn pruning(&self) -> Result<(), FrameworkError> {
           // Deleta posts.
           Post::query().filter("user_id", self.id).get().await?
               .into_iter()
               .map(|p| p.delete());
           // Faz detach dos pivots de role.
           self.roles().sync(Vec::<i64>::new()).await?;
           Ok(())
       }
   }
   ```

`MassPrunable` é baseado em conjunto - `pruning()` não dispara. Use
`Prunable` simples sempre que precisar de cascade. O framework não
vai emitir silenciosamente um DELETE por linha quando você opta por
`MassPrunable`; o trade-off é documentado de forma evidente.

### Mecanismo de registro

O registro de pruner usa o mesmo padrão de inventory que observers,
commands, e supervisors. O attribute `#[suprnova::prunable]` no
bloco `impl Prunable for T { ... }` se auto-registra via
`inventory::submit!` em tempo de compilação. Sem arquivo de config
central; adicionar um novo tipo prunable é um attribute.

## Roteamento multi-conexão

Apps de produção regularmente precisam de mais de uma conexão de
banco de dados - o caso canônico é uma read replica para analytics +
a primária para escritas, mas a superfície generaliza para qualquer
conexão nomeada (BD de reporting, BD de archive, shard por tenant).

### Registrando uma conexão

Chame `DB::register_named(name, config)` no boot para toda conexão
não padrão com a qual seu app conversa:

```rust
DB::register_named(
    "reporting",
    DatabaseConfig {
        url: env::var("REPORTING_DATABASE_URL")?,
        max_connections: Some(20),
        ..Default::default()
    },
).await?;
```

Dois nomes são reservados: `__primary__` faz short-circuit do
registro para `DB::connection()`, e `__read_replica__` inclui a
conexão no roteamento automático de split leitura-escrita - veja
abaixo.

### Opt-in por consulta: `Model::on(name)`

`Model::on("reporting")` retorna um `Builder<M>` pré-configurado
para rotear através da conexão nomeada:

```rust
let totals = Order::on("reporting")
    .order_by_desc("total")
    .limit(100)
    .get()
    .await?;
```

`on(...)` tem scope de solicitação - só afeta o builder encadeado. A
próxima chamada simples a `Order::query()` resolve através do
padrão.

### Padrão por model: `#[model(connection = "...")]`

Quando um model sempre vive em uma conexão, declare o padrão no
attribute:

```rust
#[model(table = "events", connection = "events_db")]
pub struct Event { /* ... */ }
```

Toda chamada `Event::query()` / `Event::create()` / `Event::find()`
roteia através de `events_db` sem precisar do override `.on(...)`
por consulta. Um `.on(...)` explícito em um builder ainda vence.

### Split leitura-escrita

Registrar uma conexão sob o nome reservado `__read_replica__` inclui
todo model no roteamento automático: métodos de leitura (`first` /
`get` / `find` / `count` / `paginate` / `chunk` / os walkers
conduzidos por closure) fluem através da replica; escritas (`save` /
`create` / `update` / `delete` / `force_delete` / `replicate` /
`attach` / `detach` / `sync` / `increment` / `decrement`) fluem
através da primária.

`Model::on_write_connection()` retira um único builder da replica -
útil quando a consistência read-your-writes importa (por exemplo,
imediatamente depois de um `save`, antes que a replicação se
atualize).

### Precedência de roteamento

A chain de dispatch roda toda operação através de
`ExecutorChoice::resolve_read` ou `resolve_write`. A ordem é:

1. **Uma transação ativa vence absolutamente.** Dentro de
   `DB::transaction`, toda leitura E toda escrita usa a conexão da
   tx. `on(name)` é IGNORADO dentro de uma transação - a tx é
   vinculada a uma conexão física específica. O SeaORM não consegue
   iniciar uma transação em uma conexão e rodar instruções contra
   outra.
2. **`on(name)` por builder.** Definido via `Model::on(name)` /
   `Builder::on(name)`. Vence o padrão do model e o split de
   leitura/escrita.
3. **`Model::on_write_connection()`.** Força a primária mesmo
   quando a operação de outra forma rotearia para a replica.
4. **Padrão `#[model(connection = "...")]` por model.** Vence o
   split de leitura/escrita para as próprias consultas do model.
5. **Split de leitura/escrita.** Quando `__read_replica__` está
   registrado, métodos de leitura roteiam para lá; escritas roteiam
   para a primária.
6. **Padrão.** `DB::connection()` - a primária, a que `DB::init()`
   configurou.

### Ressalvas

- Transações ativas IGNORAM `on(name)` (veja o §1 acima). Se você
  precisar de uma escrita em uma conexão diferente em meio a uma tx,
  você não pode - a tx é vinculada a uma conexão.
- Os nomes reservados `__primary__` e `__read_replica__` não podem
  ser usados como nomes de conexão do usuário. `DB::register_named`
  retorna um erro em caso de colisão.
- O lag da replica é SEU problema. O Suprnova não faz retry na
  leitura nem recorre à primária quando a replica está desatualizada;
  se você precisar de read-your-writes depois de um save, use
  `Model::on_write_connection()` explicitamente.

## Replicação

`Model::replicate()` retorna uma cópia não salva do model com a
chave primária resetada para seu padrão. Útil para UX de "duplicar
este registro", em que o usuário quer partir de uma linha existente.

```rust
let template: User = User::find_or_fail(42).await?;
let mut copy = template.replicate().await?;  // id resetado para o padrão
copy.email = "fresh@example.com".into();
copy.save().await?;  // INSERT, não UPDATE
```

`replicate` é **async** no Suprnova (diverge do Laravel) porque
dispara o evento `Replicating` - listeners de `Saving` / `Created` /
etc. podem mutar a replica antes que ela seja retornada. Veja
[Evento Replicating](#evento-replicating) para o contrato de mutação
de listener.

### `replicate_except`

Descarta campos nomeados da replica:

```rust
let copy = order.replicate_except(["payment_token", "stripe_id"]).await?;
```

Campos listados recorrem ao impl `Default` do model - `String`s se
tornam `""`, `Option`s se tornam `None`, etc. Use isto para colunas
sensíveis que a linha replicada não deveria levar adiante.

### `replicate_into::<T>` entre tipos

A divergência do Suprnova - o Laravel não consegue porque o PHP não
tem tipos. `replicate_into::<T>()` faz a ponte para um tipo irmão
via `serde_json`:

```rust
let order: Order = Order::find_or_fail(42).await?;
let invoice: Invoice = order.replicate_into::<Invoice>().await?;
invoice.save().await?;
```

Campos com nomes correspondentes + tipos compatíveis com serde são
levados adiante; campos que não correspondem em qualquer um dos
lados são descartados silenciosamente. `T` precisa implementar
`Default` para que campos não preenchidos tenham um valor. A
replicação entre tipos NÃO dispara `Replicating` (o evento carrega
um `&mut Self` - não há como endereçar `T` através dele). Se você
precisar de mutação orientada a evento, replique o mesmo tipo
primeiro e depois materialize `T` a partir do resultado.

## Depuração - dump e dd

Dois auxiliares de depuração interativos em todo `Builder<M>`:

```rust
// Loga SQL + bindings via tracing::info!, retorna self.
let users = User::query()
    .filter("active", true)
    .dump()                       // → linha de log, o builder continua
    .order_by_desc("created_at")
    .get()
    .await?;

// Loga em tracing::error!, depois entra em panic com o SQL na mensagem.
User::query().filter("id", 1).dd();  // - !
```

`dump` é encadeável; `dd` retorna `!` (nunca retorna - o panic é o
contrato). Ambos espelham exatamente `Builder::dump()` /
`Builder::dd()` do Laravel.

Ambos os helpers recorrem ao dialeto SQLite quando nenhuma conexão
de BD viva está vinculada (corresponde ao fallback de
`to_sql_with_bindings`), então continuam úteis em um REPL ou em um
teste sem `TestDatabase`.

A mensagem de panic usa o prefixo literal `eloquent dd:` para que
testes possam fazer assert contra ela:

```rust
#[test]
#[should_panic(expected = "eloquent dd")]
fn dd_panics_with_sql_in_message() {
    User::query().filter("id", 1).dd();
}
```

**Nunca faça commit de `dd()` em um caminho de código de
produção.** É um auxiliar de depuração interativo; o panic na saída
é o ponto principal. `dump()` é mais seguro (só loga), mas usá-lo
demais em hot paths vai encher seus logs - remova-o antes de fazer
push.

Se você quiser o SQL sem os efeitos colaterais, use os helpers que
não logam:

- `Builder::to_sql()` - retorna o SQL renderizado como uma `String`.
- `Builder::to_sql_with_bindings()` - retorna
  `(String, Vec<SeaValue>)`.
- `Builder::to_sql_for(backend)` - renderiza para um dialeto
  explícito (depuração cross-backend).

## Testando models

Testes instanciam um banco de dados real via `TestDatabase`, que
registra a conexão no container por teste, para que qualquer coisa
que chame `DB::connection()` dentro do SUT resolva para o BD de
teste.

### Dois pontos de entrada

- **`TestDatabase::fresh::<MyMigrator>().await`** - roda toda
  migration que o migrator de produção roda. Use isto para testes de
  dogfood no nível do app, em que você quer que o schema de teste
  corresponda exatamente ao que `suprnova migrate` produz.
- **`TestDatabase::sqlite_memory().await`** - abre um banco de dados
  SQLite em memória SEM aplicar nenhuma migration. Use isto para
  testes de unidade no nível do framework, em que você quer controle
  preciso da forma da coluna via
  `db.execute_unprepared("CREATE TABLE …")` por teste.

### Padrão de dogfood no nível do app

```rust
use app::migrations::Migrator;
use app::models::users::User;
use suprnova::testing::TestDatabase;
use suprnova::{attrs, Model};

#[tokio::test]
async fn user_lifecycle() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
        password: "hashed",
    }).await.unwrap();

    assert!(alice.id > 0);

    alice.delete().await.unwrap();
    assert!(User::find(alice.id).await.unwrap().is_none(),
        "default scope hides soft-deleted rows");
}
```

O binding `_db` mantém o `TestDatabase` durante todo o teste -
dropá-lo desmonta o container e libera a conexão SQLite em memória.
Não o sombreie para `_` ou a conexão desaparece antes do SUT rodar.

### Padrão de forma no nível do framework

```rust
use suprnova::testing::TestDatabase;
use suprnova::{attrs, model, Model};

#[model(table = "t_users", timestamps = false)]
pub struct TUser { pub id: i64, pub name: String }

#[tokio::test]
async fn shape_test() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT)"
    ).await.unwrap();

    let u = TUser::create(attrs! { name: "Alice" }).await.unwrap();
    assert_eq!(u.name, "Alice");
}
```

### Padrões-chave

- `TestDatabase::fresh::<MyMigrator>()` para testes no nível do app
  com o schema de produção. `TestDatabase::sqlite_memory()` para
  testes de forma no nível de unidade.
- Use `TestContainer::bind` (NÃO `App::bind`) para qualquer
  singleton que o teste muta - overrides de registro global entram
  em race em execuções paralelas. O construtor de `TestDatabase`
  cuida do binding do BD para você.
- Mantenha declarações de model no scope do módulo, não dentro de
  fns de teste. A macro emite um `mod` interno cujo `use super::*;`
  só vê os imports de nível superior do arquivo - declarar um model
  dentro de uma função de teste quebra a resolução de tipo do
  SeaORM.

## Caindo para o SeaORM

Três válvulas de escape mantêm o SeaORM acessível de dentro da
camada Eloquent:

1. **O módulo interno** - `user::Entity`, `user::Column`,
   `user::ActiveModel`, `user::Model`. A macro emite esses para todo
   model; são tipos do SeaORM que você pode usar diretamente. Veja
   [Layout do módulo do model](#layout-do-módulo-do-model) para o
   layout completo e quando acessá-lo.
2. **Conversões `From`** - `From<user::Model> for User` e
   `From<User> for user::Model` fazem a ponte entre linhas no
   formato SeaORM (colunas tipadas por storage) e linhas no formato
   Eloquent (colunas tipadas em runtime). Útil quando você quer
   emitir uma consulta SeaORM e converter o resultado para a forma
   Eloquent, ou vice-versa.
3. **Os tipos do SeaORM com alias do Suprnova** - todo tipo do
   SeaORM que um consumidor tocaria é reexportado sob
   `suprnova::*`. Você não deveria precisar de `use sea_orm::*` no
   código do app.

```rust
use suprnova::sea_orm::{ColumnTrait, EntityTrait};

// Cai para o SeaORM em meio à consulta - o Eloquent não tem um
// método para isto, mas o SeaORM tem:
let db = suprnova::DB::connection()?;
let users = user::Entity::find()
    .filter(user::Column::Email.like("%@example.com"))
    .all(db.inner())
    .await?;

// Converte para a forma Eloquent:
let eloquent: Vec<User> = users.into_iter().map(User::from).collect();
```

Três válvulas de escape e a ponte From significam que a camada
Eloquent nunca bloqueia você de alcançar o ORM subjacente.

## Migrando de `database::Model`

Código mais antigo pode carregar
`impl suprnova::database::Model for Entity {}` em uma entity SeaORM
escrita à mão. A trait foi renomeada para `EntityExt` para abrir
espaço para a nova trait `Model` - que fica na struct voltada ao
usuário, não na entity do SeaORM.

O caminho de migração recomendado é trocar o tipo para
`#[suprnova::model]`, que dá a você a superfície completa do
Eloquent mais as traits renomeadas `EntityExt` como bônus. Para o
raro caso em que você quer manter a antiga forma de extensão de
Entity do SeaORM, os nomes de trait `EntityExt` / `EntityExtMut`
ainda estão disponíveis sob `suprnova::database::*`. Eles se
comportam exatamente como o antigo `database::Model` se comportava.

## Facade DB - consultas sem model

Algumas tabelas não pertencem a uma struct `#[suprnova::model]`:
audit logs de vida curta, joins de reporting ad-hoc, agregados de
dashboard. Para essas, use a facade `DB`. Duas superfícies ficam sob
ela:

### `DB::table(name)` - construtor de consultas encadeável

`DbTableBuilder` espelha a forma de where / order / limit de
`Builder<M>`, mas retorna linhas como `DynamicRow` (um newtype de
acessador tipado sobre `serde_json::Map<String, Value>`):

```rust
use suprnova::DB;

let rows = DB::table("audit_log")
    .filter("actor_id", 42)
    .filter_op("created_at", ">=", "2026-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

for row in rows.iter() {
    let event: String = row.get_string("event")?;
    let actor_id: i64 = row.get_int("actor_id")?;
    println!("{actor_id}: {event}");
}
```

A superfície completa:

| Método | Retorna | Propósito |
|--------|---------|---------|
| `.select(["id", "event"])` | `DbTableBuilder` | Restringe colunas (padrão `*`) |
| `.filter(col, val)` | `DbTableBuilder` | `WHERE col = ?` |
| `.filter_op(col, op, val)` | `DbTableBuilder` | `WHERE col <op> ?` |
| `.order_by_asc(col) / _desc(col)` | `DbTableBuilder` | Ordenação |
| `.limit(n) / .offset(n)` | `DbTableBuilder` | Janela |
| `.get()` | `Collection<DynamicRow>` | Toda linha correspondente |
| `.first()` | `Option<DynamicRow>` | Primeira linha ou `None` |
| `.count()` | `u64` | `SELECT COUNT(*) ...` |
| `.insert(attrs)` | `i64` | `id` da nova linha |
| `.update(attrs)` | `u64` | Linhas afetadas |
| `.delete()` | `u64` | Linhas afetadas |

**Fronteira de confiança de identificador.** Nomes de tabela, nomes
de coluna, operadores SQL, e direções ORDER BY são interpolados na
string SQL literalmente - eles NÃO são vinculados como parâmetros.
Passe só literais confiáveis, de tempo de compilação, para esses
argumentos. Valores (o lado direito de `filter` / `filter_op`) SÃO
vinculados e são seguros de passar a partir de dados de solicitação.

**Um WHERE vazio em `update` / `delete` opera em toda linha.**
`DB::table("audit_log").delete().await?` truncata a tabela por
design - adicione um `filter` se não é isso que você quer.

**Split de backend no insert.** `RETURNING id` é usado no Postgres e
no SQLite; o MySQL roda o INSERT e depois emite
`SELECT LAST_INSERT_ID() as id` para recuperar o auto-increment.

### `DynamicRow` - acessadores tipados sobre map JSON

`DynamicRow` envolve um `serde_json::Map<String, Value>` e expõe
getters tipados. Cada um retorna `Result<T, FrameworkError>` com uma
mensagem de erro clara em caso de chave ausente ou incompatibilidade
de tipo:

```rust
let event: String     = row.get_string("event")?;
let actor_id: i64     = row.get_int("actor_id")?;
let active: bool      = row.get_bool("active")?;
let prefs: Prefs      = row.get_as("prefs")?;  // qualquer DeserializeOwned
let raw: serde_json::Value = row.get_value("meta")?;
```

Colunas nullable: use `get_optional_*`. Esses distinguem "coluna
ausente" (erro - incompatibilidade de schema) de "coluna presente,
valor null" (`Ok(None)`):

```rust
let score: Option<i64>      = row.get_optional_int("score")?;
let title: Option<String>   = row.get_optional_string("title")?;
```

`DynamicRow` faz deref para `Map<String, Value>`, então iteração e
verificações de existência de chave funcionam naturalmente:

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}
```

### Escapes de SQL bruto

Quando o builder não é suficiente - window functions, CTEs
recursivas, DDL específico de backend - caia para uma string bruta.
Placeholders correspondem ao backend ativo (`$1, $2, ...` para
Postgres, `?` para MySQL + SQLite):

```rust
// SELECT bruto, materializado como DynamicRow.
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) as post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING post_count > ?",
    vec![5i64.into()],
).await?;

// UPDATE / DELETE bruto - retorna linhas afetadas.
let updated = DB::update(
    "UPDATE users SET verified_at = NOW() WHERE id = ANY($1)",
    vec![ids.into()],
).await?;

let deleted = DB::delete(
    "DELETE FROM stale_sessions WHERE expires_at < ?",
    vec![now.into()],
).await?;

// DDL bruto ou instruções sem binding.
DB::statement("CREATE INDEX CONCURRENTLY idx_users_email ON users(email)")
    .await?;

// Instrução genérica com efeito - para INSERT ... ON CONFLICT etc.
let rows = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1) ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec!["page_views".into()],
).await?;
```

Use essas válvulas de escape com moderação - o builder tipado
captura mais erros em tempo de compilação e lê mais limpo na lógica
de negócio. Mas quando você precisar delas, elas estão aqui.

**Pegadinha de coluna de agregado.** Agregados sem tipo como
`SELECT COUNT(*) AS n FROM t` funcionam através do helper `.count()`
do builder, mas podem ser descartados silenciosamente de linhas
brutas de `DB::select` no SQLite - o `JsonValue::from_query_result`
subjacente percorre a informação de tipo por coluna do sqlx, e um
agregado nu não carrega nenhuma. Se você precisar do caminho de
select bruto com agregados, dê à expressão um contexto tipado: use
um wrapper `CAST(... AS BIGINT)` ou leia a coluna com um helper
tipado `DB::table(...).count()` / `.max(...)` que usa `query_one` +
`try_get` por baixo dos panos.

## Existência de relação + atalhos baratos

O Suprnova espelha a família de consultas de existência de relação do
Laravel. Todo método aqui emparelha o nome no formato Laravel com um
alias idiomático em Rust (a convenção permanente de API dupla do
Suprnova).

### Filtros de existência de relação (`has` / `where_has` / `where_belongs_to`)

A família `EXISTS (...)` correlacionada restringe a consulta pai pela
existência (ou ausência, ou contagem) de linhas relacionadas, sem
fazer join da relação no SELECT externo.

```rust
use suprnova::Model;

// Usuários com pelo menos um post.
let users = User::query().has("posts").get().await?;

// Usuários SEM posts.
let empty = User::query().doesnt_have("posts").get().await?;

// Usuários com >= 3 posts (`has("posts", ">=", 3)` do Laravel).
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// Restrição interna via closure - restringe o corpo da subconsulta EXISTS.
let recent = User::query()
    .where_has::<Post, _>("posts", |q| q.filter_op("created_at", ">=", "2026-01-01"))
    .get()
    .await?;

// Atalho de uma coluna - equivalente a `where_has` com uma closure minúscula.
let with_pub = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// Join direto de belongs-to (sem EXISTS - a FK vive nesta tabela).
let posts = Post::query().where_belongs_to("author", author.id).get().await?;
```

Todas as variantes compõem com os companheiros `or_*` e
`*_doesnt_have`:

- `has` / `or_has` / `has_count` / `doesnt_have` / `or_doesnt_have`
- `where_has` / `or_where_has` / `where_doesnt_have` / `or_where_doesnt_have`
- `where_relation` / `where_relation_op` / `or_where_relation`
- `where_belongs_to`

O motor lê os metadados de relação do inventory `RelationEntry` gerado
pela macro: colunas de join, tabelas de pivot e discriminadores de
morph fluem todos automaticamente. Três formatos de subconsulta são
renderizados:

- **Has** - `EXISTS (SELECT 1 FROM child WHERE child.fk = parent.pk)`
- **Pivot** - `EXISTS (SELECT 1 FROM pivot INNER JOIN target ON ... WHERE pivot.parent_fk = parent.pk)`
- **Morph** - formato has/pivot mais `AND target.<morph>_type = '<value>'`

Nomes de relação desconhecidos renderizam a forma de falha segura
(`EXISTS (SELECT 1 WHERE 1 = 0)`), que avalia para `FALSE` e retorna
zero linhas. Um erro de digitação nunca vaza uma varredura de tabela
inteira.

### Divergência de `MorphTo`

O inverso de `MorphTo` do Laravel (`whereMorphedTo`, `whereHasMorph`)
percorre múltiplas tabelas-alvo porque o filho morph carrega um
discriminador `*_type` que escolhe um entre N pais possíveis. O
`MorphTo` do Suprnova faz lowering para um enum por família no momento
da expansão da macro - o tipo alvo é estaticamente um
`<Family>Morph { Variant1(...), ... }`, não uma única tabela SQL. O
motor de existência não consegue renderizar um
`EXISTS (SELECT 1 FROM <table>)` fixo para esse caso porque não há
uma única tabela.

Migração recomendada: faça a verificação de existência no nível do
filho morph. Onde o Laravel escreve:

```php
Comment::whereHasMorph('commentable', [Post::class], fn ($q) => $q->where('published', true))
```

O Suprnova escreve:

```rust
Comment::query()
    .filter("commentable_type", "post")
    .where_has::<Post, _>("commentable_post", |q| q.filter("published", true))
    .get()
    .await?;
```

A forma com tipagem mais estreita dá completion completo da IDE no
builder interno, coisa que o `whereHasMorph` de tipagem frouxa não
consegue.

### Atalhos baratos do builder

```rust
// Filtros de PK.
User::query().where_key(7).first().await?;        // açúcar para filter("id", 7)
User::query().where_key_not(7).get().await?;      // açúcar para filter_op("id", "!=", 7)
// Aliases idiomáticos em Rust: filter_key / filter_key_not.

// Ordenar por created_at.
Post::query().latest().get().await?;              // ORDER BY created_at DESC
Post::query().oldest().get().await?;              // ORDER BY created_at ASC
Post::query().latest_by("published_at").get().await?;  // coluna nomeada

// Correspondência de exatamente um.
let one = User::query().filter("email", e).sole().await?;          // erro em 0 ou >1
let val: i64 = User::query().filter("id", 1).sole_value("views").await?;
let v: i64 = User::query().filter("name", "x").value_or_fail("views").await?;

// Opt-outs de eager-load.
User::query().with(["posts","tags"]).without(["tags"]).get().await?;
User::query().with_only(["posts"]).get().await?;   // limpa o plano primeiro

// Colunas totalmente qualificadas (para joins).
Builder::<User>::qualify_column("name");           // -> "users.name"
Builder::<User>::qualify_columns(["name", "id"]);  // -> ["users.name", "users.id"]
```

### Mutação em massa - `update_all` / `delete_all` / `upsert` / `*_each`

Estes atingem o banco de dados diretamente com uma única instrução e
NÃO disparam eventos de model por linha. Use-os quando estreitar o
scope for suficiente e você não precisar de hooks de ciclo de vida;
para hooks por linha, itere com `.get()` e chame `.update()` /
`.delete()` em cada linha.
`delete_all` sempre mira o `M::TABLE` estático do model; nomes de
tabela em runtime não são aceitos como SQL executável.
Atributos nulos explícitos são emitidos como `NULL` do SQL, então
colunas anuláveis de bigint, integer, boolean, timestamp e outras não
textuais mantêm seu tipo de banco de dados no PostgreSQL. Todo atributo
não nulo continua vinculado por parâmetro. Linhas de upsert precisam
ter o mesmo conjunto de colunas; uma chave ausente ou extra é
rejeitada em vez de ser interpretada como nula.

```rust
// UPDATE em massa.
let n = User::query()
    .filter("active", false)
    .update_all(attrs! { archived_at: Utc::now() })
    .await?;

// DELETE em massa.
let n = Session::query()
    .filter_op("expires_at", "<", cutoff)
    .delete_all()
    .await?;

// INSERT ... ON CONFLICT (Postgres / SQLite) / ON DUPLICATE KEY UPDATE (MySQL).
let n = Counter::query()
    .upsert(
        vec![attrs! { key: "page_views", n: 1 }, attrs! { key: "signups", n: 1 }],
        vec!["key"],                  // alvo do conflito
        Some(vec!["n"]),              // colunas de update; None = toda coluna não única
    )
    .await?;

// Incremento/decremento atômico contra um scope.
User::query()
    .filter("id", 7)
    .increment_each(vec![("views", 1), ("likes", 1)])
    .await?;

User::query()
    .filter("id", 7)
    .decrement_each(vec![("balance", 100)])
    .await?;
```

### Helpers estáticos de `Model`

```rust
// Destruição em massa por conjunto de PKs. Eventos por linha disparam
// (cada linha passa por .delete(), então a semântica de tombstone de
// soft-delete + o dispatch de Deleting/Deleted são honrados).
let removed: u64 = User::destroy(vec![1i64, 2, 3]).await?;
let removed: u64 = User::force_destroy(vec![1i64, 2, 3]).await?;

// Comparação de identidade por PK.
assert!(alice.is(&also_alice));
assert!(alice.is_not(&bob));
```

### Variantes `*Quietly` - suprimem eventos de ciclo de vida

Açúcar sintático sobre `seed::without_events`. Os cinco eventos
estáticos de ciclo de vida
(`Saving`/`Creating`/`Updating`/`Deleting`/`Restoring`) e os
after-events não canceláveis fazem curto-circuito dentro do scope.

```rust
user.save_quietly().await?;            // sem Saving / Updated / Saved
user.update_quietly(attrs).await?;
user.delete_quietly().await?;
user.force_delete_quietly().await?;
```

### Variantes `*_or_fail`

Erro explícito no caso de não encontrado. Útil em caminhos de código
que checam invariantes, onde uma linha ausente é um bug.

```rust
let user = user.update_or_fail(attrs).await?;   // not_found se a linha foi excluída em voo
user.delete_or_fail().await?;
```

### Serialização filtrada - `to_array_except` / `to_array_only`

A substituição nativa em Rust do Suprnova para o `makeHidden` /
`makeVisible` por instância do Laravel. A struct Eloquent não carrega
um conjunto de atributos em runtime, então a lista de colunas é
fornecida no ponto de chamada:

```rust
return Json::ok(user.to_array_except(&["password_hash", "remember_token"]));
return Json::ok(user.to_array_only(&["id", "name", "email"]));
```

**Nota de divergência.** O `makeHidden` por instância do Laravel muta
estado que se propaga quando o model está aninhado dentro da chamada
`toArray()` de um pai. O filtro do Suprnova é terminal - ele produz um
`serde_json::Value` e não afeta serializações futuras de `self`. Para
controle de visibilidade declarativo e permanente, use os attributes
`#[model(hidden = [...])]` / `#[model(visible = [...])]`.

### Chaves primárias UUID / ULID - `#[model(unique_id = "...")]`

O análogo do Suprnova para a família de traits `HasUuids` / `HasUlids`
/ `HasVersion4Uuids` do Laravel. Defina o attribute, tipe a PK como
`String`, e a macro preenche o ID automaticamente antes do INSERT.

```rust
#[model(
    table = "users",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "uuid",      // ou "uuid_v4", "ulid"
)]
pub struct User {
    pub id: String,
    pub email: String,
}

// Preenchido automaticamente:
let u = User::create(attrs! { email: "a@b.com" }).await?;
// u.id é um UUID v7 novo.

// IDs fornecidos pelo chamador ainda vencem (casa com o comportamento do HasUuids do Laravel).
let u = User::create(attrs! { id: "...", email: "..." }).await?;
```

Estratégias suportadas:

- `"uuid"` / `"uuid_v7"` - UUID v7 (ordenado por timestamp,
  recomendado; corresponde ao `Str::uuid7()` padrão do Laravel 11+)
- `"uuid_v4"` - UUID aleatório (corresponde a `HasVersion4Uuids`)
- `"ulid"` - ULID Crockford-base32 de 26 caracteres em minúsculas

A macro emite um bloco `impl HasUniqueId for YourStruct` que expõe
`UNIQUE_ID_KIND` e um hook `new_unique_id()` que você pode sobrescrever
no tipo para um gerador customizado (por exemplo, IDs com prefixo como
`usr_<uuid>`).

### `find_or` / `find_or_new` / `create_or_first`

Completam a superfície da trait `FirstOrCreate`.

```rust
// Busca por PK; roda o fallback se não encontrar.
let user = User::find_or(id, || async {
    User::create(attrs! { id, name: "guest" }).await
}).await?;

// Busca por PK; constrói uma instância não salva a partir dos padrões se não encontrar.
let user = User::find_or_new(id, attrs! { name: "draft" }).await?;
// user.id == 0 aqui - a instância existe só em memória.

// Insert seguro contra corridas: tenta criar, recai para buscar em caso de conflito.
let user = User::create_or_first(
    attrs! { email: "race@x.com" },
    attrs! { name: "race winner" },
).await?;
```

### Scope `without_touching`

O análogo do Suprnova para o `Model::withoutTouching` do Laravel.
Dentro do scope, toda chamada `model.touch().await` faz
curto-circuito - útil ao executar migrações de dados ou jobs em batch
que mutam timestamps por outros caminhos.

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    // chamadas .touch() aqui não fazem nada.
    for post in posts {
        post.touch().await?;
    }
}).await;
```

O scope é apoiado em `tokio::task_local`, então solicitações
concorrentes em outras tasks continuam a honrar o próprio scope (ou a
ausência dele).

## Próximos passos

- [Relacionamentos Eloquent](eloquent-relationships.md) - mergulho
  profundo em todo tipo de relação, o registro de morph, e o
  lowering do enum polimórfico
- [Coleções Eloquent](eloquent-collections.md) - superfície completa
  de `Collection<T>`, a divisão genérico-vs-model, e streaming de
  `LazyCollection<M>`
- [Eloquent - casts, acessadores e mutadores](eloquent-mutators.md) -
  os 22 casts embutidos mais o override em runtime `casts!`
- [Serialização Eloquent](eloquent-serialization.md) - `to_array`,
  `to_json`, hidden / visible / appends, terminais filtrados
- [Factories Eloquent](eloquent-factories.md) - instâncias de model
  randomizadas para testes e seeders

# Construtor de consultas

Quando você quer consultar uma tabela sem modelá-la como uma struct
tipada `#[suprnova::model]`, use `DB::table(name)`. Ele retorna um
construtor encadeável no mesmo formato do `Builder<M>` tipado do
Eloquent, mas materializa as linhas como `DynamicRow` - um newtype
`serde_json::Map` com acessores tipados. Este é o capítulo para logs
de auditoria, relatórios ad-hoc, agregados de dashboard, e qualquer
tabela que você não se deu ao trabalho de modelar. Para o equivalente
tipado, veja [Eloquent](eloquent.md). Para `DB::select` bruto dentro
de transações ou com observação via `DB::listen`, veja
[Banco de dados](database.md).

```rust
use suprnova::DB;

let rows = DB::table("audit_log")
    .select(["id", "event", "actor_id"])
    .filter("actor_id", 42i64)
    .filter_op("created_at", ">=", "2026-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

for row in rows.iter() {
    let id: i64 = row.get_int("id")?;
    let event: String = row.get_string("event")?;
    println!("{id}: {event}");
}
```

## Quando usar qual superfície

Três superfícies de consulta se sobrepõem; escolha a certa para a
tabela.

| A tabela é… | Use | Retorna |
|---|---|---|
| Modelada com `#[suprnova::model]` | `Model::query()` → `Builder<M>` | valores `M` tipados |
| Não modelada mas você quer uma forma encadeável de WHERE/ORDER/LIMIT | `DB::table(name)` → `DbTableBuilder` | `DynamicRow` |
| Qualquer coisa que os construtores não conseguem expressar - CTEs, funções de janela, DDL de backend | `DB::select` / `DB::statement` / `DB::affecting_statement` | `DynamicRow` / `bool` / `u64` |

`DbTableBuilder` existe para o caso do meio. Você ganha a cadeia de
WHERE / ORDER / LIMIT sem se comprometer com uma struct
`#[suprnova::model]` e sem cair inteiramente para strings SQL brutas.

## A superfície encadeável

`DB::table(name)` retorna um `DbTableBuilder`. Construa-o, então chame
um método terminal para executar.

### Filtragem

```rust
// Igualdade.
DB::table("users").filter("email", "alice@example.com").get().await?;

// Operador arbitrário. Allowlist: =, <>, <, <=, >, >=, LIKE, NOT LIKE,
// ILIKE, NOT ILIKE, IS, IS NOT.
DB::table("orders").filter_op("total", ">=", 100i64).get().await?;
DB::table("posts").filter_op("title", "LIKE", "%rust%").get().await?;

// Múltiplos filtros se combinam com AND.
DB::table("audit_log")
    .filter("actor_id", 42i64)
    .filter_op("event", "<>", "noop")
    .get()
    .await?;
```

`filter` e `filter_op` aceitam qualquer `Into<SeaValue>` para o lado
direito, o que cobre `i64`, `String`, `&str`, `bool`, `f64`,
`Option<T>`, `chrono::*`, `uuid::Uuid`, e `serde_json::Value` - todo
tipo de coluna que o backend entende.

### Selecionando colunas

```rust
// O padrão é SELECT *.
DB::table("users").get().await?;

// Restrinja colunas quando você só precisa de algumas.
DB::table("users").select(["id", "email"]).get().await?;
```

### Ordenação e janelamento

```rust
DB::table("posts")
    .order_by_desc("created_at")
    .order_by_asc("title")
    .limit(20)
    .offset(40)
    .get()
    .await?;
```

`order_by_desc` e `order_by_asc` se encadeiam na ordem de inserção; o
SQL gerado a preserva.

### Terminais

```rust
// Todas as linhas correspondentes.
let rows: Collection<DynamicRow> = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .get()
    .await?;

// Primeira linha ou None.
let first: Option<DynamicRow> = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

// Apenas a contagem (limpa qualquer select/order/limit/offset antes
// de renderizar - a semântica de count não se importa com eles).
let n: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;
```

`get()` retorna `Collection<DynamicRow>` - o mesmo wrapper de coleção
que os models tipados usam, com a mesma superfície `.iter()`,
`.len()`, `.into_vec()`. Veja
[Coleções Eloquent](eloquent-collections.md).

### Inserções, atualizações, exclusões

```rust
use suprnova::attrs;

// INSERT, retorna o id auto-increment da nova linha.
let id: i64 = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

// UPDATE, retorna linhas afetadas.
let updated: u64 = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

// DELETE, retorna linhas afetadas.
let deleted: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

A macro `attrs!` constrói o mapa de coluna-para-valor no call site.
As chaves são identificadores SQL (validados) e os valores são
vinculados como parâmetros. Um valor nulo explícito é emitido como
`NULL` SQL porque o mapa de attributes JSON não carrega mais seu tipo
Rust original; todos os valores não nulos continuam vinculados como
parâmetros. A mesma regra vale para escritas em massa Eloquent tipadas
e extras de pivot many-to-many.

#### Aliases `update_all` e `delete_all`

`update` e `delete` são os nomes fiéis ao Laravel. Os aliases no
estilo `Builder<M>` - `update_all` e `delete_all` - chamam a mesma
implementação. Prefira a forma `_all` quando a intenção de afetar a
tabela inteira for o ponto do call site; ela torna um `filter`
ausente visível para revisores:

```rust
// Mesmo comportamento que DB::table("rate_limits").delete().await?
// mas o sufixo _all diz aos revisores "sim, eu quis truncar a
// tabela".
DB::table("rate_limits").delete_all().await?;

// Atualização em massa com um WHERE - o sufixo _all aqui casa com a
// convenção do Builder<M> tipado para a mesma operação.
DB::table("sessions")
    .filter_op("expires_at", "<", chrono::Utc::now())
    .update_all(attrs! { status: "expired" })
    .await?;
```

#### Um WHERE vazio em update ou delete opera em toda linha

`DB::table("x").delete().await?` remove toda linha da tabela. Isso é
suportado por design - às vezes você realmente quer truncar - mas
raramente é o correto. Sempre olhe para uma chamada `delete()` /
`delete_all()` e confira se há um `filter` na frente dela. O mesmo
vale para `update` / `update_all`.

#### Divisão de backend no insert

`RETURNING id` é usado no Postgres e no SQLite. O MySQL não suporta
`RETURNING`, então o construtor executa o INSERT e lê o
`last_insert_id()` por conexão do driver a partir do resultado. O
construtor sem model assume uma chave primária `id` auto-increment
padrão. Chaves primárias UUID, compostas, renomeadas, ou não
inteiras não são suportadas nesta superfície - use a interface
`Model` tipada do [Eloquent](eloquent.md) em vez disso, que consulta
a definição do model para a forma da chave primária.

## `DynamicRow` - acessores tipados sobre um mapa JSON

Toda linha retornada por `DB::table` ou `DB::select` materializa como
`DynamicRow`, um newtype `serde_json::Map<String, Value>` com
acessores tipados. Cada getter retorna `Result<T, FrameworkError>`
com uma mensagem de erro clara em chave ausente ou incompatibilidade
de tipo.

```rust
for row in rows.iter() {
    let id: i64                 = row.get_int("id")?;
    let event: String           = row.get_string("event")?;
    let active: bool            = row.get_bool("active")?;
    let weight: f64             = row.get_float("weight")?;
    let payload: serde_json::Value = row.get_value("payload")?;
}
```

Para colunas anuláveis, use `get_optional_*`. Elas distinguem "coluna
ausente" (erro - descompasso de esquema) de "coluna presente, valor
SQL NULL" (`Ok(None)`):

```rust
let title: Option<String> = row.get_optional_string("title")?;
let score: Option<i64>    = row.get_optional_int("score")?;
```

Hoje a família optional cobre `String` e `i64`. Para outros tipos
anuláveis, use `get_value` e faça o match em
`serde_json::Value::Null` você mesmo, ou leia a coluna através de
`get_as::<Option<T>>` (qualquer `T: DeserializeOwned`).

Para desserializar uma coluna em qualquer struct ou tipo container,
use `get_as`. A superfície completa de desserialização do
`serde_json` está disponível:

```rust
#[derive(serde::Deserialize)]
struct UserPrefs {
    theme: String,
    notifications: bool,
}

let prefs: UserPrefs    = row.get_as("prefs")?;
let tags: Vec<String>   = row.get_as("tags")?;
let when: chrono::DateTime<chrono::Utc> = row.get_as("created_at")?;
```

`DynamicRow` faz deref para `Map<String, Value>`, então iteração e
verificações de existência de chave funcionam diretamente:

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}

if row.contains_key("deleted_at") { /* … */ }
```

## Fronteira de confiança de identificadores

Nomes de tabela, nomes de coluna, direções de ORDER BY, e operadores
SQL são interpolados na string SQL literalmente - eles NÃO são
vinculados como parâmetros (SQL não permite identificadores
vinculados a placeholder). Trate todo argumento `impl Into<String>`
como um literal confiável, definido em tempo de compilação.

```rust
// Seguro - o nome da coluna é uma constante; o valor é vinculado.
DB::table("users").filter("email", request.email()).get().await?;

// INSEGURO - nunca injete entrada do usuário em um nome de coluna.
DB::table("users")
    .filter(request.user_supplied_column(), value)
    .get()
    .await?;
```

O framework aplica uma allowlist estrita na fronteira de I/O - os
identificadores precisam corresponder a `[A-Za-z_][A-Za-z0-9_]*` com
um prefixo `schema.` opcional, e os operadores precisam vir de uma
lista fixa. Violações falham de forma fechada com um
`FrameworkError::Database` antes de qualquer SQL ser renderizado.
Isso é uma rede de segurança, não uma licença: mantenha os
identificadores literais no seu código.

Valores no lado direito de `filter` / `filter_op` são sempre
vinculados como parâmetros e seguros de injetar a partir de dados da
solicitação.

## Consultas brutas

Quando o construtor não consegue expressar o que você precisa - CTEs
recursivas, funções de janela, DDL específico de backend,
`INSERT … ON CONFLICT DO UPDATE` - caia para uma string bruta. Os
placeholders casam com o backend ativo (`$1, $2, …` para Postgres,
`?` para MySQL e SQLite); o framework detecta automaticamente a
partir de `DatabaseConfig::url`.

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - toda linha como DynamicRow.
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) AS post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING COUNT(p.id) > ?",
    vec![Value::from(5i64)],
).await?;

// SELECT - apenas a primeira linha, espelha o DB::selectOne do
// Laravel.
let alice = DB::select_one(
    "SELECT * FROM users WHERE email = ?",
    vec![Value::from("alice@example.com")],
).await?;

// SELECT - primeira coluna da primeira linha como um escalar tipado.
let total: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// INSERT - true quando pelo menos uma linha foi afetada.
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - retornam a contagem de linhas afetadas.
let updated: u64 = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1i64)],
).await?;

let deleted: u64 = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// Qualquer prepared statement com bindings.
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1i64), Value::from(42i64)],
).await?;

// DDL ou outros statements sem binding que rejeitam binding de
// placeholder.
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// Caminho genérico de "linhas afetadas" - para upserts e operações
// que não se encaixam nos helpers nomeados.
let n: u64 = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1)
     ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec![Value::from("page_views")],
).await?;
```

### Pegadinha de coluna agregada

Agregados sem tipo como `SELECT COUNT(*) AS n FROM t` funcionam
através do helper `.count()` do construtor, mas podem voltar
silenciosamente descartados de linhas brutas de `DB::select` no
SQLite. O materializador de linha subjacente percorre as informações
de tipo por coluna do sqlx, e um agregado nu não carrega nenhuma. Se
você precisa de `DB::select` bruto com agregados no SQLite, ou
envolva a expressão em `CAST(… AS BIGINT)` para dar a ela uma
etiqueta de tipo, ou use `DB::scalar::<i64>`, que passa por
`query_one` + `try_get` e não depende da detecção de tipo por
coluna.

## Ponte para o Eloquent tipado

Quando a tabela merece uma struct `#[suprnova::model]`, a forma
encadeável se mantém. `Model::query()` retorna `Builder<M>`, que
oferece a mesma superfície `filter` / `filter_op` / `order_by_*` /
`limit` / `offset` / `get` / `first` / `count` - além de um
vocabulário de WHERE muito mais amplo (`filter_in`, `filter_between`,
`filter_null`, `filter_has`, `filter_raw`, …) e aliases no formato
Laravel (`db_where`, `where_in`, `where_between`, `where_null`,
`where_has`, `where_raw`, …).

```rust
use suprnova::Model;

let admins = User::query()
    .filter("role", "admin")
    .filter_op("created_at", ">=", since)
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;     // Collection<User> - tipado, não DynamicRow

let alice = User::query().filter("email", &email).first().await?;
let total = User::query().filter("active", true).count().await?;
// Nota: Builder<M>::count retorna i64 (casa com o Eloquent do
// Laravel), enquanto DbTableBuilder::count retorna u64. As duas
// superfícies dão a você um SQL COUNT não negativo - elas só
// diferem no tipo de fio.
```

A superfície completa do `Builder<M>` - toda forma de WHERE,
agregados, relações, eager loading, scopes, paginadores, iteração em
chunks - está em [Eloquent](eloquent.md). A forma encadeável que você
aprendeu acima é a mesma forma; as diferenças são de tipagem e
alcance.

## Roteamento para uma conexão nomeada

`DB::table` e os helpers brutos usam a conexão primária por padrão.
Para direcionar a uma read replica, shard, ou pool de warehouse, fixe
a chamada:

```rust
// Construtor fixado em uma conexão nomeada.
let rows = DB::table("audit_log").on("warehouse").get().await?;

// Atalho equivalente.
let rows = DB::table_on("warehouse", "audit_log").get().await?;

// Os escapes brutos também têm variantes _on.
let rows = DB::select_on("warehouse", "SELECT …", vec![]).await?;
let n    = DB::affecting_statement_on(
    "warehouse",
    "UPDATE …",
    vec![],
).await?;
```

Quando `__read_replica__` está registrada, todo terminal com forma de
leitura roteia automaticamente através dela; escritas (`insert` /
`update` / `delete` / `update_all` / `delete_all`) sempre têm como
destino a primária. Dentro de uma closure `DB::transaction` a conexão
da transação ativa vence absolutamente - `on(name)` é silenciosamente
ignorado para preservar a atomicidade. Veja
[Banco de dados - Conexões nomeadas](database.md) para a cadeia de
precedência completa.

### Por que Suprnova diverge

O `DB::table(...)` do Laravel é seu construtor de consultas sem
model; por baixo dos panos ele retorna um `stdClass` por linha (um
objeto PHP cujas propriedades são as colunas). O Suprnova retorna
`DynamicRow` em vez disso - um newtype `serde_json::Map` com
acessores tipados. A forma de acessor captura erros de
coluna-ausente e tipo-errado na fronteira em vez de entrar em panic
lá no fundo do código do usuário com uma exceção de acesso a
propriedade.

Os nomes duplos `update`/`update_all` e `delete`/`delete_all` existem
porque a superfície `Builder<M>` tipada do Eloquent usa o sufixo
`_all` para tornar explícita a intenção de afetar a tabela inteira no
call site. Em vez de escolher um lado, o construtor sem model traz os
dois - `update` e `delete` casam letra por letra com o
`DB::table($t)->update(...)` e o `->delete()` do Laravel; `update_all`
e `delete_all` casam com a convenção que os usuários de `M` já vão
ter na memória muscular.

## Próximos passos

- [Banco de dados](database.md) - facade `DB`, transações com
  savepoints, observabilidade via `DB::listen`, conexões nomeadas
- [Eloquent](eloquent.md) - structs `#[suprnova::model]` tipadas e a
  superfície completa do `Builder<M>`
- [Paginação](pagination.md) - `paginate` / `simple_paginate` /
  `cursor_paginate` em construtores tipados
- [Coleções Eloquent](eloquent-collections.md) - a `Collection<T>`
  retornada por `get()` nas duas superfícies
- [Migrações](migrations.md) - definindo o esquema que os
  construtores consultam

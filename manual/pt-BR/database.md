# Banco de dados

A camada de banco de dados do Suprnova envolve o SeaORM com uma
facade `DB` no formato Laravel: escapes de consulta bruta, um
construtor de consultas sem model, transações com savepoints e retry
em deadlock, registry de conexão para read replicas e shards, e uma
superfície completa de observabilidade que espelha a API
`DB::listen` / `QueryExecuted` / query log do Laravel 13.

O ORM Eloquent (`use suprnova::eloquent::*`) se constrói sobre esta
camada e vive em [eloquent.md](eloquent.md). Quando você quer um
model tipado, vá lá; quando você quer uma consulta bruta contra uma
tabela não modelada ou quer observar toda consulta que o framework
executa, esta é a página.

## Configuração

```rust
use suprnova::{Config, DB, DatabaseConfig};

// Em bootstrap.rs
Config::register(DatabaseConfig::from_env());
DB::init().await.expect("DB::init failed");
```

`DatabaseConfig::from_env` lê `DATABASE_URL` e (opcionalmente) os
ajustes de pool `DB_MAX_CONNECTIONS`, `DB_MIN_CONNECTIONS`,
`DB_CONNECT_TIMEOUT`, `DB_LOGGING`. Quando `DATABASE_URL` não está
definida a config recai para `sqlite://./database.db` - conveniente
para desenvolvimento sem setup; boots de produção recusam o fallback
via `validate_for_environment` para que você não consiga
acidentalmente enviar um arquivo SQLite em `APP_ENV=production`.

URL → detecção de driver:

```text
postgres://user:pass@host/db       → DatabaseType::Postgres
postgresql://user:pass@host/db     → DatabaseType::Postgres
mysql://user:pass@host/db          → DatabaseType::Mysql
sqlite://./file.db                 → DatabaseType::Sqlite
sqlite::memory:                    → DatabaseType::Sqlite
```

### Vivacidade do pool

Um gateway NAT, um load balancer ou um firewall vai derrubar
silenciosamente uma conexão TCP que ficou ociosa por tempo demais. O
pool não fica sabendo. A próxima consulta naquela conexão falha, e ela
falha em uma solicitação que não tinha nada a ver com a interrupção.

O Laravel responde a isso com as opções de DSN `keepalives`,
`keepalives_idle`, `keepalives_interval` e `keepalives_count` da libpq,
que mantêm o socket aquecido. **Elas não são alcançáveis a partir do
Suprnova.** O sqlx 0.9 parseia de uma URL de Postgres apenas `sslmode`,
`application_name`, `options` e o tamanho do cache de statements, e não
carrega nenhum setter de keepalive TCP em camada alguma, então não há
para onde encaminhá-las.

O que o Suprnova te dá no lugar é a resposta do lado do pool: pare de
confiar em conexões velhas.

```bash
# Fecha uma conexão que ficou ociosa por dois minutos.
DB_IDLE_TIMEOUT=120
# Recicla toda conexão depois de quinze minutos, de todo jeito.
DB_MAX_LIFETIME=900
# Faz ping em uma conexão antes de entregá-la, mas só depois que ela
# ficou ociosa por trinta segundos. Conexões quentes pulam o round trip.
DB_PING_AFTER_IDLE=30
```

Ou programaticamente:

```rust
Config::register(
    DatabaseConfig::builder()
        .url(std::env::var("DATABASE_URL")?)
        .idle_timeout(120)
        .max_lifetime(900)
        .ping_after_idle(30)
        .build(),
);
```

Todo ajuste vem sem valor definido por padrão, o que significa que o
pool mantém os próprios padrões do sqlx: conexões fecham depois de 600
segundos ociosas, reciclam depois de 1800 segundos e sofrem ping antes
de cada checkout. Defina `DB_IDLE_TIMEOUT=0` ou `DB_MAX_LIFETIME=0`
para desligar por completo essa forma de coleta.

`DB_PING_AFTER_IDLE` e `DB_TEST_BEFORE_ACQUIRE` são alternativas, não
um par: definir um limiar desliga o ping por checkout, porque executar
os dois faria ping a cada aquisição e tornaria o limiar sem sentido.

### Por que Suprnova diverge

Keepalives e reciclagem de pool resolvem a mesma falha por pontas
opostas. Keepalives impedem que um middlebox expire a conexão; a
reciclagem aceita que ele vai expirar e garante que o pool nunca
entregue uma conexão velha o bastante para ter sido expirada. A segunda
é o que a pilha de drivers expõe, e ela também cobre falhas que
keepalives não cobrem - uma réplica que assumiu num failover, uma
credencial rotacionada, uma desconexão por ociosidade do lado do
servidor. Se você precisa especificamente das opções da libpq, isso é
uma mudança no sqlx, não no Suprnova.

## Consultas brutas

A facade `DB` traz a superfície completa de escape bruto do Laravel 13. Todo
helper passa pelo mesmo executor instrumentado - toda
chamada dispara `QueryExecuted` (veja
[Observabilidade](#observabilidade)).

Bindings são `sea_orm::Value` - um dos poucos tipos do sea_orm que o
framework intencionalmente NÃO remascara, porque todo valor que é
enviado ao banco de dados passa por ele. `Value::from(...)` funciona
para todo primitivo que o banco de dados entende.

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - todas as linhas como DynamicRow.
let users = DB::select(
    "SELECT * FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// SELECT - apenas a primeira linha.
let alice = DB::select_one(
    "SELECT * FROM users WHERE name = ?",
    vec![Value::from("alice")],
).await?;

// SELECT - primeira coluna da primeira linha como um valor tipado.
let count: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users",
    vec![],
).await?;

// INSERT - retorna bool (true quando pelo menos uma linha foi afetada).
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - retornam a contagem de linhas afetadas.
let updated = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1)],
).await?;
let deleted = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// Qualquer prepared statement com bindings.
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1), Value::from(42)],
).await?;

// DDL sem bindings - `unprepared` espelha o `DB::unprepared` do
// Laravel para statements (CREATE INDEX, ALTER TABLE, VACUUM) que
// rejeitam binding de placeholder.
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// affecting_statement é a forma explícita usada por update/delete
// internamente - use-a diretamente para ops que não se encaixam em
// nenhum dos dois nomes (ex.: INSERT...ON CONFLICT DO UPDATE).
let affected = DB::affecting_statement(
    "INSERT INTO users (id, name) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name",
    vec![Value::from(1), Value::from("alice")],
).await?;
```

### Sintaxe de placeholder

`?` para SQLite + MySQL. `$1`, `$2`, ... para Postgres. O backend
ativo é detectado automaticamente a partir de `DatabaseConfig::url`.

### DynamicRow

Linhas sem tipo materializam como `DynamicRow` - um newtype
`serde_json::Map` com acessores tipados:

```rust
for row in users {
    let id: i64 = row.get_int("id")?;
    let name: String = row.get_string("name")?;
    let nickname: Option<String> = row.get_optional_string("nickname")?;
    let score: Option<i64> = row.get_optional_int("score")?;
    // Desserializa um T arbitrário (chrono::DateTime, sua própria struct, etc.):
    let prefs: UserPrefs = row.get_as("prefs")?;
}
```

`get_*` falha quando a coluna está ausente OU é null.
`get_optional_*` falha apenas quando ausente e retorna `Ok(None)`
para SQL NULL. A lista completa de acessores é `get_int` /
`get_string` / `get_bool` / `get_float` / `get_value` / `get_as<T>`
mais `get_optional_string` / `get_optional_int`; para tipos
anuláveis sem um `get_optional_*` dedicado, recorra a `get_value` +
um match em `serde_json::Value`, ou `get_as::<Option<T>>`.

## Construtor de consultas sem model - `DB::table`

Para consultas ad-hoc contra tabelas que você não se deu ao trabalho
de modelar com `#[suprnova::model]`, `DB::table(...)` retorna um
construtor encadeável no mesmo formato do `Builder<M>` do Eloquent,
mas materializando as linhas como `DynamicRow`:

```rust
use suprnova::{DB, attrs};

let rows = DB::table("audit_log")
    .select(["id", "event", "actor_id"])
    .filter("actor_id", 42i64)
    .filter_op("created_at", ">=", "2025-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

let first = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

let count = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;

let id = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

let updated = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

let deleted = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

### Fronteira de confiança nos identificadores

Nomes de tabela, nomes de coluna, direções de ORDER BY, e operadores
SQL são interpolados NA string SQL literalmente - eles NÃO são
vinculados como parâmetros (SQL não permite identificadores
vinculados a placeholder). Trate todo argumento `impl Into<String>`
como um literal CONFIÁVEL:

```rust
// Seguro - o nome da coluna é uma constante.
DB::table("users").filter("email", request.email()).get().await?;

// INSEGURO - nunca injete entrada do usuário em um nome de coluna.
DB::table("users").filter(&request.column_name(), value).get().await?;
```

Valores (o lado direito de `filter` / `filter_op`) SÃO vinculados
como parâmetros e seguros para entrada do usuário.

O framework aplica uma allowlist estrita sobre identificadores
(`[A-Za-z_][A-Za-z0-9_]*` com um prefixo `schema.` opcional) e
operadores (`=`, `<>`, `<`, `<=`, `>`, `>=`, `LIKE`, `NOT LIKE`,
`ILIKE`, `NOT ILIKE`, `IS`, `IS NOT`). Violações falham na fronteira
de I/O antes de a string SQL ser renderizada.

## Transações

Três pontos de entrada, cada um com os hooks de observação
`QueryExecuted` / `TransactionBeginning` / `TransactionCommitted` /
`TransactionRolledBack` conectados.

### Forma de closure

```rust
use suprnova::DB;

DB::transaction(|_tx| {
    Box::pin(async move {
        let mut alice = User::query().filter("name", "alice").first_or_fail().await?;
        alice.balance -= 30;
        alice.save().await?;

        let mut bob = User::query().filter("name", "bob").first_or_fail().await?;
        bob.balance += 30;
        bob.save().await?;
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

Commit em `Ok(_)`. Rollback + propaga o erro em `Err(_)`.

Um `Err` nem sempre é um rollback. Se um callback
[pós-commit](queues.md#after-commit-dispatch) falha, o commit já
aconteceu e é durável; o `DB::transaction` ainda assim retorna `Err`, e a
mensagem diz `after-commit callback failed (the transaction itself
committed): <o erro do callback>`. O valor de retorno da closure se perde,
as escritas dela não, e só um dispatch adiado falhou. Todo callback
registrado ainda roda, e o erro que você recebe é o primeiro deles. O
`DB::transaction_with_attempts` nunca repete esse erro, por mais cara de
deadlock que ele tenha: reexecutar uma closure cujas escritas já são
duráveis as aplicaria duas vezes.

Operações dentro da closure pegam a transação ativa automaticamente
via um `tokio::task_local` - você NÃO precisa passar um handle `&tx`
através de toda chamada de model. Um `DB::transaction` aninhado
retorna um erro de banco de dados; use `tx.savepoint(...)` para
comportamento de rollback aninhado.

A forma de closure também é a única forma capaz de adiar trabalho para o
commit. Um job cujo tipo declara `Job::after_commit()` (ou um dispatch
feito com `Queue::push_after_commit`) espera dentro desta closure e só
chega ao driver de fila quando o commit tem sucesso; um rollback o
descarta. Veja
[Dispatch pós-commit](queues.md#after-commit-dispatch).

Para agregado tipado ou SQL customizado que precisa executar na
mesma conexão fixada, use o handle da transação diretamente:

```rust
use sea_orm::{DbBackend, Statement};

DB::transaction(|tx| {
    Box::pin(async move {
        let backend = tx.backend();
        let rows = tx.query_all(Statement::from_string(
            backend,
            "SELECT CAST(COUNT(*) AS BIGINT) AS total FROM orders".to_owned(),
        )).await?;
        let total = rows[0].try_get::<i64>("", "total")?;
        Ok::<_, suprnova::FrameworkError>(total)
    })
}).await?;
```

`query_all` emite observações `QueryExecuted` normais e retorna
linhas `QueryResult` tipadas do SeaORM. Use
`Statement::from_sql_and_values` vinculado para valores dinâmicos;
não interpole entrada não confiável.

### Retry em deadlock

```rust
DB::transaction_with_attempts(5, |_tx| {
    Box::pin(async move {
        // Mesmo corpo de closure de acima. Executa de novo do zero em
        // SQLSTATE 40001 / 40P01 / qualquer erro contendo "deadlock"
        // (sem diferenciar maiúsculas/minúsculas).
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

### Forma manual

```rust
use suprnova::{DB, attrs};

let tx = DB::begin_transaction().await?;

// Por model: os shims `*_with_tx` fixam uma operação de CRUD à tx manual.
User::create_with_tx(&tx, attrs! { name: "alice" }).await?;
Order::create_with_tx(&tx, attrs! { user_id: 1, total: 30 }).await?;

// Por consulta: `Builder::with_tx(&tx)` fixa uma cadeia de construtor.
let stale = Order::query()
    .filter("status", "pending")
    .with_tx(&tx)
    .get()
    .await?;

if some_condition() {
    tx.rollback().await?;
} else {
    tx.commit().await?;
}
```

O modo manual NÃO instala o task-local - toda operação que deveria
executar dentro da transação precisa aderir explicitamente, seja via
`Builder::with_tx(&tx)` em uma consulta encadeada ou um dos shims
`Model::*_with_tx` (`create_with_tx`, `save_with_tx`,
`delete_with_tx`, etc.). Operações que esquecem de aderir executam
contra o pool global e NÃO fazem parte da transação.

Manter um handle `Transaction` fixa uma conexão do pool por toda sua
vida; pré-carregue qualquer linha que você precise ler ANTES da
chamada `begin_transaction()`, especialmente no SQLite (conexão
única compartilhada).

Como o modo manual não instala task-local, ele também não tem commit em
que um dispatch adiado possa se pendurar: um job
[pós-commit](queues.md#after-commit-dispatch) enviado dentro de uma
transação manual é enviado imediatamente. Use a forma de closure quando um
dispatch tiver de esperar pelo commit.

### Savepoints

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        Order::create(/* ... */).await?;

        tx.savepoint("after_order").await?;
        if let Err(e) = Payment::charge().await {
            // Descarta a tentativa de pagamento mas mantém o pedido.
            tx.rollback_to("after_order").await?;
        }
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

Os três backends de primeira classe suportam `SAVEPOINT` /
`ROLLBACK TO SAVEPOINT` - SQLite incluso.

Um rollback de savepoint também desfaz o
[registry pós-commit](queues.md#after-commit-dispatch). Um push de fila
adiado para o commit dentro do savepoint é descartado junto com as linhas
que ele descrevia, e a compensação registrada com ele roda imediatamente,
então o lock de deduplicação de um `push_unique` adiado volta e um
re-dispatch dentro da mesma transação pode ganhá-lo. Qualquer coisa
registrada antes do savepoint fica intocada, e um savepoint que você
libera, ou em que simplesmente nunca faz rollback, mantém tudo que foi
registrado dentro dele.

Repetir um nome de savepoint é permitido, e o registry segue o banco de
dados: `ROLLBACK TO SAVEPOINT x` desfaz até o `x` mais recente e destrói os
savepoints estabelecidos depois dele. Transações manuais não têm registry
pós-commit, então os savepoints delas fazem rollback de linhas e de mais
nada.

Só o `Transaction::savepoint` marca o registry. Um savepoint que você cria
com SQL bruto é invisível para ele, então o `rollback_to` faz rollback
daquelas linhas, registra um aviso e deixa no lugar todo dispatch adiado
registrado dentro dele - descartar um no chute seria a falha pior. Use o
`Transaction::savepoint` quando os dispatches adiados devem ser desfeitos
junto com as linhas.

## Observabilidade

A superfície `DB::listen` / `QueryExecuted` / query log do Laravel
13, portada para Rust através do dispatcher de eventos do Suprnova.

### `DB::listen` - callback direto

```rust
use suprnova::{DB, QueryExecuted};

// Em bootstrap.rs (ou um service provider).
DB::listen(|event: &QueryExecuted| {
    tracing::debug!(
        sql = %event.sql,
        bindings = ?event.bindings,
        time_ms = event.time.as_millis(),
        connection = %event.connection_name,
        "query executed",
    );
})?;
```

Listeners executam **de forma síncrona dentro do helper de
executor**. Um listener lento deixa a consulta lenta - mantenha os
callbacks diretos leves. Para qualquer coisa que possa falhar,
prefira o caminho do `EventFacade` abaixo; ele executa através de
`dispatch_best_effort` e tolera erros.

### Caminho de dispatch do `EventFacade`

`QueryExecuted` é um `suprnova::Event` real - escute através do
dispatcher para obter entrega enfileirada, fakeable, e tolerante a
falhas:

```rust
use suprnova::{EventFacade, Listener, QueryExecuted, FrameworkError};
use std::sync::Arc;

struct LogToDatabase;

#[suprnova::async_trait]
impl Listener<QueryExecuted> for LogToDatabase {
    async fn handle(&self, event: &QueryExecuted) -> Result<(), FrameworkError> {
        // Mesmo que ESTE listener consulte o banco de dados, a
        // salvaguarda de reentrância previne recursão infinita.
        DB::statement(
            "INSERT INTO query_log (sql, time_ms) VALUES (?, ?)",
            vec![event.sql.clone().into(), (event.time.as_millis() as i64).into()],
        ).await?;
        Ok(())
    }
}

// Em bootstrap.rs.
EventFacade::listen::<QueryExecuted, _>(Arc::new(LogToDatabase)).await;
```

Listeners neste caminho:

- Executam através de `dispatch_best_effort` - um listener que falha
  NÃO falha a consulta.
- Fazem short-circuit quando eles mesmos emitem uma consulta
  (salvaguarda de reentrância).
- Podem usar `Event::fake()` em testes para afirmar o dispatch sem
  de fato executar os listeners.

### Log de consulta em memória

```rust
DB::enable_query_log()?;

User::query().filter("active", true).get().await?;
Order::query().count().await?;

let log = DB::get_query_log()?;
for query in &log {
    println!("{} ({}ms)", query.sql, query.time.as_millis());
}

DB::flush_query_log()?;     // descarta entradas, mantém habilitado
DB::disable_query_log()?;   // para de capturar
let still_capturing = DB::logging();
```

O log é **ilimitado** - toda consulta capturada o faz crescer até o
processo sair, `flush_query_log()` executar, ou
`disable_query_log()` ser chamado. Use-o para desenvolvimento, não
como um profiler de produção de longa duração.

### Eventos de ciclo de vida da transação

`TransactionBeginning`, `TransactionCommitted`, e
`TransactionRolledBack` são tipos `suprnova::Event` reais - escute
por eles através de `EventFacade::listen` para acionar auditoria,
locks distribuídos, ou lógica de compensação.

```rust
EventFacade::listen::<TransactionCommitted, _>(Arc::new(AuditCommit)).await;
EventFacade::listen::<TransactionRolledBack, _>(Arc::new(MetricRollback)).await;
```

Os três pontos de entrada de transação (`DB::transaction` /
`DB::transaction_with_attempts` / `DB::begin_transaction` +
`Transaction::commit`/`rollback`) disparam os eventos. Um handle
`Transaction` manual vazado que dropa sem commit/rollback explícito
não emite nenhum evento - a impl `Drop` do SeaORM é síncrona e não
consegue alcançar o dispatcher async.

### Payload de `QueryExecuted`

```rust
pub struct QueryExecuted {
    pub sql: String,
    pub bindings: Vec<String>,         // renderizado em debug (`{:?}`)
    pub time: std::time::Duration,
    pub connection_name: String,
    pub read_write_type: Option<ReadWriteType>,
    pub result: Result<(), String>,    // Err em erro de driver
}
```

`to_raw_sql()` substitui os bindings capturados na SQL para exibição:

```rust
let query = /* capturado de um listener */;
println!("{}", query.to_raw_sql());
// SELECT * FROM users WHERE id = 42 AND active = true
```

A substituição é em **formato debug** (não é escaping seguro para
SQL) e é destinada apenas para saída de log. Nunca realimente o
resultado de volta em uma consulta.

### Escopo de cobertura

Hoje, `QueryExecuted` dispara para toda consulta que passa pelos
helpers instrumentados de `ExecutorChoice`:

- Todo helper bruto em `DB` (`select` / `select_one` / `scalar` /
  `insert` / `update` / `delete` / `statement` /
  `affecting_statement` / `unprepared`).
- Todo método terminal em `DbTableBuilder` (o construtor sem model).
- `DB::transaction` / `DB::begin_transaction` disparam eventos de
  transação em BEGIN / COMMIT / ROLLBACK.
- `DbConnection::connect` dispara `ConnectionEstablished`.

O ORM Eloquent (`Builder<M>::get` / `first` / `count`, CRUD de
model) casa diretamente com os braços `Tx` / `Pool` de
`ExecutorChoice` hoje em vez de chamar através dos helpers
instrumentados - adotar os helpers (e portanto o hook de observação)
pousa no módulo Eloquent.

## Metadados de conexão

```rust
let name = DB::database_name()?;        // "myapp" para postgres://.../myapp
let driver = DB::driver_name()?;        // "postgres" | "mysql" | "sqlite"
let title = DB::driver_title()?;        // "Postgres" | "MySQL" | "SQLite"
let version = DB::server_version().await?;  // "15.5" | "8.0.36" | "3.42.0"
```

`server_version` emite uma consulta de introspecção específica de
backend (`SELECT VERSION()` para Postgres + MySQL,
`SELECT sqlite_version()` para SQLite). Faça cache do resultado se
você o chama com frequência - toda chamada é um round trip.

## Conexões nomeadas

Para read replicas, shards fragmentados, ou pools de warehouse por
model:

```rust
// Em bootstrap.rs
DB::register_named("__read_replica__", read_config).await?;
DB::register_named("warehouse", warehouse_config).await?;

// Roteamento por consulta:
let rows = User::query().on("__read_replica__").get().await?;
let warehouse_rows = DB::table("audit_log").on("warehouse").get().await?;
let raw = DB::select_on("warehouse", "SELECT ...", vec![]).await?;
```

O nome `__read_replica__` é conhecido pelo framework: quando
registrado, todo método terminal com forma de leitura roteia
automaticamente através dele. Escritas ignoram a replica e têm como
destino a primária. Use `Builder::on_write_connection` (por consulta)
ou `#[model(connection = "...")]` (padrão por model) para voltar à
primária em operações específicas.

Nomes reservados:

- `__primary__` - o pool padrão. Não pode ser registrado (é o valor
  de retorno de `DB::connection()`).
- `__read_replica__` - read replica conhecida pelo framework.
  QUALQUER conexão registrada sob este nome assume o roteamento de
  leitura.

Veja
[eloquent.md → Roteamento multiconexão](eloquent.md#multi-connection-routing)
para a cadeia de precedência completa (override de tx do construtor
→ tx ambiente → `on(name)` do construtor → padrão do model →
`__read_replica__` → primária).

## Testes

`TestDatabase` constrói um banco de dados SQLite em memória,
registra-o no contêiner de teste para que `DB::connection()` resolva
a ele, e executa suas migrações:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn test_user_creation() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // Qualquer código que chame DB::connection() agora recebe este BD em memória.
    let _ = CreateUser::run("alice@example.com").await.unwrap();
}

// `test_database!()` é o atalho de macro.
let db = test_database!();
```

Para testes que constroem seu próprio esquema ad-hoc:

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").await.unwrap();
```

Quando um `TestDatabase` dropa, o contêiner de teste é limpo e o
registry de conexão é apagado - sem vazamento entre testes. Testes
que mutam estado global ao processo (o registry, o registry de
listener, o log de consulta) devem ser anotados
`#[serial_test::serial]` para que não colidam.

## Próximos passos

- [Eloquent](eloquent.md) - o ORM tipado `#[suprnova::model]` que se
  apoia sobre esta camada
- [Migrações](migrations.md) - `Migrator`, `make:migration`, e o
  fluxo de trabalho `db:sync`
- [Testes de banco de dados](database-testing.md) - `TestDatabase`,
  carregamento de fixture, e anotações de teste serial
- [Eventos](events.md) - o dispatcher por trás dos listeners de
  `QueryExecuted` / `TransactionCommitted`
- [Configuração](configuration.md) - registrando `DatabaseConfig` ao
  lado do resto da sua config tipada

## Índice de superfície

| Superfície | Análogo no Laravel |
| --- | --- |
| `DB::init` / `DB::init_with` / `DB::connection` / `DB::is_connected` / `DB::get` | `DB::connection()` |
| `DB::table(name)` → `DbTableBuilder` | `DB::table($name)` |
| `DB::select` / `select_one` / `scalar` / `insert` / `update` / `delete` / `statement` / `affecting_statement` / `unprepared` | `DB::select` / `selectOne` / `scalar` / `insert` / `update` / `delete` / `statement` / `affectingStatement` / `unprepared` |
| `DB::transaction` / `transaction_with_attempts` / `begin_transaction` | `DB::transaction($cb, $attempts)` / `DB::beginTransaction` |
| `Transaction::commit` / `rollback` / `savepoint` / `rollback_to` | `DB::commit` / `rollBack` / helpers de savepoint |
| `DB::listen(callback)` | `DB::listen` |
| `DB::enable_query_log` / `disable_query_log` / `get_query_log` / `flush_query_log` / `logging` | `DB::enableQueryLog` / `disableQueryLog` / `getQueryLog` / `flushQueryLog` / `logging` |
| `DB::database_name` / `driver_name` / `driver_title` / `server_version` | `getDatabaseName` / `getDriverName` / `getDriverTitle` / `getServerVersion` |
| `DB::register_named` / `named` / `select_on` / `table_on` / `statement_on` / `affecting_statement_on` | `DB::connection($name)` com múltiplas conexões |
| `QueryExecuted` / `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` / `ConnectionEstablished` / `DatabaseBusy` | `Illuminate\Database\Events\*` |
| `DatabaseConfig::builder()` / `from_env` / `validate_for_environment` / `idle_timeout` / `max_lifetime` / `acquire_timeout` / `test_before_acquire` / `ping_after_idle` | `config/database.php` |
| `TestDatabase::fresh::<M>` / `sqlite_memory` / `execute_unprepared` / `fetch_one` / `fetch_all` | trait de teste `RefreshDatabase` |

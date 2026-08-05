# Testes de banco de dados

O complemento específico de banco de dados para [Testes](testing.md).
Enquanto aquele capítulo cobre o harness de teste -
`#[suprnova_test]`, `describe!` / `test!`, `expect!`, e os fakes em
processo - este cobre o que muda quando seu teste precisa de um banco
de dados: como o `TestDatabase` constrói um para você, como o
isolamento de fato funciona, onde factories e seeders se encaixam, e
quando um SQLite em memória é ou não suficiente.

## Os dois construtores

Todo teste de banco de dados começa construindo um `TestDatabase`.
Dois construtores, duas intenções.

### `TestDatabase::fresh::<Migrator>()`

Constrói um banco de dados SQLite em memória, executa seu migrator de
ponta a ponta, e registra a conexão no contêiner de teste para que
qualquer código que chame `DB::connection()` ou
`App::resolve::<DbConnection>()` resolva a ele. Este é o padrão certo
para tudo que toca em esquema real.

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn user_lifecycle_end_to_end() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);
    // Consulte diretamente quando quiser contornar a superfície do
    // model:
    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`Migrator` é a implementação `MigratorTrait` da sua aplicação - o
mesmo tipo que o comando `suprnova migrate` de produção executa. Ao
passar o migrator real pelo esquema de teste você torna o drift de
esquema impossível: uma coluna que o migrator esqueceu de adicionar
não pode estar presente silenciosamente no BD de teste.

A macro `test_database!()` é açúcar sintático para o caso comum
(`crate::migrations::Migrator`):

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();          // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}

// Ou com um caminho de migrator customizado:
let db = test_database!(my_crate::CustomMigrator);
```

### `TestDatabase::sqlite_memory()`

Mesma fiação de contêiner e registry, mas **não executa nenhum
migrator**. Use isto quando o teste quer controle preciso sobre a
forma da coluna - tipicamente round-trips de cast, testes de
superfície SQL do construtor de consultas, ou casos extremos em nível
de driver em que um migrator completo é excesso ou ruído:

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared(
    "CREATE TABLE casts_t (id INTEGER PRIMARY KEY, payload BLOB)",
)
.await
.unwrap();

// Então escreva diretamente e leia de volta com os helpers tipados:
let row = db.fetch_one(
    "INSERT INTO casts_t (payload) VALUES (?) RETURNING id, payload",
    vec![sea_orm::Value::Bytes(Some(Box::new(b"hello".to_vec())))],
).await.unwrap();
```

`sqlite_memory()` é a fundação sobre a qual `fresh()` é construído -
`fresh` a chama e então executa seu migrator. Qualquer coisa que você
consiga fazer com `fresh` você consegue fazer aqui; você só traz seu
próprio DDL.

### `execute_unprepared`, `fetch_one`, `fetch_all`

`TestDatabase` re-exporta as três formas de execução do SeaORM que
você mais usa em testes, para que os arquivos de teste não precisem
importar `ConnectionTrait`:

| Método | Use para |
| --- | --- |
| `execute_unprepared(sql)` | DDL ou DML sem placeholders. Retorna `Result<(), FrameworkError>` |
| `fetch_one(sql, bindings)` | Um SELECT de uma linha. Falha se zero linhas |
| `fetch_all(sql, bindings)` | Um SELECT de todas as linhas |

Os bindings são `Vec<sea_orm::Value>` - a mesma forma que o caminho
de consulta de produção usa. O backend da conexão (SQLite para os
dois construtores) é fornecido para você, então um placeholder `?` é
correto.

## Como o isolamento de fato funciona

O modelo de um banco de dados novo por teste é o mecanismo de
isolamento. Toda chamada a `fresh()` ou `sqlite_memory()` abre uma
nova conexão `sqlite::memory:`, que no SQLite é uma instância de
banco de dados inteiramente separada - sem esquema compartilhado, sem
linhas compartilhadas, nenhum outro teste consegue ver dentro dela.
Não há wrapper de transação, nenhuma trait `RefreshDatabase` para
aderir e nenhum rollback para lembrar: o *próximo* teste ganha um BD
vazio limpo porque ele constrói o seu próprio.

Quando o valor `TestDatabase` dropa, três coisas acontecem, nesta
ordem:

1. A `TestContainerGuard` mantida limpa o contêiner de teste
   thread-local, então qualquer `App::get::<DbConnection>()`
   subsequente não encontra mais a conexão de teste.
2. Se esta foi a *última* `TestContainerGuard` viva no processo, o
   [`ConnectionRegistry`](database.md#named-connections) nomeado é
   apagado. (Um refcount sobre `FAKE_GUARDS` garante que o drop de um
   teste interno não pode apagar um nome de conexão do qual um teste
   externo concorrente ainda depende - a armadilha permanente que
   motivou o refcount.)
3. A própria conexão SQLite dropa, o que destrói o banco de dados em
   memória.

Porque o estado é reconstruído em vez de revertido, o isolamento é
mais forte do que o wrapping `BEGIN`/`ROLLBACK`: não há estado
commitado para sobreviver por engano, nenhuma peculiaridade de
transação aninhada, nenhum drift de contador de sequência entre
testes. O custo é que você paga por executar o migrator uma vez por
teste (insignificante para SQLite na maioria dos esquemas; se isso se
tornar um custo real, veja "Compartilhando um banco de dados migrado
entre testes" abaixo).

## Por que o pool é fixado em uma conexão

Os dois construtores constroem o banco de dados com
`max_connections(1)` e `min_connections(1)`. Isso é estrutural para
`sqlite::memory:`, não uma política genérica.

`sqlite::memory:` é um banco de dados por conexão - cada *nova*
conexão no pool seria uma instância SQLite separada e vazia. Um pool
de tamanho 2 significaria que metade das suas consultas vê o banco de
dados migrado e metade vê um vazio. Fixar o pool em uma conexão faz
com que toda consulta no teste caia sobre o mesmo banco de dados em
memória contra o qual o migrator rodou.

A consequência: um teste que exercita concorrência de conexão real
(duas transações competindo, roteamento de replica, um worker de
fila atingindo o BD enquanto um handler de solicitação faz o mesmo)
precisa de um banco de dados real. Veja "Quando o SQLite em memória
não é suficiente" abaixo.

## Factories em testes

Factories produzem instâncias de model randomizadas e (opcionalmente)
as persistem. O caminho de persistência resolve a conexão de teste
vinculada automaticamente - não há fiação do lado da factory para
testes.

```rust
use crate::factories::UserFactory;

#[tokio::test]
async fn factory_round_trip() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // Apenas em memória: mais rápido, sem round trip de BD.
    let alice = UserFactory::new()
        .with(|u| u.email = "alice@example.com".into())
        .make();
    assert_eq!(alice.email, "alice@example.com");

    // Persiste um + retorna o model pós-insert (id atribuído).
    let bob = UserFactory::new().create().await.unwrap();
    assert!(bob.id > 0);

    // Em massa: persiste 50 em sequência.
    let many = UserFactory::times(50).create_many().await.unwrap();
    assert_eq!(many.len(), 50);
}
```

Dois padrões que vale a pena conhecer:

**Inserções de factory contornam eventos de model.** A impl
`Persistable` que sustenta `create()` / `create_many()` escreve
diretamente através de `ActiveModelTrait::insert` do SeaORM - ela
*não* passa pela superfície `Model::create` que despacha `Creating` /
`Created` / `Saving` / `Saved`. Um teste que afirma "nenhum observer
dispara enquanto construímos a fixture" não precisa de nada especial;
um teste que afirma "o observer de `Created` DISPAROU" precisa
acionar `Model::create(...)` (ou `save()`) em vez de uma factory.

**`create_many` não transaciona.** As inserções são sequenciais. Se
uma linha posterior falhar, as linhas anteriores não são revertidas.
Envolva a chamada na sua própria `DB::transaction` se um teste exigir
atomicidade:

```rust
DB::transaction(|tx| async move {
    UserFactory::times(50).create_many().await?;
    PostFactory::times(200).create_many().await?;
    Ok::<_, FrameworkError>(())
}).await.unwrap();
```

Veja [Eloquent → Factories](eloquent-factories.md) para a superfície
completa de factory (states, sequences, relações `with`, `count`,
`times`, `make_one` / `create_one`).

## Seeders em testes

Seeders são funções que você registrou no registry de seeders do
framework sob um nome estável. Dois padrões para acioná-los a partir
de testes, um para cada eixo de intenção.

### Executar um único seeder pelo nome

```rust
use suprnova::seed;
use my_app::seeders::UsersSeeder;

#[tokio::test]
async fn users_seeder_populates_fixtures() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<UsersSeeder>();
    seed::run_one("UsersSeeder").await.unwrap();

    let count = User::query().count().await.unwrap();
    assert!(count > 0);
}
```

### Executar o conjunto completo de seeders do bootstrap

```rust
use serial_test::serial;
use suprnova::seed;

#[tokio::test]
#[serial]
async fn full_seed_lands_expected_row_counts() {
    seed::clear();                              // começa de um registry vazio conhecido
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<my_app::seeders::UsersSeeder>();
    seed::register::<my_app::seeders::PostsSeeder>();
    seed::run_all().await.unwrap();

    let users = User::query().count().await.unwrap();
    let posts = Post::query().count().await.unwrap();
    assert_eq!(users, 50);
    assert_eq!(posts, 200);

    seed::clear();
}
```

Dois detalhes de contrato importantes:

**O registry de seeders é global ao processo.**
`seed::register::<S>()` insere em um `RwLock<IndexMap>` indexado por
`S::name()`. Um teste que muta o registry deve chamar `seed::clear()`
na entrada, registrar os seeders de que precisa, executar, e
`clear()` de novo na saída - e o próprio teste deve ser
`#[serial_test::serial]` para que dois testes paralelos não disputem
o registry. `#[suprnova_test]` **não** registra seeders
automaticamente; apenas a chamada explícita `seed::register::<>()` no
seu próprio `bootstrap.rs` ou no corpo do teste os coloca no registry.

**Seeds guiados por model vs. seeds guiados por factory.** Um seeder
que percorre `User::create(...)` em um `for` dispara `Creating` /
`Saving` / `Created` / `Saved` por linha e invoca todo observer
registrado. Para preenchimento em massa em que esse fanout é
indesejado, envolva o loop em `seed::without_events`:

```rust
seed::without_events(async {
    for i in 0..50 {
        User::create(attrs! { name: format!("user{i}"), email: format!("user{i}@example.com") }).await?;
    }
    Ok::<_, FrameworkError>(())
}).await?;
```

O silenciamento é **restrito à task** - apenas o trabalho realizado
dentro do future é silenciado; handlers de solicitação concorrentes e
workers de fila continuam disparando eventos normalmente. Factories
(`create_many`) já contornam o caminho de evento, então
`without_events` é desnecessário ao redor delas.

Veja [Preenchimento de dados](seeding.md) para a superfície de
autoria de seeder e [Eloquent → Factories](eloquent-factories.md)
para a relação entre os dois.

## Testes de banco de dados seguros em paralelo

`cargo test` executa testes em paralelo por thread. A expansão padrão
de `#[suprnova_test]` (que é `#[tokio::test]`, ou seja, um runtime
`current_thread` por teste) interage de forma segura com isso por
dois motivos:

- **Cada teste ganha sua própria conexão `sqlite::memory:`.** Testes
  não compartilham estado de BD.
- **A conexão vinculada vive no `TestContainer` thread-local.** Testes
  não compartilham vinculações de contêiner.

O que você não precisa pensar sobre: `DB::connection()`,
`App::resolve`, persistência de factory, escritas via trait de
model - todos esses caem de forma transparente no banco de dados
certo por teste.

O que você *precisa* pensar sobre:

| Superfície | Por que é global ao processo | Mitigação |
| --- | --- | --- |
| `ConnectionRegistry` (`DB::register_named`, `__read_replica__`) | Um único `RwLock<HashMap>` compartilhado pelo processo | `#[serial_test::serial]` para qualquer teste que registre ou leia conexões nomeadas |
| O registry de seeders | Um único `RwLock<IndexMap>` | `#[serial_test::serial]` + `seed::clear()` na entrada e na saída |
| Os registries de observer / scope do Eloquent | Indexados por `TypeId::<M>()` | Cada teste deve usar uma struct de model única, ou ser `#[serial]` e chamar o helper `clear()` do registry |
| O log de consulta nomeado (`DB::enable_query_log`) | Um único ring buffer global ao processo | `#[serial]` se as asserções leem o log |

O refcount do registry de conexão torna isso mais seguro do que
parece: um teste que mantém uma `TestContainerGuard` mantém o
registry vivo mesmo quando a guarda de um teste **vizinho** dropa.
Ainda assim você quer `#[serial]` para os testes que de fato mutam o
registry, para que suas leituras e escritas não se intercalem.

### Ressalva do runtime multi-thread

`#[suprnova_test]` se expande para `#[tokio::test]` com o runtime
`current_thread` padrão, então o caminho de contêiner thread-local
sempre funciona. Se você opta explicitamente um teste pelo runtime
multi-thread:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_io_test() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // PROBLEMA: tasks geradas com `tokio::spawn` podem executar em
    // uma worker thread diferente daquela que construiu o
    // TestDatabase. Elas não vão ver a vinculação thread-local do
    // TestContainer, e DB::connection() vai retornar o valor do
    // contêiner global (de produção) ou dar erro.
}
```

Duas correções, dependendo do que o teste faz:

1. **Acesso direto à conexão** - `db.conn()` ainda retorna a
   `&DatabaseConnection` certa independentemente de qual worker
   thread a lê. Se o teste só fala com o BD através do handle `db`
   (não através de `DB::connection()`), o runtime multi-thread está
   ok.

2. **`TestContainer::scope`** - envolva o corpo do teste em
   `TestContainer::scope(async { ... }).await` e vincule seus fakes
   (e a conexão de BD) dentro dele. O escopo vincula o contêiner à
   camada task-local, que é preservada através de awaits mesmo
   quando o runtime salta o future entre worker threads. Para
   sub-tasks geradas, use `TestContainer::spawn` (não o
   `tokio::spawn` puro) para que o contêiner task-local seja capturado
   e reinstalado dentro do future gerado.

Veja [Contêiner de serviços → Ordem de lookup](container.md) para o
esquema completo de camadas task-local / thread-local / global.

## SQLite em memória vs. um Postgres / MySQL / MariaDB real

`TestDatabase` é intencionalmente exclusivo para SQLite. O driver é
fixo em `sqlite::memory:`; não há `TestDatabase::postgres()`,
`fresh_with_url()`, ou variante guiada por env. Para a vasta maioria
da superfície de teste - CRUD de model, forma do construtor de
consultas, round-trips de cast, carregamento de relacionamento, ordem
de disparo de observer, semântica de soft-delete - o SQLite em
memória é a ferramenta certa: zero setup, zero rede, milissegundos
por teste, isolamento perfeito, nenhum serviço externo para manter
vivo na CI.

Existem quatro casos em que o SQLite em memória não é suficiente:

1. **SQL específico de driver.** Uma consulta que usa `LATERAL` do
   Postgres, operadores `JSONB`, `ON CONFLICT ... WHERE`, funções de
   janela do MySQL, ou qualquer outra superfície específica de
   dialeto não vai executar no SQLite. O caminho de model+construtor
   tenta se manter genérico, mas um teste de SQL bruto que afirma uma
   saída no formato Postgres precisa de Postgres.
2. **Concorrência sob contenção de conexão real.** O SQLite em
   memória é de conexão única (veja "Por que o pool é fixado em uma
   conexão"). Testes que competem duas transações, exercitam
   roteamento de read-replica sob carga, ou medem retry de deadlock
   precisam de um servidor multi-conexão.
3. **Superfícies vetoriais / NoSQL / temporais.** O driver `VECTOR`
   do MariaDB do Suprnova, a integração com Qdrant, a integração com
   Pinecone, e drivers não-SQL semelhantes não podem ser modelados no
   SQLite de forma alguma.
4. **Testes de fumaça de paridade de produção.** Um punhado de
   testes "isso de fato funciona no BD real para o qual fazemos
   deploy?", isolados para a CI, vale a pena manter mesmo quando a
   camada de teste unitário é SQLite.

Para os quatro casos o padrão é o mesmo: saia inteiramente de
`TestDatabase`, construa um `DbConnection` contra uma variável de env
no estilo `DATABASE_URL` fornecida pelo operador, faça o teste
verificar a env e pular quando ela estiver ausente, e marque-o como
`#[serial]` para que dois deles não disputem o mesmo banco de dados
real. O padrão `MARIADB_URL` em
`framework/tests/vector_mariadb.rs` é o exemplo canônico:

```rust
use serial_test::serial;
use suprnova::database::{DatabaseConfig, DbConnection};

async fn maybe_real_db(test_name: &str) -> Option<DbConnection> {
    let url = match std::env::var("POSTGRES_TEST_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("[{test_name}] skipping: POSTGRES_TEST_URL not set");
            return None;
        }
    };
    let config = DatabaseConfig::builder().url(&url).build();
    Some(DbConnection::connect(&config).await.expect("real DB connects"))
}

#[tokio::test]
#[serial]
async fn jsonb_operator_works_against_postgres() {
    let Some(conn) = maybe_real_db("jsonb_operator_works_against_postgres").await else {
        return;
    };
    // Execute SQL específico de Postgres diretamente contra `conn`.
}
```

A convenção padrão: nomeie a variável de env a partir do driver alvo
(`POSTGRES_TEST_URL`, `MYSQL_TEST_URL`, `MARIADB_URL`), imprima uma
linha de skip para que um desenvolvedor executando a suíte localmente
veja que o teste foi pulado (não que passou silenciosamente), e
documente a variável de env no doc-comment inicial do módulo de teste
para que a CI possa conectá-la.

## Um exemplo trabalhado

O padrão completo de dogfood da aplicação, combinando tudo neste
capítulo:

```rust
use app::migrations::Migrator;
use app::models::posts::Post;
use app::models::users::User;
use serial_test::serial;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, seed, FrameworkError};

#[tokio::test]
#[serial]
async fn users_and_posts_full_seed_round_trip() {
    // 1. Registry de seeders vazio.
    seed::clear();

    // 2. BD novo em memória com o migrator da aplicação.
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // 3. Registra os seeders com que o teste se importa.
    seed::register::<app::seeders::UsersSeeder>();
    seed::register::<app::seeders::PostsSeeder>();

    // 4. Aciona o seed dentro de without_events para que o fanout de
    //    observer não tente enfileirar jobs (nenhuma fila está
    //    rodando aqui).
    seed::without_events(async {
        seed::run_all().await
    }).await.unwrap();

    // 5. Lê de volta através da superfície de model e da conexão
    //    bruta.
    let user_count = User::query().count().await.unwrap();
    assert_eq!(user_count, 50);

    let raw_post_count = db.fetch_one(
        "SELECT COUNT(*) AS n FROM posts",
        vec![],
    ).await.unwrap();
    let n: i64 = raw_post_count.try_get("", "n").unwrap();
    assert_eq!(n, 200);

    // 6. Exercita o caminho do observer cancelável em um model novo.
    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    }).await.unwrap();
    assert!(alice.id > 0);

    seed::clear();
}
```

O passo 5 é a parte que comprova a fiação: a consulta de model e o
`fetch_one` bruto estão ambos lendo o mesmo banco de dados em
memória - a superfície de model porque a busca de `DB::connection()`
encontrou a vinculação do `TestContainer`, o `fetch_one` bruto porque
`db.conn()` retorna essa mesma conexão diretamente.

## Referências cruzadas

- [Testes](testing.md) - o harness de teste, `expect!`, `describe!`,
  `test!`, fakes.
- [Banco de dados](database.md#testing) - a seção de teste de
  superfície que introduz o `TestDatabase`.
- [Eloquent → Factories](eloquent-factories.md) - sintaxe de
  definição de factory, states, sequences, relações.
- [Preenchimento de dados](seeding.md) - autoria de seeder, ordenação,
  idempotência.
- [Contêiner de serviços](container.md) - lookup task-local vs.
  thread-local vs. global, que decide a que `DB::connection()`
  resolve dentro de um teste.
- [Mocking e Fakes](mocking.md) - `Storage::fake`, `Mail::fake`,
  `Queue::fake`, `Notification::fake`, e o padrão de vinculação por
  trait para trocar clientes HTTP fake e outras superfícies externas.
- [Testes HTTP](http-tests.md) - acionando handlers através da stack
  de roteamento com um `TestDatabase` vinculado.

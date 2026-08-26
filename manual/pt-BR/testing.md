# Testes

Este é o capítulo hub para a superfície de testes do Suprnova - as
macros, o banco de dados em processo, os fakes de contêiner, e os
helpers de chave de criptografia que seus binários de teste buscam.
Os capítulos que vão a fundo vivem ao lado dele:
[Testes HTTP](http-tests.md) para rotas + middleware,
[Testes de banco de dados](database-testing.md) para tudo em torno de
`TestDatabase`, [Mocking e Fakes](mocking.md) para as sete superfícies
externas (Mail, Notify, Queue, Bus, Events, Storage, cliente HTTP).
Leia este para aprender o que tem na caixa; salte para um capítulo
irmão quando precisar da forma longa.

## As peças

| Peça | Papel |
|---|---|
| `#[tokio::test]` + `TestDatabase::fresh::<Migrator>()` | O cavalo de batalha padrão - todo teste real no framework usa isso |
| `#[suprnova_test]` | Açúcar de attribute macro - executa `App::init()` + `App::boot_services()` e constrói um `TestDatabase` para você |
| `describe!` + `test!` | Macros de agrupamento no formato do Jest, pareadas com `expect!` para saída de falha nomeada |
| `expect!` | Macro de asserção fluente com matchers tipados (igualdade, option, result, string, vec, ordenação) |
| `TestDatabase::fresh` / `sqlite_memory` | SQLite em memória + registro no contêiner, com ou sem seu migrator |
| `TestContainer::fake` / `scope` / `spawn` | Overrides de DI thread-local ou task-local, herméticos entre testes paralelos |
| `install_test_encryption_key[ring]` | `APP_KEY` determinística para testes que tocam casts criptografados ou payloads assinados |
| Helpers `fake()` por superfície | Mail, Notify, Queue, Bus, Events, Storage, HTTP - veja [Mocking](mocking.md) |
| `TestResponse` | Asserções fluentes sobre a tripla `(status, headers, body)` de um teste HTTP - veja [Testes HTTP](http-tests.md#fluent-response-assertions-with-testresponse) |
| `AssertableInertia` | Asserções fluentes sobre um objeto de página Inertia - veja [Testes HTTP](http-tests.md#testing-inertia-responses) |

Você não vai usar tudo em um único teste. Um teste de action típico
usa os três primeiros; um teste pesado em DI acrescenta
`TestContainer`; um teste HTTP troca `TestDatabase` pelo pipeline do
`handle_request`; um teste de pagamentos instala o keyring de
criptografia.

## O cavalo de batalha padrão

Todo teste real no framework se parece com isto:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn create_user_persists_it() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);

    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`TestDatabase::fresh::<M>()` abre uma conexão `sqlite::memory:` nova,
executa seu migrator de ponta a ponta, e registra a conexão no
contêiner de teste. Qualquer código que chame `DB::connection()` ou
`App::resolve::<DbConnection>()` depois disso resolve para ela -
incluindo o construtor de consultas `#[suprnova::model]` e qualquer
serviço que você resolveu a partir do contêiner. Quando o
`TestDatabase` dropa, o registro vai junto.

A macro `test_database!()` é açúcar de uma linha para o caso
`crate::migrations::Migrator`:

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();         // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}
```

Para testes que querem controle preciso sobre a forma da coluna
(round-trips de cast, superfície SQL do construtor de consultas), use
`TestDatabase::sqlite_memory()` - a mesma fiação de contêiner, sem
migrator. O DDL é seu. Veja
[Testes de banco de dados](database-testing.md) para o catálogo
completo, mais os helpers `execute_unprepared` / `fetch_one` /
`fetch_all`.

## `#[suprnova_test]` - quando você quer o açúcar

`#[suprnova_test]` é uma attribute macro que envolve `#[tokio::test]`,
chama `App::init()` + `App::boot_services()` para que tipos
`#[injectable]` resolvam, e vincula um `TestDatabase` novo. É açúcar
opcional sobre a forma explícita acima, útil quando um teste resolve
serviços registrados no contêiner:

```rust
use suprnova::suprnova_test;
use suprnova::{App, testing::TestDatabase};

#[suprnova_test]
async fn create_user_via_action(db: TestDatabase) {
    let action = App::resolve::<CreateUserAction>().unwrap();
    let user = action.execute("test@example.com").await.unwrap();

    assert_eq!(user.email, "test@example.com");
    assert!(user.id > 0);
}
```

Se a função recebe um parâmetro `TestDatabase` (pelo nome), a macro
vincula o banco de dados novo a esse nome. Se não recebe, o banco de
dados ainda é construído e registrado (então `DB::connection()`
funciona) - ele simplesmente não é vinculado a uma local.

Sobrescreva o migrator com a chave `migrator = …`:

```rust
#[suprnova_test(migrator = my_crate::tests::IsolatedMigrator)]
async fn create_user_with_isolated_schema(db: TestDatabase) {
    // ...
}
```

Chaves desconhecidas são um erro de compilação (um erro de digitação
`migrtor = …` não vai silenciosamente manter o migrator padrão).

## `describe!` e `test!` - quando o agrupamento ajuda

Para arquivos de teste em que a mesma action tem muitos casos, o par
`describe!` + `test!`, no formato do Jest, te dá agrupamento aninhado
e saída de falha nomeada:

```rust
use suprnova::{App, describe, test, expect, testing::TestDatabase};
use crate::migrations::Migrator;

describe!("ListTodosAction", {
    test!("returns empty list when no todos exist", async fn(db: TestDatabase) {
        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_be_empty();
    });

    test!("returns all todos", async fn(db: TestDatabase) {
        Todo::create(attrs! { title: "Buy bread" }).await.unwrap();
        Todo::create(attrs! { title: "Walk dog" }).await.unwrap();

        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_have_length(2);
    });

    describe!("with pagination", {
        test!("returns first page", async fn(db: TestDatabase) {
            // grupos aninhados se compõem
        });
    });
});
```

`test!` aceita três formas:

```rust
// Teste assíncrono com parâmetro TestDatabase
test!("creates a user", async fn(db: TestDatabase) { … });

// Teste assíncrono sem banco de dados
test!("calculates the right sum", async fn() { … });

// Teste síncrono
test!("adds numbers", fn() { … });
```

O wrapper de teste nomeado costura o nome do teste através da
maquinaria do `expect!`, para que uma falha apareça:

```text
Test: "returns all todos"
  at src/actions/todo_action.rs:25

  expect!(actual).to_equal(expected)

  Expected: 2
  Received: 0
```

Sem `describe!`/`test!` você recebe a saída padrão do `panic!`. Com
eles, a localização e o nome de teste legível por humanos lideram a
mensagem.

## `expect!` - o catálogo de matchers

`expect!(value)` retorna um wrapper `Expect<T>`. Os matchers são
tipados para `T` - chamar `to_be_some()` em uma `String` é um erro de
compilação, não um panic em runtime.

```rust
use suprnova::expect;

// Igualdade (T: Debug + PartialEq)
expect!(actual).to_equal(expected);
expect!(actual).to_not_equal(unexpected);

// Booleano
expect!(condition).to_be_true();
expect!(condition).to_be_false();

// Option<T>
expect!(option).to_be_some();
expect!(option).to_be_none();
expect!(option).to_contain_value(5);     // verificação Some(5)

// Result<T, E>
expect!(result).to_be_ok();
expect!(result).to_be_err();

// String / &str
expect!(s).to_contain("substring");
expect!(s).to_start_with("prefix");
expect!(s).to_end_with("suffix");
expect!(s).to_have_length(10);
expect!(s).to_be_empty();

// Vec<T>
expect!(v).to_have_length(3);
expect!(v).to_contain(&item);
expect!(v).to_be_empty();

// Ordenação (T: Debug + PartialOrd)
expect!(10).to_be_greater_than(5);
expect!(5).to_be_less_than(10);
expect!(10).to_be_greater_than_or_equal(10);
expect!(5).to_be_less_than_or_equal(5);
```

Você pode usar `expect!` fora de `test!` - o arquivo/linha na mensagem
de falha vem de `concat!(file!(), ":", line!())`. O cabeçalho de teste
nomeado é a única coisa que a macro não adiciona por conta própria.

## `TestContainer` - fakes de DI que não vazam

O capítulo do contêiner cobre o [lookup em três camadas](container.md)
em detalhe. Para testes, os dois pontos de entrada são
`TestContainer::fake()` (thread-local) e
`TestContainer::scope(…).await` (task-local).

### Thread-local, o caso comum

`TestContainer::fake()` retorna uma guarda. Até a guarda dropar,
escritas de `TestContainer::singleton` / `bind` / `factory` caem na
camada de override thread-local e fazem shadowing do contêiner
global:

```rust
use std::sync::Arc;
use suprnova::App;
use suprnova::testing::TestContainer;

#[tokio::test]
async fn order_dispatches_email() {
    let _guard = TestContainer::fake();

    let fake = Arc::new(FakeEmailGateway::new());
    let probe = Arc::clone(&fake);
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.unwrap();

    assert_eq!(probe.sent_count(), 1);
}
```

`TestDatabase::fresh` / `sqlite_memory` instalam sua própria guarda
`TestContainer::fake` internamente - você não as empilha, a menos que
esteja testando o próprio registry.

### Task-local, para runtimes `multi_thread`

A camada thread-local é definida na thread do SO que chamou `fake()`,
seja ela qual for. Um runtime tokio `multi_thread` pode migrar sua
future para outra worker thread através de um `.await`, e o override
desaparece silenciosamente. `TestContainer::scope` resolve isso
vinculando o override à future em vez disso:

```rust
use suprnova::testing::TestContainer;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_worker_safe() {
    TestContainer::scope(async {
        TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
        do_async_work_that_may_hop_workers().await;
    })
    .await;
}
```

Sub-tasks criadas com `tokio::spawn` não herdam os task-locals do
tokio; use `TestContainer::spawn` em vez disso - ele captura o
contêiner do escopo atual e o reinstala dentro da future spawnada:

```rust
TestContainer::scope(async {
    TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
    let h = TestContainer::spawn(async {
        App::make::<dyn HttpClient>().unwrap()  // vê o fake
    });
    let _client = h.await.unwrap();
})
.await;
```

### Por que existe um refcount `FAKE_GUARDS`

O contêiner thread-local é por teste, mas o Suprnova também tem um
`ConnectionRegistry` global ao processo, indexado por nome
(`__read_replica__`, labels de conexão customizados), que sobrevive a
um reset thread-local. Uma impl `Drop` ingênua chamaria
`ConnectionRegistry::clear()` toda vez que *qualquer*
`TestContainerGuard` fosse embora - apagando a conexão nomeada de
outro teste concorrente no meio da sua execução.

A correção é um `AtomicUsize` (`FAKE_GUARDS`) global ao processo.
`fake()` o incrementa; `drop` o decrementa; somente a transição de
volta a zero limpa o registry nomeado. Dois testes paralelos usando
`__read_replica__` estão seguros: a guarda que dropar por último é a
dona da limpeza.

Você não chama isso de dentro de um teste - isso executa a partir do
`Drop` do `TestContainerGuard`. Você só precisa saber que existe se
estiver debugando um sintoma de "conexão nomeada desapareceu no meio
do teste", o que geralmente significa que um teste irmão esqueceu de
esperar sua própria guarda dropar primeiro.

## Helpers de teste para chave de criptografia

Testes que exercitam casts criptografados (`casts = { secret =
AsEncrypted }` em um `#[model(...)]`), payloads assinados, ou o
fallback de chave anterior do keyring precisam de uma `APP_KEY`
instalada em processo. O framework distribui dois helpers
só-para-teste sob a feature `testing`:

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn cast_roundtrip() {
    install_test_encryption_key();   // idempotente; chave determinística de 32 bytes zerados
    let db = TestDatabase::sqlite_memory().await.unwrap();
    // … criptografe + leia de volta …
}
```

`install_test_encryption_key` é idempotente - a facade `Crypt`
subjacente é apoiada em `OnceLock`, então a segunda chamada é um
no-op. A maioria dos binários de teste de cast a chama a partir de
todo teste que toca em um cast criptografado; a primeira vence, o
resto é de graça.

Para testes de rotação (escritas sob a chave antiga, leituras sob a
nova), use a variante de keyring:

```rust
use suprnova::crypto::EncryptionKey;
use suprnova::testing::install_test_encryption_keyring;

let new = EncryptionKey::from_base64("...").unwrap();
let old = EncryptionKey::from_base64("...").unwrap();
let installed = install_test_encryption_keyring(new, vec![old]);
assert!(installed, "first install wins");
```

O helper de keyring retorna `true` somente se a chamada de fato
instalou o ring (o `OnceLock` estava vazio). Para cunhar texto cifrado
sob uma chave arbitrária para um teste de rotação, use
`suprnova::crypto::_test_encrypt_with` em vez de instalar duas vezes.

Os dois helpers são `#[doc(hidden)]` na camada de crypto e
re-exportados sob o módulo `testing` - eles são só-para-teste e
contornam o caminho de validação de `APP_KEY` de produção.

## A feature `testing` e os builds de produção

O `suprnova` expõe seus helpers de teste (`Storage::fake()`,
`TestContainer`, `TestDatabase`, hooks de rotação de cripto como
`_test_install_key`) atrás de uma feature Cargo chamada `testing`. A
feature está no conjunto padrão, então suítes de teste consumidoras a
recebem de graça:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.4" }

[dev-dependencies]
# `testing` vem ligada transitivamente pela dependência acima - nada extra.
```

Os hooks são `#[doc(hidden)]` e prefixados com `_test_`, então não são
alcançáveis a partir de código de aplicação idiomático mesmo com a
feature ligada. A salvaguarda que sustenta tudo é
`Server::from_config`: ela valida `APP_KEY` em **todo** boot, não só
quando o keyring está sem inicializar. Uma chave de teste
pré-instalada não consegue burlar essa checagem - o boot falha rápido
se `APP_KEY` estiver ausente ou malformada, independentemente de algo
em processo ter pré-instalado uma chave.

Se você prefere que os helpers não sejam linkados no seu artefato de
produção de forma alguma (defesa em profundidade), dependa de
`suprnova` com as features padrão desligadas e habilite apenas o que
você publica:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.4", default-features = false, features = ["..."] }

[dev-dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.4", features = ["testing", "..."] }
```

Isto é um aperto, não uma correção - a validação no boot fecha o
exploit real, independentemente da postura que você escolher.

### Por que Suprnova diverge

O harness de teste PHP do Laravel ganha isolamento de testes paralelos
quase de graça porque o runtime é de thread única por solicitação e os
testes fazem fork de um novo processo por arquivo. O binário de teste
do Suprnova é um processo rodando muitos `#[tokio::test]`s em uma ou
mais threads de worker concorrentemente. Um único contêiner global
significaria que o fake de um teste vaza para a busca do teste
seguinte no instante em que os dois se sobrepõem numa thread de
worker.

É por isso que `TestContainer` tem os dois sabores - thread-local para
o caso comum `current_thread`, task-local para `multi_thread`. O clear
de `FAKE_GUARDS` com contagem de referências sobre o
`ConnectionRegistry` global do processo existe pelo mesmo motivo:
estado compartilhado que não dá para tornar por teste precisa ao menos
saber que não deve se apagar enquanto outro teste ainda se apoia nele.

O catálogo de matchers (`expect!`) é tipado porque o Rust permite. O
`expect(x).toBeSome()` do Jest só sabe em runtime se `x` é um
`Option`; o `Expect<T>` do Suprnova sabe em tempo de compilação, então
um matcher errado é um erro de compilação, não um teste instável.

## Onde cada peça vive

| Peça | Fonte |
|---|---|
| `#[suprnova_test]` attribute macro | `suprnova-macros/src/suprnova_test.rs` |
| `describe!` / `test!` proc-macros | `suprnova-macros/src/describe.rs`, `test_macro.rs` |
| `expect!` macro + matchers `Expect<T>` | `framework/src/lib.rs` (macro), `framework/src/testing/expect.rs` (impls) |
| `TestResponse` | `framework/src/testing/response.rs` |
| `AssertableInertia`, `ReloadRequest` | `framework/src/testing/inertia.rs` |
| `TestDatabase::fresh` / `sqlite_memory` / helpers | `framework/src/database/testing.rs` |
| macro `test_database!` | `framework/src/database/testing.rs` |
| `TestContainer` + `TestContainerGuard` + `FAKE_GUARDS` | `framework/src/container/testing.rs` |
| `install_test_encryption_key[ring]` | `framework/src/testing/mod.rs` |
| Fakes por superfície (Mail, Notify, Queue, Bus, Events, Storage, HTTP) | submódulos `testing` por domínio - veja [Mocking](mocking.md) |

## Executando os testes

As invocações padrão do cargo se aplicam:

```bash
# Workspace inteiro
cargo test --workspace

# Um crate
cargo test -p suprnova

# Um teste por nome (correspondência de substring)
cargo test create_user_persists_it

# Com saída de println! e dbg!
cargo test -- --nocapture
```

O Suprnova não distribui seu próprio test runner; o framework se
integra com o do cargo. Testes de banco de dados executam em paralelo
por padrão - o contêiner thread-local e o SQLite em memória por teste
são desenhados exatamente para isso.

## Próximos passos

- [Testes HTTP](http-tests.md) - dirigindo o pipeline completo de
  solicitação através de `handle_request`
- [Testes de banco de dados](database-testing.md) - `TestDatabase`,
  factories em testes, seeders em testes, testes de BD seguros em
  paralelo
- [Mocking e Fakes](mocking.md) - os sete fakes de superfície externa
  e os padrões que compartilham
- [Contêiner de serviços](container.md) - o lookup em três camadas
  que `TestContainer` sobrescreve
- [Modelo de erros](error-model.md) - as formas de `FrameworkError`
  sobre as quais você vai fazer asserções

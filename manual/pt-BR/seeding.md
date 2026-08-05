# Preenchimento de dados

Seeders preenchem o banco de dados com dados de fixture - as linhas
que sua app precisa antes de qualquer usuário real ter feito
qualquer coisa. Pense em uma conta de admin padrão, a lista canônica
de países, os posts de demonstração no ambiente de staging, os 50
usuários + 200 posts de que o loop de iteração do seu dev local
depende. Eles são a contraparte em runtime das
[migrações](migrations.md): migrações constroem o esquema vazio,
seeders o preenchem.

Um seeder é um tipo de tamanho zero que implementa a trait `Seeder`.
O framework mantém um registry global ao processo e ordenado; o
comando `console db:seed` por projeto executa todo seeder registrado
na ordem de registro, ou um seeder específico via `--class=<Name>`.
A maioria dos seeders termina sendo algumas linhas que chamam uma
[factory de model](eloquent.md) e deixam a factory fazer o trabalho
de geração de linhas.

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

Registre-o uma vez no boot:

```rust
// src/bootstrap.rs
suprnova::seed::register::<crate::seeders::UsersSeeder>();
```

Então:

```bash
cargo run --bin console -- db:seed
# running seeder UsersSeeder
# (50 rows inserted)
```

Esse é o loop completo. O resto deste capítulo cobre as convenções
de layout, os padrões maiores de composição de registry, a flag de
direcionamento `--class`, a integração com factory, a válvula de
escape `without_events`, e a decisão de quando preencher, migrar, ou
usar factory.

## Escrevendo um seeder

Um seeder é um tipo unitário mais uma impl de `Seeder`. `name()` é a
chave no registry (também o que `db:seed --class=<Name>` compara), e
`run()` é a fn async que realiza as inserções.

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

`Seeder` é re-exportado na raiz do crate, então
`use suprnova::Seeder` já basta - você não precisa alcançar
`suprnova::seed::Seeder`. `async_trait` também é re-exportado
(`use suprnova::async_trait`) porque o método da trait retorna um
future e o Rust ainda não permite `async fn` em traits sem ele.

O tipo de retorno `FrameworkError` é o mesmo envelope de erro que
toda outra superfície async do framework usa; propagar o `?` para
fora de uma chamada de factory ou de um `Model::create` é a forma
esperada. Veja [Modelo de erros](error-model.md) para a taxonomia
completa.

### Convenção de layout

Espelhe o diretório `database/seeders/` do Laravel, mas na raiz do
código-fonte:

```
src/
├── bootstrap.rs
├── factories/
│   ├── mod.rs
│   ├── user_factory.rs
│   └── post_factory.rs
├── seeders/
│   ├── mod.rs              // pub mod base_seeder; pub use base_seeder::BaseSeeder;
│   └── base_seeder.rs      // impl de Seeder, registrada em bootstrap.rs
└── …
```

Gere o arquivo à mão - não há gerador `make:seeder` (isto é um
arquivo com cerca de dez linhas de boilerplate). As factories que o
seeder chama recebem o mesmo tratamento.

### Um seeder que executa outros seeders

O idioma do Laravel de um único `DatabaseSeeder::run` de nível
superior que orquestra os seeds por model funciona aqui também. Em
vez de registrar cinco seeders pequenos no bootstrap e confiar na
ordem de registro deles, registre um seeder composto e chame o resto
deles você mesmo:

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // 50 usuários primeiro - a factory de posts gera author_id
        // em 1..=50, então as referências resolvem.
        UserFactory::new().count(50).create_many().await?;

        // 200 posts referenciando os ids de usuário acima.
        PostFactory::new().count(200).create_many().await?;

        Ok(())
    }
}
```

Este é o padrão recomendado. Ele mantém a ordem de dependência
(`users` antes de `posts`) dentro do seeder em vez de espalhada pelo
arquivo de bootstrap, e `db:seed --class=BaseSeeder` é uma invocação
de alvo único que executa o pacote inteiro.

Se você quiser encadear seeders pelo nome em vez de por chamada
direta de factory, use `seed::run_one` de dentro do seeder composto:

```rust
async fn run() -> Result<(), FrameworkError> {
    suprnova::seed::run_one("UsersSeeder").await?;
    suprnova::seed::run_one("PostsSeeder").await?;
    suprnova::seed::run_one("CommentsSeeder").await?;
    Ok(())
}
```

Os sub-seeders ainda precisam ser registrados em `bootstrap.rs` para
que `run_one` os encontre.

## O registry de seeders

O framework mantém um mapa ordenado global ao processo
(`IndexMap<String, fn() -> _>`) de todo seeder registrado. Três
controles o governam.

### `register::<S>()`

Adicione um seeder ao registry sob seu `Seeder::name()`:

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

Duas coisas para saber sobre o registry:

- **A ordem importa.** `run_all` visita os seeders na ordem em que
  foram registrados. Se `B` precisa de linhas de `A`, registre `A`
  primeiro.
- **Registrar de novo um nome substitui no lugar.** O slot mantém
  sua posição original, o function pointer muda. Isso é
  intencional - permite que um teste vincule um seeder stub sobre o
  real sem deslocar a ordem. Em código de produção, registre cada
  seeder exatamente uma vez no boot.

### `run_all()`

Executa todo seeder registrado na ordem de registro. É isso que a
invocação nua `console db:seed` chama.

```rust
suprnova::seed::run_all().await?;
```

Para no primeiro erro. Seeders que já executaram não são
revertidos - `run_all` não envolve o batch em uma transação porque a
maioria dos seeders abrange múltiplos statements e muitos backends
não aninham transações de forma limpa. Se você precisa de semântica
de rollback, abra a transação dentro do seeder e mantenha todo o
trabalho dele dentro daquele escopo.

### `run_one(name)`

Executa um seeder nomeado sem executar os outros. Este é o motor por
trás de `db:seed --class=<Name>` e também é útil a partir de scripts
pontuais:

```rust
suprnova::seed::run_one("AdminAccountSeeder").await?;
```

Uma busca sem correspondência retorna
`FrameworkError::not_found("no seeder registered for \`X\`")`. O
comando console propaga isso para um exit não-zero e uma linha de
stderr - sem no-op silencioso.

### `count()` e `is_registered(name)`

Dois helpers de leitura, ambos úteis em testes que afirmam "o
bootstrap conectou os seeders esperados":

```rust
assert_eq!(suprnova::seed::count(), 3);
assert!(suprnova::seed::is_registered("BaseSeeder"));
```

Os dois retornam zero / false em um lock de registry envenenado
(depois de logar um erro), o que mantém os testes determinísticos
frente a um panic upstream.

## O comando `db:seed`

`db:seed` é um comando de console fornecido pelo framework - ele vem
com o framework e aterrissa automaticamente no binário `console` do
seu projeto através do mesmo registry `inventory` que capta seus
próprios `#[command]`s. Veja [Console](console.md) para a mecânica
do binário; esta seção cobre a superfície específica de seeder.

### Executa tudo

```bash
cargo run --bin console -- db:seed
```

Executa todo seeder registrado em ordem. Em um registry vazio ele
imprime um aviso no stderr (`db:seed: no seeders registered -
nothing to run`) e sai com zero - esse é o comportamento correto
para "alguém executou o comando antes de registrar qualquer coisa" e
evita que suítes de teste que não preencheram nada específico falhem.

### Executa um seeder

Três formas aceitas, em ordem crescente de quão parecidas com o
Laravel elas parecem:

```bash
cargo run --bin console -- db:seed --class=UsersSeeder
cargo run --bin console -- db:seed --class UsersSeeder
cargo run --bin console -- db:seed UsersSeeder
```

As três buscam o seeder no registry pelo nome exato e o executam. Um
nome desconhecido falha rápido:

```bash
cargo run --bin console -- db:seed --class=NotARealSeeder
# Error: no seeder registered for `NotARealSeeder`
# (exit 1)
```

Uma flag malformada (`--class` sem valor seguinte, `--class=` com
valor vazio, `--class --force`) também falha rápido, com um
diagnóstico que nomeia a forma esperada.

### A partir de um binário compilado

Em um deploy containerizado ou gerenciado por systemd, o binário
console vive em `target/release/console` (ou onde quer que seu
artefato de release aterrisse). Mesma sintaxe, sem `cargo` na frente:

```bash
./console db:seed
./console db:seed --class=BaseSeeder
```

O binário console chama
`suprnova::console::dispatch_argv(std::env::args())`, que roteia
através do mesmo registry que `cargo run --bin console --`. Não há
caminho de dispatch separado para artefatos compilados.

## Compondo com factories

Seeders quase sempre terminam chamando [factories](eloquent.md). A
trait de factory sabe como construir uma instância randomizada de um
model; o seeder sequencia as chamadas de factory e qualquer fiação
não randomizável (credenciais de admin determinísticas, linhas de
tabela de junção, uploads de arquivo).

O par mínimo factory + seeder:

```rust
// src/factories/user_factory.rs
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        User {
            id: 0,                              // persist_via_seaorm inverte a PK para NotSet
            name: "Factory User".into(),
            email: "factory@example.suprnova.app".into(),
            password: "factory-placeholder".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ..Default::default()
        }
    }
}
```

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

O construtor fluente vive em `FactoryBuilder<M>`; o que você pode
encadear antes de `create_many` casa com o Laravel:

```rust
// Constrói uma linha persistida com overrides:
let admin = UserFactory::new()
    .with(|u| u.email = "admin@example.com".into())
    .with(|u| u.role = "admin".into())
    .create()
    .await?;

// Constrói N linhas persistidas, todas admins:
UserFactory::times(5)
    .with(|u| u.role = "admin".into())
    .create_many()
    .await?;

// Estado condicional - aplica a closure apenas quando a flag está definida:
UserFactory::times(10)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

`make` / `make_one` / `make_many` são as contrapartes em memória
(sem insert) para testes unitários que não querem um round-trip de
banco de dados. Veja o capítulo [Eloquent](eloquent.md) para a
superfície completa de factory (incluindo `prepend`, `Sequence`, e a
macro `#[derive(Factory)]` que gera a struct marcadora a partir de
um attribute `#[factory(model = "…")]`).

### Idempotência é responsabilidade do seeder

`run_all` não tira snapshot nem envolve uma transação; se um seeder
insere incondicionalmente, executá-lo de novo produz duplicatas. As
duas formas padrão de tornar um seeder seguro para re-executar:

- **Reset primeiro.** O loop de "limpar e re-preencher" do dev local
  geralmente faz
  `suprnova migrate:fresh && cargo run --bin console -- db:seed` -
  `migrate:fresh` remove e reconstrói toda tabela, então o seeder
  sempre começa vazio. Esta é a forma que a maioria dos projetos usa
  no dia a dia.
- **Upsert / verificar antes.** Para um seeder que precisa coexistir
  com dados existentes (uma conta de admin padrão em produção, a
  lista canônica de países), proteja a inserção com uma consulta ou
  use uma query de upsert.

```rust
async fn run() -> Result<(), FrameworkError> {
    let exists = User::query()
        .db_where("email", "admin@example.com")
        .exists()
        .await?;

    if !exists {
        let password_hash = suprnova::hashing::hash("change-me-on-first-login")?;
        User::create(attrs!{
            email: "admin@example.com",
            name: "Admin",
            password: password_hash,
        }).await?;
    }
    Ok(())
}
```

## Silenciando eventos de model com `without_events`

Um seeder que chama `Model::create` em um loop dispara todo evento
de ciclo de vida - `Creating`, `Saving`, `Created`, `Saved` - em
cada linha. Isso desperta todo `Observer<M>` registrado, executa
todo listener de broadcast enfileirado, e pode incidentalmente
enfileirar uma centena de jobs em background que você não quer de
fato. `seed::without_events` é o análogo ao `WithoutModelEvents` do
Laravel:

```rust
use suprnova::{async_trait, FrameworkError, Seeder, seed};
use crate::models::users::User;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        seed::without_events(async {
            for i in 0..50 {
                User::create(attrs!{
                    name: format!("user{i}"),
                    email: format!("user{i}@example.com"),
                }).await?;
            }
            Ok(())
        }).await
    }
}
```

Enquanto o future interno está em await, tanto o caminho de veto
cancelável (`dispatch_cancellable`) quanto o fanout de after-event
(`dispatch_after`) fazem short-circuit para `Ok(())`. Observers ficam
silenciosos, o broadcaster não desperta, jobs downstream não são
enfileirados.

O efeito é restrito à task - apenas o trabalho realizado dentro de
`fut` é silenciado. Trabalho concorrente em outras tasks (handlers de
solicitação HTTP, workers de fila rodando em background, outros
seeders) continua disparando eventos normalmente. Chamadas aninhadas
se compõem: um bloco `without_events` interno herda a flag externa.

### Factories já contornam eventos de model

Vale saber porque isso muda quando você recorre a `without_events`:
factories persistem via `ActiveModelTrait::insert` (a impl
`Persistable` no model SeaORM), que não passa pelos métodos `create`
/ `save` da trait `Model`. Não há dispatch de model-event para
silenciar em um caminho guiado por factory. `seed::without_events` é
para código que aciona a trait `Model` diretamente - tipicamente
porque você precisa da ergonomia de forma-em-runtime que factories
contornam, ou porque você está tocando um model no meio do
preenchimento que um observer deveria reagir a em produção mas não
durante um carregamento de fixture.

Na prática: se seu seeder é uma pilha de chamadas
`UserFactory::new().create_many()`, você não precisa de
`without_events`. Se é um loop feito à mão de `User::create(attrs)`,
você provavelmente precisa.

## Usando seeders em testes

O mesmo registry que o binário console aciona é chamável a partir de
um `#[tokio::test]` - útil quando você quer um conjunto de fixture
conhecido na frente de um teste de integração:

```rust
use serial_test::serial;
use suprnova::container::testing::TestContainer;
use suprnova::{DbConnection, seed};

use app::seeders::BaseSeeder;

#[tokio::test]
#[serial]
async fn dashboard_renders_seeded_posts() {
    // Reseta o registry para que os registros de um teste anterior
    // não vazem.
    seed::clear();

    let _guard = TestContainer::fake();
    let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    app::migrations::Migrator::up(&conn, None).await.unwrap();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    // Registra o seeder que você quer, executa-o, e afirma contra o
    // banco de dados novo.
    seed::register::<BaseSeeder>();
    seed::run_all().await.unwrap();

    // …teste de controller contra os dados preenchidos…

    seed::clear();
}
```

Duas notas sobre a forma do teste:

- `#[serial]` é obrigatório quando o teste muta o registry global ao
  processo - testes paralelos compartilhando o mesmo registry vão
  competir. Adicione `serial_test` como dev-dependency no
  `Cargo.toml` do seu projeto para obter o attribute.
- `seed::clear()` é um helper `#[doc(hidden)]` só para teste. Não o
  chame a partir de código de produção; o registry é construído uma
  vez no boot e nunca é resetado.

Veja [Testes](testing.md) para as convenções mais amplas de harness
de teste (`#[suprnova_test]`, `TestContainer`,
`TestDatabase::fresh::<Migrator>()`, os fakes para toda superfície
externa).

## Quando preencher, migrar, ou usar factory

Esses três padrões colocam linhas em tabelas. A decisão geralmente é
direta, mas vale a pena nomear as linhas divisórias explicitamente
porque equipes PHP costumam borrar essa distinção.

| Você quer… | Use |
|---|---|
| Uma coluna existir | [Migração](migrations.md) |
| Uma linha que precisa existir para a app inicializar (o admin padrão, a linha singleton de config do site, a lista canônica de moedas) | **Seeder** - idempotente, executa em todo ambiente, incluindo produção |
| Um conjunto randomizado de linhas para dev local ou staging (50 usuários, 200 posts, 1000 eventos) | Seeder que chama uma factory |
| Uma linha de que um teste unitário precisa | [Factory](eloquent.md) chamada diretamente dentro do teste |
| A forma de uma linha | [Factory](eloquent.md) |

Os erros a evitar:

- **Não insira dados a partir de uma migração.** Migrações
  descrevem esquema, não estado. Uma migração que insere uma linha
  padrão vai executar uma vez no banco de dados de produção e depois
  nunca mais - no momento em que uma coluna muda, você tem uma fonte
  da verdade bifurcada entre o histórico de migração e o seeder.
  Coloque o insert em um seeder; se a produção precisa da linha,
  execute `console db:seed --class=DefaultsSeeder` como parte do
  deploy.
- **Não escreva dados de fixture no seu teste à mão.** Recorra a uma
  factory. Cinco blocos `User::create(attrs!{ … })` em um teste são
  cinco reescritas no momento em que você adiciona uma coluna NOT
  NULL. Um `UserFactory::new().create()` sobrevive.
- **Não coloque dados de produção em um seeder.** Um seeder é para
  as linhas que a aplicação exige para funcionar, não para "aqui
  estão os 8.000 registros históricos que estamos importando".
  Imports são scripts pontuais (escreva um `#[command]` para eles;
  veja [Console](console.md)).

### Por que Suprnova diverge

O Laravel traz uma classe `DatabaseSeeder` com um helper especial
`call($seeders)` que o loader de seeder do Eloquent reconhece. O
Suprnova não - o registry é um `IndexMap` plano, todo seeder é um
par, e um seeder composto chama `seed::run_one(name)` (ou
simplesmente chama as sub-factories diretamente) para encadear.

O motivo é o mesmo trade-off que você vê em outros lugares do
Suprnova: um único registry genérico com uma regra de ordenação é
mais fácil de raciocinar sobre do que uma hierarquia de classes com
uma raiz mágica. O padrão do Laravel funciona porque o autoloading
de classes do PHP e a reflexão estática de `make()` deixam
`call([A::class, B::class])` encontrar e instanciar essas classes
pelo nome; em Rust estaríamos pedindo ao usuário para passar objetos
de trait `dyn Seeder` por aí, o que é mais desajeitado do que o
registry de function-pointer que já está lá.

A convenção de seeder composto recupera a mesma ergonomia -
`BaseSeeder` desempenha o papel que `DatabaseSeeder` desempenha no
Laravel - sem precisar que o framework consagre um nome como
especial.

## Registro no bootstrap

Todo seeder precisa de uma chamada `seed::register` em
`bootstrap.rs`, ao lado da outra fiação global ao processo (config,
observers, supervisors, jobs de fila). O padrão tem a mesma forma
usada em outros lugares do arquivo de bootstrap:

```rust
// src/bootstrap.rs
pub async fn register() {
    // …config + vinculações de contêiner + fiação de auth…

    // Seeders. A ordem importa - run_all visita na ordem de registro.
    suprnova::seed::register::<crate::seeders::BaseSeeder>();
    suprnova::seed::register::<crate::seeders::DemoContentSeeder>();

    // …observers, supervisors, jobs de fila…
}
```

Se você esquecer de registrar um seeder, `console db:seed --class=X`
falha com "no seeder registered for `X`" - um sinal claro em vez de
um skip silencioso. Os helpers `seed::count()` e
`seed::is_registered("…")` existem exatamente para que um teste
possa afirmar que o bootstrap registrou todo seeder que você
esperava.

Veja [Inicialização da aplicação](bootstrap.md) para a estrutura
completa do arquivo e a ordem em que o framework espera que cada
subsistema seja conectado.

## Próximos passos

- [Migrações](migrations.md) - a metade do esquema do par
  preenchimento/migração
- [Eloquent](eloquent.md) - models, factories, e a maquinaria
  `Persistable` que todo seeder aciona
- [Console](console.md) - o binário `console` por projeto que
  hospeda `db:seed` ao lado dos seus próprios `#[command]`s
- [Testes](testing.md) - `TestContainer`, `TestDatabase::fresh`, e o
  padrão `#[serial]` para testes que tocam o registry de seeders
- [Modelo de erros](error-model.md) - o que é `FrameworkError` e como
  a forma `Result<(), _>` de `run` se compõe com o resto do framework

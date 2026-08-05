# Factories Eloquent

Factories produzem instâncias de model randomizadas para testes e
seeders. A forma é a do Laravel:
`UserFactory::new().count(10).create_many().await?`. O contrato é um
trait mais um builder fluente, com um atalho `#[derive(Factory)]`
para o caso comum em que o model já tem uma representação
randomizada sensata.

Este capítulo cobre definir factories manualmente e por derive,
compor overrides em "states" reutilizáveis, IDs determinísticos via
`Sequence`, a costura `Persistable` que impulsiona `create`, e a
diferença entre `make` (em memória) e `create` (persistido). Para o
contexto de escrita de testes em que factories são mais úteis, veja
[Testes](testing.md).

## O trait `Factory`

O trait tem exatamente um método obrigatório:

```rust
pub trait Factory {
    type Model;

    fn definition() -> Self::Model
    where
        Self: Sized;
}
```

`definition()` retorna um model totalmente populado com todo campo
randomizado para o que fizer sentido como padrão. O trait não carrega
estado por instância - implementadores são tipicamente marcadores de
tamanho zero (`struct UserFactory;`), então quem chama pode alcançar
a factory pelo nome sem manter um handle.

O trait também fornece dois pontos de entrada de builder com
implementações padrão:

```rust
fn new() -> FactoryBuilder<Self::Model>;       // count = 1, sem overrides
fn times(n: usize) -> FactoryBuilder<Self::Model>;  // açúcar para new().count(n)
```

Todo outro método que você vai chamar (`with`, `count`, `make`,
`create`, `create_many`, …) vive em `FactoryBuilder<M>`.

## Definindo uma factory manualmente

A forma manual mínima combina uma struct marcadora com uma impl
`Factory` que sabe como construir uma instância. Você tipicamente vai
recorrer a isto quando o model não deriva `fake::Dummy` - talvez
porque alguns campos precisem de seeding determinístico (IDs de
relação em uma faixa conhecida) ou a representação randomizada
precise de conhecimento de regra de negócio:

```rust
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        let now = chrono::Utc::now();
        User {
            // `0` é um placeholder - `persist_via_seaorm` inverte
            // colunas de chave primária para `NotSet` antes de
            // inserir, para que o banco de dados atribua o id real.
            id: 0,
            name: format!("Factory User #{}", next_seq()),
            email: format!("factory-{}@example.test", next_seq()),
            password: "factory-placeholder".into(),
            remember_token: None,
            active: true,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

Os campos `__eager` e `__pivot` são o estado de rascunho de
eager-load e pivot que a macro `#[suprnova::model]` injeta em toda
struct Eloquent. Sempre deixe-os no padrão - eles são populados pelo
construtor de consultas, não por factories.

`next_seq()` pode ser o que você quiser - um `static AtomicU64`, uma
`Sequence` (coberta abaixo), ou um contador thread-local. O ponto é
que `definition()` roda do zero a cada chamada dentro de `make_many` /
`create_many`, então qualquer unicidade que você precise tem que vir
de um contador que a função consiga alcançar.

## `#[derive(Factory)]` para o caso comum

Quando o próprio model implementa `fake::Dummy` - seja via
`#[derive(Dummy)]` ou uma impl `Dummy<Faker> for Model` escrita à
mão - o derive colapsa o marcador + impl em uma linha no model:

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub author_id: i64,
    pub is_public: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

O derive emite `pub struct PostFactory;` como um tipo irmão e uma
`impl Factory for PostFactory` cujo `definition()` chama
`Faker.fake::<Post>()`. A visibilidade na factory espelha a
visibilidade no model - um model `pub` recebe uma factory `pub`, um
model `pub(crate)` recebe uma factory `pub(crate)`.

### Sobrescrevendo o nome gerado

Por padrão, `#[derive(Factory)]` emite `<Model>Factory`. Sobrescreva
via o attribute `name`:

```rust
#[derive(Dummy, Factory)]
#[factory(name = "AccountFactory")]
pub struct User { /* … */ }
```

O valor precisa parsear como um identificador Rust - `name = "User
Factory"` ou `name = "user-factory"` falha ao compilar com um erro
claro, apontado por span. A macro emite `pub struct <Name>;`
literalmente, então nada que não possa ser um nome de tipo pode ser
um nome de factory.

### `Dummy` escrito à mão para randomização mais rica

`#[derive(Dummy)]` funciona para structs de tipos primitivos, mas não
te dá controle sobre distribuições ou invariantes entre campos. Para
qualquer coisa não trivial, escreva a impl `Dummy` à mão e combine-a
com `#[derive(Factory)]`:

```rust
use suprnova::__fake::rand::Rng;
use suprnova::__fake::{Dummy, Fake, Faker, faker::lorem::en::{Paragraph, Sentence}};
use suprnova::Factory;

#[derive(Factory)]
pub struct Post { /* fields … */ }

impl Dummy<Faker> for Post {
    fn dummy_with_rng<R: Rng + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        let title: String = Sentence(3..7).fake_with_rng(rng);
        let body: String = Paragraph(3..6).fake_with_rng(rng);
        let author_id: i64 = (1..=50i64).fake_with_rng(rng);
        let now = chrono::Utc::now();

        Post {
            id: 0,
            author_id,
            title,
            body,
            is_public: Faker.fake_with_rng::<bool, _>(rng),
            created_at: now,
            updated_at: now,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

O crate `fake` é reexportado como `suprnova::__fake`, então quem
consome não precisa de uma linha `fake = "…"` separada no
`Cargo.toml`. Tipos comuns também são reexportados sob a raiz do
crate: `suprnova::{Dummy, Fake, Faker}`.

### Por que `#[derive(Factory)]` só aceita structs simples

O derive rejeita enums, unions, e models genéricos com um erro claro
de compilação. Enums e unions não têm uma representação padrão com
sentido. Genéricos forçariam uma decisão sobre como o tipo da factory
parametriza seu model - e não há um padrão bom, então o derive se
recusa a adivinhar. Escreva a `impl Factory` à mão para esses casos.

## O construtor fluente

`Factory::new()` / `Factory::times(n)` retornam um `FactoryBuilder<M>`.
Toda operação é encadeável; nada acontece até você chamar um método
terminal (`make`, `make_one`, `make_many`, `create`, `create_one`,
`create_many`).

### `count(n)` - quantas instâncias

```rust
let user = UserFactory::new().make();             // 1 usuário
let users = UserFactory::new().count(10).make_many();  // 10 usuários
let same = UserFactory::times(10).make_many();   // idêntico
```

`count(n)` é ignorado por `make` / `create` (sempre um) e respeitado
por `make_many` / `create_many`. `times(n)` é só açúcar para
`Self::new().count(n)` e corresponde ao `Factory::times($n)` do
Laravel.

### `with(|m| { … })` - overrides por chamada

`with` registra uma closure que roda contra toda instância produzida
depois de `definition()`. Múltiplas chamadas `with` compõem na ordem
de registro, então um override posterior sobrescreve um anterior no
mesmo campo:

```rust
let admin = UserFactory::new()
    .with(|u| u.active = true)
    .with(|u| u.role = "admin".into())
    .make();
```

Overrides são armazenados como `Box<dyn Fn(&mut M) + Send + Sync + 'static>`
para que o builder permaneça `Send` - importante para os caminhos
assíncronos `create` / `create_many`, que mantêm o builder através de
um `.await` no insert do SeaORM.

### `prepend(|m| { … })` - padrões que quem chama ainda pode sobrescrever

`prepend` insere uma closure na **frente** da cadeia de overrides,
então ela roda **antes** de qualquer outro `with(...)`. Use-a dentro
de um método de estado quando você quiser fornecer um padrão que quem
chama ainda pode sobrescrever com um `.with(...)` posterior:

```rust
impl UserFactory {
    /// Método de estado - padrões de admin, quem chama ainda pode personalizar.
    pub fn admin() -> suprnova::FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.active = true)
    }
}

// Quem chama vence em `role` porque o .with() dele vem depois dos prepends.
let owner = UserFactory::admin()
    .with(|u| u.role = "owner".into())
    .make();
```

Este é o equivalente do Suprnova ao `Factory::prependState` do
Laravel. É o primitivo certo especificamente para métodos de estado -
`with` perderia para um `.with(...)` de quem chama, que é o oposto do
que um padrão deveria fazer.

### `when(cond, |b| { … })` - encadeamento condicional

`when` propaga uma flag por uma cadeia sem quebrar o estilo fluente.
A closure recebe o builder, retorna o builder. Quando `cond` é falso,
o builder passa direto, sem mudanças:

```rust
UserFactory::times(10)
    .with(|u| u.active = true)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

Espelha o `Conditionable::when($cond, $cb)` do Laravel. A assinatura
`FnOnce(Self) -> Self` significa que você pode `await` dentro da
closure, desde que você `.await` antes de retornar o builder.

### Métodos terminais

| Método | Retorna | Persistido? |
|---|---|---|
| `make()` | um `M` | não |
| `make_one()` | um `M` (força count = 1) | não |
| `make_many()` | `Vec<M>` de `count` itens | não |
| `create()` | `Result<M, FrameworkError>` | sim |
| `create_one()` | `Result<M, FrameworkError>` (força count = 1) | sim |
| `create_many()` | `Result<Vec<M>, FrameworkError>` | sim |

`make_one` e `create_one` são úteis quando um método de estado
definiu `count` internamente para outro valor e quem chama quer
exatamente um resultado:

```rust
pub fn admins_in_org(org_id: i64) -> suprnova::FactoryBuilder<User> {
    UserFactory::times(5)               // padrão sensato para fixtures
        .with(move |u| u.org_id = org_id)
        .with(|u| u.role = "admin".into())
}

// O teste só quer um - `create_one` descarta o count(5).
let admin = admins_in_org(42).create_one().await?;
```

## Estados: combinações de preset reutilizáveis

O Suprnova não traz uma tabela de lookup `state("name")`. Em vez
disso, states são métodos simples no marcador da sua factory que
retornam um `FactoryBuilder<M>` pré-configurado. O padrão compõe por
herança - todo método de estado retorna o mesmo tipo
`FactoryBuilder<M>`, então você pode encadear mais métodos sobre o
resultado:

```rust
use suprnova::FactoryBuilder;
use crate::models::users::User;

pub struct UserFactory;

impl suprnova::Factory for UserFactory {
    type Model = User;
    fn definition() -> User { /* … */ }
}

impl UserFactory {
    /// Variante inativa - sobrepõe um padrão `active: false`.
    pub fn inactive() -> FactoryBuilder<User> {
        Self::new().prepend(|u| u.active = false)
    }

    /// Variante admin - sobrepõe role + email verificado.
    pub fn admin() -> FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.email_verified_at = Some(chrono::Utc::now()))
    }

    /// Composável: admin inativo.
    pub fn inactive_admin() -> FactoryBuilder<User> {
        Self::admin().prepend(|u| u.active = false)
    }
}
```

```rust
// Componha no call site também - encadeie mais overrides livremente.
let user = UserFactory::admin()
    .with(|u| u.name = "Alice".into())
    .create()
    .await?;

let batch = UserFactory::inactive().count(20).create_many().await?;
```

A escolha de `prepend` é deliberada: os overrides de um estado são
*padrões* que quem chama ainda pode reescrever. Se você quiser que a
configuração de um estado seja inegociável, use `with` em vez disso -
ele vai para o final da cadeia e vence.

### Por que não existe um lookup de `state("name")`

Um registry de estado chaveado por nome forçaria correspondência de
string em runtime para algo que o compilador consegue verificar.
Métodos de estado te dão verificação em tempo de compilação (o erro
de digitação `UserFactor::admn()` é um erro de compilação) e
autocomplete completo da IDE. A composabilidade - encadear
`Self::admin()` de dentro de `inactive_admin()` - vem de graça.

## IDs determinísticos com `Sequence`

`Sequence` é um contador monotônico para semear campos únicos por
chamada. Toda chamada a `next()` retorna 1, 2, 3, … atomicamente
entre threads:

```rust
use suprnova::{Fake, Sequence};

static ORDER_IDS: Sequence = Sequence::new();

pub struct OrderFactory;
impl suprnova::Factory for OrderFactory {
    type Model = Order;
    fn definition() -> Order {
        Order {
            id: 0,
            number: format!("ORD-{:06}", ORDER_IDS.next()),
            total_cents: (100..=10_000).fake(),
            created_at: chrono::Utc::now(),
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

`Sequence::new()` é `const`, então funciona como um inicializador
`static`. O contador começa em 0 e incrementa para 1 na primeira
chamada. Use `reset()` entre testes se você quiser uma contagem
limpa - a macro `#[suprnova_test]` não faz isso por você porque o
framework não pode saber quais sequences são suas:

```rust
#[suprnova::suprnova_test]
async fn each_order_gets_a_unique_number(db: TestDatabase) {
    ORDER_IDS.reset();   // começa em 1 para este teste
    let orders = OrderFactory::new().count(5).create_many().await?;
    assert_eq!(orders[0].number, "ORD-000001");
    assert_eq!(orders[4].number, "ORD-000005");
}
```

`Sequence` usa ordenação `SeqCst` - um exagero para "me dê um id
único", mas mantém o raciocínio trivial. Se uma Sequence algum dia
aparecer em um hot path, você pode escrever a sua própria com
`Relaxed`.

## `Persistable`: a costura para seu armazenamento

A família de métodos `create` está disponível sempre que o model
implementa `Persistable`:

```rust
#[async_trait]
pub trait Persistable: Sized + Send {
    async fn persist(self) -> Result<Self, FrameworkError>;
}
```

Uma impl blanket em `factory::persist` cobre todo model SeaORM que
consegue `IntoActiveModel<ActiveModel>` - que é todo model que a
macro `#[suprnova::model]` emite. Sem boilerplate por model; se
`User` é um model, `UserFactory::new().create()` funciona.

O blanket puxa `DB::connection()` e insere. O `Self` retornado é o
que o SeaORM devolve a partir do insert - id atribuído, colunas com
padrão resolvidas, etc.

### Tratamento de chave primária

Uma impl `IntoActiveModel` do SeaORM marca todo campo - incluindo a
PK - como `Set(value)`. Para models produzidos por factory, a PK é
um placeholder (`0` para `AUTO_INCREMENT i64`), então um insert direto
colide na segunda chamada com uma falha de constraint UNIQUE.

`persist_via_seaorm` (o helper por trás do blanket) inverte toda
coluna de chave primária para `NotSet` antes de inserir, o que deixa
o banco de dados atribuir seu próprio id - a semântica que factories
realmente precisam:

```rust
pub async fn persist_via_seaorm<M, E, C>(model: M, db: &C) -> Result<M, FrameworkError>
where
    M: ModelTrait<Entity = E> + IntoActiveModel<<E as EntityTrait>::ActiveModel> + Send,
    E: EntityTrait<Model = M>,
    /* … bounds … */
    C: ConnectionTrait,
{
    let mut active = model.into_active_model();
    for pk in <<E as EntityTrait>::PrimaryKey as Iterable>::iter() {
        active.not_set(pk.into_column());
    }
    active.insert(db).await.map_err(/* … */)
}
```

Se você realmente *quiser* atribuir um id específico (teste de
replay, restaurar uma fixture por id), contorne o helper e chame
`model.into_active_model().insert(db).await` diretamente.

### Persistindo contra uma conexão explícita

`persist_via_seaorm` recebe a conexão como argumento. Útil quando
você quer direcionar a persistência contra uma conexão que não é o
`DB::connection()` vinculado do framework - mais frequentemente um
handle `sqlite::memory:` específico em um teste de integração:

```rust
use suprnova::factory::persist_via_seaorm;

let model = UserFactory::new().make();
let row = persist_via_seaorm(model, db.inner()).await?;
```

### Backends não-SeaORM personalizados

Porque a impl blanket tem como alvo todo tipo `ModelTrait`, você não
pode escrever `impl Persistable for MyOrm::Model` a partir de um
crate downstream sem colidir. Para persistência customizada não-SeaORM
(Redis, Surreal, stores só-de-blob), envolva o model em um newtype e
implemente `Persistable` no wrapper:

```rust
use suprnova::{FrameworkError, Persistable};
use suprnova::async_trait;

pub struct RedisCached<T>(pub T);

#[async_trait]
impl Persistable for RedisCached<MyValue> {
    async fn persist(self) -> Result<Self, FrameworkError> {
        let client = suprnova::App::make::<RedisClient>()
            .ok_or_else(|| FrameworkError::internal("redis client not bound"))?;
        client.set(&self.0.key, &serde_json::to_vec(&self.0)?).await?;
        Ok(self)
    }
}
```

Um `Factory<Model = RedisCached<MyValue>>` então recebe `create` /
`create_many` de graça.

## `make` vs `create`: quando usar qual

`make` retorna o model sem tocar no banco de dados:

```rust
// Teste unitário para uma função pura - sem necessidade de BD.
let draft = PostFactory::new().with(|p| p.is_public = false).make();
let snippet = my_lib::extract_summary(&draft);
assert!(snippet.len() < 200);
```

`create` persiste e retorna a versão pós-insert:

```rust
// Teste de integração - a action precisa de uma linha real.
let post = PostFactory::new().create().await?;
let action = App::resolve::<PublishPostAction>().unwrap();
let published = action.execute(post.id).await?;
assert!(published.is_public);
```

Recorra a `make` sempre que o teste não se importa que a linha
exista. Recorra a `create` quando você for consultar a linha de
volta, quando uma foreign key precisa de um id real, ou quando você
está populando fixtures para um subsistema que lê do BD. Note que
`create_many` persiste sequencialmente - se um insert posterior
falhar, os inserts anteriores NÃO são desfeitos. `create` /
`create_many` passam pelo blanket `Persistable`, que fala diretamente
com o `DB::connection()` vinculado do framework - eles **não** entram
em um escopo `DB::transaction(...)` ambiente. Se você precisa de
atomicidade através de um lote de inserts, desça para o
`Model::create(attrs!{...})` do trait Model dentro da closure (esse
caminho roteia através do mesmo executor que respeita `CURRENT_TX`):

```rust
use suprnova::{DB, Model, attrs};

DB::transaction(|_tx| Box::pin(async move {
    for i in 0..50 {
        User::create(attrs!{
            name: format!("user-{i}"),
            email: format!("user-{i}@example.test"),
        }).await?;
    }
    Ok::<_, suprnova::FrameworkError>(())
})).await?;
```

## Comportamento "pós-criação"

O Suprnova não traz um callback nomeado `after_creating(|m| { … })`.
Dois padrões cobrem os casos de uso para os quais esse callback
existe no Laravel:

**1. A cadeia - faça o follow-up depois de `create`/`create_many`:**

```rust
let user = UserFactory::new().create().await?;
ProfileFactory::new()
    .with(move |p| p.user_id = user.id)
    .create()
    .await?;
```

Este é o padrão canônico quando o id de um model precisa fluir para
um insert de follow-up. `create` retorna a linha persistida, então o
id fica imediatamente disponível.

**2. Observers de model - reaja no ciclo de vida do model, não na factory:**

Use [Observers de model](eloquent.md#observers) para conectar
comportamento pós-insert ao próprio model em vez da factory. O
observer dispara para `User::create(...)`, `UserFactory::new().create()`,
e qualquer outro caminho de persistência - exatamente o que você quer
quando o comportamento é "toda vez que esta linha chegar, faça X":

```rust
use suprnova::{FrameworkError, Observer, async_trait, observer};

#[observer(User)]
pub struct AuditUser;

#[async_trait]
impl Observer<User> for AuditUser {
    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user_id = user.id, "user created");
        Ok(())
    }
}
```

Callbacks só-de-factory convidariam à divergência entre inserts de
teste e inserts reais. Observers permanecem consistentes nos dois.

## Seeders

Factories produzem instâncias; seeders as orquestram. Um `Seeder` é
um tipo de tamanho zero com um `run` assíncrono que sabe o que
popular:

```rust
use suprnova::{Factory, FrameworkError, Seeder};
use suprnova::async_trait;

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // Usuários primeiro - posts referenciam ids de usuário em 1..=50.
        UserFactory::new().count(50).create_many().await?;
        PostFactory::new().count(200).create_many().await?;
        Ok(())
    }
}
```

Registre o seeder em `bootstrap.rs` para que o comando `db:seed` do
binário `console` do projeto saiba dele:

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

Execute através do binário `console` do projeto (toda app com scaffold
traz um em `src/bin/console.rs`):

```bash
cargo run --bin console -- db:seed
```

Seeders rodam na ordem de registro. Idempotência é responsabilidade
do seeder - `run` não tira snapshot nem desfaz, então um seeder que
insere incondicionalmente produz duplicatas em uma re-execução. Use
`migrate:fresh` seguido de `db:seed` para um estado limpo.

## Juntando tudo: uma fixture de teste completa

```rust
use suprnova::{App, describe, test, expect};
use suprnova::events::{EventFacade, assert_dispatched_times};
use suprnova::testing::TestDatabase;
use crate::factories::{PostFactory, UserFactory};
use crate::actions::publish_post::PublishPostAction;

describe!("PublishPostAction", {
    test!("publishes a draft post", async fn(db: TestDatabase) {
        // Arrange - um autor e um post rascunho pertencente a ele.
        let author = UserFactory::new()
            .with(|u| u.active = true)
            .create()
            .await
            .unwrap();

        let draft = PostFactory::new()
            .with(move |p| p.author_id = author.id)
            .with(|p| p.is_public = false)
            .create()
            .await
            .unwrap();

        // Act.
        let action = App::resolve::<PublishPostAction>().unwrap();
        let published = action.execute(draft.id).await.unwrap();

        // Assert.
        expect!(published.is_public).to_equal(true);
        expect!(published.author_id).to_equal(author.id);
    });

    test!("publishing emits exactly one event", async fn(db: TestDatabase) {
        let _guard = EventFacade::fake();
        let post = PostFactory::new().create().await.unwrap();

        App::resolve::<PublishPostAction>().unwrap()
            .execute(post.id).await.unwrap();

        assert_dispatched_times::<crate::events::PostPublished>(1);
    });
});
```

Três padrões que vale destacar:

- O `id` do autor flui para o post via uma closure `move` dentro de
  `.with(...)`. Capturas são explícitas, o que mantém a relação
  visível no call site.
- `create().await.unwrap()` é o idioma de teste - o teste tem
  permissão para entrar em panic numa falha de setup porque uma
  fixture quebrada é um teste quebrado, não um modo de falha
  gracioso.
- Factories compõem com o resto da superfície de teste
  (`EventFacade::fake`, `Storage::fake`, `Mail::fake`, …) - nenhum
  dos fakes sabe sobre factories, mas todo teste que você escrever
  vai usá-los juntos.

### Por que Suprnova diverge

As factories do Laravel vêm com states nomeados (`->state('admin')`),
sequences em runtime (`->sequence(['name' => 'A'], ['name' => 'B'])`),
e um callback `afterCreating` registrado na própria factory. O
Suprnova descarta os três e os substitui por primitivos no formato do
Rust:

- **States são métodos, não strings.** Verificação de erro de
  digitação em tempo de compilação e autocomplete de IDE são
  gratuitos; o único custo é "você escreve `pub fn admin()` em vez de
  `protected function admin()`", o que não é custo nenhum.
- **Sequences são um primitivo separado.** `Sequence` faz uma coisa
  (contador atômico) e é reutilizável fora da superfície de factory -
  você pode colocar uma em um gerador de request id, um contador de
  passo de workflow, ou um harness de teste sem precisar explicar o
  que ela é.
- **After-creating está conectado ao model, não à factory.** O
  framework já tem [Observers de model](eloquent.md#observers)
  exatamente para esse propósito. Adicionar um mecanismo paralelo na
  factory faria o comportamento em tempo de teste e o comportamento
  em tempo de produção divergirem por construção.

A superfície fluente - `count(10)`, `times(10)`, `with`, `prepend`,
`when`, `make`, `create`, `create_many`, `make_one`, `create_one` -
espelha a do Laravel diretamente, então a memória muscular migra sem
precisar de um glossário.

## Próximos passos

- [Testes](testing.md) - `#[suprnova_test]`, `TestDatabase`, as
  facades fake que se combinam com fixtures construídas por factory.
- [Eloquent](eloquent.md) - derivação de model, observers, o pipeline
  de cast que roda quando `create` persiste a saída da sua factory.
- [Migrações](migrations.md) - o esquema que suas factories precisam
  que exista; use `migrate:fresh && db:seed` para um estado limpo de
  fixture.
- [Banco de dados](database.md) - `DB::transaction`, roteamento
  multi-conexão, savepoints - o que usar quando `create_many`
  precisa de atomicidade.
- [Contêiner de serviços](container.md) - como `App::resolve` e
  `App::make` encontram os tipos de action e service que seus testes
  chamam, ao lado de factories.

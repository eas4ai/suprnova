# Contêiner de serviços

O contêiner é onde Suprnova mantém os serviços de sua aplicação -
o pool de conexão de DB, o driver de mail, seu `Arc<MyService>`. Você
vincula valores a ele no tempo de inicialização e os resolve em handlers e
workers. É o equivalente Suprnova do service container do Laravel,
com uma diferença importante: lookup é task-local primeiro, então testes
executados concorrentemente não veem as vinculações uns dos outros.

## As duas peças

| Tipo | Papel |
|---|---|
| `Container` | O registro subjacente: mantém vinculações, factories, e singletons |
| `App` | A fachada global que você realmente chama - `App::bind`, `App::get`, etc. |

Você quase sempre chama `App::*` em vez de construir um
`Container` diretamente. O contêiner é encanamento; a fachada `App`
é a API.

## Ordem de lookup

Cada chamada `App::get` / `App::make` verifica **três camadas** em ordem:

```
        task-local
            │
            ▼  (miss)
       thread-local
            │
            ▼  (miss)
          global
            │
            ▼  (miss)
          None
```

Isto importa porque:

- **Estado por solicitação passa por task-local** - dados compartilhados Inertia,
  flash bag, request id. Cada solicitação recebe sua própria camada, transparentemente.
- **Testes usam thread-local** - `let _g = TestContainer::fake();`
  seguido por `TestContainer::bind(...)` vincula dentro de uma thread
  sem tocar o contêiner global, então testes paralelos não
  vazam serviços uns nos outros. A guarda limpa o contêiner
  de teste quando é dropada.
- **Serviços app-wide passam por global** - vinculado uma vez na inicialização,
  resolvido em todos os lugares.

Você raramente pensa em qual camada uma vinculação vive - `App::bind`
a coloca onde faz sentido, e `App::get` a encontra onde quer que
viva. O modelo importa apenas quando algo se comporta inesperadamente
sob concorrência, e então o capítulo [Testes](testing.md) tem o
detalhe.

## Vinculando um valor

Cinco formas de colocar algo no contêiner, dependendo do que você
tem:

### `App::singleton(value)` - o contêiner é dono do valor, clonado no lookup

Para qualquer valor `T: Any + Send + Sync + 'static` que deve viver
para sempre. O bound `Clone` está no *getter* (`App::get`), não na
vinculação - o valor é armazenado uma vez dentro de um `Arc` e clonado
daquele `Arc` em cada `get`:

```rust
use suprnova::App;

App::singleton(MyConfig {
    timeout_secs: 30,
    retries: 3,
});

let cfg = App::get::<MyConfig>().expect("registered at boot");
println!("{}", cfg.timeout_secs);
```

O valor é armazenado uma vez; `App::get::<MyConfig>()` retorna um clone.
Use isto para dados em forma de config simples que são baratos de clonar.

### `App::bind(Arc<T>)` - para traits e serviços compartilhados

Para objetos trait ou qualquer coisa que você queira atrás de um `Arc`:

```rust
use std::sync::Arc;
use suprnova::App;

let store: Arc<dyn KeyValueStore> = Arc::new(RedisStore::connect(url)?);
App::bind(store);

let store = App::make::<dyn KeyValueStore>().expect("bound at boot");
store.put("hello", b"world").await?;
```

`App::make::<T>()` retorna o clone `Arc<T>` (bump de refcount atômico barato).
Use isto para qualquer serviço compartilhado entre threads, especialmente
objetos trait.

### `App::factory(|| { … })` - construído sob demanda

Quando construir o valor deve acontecer no primeiro uso (ou toda vez):

```rust
App::factory(|| {
    HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client config is hand-rolled and known-good")
});
```

`App::factory` registra uma factory *concrete-type* (`Fn() -> T`);
`App::bind_factory` registra uma factory *trait-object*
(`Fn() -> Arc<T>`). Nenhuma closure retorna `Result` - trate falha de
construção dentro da closure (panic na inicialização, ou construa um
valor sentinela) ou use um `App::singleton` / `App::bind` comum após
construir o valor você mesmo com `?`. Ambos invocam a closure fora de
qualquer lock de contêiner, então uma factory que reentra no contêiner
não fará deadlock e um construtor caro não bloqueará outras vinculações.

### `App::*_if_absent(value)` - registro amigável a ordem de boot

Às vezes um serviço padrão é registrado por um crate de serviço, e o
app quer sobrescrevê-lo apenas quando presente. As variantes
`_if_absent` permitem que você registre um padrão que não vai
sobrescrever uma vinculação existente:

```rust
// Dentro de um starter ou crate de biblioteca:
App::singleton_if_absent(DefaultMailDriver::new());

// No bootstrap.rs do seu app:
App::singleton(MyCustomMailDriver::new());  // vence porque executou depois
```

`bind_if_absent`, `singleton_if_absent`, e as variantes de factory
todas retornam `bool` - `true` se realmente inseriram, `false` se já
havia uma vinculação.

## Resolvendo um valor

Dois métodos de leitura, mais seus irmãos que retornam `Result`:

```rust
// Clone o valor vinculado:
let cfg: MyConfig = App::get::<MyConfig>().expect("bound at boot");

// Clone o Arc:
let store: Arc<dyn KeyValueStore> = App::make().expect("bound at boot");

// Mesmo mas Result, para o idioma `?` em caminhos falíveis:
let cfg = App::resolve::<MyConfig>()?;
let store = App::resolve_make::<dyn KeyValueStore>()?;
```

`resolve` e `resolve_make` retornam `Result<_, FrameworkError>`
(especificamente a variante `ServiceNotFound` quando o lookup falha) -
útil em caminhos de handler onde um serviço ausente deve aparecer como
um 500 com um log apropriado, não um panic.

Verificações de associação (raramente necessárias):

```rust
if App::has::<MyConfig>() { … }
if App::has_binding::<dyn KeyValueStore>() { … }
```

## Onde a vinculação acontece

O lugar padrão é `src/bootstrap.rs` - uma função que executa
uma vez na inicialização:

```rust
use std::sync::Arc;
use suprnova::App;
use crate::services::{MyService, RealEmailGateway};

pub async fn register() {
    // Singletons simples
    App::singleton(MyAppConfig {
        max_uploads_per_user: 100,
    });

    // Serviços trait-object
    let gateway: Arc<dyn EmailGateway> = Arc::new(RealEmailGateway::new());
    App::bind(gateway);

    // Serviços lazy (construídos no primeiro uso)
    App::bind_factory::<dyn HttpClient, _>(|| {
        Arc::new(ReqwestClient::with_timeout(30))
    });
}
```

O nome da função `register` corresponde ao padrão scaffold (`src/bootstrap.rs::register`); o tipo de retorno é `()`, não `Result`. Erros de binding que acontecem durante a inicialização (por exemplo, falhas de conexão de driver) devem se propagar através do construtor do driver/serviço, não de `register` em si - veja [Inicialização da aplicação](bootstrap.md) para a fiação de boot completa.

O framework também chama o contêiner em si durante a inicialização:

- `App::init()` executa primeiro, inicializando o registro
- `App::boot_services()` resolve dependências de boot-time (drivers,
  chaves de criptografia, etc.) - seus serviços veem um framework totalmente iniciado
- Seu `bootstrap_fn` executa depois disso, então pode contar com os
  serviços do framework estarem disponíveis

Veja [Inicialização da aplicação](bootstrap.md) para a ordem de boot completa.

## Dados compartilhados de Inertia

O contêiner também é onde os dados compartilhados de Inertia vivem. Três
APIs de conveniência tornam isto explícito:

```rust
use suprnova::App;

// Eager value - serializado uma vez e reutilizado para cada resposta Inertia.
App::inertia_share("appName", "Suprnova");

// Lazy value - resolver executa por resposta. Use para dados por solicitação
// que precisam de trabalho assíncrono.
App::inertia_share_lazy("locale", || async {
    Ok::<_, suprnova::FrameworkError>(detect_locale().await)
});

// Adiciona uma única entrada flash ao flash bag por solicitação.
App::flash("message", "Saved!");
```

Estes leem de `Container::inertia()` que retorna
`&Arc<InertiaRegistry>` - você pode interagir com ela diretamente se
precisar de acesso de nível mais baixo. Veja [Inertia / Frontend](frontend.md) para
como os dados compartilhados terminam na resposta da página.

## Por que três camadas?

A cascata task-local → thread-local → global existe por uma
razão: **isolamento sob concorrência**. Três coisas se beneficiam:

**Isolamento por solicitação.** O flash bag de Inertia é vinculado por solicitação
via a camada task-local. Duas solicitações concorrentes não veem
flash umas das outras porque seus contêineres task-local não se sobrepõem. A
vinculação se evapora quando a tarefa da solicitação termina.

**Isolamento por teste.** Um teste que vincula um fake mail driver não deve
ver um fake vinculado por um teste irmão. `TestContainer::fake()`
retorna uma guarda thread-local, e `TestContainer::bind` /
`TestContainer::singleton` roteiam escritas para o escopo ativo.
Testes paralelos permanecem herméticos:

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn one_test_binds_a_fake() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn Mailer>(Arc::new(FakeMailer::new()));

    // … este teste usa FakeMailer
    // um teste irmão executando em paralelo não o vê
}
```

Para runtimes tokio multi-thread - onde o future pode migrar entre
worker threads - use `TestContainer::scope(async { ... })` em vez disso;
isso instala um override task-local que sobrevive à migração.

**Override-na-inicialização.** Código de aplicação pode sobrescrever padrões registrados
por crates de biblioteca. As variantes `_if_absent` e o lookup em
camadas combinam para dar aos crates de biblioteca registro de padrão
limpo sem lutar com overrides de aplicação.

## Padrões comuns

### Vincular um struct que contém o pool de DB

Você quase nunca faz isto diretamente - o framework vincula o pool de DB
em si. Mas se você tiver seu próprio subsistema com um recurso
compartilhado caro:

```rust
let pool = MyResourcePool::connect(url).await?;
App::bind(Arc::new(pool));

// mais tarde:
let pool = App::resolve_make::<MyResourcePool>()?;
let conn = pool.checkout().await?;
```

`App::make` retorna `Option<Arc<T>>` e emparelha com `.expect(...)`; `App::resolve_make` retorna `Result<Arc<T>, FrameworkError::ServiceNotFound>` e emparelha com `?` em código falível. Use aquele que corresponde à história de erro do seu chamador.

### Trocar um padrão por um fake em testes

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn order_dispatches_email() {
    let fake = Arc::new(FakeEmailGateway::new());
    let fake_for_assert = Arc::clone(&fake);

    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.expect("place_order succeeds");

    assert_eq!(fake_for_assert.sent_count(), 1);
}
```

### Construção lazy cara

```rust
// Constrói o embedding model na primeira solicitação, não na inicialização.
App::bind_factory::<dyn EmbeddingModel, _>(|| {
    Arc::new(
        OnnxEmbedding::load_from_disk("/models/all-mini-lm.onnx")
            .expect("embedding model must load"),
    )
});
```

Para construção falível que precisa expor um erro estruturado para
o operador, construa o valor você mesmo em `bootstrap()` com `?` e
chame `App::bind(...)` uma vez que esteja pronto.

## Por que Suprnova diverge

O contêiner do Laravel tem um escopo global - vinculações são globais, e
isolar entre testes requer disciplina `setUp` / `tearDown` mais
a transação de banco de dados por teste do framework. O modelo
request-per-process do PHP torna isto seguro por acaso: um processo novo
por solicitação significa que o contêiner é resetado toda vez.

O modelo de processo do Rust é o oposto - um processo serve muitas
solicitações concorrentes em muitas threads. Um contêiner somente global
significaria que um teste em uma thread pode ver um fake vinculado por outro, ou uma
solicitação pode ver dados por solicitação de outra solicitação. É por isso que
Suprnova tem a cascata de três camadas: task-local para por solicitação,
thread-local para por teste, global para app-wide.

A API do contêiner é a mesma que a do Laravel; a maquinaria de lookup
é diferente porque o runtime é diferente.

## Próximos passos

- [Inicialização da aplicação](bootstrap.md) - onde o código de vinculação vai
- [Configuração](configuration.md) - registro de config tipado
  junto com serviços
- [Testes](testing.md) - `TestContainer::fake` e `#[suprnova_test]`
- [Política de bloqueio](lock-policy.md) - por que a recuperação de poisoned-lock importa
  em uma aplicação apoiada por contêiner
